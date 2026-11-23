use std::{
	io::{self, Read, Write},
	path::{Path, PathBuf},
	sync::{
		atomic::{AtomicBool, Ordering},
		Arc, OnceLock
	},
	time::{Duration, Instant}
};

pub use interprocess::local_socket::Stream;
use interprocess::{
	local_socket::{prelude::*, ConnectOptions, GenericNamespaced},
	ConnectWaitMode
};

use auxtools_protocol::{old_socket_name, read, world_socket_name, Hello, Request, Response, LAST_NAMED_PROTOCOL, PROTOCOL_VERSION};

use crate::client::{Client, Heard, Lost};

pub fn world_name(pid: u32) -> String {
	world_socket_name(pid)
}

fn connect_to(name: String) -> io::Result<Stream> {
	ConnectOptions::new()
		.name(name.to_ns_name::<GenericNamespaced>()?)
		.wait_mode(ConnectWaitMode::Timeout(Duration::from_secs(1)))
		.connect_sync()
}

pub fn connect(pid: u32) -> io::Result<Stream> {
	let (stream, hello) = read_hello(open(pid)?)?;
	if hello.protocol != PROTOCOL_VERSION {
		return Err(incompatible(hello.protocol));
	}
	Ok(stream)
}

pub fn open(pid: u32) -> io::Result<Stream> {
	connect_to(world_name(pid)).map_err(|e| auxtools3_protocol(pid).map_or_else(|| not_listening(e), incompatible))
}

fn not_listening(e: io::Error) -> io::Error {
	io::Error::new(
		e.kind(),
		format!(
			"no debug server is listening in it: {}. Either it isn't a world with auxtools' debug library loaded, or its world is still loading",
			e
		)
	)
}

pub fn incompatible(theirs: u32) -> io::Error {
	io::Error::other(format!(
		"its debug library speaks protocol {}, and this auxtools speaks {}: use the auxtools that goes with it (the one that started the world), or \
		 start the world with this one",
		theirs, PROTOCOL_VERSION
	))
}

const HELLO_PATIENCE: Duration = Duration::from_secs(5);

fn read_hello(stream: Stream) -> io::Result<(Stream, Hello)> {
	let connection = Shared::new(stream);
	let (sender, receiver) = std::sync::mpsc::channel();
	let mut reader = connection.clone();
	std::thread::Builder::new().name("world hello".to_owned()).spawn(move || {
		let hello = read::<Hello>(&mut reader);
		drop(reader);
		let _ = sender.send(hello);
	})?;
	let Ok(hello) = receiver.recv_timeout(HELLO_PATIENCE) else {
		connection.hang_up();
		return Err(io::Error::new(
			io::ErrorKind::TimedOut,
			format!(
				"its debug library said nothing for {} seconds after the connection opened: the process is stopped or hung",
				HELLO_PATIENCE.as_secs()
			)
		));
	};
	let hello = hello?;
	let stream = connection
		.into_stream()
		.ok_or_else(|| io::Error::other("the connection is still being read"))?;
	Ok((stream, hello))
}

fn auxtools3_protocol(pid: u32) -> Option<u32> {
	(1..=LAST_NAMED_PROTOCOL)
		.rev()
		.find(|version| connect_to(old_socket_name(*version, pid)).is_ok())
}

#[derive(Clone)]
pub struct Shared(Arc<Connection>);

struct Connection {
	stream: Stream,
	hung_up: AtomicBool,
	#[cfg(windows)]
	reading: std::sync::atomic::AtomicUsize
}

impl Shared {
	pub fn new(stream: Stream) -> Self {
		Self(Arc::new(Connection {
			stream,
			hung_up: AtomicBool::new(false),
			#[cfg(windows)]
			reading: Default::default()
		}))
	}

	fn into_stream(self) -> Option<Stream> {
		Arc::into_inner(self.0).map(|connection| connection.stream)
	}

	pub fn hang_up(&self) {
		self.0.hung_up.store(true, Ordering::SeqCst);
		self.0.cancel_reads();
	}
}

impl Connection {
	fn check(&self) -> io::Result<()> {
		if self.hung_up.load(Ordering::SeqCst) {
			return Err(io::Error::new(io::ErrorKind::NotConnected, "the connection was hung up"));
		}
		Ok(())
	}

