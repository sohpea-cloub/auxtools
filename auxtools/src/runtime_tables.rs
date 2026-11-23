//! Extends BYOND's tables with BYOND's own allocators, so BYOND can free the
//! memory. Every function here reads or writes BYOND's memory, so each is
//! `unsafe`.

use std::ffi::c_void;

use crate::{
	raw_types::{misc, procs::ProcId},
	registry::{builds, find}
};

/// A proc entry's words: path, name, desc, category, basename, flags, packed
/// bytes, bytecode, locals, parameters, and the proc that `..()` calls.
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
	/// A verb's `set src` range and kind, and its invisibility. A byte each.
	pub const SETTINGS: usize = 6;
}

/// No proc, string or misc entry.
pub const NONE: u32 = 0xFFFF;

#[repr(C)]
pub struct ProcNameMap {
	pub buckets: *mut ProcNameBucket,
	pub count: u16,
	pub built: u8
}

/// Most derived first. Types' maps share `ids` through `AllocShared`.
#[repr(C)]
pub struct ProcNameBucket {
	pub key: u32,
	pub ids: *mut u32,
	pub count: u16,
	/// Keyed by path basename, not by name. Builds no `..()` chain.
	pub no_link: u8
}

pub mod type_fields {
	pub const PATH: usize = 0x00;
	pub const PARENT: usize = 0x04;
	/// String ids, or the resource id for the icon.
	pub const NAME: usize = 0x08;
	pub const DESC: usize = 0x0C;
	pub const ICON: usize = 0x10;
	pub const ICON_STATE: usize = 0x14;
	pub const TEXT: usize = 0x18;
	pub const MAPTEXT: usize = 0x1C;
	/// 16 bits each.
	pub const MAPTEXT_X: usize = 0x20;
	pub const MAPTEXT_Y: usize = 0x22;
	pub const MAPTEXT_WIDTH: usize = 0x24;
	pub const MAPTEXT_HEIGHT: usize = 0x26;
	/// Bit 1 is opacity, 2 density, 6-7 gender, 8 mouse_drop_zone, 12-13
	/// mouse_opacity, and 10 and 14-15 animate_movement. Bits 21 and up are the
	/// low bits of appearance_flags.
	pub const FLAGS: usize = 0x28;
	/// The rest of `appearance_flags`, from bit 0.
	pub const FLAGS_HIGH: usize = 0x2C;
	/// A byte.
	pub const DIR: usize = 0x32;
	/// Pointers to the type's transform of 6 floats and its color matrix of 20.
	pub const TRANSFORM: usize = 0x34;
	pub const COLOR_MATRIX: usize = 0x38;
	pub const SUFFIX: usize = 0x54;
	/// A float.
	pub const LAYER: usize = 0x5C;
	/// Misc id: the type's own verbs.
	pub const VERBS: usize = 0x60;
	/// Misc id: the type's own procs.
	pub const PROCS: usize = 0x64;
	/// Computed var initializer, merged from the ancestors' at startup.
	pub const INIT_PROC: usize = 0x68;
	/// Misc id: (var id, flags) pairs for the vars the type declares.
	pub const DECLARED_VARS: usize = 0x70;
	/// Misc id: (var id, tag, data) as the .dmb sets them, read at startup
	/// only.
	pub const OWN_OVERRIDES: usize = 0x6C;
	/// Misc id: the built-in vars set outside the record, as name and value.
	pub const BUILTIN_VALUES: usize = 0x74;
	/// Misc id: (name, var id, tag, data) for each var by name. Read for
	/// defaults.
	pub const VAR_TABLE: usize = 0x78;
	pub const EXTRA: usize = 0x7C;
	/// Var initializer bytecode ranges, used when merging init procs.
	pub const INIT_SEGMENTS: usize = 0x80;
	/// The type's place in the preorder, and its subtree's end.
	pub const PREORDER: usize = 0x84;
	pub const PREORDER_END: usize = 0x88;
	/// (tag, data) from 515.1641. Earlier entries end after PREORDER_END.
	pub const TYPEPATH: usize = 0x8C;
}

const PROC_VALUE: u32 = 0x26;

unsafe fn path_cache(build: u32, register: Option<usize>) -> Option<*const *mut u8> {
	if !(builds::TYPEPATHS..=builds::NEWEST_CHECKED).contains(&build) {
		return None;
	}
	let code = std::slice::from_raw_parts(register? as *const u8, 0x100);
	let push = (0..code.len() - 10).find(|&at| code[at] == 0x68 && code[at + 5] == 0xE8)?;
	let address = u32::from_le_bytes([code[push + 1], code[push + 2], code[push + 3], code[push + 4]]);
	Some(address as usize as *const *mut u8)
}

/// BYOND doesn't refresh cached misses itself. A node is a text key with tag
/// 6, then value, left, right, parent and colour.
///
/// # Safety
/// Call on BYOND's thread while a world is live. `value` must be the path's tag
/// and data.
pub unsafe fn recache_path(path: u32, value: [u32; 2]) {
	const TEXT: u8 = 0x06;
	const VALUE: usize = 0x08;
	const BEFORE: usize = 0x10;
	const AFTER: usize = 0x14;
	let Some(cache) = functions().ok().and_then(|functions| functions.path_cache) else {
		return;
	};
	unsafe {
		let mut node = *cache;
		while !node.is_null() {
			let key = (*node, *(node.add(4) as *const u32));
			node = match (TEXT, path).cmp(&key) {
				std::cmp::Ordering::Less => *(node.add(BEFORE) as *const *mut u8),
				std::cmp::Ordering::Greater => *(node.add(AFTER) as *const *mut u8),
				std::cmp::Ordering::Equal => {
					if *node.add(VALUE) == 0 {
						*(node.add(VALUE) as *mut [u32; 2]) = value;
					}
					return;
				}
			};
		}
	}
}

