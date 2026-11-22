use std::{cell::UnsafeCell, collections::HashMap, sync::Mutex};

use auxtools::*;
use instruction_hooking::{disassemble_env, InstructionHook};
use lazy_static::lazy_static;

use auxtools_protocol::{BreakpointReason, ContinueKind};

use crate::{server::Server, DEBUG_SERVER};

const OPCODE_DBGFILE: u32 = 0x84;
const OPCODE_DBGLINE: u32 = 0x85;
const OPCODE_THROW: u32 = 0x12D;
const OPCODE_DEBUG_BREAK: u32 = 0x1337;
const OPCODE_DEBUG_OPERAND: u32 = 0x1338;

#[shutdown]
fn instruction_hooking_shutdown() {
	unhook_all();
}

#[derive(PartialEq, Eq, Copy, Clone)]
struct ProcInstanceRef(u16);

impl ProcInstanceRef {
	fn new(ptr: *mut raw_types::procs::ProcInstance) -> Self {
		unsafe {
			static mut PTR_REF_ID: u16 = 0x8000;
			PTR_REF_ID = if PTR_REF_ID == u16::MAX { 0x8001 } else { PTR_REF_ID + 1 };
			(*ptr).mega_hack = PTR_REF_ID;
			Self(PTR_REF_ID)
		}
	}

	fn is(&self, ptr: *mut raw_types::procs::ProcInstance) -> bool {
		unsafe { self.0 == (*ptr).mega_hack }
	}

	unsafe fn is_frame(&self, ctx: *mut raw_types::procs::ExecutionContext) -> bool {
		unsafe { self.is(*(*ctx).proc_instance()) && !self.is_copy(ctx) }
	}

	unsafe fn is_copy(&self, ctx: *mut raw_types::procs::ExecutionContext) -> bool {
		unsafe {
			let mut at = *(*ctx).parent_context();
			while !at.is_null() {
				if self.is(*(*at).proc_instance()) {
					return true;
				}
				at = *(*at).parent_context();
			}
			false
		}
	}

	unsafe fn forget_copy(&self, ctx: *mut raw_types::procs::ExecutionContext) {
		unsafe {
			let instance = *(*ctx).proc_instance();
			if self.is(instance) && self.is_copy(ctx) {
				(*instance).mega_hack = 0;
			}
		}
	}
}

#[derive(Copy, Clone)]
enum DebuggerAction {
	None,
	Pause,
	StepOver {
		target: ProcInstanceRef,
		caller: Option<ProcInstanceRef>
	},
	StepInto {
		parent: ProcInstanceRef,
		caller: Option<ProcInstanceRef>
	},
	BreakOnNext,
	StepOut {
		target: ProcInstanceRef
	},
	StepOverInstruction {
		target: ProcInstanceRef,
		caller: Option<ProcInstanceRef>
	},
	StepIntoInstruction {
		parent: ProcInstanceRef,
		caller: Option<ProcInstanceRef>
	}
}

static mut CURRENT_ACTION: DebuggerAction = DebuggerAction::None;

static mut LAST_FRAME: *mut raw_types::procs::ExecutionContext = std::ptr::null_mut();
static mut LAST_OPCODE: u32 = 0;

static mut PAUSE_ASKED: Option<std::time::Instant> = None;
const PAUSE_PATIENCE: std::time::Duration = std::time::Duration::from_secs(1);

static mut DEFERRED_INSTRUCTION_REPLACE: UnsafeCell<Option<(Vec<u32>, *mut u32)>> = UnsafeCell::new(None);

#[derive(PartialEq, Eq, Hash)]
struct PtrKey(usize);

impl PtrKey {
	fn new(ptr: *mut u32) -> Self {
		unsafe { Self(std::mem::transmute(ptr)) }
	}
}

lazy_static! {
	static ref ORIGINAL_BYTECODE: Mutex<HashMap<PtrKey, Vec<u32>>> = Mutex::new(HashMap::new());
}

fn is_generated_proc(ctx: *mut raw_types::procs::ExecutionContext) -> bool {
	unsafe {
		let instance = *(*ctx).proc_instance();
		if let Some(proc) = Proc::from_id((*instance).proc) {
			return proc.path.ends_with("(init)");
		}
	}

	true
}

