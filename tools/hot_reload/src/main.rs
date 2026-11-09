//! Watches a codebase's source and hot reloads the procs that change into a
//! world running auxtools' debug server.
//!
//! The work of finding and compiling what changed is dmasm's
//! (`dmasm::reload`), so the launcher can do the same thing later. This only
//! watches files and talks to the debug server.

use std::{
	collections::{HashMap, HashSet},
	io::ErrorKind,
	path::{Path, PathBuf},
	sync::mpsc,
	time::{Duration, Instant, SystemTime}
};

use dmasm::reload::{diff, Changes, ParseOptions, ProcKey, Snapshot};
use auxtools_cli::{
	local::{self, Shared},
	server_types::{
		HotReloadInitProc, HotReloadLines, HotReloadNewProc, HotReloadProc, HotReloadRequest, HotReloadResult, HotReloadVarDefault, ProcRef, Request,
		Response
	}
};

const USAGE: &str = "\
usage: hot_reload WORLD.dme --pid PID [options]

Watches the code of WORLD.dme and hot reloads the procs that change into the
world running in process PID. The world has to be one the launcher started,
with debugging or without (`auxtools --list` lists them): its debug
server takes any number of clients, and debuggers can be there at the same
time.

The world should be running the code as it is when this starts. Proc code,
new procs and verbs (more definitions of one too), definitions taken away,
new types (mobs too), new vars and static locals, var defaults (built-in
ones too; those that aren't constants reach new instances), parameters, `in`
lists, prefabs and `set` statements can change. Changed var declarations and
parent types, and the few built-in vars a reload can't set (`ckey`, ...),
need the world rebuilt, and are refused, along with the rest of the change.

  --pid PID         the world's process
  -DNAME[=VALUE]    a define DreamMaker was given on its command line
  --interval MS     how often to look for changes (default 500)";

struct Options {
	dme: PathBuf,
	pid: u32,
	defines: Vec<(String, Option<String>)>,
	interval: Duration
}

fn parse_args() -> Result<Options, String> {
	let mut dme = None;
	let mut pid = None;
	let mut defines = vec![];
	let mut interval = Duration::from_millis(500);

	let mut args = std::env::args().skip(1);
	while let Some(arg) = args.next() {
		let mut value = |name: &str| args.next().ok_or_else(|| format!("{} needs a value", name));
		match arg.as_str() {
			"--pid" => pid = Some(value("--pid")?.parse().map_err(|e| format!("--pid: {}", e))?),
			"--interval" => interval = Duration::from_millis(value("--interval")?.parse().map_err(|e| format!("--interval: {}", e))?),
			"-h" | "--help" => return Err(String::new()),
			_ if arg.starts_with("-D") => {
				let define = &arg[2..];
				defines.push(match define.split_once('=') {
					Some((name, value)) => (name.to_owned(), Some(value.to_owned())),
					None => (define.to_owned(), None)
				});
			}
			_ if arg.starts_with('-') => return Err(format!("unknown option {}", arg)),
			_ if dme.is_none() => dme = Some(PathBuf::from(arg)),
			_ => return Err(format!("unexpected argument {}", arg))
		}
	}

	let dme = dme.ok_or("which .dme?")?;
	let dme = std::fs::canonicalize(&dme).map_err(|e| format!("{}: {}", dme.display(), e))?;
	Ok(Options {
		dme,
		pid: pid.ok_or("which world? (--pid)")?,
		defines,
		interval
	})
}

fn log(message: impl std::fmt::Display) {
	let secs = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
	println!("[{:02}:{:02}:{:02}] {}", secs / 3600 % 24, secs / 60 % 60, secs % 60, message);
}

/// The connection to the debug server, read on a thread of its own
struct Connection {
	writer: Shared,
	messages: mpsc::Receiver<std::io::Result<Response>>
}

impl Connection {
	fn connect(pid: u32) -> std::io::Result<Self> {
		let writer = Shared::new(local::connect(pid)?);
		let mut reader = writer.clone();
		let (sender, messages) = mpsc::channel();
		std::thread::spawn(move || loop {
			let message = local::receive(&mut reader);
			let end = message.is_err();
			if sender.send(message).is_err() || end {
				return;
			}
		});
		Ok(Self { writer, messages })
	}

	fn send(&mut self, request: &Request) -> std::io::Result<()> {
		local::send(&mut self.writer, request)
	}

	/// Deals with what the debug server says unasked. Err means it's gone.
	fn handle(response: std::io::Result<Response>) -> std::io::Result<Option<Response>> {
		match response? {
			// The world pausing for a debugger, and carrying on, are the
			// debugger's business
			Response::Notification { .. } | Response::BreakpointHit { .. } | Response::Resumed => Ok(None),
			Response::Disconnect => Err(std::io::Error::new(ErrorKind::ConnectionAborted, "the debug server hung up")),
			response => Ok(Some(response))
		}
	}

	/// The answer to the last request.
	fn recv(&mut self) -> std::io::Result<Response> {
		loop {
			let message = self
				.messages
				.recv()
				.map_err(|_| std::io::Error::new(ErrorKind::ConnectionAborted, "the debug server hung up"))?;
			if let Some(response) = Self::handle(message)? {
				return Ok(response);
			}
		}
	}

	fn request(&mut self, request: &Request) -> std::io::Result<Response> {
		self.send(request)?;
		self.recv()
	}

	/// Reads whatever the debug server sent unasked, to notice it going away.
	fn poll(&mut self) -> std::io::Result<()> {
		loop {
			match self.messages.try_recv() {
				Ok(message) => {
					Self::handle(message)?;
				}
				Err(mpsc::TryRecvError::Empty) => return Ok(()),
				Err(mpsc::TryRecvError::Disconnected) => return Err(std::io::Error::new(ErrorKind::ConnectionAborted, "the debug server hung up"))
			}
		}
	}
}

fn proc_ref(key: &ProcKey) -> ProcRef {
	ProcRef {
		path: key.path.clone(),
		override_id: key.override_id
	}
}

fn proc_code(r: &dmasm::reload::ProcReload) -> HotReloadProc {
	HotReloadProc {
		proc: proc_ref(&r.proc),
		nodes: r.nodes.clone(),
		locals: r.locals.clone(),
		params: r.params.clone(),
		settings: r.settings.clone(),
		statics: r.statics.clone()
	}
}

fn reload_request(changes: &Changes) -> Request {
	Request::HotReload(HotReloadRequest {
		new_types: changes.new_types.clone(),
		procs: changes.reloads.iter().map(proc_code).collect(),
		settings: changes.settings.clone(),
		new_procs: changes
			.new_procs
			.iter()
			.map(|p| HotReloadNewProc {
				type_path: p.type_path.clone(),
				name: p.name.clone(),
				declared: p.declared,
				verb: p.verb,
				code: proc_code(&p.code)
			})
			.collect(),
		removed_procs: changes.removed_procs.clone(),
		builtin_defaults: changes.builtin_defaults.clone(),
		new_vars: changes.new_vars.clone(),
		init_procs: changes
			.init_procs
			.iter()
			.map(|i| HotReloadInitProc {
				type_path: i.type_path.clone(),
				code: i.code.as_ref().map(proc_code)
			})
			.collect(),
		var_defaults: changes
			.var_defaults
			.iter()
			.map(|d| HotReloadVarDefault {
				var: d.var.clone(),
				value: d.value.clone(),
				types: d.types.clone()
			})
			.collect(),
		line_moves: changes
			.line_moves
			.iter()
			.map(|m| HotReloadLines {
				proc: proc_ref(&m.proc),
				lines: m.lines.clone()
			})
			.collect()
	})
}

/// When each of the codebase's files was last changed.
fn modified_times(root: &Path, files: &[PathBuf]) -> HashMap<PathBuf, Option<SystemTime>> {
	files
		.iter()
		.map(|file| (file.clone(), std::fs::metadata(root.join(file)).and_then(|m| m.modified()).ok()))
		.collect()
}

struct Watcher {
	options: Options,
	parse_options: ParseOptions,
	root: PathBuf,
	/// The code the world started with: the source when we started, unless
	/// the .dmb has been built since
	built: Snapshot,
	/// Files changed since `built`
	edited_since_built: HashSet<PathBuf>,
	dmb_modified: Option<SystemTime>,
	/// The code the world runs now, when it isn't `built`
	applied: Option<Snapshot>,
	modified: HashMap<PathBuf, Option<SystemTime>>
}

impl Watcher {
	fn applied(&self) -> &Snapshot {
		self.applied.as_ref().unwrap_or(&self.built)
	}

	fn parse(&self) -> Snapshot {
		let started = Instant::now();
		let snapshot = Snapshot::parse(&self.options.dme, &self.parse_options).unwrap_or_else(|e| {
			eprintln!("{}", e);
			std::process::exit(1);
		});
		log(format!(
			"parsed {} procs in {:.1} s",
			snapshot.procs().count(),
			started.elapsed().as_secs_f32()
		));
		snapshot
	}

	fn dmb_modified(&self) -> Option<SystemTime> {
		std::fs::metadata(self.options.dme.with_extension("dmb")).and_then(|m| m.modified()).ok()
	}

	fn files(snapshot: &Snapshot, dme: &Path) -> Vec<PathBuf> {
		let mut files = snapshot.files();
		files.push(dme.to_owned());
		files
	}

	/// Files changed since we last looked.
	fn changed_files(&mut self) -> HashSet<PathBuf> {
		let files: Vec<PathBuf> = self.modified.keys().cloned().collect();
		let now = modified_times(&self.root, &files);
		let changed = now
			.iter()
			.filter(|(file, time)| self.modified.get(*file) != Some(time))
			.map(|(file, _)| file.clone())
			.collect();
		self.modified = now;
		changed
	}

	/// Sends `changes` (from `self.applied` to `new`) and reports what came of
	/// it. True when the world has the new code.
	fn send(&mut self, conn: &mut Connection, changes: &Changes) -> std::io::Result<bool> {
		if !changes.refused.is_empty() {
			log("not reloaded, because the world can't take all of it (rebuild for these):");
			for why in &changes.refused {
				println!("    {}", why);
			}
			return Ok(false);
		}

		for why in &changes.warnings {
			println!("    note: {}", why);
		}

		let request = reload_request(changes);
		if matches!(&request, Request::HotReload(r) if r.is_empty()) {
			log("nothing the world runs changed");
			return Ok(true);
		}

		let started = Instant::now();
		let result = match conn.request(&request)? {
			Response::HotReload(result) => result,
			other => {
				log(format!("the debug server answered with {:?}", other));
				return Ok(false);
			}
		};
		let HotReloadResult { applied, errors, warnings } = result;

		if applied {
			let list = |names: Vec<String>, what: &str| match names.len() {
				0 => None,
				1..=5 => Some(names.join(", ")),
				n => Some(format!("{} {}", n, what))
			};
			let parts: Vec<String> = [
				list(changes.new_types.iter().map(|t| t.path.clone()).collect(), "types").map(|s| format!("added {}", s)),
				list(changes.reloads.iter().map(|r| r.proc.to_string()).collect(), "procs").map(|s| format!("reloaded {}", s)),
				list(changes.settings.iter().map(|c| c.proc.to_string()).collect(), "procs").map(|s| format!("set the settings of {}", s)),
				list(changes.new_procs.iter().map(|p| p.code.proc.to_string()).collect(), "procs").map(|s| format!("added {}", s)),
				list(changes.removed_procs.iter().map(|p| p.proc.to_string()).collect(), "procs").map(|s| format!("unlinked {}", s)),
				list(
					changes.new_vars.iter().map(|v| format!("{}/var/{}", v.type_path, v.var)).collect(),
					"vars"
				)
				.map(|s| format!("added {}", s)),
				list(
					changes
						.var_defaults
						.iter()
						.chain(&changes.builtin_defaults)
						.map(|d| format!("{}/var/{}", d.types[0], d.var))
						.collect(),
					"vars"
				)
				.map(|s| format!("set the default of {}", s)),
				list(changes.init_procs.iter().map(|i| i.type_path.clone()).collect(), "types").map(|s| format!("rebuilt the init procs of {}", s)),
				(!changes.line_moves.is_empty()).then(|| format!("moved the lines of {} more", changes.line_moves.len()))
			]
			.into_iter()
			.flatten()
			.collect();
			log(format!("{} in {} ms", parts.join("; "), started.elapsed().as_millis()));
		} else {
			log("not reloaded:");
		}
		for e in &errors {
			println!("    {}", e);
		}
		for w in &warnings {
			println!("    warning: {}", w);
		}
		Ok(applied)
	}

	/// Watches until the connection goes.
	fn watch(&mut self, conn: &mut Connection) -> std::io::Result<()> {
		loop {
			std::thread::sleep(self.options.interval);
			conn.poll()?;

			let mut changed = self.changed_files();
			if changed.is_empty() {
				continue;
			}

			// Editors save in steps, and a checkout touches many files
			loop {
				std::thread::sleep(Duration::from_millis(200));
				let more = self.changed_files();
				if more.is_empty() {
					break;
				}
				changed.extend(more);
			}

			let mut shown: Vec<String> = changed.iter().map(|f| f.display().to_string()).collect();
			shown.sort();
			log(format!("changed: {}", shown.join(", ")));

			let new = self.parse();
			let changes = diff(self.applied(), &new, Some(&changed));
			self.edited_since_built.extend(changed);

			// A file the new parse includes for the first time
			for file in Self::files(&new, &self.options.dme) {
				if !self.modified.contains_key(&file) {
					let time = std::fs::metadata(self.root.join(&file)).and_then(|m| m.modified()).ok();
					self.modified.insert(file, time);
				}
			}

			if self.send(conn, &changes)? {
				self.applied = Some(new);
			}
		}
	}

	/// Brings a world that just connected up to date. It might have rebooted
	/// and lost earlier reloads, or been rebuilt.
	fn catch_up(&mut self, conn: &mut Connection) -> std::io::Result<()> {
		let dmb_modified = self.dmb_modified();
		if dmb_modified != self.dmb_modified {
			log("the .dmb was rebuilt, so the world runs the source as it is now");
			self.dmb_modified = dmb_modified;
			self.built = self.parse();
			self.applied = None;
			self.edited_since_built.clear();
			return Ok(());
		}

		// Whatever was reloaded went with the old world
		self.applied = None;
		if self.edited_since_built.is_empty() {
			return Ok(());
		}

		log("the world started again from its .dmb: reloading the changes since then");
		let edited = self.edited_since_built.clone();
		let current = self.parse();
		let changes = diff(&self.built, &current, Some(&edited));
		if self.send(conn, &changes)? {
			self.applied = Some(current);
		}
		Ok(())
	}
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

	let mut watcher: Option<Watcher> = None;
	let mut options = Some(options);
	let pid = options.as_ref().unwrap().pid;
	let mut waiting = false;

	loop {
		let mut conn = match Connection::connect(pid) {
			Ok(conn) => conn,
			Err(_) => {
				if !local::alive(pid) {
					log(format!("process {} has ended", pid));
					return;
				}
				if watcher.is_none() && !waiting {
					log(format!("waiting for the debug server in process {}", pid));
				}
				waiting = true;
				std::thread::sleep(Duration::from_secs(1));
				continue;
			}
		};
		waiting = false;

		let result = (|| -> std::io::Result<()> {
			let (version, build) = match conn.request(&Request::ByondVersion)? {
				Response::ByondVersion { version, build } => (version, build),
				other => {
					return Err(std::io::Error::other(format!(
						"the debug server doesn't know hot reloading (it answered {:?}); it needs a newer debug_server",
						other
					)));
				}
			};
			log(format!("connected to a world on BYOND {}.{}", version, build));

			match &mut watcher {
				None => {
					let options = options.take().unwrap();
					let root = options.dme.parent().unwrap().to_owned();
					let parse_options = ParseOptions {
						defines: options.defines.clone(),
						byond: Some((version, build))
					};
					let started = Instant::now();
					let mut w = Watcher {
						dmb_modified: None,
						modified: HashMap::new(),
						edited_since_built: HashSet::new(),
						built: Snapshot::parse(&options.dme, &parse_options).map_err(std::io::Error::other)?,
						applied: None,
						options,
						parse_options,
						root
					};
					w.dmb_modified = w.dmb_modified();
					let files = Watcher::files(&w.built, &w.options.dme);
					w.modified = modified_times(&w.root, &files);
					log(format!(
						"parsed in {:.1} s; watching {} files with {} procs",
						started.elapsed().as_secs_f32(),
						files.len(),
						w.built.procs().count()
					));
					watcher = Some(w);
				}
				Some(w) => w.catch_up(&mut conn)?
			}

			watcher.as_mut().unwrap().watch(&mut conn)
		})();

		match result {
			Err(e) if e.kind() == ErrorKind::Other => {
				eprintln!("{}", e);
				std::process::exit(1);
			}
			Err(e) => log(format!("lost the world ({}); waiting for it to come back", e)),
			Ok(()) => {}
		}
		std::thread::sleep(Duration::from_secs(1));
	}
}
