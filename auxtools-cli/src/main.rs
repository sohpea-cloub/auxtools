//! auxtools' launcher. `auxtools dap` is the debug adapter: a client (VS Code)
//! talks to it in the Debug Adapter Protocol on stdin and stdout. It starts a
//! world in DreamSeeker or DreamDaemon with the debug library loaded into it,
//! which needs nothing from the world's code, or attaches to one already
//! running it, and passes on what the world prints. It hot reloads the world's
//! code as files are saved, with or without debugging. Every other command is
//! one thing done to a running world from the command line (see `cli`).

mod adapter;
mod cli;
mod code;
mod dap;
mod debuggee;
mod lines;
mod stddef;

fn main() {
	let args: Vec<String> = std::env::args().skip(1).collect();
	cli::run(&args)
}
