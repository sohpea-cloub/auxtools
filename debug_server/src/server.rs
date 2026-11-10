use std::{
	cell::{OnceCell, RefCell},
	collections::{BTreeMap, HashMap},
	error::Error,
	io::{Read, Write},
	sync::{mpsc, Mutex},
	thread
};

use auxtools::{
	raw_types::values::{ValueData, ValueTag},
	*
};
use clap::{Arg, Command};
use instruction_hooking::disassemble_env;

use super::{
	instruction_hooking::{hook_instruction, original_bytecode, unhook_all, unhook_instruction},
	server_types::*
};
use crate::{
	connection::{self, Connection},
	hot_reload, mem_profiler, waker
};

/// Something a client can open, for the variables inside it. Those that are
/// reached through an expression carry it, so what's inside can say how it's
/// reached in turn (`a.b[2]`).
#[derive(Clone, Hash, PartialEq, Eq)]
enum Variables {
	Arguments { frame: u32 },
	Locals { frame: u32 },
	/// The world's own global vars, and a row for BYOND's
	Globals,
	/// The constants BYOND itself declares (`NORTH`, `MOB_LAYER`, ...)
	Builtins,
	ObjectVars(Value, Option<String>),
	ListContents(Value, Option<String>),
	/// An entry of the list `of` that has a value for its key. `list` is
	/// the expression the list is reached by.
	ListPair {
		key: Value,
		value: Value,
		of: Value,
		list: Option<String>
	}
}

/// What a pause has been asked about. The stacks are read when they're first
/// wanted: a breakpoint with a condition needs the running proc's to work its
/// condition out, and most hits end there. The sleeping procs' stacks, which
/// a world can have thousands of, are only read for a client that looks at
/// them.
struct State {
	active: OnceCell<Vec<debug::StackFrame>>,
	suspended: OnceCell<Vec<Vec<debug::StackFrame>>>,
	/// How many of the sleeping procs' stacks are read: as many as a client
	/// has asked to see
	sleeping: usize,
	variables: RefCell<Vec<Variables>>,
	variables_to_refs: RefCell<HashMap<Variables, VariablesRef>>
}

impl State {
	fn new() -> Self {
		Self {
			active: OnceCell::new(),
			suspended: OnceCell::new(),
			sleeping: DEFAULT_SLEEPING_STACKS as usize,
			variables: RefCell::new(vec![]),
			variables_to_refs: RefCell::new(HashMap::new())
		}
	}

	/// Code has run since the stacks were read: they're read again when
	/// they're next wanted
	fn invalidate_stacks(&mut self) {
		self.active = OnceCell::new();
		self.suspended = OnceCell::new();
	}

	/// The running proc's stack
	fn active(&self) -> &Vec<debug::StackFrame> {
		self.active.get_or_init(debug::CallStacks::active)
	}

	/// The sleeping procs' stacks, the first `sleeping` of them
	fn suspended(&self) -> &Vec<Vec<debug::StackFrame>> {
		self.suspended.get_or_init(|| debug::CallStacks::suspended_first(self.sleeping))
	}

	/// Has `suspended` read at least `sleeping` stacks. The ones it had
	/// stay where they were, so frames keep their ids.
	fn read_sleeping(&mut self, sleeping: usize) {
		if sleeping > self.sleeping {
			self.sleeping = sleeping;
			self.suspended = OnceCell::new();
		}
	}

	fn get_ref(&self, vars: Variables) -> VariablesRef {
		let mut variables_to_refs = self.variables_to_refs.borrow_mut();
		let mut variables = self.variables.borrow_mut();
		(*variables_to_refs.entry(vars.clone()).or_insert_with(|| {
			let reference = VariablesRef(variables.len() as i32 + 1);
			variables.push(vars);
			reference
		}))
		.clone()
	}

	fn get_variables(&self, reference: VariablesRef) -> Option<Variables> {
		let variables = self.variables.borrow();
		// References start at 1: 0 is what a client has for "nothing to open"
		let index = usize::try_from(reference.0).ok()?.checked_sub(1)?;
		variables.get(index).cloned()
	}
}

// Server = main-thread code
// ServerThread = networking-thread code
//
// The ServerThreads tell the Server everything through one channel of Events,
// so they arrive in the order they happened.
//
// The world listens on a local socket named after its process, and takes as
// many debug-clients as connect: the launcher that started it, launchers
// attached to it, a hot reload watcher. Each has its own breakpoints and its
// own say on pausing for runtime errors. A pause stops the world for all of
// them, and any of them can let it carry on. It lasts while a client that
// debugs is there: when the last of those leaves, the world carries on,
// whoever else is still connected.
//

type ClientId = u32;

enum Event {
	// A debug-client arrived. The Connection is for the Server to answer on.
	Connected(ClientId, Connection),

	// A request from a debug-client for the Server to handle
	Request(ClientId, Request),

	// The debug-client left, or its connection broke
	Disconnected(ClientId)
}

struct Client {
	connection: Connection,
	// Which runtime errors it wants the world to pause on: of those nothing
	// catches, and of those inside a `try`
	uncaught: Option<RuntimeFilter>,
	caught: Option<RuntimeFilter>,
	// Whether it debugs: a pause waits for those that do (`Request::Watcher`)
	debugs: bool
}

/// What a client wants of a breakpoint it set
pub struct Claim {
	/// It only counts as hit when this holds: `None` for every time
	condition: Option<String>,
	/// Which of its hits to stop at: `None` for all of them
	hit: Option<HitCondition>,
	/// How many times it's been hit since it was set
	hits: u32,
	/// A message for the client at the hits it wants, in place of a stop
	log: Option<Vec<LogPart>>
}

/// The breakpoints, by where they are, and the clients that set each. A
/// client can have two in one place: one on the line, and one on the proc
/// (set with offset 0, `true` here), which starts on that line. Each has its
/// own condition, and goes when it's taken away, not when the other is.
pub type Breakpoints = HashMap<(raw_types::procs::ProcId, u16), BTreeMap<(ClientId, bool), Claim>>;

/// The name an expression's code knows the world's list of globals by. It
/// isn't one an expression can write, so it can't hide anything.
const GLOBALS: &str = "global vars";

/// How long to wait for a request
#[derive(Clone, Copy, PartialEq, Eq)]
enum Wait {
	No,
	/// While there's a client that debugs to send one
	WhileClients,
	/// For as long as it takes, clients coming and going
	Forever
}

/// The world's one listener, which lasts as long as the process. A world
/// reboot puts its Server away, and clients that connect before the next one
/// is made wait for it.
struct Acceptor {
	/// Where the listener's thread sends what it hears
	events: mpsc::Sender<Event>,
	/// The other end, for the next Server to take
	waiting: Option<mpsc::Receiver<Event>>
}

static ACCEPTOR: Mutex<Option<Acceptor>> = Mutex::new(None);

// Nothing a debug-client sends is anywhere near this big. Without a limit, any
// stray bytes on the socket get read as a length and we try to allocate
// it. A hot reload of every proc that uses a changed #define can get big.
const MAX_REQUEST_LEN: u32 = 256 * 1024 * 1024;

pub struct Server {
	events: mpsc::Receiver<Event>,
	clients: BTreeMap<ClientId, Client>,
	// The client whose request is being answered
	current: Option<ClientId>,
	state: Option<State>,
	in_eval: bool,
	eval_error: Option<String>,
	breakpoints: Breakpoints,
	/// Why and where the world is paused, while it is
	paused: Option<Stop>,
	/// The client that asked for the world to stay paused once every client
	/// has gone (`Request::HoldPause`)
	hold: Option<ClientId>,
	/// The procs in each file (as the world's DbgFile spells them), and how
	/// many procs the world had when they were counted
	procs_by_file: Option<(u32, HashMap<String, Vec<u32>>)>,
	moved_breakpoints: hot_reload::MovedBreakpoints,
	app: Command<'static>
}

struct ServerThread {
	events: mpsc::Sender<Event>
}

