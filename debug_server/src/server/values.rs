use super::*;

const MOST_COMPLETIONS: usize = 200;

#[derive(Clone, Hash, PartialEq, Eq)]
pub(super) enum Variables {
	Arguments {
		frame: u32
	},
	Locals {
		frame: u32
	},
	Globals {
		frame: u32
	},
	Builtins,
	ObjectVars(Value, Option<String>, Option<u32>),
	ListContents(Value, Option<String>, Option<u32>),
	ListPair {
		key: Value,
		of: Value,
		entry: Option<String>,
		frame: Option<u32>
	},
	Pointee(Value, Option<String>, Option<u32>)
}

impl Variables {
	const fn frame(&self) -> Option<u32> {
		match self {
			Variables::Arguments { frame } | Variables::Locals { frame } | Variables::Globals { frame } => Some(*frame),
			Variables::ObjectVars(.., frame) | Variables::ListContents(.., frame) | Variables::Pointee(.., frame) => *frame,
			Variables::ListPair { frame, .. } => *frame,
			Variables::Builtins => None
		}
	}
}

pub(crate) fn visible_locals(frame: &debug::StackFrame) -> Vec<Option<String>> {
	let names: Vec<String> = frame.locals.iter().map(|(name, _)| String::from(name)).collect();
	let shared = |name: &String| names.iter().filter(|other| *other == name).count() > 1;
	if !names.iter().any(shared) {
		return names.into_iter().map(Some).collect();
	}
	let code = Codes::old_code_of(frame).unwrap_or_else(|| code_index::of(&frame.proc));
	let at = frame.offset as u32;
	let rank = |slot: usize| {
		code.local_use(slot as u32)
			.filter(|(first, _)| *first <= at)
			.map(|(first, last)| (at <= last, first))
	};
	names
		.iter()
		.enumerate()
		.map(|(slot, name)| {
			if !shared(name) {
				return Some(name.clone());
			}
			let mine = rank(slot)?;
			let meant = names
				.iter()
				.enumerate()
				.filter(|(other, other_name)| *other != slot && *other_name == name)
				.filter_map(|(other, _)| rank(other))
				.all(|theirs| theirs < mine);
			meant.then(|| name.clone())
		})
		.collect()
}

pub(super) const fn primitive_fields(value: &Value) -> Option<&'static [&'static str]> {
	match value.raw.tag {
		ValueTag::Vector => Some(&["x", "y", "z", "len", "size"]),
		ValueTag::PixLoc => Some(&["x", "y", "z", "step_x", "step_y", "loc"]),
		ValueTag::Callee => Some(&["proc", "file", "line", "src", "usr", "args", "caller"]),
		ValueTag::SaveFile => Some(&["type", "name", "cd", "dir", "eof", "byond_version", "byond_build"]),
		_ => None
	}
}

pub(super) fn is_object(value: &Value) -> bool {
	if value.raw.tag == ValueTag::World && unsafe { value.raw.data.id == 1 } {
		return true;
	}

	primitive_fields(value).is_some() || value.get(byond_string!("vars")).is_ok_and(|vars| List::is_list(&vars))
}

pub(super) fn stringify(value: &Value) -> String {
	if List::is_list(value) {
		match List::from_value(value) {
			Ok(list) => format!("/{} {{len = {}}}", if list.is_alist() { "alist" } else { "list" }, list.len()),
			Err(Runtime { message }) => format!("/list (failed to get len: {:?})", message)
		}
	} else {
		match value.to_string() {
			Ok(v) if v.is_empty() => match value.raw.tag {
				ValueTag::World => "world".to_owned(),
				ValueTag::Pointer => "a pointer to a var that's gone".to_owned(),
				ValueTag::Image => "/image".to_owned(),
				ValueTag::Appearance => "appearance".to_owned(),
				_ => value.raw.to_string()
			},
			Ok(value) => value,
			Err(Runtime { message }) => {
				format!("{} -- stringify error: {:?}", value.raw, message)
			}
		}
	}
}

