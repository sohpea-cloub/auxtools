//! For when BYOND is not enough. Probably often.

//#[cfg(not(target_pointer_width = "32"))]
// compile_error!("Auxtools must be compiled for a 32-bit target");

// public so `byond_ffi_fn!` can reach it from another crate
#[doc(hidden)]
pub mod byond_ffi;
mod bytecode_manager;
pub mod debug;
pub mod hooks;
mod init;
mod list;
mod proc;
pub mod raw_types;
pub mod runtime_tables;
mod runtime;
mod string;
mod string_intern;
mod symbols;
mod value;
mod value_from;
pub mod version;
mod weak_value;

use std::sync::{
	atomic::{AtomicBool, Ordering},
	Mutex, OnceLock
};

pub use auxtools_impl::{full_shutdown, hook, init, pin_dll, runtime_handler, shutdown};
/// For crates that need to find something else in BYOND's binary.
pub use byond_scan;
pub use bytecode_manager::{bytecode_at, remember_bytecode};
/// Used by the [pin_dll] macro to set dll pinning
pub use ctor;
pub use hooks::{CompileTimeHook, HookFailure, ProcHook, RuntimeErrorHook};
use init::{get_init_level, set_init_level, InitLevel};
pub use init::{FullInitFunc, FullShutdownFunc, PartialInitFunc, PartialShutdownFunc};
/// Used by the [hook](attr.hook.html) macro to aggregate all compile-time hooks
pub use inventory;
pub use list::List;
pub use proc::Proc;
pub use raw_types::variables::VariableNameIdTable;
pub use runtime::{DMResult, Runtime};
pub use string::StringRef;
pub use string_intern::InternedString;
pub use symbols::{byond_build, find_recipe, find_recipe_of};
pub use value::Value;
pub use weak_value::WeakValue;

// We need winapi to call GetModuleHandleExW which lets us prevent our DLL from
// unloading.
#[cfg(windows)]
extern crate winapi;

pub static PIN_DLL: AtomicBool = AtomicBool::new(true);

/// wrapper for [`byond_scan::MODULE_NAME`](byond_scan::MODULE_NAME)
pub const BYONDCORE: &str = byond_scan::MODULE_NAME;

// This strange section of code retrieves our DLL using the init function's
// address. This increments the DLL reference count, which prevents unloading.
#[cfg(windows)]
fn pin_dll() -> Result<(), ()> {
	unsafe {
		use winapi::um::libloaderapi::{GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN};
		let mut module = std::ptr::null_mut();

		let flags = match PIN_DLL.load(Ordering::Relaxed) {
			true => GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
			false => GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
		};

		let res = GetModuleHandleExW(flags, pin_dll as *const _, &mut module);

		if res == 0 {
			return Err(());
		}
	}
	Ok(())
}
#[cfg(unix)]
fn pin_dll() -> Result<(), ()> {
	Ok(())
}

// The first failed init. Trying again would run setup on top of what the
// failed attempt left behind, so every later call hands this back instead.
static INIT_FAILURE: OnceLock<String> = OnceLock::new();

byond_ffi_fn! { auxtools_init(_input) {
	if let Some(failure) = INIT_FAILURE.get() {
		return Some(failure.clone());
	}

	match try_init() {
		Ok(()) => Some("SUCCESS".to_owned()),
		Err(e) => {
			// a hook must not fire in a library that only got half set up
			hooks::clear_hooks();
			proc::clear_procs();
			Some(INIT_FAILURE.get_or_init(|| format!("FAILED ({})", e)).clone())
		}
	}
} }

fn try_init() -> Result<(), String> {
	try_init_with(None)
}