fn get_proc_ctx(stack_id: u32) -> (*mut raw_types::procs::ExecutionContext, *mut raw_types::procs::ExecutionContext) {
	unsafe {
		let running = *raw_types::funcs::CURRENT_EXECUTION_CONTEXT;
		let running = (running, *(*running).parent_context());
		if stack_id == 0 {
			return running;
		}

		let buffer = (*raw_types::funcs::SUSPENDED_PROCS_BUFFER).buffer;
		let std::ops::Range { start: front, end: back } = raw_types::funcs::suspended_range();

		if back - front < stack_id as usize {
			return running;
		}

		let instance = *buffer.add(front + (stack_id - 1) as usize);
		let mut caller = std::ptr::null_mut();
		let mut ctx = (*instance).context;
		loop {
			let called = *(*ctx).parent_context();
			if called.is_null() {
				return (ctx, caller);
			}
			caller = ctx;
			ctx = called;
		}
	}
}

unsafe fn mark(ctx: *mut raw_types::procs::ExecutionContext) -> Option<ProcInstanceRef> {
	if ctx.is_null() {
		None
	} else {
		Some(ProcInstanceRef::new(unsafe { *(*ctx).proc_instance() }))
	}
}

unsafe fn mark_caller(ctx: *mut raw_types::procs::ExecutionContext) -> Option<ProcInstanceRef> {
	if ctx.is_null() || Proc::from_id(unsafe { (**(*ctx).proc_instance()).proc }).is_some_and(|proc| crate::launched::runs_spawns(&proc)) {
		return None;
	}
	unsafe { mark(ctx) }
}

fn handle_breakpoint(ctx: *mut raw_types::procs::ExecutionContext, reason: BreakpointReason) -> Option<DebuggerAction> {
	let action = unsafe {
		match &mut *DEBUG_SERVER.get() {
			Some(server) => server.handle_breakpoint(ctx, reason)?,
			None => return None
		}
	};

	Some(match action {
		ContinueKind::Continue => DebuggerAction::None,
		ContinueKind::StepOver { stack_id } => {
			let (ctx, caller) = get_proc_ctx(stack_id);
			DebuggerAction::StepOver {
				target: ProcInstanceRef::new(unsafe { *(*ctx).proc_instance() }),
				caller: unsafe { mark_caller(caller) }
			}
		}
		ContinueKind::StepInto { stack_id } => {
			let (ctx, caller) = get_proc_ctx(stack_id);
			DebuggerAction::StepInto {
				parent: ProcInstanceRef::new(unsafe { *(*ctx).proc_instance() }),
				caller: unsafe { mark_caller(caller) }
			}
		}
		ContinueKind::StepOverInstruction { stack_id } => {
			let (ctx, caller) = get_proc_ctx(stack_id);
			DebuggerAction::StepOverInstruction {
				target: ProcInstanceRef::new(unsafe { *(*ctx).proc_instance() }),
				caller: unsafe { mark_caller(caller) }
			}
		}
		ContinueKind::StepIntoInstruction { stack_id } => {
			let (ctx, caller) = get_proc_ctx(stack_id);
			DebuggerAction::StepIntoInstruction {
				parent: ProcInstanceRef::new(unsafe { *(*ctx).proc_instance() }),
				caller: unsafe { mark_caller(caller) }
			}
		}
		ContinueKind::StepOut { stack_id } => {
			let (_, caller) = get_proc_ctx(stack_id);
			match unsafe { mark_caller(caller) } {
				Some(target) => DebuggerAction::StepOut { target },
				None => {
					step_ended();
					DebuggerAction::None
				}
			}
		}
	})
}

fn step_ended() {
	unsafe {
		if let Some(server) = &mut *DEBUG_SERVER.get() {
			server.step_ended();
		}
	}
}

fn proc_instance_is_in_stack(mut ctx: *mut raw_types::procs::ExecutionContext, proc_ref: ProcInstanceRef) -> bool {
	unsafe {
		let mut found = false;

		while !ctx.is_null() {
			if proc_ref.is(*(*ctx).proc_instance()) {
				found = true;
				break;
			}
			ctx = *(*ctx).parent_context();
		}

		found
	}
}