pub(super) fn kind(value: &Value) -> Option<String> {
	use ValueTag as Tag;
	if List::is_list(value) {
		return Some(
			if List::from_value(value).is_ok_and(|list| list.is_alist()) {
				"/alist"
			} else {
				"/list"
			}
			.to_owned()
		);
	}
	let word = match value.raw.tag {
		Tag::Null => "null",
		Tag::Number => "num",
		Tag::String => "text",
		Tag::World => "/world",
		Tag::Resource | Tag::File => "file",
		Tag::ProcRef => "proc",
		Tag::Pointer => "pointer",
		Tag::Appearance => "appearance",
		Tag::Vector => "/vector",
		Tag::PixLoc => "/pixloc",
		Tag::Callee => "/callee",
		Tag::MobTypepath
		| Tag::ObjTypepath
		| Tag::TurfTypepath
		| Tag::AreaTypepath
		| Tag::DatumTypepath
		| Tag::SaveFileTypepath
		| Tag::ListTypepath
		| Tag::ClientTypepath
		| Tag::ImageTypepath
		| Tag::BuiltinTypepath => "path",
		_ => return value.get(byond_string!("type")).ok().map(|path| stringify(&path))
	};
	Some(word.to_owned())
}

pub(super) fn literal(value: &Value) -> Option<String> {
	use ValueTag as Tag;
	match value.raw.tag {
		Tag::String => text_literal(StringRef::from_value(value.clone())?.data()),
		Tag::Number | Tag::MobTypepath | Tag::ObjTypepath | Tag::TurfTypepath | Tag::AreaTypepath | Tag::DatumTypepath => Some(stringify(value)),
		_ => None
	}
}

pub(super) fn text_literal(bytes: &[u8]) -> Option<String> {
	let mut literal = String::with_capacity(bytes.len() + 2);
	literal.push('"');
	for c in std::str::from_utf8(bytes).ok()?.chars() {
		match c {
			'\\' | '"' | '[' | ']' => {
				literal.push('\\');
				literal.push(c);
			}
			'\n' => literal.push_str("\\n"),
			'\t' => literal.push_str("\\t"),
			_ if c.is_control() => return None,
			_ => literal.push(c)
		}
	}
	literal.push('"');
	Some(literal)
}

pub(super) fn associated(list: &List, key: &Value) -> Option<Value> {
	if key.raw.tag == ValueTag::Number {
		return None;
	}
	list.get(key).ok().filter(|value| value.raw.tag != ValueTag::Null)
}

pub(super) fn entry_expr(list: &str, key: &Value, position: Option<u32>) -> Option<String> {
	use ValueTag as Tag;
	if let Some(key) = literal(key) {
		return Some(format!("{}[{}]", list, key));
	}
	let locatable = matches!(
		key.raw.tag,
		Tag::Turf | Tag::Obj | Tag::Mob | Tag::Area | Tag::Client | Tag::Image | Tag::Datum | Tag::List | Tag::Alist
	);
	if locatable {
		return Some(format!("{}[locate(\"\\{}\")]", list, render::reference(key)));
	}
	position.map(|i| format!("{}[{}[{}]]", list, list, i))
}

pub(super) fn entries(value: &Value) -> Option<u32> {
	if !List::is_list(value) {
		return None;
	}
	List::from_value(value).ok().map(|list| list.len())
}

pub(super) fn builtin_constants() -> std::collections::HashSet<String> {
	let mut names = std::collections::HashSet::new();
	let mut in_consts = false;
	for line in crate::stddef::get_stddef().unwrap_or_default().lines() {
		if line.trim_end() == "var/const" {
			in_consts = true;
		} else if in_consts && line.starts_with([' ', '\t']) {
			if let Some(name) = line.split(['=', '/']).next().map(str::trim).filter(|name| !name.is_empty()) {
				names.insert(name.to_owned());
			}
		} else if !line.trim().is_empty() {
			in_consts = false;
		}
	}
	names
}

pub(super) fn frame_arguments(frame: &debug::StackFrame) -> Vec<(String, String, &Value)> {
	let mut values = vec![
		("src".to_owned(), "src".to_owned(), &frame.src),
		("usr".to_owned(), "usr".to_owned(), &frame.usr),
	];

	let mut unnamed_count = 0;
	for (i, (name, value)) in frame.args.iter().enumerate() {
		let (name, expr) = match name {
			Some(name) => (String::from(name), String::from(name)),
			None => {
				unnamed_count += 1;
				(format!("undefined argument #{}", unnamed_count), format!("args[{}]", i + 1))
			}
		};
		values.push((name, expr, value));
	}

	values
}

