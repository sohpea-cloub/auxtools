use std::{
	ffi::OsStr,
	io::{BufRead, BufReader, Read, Write},
	process::{Child, Command, Stdio},
	sync::mpsc::{self, RecvTimeoutError},
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

pub const COMMAND_LIMIT: Duration = Duration::from_secs(60);

fn chunks_of(mut stream: impl Read + Send + 'static) -> mpsc::Receiver<Vec<u8>> {
	let (sender, chunks) = mpsc::channel();
	std::thread::spawn(move || {
		let mut chunk = [0; 4096];
		while let Ok(read @ 1..) = stream.read(&mut chunk) {
			if sender.send(chunk[..read].to_vec()).is_err() {
				return;
			}
		}
	});
	chunks
}

fn text_of(chunks: &mpsc::Receiver<Vec<u8>>, deadline: Instant) -> (String, bool) {
	let mut bytes = Vec::new();
	let closed = loop {
		match chunks.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
			Ok(chunk) => bytes.extend(chunk),
			Err(RecvTimeoutError::Timeout) => break false,
			Err(RecvTimeoutError::Disconnected) => break true
		}
	};
	(String::from_utf8_lossy(&bytes).into_owned(), closed)
}

fn command_line(command: &Command) -> String {
	let args: Vec<_> = command.get_args().map(|arg| arg.to_string_lossy()).collect();
	format!("{} {}", command.get_program().to_string_lossy(), args.join(" "))
}

#[track_caller]
pub fn ran(command: &mut Command, input: Option<&str>) -> Ran {
	ran_within(command, input, COMMAND_LIMIT)
}

#[track_caller]
fn ran_within(command: &mut Command, input: Option<&str>, limit: Duration) -> Ran {
	let stdin = if input.is_some() { Stdio::piped() } else { Stdio::null() };
	let mut child = command.stdin(stdin).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
	if let Some(input) = input {
		let (mut stdin, input) = (child.stdin.take().unwrap(), input.to_owned());
		std::thread::spawn(move || stdin.write_all(input.as_bytes()));
	}
	let (out, err) = (chunks_of(child.stdout.take().unwrap()), chunks_of(child.stderr.take().unwrap()));
	let deadline = Instant::now() + limit;
	let status = loop {
		if let Some(status) = child.try_wait().unwrap() {
			break Some(status);
		}
		if Instant::now() >= deadline {
			let _ = child.kill();
			let _ = child.wait();
			break None;
		}
		std::thread::sleep(Duration::from_millis(20));
	};
	let Some(status) = status else {
		let read_until = Instant::now() + Duration::from_secs(2);
		panic!(
			"`{}` was stopped after {:?}; it printed {}{}",
			command_line(command),
			limit,
			text_of(&out, read_until).0,
			text_of(&err, read_until).0
		)
	};
	let ((out, out_closed), (err, err_closed)) = (text_of(&out, deadline), text_of(&err, deadline));
	if !(out_closed && err_closed) {
		panic!(
			"`{}` exited with {}, but a process it started still held its output open after {:?}; it printed {}{}",
			command_line(command),
			status,
			limit,
			out,
			err
		)
	}
	Ran {
		code: status.code().unwrap_or(-1),
		out,
		err
	}
}

