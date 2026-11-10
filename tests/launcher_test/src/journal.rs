//! The world's record of what happened in it, read from the command line:
//! `log`, and the waits that come from it

use std::{
	io::{BufRead, BufReader, Read},
	path::Path,
	process::{Child, Command, Stdio},
	sync::mpsc,
	time::{Duration, Instant}
};

use crate::client::{launcher_command, launcher_path};

const TIMEOUT: Duration = Duration::from_secs(60);

/// Runs a command against the world in process `pid`, and checks its exit code
pub fn run(pid: u64, args: &[&str], code: i32) -> String {
	let pid = pid.to_string();
	let args: Vec<&str> = ["--pid", pid.as_str()].iter().chain(args).copied().collect();
	let (got, printed) = launcher_command(&args);
	assert_eq!(got, code, "`{}` gave {}, not {}: {}", args.join(" "), got, code, printed);
	printed
}

/// A command left running, whose lines are read as they come
pub struct Background {
	child: Child,
	lines: mpsc::Receiver<String>,
	/// What it printed, so far
	pub printed: String
}

impl Background {
	pub fn start(pid: u64, args: &[&str]) -> Self {
		let mut child = Command::new(launcher_path())
			.arg("--pid")
			.arg(pid.to_string())
			.args(args)
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.unwrap();
		let (sender, lines) = mpsc::channel();
		let stdout = child.stdout.take().unwrap();
		let errors = sender.clone();
		std::thread::spawn(move || {
			for line in BufReader::new(stdout).lines().map_while(Result::ok) {
				let _ = sender.send(line);
			}
		});
		let mut stderr = child.stderr.take().unwrap();
		std::thread::spawn(move || {
			let mut text = String::new();
			let _ = stderr.read_to_string(&mut text);
			for line in text.lines() {
				let _ = errors.send(line.to_owned());
			}
		});
		Self {
			child,
			lines,
			printed: String::new()
		}
	}

	/// Waits for a line that has `text` in it
	pub fn line_with(&mut self, text: &str) -> String {
		let deadline = Instant::now() + TIMEOUT;
		loop {
			let line = self
				.lines
				.recv_timeout(deadline.saturating_duration_since(Instant::now()))
				.unwrap_or_else(|_| panic!("no line with {:?} came; it printed {}", text, self.printed));
			self.printed += &line;
			self.printed += "\n";
			if line.contains(text) {
				return line;
			}
		}
	}

	/// Waits for it to exit: its exit code and everything it printed
	pub fn finish(mut self) -> (i32, String) {
		let deadline = Instant::now() + TIMEOUT;
		let code = loop {
			if let Some(status) = self.child.try_wait().unwrap() {
				break status.code().unwrap_or(-1);
			}
			assert!(Instant::now() < deadline, "it didn't exit; it printed {}", self.printed);
			std::thread::sleep(Duration::from_millis(50));
		};
		// What's left to read is read by now, or soon
		while let Ok(line) = self.lines.recv_timeout(Duration::from_secs(2)) {
			self.printed += &line;
			self.printed += "\n";
		}
		(code, self.printed)
	}

	pub fn stop(mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

/// The number of the newest entry in the record
pub fn newest(pid: u64) -> u64 {
	let printed = run(pid, &["log", "--last", "1", "--json"], 0);
	printed
		.lines()
		.last()
		.map_or(0, |line| serde_json::from_str::<serde_json::Value>(line).unwrap()["number"].as_u64().unwrap())
}

/// Runtime errors go in the record, with their stacks, while the world runs
/// on: `log --runtimes` prints them, and `--follow` as they come
pub fn runtimes(pid: u64, dir: &Path) {
	let before = newest(pid);
	let since = before.to_string();
	std::fs::write(dir.join("fail"), "yes").unwrap();
	let deadline = Instant::now() + TIMEOUT;
	let printed = loop {
		let printed = run(pid, &["log", "--runtimes", "--since", &since], 0);
		if !printed.is_empty() {
			break printed;
		}
		assert!(Instant::now() < deadline, "the runtime error didn't go in the record");
		std::thread::sleep(Duration::from_millis(200));
	};
	let mut lines = printed.lines();
	let first = lines.next().unwrap();
	assert!(
		first.starts_with('#') && first.ends_with("runtime error: cannot read from list"),
		"log --runtimes said {}",
		printed
	);
	assert!(lines.next().is_some_and(|line| line.starts_with("  /fail  attach_host.dm:")), "log --runtimes said {}", printed);
	assert_eq!(run(pid, &["status"], 0).lines().next(), Some("running"));
	println!("ok: a runtime error is in log --runtimes with its stack, and the world runs on");

	let printed = run(pid, &["log", "--since", &since, "--json"], 0);
	let entry: serde_json::Value = serde_json::from_str(printed.lines().find(|line| line.contains("\"runtime\"")).unwrap()).unwrap();
	assert_eq!((&entry["kind"], &entry["caught"], &entry["proc"]), (&"runtime".into(), &false.into(), &"/fail".into()), "{}", entry);
	assert!(entry["number"].as_u64().unwrap() > before && entry["line"].as_u64().is_some(), "{}", entry);
	println!("ok: log --json prints an entry a line, as JSON");

	// It starts with what's there already, after --since
	let mut follow = Background::start(pid, &["log", "--runtimes", "--follow", "--json", "--since", &before.to_string()]);
	follow.line_with("cannot read from list");
	let now = newest(pid);
	std::fs::write(dir.join("fail"), "yes").unwrap();
	let line = follow.line_with("cannot read from list");
	let entry: serde_json::Value = serde_json::from_str(&line).unwrap();
	assert!(entry["number"].as_u64().unwrap() > now && entry["stack"].is_array(), "log --follow printed {}", follow.printed);
	follow.stop();
	println!("ok: log --runtimes --follow prints runtime errors as they come");
}
