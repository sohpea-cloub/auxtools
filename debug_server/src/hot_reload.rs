use std::{
	borrow::Cow,
	collections::{btree_map::Entry, BTreeMap, BTreeSet, HashMap, HashSet},
	rc::Rc,
	sync::Mutex
};

use auxtools::{
	debug, raw_types,
	runtime_tables::{self, type_fields, ProcEntryWords, NONE},
	shutdown, Proc, StringRef
};
use auxtools_protocol::{HotReloadRequest, LocalReads, ParsedReads, Reloaded, ShadowingProc};
use dmasm::{
	assembler::AssembleEnv as _,
	reload::{
		BuiltinInherited, ConstValue, InList, InitProc, NewProc, NewType, NewVar, Plan, ProcKey, ProcReload, ProcSettings, Redeclared, RemovedProc,
		RemovedVar, Reparented, VarDefault
	},
	Instruction, Node, Opcode, StaticScope
};

use crate::{
	assemble_env::AssembleEnv,
	instruction_hooking::{hook_instruction, hooked_offsets, original_code, unhook_instruction}
};

#[path = "reload_inherit.rs"]
mod inherit;
#[path = "reload_reparent.rs"]
mod reparent;

#[derive(Default)]
pub struct Prepared {
	new_vars: Vec<PreparedNewVar>,
	new_globals: Vec<(String, u32, u32)>,
	removed_procs: Vec<PreparedRemoval>,
	new_procs: Vec<PreparedNewProc>,
	init_procs: Vec<PreparedInitProc>,
	code: Vec<PreparedCode>,
	entry_words: Vec<(u32, SettingWord)>,
	renames: Vec<Rename>,
	appearances: BTreeSet<u32>,
	builtin_writes: Vec<(u32, Vec<BuiltinWrite>)>,
	type_fields: Vec<(u32, usize, u32)>,
	run_once: Vec<(u32, String, Vec<u32>)>,
	global_values: Vec<(u32, (u32, u32), Option<StringRef>)>,
	global_init_now: Option<Vec<u32>>,
	global_init_at_start: Option<raw_types::procs::ProcId>,
	in_lists: Vec<(u32, ProcEntryWords)>,
	instances: Vec<(u32, runtime_tables::Instance, ProcEntryWords, StringRef)>,
	builtin_inherits: Vec<(u32, String)>,
	reparent: reparent::PreparedReparents,
	new_types: Vec<String>,
	table_strings: Vec<StringRef>,
	proc_locals: Vec<u32>
}

struct PreparedNewVar {
	id: u32,
	name: StringRef,
	value: (u32, u32),
	text: Option<StringRef>
}

struct AddedVars {
	first: u32,
	vars: Vec<PreparedNewVar>,
	param_names: HashMap<String, u32>,
	run_once: Vec<(u32, String, ProcReload, HashMap<(StaticScope, Vec<u8>), u32>)>,
	in_lists_before: u32,
	in_lists: Vec<(u32, ProcEntryWords)>,
	instances: StagedInstances,
	statics: HashMap<(StaticScope, Vec<u8>), u32>,
	const_values: Vec<(u32, (u32, u32), Option<StringRef>)>,
	proc_locals: Vec<u32>
}

#[derive(Default)]
struct StagedInstances {
	existing: Option<Vec<runtime_tables::Instance>>,
	new: Vec<(runtime_tables::Instance, ProcEntryWords, Vec<u32>, StringRef)>
}

impl StagedInstances {
	fn find_or_add(&mut self, path: &str, (tag, data): (u8, u32), init: &[u32]) -> Result<u32, String> {
		let existing = match &mut self.existing {
			Some(existing) => existing,
			none => none.insert(runtime_tables::instances()?)
		};
		let runs_init = |proc: u32| {
			runtime_tables::proc_entry(raw_types::procs::ProcId(proc))
				.and_then(|entry| runtime_tables::misc_words(entry.0[ProcEntryWords::BYTECODE]))
				.is_some_and(|code| code == init)
		};
		if let Some(id) = existing
			.iter()
			.position(|i| i.tag == tag && i.data == data && i.init != NONE && runs_init(i.init))
		{
			return Ok(id as u32);
		}
		let count = existing.len() as u32;
		if let Some(id) = self.new.iter().position(|(i, _, code, _)| i.tag == tag && i.data == data && code == init) {
			return Ok(count + id as u32);
		}

		let mut entry = proc_template()?;
		for word in [
			ProcEntryWords::NAME,
			ProcEntryWords::DESC,
			ProcEntryWords::CATEGORY,
			ProcEntryWords::PATH_BASENAME,
			ProcEntryWords::PARAMETERS,
			ProcEntryWords::NEXT_OVERRIDE
		] {
			entry.0[word] = NONE;
		}
		let path = new_string(&format!("{}/proc/(map_init)", path))?;
		entry.0[ProcEntryWords::PATH] = path.get_id().0;
		entry.0[ProcEntryWords::BYTECODE] = runtime_tables::intern_misc(init)?;
		self.new
			.push((runtime_tables::Instance { tag, data, init: NONE }, entry, init.to_vec(), path));
		Ok(count + self.new.len() as u32 - 1)
	}
}

impl AddedVars {
	const fn next_id(&self) -> u32 {
		self.first + self.vars.len() as u32
	}

	fn param_name(&mut self, name: &str) -> Result<u32, String> {
		if let Some(id) = variable_ids(name.as_bytes()).first().or_else(|| self.param_names.get(name)).copied() {
			return Ok(id);
		}
		let id = self.add(new_string(name)?, ((0, 0), None));
		self.param_names.insert(name.to_owned(), id);
		Ok(id)
	}

	fn add(&mut self, name: StringRef, (value, text): ((u32, u32), Option<StringRef>)) -> u32 {
		let id = self.next_id();
		self.vars.push(PreparedNewVar { id, name, value, text });
		id
	}
}

#[derive(Default)]
struct StagedProcs {
	maps: HashMap<(u32, u32), Vec<u32>>,
	next_override: HashMap<u32, u32>,
	entries: HashMap<u32, ProcEntryWords>,
	own: HashMap<(u32, usize), Vec<u32>>,
	listed_after: HashMap<u32, u32>,
	removed: HashSet<u32>,
	world: HashMap<String, Vec<u32>>,
	strings: Vec<StringRef>
}

impl StagedProcs {
	fn named_procs(&self, type_id: u32, key: u32) -> Result<Vec<u32>, String> {
		match self.maps.get(&(type_id, key)) {
			Some(procs) => Ok(procs.clone()),
			None => runtime_tables::named_procs(type_id, key)
		}
	}

	fn entry(&self, proc: u32) -> Option<ProcEntryWords> {
		if let Some(entry) = self.entries.get(&proc) {
			return Some(*entry);
		}
		let mut entry = runtime_tables::proc_entry(raw_types::procs::ProcId(proc))?;
		if let Some(&next) = self.next_override.get(&proc) {
			entry.0[ProcEntryWords::NEXT_OVERRIDE] = next;
		}
		Some(entry)
	}

	fn world_definitions(&mut self, path: &str) -> &mut Vec<u32> {
		self.world
			.entry(path.to_owned())
			.or_insert_with(|| Proc::overrides(path).iter().map(|p| p.id.0).collect())
	}

	fn own(&self, type_id: u32, field: usize) -> Result<Vec<u32>, String> {
		let mut procs = runtime_tables::misc_words(runtime_tables::type_field(type_id, field)?).unwrap_or_default();
		procs.retain(|proc| !self.removed.contains(proc));
		for &proc in self.own.get(&(type_id, field)).into_iter().flatten() {
			match self.listed_after.get(&proc).and_then(|after| procs.iter().position(|p| p == after)) {
				Some(at) if !procs.contains(&proc) => procs.insert(at + 1, proc),
				_ => add_to_list(&mut procs, field, proc)
			}
		}
		Ok(procs)
	}
}

#[derive(Default)]
struct Pending {
	procs: Vec<PendingProc>,
	removed: HashSet<(String, u32)>,
	globals: HashMap<(StaticScope, Vec<u8>), u32>
}

struct PendingProc {
	path: String,
	override_id: u32,
	id: u32
}

enum Named {
	Unchanged,
	Proc(u32),
	Removed
}

impl Pending {
	fn proc_named(&self, path: &str) -> Named {
		let stripped = strip_path(path);
		let mut added: Vec<&PendingProc> = self.procs.iter().filter(|new| strip_path(&new.path) == stripped).collect();
		if added.is_empty() && !self.removed.iter().any(|(removed, _)| *removed == stripped) {
			return Named::Unchanged;
		}
		added.sort_by_key(|new| new.override_id);
		let mut order: Vec<(String, u32)> = Proc::overrides(path)
			.iter()
			.enumerate()
			.filter(|(at, _)| !self.removed.contains(&(stripped.clone(), *at as u32)))
			.map(|(_, proc)| (crate::assemble_env::own_path(proc), proc.id.0))
			.collect();
		for new in added {
			let at = (new.override_id as usize).min(order.len());
			order.insert(at, (new.path.clone(), new.id));
		}
		match order.into_iter().find(|(own, _)| own == path) {
			Some((_, id)) => Named::Proc(id),
			None => Named::Removed
		}
	}
}

fn removed_proc(path: &str) -> String {
	format!(
		"{} names a proc that this reload removes. Keep a definition of it, or change the code that names it",
		path
	)
}

fn removed_path_type(path: &str) -> Option<(u8, u32)> {
	AssembleEnv::type_value(path).filter(|(tag, _)| *tag != raw_types::values::ValueTag::ProcRef.0)
}

fn new_proc_path(new: &NewProc) -> String {
	match (new.type_path.as_str(), new.declared, new.verb) {
		("", true, _) => format!("/proc/{}", new.name),
		(ty, true, false) => format!("{}/proc/{}", ty, new.name),
		(ty, true, true) => format!("{}/verb/{}", ty, new.name),
		(ty, false, _) => format!("{}/{}", ty, new.name)
	}
}

#[derive(Default)]
struct TypeVars {
	tables: BTreeMap<u32, Vec<u32>>,
	declared: BTreeMap<u32, Vec<u32>>,
	new_names: HashMap<u32, u32>,
	parents: Option<Vec<u32>>
}

impl TypeVars {
	fn words(tables: &mut BTreeMap<u32, Vec<u32>>, type_id: u32, field: usize) -> Result<&mut Vec<u32>, String> {
		Ok(match tables.entry(type_id) {
			Entry::Occupied(entry) => entry.into_mut(),
			Entry::Vacant(entry) => {
				let id = runtime_tables::type_field(type_id, field)?;
				entry.insert(runtime_tables::misc_words(id).ok_or_else(|| format!("couldn't read the vars of type {}", type_id))?)
			}
		})
	}

	fn table(&mut self, type_id: u32) -> Result<&mut Vec<u32>, String> {
		Self::words(&mut self.tables, type_id, type_fields::VAR_TABLE)
	}

	fn declared(&mut self, type_id: u32) -> Result<&mut Vec<u32>, String> {
		Self::words(&mut self.declared, type_id, type_fields::DECLARED_VARS)
	}

	fn declared_now(&self, type_id: u32) -> Result<Cow<'_, [u32]>, String> {
		if let Some(words) = self.declared.get(&type_id) {
			return Ok(Cow::Borrowed(words));
		}
		let id = runtime_tables::type_field(type_id, type_fields::DECLARED_VARS)?;
		let words = runtime_tables::misc_words(id).ok_or_else(|| format!("couldn't read the vars of type {}", type_id))?;
		Ok(Cow::Owned(words))
	}

	fn name_of(&self, var_id: u32) -> u32 {
		self.new_names
			.get(&var_id)
			.copied()
			.or_else(|| runtime_tables::variable_name(var_id))
			.unwrap_or(u32::MAX)
	}

	fn subtree(&mut self, type_id: u32) -> Result<Vec<u32>, String> {
		if self.parents.is_none() {
			let count = runtime_tables::type_count()?;
			self.parents = Some((0..count).map(|t| reparent::parent_of(t).unwrap_or(NONE)).collect());
		}
		let parents = self.parents.as_ref().unwrap();
		Ok((0..parents.len() as u32)
			.filter(|&t| {
				let mut at = t;
				for _ in 0..parents.len() {
					if at == type_id {
						return true;
					}
					match parents.get(at as usize) {
						Some(&parent) if parent != NONE => at = parent,
						_ => return false
					}
				}
				false
			})
			.collect())
	}

	fn intern(self) -> Result<Vec<(u32, usize, u32)>, String> {
		let mut out = vec![];
		for (field, tables) in [(type_fields::VAR_TABLE, self.tables), (type_fields::DECLARED_VARS, self.declared)] {
			for (type_id, words) in tables {
				let id = if words.is_empty() { NONE } else { runtime_tables::intern_misc(&words)? };
				out.push((type_id, field, id));
			}
		}
		Ok(out)
	}
}

struct PreparedCode {
	proc: Proc,
	name: String,
	bytecode: Vec<u32>,
	bytecode_id: u32,
	locals: Option<LocalsTable>,
	params_id: Option<u32>,
	settings: Vec<SettingWord>,
	rename: Option<Rename>,
	local_names: Option<Vec<String>>,
	lines: Option<Vec<(u32, u32)>>
}

enum LocalsTable {
	Made(u32),
	Shared(u32)
}

struct PreparedInitProc {
	type_id: u32,
	proc: Option<(ProcEntryWords, Vec<String>)>,
	path: Option<StringRef>
}

struct PreparedNewProc {
	entry: ProcEntryWords,
	path: StringRef,
	basename: StringRef,
	settings: Vec<SettingWord>,
	id: u32,
	place: ProcKey,
	local_names: Vec<String>,
	on_type: Option<NewProcOnType>,
	on_world: Option<WorldEdit>
}

struct PreparedRemoval {
	id: u32,
	type_id: u32,
	field: usize,
	map_entries: Vec<(u32, u32, Vec<u32>)>,
	relinked: Vec<u32>,
	next: u32,
	on_world: Option<WorldEdit>
}

struct NewProcOnType {
	type_id: u32,
	field: usize,
	map_entries: Vec<(u32, u32, Vec<u32>)>,
	relinked: Vec<u32>,
	paths: Vec<(u32, StringRef)>,
	after: Option<u32>
}

#[derive(Clone)]
enum WorldEdit {
	Add {
		id: u32,
		keys: Vec<u32>,
		below: u32,
		above: Option<u32>
	},
	Remove {
		id: u32,
		keys: Vec<u32>
	}
}

pub trait Claimant: Ord {
	fn on_entry(&self) -> bool;
}

impl Claimant for () {
	fn on_entry(&self) -> bool {
		false
	}
}

fn describe(proc: &ProcKey) -> String {
	proc.to_string()
}

fn find(proc: &ProcKey) -> Result<Proc, String> {
	Proc::find_override(&proc.path, proc.override_id).ok_or_else(|| format!("{}: there's no such proc in the running world", describe(proc)))
}

fn strip_path(path: &str) -> String {
	path.replace("/proc/", "/").replace("/verb/", "/")
}

