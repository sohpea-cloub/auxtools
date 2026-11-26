pub mod ops;

use std::{
	io,
	ops::ControlFlow,
	sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
	thread::JoinHandle,
	time::{Duration, Instant}
};

use auxtools_protocol::{ClientMessage, Hello, Pending, Request, RequestId, Response, ServerMessage, Sorted, PROTOCOL_VERSION};

use crate::local::{self, Shared, Stream};

#[derive(Debug)]
pub enum Lost {
	Incompatible(u32),
	Unsent(io::Error),
	Closed
}

#[derive(Debug)]
pub enum Heard {
	Answer { to: RequestId, response: Response, after: u64 },
	Event(Response)
}

pub trait Listener: Send + 'static {
	fn event(&mut self, number: u64, event: Response) -> ControlFlow<()>;

	fn closed(self: Box<Self>, error: Option<io::Error>);
}

enum Read {
	Hello(Hello),
	Message { message: ServerMessage, events_read: u64 }
}

pub struct Client {
	writer: Shared,
	read: Receiver<Read>,
	reader: Option<JoinHandle<()>>,
	greeted: bool,
	requests: Pending
}

impl Client {
	pub fn open(pid: u32) -> io::Result<Self> {
		Self::start(local::open(pid)?, false, None, "world reader")
	}

	pub fn connect(pid: u32) -> io::Result<Self> {
		Self::start(local::connect(pid)?, true, None, "world reader")
	}

	pub fn on(stream: Stream) -> io::Result<Self> {
		Self::start(stream, false, None, "world reader")
	}

	pub fn probing(stream: Stream) -> io::Result<Self> {
		Self::start(stream, false, None, "probe reader")
	}

	pub fn listened_to(stream: Stream, listener: impl Listener) -> io::Result<Self> {
		Self::start(stream, true, Some(Box::new(listener)), "world reader")
	}

	fn start(stream: Stream, greeted: bool, listener: Option<Box<dyn Listener>>, reader_name: &str) -> io::Result<Self> {
		let writer = Shared::new(stream);
		let reader = writer.clone();
		let (sender, read) = mpsc::channel();
		let reader = std::thread::Builder::new()
			.name(reader_name.to_owned())
			.spawn(move || read_all(reader, greeted, &sender, listener))?;
		Ok(Self {
			writer,
			read,
			reader: Some(reader),
			greeted,
			requests: Pending::new()
		})
	}

	fn next(&mut self, until: Option<Instant>) -> Result<Option<Read>, Lost> {
		match until {
			Some(until) => {
				let Some(left) = until.checked_duration_since(Instant::now()).filter(|left| !left.is_zero()) else {
					return Ok(None);
				};
				match self.read.recv_timeout(left) {
					Ok(read) => Ok(Some(read)),
					Err(RecvTimeoutError::Timeout) => Ok(None),
					Err(RecvTimeoutError::Disconnected) => Err(Lost::Closed)
				}
			}
			None => self.read.recv().map(Some).map_err(|_| Lost::Closed)
		}
	}

	fn greet(&mut self, hello: &Hello) -> Result<(), Lost> {
		if hello.protocol != PROTOCOL_VERSION {
			return Err(Lost::Incompatible(hello.protocol));
		}
		self.greeted = true;
		Ok(())
	}

	pub fn greeted_by(&mut self, until: Option<Instant>) -> Result<bool, Lost> {
		while !self.greeted {
			match self.next(until)? {
				Some(Read::Hello(hello)) => self.greet(&hello)?,
				Some(Read::Message { .. }) => {}
				None => return Ok(false)
			}
		}
		Ok(true)
	}

	fn write(&mut self, id: RequestId, request: &Request) -> io::Result<()> {
		auxtools_protocol::write(&mut self.writer, &ClientMessage { id, request })
	}

	pub fn ask(&mut self, request: &Request, until: Option<Instant>) -> Result<Option<RequestId>, Lost> {
		if !self.greeted_by(until)? {
			return Ok(None);
		}
		let id = self.requests.ask();
		match self.write(id, request) {
			Ok(()) => Ok(Some(id)),
			Err(e) => {
				self.requests.give_up(id);
				Err(Lost::Unsent(e))
			}
		}
	}

