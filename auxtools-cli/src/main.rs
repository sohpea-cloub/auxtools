mod adapter;
mod cli;
mod code;
mod dap;
mod debuggee;
mod lines;
mod profile;
mod resources;
mod script;
mod skin;
mod stddef;

fn main() {
	let args: Vec<String> = std::env::args().skip(1).collect();
	cli::run(&args)
}