pub fn prepare(request: &HotReloadRequest, warnings: &mut Vec<String>, skipped: &mut Vec<String>) -> Result<Prepared, Vec<String>> {
	let HotReloadRequest {
		plan,
		resources: _,
		resource_dirs: _,
		global_init,
		shadowing_procs,
		local_reads
	} = request;
	let Plan {
		new_types,
		reloads,
		settings,
		new_procs,
		removed_procs,
		new_vars,
		redeclared,
		var_defaults,
		builtin_defaults,
		builtin_inherited,
		init_procs,
		reparented,
		line_moves,
		removed_vars
	} = plan;
	runtime_tables::available().map_err(|e| vec![format!("hot reloading isn't available on this build of BYOND: {}", e)])?;

	for var in removed_vars {
		warnings.push(if world_lacks(var) {
			format!(
				"{}: var {} was removed from source and isn't in the running world",
				var.type_path, var.var
			)
		} else {
			format!(
				"{}: var {} was removed from source, but the running world still has it",
				var.type_path, var.var
			)
		});
	}

	let mut prepared = Prepared {
		new_types: new_types.iter().map(|new| new.path.clone()).collect(),
		..Default::default()
	};
	let mut errors = vec![];
	let mut world_globals = WorldGlobals::default();
	for reads in local_reads {
		if let Ok(proc) = find(&reads.proc) {
			world_globals.local_reads.entry(proc.id.0).or_default().push(reads.clone());
		}
	}

	let mut pending = Pending::default();
	let first_id = runtime_tables::proc_count();
	let ids: Vec<u32> = (first_id..).take(new_procs.len()).collect();
	pending.procs = new_procs
		.iter()
		.zip(&ids)
		.map(|(new, &id)| PendingProc {
			path: new_proc_path(new),
			override_id: new.code.proc.override_id,
			id
		})
		.collect();
	pending.removed = removed_procs
		.iter()
		.map(|removed| (strip_path(&removed.proc.path), removed.proc.override_id))
		.collect();
	let mut type_vars = TypeVars::default();
	let mut staged = StagedProcs::default();
	reparent::prepare(reparented, &mut type_vars, &mut staged, &mut prepared, &mut errors);

	let mut added = AddedVars {
		first: runtime_tables::variable_count(),
		vars: vec![],
		param_names: HashMap::new(),
		run_once: vec![],
		in_lists_before: runtime_tables::in_list_count().unwrap_or(0),
		in_lists: vec![],
		instances: StagedInstances::default(),
		statics: HashMap::new(),
		const_values: vec![],
		proc_locals: vec![]
	};
	for new in new_vars {
		if let Err(e) = prepare_new_var(new, &mut added, &mut type_vars, &mut pending, warnings) {
			errors.push(e);
		}
	}
	for var in redeclared {
		let shadowing: Vec<&ShadowingProc> = shadowing_procs.iter().filter(|p| p.global == var.var).collect();
		if let Err(e) = prepare_redeclared(
			var,
			&shadowing,
			&mut added,
			&mut type_vars,
			&mut world_globals,
			&mut pending,
			&mut prepared.global_values
		) {
			errors.push(e);
		}
	}

	for removed in removed_procs {
		if Proc::overrides(removed.proc.path.as_str()).is_empty() {
			warnings.push(format!(
				"{}: not found in the running world, so there is nothing to remove",
				removed.proc.path
			));
			if !skipped.contains(&removed.proc.path) {
				skipped.push(removed.proc.path.clone());
			}
			continue;
		}
		if let Ok(proc) = find(&removed.proc) {
			let own = world_globals.globals_of(&proc).locals;
			added.proc_locals.extend(own.values().flat_map(|own| own.values().copied()));
		}
		match prepare_removal(removed, &mut staged) {
			Ok(p) => {
				if removed.verb {
					prepared.appearances.extend(type_vars.subtree(p.type_id).unwrap_or_default());
				}
				prepared.removed_procs.push(p);
			}
			Err(e) => errors.push(e)
		}
	}

	for (new, &id) in new_procs.iter().zip(&ids) {
		match prepare_new_proc(new, id, &pending, &mut world_globals, &mut added, &mut staged) {
			Ok(p) => {
				if let Some(on_type) = p.on_type.as_ref().filter(|_| new.verb) {
					prepared.appearances.extend(type_vars.subtree(on_type.type_id).unwrap_or_default());
				}
				prepared.new_procs.push(p);
			}
			Err(e) => errors.push(e)
		}
	}

	for init in init_procs {
		match prepare_init_proc(init, &pending, &mut world_globals, &mut added) {
			Ok(p) => prepared.init_procs.push(p),
			Err(e) => errors.push(e)
		}
	}

	for reload in reloads {
		match prepare_code(reload, &pending, &mut world_globals, &mut added, warnings) {
			Ok(p) => prepared.code.push(p),
			Err(e) => errors.push(e)
		}
	}

	let mut restarted = false;
	if let Some(init) = global_init {
		match prepare_global_init(init, &pending, &mut world_globals, &mut added, warnings) {
			Ok(Some(StartingInit::Proc(code, at_start))) => {
				prepared.global_init_at_start = at_start.then_some(code.proc.id);
				prepared.code.push(code);
				restarted = true;
			}
			Ok(Some(StartingInit::Now(bytecode))) => {
				prepared.global_init_now = Some(bytecode);
				restarted = true;
			}
			Ok(None) => {}
			Err(e) => warnings.push(format!("{}: reboot will use the build's global and static initializers", e))
		}
	}
	if restarted {
		added.run_once.clear();
	}

	for change in settings {
		let proc_ref = &change.proc;
		let words = find(proc_ref).and_then(|proc| {
			let entry = runtime_tables::proc_entry(proc.id).ok_or_else(|| format!("{}: couldn't read its table entry", describe(proc_ref)))?;
			let words = settings_words(&entry, &change.settings, &proc_ref.path)?;
			prepared.renames.extend(renamed(&proc, &entry, &words));
			Ok(words.into_iter().map(move |setting| (proc.id.0, setting)))
		});
		match words {
			Ok(words) => prepared.entry_words.extend(words),
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

		if let Some(bytecode) = remap_lines(&original_code(&proc), &moved.lines) {
			match runtime_tables::intern_misc(&bytecode) {
				Ok(bytecode_id) if bytecode.len() >= u16::MAX as usize && moved_names_need_copy(&proc, bytecode_id) => errors.push(format!(
					"{}: moving its lines gives back code that older calls run with other names, and {} words leave no room for a copy with the new \
					 names. Rebuild to move them",
					describe(&moved.proc),
					bytecode.len()
				)),
				Ok(bytecode_id) => prepared.code.push(PreparedCode {
					proc,
					name: describe(&moved.proc),
					bytecode,
					bytecode_id,
					locals: None,
					params_id: None,
					settings: vec![],
					rename: None,
					local_names: None,
					lines: Some(moved.lines.clone())
				}),
				Err(e) => errors.push(format!("{}: {}", describe(&moved.proc), e))
			}
		}
	}

	for default in var_defaults {
		if let Err(e) = prepare_var_default(default, &pending, &mut type_vars, &mut added, &mut prepared.table_strings) {
			errors.push(e);
		}
	}
	for default in builtin_defaults {
		let writes = builtin_writes(&default.var, &default.value).map_err(|e| format!("{}: {}", default.var, e));
		match writes {
			Ok(writes) => {
				for path in &default.types {
					match find_type(path) {
						Ok(type_id) => {
							prepared.builtin_writes.push((type_id, writes.clone()));
							prepared.appearances.insert(type_id);
						}
						Err(e) => errors.push(e)
					}
				}
			}
			Err(e) => errors.push(e)
		}
	}

	inherit::prepare(builtin_inherited, &mut prepared, &mut errors);

	for (id, name, init, owner_slots) in std::mem::take(&mut added.run_once) {
		let code = value_code(init);
		if !code.locals.is_empty() {
			errors.push(format!(
				"{}: its initial value needs local variables, which hot reload does not support yet",
				name
			));
			continue;
		}
		match assemble_with(&name, &code, None, owner_slots, &pending, &mut world_globals, &mut added) {
			Ok(bytecode) => prepared.run_once.push((id, name, bytecode)),
			Err(e) => errors.push(e)
		}
	}

	if !prepared.appearances.is_empty() {
		warnings.push("verb and built-in changes apply only to new instances. Existing instances keep their values".to_owned());
	}

	prepared.global_values.append(&mut added.const_values);
	prepared.proc_locals = std::mem::take(&mut added.proc_locals);
	prepared.new_vars = added.vars;
	prepared.new_globals = pending
		.globals
		.iter()
		.filter(|((scope, _), _)| *scope == StaticScope::Global)
		.map(|((_, name), id)| {
			let name = String::from_utf8_lossy(name).into_owned();
			let flags = request
				.plan
				.new_vars
				.iter()
				.find(|new| new.type_path.is_empty() && new.var == name)
				.map_or(0, |new| new.flags);
			(name, *id, flags)
		})
		.collect();
	prepared.in_lists = added.in_lists;
	let first_instance = added.instances.existing.as_ref().map_or(0, Vec::len) as u32;
	prepared.instances = added
		.instances
		.new
		.into_iter()
		.enumerate()
		.map(|(n, (instance, entry, _, path))| (first_instance + n as u32, instance, entry, path))
		.collect();
	if !prepared.new_globals.is_empty() {
		if let Err(e) = runtime_tables::global_vars_available() {
			errors.push(format!(
				"new global vars can't be added to global.vars on this build of BYOND: {}. Rebuild to add them",
				e
			));
		}
	}
	let stub = Proc::find("/proc/auxtools_expr_stub").is_some() || matches!(pending.proc_named("/proc/auxtools_expr_stub"), Named::Proc(_));
	if (prepared.global_init_now.is_some() || !prepared.run_once.is_empty()) && !stub {
		errors.push("initial values can't be worked out: the world has no /proc/auxtools_expr_stub to run them in".to_owned());
	}
	if errors.is_empty() {
		match type_vars.intern() {
			Ok(fields) => prepared.type_fields = fields,
			Err(e) => errors.push(format!("couldn't make the types' new vars: {}", e))
		}
	}

	reparent::finish();
	if errors.is_empty() {
		Ok(prepared)
	} else {
		Err(errors)
	}
}

fn assemble(
	name: &str,
	code: &ProcReload,
	original: Option<&Proc>,
	pending: &Pending,
	world_globals: &mut WorldGlobals,
	added: &mut AddedVars
) -> Result<Vec<u32>, String> {
	assemble_with(name, code, original, HashMap::new(), pending, world_globals, added)
}

fn assemble_with(
	name: &str,
	code: &ProcReload,
	original: Option<&Proc>,
	known: HashMap<(StaticScope, Vec<u8>), u32>,
	pending: &Pending,
	world_globals: &mut WorldGlobals,
	added: &mut AddedVars
) -> Result<Vec<u32>, String> {
	let nodes = adapt_debug_info(&code.nodes, &original.map(original_code).unwrap_or_default());

	let uses = original.map(|proc| world_globals.of(proc)).unwrap_or_default();
	let proc_globals = original.map(|proc| world_globals.globals_of(proc)).unwrap_or_default();
	added
		.proc_locals
		.extend(proc_globals.locals.values().flat_map(|own| own.values().copied()));
	let first_run_once = added.run_once.len();
	let new_statics = new_statics(code, &proc_globals, pending, added).map_err(|e| format!("{}: {}", name, e))?;
	let mut env = ReloadEnv {
		proc_globals,
		world_globals,
		pending,
		new_statics,
		known,
		instances: &mut added.instances,
		place: uses.place(),
		error: None
	};
	let bytecode = match dmasm::assembler::assemble(&nodes, &mut env) {
		Ok(bytecode) => bytecode,
		Err(e) => return Err(format!("{}: {}", name, env.error.take().unwrap_or_else(|| describe_assemble_error(&e))))
	};
	for (.., init, owner_slots) in &mut added.run_once[first_run_once..] {
		*owner_slots = env.proc_statics_read(&init.nodes);
	}

	if bytecode.len() > u16::MAX as usize {
		return Err(format!(
			"{}: the new code is {} words long, and BYOND's limit is {}",
			name,
			bytecode.len(),
			u16::MAX
		));
	}

	Ok(bytecode)
}

fn prepare_code(
	reload: &ProcReload,
	pending: &Pending,
	world_globals: &mut WorldGlobals,
	added: &mut AddedVars,
	warnings: &mut Vec<String>
) -> Result<PreparedCode, String> {
	let name = describe(&reload.proc);
	let proc = find(&reload.proc)?;
	if proc.is_hooked() {
		warnings.push(format!("{}: auxtools has hooked it, so the hook runs instead of the new code", name));
	}
	let bytecode = assemble(&name, reload, Some(&proc), pending, world_globals, added)?;
	prepared_code(proc, name, reload, bytecode, pending, world_globals, added)
}

fn prepared_code(
	proc: Proc,
	name: String,
	reload: &ProcReload,
	bytecode: Vec<u32>,
	pending: &Pending,
	world_globals: &mut WorldGlobals,
	added: &mut AddedVars
) -> Result<PreparedCode, String> {
	let entry = runtime_tables::proc_entry(proc.id).ok_or_else(|| format!("{}: couldn't read its table entry", name))?;
	let bytecode_id = runtime_tables::intern_misc(&bytecode).map_err(|e| format!("{}: {}", name, e))?;

	let locals = if reload.locals.len() == proc.locals_count() {
		None
	} else if reload.locals.is_empty() {
		Some(LocalsTable::Shared(empty_locals().map_err(|e| format!("{}: {}", name, e))?))
	} else {
		Some(LocalsTable::Made(
			locals_table(&reload.locals, NONE).map_err(|e| format!("{}: {}", name, e))?
		))
	};

	let params_id =
		params_table(&reload.params, entry.0[ProcEntryWords::PARAMETERS], pending, world_globals, added).map_err(|e| format!("{}: {}", name, e))?;

	let settings = match &reload.settings {
		Some(settings) => settings_words(&entry, settings, &reload.proc.path).map_err(|e| format!("{}: {}", name, e))?,
		None => vec![]
	};
	let rename = renamed(&proc, &entry, &settings);

	let params: Vec<String> = reload.params.iter().map(|param| param.name.clone()).collect();
	if names_need_copy(&proc, entry.0[ProcEntryWords::BYTECODE], bytecode_id, &reload.locals, &params) {
		failing(FailAt::Copy).map_err(|e| format!("{}: {}", name, e))?;
		if bytecode.len() >= u16::MAX as usize {
			return Err(format!(
				"{}: renaming only its locals or parameters needs a copy of its code, and {} words leave no room for one. Rebuild to rename them",
				name,
				bytecode.len()
			));
		}
	}

	Ok(PreparedCode {
		proc,
		name,
		bytecode,
		bytecode_id,
		locals,
		params_id,
		settings,
		rename,
		local_names: Some(reload.locals.clone()),
		lines: None
	})
}

fn names_need_copy(proc: &Proc, current: u32, bytecode_id: u32, locals: &[String], params: &[String]) -> bool {
	code_address(bytecode_id)
		.filter(|&at| current == bytecode_id || debug::has_local_names(proc, at))
		.is_some_and(|at| debug::local_names_for(proc, at) != locals || debug::parameter_names_for(proc, at) != params)
}

fn moved_names_need_copy(proc: &Proc, bytecode_id: u32) -> bool {
	let current = runtime_tables::proc_entry(proc.id).map_or(NONE, |entry| entry.0[ProcEntryWords::BYTECODE]);
	let locals = debug::local_names_for(proc, unsafe { proc.bytecode_mut_ptr() }.0);
	let params: Vec<String> = proc.parameter_names().into_iter().map(String::from).collect();
	names_need_copy(proc, current, bytecode_id, &locals, &params)
}

fn code_address(bytecode_id: u32) -> Option<*const u32> {
	let mut entry = std::ptr::null_mut();
	unsafe {
		if raw_types::funcs::get_misc_by_id(&mut entry, raw_types::misc::MiscId::from_raw(bytecode_id)) != 1 || entry.is_null() {
			return None;
		}
		Some((*(entry as *const raw_types::misc::Misc_V2)).bytecode.bytecode)
	}
}

enum StartingInit {
	Proc(PreparedCode, bool),
	Now(Vec<u32>)
}

fn prepare_global_init(
	init: &dmasm::reload::GlobalInit,
	pending: &Pending,
	world_globals: &mut WorldGlobals,
	added: &mut AddedVars,
	warnings: &mut Vec<String>
) -> Result<Option<StartingInit>, String> {
	const NAME: &str = auxtools_protocol::GLOBAL_INIT;
	let ran = || {
		format!(
			"{}: global initialization has already run. Rebuild to apply changed global and static initializers",
			NAME
		)
	};
	let Some(start) = auxtools::starting_call().filter(|_| !crate::world_resources::is_loaded()) else {
		warnings.push(ran());
		return Ok(None);
	};
	if !init.code.locals.is_empty() {
		return Err(format!(
			"{}: the new global init proc needs local variables, which hot reload does not support yet",
			NAME
		));
	}
	let code = value_code(init.code.clone());
	let Some(proc) = global_init_proc() else {
		let known = global_slots(&code, &init.proc_statics, pending, added, world_globals);
		let bytecode = assemble_with(NAME, &code, None, known, pending, world_globals, added)?;
		return Ok(Some(StartingInit::Now(bytecode)));
	};
	if proc.id != start && debug::CallStacks::active().iter().any(|frame| frame.proc.id == proc.id) {
		warnings.push(ran());
		return Ok(None);
	}
	let known = global_slots(&code, &init.proc_statics, pending, added, world_globals);
	let bytecode = assemble_with(NAME, &code, None, known, pending, world_globals, added)?;
	let at_start = proc.id == start;
	prepared_code(proc, NAME.to_owned(), &code, bytecode, pending, world_globals, added).map(|code| Some(StartingInit::Proc(code, at_start)))
}

pub fn early_global_init(init: &dmasm::reload::GlobalInit, rest: &HotReloadRequest) -> (dmasm::reload::GlobalInit, usize) {
	let later: HashSet<(StaticScope, Vec<u8>)> = rest
		.plan
		.reloads
		.iter()
		.chain(rest.plan.new_procs.iter().map(|new| &new.code))
		.flat_map(|code| code.statics.iter().map(|s| (s.scope.clone(), s.name.as_bytes().to_vec())))
		.chain(
			rest.plan
				.new_vars
				.iter()
				.filter(|var| !var.type_path.is_empty())
				.map(|var| (StaticScope::Type(var.type_path.clone()), var.var.as_bytes().to_vec()))
		)
		.collect();
	let added = AddedVars {
		first: runtime_tables::variable_count(),
		vars: vec![],
		param_names: HashMap::new(),
		run_once: vec![],
		in_lists_before: 0,
		in_lists: vec![],
		instances: StagedInstances::default(),
		statics: HashMap::new(),
		const_values: vec![],
		proc_locals: vec![]
	};
	let code = value_code(init.code.clone());
	let known = global_slots(&code, &init.proc_statics, &Pending::default(), &added, &mut WorldGlobals::default());
	let mut left = 0;
	let init = init.without(|var| {
		let key = (var.scope.clone().unwrap_or_default(), var.name.0.clone());
		let leave = later.contains(&key) && !known.contains_key(&key);
		left += usize::from(leave);
		leave
	});
	(init, left)
}

fn global_slots(
	code: &ProcReload,
	counts: &BTreeMap<String, BTreeMap<String, usize>>,
	pending: &Pending,
	added: &AddedVars,
	world_globals: &mut WorldGlobals
) -> HashMap<(StaticScope, Vec<u8>), u32> {
	let mut known = HashMap::new();
	let mut file = vec![];
	for node in &code.nodes {
		let global = match node {
			Node::Instruction(dmasm::Instruction::DbgFile(name), _) => {
				file = name.0.clone();
				continue;
			}
			Node::Instruction(dmasm::Instruction::SetVar(dmasm::Variable::Global(global)), _) => global,
			_ => continue
		};
		let (scope, name) = (global.scope.clone().unwrap_or_default(), global.name.0.clone());
		let key = (scope, name);
		if known.contains_key(&key) {
			continue;
		}
		let (scope, name) = &key;
		let id = pending
			.globals
			.get(&key)
			.or_else(|| added.statics.get(&key))
			.copied()
			.or_else(|| match scope {
				StaticScope::Global => {
					let globals = runtime_tables::global_var_ids().ok()?;
					match variable_ids(name).into_iter().filter(|id| globals.contains(id)).collect::<Vec<_>>()[..] {
						[id] => Some(id),
						_ => None
					}
				}
				StaticScope::Proc { path, nth } => proc_static(path, *nth, name, counts, world_globals).or_else(|| {
					let unique = |ids: BTreeSet<u32>| (ids.len() == 1).then(|| ids.first().copied()).flatten();
					let places: Vec<_> = Proc::overrides(&strip_path(path))
						.iter()
						.filter_map(|proc| world_globals.of(proc).place())
						.collect();
					let initialized = world_globals.initialized();
					let in_proc = initialized
						.iter()
						.filter(|(at, global, line, _)| {
							global == name && places.iter().any(|(file, first, last)| at == file && (*first..=*last).contains(line))
						})
						.map(|(.., id)| *id)
						.collect();
					let in_file = initialized
						.iter()
						.filter(|(at, global, ..)| *at == file && global == name)
						.map(|(.., id)| *id)
						.collect();
					unique(in_proc).or_else(|| unique(in_file))
				}),
				StaticScope::Type(path) => declared_global(path, &String::from_utf8_lossy(name))
			});
		if let Some(id) = id {
			known.insert(key, id);
		}
	}
	known
}

fn global_init_proc() -> Option<Proc> {
	let proc = crate::unnamed::find(auxtools_protocol::GLOBAL_INIT, 0)?;
	let types = runtime_tables::type_count().ok()?;
	let a_types = (0..types).any(|type_id| runtime_tables::type_field(type_id, type_fields::INIT_PROC).ok() == Some(proc.id.0));
	(!a_types).then_some(proc)
}

fn proc_static(
	path: &str,
	nth: u32,
	name: &[u8],
	counts: &BTreeMap<String, BTreeMap<String, usize>>,
	world_globals: &mut WorldGlobals
) -> Option<u32> {
	let declared = *counts.get(path)?.get(String::from_utf8_lossy(name).as_ref())?;
	let ids: BTreeSet<u32> = Proc::overrides(&strip_path(path))
		.iter()
		.filter_map(|proc| {
			let named = world_globals.of(proc).by_name().remove(name)?;
			(named.len() == declared).then(|| named.get(nth as usize).copied())?
		})
		.collect();
	match ids.len() {
		1 => ids.first().copied(),
		_ => None
	}
}

#[derive(Clone)]
struct SettingWord {
	word: usize,
	value: u32,
	made: Option<StringRef>
}

impl SettingWord {
	const fn number(word: usize, value: u32) -> Self {
		Self { word, value, made: None }
	}

	fn string(word: usize, text: Option<&str>) -> Result<Self, String> {
		let Some(text) = text else {
			return Ok(Self::number(word, NONE));
		};
		let made = new_string(text)?;
		Ok(Self {
			word,
			value: made.get_id().0,
			made: Some(made)
		})
	}
}

fn settings_words(entry: &ProcEntryWords, settings: &ProcSettings, path: &str) -> Result<Vec<SettingWord>, String> {
	let basename = path.rsplit('/').next().unwrap_or(path).replace('_', " ");
	let words = [
		SettingWord::number(
			ProcEntryWords::FLAGS,
			(entry.0[ProcEntryWords::FLAGS] & !ProcSettings::FLAGS) | settings.flags()
		),
		SettingWord::number(ProcEntryWords::SETTINGS, settings.packed()),
		SettingWord::string(ProcEntryWords::NAME, Some(settings.name.as_deref().unwrap_or(&basename)))?,
		SettingWord::string(ProcEntryWords::DESC, settings.desc.as_deref())?,
		SettingWord::string(ProcEntryWords::CATEGORY, settings.category.as_deref())?
	];
	Ok(words.into_iter().filter(|setting| entry.0[setting.word] != setting.value).collect())
}

static PROC_STRINGS: Mutex<BTreeMap<(u32, usize), u32>> = Mutex::new(BTreeMap::new());

fn hold_setting(proc: u32, setting: SettingWord) {
	let mut held = PROC_STRINGS.lock().unwrap();
	let released = match setting.made {
		Some(made) => {
			std::mem::forget(made);
			held.insert((proc, setting.word), setting.value)
		}
		None => held.remove(&(proc, setting.word))
	};
	drop(held);
	if let Some(released) = released {
		release_string(released);
	}
}

fn set_setting(proc: u32, setting: SettingWord) {
	runtime_tables::set_proc_entry_word(raw_types::procs::ProcId(proc), setting.word, setting.value);
	hold_setting(proc, setting);
}

#[derive(Clone, Copy)]
struct Rename {
	proc: u32,
	type_id: Option<u32>,
	from: u32,
	to: u32,
	basename: u32
}

fn renamed(proc: &Proc, entry: &ProcEntryWords, words: &[SettingWord]) -> Option<Rename> {
	let to = words.iter().find(|setting| setting.word == ProcEntryWords::NAME)?.value;
	let from = entry.0[ProcEntryWords::NAME];
	if from == NONE || from == to {
		return None;
	}
	let path = strip_path(&proc.path);
	let (type_path, _) = path.rsplit_once('/')?;
	let type_id = match type_path {
		"/world" => None,
		_ => Some(find_type(type_path).ok()?)
	};
	let named = match type_id {
		Some(type_id) => runtime_tables::named_procs(type_id, from),
		None => runtime_tables::world_named_procs(from)
	};
	named.is_ok_and(|procs| procs.contains(&proc.id.0)).then_some(Rename {
		proc: proc.id.0,
		type_id,
		from,
		to,
		basename: entry.0[ProcEntryWords::PATH_BASENAME]
	})
}

fn rename(r: Rename) -> Result<(), String> {
	let Some(type_id) = r.type_id else {
		return rename_world_proc(r);
	};
	let depth = |id: u32| {
		Proc::from_id(raw_types::procs::ProcId(id))
			.and_then(|p| {
				let path = strip_path(&p.path);
				let (type_path, _) = path.rsplit_once('/')?;
				find_type(type_path).ok()
			})
			.map_or(0, |t| type_and_ancestors(t).len())
	};
	let own_depth = depth(r.proc);
	for sub in 0..runtime_tables::type_count()? {
		if !type_and_ancestors(sub).contains(&type_id) {
			continue;
		}
		if r.from != r.basename {
			let mut old = runtime_tables::named_procs(sub, r.from)?;
			if let Some(at) = old.iter().position(|&p| p == r.proc) {
				old.remove(at);
				runtime_tables::set_named_procs(sub, r.from, &old)?;
			}
		}
		let mut new = runtime_tables::named_procs(sub, r.to)?;
		if !new.contains(&r.proc) {
			let at = new.iter().position(|&p| depth(p) < own_depth).unwrap_or(new.len());
			new.insert(at, r.proc);
			runtime_tables::set_named_procs(sub, r.to, &new)?;
		}
	}
	Ok(())
}

fn rename_world_proc(r: Rename) -> Result<(), String> {
	if r.from != r.basename {
		let mut old = runtime_tables::world_named_procs(r.from)?;
		old.retain(|&p| p != r.proc);
		runtime_tables::set_world_named_procs(r.from, &old)?;
	}
	let mut new = runtime_tables::world_named_procs(r.to)?;
	if !new.contains(&r.proc) {
		let pinned = PINNED_WORLD_PROCS.lock().unwrap().clone();
		let definitions = Proc::from_id(raw_types::procs::ProcId(r.proc))
			.map(|p| Proc::overrides(p.path))
			.unwrap_or_default();
		let later: Vec<u32> = definitions.iter().map(|p| p.id.0).skip_while(|&p| p != r.proc).skip(1).collect();
		let at = new.iter().position(|p| !pinned.contains(p) && !later.contains(p)).unwrap_or(new.len());
		new.insert(at, r.proc);
		runtime_tables::set_world_named_procs(r.to, &new)?;
	}
	Ok(())
}

pub(crate) fn locals_table(names: &[String], empty: u32) -> Result<u32, String> {
	if names.is_empty() {
		return Ok(empty);
	}
	let null_slot = |id: &u32| runtime_tables::global_value(*id).is_ok_and(|v| v.tag == raw_types::values::ValueTag::Null);
	let mut words = vec![];
	for name in names {
		let id = match variable_ids(name.as_bytes()).into_iter().find(null_slot) {
			Some(id) => id,
			None => {
				let count = unsafe { *raw_types::funcs::VARIABLE_NAME_COUNT };
				(0..count).find(null_slot).ok_or("no var id has a null value to start a local with")?
			}
		};
		words.push(id);
	}
	runtime_tables::intern_misc(&words)
}

fn empty_locals() -> Result<u32, String> {
	(0..)
		.map_while(|id| runtime_tables::proc_entry(raw_types::procs::ProcId(id)))
		.map(|entry| entry.0[ProcEntryWords::LOCALS])
		.find(|&id| id != NONE && runtime_tables::misc_words(id).is_some_and(|names| names.is_empty()))
		.ok_or_else(|| "no proc in the world has an empty locals table to share. Rebuild to remove its last local".to_owned())
}

fn value_code(code: ProcReload) -> ProcReload {
	ProcReload {
		proc: ProcKey {
			path: code.proc.path,
			override_id: 0
		},
		nodes: code.nodes,
		locals: code.locals,
		params: vec![],
		settings: code.settings,
		statics: vec![],
		warnings: vec![]
	}
}

fn in_list_proc(param: &str, code: &ProcReload, pending: &Pending, world_globals: &mut WorldGlobals, added: &mut AddedVars) -> Result<u32, String> {
	let shown = format!("parameter {}'s `in` list", param);
	let template = proc_template()?;
	let code = value_code(code.clone());
	let bytecode = assemble(&shown, &code, None, pending, world_globals, added)?;

	let mut entry = template;
	for word in [
		ProcEntryWords::PATH,
		ProcEntryWords::NAME,
		ProcEntryWords::DESC,
		ProcEntryWords::CATEGORY,
		ProcEntryWords::PATH_BASENAME,
		ProcEntryWords::NEXT_OVERRIDE
	] {
		entry.0[word] = NONE;
	}
	entry.0[ProcEntryWords::BYTECODE] = runtime_tables::intern_misc(&bytecode)?;
	entry.0[ProcEntryWords::LOCALS] = locals_table(&code.locals, template.0[ProcEntryWords::LOCALS])?;

	let index = added.in_lists_before + added.in_lists.len() as u32;
	added.in_lists.push((index, entry));
	Ok((index << 8) | 0x40)
}

fn params_table(
	params: &[dmasm::reload::Param],
	current: u32,
	pending: &Pending,
	world_globals: &mut WorldGlobals,
	added: &mut AddedVars
) -> Result<Option<u32>, String> {
	let current_words = runtime_tables::misc_words(current).unwrap_or_default();
	let name_of = |words: &[u32]| {
		unsafe { StringRef::from_variable_id(raw_types::strings::VariableId(words[2])) }
			.data()
			.to_vec()
	};
	let current_in_list = |name: &str| -> Option<u32> {
		current_words
			.chunks(4)
			.find(|words| words.len() == 4 && name_of(words) == name.as_bytes())
			.map(|words| words[1])
	};

	let mut in_lists = vec![];
	for param in params {
		in_lists.push(match &param.in_list {
			InList::Builtin(word) => *word,
			InList::Kept => current_in_list(&param.name)
				.ok_or_else(|| format!("parameter {} has an `in` list that the running proc does not have", param.name))?,
			InList::New(code) => in_list_proc(&param.name, code, pending, world_globals, added)?
		});
	}

	let unchanged = current_words.len() == params.len() * 4
		&& params
			.iter()
			.zip(&in_lists)
			.zip(current_words.chunks(4))
			.all(|((param, &in_list), words)| {
				words[0] == param.input && words[1] == in_list && words[3] == param.input_extra && name_of(words) == param.name.as_bytes()
			});
	if unchanged {
		return Ok(None);
	}
	if params.is_empty() {
		return Ok(Some(NONE));
	}

	let mut words = vec![];
	for (param, in_list) in params.iter().zip(in_lists) {
		let id = added.param_name(&param.name).map_err(|e| format!("parameter {}: {}", param.name, e))?;
		words.extend([param.input, in_list, id, param.input_extra]);
	}
	runtime_tables::intern_misc(&words).map(Some)
}

fn world_lacks(var: &RemovedVar) -> bool {
	let Ok(type_id) = find_type(&var.type_path) else {
		return true;
	};
	let Ok(name) = StringRef::new(&var.var) else {
		return false;
	};
	let name = name.get_id().0;
	let mut type_vars = TypeVars::default();
	match type_vars.declared(type_id).cloned() {
		Ok(declared) => !declared.chunks(2).any(|pair| type_vars.name_of(pair[0]) == name),
		Err(_) => false
	}
}

fn find_type(path: &str) -> Result<u32, String> {
	let id = StringRef::new(path).map_err(|_| format!("couldn't make the string {}", path))?.get_id().0;
	(0..runtime_tables::type_count()?)
		.find(|&t| runtime_tables::type_field(t, type_fields::PATH) == Ok(id))
		.ok_or_else(|| format!("there's no type {} in the running world", path))
}

pub(crate) fn type_id(path: &str) -> Option<u32> {
	find_type(path).ok()
}

pub(crate) fn image_type(path: &str) -> Option<u32> {
	let id = find_type(path).ok()?;
	let image = find_type("/image").ok()?;
	type_and_ancestors(id).contains(&image).then_some(id)
}

pub(crate) fn ancestry(path: &str) -> Option<Vec<String>> {
	let id = find_type(path).ok()?;
	Some(type_and_ancestors(id).into_iter().map(type_path).collect())
}

fn type_path(type_id: u32) -> String {
	runtime_tables::type_field(type_id, type_fields::PATH)
		.ok()
		.map(|id| String::from(unsafe { StringRef::from_id(raw_types::strings::StringId(id)) }))
		.unwrap_or_else(|| format!("type {}", type_id))
}

fn type_and_ancestors(type_id: u32) -> Vec<u32> {
	let mut out = vec![type_id];
	let mut at = type_id;
	while let Ok(parent) = reparent::parent_of(at) {
		if parent == NONE || out.contains(&parent) {
			break;
		}
		out.push(parent);
		at = parent;
	}
	out
}

fn prepare_init_proc(
	init: &InitProc,
	pending: &Pending,
	world_globals: &mut WorldGlobals,
	added: &mut AddedVars
) -> Result<PreparedInitProc, String> {
	let shown = format!("{}'s init proc", init.type_path);
	let fail = |e: String| format!("{}: {}", shown, e);
	let type_id = find_type(&init.type_path).map_err(fail)?;
	let Some(code) = &init.code else {
		return Ok(PreparedInitProc {
			type_id,
			proc: None,
			path: None
		});
	};

	let current = runtime_tables::type_field(type_id, type_fields::INIT_PROC).map_err(fail)?;
	let current = (current != NONE).then(|| Proc::from_id(raw_types::procs::ProcId(current))).flatten();
	let template = match &current {
		Some(proc) => runtime_tables::proc_entry(proc.id),
		None => proc_template().ok()
	}
	.ok_or_else(|| fail("couldn't find a proc entry to copy".to_owned()))?;
	let bytecode = assemble(&shown, code, current.as_ref(), pending, world_globals, added)?;
	let mut entry = template;
	let mut path = None;
	if current.is_none() {
		let made = new_string(&format!("{}/proc/(init)", init.type_path)).map_err(fail)?;
		entry.0[ProcEntryWords::PATH] = made.get_id().0;
		path = Some(made);
		entry.0[ProcEntryWords::NAME] = NONE;
		entry.0[ProcEntryWords::DESC] = NONE;
		entry.0[ProcEntryWords::CATEGORY] = NONE;
		entry.0[ProcEntryWords::PATH_BASENAME] = NONE;
		entry.0[ProcEntryWords::PARAMETERS] = NONE;
	}
	entry.0[ProcEntryWords::BYTECODE] = runtime_tables::intern_misc(&bytecode).map_err(fail)?;
	entry.0[ProcEntryWords::LOCALS] = locals_table(&code.locals, template.0[ProcEntryWords::LOCALS]).map_err(fail)?;
	entry.0[ProcEntryWords::NEXT_OVERRIDE] = NONE;

	Ok(PreparedInitProc {
		type_id,
		proc: Some((entry, code.locals.clone())),
		path
	})
}

fn prepare_removal(removed: &RemovedProc, staged: &mut StagedProcs) -> Result<PreparedRemoval, String> {
	let shown = describe(&removed.proc);
	let fail = |e: String| format!("{}: {}", shown, e);
	let id = find(&removed.proc)?.id.0;
	let entry = staged.entry(id).ok_or_else(|| fail("couldn't read its table entry".to_owned()))?;
	let next = entry.0[ProcEntryWords::NEXT_OVERRIDE];

	if removed.type_path.is_empty() || (removed.type_path == "/world" && !removed.verb) {
		let on_world = if removed.type_path.is_empty() {
			None
		} else {
			staged.world_definitions(&strip_path(&removed.proc.path)).retain(|&p| p != id);
			Some(WorldEdit::Remove {
				id,
				keys: world_keys(&entry)
			})
		};
		return Ok(PreparedRemoval {
			id,
			type_id: NONE,
			field: type_fields::PROCS,
			map_entries: vec![],
			relinked: vec![],
			next,
			on_world
		});
	}

	let type_id = find_type(&removed.type_path).map_err(fail)?;
	let field = if removed.verb { type_fields::VERBS } else { type_fields::PROCS };
	let keys = if removed.verb {
		vec![]
	} else {
		vec![entry.0[ProcEntryWords::NAME], entry.0[ProcEntryWords::PATH_BASENAME]]
	};

	let mut map_entries = vec![];
	let mut relinked = vec![];
	for sub in 0..runtime_tables::type_count().map_err(fail)? {
		if !type_and_ancestors(sub).contains(&type_id) {
			continue;
		}
		for (k, &key) in keys.iter().enumerate() {
			if k == 1 && key == keys[0] {
				continue;
			}
			let mut procs = staged.named_procs(sub, key).map_err(fail)?;
			let before = procs.len();
			procs.retain(|&p| p != id);
			if procs.len() != before {
				staged.maps.insert((sub, key), procs.clone());
				map_entries.push((sub, key, procs));
			}
		}
		for proc in staged.own(sub, field).map_err(fail)? {
			if proc != id && staged.entry(proc).is_some_and(|e| e.0[ProcEntryWords::NEXT_OVERRIDE] == id) && !relinked.contains(&proc) {
				relinked.push(proc);
			}
		}
	}

	for &proc in &relinked {
		if let Some(e) = staged.entries.get_mut(&proc) {
			e.0[ProcEntryWords::NEXT_OVERRIDE] = next;
		} else {
			staged.next_override.insert(proc, next);
		}
	}
	staged.removed.insert(id);

	Ok(PreparedRemoval {
		id,
		type_id,
		field,
		map_entries,
		relinked,
		next,
		on_world: None
	})
}

fn prepare_new_proc(
	new: &NewProc,
	id: u32,
	pending: &Pending,
	world_globals: &mut WorldGlobals,
	added: &mut AddedVars,
	staged: &mut StagedProcs
) -> Result<PreparedNewProc, String> {
	let shown = describe(&new.code.proc);
	let fail = |e: String| format!("{}: {}", shown, e);

	let template = proc_template().map_err(fail)?;

	let bytecode = assemble(&shown, &new.code, None, pending, world_globals, added)?;

	let string = |text: &str| StringRef::new(text).map_err(|_| fail(format!("couldn't make the string {}", text)));

	let path = new_proc_path(new);

	let (path, basename) = (string(&path)?, string(&new.name)?);
	let mut entry = template;
	entry.0[ProcEntryWords::PATH] = path.get_id().0;
	entry.0[ProcEntryWords::PATH_BASENAME] = basename.get_id().0;
	let settings = new.code.settings.clone().unwrap_or_default();
	let settings = settings_words(&entry, &settings, &new.name).map_err(fail)?;
	for setting in &settings {
		entry.0[setting.word] = setting.value;
	}
	entry.0[ProcEntryWords::BYTECODE] = runtime_tables::intern_misc(&bytecode).map_err(fail)?;
	entry.0[ProcEntryWords::LOCALS] = locals_table(&new.code.locals, template.0[ProcEntryWords::LOCALS]).map_err(fail)?;
	entry.0[ProcEntryWords::PARAMETERS] = if new.code.params.is_empty() {
		template.0[ProcEntryWords::PARAMETERS]
	} else {
		params_table(&new.code.params, NONE, pending, world_globals, added)
			.map_err(fail)?
			.unwrap()
	};
	entry.0[ProcEntryWords::NEXT_OVERRIDE] = NONE;

	let at = new.code.proc.override_id as usize;
	let mut on_world = None;
	if new.type_path == "/world" && !new.verb {
		let definitions = staged.world_definitions(&strip_path(&new.code.proc.path));
		let at = at.min(definitions.len());
		let below = at.checked_sub(1).map_or(NONE, |below| definitions[below]);
		let new_ids: Vec<u32> = staged.entries.keys().copied().collect();
		let definitions = staged.world_definitions(&strip_path(&new.code.proc.path));
		let above = definitions[at..].iter().copied().find(|p| !new_ids.contains(p));
		definitions.insert(at, id);
		entry.0[ProcEntryWords::NEXT_OVERRIDE] = below;
		staged.entries.insert(id, entry);
		on_world = Some(WorldEdit::Add {
			id,
			keys: world_keys(&entry),
			below,
			above
		});
	}

	let on_type = if new.type_path.is_empty() || on_world.is_some() {
		None
	} else if new.verb {
		Some(new_verb_on_type(&new.type_path, id, at, &mut entry, staged).map_err(fail)?)
	} else {
		let type_id = find_type(&new.type_path).map_err(fail)?;
		let keys = [entry.0[ProcEntryWords::NAME], entry.0[ProcEntryWords::PATH_BASENAME]];

		let own = own_definitions(staged, type_id, type_fields::PROCS, ProcEntryWords::NAME, keys[0]).map_err(fail)?;
		let overridden = match (own.get(at), at.checked_sub(1).and_then(|below| own.get(below))) {
			(Some(_), Some(&below)) => below,
			(Some(&first), None) => staged.entry(first).map_or(NONE, |e| e.0[ProcEntryWords::NEXT_OVERRIDE]),
			(None, _) => staged.named_procs(type_id, keys[0]).map_err(fail)?.first().copied().unwrap_or(NONE)
		};
		entry.0[ProcEntryWords::NEXT_OVERRIDE] = overridden;

		let mut map_entries = vec![];
		let mut relinked = vec![];
		for sub in 0..runtime_tables::type_count().map_err(fail)? {
			let line = type_and_ancestors(sub);
			if !line.contains(&type_id) {
				continue;
			}

			for (k, &key) in keys.iter().enumerate() {
				if k == 1 && key == keys[0] {
					continue;
				}
				let mut procs = staged.named_procs(sub, key).map_err(fail)?;
				let at = procs.iter().position(|&p| p == overridden).unwrap_or(procs.len());
				procs.insert(at, id);
				map_entries.push((sub, key, procs));
			}

			for proc in staged.own(sub, type_fields::PROCS).map_err(fail)? {
				if let Some(e) = staged.entry(proc) {
					if e.0[ProcEntryWords::NAME] == keys[0] && e.0[ProcEntryWords::NEXT_OVERRIDE] == overridden && !relinked.contains(&proc) {
						relinked.push(proc);
					}
				}
			}
		}

		for (sub, key, procs) in &map_entries {
			staged.maps.insert((*sub, *key), procs.clone());
		}
		for &proc in &relinked {
			if let Some(e) = staged.entries.get_mut(&proc) {
				e.0[ProcEntryWords::NEXT_OVERRIDE] = id;
			} else {
				staged.next_override.insert(proc, id);
			}
		}
		staged.entries.insert(id, entry);
		staged.own.entry((type_id, type_fields::PROCS)).or_default().push(id);

		let mut paths = vec![];
		for &proc in relinked.iter().filter(|_| new.declared) {
			let Some(path) = staged.entry(proc).map(|e| e.0[ProcEntryWords::PATH]) else {
				continue;
			};
			let path = String::from(unsafe { StringRef::from_id(raw_types::strings::StringId(path)) });
			if let Some(at) = path.rfind("/proc/") {
				let renamed = new_string(&format!("{}/{}", &path[..at], &path[at + "/proc/".len()..])).map_err(fail)?;
				match staged.entries.get_mut(&proc) {
					Some(e) => {
						e.0[ProcEntryWords::PATH] = renamed.get_id().0;
						staged.strings.push(renamed);
					}
					None => paths.push((proc, renamed))
				}
			}
		}

		Some(NewProcOnType {
			type_id,
			field: type_fields::PROCS,
			map_entries,
			relinked,
			paths,
			after: None
		})
	};

	Ok(PreparedNewProc {
		entry,
		path,
		basename,
		settings,
		id,
		place: new.code.proc.clone(),
		local_names: new.code.locals.clone(),
		on_type,
		on_world
	})
}

fn add_to_list(list: &mut Vec<u32>, field: usize, proc: u32) {
	if list.contains(&proc) {
		return;
	}
	if field == type_fields::VERBS {
		list.insert(0, proc);
	} else {
		list.push(proc);
	}
}

fn new_verb_on_type(type_path: &str, id: u32, at: usize, entry: &mut ProcEntryWords, staged: &mut StagedProcs) -> Result<NewProcOnType, String> {
	let type_id = find_type(type_path)?;
	let name = entry.0[ProcEntryWords::PATH_BASENAME];

	let own = own_definitions(staged, type_id, type_fields::VERBS, ProcEntryWords::PATH_BASENAME, name)?;
	let below = match at.checked_sub(1).and_then(|below| own.get(below)) {
		Some(&below) => below,
		None => {
			let mut overridden = NONE;
			for t in type_and_ancestors(type_id).into_iter().skip(1) {
				if let Some(&newest) = own_definitions(staged, t, type_fields::VERBS, ProcEntryWords::PATH_BASENAME, name)?.last() {
					overridden = newest;
					break;
				}
			}
			overridden
		}
	};
	entry.0[ProcEntryWords::NEXT_OVERRIDE] = below;

	let mut relinked = vec![];
	let after = own.get(at).copied();
	match after {
		Some(above) => relinked.push(above),
		None => {
			for sub in 0..runtime_tables::type_count()? {
				if sub == type_id || !type_and_ancestors(sub).contains(&type_id) {
					continue;
				}
				for proc in staged.own(sub, type_fields::VERBS)? {
					let calls_below = staged
						.entry(proc)
						.is_some_and(|e| e.0[ProcEntryWords::PATH_BASENAME] == name && e.0[ProcEntryWords::NEXT_OVERRIDE] == below);
					if below != NONE && calls_below && !relinked.contains(&proc) {
						relinked.push(proc);
					}
				}
			}
		}
	}

	for &proc in &relinked {
		if let Some(e) = staged.entries.get_mut(&proc) {
			e.0[ProcEntryWords::NEXT_OVERRIDE] = id;
		} else {
			staged.next_override.insert(proc, id);
		}
	}
	staged.entries.insert(id, *entry);
	staged.own.entry((type_id, type_fields::VERBS)).or_default().push(id);
	if let Some(after) = after {
		staged.listed_after.insert(id, after);
	}

	Ok(NewProcOnType {
		type_id,
		field: type_fields::VERBS,
		map_entries: vec![],
		relinked,
		paths: vec![],
		after
	})
}

fn world_keys(entry: &ProcEntryWords) -> Vec<u32> {
	let mut keys = vec![entry.0[ProcEntryWords::NAME]];
	if entry.0[ProcEntryWords::PATH_BASENAME] != keys[0] {
		keys.push(entry.0[ProcEntryWords::PATH_BASENAME]);
	}
	keys
}

fn new_statics(
	code: &ProcReload,
	proc_globals: &ProcGlobals,
	pending: &Pending,
	added: &mut AddedVars
) -> Result<HashMap<(StaticScope, Vec<u8>), u32>, String> {
	let mut out = HashMap::new();
	for local in &code.statics {
		let name = local.name.as_bytes().to_vec();
		let nth = match &local.scope {
			StaticScope::Proc { nth, .. } => *nth,
			_ => 0
		};
		let existing = proc_globals.local(&name, nth);
		if let (true, Some(id), Some(value)) = (local.constant, existing, &local.value) {
			let (value, text) = owned_value(value, pending).map_err(|e| format!("local const {}: {}", local.name, e))?;
			added.const_values.push((id, value, text));
			out.insert((local.scope.clone(), name), id);
			continue;
		}
		let value = match (&local.value, &local.init) {
			(Some(value), _) => owned_value(value, pending).map_err(|e| format!("static local {}: {}", local.name, e))?,
			(None, Some(_)) => ((0, 0), None),
			(None, None) => return Err(format!("static local {} has no initial value to start with", local.name))
		};
		let id = added.add(new_string(&local.name)?, value);
		if let (None, Some(init)) = (&local.value, &local.init) {
			added
				.run_once
				.push((id, format!("static local {}", local.name), (**init).clone(), HashMap::new()));
		}
		if matches!(local.scope, StaticScope::Proc { .. }) {
			added.proc_locals.push(id);
		}
		added.statics.insert((local.scope.clone(), name.clone()), id);
		out.insert((local.scope.clone(), name), id);
	}
	Ok(out)
}

pub fn add_types(new_types: &[NewType]) -> Result<usize, Vec<String>> {
	if new_types.is_empty() {
		return Ok(0);
	}
	runtime_tables::available().map_err(|e| vec![format!("hot reloading isn't available on this build of BYOND: {}", e)])?;
	runtime_tables::types_available().map_err(|e| vec![format!("adding types isn't available on this build of BYOND: {}", e)])?;
	code_changed();

	let refused: Vec<String> = {
		let unsettled = UNSETTLED_TYPES.lock().unwrap();
		new_types
			.iter()
			.filter_map(|new| {
				let (added, inherited) = unsettled.get(&new.path)?;
				if added != new {
					Some(format!(
						"{}: a reload that failed added this type with another parent or other built-in values. Undo the change to them, or rebuild \
						 the .dmb and restart the world",
						new.path
					))
				} else if !inherits_as_added(new, inherited) {
					Some(format!(
						"{}: a reload that failed added this type, and the values it inherits have changed since. Undo the change to them, or \
						 rebuild the .dmb and restart the world",
						new.path
					))
				} else {
					None
				}
			})
			.collect()
	};
	if !refused.is_empty() {
		return Err(refused);
	}

	let mut added = 0;
	for new in new_types {
		if find_type(&new.path).is_ok() {
			continue;
		}
		let inherited = add_type(new).map_err(|e| vec![format!("{}: {}", new.path, e)])?;
		UNSETTLED_TYPES.lock().unwrap().insert(new.path.clone(), (new.clone(), inherited));
		added += 1;
	}
	Ok(added)
}

static UNSETTLED_TYPES: Mutex<BTreeMap<String, (NewType, Inherited)>> = Mutex::new(BTreeMap::new());

#[derive(PartialEq)]
struct Inherited {
	record: Vec<u32>,
	builtins: Vec<u32>,
	mob: Option<(u32, u32, u8, u8)>
}

fn inherited(parent: u32) -> Result<Inherited, String> {
	use type_fields::*;
	let words = [
		NAME,
		DESC,
		ICON,
		ICON_STATE,
		TEXT,
		MAPTEXT,
		MAPTEXT_X,
		MAPTEXT_WIDTH,
		FLAGS,
		FLAGS_HIGH,
		DIR & !3
	]
	.into_iter()
	.chain((0x40..0x54).step_by(4))
	.chain([SUFFIX, LAYER]);
	let mut mob = None;
	let _ = runtime_tables::set_mob_type(parent, |m| mob = Some((m.key, m.sight, m.see_in_dark, m.see_invisible)));
	Ok(Inherited {
		record: words.map(|offset| runtime_tables::type_field(parent, offset)).collect::<Result<_, _>>()?,
		builtins: runtime_tables::misc_words(runtime_tables::type_field(parent, BUILTIN_VALUES)?).unwrap_or_default(),
		mob
	})
}

fn inherits_as_added(new: &NewType, inherited_then: &Inherited) -> bool {
	let (Ok(parent), Ok(type_id)) = (find_type(&new.parent), find_type(&new.path)) else {
		return false;
	};
	let shared = |field: usize| {
		let words = |t: u32| runtime_tables::type_field(t, field).ok().and_then(runtime_tables::misc_words);
		words(type_id).is_some() && words(type_id) == words(parent)
	};
	inherited(parent).is_ok_and(|now| now == *inherited_then)
		&& shared(type_fields::VAR_TABLE)
		&& shared(type_fields::DECLARED_VARS)
		&& runtime_tables::type_field(type_id, type_fields::INIT_PROC) == runtime_tables::type_field(parent, type_fields::INIT_PROC)
}

fn add_type(new: &NewType) -> Result<Inherited, String> {
	let parent = find_type(&new.parent)?;
	let mut writes = vec![];
	for (field, value) in &new.fields {
		writes.extend(builtin_writes(field, value).map_err(|e| format!("{}: {}", field, e))?);
	}
	let inherited = inherited(parent)?;
	let id = runtime_tables::add_type(parent, leaked_string(&new.path)?)?;
	let table = runtime_tables::misc_words(runtime_tables::type_field(id, type_fields::VAR_TABLE)?).unwrap_or_default();
	hold_table_strings(&[(id, table)], &[]);
	for place in TEXT_PLACES {
		if let Ok(TypeString::Made(text)) = type_string(parent, place) {
			set_type_string(id, place, TypeString::Made(text))?;
		}
	}
	for name in listed_held_by(parent) {
		inherit_listed(id, parent, name);
	}
	write_builtins(id, writes)?;
	Ok(inherited)
}

static TYPE_STRINGS: Mutex<BTreeMap<(u32, usize), u32>> = Mutex::new(BTreeMap::new());

static TABLE_STRINGS: Mutex<BTreeMap<(u32, u32), u32>> = Mutex::new(BTreeMap::new());

fn hold_table_strings(tables: &[(u32, Vec<u32>)], made: &[StringRef]) {
	let string = raw_types::values::ValueTag::String.0 as u32;
	let mut held = TABLE_STRINGS.lock().unwrap();
	let known: HashSet<u32> = held.values().copied().chain(made.iter().map(|text| text.get_id().0)).collect();
	let mut released = vec![];
	for (type_id, table) in tables {
		let now: BTreeMap<u32, u32> = (table.as_chunks::<4>().0.iter())
			.filter(|&&[_, _, tag, id]| tag == string && known.contains(&id))
			.map(|&[name, _, _, id]| (name, id))
			.collect();
		let before: Vec<(u32, u32)> = (held.range((*type_id, 0)..=(*type_id, u32::MAX)))
			.map(|(&(_, name), &id)| (name, id))
			.collect();
		for (name, id) in before {
			if now.get(&name) != Some(&id) {
				held.remove(&(*type_id, name));
				released.push(id);
			}
		}
		for (name, id) in now {
			if held.insert((*type_id, name), id).is_none() {
				std::mem::forget(unsafe { StringRef::from_id(raw_types::strings::StringId(id)) });
			}
		}
	}
	drop(held);
	released.into_iter().for_each(release_string);
}

static LISTED_STRINGS: Mutex<BTreeMap<(u32, u32), Option<u32>>> = Mutex::new(BTreeMap::new());

fn hold_listed(type_id: u32, name: u32, text: Option<Option<u32>>) {
	let mut held = LISTED_STRINGS.lock().unwrap();
	let released = match text {
		Some(text) => held.insert((type_id, name), text),
		None => held.remove(&(type_id, name))
	};
	drop(held);
	if let Some(text) = released {
		std::iter::once(name).chain(text).for_each(release_string);
	}
}

fn inherit_listed(type_id: u32, from: u32, name: u32) {
	let text = LISTED_STRINGS.lock().unwrap().get(&(from, name)).copied();
	if let Some(text) = text {
		for id in std::iter::once(name).chain(text) {
			std::mem::forget(unsafe { StringRef::from_id(raw_types::strings::StringId(id)) });
		}
	}
	hold_listed(type_id, name, text);
}

fn listed_held_by(type_id: u32) -> Vec<u32> {
	(LISTED_STRINGS.lock().unwrap().range((type_id, 0)..=(type_id, u32::MAX)))
		.map(|(&(_, name), _)| name)
		.collect()
}

const MOB_KEY: usize = usize::MAX;

const TEXT_PLACES: [usize; 7] = [
	type_fields::NAME,
	type_fields::DESC,
	type_fields::ICON_STATE,
	type_fields::TEXT,
	type_fields::MAPTEXT,
	type_fields::SUFFIX,
	MOB_KEY
];

#[derive(Clone, Debug)]
enum TypeString {
	Built(u32),
	Made(StringRef)
}

fn made_string(text: &str) -> Result<TypeString, String> {
	StringRef::new(text)
		.map(TypeString::Made)
		.map_err(|_| format!("couldn't make the string {}", text))
}

fn type_string(type_id: u32, place: usize) -> Result<TypeString, String> {
	let id = if place == MOB_KEY {
		let mut key = NONE;
		runtime_tables::set_mob_type(type_id, |mob| key = mob.key)?;
		key
	} else {
		runtime_tables::type_field(type_id, place)?
	};
	if TYPE_STRINGS.lock().unwrap().get(&(type_id, place)) == Some(&id) {
		Ok(TypeString::Made(unsafe { StringRef::from_id(raw_types::strings::StringId(id)) }))
	} else {
		Ok(TypeString::Built(id))
	}
}

fn set_type_string(type_id: u32, place: usize, string: TypeString) -> Result<(), String> {
	let id = match &string {
		TypeString::Built(id) => *id,
		TypeString::Made(text) => text.get_id().0
	};
	if place == MOB_KEY {
		runtime_tables::set_mob_type(type_id, |mob| mob.key = id)?;
	} else {
		runtime_tables::set_type_field(type_id, place, id)?;
	}
	let mut held = TYPE_STRINGS.lock().unwrap();
	let released = match string {
		TypeString::Built(_) => held.remove(&(type_id, place)),
		TypeString::Made(text) => {
			std::mem::forget(text);
			held.insert((type_id, place), id)
		}
	};
	drop(held);
	if let Some(released) = released {
		release_string(released);
	}
	Ok(())
}

#[derive(Clone, Debug)]
enum BuiltinWrite {
	Word(usize, u32),
	Text(usize, TypeString),
	Half(usize, u16),
	Byte(usize, u8),
	Bits(usize, u32, u32),
	Listed(StringRef, Vec<u32>, Option<StringRef>),
	Mob(&'static str, u32)
}

fn builtin_writes(var: &str, value: &ConstValue) -> Result<Vec<BuiltinWrite>, String> {
	use BuiltinWrite::*;
	let number = || match value {
		ConstValue::Number(n) => Ok(*n),
		ConstValue::Null => Ok(0.0),
		_ => Err("expected a number".to_owned())
	};
	let string = || match value {
		ConstValue::Null => Ok(TypeString::Built(NONE)),
		ConstValue::String(text) => made_string(text),
		_ => Err("expected text".to_owned())
	};
	let bits = |offset: usize, mask: u32, on: bool| Bits(offset, mask, if on { mask } else { 0 });
	Ok(match var {
		"name" => vec![Text(type_fields::NAME, string()?)],
		"desc" => vec![Text(type_fields::DESC, string()?)],
		"icon_state" => vec![Text(type_fields::ICON_STATE, string()?)],
		"text" => vec![Text(type_fields::TEXT, string()?)],
		"suffix" => vec![Text(type_fields::SUFFIX, string()?)],
		"maptext" => vec![Text(type_fields::MAPTEXT, string()?)],
		"icon" => vec![Word(type_fields::ICON, match value {
			ConstValue::Null => NONE,
			ConstValue::Resource(file) => AssembleEnv
				.get_resource_index(file)
				.ok_or_else(|| format!("couldn't find the file {}", file))?,
			_ => return Err("expected a resource file".to_owned())
		})],
		"layer" => vec![Word(type_fields::LAYER, number()?.to_bits())],
		"dir" => vec![Byte(type_fields::DIR, number()? as u8)],
		"maptext_x" => vec![Half(type_fields::MAPTEXT_X, number()? as i16 as u16)],
		"maptext_y" => vec![Half(type_fields::MAPTEXT_Y, number()? as i16 as u16)],
		"maptext_width" => vec![Half(type_fields::MAPTEXT_WIDTH, number()? as u16)],
		"maptext_height" => vec![Half(type_fields::MAPTEXT_HEIGHT, number()? as u16)],
		"opacity" => vec![bits(type_fields::FLAGS, 0x1, number()? != 0.0)],
		"density" => vec![bits(type_fields::FLAGS, 0x2, number()? != 0.0)],
		"mouse_drop_zone" => vec![bits(type_fields::FLAGS, 0x100, number()? != 0.0)],
		"gender" => {
			let gender = match value {
				ConstValue::String(g) if g == "neuter" => 0,
				ConstValue::String(g) if g == "male" => 1,
				ConstValue::String(g) if g == "female" => 2,
				ConstValue::String(g) if g == "plural" => 3,
				_ => return Err("has to be \"neuter\", \"male\", \"female\" or \"plural\"".to_owned())
			};
			vec![Bits(type_fields::FLAGS, 0xC0, gender << 6)]
		}
		"mouse_opacity" => vec![Bits(type_fields::FLAGS, 0x3000, ((number()? as i32 - 1) as u32 & 3) << 12)],
		"animate_movement" => {
			let bits = match number()? as i32 {
				0 => 0xC400,
				1 => 0,
				2 => 0x4000,
				3 | -1 => 0x8000,
				_ => return Err("has to be -1, 0, 1, 2 or 3".to_owned())
			};
			vec![Bits(type_fields::FLAGS, 0xC400, bits)]
		}
		"appearance_flags" => {
			let flags = number()? as u32;
			if flags >= 1 << 12 {
				return Err("flags cannot be stored in the type record".to_owned());
			}
			vec![
				Bits(type_fields::FLAGS, 0x7FF << 21, (flags & 0x7FF) << 21),
				Bits(type_fields::FLAGS_HIGH, 1, flags >> 11),
			]
		}
		"alpha"
		| "blend_mode"
		| "color"
		| "invisibility"
		| "infra_luminosity"
		| "luminosity"
		| "pixel_x"
		| "pixel_y"
		| "pixel_w"
		| "pixel_z"
		| "plane"
		| "render_source"
		| "render_target"
		| "bound_x"
		| "bound_y"
		| "bound_width"
		| "bound_height"
		| "step_size"
		| "step_x"
		| "step_y"
		| "glide_size"
		| "screen_loc"
		| "vis_flags"
		| "mouse_drag_pointer"
		| "mouse_drop_pointer"
		| "mouse_over_pointer"
		| "override"
		| "icon_w"
		| "icon_z"
		| "bounds"
		| "fps"
		| "view"
		| "perspective"
		| "tick_lag"
		| "default_verb_category"
		| "edge_limit"
		| "mouse_pointer_icon" => {
			let text = match value {
				ConstValue::String(text) => Some(StringRef::new(text).map_err(|_| format!("couldn't make the string {}", text))?),
				_ => None
			};
			let (tag, data) = match &text {
				Some(text) => (0x06, text.get_id().0),
				None => const_value(value, &Pending::default())?
			};
			let words = if tag == 0x2A {
				vec![tag, data >> 16, data & 0xFFFF]
			} else {
				vec![tag | ((data & 0xFF_0000) >> 8), data & 0xFFFF]
			};
			let name = StringRef::new(var).map_err(|_| format!("couldn't make the string {}", var))?;
			vec![Listed(name, words, text)]
		}
		"key" => vec![Text(MOB_KEY, string()?)],
		"sight" => vec![Mob("sight", number()? as u32)],
		"see_in_dark" => vec![Mob("see_in_dark", number()? as u32)],
		"see_invisible" => vec![Mob("see_invisible", number()? as u32)],
		_ => return Err("built-in variable cannot be changed by hot reload".to_owned())
	})
}

fn write_builtins(type_id: u32, writes: Vec<BuiltinWrite>) -> Result<(), String> {
	use runtime_tables::{set_type_byte, set_type_field, set_type_half, type_field};
	for write in writes {
		match write {
			BuiltinWrite::Word(offset, word) => set_type_field(type_id, offset, word)?,
			BuiltinWrite::Text(place, text) => set_type_string(type_id, place, text)?,
			BuiltinWrite::Half(offset, half) => set_type_half(type_id, offset, half)?,
			BuiltinWrite::Byte(offset, byte) => set_type_byte(type_id, offset, byte)?,
			BuiltinWrite::Bits(offset, mask, bits) => {
				let word = type_field(type_id, offset)?;
				set_type_field(type_id, offset, (word & !mask) | bits)?;
			}
			BuiltinWrite::Listed(name, value, text) => {
				let name_id = name.get_id().0;
				let list = runtime_tables::misc_words(type_field(type_id, type_fields::BUILTIN_VALUES)?).unwrap_or_default();
				let mut words = vec![];
				let mut at = 0;
				while at < list.len() {
					let len = if list.get(at + 1).is_some_and(|tag| tag & 0xFF == 0x2A) { 4 } else { 3 };
					let entry = &list[at..(at + len).min(list.len())];
					if entry[0] != name_id {
						words.extend_from_slice(entry);
					}
					at += len;
				}
				words.push(name_id);
				words.extend_from_slice(&value);
				set_type_field(type_id, type_fields::BUILTIN_VALUES, runtime_tables::intern_misc(&words)?)?;
				let text_id = text.as_ref().map(|text| text.get_id().0);
				std::mem::forget(name);
				std::mem::forget(text);
				hold_listed(type_id, name_id, Some(text_id));
			}
			BuiltinWrite::Mob(var, value) => runtime_tables::set_mob_type(type_id, |mob| match var {
				"sight" => mob.sight = value,
				"see_in_dark" => mob.see_in_dark = value as u8,
				_ => mob.see_invisible = value as u8
			})?
		}
	}
	Ok(())
}

fn const_value(value: &ConstValue, pending: &Pending) -> Result<(u32, u32), String> {
	Ok(match value {
		ConstValue::Null => (0, 0),
		ConstValue::Number(n) => (0x2A, n.to_bits()),
		ConstValue::String(text) => (0x06, leaked_string(text)?),
		ConstValue::Path(path) => match pending.proc_named(path) {
			Named::Proc(id) => (raw_types::values::ValueTag::ProcRef.0 as u32, id),
			Named::Unchanged => {
				let (tag, data) = AssembleEnv
					.get_type(path)
					.ok_or_else(|| format!("there's no type {} in the running world", path))?;
				(tag as u32, data)
			}
			Named::Removed => {
				let (tag, data) = removed_path_type(path).ok_or_else(|| removed_proc(path))?;
				(tag as u32, data)
			}
		},
		ConstValue::Resource(file) => (
			0x0C,
			AssembleEnv
				.get_resource_index(file)
				.ok_or_else(|| format!("couldn't find the file {}", file))?
		)
	})
}

fn owned_value(value: &ConstValue, pending: &Pending) -> Result<((u32, u32), Option<StringRef>), String> {
	match value {
		ConstValue::String(text) => {
			let text = new_string(text)?;
			Ok(((raw_types::values::ValueTag::String.0 as u32, text.get_id().0), Some(text)))
		}
		value => Ok((const_value(value, pending)?, None))
	}
}

fn release_string(id: u32) {
	unsafe {
		raw_types::funcs::dec_ref_count(raw_types::values::Value {
			tag: raw_types::values::ValueTag::String,
			data: raw_types::values::ValueData { id }
		});
	}
}

#[cfg(feature = "test-commands")]
pub fn string_references(text: &str) -> Result<u32, String> {
	let string = StringRef::new(text).map_err(|e| e.message)?;
	let mut entry = std::ptr::null_mut();
	unsafe {
		if raw_types::funcs::get_string_table_entry(&mut entry, string.get_id()) != 1 || entry.is_null() {
			return Err(format!("there's no string table entry for {}", text));
		}
		Ok((*entry).ref_count - 1)
	}
}

#[cfg(feature = "test-commands")]
pub fn locals_references(path: &str) -> Result<(u32, u16, u32), String> {
	let proc = Proc::find(path).ok_or_else(|| format!("there's no proc {}", path))?;
	let id = runtime_tables::proc_entry(proc.id)
		.ok_or_else(|| format!("couldn't read the table entry of {}", path))?
		.0[ProcEntryWords::LOCALS];
	let (count, references) = misc_references(id).ok_or_else(|| format!("{} has no locals table", path))?;
	Ok((id, count, references))
}

#[cfg(feature = "test-commands")]
pub fn declared_references(path: &str) -> Result<(u32, u32), String> {
	let id = runtime_tables::type_field(find_type(path)?, type_fields::DECLARED_VARS)?;
	let (_, references) = misc_references(id).ok_or_else(|| format!("{} has no declared vars", path))?;
	Ok((id, references))
}

#[cfg(feature = "test-commands")]
fn misc_references(id: u32) -> Option<(u16, u32)> {
	let mut entry = std::ptr::null_mut();
	unsafe {
		if raw_types::funcs::get_misc_by_id(&mut entry, raw_types::misc::MiscId::from_raw(id)) != 1 || entry.is_null() {
			return None;
		}
		Some((*(entry as *const u16), *((entry as *const u8).add(4) as *const u32)))
	}
}

fn new_string(text: &str) -> Result<StringRef, String> {
	StringRef::new(text).map_err(|_| format!("couldn't make the string {}", text))
}

fn leaked_string(text: &str) -> Result<u32, String> {
	let string = new_string(text)?;
	let id = string.get_id().0;
	std::mem::forget(string);
	Ok(id)
}

fn prepare_new_var(
	new: &NewVar,
	added: &mut AddedVars,
	type_vars: &mut TypeVars,
	pending: &mut Pending,
	warnings: &mut Vec<String>
) -> Result<(), String> {
	const STATIC: u32 = 1;
	let fail = |e: String| format!("{}/var/{}: {}", new.type_path, new.var, e);

	if !new.type_path.is_empty() {
		let type_id = find_type(&new.type_path).map_err(fail)?;
		let name = new_string(&new.var).map_err(fail)?;
		let declared = type_vars.declared(type_id).map_err(fail)?.clone();
		if let Some(pair) = declared.chunks(2).find(|pair| type_vars.name_of(pair[0]) == name.get_id().0) {
			if pair[1] != new.flags {
				return Err(fail(
					"the running world still has this var with different static, const or tmp flags. Rebuild to change them".to_owned()
				));
			}
			if new.flags & STATIC != 0 {
				pending
					.globals
					.insert((StaticScope::of_type(&new.type_path), new.var.as_bytes().to_vec()), pair[0]);
			}
			warnings.push(format!(
				"{}: var {} uses the declaration that the running world still has. Existing static values do not change",
				new.type_path, new.var
			));
			return Ok(());
		}
	}

	let id = added.next_id();
	let (value, text) = owned_value(&new.value, pending).map_err(fail)?;
	let name_ref = new_string(&new.var).map_err(fail)?;
	let name = name_ref.get_id().0;
	type_vars.new_names.insert(id, name);
	if let Some(init) = &new.init {
		added
			.run_once
			.push((id, format!("{}/var/{}", new.type_path, new.var), (**init).clone(), HashMap::new()));
	}

	if !new.type_path.is_empty() {
		let type_id = find_type(&new.type_path).map_err(fail)?;
		let moved_from = new
			.moved_from
			.iter()
			.map(|path| find_type(path))
			.collect::<Result<Vec<u32>, String>>()
			.map_err(fail)?;
		let mut kept = vec![];
		for sub in type_vars.subtree(type_id).map_err(fail)? {
			let declared = type_vars.declared(sub).map_err(fail)?.clone();
			let names: Vec<u32> = declared.chunks(2).map(|pair| type_vars.name_of(pair[0])).collect();
			match names.binary_search(&name) {
				Ok(_) if type_and_ancestors(sub).iter().any(|t| moved_from.contains(t)) => {
					kept.push(sub);
					continue;
				}
				Ok(_) => {
					return Err(fail(format!(
						"{} already has a var of that name. This is a duplicate definition",
						type_path(sub)
					)))
				}
				Err(at) => {
					type_vars.declared(sub).map_err(fail)?.splice(at * 2..at * 2, [id, new.flags]);
				}
			}

			if new.flags & STATIC == 0 {
				let table = type_vars.table(sub).map_err(fail)?;
				let at = table.chunks(4).position(|entry| entry[0] >= name).unwrap_or(table.len() / 4);
				if table.get(at * 4) == Some(&name) {
					return Err(fail(format!(
						"{} already has a var of that name. This is a duplicate definition",
						type_path(sub)
					)));
				}
				table.splice(at * 4..at * 4, [name, id, value.0, value.1]);
			}
		}
		if let Some(warning) = moved_var_warning(&new.type_path, &new.var, &kept) {
			warnings.push(warning);
		}
	}

	if new.type_path.is_empty() || new.flags & STATIC != 0 {
		pending
			.globals
			.insert((StaticScope::of_type(&new.type_path), new.var.as_bytes().to_vec()), id);
	}

	added.vars.push(PreparedNewVar {
		id,
		name: name_ref,
		value,
		text
	});
	Ok(())
}

fn moved_var_warning(declaring: &str, var: &str, kept: &[u32]) -> Option<String> {
	let tops: Vec<String> = kept
		.iter()
		.filter(|&&t| !type_and_ancestors(t).iter().skip(1).any(|a| kept.contains(a)))
		.map(|&t| type_path(t))
		.collect();
	if tops.is_empty() {
		return None;
	}
	let (they, them) = if tops.len() == 1 { ("its", "it") } else { ("their", "them") };
	Some(format!(
		"{}: var {} moved here. In the running world, {} and {} subtypes keep their declarations, values and var IDs. Rebuild to give {} the new \
		 declaration",
		declaring,
		var,
		tops.join(", "),
		they,
		them
	))
}

fn prepare_redeclared(
	var: &Redeclared,
	shadowing: &[&ShadowingProc],
	added: &mut AddedVars,
	type_vars: &mut TypeVars,
	world_globals: &mut WorldGlobals,
	pending: &mut Pending,
	global_values: &mut Vec<(u32, (u32, u32), Option<StringRef>)>
) -> Result<(), String> {
	const STATIC: u32 = 1;
	let fail = |e: String| format!("{}/var/{}: {}", var.type_path, var.var, e);

	if var.type_path.is_empty() {
		let mut ids = variable_ids(var.var.as_bytes());
		if ids.len() > 1 {
			for type_id in 0..runtime_tables::type_count().map_err(fail)? {
				let declared = type_vars.declared_now(type_id).map_err(fail)?;
				ids.retain(|id| !declared.chunks(2).any(|pair| pair[0] == *id));
			}
			let proc_locals = PROC_LOCALS.lock().unwrap();
			ids.retain(|id| !proc_locals.contains(id));
			drop(proc_locals);
			let mut global_reads = vec![];
			for shadowing in shadowing {
				let Ok(proc) = find(&shadowing.proc) else {
					continue;
				};
				let read = world_globals.of(&proc).ids_of(var.var.as_bytes());
				if shadowing.reads.iter().all(Option::is_some) {
					ids.retain(|id| !read.contains(id));
				} else if let Some((own, globals)) = ProcGlobals::told_apart(&read, &shadowing.reads) {
					ids.retain(|id| !own.values().any(|own| own == id));
					added.proc_locals.extend(own.values());
					global_reads.extend(globals);
				}
			}
			let read: Vec<u32> = ids.iter().copied().filter(|id| global_reads.contains(id)).collect();
			if let [id] = read[..] {
				ids = vec![id];
			}
		}
		let [id] = ids[..] else {
			return Err(fail(format!("expected one matching runtime variable, found {}", ids.len())));
		};
		if let Some(init) = &var.init {
			added.run_once.push((id, format!("/var/{}", var.var), (**init).clone(), HashMap::new()));
		} else if let Some(value) = &var.value {
			let (value, text) = owned_value(value, pending).map_err(fail)?;
			global_values.push((id, value, text));
		}
		return Ok(());
	}

	let type_id = find_type(&var.type_path).map_err(fail)?;
	let name_string = new_string(&var.var).map_err(fail)?;
	let name = name_string.get_id().0;
	let declared = type_vars.declared(type_id).map_err(fail)?.clone();
	let id = declared
		.chunks(2)
		.map(|pair| pair[0])
		.find(|&id| type_vars.name_of(id) == name)
		.ok_or_else(|| fail("variable not found in the running world".to_owned()))?;

	for sub in type_vars.subtree(type_id).map_err(fail)? {
		if let Some(pair) = type_vars.declared(sub).map_err(fail)?.chunks_mut(2).find(|pair| pair[0] == id) {
			pair[1] = var.flags;
		}
		let table = type_vars.table(sub).map_err(fail)?;
		let at = table.chunks(4).position(|entry| entry[0] >= name).unwrap_or(table.len() / 4);
		let there = table.get(at * 4) == Some(&name);
		if var.flags & STATIC != 0 {
			if there {
				table.drain(at * 4..at * 4 + 4);
			}
		} else if !there {
			table.splice(at * 4..at * 4, [name, id, 0, 0]);
		}
	}

	if var.flags & STATIC != 0 {
		pending
			.globals
			.insert((StaticScope::of_type(&var.type_path), var.var.as_bytes().to_vec()), id);
		if let Some(init) = &var.init {
			added
				.run_once
				.push((id, format!("{}/var/{}", var.type_path, var.var), (**init).clone(), HashMap::new()));
		} else if let Some(value) = &var.value {
			let (value, text) = owned_value(value, pending).map_err(fail)?;
			global_values.push((id, value, text));
		}
	}
	Ok(())
}

fn prepare_var_default(
	default: &VarDefault,
	pending: &Pending,
	type_vars: &mut TypeVars,
	added: &mut AddedVars,
	strings: &mut Vec<StringRef>
) -> Result<(), String> {
	let fail = |e: String| format!("var {}: {}", default.var, e);

	let value = match &default.value {
		ConstValue::String(text) => {
			let string = StringRef::new(text).map_err(|_| fail(format!("couldn't make the string {}", text)))?;
			let id = string.get_id().0;
			strings.push(string);
			(raw_types::values::ValueTag::String.0 as u32, id)
		}
		value => const_value(value, pending).map_err(fail)?
	};
	let key = StringRef::new(&default.var)
		.map_err(|_| fail("couldn't make its name".to_owned()))?
		.get_id()
		.0;

	for path in &default.types {
		let type_id = find_type(path).map_err(fail)?;
		let table = type_vars.table(type_id).map_err(fail)?;
		match table.chunks_mut(4).find(|entry| entry[0] == key) {
			Some(entry) => {
				entry[2] = value.0;
				entry[3] = value.1;
			}
			None if inherit::builtin_in_table(type_id, &default.var) => {
				let id = match variable_ids(default.var.as_bytes()).first() {
					Some(&id) => id,
					None => added.add(new_string(&default.var).map_err(fail)?, ((0, 0), None))
				};
				let at = table.chunks(4).position(|entry| entry[0] > key).unwrap_or(table.len() / 4);
				table.splice(at * 4..at * 4, [key, id, value.0, value.1]);
			}
			None => return Err(fail(format!("variable not found on runtime type {}", path)))
		}
	}
	Ok(())
}

pub struct Appended {
	prepared: Prepared,
	procs_before: u32,
	init_procs: Vec<u32>,
	added: Added
}

#[derive(Default)]
struct Added {
	vars: usize,
	procs: usize,
	prefabs: usize,
	in_lists: usize
}

impl Added {
	fn unused(&self) -> Option<String> {
		let counted = |n: usize, one: &str, many: &str| (n > 0).then(|| format!("{} {}", n, if n == 1 { one } else { many }));
		let parts: Vec<String> = [
			counted(self.vars, "var", "vars"),
			counted(self.procs, "proc", "procs"),
			counted(self.prefabs, "prefab", "prefabs"),
			counted(self.in_lists, "`in` list", "`in` lists")
		]
		.into_iter()
		.flatten()
		.collect();
		(!parts.is_empty()).then(|| format!("new entries remain allocated but unused: {}", parts.join(", ")))
	}
}

impl Appended {
	pub fn unused(&self) -> Option<String> {
		self.added.unused()
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "test-commands"), allow(dead_code))]
pub enum FailAt {
	Append,
	Change,
	Copy
}

#[cfg(feature = "test-commands")]
static FAIL_NEXT: Mutex<Option<FailAt>> = Mutex::new(None);

#[cfg(feature = "test-commands")]
pub fn fail_next(at: FailAt) {
	*FAIL_NEXT.lock().unwrap() = Some(at);
}

fn failing(at: FailAt) -> Result<(), String> {
	#[cfg(feature = "test-commands")]
	if FAIL_NEXT.lock().unwrap().take_if(|next| *next == at).is_some() {
		return Err("asked for with #fail-reload".to_owned());
	}
	let _ = at;
	Ok(())
}

pub fn append(prepared: Prepared) -> Result<Appended, Vec<String>> {
	let mut added = Added::default();
	let procs_before = runtime_tables::proc_count();
	let appended = append_entries(&prepared, &mut added);
	added.procs = (runtime_tables::proc_count() - procs_before) as usize;
	match appended {
		Ok(init_procs) => Ok(Appended {
			prepared,
			procs_before,
			init_procs,
			added
		}),
		Err(e) => Err(std::iter::once(e).chain(added.unused()).collect())
	}
}

fn append_entries(prepared: &Prepared, added: &mut Added) -> Result<Vec<u32>, String> {
	for new in &prepared.new_vars {
		let id =
			runtime_tables::add_variable(new.name.get_id().0, new.value.0 as u8, new.value.1).map_err(|e| format!("couldn't add a var: {}", e))?;
		std::mem::forget(new.name.clone());
		std::mem::forget(new.text.clone());
		added.vars += 1;
		if id != new.id {
			return Err(format!("new variable ID mismatch: got {}, expected {}", id, new.id));
		}
	}
	for new in &prepared.new_procs {
		let id = failing(FailAt::Append)
			.and_then(|()| runtime_tables::append_proc(&new.entry))
			.map_err(|e| format!("couldn't add a proc: {}", e))?;
		let mut held = PROC_STRINGS.lock().unwrap();
		held.insert((id.0, ProcEntryWords::PATH), new.path.get_id().0);
		for setting in new.settings.iter().filter(|setting| setting.made.is_some()) {
			held.insert((id.0, setting.word), setting.value);
		}
		drop(held);
		std::mem::forget(new.basename.clone());
		if id.0 != new.id {
			return Err(format!("new proc ID mismatch: got {}, expected {}", id.0, new.id));
		}
	}
	for (expected, instance, entry, path) in &prepared.instances {
		let fail = |e: String| format!("couldn't add a prefab: {}", e);
		let init = runtime_tables::add_proc(entry).map_err(fail)?;
		std::mem::forget(path.clone());
		let id = runtime_tables::add_instance(runtime_tables::Instance { init: init.0, ..*instance }).map_err(fail)?;
		added.prefabs += 1;
		if id != *expected {
			return Err(format!("prefab ID mismatch: got {}, expected {}", id, expected));
		}
	}
	for (expected, entry) in &prepared.in_lists {
		let index = runtime_tables::add_proc(entry)
			.and_then(runtime_tables::add_in_list)
			.map_err(|e| format!("couldn't add an `in` list: {}", e))?;
		added.in_lists += 1;
		if index != *expected {
			return Err(format!("input list index mismatch: got {}, expected {}", index, expected));
		}
	}
	let mut init_procs = vec![];
	for init in &prepared.init_procs {
		init_procs.push(match &init.proc {
			None => NONE,
			Some((entry, _)) => {
				let id = runtime_tables::add_proc(entry).map_err(|e| format!("couldn't add an init proc: {}", e))?;
				std::mem::forget(init.path.clone());
				id.0
			}
		});
	}
	if !prepared.new_globals.is_empty() {
		let pairs: Vec<(u32, u32)> = prepared.new_globals.iter().map(|&(_, id, flags)| (id, flags)).collect();
		runtime_tables::add_global_vars(&pairs).map_err(|e| format!("the new global vars are missing from global.vars: {}", e))?;
	}
	Ok(init_procs)
}

pub fn apply<K: Claimant, T>(
	appended: Appended,
	breakpoints: &mut HashMap<(raw_types::procs::ProcId, u16), BTreeMap<K, T>>,
	warnings: &mut Vec<String>,
	failures: &mut Vec<String>
) {
	let Appended {
		mut prepared,
		procs_before,
		init_procs,
		added: _
	} = appended;
	code_changed();
	let mut unsettled = UNSETTLED_TYPES.lock().unwrap();
	for path in &prepared.new_types {
		unsettled.remove(path);
	}
	drop(unsettled);
	reparent::apply_tree(&mut prepared.reparent);
	PROC_LOCALS.lock().unwrap().extend(&prepared.proc_locals);
	ADDED_GLOBALS
		.lock()
		.unwrap()
		.extend(prepared.new_globals.iter().map(|(name, id, _)| (name.clone(), *id)));

	for removal in &prepared.removed_procs {
		if let Some(edit) = &removal.on_world {
			if let Err(e) = edit_world_names(edit.clone()) {
				failures.push(format!("could not unlink world proc: {}", e));
			}
		}
		if removal.type_id == NONE {
			continue;
		}
		let procs = runtime_tables::type_field(removal.type_id, removal.field).unwrap_or(NONE);
		let mut list = runtime_tables::misc_words(procs).unwrap_or_default();
		list.retain(|&proc| proc != removal.id);
		let list = if list.is_empty() { Ok(NONE) } else { runtime_tables::intern_misc(&list) };
		match list {
			Ok(list) => {
				let _ = runtime_tables::set_type_field(removal.type_id, removal.field, list);
			}
			Err(e) => failures.push(format!("could not unlink proc from type list: {}", e))
		}
		for (sub, key, procs) in &removal.map_entries {
			if let Err(e) = runtime_tables::set_named_procs(*sub, *key, procs) {
				failures.push(format!("could not unlink proc from type name map: {}", e));
			}
		}
		for &proc in &removal.relinked {
			runtime_tables::set_proc_entry_word(raw_types::procs::ProcId(proc), ProcEntryWords::NEXT_OVERRIDE, removal.next);
		}
	}

	for new in &prepared.new_procs {
		let id = new.id;
		mark_reloaded(id, Reloaded::Added);
		if let Some(on_type) = &new.on_type {
			let procs = runtime_tables::type_field(on_type.type_id, on_type.field).unwrap_or(NONE);
			let mut list = runtime_tables::misc_words(procs).unwrap_or_default();
			match on_type.after.and_then(|after| list.iter().position(|&p| p == after)) {
				Some(at) if !list.contains(&id) => list.insert(at + 1, id),
				_ => add_to_list(&mut list, on_type.field, id)
			}
			match runtime_tables::intern_misc(&list) {
				Ok(list) => {
					let _ = runtime_tables::set_type_field(on_type.type_id, on_type.field, list);
				}
				Err(e) => failures.push(format!("couldn't add a proc to its type's list: {}", e))
			}

			for (sub, key, procs) in &on_type.map_entries {
				if let Err(e) = runtime_tables::set_named_procs(*sub, *key, procs) {
					failures.push(format!("couldn't add a proc to a type's proc names: {}", e));
				}
			}

			for &proc in &on_type.relinked {
				runtime_tables::set_proc_entry_word(raw_types::procs::ProcId(proc), ProcEntryWords::NEXT_OVERRIDE, id);
			}
			for (proc, path) in &on_type.paths {
				set_setting(*proc, SettingWord {
					word: ProcEntryWords::PATH,
					value: path.get_id().0,
					made: Some(path.clone())
				});
			}
		}

		if let Some(edit) = &new.on_world {
			if let Err(e) = edit_world_names(edit.clone()) {
				failures.push(format!("couldn't add a proc to the world's procs: {}", e));
			}
		}

		if let Some(proc) = Proc::from_id(raw_types::procs::ProcId(id)) {
			let (ptr, _) = unsafe { proc.bytecode_mut_ptr() };
			auxtools::debug::set_local_names(&proc, ptr, new.local_names.clone());
		}
	}
	for (init, id) in prepared.init_procs.iter().zip(init_procs) {
		if let Some((_, local_names)) = &init.proc {
			if let Some(proc) = Proc::from_id(raw_types::procs::ProcId(id)) {
				let (ptr, _) = unsafe { proc.bytecode_mut_ptr() };
				auxtools::debug::set_local_names(&proc, ptr, local_names.clone());
			}
		}
		let _ = runtime_tables::set_type_field(init.type_id, type_fields::INIT_PROC, id);
	}

	if runtime_tables::proc_count() > procs_before {
		Proc::add_to_table(raw_types::procs::ProcId(procs_before));
	}
	renumber(&prepared.removed_procs, &prepared.new_procs);

	let mut renames = prepared.renames;
	let mut unhooked = vec![];
	for p in prepared.code {
		renames.extend(p.rename);
		unhooked.extend(apply_code(p, breakpoints, failures));
	}
	for code in unhooked {
		restore_breakpoints(code, breakpoints, warnings);
	}
	for r in renames {
		if let Err(e) = rename(r) {
			failures.push(format!("could not update dynamic lookup for renamed proc: {}", e));
		}
	}
	for (proc, setting) in prepared.entry_words {
		set_setting(proc, setting);
	}
	let mut var_tables = vec![];
	for (type_id, field, words) in prepared.type_fields {
		let _ = runtime_tables::set_type_field(type_id, field, words);
		if field == type_fields::VAR_TABLE {
			var_tables.push((type_id, runtime_tables::misc_words(words).unwrap_or_default()));
		}
	}
	hold_table_strings(&var_tables, &prepared.table_strings);
	reparent::apply_builtins(&prepared.reparent, warnings, failures);
	for (type_id, writes) in std::mem::take(&mut prepared.builtin_writes) {
		if let Err(e) = failing(FailAt::Change).and_then(|()| write_builtins(type_id, writes)) {
			failures.push(format!("couldn't set a built-in var: {}", e));
		}
	}
	inherit::apply(&prepared.builtin_inherits, failures);
	for &type_id in &prepared.appearances {
		let _ = runtime_tables::forget_type_appearance(type_id);
	}

	for (id, (tag, data), text) in prepared.global_values {
		match runtime_tables::set_global_words(id, tag as u8, data) {
			Ok(()) => std::mem::forget(text),
			Err(e) => failures.push(format!("couldn't set a static var's value: {}", e))
		}
	}

	if let Some(proc) = prepared.global_init_at_start {
		auxtools::start_runs_new_code(proc);
	}
	if let Some(bytecode) = prepared.global_init_now {
		match Proc::find("/proc/auxtools_expr_stub") {
			Some(stub) => match stub.set_bytecode(bytecode) {
				Ok(()) => {
					if let Err(e) = stub.call(&[]) {
						warnings.push(format!("(global init): {}", e.message));
					}
				}
				Err(e) => failures.push(format!("(global init): {}", e))
			},
			None => failures.push("(global init): missing /proc/auxtools_expr_stub to run it".to_owned())
		}
	}
	for (id, name, bytecode) in prepared.run_once {
		let Some(stub) = Proc::find("/proc/auxtools_expr_stub") else {
			failures.push(format!("{}: missing /proc/auxtools_expr_stub for initializer evaluation", name));
			continue;
		};
		if let Err(e) = stub.set_bytecode(bytecode) {
			failures.push(format!("{}: {}", name, e));
			continue;
		}
		let ends = crate::WORLD_ENDS.load(std::sync::atomic::Ordering::Relaxed);
		let tries = crate::server::Evaluation::hide_tries();
		let result = stub.call(&[]);
		if crate::WORLD_ENDS.load(std::sync::atomic::Ordering::Relaxed) == ends {
			crate::server::Evaluation::show_tries(tries);
		}
		match result {
			Ok(value) => {
				if let Err(e) = runtime_tables::set_global_value(id, value) {
					failures.push(format!("{}: {}", name, e));
				}
			}
			Err(e) => warnings.push(format!("{}: initializer evaluation failed: {}", name, e.message))
		}
	}
}

struct UnhookedCode<K, T> {
	code: PreparedCode,
	old_code: std::sync::Arc<crate::code_index::Code>,
	old_bytecode: Vec<u32>,
	old_file: Option<u32>,
	old_locals: Vec<String>,
	new_locals: Vec<String>,
	lines: Option<Vec<(u32, u32)>>,
	hooked: Vec<(u32, Option<u32>, Option<String>, Option<BTreeMap<K, T>>)>
}

fn apply_code<K: Claimant, T>(
	mut p: PreparedCode,
	breakpoints: &mut HashMap<(raw_types::procs::ProcId, u16), BTreeMap<K, T>>,
	failures: &mut Vec<String>
) -> Option<UnhookedCode<K, T>> {
	let (old_ptr, _) = unsafe { p.proc.bytecode_mut_ptr() };
	mark_reloaded(p.proc.id.0, if p.lines.is_some() { Reloaded::Lines } else { Reloaded::Code });

	if !auxtools::debug::has_local_names(&p.proc, old_ptr) {
		let names = auxtools::debug::local_names_for(&p.proc, old_ptr);
		auxtools::debug::set_local_names(&p.proc, old_ptr, names);
	}

	let hooked_at = hooked_offsets(&p.proc);
	let old_code = (!hooked_at.is_empty()).then(|| crate::code_index::of(&p.proc));
	let hooked: Vec<(u32, Option<u32>, Option<String>, Option<BTreeMap<K, T>>)> = match &old_code {
		Some(code) => hooked_at
			.into_iter()
			.map(|offset| {
				let other_file = (code.place(offset, false).and_then(|(file, _)| file))
					.filter(|&file| code.file().is_some_and(|own| own != file))
					.map(str::to_owned);
				let known = breakpoints.remove(&(p.proc.id, offset as u16));
				let _ = unhook_instruction(&p.proc, offset);
				(offset, code.line_at(offset), other_file, known)
			})
			.collect(),
		None => vec![]
	};

	let local_names = p
		.local_names
		.clone()
		.unwrap_or_else(|| auxtools::debug::local_names_for(&p.proc, old_ptr));
	let old_bytecode = original_code(&p.proc).into_owned();
	auxtools::remember_bytecode(old_ptr, &old_bytecode);
	let old_file = debug_file(&old_bytecode);
	let old_locals = auxtools::debug::local_names_for(&p.proc, old_ptr);
	let lines = match &p.lines {
		Some(lines) => Some(lines.clone()),
		None if !hooked.is_empty() && old_file == debug_file(&p.bytecode) => Some(kept_lines(&old_bytecode, &old_locals, &p.bytecode, &local_names)),
		None => None
	};

	runtime_tables::set_proc_entry_word(p.proc.id, ProcEntryWords::BYTECODE, p.bytecode_id);
	match p.locals {
		Some(LocalsTable::Made(locals)) => runtime_tables::set_proc_entry_word(p.proc.id, ProcEntryWords::LOCALS, locals),
		Some(LocalsTable::Shared(locals)) => {
			raw_types::misc::add_ref(&raw_types::misc::MiscId::from_raw(locals));
			runtime_tables::set_proc_entry_word(p.proc.id, ProcEntryWords::LOCALS, locals);
		}
		None => {}
	}
	if let Some(params) = p.params_id {
		runtime_tables::set_proc_entry_word(p.proc.id, ProcEntryWords::PARAMETERS, params);
	}
	for setting in std::mem::take(&mut p.settings) {
		set_setting(p.proc.id.0, setting);
	}

	let (new_ptr, _) = unsafe { p.proc.bytecode_mut_ptr() };
	let params: Vec<String> = p.proc.parameter_names().into_iter().map(String::from).collect();
	let names_at = if auxtools::debug::has_local_names(&p.proc, new_ptr)
		&& (auxtools::debug::local_names_for(&p.proc, new_ptr) != local_names || auxtools::debug::parameter_names_for(&p.proc, new_ptr) != params)
	{
		auxtools::copy_code(&p.proc).map(|()| unsafe { p.proc.bytecode_mut_ptr() }.0)
	} else {
		Ok(new_ptr)
	};
	let new_locals = local_names.clone();
	match names_at {
		Ok(at) => auxtools::debug::set_local_names(&p.proc, at, local_names),
		Err(e) => failures.push(format!("{}: new calls show the old names of its locals: {}", p.name, e))
	}
	crate::code_index::proc_changed(p.proc.id);

	let old_code = old_code.filter(|_| !hooked.is_empty())?;
	Some(UnhookedCode {
		code: p,
		old_code,
		old_bytecode,
		old_file,
		old_locals,
		new_locals,
		lines,
		hooked
	})
}

fn restore_breakpoints<K: Claimant, T>(
	unhooked: UnhookedCode<K, T>,
	breakpoints: &mut HashMap<(raw_types::procs::ProcId, u16), BTreeMap<K, T>>,
	warnings: &mut Vec<String>
) {
	let UnhookedCode {
		code: p,
		old_code,
		old_bytecode,
		old_file,
		old_locals,
		new_locals,
		lines,
		hooked
	} = unhooked;
	let new_code = crate::code_index::of(&p.proc);
	let firsts = match (old_code.lines().next(), new_code.lines().next()) {
		(Some(old_first), Some(new_first)) if old_file != debug_file(&p.bytecode) => Some((old_first, new_first)),
		_ => None
	};

	let (old_marks, new_marks) = (line_marks(&old_bytecode), line_marks(&p.bytecode));

	let new_entry = new_code.entry_offset();

	for (old_offset, line, other_file, known) in hooked {
		let (entry, known) = match known {
			Some(known) => {
				let (entry, rest): (BTreeMap<K, T>, BTreeMap<K, T>) = known.into_iter().partition(|(who, _)| who.on_entry());
				let rest = (entry.is_empty() || !rest.is_empty()).then_some(rest);
				(Some(entry).filter(|entry| !entry.is_empty()), rest)
			}
			None => (None, None)
		};
		if entry.is_some() {
			let place = "at the start of the proc";
			match hook_instruction(&p.proc, new_entry) {
				Ok(()) => keep_claims(entry, breakpoints, (p.proc.id, new_entry as u16), &p.name, place, warnings),
				Err(_) => warnings.push(format!("{}: could not restore breakpoint {}", p.name, place))
			}
			if known.is_none() {
				continue;
			}
		}

		let target = match line {
			None if new_code.line_at(old_offset).is_none() && new_code.instruction_length(old_offset).is_some() => {
				Ok((old_offset, format!("at instruction {}", old_offset)))
			}
			None => Err(format!(
				"{}: removed the breakpoint at instruction {}: the new code has no instruction there before its first line",
				p.name, old_offset
			)),
			Some(old_line) => {
				let new_line = |old_line: u32| {
					let line = match &lines {
						Some(lines) if other_file.is_none() => map_line(old_line, lines),
						_ => old_line
					};
					let moved = firsts
						.filter(|(old_first, _)| line >= *old_first)
						.map(|(old_first, new_first)| line - old_first + new_first);
					moved.unwrap_or(line)
				};
				let in_other_file = |file: &str| Some(file) == other_file.as_deref();
				let wanted = other_file.is_some().then_some(&in_other_file as &dyn Fn(&str) -> bool);
				let new_offset = |old_offset: u32| match old_code.line_at(old_offset) {
					Some(line) => {
						let (old_start, new_start) = (old_code.offset_of(line, wanted)?, new_code.offset_of(new_line(line), wanted)?);
						Some(jump_target(
							(&old_bytecode, &old_marks, old_start),
							old_offset,
							(&p.bytecode, &new_marks, new_start)
						))
					}
					None => Some(old_offset)
				};
				let line = new_line(old_line);
				new_code
					.offset_of(line, wanted)
					.map(|start| {
						let kept = (old_code.offset_of(old_line, wanted)).and_then(|old_start| {
							same_place(
								&old_code,
								(&old_bytecode, &old_locals, old_start),
								old_offset,
								(&p.bytecode, &new_locals, start),
								new_offset
							)
						});
						(kept.unwrap_or(start), format!("on line {}", line))
					})
					.ok_or_else(|| format!("{}: removed breakpoint on line {}: no executable code after reload", p.name, line))
			}
		};
		let (offset, place) = match target {
			Ok(target) => target,
			Err(warning) => {
				warnings.push(warning);
				continue;
			}
		};

		if hook_instruction(&p.proc, offset).is_err() {
			warnings.push(format!("{}: could not restore breakpoint {}", p.name, place));
			continue;
		}

		keep_claims(known, breakpoints, (p.proc.id, offset as u16), &p.name, &place, warnings);
	}
}

fn line_marks(code: &[u32]) -> Vec<u32> {
	let instructions = dmasm::scan(code).map(|scanned| scanned.instructions).unwrap_or_default();
	(instructions.iter())
		.filter(|ins| matches!(ins.opcode, Opcode::DbgLine | Opcode::DbgFile))
		.map(|ins| ins.offset)
		.collect()
}

fn jump_target(old: (&[u32], &[u32], u32), old_offset: u32, new: (&[u32], &[u32], u32)) -> u32 {
	let ((old, old_marks, old_start), (new, new_marks, new_start)) = (old, new);
	let from_start = new_start.wrapping_add(old_offset.wrapping_sub(old_start));
	fn words(code: &[u32], from: u32, to: u32) -> Option<&[u32]> {
		code.get(from as usize..to as usize)
	}
	if old_offset < old_start || words(old, old_start, old_offset) == words(new, new_start, from_start) {
		return from_start;
	}
	let end = |code: &[u32], marks: &[u32], start: u32| marks.iter().copied().find(|&mark| mark >= start).unwrap_or(code.len() as u32);
	let (old_end, new_end) = (end(old, old_marks, old_start), end(new, new_marks, new_start));
	let from_end = (old_end.checked_sub(old_offset))
		.filter(|&after| after > 0)
		.and_then(|after| new_end.checked_sub(after))
		.filter(|&at| at >= new_start && words(old, old_offset, old_end) == words(new, at, new_end));
	from_end.unwrap_or(from_start)
}

fn same_place(
	old_code: &crate::code_index::Code,
	old: (&[u32], &[String], u32),
	old_offset: u32,
	new: (&[u32], &[String], u32),
	new_offset: impl Fn(u32) -> Option<u32>
) -> Option<u32> {
	let ((old, old_locals, old_start), (new, new_locals, new_start)) = (old, new);
	let length = (old_offset.checked_sub(old_start)? as usize) + old_code.instruction_length(old_offset)?;
	let (old_scan, new_scan) = (dmasm::scan(old).ok()?, dmasm::scan(new).ok()?);
	let final_end = |scanned: &dmasm::ScannedBytecode| {
		(scanned.instructions.last())
			.filter(|ins| ins.opcode == Opcode::End)
			.map(|ins| ins.offset)
	};
	let (old_end, new_end) = (final_end(&old_scan), final_end(&new_scan));
	let mut old_words = named_words(old, old_locals, &old_scan.local_operands);
	for label in &old_scan.label_operands {
		if !(old_start..old_start + length as u32).contains(&label.word_offset) {
			continue;
		}
		let new_word = new.get((label.word_offset - old_start + new_start) as usize).copied();
		let target = match new_end.filter(|&end| Some(label.target) == old_end && new_word == Some(end)) {
			Some(end) => end,
			None => new_offset(label.target)?
		};
		old_words[label.word_offset as usize] = Word::Same(target);
	}
	let new_words = named_words(new, new_locals, &new_scan.local_operands);
	let (old_span, new_span) = (
		old_start as usize..old_start as usize + length,
		new_start as usize..new_start as usize + length
	);
	(old_words.get(old_span)? == new_words.get(new_span)?).then_some(new_start + (old_offset - old_start))
}

fn keep_claims<K: Ord, T>(
	known: Option<BTreeMap<K, T>>,
	breakpoints: &mut HashMap<(raw_types::procs::ProcId, u16), BTreeMap<K, T>>,
	at: (raw_types::procs::ProcId, u16),
	name: &str,
	place: &str,
	warnings: &mut Vec<String>
) {
	let Some(known) = known else {
		return;
	};
	let claims = breakpoints.entry(at).or_default();
	for (client, claim) in known {
		match claims.entry(client) {
			Entry::Vacant(entry) => {
				entry.insert(claim);
			}
			Entry::Occupied(_) => warnings.push(format!(
				"{}: two breakpoints {} moved to one instruction. Only the first keeps its condition and hit count",
				name, place
			))
		}
	}
}

fn adapt_debug_info(nodes: &[Node], original: &[u32]) -> Vec<Node> {
	let is_debug_info = |node: &Node| matches!(node, Node::Instruction(Instruction::DbgFile(_) | Instruction::DbgLine(_), _));

	if original.is_empty() && world_has_debug_info() {
		return nodes.to_vec();
	}
	if original.first() != Some(&Opcode::DbgFile.word()) || original.len() < 2 {
		return nodes.iter().filter(|node| !is_debug_info(node)).cloned().collect();
	}

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

static WAITING_WORLD_EDITS: Mutex<Vec<WorldEdit>> = Mutex::new(Vec::new());

static CODE_GENERATION: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn code_changed() {
	CODE_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

pub fn code_generation() -> u32 {
	CODE_GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

static ADDED_GLOBALS: Mutex<BTreeMap<String, u32>> = Mutex::new(BTreeMap::new());

static PROC_LOCALS: Mutex<BTreeSet<u32>> = Mutex::new(BTreeSet::new());

pub fn added_global(name: &str) -> Option<u32> {
	ADDED_GLOBALS.lock().unwrap().get(name).copied()
}

pub fn added_globals() -> Vec<String> {
	ADDED_GLOBALS.lock().unwrap().keys().cloned().collect()
}

static RELOADED: Mutex<BTreeMap<u32, Reloaded>> = Mutex::new(BTreeMap::new());

pub fn reloaded(id: u32) -> Option<Reloaded> {
	RELOADED.lock().unwrap().get(&id).copied()
}

fn mark_reloaded(id: u32, how: Reloaded) {
	let mut reloaded = RELOADED.lock().unwrap();
	let was = reloaded.entry(id).or_insert(how);
	*was = (*was).max(how);
}

static PINNED_WORLD_PROCS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

pub fn pin_world_proc(id: u32) {
	PINNED_WORLD_PROCS.lock().unwrap().push(id);
}

fn edit_world_names(edit: WorldEdit) -> Result<(), String> {
	if runtime_tables::world_names_built()? {
		return edit_built_world_names(&edit);
	}
	WAITING_WORLD_EDITS.lock().unwrap().push(edit);
	auxtools::at_proc_calls(make_waiting_world_edits);
	Ok(())
}

fn make_waiting_world_edits() -> bool {
	if !runtime_tables::world_names_built().unwrap_or(false) {
		return false;
	}
	for edit in std::mem::take(&mut *WAITING_WORLD_EDITS.lock().unwrap()) {
		if let Err(e) = edit_built_world_names(&edit) {
			eprintln!("auxtools: couldn't change the world's procs: {}", e);
		}
	}
	true
}

fn edit_built_world_names(edit: &WorldEdit) -> Result<(), String> {
	let set_next = |proc: u32, next: u32| runtime_tables::set_proc_entry_word(raw_types::procs::ProcId(proc), ProcEntryWords::NEXT_OVERRIDE, next);
	let next_of = |proc: u32| runtime_tables::proc_entry(raw_types::procs::ProcId(proc)).map_or(NONE, |e| e.0[ProcEntryWords::NEXT_OVERRIDE]);
	match edit {
		WorldEdit::Add { id, keys, below, above } => {
			let pinned = PINNED_WORLD_PROCS.lock().unwrap().clone();
			set_next(*id, *below);
			for (k, &key) in keys.iter().enumerate() {
				let mut procs = runtime_tables::world_named_procs(key)?;
				procs.retain(|proc| proc != id);
				let at = match above.and_then(|above| procs.iter().position(|&p| p == above)) {
					Some(at) => at + 1,
					None => procs.iter().take_while(|proc| pinned.contains(proc)).count()
				};
				if k == 0 {
					if let Some(&over) = at.checked_sub(1).and_then(|i| procs.get(i)) {
						set_next(over, *id);
					}
				}
				procs.insert(at, *id);
				runtime_tables::set_world_named_procs(key, &procs)?;
			}
		}
		WorldEdit::Remove { id, keys } => {
			let next = next_of(*id);
			for &key in keys {
				let mut procs = runtime_tables::world_named_procs(key)?;
				for &proc in &procs {
					if proc != *id && next_of(proc) == *id {
						set_next(proc, next);
					}
				}
				procs.retain(|proc| proc != id);
				runtime_tables::set_world_named_procs(key, &procs)?;
			}
		}
	}
	Ok(())
}

fn own_definitions(staged: &StagedProcs, type_id: u32, field: usize, word: usize, name: u32) -> Result<Vec<u32>, String> {
	let own: Vec<u32> = staged
		.own(type_id, field)?
		.into_iter()
		.filter(|&p| staged.entry(p).is_some_and(|e| e.0[word] == name))
		.collect();
	if field == type_fields::VERBS {
		return Ok(own.into_iter().rev().collect());
	}
	let next = |p: u32| staged.entry(p).map_or(NONE, |e| e.0[ProcEntryWords::NEXT_OVERRIDE]);
	let mut chain = vec![];
	let mut at = own.iter().copied().find(|&p| !own.iter().any(|&q| next(q) == p));
	while let Some(p) = at {
		chain.push(p);
		at = Some(next(p)).filter(|n| own.contains(n) && !chain.contains(n));
	}
	chain.reverse();
	Ok(chain)
}

fn renumber(removed: &[PreparedRemoval], added: &[PreparedNewProc]) {
	let mut paths: Vec<String> = vec![];
	let removed_ids: HashSet<u32> = removed.iter().map(|r| r.id).collect();
	let added_ids: HashSet<u32> = added.iter().map(|a| a.id).collect();
	for id in removed.iter().map(|r| r.id).chain(added_ids.iter().copied()) {
		if let Some(proc) = Proc::from_id(raw_types::procs::ProcId(id)) {
			if !paths.contains(&proc.path) {
				paths.push(proc.path.clone());
			}
		}
	}
	for path in paths {
		let mut procs: Vec<Proc> = Proc::overrides(path.as_str())
			.into_iter()
			.filter(|p| !removed_ids.contains(&p.id.0) && !added_ids.contains(&p.id.0))
			.collect();
		let mut new: Vec<&PreparedNewProc> = added.iter().filter(|a| strip_path(&a.place.path) == path).collect();
		new.sort_by_key(|a| a.place.override_id);
		for a in new {
			if let Some(proc) = Proc::from_id(raw_types::procs::ProcId(a.id)) {
				let at = (a.place.override_id as usize).min(procs.len());
				procs.insert(at, proc);
			}
		}
		Proc::set_overrides(path.as_str(), procs);
	}
}

fn proc_template() -> Result<ProcEntryWords, String> {
	Proc::find("/proc/auxtools_expr_stub")
		.or_else(|| Proc::find("/icon/proc/Width"))
		.and_then(|proc| runtime_tables::proc_entry(proc.id))
		.ok_or_else(|| "couldn't find a proc entry to copy (/proc/auxtools_expr_stub or /icon/proc/Width)".to_owned())
}

static WORLD_HAS_DEBUG_INFO: Mutex<Option<bool>> = Mutex::new(None);

fn world_has_debug_info() -> bool {
	let mut known = WORLD_HAS_DEBUG_INFO.lock().unwrap();
	*known.get_or_insert_with(|| {
		(0..)
			.map_while(|id| Proc::from_id(raw_types::procs::ProcId(id)))
			.any(|proc| original_code(&proc).first() == Some(&Opcode::DbgFile.word()))
	})
}

#[shutdown]
fn forget_world() {
	ADDED_GLOBALS.lock().unwrap().clear();
	PROC_LOCALS.lock().unwrap().clear();
	TYPE_STRINGS.lock().unwrap().clear();
	PROC_STRINGS.lock().unwrap().clear();
	TABLE_STRINGS.lock().unwrap().clear();
	LISTED_STRINGS.lock().unwrap().clear();
	UNSETTLED_TYPES.lock().unwrap().clear();
	RELOADED.lock().unwrap().clear();
	*WORLD_HAS_DEBUG_INFO.lock().unwrap() = None;
	WAITING_WORLD_EDITS.lock().unwrap().clear();
	PINNED_WORLD_PROCS.lock().unwrap().clear();
}

fn remap_lines(bytecode: &[u32], lines: &[(u32, u32)]) -> Option<Vec<u32>> {
	let scanned = dmasm::scan(bytecode).ok()?;
	let mut file = None;
	let mut out = bytecode.to_vec();
	let mut changed = false;
	let mut inside = true;

	for ins in scanned.instructions {
		let at = ins.offset as usize + 1;
		if ins.opcode == Opcode::DbgFile {
			inside = *file.get_or_insert(out[at]) == out[at];
		} else if ins.opcode == Opcode::DbgLine && inside {
			let line = map_line(out[at], lines);
			changed |= line != out[at];
			out[at] = line;
		}
	}

	changed.then_some(out)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Word<'a> {
	Same(u32),
	Local(Option<&'a str>),
	Jump
}

fn named_words<'a>(bytecode: &[u32], locals: &'a [String], local_operands: &[u32]) -> Vec<Word<'a>> {
	let mut words: Vec<Word> = bytecode.iter().map(|&word| Word::Same(word)).collect();
	for &at in local_operands {
		words[at as usize] = Word::Local(locals.get(bytecode[at as usize] as usize).map(String::as_str));
	}
	words
}

fn line_code<'a>(bytecode: &[u32], locals: &'a [String]) -> Vec<(Option<u32>, Vec<Word<'a>>)> {
	let Ok(mut scanned) = dmasm::scan(bytecode) else {
		return vec![];
	};
	let mut words = named_words(bytecode, locals, &scanned.local_operands);
	let mut targets = HashMap::new();
	for label in &scanned.label_operands {
		words[label.word_offset as usize] = Word::Jump;
		targets.insert(label.word_offset as usize, label.target as usize);
	}
	if scanned.instructions.last().is_some_and(|ins| ins.opcode == Opcode::End) {
		scanned.instructions.pop();
	}
	let (mut lines, mut file, mut inside) = (vec![], None, true);
	let mut spawn_ends = HashSet::new();
	for ins in scanned.instructions {
		let at = ins.offset as usize;
		let span = at..at + ins.bytecode.len();
		match ins.opcode {
			Opcode::DbgFile => inside = *file.get_or_insert(bytecode[at + 1]) == bytecode[at + 1],
			Opcode::DbgLine => lines.push((inside.then_some(bytecode[at + 1]), at, vec![])),
			Opcode::End if spawn_ends.contains(&span.end) && lines.last().is_some_and(|(_, _, code)| !code.is_empty()) => {}
			_ => {
				if ins.opcode == Opcode::Spawn {
					spawn_ends.extend(span.clone().filter_map(|word| targets.get(&word).copied()));
				}
				if let Some((_, _, code)) = lines.last_mut() {
					code.push((ins.opcode, span));
				}
			}
		}
	}
	let ends: Vec<usize> = (lines.iter().skip(1).map(|&(_, start, _)| start)).chain([bytecode.len()]).collect();
	(lines.iter().zip(ends))
		.map(|((line, start, code), end)| {
			let leaves = |span: &std::ops::Range<usize>| {
				span.clone()
					.any(|word| targets.get(&word).is_some_and(|target| !(*start..end).contains(target)))
			};
			let kept = (code.iter().enumerate().skip(1))
				.find(|(_, (opcode, span))| matches!(opcode, Opcode::Jmp | Opcode::JmpLoop) && leaves(span))
				.map_or(code.len(), |(i, _)| i);
			let words = code[..kept].iter().flat_map(|(_, span)| &words[span.clone()]).copied().collect();
			(*line, words)
		})
		.collect()
}

fn kept_lines(old: &[u32], old_locals: &[String], new: &[u32], new_locals: &[String]) -> Vec<(u32, u32)> {
	const MOST_CELLS: usize = 4_000_000;
	let (old, new) = (line_code(old, old_locals), line_code(new, new_locals));
	let same = |a: &(Option<u32>, Vec<Word>), b: &(Option<u32>, Vec<Word>)| a.0.is_some() == b.0.is_some() && a.1 == b.1;
	let prefix = old.iter().zip(&new).take_while(|(a, b)| same(a, b)).count();
	let suffix = (old[prefix..].iter().rev())
		.zip(new[prefix..].iter().rev())
		.take_while(|(a, b)| same(a, b))
		.count();
	let (old_mid, new_mid) = (&old[prefix..old.len() - suffix], &new[prefix..new.len() - suffix]);

	let mut pairs: Vec<(usize, usize)> = (0..prefix).map(|i| (i, i)).collect();
	if !old_mid.is_empty() && !new_mid.is_empty() && old_mid.len() * new_mid.len() <= MOST_CELLS {
		let width = new_mid.len() + 1;
		let mut common = vec![0u32; (old_mid.len() + 1) * width];
		for i in (0..old_mid.len()).rev() {
			for j in (0..new_mid.len()).rev() {
				common[i * width + j] = if same(&old_mid[i], &new_mid[j]) {
					common[(i + 1) * width + j + 1] + 1
				} else {
					common[(i + 1) * width + j].max(common[i * width + j + 1])
				};
			}
		}
		let (mut i, mut j) = (0, 0);
		while i < old_mid.len() && j < new_mid.len() {
			if same(&old_mid[i], &new_mid[j]) {
				pairs.push((prefix + i, prefix + j));
				i += 1;
				j += 1;
			} else if common[(i + 1) * width + j] >= common[i * width + j + 1] {
				i += 1;
			} else {
				j += 1;
			}
		}
	}
	pairs.extend((0..suffix).map(|i| (old.len() - suffix + i, new.len() - suffix + i)));

	let (old_paired, new_paired): (HashSet<usize>, HashSet<usize>) = pairs.iter().copied().unzip();
	let new_unique = unique_lines(&new, &new_paired);
	let mut moved: Vec<(usize, usize)> = (unique_lines(&old, &old_paired).into_iter())
		.filter_map(|(code, i)| Some((i, *new_unique.get(&code)?)))
		.collect();
	moved.sort_unstable();
	pairs.extend(moved);

	let mut lines = BTreeMap::new();
	for (i, j) in pairs {
		if let (Some(old_line), Some(new_line)) = (old[i].0, new[j].0) {
			lines.entry(old_line).or_insert(new_line);
		}
	}
	lines.into_iter().collect()
}

fn unique_lines<'b, 'a>(lines: &'b [(Option<u32>, Vec<Word<'a>>)], paired: &HashSet<usize>) -> HashMap<(bool, &'b [Word<'a>]), usize> {
	let mut found: HashMap<_, (usize, usize)> = HashMap::new();
	for (i, (line, code)) in lines.iter().enumerate() {
		found.entry((line.is_some(), code.as_slice())).or_insert((0, i)).0 += 1;
	}
	(found.into_iter())
		.filter(|(_, (count, i))| *count == 1 && !paired.contains(i))
		.map(|(code, (_, i))| (code, i))
		.collect()
}

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

