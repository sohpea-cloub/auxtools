use std::{
	collections::VecDeque,
	io::{self, Read, Write},
	path::Path,
	process::{Child, Command, Stdio},
	sync::{
		atomic::{AtomicBool, AtomicUsize, Ordering},
		mpsc, Arc, Mutex
	},
	time::{Duration, Instant}
};

use auxtools_protocol::{world_socket_name, ClientMessage, Hello, Request, Response, ServerMessage};
use interprocess::local_socket::{prelude::*, GenericNamespaced, Listener, ListenerNonblockingMode, ListenerOptions, Stream};
use serde_json::{json, Value};

use crate::temp::TempDir;

pub const PROCESS_VAR: &str = "AUXTOOLS_STANDIN";

pub const PROCESS_TEST: &str = "world_process";

pub const PATIENCE: Duration = Duration::from_secs(15);

const POLL: Duration = Duration::from_millis(2);

pub fn world_process() {
	let Ok(name) = std::env::var(PROCESS_VAR) else {
		return;
	};
	let Ok(mut control) = Stream::connect(name.to_ns_name::<GenericNamespaced>().unwrap()) else {
		std::process::exit(2);
	};
	let _ = writeln!(control, "{}", std::process::id());
	let _ = io::copy(&mut control, &mut io::sink());
	std::process::exit(0);
}

pub enum Answer {
	Reply(Response),
	Nothing,
	ToTest
}

pub type Answers = Box<dyn FnMut(&Request) -> Answer + Send>;

static STAND_INS: AtomicUsize = AtomicUsize::new(0);

pub struct StandIn {
	number: usize,
	control_name: String,
	control: Listener,
	process: Option<(u32, Stream)>,
	child: Option<Child>,
	answers: Arc<Mutex<Answers>>,
	worlds: mpsc::Receiver<World>,
	worlds_sender: mpsc::Sender<World>,
	listening: Arc<AtomicBool>,
	byond: Option<TempDir>
}

impl StandIn {
	pub fn new(answers: Answers) -> Self {
		let number = STAND_INS.fetch_add(1, Ordering::SeqCst);
		let control_name = format!("auxtools-standin-{}-{}", std::process::id(), number);
		let control = ListenerOptions::new()
			.name(control_name.clone().to_ns_name::<GenericNamespaced>().unwrap())
			.nonblocking(ListenerNonblockingMode::Accept)
			.create_sync()
			.unwrap();
		let (worlds_sender, worlds) = mpsc::channel();
		Self {
			number,
			control_name,
			control,
			process: None,
			child: None,
			answers: Arc::new(Mutex::new(answers)),
			worlds,
			worlds_sender,
			listening: Arc::new(AtomicBool::new(true)),
			byond: None
		}
	}

	#[track_caller]
	pub fn spawn(&mut self) -> u32 {
		self.start_process();
		self.started()
	}

	pub fn launch_arguments(&mut self, dme: &Path, more: Value) -> Value {
		let byond = self.byond.get_or_insert_with(|| {
			let byond = TempDir::new(&format!("standin_byond_{}", self.number));
			std::fs::create_dir_all(byond.join("bin")).unwrap();
			let server = byond.join("bin").join(if cfg!(windows) { "dd.exe" } else { "DreamDaemon" });
			let us = std::env::current_exe().unwrap();
			if std::fs::hard_link(&us, &server).is_err() {
				std::fs::copy(&us, &server).unwrap();
			}
			byond
		});
		let mut arguments = json!({
			"dme": dme,
			"byondPath": &**byond,
			"server": "dreamdaemon",
			"debugLibrary": byond.join("no-debug-library"),
			"args": ["--exact", PROCESS_TEST, "--nocapture", "--test-threads", "1"],
			"env": { PROCESS_VAR: &self.control_name },
			"hotReload": false
		});
		for (name, value) in more.as_object().unwrap() {
			arguments[name] = value.clone();
		}
		arguments
	}

