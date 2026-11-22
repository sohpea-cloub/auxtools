use auxtools::raw_types::values::Value as RawValue;

use super::*;

pub(super) struct Watched {
	id: u32,
	client: ClientId,
	name: String,
	place: Place,
	last: Value,
	condition: Option<String>,
	hit: Option<HitCondition>,
	hits: u32,
	told: bool
}

enum Place {
	Pointer(Value),
	Slot {
		instance: u32,
		proc: raw_types::procs::ProcId,
		slot: Slot
	}
}

unsafe fn instance_id(instance: *mut raw_types::procs::ProcInstance) -> u32 {
	unsafe { (*instance).argslist_idx.id }
}

#[derive(Clone, Copy)]
enum Slot {
	Local(usize),
	Argument(usize),
	Dot
}

const CONDITION_NEW: &str = "value";
const CONDITION_OLD: &str = "old";

fn same(a: RawValue, b: RawValue) -> bool {
	a.tag == b.tag && unsafe { a.data.id == b.data.id }
}

unsafe fn read_slot(ctx: *mut raw_types::procs::ExecutionContext, slot: Slot) -> Option<RawValue> {
	unsafe {
		match slot {
			Slot::Local(index) => (index < *(*ctx).locals_count() as usize).then(|| *(*(*ctx).locals()).add(index)),
			Slot::Argument(index) => {
				let instance = *(*ctx).proc_instance();
				(index < (*instance).args_count() as usize).then(|| *(*instance).args().add(index))
			}
			Slot::Dot => Some(*(*ctx).dot())
		}
	}
}
pub(super) struct Watches {
	pub(super) watched: Vec<Watched>,
	pub(super) next_watch: u32
}

impl Watches {
	pub(super) fn new() -> Self {
		Self {
			watched: vec![],
			next_watch: 0
		}
	}

	pub(super) fn forget_watches_of(&mut self, client: ClientId) {
		self.watched.retain(|watch| watch.client != client);
	}

	fn slot_of(frame: &debug::StackFrame, name: &str) -> Result<Option<Slot>, String> {
		if name == "." {
			return Ok(Some(Slot::Dot));
		}
		let visible = values::visible_locals(frame);
		let mut locals = visible.iter().enumerate().filter(|(_, local)| local.as_deref() == Some(name));
		match (locals.next(), locals.next()) {
			(Some((index, _)), None) => return Ok(Some(Slot::Local(index))),
			(Some(_), Some(_)) => return Err(format!("the proc has more than one local called {}", name)),
			_ => {}
		}
		let argument = frame
			.args
			.iter()
			.position(|(argument, _)| argument.as_ref().is_some_and(|argument| String::from(argument) == name));
		Ok(argument.map(Slot::Argument))
	}

	fn slot_place(frame: &debug::StackFrame, slot: Slot) -> Result<(Place, Value), String> {
		let now = unsafe { read_slot(frame.context, slot) }.ok_or("the frame has no such var")?;
		let place = Place::Slot {
			instance: unsafe { instance_id(frame.instance) },
			proc: frame.proc.id,
			slot
		};
		Ok((place, unsafe { Value::from_raw(now) }))
	}
}

impl Server {
	pub fn has_watches(&self) -> bool {
		!self.watches.watched.is_empty()
	}

