use serde::{Deserialize, Serialize};

pub const WORLD_SHUTTING_DOWN: &str = "The world is shutting down";

pub const GLOBAL_INIT: &str = "(global init)";

pub const HELPER_FILE: &str = "auxtools_launcher.dm";

#[derive(Serialize, Deserialize, Debug)]
pub enum Request {
	Disconnect,
	Configured,
	StdDef,
	Eval {
		frame_id: Option<u32>,
		command: String,
		ending: bool
	},
	Read {
		frame_id: Option<u32>,
		expression: String
	},
	BreakpointSet {
		instruction: InstructionRef,
		condition: Option<String>,
		hit: Option<HitCondition>,
		log: Option<Vec<LogPart>>,
		exact: bool,
		id: u32
	},
	BreakpointUnset {
		id: u32
	},
	CatchRuntimes {
		uncaught: Option<RuntimeFilter>,
		caught: Option<RuntimeFilter>
	},
	Offset {
		proc: ProcRef,
		line: u32
	},
	Stacks {
		sleeping: Option<u32>
	},
	StackFrames {
		stack_id: u32,
		start_frame: Option<u32>,
		count: Option<u32>
	},
	Scopes {
		frame_id: u32
	},
	Variables {
		vars: VariablesRef,
		start: Option<u32>,
		count: Option<u32>
	},
	Continue {
		kind: ContinueKind
	},
	Pause,