fn proc_instance_is_suspended(proc_ref: ProcInstanceRef) -> bool {
	unsafe {
		let buffer = (*raw_types::funcs::SUSPENDED_PROCS_BUFFER).buffer;
		let mut found = false;

		for x in raw_types::funcs::suspended_range() {
			let instance = *buffer.add(x);

			if proc_instance_is_in_stack((*instance).context, proc_ref) {
				found = true;
				break;
			}
		}

		found
	}
}

#[runtime_handler]
fn handle_runtime(error: &str) {
	let _ = crate::guard(|| unsafe {
		let ctx = *raw_types::funcs::CURRENT_EXECUTION_CONTEXT;

		if let Some(server) = &mut *DEBUG_SERVER.get() {
			if server.is_in_eval() {
				if !in_try_above_expression(ctx) {
					server.set_eval_error(error, ctx);
				}
				return;
			}
		}

		if let Some(server) = &mut *DEBUG_SERVER.get() {
			server.spawn_runtime(error, ctx);
		}

		let reason = BreakpointReason::Runtime {
			message: error.to_string(),
			caught: in_try(ctx)
		};
		if let Some(action) = handle_breakpoint(ctx, reason) {
			CURRENT_ACTION = action;
			auxtools::instruction_hook::hear_all();
		}
	});
}

unsafe fn in_try(mut ctx: *mut raw_types::procs::ExecutionContext) -> bool {
	unsafe {
		while !ctx.is_null() {
			if *(*ctx).throw_handler() != 0 {
				return true;
			}
			ctx = *(*ctx).parent_context();
		}
	}
	false
}

unsafe fn in_try_above_expression(mut ctx: *mut raw_types::procs::ExecutionContext) -> bool {
	unsafe {
		while !ctx.is_null() {
			if *(*ctx).throw_handler() != 0 {
				return true;
			}
			if crate::server::is_expression_frame(ctx) {
				return false;
			}
			ctx = *(*ctx).parent_context();
		}
	}
	false
}

unsafe fn thrown(ctx: *mut raw_types::procs::ExecutionContext) -> String {
	let size = unsafe { *(*ctx).stack_size() } as usize;
	if size == 0 {
		return "an exception".to_owned();
	}
	let value = unsafe { Value::from_raw(*(*(*ctx).stack()).add(size - 1)) };
	if let Ok(name) = value.get(byond_string!("name")) {
		if let Ok(text) = name.as_string() {
			return text;
		}
		if name != Value::NULL {
			if let Ok(text) = name.to_string() {
				return text;
			}
		}
	}
	match value.as_string() {
		Ok(text) => text,
		Err(_) => value.to_string().unwrap_or_else(|_| "an exception".to_owned())
	}
}

pub(crate) fn is_breakpoint_word(word: u32) -> bool {
	matches!(word, OPCODE_DEBUG_BREAK | OPCODE_DEBUG_OPERAND)
}

pub(crate) fn is_breakpoint(ctx: *mut raw_types::procs::ExecutionContext) -> bool {
	unsafe { *(*(*ctx).bytecode()).add(*(*ctx).bytecode_offset() as usize) == OPCODE_DEBUG_BREAK }
}

unsafe fn in_helper(ctx: *mut raw_types::procs::ExecutionContext) -> bool {
	let id = unsafe { (**(*ctx).proc_instance()).proc };
	Proc::from_id(id).is_some_and(|proc| crate::launched::is_helper(&proc))
}

#[inline(always)]
pub(crate) fn passes(server: &Server, ctx: *mut raw_types::procs::ExecutionContext) -> bool {
	unsafe {
		let opcode = *(*(*ctx).bytecode()).add(*(*ctx).bytecode_offset() as usize);
		if opcode == OPCODE_DEBUG_BREAK
			|| opcode == OPCODE_THROW
			|| !matches!(CURRENT_ACTION, DebuggerAction::None)
			|| (*DEFERRED_INSTRUCTION_REPLACE.get()).is_some()
			|| server.is_in_eval()
			|| server.has_watches()
			|| crate::profiler::running()
			|| crate::waker::wants_world()
		{
			return false;
		}
		crate::waker::alive();
		LAST_FRAME = ctx;
		LAST_OPCODE = opcode;
		true
	}
}

