//! Intercept interpreter instructions for debugger callbacks.
//! Proc hooks also require this on builds where interpreter calls bypass
//! call_proc_by_id; see `hooks::calls_bypass_detour`.
//!
//! Redirect opcode jump-table entries through a handler stub. `hear_only`
//! restores BYOND's entries for unselected opcodes, avoiding hook overhead.
//! Unknown opcodes, including breakpoints, use a separate detour. Fall back
//! to detouring opcode dispatch when the table cannot be used.

use std::{
	ffi::c_void,
	sync::{
		atomic::{AtomicBool, AtomicUsize, Ordering},
		Mutex, OnceLock
	}
};

use byond_scan::{Anchor, Extract, Recipe, SignatureTreatment, VersionRange};
use retour::RawDetour;

use crate::raw_types;

// Where the interpreter reads the next opcode, inside exec_proc.
//
// The running proc's frame sits in EDI here and in ESI on Linux, so each
// platform has its own assembly in execute_instruction_hook.*.
#[cfg(windows)]
const DISPATCH_MASK: &str = "0F B7 47 ?? 8B 4F ?? 8B F0 8B 14 ?? 89 95 ?? ?? ?? ?? 81 FA ?? 01 00 00";
// Linux 515 before 515.1644: the frame is in EDI
#[cfg(unix)]
const DISPATCH_MASK_EARLY_515: &str = "0F B7 57 14 8B 47 10 0F B7 DA 8B 0C 98 81 F9 ?? 01 00 00 77 ?? FF 24 8D ?? ?? ?? ??";
#[cfg(unix)]
const DISPATCH_MASK_515: &str = "0F B7 46 14 8B 56 10 8D 3C 85 00 00 00 00 66 89 85 ?? ?? ?? ?? 89 85 ?? ?? ?? ?? 8D 04 3A 8B 18 81 FB ?? 01 00 00 \
                                 0F 87 ?? ?? ?? ?? FF 24 9D ?? ?? ?? ??";
#[cfg(unix)]
const DISPATCH_MASK: &str = "0F B7 46 ?? 8B 56 ?? 66 89 85 ?? ?? ?? ?? 89 85 ?? ?? ?? ?? C1 E0 02 89 85 ?? ?? ?? ?? 01 D0 8B 18 81 FB ?? 01 00 00 \
                             0F 87 ?? ?? ?? ?? FF 24 9D ?? ?? ?? ??";

#[cfg(windows)]
const CHECKED: VersionRange = VersionRange { min: 1637, max: 1688 };
#[cfg(unix)]
const CHECKED: VersionRange = VersionRange { min: 1648, max: 1688 };

const EXECUTE_INSTRUCTION: Recipe = Recipe {
	name: "execute_instruction",
	versions: CHECKED,
	anchor: Anchor::Signature(SignatureTreatment::NoOffset, DISPATCH_MASK),
	hops: &[],
	extract: Extract::Entry
};

// The frame is in ESI from 515.1644, as on 516; in EDI before
#[cfg(unix)]
const EXECUTE_INSTRUCTIONS: &[Recipe] = &[
	EXECUTE_INSTRUCTION,
	Recipe {
		name: "execute_instruction",
		versions: VersionRange { min: 1644, max: 1647 },
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, DISPATCH_MASK_515),
		hops: &[],
		extract: Extract::Entry
	},
	Recipe {
		name: "execute_instruction",
		versions: VersionRange { min: 1637, max: 1643 },
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, DISPATCH_MASK_EARLY_515),
		hops: &[],
		extract: Extract::Entry
	}
];
#[cfg(windows)]
const EXECUTE_INSTRUCTIONS: &[Recipe] = &[EXECUTE_INSTRUCTION];

extern "C" {
	// Trampoline to the original un-hooked BYOND execute_instruction code
	static mut execute_instruction_original: *const c_void;

	// Assembly entry point with a nonstandard calling convention.
	// Do not call directly from Rust.
	fn execute_instruction_hook();

	// For Linux 515 before 515.1644, where the frame is in EDI
	#[cfg(unix)]
	fn execute_instruction_hook_edi();
}

type Handler = fn(*mut raw_types::procs::ExecutionContext);

// Read on every instruction
static mut HANDLER: Option<Handler> = None;
// Timestamps around the handler, taken while `TIMED`
static mut TIMED: bool = false;
static mut ENTERED: u64 = 0;
static mut LEFT: u64 = 0;
static mut CALLS_BYPASS_DETOUR: bool = false;
// The detour stays for the life of the process, through reboots
static mut DETOURED: bool = false;

