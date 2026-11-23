use auxtools::*;

pub struct AssembleEnv;

impl AssembleEnv {
	fn eval_with_name(code: &str, arg: &str) -> Option<Value> {
		let arg = Value::from_string(arg).ok()?;
		Self::eval_with(code, &[("name", &arg)])
	}

	fn eval_with(code: &str, params: &[(&str, &Value)]) -> Option<Value> {
		let names: Vec<&str> = params.iter().map(|(name, _)| *name).collect();
		let values: Vec<&Value> = params.iter().map(|(_, value)| *value).collect();
		let expr = dmasm::compiler::compile_expr_for(code, &names, crate::world_byond()).unwrap();
		let assembly = dmasm::assembler::assemble(&expr, &mut Self).unwrap();

		let proc = Proc::find("/proc/auxtools_expr_stub")?;
		proc.set_bytecode(assembly).ok()?;

		let res = proc.call(&values).ok()?.as_list().ok()?.get(1).ok()?;

		if res == Value::NULL {
			return None;
		}

		Some(res)
	}

	pub fn has_proc(object: &Value, name: &str) -> bool {
		let Ok(name) = Value::from_string(name) else {
			return false;
		};
		Self::eval_with("hascall(object, name)", &[("object", object), ("name", &name)]).is_some_and(|has| has.is_truthy())
	}
}

fn last_definition(path: &str) -> Option<Proc> {
	let mut last = None;
	let mut n = 0;
	while let Some(proc) = Proc::find_override(path, n) {
		last = Some(proc);
		n += 1;
	}
	last
}

fn declaration(path: &str) -> Option<Proc> {
	let overrides = Proc::overrides(path);
	overrides.iter().find(|proc| declares(proc)).or_else(|| overrides.first()).cloned()
}

fn declares(proc: &Proc) -> bool {
	let path = own_path(proc);
	path.contains("/proc/") || path.contains("/verb/")
}

pub fn own_path(proc: &Proc) -> String {
	String::from(unsafe { StringRef::from_id((*proc.entry()).path) })
}

pub fn first_override(path: &str) -> Option<Proc> {
	Proc::overrides(path).into_iter().find(|proc| own_path(proc) == path)
}

fn added_type(path: &str) -> Option<(u8, u32)> {
	let id = crate::hot_reload::type_id(path)?;
	let tag = ancestry(path)
		.iter()
		.skip(1)
		.find_map(|parent| AssembleEnv::eval_with_name("text2path(name)", parent))?
		.raw
		.tag
		.0;
	Some((tag, id))
}

fn ancestry(ty: &str) -> Vec<String> {
	let ty = ty.trim_end_matches('/');
	if let Some(paths) = crate::hot_reload::ancestry(ty) {
		return paths;
	}
	let mut paths = vec![];
	let mut at = ty;
	while !at.is_empty() {
		paths.push(at.to_owned());
		at = at.rsplit_once('/').map_or("", |(parent, _)| parent);
	}
	paths
}

fn found(proc: Proc, warning: Option<String>) -> dmasm::compiler::FoundProc {
	dmasm::compiler::FoundProc {
		tag: raw_types::values::ValueTag::ProcRef.0,
		data: proc.id.0,
		warning
	}
}

pub fn type_proc(ty: &str, name: &str) -> Result<dmasm::compiler::FoundProc, String> {
	if dmasm::assembler::AssembleEnv::get_type(&mut AssembleEnv, ty).is_none() {
		return Err(format!("there's no type {} in the running world", ty));
	}
	ancestry(ty)
		.iter()
		.find_map(|at| last_definition(&format!("{}/{}", at, name)))
		.map(|proc| found(proc, None))
		.ok_or_else(|| format!("{} has no proc called {}", ty, name))
}

pub fn proc_path(path: &str) -> Option<dmasm::compiler::FoundProc> {
	let (ty, name) = path.rsplit_once("/proc/").or_else(|| path.rsplit_once("/verb/"))?;
	if name.is_empty() || name.contains('/') {
		return None;
	}
	if ty.is_empty() {
		return declaration(path).map(|proc| found(proc, None));
	}
	let (at, proc) = ancestry(ty)
		.into_iter()
		.filter_map(|at| declaration(&format!("{}/{}", at, name)).map(|proc| (at, proc)))
		.last()?;
	let warning = (at != ty).then(|| {
		format!(
			"{}: DreamMaker does not accept this path, because {} is not declared on {}. The proc is {}/proc/{}",
			path, name, ty, at, name
		)
	});
	Some(found(proc, warning))
}