fn debug_file(bytecode: &[u32]) -> Option<u32> {
	(bytecode.first() == Some(&Opcode::DbgFile.word()))
		.then(|| bytecode.get(1).copied())
		.flatten()
}

fn describe_assemble_error(e: &dmasm::assembler::AssembleError) -> String {
	use dmasm::assembler::AssembleError::*;
	let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
	match e {
		ProcNotFound(path) => format!("proc not found: {} in the running world", path),
		StringNotFound(s) => format!("couldn't make the string {:?}", text(s)),
		VariableNameNotFound(name) => format!("variable not found in the running world: {}", text(name)),
		TypeNotFound(path) => format!("type not found in the running world: {}", path),
		ResourceNotFound(path) => format!("couldn't find the file {}", path),
		InstanceNotFound(path) => format!("prefab instance not found: {}{{...}}", path),
		LabelNotFound(label) => format!("undefined label in compiled code: {}", label)
	}
}

fn variable_ids(name: &[u8]) -> Vec<u32> {
	let Ok(string) = StringRef::from_raw(name) else {
		return vec![];
	};
	let id = string.get_id();

	unsafe {
		let entries = *raw_types::funcs::VARIABLE_NAME_ENTRIES;
		(0..*raw_types::funcs::VARIABLE_NAME_COUNT)
			.filter(|&i| *entries.add(i as usize) == id)
			.collect()
	}
}