/// Install a handler called before each instruction with its running frame.
/// Replaces the previous handler.
///
/// The handler runs unguarded, to keep the cost of an instruction down: a
/// panic that leaves it aborts the game. It catches its own with
/// [`crate::guard`] where it can panic.
pub fn set_handler(handler: Handler) -> Result<(), String> {
	detour()?;
	unsafe {
		HANDLER = Some(handler);
	}
	Ok(())
}

/// Capacity of `ORIGINAL_TABLE`. `take_table` reads the build's opcode count
/// (0x18D in 516, 0x179 in 515). Larger tables fall back to the detour.
const MOST_OPCODES: usize = 0x200;

/// Common frame prefix. Read bytecode and offset at 0x10 and 0x14 directly
/// to avoid accessor calls on every instruction. See the dispatch masks.
#[repr(C)]
struct FrameStart {
	proc_instance: *mut c_void,
	parent: *mut c_void,
	filename: u32,
	line: u32,
	bytecode: *mut u32,
	offset: u16
}

/// Redirected interpreter jump table.
struct Table {
	entries: *const AtomicUsize,
	/// Number of opcode entries.
	count: usize,
	/// Stub that invokes the instruction handler.
	stub: usize,
	/// Stub for unselected opcodes on builds requiring proc-entry hooks.
	/// Zero on other builds, which use BYOND's original entries.
	first_stub: usize
}

// SAFETY: entries are updated atomically while holding `LISTENING`.
unsafe impl Send for Table {}
unsafe impl Sync for Table {}

static TABLE: OnceLock<Table> = OnceLock::new();
/// Serializes table updates.
static LISTENING: Mutex<()> = Mutex::new(());
/// Whether every opcode invokes the handler.
static ALL: AtomicBool = AtomicBool::new(true);

// Stub state. Only BYOND's thread writes after table installation.
/// Original interpreter jump targets.
static mut ORIGINAL_TABLE: [usize; MOST_OPCODES] = [0; MOST_OPCODES];
/// Where the interpreter reads the next instruction
static mut DISPATCH_START: usize = 0;
/// Original unknown-opcode handler, past the detour.
static mut UNKNOWN_ORIGINAL: usize = 0;
/// Next jump target for the unknown-opcode stub.
static mut UNKNOWN_NEXT: usize = 0;
/// Skip the handler once when the interpreter rereads an instruction.
static mut PASS: u8 = 0;
/// Skip the unknown-opcode handler once on redispatch.
static mut PASS_UNKNOWN: bool = false;

/// Restore callbacks for every instruction. May be called from any thread
/// to notify the handler of pending work.
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
		unsafe { (*table.entries.add(opcode)).store(table.stub, Ordering::SeqCst) };
	}
	// Set only once every entry is written: a caller returning early on the
	// flag relies on the whole table.
	ALL.store(true, Ordering::SeqCst);
}

/// Limit callbacks to `opcodes` and unknown opcodes until `hear_all`.
/// Call on BYOND's thread, then recheck pending work in case another thread
/// called `hear_all` during the update.
///
/// Builds with `hooks::calls_bypass_detour` retain a proc-entry stub for
/// unselected opcodes. Other builds restore BYOND's original entries.
/// Returns false if no table is available, leaving all callbacks enabled.
pub fn hear_only(opcodes: &[u32]) -> bool {
	let Some(table) = TABLE.get() else {
		return false;
	};
	let _writing = LISTENING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
	ALL.store(false, Ordering::SeqCst);
	for opcode in 0..table.count {
		let entry = if opcodes.contains(&(opcode as u32)) {
			table.stub
		} else if table.first_stub != 0 {
			table.first_stub
		} else {
			unsafe { ORIGINAL_TABLE[opcode] }
		};
		unsafe { (*table.entries.add(opcode)).store(entry, Ordering::SeqCst) };
	}
	true
}

/// Bytecode pointer, offset and opcode at the current frame position.
unsafe fn about_to_run(ctx: *mut raw_types::procs::ExecutionContext) -> (*mut u32, u16, usize) {
	unsafe {
		let frame = ctx as *const FrameStart;
		let (bytecode, offset) = ((*frame).bytecode, (*frame).offset);
		(bytecode, offset, *bytecode.add(offset as usize) as usize)
	}
}