#[track_caller]
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

	#[track_caller]
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
	stdout_lines: mpsc::Receiver<String>,
	stderr_lines: mpsc::Receiver<String>,
	pub printed: String,
	printed_on_stderr: String
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
			stdout_lines: lines_of(child.stdout.take().unwrap()),
			stderr_lines: lines_of(child.stderr.take().unwrap()),
			child,
			printed: String::new(),
			printed_on_stderr: String::new()
		}
	}

	#[track_caller]
	pub fn line_with(&mut self, text: &str) -> String {
		let deadline = Instant::now() + TIMEOUT;
		loop {
			match next_line(&self.stdout_lines, &mut self.printed, deadline) {
				Some(line) if line.contains(text) => return line,
				Some(_) => {}
				None => {
					self.read_errors();
					panic!("no line with {:?} came; it printed {}{}", text, self.printed, self.printed_on_stderr)
				}
			}
		}
	}

	#[track_caller]
	pub fn error_with(&mut self, text: &str) -> String {
		let deadline = Instant::now() + TIMEOUT;
		loop {
			match next_line(&self.stderr_lines, &mut self.printed_on_stderr, deadline) {
				Some(line) if line.contains(text) => return line,
				Some(_) => {}
				None => panic!(
					"no line with {:?} came on stderr; it printed {}{}",
					text, self.printed, self.printed_on_stderr
				)
			}
		}
	}

	fn read_errors(&mut self) {
		while next_line(&self.stderr_lines, &mut self.printed_on_stderr, Instant::now()).is_some() {}
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
				panic!("timed out waiting for process exit; output: {}{}", self.printed, self.printed_on_stderr);
			}
			std::thread::sleep(Duration::from_millis(50));
		};
		while next_line(&self.stdout_lines, &mut self.printed, deadline).is_some() {}
		while next_line(&self.stderr_lines, &mut self.printed_on_stderr, deadline).is_some() {}
		(code, self.printed, self.printed_on_stderr)
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
	let list = ran(&mut auxtools(["list", "--json"]), None);
	let worlds: Value = serde_json::from_str(&list.out).unwrap();
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
	fn a_command_past_its_limit_is_stopped_and_named() {
		let mut command = if cfg!(windows) {
			let mut command = Command::new("cmd");
			command.args(["/C", "echo started&& ping -n 30 127.0.0.1 >NUL"]);
			command
		} else {
			let mut command = Command::new("sh");
			command.args(["-c", "echo started; exec sleep 30"]);
			command
		};
		let started = Instant::now();
		let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ran_within(&mut command, None, Duration::from_secs(1)))).unwrap_err();
		let said = failed.downcast_ref::<String>().unwrap();
		assert!(started.elapsed() < Duration::from_secs(10), "the command ran for {:?}", started.elapsed());
		assert!(
			said.contains("was stopped after 1s") && said.contains("started"),
			"the failure said {}",
			said
		);
	}

	#[cfg(unix)]
	#[test]
	fn a_command_whose_output_outlives_it_is_stopped_at_the_limit() {
		let mut command = Command::new("sh");
		command.args(["-c", "sleep 30 & echo $!"]);
		let started = Instant::now();
		let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ran_within(&mut command, None, Duration::from_secs(1))));
		let said = match &outcome {
			Ok(ran) => ran.printed(),
			Err(failed) => failed.downcast_ref::<String>().unwrap().clone()
		};
		let holder = said.rsplit_once("printed ").map_or("", |(_, printed)| printed.trim());
		let _ = Command::new("kill").arg(holder).status();
		assert!(started.elapsed() < Duration::from_secs(10), "the command ran for {:?}", started.elapsed());
		assert!(
			outcome.is_err() && said.contains("still held its output open after 1s"),
			"the outcome was {}",
			said
		);
	}

	#[test]
	fn finish_gives_stdout_then_stderr() {
		let running = Background::spawn(printing(&[("one", true), ("two", false), ("three", true), ("four", false)], 3));
		assert_eq!(running.finish(), (3, "two\nfour\none\nthree\n".to_owned()));
	}

	#[test]
	fn lines_are_taken_from_each_stream_in_its_order() {
		let mut running = Background::spawn(printing(&[("first", false), ("ready", true), ("second", false), ("last", true)], 0));
		assert_eq!(running.error_with("rea"), "ready", "a line of stderr is found whatever stdout has");
		assert_eq!(running.line_with("second"), "second");
		assert_eq!(running.printed, "first\nsecond\n", "the lines passed over stay in what it printed");
		assert_eq!(running.finish_apart(), (0, "first\nsecond\n".to_owned(), "ready\nlast\n".to_owned()));
	}
}
