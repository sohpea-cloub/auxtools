use super::*;

const GLOBALS: &str = "global vars";

const EVAL_CODE_KEPT: usize = 256;

#[derive(PartialEq, Eq, Hash)]
pub(super) struct EvalCodeKey {
	generation: u32,
	command: String,
	names: Vec<String>,
	src_type: Option<String>
}

pub(crate) struct Evaluation {
	pub(super) in_eval: bool,
	pub(super) eval_error: Option<String>,
	pub(super) eval_callee_errors: Vec<String>,
	pub(super) eval_callee_more: usize,
	pub(super) eval_ran: *mut raw_types::procs::ExecutionContext,
	pub(super) eval_code: HashMap<EvalCodeKey, Result<(Vec<u32>, Vec<String>, Vec<String>), CodeError>>
}

#[derive(Clone)]
pub(super) struct CodeError {
	text: String,
	unknown: Option<String>
}

impl CodeError {
	fn compiled(error: dmasm::compiler::CompileError) -> Self {
		use dmasm::compiler::CompileError;
		let unknown = match &error {
			CompileError::UndefinedVar(name) => Some(name.clone()),
			_ => None
		};
		CodeError {
			text: error.to_string(),
			unknown
		}
	}

	fn assembled(error: dmasm::assembler::AssembleError) -> Self {
		use dmasm::assembler::AssembleError;
		let unknown = match &error {
			AssembleError::ProcNotFound(path) => Some(path.rsplit('/').next().unwrap_or(path).to_owned()),
			_ => None
		};
		CodeError {
			text: Evaluation::assemble_error(error),
			unknown
		}
	}
}

pub(super) struct EvalRequest<'a> {
	pub(super) frame: Option<u32>,
	pub(super) command: &'a str,
	pub(super) src: Option<Value>,
	pub(super) given: Vec<(String, Value)>,
	pub(super) mode: EvalMode
}

pub(super) enum EvalMode {
	Run,
	Queue(u64)
}

impl<'a> EvalRequest<'a> {
	pub(super) fn new(frame: Option<u32>, command: &'a str) -> Self {
		Self {
			frame,
			command,
			src: None,
			given: vec![],
			mode: EvalMode::Run
		}
	}
}

pub(super) struct EvalOutcome {
	pub(super) value: Result<Value, String>,
	pub(super) still_running: bool,
	pub(super) notes: EvalNotes,
	pub(super) unknown: Option<String>
}

impl EvalOutcome {
	pub(super) fn failed(message: String) -> Self {
		Self {
			value: Err(message),
			still_running: false,
			notes: EvalNotes::default(),
			unknown: None
		}
	}
}

#[derive(Default)]
pub(super) struct EvalNotes {
	pub(super) warnings: Vec<String>,
	pub(super) callee_errors: Vec<String>,
	pub(super) callee_more: usize
}

impl Evaluation {
	pub(super) fn new() -> Self {
		Self {
			in_eval: false,
			eval_error: None,
			eval_callee_errors: vec![],
			eval_callee_more: 0,
			eval_ran: std::ptr::null_mut(),
			eval_code: HashMap::new()
		}
	}

	pub(super) fn eval_runs(&mut self, ctx: *mut raw_types::procs::ExecutionContext) {
		self.eval_ran = ctx;
	}

	pub(super) fn set_eval_error(&mut self, message: &str, ctx: *mut raw_types::procs::ExecutionContext) {
		if !Self::called_by_expression(ctx) {
			self.eval_error = Some(Self::error_with_stack(message, ctx));
		} else if self.eval_callee_errors.len() < MAX_CALLEE_ERRORS {
			let error = Self::error_with_stack(message, ctx);
			self.eval_callee_errors.push(error);
		} else {
			self.eval_callee_more += 1;
		}
	}

	pub(super) fn called_by_expression(ctx: *mut raw_types::procs::ExecutionContext) -> bool {
		if ctx.is_null() || is_expression_frame(ctx) {
			return false;
		}
		let mut at = unsafe { *(*ctx).parent_context() };
		while !at.is_null() {
			if is_expression_frame(at) {
				return true;
			}
			at = unsafe { *(*at).parent_context() };
		}
		false
	}

