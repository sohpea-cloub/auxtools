//! The local socket (a named pipe on Windows) the world listens on for its
//! clients: the launcher that started it, launchers that attach to it, and
//! anything else that speaks the debug server's protocol. It's named after the
//! process, so a client only needs the process id to find it.

use std::{
	io::{self, Read, Write},
	sync::Arc
};

use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions, Stream as LocalStream};

use crate::server_types::world_socket_name;

/// The name this world listens on
pub fn world_name() -> String {
	world_socket_name(std::process::id())
}

/// Blocks in accept(), which nothing makes it leave: it lasts as long as the
/// process (see `Server::listen`). A non-blocking named pipe doesn't take
/// connections at all under Wine, and is a mode Windows keeps for LAN Manager.
pub struct Listener(interprocess::local_socket::Listener);

impl Listener {
	/// Waits for a client
	pub fn accept(&self) -> io::Result<Connection> {
		self.0.accept().map(Connection::new)
	}
}

pub fn listen(name: &str) -> io::Result<Listener> {
	Ok(Listener(
		ListenerOptions::new().name(name.to_ns_name::<GenericNamespaced>()?).create_sync()?
	))
}

/// A client's connection, shared by the Server, which writes, and the client's
/// thread, which reads
#[derive(Clone)]
pub struct Connection(Arc<LocalStream>);

impl Connection {
	pub fn new(stream: LocalStream) -> Self {
		Self(Arc::new(stream))
	}

	/// Connects to a listener, as a client would
	#[cfg(test)]
	pub fn connect(name: &str) -> io::Result<Self> {
		Ok(Self::new(LocalStream::connect(name.to_ns_name::<GenericNamespaced>()?)?))
	}
}

impl Read for Connection {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		(&*self.0).read(buf)
	}
}

impl Write for Connection {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		(&*self.0).write(buf)
	}

	fn flush(&mut self) -> io::Result<()> {
		(&*self.0).flush()
	}
}