	#[cfg(unix)]
	fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
		self.check()?;
		(&self.stream).read(buf)
	}

	#[cfg(unix)]
	fn cancel_reads(&self) {
		use std::os::fd::{AsFd, AsRawFd};

		let Stream::UdSocket(socket) = &self.stream;
		unsafe {
			libc::shutdown(socket.as_fd().as_raw_fd(), libc::SHUT_RDWR);
		}
	}

	#[cfg(windows)]
	fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
		self.reading.fetch_add(1, Ordering::SeqCst);
		let read = self.check().and_then(|()| (&self.stream).read(buf));
		self.reading.fetch_sub(1, Ordering::SeqCst);
		self.check()?;
		read
	}

	#[cfg(windows)]
	fn cancel_reads(&self) {
		use std::os::windows::io::{AsHandle, AsRawHandle};

		let Stream::NamedPipe(pipe) = &self.stream;
		while self.reading.load(Ordering::SeqCst) > 0 {
			unsafe {
				winapi::um::ioapiset::CancelIoEx(pipe.as_handle().as_raw_handle().cast(), std::ptr::null_mut());
			}
			std::thread::sleep(Duration::from_millis(1));
		}
	}
}

impl Read for Shared {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		self.0.read(buf)
	}
}

impl Write for Shared {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		self.0.check()?;
		(&self.0.stream).write(buf)
	}

	fn flush(&mut self) -> io::Result<()> {
		(&self.0.stream).flush()
	}
}

#[derive(Debug, serde::Serialize)]
pub struct World {
	pub pid: u32,
	pub program: String,
	pub dmb: Option<PathBuf>,
	pub args: Vec<String>,
	pub cwd: String,
	pub protocol: u32,
	pub compatible: bool,
	pub responsive: Option<bool>,
	pub unavailable: Option<String>
}

enum Probed {
	Info { args: Vec<String>, cwd: String },
	Unavailable(String),
	Incompatible(u32),
	Unanswered
}

pub const PROBE_PATIENCE: Duration = Duration::from_secs(5);

const TEST_PATIENCE_VAR: &str = "AUXTOOLS_TEST_PATIENCE";

pub fn test_patience(usual: Duration) -> Duration {
	static GIVEN: OnceLock<Option<Duration>> = OnceLock::new();
	let given = GIVEN.get_or_init(|| std::env::var(TEST_PATIENCE_VAR).ok()?.parse().ok().map(Duration::from_millis));
	given.map_or(usual, |given| given.min(usual))
}

pub fn worlds() -> Vec<World> {
	worlds_among(server_processes())
}

fn worlds_among(processes: Vec<(u32, String)>) -> Vec<World> {
	let until = Instant::now() + test_patience(PROBE_PATIENCE);
	let probes: Vec<_> = processes
		.into_iter()
		.map(|(pid, program)| {
			let answer = connect_to(world_name(pid)).ok().map(|stream| probe(stream, until));
			(pid, program, answer)
		})
		.collect();
	probes
		.into_iter()
		.filter_map(|(pid, program, answer)| {
			let unknown = |protocol, compatible, responsive| World {
				pid,
				program: program.clone(),
				dmb: None,
				args: vec![],
				cwd: String::new(),
				protocol,
				compatible,
				responsive,
				unavailable: None
			};
			match answer.map(|answer| answer.recv_timeout(until.saturating_duration_since(Instant::now()))) {
				Some(Ok(Ok(Probed::Info { args, cwd }))) => Some(World {
					pid,
					program: program.clone(),
					dmb: world_dmb(&args, &cwd),
					args,
					cwd,
					protocol: PROTOCOL_VERSION,
					compatible: true,
					responsive: Some(true),
					unavailable: None
				}),
				Some(Ok(Ok(Probed::Unavailable(why)))) => Some(World {
					unavailable: Some(why),
					..unknown(PROTOCOL_VERSION, true, Some(true))
				}),
				Some(Ok(Ok(Probed::Incompatible(protocol)))) => Some(unknown(protocol, false, None)),
				Some(Ok(Ok(Probed::Unanswered)) | Err(_)) => Some(unknown(PROTOCOL_VERSION, true, Some(false))),
				Some(Ok(Err(_))) | None => auxtools3_protocol(pid).map(|protocol| unknown(protocol, false, None))
			}
		})
		.collect()
}

