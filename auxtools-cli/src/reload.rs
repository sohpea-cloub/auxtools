use std::path::PathBuf;

use auxtools_protocol::{HotReloadRequest, LocalReads, ParsedReads, ShadowingProc};
use dmasm::{
	diagnostic::Id,
	reload::{Changes, Plan, ProcKey, ProcReload, Refusal, Snapshot},
	Node, StaticScope
};

mod locals;

pub const RESTART: &str = "Rebuild the .dmb, then restart the world to load all of the new code.";

pub fn request(changes: &Changes, before: &Snapshot, restarted: bool) -> HotReloadRequest {
	let mut plan = changes.plan.clone();
	let local_reads = without_existing_statics(&mut plan, changes, before);
	HotReloadRequest {
		shadowing_procs: shadowing_procs(&plan, before),
		local_reads,
		plan,
		resources: vec![],
		resource_dirs: vec![],
		global_init: changes.global_init.clone().filter(|_| restarted)
	}
}

pub fn unmatched_locals(changes: &Changes, before: &Snapshot) -> Vec<Refusal> {
	let mut refused = vec![];
	for code in &changes.plan.reloads {
		let (old, new) = old_and_new_locals(code, changes, before);
		for unmatched in locals::matched(&old, &new).1 {
			refused.push(Refusal {
				file: before.proc_file(&code.proc),
				line: Some(unmatched.line),
				..Refusal::new(
					Id::DuplicateDefinition,
					code.proc.to_string(),
					format!(
						"the edit leaves {} static or const locals of this name where there were {}, in blocks of the same kinds. Which of them \
						 keep their values can't be told. Rebuild to apply it",
						unmatched.new, unmatched.old
					)
				)
				.named(unmatched.name)
			});
		}
	}
	refused
}

fn old_and_new_locals<'a>(code: &ProcReload, changes: &'a Changes, before: &'a Snapshot) -> (Vec<locals::Declared<'a>>, Vec<locals::Declared<'a>>) {
	let old = before.proc_code(&code.proc);
	let new = if code.statics.is_empty() {
		None
	} else {
		changes.new_code(&code.proc).or(old)
	};
	let declared = |body: Option<&'a dreammaker::ast::Block>| body.map(|body| locals::declared(body)).unwrap_or_default();
	(declared(old), declared(new))
}

fn shadowing_procs(plan: &Plan, before: &Snapshot) -> Vec<ShadowingProc> {
	let globals: Vec<&str> = plan
		.redeclared
		.iter()
		.filter(|var| var.type_path.is_empty())
		.map(|var| var.var.as_str())
		.collect();
	if globals.is_empty() {
		return vec![];
	}
	let mut found = vec![];
	for key in before.procs() {
		let Some(code) = before.proc_code(key) else {
			continue;
		};
		let declared = locals::declared(code);
		let shadowed: Vec<&str> = globals
			.iter()
			.copied()
			.filter(|global| declared.iter().any(|local| local.name == *global))
			.collect();
		if shadowed.is_empty() {
			continue;
		}
		let Ok(compiled) = before.compile(key) else {
			continue;
		};
		let reads = local_reads(&compiled.nodes);
		for global in shadowed {
			found.push(ShadowingProc {
				global: global.to_owned(),
				proc: key.clone(),
				reads: reads
					.iter()
					.find(|(name, _)| name == global)
					.map(|(_, reads)| reads.clone())
					.unwrap_or_default()
			});
		}
	}
	found
}