impl Server {
	pub fn setup_app() -> Command<'static> {
		Command::new("Auxtools Debug Server")
			.version(env!("CARGO_PKG_VERSION"))
			.subcommand_required(true)
			.no_binary_name(true)
			.color(clap::ColorChoice::Never)
			.disable_version_flag(true)
			.disable_help_flag(true)
			.override_usage("#<SUBCOMMAND>")
			.subcommand(
				Command::new("disassemble")
					.alias("dis")
					.about("Disassembles a proc and displays its bytecode in an assembly-like format")
					.after_help("If no parameters are provided, the proc executing in the currently debugged stack frame will be disassembled")
					.arg(
						Arg::new("proc")
							.help("Path of the proc to disassemble (e.g. /proc/do_stuff)")
							.takes_value(true)
					)
					.arg(
						Arg::new("id")
							.help("Id of the proc to disassemble (for when multiple procs are defined with the same path)")
							.takes_value(true)
					)
			)
			.subcommand(
				Command::new("guest_override")
					.about("Override the CKey used by guest connections")
					.arg(Arg::new("ckey").takes_value(true))
			)
			.subcommand(
				Command::new("mem_profiler")
					.about("Memory profiler")
					.subcommand(
						Command::new("begin")
							.about("Begins memory profiling. Output goes to the specified file path")
							.arg(Arg::new("path").help("Where to output memory profiler results").takes_value(true))
					)
					.subcommand(Command::new("end").about("Finishes current memory profiler."))
			)
	}

	/// Takes the clients from the world's local socket, which it starts
	/// listening on the first time
	pub fn listen() -> std::io::Result<Server> {
		let events = Self::take_events()?;
		Ok(Server {
			events,
			clients: BTreeMap::new(),
			current: None,
			state: None,
			in_eval: false,
			eval_error: None,
			breakpoints: HashMap::new(),
			moved_breakpoints: HashMap::new(),
			paused: None,
			hold: None,
			procs_by_file: None,
			app: Self::setup_app()
		})
	}

	/// What the listener hears, for one Server at a time
	fn take_events() -> std::io::Result<mpsc::Receiver<Event>> {
		let mut acceptor = ACCEPTOR.lock().unwrap();
		if acceptor.is_none() {
			let listener = connection::listen(&connection::world_name())?;
			let (events, waiting) = mpsc::channel();
			*acceptor = Some(Acceptor {
				events,
				waiting: Some(waiting)
			});
			thread::Builder::new()
				.name("debug server listener".to_owned())
				.spawn(move || Self::accept(&listener))?;
			waker::start()?;
		}
		acceptor
			.as_mut()
			.and_then(|acceptor| acceptor.waiting.take())
			.ok_or_else(|| std::io::Error::other("there's a debug server already"))
	}

	/// The listener's thread
	fn accept(listener: &connection::Listener) {
		let mut next_id = 0;
		loop {
			let Ok(connection) = listener.accept() else {
				continue;
			};
			next_id += 1;

			// Under the lock, so it goes to the Server that's there now, or
			// the next one if there's none
			let events = {
				let acceptor = ACCEPTOR.lock().unwrap();
				let Some(acceptor) = acceptor.as_ref() else {
					return;
				};
				let _ = acceptor.events.send(Event::Connected(next_id, connection.clone()));
				acceptor.events.clone()
			};

			// It reads until the client hangs up, which it does when told to
			// disconnect, so nothing waits for it
			let server_thread = ServerThread { events };
			let id = next_id;
			thread::spawn(move || server_thread.run(id, connection));
		}
	}

	/// Tells whoever connects that there's no debugger, and why
	pub fn refuse(message: String) {
		let Ok(events) = Self::take_events() else {
			return;
		};
		thread::spawn(move || {
			while let Ok(event) = events.recv() {
				if let Event::Connected(_, mut connection) = event {
					for response in [Response::Notification { message: message.clone() }, Response::Disconnect] {
						if Self::send(&mut connection, &response).is_err() {
							break;
						}
					}
				}
			}
		});
	}

	pub const fn is_in_eval(&self) -> bool {
		self.in_eval
	}

	pub fn set_eval_error(&mut self, err: String) {
		self.eval_error = Some(err);
	}

	fn get_line_number(&self, proc: ProcRef, offset: u32) -> Option<u32> {
		match auxtools::Proc::find_override(proc.path, proc.override_id) {
			Some(proc) => Self::line_number_in(unsafe { proc.bytecode() }, offset),
			None => None
		}
	}

	/// The code a frame runs, when it isn't its proc's current code but what
	/// that was before a hot reload: a frame runs the code it started with.
	/// `None` too when the two only differ in their line numbers (the proc's
	/// lines moved, its code didn't): the current ones are right for it.
	fn old_code_of(frame: &debug::StackFrame) -> Option<Vec<u32>> {
		let running = unsafe { *(*frame.context).bytecode() };
		let (current, _) = unsafe { frame.proc.bytecode_mut_ptr() };
		if running == current {
			return None;
		}
		let old = auxtools::bytecode_at(running)?;

		let instructions = |bytecode: &[u32]| -> Vec<dmasm::Instruction> {
			let mut env = disassemble_env::DisassembleEnv;
			let (nodes, _error) = dmasm::disassembler::disassemble(bytecode, &mut env);
			nodes
				.into_iter()
				.filter_map(|node| match node {
					dmasm::Node::Instruction(dmasm::Instruction::DbgLine(_), _) => Some(dmasm::Instruction::DbgLine(0)),
					dmasm::Node::Instruction(ins, _) => Some(ins),
					_ => None
				})
				.collect()
		};
		let now = original_bytecode(&frame.proc);
		if old.len() == now.len() && instructions(&old) == instructions(&now) {
			return None;
		}
		Some(old)
	}

	/// The line a frame is on. A frame that's `asleep` is where it will wake,
	/// after the instruction it sleeps in: when that's the start of the next
	/// line, it's still on the one before.
	fn frame_line_number(&self, frame: &debug::StackFrame, asleep: bool) -> Option<u32> {
		let offset = if asleep { (frame.offset as u32).saturating_sub(1) } else { frame.offset as u32 };

		if let Some(bytecode) = Self::old_code_of(frame) {
			return Self::line_number_in(&bytecode, offset);
		}

		self.get_line_number(
			ProcRef {
				path: frame.proc.path.clone(),
				override_id: frame.proc.override_id()
			},
			offset
		)
	}

	/// The file a proc is in, if its code starts with DbgFile
	fn file_of(proc: &Proc) -> Option<String> {
		let dbg_file = dmasm::Opcode::DbgFile.word();
		let bytecode = unsafe { proc.bytecode() };
		let id = match bytecode {
			[op, id, ..] if *op == dbg_file => *id,
			// A breakpoint at its start hides it
			[_, ..] => match crate::instruction_hooking::original_bytecode(proc).as_slice() {
				[op, id, ..] if *op == dbg_file => *id,
				_ => return None
			},
			[] => return None
		};
		let name = unsafe { StringRef::from_id(raw_types::strings::StringId(id)) };
		Some(String::from_utf8_lossy(name.data()).into_owned())
	}

	/// The procs in each file, by the name the world was built with
	fn file_index(&mut self) -> &HashMap<String, Vec<u32>> {
		// Counted again when procs were added
		let count = auxtools::runtime_tables::proc_count();
		if self.procs_by_file.as_ref().is_none_or(|(counted, _)| *counted != count) {
			let mut by_file: HashMap<String, Vec<u32>> = HashMap::new();
			for id in 0..count {
				if let Some(file) = Proc::from_id(raw_types::procs::ProcId(id)).and_then(|proc| Self::file_of(&proc)) {
					by_file.entry(file).or_default().push(id);
				}
			}
			self.procs_by_file = Some((count, by_file));
		}
		&self.procs_by_file.as_ref().unwrap().1
	}

	/// The procs with code in `file` (spelled as `Request::Locate` has it),
	/// and the lines each has code on, as the world runs it now
	fn procs_in(&mut self, file: &str) -> Vec<(ProcRef, Vec<u32>)> {
		let normalize = |path: &str| path.replace('\\', "/").to_ascii_lowercase();
		let wanted = normalize(file);
		let same = |file: &str| file == wanted || file.ends_with(&format!("/{}", wanted)) || wanted.ends_with(&format!("/{}", file));

		let ids: Vec<u32> = self
			.file_index()
			.iter()
			.filter(|(file, _)| same(&normalize(file)))
			.flat_map(|(_, ids)| ids.iter().copied())
			.collect();
		ids.into_iter()
			.filter_map(|id| Proc::from_id(raw_types::procs::ProcId(id)))
			.map(|proc| {
				let lines = Self::lines_of(&proc);
				let proc = ProcRef {
					path: proc.path.clone(),
					override_id: proc.override_id()
				};
				(proc, lines)
			})
			.collect()
	}

	/// The lines from `start` to `end` of `file` that some proc has code on
	fn lines_with_code(&mut self, file: &str, start: u32, end: u32) -> Vec<u32> {
		let mut lines: Vec<u32> = self
			.procs_in(file)
			.into_iter()
			.flat_map(|(_, lines)| lines)
			.filter(|line| (start..=end).contains(line))
			.collect();
		lines.sort_unstable();
		lines.dedup();
		lines
	}

	/// Where each proc with code on `line` of `file` has it
	fn locate(&mut self, file: &str, line: u32) -> Vec<InstructionRef> {
		// A line with no code of its own (a proc's declaration, a comment, the
		// middle of a statement) has its breakpoint on the next one that has
		let procs = self.procs_in(file);
		let Some(line) = procs.iter().flat_map(|(_, lines)| lines.iter().copied()).filter(|&l| l >= line).min() else {
			return vec![];
		};

		let mut found = vec![];
		for (proc, lines) in procs {
			if !lines.contains(&line) {
				continue;
			}
			if let Some(offset) = self.get_offset(proc.clone(), line) {
				found.push(InstructionRef { proc, offset });
			}
		}
		found
	}

	/// The lines a proc has code on (its DbgLines)
	fn lines_of(proc: &Proc) -> Vec<u32> {
		let bytecode = crate::instruction_hooking::original_bytecode(proc);
		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, _error) = dmasm::disassembler::disassemble(&bytecode, &mut env);
		nodes
			.into_iter()
			.filter_map(|node| match node {
				dmasm::Node::Instruction(dmasm::Instruction::DbgLine(line), _) => Some(line),
				_ => None
			})
			.collect()
	}

	fn line_number_in(bytecode: &[u32], offset: u32) -> Option<u32> {
		let mut current_line_number = None;
		let mut reached_offset = false;

		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, _error) = dmasm::disassembler::disassemble(bytecode, &mut env);

		for node in nodes {
			if let dmasm::Node::Instruction(ins, debug) = node {
				if debug.offset > offset && !reached_offset {
					reached_offset = true;
					if current_line_number.is_some() {
						break;
					}
				}

				if let dmasm::Instruction::DbgLine(line) = ins {
					current_line_number = Some(line);
					// Before its first line, a proc is at its first line
					if reached_offset {
						break;
					}
				}

				if debug.offset == offset {
					reached_offset = true;
					if current_line_number.is_some() {
						break;
					}
				}
			}
		}

		if reached_offset {
			current_line_number
		} else {
			None
		}
	}

	fn get_offset(&self, proc: ProcRef, line: u32) -> Option<u32> {
		let proc = auxtools::Proc::find_override(proc.path, proc.override_id)?;
		let mut offset = None;
		let mut at_offset = false;

		let bytecode = unsafe { proc.bytecode() };

		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, _error) = dmasm::disassembler::disassemble(bytecode, &mut env);

		for node in nodes {
			if let dmasm::Node::Instruction(ins, debug) = node {
				if at_offset {
					offset = Some(debug.offset);
					break;
				}

				if let dmasm::Instruction::DbgLine(current_line) = ins {
					if current_line == line {
						at_offset = true;
					}
				}
			}
		}

		offset
	}

	// 516 primitives have no `vars` list, so their fields are spelled out here.
	// a callee only reads while its proc is still running. A savefile has a
	// `vars` list, but not one BYOND counts references to: holding it gets a
	// "Bad ref" in the world's log each time.
	const fn primitive_fields(value: &Value) -> Option<&'static [&'static str]> {
		match value.raw.tag {
			ValueTag::Vector => Some(&["x", "y", "z", "len", "size"]),
			ValueTag::PixLoc => Some(&["x", "y", "z", "step_x", "step_y", "loc"]),
			ValueTag::Callee => Some(&["proc", "file", "line", "src", "usr", "args", "caller"]),
			ValueTag::SaveFile => Some(&["type", "name", "cd", "dir", "eof", "byond_version", "byond_build"]),
			_ => None
		}
	}

	fn is_object(value: &Value) -> bool {
		// Hack for globals
		if value.raw.tag == ValueTag::World && unsafe { value.raw.data.id == 1 } {
			return true;
		}

		// A typepath answers for `vars` too, with something that isn't a list
		Self::primitive_fields(value).is_some() || value.get(byond_string!("vars")).is_ok_and(|vars| List::is_list(&vars))
	}

	fn stringify(value: &Value) -> String {
		if List::is_list(value) {
			match List::from_value(value) {
				Ok(list) => format!("/{} {{len = {}}}", if list.is_alist() { "alist" } else { "list" }, list.len()),
				Err(Runtime { message }) => format!("/list (failed to get len: {:?})", message)
			}
		} else {
			match value.to_string() {
				// What BYOND has no text for
				Ok(v) if v.is_empty() => match value.raw.tag {
					ValueTag::World => "world".to_owned(),
					ValueTag::Image => "/image".to_owned(),
					ValueTag::Appearance => "appearance".to_owned(),
					_ => value.raw.to_string()
				},
				Ok(value) => value,
				Err(Runtime { message }) => {
					format!("{} -- stringify error: {:?}", value.raw, message)
				}
			}
		}
	}

	/// What kind of value it is: an object's type, or a word for what isn't
	/// an object
	fn kind(value: &Value) -> Option<String> {
		use ValueTag as Tag;
		if List::is_list(value) {
			return Some(if List::from_value(value).is_ok_and(|list| list.is_alist()) { "/alist" } else { "/list" }.to_owned());
		}
		let word = match value.raw.tag {
			Tag::Null => "null",
			Tag::Number => "num",
			Tag::String => "text",
			Tag::World => "/world",
			Tag::Resource | Tag::File => "file",
			Tag::ProcRef => "proc",
			Tag::Pointer => "pointer",
			Tag::Appearance => "appearance",
			Tag::Vector => "/vector",
			Tag::PixLoc => "/pixloc",
			Tag::Callee => "/callee",
			Tag::MobTypepath |
			Tag::ObjTypepath |
			Tag::TurfTypepath |
			Tag::AreaTypepath |
			Tag::DatumTypepath |
			Tag::SaveFileTypepath |
			Tag::ListTypepath |
			Tag::ClientTypepath |
			Tag::ImageTypepath |
			Tag::BuiltinTypepath => "path",
			_ => return value.get(byond_string!("type")).ok().map(|path| Self::stringify(&path))
		};
		Some(word.to_owned())
	}

	/// A value written as DM code, if it's one that code can write: a key
	/// to read a list's entry by
	fn literal(value: &Value) -> Option<String> {
		use ValueTag as Tag;
		match value.raw.tag {
			Tag::String | Tag::Number | Tag::MobTypepath | Tag::ObjTypepath | Tag::TurfTypepath | Tag::AreaTypepath | Tag::DatumTypepath => {
				Some(Self::stringify(value))
			}
			_ => None
		}
	}

	/// A variable as a client is told of it. `expr` is an expression that
	/// gives it, where there is one.
	fn value_to_variable(&self, name: String, value: &Value, expr: Option<String>) -> Variable {
		Variable {
			name,
			value: Self::stringify(value),
			variables: self.value_to_variables_ref(value, expr.clone()),
			kind: Self::kind(value),
			entries: Self::entries(value),
			expr
		}
	}

	/// How many entries a list has
	fn entries(value: &Value) -> Option<u32> {
		if !List::is_list(value) {
			return None;
		}
		List::from_value(value).ok().map(|list| list.len())
	}

	fn value_to_variables_ref(&self, value: &Value, expr: Option<String>) -> Option<VariablesRef> {
		match self.state.as_ref() {
			Some(state) if List::is_list(value) => Some(state.get_ref(Variables::ListContents(value.clone(), expr))),

			Some(state) if Self::is_object(value) => Some(state.get_ref(Variables::ObjectVars(value.clone(), expr))),

			_ => None
		}
	}

	/// A list's entries, or `count` of them from `start` (from 0) for a
	/// client that takes a long list a piece at a time
	fn list_to_variables(&mut self, value: &Value, expr: Option<&str>, start: Option<u32>, count: Option<u32>) -> Result<Vec<Variable>, Runtime> {
		let state = self.state.as_ref().unwrap();
		let list = List::from_value(value)?;
		let len = list.len();
		let first = start.unwrap_or(0).min(len);
		let last = count.map_or(len, |count| first.saturating_add(count).min(len));

		// An entry with a value for its key: `key = value`, to open for the
		// two, and reached as `list[key]`
		let pair = |i: u32, key: Value, value: Value| Variable {
			name: format!("[{}]", i),
			value: format!("{} = {}", Self::stringify(&key), Self::stringify(&value)),
			kind: Self::kind(&value),
			entries: None,
			expr: expr.zip(Self::literal(&key)).map(|(list, key)| format!("{}[{}]", list, key)),
			variables: Some(state.get_ref(Variables::ListPair {
				key,
				value,
				of: Value::from(&list),
				list: expr.map(str::to_owned)
			}))
		};

		// numbers are keys in an alist, so the position walk below would read
		// the wrong entries
		if list.is_alist() {
			return Ok(list
				.alist_pairs()?
				.into_iter()
				.enumerate()
				.skip(first as usize)
				.take((last - first) as usize)
				.map(|(i, (key, value))| pair(i as u32 + 1, key, value))
				.collect());
		}

		let mut variables = vec![];

		for i in first + 1..=last {
			let key = list.get(i)?;

			// assoc entry
			if key.raw.tag != raw_types::values::ValueTag::Number {
				if let Ok(value) = list.get(&key) {
					if value.raw.tag != raw_types::values::ValueTag::Null {
						variables.push(pair(i, key, value));
						continue;
					}
				}
			}

			// non-assoc entry
			variables.push(self.value_to_variable(format!("[{}]", i), &key, expr.map(|list| format!("{}[{}]", list, i))));
		}

		Ok(variables)
	}

	fn object_to_variables(&mut self, value: &Value, expr: Option<&str>) -> Result<Vec<Variable>, Runtime> {
		let member = |name: &str| expr.map(|object| format!("{}.{}", object, name));

		if let Some(fields) = Self::primitive_fields(value) {
			return fields
				.iter()
				.map(|&name| Ok(self.value_to_variable(name.to_owned(), &value.get(StringRef::new(name)?)?, member(name))))
				.collect();
		}

		let vars = List::from_value(&value.get(byond_string!("vars"))?)?;

		let mut variables = vec![];
		let mut top_variables = vec![]; // These fields get displayed on top of
										// all others

		for i in 1..=vars.len() {
			let name = vars.get(i)?.as_string()?;
			let value = value.get(StringRef::new(name.as_str())?)?;
			let expr = member(&name);
			let variable = self.value_to_variable(name, &value, expr);
			if variable.name == "type" {
				top_variables.push(variable);
			} else {
				variables.push(variable);
			}
		}

		variables.sort_by_key(|a| a.name.to_lowercase());
		top_variables.append(&mut variables);

		Ok(top_variables)
	}

	/// The names of the constants BYOND declares itself, from its stddef.dm:
	/// the lines under each `var/const`
	fn builtin_constants() -> std::collections::HashSet<String> {
		let mut names = std::collections::HashSet::new();
		let mut in_consts = false;
		for line in crate::stddef::get_stddef().unwrap_or_default().lines() {
			if line.trim_end() == "var/const" {
				in_consts = true;
			} else if in_consts && line.starts_with([' ', '\t']) {
				if let Some(name) = line.split(['=', '/']).next().map(str::trim).filter(|name| !name.is_empty()) {
					names.insert(name.to_owned());
				}
			} else if !line.trim().is_empty() {
				in_consts = false;
			}
		}
		names
	}

	/// The global vars: the world's own (types' static and const vars are
	/// among them), or the constants BYOND declares, which would bury them
	fn globals_to_variables(&mut self, builtin: bool) -> Result<Vec<Variable>, Runtime> {
		let names = List::from_value(&unsafe { Value::new(ValueTag::GlobalVars, ValueData { id: 0 }) })?;
		let builtins = Self::builtin_constants();

		let mut variables = vec![];
		let mut constants = 0;
		for i in 1..=names.len() {
			let name = names.get(i)?.as_string()?;
			if builtins.contains(&name) != builtin {
				constants += 1;
				continue;
			}
			let value = Value::GLOBAL.get(StringRef::new(name.as_str())?)?;
			let expr = format!("global.{}", name);
			variables.push(self.value_to_variable(name, &value, Some(expr)));
		}
		variables.sort_by_key(|a| a.name.to_lowercase());

		if !builtin && constants > 0 {
			variables.push(Variable {
				name: "(BYOND's)".to_owned(),
				value: format!("{} constants", constants),
				variables: self.state.as_ref().map(|state| state.get_ref(Variables::Builtins)),
				kind: None,
				expr: None,
				entries: None
			});
		}
		Ok(variables)
	}

	/// The id of a stack (`Stack::id`), the same from one stop to the next:
	/// the id BYOND gives the proc instance of its outermost frame, which is
	/// what its `args` list is known by. No two instances have the same at
	/// once, and an instance keeps its own for as long as the proc goes on.
	/// Its address won't do: a proc that's called starts out on the machine's
	/// stack, and BYOND moves it the first time it sleeps. Nor is it a
	/// `spawn`ed copy's, which gets an id of its own.
	fn thread_of(&self, stack: &[debug::StackFrame]) -> u32 {
		// 0 is "the running proc's" in a request
		stack.last().map_or(0, |frame| unsafe { (*frame.instance).argslist_idx.id }.wrapping_add(1))
	}

	/// Where the stack a client calls `thread` is: 0 for the running proc's
	/// (which a client can call 0 too), then the sleeping ones as BYOND has
	/// them
	fn stack_position(&self, thread: u32) -> Option<usize> {
		let state = self.state.as_ref()?;
		if thread == 0 {
			return Some(0);
		}
		let is = |stack: &[debug::StackFrame]| self.thread_of(stack) == thread;
		if is(state.active()) {
			return Some(0);
		}
		state.suspended().iter().position(|stack| is(stack)).map(|at| at + 1)
	}

	fn get_stack(&self, position: usize) -> Option<&Vec<debug::StackFrame>> {
		let state = self.state.as_ref()?;

		if position == 0 {
			return Some(state.active());
		}

		state.suspended().get(position - 1)
	}

	fn get_stack_base_frame_id(&self, position: usize) -> u32 {
		let Some(state) = &self.state else {
			return 0;
		};

		if position == 0 {
			return 0;
		}

		let mut current_base = state.active().len();

		for frame in &state.suspended()[..position - 1] {
			current_base += frame.len();
		}

		current_base as u32
	}

	fn get_stack_frame(&self, frame_index: u32) -> Option<&debug::StackFrame> {
		let mut frame_index = frame_index as usize;
		let state = self.state.as_ref()?;
		let active = state.active();

		if frame_index < active.len() {
			return Some(&active[frame_index]);
		}

		frame_index -= active.len();

		for frame in state.suspended() {
			if frame_index < frame.len() {
				return Some(&frame[frame_index]);
			}

			frame_index -= frame.len();
		}

		None
	}

	fn get_args(&mut self, frame_index: u32) -> Vec<Variable> {
		let Some(frame) = self.get_stack_frame(frame_index) else {
			return vec![];
		};
		let mut vars = vec![
			self.value_to_variable("src".to_owned(), &frame.src, Some("src".to_owned())),
			self.value_to_variable("usr".to_owned(), &frame.usr, Some("usr".to_owned())),
		];

		let mut unnamed_count = 0;
		for (i, (name, value)) in frame.args.iter().enumerate() {
			// One more than the proc declares is only in its args
			let (name, expr) = match name {
				Some(name) => (String::from(name), String::from(name)),
				None => {
					unnamed_count += 1;
					(format!("undefined argument #{}", unnamed_count), format!("args[{}]", i + 1))
				}
			};
			vars.push(self.value_to_variable(name, value, Some(expr)));
		}

		vars
	}

	fn get_locals(&mut self, frame_index: u32) -> Vec<Variable> {
		let Some(frame) = self.get_stack_frame(frame_index) else {
			return vec![];
		};
		let mut vars = vec![self.value_to_variable(".".to_owned(), &frame.dot, Some(".".to_owned()))];

		for (name, local) in &frame.locals {
			// Two of one name (each in a block of its own) can't both be
			// meant by it
			let name = String::from(name);
			let alone = frame.locals.iter().filter(|(other, _)| String::from(other) == name).count() == 1;
			let expr = alone.then(|| name.clone());
			vars.push(self.value_to_variable(name, local, expr));
		}

		vars
	}

	/// Where a breakpoint on a proc itself goes: after the records of its
	/// file and first line (a verb has a few), so the proc is stopped on that
	/// line like any other, and the first step from there moves on. 0 for
	/// code that doesn't start with them.
	fn entry_offset(proc: &Proc) -> u32 {
		let bytecode = original_bytecode(proc);
		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, _error) = dmasm::disassembler::disassemble(&bytecode, &mut env);

		let mut has_line = false;
		for node in nodes {
			match node {
				dmasm::Node::Instruction(dmasm::Instruction::DbgFile(_), _) => {}
				dmasm::Node::Instruction(dmasm::Instruction::DbgLine(_), _) => has_line = true,
				dmasm::Node::Instruction(_, debug) if has_line => return debug.offset,
				dmasm::Node::Instruction(..) => return 0,
				_ => {}
			}
		}
		0
	}

	/// The offset a client means: 0 is the proc itself (see `entry_offset`)
	fn breakpoint_offset(proc: &Proc, offset: u32) -> u32 {
		if offset == 0 {
			Self::entry_offset(proc)
		} else {
			offset
		}
	}

	fn handle_breakpoint_set(&mut self, mut instruction: InstructionRef, condition: Option<String>, hit: Option<HitCondition>, log: Option<Vec<LogPart>>) {
		let (Some(proc), Some(client)) = (
			auxtools::Proc::find_override(instruction.proc.path.clone(), instruction.proc.override_id),
			self.current
		) else {
			self.send_or_disconnect(Response::BreakpointSet {
				result: BreakpointSetResult::Failed
			});
			return;
		};
		let on_proc = instruction.offset == 0;
		instruction.offset = Self::breakpoint_offset(&proc, instruction.offset);
		let line = self.get_line_number(instruction.proc.clone(), instruction.offset);

		// A condition that doesn't compile would pause at every hit, to say so
		// each time: better said once, now. The same goes for a message's
		// expressions.
		let log_error = || {
			log.iter().flatten().find_map(|part| match part {
				LogPart::Expr(expression) => {
					Self::expression_error(&proc, expression).map(|e| format!("the message's {{{}}} doesn't compile: {}", expression, e))
				}
				LogPart::Text(_) => None
			})
		};
		let condition_error = condition
			.as_deref()
			.and_then(|condition| Self::expression_error(&proc, condition))
			.map(|e| format!("the condition doesn't compile: {}", e));
		if let Some(reason) = condition_error.or_else(log_error) {
			self.send_or_disconnect(Response::BreakpointSet {
				result: BreakpointSetResult::Refused { reason }
			});
			return;
		}

		match hook_instruction(&proc, instruction.offset) {
			Ok(()) => {
				// The client re-sends breakpoints it already set, and no
				// condition this time means the old one is gone. Its hits are
				// counted from now.
				let key = (proc.id, instruction.offset as u16);
				self.breakpoints.entry(key).or_default().insert((client, on_proc), Claim {
					condition,
					hit,
					hits: 0,
					log
				});

				// The client has found out where a reload moved a breakpoint to
				if !self.breakpoints.values().any(|claims| claims.keys().any(|(other, _)| *other != client)) {
					self.moved_breakpoints.retain(|(id, _), at| *id != proc.id || *at != instruction.offset);
				}

				// As the proc itself names it, whatever the client called it
				instruction.proc = ProcRef {
					path: proc.path.clone(),
					override_id: proc.override_id()
				};
				self.send_or_disconnect(Response::BreakpointSet {
					result: BreakpointSetResult::Success { line, at: instruction }
				});
			}

			Err(_) => {
				self.send_or_disconnect(Response::BreakpointSet {
					result: BreakpointSetResult::Failed
				});
			}
		}
	}

	/// What's wrong with an expression of a breakpoint in a proc (its
	/// condition, one in its message), if it doesn't compile. The names it can
	/// use are checked when it's hit, in the frame it's hit in.
	fn expression_error(proc: &Proc, condition: &str) -> Option<String> {
		let mut names = vec![".".to_owned(), "usr".to_owned(), "src".to_owned()];
		names.extend(proc.parameter_names().iter().map(String::from));
		names.extend(proc.local_names().iter().map(String::from));
		let names: Vec<&str> = names.iter().map(String::as_str).collect();
		dmasm::compiler::compile_expr_for(condition, &names, crate::world_byond())
			.err()
			.map(|e| e.to_string())
	}

	/// A breakpoint's message, with its expressions' values in the frame
	/// that hit it. A text's value is the text itself, not quoted.
	fn log_message(&mut self, parts: &[LogPart]) -> String {
		let mut message = String::new();
		for part in parts {
			match part {
				LogPart::Text(text) => message.push_str(text),
				LogPart::Expr(expression) => {
					match self.eval_expr(Some(0), expression) {
						Ok(value) => message.push_str(&value.as_string().unwrap_or_else(|_| Self::stringify(&value))),
						Err(error) => message.push_str(&format!("{{{}: {}}}", expression, error))
					}
					self.state.as_mut().unwrap().invalidate_stacks();
				}
			}
		}
		message
	}

	fn handle_breakpoint_unset(&mut self, instruction: InstructionRef) {
		let (Some(proc), Some(client)) = (
			auxtools::Proc::find_override(instruction.proc.path, instruction.proc.override_id),
			self.current
		) else {
			self.send_or_disconnect(Response::BreakpointUnset { success: false });
			return;
		};

		// It might be one a reload moved, known to the client by where it was
		let on_proc = instruction.offset == 0;
		let asked = Self::breakpoint_offset(&proc, instruction.offset);
		let offset = self.moved_breakpoints.get(&(proc.id, asked)).copied().unwrap_or(asked);

		let key = (proc.id, offset as u16);
		let claims = self.breakpoints.entry(key).or_default();
		let had = claims.remove(&(client, on_proc)).is_some();
		let success = if claims.is_empty() {
			// Nobody else wants it
			self.breakpoints.remove(&key);
			self.moved_breakpoints.remove(&(proc.id, asked));
			unhook_instruction(&proc, offset).is_ok()
		} else {
			had
		};
		self.send_or_disconnect(Response::BreakpointUnset { success });
	}

	fn handle_stacks(&mut self, sleeping: Option<u32>) {
		let sleeping = sleeping.unwrap_or(DEFAULT_SLEEPING_STACKS) as usize;
		if let Some(state) = &mut self.state {
			state.read_sleeping(sleeping);
		}
		let mut asleep = 0;
		let stacks = match &self.state {
			Some(state) => {
				asleep = debug::CallStacks::suspended_instances().len();
				let listed = std::iter::once(state.active()).chain(state.suspended().iter().take(sleeping));
				listed
					.enumerate()
					.filter(|(_, stack)| !stack.is_empty())
					.map(|(position, stack)| Stack {
						id: self.thread_of(stack),
						name: stack[0].proc.path.clone(),
						file: Self::file_of(&stack[0].proc),
						// A sleeping stack's innermost frame is the one asleep
						line: self.frame_line_number(&stack[0], position != 0),
						sleeping: position != 0
					})
					.collect()
			}

			None => vec![]
		};

		self.send_or_disconnect(Response::Stacks {
			stacks,
			sleeping: asleep as u32
		});
	}

	fn handle_stack_frames(&mut self, thread: u32, start_frame: Option<u32>, count: Option<u32>) {
		let position = self.stack_position(thread);
		let response = match position.and_then(|position| self.get_stack(position)) {
			Some(stack) => {
				let stack_id = position.unwrap_or(0);
				let frame_base = self.get_stack_base_frame_id(stack_id);
				let start_frame = start_frame.unwrap_or(0);
				let end_frame = start_frame + count.unwrap_or(stack.len() as u32);

				let start_frame = start_frame as usize;
				let end_frame = end_frame as usize;

				let mut frames = vec![];

				for i in start_frame..end_frame {
					if i >= stack.len() {
						break;
					}

					let proc_ref = ProcRef {
						path: stack[i].proc.path.to_owned(),
						override_id: stack[i].proc.override_id()
					};

					frames.push(StackFrame {
						id: frame_base + (i as u32),
						instruction: InstructionRef {
							proc: proc_ref.clone(),
							offset: stack[i].offset as u32
						},
						// A sleeping stack's innermost frame is the one asleep
						line: self.frame_line_number(&stack[i], stack_id != 0 && i == 0),
						file: Self::file_of(&stack[i].proc),
						old_code: Self::old_code_of(&stack[i]).is_some()
					});
				}

				Response::StackFrames {
					frames,
					total_count: stack.len() as u32
				}
			}

			None => {
				if self.state.is_none() {
					self.notify("received StackFrames request when not paused");
				}
				Response::StackFrames {
					frames: vec![],
					total_count: 0
				}
			}
		};

		self.send_or_disconnect(response);
	}

	fn handle_scopes(&mut self, frame_id: u32) {
		let response = match &self.state {
			Some(state) => Response::Scopes {
				arguments: Some(state.get_ref(Variables::Arguments { frame: frame_id })),
				locals: Some(state.get_ref(Variables::Locals { frame: frame_id })),
				globals: Some(state.get_ref(Variables::Globals))
			},
			None => Response::Scopes {
				arguments: None,
				locals: None,
				globals: None
			}
		};

		self.send_or_disconnect(response);
	}

	/// What's inside something a client was given to open
	fn list_variables(&mut self, container: Variables, start: Option<u32>, count: Option<u32>) -> Result<Vec<Variable>, Runtime> {
		match container {
			Variables::Arguments { frame } => Ok(self.get_args(frame)),
			Variables::Locals { frame } => Ok(self.get_locals(frame)),
			Variables::Globals => self.globals_to_variables(false),
			Variables::Builtins => self.globals_to_variables(true),
			Variables::ObjectVars(value, expr) => self.object_to_variables(&value, expr.as_deref()),
			Variables::ListContents(value, expr) => self.list_to_variables(&value, expr.as_deref(), start, count),
			Variables::ListPair { key, value, list, .. } => {
				let entry = list.zip(Self::literal(&key)).map(|(list, key)| format!("{}[{}]", list, key));
				Ok(vec![
					self.value_to_variable("key".to_owned(), &key, None),
					self.value_to_variable("value".to_owned(), &value, entry),
				])
			}
		}
	}

	/// A reference from before the world last carried on, or one that never
	/// was, has nothing inside.
	fn handle_variables(&mut self, vars: VariablesRef, start: Option<u32>, count: Option<u32>) {
		let listed = match self.state.as_ref().and_then(|state| state.get_variables(vars)) {
			Some(container) => self.list_variables(container, start, count),
			None => Ok(vec![])
		};

		let vars = listed.unwrap_or_else(|e| {
			self.notify(format!("Couldn't list variables: {}", e.message));
			vec![]
		});
		self.send_or_disconnect(Response::Variables { vars });
	}

	/// The most names a completion gives: more than that is no help to choose
	/// from
	const MOST_COMPLETIONS: usize = 200;

	/// The types an object is of, its own first: `/mob/player`, `/mob`,
	/// `/atom/movable`, `/atom`, `/datum`
	fn types_of(value: &Value) -> Vec<String> {
		let mut types = vec![];
		let mut path = value.get(byond_string!("type")).ok();
		// (a type's parent isn't always the path without its last part)
		while let Some(this) = path.filter(|path| path.raw.tag != ValueTag::Null) {
			let name = Self::stringify(&this);
			if !name.starts_with('/') || types.contains(&name) || types.len() > 64 {
				break;
			}
			types.push(name);
			path = this.get(byond_string!("parent_type")).ok();
		}
		types
	}

	/// The vars and procs of a value, into `found` (a name, and whether
	/// it's a proc)
	fn members_of(value: &Value, found: &mut BTreeMap<String, bool>) {
		if let Some(fields) = Self::primitive_fields(value) {
			found.extend(fields.iter().map(|&name| (name.to_owned(), false)));
		} else if let Ok(vars) = value.get(byond_string!("vars")).and_then(|vars| List::from_value(&vars)) {
			for i in 1..=vars.len() {
				if let Ok(name) = vars.get(i).and_then(|name| name.as_string()) {
					found.insert(name, false);
				}
			}
		}

		let types = Self::types_of(value);
		if types.is_empty() {
			return;
		}
		Self::each_proc(|owner, name| {
			if types.iter().any(|path| path == owner) {
				found.entry(name.to_owned()).or_insert(true);
			}
		});
	}

	/// Calls `each` with every proc's type (`""` for a global proc) and name
	fn each_proc(mut each: impl FnMut(&str, &str)) {
		for id in 0..auxtools::runtime_tables::proc_count() {
			let Some(proc) = Proc::from_id(raw_types::procs::ProcId(id)) else {
				continue;
			};
			// The debugger's own, and a type's init code, aren't for calling
			let Some((owner, name)) = proc.path.rsplit_once('/') else {
				continue;
			};
			if name.starts_with("auxtools_") || name.ends_with(')') || name.is_empty() {
				continue;
			}
			each(owner, name);
		}
	}

	/// The names that could follow what's typed (see `Request::Complete`)
	fn complete(&mut self, frame_id: Option<u32>, base: Option<&str>, prefix: &str) -> Vec<Completion> {
		let mut found: BTreeMap<String, bool> = BTreeMap::new();
		match base {
			Some(base) => {
				let Ok(value) = self.eval_expr(frame_id, base) else {
					return vec![];
				};
				if let Some(state) = self.state.as_mut() {
					state.invalidate_stacks();
				}
				Self::members_of(&value, &mut found);
			}

			None => {
				// What the frame has, then its src's, then the world's
				let mut src = None;
				if let Some(frame) = frame_id.and_then(|frame_id| self.get_stack_frame(frame_id)) {
					for name in [".", "src", "usr", "args"] {
						found.insert(name.to_owned(), false);
					}
					found.extend(frame.args.iter().filter_map(|(name, _)| name.as_ref()).map(|name| (String::from(name), false)));
					found.extend(frame.locals.iter().map(|(name, _)| (String::from(name), false)));
					src = Some(frame.src.clone()).filter(|src| *src != Value::NULL);
				}
				if let Some(src) = &src {
					Self::members_of(src, &mut found);
				}

				for name in ["world", "global"] {
					found.entry(name.to_owned()).or_insert(false);
				}
				if let Ok(names) = List::from_value(&unsafe { Value::new(ValueTag::GlobalVars, ValueData { id: 0 }) }) {
					for i in 1..=names.len() {
						if let Ok(name) = names.get(i).and_then(|name| name.as_string()) {
							found.entry(name).or_insert(false);
						}
					}
				}
				for name in hot_reload::added_globals() {
					found.entry(name).or_insert(false);
				}
				Self::each_proc(|owner, name| {
					if owner.is_empty() {
						found.entry(name.to_owned()).or_insert(true);
					}
				});
			}
		}

		let prefix = prefix.to_lowercase();
		found
			.into_iter()
			.filter(|(name, _)| name.to_lowercase().starts_with(&prefix))
			.take(Self::MOST_COMPLETIONS)
			.map(|(name, proc)| Completion { name, proc })
			.collect()
	}

	/// The entry of a list that a client knows as `[n]`: its place, from 1
	fn entry_number(name: &str) -> Option<u32> {
		name.strip_prefix('[')?.strip_suffix(']')?.parse().ok().filter(|&n| n > 0)
	}

	/// Puts a value where a variable is. The world's own objections (a var
	/// that can't be written) come back as runtime errors.
	fn write_variable(&mut self, container: &Variables, name: &str, value: &Value) -> Result<(), String> {
		let runtime = |e: Runtime| format!("runtime error: {}", e.message);
		// A frame's slot takes a reference of its own, and lets go of what
		// it had
		unsafe fn put(slot: *mut raw_types::values::Value, value: &Value) {
			unsafe {
				let _ = Value::from_raw_owned(*slot);
				let value = value.clone();
				*slot = value.raw;
				std::mem::forget(value);
			}
		}

		match container {
			Variables::Locals { frame } => {
				let frame = self.get_stack_frame(*frame).ok_or("that frame is gone")?;
				if name == "." {
					unsafe { put((*frame.context).dot_mut(), value) };
					return Ok(());
				}
				let mut named = frame.locals.iter().enumerate().filter(|(_, (local, _))| String::from(local) == name);
				match (named.next(), named.next()) {
					(Some((index, _)), None) => {
						unsafe { put((*(*frame.context).locals()).add(index), value) };
						Ok(())
					}
					(Some(_), Some(_)) => Err(format!("the proc has more than one local called {}", name)),
					_ => Err(format!("the frame has no local called {}", name))
				}
			}

			Variables::Arguments { frame } => {
				let frame = self.get_stack_frame(*frame).ok_or("that frame is gone")?;
				match name {
					"src" => unsafe { put(&raw mut (*frame.instance).src, value) },
					"usr" => unsafe { put(&raw mut (*frame.instance).usr, value) },
					_ => {
						// One the proc doesn't declare is known by its number
						// among those
						let unnamed: Option<usize> = name.strip_prefix("undefined argument #").and_then(|n| n.parse().ok());
						let mut without_names = 0;
						let index = frame.args.iter().position(|(arg, _)| match arg {
							Some(arg) => String::from(arg) == name,
							None => {
								without_names += 1;
								unnamed == Some(without_names)
							}
						});
						let index = index.ok_or_else(|| format!("the frame has no argument called {}", name))?;
						unsafe { put((*frame.instance).args().add(index), value) };
					}
				}
				Ok(())
			}

			Variables::Globals => Value::GLOBAL.set(StringRef::new(name).map_err(runtime)?, value).map_err(runtime),
			Variables::Builtins => Err("BYOND's constants can't be changed".to_owned()),
			Variables::ObjectVars(object, _) => object.set(StringRef::new(name).map_err(runtime)?, value).map_err(runtime),

			Variables::ListContents(list, _) => {
				let list = List::from_value(list).map_err(runtime)?;
				let number = Self::entry_number(name).filter(|&n| n <= list.len()).ok_or_else(|| format!("the list has no entry {}", name))?;
				// An entry with a value for its key is given another value
				if list.is_alist() {
					let (key, _) = list.alist_pairs().map_err(runtime)?.into_iter().nth(number as usize - 1).ok_or("the list has changed")?;
					return list.set(&key, value).map_err(runtime);
				}
				let key = list.get(number).map_err(runtime)?;
				let has_value = key.raw.tag != ValueTag::Number && list.get(&key).is_ok_and(|of_key| of_key.raw.tag != ValueTag::Null);
				if has_value {
					list.set(&key, value).map_err(runtime)
				} else {
					list.set(number, value).map_err(runtime)
				}
			}

			Variables::ListPair { key, of, .. } => match name {
				"value" => List::from_value(of).map_err(runtime)?.set(key, value).map_err(runtime),
				_ => Err("an entry's key can't be changed: take the entry out of the list and add another".to_owned())
			}
		}
	}

	/// Gives a variable the value of an expression, and returns it as it is
	/// after that
	fn set_variable(&mut self, vars: VariablesRef, name: &str, value: &str) -> Result<Variable, String> {
		let container = self
			.state
			.as_ref()
			.and_then(|state| state.get_variables(vars))
			.ok_or("the world has carried on since that was listed")?;
		let frame = match &container {
			Variables::Arguments { frame } | Variables::Locals { frame } => *frame,
			_ => 0
		};
		let value = self.eval_expr(Some(frame), value)?;
		self.state.as_mut().unwrap().invalidate_stacks();

		// A runtime error on the way is this request's, not one to pause for
		self.in_eval = true;
		self.eval_error = None;
		let written = self.write_variable(&container, name, &value);
		self.in_eval = false;
		self.state.as_mut().unwrap().invalidate_stacks();
		if let Some(error) = self.eval_error.take() {
			return Err(format!("runtime error: {}", error));
		}
		written?;

		// An entry opened for its key and value was what it was when it was
		// opened: its value is what was just written
		if let Variables::ListPair { key, list, .. } = &container {
			let entry = list.clone().zip(Self::literal(key)).map(|(list, key)| format!("{}[{}]", list, key));
			return Ok(self.value_to_variable(name.to_owned(), &value, entry));
		}

		// As the world has it now, which isn't always what it was given
		let (start, count) = match &container {
			Variables::ListContents(..) => (Self::entry_number(name).map(|n| n - 1), Some(1)),
			_ => (None, None)
		};
		let listed = self.list_variables(container, start, count).map_err(|e| e.message)?;
		listed.into_iter().find(|variable| variable.name == name).ok_or_else(|| format!("{} is gone", name))
	}

	fn handle_command(&mut self, frame_id: Option<u32>, command: &str) -> String {
		// How many matches variables can you spot? It could be better...
		let response = match self.app.try_get_matches_from_mut(command.split_ascii_whitespace()) {
			Ok(matches) => {
				match matches.subcommand() {
					Some(("disassemble", matches)) => {
						if let Some(proc) = matches.value_of("proc") {
							// Default id to 0 in the worst way possible
							let id = matches.value_of("id").and_then(|x| x.parse::<u32>().ok()).unwrap_or(0);

							self.handle_disassemble(proc, id)
						} else if let Some(frame_id) = frame_id {
							if let Some(frame) = self.get_stack_frame(frame_id) {
								let proc = frame.proc.path.clone();
								let id = frame.proc.override_id();
								self.handle_disassemble(&proc, id)
							} else {
								"couldn't find stack frame (is execution not paused?)".to_owned()
							}
						} else {
							"no execution frame selected".to_owned()
						}
					}

					Some(("guest_override", matches)) => match matches.value_of("ckey") {
						Some(ckey) => match crate::ckey_override::override_guest_ckey(ckey) {
							Ok(()) => "Success".to_owned(),

							Err(e) => {
								format!("Failed: {:?}", e)
							}
						},

						None => "no ckey provided".to_owned()
					},

					Some(("mem_profiler", matches)) => match matches.subcommand() {
						Some(("begin", matches)) => match matches.value_of("path") {
							Some(path) => mem_profiler::begin(path)
								.map(|_| "Memory profiler enabled".to_owned())
								.unwrap_or_else(|e| format!("Failed: {}", e)),

							None => "no path provided".to_owned()
						},

						Some(("end", _)) => {
							mem_profiler::end();
							"Memory profiler disabled".to_owned()
						}

						_ => "unknown memory profiler sub-command".to_owned()
					},

					_ => "unknown command".to_owned()
				}
			}
			Err(e) => e.to_string()
		};

		response
	}

	/// What an expression gives in a frame (or in none), or why it gives
	/// nothing
	fn eval_expr(&mut self, frame_id: Option<u32>, command: &str) -> Result<Value, String> {
		// Assembling the expression runs DM code of its own to look up
		// typepaths and resources, so it counts as eval code too. Otherwise the
		// instruction hook would handle the client's next request in the middle
		// of this one, and a runtime would pause inside the pause we're in.
		self.in_eval = true;
		self.eval_error = None;

		let result = self.compile_and_run_expr(frame_id, command);

		self.in_eval = false;

		// Its code failed: whatever it returned isn't the expression's value
		match self.eval_error.take() {
			Some(error) => Err(format!("runtime error: {}", error)),
			None => result
		}
	}

	/// An assemble error in plain words
	fn assemble_error(error: dmasm::assembler::AssembleError) -> String {
		use dmasm::assembler::AssembleError;
		let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
		match error {
			AssembleError::ProcNotFound(path) => format!("there's no proc {}", path),
			AssembleError::VariableNameNotFound(name) => format!("there's no var called {}", text(&name)),
			AssembleError::StringNotFound(string) => format!("nothing in the world is called {}", text(&string)),
			AssembleError::TypeNotFound(path) => format!("there's no type {}", path),
			AssembleError::ResourceNotFound(path) => format!("there's no file '{}'", path),
			other => format!("{:?}", other)
		}
	}

	fn compile_and_run_expr(&mut self, frame_id: Option<u32>, command: &str) -> Result<Value, String> {
		enum ArgType {
			Dot,
			Usr,
			Src,
			Arg(u32),
			Local(u32),
			/// The expression's to use, not the frame's to take back
			Given
		}

		// The world's globals, which the expression reads and writes by name
		let globals = unsafe { Value::new(ValueTag::GlobalVars, ValueData { id: 0 }) };
		let mut src = None;

		let (ctx, instance, mut args) = match frame_id {
			// Global context
			None => (std::ptr::null_mut(), std::ptr::null_mut(), vec![]),

			Some(frame_id) => {
				let Some(frame) = self.get_stack_frame(frame_id) else {
					return Err(format!("there's no frame {} (the world isn't paused, or has carried on since)", frame_id));
				};

				// `args` would be the expression's own, were it not given the
				// frame's
				let frame_args = List::new();
				for (_, value) in &frame.args {
					frame_args.append(value);
				}
				if frame.src != Value::NULL {
					src = Some(frame.src.clone());
				}

				let mut args = vec![
					("args".to_owned(), Value::from(frame_args), ArgType::Given),
					(".".to_owned(), frame.dot.clone(), ArgType::Dot),
					("usr".to_owned(), frame.usr.clone(), ArgType::Usr),
					("src".to_owned(), frame.src.clone(), ArgType::Src),
				];

				for (idx, arg) in frame.args.iter().enumerate() {
					if let Some(name) = &arg.0 {
						args.push((name.into(), arg.1.clone(), ArgType::Arg(idx as u32)));
					}
				}

				for (idx, local) in frame.locals.iter().enumerate() {
					args.push(((&local.0).into(), local.1.clone(), ArgType::Local(idx as u32)));
				}

				(frame.context, frame.instance, args)
			}
		};

		args.push((GLOBALS.to_owned(), globals, ArgType::Given));
		let arg_names: Vec<&str> = args.iter().map(|(name, ..)| name.as_str()).collect();
		let arg_values: Vec<&Value> = args.iter().map(|(_, value, _)| value).collect();

		// A name that isn't the frame's own is a var of its src, then a
		// global, as it would be in the proc's code; a call is src's proc if
		// it has one of that name
		let has_var = |object: &Value, name: &str| StringRef::new(name).is_ok_and(|name| object.get(name).is_ok());
		let scope = dmasm::compiler::ExprScope {
			has_src_var: &|name| src.as_ref().is_some_and(|src| has_var(src, name)),
			has_src_proc: &|name| src.as_ref().is_some_and(|src| crate::assemble_env::AssembleEnv::has_proc(src, name)),
			// One a hot reload added isn't on the world's list of them
			global: &|name| {
				if has_var(&Value::GLOBAL, name) {
					Some(dmasm::compiler::ExprGlobal::Listed)
				} else {
					hot_reload::added_global(name).map(dmasm::compiler::ExprGlobal::Id)
				}
			},
			globals: GLOBALS
		};
		let expr = dmasm::compiler::compile_expr_in(command, &arg_names, Some(&scope), crate::world_byond()).map_err(|e| e.to_string())?;
		let assembly = dmasm::assembler::assemble(&expr, &mut crate::assemble_env::AssembleEnv).map_err(Self::assemble_error)?;
		let proc = Proc::find("/proc/auxtools_expr_stub").ok_or("the world has no /proc/auxtools_expr_stub to evaluate expressions in")?;

		proc.set_bytecode(assembly);

		let ends = crate::WORLD_ENDS.load(std::sync::atomic::Ordering::Relaxed);
		let result = proc.call(&arg_values);
		if crate::WORLD_ENDS.load(std::sync::atomic::Ordering::Relaxed) != ends {
			// The expression ended the world (`world.Reboot()`), and this
			// server with it. The values were the old world's.
			std::mem::forget(result);
			std::mem::forget(args);
			std::panic::resume_unwind(Box::new(crate::WorldEnded));
		}

		match result {
			Ok(res) => {
				if let Ok(list) = res.as_list() {
					// The rest are the potentially mutated parameters. We need
					// to commit them to the function that
					// called us. TODO: This sucks, obviously.
					let len = list.len();
					for i in 2..=len {
						let value = list.get(i).unwrap();
						let slot = &args[i as usize - 2].2;

						unsafe {
							match slot {
								ArgType::Dot => {
									let dot = (*ctx).dot_mut();
									let _ = Value::from_raw_owned(*dot);
									*dot = value.raw;
								}
								ArgType::Usr => {
									let _ = Value::from_raw_owned((*instance).usr);
									(*instance).usr = value.raw;
								}
								ArgType::Src => {
									let _ = Value::from_raw_owned((*instance).src);
									(*instance).src = value.raw;
								}
								ArgType::Arg(idx) => {
									let args = (*instance).args();
									let arg = args.add(*idx as usize);
									let _ = Value::from_raw_owned(*arg);
									(*arg) = value.raw;
								}
								ArgType::Local(idx) => {
									let locals = *(*ctx).locals();
									let local = locals.add(*idx as usize);
									let _ = Value::from_raw_owned(*local);
									(*local) = value.raw;
								}
								ArgType::Given => continue
							}
						}

						std::mem::forget(value);
					}

					Ok(list.get(1).unwrap())
				} else {
					// The stub returns a list unless it's cut short, and a
					// runtime error is told apart above
					Err("the expression didn't finish: something it called went to sleep".to_owned())
				}
			}

			// A runtime error, which the world's runtime hook has the text of
			Err(Runtime { message }) => Err(format!("runtime error: {}", message))
		}
	}

	fn handle_eval(&mut self, frame_id: Option<u32>, command: &str) {
		if let Some(command) = command.strip_prefix('#') {
			let response = self.handle_command(frame_id, command);
			self.send_or_disconnect(Response::Eval(Ok(EvalResponse {
				value: response,
				variables: None,
				kind: None,
				entries: None
			})));
			return;
		}

		// What's inside the result is reached through the expression
		let response = self.eval_expr(frame_id, command).map(|result| EvalResponse {
			value: Self::stringify(&result),
			variables: self.value_to_variables_ref(&result, Some(format!("({})", command))),
			kind: Self::kind(&result),
			entries: Self::entries(&result)
		});
		self.send_or_disconnect(Response::Eval(response));
	}

	fn handle_hot_reload(&mut self, request: &HotReloadRequest) {
		let result = self.hot_reload(request);
		self.send_or_disconnect(Response::HotReload(result));
	}

	fn hot_reload(&mut self, request: &HotReloadRequest) -> HotReloadResult {
		// Assembling runs DM code to look up typepaths and resources, which
		// counts as eval code (see eval_expr)
		self.in_eval = true;
		self.eval_error = None;

		let mut warnings = vec![];
		// New types come first, so the rest can be prepared against them
		let prepared = hot_reload::add_types(&request.new_types).and_then(|added| {
			hot_reload::prepare(request, &mut warnings).map_err(|mut errors| {
				if added > 0 {
					errors.push(format!("the {} new types were added anyway; nothing uses them yet", added));
				}
				errors
			})
		});
		let result = match prepared {
			Ok(prepared) => {
				hot_reload::apply(prepared, &mut self.breakpoints, &mut self.moved_breakpoints, &mut warnings);
				HotReloadResult {
					applied: true,
					errors: vec![],
					warnings
				}
			}

			Err(errors) => HotReloadResult {
				applied: false,
				errors,
				warnings
			}
		};

		self.in_eval = false;
		if let Some(err) = self.eval_error.take() {
			self.notify(format!("runtime occured during a hot reload: {}", err));
		}

		// Every client hears what changed
		if result.applied && !request.is_empty() {
			self.broadcast(Response::CodeChanged, self.current);
			self.notify(format!(
				"Hot reload: new code for {} procs, new settings for {}, {} new procs, {} unlinked, {} new vars, {} new var defaults, {} new init \
				 procs, {} moved",
				request.procs.len(),
				request.settings.len(),
				request.new_procs.len(),
				request.removed_procs.len(),
				request.new_vars.len(),
				request.var_defaults.len(),
				request.init_procs.len(),
				request.line_moves.len()
			));
		}

		result
	}

	fn handle_disassemble(&mut self, path: &str, id: u32) -> String {
		match auxtools::Proc::find_override(path, id) {
			Some(proc) => {
				// The live bytecode has our breakpoints patched into it, which
				// nobody wants to read
				let bytecode = original_bytecode(&proc);

				let mut env = crate::DisassembleEnv;
				let (nodes, error) = dmasm::disassembler::disassemble(&bytecode, &mut env);
				let dism = dmasm::format_disassembly(&nodes, None);

				match error {
					Some(error) => {
						format!("Dism for {:?}\n{}\n\tError: {:?}", proc, dism, error)
					}

					None => {
						format!("Dism for {:?}\n{}", proc, dism)
					}
				}
			}

			None => "Proc not found".to_owned()
		}
	}

	// returns true if we need to break
	fn handle_request(&mut self, request: Request) -> bool {
		match request {
			Request::Disconnect => unreachable!(),
			Request::CatchRuntimes { uncaught, caught } => {
				if let Some(client) = self.current.and_then(|id| self.clients.get_mut(&id)) {
					client.uncaught = uncaught;
					client.caught = caught;
				}
			}
			Request::BreakpointSet {
				instruction,
				condition,
				hit,
				log
			} => self.handle_breakpoint_set(instruction, condition, hit, log),
			Request::BreakpointUnset { instruction } => self.handle_breakpoint_unset(instruction),
			Request::Stacks { sleeping } => self.handle_stacks(sleeping),
			Request::Scopes { frame_id } => self.handle_scopes(frame_id),
			Request::Variables { vars, start, count } => self.handle_variables(vars, start, count),
			Request::Complete { frame_id, base, prefix } => {
				let names = self.complete(frame_id, base.as_deref(), &prefix);
				self.send_or_disconnect(Response::Complete(names));
			}
			Request::SetVariable { vars, name, value } => {
				let result = self.set_variable(vars, &name, &value);
				self.send_or_disconnect(Response::SetVariable(result));
			}
			Request::Eval { frame_id, command, .. } => self.handle_eval(frame_id, &command),

			Request::StackFrames {
				stack_id,
				start_frame,
				count
			} => self.handle_stack_frames(stack_id, start_frame, count),

			Request::Offset { proc, line } => {
				self.send_or_disconnect(Response::Offset {
					offset: self.get_offset(proc, line)
				});
			}

			Request::Pause => {
				self.send_or_disconnect(Response::Ack);
				return true;
			}

			Request::ByondVersion => {
				let (version, build) = auxtools::version::get();
				self.send_or_disconnect(Response::ByondVersion { version, build });
			}

			Request::HotReload(request) => self.handle_hot_reload(&request),

			Request::Status => {
				let paused = self.paused.clone();
				self.send_or_disconnect(Response::Status { paused });
			}

			Request::HoldPause => {
				self.hold = self.current;
				self.send_or_disconnect(Response::Ack);
			}

			Request::Locate { file, line } => {
				let instructions = self.locate(&file, line);
				self.send_or_disconnect(Response::Locate { instructions });
			}

			Request::LinesWithCode { file, start, end } => {
				let lines = self.lines_with_code(&file, start, end);
				self.send_or_disconnect(Response::LinesWithCode { lines });
			}

			Request::Files => {
				let mut files: Vec<String> = self.file_index().keys().cloned().collect();
				files.sort();
				self.send_or_disconnect(Response::Files { files });
			}

			Request::WorldInfo => {
				let (args, cwd) = crate::launched::command_line();
				self.send_or_disconnect(Response::WorldInfo { args, cwd });
			}

			Request::StdDef => {
				let stddef = crate::stddef::get_stddef().map(|x| x.to_string());
				self.send_or_disconnect(Response::StdDef(stddef));
			}

			Request::Watcher => {
				if let Some(client) = self.current.and_then(|id| self.clients.get_mut(&id)) {
					client.debugs = false;
				}
				self.send_or_disconnect(Response::Ack);
			}

			// The following requests are special cases and handled outside of
			// this function
			Request::Configured | Request::Continue { .. } => {
				self.send_or_disconnect(Response::Ack);
			}
		}

		false
	}

	/// Applies whatever the networking threads have reported and hands back the
	/// next request, with `current` set to the client that sent it. `None`
	/// means nothing came in time: for `Wait::WhileClients`, that the last
	/// client left.
	fn recv_request(&mut self, wait: Wait) -> Option<Request> {
		loop {
			let event = if wait == Wait::No {
				self.events.try_recv().ok()?
			} else {
				self.events.recv().ok()?
			};

			match event {
				Event::Connected(id, connection) => {
					self.clients.insert(id, Client {
						connection,
						uncaught: None,
						caught: None,
						debugs: true
					});
				}

				Event::Disconnected(id) => {
					self.remove_client(id);

					if wait == Wait::WhileClients && !self.has_debuggers() {
						return None;
					}
				}

				// Sent by a client we've since given up on
				Event::Request(id, _) if !self.clients.contains_key(&id) => waker::request_taken(),

				Event::Request(id, request) => {
					waker::request_taken();
					self.current = Some(id);
					return Some(request);
				}
			}
		}
	}

	/// Whether a client that debugs is connected
	fn has_debuggers(&self) -> bool {
		self.clients.values().any(|client| client.debugs)
	}

	/// Tells every client, or the world's own output when there's nobody to
	/// tell (a launcher passes that on too, so it's one or the other)
	pub fn notify<T: Into<String>>(&mut self, message: T) {
		let message = message.into();
		if self.clients.is_empty() {
			eprintln!("Debug Server: {}", message);
		}

		self.broadcast(Response::Notification { message }, None);
	}

	/// Something in here panicked. We don't know what state that left the
	/// session in, so the debug-client gets dropped and can connect again.
	pub fn recover_from_panic(&mut self) {
		eprintln!("Debug server panicked, dropping the debug clients");
		self.in_eval = false;
		// A panic mid-pause skips the cleanup at the end of handle_breakpoint.
		// The frames in here point at procs that are about to carry on running.
		self.state = None;
		self.disconnect_all();
	}

	/// Pauses the world, if anyone wants it paused for this, until a client
	/// says how it should carry on. `None` when it didn't pause: whatever
	/// the world was doing (a step, say) goes on.
	pub fn handle_breakpoint(&mut self, _ctx: *mut raw_types::procs::ExecutionContext, reason: BreakpointReason) -> Option<ContinueKind> {
		// Ignore all breakpoints unless someone's connected, and runtime errors
		// unless someone wants them
		let wanted = match &reason {
			BreakpointReason::Runtime { message, caught } => self.clients.values().any(|client| {
				let filter = if *caught { &client.caught } else { &client.uncaught };
				filter.as_ref().is_some_and(|filter| filter.takes(message))
			}),
			_ => self.has_debuggers() || self.hold.is_some()
		};
		if !wanted {
			return None;
		}

		self.state = Some(State::new());

		// A breakpoint stops the world when a client that set it wants this
		// hit: its condition holds, if it has one, and the hit is one of
		// those it asked for. Each client's is looked at, so each one's
		// count of hits is its own.
		if reason == BreakpointReason::Breakpoint {
			let proc = unsafe { (*(*(*_ctx).proc_instance())).proc };
			let offset = unsafe { *(*_ctx).bytecode_offset() };
			let key = (proc, offset);
			let clients: Vec<(ClientId, bool)> = self.breakpoints.get(&key).map(|claims| claims.keys().copied().collect()).unwrap_or_default();

			// One nobody is known to have set stops, as it always has
			let mut pause = clients.is_empty();
			for client in clients {
				let Some(condition) = self.breakpoints.get(&key).and_then(|claims| claims.get(&client)).map(|claim| claim.condition.clone()) else {
					continue;
				};
				if let Some(condition) = condition {
					// One that can't be worked out pauses: better a stop too
					// many than one missed
					let passed = match self.eval_expr(Some(0), &condition) {
						Ok(result) => result.is_truthy(),
						Err(error) => {
							self.notify(format!("A breakpoint's condition ({}) failed, so the world pauses there: {}", condition, error));
							true
						}
					};

					// We might have just executed some code so invalidate the
					// stacks we already fetched
					self.state.as_mut().unwrap().invalidate_stacks();
					if !passed {
						continue;
					}
				}

				// The condition's code could have taken the breakpoint away
				let Some(claim) = self.breakpoints.get_mut(&key).and_then(|claims| claims.get_mut(&client)) else {
					continue;
				};
				claim.hits += 1;
				if claim.hit.is_some_and(|hit| !hit.takes(claim.hits)) {
					continue;
				}

				// A breakpoint with a message says it to its client, and the
				// world carries on
				let Some(log) = claim.log.clone() else {
					pause = true;
					continue;
				};
				let message = self.log_message(&log);
				let logged = Proc::from_id(proc).map(|proc| {
					let place = ProcRef {
						path: proc.path.clone(),
						override_id: proc.override_id()
					};
					Response::Log {
						message,
						file: Self::file_of(&proc),
						line: self.get_line_number(place, offset as u32)
					}
				});
				if let (Some(logged), Some(to)) = (logged, self.clients.get_mut(&client.0)) {
					if Self::send(&mut to.connection, &logged).is_err() {
						self.remove_client(client.0);
					}
				}
			}

			if !pause {
				self.state = None;
				return None;
			}
		}

		let at = unsafe {
			let proc = Proc::from_id((*(*(*_ctx).proc_instance())).proc);
			InstructionRef {
				proc: ProcRef {
					path: proc.as_ref().map(|proc| proc.path.clone()).unwrap_or_default(),
					override_id: proc.as_ref().map_or(0, Proc::override_id)
				},
				offset: u32::from(*(*_ctx).bytecode_offset())
			}
		};
		let stop = Stop {
			reason,
			thread: self.thread_of(self.state.as_ref().unwrap().active()),
			at
		};
		self.paused = Some(stop.clone());
		self.broadcast(Response::BreakpointHit(stop), None);

		loop {
			// Held, it waits for clients to come back
			let wait = if self.hold.is_some() { Wait::Forever } else { Wait::WhileClients };
			if wait == Wait::WhileClients && !self.has_debuggers() {
				break;
			}
			let Some(request) = self.recv_request(wait) else {
				break;
			};

			// Hijack and handle any Continue requests. The other clients are
			// told the world carries on.
			if let Request::Continue { kind } = request {
				// A step is in a stack, which the client names by its id
				let position = |thread: u32| self.stack_position(thread).unwrap_or(0) as u32;
				let kind = match kind {
					ContinueKind::Continue => ContinueKind::Continue,
					ContinueKind::StepOver { stack_id } => ContinueKind::StepOver { stack_id: position(stack_id) },
					ContinueKind::StepInto { stack_id } => ContinueKind::StepInto { stack_id: position(stack_id) },
					ContinueKind::StepOut { stack_id } => ContinueKind::StepOut { stack_id: position(stack_id) }
				};
				self.send_or_disconnect(Response::Ack);
				self.broadcast(Response::Resumed, self.current);
				self.state = None;
				self.paused = None;
				self.hold = None;
				return Some(kind);
			}

			// Hijack eval too so that we can refresh our state after it
			if let Request::Eval { frame_id, command, .. } = request {
				self.handle_eval(frame_id, &command);
				self.state.as_mut().unwrap().invalidate_stacks();
				continue;
			}

			// if we get a pause request here we can ignore it
			self.handle_request(request);
		}

		// Every client that debugs left. The others hear the world carries on.
		self.broadcast(Response::Resumed, None);
		self.state = None;
		self.paused = None;
		Some(ContinueKind::Continue)
	}

	// returns true if we need to pause
	pub fn process(&mut self) -> bool {
		let mut should_pause = false;

		waker::alive();
		if waker::woke() {
			self.notify("The world was asleep with no players connected (world.sleep_offline): auxtools woke it, as debuggers count as players");
		}

		while let Some(request) = self.recv_request(Wait::No) {
			// Not `||`, that would skip every request after a Pause
			should_pause |= self.handle_request(request);
		}

		should_pause
	}

	/// Block while processing all received requests normally until a debug
	/// client says it's configured, however long that takes to connect
	pub fn process_until_configured(&mut self) {
		while let Some(request) = self.recv_request(Wait::Forever) {
			if let Request::Configured = request {
				self.send_or_disconnect(Response::Ack);
				break;
			}

			self.handle_request(request);
		}
	}

	/// Answers the client whose request this is
	fn send_or_disconnect(&mut self, response: Response) {
		// Nobody to tell: the client left while we were busy
		let Some(id) = self.current else {
			return;
		};
		let Some(client) = self.clients.get_mut(&id) else {
			return;
		};

		if let Err(e) = Self::send(&mut client.connection, &response) {
			eprintln!("Debug server failed to send message: {}", e);
			self.remove_client(id);
		}
	}

	/// Tells every client but `except`
	fn broadcast(&mut self, response: Response, except: Option<ClientId>) {
		let mut gone = vec![];
		for (&id, client) in &mut self.clients {
			if Some(id) != except && Self::send(&mut client.connection, &response).is_err() {
				gone.push(id);
			}
		}
		for id in gone {
			self.remove_client(id);
		}
	}

	/// Says goodbye to a client and takes away its breakpoints
	fn remove_client(&mut self, id: ClientId) {
		let Some(mut client) = self.clients.remove(&id) else {
			return;
		};
		let _ = Self::send(&mut client.connection, &Response::Disconnect);
		if self.current == Some(id) {
			self.current = None;
		}
		// A hold on a pause that hasn't come yet goes with its client
		if self.hold == Some(id) && self.paused.is_none() {
			self.hold = None;
		}

		let mut unclaimed = vec![];
		for (key, claims) in &mut self.breakpoints {
			claims.retain(|(client, _), _| *client != id);
			if claims.is_empty() {
				unclaimed.push(*key);
			}
		}
		for (proc, offset) in unclaimed {
			self.breakpoints.remove(&(proc, offset));
			if let Some(proc) = Proc::from_id(proc) {
				let _ = unhook_instruction(&proc, offset as u32);
			}
		}

		// Whoever connects next sets up their own breakpoints. Until then the
		// old ones would only slow down the procs they sit in.
		if self.clients.is_empty() {
			self.breakpoints.clear();
			self.moved_breakpoints.clear();
			unhook_all();
		}
	}

	fn disconnect_all(&mut self) {
		let ids: Vec<ClientId> = self.clients.keys().copied().collect();
		for id in ids {
			self.remove_client(id);
		}
	}

	pub(crate) fn send(stream: &mut impl Write, response: &Response) -> Result<(), Box<dyn Error>> {
		let data = bincode::serialize(response)?;
		stream.write_all(&(data.len() as u32).to_le_bytes())?;
		stream.write_all(&data[..])?;
		stream.flush()?;
		Ok(())
	}
}

