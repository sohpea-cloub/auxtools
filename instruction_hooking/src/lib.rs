pub mod disassemble_env;

use auxtools::*;
use std::cell::UnsafeCell;

pub trait InstructionHook {
	fn handle_instruction(&mut self, ctx: *mut raw_types::procs::ExecutionContext);
}

pub static mut INSTRUCTION_HOOKS: UnsafeCell<Vec<Box<dyn InstructionHook>>> = UnsafeCell::new(Vec::new());

#[init(full)]
fn instruction_hooking_init() -> Result<(), String> {
	auxtools::instruction_hook::set_handler(handle_instruction)
}

#[shutdown]
fn instruction_hooking_shutdown() {
	unsafe {
		INSTRUCTION_HOOKS.get_mut().clear();
	}
}

fn handle_instruction(ctx: *mut raw_types::procs::ExecutionContext) {
	unsafe {
		for vec_box in &mut *INSTRUCTION_HOOKS.get() {
			vec_box.handle_instruction(ctx);
		}
	}
}
