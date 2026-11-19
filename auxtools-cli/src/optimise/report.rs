//! What a world's types cost DreamDaemon before anything is made.
//!
//! Every type has a proc table with an entry of 12 bytes for each name of
//! each proc it has, inherited ones included. A type that sets or declares
//! a var has a var table with an entry of 16 bytes for each var it has,
//! inherited ones included; a type that does neither shares its parent's.
//! So a proc or a var costs its type and the types below it, and the ones
//! declared near the root of a large tree cost the most.

use std::collections::{HashMap, HashSet};

use dmasm::{
	dmb::{Dmb, NONE},
	ReferenceKind
};

use super::{below, own_proc, parents, ProcPath, ProcTables, Strings, Uses, PROC_ENTRY_BYTES};

/// Bytes of a type's var table that one var takes.
pub const VAR_ENTRY_BYTES: u64 = 16;
/// How many types and procs each list of the report names.
const LISTED: usize = 10;

/// What a type's own procs or vars cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cost {
	pub path: String,
	/// The table entries the type adds: names of procs, or vars.
	pub entries: u64,
	/// The types with a table that holds them: the type and those below.
	pub types: u64,
	pub bytes: u64
}

#[derive(Debug, Default)]
pub struct Report {
	pub types: usize,
	pub procs: usize,
	pub strings: usize,
	pub proc_tables: ProcTables,
	/// The types whose procs cost most, most first.
	pub proc_costs: Vec<Cost>,
	/// The types with a var table, and the entries of them all.
	pub var_tables: u64,
	pub var_entries: u64,
	/// The types whose vars cost most, most first.
	pub var_costs: Vec<Cost>,
	/// Procs declared on types that no call, text or value names, with the
	/// bytes of proc table each takes, most first.
	pub unused: Vec<(String, u64)>
}

fn text(dmb: &Dmb, string: u32) -> String {
	String::from_utf8_lossy(dmb.strings.get(string as usize).map_or(&[][..], Vec::as_slice)).into_owned()
}

fn costliest(mut costs: Vec<Cost>) -> Vec<Cost> {
	costs.retain(|cost| cost.bytes > 0);
	costs.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
	costs.truncate(LISTED);
	costs
}

/// The procs declared on types that nothing names: no call by their name
/// or identifier or by id, no text that `call()` could find them by, and
/// no use of their paths. A verb, or a proc with verb settings, is used by
/// players.
fn unused(dmb: &Dmb, types_below: &[u64]) -> Vec<(String, u64)> {
	let strings = Strings::new(dmb);
	let uses = Uses::new(dmb);
	let mut named: HashSet<u32> = HashSet::new();
	let mut called: HashSet<u32> = HashSet::new();
	for &list in &uses.code {
		for reference in uses.references[&list].iter().flatten() {
			match reference.kind {
				ReferenceKind::ProcName => named.insert(reference.id),
				ReferenceKind::Proc => called.insert(reference.id),
				_ => false
			};
		}
	}
	let mut owner: HashMap<u32, usize> = HashMap::new();
	let mut verbs: HashSet<u32> = HashSet::new();
	for (index, ty) in dmb.types.iter().enumerate() {
		if let Some(procs) = dmb.lists.get(ty.procs as usize).filter(|_| ty.procs != NONE) {
			owner.extend(procs.iter().map(|&proc| (proc, index)));
		}
		if let Some(list) = dmb.lists.get(ty.verbs as usize).filter(|_| ty.verbs != NONE) {
			verbs.extend(list.iter().copied());
		}
	}
	// A proc is used if any proc of its identifier is: a call by name
	// reaches whichever the object has
	let mut identifiers: HashMap<&[u8], (bool, Vec<u32>)> = HashMap::new();
	for (proc, record) in dmb.procs.iter().enumerate() {
		let proc = proc as u32;
		let Some(path_text) = dmb.strings.get(record.path as usize).filter(|_| record.path != NONE) else {
			continue;
		};
		let Some(path) = ProcPath::parse(path_text) else {
			continue;
		};
		let names = [Some(record.name).filter(|&name| name != NONE), strings.get(path.identifier)];
		let used = !uses.unread_words.is_empty()
			|| path.verb
			|| verbs.contains(&proc)
			|| record.desc != NONE
			|| record.category != NONE
			|| path.identifier.starts_with(b"operator")
			|| called.contains(&proc)
			|| uses.proc_values.contains(&proc)
			|| names.iter().flatten().any(|name| named.contains(name) || uses.text.contains(name));
		let entry = identifiers.entry(path.identifier).or_default();
		entry.0 |= used;
		// Declarations on types: an override alone adds no entry. BYOND's
		// own procs are no one's to remove.
		if !path.global() && !own_proc(&path) && path_text[path.owner.len()..].starts_with(b"/proc/") {
			entry.1.push(proc);
		}
	}
	let mut found: Vec<(String, u64)> = identifiers
		.into_values()
		.filter(|(used, _)| !used)
		.flat_map(|(_, procs)| procs)
		.filter_map(|proc| {
			let record = &dmb.procs[proc as usize];
			let path = dmb.strings.get(record.path as usize)?;
			let second = ProcPath::parse(path).is_some_and(|path| strings.get(path.identifier) != Some(record.name));
			let bytes = types_below[*owner.get(&proc)?] * PROC_ENTRY_BYTES * (1 + u64::from(second));
			Some((String::from_utf8_lossy(path).into_owned(), bytes))
		})
		.collect();
	found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
	found
}

