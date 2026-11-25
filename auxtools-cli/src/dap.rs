use std::{
	io::{self, BufRead, Write},
	sync::{
		atomic::{AtomicI64, Ordering},
		Mutex
	}
};

use dap_types::{Breakpoint, Event, EventMessage, OutputEvent, ProtocolMessage, ResponseMessage};

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

	let length = length.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length header"))?;
	let mut content = vec![0; length];
	input.read_exact(&mut content)?;
	String::from_utf8(content)
		.map(Some)
		.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub struct Output {
	seq: AtomicI64,
	to: To
}

enum To {
	Client(Mutex<io::Stdout>),
	Plain(Box<dyn Fn(Note) + Send + Sync>)
}

pub enum Note {
	Line { category: String, text: String },
	Failed { command: String, message: String },
	Process(u32),
	Ready,
	Unavailable(String),
	Exited(i64)
}

impl Output {
	pub fn new() -> Self {
		Self {
			seq: AtomicI64::new(1),
			to: To::Client(Mutex::new(io::stdout()))
		}
	}

	pub fn plain(to: impl Fn(Note) + Send + Sync + 'static) -> Self {
		Self {
			seq: AtomicI64::new(1),
			to: To::Plain(Box::new(to))
		}
	}

	fn protocol_message(&self, type_: &str) -> ProtocolMessage {
		ProtocolMessage {
			seq: self.seq.fetch_add(1, Ordering::Relaxed),
			type_: type_.to_owned()
		}
	}

	fn send(&self, message: &impl serde::Serialize) {
		let To::Client(stdout) = &self.to else {
			return;
		};
		let json = serde_json::to_string(message).expect("a DAP message that can't be encoded");
		let mut stdout = stdout.lock().unwrap();
		let _ = write!(stdout, "Content-Length: {}\r\n\r\n{}", json.len(), json);
		let _ = stdout.flush();
	}

	pub fn event<E: Event>(&self, event: E) {
		let body = serde_json::to_value(event).expect("a DAP event that can't be encoded");
		if let To::Plain(to) = &self.to {
			let text = |name: &str| body[name].as_str().unwrap_or_default().to_owned();
			match E::EVENT {
				"output" => to(Note::Line {
					category: text("category"),
					text: text("output")
				}),
				"process" => to(Note::Process(body["systemProcessId"].as_u64().unwrap_or_default() as u32)),
				"exited" => to(Note::Exited(body["exitCode"].as_i64().unwrap_or(-1))),
				_ => {}
			}
			return;
		}
		self.send(&EventMessage {
			protocol_message: self.protocol_message(EventMessage::TYPE),
			event: E::EVENT.to_owned(),
			body: Some(body)
		});
	}

