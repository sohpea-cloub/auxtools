mod assemble_env;
mod ckey_override;
mod client_script;
mod connection;
mod dmb_tables;
mod hot_reload;
mod instruction_hooking;
mod journal;
mod launched;
mod macros;
mod profiler;
mod resource_reload;
mod server;
mod server_types;
mod skin;
mod stddef;
mod unnamed;
mod waker;
mod world_resources;

#[cfg(windows)]
mod crash_handler_windows;

#[cfg(windows)]
mod socket_hold_windows;

#[cfg(windows)]
mod mem_profiler;

#[cfg(not(windows))]
mod mem_profiler_stub;

use std::{
	any::Any,
	cell::UnsafeCell,
	sync::atomic::{AtomicU32, Ordering}
};

pub(crate) use ::instruction_hooking::disassemble_env::DisassembleEnv;
use ::instruction_hooking::{InstructionHook, INSTRUCTION_HOOKS};
use auxtools::*;
#[cfg(not(windows))]
use mem_profiler_stub as mem_profiler;

pub static mut DEBUG_SERVER: UnsafeCell<Option<server::Server>> = UnsafeCell::new(None);

/// World generation, incremented on reboot or shutdown.
pub static WORLD_ENDS: AtomicU32 = AtomicU32::new(0);

/// Abort debug server work when evaluated DM code ends the world.
/// Unwinding code must not access the destroyed server. Whatever can run an
/// evaluation runs under [`guard`], where the unwinding ends.
pub struct WorldEnded;

/// Running BYOND build used for compilation.
pub(crate) fn world_byond() -> dmasm::ByondVersion {
	let (major, build) = auxtools::version::get();
	dmasm::ByondVersion::new(major, build)
}

/// Panics [`guard`] caught in a row. From [`LOST`] on, the debugger has
/// stopped for good.
static PANICS: AtomicU32 = AtomicU32::new(0);

/// A debugger that panics this many times running isn't going to recover: a
/// lock a panic poisoned, or state it left half changed, fails every call
/// that follows.
const LOST: u32 = 3;

const LOST_REASON: &str = "the debugger stopped after panicking repeatedly";

/// Whether the debugger stopped for good after panicking repeatedly.
fn lost() -> bool {
	PANICS.load(Ordering::Relaxed) >= LOST
}

/// Run debugger work that BYOND called for. BYOND calls through `extern "C"`
/// code, and a panic that unwinds into it aborts the whole game. Losing the
/// debugger is the better deal: the panic ends here, the server drops its
/// clients, which can connect again, and the error says what panicked. A
/// debugger that keeps panicking stops, and its work is no longer run.
///
/// Work that ended the world (see [`WorldEnded`]) is an error too, and not
/// a panic.
pub(crate) fn guard<R>(work: impl FnOnce() -> R) -> Result<R, String> {
	if lost() {
		return Err(LOST_REASON.to_owned());
	}
	let done = auxtools::guard(work);
	settle(done, || unsafe { (*DEBUG_SERVER.get()).as_mut() })
}

/// [`guard`] for work on a server that isn't the world's yet.
pub(crate) fn guard_server<R>(server: &mut server::Server, work: impl FnOnce(&mut server::Server) -> R) -> Result<R, String> {
	if lost() {
		return Err(LOST_REASON.to_owned());
	}
	let done = auxtools::guard(|| work(&mut *server));
	settle(done, || Some(server))
}

/// [`guard`] for work on the world's server, if it has one.
pub(crate) fn on_server<R>(work: impl FnOnce(&mut server::Server) -> R) -> Option<R> {
	let server = unsafe { (*DEBUG_SERVER.get()).as_mut() }?;
	guard_server(server, work).ok()
}