pub(super) fn argument_index(frame: &debug::StackFrame, name: &str) -> Option<usize> {
	let unnamed: Option<usize> = name.strip_prefix("undefined argument #").and_then(|n| n.parse().ok());
	let mut without_names = 0;
	frame.args.iter().position(|(arg, _)| match arg {
		Some(arg) => String::from(arg) == name,
		None => {
			without_names += 1;
			unnamed == Some(without_names)
		}
	})
}

pub(super) fn frame_locals(frame: &debug::StackFrame) -> Vec<(String, &Value)> {
	let mut values = vec![(".".to_owned(), &frame.dot)];
	let visible = visible_locals(frame).into_iter().zip(&frame.locals);
	values.extend(visible.filter_map(|(name, (_, local))| Some((name?, local))));
	values
}

pub(super) fn types_of(value: &Value) -> Vec<String> {
	let mut types = vec![];
	let mut path = value.get(byond_string!("type")).ok();
	while let Some(this) = path.filter(|path| path.raw.tag != ValueTag::Null) {
		let name = stringify(&this);
		if !name.starts_with('/') || types.contains(&name) || types.len() > 64 {
			break;
		}
		types.push(name);
		path = this.get(byond_string!("parent_type")).ok();
	}
	types
}

pub(super) fn members_of(value: &Value, found: &mut BTreeMap<String, bool>) {
	if let Some(fields) = primitive_fields(value) {
		found.extend(fields.iter().map(|&name| (name.to_owned(), false)));
	} else if let Ok(vars) = value.get(byond_string!("vars")).and_then(|vars| List::from_value(&vars)) {
		for i in 1..=vars.len() {
			if let Ok(name) = vars.get(i).and_then(|name| name.as_string()) {
				found.insert(name, false);
			}
		}
	}

	let types = types_of(value);
	if types.is_empty() {
		return;
	}
	each_proc(|owner, name| {
		if types.iter().any(|path| path == owner) {
			found.entry(name.to_owned()).or_insert(true);
		}
	});
}

pub(super) fn each_proc(mut each: impl FnMut(&str, &str)) {
	for id in 0..auxtools::runtime_tables::proc_count() {
		let Some(proc) = Proc::from_id(raw_types::procs::ProcId(id)) else {
			continue;
		};
		let Some((owner, name)) = proc.path.rsplit_once('/') else {
			continue;
		};
		if name.starts_with("auxtools_") || name.ends_with(')') || name.is_empty() {
			continue;
		}
		each(owner, name);
	}
}

pub(super) fn entry_number(name: &str) -> Option<u32> {
	name.strip_prefix('[')?.strip_suffix(']')?.parse().ok().filter(|&n| n > 0)
}

impl Server {
	pub(super) fn handle_scopes(&mut self, frame_id: u32) {
		let response = match &self.pause.state {
			Some(state) => Response::Scopes {
				arguments: Some(state.get_ref(Variables::Arguments { frame: frame_id })),
				locals: Some(state.get_ref(Variables::Locals { frame: frame_id })),
				globals: Some(state.get_ref(Variables::Globals { frame: frame_id }))
			},
			None => Response::Scopes {
				arguments: None,
				locals: None,
				globals: None
			}
		};

		self.answer(response);
	}

	pub(super) fn handle_variables(&mut self, vars: VariablesRef, start: Option<u32>, count: Option<u32>) {
		let listed = match self.pause.state.as_ref().and_then(|state| state.get_variables(vars)) {
			Some(container) => self.pause.list_variables(container, start, count),
			None => Ok(vec![])
		};

		let vars = listed.unwrap_or_else(|e| {
			self.notify(format!("Couldn't list variables: {}", e.message));
			vec![]
		});
		self.answer(Response::Variables { vars });
	}

