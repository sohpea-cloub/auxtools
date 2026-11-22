use std::{
	ffi::OsStr,
	io::{BufRead, BufReader, Read, Write},
	process::{Child, Command, Stdio},
	sync::mpsc,
	time::{Duration, Instant}
};

use serde_json::Value;

use crate::paths::launcher_path;

pub const TIMEOUT: Duration = Duration::from_secs(20);

pub fn auxtools<I, S>(args: I) -> Command
where
	I: IntoIterator<Item = S>,
	S: AsRef<OsStr>
{
	let mut command = Command::new(launcher_path());
	command.args(args);
	command
}

#[derive(Debug)]
pub struct Ran {
	pub code: i32,
	pub out: String,
	pub err: String
}

impl Ran {
	pub fn printed(&self) -> String {
		format!("{}{}", self.out, self.err)
	}
}

pub fn ran(command: &mut Command, input: Option<&str>) -> Ran {
	let out = match input {
		Some(input) => {
			let mut child = command
				.stdin(Stdio::piped())
				.stdout(Stdio::piped())
				.stderr(Stdio::piped())
				.spawn()
				.unwrap();
			child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
			child.wait_with_output().unwrap()
		}
		None => command.output().unwrap()
	};
	Ran {
		code: out.status.code().unwrap_or(-1),
		out: String::from_utf8_lossy(&out.stdout).into_owned(),
		err: String::from_utf8_lossy(&out.stderr).into_owned()
	}
}

pub fn launcher_command(args: &[&str]) -> (i32, String) {
	let ran = ran(&mut auxtools(args), None);
	(ran.code, ran.printed())
}

pub struct Cli(String);

impl Cli {
	pub fn new(pid: impl ToString) -> Self {
		Self(pid.to_string())
	}

	pub fn command(&self, args: &[&str]) -> Command {
		let mut command = auxtools(["--pid", self.0.as_str()]);
		command.args(args);
		command
	}

	pub fn ran(&self, args: &[&str], input: Option<&str>) -> Ran {
		ran(&mut self.command(args), input)
	}

	#[track_caller]
	pub fn run(&self, args: &[&str], code: i32) -> String {
		self.run_apart(args, code).printed()
	}

	#[track_caller]
	pub fn run_apart(&self, args: &[&str], code: i32) -> Ran {
		let ran = self.ran(args, None);
		assert_eq!(
			ran.code,
			code,
			"`--pid {} {}` gave {}, not {}: {}",
			self.0,
			args.join(" "),
			ran.code,
			code,
			ran.printed()
		);
		ran
	}

	pub fn start(&self, args: &[&str]) -> Background {
		Background::spawn(self.command(args))
	}
}

pub struct Background {
	child: Child,
	lines: mpsc::Receiver<String>,
	errors: mpsc::Receiver<String>,
	pub printed: String,
	said: String
}

fn lines_of(stream: impl Read + Send + 'static) -> mpsc::Receiver<String> {
	let (sender, lines) = mpsc::channel();
	std::thread::spawn(move || {
		for line in BufReader::new(stream).lines().map_while(Result::ok) {
			if sender.send(line).is_err() {
				return;
			}
		}
	});
	lines
}

fn next_line(lines: &mpsc::Receiver<String>, read: &mut String, deadline: Instant) -> Option<String> {
	let line = lines.recv_timeout(deadline.saturating_duration_since(Instant::now())).ok()?;
	*read += &line;
	*read += "\n";
	Some(line)
}

impl Background {
	pub fn start(pid: u64, args: &[&str]) -> Self {
		Cli::new(pid).start(args)
	}

	pub fn spawn(mut command: Command) -> Self {
		let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
		Self {
			lines: lines_of(child.stdout.take().unwrap()),
			errors: lines_of(child.stderr.take().unwrap()),
			child,
			printed: String::new(),
			said: String::new()
		}
	}

	#[track_caller]
	pub fn line_with(&mut self, text: &str) -> String {
		let deadline = Instant::now() + TIMEOUT;
		loop {
			match next_line(&self.lines, &mut self.printed, deadline) {
				Some(line) if line.contains(text) => return line,
				Some(_) => {}
				None => {
					self.read_errors();
					panic!("no line with {:?} came; it printed {}{}", text, self.printed, self.said)
				}
			}
		}
	}