	pub fn watched_changes(&mut self, ctx: *mut raw_types::procs::ExecutionContext) -> Option<Vec<DataChange>> {
		let instance = unsafe { *(*ctx).proc_instance() };
		let mut changed: Vec<(usize, RawValue)> = vec![];
		let mut over: Vec<(u32, &'static str)> = vec![];

		self.eval.in_eval = true;
		auxtools::instruction_hook::hear_all();
		for (index, watch) in self.watches.watched.iter().enumerate() {
			match &watch.place {
				Place::Pointer(pointer) => match auxtools::pointer::read_raw(pointer) {
					Some(now) => {
						if !same(now, watch.last.raw) {
							changed.push((index, now));
						}
					}
					None => over.push((watch.id, "watch target no longer exists"))
				},

				Place::Slot {
					instance: watched,
					proc,
					slot
				} => {
					if unsafe { instance_id(instance) } != *watched {
						continue;
					}
					unsafe {
						if (*instance).proc != *proc || *(*ctx).bytecode_offset() == 0 {
							over.push((watch.id, "watched proc ended"));
							continue;
						}
						match read_slot(ctx, *slot) {
							Some(now) => {
								if !same(now, watch.last.raw) {
									changed.push((index, now));
								}
							}
							None => {
								over.push((watch.id, "watched proc ended"));
								continue;
							}
						}
						let opcode = *(*(*ctx).bytecode()).add(*(*ctx).bytecode_offset() as usize);
						if opcode == dmasm::Opcode::Ret.word() || opcode == dmasm::Opcode::End.word() {
							over.push((watch.id, "watched proc returned"));
						}
					}
				}
			}
		}
		self.eval.in_eval = false;
		self.eval.eval_error = None;

		if changed.is_empty() && over.is_empty() {
			return None;
		}

		let mut changes = vec![];
		for (index, now) in changed {
			let watch = &mut self.watches.watched[index];
			let now = unsafe { Value::from_raw(now) };
			let before = std::mem::replace(&mut watch.last, now.clone());
			changes.push((watch.id, before, now));
		}
		for (id, reason) in over {
			self.end_watch(id, reason);
		}

		let paused = self.pause.state.is_some();
		if !paused {
			self.pause.state = Some(State::new());
		}
		let mut wanted = vec![];
		for (id, before, now) in changes {
			let Some(watch) = self.watches.watched.iter().find(|watch| watch.id == id) else {
				continue;
			};
			let (name, condition) = (watch.name.clone(), watch.condition.clone());
			if let Some(condition) = condition {
				let passed = self.eval_expr(EvalRequest {
					given: vec![(CONDITION_NEW.to_owned(), now.clone()), (CONDITION_OLD.to_owned(), before.clone())],
					..EvalRequest::new(Some(0), &condition)
				});
				let passed = passed.value.map(|result| result.is_truthy());
				if let Some(state) = self.pause.state.as_mut() {
					state.invalidate_stacks();
				}
				match passed {
					Ok(true) => {}
					Ok(false) => continue,
					Err(error) => {
						let Some(watch) = self.watches.watched.iter_mut().find(|watch| watch.id == id) else {
							continue;
						};
						if !std::mem::replace(&mut watch.told, true) {
							let client = watch.client;
							let message = format!(
								"The condition ({}) of the watch on {} failed; skipping affected changes (reported once): {}",
								condition, name, error
							);
							self.tell(client, Response::ConditionFailed { message });
						}
						continue;
					}
				}
			}
			let Some(watch) = self.watches.watched.iter_mut().find(|watch| watch.id == id) else {
				continue;
			};
			watch.hits += 1;
			if watch.hit.is_some_and(|hit| !hit.takes(watch.hits)) {
				continue;
			}
			wanted.push(DataChange {
				id,
				name,
				old: values::stringify(&before),
				new: values::stringify(&now)
			});
		}
		if !paused {
			self.pause.state = None;
		}
		(!wanted.is_empty()).then_some(wanted)
	}

	pub(super) fn refresh_watches(&mut self) {
		if self.watches.watched.is_empty() {
			return;
		}
		let frames: Vec<(u32, *mut raw_types::procs::ExecutionContext)> = match &self.pause.state {
			Some(state) => {
				let asleep = state.suspended.get().map(|stacks| stacks.iter().flatten());
				state
					.active()
					.iter()
					.chain(asleep.into_iter().flatten())
					.map(|frame| (unsafe { instance_id(frame.instance) }, frame.context))
					.collect()
			}
			None => vec![]
		};
		let was_in_eval = std::mem::replace(&mut self.eval.in_eval, true);
		auxtools::instruction_hook::hear_all();
		for watch in &mut self.watches.watched {
			let now = match &watch.place {
				Place::Pointer(pointer) => auxtools::pointer::read_raw(pointer),
				Place::Slot { instance, slot, .. } => frames
					.iter()
					.find(|(frame, _)| frame == instance)
					.and_then(|(_, ctx)| unsafe { read_slot(*ctx, *slot) })
			};
			if let Some(now) = now {
				watch.last = unsafe { Value::from_raw(now) };
			}
		}
		self.eval.in_eval = was_in_eval;
	}

	fn end_watch(&mut self, id: u32, reason: &str) {
		let Some(at) = self.watches.watched.iter().position(|watch| watch.id == id) else {
			return;
		};
		let watch = self.watches.watched.remove(at);
		let ended = Response::WatchEnded {
			id,
			reason: reason.to_owned()
		};
		if let Some(to) = self.sessions.clients.get_mut(&watch.client) {
			if !Sessions::deliver(to, &ended) {
				self.remove_client(watch.client);
			}
		}
	}

	fn pointer_to(&mut self, frame_id: Option<u32>, expression: &str, given: Vec<(String, Value)>) -> Result<Value, String> {
		if !auxtools::pointer::readable() {
			return Err("pointer reads are unavailable in this BYOND build".to_owned());
		}
		let expression = format!("&({})", expression);
		let pointer = self.eval_expr(EvalRequest {
			given,
			..EvalRequest::new(frame_id, &expression)
		});
		let pointer = pointer.value;
		if let Some(state) = self.pause.state.as_mut() {
			state.invalidate_stacks();
		}
		let pointer = pointer?;
		let names_something = pointer.raw.tag == ValueTag::Pointer && pointer.to_string().is_ok_and(|text| !text.is_empty());
		if !names_something || auxtools::pointer::read_raw(&pointer).is_none() {
			return Err("expression has no persistent watch target; select an object variable, list entry or global".to_owned());
		}
		Ok(pointer)
	}

	fn watched_place(&mut self, place: &WatchPlace) -> Result<(Place, String, Value), String> {
		const ITS_VARS: &str = "select a variable of this value to watch";
		let pointed = |pointer: Value, name: String| -> Result<(Place, String, Value), String> {
			let now = auxtools::pointer::read(&pointer).ok_or("it can't be read")?;
			Ok((Place::Pointer(pointer), name, now))
		};

		match place {
			WatchPlace::Expression { frame_id, expression } => {
				let expression = expression.trim();
				let plain = expression == "." || (!expression.is_empty() && expression.chars().all(|c| c.is_alphanumeric() || c == '_'));
				if let (true, Some(frame_id)) = (plain, frame_id) {
					if expression == "src" || expression == "usr" {
						return Err(ITS_VARS.to_owned());
					}
					let frame = self.pause.get_stack_frame(*frame_id).ok_or("frame no longer exists")?;
					if let Some(slot) = Watches::slot_of(frame, expression)? {
						let (place, now) = Watches::slot_place(frame, slot)?;
						return Ok((place, expression.to_owned(), now));
					}
				}
				let pointer = self.pointer_to(*frame_id, expression, vec![])?;
				pointed(pointer, expression.to_owned())
			}

			WatchPlace::Variable { vars, name } => {
				let container = self
					.pause
					.state
					.as_ref()
					.and_then(|state| state.get_variables(vars.clone()))
					.ok_or("variable reference expired after execution resumed")?;
				const HOLDER: &str = "__watched";
				const KEY: &str = "__key";
				let within = |holder: &Option<String>, rest: String| match holder {
					Some(holder) => format!("{}{}", holder, rest),
					None => name.clone()
				};
				match container {
					Variables::Locals { frame } | Variables::Arguments { frame } => {
						if name == "src" || name == "usr" {
							return Err(ITS_VARS.to_owned());
						}
						let frame = self.pause.get_stack_frame(frame).ok_or("frame no longer exists")?;
						let slot = Watches::slot_of(frame, name)?.ok_or_else(|| format!("the frame has no var called {}", name))?;
						let (place, now) = Watches::slot_place(frame, slot)?;
						Ok((place, name.clone(), now))
					}
					Variables::Globals => {
						let pointer = self.pointer_to(None, &format!("global.{}", name), vec![])?;
						pointed(pointer, name.clone())
					}
					Variables::Builtins => Err("BYOND's constants don't change".to_owned()),
					Variables::ObjectVars(object, holder) => {
						let pointer = self.pointer_to(None, &format!("{}.{}", HOLDER, name), vec![(HOLDER.to_owned(), object)])?;
						pointed(pointer, within(&holder, format!(".{}", name)))
					}
					Variables::ListContents(list, holder) => {
						let number = values::entry_number(name).ok_or_else(|| format!("the list has no entry {}", name))?;
						let runtime = |e: Runtime| format!("runtime error: {}", e.message);
						let entries = List::from_value(&list).map_err(runtime)?;
						let key = if entries.is_alist() {
							let (key, _) = entries
								.alist_pairs()
								.map_err(runtime)?
								.into_iter()
								.nth(number as usize - 1)
								.ok_or("the list has changed")?;
							Some(key)
						} else {
							let key = entries.get(number).map_err(runtime)?;
							let has_value = key.raw.tag != ValueTag::Number && entries.get(&key).is_ok_and(|of_key| of_key.raw.tag != ValueTag::Null);
							has_value.then_some(key)
						};
						let pointer = match key {
							Some(key) => self.pointer_to(None, &format!("{}[{}]", HOLDER, KEY), vec![
								(HOLDER.to_owned(), list),
								(KEY.to_owned(), key),
							])?,
							None => self.pointer_to(None, &format!("{}[{}]", HOLDER, number), vec![(HOLDER.to_owned(), list)])?
						};
						pointed(pointer, within(&holder, name.clone()))
					}
					Variables::ListPair { key, of, entry, .. } => {
						if name != "value" {
							return Err("list keys cannot be watched for value changes".to_owned());
						}
						let pointer = self.pointer_to(None, &format!("{}[{}]", HOLDER, KEY), vec![
							(HOLDER.to_owned(), of),
							(KEY.to_owned(), key),
						])?;
						pointed(pointer, entry.unwrap_or_else(|| name.clone()))
					}
					Variables::Pointee(pointer, holder) => pointed(pointer, within(&holder, String::new()).trim_start_matches('*').to_owned())
				}
			}
		}
	}

	pub(super) fn watch_info(&mut self, place: &WatchPlace) -> Result<String, String> {
		self.watched_place(place).map(|(_, name, _)| name)
	}

	pub(super) fn watch_set(&mut self, place: &WatchPlace, condition: Option<String>, hit: Option<HitCondition>) -> Result<Watch, String> {
		let client = self.sessions.requester().ok_or("nobody asked")?;
		let (place, name, now) = self.watched_place(place)?;
		self.watches.next_watch += 1;
		let watch = Watch {
			id: self.watches.next_watch,
			name: name.clone(),
			value: values::stringify(&now)
		};
		self.watches.watched.push(Watched {
			id: watch.id,
			client,
			name,
			place,
			last: now,
			condition,
			hit,
			hits: 0,
			told: false
		});
		Ok(watch)
	}

	pub(super) fn watch_unset(&mut self, id: u32) {
		let client = self.sessions.requester();
		self.watches.watched.retain(|watch| watch.id != id || Some(watch.client) != client);
	}
}
