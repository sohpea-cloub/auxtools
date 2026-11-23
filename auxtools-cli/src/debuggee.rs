use std::{
	collections::HashMap,
	io,
	ops::ControlFlow,
	sync::{
		atomic::{AtomicBool, AtomicU32, Ordering},
		mpsc, Arc, Mutex
	},
	time::{Duration, Instant}
};

use auxtools_cli::{
	client::{ops::Ending, Client, Heard, Listener, Lost},
	local::{self, Stream}
};
use auxtools_protocol::{BreakpointKind, BreakpointReason, DataChange, InstructionRef, Request, Response, Stop};
use dap_types::{ContinuedEvent, StoppedEvent as Reasons};

use crate::{
	adapter::Input,
	dap::{self, Output}
};

#[derive(Default, Clone)]
pub struct LastError {
	pub message: String,
	pub caught: bool
}

pub const fn error_kind(caught: bool) -> &'static str {
	if caught {
		"caught runtime error"
	} else {
		"runtime error"
	}
}

#[derive(Clone)]
pub struct SetBreakpoint {
	pub asked: InstructionRef,
	pub at: Option<InstructionRef>,
	pub watch: Option<u32>,
	pub id: i64
}

impl SetBreakpoint {
	const fn kind(&self, exact: bool) -> BreakpointKind {
		match self.asked.offset {
			_ if exact => BreakpointKind::Instruction,
			0 => BreakpointKind::Proc,
			_ => BreakpointKind::Line
		}
	}
}

fn changes_text(changes: &[DataChange]) -> String {
	changes
		.iter()
		.map(|change| format!("{} changed from {} to {}", change.name, change.old, change.new))
		.collect::<Vec<_>>()
		.join("\n")
}

pub const INSTRUCTION_BREAKPOINTS: &str = "<instructions>";

pub type SetBreakpoints = Arc<Mutex<HashMap<String, Vec<SetBreakpoint>>>>;

fn announce_stop(out: &Output, last_error: &Mutex<LastError>, set: &SetBreakpoints, stop: Stop) {
	let mut description = None;
	let mut text = None;
	let mut hit = vec![];
	let reason = match stop.reason {
		BreakpointReason::Step => Reasons::REASON_STEP,
		BreakpointReason::Pause => Reasons::REASON_PAUSE,
		BreakpointReason::Breakpoint => {
			let mut by_instruction = true;
			for (key, breakpoints) in set.lock().unwrap().iter() {
				let exact = key == INSTRUCTION_BREAKPOINTS;
				let paused =
					|breakpoint: &&SetBreakpoint| breakpoint.at.as_ref() == Some(&stop.at) && stop.paused_by.contains(&breakpoint.kind(exact));
				for breakpoint in breakpoints.iter().filter(paused) {
					hit.push(breakpoint.id);
					by_instruction &= exact;
				}
			}
			hit.sort_unstable();
			if by_instruction && !hit.is_empty() {
				"instruction breakpoint"
			} else {
				Reasons::REASON_BREAKPOINT
			}
		}
		BreakpointReason::Runtime { message, caught } => {
			*last_error.lock().unwrap() = LastError {
				message: message.clone(),
				caught
			};
			description = Some(message);
			text = Some(error_kind(caught).to_owned());
			Reasons::REASON_EXCEPTION
		}
		BreakpointReason::Data { changes } => {
			hit = set
				.lock()
				.unwrap()
				.values()
				.flatten()
				.filter(|breakpoint| breakpoint.watch.is_some_and(|watch| changes.iter().any(|change| change.id == watch)))
				.map(|breakpoint| breakpoint.id)
				.collect();
			hit.sort_unstable();
			out.console(changes_text(&changes));
			description = changes
				.first()
				.map(|change| format!("{} changed: {} → {}", change.name, change.old, change.new));
			"data breakpoint"
		}
	};
	out.event(dap::StoppedEvent {
		reason: reason.to_owned(),
		description,
		text,
		threadId: stop.thread.into(),
		allThreadsStopped: true,
		hitBreakpointIds: hit
	});
}

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

const GOODBYE_TIMEOUT: Duration = Duration::from_secs(2);

