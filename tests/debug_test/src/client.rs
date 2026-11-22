use std::{
	io::{Read, Write},
	path::{Path, PathBuf},
	sync::{mpsc, Arc, Mutex},
	time::{Duration, Instant}
};

use auxtools_cli::{
	game::{Game, Launch},
	local::{self, Shared}
};

use auxtools_protocol::{
	BreakpointReason, ClientMessage, ContinueKind, Pending, Request, RequestId, Response, ServerMessage, Sorted, WORLD_SHUTTING_DOWN
};

const TIMEOUT: Duration = Duration::from_secs(15);

const START_TIMEOUT: Duration = Duration::from_secs(30);

pub struct World {
	game: Game,
	output: Arc<Mutex<String>>,
	exited: mpsc::Receiver<Option<i32>>
}

impl World {
	pub fn start(dmb: &Path, env: &[(&str, String)]) -> Self {
		let env = env.iter().map(|(name, value)| (name.to_string(), value.clone())).collect();
		let args = ["-trusted".to_owned(), "-close".to_owned()];
		let output = Arc::new(Mutex::new(String::new()));
		let printed = output.clone();
		let (exit, exited) = mpsc::channel();
		let game = Game::start(
			Launch {
				program: crate::paths::find_dreamdaemon(),
				dmb,
				args: &args,
				env: &env,
				debug: Some((find_debug_server(), true))
			},
			move |_, line| {
				print!("  | {}", line);
				printed.lock().unwrap().push_str(&line);
			},
			move |code| {
				println!("  | the world exited ({:?})", code);
				let _ = exit.send(code);
			}
		)
		.unwrap();

		Self { game, output, exited }
	}

	#[track_caller]
	pub fn exit_code(&self) -> Option<i32> {
		self.exited.recv_timeout(TIMEOUT).expect("the world is still running")
	}

	pub fn output(&self) -> String {
		self.output.lock().unwrap().clone()
	}

	#[track_caller]
	pub fn connect(&self) -> Client {
		self.connect_as(false)
	}

	#[track_caller]
	pub fn connect_watcher(&self) -> Client {
		self.connect_as(true)
	}

	#[track_caller]
	pub fn connect_silent(&self) -> Client {
		let start = Instant::now();
		loop {
			assert!(start.elapsed() < START_TIMEOUT, "timed out connecting to debug server");
			if let Ok(stream) = local::connect(self.game.pid()) {
				let stream = Shared::new(stream);
				return Client::new(Box::new(stream.clone()), Box::new(stream), false);
			}
			std::thread::sleep(Duration::from_millis(100));
		}
	}

	#[track_caller]
	fn connect_as(&self, watcher: bool) -> Client {
		let start = Instant::now();
		loop {
			assert!(start.elapsed() < START_TIMEOUT, "timed out connecting to debug server");
			let Ok(stream) = local::connect(self.game.pid()) else {
				std::thread::sleep(Duration::from_millis(100));
				continue;
			};
			let stream = Shared::new(stream);
			let mut client = Client::new(Box::new(stream.clone()), Box::new(stream), watcher);
			if client.try_send(&Request::Configured).is_err() {
				std::thread::sleep(Duration::from_millis(100));
				continue;
			}
			match client.messages.recv_timeout(TIMEOUT).map(|message| client.requests.sort(message)) {
				Ok(Sorted::Answer(_, Response::Ack)) => {
					if watcher {
						let response = client.request(Request::Watcher);
						assert!(matches!(response, Response::Ack), "expected Ack for Watcher, got {:?}", response);
					}
					return client;
				}
				Ok(Sorted::Event(Response::Disconnect)) | Err(mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(Duration::from_millis(100)),
				other => panic!("expected Ack for Configured, got {:?}", other)
			}
		}
	}
}

impl Drop for World {
	fn drop(&mut self) {
		self.game.kill();
	}
}

fn find_debug_server() -> PathBuf {
	let mut path = std::env::current_exe().unwrap();
	path.pop();

	#[cfg(unix)]
	path.push("libdebug_server.so");

	#[cfg(windows)]
	path.push("debug_server.dll");

	assert!(path.is_file(), "couldn't find debug_server");
	path
}

pub struct Client {
	writer: Box<dyn Write + Send>,
	messages: mpsc::Receiver<ServerMessage>,
	requests: Pending,
	pub late: usize,
	watcher: bool
}

impl Client {
	fn new(mut reader: Box<dyn Read + Send>, writer: Box<dyn Write + Send>, watcher: bool) -> Self {
		let (sender, messages) = mpsc::channel();
		std::thread::spawn(move || {
			while let Ok(message) = auxtools_protocol::read(&mut reader) {
				if sender.send(message).is_err() {
					return;
				}
			}
		});
		Self {
			writer,
			messages,
			requests: Pending::new(),
			late: 0,
			watcher
		}
	}

