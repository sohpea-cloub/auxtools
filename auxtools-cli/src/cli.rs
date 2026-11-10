//! One command to a running world, from the command line: connect, do it, print
//! what came of it and leave. For scripts and agents, which can run `wait` in
//! the background to hear when the world pauses at a breakpoint or a runtime.
//!
//! A command that leaves the world paused (`pause`, `next`, `wait` and the
//! like) asks it to stay paused after the connection goes, until a `continue`.

mod hold;
mod log;

use std::{
	io,
	sync::mpsc::{self, Receiver, RecvTimeoutError},
	time::{Duration, Instant}
};

use auxtools_cli::{
	local::{self, Shared},
	server_types::*
};

mod launch;
mod stop;

/// What went wrong, and the exit code it gives
pub struct Failure(pub i32, pub String);

impl From<io::Error> for Failure {
	fn from(e: io::Error) -> Self {
		Self(1, e.to_string())
	}
}

impl From<String> for Failure {
	fn from(message: String) -> Self {
		Self(1, message)
	}
}

impl From<&str> for Failure {
	fn from(message: &str) -> Self {
		Self(1, message.to_owned())
	}
}

pub type Result<T> = std::result::Result<T, Failure>;

const HUNG_UP: i32 = 3;
const TIMED_OUT: i32 = 2;

/// A command: `auxtools NAME ...`
pub struct Command {
	pub name: &'static str,
	/// Its arguments, as the help shows them after its name
	pub usage: &'static str,
	/// What it does, in a line
	pub about: &'static str,
	/// The rest, for `auxtools help NAME`: its options, a line each
	pub details: &'static str,
	pub run: Run
}

pub enum Run {
	/// Needs no world, or finds its own
	Alone(fn(&[String]) -> Result<()>),
	/// One command to a running world: connect, do it, leave
	World(fn(&mut World, &[String]) -> Result<()>)
}

