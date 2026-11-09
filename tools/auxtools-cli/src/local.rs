//! Reaching worlds that run the debug library. Each listens on a local socket
//! (a named pipe on Windows) named after its process, for any number of
//! clients: connecting needs only the process id. A reboot lets the clients
//! go, to connect again; one that connects while the world reboots waits for
//! it to load.

use std::{
	io::{self, Read, Write},
	sync::Arc,
	time::Duration
};

pub use interprocess::local_socket::Stream;
use interprocess::{
	local_socket::{prelude::*, ConnectOptions, GenericNamespaced},
	ConnectWaitMode
};

use crate::server_types::{world_socket_name, Request, Response};

/// The name a world's process listens on
pub fn world_name(pid: u32) -> String {
	world_socket_name(pid)
}

/// Connects to the world in process `pid`, if it's listening
pub fn connect(pid: u32) -> io::Result<Stream> {
	// A named pipe can be there with no instance free, and waiting for one
	// can last forever when the world puts it away
	ConnectOptions::new()
		.name(world_name(pid).to_ns_name::<GenericNamespaced>()?)
		.wait_mode(ConnectWaitMode::Timeout(Duration::from_secs(1)))
		.connect_sync()
}

/// A connection read on one thread and written on another, which it allows
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

/// Sends a message to the debug server
pub fn send(stream: &mut impl Write, request: &Request) -> io::Result<()> {
	let data = bincode::serialize(request).map_err(io::Error::other)?;
	stream.write_all(&(data.len() as u32).to_le_bytes())?;
	stream.write_all(&data)?;
	stream.flush()
}

/// Reads a message from the debug server
pub fn receive(stream: &mut impl Read) -> io::Result<Response> {
	// The debug server limits requests to this, and nothing it sends is bigger
	const MAX_MESSAGE_LEN: u32 = 256 * 1024 * 1024;

	let mut len = [0u8; 4];
	stream.read_exact(&mut len)?;
	let len = u32::from_le_bytes(len);
	if len > MAX_MESSAGE_LEN {
		return Err(io::Error::other(format!("a {} byte message is too big to be real", len)));
	}
	let mut data = vec![0; len as usize];
	stream.read_exact(&mut data)?;
	bincode::deserialize(&data).map_err(io::Error::other)
}

/// Whether process `pid` is still running
#[cfg(unix)]
pub fn alive(pid: u32) -> bool {
	// Our own children are waited for, so a zombie is never one of ours for
	// long. Signal 0 only checks.
	unsafe { libc::kill(pid as i32, 0) == 0 }
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

/// Ends process `pid`
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
