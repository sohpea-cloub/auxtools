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
	debuggee::{Debuggee, LastError, SetBreakpoint, SetBreakpoints},
	stddef::{strip_declaration, Stddef}
};

/// The exception filter that pauses on runtime errors nothing catches
const RUNTIMES: &str = "runtimes";
/// The exception filter that pauses on runtime errors inside a `try`
const CAUGHT: &str = "caught";
/// The source reference stddef.dm is shown under
const STDDEF_REFERENCE: i64 = 1;
/// How long to wait for the client to say goodbye once the world has exited
const EXIT_GRACE: Duration = Duration::from_secs(5);
/// How long a world that's asked to end has to do it in
const TERMINATE_GRACE: Duration = Duration::from_secs(5);
/// The proc the debug library leaves sleeping in a loop
const KEEPALIVE: &str = "/auxtools_keepalive";
/// The file the debug library's own procs are in
const HELPER_FILE: &str = "auxtools_launcher.dm";
/// Where the key for function breakpoints' records is
const FUNCTION_BREAKPOINTS: &str = "<functions>";
/// The things that take a while, by the ids the client is shown them under
const BUILDING: &str = "building";
const STARTING: &str = "starting";
const READING: &str = "reading";
const RESTARTING: &str = "restarting";
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
	/// Another client hot reloaded the world
	CodeChanged {
		generation: u64
	},
	/// A breakpoint with a message was hit
	Logged {
		message: String,
		file: Option<String>,
		line: Option<u32>
	},
	/// The world the adapter launched exited
	GameExited(Option<i32>),
	/// The process attached to is gone
	ProcessGone,
	/// The world asked to end has had its time. `process` is the one asked.
	TerminateOverdue {
		process: u32
	},
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
	debug_library: Option<PathBuf>,
	/// How many sleeping procs the thread list has
	sleeping_threads: Option<u32>
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
	hot_reload: bool,
	/// How many sleeping procs the thread list has
	sleeping_threads: Option<u32>
}

/// What the client says of itself in `initialize`, of what matters here
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ClientAbilities {
	/// It shows `progressStart` and `progressEnd`
	#[serde(default)]
	supports_progress_reporting: bool,
	/// It asks again for what an `invalidated` event names
	#[serde(default)]
	supports_invalidated_event: bool
}

/// What `completions` takes
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompletionsArguments {
	frame_id: Option<i64>,
	text: String,
	column: i64
}

/// What `breakpointLocations` takes (dap-types has none of it)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BreakpointLocationsArguments {
	source: Source,
	line: i64,
	end_line: Option<i64>
}

/// What `setVariable` takes (dap-types has none of it)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetVariableArguments {
	variables_reference: i64,
	name: String,
	value: String,
	format: Option<ValueFormat>
}

/// What `setExpression` takes
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetExpressionArguments {
	expression: String,
	value: String,
	frame_id: Option<i64>,
	format: Option<ValueFormat>
}

