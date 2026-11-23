use std::{
	cell::UnsafeCell,
	collections::{HashMap, HashSet},
	convert::TryFrom
};

use crate::{
	debug, raw_types,
	runtime_tables::{self, ProcEntryWords},
	*
};

static mut BYTECODE_ALLOCATIONS: UnsafeCell<Option<State>> = UnsafeCell::new(None);

static CHANGES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A counter that changes when a proc is repointed, and at world start and
/// shutdown.
pub fn bytecode_changes() -> u64 {
	CHANGES.load(std::sync::atomic::Ordering::Relaxed)
}

fn changed() {
	CHANGES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

struct State {
	allocations: HashMap<(raw_types::procs::ProcId, Vec<u32>), Box<[u32]>>,
	original: HashMap<raw_types::procs::ProcId, (*mut u32, u16)>,
	retired_by_address: HashMap<usize, Vec<u32>>
}

pub fn init() {
	changed();
	unsafe {
		let ptr = BYTECODE_ALLOCATIONS.get();
		*ptr = Some(State {
			allocations: HashMap::new(),
			original: HashMap::new(),
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
	let active_ptrs = get_active_bytecode_ptrs();

	let state = unsafe { (*BYTECODE_ALLOCATIONS.get()).take().unwrap() };

	for (id, (ptr, len)) in state.original {
		let proc = Proc::from_id(id).unwrap();

		unsafe {
			raw_types::misc::set_bytecode((*proc.entry()).metadata.bytecode, ptr, len);
		}
	}

	for mut buffer in state.allocations.into_values() {
		if active_ptrs.contains(&buffer.as_mut_ptr()) {
			std::mem::forget(buffer);
		}
	}
}

fn state() -> &'static mut State {
	unsafe { (*BYTECODE_ALLOCATIONS.get()).as_mut().unwrap() }
}

fn remember_original(state: &mut State, proc: &Proc) {
	state.original.entry(proc.id).or_insert_with(|| unsafe { proc.bytecode_mut_ptr() });
}

pub fn remember_bytecode(ptr: *const u32, bytecode: &[u32]) {
	changed();
	state().retired_by_address.entry(ptr as usize).or_insert_with(|| bytecode.to_vec());
}

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

fn procs_sharing_code(proc: &Proc) -> Vec<Proc> {
	let Some(entry) = runtime_tables::proc_entry(proc.id) else {
		return vec![];
	};
	let code = entry.0[ProcEntryWords::BYTECODE];
	(0..)
		.map(raw_types::procs::ProcId)
		.map_while(|id| runtime_tables::proc_entry(id).map(|entry| (id, entry)))
		.filter(|(id, entry)| *id != proc.id && entry.0[ProcEntryWords::BYTECODE] == code)
		.filter_map(|(id, _)| Proc::from_id(id))
		.collect()
}

const OWN_ENTRY_MARK: u32 = 0xFFFF_FFFF;

fn give_new_code_entry(proc: &Proc, code: &[u32]) -> Result<(), String> {
	let mut words = code.to_vec();
	words.push(OWN_ENTRY_MARK);
	let entry = runtime_tables::intern_misc(&words)?;
	if runtime_tables::misc_words(entry).as_deref() != Some(&words[..]) {
		return Err(format!("BYOND gave {} a code entry that other code already uses", proc.path));
	}

	let (old, _) = unsafe { proc.bytecode_mut_ptr() };
	runtime_tables::set_proc_entry_word(proc.id, ProcEntryWords::BYTECODE, entry);
	unsafe {
		let id = (*proc.entry()).metadata.bytecode;
		let (new, _) = raw_types::misc::get_bytecode(id);
		raw_types::misc::set_bytecode(id, new, u16::try_from(code.len()).unwrap());
		if debug::has_local_names(old) {
			debug::set_local_names(new, debug::local_names_for(proc, old));
		}
	}
	changed();
	Ok(())
}

pub fn unshare_code(proc: &Proc, code: &[u32]) -> Result<(), String> {
	for other in procs_sharing_code(proc) {
		give_new_code_entry(&other, code)?;
	}
	Ok(())
}

pub fn set_bytecode(proc: &Proc, bytecode: Vec<u32>) {
	let state = state();
	if !state.original.contains_key(&proc.id) && !procs_sharing_code(proc).is_empty() {
		let code = unsafe { proc.bytecode() }.to_vec();
		let _ = give_new_code_entry(proc, &code);
	}
	remember_original(state, proc);

	let len = u16::try_from(bytecode.len()).unwrap();
	let ptr = state
		.allocations
		.entry((proc.id, bytecode))
		.or_insert_with_key(|(_, bytecode)| bytecode.clone().into_boxed_slice())
		.as_mut_ptr();

	unsafe {
		raw_types::misc::set_bytecode((*proc.entry()).metadata.bytecode, ptr, len);
	}
}
