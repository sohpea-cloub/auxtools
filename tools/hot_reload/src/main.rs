//! Watches a codebase's source and hot reloads the procs that change into a
//! world running auxtools' debug server.
//!
//! The work of finding and compiling what changed is dmasm's
//! (`dmasm::reload`), so dm-langserver can do the same thing later. This only
//! watches files and talks to the debug server.

// The debug server's message types aren't exported from its cdylib, so they
// are pulled in as source, like tests/debug_test does
#[allow(dead_code)]
#[path = "../../../debug_server/src/server_types.rs"]
mod server_types;

use std::{
	collections::{HashMap, HashSet},
	io::{ErrorKind, Read, Write},
	net::{Ipv4Addr, TcpStream},
	path::{Path, PathBuf},
	time::{Duration, Instant, SystemTime}
};

use dmasm::reload::{diff, Changes, ParseOptions, ProcKey, Snapshot};
use server_types::{
	ContinueKind, HotReloadInitProc, HotReloadLines, HotReloadNewProc, HotReloadProc, HotReloadRequest, HotReloadResult, HotReloadVarDefault,
	ProcRef, Request, Response, DEFAULT_RELOAD_PORT
};

const USAGE: &str = "\
usage: hot_reload WORLD.dme [options]

Watches the code of WORLD.dme and hot reloads the procs that change into the
running world. The world has to run auxtools' debug server (any
AUXTOOLS_DEBUG_MODE but NONE), which listens for watchers on its reload port,
AUXTOOLS_RELOAD_PORT (2449 unless set). A debugger can be attached at the same
time.

The world should be running the code as it is when this starts. Proc code,
new procs (more definitions of one too), new vars and static locals, var
defaults (those that aren't constants reach new instances), parameters,
`set waitfor` and `set background` can change. New types, verbs, prefabs with
var overrides, new `in` list expressions and other `set` statements need the
world rebuilt, and are refused, along with the rest of the change.

  --port N          the debug server's reload port (default 2449)
  -DNAME[=VALUE]    a define DreamMaker was given on its command line
  --interval MS     how often to look for changes (default 500)";

struct Options {
	dme: PathBuf,
	port: u16,
	defines: Vec<(String, Option<String>)>,
	interval: Duration
}

fn parse_args() -> Result<Options, String> {
	let mut dme = None;
	let mut port = DEFAULT_RELOAD_PORT;
	let mut defines = vec![];
	let mut interval = Duration::from_millis(500);

	let mut args = std::env::args().skip(1);
	while let Some(arg) = args.next() {
		let mut value = |name: &str| args.next().ok_or_else(|| format!("{} needs a value", name));
		match arg.as_str() {
			"--port" => port = value("--port")?.parse().map_err(|e| format!("--port: {}", e))?,
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
		port,
		defines,
		interval
	})
}

fn log(message: impl std::fmt::Display) {
	let secs = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
	println!("[{:02}:{:02}:{:02}] {}", secs / 3600 % 24, secs / 60 % 60, secs % 60, message);
}

/// The connection to the debug server.
struct Connection(TcpStream);

impl Connection {
	fn connect(port: u16) -> std::io::Result<Self> {
		let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
		stream.set_nodelay(true)?;
		Ok(Self(stream))
	}

	fn send(&mut self, request: &Request) -> std::io::Result<()> {
		let data = bincode::serialize(request).map_err(std::io::Error::other)?;
		self.0.write_all(&(data.len() as u32).to_le_bytes())?;
		self.0.write_all(&data)?;
		self.0.flush()
	}

	fn read_message(&mut self) -> std::io::Result<Response> {
		let mut len = [0u8; 4];
		self.0.read_exact(&mut len)?;
		let mut data = vec![0u8; u32::from_le_bytes(len) as usize];
		self.0.read_exact(&mut data)?;
		bincode::deserialize(&data).map_err(std::io::Error::other)
	}

	/// Deals with what the debug server says unasked. Err means it's gone.
	fn handle(&mut self, response: Response) -> std::io::Result<Option<Response>> {
		match response {
			Response::Notification { .. } => Ok(None),
			Response::Disconnect => Err(std::io::Error::new(ErrorKind::ConnectionAborted, "the debug server hung up")),
			// The reload port never sends these, but on the debugger's port
			// don't leave the world stuck
			Response::BreakpointHit { reason } => {
				log(format!("the world paused ({:?}); telling it to go on", reason));
				self.send(&Request::Continue {
					kind: ContinueKind::Continue
				})?;
				Ok(None)
			}
			response => Ok(Some(response))
		}
	}

	/// The answer to the last request.
	fn recv(&mut self) -> std::io::Result<Response> {
		loop {
			let message = self.read_message()?;
			if let Some(response) = self.handle(message)? {
				// The Ack for a Continue sent from handle()
				if !matches!(response, Response::Ack) {
					return Ok(response);
				}
			}
		}
	}

	fn request(&mut self, request: &Request) -> std::io::Result<Response> {
		self.send(request)?;
		self.recv()
	}

	/// Reads whatever the debug server sent unasked, to notice it going away.
	fn poll(&mut self) -> std::io::Result<()> {
		self.0.set_nonblocking(true)?;
		let mut byte = [0u8; 1];
		let waiting = match self.0.peek(&mut byte) {
			Ok(0) => Err(std::io::Error::new(ErrorKind::ConnectionAborted, "the debug server hung up")),
			Ok(_) => Ok(true),
			Err(e) if e.kind() == ErrorKind::WouldBlock => Ok(false),
			Err(e) => Err(e)
		};
		self.0.set_nonblocking(false)?;

		if waiting? {
			let message = self.read_message()?;
			self.handle(message)?;
		}
		Ok(())
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
		settings: r.settings,
		statics: r.statics.clone()
	}
}

fn reload_request(changes: &Changes) -> Request {
	Request::HotReload(HotReloadRequest {
		new_types: changes.new_types.clone(),
		procs: changes.reloads.iter().map(proc_code).collect(),
		new_procs: changes
			.new_procs
			.iter()
			.map(|p| HotReloadNewProc {
				type_path: p.type_path.clone(),
				name: p.name.clone(),
				declared: p.declared,
				code: proc_code(&p.code)
			})
			.collect(),
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
				list(changes.new_procs.iter().map(|p| p.code.proc.to_string()).collect(), "procs").map(|s| format!("added {}", s)),
				list(
					changes.new_vars.iter().map(|v| format!("{}/var/{}", v.type_path, v.var)).collect(),
					"vars"
				)
				.map(|s| format!("added {}", s)),
				list(
					changes.var_defaults.iter().map(|d| format!("{}/var/{}", d.types[0], d.var)).collect(),
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
	let port = options.as_ref().unwrap().port;
	let mut waiting = false;

	loop {
		let mut conn = match Connection::connect(port) {
			Ok(conn) => conn,
			Err(_) => {
				if watcher.is_none() && !waiting {
					log(format!("waiting for the debug server on port {}", port));
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
