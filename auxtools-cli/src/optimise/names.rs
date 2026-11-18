//! Name procs after their identifiers.
//!
//! DreamMaker names a proc after its identifier with spaces for underscores
//! (`attack_hand` is "attack hand"), and DreamDaemon lists a proc in the
//! table of every type that has it under its name and, when the identifier
//! differs, under that as well. Naming the proc `attack_hand`, as
//! `set name = "attack_hand"` does in source, halves what it costs.
//!
//! Compiled calls look a proc up by its name, so every proc of an identifier
//! is renamed, with the calls, or none is. What changes for the world:
//! runtime errors and call stacks print `attack_hand` where they printed
//! `attack hand`.

use std::collections::{BTreeMap, HashMap, HashSet};

use dmasm::{
	dmb::{Dmb, Proc, NONE},
	ReferenceKind
};

use super::{lists_not_code, ProcPath, ProcTables, Strings, Uses};

/// Why an identifier's procs keep their names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kept {
	/// One of them is a verb or has verb settings: players see its name.
	Verb,
	/// One of them was given a name of its own.
	Named,
	/// The path of one is used as a value, which can make it a verb.
	Value,
	/// The name with spaces is also text in the code, which `call()` can
	/// look a proc up by.
	Text,
	/// A proc with another identifier goes by one of the two names.
	Shared,
	/// An operator: the engine looks it up by a name of its own choosing.
	Operator,
	/// Bytecode that could not be read uses the name's string.
	Unread,
	/// The world's 16-bit string ids are all taken.
	Full
}

impl Kept {
	pub const ALL: [Self; 8] = [
		Self::Verb,
		Self::Named,
		Self::Value,
		Self::Text,
		Self::Shared,
		Self::Operator,
		Self::Unread,
		Self::Full
	];

	pub const fn key(self) -> &'static str {
		match self {
			Self::Verb => "verb",
			Self::Named => "named",
			Self::Value => "value",
			Self::Text => "text",
			Self::Shared => "shared",
			Self::Operator => "operator",
			Self::Unread => "unread",
			Self::Full => "full"
		}
	}

	pub const fn reason(self) -> &'static str {
		match self {
			Self::Verb => "a verb or a proc with verb settings",
			Self::Named => "a proc with a name of its own",
			Self::Value => "a proc whose path is used as a value",
			Self::Text => "a name that is also text in the code",
			Self::Shared => "a name another proc goes by",
			Self::Operator => "an operator's name",
			Self::Unread => "a name in bytecode that could not be read",
			Self::Full => "no string id left"
		}
	}
}

#[derive(Debug, Default)]
pub struct Report {
	/// Identifiers whose procs were renamed, the procs, and the calls
	/// changed to the new names.
	pub identifiers: usize,
	pub procs: usize,
	pub calls: usize,
	/// Identifiers whose procs keep the names DreamMaker gave them, by
	/// reason.
	pub kept: BTreeMap<Kept, Vec<String>>,
	/// The proc tables before and after.
	pub before: ProcTables,
	pub after: ProcTables
}

/// The name DreamMaker gives a proc with this identifier.
fn default_name(identifier: &[u8]) -> Vec<u8> {
	identifier.iter().map(|&byte| if byte == b'_' { b' ' } else { byte }).collect()
}

/// Whether the proc has only the settings of a proc that was never meant to
/// be a verb: no description, category, `set src`, `hidden` or
/// `invisibility`. `waitfor` and `background` are fine.
fn plain(proc: &Proc) -> bool {
	const WAITFOR: u32 = 0x04;
	const EXTENDED: u32 = 0x80;
	const BACKGROUND: u32 = 0x100;
	let flags = match proc.flags_ex {
		Some((flags, invisibility)) => invisibility == 0 && flags & !(WAITFOR | EXTENDED | BACKGROUND) == 0,
		None => u32::from(proc.flags) & !WAITFOR == 0
	};
	flags && proc.desc == NONE && proc.category == NONE && (proc.src_range, proc.src_kind) == (0xFF, 0)
}

struct Group {
	/// The procs on types that have this identifier.
	procs: Vec<u32>,
	kept: Option<Kept>
}

