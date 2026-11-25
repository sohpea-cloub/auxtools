use dreammaker::ast::{Expression, Follow, ListAccessKind, PathOp, PropertyAccessKind, Term};

use super::*;

const RUNS_CODE: &str = "the expression may run code, so it is evaluated only when asked";

impl Server {
	pub(super) fn read_expr(&mut self, frame_id: Option<u32>, expression: &str) -> Result<Value, String> {
		let expanded = crate::macros::expand(expression)?;
		let parsed = dmasm::compiler::parse_expr(&expanded, crate::world_byond()).map_err(|e| e.to_string())?;
		let frame = match frame_id {
			Some(id) => Some(
				self.pause
					.get_stack_frame(id)
					.ok_or_else(|| format!("frame {} does not exist. The world is not paused, or it has resumed since", id))?
			),
			None => None
		};
		let was_in_eval = std::mem::replace(&mut self.eval.in_eval, true);
		auxtools::instruction_hook::hear_all();
		let value = read(&parsed, frame);
		self.eval.in_eval = was_in_eval;
		value
	}

	pub(super) fn handle_read(&mut self, frame_id: Option<u32>, expression: &str) {
		let response = self
			.read_expr(frame_id, expression)
			.map(|result| EvalResponse {
				value: values::stringify(&result),
				variables: self.pause.value_to_variables_ref(&result, Some(format!("({})", expression)), frame_id),
				kind: values::kind(&result),
				entries: values::entries(&result)
			})
			.map_err(|message| EvalError {
				message,
				still_running: false
			});
		self.answer(Response::Eval(response));
	}
}

fn read(expression: &Expression, frame: Option<&debug::StackFrame>) -> Result<Value, String> {
	let runtime = |e: Runtime| format!("runtime error: {}", e.message);
	let Expression::Base { term, follow } = expression else {
		return Err(RUNS_CODE.to_owned());
	};
	let mut value = match &term.elem {
		Term::Null => Value::NULL,
		Term::Int(number) => Value::from(*number as f32),
		Term::Float(number) => Value::from(*number),
		Term::String(text) => {
			let bytes = dmasm::compiler::text_bytes(text).ok_or_else(|| format!("invalid text: {}", text))?;
			Value::from_string_raw(&bytes).map_err(runtime)?
		}
		Term::Prefab(prefab) if prefab.vars.is_empty() && prefab.path.iter().all(|(op, _)| *op == PathOp::Slash) => {
			let path = prefab.path.to_string();
			let (tag, id) = crate::assemble_env::AssembleEnv::type_value(&path).ok_or_else(|| format!("undefined type path: {}", path))?;
			unsafe { Value::new(ValueTag(tag), ValueData { id }) }
		}
		Term::Expr(inner) => read(inner, frame)?,
		Term::Ident(name) => named(name, frame)?,
		Term::GlobalIdent(name) => global(name)?,
		_ => return Err(RUNS_CODE.to_owned())
	};
	for each in follow.iter() {
		value = match &each.elem {
			Follow::Field(kind, name) if *kind != PropertyAccessKind::Scope => {
				if value == Value::NULL && matches!(kind, PropertyAccessKind::SafeDot | PropertyAccessKind::SafeColon) {
					return Ok(Value::NULL);
				}
				value.get(StringRef::new(name).map_err(runtime)?).map_err(runtime)?
			}
			Follow::Index(kind, key) => {
				if value == Value::NULL && *kind == ListAccessKind::Safe {
					return Ok(Value::NULL);
				}
				if !List::is_list(&value) {
					return Err(RUNS_CODE.to_owned());
				}
				let key = read(key, frame)?;
				List::from_value(&value).and_then(|list| list.get(&key)).map_err(runtime)?
			}
			_ => return Err(RUNS_CODE.to_owned())
		};
	}
	Ok(value)
}

fn named(name: &str, frame: Option<&debug::StackFrame>) -> Result<Value, String> {
	if let Some(frame) = frame {
		match name {
			"src" => return Ok(frame.src.clone()),
			"usr" => return Ok(frame.usr.clone()),
			"." => return Ok(frame.dot.clone()),
			"args" => {
				let args = List::new();
				for (_, value) in &frame.args {
					args.append(value);
				}
				return Ok(Value::from(args));
			}
			_ => {}
		}
		let mut visible = values::visible_locals(frame).into_iter().zip(&frame.locals);
		if let Some((_, (_, value))) = visible.find(|(local, _)| local.as_deref() == Some(name)) {
			return Ok(value.clone());
		}
		if let Some((_, value)) = frame
			.args
			.iter()
			.find(|(arg, _)| arg.as_ref().is_some_and(|arg| String::from(arg) == name))
		{
			return Ok(value.clone());
		}
		if frame.src != Value::NULL {
			if let Ok(value) = StringRef::new(name).and_then(|var| frame.src.get(var)) {
				return Ok(value);
			}
		}
	}
	match name {
		"world" => Ok(Value::WORLD),
		"global" => Ok(Value::GLOBAL),
		_ => global(name)
	}
}

fn global(name: &str) -> Result<Value, String> {
	StringRef::new(name)
		.and_then(|var| Value::GLOBAL.get(var))
		.map_err(|_| format!("undefined var: {}", name))
}
