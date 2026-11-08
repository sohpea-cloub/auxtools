//! Hot reloading (`Request::HotReload`): new code for procs of the running
//! world, compiled by dmasm from edited source.
//!
//! Everything is checked and assembled before anything changes, so a reload
//! that can't be done in full leaves the world as it was. Then each proc gets
//! its new bytecode in a buffer of its own. Calls already running, sleeping
//! ones included, finish on the code they started with, which is never freed.
//!
//! What a reload can't do yet, and refuses:
//! - change a proc's parameters (BYOND keeps their names and types in a table
//!   shared between procs, and named arguments go by those names)
//! - add static vars, or use prefabs with var overrides (`/obj{a = 1}`)
//! - find a locals table with room for the new code's locals, when no proc in
//!   the world has that many

use std::collections::{BTreeSet, HashMap};

use auxtools::{raw_types, Proc, StringRef};
use dmasm::{Instruction, Node, Opcode};

use crate::{
	assemble_env::AssembleEnv,
	instruction_hooking::{hook_instruction, hooked_offsets, original_bytecode, unhook_instruction},
	server_types::{HotReloadLines, HotReloadProc, ProcRef}
};

/// A proc's new bytecode, checked and ready to swap in.
pub struct Prepared {
	proc: Proc,
	name: String,
	bytecode: Vec<u32>,
	/// A bigger locals table, when the new code has more locals than the
	/// proc's frames get now
	locals_table: Option<raw_types::misc::LocalsId>,
	/// The new code's local names, when they're new
	local_names: Option<Vec<String>>,
	/// How the proc's lines moved. `None` for new code, where a breakpoint
	/// stays on the line it was on.
	lines: Option<Vec<(u32, u32)>>
}

/// Where reloads moved breakpoints, by the offset the debug-client set them at,
/// so it can still remove them by that offset. An entry goes once the client
/// sets a breakpoint at the new offset itself, which is how it learns where a
/// breakpoint went.
pub type MovedBreakpoints = HashMap<(raw_types::procs::ProcId, u32), u32>;

fn describe(proc: &ProcRef) -> String {
	if proc.override_id == 0 {
		proc.path.clone()
	} else {
		format!("{} (override {})", proc.path, proc.override_id)
	}
}

fn find(proc: &ProcRef) -> Result<Proc, String> {
	Proc::find_override(&proc.path, proc.override_id).ok_or_else(|| format!("{}: there's no such proc in the running world", describe(proc)))
}

/// Checks and assembles everything in a reload. Errors mean none of it can be
/// applied.
pub fn prepare(procs: &[HotReloadProc], line_moves: &[HotReloadLines], warnings: &mut Vec<String>) -> Result<Vec<Prepared>, Vec<String>> {
	let mut prepared = vec![];
	let mut errors = vec![];
	let mut world_globals = WorldGlobals::default();

	for reload in procs {
		match prepare_proc(reload, &mut world_globals, warnings) {
			Ok(p) => prepared.push(p),
			Err(e) => errors.push(e)
		}
	}

	for moved in line_moves {
		let proc = match find(&moved.proc) {
			Ok(proc) => proc,
			Err(e) => {
				errors.push(e);
				continue;
			}
		};

		// A world built without DEBUG has no lines to move
		if let Some(bytecode) = remap_lines(&original_bytecode(&proc), &moved.lines) {
			prepared.push(Prepared {
				proc,
				name: describe(&moved.proc),
				bytecode,
				locals_table: None,
				local_names: None,
				lines: Some(moved.lines.clone())
			});
		}
	}

	if errors.is_empty() {
		Ok(prepared)
	} else {
		Err(errors)
	}
}

