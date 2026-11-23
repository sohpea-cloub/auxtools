use std::{
	collections::{btree_map::Entry, BTreeMap, BTreeSet, HashMap, HashSet},
	rc::Rc,
	sync::Mutex
};

use auxtools::{
	debug, raw_types,
	runtime_tables::{self, type_fields, ProcEntryWords, NONE},
	shutdown, Proc, StringRef
};
use auxtools_protocol::{HotReloadRequest, Reloaded};
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
	entry_words: Vec<(u32, usize, u32)>,
	renames: Vec<Rename>,
	appearances: BTreeSet<u32>,
	builtin_writes: Vec<(u32, Vec<BuiltinWrite>)>,
	type_fields: Vec<(u32, usize, u32)>,
	run_once: Vec<(u32, String, Vec<u32>)>,
	global_values: Vec<(u32, (u32, u32))>,
	global_init_now: Option<Vec<u32>>,
	global_init_at_start: Option<raw_types::procs::ProcId>,
	in_lists: Vec<(u32, ProcEntryWords)>,
	instances: Vec<(u32, runtime_tables::Instance, ProcEntryWords)>,
	builtin_inherits: Vec<(u32, String)>,
	reparent: reparent::PreparedReparents
}

struct PreparedNewVar {
	id: u32,
	name: u32,
	value: (u32, u32)
}

struct AddedVars {
	first: u32,
	vars: Vec<PreparedNewVar>,
	param_names: HashMap<String, u32>,
	run_once: Vec<(u32, String, ProcReload)>,
	in_lists_before: u32,
	in_lists: Vec<(u32, ProcEntryWords)>,
	instances: StagedInstances,
	statics: HashMap<(StaticScope, Vec<u8>), u32>
}

#[derive(Default)]
struct StagedInstances {
	existing: Option<Vec<runtime_tables::Instance>>,
	new: Vec<(runtime_tables::Instance, ProcEntryWords, Vec<u32>)>
}

