use std::{
	collections::HashMap,
	ops::{Deref, DerefMut},
	path::PathBuf
};

use serde_json::{json, Value};

use crate::{client::Launcher, paths};

pub fn host_file(name: &str) -> PathBuf {
	std::env::current_dir().unwrap().join("tests/dap_test_host").join(name)
}

#[derive(Clone)]
pub struct Marks {
	pub dir: PathBuf
}

impl Marks {
	fn files(&self) -> Vec<PathBuf> {
		fn files_in(dir: &std::path::Path, files: &mut Vec<PathBuf>) {
			let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().path()).collect();
			entries.sort();
			files.extend(
				entries
					.iter()
					.filter(|path| path.is_file() && path.extension().is_some_and(|e| e == "dm"))
					.cloned()
			);
			for directory in entries.iter().filter(|path| path.is_dir()) {
				files_in(directory, files);
			}
		}
		let mut files = vec![];
		files_in(&self.dir, &mut files);
		files
	}

	pub fn place(&self, name: &str) -> (PathBuf, u64) {
		let marker = format!("// @{}", name);
		for file in self.files() {
			let source = std::fs::read_to_string(&file).unwrap();
			if let Some(at) = source.lines().position(|line| line.trim_end().ends_with(&marker)) {
				return (file, at as u64 + 1);
			}
		}
		panic!("no line marked {} in {}", marker, self.dir.display())
	}

	pub fn line(&self, name: &str) -> u64 {
		self.place(name).1
	}

	pub fn file(&self, name: &str) -> PathBuf {
		let path = self.dir.join(name);
		assert!(path.is_file(), "there's no {}", path.display());
		path
	}

	pub fn line_starting(&self, file: &str, text: &str) -> u64 {
		let source = std::fs::read_to_string(self.file(file)).unwrap();
		source
			.lines()
			.position(|line| line.starts_with(text))
			.unwrap_or_else(|| panic!("no line starting {:?} in {}", text, file)) as u64
			+ 1
	}

	pub fn edit(&self, file: &str, old: &str, new: &str) {
		let path = self.file(file);
		let source = std::fs::read_to_string(&path).unwrap();
		assert!(source.contains(old), "{} has no {:?}", file, old);
		std::fs::write(&path, source.replacen(old, new, 1)).unwrap();
	}
}

pub struct World {
	pub launcher: Launcher,
	pub marks: Marks,
	with_breakpoints: Vec<PathBuf>,
	finished: HashMap<String, usize>
}

impl Deref for World {
	type Target = Launcher;

	fn deref(&self) -> &Launcher {
		&self.launcher
	}
}

impl DerefMut for World {
	fn deref_mut(&mut self) -> &mut Launcher {
		&mut self.launcher
	}
}

impl World {
	pub fn launch_in(dir: PathBuf, more: Value) -> Self {
		Self::launch_as(json!({}), dir, more)
	}

	pub fn launch_as(abilities: Value, dir: PathBuf, more: Value) -> Self {
		let mut launcher = Launcher::start();
		launcher.initialize_as(abilities);
		launcher.launch(&paths::find_byond(), &dir.join("dap_test_host.dme"), more);
		Self {
			launcher,
			marks: Marks { dir },
			with_breakpoints: vec![],
			finished: HashMap::new()
		}
	}

	pub fn attach_in(dir: PathBuf, pid: u64) -> Self {
		let mut launcher = Launcher::start();
		launcher.initialize();
		launcher.attach(pid, json!({}));
		Self {
			launcher,
			marks: Marks { dir },
			with_breakpoints: vec![],
			finished: HashMap::new()
		}
	}

	pub fn attach(pid: u64) -> Self {
		Self::attach_in(host_file(""), pid)
	}

	pub fn launch(more: Value) -> Self {
		Self::launch_in(host_file(""), more)
	}

	pub fn configure(&mut self) {
		self.request("configurationDone", json!({}));
		self.wait_output("dap: world/New");
	}

	pub fn start() -> Self {
		let mut world = Self::launch(json!({}));
		world.catch_runtimes(false);
		world.configure();
		world
	}

	pub fn break_on(&mut self, names: &[&str]) {
		let marked: Vec<(&str, &str)> = names.iter().map(|name| (*name, "")).collect();
		self.break_if(&marked);
	}