impl Drop for Server {
	fn drop(&mut self) {
		self.disconnect_all();

		// Clients from now on wait for the next Server
		if let Some(acceptor) = ACCEPTOR.lock().unwrap().as_mut() {
			let (events, waiting) = mpsc::channel();
			acceptor.events = events;
			acceptor.waiting = Some(waiting);
		}

		// Clients that connected before that, which we never got told about,
		// are told to go
		while let Ok(event) = self.events.try_recv() {
			if let Event::Connected(_, mut connection) = event {
				let _ = Self::send(&mut connection, &Response::Disconnect);
			}
		}
	}
}

/// Reads one request off a connection: a u32 length, then the request in
/// bincode.
fn read_request(stream: &mut impl Read, buf: &mut Vec<u8>) -> Result<Request, Box<dyn Error>> {
	let mut len_bytes = [0u8; 4];
	stream.read_exact(&mut len_bytes)?;
	let len = u32::from_le_bytes(len_bytes);

	if len > MAX_REQUEST_LEN {
		return Err(format!("a {} byte request is too big to be real", len).into());
	}

	buf.resize(len as usize, 0);
	stream.read_exact(buf)?;
	Ok(bincode::deserialize::<Request>(buf)?)
}

impl ServerThread {
	fn run(&self, id: ClientId, mut stream: Connection) {
		let mut buf = vec![];
		waker::client_connected();

		// Until it says goodbye or hangs up, which clients come and go by, so
		// neither is worth a line in the world's log
		while let Ok(request) = read_request(&mut stream, &mut buf) {
			if let Request::Disconnect = request {
				break;
			}

			waker::request_waiting();
			if let Err(e) = self.events.send(Event::Request(id, request)) {
				eprintln!("Debug server thread failed to handle request: {}", e);
				break;
			}
		}

		waker::client_left();
		let _ = self.events.send(Event::Disconnected(id));
	}
}

