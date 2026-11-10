//! The connection to the debug server in the world. Requests go out from the
//! adapter's thread, which waits for their responses. A thread reads
//! everything that comes back and passes on what the server says on its own:
//! the world pausing, for this client or another, and carrying on. Those go
//! through the adapter's thread, so the client hears of them after the
//! response to the request being handled: VS Code takes the response to a
//! step to mean the world is running, even if it's paused again already.

use std::{
	collections::HashMap,
	sync::{
		atomic::{AtomicBool, AtomicU32, Ordering},
		mpsc, Arc, Mutex
	},
	time::Duration
};

use dap_types::{ContinuedEvent, StoppedEvent as Reasons};
use auxtools_cli::{
	local::{self, Shared, Stream},
	server_types::{BreakpointReason, DataChange, InstructionRef, Request, Response, Stop}
};

use crate::{
	adapter::Input,
	dap::{self, Output}
};

/// The runtime error the world last stopped on
#[derive(Default, Clone)]
pub struct LastError {
	pub message: String,
	/// Whether it's inside a `try`
	pub caught: bool
}

/// What a runtime error is called, by whether a `try` will catch it
pub const fn error_kind(caught: bool) -> &'static str {
	if caught {
		"caught runtime error"
	} else {
		"runtime error"
	}
}

/// Tells the client the world stopped, and why
/// A breakpoint the launcher set in the world
#[derive(Clone)]
pub struct SetBreakpoint {
	/// What to take it away with
	pub asked: InstructionRef,
	/// Where the world says it is, if it took it
	pub at: Option<InstructionRef>,
	/// For a data breakpoint, the world's watch (and nothing in the two
	/// above that's any place)
	pub watch: Option<u32>,
	/// What the client knows it as
	pub id: i64
}

/// What changed at the places a stop is for, a line each
fn changes_text(changes: &[DataChange]) -> String {
	changes
		.iter()
		.map(|change| format!("{} changed from {} to {}", change.name, change.old, change.new))
		.collect::<Vec<_>>()
		.join("\n")
}

/// Where the key for instruction breakpoints' records is
pub const INSTRUCTION_BREAKPOINTS: &str = "<instructions>";

