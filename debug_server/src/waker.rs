use std::{
	net::{Ipv4Addr, SocketAddr, TcpStream},
	sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering},
	thread,
	time::{Duration, Instant}
};

static ALIVE: AtomicU32 = AtomicU32::new(0);
static WAITING: AtomicU32 = AtomicU32::new(0);
static CLIENTS: AtomicU32 = AtomicU32::new(0);
static PORT: AtomicU16 = AtomicU16::new(0);
static WOKE: AtomicBool = AtomicBool::new(false);
static EVENTS: AtomicBool = AtomicBool::new(false);

const PATIENCE: Duration = Duration::from_secs(1);
const CHECK_EVERY: Duration = Duration::from_millis(250);

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

pub fn event_sent() {
	EVENTS.store(true, Ordering::SeqCst);
	auxtools::instruction_hook::hear_all();
}

pub fn events_pending() -> bool {
	EVENTS.load(Ordering::Acquire) && EVENTS.swap(false, Ordering::AcqRel)
}

pub fn wants_world() -> bool {
	EVENTS.load(Ordering::SeqCst) || WOKE.load(Ordering::SeqCst)
}

pub fn woke() -> bool {
	WOKE.load(Ordering::Relaxed) && WOKE.swap(false, Ordering::Relaxed)
}

pub fn start() -> std::io::Result<()> {
	thread::Builder::new().name("debug server waker".to_owned()).spawn(watch)?;
	Ok(())
}

fn wanted() -> bool {
	WAITING.load(Ordering::Relaxed) > 0 || CLIENTS.load(Ordering::Relaxed) > 0
}

fn activity() -> u32 {
	ALIVE.load(Ordering::Relaxed).wrapping_add(auxtools::proc_calls())
}

fn watch() {
	let mut last_alive = activity();
	let mut quiet_since = Instant::now();
	let mut connection: Option<TcpStream> = None;
	loop {
		thread::sleep(CHECK_EVERY);

		if !wanted() {
			connection = None;
		}
		if connection.as_ref().is_some_and(|c| !open(c)) {
			connection = None;
		}

		let alive = activity();
		if alive != last_alive || !wanted() || connection.is_some() {
			last_alive = alive;
			quiet_since = Instant::now();
			continue;
		}
		auxtools::instruction_hook::hear_all();
		if quiet_since.elapsed() < PATIENCE {
			continue;
		}

		let port = PORT.load(Ordering::Relaxed);
		if port != 0 {
			let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
			connection = TcpStream::connect_timeout(&address, Duration::from_secs(1)).ok();
			if connection.is_some() {
				WOKE.store(true, Ordering::SeqCst);
				auxtools::instruction_hook::hear_all();
			}
		}
		quiet_since = Instant::now();
	}
}

fn open(connection: &TcpStream) -> bool {
	use std::io::{ErrorKind, Read};

	if connection.set_nonblocking(true).is_err() {
		return false;
	}
	let mut buffer = [0u8; 256];
	let open = loop {
		match (&mut &*connection).read(&mut buffer) {
			Ok(0) => break false,
			Ok(_) => continue,
			Err(e) if e.kind() == ErrorKind::Interrupted => continue,
			Err(e) => break e.kind() == ErrorKind::WouldBlock
		}
	};
	let _ = connection.set_nonblocking(false);
	open
}