	ByondVersion,
	HotReload(Box<HotReloadRequest>),
	WorldInfo,
	Status,
	HoldPause,
	Locate {
		file: String,
		line: u32
	},
	SetVariable {
		vars: VariablesRef,
		name: String,
		value: String
	},
	Complete {
		frame_id: Option<u32>,
		base: Option<String>,
		prefix: String
	},
	Watcher,
	LinesWithCode {
		file: String,
		start: u32,
		end: u32
	},
	Files,
	WatchInfo {
		place: WatchPlace
	},
	WatchSet {
		place: WatchPlace,
		condition: Option<String>,
		hit: Option<HitCondition>
	},
	WatchUnset {
		id: u32
	},
	Disassemble {
		code: CodeRef
	},
	Journal {
		after: u64,
		kinds: Vec<JournalKind>,
		limit: Option<u32>,
		follow: bool
	},
	JournalAdd(JournalEvent),
	Uptime,
	HoldMax {
		seconds: Option<f64>
	},
	ProcInfo {
		proc: ProcRef
	},
	Render {
		frame_id: Option<u32>,
		expression: String,
		depth: u32
	},
	RenderVariables {
		vars: VariablesRef,
		depth: u32
	},
	EvalOn {
		object: String,
		expression: String,
		depth: Option<u32>
	},
	Defines(Vec<Macro>),
	Spawn {
		expression: String,
		object: Option<String>
	},
	Reloader {
		who: String
	},
	Skin(SkinRequest),
	Reconnect,
	Script(ScriptRequest),
	Profile(ProfileRequest),
	WatchTrigger {
		id: u32,
		condition: Option<String>,
		hit: Option<HitCondition>
	},
	BreakpointTrigger {
		id: u32,
		condition: Option<String>,
		hit: Option<HitCondition>,
		log: Option<Vec<LogPart>>
	},
	Breakpoints
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Macro {
	pub name: String,
	pub params: Vec<String>,
	pub text: String
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Response {
	Ack,
	StdDef(Option<String>),
	Eval(Result<EvalResponse, EvalError>),
	BreakpointSet {
		result: BreakpointSetResult
	},
	BreakpointUnset {
		success: bool,
		skipped: u32
	},
	Offset {
		offset: Option<u32>
	},
	Stacks {
		stacks: Vec<Stack>,
		sleeping: u32
	},
	StackFrames {
		frames: Vec<StackFrame>,
		total_count: u32
	},
	Scopes {
		arguments: Option<VariablesRef>,
		locals: Option<VariablesRef>,
		globals: Option<VariablesRef>
	},
	Variables {
		vars: Vec<Variable>
	},

	Disconnect,
	ShuttingDown,
	Notification {
		message: String
	},
	BreakpointHit(Stop),

	ByondVersion {
		version: u32,
		build: u32
	},
	HotReload(HotReloadResult),
	WorldInfo {
		args: Vec<String>,
		cwd: String,
		port: u16
	},
	Resumed,
	Status {
		paused: Option<Stop>
	},
	Locate {
		instructions: Vec<InstructionRef>
	},
	CodeChanged,
	Complete(Vec<Completion>),
	SetVariable(Result<Variable, String>),
	Log {
		message: String,
		file: Option<String>,
		line: Option<u32>,
		proc: ProcRef,
		caller: Option<StackFrame>
	},
	LinesWithCode {
		lines: Vec<u32>
	},
	Files {
		files: Vec<String>
	},
	WatchInfo(Result<String, String>),
	WatchSet(Result<Watch, String>),
	WatchEnded {
		id: u32,
		reason: String
	},
	Disassemble(Result<Disassembly, String>),
	Journal {
		entries: Vec<JournalEntry>,
		first: u64,
		last: u64
	},
	Journaled(JournalEntry),
	Uptime {
		loaded: u64,
		up: f64,
		world_time: f64,
		loads: u32,
		last_reload: Option<JournalEntry>,
		started: bool
	},
	ConditionFailed {
		message: String
	},
	ProcInfo(Result<ProcInfo, String>),
	Render(Result<Rendered, String>),
	RenderVariables(Result<Vec<(String, Rendered)>, String>),
	Spawned(Result<u64, String>),
	StepEnded,
	Reloader(Result<(), String>),
	Skin(Result<SkinReloaded, String>),
	Reconnect(Result<Reconnected, String>),
	Script(Result<ScriptReloaded, String>),
	Profile(Result<ProfileReply, String>),
	Unavailable {
		reason: String
	},
	Breakpoints(Vec<PlacedBreakpoint>)
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PlacedBreakpoint {
	pub id: u32,
	pub at: InstructionRef,
	pub file: Option<String>,
	pub line: Option<u32>,
	#[serde(default)]
	pub unlinked: bool
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SkinRequest {
	pub path: String
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SkinReloaded {
	pub clients: u32,
	pub hosted: bool
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ScriptRequest {
	pub text: Option<String>,
	pub built: Option<String>
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ScriptReloaded {
	pub clients: u32,
	pub hosted: bool
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum ProfileRequest {
	Start { procs: Vec<ProcRef> },
	Stop,
	Totals,
	Code { code: CodeRef }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum ProfileReply {
	Started,
	Totals(ProfileTotals),
	Code(Vec<ProfiledInstruction>)
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ProfileTotals {
	pub seconds: f64,
	pub running: bool,
	pub overhead_ns: f64,
	pub codes: Vec<ProfiledCode>
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ProfiledCode {
	pub proc: ProcRef,
	pub code: CodeRef,
	pub file: Option<String>,
	pub line: Option<u32>,
	pub total: ProfiledTotal
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfiledTotal {
	pub calls: u64,
	pub hits: u64,
	pub own_ns: u64,
	pub callee_ns: u64
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfiledInstruction {
	pub offset: u16,
	pub hits: u64,
	pub own_ns: u64,
	pub callee_ns: u64
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum Reconnected {
	Clients(u32),
	Hosted
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct JournalEntry {
	pub number: u64,
	pub time: u64,
	pub world_time: f64,
	pub event: JournalEvent
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum JournalEvent {
	Runtime {
		message: String,
		caught: bool,
		stack: Vec<JournalFrame>
	},
	Reload {
		applied: bool,
		summary: String,
		errors: Vec<String>,
		warnings: Vec<String>,
		failures: Vec<String>
	},
	Loaded {
		load: u32
	},
	Output {
		lines: Vec<String>
	},
	HoldEnded {
		seconds: f64
	},
	Spawned {
		spawn: u64,
		expression: String,
		result: Result<SpawnedValue, String>
	}
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SpawnedValue {
	pub text: String,
	pub rendered: Rendered
}

impl JournalEvent {
	pub const fn kind(&self) -> JournalKind {
		match self {
			Self::Runtime { .. } => JournalKind::Runtime,
			Self::Reload { .. } => JournalKind::Reload,
			Self::Loaded { .. } | Self::HoldEnded { .. } => JournalKind::World,
			Self::Output { .. } => JournalKind::Output,
			Self::Spawned { .. } => JournalKind::Spawned
		}
	}
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalKind {
	Runtime,
	Reload,
	World,
	Output,
	Spawned
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct JournalFrame {
	pub proc: String,
	pub file: Option<String>,
	pub line: Option<u32>
}

#[derive(Serialize, Deserialize, Debug, Hash, PartialEq, Eq, Clone)]
pub struct ProcRef {
	pub path: String,
	pub override_id: u32
}

#[derive(Serialize, Deserialize, Debug, Hash, PartialEq, Eq, Clone)]
pub struct InstructionRef {
	pub proc: ProcRef,
	pub offset: u32
}

#[derive(Serialize, Deserialize, Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub struct CodeRef {
	pub proc: u32,
	pub old: u16
}

pub const fn address(code: CodeRef, offset: u16) -> u64 {
	(code.proc as u64) << 32 | (code.old as u64) << 16 | offset as u64
}

pub const fn from_address(address: u64) -> (CodeRef, u16) {
	let code = CodeRef {
		proc: (address >> 32) as u32,
		old: (address >> 16) as u16
	};
	(code, address as u16)
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Disassembly {
	pub proc: ProcRef,
	pub file: Option<String>,
	pub instructions: Vec<BytecodeInstruction>,
	pub error: Option<String>
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct BytecodeInstruction {
	pub offset: u16,
	pub words: Vec<u32>,
	pub text: String,
	pub line: Option<u32>
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Rendered {
	Null,
	Number(f32),
	Text(String),
	List {
		len: u32,
		entries: Option<Vec<(Rendered, Option<Rendered>)>>
	},
	Object {
		reference: String,
		kind: String,
		name: Option<String>,
		vars: Option<Vec<(String, Rendered)>>
	},
	Other {
		kind: String,
		text: String
	}
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
	pub proc: ProcRef,
	pub code: CodeRef,
	pub file: Option<String>,
	pub line: Option<u32>,
	pub definitions: u32,
	pub reloaded: Option<Reloaded>
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reloaded {
	Lines,
	Code,
	Added
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum BreakpointReason {
	Breakpoint,
	Step,
	Pause,
	Runtime { message: String, caught: bool },
	Data { changes: Vec<DataChange> }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum WatchPlace {
	Variable { vars: VariablesRef, name: String },
	Expression { frame_id: Option<u32>, expression: String }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Watch {
	pub id: u32,
	pub name: String,
	pub value: String
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DataChange {
	pub id: u32,
	pub name: String,
	pub old: String,
	pub new: String
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Completion {
	pub name: String,
	pub proc: bool
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum LogPart {
	Text(String),
	Expr(String)
}

impl LogPart {
	pub fn parse(message: &str) -> Vec<Self> {
		let mut parts = vec![];
		let mut text = String::new();
		let mut rest = message;
		while let Some(at) = rest.find(['{', '}']) {
			text.push_str(&rest[..at]);
			let brace = &rest[at..at + 1];
			if rest[at + 1..].starts_with(brace) {
				text.push_str(brace);
				rest = &rest[at + 2..];
				continue;
			}
			if brace == "}" {
				text.push('}');
				rest = &rest[at + 1..];
				continue;
			}
			let Some(length) = Self::expression_length(&rest[at + 1..]) else {
				text.push_str(&rest[at..]);
				rest = "";
				break;
			};
			if !text.is_empty() {
				parts.push(Self::Text(std::mem::take(&mut text)));
			}
			parts.push(Self::Expr(rest[at + 1..at + 1 + length].trim().to_owned()));
			rest = &rest[at + 1 + length + 1..];
		}
		text.push_str(rest);
		if !text.is_empty() {
			parts.push(Self::Text(text));
		}
		parts
	}

	fn expression_length(text: &str) -> Option<usize> {
		let mut depth = 0;
		let mut quoted = false;
		let mut escaped = false;
		for (at, c) in text.char_indices() {
			if quoted {
				match c {
					_ if escaped => escaped = false,
					'\\' => escaped = true,
					'"' => quoted = false,
					_ => {}
				}
				continue;
			}
			match c {
				'"' => quoted = true,
				'{' => depth += 1,
				'}' if depth == 0 => return Some(at),
				'}' => depth -= 1,
				_ => {}
			}
		}
		None
	}
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitCondition {
	AtLeast(u32),
	Equal(u32),
	More(u32),
	Less(u32),
	AtMost(u32),
	Every(u32)
}

impl HitCondition {
	pub fn parse(text: &str) -> Result<Self, String> {
		let text = text.trim();
		let (make, number): (fn(u32) -> Self, &str) = if let Some(rest) = text.strip_prefix(">=") {
			(Self::AtLeast, rest)
		} else if let Some(rest) = text.strip_prefix("<=") {
			(Self::AtMost, rest)
		} else if let Some(rest) = text.strip_prefix("==").or_else(|| text.strip_prefix('=')) {
			(Self::Equal, rest)
		} else if let Some(rest) = text.strip_prefix('>') {
			(Self::More, rest)
		} else if let Some(rest) = text.strip_prefix('<') {
			(Self::Less, rest)
		} else if let Some(rest) = text.strip_prefix('%') {
			(Self::Every, rest)
		} else {
			(Self::AtLeast, text)
		};
		match number.trim().parse::<u32>() {
			Ok(0) if make(0) == Self::Every(0) => Err("hit interval must be greater than zero".to_owned()),
			Ok(number) => Ok(make(number)),
			Err(_) => Err(format!(
				"invalid hit count {:?}; expected N or an operator (==, >, >=, <, <=, %) followed by N",
				text
			))
		}
	}

	pub const fn takes(self, hits: u32) -> bool {
		match self {
			Self::AtLeast(n) => hits >= n,
			Self::Equal(n) => hits == n,
			Self::More(n) => hits > n,
			Self::Less(n) => hits < n,
			Self::AtMost(n) => hits <= n,
			Self::Every(n) => n != 0 && hits.is_multiple_of(n)
		}
	}
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeFilter {
	pub with: Vec<String>,
	pub without: Vec<String>
}

impl RuntimeFilter {
	pub fn parse(condition: &str) -> Self {
		let mut filter = Self::default();
		for text in condition.split(',').map(str::trim).filter(|text| !text.is_empty()) {
			match text.strip_prefix('!') {
				Some(text) if !text.trim().is_empty() => filter.without.push(text.trim().to_lowercase()),
				Some(_) => {}
				None => filter.with.push(text.to_lowercase())
			}
		}
		filter
	}

	pub fn takes(&self, message: &str) -> bool {
		let message = message.to_lowercase();
		(self.with.is_empty() || self.with.iter().any(|text| message.contains(text))) && !self.without.iter().any(|text| message.contains(text))
	}
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum ContinueKind {
	Continue,
	StepOver { stack_id: u32 },
	StepInto { stack_id: u32 },
	StepOut { stack_id: u32 },
	StepOverInstruction { stack_id: u32 },
	StepIntoInstruction { stack_id: u32 }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Stack {
	pub id: u32,
	pub name: String,
	pub file: Option<String>,
	pub line: Option<u32>,
	pub sleeping: bool
}

pub const DEFAULT_SLEEPING_STACKS: u32 = 100;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Stop {
	pub reason: BreakpointReason,
	pub thread: u32,
	pub at: InstructionRef,
	#[serde(default)]
	pub paused_by: Vec<u32>
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct StackFrame {
	pub id: u32,
	pub instruction: InstructionRef,
	pub line: Option<u32>,
	pub file: Option<String>,
	pub old_code: bool,
	pub code: CodeRef
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum BreakpointSetResult {
	Success {
		line: Option<u32>,
		at: InstructionRef,
		file: Option<String>
	},
	Failed,
	Refused {
		reason: String
	}
}

#[derive(Clone, Hash, Eq, PartialEq, Serialize, Deserialize, Debug)]
pub struct VariablesRef(pub i32);

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Variable {
	pub name: String,
	pub value: String,
	pub variables: Option<VariablesRef>,
	pub kind: Option<String>,
	pub expr: Option<String>,
	pub entries: Option<u32>
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct EvalError {
	pub message: String,
	pub still_running: bool
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct EvalResponse {
	pub value: String,
	pub variables: Option<VariablesRef>,
	pub kind: Option<String>,
	pub entries: Option<u32>
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct HotReloadRequest {
	pub plan: dmasm::reload::Plan,
	pub resources: Vec<HotReloadResource>,
	pub resource_dirs: Vec<String>,
	pub global_init: Option<dmasm::reload::GlobalInit>,
	pub shadowing_procs: Vec<ShadowingProc>,
	#[serde(default)]
	pub local_reads: Vec<LocalReads>
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LocalReads {
	pub proc: dmasm::reload::ProcKey,
	pub name: String,
	pub reads: Vec<Option<u32>>,
	#[serde(default)]
	pub parsed: Option<ParsedReads>,
	#[serde(default)]
	pub kept: Vec<(u32, u32)>
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedReads {
	Only(u32),
	Unknown
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ShadowingProc {
	pub global: String,
	pub proc: dmasm::reload::ProcKey,
	#[serde(default)]
	pub reads: Vec<Option<u32>>
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct HotReloadResource {
	pub path: String,
	pub ids: Vec<u32>
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ReloadedResource {
	pub path: String,
	pub id: u32,
	pub appearances: u32
}

impl HotReloadRequest {
	pub fn len(&self) -> usize {
		self.plan.len() + self.resources.len() + self.global_init.is_some() as usize
	}

	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct HotReloadResult {
	pub applied: bool,
	pub errors: Vec<String>,
	pub warnings: Vec<String>,
	#[serde(default)]
	pub failures: Vec<String>,
	pub skipped: Vec<String>,
	#[serde(default)]
	pub resources: Vec<ReloadedResource>
}