	#[track_caller]
	pub fn started(&mut self) -> u32 {
		let pid = self.reported();
		let world = ListenerOptions::new()
			.name(world_socket_name(pid).to_ns_name::<GenericNamespaced>().unwrap())
			.nonblocking(ListenerNonblockingMode::Accept)
			.create_sync()
			.unwrap();
		let (answers, worlds, listening) = (self.answers.clone(), self.worlds_sender.clone(), self.listening.clone());
		std::thread::spawn(move || {
			while listening.load(Ordering::SeqCst) {
				match world.accept() {
					Ok(stream) => match World::serve(stream, pid, answers.clone()) {
						Ok(world) => {
							if worlds.send(world).is_err() {
								return;
							}
						}
						Err(_) => continue
					},
					Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
					Err(_) => return
				}
			}
		});
		pid
	}

	#[track_caller]
	pub fn relaying(&mut self, world_pid: u32, log: impl Write + Send + 'static) -> u32 {
		self.start_process();
		let pid = self.reported();
		std::thread::spawn(move || relay(pid, world_pid, log));
		pid
	}

	fn start_process(&mut self) {
		let child = Command::new(std::env::current_exe().unwrap())
			.args(["--exact", PROCESS_TEST, "--nocapture", "--test-threads", "1"])
			.env(PROCESS_VAR, &self.control_name)
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.spawn()
			.unwrap();
		self.child = Some(child);
	}

	#[track_caller]
	fn reported(&mut self) -> u32 {
		let deadline = Instant::now() + PATIENCE;
		let mut control = loop {
			match self.control.accept() {
				Ok(stream) => break stream,
				Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
					assert!(Instant::now() < deadline, "the world's process didn't start");
					std::thread::sleep(Duration::from_millis(10));
				}
				Err(e) => panic!("the world's process couldn't report: {}", e)
			}
		};
		let mut pid = String::new();
		let mut byte = [0];
		while control.read(&mut byte).unwrap() == 1 && byte[0] != b'\n' {
			pid.push(byte[0] as char);
		}
		let pid: u32 = pid.trim().parse().unwrap();
		self.process = Some((pid, control));
		pid
	}

	#[track_caller]
	pub fn world(&self) -> World {
		self.worlds
			.recv_timeout(PATIENCE)
			.unwrap_or_else(|_| panic!("the launcher didn't connect to the world"))
	}
}

impl Drop for StandIn {
	fn drop(&mut self) {
		self.listening.store(false, Ordering::SeqCst);
		self.process = None;
		if let Some(child) = &mut self.child {
			let deadline = Instant::now() + Duration::from_secs(5);
			while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
				std::thread::sleep(Duration::from_millis(10));
			}
			let _ = child.kill();
			let _ = child.wait();
		}
	}
}

struct Connection {
	stream: Stream,
	closed: AtomicBool
}

impl Connection {
	fn send(&self, message: &ServerMessage) -> io::Result<()> {
		let mut bytes = &auxtools_protocol::frame(message)?[..];
		while !bytes.is_empty() {
			if self.closed.load(Ordering::SeqCst) {
				return Err(io::ErrorKind::BrokenPipe.into());
			}
			match (&self.stream).write(bytes) {
				Ok(0) => std::thread::sleep(POLL),
				Ok(n) => bytes = &bytes[n..],
				Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(POLL),
				Err(e) => return Err(e)
			}
		}
		Ok(())
	}
}

pub struct World {
	connection: Arc<Connection>,
	waiting: mpsc::Receiver<ClientMessage>,
	held: VecDeque<ClientMessage>,
	log: Arc<Mutex<Vec<String>>>
}