	pub(super) fn complete(&mut self, frame_id: Option<u32>, base: Option<&str>, prefix: &str) -> Vec<Completion> {
		let mut found: BTreeMap<String, bool> = BTreeMap::new();
		match base {
			Some(base) => {
				let Ok(value) = self.read_expr(frame_id, base) else {
					return vec![];
				};
				members_of(&value, &mut found);
			}

			None => {
				let mut src = None;
				if let Some(frame) = frame_id.and_then(|frame_id| self.pause.get_stack_frame(frame_id)) {
					for name in [".", "src", "usr", "args"] {
						found.insert(name.to_owned(), false);
					}
					found.extend(
						frame
							.args
							.iter()
							.filter_map(|(name, _)| name.as_ref())
							.map(|name| (String::from(name), false))
					);
					found.extend(visible_locals(&frame).into_iter().flatten().map(|name| (name, false)));
					src = Some(frame.src.clone()).filter(|src| *src != Value::NULL);
				}
				if let Some(src) = &src {
					members_of(src, &mut found);
				}

				for name in ["world", "global"] {
					found.entry(name.to_owned()).or_insert(false);
				}
				if let Ok(names) = List::from_value(&unsafe { Value::new(ValueTag::GlobalVars, ValueData { id: 0 }) }) {
					for i in 1..=names.len() {
						if let Ok(name) = names.get(i).and_then(|name| name.as_string()) {
							found.entry(name).or_insert(false);
						}
					}
				}
				for name in hot_reload::added_globals() {
					found.entry(name).or_insert(false);
				}
				each_proc(|owner, name| {
					if owner.is_empty() {
						found.entry(name.to_owned()).or_insert(true);
					}
				});
			}
		}

		let prefix = prefix.to_lowercase();
		found
			.into_iter()
			.filter(|(name, _)| name.to_lowercase().starts_with(&prefix))
			.take(MOST_COMPLETIONS)
			.map(|(name, proc)| Completion { name, proc })
			.collect()
	}

	pub(super) fn set_variable(&mut self, vars: VariablesRef, name: &str, value: &str, notes: &mut EvalNotes) -> Result<Variable, String> {
		let container = self
			.pause
			.state
			.as_ref()
			.and_then(|state| state.get_variables(vars))
			.ok_or("variable reference expired after execution resumed")?;
		let outcome = self.eval_expr(EvalRequest::new(container.frame(), value));
		*notes = outcome.notes;
		self.pause.state.as_mut().unwrap().invalidate_stacks();
		let value = outcome.value?;

		self.eval.in_eval = true;
		auxtools::instruction_hook::hear_all();
		self.eval.eval_error = None;
		let written = self.pause.write_variable(&container, name, &value);
		self.eval.in_eval = false;
		self.refresh_watches();
		self.pause.state.as_mut().unwrap().invalidate_stacks();
		if let Some(error) = self.eval.eval_error.take() {
			return Err(format!("runtime error: {}", error));
		}
		written?;

		if let Variables::ListPair { entry, .. } = &container {
			return Ok(self.pause.value_to_variable(name.to_owned(), &value, entry.clone(), container.frame()));
		}

		let (start, count) = match &container {
			Variables::ListContents(..) => (entry_number(name).map(|n| n - 1), Some(1)),
			_ => (None, None)
		};
		let listed = self.pause.list_variables(container, start, count).map_err(|e| e.message)?;
		listed
			.into_iter()
			.find(|variable| variable.name == name)
			.ok_or_else(|| format!("{} is gone", name))
	}
}

impl Pausing {
	pub(super) fn value_to_variable(&self, name: String, value: &Value, expr: Option<String>, frame: Option<u32>) -> Variable {
		Variable {
			name,
			value: stringify(value),
			variables: self.value_to_variables_ref(value, expr.clone(), frame),
			kind: kind(value),
			entries: entries(value),
			expr
		}
	}

	pub(super) fn value_to_variables_ref(&self, value: &Value, expr: Option<String>, frame: Option<u32>) -> Option<VariablesRef> {
		match self.state.as_ref() {
			Some(state) if List::is_list(value) => Some(state.get_ref(Variables::ListContents(value.clone(), expr, frame))),

			Some(state) if is_object(value) => Some(state.get_ref(Variables::ObjectVars(value.clone(), expr, frame))),

			Some(state) if value.raw.tag == ValueTag::Pointer && auxtools::pointer::read_raw(value).is_some() => {
				Some(state.get_ref(Variables::Pointee(value.clone(), expr, frame)))
			}

			_ => None
		}
	}

