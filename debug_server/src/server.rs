use std::{
	borrow::Cow,
	cell::{OnceCell, RefCell},
	collections::{BTreeMap, HashMap, VecDeque},
	io::Write,
	sync::{
		atomic::{AtomicU64, Ordering},
		mpsc, Arc, Mutex
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

use super::instruction_hooking::{hook_instruction, original_code, unhook_all, unhook_instruction};
use crate::{
	code_index::{self, Code},
	connection::{self, Connection},
	console, hot_reload, journal, resource_reload, unnamed, waker
};

mod code;
mod profile;
mod render;
mod sessions;
mod values;
mod watches;

use code::Codes;
use sessions::{ClientId, Sessions, Wait};
use values::Variables;

static NEXT_SPAWN: AtomicU64 = AtomicU64::new(1);

struct Spawn {
	number: u64,
	bytecode: Vec<u32>,
	locals: Vec<String>,
	args: Vec<Value>
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

const EVAL_CODE_KEPT: usize = 256;

#[derive(PartialEq, Eq, Hash)]
struct EvalCodeKey {
	generation: u32,
	command: String,
	names: Vec<String>,
	src_type: Option<String>
}

pub struct Server {
	sessions: Sessions,
	state: Option<State>,
	in_eval: bool,
	eval_error: Option<String>,
	eval_still_running: bool,
	eval_callee_errors: Vec<String>,
	eval_callee_more: usize,
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
	stepper: Option<ClientId>,
	moved_breakpoints: hot_reload::MovedBreakpoints<(ClientId, Placed)>,
	codes: Codes,
	waiting_reloads: Vec<HotReloadRequest>
}

impl Server {
	pub fn listen() -> std::io::Result<Server> {
		Ok(Server {
			sessions: Sessions::listen()?,
			state: None,
			in_eval: false,
			eval_error: None,
			eval_still_running: false,
			eval_callee_errors: vec![],
			eval_callee_more: 0,
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
			stepper: None,
			codes: Codes::new(),
			waiting_reloads: vec![]
		})
	}

	pub fn refuse(reason: String) {
		Sessions::refuse(reason);
	}

	pub fn give_up(&mut self, reason: String) {
		self.sessions.give_up(reason);
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
		let Some(client) = self.sessions.current else {
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
		match (proc.as_ref().and_then(code_index::file_of), line) {
			(Some(file), Some(line)) => place += &format!("  {}:{}", file, line),
			(None, Some(line)) => place += &format!("  line {}", line),
			_ => {}
		}
		place
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

	fn breakpoint_offset(proc: &Proc, offset: u32) -> u32 {
		if offset == 0 {
			code_index::of(proc).entry_offset()
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
		let (Some(proc), Some(client)) = (unnamed::find(&instruction.proc.path, instruction.proc.override_id), self.sessions.current) else {
			self.answer(Response::BreakpointSet {
				result: BreakpointSetResult::Failed
			});
			return;
		};
		let placed = Placed::of(instruction.offset, exactly);
		if !exactly {
			instruction.offset = Self::breakpoint_offset(&proc, instruction.offset);
		}
		let (file, line) = Codes::breakpoint_place(&proc, instruction.offset);

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
						Ok(value) => message.push_str(&value.as_string().unwrap_or_else(|_| values::stringify(&value))),
						Err(error) => message.push_str(&format!("{{{}: {}}}", expression, error))
					}
					self.state.as_mut().unwrap().invalidate_stacks();
				}
			}
		}
		message
	}

	fn handle_breakpoint_unset(&mut self, instruction: InstructionRef, exactly: bool) {
		let (Some(proc), Some(client)) = (unnamed::find(&instruction.proc.path, instruction.proc.override_id), self.sessions.current) else {
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
						let (file, line) = self.codes.frame_place(&stack[0], position != 0);
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
					frames.push(self.codes.frame_info(&stack[i], id, stack_id != 0 && i == 0));
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

	fn handle_command(&mut self, frame_id: Option<u32>, command: &str) -> String {
		match console::parse(command) {
			Ok(console::Command::Disassemble { proc: Some(proc), id }) => self.codes.handle_disassemble(proc, id),
			Ok(console::Command::Disassemble { proc: None, .. }) => match frame_id {
				Some(frame_id) => match self.get_stack_frame(frame_id) {
					Some(frame) => {
						let proc = unnamed::proc_ref(&frame.proc);
						self.codes.handle_disassemble(&proc.path, proc.override_id)
					}
					None => "couldn't find stack frame (is execution not paused?)".to_owned()
				},
				None => "no execution frame selected".to_owned()
			},
			#[cfg(feature = "test-commands")]
			Ok(console::Command::Skin) => match self.evaluating(crate::skin::state) {
				Ok(None) => "no skin".to_owned(),
				Ok(Some((resource, text))) => format!("resource {}\n{}", resource, text),
				Err(e) => format!("Failed: {}", e)
			},
			#[cfg(feature = "test-commands")]
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
		self.eval_still_running = false;

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

				for (idx, (local, name)) in frame.locals.iter().zip(values::visible_locals(&frame)).enumerate() {
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
			src_type: src.as_ref().and_then(values::kind)
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
					self.eval_still_running = true;
					Err(
						"evaluation aborted: BYOND moved the code to the background, so its result is unavailable. This can happen after about a \
						 million loop iterations (world.loop_checks), or with `set background = 1`. The code is still running. Use `eval --spawn \
						 --wait` to wait for a result"
							.to_owned()
					)
				} else if !background_known() {
					self.eval_still_running = true;
					Err(
						"evaluation aborted because the code slept or BYOND moved it to the background, so its result is unavailable. BYOND does \
						 that after about a million loop iterations (world.loop_checks), or with `set background = 1`. Use `eval --spawn --wait` to \
						 wait for a result"
							.to_owned()
					)
				} else {
					self.eval_still_running = true;
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

	fn handle_eval_request(&mut self, frame_id: Option<u32>, command: &str, ending: bool) {
		if ending {
			self.ending = Some(Instant::now());
		}
		let still_running = self.handle_eval(frame_id, command);
		if ending && !still_running {
			self.ending = None;
		}
	}

	fn handle_eval(&mut self, frame_id: Option<u32>, command: &str) -> bool {
		#[cfg(feature = "test-commands")]
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
		let response = self
			.eval_expr(frame_id, command)
			.map(|result| EvalResponse {
				value: values::stringify(&result),
				variables: self.value_to_variables_ref(&result, reached),
				kind: values::kind(&result),
				entries: values::entries(&result)
			})
			.map_err(|message| self.eval_failed(message));
		let still_running = matches!(&response, Err(error) if error.still_running);
		self.send_callee_errors();
		self.answer(Response::Eval(response));
		still_running
	}

	fn eval_failed(&self, message: String) -> EvalError {
		EvalError {
			message,
			still_running: self.eval_still_running
		}
	}

	fn handle_eval_on(&mut self, object: &str, expression: &str, depth: Option<u32>) {
		self.eval_still_running = false;
		let value = self.object_of(object).and_then(|object| {
			self.eval_src = Some(object);
			self.eval_expr(None, expression)
		});
		let response = match depth {
			Some(depth) => Response::Render(value.map(|value| render::render(&value, depth))),
			None => Response::Eval(
				value
					.map(|result| EvalResponse {
						value: values::stringify(&result),
						variables: self.value_to_variables_ref(&result, None),
						kind: values::kind(&result),
						entries: values::entries(&result)
					})
					.map_err(|message| self.eval_failed(message))
			)
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
					text: values::stringify(&value),
					rendered: render::render(&value, 1)
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
		if let Some(why) = self.sessions.reloads_refused() {
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
					let (globals, others) = request.plan.new_vars.iter().cloned().partition(|var| var.type_path.is_empty());
					let (init, left) = crate::hot_reload::early_global_init(init, request);
					if left > 0 {
						early.push(format!(
							"(global init): {} new static vars will initialize with the deferred reload at world/New",
							left
						));
					}
					let alone = HotReloadRequest {
						global_init: Some(init),
						plan: dmasm::reload::Plan {
							new_vars: globals,
							..Default::default()
						},
						..Default::default()
					};
					if crate::world_resources::not_built(&alone).is_empty() {
						let result = self.apply_hot_reload(&alone);
						if result.applied {
							waiting.global_init = None;
							waiting.plan.new_vars = others;
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
		let prepared = hot_reload::add_types(&request.plan.new_types).and_then(|added| {
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
			self.broadcast(Response::CodeChanged, self.sessions.current);
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

	fn handle_request(&mut self, request: Request) -> bool {
		auxtools::instruction_hook::hear_all();
		match request {
			Request::Disconnect => unreachable!(),
			Request::CatchRuntimes { uncaught, caught } => {
				if let Some(client) = self.sessions.current.and_then(|id| self.sessions.clients.get_mut(&id)) {
					client.uncaught = uncaught;
					client.caught = caught;
				}
			}
			Request::BreakpointSet {
				instruction,
				condition,
				hit,
				log,
				exact
			} => self.handle_breakpoint_set(instruction, condition, hit, log, exact),
			Request::BreakpointUnset { instruction, exact } => self.handle_breakpoint_unset(instruction, exact),
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
			Request::Eval { frame_id, command, ending } => self.handle_eval_request(frame_id, &command, ending),

			Request::StackFrames {
				stack_id,
				start_frame,
				count
			} => self.handle_stack_frames(stack_id, start_frame, count),

			Request::Offset { proc, line } => {
				self.answer(Response::Offset {
					offset: self.codes.get_offset(proc, line)
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
				let claimed = self.sessions.claim_reloads(who);
				self.answer(Response::Reloader(claimed));
			}
			Request::Skin(request) => {
				let result = match self.sessions.reloads_refused() {
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
				let result = match self.sessions.reloads_refused() {
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
				self.sessions.hold = self.sessions.current;
				self.sessions.hold_max = self
					.sessions
					.current
					.and_then(|id| self.sessions.clients.get(&id))
					.and_then(|client| client.hold_max);
				self.answer(Response::Ack);
			}

			Request::Locate { file, line } => {
				let instructions = self.codes.locate(&file, line);
				self.answer(Response::Locate { instructions });
			}

			Request::LinesWithCode { file, start, end } => {
				let lines = Codes::lines_with_code(&file, start, end);
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
				let mut files: Vec<String> = code_index::by_file().keys().cloned().collect();
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
				if let Some(client) = self.sessions.current.and_then(|id| self.sessions.clients.get_mut(&id)) {
					client.debugs = false;
				}
				self.answer(Response::Ack);
			}

			Request::Disassemble { code } => {
				let disassembly = self.codes.disassemble(code);
				self.answer(Response::Disassemble(disassembly));
			}

			Request::Journal { after, kinds, limit, follow } => self.handle_journal(after, kinds, limit, follow),
			Request::JournalAdd(event) => self.journal(event, self.sessions.current),
			Request::HoldMax { seconds } => {
				let hold_max = seconds
					.filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
					.map(Duration::from_secs_f64);
				if let Some(client) = self.sessions.current.and_then(|id| self.sessions.clients.get_mut(&id)) {
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
				let info = Codes::proc_info(&proc);
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

	pub fn recover_from_panic(&mut self) {
		eprintln!("Debug server panicked, dropping the debug clients");
		self.in_eval = false;
		self.waiting_to_start = false;
		self.state = None;
		self.paused = None;
		self.disconnect_all();
		self.sessions.profiling = None;
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
				.map(|frame| self.codes.frame_info(frame, 1, false));
			let logged = Proc::from_id(proc).map(|proc| {
				let place = unnamed::proc_ref(&proc);
				let (file, line) = Codes::place_of(&proc, offset as u32);
				Response::Log {
					message,
					file,
					line,
					proc: place,
					caller
				}
			});
			if let (Some(logged), Some(to)) = (logged, self.sessions.clients.get_mut(&client.0)) {
				if !Sessions::deliver(to, &logged) {
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
			Some(proc) => match Codes::place_of(&proc, key.1 as u32) {
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
			BreakpointReason::Runtime { message, caught } => self.sessions.clients.values().any(|client| {
				let filter = if *caught { &client.caught } else { &client.uncaught };
				filter.as_ref().is_some_and(|filter| filter.takes(message))
			}),
			_ => self.sessions.has_debuggers() || self.sessions.hold.is_some()
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
			let mut wait = if self.sessions.hold.is_some() {
				Wait::Forever
			} else {
				Wait::WhileClients
			};
			if wait == Wait::WhileClients && !self.sessions.has_debuggers() {
				break;
			}
			if let (Wait::Forever, Some(max)) = (wait, self.sessions.hold_max) {
				if self.sessions.has_debuggers() {
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
				self.broadcast(Response::Resumed, self.sessions.current);
				self.state = None;
				self.paused = None;
				self.sessions.hold = None;
				self.sessions.hold_max = None;
				self.stepper = if kind == ContinueKind::Continue { None } else { self.sessions.current };
				return Some(kind);
			}

			if let Request::Eval { frame_id, command, ending } = request {
				self.handle_eval_request(frame_id, &command, ending);
				if self.ending() {
					self.broadcast(Response::Resumed, None);
					self.state = None;
					self.paused = None;
					self.sessions.hold = None;
					self.sessions.hold_max = None;
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
		let seconds = self.sessions.hold_max.map_or(0.0, |max| max.as_secs_f64());
		eprintln!("Debug Server: resumed after {} seconds without a debugger", seconds);
		self.sessions.hold = None;
		self.sessions.hold_max = None;
		self.state = None;
		self.paused = None;
		self.journal(JournalEvent::HoldEnded { seconds }, None);
		self.broadcast(Response::Resumed, None);
	}

	pub fn step_ended(&mut self) {
		let Some(id) = self.stepper.take() else {
			return;
		};
		if let Some(client) = self.sessions.clients.get_mut(&id) {
			if !Sessions::deliver(client, &Response::StepEnded) {
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

	pub fn world_carries_on(&mut self) {
		self.ending = None;
	}

	fn ending(&self) -> bool {
		self.ending.is_some_and(|since| since.elapsed() < ENDING_LIMIT)
	}

	fn journal(&mut self, event: JournalEvent, from: Option<ClientId>) {
		let entry = journal::add(event);
		let mut gone = vec![];
		for (&id, client) in &mut self.sessions.clients {
			let follows = client.follow.as_ref().is_some_and(|kinds| journal::wanted(&entry, kinds));
			if follows && Some(id) != from && !Sessions::deliver(client, &Response::Journaled(entry.clone())) {
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
					let (file, line) = Codes::place_running(&proc, ctx, offset, false);
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
		if let Some(client) = self.sessions.current.and_then(|id| self.sessions.clients.get_mut(&id)) {
			client.follow = follow.then_some(kinds);
		}
		self.answer(Response::Journal { entries, first, last });
	}
}

fn background_known() -> bool {
	auxtools::version::get().0 >= 516
}

const MAX_CALLEE_ERRORS: usize = 5;

pub(crate) fn is_expression_frame(ctx: *mut raw_types::procs::ExecutionContext) -> bool {
	let id = unsafe { (**(*ctx).proc_instance()).proc };
	["/proc/auxtools_expr_stub", "/proc/auxtools_spawn_stub"]
		.into_iter()
		.any(|path| Proc::find(path).is_some_and(|stub| stub.id == id))
}

#[cfg(test)]
mod tests {
	use std::{sync::mpsc, thread, time::Duration};

	use auxtools_protocol::{Hello, Response, ServerMessage};

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
			assert_eq!(server.sessions.clients.len(), 2);

			drop(server);
			let _ = done.send((first, second));
		});

		let (mut first, mut second) = dropped
			.recv_timeout(Duration::from_secs(5))
			.expect("server drop blocked on client connections");
		for client in [&mut first, &mut second] {
			let hello: Hello = auxtools_protocol::read(client).expect("no hello");
			assert_eq!(hello, Hello::new(env!("CARGO_PKG_VERSION")));
			let told: ServerMessage = auxtools_protocol::read(client).expect("no Disconnect");
			assert!(
				matches!(told, ServerMessage {
					reply_to: None,
					response: Response::Disconnect
				}),
				"{:?}",
				told
			);
		}

		let _third = Connection::connect(&connection::world_name()).expect("listener unavailable between server instances");
		thread::sleep(Duration::from_millis(200));
		let mut server = Server::listen().expect("the next server couldn't listen");
		assert!(server.recv_request(super::Wait::No).is_none());
		assert_eq!(server.sessions.clients.len(), 1);
		assert!(Server::listen().is_err(), "multiple active servers accepted");
	}
}