/// The commands, in the groups the help shows them in. A new one is a
/// function, in a file of its own under `cli/` if it's more than a few lines,
/// and a line here.
pub const COMMANDS: &[(&str, &[Command])] = &[
	("Worlds", &[
		Command {
			name: "launch",
			usage: "DMB [--detach] [--port N] [OPTIONS] [-- ARGS...]",
			about: "Starts a world with the debug library in it, and hot reloads its code",
			details: "In the foreground it prints what the world prints and each hot reload, and stays
until the world ends: exit code 0 if it exited with 0, 4 if it exited with another or
was killed. Ctrl-C (or SIGTERM, or the terminal closing) has the world end itself, as
`stop` does; a second ends it there and then. It doesn't build the world (DreamMaker
does): after a rebuild, `reboot` loads the new .dmb. It doesn't debug it either: other
commands, and editors, attach to it as to any world, and it never pauses for them.
  --detach         Run in the background, and return once the world is up: prints its
                   process id, and where the launcher's output goes
  --json           With --detach: print {\"pid\", \"launcher\", \"log\"}
  --log FILE       Write what would be printed to FILE. With --detach, NAME.auxtools.log
                   next to NAME.dmb unless given
  --dme DME        The world's code, if it isn't NAME.dme next to NAME.dmb
  -D NAME[=VALUE]  A define the .dmb was built with, so its code is read as it was
                   built (for hot reloading). Can be given more than once.
  --no-hot-reload  Don't watch the code
  --server PROGRAM dd, dreamdaemon or dreamseeker: on Windows, dreamseeker unless given;
                   on Linux there's only DreamDaemon
  --port N         The port the world listens on (DreamDaemon and dd)
  --byond PATH     Where BYOND is, if not where it's installed (or BYOND_SYSTEM)
  -- ARGS...       The program's arguments after the .dmb (and the port), in place of
                   -trusted (and -close for Windows' DreamDaemon)",
			run: Run::Alone(launch::launch)
		},
		Command {
			name: "stop",
			usage: "[--timeout SECONDS] [--kill]",
			about: "Has the world end itself, through its world/Del(), and waits for it to be gone",
			details: "A world started by `launch` takes its launcher with it. One whose world/Del() doesn't
end it has had its debugger shut down all the same (the debug library ends with
world/Del()), so other commands can't reach it: `--kill` is the way to end it.
  --timeout SECONDS  How long to wait for the process to be gone (10 unless given): if
                     it isn't, the exit code is 1
  --kill             End the process if it's still there then",
			run: Run::World(stop::stop)
		},
		Command {
			name: "reboot",
			usage: "[--timeout SECONDS]",
			about: "Reboots the world, which loads its .dmb again, and waits for it to answer",
			details: "world.Reboot(): how a world rebuilt by its own tools is picked up. Every client is
let go (a `wait` exits with 3); a launcher connects again, and hot reloads what changed
since the .dmb was built.
  --timeout SECONDS  How long to wait for it (60 unless given): exit code 2 if it's longer",
			run: Run::World(stop::reboot)
		},
		Command {
			name: "list",
			usage: "",
			about: "The worlds running the debug library, as JSON",
			details: "Each has its process id (`pid`), its program, the .dmb it runs, and the version of
the protocol its debug library speaks (`protocol`). One whose `compatible` is false
was started by another version of auxtools: this one can't attach to it.",
			run: Run::Alone(list)
		},
		Command {
			name: "status",
			usage: "",
			about: "Whether the world is running or paused, and if paused, why and where",
			details: "The first line is `running`, or `paused` and why; a paused world's stack follows. Then
how long the world has been up (since it last loaded, and its world.time), and what came of
its last hot reload.",
			run: Run::World(|world, args| {
				Flags::parse(args, &[], &[])?.nothing_else("status")?;
				world.status()?;
				world.uptime()
			})
		},
		Command {
			name: "log",
			usage: "[--follow] [--runtimes] [--reloads] [--output] [--world] [OPTIONS]",
			about: "What happened in the world: runtime errors, hot reloads, reboots, its output",
			details: "The world keeps a record of the last few thousand things that happened in it, numbered
from 1 (the numbers go on through reboots). This prints it, oldest first: every kind, or
those named. Runtime errors are there with their stacks whether or not anything paused for
them, so `log --runtimes --follow` watches them without pausing the world.
  --follow     Keep printing new entries as they come, until interrupted (or the world ends)
  --runtimes   Runtime errors, and thrown exceptions: caught or not, each with its stack
  --reloads    Hot reloads: applied or not, and why not (compile errors among them)
  --output     What the world printed (world.log, runtime errors), passed on by the launcher
               that started it and reads it: DreamDaemon's and dd's. DreamSeeker gives a
               launcher nothing to read, so a world it hosts has none.
  --world      The world loading and rebooting, and what the debugger did by itself
  --since N    Only the entries after number N
  --last N     Only the last N entries (of those asked for)
  --json       An entry a line, as a JSON object: number, time (milliseconds since 1970,
               UTC), world_time (deciseconds), kind, and what that kind has",
			run: Run::World(log::log)
		},
	]),
	("Looking at a paused world", &[
		Command {
			name: "stack",
			usage: "[--all]",
			about: "The procs the world is paused in, innermost first",
			details: "  --all    The sleeping procs' stacks too, each with the id it keeps",
			run: Run::World(|world, args| {
				let flags = Flags::parse(args, &[], &["--all"])?;
				flags.nothing_else("stack")?;
				world.stack(flags.has("--all"))
			})
		},
		Command {
			name: "vars",
			usage: "[--frame N] [--globals]",
			about: "A frame's arguments and local vars",
			details: "  --frame N    The Nth frame of the stack, from 0, the innermost (the default)
  --globals    The world's global vars too",
			run: Run::World(|world, args| {
				let flags = Flags::parse(args, &["--frame"], &["--globals"])?;
				flags.nothing_else("vars")?;
				world.vars(flags.number("--frame")?.unwrap_or(0), flags.has("--globals"))
			})
		},
		Command {
			name: "eval",
			usage: "EXPRESSION [--frame N]",
			about: "The value of a DM expression",
			details: "In a paused world, the expression is in a frame of the stack: it can name that
proc's vars, and `src`. In a running world it can name globals and call procs.
  --frame N    The Nth frame of the stack, from 0, the innermost (the default)",
			run: Run::World(|world, args| {
				let flags = Flags::parse(args, &["--frame"], &[])?;
				if flags.rest.is_empty() {
					return Err("eval what?".into());
				}
				world.eval(&flags.rest.join(" "), flags.number("--frame")?)
			})
		},
	]),
	("Running it", &[
		Command {
			name: "pause",
			usage: "[--hold-max SECONDS]",
			about: "Pauses the world where it is",
			details: "The world stays paused after this command has gone, until `continue`.
  --hold-max SECONDS  Let the world carry on by itself this long after the command has
                      gone (once nobody debugging is connected to it). The default is
                      no limit, or AUXTOOLS_HOLD_MAX if it's set. Its `log` says so.",
			run: Run::World(|world, args| {
				Flags::parse(args, &[], &[])?.nothing_else("pause")?;
				world.pause()
			})
		},
		Command {
			name: "continue",
			usage: "",
			about: "Lets a paused world carry on",
			details: "",
			run: Run::World(|world, args| {
				Flags::parse(args, &[], &[])?.nothing_else("continue")?;
				world.resume(ContinueKind::Continue)
			})
		},
		Command {
			name: "next",
			usage: "[--hold-max SECONDS]",
			about: "Runs to the next line, over any procs this one calls",
			details: "The world stays paused afterwards, until `continue`.
  --hold-max SECONDS  As for `pause`",
			run: Run::World(|world, args| {
				Flags::parse(args, &[], &[])?.nothing_else("next")?;
				world.resume(ContinueKind::StepOver { stack_id: 0 })
			})
		},
		Command {
			name: "step",
			usage: "[--hold-max SECONDS]",
			about: "Runs to the next line, going into a proc this one calls",
			details: "The world stays paused afterwards, until `continue`.
  --hold-max SECONDS  As for `pause`",
			run: Run::World(|world, args| {
				Flags::parse(args, &[], &[])?.nothing_else("step")?;
				world.resume(ContinueKind::StepInto { stack_id: 0 })
			})
		},
		Command {
			name: "out",
			usage: "[--hold-max SECONDS]",
			about: "Runs until this proc returns to the one that called it",
			details: "The world stays paused afterwards, until `continue`.
  --hold-max SECONDS  As for `pause`",
			run: Run::World(|world, args| {
				Flags::parse(args, &[], &[])?.nothing_else("out")?;
				world.resume(ContinueKind::StepOut { stack_id: 0 })
			})
		},
		Command {
			name: "wait",
			usage: "[--break PLACE]... [--change EXPRESSION]... [--runtimes] [OPTIONS]",
			about: "Waits for the world to pause, and says why and where",
			details: "With nothing to pause it for, it waits for whatever pauses it: another client's
breakpoint, a `pause`. The world stays paused afterwards, until `continue`.
  --break PLACE        Pause at FILE:LINE, or when /proc/path is called (/proc/path#N for
                       its Nth override). Can be given more than once.
  --change EXPRESSION  Pause when what EXPRESSION names gets another value: a var of an
                       object, an entry of a list, a global var, a proc's own var. Given
                       while the world is paused, EXPRESSION is of a frame there, and the
                       world carries on until it changes.
  --frame N            The frame --change's EXPRESSION is of (0, the innermost)
  --if CONDITION       Only pause when CONDITION, a DM expression, is true there. For
                       --change, `value` is the new value and `old` the one before.
  --runtimes           Pause on a runtime error that nothing catches
  --caught             Pause on runtime errors inside a `try` as well
  --log MESSAGE        Don't pause at the breakpoints: print MESSAGE at each hit, with the
                       values of the {expressions} in it
  --timeout SECONDS    Give up after this long (exit code 2)
  --no-hold            Let the world carry on once this command has gone
  --hold-max SECONDS   As for `pause`: let it carry on by itself this long after this
                       command has gone (once nobody debugging is connected to it)

wait --reload [--since N] [--timeout SECONDS] [--json]
  Waits for the next hot reload to be tried (or the first after entry N of `log`), by the
  launcher that reloads the world, and prints what came of it as `log --reloads` does.
  Exit code 0 if it was applied, 1 if not (compile errors, changes that need a rebuild).
  Start it before saving the change, or give --since.
wait --reboot [--since N] [--timeout SECONDS] [--json]
  Waits for the world to reboot (or for the first load after entry N), and for the new
  world to answer. Exit code 3 if the world ends instead.",
			run: Run::World(wait)
		},
	]),
	("For editors", &[
		Command {
			name: "dap",
			usage: "",
			about: "The debug adapter: talks the Debug Adapter Protocol on stdin and stdout",
			details: "What an editor's extension starts. It launches worlds, attaches to them, and hot
reloads their code as files are saved.",
			run: Run::Alone(|_| {
				crate::adapter::run();
				Ok(())
			})
		},
	]),
	("About auxtools", &[
		Command {
			name: "help",
			usage: "[COMMAND]",
			about: "This, or a command's options",
			details: "",
			run: Run::Alone(help)
		},
		Command {
			name: "version",
			usage: "",
			about: "The version of auxtools, and of the protocol it speaks to worlds",
			details: "",
			run: Run::Alone(|_| {
				println!("{}", version());
				Ok(())
			})
		},
	])
];

fn version() -> String {
	format!("auxtools {} (protocol {})", env!("CARGO_PKG_VERSION"), PROTOCOL_VERSION)
}

fn command(name: &str) -> Option<&'static Command> {
	COMMANDS.iter().flat_map(|(_, commands)| commands.iter()).find(|command| command.name == name)
}

