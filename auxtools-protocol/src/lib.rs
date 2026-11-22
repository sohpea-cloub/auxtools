mod framing;
mod messages;

pub use framing::{frame, read, write, MAX_MESSAGE_LEN};
pub use messages::*;

pub const PROTOCOL_VERSION: u32 = 12;

pub fn world_socket_name(pid: u32) -> String {
	world_socket_name_for(PROTOCOL_VERSION, pid)
}

pub fn world_socket_name_for(protocol: u32, pid: u32) -> String {
	format!("auxtools-debug-v{}-{}", protocol, pid)
}
