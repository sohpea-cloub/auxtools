use std::{
	cell::{OnceCell, RefCell},
	collections::{BTreeMap, HashMap, VecDeque},
	io::Write,
	sync::{
		atomic::{AtomicU64, Ordering},
		mpsc, Mutex
	},
	thread,
	time::{Duration, Instant}
};

use auxtools::{
	raw_types::values::{ValueData, ValueTag},
	*
};
use auxtools_protocol::*;
use instruction_hooking::disassemble_env;

use super::instruction_hooking::{hook_instruction, original_bytecode, unhook_all, unhook_instruction};
use crate::{
	connection::{self, Connection},
	console, hot_reload, journal, resource_reload, unnamed, waker
};

mod profile;
mod render;
mod watches;

static NEXT_SPAWN: AtomicU64 = AtomicU64::new(1);

struct Spawn {
	number: u64,
	bytecode: Vec<u32>,
	locals: Vec<String>,
	args: Vec<Value>
}

#[derive(Clone, Hash, PartialEq, Eq)]
enum Variables {
	Arguments {
		frame: u32
	},
	Locals {
		frame: u32
	},
	Globals,
	Builtins,
	ObjectVars(Value, Option<String>),
	ListContents(Value, Option<String>),
	ListPair {
		key: Value,
		value: Value,
		of: Value,
		entry: Option<String>
	},
	Pointee(Value, Option<String>)
}

struct State {
	active: OnceCell<Vec<debug::StackFrame>>,
	suspended: OnceCell<Vec<Vec<debug::StackFrame>>>,
	sleeping: usize,
	variables: RefCell<Vec<Variables>>,
	variables_to_refs: RefCell<HashMap<Variables, VariablesRef>>,
	sleeping_frames: RefCell<Vec<SleepingFrame>>,
	sleeping_frame_ids: RefCell<HashMap<SleepingFrame, u32>>
}

const SLEEPING_FRAMES: u32 = 1 << 30;

const ENDING_LIMIT: Duration = Duration::from_secs(60);

const EVAL_ABORTED: &str = "evaluation aborted";

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
struct SleepingFrame {
	thread: u32,
	depth: usize,
	instance: usize
}

impl State {
	fn new() -> Self {
		Self {
			active: OnceCell::new(),
			suspended: OnceCell::new(),
			sleeping: DEFAULT_SLEEPING_STACKS as usize,
			variables: RefCell::new(vec![]),
			variables_to_refs: RefCell::new(HashMap::new()),
			sleeping_frames: RefCell::new(vec![]),
			sleeping_frame_ids: RefCell::new(HashMap::new())
		}
	}

	fn sleeping_frame_id(&self, frame: SleepingFrame) -> u32 {
		let mut frames = self.sleeping_frames.borrow_mut();
		*self.sleeping_frame_ids.borrow_mut().entry(frame).or_insert_with(|| {
			frames.push(frame);
			SLEEPING_FRAMES + (frames.len() as u32 - 1)
		})
	}

	fn sleeping_frame(&self, id: u32) -> Option<SleepingFrame> {
		self.sleeping_frames.borrow().get(id.checked_sub(SLEEPING_FRAMES)? as usize).copied()
	}

	fn invalidate_stacks(&mut self) {
		self.active = OnceCell::new();
		self.suspended = OnceCell::new();
	}

	fn active(&self) -> &Vec<debug::StackFrame> {
		self.active.get_or_init(debug::CallStacks::active)
	}

	fn suspended(&self) -> &Vec<Vec<debug::StackFrame>> {
		self.suspended.get_or_init(|| debug::CallStacks::suspended_first(self.sleeping))
	}

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
		let index = usize::try_from(reference.0).ok()?.checked_sub(1)?;
		variables.get(index).cloned()
	}
}

type ClientId = u32;

enum Mark {
	File(String),
	Line(u32),
	Other
}

enum Event {
	Connected(ClientId, Connection),

	Request(ClientId, RequestId, Request),

	Disconnected(ClientId)
}

struct Client {
	uncaught: Option<RuntimeFilter>,
	caught: Option<RuntimeFilter>,
	debugs: bool,
	follow: Option<Vec<JournalKind>>,
	outbox: mpsc::SyncSender<Vec<u8>>,
	writer: Option<thread::JoinHandle<()>>,
	hold_max: Option<Duration>
}

const OUTBOX: usize = 1024;

const JOURNAL_FRAMES: usize = 64;

pub struct Claim {
	condition: Option<String>,
	hit: Option<HitCondition>,
	hits: u32,
	log: Option<Vec<LogPart>>,
	skipped: u32,
	told: bool
}

pub type Breakpoints = HashMap<(raw_types::procs::ProcId, u16), BTreeMap<(ClientId, Placed), Claim>>;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Placed {
	Line,
	Proc,
	Instruction
}

impl Placed {
	fn of(offset: u32, exactly: bool) -> Self {
		match offset {
			_ if exactly => Self::Instruction,
			0 => Self::Proc,
			_ => Self::Line
		}
	}
}

const GLOBALS: &str = "global vars";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Wait {
	No,
	WhileClients,
	Forever,
	Until(Option<Instant>)
}

struct Acceptor {
	events: mpsc::Sender<Event>,
	waiting: Option<mpsc::Receiver<Event>>
}

static ACCEPTOR: Mutex<Option<Acceptor>> = Mutex::new(None);

const EVAL_CODE_KEPT: usize = 256;

#[derive(PartialEq, Eq, Hash)]
struct EvalCodeKey {
	generation: u32,
	command: String,
	names: Vec<String>,
	src_type: Option<String>
}

pub struct Server {
	events: mpsc::Receiver<Event>,
	clients: BTreeMap<ClientId, Client>,
	current: Option<ClientId>,
	answering: Option<RequestId>,
	state: Option<State>,
	in_eval: bool,
	eval_error: Option<String>,
	eval_callee_errors: Vec<String>,
	eval_callee_more: usize,
	reloader: Option<(ClientId, String)>,
	profiling: Option<ClientId>,
	eval_ran: *mut raw_types::procs::ExecutionContext,
	breakpoints: Breakpoints,
	paused: Option<Stop>,
	watches: Vec<watches::Watched>,
	next_watch: u32,
	eval_given: Vec<(String, Value)>,
	eval_code: HashMap<EvalCodeKey, Result<(Vec<u32>, Vec<String>, Vec<String>), String>>,
	eval_warnings: Vec<String>,
	waiting_to_start: bool,
	pending_skin: Option<SkinRequest>,
	pending_script: Option<ScriptRequest>,
	resource_ids: HashMap<String, Vec<u32>>,
	resource_origins: HashMap<String, Vec<u32>>,
	eval_src: Option<Value>,
	spawning: Option<u64>,
	spawns: VecDeque<Spawn>,
	spawn_inbox: Option<Value>,
	spawn_stub_locals: Option<u32>,
	spawned: HashMap<u64, (String, Option<String>)>,
	ending: Option<Instant>,
	hold: Option<ClientId>,
	hold_max: Option<Duration>,
	stepper: Option<ClientId>,
	procs_by_file: Option<(u32, HashMap<String, Vec<u32>>)>,
	moved_breakpoints: hot_reload::MovedBreakpoints<(ClientId, Placed)>,
	old_codes: RefCell<Vec<usize>>,
	waiting_reloads: Vec<HotReloadRequest>
}

struct ServerThread {
	events: mpsc::Sender<Event>
}

impl Server {
	const MOST_COMPLETIONS: usize = 200;

	pub fn listen() -> std::io::Result<Server> {
		let events = Self::take_events()?;
		Ok(Server {
			events,
			clients: BTreeMap::new(),
			current: None,
			answering: None,
			state: None,
			in_eval: false,
			eval_error: None,
			eval_callee_errors: vec![],
			eval_callee_more: 0,
			reloader: None,
			profiling: None,
			eval_ran: std::ptr::null_mut(),
			breakpoints: HashMap::new(),
			moved_breakpoints: HashMap::new(),
			paused: None,
			watches: vec![],
			next_watch: 0,
			eval_given: vec![],
			eval_code: HashMap::new(),
			eval_warnings: vec![],
			waiting_to_start: false,
			pending_skin: None,
			pending_script: None,
			resource_ids: HashMap::new(),
			resource_origins: HashMap::new(),
			eval_src: None,
			spawning: None,
			spawns: VecDeque::new(),
			spawn_inbox: None,
			spawn_stub_locals: None,
			spawned: HashMap::new(),
			ending: None,
			hold: None,
			hold_max: None,
			stepper: None,
			procs_by_file: None,
			old_codes: RefCell::new(vec![]),
			waiting_reloads: vec![]
		})
	}

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

	fn accept(listener: &connection::Listener) {
		let mut next_id = 0;
		loop {
			let Ok(connection) = listener.accept() else {
				continue;
			};
			next_id += 1;

			let events = {
				let acceptor = ACCEPTOR.lock().unwrap();
				let Some(acceptor) = acceptor.as_ref() else {
					return;
				};
				let _ = acceptor.events.send(Event::Connected(next_id, connection.clone()));
				waker::event_sent();
				acceptor.events.clone()
			};

			let server_thread = ServerThread { events };
			let id = next_id;
			thread::spawn(move || server_thread.run(id, connection));
		}
	}

	pub fn refuse(reason: String) {
		if let Ok(events) = Self::take_events() {
			Self::refuse_on(events, reason);
		}
	}

	pub fn give_up(&mut self, reason: String) {
		self.clients.clear();
		self.hold = None;
		self.hold_max = None;
		let (_, nothing) = mpsc::channel();
		Self::refuse_on(std::mem::replace(&mut self.events, nothing), reason);
	}

