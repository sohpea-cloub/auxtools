//! The debug adapter: DAP requests from the client, turned into the debug
//! server's requests, and everything else that happens while a world runs.
//!
//! The adapter either launches a world, with the debug library loaded into
//! it, or attaches to one already running it, by process id. Either way it
//! connects to the world's local socket, as any number of other clients can,
//! and connects again after each reboot. Without debugging (noDebug) it stays
//! connected for hot reloading, and other launchers can still attach.
//!
//! One thread handles everything here, in the order it happens: the client's
//! messages, the world connecting, the world's code being read (and changed,
//! see `code`), and the world exiting. The client's breakpoints are kept so
//! they can be set again in each world that connects.

use std::{
	collections::{BTreeMap, HashMap},
	io::BufReader,
	path::{Path, PathBuf},
	sync::{mpsc, Arc, Mutex},
	time::{Duration, Instant}
};

use dap_types::*;
use auxtools_cli::{
	byond::{self, Server},
	game::{Game, Launch},
	local::{self, Stream},
	server_types::{self as aux, ContinueKind, InstructionRef, ProcRef, Request as AuxRequest, Response as AuxResponse}
};
use serde::Deserialize;
use serde_json::Value;

use crate::{
	code,
	dap::{self, Output},
	debuggee::Debuggee,
	stddef::{strip_declaration, Stddef}
};

/// The exception filter that pauses on runtime errors
const RUNTIMES: &str = "runtimes";
/// The source reference stddef.dm is shown under
const STDDEF_REFERENCE: i64 = 1;
/// How long to wait for the client to say goodbye once the world has exited
const EXIT_GRACE: Duration = Duration::from_secs(5);
/// The proc the debug library leaves sleeping in a loop
const KEEPALIVE: &str = "/auxtools_keepalive";
/// The file the debug library's own procs are in
const HELPER_FILE: &str = "auxtools_launcher.dm";
/// Where the key for function breakpoints' records is
const FUNCTION_BREAKPOINTS: &str = "<functions>";
/// How often to try the world's socket while it isn't listening
const CONNECT_INTERVAL: Duration = Duration::from_millis(100);