fn probe(stream: Stream, until: Instant) -> std::sync::mpsc::Receiver<io::Result<Probed>> {
	let (sender, receiver) = std::sync::mpsc::channel();
	std::thread::Builder::new()
		.name("world probe".to_owned())
		.spawn(move || {
			let answer = Client::on(stream).and_then(|mut client| {
				let answer = probed(&mut client, until);
				let _ = client.leave();
				client.hang_up();
				match answer {
					Ok(Some(probed)) => Ok(probed),
					Ok(None) => Ok(Probed::Unanswered),
					Err(Lost::Unsent(e)) => Err(e),
					Err(Lost::Closed | Lost::Incompatible(_)) => Err(io::Error::other("it hung up"))
				}
			});
			let _ = sender.send(answer);
		})
		.expect("couldn't start a thread");
	receiver
}

fn probed(client: &mut Client, until: Instant) -> Result<Option<Probed>, Lost> {
	let asked = match client.ask(&Request::WorldInfo, Some(until)) {
		Ok(Some(asked)) => asked,
		Ok(None) => return Ok(None),
		Err(Lost::Incompatible(theirs)) => return Ok(Some(Probed::Incompatible(theirs))),
		Err(lost) => return Err(lost)
	};
	loop {
		match client.hear(Some(until))? {
			None => return Ok(None),
			Some(Heard::Answer {
				to,
				response: Response::WorldInfo { args, cwd, .. },
				..
			}) if to == asked => return Ok(Some(Probed::Info { args, cwd })),
			Some(Heard::Event(Response::Unavailable { reason })) => return Ok(Some(Probed::Unavailable(reason))),
			Some(Heard::Event(Response::Disconnect)) => return Err(Lost::Closed),
			_ => {}
		}
	}
}

pub fn world_dmb(args: &[String], cwd: &str) -> Option<PathBuf> {
	let dmb = args.iter().skip(1).find(|arg| arg.to_ascii_lowercase().ends_with(".dmb"))?;
	Some(Path::new(cwd).join(dmb))
}

#[cfg(unix)]
fn server_processes() -> Vec<(u32, String)> {
	let Ok(entries) = std::fs::read_dir("/proc") else {
		return vec![];
	};
	entries
		.filter_map(|entry| {
			let pid: u32 = entry.ok()?.file_name().to_str()?.parse().ok()?;
			let program = std::fs::read_to_string(format!("/proc/{}/comm", pid)).ok()?.trim().to_owned();
			(program == "DreamDaemon").then_some((pid, program))
		})
		.collect()
}

#[cfg(windows)]
fn server_processes() -> Vec<(u32, String)> {
	use winapi::um::{
		handleapi::{CloseHandle, INVALID_HANDLE_VALUE},
		tlhelp32::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS}
	};

	let mut found = vec![];
	unsafe {
		let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
		if snapshot == INVALID_HANDLE_VALUE {
			return found;
		}
		let mut entry: PROCESSENTRY32W = std::mem::zeroed();
		entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
		let mut more = Process32FirstW(snapshot, &mut entry) != 0;
		while more {
			let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
			let program = String::from_utf16_lossy(&entry.szExeFile[..len]);
			if ["dd.exe", "dreamdaemon.exe", "dreamseeker.exe"].contains(&program.to_ascii_lowercase().as_str()) {
				found.push((entry.th32ProcessID, program));
			}
			more = Process32NextW(snapshot, &mut entry) != 0;
		}
		CloseHandle(snapshot);
	}
	found
}

#[cfg(unix)]
pub fn alive(pid: u32) -> bool {
	if unsafe { libc::kill(pid as i32, 0) } != 0 {
		return false;
	}
	let zombie = std::fs::read_to_string(format!("/proc/{}/stat", pid))
		.ok()
		.and_then(|stat| Some(stat.rsplit_once(") ")?.1.starts_with('Z')));
	zombie != Some(true)
}

