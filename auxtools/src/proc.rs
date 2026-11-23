use std::{
	cell::RefCell,
	collections::{hash_map::Entry, HashMap},
	fmt,
	marker::PhantomData
};

use ahash::RandomState;
use fxhash::FxHashMap;

use crate::*;

/// Not `Send`. A call from another thread would run DM code during a tick.
///
/// ```compile_fail
/// fn assert_send<T: Send>() {}
/// assert_send::<auxtools::Proc>();
/// ```
#[derive(Clone)]
pub struct Proc {
	pub id: raw_types::procs::ProcId,
	pub path: String,
	not_send: PhantomData<*mut ()>
}

fn entry_by_id(id: raw_types::procs::ProcId) -> *mut raw_types::procs::ProcEntry {
	let mut entry: *mut raw_types::procs::ProcEntry = std::ptr::null_mut();
	unsafe {
		assert_eq!(raw_types::funcs::get_proc_array_entry(&mut entry, id), 1);
	}
	entry
}

impl Proc {
	/// The override id of a proc set apart by [`Proc::set_apart`].
	pub const APART: u32 = u32::MAX;

	pub fn overrides<S: Into<String>>(path: S) -> Vec<Self> {
		let path = strip_path(path.into());
		PROCS_BY_NAME.with(|h| h.borrow().get(&path).cloned().unwrap_or_default())
	}

	/// [`Proc::find_override`] no longer finds the definitions left out.
	pub fn set_overrides<S: Into<String>>(path: S, procs: Vec<Self>) {
		let path = strip_path(path.into());
		PROC_OVERRIDE_IDS.with(|override_ids| {
			let mut override_ids = override_ids.borrow_mut();
			for (i, proc) in procs.iter().enumerate() {
				override_ids.insert(proc.id, i as u32);
			}
		});
		PROCS_BY_NAME.with(|h| h.borrow_mut().insert(path, procs));
	}

	/// Valid only until DM code runs again. BYOND moves the table when it
	/// grows.
	pub fn entry(&self) -> *mut raw_types::procs::ProcEntry {
		entry_by_id(self.id)
	}

	/// Finds the first definition of `path`. [`Proc::find_override`] finds the
	/// others.
	pub fn find<S: Into<String>>(path: S) -> Option<Self> {
		get_proc(path)
	}

	/// With [`Proc::APART`], finds the proc set apart for `path`.
	pub fn find_override<S: Into<String>>(path: S, override_id: u32) -> Option<Self> {
		get_proc_override(path, override_id)
	}

	pub fn from_id(id: raw_types::procs::ProcId) -> Option<Self> {
		let proc_entry = entry_by_id(id);
		if proc_entry.is_null() {
			return None;
		}
		let path = strip_path(unsafe { StringRef::from_id((*proc_entry).path).into() });
		Some(Proc {
			id,
			path,
			not_send: PhantomData
		})
	}

	pub unsafe fn file_name(&self) -> Option<StringRef> {
		let bytecode = self.bytecode();
		if bytecode.len() < 2 || bytecode[0] != 0x84 {
			return None;
		}

		let file_id = raw_types::strings::StringId(bytecode[0x01]);
		if !file_id.valid() {
			return None;
		}

		Some(StringRef::from_id(file_id))
	}

	pub fn parameter_names(&self) -> Vec<StringRef> {
		unsafe {
			let (data, count) = raw_types::misc::get_parameters((*self.entry()).metadata.parameters);
			(0..count).map(|i| StringRef::from_variable_id((*data.add(i)).name)).collect()
		}
	}

	pub fn local_names(&self) -> Vec<StringRef> {
		unsafe {
			let (names, count) = raw_types::misc::get_locals((*self.entry()).metadata.locals);
			(0..count).map(|i| StringRef::from_variable_id(*names.add(i))).collect()
		}
	}

	pub fn set_bytecode(&self, bytecode: Vec<u32>) {
		crate::bytecode_manager::set_bytecode(self, bytecode);
	}

	pub fn add_to_table(first: raw_types::procs::ProcId) {
		let mut id = first.0;
		while let Some(proc) = Proc::from_id(raw_types::procs::ProcId(id)) {
			register_proc(proc);
			id += 1;
		}
	}

	pub fn locals_count(&self) -> usize {
		unsafe { raw_types::misc::get_locals((*self.entry()).metadata.locals).1 }
	}

	pub unsafe fn bytecode_mut_ptr(&self) -> (*mut u32, u16) {
		raw_types::misc::get_bytecode((*self.entry()).metadata.bytecode)
	}

	pub unsafe fn bytecode(&self) -> &[u32] {
		let (ptr, count) = self.bytecode_mut_ptr();
		std::slice::from_raw_parts(ptr, count as usize)
	}

