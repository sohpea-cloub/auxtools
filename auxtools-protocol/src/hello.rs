use serde::{Deserialize, Serialize};

use crate::PROTOCOL_VERSION;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Hello {
	pub protocol: u32,
	pub auxtools: String,
	pub pid: u32
}

impl Hello {
	pub fn new(auxtools: &str) -> Self {
		Self {
			protocol: PROTOCOL_VERSION,
			auxtools: auxtools.to_owned(),
			pid: std::process::id()
		}
	}
}

pub fn world_socket_name(pid: u32) -> String {
	format!("auxtools-debug-{}", pid)
}

pub const LAST_NAMED_PROTOCOL: u32 = 12;

const _: () = assert!(PROTOCOL_VERSION > LAST_NAMED_PROTOCOL);

pub fn old_socket_name(protocol: u32, pid: u32) -> String {
	format!("auxtools-debug-v{}-{}", protocol, pid)
}

#[cfg(test)]
mod tests {
	use std::io::Cursor;

	use serde::Serialize;

	use super::{old_socket_name, world_socket_name, Hello, LAST_NAMED_PROTOCOL};
	use crate::{read, write, PROTOCOL_VERSION};

	#[test]
	fn a_hello_says_this_version_and_process() {
		let hello = Hello::new("4.0.0");
		assert_eq!(
			(hello.protocol, hello.auxtools.as_str(), hello.pid),
			(PROTOCOL_VERSION, "4.0.0", std::process::id())
		);
		let mut wire = vec![];
		write(&mut wire, &hello).unwrap();
		assert_eq!(read::<Hello>(&mut Cursor::new(wire)).unwrap(), hello);
	}

	#[test]
	fn a_hello_with_more_after_its_fields_is_still_read() {
		#[derive(Serialize)]
		struct LaterHello {
			protocol: u32,
			auxtools: String,
			pid: u32,
			byond: (u32, u32)
		}
		let mut wire = vec![];
		write(&mut wire, &LaterHello {
			protocol: PROTOCOL_VERSION + 5,
			auxtools: "5.1.0".to_owned(),
			pid: 1234,
			byond: (516, 1700)
		})
		.unwrap();
		assert_eq!(read::<Hello>(&mut Cursor::new(wire)).unwrap(), Hello {
			protocol: PROTOCOL_VERSION + 5,
			auxtools: "5.1.0".to_owned(),
			pid: 1234
		});
	}

	#[test]
	fn the_socket_is_named_for_the_process_alone() {
		assert_eq!(world_socket_name(1234), "auxtools-debug-1234");
		assert_eq!(old_socket_name(LAST_NAMED_PROTOCOL, 1234), "auxtools-debug-v12-1234");
	}
}
