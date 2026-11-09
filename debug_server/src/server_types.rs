// TODO: These will be shared properly

use serde::{Deserialize, Serialize};

#[allow(dead_code)]
pub const DEFAULT_PORT: u16 = 2448;

/// Where hot reload watchers connect by default (see `Request::HotReload`)
#[allow(dead_code)]
pub const DEFAULT_RELOAD_PORT: u16 = 2449;

// Message from client -> server
#[derive(Serialize, Deserialize, Debug)]
pub enum Request {
	Disconnect,
	Configured,
	StdDef,
	Eval {
		frame_id: Option<u32>,
		command: String,
		context: Option<String>
	},
	CurrentInstruction {
		frame_id: u32
	},
	BreakpointSet {
		instruction: InstructionRef,
		condition: Option<String>
	},
	BreakpointUnset {
		instruction: InstructionRef
	},
	CatchRuntimes {
		should_catch: bool
	},
	LineNumber {
		proc: ProcRef,
		offset: u32
	},
	Offset {
		proc: ProcRef,
		line: u32
	},
	Stacks,
	StackFrames {
		stack_id: u32,
		start_frame: Option<u32>,
		count: Option<u32>
	},
	Scopes {
		frame_id: u32
	},
	Variables {
		vars: VariablesRef
	},
	Continue {
		kind: ContinueKind
	},
	Pause,

	// New requests go last, so older debug-clients' requests keep their numbers
	/// Which BYOND version the world runs on
	ByondVersion,
	/// Swaps new code into procs, adds vars and procs, sets new var defaults
	/// and renumbers the lines of procs that moved. Either all of it is
	/// applied or none of it is.
	HotReload(HotReloadRequest)
}

// Message from server -> client
#[derive(Serialize, Deserialize, Debug)]
pub enum Response {
	Ack,
	StdDef(Option<String>),
	Eval(EvalResponse),
	CurrentInstruction(Option<InstructionRef>),
	BreakpointSet {
		result: BreakpointSetResult
	},
	BreakpointUnset {
		success: bool
	},
	LineNumber {
		line: Option<u32>
	},
	Offset {
		offset: Option<u32>
	},
	Stacks {
		stacks: Vec<Stack>
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

	// These responses can occur at any moment, even between a request and its response
	// I guess they aren't really responses...
	Disconnect,
	Notification {
		message: String
	},
	BreakpointHit {
		reason: BreakpointReason
	},

	ByondVersion {
		version: u32,
		build: u32
	},
	HotReload(HotReloadResult)
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

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum BreakpointReason {
	Breakpoint,
	Step,
	Pause,
	Runtime(String)
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum ContinueKind {
	Continue,
	StepOver { stack_id: u32 },
	StepInto { stack_id: u32 },
	StepOut { stack_id: u32 }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Stack {
	pub id: u32,
	pub name: String
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct StackFrame {
	pub id: u32,
	pub instruction: InstructionRef,
	pub line: Option<u32>
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum BreakpointSetResult {
	Success { line: Option<u32> },
	Failed
}

#[derive(Clone, Hash, Eq, PartialEq, Serialize, Deserialize, Debug)]
pub struct VariablesRef(pub i32);

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Variable {
	pub name: String,
	pub value: String,
	pub variables: Option<VariablesRef>
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct EvalResponse {
	pub value: String,
	pub variables: Option<VariablesRef>
}

/// What a hot reload changes (see `Request::HotReload`)
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct HotReloadRequest {
	/// Added before anything else, each after its parent. If the rest can't
	/// be applied, they stay: nothing uses them.
	pub new_types: Vec<dmasm::reload::NewType>,
	pub procs: Vec<HotReloadProc>,
	/// Procs whose code is as it was, but not their settings
	pub settings: Vec<dmasm::reload::SettingsChange>,
	pub new_procs: Vec<HotReloadNewProc>,
	/// Definitions on types that are gone, unlinked before new procs are added
	pub removed_procs: Vec<dmasm::reload::RemovedProc>,
	/// Added before anything else, so the code in the reload can use them
	pub new_vars: Vec<dmasm::reload::NewVar>,
	/// Applied after the new vars, so they can set the defaults of those too
	pub var_defaults: Vec<HotReloadVarDefault>,
	/// Types whose new instances run other code to set their vars
	pub init_procs: Vec<HotReloadInitProc>,
	pub line_moves: Vec<HotReloadLines>
}

impl HotReloadRequest {
	/// How many things it changes
	pub const fn len(&self) -> usize {
		self.new_types.len() + self.procs.len() + self.settings.len() + self.new_procs.len() + self.removed_procs.len() + self.new_vars.len() + self.var_defaults.len() + self.init_procs.len() + self.line_moves.len()
	}

	pub const fn is_empty(&self) -> bool {
		self.len() == 0
	}
}

/// New code for a proc, as dmasm's proc compiler produced it
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HotReloadProc {
	pub proc: ProcRef,
	/// The body, with DbgFile and DbgLine. They're dropped for a world built
	/// without them.
	pub nodes: Vec<dmasm::Node>,
	/// Local variable names by slot
	pub locals: Vec<String>,
	pub params: Vec<dmasm::reload::Param>,
	/// `None` where they're as they were
	pub settings: Option<dmasm::reload::ProcSettings>,
	/// Its static locals, for those the running proc doesn't have yet
	pub statics: Vec<dmasm::reload::StaticVar>
}

/// A proc the world doesn't have yet. `code.proc` is how it will be known.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HotReloadNewProc {
	/// The type it's on, `""` for a global proc
	pub type_path: String,
	pub name: String,
	/// Declared on the type (`proc/name`), not overriding a parent's proc
	pub declared: bool,
	/// On the type's list of verbs, not of procs
	pub verb: bool,
	pub code: HotReloadProc
}

/// A new default for a var, on the type and each subtype that inherits it.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HotReloadVarDefault {
	pub var: String,
	pub value: dmasm::reload::ConstValue,
	pub types: Vec<String>
}

/// A type's new init proc: the code each new instance runs to set the vars
/// whose defaults aren't constants, its own and those it inherits
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HotReloadInitProc {
	pub type_path: String,
	/// `None` when there's nothing left for it to set
	pub code: Option<HotReloadProc>
}

/// A proc whose code is unchanged but now sits on other lines
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HotReloadLines {
	pub proc: ProcRef,
	/// (old line, new line) pairs
	pub lines: Vec<(u32, u32)>
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct HotReloadResult {
	/// Whether anything changed. When there are errors, nothing did.
	pub applied: bool,
	pub errors: Vec<String>,
	pub warnings: Vec<String>
}
