//! Rewrite a compiled world so that DreamDaemon runs it in less memory.
//!
//! Each pass changes the tables of a [`Dmb`] in place and says what it did.
//! A pass leaves alone whatever it cannot show to be safe to change, so the
//! world behaves as before except where the pass's documentation says.

pub mod names;
pub mod refs;

use std::collections::{HashMap, HashSet};

use dmasm::{
	dmb::{Dmb, NONE},
	Reference, ReferenceKind
};

/// The tag of a value that is a proc's path.
const PROC_VALUE: u8 = 0x26;
/// The tag of a value that is text.
const TEXT_VALUE: u8 = 0x06;
/// Bytes of a type's proc table that one name of one proc takes.
pub const PROC_ENTRY_BYTES: u64 = 12;

/// A proc's path, split into the type it is on and its identifier.
pub struct ProcPath<'a> {
	/// The type's path, empty for a global proc.
	pub owner: &'a [u8],
	/// Whether the path names a verb (`/mob/verb/say`).
	pub verb: bool,
	/// The last part of the path, which code calls the proc by.
	pub identifier: &'a [u8]
}

impl<'a> ProcPath<'a> {
	pub fn parse(path: &'a [u8]) -> Option<Self> {
		let slash = path.iter().rposition(|&byte| byte == b'/')?;
		let (mut owner, identifier) = (&path[..slash], &path[slash + 1..]);
		if identifier.is_empty() {
			return None;
		}
		// A declaration has `proc` or `verb` before the identifier, an
		// override has neither.
		let mut verb = false;
		if let Some(rest) = owner.strip_suffix(b"/proc") {
			owner = rest;
		} else if let Some(rest) = owner.strip_suffix(b"/verb") {
			owner = rest;
			verb = true;
		}
		Some(Self { owner, verb, identifier })
	}

	pub const fn global(&self) -> bool {
		self.owner.is_empty()
	}
}

/// The ids of the world's strings by their text, and new strings as they are
/// added.
pub struct Strings {
	ids: HashMap<Vec<u8>, u32>
}

impl Strings {
	pub fn new(dmb: &Dmb) -> Self {
		let mut ids = HashMap::with_capacity(dmb.strings.len());
		for (id, text) in dmb.strings.iter().enumerate() {
			ids.entry(text.clone()).or_insert(id as u32);
		}
		Self { ids }
	}

	pub fn get(&self, text: &[u8]) -> Option<u32> {
		self.ids.get(text).copied()
	}

	/// The id of `text`, added to the world if it has no such string. `None`
	/// when the world's ids are 16 bits wide and all are taken.
	pub fn intern(&mut self, dmb: &mut Dmb, text: &[u8]) -> Option<u32> {
		if let Some(id) = self.get(text) {
			return Some(id);
		}
		// 0xFFFF means "none" where an id is optional. DreamMaker leaves an
		// empty string at an id it cannot use, too.
		if dmb.strings.len() == NONE as usize {
			if !dmb.large_ids() {
				return None;
			}
			dmb.strings.push(Vec::new());
		}
		if !dmb.large_ids() && dmb.strings.len() >= NONE as usize {
			return None;
		}
		let id = dmb.strings.len() as u32;
		dmb.strings.push(text.to_vec());
		self.ids.insert(text.to_vec(), id);
		Some(id)
	}
}

/// What the world's bytecode and var values refer to.
pub struct Uses {
	/// The lists that hold a proc's bytecode, each once.
	pub code: Vec<u32>,
	/// The references of each list of `code`, or `None` for bytecode that
	/// could not be read.
	pub references: HashMap<u32, Option<Vec<Reference>>>,
	/// Procs whose paths are used as values.
	pub proc_values: HashSet<u32>,
	/// Strings used as text.
	pub text: HashSet<u32>,
	/// Every word of the bytecode that could not be read.
	pub unread_words: HashSet<u32>,
	/// How many procs have bytecode that could not be read.
	pub unread: usize
}

impl Uses {
	pub fn new(dmb: &Dmb) -> Self {
		let mut uses = Self {
			code: Vec::new(),
			references: HashMap::new(),
			proc_values: HashSet::new(),
			text: HashSet::new(),
			unread_words: HashSet::new(),
			unread: 0
		};
		for proc in &dmb.procs {
			if uses.references.contains_key(&proc.code) {
				continue;
			}
			let Some(words) = dmb.lists.get(proc.code as usize) else {
				continue;
			};
			let references = match dmasm::scan(words) {
				Ok(scanned) => {
					for reference in &scanned.references {
						match reference.kind {
							ReferenceKind::String => {
								uses.text.insert(reference.id);
							}
							ReferenceKind::Value(PROC_VALUE) => {
								uses.proc_values.insert(reference.id);
							}
							_ => {}
						}
					}
					Some(scanned.references)
				}
				Err(_) => {
					uses.unread += 1;
					uses.unread_words.extend(words.iter().copied());
					None
				}
			};
			uses.code.push(proc.code);
			uses.references.insert(proc.code, references);
		}
		// What vars start as: globals, and the vars types set.
		for var in &dmb.vars {
			match var.tag {
				PROC_VALUE => {
					uses.proc_values.insert(var.data);
				}
				TEXT_VALUE => {
					uses.text.insert(var.data);
				}
				_ => {}
			}
		}
		uses
	}
}