fn prepare_proc(reload: &HotReloadProc, world_globals: &mut WorldGlobals, warnings: &mut Vec<String>) -> Result<Prepared, String> {
	let name = describe(&reload.proc);
	let proc = find(&reload.proc)?;

	let params: Vec<String> = proc.parameter_names().into_iter().map(String::from).collect();
	if params != reload.params {
		return Err(format!(
			"{}: its parameters are ({}) in the running world and ({}) in the new code, and parameters can't change in a reload",
			name,
			params.join(", "),
			reload.params.join(", ")
		));
	}

	if proc.is_hooked() {
		warnings.push(format!("{}: auxtools has hooked it, so the hook runs instead of the new code", name));
	}

	let original = original_bytecode(&proc);
	let nodes = adapt_debug_info(&reload.nodes, &original);

	let mut env = ReloadEnv::new(&original, world_globals);
	let bytecode = match dmasm::assembler::assemble(&nodes, &mut env) {
		Ok(bytecode) => bytecode,
		Err(e) => return Err(format!("{}: {}", name, env.error.take().unwrap_or_else(|| describe_assemble_error(&e))))
	};

	if bytecode.len() > u16::MAX as usize {
		return Err(format!(
			"{}: the new code is {} words long, and BYOND's limit is {}",
			name,
			bytecode.len(),
			u16::MAX
		));
	}

	let locals_table = if reload.locals.len() > proc.locals_count() {
		Some(find_locals_table(reload.locals.len()).ok_or_else(|| {
			format!(
				"{}: the new code has {} locals, and no proc in the world has a locals table that big to borrow",
				name,
				reload.locals.len()
			)
		})?)
	} else {
		None
	};

	Ok(Prepared {
		proc,
		name,
		bytecode,
		locals_table,
		local_names: Some(reload.locals.clone()),
		lines: None
	})
}

/// Swaps everything in. Breakpoints in the procs come out first and go back in
/// on the same lines (where the lines moved, on the lines they moved to).
pub fn apply(
	prepared: Vec<Prepared>,
	conditions: &mut HashMap<(raw_types::procs::ProcId, u16), String>,
	moved_breakpoints: &mut MovedBreakpoints,
	warnings: &mut Vec<String>
) {
	for p in prepared {
		let (old_ptr, _) = unsafe { p.proc.bytecode_mut_ptr() };
		let old_bytecode = original_bytecode(&p.proc);

		// Frames still running the old code have the old locals. The proc's
		// table might not name them after this.
		if !auxtools::debug::has_local_names(old_ptr) {
			let names = auxtools::debug::local_names_for(&p.proc, old_ptr);
			auxtools::debug::set_local_names(old_ptr, names);
		}

		let breakpoints: Vec<(u32, u32, Option<String>)> = hooked_offsets(&p.proc)
			.into_iter()
			.filter_map(|offset| {
				let line = line_at(&old_bytecode, offset)?;
				let condition = conditions.remove(&(p.proc.id, offset as u16));
				let _ = unhook_instruction(&p.proc, offset);
				Some((offset, line, condition))
			})
			.collect();

		let local_names = p.local_names.unwrap_or_else(|| auxtools::debug::local_names_for(&p.proc, old_ptr));
		let new_ptr = p.proc.set_bytecode_unshared(p.bytecode.clone());
		if let Some(table) = p.locals_table {
			p.proc.set_locals_id(table);
		}
		auxtools::debug::set_local_names(new_ptr, local_names);

		for (old_offset, line, condition) in breakpoints {
			let line = match &p.lines {
				Some(lines) => map_line(line, lines),
				None => line
			};

			// The debug-client knows this breakpoint by the offset it set it
			// at, or one an earlier reload moved it from
			let known_as: Vec<(raw_types::procs::ProcId, u32)> = moved_breakpoints
				.iter()
				.filter(|((proc, _), at)| *proc == p.proc.id && **at == old_offset)
				.map(|(key, _)| *key)
				.collect();
			for key in &known_as {
				moved_breakpoints.remove(key);
			}

			let Some(offset) = offset_of_line(&p.bytecode, line) else {
				warnings.push(format!(
					"{}: the breakpoint on line {} was removed, because that line has no code now",
					p.name, line
				));
				continue;
			};

			if hook_instruction(&p.proc, offset).is_err() {
				warnings.push(format!("{}: couldn't put the breakpoint on line {} back", p.name, line));
				continue;
			}

			if let Some(condition) = condition {
				conditions.insert((p.proc.id, offset as u16), condition);
			}
			for key in known_as.into_iter().chain([(p.proc.id, old_offset)]) {
				if key.1 != offset {
					moved_breakpoints.insert(key, offset);
				}
			}
		}
	}
}