const LEAVING_TIMEOUT: Duration = Duration::from_secs(2);

const LEAVING_LOOK: Duration = Duration::from_millis(50);

struct ToAdapter {
	generation: u64,
	debugging: bool,
	out: Arc<Output>,
	inputs: mpsc::Sender<Input>,
	ended: Option<&'static str>
}

impl ToAdapter {
	fn input(&mut self, number: u64, event: Response) -> ControlFlow<(), Option<Input>> {
		let generation = self.generation;
		ControlFlow::Continue(Some(match event {
			Response::Disconnect => {
				self.ended = Some("the debug server disconnected");
				return ControlFlow::Break(());
			}

			Response::Unavailable { reason } => {
				let _ = self.inputs.send(Input::Unavailable { generation, reason });
				self.ended = Some("debugger initialization failed");
				return ControlFlow::Break(());
			}

			Response::ShuttingDown => Input::ShuttingDown { generation },

			Response::Notification { message } | Response::ConditionFailed { message } => {
				self.out.console(message);
				return ControlFlow::Continue(None);
			}

			Response::BreakpointHit { .. } | Response::Resumed if !self.debugging => return ControlFlow::Continue(None),

			Response::BreakpointHit(stop) => Input::Stopped { generation, stop, number },

			Response::Log { message, file, line, .. } => Input::Logged { message, file, line },

			Response::WatchEnded { id, reason } => Input::WatchEnded { generation, id, reason },

			Response::CodeChanged if self.debugging => Input::CodeChanged { generation },

			Response::Journaled(entry) => {
				if let Some(text) = crate::adapter::reload_by_another(&entry) {
					self.out.console(text);
				}
				return ControlFlow::Continue(None);
			}

			Response::Resumed => Input::Resumed { generation },

			Response::StepEnded => return ControlFlow::Continue(None),

			_ => return ControlFlow::Continue(None)
		}))
	}
}

impl Listener for ToAdapter {
	fn event(&mut self, number: u64, event: Response) -> ControlFlow<()> {
		if let Some(input) = self.input(number, event)? {
			if self.inputs.send(input).is_err() {
				self.ended = Some("adapter event receiver closed");
				return ControlFlow::Break(());
			}
		}
		ControlFlow::Continue(())
	}

	fn closed(self: Box<Self>, error: Option<io::Error>) {
		let reason = match (self.ended, error) {
			(Some(reason), _) => reason.to_owned(),
			(None, Some(error)) => format!("the connection closed: {}", error),
			(None, None) => "response receiver closed".to_owned()
		};
		let _ = self.inputs.send(Input::ConnectionClosed {
			generation: self.generation,
			reason
		});
	}
}

fn lost(lost: Lost) -> String {
	match lost {
		Lost::Unsent(e) => format!("couldn't reach the world: {}", e),
		Lost::Closed => "the world disconnected".to_owned(),
		Lost::Incompatible(theirs) => local::incompatible(theirs).to_string()
	}
}

pub struct Debuggee {
	client: Client,
	stopped: Arc<AtomicBool>,
	out: Arc<Output>,
	last_error: Arc<Mutex<LastError>>,
	set: SetBreakpoints,
	thread: Arc<AtomicU32>,
	caught_up: bool,
	stops: u32,
	resumed_after: u64,
	pub generation: u64
}

impl Debuggee {
	pub fn new(
		stream: Stream,
		generation: u64,
		debugging: bool,
		out: Arc<Output>,
		inputs: mpsc::Sender<Input>,
		last_error: Arc<Mutex<LastError>>,
		set: SetBreakpoints
	) -> Result<Self, String> {
		let listener = ToAdapter {
			generation,
			debugging,
			out: out.clone(),
			inputs,
			ended: None
		};
		Ok(Self {
			client: Client::listened_to(stream, listener).map_err(|e| e.to_string())?,
			stopped: Arc::new(AtomicBool::new(false)),
			out,
			last_error,
			set,
			thread: Arc::new(AtomicU32::new(0)),
			caught_up: false,
			stops: 0,
			resumed_after: 0,
			generation
		})
	}

