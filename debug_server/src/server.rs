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
mod pause;
mod profile;
mod render;
mod sessions;
mod spawn;
mod values;
mod watches;

use breakpoints::Breakpoints;
use code::Codes;
pub(crate) use eval::{is_expression_frame, Evaluation};
use pause::{Pausing, State};
use sessions::{ClientId, Sessions, Wait};
use spawn::{Spawn, Spawns};
use values::Variables;
use watches::Watches;

const JOURNAL_FRAMES: usize = 64;

pub struct Server {
	sessions: Sessions,
	pause: Pausing,
	breakpoints: Breakpoints,
	eval: Evaluation,
	spawns: Spawns,
	watches: Watches,
	waiting_to_start: bool,
	pending_skin: Option<SkinRequest>,
	pending_script: Option<ScriptRequest>,
	resource_ids: HashMap<String, Vec<u32>>,
	resource_origins: HashMap<String, Vec<u32>>,
	codes: Codes,
	waiting_reloads: Vec<HotReloadRequest>
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
			waiting_to_start: false,
			pending_skin: None,
			pending_script: None,
			resource_ids: HashMap::new(),
			resource_origins: HashMap::new(),
			codes: Codes::new(),
			waiting_reloads: vec![]
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

	fn apply_pending_skin(&mut self) {
		let Some(request) = self.pending_skin.take() else {
			return;
		};
		if let Err(e) = self.evaluating(|| crate::skin::set(&request)) {
			self.notify(format!("could not restore skin {} after reboot: {}", request.path, e));
		}
	}

	fn apply_pending_script(&mut self) {
		let Some(request) = self.pending_script.take() else {
			return;
		};
		if let Err(e) = self.evaluating(|| crate::client_script::set(&request)) {
			self.notify(format!("could not restore client script after reboot: {}", e));
		}
	}

	fn handle_hot_reload(&mut self, request: &HotReloadRequest) {
		if let Some(why) = self.sessions.reloads_refused() {
			let refused = HotReloadResult {
				applied: false,
				errors: vec![why],
				warnings: vec![],
				skipped: vec![],
				resources: vec![]
			};
			self.answer(Response::HotReload(refused));
			return;
		}
		let result = self.hot_reload(request);
		self.answer(Response::HotReload(result));
	}

	fn hot_reload(&mut self, request: &HotReloadRequest) -> HotReloadResult {
		resource_reload::use_dirs(&request.resource_dirs);
		if !crate::world_resources::is_loaded() {
			let mut files = crate::world_resources::not_built(request);
			for resource in &request.resources {
				if !files.contains(&resource.path) {
					files.push(resource.path.clone());
				}
			}
			if !files.is_empty() || !self.waiting_reloads.is_empty() {
				let mut waiting = request.clone();
				let mut early = vec![];
				if let Some(init) = &request.global_init {
					let (globals, others) = request.plan.new_vars.iter().cloned().partition(|var| var.type_path.is_empty());
					let (init, left) = crate::hot_reload::early_global_init(init, request);
					if left > 0 {
						early.push(format!(
							"(global init): {} new static vars will initialize with the deferred reload at world/New",
							left
						));
					}
					let alone = HotReloadRequest {
						global_init: Some(init),
						plan: dmasm::reload::Plan {
							new_vars: globals,
							..Default::default()
						},
						..Default::default()
					};
					if crate::world_resources::not_built(&alone).is_empty() {
						let result = self.apply_hot_reload(&alone);
						if result.applied {
							waiting.global_init = None;
							waiting.plan.new_vars = others;
						}
						early.extend(result.errors.into_iter().chain(result.warnings));
					}
				}
				self.waiting_reloads.push(waiting);
				let why = if files.is_empty() {
					"queued behind an earlier reload; applies as world/New starts".to_owned()
				} else {
					format!(
						"resource files need the loaded cache ({}); reload applies as world/New starts. Map atoms use the built code until then",
						files.join(", ")
					)
				};
				return HotReloadResult {
					applied: true,
					errors: vec![],
					warnings: std::iter::once(why).chain(early).collect(),
					skipped: vec![],
					resources: vec![]
				};
			}
		}
		self.apply_hot_reload(request)
	}

