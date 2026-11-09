//! The Debug Adapter Protocol's framing: JSON messages, each after a
//! `Content-Length` header, on stdin and stdout.

use std::{
	io::{self, BufRead, Write},
	sync::{
		atomic::{AtomicI64, Ordering},
		Mutex
	}
};

use dap_types::{Event, EventMessage, OutputEvent, ProtocolMessage, ResponseMessage};

/// Reads one message, or `None` at the end of the input.
pub fn read_message(input: &mut impl BufRead) -> io::Result<Option<String>> {
	let mut length = None;
	loop {
		let mut line = String::new();
		if input.read_line(&mut line)? == 0 {
			return Ok(None);
		}
		let line = line.trim_end();
		if line.is_empty() {
			break;
		}
		if let Some((name, value)) = line.split_once(':') {
			if name.trim().eq_ignore_ascii_case("content-length") {
				length = value.trim().parse::<usize>().ok();
			}
		}
	}

	let length = length.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "a message without a Content-Length"))?;
	let mut content = vec![0; length];
	input.read_exact(&mut content)?;
	String::from_utf8(content)
		.map(Some)
		.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Where messages to the client go. Shared by every thread that has
/// something to say.
pub struct Output {
	seq: AtomicI64,
	stdout: Mutex<io::Stdout>
}

impl Output {
	pub fn new() -> Self {
		Self {
			seq: AtomicI64::new(1),
			stdout: Mutex::new(io::stdout())
		}
	}

	fn protocol_message(&self, type_: &str) -> ProtocolMessage {
		ProtocolMessage {
			seq: self.seq.fetch_add(1, Ordering::Relaxed),
			type_: type_.to_owned()
		}
	}

	fn send(&self, message: &impl serde::Serialize) {
		let json = serde_json::to_string(message).expect("a DAP message that can't be encoded");
		let mut stdout = self.stdout.lock().unwrap();
		// Nobody to tell if the client has gone
		let _ = write!(stdout, "Content-Length: {}\r\n\r\n{}", json.len(), json);
		let _ = stdout.flush();
	}

	pub fn event<E: Event>(&self, event: E) {
		let body = serde_json::to_value(event).expect("a DAP event that can't be encoded");
		self.send(&EventMessage {
			protocol_message: self.protocol_message(EventMessage::TYPE),
			event: E::EVENT.to_owned(),
			body: Some(body)
		});
	}

	pub fn response(&self, request_seq: i64, command: &str, result: Result<serde_json::Value, String>) {
		let (success, message, body) = match result {
			Ok(body) => (true, None, Some(body)),
			Err(message) => (false, Some(message), None)
		};
		self.send(&ResponseMessage {
			protocol_message: self.protocol_message(ResponseMessage::TYPE),
			request_seq,
			success,
			command: command.to_owned(),
			message,
			body
		});
	}

	/// A line in the debug console
	pub fn console(&self, text: impl Into<String>) {
		self.output("console", text.into() + "\n");
	}

	pub fn output(&self, category: &str, output: String) {
		self.event(OutputEvent {
			output,
			category: Some(category.to_owned()),
			..Default::default()
		});
	}
}
