//! Keeps a world awake that would sleep because no players are connected
//! (`world.sleep_offline`): a connected debugger counts as a player. A sleeping
//! world runs no code, so the debug server would never get to answer. BYOND
//! keeps the world awake while there's a connection to its port, even one that
//! says nothing, so when a client is connected (or its request is waiting) and
//! no code has run for a while, this connects, and stays until the last client
//! has gone. An awake world always runs some code: the library's loop that
//! sleeps a tick at a time.

use std::{
	net::{Ipv4Addr, SocketAddr, TcpStream},
	sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering},
	thread,
	time::{Duration, Instant}
};

/// Bumped by the world's thread for each instruction the debug server sees
static ALIVE: AtomicU32 = AtomicU32::new(0);
/// Requests read off clients' connections and not yet taken by the server
static WAITING: AtomicU32 = AtomicU32::new(0);
/// Clients connected
static CLIENTS: AtomicU32 = AtomicU32::new(0);
/// The world's port, once it's known
static PORT: AtomicU16 = AtomicU16::new(0);
/// Set when the world was woken, for the server to tell its clients
static WOKE: AtomicBool = AtomicBool::new(false);

/// How long the world can run no code, with a client there, before it's woken
const PATIENCE: Duration = Duration::from_secs(1);
const CHECK_EVERY: Duration = Duration::from_millis(250);

/// Code is running (the world's thread, once per instruction)
pub fn alive() {
	ALIVE.store(ALIVE.load(Ordering::Relaxed).wrapping_add(1), Ordering::Relaxed);
}

pub fn request_waiting() {
	WAITING.fetch_add(1, Ordering::Relaxed);
}

pub fn request_taken() {
	WAITING.fetch_sub(1, Ordering::Relaxed);
}

pub fn client_connected() {
	CLIENTS.fetch_add(1, Ordering::Relaxed);
}

pub fn client_left() {
	CLIENTS.fetch_sub(1, Ordering::Relaxed);
}

pub fn set_port(port: u16) {
	PORT.store(port, Ordering::Relaxed);
}

/// Whether the world was woken since this was last asked
pub fn woke() -> bool {
	// Asked on every instruction, so it only writes when it was
	WOKE.load(Ordering::Relaxed) && WOKE.swap(false, Ordering::Relaxed)
}

/// Starts watching, for as long as the process runs
pub fn start() -> std::io::Result<()> {
	thread::Builder::new().name("debug server waker".to_owned()).spawn(watch)?;
	Ok(())
}

fn wanted() -> bool {
	WAITING.load(Ordering::Relaxed) > 0 || CLIENTS.load(Ordering::Relaxed) > 0
}

fn watch() {
	let mut last_alive = ALIVE.load(Ordering::Relaxed);
	let mut quiet_since = Instant::now();
	let mut connection: Option<TcpStream> = None;
	loop {
		thread::sleep(CHECK_EVERY);

		if !wanted() {
			// The world may sleep again
			connection = None;
		}
		// BYOND hangs up on a connection that never says anything, in time
		if connection.as_ref().is_some_and(|c| !open(c)) {
			connection = None;
		}

		let alive = ALIVE.load(Ordering::Relaxed);
		if alive != last_alive || !wanted() || connection.is_some() {
			last_alive = alive;
			quiet_since = Instant::now();
			continue;
		}
		if quiet_since.elapsed() < PATIENCE {
			continue;
		}

		let port = PORT.load(Ordering::Relaxed);
		if port != 0 {
			let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
			connection = TcpStream::connect_timeout(&address, Duration::from_secs(1)).ok();
			if connection.is_some() {
				WOKE.store(true, Ordering::Relaxed);
			}
		}
		quiet_since = Instant::now();
	}
}

/// Whether the world hasn't hung up
fn open(connection: &TcpStream) -> bool {
	let mut byte = [0u8; 1];
	if connection.set_nonblocking(true).is_err() {
		return false;
	}
	let open = match connection.peek(&mut byte) {
		Ok(0) => false,
		Ok(_) => true,
		Err(e) => e.kind() == std::io::ErrorKind::WouldBlock
	};
	let _ = connection.set_nonblocking(false);
	open
}
