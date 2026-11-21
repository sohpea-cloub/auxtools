use std::{
	cell::RefCell,
	collections::HashMap,
	ffi::{c_void, CStr},
	hash::BuildHasherDefault,
	os::raw::c_char
};

use fxhash::FxHashMap;
use retour::RawDetour;

use super::{proc::Proc, raw_types, value::Value};
use crate::runtime::DMResult;

#[doc(hidden)]
pub struct CompileTimeHook {
	pub proc_path: &'static str,
	pub hook: ProcHook
}

inventory::collect!(CompileTimeHook);

#[doc(hidden)]
pub struct RuntimeErrorHook(pub fn(&str));
inventory::collect!(RuntimeErrorHook);

extern "C" {
	static mut call_proc_by_id_original: *const c_void;

	static mut runtime_original: *const c_void;
	fn runtime_hook(error: *const c_char);

	// Set while a shim replaces BYOND's runtime errors with its own (hooks.h)
	static runtime_intercepted: bool;

	fn call_proc_by_id_hook_trampoline(
		usr: raw_types::values::Value,
		proc_type: u32,
		proc_id: raw_types::procs::ProcId,
		override_depth: u32,
		src: raw_types::values::Value,
		args: *mut raw_types::values::Value,
		args_count_l: usize,
		callback: u32,
		callback_value: u32
	) -> raw_types::values::Value;
}

struct Detours {
	pub runtime_detour: Option<RawDetour>,
	pub call_proc_detour: Option<RawDetour>
}

impl Detours {
	pub const fn new() -> Self {
		Self {
			runtime_detour: None,
			call_proc_detour: None
		}
	}
}

thread_local!(static DETOURS: RefCell<Detours> = const { RefCell::new(Detours::new()) });

pub enum HookFailure {
	NotInitialized,
	ProcNotFound,
	AlreadyHooked,
	UnknownFailure
}

impl std::fmt::Debug for HookFailure {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::NotInitialized => write!(f, "Library not initialized"),
			Self::ProcNotFound => write!(f, "Proc not found"),
			Self::AlreadyHooked => write!(f, "Proc is already hooked"),
			Self::UnknownFailure => write!(f, "Unknown failure")
		}
	}
}

pub fn init() -> Result<(), String> {
	unsafe {
		let runtime_hook = RawDetour::new(raw_types::funcs::runtime_byond as *const (), runtime_hook as *const ()).unwrap();

		runtime_hook.enable().unwrap();
		runtime_original = runtime_hook.trampoline() as *const () as *const c_void;

		let call_hook = RawDetour::new(raw_types::funcs::call_proc_by_id_byond as *const (), call_proc_by_id_entry as *const ()).unwrap();

		call_hook.enable().unwrap();
		call_proc_by_id_original = call_hook.trampoline() as *const () as *const c_void;

		DETOURS.with(|detours_cell| {
			let mut detours = detours_cell.borrow_mut();
			detours.runtime_detour = Some(runtime_hook);
			detours.call_proc_detour = Some(call_hook);
		});
	}
	Ok(())
}

pub fn shutdown() {
	unsafe {
		DETOURS.with(|detours_cell| {
			let detours = detours_cell.borrow();
			let runtime_hook = detours.runtime_detour.as_ref().unwrap();
			let call_proc_hook = detours.call_proc_detour.as_ref().unwrap();
			runtime_hook.disable().unwrap();
			call_proc_hook.disable().unwrap();
		});
	}
}

pub type ProcHook = fn(&Value, &Value, Vec<Value>) -> DMResult;

/// Lets a `RefCell` live in a plain `static`.
///
/// Everything here is only touched from BYOND's game thread: its proc calls and
/// init/shutdown, which it also drives. The table used to be a `thread_local!`,
/// but in a dlopen'd .so every access goes through `__tls_get_addr`, and it is
/// read on every single DM proc call.
struct GameThreadOnly<T>(T);

// SAFETY: only the game thread ever reaches these statics, see above
unsafe impl<T> Sync for GameThreadOnly<T> {}

static PROC_HOOKS: GameThreadOnly<RefCell<FxHashMap<raw_types::procs::ProcId, (ProcHook, String)>>> =
	GameThreadOnly(RefCell::new(HashMap::with_hasher(BuildHasherDefault::new())));

/// How many hooked procs have each low word of a proc ID. The entry to the
/// call detour looks a called proc up here, so a call to a proc that shares no
/// low word with a hooked one never reaches `PROC_HOOKS`.
static mut HOOKED_LOW_WORDS: [u8; 0x10000] = [0; 0x10000];

/// Where `call_proc_by_id`'s proc ID argument is on the stack at entry: after
/// the return address, `usr` and the proc type, and on Linux the pointer the
/// result is written through, which Windows returns in registers instead.
#[cfg(unix)]
const PROC_ID_ARGUMENT: usize = 0x14;
#[cfg(windows)]
const PROC_ID_ARGUMENT: usize = 0x10;