#[derive(Default)]
struct GlobalUses {
	files: Vec<Vec<u8>>,
	lines: Option<(u32, u32)>,
	uses: Vec<(Option<usize>, Vec<u8>, u32, u32)>
}

impl GlobalUses {
	fn read(bytecode: &[u32]) -> Self {
		let mut read = Self::default();
		let (nodes, _) = dmasm::disassembler::disassemble(bytecode, &mut LenientDisassembleEnv);
		let (mut file, mut line) = (None, 0);
		for node in &nodes {
			let Node::Instruction(ins, _) = node else { continue };
			match ins {
				dmasm::Instruction::DbgFile(name) => {
					file = Some(read.files.iter().position(|known| *known == name.0).unwrap_or_else(|| {
						read.files.push(name.0.clone());
						read.files.len() - 1
					}));
				}
				dmasm::Instruction::DbgLine(at) => {
					line = *at;
					read.lines = Some(read.lines.map_or((line, line), |(first, last)| (first.min(line), last.max(line))));
				}
				_ => {
					for var in ins.variables() {
						for global in var.globals() {
							if let Some(id) = global.id {
								read.uses.push((file, global.name.0.clone(), line, id));
							}
						}
					}
				}
			}
		}
		read
	}

	fn by_name(&self) -> HashMap<Vec<u8>, Vec<u32>> {
		let mut out: HashMap<Vec<u8>, Vec<u32>> = HashMap::new();
		for (_, name, _, id) in &self.uses {
			let ids = out.entry(name.clone()).or_default();
			if let Err(at) = ids.binary_search(id) {
				ids.insert(at, *id);
			}
		}
		out
	}

