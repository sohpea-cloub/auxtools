use super::*;

pub(super) type ClientId = u32;

pub(super) enum Event {
	Connected(ClientId, Connection),
	Request(ClientId, RequestId, Request),
	Disconnected(ClientId)
}

pub(super) struct Client {
	pub(super) uncaught: Option<RuntimeFilter>,
	pub(super) caught: Option<RuntimeFilter>,
	pub(super) debugs: bool,
	pub(super) follow: Option<Vec<JournalKind>>,
	pub(super) outbox: mpsc::SyncSender<Vec<u8>>,
	pub(super) writer: Option<thread::JoinHandle<()>>,
	pub(super) hold_max: Option<Duration>
}

#[derive(Clone, Copy)]
pub(super) struct Asking {
	pub(super) client: ClientId,
	pub(super) id: Option<RequestId>
}

const OUTBOX: usize = 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Wait {
	No,
	WhileClients,
	Forever,
	Until(Option<Instant>)
}

struct Acceptor {
	events: mpsc::Sender<Event>,
	waiting: Option<mpsc::Receiver<Event>>
}

static ACCEPTOR: Mutex<Option<Acceptor>> = Mutex::new(None);

pub(super) struct Sessions {
	pub(super) events: mpsc::Receiver<Event>,
	pub(super) clients: BTreeMap<ClientId, Client>,
	pub(super) asking: Option<Asking>,
	pub(super) reloader: Option<(ClientId, String)>,
	pub(super) profiling: Option<ClientId>,
	pub(super) hold: Option<ClientId>,
	pub(super) hold_max: Option<Duration>
}

impl Sessions {
	pub(super) fn listen() -> std::io::Result<Sessions> {
		let events = Self::take_events()?;
		Ok(Sessions {
			events,
			clients: BTreeMap::new(),
			asking: None,
			reloader: None,
			profiling: None,
			hold: None,
			hold_max: None
		})
	}

	fn take_events() -> std::io::Result<mpsc::Receiver<Event>> {
		let mut acceptor = ACCEPTOR.lock().unwrap();
		if acceptor.is_none() {
			let listener = connection::listen(&connection::world_name())?;
			let (events, waiting) = mpsc::channel();
			*acceptor = Some(Acceptor {
				events,
				waiting: Some(waiting)
			});
			thread::Builder::new()
				.name("debug server listener".to_owned())
				.spawn(move || Self::accept(&listener))?;
			waker::start()?;
		}
		acceptor
			.as_mut()
			.and_then(|acceptor| acceptor.waiting.take())
			.ok_or_else(|| std::io::Error::other("there's a debug server already"))
	}

	fn accept(listener: &connection::Listener) {
		let mut next_id = 0;
		loop {
			let Ok(connection) = listener.accept() else {
				continue;
			};
			next_id += 1;

			let id = next_id;
			thread::spawn(move || Self::serve(id, connection));
		}
	}

	fn serve(id: ClientId, mut connection: Connection) {
		if auxtools_protocol::write(&mut connection, &Hello::new(env!("CARGO_PKG_VERSION"))).is_err() {
			return;
		}

		let events = {
			let acceptor = ACCEPTOR.lock().unwrap();
			let Some(acceptor) = acceptor.as_ref() else {
				return;
			};
			let _ = acceptor.events.send(Event::Connected(id, connection.clone()));
			waker::event_sent();
			acceptor.events.clone()
		};
		ServerThread { events }.run(id, connection);
	}

	pub(super) fn refuse(reason: String) {
		if let Ok(events) = Self::take_events() {
			Self::refuse_on(events, reason);
		}
	}

	pub(super) fn give_up(&mut self, reason: String) {
		self.clients.clear();
		self.hold = None;
		self.hold_max = None;
		let (_, nothing) = mpsc::channel();
		Self::refuse_on(std::mem::replace(&mut self.events, nothing), reason);
	}