/// The lists that something other than a proc's bytecode refers to. Lists
/// with the same words are one list in the file, so bytecode is changed in
/// place only where no other table uses its list.
pub fn lists_not_code(dmb: &Dmb) -> HashSet<u32> {
	let mut lists = HashSet::new();
	for proc in &dmb.procs {
		lists.extend([proc.locals, proc.params]);
	}
	for ty in &dmb.types {
		lists.extend([ty.verbs, ty.procs, ty.set_vars, ty.declared_vars, ty.builtin_vars, ty.unknown]);
	}
	for run in &dmb.grid.runs {
		lists.insert(run.contents);
	}
	lists.extend(dmb.extra_cells.iter().map(|&(_, contents)| contents));
	lists.insert(dmb.world.procs);
	lists
}

/// The proc tables DreamDaemon builds as it loads the world: every type has
/// an entry for each name a proc of its own or of an ancestor goes by. A
/// proc goes by its name and, when that differs, by its identifier too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProcTables {
	/// Entries under procs' names.
	pub names: u64,
	/// Entries under identifiers that are not the proc's name.
	pub second_names: u64
}

impl ProcTables {
	pub const fn bytes(&self) -> u64 {
		(self.names + self.second_names) * PROC_ENTRY_BYTES
	}

	pub fn count(dmb: &Dmb) -> Self {
		// The two names of each proc, as string ids; an identifier with no
		// string of its own gets an id past the table's end.
		let mut identifiers: HashMap<Vec<u8>, u32> = HashMap::new();
		let strings = Strings::new(dmb);
		let mut keys: Vec<Option<(u32, Option<u32>)>> = Vec::with_capacity(dmb.procs.len());
		for proc in &dmb.procs {
			let path = dmb.strings.get(proc.path as usize).filter(|_| proc.path != NONE);
			let Some(path) = path.and_then(|path| ProcPath::parse(path)) else {
				keys.push(None);
				continue;
			};
			let next = (dmb.strings.len() + identifiers.len()) as u32;
			let identifier = match strings.get(path.identifier) {
				Some(id) => id,
				None => *identifiers.entry(path.identifier.to_vec()).or_insert(next)
			};
			let name = if proc.name == NONE { identifier } else { proc.name };
			keys.push(Some((name, (name != identifier).then_some(identifier))));
		}
		let own: Vec<&[u32]> = dmb
			.types
			.iter()
			.map(|ty| {
				dmb.lists
					.get(ty.procs as usize)
					.filter(|_| ty.procs != NONE)
					.map_or(&[][..], Vec::as_slice)
			})
			.collect();
		let mut children: Vec<Vec<usize>> = vec![Vec::new(); dmb.types.len()];
		let mut roots = Vec::new();
		for (index, ty) in dmb.types.iter().enumerate() {
			match ty.parent as usize {
				parent if ty.parent != NONE && parent < dmb.types.len() && parent != index => children[parent].push(index),
				_ => roots.push(index)
			}
		}
		// Depth first: a type's new names go in on the way down and come out
		// on the way back.
		enum Step {
			Enter(usize),
			Leave(Vec<(u32, bool)>)
		}
		let mut tables = Self::default();
		let mut held: HashSet<u32> = HashSet::new();
		let mut second_held = 0u64;
		let mut stack: Vec<Step> = roots.into_iter().rev().map(Step::Enter).collect();
		while let Some(step) = stack.pop() {
			match step {
				Step::Enter(index) => {
					let mut added = Vec::new();
					let procs = || own[index].iter().filter_map(|&proc| keys.get(proc as usize).copied().flatten());
					for (name, _) in procs() {
						if held.insert(name) {
							added.push((name, false));
						}
					}
					for (_, identifier) in procs() {
						if let Some(identifier) = identifier.filter(|&identifier| held.insert(identifier)) {
							added.push((identifier, true));
							second_held += 1;
						}
					}
					tables.second_names += second_held;
					tables.names += held.len() as u64 - second_held;
					stack.push(Step::Leave(added));
					stack.extend(children[index].iter().rev().map(|&child| Step::Enter(child)));
				}
				Step::Leave(added) => {
					for (key, second) in added {
						held.remove(&key);
						second_held -= u64::from(second);
					}
				}
			}
		}
		tables
	}
}