	fn apply_hot_reload(&mut self, request: &HotReloadRequest) -> HotReloadResult {
		let mut id_warnings = vec![];
		let mut with_ids;
		let request = if request.resources.is_empty() {
			request
		} else {
			with_ids = request.clone();
			let named: Vec<(String, Vec<u32>)> = request.resources.iter().map(|r| (r.path.clone(), r.ids.clone())).collect();
			for (path, ids) in &named {
				if !self.resource_ids.contains_key(path) {
					self.resource_origins.entry(path.clone()).or_insert_with(|| ids.clone());
				}
			}
			for resource in &mut with_ids.resources {
				if let Some(&last) = self.resource_ids.get(&resource.path).and_then(|ids| ids.last()) {
					resource.ids = vec![last];
				}
				let path = resource.path.clone();
				let owner = |id: u32| {
					self.resource_ids
						.iter()
						.chain(named.iter().map(|(p, ids)| (p, ids)))
						.find(|(p, ids)| **p != path && ids.contains(&id))
						.map(|(p, _)| p.clone())
				};
				let mut kept = vec![];
				for &id in &resource.ids {
					match owner(id) {
						Some(other) => id_warnings.push(format!(
							"resource reload: {} and {} share an ID (same bytes); appearances are unchanged. Rebuild to separate them",
							path, other
						)),
						None => kept.push(id)
					}
				}
				resource.ids = kept;
			}
			&with_ids
		};
		self.eval.in_eval = true;
		auxtools::instruction_hook::hear_all();
		self.eval.eval_error = None;

		let mut warnings = id_warnings;
		let mut skipped = vec![];
		let prepared = hot_reload::add_types(&request.plan.new_types).and_then(|added| {
			hot_reload::prepare(request, &mut warnings, &mut skipped)
				.and_then(|prepared| Ok((prepared, resource_reload::prepare(&request.resources)?)))
				.map_err(|mut errors| {
					if added > 0 {
						errors.push(format!("{} new types remain allocated but unused", added));
					}
					errors
				})
		});
		let prepared = prepared.and_then(|(prepared, resources)| Ok((prepared, resource_reload::apply(&resources).map_err(|e| vec![e])?)));
		let result = match prepared {
			Ok((prepared, applied)) => {
				hot_reload::apply(prepared, &mut self.breakpoints.claims, &mut self.breakpoints.moved, &mut warnings);
				warnings.extend(applied.warnings);
				if let Err(e) = resource_reload::resend(&applied.appearances) {
					warnings.push(format!("resource reload: could not send updated appearances to clients: {}", e));
				}
				self.record_resource_ids(&applied.reloaded, &mut warnings);
				self.record_named_files(request);
				HotReloadResult {
					applied: true,
					errors: vec![],
					warnings,
					skipped,
					resources: applied.reloaded
				}
			}

			Err(errors) => HotReloadResult {
				applied: false,
				errors,
				warnings,
				skipped,
				resources: vec![]
			}
		};

		self.eval.in_eval = false;
		if let Some(err) = self.eval.eval_error.take() {
			self.notify(format!("runtime error during hot reload: {}", err));
		}

		if result.applied && !request.is_empty() {
			self.broadcast(Response::CodeChanged, self.sessions.current);
		}

		result
	}

	fn record_named_files(&mut self, request: &HotReloadRequest) {
		use dmasm::assembler::AssembleEnv as _;
		for name in crate::world_resources::not_built(request) {
			let Some(file) = crate::assemble_env::resource_file(&name) else {
				continue;
			};
			let path = file.to_string_lossy().replace('\\', "/");
			if self.resource_ids.contains_key(&path) || crate::world_resources::built_file(&file) {
				continue;
			}
			if let Some(id) = crate::assemble_env::AssembleEnv.get_resource_index(&name) {
				self.resource_ids.insert(path, vec![id]);
			}
		}
	}

	fn record_resource_ids(&mut self, reloaded: &[ReloadedResource], warnings: &mut Vec<String>) {
		let built = crate::world_resources::built_count();
		for resource in reloaded {
			let own = self.resource_origins.get(&resource.path).is_some_and(|ids| ids.contains(&resource.id));
			let shared = (!own && built.is_some_and(|count| (resource.id as usize) < count))
				|| self
					.resource_ids
					.iter()
					.any(|(path, ids)| *path != resource.path && ids.contains(&resource.id));
			if shared {
				self.resource_ids.remove(&resource.path);
				warnings.push(format!(
					"resource reload: {} shares existing resource {} (same bytes); later edits to {} will leave appearances using that shared ID \
					 unchanged",
					resource.path, resource.id, resource.path
				));
			} else {
				let ids = self.resource_ids.entry(resource.path.clone()).or_default();
				ids.retain(|&id| id != resource.id);
				ids.push(resource.id);
			}
		}
	}

