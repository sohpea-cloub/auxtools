use std::{
	borrow::Cow,
	cell::{OnceCell, RefCell},
	collections::{BTreeMap, HashMap, VecDeque},
	io::Write,
	sync::{
		atomic::{AtomicU64, Ordering},
		mpsc, Arc, Mutex
	},
	thread,
	time::{Duration, Instant}
};

use auxtools::{
	raw_types::values::{ValueData, ValueTag},
	*
};
use auxtools_protocol::*;
use instruction_hooking::disassemble_env;

use super::instruction_hooking::{hook_instruction, original_code, unhook_all, unhook_instruction};
use crate::{
	code_index::{self, Code},
	connection::{self, Connection},
	console, hot_reload, journal, resource_reload, unnamed, waker
};

mod breakpoints;
mod code;
mod eval;
mod journaling;
mod pause;
mod profile;
mod reload;
mod render;
mod sessions;
mod spawn;
mod values;
mod watches;

use breakpoints::Breakpoints;
use code::Codes;
pub(crate) use eval::{is_expression_frame, Evaluation};
use pause::{Pausing, State};
use reload::Reloads;
use sessions::{ClientId, Sessions, Wait};
use spawn::{Spawn, Spawns};
use values::Variables;
use watches::Watches;

pub struct Server {
	sessions: Sessions,
	pause: Pausing,
	breakpoints: Breakpoints,
	eval: Evaluation,
	spawns: Spawns,
	watches: Watches,
	reloads: Reloads,
	waiting_to_start: bool,
	codes: Codes
}

impl Server {
	pub fn listen() -> std::io::Result<Server> {
		Ok(Server {
			sessions: Sessions::listen()?,
			pause: Pausing::new(),
			breakpoints: Breakpoints::new(),
			eval: Evaluation::new(),
			spawns: Spawns::new(),
			watches: Watches::new(),
			reloads: Reloads::new(),
			waiting_to_start: false,
			codes: Codes::new()
		})
	}

	pub fn refuse(reason: String) {
		Sessions::refuse(reason);
	}

	pub const fn is_in_eval(&self) -> bool {
		self.eval.in_eval
	}

	pub fn eval_runs(&mut self, ctx: *mut raw_types::procs::ExecutionContext) {
		self.eval.eval_runs(ctx);
	}

	pub fn set_eval_error(&mut self, message: &str, ctx: *mut raw_types::procs::ExecutionContext) {
		self.eval.set_eval_error(message, ctx);
	}

	pub fn spawn_runtime(&mut self, message: &str, ctx: *mut raw_types::procs::ExecutionContext) {
		self.spawns.spawn_runtime(message, ctx);
	}

	pub fn spawn_inbox(&mut self) -> Value {
		self.spawns.spawn_inbox()
	}

	pub fn give_up(&mut self, reason: String) {
		self.sessions.give_up(reason);
	}

