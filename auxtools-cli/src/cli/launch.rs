use std::{
	fs::{File, OpenOptions},
	io::{self, BufRead, BufReader, Read, Write},
	path::{Path, PathBuf},
	process::{Command, Stdio},
	sync::{
		atomic::{AtomicBool, AtomicU32, Ordering},
		mpsc, Arc, Mutex
	},
	time::Duration
};

use auxtools_cli::byond::Server;
use serde_json::json;

use super::{args, Failure, Result, TIMED_OUT};
use crate::{
	adapter::{self, Ask, CommandLine, Input},
	dap::{Note, Output}
};

pub const WORLD_FAILED: i32 = 4;
pub const COMING_UP: Duration = Duration::from_secs(120);
const DETACHED: &str = "--detached";

struct Options {
	dmb: PathBuf,
	dme: Option<PathBuf>,
	server: Option<String>,
	port: Option<u16>,
	hot_reload: bool,
	defines: Vec<String>,
	byond: Option<PathBuf>,
	args: Option<Vec<String>>,
	log: Option<PathBuf>
}

pub fn launch(args: &args::Launch, replaces_args: bool) -> Result<()> {
	let dmb = absolute(&args.dmb)?;
	if !dmb.is_file() {
		return Err(format!(
			"{} does not exist. Compile the world first: the launcher does not compile the DMB.",
			dmb.display()
		)
		.into());
	}

	let program = match &args.server {
		Some(name) => serde_json::from_value::<Server>(json!(name)).map_err(|_| {
			format!(
				"invalid value '{}' for --server: expected dreamdaemon, dreamdaemon-gui, or dreamseeker.",
				name
			)
		})?,
		None => Server::default_for_platform()
	};
	if args.port.is_some() && program == Server::DreamSeeker && cfg!(windows) {
		return Err("--port cannot be used with DreamSeeker. Use --server dreamdaemon.".into());
	}
	let options = Options {
		dmb,
		dme: args.dme.as_deref().map(absolute).transpose()?,
		server: args.server.clone(),
		port: args.port,
		hot_reload: !args.no_hot_reload,
		defines: args.defines.clone(),
		byond: args.byond.as_deref().map(absolute).transpose()?,
		args: replaces_args.then(|| args.args.clone()),
		log: args.log.as_deref().map(absolute).transpose()?
	};

	if args.detached {
		run(&options, true)
	} else if args.detach {
		detach(&options, args.json)
	} else {
		run(&options, false)
	}
}

fn absolute(path: &Path) -> Result<PathBuf> {
	std::path::absolute(path).map_err(|e| format!("invalid path {}: {}.", path.display(), e).into())
}

enum Printer {
	Terminal,
	File(Mutex<File>)
}

impl Printer {
	fn print(&self, category: &str, text: &str) {
		match self {
			Self::Terminal if category == "stderr" => {
				let _ = io::stderr().write_all(text.as_bytes());
			}
			Self::Terminal => {
				let mut stdout = io::stdout().lock();
				let _ = stdout.write_all(text.as_bytes());
				let _ = stdout.flush();
			}
			Self::File(file) => {
				let _ = file.lock().unwrap().write_all(text.as_bytes());
			}
		}
	}

	fn line(&self, text: &str) {
		self.print("console", &format!("{}\n", text));
	}
}

#[derive(Default)]
struct Outcome {
	pid: Option<u32>,
	failed: Option<String>,
	exit_code: Option<i64>,
	ready: bool
}

