//! The connection to the debug server in the world. Requests go out from the
//! adapter's thread, which waits for their responses. A thread reads
//! everything that comes back and passes on what the server says on its own:
//! the world pausing, for this client or another, and carrying on.

use std::{
	sync::{
		atomic::{AtomicBool, Ordering},
		mpsc, Arc, Mutex
	},
	time::Duration
};

use dap_types::{ContinuedEvent, StoppedEvent};
use auxtools_cli::{
	local::{self, Shared, Stream},
	server_types::{BreakpointReason, Request, Response}
};

use crate::{adapter::Input, dap::Output};

/// Tells the client the world stopped, and why
fn announce_stop(out: &Output, last_error: &Mutex<String>, reason: BreakpointReason) {
	let mut description = None;
	let reason = match reason {
		BreakpointReason::Step => StoppedEvent::REASON_STEP,
		BreakpointReason::Pause => StoppedEvent::REASON_PAUSE,
		BreakpointReason::Breakpoint => StoppedEvent::REASON_BREAKPOINT,
		BreakpointReason::Runtime(error) => {
			last_error.lock().unwrap().clone_from(&error);
			description = Some(error);
			StoppedEvent::REASON_EXCEPTION
		}
	};
	out.event(StoppedEvent {
		reason: reason.to_owned(),
		description,
		threadId: Some(0),
		allThreadsStopped: Some(true),
		..Default::default()
	});
}

/// How long a request can go unanswered. The server answers between DM
/// instructions, and the world runs at least one a tick.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

pub struct Debuggee {
	writer: Shared,
	responses: mpsc::Receiver<Response>,
	/// Whether the world is paused
	stopped: Arc<AtomicBool>,
	out: Arc<Output>,
	last_error: Arc<Mutex<String>>,
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
		last_error: Arc<Mutex<String>>
	) -> Result<Self, String> {
		let writer = Shared::new(stream);
		let mut reader = writer.clone();
		let debuggee_out = out.clone();
		let debuggee_last_error = last_error.clone();
		let (responses_sender, responses) = mpsc::channel();
		let stopped = Arc::new(AtomicBool::new(false));
		let reader_stopped = stopped.clone();

		let continued = move |out: &Output| {
			out.event(ContinuedEvent {
				threadId: 0,
				allThreadsContinued: Some(true)
			});
		};

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

						Response::BreakpointHit { .. } | Response::Resumed if !debugging => {}

						Response::BreakpointHit { reason } => {
							reader_stopped.store(true, Ordering::Relaxed);
							announce_stop(&out, &last_error, reason);
						}

						// Another client let it carry on
						Response::Resumed => {
							reader_stopped.store(false, Ordering::Relaxed);
							continued(&out);
						}

						response => {
							if responses_sender.send(response).is_err() {
								break "nobody was waiting for responses".to_owned();
							}
						}
					}
				};
				if reader_stopped.swap(false, Ordering::Relaxed) {
					continued(&out);
				}
				let _ = inputs.send(Input::ConnectionClosed { generation, reason });
			})
			.map_err(|e| e.to_string())?;

		Ok(Self {
			writer,
			responses,
			stopped,
			out: debuggee_out,
			last_error: debuggee_last_error,
			generation
		})
	}

	/// Tells the client if the world was already paused when this connected,
	/// which the server said only to the clients there then
	pub fn catch_up_on_pause(&mut self) -> Result<(), String> {
		let Response::Status { paused: Some(reason) } = self.request(&Request::Status)? else {
			return Ok(());
		};
		if !self.stopped.swap(true, Ordering::Relaxed) {
			announce_stop(&self.out, &self.last_error, reason);
		}
		Ok(())
	}

	/// Sends a request that isn't answered
	pub fn send(&mut self, request: &Request) -> Result<(), String> {
		local::send(&mut self.writer, request).map_err(|e| format!("couldn't reach the world: {}", e))
	}

	/// Sends a request and waits for its response
	pub fn request(&mut self, request: &Request) -> Result<Response, String> {
		// Anything still waiting belonged to a request that timed out
		while self.responses.try_recv().is_ok() {}

		if let Request::Continue { .. } = request {
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