/// The new code's DbgFile and DbgLine, made to fit the running proc.
fn adapt_debug_info(nodes: &[Node], original: &[u32]) -> Vec<Node> {
	let is_debug_info = |node: &Node| matches!(node, Node::Instruction(Instruction::DbgFile(_) | Instruction::DbgLine(_), _));

	// DreamMaker only writes them for worlds built with DEBUG
	if original.first() != Some(&Opcode::DbgFile.word()) || original.len() < 2 {
		return nodes.iter().filter(|node| !is_debug_info(node)).cloned().collect();
	}

	// DreamMaker names a file in a nested #include by its #include string,
	// which can be shorter than its path from the .dme. Runtime errors and
	// breakpoints go by the name the proc has.
	let old_name = unsafe { StringRef::from_id(raw_types::strings::StringId(original[1])) }.data().to_vec();
	let mut suffix = b"/".to_vec();
	suffix.extend_from_slice(&old_name);

	nodes
		.iter()
		.map(|node| match node {
			Node::Instruction(Instruction::DbgFile(name), ()) if name.0 != old_name && name.0.ends_with(&suffix) => {
				Node::Instruction(Instruction::DbgFile(dmasm::DMString(old_name.clone())), ())
			}
			_ => node.clone()
		})
		.collect()
}

/// `bytecode` with its DbgLines renumbered, or `None` when none changes.
fn remap_lines(bytecode: &[u32], lines: &[(u32, u32)]) -> Option<Vec<u32>> {
	let scanned = dmasm::scan(bytecode).ok()?;
	let mut out = bytecode.to_vec();
	let mut changed = false;

	for ins in scanned.instructions {
		if ins.opcode == Opcode::DbgLine {
			let at = ins.offset as usize + 1;
			let line = map_line(out[at], lines);
			changed |= line != out[at];
			out[at] = line;
		}
	}

	changed.then_some(out)
}

/// Where `line` went. A line the map doesn't name moved as far as the closest
/// one above it that it does.
fn map_line(line: u32, lines: &[(u32, u32)]) -> u32 {
	let mut sorted = lines.to_vec();
	sorted.sort_unstable();

	let known = match sorted.binary_search_by_key(&line, |(old, _)| *old) {
		Ok(i) => return sorted[i].1,
		Err(0) => sorted.first(),
		Err(i) => sorted.get(i - 1)
	};

	match known {
		Some(&(old, new)) => (line as i64 + new as i64 - old as i64) as u32,
		None => line
	}
}

/// The line the instruction at `offset` is on.
fn line_at(bytecode: &[u32], offset: u32) -> Option<u32> {
	let mut line = None;
	for ins in dmasm::scan(bytecode).ok()?.instructions {
		if ins.offset > offset {
			break;
		}
		if ins.opcode == Opcode::DbgLine {
			line = ins.bytecode.get(1).copied();
		}
	}
	line
}

/// Where a breakpoint on `line` goes: the instruction after its DbgLine, as
/// the debug server's `Offset` request has it.
fn offset_of_line(bytecode: &[u32], line: u32) -> Option<u32> {
	let instructions = dmasm::scan(bytecode).ok()?.instructions;
	let at = instructions
		.iter()
		.position(|ins| ins.opcode == Opcode::DbgLine && ins.bytecode.get(1) == Some(&line))?;
	instructions.get(at + 1).map(|ins| ins.offset)
}

/// The smallest locals table in the world with room for `count` locals. Its
/// names don't matter: BYOND only takes the count from it, and the debugger
/// names the new code's locals itself.
fn find_locals_table(count: usize) -> Option<raw_types::misc::LocalsId> {
	let mut best: Option<(usize, raw_types::misc::LocalsId)> = None;
	let mut id = 0;
	while let Some(proc) = Proc::from_id(raw_types::procs::ProcId(id)) {
		let size = proc.locals_count();
		if size >= count && best.is_none_or(|(best_size, _)| size < best_size) {
			best = Some((size, proc.locals_id()));
		}
		id += 1;
	}
	best.map(|(_, table)| table)
}

