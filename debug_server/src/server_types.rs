// TODO: These will be shared properly

use serde::{Deserialize, Serialize};

/// The version of the messages here. Bump it by hand with any change a client
/// and a world built before it wouldn't agree on: it's in the name of the
/// world's socket, so a client only finds worlds that speak its version.
pub const PROTOCOL_VERSION: u32 = 4;

/// The local socket the world in process `pid` listens on, when it has the
/// debug library: `auxtools-debug-v1-1234`
#[allow(dead_code)]
pub fn world_socket_name(pid: u32) -> String {
	world_socket_name_for(PROTOCOL_VERSION, pid)
}

/// The same for a debug library that speaks another version of the
/// protocol, which is how one is told from this
#[allow(dead_code)]
pub fn world_socket_name_for(protocol: u32, pid: u32) -> String {
	format!("auxtools-debug-v{}-{}", protocol, pid)
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
	/// The world's stacks: the running proc's, and those of the first
	/// `sleeping` procs that are asleep (`None` for
	/// [`DEFAULT_SLEEPING_STACKS`])
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
	/// Gives a variable another value: `name` among what `vars` opens to,
	/// and an expression for the value, evaluated in the frame the variable
	/// is in (the paused proc's, for what isn't a frame's own)
	SetVariable {
		vars: VariablesRef,
		name: String,
		value: String
	},
	/// The names that could go on from what's typed: with a `base`, the vars
	/// and procs of what that expression is in the frame; without one, what
	/// the frame can name. Those that start with `prefix`.
	Complete {
		frame_id: Option<u32>,
		base: Option<String>,
		prefix: String
	},
	/// From a client that isn't debugging: it's connected for something else,
	/// like the launcher of a world run without debugging, there to hot
	/// reload it. A pause doesn't wait for it, as it wouldn't let the world
	/// carry on: with only such clients left, the world does.
	Watcher,
	/// The lines from `start` to `end` (both included) of a file that have
	/// code, in the code the world runs now. `file` is matched as in `Locate`.
	LinesWithCode {
		file: String,
		start: u32,
		end: u32
	},
	/// The files the world's code is from, as it was built with them: those
	/// with a proc in them, as a file is known by its procs' DbgFile
	Files,
	/// What a watch on a place would watch, without watching it
	WatchInfo {
		place: WatchPlace
	},
	/// Has the world pause when what's at a place changes. A condition is
	/// worked out in the frame the world would pause in, where `value` is
	/// what's at the place now and `old` what was.
	WatchSet {
		place: WatchPlace,
		condition: Option<String>,
		hit: Option<HitCondition>
	},
	WatchUnset {
		id: u32
	},
	/// The instructions of some code: a proc's, or the old code a frame runs
	Disassemble {
		code: CodeRef
	},
	/// A breakpoint at exactly this instruction, which has to be the start of
	/// one: offset 0 is the proc's first instruction, not the proc itself as
	/// for `BreakpointSet`. It's the client's apart from any breakpoint it
	/// has on the instruction's line or proc. Answered as `BreakpointSet` is.
	InstructionBreakpointSet {
		instruction: InstructionRef,
		condition: Option<String>,
		hit: Option<HitCondition>,
		log: Option<Vec<LogPart>>
	},
	/// Takes away what `InstructionBreakpointSet` set. Answered as
	/// `BreakpointUnset` is.
	InstructionBreakpointUnset {
		instruction: InstructionRef
	},
	/// The world's record of what happened in it (`JournalEntry`): the
	/// entries numbered after `after`, of these kinds (every kind, if none are
	/// given), oldest first; with a `limit`, only that many of the newest of
	/// them. With `follow`, each new entry of those kinds is sent to this
	/// client as it's made (`Response::Journaled`), until it asks again
	/// without. Answered with `Response::Journal`.
	Journal {
		after: u64,
		kinds: Vec<JournalKind>,
		limit: Option<u32>,
		follow: bool
	},
	/// Something for the record that the world can't see for itself: what
	/// came of a hot reload this client tried, the world's output it reads.
	/// It has no answer, and isn't sent back to this client if it follows.
	JournalAdd(JournalEvent),
	/// How long the world has been up, and its last hot reload
	Uptime,
	/// How long a pause this client holds (`HoldPause`) lasts once no client
	/// that debugs is connected, before the world carries on by itself and
	/// its record says so (if one is connected when the time is up, once it
	/// goes). `None` for no limit, as there is unless asked.
	/// Answered with `Ack`.
	HoldMax {
		seconds: Option<f64>
	},
	/// Where a proc is, and what hot reloads did to it. `proc.path` can be
	/// written with `/proc/` or `/verb/` in it, or without.
	ProcInfo {
		proc: ProcRef
	}
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
		success: bool,
		/// How many of its hits were skipped because its condition failed
		/// with an error
		skipped: u32
	},
	Offset {
		offset: Option<u32>
	},
	Stacks {
		stacks: Vec<Stack>,
		/// How many procs are asleep, listed or not
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

	// These responses can occur at any moment, even between a request and its response
	// I guess they aren't really responses...
	Disconnect,
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
		cwd: String
	},
	/// Another client let the world carry on from a pause
	Resumed,
	Status {
		paused: Option<Stop>
	},
	Locate {
		instructions: Vec<InstructionRef>
	},
	/// Another client hot reloaded the world: its code has changed, and
	/// lines may be elsewhere
	CodeChanged,
	Complete(Vec<Completion>),
	/// The variable as it is now, or why it couldn't be set
	SetVariable(Result<Variable, String>),
	/// A breakpoint this client set with a message was hit: the message,
	/// where the breakpoint is, and the frame that called the proc it's in
	/// (`None` for a proc nothing called: one the world started, or one
	/// that carries on after a `sleep` or `spawn`)
	Log {
		message: String,
		file: Option<String>,
		line: Option<u32>,
		proc: ProcRef,
		caller: Option<StackFrame>
	},
	/// Sorted, each once
	LinesWithCode {
		lines: Vec<u32>
	},
	/// Sorted, spelled as the world has them
	Files {
		files: Vec<String>
	},
	/// What the place is called, or why it can't be watched
	WatchInfo(Result<String, String>),
	WatchSet(Result<Watch, String>),
	/// A watch is over, without having been taken away: its place is gone
	/// (a proc that returned, an object that was deleted). Sent to the
	/// client that set it, at any moment.
	WatchEnded {
		id: u32,
		reason: String
	},
	Disassemble(Result<Disassembly, String>),
	/// The entries asked for, and the numbers of the oldest entry the
	/// record still has and of the newest (0 for none)
	Journal {
		entries: Vec<JournalEntry>,
		first: u64,
		last: u64
	},
	/// A new entry, for a client that follows the record. At any moment.
	Journaled(JournalEntry),
	Uptime {
		/// When this world loaded, in milliseconds since 1970 (UTC)
		loaded: u64,
		/// How long ago that was, in seconds
		up: f64,
		/// `world.time`, in deciseconds
		world_time: f64,
		/// How many worlds the process has loaded: 1, then one more for each
		/// reboot
		loads: u32,
		last_reload: Option<JournalEntry>
	},
	/// The condition of a breakpoint or a watch this client set failed with
	/// an error where it was hit, so that hit was skipped, as are the others
	/// where it fails. Said once for each breakpoint or watch, the first time,
	/// at any moment.
	ConditionFailed {
		message: String
	},
	/// What the world knows of a proc, or why it knows nothing
	ProcInfo(Result<ProcInfo, String>)
}

