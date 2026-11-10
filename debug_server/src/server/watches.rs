//! Data breakpoints: the world pauses when what's at a place changes.
//!
//! Too many things write to a var to hook each (assignments, `+=` and its
//! kin, list procs, `vars[name]`, pointers, savefiles, the engine moving an
//! atom, byondapi), so nothing is hooked: the debug server runs between
//! every two instructions anyway, and looks at each watched place then. It
//! notices every write that leaves another value there, whoever made it, an
//! instruction late: the world stops just after the change, not on it.
//!
//! A place is named in one of two ways. An object's var, a list's entry and
//! a global are named by a pointer BYOND makes for `&place`, read back with
//! byondapi's own function. A proc's local or argument can't be: the pointer
//! would have to be made by code running in that proc. Those are read where
//! they are, in the one frame that was asked about, whenever that frame is
//! the one running: nothing else changes them, but for a pointer the proc
//! itself gave out, whose writes are seen when the proc next runs.

use auxtools::raw_types::values::Value as RawValue;

use super::*;

/// A place a client has asked to be told about (`Request::WatchSet`)
pub(super) struct Watched {
	id: u32,
	client: ClientId,
	/// What the client knows it as
	name: String,
	place: Place,
	/// What was there when it was last looked at
	last: Value,
	condition: Option<String>,
	hit: Option<HitCondition>,
	hits: u32
}

enum Place {
	/// An object's var, a list's entry, a global: BYOND's pointer to it
	Pointer(Value),
	/// A proc's own var, in one frame
	Slot {
		/// The frame's proc instance, by the id BYOND gives it (what its
		/// `args` list is known by). Its address won't do: BYOND moves an
		/// instance when its proc first sleeps. Nor would anything in it
		/// that a `spawn`ed copy takes along: the copy has its own vars,
		/// and its own id.
		instance: u32,
		/// The proc the frame is of: an id is another call's once the
		/// frame has ended
		proc: raw_types::procs::ProcId,
		slot: Slot
	}
}

/// A proc instance's id
unsafe fn instance_id(instance: *mut raw_types::procs::ProcInstance) -> u32 {
	unsafe { (*instance).argslist_idx.id }
}

#[derive(Clone, Copy)]
enum Slot {
	Local(usize),
	Argument(usize),
	Dot
}

/// The names a watch's condition has for what's at the place now, and what
/// was there before
const CONDITION_NEW: &str = "value";
const CONDITION_OLD: &str = "old";

fn same(a: RawValue, b: RawValue) -> bool {
	a.tag == b.tag && unsafe { a.data.id == b.data.id }
}

/// What's in a frame's slot, if it has one. `ctx` is the frame's own.
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

impl Server {
	pub fn has_watches(&self) -> bool {
		!self.watches.is_empty()
	}