	pub fn tell(&mut self, request: &Request, until: Option<Instant>) -> Result<bool, Lost> {
		if !self.greeted_by(until)? {
			return Ok(false);
		}
		let id = self.requests.tell();
		self.write(id, request).map_err(Lost::Unsent)?;
		Ok(true)
	}

	pub fn give_up(&mut self, id: RequestId) {
		self.requests.give_up(id);
	}

	pub fn hear(&mut self, until: Option<Instant>) -> Result<Option<Heard>, Lost> {
		loop {
			let Some(read) = self.next(until)? else {
				return Ok(None);
			};
			let (message, events) = match read {
				Read::Hello(hello) => {
					self.greet(&hello)?;
					continue;
				}
				Read::Message { message, events_read } => (message, events_read)
			};
			match self.requests.sort(message) {
				Sorted::Answer(to, response) => return Ok(Some(Heard::Answer { to, response, after: events })),
				Sorted::Late(..) => {}
				Sorted::Event(response) => return Ok(Some(Heard::Event(response)))
			}
		}
	}

	pub fn leave(&mut self) -> Result<(), Lost> {
		if self.greeted {
			self.tell(&Request::Disconnect, None)?;
		}
		Ok(())
	}

	pub fn hang_up(&self) {
		self.writer.hang_up();
	}

	pub fn close(&mut self) {
		self.hang_up();
		if let Some(reader) = self.reader.take() {
			let _ = reader.join();
		}
	}

	pub fn closed_by(&mut self, until: Instant) -> bool {
		loop {
			match self.hear(Some(until)) {
				Ok(Some(_)) => {}
				Ok(None) => return false,
				Err(_) => return true
			}
		}
	}
}

fn read_all(mut reader: Shared, greeted: bool, read: &Sender<Read>, mut listener: Option<Box<dyn Listener>>) {
	let failed = read_on(&mut reader, greeted, read, listener.as_deref_mut()).err();
	if let Some(listener) = listener {
		listener.closed(failed);
	}
}

fn read_on(reader: &mut Shared, greeted: bool, read: &Sender<Read>, mut listener: Option<&mut (dyn Listener + 'static)>) -> io::Result<()> {
	if !greeted {
		let hello = auxtools_protocol::read(reader)?;
		if read.send(Read::Hello(hello)).is_err() {
			return Ok(());
		}
	}
	let mut events_read = 0;
	loop {
		let message: ServerMessage = auxtools_protocol::read(reader)?;
		if message.reply_to.is_none() {
			events_read += 1;
			if let Some(listener) = &mut listener {
				if listener.event(events_read, message.response).is_break() {
					return Ok(());
				}
				continue;
			}
		}
		if read.send(Read::Message { message, events_read }).is_err() {
			return Ok(());
		}
	}
}

pub const LISTEN_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, PartialEq, Eq)]
pub enum NotListening {
	Gone,
	TimedOut
}

pub fn when_listening<T, E>(pid: u32, until: Option<Instant>, mut connect: impl FnMut(u32) -> Result<T, E>) -> Result<T, NotListening> {
	loop {
		if let Ok(connected) = connect(pid) {
			return Ok(connected);
		}
		if !local::alive(pid) {
			return Err(NotListening::Gone);
		}
		if until.is_some_and(|until| Instant::now() >= until) {
			return Err(NotListening::TimedOut);
		}
		std::thread::sleep(LISTEN_INTERVAL);
	}
}

#[cfg(test)]
mod tests {
	use std::{
		io,
		ops::ControlFlow,
		sync::mpsc,
		time::{Duration, Instant}
	};