/// Every command, a line each
pub fn overview() -> String {
	let mut text = String::from(
		"auxtools: debug BYOND worlds, from the command line and from editors

Usage: auxtools [--pid PID] COMMAND [OPTIONS]

A command is for the one world that's running the debug library. With more than one,
--pid says which (`auxtools list` shows them).
"
	);
	let width = COMMANDS.iter().flat_map(|(_, commands)| commands.iter()).map(|command| command.name.len()).max().unwrap_or(0);
	for (group, commands) in COMMANDS {
		text += &format!("\n{}:\n", group);
		for command in *commands {
			text += &format!("  {:width$}  {}\n", command.name, command.about);
		}
	}
	text += "
`auxtools help COMMAND` shows a command's options.

A command that leaves the world paused (pause, wait, next, step, out) can be given
--hold-max SECONDS, or AUXTOOLS_HOLD_MAX in its environment: a pause nobody debugging comes
back to in that long ends by itself, so a client that dies doesn't leave every player frozen.

Exit codes: 0 done (for `wait`, the world paused), 1 failed, 2 timed out (`wait`,
`reboot`), 3 the world went away (it ended or rebooted), 4 the world `launch` started
ended with an exit code other than 0, or was killed.
";
	text
}

/// One command, with its options
fn describe(command: &Command) -> String {
	let mut text = format!("Usage: auxtools [--pid PID] {}", command.name);
	if !command.usage.is_empty() {
		text += &format!(" {}", command.usage);
	}
	text += &format!("\n\n{}.\n", command.about);
	if !command.details.is_empty() {
		text += &format!("\n{}\n", command.details);
	}
	text
}