	pub(super) fn error_with_stack(message: &str, ctx: *mut raw_types::procs::ExecutionContext) -> String {
		let mut error = message.to_owned();
		let mut frames = vec![];
		let mut at = ctx;
		while !at.is_null() && !is_expression_frame(at) {
			frames.push(Self::context_place(at));
			at = unsafe { *(*at).parent_context() };
		}
		if !at.is_null() {
			for (i, place) in frames.iter().enumerate() {
				error += &format!("\n  {} {}", if i == 0 { "in" } else { "called by" }, place);
			}
			if !frames.is_empty() {
				error += "\n  called by the expression";
			}
		}
		error
	}

	pub(super) fn context_place(ctx: *mut raw_types::procs::ExecutionContext) -> String {
		let proc = Proc::from_id(unsafe { (**(*ctx).proc_instance()).proc });
		let mut place = match &proc {
			Some(proc) => unnamed::proc_name(proc),
			None => "a proc the world doesn't have".to_owned()
		};
		let line = unsafe { (*ctx).filename().valid().then(|| *(*ctx).line()) };
		match (proc.as_ref().and_then(code_index::file_of), line) {
			(Some(file), Some(line)) => place += &format!("  {}:{}", file, line),
			(None, Some(line)) => place += &format!("  line {}", line),
			_ => {}
		}
		place
	}

	pub(super) fn callee_notes(&mut self) -> EvalNotes {
		EvalNotes {
			warnings: vec![],
			callee_errors: std::mem::take(&mut self.eval_callee_errors),
			callee_more: std::mem::take(&mut self.eval_callee_more)
		}
	}

	pub(super) fn unknown_name(error: String, name: &str) -> String {
		let capitals = name.chars().any(|c| c.is_ascii_uppercase()) && !name.chars().any(|c| c.is_ascii_lowercase());
		if !capitals || crate::macros::given() {
			return error;
		}
		format!(
			"{}. If it is a #define, evaluation can use it only after the launcher has read the project for hot reload",
			error
		)
	}

	pub(super) fn assemble_error(error: dmasm::assembler::AssembleError) -> String {
		use dmasm::assembler::AssembleError;
		let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
		match error {
			AssembleError::ProcNotFound(path) => format!("proc not found: {}", path),
			AssembleError::VariableNameNotFound(name) => format!("variable not found: {}", text(&name)),
			AssembleError::StringNotFound(string) => format!("string not found: {}", text(&string)),
			AssembleError::TypeNotFound(path) => format!("there's no type {}", path),
			AssembleError::ResourceNotFound(path) => format!("there's no file '{}'", path),
			other => format!("{:?}", other)
		}
	}

	pub(super) fn compile_eval(
		command: &str,
		arg_names: &[&str],
		scope: &dmasm::compiler::ExprScope
	) -> Result<(Vec<dmasm::Node>, Vec<String>, Vec<String>), CodeError> {
		let byond = crate::world_byond();
		if dmasm::compiler::is_body(command) {
			let body = dmasm::compiler::compile_body_in(command, arg_names, Some(scope), byond).map_err(CodeError::compiled)?;
			return Ok((body.nodes, body.locals, texts(&body.warnings)));
		}
		match dmasm::compiler::compile_expr_warned(command, arg_names, Some(scope), byond) {
			Ok((expr, warnings)) => Ok((expr, vec![], texts(&warnings))),
			Err(_) if command.trim().contains(['\n', ';']) => {
				let error = dmasm::compiler::compile_body_in(command, arg_names, Some(scope), byond).err();
				Err(error.map_or_else(
					|| CodeError {
						text: "invalid expression or proc body".to_owned(),
						unknown: None
					},
					CodeError::compiled
				))
			}
			Err(e) => Err(CodeError::compiled(e))
		}
	}

	pub(crate) fn call_with_locals(proc: &Proc, locals: &[String], args: &[&Value]) -> Result<Result<Value, Runtime>, String> {
		use auxtools::runtime_tables::{self, ProcEntryWords};
		let ends = crate::WORLD_ENDS.load(std::sync::atomic::Ordering::Relaxed);
		let none = match locals.is_empty() {
			true => None,
			false => {
				let none = unsafe { runtime_tables::proc_entry(proc.id()) }
					.ok_or("couldn't read the stub's proc entry")?
					.0[ProcEntryWords::LOCALS];
				let table = hot_reload::locals_table(locals, none)?;
				unsafe { runtime_tables::set_proc_entry_word(proc.id(), ProcEntryWords::LOCALS, table) };
				Some(none)
			}
		};
		let tries = Self::hide_tries();
		let result = proc.call(args);
		if crate::WORLD_ENDS.load(std::sync::atomic::Ordering::Relaxed) == ends {
			Self::show_tries(tries);
			if let Some(none) = none {
				unsafe { runtime_tables::set_proc_entry_word(proc.id(), ProcEntryWords::LOCALS, none) };
			}
		}
		Ok(result)
	}