pub fn run(dmb: &mut Dmb) -> Report {
	let mut report = Report {
		before: ProcTables::count(dmb),
		..Report::default()
	};
	let mut strings = Strings::new(dmb);
	let uses = Uses::new(dmb);
	let mut verbs: HashSet<u32> = HashSet::new();
	for ty in &dmb.types {
		if let Some(list) = dmb.lists.get(ty.verbs as usize).filter(|_| ty.verbs != NONE) {
			verbs.extend(list.iter().copied());
		}
	}

	// The procs on types by identifier, and the names each identifier's
	// procs go by, global procs included.
	let mut groups: BTreeMap<Vec<u8>, Group> = BTreeMap::new();
	let mut names: HashMap<u32, HashSet<&[u8]>> = HashMap::new();
	for (id, proc) in dmb.procs.iter().enumerate() {
		let path = dmb.strings.get(proc.path as usize).filter(|_| proc.path != NONE);
		let Some(path) = path.and_then(|path| ProcPath::parse(path)) else {
			continue;
		};
		if proc.name != NONE {
			names.entry(proc.name).or_default().insert(path.identifier);
		}
		// A global proc is called by its id and is in no type's table.
		if path.global() || !path.identifier.contains(&b'_') {
			continue;
		}
		let group = groups.entry(path.identifier.to_vec()).or_insert_with(|| Group {
			procs: Vec::new(),
			kept: None
		});
		group.procs.push(id as u32);
		let default = strings.get(&default_name(path.identifier));
		let own = strings.get(path.identifier);
		let kept = if path.identifier.starts_with(b"operator") {
			Some(Kept::Operator)
		} else if path.verb || verbs.contains(&(id as u32)) || !plain(proc) {
			Some(Kept::Verb)
		} else if proc.name == NONE || (Some(proc.name) != default && Some(proc.name) != own) {
			Some(Kept::Named)
		} else if uses.proc_values.contains(&(id as u32)) {
			Some(Kept::Value)
		} else {
			None
		};
		group.kept = group.kept.max(kept);
	}

	// The calls by each name: the list and the word that holds the name.
	let mut calls: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
	for &list in &uses.code {
		let Some(references) = &uses.references[&list] else {
			continue;
		};
		for reference in references.iter().filter(|reference| reference.kind == ReferenceKind::ProcName) {
			let name = dmb.lists[list as usize][reference.word_offset as usize];
			calls.entry(name).or_default().push((list, reference.word_offset));
		}
	}

	let shared = |name: Option<u32>, identifier: &[u8]| {
		name.and_then(|name| names.get(&name))
			.is_some_and(|identifiers| identifiers.iter().any(|other| *other != identifier))
	};
	let mut renames: Vec<(Vec<u8>, u32, Vec<u32>)> = Vec::new();
	for (identifier, group) in &mut groups {
		let default = strings.get(&default_name(identifier));
		let own = strings.get(identifier);
		// Nothing to do where no proc has the name DreamMaker gives
		let Some(default) = default.filter(|default| group.procs.iter().any(|&proc| dmb.procs[proc as usize].name == *default)) else {
			continue;
		};
		if group.kept.is_none() {
			group.kept = if shared(Some(default), identifier) || shared(own, identifier) {
				Some(Kept::Shared)
			} else if uses.text.contains(&default) {
				Some(Kept::Text)
			} else if uses.unread_words.contains(&default) {
				Some(Kept::Unread)
			} else {
				None
			};
		}
		match group.kept {
			Some(kept) => report
				.kept
				.entry(kept)
				.or_default()
				.push(String::from_utf8_lossy(identifier).into_owned()),
			None => renames.push((identifier.clone(), default, group.procs.clone()))
		}
	}

	let not_code = lists_not_code(dmb);
	let mut moved: HashMap<u32, u32> = HashMap::new();
	for (identifier, default, procs) in renames {
		let Some(name) = strings.intern(dmb, &identifier) else {
			report
				.kept
				.entry(Kept::Full)
				.or_default()
				.push(String::from_utf8_lossy(&identifier).into_owned());
			continue;
		};
		report.identifiers += 1;
		for proc in procs {
			if dmb.procs[proc as usize].name != name {
				dmb.procs[proc as usize].name = name;
				report.procs += 1;
			}
		}
		for &(list, word) in calls.get(&default).map_or(&[][..], Vec::as_slice) {
			// A list something else uses too is left as it is: the bytecode
			// moves to a copy.
			let list = if not_code.contains(&list) {
				*moved.entry(list).or_insert_with(|| {
					dmb.lists.push(dmb.lists[list as usize].clone());
					dmb.lists.len() as u32 - 1
				})
			} else {
				list
			};
			dmb.lists[list as usize][word as usize] = name;
			report.calls += 1;
		}
	}
	if !moved.is_empty() {
		for proc in &mut dmb.procs {
			if let Some(&copy) = moved.get(&proc.code) {
				proc.code = copy;
			}
		}
	}
	report.after = ProcTables::count(dmb);
	report
}

#[cfg(test)]
mod tests {
	use dmasm::dmb::{Dmb, Proc, Type, Var, NONE};

	use super::{run, Kept};

	/// `x.NAME()`, then the end of the proc.
	fn call(name: u32) -> Vec<u32> {
		vec![0x29, 0xFFDD, name, 0, 0x00]
	}