fn help(args: &[String]) -> Result<()> {
	match args {
		[] => print!("{}", overview()),
		[name] => match command(name) {
			Some(command) => print!("{}", describe(command)),
			None => return Err(unknown(name))
		},
		_ => return Err("help takes one command's name".into())
	}
	Ok(())
}

fn unknown(name: &str) -> Failure {
	Failure(1, format!("there's no command {}: `auxtools help` lists them", name))
}

fn list(args: &[String]) -> Result<()> {
	Flags::parse(args, &[], &[])?.nothing_else("list")?;
	println!(
		"{}",
		serde_json::to_string_pretty(&local::worlds()).map_err(|e| e.to_string())?
	);
	Ok(())
}

/// Runs `args` (everything after the program's name), and exits with its code
pub fn run(args: &[String]) -> ! {
	// Its output piped into something that stops reading (`| head`) ends it
	// quietly, as it does other programs, rather than with a panic
	#[cfg(unix)]
	unsafe {
		libc::signal(libc::SIGPIPE, libc::SIG_DFL);
	}
	let code = match parse(args).and_then(|(pid, command)| execute(pid, &command)) {
		Ok(()) => 0,
		Err(Failure(code, message)) => {
			eprintln!("{}", message);
			code
		}
	};
	std::process::exit(code)
}

fn parse(args: &[String]) -> Result<(Option<u32>, Vec<String>)> {
	match args {
		[flag, pid, rest @ ..] if flag == "--pid" => {
			let pid = pid.parse().map_err(|_| format!("{} isn't a process id", pid))?;
			Ok((Some(pid), rest.to_vec()))
		}
		_ => Ok((None, args.to_vec()))
	}
}

/// Its flags: `--name value` pairs and bare `--name`s, and what's left
struct Flags {
	given: Vec<(String, Option<String>)>,
	rest: Vec<String>
}

impl Flags {
	fn parse(args: &[String], with_values: &[&str], bare: &[&str]) -> Result<Self> {
		let mut flags = Self { given: vec![], rest: vec![] };
		let mut args = args.iter();
		while let Some(arg) = args.next() {
			if with_values.contains(&arg.as_str()) {
				let value = args.next().ok_or_else(|| format!("{} needs a value", arg))?;
				flags.given.push((arg.clone(), Some(value.clone())));
			} else if bare.contains(&arg.as_str()) {
				flags.given.push((arg.clone(), None));
			} else if arg.starts_with("--") {
				return Err(format!("there's no {} here: `--help` after the command shows its options", arg).into());
			} else {
				flags.rest.push(arg.clone());
			}
		}
		Ok(flags)
	}

	/// Refuses words the command has no use for
	fn nothing_else(&self, command: &str) -> Result<()> {
		if self.rest.is_empty() {
			Ok(())
		} else {
			Err(format!("{} doesn't take {}", command, self.rest.join(" ")).into())
		}
	}

	fn has(&self, name: &str) -> bool {
		self.given.iter().any(|(n, _)| n == name)
	}

	fn values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> {
		self.given.iter().filter(move |(n, _)| n == name).filter_map(|(_, v)| v.as_deref())
	}

	fn value(&self, name: &str) -> Option<&str> {
		self.given.iter().rev().find(|(n, _)| n == name).and_then(|(_, v)| v.as_deref())
	}

	fn number<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>> {
		self.value(name)
			.map(|v| v.parse().map_err(|_| Failure::from(format!("{} {} isn't a number", name, v))))
			.transpose()
	}
}

fn execute(pid: Option<u32>, words: &[String]) -> Result<()> {
	let (name, args) = match words.split_first() {
		// Asked how it's used, in any of the usual ways
		None => return help(&[]),
		Some((name, rest)) if matches!(name.as_str(), "--help" | "-h") => return help(rest),
		Some((name, _)) if matches!(name.as_str(), "--version" | "-V") => ("version", &[][..]),
		Some((name, args)) => (name.as_str(), args)
	};
	let command = command(name).ok_or_else(|| unknown(name))?;
	if args.iter().any(|arg| arg == "--help" || arg == "-h") {
		print!("{}", describe(command));
		return Ok(());
	}
	match command.run {
		Run::Alone(run) => run(args),
		Run::World(run) => {
			let (args, hold_max) = hold::take(command.name, args)?;
			let pid = match pid {
				Some(pid) => pid,
				None => only_world()?
			};
			let mut world = World::connect(pid)?;
			if let Some(seconds) = hold_max {
				world.hold_max(seconds)?;
			}
			let result = run(&mut world, &args);
			world.leave();
			result
		}
	}
}