impl dmasm::assembler::AssembleEnv for AssembleEnv {
	fn get_string_index(&mut self, string: &[u8]) -> Option<u32> {
		let string = StringRef::from_raw(string).ok()?;
		let id = string.get_id();

		std::mem::forget(string);

		Some(id.0)
	}

	fn get_variable_name_index(&mut self, name: &[u8]) -> Option<u32> {
		let id = self.get_string_index(name)?;

		unsafe {
			let mut names = *raw_types::funcs::VARIABLE_NAME_ENTRIES;

			for i in 0..*raw_types::funcs::VARIABLE_NAME_COUNT {
				if (*names).0 == id {
					return Some(i);
				}

				names = names.add(1);
			}

			None
		}
	}

	fn get_proc_index(&mut self, path: &str) -> Option<u32> {
		let proc = if path.contains("/proc/") || path.contains("/verb/") {
			declaration(path)
		} else {
			Proc::find(path)
		};
		proc.map(|p| p.id.0)
	}

	fn get_type(&mut self, path: &str) -> Option<(u8, u32)> {
		match path {
			"/list" => return Some((raw_types::values::ValueTag::ListTypepath.0, 0)),
			"/savefile" => return Some((raw_types::values::ValueTag::SaveFileTypepath.0, 0)),
			"/client" => return Some((raw_types::values::ValueTag::ClientTypepath.0, 0)),
			_ => {}
		}

		if let Some(res) = Self::eval_with_name("text2path(name)", path) {
			return Some((res.raw.tag.0, unsafe { res.raw.data.id }));
		}
		if let Some(id) = crate::hot_reload::image_type(path) {
			return Some((raw_types::values::ValueTag::ImageTypepath.0, id));
		}
		if let Some(found) = added_type(path) {
			return Some(found);
		}

		first_override(path).map(|proc| (raw_types::values::ValueTag::ProcRef.0, proc.id.0))
	}

	fn get_resource_index(&mut self, path: &str) -> Option<u32> {
		if !crate::world_resources::is_loaded() {
			return crate::world_resources::built_id(&resource_file(path)?);
		}
		let res = resource_paths(path).find_map(|path| {
			Self::eval_with_name("fcopy_rsc(name)", &path).or_else(|| Self::eval_with_name("fcopy_rsc(name)", &case_insensitive_path(&path)?))
		})?;

		if res.raw.tag != raw_types::values::ValueTag::Resource {
			return None;
		}

		Some(unsafe { res.raw.data.id })
	}
}

pub(crate) static RESOURCE_DIRS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(vec![]);

fn resource_paths(path: &str) -> impl Iterator<Item = String> + '_ {
	let dirs = RESOURCE_DIRS.lock().map(|dirs| dirs.clone()).unwrap_or_default();
	std::iter::once(path.to_owned()).chain(dirs.into_iter().map(move |dir| format!("{}/{}", dir.trim_end_matches('/'), path)))
}

pub(crate) fn resource_file(path: &str) -> Option<std::path::PathBuf> {
	resource_paths(path).find_map(|path| {
		let file = std::path::Path::new(&path);
		if file.is_file() {
			return Some(file.to_owned());
		}
		case_insensitive_path(&path).map(Into::into)
	})
}

fn case_insensitive_path(path: &str) -> Option<String> {
	use std::path::{Component, Path, PathBuf};

	if Path::new(path).exists() {
		return None;
	}
	let mut found = PathBuf::new();
	for component in Path::new(path).components() {
		let Component::Normal(part) = component else {
			found.push(component);
			continue;
		};
		let exact = found.join(part);
		if exact.symlink_metadata().is_ok() {
			found = exact;
			continue;
		}
		let part = part.to_str()?;
		let dir = if found.as_os_str().is_empty() { Path::new(".") } else { found.as_path() };
		let mut names: Vec<String> = std::fs::read_dir(dir)
			.ok()?
			.filter_map(|entry| entry.ok()?.file_name().into_string().ok())
			.filter(|name| name.eq_ignore_ascii_case(part))
			.collect();
		names.sort();
		found.push(names.into_iter().next()?);
	}
	found.to_str().map(|found| found.replace('\\', "/"))
}