	pub fn break_if(&mut self, marked: &[(&str, &str)]) {
		for (breakpoint, (name, _)) in self.try_break_if(marked).iter().zip(marked) {
			let mut breakpoint = breakpoint.clone();
			if breakpoint["message"] == "pending hot reload" {
				let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
				while breakpoint["verified"] != true && std::time::Instant::now() < deadline {
					match self.launcher.event_within("breakpoint", std::time::Duration::from_secs(1)) {
						Some(event) if event["body"]["breakpoint"]["id"] == breakpoint["id"] => breakpoint = event["body"]["breakpoint"].clone(),
						_ => {}
					}
				}
			}
			assert_eq!(breakpoint["verified"], true, "the breakpoint at {} wasn't set: {}", name, breakpoint);
		}
	}

	pub fn try_break_if(&mut self, marked: &[(&str, &str)]) -> Vec<Value> {
		let mut by_file: Vec<(PathBuf, Vec<(usize, Value)>)> = self.with_breakpoints.drain(..).map(|file| (file, vec![])).collect();
		for (i, (name, condition)) in marked.iter().enumerate() {
			let (file, line) = self.marks.place(name);
			let breakpoint = if condition.is_empty() {
				json!({ "line": line })
			} else {
				json!({ "line": line, "condition": condition })
			};
			match by_file.iter_mut().find(|(f, _)| *f == file) {
				Some((_, breakpoints)) => breakpoints.push((i, breakpoint)),
				None => by_file.push((file, vec![(i, breakpoint)]))
			}
		}
		let mut answered = vec![Value::Null; marked.len()];
		for (file, breakpoints) in by_file {
			let wanted: Vec<Value> = breakpoints.iter().map(|(_, b)| b.clone()).collect();
			let set = self.launcher.set_breakpoints(&file, Value::Array(wanted));
			for ((i, _), breakpoint) in breakpoints.iter().zip(set) {
				answered[*i] = breakpoint;
			}
			if !breakpoints.is_empty() {
				self.with_breakpoints.push(file);
			}
		}
		answered
	}

	pub fn stop_on(&mut self, reason: &str, proc: &str, name: &str) {
		let line = self.marks.line(name);
		self.launcher.stop_at(reason, proc, line);
	}

	pub fn step_on(&mut self, command: &str, proc: &str, name: &str) {
		let line = self.marks.line(name);
		self.launcher.step_to(command, proc, line);
	}

	pub fn run_to(&mut self, scenario: &str, proc: &str, name: &str) {
		self.break_on(&[name]);
		self.run(scenario);
		self.stop_on("breakpoint", proc, name);
		self.break_on(&[]);
	}

	pub fn argument(&mut self, name: &str) -> String {
		self.launcher.variable("Arguments", name)["value"].as_str().unwrap().to_owned()
	}

	pub fn run(&mut self, scenario: &str) {
		let finished = self.count(&format!("dap: finished {}\n", scenario));
		self.finished.insert(scenario.to_owned(), finished);
		let set = self.eval_global(&format!("scenario = \"{}\"", scenario));
		assert_eq!(set, format!("\"{}\"", scenario), "couldn't ask for the scenario");
	}

	pub fn finish(&mut self, scenario: &str) {
		let finished = self.finished[scenario];
		self.wait_count(&format!("dap: finished {}\n", scenario), finished + 1);
	}

	pub fn run_through(&mut self, scenario: &str) {
		self.run(scenario);
		self.finish(scenario);
		assert!(!self.has_event("stopped"), "the world stopped during {}", scenario);
	}

	pub fn run_collecting<T>(&mut self, scenario: &str, mut at_stop: impl FnMut(&mut World) -> T) -> Vec<T> {
		self.run(scenario);
		let finished = self.finished[scenario];
		let over = format!("dap: finished {}\n", scenario);
		let deadline = std::time::Instant::now() + crate::client::TIMEOUT;
		let mut seen = vec![];
		loop {
			if self.launcher.event_within("stopped", std::time::Duration::from_millis(20)).is_some() {
				seen.push(at_stop(self));
				self.resume();
			} else if self.count(&over) > finished {
				return seen;
			}
			assert!(std::time::Instant::now() < deadline, "timed out waiting for {}", scenario);
		}
	}

	pub fn resume_through(&mut self, scenario: &str) {
		self.resume();
		self.finish(scenario);
		assert!(!self.has_event("stopped"), "the world stopped again during {}", scenario);
	}

	pub fn end(self) {
		self.launcher.disconnect(json!({}));
	}
}