	fn handle_request(&mut self, request: Request) -> bool {
		auxtools::instruction_hook::hear_all();
		match request {
			Request::Disconnect => unreachable!(),
			Request::CatchRuntimes { uncaught, caught } => {
				if let Some(client) = self.sessions.current.and_then(|id| self.sessions.clients.get_mut(&id)) {
					client.uncaught = uncaught;
					client.caught = caught;
				}
			}
			Request::BreakpointSet {
				instruction,
				condition,
				hit,
				log,
				exact
			} => self.handle_breakpoint_set(instruction, condition, hit, log, exact),
			Request::BreakpointUnset { instruction, exact } => self.handle_breakpoint_unset(instruction, exact),
			Request::Stacks { sleeping } => self.handle_stacks(sleeping),
			Request::Scopes { frame_id } => self.handle_scopes(frame_id),
			Request::Variables { vars, start, count } => self.handle_variables(vars, start, count),
			Request::Complete { frame_id, base, prefix } => {
				let names = self.complete(frame_id, base.as_deref(), &prefix);
				self.answer(Response::Complete(names));
			}
			Request::SetVariable { vars, name, value } => {
				let result = self.set_variable(vars, &name, &value);
				self.send_callee_errors();
				self.answer(Response::SetVariable(result));
			}
			Request::Eval { frame_id, command, ending } => self.handle_eval_request(frame_id, &command, ending),

			Request::StackFrames {
				stack_id,
				start_frame,
				count
			} => self.handle_stack_frames(stack_id, start_frame, count),

			Request::Offset { proc, line } => {
				self.answer(Response::Offset {
					offset: self.codes.get_offset(proc, line)
				});
			}

			Request::Pause => {
				self.answer(Response::Ack);
				return true;
			}

			Request::ByondVersion => {
				let (version, build) = auxtools::version::get();
				self.answer(Response::ByondVersion { version, build });
			}

			Request::HotReload(request) => self.handle_hot_reload(&request),
			Request::Reloader { who } => {
				let claimed = self.sessions.claim_reloads(who);
				self.answer(Response::Reloader(claimed));
			}
			Request::Skin(request) => self.handle_skin(request),
			Request::Script(request) => self.handle_script(request),
			Request::Reconnect => {
				let reconnected = self.evaluating(crate::skin::reconnect);
				self.answer(Response::Reconnect(reconnected));
			}
			Request::Profile(request) => {
				let reply = self.profile(request);
				self.answer(Response::Profile(reply));
			}

			Request::Status => {
				let paused = self.pause.paused.clone();
				self.answer(Response::Status { paused });
			}

			Request::HoldPause => {
				self.sessions.hold = self.sessions.current;
				self.sessions.hold_max = self
					.sessions
					.current
					.and_then(|id| self.sessions.clients.get(&id))
					.and_then(|client| client.hold_max);
				self.answer(Response::Ack);
			}

			Request::Locate { file, line } => {
				let instructions = self.codes.locate(&file, line);
				self.answer(Response::Locate { instructions });
			}

			Request::LinesWithCode { file, start, end } => {
				let lines = Codes::lines_with_code(&file, start, end);
				self.answer(Response::LinesWithCode { lines });
			}

			Request::WatchInfo { place } => {
				let info = self.watch_info(&place);
				self.answer(Response::WatchInfo(info));
			}

			Request::WatchSet { place, condition, hit } => {
				let set = self.watch_set(&place, condition, hit);
				self.answer(Response::WatchSet(set));
			}

			Request::WatchUnset { id } => {
				self.watch_unset(id);
				self.answer(Response::Ack);
			}

			Request::Files => {
				let mut files: Vec<String> = code_index::by_file().keys().cloned().collect();
				files.sort();
				self.answer(Response::Files { files });
			}

			Request::WorldInfo => {
				let (args, cwd) = crate::launched::command_line();
				let port = Value::WORLD.get_number(byond_string!("port")).map_or(0, |port| port as u16);
				self.answer(Response::WorldInfo { args, cwd, port });
			}

			Request::StdDef => {
				let stddef = crate::stddef::get_stddef().map(|x| x.to_string());
				self.answer(Response::StdDef(stddef));
			}

			Request::Watcher => {
				if let Some(client) = self.sessions.current.and_then(|id| self.sessions.clients.get_mut(&id)) {
					client.debugs = false;
				}
				self.answer(Response::Ack);
			}

			Request::Disassemble { code } => {
				let disassembly = self.codes.disassemble(code);
				self.answer(Response::Disassemble(disassembly));
			}

			Request::Journal { after, kinds, limit, follow } => self.handle_journal(after, kinds, limit, follow),
			Request::JournalAdd(event) => self.journal(event, self.sessions.current),
			Request::HoldMax { seconds } => {
				let hold_max = seconds
					.filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
					.map(Duration::from_secs_f64);
				if let Some(client) = self.sessions.current.and_then(|id| self.sessions.clients.get_mut(&id)) {
					client.hold_max = hold_max;
				}
				self.answer(Response::Ack);
			}
			Request::Uptime => {
				let (loaded, up, loads, last_reload) = journal::uptime();
				self.answer(Response::Uptime {
					loaded,
					up,
					world_time: journal::world_time(),
					loads,
					last_reload,
					started: !self.waiting_to_start
				});
			}
			Request::ProcInfo { proc } => {
				let info = Codes::proc_info(&proc);
				self.answer(Response::ProcInfo(info));
			}

			Request::Render { frame_id, expression, depth } => self.handle_render(frame_id, &expression, depth),
			Request::RenderVariables { vars, depth } => self.handle_render_variables(vars, depth),
			Request::EvalOn { object, expression, depth } => self.handle_eval_on(&object, &expression, depth),
			Request::Defines(macros) => {
				crate::macros::set(macros);
				hot_reload::code_changed();
			}
			Request::Spawn { expression, object } => self.handle_spawn(&expression, object.as_deref()),

			Request::Configured | Request::Continue { .. } => {
				self.answer(Response::Ack);
			}
		}

		false
	}

