use std::{
	ffi::c_void,
	sync::{
		atomic::{AtomicBool, AtomicUsize, Ordering},
		Mutex, OnceLock
	}
};

use retour::RawDetour;

use crate::raw_types;

extern "C" {
	static mut execute_instruction_original: *const c_void;

	fn execute_instruction_hook();

	#[cfg(unix)]
	fn execute_instruction_hook_edi();
}

type Handler = fn(*mut raw_types::procs::ExecutionContext);

static mut HANDLER: Option<Handler> = None;
static mut TIMED: bool = false;
static mut ENTERED: u64 = 0;
static mut LEFT: u64 = 0;
static mut CALLS_BYPASS_DETOUR: bool = false;
static mut DETOURED: bool = false;

/// The handler runs without a panic guard. A panic that leaves it aborts the
/// game.
pub fn set_handler(handler: Handler) -> Result<(), String> {
	detour()?;
	unsafe {
		HANDLER = Some(handler);
	}
	Ok(())
}

const MOST_OPCODES: usize = 0x200;

#[repr(C)]
struct FrameStart {
	proc_instance: *mut c_void,
	parent: *mut c_void,
	filename: u32,
	line: u32,
	bytecode: *mut u32,
	offset: u16
}

struct Table {
	entries: *const AtomicUsize,
	count: usize,
	handler_stub: usize,
	first_stub: usize
}

unsafe impl Send for Table {}
unsafe impl Sync for Table {}

static TABLE: OnceLock<Table> = OnceLock::new();
static LISTENING: Mutex<()> = Mutex::new(());
static ALL: AtomicBool = AtomicBool::new(true);

static mut ORIGINAL_TABLE: [usize; MOST_OPCODES] = [0; MOST_OPCODES];
static mut DISPATCH_START: usize = 0;
static mut UNKNOWN_ORIGINAL: usize = 0;
static mut UNKNOWN_NEXT: usize = 0;
static mut PASS: u8 = 0;
static mut PASS_UNKNOWN: bool = false;

/// Sends every opcode to the handler. Any thread may call it to bring pending
/// work to the handler.
pub fn hear_all() {
	let Some(table) = TABLE.get() else {
		return;
	};
	if ALL.load(Ordering::SeqCst) {
		return;
	}
	let _writing = LISTENING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
	if ALL.load(Ordering::SeqCst) {
		return;
	}
	for opcode in 0..table.count {
		unsafe { (*table.entries.add(opcode)).store(table.handler_stub, Ordering::SeqCst) };
	}
	ALL.store(true, Ordering::SeqCst);
}

/// Call it on BYOND's thread. Then check pending work again, because another
/// thread may have called `hear_all` meanwhile.
pub fn hear_only(opcodes: &[u32]) -> bool {
	let Some(table) = TABLE.get() else {
		return false;
	};
	let _writing = LISTENING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
	ALL.store(false, Ordering::SeqCst);
	let originals = unsafe { &*std::ptr::addr_of!(ORIGINAL_TABLE) };
	for (opcode, original) in originals.iter().enumerate().take(table.count) {
		let entry = if opcodes.contains(&(opcode as u32)) {
			table.handler_stub
		} else if table.first_stub != 0 {
			table.first_stub
		} else {
			*original
		};
		unsafe { (*table.entries.add(opcode)).store(entry, Ordering::SeqCst) };
	}
	true
}

unsafe fn about_to_run(ctx: *mut raw_types::procs::ExecutionContext) -> (*mut u32, u16, usize) {
	unsafe {
		let frame = ctx as *const FrameStart;
		let (bytecode, offset) = ((*frame).bytecode, (*frame).offset);
		(bytecode, offset, *bytecode.add(offset as usize) as usize)
	}
}

unsafe fn opcode_at(ctx: *mut raw_types::procs::ExecutionContext) -> usize {
	unsafe { about_to_run(ctx).2 }
}

