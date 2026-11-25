use std::{
	collections::{BTreeMap, HashMap, HashSet},
	io::BufReader,
	path::{Path, PathBuf},
	sync::{
		atomic::{AtomicBool, Ordering},
		mpsc, Arc, Mutex
	},
	time::{Duration, Instant}
};

use auxtools_cli::{
	byond::{self, Server},
	client::{
		ops::{Asks, Ending, Operations, Placed, Trigger},
		when_listening
	},
	game::{Game, Launch},
	local::{self, Stream}
};
use auxtools_protocol::{
	self as aux, ContinueKind, InstructionRef, ProcRef, Request as AuxRequest, Response as AuxResponse, GLOBAL_INIT, HELPER_FILE
};
use dap_types::*;
use dmasm::diagnostic::{Diagnostic, Id};
use serde::Deserialize;
use serde_json::Value;

use crate::{
	code,
	dap::{self, Output},
	debuggee::{Debuggee, LastError, SetBreakpoint, SetBreakpoints, INSTRUCTION_BREAKPOINTS},
	stddef::Stddef
};

mod data;
mod files;
mod join;
mod journal;
mod lifecycle;
mod profile;
mod script;
mod skin;
mod status;

pub use journal::reload_by_another;
pub use lifecycle::{default_args, Ask, CommandLine};

const RUNTIMES: &str = "runtimes";
const CAUGHT: &str = "caught";
const STDDEF_REFERENCE: i64 = 1;

pub const NO_LINES: &str = "the world has no line numbers because it was compiled without debug information. Add #define DEBUG to the .dme and \
                            rebuild, or set a breakpoint on a proc";
const NO_CODE: &str = "no executable code at or after this line";
const REMOVED_BY_RELOAD: &str = "a hot reload removed the code at this address";
const FUNCTION_BREAKPOINTS: &str = "<functions>";
const STARTING: &str = "starting";
const READING: &str = "reading";
const RESTARTING: &str = "restarting";
const RELOADING: &str = "reloading";
const UNAVAILABLE: &str = "debugger initialization failed";

pub enum Input {
	Message(String),
	Asked(Ask),
	StdinClosed,
	Connected {
		generation: u64,
		stream: Stream
	},
	ConnectionClosed {
		generation: u64,
		reason: String
	},
	ShuttingDown {
		generation: u64
	},
	Unavailable {
		generation: u64,
		reason: String
	},
	Code {
		worker: u64,
		event: code::Event
	},
	RebootOverdue {
		generation: u64
	},
	CodeChanged {
		generation: u64
	},
	WatchEnded {
		generation: u64,
		id: u32,
		reason: String
	},
	Stopped {
		generation: u64,
		stop: aux::Stop,
		number: u64
	},
	Resumed {
		generation: u64
	},
	Logged {
		message: String,
		file: Option<String>,
		line: Option<u32>
	},
	WorldOutput(Vec<String>),
	GameExited(Option<i32>),
	ProcessGone,
	TerminateOverdue {
		process: u32
	},
	Quit
}