	use auxtools_protocol::{ClientMessage, EvalError, Hello, Request, Response, ServerMessage, PROTOCOL_VERSION};
	use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions};

	use super::{when_listening, Client, Heard, Listener, Lost, NotListening, Read};
	use crate::local::Stream;

	fn connection(name: &str) -> (Stream, Stream) {
		let name = format!("auxtools-cli-test-{}-{}", std::process::id(), name);
		let listener = ListenerOptions::new()
			.name(name.clone().to_ns_name::<GenericNamespaced>().unwrap())
			.create_sync()
			.unwrap();
		let client = Stream::connect(name.to_ns_name::<GenericNamespaced>().unwrap()).unwrap();
		(client, listener.accept().unwrap())
	}

	fn read_request(world: &mut Stream) -> ClientMessage {
		auxtools_protocol::read(world).unwrap()
	}

	fn answer(world: &mut Stream, to: &ClientMessage, response: &Response) {
		auxtools_protocol::write(world, &ServerMessage::answer(to.id, response)).unwrap();
	}

	fn event(world: &mut Stream, response: &Response) {
		auxtools_protocol::write(world, &ServerMessage::event(response)).unwrap();
	}

	fn soon() -> Option<Instant> {
		Some(Instant::now() + Duration::from_secs(10))
	}

	#[test]
	fn nothing_is_sent_before_the_hello() {
		let (client, mut world) = connection("hello");
		let mut client = Client::on(client).unwrap();
		let shortly = Some(Instant::now() + Duration::from_millis(50));
		assert!(matches!(client.ask(&Request::Status, shortly), Ok(None)));
		assert!(matches!(client.tell(&Request::Pause, shortly), Ok(false)));
		assert!(!client.greeted);
		client.leave().unwrap();

		auxtools_protocol::write(&mut world, &Hello::new("4.0.0")).unwrap();
		let id = client.ask(&Request::Status, soon()).unwrap().unwrap();
		let asked = read_request(&mut world);
		assert!(matches!(asked.request, Request::Status));
		assert_eq!(asked.id, id);
		answer(&mut world, &asked, &Response::Status { paused: None });
		assert!(matches!(
			client.hear(soon()),
			Ok(Some(Heard::Answer { to, response: Response::Status { paused: None }, after: 0 })) if to == id
		));
		client.leave().unwrap();
		assert!(matches!(read_request(&mut world).request, Request::Disconnect));
	}

	#[test]
	fn a_hello_of_another_protocol_is_not_answered() {
		let (client, mut world) = connection("other");
		let mut client = Client::on(client).unwrap();
		let mut hello = Hello::new("9.0.0");
		hello.protocol = PROTOCOL_VERSION + 1;
		auxtools_protocol::write(&mut world, &hello).unwrap();
		assert!(matches!(client.ask(&Request::Status, soon()), Err(Lost::Incompatible(theirs)) if theirs == PROTOCOL_VERSION + 1));
		assert!(!client.greeted);
		client.leave().unwrap();
		client.hang_up();
		drop(client);
		let mut sent = vec![];
		io::Read::read_to_end(&mut world, &mut sent).unwrap();
		assert!(sent.is_empty(), "the client sent {:?}", sent);
	}

	#[test]
	fn an_answer_nobody_waits_for_isnt_taken_for_the_next() {
		let (client, mut world) = connection("late");
		let mut client = Client::on(client).unwrap();
		auxtools_protocol::write(&mut world, &Hello::new("4.0.0")).unwrap();

		assert!(client.tell(&Request::Configured, soon()).unwrap());
		let given_up = client.ask(&Request::Uptime, soon()).unwrap().unwrap();
		client.give_up(given_up);
		let waited = client.ask(&Request::Status, soon()).unwrap().unwrap();

		let told = read_request(&mut world);
		let slow = read_request(&mut world);
		let status = read_request(&mut world);
		answer(&mut world, &told, &Response::Ack);
		answer(&mut world, &slow, &Response::Ack);
		event(&mut world, &Response::Ack);
		answer(&mut world, &status, &Response::Status { paused: None });

		assert!(matches!(client.hear(soon()), Ok(Some(Heard::Event(Response::Ack)))));
		assert!(matches!(
			client.hear(soon()),
			Ok(Some(Heard::Answer { to, response: Response::Status { paused: None }, after: 1 })) if to == waited
		));
		drop(world);
		assert!(matches!(client.hear(soon()), Err(Lost::Closed)));
		assert!(matches!(client.ask(&Request::Status, soon()), Err(Lost::Unsent(_))));
	}

	#[test]
	fn a_queued_event_is_not_read_after_the_deadline() {
		let (client, _world) = connection("deadline");
		let mut client = Client::on(client).unwrap();
		let (queue, read) = mpsc::channel();
		client.read = read;
		client.greeted = true;
		let message = ServerMessage::event(Response::Resumed);
		queue.send(Read::Message { message, events_read: 1 }).unwrap();
		assert!(matches!(client.hear(Some(Instant::now())), Ok(None)));
		assert!(matches!(client.hear(soon()), Ok(Some(Heard::Event(Response::Resumed)))));
	}

	struct Passes(mpsc::Sender<Result<(u64, Response), bool>>);

	impl Listener for Passes {
		fn event(&mut self, number: u64, event: Response) -> ControlFlow<()> {
			let last = matches!(event, Response::Disconnect);
			let _ = self.0.send(Ok((number, event)));
			if last {
				ControlFlow::Break(())
			} else {
				ControlFlow::Continue(())
			}
		}

		fn closed(self: Box<Self>, error: Option<io::Error>) {
			let _ = self.0.send(Err(error.is_some()));
		}
	}

	#[test]
	fn a_listener_has_the_events_and_an_answer_says_how_many_came_first() {
		let (client, mut world) = connection("listener");
		let (events, heard) = mpsc::channel();
		let mut client = Client::listened_to(client, Passes(events)).unwrap();

		let id = client.ask(&Request::Status, None).unwrap().unwrap();
		let asked = read_request(&mut world);
		event(&mut world, &Response::Resumed);
		event(&mut world, &Response::StepEnded);
		answer(&mut world, &asked, &Response::Ack);
		event(&mut world, &Response::CodeChanged);
		assert!(matches!(
			client.hear(soon()),
			Ok(Some(Heard::Answer { to, response: Response::Ack, after: 2 })) if to == id
		));
		let slept = EvalError {
			message: "slept".to_owned(),
			still_running: true
		};
		answer(&mut world, &asked, &Response::Eval(Err(slept)));
		event(&mut world, &Response::Disconnect);
		for (number, expected) in [(1, "Resumed"), (2, "StepEnded"), (3, "CodeChanged"), (4, "Disconnect")] {
			let (at, event) = heard.recv().unwrap().unwrap();
			assert_eq!((at, format!("{:?}", event).as_str()), (number, expected));
		}
		assert!(!heard.recv().unwrap().unwrap_err());
		assert!(client.closed_by(Instant::now() + Duration::from_secs(10)));
	}

	#[test]
	fn a_listener_is_told_when_the_connection_ends() {
		let (client, world) = connection("ends");
		let (events, heard) = mpsc::channel();
		let mut client = Client::listened_to(client, Passes(events)).unwrap();
		drop(world);
		assert!(heard.recv().unwrap().unwrap_err());
		assert!(matches!(client.hear(None), Err(Lost::Closed)));
	}

	#[test]
	fn a_process_that_is_gone_is_not_waited_for() {
		let nobody = u32::MAX - 1;
		let mut tries = 0;
		let found = when_listening(nobody, None, |_| {
			tries += 1;
			Err::<(), _>("nothing listens")
		});
		assert_eq!((found, tries), (Err(NotListening::Gone), 1));
		let this = std::process::id();
		let mut tries = 0;
		let found = when_listening(this, None, |pid| {
			tries += 1;
			if tries < 3 {
				Err("not yet")
			} else {
				Ok(pid)
			}
		});
		assert_eq!((found, tries), (Ok(this), 3));
		let found = when_listening(this, Some(Instant::now()), |_| Err::<(), _>("never"));
		assert_eq!(found, Err(NotListening::TimedOut));
	}
}
