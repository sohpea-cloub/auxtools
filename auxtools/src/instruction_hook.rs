//! Intercept interpreter instructions for debugger callbacks.
//! Proc hooks also require this on builds where interpreter calls bypass
//! call_proc_by_id; see `hooks::calls_bypass_detour`.
//!
//! The interpreter reads an instruction's opcode and jumps through a table
//! with an entry for each. Where that table is found, the handler is reached
//! by pointing entries at a stub (`Table`), which lets a handler with nothing
//! to do for most instructions name the opcodes it wants and leave the rest
//! of the table as BYOND made it: `hear_only`. Those instructions then run as
//! they do without us. An opcode BYOND doesn't have, as a breakpoint's is,
//! takes the interpreter's path for bad instructions, which is detoured.
//! Elsewhere the code that reads the opcode is detoured, and the handler
//! hears every instruction.

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

	// The 514 version of the instruction hook.
	#[cfg(windows)]
	fn execute_instruction_hook_514();

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
pub fn set_handler(handler: Handler) -> Result<(), String> {
	detour()?;
	unsafe {
		HANDLER = Some(handler);
	}
	Ok(())
}

/// The most opcodes a jump table found here can have (516 has 0x18D)
const MOST_OPCODES: usize = 0x200;

/// BYOND's jump table, taken over
struct Table {
	entries: *const AtomicUsize,
	/// Opcodes it has entries for
	count: usize,
	/// What an entry is pointed at for the handler to hear its instruction
	stub: usize
}

// SAFETY: the entries are written through atomics, under `LISTENING`
unsafe impl Send for Table {}
unsafe impl Sync for Table {}

static TABLE: OnceLock<Table> = OnceLock::new();
/// Held while the table's entries are rewritten
static LISTENING: Mutex<()> = Mutex::new(());
/// Whether every entry is the stub
static ALL: AtomicBool = AtomicBool::new(true);

// What the stubs read. BYOND's thread alone writes them once the table is
// taken over.
/// The table's entries as BYOND made them
static mut ORIGINAL_TABLE: [usize; MOST_OPCODES] = [0; MOST_OPCODES];
/// Where the interpreter reads the next instruction
static mut DISPATCH_START: usize = 0;
/// The interpreter's code for an opcode it doesn't have, past our detour
static mut UNKNOWN_ORIGINAL: usize = 0;
/// Where the stub for such an opcode goes on to
static mut UNKNOWN_NEXT: usize = 0;
/// Set for the table's stub to pass one instruction on to BYOND's code: the
/// one the handler has just heard, read again by the interpreter
static mut PASS: u8 = 0;
/// The same for the stub on the path of opcodes BYOND doesn't have
static mut PASS_UNKNOWN: bool = false;

/// Have the handler called before every instruction, as it is from the
/// start. Any thread may ask: a handler told of work by another thread hears
/// the next instruction.
pub fn hear_all() {
	let Some(table) = TABLE.get() else {
		return;
	};
	if ALL.load(Ordering::SeqCst) {
		return;
	}
	let _writing = LISTENING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
	if ALL.swap(true, Ordering::SeqCst) {
		return;
	}
	for opcode in 0..table.count {
		unsafe { (*table.entries.add(opcode)).store(table.stub, Ordering::SeqCst) };
	}
}

/// Have the handler called only before instructions with one of `opcodes` or
/// with an opcode BYOND doesn't have, until `hear_all`. The rest run as they
/// do without a handler. Call on BYOND's thread, and afterwards look again
/// for work another thread may have announced with `hear_all` meanwhile.
///
/// False, with every instruction still heard, where the jump table wasn't
/// found or proc hooks need the instructions (`hooks::calls_bypass_detour`).
pub fn hear_only(opcodes: &[u32]) -> bool {
	let Some(table) = TABLE.get() else {
		return false;
	};
	if unsafe { CALLS_BYPASS_DETOUR } {
		return false;
	}
	let _writing = LISTENING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
	ALL.store(false, Ordering::SeqCst);
	for opcode in 0..table.count {
		let entry = if opcodes.contains(&(opcode as u32)) {
			table.stub
		} else {
			unsafe { ORIGINAL_TABLE[opcode] }
		};
		unsafe { (*table.entries.add(opcode)).store(entry, Ordering::SeqCst) };
	}
	true
}