#[derive(Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
struct LaunchArguments {
	#[serde(default)]
	no_debug: bool,
	dme: Option<PathBuf>,
	dmb: Option<PathBuf>,
	server: Option<Server>,
	byond_path: Option<PathBuf>,
	#[serde(default)]
	defines: Vec<String>,
	#[serde(default)]
	build_flags: Vec<String>,
	#[serde(default = "yes")]
	hot_reload: bool,
	port: Option<u16>,
	args: Option<Vec<String>>,
	#[serde(default)]
	env: HashMap<String, String>,
	debug_library: Option<PathBuf>,
	sleeping_threads: Option<u32>
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachArguments {
	process_id: Value,
	dme: Option<PathBuf>,
	#[serde(default)]
	defines: Vec<String>,
	#[serde(default)]
	hot_reload: bool,
	sleeping_threads: Option<u32>
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ClientAbilities {
	#[serde(default)]
	supports_progress_reporting: bool,
	#[serde(default)]
	supports_invalidated_event: bool
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompletionsArguments {
	frame_id: Option<i64>,
	text: String,
	line: Option<i64>,
	column: i64
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BreakpointLocationsArguments {
	source: Source,
	line: i64,
	end_line: Option<i64>
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetVariableArguments {
	variables_reference: i64,
	name: String,
	value: String,
	format: Option<ValueFormat>
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetExpressionArguments {
	expression: String,
	value: String,
	frame_id: Option<i64>,
	format: Option<ValueFormat>
}

#[derive(Deserialize)]
struct UnsavedArguments {
	path: String,
	unsaved: bool
}

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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StepArguments {
	thread_id: i64,
	granularity: Option<String>
}

impl StepArguments {
	fn by_instruction(&self) -> bool {
		self.granularity.as_deref() == Some("instruction")
	}
}

#[derive(Deserialize)]
struct InstructionBreakpointsArguments {
	breakpoints: Vec<InstructionBreakpoint>
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct InstructionBreakpoint {
	instruction_reference: String,
	offset: Option<i64>,
	condition: Option<String>,
	hit_condition: Option<String>
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InstructionShown {
	id: i64,
	verified: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	message: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	line: Option<u32>,
	instruction_reference: String
}

impl InstructionShown {
	const fn unverified(id: i64, instruction_reference: String, message: String) -> Self {
		Self {
			id,
			verified: false,
			message: Some(message),
			line: None,
			instruction_reference
		}
	}
}

const fn yes() -> bool {
	true
}

type Handled = Result<Value, String>;

struct CodeSettings {
	dme: Option<PathBuf>,
	defines: Vec<String>,
	hot_reload: bool
}

pub struct Adapter {
	out: Arc<Output>,
	inputs: mpsc::Sender<Input>,
	leaving: Arc<AtomicBool>,
	answering: bool,

	debugging: bool,
	wanted_defines: Option<Vec<String>>,
	joined: join::Joined,
	reload_status: status::ReloadStatus,

	code_settings: Option<CodeSettings>,
	code: Option<mpsc::Sender<code::Command>>,
	code_worker: u64,
	hot_reload: bool,
	code_read: bool,
	watching: bool,
	stddef: Option<Stddef>,
	macros: Option<Vec<aux::Macro>>,

	debuggee: Option<Debuggee>,
	world: lifecycle::Lifecycle,
	last_error: Arc<Mutex<LastError>>,
	reload_started: Option<Instant>,

	breakpoints: BTreeMap<String, Vec<(i64, SourceBreakpoint)>>,
	client: ClientAbilities,
	progress: Vec<&'static str>,
	files: Arc<Mutex<files::WorldFiles>>,
	waiting_output: Vec<String>,
	breakpoint_texts: HashMap<String, Option<Vec<String>>>,
	unsaved: HashSet<String>,
	world_texts: HashMap<String, Option<Vec<String>>>,
	caught_up: Option<code::Texts>,
	world_as_of: Option<std::time::SystemTime>,
	deferred: HashSet<String>,
	waiting: HashSet<String>,
	reload_seen: Option<std::time::SystemTime>,
	reloaded_resources: Vec<aux::ReloadedResource>,
	said_changed: HashSet<String>,
	next_breakpoint_id: i64,
	function_breakpoints: Vec<FunctionBreakpoint>,
	instruction_breakpoints: Vec<(i64, InstructionBreakpoint)>,
	removed_by_reload: HashSet<i64>,
	refused_at: HashMap<i64, InstructionRef>,
	relocated: HashSet<String>,
	claims: u32,
	function_breakpoint_ids: Vec<i64>,
	told: HashMap<i64, (bool, Option<i64>, Option<String>)>,
	uncaught: Option<aux::RuntimeFilter>,
	caught: Option<aux::RuntimeFilter>,
	set: SetBreakpoints,
	sleeping_threads: u32,
	said_more_sleep: bool,
	data: data::DataBreakpoints
}

pub fn run() {
	let (inputs, receiver) = mpsc::channel();
	let leaving = Arc::new(AtomicBool::new(false));

	let stdin_inputs = inputs.clone();
	let stdin_leaving = leaving.clone();
	std::thread::spawn(move || {
		let mut stdin = BufReader::new(std::io::stdin());
		while let Ok(Some(message)) = dap::read_message(&mut stdin) {
			if asks_to_disconnect(&message) {
				stdin_leaving.store(true, Ordering::Relaxed);
			}
			if stdin_inputs.send(Input::Message(message)).is_err() {
				return;
			}
		}
		stdin_leaving.store(true, Ordering::Relaxed);
		let _ = stdin_inputs.send(Input::StdinClosed);
	});

	serve(Output::new(), inputs, &receiver, leaving);
}

fn asks_to_disconnect(message: &str) -> bool {
	#[derive(Deserialize)]
	struct Asked {
		command: Option<String>
	}
	message.contains("disconnect") && serde_json::from_str::<Asked>(message).is_ok_and(|asked| asked.command.as_deref() == Some("disconnect"))
}

pub fn serve(out: Output, inputs: mpsc::Sender<Input>, receiver: &mpsc::Receiver<Input>, leaving: Arc<AtomicBool>) {
	let mut adapter = Adapter {
		out: Arc::new(out),
		inputs,
		leaving,
		answering: false,
		debugging: true,
		wanted_defines: None,
		joined: join::Joined::default(),
		reload_status: status::ReloadStatus::default(),
		code_settings: None,
		code: None,
		code_worker: 0,
		hot_reload: false,
		code_read: false,
		watching: false,
		stddef: None,
		macros: None,
		debuggee: None,
		world: lifecycle::Lifecycle::default(),
		last_error: Arc::new(Mutex::new(LastError::default())),
		reload_started: None,
		client: ClientAbilities::default(),
		progress: vec![],
		files: Arc::new(Mutex::new(files::WorldFiles::default())),
		waiting_output: vec![],
		breakpoints: BTreeMap::new(),
		breakpoint_texts: HashMap::new(),
		unsaved: HashSet::new(),
		deferred: HashSet::new(),
		waiting: HashSet::new(),
		reload_seen: None,
		reloaded_resources: vec![],
		world_texts: HashMap::new(),
		caught_up: None,
		world_as_of: None,
		said_changed: HashSet::new(),
		next_breakpoint_id: 1,
		function_breakpoints: vec![],
		instruction_breakpoints: vec![],
		removed_by_reload: HashSet::new(),
		refused_at: HashMap::new(),
		relocated: HashSet::new(),
		claims: 0,
		function_breakpoint_ids: vec![],
		told: HashMap::new(),
		uncaught: Some(aux::RuntimeFilter::default()),
		caught: None,
		set: SetBreakpoints::default(),
		sleeping_threads: aux::DEFAULT_SLEEPING_STACKS,
		said_more_sleep: false,
		data: data::DataBreakpoints::default()
	};

	while let Ok(input) = receiver.recv() {
		if !adapter.handle(input) {
			break;
		}
	}

	if let Some(debuggee) = &mut adapter.debuggee {
		let _ = debuggee.send(&AuxRequest::Disconnect);
	}
	if let Some(game) = adapter.world.game.take() {
		game.kill();
	}
	adapter.joined.world_ended();
}

#[derive(Deserialize)]
struct BuildFlagsArguments {
	flags: Vec<String>
}

fn all_defines(args: &LaunchArguments) -> Vec<String> {
	let mut all = args.defines.clone();
	for flag in &args.build_flags {
		if !all.contains(flag) {
			all.push(flag.clone());
		}
	}
	all
}

fn arguments<T: for<'de> Deserialize<'de>>(arguments: Option<Value>) -> Result<T, String> {
	let arguments = arguments
		.filter(|arguments| !arguments.is_null())
		.unwrap_or_else(|| Value::Object(Default::default()));
	serde_json::from_value(arguments).map_err(|e| format!("bad arguments: {}", e))
}

fn json(value: impl serde::Serialize) -> Handled {
	serde_json::to_value(value).map_err(|e| e.to_string())
}

impl Adapter {
	fn handle(&mut self, input: Input) -> bool {
		match input {
			Input::Message(message) => {
				self.answering = true;
				let more = self.message(&message);
				self.answering = false;
				return more;
			}
			Input::Asked(ask) => {
				self.answering = true;
				let more = self.asked(ask);
				self.answering = false;
				return more;
			}
			Input::StdinClosed | Input::Quit => return false,
			Input::Connected { generation, stream } => {
				if generation == self.world.generation && self.debuggee.is_none() {
					self.connected(stream);
				}
			}
			Input::ConnectionClosed { generation, reason } => self.connection_closed(generation, &reason),
			Input::ShuttingDown { generation } => {
				if self.debuggee.as_ref().is_some_and(|d| d.generation == generation) {
					self.world.shutting_down = true;
				}
			}
			Input::Unavailable { generation, reason } => self.debugger_unavailable(generation, &reason),
			Input::Code { worker, event } => {
				if worker == self.code_worker {
					self.code_event(event);
				}
			}
			Input::RebootOverdue { generation } => self.reboot_overdue(generation),
			Input::WorldOutput(lines) => self.journal_output(lines),
			Input::Logged { message, file, line } => {
				let source = file.map(|file| self.file_source(&file));
				self.out.output_event(OutputEvent {
					output: message + "\n",
					category: Some("console".to_owned()),
					line: source.as_ref().and(line).map(i64::from),
					source,
					..Default::default()
				});
			}
			Input::Stopped { generation, stop, number } => {
				if let Some(debuggee) = self.debuggee.as_mut().filter(|d| d.generation == generation) {
					debuggee.stopped_at(stop, number);
				}
			}
			Input::Resumed { generation } => {
				if let Some(debuggee) = self.debuggee.as_mut().filter(|d| d.generation == generation) {
					debuggee.resumed();
				}
			}
			Input::CodeChanged { generation } => {
				if self.debuggee.as_ref().is_some_and(|d| d.generation == generation) {
					self.follow_reloaded_breakpoints();
					self.reapply_breakpoints(None);
				}
			}
			Input::WatchEnded { generation, id, reason } => {
				if self.debuggee.as_ref().is_some_and(|d| d.generation == generation) {
					self.watch_ended(id, &reason);
				}
			}
			Input::GameExited(code) => self.game_exited(code),
			Input::TerminateOverdue { process } => self.terminate_overdue(process),
			Input::ProcessGone => self.process_gone()
		}
		true
	}

	fn message(&mut self, message: &str) -> bool {
		let request: RequestMessage = match serde_json::from_str(message) {
			Ok(request) => request,
			Err(e) => {
				self.out.console(format!("Invalid DAP request: {}", e));
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
			"auxtoolsUnsaved" => arguments(args).and_then(|args| self.unsaved_changed(args)),
			"auxtoolsReconnect" => self.reconnect_clients(),
			"auxtoolsJoin" => arguments(args).and_then(|args| self.join(args)),
			"auxtoolsReboot" => self.reboot(),
			"auxtoolsProfile" => arguments(args).and_then(|args| self.profile(args)),
			"auxtoolsBuildFlags" => arguments(args).and_then(|args| self.build_flags_changed(args)),
			"restart" => arguments(args).and_then(|args| self.restart(args)),
			"setFunctionBreakpoints" => arguments(args).and_then(|args| self.set_function_breakpoints(args)),
			"setInstructionBreakpoints" => arguments(args).and_then(|args| self.set_instruction_breakpoints(args)),
			"setExceptionBreakpoints" => arguments(args).and_then(|args| self.set_exception_breakpoints(args)),
			"dataBreakpointInfo" => arguments(args).and_then(|args| self.data_breakpoint_info(args)),
			"setDataBreakpoints" => arguments(args).and_then(|args| self.set_data_breakpoints(args)),
			"threads" => self.threads(),
			"stackTrace" => arguments(args).and_then(|args| self.stack_trace(args)),
			"scopes" => arguments(args).and_then(|args| self.scopes(args)),
			"variables" => arguments(args).and_then(|args| self.variables(args)),
			"continue" => self.resume(ContinueKind::Continue).and_then(|()| {
				json(ContinueResponse {
					allThreadsContinued: Some(true)
				})
			}),
			"next" => arguments::<StepArguments>(args).and_then(|args| {
				let by_instruction = args.by_instruction();
				self.step(
					|stack_id| {
						if by_instruction {
							ContinueKind::StepOverInstruction { stack_id }
						} else {
							ContinueKind::StepOver { stack_id }
						}
					},
					args.thread_id
				)
			}),
			"stepIn" => arguments::<StepArguments>(args).and_then(|args| {
				let by_instruction = args.by_instruction();
				self.step(
					|stack_id| {
						if by_instruction {
							ContinueKind::StepIntoInstruction { stack_id }
						} else {
							ContinueKind::StepInto { stack_id }
						}
					},
					args.thread_id
				)
			}),
			"stepOut" => arguments::<StepOutArguments>(args).and_then(|args| self.step(|stack_id| ContinueKind::StepOut { stack_id }, args.threadId)),
			"pause" => self.pause(),
			"evaluate" => arguments(args).and_then(|args| self.evaluate(args)),
			"completions" => arguments(args).and_then(|args| self.completions(args)),
			"setVariable" => arguments(args).and_then(|args| self.set_variable(args)),
			"setExpression" => arguments(args).and_then(|args| self.set_expression(args)),
			"exceptionInfo" => self.exception_info(),
			"source" => arguments(args).and_then(|args| self.source(args)),
			"breakpointLocations" => arguments(args).and_then(|args| self.breakpoint_locations(args)),
			"loadedSources" => self.loaded_sources(),
			"terminate" => self.terminate(),
			"disassemble" => arguments(args).and_then(|args| self.disassemble(args)),
			"disconnect" => {
				let args: Option<DisconnectArguments> = arguments(args).ok();
				self.disconnect(args.and_then(|args| args.terminateDebuggee));
				self.out.response(request.protocol_message.seq, command, Ok(Value::Null));
				return false;
			}
			_ => Err(format!("unsupported request: {}", command))
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
			supportsRestartRequest: Some(true),
			supportsBreakpointLocationsRequest: Some(true),
			supportsDataBreakpoints: Some(true),
			supportsLoadedSourcesRequest: Some(true),
			supportsDisassembleRequest: Some(true),
			..Default::default()
		})?;
		capabilities["supportsInstructionBreakpoints"] = true.into();
		capabilities["supportsSteppingGranularity"] = true.into();
		let condition = "Comma-separated message filters, e.g. \"bounds, null\". Prefix exclusions with !, e.g. \"!libdreamluau\".";
		capabilities["supportsExceptionFilterOptions"] = true.into();
		capabilities["supportsCompletionsRequest"] = true.into();
		capabilities["completionTriggerCharacters"] = serde_json::json!([".", ":"]);
		capabilities["exceptionBreakpointFilters"] = serde_json::json!([
			{
				"filter": RUNTIMES,
				"label": "Runtime errors",
				"description": "Pause on uncaught runtime errors and throws",
				"default": true,
				"supportsCondition": true,
				"conditionDescription": condition
			},
			{
				"filter": CAUGHT,
				"label": "Caught runtime errors",
				"description": "Also pause on runtime errors and throws inside try blocks",
				"default": false,
				"supportsCondition": true,
				"conditionDescription": condition
			}
		]);
		Ok(capabilities)
	}

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

	fn drop_code(&mut self) {
		self.code = None;
		self.code_worker += 1;
		self.progress_end(READING);
		self.progress_end(RELOADING);
		self.forget_refusals();
	}

	fn start_code(&mut self, settings: CodeSettings) {
		let dmb = match self.request(&AuxRequest::WorldInfo) {
			Ok(AuxResponse::WorldInfo { args, cwd, .. }) => local::world_dmb(&args, &cwd),
			_ => None
		};
		let dme = settings.dme.or_else(|| Some(dmb.as_ref()?.with_extension("dme")));
		*self.files.lock().unwrap() = files::WorldFiles::new(dme.as_ref().and_then(|dme| std::path::absolute(dme).ok()));
		self.hot_reload = settings.hot_reload;
		if !settings.hot_reload {
			self.code_read = true;
			return;
		}
		let Some(dme) = dme.filter(|dme| dme.is_file()) else {
			self.out.console("Hot reload disabled: project .dme not found");
			self.reload_state("off", Some("project .dme not found".to_owned()));
			self.code_read = true;
			return;
		};

		let options = code::Options {
			dmb: dmb.filter(|dmb| dmb.exists()).unwrap_or_else(|| dme.with_extension("dmb")),
			dme,
			defines: settings.defines.iter().map(|define| code::parse_define(define)).collect(),
			hot_reload: settings.hot_reload
		};
		self.progress_start(READING, "Reading source for hot reload".to_owned());
		self.reload_state("reading", None);
		self.code_worker += 1;
		self.code = Some(code::start(options, self.out.clone(), self.inputs.clone(), self.code_worker));
		if let Some(defines) = self.wanted_defines.clone() {
			self.give_defines(defines);
		}
	}

	fn build_flags_changed(&mut self, args: BuildFlagsArguments) -> Handled {
		let Some(launched) = self.world.launched.as_mut() else {
			return Ok(Value::Null);
		};
		launched.build_flags = args.flags;
		let defines = all_defines(launched);
		self.wanted_defines = Some(defines.clone());
		self.give_defines(defines);
		Ok(Value::Null)
	}

	fn give_defines(&self, defines: Vec<String>) {
		if let Some(code) = &self.code {
			let _ = code.send(code::Command::Defines(defines.iter().map(|define| code::parse_define(define)).collect()));
		}
	}

	fn code_event(&mut self, event: code::Event) {
		if !matches!(event, code::Event::Reload { .. }) {
			self.progress_end(READING);
		}
		match event {
			code::Event::Changed(texts) => self.code_changed(texts.as_ref()),
			code::Event::Failed(e) => {
				self.out.console(format!("Hot reload disabled: could not parse source: {}", e));
				self.reload_state("off", Some(format!("could not parse source: {}", e)));
				self.code_read = true;
				self.code = None;
				self.say_ready();
				if self.world.waiting_for_code {
					self.world.waiting_for_code = false;
					self.ready();
				}
			}
			code::Event::Ready => {
				if self.world.waiting_for_code {
					self.world.waiting_for_code = false;
					if let Some(texts) = self.caught_up.take() {
						self.reapply_breakpoints(Some(&texts));
					}
					self.ready();
				}
			}
			code::Event::Watching => {
				self.watching = true;
				self.reload_state("active", None);
				self.say_ready();
			}
			code::Event::Reloading => self.reloading(),
			code::Event::Looked => self.looked(),
			code::Event::Refusals(refusals) => self.outstanding(refusals),
			code::Event::Outdated(files) => self.outdated(files),
			code::Event::Reload {
				request,
				summary,
				notes,
				world,
				texts
			} => {
				let applied = if self.debuggee.as_ref().is_some_and(|d| d.generation != world) {
					self.out.console(format!(
						"Hot reload skipped because the world restarted after it was prepared: {}",
						summary.text(&[])
					));
					false
				} else {
					self.reload_seen = Some(std::time::SystemTime::now());
					self.reload(*request, &summary, notes)
				};
				if let Some(texts) = texts.filter(|_| applied) {
					self.code_changed(Some(&texts));
				}
				if let Some(code) = &self.code {
					let _ = code.send(code::Command::Resources(std::mem::take(&mut self.reloaded_resources)));
					let _ = code.send(code::Command::Reloaded(applied));
				}
			}
			code::Event::NotReloaded { refused, warnings, .. } if !refused.is_empty() => self.refused(refused, texts(&warnings)),
			code::Event::NotReloaded {
				summary, errors, warnings, ..
			} => {
				self.reload_state("failed", Some(summary.clone()));
				self.journal_reload(false, summary, errors, texts(&warnings));
			}
			code::Event::NothingChanged { warnings, texts: read } => {
				if let Some(read) = read {
					self.code_changed(Some(&read));
				}
				self.reload_state("reloaded", Some("no changes to the running code".to_owned()));
				self.journal_reload(true, "no runtime changes".to_owned(), vec![], texts(&warnings));
			}
			code::Event::Macros(macros) => {
				self.macros = Some(macros);
				self.give_macros();
			}
			code::Event::Skin {
				request,
				file,
				images,
				restored,
				world
			} => {
				let applied = self.debuggee.as_ref().is_some_and(|d| d.generation == world) && self.reload_skin(request, &file, &images, restored);
				match applied {
					true => self.reload_state("reloaded", Some(format!("skin {}", file.display()))),
					false => self.reload_state("failed", Some(format!("skin {}", file.display())))
				}
				if let Some(code) = &self.code {
					let _ = code.send(code::Command::Reloaded(applied));
				}
			}
			code::Event::Script { request, restored, world } => {
				let applied = self.debuggee.as_ref().is_some_and(|d| d.generation == world) && self.reload_script(request, restored);
				match applied {
					true => self.reload_state("reloaded", Some("client script".to_owned())),
					false => self.reload_state("failed", Some("client script".to_owned()))
				}
				if let Some(code) = &self.code {
					let _ = code.send(code::Command::Reloaded(applied));
				}
			}
		}
	}

	fn code_changed(&mut self, texts: Option<&code::Texts>) {
		self.world_as_of = self.reload_seen.take().or_else(|| Some(std::time::SystemTime::now()));
		if self.code_read && self.debuggee.is_some() && self.debugging {
			match (self.world.waiting_for_code, texts) {
				(false, _) => self.reapply_breakpoints(texts),
				(true, Some(texts)) => self.caught_up.get_or_insert_default().extend(texts.clone()),
				(true, None) => {}
			}
		}
		self.code_read = true;
	}

	fn give_macros(&mut self) {
		if let (Some(macros), Some(debuggee)) = (&self.macros, &mut self.debuggee) {
			let _ = debuggee.send(&AuxRequest::Defines(macros.clone()));
		}
	}

	fn reload(&mut self, request: aux::HotReloadRequest, summary: &code::Summary, notes: Vec<Diagnostic>) -> bool {
		self.reload_started = Some(Instant::now());
		let unlinked: Vec<String> = request
			.plan
			.removed_procs
			.iter()
			.filter(|removed| removed.type_path.is_empty())
			.map(|removed| removed.proc.path.clone())
			.collect();
		let result = match self.request(&AuxRequest::HotReload(request)) {
			Ok(AuxResponse::HotReload(result)) => result,
			Ok(other) => {
				let why = unexpected(other);
				let notes = texts(&notes);
				self.notes(&notes);
				self.out.console(format!("Hot reload failed: {}", why));
				self.reload_state("failed", Some(why.clone()));
				self.journal_reload(false, "reload failed".to_owned(), vec![why], notes);
				return false;
			}
			Err(e) => {
				let notes = texts(&notes);
				self.notes(&notes);
				self.out.console(format!("Hot reload failed: {}", e));
				self.reload_state("failed", Some(e.clone()));
				self.journal_reload(false, "reload failed".to_owned(), vec![e], notes);
				return false;
			}
		};
		if result.applied {
			self.follow_reloaded_breakpoints();
		}
		self.reloaded_resources = result.resources.clone();
		let summary = summary.text(&result.skipped);
		let notes: Vec<String> = notes
			.into_iter()
			.filter_map(|note| {
				let unlinked = note
					.subject
					.as_ref()
					.filter(|path| note.id == Id::RemovedProcRetained && unlinked.contains(path));
				let Some(path) = unlinked.filter(|_| result.applied) else {
					return Some(note.to_string());
				};
				if result.skipped.iter().any(|skipped| skipped == path) {
					return None;
				}
				Some(format!(
					"{}: removed from source. Calls by name no longer find it. Direct references, such as call(/proc/name)(), still run the old \
					 definition until the world restarts",
					path
				))
			})
			.collect();
		self.notes(&notes);
		let summary_of_outcome = if result.applied {
			summary.clone()
		} else {
			format!("reload failed: {}", summary)
		};
		self.journal_reload_failures(
			result.applied,
			summary_of_outcome,
			result.errors.clone(),
			notes.into_iter().chain(result.warnings.iter().cloned()).collect(),
			result.failures.clone()
		);

		let took = self.reload_started.take().map_or(0, |started| started.elapsed().as_millis());
		if !result.applied {
			self.out.console("Hot reload failed:");
			self.reload_state("failed", Some(result.errors.first().cloned().unwrap_or_default()));
		} else if let Some(failed) = result.failures.first() {
			self.out.console(format!(
				"Hot reload partly applied: {} in {} ms. {}",
				summary,
				took,
				auxtools_cli::reload::RESTART
			));
			self.reload_state("failed", Some(format!("partly applied: {}", failed)));
		} else {
			self.out.console(format!("Hot reload: {} in {} ms", summary, took));
			self.reload_state("reloaded", Some(summary.clone()));
		}
		for f in &result.failures {
			self.out.console(format!("    failed: {}", f));
		}
		for e in &result.errors {
			self.out.console(format!("    {}", e));
		}
		for w in &result.warnings {
			self.out.console(format!("    warning: {}", w));
		}
		result.applied
	}

	fn follow_reloaded_breakpoints(&mut self) {
		self.refused_at.clear();
		if !self.debugging || self.debuggee.is_none() {
			return;
		}
		let Ok(placed) = self.placed_breakpoints() else {
			return;
		};
		let mut changed = HashSet::new();
		let mut instructions = vec![];
		let mut find_again = vec![];
		for (key, breakpoints) in self.set.lock().unwrap().iter_mut() {
			let exact = key == INSTRUCTION_BREAKPOINTS;
			let by_name = key == FUNCTION_BREAKPOINTS;
			breakpoints.retain_mut(|breakpoint| {
				if breakpoint.at.is_none() {
					return true;
				}
				let now = placed.iter().find(|placed| placed.id == breakpoint.claim);
				let renumbered = |now: &aux::PlacedBreakpoint| breakpoint.at.as_ref().is_some_and(|at| at.proc != now.at.proc);
				if by_name && now.is_some_and(|now| now.unlinked || renumbered(now)) {
					find_again.push(breakpoint.claim);
					return false;
				}
				if now.is_some_and(|now| breakpoint.at.as_ref() == Some(&now.at) && breakpoint.file == now.file && breakpoint.line == now.line) {
					return true;
				}
				changed.insert(key.clone());
				if exact {
					instructions.push((breakpoint.id, now.is_none()));
				}
				let Some(now) = now else {
					return false;
				};
				breakpoint.at = Some(now.at.clone());
				breakpoint.file = now.file.clone();
				breakpoint.line = now.line;
				true
			});
		}
		changed.remove(INSTRUCTION_BREAKPOINTS);
		self.relocated.extend(changed);
		for claim in find_again {
			let _ = self.remove_breakpoint(claim);
		}
		self.retry_function_breakpoints();
		for (id, removed) in instructions {
			if removed {
				self.removed_by_reload.insert(id);
			}
			let Some((_, breakpoint)) = self.instruction_breakpoints.iter().find(|(wanted, _)| *wanted == id) else {
				continue;
			};
			let shown = self.instruction_shown(id, breakpoint);
			self.out.event(dap::BreakpointEvent {
				reason: "changed".to_owned(),
				breakpoint: shown
			});
		}
	}

	fn notes(&self, notes: &[String]) {
		for why in notes {
			self.out.console(format!("    note: {}", why));
		}
	}

	fn reapply_breakpoints(&mut self, texts: Option<&code::Texts>) {
		let files: Vec<String> = self.breakpoints.keys().cloned().collect();
		for file in files {
			let reloaded = texts.map(|texts| reloaded_text(texts, &file));
			let now = match reloaded {
				Some(Some(text)) => text.clone(),
				Some(None) if self.world_texts.contains_key(&file) => self.world_texts[&file].clone(),
				_ => crate::lines::read(&file)
			};
			let had = self.world_texts.insert(file.clone(), now.clone());
			if self.deferred.contains(&file) {
				continue;
			}
			if self.waiting.contains(&file) && matches!(reloaded, Some(None)) {
				continue;
			}
			if matches!(reloaded, Some(Some(None))) {
				let breakpoints = Self::waiting_for_the_world(&self.breakpoints[&file]);
				self.tell(breakpoints, true);
				self.waiting.insert(file);
				continue;
			}
			let then = self.breakpoint_texts.get(&file).cloned().flatten();
			if let (Some(then), Some(now)) = (&then, &now) {
				if then != now {
					let map = crate::lines::LineMap::new(then, now);
					for (_, breakpoint) in self.breakpoints.get_mut(&file).into_iter().flatten() {
						breakpoint.line = map.line(breakpoint.line as u32).into();
					}
				}
			}
			let wants_code = self.breakpoints[&file].iter().any(|(id, _)| match self.told.get(id) {
				Some((verified, _, message)) => !*verified && matches!(message.as_deref(), Some(NO_CODE | NO_LINES)),
				None => true
			});
			let changed = then != now || had.as_ref() != Some(&now) || self.waiting.contains(&file) || self.relocated.contains(&file);
			if !changed && !wants_code {
				continue;
			}
			self.waiting.remove(&file);
			self.breakpoint_texts.insert(file.clone(), now);

			let breakpoints = self.apply_breakpoints(&file);
			self.tell(breakpoints, changed);
		}
		self.retry_function_breakpoints();

		if self.client.supports_invalidated_event && self.debuggee.as_ref().is_some_and(Debuggee::stopped) {
			self.out.event(dap::InvalidatedEvent {
				areas: vec!["stacks".to_owned()]
			});
		}
	}

	fn debuggee(&mut self) -> Result<&mut Debuggee, String> {
		match self.world.unavailable {
			true => Err(UNAVAILABLE.to_owned()),
			false => self.debuggee.as_mut().ok_or_else(|| "debug server disconnected".to_owned())
		}
	}

	const fn not_connected(&self) -> &'static str {
		match self.world.unavailable {
			true => UNAVAILABLE,
			false => "world is not running"
		}
	}

	fn request(&mut self, request: &AuxRequest) -> Result<AuxResponse, String> {
		let leaving = self.answering.then(|| self.leaving.clone());
		self.debuggee()?.request(request, leaving.as_deref())
	}

	fn ack(&mut self, request: &AuxRequest) -> Result<(), String> {
		match self.request(request)? {
			AuxResponse::Ack => Ok(()),
			other => Err(unexpected(other))
		}
	}

	fn set_breakpoints(&mut self, args: SetBreakpointsArguments) -> Handled {
		let path = args.source.path.ok_or("breakpoint source is missing a path")?;
		let saved_since = args.sourceModified.unwrap_or(false);
		let reloading = self.code.is_some();
		let unsaved = !saved_since && reloading && self.unsaved.contains(&path);
		let text = crate::lines::read(&path);
		let same_lines = !unsaved && text.is_some() && self.breakpoint_texts.get(&path) == Some(&text);
		let mut had: Vec<(i64, i64)> = (self.breakpoints.get(&path).into_iter().flatten())
			.map(|(id, breakpoint)| (*id, breakpoint.line))
			.collect();
		for (id, _) in &had {
			self.told.remove(id);
		}
		let mut wanted = vec![];
		for breakpoint in args.breakpoints.unwrap_or_default() {
			let id = match had.iter().position(|(_, line)| same_lines && *line == breakpoint.line) {
				Some(at) => had.remove(at).0,
				None => self.new_breakpoint_id()
			};
			wanted.push((id, breakpoint));
		}

		if unsaved {
			let breakpoints = wanted
				.iter()
				.map(|(number, breakpoint)| Breakpoint {
					id: Some(*number),
					verified: false,
					message: Some("pending save and hot reload".to_owned()),
					line: Some(breakpoint.line),
					..Default::default()
				})
				.collect::<Vec<_>>();
			self.told(&breakpoints);
			if wanted.is_empty() {
				self.unset(&path);
			}
			self.world_texts.entry(path.clone()).or_insert_with(|| crate::lines::read(&path));
			self.breakpoints.insert(path.clone(), wanted);
			self.breakpoint_texts.insert(path.clone(), None);
			self.deferred.insert(path);
			return json(SetBreakpointsResponse { breakpoints });
		}

		if saved_since && !reloading && self.said_changed.insert(path.clone()) {
			self.out.console(format!(
				"{} has changed since the world was built, and hot reload is off. Its breakpoints follow the built code, so they may be on other \
				 lines",
				Path::new(&path).file_name().unwrap_or_default().to_string_lossy()
			));
		}
		self.deferred.remove(&path);
		if self.world_is_behind(&path, &text) {
			let breakpoints = Self::waiting_for_the_world(&wanted);
			self.told(&breakpoints);
			if wanted.is_empty() {
				self.unset(&path);
			}
			self.breakpoints.insert(path.clone(), wanted);
			self.breakpoint_texts.insert(path.clone(), text);
			self.waiting.insert(path);
			return json(SetBreakpointsResponse { breakpoints });
		}
		self.waiting.remove(&path);
		self.world_texts.entry(path.clone()).or_insert_with(|| text.clone());
		self.breakpoints.insert(path.clone(), wanted);
		self.breakpoint_texts.insert(path.clone(), text);
		let breakpoints = self.apply_breakpoints(&path);
		self.told(&breakpoints);
		json(SetBreakpointsResponse { breakpoints })
	}

	fn world_is_behind(&self, path: &str, text: &Option<Vec<String>>) -> bool {
		if self.code.is_none() || !self.code_read {
			return false;
		}
		match self.world_texts.get(path) {
			Some(world) => world != text,
			None => {
				let modified = std::fs::metadata(path).and_then(|file| file.modified()).ok();
				matches!((modified, self.world_as_of), (Some(modified), Some(as_of)) if modified > as_of)
			}
		}
	}

	fn waiting_for_the_world(wanted: &[(i64, SourceBreakpoint)]) -> Vec<Breakpoint> {
		wanted
			.iter()
			.map(|(number, breakpoint)| Breakpoint {
				id: Some(*number),
				verified: false,
				message: Some("pending hot reload".to_owned()),
				line: Some(breakpoint.line),
				..Default::default()
			})
			.collect()
	}

	fn told(&mut self, breakpoints: &[Breakpoint]) {
		for breakpoint in breakpoints {
			if let Some(id) = breakpoint.id {
				self.told.insert(id, (breakpoint.verified, breakpoint.line, breakpoint.message.clone()));
			}
		}
	}

	fn tell(&mut self, breakpoints: Vec<Breakpoint>, all: bool) {
		for breakpoint in breakpoints {
			if let Some(id) = breakpoint.id {
				let now = (breakpoint.verified, breakpoint.line, breakpoint.message.clone());
				if !all && self.told.get(&id) == Some(&now) {
					continue;
				}
				self.told.insert(id, now);
			}
			self.out.event(dap::BreakpointEvent {
				reason: "changed".to_owned(),
				breakpoint
			});
		}
	}

	fn unsaved_changed(&mut self, args: UnsavedArguments) -> Handled {
		let path = args.path;
		if args.unsaved {
			self.unsaved.insert(path);
			return Ok(Value::Null);
		}
		self.unsaved.remove(&path);
		if self.deferred.remove(&path) && self.breakpoints.contains_key(&path) {
			let text = crate::lines::read(&path);
			let behind = self.world_is_behind(&path, &text);
			self.breakpoint_texts.insert(path.clone(), text);
			if behind {
				self.waiting.insert(path.clone());
				let breakpoints = Self::waiting_for_the_world(self.breakpoints.get(&path).map_or(&[], Vec::as_slice));
				self.tell(breakpoints, false);
				return Ok(Value::Null);
			}
			let breakpoints = self.apply_breakpoints(&path);
			self.tell(breakpoints, true);
		}
		Ok(Value::Null)
	}

	fn world_file_name(&self, path: &str) -> String {
		self.files.lock().unwrap().name(path)
	}

	const fn new_breakpoint_id(&mut self) -> i64 {
		self.next_breakpoint_id += 1;
		self.next_breakpoint_id - 1
	}

	fn apply_breakpoints(&mut self, path: &str) -> Vec<Breakpoint> {
		self.relocated.remove(path);
		let mut live = self.set.lock().unwrap().remove(path).unwrap_or_default();
		let wanted: Vec<(i64, i64, Option<String>, Option<String>, Option<String>)> = self.breakpoints[path]
			.iter()
			.map(|(number, b)| (*number, b.line, b.condition.clone(), b.hitCondition.clone(), b.logMessage.clone()))
			.collect();
		let world_name = self.world_file_name(path);

		let mut breakpoints = vec![];
		let mut set = vec![];
		for (number, line, condition, hit, log) in wanted {
			let log = log.filter(|message| !message.is_empty());
			let unverified = |message: &str| Breakpoint {
				id: Some(number),
				verified: false,
				message: Some(message.to_owned()),
				line: Some(line),
				..Default::default()
			};

			if !self.debugging {
				breakpoints.push(unverified("debugging is disabled"));
				continue;
			}
			if self.debuggee.is_none() {
				breakpoints.push(unverified(self.not_connected()));
				continue;
			}
			let hit = match hit_condition(hit.as_deref()) {
				Ok(hit) => hit,
				Err(e) => {
					breakpoints.push(unverified(&e));
					continue;
				}
			};
			let trigger = Trigger { condition, hit };
			let mut result = None;
			let mut claims = vec![];

			let moved: Vec<SetBreakpoint> = live
				.extract_if(.., |b| {
					b.id == number && b.file.as_ref().is_some_and(|file| !same_file(file, &world_name))
				})
				.collect();
			for kept in moved {
				match self.keep(kept, &trigger, log.clone()) {
					Ok(Some(claimed)) => {
						result.get_or_insert(Ok((claimed.line, claimed.file.clone())));
						claims.push(claimed);
					}
					Ok(None) => {}
					Err(e) => {
						result.get_or_insert(Err(e));
					}
				}
			}

			if result.is_none() {
				let instructions = match self.locate(&world_name, line as u32) {
					Ok(instructions) if instructions.is_empty() => {
						breakpoints.push(unverified(if self.has_lines() { NO_CODE } else { NO_LINES }));
						continue;
					}
					Ok(instructions) => instructions,
					Err(e) => {
						breakpoints.push(unverified(&e));
						continue;
					}
				};
				for instruction in instructions {
					let kept = (live.iter())
						.position(|b| b.id == number && b.at.as_ref() == Some(&instruction))
						.map(|at| live.remove(at));
					let claimed = match kept.map(|kept| self.keep(kept, &trigger, log.clone())) {
						Some(Ok(None)) | None => self.claim(number, &instruction, &trigger, log.clone(), false),
						Some(kept) => kept
					};
					match claimed {
						Ok(Some(claimed)) => {
							result.get_or_insert(Ok((claimed.line, claimed.file.clone())));
							claims.push(claimed);
						}
						Ok(None) => {}
						Err(e) => {
							result.get_or_insert(Err(e));
						}
					}
				}
			}

			let at_start = claims
				.iter()
				.any(|claimed| claimed.at.as_ref().is_some_and(|at| at.proc.path == GLOBAL_INIT));
			set.extend(claims);
			breakpoints.push(match result {
				Some(Ok((set_line, file))) => Breakpoint {
					id: Some(number),
					verified: true,
					line: Some(set_line.unwrap_or(line as u32).into()),
					source: file.filter(|file| !same_file(file, &world_name)).map(|file| self.file_source(&file)),
					message: at_start.then(|| "This line runs only at world startup, to initialize globals or static vars.".to_owned()),
					..Default::default()
				},
				Some(Err(e)) => unverified(&e),
				None => unverified("could not set breakpoint")
			});
		}
		self.release(live);
		self.set.lock().unwrap().insert(path.to_owned(), set);
		breakpoints
	}

	fn claim(
		&mut self,
		id: i64,
		instruction: &InstructionRef,
		trigger: &Trigger,
		log: Option<String>,
		exact: bool
	) -> Result<Option<SetBreakpoint>, String> {
		self.claims += 1;
		let claim = self.claims;
		let (at, file, line) = match self.place_breakpoint(claim, instruction, trigger, log.as_deref().map(aux::LogPart::parse), exact)? {
			Placed::Set { at, file, line } => (at, file, line),
			Placed::Nowhere => return Ok(None),
			Placed::Refused(reason) => return Err(reason)
		};
		Ok(Some(SetBreakpoint {
			id,
			claim,
			at: Some(at),
			file,
			line,
			trigger: trigger.clone(),
			log,
			watch: None
		}))
	}

	fn keep(&mut self, mut kept: SetBreakpoint, trigger: &Trigger, log: Option<String>) -> Result<Option<SetBreakpoint>, String> {
		if kept.trigger == *trigger && kept.log == log {
			return Ok(Some(kept));
		}
		match self.edit_breakpoint(kept.claim, trigger, log.as_deref().map(aux::LogPart::parse))? {
			Placed::Set { at, file, line } => {
				kept.at = Some(at);
				kept.file = file;
				kept.line = line;
				kept.trigger = trigger.clone();
				kept.log = log;
				Ok(Some(kept))
			}
			Placed::Nowhere => Ok(None),
			Placed::Refused(reason) => {
				let _ = self.remove_breakpoint(kept.claim);
				Err(reason)
			}
		}
	}

	fn release(&mut self, breakpoints: Vec<SetBreakpoint>) {
		for breakpoint in breakpoints {
			let _ = self.remove_breakpoint(breakpoint.claim);
		}
	}

	fn unset(&mut self, key: &str) {
		let set = self.set.lock().unwrap().remove(key).unwrap_or_default();
		self.release(set);
	}

	fn set_function_breakpoints(&mut self, args: SetFunctionBreakpointsArguments) -> Handled {
		let mut had: Vec<(i64, String)> = (self.function_breakpoint_ids.iter())
			.zip(&self.function_breakpoints)
			.map(|(id, b)| (*id, b.name.clone()))
			.collect();
		for (id, _) in &had {
			self.told.remove(id);
		}
		self.function_breakpoint_ids = (args.breakpoints.iter())
			.map(|wanted| match had.iter().position(|(_, name)| *name == wanted.name) {
				Some(at) => had.remove(at).0,
				None => self.new_breakpoint_id()
			})
			.collect();
		self.function_breakpoints = args.breakpoints;
		let breakpoints = self.apply_function_breakpoints();
		self.told(&breakpoints);
		json(SetFunctionBreakpointsResponse { breakpoints })
	}

	fn apply_function_breakpoints(&mut self) -> Vec<Breakpoint> {
		let mut live = self.set.lock().unwrap().remove(FUNCTION_BREAKPOINTS).unwrap_or_default();
		let wanted: Vec<(i64, String, Option<String>, Option<String>)> = self
			.function_breakpoints
			.iter()
			.zip(&self.function_breakpoint_ids)
			.map(|(b, id)| (*id, b.name.clone(), b.condition.clone(), b.hitCondition.clone()))
			.collect();

		let mut breakpoints = vec![];
		let mut set = vec![];
		for (id, name, condition, hit) in wanted {
			if !self.debugging || self.debuggee.is_none() {
				breakpoints.push(Breakpoint {
					id: Some(id),
					verified: false,
					message: Some("no active debugging connection".to_owned()),
					..Default::default()
				});
				continue;
			}
			let (breakpoint, in_world) = self.function_breakpoint(id, &name, condition, hit.as_deref(), &mut live);
			breakpoints.push(breakpoint);
			set.extend(in_world);
		}
		self.release(live);
		self.set.lock().unwrap().insert(FUNCTION_BREAKPOINTS.to_owned(), set);
		breakpoints
	}

	fn function_breakpoint(
		&mut self,
		id: i64,
		name: &str,
		condition: Option<String>,
		hit: Option<&str>,
		live: &mut Vec<SetBreakpoint>
	) -> (Breakpoint, Option<SetBreakpoint>) {
		let instruction = InstructionRef {
			proc: named_proc(name),
			offset: 0
		};
		let claimed = hit_condition(hit).and_then(|hit| {
			let trigger = Trigger { condition, hit };
			let kept = live.iter().position(|b| b.id == id).map(|at| live.remove(at));
			match kept.map(|kept| self.keep(kept, &trigger, None)) {
				Some(Ok(None)) | None => self.claim(id, &instruction, &trigger, None, false),
				Some(kept) => kept
			}
		});
		let unverified = |message: String| Breakpoint {
			id: Some(id),
			verified: false,
			message: Some(message),
			..Default::default()
		};
		match claimed {
			Ok(Some(claimed)) => {
				let breakpoint = Breakpoint {
					id: Some(id),
					verified: true,
					line: claimed.line.map(i64::from),
					..Default::default()
				};
				(breakpoint, Some(claimed))
			}
			Ok(None) => (unverified(format!("proc not found: {}", name)), None),
			Err(e) => (unverified(e), None)
		}
	}

	fn retry_function_breakpoints(&mut self) {
		if !self.debugging || self.debuggee.is_none() {
			return;
		}
		let unset: Vec<(i64, String, Option<String>, Option<String>)> = {
			let set = self.set.lock().unwrap();
			let in_world = set.get(FUNCTION_BREAKPOINTS).map_or(&[][..], Vec::as_slice);
			self.function_breakpoints
				.iter()
				.zip(&self.function_breakpoint_ids)
				.filter(|(_, id)| !in_world.iter().any(|b| b.id == **id))
				.map(|(b, id)| (*id, b.name.clone(), b.condition.clone(), b.hitCondition.clone()))
				.collect()
		};
		let mut told = vec![];
		for (id, name, condition, hit) in unset {
			let (breakpoint, in_world) = self.function_breakpoint(id, &name, condition, hit.as_deref(), &mut vec![]);
			if self.debuggee.as_ref().is_some_and(Debuggee::lost) {
				break;
			}
			if let Some(in_world) = in_world {
				let mut set = self.set.lock().unwrap();
				set.entry(FUNCTION_BREAKPOINTS.to_owned()).or_default().push(in_world);
			}
			told.push(breakpoint);
		}
		self.tell(told, false);
	}

	fn requested_address(breakpoint: &InstructionBreakpoint) -> Option<u64> {
		parse_address(&breakpoint.instruction_reference).map(|address| address.wrapping_add_signed(breakpoint.offset.unwrap_or(0)))
	}

	fn live_address(&self, id: i64, breakpoint: &InstructionBreakpoint) -> Option<u64> {
		let set = self.set.lock().unwrap();
		let claimed = set.get(INSTRUCTION_BREAKPOINTS).and_then(|set| set.iter().find(|b| b.id == id));
		let at = claimed.and_then(|b| b.at.as_ref()).or_else(|| self.refused_at.get(&id))?;
		Some(Self::requested_address(breakpoint)? & !0xFFFF | u64::from(at.offset))
	}

	fn set_instruction_breakpoints(&mut self, args: InstructionBreakpointsArguments) -> Handled {
		type Had = (i64, InstructionBreakpoint, Option<u64>);
		let had: Vec<Had> = (self.instruction_breakpoints.iter())
			.map(|(id, breakpoint)| (*id, breakpoint.clone(), self.live_address(*id, breakpoint)))
			.collect();
		let asked = |breakpoint: &InstructionBreakpoint, (_, old, _): &Had| match Self::requested_address(breakpoint) {
			Some(address) => Self::requested_address(old) == Some(address),
			None => old.instruction_reference == breakpoint.instruction_reference && old.offset == breakpoint.offset
		};
		let moved = |breakpoint: &InstructionBreakpoint, (_, _, live): &Had| live.is_some() && *live == Self::requested_address(breakpoint);
		let pair = |order: [&dyn Fn(&InstructionBreakpoint, &Had) -> bool; 2]| {
			let mut taken = vec![false; had.len()];
			let mut pairs: Vec<Option<usize>> = vec![None; args.breakpoints.len()];
			for same in order {
				for (breakpoint, pair) in args.breakpoints.iter().zip(&mut pairs) {
					if pair.is_none() {
						*pair = (0..had.len()).find(|&i| !taken[i] && same(breakpoint, &had[i]));
						if let Some(i) = *pair {
							taken[i] = true;
						}
					}
				}
			}
			pairs
		};
		let (by_asked, by_moved) = (pair([&asked, &moved]), pair([&moved, &asked]));
		let count = |pairs: &[Option<usize>]| pairs.iter().flatten().count();
		let pairs = if count(&by_moved) > count(&by_asked) { by_moved } else { by_asked };
		let mut wanted = vec![];
		for (breakpoint, pair) in args.breakpoints.into_iter().zip(pairs) {
			let id = match pair {
				Some(i) => had[i].0,
				None => self.new_breakpoint_id()
			};
			wanted.push((id, breakpoint));
		}
		self.instruction_breakpoints = wanted;
		let breakpoints = self.apply_instruction_breakpoints();
		Ok(serde_json::json!({ "breakpoints": breakpoints }))
	}

	fn instruction_shown(&self, id: i64, breakpoint: &InstructionBreakpoint) -> InstructionShown {
		let reference = breakpoint.instruction_reference.clone();
		if self.removed_by_reload.contains(&id) {
			return InstructionShown::unverified(id, reference, REMOVED_BY_RELOAD.to_owned());
		}
		let line = (self.set.lock().unwrap().get(INSTRUCTION_BREAKPOINTS))
			.and_then(|set| set.iter().find(|b| b.id == id))
			.map(|b| b.line);
		match (line, self.live_address(id, breakpoint)) {
			(Some(line), Some(address)) => InstructionShown {
				id,
				verified: true,
				message: None,
				line,
				instruction_reference: address_text(address)
			},
			_ => InstructionShown::unverified(id, reference, "could not set breakpoint".to_owned())
		}
	}

	fn apply_instruction_breakpoints(&mut self) -> Vec<InstructionShown> {
		let wanted = self.instruction_breakpoints.clone();
		self.removed_by_reload.retain(|id| wanted.iter().any(|(wanted, _)| wanted == id));
		self.refused_at.retain(|id, _| wanted.iter().any(|(wanted, _)| wanted == id));
		let mut live = self.set.lock().unwrap().remove(INSTRUCTION_BREAKPOINTS).unwrap_or_default();
		let mut breakpoints = vec![];
		let mut set = vec![];
		let mut code: HashMap<aux::CodeRef, Option<aux::Disassembly>> = HashMap::new();
		for (id, breakpoint) in wanted {
			let reference = breakpoint.instruction_reference.clone();
			let unverified = |message: String| InstructionShown::unverified(id, reference.clone(), message);
			if !self.debugging || self.debuggee.is_none() {
				breakpoints.push(unverified("no active debugging connection".to_owned()));
				continue;
			}
			if self.removed_by_reload.contains(&id) {
				breakpoints.push(unverified(REMOVED_BY_RELOAD.to_owned()));
				continue;
			}
			let Some(address) = Self::requested_address(&breakpoint) else {
				breakpoints.push(unverified(format!("invalid instruction address: {:?}", reference)));
				continue;
			};
			let kept = live.iter().position(|b| b.id == id).map(|at| live.remove(at));
			let hit = match hit_condition(breakpoint.hit_condition.as_deref()) {
				Ok(hit) => hit,
				Err(e) => {
					if let Some(kept) = kept {
						self.refused_at.extend(kept.at.map(|at| (id, at)));
						let _ = self.remove_breakpoint(kept.claim);
					}
					breakpoints.push(unverified(e));
					continue;
				}
			};
			let trigger = Trigger {
				condition: breakpoint.condition.clone(),
				hit
			};
			let shown = |claimed: &SetBreakpoint| InstructionShown {
				id,
				verified: true,
				message: None,
				line: claimed.line,
				instruction_reference: address_text(address & !0xFFFF | u64::from(claimed.at.as_ref().map_or(0, |at| at.offset)))
			};

			if let Some(kept) = kept {
				let at = kept.at.clone();
				breakpoints.push(match self.keep(kept, &trigger, None) {
					Ok(Some(claimed)) => {
						let reply = shown(&claimed);
						set.push(claimed);
						reply
					}
					Ok(None) => {
						self.removed_by_reload.insert(id);
						unverified(REMOVED_BY_RELOAD.to_owned())
					}
					Err(e) => {
						self.refused_at.extend(at.map(|at| (id, at)));
						unverified(e)
					}
				});
				continue;
			}
			if let Some(at) = self.refused_at.get(&id).cloned() {
				breakpoints.push(match self.claim(id, &at, &trigger, None, true) {
					Ok(Some(claimed)) => {
						self.refused_at.remove(&id);
						let reply = shown(&claimed);
						set.push(claimed);
						reply
					}
					Ok(None) => unverified("could not set breakpoint".to_owned()),
					Err(e) => unverified(e)
				});
				continue;
			}

			let (place, offset) = aux::from_address(address);
			if place.old != 0 {
				breakpoints.push(unverified("cannot set a breakpoint in pre-reload bytecode".to_owned()));
				continue;
			}
			if !code.contains_key(&place) {
				let disassembly = match self.request(&AuxRequest::Disassemble { code: place }) {
					Ok(AuxResponse::Disassemble(Ok(disassembly))) => Some(disassembly),
					_ => None
				};
				code.insert(place, disassembly);
			}
			let Some(disassembly) = &code[&place] else {
				breakpoints.push(unverified(format!("no proc at address {}", address_text(address))));
				continue;
			};
			let proc = disassembly.proc.clone();
			let starts = disassembly
				.instructions
				.iter()
				.rev()
				.find(|instruction| instruction.offset <= offset)
				.map(|instruction| instruction.offset);
			let past_end = disassembly
				.instructions
				.last()
				.is_none_or(|last| usize::from(offset) >= usize::from(last.offset) + last.words.len());
			match starts {
				_ if past_end => {
					breakpoints.push(unverified(format!("{} is past the end of {}", address_text(address), proc.path)));
					continue;
				}
				Some(start) if start != offset => {
					breakpoints.push(unverified(format!(
						"{} is in the middle of an instruction of {}, which starts at {}",
						address_text(address),
						proc.path,
						address_text(aux::address(place, start))
					)));
					continue;
				}
				_ => {}
			}

			let instruction = InstructionRef { proc, offset: offset.into() };
			breakpoints.push(match self.claim(id, &instruction, &trigger, None, true) {
				Ok(Some(claimed)) => {
					let reply = shown(&claimed);
					set.push(claimed);
					reply
				}
				Ok(None) => unverified("could not set breakpoint".to_owned()),
				Err(e) => unverified(e)
			});
		}
		self.release(live);
		self.set.lock().unwrap().insert(INSTRUCTION_BREAKPOINTS.to_owned(), set);
		breakpoints
	}

	fn set_exception_breakpoints(&mut self, args: ExceptionBreakpointsArguments) -> Handled {
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

	fn catch_runtimes(&mut self) -> Result<(), String> {
		let (uncaught, caught) = (self.uncaught.clone(), self.caught.clone());
		self.debuggee()?.send(&AuxRequest::CatchRuntimes { uncaught, caught })
	}

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
			breakMode: if error.caught {
				ExceptionBreakMode::Always
			} else {
				ExceptionBreakMode::Unhandled
			},
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
			let limit = self.sleeping_threads as usize;
			match self.request(&AuxRequest::Stacks {
				sleeping: Some(self.sleeping_threads)
			})? {
				AuxResponse::Stacks { stacks, sleeping } => {
					let asleep = sleeping as usize;
					let mut listed = 0;
					for stack in stacks {
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
							"{} procs are asleep. The threads list shows the first {}. Set \"sleepingThreads\" in the launch configuration to show \
							 more or fewer",
							asleep, listed
						));
					}
				}
				other => return Err(unexpected(other))
			}
		}

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
				name: if frame.old_code {
					format!("{} (old code)", proc.path)
				} else {
					proc.path.clone()
				},
				instructionPointerReference: Some(address_text(aux::address(frame.code, frame.instruction.offset as u16))),
				column: if i == 0 { 1 } else { 0 },
				..Default::default()
			};

			match &frame.file {
				Some(file) if file == HELPER_FILE => dap_frame.presentationHint = Some(StackFramePresentationHint::Subtle),
				Some(file) => dap_frame.source = Some(self.file_source(file)),
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
		let scopes = [
			("Locals", locals, Some("locals")),
			("Arguments", arguments, Some("arguments")),
			("Globals", globals, None)
		]
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
		if !self.debugging {
			return Err("debugging is disabled".to_owned());
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
		if args.context.as_deref() == Some("hover") {
			if let Some(why) = hover_refuses(&args.expression) {
				return Err(why.to_owned());
			}
		}
		let hex = args.format.is_some_and(|format| format.hex == Some(true));
		let frame_id = args.frameId.map(|id| id as u32);
		let request = match args.context.as_deref() {
			Some("hover") => AuxRequest::Read {
				frame_id,
				expression: args.expression
			},
			_ => AuxRequest::Eval {
				frame_id,
				command: args.expression,
				ending: false
			}
		};
		match self.request(&request)? {
			AuxResponse::Eval(Ok(result)) => json(EvaluateResponse {
				result: shown(result.value, hex),
				type_: result.kind,
				variablesReference: result.variables.map_or(0, |vars| vars.0.into()),
				indexedVariables: result.entries.map(|entries| entries as usize),
				..Default::default()
			}),
			AuxResponse::Eval(Err(why)) => Err(why.message),
			other => Err(unexpected(other))
		}
	}

	fn completions(&mut self, args: CompletionsArguments) -> Handled {
		let line = args
			.text
			.split('\n')
			.nth(args.line.map_or(0, |line| line.max(1) as usize - 1))
			.unwrap_or_default();
		let text = line.strip_suffix('\r').unwrap_or(line);
		let before = args.column.max(1) as usize - 1;
		let cursor = text
			.char_indices()
			.scan(0, |units, (at, c)| {
				let here = *units;
				*units += c.len_utf16();
				Some((at, here))
			})
			.find(|(_, units)| *units >= before)
			.map_or(text.len(), |(at, _)| at);
		let Some(place) = completion_place(&text[..cursor]) else {
			return Ok(serde_json::json!({ "targets": [] }));
		};
		let prefix = &text[place.start..cursor];
		let (start, length) = (text[..place.start].encode_utf16().count() + 1, prefix.encode_utf16().count());
		let target = |label: &str, kind: &str| serde_json::json!({ "label": label, "type": kind, "start": start, "length": length });

		if place.command {
			let targets: Vec<Value> = ["dis", "disassemble", "help"]
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

	fn set_expression(&mut self, args: SetExpressionArguments) -> Handled {
		let hex = args.format.is_some_and(|format| format.hex == Some(true));
		match self.request(&AuxRequest::Eval {
			frame_id: args.frame_id.map(|id| id as u32),
			command: format!("{} = ({})", args.expression, args.value),
			ending: false
		})? {
			AuxResponse::Eval(Ok(result)) => Ok(serde_json::json!({
				"value": shown(result.value, hex),
				"type": result.kind,
				"variablesReference": result.variables.map_or(0, |vars| vars.0),
				"indexedVariables": result.entries
			})),
			AuxResponse::Eval(Err(why)) => Err(why.message),
			other => Err(unexpected(other))
		}
	}

	fn disassemble(&mut self, args: DisassembleArguments) -> Handled {
		let count = usize::try_from(args.instructionCount).map_err(|_| "the instruction count is negative".to_owned())?;
		let reference = parse_address(&args.memoryReference).ok_or_else(|| format!("invalid instruction address: {:?}", args.memoryReference))?;
		let reference = reference.wrapping_add_signed(args.offset.unwrap_or(0));
		let (code, offset) = aux::from_address(reference);
		let disassembly = match self.request(&AuxRequest::Disassemble { code })? {
			AuxResponse::Disassemble(Ok(disassembly)) => Some(disassembly),
			AuxResponse::Disassemble(Err(_)) => None,
			other => return Err(unexpected(other))
		};

		let empty = vec![];
		let listed = disassembly.as_ref().map_or(&empty, |disassembly| &disassembly.instructions);
		let file = disassembly
			.as_ref()
			.filter(|_| code.old == 0)
			.and_then(|disassembly| disassembly.file.clone());
		let symbol = disassembly.as_ref().map(|disassembly| {
			let path = &disassembly.proc.path;
			if code.old != 0 {
				format!("{} (old code)", path)
			} else {
				path.clone()
			}
		});
		let error = disassembly.as_ref().and_then(|disassembly| disassembly.error.clone());

		let instructions: Vec<DisassembledInstruction> =
			instruction_window(reference, code, offset, listed, args.instructionOffset.unwrap_or(0), count)
				.into_iter()
				.map(|place| match place {
					Ok(index) => {
						let instruction = &listed[index];
						let text = match (&instruction.text, &error) {
							(text, _) if !text.is_empty() => text.clone(),
							(_, Some(error)) => format!("(not read: {})", error),
							(_, None) => "(not read)".to_owned()
						};
						DisassembledInstruction {
							address: address_text(aux::address(code, instruction.offset)),
							instructionBytes: Some(instruction.words.iter().map(|word| format!("{:08X}", word)).collect::<Vec<_>>().join(" ")),
							instruction: text,
							symbol: symbol.clone(),
							location: file.as_deref().filter(|_| instruction.line.is_some()).map(|file| self.file_source(file)),
							line: instruction.line.map(i64::from),
							..Default::default()
						}
					}
					Err(address) => DisassembledInstruction {
						address: address_text(address),
						instruction: String::new(),
						..Default::default()
					}
				})
				.collect();

		let mut instructions = json(instructions)?;
		for instruction in instructions.as_array_mut().into_iter().flatten() {
			if instruction["instructionBytes"].is_null() {
				instruction["presentationHint"] = "invalid".into();
			}
		}
		Ok(serde_json::json!({ "instructions": instructions }))
	}

	fn file_source(&self, file: &str) -> Source {
		source_at(&self.files.lock().unwrap().path(file))
	}

	fn source(&mut self, args: SourceArguments) -> Handled {
		let reference = args.source.and_then(|source| source.sourceReference).unwrap_or(args.sourceReference);
		match (reference, &self.stddef) {
			(STDDEF_REFERENCE, Some(stddef)) => json(SourceResponse {
				content: stddef.text.clone(),
				mimeType: None
			}),
			_ => Err("source not found".to_owned())
		}
	}

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

	fn loaded_sources(&mut self) -> Handled {
		if !self.debugging || self.debuggee.is_none() {
			return Ok(serde_json::json!({ "sources": [] }));
		}
		let files = match self.request(&AuxRequest::Files)? {
			AuxResponse::Files { files } => files,
			other => return Err(unexpected(other))
		};
		let mut paths: Vec<PathBuf> = files
			.into_iter()
			.filter(|file| file != HELPER_FILE)
			.map(|file| self.files.lock().unwrap().path(&file))
			.collect();
		paths.sort();
		paths.dedup();
		let mut sources: Vec<Source> = paths.iter().map(|path| source_at(path)).collect();
		if self.stddef.is_some() {
			sources.push(Source {
				name: Some("stddef.dm".to_owned()),
				sourceReference: Some(STDDEF_REFERENCE),
				..Default::default()
			});
		}
		json(serde_json::json!({ "sources": sources }))
	}
}

fn runtime_place(line: &str) -> Option<(&str, i64)> {
	let (file, number) = line.trim().strip_prefix("source file: ")?.rsplit_once(',')?;
	Some((file, number.trim().parse().ok()?))
}

#[derive(Debug, PartialEq, Eq)]
struct CompletionPlace<'a> {
	start: usize,
	base: Option<&'a str>,
	command: bool
}

fn completion_place(text: &str) -> Option<CompletionPlace<'_>> {
	let name_char = |c: char| c.is_ascii_alphanumeric() || c == '_';
	if text.chars().filter(|&c| c == '"').count() % 2 == 1 {
		return None;
	}
	let start = text.trim_end_matches(name_char).len();
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

fn hover_refuses(code: &str) -> Option<&'static str> {
	dmasm::compiler::is_body(code).then_some("hover requires a single expression")
}

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

fn hit_condition(text: Option<&str>) -> Result<Option<aux::HitCondition>, String> {
	match text.map(str::trim).filter(|text| !text.is_empty()) {
		Some(text) => aux::HitCondition::parse(text).map(Some),
		None => Ok(None)
	}
}

fn same_file(one: &str, other: &str) -> bool {
	one.replace('\\', "/").eq_ignore_ascii_case(&other.replace('\\', "/"))
}

fn address_text(address: u64) -> String {
	format!("0x{:016X}", address)
}

fn parse_address(text: &str) -> Option<u64> {
	let text = text.trim();
	match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
		Some(hex) => u64::from_str_radix(hex, 16).ok(),
		None => text.parse().ok()
	}
}

fn instruction_window(
	reference: u64,
	code: aux::CodeRef,
	offset: u16,
	instructions: &[aux::BytecodeInstruction],
	shift: i64,
	count: usize
) -> Vec<Result<usize, u64>> {
	let Some(last) = instructions.last() else {
		return (0..count as i64).map(|k| Err(reference.wrapping_add_signed(shift + k))).collect();
	};
	let start = aux::address(code, 0);
	let length = i64::from(last.offset) + last.words.len() as i64;
	let listed = instructions.len() as i64;
	let at = if i64::from(offset) >= length {
		listed + i64::from(offset) - length
	} else {
		instructions.partition_point(|instruction| instruction.offset <= offset) as i64 - 1
	};
	(0..count as i64)
		.map(|k| {
			let index = at + shift + k;
			if index < 0 {
				Err(start.wrapping_add_signed(index))
			} else if index >= listed {
				Err(start.wrapping_add_signed(length + index - listed))
			} else {
				Ok(index as usize)
			}
		})
		.collect()
}

fn named_proc(name: &str) -> ProcRef {
	let (path, override_id) = match name.split_once('#') {
		Some((path, id)) => (path.trim().to_owned(), id.trim().parse().unwrap_or(0)),
		None => (name.trim().to_owned(), 0)
	};
	ProcRef { path, override_id }
}

fn source_at(path: &Path) -> Source {
	Source {
		name: path.file_name().map(|name| name.to_string_lossy().into_owned()),
		path: Some(path.to_string_lossy().into_owned()),
		..Default::default()
	}
}

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
	Adapter::unexpected(&response)
}

fn texts(diagnostics: &[Diagnostic]) -> Vec<String> {
	diagnostics.iter().map(ToString::to_string).collect()
}

impl Asks for Adapter {
	type Error = String;

	fn ask(&mut self, request: &AuxRequest) -> Result<AuxResponse, String> {
		self.request(request)
	}

	fn unexpected(response: &AuxResponse) -> String {
		format!("unexpected debug server response: {:?}", response)
	}
}

fn reloaded_text<'a>(texts: &'a code::Texts, file: &str) -> Option<&'a Option<Vec<String>>> {
	if let Some(text) = texts.get(Path::new(file)) {
		return Some(text);
	}
	let file = std::fs::canonicalize(file).ok()?;
	texts
		.iter()
		.find(|(path, _)| std::fs::canonicalize(path).is_ok_and(|path| path == file))
		.map(|(_, text)| text)
}

#[cfg(test)]
mod tests {
	use std::path::Path;