fn wait(world: &mut World, args: &[String]) -> Result<()> {
	// For a hot reload, or a reboot: not a pause
	if let Some(waited) = log::wait_on_record(world, args) {
		return waited;
	}
	let flags = Flags::parse(
		args,
		&["--break", "--if", "--log", "--timeout", "--change", "--frame"],
		&["--runtimes", "--caught", "--no-hold"]
	)?;
	flags.nothing_else("wait")?;
	world.wait(Wait {
		breaks: flags.values("--break").map(str::to_owned).collect(),
		changes: flags.values("--change").map(str::to_owned).collect(),
		frame: flags.number::<u32>("--frame")?.unwrap_or(0),
		condition: flags.value("--if").map(str::to_owned),
		log: flags.value("--log").map(LogPart::parse),
		runtimes: flags.has("--runtimes") || flags.has("--caught"),
		caught: flags.has("--caught"),
		timeout: flags.number::<f64>("--timeout")?.map(Duration::from_secs_f64),
		hold: !flags.has("--no-hold")
	})
}

fn only_world() -> Result<u32> {
	let worlds = local::worlds();
	let shown = |worlds: &[&local::World]| {
		worlds
			.iter()
			.map(|w| format!("  {} {}", w.pid, w.dmb.as_ref().map_or(w.program.clone(), |dmb| dmb.display().to_string())))
			.collect::<Vec<_>>()
			.join("\n")
	};
	let ours: Vec<&local::World> = worlds.iter().filter(|world| world.compatible).collect();
	let others: Vec<&local::World> = worlds.iter().filter(|world| !world.compatible).collect();
	match ours.as_slice() {
		[world] => Ok(world.pid),
		[] if others.is_empty() => Err("no world is running the debug library".into()),
		[] => Err(format!(
			"no world is running this auxtools' debug library (protocol {}). Started by another version:\n{}",
			PROTOCOL_VERSION,
			others
				.iter()
				.map(|w| format!("  {} {} (protocol {})", w.pid, w.program, w.protocol))
				.collect::<Vec<_>>()
				.join("\n")
		)
		.into()),
		_ => Err(format!("more than one world is running the debug library, so say which with --pid:\n{}", shown(&ours)).into())
	}
}

struct Wait {
	breaks: Vec<String>,
	/// Expressions for places to watch for a change
	changes: Vec<String>,
	/// The frame they're of, when the world is paused
	frame: u32,
	condition: Option<String>,
	/// A message the breakpoints print at each hit, in place of pausing
	log: Option<Vec<LogPart>>,
	runtimes: bool,
	/// Those inside a `try` too
	caught: bool,
	timeout: Option<Duration>,
	hold: bool
}

pub struct World {
	/// Its process
	pid: u32,
	stream: Shared,
	received: Receiver<io::Result<Response>>,
	/// Pauses and resumes that came between a request and its response
	events: Vec<Response>,
	/// What the world said in notifications, like why an expression failed
	notes: Vec<String>
}

impl World {
	fn connect(pid: u32) -> Result<Self> {
		let stream = Shared::new(local::connect(pid).map_err(|e| format!("couldn't reach the world in process {}: {}", pid, e))?);
		let (sender, received) = mpsc::channel();
		let mut reader = stream.clone();
		std::thread::spawn(move || loop {
			let response = local::receive(&mut reader);
			let failed = response.is_err();
			if sender.send(response).is_err() || failed {
				break;
			}
		});
		Ok(Self {
			pid,
			stream,
			received,
			events: vec![],
			notes: vec![]
		})
	}

	fn leave(&mut self) {
		let _ = local::send(&mut self.stream, &Request::Disconnect);
	}

	/// The next message, other than notifications. `None` when `until` passes
	/// first.
	fn next(&mut self, until: Option<Instant>) -> Result<Option<Response>> {
		loop {
			let response = match until {
				Some(until) => match self.received.recv_timeout(until.saturating_duration_since(Instant::now())) {
					Ok(response) => response,
					Err(RecvTimeoutError::Timeout) => return Ok(None),
					Err(RecvTimeoutError::Disconnected) => return Err(hung_up())
				},
				None => self.received.recv().map_err(|_| hung_up())?
			};
			match response.map_err(|_| hung_up())? {
				Response::Disconnect => return Err(hung_up()),
				Response::Notification { message } => self.notes.push(message),
				Response::CodeChanged => {}
				// A breakpoint's message, as it comes
				Response::Log { message, .. } => println!("{}", message),
				Response::WatchEnded { reason, .. } => println!("a watch is over: {}", reason),
				response => return Ok(Some(response))
			}
		}
	}

	fn request(&mut self, request: Request) -> Result<Response> {
		local::send(&mut self.stream, &request).map_err(|_| hung_up())?;
		loop {
			match self.next(None)?.expect("no time limit") {
				event @ (Response::BreakpointHit { .. } | Response::Resumed) => self.events.push(event),
				response => return Ok(response)
			}
		}
	}

	/// Why it's paused, if it is
	fn paused(&mut self) -> Result<Option<BreakpointReason>> {
		match self.request(Request::Status)? {
			Response::Status { paused } => Ok(paused.map(|stop| stop.reason)),
			other => Err(unexpected(&other))
		}
	}

