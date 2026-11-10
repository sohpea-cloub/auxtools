//! Starting a BYOND world with auxtools' debug library loaded into it, which is
//! the only way to debug one, and reaching the worlds that run it. The launcher
//! (`auxtools`, a debug adapter) is built on this, and so are the
//! tests and `debug_client`, which talk to the debug server directly.

pub mod byond;
pub mod game;
#[cfg(windows)]
mod inject;
pub mod local;

// The debug server's message types aren't exported from its cdylib, so they
// are pulled in as source, like tests/debug_test does
#[allow(dead_code)]
#[path = "../../debug_server/src/server_types.rs"]
pub mod server_types;
