use super::*;

impl Server {
	pub(super) fn handle_render(&mut self, frame_id: Option<u32>, expression: &str, depth: u32) {
		let rendered = match expression.strip_prefix('#') {
			Some(command) => {
				self.eval_warnings.clear();
				self.eval_callee_errors.clear();
				self.eval_callee_more = 0;
				Ok(Rendered::Text(self.handle_command(frame_id, command)))
			}
			None => self.eval_expr(frame_id, expression).map(|value| render(&value, depth))
		};
		self.send_callee_errors();
		self.answer(Response::Render(rendered));
	}

	pub(super) fn handle_render_variables(&mut self, vars: VariablesRef, depth: u32) {
		let rendered = self
			.values_in(vars)
			.map(|values| values.iter().map(|(name, value)| (name.clone(), render(value, depth))).collect());
		self.answer(Response::RenderVariables(rendered));
	}

	fn values_in(&self, vars: VariablesRef) -> Result<Vec<(String, Value)>, String> {
		let container = self
			.state
			.as_ref()
			.and_then(|state| state.get_variables(vars))
			.ok_or("variable reference expired after execution resumed")?;
		let gone = || "frame no longer exists".to_owned();
		match container {
			Variables::Arguments { frame } => {
				let frame = self.get_stack_frame(frame).ok_or_else(gone)?;
				let arguments = values::frame_arguments(frame).into_iter();
				Ok(arguments.map(|(name, _, value)| (name, value.clone())).collect())
			}
			Variables::Locals { frame } => {
				let frame = self.get_stack_frame(frame).ok_or_else(gone)?;
				let locals = values::frame_locals(frame).into_iter();
				Ok(locals.map(|(name, value)| (name, value.clone())).collect())
			}
			Variables::Globals | Variables::Builtins => {
				let builtin = matches!(container, Variables::Builtins);
				let builtins = values::builtin_constants();
				let names = List::from_value(&unsafe { Value::new(ValueTag::GlobalVars, ValueData { id: 0 }) }).map_err(|e| e.message)?;
				let mut values = vec![];
				for i in 1..=names.len() {
					let name = names.get(i).and_then(|name| name.as_string()).map_err(|e| e.message)?;
					if builtins.contains(&name) != builtin {
						continue;
					}
					let value = StringRef::new(name.as_str())
						.and_then(|var| Value::GLOBAL.get(var))
						.map_err(|e| e.message)?;
					values.push((name, value));
				}
				values.sort_by_key(|(name, _)| name.to_lowercase());
				Ok(values)
			}
			Variables::ObjectVars(object, _) => Ok(object_values(&object)),
			_ => Err("rendering supports frame variables, globals and object variables".to_owned())
		}
	}
}

pub(super) fn object_values(object: &Value) -> Vec<(String, Value)> {
	let names: Vec<String> = match values::primitive_fields(object) {
		Some(fields) => fields.iter().map(|&name| name.to_owned()).collect(),
		None => {
			let Ok(vars) = object.get(byond_string!("vars")).and_then(|vars| List::from_value(&vars)) else {
				return vec![];
			};
			let mut names: Vec<String> = (1..=vars.len())
				.filter_map(|i| vars.get(i).and_then(|name| name.as_string()).ok())
				.collect();
			names.sort_by_key(|name| name.to_lowercase());
			names
		}
	};
	names
		.into_iter()
		.filter(|name| name != "vars")
		.filter_map(|name| {
			let value = StringRef::new(name.as_str()).and_then(|var| object.get(var)).ok()?;
			Some((name, value))
		})
		.collect()
}

pub(super) fn render(value: &Value, depth: u32) -> Rendered {
	match value.raw.tag {
		ValueTag::Null => return Rendered::Null,
		ValueTag::Number => return Rendered::Number(unsafe { value.raw.data.number }),
		ValueTag::String => return Rendered::Text(value.as_string().unwrap_or_default()),
		_ => {}
	}

	if List::is_list(value) {
		let Ok(list) = List::from_value(value) else {
			return Rendered::Other {
				kind: "/list".to_owned(),
				text: values::stringify(value)
			};
		};
		let entries = (depth > 0).then(|| entries_of(&list, depth - 1));
		return Rendered::List { len: list.len(), entries };
	}

	if values::is_object(value) {
		let name = value.get(byond_string!("name")).ok().and_then(|name| name.as_string().ok());
		let vars = (depth > 1).then(|| {
			object_values(value)
				.iter()
				.map(|(name, value)| (name.clone(), render(value, depth - 1)))
				.collect()
		});
		return Rendered::Object {
			reference: reference(value),
			kind: values::kind(value).unwrap_or_default(),
			name,
			vars
		};
	}

	Rendered::Other {
		kind: values::kind(value).unwrap_or_default(),
		text: values::stringify(value)
	}
}

pub(super) fn entries_of(list: &List, depth: u32) -> Vec<(Rendered, Option<Rendered>)> {
	if list.is_alist() {
		return list
			.alist_pairs()
			.unwrap_or_default()
			.iter()
			.map(|(key, value)| (render(key, depth), Some(render(value, depth))))
			.collect();
	}
	let mut entries = vec![];
	for i in 1..=list.len() {
		let Ok(key) = list.get(i) else {
			continue;
		};
		let value = values::associated(list, &key).map(|value| render(&value, depth));
		entries.push((render(&key, depth), value));
	}
	entries
}

pub(super) fn reference(value: &Value) -> String {
	let id = unsafe { value.raw.data.id };
	format!("[0x{:x}]", (value.raw.tag.0 as u32) << 24 | id)
}