	fn status(&mut self) -> Result<()> {
		match self.paused()? {
			None => println!("running"),
			Some(reason) => self.print_pause(&reason)?
		}
		Ok(())
	}

	fn frames(&mut self, stack_id: u32, count: Option<u32>) -> Result<Vec<StackFrame>> {
		match self.request(Request::StackFrames {
			stack_id,
			start_frame: None,
			count
		})? {
			Response::StackFrames { frames, .. } => Ok(frames),
			other => Err(unexpected(&other))
		}
	}

	fn print_pause(&mut self, reason: &BreakpointReason) -> Result<()> {
		let why = match reason {
			BreakpointReason::Breakpoint => "at a breakpoint".to_owned(),
			BreakpointReason::Step => "after a step".to_owned(),
			BreakpointReason::Pause => "where it was asked to".to_owned(),
			BreakpointReason::Runtime { message, caught: false } => format!("on a runtime error: {}", message),
			BreakpointReason::Runtime { message, caught: true } => format!("on a caught runtime error: {}", message),
			BreakpointReason::Data { changes } => {
				let changes: Vec<String> = changes
					.iter()
					.map(|change| format!("{} changed from {} to {}", change.name, change.old, change.new))
					.collect();
				format!("after a change: {}", changes.join("; "))
			}
		};
		println!("paused {}", why);
		self.print_stack(0, Some(20))
	}

	fn print_stack(&mut self, stack_id: u32, count: Option<u32>) -> Result<()> {
		for (i, frame) in self.frames(stack_id, count)?.iter().enumerate() {
			println!("#{} {}", i, place(frame));
		}
		Ok(())
	}

	fn need_pause(&mut self) -> Result<()> {
		match self.paused()? {
			Some(_) => Ok(()),
			None => Err("the world is running: `pause` it, or `wait` for it to pause".into())
		}
	}

	fn stack(&mut self, all: bool) -> Result<()> {
		self.need_pause()?;
		// Every sleeping proc, when they're asked for
		let stacks = match self.request(Request::Stacks {
			sleeping: Some(if all { u32::MAX } else { 0 })
		})? {
			Response::Stacks { stacks, .. } => stacks,
			other => return Err(unexpected(&other))
		};
		for stack in stacks.iter().filter(|s| all || !s.sleeping) {
			if all {
				println!("stack {}: {}{}", stack.id, stack.name, if stack.sleeping { " (sleeping)" } else { "" });
			}
			self.print_stack(stack.id, None)?;
		}
		Ok(())
	}

	/// The id of the `n`th frame of the paused stack
	fn frame_id(&mut self, n: u32) -> Result<u32> {
		self.need_pause()?;
		let frames = self.frames(0, Some(n + 1))?;
		frames
			.get(n as usize)
			.map(|f| f.id)
			.ok_or_else(|| format!("there's no frame {}", n).into())
	}

	fn vars(&mut self, frame: u32, globals: bool) -> Result<()> {
		let frame_id = self.frame_id(frame)?;
		let Response::Scopes {
			arguments,
			locals,
			globals: global_vars
		} = self.request(Request::Scopes { frame_id })?
		else {
			return Err("the world didn't say what the frame has".into());
		};
		let mut scopes = vec![("arguments", arguments), ("locals", locals)];
		if globals {
			scopes.push(("globals", global_vars));
		}
		for (name, vars) in scopes {
			let Some(vars) = vars else { continue };
			let listed = self.request(Request::Variables {
				vars,
				start: None,
				count: None
			})?;
			let Response::Variables { vars } = listed else {
				return Err(format!("the world didn't list the {}", name).into());
			};
			println!("{}:", name);
			for var in vars {
				println!("  {} = {}", var.name, var.value);
			}
		}
		Ok(())
	}

	fn eval(&mut self, expression: &str, frame: Option<u32>) -> Result<()> {
		let frame_id = match frame {
			Some(n) => Some(self.frame_id(n)?),
			// The top frame, if it's paused
			None => match self.paused()? {
				Some(_) => Some(self.frame_id(0)?),
				None => None
			}
		};
		self.notes.clear();
		let result = match self.request(Request::Eval {
			frame_id,
			command: expression.to_owned(),
			context: Some("repl".to_owned())
		})? {
			Response::Eval(result) => result,
			other => return Err(unexpected(&other))
		};
		// What the world said on the way
		for note in std::mem::take(&mut self.notes) {
			eprintln!("{}", note);
		}
		println!("{}", result?.value);
		Ok(())
	}

	/// Waits for a pause, and keeps it held after we leave
	fn await_pause(&mut self, until: Option<Instant>, hold: bool) -> Result<Option<BreakpointReason>> {
		let reason = loop {
			if let Some(i) = self.events.iter().position(|e| matches!(e, Response::BreakpointHit { .. })) {
				let Response::BreakpointHit(stop) = self.events.remove(i) else {
					unreachable!()
				};
				break stop.reason;
			}
			match self.next(until)? {
				None => return Ok(None),
				Some(event @ Response::BreakpointHit { .. }) => self.events.push(event),
				Some(_) => {}
			}
		};
		if hold {
			self.request(Request::HoldPause)?;
		}
		Ok(Some(reason))
	}