fn describe_assemble_error(e: &dmasm::assembler::AssembleError) -> String {
	use dmasm::assembler::AssembleError::*;
	let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
	match e {
		ProcNotFound(path) => format!("there's no proc {} in the running world", path),
		StringNotFound(s) => format!("couldn't make the string {:?}", text(s)),
		VariableNameNotFound(name) => format!("there's no var {} in the running world (vars can't be added in a reload)", text(name)),
		TypeNotFound(path) => format!("there's no type {} in the running world (types can't be added in a reload)", path),
		ResourceNotFound(path) => format!("couldn't find the file {}", path),
		InstanceNotFound(path) => format!("prefabs with var overrides ({}{{...}}) can't be used in a reload yet", path),
		LabelNotFound(label) => format!("the compiled code jumps to a label it doesn't have ({})", label)
	}
}

/// Every variable id whose name is `name`.
fn variable_ids(name: &[u8]) -> Vec<u32> {
	let Ok(string) = StringRef::from_raw(name) else {
		return vec![];
	};
	let id = string.get_id();

	unsafe {
		let table = &*raw_types::funcs::VARIABLE_NAMES;
		(0..table.count).filter(|&i| *table.entries.add(i as usize) == id).collect()
	}
}

/// The global vars `bytecode` uses, by name, each name's ids in ascending
/// order. Same-named static locals of one proc are in the order they were
/// declared in, which is the order of their ids.
fn globals_in(bytecode: &[u32]) -> HashMap<Vec<u8>, Vec<u32>> {
	let mut out: HashMap<Vec<u8>, Vec<u32>> = HashMap::new();
	let (nodes, _) = dmasm::disassembler::disassemble(bytecode, &mut LenientDisassembleEnv);

	for node in &nodes {
		let Node::Instruction(ins, _) = node else { continue };
		for var in ins.variables() {
			for global in var.globals() {
				let Some(id) = global.id else { continue };
				let ids = out.entry(global.name.0.clone()).or_default();
				if let Err(at) = ids.binary_search(&id) {
					ids.insert(at, id);
				}
			}
		}
	}

	out
}

/// Which procs use each global var id, from all the world's bytecode. Only
/// built when a name is ambiguous, because it disassembles every proc.
#[derive(Default)]
struct WorldGlobals(Option<HashMap<Vec<u8>, Vec<(u32, String)>>>);

impl WorldGlobals {
	fn users(&mut self, name: &[u8]) -> &[(u32, String)] {
		let map = self.0.get_or_insert_with(|| {
			let mut map: HashMap<Vec<u8>, Vec<(u32, String)>> = HashMap::new();
			let mut id = 0;
			while let Some(proc) = Proc::from_id(raw_types::procs::ProcId(id)) {
				for (name, ids) in globals_in(&original_bytecode(&proc)) {
					map.entry(name).or_default().extend(ids.into_iter().map(|id| (id, proc.path.clone())));
				}
				id += 1;
			}
			map
		});
		map.get(name).map_or(&[], |users| users.as_slice())
	}
}

/// What the assembler needs for a proc that already exists: the debugger's
/// lookups, plus the global var slots the proc already uses.
struct ReloadEnv<'a> {
	proc_globals: HashMap<Vec<u8>, Vec<u32>>,
	world_globals: &'a mut WorldGlobals,
	/// Why a lookup failed, when that says more than the assembler's error
	error: Option<String>
}

impl<'a> ReloadEnv<'a> {
	fn new(original: &[u32], world_globals: &'a mut WorldGlobals) -> Self {
		Self {
			proc_globals: globals_in(original),
			world_globals,
			error: None
		}
	}
}