pub(crate) fn rest() {
	if !auxtools::instruction_hook::hear_only(&[OPCODE_THROW]) {
		return;
	}
	unsafe {
		LAST_FRAME = std::ptr::null_mut();
	}
	if crate::waker::wants_world() {
		auxtools::instruction_hook::hear_all();
	}
}

impl InstructionHook for Server {
	fn handle_instruction(&mut self, ctx: *mut raw_types::procs::ExecutionContext) {
		unsafe {
			let deferred = DEFERRED_INSTRUCTION_REPLACE.get();
			if let Some((src, dst)) = &*deferred {
				std::ptr::copy_nonoverlapping(src.as_ptr(), *dst, src.len());
				*deferred = None;
			}
		}

		let server = self;
		let in_eval = server.is_in_eval();
		if in_eval {
			server.eval_runs(ctx);
		}

		if !in_eval && server.process() {
			unsafe {
				CURRENT_ACTION = DebuggerAction::Pause;
				PAUSE_ASKED = Some(std::time::Instant::now());
			}
		}

		let opcode_ptr = unsafe { (*(*ctx).bytecode()).add(*(*ctx).bytecode_offset() as usize) };
		let opcode = unsafe { *opcode_ptr };
		let is_dbgline = opcode == OPCODE_DBGLINE;

		let mut did_breakpoint = false;

		if !in_eval && server.has_watches() {
			if let Some(changes) = server.watched_changes(ctx) {
				unsafe {
					if let Some(action) = handle_breakpoint(ctx, BreakpointReason::Data { changes }) {
						CURRENT_ACTION = action;
						did_breakpoint = true;
					}
				}
			}
		}

		unsafe {
			let action = if in_eval { DebuggerAction::None } else { CURRENT_ACTION };

			match action {
				DebuggerAction::StepOver { target: frame, .. }
				| DebuggerAction::StepInto { parent: frame, .. }
				| DebuggerAction::StepOut { target: frame }
				| DebuggerAction::StepOverInstruction { target: frame, .. }
				| DebuggerAction::StepIntoInstruction { parent: frame, .. } => {
					if *(*ctx).bytecode_offset() == 0 && !(ctx == LAST_FRAME && crate::profiler::can_jump(LAST_OPCODE)) {
						(**(*ctx).proc_instance()).mega_hack = 0;
					}
					frame.forget_copy(ctx);
				}
				_ => {}
			}
			LAST_FRAME = ctx;
			LAST_OPCODE = opcode;

			match action {
				DebuggerAction::None => {}

				DebuggerAction::Pause if PAUSE_ASKED.is_some_and(|asked| asked.elapsed() < PAUSE_PATIENCE) && in_helper(ctx) => {}

				DebuggerAction::Pause => {
					CURRENT_ACTION = DebuggerAction::None;
					CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Pause).unwrap_or(DebuggerAction::None);
					did_breakpoint = true;
				}

				DebuggerAction::BreakOnNext if is_dbgline || opcode == OPCODE_DBGFILE => {}

				DebuggerAction::BreakOnNext => {
					CURRENT_ACTION = DebuggerAction::None;
					CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Step).unwrap_or(DebuggerAction::None);
					did_breakpoint = true;
				}

				DebuggerAction::StepOver { target, caller } => {
					if is_dbgline && target.is_frame(ctx) {
						CURRENT_ACTION = DebuggerAction::BreakOnNext;
					} else if !proc_instance_is_in_stack(ctx, target) && !proc_instance_is_suspended(target) {
						CURRENT_ACTION = DebuggerAction::None;
						if caller.is_some_and(|caller| caller.is_frame(ctx)) {
							CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Step).unwrap_or(DebuggerAction::None);
							did_breakpoint = true;
						} else {
							step_ended();
						}
					}
				}

