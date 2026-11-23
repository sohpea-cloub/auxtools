use std::{
	cell::UnsafeCell,
	collections::{HashMap, HashSet},
	convert::TryFrom,
	ffi::c_void,
	sync::atomic::{AtomicU32, AtomicU64, Ordering}
};

use crate::{
	debug,
	raw_types::{
		self,
		misc::{MiscId, Misc_V2},
		procs::ProcId
	},
	runtime_tables::{self, ProcEntryWords},
	*
};

static mut BYTECODE_ALLOCATIONS: UnsafeCell<Option<State>> = UnsafeCell::new(None);

static CHANGES: AtomicU64 = AtomicU64::new(0);

static SERIAL: AtomicU32 = AtomicU32::new(u32::MAX);

/// A counter that changes when a proc is repointed, and at world start and
/// shutdown.
pub fn bytecode_changes() -> u64 {
	CHANGES.load(Ordering::Relaxed)
}

fn changed() {
	CHANGES.fetch_add(1, Ordering::Relaxed);
}

struct State {
	allocations: HashMap<(ProcId, Vec<u32>), Box<[u32]>>,
	original: HashMap<u32, (*mut u32, u16)>,
	made: HashMap<u32, ProcId>,
	retired_by_address: HashMap<usize, Vec<u32>>
}

pub fn init() {
	changed();
	unsafe {
		let ptr = BYTECODE_ALLOCATIONS.get();
		*ptr = Some(State {
			allocations: HashMap::new(),
			original: HashMap::new(),
			made: HashMap::new(),
			retired_by_address: HashMap::new()
		});
	}
}

fn get_active_bytecode_ptrs() -> HashSet<*mut u32> {
	fn visit(dst: &mut HashSet<*mut u32>, frames: Vec<debug::StackFrame>) {
		for frame in frames {
			let ptr = unsafe { *(*frame.context).bytecode() };

			dst.insert(ptr);
		}
	}

	let stacks = debug::CallStacks::new();

	let mut ptrs = HashSet::new();
	visit(&mut ptrs, stacks.active);
	for stack in stacks.suspended {
		visit(&mut ptrs, stack);
	}

	ptrs
}

pub fn shutdown() {
	changed();
	let mut kept = get_active_bytecode_ptrs();

	let state = unsafe { (*BYTECODE_ALLOCATIONS.get()).take().unwrap() };

	for (entry, original) in state.original {
		unsafe {
			if references(entry) == Ok(1) {
				let _ = point_at(entry, original);
			} else if let Ok((buffer, _)) = code_at(entry) {
				kept.insert(buffer);
			}
		}
	}

	for mut buffer in state.allocations.into_values() {
		if kept.contains(&buffer.as_mut_ptr()) {
			std::mem::forget(buffer);
		}
	}
}

fn state() -> &'static mut State {
	unsafe { (*BYTECODE_ALLOCATIONS.get()).as_mut().unwrap() }
}

unsafe fn misc_entry(entry: u32) -> Result<*mut Misc_V2, String> {
	let mut misc: *mut c_void = std::ptr::null_mut();
	if raw_types::funcs::get_misc_by_id(&mut misc, MiscId::from_raw(entry)) != 1 || misc.is_null() {
		return Err(format!("there's no misc entry {}", entry));
	}
	Ok(misc as *mut Misc_V2)
}

unsafe fn code_at(entry: u32) -> Result<(*mut u32, u16), String> {
	let misc = misc_entry(entry)?;
	Ok(((*misc).bytecode.bytecode, (*misc).bytecode.count))
}

unsafe fn point_at(entry: u32, (code, len): (*mut u32, u16)) -> Result<(), String> {
	let misc = misc_entry(entry)?;
	(*misc).bytecode.bytecode = code;
	(*misc).bytecode.count = len;
	Ok(())
}

unsafe fn references(entry: u32) -> Result<u32, String> {
	Ok(*((misc_entry(entry)? as *const u8).add(4) as *const u32))
}

/// Keeps `bytecode` as the code at `ptr` for readers of old frames.
pub fn remember_bytecode(ptr: *const u32, bytecode: &[u32]) {
	changed();
	state().retired_by_address.entry(ptr as usize).or_insert_with(|| bytecode.to_vec());
}

/// The code at `ptr`, if it is code that this library replaced or kept.
pub fn bytecode_at(ptr: *const u32) -> Option<Vec<u32>> {
	let state = state();

	if let Some(bytecode) = state.retired_by_address.get(&(ptr as usize)) {
		return Some(bytecode.clone());
	}

	if let Some(&(original, len)) = state.original.values().find(|(original, _)| std::ptr::eq(*original, ptr)) {
		return Some(unsafe { std::slice::from_raw_parts(original, len as usize) }.to_vec());
	}

	state
		.allocations
		.values()
		.find(|buffer| buffer.as_ptr() == ptr)
		.map(|buffer| buffer.to_vec())
}