	fn pause(&mut self) -> Result<()> {
		if let Some(reason) = self.paused()? {
			self.request(Request::HoldPause)?;
			return self.print_pause(&reason);
		}
		self.request(Request::Pause)?;
		let reason = self.await_pause(None, true)?.expect("no time limit");
		self.print_pause(&reason)
	}

	fn resume(&mut self, kind: ContinueKind) -> Result<()> {
		self.need_pause()?;
		let stepping = kind != ContinueKind::Continue;
		self.events.clear();
		self.request(Request::Continue { kind })?;
		if !stepping {
			println!("running");
			return Ok(());
		}
		// A step that leaves the last proc ends with nothing to pause in
		let until = Instant::now() + Duration::from_secs(5);
		match self.await_pause(Some(until), true)? {
			Some(reason) => self.print_pause(&reason),
			None => {
				println!("running (the step left the code that was running)");
				Ok(())
			}
		}
	}

	fn wait(&mut self, wait: Wait) -> Result<()> {
		let paused = self.paused()?;
		if let (Some(reason), true) = (&paused, wait.changes.is_empty()) {
			if wait.hold {
				self.request(Request::HoldPause)?;
			}
			println!("already paused");
			return self.print_pause(reason);
		}
		// What's watched from a pause is of a frame there, and the wait is
		// for the world to carry on and change it
		let frame_id = match &paused {
			Some(_) => Some(self.frame_id(wait.frame)?),
			None => None
		};
		for expression in &wait.changes {
			let watch = Request::WatchSet {
				place: WatchPlace::Expression {
					frame_id,
					expression: expression.clone()
				},
				condition: wait.condition.clone(),
				hit: None
			};
			match self.request(watch)? {
				Response::WatchSet(Ok(watch)) => println!("watching {}, now {}", watch.name, watch.value),
				Response::WatchSet(Err(why)) => return Err(format!("couldn't watch {}: {}", expression, why).into()),
				other => return Err(unexpected(&other))
			}
		}
		// The condition is the watches' when there are any
		let condition = if wait.changes.is_empty() { wait.condition.clone() } else { None };
		for place in &wait.breaks {
			self.set_breakpoint(place, condition.clone(), wait.log.clone())?;
		}
		if paused.is_some() {
			self.events.clear();
			self.request(Request::Continue { kind: ContinueKind::Continue })?;
		}
		if wait.runtimes {
			// It has no answer
			let catch = Request::CatchRuntimes {
				uncaught: Some(RuntimeFilter::default()),
				caught: wait.caught.then(RuntimeFilter::default)
			};
			local::send(&mut self.stream, &catch).map_err(|_| hung_up())?;
		}
		let until = wait.timeout.map(|t| Instant::now() + t);
		match self.await_pause(until, wait.hold)? {
			Some(reason) => self.print_pause(&reason),
			None => Err(Failure(TIMED_OUT, "timed out: the world didn't pause".to_owned()))
		}
	}

	/// At `FILE:LINE`, or the start of `/proc/path` (`#N` for an override)
	fn set_breakpoint(&mut self, place: &str, condition: Option<String>, log: Option<Vec<LogPart>>) -> Result<()> {
		let instructions = if place.starts_with('/') {
			let (path, override_id) = match place.split_once('#') {
				Some((path, id)) => (path.to_owned(), id.parse().map_err(|_| format!("{} isn't an override number", id))?),
				None => (place.to_owned(), 0)
			};
			vec![InstructionRef {
				proc: ProcRef { path, override_id },
				offset: 0
			}]
		} else {
			let (file, line) = place
				.rsplit_once(':')
				.and_then(|(file, line)| Some((file, line.parse::<u32>().ok()?)))
				.ok_or_else(|| format!("{} is neither FILE:LINE nor /proc/path", place))?;
			match self.request(Request::Locate { file: file.to_owned(), line })? {
				Response::Locate { instructions } if instructions.is_empty() => {
					return Err(format!(
						"no proc in {} has code on line {} or after it (or no file of the world's ends with that path)",
						file, line
					)
					.into());
				}
				Response::Locate { instructions } => instructions,
				other => return Err(unexpected(&other))
			}
		};
		for instruction in instructions {
			match self.request(Request::BreakpointSet {
				instruction,
				condition: condition.clone(),
				hit: None,
				log: log.clone()
			})? {
				Response::BreakpointSet {
					result: BreakpointSetResult::Success { .. }
				} => {}
				Response::BreakpointSet {
					result: BreakpointSetResult::Refused { reason }
				} => return Err(format!("couldn't break at {}: {}", place, reason).into()),
				Response::BreakpointSet { .. } => return Err(format!("couldn't break at {}: there's no such proc", place).into()),
				other => return Err(unexpected(&other))
			}
		}
		Ok(())
	}
}

