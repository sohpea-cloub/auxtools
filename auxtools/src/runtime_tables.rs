//! BYOND's runtime tables that hot reloading adds to: the proc table, misc
//! entries (bytecode, locals and parameter tables, a type's proc and var
//! lists), the type table and the per-type proc name maps.
//!
//! Everything here goes through BYOND's own functions where it has them, so
//! what it allocates is BYOND's and BYOND frees it like its own. Found on
//! Windows only for now; on Linux every function here fails with an error that
//! says so.
//!
//! Layouts are from 516.1688 `byondcore.dll` (see the hot reload plan in the
//! auxtools-debug workspace for where each was read), and the patterns match
//! 1659, 1685, 1687 and 1688.

use std::ffi::c_void;

use crate::raw_types::{misc, procs::ProcId};

/// A proc table entry as BYOND stores it, word by word: path, name, desc and
/// category string ids, the path's last part, flags, three packed bytes,
/// bytecode, locals and parameters misc ids, and the proc `..()` goes to.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcEntryWords(pub [u32; 11]);

impl ProcEntryWords {
	pub const BYTECODE: usize = 7;
	pub const CATEGORY: usize = 3;
	pub const DESC: usize = 2;
	pub const FLAGS: usize = 5;
	pub const LOCALS: usize = 8;
	pub const NAME: usize = 1;
	pub const NEXT_OVERRIDE: usize = 10;
	pub const PARAMETERS: usize = 9;
	pub const PATH: usize = 0;
	pub const PATH_BASENAME: usize = 4;
}

/// `ProcEntry::flags`: `set waitfor` (on unless set to 0) and `set background`.
pub const PROC_FLAG_WAITFOR: u32 = 0x04;
pub const PROC_FLAG_BACKGROUND: u32 = 0x100;

/// No proc, string or misc entry.
pub const NONE: u32 = 0xFFFF;

/// One type's proc names, sorted by key.
#[repr(C)]
pub struct ProcNameMap {
	pub buckets: *mut ProcNameBucket,
	pub count: u16,
	pub built: u8
}

/// The procs a name finds on a type, most derived first. `ids` is a shared
/// buffer (`AllocShared`), and the same buffer can be in several types' maps.
#[repr(C)]
pub struct ProcNameBucket {
	/// The name's string id
	pub key: u32,
	pub ids: *mut u32,
	pub count: u16,
	/// Set for keys that are a proc's path basename rather than its name. Such
	/// a bucket builds no `..()` chain.
	pub no_link: u8
}

/// Byte offsets into a type table entry. The same on Windows and Linux.
pub mod type_fields {
	pub const PATH: usize = 0x00;
	pub const PARENT: usize = 0x04;
	/// Misc id: the type's own verbs
	pub const VERBS: usize = 0x60;
	/// Misc id: the type's own procs
	pub const PROCS: usize = 0x64;
	/// Misc id: (var id, flags) pairs for the vars the type declares
	pub const DECLARED_VARS: usize = 0x70;
	/// Misc id: every var the type has, (name string id, var id, tag, data)
	/// per var, sorted by name. Instances read their defaults from here.
	pub const VAR_TABLE: usize = 0x78;
}

#[cfg(windows)]
const TYPE_STRIDE: usize = 0x98;
#[cfg(unix)]
const TYPE_STRIDE: usize = 0x94;

struct Functions {
	procdef_add: extern "C" fn(*const ProcEntryWords) -> u32,
	misc_intern_copy: extern "C" fn(*const u32, u32) -> u32,
	alloc_shared: extern "C" fn(usize) -> *mut c_void,
	free_shared: extern "C" fn(*mut c_void) -> bool,
	crt_realloc: extern "C" fn(*mut c_void, usize) -> *mut c_void,
	type_count: *const u32,
	type_table: *const *mut u8,
	name_maps: *const *mut ProcNameMap,
	/// One Value per var id: a global var's value, and the default of a
	/// type's declared var. A new frame's locals start as their var ids'
	/// values.
	global_values: *const *const crate::raw_types::values::Value
}

static mut FUNCTIONS: Option<Result<Functions, String>> = None;

/// Forgets what was looked up, for the next world.
pub(crate) fn reset() {
	unsafe {
		FUNCTIONS = None;
	}
}

#[cfg(windows)]
mod recipes {
	use byond_scan::{Anchor, Extract, OperandSelect, Recipe, SignatureTreatment, VersionRange};

	const CHECKED: VersionRange = VersionRange { min: 1659, max: 1688 };