/// `prepare` runs once the world's procs are known and before the
/// compile-time hooks go on, so it can add procs they need.
fn try_init_with(prepare: Option<fn() -> Result<(), String>>) -> Result<(), String> {
	if get_init_level() == InitLevel::None {
		return Ok(());
	}

	let mut did_full = false;
	let mut did_partial = false;

	if get_init_level() == InitLevel::Full {
		did_full = true;
		init_full()?;
	}

	if get_init_level() == InitLevel::Partial {
		did_partial = true;

		// This is a heap ptr so fetch it on partial loads
		symbols::resolve_partial()?;

		proc::populate_procs();
		bytecode_manager::init();
		string_intern::setup_interned_strings();

		if let Some(prepare) = prepare {
			prepare()?;
		}

		for cthook in inventory::iter::<hooks::CompileTimeHook> {
			hooks::hook(cthook.proc_path, cthook.hook).map_err(|e| format!("Could not hook proc {}: {:?}", cthook.proc_path, e))?;
		}
		set_init_level(InitLevel::None);
	}

	// Run user-defined initializers
	if did_full || FULL_INIT_FUNCS_PENDING.swap(false, Ordering::Relaxed) {
		init::run_full_init()?;
	}

	if did_partial {
		init::run_partial_init()?;
	}

	Ok(())
}

/// What needs no world: the BYOND version, the symbols and the detours
fn init_full() -> Result<(), String> {
	version::init()?;
	symbols::resolve_full()?;
	pin_dll().map_err(|()| "Could not pin the library in memory.".to_owned())?;
	hooks::init().map_err(|_| "Couldn't initialize proc hooking".to_owned())?;

	set_init_level(InitLevel::Partial);
	Ok(())
}

/// Called on BYOND's thread by [`init_on_world_load`] each time a world loads
pub struct WorldLoad {
	/// Runs once the world's procs are known, before the compile-time hooks
	/// go on. It can add procs they need.
	pub prepare: fn() -> Result<(), String>,
	/// Runs once auxtools is initialised
	pub ready: fn(),
	/// Runs instead of `ready` when initialising failed
	pub failed: fn(&str)
}

static WORLD_LOAD: OnceLock<&'static WorldLoad> = OnceLock::new();
// Set while a world is yet to load. Read on every DM proc call.
static WORLD_LOAD_PENDING: AtomicBool = AtomicBool::new(false);
// The init funcs run after the partial init, as they would with auxtools_init
static FULL_INIT_FUNCS_PENDING: AtomicBool = AtomicBool::new(false);

/// For a library that is loaded into BYOND before the world is, rather than by
/// a world calling `auxtools_init`. Does the part of init that needs no world
/// now, and the rest on BYOND's thread when the world makes its first proc
/// call, around `on_load`. That's before any of the world's own code runs:
/// global var initialisers come first.
///
/// [`init_on_next_world_load`] does it again after a reboot, which reloads the
/// world.
pub fn init_on_world_load(on_load: &'static WorldLoad) -> Result<(), String> {
	if WORLD_LOAD.set(on_load).is_err() {
		return Err("already waiting for the world to load".to_owned());
	}

	if let Err(e) = init_full() {
		hooks::clear_hooks();
		return Err(e);
	}

	FULL_INIT_FUNCS_PENDING.store(true, Ordering::Relaxed);
	WORLD_LOAD_PENDING.store(true, Ordering::Relaxed);
	Ok(())
}

/// Shuts down what `auxtools_shutdown` does, and initialises again on the next
/// proc call. For a world that's about to reboot, after
/// [`init_on_world_load`].
pub fn init_on_next_world_load() {
	if WORLD_LOAD.get().is_none() || get_init_level() != InitLevel::None {
		return;
	}
	shutdown_partial();
	WORLD_LOAD_PENDING.store(true, Ordering::Relaxed);
}

/// Shuts down what `auxtools_shutdown` does, for good: for a world that's
/// ending, which may still run code on its way out, after
/// [`init_on_world_load`].
pub fn shut_down_with_world() {
	if WORLD_LOAD.get().is_none() || get_init_level() != InitLevel::None {
		return;
	}
	shutdown_partial();
}

