//! The debug adapter for auxtools' debugger. A client (VS Code) talks to it in
//! the Debug Adapter Protocol on stdin and stdout. It starts a world in
//! DreamSeeker or DreamDaemon with the debug library loaded into it, which
//! needs nothing from the world's code, or attaches to one already running it,
//! and passes on what the world prints. It hot reloads the world's code as
//! files are saved, with or without debugging. It also takes one command to a
//! running world from the command line (see `cli`).

mod adapter;
mod cli;
mod code;
mod dap;
mod debuggee;
mod sources;

const USAGE: &str = "\
auxtools: the debug adapter for auxtools' debugger

auxtools                                talk DAP on stdin and stdout
auxtools --list                         list the running worlds that can be
                                        attached to, as JSON
auxtools --build DME [BYOND]            build DME with DreamMaker, from the BYOND
                                        installed in BYOND or the usual places
auxtools --version
";

fn main() {
	let args: Vec<String> = std::env::args().skip(1).collect();
	if args.first().is_some_and(|a| cli::is_command(a)) {
		cli::run(&args);
	}
	match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
		[] => adapter::run(),
		["--version"] => println!("auxtools {}", env!("CARGO_PKG_VERSION")),
		["--list"] => println!(
			"{}",
			serde_json::to_string_pretty(&auxtools_cli::local::worlds()).expect("worlds that can't be written as JSON")
		),
		["--build", dme, rest @ ..] if rest.len() <= 1 => {
			let built = auxtools_cli::byond::bin_dir(rest.first().map(std::path::Path::new))
				.and_then(|bin| auxtools_cli::byond::build_with(&bin, std::path::Path::new(dme), &[], |line| println!("{}", line)));
			if let Err(e) = built {
				eprintln!("{}", e);
				std::process::exit(1);
			}
		}
		_ => {
			eprintln!("{}{}", USAGE, cli::USAGE);
			std::process::exit(2);
		}
	}
}