	pub(super) fn list_to_variables(
		&mut self,
		value: &Value,
		expr: Option<&str>,
		frame: Option<u32>,
		start: Option<u32>,
		count: Option<u32>
	) -> Result<Vec<Variable>, Runtime> {
		let state = self.state.as_ref().unwrap();
		let list = List::from_value(value)?;
		let len = list.len();
		let first = start.unwrap_or(0).min(len);
		let last = count.map_or(len, |count| first.saturating_add(count).min(len));

		let alist = list.is_alist();
		let pair = |i: u32, key: Value, value: Value| {
			let entry = expr.and_then(|list| entry_expr(list, &key, (!alist).then_some(i)));
			Variable {
				name: format!("[{}]", i),
				value: format!("{} = {}", stringify(&key), stringify(&value)),
				kind: kind(&value),
				entries: None,
				expr: entry.clone(),
				variables: Some(state.get_ref(Variables::ListPair {
					key,
					of: Value::from(&list),
					entry,
					frame
				}))
			}
		};

		if alist {
			return Ok(list
				.alist_pairs()?
				.into_iter()
				.enumerate()
				.skip(first as usize)
				.take((last - first) as usize)
				.map(|(i, (key, value))| pair(i as u32 + 1, key, value))
				.collect());
		}

		let mut variables = vec![];

		for i in first + 1..=last {
			let key = list.get(i)?;

			if let Some(value) = associated(&list, &key) {
				variables.push(pair(i, key, value));
				continue;
			}

			variables.push(self.value_to_variable(format!("[{}]", i), &key, expr.map(|list| format!("{}[{}]", list, i)), frame));
		}

		Ok(variables)
	}

	pub(super) fn object_to_variables(&mut self, value: &Value, expr: Option<&str>, frame: Option<u32>) -> Result<Vec<Variable>, Runtime> {
		let member = |name: &str| expr.map(|object| format!("{}.{}", object, name));

		if let Some(fields) = primitive_fields(value) {
			return fields
				.iter()
				.map(|&name| Ok(self.value_to_variable(name.to_owned(), &value.get(StringRef::new(name)?)?, member(name), frame)))
				.collect();
		}

		let vars = List::from_value(&value.get(byond_string!("vars"))?)?;

		let mut variables = vec![];
		let mut listed_first = vec![];

		for i in 1..=vars.len() {
			let name = vars.get(i)?.as_string()?;
			let value = value.get(StringRef::new(name.as_str())?)?;
			let expr = member(&name);
			let variable = self.value_to_variable(name, &value, expr, frame);
			if variable.name == "type" {
				listed_first.push(variable);
			} else {
				variables.push(variable);
			}
		}

		variables.sort_by_key(|a| a.name.to_lowercase());
		listed_first.append(&mut variables);

		Ok(listed_first)
	}

	pub(super) fn globals_to_variables(&mut self, builtin: bool, frame: Option<u32>) -> Result<Vec<Variable>, Runtime> {
		let names = List::from_value(&unsafe { Value::new(ValueTag::GlobalVars, ValueData { id: 0 }) })?;
		let builtins = builtin_constants();

		let mut variables = vec![];
		let mut constants = 0;
		for i in 1..=names.len() {
			let name = names.get(i)?.as_string()?;
			if builtins.contains(&name) != builtin {
				constants += 1;
				continue;
			}
			let value = Value::GLOBAL.get(StringRef::new(name.as_str())?)?;
			let expr = format!("global.{}", name);
			variables.push(self.value_to_variable(name, &value, Some(expr), frame));
		}
		variables.sort_by_key(|a| a.name.to_lowercase());

		if !builtin && constants > 0 {
			variables.push(Variable {
				name: "(BYOND's)".to_owned(),
				value: format!("{} constants", constants),
				variables: self.state.as_ref().map(|state| state.get_ref(Variables::Builtins)),
				kind: None,
				expr: None,
				entries: None
			});
		}
		Ok(variables)
	}

	pub(super) fn get_args(&mut self, frame_index: u32) -> Vec<Variable> {
		let Some(frame) = self.get_stack_frame(frame_index) else {
			return vec![];
		};
		frame_arguments(frame)
			.into_iter()
			.map(|(name, expr, value)| self.value_to_variable(name, value, Some(expr), Some(frame_index)))
			.collect()
	}

	pub(super) fn get_locals(&mut self, frame_index: u32) -> Vec<Variable> {
		let Some(frame) = self.get_stack_frame(frame_index) else {
			return vec![];
		};
		frame_locals(frame)
			.into_iter()
			.map(|(name, value)| self.value_to_variable(name.clone(), value, Some(name), Some(frame_index)))
			.collect()
	}