				DebuggerAction::StepInto { parent, caller } => {
					if !is_generated_proc(ctx) {
						let is_in_parent = parent.is_frame(ctx);

						if is_dbgline && is_in_parent {
							CURRENT_ACTION = DebuggerAction::BreakOnNext;
						} else if !is_in_parent {
							let in_stack = proc_instance_is_in_stack(ctx, parent);
							let is_suspended = proc_instance_is_suspended(parent);

							if !in_stack && !is_suspended {
								CURRENT_ACTION = DebuggerAction::None;
								if caller.is_some_and(|caller| caller.is_frame(ctx)) {
									CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Step).unwrap_or(DebuggerAction::None);
									did_breakpoint = true;
								} else {
									step_ended();
								}
							} else if in_stack && is_dbgline {
								CURRENT_ACTION = DebuggerAction::BreakOnNext;
							}
						}
					}
				}

				DebuggerAction::StepOverInstruction { target: frame, caller } | DebuggerAction::StepIntoInstruction { parent: frame, caller } => {
					let into = matches!(action, DebuggerAction::StepIntoInstruction { .. });
					let here = frame.is_frame(ctx) || (into && proc_instance_is_in_stack(ctx, frame));
					let over = !here && !proc_instance_is_in_stack(ctx, frame) && !proc_instance_is_suspended(frame);
					if here || over {
						CURRENT_ACTION = DebuggerAction::None;
						if here || caller.is_some_and(|caller| caller.is_frame(ctx)) {
							CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Step).unwrap_or(DebuggerAction::None);
							did_breakpoint = true;
						} else {
							step_ended();
						}
					}
				}

				DebuggerAction::StepOut { target } => {
					if !is_generated_proc(ctx) {
						if target.is_frame(ctx) {
							CURRENT_ACTION = DebuggerAction::None;
							CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Step).unwrap_or(DebuggerAction::None);
							did_breakpoint = true;
						} else {
							let in_stack = proc_instance_is_in_stack(ctx, target);
							let is_suspended = proc_instance_is_suspended(target);

							if !in_stack && !is_suspended {
								CURRENT_ACTION = DebuggerAction::None;
								step_ended();
							}
						}
					}
				}
			}
		}

		if opcode == OPCODE_THROW && !did_breakpoint && !in_eval {
			unsafe {
				let reason = BreakpointReason::Runtime {
					message: thrown(ctx),
					caught: in_try(ctx)
				};
				if let Some(action) = handle_breakpoint(ctx, reason) {
					CURRENT_ACTION = action;
					did_breakpoint = true;
				}
			}
		}

		let opcode = unsafe { *opcode_ptr };
		if opcode == OPCODE_DEBUG_BREAK {
			if !did_breakpoint && !in_eval {
				unsafe {
					let stepping = CURRENT_ACTION;
					CURRENT_ACTION = DebuggerAction::None;
					CURRENT_ACTION = handle_breakpoint(ctx, BreakpointReason::Breakpoint).unwrap_or(stepping);
				}
			}

			let map = ORIGINAL_BYTECODE.lock().unwrap();
			if let Some(original) = map.get(&PtrKey::new(opcode_ptr)) {
				unsafe {
					if original.first() == Some(&OPCODE_DBGLINE) && !in_eval {
						match CURRENT_ACTION {
							DebuggerAction::StepOver { target: frame, .. } | DebuggerAction::StepInto { parent: frame, .. }
								if frame.is_frame(ctx) =>
							{
								CURRENT_ACTION = DebuggerAction::BreakOnNext;
							}
							_ => {}
						}
					}

					let deferred_replace = DEFERRED_INSTRUCTION_REPLACE.get();
					if let Some((src, dst)) = &*deferred_replace {
						std::ptr::copy_nonoverlapping(src.as_ptr(), *dst, src.len());
					}
					*deferred_replace = Some((std::slice::from_raw_parts(opcode_ptr, original.len()).to_vec(), opcode_ptr));
					std::ptr::copy_nonoverlapping(original.as_ptr(), opcode_ptr, original.len());
				}
			}
		}
	}
}

#[derive(Debug)]
pub enum InstructionHookError {
	InvalidOffset
}

fn instruction_length(proc: &Proc, offset: u32) -> Option<usize> {
	let bytecode = original_bytecode(proc);
	let mut env = disassemble_env::DisassembleEnv;
	let (nodes, _error) = dmasm::disassembler::disassemble(&bytecode, &mut env);

	nodes.into_iter().find_map(|node| match node {
		dmasm::Node::Instruction(_, debug) if debug.offset == offset => Some(debug.bytecode.len()),
		_ => None
	})
}

