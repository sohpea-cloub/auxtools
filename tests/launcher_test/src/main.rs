//! Drives the launcher the way VS Code would, in the Debug Adapter Protocol,
//! against a world with none of auxtools' procs in it: it has to load the
//! debugger itself, stop at breakpoints, pick up again after the world
//! reboots, and go away when the world ends. Then a world launched without
//! debugging is found with `--list`, two more launchers attach to it at once,
//! and the first hot reloads it.
//!
//! Build the launcher and the debug library first:
//! `cargo build -p auxtools-cli -p debug_server -p launcher_test`, then run this
//! from the repository's root with `BYOND_PATH` set. `LAUNCHER_TEST_SERVER`
//! picks the program on Windows (`dd` unless set: `dreamseeker` and
//! `dreamdaemon` need a display). `LAUNCHER_TEST_ARGS` replaces the program's
//! arguments (Windows' DreamDaemon asks before it runs a world `-trusted`).

#[allow(dead_code)]
#[path = "../../test_runner/src/paths.rs"]
mod paths;

use std::{
	collections::VecDeque,
	io::{BufRead, BufReader, Read, Write},
	path::{Path, PathBuf},
	process::{Child, ChildStdin, Command, Stdio},
	sync::mpsc,
	time::{Duration, Instant}
};

use paths::ByondCommand;
use serde_json::{json, Value};

const TIMEOUT: Duration = Duration::from_secs(90);
/// The line `target` doubles its argument on
const TARGET_LINE: u64 = 28;

fn next_to_us(name: &str) -> PathBuf {
	let mut path = std::env::current_exe().unwrap();
	path.pop();
	path.push(name);
	assert!(path.is_file(), "couldn't find {}; build it first", path.display());
	path
}

fn host_file(name: &str) -> PathBuf {
	std::env::current_dir().unwrap().join("tests/launcher_test_host").join(name)
}

struct Launcher {
	child: Child,
	stdin: ChildStdin,
	messages: mpsc::Receiver<Value>,
	seq: i64,
	/// Events that came while waiting for something else
	events: VecDeque<Value>,
	/// Everything the world printed
	output: String
}

impl Launcher {
	fn start() -> Self {
		let exe = if cfg!(windows) { "auxtools.exe" } else { "auxtools" };
		let mut child = Command::new(next_to_us(exe))
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.spawn()
			.unwrap();
		let stdin = child.stdin.take().unwrap();
		let mut stdout = BufReader::new(child.stdout.take().unwrap());

		let (sender, messages) = mpsc::channel();
		std::thread::spawn(move || loop {
			let mut length = 0;
			loop {
				let mut line = String::new();
				if stdout.read_line(&mut line).unwrap_or(0) == 0 {
					return;
				}
				let line = line.trim();
				if line.is_empty() {
					break;
				}
				if let Some(value) = line.strip_prefix("Content-Length:") {
					length = value.trim().parse().unwrap();
				}
			}
			let mut body = vec![0; length];
			if stdout.read_exact(&mut body).is_err() {
				return;
			}
			if sender.send(serde_json::from_slice(&body).unwrap()).is_err() {
				return;
			}
		});

		Self {
			child,
			stdin,
			messages,
			seq: 1,
			events: VecDeque::new(),
			output: String::new()
		}
	}

	fn next_message(&mut self, deadline: Instant) -> Value {
		let message = self
			.messages
			.recv_timeout(deadline.saturating_duration_since(Instant::now()))
			.unwrap_or_else(|_| panic!("the launcher went quiet; the world said:\n{}", self.output));
		if message["type"] == "event" && message["event"] == "output" {
			let output = message["body"]["output"].as_str().unwrap_or_default();
			print!("  | {}", output);
			self.output.push_str(output);
		}
		message
	}

	fn send(&mut self, command: &str, arguments: Value) -> i64 {
		let seq = self.seq;
		self.seq += 1;
		let message = json!({ "seq": seq, "type": "request", "command": command, "arguments": arguments }).to_string();
		write!(self.stdin, "Content-Length: {}\r\n\r\n{}", message.len(), message).unwrap();
		self.stdin.flush().unwrap();
		seq
	}

	/// Sends a request and returns its response's body, which has to be a
	/// success
	fn request(&mut self, command: &str, arguments: Value) -> Value {
		let response = self.try_request(command, arguments);
		assert_eq!(response["success"], true, "{} failed: {}", command, response);
		response["body"].clone()
	}