	/// Calls the proc with a null `src` and `usr`.
	pub fn call(&self, args: &[&Value]) -> runtime::DMResult {
		let mut ret = raw_types::values::Value {
			tag: raw_types::values::ValueTag::Null,
			data: raw_types::values::ValueData { id: 0 }
		};

		unsafe {
			for v in args {
				raw_types::funcs::inc_ref_count(v.raw);
			}

			let args: Vec<_> = args.iter().map(|e| e.raw).collect();

			if raw_types::funcs::call_proc_by_id(&mut ret, Value::NULL.raw, 2, self.id, 0, Value::NULL.raw, args.as_ptr(), args.len(), 0, 0) == 1 {
				return Ok(Value::from_raw_owned(ret));
			}
		}

		Err(runtime!("External proc call failed"))
	}

	/// Gives this proc the override id [`Proc::APART`]. The other definitions
	/// keep their ids. A path has only one proc set apart, so this replaces any
	/// earlier one.
	pub fn set_apart(&self) {
		PROC_OVERRIDE_IDS.with(|override_ids| {
			let mut override_ids = override_ids.borrow_mut();
			PROCS_BY_NAME.with(|h| {
				if let Some(procs) = h.borrow_mut().get_mut(&self.path) {
					procs.retain(|proc| proc.id != self.id);
					for (i, proc) in procs.iter().enumerate() {
						override_ids.insert(proc.id, i as u32);
					}
				}
			});
			override_ids.insert(self.id, Self::APART);
		});
		PROCS_APART.with(|apart| apart.borrow_mut().insert(self.path.clone(), self.clone()));
	}

	pub fn override_id(&self) -> u32 {
		PROC_OVERRIDE_IDS.with(|override_ids| match override_ids.borrow().get(&self.id) {
			Some(id) => *id,
			None => 0
		})
	}
}

impl fmt::Debug for Proc {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let path = unsafe { (*self.entry()).path };
		write!(f, "Proc({:?})", unsafe { StringRef::from_id(path) })
	}
}

thread_local!(static PROCS_BY_NAME: RefCell<HashMap<String, Vec<Proc>, RandomState>> = RefCell::new(HashMap::with_hasher(RandomState::default())));
thread_local!(static PROC_OVERRIDE_IDS: RefCell<FxHashMap<raw_types::procs::ProcId, u32>> = RefCell::new(FxHashMap::default()));
thread_local!(static PROCS_APART: RefCell<HashMap<String, Proc, RandomState>> = RefCell::new(HashMap::with_hasher(RandomState::default())));

fn strip_path(p: String) -> String {
	p.replace("/proc/", "/").replace("/verb/", "/")
}

pub fn populate_procs() {
	let mut i: u32 = 0;
	loop {
		let proc = Proc::from_id(raw_types::procs::ProcId(i));
		if proc.is_none() {
			break;
		}
		register_proc(proc.unwrap());

		i += 1;
	}
}

fn register_proc(proc: Proc) {
	PROC_OVERRIDE_IDS.with(|override_ids| {
		let mut override_ids = override_ids.borrow_mut();

		PROCS_BY_NAME.with(|h| {
			match h.borrow_mut().entry(proc.path.clone()) {
				Entry::Occupied(mut o) => {
					let vec = o.get_mut();
					override_ids.insert(proc.id, vec.len() as u32);
					vec.push(proc);
				}
				Entry::Vacant(v) => {
					override_ids.insert(proc.id, 0);
					v.insert(vec![proc]);
				}
			};
		});
	});
}

pub fn clear_procs() {
	PROCS_BY_NAME.with(|h| h.borrow_mut().clear());
	PROC_OVERRIDE_IDS.with(|override_ids| override_ids.borrow_mut().clear());
	PROCS_APART.with(|apart| apart.borrow_mut().clear());
}

pub fn get_proc_override<S: Into<String>>(path: S, override_id: u32) -> Option<Proc> {
	let s = strip_path(path.into());
	if override_id == Proc::APART {
		return PROCS_APART.with(|apart| apart.borrow().get(&s).cloned());
	}
	let found = PROCS_BY_NAME.with(|h| h.borrow().get(&s)?.get(override_id as usize).cloned());
	if found.is_none() && s.ends_with("/(init)") && adopt_named(&s) {
		return PROCS_BY_NAME.with(|h| h.borrow().get(&s)?.get(override_id as usize).cloned());
	}
	found
}

fn adopt_named(path: &str) -> bool {
	if PROCS_BY_NAME.with(|h| h.borrow().contains_key(path)) {
		return false;
	}
	let unnamed = PROCS_BY_NAME.with(|h| h.borrow().get("").cloned().unwrap_or_default());
	let named: Vec<Proc> = unnamed
		.iter()
		.filter_map(|proc| Proc::from_id(proc.id))
		.filter(|proc| proc.path == path)
		.collect();
	let any = !named.is_empty();
	for proc in named {
		register_proc(proc);
	}
	any
}

pub fn get_proc<S: Into<String>>(path: S) -> Option<Proc> {
	get_proc_override(path, 0)
}