fn type_stride(build: u32) -> usize {
	if build < builds::TYPEPATHS {
		if cfg!(windows) {
			0x90
		} else {
			0x8C
		}
	} else if cfg!(windows) {
		0x98
	} else {
		0x94
	}
}

struct Functions {
	procdef_add: extern "C" fn(*const ProcEntryWords) -> u32,
	misc_intern: MiscIntern,
	alloc_shared: extern "C" fn(usize) -> *mut c_void,
	free_shared: extern "C" fn(*mut c_void) -> bool,
	copy_shared: extern "C" fn(*mut c_void) -> *mut c_void,
	register_typepath: Result<Option<extern "C" fn(u32)>, String>,
	path_cache: Option<*const *mut u8>,
	type_stride: usize,
	crt_realloc: extern "C" fn(*mut c_void, usize) -> *mut c_void,
	crt_free: extern "C" fn(*mut c_void),
	type_count: *const u32,
	type_table: *const *mut u8,
	name_maps: *const *mut ProcNameMap,
	preorder: *const *mut u32,
	appearance_cache: *const *mut u32,
	in_list_count: *const u32,
	in_list_procs: *const *mut u32,
	instance_count: *const u32,
	instances: *const *mut Instance,
	mob_count: *const u32,
	mobs: *const *mut MobType,
	world_name_map: *mut ProcNameMap,

	global_values: *const *const crate::raw_types::values::Value,
	global_vars: Result<*mut u32, String>
}

#[derive(Clone, Copy)]
enum MiscIntern {
	Copy(extern "C" fn(*const u32, u32) -> u32),
	Take(extern "C" fn(*mut u8) -> u32)
}

static mut FUNCTIONS: Option<Result<Functions, String>> = None;

pub(crate) fn reset() {
	unsafe {
		FUNCTIONS = None;
	}
}

#[cfg(windows)]
fn ucrtbase(name: &std::ffi::CStr) -> Result<usize, String> {
	unsafe {
		use winapi::um::libloaderapi::{GetModuleHandleA, GetProcAddress};
		let module = GetModuleHandleA(c"ucrtbase.dll".as_ptr());
		if module.is_null() {
			return Err("Couldn't find ucrtbase.dll".to_owned());
		}
		let function = GetProcAddress(module, name.as_ptr());
		if function.is_null() {
			return Err(format!("Couldn't find {} in ucrtbase.dll", name.to_string_lossy()));
		}
		Ok(function as usize)
	}
}

#[cfg(windows)]
fn crt_realloc() -> Result<extern "C" fn(*mut c_void, usize) -> *mut c_void, String> {
	Ok(unsafe { std::mem::transmute::<usize, _>(ucrtbase(c"realloc")?) })
}

#[cfg(windows)]
fn crt_free() -> Result<extern "C" fn(*mut c_void), String> {
	Ok(unsafe { std::mem::transmute::<usize, _>(ucrtbase(c"free")?) })
}

#[cfg(unix)]
fn crt_realloc() -> Result<extern "C" fn(*mut c_void, usize) -> *mut c_void, String> {
	extern "C" {
		fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
	}
	Ok(unsafe { std::mem::transmute::<unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void, _>(realloc) })
}

#[cfg(unix)]
fn crt_free() -> Result<extern "C" fn(*mut c_void), String> {
	extern "C" {
		fn free(ptr: *mut c_void);
	}
	Ok(unsafe { std::mem::transmute::<unsafe extern "C" fn(*mut c_void), _>(free) })
}

#[cfg(windows)]
unsafe fn misc_intern(_build: u32) -> Result<MiscIntern, String> {
	Ok(MiscIntern::Copy(std::mem::transmute(find!("misc_intern_copy")?)))
}

#[cfg(unix)]
unsafe fn misc_intern(build: u32) -> Result<MiscIntern, String> {
	Ok(if build < builds::LINUX_FRAMELESS {
		MiscIntern::Take(std::mem::transmute(find!("misc_intern")?))
	} else {
		MiscIntern::Copy(std::mem::transmute(find!("misc_intern_copy")?))
	})
}

fn resolve() -> Result<Functions, String> {
	let build = crate::byond_build()?;
	let type_count = find!("obj_type_count")?;
	#[cfg(windows)]
	let type_table = type_count - 4;
	#[cfg(unix)]
	let type_table = if build < builds::LINUX_FRAMELESS {
		type_count - 4
	} else {
		type_count + 4
	};
	let register_typepath = if build < builds::TYPEPATHS {
		Ok(None)
	} else {
		find!("register_typepath").map(Some)
	};
	unsafe {
		Ok(Functions {
			procdef_add: std::mem::transmute(find!("procdef_add")?),
			misc_intern: misc_intern(build)?,
			alloc_shared: std::mem::transmute(find!("AllocShared")?),
			free_shared: std::mem::transmute(find!("FreeShared")?),
			copy_shared: std::mem::transmute(find!("CopyShared")?),
			path_cache: path_cache(build, register_typepath.clone().ok().flatten()),
			register_typepath: register_typepath.map(|address| address.map(|address| std::mem::transmute::<usize, extern "C" fn(u32)>(address))),
			type_stride: type_stride(build),
			crt_realloc: crt_realloc()?,
			crt_free: crt_free()?,
			type_count: type_count as *const u32,
			type_table: type_table as *const *mut u8,
			name_maps: find!("proc_name_maps")? as *const *mut ProcNameMap,
			preorder: find!("type_preorder")? as *const *mut u32,
			appearance_cache: find!("type_appearance_cache")? as *const *mut u32,
			in_list_count: find!("in_list_count")? as *const u32,
			in_list_procs: find!("in_list_procs")? as *const *mut u32,
			instance_count: find!("instance_count")? as *const u32,
			instances: find!("instances")? as *const *mut Instance,
			mob_count: find!("mob_count")? as *const u32,
			mobs: find!("mobs")? as *const *mut MobType,
			world_name_map: (find!("world_name_map")? as *const *mut ProcNameMap).read_unaligned(),
			global_values: find!("global_values")? as *const *const crate::raw_types::values::Value,
			global_vars: find!("global_vars").map(|address| address as *mut u32)
		})
	}
}