/// What the adapter's thread hears about
pub enum Input {
	/// A message from the client
	Message(String),
	StdinClosed,
	/// The world's debug server took us on
	Connected {
		generation: u64,
		stream: Stream
	},
	ConnectionClosed {
		generation: u64,
		reason: String
	},
	Code(code::Event),
	/// The world the adapter launched exited
	GameExited(Option<i32>),
	/// The process attached to is gone
	ProcessGone,
	/// Time to go, whether the client said goodbye or not
	Quit
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LaunchArguments {
	#[serde(default)]
	no_debug: bool,
	/// The world's environment. Breakpoints, stack traces and hot reloading
	/// need it.
	dme: Option<PathBuf>,
	/// The world to run, if it isn't the .dme's .dmb
	dmb: Option<PathBuf>,
	server: Option<Server>,
	/// Where BYOND is, if it isn't where it's usually installed
	byond_path: Option<PathBuf>,
	/// Build the .dme first
	#[serde(default)]
	build: bool,
	/// DreamMaker's `-D` defines, `NAME` or `NAME=VALUE`, for building and
	/// reading the code
	#[serde(default)]
	defines: Vec<String>,
	/// Swap changed code into the world as files are saved
	#[serde(default = "yes")]
	hot_reload: bool,
	/// What follows the .dmb on the program's command line, if not the
	/// defaults
	args: Option<Vec<String>>,
	#[serde(default)]
	env: HashMap<String, String>,
	/// The debug library, if not the one next to the launcher
	debug_library: Option<PathBuf>
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachArguments {
	/// The process running the world, as a number or a string of one
	process_id: Value,
	/// The world's environment, if it isn't next to the .dmb it runs
	dme: Option<PathBuf>,
	#[serde(default)]
	defines: Vec<String>,
	/// Off by default: whoever launched the world might be reloading it too
	#[serde(default)]
	hot_reload: bool
}

const fn yes() -> bool {
	true
}

type Handled = Result<Value, String>;

/// The world's code, once it's known where it is
struct CodeSettings {
	dme: Option<PathBuf>,
	defines: Vec<String>,
	hot_reload: bool
}

pub struct Adapter {
	out: Arc<Output>,
	inputs: mpsc::Sender<Input>,

	/// Whether the client debugs, rather than only running the world
	debugging: bool,
	/// The world's process, once launched or attached to
	pid: Option<u32>,
	/// The world the adapter launched
	game: Option<Game>,

	/// How to read the code, until the world first connects
	code_settings: Option<CodeSettings>,
	/// The code's thread, while it's listening
	code: Option<mpsc::Sender<code::Command>>,
	hot_reload: bool,
	/// Whether the code has been read (or couldn't be) once
	code_read: bool,
	/// The directory the world's code is in, which the world names its files
	/// relative to
	root: Option<PathBuf>,
	stddef: Option<Stddef>,

	debuggee: Option<Debuggee>,
	generation: u64,
	/// The connected world waits for the code's thread before it's configured
	waiting_for_code: bool,
	/// Whether the client has been told it can set breakpoints
	initialized: bool,
	last_error: Arc<Mutex<String>>,
	/// When the last reload was asked for, to say how long it took
	reload_started: Option<Instant>,

	/// The client's breakpoints by file, with the ids they were given
	breakpoints: BTreeMap<String, Vec<(i64, SourceBreakpoint)>>,
	next_breakpoint_id: i64,
	function_breakpoints: Vec<FunctionBreakpoint>,
	catch_runtimes: bool,
	/// What's set in the world now, by file (or `FUNCTION_BREAKPOINTS`)
	set: HashMap<String, Vec<InstructionRef>>
}

/// Runs the adapter on stdin and stdout until the client leaves
pub fn run() {
	let out = Arc::new(Output::new());
	let (inputs, receiver) = mpsc::channel();

	let stdin_inputs = inputs.clone();
	std::thread::spawn(move || {
		let mut stdin = BufReader::new(std::io::stdin());
		while let Ok(Some(message)) = dap::read_message(&mut stdin) {
			if stdin_inputs.send(Input::Message(message)).is_err() {
				return;
			}
		}
		let _ = stdin_inputs.send(Input::StdinClosed);
	});

	let mut adapter = Adapter {
		out,
		inputs,
		debugging: true,
		pid: None,
		game: None,
		code_settings: None,
		code: None,
		hot_reload: false,
		code_read: false,
		root: None,
		stddef: None,
		debuggee: None,
		generation: 0,
		waiting_for_code: false,
		initialized: false,
		last_error: Arc::new(Mutex::new(String::new())),
		reload_started: None,
		breakpoints: BTreeMap::new(),
		next_breakpoint_id: 1,
		function_breakpoints: vec![],
		catch_runtimes: true,
		set: HashMap::new()
	};

	while let Ok(input) = receiver.recv() {
		if !adapter.handle(input) {
			break;
		}
	}

	if let Some(debuggee) = &mut adapter.debuggee {
		let _ = debuggee.send(&AuxRequest::Disconnect);
	}
	if let Some(game) = adapter.game.take() {
		game.kill();
	}
}

fn arguments<T: for<'de> Deserialize<'de>>(arguments: Option<Value>) -> Result<T, String> {
	serde_json::from_value(arguments.unwrap_or(Value::Null)).map_err(|e| format!("bad arguments: {}", e))
}

fn json(value: impl serde::Serialize) -> Handled {
	serde_json::to_value(value).map_err(|e| e.to_string())
}

impl Adapter {
	/// Returns whether to keep going
	fn handle(&mut self, input: Input) -> bool {
		match input {
			Input::Message(message) => return self.message(&message),
			Input::StdinClosed | Input::Quit => return false,
			Input::Connected { generation, stream } => {
				if generation == self.generation && self.debuggee.is_none() {
					self.connected(stream);
				}
			}
			Input::ConnectionClosed { generation, reason } => self.connection_closed(generation, &reason),
			Input::Code(event) => self.code_event(event),
			Input::GameExited(code) => {
				match code {
					Some(code) => self.out.console(format!("The world exited with code {}", code)),
					None => self.out.console("The world was ended")
				}
				self.game = None;
				self.ended(code.unwrap_or(-1));
			}
			Input::ProcessGone => {
				if self.game.is_none() {
					self.out.console("The world's process has ended");
					self.ended(0);
				}
			}
		}
		true
	}

	/// The world is gone: so is the launcher, once the client is told
	fn ended(&mut self, exit_code: i32) {
		self.pid = None;
		self.debuggee = None;
		self.code = None;
		self.out.event(TerminatedEvent::default());
		self.out.event(ExitedEvent { exitCode: exit_code.into() });

		let inputs = self.inputs.clone();
		std::thread::spawn(move || {
			std::thread::sleep(EXIT_GRACE);
			let _ = inputs.send(Input::Quit);
		});
	}

	fn message(&mut self, message: &str) -> bool {
		let request: RequestMessage = match serde_json::from_str(message) {
			Ok(request) => request,
			Err(e) => {
				self.out.console(format!("A message that isn't a request: {}", e));
				return true;
			}
		};

		let command = request.command.as_str();
		let args = request.arguments;
		let result = match command {
			"initialize" => self.initialize(),
			"launch" => arguments(args).and_then(|args| self.launch(args)),
			"attach" => arguments(args).and_then(|args| self.attach(args)),
			"configurationDone" => self.configuration_done(),
			"setBreakpoints" => arguments(args).and_then(|args| self.set_breakpoints(args)),
			"setFunctionBreakpoints" => arguments(args).and_then(|args| self.set_function_breakpoints(args)),
			"setExceptionBreakpoints" => arguments(args).and_then(|args| self.set_exception_breakpoints(args)),
			"threads" => self.threads(),
			"stackTrace" => arguments(args).and_then(|args| self.stack_trace(args)),
			"scopes" => arguments(args).and_then(|args| self.scopes(args)),
			"variables" => arguments(args).and_then(|args| self.variables(args)),
			"continue" => self.resume(ContinueKind::Continue).and_then(|()| {
				json(ContinueResponse {
					allThreadsContinued: Some(true)
				})
			}),
			"next" => arguments::<NextArguments>(args).and_then(|args| self.step(|stack_id| ContinueKind::StepOver { stack_id }, args.threadId)),
			"stepIn" => arguments::<StepInArguments>(args).and_then(|args| self.step(|stack_id| ContinueKind::StepInto { stack_id }, args.threadId)),
			"stepOut" => arguments::<StepOutArguments>(args).and_then(|args| self.step(|stack_id| ContinueKind::StepOut { stack_id }, args.threadId)),
			"pause" => self.pause(),
			"evaluate" => arguments(args).and_then(|args| self.evaluate(args)),
			"exceptionInfo" => json(ExceptionInfoResponse {
				exceptionId: self.last_error.lock().unwrap().clone(),
				description: None,
				breakMode: ExceptionBreakMode::Always,
				details: None
			}),
			"source" => arguments(args).and_then(|args| self.source(args)),
			"disconnect" => {
				let args: Option<DisconnectArguments> = arguments(args).ok();
				self.disconnect(args.and_then(|args| args.terminateDebuggee));
				self.out.response(request.protocol_message.seq, command, Ok(Value::Null));
				return false;
			}
			_ => Err(format!("{} isn't supported", command))
		};

		self.out.response(request.protocol_message.seq, command, result);
		true
	}

	fn initialize(&mut self) -> Handled {
		json(Capabilities {
			supportsConfigurationDoneRequest: Some(true),
			supportsFunctionBreakpoints: Some(true),
			supportsConditionalBreakpoints: Some(true),
			supportsExceptionInfoRequest: Some(true),
			supportTerminateDebuggee: Some(true),
			exceptionBreakpointFilters: Some(vec![ExceptionBreakpointsFilter {
				filter: RUNTIMES.to_owned(),
				label: "Runtime errors".to_owned(),
				default: Some(true)
			}]),
			..Default::default()
		})
	}

	fn launch(&mut self, args: LaunchArguments) -> Handled {
		if self.pid.is_some() {
			return Err("there's a world already".to_owned());
		}
		let dme = args.dme.clone().or_else(|| args.dmb.as_ref().map(|dmb| dmb.with_extension("dme")));
		let dmb = args
			.dmb
			.clone()
			.or_else(|| args.dme.as_ref().map(|dme| dme.with_extension("dmb")))
			.ok_or("say which world to run: dme or dmb")?;

		let bin = byond::bin_dir(args.byond_path.as_deref())?;
		let server = args.server.unwrap_or_else(Server::default_for_platform);
		let program = bin.join(server.program());
		if !program.is_file() {
			return Err(format!("there's no {} in {}", server.program(), bin.display()));
		}

		if args.build {
			let dme = dme.as_ref().ok_or("there's no .dme to build")?;
			let out = self.out.clone();
			byond::build_with(&bin, dme, &args.defines, move |line| out.console(line))?;
		}
		if !dmb.is_file() {
			return Err(format!("there's no {}", dmb.display()));
		}

		let library = match args.debug_library {
			Some(library) => library,
			None => default_library()?
		};

		self.debugging = !args.no_debug;
		self.code_settings = Some(CodeSettings {
			dme: dme.filter(|dme| dme.is_file()),
			defines: args.defines,
			hot_reload: args.hot_reload
		});

		let default_args = match server {
			Server::DreamDaemon if cfg!(windows) => vec!["-trusted".to_owned(), "-close".to_owned()],
			_ => vec!["-trusted".to_owned()]
		};
		let game_args = args.args.unwrap_or(default_args);
		// While debugging, the world waits for the client's breakpoints
		let launch = Launch {
			program,
			dmb: &dmb,
			args: &game_args,
			env: &args.env,
			debug: Some((library, self.debugging))
		};
		let (out, inputs) = (self.out.clone(), self.inputs.clone());
		let game = Game::start(
			launch,
			move |category, line| out.output(category, line),
			move |code| {
				let _ = inputs.send(Input::GameExited(code));
			}
		)?;
		self.pid = Some(game.pid());
		self.game = Some(game);
		self.started(None);
		Ok(Value::Null)
	}

	fn attach(&mut self, args: AttachArguments) -> Handled {
		if self.pid.is_some() {
			return Err("there's a world already".to_owned());
		}
		let pid = match &args.process_id {
			Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
			Value::String(s) => s.trim().parse().ok(),
			_ => None
		}
		.ok_or("say which world to attach to: processId")?;
		if !local::alive(pid) {
			return Err(format!("there's no process {}", pid));
		}
		// It's there, so there's an answer straight away, if it runs the
		// debug library at all
		let stream = local::connect(pid).map_err(|e| {
			format!(
				"process {} isn't a world with this version of auxtools' debugger in it (protocol {}): {}",
				pid,
				aux::PROTOCOL_VERSION,
				e
			)
		})?;

		self.code_settings = Some(CodeSettings {
			dme: args.dme,
			defines: args.defines,
			hot_reload: args.hot_reload
		});
		self.pid = Some(pid);
		self.out.console(format!("Attaching to process {}", pid));
		self.started(Some(stream));
		Ok(Value::Null)
	}

	/// The world is running: connect to it, unless that's done
	fn started(&mut self, stream: Option<Stream>) {
		// Without debugging, there are no breakpoints to wait for
		if !self.debugging {
			self.initialized = true;
			self.out.event(InitializedEvent);
		}
		match stream {
			// Handled once the client has its answer
			Some(stream) => {
				self.generation += 1;
				let _ = self.inputs.send(Input::Connected {
					generation: self.generation,
					stream
				});
			}
			None => self.connect()
		}
	}

	/// Connects to the world when it listens, in the background
	fn connect(&mut self) {
		let Some(pid) = self.pid else {
			return;
		};
		self.generation += 1;
		let generation = self.generation;
		let inputs = self.inputs.clone();
		let launched = self.game.is_some();
		std::thread::Builder::new()
			.name("connect".to_owned())
			.spawn(move || loop {
				// The world isn't listening while it loads, or reboots
				if let Ok(stream) = local::connect(pid) {
					let _ = inputs.send(Input::Connected { generation, stream });
					return;
				}
				// The launched world's exit is heard about anyway
				if !launched && !local::alive(pid) {
					let _ = inputs.send(Input::ProcessGone);
					return;
				}
				std::thread::sleep(CONNECT_INTERVAL);
			})
			.expect("couldn't start a thread");
	}

	fn connected(&mut self, stream: Stream) {
		let debuggee = Debuggee::new(
			stream,
			self.generation,
			self.debugging,
			self.out.clone(),
			self.inputs.clone(),
			self.last_error.clone()
		);
		match debuggee {
			Ok(debuggee) => self.debuggee = Some(debuggee),
			Err(e) => {
				self.out.console(format!("Couldn't talk to the world: {}", e));
				return;
			}
		}
		self.set.clear();

		// It hangs up straight away if it's on its way to a reboot
		let byond = match self.request(&AuxRequest::ByondVersion) {
			Ok(AuxResponse::ByondVersion { version, build }) => (version, build),
			_ => return
		};
		// Here to run the world and reload it: a pause another client makes
		// isn't this one's to end
		if !self.debugging {
			let _ = self.ack(&AuxRequest::Watcher);
		}
		if self.stddef.is_none() {
			if let Ok(AuxResponse::StdDef(Some(text))) = self.request(&AuxRequest::StdDef) {
				self.stddef = Some(Stddef::new(text));
			}
		}

		match self.code_settings.take() {
			Some(settings) => self.start_code(settings),
			None => self.out.console("The world restarted")
		}
		self.waiting_for_code = match &self.code {
			// A world that restarted waits for its reloads again
			Some(code) => code.send(code::Command::Connected { byond }).is_ok() && self.code_read,
			None => false
		};
		if !self.waiting_for_code {
			self.ready();
		}
	}

	/// Finds the world's code, and starts reading it to hot reload it. The
	/// world says where its procs are itself, so it's read only for that.
	fn start_code(&mut self, settings: CodeSettings) {
		let dme = settings.dme.or_else(|| {
			// The world's command line says which .dmb it runs
			let AuxResponse::WorldInfo { args, cwd } = self.request(&AuxRequest::WorldInfo).ok()? else {
				return None;
			};
			Some(local::world_dmb(&args, &cwd)?.with_extension("dme"))
		});
		self.root = dme.as_ref().and_then(|dme| std::path::absolute(dme).ok()?.parent().map(Path::to_owned));
		self.hot_reload = settings.hot_reload;
		if !settings.hot_reload {
			self.code_read = true;
			return;
		}
		let Some(dme) = dme.filter(|dme| dme.is_file()) else {
			self.out.console("Couldn't find the world's code (dme), so hot reloading won't work");
			self.code_read = true;
			return;
		};

		let options = code::Options {
			dme,
			defines: settings.defines.iter().map(|define| code::parse_define(define)).collect(),
			hot_reload: settings.hot_reload
		};
		self.code = Some(code::start(options, self.out.clone(), self.inputs.clone()));
	}

	fn code_event(&mut self, event: code::Event) {
		match event {
			code::Event::Changed => {
				// Reloaded code moves lines: the breakpoints go where the
				// client has them now
				if self.code_read && !self.waiting_for_code && self.debuggee.is_some() && self.debugging {
					self.reapply_breakpoints();
				}
				self.code_read = true;
			}
			code::Event::Failed(e) => {
				self.out.console(format!("Couldn't read the code, so hot reloading won't work: {}", e));
				self.code_read = true;
				self.code = None;
				if self.waiting_for_code {
					self.waiting_for_code = false;
					self.ready();
				}
			}
			code::Event::Ready => {
				if self.waiting_for_code {
					self.waiting_for_code = false;
					self.ready();
				}
			}
			code::Event::Reload { request, summary } => {
				let applied = self.reload(*request, &summary);
				if let Some(code) = &self.code {
					let _ = code.send(code::Command::Reloaded(applied));
				}
			}
		}
	}

	/// Swaps code into the world. Whether it was.
	fn reload(&mut self, request: aux::HotReloadRequest, summary: &str) -> bool {
		self.reload_started = Some(Instant::now());
		let result = match self.request(&AuxRequest::HotReload(request)) {
			Ok(AuxResponse::HotReload(result)) => result,
			Ok(other) => {
				self.out.console(format!("Hot reload: not reloaded: {}", unexpected(other)));
				return false;
			}
			Err(e) => {
				self.out.console(format!("Hot reload: not reloaded: {}", e));
				return false;
			}
		};

		let took = self.reload_started.take().map_or(0, |started| started.elapsed().as_millis());
		if result.applied {
			self.out.console(format!("Hot reload: {} in {} ms", summary, took));
		} else {
			self.out.console("Hot reload: not reloaded:");
		}
		for e in &result.errors {
			self.out.console(format!("    {}", e));
		}
		for w in &result.warnings {
			self.out.console(format!("    warning: {}", w));
		}
		result.applied
	}

	/// The world has the code it should: give it the breakpoints and let it run
	fn ready(&mut self) {
		if !self.initialized {
			// The client sets its breakpoints, then says it's done
			self.initialized = true;
			self.out.event(InitializedEvent);
			return;
		}
		self.configure();
	}

	/// Gives the world the client's breakpoints and lets it run
	fn configure(&mut self) {
		if self.debugging {
			let _ = self.catch_runtimes();
			let files: Vec<String> = self.breakpoints.keys().cloned().collect();
			for file in files {
				let _ = self.apply_breakpoints(&file);
			}
			let _ = self.apply_function_breakpoints();
		}
		let _ = self.ack(&AuxRequest::Configured);
		self.catch_up_on_pause();
	}

	/// Tells the client if the world was paused before this connected
	fn catch_up_on_pause(&mut self) {
		if self.debugging {
			if let Some(debuggee) = &mut self.debuggee {
				let _ = debuggee.catch_up_on_pause();
			}
		}
	}

	/// Sets every breakpoint again, telling the client where they are now
	fn reapply_breakpoints(&mut self) {
		let files: Vec<String> = self.breakpoints.keys().cloned().collect();
		for file in files {
			for breakpoint in self.apply_breakpoints(&file) {
				self.out.event(dap::BreakpointEvent {
					reason: "changed".to_owned(),
					breakpoint
				});
			}
		}
	}

	fn connection_closed(&mut self, generation: u64, reason: &str) {
		if self.debuggee.as_ref().is_none_or(|d| d.generation != generation) {
			return;
		}
		self.debuggee = None;
		self.waiting_for_code = false;
		self.set.clear();

		if self.pid.is_some() {
			self.out
				.console(format!("The world's debugger disconnected ({}); connecting again when it's back", reason));
			self.connect();
		}
	}

	fn disconnect(&mut self, terminate: Option<bool>) {
		// A paused world stays paused for the other clients that debug it,
		// and carries on when this was the last
		if let Some(debuggee) = &mut self.debuggee {
			debuggee.leave();
		}
		self.debuggee = None;
		// A world this launcher started has its output read here, and can't
		// outlive it
		if terminate == Some(false) && self.game.is_some() {
			self.out
				.console("The world was started by this session, and ends with it (attach to a world to leave it running)");
		}
		// A world that was attached to carries on unless asked otherwise
		if terminate.unwrap_or(false) || self.game.is_some() {
			if let Some(game) = self.game.take() {
				self.out.console(format!("Ending the world (process {})", game.pid()));
				game.kill();
			} else if let Some(pid) = self.pid {
				self.out.console(format!("Ending the world (process {})", pid));
				local::kill(pid);
			}
		}
		// Stop trying to connect
		self.pid = None;
		self.generation += 1;
	}

	fn debuggee(&mut self) -> Result<&mut Debuggee, String> {
		self.debuggee.as_mut().ok_or_else(|| "the world isn't connected".to_owned())
	}

	fn request(&mut self, request: &AuxRequest) -> Result<AuxResponse, String> {
		self.debuggee()?.request(request)
	}

	/// A request the server answers with `Ack`
	fn ack(&mut self, request: &AuxRequest) -> Result<(), String> {
		match self.request(request)? {
			AuxResponse::Ack => Ok(()),
			other => Err(unexpected(other))
		}
	}

	fn configuration_done(&mut self) -> Handled {
		if self.debugging && self.debuggee.is_some() {
			// A client that said nothing about runtime errors has the
			// filter's default, here as in the worlds after a reboot
			self.catch_runtimes()?;
			self.ack(&AuxRequest::Configured)?;
		}
		self.catch_up_on_pause();
		Ok(Value::Null)
	}

	fn set_breakpoints(&mut self, args: SetBreakpointsArguments) -> Handled {
		let path = args.source.path.ok_or("the breakpoints' source has no path")?;
		if args.sourceModified.unwrap_or(false) {
			return Err("the file has changes that haven't been saved".to_owned());
		}
		let mut wanted = vec![];
		for breakpoint in args.breakpoints.unwrap_or_default() {
			wanted.push((self.next_breakpoint_id, breakpoint));
			self.next_breakpoint_id += 1;
		}
		self.breakpoints.insert(path.clone(), wanted);
		let breakpoints = self.apply_breakpoints(&path);
		json(SetBreakpointsResponse { breakpoints })
	}

	/// What the world calls a file of its code: its path from the code's
	/// directory. Otherwise the world matches the end of the path.
	fn world_file_name(&self, path: &str) -> String {
		file_name_in(self.root.as_deref(), path)
	}

	/// Sets a file's breakpoints in the world, in place of the ones it had
	fn apply_breakpoints(&mut self, path: &str) -> Vec<Breakpoint> {
		self.unset(path);
		let wanted: Vec<(i64, i64, Option<String>)> = self.breakpoints[path]
			.iter()
			.map(|(number, b)| (*number, b.line, b.condition.clone()))
			.collect();

		let mut breakpoints = vec![];
		let mut set = vec![];
		for (number, line, condition) in wanted {
			let unverified = |message: &str| Breakpoint {
				id: Some(number),
				verified: false,
				message: Some(message.to_owned()),
				line: Some(line),
				..Default::default()
			};

			if !self.debugging {
				breakpoints.push(unverified("the world runs without debugging"));
				continue;
			}
			if self.debuggee.is_none() {
				breakpoints.push(unverified("the world isn't running"));
				continue;
			}

			// The world knows which of its procs have code on the line
			let instructions = match self.request(&AuxRequest::Locate {
				file: self.world_file_name(path),
				line: line as u32
			}) {
				Ok(AuxResponse::Locate { instructions }) if instructions.is_empty() => {
					breakpoints.push(unverified("no code on or after this line in the world's code"));
					continue;
				}
				Ok(AuxResponse::Locate { instructions }) => instructions,
				Ok(other) => {
					breakpoints.push(unverified(&unexpected(other)));
					continue;
				}
				Err(e) => {
					breakpoints.push(unverified(&e));
					continue;
				}
			};

			let mut result = None;
			for instruction in instructions {
				match self.set_breakpoint(&instruction, condition.clone()) {
					Ok(Some(set_line)) => {
						result.get_or_insert(Ok(set_line));
					}
					Ok(None) => {}
					Err(e) => {
						result.get_or_insert(Err(e));
					}
				}
				set.push(instruction);
			}
			breakpoints.push(match result {
				Some(Ok(set_line)) => Breakpoint {
					id: Some(number),
					verified: true,
					line: Some(set_line.unwrap_or(line as u32).into()),
					..Default::default()
				},
				Some(Err(e)) => unverified(&e),
				None => unverified("the world couldn't set it")
			});
		}
		self.set.insert(path.to_owned(), set);
		breakpoints
	}

	/// `Some(line)` when it's set
	fn set_breakpoint(&mut self, instruction: &InstructionRef, condition: Option<String>) -> Result<Option<Option<u32>>, String> {
		match self.request(&AuxRequest::BreakpointSet {
			instruction: instruction.clone(),
			condition
		})? {
			AuxResponse::BreakpointSet {
				result: aux::BreakpointSetResult::Success { line }
			} => Ok(Some(line)),
			AuxResponse::BreakpointSet {
				result: aux::BreakpointSetResult::Failed
			} => Ok(None),
			AuxResponse::BreakpointSet {
				result: aux::BreakpointSetResult::Refused { reason }
			} => Err(reason),
			other => Err(unexpected(other))
		}
	}

	/// Takes away the breakpoints set under `key`
	fn unset(&mut self, key: &str) {
		for instruction in self.set.remove(key).unwrap_or_default() {
			let _ = self.request(&AuxRequest::BreakpointUnset { instruction });
		}
	}

	fn set_function_breakpoints(&mut self, args: SetFunctionBreakpointsArguments) -> Handled {
		self.function_breakpoints = args.breakpoints;
		let breakpoints = self.apply_function_breakpoints();
		json(SetFunctionBreakpointsResponse { breakpoints })
	}

	/// Breakpoints at the start of procs: `/mob/proc/Login`, or `#1` after it
	/// for its second definition
	fn apply_function_breakpoints(&mut self) -> Vec<Breakpoint> {
		self.unset(FUNCTION_BREAKPOINTS);
		let wanted: Vec<(String, Option<String>)> = self.function_breakpoints.iter().map(|b| (b.name.clone(), b.condition.clone())).collect();

		let mut breakpoints = vec![];
		let mut set = vec![];
		for (name, condition) in wanted {
			if !self.debugging || self.debuggee.is_none() {
				breakpoints.push(Breakpoint {
					verified: false,
					message: Some("the world isn't being debugged".to_owned()),
					..Default::default()
				});
				continue;
			}
			let instruction = InstructionRef {
				proc: named_proc(&name),
				offset: 0
			};
			breakpoints.push(match self.set_breakpoint(&instruction, condition) {
				Ok(Some(line)) => Breakpoint {
					verified: true,
					line: line.map(i64::from),
					..Default::default()
				},
				Ok(None) => Breakpoint {
					verified: false,
					message: Some(format!("there's no {}", name)),
					..Default::default()
				},
				Err(e) => Breakpoint {
					verified: false,
					message: Some(e),
					..Default::default()
				}
			});
			set.push(instruction);
		}
		self.set.insert(FUNCTION_BREAKPOINTS.to_owned(), set);
		breakpoints
	}

	fn set_exception_breakpoints(&mut self, args: SetExceptionBreakpointsArguments) -> Handled {
		self.catch_runtimes = args.filters.iter().any(|filter| filter == RUNTIMES);
		if self.debugging && self.debuggee.is_some() {
			self.catch_runtimes()?;
		}
		Ok(Value::Null)
	}

	/// Tells the world whether to pause on runtime errors (it doesn't answer)
	fn catch_runtimes(&mut self) -> Result<(), String> {
		let should_catch = self.catch_runtimes;
		self.debuggee()?.send(&AuxRequest::CatchRuntimes { should_catch })
	}

	fn threads(&mut self) -> Handled {
		let mut threads = vec![];
		if self.debuggee.is_some() {
			match self.request(&AuxRequest::Stacks)? {
				AuxResponse::Stacks { stacks } => {
					threads.extend(
						stacks
							.into_iter()
							.filter(|stack| strip_declaration(&stack.name) != KEEPALIVE)
							.map(|stack| Thread {
								id: stack.id.into(),
								name: stack.name
							})
					);
				}
				other => return Err(unexpected(other))
			}
		}

		// The client only asks to pause a thread it knows of
		if threads.is_empty() {
			threads.push(Thread {
				id: 0,
				name: "Main".to_owned()
			});
		}
		json(ThreadsResponse { threads })
	}

	fn stack_trace(&mut self, args: StackTraceArguments) -> Handled {
		let (frames, total) = match self.request(&AuxRequest::StackFrames {
			stack_id: args.threadId as u32,
			start_frame: args.startFrame.map(|x| x as u32),
			count: args.levels.filter(|&levels| levels > 0).map(|x| x as u32)
		})? {
			AuxResponse::StackFrames { frames, total_count } => (frames, total_count),
			other => return Err(unexpected(other))
		};

		let mut stack_frames = vec![];
		for (i, frame) in frames.into_iter().enumerate() {
			let proc = &frame.instruction.proc;
			let mut dap_frame = StackFrame {
				id: frame.id.into(),
				name: proc.path.clone(),
				instructionPointerReference: Some(format!("{}#{}@{}", proc.path, proc.override_id, frame.instruction.offset)),
				// Has to be more than 0 for the client to show an exception
				column: if i == 0 { 1 } else { 0 },
				..Default::default()
			};

			// The world says which file the proc is in, but for stddef.dm's
			match &frame.file {
				// One of the debug library's own
				Some(file) if file == HELPER_FILE => dap_frame.presentationHint = Some(StackFramePresentationHint::Subtle),
				Some(file) => {
					let file = file.replace('\\', "/");
					let path = match &self.root {
						Some(root) => root.join(&file),
						None => PathBuf::from(&file)
					};
					dap_frame.source = Some(Source {
						name: path.file_name().map(|name| name.to_string_lossy().into_owned()),
						path: Some(path.to_string_lossy().into_owned()),
						..Default::default()
					});
				}
				None => match self.stddef.as_ref().and_then(|stddef| stddef.line(&proc.path, proc.override_id)) {
					Some(line) => {
						dap_frame.source = Some(Source {
							name: Some("stddef.dm".to_owned()),
							sourceReference: Some(STDDEF_REFERENCE),
							..Default::default()
						});
						dap_frame.line = line.into();
					}
					None => dap_frame.presentationHint = Some(StackFramePresentationHint::Subtle)
				}
			}
			if let Some(line) = frame.line {
				dap_frame.line = line.into();
			}
			stack_frames.push(dap_frame);
		}

		json(StackTraceResponse {
			stackFrames: stack_frames,
			totalFrames: Some(total.into())
		})
	}

	fn scopes(&mut self, args: ScopesArguments) -> Handled {
		let (arguments, locals, globals) = match self.request(&AuxRequest::Scopes {
			frame_id: args.frameId as u32
		})? {
			AuxResponse::Scopes { arguments, locals, globals } => (arguments, locals, globals),
			other => return Err(unexpected(other))
		};
		let scopes = [("Locals", locals), ("Arguments", arguments), ("Globals", globals)]
			.into_iter()
			.filter_map(|(name, vars)| {
				vars.map(|vars| Scope {
					name: name.to_owned(),
					variablesReference: vars.0.into(),
					..Default::default()
				})
			})
			.collect();
		json(ScopesResponse { scopes })
	}

	fn variables(&mut self, args: VariablesArguments) -> Handled {
		let vars = match self.request(&AuxRequest::Variables {
			vars: aux::VariablesRef(args.variablesReference as i32)
		})? {
			AuxResponse::Variables { vars } => vars,
			other => return Err(unexpected(other))
		};

		// The client needs names to be different
		let mut seen: HashMap<String, u32> = HashMap::new();
		let variables = vars
			.into_iter()
			.map(|var| {
				let count = seen.entry(var.name.clone()).or_default();
				let name = if *count == 0 { var.name } else { format!("{} #{}", var.name, count) };
				*count += 1;
				Variable {
					name,
					value: var.value,
					variablesReference: var.variables.map_or(0, |vars| vars.0.into()),
					..Default::default()
				}
			})
			.collect();
		json(VariablesResponse { variables })
	}

	fn pause(&mut self) -> Handled {
		// The client isn't told of stops, so it would have nothing to let
		// the world carry on from
		if !self.debugging {
			return Err("the world runs without debugging".to_owned());
		}
		self.ack(&AuxRequest::Pause).map(|()| Value::Null)
	}

	fn resume(&mut self, kind: ContinueKind) -> Result<(), String> {
		self.ack(&AuxRequest::Continue { kind })
	}

	fn step(&mut self, kind: impl Fn(u32) -> ContinueKind, thread: i64) -> Handled {
		self.resume(kind(thread as u32)).map(|()| Value::Null)
	}

	fn evaluate(&mut self, args: EvaluateArguments) -> Handled {
		match self.request(&AuxRequest::Eval {
			frame_id: args.frameId.map(|id| id as u32),
			command: args.expression,
			context: args.context
		})? {
			AuxResponse::Eval(Ok(result)) => json(EvaluateResponse {
				result: result.value,
				variablesReference: result.variables.map_or(0, |vars| vars.0.into()),
				..Default::default()
			}),
			// The client shows it where the value would be
			AuxResponse::Eval(Err(why)) => Err(why),
			other => Err(unexpected(other))
		}
	}

	fn source(&mut self, args: SourceArguments) -> Handled {
		let reference = args.source.and_then(|source| source.sourceReference).unwrap_or(args.sourceReference);
		match (reference, &self.stddef) {
			(STDDEF_REFERENCE, Some(stddef)) => json(SourceResponse {
				content: stddef.text.clone(),
				mimeType: None
			}),
			_ => Err("there's no such source".to_owned())
		}
	}
}

/// The proc a function breakpoint names: its path, and after a `#` which of
/// its definitions, the first unless it says
fn named_proc(name: &str) -> ProcRef {
	let (path, override_id) = match name.split_once('#') {
		Some((path, id)) => (path.trim().to_owned(), id.trim().parse().unwrap_or(0)),
		None => (name.trim().to_owned(), 0)
	};
	ProcRef { path, override_id }
}

/// A file's name as the world has it: its path from `root`, the directory of
/// the world's code, with forward slashes. A file that isn't under it keeps
/// its path, which the world matches by its end.
fn file_name_in(root: Option<&Path>, path: &str) -> String {
	let path = Path::new(path);
	match root.and_then(|root| path.strip_prefix(root).ok()) {
		Some(relative) => relative.to_string_lossy().replace('\\', "/"),
		None => path.to_string_lossy().into_owned()
	}
}

fn unexpected(response: AuxResponse) -> String {
	format!("the world answered something else: {:?}", response)
}

/// The debug library, next to the launcher
fn default_library() -> Result<PathBuf, String> {
	let name = if cfg!(windows) { "debug_server.dll" } else { "libdebug_server.so" };
	let exe = std::env::current_exe().map_err(|e| e.to_string())?;
	let library = exe.parent().map(|dir| dir.join(name)).unwrap_or_else(|| Path::new(name).to_owned());
	if !library.is_file() {
		return Err(format!("there's no debug library at {} (debugLibrary)", library.display()));
	}
	Ok(library)
}

#[cfg(test)]
mod tests {
	use std::path::Path;

	use super::{file_name_in, named_proc};

	#[test]
	fn a_function_breakpoint_names_a_proc() {
		let proc = named_proc("/mob/proc/Login");
		assert_eq!((proc.path.as_str(), proc.override_id), ("/mob/proc/Login", 0));
		let proc = named_proc("  /proc/leaf ");
		assert_eq!((proc.path.as_str(), proc.override_id), ("/proc/leaf", 0));
	}

	#[test]
	fn a_function_breakpoint_names_a_definition() {
		let proc = named_proc("/mob/Login#2");
		assert_eq!((proc.path.as_str(), proc.override_id), ("/mob/Login", 2));
		let proc = named_proc("/mob/Login # 1 ");
		assert_eq!((proc.path.as_str(), proc.override_id), ("/mob/Login", 1));
	}

	#[test]
	fn a_definition_that_isnt_a_number_is_the_first() {
		let proc = named_proc("/mob/Login#last");
		assert_eq!((proc.path.as_str(), proc.override_id), ("/mob/Login", 0));
	}

	#[test]
	fn a_file_under_the_code_is_named_from_it() {
		let root = Path::new("code").join("world");
		let file = root.join("mobs").join("living.dm");
		assert_eq!(file_name_in(Some(&root), &file.to_string_lossy()), "mobs/living.dm");
		let file = root.join("world.dm");
		assert_eq!(file_name_in(Some(&root), &file.to_string_lossy()), "world.dm");
	}

	#[test]
	fn a_file_elsewhere_keeps_its_path() {
		let root = Path::new("code").join("world");
		let file = Path::new("other").join("living.dm");
		assert_eq!(file_name_in(Some(&root), &file.to_string_lossy()), file.to_string_lossy());
		assert_eq!(file_name_in(None, "living.dm"), "living.dm");
	}
}