#[cfg(windows)]
pub fn alive(pid: u32) -> bool {
	use winapi::{
		shared::winerror::WAIT_TIMEOUT,
		um::{handleapi::CloseHandle, processthreadsapi::OpenProcess, synchapi::WaitForSingleObject, winnt::SYNCHRONIZE}
	};

	unsafe {
		let process = OpenProcess(SYNCHRONIZE, 0, pid);
		if process.is_null() {
			return false;
		}
		let running = WaitForSingleObject(process, 0) == WAIT_TIMEOUT;
		CloseHandle(process);
		running
	}
}

pub fn kill(pid: u32) {
	#[cfg(unix)]
	unsafe {
		libc::kill(pid as i32, libc::SIGKILL);
	}

	#[cfg(windows)]
	unsafe {
		use winapi::um::{
			handleapi::CloseHandle,
			processthreadsapi::{OpenProcess, TerminateProcess},
			winnt::PROCESS_TERMINATE
		};

		let process = OpenProcess(PROCESS_TERMINATE, 0, pid);
		if !process.is_null() {
			TerminateProcess(process, 1);
			CloseHandle(process);
		}
	}
}

#[cfg(test)]
mod tests {
	use std::{
		io::{self, Read},
		sync::{
			atomic::{AtomicUsize, Ordering},
			Arc
		},
		time::{Duration, Instant}
	};

	use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions, Stream};

	use super::{connect, world_name, worlds_among};

	#[derive(Default)]
	struct Connections {
		accepted: AtomicUsize,
		open: AtomicUsize
	}

	fn silent_world(pid: u32) -> Arc<Connections> {
		let listener = ListenerOptions::new()
			.name(world_name(pid).to_ns_name::<GenericNamespaced>().unwrap())
			.create_sync()
			.unwrap();
		let connections = Arc::new(Connections::default());
		let counted = connections.clone();
		std::thread::spawn(move || {
			for stream in listener.incoming().flatten() {
				counted.accepted.fetch_add(1, Ordering::SeqCst);
				counted.open.fetch_add(1, Ordering::SeqCst);
				let counted = counted.clone();
				std::thread::spawn(move || {
					let mut stream: Stream = stream;
					let mut buffer = [0; 256];
					while matches!(stream.read(&mut buffer), Ok(n) if n > 0) {}
					counted.open.fetch_sub(1, Ordering::SeqCst);
				});
			}
		});
		connections
	}

	fn unused_pid(offset: u32) -> u32 {
		4_000_000_000 + std::process::id() % 100_000 * 10 + offset
	}

	fn threads_named(name: &str) -> usize {
		let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
			return 0;
		};
		tasks
			.flatten()
			.filter(|task| std::fs::read_to_string(task.path().join("comm")).is_ok_and(|comm| comm.trim_end() == name))
			.count()
	}

	#[track_caller]
	fn released(connections: &Connections, accepted: usize, thread: &str) {
		let deadline = Instant::now() + Duration::from_secs(5);
		let left = || (connections.open.load(Ordering::SeqCst), threads_named(thread));
		while left() != (0, 0) && Instant::now() < deadline {
			std::thread::sleep(Duration::from_millis(10));
		}
		assert_eq!(connections.accepted.load(Ordering::SeqCst), accepted, "connections opened");
		assert_eq!(left(), (0, 0), "connections still open and {} threads left", thread);
	}

	#[test]
	fn a_connection_without_a_hello_is_released() {
		let pid = unused_pid(1);
		let connections = silent_world(pid);
		for attempt in 1..=2 {
			let error = connect(pid).expect_err("a world that said nothing was connected to");
			assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{}", error);
			released(&connections, attempt, "world hello");
		}
	}

	#[test]
	fn a_probe_without_an_answer_is_released() {
		let pid = unused_pid(2);
		let connections = silent_world(pid);
		for attempt in 1..=2 {
			let worlds = worlds_among(vec![(pid, "DreamDaemon".to_owned())]);
			let responsive: Vec<_> = worlds.iter().map(|world| (world.pid, world.responsive)).collect();
			assert_eq!(responsive, [(pid, Some(false))]);
			released(&connections, attempt, "world probe");
		}
	}
}