	fn ids_of(&self, name: &[u8]) -> Vec<u32> {
		self.uses.iter().filter(|(_, used, ..)| used == name).map(|(.., id)| *id).collect()
	}

	fn place(&self) -> Option<(Vec<u8>, u32, u32)> {
		let (first, last) = self.lines?;
		Some((self.files.first()?.clone(), first, last))
	}
}

#[derive(Default)]
struct ProcGlobals {
	by_name: HashMap<Vec<u8>, Vec<u32>>,
	locals: HashMap<Vec<u8>, HashMap<u32, u32>>,
	kept: HashMap<Vec<u8>, HashMap<u32, u32>>
}

impl ProcGlobals {
	fn new(uses: &GlobalUses, local_reads: &[LocalReads]) -> Self {
		let mut by_name = uses.by_name();
		let mut locals = HashMap::new();
		let mut kept = HashMap::new();
		for reads in local_reads {
			let name = reads.name.as_bytes().to_vec();
			kept.insert(name.clone(), reads.kept.iter().copied().collect());
			let ids = uses.ids_of(&name);
			let told = match reads.parsed {
				Some(ParsedReads::Only(nth)) => Self::only_local(&ids, nth),
				Some(ParsedReads::Unknown) => None,
				None => Self::told_apart(&ids, &reads.reads).or_else(|| {
					let [Some(nth), ..] = reads.reads[..] else {
						return None;
					};
					reads.reads.iter().all(|read| *read == Some(nth)).then(|| Self::only_local(&ids, nth))?
				})
			};
			match told {
				Some((own, globals)) => {
					by_name.insert(name.clone(), globals);
					locals.insert(name, own);
				}
				None => {
					by_name.remove(&name);
					locals.insert(name, HashMap::new());
				}
			}
		}
		Self { by_name, locals, kept }
	}