/// Where the detour on `call_proc_by_id` lands, on every DM proc call. Nearly
/// all of them have nothing for us: no work or world load is waiting for a
/// call, no shim is taking runtime errors and the proc isn't hooked. Those go
/// on to BYOND's function from here; the rest take
/// `call_proc_by_id_hook_trampoline`.
///
/// On Linux BYOND's function can return with EBX, ESI, EDI and EBP overwritten
/// (see `call_keeping_registers` in hooks.cpp), and BYOND's own caller crashes
/// on some of those. So the call is made on a copy of the twelve argument
/// dwords with the four registers saved around it, as the trampoline does.
#[cfg(unix)]
#[unsafe(naked)]
unsafe extern "C" fn call_proc_by_id_entry() {
	core::arch::naked_asm!(
		"cmp byte ptr [{work}], 0",
		"jne 2f",
		"cmp byte ptr [{load}], 0",
		"jne 2f",
		"cmp byte ptr [{intercepted}], 0",
		"jne 2f",
		// EAX is free at a function's entry
		"movzx eax, word ptr [esp + {proc_id}]",
		"cmp byte ptr [{hooked} + eax], 0",
		"jne 2f",
		"push ebp",
		"push ebx",
		"push esi",
		"push edi",
		// twelve bytes more keep the stack aligned to sixteen at the call
		"sub esp, 12",
		// The last argument dword, then each one before it: every push moves
		// ESP down to where the same offset finds the next
		".rept 12",
		"push dword ptr [esp + 76]",
		".endr",
		"call dword ptr [{original}]",
		// The function popped the pointer its result is written through, the
		// first of the twelve; EAX holds it
		"add esp, 56",
		"pop edi",
		"pop esi",
		"pop ebx",
		"pop ebp",
		"ret 4",
		"2:",
		"jmp {trampoline}",
		work = sym crate::PROC_CALL_WORK_PENDING,
		load = sym crate::WORLD_LOAD_PENDING,
		intercepted = sym runtime_intercepted,
		proc_id = const PROC_ID_ARGUMENT,
		hooked = sym HOOKED_LOW_WORDS,
		original = sym call_proc_by_id_original,
		trampoline = sym call_proc_by_id_hook_trampoline
	)
}

/// The same for Windows, where BYOND's function keeps the registers it
/// should, so the call's own arguments serve.
#[cfg(windows)]
#[unsafe(naked)]
unsafe extern "C" fn call_proc_by_id_entry() {
	core::arch::naked_asm!(
		"cmp byte ptr [{work}], 0",
		"jne 2f",
		"cmp byte ptr [{load}], 0",
		"jne 2f",
		"cmp byte ptr [{intercepted}], 0",
		"jne 2f",
		// EAX is free at a function's entry
		"movzx eax, word ptr [esp + {proc_id}]",
		"cmp byte ptr [{hooked} + eax], 0",
		"jne 2f",
		"jmp dword ptr [{original}]",
		"2:",
		"jmp {trampoline}",
		work = sym crate::PROC_CALL_WORK_PENDING,
		load = sym crate::WORLD_LOAD_PENDING,
		intercepted = sym runtime_intercepted,
		proc_id = const PROC_ID_ARGUMENT,
		hooked = sym HOOKED_LOW_WORDS,
		original = sym call_proc_by_id_original,
		trampoline = sym call_proc_by_id_hook_trampoline
	)
}

fn hook_by_id(id: raw_types::procs::ProcId, hook: ProcHook, hook_path: String) -> Result<(), HookFailure> {
	let mut map = PROC_HOOKS.0.borrow_mut();
	if let std::collections::hash_map::Entry::Vacant(e) = map.entry(id) {
		e.insert((hook, hook_path));
		unsafe {
			let count = &mut HOOKED_LOW_WORDS[id.0 as usize & 0xFFFF];
			*count = count.saturating_add(1);
		}
		Ok(())
	} else {
		Err(HookFailure::AlreadyHooked)
	}
}

pub fn clear_hooks() {
	PROC_HOOKS.0.borrow_mut().clear();
	unsafe {
		HOOKED_LOW_WORDS.fill(0);
	}
}

pub fn hook<S: Into<String>>(name: S, hook: ProcHook) -> Result<(), HookFailure> {
	match super::proc::get_proc(name) {
		Some(p) => hook_by_id(p.id, hook, p.path.to_owned()),
		None => Err(HookFailure::ProcNotFound)
	}
}

impl Proc {
	pub fn hook(&self, func: ProcHook) -> Result<(), HookFailure> {
		hook_by_id(self.id, func, self.path.to_owned())
	}