/// The opcode a frame is about to run
unsafe fn opcode_at(ctx: *mut raw_types::procs::ExecutionContext) -> usize {
	unsafe { *(*(*ctx).bytecode()).add(*(*ctx).bytecode_offset() as usize) as usize }
}

/// After the handler: the interpreter reads the instruction again, which
/// brings it back to a stub if its opcode still leads to one. That stub has
/// to pass it on.
unsafe fn pass_next(table: &Table, ctx: *mut raw_types::procs::ExecutionContext) {
	unsafe {
		let opcode = opcode_at(ctx);
		if opcode >= table.count {
			PASS_UNKNOWN = true;
		} else if (*table.entries.add(opcode)).load(Ordering::SeqCst) == table.stub {
			PASS = 1;
		}
	}
}

/// Called by the table's stub before an instruction it doesn't pass on.
/// Returns 0 if the frame is where it was, about to run what it was: the
/// stub goes on to BYOND's code for the opcode the interpreter read. If the
/// handler changed any of that, 1: the interpreter reads the instruction
/// again.
extern "C" fn instruction_dispatched(ctx: *mut raw_types::procs::ExecutionContext) -> u32 {
	unsafe {
		let at = |ctx: *mut raw_types::procs::ExecutionContext| (*(*ctx).bytecode(), *(*ctx).bytecode_offset(), opcode_at(ctx));
		let before = at(ctx);
		handle_instruction(ctx);
		if at(ctx) == before {
			return 0;
		}
		if let Some(table) = TABLE.get() {
			pass_next(table, ctx);
		}
		1
	}
}

/// Called by the stub on the interpreter's path for an opcode it doesn't
/// have. Sets `UNKNOWN_NEXT`.
extern "C" fn instruction_unknown(ctx: *mut raw_types::procs::ExecutionContext) {
	unsafe {
		let Some(table) = TABLE.get() else {
			UNKNOWN_NEXT = UNKNOWN_ORIGINAL;
			return;
		};
		// Heard already, or code that isn't reading an instruction came here
		if std::mem::take(&mut PASS_UNKNOWN) || opcode_at(ctx) < table.count {
			UNKNOWN_NEXT = UNKNOWN_ORIGINAL;
			return;
		}
		handle_instruction(ctx);
		pass_next(table, ctx);
		UNKNOWN_NEXT = DISPATCH_START;
	}
}