fn run(options: &Options, detached: bool) -> Result<()> {
	let printer = Arc::new(match &options.log {
		Some(log) => {
			let file = if detached {
				OpenOptions::new().append(true).create(true).open(log)
			} else {
				File::create(log)
			};
			Printer::File(Mutex::new(file.map_err(|e| format!("cannot write {}: {}.", log.display(), e))?))
		}
		None => Printer::Terminal
	});
	let (inputs, receiver) = mpsc::channel();
	let outcome = Arc::new(Mutex::new(Outcome::default()));

	let (told, print, quit) = (outcome.clone(), printer.clone(), inputs.clone());
	let out = Output::plain(move |note| {
		let mut outcome = told.lock().unwrap();
		match note {
			Note::Line { category, text } => print.print(&category, &text),
			Note::Process(pid) => outcome.pid = Some(pid),
			Note::Ready => {
				if detached && !outcome.ready {
					say_to_starter(&format!("ready {}", outcome.pid.unwrap_or_default()));
				}
				outcome.ready = true;
			}
			Note::Unavailable(why) => {
				if detached && !outcome.ready {
					say_to_starter(&format!("unavailable {} {}", outcome.pid.unwrap_or_default(), why.replace('\n', " ")));
				}
				outcome.ready = true;
			}
			Note::LaunchFailed(message) => {
				if detached {
					say_to_starter(&format!("failed {}", message));
				}
				outcome.failed = Some(message);
				let _ = quit.send(Input::Quit);
			}
			Note::Exited(code) => {
				outcome.exit_code = Some(code);
				let _ = quit.send(Input::Quit);
			}
		}
	});

	let server = options
		.server
		.as_deref()
		.map(|name| serde_json::from_value(json!(name)).unwrap_or(Server::default_for_platform()));
	let args = options
		.args
		.clone()
		.unwrap_or_else(|| adapter::default_args(server.unwrap_or(Server::default_for_platform())));
	let _ = inputs.send(Input::Asked(Ask::Launch(CommandLine {
		dmb: options.dmb.clone(),
		dme: options.dme.clone(),
		server,
		port: options.port,
		hot_reload: options.hot_reload,
		defines: options.defines.clone(),
		byond: options.byond.clone(),
		args
	})));

	interrupts::listen(inputs.clone(), printer.clone());
	adapter::serve(out, inputs, &receiver, Default::default());
	interrupts::done();

	let outcome = outcome.lock().unwrap();
	if let Some(why) = &outcome.failed {
		return Err(why.clone().into());
	}
	match outcome.exit_code {
		Some(0) => Ok(()),
		Some(code) if code >= 0 => Err(Failure(WORLD_FAILED, format!("the world exited with code {}.", code))),
		_ => Err(Failure(WORLD_FAILED, "the world was terminated.".to_owned()))
	}
}

fn say_to_starter(line: &str) {
	let mut stdout = io::stdout().lock();
	let _ = writeln!(stdout, "{}", line);
	let _ = stdout.flush();
}

pub const LOG_EXTENSION: &str = "auxtools.log";

fn default_log(dmb: &Path) -> PathBuf {
	dmb.with_extension(LOG_EXTENSION)
}

fn detach(options: &Options, as_json: bool) -> Result<()> {
	let log = options.log.clone().unwrap_or_else(|| default_log(&options.dmb));
	let stderr = File::create(&log)
		.and_then(|_| OpenOptions::new().append(true).open(&log))
		.map_err(|e| format!("cannot write {}: {}.", log.display(), e))?;

	let mut args: Vec<std::ffi::OsString> = vec!["launch".into(), options.dmb.clone().into()];
	let mut flag = |name: &str, value: Option<std::ffi::OsString>| {
		if let Some(value) = value {
			args.push(name.into());
			args.push(value);
		}
	};
	flag("--dme", options.dme.clone().map(Into::into));
	flag("--server", options.server.clone().map(Into::into));
	flag("--port", options.port.map(|port| port.to_string().into()));
	flag("--byond", options.byond.clone().map(Into::into));
	flag("--log", Some(log.clone().into()));
	for define in &options.defines {
		flag("-D", Some(define.into()));
	}
	if !options.hot_reload {
		args.push("--no-hot-reload".into());
	}
	args.push(DETACHED.into());
	if let Some(theirs) = &options.args {
		args.push("--".into());
		args.extend(theirs.iter().map(Into::into));
	}

	let program = std::env::current_exe().map_err(|e| format!("cannot find the auxtools program: {}.", e))?;
	let mut command = Command::new(program);
	command.args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(stderr);
	let mut launcher = background::spawn(&mut command).map_err(|e| format!("cannot start the launcher in the background: {}.", e))?;
	let launcher_pid = launcher.id();

	let mut stdout = launcher.stdout.take().expect("piped");
	let (sender, said) = mpsc::channel();
	std::thread::spawn(move || {
		let mut line = String::new();
		let read = BufReader::new(&mut stdout).read_line(&mut line);
		let _ = sender.send(read.ok().filter(|n| *n > 0).map(|_| line.trim_end().to_owned()));
		let _ = io::copy(&mut stdout, &mut io::sink());
	});
	let line = match said.recv_timeout(COMING_UP) {
		Ok(line) => line,
		Err(_) => {
			let _ = launcher.kill();
			let _ = launcher.wait();
			return Err(Failure(
				TIMED_OUT,
				format!(
					"timed out after {} seconds waiting for the world to start; it was terminated. See the launcher log: {}",
					COMING_UP.as_secs(),
					log.display()
				)
			));
		}
	};
	let started = match line.as_deref().map(|line| line.split_once(' ').unwrap_or((line, ""))) {
		Some(("ready", pid)) => Some((pid, None)),
		Some(("unavailable", said)) => Some(said.split_once(' ').map_or((said, Some("")), |(pid, why)| (pid, Some(why)))),
		_ => None
	};
	if let Some((pid, unavailable)) = started {
		let pid: u32 = pid.parse().unwrap_or_default();
		if as_json {
			say!(
				"{}",
				json!({ "pid": pid, "launcher": launcher_pid, "log": log, "unavailable": unavailable })
			);
		} else {
			match unavailable {
				Some(why) => say!("pid {}: running without debugging: {}. Use 'auxtools stop' to terminate it.", pid, why),
				None => say!("pid {}: started", pid)
			}
			say!("launcher: pid {}", launcher_pid);
			say!("log: {}", log.display());
		}
		return Ok(());
	}

	let status = launcher.wait().ok().and_then(|status| status.code());
	match line.as_deref().and_then(|line| line.strip_prefix("failed ")) {
		Some(why) => Err(why.to_owned().into()),
		None => Err(format!(
			"the world did not start: its launcher exited{} first. The end of the launcher log ({}):\n{}",
			status.map(|code| format!(" with code {}", code)).unwrap_or_default(),
			log.display(),
			last_lines(&log, 20)
		)
		.into())
	}
}