#[cfg(test)]
mod tests {
	use std::{io::Read, sync::mpsc, thread, time::Duration};

	use super::{Connection, Server};
	use crate::connection;

	#[test]
	fn drop_hangs_up_on_clients_and_the_next_server_takes_new_ones() {
		let (done, dropped) = mpsc::channel();

		thread::spawn(move || {
			let mut server = Server::listen().unwrap();
			let first = Connection::connect(&connection::world_name()).unwrap();
			let second = Connection::connect(&connection::world_name()).unwrap();
			thread::sleep(Duration::from_millis(200));
			// Both are taken in
			assert!(server.recv_request(super::Wait::No).is_none());
			assert_eq!(server.clients.len(), 2);

			drop(server);
			let _ = done.send((first, second));
		});

		let (mut first, mut second) = dropped
			.recv_timeout(Duration::from_secs(5))
			.expect("dropping the server hung on its clients");
		// Each was told to go: a Disconnect, then the end of the connection
		for client in [&mut first, &mut second] {
			let mut len = [0u8; 4];
			client.read_exact(&mut len).expect("no Disconnect");
		}

		// A client that connects while the world reboots waits for the next
		// server
		let _third = Connection::connect(&connection::world_name()).expect("nothing listens between servers");
		thread::sleep(Duration::from_millis(200));
		let mut server = Server::listen().expect("the next server couldn't listen");
		assert!(server.recv_request(super::Wait::No).is_none());
		assert_eq!(server.clients.len(), 1);
		assert!(Server::listen().is_err(), "two servers at once");
	}
}