impl StagedInstances {
	fn find_or_add(&mut self, path: &str, (tag, data): (u8, u32), init: &[u32]) -> Result<u32, String> {
		let existing = match &mut self.existing {
			Some(existing) => existing,
			none => none.insert(unsafe { runtime_tables::instances() }?)
		};
		let runs_init = |proc: u32| {
			(unsafe { runtime_tables::proc_entry(raw_types::procs::ProcId(proc)) })
				.and_then(|entry| unsafe { runtime_tables::misc_words(entry.0[ProcEntryWords::BYTECODE]) })
				.is_some_and(|code| code == init)
		};
		if let Some(id) = existing
			.iter()
			.position(|i| i.tag == tag && i.data == data && i.init != NONE && runs_init(i.init))
		{
			return Ok(id as u32);
		}
		let count = existing.len() as u32;
		if let Some(id) = self.new.iter().position(|(i, _, code)| i.tag == tag && i.data == data && code == init) {
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
		entry.0[ProcEntryWords::PATH] = leaked_string(&format!("{}/proc/(map_init)", path))?;
		entry.0[ProcEntryWords::BYTECODE] = unsafe { runtime_tables::intern_misc(init) }?;
		self.new.push((runtime_tables::Instance { tag, data, init: NONE }, entry, init.to_vec()));
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
		let id = self.add(leaked_string(name)?, (0, 0));
		self.param_names.insert(name.to_owned(), id);
		Ok(id)
	}

	fn add(&mut self, name: u32, value: (u32, u32)) -> u32 {
		let id = self.next_id();
		self.vars.push(PreparedNewVar { id, name, value });
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
	world: HashMap<String, Vec<u32>>
}

impl StagedProcs {
	fn named_procs(&self, type_id: u32, key: u32) -> Result<Vec<u32>, String> {
		match self.maps.get(&(type_id, key)) {
			Some(procs) => Ok(procs.clone()),
			None => unsafe { runtime_tables::named_procs(type_id, key) }
		}
	}

	fn entry(&self, proc: u32) -> Option<ProcEntryWords> {
		if let Some(entry) = self.entries.get(&proc) {
			return Some(*entry);
		}
		let mut entry = unsafe { runtime_tables::proc_entry(raw_types::procs::ProcId(proc)) }?;
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
		let mut procs = unsafe { runtime_tables::misc_words(runtime_tables::type_field(type_id, field)?) }.unwrap_or_default();
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
	procs: HashMap<String, u32>,
	globals: HashMap<(StaticScope, Vec<u8>), u32>
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
				let id = unsafe { runtime_tables::type_field(type_id, field) }?;
				entry.insert(unsafe { runtime_tables::misc_words(id) }.ok_or_else(|| format!("couldn't read the vars of type {}", type_id))?)
			}
		})
	}

	fn table(&mut self, type_id: u32) -> Result<&mut Vec<u32>, String> {
		Self::words(&mut self.tables, type_id, type_fields::VAR_TABLE)
	}

	fn declared(&mut self, type_id: u32) -> Result<&mut Vec<u32>, String> {
		Self::words(&mut self.declared, type_id, type_fields::DECLARED_VARS)
	}

	fn name_of(&self, var_id: u32) -> u32 {
		self.new_names
			.get(&var_id)
			.copied()
			.or_else(|| unsafe { runtime_tables::variable_name(var_id) })
			.unwrap_or(u32::MAX)
	}

	fn subtree(&mut self, type_id: u32) -> Result<Vec<u32>, String> {
		if self.parents.is_none() {
			let count = unsafe { runtime_tables::type_count() }?;
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
				let id = if words.is_empty() {
					NONE
				} else {
					unsafe { runtime_tables::intern_misc(&words) }?
				};
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
	locals_id: Option<u32>,
	params_id: Option<u32>,
	settings: Vec<(usize, u32)>,
	rename: Option<Rename>,
	local_names: Option<Vec<String>>,
	lines: Option<Vec<(u32, u32)>>
}

struct PreparedInitProc {
	type_id: u32,
	proc: Option<(ProcEntryWords, Vec<String>)>
}

struct PreparedNewProc {
	entry: ProcEntryWords,
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
	paths: Vec<(u32, u32)>,
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

pub type MovedBreakpoints<K> = HashMap<(raw_types::procs::ProcId, u32, K), u32>;

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
		global_init
	} = request;
	let Plan {
		new_types: _,
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
	(unsafe { runtime_tables::available() }).map_err(|e| vec![format!("hot reloading isn't available on this build of BYOND: {}", e)])?;

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

	let mut prepared = Prepared::default();
	let mut errors = vec![];
	let mut world_globals = WorldGlobals::default();

	let mut pending = Pending::default();
	let first_id = unsafe { runtime_tables::proc_count() };
	let ids: Vec<u32> = (first_id..).take(new_procs.len()).collect();
	pending.procs = new_procs
		.iter()
		.zip(&ids)
		.map(|(new, &id)| (strip_path(&new.code.proc.path), id))
		.collect();
	let mut type_vars = TypeVars::default();
	let mut staged = StagedProcs::default();
	reparent::prepare(reparented, &mut type_vars, &mut staged, &mut prepared, &mut errors);

	let mut added = AddedVars {
		first: unsafe { runtime_tables::variable_count() },
		vars: vec![],
		param_names: HashMap::new(),
		run_once: vec![],
		in_lists_before: unsafe { runtime_tables::in_list_count() }.unwrap_or(0),
		in_lists: vec![],
		instances: StagedInstances::default(),
		statics: HashMap::new()
	};
	for new in new_vars {
		if let Err(e) = prepare_new_var(new, &mut added, &mut type_vars, &mut pending, warnings) {
			errors.push(e);
		}
	}
	for var in redeclared {
		if let Err(e) = prepare_redeclared(var, &mut added, &mut type_vars, &mut pending, &mut prepared.global_values) {
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
			let entry =
				unsafe { runtime_tables::proc_entry(proc.id) }.ok_or_else(|| format!("{}: couldn't read its table entry", describe(proc_ref)))?;
			let words = settings_words(&entry, &change.settings, &proc_ref.path)?;
			prepared.renames.extend(renamed(&proc, &entry, &words));
			Ok(words.into_iter().map(move |(word, value)| (proc.id.0, word, value)))
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
			match unsafe { runtime_tables::intern_misc(&bytecode) } {
				Ok(bytecode_id) => prepared.code.push(PreparedCode {
					proc,
					name: describe(&moved.proc),
					bytecode,
					bytecode_id,
					locals_id: None,
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
		if let Err(e) = prepare_var_default(default, &pending, &mut type_vars, &mut added) {
			errors.push(e);
		}
	}
	for default in builtin_defaults {
		let writes = builtin_writes(&default.var, &default.value).map_err(|e| format!("{}: {}", default.var, e));
		match writes {
			Ok(writes) => {
				let mut given = false;
				for path in &default.types {
					match find_type(path) {
						Ok(type_id) => {
							if given {
								writes.iter().filter_map(BuiltinWrite::string).for_each(keep_string);
							}
							given = true;
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

	for (id, name, init) in std::mem::take(&mut added.run_once) {
		let code = value_code(init);
		if !code.locals.is_empty() {
			errors.push(format!(
				"{}: its initial value needs local variables, which hot reload does not support yet",
				name
			));
			continue;
		}
		match assemble(&name, &code, None, &pending, &mut world_globals, &mut added) {
			Ok(bytecode) => prepared.run_once.push((id, name, bytecode)),
			Err(e) => errors.push(e)
		}
	}

	if !prepared.appearances.is_empty() {
		warnings.push("verb and built-in changes apply only to new instances. Existing instances keep their values".to_owned());
	}

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
		.map(|(n, (instance, entry, _))| (first_instance + n as u32, instance, entry))
		.collect();
	if !prepared.new_globals.is_empty() {
		if let Err(e) = unsafe { runtime_tables::global_vars_available() } {
			errors.push(format!(
				"new global vars can't be added to global.vars on this build of BYOND: {}. Rebuild to add them",
				e
			));
		}
	}
	let stub = Proc::find("/proc/auxtools_expr_stub").is_some() || pending.procs.contains_key(&strip_path("/proc/auxtools_expr_stub"));
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
	let proc_globals = uses.by_name();
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
	let entry = unsafe { runtime_tables::proc_entry(proc.id) }.ok_or_else(|| format!("{}: couldn't read its table entry", name))?;
	let bytecode_id = unsafe { runtime_tables::intern_misc(&bytecode) }.map_err(|e| format!("{}: {}", name, e))?;

	let locals_id = if reload.locals.len() == proc.locals_count() {
		None
	} else {
		Some(locals_table(&reload.locals, entry.0[ProcEntryWords::LOCALS]).map_err(|e| format!("{}: {}", name, e))?)
	};

	let params_id =
		params_table(&reload.params, entry.0[ProcEntryWords::PARAMETERS], pending, world_globals, added).map_err(|e| format!("{}: {}", name, e))?;

	let settings = match &reload.settings {
		Some(settings) => settings_words(&entry, settings, &reload.proc.path).map_err(|e| format!("{}: {}", name, e))?,
		None => vec![]
	};
	let rename = renamed(&proc, &entry, &settings);

	let (at, _) = unsafe { proc.bytecode_mut_ptr() };
	if entry.0[ProcEntryWords::BYTECODE] == bytecode_id && debug::local_names_for(&proc, at) != reload.locals {
		failing(FailAt::Copy).map_err(|e| format!("{}: {}", name, e))?;
		if bytecode.len() >= u16::MAX as usize {
			return Err(format!(
				"{}: renaming only its locals needs a copy of its code, and {} words leave no room for one. Rebuild to rename them",
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
		locals_id,
		params_id,
		settings,
		rename,
		local_names: Some(reload.locals.clone()),
		lines: None
	})
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
		first: unsafe { runtime_tables::variable_count() },
		vars: vec![],
		param_names: HashMap::new(),
		run_once: vec![],
		in_lists_before: 0,
		in_lists: vec![],
		instances: StagedInstances::default(),
		statics: HashMap::new()
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
					let globals = unsafe { runtime_tables::global_var_ids() }.ok()?;
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
	let types = unsafe { runtime_tables::type_count() }.ok()?;
	let a_types = (0..types).any(|type_id| unsafe { runtime_tables::type_field(type_id, type_fields::INIT_PROC) }.ok() == Some(proc.id.0));
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

fn settings_words(entry: &ProcEntryWords, settings: &ProcSettings, path: &str) -> Result<Vec<(usize, u32)>, String> {
	let string = |text: Option<&str>| text.map_or(Ok(NONE), leaked_string);
	let basename = path.rsplit('/').next().unwrap_or(path).replace('_', " ");
	let words = [
		(
			ProcEntryWords::FLAGS,
			(entry.0[ProcEntryWords::FLAGS] & !ProcSettings::FLAGS) | settings.flags()
		),
		(ProcEntryWords::SETTINGS, settings.packed()),
		(ProcEntryWords::NAME, string(Some(settings.name.as_deref().unwrap_or(&basename)))?),
		(ProcEntryWords::DESC, string(settings.desc.as_deref())?),
		(ProcEntryWords::CATEGORY, string(settings.category.as_deref())?)
	];
	Ok(words.into_iter().filter(|&(word, value)| entry.0[word] != value).collect())
}

#[derive(Clone, Copy)]
struct Rename {
	proc: u32,
	type_id: u32,
	from: u32,
	to: u32,
	basename: u32
}

fn renamed(proc: &Proc, entry: &ProcEntryWords, words: &[(usize, u32)]) -> Option<Rename> {
	let to = words.iter().find(|&&(word, _)| word == ProcEntryWords::NAME)?.1;
	let from = entry.0[ProcEntryWords::NAME];
	if from == NONE || from == to {
		return None;
	}
	let path = strip_path(&proc.path);
	let (type_path, _) = path.rsplit_once('/')?;
	let type_id = find_type(type_path).ok()?;
	let on_map = unsafe { runtime_tables::named_procs(type_id, from) }.is_ok_and(|procs| procs.contains(&proc.id.0));
	on_map.then_some(Rename {
		proc: proc.id.0,
		type_id,
		from,
		to,
		basename: entry.0[ProcEntryWords::PATH_BASENAME]
	})
}

fn rename(r: Rename) -> Result<(), String> {
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
	for sub in 0..unsafe { runtime_tables::type_count() }? {
		if !type_and_ancestors(sub).contains(&r.type_id) {
			continue;
		}
		if r.from != r.basename {
			let mut old = unsafe { runtime_tables::named_procs(sub, r.from) }?;
			if let Some(at) = old.iter().position(|&p| p == r.proc) {
				old.remove(at);
				(unsafe { runtime_tables::set_named_procs(sub, r.from, &old) })?;
			}
		}
		let mut new = unsafe { runtime_tables::named_procs(sub, r.to) }?;
		if !new.contains(&r.proc) {
			let at = new.iter().position(|&p| depth(p) < own_depth).unwrap_or(new.len());
			new.insert(at, r.proc);
			(unsafe { runtime_tables::set_named_procs(sub, r.to, &new) })?;
		}
	}
	Ok(())
}

pub(crate) fn locals_table(names: &[String], empty: u32) -> Result<u32, String> {
	if names.is_empty() {
		return Ok(empty);
	}
	let null_slot = |id: &u32| unsafe { runtime_tables::global_value(*id) }.is_ok_and(|v| v.tag == raw_types::values::ValueTag::Null);
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
	unsafe { runtime_tables::intern_misc(&words) }
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
	entry.0[ProcEntryWords::BYTECODE] = unsafe { runtime_tables::intern_misc(&bytecode) }?;
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
	let current_words = unsafe { runtime_tables::misc_words(current) }.unwrap_or_default();
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
	(unsafe { runtime_tables::intern_misc(&words) }).map(Some)
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
	(0..unsafe { runtime_tables::type_count() }?)
		.find(|&t| unsafe { runtime_tables::type_field(t, type_fields::PATH) } == Ok(id))
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
	(unsafe { runtime_tables::type_field(type_id, type_fields::PATH) })
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
		return Ok(PreparedInitProc { type_id, proc: None });
	};

	let current = unsafe { runtime_tables::type_field(type_id, type_fields::INIT_PROC) }.map_err(fail)?;
	let current = (current != NONE).then(|| Proc::from_id(raw_types::procs::ProcId(current))).flatten();
	let template = match &current {
		Some(proc) => unsafe { runtime_tables::proc_entry(proc.id) },
		None => proc_template().ok()
	}
	.ok_or_else(|| fail("couldn't find a proc entry to copy".to_owned()))?;
	let bytecode = assemble(&shown, code, current.as_ref(), pending, world_globals, added)?;
	let mut entry = template;
	if current.is_none() {
		entry.0[ProcEntryWords::PATH] = leaked_string(&format!("{}/proc/(init)", init.type_path)).map_err(fail)?;
		entry.0[ProcEntryWords::NAME] = NONE;
		entry.0[ProcEntryWords::DESC] = NONE;
		entry.0[ProcEntryWords::CATEGORY] = NONE;
		entry.0[ProcEntryWords::PATH_BASENAME] = NONE;
		entry.0[ProcEntryWords::PARAMETERS] = NONE;
	}
	entry.0[ProcEntryWords::BYTECODE] = unsafe { runtime_tables::intern_misc(&bytecode) }.map_err(fail)?;
	entry.0[ProcEntryWords::LOCALS] = locals_table(&code.locals, template.0[ProcEntryWords::LOCALS]).map_err(fail)?;
	entry.0[ProcEntryWords::NEXT_OVERRIDE] = NONE;

	Ok(PreparedInitProc {
		type_id,
		proc: Some((entry, code.locals.clone()))
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
	for sub in 0..unsafe { runtime_tables::type_count() }.map_err(fail)? {
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

	let string = |text: &str| -> Result<u32, String> {
		let string = StringRef::new(text).map_err(|_| fail(format!("couldn't make the string {}", text)))?;
		let id = string.get_id().0;
		std::mem::forget(string);
		Ok(id)
	};

	let path = match (new.type_path.as_str(), new.declared, new.verb) {
		("", true, _) => format!("/proc/{}", new.name),
		(ty, true, false) => format!("{}/proc/{}", ty, new.name),
		(ty, true, true) => format!("{}/verb/{}", ty, new.name),
		(ty, false, _) => format!("{}/{}", ty, new.name)
	};

	let mut entry = template;
	entry.0[ProcEntryWords::PATH] = string(&path)?;
	entry.0[ProcEntryWords::PATH_BASENAME] = string(&new.name)?;
	let settings = new.code.settings.clone().unwrap_or_default();
	for (word, value) in settings_words(&entry, &settings, &new.name).map_err(fail)? {
		entry.0[word] = value;
	}
	entry.0[ProcEntryWords::BYTECODE] = unsafe { runtime_tables::intern_misc(&bytecode) }.map_err(fail)?;
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
		for sub in 0..unsafe { runtime_tables::type_count() }.map_err(fail)? {
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
		for &proc in &relinked {
			let Some(path) = staged.entry(proc).map(|e| e.0[ProcEntryWords::PATH]) else {
				continue;
			};
			let path = String::from(unsafe { StringRef::from_id(raw_types::strings::StringId(path)) });
			if let Some(at) = path.rfind("/proc/") {
				let renamed = leaked_string(&format!("{}/{}", &path[..at], &path[at + "/proc/".len()..])).map_err(fail)?;
				match staged.entries.get_mut(&proc) {
					Some(e) => e.0[ProcEntryWords::PATH] = renamed,
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
			for sub in 0..unsafe { runtime_tables::type_count() }? {
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

const fn static_number(scope: &StaticScope) -> usize {
	match scope {
		StaticScope::Proc { nth, .. } => *nth as usize,
		_ => 0
	}
}

fn new_statics(
	code: &ProcReload,
	proc_globals: &HashMap<Vec<u8>, Vec<u32>>,
	pending: &Pending,
	added: &mut AddedVars
) -> Result<HashMap<(StaticScope, Vec<u8>), u32>, String> {
	let mut out = HashMap::new();
	for local in &code.statics {
		let name = local.name.as_bytes().to_vec();
		if proc_globals.get(&name).is_some_and(|ids| ids.len() > static_number(&local.scope)) {
			continue;
		}
		let value = match (&local.value, &local.init) {
			(Some(value), _) => const_value(value, pending).map_err(|e| format!("static local {}: {}", local.name, e))?,
			(None, Some(_)) => (0, 0),
			(None, None) => return Err(format!("static local {} has no initial value to start with", local.name))
		};
		let id = added.add(leaked_string(&local.name)?, value);
		if let (None, Some(init)) = (&local.value, &local.init) {
			added.run_once.push((id, format!("static local {}", local.name), (**init).clone()));
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
	(unsafe { runtime_tables::available() }).map_err(|e| vec![format!("hot reloading isn't available on this build of BYOND: {}", e)])?;
	(unsafe { runtime_tables::types_available() }).map_err(|e| vec![format!("adding types isn't available on this build of BYOND: {}", e)])?;
	code_changed();

	let mut added = 0;
	for new in new_types {
		if find_type(&new.path).is_ok() {
			continue;
		}
		add_type(new).map_err(|e| vec![format!("{}: {}", new.path, e)])?;
		added += 1;
	}
	Ok(added)
}

fn add_type(new: &NewType) -> Result<(), String> {
	let parent = find_type(&new.parent)?;
	let mut writes = vec![];
	for (field, value) in &new.fields {
		writes.extend(builtin_writes(field, value).map_err(|e| format!("{}: {}", field, e))?);
	}
	let id = unsafe { runtime_tables::add_type(parent, leaked_string(&new.path)?) }?;
	write_builtins(id, &writes)
}

#[derive(Clone, Debug)]
enum BuiltinWrite {
	Word(usize, u32),
	Half(usize, u16),
	Byte(usize, u8),
	Bits(usize, u32, u32),
	Listed(u32, Vec<u32>),
	Mob(&'static str, u32)
}

impl BuiltinWrite {
	fn string(&self) -> Option<u32> {
		match *self {
			BuiltinWrite::Word(offset, id) if offset != type_fields::ICON && offset != type_fields::LAYER => Some(id),
			BuiltinWrite::Mob("key", id) => Some(id),
			_ => None
		}
		.filter(|&id| id != NONE)
	}
}

fn builtin_writes(var: &str, value: &ConstValue) -> Result<Vec<BuiltinWrite>, String> {
	use BuiltinWrite::*;
	let number = || match value {
		ConstValue::Number(n) => Ok(*n),
		ConstValue::Null => Ok(0.0),
		_ => Err("expected a number".to_owned())
	};
	let string = || match value {
		ConstValue::Null => Ok(NONE),
		ConstValue::String(text) => leaked_string(text),
		_ => Err("expected text".to_owned())
	};
	let bits = |offset: usize, mask: u32, on: bool| Bits(offset, mask, if on { mask } else { 0 });
	Ok(match var {
		"name" => vec![Word(type_fields::NAME, string()?)],
		"desc" => vec![Word(type_fields::DESC, string()?)],
		"icon_state" => vec![Word(type_fields::ICON_STATE, string()?)],
		"text" => vec![Word(type_fields::TEXT, string()?)],
		"suffix" => vec![Word(type_fields::SUFFIX, string()?)],
		"maptext" => vec![Word(type_fields::MAPTEXT, string()?)],
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
			let (tag, data) = const_value(value, &Pending::default())?;
			let words = if tag == 0x2A {
				vec![tag, data >> 16, data & 0xFFFF]
			} else {
				vec![tag | ((data & 0xFF_0000) >> 8), data & 0xFFFF]
			};
			vec![Listed(leaked_string(var)?, words)]
		}
		"key" => vec![Mob("key", string()?)],
		"sight" => vec![Mob("sight", number()? as u32)],
		"see_in_dark" => vec![Mob("see_in_dark", number()? as u32)],
		"see_invisible" => vec![Mob("see_invisible", number()? as u32)],
		_ => return Err("built-in variable cannot be changed by hot reload".to_owned())
	})
}

fn write_builtins(type_id: u32, writes: &[BuiltinWrite]) -> Result<(), String> {
	use runtime_tables::{set_type_byte, set_type_field, set_type_half, type_field};
	for write in writes {
		match write {
			BuiltinWrite::Word(offset, word) => unsafe { set_type_field(type_id, *offset, *word)? },
			BuiltinWrite::Half(offset, half) => unsafe { set_type_half(type_id, *offset, *half)? },
			BuiltinWrite::Byte(offset, byte) => unsafe { set_type_byte(type_id, *offset, *byte)? },
			BuiltinWrite::Bits(offset, mask, bits) => {
				let word = unsafe { type_field(type_id, *offset) }?;
				unsafe { set_type_field(type_id, *offset, (word & !mask) | bits)? };
			}
			BuiltinWrite::Listed(name, value) => {
				let list = unsafe { runtime_tables::misc_words(type_field(type_id, type_fields::BUILTIN_VALUES)?) }.unwrap_or_default();
				let mut words = vec![];
				let mut at = 0;
				while at < list.len() {
					let len = if list.get(at + 1).is_some_and(|tag| tag & 0xFF == 0x2A) { 4 } else { 3 };
					let entry = &list[at..(at + len).min(list.len())];
					if entry[0] != *name {
						words.extend_from_slice(entry);
					}
					at += len;
				}
				words.push(*name);
				words.extend_from_slice(value);
				let list = unsafe { runtime_tables::intern_misc(&words) }?;
				unsafe { set_type_field(type_id, type_fields::BUILTIN_VALUES, list)? };
			}
			BuiltinWrite::Mob(var, value) => unsafe {
				runtime_tables::set_mob_type(type_id, |mob| match *var {
					"key" => mob.key = *value,
					"sight" => mob.sight = *value,
					"see_in_dark" => mob.see_in_dark = *value as u8,
					_ => mob.see_invisible = *value as u8
				})
			}?
		}
	}
	Ok(())
}

fn const_value(value: &ConstValue, pending: &Pending) -> Result<(u32, u32), String> {
	Ok(match value {
		ConstValue::Null => (0, 0),
		ConstValue::Number(n) => (0x2A, n.to_bits()),
		ConstValue::String(text) => (0x06, leaked_string(text)?),
		ConstValue::Path(path) if pending.procs.contains_key(&strip_path(path)) => {
			(raw_types::values::ValueTag::ProcRef.0 as u32, pending.procs[&strip_path(path)])
		}
		ConstValue::Path(path) => {
			let (tag, data) = AssembleEnv
				.get_type(path)
				.ok_or_else(|| format!("there's no type {} in the running world", path))?;
			(tag as u32, data)
		}
		ConstValue::Resource(file) => (
			0x0C,
			AssembleEnv
				.get_resource_index(file)
				.ok_or_else(|| format!("couldn't find the file {}", file))?
		)
	})
}

fn keep_string(id: u32) {
	if id != NONE {
		unsafe {
			raw_types::funcs::inc_ref_count(raw_types::values::Value {
				tag: raw_types::values::ValueTag::String,
				data: raw_types::values::ValueData { id }
			});
		}
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

fn leaked_string(text: &str) -> Result<u32, String> {
	let string = StringRef::new(text).map_err(|_| format!("couldn't make the string {}", text))?;
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
		let name = leaked_string(&new.var).map_err(fail)?;
		let declared = type_vars.declared(type_id).map_err(fail)?.clone();
		if let Some(pair) = declared.chunks(2).find(|pair| type_vars.name_of(pair[0]) == name) {
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
	let value = const_value(&new.value, pending).map_err(fail)?;
	let name = leaked_string(&new.var).map_err(fail)?;
	type_vars.new_names.insert(id, name);
	if let Some(init) = &new.init {
		added.run_once.push((id, format!("{}/var/{}", new.type_path, new.var), (**init).clone()));
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

	added.vars.push(PreparedNewVar { id, name, value });
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
	added: &mut AddedVars,
	type_vars: &mut TypeVars,
	pending: &mut Pending,
	global_values: &mut Vec<(u32, (u32, u32))>
) -> Result<(), String> {
	const STATIC: u32 = 1;
	let fail = |e: String| format!("{}/var/{}: {}", var.type_path, var.var, e);

	if var.type_path.is_empty() {
		let ids = variable_ids(var.var.as_bytes());
		let [id] = ids[..] else {
			return Err(fail(format!("expected one matching runtime variable, found {}", ids.len())));
		};
		if let Some(init) = &var.init {
			added.run_once.push((id, format!("/var/{}", var.var), (**init).clone()));
		} else if let Some(value) = &var.value {
			global_values.push((id, const_value(value, pending).map_err(fail)?));
		}
		return Ok(());
	}

	let type_id = find_type(&var.type_path).map_err(fail)?;
	let name = leaked_string(&var.var).map_err(fail)?;
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
			added.run_once.push((id, format!("{}/var/{}", var.type_path, var.var), (**init).clone()));
		} else if let Some(value) = &var.value {
			global_values.push((id, const_value(value, pending).map_err(fail)?));
		}
	}
	Ok(())
}

fn prepare_var_default(default: &VarDefault, pending: &Pending, type_vars: &mut TypeVars, added: &mut AddedVars) -> Result<(), String> {
	let fail = |e: String| format!("var {}: {}", default.var, e);

	let value = const_value(&default.value, pending).map_err(fail)?;
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
					None => added.add(leaked_string(&default.var).map_err(fail)?, (0, 0))
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
	let procs_before = unsafe { runtime_tables::proc_count() };
	let appended = append_entries(&prepared, &mut added);
	added.procs = (unsafe { runtime_tables::proc_count() } - procs_before) as usize;
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
			unsafe { runtime_tables::add_variable(new.name, new.value.0 as u8, new.value.1) }.map_err(|e| format!("couldn't add a var: {}", e))?;
		added.vars += 1;
		if id != new.id {
			return Err(format!("new variable ID mismatch: got {}, expected {}", id, new.id));
		}
	}
	for new in &prepared.new_procs {
		let id = failing(FailAt::Append)
			.and_then(|()| unsafe { runtime_tables::append_proc(&new.entry) })
			.map_err(|e| format!("couldn't add a proc: {}", e))?;
		if id.0 != new.id {
			return Err(format!("new proc ID mismatch: got {}, expected {}", id.0, new.id));
		}
	}
	for (expected, instance, entry) in &prepared.instances {
		let id = unsafe { runtime_tables::add_proc(entry) }
			.and_then(|init| unsafe { runtime_tables::add_instance(runtime_tables::Instance { init: init.0, ..*instance }) })
			.map_err(|e| format!("couldn't add a prefab: {}", e))?;
		added.prefabs += 1;
		if id != *expected {
			return Err(format!("prefab ID mismatch: got {}, expected {}", id, expected));
		}
	}
	for (expected, entry) in &prepared.in_lists {
		let index = unsafe { runtime_tables::add_proc(entry) }
			.and_then(|proc| unsafe { runtime_tables::add_in_list(proc) })
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
				(unsafe { runtime_tables::add_proc(entry) })
					.map_err(|e| format!("couldn't add an init proc: {}", e))?
					.0
			}
		});
	}
	if !prepared.new_globals.is_empty() {
		let pairs: Vec<(u32, u32)> = prepared.new_globals.iter().map(|&(_, id, flags)| (id, flags)).collect();
		(unsafe { runtime_tables::add_global_vars(&pairs) }).map_err(|e| format!("the new global vars are missing from global.vars: {}", e))?;
	}
	Ok(init_procs)
}

pub fn apply<K: Ord + std::hash::Hash + Copy, T>(
	appended: Appended,
	breakpoints: &mut HashMap<(raw_types::procs::ProcId, u16), BTreeMap<K, T>>,
	moved_breakpoints: &mut MovedBreakpoints<K>,
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
	reparent::apply_tree(&mut prepared.reparent);
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
		let procs = unsafe { runtime_tables::type_field(removal.type_id, removal.field) }.unwrap_or(NONE);
		let mut list = unsafe { runtime_tables::misc_words(procs) }.unwrap_or_default();
		list.retain(|&proc| proc != removal.id);
		let list = if list.is_empty() {
			Ok(NONE)
		} else {
			unsafe { runtime_tables::intern_misc(&list) }
		};
		match list {
			Ok(list) => {
				let _ = unsafe { runtime_tables::set_type_field(removal.type_id, removal.field, list) };
			}
			Err(e) => failures.push(format!("could not unlink proc from type list: {}", e))
		}
		for (sub, key, procs) in &removal.map_entries {
			if let Err(e) = unsafe { runtime_tables::set_named_procs(*sub, *key, procs) } {
				failures.push(format!("could not unlink proc from type name map: {}", e));
			}
		}
		for &proc in &removal.relinked {
			unsafe { runtime_tables::set_proc_entry_word(raw_types::procs::ProcId(proc), ProcEntryWords::NEXT_OVERRIDE, removal.next) };
		}
	}

	for new in &prepared.new_procs {
		let id = new.id;
		mark_reloaded(id, Reloaded::Added);
		if let Some(on_type) = &new.on_type {
			let procs = unsafe { runtime_tables::type_field(on_type.type_id, on_type.field) }.unwrap_or(NONE);
			let mut list = unsafe { runtime_tables::misc_words(procs) }.unwrap_or_default();
			match on_type.after.and_then(|after| list.iter().position(|&p| p == after)) {
				Some(at) if !list.contains(&id) => list.insert(at + 1, id),
				_ => add_to_list(&mut list, on_type.field, id)
			}
			match unsafe { runtime_tables::intern_misc(&list) } {
				Ok(list) => {
					let _ = unsafe { runtime_tables::set_type_field(on_type.type_id, on_type.field, list) };
				}
				Err(e) => failures.push(format!("couldn't add a proc to its type's list: {}", e))
			}

			for (sub, key, procs) in &on_type.map_entries {
				if let Err(e) = unsafe { runtime_tables::set_named_procs(*sub, *key, procs) } {
					failures.push(format!("couldn't add a proc to a type's proc names: {}", e));
				}
			}

			for &proc in &on_type.relinked {
				unsafe { runtime_tables::set_proc_entry_word(raw_types::procs::ProcId(proc), ProcEntryWords::NEXT_OVERRIDE, id) };
			}
			for &(proc, path) in &on_type.paths {
				unsafe { runtime_tables::set_proc_entry_word(raw_types::procs::ProcId(proc), ProcEntryWords::PATH, path) };
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
		let _ = unsafe { runtime_tables::set_type_field(init.type_id, type_fields::INIT_PROC, id) };
	}

	if unsafe { runtime_tables::proc_count() } > procs_before {
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
		restore_breakpoints(code, breakpoints, moved_breakpoints, warnings);
	}
	for r in renames {
		if let Err(e) = rename(r) {
			failures.push(format!("could not update dynamic lookup for renamed proc: {}", e));
		}
	}
	for (proc, word, value) in prepared.entry_words {
		unsafe { runtime_tables::set_proc_entry_word(raw_types::procs::ProcId(proc), word, value) };
	}
	for (type_id, field, words) in prepared.type_fields {
		let _ = unsafe { runtime_tables::set_type_field(type_id, field, words) };
	}
	reparent::apply_builtins(&prepared.reparent, warnings, failures);
	for (type_id, writes) in &prepared.builtin_writes {
		if let Err(e) = failing(FailAt::Change).and_then(|()| write_builtins(*type_id, writes)) {
			failures.push(format!("couldn't set a built-in var: {}", e));
		}
	}
	inherit::apply(&prepared.builtin_inherits, failures);
	for &type_id in &prepared.appearances {
		let _ = unsafe { runtime_tables::forget_type_appearance(type_id) };
	}

	for (id, (tag, data)) in prepared.global_values {
		if let Err(e) = unsafe { runtime_tables::set_global_words(id, tag as u8, data) } {
			failures.push(format!("couldn't set a static var's value: {}", e));
		}
	}

	if let Some(proc) = prepared.global_init_at_start {
		auxtools::start_runs_new_code(proc);
	}
	if let Some(bytecode) = prepared.global_init_now {
		match Proc::find("/proc/auxtools_expr_stub") {
			Some(stub) => match unsafe { stub.set_bytecode(bytecode) } {
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
		if let Err(e) = unsafe { stub.set_bytecode(bytecode) } {
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
				if let Err(e) = unsafe { runtime_tables::set_global_value(id, value) } {
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
	old_file: Option<u32>,
	hooked: Vec<(u32, u32, Option<String>, Option<BTreeMap<K, T>>)>
}

fn apply_code<K: Ord + std::hash::Hash + Copy, T>(
	p: PreparedCode,
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
	let hooked: Vec<(u32, u32, Option<String>, Option<BTreeMap<K, T>>)> = hooked_at
		.into_iter()
		.filter_map(|offset| {
			let code = old_code.as_ref()?;
			let line = code.line_at(offset)?;
			let other_file = (code.place(offset, false).and_then(|(file, _)| file))
				.filter(|&file| code.file().is_some_and(|own| own != file))
				.map(str::to_owned);
			let known = breakpoints.remove(&(p.proc.id, offset as u16));
			let _ = unhook_instruction(&p.proc, offset);
			Some((offset, line, other_file, known))
		})
		.collect();

	let local_names = p
		.local_names
		.clone()
		.unwrap_or_else(|| auxtools::debug::local_names_for(&p.proc, old_ptr));
	let old_file = {
		let old = original_code(&p.proc);
		auxtools::remember_bytecode(old_ptr, &old);
		debug_file(&old)
	};

	unsafe { runtime_tables::set_proc_entry_word(p.proc.id, ProcEntryWords::BYTECODE, p.bytecode_id) };
	if let Some(locals) = p.locals_id {
		unsafe { runtime_tables::set_proc_entry_word(p.proc.id, ProcEntryWords::LOCALS, locals) };
	}
	if let Some(params) = p.params_id {
		unsafe { runtime_tables::set_proc_entry_word(p.proc.id, ProcEntryWords::PARAMETERS, params) };
	}
	for &(word, value) in &p.settings {
		unsafe { runtime_tables::set_proc_entry_word(p.proc.id, word, value) };
	}

	let (new_ptr, _) = unsafe { p.proc.bytecode_mut_ptr() };
	let names_at = if auxtools::debug::has_local_names(&p.proc, new_ptr) && auxtools::debug::local_names_for(&p.proc, new_ptr) != local_names {
		auxtools::copy_code(&p.proc).map(|()| unsafe { p.proc.bytecode_mut_ptr() }.0)
	} else {
		Ok(new_ptr)
	};
	match names_at {
		Ok(at) => auxtools::debug::set_local_names(&p.proc, at, local_names),
		Err(e) => failures.push(format!("{}: new calls show the old names of its locals: {}", p.name, e))
	}
	crate::code_index::proc_changed(p.proc.id);

	let old_code = old_code.filter(|_| !hooked.is_empty())?;
	Some(UnhookedCode {
		code: p,
		old_code,
		old_file,
		hooked
	})
}

fn restore_breakpoints<K: Ord + std::hash::Hash + Copy, T>(
	unhooked: UnhookedCode<K, T>,
	breakpoints: &mut HashMap<(raw_types::procs::ProcId, u16), BTreeMap<K, T>>,
	moved_breakpoints: &mut MovedBreakpoints<K>,
	warnings: &mut Vec<String>
) {
	let UnhookedCode {
		code: p,
		old_code,
		old_file,
		hooked
	} = unhooked;
	let new_code = crate::code_index::of(&p.proc);
	let firsts = match (old_code.lines().next(), new_code.lines().next()) {
		(Some(old_first), Some(new_first)) if old_file != debug_file(&p.bytecode) => Some((old_first, new_first)),
		_ => None
	};

	for (old_offset, line, other_file, known) in hooked {
		let line = match &p.lines {
			Some(lines) if other_file.is_none() => map_line(line, lines),
			_ => line
		};

		let mut known_as: Vec<(raw_types::procs::ProcId, u32, K)> = moved_breakpoints
			.iter()
			.filter(|((proc, ..), at)| *proc == p.proc.id && **at == old_offset)
			.map(|(key, _)| *key)
			.collect();
		for key in &known_as {
			moved_breakpoints.remove(key);
		}
		for client in known.iter().flat_map(BTreeMap::keys) {
			if !known_as.iter().any(|(.., other)| other == client) {
				known_as.push((p.proc.id, old_offset, *client));
			}
		}

		let moved = firsts
			.filter(|(old_first, _)| line >= *old_first)
			.map(|(old_first, new_first)| line - old_first + new_first);
		let in_other_file = |file: &str| Some(file) == other_file.as_deref();
		let wanted = other_file.is_some().then_some(&in_other_file as &dyn Fn(&str) -> bool);
		let Some((line, offset)) = new_code
			.offset_of(moved.unwrap_or(line), wanted)
			.map(|offset| (moved.unwrap_or(line), offset))
		else {
			warnings.push(format!(
				"{}: removed breakpoint on line {}: no executable code after reload",
				p.name, line
			));
			continue;
		};

		if hook_instruction(&p.proc, offset).is_err() {
			warnings.push(format!("{}: could not restore breakpoint on line {}", p.name, line));
			continue;
		}

		if let Some(known) = known {
			breakpoints.insert((p.proc.id, offset as u16), known);
		}
		for key in known_as {
			if key.1 != offset {
				moved_breakpoints.insert(key, offset);
			}
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
	if unsafe { runtime_tables::world_names_built() }? {
		return edit_built_world_names(&edit);
	}
	WAITING_WORLD_EDITS.lock().unwrap().push(edit);
	auxtools::at_proc_calls(make_waiting_world_edits);
	Ok(())
}

fn make_waiting_world_edits() -> bool {
	if !unsafe { runtime_tables::world_names_built() }.unwrap_or(false) {
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
	let set_next =
		|proc: u32, next: u32| unsafe { runtime_tables::set_proc_entry_word(raw_types::procs::ProcId(proc), ProcEntryWords::NEXT_OVERRIDE, next) };
	let next_of =
		|proc: u32| unsafe { runtime_tables::proc_entry(raw_types::procs::ProcId(proc)) }.map_or(NONE, |e| e.0[ProcEntryWords::NEXT_OVERRIDE]);
	match edit {
		WorldEdit::Add { id, keys, below, above } => {
			let pinned = PINNED_WORLD_PROCS.lock().unwrap().clone();
			set_next(*id, *below);
			for (k, &key) in keys.iter().enumerate() {
				let mut procs = unsafe { runtime_tables::world_named_procs(key) }?;
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
				(unsafe { runtime_tables::set_world_named_procs(key, &procs) })?;
			}
		}
		WorldEdit::Remove { id, keys } => {
			let next = next_of(*id);
			for &key in keys {
				let mut procs = unsafe { runtime_tables::world_named_procs(key) }?;
				for &proc in &procs {
					if proc != *id && next_of(proc) == *id {
						set_next(proc, next);
					}
				}
				procs.retain(|proc| proc != id);
				(unsafe { runtime_tables::set_world_named_procs(key, &procs) })?;
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
		.and_then(|proc| unsafe { runtime_tables::proc_entry(proc.id) })
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

	fn place(&self) -> Option<(Vec<u8>, u32, u32)> {
		let (first, last) = self.lines?;
		Some((self.files.first()?.clone(), first, last))
	}
}

#[derive(Default)]
struct WorldGlobals {
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
	proc_globals: HashMap<Vec<u8>, Vec<u32>>,
	world_globals: &'a mut WorldGlobals,
	pending: &'a Pending,
	new_statics: HashMap<(StaticScope, Vec<u8>), u32>,
	known: HashMap<(StaticScope, Vec<u8>), u32>,
	instances: &'a mut StagedInstances,
	place: Option<(Vec<u8>, u32, u32)>,
	error: Option<String>
}

impl dmasm::assembler::AssembleEnv for ReloadEnv<'_> {
	fn get_string_index(&mut self, string: &[u8]) -> Option<u32> {
		AssembleEnv.get_string_index(string)
	}

	fn get_variable_name_index(&mut self, name: &[u8]) -> Option<u32> {
		AssembleEnv.get_variable_name_index(name)
	}

	fn get_proc_index(&mut self, path: &str) -> Option<u32> {
		if let Some(&id) = self.pending.procs.get(&strip_path(path)) {
			return Some(id);
		}
		AssembleEnv.get_proc_index(path)
	}

	fn get_type(&mut self, path: &str) -> Option<(u8, u32)> {
		if let Some(&id) = self.pending.procs.get(&strip_path(path)) {
			return Some((raw_types::values::ValueTag::ProcRef.0, id));
		}
		AssembleEnv.get_type(path)
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
			let id = self.proc_globals.get(name).and_then(|ids| ids.get(*nth as usize)).copied();
			let id = id.or_else(|| self.new_statics.get(&key).copied());
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
			if let Ok(globals) = unsafe { runtime_tables::global_var_ids() } {
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

		if let Some([id]) = self.proc_globals.get(name).map(Vec::as_slice) {
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
	let declared = unsafe { runtime_tables::misc_words(runtime_tables::type_field(type_id, type_fields::DECLARED_VARS).ok()?) }?;
	let found: Vec<u32> = declared
		.chunks(2)
		.filter(|c| c.len() == 2 && c[1] & 3 != 0 && unsafe { runtime_tables::variable_name(c[0]) } == Some(name))
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
}