	pub(crate) fn hide_tries() -> Vec<(*mut raw_types::procs::ExecutionContext, u32)> {
		let mut hidden = vec![];
		let mut at = unsafe { *raw_types::funcs::CURRENT_EXECUTION_CONTEXT };
		while !at.is_null() {
			let handler = unsafe { *(*at).throw_handler() };
			if handler != 0 {
				hidden.push((at, handler));
				unsafe { *(*at).throw_handler_mut() = 0 };
			}
			at = unsafe { *(*at).parent_context() };
		}
		hidden
	}

	pub(crate) fn show_tries(tries: Vec<(*mut raw_types::procs::ExecutionContext, u32)>) {
		for (ctx, handler) in tries {
			unsafe { *(*ctx).throw_handler_mut() = handler };
		}
	}

	pub(super) fn frame_is_live(ctx: *mut raw_types::procs::ExecutionContext) -> bool {
		let on = |mut at: *mut raw_types::procs::ExecutionContext| {
			while !at.is_null() {
				if at == ctx {
					return true;
				}
				at = unsafe { *(*at).parent_context() };
			}
			false
		};
		on(unsafe { *raw_types::funcs::CURRENT_EXECUTION_CONTEXT })
			|| debug::CallStacks::suspended_instances()
				.into_iter()
				.any(|instance| on(unsafe { (*instance).context }))
	}
}

impl Server {
	pub(super) fn send_callee_errors(&mut self, notes: EvalNotes) {
		let EvalNotes {
			warnings,
			callee_errors: errors,
			callee_more: more
		} = notes;
		let mut notes: Vec<String> = warnings.into_iter().map(|warning| format!("warning: {}", warning)).collect();
		notes.extend(
			errors
				.into_iter()
				.map(|error| format!("the evaluation continued after a runtime error in a called proc: {}", error))
		);
		if more > 0 {
			notes.push(format!("and {} more runtime errors in called procs", more));
		}
		let Some(client) = self.sessions.requester() else {
			return;
		};
		for message in notes {
			self.tell(client, Response::Notification { message });
		}
	}

	pub(super) fn handle_command(&mut self, frame_id: Option<u32>, command: &str) -> String {
		match console::parse(command) {
			Ok(console::Command::Disassemble { proc: Some(proc), id }) => self.codes.handle_disassemble(proc, id),
			Ok(console::Command::Disassemble { proc: None, .. }) => match frame_id {
				Some(frame_id) => match self.pause.get_stack_frame(frame_id) {
					Some(frame) => {
						let proc = unnamed::proc_ref(&frame.proc);
						self.codes.handle_disassemble(&proc.path, proc.override_id)
					}
					None => "couldn't find the stack frame. The world may not be paused".to_owned()
				},
				None => "no execution frame selected".to_owned()
			},
			#[cfg(feature = "test-commands")]
			Ok(console::Command::Skin) => match self.evaluating(crate::skin::state) {
				Ok(None) => "no skin".to_owned(),
				Ok(Some((resource, text))) => format!("resource {}\n{}", resource, text),
				Err(e) => format!("Failed: {}", e)
			},
			#[cfg(feature = "test-commands")]
			Ok(console::Command::Script) => self.evaluating(crate::client_script::state).unwrap_or_else(|e| format!("Failed: {}", e)),
			#[cfg(feature = "test-commands")]
			Ok(console::Command::FailReload(at)) => {
				hot_reload::fail_next(at);
				format!("the next reload fails at {:?}", at)
			}
			#[cfg(feature = "test-commands")]
			Ok(console::Command::StringReferences(text)) => match hot_reload::string_references(text) {
				Ok(count) => count.to_string(),
				Err(e) => format!("Failed: {}", e)
			},
			Ok(console::Command::Help(text)) => text.to_owned(),
			Err(error) => error
		}
	}

	pub(super) fn eval_expr(&mut self, request: EvalRequest) -> EvalOutcome {
		let mut outcome = self.eval_expr_plain(request);
		if let Some(name) = outcome.unknown.take() {
			outcome.value = outcome.value.map_err(|error| Evaluation::unknown_name(error, &name));
		}
		outcome
	}