// Work waiting for a later proc call (see `at_proc_calls`), and whether there
// is any. Read on every DM proc call.
static PROC_CALL_WORK: Mutex<Vec<fn() -> bool>> = Mutex::new(Vec::new());
static PROC_CALL_WORK_PENDING: AtomicBool = AtomicBool::new(false);

/// Runs `work` on BYOND's thread at the start of the next DM proc call, and
/// each one after until it returns true. For what has to wait until the world
/// has got further with loading. Forgotten when auxtools shuts down.
pub fn at_proc_calls(work: fn() -> bool) {
	PROC_CALL_WORK.lock().unwrap().push(work);
	PROC_CALL_WORK_PENDING.store(true, Ordering::Relaxed);
}

/// Every DM proc call comes here first. The first of a world finishes the init
/// `init_on_world_load` started.
pub(crate) fn on_proc_call() {
	if PROC_CALL_WORK_PENDING.load(Ordering::Relaxed) {
		// Taken out first: the work can make proc calls of its own
		let work = std::mem::take(&mut *PROC_CALL_WORK.lock().unwrap());
		let waiting: Vec<_> = work.into_iter().filter(|work| !work()).collect();
		let mut queue = PROC_CALL_WORK.lock().unwrap();
		queue.extend(waiting);
		PROC_CALL_WORK_PENDING.store(!queue.is_empty(), Ordering::Relaxed);
	}

	if !WORLD_LOAD_PENDING.swap(false, Ordering::Relaxed) {
		return;
	}
	let Some(on_load) = WORLD_LOAD.get() else {
		return;
	};

	match try_init_with(Some(on_load.prepare)) {
		Ok(()) => (on_load.ready)(),
		Err(e) => {
			hooks::clear_hooks();
			proc::clear_procs();
			(on_load.failed)(&e);
		}
	}
}

byond_ffi_fn! { auxtools_shutdown(_input) {
	if get_init_level() != InitLevel::None {
		return Some("FAILED (already shut down)".to_owned())
	};
	shutdown_partial();
	Some("SUCCESS".to_owned())
} }

fn shutdown_partial() {
	PROC_CALL_WORK.lock().unwrap().clear();
	PROC_CALL_WORK_PENDING.store(false, Ordering::Relaxed);
	init::run_partial_shutdown();
	string_intern::destroy_interned_strings();
	bytecode_manager::shutdown();
	debug::clear_local_names();
	runtime_tables::reset();

	hooks::clear_hooks();
	proc::clear_procs();

	unsafe {
		raw_types::funcs::VARIABLE_NAMES = std::ptr::null();
	}

	set_init_level(InitLevel::Partial);
}

byond_ffi_fn! { auxtools_full_shutdown(_input) {
	if get_init_level() == InitLevel::Full {
		return Some("FAILED (already shut down)".to_owned())
	};
	if get_init_level() == InitLevel::None {
		shutdown_partial();
	}
	hooks::shutdown();
	set_init_level(InitLevel::Full);
	init::run_full_shutdown();

	if !PIN_DLL.load(Ordering::Relaxed) {
		#[cfg(windows)]
		unsafe {
			use winapi::um::libloaderapi::{
				FreeLibrary, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
				GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
			};
			let mut module = std::ptr::null_mut();

			let get_handle_res = GetModuleHandleExW(
				GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
				auxtools_full_shutdown as *const _,
				&mut module,
			);

			if get_handle_res == 0 {
				return Some("FAILED (Could not unpin the library from memory.)".to_owned())
			}

			FreeLibrary(module);
		}
	};
	Some("SUCCESS".to_owned())
} }

byond_ffi_fn! { auxtools_check_signatures(_input) {
	if let Err(e) = version::init() {
		return Some(format!("FAILED ({})", e));
	}
	match symbols::unresolved() {
		Err(e) => Some(format!("FAILED ({})", e)),
		Ok(missing) if missing.is_empty() => Some("SUCCESS".to_owned()),
		Ok(missing) => Some(format!("MISSING: {}", missing.join(", ")))
	}
} }