fn code_entry(proc: &Proc) -> Result<u32, String> {
	runtime_tables::proc_entry(proc.id)
		.map(|entry| entry.0[ProcEntryWords::BYTECODE])
		.ok_or_else(|| format!("{} has no proc entry", proc.path))
}

unsafe fn owned_entry(state: &State, proc: &Proc) -> Option<u32> {
	let entry = code_entry(proc).ok()?;
	(state.made.get(&entry) == Some(&proc.id) && references(entry) == Ok(1)).then_some(entry)
}

unsafe fn new_entry(code: &[u32]) -> Result<u32, String> {
	let mut words = code.to_vec();
	words.push(0);
	for _ in 0..8 {
		*words.last_mut().unwrap() = SERIAL.fetch_sub(1, Ordering::Relaxed);
		let entry = runtime_tables::intern_misc(&words)?;
		if references(entry)? == 1 && runtime_tables::misc_words(entry).as_ref() == Some(&words) {
			return Ok(entry);
		}
	}
	Err("BYOND gave out code entries that other code uses".to_owned())
}

fn keep_frame_names(code: *mut u32, proc: ProcId) {
	let stacks = debug::CallStacks::new();
	for frame in stacks.active.iter().chain(stacks.suspended.iter().flatten()) {
		let running = unsafe { *(*frame.context).bytecode() };
		if frame.proc.id != proc && running == code && !debug::has_local_names(&frame.proc, code) {
			let names = debug::local_names_for(&frame.proc, code);
			debug::set_local_names(&frame.proc, code, names);
		}
	}
}

unsafe fn give_own_entry(state: &mut State, proc: &Proc, keep_address: bool) -> Result<u32, String> {
	let old = code_entry(proc)?;
	let (code, len) = code_at(old)?;
	let words = std::slice::from_raw_parts(code, len as usize).to_vec();
	let new = new_entry(&words)?;
	let (copy, _) = code_at(new)?;

	if keep_address {
		keep_frame_names(code, proc.id);
		point_at(new, (code, len))?;
		point_at(old, (copy, len))?;
		if let Some(original) = state.original.remove(&old) {
			state.original.insert(new, original);
		}
		debug::copy_local_names(code, copy, |id| id != proc.id);
		state.retired_by_address.entry(code as usize).or_insert(words);
	} else {
		point_at(new, (copy, len))?;
		debug::copy_local_names(code, copy, |id| id == proc.id);
		state.retired_by_address.entry(code as usize).or_insert(words);
	}
	runtime_tables::set_proc_entry_word(proc.id, ProcEntryWords::BYTECODE, new);
	state.made.insert(new, proc.id);
	changed();
	Ok(new)
}

/// Gives `proc` a code entry that nothing else uses, unless it has one. Its
/// code keeps its address, so its running frames run any patch to it.
///
/// # Safety
/// No other library has replaced `proc`'s code. The new entry takes over the
/// code's buffer, and that library would free the buffer when it shuts down.
pub unsafe fn own_code(proc: &Proc) -> Result<(), String> {
	let state = state();
	if owned_entry(state, proc).is_none() {
		give_own_entry(state, proc, true).map_err(|e| format!("{} couldn't get code of its own: {}", proc.path, e))?;
	}
	Ok(())
}

/// Gives `proc` a new code entry of its own, with a copy of its code at a new
/// address.
pub fn copy_code(proc: &Proc) -> Result<(), String> {
	unsafe { give_own_entry(state(), proc, false) }
		.map(|_| ())
		.map_err(|e| format!("{} couldn't get a copy of its code: {}", proc.path, e))
}

pub fn set_bytecode(proc: &Proc, bytecode: Vec<u32>) -> Result<(), String> {
	let len = u16::try_from(bytecode.len()).map_err(|_| format!("{} words are too many for {}", bytecode.len(), proc.path))?;
	unsafe {
		let state = state();
		let entry = match owned_entry(state, proc) {
			Some(entry) => entry,
			None => give_own_entry(state, proc, false).map_err(|e| format!("{} couldn't get code of its own: {}", proc.path, e))?
		};
		let current = code_at(entry)?;
		state.original.entry(entry).or_insert(current);

		let ptr = state
			.allocations
			.entry((proc.id, bytecode))
			.or_insert_with_key(|(_, bytecode)| bytecode.clone().into_boxed_slice())
			.as_mut_ptr();
		point_at(entry, (ptr, len))?;
		changed();
		Ok(())
	}
}