impl World {
	fn serve(stream: Stream, pid: u32, answers: Arc<Mutex<Answers>>) -> io::Result<Self> {
		let hello = Hello {
			pid,
			..Hello::new("stand-in")
		};
		auxtools_protocol::write(&mut &stream, &hello)?;
		stream.set_nonblocking(true)?;
		let connection = Arc::new(Connection {
			stream,
			closed: AtomicBool::new(false)
		});
		let log = Arc::new(Mutex::new(vec![]));
		let (sender, waiting) = mpsc::channel();
		let (reading, logged) = (connection.clone(), log.clone());
		std::thread::spawn(move || {
			let mut buffer = vec![];
			let mut chunk = vec![0; 64 * 1024];
			while !reading.closed.load(Ordering::SeqCst) {
				match (&reading.stream).read(&mut chunk) {
					Ok(0) if cfg!(windows) => {
						std::thread::sleep(POLL);
						continue;
					}
					Ok(0) => break,
					Ok(n) => buffer.extend_from_slice(&chunk[..n]),
					Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
						std::thread::sleep(POLL);
						continue;
					}
					Err(_) => break
				}
				while let Some(message) = take_message(&mut buffer) {
					logged.lock().unwrap().push(format!("{:?}", message.request));
					let answer = (answers.lock().unwrap())(&message.request);
					match answer {
						Answer::Reply(response) => {
							if reading.send(&ServerMessage::answer(message.id, response)).is_err() {
								return;
							}
						}
						Answer::Nothing => {}
						Answer::ToTest => {
							if sender.send(message).is_err() {
								return;
							}
						}
					}
				}
			}
			reading.closed.store(true, Ordering::SeqCst);
		});
		Ok(Self {
			connection,
			waiting,
			held: VecDeque::new(),
			log
		})
	}

	#[track_caller]
	pub fn next(&mut self, what: &str, wanted: impl Fn(&Request) -> bool) -> ClientMessage {
		self.next_within(PATIENCE, wanted)
			.unwrap_or_else(|| panic!("the launcher didn't ask for {}; it asked for {:#?}", what, self.requests()))
	}

	pub fn next_within(&mut self, patience: Duration, wanted: impl Fn(&Request) -> bool) -> Option<ClientMessage> {
		if let Some(at) = self.held.iter().position(|message| wanted(&message.request)) {
			return self.held.remove(at);
		}
		let deadline = Instant::now() + patience;
		loop {
			let message = self.waiting.recv_timeout(deadline.saturating_duration_since(Instant::now())).ok()?;
			if wanted(&message.request) {
				return Some(message);
			}
			self.held.push_back(message);
		}
	}

	pub fn answer(&self, to: &ClientMessage, response: Response) {
		self.connection.send(&ServerMessage::answer(to.id, response)).unwrap();
	}

	pub fn event(&self, response: Response) {
		let _ = self.connection.send(&ServerMessage::event(response));
	}

	pub fn requests(&self) -> Vec<String> {
		self.log.lock().unwrap().clone()
	}

	pub fn close(self) {}
}

impl Drop for World {
	fn drop(&mut self) {
		self.connection.closed.store(true, Ordering::SeqCst);
	}
}

fn take_message(buffer: &mut Vec<u8>) -> Option<ClientMessage> {
	let len = u32::from_le_bytes(buffer.get(..4)?.try_into().unwrap()) as usize;
	if buffer.len() < 4 + len {
		return None;
	}
	let message = auxtools_protocol::read(&mut &buffer[..4 + len]).expect("the launcher sent bytes that are no message");
	buffer.drain(..4 + len);
	Some(message)
}

pub fn relay(as_pid: u32, world_pid: u32, log: impl Write + Send + 'static) {
	let log = Arc::new(Mutex::new(log));
	let listener = ListenerOptions::new()
		.name(world_socket_name(as_pid).to_ns_name::<GenericNamespaced>().unwrap())
		.create_sync()
		.unwrap();
	for (number, launcher) in listener.incoming().flatten().enumerate() {
		let Ok(world) = Stream::connect(world_socket_name(world_pid).to_ns_name::<GenericNamespaced>().unwrap()) else {
			continue;
		};
		let (launcher, world) = (Arc::new(launcher), Arc::new(world));
		let (to_world, from_world) = (log.clone(), log.clone());
		let (launcher_in, world_out) = (launcher.clone(), world.clone());
		std::thread::spawn(move || {
			while let Ok(message) = auxtools_protocol::read::<ClientMessage>(&mut &*launcher_in) {
				let _ = writeln!(to_world.lock().unwrap(), "{} >> {:?}", number, message);
				if auxtools_protocol::write(&mut &*world_out, &message).is_err() {
					break;
				}
			}
		});
		std::thread::spawn(move || {
			let Ok(hello) = auxtools_protocol::read::<Hello>(&mut &*world) else {
				return;
			};
			let _ = writeln!(from_world.lock().unwrap(), "{} << {:?}", number, hello);
			if auxtools_protocol::write(&mut &*launcher, &hello).is_err() {
				return;
			}
			while let Ok(message) = auxtools_protocol::read::<ServerMessage>(&mut &*world) {
				let _ = writeln!(from_world.lock().unwrap(), "{} << {:?}", number, message);
				if auxtools_protocol::write(&mut &*launcher, &message).is_err() {
					break;
				}
			}
		});
	}
}