	pub fn stopped_at(&mut self, stop: Stop, number: u64) {
		if self.caught_up || number <= self.resumed_after {
			return;
		}
		self.stopped.store(true, Ordering::Relaxed);
		self.stops += 1;
		self.thread.store(stop.thread, Ordering::Relaxed);
		announce_stop(&self.out, &self.last_error, &self.set, stop);
	}

	pub fn resumed(&mut self) {
		self.caught_up = false;
		self.stopped.store(false, Ordering::Relaxed);
		self.continued();
	}

	pub fn gone(&mut self) {
		if self.stopped.swap(false, Ordering::Relaxed) {
			self.continued();
		}
	}

	fn continued(&self) {
		self.out.event(ContinuedEvent {
			threadId: self.thread.load(Ordering::Relaxed).into(),
			allThreadsContinued: Some(true)
		});
	}

	pub fn catch_up_on_pause(&mut self) -> Result<(), String> {
		let Response::Status { paused: Some(stop) } = self.request(&Request::Status, None)? else {
			return Ok(());
		};
		if !self.stopped.swap(true, Ordering::Relaxed) {
			self.caught_up = true;
			self.stops += 1;
			self.thread.store(stop.thread, Ordering::Relaxed);
			announce_stop(&self.out, &self.last_error, &self.set, stop);
		}
		Ok(())
	}

	pub fn stop_number(&self) -> u32 {
		self.stops
	}

	pub fn stopped(&self) -> bool {
		self.stopped.load(Ordering::Relaxed)
	}

	pub fn leave(&mut self) {
		if self.client.leave().is_ok() {
			self.client.closed_by(Instant::now() + GOODBYE_TIMEOUT);
		}
	}

	pub fn send(&mut self, request: &Request) -> Result<(), String> {
		self.client.tell(request, None).map(|_| ()).map_err(lost)
	}

	pub fn end(&mut self, ending: Ending) -> Result<(), String> {
		self.send(&ending.request())
	}