unsafe fn pass_next(table: &Table, ctx: *mut raw_types::procs::ExecutionContext) {
	unsafe {
		let opcode = opcode_at(ctx);
		if opcode >= table.count {
			PASS_UNKNOWN = true;
		} else {
			let entry = (*table.entries.add(opcode)).load(Ordering::SeqCst);
			if entry == table.handler_stub || (table.first_stub != 0 && entry == table.first_stub && about_to_run(ctx).1 == 0) {
				PASS = 1;
			}
		}
	}
}

extern "C" fn instruction_dispatched(ctx: *mut raw_types::procs::ExecutionContext) -> u32 {
	unsafe {
		let before = about_to_run(ctx);
		handle_instruction(ctx);
		if about_to_run(ctx) == before {
			return 0;
		}
		if let Some(table) = TABLE.get() {
			pass_next(table, ctx);
		}
		1
	}
}

#[cfg(unix)]
extern "C" fn instruction_first(ctx: *mut raw_types::procs::ExecutionContext) -> u32 {
	unsafe {
		let before = about_to_run(ctx);
		crate::hooks::proc_starting(ctx);
		if about_to_run(ctx) == before {
			return 0;
		}
		if let Some(table) = TABLE.get() {
			pass_next(table, ctx);
		}
		1
	}
}

extern "C" fn instruction_unknown(ctx: *mut raw_types::procs::ExecutionContext) {
	unsafe {
		let Some(table) = TABLE.get() else {
			UNKNOWN_NEXT = UNKNOWN_ORIGINAL;
			return;
		};
		if std::mem::take(&mut PASS_UNKNOWN) || opcode_at(ctx) < table.count {
			UNKNOWN_NEXT = UNKNOWN_ORIGINAL;
			return;
		}
		handle_instruction(ctx);
		pass_next(table, ctx);
		UNKNOWN_NEXT = DISPATCH_START;
	}
}

macro_rules! table_stub {
	($name:ident, $opcode:literal, $frame:literal) => {
		table_stub!($name, $opcode, $frame, instruction_dispatched, "");
	};
	($name:ident, $opcode:literal, $frame:literal, first $called:ident) => {
		table_stub!($name, $opcode, $frame, $called, concat!("cmp word ptr [", $frame, " + 0x14], 0\njne 3f"));
	};
	($name:ident, $opcode:literal, $frame:literal, $called:ident, $only:expr) => {
		#[unsafe(naked)]
		unsafe extern "C" fn $name() {
			core::arch::naked_asm!(
				$only,
				"cmp byte ptr [{pass}], 0",
				"jne 2f",
				"push eax",
				"push ecx",
				"push edx",
				concat!("push ", $frame),
				"call {dispatched}",
				"add esp, 4",
				"pop edx",
				"pop ecx",
				"test eax, eax",
				"pop eax",
				"jz 3f",
				"jmp dword ptr [{start}]",
				"2:",
				"mov byte ptr [{pass}], 0",
				"3:",
				concat!("jmp dword ptr [{original} + ", $opcode, "*4]"),
				pass = sym PASS,
				dispatched = sym $called,
				start = sym DISPATCH_START,
				original = sym ORIGINAL_TABLE
			)
		}
	};
}

macro_rules! unknown_stub {
	($name:ident, $frame:literal) => {
		#[unsafe(naked)]
		unsafe extern "C" fn $name() {
			core::arch::naked_asm!(
				"push eax",
				"push ecx",
				"push edx",
				concat!("push ", $frame),
				"call {unknown}",
				"add esp, 4",
				"pop edx",
				"pop ecx",
				"pop eax",
				"jmp dword ptr [{next}]",
				unknown = sym instruction_unknown,
				next = sym UNKNOWN_NEXT
			)
		}
	};
}

#[cfg(unix)]
table_stub!(table_stub_ebx_esi, "ebx", "esi");
#[cfg(unix)]
table_stub!(first_stub_ebx_esi, "ebx", "esi", first instruction_first);
#[cfg(unix)]
unknown_stub!(unknown_stub_esi, "esi");
#[cfg(unix)]
table_stub!(table_stub_ecx_edi, "ecx", "edi");
#[cfg(unix)]
table_stub!(first_stub_ecx_edi, "ecx", "edi", first instruction_first);
#[cfg(windows)]
table_stub!(table_stub_edx_edi, "edx", "edi");
unknown_stub!(unknown_stub_edi, "edi");

