// TODO: These will be shared properly

use serde::{Deserialize, Serialize};

#[allow(dead_code)]
pub const DEFAULT_PORT: u16 = 2448;

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
	/// Swaps new code into procs and renumbers the lines of procs that moved.
	/// Either all of it is applied or none of it is.
	HotReload {
		procs: Vec<HotReloadProc>,
		line_moves: Vec<HotReloadLines>
	}
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

/// New code for a proc, as dmasm's proc compiler produced it
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HotReloadProc {
	pub proc: ProcRef,
	/// The body, with DbgFile and DbgLine. They're dropped for a world built
	/// without them.
	pub nodes: Vec<dmasm::Node>,
	/// Local variable names by slot
	pub locals: Vec<String>,
	/// Parameter names, which have to be the proc's own
	pub params: Vec<String>
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