fn last_lines(path: &Path, n: usize) -> String {
	let mut text = String::new();
	let _ = File::open(path).and_then(|mut file| file.read_to_string(&mut text));
	let lines: Vec<&str> = text.lines().collect();
	lines[lines.len().saturating_sub(n)..].join("\n")
}

mod background {
	use std::process::{Child, Command};

	#[cfg(unix)]
	pub fn spawn(command: &mut Command) -> std::io::Result<Child> {
		use std::os::unix::process::CommandExt;

		unsafe {
			command.pre_exec(|| {
				libc::setsid();
				Ok(())
			});
		}
		command.spawn()
	}

	#[cfg(windows)]
	pub fn spawn(command: &mut Command) -> std::io::Result<Child> {
		use std::os::windows::{io::AsRawHandle, process::CommandExt};

		use winapi::um::{
			handleapi::SetHandleInformation,
			winbase::{CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, HANDLE_FLAG_INHERIT}
		};

		for handle in [
			std::io::stdin().as_raw_handle(),
			std::io::stdout().as_raw_handle(),
			std::io::stderr().as_raw_handle()
		] {
			if !handle.is_null() {
				unsafe {
					SetHandleInformation(handle as _, HANDLE_FLAG_INHERIT, 0);
				}
			}
		}
		let flags = CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP;
		command.creation_flags(flags | CREATE_BREAKAWAY_FROM_JOB);
		command.spawn().or_else(|_| {
			command.creation_flags(flags);
			command.spawn()
		})
	}
}

mod interrupts {
	use super::*;

	static INTERRUPTS: AtomicU32 = AtomicU32::new(0);
	static DONE: AtomicBool = AtomicBool::new(false);

	pub fn listen(inputs: mpsc::Sender<Input>, printer: Arc<Printer>) {
		catch();
		std::thread::spawn(move || {
			let mut seen = 0;
			loop {
				std::thread::sleep(Duration::from_millis(100));
				let now = INTERRUPTS.load(Ordering::SeqCst);
				if now == seen {
					continue;
				}
				let asked = if seen == 0 {
					printer.line("Interrupted: requesting graceful shutdown (interrupt again to force exit)");
					Ask::Stop
				} else {
					printer.line("Interrupted again: terminating the world");
					Ask::Kill
				};
				seen = now;
				if inputs.send(Input::Asked(asked)).is_err() {
					return;
				}
			}
		});
	}

	pub fn done() {
		DONE.store(true, Ordering::SeqCst);
	}

	#[cfg(unix)]
	fn catch() {
		extern "C" fn interrupted(_: libc::c_int) {
			INTERRUPTS.fetch_add(1, Ordering::SeqCst);
		}
		unsafe {
			for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
				libc::signal(signal, interrupted as extern "C" fn(libc::c_int) as libc::sighandler_t);
			}
			libc::signal(libc::SIGPIPE, libc::SIG_IGN);
		}
	}

	#[cfg(windows)]
	fn catch() {
		unsafe extern "system" fn interrupted(event: u32) -> i32 {
			INTERRUPTS.fetch_add(1, Ordering::SeqCst);
			const CTRL_CLOSE_EVENT: u32 = 2;
			if event >= CTRL_CLOSE_EVENT {
				for _ in 0..100 {
					if DONE.load(Ordering::SeqCst) {
						break;
					}
					std::thread::sleep(Duration::from_millis(100));
				}
			}
			1
		}
		extern "system" {
			fn SetConsoleCtrlHandler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
		}
		unsafe {
			SetConsoleCtrlHandler(Some(interrupted), 1);
		}
	}
}