	// Appends an entry to the proc table, or finds an identical one
	pub const PROCDEF_ADD: Recipe = Recipe {
		name: "procdef_add",
		versions: CHECKED,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC A1 ?? ?? ?? ?? 53 8B 5D 08 56 33 F6 57 85 C0 74 ?? 33 FF A1 ?? ?? ?? ?? 03 C7 53 50 E8"
		),
		hops: &[],
		extract: Extract::Entry
	};

	// Copies words into a new misc entry, or finds an identical one
	pub const MISC_INTERN_COPY: Recipe = Recipe {
		name: "misc_intern_copy",
		versions: CHECKED,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"55 8B EC 56 8B 35 ?? ?? ?? ?? 57 85 F6 74 0A 8B 46 08 A3 ?? ?? ?? ?? EB 0D 6A 0C FF 15 ?? ?? ?? ?? 83 C4 04 8B F0 85 F6 74 ?? 66 8B 4D \
			 0C"
		),
		hops: &[],
		extract: Extract::Entry
	};

	// The proc name map of a type: the type count is operand 0, the maps 1. The
	// type table is the word before the count.
	const TYPE_MAP_ACCESSOR: &str = "55 8B EC 8B 4D 08 3B 0D ?? ?? ?? ?? 77 0A A1 ?? ?? ?? ?? 8D 04 C8 5D C3 33 C0 5D C3";

	pub const TYPE_COUNT: Recipe = Recipe {
		name: "obj_type_count",
		versions: CHECKED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_MAP_ACCESSOR),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(0))
	};

	pub const NAME_MAPS: Recipe = Recipe {
		name: "proc_name_maps",
		versions: CHECKED,
		anchor: Anchor::Signature(SignatureTreatment::NoOffset, TYPE_MAP_ACCESSOR),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	};

	// The loader of the var tables, as auxtools' `variable_names` finds it.
	// global_values is operand 1.
	pub const GLOBAL_VALUES: Recipe = Recipe {
		name: "global_values",
		versions: CHECKED,
		anchor: Anchor::Signature(
			SignatureTreatment::NoOffset,
			"A3 ?? ?? ?? ?? 56 FF D3 83 C4 04 85 F6 74 08 85 C0 0F 84 4B 01 00 00 A3 ?? ?? ?? ?? A1 ?? ?? ?? ?? 8D 34 85 00 00 00 00 56 FF D3 83 C4 \
			 04 85 F6 74 08 85 C0 0F 84 2D 01 00 00 33 DB A3 ?? ?? ?? ?? 39 1D"
		),
		hops: &[],
		extract: Extract::AbsMem(OperandSelect::NthAbsMem(1))
	};

	pub const ALLOC_SHARED: Recipe = Recipe {
		name: "AllocShared",
		versions: CHECKED,
		anchor: Anchor::Export(c"?AllocShared@@YAPAXK@Z"),
		hops: &[],
		extract: Extract::Entry
	};

	pub const FREE_SHARED: Recipe = Recipe {
		name: "FreeShared",
		versions: CHECKED,
		anchor: Anchor::Export(c"?FreeShared@@YA_NPAX@Z"),
		hops: &[],
		extract: Extract::Entry
	};
}

#[cfg(windows)]
fn resolve() -> Result<Functions, String> {
	use recipes::*;
	let find = |recipe| crate::find_recipe(recipe);

	// BYOND's tables are allocated by the CRT DLL it links (ucrtbase), and
	// growing one has to go through the same heap. Ours is linked statically.
	let crt_realloc = unsafe {
		use winapi::um::libloaderapi::{GetModuleHandleA, GetProcAddress};
		let module = GetModuleHandleA(c"ucrtbase.dll".as_ptr());
		if module.is_null() {
			return Err("Couldn't find ucrtbase.dll".to_owned());
		}
		let realloc = GetProcAddress(module, c"realloc".as_ptr());
		if realloc.is_null() {
			return Err("Couldn't find realloc in ucrtbase.dll".to_owned());
		}
		realloc
	};

	let type_count = find(&TYPE_COUNT)?;
	unsafe {
		Ok(Functions {
			procdef_add: std::mem::transmute(find(&PROCDEF_ADD)?),
			misc_intern_copy: std::mem::transmute(find(&MISC_INTERN_COPY)?),
			alloc_shared: std::mem::transmute(find(&ALLOC_SHARED)?),
			free_shared: std::mem::transmute(find(&FREE_SHARED)?),
			crt_realloc: std::mem::transmute(crt_realloc),
			type_count: type_count as *const u32,
			type_table: (type_count - 4) as *const *mut u8,
			name_maps: find(&NAME_MAPS)? as *const *mut ProcNameMap,
			global_values: find(&GLOBAL_VALUES)? as *const *const crate::raw_types::values::Value
		})
	}
}

