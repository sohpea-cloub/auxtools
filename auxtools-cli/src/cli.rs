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
mod trace;
mod json;

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

/// What `status --json` says, which commands that print a pause share
/// What `status` says, and what `--json` makes of a pause for every command
/// that prints one
const STATUS: &str = "The first line is `running`, or `paused` and why; a paused world's stack follows. Then
how long the world has been up (since it last loaded, and its world.time), and what came of
its last hot reload.
  --json   As JSON: {\"paused\": false}, or {\"paused\": true} with the `reason`
           (`breakpoint`, `step`, `pause`, `runtime` or `change`), the runtime's
           `message` and whether it's `caught`, the `changes` (each with its `name`,
           `old` and `new`), the `stack` (as `auxtools help stack` says), and the
           `instruction` (its `offset` and `text`) after a step of one";

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
			usage: "[--json]",
			about: "Whether the world is running or paused, and if paused, why and where",
			details: STATUS,
			run: Run::World(|world, args| {
				let flags = Flags::parse(args, &[], &["--json"])?;
				flags.nothing_else("status")?;
				world.output(&flags)?;
				world.status()?;
				// (the JSON document is the pause's alone)
				if flags.has("--json") {
					Ok(())
				} else {
					world.uptime()
				}
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
			usage: "[--all] [--json]",
			about: "The procs the world is paused in, innermost first",
			details: "  --all    The sleeping procs' stacks too, each with the id it keeps
  --json   As JSON: an array of frames, each with its `proc`, `file`, `line`, `offset`
           (its instruction's) and `old_code`. With --all, an array of stacks, each
           with its `id`, `name`, `sleeping` and `frames`.",
			run: Run::World(|world, args| {
				let flags = Flags::parse(args, &[], &["--all", "--json"])?;
				flags.nothing_else("stack")?;
				world.output(&flags)?;
				world.stack(flags.has("--all"))
			})
		},
		Command {
			name: "vars",
			usage: "[--frame N] [--globals] [--json [--depth N]]",
			about: "A frame's arguments and local vars",
			details: "  --frame N    The Nth frame of the stack, from 0, the innermost (the default)
  --globals    The world's global vars too
  --json       As JSON: an object with `arguments`, `locals` (and `globals`), each an
               object of the vars' values, as `auxtools help eval` says. The vars are
               a level down: by default a list in them is in full, an object isn't.
  --depth N    How deep to write out lists and objects (2 unless given)",
			run: Run::World(|world, args| {
				let flags = Flags::parse(args, &["--frame", "--depth"], &["--globals", "--json"])?;
				flags.nothing_else("vars")?;
				world.output(&flags)?;
				world.vars(flags.number("--frame")?.unwrap_or(0), flags.has("--globals"))
			})
		},
		Command {
			name: "eval",
			usage: "EXPRESSION | -f FILE | - [--frame N] [--json [--depth N]]",
			about: "The value of a DM expression",
			details: "In a paused world, the expression is in a frame of the stack: it can name that
proc's vars, and `src`. In a running world it can name globals and call procs.
When it fails, it says why on stderr and exits with 1: for a runtime error in a proc
it called, with that proc's file and line and the procs between it and the expression.
  -f FILE      Read the expression from FILE, where no shell can change its quotes or paths
  -            Read the expression from stdin
  --frame N    The Nth frame of the stack, from 0, the innermost (the default)
  --json       The value as JSON: a number, text or null as itself; a list as an array of
               its entries, or an object when they're text keys with values (else an
               array of {\"key\", \"value\"}); an object (a datum, atom, client...) as
               {\"ref\", \"type\", \"name\"}, with its \"vars\" when they're written out; a
               typepath as {\"path\"}; anything else as {\"kind\", \"text\"}
  --depth N    How deep to write out lists and objects (2 unless given): a list's entries
               while N is 1 or more, an object's vars while it's 2 or more, each a level
               lower. Past that, a list is {\"list\": true, \"len\"} and an object no vars.",
			run: Run::World(|world, args| {
				let flags = Flags::parse(args, &["--frame", "-f", "--depth"], &["--json"])?;
				world.output(&flags)?;
				let expression = expression(&flags.rest, flags.value("-f"), read_stdin)?;
				world.eval(&expression, flags.number("--frame")?)
			})
		},
	]),
	("Looking at its code", &[
		Command {
			name: "proc",
			usage: "/PATH[#N] [--json]",
			about: "Where a proc is, how many definitions it has, and whether a hot reload changed it",
			details: "/PATH is the proc's path, with or without its `proc/`: `/mob/proc/attack` or
`/mob/attack`. #N is its Nth definition, from 0, the first (the default).
  --json    As JSON: its `proc`, `file`, `line`, `definitions` and `reloaded` (null,
            `lines`, `code` or `added`)",
			run: Run::World(|world, args| {
				let flags = Flags::parse(args, &[], &["--json"])?;
				world.output(&flags)?;
				world.proc_info(&one_proc("proc", &flags.rest)?)
			})
		},
		Command {
			name: "dis",
			usage: "/PATH[#N] [--json]",
			about: "A proc's bytecode: each instruction's offset and text, and the lines they're on",
			details: "/PATH[#N] is as for `auxtools proc`. The bytecode is the proc's code as it runs
now: after a hot reload, the new code.
  --json    As JSON: its `proc`, `file`, and `instructions`, each with its `offset`,
            `text` and `line`",
			run: Run::World(|world, args| {
				let flags = Flags::parse(args, &[], &["--json"])?;
				world.output(&flags)?;
				world.disassemble(&one_proc("dis", &flags.rest)?)
			})
		},
	]),
	("Running it", &[
		Command {
			name: "pause",
			usage: "[--hold-max SECONDS] [--json]",
			about: "Pauses the world where it is",
			details: "The world stays paused after this command has gone, until `continue`.
  --hold-max SECONDS  Let the world carry on by itself this long after the command has
                      gone (once nobody debugging is connected to it). The default is
                      no limit, or AUXTOOLS_HOLD_MAX if it's set. Its `log` says so.
  --json              Where it paused as JSON, as `auxtools help status` says",
			run: Run::World(|world, args| {
				let flags = Flags::parse(args, &[], &["--json"])?;
				flags.nothing_else("pause")?;
				world.output(&flags)?;
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
			usage: "[--instruction] [--hold-max SECONDS] [--json]",
			about: "Runs to the next line, over any procs this one calls",
			details: "The world stays paused afterwards, until `continue`.
  --instruction       One instruction on, in this proc: a call it makes runs to its end
                      (the pause it ends in shows that instruction's offset and text too)
  --hold-max SECONDS  As for `pause`
  --json              Where it paused as JSON, as `auxtools help status` says",
			run: Run::World(|world, args| {
				let flags = Flags::parse(args, &[], &["--instruction", "--json"])?;
				flags.nothing_else("next")?;
				world.output(&flags)?;
				world.show_instruction = flags.has("--instruction");
				world.resume(if world.show_instruction {
					ContinueKind::StepOverInstruction { stack_id: 0 }
				} else {
					ContinueKind::StepOver { stack_id: 0 }
				})
			})
		},
		Command {
			name: "step",
			usage: "[--instruction] [--hold-max SECONDS] [--json]",
			about: "Runs to the next line, going into a proc this one calls",
			details: "The world stays paused afterwards, until `continue`.
  --instruction       One instruction on, into a proc this one calls
                      (the pause it ends in shows that instruction's offset and text too)
  --hold-max SECONDS  As for `pause`
  --json              Where it paused as JSON, as `auxtools help status` says",
			run: Run::World(|world, args| {
				let flags = Flags::parse(args, &[], &["--instruction", "--json"])?;
				flags.nothing_else("step")?;
				world.output(&flags)?;
				world.show_instruction = flags.has("--instruction");
				world.resume(if world.show_instruction {
					ContinueKind::StepIntoInstruction { stack_id: 0 }
				} else {
					ContinueKind::StepInto { stack_id: 0 }
				})
			})
		},
		Command {
			name: "out",
			usage: "[--hold-max SECONDS] [--json]",
			about: "Runs until this proc returns to the one that called it",
			details: "The world stays paused afterwards, until `continue`.
  --hold-max SECONDS  As for `pause`
  --json              Where it paused as JSON, as `auxtools help status` says",
			run: Run::World(|world, args| {
				let flags = Flags::parse(args, &[], &["--json"])?;
				flags.nothing_else("out")?;
				world.output(&flags)?;
				world.resume(ContinueKind::StepOut { stack_id: 0 })
			})
		},
		Command {
			name: "wait",
			usage: "[--break PLACE]... [--change EXPRESSION]... [--runtimes] [OPTIONS]",
			about: "Waits for the world to pause, and says why and where",
			details: "With nothing to pause it for, it waits for whatever pauses it: another client's
breakpoint, a `pause`. The world stays paused afterwards, until `continue`. To print
something at a breakpoint without pausing, or count its hits, use `trace`.
  --break PLACE        Pause at FILE:LINE, or when /proc/path is called (/proc/path#N for
                       its Nth override). Can be given more than once. Says where each
                       went (on stderr), and fails at once on a place the world hasn't got.
  --change EXPRESSION  Pause when what EXPRESSION names gets another value: a var of an
                       object, an entry of a list, a global var, a proc's own var. Given
                       while the world is paused, EXPRESSION is of a frame there, and the
                       world carries on until it changes.
  --frame N            The frame --change's EXPRESSION is of (0, the innermost)
  --if CONDITION       Only pause when CONDITION, a DM expression, is true there. For
                       --change, `value` is the new value and `old` the one before.
  --runtimes           Pause on a runtime error that nothing catches
  --caught             Pause on runtime errors inside a `try` as well
  --timeout SECONDS    Give up after this long (exit code 2)
  --no-hold            Let the world carry on once this command has gone
  --json               Where it paused as JSON, as `auxtools help status` says, with
                       `already` (it was paused before), `breakpoints` (each with the
                       `place` given and the `proc`, `file` and `line` it went to) and
                       `watching` (each with its `name` and `value`)
  --hold-max SECONDS   As for `pause`: let it carry on by itself this long after this
                       command has gone (once nobody debugging is connected to it)

wait --reload [--since N] [--timeout SECONDS] [--json]
  Waits for the next hot reload to be tried (or the first after entry N of `log`), by the
  launcher that reloads the world, and prints what came of it as `log --reloads` does.
  Exit code 0 if it was applied, 1 if not (compile errors, changes that need a rebuild).
  Start it before saving the change, or give --since.
wait --reboot [--since N] [--timeout SECONDS] [--json]
  Waits for the world to reboot (or for the first load after entry N), and for the new
  world to answer. Exit code 3 if the world ends instead.

An EXPRESSION, CONDITION or MESSAGE given as @FILE is read from FILE (@@ for one that
starts with @).",
			run: Run::World(wait)
		},
		Command {
			name: "trace",
			usage: "PLACE... [--log MESSAGE] [--by EXPRESSION|caller] [OPTIONS]",
			about: "Prints a line at each hit of breakpoints that never pause, or counts the hits",
			details: "A PLACE is FILE:LINE, or /proc/path for when the proc is called (/proc/path#N for its
Nth override). It says where each breakpoint went, then a line per hit: which proc and
line it was, and what called it. The world doesn't pause for them, and they go when
trace ends: after --for or --count, or when it's interrupted (Ctrl-C), with exit code 0;
or when the world goes away, with 3.
  --log MESSAGE         Print MESSAGE at each hit instead, with the values of the
                        {expressions} in it, worked out there
  --if CONDITION        Only the hits where CONDITION, a DM expression, is true. A hit
                        where it fails with an error is skipped: said once, and counted.
  --by EXPRESSION       Nothing per hit: at the end, how many hits there were for each
                        value of EXPRESSION, most first, and in all. A hit where it fails
                        counts under the error, in {braces}.
  --by caller           The same, for each proc and line that called it
  --for SECONDS         Stop after this long
  --count N             Stop after N hits
  --json                A JSON object a line: `bound`, `hit` and `summary` events",
			run: Run::World(trace::trace)
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
		&["--break", "--if", "--timeout", "--change", "--frame"],
		&["--runtimes", "--caught", "--no-hold", "--json"]
	)?;
	flags.nothing_else("wait")?;
	world.output(&flags)?;
	world.wait(Wait {
		breaks: flags.values("--break").map(str::to_owned).collect(),
		changes: flags.values("--change").map(given_text).collect::<Result<_>>()?,
		frame: flags.number::<u32>("--frame")?.unwrap_or(0),
		condition: flags.value("--if").map(given_text).transpose()?,
		runtimes: flags.has("--runtimes") || flags.has("--caught"),
		caught: flags.has("--caught"),
		timeout: flags.number::<f64>("--timeout")?.map(Duration::from_secs_f64),
		hold: !flags.has("--no-hold")
	})
}

/// An expression or a message as an option was given it: `@FILE` is what's
/// in FILE, `@@...` is `@...`. A shell can't mangle what's in a file.
fn given_text(value: &str) -> Result<String> {
	match value.strip_prefix('@') {
		Some(rest) if rest.starts_with('@') => Ok(rest.to_owned()),
		Some(file) => read_text(file),
		None => Ok(value.to_owned())
	}
}

/// What's in a file, without the line end it finishes with
fn read_text(file: &str) -> Result<String> {
	let text = std::fs::read_to_string(file).map_err(|e| format!("couldn't read {}: {}", file, e))?;
	Ok(text.trim_end_matches(['\r', '\n']).to_owned())
}

fn read_stdin() -> Result<String> {
	use std::io::{IsTerminal, Read};
	// Nobody would type it in: an agent waiting on us would wait forever
	if io::stdin().is_terminal() {
		return Err("`-` reads the expression from stdin, and stdin is a terminal here".into());
	}
	let mut text = String::new();
	io::stdin().read_to_string(&mut text).map_err(|e| format!("couldn't read stdin: {}", e))?;
	Ok(text.trim_end_matches(['\r', '\n']).to_owned())
}

/// `eval`'s expression: its words, what's in `-f FILE`, or stdin for `-`
fn expression(words: &[String], file: Option<&str>, stdin: impl FnOnce() -> Result<String>) -> Result<String> {
	let expression = match (words, file) {
		([], None) => return Err("eval what?".into()),
		([], Some(file)) => read_text(file)?,
		([dash], None) if dash == "-" => stdin()?,
		(_, None) => words.join(" "),
		(_, Some(_)) => return Err("eval takes one expression: its words, -f FILE or -".into())
	};
	if expression.trim().is_empty() {
		return Err(match file {
			Some(file) => format!("{} has no expression in it", file),
			None => "stdin had no expression on it".to_owned()
		}
		.into());
	}
	Ok(expression)
}

/// A proc as it's written on the command line: `/path`, or `/path#N` for its
/// Nth definition
fn proc_ref(text: &str) -> Result<ProcRef> {
	if !text.starts_with('/') {
		return Err(format!("{} isn't a proc's path: it starts with /", text).into());
	}
	let (path, override_id) = match text.split_once('#') {
		Some((path, id)) => (path, id.parse().map_err(|_| format!("{} isn't a definition's number", id))?),
		None => (text, 0)
	};
	Ok(ProcRef {
		path: path.to_owned(),
		override_id
	})
}

/// The one proc a command is about
fn one_proc(command: &str, words: &[String]) -> Result<ProcRef> {
	match words {
		[proc] => proc_ref(proc),
		[] => Err(format!("{} which proc?", command).into()),
		_ => Err(format!("{} takes one proc", command).into())
	}
}

/// A proc's name as the stack shows it: its path, and `#N` for a definition
/// after the first
fn proc_name(proc: &ProcRef) -> String {
	match proc.override_id {
		0 => proc.path.clone(),
		n => format!("{}#{}", proc.path, n)
	}
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
	notes: Vec<String>,
	/// What it said of breakpoints' conditions that failed, once each
	failed_conditions: Vec<String>,
	/// A pause it prints shows the instruction it's on, after a step of one
	show_instruction: bool,
	/// What it prints is one JSON document (`--json`), with lists and objects
	/// to this depth
	json: Option<u32>,
	/// What `wait` adds to the JSON of a pause
	waited: serde_json::Map<String, serde_json::Value>
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
			notes: vec![],
			failed_conditions: vec![],
			show_instruction: false,
			json: None,
			waited: serde_json::Map::new()
		})
	}

	/// How it prints what it prints: as text, or with `--json`
	fn output(&mut self, flags: &Flags) -> Result<()> {
		let depth = flags.number::<u32>("--depth")?;
		self.json = match (flags.has("--json"), depth) {
			(true, depth) => Some(depth.unwrap_or(json::DEFAULT_DEPTH)),
			(false, None) => None,
			(false, Some(_)) => return Err("--depth goes with --json".into())
		};
		Ok(())
	}

	/// What's printed on stdout but isn't the answer: on stderr when stdout
	/// has a JSON document to itself
	fn aside(&self, text: &str) {
		if self.json.is_some() {
			eprintln!("{}", text);
		} else {
			println!("{}", text);
		}
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
				// Said once for each breakpoint whose condition fails, as it
				// comes: those hits are skipped
				Response::ConditionFailed { message } => {
					eprintln!("{}", message);
					self.failed_conditions.push(message);
				}
				Response::WatchEnded { reason, .. } => self.aside(&format!("a watch is over: {}", reason)),
				response => return Ok(Some(response))
			}
		}
	}

	fn request(&mut self, request: Request) -> Result<Response> {
		local::send(&mut self.stream, &request).map_err(|_| hung_up())?;
		loop {
			match self.next(None)?.expect("no time limit") {
				event @ (Response::BreakpointHit { .. } | Response::Resumed | Response::Log { .. }) => self.events.push(event),
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
			None if self.json.is_some() => println!("{}", serde_json::json!({ "paused": false })),
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
		let frames = self.frames(0, Some(20))?;
		if self.json.is_some() {
			let instruction = match (self.show_instruction, frames.first()) {
				(true, Some(top)) => Some(self.instruction_of(top)?),
				_ => None
			};
			let mut document = json::pause(reason, &frames, instruction);
			document.extend(std::mem::take(&mut self.waited));
			println!("{}", serde_json::Value::Object(document));
			return Ok(());
		}
		println!("paused {}", why);
		for (i, frame) in frames.iter().enumerate() {
			println!("#{} {}", i, place(frame));
			if i == 0 && self.show_instruction {
				let (offset, text) = self.instruction_of(frame)?;
				println!("   instruction {}: {}", offset, text);
			}
		}
		Ok(())
	}

	/// The offset of the instruction a frame is about to run, and its text
	fn instruction_of(&mut self, frame: &StackFrame) -> Result<(u32, String)> {
		let offset = frame.instruction.offset;
		let disassembly = self.instructions(frame.code)?;
		let text = disassembly
			.instructions
			.iter()
			.find(|instruction| u32::from(instruction.offset) == offset)
			.map(|instruction| instruction.text.clone())
			.filter(|text| !text.is_empty())
			.unwrap_or_else(|| "(not read)".to_owned());
		Ok((offset, text))
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
		if self.json.is_some() {
			let mut listed = vec![];
			for stack in stacks.iter().filter(|s| all || !s.sleeping) {
				let frames: Vec<_> = self.frames(stack.id, None)?.iter().map(json::frame).collect();
				listed.push(match all {
					true => serde_json::json!({ "id": stack.id, "name": stack.name, "sleeping": stack.sleeping, "frames": frames }),
					false => serde_json::Value::Array(frames)
				});
			}
			let document = match all {
				true => serde_json::Value::Array(listed),
				false => listed.pop().unwrap_or_default()
			};
			println!("{}", document);
			return Ok(());
		}
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
		// The vars are a level down from the frame
		if let Some(depth) = self.json {
			let mut document = serde_json::Map::new();
			for (name, vars) in scopes {
				let Some(vars) = vars else { continue };
				let depth = depth.saturating_sub(1);
				let values = match self.request(Request::RenderVariables { vars, depth })? {
					Response::RenderVariables(values) => values?,
					other => return Err(unexpected(&other))
				};
				document.insert(name.to_owned(), json::named(&values));
			}
			println!("{}", serde_json::Value::Object(document));
			return Ok(());
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
		if let Some(depth) = self.json {
			let result = match self.request(Request::Render {
				frame_id,
				expression: expression.to_owned(),
				depth
			})? {
				Response::Render(result) => result,
				other => return Err(unexpected(&other))
			};
			for note in std::mem::take(&mut self.notes) {
				eprintln!("{}", note);
			}
			println!("{}", json::value(&result?));
			return Ok(());
		}
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

	/// What the world knows of a proc
	fn find_proc(&mut self, proc: &ProcRef) -> Result<ProcInfo> {
		match self.request(Request::ProcInfo { proc: proc.clone() })? {
			Response::ProcInfo(info) => Ok(info?),
			other => Err(unexpected(&other))
		}
	}

	fn proc_info(&mut self, proc: &ProcRef) -> Result<()> {
		let info = self.find_proc(proc)?;
		if self.json.is_some() {
			let reloaded = info.reloaded.map(|how| match how {
				Reloaded::Lines => "lines",
				Reloaded::Code => "code",
				Reloaded::Added => "added"
			});
			let document = serde_json::json!({
				"proc": proc_name(&info.proc),
				"file": info.file,
				"line": info.line,
				"definitions": info.definitions,
				"reloaded": reloaded
			});
			println!("{}", document);
			return Ok(());
		}
		let mut place = proc_name(&info.proc);
		match (&info.file, info.line) {
			(Some(file), Some(line)) => place += &format!("  {}:{}", file, line),
			(Some(file), None) => place += &format!("  {}", file),
			(None, Some(line)) => place += &format!("  line {}", line),
			(None, None) => {}
		}
		println!("{}", place);
		println!("definitions: {} (#0 to #{})", info.definitions, info.definitions.saturating_sub(1));
		let reloaded = match info.reloaded {
			None => "not since the world started",
			Some(Reloaded::Lines) => "its code is as it was, on other lines",
			Some(Reloaded::Code) => "new code",
			Some(Reloaded::Added) => "added it"
		};
		println!("hot reload: {}", reloaded);
		Ok(())
	}

	/// The instructions of some code, as the world reads them
	fn instructions(&mut self, code: CodeRef) -> Result<Disassembly> {
		match self.request(Request::Disassemble { code })? {
			Response::Disassemble(disassembly) => Ok(disassembly?),
			other => Err(unexpected(&other))
		}
	}

	fn disassemble(&mut self, proc: &ProcRef) -> Result<()> {
		let info = self.find_proc(proc)?;
		let disassembly = self.instructions(info.code)?;
		if self.json.is_some() {
			let instructions: Vec<_> = disassembly
				.instructions
				.iter()
				.map(|instruction| serde_json::json!({ "offset": instruction.offset, "text": instruction.text, "line": instruction.line }))
				.collect();
			let document = serde_json::json!({
				"proc": proc_name(&disassembly.proc),
				"file": disassembly.file,
				"instructions": instructions
			});
			println!("{}", document);
			return Ok(());
		}
		match &disassembly.file {
			Some(file) => println!("{}  {}", proc_name(&disassembly.proc), file),
			None => println!("{}", proc_name(&disassembly.proc))
		}
		let mut line = None;
		for instruction in &disassembly.instructions {
			if instruction.line != line && instruction.line.is_some() {
				line = instruction.line;
				println!("line {}:", line.unwrap());
			}
			let text = match (&instruction.text, &disassembly.error) {
				(text, _) if !text.is_empty() => text.clone(),
				(_, Some(error)) => format!("(not read: {})", error),
				(_, None) => "(not read)".to_owned()
			};
			println!("{:>6}  {}", instruction.offset, text);
		}
		Ok(())
	}

	/// Where a breakpoint `wait` set went: on stderr, or in the JSON
	fn say_bound(&mut self, place: &str, bound: &Bound) {
		if self.json.is_none() {
			eprintln!("{}", bound.describe());
			return;
		}
		let said = serde_json::json!([{ "place": place, "proc": proc_name(&bound.at.proc), "file": bound.file, "line": bound.line }]);
		self.say_waited("breakpoints", said, "");
	}

	/// Something `wait` says before the pause: a line of text, or a key of
	/// the pause's JSON (an array's entries are added to it)
	fn say_waited(&mut self, key: &str, value: serde_json::Value, text: &str) {
		if self.json.is_none() {
			println!("{}", text);
			return;
		}
		match (self.waited.get_mut(key), value) {
			(Some(serde_json::Value::Array(had)), serde_json::Value::Array(more)) => had.extend(more),
			(_, value) => {
				self.waited.insert(key.to_owned(), value);
			}
		}
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
		if self.json.is_some() {
			// The same keys each time
			self.waited = serde_json::json!({ "already": false, "breakpoints": [], "watching": [] }).as_object().cloned().unwrap_or_default();
		}
		let paused = self.paused()?;
		// Each place first, so one that isn't there fails before anything
		// else. The condition is the watches' when there are any.
		let condition = if wait.changes.is_empty() { wait.condition.clone() } else { None };
		for place in &wait.breaks {
			for bound in self.set_breakpoint(place, condition.clone(), None)? {
				self.say_bound(place, &bound);
			}
		}
		if let (Some(reason), true) = (&paused, wait.changes.is_empty()) {
			if wait.hold {
				self.request(Request::HoldPause)?;
			}
			self.say_waited("already", serde_json::Value::Bool(true), "already paused");
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
				Response::WatchSet(Ok(watch)) => {
					let said = serde_json::json!([{ "name": watch.name, "value": watch.value }]);
					self.say_waited("watching", said, &format!("watching {}, now {}", watch.name, watch.value));
				}
				Response::WatchSet(Err(why)) => return Err(format!("couldn't watch {}: {}", expression, why).into()),
				other => return Err(unexpected(&other))
			}
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

	/// At `FILE:LINE`, or the start of `/proc/path` (`#N` for an override):
	/// where each breakpoint went (a line can be in more than one proc). A
	/// place the world doesn't have is an error. With a `log`, it says that
	/// at each hit (`Response::Log`) and doesn't pause.
	fn set_breakpoint(&mut self, place: &str, condition: Option<String>, log: Option<Vec<LogPart>>) -> Result<Vec<Bound>> {
		let (instructions, asked_line) = if place.starts_with('/') {
			// Asked first, for what's wrong with it if it isn't there
			let proc = self.find_proc(&proc_ref(place)?).map_err(|Failure(code, why)| Failure(code, format!("couldn't break at {}: {}", place, why)))?;
			(vec![InstructionRef { proc: proc.proc, offset: 0 }], None)
		} else {
			let (file, line) = place
				.rsplit_once(':')
				.and_then(|(file, line)| Some((file, line.parse::<u32>().ok()?)))
				.ok_or_else(|| format!("{} is neither FILE:LINE nor /proc/path", place))?;
			match self.request(Request::Locate { file: file.to_owned(), line })? {
				Response::Locate { instructions } if instructions.is_empty() => return Err(self.nowhere(file, line)?),
				Response::Locate { instructions } => (instructions, Some(line)),
				other => return Err(unexpected(&other))
			}
		};
		let mut bound = vec![];
		for instruction in instructions {
			match self.request(Request::BreakpointSet {
				instruction: instruction.clone(),
				condition: condition.clone(),
				hit: None,
				log: log.clone()
			})? {
				Response::BreakpointSet {
					result: BreakpointSetResult::Success { line, at, file }
				} => bound.push(Bound {
					place: place.to_owned(),
					asked: instruction,
					at,
					file,
					line,
					asked_line
				}),
				Response::BreakpointSet {
					result: BreakpointSetResult::Refused { reason }
				} => return Err(format!("couldn't break at {}: {}", place, reason).into()),
				Response::BreakpointSet { .. } => return Err(format!("couldn't break at {}: there's no such proc", place).into()),
				other => return Err(unexpected(&other))
			}
		}
		Ok(bound)
	}

	/// Takes a breakpoint away: how many of its hits were skipped because its
	/// condition failed
	fn unset_breakpoint(&mut self, bound: &Bound) -> Result<u32> {
		// By what it was set with, which is how the world knows it
		match self.request(Request::BreakpointUnset {
			instruction: bound.asked.clone()
		})? {
			Response::BreakpointUnset { skipped, .. } => Ok(skipped),
			other => Err(unexpected(&other))
		}
	}

	/// Why there's no code at `line` of `file`, or after it
	fn nowhere(&mut self, file: &str, line: u32) -> Result<Failure> {
		let lines = match self.request(Request::LinesWithCode {
			file: file.to_owned(),
			start: 0,
			end: u32::MAX
		})? {
			Response::LinesWithCode { lines } => lines,
			other => return Err(unexpected(&other))
		};
		Ok(match lines.last() {
			None => format!("couldn't break at {}:{}: the world has no code from a file called {}", file, line, file),
			Some(last) => format!("couldn't break at {}:{}: {} has no code after line {}", file, line, file, last)
		}
		.into())
	}
}

/// A breakpoint that was set
struct Bound {
	/// The place it was asked for, as it was given
	place: String,
	/// What it was set with
	asked: InstructionRef,
	/// Where it went: its proc, as the proc names itself, and the instruction
	at: InstructionRef,
	file: Option<String>,
	line: Option<u32>,
	/// The line that was asked for, for one given as FILE:LINE
	asked_line: Option<u32>
}

impl Bound {
	/// Its proc and line, as a frame there is shown
	fn spot(&self) -> String {
		spot(&self.at.proc, false, self.file.as_deref(), self.line)
	}

	/// The same, for `wait` to say where it will pause
	fn describe(&self) -> String {
		let mut text = format!("breakpoint at {}", self.spot());
		if let (Some(asked), Some(line)) = (self.asked_line, self.line) {
			if asked != line {
				text += &format!(" (line {} has no code: this is the next that has)", asked);
			}
		}
		text
	}
}

fn place(frame: &StackFrame) -> String {
	spot(&frame.instruction.proc, frame.old_code, frame.file.as_deref(), frame.line)
}

/// A proc and where in it
fn spot(proc: &ProcRef, old_code: bool, file: Option<&str>, line: Option<u32>) -> String {
	let name = match proc.override_id {
		0 => proc.path.clone(),
		n => format!("{}#{}", proc.path, n)
	};
	let name = if old_code { format!("{} (old code)", name) } else { name };
	match (file, line) {
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
	use super::{command, describe, expression, given_text, one_proc, overview, parse, proc_ref, Failure, Flags, COMMANDS};

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
		for name in ["dap", "list", "version", "help", "status", "wait", "trace", "eval", "launch", "stop", "reboot", "proc", "dis"] {
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

	/// A file of its own for a test, which goes when it's dropped
	struct Scratch(std::path::PathBuf);

	impl Scratch {
		fn new(name: &str, text: &str) -> Self {
			let path = std::env::temp_dir().join(format!("auxtools-cli-{}-{}", std::process::id(), name));
			std::fs::write(&path, text).unwrap();
			Self(path)
		}

		fn path(&self) -> &str {
			self.0.to_str().unwrap()
		}
	}

	impl Drop for Scratch {
		fn drop(&mut self) {
			let _ = std::fs::remove_file(&self.0);
		}
	}

	#[test]
	fn an_option_reads_its_text_from_a_file_after_an_at() {
		let file = Scratch::new("given", "\"a b\" == /datum/x\r\n");
		assert_eq!(given_text("x > 1").ok().unwrap(), "x > 1");
		assert_eq!(given_text(&format!("@{}", file.path())).ok().unwrap(), "\"a b\" == /datum/x");
		assert_eq!(given_text("@@home").ok().unwrap(), "@home");
		let Err(Failure(1, why)) = given_text("@/no/such/file") else {
			panic!("a file that isn't there was read");
		};
		assert!(why.starts_with("couldn't read /no/such/file: "), "{}", why);
	}

	#[test]
	fn evals_expression_is_its_words_a_file_or_stdin() {
		let none = || -> super::Result<String> { panic!("stdin was read") };
		assert_eq!(expression(&args(&["6", "*", "7"]), None, none).ok().unwrap(), "6 * 7");
		let file = Scratch::new("expression", "list(1, \"two\")\n\n");
		assert_eq!(expression(&[], Some(file.path()), none).ok().unwrap(), "list(1, \"two\")");
		assert_eq!(expression(&args(&["-"]), None, || Ok("1 + 1".to_owned())).ok().unwrap(), "1 + 1");
		// A negative number is an expression, not stdin
		assert_eq!(expression(&args(&["-", "1"]), None, none).ok().unwrap(), "- 1");
		for (words, file, said) in [
			(&[][..], None, "eval what?"),
			(&["1"][..], Some(file.path()), "eval takes one expression: its words, -f FILE or -")
		] {
			let Err(Failure(1, why)) = expression(&args(words), file, none) else {
				panic!("{:?} {:?} was an expression", words, file);
			};
			assert_eq!(why, said);
		}
		let Err(Failure(1, why)) = expression(&args(&["-"]), None, || Ok("\n".to_owned())) else {
			panic!("nothing was an expression");
		};
		assert_eq!(why, "stdin had no expression on it");
	}

	#[test]
	fn a_proc_is_its_path_and_which_definition() {
		let proc = proc_ref("/mob/proc/attack#2").ok().unwrap();
		assert_eq!((proc.path.as_str(), proc.override_id), ("/mob/proc/attack", 2));
		let proc = proc_ref("/value").ok().unwrap();
		assert_eq!((proc.path.as_str(), proc.override_id), ("/value", 0));
		for wrong in ["value", "/value#", "/value#two", "#1"] {
			assert!(proc_ref(wrong).is_err(), "{}", wrong);
		}
		assert!(one_proc("dis", &args(&["/a", "/b"])).is_err() && one_proc("dis", &[]).is_err());
	}

	#[test]
	fn flags_that_are_wrong_are_errors() {
		assert!(Flags::parse(&args(&["--sideways"]), &["--frame"], &["--all"]).is_err());
		assert!(Flags::parse(&args(&["--frame"]), &["--frame"], &[]).is_err());
		let flags = Flags::parse(&args(&["--frame", "top"]), &["--frame"], &[]).ok().unwrap();
		assert!(flags.number::<u32>("--frame").is_err());
	}
}