fn place(frame: &StackFrame) -> String {
	let proc = &frame.instruction.proc;
	let name = match proc.override_id {
		0 => proc.path.clone(),
		n => format!("{}#{}", proc.path, n)
	};
	let name = if frame.old_code { format!("{} (old code)", name) } else { name };
	match (&frame.file, frame.line) {
		(Some(file), Some(line)) => format!("{}  {}:{}", name, file, line),
		(None, Some(line)) => format!("{}  line {}", name, line),
		// At its start, before its first line
		(Some(file), None) => format!("{}  {}", name, file),
		(None, None) => name
	}
}

fn hung_up() -> Failure {
	Failure(HUNG_UP, "the world hung up (it ended or rebooted)".to_owned())
}

fn unexpected(response: &Response) -> Failure {
	Failure(1, format!("the world answered with {:?}", response))
}

#[cfg(test)]
mod tests {
	use super::{command, describe, overview, parse, Flags, COMMANDS};

	fn args(args: &[&str]) -> Vec<String> {
		args.iter().map(|arg| (*arg).to_owned()).collect()
	}

	#[test]
	fn the_process_comes_first() {
		let (pid, rest) = parse(&args(&["--pid", "1234", "eval", "1"])).ok().unwrap();
		assert_eq!((pid, rest), (Some(1234), args(&["eval", "1"])));
		let (pid, rest) = parse(&args(&["status"])).ok().unwrap();
		assert_eq!((pid, rest), (None, args(&["status"])));
		assert!(parse(&args(&["--pid", "soon", "status"])).is_err());
	}

	#[test]
	fn every_command_is_a_word_in_the_help() {
		let help = overview();
		let mut names = vec![];
		for (_, commands) in COMMANDS {
			for listed in *commands {
				assert!(!listed.name.starts_with('-') && !listed.name.contains(' '), "{}", listed.name);
				assert!(help.contains(&format!("  {} ", listed.name)), "{} isn't in the help", listed.name);
				assert!(!listed.about.is_empty() && !listed.about.ends_with('.'), "{}: {:?}", listed.name, listed.about);
				assert!(describe(listed).starts_with(&format!("Usage: auxtools [--pid PID] {}", listed.name)));
				names.push(listed.name);
			}
		}
		for name in ["dap", "list", "version", "help", "status", "wait", "eval", "launch", "stop", "reboot"] {
			assert!(command(name).is_some(), "{}", name);
		}
		for gone in ["--list", "--build", "--version", "run", ""] {
			assert!(command(gone).is_none(), "{}", gone);
		}
		names.sort_unstable();
		names.dedup();
		assert_eq!(names.len(), COMMANDS.iter().map(|(_, commands)| commands.len()).sum::<usize>(), "a name is there twice");
	}

	#[test]
	fn no_line_of_the_help_is_wider_than_a_terminal() {
		for text in std::iter::once(overview()).chain(COMMANDS.iter().flat_map(|(_, commands)| commands.iter()).map(describe)) {
			for line in text.lines() {
				assert!(line.chars().count() <= 100, "{} characters: {}", line.chars().count(), line);
			}
		}
	}

	#[test]
	fn flags_take_values_or_stand_alone() {
		let flags = Flags::parse(
			&args(&["--break", "a.dm:3", "--runtimes", "--break", "/proc/x", "--timeout", "2.5"]),
			&["--break", "--timeout"],
			&["--runtimes", "--no-hold"]
		)
		.ok()
		.unwrap();
		assert_eq!(flags.values("--break").collect::<Vec<_>>(), ["a.dm:3", "/proc/x"]);
		assert!(flags.has("--runtimes") && !flags.has("--no-hold"));
		assert_eq!(flags.number::<f64>("--timeout").ok().unwrap(), Some(2.5));
		assert_eq!(flags.number::<u32>("--frame").ok().unwrap(), None);
		assert!(flags.rest.is_empty());
	}

	#[test]
	fn what_isnt_a_flag_is_left_in_order() {
		let flags = Flags::parse(&args(&["6", "*", "--frame", "2", "7"]), &["--frame"], &[]).ok().unwrap();
		assert_eq!(flags.rest, args(&["6", "*", "7"]));
		assert_eq!(flags.number::<u32>("--frame").ok().unwrap(), Some(2));
	}

	#[test]
	fn the_last_of_a_flag_given_twice_counts() {
		let flags = Flags::parse(&args(&["--frame", "1", "--frame", "3"]), &["--frame"], &[]).ok().unwrap();
		assert_eq!(flags.value("--frame"), Some("3"));
	}

	#[test]
	fn flags_that_are_wrong_are_errors() {
		assert!(Flags::parse(&args(&["--sideways"]), &["--frame"], &["--all"]).is_err());
		assert!(Flags::parse(&args(&["--frame"]), &["--frame"], &[]).is_err());
		let flags = Flags::parse(&args(&["--frame", "top"]), &["--frame"], &[]).ok().unwrap();
		assert!(flags.number::<u32>("--frame").is_err());
	}
}