/// The breakpoints set in the world now, by file (or `FUNCTION_BREAKPOINTS`,
/// or `INSTRUCTION_BREAKPOINTS`)
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
				for breakpoint in breakpoints.iter().filter(|breakpoint| breakpoint.at.as_ref() == Some(&stop.at)) {
					hit.push(breakpoint.id);
					by_instruction &= key == INSTRUCTION_BREAKPOINTS;
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
			// The stop itself has room for a line; the console has it all
			out.console(changes_text(&changes));
			description = changes.first().map(|change| format!("{} changed: {} → {}", change.name, change.old, change.new));
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

/// How long a request can go unanswered. The server answers between DM
/// instructions, and the world runs at least one a tick.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

/// How long to wait for the server's goodbye. It answers between DM
/// instructions, so a world that's busy takes a moment.
const GOODBYE_TIMEOUT: Duration = Duration::from_secs(2);

pub struct Debuggee {
	writer: Shared,
	responses: mpsc::Receiver<Response>,
	/// Whether the world is paused
	stopped: Arc<AtomicBool>,
	out: Arc<Output>,
	last_error: Arc<Mutex<LastError>>,
	set: SetBreakpoints,
	/// The stack the world last stopped in
	thread: Arc<AtomicU32>,
	/// The client was told of a pause that was there when this connected,
	/// and the world hasn't carried on since: the server may have said so
	/// too, which isn't news
	caught_up: bool,
	/// How many times the world has stopped
	stops: u32,
	/// How many evaluations were sent without waiting for their answers,
	/// which aren't to be taken for other requests'
	unanswered: Arc<AtomicU32>,
	/// Tells this connection's events from an earlier one's
	pub generation: u64
}

impl Debuggee {
	/// Starts reading what the server sends. `inputs` hears when the
	/// connection closes. Without `debugging`, the world pausing (for another
	/// client) isn't passed on.
	pub fn new(
		stream: Stream,
		generation: u64,
		debugging: bool,
		out: Arc<Output>,
		inputs: mpsc::Sender<Input>,
		last_error: Arc<Mutex<LastError>>,
		set: SetBreakpoints
	) -> Result<Self, String> {
		let writer = Shared::new(stream);
		let mut reader = writer.clone();
		let debuggee_out = out.clone();
		let (responses_sender, responses) = mpsc::channel();
		let unanswered = Arc::new(AtomicU32::new(0));
		let reader_unanswered = unanswered.clone();

		std::thread::Builder::new()
			.name("debuggee reader".to_owned())
			.spawn(move || {
				let reason = loop {
					let response = match local::receive(&mut reader) {
						Ok(response) => response,
						Err(e) => break format!("the connection closed: {}", e)
					};

					match response {
						Response::Disconnect => break "the debug server disconnected".to_owned(),

						Response::Notification { message } => out.console(format!("[auxtools] {}", message)),

						// A breakpoint's condition failed: said once, and the
						// world doesn't stop for it
						Response::ConditionFailed { message } => out.console(format!("[auxtools] {}", message)),

						Response::BreakpointHit { .. } | Response::Resumed if !debugging => {}

						Response::BreakpointHit(stop) => {
							if inputs.send(Input::Stopped { generation, stop }).is_err() {
								break "nobody was listening".to_owned();
							}
						}

						// A breakpoint with a message
						Response::Log { message, file, line } => {
							if inputs.send(Input::Logged { message, file, line }).is_err() {
								break "nobody was listening".to_owned();
							}
						}

						// What a data breakpoint watched is gone
						Response::WatchEnded { id, reason } => {
							if inputs.send(Input::WatchEnded { generation, id, reason }).is_err() {
								break "nobody was listening".to_owned();
							}
						}

						// Another client reloaded its code
						Response::CodeChanged => {
							if debugging && inputs.send(Input::CodeChanged { generation }).is_err() {
								break "nobody was listening".to_owned();
							}
						}

						// A hot reload another client reported, for a launcher
						// attached to a world it didn't start
						Response::Journaled(entry) => {
							if let Some(text) = crate::adapter::reload_by_another(&entry) {
								out.console(text);
							}
						}

						// Another client let it carry on
						Response::Resumed => {
							if inputs.send(Input::Resumed { generation }).is_err() {
								break "nobody was listening".to_owned();
							}
						}

						// The answer to an evaluation nobody waited for: the
						// server answers in order, so it's the first
						Response::Eval(_)
							if reader_unanswered
								.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
								.is_ok() => {}

						response => {
							if responses_sender.send(response).is_err() {
								break "nobody was waiting for responses".to_owned();
							}
						}
					}
				};
				let _ = inputs.send(Input::ConnectionClosed { generation, reason });
			})
			.map_err(|e| e.to_string())?;

		Ok(Self {
			writer,
			responses,
			stopped: Arc::new(AtomicBool::new(false)),
			out: debuggee_out,
			last_error,
			set,
			thread: Arc::new(AtomicU32::new(0)),
			caught_up: false,
			stops: 0,
			unanswered,
			generation
		})
	}

	/// The world stopped (`Input::Stopped`): the client is told
	pub fn stopped_at(&mut self, stop: Stop) {
		if self.caught_up {
			return;
		}
		self.stopped.store(true, Ordering::Relaxed);
		self.stops += 1;
		self.thread.store(stop.thread, Ordering::Relaxed);
		announce_stop(&self.out, &self.last_error, &self.set, stop);
	}

	/// Another client let the world carry on (`Input::Resumed`)
	pub fn resumed(&mut self) {
		self.caught_up = false;
		self.stopped.store(false, Ordering::Relaxed);
		self.continued();
	}

	/// The connection is gone: a world that was paused isn't, as far as the
	/// client is to know
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

	/// Tells the client if the world was already paused when this connected,
	/// which the server said only to the clients there then
	pub fn catch_up_on_pause(&mut self) -> Result<(), String> {
		let Response::Status { paused: Some(stop) } = self.request(&Request::Status)? else {
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

	/// Which of the world's stops this is, or the last one was: what a
	/// client was shown at one (a frame, a variable) isn't there at the next
	pub fn stop_number(&self) -> u32 {
		self.stops
	}

	/// Whether the world is paused
	pub fn stopped(&self) -> bool {
		self.stopped.load(Ordering::Relaxed)
	}

	/// Says goodbye, and waits a little for the server to have let this
	/// client go, its breakpoints with it: what the client does next (in
	/// another session, say) then comes after
	pub fn leave(&mut self) {
		if self.send(&Request::Disconnect).is_err() {
			return;
		}
		let deadline = std::time::Instant::now() + GOODBYE_TIMEOUT;
		// The reader stops when the server hangs up
		while let Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) = self.responses.recv_timeout(Duration::from_millis(50)) {
			if std::time::Instant::now() >= deadline {
				return;
			}
		}
	}

	/// Sends a request that isn't answered
	pub fn send(&mut self, request: &Request) -> Result<(), String> {
		local::send(&mut self.writer, request).map_err(|e| format!("couldn't reach the world: {}", e))
	}

	/// Sends an evaluation without waiting for its answer, which is thrown
	/// away when it comes
	pub fn eval_unanswered(&mut self, command: String) -> Result<(), String> {
		self.unanswered.fetch_add(1, Ordering::Relaxed);
		let sent = self.send(&Request::Eval {
			frame_id: None,
			command,
			context: None
		});
		if sent.is_err() {
			self.unanswered.fetch_sub(1, Ordering::Relaxed);
		}
		sent
	}

	/// Sends a request and waits for its response
	pub fn request(&mut self, request: &Request) -> Result<Response, String> {
		// Anything still waiting belonged to a request that timed out
		while self.responses.try_recv().is_ok() {}

		if let Request::Continue { .. } = request {
			self.caught_up = false;
			self.stopped.store(false, Ordering::Relaxed);
		}
		self.send(request)?;
		match self.responses.recv_timeout(RESPONSE_TIMEOUT) {
			Ok(response) => Ok(response),
			Err(mpsc::RecvTimeoutError::Timeout) => Err("the world didn't answer".to_owned()),
			Err(mpsc::RecvTimeoutError::Disconnected) => Err("the world disconnected".to_owned())
		}
	}
}

#[cfg(test)]
mod tests {
	use std::{
		io::{Read, Write},
		sync::{mpsc, Arc}
	};

	use auxtools_cli::{
		local::Stream,
		server_types::{Request, Response}
	};
	use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions};

	use super::Debuggee;
	use crate::dap::Output;

	fn read_request(stream: &mut Stream) -> Request {
		let mut len = [0u8; 4];
		stream.read_exact(&mut len).unwrap();
		let mut data = vec![0; u32::from_le_bytes(len) as usize];
		stream.read_exact(&mut data).unwrap();
		bincode::deserialize(&data).unwrap()
	}

	fn write_response(stream: &mut Stream, response: &Response) {
		let data = bincode::serialize(response).unwrap();
		stream.write_all(&(data.len() as u32).to_le_bytes()).unwrap();
		stream.write_all(&data).unwrap();
		stream.flush().unwrap();
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
		debuggee.eval_unanswered("world.Del()".to_owned()).unwrap();
		// The world answers the evaluation once the next request is in
		let answering = std::thread::spawn(move || {
			assert!(matches!(read_request(&mut world), Request::Eval { .. }));
			assert!(matches!(read_request(&mut world), Request::Pause));
			write_response(&mut world, &Response::Eval(Err("world/Del() slept".to_owned())));
			write_response(&mut world, &Response::Ack);
			world
		});
		let answer = debuggee.request(&Request::Pause).unwrap();
		assert!(matches!(answer, Response::Ack), "{:?}", answer);
		answering.join().unwrap();
	}
}
