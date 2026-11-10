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
		condition: Option<String>,
		/// Which of its hits to stop at, counted from when it's set (those
		/// where its condition holds, if it has one). `None` for all.
		hit: Option<HitCondition>,
		/// A message to send this client at those hits, in place of
		/// stopping the world (`Response::Log`)
		log: Option<Vec<LogPart>>
	},
	BreakpointUnset {
		instruction: InstructionRef
	},
	/// Which runtime errors this client wants the world to pause on: those
	/// that nothing catches (an uncaught `throw` is one), and those inside a
	/// `try`. `None` for none of a kind. It has no answer.
	CatchRuntimes {
		uncaught: Option<RuntimeFilter>,
		caught: Option<RuntimeFilter>
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
		vars: VariablesRef,
		/// For a list: how many entries to give, and from which (from 0).
		/// `None` for all of them.
		start: Option<u32>,
		count: Option<u32>
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
	/// What an expression gave, or why it gave nothing
	Eval(Result<EvalResponse, String>),
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
	},
	/// Another client hot reloaded the world: its code has changed, and
	/// lines may be elsewhere
	CodeChanged,
	/// A breakpoint this client set with a message was hit: the message,
	/// and where the breakpoint is
	Log {
		message: String,
		file: Option<String>,
		line: Option<u32>
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
	/// A runtime error, or a `throw`: its message, and whether it's inside
	/// a `try` that will catch it
	Runtime { message: String, caught: bool }
}

/// A piece of a breakpoint's log message
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum LogPart {
	Text(String),
	/// An expression, evaluated in the frame of the hit: its value goes in
	/// its place
	Expr(String)
}

#[allow(dead_code)]
impl LogPart {
	/// As a client writes a message: text, with expressions in `{braces}`.
	/// Braces inside an expression are its own when they pair up or are in
	/// a text; one that's never closed is text.
	pub fn parse(message: &str) -> Vec<Self> {
		let mut parts = vec![];
		let mut text = String::new();
		let mut rest = message;
		while let Some(open) = rest.find('{') {
			let Some(length) = Self::expression_length(&rest[open + 1..]) else {
				break;
			};
			text.push_str(&rest[..open]);
			if !text.is_empty() {
				parts.push(Self::Text(std::mem::take(&mut text)));
			}
			parts.push(Self::Expr(rest[open + 1..open + 1 + length].trim().to_owned()));
			rest = &rest[open + 1 + length + 1..];
		}
		text.push_str(rest);
		if !text.is_empty() {
			parts.push(Self::Text(text));
		}
		parts
	}

	/// How far it is to the `}` that closes an expression starting here
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

/// Which hits of a breakpoint stop the world, by their number, from 1
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitCondition {
	AtLeast(u32),
	Equal(u32),
	More(u32),
	Less(u32),
	AtMost(u32),
	/// Every so many: the 3rd, the 6th, ...
	Every(u32)
}

#[allow(dead_code)]
impl HitCondition {
	/// As a client writes one: `5` for the fifth hit and every one after,
	/// or a number after `==`, `>`, `>=`, `<`, `<=`, or `%` (every so many)
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
			Ok(0) if make(0) == Self::Every(0) => Err("every 0 hits is never".to_owned()),
			Ok(number) => Ok(make(number)),
			Err(_) => Err(format!(
				"{:?} isn't a hit count: it's a number (stop from that hit on), or one after ==, >, >=, <, <= or % (every so many)",
				text
			))
		}
	}

	/// Whether the hit of this number is one to stop at
	pub const fn takes(self, hits: u32) -> bool {
		match self {
			Self::AtLeast(n) => hits >= n,
			Self::Equal(n) => hits == n,
			Self::More(n) => hits > n,
			Self::Less(n) => hits < n,
			Self::AtMost(n) => hits <= n,
			Self::Every(n) => n != 0 && hits % n == 0
		}
	}
}

/// Which runtime errors to pause on, by their messages
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeFilter {
	/// Text the message has to have some of, if there's any here
	pub with: Vec<String>,
	/// Text the message mustn't have any of
	pub without: Vec<String>
}

#[allow(dead_code)]
impl RuntimeFilter {
	/// As a client writes one: texts with commas between them, one starting
	/// with `!` for text the message mustn't have. `divi, bounds` is errors
	/// with either word, `!libdreamluau` every error without that one. Case
	/// doesn't matter. Empty is every error.
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

	/// Whether an error with this message is one to pause on
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
	pub file: Option<String>,
	/// It started before a hot reload changed its proc's code, and runs the
	/// code it started with: its line is a line of the file as it was then
	pub old_code: bool
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum BreakpointSetResult {
	Success { line: Option<u32> },
	/// There's no such proc, or no instruction there
	Failed,
	/// It can't be set as it was asked for: its condition doesn't compile
	Refused { reason: String }
}

#[derive(Clone, Hash, Eq, PartialEq, Serialize, Deserialize, Debug)]
pub struct VariablesRef(pub i32);

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Variable {
	pub name: String,
	pub value: String,
	pub variables: Option<VariablesRef>,
	/// What kind of value it is: an object's type, or `num`, `text`, `/list`
	/// and the like
	pub kind: Option<String>,
	/// An expression that gives it, in the frame it was listed for
	pub expr: Option<String>,
	/// For a list: how many entries it has
	pub entries: Option<u32>
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct EvalResponse {
	pub value: String,
	pub variables: Option<VariablesRef>,
	/// As a `Variable`'s
	pub kind: Option<String>,
	pub entries: Option<u32>
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