impl dmasm::assembler::AssembleEnv for ReloadEnv<'_> {
	fn get_string_index(&mut self, string: &[u8]) -> Option<u32> {
		AssembleEnv.get_string_index(string)
	}

	fn get_variable_name_index(&mut self, name: &[u8]) -> Option<u32> {
		AssembleEnv.get_variable_name_index(name)
	}

	fn get_proc_index(&mut self, path: &str) -> Option<u32> {
		AssembleEnv.get_proc_index(path)
	}

	fn get_type(&mut self, path: &str) -> Option<(u8, u32)> {
		AssembleEnv.get_type(path)
	}

	fn get_resource_index(&mut self, path: &str) -> Option<u32> {
		AssembleEnv.get_resource_index(path)
	}

	fn get_global_index(&mut self, name: &[u8], scope: Option<&str>) -> Option<u32> {
		let shown = String::from_utf8_lossy(name).into_owned();

		// A static local (`proc:PATH#N`, the Nth of that name in the proc):
		// the proc's own, which its current code already uses
		if let Some(scope) = scope.filter(|s| s.starts_with("proc:")) {
			let n = scope.rsplit_once('#').and_then(|(_, n)| n.parse::<usize>().ok()).unwrap_or(0);
			let id = self.proc_globals.get(name).and_then(|ids| ids.get(n)).copied();
			if id.is_none() {
				self.error = Some(format!("static local {} is new (vars can't be added in a reload)", shown));
			}
			return id;
		}

		let candidates = variable_ids(name);
		match candidates.len() {
			0 => {
				self.error = Some(format!(
					"there's no global var {} in the running world (vars can't be added in a reload)",
					shown
				));
				return None;
			}
			1 => return Some(candidates[0]),
			_ => {}
		}

		// Several vars have this name. One the proc already uses is the one.
		if let Some([id]) = self.proc_globals.get(name).map(Vec::as_slice) {
			return Some(*id);
		}

		// Otherwise go by what other code uses: there may be only one global
		// of that name, or only one that the static var's type uses
		let users = self.world_globals.users(name);
		let ids: BTreeSet<u32> = users.iter().map(|(id, _)| *id).collect();
		if ids.len() == 1 {
			return ids.first().copied();
		}
		if let Some(ty) = scope.filter(|s| !s.is_empty()) {
			let prefix = format!("{}/", ty);
			let ids: BTreeSet<u32> = users.iter().filter(|(_, path)| path.starts_with(&prefix)).map(|(id, _)| *id).collect();
			if ids.len() == 1 {
				return ids.first().copied();
			}
		}

		self.error = Some(format!(
			"couldn't tell which of the world's {} vars called {} the code means; another proc of {} that uses it would settle it",
			candidates.len(),
			shown,
			scope.filter(|s| !s.is_empty()).unwrap_or("the world")
		));
		None
	}
}

/// Disassembles any proc to the end: a value it can't put a name to comes
/// out as a raw value instead of ending the disassembly.
struct LenientDisassembleEnv;

impl dmasm::disassembler::DisassembleEnv for LenientDisassembleEnv {
	fn get_string_data(&mut self, index: u32) -> Option<Vec<u8>> {
		crate::DisassembleEnv.get_string_data(index)
	}

	fn get_variable_name(&mut self, index: u32) -> Option<Vec<u8>> {
		crate::DisassembleEnv.get_variable_name(index)
	}

	fn get_proc_name(&mut self, index: u32) -> Option<String> {
		crate::DisassembleEnv.get_proc_name(index).or_else(|| Some(format!("<proc {}>", index)))
	}

	fn value_to_string_data(&mut self, tag: u32, data: u32) -> Option<Vec<u8>> {
		crate::DisassembleEnv
			.value_to_string_data(tag, data)
			.or_else(|| Some(format!("<value {:#x} {}>", tag, data).into_bytes()))
	}
}

#[cfg(test)]
mod tests {
	use super::{map_line, remap_lines};

	#[test]
	fn lines_move_with_the_closest_known_line_above() {
		let lines = [(10, 12), (14, 20)];
		assert_eq!(map_line(10, &lines), 12);
		assert_eq!(map_line(14, &lines), 20);
		assert_eq!(map_line(11, &lines), 13);
		assert_eq!(map_line(16, &lines), 22);
		assert_eq!(map_line(5, &lines), 7);
		assert_eq!(map_line(5, &[]), 5);
	}

	#[test]
	fn renumbers_dbgline_operands_only() {
		// DbgLine 10; PushInt 10; DbgLine 11; Ret
		let dbgline = dmasm::Opcode::DbgLine.word();
		let push_int = dmasm::Opcode::PushInt.word();
		let ret = dmasm::Opcode::Ret.word();
		let bytecode = [dbgline, 10, push_int, 10, dbgline, 11, ret];
		let moved = remap_lines(&bytecode, &[(10, 15), (11, 16)]).unwrap();
		assert_eq!(moved, [dbgline, 15, push_int, 10, dbgline, 16, ret]);
		assert_eq!(remap_lines(&bytecode, &[(10, 10), (11, 11)]), None);
	}
}
