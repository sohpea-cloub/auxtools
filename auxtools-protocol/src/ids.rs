use serde::{Deserialize, Serialize};

use crate::{Request, Response};

pub type RequestId = u64;

#[derive(Serialize, Deserialize, Debug)]
pub struct ClientMessage<R = Request> {
	pub id: RequestId,
	pub request: R
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ServerMessage<R = Response> {
	pub reply_to: Option<RequestId>,
	pub response: R
}

impl<R> ServerMessage<R> {
	pub const fn event(response: R) -> Self {
		Self { reply_to: None, response }
	}

	pub const fn answer(to: RequestId, response: R) -> Self {
		Self {
			reply_to: Some(to),
			response
		}
	}
}

#[derive(Debug)]
pub enum Sorted {
	Answer(RequestId, Response),
	Event(Response),
	Late(RequestId, Response)
}

#[derive(Debug, Default)]
pub struct Pending {
	last: RequestId,
	waiting: Vec<RequestId>
}

impl Pending {
	pub const fn new() -> Self {
		Self { last: 0, waiting: vec![] }
	}

	pub fn ask(&mut self) -> RequestId {
		let id = self.tell();
		self.waiting.push(id);
		id
	}

	pub const fn tell(&mut self) -> RequestId {
		self.last += 1;
		self.last
	}

	pub fn give_up(&mut self, id: RequestId) {
		self.waiting.retain(|waiting| *waiting != id);
	}

	pub fn waits_for(&self, id: RequestId) -> bool {
		self.waiting.contains(&id)
	}

	pub fn sort(&mut self, message: ServerMessage) -> Sorted {
		match message.reply_to {
			None => Sorted::Event(message.response),
			Some(id) if self.waits_for(id) => {
				self.give_up(id);
				Sorted::Answer(id, message.response)
			}
			Some(id) => Sorted::Late(id, message.response)
		}
	}
}

#[cfg(test)]
mod tests {
	use std::io::Cursor;

	use super::{ClientMessage, Pending, ServerMessage, Sorted};
	use crate::{read, write, Request, Response};

	#[test]
	fn an_answer_goes_to_the_request_with_its_id() {
		let mut pending = Pending::new();
		let first = pending.ask();
		let second = pending.ask();
		assert_ne!(first, second);
		assert!(matches!(
			pending.sort(ServerMessage::answer(second, Response::Ack)),
			Sorted::Answer(id, Response::Ack) if id == second
		));
		assert!(matches!(
			pending.sort(ServerMessage::answer(first, Response::Resumed)),
			Sorted::Answer(id, Response::Resumed) if id == first
		));
		assert!(!pending.waits_for(first) && !pending.waits_for(second));
	}

	#[test]
	fn a_late_answer_is_not_the_next_requests() {
		let mut pending = Pending::new();
		let timed_out = pending.ask();
		pending.give_up(timed_out);
		let next = pending.ask();
		assert!(matches!(
			pending.sort(ServerMessage::answer(timed_out, Response::StdDef(None))),
			Sorted::Late(id, Response::StdDef(None)) if id == timed_out
		));
		assert!(pending.waits_for(next));
		assert!(matches!(
			pending.sort(ServerMessage::answer(next, Response::Ack)),
			Sorted::Answer(id, Response::Ack) if id == next
		));
		assert!(matches!(pending.sort(ServerMessage::answer(next, Response::Ack)), Sorted::Late(..)));
	}

	#[test]
	fn the_answer_to_a_request_sent_without_waiting_is_late() {
		let mut pending = Pending::new();
		let told = pending.tell();
		let asked = pending.ask();
		assert!(matches!(pending.sort(ServerMessage::answer(told, Response::Ack)), Sorted::Late(..)));
		assert!(matches!(pending.sort(ServerMessage::answer(asked, Response::Ack)), Sorted::Answer(..)));
	}

	#[test]
	fn an_event_is_nobodys_answer() {
		let mut pending = Pending::new();
		let asked = pending.ask();
		assert!(matches!(pending.sort(ServerMessage::event(Response::Ack)), Sorted::Event(Response::Ack)));
		assert!(matches!(
			pending.sort(ServerMessage::event(Response::Resumed)),
			Sorted::Event(Response::Resumed)
		));
		assert!(pending.waits_for(asked));
	}

	#[test]
	fn a_borrowed_message_is_read_as_an_owned_one() {
		let mut wire = vec![];
		let request = Request::Stacks { sleeping: Some(3) };
		write(&mut wire, &ClientMessage { id: 7, request: &request }).unwrap();
		write(&mut wire, &ServerMessage::answer(7, &Response::Ack)).unwrap();
		write(&mut wire, &ServerMessage::event(&Response::Resumed)).unwrap();
		let mut wire = Cursor::new(wire);
		let asked: ClientMessage = read(&mut wire).unwrap();
		assert!(matches!(asked, ClientMessage {
			id: 7,
			request: Request::Stacks { sleeping: Some(3) }
		}));
		let answer: ServerMessage = read(&mut wire).unwrap();
		assert!(matches!(answer, ServerMessage {
			reply_to: Some(7),
			response: Response::Ack
		}));
		let event: ServerMessage = read(&mut wire).unwrap();
		assert!(matches!(event, ServerMessage {
			reply_to: None,
			response: Response::Resumed
		}));
	}
}