	/// Looks at every watched place, as the world is about to run the
	/// instruction at `ctx`. Returns the changes the world is to pause for:
	/// those whose watch's condition holds, and whose turn it is by its hit
	/// count. Watches whose place is gone end here.
	pub fn watched_changes(&mut self, ctx: *mut raw_types::procs::ExecutionContext) -> Option<Vec<DataChange>> {
		let instance = unsafe { *(*ctx).proc_instance() };
		let mut changed: Vec<(usize, RawValue)> = vec![];
		let mut over: Vec<(u32, &'static str)> = vec![];

		// Reading through a pointer to a var of a deleted object is a runtime
		// error, which isn't one of the world's to pause for
		self.in_eval = true;
		for (index, watch) in self.watches.iter().enumerate() {
			match &watch.place {
				Place::Pointer(pointer) => match auxtools::pointer::read_raw(pointer) {
					Some(now) => {
						if !same(now, watch.last.raw) {
							changed.push((index, now));
						}
					}
					None => over.push((watch.id, "what it watched is gone"))
				},

				Place::Slot { instance: watched, proc, slot } => {
					if unsafe { instance_id(instance) } != *watched {
						continue;
					}
					unsafe {
						// The frame ended some way that wasn't seen (a runtime
						// error, its src deleted), and this is another call
						if (*instance).proc != *proc || *(*ctx).bytecode_offset() == 0 {
							over.push((watch.id, "its proc has ended"));
							continue;
						}
						match read_slot(ctx, *slot) {
							Some(now) => {
								if !same(now, watch.last.raw) {
									changed.push((index, now));
								}
							}
							None => {
								over.push((watch.id, "its proc has ended"));
								continue;
							}
						}
						// Its last instruction: nothing changes it after this
						let opcode = *(*(*ctx).bytecode()).add(*(*ctx).bytecode_offset() as usize);
						if opcode == dmasm::Opcode::Ret.word() || opcode == dmasm::Opcode::End.word() {
							over.push((watch.id, "its proc has returned"));
						}
					}
				}
			}
		}
		self.in_eval = false;
		self.eval_error = None;

		if changed.is_empty() && over.is_empty() {
			return None;
		}

		// What was there, and what is now
		let mut changes = vec![];
		for (index, now) in changed {
			let watch = &mut self.watches[index];
			let now = unsafe { Value::from_raw(now) };
			let before = std::mem::replace(&mut watch.last, now.clone());
			changes.push((watch.id, before, now));
		}
		for (id, reason) in over {
			self.end_watch(id, reason);
		}

		// A condition is worked out in the frame that's running, which
		// needs the stacks
		let paused = self.state.is_some();
		if !paused {
			self.state = Some(State::new());
		}
		let mut wanted = vec![];
		for (id, before, now) in changes {
			let Some(watch) = self.watches.iter().find(|watch| watch.id == id) else {
				continue;
			};
			let (name, condition) = (watch.name.clone(), watch.condition.clone());
			if let Some(condition) = condition {
				self.eval_given = vec![(CONDITION_NEW.to_owned(), now.clone()), (CONDITION_OLD.to_owned(), before.clone())];
				let passed = match self.eval_expr(Some(0), &condition) {
					Ok(result) => result.is_truthy(),
					Err(error) => {
						self.notify(format!("The condition of the watch on {} ({}) failed, so the world pauses: {}", name, condition, error));
						true
					}
				};
				if let Some(state) = self.state.as_mut() {
					state.invalidate_stacks();
				}
				if !passed {
					continue;
				}
			}
			// The condition's code could have taken the watch away
			let Some(watch) = self.watches.iter_mut().find(|watch| watch.id == id) else {
				continue;
			};
			watch.hits += 1;
			if watch.hit.is_some_and(|hit| !hit.takes(watch.hits)) {
				continue;
			}
			wanted.push(DataChange {
				id,
				name,
				old: Self::stringify(&before),
				new: Self::stringify(&now)
			});
		}
		if !paused {
			self.state = None;
		}
		(!wanted.is_empty()).then_some(wanted)
	}

	/// Takes what's at each watched place now as what it has: code the
	/// debugger ran itself (an expression, a value set) isn't the world's
	/// to be caught at
	pub(super) fn refresh_watches(&mut self) {
		if self.watches.is_empty() {
			return;
		}
		// The frames this pause has read, which are still there
		let frames: Vec<(u32, *mut raw_types::procs::ExecutionContext)> = match &self.state {
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
		let was_in_eval = std::mem::replace(&mut self.in_eval, true);
		for watch in &mut self.watches {
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
		self.in_eval = was_in_eval;
	}

	/// A watch is over: its client is told why
	fn end_watch(&mut self, id: u32, reason: &str) {
		let Some(at) = self.watches.iter().position(|watch| watch.id == id) else {
			return;
		};
		let watch = self.watches.remove(at);
		let ended = Response::WatchEnded {
			id,
			reason: reason.to_owned()
		};
		if let Some(to) = self.clients.get_mut(&watch.client) {
			if Self::send(&mut to.connection, &ended).is_err() {
				self.remove_client(watch.client);
			}
		}
	}

	pub(super) fn forget_watches_of(&mut self, client: ClientId) {
		self.watches.retain(|watch| watch.client != client);
	}

	/// A pointer to a place an expression names, made by the expression's
	/// own code: `&(expression)`
	fn pointer_to(&mut self, frame_id: Option<u32>, expression: &str, given: Vec<(String, Value)>) -> Result<Value, String> {
		if !auxtools::pointer::readable() {
			return Err("this BYOND build has no way to read a pointer".to_owned());
		}
		self.eval_given = given;
		let pointer = self.eval_expr(frame_id, &format!("&({})", expression));
		if let Some(state) = self.state.as_mut() {
			state.invalidate_stacks();
		}
		let pointer = pointer?;
		// A pointer to a var of the expression's own code names nothing
		// once that has run, which BYOND has no text for
		let names_something = pointer.raw.tag == ValueTag::Pointer && pointer.to_string().is_ok_and(|text| !text.is_empty());
		if !names_something || auxtools::pointer::read_raw(&pointer).is_none() {
			return Err("that isn't kept anywhere to watch: a var of an object, an entry of a list and a global var are".to_owned());
		}
		Ok(pointer)
	}

	/// A frame's own var by name: its slot, if it has one of that name
	fn slot_of(frame: &debug::StackFrame, name: &str) -> Result<Option<Slot>, String> {
		if name == "." {
			return Ok(Some(Slot::Dot));
		}
		let mut locals = frame.locals.iter().enumerate().filter(|(_, (local, _))| String::from(local) == name);
		match (locals.next(), locals.next()) {
			(Some((index, _)), None) => return Ok(Some(Slot::Local(index))),
			(Some(_), Some(_)) => return Err(format!("the proc has more than one local called {}", name)),
			_ => {}
		}
		let argument = frame.args.iter().position(|(argument, _)| argument.as_ref().is_some_and(|argument| String::from(argument) == name));
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

	/// Finds the place a client means, and what's there now
	fn watched_place(&mut self, place: &WatchPlace) -> Result<(Place, String, Value), String> {
		const ITS_VARS: &str = "it isn't watched itself: watch one of its vars";
		let pointed = |pointer: Value, name: String| -> Result<(Place, String, Value), String> {
			let now = auxtools::pointer::read(&pointer).ok_or("it can't be read")?;
			Ok((Place::Pointer(pointer), name, now))
		};

		match place {
			WatchPlace::Expression { frame_id, expression } => {
				let expression = expression.trim();
				// A name alone may be the frame's own var
				let plain = expression == "." || (!expression.is_empty() && expression.chars().all(|c| c.is_alphanumeric() || c == '_'));
				if let (true, Some(frame_id)) = (plain, frame_id) {
					if expression == "src" || expression == "usr" {
						return Err(ITS_VARS.to_owned());
					}
					let frame = self.get_stack_frame(*frame_id).ok_or("that frame is gone")?;
					if let Some(slot) = Self::slot_of(frame, expression)? {
						let (place, now) = Self::slot_place(frame, slot)?;
						return Ok((place, expression.to_owned(), now));
					}
				}
				let pointer = self.pointer_to(*frame_id, expression, vec![])?;
				pointed(pointer, expression.to_owned())
			}

			WatchPlace::Variable { vars, name } => {
				let container = self
					.state
					.as_ref()
					.and_then(|state| state.get_variables(vars.clone()))
					.ok_or("the world has carried on since that was listed")?;
				// What's inside something is reached through it, by what
				// it's called in these expressions
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
						let frame = self.get_stack_frame(frame).ok_or("that frame is gone")?;
						let slot = Self::slot_of(frame, name)?.ok_or_else(|| format!("the frame has no var called {}", name))?;
						let (place, now) = Self::slot_place(frame, slot)?;
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
						let number = Self::entry_number(name).ok_or_else(|| format!("the list has no entry {}", name))?;
						// An entry with a value for its key: the value is
						// what changes
						let runtime = |e: Runtime| format!("runtime error: {}", e.message);
						let entries = List::from_value(&list).map_err(runtime)?;
						let key = if entries.is_alist() {
							let (key, _) = entries.alist_pairs().map_err(runtime)?.into_iter().nth(number as usize - 1).ok_or("the list has changed")?;
							Some(key)
						} else {
							let key = entries.get(number).map_err(runtime)?;
							let has_value = key.raw.tag != ValueTag::Number && entries.get(&key).is_ok_and(|of_key| of_key.raw.tag != ValueTag::Null);
							has_value.then_some(key)
						};
						let pointer = match key {
							Some(key) => self.pointer_to(None, &format!("{}[{}]", HOLDER, KEY), vec![(HOLDER.to_owned(), list), (KEY.to_owned(), key)])?,
							None => self.pointer_to(None, &format!("{}[{}]", HOLDER, number), vec![(HOLDER.to_owned(), list)])?
						};
						pointed(pointer, within(&holder, name.clone()))
					}
					Variables::ListPair { key, of, list, .. } => {
						if name != "value" {
							return Err("an entry's key doesn't change".to_owned());
						}
						let shown = list.zip(Self::literal(&key)).map(|(list, key)| format!("{}[{}]", list, key));
						let pointer = self.pointer_to(None, &format!("{}[{}]", HOLDER, KEY), vec![(HOLDER.to_owned(), of), (KEY.to_owned(), key)])?;
						pointed(pointer, shown.unwrap_or_else(|| name.clone()))
					}
					// What a pointer names is the place itself
					Variables::Pointee(pointer, holder) => pointed(pointer, within(&holder, String::new()).trim_start_matches('*').to_owned())
				}
			}
		}
	}

	/// What a watch on a place would watch, in words for the client to show
	pub(super) fn watch_info(&mut self, place: &WatchPlace) -> Result<String, String> {
		self.watched_place(place).map(|(_, name, _)| name)
	}

	pub(super) fn watch_set(&mut self, place: &WatchPlace, condition: Option<String>, hit: Option<HitCondition>) -> Result<Watch, String> {
		let client = self.current.ok_or("nobody asked")?;
		let (place, name, now) = self.watched_place(place)?;
		self.next_watch += 1;
		let watch = Watch {
			id: self.next_watch,
			name: name.clone(),
			value: Self::stringify(&now)
		};
		self.watches.push(Watched {
			id: watch.id,
			client,
			name,
			place,
			last: now,
			condition,
			hit,
			hits: 0
		});
		Ok(watch)
	}

	pub(super) fn watch_unset(&mut self, id: u32) {
		let client = self.current;
		self.watches.retain(|watch| watch.id != id || Some(watch.client) != client);
	}
}