/// An entry in the world's record of what happened in it. The record keeps
/// the last few thousand, and lasts as long as the process: through reboots.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct JournalEntry {
	/// From 1, one more for each entry, never given again
	pub number: u64,
	/// When it was made, in milliseconds since 1970 (UTC)
	pub time: u64,
	/// `world.time` then, in deciseconds
	pub world_time: f64,
	pub event: JournalEvent
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum JournalEvent {
	/// A runtime error, or a `throw`, whether or not anything paused for it:
	/// the procs it happened in, innermost first
	Runtime {
		message: String,
		/// It's inside a `try` that catches it
		caught: bool,
		stack: Vec<JournalFrame>
	},
	/// What came of a hot reload, applied or not, as the client that tried
	/// it says: errors are why it wasn't (compile errors among them, which
	/// stop it before the world is sent anything)
	Reload {
		applied: bool,
		summary: String,
		errors: Vec<String>,
		warnings: Vec<String>
	},
	/// A world has loaded and is running: the process's `load`th, so 2 and
	/// on are reboots
	Loaded { load: u32 },
	/// Lines the world printed, as a launcher that reads them passes them on
	Output { lines: Vec<String> },
	/// A pause held for clients that come and go (`Request::HoldPause`) was
	/// let go by the debug server: no client that debugs was there for this
	/// many seconds
	HoldEnded { seconds: f64 }
}