pub fn run(dmb: &Dmb) -> Report {
	let parents = parents(dmb);
	let types_below = below(&parents, &vec![1; dmb.types.len()]);
	let introduced = ProcTables::introduced(dmb);
	let proc_costs = dmb
		.types
		.iter()
		.enumerate()
		.map(|(index, ty)| {
			let entries = u64::from(introduced[index].0 + introduced[index].1);
			Cost {
				path: text(dmb, ty.path),
				entries,
				types: types_below[index],
				bytes: entries * types_below[index] * PROC_ENTRY_BYTES
			}
		})
		.collect();

	// A var's entry is in the table of each type at or below its own that
	// has a table
	let has_table: Vec<u64> = dmb
		.types
		.iter()
		.map(|ty| u64::from(ty.set_vars != NONE || ty.declared_vars != NONE))
		.collect();
	let tables_below = below(&parents, &has_table);
	let declared = |index: usize| {
		let ty = &dmb.types[index];
		// (pairs of a var and its flags)
		dmb.lists
			.get(ty.declared_vars as usize)
			.filter(|_| ty.declared_vars != NONE)
			.map_or(0, |list| list.len() as u64 / 2)
	};
	let var_costs: Vec<Cost> = (0..dmb.types.len())
		.map(|index| Cost {
			path: text(dmb, dmb.types[index].path),
			entries: declared(index),
			types: tables_below[index],
			bytes: declared(index) * tables_below[index] * VAR_ENTRY_BYTES
		})
		.collect();

	Report {
		types: dmb.types.len(),
		procs: dmb.procs.len(),
		strings: dmb.strings.len(),
		proc_tables: ProcTables::count(dmb),
		proc_costs: costliest(proc_costs),
		var_tables: has_table.iter().sum(),
		var_entries: var_costs.iter().map(|cost| cost.entries * cost.types).sum(),
		var_costs: costliest(var_costs),
		unused: unused(dmb, &types_below)
	}
}

#[cfg(test)]
mod tests {
	use dmasm::dmb::{Dmb, Proc, Type, NONE};

	use super::{run, Cost};

	fn string(dmb: &mut Dmb, text: &str) -> u32 {
		if let Some(id) = dmb.strings.iter().position(|string| string == text.as_bytes()) {
			return id as u32;
		}
		dmb.strings.push(text.as_bytes().to_vec());
		dmb.strings.len() as u32 - 1
	}

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

	fn ty(dmb: &mut Dmb, path: &str, parent: u32, procs: Vec<u32>, vars: u32) -> u32 {
		let path = string(dmb, path);
		let mut list = |list: Vec<u32>| {
			if list.is_empty() {
				return NONE;
			}
			dmb.lists.push(list);
			dmb.lists.len() as u32 - 1
		};
		let (procs, declared_vars) = (list(procs), list((0..vars).flat_map(|var| [var, 0]).collect()));
		dmb.types.push(Type {
			path,
			parent,
			procs,
			declared_vars,
			verbs: NONE,
			set_vars: NONE,
			builtin_vars: NONE,
			unknown: NONE,
			..Type::default()
		});
		dmb.types.len() as u32 - 1
	}

	#[test]
	fn a_proc_and_a_var_cost_their_type_and_the_types_below() {
		let mut dmb = Dmb::default();
		// /datum/a has two procs, one with a second name, and three vars;
		// two types below it, one with a var table of its own
		let called = string(&mut dmb, "do thing");
		let do_thing = proc(&mut dmb, "/datum/a/proc/do_thing", "do thing", vec![0x00]);
		let unused = proc(&mut dmb, "/datum/a/proc/unused", "unused", vec![0x00]);
		let over = proc(&mut dmb, "/datum/a/b/unused", "unused", vec![0x00]);
		let own = proc(&mut dmb, "/datum/a/c/proc/own", "own", vec![0x29, 0xFFDD, called, 0, 0x00]);
		let a = ty(&mut dmb, "/datum/a", NONE, vec![do_thing, unused], 3);
		ty(&mut dmb, "/datum/a/b", a, vec![over], 0);
		ty(&mut dmb, "/datum/a/c", a, vec![own], 1);

		let report = run(&dmb);
		assert_eq!((report.types, report.procs), (3, 4));
		// do_thing, "do thing" and unused in three tables, own in one
		assert_eq!((report.proc_tables.names, report.proc_tables.second_names), (7, 3));
		assert_eq!(report.proc_costs, [
			Cost {
				path: "/datum/a".to_owned(),
				entries: 3,
				types: 3,
				bytes: 108
			},
			Cost {
				path: "/datum/a/c".to_owned(),
				entries: 1,
				types: 1,
				bytes: 12
			}
		]);
		// /datum/a's three vars in two tables, /datum/a/c's one in its own
		assert_eq!((report.var_tables, report.var_entries), (2, 7));
		assert_eq!(report.var_costs[0], Cost {
			path: "/datum/a".to_owned(),
			entries: 3,
			types: 2,
			bytes: 96
		});
		// own calls do_thing by name; nothing names unused or own
		assert_eq!(report.unused, [
			("/datum/a/proc/unused".to_owned(), 36),
			("/datum/a/c/proc/own".to_owned(), 12)
		]);
	}
}