/// The opcode a frame is about to run
unsafe fn opcode_at(ctx: *mut raw_types::procs::ExecutionContext) -> usize {
	unsafe { about_to_run(ctx).2 }
}

/// Skip the next callback if redispatch will reach a stub again.
unsafe fn pass_next(table: &Table, ctx: *mut raw_types::procs::ExecutionContext) {
	unsafe {
		let opcode = opcode_at(ctx);
		if opcode >= table.count {
			PASS_UNKNOWN = true;
		} else {
			let entry = (*table.entries.add(opcode)).load(Ordering::SeqCst);
			// The proc-entry stub checks PASS only at offset zero.
			if entry == table.stub || (table.first_stub != 0 && entry == table.first_stub && about_to_run(ctx).1 == 0) {
				PASS = 1;
			}
		}
	}
}

/// Invoke the handler. Return 0 to run the original opcode, or 1 to
/// redispatch if the handler changed the bytecode, offset or opcode.
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

/// Run proc-entry hooks and pending work on builds that bypass the call
/// detour. Returns the same redispatch flag as `instruction_dispatched`.
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

/// Handle an unknown opcode and set `UNKNOWN_NEXT`.
extern "C" fn instruction_unknown(ctx: *mut raw_types::procs::ExecutionContext) {
	unsafe {
		let Some(table) = TABLE.get() else {
			UNKNOWN_NEXT = UNKNOWN_ORIGINAL;
			return;
		};
		// Already handled, or entered from outside opcode dispatch.
		if std::mem::take(&mut PASS_UNKNOWN) || opcode_at(ctx) < table.count {
			UNKNOWN_NEXT = UNKNOWN_ORIGINAL;
			return;
		}
		handle_instruction(ctx);
		pass_next(table, ctx);
		UNKNOWN_NEXT = DISPATCH_START;
	}
}

// Preserve EAX, ECX and EDX; the frame uses a callee-saved register.
// Pushing sixteen bytes preserves Linux stack alignment at the call.
macro_rules! table_stub {
	($name:ident, $opcode:literal, $frame:literal) => {
		table_stub!($name, $opcode, $frame, instruction_dispatched, "");
	};
	// Invoke `$called` only when the frame's bytecode offset at 0x14 is zero.
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
				// POP preserves the flags set by TEST.
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

// Linux from 515.1644: the opcode in EBX, the frame in ESI
#[cfg(unix)]
table_stub!(table_stub_ebx_esi, "ebx", "esi");
#[cfg(unix)]
table_stub!(first_stub_ebx_esi, "ebx", "esi", first instruction_first);
#[cfg(unix)]
unknown_stub!(unknown_stub_esi, "esi");
// Linux before 515.1644: ECX and EDI
#[cfg(unix)]
table_stub!(table_stub_ecx_edi, "ecx", "edi");
#[cfg(unix)]
table_stub!(first_stub_ecx_edi, "ecx", "edi", first instruction_first);
// Windows: EDX and EDI
#[cfg(windows)]
table_stub!(table_stub_edx_edi, "edx", "edi");
unknown_stub!(unknown_stub_edi, "edi");

/// Make table pages writable without removing execute permission.
/// Jump tables may share pages with code, as in Windows BYOND.
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
	// Preserve execute permission if any overlapping mapping has it. Without
	// the map, or the table's mapping in it, leave the table alone: dropping
	// execute from code would crash.
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

/// Make Windows table pages writable, preserving execute permission.
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
		// Keep data-only pages non-executable.
		if matches!(old, PAGE_READONLY | PAGE_READWRITE | PAGE_WRITECOPY) {
			let mut ignored = 0;
			return VirtualProtect(start as *mut c_void, length, PAGE_READWRITE, &mut ignored) != 0;
		}
		true
	}
}