	fn only_local(ids: &[u32], nth: u32) -> Option<(HashMap<u32, u32>, Vec<u32>)> {
		let [id, ..] = ids[..] else {
			return None;
		};
		ids.iter().all(|other| *other == id).then(|| (HashMap::from([(nth, id)]), vec![]))
	}

	fn told_apart(ids: &[u32], reads: &[Option<u32>]) -> Option<(HashMap<u32, u32>, Vec<u32>)> {
		if ids.len() != reads.len() {
			return None;
		}
		let mut own: HashMap<u32, u32> = HashMap::new();
		let mut globals = vec![];
		for (id, read) in ids.iter().zip(reads) {
			match read {
				Some(nth) => {
					if *own.entry(*nth).or_insert(*id) != *id {
						return None;
					}
				}
				None => {
					if let Err(at) = globals.binary_search(id) {
						globals.insert(at, *id);
					}
				}
			}
		}
		(!own.values().any(|id| globals.contains(id))).then_some((own, globals))
	}

	fn local(&self, name: &[u8], nth: u32) -> Option<u32> {
		let old = self.kept.get(name)?.get(&nth)?;
		self.locals.get(name)?.get(old).copied()
	}
}

#[derive(Default)]
struct WorldGlobals {
	local_reads: HashMap<u32, Vec<LocalReads>>,
	users: Option<HashMap<Vec<u8>, Vec<(u32, String)>>>,
	initialized: Option<Vec<(Vec<u8>, Vec<u8>, u32, u32)>>,
	procs: HashMap<u32, (usize, u16, Rc<GlobalUses>)>
}

