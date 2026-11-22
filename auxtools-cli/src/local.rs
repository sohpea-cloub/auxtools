use std::{
	io::{self, Read, Write},
	path::{Path, PathBuf},
	sync::{Arc, OnceLock},
	time::{Duration, Instant}
};

pub use interprocess::local_socket::Stream;
use interprocess::{
	local_socket::{prelude::*, ConnectOptions, GenericNamespaced},
	ConnectWaitMode
};

use auxtools_protocol::{
	old_socket_name, read, world_socket_name, write, ClientMessage, Hello, Request, Response, ServerMessage, LAST_NAMED_PROTOCOL, PROTOCOL_VERSION
};

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
	let (stream, hello) = greeted(open(pid)?)?;
	if hello.protocol != PROTOCOL_VERSION {
		return Err(incompatible(hello.protocol));
	}
	Ok(stream)
}

pub fn open(pid: u32) -> io::Result<Stream> {
	connect_to(world_name(pid)).map_err(|e| old_protocol(pid).map_or(e, incompatible))
}

pub fn incompatible(theirs: u32) -> io::Error {
	io::Error::other(format!(
		"its debug library speaks protocol {}, and this auxtools speaks {}: use the auxtools that goes with it (the one that started the world), or \
		 start the world with this one",
		theirs, PROTOCOL_VERSION
	))
}

const HELLO_PATIENCE: Duration = Duration::from_secs(5);

fn greeted(stream: Stream) -> io::Result<(Stream, Hello)> {
	let stream = Arc::new(stream);
	let (sender, receiver) = std::sync::mpsc::channel();
	let mut reader = Shared(stream.clone());
	std::thread::spawn(move || {
		let hello = read::<Hello>(&mut reader);
		drop(reader);
		let _ = sender.send(hello);
	});
	let hello = receiver.recv_timeout(HELLO_PATIENCE).map_err(|_| {
		io::Error::new(
			io::ErrorKind::TimedOut,
			format!(
				"its debug library said nothing for {} seconds after the connection opened: the process is stopped or hung",
				HELLO_PATIENCE.as_secs()
			)
		)
	})??;
	let stream = Arc::try_unwrap(stream).map_err(|_| io::Error::other("the connection is still being read"))?;
	Ok((stream, hello))
}

fn old_protocol(pid: u32) -> Option<u32> {
	(1..=LAST_NAMED_PROTOCOL)
		.rev()
		.find(|version| connect_to(old_socket_name(*version, pid)).is_ok())
}

#[derive(Clone)]
pub struct Shared(Arc<Stream>);

impl Shared {
	pub fn new(stream: Stream) -> Self {
		Self(Arc::new(stream))
	}
}

impl Read for Shared {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		(&*self.0).read(buf)
	}
}

impl Write for Shared {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		(&*self.0).write(buf)
	}

	fn flush(&mut self) -> io::Result<()> {
		(&*self.0).flush()
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
	Info(Vec<String>, String),
	Unavailable(String),
	Incompatible(u32)
}

pub const PROBE_PATIENCE: Duration = Duration::from_secs(5);

const TEST_PATIENCE_VAR: &str = "AUXTOOLS_TEST_PATIENCE";

pub fn test_patience(usual: Duration) -> Duration {
	static GIVEN: OnceLock<Option<Duration>> = OnceLock::new();
	let given = GIVEN.get_or_init(|| std::env::var(TEST_PATIENCE_VAR).ok()?.parse().ok().map(Duration::from_millis));
	given.map_or(usual, |given| given.min(usual))
}

pub fn worlds() -> Vec<World> {
	let probes: Vec<_> = server_processes()
		.into_iter()
		.map(|(pid, program)| {
			let answer = connect_to(world_name(pid)).ok().map(probe);
			(pid, program, answer)
		})
		.collect();
	let until = Instant::now() + test_patience(PROBE_PATIENCE);
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
				Some(Ok(Ok(Probed::Info(args, cwd)))) => Some(World {
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
				Some(Err(_)) => Some(unknown(PROTOCOL_VERSION, true, Some(false))),
				Some(Ok(Err(_))) | None => old_protocol(pid).map(|protocol| unknown(protocol, false, None))
			}
		})
		.collect()
}

fn probe(stream: Stream) -> std::sync::mpsc::Receiver<io::Result<Probed>> {
	let (sender, receiver) = std::sync::mpsc::channel();
	std::thread::spawn(move || {
		let mut stream = stream;
		let mut greeted = false;
		let answer = (|| {
			let hello: Hello = read(&mut stream)?;
			if hello.protocol != PROTOCOL_VERSION {
				return Ok(Probed::Incompatible(hello.protocol));
			}
			greeted = true;
			write(&mut stream, &ClientMessage {
				id: 1,
				request: &Request::WorldInfo
			})?;
			loop {
				let message: ServerMessage = read(&mut stream)?;
				match message.response {
					Response::WorldInfo { args, cwd, .. } if message.reply_to == Some(1) => return Ok(Probed::Info(args, cwd)),
					Response::Unavailable { reason } => return Ok(Probed::Unavailable(reason)),
					Response::Disconnect => return Err(io::Error::other("it hung up")),
					_ => {}
				}
			}
		})();
		if greeted {
			let _ = write(&mut stream, &ClientMessage {
				id: 2,
				request: &Request::Disconnect
			});
		}
		let _ = sender.send(answer);
	});
	receiver
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
