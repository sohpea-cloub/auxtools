// TODO: These will be shared properly

use serde::{Deserialize, Serialize};

/// The version of the messages here. Bump it by hand with any change a client
/// and a world built before it wouldn't agree on: it's in the name of the
/// world's socket, so a client only finds worlds that speak its version.
pub const PROTOCOL_VERSION: u32 = 3;

/// The local socket the world in process `pid` listens on, when it has the
/// debug library: `auxtools-debug-v1-1234`
#[allow(dead_code)]
pub fn world_socket_name(pid: u32) -> String {
	format!("auxtools-debug-v{}-{}", PROTOCOL_VERSION, pid)
}

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
	HotReload(HotReloadRequest),
	/// The program's command line and working directory, which say which world
	/// it runs
	WorldInfo,
	/// Whether the world is paused, and why
	Status,
	/// Keeps the world paused (now, or at its next pause) after every client
	/// has gone, until one tells it to carry on: for clients that come and go,
	/// like the launcher's command line. Forgotten if the client that asked
	/// leaves before a pause.
	HoldPause,
	/// Where each proc with code on a line of a file has it, found in the
	/// world's code (its DbgFile and DbgLine). `file` can be the path the world
	/// was built with, a path that ends with it, or its end. A line with no
	/// code of its own gives the next line that has.
	Locate {
		file: String,
		line: u32
	},
	/// From a client that isn't debugging: it's connected for something else,
	/// like the launcher of a world run without debugging, there to hot
	/// reload it. A pause doesn't wait for it, as it wouldn't let the world
	/// carry on: with only such clients left, the world does.
	Watcher
}

// Message from server -> client
#[derive(Serialize, Deserialize, Debug)]
pub enum Response {
	Ack,
	StdDef(Option<String>),
	Eval(EvalResponse),
	BreakpointSet {
		result: BreakpointSetResult
	},
	BreakpointUnset {
		success: bool
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
	HotReload(HotReloadResult),
	WorldInfo {
		args: Vec<String>,
		cwd: String
	},
	/// Another client let the world carry on from a pause
	Resumed,
	Status {
		paused: Option<BreakpointReason>
	},
	Locate {
		instructions: Vec<InstructionRef>
	}
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
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
	pub line: Option<u32>,
	/// The file its proc is in, as its DbgFile names it (a world built with
	/// DEBUG has them)
	pub file: Option<String>
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
	/// Vars declared another way, after the new vars and before the var
	/// defaults, which can be for them
	#[serde(default)]
	pub redeclared: Vec<dmasm::reload::Redeclared>,
	/// Applied after the new vars, so they can set the defaults of those too
	pub var_defaults: Vec<HotReloadVarDefault>,
	/// New defaults of built-in vars, which types keep in their records
	pub builtin_defaults: Vec<dmasm::reload::VarDefault>,
	/// Types whose new instances run other code to set their vars
	pub init_procs: Vec<HotReloadInitProc>,
	pub line_moves: Vec<HotReloadLines>
}

impl HotReloadRequest {
	/// How many things it changes
	pub const fn len(&self) -> usize {
		self.new_types.len() + self.procs.len() + self.settings.len() + self.new_procs.len() + self.removed_procs.len() + self.new_vars.len() + self.redeclared.len() + self.var_defaults.len() + self.builtin_defaults.len() + self.init_procs.len() + self.line_moves.len()
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