unsafe fn functions() -> Result<&'static Functions, String> {
	if (*std::ptr::addr_of!(FUNCTIONS)).is_none() {
		*std::ptr::addr_of_mut!(FUNCTIONS) = Some(resolve());
	}
	(*std::ptr::addr_of!(FUNCTIONS)).as_ref().unwrap().as_ref().map_err(Clone::clone)
}

/// Whether the tables can be extended. [`types_available`] checks adding
/// types.
///
/// # Safety
/// Call on BYOND's thread while a world is live.
pub unsafe fn available() -> Result<(), String> {
	functions().map(|_| ())
}

pub unsafe fn types_available() -> Result<(), String> {
	functions()?.register_typepath.as_ref().map(|_| ()).map_err(Clone::clone)
}

pub unsafe fn global_vars_available() -> Result<(), String> {
	functions()?.global_vars.as_ref().map(|_| ()).map_err(Clone::clone)
}

pub unsafe fn proc_entry(id: ProcId) -> Option<ProcEntryWords> {
	let mut entry: *mut crate::raw_types::procs::ProcEntry = std::ptr::null_mut();
	unsafe {
		if crate::raw_types::funcs::get_proc_array_entry(&mut entry, id) != 1 || entry.is_null() {
			return None;
		}
		Some(*(entry as *const ProcEntryWords))
	}
}

pub unsafe fn set_proc_entry_word(id: ProcId, word: usize, value: u32) {
	let mut entry: *mut crate::raw_types::procs::ProcEntry = std::ptr::null_mut();
	unsafe {
		if crate::raw_types::funcs::get_proc_array_entry(&mut entry, id) == 1 && !entry.is_null() {
			(*(entry as *mut ProcEntryWords)).0[word] = value;
		}
	}
}

pub unsafe fn set_proc_entry(id: ProcId, words: &ProcEntryWords) {
	for (word, &value) in words.0.iter().enumerate() {
		set_proc_entry_word(id, word, value);
	}
}

/// BYOND takes references to the entry's strings. The caller must take
/// references to its misc entries.
///
/// # Safety
/// Call on BYOND's thread while a world is live. The entry's IDs must be valid,
/// as for [`set_proc_entry_word`].
pub unsafe fn add_proc(entry: &ProcEntryWords) -> Result<ProcId, String> {
	let id = (functions()?.procdef_add)(entry);
	if id == NONE {
		return Err("BYOND couldn't add the proc".to_owned());
	}
	recache_path(entry.0[ProcEntryWords::PATH], [PROC_VALUE, id]);
	Ok(ProcId(id))
}

/// Adds the entry even when an identical one exists, unlike `add_proc`.
///
/// # Safety
/// Call on BYOND's thread while a world is live. The entry's IDs must be valid,
/// as for [`set_proc_entry_word`].
pub unsafe fn append_proc(entry: &ProcEntryWords) -> Result<ProcId, String> {
	let path = entry.0[ProcEntryWords::PATH];
	let hidden: Vec<ProcId> = (0..proc_count())
		.map(ProcId)
		.filter(|&id| proc_entry(id).is_some_and(|e| e.0[ProcEntryWords::PATH] == path))
		.collect();
	for &id in &hidden {
		set_proc_entry_word(id, ProcEntryWords::PATH, !path);
	}
	let added = add_proc(entry);
	for &id in &hidden {
		set_proc_entry_word(id, ProcEntryWords::PATH, path);
	}
	added
}

pub unsafe fn proc_count() -> u32 {
	let mut id = 0;
	while proc_entry(ProcId(id)).is_some() {
		id += 1;
	}
	id
}

pub unsafe fn misc_words(id: u32) -> Option<Vec<u32>> {
	if id == NONE {
		return Some(vec![]);
	}
	let mut entry: *mut c_void = std::ptr::null_mut();
	unsafe {
		if crate::raw_types::funcs::get_misc_by_id(&mut entry, misc::MiscId::from_raw(id)) != 1 || entry.is_null() {
			return None;
		}
		let count = *(entry as *const u16) as usize;
		let data = *((entry as *const u8).add(8) as *const *const u32);
		if count == 0 || data.is_null() {
			return Some(vec![]);
		}
		Some(std::slice::from_raw_parts(data, count).to_vec())
	}
}