	fn refuse_on(events: mpsc::Receiver<Event>, reason: String) {
		thread::spawn(move || {
			while let Ok(event) = events.recv() {
				if let Event::Connected(_, mut connection) = event {
					for response in [Response::Unavailable { reason: reason.clone() }, Response::Disconnect] {
						if Self::say(&mut connection, &response).is_err() {
							break;
						}
					}
				}
			}
		});
	}

	pub(super) fn requester(&self) -> Option<ClientId> {
		self.asking.map(|asking| asking.client)
	}

	pub(super) const fn request_over(&mut self) {
		self.asking = None;
	}

	pub(super) fn has_debuggers(&self) -> bool {
		self.clients.values().any(|client| client.debugs)
	}

	pub(super) fn claim_reloads(&mut self, who: String) -> Result<(), String> {
		let id = self.requester().ok_or("no client")?;
		match &self.reloader {
			Some((holder, theirs)) if *holder != id && self.clients.contains_key(holder) => Err(format!(
				"{} already owns hot reload for this world. A second watcher would apply each change twice",
				theirs
			)),
			_ => {
				self.reloader = Some((id, who));
				Ok(())
			}
		}
	}

	pub(super) fn reloads_refused(&self) -> Option<String> {
		let (holder, who) = self.reloader.as_ref()?;
		(Some(*holder) != self.requester() && self.clients.contains_key(holder)).then(|| format!("{} already owns hot reload for this world", who))
	}

	pub(super) fn deliver(client: &mut Client, response: &Response) -> bool {
		Self::post(client, None, response)
	}

	pub(super) fn post(client: &mut Client, reply_to: Option<RequestId>, response: &Response) -> bool {
		auxtools_protocol::frame(&ServerMessage { reply_to, response }).is_ok_and(|bytes| client.outbox.try_send(bytes).is_ok())
	}

	pub(super) fn say(connection: &mut Connection, response: &Response) -> std::io::Result<()> {
		auxtools_protocol::write(connection, &ServerMessage::event(response))
	}

	pub(super) fn writer(mut connection: Connection) -> (mpsc::SyncSender<Vec<u8>>, thread::JoinHandle<()>) {
		let (outbox, sent) = mpsc::sync_channel::<Vec<u8>>(OUTBOX);
		let writer = thread::spawn(move || {
			while let Ok(bytes) = sent.recv() {
				if connection.write_all(&bytes).and_then(|()| connection.flush()).is_err() {
					return;
				}
			}
			let _ = Self::say(&mut connection, &Response::Disconnect);
		});
		(outbox, writer)
	}
}

impl Server {
	pub(super) fn recv_request(&mut self, wait: Wait) -> Option<Request> {
		loop {
			let event = match wait {
				Wait::No => self.sessions.events.try_recv().ok()?,
				Wait::Until(Some(until)) => self.sessions.events.recv_timeout(until.saturating_duration_since(Instant::now())).ok()?,
				_ => self.sessions.events.recv().ok()?
			};

			match event {
				Event::Connected(id, connection) => {
					let (outbox, writer) = Sessions::writer(connection.clone());
					self.sessions.clients.insert(id, Client {
						uncaught: None,
						caught: None,
						debugs: true,
						follow: None,
						outbox,
						writer: Some(writer),
						hold_max: None
					});
					if let Wait::Until(_) = wait {
						return None;
					}
				}

				Event::Disconnected(id) => {
					self.remove_client(id);

					if (wait == Wait::WhileClients && !self.sessions.has_debuggers()) || matches!(wait, Wait::Until(_)) {
						return None;
					}
				}

				Event::Request(id, ..) if !self.sessions.clients.contains_key(&id) => waker::request_taken(),

				Event::Request(id, request_id, request) => {
					waker::request_taken();
					self.sessions.asking = Some(Asking {
						client: id,
						id: Some(request_id)
					});
					return Some(request);
				}
			}
		}
	}