impl WorldGlobals {
	fn of(&mut self, proc: &Proc) -> Rc<GlobalUses> {
		let (at, len) = unsafe { proc.bytecode_mut_ptr() };
		let at = at as usize;
		if let Some((_, _, read)) = self.procs.get(&proc.id.0).filter(|(was_at, was_len, _)| *was_at == at && *was_len == len) {
			return read.clone();
		}
		let read = Rc::new(GlobalUses::read(&original_code(proc)));
		self.procs.insert(proc.id.0, (at, len, read.clone()));
		read
	}

	fn globals_of(&mut self, proc: &Proc) -> ProcGlobals {
		let uses = self.of(proc);
		ProcGlobals::new(&uses, self.local_reads.get(&proc.id.0).map_or(&[], Vec::as_slice))
	}

	fn initialized(&mut self) -> &[(Vec<u8>, Vec<u8>, u32, u32)] {
		if self.initialized.is_none() {
			let read = crate::unnamed::find(auxtools_protocol::GLOBAL_INIT, 0).map(|proc| self.of(&proc));
			let uses = read.iter().flat_map(|read| {
				read.uses.iter().map(|(file, name, line, id)| {
					let file = file.map_or_else(Vec::new, |file| read.files[file].clone());
					(file, name.clone(), *line, *id)
				})
			});
			self.initialized = Some(uses.collect());
		}
		self.initialized.as_deref().unwrap_or_default()
	}