/// Holds a permanent reference. BYOND shares entries that have the same words.
///
/// # Safety
/// Call on BYOND's thread while a world is live.
pub unsafe fn intern_misc(words: &[u32]) -> Result<u32, String> {
	if words.len() > u16::MAX as usize {
		return Err(format!("{} words is too many for a misc entry", words.len()));
	}
	let functions = functions()?;
	let id = match functions.misc_intern {
		MiscIntern::Copy(copy) => copy(words.as_ptr(), words.len() as u32),
		MiscIntern::Take(take) => unsafe {
			let entry = (functions.crt_realloc)(std::ptr::null_mut(), 12) as *mut u8;
			if entry.is_null() {
				return Err("out of memory making a misc entry".to_owned());
			}
			let mut data = std::ptr::null_mut::<u32>();
			if !words.is_empty() {
				data = (functions.crt_realloc)(std::ptr::null_mut(), words.len() * 4) as *mut u32;
				if data.is_null() {
					return Err("out of memory making a misc entry".to_owned());
				}
				std::ptr::copy_nonoverlapping(words.as_ptr(), data, words.len());
			}
			*(entry as *mut u16) = words.len() as u16;
			*(entry.add(2) as *mut u16) = 0;
			*(entry.add(4) as *mut u32) = 0;
			*(entry.add(8) as *mut *mut u32) = data;
			take(entry)
		}
	};
	if id == NONE {
		return Err("BYOND couldn't make the misc entry".to_owned());
	}
	misc::add_ref(&misc::MiscId::from_raw(id));
	Ok(id)
}

pub unsafe fn type_count() -> Result<u32, String> {
	Ok(unsafe { *functions()?.type_count })
}

unsafe fn type_entry(type_id: u32) -> Result<*mut u8, String> {
	let functions = functions()?;
	unsafe {
		if type_id >= *functions.type_count {
			return Err(format!("there's no type {}", type_id));
		}
		Ok((*functions.type_table).add(type_id as usize * functions.type_stride))
	}
}

unsafe fn type_slot<T>(type_id: u32, offset: usize) -> Result<*mut T, String> {
	let size = std::mem::size_of::<T>();
	if !offset.is_multiple_of(std::mem::align_of::<T>()) || offset > functions()?.type_stride - size {
		return Err(format!("+{:#x} isn't a {}-byte field of a type", offset, size));
	}
	Ok(unsafe { type_entry(type_id)?.add(offset) as *mut T })
}

pub unsafe fn type_field(type_id: u32, offset: usize) -> Result<u32, String> {
	Ok(unsafe { *type_slot::<u32>(type_id, offset)? })
}

pub unsafe fn set_type_field(type_id: u32, offset: usize, value: u32) -> Result<(), String> {
	*type_slot(type_id, offset)? = value;
	Ok(())
}

/// A mob type's entry in the mob table, indexed by a mob typepath value's data.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MobType {
	pub type_id: u32,
	pub key: u32,
	pub sight: u32,
	pub see_in_dark: u8,
	pub see_invisible: u8,
	pub padding: u16
}

/// The proc is named `TYPE/proc/(map_init)`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instance {
	pub tag: u8,
	pub data: u32,
	/// NONE for a type placed on the map without changes.
	pub init: u32
}

const MOB_TYPEPATH: u8 = 0x08;

unsafe fn add_mob_type(mob: MobType) -> Result<u32, String> {
	let functions = functions()?;
	unsafe {
		let count = *functions.mob_count;
		let table = functions.mobs as *mut *mut MobType;
		let grown = (functions.crt_realloc)(*table as *mut c_void, (count as usize + 1) * std::mem::size_of::<MobType>()) as *mut MobType;
		if grown.is_null() {
			return Err("out of memory growing the mob table".to_owned());
		}
		*grown.add(count as usize) = mob;
		*table = grown;
		*(functions.mob_count as *mut u32) = count + 1;
		Ok(count)
	}
}

pub unsafe fn set_mob_type(type_id: u32, set: impl FnOnce(&mut MobType)) -> Result<(), String> {
	let mut mob = *mob_slot(type_id)?;
	set(&mut mob);
	*mob_slot(type_id)? = mob;
	Ok(())
}

unsafe fn mob_slot(type_id: u32) -> Result<*mut MobType, String> {
	let functions = functions()?;
	let mobs = std::slice::from_raw_parts_mut(*functions.mobs, *functions.mob_count as usize);
	let mob = mobs.iter_mut().find(|m| m.type_id == type_id).ok_or("it isn't a mob type")?;
	Ok(mob)
}

pub unsafe fn instances() -> Result<Vec<Instance>, String> {
	let functions = functions()?;
	unsafe {
		let count = *functions.instance_count as usize;
		let table = *functions.instances;
		if count == 0 || table.is_null() {
			return Ok(vec![]);
		}
		Ok(std::slice::from_raw_parts(table, count).to_vec())
	}
}

pub unsafe fn add_instance(instance: Instance) -> Result<u32, String> {
	let functions = functions()?;
	unsafe {
		let count = *functions.instance_count;
		let table = functions.instances as *mut *mut Instance;
		let grown = (functions.crt_realloc)(*table as *mut c_void, (count as usize + 1) * std::mem::size_of::<Instance>()) as *mut Instance;
		if grown.is_null() {
			return Err("out of memory growing the prefab instances".to_owned());
		}
		*grown.add(count as usize) = instance;
		*table = grown;
		*(functions.instance_count as *mut u32) = count + 1;
		Ok(count)
	}
}

pub unsafe fn in_list_count() -> Result<u32, String> {
	Ok(unsafe { *functions()?.in_list_count })
}

pub unsafe fn in_list_procs() -> Result<Vec<u32>, String> {
	let functions = functions()?;
	unsafe {
		let count = *functions.in_list_count as usize;
		let table = *functions.in_list_procs;
		if table.is_null() {
			return Ok(vec![]);
		}
		Ok(std::slice::from_raw_parts(table, count).to_vec())
	}
}