fn without_existing_statics(plan: &mut Plan, changes: &Changes, before: &Snapshot) -> Vec<LocalReads> {
	let mut found = vec![];
	for code in &mut plan.reloads {
		let old = OldLocals::of(before, &code.proc);
		if code.statics.is_empty() && old.declared.is_empty() {
			continue;
		}
		let (_, new_locals) = old_and_new_locals(code, changes, before);
		let (matched, _) = locals::matched(&old.declared, &new_locals);
		let was = |name: &str, nth: u32| matched.iter().find(|(had, new, _)| *had == name && *new == nth).map(|(.., old)| *old);
		let nodes = &code.nodes;
		code.statics.retain_mut(|local| {
			let nth = local_number(&local.scope);
			let old_nth = was(&local.name, nth);
			let read_now = reads_static(nodes, &local.name, &local.scope);
			let initialized = local.init.is_some() && local.value.is_none() && !local.constant;
			if !old_nth.is_some_and(|old_nth| old.read_as(&local.name, old_nth)) {
				local.constant = false;
				return old_nth.is_none() || read_now && !initialized;
			}
			local.constant && read_now
		});
		found.extend(old.reads.iter().map(|(name, reads, parsed)| {
			LocalReads {
				proc: code.proc.clone(),
				name: name.clone(),
				reads: reads.clone(),
				parsed: *parsed,
				kept: matched
					.iter()
					.filter(|(had, _, old_nth)| had == name && old.read_as(had, *old_nth))
					.map(|(_, new, old)| (*new, *old))
					.collect()
			}
		}));
	}
	found
}

struct OldLocals<'a> {
	declared: Vec<locals::Declared<'a>>,
	reads: Vec<(String, Vec<Option<u32>>, Option<ParsedReads>)>
}

impl<'a> OldLocals<'a> {
	fn of(snapshot: &'a Snapshot, proc: &ProcKey) -> Self {
		let body = snapshot.proc_code(proc).map_or(&[][..], |code| &code[..]);
		let declared = locals::declared(body);
		let mut names: Vec<&str> = declared.iter().map(|local| local.name).collect();
		names.sort_unstable();
		names.dedup();
		if names.is_empty() {
			return Self { declared, reads: vec![] };
		}
		let reads = match snapshot.compile(proc) {
			Ok(old) => local_reads(&old.nodes)
				.into_iter()
				.filter(|(name, _)| names.contains(&name.as_str()))
				.map(|(name, reads)| (name, reads, None))
				.collect(),
			Err(_) => {
				let params = snapshot.proc_parameters(proc).unwrap_or_default();
				names
					.into_iter()
					.filter_map(|name| Some((name.to_owned(), vec![], Some(locals::parsed_reads(params, body, name)?))))
					.collect()
			}
		};
		Self { declared, reads }
	}

	fn read_as(&self, name: &str, nth: u32) -> bool {
		self.reads.iter().any(|(had, reads, parsed)| {
			had == name
				&& match parsed {
					Some(parsed) => *parsed == ParsedReads::Only(nth),
					None => reads.contains(&Some(nth))
				}
		})
	}
}

const fn local_number(scope: &StaticScope) -> u32 {
	match scope {
		StaticScope::Proc { nth, .. } => *nth,
		_ => 0
	}
}

fn local_reads(nodes: &[Node]) -> Vec<(String, Vec<Option<u32>>)> {
	let mut out: Vec<(String, Vec<Option<u32>>)> = vec![];
	for node in nodes {
		let Node::Instruction(ins, _) = node else {
			continue;
		};
		for var in ins.variables() {
			for global in var.globals() {
				let name = String::from_utf8_lossy(&global.name.0);
				let read = match &global.scope {
					Some(StaticScope::Proc { nth, .. }) => Some(*nth),
					_ => None
				};
				match out.iter_mut().find(|(had, _)| *had == name) {
					Some((_, reads)) => reads.push(read),
					None => out.push((name.into_owned(), vec![read]))
				}
			}
		}
	}
	out
}

fn reads_static(nodes: &[Node], name: &str, scope: &StaticScope) -> bool {
	nodes.iter().any(|node| {
		let Node::Instruction(ins, _) = node else {
			return false;
		};
		ins.variables().iter().any(|var| {
			var.globals()
				.iter()
				.any(|global| global.name.0 == name.as_bytes() && global.scope.as_ref() == Some(scope))
		})
	})
}