	pub(super) fn list_variables(&mut self, container: Variables, start: Option<u32>, count: Option<u32>) -> Result<Vec<Variable>, Runtime> {
		match container {
			Variables::Arguments { frame } => Ok(self.get_args(frame)),
			Variables::Locals { frame } => Ok(self.get_locals(frame)),
			Variables::Globals { frame } => self.globals_to_variables(false, Some(frame)),
			Variables::Builtins => self.globals_to_variables(true, None),
			Variables::ObjectVars(value, expr, frame) => self.object_to_variables(&value, expr.as_deref(), frame),
			Variables::ListContents(value, expr, frame) => self.list_to_variables(&value, expr.as_deref(), frame, start, count),
			Variables::ListPair { key, of, entry, frame } => {
				let value = List::from_value(&of)?.get(&key)?;
				Ok(vec![
					self.value_to_variable("key".to_owned(), &key, None, frame),
					self.value_to_variable("value".to_owned(), &value, entry, frame),
				])
			}
			Variables::Pointee(pointer, expr, frame) => Ok(match auxtools::pointer::read(&pointer) {
				Some(named) => vec![self.value_to_variable("*".to_owned(), &named, expr.map(|pointer| format!("*{}", pointer)), frame)],
				None => vec![]
			})
		}
	}

	pub(super) fn write_variable(&mut self, container: &Variables, name: &str, value: &Value) -> Result<(), String> {
		let runtime = |e: Runtime| format!("runtime error: {}", e.message);
		unsafe fn put(slot: *mut raw_types::values::Value, value: &Value) {
			unsafe {
				let _ = Value::from_raw_owned(*slot);
				let value = value.clone();
				*slot = value.raw;
				std::mem::forget(value);
			}
		}

		match container {
			Variables::Locals { frame } => {
				let frame = self.get_stack_frame(*frame).ok_or("frame no longer exists")?;
				if name == "." {
					unsafe { put((*frame.context).dot_mut(), value) };
					return Ok(());
				}
				let visible = visible_locals(&frame);
				let mut named = visible.iter().enumerate().filter(|(_, local)| local.as_deref() == Some(name));
				match (named.next(), named.next()) {
					(Some((index, _)), None) => {
						unsafe { put((*(*frame.context).locals()).add(index), value) };
						Ok(())
					}
					(Some(_), Some(_)) => Err(format!("the proc has more than one local called {}", name)),
					_ => Err(format!("the frame has no local called {}", name))
				}
			}

			Variables::Arguments { frame } => {
				let frame = self.get_stack_frame(*frame).ok_or("frame no longer exists")?;
				match name {
					"src" => unsafe { put(&raw mut (*frame.instance).src, value) },
					"usr" => unsafe { put(&raw mut (*frame.instance).usr, value) },
					_ => {
						let index = argument_index(frame, name).ok_or_else(|| format!("the frame has no argument called {}", name))?;
						unsafe { put((*frame.instance).args().add(index), value) };
					}
				}
				Ok(())
			}

			Variables::Globals { .. } => Value::GLOBAL.set(StringRef::new(name).map_err(runtime)?, value).map_err(runtime),
			Variables::Builtins => Err("BYOND's constants can't be changed".to_owned()),
			Variables::ObjectVars(object, ..) => object.set(StringRef::new(name).map_err(runtime)?, value).map_err(runtime),

			Variables::ListContents(list, ..) => {
				let list = List::from_value(list).map_err(runtime)?;
				let number = entry_number(name)
					.filter(|&n| n <= list.len())
					.ok_or_else(|| format!("the list has no entry {}", name))?;
				if list.is_alist() {
					let (key, _) = list
						.alist_pairs()
						.map_err(runtime)?
						.into_iter()
						.nth(number as usize - 1)
						.ok_or("the list has changed")?;
					return list.set(&key, value).map_err(runtime);
				}
				let key = list.get(number).map_err(runtime)?;
				if associated(&list, &key).is_some() {
					list.set(&key, value).map_err(runtime)
				} else {
					list.set(number, value).map_err(runtime)
				}
			}

			Variables::ListPair { key, of, .. } => match name {
				"value" => List::from_value(of).map_err(runtime)?.set(key, value).map_err(runtime),
				_ => Err("list keys cannot be edited. Remove the entry and add a new one".to_owned())
			},

			Variables::Pointee(..) => Err("assign to the target variable to change the pointed-to value".to_owned())
		}
	}
}