	pub fn world_data_loaded(&mut self) {
		self.apply_pending_script();
		for request in std::mem::take(&mut self.waiting_reloads) {
			let result = self.hot_reload(&request);
			if !result.applied {
				self.notify(format!("Hot reload failed as world/New started: {}", result.errors.join("; ")));
			}
			for warning in result.warnings {
				self.notify(format!("Hot reload: {}", warning));
			}
		}
		self.apply_pending_skin();
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
			Request::Skin(request) => {
				let result = match self.sessions.reloads_refused() {
					Some(why) => Err(why),
					None if !crate::world_resources::is_loaded() => {
						self.pending_skin = Some(request);
						Ok(SkinReloaded { clients: 0, hosted: false })
					}
					None => self.evaluating(|| crate::skin::set(&request))
				};
				self.answer(Response::Skin(result));
			}
			Request::Script(request) => {
				let result = match self.sessions.reloads_refused() {
					Some(why) => Err(why),
					None if !crate::world_resources::is_loaded() => {
						self.pending_script = Some(request);
						Ok(ScriptReloaded { clients: 0, hosted: false })
					}
					None => self.evaluating(|| crate::client_script::set(&request))
				};
				self.answer(Response::Script(result));
			}
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
		self.resource_ids.clear();
		self.resource_origins.clear();
		while let Some(request) = self.recv_request(Wait::Forever) {
			if let Request::Configured = request {
				self.answer(Response::Ack);
				break;
			}

			self.handle_request(request);
		}
		self.waiting_to_start = false;
	}

	fn journal(&mut self, event: JournalEvent, from: Option<ClientId>) {
		let entry = journal::add(event);
		let mut gone = vec![];
		for (&id, client) in &mut self.sessions.clients {
			let follows = client.follow.as_ref().is_some_and(|kinds| journal::wanted(&entry, kinds));
			if follows && Some(id) != from && !Sessions::deliver(client, &Response::Journaled(entry.clone())) {
				gone.push(id);
			}
		}
		for id in gone {
			self.remove_client(id);
		}
	}

	fn journal_stack(&self) -> Vec<JournalFrame> {
		type Known = HashMap<(raw_types::procs::ProcId, u16, usize), JournalFrame>;
		static KNOWN: Mutex<(u64, Option<Known>)> = Mutex::new((0, None));
		const KEEP: usize = 16384;

		let mut uncached: (u64, Option<Known>) = (0, None);
		let mut guard = match KNOWN.try_lock() {
			Ok(guard) => Some(guard),
			Err(std::sync::TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
			Err(std::sync::TryLockError::WouldBlock) => None
		};
		let (changes, known) = match guard.as_mut() {
			Some(guard) => &mut **guard,
			None => &mut uncached
		};
		let now = auxtools::bytecode_changes();
		if *changes != now || known.as_ref().is_some_and(|known| known.len() >= KEEP) {
			*changes = now;
			*known = None;
		}
		let known = known.get_or_insert_with(HashMap::new);

		let mut frames: Vec<(JournalFrame, bool)> = vec![];
		let mut ctx = unsafe { *raw_types::funcs::CURRENT_EXECUTION_CONTEXT };
		while !ctx.is_null() {
			let (id, offset, running) = unsafe { ((**(*ctx).proc_instance()).proc, *(*ctx).bytecode_offset(), *(*ctx).bytecode()) };
			let Some(proc) = Proc::from_id(id) else {
				break;
			};
			let helper = crate::launched::runs_spawns(&proc);
			let (current, length) = unsafe { proc.bytecode_mut_ptr() };
			let breakpoint =
				(offset as u32) < length as u32 && crate::instruction_hooking::is_breakpoint_word(unsafe { *current.add(offset as usize) });
			let plain = std::ptr::eq(running, current) && !breakpoint;
			let key = (id, offset, running as usize);
			let frame = match known.get(&key).filter(|_| plain) {
				Some(frame) => frame.clone(),
				None => {
					let (file, line) = Codes::place_running(&proc, ctx, offset, false);
					let frame = JournalFrame {
						proc: unnamed::proc_name(&proc),
						file,
						line
					};
					if plain && !frame.proc.starts_with('(') {
						known.insert(key, frame.clone());
					}
					frame
				}
			};
			frames.push((frame, helper));
			ctx = unsafe { *(*ctx).parent_context() };
		}

		let shown = match frames.iter().rposition(|(_, helper)| !helper) {
			Some(last) => last + 1,
			None => frames.len()
		};
		frames.truncate(shown.min(JOURNAL_FRAMES));
		frames.into_iter().map(|(frame, _)| frame).collect()
	}

	fn handle_journal(&mut self, after: u64, kinds: Vec<JournalKind>, limit: Option<u32>, follow: bool) {
		let (entries, first, last) = journal::read(after, &kinds, limit);
		if let Some(client) = self.sessions.current.and_then(|id| self.sessions.clients.get_mut(&id)) {
			client.follow = follow.then_some(kinds);
		}
		self.answer(Response::Journal { entries, first, last });
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
