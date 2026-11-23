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

pub(crate) fn init() -> Result<(), String> {
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

pub(crate) fn shutdown() {
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

struct GameThreadOnly<T>(T);

unsafe impl<T> Sync for GameThreadOnly<T> {}

static PROC_HOOKS: GameThreadOnly<RefCell<FxHashMap<raw_types::procs::ProcId, (ProcHook, String)>>> =
	GameThreadOnly(RefCell::new(HashMap::with_hasher(BuildHasherDefault::new())));

static mut HOOKED_LOW_WORDS: [u8; 0x10000] = [0; 0x10000];

#[cfg(unix)]
const PROC_ID_ARGUMENT: usize = 0x14;
#[cfg(windows)]
const PROC_ID_ARGUMENT: usize = 0x10;

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
		"movzx eax, word ptr [esp + {proc_id}]",
		"cmp byte ptr [{hooked} + eax], 0",
		"jne 2f",
		"inc dword ptr [{calls}]",
		"push ebp",
		"push ebx",
		"push esi",
		"push edi",
		"sub esp, 12",
		".rept 12",
		"push dword ptr [esp + 76]",
		".endr",
		"call dword ptr [{original}]",
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
		calls = sym crate::PROC_CALLS,
		original = sym call_proc_by_id_original,
		trampoline = sym call_proc_by_id_hook_trampoline
	)
}

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
		"movzx eax, word ptr [esp + {proc_id}]",
		"cmp byte ptr [{hooked} + eax], 0",
		"jne 2f",
		"inc dword ptr [{calls}]",
		"jmp dword ptr [{original}]",
		"2:",
		"jmp {trampoline}",
		work = sym crate::PROC_CALL_WORK_PENDING,
		load = sym crate::WORLD_LOAD_PENDING,
		intercepted = sym runtime_intercepted,
		proc_id = const PROC_ID_ARGUMENT,
		hooked = sym HOOKED_LOW_WORDS,
		calls = sym crate::PROC_CALLS,
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

pub(crate) fn clear_hooks() {
	PROC_HOOKS.0.borrow_mut().clear();
	unsafe {
		HOOKED_LOW_WORDS.fill(0);
	}
}

pub fn hook<S: Into<String>>(name: S, hook: ProcHook) -> Result<(), HookFailure> {
	if !crate::world_is_live() {
		return Err(HookFailure::NotInitialized);
	}
	match super::proc::get_proc(name) {
		Some(p) => hook_by_id(p.id, hook, p.path.to_owned()),
		None => Err(HookFailure::ProcNotFound)
	}
}

impl Proc {
	pub fn hook(&self, func: ProcHook) -> Result<(), HookFailure> {
		if self.entry().is_null() {
			return Err(HookFailure::ProcNotFound);
		}
		hook_by_id(self.id, func, self.path.to_owned())
	}

	/// Doesn't see hooks from other auxtools libraries in the same world.
	pub fn is_hooked(&self) -> bool {
		PROC_HOOKS.0.borrow().contains_key(&self.id)
	}
}

#[no_mangle]
extern "C" fn on_runtime(error: *const c_char) {
	let str = crate::text_of(unsafe { CStr::from_ptr(error) }.to_bytes());

	for func in inventory::iter::<RuntimeErrorHook> {
		let _ = crate::guard(|| func.0(&str));
	}
}

fn call_hook(hook: ProcHook, src: &Value, usr: &Value, args: Vec<Value>) -> DMResult {
	crate::guard(|| hook(src, usr, args)).unwrap_or_else(|panic| Err(crate::runtime!("panicked: {}", crate::panic_message(&*panic))))
}

fn hook_failed(proc_id: raw_types::procs::ProcId, error: &crate::Runtime) {
	if !crate::world_is_live() {
		return;
	}
	let path = PROC_HOOKS.0.borrow().get(&proc_id).map(|(_, path)| path.clone()).unwrap_or_default();
	if let (Some(trace), Ok(message)) = (
		Proc::find("/proc/auxtools_stack_trace"),
		Value::from_string(format!("{} HookPath: {}", error.message, path))
	) {
		let _ = trace.call(&[&message]);
	}
}

/// On Linux before 516.1664, GCC inlined the interpreter's proc calls. They
/// don't reach the detour.
pub fn calls_bypass_detour() -> bool {
	cfg!(unix) && crate::version::get().1 < 1664
}

static RETURN_DOT: [u32; 1] = [0x00];

/// # Safety
/// `ctx` must be the running frame on BYOND's thread.
pub unsafe fn proc_starting(ctx: *mut raw_types::procs::ExecutionContext) {
	if std::ptr::eq(*(*ctx).bytecode() as *const u32, RETURN_DOT.as_ptr()) {
		return;
	}
	let instance = *(*ctx).proc_instance();
	if crate::on_proc_call((*instance).proc, (*instance).src.tag) && *(*ctx).bytecode_offset() == 0 && crate::world_is_live() {
		if let Some(proc) = Proc::from_id((*instance).proc) {
			let (now, _) = proc.bytecode_mut_ptr();
			*(*ctx).bytecode_mut() = now;
		}
	}

	let proc_id = (*instance).proc;
	if HOOKED_LOW_WORDS[proc_id.0 as usize & 0xFFFF] == 0 {
		return;
	}
	let Some(hook) = PROC_HOOKS.0.borrow().get(&proc_id).map(|(hook, _)| *hook) else {
		return;
	};
	let src = Value::from_raw((*instance).src);
	let usr = Value::from_raw((*instance).usr);
	let args = std::slice::from_raw_parts((*instance).args(), (*instance).args_count() as usize)
		.iter()
		.map(|v| Value::from_raw(*v))
		.collect();
	let result = match call_hook(hook, &src, &usr, args) {
		Ok(r) => {
			let raw = r.raw;
			std::mem::forget(r);
			raw
		}
		Err(e) => {
			hook_failed(proc_id, &e);
			Value::NULL.raw
		}
	};
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
	_callback: u32,
	_callback_value: u32
) -> u8 {
	crate::on_proc_call(proc_id, src_raw.tag);

	let Some(hook) = PROC_HOOKS.0.borrow().get(&proc_id).map(|(hook, _)| *hook) else {
		return 0;
	};

	let (src, usr, args) = unsafe {
		(
			Value::from_raw(src_raw),
			Value::from_raw(usr_raw),
			std::slice::from_raw_parts(args_ptr, num_args)
				.iter()
				.map(|v| Value::from_raw_owned(*v))
				.collect()
		)
	};

	let result = match call_hook(hook, &src, &usr, args) {
		Ok(r) => {
			let result_raw = r.raw;
			std::mem::forget(r);
			result_raw
		}
		Err(e) => {
			hook_failed(proc_id, &e);
			Value::NULL.raw
		}
	};
	unsafe {
		*ret = result;
	}
	1
}