impl JournalEvent {
	pub const fn kind(&self) -> JournalKind {
		match self {
			Self::Runtime { .. } => JournalKind::Runtime,
			Self::Reload { .. } => JournalKind::Reload,
			Self::Loaded { .. } | Self::HoldEnded { .. } => JournalKind::World,
			Self::Output { .. } => JournalKind::Output
		}
	}
}

/// What a `Request::Journal` can ask for
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalKind {
	Runtime,
	Reload,
	/// Worlds loading, and what the debug server did by itself
	World,
	Output
}

/// A proc a runtime error happened in
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct JournalFrame {
	/// Its path, with `#N` after it for the Nth override
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

/// Some code: a proc's as it is now, or code a frame still runs that a hot
/// reload replaced. An instruction of it is known by one number, its
/// [`address`]: what clients that show bytecode want.
#[derive(Serialize, Deserialize, Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub struct CodeRef {
	/// The engine's id of the proc, which is the same in each world started
	/// from the same .dmb
	pub proc: u32,
	/// 0 for the proc's code now; else the number the world gave the old
	/// code, from 1, which lasts until the world reboots
	pub old: u16
}

/// An instruction's address: the proc's id, which of its codes, and the
/// offset in words, as `0xPPPPPPPPCCCCOOOO`. Each code's instructions are in
/// order, one word to one.
#[allow(dead_code)]
pub const fn address(code: CodeRef, offset: u16) -> u64 {
	(code.proc as u64) << 32 | (code.old as u64) << 16 | offset as u64
}

/// The code and offset an [`address`] is of
#[allow(dead_code)]
pub const fn from_address(address: u64) -> (CodeRef, u16) {
	let code = CodeRef {
		proc: (address >> 32) as u32,
		old: (address >> 16) as u16
	};
	(code, address as u16)
}

/// The instructions of some code (`Request::Disassemble`)
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Disassembly {
	pub proc: ProcRef,
	/// The file its DbgFile names
	pub file: Option<String>,
	/// In order. Words past the last that could be read, if there are any,
	/// are each one with no text, and `error` says why.
	pub instructions: Vec<BytecodeInstruction>,
	pub error: Option<String>
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct BytecodeInstruction {
	/// In words, from the code's start
	pub offset: u16,
	/// Its opcode and operands, as the code has them (without the debugger's
	/// breakpoints)
	pub words: Vec<u32>,
	/// As dmasm writes it
	pub text: String,
	/// The line it's on, by the DbgLine before it (a line of the file as it
	/// was, for old code)
	pub line: Option<u32>
}

/// A proc, as `Request::ProcInfo` finds it
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
	/// Its path as the world spells it, and which definition of it it is
	pub proc: ProcRef,
	/// Its code now, for `Request::Disassemble`
	pub code: CodeRef,
	/// The file its DbgFile names
	pub file: Option<String>,
	/// The first line it has code on
	pub line: Option<u32>,
	/// How many definitions its path has: `#0` (the first) to one less than
	/// this
	pub definitions: u32,
	/// What hot reloads have done to it since the world started, if anything
	pub reloaded: Option<Reloaded>
}

