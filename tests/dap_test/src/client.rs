use std::{
	collections::VecDeque,
	io::{BufRead, BufReader, Read, Write},
	path::Path,
	process::{Child, ChildStdin, Command, Stdio},
	sync::mpsc,
	time::{Duration, Instant}
};

use serde_json::{json, Value};
use test_support::{cli, paths::library_path};

pub const TIMEOUT: Duration = Duration::from_secs(15);

pub const START_TIMEOUT: Duration = Duration::from_secs(90);

pub fn launch_arguments(byond: &Path, dme: &Path, more: Value) -> Value {
	let mut arguments = json!({
		"dme": dme,
		"byondPath": byond,
		"server": server(),
		"debugLibrary": library_path(),
		"args": test_args(),
		"hotReload": false
	});
	for (name, value) in more.as_object().unwrap() {
		arguments[name] = value.clone();
	}
	arguments
}

pub fn server() -> String {
	std::env::var("LAUNCHER_TEST_SERVER").unwrap_or_else(|_| "dreamdaemon".to_owned())
}

pub fn test_args() -> Option<Vec<String>> {
	std::env::var("LAUNCHER_TEST_ARGS")
		.ok()
		.map(|args| args.split_whitespace().map(str::to_owned).collect())
}

#[track_caller]
pub fn kill_process(pid: u64) {
	let killed = if cfg!(windows) {
		Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output()
	} else {
		Command::new("kill").args(["-9", &pid.to_string()]).output()
	};
	assert!(killed.is_ok_and(|out| out.status.success()), "couldn't kill process {}", pid);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stop {
	pub reason: String,
	pub proc: String,
	pub line: u64,
	pub thread: u64,
	pub hit: Vec<u64>
}

pub struct Launcher {
	pub child: Child,
	stdin: ChildStdin,
	messages: mpsc::Receiver<Value>,
	seq: i64,
	pub events: VecDeque<Value>,
	pub output: String,
	pub categories: Vec<(String, String)>,
	pub outputs: Vec<Value>,
	pub echo: bool,
	pub thread: u64
}

impl Launcher {
	pub fn start() -> Self {
		Self::start_with(&[])
	}

	pub fn start_with(env: &[(&str, &str)]) -> Self {
		let mut child = cli::auxtools(["dap"])
			.envs(env.iter().copied())
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
					let Ok(given) = value.trim().parse() else {
						return;
					};
					length = given;
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
			output: String::new(),
			categories: vec![],
			outputs: vec![],
			echo: true,
			thread: 0
		}
	}

	fn poll_message(&mut self, deadline: Instant) -> Option<Value> {
		let message = self.messages.recv_timeout(deadline.saturating_duration_since(Instant::now())).ok()?;
		if message["type"] == "event" && message["event"] == "stopped" {
			self.thread = message["body"]["threadId"].as_u64().expect("a stop in no thread");
		}
		if message["type"] == "event" && message["event"] == "output" {
			let output = message["body"]["output"].as_str().unwrap_or_default().replace("\r\n", "\n");
			if self.echo {
				print!("  | {}", output);
			}
			self.output.push_str(&output);
			self.outputs.push(message["body"].clone());
			self.categories
				.push((message["body"]["category"].as_str().unwrap_or_default().to_owned(), output));
		}
		Some(message)
	}

	#[track_caller]
	fn next_message(&mut self, deadline: Instant) -> Value {
		match self.poll_message(deadline) {
			Some(message) => message,
			None => panic!("timed out waiting for launcher response; output:\n{}", self.output)
		}
	}

	#[track_caller]
	pub fn send(&mut self, command: &str, arguments: Value) -> i64 {
		let seq = self.seq;
		self.seq += 1;
		let message = json!({ "seq": seq, "type": "request", "command": command, "arguments": arguments }).to_string();
		write!(self.stdin, "Content-Length: {}\r\n\r\n{}", message.len(), message).unwrap();
		self.stdin.flush().unwrap();
		seq
	}

	#[track_caller]
	pub fn request(&mut self, command: &str, arguments: Value) -> Value {
		let response = self.try_request(command, arguments);
		assert_eq!(response["success"], true, "{} failed: {}", command, response);
		response["body"].clone()
	}

	#[track_caller]
	pub fn try_request(&mut self, command: &str, arguments: Value) -> Value {
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

	#[track_caller]
	pub fn refused(&mut self, command: &str, arguments: Value) -> String {
		let response = self.try_request(command, arguments);
		assert_eq!(response["success"], false, "{} should have failed: {}", command, response);
		response["message"].as_str().unwrap_or_default().to_owned()
	}

	#[track_caller]
	pub fn event(&mut self, name: &str) -> Value {
		match self.event_within(name, TIMEOUT) {
			Some(event) => event,
			None => panic!("missing {} event; output:\n{}", name, self.output)
		}
	}

	pub fn event_within(&mut self, name: &str, wait: Duration) -> Option<Value> {
		if let Some(at) = self.events.iter().position(|e| e["event"] == name) {
			return self.events.remove(at);
		}
		let deadline = Instant::now() + wait;
		loop {
			let message = self.poll_message(deadline)?;
			if message["type"] == "event" {
				if message["event"] == name {
					return Some(message);
				}
				self.events.push_back(message);
			}
		}
	}

	#[track_caller]
	pub fn reloaded(&mut self, what: &str) -> Value {
		let deadline = Instant::now() + TIMEOUT;
		loop {
			let Some(event) = self.event_within("auxtoolsReload", deadline.saturating_duration_since(Instant::now())) else {
				panic!("no successful reload reported for {}; output:\n{}", what, self.output)
			};
			if event["body"]["state"] == "reloaded" && event["body"]["summary"].as_str().is_some_and(|summary| summary.contains(what)) {
				return event;
			}
		}
	}

	pub fn has_event(&mut self, name: &str) -> bool {
		while let Some(message) = self.poll_message(Instant::now()) {
			if message["type"] == "event" {
				self.events.push_back(message);
			}
		}
		self.events.iter().any(|e| e["event"] == name)
	}

	pub fn drop_earlier_events(&mut self, name: &str) {
		self.events.retain(|e| e["event"] != name);
	}

	pub fn drop_events(&mut self, name: &str) {
		self.has_event(name);
		self.events.retain(|e| e["event"] != name);
	}

	#[track_caller]
	pub fn stopped(&mut self, reason: &str) -> Value {
		let event = self.event("stopped");
		assert_eq!(event["body"]["reason"], reason, "stopped for the wrong reason: {}", event);
		event
	}

	#[track_caller]
	pub fn stop(&mut self) -> Stop {
		let event = self.event("stopped");
		let frame = self.top_frame();
		let ids = |list: &Value| -> Vec<u64> {
			list.as_array()
				.map(|ids| ids.iter().filter_map(Value::as_u64).collect())
				.unwrap_or_default()
		};
		Stop {
			reason: event["body"]["reason"].as_str().unwrap_or_default().to_owned(),
			proc: frame["name"].as_str().unwrap_or_default().to_owned(),
			line: frame["line"].as_u64().unwrap_or(0),
			thread: event["body"]["threadId"].as_u64().unwrap(),
			hit: ids(&event["body"]["hitBreakpointIds"])
		}
	}

	#[track_caller]
	pub fn stop_at(&mut self, reason: &str, proc: &str, line: u64) {
		let stop = self.stop();
		assert_eq!(
			(stop.reason.as_str(), stop.proc.as_str(), stop.line),
			(reason, proc, line),
			"stopped somewhere else"
		);
	}

	#[track_caller]
	pub fn frames(&mut self, thread: u64) -> Vec<Value> {
		let trace = self.request("stackTrace", json!({ "threadId": thread }));
		trace["stackFrames"].as_array().unwrap().clone()
	}

	#[track_caller]
	pub fn top_frame(&mut self) -> Value {
		let threads = self.request("threads", json!({}));
		let names: Vec<&str> = threads["threads"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
		assert!(
			!names.iter().any(|name| name.contains("auxtools_keepalive")),
			"the debugger's own loop is a thread: {:?}",
			names
		);
		let thread = self.thread;
		assert!(
			thread == 0 || threads["threads"].as_array().unwrap().iter().any(|t| t["id"] == thread),
			"the world stopped in thread {}, which isn't listed: {}",
			thread,
			threads
		);
		let trace = self.request("stackTrace", json!({ "threadId": thread }));
		trace["stackFrames"][0].clone()
	}

	#[track_caller]
	pub fn frame_id(&mut self) -> Value {
		self.top_frame()["id"].clone()
	}

	#[track_caller]
	pub fn scope(&mut self, frame: &Value, name: &str) -> Value {
		let scopes = self.request("scopes", json!({ "frameId": frame }));
		match scopes["scopes"].as_array().unwrap().iter().find(|s| s["name"] == name) {
			Some(scope) => scope["variablesReference"].clone(),
			None => panic!("no {} scope: {}", name, scopes)
		}
	}

	#[track_caller]
	pub fn variables(&mut self, reference: &Value) -> Vec<Value> {
		let vars = self.request("variables", json!({ "variablesReference": reference }));
		vars["variables"].as_array().unwrap().clone()
	}

	#[track_caller]
	pub fn scope_values(&mut self, name: &str) -> Vec<(String, String)> {
		let frame = self.frame_id();
		let reference = self.scope(&frame, name);
		named(&self.variables(&reference))
	}

	#[track_caller]
	pub fn variable(&mut self, scope: &str, name: &str) -> Value {
		let frame = self.frame_id();
		let reference = self.scope(&frame, scope);
		let vars = self.variables(&reference);
		match vars.iter().find(|v| v["name"] == name) {
			Some(variable) => variable.clone(),
			None => panic!("no {} among the {}: {:?}", name, scope, named(&vars))
		}
	}

	#[track_caller]
	pub fn local(&mut self, name: &str) -> String {
		self.variable("Locals", name)["value"].as_str().unwrap().to_owned()
	}

	#[track_caller]
	pub fn children(&mut self, variable: &Value) -> Vec<(String, String)> {
		assert_ne!(variable["variablesReference"], 0, "{} can't be opened", variable["name"]);
		let vars = self.variables(&variable["variablesReference"].clone());
		named(&vars)
	}

	#[track_caller]
	pub fn evaluate(&mut self, frame: &Value, expression: &str) -> String {
		let result = self.request("evaluate", json!({ "expression": expression, "frameId": frame["id"], "context": "repl" }));
		result["result"].as_str().unwrap().to_owned()
	}

	#[track_caller]
	pub fn eval(&mut self, expression: &str) -> String {
		let frame = self.top_frame();
		self.evaluate(&frame, expression)
	}

	#[track_caller]
	pub fn eval_global(&mut self, expression: &str) -> String {
		let result = self.request("evaluate", json!({ "expression": expression, "context": "repl" }));
		result["result"].as_str().unwrap().to_owned()
	}

	#[track_caller]
	pub fn resume(&mut self) {
		self.request("continue", json!({ "threadId": self.thread }));
	}

	#[track_caller]
	pub fn step(&mut self, command: &str) -> Stop {
		self.request(command, json!({ "threadId": self.thread }));
		self.stop()
	}

	#[track_caller]
	pub fn step_to(&mut self, command: &str, proc: &str, line: u64) {
		let stop = self.step(command);
		assert_eq!(
			(stop.reason.as_str(), stop.proc.as_str(), stop.line),
			("step", proc, line),
			"{} stopped somewhere else",
			command
		);
	}

	#[track_caller]
	pub fn initialize(&mut self) -> Value {
		self.initialize_as(json!({}))
	}

	#[track_caller]
	pub fn initialize_as(&mut self, abilities: Value) -> Value {
		let mut arguments = json!({ "adapterID": "auxtools", "linesStartAt1": true, "columnsStartAt1": true, "pathFormat": "path" });
		for (name, value) in abilities.as_object().unwrap() {
			arguments[name] = value.clone();
		}
		let capabilities = self.request("initialize", arguments);
		assert_eq!(capabilities["supportsConfigurationDoneRequest"], true);
		capabilities
	}

	#[track_caller]
	pub fn started(&mut self) {
		if self.event_within("initialized", START_TIMEOUT).is_none() {
			panic!("the world didn't start: no initialized event; output:\n{}", self.output);
		}
	}

	#[track_caller]
	pub fn launch(&mut self, byond: &Path, dme: &Path, more: Value) {
		self.request("launch", launch_arguments(byond, dme, more));
		self.started();
	}

	#[track_caller]
	pub fn attach(&mut self, pid: u64, more: Value) {
		let mut arguments = json!({ "processId": pid });
		for (name, value) in more.as_object().unwrap() {
			arguments[name] = value.clone();
		}
		self.request("attach", arguments);
		self.event("initialized");
		self.request("configurationDone", json!({}));
	}

	#[track_caller]
	pub fn set_breakpoints(&mut self, file: &Path, breakpoints: Value) -> Vec<Value> {
		let breakpoints: Vec<Value> = breakpoints
			.as_array()
			.unwrap()
			.iter()
			.map(|b| if b.is_object() { b.clone() } else { json!({ "line": b }) })
			.collect();
		let set = self.request("setBreakpoints", json!({ "source": { "path": file }, "breakpoints": breakpoints }));
		set["breakpoints"].as_array().unwrap().clone()
	}

	#[track_caller]
	pub fn break_at(&mut self, file: &Path, lines: &[u64]) {
		for (breakpoint, line) in self.set_breakpoints(file, json!(lines)).iter().zip(lines) {
			assert_eq!(breakpoint["verified"], true, "the breakpoint on line {} wasn't set: {}", line, breakpoint);
		}
	}

	#[track_caller]
	pub fn function_breakpoints(&mut self, names: &[&str]) -> Vec<Value> {
		let breakpoints: Vec<Value> = names.iter().map(|name| json!({ "name": name })).collect();
		let set = self.request("setFunctionBreakpoints", json!({ "breakpoints": breakpoints }));
		set["breakpoints"].as_array().unwrap().clone()
	}

	#[track_caller]
	pub fn catch_runtimes(&mut self, catch: bool) {
		let filters: &[&str] = if catch { &["runtimes"] } else { &[] };
		self.request("setExceptionBreakpoints", json!({ "filters": filters }));
	}

	#[track_caller]
	pub fn exception_filters(&mut self, filters: &[&str], with_conditions: &[(&str, &str)]) {
		let options: Vec<Value> = with_conditions
			.iter()
			.map(|(filter, condition)| json!({ "filterId": filter, "condition": condition }))
			.collect();
		self.request("setExceptionBreakpoints", json!({ "filters": filters, "filterOptions": options }));
	}

	#[track_caller]
	pub fn pipeline(&mut self, requests: &[(&str, Value)]) -> Vec<Value> {
		let seqs: Vec<i64> = requests
			.iter()
			.map(|(command, arguments)| self.send(command, arguments.clone()))
			.collect();
		let deadline = Instant::now() + TIMEOUT;
		let mut responses: Vec<Option<Value>> = vec![None; seqs.len()];
		while responses.iter().any(Option::is_none) {
			let message = self.next_message(deadline);
			if message["type"] == "response" {
				if let Some(at) = seqs.iter().position(|seq| message["request_seq"] == *seq) {
					responses[at] = Some(message);
				}
			} else if message["type"] == "event" {
				self.events.push_back(message);
			}
		}
		responses.into_iter().flatten().collect()
	}

	#[track_caller]
	pub fn threads(&mut self) -> Vec<(u64, String)> {
		let threads = self.request("threads", json!({}));
		threads["threads"]
			.as_array()
			.unwrap()
			.iter()
			.map(|t| (t["id"].as_u64().unwrap(), t["name"].as_str().unwrap().to_owned()))
			.collect()
	}

	#[track_caller]
	pub fn evaluate_in(&mut self, frame: &Value, expression: &str, context: &str) -> Value {
		self.request("evaluate", json!({ "expression": expression, "frameId": frame, "context": context }))
	}

	#[track_caller]
	pub fn wait_output(&mut self, text: &str) {
		self.wait_count(text, 1);
	}

	#[track_caller]
	pub fn wait_count(&mut self, text: &str, count: usize) {
		let deadline = Instant::now() + TIMEOUT;
		while self.output.matches(text).count() < count {
			let message = self.next_message(deadline);
			if message["type"] == "event" && message["event"] != "output" {
				self.events.push_back(message);
			}
		}
	}

	#[track_caller]
	pub fn wait_until(&mut self, what: &str, done: impl Fn(&str) -> bool) {
		let deadline = Instant::now() + TIMEOUT;
		while !done(&self.output) {
			let Some(message) = self.poll_message(deadline) else {
				panic!("timed out waiting for {}; output:\n{}", what, self.output);
			};
			if message["type"] == "event" && message["event"] != "output" {
				self.events.push_back(message);
			}
		}
	}

	pub fn count(&self, text: &str) -> usize {
		self.output.matches(text).count()
	}

	#[track_caller]
	pub fn disconnect(mut self, arguments: Value) {
		self.try_request("disconnect", arguments);
		let deadline = Instant::now() + Duration::from_secs(20);
		loop {
			if let Some(status) = self.child.try_wait().unwrap() {
				assert!(status.success(), "the launcher exited with {}", status);
				return;
			}
			assert!(Instant::now() < deadline, "timed out waiting for launcher exit");
			std::thread::sleep(Duration::from_millis(100));
		}
	}
}

pub fn named(vars: &[Value]) -> Vec<(String, String)> {
	vars.iter()
		.map(|v| (v["name"].as_str().unwrap().to_owned(), v["value"].as_str().unwrap().to_owned()))
		.collect()
}

#[track_caller]
pub fn value_of<'a>(vars: &'a [(String, String)], name: &str) -> &'a str {
	match vars.iter().find(|(n, _)| n == name) {
		Some((_, value)) => value.as_str(),
		None => panic!("no {} in {:?}", name, vars)
	}
}
