use super::*;

static NEXT_SPAWN: AtomicU64 = AtomicU64::new(1);

pub(super) struct Spawn {
	pub(super) number: u64,
	pub(super) bytecode: Vec<u32>,
	pub(super) locals: Vec<String>,
	pub(super) args: Vec<Value>
}

pub(super) struct Spawns {
	pub(super) queued: VecDeque<Spawn>,
	pub(super) spawn_inbox: Option<Value>,
	pub(super) spawn_stub_locals: Option<u32>,
	pub(super) spawned: HashMap<u64, (String, Option<String>)>
}

impl Spawns {
	pub(super) fn new() -> Self {
		Self {
			queued: VecDeque::new(),
			spawn_inbox: None,
			spawn_stub_locals: None,
			spawned: HashMap::new()
		}
	}

	pub(super) fn spawn_runtime(&mut self, message: &str, ctx: *mut raw_types::procs::ExecutionContext) {
		if self.spawned.is_empty() {
			return;
		}
		let (Some(stub), Some(spawned)) = (Proc::find("/proc/auxtools_spawn_stub"), Proc::find("/proc/auxtools_spawned")) else {
			return;
		};
		let proc_of = |at: *mut raw_types::procs::ExecutionContext| unsafe { (**(*at).proc_instance()).proc };
		let mut at = ctx;
		let mut in_stub = false;
		while !at.is_null() {
			let id = proc_of(at);
			if id == stub.id {
				if unsafe { *(*at).throw_handler() } != 0 {
					return;
				}
				if at != ctx {
					return;
				}
				in_stub = true;
			} else if in_stub && id == spawned.id {
				let number = unsafe { Value::from_raw(*(**(*at).proc_instance()).args()) }.as_number().ok();
				if let Some((_, error @ None)) = number.and_then(|number| self.spawned.get_mut(&(number as u64))) {
					*error = Some(format!("runtime error: {}", Evaluation::error_with_stack(message, ctx)));
				}
				return;
			} else if !in_stub && unsafe { *(*at).throw_handler() } != 0 {
				return;
			}
			at = unsafe { *(*at).parent_context() };
		}
	}

	pub(super) fn spawn_inbox(&mut self) -> Value {
		let inbox = Value::from(List::new());
		self.spawn_inbox = Some(inbox.clone());
		inbox
	}

	pub(super) fn set_stub_locals(&mut self, stub: &Proc, locals: &[String]) -> Result<(), String> {
		use auxtools::runtime_tables::{self, ProcEntryWords};
		let now = runtime_tables::proc_entry(stub.id).ok_or("couldn't read the spawn stub's proc entry")?.0[ProcEntryWords::LOCALS];
		let none = *self.spawn_stub_locals.get_or_insert(now);
		let table = hot_reload::locals_table(locals, none)?;
		if table != now {
			runtime_tables::set_proc_entry_word(stub.id, ProcEntryWords::LOCALS, table);
		}
		Ok(())
	}
}

impl Server {
	pub(super) fn handle_spawn(&mut self, expression: &str, object: Option<&str>) {
		let result = self.spawn(expression, object);
		self.answer(Response::Spawned(result));
	}

	pub(super) fn spawn(&mut self, expression: &str, object: Option<&str>) -> Result<u64, String> {
		crate::launched::can_spawn()?;
		let object = object.map(|object| self.object_of(object)).transpose()?;
		let number = NEXT_SPAWN.fetch_add(1, Ordering::Relaxed);
		let compiled = self.eval_expr(EvalRequest {
			src: object,
			mode: EvalMode::Queue(number),
			..EvalRequest::new(None, expression)
		});
		compiled.value?;
		self.spawns.spawned.insert(number, (expression.to_owned(), None));
		Ok(number)
	}

	pub fn next_spawn(&mut self) -> DMResult {
		let Some(spawn) = self.spawns.queued.pop_front() else {
			return Ok(Value::NULL);
		};
		let Some(stub) = Proc::find("/proc/auxtools_spawn_stub") else {
			self.spawn_failed(spawn.number, "the world has no /proc/auxtools_spawn_stub to run it in".to_owned());
			return Ok(Value::NULL);
		};
		if let Err(error) = self.spawns.set_stub_locals(&stub, &spawn.locals) {
			self.spawn_failed(spawn.number, error);
			return self.next_spawn();
		}
		stub.set_bytecode(spawn.bytecode);
		let args = List::new();
		for arg in &spawn.args {
			args.append(arg);
		}
		let next = List::new();
		next.append(Value::from(spawn.number as f32));
		next.append(Value::from(args));
		Ok(Value::from(next))
	}

	pub fn spawn_done(&mut self, number: u64, returned: &Value) {
		let Some((expression, error)) = self.spawns.spawned.remove(&number) else {
			return;
		};
		let result = match (error, returned.as_list()) {
			(Some(error), _) => Err(error),
			(None, Ok(list)) => {
				let value = list.get(1).unwrap_or(Value::NULL);
				Ok(SpawnedValue {
					text: values::stringify(&value),
					rendered: render::render(&value, 1)
				})
			}
			(None, Err(_)) => Err("evaluation ended without a value".to_owned())
		};
		self.journal(
			JournalEvent::Spawned {
				spawn: number,
				expression,
				result
			},
			None
		);
	}

	pub(super) fn spawn_failed(&mut self, number: u64, why: String) {
		let expression = self.spawns.spawned.remove(&number).map(|(expression, _)| expression).unwrap_or_default();
		self.journal(
			JournalEvent::Spawned {
				spawn: number,
				expression,
				result: Err(why)
			},
			None
		);
	}
}