	fn refuse_on(events: mpsc::Receiver<Event>, reason: String) {
		thread::spawn(move || {
			while let Ok(event) = events.recv() {
				if let Event::Connected(_, mut connection) = event {
					for response in [Response::Unavailable { reason: reason.clone() }, Response::Disconnect] {
						if Self::say(&mut connection, &response).is_err() {
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

	pub fn eval_runs(&mut self, ctx: *mut raw_types::procs::ExecutionContext) {
		self.eval_ran = ctx;
	}

	pub fn set_eval_error(&mut self, message: &str, ctx: *mut raw_types::procs::ExecutionContext) {
		if !Self::called_by_expression(ctx) {
			self.eval_error = Some(Self::error_with_stack(message, ctx));
		} else if self.eval_callee_errors.len() < MAX_CALLEE_ERRORS {
			let error = Self::error_with_stack(message, ctx);
			self.eval_callee_errors.push(error);
		} else {
			self.eval_callee_more += 1;
		}
	}

	fn called_by_expression(ctx: *mut raw_types::procs::ExecutionContext) -> bool {
		if ctx.is_null() || is_expression_frame(ctx) {
			return false;
		}
		let mut at = unsafe { *(*ctx).parent_context() };
		while !at.is_null() {
			if is_expression_frame(at) {
				return true;
			}
			at = unsafe { *(*at).parent_context() };
		}
		false
	}

	fn send_callee_errors(&mut self) {
		let mut notes: Vec<String> = std::mem::take(&mut self.eval_warnings)
			.into_iter()
			.map(|warning| format!("warning: {}", warning))
			.collect();
		let errors = std::mem::take(&mut self.eval_callee_errors);
		let more = std::mem::take(&mut self.eval_callee_more);
		notes.extend(
			errors
				.into_iter()
				.map(|error| format!("runtime error in a called proc (the evaluation carried on): {}", error))
		);
		if more > 0 {
			notes.push(format!("and {} more runtime errors in called procs", more));
		}
		let Some(client) = self.current else {
			return;
		};
		for message in notes {
			self.tell(client, Response::Notification { message });
		}
	}

	fn error_with_stack(message: &str, ctx: *mut raw_types::procs::ExecutionContext) -> String {
		let mut error = message.to_owned();
		let mut frames = vec![];
		let mut at = ctx;
		while !at.is_null() && !is_expression_frame(at) {
			frames.push(Self::context_place(at));
			at = unsafe { *(*at).parent_context() };
		}
		if !at.is_null() {
			for (i, place) in frames.iter().enumerate() {
				error += &format!("\n  {} {}", if i == 0 { "in" } else { "called by" }, place);
			}
			if !frames.is_empty() {
				error += "\n  called by the expression";
			}
		}
		error
	}

	pub fn spawn_runtime(&mut self, message: &str, ctx: *mut raw_types::procs::ExecutionContext) {
		if self.spawned.is_empty() {
			return;
		}
		let (Some(stub), Some(spawned)) = (Proc::find("/proc/auxtools_spawn_stub"), Proc::find("/proc/auxtools_spawned")) else {
			return;
		};
		let proc_of = |at: *mut raw_types::procs::ExecutionContext| unsafe { (**(*at).proc_instance()).proc };
		let mut at = ctx;
		let mut in_stub = false;
		while !at.is_null() {
			let id = proc_of(at);
			if id == stub.id {
				if unsafe { *(*at).throw_handler() } != 0 {
					return;
				}
				if at != ctx {
					return;
				}
				in_stub = true;
			} else if in_stub && id == spawned.id {
				let number = unsafe { Value::from_raw(*(**(*at).proc_instance()).args()) }.as_number().ok();
				if let Some((_, error @ None)) = number.and_then(|number| self.spawned.get_mut(&(number as u64))) {
					*error = Some(format!("runtime error: {}", Self::error_with_stack(message, ctx)));
				}
				return;
			} else if !in_stub && unsafe { *(*at).throw_handler() } != 0 {
				return;
			}
			at = unsafe { *(*at).parent_context() };
		}
	}

	fn context_place(ctx: *mut raw_types::procs::ExecutionContext) -> String {
		let proc = Proc::from_id(unsafe { (**(*ctx).proc_instance()).proc });
		let mut place = match &proc {
			Some(proc) => unnamed::proc_name(proc),
			None => "a proc the world doesn't have".to_owned()
		};
		let line = unsafe { (*ctx).filename().valid().then(|| *(*ctx).line()) };
		match (proc.as_ref().and_then(Self::file_of), line) {
			(Some(file), Some(line)) => place += &format!("  {}:{}", file, line),
			(None, Some(line)) => place += &format!("  line {}", line),
			_ => {}
		}
		place
	}

	pub(crate) fn visible_locals(frame: &debug::StackFrame) -> Vec<Option<String>> {
		let names: Vec<String> = frame.locals.iter().map(|(name, _)| String::from(name)).collect();
		let shared = |name: &String| names.iter().filter(|other| *other == name).count() > 1;
		if !names.iter().any(shared) {
			return names.into_iter().map(Some).collect();
		}
		let code = Self::old_code_of(frame).unwrap_or_else(|| original_bytecode(&frame.proc));
		let used = local_uses(&code);
		let at = frame.offset as u32;
		let rank = |slot: usize| {
			used.get(&(slot as u32))
				.filter(|(first, _)| *first <= at)
				.map(|(first, last)| (at <= *last, *first))
		};
		names
			.iter()
			.enumerate()
			.map(|(slot, name)| {
				if !shared(name) {
					return Some(name.clone());
				}
				let mine = rank(slot)?;
				let meant = names
					.iter()
					.enumerate()
					.filter(|(other, other_name)| *other != slot && *other_name == name)
					.filter_map(|(other, _)| rank(other))
					.all(|theirs| theirs < mine);
				meant.then(|| name.clone())
			})
			.collect()
	}

	fn old_code_of(frame: &debug::StackFrame) -> Option<Vec<u32>> {
		Self::old_code_running(&frame.proc, frame.context)
	}

	fn old_code_running(proc: &Proc, context: *mut raw_types::procs::ExecutionContext) -> Option<Vec<u32>> {
		let running = unsafe { *(*context).bytecode() };
		let (current, _) = unsafe { proc.bytecode_mut_ptr() };
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
		let now = original_bytecode(proc);
		if old.len() == now.len() && instructions(&old) == instructions(&now) {
			return None;
		}
		Some(old)
	}

	fn code_of(&self, frame: &debug::StackFrame, old: bool) -> CodeRef {
		let mut code = CodeRef {
			proc: frame.proc.id.0,
			old: 0
		};
		if old {
			code.old = self.old_code_number(unsafe { *(*frame.context).bytecode() } as usize);
		}
		code
	}

	fn old_code_number(&self, running: usize) -> u16 {
		let mut old_codes = self.old_codes.borrow_mut();
		let number = match old_codes.iter().position(|&old| old == running) {
			Some(at) => at + 1,
			None => {
				old_codes.push(running);
				old_codes.len()
			}
		};
		u16::try_from(number).unwrap_or(u16::MAX)
	}

	fn disassemble(&self, code: CodeRef) -> Result<Disassembly, String> {
		let proc = Proc::from_id(raw_types::procs::ProcId(code.proc)).ok_or_else(|| format!("the world has no proc {:#x}", code.proc))?;
		let bytecode = if code.old == 0 {
			original_bytecode(&proc)
		} else {
			let running = self.old_codes.borrow().get(code.old as usize - 1).copied();
			let old = running.and_then(|running| Some((running, auxtools::bytecode_at(running as *const u32)?)));
			match old {
				Some((running, old)) => crate::instruction_hooking::unpatched(running as *const u32, old),
				None => return Err(format!("{} has no old code {}", proc.path, code.old))
			}
		};

		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, error) = dmasm::disassembler::disassemble(&bytecode, &mut env);
		let first_line = nodes.iter().find_map(|node| match node {
			dmasm::Node::Instruction(dmasm::Instruction::DbgLine(line), _) => Some(*line),
			_ => None
		});
		let mut line = first_line;
		let mut instructions = vec![];
		let mut read = 0;
		for node in &nodes {
			let dmasm::Node::Instruction(ins, debug) = node else {
				continue;
			};
			if let dmasm::Instruction::DbgLine(at) = ins {
				line = Some(*at);
			}
			instructions.push(BytecodeInstruction {
				offset: debug.offset as u16,
				words: debug.bytecode.to_vec(),
				text: ins.to_string(),
				line
			});
			read = debug.offset as usize + debug.bytecode.len();
		}
		for (at, word) in bytecode.iter().enumerate().skip(read) {
			instructions.push(BytecodeInstruction {
				offset: at as u16,
				words: vec![*word],
				text: String::new(),
				line: None
			});
		}

		Ok(Disassembly {
			proc: unnamed::proc_ref(&proc),
			file: Self::file_of(&proc),
			instructions,
			error: error.map(|error| format!("{:?}", error))
		})
	}

	fn proc_info(wanted: &ProcRef) -> Result<ProcInfo, String> {
		let definitions = (0..).take_while(|&n| unnamed::find(&wanted.path, n).is_some()).count() as u32;
		let proc = match unnamed::find(&wanted.path, wanted.override_id) {
			Some(proc) => proc,
			None if definitions == 0 => return Err(format!("the world has no proc {}", wanted.path)),
			None if definitions == 1 => return Err(format!("{} has one definition, #0: there's no #{}", wanted.path, wanted.override_id)),
			None => {
				return Err(format!(
					"{} has {} definitions, #0 to #{}: there's no #{}",
					wanted.path,
					definitions,
					definitions - 1,
					wanted.override_id
				))
			}
		};
		Ok(ProcInfo {
			proc: unnamed::proc_ref(&proc),
			code: CodeRef { proc: proc.id.0, old: 0 },
			file: Self::file_of(&proc),
			line: Self::lines_of(&proc).into_iter().next(),
			definitions,
			reloaded: hot_reload::reloaded(proc.id.0)
		})
	}

	fn frame_place(&self, frame: &debug::StackFrame, asleep: bool) -> (Option<String>, Option<u32>) {
		Self::place_running(&frame.proc, frame.context, frame.offset, asleep)
	}

	fn place_running(proc: &Proc, context: *mut raw_types::procs::ExecutionContext, offset: u16, asleep: bool) -> (Option<String>, Option<u32>) {
		let offset = if asleep { (offset as u32).saturating_sub(1) } else { offset as u32 };

		if let Some(bytecode) = Self::old_code_running(proc, context) {
			return match Self::place_in(&bytecode, offset, false) {
				Some((file, line)) => (file.or_else(|| Self::file_of(proc)), Some(line)),
				None => (Self::file_of(proc), None)
			};
		}
		Self::place_of(proc, offset)
	}

	fn frame_info(&self, frame: &debug::StackFrame, id: u32, asleep: bool) -> StackFrame {
		let old_code = Self::old_code_of(frame).is_some();
		let (file, line) = self.frame_place(frame, asleep);
		StackFrame {
			id,
			instruction: InstructionRef {
				proc: unnamed::proc_ref(&frame.proc),
				offset: frame.offset as u32
			},
			line,
			file,
			old_code,
			code: self.code_of(frame, old_code)
		}
	}

	fn file_of(proc: &Proc) -> Option<String> {
		let dbg_file = dmasm::Opcode::DbgFile.word();
		let bytecode = unsafe { proc.bytecode() };
		let id = match bytecode {
			[op, id, ..] if *op == dbg_file => *id,
			[_, ..] => match crate::instruction_hooking::original_bytecode(proc).as_slice() {
				[op, id, ..] if *op == dbg_file => *id,
				_ => return None
			},
			[] => return None
		};
		let name = unsafe { StringRef::from_id(raw_types::strings::StringId(id)) };
		Some(String::from_utf8_lossy(name.data()).into_owned())
	}

	fn file_index(&mut self) -> &HashMap<String, Vec<u32>> {
		let count = auxtools::runtime_tables::proc_count();
		if self.procs_by_file.as_ref().is_none_or(|(counted, _)| *counted != count) {
			let dbg_file = dmasm::Opcode::DbgFile.word();
			let mut by_file: HashMap<String, Vec<u32>> = HashMap::new();
			for id in 0..count {
				let Some(proc) = Proc::from_id(raw_types::procs::ProcId(id)) else {
					continue;
				};
				let Some(file) = Self::file_of(&proc) else { continue };
				if unsafe { proc.bytecode() }.iter().skip(2).any(|&word| word == dbg_file) {
					for other in Self::files_of(&proc) {
						if other != file {
							by_file.entry(other).or_default().push(id);
						}
					}
				}
				by_file.entry(file).or_default().push(id);
			}
			self.procs_by_file = Some((count, by_file));
		}
		&self.procs_by_file.as_ref().unwrap().1
	}

	fn files_of(proc: &Proc) -> Vec<String> {
		let bytecode = crate::instruction_hooking::original_bytecode(proc);
		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, _error) = dmasm::disassembler::disassemble(&bytecode, &mut env);
		let mut files: Vec<String> = vec![];
		for node in nodes {
			if let dmasm::Node::Instruction(dmasm::Instruction::DbgFile(name), _) = node {
				let name = String::from_utf8_lossy(&name.0).into_owned();
				if !files.contains(&name) {
					files.push(name);
				}
			}
		}
		files
	}

	fn normalized(path: &str) -> String {
		path.replace('\\', "/").to_ascii_lowercase()
	}

	fn procs_in(&mut self, file: &str) -> (Vec<String>, Vec<(ProcRef, Vec<u32>)>) {
		let normalize = Self::normalized;
		let wanted = normalize(file);
		let same = |file: &str| file == wanted;
		let alike = |file: &str| file.ends_with(&format!("/{}", wanted)) || wanted.ends_with(&format!("/{}", file));

		let index = self.file_index();
		let matching = |test: &dyn Fn(&str) -> bool| -> (Vec<String>, Vec<u32>) {
			let files: Vec<(&String, &Vec<u32>)> = index.iter().filter(|(file, _)| test(&normalize(file))).collect();
			let mut ids: Vec<u32> = files.iter().flat_map(|(_, ids)| ids.iter().copied()).collect();
			ids.sort_unstable();
			ids.dedup();
			(files.iter().map(|(file, _)| normalize(file)).collect(), ids)
		};
		let (mut files, mut ids) = matching(&same);
		if ids.is_empty() {
			(files, ids) = matching(&alike);
		}
		let procs = ids
			.into_iter()
			.filter_map(|id| Proc::from_id(raw_types::procs::ProcId(id)))
			.map(|proc| {
				let lines = Self::lines_in(&proc, &files);
				let proc = unnamed::proc_ref(&proc);
				(proc, lines)
			})
			.collect();
		(files, procs)
	}

	fn lines_with_code(&mut self, file: &str, start: u32, end: u32) -> Vec<u32> {
		let mut lines: Vec<u32> = self
			.procs_in(file)
			.1
			.into_iter()
			.flat_map(|(_, lines)| lines)
			.filter(|line| (start..=end).contains(line))
			.collect();
		lines.sort_unstable();
		lines.dedup();
		lines
	}

	fn locate(&mut self, file: &str, line: u32) -> Vec<InstructionRef> {
		let (files, procs) = self.procs_in(file);
		let Some(line) = procs.iter().flat_map(|(_, lines)| lines.iter().copied()).filter(|&l| l >= line).min() else {
			return vec![];
		};

		let mut found = vec![];
		for (proc, lines) in procs {
			if !lines.contains(&line) {
				continue;
			}
			if let Some(offset) = self.offset_of(proc.clone(), line, Some(&files)) {
				found.push(InstructionRef { proc, offset });
			}
		}
		found
	}

	fn lines_in(proc: &Proc, files: &[String]) -> Vec<u32> {
		let bytecode = crate::instruction_hooking::original_bytecode(proc);
		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, _error) = dmasm::disassembler::disassemble(&bytecode, &mut env);
		let mut inside = false;
		let mut lines = vec![];
		for node in nodes {
			match node {
				dmasm::Node::Instruction(dmasm::Instruction::DbgFile(name), _) => {
					inside = files.contains(&Self::normalized(&String::from_utf8_lossy(&name.0)));
				}
				dmasm::Node::Instruction(dmasm::Instruction::DbgLine(line), _) if inside => lines.push(line),
				_ => {}
			}
		}
		lines
	}

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

	fn place_in(bytecode: &[u32], offset: u32, before: bool) -> Option<(Option<String>, u32)> {
		let mut current_line_number = None;
		let mut current_file = None;
		let mut reached_offset = false;

		for (at, mark) in Self::marks(bytecode).iter() {
			if *at > offset && !reached_offset {
				reached_offset = true;
				if current_line_number.is_some() {
					break;
				}
			}

			match mark {
				Mark::File(name) if !((reached_offset || *at == offset) && current_line_number.is_some()) => {
					current_file = Some(name.clone());
				}
				Mark::Line(_) if before && *at == offset => {}
				Mark::Line(line) => {
					current_line_number = Some(*line);
					if reached_offset {
						break;
					}
				}
				_ => {}
			}

			if *at == offset {
				reached_offset = true;
				if current_line_number.is_some() {
					break;
				}
			}
		}

		if reached_offset {
			current_line_number.map(|line| (current_file, line))
		} else {
			None
		}
	}

	fn marks(bytecode: &[u32]) -> std::sync::Arc<[(u32, Mark)]> {
		use std::hash::{Hash, Hasher};
		type Marks = std::sync::Arc<[(u32, Mark)]>;
		static MARKS: std::sync::LazyLock<Mutex<HashMap<(u64, usize), Marks>>> = std::sync::LazyLock::new(Default::default);
		const KEEP: usize = 8192;

		let key = {
			let mut hasher = std::collections::hash_map::DefaultHasher::new();
			bytecode.hash(&mut hasher);
			(hasher.finish(), bytecode.len())
		};
		if let Some(marks) = MARKS.lock().unwrap().get(&key) {
			return marks.clone();
		}

		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, _error) = dmasm::disassembler::disassemble(bytecode, &mut env);
		let marks: Marks = nodes
			.into_iter()
			.filter_map(|node| match node {
				dmasm::Node::Instruction(ins, debug) => Some((debug.offset, match ins {
					dmasm::Instruction::DbgFile(name) => Mark::File(String::from_utf8_lossy(&name.0).into_owned()),
					dmasm::Instruction::DbgLine(line) => Mark::Line(line),
					_ => Mark::Other
				})),
				_ => None
			})
			.collect();

		let mut cache = MARKS.lock().unwrap();
		if cache.len() >= KEEP {
			cache.clear();
		}
		cache.insert(key, marks.clone());
		marks
	}

	fn place_of(proc: &Proc, offset: u32) -> (Option<String>, Option<u32>) {
		let original = original_bytecode(proc);
		let here = offset as usize;
		let breakpoint_here = unsafe { proc.bytecode() }.get(here) != original.get(here);
		Self::place_at(proc, &original, offset, breakpoint_here)
	}

	fn breakpoint_place(proc: &Proc, offset: u32) -> (Option<String>, Option<u32>) {
		Self::place_at(proc, &original_bytecode(proc), offset, true)
	}

	fn place_at(proc: &Proc, original: &[u32], offset: u32, before: bool) -> (Option<String>, Option<u32>) {
		match Self::place_in(original, offset, before) {
			Some((file, line)) => (file.or_else(|| Self::file_of(proc)), Some(line)),
			None => (Self::file_of(proc), None)
		}
	}

	fn get_offset(&self, proc: ProcRef, line: u32) -> Option<u32> {
		self.offset_of(proc, line, None)
	}

	fn offset_of(&self, proc: ProcRef, line: u32, files: Option<&[String]>) -> Option<u32> {
		let proc = unnamed::find(&proc.path, proc.override_id)?;
		let mut offset = None;
		let mut at_offset = false;
		let mut other_file = false;
		let mut inside = files.is_none();

		let bytecode = original_bytecode(&proc);

		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, _error) = dmasm::disassembler::disassemble(&bytecode, &mut env);

		for node in nodes {
			if let dmasm::Node::Instruction(ins, debug) = node {
				if at_offset {
					match ins {
						dmasm::Instruction::DbgFile(_) => {
							other_file = true;
							continue;
						}
						dmasm::Instruction::DbgLine(_) if other_file => continue,
						_ => {
							offset = Some(debug.offset);
							break;
						}
					}
				}

				match ins {
					dmasm::Instruction::DbgFile(name) => {
						inside = files.is_none_or(|files| files.contains(&Self::normalized(&String::from_utf8_lossy(&name.0))));
					}
					dmasm::Instruction::DbgLine(current_line) if inside && current_line == line => at_offset = true,
					_ => {}
				}
			}
		}

		offset
	}

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
		if value.raw.tag == ValueTag::World && unsafe { value.raw.data.id == 1 } {
			return true;
		}

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
				Ok(v) if v.is_empty() => match value.raw.tag {
					ValueTag::World => "world".to_owned(),
					ValueTag::Pointer => "a pointer to a var that's gone".to_owned(),
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

	fn kind(value: &Value) -> Option<String> {
		use ValueTag as Tag;
		if List::is_list(value) {
			return Some(
				if List::from_value(value).is_ok_and(|list| list.is_alist()) {
					"/alist"
				} else {
					"/list"
				}
				.to_owned()
			);
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
			Tag::MobTypepath
			| Tag::ObjTypepath
			| Tag::TurfTypepath
			| Tag::AreaTypepath
			| Tag::DatumTypepath
			| Tag::SaveFileTypepath
			| Tag::ListTypepath
			| Tag::ClientTypepath
			| Tag::ImageTypepath
			| Tag::BuiltinTypepath => "path",
			_ => return value.get(byond_string!("type")).ok().map(|path| Self::stringify(&path))
		};
		Some(word.to_owned())
	}

	fn literal(value: &Value) -> Option<String> {
		use ValueTag as Tag;
		match value.raw.tag {
			Tag::String => Self::text_literal(StringRef::from_value(value.clone())?.data()),
			Tag::Number | Tag::MobTypepath | Tag::ObjTypepath | Tag::TurfTypepath | Tag::AreaTypepath | Tag::DatumTypepath => {
				Some(Self::stringify(value))
			}
			_ => None
		}
	}

	fn text_literal(bytes: &[u8]) -> Option<String> {
		let mut literal = String::with_capacity(bytes.len() + 2);
		literal.push('"');
		for c in std::str::from_utf8(bytes).ok()?.chars() {
			match c {
				'\\' | '"' | '[' | ']' => {
					literal.push('\\');
					literal.push(c);
				}
				'\n' => literal.push_str("\\n"),
				'\t' => literal.push_str("\\t"),
				_ if c.is_control() => return None,
				_ => literal.push(c)
			}
		}
		literal.push('"');
		Some(literal)
	}

	fn entry_expr(list: &str, key: &Value, position: Option<u32>) -> Option<String> {
		use ValueTag as Tag;
		if let Some(key) = Self::literal(key) {
			return Some(format!("{}[{}]", list, key));
		}
		let locatable = matches!(
			key.raw.tag,
			Tag::Turf | Tag::Obj | Tag::Mob | Tag::Area | Tag::Client | Tag::Image | Tag::Datum | Tag::List | Tag::Alist
		);
		if locatable {
			return Some(format!("{}[locate(\"\\{}\")]", list, Self::reference(key)));
		}
		position.map(|i| format!("{}[{}[{}]]", list, list, i))
	}

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

			Some(state) if value.raw.tag == ValueTag::Pointer && auxtools::pointer::read_raw(value).is_some() => {
				Some(state.get_ref(Variables::Pointee(value.clone(), expr)))
			}

			_ => None
		}
	}

	fn list_to_variables(&mut self, value: &Value, expr: Option<&str>, start: Option<u32>, count: Option<u32>) -> Result<Vec<Variable>, Runtime> {
		let state = self.state.as_ref().unwrap();
		let list = List::from_value(value)?;
		let len = list.len();
		let first = start.unwrap_or(0).min(len);
		let last = count.map_or(len, |count| first.saturating_add(count).min(len));

		let alist = list.is_alist();
		let pair = |i: u32, key: Value, value: Value| {
			let entry = expr.and_then(|list| Self::entry_expr(list, &key, (!alist).then_some(i)));
			Variable {
				name: format!("[{}]", i),
				value: format!("{} = {}", Self::stringify(&key), Self::stringify(&value)),
				kind: Self::kind(&value),
				entries: None,
				expr: entry.clone(),
				variables: Some(state.get_ref(Variables::ListPair {
					key,
					value,
					of: Value::from(&list),
					entry
				}))
			}
		};

		if alist {
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

			if key.raw.tag != raw_types::values::ValueTag::Number {
				if let Ok(value) = list.get(&key) {
					if value.raw.tag != raw_types::values::ValueTag::Null {
						variables.push(pair(i, key, value));
						continue;
					}
				}
			}

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
		let mut top_variables = vec![];

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

	fn shown(stack: &[debug::StackFrame]) -> &[debug::StackFrame] {
		match stack.iter().rposition(|frame| !crate::launched::runs_spawns(&frame.proc)) {
			Some(last) => &stack[..=last],
			None => stack
		}
	}

	fn only_runs_spawns(instance: *mut raw_types::procs::ProcInstance) -> bool {
		let mut ctx = unsafe { (*instance).context };
		while !ctx.is_null() {
			let proc = Proc::from_id(unsafe { (**(*ctx).proc_instance()).proc });
			if !proc.is_some_and(|proc| crate::launched::runs_spawns(&proc)) {
				return false;
			}
			ctx = unsafe { *(*ctx).parent_context() };
		}
		true
	}

	fn thread_of(&self, stack: &[debug::StackFrame]) -> u32 {
		Self::shown(stack)
			.last()
			.map_or(0, |frame| unsafe { (*frame.instance).argslist_idx.id }.wrapping_add(1))
	}

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

	fn get_stack_frame(&self, frame_index: u32) -> Option<&debug::StackFrame> {
		let state = self.state.as_ref()?;
		if frame_index < SLEEPING_FRAMES {
			return state.active().get(frame_index as usize);
		}

		let frame = state.sleeping_frame(frame_index)?;
		let position = self.stack_position(frame.thread).filter(|&position| position != 0)?;
		self.get_stack(position)?
			.get(frame.depth)
			.filter(|found| found.instance as usize == frame.instance)
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

		for (name, (_, local)) in Self::visible_locals(&frame).into_iter().zip(&frame.locals) {
			if let Some(name) = name {
				vars.push(self.value_to_variable(name.clone(), local, Some(name)));
			}
		}

		vars
	}

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

	fn breakpoint_offset(proc: &Proc, offset: u32) -> u32 {
		if offset == 0 {
			Self::entry_offset(proc)
		} else {
			offset
		}
	}

	fn handle_breakpoint_set(
		&mut self,
		mut instruction: InstructionRef,
		condition: Option<String>,
		hit: Option<HitCondition>,
		log: Option<Vec<LogPart>>,
		exactly: bool
	) {
		let (Some(proc), Some(client)) = (unnamed::find(&instruction.proc.path, instruction.proc.override_id), self.current) else {
			self.answer(Response::BreakpointSet {
				result: BreakpointSetResult::Failed
			});
			return;
		};
		let placed = Placed::of(instruction.offset, exactly);
		if !exactly {
			instruction.offset = Self::breakpoint_offset(&proc, instruction.offset);
		}
		let (file, line) = Self::breakpoint_place(&proc, instruction.offset);

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
			self.answer(Response::BreakpointSet {
				result: BreakpointSetResult::Refused { reason }
			});
			return;
		}

		match hook_instruction(&proc, instruction.offset) {
			Ok(()) => {
				let key = (proc.id, instruction.offset as u16);
				let claims = self.breakpoints.entry(key).or_default();
				let told = claims.get(&(client, placed)).is_some_and(|old| old.told && old.condition == condition);
				claims.insert((client, placed), Claim {
					condition,
					hit,
					hits: 0,
					log,
					skipped: 0,
					told
				});

				let offset = instruction.offset;
				self.moved_breakpoints
					.retain(|&(id, from, who), &mut at| id != proc.id || who != (client, placed) || (at != offset && from != offset));

				instruction.proc = unnamed::proc_ref(&proc);
				self.answer(Response::BreakpointSet {
					result: BreakpointSetResult::Success { line, at: instruction, file }
				});
			}

			Err(_) => {
				self.answer(Response::BreakpointSet {
					result: BreakpointSetResult::Failed
				});
			}
		}
	}

	fn expression_error(proc: &Proc, condition: &str) -> Option<String> {
		let mut names = vec![".".to_owned(), "usr".to_owned(), "src".to_owned()];
		names.extend(proc.parameter_names().iter().map(String::from));
		names.extend(proc.local_names().iter().map(String::from));
		let names: Vec<&str> = names.iter().map(String::as_str).collect();
		let condition = match crate::macros::expand(condition) {
			Ok(condition) => condition,
			Err(e) => return Some(e)
		};
		dmasm::compiler::compile_expr_for(&condition, &names, crate::world_byond())
			.err()
			.map(|e| e.to_string())
	}

	fn log_message(&mut self, parts: &[LogPart]) -> String {
		let mut message = String::new();
		for part in parts {
			match part {
				LogPart::Text(text) => message.push_str(text),
				LogPart::Expr(expression) => {
					match self.eval_expr_plain(Some(0), expression) {
						Ok(value) => message.push_str(&value.as_string().unwrap_or_else(|_| Self::stringify(&value))),
						Err(error) => message.push_str(&format!("{{{}: {}}}", expression, error))
					}
					self.state.as_mut().unwrap().invalidate_stacks();
				}
			}
		}
		message
	}

	fn handle_breakpoint_unset(&mut self, instruction: InstructionRef, exactly: bool) {
		let (Some(proc), Some(client)) = (unnamed::find(&instruction.proc.path, instruction.proc.override_id), self.current) else {
			self.answer(Response::BreakpointUnset { success: false, skipped: 0 });
			return;
		};

		let placed = Placed::of(instruction.offset, exactly);
		let asked = if exactly {
			instruction.offset
		} else {
			Self::breakpoint_offset(&proc, instruction.offset)
		};
		let who = (client, placed);
		let offset = self.moved_breakpoints.get(&(proc.id, asked, who)).copied().unwrap_or(asked);

		self.moved_breakpoints
			.retain(|(id, _, other), at| *id != proc.id || *at != offset || *other != who);

		let key = (proc.id, offset as u16);
		let claims = self.breakpoints.entry(key).or_default();
		let had = claims.remove(&who);
		let skipped = had.as_ref().map_or(0, |claim| claim.skipped);
		let success = if claims.is_empty() {
			self.breakpoints.remove(&key);
			unhook_instruction(&proc, offset).is_ok()
		} else {
			had.is_some()
		};
		self.answer(Response::BreakpointUnset { success, skipped });
	}

	fn handle_stacks(&mut self, sleeping: Option<u32>) {
		let sleeping = sleeping.unwrap_or(DEFAULT_SLEEPING_STACKS) as usize;
		if let Some(state) = &mut self.state {
			state.read_sleeping(sleeping.saturating_add(1));
		}
		let mut asleep = 0;
		let stacks = match &self.state {
			Some(state) => {
				let instances = debug::CallStacks::suspended_instances();
				asleep = instances.len() - instances.into_iter().filter(|&instance| Self::only_runs_spawns(instance)).count();
				let suspended = state
					.suspended()
					.iter()
					.filter(|stack| !stack.iter().all(|frame| crate::launched::runs_spawns(&frame.proc)))
					.take(sleeping);
				let listed = std::iter::once(state.active()).chain(suspended);
				listed
					.enumerate()
					.filter(|(_, stack)| !stack.is_empty())
					.map(|(position, stack)| {
						let (file, line) = self.frame_place(&stack[0], position != 0);
						Stack {
							id: self.thread_of(stack),
							name: unnamed::proc_name(&stack[0].proc),
							file,
							line,
							sleeping: position != 0
						}
					})
					.collect()
			}

			None => vec![]
		};

		self.answer(Response::Stacks {
			stacks,
			sleeping: asleep as u32
		});
	}

	fn handle_stack_frames(&mut self, thread: u32, start_frame: Option<u32>, count: Option<u32>) {
		let position = self.stack_position(thread);
		let response = match position.and_then(|position| self.get_stack(position)) {
			Some(stack) => {
				let stack_id = position.unwrap_or(0);
				let thread = self.thread_of(stack);
				let stack = Self::shown(stack);
				let start_frame = start_frame.unwrap_or(0);
				let end_frame = start_frame.saturating_add(count.unwrap_or(stack.len() as u32));

				let start_frame = start_frame as usize;
				let end_frame = end_frame as usize;

				let mut frames = vec![];

				for i in start_frame..end_frame {
					if i >= stack.len() {
						break;
					}

					let id = match &self.state {
						Some(state) if stack_id != 0 => state.sleeping_frame_id(SleepingFrame {
							thread,
							depth: i,
							instance: stack[i].instance as usize
						}),
						_ => i as u32
					};
					frames.push(self.frame_info(&stack[i], id, stack_id != 0 && i == 0));
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

		self.answer(response);
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

		self.answer(response);
	}

	fn list_variables(&mut self, container: Variables, start: Option<u32>, count: Option<u32>) -> Result<Vec<Variable>, Runtime> {
		match container {
			Variables::Arguments { frame } => Ok(self.get_args(frame)),
			Variables::Locals { frame } => Ok(self.get_locals(frame)),
			Variables::Globals => self.globals_to_variables(false),
			Variables::Builtins => self.globals_to_variables(true),
			Variables::ObjectVars(value, expr) => self.object_to_variables(&value, expr.as_deref()),
			Variables::ListContents(value, expr) => self.list_to_variables(&value, expr.as_deref(), start, count),
			Variables::ListPair { key, value, entry, .. } => Ok(vec![
				self.value_to_variable("key".to_owned(), &key, None),
				self.value_to_variable("value".to_owned(), &value, entry),
			]),
			Variables::Pointee(pointer, expr) => Ok(match auxtools::pointer::read(&pointer) {
				Some(named) => vec![self.value_to_variable("*".to_owned(), &named, expr.map(|pointer| format!("*{}", pointer)))],
				None => vec![]
			})
		}
	}

	fn handle_variables(&mut self, vars: VariablesRef, start: Option<u32>, count: Option<u32>) {
		let listed = match self.state.as_ref().and_then(|state| state.get_variables(vars)) {
			Some(container) => self.list_variables(container, start, count),
			None => Ok(vec![])
		};

		let vars = listed.unwrap_or_else(|e| {
			self.notify(format!("Couldn't list variables: {}", e.message));
			vec![]
		});
		self.answer(Response::Variables { vars });
	}

	fn types_of(value: &Value) -> Vec<String> {
		let mut types = vec![];
		let mut path = value.get(byond_string!("type")).ok();
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

	fn each_proc(mut each: impl FnMut(&str, &str)) {
		for id in 0..auxtools::runtime_tables::proc_count() {
			let Some(proc) = Proc::from_id(raw_types::procs::ProcId(id)) else {
				continue;
			};
			let Some((owner, name)) = proc.path.rsplit_once('/') else {
				continue;
			};
			if name.starts_with("auxtools_") || name.ends_with(')') || name.is_empty() {
				continue;
			}
			each(owner, name);
		}
	}

	fn complete(&mut self, frame_id: Option<u32>, base: Option<&str>, prefix: &str) -> Vec<Completion> {
		let mut found: BTreeMap<String, bool> = BTreeMap::new();
		match base {
			Some(base) => {
				let value = self.eval_expr(frame_id, base);
				if let Some(state) = self.state.as_mut() {
					state.invalidate_stacks();
				}
				let Ok(value) = value else {
					return vec![];
				};
				Self::members_of(&value, &mut found);
			}

			None => {
				let mut src = None;
				if let Some(frame) = frame_id.and_then(|frame_id| self.get_stack_frame(frame_id)) {
					for name in [".", "src", "usr", "args"] {
						found.insert(name.to_owned(), false);
					}
					found.extend(
						frame
							.args
							.iter()
							.filter_map(|(name, _)| name.as_ref())
							.map(|name| (String::from(name), false))
					);
					found.extend(Self::visible_locals(&frame).into_iter().flatten().map(|name| (name, false)));
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

	fn entry_number(name: &str) -> Option<u32> {
		name.strip_prefix('[')?.strip_suffix(']')?.parse().ok().filter(|&n| n > 0)
	}

	fn write_variable(&mut self, container: &Variables, name: &str, value: &Value) -> Result<(), String> {
		let runtime = |e: Runtime| format!("runtime error: {}", e.message);
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
				let frame = self.get_stack_frame(*frame).ok_or("frame no longer exists")?;
				if name == "." {
					unsafe { put((*frame.context).dot_mut(), value) };
					return Ok(());
				}
				let visible = Self::visible_locals(&frame);
				let mut named = visible.iter().enumerate().filter(|(_, local)| local.as_deref() == Some(name));
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
				let frame = self.get_stack_frame(*frame).ok_or("frame no longer exists")?;
				match name {
					"src" => unsafe { put(&raw mut (*frame.instance).src, value) },
					"usr" => unsafe { put(&raw mut (*frame.instance).usr, value) },
					_ => {
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
				let number = Self::entry_number(name)
					.filter(|&n| n <= list.len())
					.ok_or_else(|| format!("the list has no entry {}", name))?;
				if list.is_alist() {
					let (key, _) = list
						.alist_pairs()
						.map_err(runtime)?
						.into_iter()
						.nth(number as usize - 1)
						.ok_or("the list has changed")?;
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
				_ => Err("list keys cannot be edited; remove the entry and add a new one".to_owned())
			},

			Variables::Pointee(..) => Err("assign to the target variable to change the pointed-to value".to_owned())
		}
	}

	fn set_variable(&mut self, vars: VariablesRef, name: &str, value: &str) -> Result<Variable, String> {
		let container = self
			.state
			.as_ref()
			.and_then(|state| state.get_variables(vars))
			.ok_or("variable reference expired after execution resumed")?;
		let frame = match &container {
			Variables::Arguments { frame } | Variables::Locals { frame } => *frame,
			_ => 0
		};
		let value = self.eval_expr(Some(frame), value);
		self.state.as_mut().unwrap().invalidate_stacks();
		let value = value?;

		self.in_eval = true;
		auxtools::instruction_hook::hear_all();
		self.eval_error = None;
		let written = self.write_variable(&container, name, &value);
		self.in_eval = false;
		self.refresh_watches();
		self.state.as_mut().unwrap().invalidate_stacks();
		if let Some(error) = self.eval_error.take() {
			return Err(format!("runtime error: {}", error));
		}
		written?;

		if let Variables::ListPair { entry, .. } = &container {
			return Ok(self.value_to_variable(name.to_owned(), &value, entry.clone()));
		}

		let (start, count) = match &container {
			Variables::ListContents(..) => (Self::entry_number(name).map(|n| n - 1), Some(1)),
			_ => (None, None)
		};
		let listed = self.list_variables(container, start, count).map_err(|e| e.message)?;
		listed
			.into_iter()
			.find(|variable| variable.name == name)
			.ok_or_else(|| format!("{} is gone", name))
	}

	fn handle_command(&mut self, frame_id: Option<u32>, command: &str) -> String {
		match console::parse(command) {
			Ok(console::Command::Disassemble { proc: Some(proc), id }) => self.handle_disassemble(proc, id),
			Ok(console::Command::Disassemble { proc: None, .. }) => match frame_id {
				Some(frame_id) => match self.get_stack_frame(frame_id) {
					Some(frame) => {
						let proc = unnamed::proc_ref(&frame.proc);
						self.handle_disassemble(&proc.path, proc.override_id)
					}
					None => "couldn't find stack frame (is execution not paused?)".to_owned()
				},
				None => "no execution frame selected".to_owned()
			},
			Ok(console::Command::Skin) => match self.evaluating(crate::skin::state) {
				Ok(None) => "no skin".to_owned(),
				Ok(Some((resource, text))) => format!("resource {}\n{}", resource, text),
				Err(e) => format!("Failed: {}", e)
			},
			Ok(console::Command::Script) => self.evaluating(crate::client_script::state).unwrap_or_else(|e| format!("Failed: {}", e)),
			Ok(console::Command::Help(text)) => text.to_owned(),
			Err(error) => error
		}
	}

	fn eval_expr(&mut self, frame_id: Option<u32>, command: &str) -> Result<Value, String> {
		self.eval_expr_plain(frame_id, command).map_err(Self::unknown_name)
	}

	fn eval_expr_plain(&mut self, frame_id: Option<u32>, command: &str) -> Result<Value, String> {
		self.in_eval = true;
		auxtools::instruction_hook::hear_all();
		self.eval_error = None;
		self.eval_callee_errors.clear();
		self.eval_callee_more = 0;
		self.eval_warnings.clear();
		self.eval_ran = std::ptr::null_mut();

		let result = self.compile_and_run_expr(frame_id, command);
		self.eval_given.clear();
		self.eval_src = None;
		self.refresh_watches();

		self.in_eval = false;

		match self.eval_error.take() {
			Some(error) => Err(format!("runtime error: {}", error)),
			None => result
		}
	}

	fn unknown_name(error: String) -> String {
		let name = match (error.strip_prefix("undefined var: "), error.strip_prefix("proc not found: ")) {
			(Some(name), _) => name,
			(_, Some(path)) => path.rsplit('/').next().unwrap_or(path),
			_ => return error
		};
		let capitals = name.chars().any(|c| c.is_ascii_uppercase()) && !name.chars().any(|c| c.is_ascii_lowercase());
		if !capitals || crate::macros::given() {
			return error;
		}
		format!(
			"{} (if that's a #define: source macros unavailable until the project is parsed for hot reload)",
			error
		)
	}

	fn assemble_error(error: dmasm::assembler::AssembleError) -> String {
		use dmasm::assembler::AssembleError;
		let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
		match error {
			AssembleError::ProcNotFound(path) => format!("proc not found: {}", path),
			AssembleError::VariableNameNotFound(name) => format!("variable not found: {}", text(&name)),
			AssembleError::StringNotFound(string) => format!("string not found: {}", text(&string)),
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
			Given
		}

		let command = &*crate::macros::expand(command)?;

		let globals = unsafe { Value::new(ValueTag::GlobalVars, ValueData { id: 0 }) };
		let mut src = None;
		let given_src = self.eval_src.take();

		let (ctx, instance, mut args) = match frame_id {
			None => {
				let mut args = vec![];
				if let Some(object) = given_src {
					src = Some(object.clone());
					args.push(("src".to_owned(), object, ArgType::Given));
				}
				(std::ptr::null_mut(), std::ptr::null_mut(), args)
			}

			Some(frame_id) => {
				let Some(frame) = self.get_stack_frame(frame_id) else {
					return Err(format!("there's no frame {} (the world isn't paused, or has carried on since)", frame_id));
				};

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

				for (idx, (local, name)) in frame.locals.iter().zip(Self::visible_locals(&frame)).enumerate() {
					if let Some(name) = name {
						args.push((name, local.1.clone(), ArgType::Local(idx as u32)));
					}
				}

				(frame.context, frame.instance, args)
			}
		};

		args.push((GLOBALS.to_owned(), globals, ArgType::Given));
		for (name, value) in std::mem::take(&mut self.eval_given).into_iter().rev() {
			args.insert(0, (name, value, ArgType::Given));
		}
		let arg_names: Vec<&str> = args.iter().map(|(name, ..)| name.as_str()).collect();
		let arg_values: Vec<&Value> = args.iter().map(|(_, value, _)| value).collect();

		let has_var = |object: &Value, name: &str| StringRef::new(name).is_ok_and(|name| object.get(name).is_ok());
		let scope = dmasm::compiler::ExprScope {
			has_src_var: &|name| src.as_ref().is_some_and(|src| has_var(src, name)),
			has_src_proc: &|name| src.as_ref().is_some_and(|src| crate::assemble_env::AssembleEnv::has_proc(src, name)),
			global: &|name| {
				if has_var(&Value::GLOBAL, name) {
					Some(dmasm::compiler::ExprGlobal::Listed)
				} else {
					hot_reload::added_global(name).map(dmasm::compiler::ExprGlobal::Id)
				}
			},
			globals: GLOBALS,
			proc_ref: &crate::assemble_env::type_proc,
			proc_path: &crate::assemble_env::proc_path
		};
		let key = EvalCodeKey {
			generation: hot_reload::code_generation(),
			command: command.to_owned(),
			names: arg_names.iter().map(|name| (*name).to_owned()).collect(),
			src_type: src.as_ref().and_then(Self::kind)
		};
		let compiled = match self.eval_code.get(&key) {
			Some(compiled) => compiled.clone(),
			None => {
				let compiled = Self::compile_eval(command, &arg_names, &scope).and_then(|(code, locals, warnings)| {
					let assembly = dmasm::assembler::assemble(&code, &mut crate::assemble_env::AssembleEnv).map_err(Self::assemble_error)?;
					Ok((assembly, locals, warnings))
				});
				if self.eval_code.len() >= EVAL_CODE_KEPT {
					self.eval_code.clear();
				}
				self.eval_code.insert(key, compiled.clone());
				compiled
			}
		};
		let (assembly, locals, warnings) = compiled?;
		self.eval_warnings = warnings;
		if let Some(number) = self.spawning.take() {
			self.spawns.push_back(Spawn {
				number,
				bytecode: assembly,
				locals,
				args: arg_values.into_iter().cloned().collect()
			});
			if let Some(inbox) = self.spawn_inbox.as_ref().and_then(|inbox| List::from_value(inbox).ok()) {
				inbox.append(Value::from(number as f32));
			}
			return Ok(Value::NULL);
		}
		let proc = Proc::find("/proc/auxtools_expr_stub").ok_or("the world has no /proc/auxtools_expr_stub to evaluate expressions in")?;

		proc.set_bytecode(assembly);

		let ends = crate::WORLD_ENDS.load(std::sync::atomic::Ordering::Relaxed);
		let result = Self::call_with_locals(&proc, &locals, &arg_values)?;
		if crate::WORLD_ENDS.load(std::sync::atomic::Ordering::Relaxed) != ends {
			std::mem::forget(result);
			std::mem::forget(args);
			std::panic::resume_unwind(Box::new(crate::WorldEnded));
		}

		match result {
			Ok(res) => {
				if let Ok(list) = res.as_list() {
					if frame_id.is_some() && !Self::frame_is_live(ctx) {
						return Err("evaluation ended the selected frame".to_owned());
					}

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
				} else if !self.eval_ran.is_null() && background_known() && unsafe { *(*self.eval_ran).background() } != 0 {
					Err(
						"evaluation aborted: BYOND moved the code to the background, so its result is unavailable. This can happen after about a \
						 million loop iterations (world.loop_checks), or with `set background = 1`. The code is still running. Use `eval --spawn \
						 --wait` to wait for a result"
							.to_owned()
					)
				} else if !background_known() {
					Err(
						"evaluation aborted because the code slept or BYOND moved it to the background, so its result is unavailable. BYOND does \
						 that after about a million loop iterations (world.loop_checks), or with `set background = 1`. Use `eval --spawn --wait` to \
						 wait for a result"
							.to_owned()
					)
				} else {
					Err("evaluation aborted because the code slept".to_owned())
				}
			}

			Err(Runtime { message }) => Err(format!("runtime error: {}", message))
		}
	}

	fn compile_eval(
		command: &str,
		arg_names: &[&str],
		scope: &dmasm::compiler::ExprScope
	) -> Result<(Vec<dmasm::Node>, Vec<String>, Vec<String>), String> {
		let byond = crate::world_byond();
		if dmasm::compiler::is_body(command) {
			let body = dmasm::compiler::compile_body_in(command, arg_names, Some(scope), byond).map_err(|e| e.to_string())?;
			return Ok((body.nodes, body.locals, body.warnings));
		}
		match dmasm::compiler::compile_expr_warned(command, arg_names, Some(scope), byond) {
			Ok((expr, warnings)) => Ok((expr, vec![], warnings)),
			Err(_) if command.trim().contains(['\n', ';']) => {
				let error = dmasm::compiler::compile_body_in(command, arg_names, Some(scope), byond).err();
				Err(error.map_or_else(|| "invalid expression or proc body".to_owned(), |e| e.to_string()))
			}
			Err(e) => Err(e.to_string())
		}
	}

	pub(crate) fn call_with_locals(proc: &Proc, locals: &[String], args: &[&Value]) -> Result<Result<Value, Runtime>, String> {
		use auxtools::runtime_tables::{self, ProcEntryWords};
		let ends = crate::WORLD_ENDS.load(std::sync::atomic::Ordering::Relaxed);
		let none = match locals.is_empty() {
			true => None,
			false => {
				let none = runtime_tables::proc_entry(proc.id).ok_or("couldn't read the stub's proc entry")?.0[ProcEntryWords::LOCALS];
				let table = hot_reload::locals_table(locals, none)?;
				runtime_tables::set_proc_entry_word(proc.id, ProcEntryWords::LOCALS, table);
				Some(none)
			}
		};
		let tries = Self::hide_tries();
		let result = proc.call(args);
		if crate::WORLD_ENDS.load(std::sync::atomic::Ordering::Relaxed) == ends {
			Self::show_tries(tries);
			if let Some(none) = none {
				runtime_tables::set_proc_entry_word(proc.id, ProcEntryWords::LOCALS, none);
			}
		}
		Ok(result)
	}

	pub(crate) fn hide_tries() -> Vec<(*mut raw_types::procs::ExecutionContext, u32)> {
		let mut hidden = vec![];
		let mut at = unsafe { *raw_types::funcs::CURRENT_EXECUTION_CONTEXT };
		while !at.is_null() {
			let handler = unsafe { *(*at).throw_handler() };
			if handler != 0 {
				hidden.push((at, handler));
				unsafe { *(*at).throw_handler_mut() = 0 };
			}
			at = unsafe { *(*at).parent_context() };
		}
		hidden
	}

	pub(crate) fn show_tries(tries: Vec<(*mut raw_types::procs::ExecutionContext, u32)>) {
		for (ctx, handler) in tries {
			unsafe { *(*ctx).throw_handler_mut() = handler };
		}
	}

	fn frame_is_live(ctx: *mut raw_types::procs::ExecutionContext) -> bool {
		let on = |mut at: *mut raw_types::procs::ExecutionContext| {
			while !at.is_null() {
				if at == ctx {
					return true;
				}
				at = unsafe { *(*at).parent_context() };
			}
			false
		};
		on(unsafe { *raw_types::funcs::CURRENT_EXECUTION_CONTEXT })
			|| debug::CallStacks::suspended_instances()
				.into_iter()
				.any(|instance| on(unsafe { (*instance).context }))
	}

	fn handle_eval_request(&mut self, frame_id: Option<u32>, command: &str, context: Option<&str>) {
		let ending = context == Some(ENDING_CONTEXT);
		if ending {
			self.ending = Some(Instant::now());
		}
		let still_running = self.handle_eval(frame_id, command);
		if ending && !still_running {
			self.ending = None;
		}
	}

	fn handle_eval(&mut self, frame_id: Option<u32>, command: &str) -> bool {
		#[cfg(debug_assertions)]
		if command == "#panic" {
			panic!("asked for with #panic");
		}
		if let Some(command) = command.strip_prefix('#') {
			let response = self.handle_command(frame_id, command);
			self.answer(Response::Eval(Ok(EvalResponse {
				value: response,
				variables: None,
				kind: None,
				entries: None
			})));
			return false;
		}

		let reached = (!dmasm::compiler::is_body(command)).then(|| format!("({})", command));
		let response = self.eval_expr(frame_id, command).map(|result| EvalResponse {
			value: Self::stringify(&result),
			variables: self.value_to_variables_ref(&result, reached),
			kind: Self::kind(&result),
			entries: Self::entries(&result)
		});
		let still_running = matches!(&response, Err(error) if error.starts_with(EVAL_ABORTED));
		self.send_callee_errors();
		self.answer(Response::Eval(response));
		still_running
	}

	fn handle_eval_on(&mut self, object: &str, expression: &str, depth: Option<u32>) {
		let value = self.object_of(object).and_then(|object| {
			self.eval_src = Some(object);
			self.eval_expr(None, expression)
		});
		let response = match depth {
			Some(depth) => Response::Render(value.map(|value| Self::render(&value, depth))),
			None => Response::Eval(value.map(|result| EvalResponse {
				value: Self::stringify(&result),
				variables: self.value_to_variables_ref(&result, None),
				kind: Self::kind(&result),
				entries: Self::entries(&result)
			}))
		};
		self.send_callee_errors();
		self.answer(response);
	}

	fn handle_spawn(&mut self, expression: &str, object: Option<&str>) {
		let result = self.spawn(expression, object);
		self.answer(Response::Spawned(result));
	}

	fn spawn(&mut self, expression: &str, object: Option<&str>) -> Result<u64, String> {
		crate::launched::can_spawn()?;
		let object = object.map(|object| self.object_of(object)).transpose()?;
		let number = NEXT_SPAWN.fetch_add(1, Ordering::Relaxed);
		self.eval_src = object;
		self.spawning = Some(number);
		let compiled = self.eval_expr(None, expression);
		self.spawning = None;
		compiled?;
		self.spawned.insert(number, (expression.to_owned(), None));
		Ok(number)
	}

	pub fn spawn_inbox(&mut self) -> Value {
		let inbox = Value::from(List::new());
		self.spawn_inbox = Some(inbox.clone());
		inbox
	}

	pub fn next_spawn(&mut self) -> DMResult {
		let Some(spawn) = self.spawns.pop_front() else {
			return Ok(Value::NULL);
		};
		let Some(stub) = Proc::find("/proc/auxtools_spawn_stub") else {
			self.spawn_failed(spawn.number, "the world has no /proc/auxtools_spawn_stub to run it in".to_owned());
			return Ok(Value::NULL);
		};
		if let Err(error) = self.set_stub_locals(&stub, &spawn.locals) {
			self.spawn_failed(spawn.number, error);
			return self.next_spawn();
		}
		stub.set_bytecode(spawn.bytecode);
		let args = List::new();
		for arg in &spawn.args {
			args.append(arg);
		}
		let next = List::new();
		next.append(Value::from(spawn.number as f32));
		next.append(Value::from(args));
		Ok(Value::from(next))
	}

	fn set_stub_locals(&mut self, stub: &Proc, locals: &[String]) -> Result<(), String> {
		use auxtools::runtime_tables::{self, ProcEntryWords};
		let now = runtime_tables::proc_entry(stub.id).ok_or("couldn't read the spawn stub's proc entry")?.0[ProcEntryWords::LOCALS];
		let none = *self.spawn_stub_locals.get_or_insert(now);
		let table = hot_reload::locals_table(locals, none)?;
		if table != now {
			runtime_tables::set_proc_entry_word(stub.id, ProcEntryWords::LOCALS, table);
		}
		Ok(())
	}

	pub fn spawn_done(&mut self, number: u64, returned: &Value) {
		let Some((expression, error)) = self.spawned.remove(&number) else {
			return;
		};
		let result = match (error, returned.as_list()) {
			(Some(error), _) => Err(error),
			(None, Ok(list)) => {
				let value = list.get(1).unwrap_or(Value::NULL);
				Ok(SpawnedValue {
					text: Self::stringify(&value),
					rendered: Self::render(&value, 1)
				})
			}
			(None, Err(_)) => Err("evaluation ended without a value".to_owned())
		};
		self.journal(
			JournalEvent::Spawned {
				spawn: number,
				expression,
				result
			},
			None
		);
	}

	fn spawn_failed(&mut self, number: u64, why: String) {
		let expression = self.spawned.remove(&number).map(|(expression, _)| expression).unwrap_or_default();
		self.journal(
			JournalEvent::Spawned {
				spawn: number,
				expression,
				result: Err(why)
			},
			None
		);
	}

	fn object_of(&mut self, reference: &str) -> Result<Value, String> {
		let text = reference.trim();
		let inner = text.strip_prefix('[').and_then(|text| text.strip_suffix(']')).unwrap_or(text);
		let id = inner
			.strip_prefix("0x")
			.or_else(|| inner.strip_prefix("0X"))
			.and_then(|hex| u32::from_str_radix(hex, 16).ok())
			.ok_or_else(|| format!("invalid object reference {}; expected a ref() value such as [0x2000001]", reference))?;
		let reference = format!("[0x{:x}]", id);
		let found = self.eval_expr(None, &format!("locate(\"\\{}\")", reference))?;
		if found == Value::NULL {
			return Err(format!("object not found: {}", reference));
		}
		Ok(found)
	}

	fn claim_reloads(&mut self, who: String) -> Result<(), String> {
		let id = self.current.ok_or("no client")?;
		match &self.reloader {
			Some((holder, theirs)) if *holder != id && self.clients.contains_key(holder) => Err(format!(
				"{} already owns hot reload for this world; two watchers would apply each change twice",
				theirs
			)),
			_ => {
				self.reloader = Some((id, who));
				Ok(())
			}
		}
	}

	fn reloads_refused(&self) -> Option<String> {
		let (holder, who) = self.reloader.as_ref()?;
		(Some(*holder) != self.current && self.clients.contains_key(holder)).then(|| format!("{} already owns hot reload for this world", who))
	}

	fn apply_pending_skin(&mut self) {
		let Some(request) = self.pending_skin.take() else {
			return;
		};
		if let Err(e) = self.evaluating(|| crate::skin::set(&request)) {
			self.notify(format!("could not restore skin {} after reboot: {}", request.path, e));
		}
	}

	fn apply_pending_script(&mut self) {
		let Some(request) = self.pending_script.take() else {
			return;
		};
		if let Err(e) = self.evaluating(|| crate::client_script::set(&request)) {
			self.notify(format!("could not restore client script after reboot: {}", e));
		}
	}

	fn evaluating<T>(&mut self, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
		self.in_eval = true;
		auxtools::instruction_hook::hear_all();
		self.eval_error = None;
		let result = work();
		self.in_eval = false;
		result
	}

	fn handle_hot_reload(&mut self, request: &HotReloadRequest) {
		if let Some(why) = self.reloads_refused() {
			let refused = HotReloadResult {
				applied: false,
				errors: vec![why],
				warnings: vec![],
				skipped: vec![],
				resources: vec![]
			};
			self.answer(Response::HotReload(refused));
			return;
		}
		let result = self.hot_reload(request);
		self.answer(Response::HotReload(result));
	}

	fn hot_reload(&mut self, request: &HotReloadRequest) -> HotReloadResult {
		resource_reload::use_dirs(&request.resource_dirs);
		if !crate::world_resources::is_loaded() {
			let mut files = crate::world_resources::not_built(request);
			for resource in &request.resources {
				if !files.contains(&resource.path) {
					files.push(resource.path.clone());
				}
			}
			if !files.is_empty() || !self.waiting_reloads.is_empty() {
				let mut waiting = request.clone();
				let mut early = vec![];
				if let Some(init) = &request.global_init {
					let (globals, others) = request.new_vars.iter().cloned().partition(|var| var.type_path.is_empty());
					let (init, left) = crate::hot_reload::early_global_init(init, request);
					if left > 0 {
						early.push(format!(
							"(global init): {} new static vars will initialize with the deferred reload at world/New",
							left
						));
					}
					let alone = HotReloadRequest {
						global_init: Some(init),
						new_vars: globals,
						..Default::default()
					};
					if crate::world_resources::not_built(&alone).is_empty() {
						let result = self.apply_hot_reload(&alone);
						if result.applied {
							waiting.global_init = None;
							waiting.new_vars = others;
						}
						early.extend(result.errors.into_iter().chain(result.warnings));
					}
				}
				self.waiting_reloads.push(waiting);
				let why = if files.is_empty() {
					"queued behind an earlier reload; applies as world/New starts".to_owned()
				} else {
					format!(
						"resource files need the loaded cache ({}); reload applies as world/New starts. Map atoms use the built code until then",
						files.join(", ")
					)
				};
				return HotReloadResult {
					applied: true,
					errors: vec![],
					warnings: std::iter::once(why).chain(early).collect(),
					skipped: vec![],
					resources: vec![]
				};
			}
		}
		self.apply_hot_reload(request)
	}

	fn apply_hot_reload(&mut self, request: &HotReloadRequest) -> HotReloadResult {
		let mut id_warnings = vec![];
		let mut with_ids;
		let request = if request.resources.is_empty() {
			request
		} else {
			with_ids = request.clone();
			let named: Vec<(String, Vec<u32>)> = request.resources.iter().map(|r| (r.path.clone(), r.ids.clone())).collect();
			for (path, ids) in &named {
				if !self.resource_ids.contains_key(path) {
					self.resource_origins.entry(path.clone()).or_insert_with(|| ids.clone());
				}
			}
			for resource in &mut with_ids.resources {
				if let Some(&last) = self.resource_ids.get(&resource.path).and_then(|ids| ids.last()) {
					resource.ids = vec![last];
				}
				let path = resource.path.clone();
				let owner = |id: u32| {
					self.resource_ids
						.iter()
						.chain(named.iter().map(|(p, ids)| (p, ids)))
						.find(|(p, ids)| **p != path && ids.contains(&id))
						.map(|(p, _)| p.clone())
				};
				let mut kept = vec![];
				for &id in &resource.ids {
					match owner(id) {
						Some(other) => id_warnings.push(format!(
							"resource reload: {} and {} share an ID (same bytes); appearances are unchanged. Rebuild to separate them",
							path, other
						)),
						None => kept.push(id)
					}
				}
				resource.ids = kept;
			}
			&with_ids
		};
		self.in_eval = true;
		auxtools::instruction_hook::hear_all();
		self.eval_error = None;

		let mut warnings = id_warnings;
		let mut skipped = vec![];
		let prepared = hot_reload::add_types(&request.new_types).and_then(|added| {
			hot_reload::prepare(request, &mut warnings, &mut skipped)
				.and_then(|prepared| Ok((prepared, resource_reload::prepare(&request.resources)?)))
				.map_err(|mut errors| {
					if added > 0 {
						errors.push(format!("{} new types remain allocated but unused", added));
					}
					errors
				})
		});
		let prepared = prepared.and_then(|(prepared, resources)| Ok((prepared, resource_reload::apply(&resources).map_err(|e| vec![e])?)));
		let result = match prepared {
			Ok((prepared, applied)) => {
				hot_reload::apply(prepared, &mut self.breakpoints, &mut self.moved_breakpoints, &mut warnings);
				warnings.extend(applied.warnings);
				if let Err(e) = resource_reload::resend(&applied.appearances) {
					warnings.push(format!("resource reload: could not send updated appearances to clients: {}", e));
				}
				self.record_resource_ids(&applied.reloaded, &mut warnings);
				self.record_named_files(request);
				HotReloadResult {
					applied: true,
					errors: vec![],
					warnings,
					skipped,
					resources: applied.reloaded
				}
			}

			Err(errors) => HotReloadResult {
				applied: false,
				errors,
				warnings,
				skipped,
				resources: vec![]
			}
		};

		self.in_eval = false;
		if let Some(err) = self.eval_error.take() {
			self.notify(format!("runtime error during hot reload: {}", err));
		}

		if result.applied && !request.is_empty() {
			self.broadcast(Response::CodeChanged, self.current);
		}

		result
	}

	fn record_named_files(&mut self, request: &HotReloadRequest) {
		use dmasm::assembler::AssembleEnv as _;
		for name in crate::world_resources::not_built(request) {
			let Some(file) = crate::assemble_env::resource_file(&name) else {
				continue;
			};
			let path = file.to_string_lossy().replace('\\', "/");
			if self.resource_ids.contains_key(&path) || crate::world_resources::built_file(&file) {
				continue;
			}
			if let Some(id) = crate::assemble_env::AssembleEnv.get_resource_index(&name) {
				self.resource_ids.insert(path, vec![id]);
			}
		}
	}

	fn record_resource_ids(&mut self, reloaded: &[ReloadedResource], warnings: &mut Vec<String>) {
		let built = crate::world_resources::built_count();
		for resource in reloaded {
			let own = self.resource_origins.get(&resource.path).is_some_and(|ids| ids.contains(&resource.id));
			let shared = (!own && built.is_some_and(|count| (resource.id as usize) < count))
				|| self
					.resource_ids
					.iter()
					.any(|(path, ids)| *path != resource.path && ids.contains(&resource.id));
			if shared {
				self.resource_ids.remove(&resource.path);
				warnings.push(format!(
					"resource reload: {} shares existing resource {} (same bytes); later edits to {} will leave appearances using that shared ID \
					 unchanged",
					resource.path, resource.id, resource.path
				));
			} else {
				let ids = self.resource_ids.entry(resource.path.clone()).or_default();
				ids.retain(|&id| id != resource.id);
				ids.push(resource.id);
			}
		}
	}

	pub fn world_data_loaded(&mut self) {
		self.apply_pending_script();
		for request in std::mem::take(&mut self.waiting_reloads) {
			let result = self.hot_reload(&request);
			if !result.applied {
				self.notify(format!("Hot reload failed as world/New started: {}", result.errors.join("; ")));
			}
			for warning in result.warnings {
				self.notify(format!("Hot reload: {}", warning));
			}
		}
		self.apply_pending_skin();
	}

	fn handle_disassemble(&mut self, path: &str, id: u32) -> String {
		match unnamed::find(path, id) {
			Some(proc) => {
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

	fn handle_request(&mut self, request: Request) -> bool {
		auxtools::instruction_hook::hear_all();
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
			} => self.handle_breakpoint_set(instruction, condition, hit, log, false),
			Request::BreakpointUnset { instruction } => self.handle_breakpoint_unset(instruction, false),
			Request::Stacks { sleeping } => self.handle_stacks(sleeping),
			Request::Scopes { frame_id } => self.handle_scopes(frame_id),
			Request::Variables { vars, start, count } => self.handle_variables(vars, start, count),
			Request::Complete { frame_id, base, prefix } => {
				let names = self.complete(frame_id, base.as_deref(), &prefix);
				self.answer(Response::Complete(names));
			}
			Request::SetVariable { vars, name, value } => {
				let result = self.set_variable(vars, &name, &value);
				self.send_callee_errors();
				self.answer(Response::SetVariable(result));
			}
			Request::Eval { frame_id, command, context } => self.handle_eval_request(frame_id, &command, context.as_deref()),

			Request::StackFrames {
				stack_id,
				start_frame,
				count
			} => self.handle_stack_frames(stack_id, start_frame, count),

			Request::Offset { proc, line } => {
				self.answer(Response::Offset {
					offset: self.get_offset(proc, line)
				});
			}

			Request::Pause => {
				self.answer(Response::Ack);
				return true;
			}

			Request::ByondVersion => {
				let (version, build) = auxtools::version::get();
				self.answer(Response::ByondVersion { version, build });
			}

			Request::HotReload(request) => self.handle_hot_reload(&request),
			Request::Reloader { who } => {
				let claimed = self.claim_reloads(who);
				self.answer(Response::Reloader(claimed));
			}
			Request::Skin(request) => {
				let result = match self.reloads_refused() {
					Some(why) => Err(why),
					None if !crate::world_resources::is_loaded() => {
						self.pending_skin = Some(request);
						Ok(SkinReloaded { clients: 0, hosted: false })
					}
					None => self.evaluating(|| crate::skin::set(&request))
				};
				self.answer(Response::Skin(result));
			}
			Request::Script(request) => {
				let result = match self.reloads_refused() {
					Some(why) => Err(why),
					None if !crate::world_resources::is_loaded() => {
						self.pending_script = Some(request);
						Ok(ScriptReloaded { clients: 0, hosted: false })
					}
					None => self.evaluating(|| crate::client_script::set(&request))
				};
				self.answer(Response::Script(result));
			}
			Request::Reconnect => {
				let reconnected = self.evaluating(crate::skin::reconnect);
				self.answer(Response::Reconnect(reconnected));
			}
			Request::Profile(request) => {
				let reply = self.profile(request);
				self.answer(Response::Profile(reply));
			}

			Request::Status => {
				let paused = self.paused.clone();
				self.answer(Response::Status { paused });
			}

			Request::HoldPause => {
				self.hold = self.current;
				self.hold_max = self.current.and_then(|id| self.clients.get(&id)).and_then(|client| client.hold_max);
				self.answer(Response::Ack);
			}

			Request::Locate { file, line } => {
				let instructions = self.locate(&file, line);
				self.answer(Response::Locate { instructions });
			}

			Request::LinesWithCode { file, start, end } => {
				let lines = self.lines_with_code(&file, start, end);
				self.answer(Response::LinesWithCode { lines });
			}

			Request::WatchInfo { place } => {
				let info = self.watch_info(&place);
				self.answer(Response::WatchInfo(info));
			}

			Request::WatchSet { place, condition, hit } => {
				let set = self.watch_set(&place, condition, hit);
				self.answer(Response::WatchSet(set));
			}

			Request::WatchUnset { id } => {
				self.watch_unset(id);
				self.answer(Response::Ack);
			}

			Request::Files => {
				let mut files: Vec<String> = self.file_index().keys().cloned().collect();
				files.sort();
				self.answer(Response::Files { files });
			}

			Request::WorldInfo => {
				let (args, cwd) = crate::launched::command_line();
				let port = Value::WORLD.get_number(byond_string!("port")).map_or(0, |port| port as u16);
				self.answer(Response::WorldInfo { args, cwd, port });
			}

			Request::StdDef => {
				let stddef = crate::stddef::get_stddef().map(|x| x.to_string());
				self.answer(Response::StdDef(stddef));
			}

			Request::Watcher => {
				if let Some(client) = self.current.and_then(|id| self.clients.get_mut(&id)) {
					client.debugs = false;
				}
				self.answer(Response::Ack);
			}

			Request::Disassemble { code } => {
				let disassembly = self.disassemble(code);
				self.answer(Response::Disassemble(disassembly));
			}

			Request::InstructionBreakpointSet {
				instruction,
				condition,
				hit,
				log
			} => self.handle_breakpoint_set(instruction, condition, hit, log, true),
			Request::InstructionBreakpointUnset { instruction } => self.handle_breakpoint_unset(instruction, true),

			Request::Journal { after, kinds, limit, follow } => self.handle_journal(after, kinds, limit, follow),
			Request::JournalAdd(event) => self.journal(event, self.current),
			Request::HoldMax { seconds } => {
				let hold_max = seconds
					.filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
					.map(Duration::from_secs_f64);
				if let Some(client) = self.current.and_then(|id| self.clients.get_mut(&id)) {
					client.hold_max = hold_max;
				}
				self.answer(Response::Ack);
			}
			Request::Uptime => {
				let (loaded, up, loads, last_reload) = journal::uptime();
				self.answer(Response::Uptime {
					loaded,
					up,
					world_time: journal::world_time(),
					loads,
					last_reload,
					started: !self.waiting_to_start
				});
			}
			Request::ProcInfo { proc } => {
				let info = Self::proc_info(&proc);
				self.answer(Response::ProcInfo(info));
			}

			Request::Render { frame_id, expression, depth } => self.handle_render(frame_id, &expression, depth),
			Request::RenderVariables { vars, depth } => self.handle_render_variables(vars, depth),
			Request::EvalOn { object, expression, depth } => self.handle_eval_on(&object, &expression, depth),
			Request::Defines(macros) => {
				crate::macros::set(macros);
				hot_reload::code_changed();
			}
			Request::Spawn { expression, object } => self.handle_spawn(&expression, object.as_deref()),

			Request::Configured | Request::Continue { .. } => {
				self.answer(Response::Ack);
			}
		}

		false
	}

	fn recv_request(&mut self, wait: Wait) -> Option<Request> {
		loop {
			let event = match wait {
				Wait::No => self.events.try_recv().ok()?,
				Wait::Until(Some(until)) => self.events.recv_timeout(until.saturating_duration_since(Instant::now())).ok()?,
				_ => self.events.recv().ok()?
			};

			match event {
				Event::Connected(id, connection) => {
					let (outbox, writer) = Self::writer(connection.clone());
					self.clients.insert(id, Client {
						uncaught: None,
						caught: None,
						debugs: true,
						follow: None,
						outbox,
						writer: Some(writer),
						hold_max: None
					});
					if let Wait::Until(_) = wait {
						return None;
					}
				}

				Event::Disconnected(id) => {
					self.remove_client(id);

					if (wait == Wait::WhileClients && !self.has_debuggers()) || matches!(wait, Wait::Until(_)) {
						return None;
					}
				}

				Event::Request(id, ..) if !self.clients.contains_key(&id) => waker::request_taken(),

				Event::Request(id, request_id, request) => {
					waker::request_taken();
					self.current = Some(id);
					self.answering = Some(request_id);
					return Some(request);
				}
			}
		}
	}

	fn has_debuggers(&self) -> bool {
		self.clients.values().any(|client| client.debugs)
	}

	pub fn notify<T: Into<String>>(&mut self, message: T) {
		let message = message.into();
		if self.clients.is_empty() {
			eprintln!("Debug Server: {}", message);
		}

		self.broadcast(Response::Notification { message }, None);
	}

	pub fn recover_from_panic(&mut self) {
		eprintln!("Debug server panicked, dropping the debug clients");
		self.in_eval = false;
		self.waiting_to_start = false;
		self.state = None;
		self.paused = None;
		self.disconnect_all();
		self.profiling = None;
		crate::profiler::discard();
	}

	fn breakpoint_hit(&mut self, key: (raw_types::procs::ProcId, u16)) -> bool {
		let (proc, offset) = key;
		let clients: Vec<(ClientId, Placed)> = self
			.breakpoints
			.get(&key)
			.map(|claims| claims.keys().copied().collect())
			.unwrap_or_default();

		let mut pause = clients.is_empty();
		for client in clients {
			let Some(condition) = self
				.breakpoints
				.get(&key)
				.and_then(|claims| claims.get(&client))
				.map(|claim| claim.condition.clone())
			else {
				continue;
			};
			if let Some(condition) = condition {
				let passed = self.eval_expr(Some(0), &condition).map(|result| result.is_truthy());

				self.state.as_mut().unwrap().invalidate_stacks();
				match passed {
					Ok(true) => {}
					Ok(false) => continue,
					Err(error) => {
						self.condition_failed(key, client, &condition, &error);
						continue;
					}
				}
			}

			let Some(claim) = self.breakpoints.get_mut(&key).and_then(|claims| claims.get_mut(&client)) else {
				continue;
			};
			claim.hits += 1;
			if claim.hit.is_some_and(|hit| !hit.takes(claim.hits)) {
				continue;
			}

			let Some(log) = claim.log.clone() else {
				pause = true;
				continue;
			};
			let message = self.log_message(&log);
			let caller = self
				.state
				.as_ref()
				.and_then(|state| state.active().get(1))
				.map(|frame| self.frame_info(frame, 1, false));
			let logged = Proc::from_id(proc).map(|proc| {
				let place = unnamed::proc_ref(&proc);
				let (file, line) = Self::place_of(&proc, offset as u32);
				Response::Log {
					message,
					file,
					line,
					proc: place,
					caller
				}
			});
			if let (Some(logged), Some(to)) = (logged, self.clients.get_mut(&client.0)) {
				if !Self::deliver(to, &logged) {
					self.remove_client(client.0);
				}
			}
		}
		pause
	}

	fn condition_failed(&mut self, key: (raw_types::procs::ProcId, u16), client: (ClientId, Placed), condition: &str, error: &str) {
		let Some(claim) = self.breakpoints.get_mut(&key).and_then(|claims| claims.get_mut(&client)) else {
			return;
		};
		claim.skipped += 1;
		if std::mem::replace(&mut claim.told, true) {
			return;
		}
		let place = match Proc::from_id(key.0) {
			Some(proc) => match Self::place_of(&proc, key.1 as u32) {
				(Some(file), Some(line)) => format!(" at {}:{}", file, line),
				_ => format!(" in {}", unnamed::proc_name(&proc))
			},
			None => String::new()
		};
		let message = format!(
			"The condition ({}) of the breakpoint{} failed; skipping affected hits (reported once): {}",
			condition, place, error
		);
		self.tell(client.0, Response::ConditionFailed { message });
	}

	fn tell(&mut self, client: ClientId, response: Response) {
		if let Some(to) = self.clients.get_mut(&client) {
			if !Self::deliver(to, &response) {
				self.remove_client(client);
			}
		}
	}

	pub fn handle_breakpoint(&mut self, _ctx: *mut raw_types::procs::ExecutionContext, reason: BreakpointReason) -> Option<ContinueKind> {
		if let BreakpointReason::Runtime { message, caught } = &reason {
			let stack = self.journal_stack();
			self.journal(
				JournalEvent::Runtime {
					message: message.clone(),
					caught: *caught,
					stack
				},
				None
			);
		}

		let wanted = match &reason {
			BreakpointReason::Runtime { message, caught } => self.clients.values().any(|client| {
				let filter = if *caught { &client.caught } else { &client.uncaught };
				filter.as_ref().is_some_and(|filter| filter.takes(message))
			}),
			_ => self.has_debuggers() || self.hold.is_some()
		};
		if !wanted || self.ending() {
			return None;
		}

		self.state = Some(State::new());

		let proc = unsafe { (*(*(*_ctx).proc_instance())).proc };
		let offset = unsafe { *(*_ctx).bytecode_offset() };
		match reason {
			BreakpointReason::Breakpoint => {
				if !self.breakpoint_hit((proc, offset)) {
					self.state = None;
					return None;
				}
			}

			BreakpointReason::Step | BreakpointReason::Pause if crate::instruction_hooking::is_breakpoint(_ctx) => {
				self.breakpoint_hit((proc, offset));
			}

			_ => {}
		}

		let at = unsafe {
			let proc = Proc::from_id((*(*(*_ctx).proc_instance())).proc);
			InstructionRef {
				proc: proc.as_ref().map(unnamed::proc_ref).unwrap_or(ProcRef {
					path: String::new(),
					override_id: 0
				}),
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

		let mut unattended: Option<Instant> = None;
		loop {
			let mut wait = if self.hold.is_some() { Wait::Forever } else { Wait::WhileClients };
			if wait == Wait::WhileClients && !self.has_debuggers() {
				break;
			}
			if let (Wait::Forever, Some(max)) = (wait, self.hold_max) {
				if self.has_debuggers() {
					wait = Wait::Until(None);
				} else {
					wait = Wait::Until(Some(*unattended.get_or_insert_with(Instant::now) + max));
				}
			}
			let request = match self.recv_request(wait) {
				Some(request) => request,
				None => match wait {
					Wait::Until(Some(until)) if Instant::now() >= until => {
						self.let_hold_go();
						return Some(ContinueKind::Continue);
					}
					Wait::Until(_) => continue,
					_ => break
				}
			};

			if let Request::Continue { kind } = request {
				let position = |thread: u32| self.stack_position(thread).unwrap_or(0) as u32;
				let kind = match kind {
					ContinueKind::Continue => ContinueKind::Continue,
					ContinueKind::StepOver { stack_id } => ContinueKind::StepOver {
						stack_id: position(stack_id)
					},
					ContinueKind::StepInto { stack_id } => ContinueKind::StepInto {
						stack_id: position(stack_id)
					},
					ContinueKind::StepOut { stack_id } => ContinueKind::StepOut {
						stack_id: position(stack_id)
					},
					ContinueKind::StepOverInstruction { stack_id } => ContinueKind::StepOverInstruction {
						stack_id: position(stack_id)
					},
					ContinueKind::StepIntoInstruction { stack_id } => ContinueKind::StepIntoInstruction {
						stack_id: position(stack_id)
					}
				};
				self.answer(Response::Ack);
				self.broadcast(Response::Resumed, self.current);
				self.state = None;
				self.paused = None;
				self.hold = None;
				self.hold_max = None;
				self.stepper = if kind == ContinueKind::Continue { None } else { self.current };
				return Some(kind);
			}

			if let Request::Eval { frame_id, command, context } = request {
				self.handle_eval_request(frame_id, &command, context.as_deref());
				if self.ending() {
					self.broadcast(Response::Resumed, None);
					self.state = None;
					self.paused = None;
					self.hold = None;
					self.hold_max = None;
					self.stepper = None;
					return Some(ContinueKind::Continue);
				}
				self.state.as_mut().unwrap().invalidate_stacks();
				continue;
			}
			if let Request::Render { frame_id, expression, depth } = request {
				self.handle_render(frame_id, &expression, depth);
				self.state.as_mut().unwrap().invalidate_stacks();
				continue;
			}
			if let Request::EvalOn { object, expression, depth } = request {
				self.handle_eval_on(&object, &expression, depth);
				self.state.as_mut().unwrap().invalidate_stacks();
				continue;
			}

			self.handle_request(request);
		}

		self.broadcast(Response::Resumed, None);
		self.state = None;
		self.paused = None;
		Some(ContinueKind::Continue)
	}

	fn let_hold_go(&mut self) {
		let seconds = self.hold_max.map_or(0.0, |max| max.as_secs_f64());
		eprintln!("Debug Server: resumed after {} seconds without a debugger", seconds);
		self.hold = None;
		self.hold_max = None;
		self.state = None;
		self.paused = None;
		self.journal(JournalEvent::HoldEnded { seconds }, None);
		self.broadcast(Response::Resumed, None);
	}

	pub fn step_ended(&mut self) {
		let Some(id) = self.stepper.take() else {
			return;
		};
		if let Some(client) = self.clients.get_mut(&id) {
			if !Self::deliver(client, &Response::StepEnded) {
				self.remove_client(id);
			}
		}
	}

	pub fn process(&mut self) -> bool {
		let mut should_pause = false;

		waker::alive();
		if waker::woke() {
			self.notify("Waking world...");
		}

		if waker::events_pending() {
			while let Some(request) = self.recv_request(Wait::No) {
				should_pause |= self.handle_request(request);
			}
		}

		should_pause
	}

	pub fn process_until_configured(&mut self) {
		self.waiting_to_start = true;
		self.resource_ids.clear();
		self.resource_origins.clear();
		while let Some(request) = self.recv_request(Wait::Forever) {
			if let Request::Configured = request {
				self.answer(Response::Ack);
				break;
			}

			self.handle_request(request);
		}
		self.waiting_to_start = false;
	}

	fn answer(&mut self, response: Response) {
		let Some(request) = self.answering.take() else {
			return;
		};
		let Some(id) = self.current else {
			return;
		};
		let Some(client) = self.clients.get_mut(&id) else {
			return;
		};

		if !Self::post(client, Some(request), &response) {
			eprintln!("Debug server failed to send a message to a client");
			self.remove_client(id);
		}
	}

	fn broadcast(&mut self, response: Response, except: Option<ClientId>) {
		let mut gone = vec![];
		for (&id, client) in &mut self.clients {
			if Some(id) != except && !Self::deliver(client, &response) {
				gone.push(id);
			}
		}
		for id in gone {
			self.remove_client(id);
		}
	}

	fn remove_client(&mut self, id: ClientId) {
		let Some(client) = self.clients.remove(&id) else {
			return;
		};
		drop(client);
		if self.current == Some(id) {
			self.current = None;
			self.answering = None;
		}
		if self.hold == Some(id) && self.paused.is_none() {
			self.hold = None;
			self.hold_max = None;
		}

		self.forget_watches_of(id);
		if self.profiling == Some(id) {
			self.profiling = None;
			crate::profiler::discard();
		}

		let mut unclaimed = vec![];
		for (key, claims) in &mut self.breakpoints {
			claims.retain(|(client, _), _| *client != id);
			if claims.is_empty() {
				unclaimed.push(*key);
			}
		}
		self.moved_breakpoints.retain(|(.., (client, _)), _| *client != id);
		for (proc, offset) in unclaimed {
			self.breakpoints.remove(&(proc, offset));
			if let Some(proc) = Proc::from_id(proc) {
				let _ = unhook_instruction(&proc, offset as u32);
			}
		}

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

	fn deliver(client: &mut Client, response: &Response) -> bool {
		Self::post(client, None, response)
	}

	fn post(client: &mut Client, reply_to: Option<RequestId>, response: &Response) -> bool {
		auxtools_protocol::frame(&ServerMessage { reply_to, response }).is_ok_and(|bytes| client.outbox.try_send(bytes).is_ok())
	}

	fn say(connection: &mut Connection, response: &Response) -> std::io::Result<()> {
		auxtools_protocol::write(connection, &ServerMessage::event(response))
	}

	fn writer(mut connection: Connection) -> (mpsc::SyncSender<Vec<u8>>, thread::JoinHandle<()>) {
		let (outbox, sent) = mpsc::sync_channel::<Vec<u8>>(OUTBOX);
		let writer = thread::spawn(move || {
			while let Ok(bytes) = sent.recv() {
				if connection.write_all(&bytes).and_then(|()| connection.flush()).is_err() {
					return;
				}
			}
			let _ = Self::say(&mut connection, &Response::Disconnect);
		});
		(outbox, writer)
	}

	pub fn world_carries_on(&mut self) {
		self.ending = None;
	}

	fn ending(&self) -> bool {
		self.ending.is_some_and(|since| since.elapsed() < ENDING_LIMIT)
	}

	pub fn world_shutting_down(&mut self) {
		self.broadcast(
			Response::Notification {
				message: WORLD_SHUTTING_DOWN.to_owned()
			},
			None
		);
		let writers: Vec<_> = self.clients.values_mut().filter_map(|client| client.writer.take()).collect();
		self.disconnect_all();
		let deadline = Instant::now() + Duration::from_secs(1);
		while writers.iter().any(|writer| !writer.is_finished()) && Instant::now() < deadline {
			thread::sleep(Duration::from_millis(5));
		}
	}

	fn journal(&mut self, event: JournalEvent, from: Option<ClientId>) {
		let entry = journal::add(event);
		let mut gone = vec![];
		for (&id, client) in &mut self.clients {
			let follows = client.follow.as_ref().is_some_and(|kinds| journal::wanted(&entry, kinds));
			if follows && Some(id) != from && !Self::deliver(client, &Response::Journaled(entry.clone())) {
				gone.push(id);
			}
		}
		for id in gone {
			self.remove_client(id);
		}
	}

	fn journal_stack(&self) -> Vec<JournalFrame> {
		type Known = HashMap<(raw_types::procs::ProcId, u16, usize), JournalFrame>;
		static KNOWN: Mutex<(u64, Option<Known>)> = Mutex::new((0, None));
		const KEEP: usize = 16384;

		let mut uncached: (u64, Option<Known>) = (0, None);
		let mut guard = match KNOWN.try_lock() {
			Ok(guard) => Some(guard),
			Err(std::sync::TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
			Err(std::sync::TryLockError::WouldBlock) => None
		};
		let (changes, known) = match guard.as_mut() {
			Some(guard) => &mut **guard,
			None => &mut uncached
		};
		let now = auxtools::bytecode_changes();
		if *changes != now || known.as_ref().is_some_and(|known| known.len() >= KEEP) {
			*changes = now;
			*known = None;
		}
		let known = known.get_or_insert_with(HashMap::new);

		let mut frames: Vec<(JournalFrame, bool)> = vec![];
		let mut ctx = unsafe { *raw_types::funcs::CURRENT_EXECUTION_CONTEXT };
		while !ctx.is_null() {
			let (id, offset, running) = unsafe { ((**(*ctx).proc_instance()).proc, *(*ctx).bytecode_offset(), *(*ctx).bytecode()) };
			let Some(proc) = Proc::from_id(id) else {
				break;
			};
			let helper = crate::launched::runs_spawns(&proc);
			let (current, length) = unsafe { proc.bytecode_mut_ptr() };
			let breakpoint =
				(offset as u32) < length as u32 && crate::instruction_hooking::is_breakpoint_word(unsafe { *current.add(offset as usize) });
			let plain = std::ptr::eq(running, current) && !breakpoint;
			let key = (id, offset, running as usize);
			let frame = match known.get(&key).filter(|_| plain) {
				Some(frame) => frame.clone(),
				None => {
					let (file, line) = Self::place_running(&proc, ctx, offset, false);
					let frame = JournalFrame {
						proc: unnamed::proc_name(&proc),
						file,
						line
					};
					if plain && !frame.proc.starts_with('(') {
						known.insert(key, frame.clone());
					}
					frame
				}
			};
			frames.push((frame, helper));
			ctx = unsafe { *(*ctx).parent_context() };
		}

		let shown = match frames.iter().rposition(|(_, helper)| !helper) {
			Some(last) => last + 1,
			None => frames.len()
		};
		frames.truncate(shown.min(JOURNAL_FRAMES));
		frames.into_iter().map(|(frame, _)| frame).collect()
	}

	fn handle_journal(&mut self, after: u64, kinds: Vec<JournalKind>, limit: Option<u32>, follow: bool) {
		let (entries, first, last) = journal::read(after, &kinds, limit);
		if let Some(client) = self.current.and_then(|id| self.clients.get_mut(&id)) {
			client.follow = follow.then_some(kinds);
		}
		self.answer(Response::Journal { entries, first, last });
	}
}

fn background_known() -> bool {
	auxtools::version::get().0 >= 516
}

impl Drop for Server {
	fn drop(&mut self) {
		self.disconnect_all();

		if let Some(acceptor) = ACCEPTOR.lock().unwrap().as_mut() {
			let (events, waiting) = mpsc::channel();
			acceptor.events = events;
			acceptor.waiting = Some(waiting);
		}

		while let Ok(event) = self.events.try_recv() {
			if let Event::Connected(_, mut connection) = event {
				thread::spawn(move || Self::say(&mut connection, &Response::Disconnect).is_ok());
			}
		}
	}
}

const MAX_CALLEE_ERRORS: usize = 5;

pub(crate) fn is_expression_frame(ctx: *mut raw_types::procs::ExecutionContext) -> bool {
	let id = unsafe { (**(*ctx).proc_instance()).proc };
	["/proc/auxtools_expr_stub", "/proc/auxtools_spawn_stub"]
		.into_iter()
		.any(|path| Proc::find(path).is_some_and(|stub| stub.id == id))
}

impl ServerThread {
	fn run(&self, id: ClientId, mut stream: Connection) {
		waker::client_connected();

		while let Ok(ClientMessage { id: request_id, request }) = auxtools_protocol::read(&mut stream) {
			if let Request::Disconnect = request {
				break;
			}

			waker::request_waiting();
			if let Err(e) = self.events.send(Event::Request(id, request_id, request)) {
				eprintln!("Debug server thread failed to handle request: {}", e);
				break;
			}
			waker::event_sent();
		}

		waker::client_left();
		let _ = self.events.send(Event::Disconnected(id));
		waker::event_sent();
	}
}

fn local_uses(code: &[u32]) -> std::sync::Arc<HashMap<u32, (u32, u32)>> {
	use std::hash::{Hash, Hasher};
	type Uses = std::sync::Arc<HashMap<u32, (u32, u32)>>;
	static USES: std::sync::LazyLock<Mutex<HashMap<(u64, usize), Uses>>> = std::sync::LazyLock::new(Default::default);
	const KEEP: usize = 1024;

	fn locals_in(var: &dmasm::Variable, found: &mut Vec<u32>) {
		match var {
			dmasm::Variable::Local(slot) => found.push(*slot),
			dmasm::Variable::SetCache(a, b) => {
				locals_in(a, found);
				locals_in(b, found);
			}
			dmasm::Variable::Initial(inner) | dmasm::Variable::IsSaved(inner) => locals_in(inner, found),
			_ => {}
		}
	}
	let key = {
		let mut hasher = std::collections::hash_map::DefaultHasher::new();
		code.hash(&mut hasher);
		(hasher.finish(), code.len())
	};
	if let Some(uses) = USES.lock().unwrap().get(&key) {
		return uses.clone();
	}
	let (nodes, _) = dmasm::disassembler::disassemble(code, &mut disassemble_env::DisassembleEnv);
	let mut uses: HashMap<u32, (u32, u32)> = HashMap::new();
	for node in &nodes {
		let dmasm::Node::Instruction(ins, debug) = node else { continue };
		let mut found = vec![];
		for var in ins.variables() {
			locals_in(var, &mut found);
		}
		for slot in found {
			let at = debug.offset as u32;
			uses.entry(slot).and_modify(|(_, last)| *last = at).or_insert((at, at));
		}
	}
	let uses = std::sync::Arc::new(uses);
	let mut kept = USES.lock().unwrap();
	if kept.len() >= KEEP {
		kept.clear();
	}
	kept.insert(key, uses.clone());
	uses
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
			assert!(server.recv_request(super::Wait::No).is_none());
			assert_eq!(server.clients.len(), 2);

			drop(server);
			let _ = done.send((first, second));
		});

		let (mut first, mut second) = dropped
			.recv_timeout(Duration::from_secs(5))
			.expect("server drop blocked on client connections");
		for client in [&mut first, &mut second] {
			let mut len = [0u8; 4];
			client.read_exact(&mut len).expect("no Disconnect");
		}

		let _third = Connection::connect(&connection::world_name()).expect("listener unavailable between server instances");
		thread::sleep(Duration::from_millis(200));
		let mut server = Server::listen().expect("the next server couldn't listen");
		assert!(server.recv_request(super::Wait::No).is_none());
		assert_eq!(server.clients.len(), 1);
		assert!(Server::listen().is_err(), "multiple active servers accepted");
	}
}