	pub fn request(&mut self, request: &Request, leaving: Option<&AtomicBool>) -> Result<Response, String> {
		let resumes = matches!(request, Request::Continue { .. });
		if resumes {
			self.caught_up = false;
			self.stopped.store(false, Ordering::Relaxed);
		}
		let sent = Instant::now();
		let deadline = sent + RESPONSE_TIMEOUT;
		let Some(id) = self.client.ask(request, Some(deadline)).map_err(lost)? else {
			return Err("debug server response timed out".to_owned());
		};
		loop {
			let left = leaving.is_some_and(|leaving| leaving.load(Ordering::Relaxed));
			let until = match leaving {
				Some(_) if left => deadline.min(sent + LEAVING_TIMEOUT),
				Some(_) => deadline.min(Instant::now() + LEAVING_LOOK),
				None => deadline
			};
			match self.client.hear(Some(until)).map_err(lost)? {
				Some(Heard::Answer { to, response, after }) if to == id => {
					if resumes && matches!(response, Response::Ack) {
						self.resumed_after = after;
					}
					return Ok(response);
				}
				Some(_) => {}
				None if !left && Instant::now() < deadline => {}
				None => {
					self.client.give_up(id);
					return Err(if Instant::now() < deadline {
						"stopped waiting for the debug server: the client asked to disconnect".to_owned()
					} else {
						"debug server response timed out".to_owned()
					});
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use std::{
		sync::{
			atomic::{AtomicBool, Ordering},
			mpsc, Arc
		},
		time::{Duration, Instant}
	};

	use auxtools_cli::{client::ops::Ending, local::Stream};
	use auxtools_protocol::{ClientMessage, EvalError, Request, Response, ServerMessage};
	use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions};

	use super::{Debuggee, LEAVING_TIMEOUT, RESPONSE_TIMEOUT};
	use crate::dap::Output;

	fn read_request(stream: &mut Stream) -> ClientMessage {
		auxtools_protocol::read(stream).unwrap()
	}

	fn answer(stream: &mut Stream, to: &ClientMessage, response: &Response) {
		auxtools_protocol::write(stream, &ServerMessage::answer(to.id, response)).unwrap();
	}

	#[test]
	fn an_answer_nobody_waited_for_isnt_taken_for_the_next() {
		let name = format!("auxtools-cli-test-{}", std::process::id());
		let listener = ListenerOptions::new()
			.name(name.clone().to_ns_name::<GenericNamespaced>().unwrap())
			.create_sync()
			.unwrap();
		let client = Stream::connect(name.to_ns_name::<GenericNamespaced>().unwrap()).unwrap();
		let mut world = listener.accept().unwrap();

		let (inputs, _heard) = mpsc::channel();
		let mut debuggee = Debuggee::new(client, 1, true, Arc::new(Output::new()), inputs, Default::default(), Default::default()).unwrap();
		debuggee.end(Ending::Stop).unwrap();
		let answering = std::thread::spawn(move || {
			let eval = read_request(&mut world);
			assert!(matches!(eval.request, Request::Eval { .. }));
			let pause = read_request(&mut world);
			assert!(matches!(pause.request, Request::Pause));
			assert_ne!(eval.id, pause.id);
			let slept = EvalError {
				message: "world/Del() slept".to_owned(),
				still_running: true
			};
			answer(&mut world, &eval, &Response::Eval(Err(slept)));
			answer(&mut world, &pause, &Response::Ack);
			let status = read_request(&mut world);
			assert!(matches!(status.request, Request::Status));
			answer(&mut world, &eval, &Response::Ack);
			auxtools_protocol::write(&mut world, &ServerMessage::event(&Response::Ack)).unwrap();
			answer(&mut world, &status, &Response::Status { paused: None });
			world
		});
		let answer = debuggee.request(&Request::Pause, None).unwrap();
		assert!(matches!(answer, Response::Ack), "{:?}", answer);
		let answer = debuggee.request(&Request::Status, None).unwrap();
		assert!(matches!(answer, Response::Status { paused: None }), "{:?}", answer);
		answering.join().unwrap();
	}

	#[test]
	fn a_request_is_not_waited_out_once_the_client_asks_to_disconnect() {
		let name = format!("auxtools-cli-test-leaving-{}", std::process::id());
		let listener = ListenerOptions::new()
			.name(name.clone().to_ns_name::<GenericNamespaced>().unwrap())
			.create_sync()
			.unwrap();
		let client = Stream::connect(name.to_ns_name::<GenericNamespaced>().unwrap()).unwrap();
		let mut world = listener.accept().unwrap();

		let (inputs, _heard) = mpsc::channel();
		let mut debuggee = Debuggee::new(client, 1, true, Arc::new(Output::new()), inputs, Default::default(), Default::default()).unwrap();
		let leaving = Arc::new(AtomicBool::new(false));
		let asked = {
			let leaving = leaving.clone();
			std::thread::spawn(move || {
				let eval = read_request(&mut world);
				assert!(matches!(eval.request, Request::Eval { .. }));
				std::thread::sleep(Duration::from_millis(200));
				leaving.store(true, Ordering::Relaxed);
				(world, eval)
			})
		};
		let started = Instant::now();
		let stuck = Request::Eval {
			frame_id: None,
			command: "forever()".to_owned(),
			ending: false
		};
		let error = debuggee.request(&stuck, Some(&leaving)).unwrap_err();
		assert_eq!(error, "stopped waiting for the debug server: the client asked to disconnect");
		let waited = started.elapsed();
		assert!(waited >= LEAVING_TIMEOUT && waited < LEAVING_TIMEOUT * 3, "{:?}", waited);

		let (mut world, eval) = asked.join().unwrap();
		let answering = std::thread::spawn(move || {
			let status = read_request(&mut world);
			let done = EvalError {
				message: "done".to_owned(),
				still_running: false
			};
			answer(&mut world, &eval, &Response::Eval(Err(done)));
			answer(&mut world, &status, &Response::Status { paused: None });
			world
		});
		let answer = debuggee.request(&Request::Status, Some(&leaving)).unwrap();
		assert!(matches!(answer, Response::Status { paused: None }), "{:?}", answer);
		assert!(started.elapsed() < RESPONSE_TIMEOUT);
		answering.join().unwrap();
	}
}