#[cfg(unix)]
unsafe fn writable(start: usize, length: usize) -> bool {
	extern "C" {
		fn mprotect(address: *mut c_void, length: usize, protection: i32) -> i32;
	}
	const PAGE: usize = 4096;
	const READ_WRITE: i32 = 3;
	const EXECUTE: i32 = 4;
	let from = start & !(PAGE - 1);
	let to = (start + length + PAGE - 1) & !(PAGE - 1);
	let Ok(maps) = std::fs::read_to_string("/proc/self/maps") else {
		return false;
	};
	let (mut mapped, mut executable) = (false, false);
	for line in maps.lines() {
		let mut fields = line.split_ascii_whitespace();
		let (Some(range), Some(access)) = (fields.next(), fields.next()) else {
			continue;
		};
		let Some((low, high)) = range.split_once('-') else {
			continue;
		};
		let (Ok(low), Ok(high)) = (usize::from_str_radix(low, 16), usize::from_str_radix(high, 16)) else {
			continue;
		};
		if low < to && from < high {
			mapped = true;
			executable |= access.contains('x');
		}
	}
	if !mapped {
		return false;
	}
	let protection = if executable { READ_WRITE | EXECUTE } else { READ_WRITE };
	unsafe { mprotect(from as *mut c_void, to - from, protection) == 0 }
}

#[cfg(windows)]
unsafe fn writable(start: usize, length: usize) -> bool {
	extern "system" {
		fn VirtualProtect(address: *mut c_void, size: usize, protection: u32, old: *mut u32) -> i32;
	}
	const PAGE_READONLY: u32 = 0x02;
	const PAGE_READWRITE: u32 = 0x04;
	const PAGE_WRITECOPY: u32 = 0x08;
	const PAGE_EXECUTE_READWRITE: u32 = 0x40;
	unsafe {
		let mut old = 0;
		if VirtualProtect(start as *mut c_void, length, PAGE_EXECUTE_READWRITE, &mut old) == 0 {
			return false;
		}
		if matches!(old, PAGE_READONLY | PAGE_READWRITE | PAGE_WRITECOPY) {
			let mut ignored = 0;
			return VirtualProtect(start as *mut c_void, length, PAGE_READWRITE, &mut ignored) != 0;
		}
		true
	}
}