	pub fn recover_from_panic(&mut self) {
		eprintln!("Debug server panicked, dropping the debug clients");
		self.eval.in_eval = false;
		self.waiting_to_start = false;
		self.pause.state = None;
		self.pause.paused = None;
		self.disconnect_all();
		self.sessions.profiling = None;
		crate::profiler::discard();
	}

	pub fn process(&mut self) -> bool {
		let mut should_pause = false;

		waker::alive();
		if waker::woke() {
			self.notify("Waking world...");
		}

		if waker::events_pending() {
			while let Some(request) = self.recv_request(Wait::No) {
				should_pause |= self.handle_request(request);
			}
		}

		should_pause
	}

	pub fn process_until_configured(&mut self) {
		self.waiting_to_start = true;
		self.reloads.resource_ids.clear();
		self.reloads.resource_origins.clear();
		while let Some(request) = self.recv_request(Wait::Forever) {
			if let Request::Configured = request {
				self.answer(Response::Ack);
				break;
			}

			self.handle_request(request);
		}
		self.waiting_to_start = false;
	}
}

#[cfg(test)]
mod tests {
	use std::{sync::mpsc, thread, time::Duration};

	use auxtools_protocol::{Hello, Response, ServerMessage};

	use super::{Connection, Server};
	use crate::connection;

	#[test]
	fn drop_hangs_up_on_clients_and_the_next_server_takes_new_ones() {
		let (done, dropped) = mpsc::channel();

		thread::spawn(move || {
			let mut server = Server::listen().unwrap();
			let first = Connection::connect(&connection::world_name()).unwrap();
			let second = Connection::connect(&connection::world_name()).unwrap();
			thread::sleep(Duration::from_millis(200));
			assert!(server.recv_request(super::Wait::No).is_none());
			assert_eq!(server.sessions.clients.len(), 2);

			drop(server);
			let _ = done.send((first, second));
		});

		let (mut first, mut second) = dropped
			.recv_timeout(Duration::from_secs(5))
			.expect("server drop blocked on client connections");
		for client in [&mut first, &mut second] {
			let hello: Hello = auxtools_protocol::read(client).expect("no hello");
			assert_eq!(hello, Hello::new(env!("CARGO_PKG_VERSION")));
			let told: ServerMessage = auxtools_protocol::read(client).expect("no Disconnect");
			assert!(
				matches!(told, ServerMessage {
					reply_to: None,
					response: Response::Disconnect
				}),
				"{:?}",
				told
			);
		}

		let _third = Connection::connect(&connection::world_name()).expect("listener unavailable between server instances");
		thread::sleep(Duration::from_millis(200));
		let mut server = Server::listen().expect("the next server couldn't listen");
		assert!(server.recv_request(super::Wait::No).is_none());
		assert_eq!(server.sessions.clients.len(), 1);
		assert!(Server::listen().is_err(), "multiple active servers accepted");
	}
}
