use std::io::{self, Read, Write};

use serde::{de::DeserializeOwned, Serialize};

pub const MAX_MESSAGE_LEN: u32 = 256 * 1024 * 1024;

pub fn frame(message: &impl Serialize) -> io::Result<Vec<u8>> {
	let data = bincode::serialize(message).map_err(io::Error::other)?;
	let len = framed_length(data.len())?;
	let mut bytes = Vec::with_capacity(data.len() + 4);
	bytes.extend_from_slice(&len.to_le_bytes());
	bytes.extend_from_slice(&data);
	Ok(bytes)
}

pub fn write(stream: &mut impl Write, message: &impl Serialize) -> io::Result<()> {
	stream.write_all(&frame(message)?)?;
	stream.flush()
}

pub fn read<T: DeserializeOwned>(stream: &mut impl Read) -> io::Result<T> {
	let mut len = [0u8; 4];
	stream.read_exact(&mut len)?;
	let len = u32::from_le_bytes(len);
	if len > MAX_MESSAGE_LEN {
		return Err(too_long(len as usize));
	}
	let mut data = vec![];
	stream.take(u64::from(len)).read_to_end(&mut data)?;
	if data.len() < len as usize {
		return Err(io::ErrorKind::UnexpectedEof.into());
	}
	bincode::deserialize(&data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn framed_length(len: usize) -> io::Result<u32> {
	u32::try_from(len).ok().filter(|len| *len <= MAX_MESSAGE_LEN).ok_or_else(|| too_long(len))
}

fn too_long(len: usize) -> io::Error {
	io::Error::new(io::ErrorKind::InvalidData, format!("message length {} exceeds the protocol limit", len))
}

#[cfg(test)]
mod tests {
	use std::io::{self, Cursor, Read};

	use super::{frame, framed_length, read, write, MAX_MESSAGE_LEN};
	use crate::{Request, Response};

	#[test]
	fn messages_come_back_as_they_went() {
		let mut wire = vec![];
		let request = Request::Locate {
			file: "code/mob.dm".to_owned(),
			line: 7
		};
		write(&mut wire, &request).unwrap();
		write(&mut wire, &Response::Notification { message: "one".to_owned() }).unwrap();
		write(&mut wire, &Response::Ack).unwrap();

		let mut wire = Cursor::new(wire);
		match read::<Request>(&mut wire).unwrap() {
			Request::Locate { file, line } => assert_eq!((file.as_str(), line), ("code/mob.dm", 7)),
			other => panic!("read {:?}", other)
		}
		assert!(matches!(read::<Response>(&mut wire).unwrap(), Response::Notification { message } if message == "one"));
		assert!(matches!(read::<Response>(&mut wire).unwrap(), Response::Ack));
		assert_eq!(read::<Response>(&mut wire).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
	}

	#[test]
	fn a_frame_is_what_write_sends() {
		let mut wire = vec![];
		write(&mut wire, &Response::Resumed).unwrap();
		assert_eq!(frame(&Response::Resumed).unwrap(), wire);
	}

	struct EmptyReader {
		bytes_asked: usize
	}

	impl Read for EmptyReader {
		fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
			self.bytes_asked += buf.len();
			Ok(0)
		}
	}

	#[test]
	fn a_length_past_the_limit_is_refused_before_anything_is_read_for_it() {
		let mut wire = Cursor::new([0xff, 0xff, 0xff, 0xff, 1, 2, 3]);
		let error = read::<Request>(&mut wire).unwrap_err();
		assert_eq!(error.kind(), io::ErrorKind::InvalidData);
		assert!(error.to_string().contains("exceeds the protocol limit"), "{}", error);
		assert_eq!(wire.position(), 4);

		let mut wire = Cursor::new((MAX_MESSAGE_LEN + 1).to_le_bytes());
		assert_eq!(read::<Request>(&mut wire).unwrap_err().kind(), io::ErrorKind::InvalidData);
	}

	#[test]
	fn a_length_nothing_follows_allocates_nothing() {
		let mut wire = Cursor::new(MAX_MESSAGE_LEN.to_le_bytes()).chain(EmptyReader { bytes_asked: 0 });
		assert_eq!(read::<Request>(&mut wire).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
		let (_, empty) = wire.into_inner();
		assert!(empty.bytes_asked < 1024 * 1024, "asked for {} bytes at once", empty.bytes_asked);
	}

	#[test]
	fn bytes_that_are_no_message_are_an_error() {
		let mut wire = Cursor::new([3, 0, 0, 0, 0xfe, 0xfe, 0xfe]);
		assert_eq!(read::<Request>(&mut wire).unwrap_err().kind(), io::ErrorKind::InvalidData);
		let mut whole = frame(&Response::Notification { message: "cut".to_owned() }).unwrap();
		whole.truncate(whole.len() - 1);
		assert_eq!(
			read::<Response>(&mut Cursor::new(whole)).unwrap_err().kind(),
			io::ErrorKind::UnexpectedEof
		);
	}

	#[test]
	fn a_message_past_the_limit_is_not_framed() {
		assert_eq!(framed_length(MAX_MESSAGE_LEN as usize).unwrap(), MAX_MESSAGE_LEN);
		assert_eq!(
			framed_length(MAX_MESSAGE_LEN as usize + 1).unwrap_err().kind(),
			io::ErrorKind::InvalidData
		);
	}
}
