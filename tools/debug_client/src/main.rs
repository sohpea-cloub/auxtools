use std::{
	collections::{HashMap, VecDeque},
	path::{Path, PathBuf},
	time::Duration
};

use auxtools_cli::{
	byond::{self, Server},
	client::{when_listening, Client, Heard},
	game::{Game, Launch}
};
use auxtools_protocol::{ContinueKind, EvalResponse, InstructionRef, ProcRef, Request, RequestId, Response, WORLD_SHUTTING_DOWN};

const SEE_THE_WORLD_RESUME: Duration = Duration::from_secs(3);

const USAGE: &str = "\
usage: debug_client WORLD.dmb --proc PATH --line N [options] [WATCH...]

Starts the world in DreamDaemon with the debug library loaded into it, as the
launcher does, and ends it when done.

  --byond DIR         where BYOND is (default: where it's installed)
  --library PATH      the debug library (default: the one next to debug_client)
  --arg ARG           an argument for DreamDaemon after the .dmb, in place of
                      -trusted (repeat it for more)
  --proc PATH         proc to break in, as the debugger names it, e.g. /obj/item/Initialize
  --override N        override id of that proc (default 0)
  --line N            source line to break on
  --condition EXPR    only stop when EXPR is true
  --skip-inspect      go straight to the watches, without the requests VS Code sends on a stop
  --wait-file PATH    before the watches, wait until PATH exists (time to attach a debugger)

A WATCH is evaluated in the top frame like a VS Code watch; prefix it with
`repl:` to evaluate it like the debug console does.";

struct Options {
	dmb: PathBuf,
	byond: Option<PathBuf>,
	library: Option<PathBuf>,
	args: Vec<String>,
	proc: ProcRef,
	line: u32,
	condition: Option<String>,
	inspect: bool,
	wait_file: Option<String>,
	watches: Vec<String>
}

fn parse_args() -> Result<Options, String> {
	let mut dmb = None;
	let mut byond = None;
	let mut library = None;
	let mut game_args = vec![];
	let mut proc = None;
	let mut override_id = 0;
	let mut line = None;
	let mut condition = None;
	let mut inspect = true;
	let mut wait_file = None;
	let mut watches = vec![];

	let mut args = std::env::args().skip(1);
	while let Some(arg) = args.next() {
		let mut value = |name: &str| args.next().ok_or_else(|| format!("{} needs a value", name));
		match arg.as_str() {
			"--byond" => byond = Some(PathBuf::from(value("--byond")?)),
			"--library" => library = Some(PathBuf::from(value("--library")?)),
			"--arg" => game_args.push(value("--arg")?),
			"--proc" => proc = Some(value("--proc")?),
			"--override" => override_id = value("--override")?.parse().map_err(|e| format!("--override: {}", e))?,
			"--line" => line = Some(value("--line")?.parse().map_err(|e| format!("--line: {}", e))?),
			"--condition" => condition = Some(value("--condition")?),
			"--skip-inspect" => inspect = false,
			"--wait-file" => wait_file = Some(value("--wait-file")?),
			"-h" | "--help" => return Err(String::new()),
			_ if arg.starts_with("--") => return Err(format!("unknown option {}", arg)),
			_ if dmb.is_none() => dmb = Some(PathBuf::from(arg)),
			_ => watches.push(arg)
		}
	}

	if game_args.is_empty() {
		game_args.push("-trusted".to_owned());
	}

	Ok(Options {
		dmb: dmb.ok_or("the .dmb is required")?,
		byond,
		library,
		args: game_args,
		proc: ProcRef {
			path: proc.ok_or("--proc is required")?,
			override_id
		},
		line: line.ok_or("--line is required")?,
		condition,
		inspect,
		wait_file,
		watches
	})
}

struct Connection {
	client: Client,
	early_events: VecDeque<Response>
}

impl Connection {
	fn send(&mut self, request: &Request) {
		self.client.tell(request, None).unwrap();
		println!(">> {:?}", request);
	}

	fn recv_any(&mut self) -> Option<(Option<RequestId>, Response)> {
		match self.client.hear(None) {
			Ok(Some(Heard::Answer { to, response, .. })) => Some((Some(to), response)),
			Ok(Some(Heard::Event(event))) => Some((None, event)),
			Ok(None) | Err(_) => {
				println!("!! connection lost");
				None
			}
		}
	}

	fn recv_skipping_notifications(&mut self) -> Option<(Option<RequestId>, Response)> {
		loop {
			match self.recv_any()? {
				(_, Response::Notification { message }) => println!("<< notification: {}", message.trim_end()),
				(_, Response::ShuttingDown) => println!("<< notification: {}", WORLD_SHUTTING_DOWN),
				(to, response) => {
					let text = format!("{:?}", response);
					if text.len() > 2000 {
						println!("<< {}... ({} chars)", &text[..2000], text.len());
					} else {
						println!("<< {}", text);
					}
					return Some((to, response));
				}
			}
		}
	}

	fn recv_event(&mut self) -> Option<Response> {
		if let Some(event) = self.early_events.pop_front() {
			return Some(event);
		}
		loop {
			if let (None, event) = self.recv_skipping_notifications()? {
				return Some(event);
			}
		}
	}

	fn request(&mut self, request: Request) -> Option<Response> {
		let id = self.client.ask(&request, None).unwrap();
		println!(">> {:?}", request);
		loop {
			match self.recv_skipping_notifications()? {
				(Some(to), answer) if Some(to) == id => return Some(answer),
				(Some(_), _) => {}
				(None, event) => self.early_events.push_back(event)
			}
		}
	}
}

fn on_stop(conn: &mut Connection, options: &Options) -> Option<()> {
	if options.inspect {
		conn.request(Request::Stacks { sleeping: None })?;
		conn.request(Request::StackFrames {
			stack_id: 0,
			start_frame: Some(0),
			count: Some(20)
		})?;
		if let Response::Scopes { arguments, locals, .. } = conn.request(Request::Scopes { frame_id: 0 })? {
			for vars in [arguments, locals].into_iter().flatten() {
				conn.request(Request::Variables {
					vars,
					start: None,
					count: None
				})?;
			}
		}
	}

	if let Some(path) = &options.wait_file {
		println!("== ready for watches, waiting for {}", path);
		while !Path::new(path).exists() {
			std::thread::sleep(Duration::from_millis(200));
		}
	}

	for watch in &options.watches {
		let command = watch.strip_prefix("repl:").unwrap_or(watch);
		let response = conn.request(Request::Eval {
			frame_id: Some(0),
			command: command.to_owned(),
			ending: false
		})?;
		if let Response::Eval(Ok(EvalResponse { variables: Some(vars), .. })) = response {
			conn.request(Request::Variables {
				vars,
				start: None,
				count: None
			})?;
		}
	}

	Some(())
}

fn main() {
	let options = match parse_args() {
		Ok(options) => options,
		Err(message) => {
			if !message.is_empty() {
				eprintln!("{}\n", message);
			}
			eprintln!("{}", USAGE);
			std::process::exit(2);
		}
	};

	let game = match start(&options) {
		Ok(game) => game,
		Err(e) => {
			eprintln!("{}", e);
			std::process::exit(1);
		}
	};
	let Ok(client) = when_listening(game.pid(), None, Client::connect) else {
		std::process::exit(1);
	};
	let mut conn = Connection {
		client,
		early_events: VecDeque::new()
	};
	println!("== connected to DreamDaemon");

	let offset = match conn.request(Request::Offset {
		proc: options.proc.clone(),
		line: options.line
	}) {
		Some(Response::Offset { offset: Some(offset) }) => offset,
		other => {
			eprintln!("no instruction on line {} of {}: {:?}", options.line, options.proc.path, other);
			std::process::exit(1);
		}
	};
	let instruction = InstructionRef {
		proc: options.proc.clone(),
		offset
	};
	conn.request(Request::BreakpointSet {
		instruction,
		condition: options.condition.clone(),
		hit: None,
		log: None,
		exact: false,
		id: 1
	});
	conn.send(&Request::CatchRuntimes {
		uncaught: None,
		caught: None
	});
	conn.request(Request::Configured);

	loop {
		let Some(response) = conn.recv_event() else {
			std::process::exit(1);
		};
		if let Response::BreakpointHit { .. } = response {
			break;
		}
	}

	if on_stop(&mut conn, &options).is_none() {
		std::process::exit(1);
	}

	println!("== watches done, continuing");
	conn.request(Request::BreakpointUnset { id: 1 });
	conn.request(Request::Continue {
		kind: ContinueKind::Continue
	});
	std::thread::sleep(SEE_THE_WORLD_RESUME);
	conn.send(&Request::Disconnect);
	game.kill();
}

fn start(options: &Options) -> Result<Game, String> {
	let bin = byond::bin_dir(options.byond.as_deref())?;
	let library = match &options.library {
		Some(library) => library.clone(),
		None => {
			let name = if cfg!(windows) { "debug_server.dll" } else { "libdebug_server.so" };
			std::env::current_exe().map_err(|e| e.to_string())?.with_file_name(name)
		}
	};
	let wait_for_debugger = true;
	Game::start(
		Launch {
			program: bin.join(Server::DreamDaemon.program()),
			dmb: &options.dmb,
			args: &options.args,
			env: &HashMap::new(),
			debug: Some((library, wait_for_debugger))
		},
		|_, line| print!("  | {}", line),
		|code| println!("== the world exited ({:?})", code)
	)
}