	pub fn notify<T: Into<String>>(&mut self, message: T) {
		let message = message.into();
		if self.sessions.clients.is_empty() {
			eprintln!("Debug Server: {}", message);
		}

		self.broadcast(Response::Notification { message }, None);
	}

	pub(super) fn answer(&mut self, response: Response) {
		let Some(asking) = self.sessions.asking.as_mut() else {
			return;
		};
		let Some(request) = asking.id.take() else {
			return;
		};
		let id = asking.client;
		let Some(client) = self.sessions.clients.get_mut(&id) else {
			return;
		};

		if !Sessions::post(client, Some(request), &response) {
			eprintln!("Debug server failed to send a message to a client");
			self.remove_client(id);
		}
	}

	pub(super) fn broadcast(&mut self, response: Response, except: Option<ClientId>) {
		let mut gone = vec![];
		for (&id, client) in &mut self.sessions.clients {
			if Some(id) != except && !Sessions::deliver(client, &response) {
				gone.push(id);
			}
		}
		for id in gone {
			self.remove_client(id);
		}
	}

	pub(super) fn tell(&mut self, client: ClientId, response: Response) {
		if let Some(to) = self.sessions.clients.get_mut(&client) {
			if !Sessions::deliver(to, &response) {
				self.remove_client(client);
			}
		}
	}

	pub(super) fn remove_client(&mut self, id: ClientId) {
		let Some(client) = self.sessions.clients.remove(&id) else {
			return;
		};
		drop(client);
		if self.sessions.requester() == Some(id) {
			self.sessions.request_over();
		}
		if self.sessions.hold == Some(id) && self.pause.paused.is_none() {
			self.sessions.hold = None;
			self.sessions.hold_max = None;
		}

		self.watches.forget_watches_of(id);
		if self.sessions.profiling == Some(id) {
			self.sessions.profiling = None;
			crate::profiler::discard();
		}

		self.breakpoints.forget_client(id);

		if self.sessions.clients.is_empty() {
			self.breakpoints.clear();
		}
	}

	pub(super) fn disconnect_all(&mut self) {
		let ids: Vec<ClientId> = self.sessions.clients.keys().copied().collect();
		for id in ids {
			self.remove_client(id);
		}
	}

	pub fn world_shutting_down(&mut self) {
		self.broadcast(Response::ShuttingDown, None);
		let writers: Vec<_> = self.sessions.clients.values_mut().filter_map(|client| client.writer.take()).collect();
		self.disconnect_all();
		let deadline = Instant::now() + Duration::from_secs(1);
		while writers.iter().any(|writer| !writer.is_finished()) && Instant::now() < deadline {
			thread::sleep(Duration::from_millis(5));
		}
	}
}

impl Drop for Server {
	fn drop(&mut self) {
		self.disconnect_all();

		if let Some(acceptor) = ACCEPTOR.lock().unwrap().as_mut() {
			let (events, waiting) = mpsc::channel();
			acceptor.events = events;
			acceptor.waiting = Some(waiting);
		}

		while let Ok(event) = self.sessions.events.try_recv() {
			if let Event::Connected(_, mut connection) = event {
				thread::spawn(move || Sessions::say(&mut connection, &Response::Disconnect).is_ok());
			}
		}
	}
}

struct ServerThread {
	events: mpsc::Sender<Event>
}

impl ServerThread {
	fn run(&self, id: ClientId, mut stream: Connection) {
		waker::client_connected();

		while let Ok(ClientMessage { id: request_id, request }) = auxtools_protocol::read(&mut stream) {
			if let Request::Disconnect = request {
				break;
			}

			waker::request_waiting();
			if let Err(e) = self.events.send(Event::Request(id, request_id, request)) {
				eprintln!("Debug server thread failed to handle request: {}", e);
				break;
			}
			waker::event_sent();
		}

		waker::client_left();
		let _ = self.events.send(Event::Disconnected(id));
		waker::event_sent();
	}
}