	fn string(dmb: &mut Dmb, text: &str) -> u32 {
		if let Some(id) = dmb.strings.iter().position(|string| string == text.as_bytes()) {
			return id as u32;
		}
		dmb.strings.push(text.as_bytes().to_vec());
		dmb.strings.len() as u32 - 1
	}

	/// A proc as DreamMaker writes one with no settings, named `name`.
	fn proc(dmb: &mut Dmb, path: &str, name: &str, code: Vec<u32>) -> u32 {
		let (path, name) = (string(dmb, path), string(dmb, name));
		dmb.lists.push(code);
		dmb.procs.push(Proc {
			path,
			name,
			desc: NONE,
			category: NONE,
			src_range: 0xFF,
			src_kind: 0,
			flags: 0x04,
			flags_ex: None,
			code: dmb.lists.len() as u32 - 1,
			locals: NONE,
			params: NONE
		});
		dmb.procs.len() as u32 - 1
	}

	fn ty(dmb: &mut Dmb, path: &str, procs: Vec<u32>, verbs: Vec<u32>) {
		let path = string(dmb, path);
		let mut list = |list: Vec<u32>| {
			if list.is_empty() {
				return NONE;
			}
			dmb.lists.push(list);
			dmb.lists.len() as u32 - 1
		};
		let (procs, verbs) = (list(procs), list(verbs));
		dmb.types.push(Type {
			path,
			parent: NONE,
			procs,
			verbs,
			set_vars: NONE,
			declared_vars: NONE,
			builtin_vars: NONE,
			unknown: NONE,
			..Type::default()
		});
	}

	/// `/datum/a/proc/do_thing`, `/datum/b/proc/do_thing` and a global proc
	/// that calls it, as DreamMaker names them.
	fn world() -> (Dmb, [u32; 3]) {
		let mut dmb = Dmb::default();
		let name = string(&mut dmb, "do thing");
		let a = proc(&mut dmb, "/datum/a/proc/do_thing", "do thing", vec![0x00]);
		let b = proc(&mut dmb, "/datum/b/proc/do_thing", "do thing", vec![0x12, 0x00]);
		let caller = proc(&mut dmb, "/proc/call_it", "call it", call(name));
		ty(&mut dmb, "/datum/a", vec![a], vec![]);
		ty(&mut dmb, "/datum/b", vec![b], vec![]);
		(dmb, [a, b, caller])
	}

	fn name(dmb: &Dmb, proc: u32) -> &str {
		std::str::from_utf8(&dmb.strings[dmb.procs[proc as usize].name as usize]).unwrap()
	}

	/// The name the proc's first instruction calls by.
	fn called(dmb: &Dmb, proc: u32) -> &str {
		let code = &dmb.lists[dmb.procs[proc as usize].code as usize];
		std::str::from_utf8(&dmb.strings[code[2] as usize]).unwrap()
	}

	fn kept(dmb: &mut Dmb) -> Vec<(Kept, Vec<String>)> {
		let before = dmb.clone();
		let report = run(dmb);
		assert_eq!((report.identifiers, report.procs, report.calls), (0, 0, 0));
		assert!(*dmb == before, "a world with nothing to rename was changed");
		report.kept.into_iter().collect()
	}

	#[test]
	fn every_proc_of_an_identifier_is_renamed_with_its_calls() {
		let (mut dmb, [a, b, caller]) = world();
		let report = run(&mut dmb);
		assert_eq!((report.identifiers, report.procs, report.calls), (1, 2, 1));
		assert!(report.kept.is_empty());
		assert_eq!([name(&dmb, a), name(&dmb, b), called(&dmb, caller)], ["do_thing"; 3]);
		// A global proc is called by its id: it keeps its name
		assert_eq!(name(&dmb, caller), "call it");
		assert_eq!((report.before.second_names, report.after.second_names), (2, 0));
		assert_eq!((report.before.names, report.after.names), (2, 2));

		let again = run(&mut dmb.clone());
		assert_eq!((again.identifiers, again.procs, again.calls), (0, 0, 0));
	}

	#[test]
	fn a_proc_already_named_after_its_identifier_is_joined_by_the_rest() {
		let (mut dmb, [a, b, caller]) = world();
		dmb.procs[a as usize].name = string(&mut dmb, "do_thing");
		let report = run(&mut dmb);
		assert_eq!((report.identifiers, report.procs, report.calls), (1, 1, 1));
		assert_eq!([name(&dmb, a), name(&dmb, b), called(&dmb, caller)], ["do_thing"; 3]);
	}