pub fn resource_names(request: &HotReloadRequest) -> Vec<String> {
	#[derive(Default)]
	struct ResourceNames(Vec<String>);

	impl ResourceNames {
		fn value(&mut self, value: &dmasm::reload::ConstValue) {
			if let dmasm::reload::ConstValue::Resource(name) = value {
				self.add(name);
			}
		}

		fn add(&mut self, name: &str) {
			if !self.0.iter().any(|had| had == name) {
				self.0.push(name.to_owned());
			}
		}

		fn code(&mut self, nodes: &[dmasm::Node], statics: &[dmasm::reload::StaticVar]) {
			let _ = dmasm::assembler::assemble(nodes, self);
			for local in statics {
				if let Some(value) = &local.value {
					self.value(value);
				}
				if let Some(init) = &local.init {
					self.code(&init.nodes, &init.statics);
				}
			}
		}
	}

	impl dmasm::assembler::AssembleEnv for ResourceNames {
		fn get_string_index(&mut self, _: &[u8]) -> Option<u32> {
			Some(0)
		}

		fn get_variable_name_index(&mut self, _: &[u8]) -> Option<u32> {
			Some(0)
		}

		fn get_proc_index(&mut self, _: &str) -> Option<u32> {
			Some(0)
		}

		fn get_type(&mut self, _: &str) -> Option<(u8, u32)> {
			Some((0x20, 0))
		}

		fn get_global_index(&mut self, _: &[u8], _: Option<&dmasm::StaticScope>) -> Option<u32> {
			Some(0)
		}

		fn get_resource_index(&mut self, path: &str) -> Option<u32> {
			self.add(path);
			Some(0)
		}

		fn get_instance(&mut self, _: &str, _: &[u32]) -> Option<u32> {
			Some(0)
		}
	}

	let mut named = ResourceNames::default();
	let plan = &request.plan;
	for code in plan.reloads.iter().chain(plan.new_procs.iter().map(|new| &new.code)) {
		named.code(&code.nodes, &code.statics);
	}
	for code in plan.init_procs.iter().filter_map(|init| init.code.as_ref()) {
		named.code(&code.nodes, &code.statics);
	}
	for default in plan
		.var_defaults
		.iter()
		.map(|d| &d.value)
		.chain(plan.builtin_defaults.iter().map(|d| &d.value))
	{
		named.value(default);
	}
	for new in &plan.new_types {
		for (_, field) in &new.fields {
			named.value(field);
		}
	}
	for var in &plan.new_vars {
		named.value(&var.value);
		if let Some(init) = &var.init {
			named.code(&init.nodes, &init.statics);
		}
	}
	for var in &plan.redeclared {
		if let Some(value) = &var.value {
			named.value(value);
		}
		if let Some(init) = &var.init {
			named.code(&init.nodes, &init.statics);
		}
	}
	if let Some(init) = &request.global_init {
		named.code(&init.code.nodes, &init.code.statics);
	}
	named.0
}

pub fn merge(changes: &mut Changes, more: Changes) {
	for reload in more.plan.reloads {
		if !changes.plan.reloads.iter().any(|r| r.proc == reload.proc) {
			changes.plan.reloads.push(reload);
		}
	}
	changes.plan.var_defaults.extend(more.plan.var_defaults);
	changes.plan.builtin_defaults.extend(more.plan.builtin_defaults);
	for init in more.plan.init_procs {
		if !changes.plan.init_procs.iter().any(|i| i.type_path == init.type_path) {
			changes.plan.init_procs.push(init);
		}
	}
	changes.warnings.extend(more.warnings);
	changes.refused.extend(more.refused);
}

pub fn refuse_hidden(old: &Snapshot, new: &Snapshot, changes: &mut Changes) {
	if changes.plan.new_types.is_empty() && changes.plan.new_procs.is_empty() && changes.plan.new_vars.is_empty() {
		return;
	}
	let mut fixed = old.parse_errors();
	for error in new.parse_errors() {
		if let Some(at) = fixed.iter().position(|e| e.key() == error.key()) {
			fixed.remove(at);
		}
	}
	for error in fixed {
		changes.refused.push(Refusal {
			file: Some(error.file),
			line: Some(error.line),
			..Refusal::new(
				Id::FixedParseError,
				"",
				format!(
					"a parse error that is now fixed may have hidden definitions: {}. Rebuild and reboot before reloading",
					error.description
				)
			)
		});
	}
}