	pub(super) fn eval_expr_plain(&mut self, request: EvalRequest) -> EvalOutcome {
		self.eval.in_eval = true;
		auxtools::instruction_hook::hear_all();
		self.eval.eval_error = None;
		self.eval.eval_callee_errors.clear();
		self.eval.eval_callee_more = 0;
		self.eval.eval_ran = std::ptr::null_mut();
		let mut outcome = EvalOutcome {
			value: Ok(Value::NULL),
			still_running: false,
			notes: EvalNotes::default(),
			unknown: None
		};

		let result = self.compile_and_run_expr(request, &mut outcome);
		self.refresh_watches();

		self.eval.in_eval = false;
		let EvalNotes {
			callee_errors, callee_more, ..
		} = self.eval.callee_notes();
		outcome.notes.callee_errors = callee_errors;
		outcome.notes.callee_more = callee_more;

		outcome.value = match self.eval.eval_error.take() {
			Some(error) => {
				outcome.unknown = None;
				Err(format!("runtime error: {}", error))
			}
			None => result
		};
		outcome
	}

	pub(super) fn compile_and_run_expr(&mut self, request: EvalRequest, outcome: &mut EvalOutcome) -> Result<Value, String> {
		enum ArgType {
			Dot,
			Usr,
			Src,
			Arg(u32),
			Local(u32),
			Given
		}

		let EvalRequest {
			frame: frame_id,
			command,
			src: given_src,
			given,
			mode
		} = request;

		let command = &*crate::macros::expand(command)?;

		let globals = unsafe { Value::new(ValueTag::GlobalVars, ValueData { id: 0 }) };
		let mut src = None;

		let (ctx, instance, mut args) = match frame_id {
			None => {
				let mut args = vec![];
				if let Some(object) = given_src {
					src = Some(object.clone());
					args.push(("src".to_owned(), object, ArgType::Given));
				}
				(std::ptr::null_mut(), std::ptr::null_mut(), args)
			}

			Some(frame_id) => {
				let Some(frame) = self.pause.get_stack_frame(frame_id) else {
					return Err(format!(
						"frame {} does not exist. The world is not paused, or it has resumed since",
						frame_id
					));
				};

				let frame_args = List::new();
				for (_, value) in &frame.args {
					frame_args.append(value);
				}
				if frame.src != Value::NULL {
					src = Some(frame.src.clone());
				}

				let mut args = vec![
					("args".to_owned(), Value::from(frame_args), ArgType::Given),
					(".".to_owned(), frame.dot.clone(), ArgType::Dot),
					("usr".to_owned(), frame.usr.clone(), ArgType::Usr),
					("src".to_owned(), frame.src.clone(), ArgType::Src),
				];

				for (idx, arg) in frame.args.iter().enumerate() {
					if let Some(name) = &arg.0 {
						args.push((name.into(), arg.1.clone(), ArgType::Arg(idx as u32)));
					}
				}

				for (idx, (local, name)) in frame.locals.iter().zip(values::visible_locals(&frame)).enumerate() {
					if let Some(name) = name {
						args.push((name, local.1.clone(), ArgType::Local(idx as u32)));
					}
				}

				(frame.context, frame.instance, args)
			}
		};

		args.push((GLOBALS.to_owned(), globals, ArgType::Given));
		for (name, value) in given.into_iter().rev() {
			args.insert(0, (name, value, ArgType::Given));
		}
		let arg_names: Vec<&str> = args.iter().map(|(name, ..)| name.as_str()).collect();
		let arg_values: Vec<&Value> = args.iter().map(|(_, value, _)| value).collect();

		let has_var = |object: &Value, name: &str| StringRef::new(name).is_ok_and(|name| object.get(name).is_ok());
		let scope = dmasm::compiler::ExprScope {
			has_src_var: &|name| src.as_ref().is_some_and(|src| has_var(src, name)),
			has_src_proc: &|name| src.as_ref().is_some_and(|src| crate::assemble_env::AssembleEnv::has_proc(src, name)),
			global: &|name| {
				if has_var(&Value::globals(), name) {
					Some(dmasm::compiler::ExprGlobal::Listed)
				} else {
					hot_reload::added_global(name).map(dmasm::compiler::ExprGlobal::Id)
				}
			},
			globals: GLOBALS,
			proc_ref: &crate::assemble_env::type_proc,
			proc_path: &crate::assemble_env::proc_path
		};
		let key = EvalCodeKey {
			generation: hot_reload::code_generation(),
			command: command.to_owned(),
			names: arg_names.iter().map(|name| (*name).to_owned()).collect(),
			src_type: src.as_ref().and_then(values::kind)
		};
		let compiled = match self.eval.eval_code.get(&key) {
			Some(compiled) => compiled.clone(),
			None => {
				let compiled = Evaluation::compile_eval(command, &arg_names, &scope).and_then(|(code, locals, warnings)| {
					let assembly = dmasm::assembler::assemble(&code, &mut crate::assemble_env::AssembleEnv).map_err(CodeError::assembled)?;
					Ok((assembly, locals, warnings))
				});
				if self.eval.eval_code.len() >= EVAL_CODE_KEPT {
					self.eval.eval_code.clear();
				}
				self.eval.eval_code.insert(key, compiled.clone());
				compiled
			}
		};
		let (assembly, locals, warnings) = compiled.map_err(|error| {
			outcome.unknown = error.unknown;
			error.text
		})?;
		outcome.notes.warnings = warnings;
		if let EvalMode::Queue(number) = mode {
			self.spawns.queued.push_back(Spawn {
				number,
				bytecode: assembly,
				locals,
				args: arg_values.into_iter().cloned().collect()
			});
			if let Some(inbox) = self.spawns.spawn_inbox.as_ref().and_then(|inbox| List::from_value(inbox).ok()) {
				inbox.append(Value::from(number as f32));
			}
			return Ok(Value::NULL);
		}
		let proc = Proc::find("/proc/auxtools_expr_stub").ok_or("the world has no /proc/auxtools_expr_stub to evaluate expressions in")?;

		unsafe { proc.set_bytecode(assembly) }?;

		let ends = crate::WORLD_ENDS.load(std::sync::atomic::Ordering::Relaxed);
		let result = Evaluation::call_with_locals(&proc, &locals, &arg_values)?;
		if crate::WORLD_ENDS.load(std::sync::atomic::Ordering::Relaxed) != ends {
			std::mem::forget(result);
			std::mem::forget(args);
			std::panic::resume_unwind(Box::new(crate::WorldEnded));
		}

		match result {
			Ok(res) => {
				if let Ok(list) = res.as_list() {
					if frame_id.is_some() && !Evaluation::frame_is_live(ctx) {
						return Err("evaluation ended the selected frame".to_owned());
					}

					let len = list.len();
					for i in 2..=len {
						let value = list.get(i).unwrap();
						let slot = &args[i as usize - 2].2;

						unsafe {
							match slot {
								ArgType::Dot => {
									let dot = (*ctx).dot_mut();
									let _ = Value::from_raw_owned(*dot);
									*dot = value.raw();
								}
								ArgType::Usr => {
									let _ = Value::from_raw_owned((*instance).usr);
									(*instance).usr = value.raw();
								}
								ArgType::Src => {
									let _ = Value::from_raw_owned((*instance).src);
									(*instance).src = value.raw();
								}
								ArgType::Arg(idx) => {
									let args = (*instance).args();
									let arg = args.add(*idx as usize);
									let _ = Value::from_raw_owned(*arg);
									(*arg) = value.raw();
								}
								ArgType::Local(idx) => {
									let locals = *(*ctx).locals();
									let local = locals.add(*idx as usize);
									let _ = Value::from_raw_owned(*local);
									(*local) = value.raw();
								}
								ArgType::Given => continue
							}
						}

						std::mem::forget(value);
					}

					Ok(list.get(1).unwrap())
				} else if !self.eval.eval_ran.is_null() && background_known() && unsafe { *(*self.eval.eval_ran).background() } != 0 {
					outcome.still_running = true;
					Err(
						"evaluation aborted: BYOND moved the code to the background, so its result is unavailable. world.loop_checks does this \
						 after about a million loop iterations. `set background = 1` does it too. The code is still running. Use `eval --spawn \
						 --wait` to wait for a result"
							.to_owned()
					)
				} else if !background_known() {
					outcome.still_running = true;
					Err(
						"evaluation aborted because the code slept or BYOND moved it to the background, so its result is unavailable. \
						 world.loop_checks moves code to the background after about a million loop iterations. `set background = 1` does it too. \
						 Use `eval --spawn --wait` to wait for a result"
							.to_owned()
					)
				} else {
					outcome.still_running = true;
					Err("evaluation aborted because the code slept".to_owned())
				}
			}

			Err(Runtime { message }) => Err(format!("runtime error: {}", message))
		}
	}

