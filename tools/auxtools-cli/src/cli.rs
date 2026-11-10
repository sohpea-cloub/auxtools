//! One command to a running world, from the command line: connect, do it, print
//! what came of it and leave. For scripts and agents, which can run `wait` in
//! the background to hear when the world pauses at a breakpoint or a runtime.
//!
//! A command that leaves the world paused (`pause`, `next`, `wait` and the
//! like) asks it to stay paused after the connection goes, until a `continue`.

use std::{
	io,
	sync::mpsc::{self, Receiver, RecvTimeoutError},
	time::{Duration, Instant}
};

use auxtools_cli::{
	local::{self, Shared},
	server_types::*
};

pub const USAGE: &str = "\
auxtools [--pid PID] COMMAND              one command to a running world, which
                                          stays paused after the ones that pause
                                          it, until `continue`. Without --pid,
                                          the only world running the debug
                                          library.
  status                                  paused or running, and where
  stack [--all]                           the paused stack, or every stack
  vars [--frame N] [--globals]            a frame's arguments and locals
  eval EXPR [--frame N]                   an expression, in a frame when paused
  pause                                   pauses it where it is
  continue                                lets it carry on
  next | step | out                       steps over, into or out of a call
  wait [--break FILE:LINE | --break /proc/path[#N]]... [--if COND]
       [--log MESSAGE] [--runtimes] [--caught] [--timeout SECS] [--no-hold]
                                          waits for it to pause, at one of these
                                          breakpoints, a runtime error (one that
                                          nothing catches, or with --caught any),
                                          or for any other client. Prints why and
                                          where. With --log the breakpoints don't
                                          pause: each hit prints MESSAGE, with the
                                          values of the {expressions} in it.
Exit codes: 0 done (for `wait`, it paused), 1 failed, 2 `wait` timed out,
3 the world went away (it ended or rebooted).";

/// What went wrong, and the exit code it gives
struct Failure(i32, String);

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

type Result<T> = std::result::Result<T, Failure>;

const HUNG_UP: i32 = 3;
const TIMED_OUT: i32 = 2;

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

/// The commands, without the world
pub fn is_command(name: &str) -> bool {
	matches!(
		name,
		"--pid" | "status" | "stack" | "vars" | "eval" | "pause" | "continue" | "next" | "step" | "out" | "wait"
	)
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
				return Err(format!("there's no {} here\n\n{}", arg, USAGE).into());
			} else {
				flags.rest.push(arg.clone());
			}
		}
		Ok(flags)
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

fn execute(pid: Option<u32>, command: &[String]) -> Result<()> {
	let Some((name, args)) = command.split_first() else {
		return Err(USAGE.into());
	};
	let pid = match pid {
		Some(pid) => pid,
		None => only_world()?
	};
	let mut world = World::connect(pid)?;
	let result = match name.as_str() {
		"status" => world.status(),
		"stack" => {
			let flags = Flags::parse(args, &[], &["--all"])?;
			world.stack(flags.has("--all"))
		}
		"vars" => {
			let flags = Flags::parse(args, &["--frame"], &["--globals"])?;
			world.vars(flags.number("--frame")?.unwrap_or(0), flags.has("--globals"))
		}
		"eval" => {
			let flags = Flags::parse(args, &["--frame"], &[])?;
			if flags.rest.is_empty() {
				return Err("eval what?".into());
			}
			world.eval(&flags.rest.join(" "), flags.number("--frame")?)
		}
		"pause" => world.pause(),
		"continue" => world.resume(ContinueKind::Continue),
		"next" => world.resume(ContinueKind::StepOver { stack_id: 0 }),
		"step" => world.resume(ContinueKind::StepInto { stack_id: 0 }),
		"out" => world.resume(ContinueKind::StepOut { stack_id: 0 }),
		"wait" => {
			let flags = Flags::parse(args, &["--break", "--if", "--log", "--timeout"], &["--runtimes", "--caught", "--no-hold"])?;
			if !flags.rest.is_empty() {
				return Err(format!("wait doesn't take {}", flags.rest.join(" ")).into());
			}
			world.wait(Wait {
				breaks: flags.values("--break").map(str::to_owned).collect(),
				condition: flags.value("--if").map(str::to_owned),
				log: flags.value("--log").map(LogPart::parse),
				runtimes: flags.has("--runtimes") || flags.has("--caught"),
				caught: flags.has("--caught"),
				timeout: flags.number::<f64>("--timeout")?.map(Duration::from_secs_f64),
				hold: !flags.has("--no-hold")
			})
		}
		_ => Err(format!("there's no command {}\n\n{}", name, USAGE).into())
	};
	world.leave();
	result
}

fn only_world() -> Result<u32> {
	let worlds = local::worlds();
	match worlds.as_slice() {
		[world] => Ok(world.pid),
		[] => Err("no world is running the debug library".into()),
		_ => Err(format!(
			"more than one world is running the debug library, so say which with --pid:\n{}",
			worlds
				.iter()
				.map(|w| format!(
					"  {} {}",
					w.pid,
					w.dmb.as_ref().map_or(w.program.clone(), |dmb| dmb.display().to_string())
				))
				.collect::<Vec<_>>()
				.join("\n")
		)
		.into())
	}
}

struct Wait {
	breaks: Vec<String>,
	condition: Option<String>,
	/// A message the breakpoints print at each hit, in place of pausing
	log: Option<Vec<LogPart>>,
	runtimes: bool,
	/// Those inside a `try` too
	caught: bool,
	timeout: Option<Duration>,
	hold: bool
}

struct World {
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
			Response::Status { paused } => Ok(paused),
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
			BreakpointReason::Runtime { message, caught: true } => format!("on a caught runtime error: {}", message)
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
		let stacks = match self.request(Request::Stacks)? {
			Response::Stacks { stacks } => stacks,
			other => return Err(unexpected(&other))
		};
		for stack in stacks.iter().filter(|s| all || s.id == 0) {
			if all {
				println!("stack {}: {}", stack.id, stack.name);
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
				let Response::BreakpointHit { reason } = self.events.remove(i) else {
					unreachable!()
				};
				break reason;
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
		if let Some(reason) = self.paused()? {
			if wait.hold {
				self.request(Request::HoldPause)?;
			}
			println!("already paused");
			return self.print_pause(&reason);
		}
		for place in &wait.breaks {
			self.set_breakpoint(place, wait.condition.clone(), wait.log.clone())?;
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
	use super::{is_command, parse, Flags};

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
	fn commands_are_told_from_the_adapter_s_own_arguments() {
		for command in ["--pid", "status", "stack", "vars", "eval", "pause", "continue", "next", "step", "out", "wait"] {
			assert!(is_command(command), "{}", command);
		}
		for other in ["--list", "--version", "--build", "run", ""] {
			assert!(!is_command(other), "{}", other);
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