/// Returns the index for the parameter's word, `(index << 8) | 0x40`.
///
/// # Safety
/// Call on BYOND's thread while a world is live. `proc` must be a proc ID.
pub unsafe fn add_in_list(proc: ProcId) -> Result<u32, String> {
	let functions = functions()?;
	unsafe {
		let count = *functions.in_list_count;
		let table = functions.in_list_procs as *mut *mut u32;
		let grown = (functions.crt_realloc)(*table as *mut c_void, (count as usize + 1) * 4) as *mut u32;
		if grown.is_null() {
			return Err("out of memory growing the `in` list procs".to_owned());
		}
		*grown.add(count as usize) = proc.0;
		*table = grown;
		*(functions.in_list_count as *mut u32) = count + 1;
		Ok(count)
	}
}

/// Existing instances keep the old one.
///
/// # Safety
/// Call on BYOND's thread while a world is live.
pub unsafe fn forget_type_appearance(type_id: u32) -> Result<(), String> {
	let functions = functions()?;
	unsafe {
		if type_id >= *functions.type_count {
			return Err(format!("there's no type {}", type_id));
		}
		*(*functions.appearance_cache).add(type_id as usize) = NONE;
	}
	Ok(())
}

unsafe fn name_map(type_id: u32) -> Result<*mut ProcNameMap, String> {
	let functions = functions()?;
	unsafe {
		if type_id >= *functions.type_count {
			return Err(format!("there's no type {}", type_id));
		}
		Ok((*functions.name_maps).add(type_id as usize))
	}
}

unsafe fn buckets<'a>(map: *mut ProcNameMap) -> &'a [ProcNameBucket] {
	if (*map).buckets.is_null() {
		&[]
	} else {
		std::slice::from_raw_parts((*map).buckets, (*map).count as usize)
	}
}

/// The procs that a name finds on a type, most derived first. `key` is the
/// name's string id.
///
/// # Safety
/// Call on BYOND's thread while a world is live.
pub unsafe fn named_procs(type_id: u32, key: u32) -> Result<Vec<u32>, String> {
	named_procs_in(name_map(type_id)?, key)
}

pub unsafe fn world_named_procs(key: u32) -> Result<Vec<u32>, String> {
	named_procs_in(functions()?.world_name_map, key)
}

unsafe fn named_procs_in(map: *mut ProcNameMap, key: u32) -> Result<Vec<u32>, String> {
	unsafe {
		let buckets = buckets(map);
		Ok(match buckets.binary_search_by_key(&key, |bucket| bucket.key) {
			Ok(i) if !buckets[i].ids.is_null() => std::slice::from_raw_parts(buckets[i].ids, buckets[i].count as usize).to_vec(),
			_ => vec![]
		})
	}
}

/// Takes over the new shared buffer. Leaves a map that BYOND hasn't built
/// unchanged.
///
/// # Safety
/// Call on BYOND's thread while a world is live. `procs` must be proc IDs.
pub unsafe fn set_named_procs(type_id: u32, key: u32, procs: &[u32]) -> Result<(), String> {
	let map = name_map(type_id)?;
	if unsafe { (*map).built } == 0 {
		return Ok(());
	}
	set_named_procs_in(map, key, procs)
}

/// BYOND builds the map after global init and before world/New. Edits made
/// before then are lost.
///
/// # Safety
/// Call on BYOND's thread while a world is live.
pub unsafe fn world_names_built() -> Result<bool, String> {
	unsafe { Ok((*functions()?.world_name_map).built != 0) }
}

pub unsafe fn set_world_named_procs(key: u32, procs: &[u32]) -> Result<(), String> {
	set_named_procs_in(functions()?.world_name_map, key, procs)
}

fn check_name_map_count(what: &str, count: usize) -> Result<u16, String> {
	u16::try_from(count).map_err(|_| format!("{} {} is more than a proc name map holds ({})", count, what, u16::MAX))
}