	use auxtools_protocol::{HitCondition, LogPart, RuntimeFilter};

	use super::{completion_place, hover_refuses, instruction_window, named_proc, parse_address, reloaded_text, runtime_place, shown};

	#[test]
	fn a_reloaded_file_is_found_by_another_spelling_of_its_path() {
		let dir = std::env::temp_dir().join(format!("auxtools-reloaded-{}", std::process::id()));
		std::fs::create_dir_all(dir.join("code")).unwrap();
		std::fs::write(dir.join("code/a.dm"), "").unwrap();
		std::fs::write(dir.join("code/b.dm"), "").unwrap();
		let text = Some(vec!["/proc/a()".to_owned()]);
		let texts = crate::code::Texts::from([(dir.join("code/a.dm"), text.clone())]);
		let spelled = |path: &Path| path.to_string_lossy().into_owned();
		assert_eq!(reloaded_text(&texts, &spelled(&dir.join("code/a.dm"))), Some(&text));
		assert_eq!(reloaded_text(&texts, &spelled(&dir.join("code/../code/a.dm"))), Some(&text));
		assert_eq!(reloaded_text(&texts, &spelled(&dir.join("code/b.dm"))), None);
		let _ = std::fs::remove_dir_all(&dir);
	}

	#[test]
	fn a_disconnect_is_known_as_it_is_read() {
		let asks = super::asks_to_disconnect;
		assert!(asks(
			r#"{"seq":9,"type":"request","command":"disconnect","arguments":{"terminateDebuggee":true}}"#
		));
		assert!(asks(r#"{"command":"disconnect"}"#));
		assert!(!asks(
			r#"{"seq":4,"type":"request","command":"evaluate","arguments":{"expression":""disconnect""}}"#
		));
		assert!(!asks(r#"{"seq":5,"type":"request","command":"terminate"}"#));
		assert!(!asks("disconnect"));
	}

	#[test]
	fn missing_arguments_are_empty() {
		assert!(super::arguments::<super::join::JoinArguments>(None).is_ok());
		assert!(super::arguments::<super::join::JoinArguments>(Some(serde_json::Value::Null)).is_ok());
	}

	#[test]
	fn byond_says_where_a_runtime_error_was() {
		assert_eq!(runtime_place("  source file: code/game/mob.dm,42"), Some(("code/game/mob.dm", 42)));
		assert_eq!(runtime_place("  source file: a, b.dm,7\r\n"), Some(("a, b.dm", 7)));
		for other in [
			"runtime error: bad",
			"proc name: fail (/proc/fail)",
			"  source file: mob.dm",
			"source file: mob.dm,x",
			"  usr: null"
		] {
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
		for text in [
			"get_mob().na",
			"(a).b",
			"items[f()].na",
			"\"text",
			"\"a\" + \"b.",
			"12",
			"x + 3",
			".na",
			"a..b",
			"]."
		] {
			assert_eq!(completion_place(text), None, "{}", text);
		}
	}

	#[test]
	fn a_hover_runs_no_body() {
		for body in ["again:\ngoto again", "var/x\nx", "return n", "n; n", "x\ny"] {
			assert_eq!(hover_refuses(body), Some("hover requires a single expression"), "{:?}", body);
		}
		for code in ["x", "src.n;", "x = 1"] {
			assert_eq!(hover_refuses(code), None, "{:?}", code);
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
		assert_eq!(LogPart::parse("x is {x}, y is { y + 1 }!"), [
			text("x is "),
			expr("x"),
			text(", y is "),
			expr("y + 1"),
			text("!")
		]);
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
	fn doubled_braces_are_literal() {
		let text = |text: &str| LogPart::Text(text.to_owned());
		let expr = |text: &str| LogPart::Expr(text.to_owned());
		assert_eq!(LogPart::parse("{{x}}"), [text("{x}")]);
		assert_eq!(LogPart::parse("{{\"a\": {a}}}"), [text("{\"a\": "), expr("a"), text("}")]);
		assert_eq!(LogPart::parse("{list(1) ~= list(1)}}}"), [expr("list(1) ~= list(1)"), text("}")]);
		assert_eq!(LogPart::parse("{a {{b}} c}"), [expr("a {{b}} c")]);
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

	fn three() -> Vec<auxtools_protocol::BytecodeInstruction> {
		[(0, 2), (2, 1), (3, 3)]
			.into_iter()
			.map(|(offset, length)| auxtools_protocol::BytecodeInstruction {
				offset,
				words: vec![0; length],
				text: "x".to_owned(),
				line: None
			})
			.collect()
	}

	#[test]
	fn an_address_is_the_proc_its_code_and_the_offset() {
		use auxtools_protocol::{address, from_address, CodeRef};
		let code = CodeRef { proc: 0x2a, old: 3 };
		assert_eq!(address(code, 0x10), 0x0000_002a_0003_0010);
		assert_eq!(from_address(0x0000_002a_0003_0010), (code, 0x10));
		assert_eq!(parse_address("0x0000002A00030010"), Some(0x0000_002a_0003_0010));
		assert_eq!(parse_address("16"), Some(16));
		assert_eq!(parse_address("nowhere"), None);
	}

	#[test]
	fn a_window_has_as_many_places_as_asked_for_in_order() {
		use auxtools_protocol::{address, CodeRef};
		let code = CodeRef { proc: 7, old: 0 };
		let start = address(code, 0);
		let listed = three();
		let window = instruction_window(start + 2, code, 2, &listed, -3, 6);
		assert_eq!(window, [Err(start - 2), Err(start - 1), Ok(0), Ok(1), Ok(2), Err(start + 6)]);
		assert_eq!(instruction_window(start + 1, code, 1, &listed, 0, 2), [Ok(0), Ok(1)]);
		assert_eq!(instruction_window(start + 7, code, 7, &listed, -2, 3), [
			Ok(2),
			Err(start + 6),
			Err(start + 7)
		]);
		assert_eq!(instruction_window(100, code, 100, &[], -1, 3), [Err(99), Err(100), Err(101)]);
	}
}