/// What hot reloads did to a proc: the most they did, of these
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reloaded {
	/// Its code is as it was, on other lines
	Lines,
	/// It has new code
	Code,
	/// A hot reload added it: the world was built without it
	Added
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum BreakpointReason {
	Breakpoint,
	Step,
	Pause,
	/// A runtime error, or a `throw`: its message, and whether it's inside
	/// a `try` that will catch it
	Runtime { message: String, caught: bool },
	/// What's at watched places changed (`Request::WatchSet`), in the
	/// instruction before the one the world stopped at, or under it: in
	/// something the engine did
	Data { changes: Vec<DataChange> }
}

/// A place to watch for a change
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum WatchPlace {
	/// A variable as `Response::Variables` listed it: `name` in `vars`
	Variable { vars: VariablesRef, name: String },
	/// What an expression names, in a frame or in none: a proc's own var by
	/// its name alone, else anything `&(expression)` makes a pointer to
	Expression { frame_id: Option<u32>, expression: String }
}

/// A watch that was set
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Watch {
	/// No two watches in a world have the same, whoever set them
	pub id: u32,
	/// What the place is called
	pub name: String,
	/// What's there now
	pub value: String
}

/// What a watch saw
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DataChange {
	/// The watch's (`Watch::id`)
	pub id: u32,
	pub name: String,
	pub old: String,
	pub new: String
}

/// A name that could be typed next
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Completion {
	pub name: String,
	/// It's a proc to call, not a var to read
	pub proc: bool
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
	StepOut { stack_id: u32 },
	/// One instruction on in the stack's frame: a call it makes runs to its
	/// end. If the frame returns, the step ends in its caller.
	StepOverInstruction { stack_id: u32 },
	/// The very next instruction the stack runs, in a proc it calls too
	StepIntoInstruction { stack_id: u32 }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Stack {
	/// The same from one stop to the next, for as long as the proc the stack
	/// starts with goes on (another stack can have it after that). 0 is
	/// never one: in a request, it means the running proc's stack.
	pub id: u32,
	/// The proc it's in: its innermost frame's
	pub name: String,
	/// Where that frame is
	pub file: Option<String>,
	pub line: Option<u32>,
	/// Asleep, not the proc that's running
	pub sleeping: bool
}

/// How many sleeping procs' stacks `Request::Stacks` lists unless told
pub const DEFAULT_SLEEPING_STACKS: u32 = 100;

/// The world, paused
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Stop {
	pub reason: BreakpointReason,
	/// The stack of the proc that was running (a [`Stack`]'s id)
	pub thread: u32,
	/// The instruction it's about to run: a breakpoint's
	/// (`BreakpointSetResult::Success::at`), if it stopped for one
	pub at: InstructionRef
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct StackFrame {
	/// The running stack's frames are 0, 1, 2, ... from the innermost. A
	/// sleeping stack's are given ids of their own, which name that frame
	/// for as long as the world is paused, or nothing once it's gone.
	pub id: u32,
	pub instruction: InstructionRef,
	pub line: Option<u32>,
	/// The file its proc is in, as its DbgFile names it (a world built with
	/// DEBUG has them)
	pub file: Option<String>,
	/// It started before a hot reload changed its proc's code, and runs the
	/// code it started with: its line is a line of the file as it was then
	pub old_code: bool,
	/// The code it runs, which `Request::Disassemble` gives the instructions
	/// of: with `instruction.offset`, its instruction's [`address`]
	pub code: CodeRef
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum BreakpointSetResult {
	Success {
		line: Option<u32>,
		/// Where it is, as a stop there has it (`Stop::at`)
		at: InstructionRef,
		/// The file its proc is in, as its DbgFile names it
		file: Option<String>
	},
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