unsafe fn take_table(execute_instruction: usize) -> Result<(), &'static str> {
	unsafe {
		let code = std::slice::from_raw_parts(execute_instruction as *const u8, 96);
		let word = |at: usize| u32::from_le_bytes([code[at], code[at + 1], code[at + 2], code[at + 3]]) as usize;
		if code[3] != 0x14 || code[6] != 0x10 {
			return Err("frame layout differs from the stubs'");
		}
		let mut found = None;
		for at in 0..code.len() - 20 {
			let (index, table_stub, first_stub, unknown_stub): (u8, usize, usize, usize) = match (code[at], code[at + 1]) {
				#[cfg(unix)]
				(0x81, 0xFB) => (
					0x9D,
					table_stub_ebx_esi as *const () as usize,
					first_stub_ebx_esi as *const () as usize,
					unknown_stub_esi as *const () as usize
				),
				#[cfg(unix)]
				(0x81, 0xF9) => (
					0x8D,
					table_stub_ecx_edi as *const () as usize,
					first_stub_ecx_edi as *const () as usize,
					unknown_stub_edi as *const () as usize
				),
				#[cfg(windows)]
				(0x81, 0xFA) => (0x95, table_stub_edx_edi as *const () as usize, 0, unknown_stub_edi as *const () as usize),
				_ => continue
			};
			let count = word(at + 2) + 1;
			let (unknown, jump) = match (code[at + 6], code[at + 7]) {
				(0x0F, 0x87) => ((execute_instruction + at + 12).wrapping_add(word(at + 8)), at + 12),
				(0x77, short) => ((execute_instruction + at + 8).wrapping_add(short as i8 as usize), at + 8),
				_ => continue
			};
			if code[jump..jump + 3] != [0xFF, 0x24, index] {
				continue;
			}
			found = Some((word(jump + 3), count, unknown, table_stub, first_stub, unknown_stub));
			break;
		}
		let (entries, count, unknown, table_stub, first_stub, unknown_stub) = found.ok_or("opcode jump table not found")?;
		if count > MOST_OPCODES {
			return Err("more opcodes than expected");
		}
		let entries = entries as *const AtomicUsize;
		const NEAR: usize = 0x0010_0000;
		let originals = &mut *std::ptr::addr_of_mut!(ORIGINAL_TABLE);
		for (opcode, original) in originals.iter_mut().enumerate().take(count) {
			let entry = (*entries.add(opcode)).load(Ordering::Relaxed);
			if entry.abs_diff(execute_instruction) > NEAR {
				return Err("jump-table entry is outside the interpreter");
			}
			*original = entry;
		}
		if !writable(entries as usize, count * std::mem::size_of::<usize>()) {
			return Err("cannot make jump table writable");
		}

		let hook = RawDetour::new(unknown as *const (), unknown_stub as *const ()).map_err(|_| "cannot create unknown-opcode detour")?;
		DISPATCH_START = execute_instruction;
		UNKNOWN_ORIGINAL = hook.trampoline() as *const () as usize;
		UNKNOWN_NEXT = UNKNOWN_ORIGINAL;
		hook.enable().map_err(|_| "cannot enable unknown-opcode detour")?;
		std::mem::forget(hook);

		let table = TABLE.get_or_init(|| Table {
			entries,
			count,
			handler_stub: table_stub,
			first_stub: if CALLS_BYPASS_DETOUR { first_stub } else { 0 }
		});
		let _writing = LISTENING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
		ALL.store(true, Ordering::SeqCst);
		for opcode in 0..count {
			(*table.entries.add(opcode)).store(table.handler_stub, Ordering::SeqCst);
		}
		Ok(())
	}
}

pub fn time_instructions(timed: bool) {
	unsafe {
		TIMED = timed;
	}
}

pub fn timestamp() -> u64 {
	unsafe { core::arch::x86::_rdtsc() }
}

/// Returns `None` when nested execution spoiled the interval.
pub fn ticks_since_last() -> Option<u64> {
	unsafe { ENTERED.checked_sub(LEFT) }
}

pub(crate) fn init() -> Result<(), String> {
	unsafe {
		CALLS_BYPASS_DETOUR = crate::hooks::calls_bypass_detour();
		if CALLS_BYPASS_DETOUR {
			detour()?;
		}
	}
	Ok(())
}

fn detour() -> Result<(), String> {
	unsafe {
		if DETOURED {
			return Ok(());
		}
	}
	let execute_instruction = crate::registry::find!("execute_instruction")?;

	if unsafe { take_table(execute_instruction) }.is_ok() {
		unsafe {
			DETOURED = true;
		}
		return Ok(());
	}

	#[cfg(windows)]
	let versioned_hook = execute_instruction_hook as *const ();
	#[cfg(unix)]
	let versioned_hook = if crate::version::get().1 < crate::registry::builds::LINUX_FRAMELESS {
		execute_instruction_hook_edi as *const ()
	} else {
		execute_instruction_hook as *const ()
	};

	unsafe {
		let hook = RawDetour::new(execute_instruction as *const (), versioned_hook).map_err(|_| "Couldn't detour execute_instruction")?;

		hook.enable().map_err(|_| "Couldn't enable execute_instruction detour")?;

		execute_instruction_original = hook.trampoline() as *const () as *const c_void;

		std::mem::forget(hook);
		DETOURED = true;
	}
	Ok(())
}

#[no_mangle]
extern "C" fn handle_instruction(ctx: *mut raw_types::procs::ExecutionContext) -> *const raw_types::procs::ExecutionContext {
	unsafe {
		if TIMED {
			ENTERED = timestamp();
		}
		if CALLS_BYPASS_DETOUR && (*(ctx as *const FrameStart)).offset == 0 {
			crate::hooks::proc_starting(ctx);
		}
		if let Some(handler) = HANDLER {
			handler(ctx);
		}
		if TIMED {
			LEFT = timestamp();
		}
	}

	ctx
}