	fn try_request(&mut self, command: &str, arguments: Value) -> Value {
		let seq = self.send(command, arguments);
		let deadline = Instant::now() + TIMEOUT;
		loop {
			let message = self.next_message(deadline);
			if message["type"] == "response" && message["request_seq"] == seq {
				return message;
			}
			if message["type"] == "event" {
				self.events.push_back(message);
			}
		}
	}

	/// Waits for an event, skipping others
	fn event(&mut self, name: &str) -> Value {
		if let Some(at) = self.events.iter().position(|e| e["event"] == name) {
			return self.events.remove(at).unwrap();
		}
		let deadline = Instant::now() + TIMEOUT;
		loop {
			let message = self.next_message(deadline);
			if message["type"] == "event" {
				if message["event"] == name {
					return message;
				}
				self.events.push_back(message);
			}
		}
	}

	/// Waits for the world to stop, and checks why
	fn stopped(&mut self, reason: &str) -> Value {
		let event = self.event("stopped");
		assert_eq!(event["body"]["reason"], reason, "stopped for the wrong reason: {}", event);
		event
	}

	fn top_frame(&mut self) -> Value {
		let threads = self.request("threads", json!({}));
		let names: Vec<&str> = threads["threads"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
		assert!(
			!names.iter().any(|name| name.contains("auxtools_keepalive")),
			"the debugger's own loop is a thread: {:?}",
			names
		);
		let trace = self.request("stackTrace", json!({ "threadId": 0 }));
		trace["stackFrames"][0].clone()
	}

	fn evaluate(&mut self, frame: &Value, expression: &str) -> String {
		let result = self.request("evaluate", json!({ "expression": expression, "frameId": frame["id"], "context": "repl" }));
		result["result"].as_str().unwrap().to_owned()
	}

	fn resume(&mut self) {
		self.request("continue", json!({ "threadId": 0 }));
	}

	fn initialize(&mut self) {
		let capabilities = self.request(
			"initialize",
			json!({ "adapterID": "auxtools", "linesStartAt1": true, "columnsStartAt1": true, "pathFormat": "path" })
		);
		assert_eq!(capabilities["supportsConfigurationDoneRequest"], true);
	}

	/// Waits for `text` in what the launcher passes on
	fn wait_output(&mut self, text: &str) {
		self.wait_count(text, 1);
	}

	/// Waits for `text` to have been passed on `count` times
	fn wait_count(&mut self, text: &str, count: usize) {
		let deadline = Instant::now() + TIMEOUT;
		while self.output.matches(text).count() < count {
			let message = self.next_message(deadline);
			if message["type"] == "event" && message["event"] != "output" {
				self.events.push_back(message);
			}
		}
	}

	/// Says goodbye and waits for the launcher to exit
	fn disconnect(mut self, arguments: Value) {
		self.try_request("disconnect", arguments);
		let deadline = Instant::now() + Duration::from_secs(20);
		loop {
			if let Some(status) = self.child.try_wait().unwrap() {
				assert!(status.success(), "the launcher exited with {}", status);
				return;
			}
			assert!(Instant::now() < deadline, "the launcher didn't exit");
			std::thread::sleep(Duration::from_millis(100));
		}
	}
}

/// What the client checks at the breakpoint in `target`, in each world
fn at_target(launcher: &mut Launcher) {
	launcher.stopped("breakpoint");
	let frame = launcher.top_frame();
	assert!(frame["name"].as_str().unwrap().ends_with("target"), "stopped in {}", frame);
	assert_eq!(frame["line"], TARGET_LINE);
	let source = frame["source"]["path"].as_str().unwrap_or_default();
	assert!(source.ends_with("launcher_test_host.dm"), "the frame's source is {:?}", source);

	let scopes = launcher.request("scopes", json!({ "frameId": frame["id"] }));
	let arguments = scopes["scopes"]
		.as_array()
		.unwrap()
		.iter()
		.find(|s| s["name"] == "Arguments")
		.expect("no arguments scope")
		.clone();
	let vars = launcher.request("variables", json!({ "variablesReference": arguments["variablesReference"] }));
	let n = vars["variables"]
		.as_array()
		.unwrap()
		.iter()
		.find(|v| v["name"] == "n")
		.expect("no argument n")
		.clone();
	assert_eq!(n["value"], "21");
	assert_eq!(launcher.evaluate(&frame, "n + 1"), "22");
	println!("ok: stopped at the breakpoint in target(), n is 21");
	launcher.resume();
}

fn at_second_target(launcher: &mut Launcher) {
	launcher.stopped("breakpoint");
	let frame = launcher.top_frame();
	assert!(frame["name"].as_str().unwrap().ends_with("second_target"), "stopped in {}", frame);
	println!("ok: stopped at the function breakpoint");
	launcher.resume();
}

fn at_runtime(launcher: &mut Launcher) {
	launcher.stopped("exception");
	let info = launcher.request("exceptionInfo", json!({ "threadId": 0 }));
	let message = info["exceptionId"].as_str().unwrap();
	assert!(message.contains("cannot read from list"), "the runtime was {:?}", message);
	let frame = launcher.top_frame();
	assert!(frame["name"].as_str().unwrap().ends_with("fail"), "the runtime was in {}", frame);
	println!("ok: stopped at the runtime error ({})", message);
	launcher.resume();
}

fn main() {
	let res = Command::new(paths::find_dm())
		.with_byond_paths()
		.arg(host_file("launcher_test_host.dme"))
		.status()
		.unwrap();
	assert!(res.success(), "dreammaker build failed");
	let _ = std::fs::remove_file(host_file("rebooted"));

	let mut launcher = Launcher::start();
	launcher.initialize();

	let server = std::env::var("LAUNCHER_TEST_SERVER").unwrap_or_else(|_| if cfg!(windows) { "dd" } else { "dreamdaemon" }.to_owned());
	let library = next_to_us(if cfg!(windows) { "debug_server.dll" } else { "libdebug_server.so" });
	launcher.request(
		"launch",
		json!({
			"dme": host_file("launcher_test_host.dme"),
			"byondPath": paths::find_byond(),
			"server": server,
			"debugLibrary": library,
			"args": test_args()
		})
	);

	launcher.event("initialized");
	println!("ok: the world loaded the debugger and connected");
	let set = launcher.request(
		"setBreakpoints",
		json!({
			"source": { "path": host_file("launcher_test_host.dm") },
			"breakpoints": [{ "line": TARGET_LINE }]
		})
	);
	assert_eq!(set["breakpoints"][0]["verified"], true, "the breakpoint wasn't set: {}", set);
	let set = launcher.request("setFunctionBreakpoints", json!({ "breakpoints": [{ "name": "/proc/second_target" }] }));
	assert_eq!(set["breakpoints"][0]["verified"], true, "the function breakpoint wasn't set: {}", set);
	launcher.request("setExceptionBreakpoints", json!({ "filters": ["runtimes"] }));
	launcher.request("configurationDone", json!({}));

	at_target(&mut launcher);
	at_second_target(&mut launcher);
	at_runtime(&mut launcher);

	// The world reboots, and the launcher gives the new one the breakpoints
	at_target(&mut launcher);
	assert!(
		launcher.output.contains("The world restarted"),
		"the launcher didn't see the world restart"
	);
	println!("ok: the breakpoints were set again after the reboot");
	at_second_target(&mut launcher);
	at_runtime(&mut launcher);

	// The world ends, and the launcher with it
	launcher.event("terminated");
	launcher.event("exited");
	// The programs with windows show world.log in them
	if server == "dd" || cfg!(unix) {
		assert!(
			launcher.output.contains("launcher test: world/New"),
			"the world's output wasn't passed on"
		);
		assert!(launcher.output.contains("launcher test: done"), "the world didn't finish");
		println!("ok: the world ended, and its output was passed on");
	} else {
		println!("ok: the world ended");
	}

	launcher.disconnect(json!({}));
	println!("ok: the launcher exited");
	let _ = std::fs::remove_file(host_file("rebooted"));

	attach_and_reload(&server, &library);
	println!("all ok");
}

fn test_args() -> Option<Vec<String>> {
	std::env::var("LAUNCHER_TEST_ARGS")
		.ok()
		.map(|args| args.split_whitespace().map(str::to_owned).collect())
}

/// The line of `file` that `text` starts
fn line_of(file: &Path, text: &str) -> u64 {
	let source = std::fs::read_to_string(file).unwrap();
	source.lines().position(|line| line.starts_with(text)).expect("no such line") as u64 + 1
}

/// A world launched without debugging, found with `--list`, attached to by
/// two launchers at once, and hot reloaded by the one that launched it
fn attach_and_reload(server: &str, library: &Path) {
	// A copy, so the code can be edited
	let dir = std::env::temp_dir().join(format!("launcher_test_attach-{}", std::process::id()));
	std::fs::create_dir_all(&dir).unwrap();
	for file in ["attach_host.dme", "attach_host.dm"] {
		std::fs::copy(host_file(file), dir.join(file)).unwrap();
	}
	let dme = dir.join("attach_host.dme");
	let code = dir.join("attach_host.dm");
	let res = Command::new(paths::find_dm()).with_byond_paths().arg(&dme).status().unwrap();
	assert!(res.success(), "dreammaker build failed");

	let mut runner = Launcher::start();
	runner.initialize();
	runner.request(
		"launch",
		json!({
			"noDebug": true,
			"dme": dme,
			"byondPath": paths::find_byond(),
			"server": server,
			"debugLibrary": library,
			"args": test_args()
		})
	);
	runner.event("initialized");
	runner.request("configurationDone", json!({}));
	runner.wait_output("attach test: 1");
	runner.wait_output("Hot reload: watching");
	println!("ok: a world launched without debugging runs, and its code is watched");

	let list = Command::new(next_to_us(if cfg!(windows) { "auxtools.exe" } else { "auxtools" }))
		.arg("--list")
		.output()
		.unwrap();
	let worlds: Value = serde_json::from_slice(&list.stdout).unwrap();
	let world = worlds
		.as_array()
		.unwrap()
		.iter()
		.find(|world| world["dmb"].as_str().is_some_and(|dmb| dmb.ends_with("attach_host.dmb")))
		.unwrap_or_else(|| panic!("--list didn't list the world: {}", worlds))
		.clone();
	let pid = world["pid"].as_u64().unwrap();
	println!("ok: --list found the world, process {}", pid);

	// Two debuggers: one finds the code from the world's command line, the
	// other is told where it is
	let mut first = Launcher::start();
	first.initialize();
	first.request("attach", json!({ "processId": pid.to_string() }));
	first.event("initialized");
	first.request("configurationDone", json!({}));
	let mut second = Launcher::start();
	second.initialize();
	second.request("attach", json!({ "processId": pid, "dme": dme }));
	second.event("initialized");
	second.request("configurationDone", json!({}));

	// Either one's breakpoint stops the world for both, and either can let it
	// carry on
	let line = line_of(&code, "\treturn 1");
	let set = first.request("setBreakpoints", json!({ "source": { "path": code }, "breakpoints": [{ "line": line }] }));
	assert_eq!(set["breakpoints"][0]["verified"], true, "the breakpoint wasn't set: {}", set);
	first.stopped("breakpoint");
	second.stopped("breakpoint");
	let frame = second.top_frame();
	assert!(frame["name"].as_str().unwrap().ends_with("value"), "stopped in {}", frame);
	first.request("setBreakpoints", json!({ "source": { "path": code }, "breakpoints": [] }));
	second.resume();
	first.event("continued");
	println!("ok: two launchers attached, both stopped at one's breakpoint, the other let it carry on");

	// The launcher that started it reloads it as the code changes
	let source = std::fs::read_to_string(&code).unwrap();
	std::fs::write(&code, source.replace("\treturn 1", "\treturn 2")).unwrap();
	runner.wait_output("Hot reload: reloaded /value");
	runner.wait_output("attach test: 2");
	println!("ok: the world was hot reloaded");

	// Leaving doesn't end a world that was attached to
	first.disconnect(json!({}));
	second.disconnect(json!({ "terminateDebuggee": false }));
	let seen = runner.output.matches("attach test: 2").count();
	runner.wait_count("attach test: 2", seen + 2);
	println!("ok: the attached launchers left, and the world carries on");

	std::fs::write(dir.join("stop"), "yes").unwrap();
	runner.event("terminated");
	assert!(runner.output.contains("attach test: done"), "the world didn't finish");
	runner.disconnect(json!({}));
	println!("ok: the world ended, and the launcher with it");
	let _ = std::fs::remove_dir_all(&dir);
}