unsafe fn set_named_procs_in(map: *mut ProcNameMap, key: u32, procs: &[u32]) -> Result<(), String> {
	let functions = functions()?;
	let proc_count = check_name_map_count("procs for one name", procs.len())?;
	unsafe {
		let count = buckets(map).len();
		let at = match buckets(map).binary_search_by_key(&key, |bucket| bucket.key) {
			Ok(at) => at,
			Err(at) => {
				check_name_map_count("names", count + 1)?;
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
		bucket.count = proc_count;
	}
	Ok(())
}

/// Holds a reference to the new value and releases the old one.
///
/// # Safety
/// Call on BYOND's thread while a world is live.
pub unsafe fn set_global_value(var_id: u32, value: crate::Value) -> Result<(), String> {
	let functions = functions()?;
	unsafe {
		if var_id >= variable_count() {
			return Err(format!("var ID {} does not exist", var_id));
		}
		let slot = (*functions.global_values as *mut crate::raw_types::values::Value).add(var_id as usize);
		drop(crate::Value::from_raw_owned(*slot));
		*slot = value.raw;
		std::mem::forget(value);
	}
	Ok(())
}

/// Takes over the caller's reference to the value and releases the old one.
///
/// # Safety
/// Call on BYOND's thread while a world is live. The caller must own a
/// reference to the value, and this takes it over.
pub unsafe fn set_global_words(var_id: u32, tag: u8, data: u32) -> Result<(), String> {
	let functions = functions()?;
	unsafe {
		if var_id >= variable_count() {
			return Err(format!("var ID {} does not exist", var_id));
		}
		let slot = (*functions.global_values as *mut [u32; 2]).add(var_id as usize);
		drop(crate::Value::from_raw_owned(*(slot as *const crate::raw_types::values::Value)));
		slot.write([tag as u32, data]);
	}
	Ok(())
}

pub unsafe fn global_value(var_id: u32) -> Result<crate::raw_types::values::Value, String> {
	let functions = functions()?;
	unsafe {
		if var_id >= variable_count() {
			return Err(format!("var ID {} does not exist", var_id));
		}
		Ok(*(*functions.global_values).add(var_id as usize))
	}
}

pub unsafe fn add_global_vars(vars: &[(u32, u32)]) -> Result<(), String> {
	let global_vars = functions()?.global_vars.clone()?;
	let current = unsafe { *global_vars };
	let mut words = misc_words(current).ok_or_else(|| format!("global-var misc entry {} is missing", current))?;
	let name = |var_id: u32| variable_name(var_id).ok_or_else(|| format!("var ID {} does not exist", var_id));
	for &(var_id, flags) in vars {
		if words.chunks(2).any(|pair| pair[0] == var_id) {
			continue;
		}
		let key = name(var_id)?;
		let mut at = 0;
		for pair in words.chunks(2) {
			if name(pair[0])? > key {
				break;
			}
			at += 2;
		}
		words.splice(at..at, [var_id, flags]);
	}
	let id = intern_misc(&words)?;
	unsafe {
		*global_vars = id;
	}
	Ok(())
}

pub unsafe fn global_var_ids() -> Result<Vec<u32>, String> {
	let current = unsafe { *functions()?.global_vars.clone()? };
	let words = misc_words(current).ok_or_else(|| format!("global-var misc entry {} is missing", current))?;
	Ok(words.chunks(2).map(|pair| pair[0]).collect())
}

pub unsafe fn variable_count() -> u32 {
	unsafe { *crate::raw_types::funcs::VARIABLE_NAME_COUNT }
}

pub unsafe fn variable_name(var_id: u32) -> Option<u32> {
	unsafe { (var_id < variable_count()).then(|| (*(*crate::raw_types::funcs::VARIABLE_NAME_ENTRIES).add(var_id as usize)).0) }
}

/// The slot takes over the reference to the initial value.
///
/// # Safety
/// Call on BYOND's thread while a world is live. The caller must own a
/// reference to the value, and the slot takes it over.
pub unsafe fn add_variable(name: u32, tag: u8, data: u32) -> Result<u32, String> {
	let functions = functions()?;
	unsafe {
		let entries = crate::raw_types::funcs::VARIABLE_NAME_ENTRIES as *mut *const crate::raw_types::strings::StringId;
		let count_ptr = crate::raw_types::funcs::VARIABLE_NAME_COUNT as *mut u32;
		let count = *count_ptr as usize;
		if count as u32 >= NONE {
			return Err("BYOND variable ID limit reached".to_owned());
		}

		let names = (functions.crt_realloc)(*entries as *mut c_void, (count + 1) * 4) as *mut u32;
		if names.is_null() {
			return Err("out of memory growing the var names".to_owned());
		}
		*entries = names as *const crate::raw_types::strings::StringId;

		let slots = functions.global_values as *mut *mut [u32; 2];
		let values = (functions.crt_realloc)(*slots as *mut c_void, (count + 1) * 8) as *mut [u32; 2];
		if values.is_null() {
			return Err("out of memory growing the var values".to_owned());
		}
		*slots = values;

		names.add(count).write(name);
		values.add(count).write([tag as u32, data]);
		*count_ptr += 1;
		Ok(count as u32)
	}
}

/// Shares the parent's var lists, init proc and proc name buffers.
///
/// # Safety
/// Call on BYOND's thread while a world is live. `path` must be a string ID
/// that holds a reference.
pub unsafe fn add_type(parent: u32, path: u32) -> Result<u32, String> {
	use type_fields::*;
	let functions = functions()?;
	let register_typepath = functions.register_typepath.clone()?;
	unsafe {
		let count = *functions.type_count as usize;
		if parent as usize >= count {
			return Err(format!("there's no type {}", parent));
		}
		if count + 1 >= NONE as usize {
			return Err("BYOND type ID limit reached".to_owned());
		}

		let grow = |table: *const *mut u8, size: usize| -> Result<*mut u8, String> {
			let grown = (functions.crt_realloc)(*table as *mut c_void, size) as *mut u8;
			if grown.is_null() {
				return Err("out of memory growing the type tables".to_owned());
			}
			*(table as *mut *mut u8) = grown;
			Ok(grown)
		};
		let stride = functions.type_stride;
		let table = grow(functions.type_table, (count + 1) * stride)?;
		let preorder = grow(functions.preorder as *const *mut u8, (count + 1) * 4)? as *mut u32;
		let maps = grow(functions.name_maps as *const *mut u8, (count + 1) * 8)? as *mut ProcNameMap;
		let cache = grow(functions.appearance_cache as *const *mut u8, (count + 1) * 4)? as *mut u32;

		let entry = |id: usize| table.add(id * stride);
		let word = |id: usize, offset: usize| entry(id).add(offset) as *mut u32;

		std::ptr::copy_nonoverlapping(entry(parent as usize), entry(count), stride);
		*word(count, PATH) = path;
		*word(count, PARENT) = parent;
		for offset in [VERBS, PROCS, OWN_OVERRIDES] {
			*word(count, offset) = NONE;
		}
		for offset in [DECLARED_VARS, BUILTIN_VALUES, VAR_TABLE, EXTRA] {
			let id = *word(count, offset);
			if id != NONE {
				misc::add_ref(&misc::MiscId::from_raw(id));
			}
		}
		for (offset, size) in [(TRANSFORM, 6 * 4), (COLOR_MATRIX, 20 * 4)] {
			let from = *word(count, offset) as *const u8;
			if !from.is_null() {
				let copy = (functions.crt_realloc)(std::ptr::null_mut(), size) as *mut u8;
				if copy.is_null() {
					return Err("out of memory copying a type".to_owned());
				}
				std::ptr::copy_nonoverlapping(from, copy, size);
				*word(count, offset) = copy as u32;
			}
		}
		*word(count, INIT_SEGMENTS) = 0;
		if register_typepath.is_some() {
			*word(count, TYPEPATH) = 0;
			*word(count, TYPEPATH + 4) = 0;
		}

		let from = &*maps.add(parent as usize);
		let mut buckets = std::ptr::null_mut();
		if from.count > 0 && !from.buckets.is_null() {
			let size = from.count as usize * std::mem::size_of::<ProcNameBucket>();
			buckets = (functions.crt_realloc)(std::ptr::null_mut(), size) as *mut ProcNameBucket;
			if buckets.is_null() {
				return Err("out of memory copying a proc name map".to_owned());
			}
			std::ptr::copy_nonoverlapping(from.buckets, buckets, from.count as usize);
			for i in 0..from.count as usize {
				let bucket = &mut *buckets.add(i);
				if !bucket.ids.is_null() {
					bucket.ids = (functions.copy_shared)(bucket.ids as *mut c_void) as *mut u32;
				}
			}
		}
		maps.add(count).write(ProcNameMap {
			buckets,
			count: from.count,
			built: from.built
		});
		*cache.add(count) = NONE;

		let at = *word(parent as usize, PREORDER_END) as usize;
		std::ptr::copy(preorder.add(at), preorder.add(at + 1), count - at);
		*preorder.add(at) = count as u32;
		for id in 0..count {
			if *word(id, PREORDER) as usize >= at {
				*word(id, PREORDER) += 1;
				*word(id, PREORDER_END) += 1;
			}
		}
		let mut ancestor = parent;
		while (ancestor as usize) < count {
			*word(ancestor as usize, PREORDER_END) += 1;
			ancestor = *word(ancestor as usize, PARENT);
		}
		*word(count, PREORDER) = at as u32;
		*word(count, PREORDER_END) = at as u32 + 1;

		*(functions.type_count as *mut u32) += 1;

		let mobs = std::slice::from_raw_parts(*functions.mobs, *functions.mob_count as usize);
		let mob = match register_typepath {
			Some(register) => {
				if *(word(parent as usize, TYPEPATH) as *const u8) == 0 {
					register(parent);
				}
				*(word(parent as usize, TYPEPATH) as *const u8) == MOB_TYPEPATH
			}
			None => mobs.iter().any(|m| m.type_id == parent)
		};
		if mob {
			let from = mobs
				.iter()
				.find(|m| m.type_id == parent)
				.copied()
				.ok_or("parent mob type is missing from the mob table")?;
			add_mob_type(MobType {
				type_id: count as u32,
				..from
			})?;
		}
		if let Some(register) = register_typepath {
			register(count as u32);
			let typepath = *(word(count, TYPEPATH) as *const [u32; 2]);
			recache_path(path, typepath);
		}
		Ok(count as u32)
	}
}

pub unsafe fn set_type_half(type_id: u32, offset: usize, value: u16) -> Result<(), String> {
	*type_slot(type_id, offset)? = value;
	Ok(())
}

pub unsafe fn set_type_byte(type_id: u32, offset: usize, value: u8) -> Result<(), String> {
	*type_slot(type_id, offset)? = value;
	Ok(())
}

pub unsafe fn name_map_entries(type_id: u32) -> Result<Vec<(u32, Vec<u32>, u8)>, String> {
	unsafe {
		Ok(buckets(name_map(type_id)?)
			.iter()
			.map(|bucket| {
				let ids = if bucket.ids.is_null() {
					vec![]
				} else {
					std::slice::from_raw_parts(bucket.ids, bucket.count as usize).to_vec()
				};
				(bucket.key, ids, bucket.no_link)
			})
			.collect())
	}
}

/// A preallocated proc name map. Dropping it unwritten releases its buffers.
pub struct NameMapPlan {
	type_id: u32,
	world: u32,
	buckets: *mut ProcNameBucket,
	count: u16,
	written: bool
}

pub unsafe fn plan_name_map(type_id: u32, entries: &[(u32, Vec<u32>, u8)]) -> Result<NameMapPlan, String> {
	let functions = functions()?;
	name_map(type_id)?;
	let mut entries: Vec<&(u32, Vec<u32>, u8)> = entries.iter().filter(|(_, ids, _)| !ids.is_empty()).collect();
	entries.sort_by_key(|(key, ..)| *key);
	check_name_map_count("names", entries.len())?;
	for (_, ids, _) in &entries {
		check_name_map_count("procs for one name", ids.len())?;
	}
	let size = entries.len().max(1) * std::mem::size_of::<ProcNameBucket>();
	unsafe {
		let buckets = (functions.crt_realloc)(std::ptr::null_mut(), size) as *mut ProcNameBucket;
		if buckets.is_null() {
			return Err("out of memory for a proc name map".to_owned());
		}
		let mut plan = NameMapPlan {
			type_id,
			world: crate::world_generation(),
			buckets,
			count: 0,
			written: false
		};
		for (key, ids, no_link) in entries {
			let buffer = (functions.alloc_shared)(ids.len() * 4) as *mut u32;
			if buffer.is_null() {
				return Err("out of memory for a proc name map entry".to_owned());
			}
			std::ptr::copy_nonoverlapping(ids.as_ptr(), buffer, ids.len());
			buckets.add(plan.count as usize).write(ProcNameBucket {
				key: *key,
				ids: buffer,
				count: ids.len() as u16,
				no_link: *no_link
			});
			plan.count += 1;
		}
		Ok(plan)
	}
}

impl NameMapPlan {
	/// Keeps the old bucket array, as with type matrices. Finds the type's map
	/// again, because adding a type moves the maps.
	///
	/// # Safety
	/// Call on BYOND's thread while a world is live.
	pub unsafe fn write(mut self) -> Result<(), String> {
		if self.world != crate::world_generation() {
			return Err("the proc name map was planned for a world that has ended".to_owned());
		}
		let functions = functions()?;
		let map = name_map(self.type_id)?;
		unsafe {
			let old_ids: Vec<*mut u32> = buckets(map).iter().map(|bucket| bucket.ids).collect();
			(*map).buckets = self.buckets;
			(*map).count = self.count;
			for ids in old_ids {
				if !ids.is_null() {
					(functions.free_shared)(ids as *mut c_void);
				}
			}
		}
		self.written = true;
		Ok(())
	}
}

impl Drop for NameMapPlan {
	fn drop(&mut self) {
		if self.written || self.world != crate::world_generation() {
			return;
		}
		unsafe {
			if let Ok(functions) = functions() {
				for at in 0..self.count as usize {
					(functions.free_shared)((*self.buckets.add(at)).ids as *mut c_void);
				}
				(functions.crt_free)(self.buckets as *mut c_void);
			}
		}
	}
}

pub struct ParentPlan {
	world: u32,
	parents_before: Vec<u32>,
	changes: Vec<(u32, u32)>,
	order: Vec<u32>,
	places: Vec<(u32, u32)>
}

/// Visits children by ascending id, as BYOND does.
///
/// # Safety
/// Call on BYOND's thread while a world is live.
pub unsafe fn plan_parents(changes: &[(u32, u32)]) -> Result<ParentPlan, String> {
	use type_fields::*;
	let functions = functions()?;
	let count = type_count()? as usize;
	for &(type_id, parent) in changes {
		if type_id as usize >= count || parent as usize >= count {
			return Err(format!("type {} or parent {} does not exist", type_id, parent));
		}
	}
	unsafe {
		let word = |id: usize, offset: usize| (*functions.type_table).add(id * functions.type_stride + offset) as *mut u32;
		let parents_before: Vec<u32> = (0..count).map(|id| *word(id, PARENT)).collect();
		let mut parents = parents_before.clone();
		for &(type_id, parent) in changes {
			parents[type_id as usize] = parent;
		}
		let mut children: Vec<Vec<u32>> = vec![vec![]; count];
		let mut roots = vec![];
		for (id, &parent) in parents.iter().enumerate() {
			if (parent as usize) < count && parent as usize != id {
				children[parent as usize].push(id as u32);
			} else {
				roots.push(id as u32);
			}
		}
		let mut order = Vec::with_capacity(count);
		let mut places = vec![(0u32, 0u32); count];
		let mut stack: Vec<(u32, usize)> = vec![];
		for root in roots {
			places[root as usize].0 = order.len() as u32;
			order.push(root);
			stack.push((root, 0));
			while let Some(top) = stack.last_mut() {
				let (id, next) = *top;
				match children[id as usize].get(next) {
					Some(&child) => {
						top.1 += 1;
						places[child as usize].0 = order.len() as u32;
						order.push(child);
						stack.push((child, 0));
					}
					None => {
						places[id as usize].1 = order.len() as u32;
						stack.pop();
					}
				}
			}
		}
		if order.len() != count {
			return Err(format!(
				"type hierarchy contains a cycle: preorder includes {} of {} types",
				order.len(),
				count
			));
		}
		Ok(ParentPlan {
			world: crate::world_generation(),
			parents_before,
			changes: changes.to_vec(),
			order,
			places
		})
	}
}

impl ParentPlan {
	/// Fails, and writes nothing, when the world or its types have changed
	/// since the plan was made.
	///
	/// # Safety
	/// Call on BYOND's thread while a world is live.
	pub unsafe fn write(&self) -> Result<(), String> {
		use type_fields::*;
		if self.world != crate::world_generation() {
			return Err("the type hierarchy was planned for a world that has ended".to_owned());
		}
		let functions = functions()?;
		unsafe {
			let count = *functions.type_count as usize;
			let word = |id: usize, offset: usize| (*functions.type_table).add(id * functions.type_stride + offset) as *mut u32;
			if count != self.parents_before.len() || (0..count).any(|id| *word(id, PARENT) != self.parents_before[id]) {
				return Err(format!(
					"the type hierarchy was planned for {} types, and types were added or moved since",
					self.parents_before.len()
				));
			}
			for &(type_id, parent) in &self.changes {
				*word(type_id as usize, PARENT) = parent;
			}
			let preorder = *functions.preorder;
			for (at, &id) in self.order.iter().enumerate() {
				*preorder.add(at) = id;
				*word(id as usize, PREORDER) = self.places[id as usize].0;
				*word(id as usize, PREORDER_END) = self.places[id as usize].1;
			}
		}
		Ok(())
	}
}

/// Each type owns its copy. The old copy is kept, because readers may still
/// use it.
///
/// # Safety
/// Call on BYOND's thread while a world is live.
pub unsafe fn copy_type_matrix(type_id: u32, from: u32, offset: usize) -> Result<(), String> {
	use type_fields::*;
	let size = match offset {
		TRANSFORM => 6 * 4,
		COLOR_MATRIX => 20 * 4,
		_ => return Err(format!("+{:#x} isn't a type's matrix", offset))
	};
	let functions = functions()?;
	unsafe {
		let source = type_field(from, offset)? as *const u8;
		let copy = if source.is_null() {
			0
		} else {
			let copy = (functions.crt_realloc)(std::ptr::null_mut(), size) as *mut u8;
			if copy.is_null() {
				return Err("out of memory copying a type's matrix".to_owned());
			}
			std::ptr::copy_nonoverlapping(source, copy, size);
			copy as u32
		};
		set_type_field(type_id, offset, copy)
	}
}
