use std::{
	collections::HashMap,
	sync::{
		atomic::{AtomicBool, AtomicU32, Ordering},
		mpsc, Arc, Mutex
	},
	time::Duration
};

use auxtools_cli::{
	local::{self, Shared, Stream},
	server_types::{BreakpointReason, DataChange, InstructionRef, Request, Response, Stop, ENDING_CONTEXT, WORLD_SHUTTING_DOWN}
};
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

pub struct Debuggee {
	writer: Shared,
	responses: mpsc::Receiver<(Response, u64)>,
	stopped: Arc<AtomicBool>,
	out: Arc<Output>,
	last_error: Arc<Mutex<LastError>>,
	set: SetBreakpoints,
	thread: Arc<AtomicU32>,
	caught_up: bool,
	stops: u32,
	resumed_after: u64,
	unanswered: Arc<AtomicU32>,
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
		let writer = Shared::new(stream);
		let mut reader = writer.clone();
		let debuggee_out = out.clone();
		let (responses_sender, responses) = mpsc::channel();
		let unanswered = Arc::new(AtomicU32::new(0));
		let reader_unanswered = unanswered.clone();
		let mut stops_read = 0;

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

						Response::Unavailable { reason } => {
							let _ = inputs.send(Input::Unavailable { generation, reason });
							break "debugger initialization failed".to_owned();
						}

						Response::Notification { message } if message == WORLD_SHUTTING_DOWN => {
							if inputs.send(Input::ShuttingDown { generation }).is_err() {
								break "adapter event receiver closed".to_owned();
							}
						}

						Response::Notification { message } => out.console(message),

						Response::ConditionFailed { message } => out.console(message),

						Response::BreakpointHit { .. } | Response::Resumed if !debugging => {}

						Response::BreakpointHit(stop) => {
							stops_read += 1;
							if inputs
								.send(Input::Stopped {
									generation,
									stop,
									number: stops_read
								})
								.is_err()
							{
								break "adapter event receiver closed".to_owned();
							}
						}

						Response::Log { message, file, line, .. } => {
							if inputs.send(Input::Logged { message, file, line }).is_err() {
								break "adapter event receiver closed".to_owned();
							}
						}

						Response::WatchEnded { id, reason } => {
							if inputs.send(Input::WatchEnded { generation, id, reason }).is_err() {
								break "adapter event receiver closed".to_owned();
							}
						}

						Response::CodeChanged => {
							if debugging && inputs.send(Input::CodeChanged { generation }).is_err() {
								break "adapter event receiver closed".to_owned();
							}
						}

						Response::Journaled(entry) => {
							if let Some(text) = crate::adapter::reload_by_another(&entry) {
								out.console(text);
							}
						}

						Response::StepEnded => {}

						Response::Resumed => {
							if inputs.send(Input::Resumed { generation }).is_err() {
								break "adapter event receiver closed".to_owned();
							}
						}

						Response::Eval(_)
							if reader_unanswered
								.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
								.is_ok() => {}

						response => {
							if responses_sender.send((response, stops_read)).is_err() {
								break "response receiver closed".to_owned();
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
			resumed_after: 0,
			unanswered,
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

	pub fn stop_number(&self) -> u32 {
		self.stops
	}

	pub fn stopped(&self) -> bool {
		self.stopped.load(Ordering::Relaxed)
	}

	pub fn leave(&mut self) {
		if self.send(&Request::Disconnect).is_err() {
			return;
		}
		let deadline = std::time::Instant::now() + GOODBYE_TIMEOUT;
		while let Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) = self.responses.recv_timeout(Duration::from_millis(50)) {
			if std::time::Instant::now() >= deadline {
				return;
			}
		}
	}

	pub fn send(&mut self, request: &Request) -> Result<(), String> {
		local::send(&mut self.writer, request).map_err(|e| format!("couldn't reach the world: {}", e))
	}

	pub fn eval_unanswered(&mut self, command: String) -> Result<(), String> {
		self.unanswered.fetch_add(1, Ordering::Relaxed);
		let sent = self.send(&Request::Eval {
			frame_id: None,
			command,
			context: Some(ENDING_CONTEXT.to_owned())
		});
		if sent.is_err() {
			self.unanswered.fetch_sub(1, Ordering::Relaxed);
		}
		sent
	}

	pub fn request(&mut self, request: &Request) -> Result<Response, String> {
		while self.responses.try_recv().is_ok() {}

		let resumes = matches!(request, Request::Continue { .. });
		if resumes {
			self.caught_up = false;
			self.stopped.store(false, Ordering::Relaxed);
		}
		self.send(request)?;
		match self.responses.recv_timeout(RESPONSE_TIMEOUT) {
			Ok((response, stops)) => {
				if resumes && matches!(response, Response::Ack) {
					self.resumed_after = stops;
				}
				Ok(response)
			}
			Err(mpsc::RecvTimeoutError::Timeout) => Err("debug server response timed out".to_owned()),
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
