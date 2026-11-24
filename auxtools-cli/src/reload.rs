use std::path::PathBuf;

use auxtools_protocol::{HotReloadRequest, LocalReads, ShadowingProc};
use dmasm::{
	compiler::definition_order,
	diagnostic::Id,
	reload::{Changes, Plan, ProcKey, Refusal, Snapshot},
	Node, StaticScope
};
use dreammaker::ast::{Spanned, Statement, VarStatement};

pub const RESTART: &str = "Rebuild the .dmb, then restart the world to load all of the new code.";

pub fn request(changes: &Changes, before: &Snapshot, restarted: bool) -> HotReloadRequest {
	let mut plan = changes.plan.clone();
	let local_reads = without_existing_statics(&mut plan, before);
	HotReloadRequest {
		shadowing_procs: shadowing_procs(&plan, before),
		local_reads,
		plan,
		resources: vec![],
		resource_dirs: vec![],
		global_init: changes.global_init.clone().filter(|_| restarted)
	}
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
	for ty in before.tree().iter_types() {
		for (name, proc) in ty.get().procs.iter() {
			for (override_id, index) in definition_order(ty, name.as_str()).into_iter().enumerate() {
				let Some(code) = proc.value.get(index).and_then(|value| value.code.as_deref()) else {
					continue;
				};
				let mut declared = vec![];
				static_locals(code, &mut declared);
				let key = ProcKey {
					path: format!("{}/{}", ty.get().path, name),
					override_id: override_id as u32
				};
				for global in globals.iter().filter(|global| declared.iter().any(|name| name == *global)) {
					let Ok(compiled) = before.compile(&key) else {
						continue;
					};
					if !reads_global(&compiled.nodes, global) {
						found.push(ShadowingProc {
							global: global.to_string(),
							proc: key.clone()
						});
					}
				}
			}
		}
	}
	found
}

fn static_locals(block: &[Spanned<Statement>], out: &mut Vec<String>) {
	for statement in block {
		statement_static_locals(&statement.elem, out);
	}
}

fn statement_static_locals(statement: &Statement, out: &mut Vec<String>) {
	let mut var = |var: &VarStatement| {
		if var.var_type.flags.is_static() || var.var_type.flags.is_const() {
			out.push(var.name.to_string());
		}
	};
	match statement {
		Statement::Var(one) => var(one),
		Statement::Vars(all) => all.iter().for_each(var),
		Statement::While { block, .. }
		| Statement::DoWhile { block, .. }
		| Statement::ForInfinite { block }
		| Statement::Spawn { block, .. }
		| Statement::Label { block, .. } => static_locals(block, out),
		Statement::ForList(each) => static_locals(&each.block, out),
		Statement::ForKeyValue(each) => static_locals(&each.block, out),
		Statement::ForRange(each) => static_locals(&each.block, out),
		Statement::ForLoop { init, inc, block, .. } => {
			for part in init.iter().chain(inc) {
				statement_static_locals(part, out);
			}
			static_locals(block, out);
		}
		Statement::If { arms, else_arm } => {
			for (_, block) in arms {
				static_locals(block, out);
			}
			if let Some(block) = else_arm {
				static_locals(block, out);
			}
		}
		Statement::Switch { cases, default, .. } => {
			for (_, block) in cases.iter() {
				static_locals(block, out);
			}
			if let Some(block) = default {
				static_locals(block, out);
			}
		}
		Statement::TryCatch { try_block, catch_block, .. } => {
			static_locals(try_block, out);
			static_locals(catch_block, out);
		}
		_ => {}
	}
}

fn reads_global(nodes: &[Node], name: &str) -> bool {
	nodes.iter().any(|node| {
		let Node::Instruction(ins, _) = node else {
			return false;
		};
		ins.variables().iter().any(|var| {
			var.globals()
				.iter()
				.any(|global| global.name.0 == name.as_bytes() && !matches!(global.scope, Some(StaticScope::Proc { .. })))
		})
	})
}

fn without_existing_statics(plan: &mut Plan, before: &Snapshot) -> Vec<LocalReads> {
	let mut found = vec![];
	for code in &mut plan.reloads {
		if code.statics.is_empty() {
			continue;
		}
		let Ok(old) = before.compile(&code.proc) else {
			continue;
		};
		let reads = local_reads(&old.nodes);
		let nodes = &code.nodes;
		code.statics.retain_mut(|local| {
			let declared = old.statics.iter().any(|had| had.name == local.name && had.scope == local.scope);
			let unread = reads
				.iter()
				.find(|(name, _)| *name == local.name)
				.is_some_and(|(_, reads)| !reads.contains(&Some(local_number(&local.scope))));
			let existing = declared && !unread;
			if !existing {
				local.constant = false;
			}
			!existing || local.constant && reads_static(nodes, &local.name, &local.scope)
		});
		found.extend(
			reads
				.into_iter()
				.filter(|(name, _)| old.statics.iter().any(|had| had.name == *name))
				.map(|(name, reads)| LocalReads {
					proc: code.proc.clone(),
					name,
					reads
				})
		);
	}
	found
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

	use super::{merge, request};

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
		let procs: Vec<(&str, &str)> = sent.shadowing_procs.iter().map(|p| (p.global.as_str(), p.proc.path.as_str())).collect();
		assert_eq!(procs, [("N", "/b"), ("N", "/c")]);
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