pub fn refuse_other_maps(built: &Snapshot, new: &Snapshot, changes: &mut Changes) {
	let listed = |maps: &[PathBuf], others: &[PathBuf]| {
		maps.iter()
			.filter(|map| !others.contains(map))
			.map(|map| map.display().to_string())
			.collect::<Vec<_>>()
	};
	let (added, removed) = (listed(new.maps(), built.maps()), listed(built.maps(), new.maps()));
	let mut changed = vec![];
	if !added.is_empty() {
		changed.push(format!("added {}", added.join(", ")));
	}
	if !removed.is_empty() {
		changed.push(format!("removed {}", removed.join(", ")));
	}
	if !changed.is_empty() {
		changes.refused.push(Refusal::new(
			Id::MapsChanged,
			"",
			format!("compiled maps changed: {}", changed.join("; "))
		));
	}
}

#[cfg(test)]
mod tests {
	use std::{
		collections::HashSet,
		path::{Path, PathBuf}
	};

	use dmasm::reload::{diff, Changes, ParseOptions, Snapshot};

	use auxtools_protocol::HotReloadRequest;

	use super::{merge, request, unmatched_locals};

	fn edited(name: &str, before: &str, after: &str) -> (Snapshot, Changes) {
		let dir = std::env::temp_dir().join(format!("auxtools-reload-{}-{}", name, std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		std::fs::write(dir.join("world.dme"), "#include \"code.dm\"\n").unwrap();
		let parse = |code: &str| {
			std::fs::write(dir.join("code.dm"), code).unwrap();
			Snapshot::parse(&dir.join("world.dme"), &ParseOptions::default()).unwrap()
		};
		let old = parse(before);
		let new = parse(after);
		let files: HashSet<PathBuf> = [Path::new("code.dm").to_owned()].into_iter().collect();
		let changes = diff(&old, &new, Some(&files));
		let _ = std::fs::remove_dir_all(&dir);
		assert!(changes.refused.is_empty(), "{:?}", changes.refused);
		(old, changes)
	}

	#[test]
	fn a_request_sends_the_local_consts_the_new_code_reads() {
		let before = "var/LIMIT = 1\n/proc/f()\n\tvar/const/LIMIT = 5\n\tvar/const/UNREAD = 2\n\treturn LIMIT\n/proc/g()\n\treturn LIMIT\n";
		let after = "var/LIMIT = 1\n/proc/f()\n\tvar/const/LIMIT = 10\n\tvar/const/UNREAD = 2\n\treturn LIMIT\n/proc/g()\n\tvar/const/LIMIT = \
		             7\n\treturn LIMIT\n";
		let (old, changes) = edited("consts", before, after);
		let sent = request(&changes, &old, false);
		let statics: Vec<String> = sent
			.plan
			.reloads
			.iter()
			.flat_map(|reload| {
				reload
					.statics
					.iter()
					.map(move |local| format!("{} {} {}", reload.proc, local.name, local.constant))
			})
			.collect();
		assert_eq!(statics, ["/f LIMIT true", "/g LIMIT false"]);
	}

	#[test]
	fn a_request_tells_a_procs_locals_from_globals_of_their_name() {
		let before = "var/LIMIT = 1\n/proc/f()\n\tvar/const/LIMIT = 5\n\treturn global.LIMIT\n/proc/g()\n\tvar/static/LIMIT = \
		              7\n\tLIMIT++\n\treturn list(LIMIT, global.LIMIT)\n";
		let after = before.replace("return global.LIMIT", "return LIMIT").replace("LIMIT++", "LIMIT += 2");
		let (old, changes) = edited("told-apart", before, &after);
		let sent = request(&changes, &old, false);
		let statics: Vec<String> = sent
			.plan
			.reloads
			.iter()
			.flat_map(|reload| {
				reload
					.statics
					.iter()
					.map(move |local| format!("{} {} {}", reload.proc, local.name, local.constant))
			})
			.collect();
		assert_eq!(statics, ["/f LIMIT false"]);
		let reads: Vec<String> = sent
			.local_reads
			.iter()
			.map(|reads| format!("{} {} {:?}", reads.proc, reads.name, reads.reads))
			.collect();
		assert_eq!(reads, ["/f LIMIT [None]", "/g LIMIT [Some(0), Some(0), None]"]);
	}

	#[test]
	fn a_request_leaves_out_the_statics_of_old_code_dmasm_cant_compile() {
		let twice = "/datum/t/proc/f()\n\treturn 0\n/datum/t/f()\n\treturn 1\n/datum/t/f()\n\treturn 2\n";
		let before = format!("{twice}/proc/f()\n\tvar/static/count = 0\n\tcount++\n\tif (/datum/t::f())\n\t\treturn count\n");
		let after = before.replace("\tif (/datum/t::f())\n\t\treturn count", "\tvar/static/added = 1\n\treturn count + added");
		let (old, changes) = edited("uncompiled-statics", &before, &after);
		assert!(old.compile(&changes.plan.reloads[0].proc).is_err(), "dmasm compiles the old code");
		let sent = request(&changes, &old, false);
		let statics: Vec<&str> = sent.plan.reloads[0].statics.iter().map(|local| local.name.as_str()).collect();
		assert_eq!(statics, ["added"]);
	}

	#[test]
	fn a_request_leaves_out_an_unread_static_whose_initializer_ran_at_the_start() {
		let before = "/proc/register()\n\treturn 1\n/proc/f()\n\tvar/static/kept = register()\n\tvar/static/plain = 2\n\treturn 0\n";
		let (old, changes) = edited("unread-initialized", before, &before.replace("return 0\n", "return kept + plain\n"));
		let sent = request(&changes, &old, false);
		let statics: Vec<&str> = sent.plan.reloads[0].statics.iter().map(|local| local.name.as_str()).collect();
		assert_eq!(statics, ["plain"]);
	}

	#[test]
	fn a_request_sends_a_const_as_new_when_old_code_that_doesnt_compile_reads_a_global() {
		let twice = "/datum/t/proc/f()\n\treturn 0\n/datum/t/f()\n\treturn 1\n/datum/t/f()\n\treturn 2\n";
		let before = format!(
			"var/LIMIT = 1\n{twice}/proc/f()\n\tvar/const/LIMIT = 5\n\tvar/static/count = 0\n\tcount++\n\tif (/datum/t::f())\n\t\treturn \
			 global.LIMIT + count\n"
		);
		let after = before.replace("\tif (/datum/t::f())\n\t\treturn global.LIMIT + count", "\treturn LIMIT + count");
		let (old, changes) = edited("uncompiled-global", &before, &after);
		assert!(old.compile(&changes.plan.reloads[0].proc).is_err(), "dmasm compiles the old code");
		let sent = request(&changes, &old, false);
		let statics: Vec<String> = sent.plan.reloads[0]
			.statics
			.iter()
			.map(|local| format!("{} {}", local.name, local.constant))
			.collect();
		assert_eq!(statics, ["LIMIT false"]);
		let reads: Vec<String> = sent
			.local_reads
			.iter()
			.map(|reads| format!("{} {:?} {:?}", reads.name, reads.reads, reads.parsed))
			.collect();
		assert_eq!(reads, ["LIMIT [] Some(Unknown)", "count [] Some(Only(0))"]);
	}

	#[test]
	fn a_request_sends_a_const_as_new_when_a_parameter_default_of_old_code_that_doesnt_compile_reads_a_global() {
		let twice = "/datum/t/proc/f()\n\treturn 0\n/datum/t/f()\n\treturn 1\n/datum/t/f()\n\treturn 2\n";
		let before =
			format!("var/LIMIT = 1\n{twice}/proc/f(x = global.LIMIT)\n\tvar/const/LIMIT = 5\n\tif (/datum/t::f())\n\t\treturn list(LIMIT + 1, x)\n");
		let after = before.replace("\tif (/datum/t::f())\n\t\treturn list(LIMIT + 1, x)", "\treturn LIMIT");
		let (old, changes) = edited("uncompiled-default", &before, &after);
		assert!(old.compile(&changes.plan.reloads[0].proc).is_err(), "dmasm compiles the old code");
		let sent = request(&changes, &old, false);
		let statics: Vec<String> = sent.plan.reloads[0]
			.statics
			.iter()
			.map(|local| format!("{} {}", local.name, local.constant))
			.collect();
		assert_eq!(statics, ["LIMIT false"]);
		let reads: Vec<String> = sent
			.local_reads
			.iter()
			.map(|reads| format!("{} {:?}", reads.name, reads.parsed))
			.collect();
		assert_eq!(reads, ["LIMIT Some(Unknown)"]);
	}

	fn sent_statics(sent: &HotReloadRequest) -> Vec<String> {
		sent.plan
			.reloads
			.iter()
			.flat_map(|reload| reload.statics.iter().map(|local| format!("{} {:?}", local.name, local.scope)))
			.collect()
	}

	#[test]
	fn a_request_keeps_a_static_in_its_branch_when_a_branch_is_added_above() {
		let before = "/proc/f(x)\n\tif (x == 1)\n\t\tvar/static/n = 0\n\t\tn++\n\t\treturn n\n";
		let added = "\tif (x == 2)\n\t\tvar/static/n = 10\n\t\tn++\n\t\treturn n\n";
		let after = before.replace("\tif (x == 1)", &format!("{added}\tif (x == 1)"));
		let (old, changes) = edited("branch-added", before, &after);
		let sent = request(&changes, &old, false);
		assert_eq!(sent_statics(&sent), ["n Proc { path: \"/proc/f\", nth: 0 }"]);
		let kept: Vec<String> = sent.local_reads.iter().map(|reads| format!("{} {:?}", reads.name, reads.kept)).collect();
		assert_eq!(kept, ["n [(1, 0)]"]);

		let (old, changes) = edited("branch-edited", before, &before.replace("x == 1", "x > 0"));
		let sent = request(&changes, &old, false);
		assert!(sent_statics(&sent).is_empty(), "expected no statics sent, got {:?}", sent_statics(&sent));
		assert_eq!(sent.local_reads[0].kept, [(0, 0)]);
	}

	#[test]
	fn a_request_is_refused_when_locals_of_a_name_cant_be_matched() {
		let before = "/proc/f(x)\n\tif (x == 1)\n\t\tvar/static/n = 0\n\t\tn++\n\t\treturn n\n";
		let added = "\tif (x == 2)\n\t\tvar/static/n = 10\n\t\tn++\n\t\treturn n\n";
		let after = before.replace("\tif (x == 1)", &format!("{added}\tif (x == 3)"));
		let (old, changes) = edited("branch-unmatched", before, &after);
		let refused: Vec<String> = unmatched_locals(&changes, &old).iter().map(ToString::to_string).collect();
		assert_eq!(refused, ["code.dm:3: /f: n: the edit leaves 2 static or const locals of this name \
		                      where there were 1, in blocks of the same kinds. Which of them keep their \
		                      values can't be told. Rebuild to apply it"]);
	}

	#[test]
	fn a_request_sends_the_reads_of_a_local_that_the_new_code_renamed() {
		let before = "/proc/f()\n\tvar/static/n = 7\n\tn++\n\treturn n\n";
		let (old, changes) = edited("renamed", before, "/proc/f()\n\tvar/m = 7\n\tm++\n\treturn m\n");
		let sent = request(&changes, &old, false);
		let reads: Vec<String> = sent
			.local_reads
			.iter()
			.map(|reads| format!("{} {} {:?}", reads.proc, reads.name, reads.kept))
			.collect();
		assert_eq!(reads, ["/f n []"]);
	}

	#[test]
	fn a_request_leaves_out_a_static_that_neither_code_reads() {
		let before = "var/state = 0\n/proc/register()\n\treturn 1\n/proc/f()\n\tvar/static/state = register()\n\treturn global.state + 1\n";
		let (old, changes) = edited("unread-static", before, &before.replace("global.state + 1", "global.state + 2"));
		assert_eq!(changes.plan.reloads[0].statics.len(), 1, "{:?}", changes.plan);
		let sent = request(&changes, &old, false);
		let statics: Vec<&str> = sent.plan.reloads[0].statics.iter().map(|local| local.name.as_str()).collect();
		assert!(statics.is_empty(), "expected no statics sent, got {:?}", statics);
	}

	#[test]
	fn a_request_is_the_plan_without_the_statics_the_world_has() {
		let before = "/datum/a\n\tvar/old = 1\n/proc/f()\n\tvar/static/kept = 1\n\treturn kept\n/proc/g()\n\treturn 1\n";
		let after = "/datum/a\n/proc/f()\n\tvar/static/kept = 1\n\tvar/static/added = 2\n\treturn kept + added\n/proc/g()\n\treturn 2\n";
		let (old, changes) = edited("statics", before, after);
		let statics = |plan: &dmasm::reload::Plan| -> Vec<String> {
			let f = plan.reloads.iter().find(|reload| reload.proc.path == "/f").expect("f isn't reloaded");
			f.statics.iter().map(|local| local.name.clone()).collect()
		};
		assert_eq!(statics(&changes.plan), ["kept", "added"]);

		let sent = request(&changes, &old, false);
		assert_eq!(statics(&sent.plan), ["added"]);
		let procs = |plan: &dmasm::reload::Plan| plan.reloads.iter().map(|reload| reload.proc.to_string()).collect::<Vec<_>>();
		assert_eq!(procs(&sent.plan), procs(&changes.plan));
		assert_eq!(sent.plan.removed_vars, changes.plan.removed_vars);
		assert_eq!(sent.plan.removed_vars.len(), 1);
		assert_eq!(sent.plan.len(), changes.plan.len());
		assert!(sent.resources.is_empty() && sent.resource_dirs.is_empty());
	}

	#[test]
	fn a_request_names_the_procs_whose_locals_share_a_changed_globals_name() {
		let before = "var/const/N = 5\n/proc/a()\n\treturn N\n/proc/b()\n\tvar/static/N = 7\n\treturn N\n/proc/c()\n\tif \
		              (prob(50))\n\t\tvar/const/N = 8\n\t\treturn N\n/proc/d()\n\tvar/static/N = 9\n\treturn N + global.N\n";
		let (old, changes) = edited("shadowing", before, &before.replace("N = 5", "N = 10"));
		assert_eq!(changes.plan.redeclared.len(), 1, "{:?}", changes.plan);
		let sent = request(&changes, &old, false);
		let procs: Vec<String> = sent
			.shadowing_procs
			.iter()
			.map(|p| format!("{} {} {:?}", p.global, p.proc.path, p.reads))
			.collect();
		assert_eq!(procs, ["N /b [Some(0)]", "N /c [Some(0)]", "N /d [Some(0), None]"]);
	}

	#[test]
	fn the_global_init_proc_goes_to_a_world_that_restarted() {
		let (old, changes) = edited("global-init", "var/list/all = list(1)\n", "var/list/all = list(2)\n");
		assert!(changes.global_init.is_some(), "{:?}", changes.deferred);
		assert!(request(&changes, &old, false).global_init.is_none());
		let restarted = request(&changes, &old, true);
		assert_eq!(restarted.global_init, changes.global_init);
		assert!(changes.plan.is_empty() && !restarted.is_empty());
	}

	#[test]
	fn merged_changes_reload_a_proc_once() {
		let before = "/proc/f()\n\treturn 1\n/proc/g()\n\treturn 1\n";
		let (_, mut changes) = edited("merge-f", before, &before.replace("/proc/f()\n\treturn 1", "/proc/f()\n\treturn 2"));
		let (_, both) = edited("merge-both", before, &before.replace("return 1", "return 2"));
		merge(&mut changes, both);
		let mut procs: Vec<&str> = changes.plan.reloads.iter().map(|reload| reload.proc.path.as_str()).collect();
		procs.sort_unstable();
		assert_eq!(procs, ["/f", "/g"]);
	}
}