	pub(super) fn handle_eval_request(&mut self, frame_id: Option<u32>, command: &str, ending: bool) {
		if ending {
			self.pause.ending = Some(Instant::now());
		}
		let still_running = self.handle_eval(frame_id, command);
		if ending && !still_running {
			self.pause.ending = None;
		}
	}

	pub(super) fn handle_eval(&mut self, frame_id: Option<u32>, command: &str) -> bool {
		#[cfg(feature = "test-commands")]
		if command == "#panic" {
			panic!("asked for with #panic");
		}
		if let Some(command) = command.strip_prefix('#') {
			let response = self.handle_command(frame_id, command);
			self.answer(Response::Eval(Ok(EvalResponse {
				value: response,
				variables: None,
				kind: None,
				entries: None
			})));
			return false;
		}

		let reached = (!dmasm::compiler::is_body(command)).then(|| format!("({})", command));
		let EvalOutcome {
			value, still_running, notes, ..
		} = self.eval_expr(EvalRequest::new(frame_id, command));
		let response = value
			.map(|result| EvalResponse {
				value: values::stringify(&result),
				variables: self.pause.value_to_variables_ref(&result, reached),
				kind: values::kind(&result),
				entries: values::entries(&result)
			})
			.map_err(|message| EvalError { message, still_running });
		let still_running = matches!(&response, Err(error) if error.still_running);
		self.send_callee_errors(notes);
		self.answer(Response::Eval(response));
		still_running
	}