/// Locate and redirect the opcode table at `execute_instruction`.
/// Match the opcode bounds check, unknown-opcode branch and table jump.
/// On error, leave dispatch unchanged.
unsafe fn take_table(execute_instruction: usize) -> Result<(), &'static str> {
	unsafe {
		let code = std::slice::from_raw_parts(execute_instruction as *const u8, 96);
		let word = |at: usize| u32::from_le_bytes([code[at], code[at + 1], code[at + 2], code[at + 3]]) as usize;
		// Every dispatch starts by reading the frame's offset and bytecode.
		// `FrameStart` and the stubs assume these displacements, which two
		// of the masks leave open.
		if code[3] != 0x14 || code[6] != 0x10 {
			return Err("frame layout differs from the stubs'");
		}
		let mut found = None;
		for at in 0..code.len() - 20 {
			// cmp REGISTER, last opcode
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
			// ja, near or short
			let (unknown, jump) = match (code[at + 6], code[at + 7]) {
				(0x0F, 0x87) => ((execute_instruction + at + 12).wrapping_add(word(at + 8)), at + 12),
				(0x77, short) => ((execute_instruction + at + 8).wrapping_add(short as i8 as usize), at + 8),
				_ => continue
			};
			// jmp [table + REGISTER * 4]
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
		// Reject tables redirected by another library; chaining stubs could
		// repeatedly redispatch the same instruction. BYOND's own entries lie
		// within 0x1A000 of the dispatch on every supported build, and another
		// library's code is megabytes away.
		let entries = entries as *const AtomicUsize;
		const NEAR: usize = 0x0010_0000;
		for opcode in 0..count {
			let entry = (*entries.add(opcode)).load(Ordering::Relaxed);
			if entry.abs_diff(execute_instruction) > NEAR {
				return Err("jump-table entry is outside the interpreter");
			}
			ORIGINAL_TABLE[opcode] = entry;
		}
		if !writable(entries as usize, count * std::mem::size_of::<usize>()) {
			return Err("cannot make jump table writable");
		}

		let hook = RawDetour::new(unknown as *const (), unknown_stub as *const ()).map_err(|_| "cannot create unknown-opcode detour")?;
		DISPATCH_START = execute_instruction;
		UNKNOWN_ORIGINAL = hook.trampoline() as *const () as usize;
		UNKNOWN_NEXT = UNKNOWN_ORIGINAL;
		hook.enable().map_err(|_| "cannot enable unknown-opcode detour")?;
		// Keep the detour and table redirects for the process lifetime.
		std::mem::forget(hook);

		let table = TABLE.get_or_init(|| Table {
			entries,
			count,
			stub: table_stub,
			first_stub: if CALLS_BYPASS_DETOUR { first_stub } else { 0 }
		});
		let _writing = LISTENING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
		ALL.store(true, Ordering::SeqCst);
		for opcode in 0..count {
			(*table.entries.add(opcode)).store(table.stub, Ordering::SeqCst);
		}
		Ok(())
	}
}

/// Enable timestamps at hook entry and exit for `ticks_since_last`.
pub fn time_instructions(timed: bool) {
	unsafe {
		TIMED = timed;
	}
}

/// The processor's timestamp counter
pub fn timestamp() -> u64 {
	unsafe { core::arch::x86::_rdtsc() }
}

/// Elapsed ticks between the previous hook exit and this entry, excluding
/// the handler. Call from the handler while timing is enabled. Returns `None`
/// if nested execution inside the hook invalidated the interval.
pub fn ticks_since_last() -> Option<u64> {
	unsafe { ENTERED.checked_sub(LEFT) }
}

/// Called when auxtools starts
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
	let execute_instruction = crate::find_recipe_of(EXECUTE_INSTRUCTIONS)?;

	// Prefer table dispatch when its layout is recognized.
	if unsafe { take_table(execute_instruction) }.is_ok() {
		unsafe {
			DETOURED = true;
		}
		return Ok(());
	}

	#[cfg(windows)]
	let versioned_hook = execute_instruction_hook as *const ();
	#[cfg(unix)]
	let versioned_hook = if crate::version::get().1 < 1644 {
		execute_instruction_hook_edi as *const ()
	} else {
		execute_instruction_hook as *const ()
	};

	unsafe {
		let hook = RawDetour::new(execute_instruction as *const (), versioned_hook).map_err(|_| "Couldn't detour execute_instruction")?;

		hook.enable().map_err(|_| "Couldn't enable execute_instruction detour")?;

		execute_instruction_original = hook.trampoline() as *const () as *const c_void;

		// The hook remains installed for the process lifetime.
		std::mem::forget(hook);
		DETOURED = true;
	}
	Ok(())
}

// Handles any instruction BYOND tries to execute.
// This function has to leave `*CURRENT_EXECUTION_CONTEXT` in EAX, so make sure
// to return it.
#[no_mangle]
extern "C" fn handle_instruction(ctx: *mut raw_types::procs::ExecutionContext) -> *const raw_types::procs::ExecutionContext {
	unsafe {
		if TIMED {
			ENTERED = timestamp();
		}
		// Dispatch proc hooks and load initialization for builds that bypass
		// the call_proc_by_id detour.
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