#[cfg(unix)]
fn resolve() -> Result<Functions, String> {
	Err("this isn't available on Linux yet".to_owned())
}

fn functions() -> Result<&'static Functions, String> {
	unsafe {
		let functions = &mut *std::ptr::addr_of_mut!(FUNCTIONS);
		functions.get_or_insert_with(resolve).as_ref().map_err(Clone::clone)
	}
}

/// Whether this build of BYOND has everything here.
pub fn available() -> Result<(), String> {
	functions().map(|_| ())
}

/// A proc's table entry.
pub fn proc_entry(id: ProcId) -> Option<ProcEntryWords> {
	let mut entry: *mut crate::raw_types::procs::ProcEntry = std::ptr::null_mut();
	unsafe {
		if crate::raw_types::funcs::get_proc_array_entry(&mut entry, id) != 1 || entry.is_null() {
			return None;
		}
		Some(*(entry as *const ProcEntryWords))
	}
}

/// Overwrites one word of a proc's table entry.
pub fn set_proc_entry_word(id: ProcId, word: usize, value: u32) {
	let mut entry: *mut crate::raw_types::procs::ProcEntry = std::ptr::null_mut();
	unsafe {
		if crate::raw_types::funcs::get_proc_array_entry(&mut entry, id) == 1 && !entry.is_null() {
			(*(entry as *mut ProcEntryWords)).0[word] = value;
		}
	}
}

/// Appends a proc to the proc table and returns its id. BYOND takes
/// references to its strings; the misc entries need theirs taken already.
pub fn add_proc(entry: &ProcEntryWords) -> Result<ProcId, String> {
	let id = (functions()?.procdef_add)(entry);
	if id == NONE {
		return Err("BYOND couldn't add the proc".to_owned());
	}
	Ok(ProcId(id))
}

/// How many procs there are. The next one added gets this id.
pub fn proc_count() -> u32 {
	let mut id = 0;
	while proc_entry(ProcId(id)).is_some() {
		id += 1;
	}
	id
}

/// A misc entry's words.
pub fn misc_words(id: u32) -> Option<Vec<u32>> {
	if id == NONE {
		return Some(vec![]);
	}
	let mut entry: *mut c_void = std::ptr::null_mut();
	unsafe {
		if crate::raw_types::funcs::get_misc_by_id(&mut entry, misc::MiscId::from_raw(id)) != 1 || entry.is_null() {
			return None;
		}
		// count (u16), reference count, data
		let count = *(entry as *const u16) as usize;
		let data = *((entry as *const u8).add(8) as *const *const u32);
		if count == 0 || data.is_null() {
			return Some(vec![]);
		}
		Some(std::slice::from_raw_parts(data, count).to_vec())
	}
}

/// A misc entry holding `words`, with a reference taken so it is never freed.
/// BYOND shares entries with the same words.
pub fn intern_misc(words: &[u32]) -> Result<u32, String> {
	if words.len() > u16::MAX as usize {
		return Err(format!("{} words is too many for a misc entry", words.len()));
	}
	let id = (functions()?.misc_intern_copy)(words.as_ptr(), words.len() as u32);
	if id == NONE {
		return Err("BYOND couldn't make the misc entry".to_owned());
	}
	misc::add_ref(&misc::MiscId::from_raw(id));
	Ok(id)
}

pub fn type_count() -> Result<u32, String> {
	Ok(unsafe { *functions()?.type_count })
}

fn type_entry(type_id: u32) -> Result<*mut u8, String> {
	let functions = functions()?;
	unsafe {
		if type_id >= *functions.type_count {
			return Err(format!("there's no type {}", type_id));
		}
		Ok((*functions.type_table).add(type_id as usize * TYPE_STRIDE))
	}
}

/// A word of a type's table entry (see `type_fields`).
pub fn type_field(type_id: u32, offset: usize) -> Result<u32, String> {
	Ok(unsafe { *(type_entry(type_id)?.add(offset) as *const u32) })
}

pub fn set_type_field(type_id: u32, offset: usize, value: u32) -> Result<(), String> {
	unsafe {
		*(type_entry(type_id)?.add(offset) as *mut u32) = value;
	}
	Ok(())
}

/// A type's proc name map.
fn name_map(type_id: u32) -> Result<*mut ProcNameMap, String> {
	let functions = functions()?;
	unsafe {
		if type_id >= *functions.type_count {
			return Err(format!("there's no type {}", type_id));
		}
		Ok((*functions.name_maps).add(type_id as usize))
	}
}