	fn users(&mut self, name: &[u8]) -> &[(u32, String)] {
		let map = self.users.get_or_insert_with(|| {
			let mut map: HashMap<Vec<u8>, Vec<(u32, String)>> = HashMap::new();
			let mut id = 0;
			while let Some(proc) = Proc::from_id(raw_types::procs::ProcId(id)) {
				for (name, ids) in GlobalUses::read(&original_code(&proc)).by_name() {
					map.entry(name).or_default().extend(ids.into_iter().map(|id| (id, proc.path.clone())));
				}
				id += 1;
			}
			map
		});
		map.get(name).map_or(&[], |users| users.as_slice())
	}
}

struct ReloadEnv<'a> {
	proc_globals: ProcGlobals,
	world_globals: &'a mut WorldGlobals,
	pending: &'a Pending,
	new_statics: HashMap<(StaticScope, Vec<u8>), u32>,
	known: HashMap<(StaticScope, Vec<u8>), u32>,
	instances: &'a mut StagedInstances,
	place: Option<(Vec<u8>, u32, u32)>,
	error: Option<String>
}

impl ReloadEnv<'_> {
	fn proc_statics_read(&mut self, nodes: &[Node]) -> HashMap<(StaticScope, Vec<u8>), u32> {
		let mut found = HashMap::new();
		for node in nodes {
			let Node::Instruction(ins, _) = node else {
				continue;
			};
			for var in ins.variables() {
				for global in var.globals() {
					let Some(scope @ StaticScope::Proc { .. }) = &global.scope else {
						continue;
					};
					let id = dmasm::assembler::AssembleEnv::get_global_index(self, &global.name.0, Some(scope));
					self.error = None;
					if let Some(id) = id {
						found.insert((scope.clone(), global.name.0.clone()), id);
					}
				}
			}
		}
		found
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
		match self.pending.proc_named(path) {
			Named::Proc(id) => Some(id),
			Named::Unchanged => AssembleEnv.get_proc_index(path),
			Named::Removed => {
				self.error = Some(removed_proc(path));
				None
			}
		}
	}

	fn get_type(&mut self, path: &str) -> Option<(u8, u32)> {
		match self.pending.proc_named(path) {
			Named::Proc(id) => Some((raw_types::values::ValueTag::ProcRef.0, id)),
			Named::Unchanged => AssembleEnv.get_type(path),
			Named::Removed => {
				let found = removed_path_type(path);
				if found.is_none() {
					self.error = Some(removed_proc(path));
				}
				found
			}
		}
	}

	fn get_resource_index(&mut self, path: &str) -> Option<u32> {
		AssembleEnv.get_resource_index(path)
	}

	fn get_instance(&mut self, path: &str, init: &[u32]) -> Option<u32> {
		let Some(value) = AssembleEnv.get_type(path) else {
			self.error = Some(format!("there's no type {} in the running world", path));
			return None;
		};
		match self.instances.find_or_add(path, value, init) {
			Ok(id) => Some(id),
			Err(e) => {
				self.error = Some(format!("{}{{...}}: {}", path, e));
				None
			}
		}
	}

	fn get_global_index(&mut self, name: &[u8], scope: Option<&StaticScope>) -> Option<u32> {
		let shown = String::from_utf8_lossy(name).into_owned();
		let key = (scope.cloned().unwrap_or_default(), name.to_vec());
		if let Some(&id) = self.known.get(&key) {
			return Some(id);
		}

		if let Some(StaticScope::Proc { nth, .. }) = scope {
			let id = self.new_statics.get(&key).copied();
			let id = id.or_else(|| self.proc_globals.local(name, *nth));
			let id = id.or_else(|| {
				let (file, first, last) = self.place.as_ref()?;
				let set: Vec<u32> = self
					.world_globals
					.initialized()
					.iter()
					.filter(|(at, global, ..)| at == file && global == name)
					.filter(|(.., line, _)| (*first..=*last).contains(line))
					.map(|(.., id)| *id)
					.collect();
				match set[..] {
					[id] => Some(id),
					[] => match variable_ids(name)[..] {
						[id] => Some(id),
						_ => None
					},
					_ => None
				}
			});
			if id.is_none() {
				self.error = Some(format!(
					"static local {}: cannot find its storage in the running world. Rebuild to use it",
					shown
				));
			}
			return id;
		}

		if let Some(&id) = self.pending.globals.get(&key) {
			return Some(id);
		}

		let candidates = variable_ids(name);
		match candidates.len() {
			0 => {
				self.error = Some(format!("there's no global var {} in the running world", shown));
				return None;
			}
			1 => return Some(candidates[0]),
			_ => {}
		}

		if scope == Some(&StaticScope::Global) {
			if let Ok(globals) = runtime_tables::global_var_ids() {
				if let [id] = candidates.iter().filter(|id| globals.contains(id)).collect::<Vec<_>>()[..] {
					return Some(*id);
				}
			}
		}

		let declaring = match scope {
			Some(StaticScope::Type(path)) => Some(path.as_str()),
			_ => None
		};
		if let Some(id) = declaring.and_then(|ty| declared_global(ty, &shown)) {
			return Some(id);
		}

		if let Some([id]) = self.proc_globals.by_name.get(name).map(Vec::as_slice) {
			return Some(*id);
		}

		let users = self.world_globals.users(name);
		let ids: BTreeSet<u32> = users.iter().map(|(id, _)| *id).collect();
		if ids.len() == 1 {
			return ids.first().copied();
		}
		if let Some(ty) = declaring {
			let prefix = format!("{}/", ty);
			let ids: BTreeSet<u32> = users.iter().filter(|(_, path)| path.starts_with(&prefix)).map(|(id, _)| *id).collect();
			if ids.len() == 1 {
				return ids.first().copied();
			}
		}

		self.error = Some(format!(
			"ambiguous variable: {} runtime slots are named {}, so the name is not unique in {}",
			candidates.len(),
			shown,
			declaring.unwrap_or("the world")
		));
		None
	}
}

fn declared_global(path: &str, name: &str) -> Option<u32> {
	let type_id = find_type(path).ok()?;
	let name = StringRef::new(name).ok()?.get_id().0;
	let declared = runtime_tables::misc_words(runtime_tables::type_field(type_id, type_fields::DECLARED_VARS).ok()?)?;
	let found: Vec<u32> = declared
		.chunks(2)
		.filter(|c| c.len() == 2 && c[1] & 3 != 0 && runtime_tables::variable_name(c[0]) == Some(name))
		.map(|c| c[0])
		.collect();
	match found[..] {
		[id] => Some(id),
		_ => None
	}
}

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
	use super::{kept_lines, map_line, remap_lines};

	fn names(names: &[&str]) -> Vec<String> {
		names.iter().map(|name| name.to_string()).collect()
	}

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
		let dbgline = dmasm::Opcode::DbgLine.word();
		let push_int = dmasm::Opcode::PushInt.word();
		let ret = dmasm::Opcode::Ret.word();
		let bytecode = [dbgline, 10, push_int, 10, dbgline, 11, ret];
		let moved = remap_lines(&bytecode, &[(10, 15), (11, 16)]).unwrap();
		assert_eq!(moved, [dbgline, 15, push_int, 10, dbgline, 16, ret]);
		assert_eq!(remap_lines(&bytecode, &[(10, 10), (11, 11)]), None);
	}

	#[test]
	fn renumbers_the_lines_of_the_first_file_only() {
		let dbgfile = dmasm::Opcode::DbgFile.word();
		let dbgline = dmasm::Opcode::DbgLine.word();
		let ret = dmasm::Opcode::Ret.word();
		let bytecode = [dbgfile, 1, dbgline, 85, dbgfile, 2, dbgline, 21, dbgfile, 1, dbgline, 85, ret];
		let moved = remap_lines(&bytecode, &[(84, 86), (85, 87)]).unwrap();
		assert_eq!(moved, [dbgfile, 1, dbgline, 87, dbgfile, 2, dbgline, 21, dbgfile, 1, dbgline, 87, ret]);
	}

	#[test]
	fn keeps_lines_whose_code_differs_only_in_local_slots_and_jumps() {
		use dmasm::Opcode::*;
		const LOCAL: u32 = 0xFFDA;
		let (line, push, set, get, jmp, ret) = (DbgLine.word(), PushInt.word(), SetVar.word(), GetVar.word(), Jmp.word(), Ret.word());
		let old = [
			line, 10, push, 1, set, LOCAL, 0, line, 11, push, 2, set, LOCAL, 1, line, 12, get, LOCAL, 1, jmp, 7, ret
		];
		let new = [
			line, 10, push, 1, set, LOCAL, 0, line, 11, push, 0, set, LOCAL, 1, line, 12, push, 2, set, LOCAL, 2, line, 13, get, LOCAL, 2, jmp, 14,
			ret
		];
		assert_eq!(kept_lines(&old, &names(&["a", "b"]), &new, &names(&["a", "c", "b"])), [
			(10, 10),
			(11, 12),
			(12, 13)
		]);
	}

	#[test]
	fn keeps_both_of_two_swapped_lines() {
		use dmasm::Opcode::*;
		const LOCAL: u32 = 0xFFDA;
		let (line, push, set, ret) = (DbgLine.word(), PushInt.word(), SetVar.word(), Ret.word());
		let statement = |number: u32, value: u32| [line, number, push, value, set, LOCAL, 0];
		let code = |values: [u32; 4]| -> Vec<u32> {
			(values.iter().zip(10..))
				.flat_map(|(&value, number)| statement(number, value))
				.chain([ret])
				.collect()
		};
		assert_eq!(kept_lines(&code([1, 2, 3, 4]), &names(&["a"]), &code([1, 3, 2, 4]), &names(&["a"])), [
			(10, 10),
			(11, 12),
			(12, 11),
			(13, 13)
		]);
	}

	#[test]
	fn keeps_a_line_moved_to_the_end_of_a_loop_body() {
		use dmasm::Opcode::*;
		const LOCAL: u32 = 0xFFDA;
		let (line, push, set, get, jz, back, ret) = (
			DbgLine.word(),
			PushInt.word(),
			SetVar.word(),
			GetVar.word(),
			Jz.word(),
			JmpLoop.word(),
			Ret.word()
		);
		let header = [line, 10, push, 0, set, LOCAL, 0];
		let moved = |value: u32| [line, 0, push, value, set, LOCAL, 1];
		let guard = [line, 0, get, LOCAL, 0, jz, 28];
		let body = |first: [u32; 7], second: [u32; 7], third: [u32; 7]| -> Vec<u32> {
			let mut code: Vec<u32> = [header, first, second, third].concat();
			for (number, at) in [(11, 7), (12, 14), (13, 21)] {
				code[at + 1] = number;
			}
			code.extend([back, 2, line, 14, ret]);
			code
		};
		let old = body(moved(5), guard, moved(2));
		let new = body(guard, moved(2), moved(5));
		assert_eq!(kept_lines(&old, &names(&["a", "b"]), &new, &names(&["a", "b"])), [
			(10, 10),
			(11, 13),
			(12, 11),
			(13, 12),
			(14, 14)
		]);
	}

	#[test]
	fn keeps_a_line_moved_to_the_end_of_an_if_with_an_else() {
		use dmasm::Opcode::*;
		const LOCAL: u32 = 0xFFDA;
		let (line, push, set, get, jz, jmp, ret) = (
			DbgLine.word(),
			PushInt.word(),
			SetVar.word(),
			GetVar.word(),
			Jz.word(),
			Jmp.word(),
			Ret.word()
		);
		let header = [line, 10, push, 0, set, LOCAL, 0];
		let moved = |value: u32| [line, 0, push, value, set, LOCAL, 1];
		let guard = [line, 0, get, LOCAL, 0, jz, 30];
		let body = |first: [u32; 7], second: [u32; 7], third: [u32; 7]| -> Vec<u32> {
			let mut code: Vec<u32> = [header, first, second, third].concat();
			for (number, at) in [(11, 7), (12, 14), (13, 21)] {
				code[at + 1] = number;
			}
			code.extend([jmp, 37, line, 15, push, 3, set, LOCAL, 1, line, 16, get, LOCAL, 1, ret]);
			code
		};
		let old = body(moved(5), guard, moved(2));
		let new = body(guard, moved(2), moved(5));
		assert_eq!(kept_lines(&old, &names(&["a", "b"]), &new, &names(&["a", "b"])), [
			(10, 10),
			(11, 13),
			(12, 11),
			(13, 12),
			(15, 15),
			(16, 16)
		]);
	}

	#[test]
	fn keeps_a_line_moved_to_the_end_of_a_spawn_block() {
		use dmasm::Opcode::*;
		const LOCAL: u32 = 0xFFDA;
		let (line, push, set, add, sub, get, spawn, ret, end) = (
			DbgLine.word(),
			PushInt.word(),
			SetVar.word(),
			AugAdd.word(),
			AugSub.word(),
			GetVar.word(),
			Spawn.word(),
			Ret.word(),
			End.word()
		);
		let old = [
			line, 10, push, 1, set, LOCAL, 0, line, 11, push, 2, add, LOCAL, 0, line, 12, push, 1, sub, LOCAL, 0, line, 13, get, LOCAL, 0, ret, end
		];
		let new = [
			line, 10, push, 1, set, LOCAL, 0, line, 11, push, 10, spawn, 21, line, 12, push, 2, add, LOCAL, 0, end, line, 13, push, 1, sub, LOCAL, 0,
			line, 14, get, LOCAL, 0, ret, end
		];
		assert_eq!(kept_lines(&old, &names(&["total"]), &new, &names(&["total"])), [
			(10, 10),
			(11, 12),
			(12, 13),
			(13, 14)
		]);
	}
}