/// What `setExceptionBreakpoints` takes: the filters ticked, with their
/// conditions if they have any (dap-types knows the first only)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExceptionBreakpointsArguments {
	#[serde(default)]
	filters: Vec<String>,
	#[serde(default)]
	filter_options: Vec<ExceptionFilterOption>
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExceptionFilterOption {
	filter_id: String,
	condition: Option<String>
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
	/// The client asked for the world to end: its debugger isn't coming back
	ending: bool,
	/// The connected world waits for the code's thread before it's configured
	waiting_for_code: bool,
	/// Whether the client has been told it can set breakpoints
	initialized: bool,
	last_error: Arc<Mutex<LastError>>,
	/// When the last reload was asked for, to say how long it took
	reload_started: Option<Instant>,

	/// The client's breakpoints by file, with the ids they were given
	breakpoints: BTreeMap<String, Vec<(i64, SourceBreakpoint)>>,
	/// What the client said it can do with what it's told
	client: ClientAbilities,
	/// The things that are taking a while, which the client is showing
	progress: Vec<&'static str>,
	/// `root`, for the thread that passes the world's output on
	output_root: Arc<Mutex<Option<PathBuf>>>,
	/// The text each file's breakpoints were given for: their lines are
	/// lines of it. `None` for a file the client hasn't saved yet, or that
	/// can't be read.
	breakpoint_texts: HashMap<String, Option<Vec<String>>>,
	next_breakpoint_id: i64,
	function_breakpoints: Vec<FunctionBreakpoint>,
	/// Which runtime errors to pause on: of those nothing catches, and of
	/// those inside a `try`
	uncaught: Option<aux::RuntimeFilter>,
	caught: Option<aux::RuntimeFilter>,
	/// What's set in the world now, by file (or `FUNCTION_BREAKPOINTS`)
	set: SetBreakpoints,
	/// How many sleeping procs the thread list has
	sleeping_threads: u32,
	/// The client has been told the thread list doesn't have them all
	said_more_sleep: bool
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
		ending: false,
		waiting_for_code: false,
		initialized: false,
		last_error: Arc::new(Mutex::new(LastError::default())),
		reload_started: None,
		client: ClientAbilities::default(),
		progress: vec![],
		output_root: Arc::new(Mutex::new(None)),
		breakpoints: BTreeMap::new(),
		breakpoint_texts: HashMap::new(),
		next_breakpoint_id: 1,
		function_breakpoints: vec![],
		uncaught: Some(aux::RuntimeFilter::default()),
		caught: None,
		set: SetBreakpoints::default(),
		sleeping_threads: aux::DEFAULT_SLEEPING_STACKS,
		said_more_sleep: false
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
			Input::Logged { message, file, line } => {
				// With where it's from, for the client to make a link of
				let source = file.map(|file| {
					let file = file.replace('\\', "/");
					let path = match &self.root {
						Some(root) => root.join(&file),
						None => PathBuf::from(&file)
					};
					Source {
						name: path.file_name().map(|name| name.to_string_lossy().into_owned()),
						path: Some(path.to_string_lossy().into_owned()),
						..Default::default()
					}
				});
				self.out.event(OutputEvent {
					output: message + "\n",
					category: Some("console".to_owned()),
					line: source.as_ref().and(line).map(i64::from),
					source,
					..Default::default()
				});
			}
			Input::CodeChanged { generation } => {
				if self.debuggee.as_ref().is_some_and(|d| d.generation == generation) {
					self.reapply_breakpoints();
				}
			}
			Input::GameExited(code) => {
				match code {
					Some(code) => self.out.console(format!("The world exited with code {}", code)),
					None => self.out.console("The world was ended")
				}
				self.game = None;
				self.ended(code.unwrap_or(-1));
			}
			Input::TerminateOverdue { process } => {
				if self.ending && self.world_process() == Some(process) {
					self.out.console(format!(
						"The world hasn't ended in {} seconds: ending its process ({})",
						TERMINATE_GRACE.as_secs(),
						process
					));
					self.kill_world();
				}
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
		for id in std::mem::take(&mut self.progress) {
			self.out.event(dap::ProgressEnd { progressId: id.to_owned() });
		}
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
			"initialize" => {
				self.client = arguments(args).unwrap_or_default();
				self.initialize()
			}
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
			"completions" => arguments(args).and_then(|args| self.completions(args)),
			"setVariable" => arguments(args).and_then(|args| self.set_variable(args)),
			"setExpression" => arguments(args).and_then(|args| self.set_expression(args)),
			"exceptionInfo" => self.exception_info(),
			"source" => arguments(args).and_then(|args| self.source(args)),
			"breakpointLocations" => arguments(args).and_then(|args| self.breakpoint_locations(args)),
			"terminate" => self.terminate(),
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
		let mut capabilities = json(Capabilities {
			supportsConfigurationDoneRequest: Some(true),
			supportsFunctionBreakpoints: Some(true),
			supportsConditionalBreakpoints: Some(true),
			supportsHitConditionalBreakpoints: Some(true),
			supportsLogPoints: Some(true),
			supportsValueFormattingOptions: Some(true),
			supportsEvaluateForHovers: Some(true),
			supportsSetVariable: Some(true),
			supportsSetExpression: Some(true),
			supportsExceptionInfoRequest: Some(true),
			supportTerminateDebuggee: Some(true),
			supportsTerminateRequest: Some(true),
			supportsBreakpointLocationsRequest: Some(true),
			..Default::default()
		})?;
		// What dap-types has no fields for
		let condition = "Text the error's message has to have, like \"bounds, null\" for either, or mustn't, like \"!libdreamluau\"";
		capabilities["supportsExceptionFilterOptions"] = true.into();
		capabilities["supportsCompletionsRequest"] = true.into();
		capabilities["completionTriggerCharacters"] = serde_json::json!([".", ":"]);
		capabilities["exceptionBreakpointFilters"] = serde_json::json!([
			{
				"filter": RUNTIMES,
				"label": "Runtime errors",
				"description": "Pause on runtime errors, and thrown exceptions, that nothing catches",
				"default": true,
				"supportsCondition": true,
				"conditionDescription": condition
			},
			{
				"filter": CAUGHT,
				"label": "Caught runtime errors",
				"description": "Pause on runtime errors and thrown exceptions inside a try block too",
				"default": false,
				"supportsCondition": true,
				"conditionDescription": condition
			}
		]);
		Ok(capabilities)
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
			self.progress_start(BUILDING, format!("Building {}", dme.file_name().unwrap_or_default().to_string_lossy()));
			let built = byond::build_with(&bin, dme, &args.defines, move |line| out.console(line));
			self.progress_end(BUILDING);
			built?;
		}
		if !dmb.is_file() {
			return Err(format!("there's no {}", dmb.display()));
		}

		let library = match args.debug_library {
			Some(library) => library,
			None => default_library()?
		};

		self.debugging = !args.no_debug;
		self.sleeping_threads = args.sleeping_threads.unwrap_or(aux::DEFAULT_SLEEPING_STACKS);
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
		let (out, inputs, root) = (self.out.clone(), self.inputs.clone(), self.output_root.clone());
		let game = Game::start(
			launch,
			// Where BYOND says a runtime error was is passed on as a place,
			// which a client makes a link of
			move |category, line| match runtime_place(&line) {
				Some((file, number)) => {
					let path = match &*root.lock().unwrap() {
						Some(root) => root.join(file),
						None => PathBuf::from(file)
					};
					out.event(OutputEvent {
						category: Some(category.to_owned()),
						source: Some(Source {
							name: path.file_name().map(|name| name.to_string_lossy().into_owned()),
							path: Some(path.to_string_lossy().into_owned()),
							..Default::default()
						}),
						line: Some(number),
						output: line,
						..Default::default()
					});
				}
				None => out.output(category, line)
			},
			move |code| {
				let _ = inputs.send(Input::GameExited(code));
			}
		)?;
		self.pid = Some(game.pid());
		self.game = Some(game);
		self.out.event(dap::ProcessEvent {
			name: dmb.to_string_lossy().into_owned(),
			systemProcessId: self.pid.unwrap_or_default(),
			isLocalProcess: true,
			startMethod: "launch".to_owned()
		});
		self.progress_start(STARTING, "Starting the world".to_owned());
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

		self.sleeping_threads = args.sleeping_threads.unwrap_or(aux::DEFAULT_SLEEPING_STACKS);
		self.code_settings = Some(CodeSettings {
			dme: args.dme,
			defines: args.defines,
			hot_reload: args.hot_reload
		});
		self.pid = Some(pid);
		self.out.console(format!("Attaching to process {}", pid));
		self.out.event(dap::ProcessEvent {
			name: format!("process {}", pid),
			systemProcessId: pid,
			isLocalProcess: true,
			startMethod: "attach".to_owned()
		});
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

	/// Has the client show that something is taking a while, if it shows
	/// such things
	fn progress_start(&mut self, id: &'static str, title: String) {
		if self.client.supports_progress_reporting && !self.progress.contains(&id) {
			self.progress.push(id);
			self.out.event(dap::ProgressStart {
				progressId: id.to_owned(),
				title
			});
		}
	}

	fn progress_end(&mut self, id: &'static str) {
		if let Some(at) = self.progress.iter().position(|shown| *shown == id) {
			self.progress.remove(at);
			self.out.event(dap::ProgressEnd { progressId: id.to_owned() });
		}
	}

	fn connected(&mut self, stream: Stream) {
		self.progress_end(STARTING);
		self.progress_end(RESTARTING);
		let debuggee = Debuggee::new(
			stream,
			self.generation,
			self.debugging,
			self.out.clone(),
			self.inputs.clone(),
			self.last_error.clone(),
			self.set.clone()
		);
		match debuggee {
			Ok(debuggee) => self.debuggee = Some(debuggee),
			Err(e) => {
				self.out.console(format!("Couldn't talk to the world: {}", e));
				return;
			}
		}
		self.set.lock().unwrap().clear();

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
		self.output_root.lock().unwrap().clone_from(&self.root);
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
		self.progress_start(READING, "Reading the world's code".to_owned());
		self.code = Some(code::start(options, self.out.clone(), self.inputs.clone()));
	}

	fn code_event(&mut self, event: code::Event) {
		// Whatever it says, it has read the code, or can't
		if !matches!(event, code::Event::Reload { .. }) {
			self.progress_end(READING);
		}
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

	/// The world's code changed. Each breakpoint goes where its line is now:
	/// it's a line of the file as it was when the client gave it, and the
	/// file's text says where that went, whoever changed it. The client is
	/// told where they are.
	fn reapply_breakpoints(&mut self) {
		let files: Vec<String> = self.breakpoints.keys().cloned().collect();
		for file in files {
			let now = crate::lines::read(&file);
			if let (Some(Some(then)), Some(now)) = (self.breakpoint_texts.get(&file), &now) {
				if then != now {
					let map = crate::lines::LineMap::new(then, now);
					for (_, breakpoint) in self.breakpoints.get_mut(&file).into_iter().flatten() {
						breakpoint.line = map.line(breakpoint.line as u32).into();
					}
				}
			}
			self.breakpoint_texts.insert(file.clone(), now);

			for breakpoint in self.apply_breakpoints(&file) {
				self.out.event(dap::BreakpointEvent {
					reason: "changed".to_owned(),
					breakpoint
				});
			}
		}

		// A paused world's frames may be on other lines now
		if self.client.supports_invalidated_event && self.debuggee.as_ref().is_some_and(Debuggee::stopped) {
			self.out.event(dap::InvalidatedEvent {
				areas: vec!["stacks".to_owned()]
			});
		}
	}

	fn connection_closed(&mut self, generation: u64, reason: &str) {
		if self.debuggee.as_ref().is_none_or(|d| d.generation != generation) {
			return;
		}
		self.debuggee = None;
		self.waiting_for_code = false;
		self.set.lock().unwrap().clear();

		// A world that's ending shuts its debugger down on the way
		if self.pid.is_some() && !self.ending {
			self.out
				.console(format!("The world's debugger disconnected ({}); connecting again when it's back", reason));
			self.progress_start(RESTARTING, "Waiting for the world to come back".to_owned());
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
			if let Some(process) = self.world_process() {
				self.out.console(format!("Ending the world (process {})", process));
				self.kill_world();
			}
		}
		// Stop trying to connect
		self.pid = None;
		self.generation += 1;
	}

	/// The process of the world: the one launched, or the one attached to
	fn world_process(&self) -> Option<u32> {
		self.game.as_ref().map(Game::pid).or(self.pid)
	}

	/// Ends the world's process, there and then
	fn kill_world(&mut self) {
		if let Some(game) = &self.game {
			game.kill();
		} else if let Some(pid) = self.pid {
			local::kill(pid);
		}
	}

	/// Has the world end itself, which runs the shutdown code it has in
	/// `world/Del()`. One that hasn't ended a few seconds on is ended.
	fn terminate(&mut self) -> Handled {
		let Some(process) = self.world_process() else {
			return Ok(Value::Null);
		};
		if self.ending {
			// Asked already, and on its way
			return Ok(Value::Null);
		}
		self.ending = true;
		// Not waited for: the world doesn't answer if it ends, and has a
		// while to if it doesn't
		let asked = self.debuggee.as_mut().is_some_and(|debuggee| {
			debuggee
				.send(&AuxRequest::Eval {
					frame_id: None,
					command: "world.Del()".to_owned(),
					context: None
				})
				.is_ok()
		});
		if asked {
			self.out.console("Asked the world to end: its world/Del() runs first");
			let inputs = self.inputs.clone();
			let launched = self.game.is_some();
			std::thread::spawn(move || {
				let deadline = std::time::Instant::now() + TERMINATE_GRACE;
				let mut overdue = false;
				// The launched world's exit is heard about anyway
				while launched || local::alive(process) {
					if !overdue && std::time::Instant::now() >= deadline {
						overdue = true;
						if inputs.send(Input::TerminateOverdue { process }).is_err() || launched {
							return;
						}
					}
					std::thread::sleep(CONNECT_INTERVAL);
				}
				let _ = inputs.send(Input::ProcessGone);
			});
		} else {
			self.out.console(format!(
				"The world isn't connected, and can't be asked to end: ending its process ({})",
				process
			));
			self.kill_world();
		}
		Ok(Value::Null)
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
		// Lines of text the client hasn't saved: the world's code, and the
		// file, are still as they were. They're right once it's saved and
		// the world has been given the change, if it's being given changes.
		let unsaved = args.sourceModified.unwrap_or(false);
		if unsaved && self.code.is_none() {
			return Err("the file has changes that haven't been saved".to_owned());
		}
		let mut wanted = vec![];
		for breakpoint in args.breakpoints.unwrap_or_default() {
			wanted.push((self.next_breakpoint_id, breakpoint));
			self.next_breakpoint_id += 1;
		}

		if unsaved {
			let breakpoints = wanted
				.iter()
				.map(|(number, breakpoint)| Breakpoint {
					id: Some(*number),
					verified: false,
					message: Some("it's set when the file is saved and the world has the change".to_owned()),
					line: Some(breakpoint.line),
					..Default::default()
				})
				.collect();
			self.breakpoints.insert(path.clone(), wanted);
			self.breakpoint_texts.insert(path, None);
			return json(SetBreakpointsResponse { breakpoints });
		}

		self.breakpoints.insert(path.clone(), wanted);
		self.breakpoint_texts.insert(path.clone(), crate::lines::read(&path));
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
		let wanted: Vec<(i64, i64, Option<String>, Option<String>, Option<String>)> = self.breakpoints[path]
			.iter()
			.map(|(number, b)| (*number, b.line, b.condition.clone(), b.hitCondition.clone(), b.logMessage.clone()))
			.collect();

		let mut breakpoints = vec![];
		let mut set = vec![];
		for (number, line, condition, hit, log) in wanted {
			// A breakpoint with a message says it, and doesn't stop
			let log = log.filter(|message| !message.is_empty()).map(|message| aux::LogPart::parse(&message));
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
			let hit = match hit_condition(hit.as_deref()) {
				Ok(hit) => hit,
				Err(e) => {
					breakpoints.push(unverified(&e));
					continue;
				}
			};

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
				let mut at = None;
				match self.set_breakpoint(&instruction, condition.clone(), hit, log.clone()) {
					Ok(Some((set_line, set_at))) => {
						result.get_or_insert(Ok(set_line));
						at = Some(set_at);
					}
					Ok(None) => {}
					Err(e) => {
						result.get_or_insert(Err(e));
					}
				}
				set.push(SetBreakpoint {
					asked: instruction,
					at,
					id: number
				});
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
		self.set.lock().unwrap().insert(path.to_owned(), set);
		breakpoints
	}

	/// `Some(line)` when it's set
	fn set_breakpoint(
		&mut self,
		instruction: &InstructionRef,
		condition: Option<String>,
		hit: Option<aux::HitCondition>,
		log: Option<Vec<aux::LogPart>>
	) -> Result<Option<(Option<u32>, InstructionRef)>, String> {
		match self.request(&AuxRequest::BreakpointSet {
			instruction: instruction.clone(),
			condition,
			hit,
			log
		})? {
			AuxResponse::BreakpointSet {
				result: aux::BreakpointSetResult::Success { line, at }
			} => Ok(Some((line, at))),
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
		let set = self.set.lock().unwrap().remove(key).unwrap_or_default();
		for breakpoint in set {
			let _ = self.request(&AuxRequest::BreakpointUnset {
				instruction: breakpoint.asked
			});
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
		let wanted: Vec<(String, Option<String>, Option<String>)> = self
			.function_breakpoints
			.iter()
			.map(|b| (b.name.clone(), b.condition.clone(), b.hitCondition.clone()))
			.collect();

		let mut breakpoints = vec![];
		let mut set = vec![];
		for (name, condition, hit) in wanted {
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
			let set_as_asked = hit_condition(hit.as_deref()).and_then(|hit| self.set_breakpoint(&instruction, condition, hit, None));
			let id = self.next_breakpoint_id;
			self.next_breakpoint_id += 1;
			let mut at = None;
			breakpoints.push(match set_as_asked {
				Ok(Some((line, set_at))) => {
					at = Some(set_at);
					Breakpoint {
						id: Some(id),
						verified: true,
						line: line.map(i64::from),
						..Default::default()
					}
				}
				Ok(None) => Breakpoint {
					id: Some(id),
					verified: false,
					message: Some(format!("there's no {}", name)),
					..Default::default()
				},
				Err(e) => Breakpoint {
					id: Some(id),
					verified: false,
					message: Some(e),
					..Default::default()
				}
			});
			set.push(SetBreakpoint {
				asked: instruction,
				at,
				id
			});
		}
		self.set.lock().unwrap().insert(FUNCTION_BREAKPOINTS.to_owned(), set);
		breakpoints
	}

	fn set_exception_breakpoints(&mut self, args: ExceptionBreakpointsArguments) -> Handled {
		// A filter that's ticked is in one list or the other: with its
		// condition, or without one
		let filter = |id: &str| -> Option<aux::RuntimeFilter> {
			if let Some(option) = args.filter_options.iter().find(|option| option.filter_id == id) {
				return Some(aux::RuntimeFilter::parse(option.condition.as_deref().unwrap_or_default()));
			}
			args.filters.iter().any(|filter| filter == id).then(aux::RuntimeFilter::default)
		};
		self.uncaught = filter(RUNTIMES);
		self.caught = filter(CAUGHT);
		if self.debugging && self.debuggee.is_some() {
			self.catch_runtimes()?;
		}
		Ok(Value::Null)
	}

	/// Tells the world which runtime errors to pause on (it doesn't answer)
	fn catch_runtimes(&mut self) -> Result<(), String> {
		let (uncaught, caught) = (self.uncaught.clone(), self.caught.clone());
		self.debuggee()?.send(&AuxRequest::CatchRuntimes { uncaught, caught })
	}

	/// The runtime error the world is stopped on: what it says, whether
	/// something will catch it, and the procs it's in
	fn exception_info(&mut self) -> Handled {
		let error = self.last_error.lock().unwrap().clone();
		let frames = match self.request(&AuxRequest::StackFrames {
			stack_id: 0,
			start_frame: None,
			count: None
		}) {
			Ok(AuxResponse::StackFrames { frames, .. }) => frames,
			_ => vec![]
		};
		let trace: Vec<String> = frames
			.iter()
			.map(|frame| match (&frame.file, frame.line) {
				(Some(file), Some(line)) => format!("{} ({}:{})", frame.instruction.proc.path, file, line),
				_ => frame.instruction.proc.path.clone()
			})
			.collect();
		json(ExceptionInfoResponse {
			exceptionId: crate::debuggee::error_kind(error.caught).to_owned(),
			description: Some(error.message.clone()),
			breakMode: if error.caught { ExceptionBreakMode::Always } else { ExceptionBreakMode::Unhandled },
			details: Some(ExceptionDetails {
				message: Some(error.message),
				stackTrace: (!trace.is_empty()).then(|| trace.join("\n")),
				..Default::default()
			})
		})
	}

	fn threads(&mut self) -> Handled {
		let mut threads = vec![];
		if self.debuggee.is_some() {
			// One more than the list has: the debug library's own is left out
			let limit = self.sleeping_threads as usize;
			let wanted = self.sleeping_threads.saturating_add(1);
			match self.request(&AuxRequest::Stacks { sleeping: Some(wanted) })? {
				AuxResponse::Stacks { stacks, sleeping } => {
					let mut asleep = sleeping as usize;
					let mut listed = 0;
					for stack in stacks {
						if strip_declaration(&stack.name) == KEEPALIVE {
							asleep -= 1;
							continue;
						}
						if stack.sleeping {
							if listed == limit {
								continue;
							}
							listed += 1;
						}
						threads.push(Thread {
							id: stack.id.into(),
							name: thread_name(&stack)
						});
					}
					if asleep > listed && !self.said_more_sleep {
						self.said_more_sleep = true;
						self.out.console(format!(
							"{} procs are asleep: the list of threads has the first {} (\"sleepingThreads\" in the launch configuration sets how many)",
							asleep, listed
						));
					}
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
				// One that runs what its proc's code was before a hot reload
				// is on a line of the file as it was
				name: if frame.old_code { format!("{} (old code)", proc.path) } else { proc.path.clone() },
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
		// The globals aren't opened until the client is asked to: a world can
		// have thousands
		let scopes = [("Locals", locals, Some("locals")), ("Arguments", arguments, Some("arguments")), ("Globals", globals, None)]
			.into_iter()
			.filter_map(|(name, vars, hint)| {
				vars.map(|vars| Scope {
					name: name.to_owned(),
					presentationHint: hint.map(str::to_owned),
					variablesReference: vars.0.into(),
					expensive: hint.is_none(),
					..Default::default()
				})
			})
			.collect();
		json(ScopesResponse { scopes })
	}

	fn variables(&mut self, args: VariablesArguments) -> Handled {
		// A client takes a long list a piece at a time
		let piece = |n: Option<i64>| n.and_then(|n| u32::try_from(n).ok());
		let vars = match self.request(&AuxRequest::Variables {
			vars: aux::VariablesRef(args.variablesReference as i32),
			start: piece(args.start),
			count: piece(args.count).filter(|&count| count > 0)
		})? {
			AuxResponse::Variables { vars } => vars,
			other => return Err(unexpected(other))
		};
		let hex = args.format.is_some_and(|format| format.hex == Some(true));

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
					value: shown(var.value, hex),
					type_: var.kind,
					evaluateName: var.expr,
					variablesReference: var.variables.map_or(0, |vars| vars.0.into()),
					indexedVariables: var.entries.map(i64::from),
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
		// The mouse resting on some code shouldn't change the world
		if args.context.as_deref() == Some("hover") && changes_things(&args.expression) {
			return Err("a hover doesn't call procs or assign".to_owned());
		}
		let hex = args.format.is_some_and(|format| format.hex == Some(true));
		match self.request(&AuxRequest::Eval {
			frame_id: args.frameId.map(|id| id as u32),
			command: args.expression,
			context: args.context
		})? {
			AuxResponse::Eval(Ok(result)) => json(EvaluateResponse {
				result: shown(result.value, hex),
				type_: result.kind,
				variablesReference: result.variables.map_or(0, |vars| vars.0.into()),
				indexedVariables: result.entries.map(|entries| entries as usize),
				..Default::default()
			}),
			// The client shows it where the value would be
			AuxResponse::Eval(Err(why)) => Err(why),
			other => Err(unexpected(other))
		}
	}

	/// What could be typed next in the debug console
	fn completions(&mut self, args: CompletionsArguments) -> Handled {
		// The column counts UTF-16 units, from 1
		let before = args.column.max(1) as usize - 1;
		let cursor = args
			.text
			.char_indices()
			.scan(0, |units, (at, c)| {
				let here = *units;
				*units += c.len_utf16();
				Some((at, here))
			})
			.find(|(_, units)| *units >= before)
			.map_or(args.text.len(), |(at, _)| at);
		let Some(place) = completion_place(&args.text[..cursor]) else {
			return Ok(serde_json::json!({ "targets": [] }));
		};
		let prefix = &args.text[place.start..cursor];
		let (start, length) = (args.text[..place.start].encode_utf16().count() + 1, prefix.encode_utf16().count());
		let target = |label: &str, kind: &str| serde_json::json!({ "label": label, "type": kind, "start": start, "length": length });

		// The debug server's own commands
		if place.command {
			let targets: Vec<Value> = ["dis", "disassemble", "guest_override", "help", "mem_profiler"]
				.into_iter()
				.filter(|command| command.starts_with(prefix))
				.map(|command| target(command, "keyword"))
				.collect();
			return Ok(serde_json::json!({ "targets": targets }));
		}

		match self.request(&AuxRequest::Complete {
			frame_id: args.frame_id.map(|id| id as u32),
			base: place.base.map(str::to_owned),
			prefix: prefix.to_owned()
		})? {
			AuxResponse::Complete(names) => {
				let member = place.base.is_some();
				let targets: Vec<Value> = names
					.iter()
					.map(|name| {
						let kind = match (name.proc, member) {
							(true, true) => "method",
							(true, false) => "function",
							(false, true) => "property",
							(false, false) => "variable"
						};
						target(&name.name, kind)
					})
					.collect();
				Ok(serde_json::json!({ "targets": targets }))
			}
			other => Err(unexpected(other))
		}
	}

	/// Gives a variable the value of an expression
	fn set_variable(&mut self, args: SetVariableArguments) -> Handled {
		let hex = args.format.is_some_and(|format| format.hex == Some(true));
		match self.request(&AuxRequest::SetVariable {
			vars: aux::VariablesRef(args.variables_reference as i32),
			name: args.name,
			value: args.value
		})? {
			AuxResponse::SetVariable(Ok(variable)) => Ok(serde_json::json!({
				"value": shown(variable.value, hex),
				"type": variable.kind,
				"variablesReference": variable.variables.map_or(0, |vars| vars.0),
				"indexedVariables": variable.entries
			})),
			AuxResponse::SetVariable(Err(why)) => Err(why),
			other => Err(unexpected(other))
		}
	}

	/// Gives what an expression names the value of another: an assignment,
	/// which is an expression in DM
	fn set_expression(&mut self, args: SetExpressionArguments) -> Handled {
		let hex = args.format.is_some_and(|format| format.hex == Some(true));
		match self.request(&AuxRequest::Eval {
			frame_id: args.frame_id.map(|id| id as u32),
			command: format!("{} = ({})", args.expression, args.value),
			context: None
		})? {
			AuxResponse::Eval(Ok(result)) => Ok(serde_json::json!({
				"value": shown(result.value, hex),
				"type": result.kind,
				"variablesReference": result.variables.map_or(0, |vars| vars.0),
				"indexedVariables": result.entries
			})),
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

	/// The lines between two that a breakpoint can go on: those with code in
	/// the world as it runs now. None while there's no world, or it runs
	/// without debugging, as breakpoints can't be set then either; none in
	/// stddef.dm, which has no lines in the world's code.
	fn breakpoint_locations(&mut self, args: BreakpointLocationsArguments) -> Handled {
		let path = match args.source.path {
			Some(path) if self.debugging && self.debuggee.is_some() => path,
			_ => return Ok(serde_json::json!({ "breakpoints": [] }))
		};
		let start = args.line.clamp(1, u32::MAX.into()) as u32;
		let end = args.end_line.unwrap_or(args.line).clamp(start.into(), u32::MAX.into()) as u32;
		match self.request(&AuxRequest::LinesWithCode {
			file: self.world_file_name(&path),
			start,
			end
		})? {
			AuxResponse::LinesWithCode { lines } => {
				let breakpoints: Vec<Value> = lines.into_iter().map(|line| serde_json::json!({ "line": line })).collect();
				Ok(serde_json::json!({ "breakpoints": breakpoints }))
			}
			other => Err(unexpected(other))
		}
	}
}

/// The file and line in BYOND's report of a runtime error:
/// `  source file: code/game/mob.dm,42`
fn runtime_place(line: &str) -> Option<(&str, i64)> {
	let (file, number) = line.trim().strip_prefix("source file: ")?.rsplit_once(',')?;
	Some((file, number.trim().parse().ok()?))
}

/// Where a name is being typed, at the end of some text
#[derive(Debug, PartialEq, Eq)]
struct CompletionPlace<'a> {
	/// Where the name starts
	start: usize,
	/// What it's a member of, if it comes after a `.` or the like: `a.b[2]`
	/// for `a.b[2].na`
	base: Option<&'a str>,
	/// It's one of the debug server's commands, after a `#`
	command: bool
}

/// Where the name being typed at the end of `text` starts, and what it's a
/// member of. `None` where names aren't to be suggested: in a text, or after
/// something that would have to be called to know what it is.
fn completion_place(text: &str) -> Option<CompletionPlace<'_>> {
	let name_char = |c: char| c.is_ascii_alphanumeric() || c == '_';
	if text.chars().filter(|&c| c == '"').count() % 2 == 1 {
		return None;
	}
	let start = text.trim_end_matches(name_char).len();
	// A number isn't a name
	if text[start..].starts_with(|c: char| c.is_ascii_digit()) {
		return None;
	}
	if text[..start].trim_start() == "#" {
		return Some(CompletionPlace {
			start,
			base: None,
			command: true
		});
	}

	// What's before it, back through `.`, `?.`, `:` and `?:`, names and
	// indexes
	let mut at = start;
	let mut base_end = None;
	loop {
		let before = &text[..at];
		let Some(joined) = ["?.", "?:", ".", ":"].into_iter().find(|by| before.ends_with(by)) else {
			break;
		};
		let member_end = at - joined.len();
		base_end.get_or_insert(member_end);
		let mut member = member_end;
		// Indexes: `[...]`, with nothing called inside
		while text[..member].ends_with(']') {
			let mut depth = 0;
			let mut open = None;
			for (i, c) in text[..member].char_indices().rev() {
				match c {
					']' => depth += 1,
					'[' => {
						depth -= 1;
						if depth == 0 {
							open = Some(i);
							break;
						}
					}
					'(' | ')' => return None,
					_ => {}
				}
			}
			member = open?;
		}
		let name = text[..member].trim_end_matches(name_char).len();
		// Something else: a call's result, a parenthesis
		if name == member {
			return None;
		}
		at = name;
	}
	Some(CompletionPlace {
		start,
		base: base_end.map(|end| &text[at..end]),
		command: false
	})
}

/// Whether an expression could do more than read: it calls something, or
/// assigns. Texts in it don't count.
fn changes_things(expression: &str) -> bool {
	let mut quoted = false;
	let mut chars = expression.chars().peekable();
	let mut last = ' ';
	while let Some(c) = chars.next() {
		if quoted {
			match c {
				'\\' => {
					chars.next();
				}
				'"' => quoted = false,
				// An expression inside the text is code again: say it might
				'[' => return true,
				_ => {}
			}
			continue;
		}
		let next = chars.peek().copied().unwrap_or(' ');
		match c {
			'"' => quoted = true,
			'(' => return true,
			'+' | '-' if next == c => return true,
			// `=` on its own, or after an operator that makes an assignment
			// of it (`+=`, `||=`), but not `==`, `!=`, `<=` or `>=`
			'=' if next != '=' && !matches!(last, '=' | '!' | '<' | '>') => return true,
			'=' if next != '=' && matches!(last, '<' | '>') => {
				// `<<=` and `>>=` assign, `<=` and `>=` compare
				let shifts = expression.contains("<<=") || expression.contains(">>=");
				if shifts {
					return true;
				}
			}
			_ => {}
		}
		last = c;
	}
	false
}

/// A value's text as the client asked for it: a whole number in hex, if it
/// wants those
fn shown(value: String, hex: bool) -> String {
	if !hex {
		return value;
	}
	match value.parse::<i64>() {
		Ok(n) if n < 0 => format!("-0x{:X}", -n),
		Ok(n) => format!("0x{:X}", n),
		Err(_) => value
	}
}

/// The hits a breakpoint stops at, as the client wrote them, if it did
fn hit_condition(text: Option<&str>) -> Result<Option<aux::HitCondition>, String> {
	match text.map(str::trim).filter(|text| !text.is_empty()) {
		Some(text) => aux::HitCondition::parse(text).map(Some),
		None => Ok(None)
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

/// What a stack is called in the list of threads: the proc it's in, where,
/// and whether it's asleep
fn thread_name(stack: &aux::Stack) -> String {
	let mut name = stack.name.clone();
	if let (Some(file), Some(line)) = (&stack.file, stack.line) {
		let file = file.rsplit(['/', '\\']).next().unwrap_or(file);
		name += &format!("  {}:{}", file, line);
	}
	if stack.sleeping {
		name += ", sleeping";
	}
	name
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

	use auxtools_cli::server_types::{HitCondition, LogPart, RuntimeFilter};

	use super::{changes_things, completion_place, file_name_in, named_proc, runtime_place, shown};

	#[test]
	fn byond_says_where_a_runtime_error_was() {
		assert_eq!(runtime_place("  source file: code/game/mob.dm,42"), Some(("code/game/mob.dm", 42)));
		assert_eq!(runtime_place("  source file: a, b.dm,7\r\n"), Some(("a, b.dm", 7)));
		for other in ["runtime error: bad", "proc name: fail (/proc/fail)", "  source file: mob.dm", "source file: mob.dm,x", "  usr: null"] {
			assert_eq!(runtime_place(other), None, "{}", other);
		}
	}

	#[test]
	fn a_name_is_completed_where_it_starts() {
		fn place(text: &str) -> Option<(&str, Option<&str>, bool)> {
			completion_place(text).map(|place| (&text[place.start..], place.base, place.command))
		}
		assert_eq!(place("hea"), Some(("hea", None, false)));
		assert_eq!(place(""), Some(("", None, false)));
		assert_eq!(place("x + hea"), Some(("hea", None, false)));
		assert_eq!(place("leaf(by, n"), Some(("n", None, false)));
		assert_eq!(place("src."), Some(("", Some("src"), false)));
		assert_eq!(place("src.na"), Some(("na", Some("src"), false)));
		assert_eq!(place("x + src.loc.na"), Some(("na", Some("src.loc"), false)));
		assert_eq!(place("a?.b:c?:d"), Some(("d", Some("a?.b:c"), false)));
		assert_eq!(place("items[2].na"), Some(("na", Some("items[2]"), false)));
		assert_eq!(place("f(x) + items[\"k\"][i + 1].na"), Some(("na", Some("items[\"k\"][i + 1]"), false)));
		assert_eq!(place("#"), Some(("", None, true)));
		assert_eq!(place("#di"), Some(("di", None, true)));
		assert_eq!(place(" #mem"), Some(("mem", None, true)));
	}

	#[test]
	fn nothing_is_completed_where_it_cant_be_known() {
		for text in ["get_mob().na", "(a).b", "items[f()].na", "\"text", "\"a\" + \"b.", "12", "x + 3", ".na", "a..b", "]."] {
			assert_eq!(completion_place(text), None, "{}", text);
		}
	}

	#[test]
	fn a_hover_reads_and_does_nothing_else() {
		for reads in [
			"x",
			"src.health",
			"a?.b:c",
			"x == 1",
			"x != y && x <= 2 || y >= 3",
			"L[2]",
			"L[\"a = b\"]",
			"\"text with (parens) and = signs++\"",
			"x - -y",
			"x + +y",
			"a < b",
			"/datum/thing"
		] {
			assert!(!changes_things(reads), "{}", reads);
		}
		for changes in [
			"f()",
			"src.bump(1)",
			"x = 1",
			"x += 1",
			"x ||= y",
			"x <<= 1",
			"x++",
			"--x",
			"L[f()]",
			"\"[f()]\"",
			"new /datum/thing()"
		] {
			assert!(changes_things(changes), "{}", changes);
		}
	}

	#[test]
	fn whole_numbers_are_shown_in_hex_when_asked() {
		assert_eq!(shown("42".to_owned(), true), "0x2A");
		assert_eq!(shown("0".to_owned(), true), "0x0");
		assert_eq!(shown("-255".to_owned(), true), "-0xFF");
		assert_eq!(shown("16777216".to_owned(), true), "0x1000000");
		assert_eq!(shown("42".to_owned(), false), "42");
		for other in ["1.5", "\"42\"", "null", "/list {len = 3}", "inf"] {
			assert_eq!(shown(other.to_owned(), true), other);
		}
	}

	#[test]
	fn a_log_message_is_text_with_expressions_in_braces() {
		let text = |text: &str| LogPart::Text(text.to_owned());
		let expr = |text: &str| LogPart::Expr(text.to_owned());
		assert_eq!(LogPart::parse("plain"), [text("plain")]);
		assert_eq!(LogPart::parse(""), []);
		assert_eq!(LogPart::parse("{x}"), [expr("x")]);
		assert_eq!(LogPart::parse("x is {x}, y is { y + 1 }!"), [text("x is "), expr("x"), text(", y is "), expr("y + 1"), text("!")]);
		assert_eq!(LogPart::parse("{a}{b}"), [expr("a"), expr("b")]);
	}

	#[test]
	fn braces_in_an_expression_are_its_own() {
		let text = |text: &str| LogPart::Text(text.to_owned());
		let expr = |text: &str| LogPart::Expr(text.to_owned());
		assert_eq!(LogPart::parse("{alist(\"a\" = 1)}"), [expr("alist(\"a\" = 1)")]);
		assert_eq!(LogPart::parse("{\"}\"} after"), [expr("\"}\""), text(" after")]);
		assert_eq!(LogPart::parse("{\"a \\\" } b\"}"), [expr("\"a \\\" } b\"")]);
		assert_eq!(LogPart::parse("{a {b} c}"), [expr("a {b} c")]);
	}

	#[test]
	fn a_brace_that_is_never_closed_is_text() {
		let text = |text: &str| LogPart::Text(text.to_owned());
		let expr = |text: &str| LogPart::Expr(text.to_owned());
		assert_eq!(LogPart::parse("{x"), [text("{x")]);
		assert_eq!(LogPart::parse("{x} and {y"), [expr("x"), text(" and {y")]);
		assert_eq!(LogPart::parse("} {"), [text("} {")]);
	}

	#[test]
	fn a_hit_count_is_a_number_or_one_after_a_sign() {
		for (text, expected) in [
			("5", HitCondition::AtLeast(5)),
			(" 5 ", HitCondition::AtLeast(5)),
			(">= 5", HitCondition::AtLeast(5)),
			(">=5", HitCondition::AtLeast(5)),
			("> 5", HitCondition::More(5)),
			("== 5", HitCondition::Equal(5)),
			("=5", HitCondition::Equal(5)),
			("< 5", HitCondition::Less(5)),
			("<= 5", HitCondition::AtMost(5)),
			("% 5", HitCondition::Every(5)),
			("%5", HitCondition::Every(5))
		] {
			assert_eq!(HitCondition::parse(text), Ok(expected), "{:?}", text);
		}
		for text in ["", "five", "5 times", "-1", "% 0", "=> 5", "5 5", "> ", "1.5"] {
			assert!(HitCondition::parse(text).is_err(), "{:?}", text);
		}
	}

	#[test]
	fn a_hit_count_takes_the_hits_it_says() {
		let taken = |hit: HitCondition| -> Vec<u32> { (1..=8).filter(|&n| hit.takes(n)).collect() };
		assert_eq!(taken(HitCondition::AtLeast(6)), [6, 7, 8]);
		assert_eq!(taken(HitCondition::More(6)), [7, 8]);
		assert_eq!(taken(HitCondition::Equal(3)), [3]);
		assert_eq!(taken(HitCondition::Less(3)), [1, 2]);
		assert_eq!(taken(HitCondition::AtMost(3)), [1, 2, 3]);
		assert_eq!(taken(HitCondition::Every(3)), [3, 6]);
		assert_eq!(taken(HitCondition::Every(1)), [1, 2, 3, 4, 5, 6, 7, 8]);
	}

	#[test]
	fn a_runtime_filter_with_nothing_in_it_takes_every_error() {
		for condition in ["", "  ", ",", "!", " , ! , "] {
			let filter = RuntimeFilter::parse(condition);
			assert_eq!(filter, RuntimeFilter::default(), "{:?}", condition);
			assert!(filter.takes("Division by zero"));
		}
	}

	#[test]
	fn a_runtime_filter_takes_errors_with_one_of_its_texts() {
		let filter = RuntimeFilter::parse("bounds, Division");
		assert!(filter.takes("list index out of bounds"));
		assert!(filter.takes("division by zero"));
		assert!(!filter.takes("cannot read from list"));
	}

	#[test]
	fn a_runtime_filter_leaves_out_errors_with_text_after_a_bang() {
		let filter = RuntimeFilter::parse("!libdreamluau, ! rust_g ");
		assert!(filter.takes("list index out of bounds"));
		assert!(!filter.takes("Could not find LibDreamLuau.so"));
		assert!(!filter.takes("rust_g: bad call"));
		// Both kinds: one of these, and none of those
		let filter = RuntimeFilter::parse("list, !bounds");
		assert!(filter.takes("cannot read from list"));
		assert!(!filter.takes("list index out of bounds"));
		assert!(!filter.takes("Division by zero"));
	}

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