	pub fn response(&self, request_seq: i64, command: &str, result: Result<serde_json::Value, String>) {
		if let To::Plain(to) = &self.to {
			if let Err(message) = result {
				to(Note::Failed {
					command: command.to_owned(),
					message
				});
			}
			return;
		}
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

	pub fn console(&self, text: impl Into<String>) {
		self.output("console", text.into() + "\n");
	}

	pub fn console_at(&self, text: impl Into<String>, file: &std::path::Path, line: Option<u32>) {
		self.event(OutputEvent {
			output: text.into() + "\n",
			category: Some("console".to_owned()),
			source: Some(dap_types::Source {
				name: file.file_name().map(|name| name.to_string_lossy().into_owned()),
				path: Some(file.to_string_lossy().into_owned()),
				..Default::default()
			}),
			line: line.map(i64::from),
			..Default::default()
		});
	}

	pub fn to_client(&self) -> bool {
		matches!(self.to, To::Client(_))
	}

	pub fn ready(&self) {
		if let To::Plain(to) = &self.to {
			to(Note::Ready);
		}
	}

	pub fn unavailable(&self, reason: &str) {
		if let To::Plain(to) = &self.to {
			to(Note::Unavailable(reason.to_owned()));
		}
	}

	pub fn output(&self, category: &str, output: String) {
		self.event(OutputEvent {
			output,
			category: Some(category.to_owned()),
			..Default::default()
		});
	}
}

#[derive(serde::Serialize)]
#[allow(non_snake_case)]
pub struct ProgressStart {
	pub progressId: String,
	pub title: String
}

impl Event for ProgressStart {
	const EVENT: &'static str = "progressStart";
}

#[derive(serde::Serialize)]
#[allow(non_snake_case)]
pub struct ProgressEnd {
	pub progressId: String
}

impl Event for ProgressEnd {
	const EVENT: &'static str = "progressEnd";
}

#[derive(serde::Serialize)]
#[allow(non_snake_case)]
pub struct ProcessEvent {
	pub name: String,
	pub systemProcessId: u32,
	pub isLocalProcess: bool,
	pub startMethod: String
}

impl Event for ProcessEvent {
	const EVENT: &'static str = "process";
}

#[derive(serde::Serialize, Default)]
#[allow(non_snake_case)]
pub struct StoppedEvent {
	pub reason: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub description: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub text: Option<String>,
	pub threadId: i64,
	pub allThreadsStopped: bool,
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub hitBreakpointIds: Vec<i64>
}

impl Event for StoppedEvent {
	const EVENT: &'static str = "stopped";
}

#[derive(serde::Serialize)]
pub struct InvalidatedEvent {
	pub areas: Vec<String>
}

impl Event for InvalidatedEvent {
	const EVENT: &'static str = "invalidated";
}

#[derive(serde::Serialize)]
pub struct BreakpointEvent {
	pub reason: String,
	pub breakpoint: Breakpoint
}

impl Event for BreakpointEvent {
	const EVENT: &'static str = "breakpoint";
}

#[cfg(test)]
mod tests {
	use std::io::Cursor;

	use super::read_message;

	fn framed(body: &str) -> String {
		format!("Content-Length: {}\r\n\r\n{}", body.len(), body)
	}

	#[test]
	fn messages_are_read_one_after_another() {
		let mut input = Cursor::new(framed("{\"seq\":1}") + &framed("{\"seq\":2}"));
		assert_eq!(read_message(&mut input).unwrap().as_deref(), Some("{\"seq\":1}"));
		assert_eq!(read_message(&mut input).unwrap().as_deref(), Some("{\"seq\":2}"));
		assert_eq!(read_message(&mut input).unwrap(), None);
	}

	#[test]
	fn the_length_counts_bytes() {
		let mut input = Cursor::new(framed("{\"text\":\"\u{e9}\"}"));
		assert_eq!(read_message(&mut input).unwrap().as_deref(), Some("{\"text\":\"\u{e9}\"}"));
	}

	#[test]
	fn other_headers_and_other_case_are_fine() {
		let mut input = Cursor::new("Content-Type: application/json\r\ncontent-length: 2\r\n\r\n{}".to_owned());
		assert_eq!(read_message(&mut input).unwrap().as_deref(), Some("{}"));
		let mut input = Cursor::new("Content-Length:2\n\n{}".to_owned());
		assert_eq!(read_message(&mut input).unwrap().as_deref(), Some("{}"));
	}

	#[test]
	fn a_message_without_a_length_is_an_error() {
		let mut input = Cursor::new("Content-Type: application/json\r\n\r\n{}".to_owned());
		assert!(read_message(&mut input).is_err());
		let mut input = Cursor::new("Content-Length: lots\r\n\r\n{}".to_owned());
		assert!(read_message(&mut input).is_err());
	}

	#[test]
	fn a_message_cut_short_is_an_error() {
		let mut input = Cursor::new("Content-Length: 10\r\n\r\n{}".to_owned());
		assert!(read_message(&mut input).is_err());
	}

	#[test]
	fn a_message_that_isnt_text_is_an_error() {
		let mut input = Cursor::new(b"Content-Length: 2\r\n\r\n\xff\xfe".to_vec());
		assert!(read_message(&mut input).is_err());
	}
}