	/// Whether this library installed a Rust hook for the proc. Hooks in other
	/// auxtools libraries loaded into the same world are not visible here.
	pub fn is_hooked(&self) -> bool {
		PROC_HOOKS.0.borrow().contains_key(&self.id)
	}
}

#[no_mangle]
extern "C" fn on_runtime(error: *const c_char) {
	let str = crate::text_of(unsafe { CStr::from_ptr(error) }.to_bytes());

	for func in inventory::iter::<RuntimeErrorHook> {
		func.0(&str);
	}
}

/// Whether interpreter calls bypass the call_proc_by_id detour. GCC inlined
/// them on Linux before 516.1664. Those builds dispatch proc hooks through
/// the instruction hook (see `proc_starting`).
pub fn calls_bypass_detour() -> bool {
	cfg!(unix) && crate::version::get().1 < 1664
}

/// What a hooked frame runs instead of its code: `End`, which returns `.`
static RETURN_DOT: [u32; 1] = [0x00];

/// Dispatch deferred work and proc hooks at the first instruction on builds
/// where [`calls_bypass_detour`] is true. A hook's result replaces execution
/// of the original proc.
///
/// # Safety
/// `ctx` must be the running frame on BYOND's thread.
pub unsafe fn proc_starting(ctx: *mut raw_types::procs::ExecutionContext) {
	if std::ptr::eq(*(*ctx).bytecode() as *const u32, RETURN_DOT.as_ptr()) {
		return;
	}
	let instance = *(*ctx).proc_instance();
	// The startup call can run code replaced during WorldLoad::ready
	// (see `start_runs_new_code`).
	if crate::on_proc_call((*instance).proc, (*instance).src.tag) && *(*ctx).bytecode_offset() == 0 {
		if let Some(proc) = Proc::from_id((*instance).proc) {
			let (now, _) = proc.bytecode_mut_ptr();
			*(*ctx).bytecode_mut() = now;
		}
	}

	let proc_id = (*instance).proc;
	let Some(hook) = PROC_HOOKS.0.borrow().get(&proc_id).map(|(hook, _)| *hook) else {
		return;
	};
	// The frame keeps its own references to these
	let src = Value::from_raw((*instance).src);
	let usr = Value::from_raw((*instance).usr);
	let args = std::slice::from_raw_parts((*instance).args(), (*instance).args_count() as usize)
		.iter()
		.map(|v| Value::from_raw(*v))
		.collect();
	let result = match hook(&src, &usr, args) {
		Ok(r) => {
			let raw = r.raw;
			std::mem::forget(r);
			raw
		}
		Err(e) => {
			let path = PROC_HOOKS.0.borrow()[&proc_id].1.clone();
			if let Some(trace) = Proc::find("/proc/auxtools_stack_trace") {
				let _ = trace.call(&[&Value::from_string(format!("{} HookPath: {}", e.message.as_str(), path)).unwrap()]);
			}
			Value::NULL.raw
		}
	};
	// `.` is null at a proc's start; it takes our reference
	*(*ctx).dot_mut() = result;
	*(*ctx).bytecode_mut() = RETURN_DOT.as_ptr() as *mut u32;
	*(*ctx).bytecode_offset_mut() = 0;
}

#[no_mangle]
extern "C" fn call_proc_by_id_hook(
	ret: *mut raw_types::values::Value,
	usr_raw: raw_types::values::Value,
	_proc_type: u32,
	proc_id: raw_types::procs::ProcId,
	_override_depth: u32,
	src_raw: raw_types::values::Value,
	args_ptr: *mut raw_types::values::Value,
	num_args: usize,
	// the trampoline deals with these two once we return
	_callback: u32,
	_callback_value: u32
) -> u8 {
	crate::on_proc_call(proc_id, src_raw.tag);

	// copy the fn out so no borrow is held while the hook runs DM code, which
	// can call back into here or register hooks
	let Some(hook) = PROC_HOOKS.0.borrow().get(&proc_id).map(|(hook, _)| *hook) else {
		return 0;
	};

	let (src, usr, args) = unsafe {
		(
			Value::from_raw(src_raw),
			Value::from_raw(usr_raw),
			// Taking ownership of args here
			std::slice::from_raw_parts(args_ptr, num_args)
				.iter()
				.map(|v| Value::from_raw_owned(*v))
				.collect()
		)
	};

	let result = match hook(&src, &usr, args) {
		Ok(r) => {
			let result_raw = r.raw;
			// Stealing our reference out of the Value
			std::mem::forget(r);
			result_raw
		}
		Err(e) => {
			let path = PROC_HOOKS.0.borrow()[&proc_id].1.clone();
			Proc::find("/proc/auxtools_stack_trace")
				.unwrap()
				.call(&[&Value::from_string(format!("{} HookPath: {}", e.message.as_str(), path)).unwrap()])
				.unwrap();
			Value::NULL.raw
		}
	};
	unsafe {
		*ret = result;
	}
	1
}