	#[test]
	fn an_identifier_keeps_its_names_when_one_proc_cannot_be_renamed() {
		let identifier = || vec!["do_thing".to_owned()];
		// A verb, by its type's list or by its settings
		let (mut dmb, [a, ..]) = world();
		ty(&mut dmb, "/datum/c", vec![], vec![a]);
		assert_eq!(kept(&mut dmb), [(Kept::Verb, identifier())]);
		let (mut dmb, [_, b, _]) = world();
		dmb.procs[b as usize].category = string(&mut dmb, "Things");
		assert_eq!(kept(&mut dmb), [(Kept::Verb, identifier())]);
		let (mut dmb, [_, b, _]) = world();
		dmb.procs[b as usize].flags = 0x05;
		assert_eq!(kept(&mut dmb), [(Kept::Verb, identifier())]);
		// `set background = 1` is no verb setting
		let (mut dmb, [_, b, _]) = world();
		dmb.procs[b as usize].flags = 0x80;
		dmb.procs[b as usize].flags_ex = Some((0x184, 0));
		assert_eq!(run(&mut dmb).procs, 2);

		// A name of its own
		let (mut dmb, [a, ..]) = world();
		dmb.procs[a as usize].name = string(&mut dmb, "Do It");
		assert_eq!(kept(&mut dmb), [(Kept::Named, identifier())]);

		// Its path as a value: in code, or what a var starts as
		let (mut dmb, [a, ..]) = world();
		proc(&mut dmb, "/proc/takes_path", "takes path", vec![0x60, 0x26, a, 0x00]);
		assert_eq!(kept(&mut dmb), [(Kept::Value, identifier())]);
		let (mut dmb, [a, ..]) = world();
		dmb.vars.push(Var { tag: 0x26, data: a, name: 0 });
		assert_eq!(kept(&mut dmb), [(Kept::Value, identifier())]);

		// The name with spaces as text
		let (mut dmb, _) = world();
		let text = string(&mut dmb, "do thing");
		proc(&mut dmb, "/proc/says", "says", vec![0x60, 0x06, text, 0x00]);
		assert_eq!(kept(&mut dmb), [(Kept::Text, identifier())]);

		// Another proc by either name
		let (mut dmb, _) = world();
		let other = proc(&mut dmb, "/datum/a/proc/dothing", "do thing", vec![0x00]);
		ty(&mut dmb, "/datum/d", vec![other], vec![]);
		assert_eq!(kept(&mut dmb), [(Kept::Shared, identifier())]);
		let (mut dmb, _) = world();
		proc(&mut dmb, "/datum/a/proc/other", "do_thing", vec![0x00]);
		assert_eq!(kept(&mut dmb), [(Kept::Shared, identifier())]);

		// An operator
		let mut dmb = Dmb::default();
		let turn = proc(&mut dmb, "/datum/a/proc/operator_turn", "operator turn", vec![0x00]);
		ty(&mut dmb, "/datum/a", vec![turn], vec![]);
		assert_eq!(kept(&mut dmb), [(Kept::Operator, vec!["operator_turn".to_owned()])]);

		// Bytecode that cannot be read and holds the name's id
		let (mut dmb, _) = world();
		let name = string(&mut dmb, "do thing");
		proc(&mut dmb, "/proc/unknown", "unknown", vec![0xFFFF_0000, name]);
		assert_eq!(kept(&mut dmb), [(Kept::Unread, identifier())]);
	}

	#[test]
	fn bytecode_in_a_list_another_table_uses_is_changed_in_a_copy() {
		let (mut dmb, [_, _, caller]) = world();
		// The caller's bytecode is also some proc's list of locals
		let shared = dmb.procs[caller as usize].code;
		dmb.procs[0].locals = shared;
		let before = dmb.lists[shared as usize].clone();
		let report = run(&mut dmb);
		assert_eq!(report.calls, 1);
		assert_eq!(dmb.lists[shared as usize], before);
		assert_ne!(dmb.procs[caller as usize].code, shared);
		assert_eq!(called(&dmb, caller), "do_thing");
	}

	#[test]
	fn a_world_out_of_string_ids_keeps_its_names() {
		let (mut dmb, _) = world();
		while dmb.strings.len() < 0xFFFF {
			dmb.strings.push(format!("filler {}", dmb.strings.len()).into_bytes());
		}
		assert_eq!(kept(&mut dmb), [(Kept::Full, vec!["do_thing".to_owned()])]);

		// With 32-bit ids there is room, past the id that means "none"
		let (mut dmb, [a, _, caller]) = world();
		dmb.flags |= Dmb::LARGE_IDS;
		while dmb.strings.len() < 0xFFFF {
			dmb.strings.push(format!("filler {}", dmb.strings.len()).into_bytes());
		}
		assert_eq!(run(&mut dmb).procs, 2);
		assert_eq!(dmb.procs[a as usize].name, 0x1_0000);
		assert_eq!([name(&dmb, a), called(&dmb, caller)], ["do_thing"; 2]);
		assert_eq!(dmb.strings[0xFFFF], b"");
	}
}
