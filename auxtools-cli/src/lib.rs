pub mod byond;
pub mod game;
#[cfg(windows)]
mod inject;
pub mod local;

#[allow(dead_code)]
#[path = "../../debug_server/src/server_types.rs"]
pub mod server_types;

#[path = "../../debug_server/src/dmb_tables.rs"]
pub mod dmb_tables;

#[allow(dead_code)]
#[path = "../../debug_server/src/macros.rs"]
pub mod macros;