	#[track_caller]
	pub fn send(&mut self, request: Request) {
		self.try_send(&request).unwrap();
	}

	fn try_send(&mut self, request: &Request) -> std::io::Result<RequestId> {
		let id = self.requests.ask();
		auxtools_protocol::write(&mut self.writer, &ClientMessage { id, request })?;
		Ok(id)
	}

	#[track_caller]
	pub fn send_bytes(&mut self, bytes: &[u8]) {
		self.writer.write_all(bytes).unwrap();
	}

	#[track_caller]
	pub fn leave(mut self) {
		let _ = self.try_send(&Request::Disconnect);
		self.wait_closed();
	}

	#[track_caller]
	pub fn recv(&mut self) -> Response {
		self.recv_within(TIMEOUT).expect("debug server stopped answering")
	}

	fn recv_within(&mut self, patience: Duration) -> Option<Response> {
		let until = Instant::now() + patience;
		loop {
			let message = match self.messages.recv_timeout(until.saturating_duration_since(Instant::now())) {
				Ok(message) => message,
				Err(mpsc::RecvTimeoutError::Timeout) => return None,
				Err(mpsc::RecvTimeoutError::Disconnected) => panic!("the debug server hung up")
			};
			match self.requests.sort(message) {
				Sorted::Answer(_, response) => return Some(response),
				Sorted::Late(..) => self.late += 1,
				Sorted::Event(Response::Notification { message }) => println!("  [debug server] {}", message),
				Sorted::Event(Response::ShuttingDown) => println!("  [debug server] {}", WORLD_SHUTTING_DOWN),
				Sorted::Event(Response::Resumed | Response::CodeChanged | Response::StepEnded) => {}
				Sorted::Event(Response::BreakpointHit { .. }) if self.watcher => {}
				Sorted::Event(response) => return Some(response)
			}
		}
	}

	#[track_caller]
	pub fn request(&mut self, request: Request) -> Response {
		self.send(request);
		self.recv()
	}

	pub fn request_within(&mut self, request: Request, patience: Duration) -> Option<Response> {
		let id = self.try_send(&request).unwrap();
		let answer = self.recv_within(patience);
		if answer.is_none() {
			self.requests.give_up(id);
		}
		answer
	}

	#[track_caller]
	pub fn expect_pause(&mut self) -> BreakpointReason {
		match self.recv() {
			Response::BreakpointHit(stop) => stop.reason,
			other => panic!("expected the world to pause, got {:?}", other)
		}
	}

	#[track_caller]
	pub fn resume(&mut self) {
		let response = self.request(Request::Continue {
			kind: ContinueKind::Continue
		});
		assert!(matches!(response, Response::Ack), "expected Ack for Continue, got {:?}", response);
	}

	#[track_caller]
	pub fn wait_closed(self) {
		loop {
			match self.messages.recv_timeout(TIMEOUT).map(|message| message.response) {
				Ok(Response::Disconnect) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
				Ok(_) => {}
				Err(mpsc::RecvTimeoutError::Timeout) => panic!("timed out waiting for debug server disconnect")
			}
		}
	}
}