	pub(super) fn handle_eval_on(&mut self, object: &str, expression: &str, depth: Option<u32>) {
		let EvalOutcome {
			value, still_running, notes, ..
		} = match self.object_of(object) {
			Ok(object) => self.eval_expr(EvalRequest {
				src: Some(object),
				..EvalRequest::new(None, expression)
			}),
			Err(error) => EvalOutcome::failed(error)
		};
		let response = match depth {
			Some(depth) => Response::Render(value.map(|value| render::render(&value, depth))),
			None => Response::Eval(
				value
					.map(|result| EvalResponse {
						value: values::stringify(&result),
						variables: self.pause.value_to_variables_ref(&result, None),
						kind: values::kind(&result),
						entries: values::entries(&result)
					})
					.map_err(|message| EvalError { message, still_running })
			)
		};
		self.send_callee_errors(notes);
		self.answer(response);
	}

	pub(super) fn object_of(&mut self, reference: &str) -> Result<Value, String> {
		let text = reference.trim();
		let inner = text.strip_prefix('[').and_then(|text| text.strip_suffix(']')).unwrap_or(text);
		let id = inner
			.strip_prefix("0x")
			.or_else(|| inner.strip_prefix("0X"))
			.and_then(|hex| u32::from_str_radix(hex, 16).ok())
			.ok_or_else(|| format!("invalid object reference {}: expected a ref() value such as [0x2000001]", reference))?;
		let reference = format!("[0x{:x}]", id);
		let found = self.eval_expr(EvalRequest::new(None, &format!("locate(\"\\{}\")", reference))).value?;
		if found == Value::NULL {
			return Err(format!("object not found: {}", reference));
		}
		Ok(found)
	}

	pub(super) fn evaluating<T>(&mut self, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
		self.eval.in_eval = true;
		auxtools::instruction_hook::hear_all();
		self.eval.eval_error = None;
		let result = work();
		self.eval.in_eval = false;
		result
	}
}

fn background_known() -> bool {
	auxtools::version::get().0 >= 516
}

const MAX_CALLEE_ERRORS: usize = 5;

pub(crate) fn is_expression_frame(ctx: *mut raw_types::procs::ExecutionContext) -> bool {
	let id = unsafe { (**(*ctx).proc_instance()).proc };
	["/proc/auxtools_expr_stub", "/proc/auxtools_spawn_stub"]
		.into_iter()
		.any(|path| Proc::find(path).is_some_and(|stub| stub.id() == id))
}

fn texts(diagnostics: &[dmasm::diagnostic::Diagnostic]) -> Vec<String> {
	diagnostics.iter().map(ToString::to_string).collect()
}