// The stubs. Each keeps EAX, ECX and EDX for the code it goes on to; the
// frame's register is one a function keeps. On Linux the sixteen bytes
// pushed keep ESP aligned at the call.
macro_rules! table_stub {
	($name:ident, $opcode:literal, $frame:literal) => {
		#[unsafe(naked)]
		unsafe extern "C" fn $name() {
			core::arch::naked_asm!(
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
				// (a pop leaves the flags)
				"test eax, eax",
				"pop eax",
				"jz 3f",
				"jmp dword ptr [{start}]",
				"2:",
				"mov byte ptr [{pass}], 0",
				"3:",
				concat!("jmp dword ptr [{original} + ", $opcode, "*4]"),
				pass = sym PASS,
				dispatched = sym instruction_dispatched,
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
unknown_stub!(unknown_stub_esi, "esi");
// Linux before 515.1644: ECX and EDI
#[cfg(unix)]
table_stub!(table_stub_ecx_edi, "ecx", "edi");
// Windows: EDX and EDI
#[cfg(windows)]
table_stub!(table_stub_edx_edi, "edx", "edi");
unknown_stub!(unknown_stub_edi, "edi");

/// Make the table's pages writable, keeping what else they allow: a
/// compiler can put a jump table among the code that uses it, as the one
/// BYOND for Windows is built with does.
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
	// Whether any mapping the table is in can be executed
	let executable = std::fs::read_to_string("/proc/self/maps").is_ok_and(|maps| {
		maps.lines().any(|line| {
			let mut fields = line.split_ascii_whitespace();
			let (Some(range), Some(access)) = (fields.next(), fields.next()) else {
				return false;
			};
			let Some((low, high)) = range.split_once('-') else {
				return false;
			};
			let (Ok(low), Ok(high)) = (usize::from_str_radix(low, 16), usize::from_str_radix(high, 16)) else {
				return false;
			};
			low < to && from < high && access.contains('x')
		})
	});
	let protection = if executable { READ_WRITE | EXECUTE } else { READ_WRITE };
	unsafe { mprotect(from as *mut c_void, to - from, protection) == 0 }
}

/// The same for Windows
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
		// Pages that held no code don't need to hold any now
		if matches!(old, PAGE_READONLY | PAGE_READWRITE | PAGE_WRITECOPY) {
			let mut ignored = 0;
			return VirtualProtect(start as *mut c_void, length, PAGE_READWRITE, &mut ignored) != 0;
		}
		true
	}
}

/// Take over the jump table that the code at `execute_instruction` jumps
/// through: it compares the opcode with the last one there is, jumps away
/// if it is above, and jumps through the table. `Err` leaves everything as
/// it was.
unsafe fn take_table(execute_instruction: usize) -> Result<(), &'static str> {
	unsafe {
		let code = std::slice::from_raw_parts(execute_instruction as *const u8, 96);
		let word = |at: usize| u32::from_le_bytes([code[at], code[at + 1], code[at + 2], code[at + 3]]) as usize;
		let mut found = None;
		for at in 0..code.len() - 20 {
			// cmp REGISTER, last opcode
			let (index, table_stub, unknown_stub): (u8, usize, usize) = match (code[at], code[at + 1]) {
				#[cfg(unix)]
				(0x81, 0xFB) => (0x9D, table_stub_ebx_esi as *const () as usize, unknown_stub_esi as *const () as usize),
				#[cfg(unix)]
				(0x81, 0xF9) => (0x8D, table_stub_ecx_edi as *const () as usize, unknown_stub_edi as *const () as usize),
				#[cfg(windows)]
				(0x81, 0xFA) => (0x95, table_stub_edx_edi as *const () as usize, unknown_stub_edi as *const () as usize),
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
			found = Some((word(jump + 3), count, unknown, table_stub, unknown_stub));
			break;
		}
		let (entries, count, unknown, table_stub, unknown_stub) = found.ok_or("no jump through a table there")?;
		if count > MOST_OPCODES {
			return Err("more opcodes than expected");
		}
		// Each entry is code of the interpreter's. One that isn't is another
		// library's stub, and two libraries' stubs would hand an instruction
		// back and forth.
		let entries = entries as *const AtomicUsize;
		const NEAR: usize = 0x0100_0000;
		for opcode in 0..count {
			let entry = (*entries.add(opcode)).load(Ordering::Relaxed);
			if entry.abs_diff(execute_instruction) > NEAR {
				return Err("an entry of the table is outside the interpreter");
			}
			ORIGINAL_TABLE[opcode] = entry;
		}
		if !writable(entries as usize, count * std::mem::size_of::<usize>()) {
			return Err("the table can't be written");
		}

		let hook = RawDetour::new(unknown as *const (), unknown_stub as *const ()).map_err(|_| "couldn't detour the path for unknown opcodes")?;
		DISPATCH_START = execute_instruction;
		UNKNOWN_ORIGINAL = hook.trampoline() as *const () as usize;
		UNKNOWN_NEXT = UNKNOWN_ORIGINAL;
		hook.enable().map_err(|_| "couldn't enable the detour on the path for unknown opcodes")?;
		// It stays for the life of the process, as the table's entries do
		std::mem::forget(hook);

		let table = TABLE.get_or_init(|| Table {
			entries,
			count,
			stub: table_stub
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

	// The jump table where there is one we know how to take over
	if unsafe { take_table(execute_instruction) }.is_ok() {
		unsafe {
			DETOURED = true;
		}
		return Ok(());
	}

	#[cfg(windows)]
	let versioned_hook = if crate::version::get().0 == 514 {
		execute_instruction_hook_514 as *const ()
	} else {
		execute_instruction_hook as *const ()
	};
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
		if CALLS_BYPASS_DETOUR && *(*ctx).bytecode_offset() == 0 {
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
