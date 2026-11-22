use std::path::PathBuf;

use auxtools_protocol::HotReloadRequest;
use dmasm::reload::{Changes, Plan, Refusal, Snapshot};

pub fn request(changes: &Changes, before: &Snapshot, restarted: bool) -> HotReloadRequest {
	let mut plan = changes.plan.clone();
	without_existing_statics(&mut plan, before);
	HotReloadRequest {
		plan,
		resources: vec![],
		resource_dirs: vec![],
		global_init: changes.global_init.clone().filter(|_| restarted)
	}
}

fn without_existing_statics(plan: &mut Plan, before: &Snapshot) {
	for code in &mut plan.reloads {
		if code.statics.is_empty() {
			continue;
		}
		let Ok(old) = before.compile(&code.proc) else {
			continue;
		};
		code.statics
			.retain(|local| !old.statics.iter().any(|had| had.name == local.name && had.scope == local.scope));
	}
}

pub fn resource_names(request: &HotReloadRequest) -> Vec<String> {
	#[derive(Default)]
	struct Named(Vec<String>);

	impl Named {
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

	impl dmasm::assembler::AssembleEnv for Named {
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

	let mut named = Named::default();
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
				"",
				format!(
					"fixed parse error may have hidden definitions: {}. Rebuild and reboot before reloading",
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
		changes
			.refused
			.push(Refusal::new("", format!("compiled maps changed ({})", changed.join("; "))));
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