pub fn hook_instruction(proc: &Proc, offset: u32) -> Result<(), InstructionHookError> {
	let instruction_length = instruction_length(proc, offset).ok_or(InstructionHookError::InvalidOffset)?;

	let bytecode;
	let opcode;
	let opcode_ptr;

	unsafe {
		bytecode = {
			let (ptr, count) = proc.bytecode_mut_ptr();
			std::slice::from_raw_parts_mut(ptr, count as usize)
		};

		opcode_ptr = bytecode.as_mut_ptr().add(offset as usize);
		opcode = *opcode_ptr;
	}

	if opcode == OPCODE_DEBUG_BREAK {
		return Ok(());
	}

	unsafe {
		ORIGINAL_BYTECODE.lock().unwrap().insert(
			PtrKey::new(opcode_ptr),
			std::slice::from_raw_parts(opcode_ptr, instruction_length).to_vec()
		);
	}

	bytecode[offset as usize] = OPCODE_DEBUG_BREAK;
	for i in (offset + 1)..(offset + instruction_length as u32) {
		bytecode[i as usize] = OPCODE_DEBUG_OPERAND;
	}
	Ok(())
}

#[derive(Debug)]
pub enum InstructionUnhookError {
	InvalidOffset
}

pub fn unhook_instruction(proc: &Proc, offset: u32) -> Result<(), InstructionUnhookError> {
	instruction_length(proc, offset).ok_or(InstructionUnhookError::InvalidOffset)?;

	let opcode_ptr = unsafe {
		let bytecode = {
			let (ptr, count) = proc.bytecode_mut_ptr();
			std::slice::from_raw_parts_mut(ptr, count as usize)
		};

		bytecode.as_mut_ptr().add(offset as usize)
	};

	let mut map = ORIGINAL_BYTECODE.lock().unwrap();
	if let Some(original) = map.get(&PtrKey::new(opcode_ptr)) {
		unsafe {
			let deferred = DEFERRED_INSTRUCTION_REPLACE.get();
			if let Some((_, dst)) = *deferred {
				if dst == opcode_ptr {
					deferred.replace(None);
				}
			}
			std::ptr::copy_nonoverlapping(original.as_ptr(), opcode_ptr, original.len());
		}

		map.remove(&PtrKey::new(opcode_ptr));
	}

	Ok(())
}

pub fn unhook_all() {
	unsafe {
		CURRENT_ACTION = DebuggerAction::None;
		*DEFERRED_INSTRUCTION_REPLACE.get() = None;
	}

	for (opcode_ptr, original) in ORIGINAL_BYTECODE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).drain() {
		unsafe {
			std::ptr::copy_nonoverlapping(original.as_ptr(), opcode_ptr.0 as *mut u32, original.len());
		}
	}
}

pub fn hooked_offsets(proc: &Proc) -> Vec<u32> {
	let (start, count) = unsafe { proc.bytecode_mut_ptr() };
	let start = start as usize;
	let end = start + count as usize * std::mem::size_of::<u32>();

	let mut offsets: Vec<u32> = ORIGINAL_BYTECODE
		.lock()
		.unwrap()
		.keys()
		.filter(|ptr| (start..end).contains(&ptr.0))
		.map(|ptr| ((ptr.0 - start) / std::mem::size_of::<u32>()) as u32)
		.collect();
	offsets.sort_unstable();
	offsets
}

pub fn original_bytecode(proc: &Proc) -> Vec<u32> {
	let (start, count) = unsafe { proc.bytecode_mut_ptr() };
	let bytecode = unsafe { std::slice::from_raw_parts(start, count as usize) }.to_vec();
	unpatched(start, bytecode)
}

pub fn unpatched(start: *const u32, mut bytecode: Vec<u32>) -> Vec<u32> {
	for (opcode_ptr, original) in ORIGINAL_BYTECODE.lock().unwrap().iter() {
		let Some(offset) = opcode_ptr.0.checked_sub(start as usize).map(|bytes| bytes / std::mem::size_of::<u32>()) else {
			continue;
		};

		if let Some(hooked) = bytecode.get_mut(offset..offset + original.len()) {
			hooked.copy_from_slice(original);
		}
	}

	bytecode
}