	#[track_caller]
	pub fn error_with(&mut self, text: &str) -> String {
		let deadline = Instant::now() + TIMEOUT;
		loop {
			match next_line(&self.errors, &mut self.said, deadline) {
				Some(line) if line.contains(text) => return line,
				Some(_) => {}
				None => panic!("no line with {:?} came on stderr; it printed {}{}", text, self.printed, self.said)
			}
		}
	}

	fn read_errors(&mut self) {
		while next_line(&self.errors, &mut self.said, Instant::now()).is_some() {}
	}

	#[track_caller]
	pub fn finish(self) -> (i32, String) {
		let (code, printed, said) = self.finish_apart();
		(code, printed + &said)
	}

	#[track_caller]
	pub fn finish_apart(mut self) -> (i32, String, String) {
		let deadline = Instant::now() + TIMEOUT;
		let code = loop {
			if let Some(status) = self.child.try_wait().unwrap() {
				break status.code().unwrap_or(-1);
			}
			if Instant::now() >= deadline {
				self.read_errors();
				panic!("timed out waiting for process exit; output: {}{}", self.printed, self.said);
			}
			std::thread::sleep(Duration::from_millis(50));
		};
		while next_line(&self.lines, &mut self.printed, deadline).is_some() {}
		while next_line(&self.errors, &mut self.said, deadline).is_some() {}
		(code, self.printed, self.said)
	}

	#[cfg(unix)]
	pub fn id(&self) -> u32 {
		self.child.id()
	}

	pub fn stop(mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

pub fn listed_worlds() -> Vec<Value> {
	let list = auxtools(["list", "--json"]).output().unwrap();
	let worlds: Value = serde_json::from_slice(&list.stdout).unwrap();
	let plain = |path: &str| path.replace('\\', "/").to_lowercase();
	let here = plain(&std::env::current_dir().unwrap().to_string_lossy());
	let made = format!("-{}/", std::process::id());
	worlds
		.as_array()
		.unwrap()
		.iter()
		.filter(|world| {
			let dmb = plain(world["dmb"].as_str().unwrap_or_default());
			dmb.starts_with(&here) || dmb.contains(&made)
		})
		.cloned()
		.collect()
}

#[track_caller]
pub fn listed_world(dmb: &str) -> u64 {
	let worlds = listed_worlds();
	match worlds.iter().find(|world| world["dmb"].as_str().is_some_and(|path| path.ends_with(dmb))) {
		Some(world) => world["pid"].as_u64().unwrap(),
		None => panic!("--list didn't list the world: {:?}", worlds)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn printing(lines: &[(&str, bool)], code: i32) -> Command {
		let echo = |(text, to_stderr): &(&str, bool)| {
			if *to_stderr {
				format!("echo {}>&2", text)
			} else {
				format!("echo {}", text)
			}
		};
		let (shell, flag, then) = if cfg!(windows) { ("cmd", "/C", "&") } else { ("sh", "-c", ";") };
		let mut script: Vec<String> = lines.iter().map(echo).collect();
		script.push(format!("exit {}", code));
		let mut command = Command::new(shell);
		command.arg(flag).arg(script.join(then));
		command
	}

	#[test]
	fn a_command_that_ran_gives_its_code_and_both_streams() {
		let ran = ran(&mut printing(&[("said", true), ("printed", false)], 4), None);
		assert_eq!((ran.code, ran.out.trim(), ran.err.trim()), (4, "printed", "said"));
		assert_eq!(ran.printed().split_whitespace().collect::<Vec<_>>(), ["printed", "said"]);
	}

	#[test]
	fn finish_gives_stdout_then_stderr() {
		let running = Background::spawn(printing(&[("one", true), ("two", false), ("three", true), ("four", false)], 3));
		assert_eq!(running.finish(), (3, "two\nfour\none\nthree\n".to_owned()));
	}

	#[test]
	fn lines_are_taken_from_each_stream_in_its_order() {
		let mut running = Background::spawn(printing(&[("first", false), ("ready", true), ("second", false), ("last", true)], 0));
		assert_eq!(running.error_with("rea"), "ready");
		assert_eq!(running.line_with("second"), "second");
		assert_eq!(running.printed, "first\nsecond\n");
		assert_eq!(running.finish_apart(), (0, "first\nsecond\n".to_owned(), "ready\nlast\n".to_owned()));
	}
}