/// The procs a name (its string id) finds on a type, most derived first.
pub fn named_procs(type_id: u32, key: u32) -> Result<Vec<u32>, String> {
	let map = name_map(type_id)?;
	unsafe {
		let buckets = std::slice::from_raw_parts((*map).buckets, (*map).count as usize);
		Ok(match buckets.binary_search_by_key(&key, |bucket| bucket.key) {
			Ok(i) if !buckets[i].ids.is_null() => std::slice::from_raw_parts(buckets[i].ids, buckets[i].count as usize).to_vec(),
			_ => vec![]
		})
	}
}

/// Gives a name these procs on a type, adding the name if the type's map
/// doesn't have it. The old buffer is let go, and the new one is the map's own.
pub fn set_named_procs(type_id: u32, key: u32, procs: &[u32]) -> Result<(), String> {
	let functions = functions()?;
	let map = name_map(type_id)?;
	unsafe {
		let count = (*map).count as usize;
		let at = match std::slice::from_raw_parts((*map).buckets, count).binary_search_by_key(&key, |bucket| bucket.key) {
			Ok(at) => at,
			Err(at) => {
				let size = std::mem::size_of::<ProcNameBucket>();
				let buckets = (functions.crt_realloc)((*map).buckets as *mut c_void, (count + 1) * size) as *mut ProcNameBucket;
				if buckets.is_null() {
					return Err("out of memory growing a proc name map".to_owned());
				}
				std::ptr::copy(buckets.add(at), buckets.add(at + 1), count - at);
				buckets.add(at).write(ProcNameBucket {
					key,
					ids: std::ptr::null_mut(),
					count: 0,
					no_link: 0
				});
				(*map).buckets = buckets;
				(*map).count += 1;
				at
			}
		};

		let ids = (functions.alloc_shared)(procs.len().max(1) * 4) as *mut u32;
		if ids.is_null() {
			return Err("out of memory for a proc name map entry".to_owned());
		}
		std::ptr::copy_nonoverlapping(procs.as_ptr(), ids, procs.len());

		let bucket = &mut *(*map).buckets.add(at);
		if !bucket.ids.is_null() {
			(functions.free_shared)(bucket.ids as *mut c_void);
		}
		bucket.ids = ids;
		bucket.count = procs.len() as u16;
	}
	Ok(())
}

/// The value of a var id's global slot (see `Functions::global_values`).
pub fn global_value(var_id: u32) -> Result<crate::raw_types::values::Value, String> {
	let functions = functions()?;
	unsafe {
		let table = &*crate::raw_types::funcs::VARIABLE_NAMES;
		if var_id >= table.count {
			return Err(format!("there's no var id {}", var_id));
		}
		Ok(*(*functions.global_values).add(var_id as usize))
	}
}

/// How many var ids there are.
pub fn variable_count() -> u32 {
	unsafe { (*crate::raw_types::funcs::VARIABLE_NAMES).count }
}

/// The string id of a var id's name.
pub fn variable_name(var_id: u32) -> Option<u32> {
	unsafe {
		let table = &*crate::raw_types::funcs::VARIABLE_NAMES;
		(var_id < table.count).then(|| (*table.entries.add(var_id as usize)).0)
	}
}

/// Appends a var id named `name` (a string id) whose global slot holds the
/// value `tag`/`data`. The slot keeps a reference to it, which the caller
/// hands over. Ids are only ever appended, so existing ones stay as they are.
pub fn add_variable(name: u32, tag: u8, data: u32) -> Result<u32, String> {
	let functions = functions()?;
	unsafe {
		let table = crate::raw_types::funcs::VARIABLE_NAMES as *mut crate::raw_types::variables::VariableNameIdTable;
		let count = (*table).count as usize;
		if count as u32 >= NONE {
			return Err("the world has as many vars as BYOND can number".to_owned());
		}

		let names = (functions.crt_realloc)((*table).entries as *mut c_void, (count + 1) * 4) as *mut u32;
		if names.is_null() {
			return Err("out of memory growing the var names".to_owned());
		}
		(*table).entries = names as *const crate::raw_types::strings::StringId;

		let slots = functions.global_values as *mut *mut [u32; 2];
		let values = (functions.crt_realloc)(*slots as *mut c_void, (count + 1) * 8) as *mut [u32; 2];
		if values.is_null() {
			return Err("out of memory growing the var values".to_owned());
		}
		*slots = values;

		names.add(count).write(name);
		values.add(count).write([tag as u32, data]);
		(*table).count += 1;
		Ok(count as u32)
	}
}