/// Recover from the panic that ended guarded work, with the server it
/// was for.
fn settle<'a, R>(done: Result<R, Box<dyn Any + Send>>, server: impl FnOnce() -> Option<&'a mut server::Server>) -> Result<R, String> {
	let payload = match done {
		Ok(done) => {
			if (1..LOST).contains(&PANICS.load(Ordering::Relaxed)) {
				PANICS.store(0, Ordering::Relaxed);
			}
			return Ok(done);
		}
		Err(payload) => payload
	};
	// Its server went with the world
	if payload.is::<WorldEnded>() {
		return Err("the world ended".to_owned());
	}
	let panic = auxtools::panic_message(&*payload);
	let message = format!("the debugger panicked: {}", panic);
	if lost() {
		return Err(message);
	}
	let panics = PANICS.fetch_add(1, Ordering::Relaxed) + 1;
	let mut server = server();
	// Recovering works on what the panic left, and can panic too
	let recovered = auxtools::guard(|| {
		if let Some(server) = server.as_deref_mut() {
			server.recover_from_panic();
		}
	});
	if recovered.is_err() || panics >= LOST {
		PANICS.store(LOST, Ordering::Relaxed);
		eprintln!("auxtools: {}. The world runs on without it.", LOST_REASON);
		// A breakpoint nothing answers for ends the proc that reaches it
		let _ = auxtools::guard(instruction_hooking::unhook_all);
		if let Some(server) = server {
			let _ = auxtools::guard(|| server.give_up(format!("{} (the last panic: {})", LOST_REASON, panic)));
		}
	}
	Err(message)
}

#[shutdown]
fn debugger_shutdown() {
	WORLD_ENDS.fetch_add(1, Ordering::Relaxed);
	// INSTRUCTION_HOOKS are cleared on shutdown so we don't need to worry about
	// that.
	unsafe {
		DEBUG_SERVER.get_mut().take();
	}
}

struct DebugServerInstructionHook<'a> {
	debug_server: &'a mut UnsafeCell<Option<server::Server>>
}

impl InstructionHook for DebugServerInstructionHook<'static> {
	#[inline]
	fn handle_instruction(&mut self, ctx: *mut raw_types::procs::ExecutionContext) {
		let Some(debug_server) = self.debug_server.get_mut() else {
			return;
		};
		if instruction_hooking::passes(debug_server, ctx) {
			// Other registered hooks may still need every instruction.
			if unsafe { (*INSTRUCTION_HOOKS.get()).len() } == 1 {
				instruction_hooking::rest();
			}
			return;
		}
		attend(debug_server, ctx);
	}
}

/// Keep debugger work out of line to minimize idle-handler overhead.
#[inline(never)]
fn attend(debug_server: &mut server::Server, ctx: *mut raw_types::procs::ExecutionContext) {
	// Breakpoints and throws can start work requiring later callbacks.
	auxtools::instruction_hook::hear_all();
	let attended = guard_server(debug_server, |debug_server| {
		profiler::enter(ctx, debug_server.is_in_eval());
		debug_server.handle_instruction(ctx);
	});
	// A debugger that stopped has no use for callbacks. Other registered
	// hooks may still need every instruction.
	if attended.is_err() && lost() && unsafe { (*INSTRUCTION_HOOKS.get()).len() } == 1 {
		auxtools::instruction_hook::hear_only(&[]);
	}
}

/// Makes `server` the debug server, run on every instruction
fn install_server(server: server::Server) {
	let debug_server_instruction_hook;
	unsafe {
		*DEBUG_SERVER.get() = Some(server);
		auxtools::instruction_hook::hear_all();
		debug_server_instruction_hook = DebugServerInstructionHook {
			debug_server: &mut *std::ptr::addr_of_mut!(DEBUG_SERVER)
		};

		INSTRUCTION_HOOKS.get_mut().push(Box::new(debug_server_instruction_hook));
	}
}

#[cfg(test)]
mod tests {
	use super::{guard, lost, WorldEnded, LOST, LOST_REASON};

	#[test]
	fn guard_ends_panics_and_stops_a_debugger_that_keeps_panicking() {
		assert_eq!(guard(|| 1), Ok(1));
		let panicked = Err("the debugger panicked: no good".to_owned());
		assert_eq!(guard(|| panic!("no {}", "good")), panicked);

		// Work that ended the world isn't a panic
		for _ in 0..LOST {
			let ended: Result<(), _> = guard(|| std::panic::resume_unwind(Box::new(WorldEnded)));
			assert_eq!(ended, Err("the world ended".to_owned()));
		}
		assert!(!lost());

		// Nor do panics count once work has run to its end again
		for _ in 0..LOST {
			assert_eq!(guard(|| panic!("no good")), panicked);
			assert_eq!(guard(|| 2), Ok(2));
		}
		assert!(!lost());

		for _ in 0..LOST {
			assert_eq!(guard(|| panic!("no good")), panicked);
		}
		assert!(lost());
		let mut ran = false;
		assert_eq!(guard(|| ran = true), Err(LOST_REASON.to_owned()));
		assert!(!ran);
	}
}
