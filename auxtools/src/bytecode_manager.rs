// This module is in control of allocating buffers for use when replacing a
// proc's bytecode. To keep things sane, we also have to return the pointers to
// their original value before BYOND shuts down. We _also_ have to check if any
// existing procs are still using our bytecode when we shut down and leak the
// memory if so. It may be possible to avoid the leaks but it really doesn't
// matter.

use std::{
	cell::UnsafeCell,
	collections::{HashMap, HashSet},
	convert::TryFrom
};

use crate::{debug, raw_types, *};

static mut BYTECODE_ALLOCATIONS: UnsafeCell<Option<State>> = UnsafeCell::new(None);

struct State {
	allocations: HashSet<Vec<u32>>,
	// Buffers that belong to one proc each (hot reloads), which breakpoints can
	// be patched into without reaching other procs
	unshared: Vec<Vec<u32>>,
	original: HashMap<raw_types::procs::ProcId, (*mut u32, u16)>,
	original_locals: HashMap<raw_types::procs::ProcId, raw_types::misc::LocalsId>
}

pub fn init() {
	unsafe {
		let ptr = BYTECODE_ALLOCATIONS.get();
		*ptr = Some(State {
			allocations: HashSet::new(),
			unshared: Vec::new(),
			original: HashMap::new(),
			original_locals: HashMap::new()
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
	let active_ptrs = get_active_bytecode_ptrs();

	let state = unsafe { (*BYTECODE_ALLOCATIONS.get()).take().unwrap() };

	for (id, (ptr, len)) in state.original {
		let proc = Proc::from_id(id).unwrap();

		unsafe {
			raw_types::misc::set_bytecode(*(*proc.entry()).metadata.bytecode(), ptr, len);
		}
	}

	for (id, locals) in state.original_locals {
		let proc = Proc::from_id(id).unwrap();

		unsafe {
			*(*proc.entry()).metadata.locals_mut() = locals;
		}
	}

	for mut vec in state.allocations.into_iter().chain(state.unshared) {
		// If a proc with this bytecode is still running, just leak the mrmoy
		if active_ptrs.contains(&vec.as_mut_ptr()) {
			std::mem::forget(vec);
		}
	}
}

fn state() -> &'static mut State {
	unsafe { (*BYTECODE_ALLOCATIONS.get()).as_mut().unwrap() }
}

fn remember_original(state: &mut State, proc: &Proc) {
	state.original.entry(proc.id).or_insert_with(|| unsafe { proc.bytecode_mut_ptr() });
}

/// Gives `proc` new bytecode in a buffer of its own, and returns where that is.
/// Unlike [`set_bytecode`], two procs never end up sharing a buffer, so a
/// breakpoint patched into one stays out of the other.
pub fn set_bytecode_unshared(proc: &Proc, mut bytecode: Vec<u32>) -> *const u32 {
	let state = state();
	remember_original(state, proc);

	let len = u16::try_from(bytecode.len()).unwrap();
	let ptr = bytecode.as_mut_ptr();
	state.unshared.push(bytecode);

	unsafe {
		raw_types::misc::set_bytecode(*(*proc.entry()).metadata.bytecode(), ptr, len);
	}

	ptr
}

/// Points `proc` at another locals table. BYOND gives each new frame as many
/// locals as the table has names; frames already running keep what they got.
pub fn set_locals(proc: &Proc, locals: raw_types::misc::LocalsId) {
	let state = state();

	unsafe {
		let current = (*proc.entry()).metadata.locals_mut();
		state.original_locals.entry(proc.id).or_insert(*current);
		// BYOND frees a table nobody holds a reference to
		raw_types::misc::add_ref(&locals);
		*current = locals;
	}
}

/// A copy of the bytecode at `ptr`, if it's a proc's original bytecode or one
/// we gave a proc. Frames that started before a proc's bytecode was replaced
/// still run the old buffer.
pub fn bytecode_at(ptr: *const u32) -> Option<Vec<u32>> {
	let state = state();

	if let Some(&(original, len)) = state.original.values().find(|(original, _)| std::ptr::eq(*original, ptr)) {
		return Some(unsafe { std::slice::from_raw_parts(original, len as usize) }.to_vec());
	}

	state
		.unshared
		.iter()
		.chain(state.allocations.iter())
		.find(|bytecode| bytecode.as_ptr() == ptr)
		.cloned()
}

pub fn set_bytecode(proc: &Proc, mut bytecode: Vec<u32>) {
	let state = state();
	remember_original(state, proc);

	let (ptr, len) = {
		let len = bytecode.len();

		let ptr = match state.allocations.get(&bytecode) {
			Some(bytecode) => {
				bytecode.as_ptr() as *mut u32 // don't @ me
			}

			None => {
				let ptr = bytecode.as_mut_ptr();
				state.allocations.insert(bytecode);
				ptr
			}
		};

		(ptr, len)
	};

	let len = u16::try_from(len).unwrap();

	unsafe {
		raw_types::misc::set_bytecode(*(*proc.entry()).metadata.bytecode(), ptr, len);
	}
}
