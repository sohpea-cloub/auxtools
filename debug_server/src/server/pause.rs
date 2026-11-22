use super::*;

pub(super) struct State {
	pub(super) active: OnceCell<Vec<debug::StackFrame>>,
	pub(super) suspended: OnceCell<Vec<Vec<debug::StackFrame>>>,
	pub(super) sleeping: usize,
	pub(super) variables: RefCell<Vec<Variables>>,
	pub(super) variables_to_refs: RefCell<HashMap<Variables, VariablesRef>>,
	pub(super) sleeping_frames: RefCell<Vec<SleepingFrame>>,
	pub(super) sleeping_frame_ids: RefCell<HashMap<SleepingFrame, u32>>
}

pub(super) const SLEEPING_FRAMES: u32 = 1 << 30;

const ENDING_LIMIT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(super) struct SleepingFrame {
	pub(super) thread: u32,
	pub(super) depth: usize,
	pub(super) instance: usize
}

impl State {
	pub(super) fn new() -> Self {
		Self {
			active: OnceCell::new(),
			suspended: OnceCell::new(),
			sleeping: DEFAULT_SLEEPING_STACKS as usize,
			variables: RefCell::new(vec![]),
			variables_to_refs: RefCell::new(HashMap::new()),
			sleeping_frames: RefCell::new(vec![]),
			sleeping_frame_ids: RefCell::new(HashMap::new())
		}
	}

	pub(super) fn sleeping_frame_id(&self, frame: SleepingFrame) -> u32 {
		let mut frames = self.sleeping_frames.borrow_mut();
		*self.sleeping_frame_ids.borrow_mut().entry(frame).or_insert_with(|| {
			frames.push(frame);
			SLEEPING_FRAMES + (frames.len() as u32 - 1)
		})
	}

	pub(super) fn sleeping_frame(&self, id: u32) -> Option<SleepingFrame> {
		self.sleeping_frames.borrow().get(id.checked_sub(SLEEPING_FRAMES)? as usize).copied()
	}

	pub(super) fn invalidate_stacks(&mut self) {
		self.active = OnceCell::new();
		self.suspended = OnceCell::new();
	}

	pub(super) fn active(&self) -> &Vec<debug::StackFrame> {
		self.active.get_or_init(debug::CallStacks::active)
	}

	pub(super) fn suspended(&self) -> &Vec<Vec<debug::StackFrame>> {
		self.suspended.get_or_init(|| debug::CallStacks::suspended_first(self.sleeping))
	}

	pub(super) fn read_sleeping(&mut self, sleeping: usize) {
		if sleeping > self.sleeping {
			self.sleeping = sleeping;
			self.suspended = OnceCell::new();
		}
	}

	pub(super) fn get_ref(&self, vars: Variables) -> VariablesRef {
		let mut variables_to_refs = self.variables_to_refs.borrow_mut();
		let mut variables = self.variables.borrow_mut();
		(*variables_to_refs.entry(vars.clone()).or_insert_with(|| {
			let reference = VariablesRef(variables.len() as i32 + 1);
			variables.push(vars);
			reference
		}))
		.clone()
	}

	pub(super) fn get_variables(&self, reference: VariablesRef) -> Option<Variables> {
		let variables = self.variables.borrow();
		let index = usize::try_from(reference.0).ok()?.checked_sub(1)?;
		variables.get(index).cloned()
	}
}

pub(super) struct Pausing {
	pub(super) state: Option<State>,
	pub(super) paused: Option<Stop>,
	pub(super) ending: Option<Instant>,
	pub(super) stepper: Option<ClientId>
}

impl Pausing {
	pub(super) fn new() -> Self {
		Self {
			state: None,
			paused: None,
			ending: None,
			stepper: None
		}
	}

	pub(super) fn shown(stack: &[debug::StackFrame]) -> &[debug::StackFrame] {
		match stack.iter().rposition(|frame| !crate::launched::runs_spawns(&frame.proc)) {
			Some(last) => &stack[..=last],
			None => stack
		}
	}

	pub(super) fn only_runs_spawns(instance: *mut raw_types::procs::ProcInstance) -> bool {
		let mut ctx = unsafe { (*instance).context };
		while !ctx.is_null() {
			let proc = Proc::from_id(unsafe { (**(*ctx).proc_instance()).proc });
			if !proc.is_some_and(|proc| crate::launched::runs_spawns(&proc)) {
				return false;
			}
			ctx = unsafe { *(*ctx).parent_context() };
		}
		true
	}

	pub(super) fn thread_of(&self, stack: &[debug::StackFrame]) -> u32 {
		Self::shown(stack)
			.last()
			.map_or(0, |frame| unsafe { (*frame.instance).argslist_idx.id }.wrapping_add(1))
	}

	pub(super) fn stack_position(&self, thread: u32) -> Option<usize> {
		let state = self.state.as_ref()?;
		if thread == 0 {
			return Some(0);
		}
		let is = |stack: &[debug::StackFrame]| self.thread_of(stack) == thread;
		if is(state.active()) {
			return Some(0);
		}
		state.suspended().iter().position(|stack| is(stack)).map(|at| at + 1)
	}

	pub(super) fn get_stack(&self, position: usize) -> Option<&Vec<debug::StackFrame>> {
		let state = self.state.as_ref()?;

		if position == 0 {
			return Some(state.active());
		}

		state.suspended().get(position - 1)
	}

	pub(super) fn get_stack_frame(&self, frame_index: u32) -> Option<&debug::StackFrame> {
		let state = self.state.as_ref()?;
		if frame_index < SLEEPING_FRAMES {
			return state.active().get(frame_index as usize);
		}

		let frame = state.sleeping_frame(frame_index)?;
		let position = self.stack_position(frame.thread).filter(|&position| position != 0)?;
		self.get_stack(position)?
			.get(frame.depth)
			.filter(|found| found.instance as usize == frame.instance)
	}

	pub(super) fn ending(&self) -> bool {
		self.ending.is_some_and(|since| since.elapsed() < ENDING_LIMIT)
	}
}

impl Server {
	pub(super) fn handle_stacks(&mut self, sleeping: Option<u32>) {
		let sleeping = sleeping.unwrap_or(DEFAULT_SLEEPING_STACKS) as usize;
		if let Some(state) = &mut self.pause.state {
			state.read_sleeping(sleeping.saturating_add(1));
		}
		let mut asleep = 0;
		let stacks = match &self.pause.state {
			Some(state) => {
				let instances = debug::CallStacks::suspended_instances();
				asleep = instances.len() - instances.into_iter().filter(|&instance| Pausing::only_runs_spawns(instance)).count();
				let suspended = state
					.suspended()
					.iter()
					.filter(|stack| !stack.iter().all(|frame| crate::launched::runs_spawns(&frame.proc)))
					.take(sleeping);
				let listed = std::iter::once(state.active()).chain(suspended);
				listed
					.enumerate()
					.filter(|(_, stack)| !stack.is_empty())
					.map(|(position, stack)| {
						let (file, line) = self.codes.frame_place(&stack[0], position != 0);
						Stack {
							id: self.pause.thread_of(stack),
							name: unnamed::proc_name(&stack[0].proc),
							file,
							line,
							sleeping: position != 0
						}
					})
					.collect()
			}

			None => vec![]
		};

		self.answer(Response::Stacks {
			stacks,
			sleeping: asleep as u32
		});
	}

	pub(super) fn handle_stack_frames(&mut self, thread: u32, start_frame: Option<u32>, count: Option<u32>) {
		let position = self.pause.stack_position(thread);
		let response = match position.and_then(|position| self.pause.get_stack(position)) {
			Some(stack) => {
				let stack_id = position.unwrap_or(0);
				let thread = self.pause.thread_of(stack);
				let stack = Pausing::shown(stack);
				let start_frame = start_frame.unwrap_or(0);
				let end_frame = start_frame.saturating_add(count.unwrap_or(stack.len() as u32));

				let start_frame = start_frame as usize;
				let end_frame = end_frame as usize;

				let mut frames = vec![];

				for i in start_frame..end_frame {
					if i >= stack.len() {
						break;
					}

					let id = match &self.pause.state {
						Some(state) if stack_id != 0 => state.sleeping_frame_id(SleepingFrame {
							thread,
							depth: i,
							instance: stack[i].instance as usize
						}),
						_ => i as u32
					};
					frames.push(self.codes.frame_info(&stack[i], id, stack_id != 0 && i == 0));
				}

				Response::StackFrames {
					frames,
					total_count: stack.len() as u32
				}
			}

			None => {
				if self.pause.state.is_none() {
					self.notify("received StackFrames request when not paused");
				}
				Response::StackFrames {
					frames: vec![],
					total_count: 0
				}
			}
		};

		self.answer(response);
	}

	pub fn handle_breakpoint(&mut self, _ctx: *mut raw_types::procs::ExecutionContext, reason: BreakpointReason) -> Option<ContinueKind> {
		if let BreakpointReason::Runtime { message, caught } = &reason {
			let stack = self.journal_stack();
			self.journal(
				JournalEvent::Runtime {
					message: message.clone(),
					caught: *caught,
					stack
				},
				None
			);
		}

		let wanted = match &reason {
			BreakpointReason::Runtime { message, caught } => self.sessions.clients.values().any(|client| {
				let filter = if *caught { &client.caught } else { &client.uncaught };
				filter.as_ref().is_some_and(|filter| filter.takes(message))
			}),
			_ => self.sessions.has_debuggers() || self.sessions.hold.is_some()
		};
		if !wanted || self.pause.ending() {
			return None;
		}

		self.pause.state = Some(State::new());

		let proc = unsafe { (*(*(*_ctx).proc_instance())).proc };
		let offset = unsafe { *(*_ctx).bytecode_offset() };
		match reason {
			BreakpointReason::Breakpoint => {
				if !self.breakpoint_hit((proc, offset)) {
					self.pause.state = None;
					return None;
				}
			}

			BreakpointReason::Step | BreakpointReason::Pause if crate::instruction_hooking::is_breakpoint(_ctx) => {
				self.breakpoint_hit((proc, offset));
			}

			_ => {}
		}

		let at = unsafe {
			let proc = Proc::from_id((*(*(*_ctx).proc_instance())).proc);
			InstructionRef {
				proc: proc.as_ref().map(unnamed::proc_ref).unwrap_or(ProcRef {
					path: String::new(),
					override_id: 0
				}),
				offset: u32::from(*(*_ctx).bytecode_offset())
			}
		};
		let stop = Stop {
			reason,
			thread: self.pause.thread_of(self.pause.state.as_ref().unwrap().active()),
			at
		};
		self.pause.paused = Some(stop.clone());
		self.broadcast(Response::BreakpointHit(stop), None);

		let mut unattended: Option<Instant> = None;
		loop {
			let mut wait = if self.sessions.hold.is_some() {
				Wait::Forever
			} else {
				Wait::WhileClients
			};
			if wait == Wait::WhileClients && !self.sessions.has_debuggers() {
				break;
			}
			if let (Wait::Forever, Some(max)) = (wait, self.sessions.hold_max) {
				if self.sessions.has_debuggers() {
					wait = Wait::Until(None);
				} else {
					wait = Wait::Until(Some(*unattended.get_or_insert_with(Instant::now) + max));
				}
			}
			let request = match self.recv_request(wait) {
				Some(request) => request,
				None => match wait {
					Wait::Until(Some(until)) if Instant::now() >= until => {
						self.let_hold_go();
						return Some(ContinueKind::Continue);
					}
					Wait::Until(_) => continue,
					_ => break
				}
			};

			if let Request::Continue { kind } = request {
				let position = |thread: u32| self.pause.stack_position(thread).unwrap_or(0) as u32;
				let kind = match kind {
					ContinueKind::Continue => ContinueKind::Continue,
					ContinueKind::StepOver { stack_id } => ContinueKind::StepOver {
						stack_id: position(stack_id)
					},
					ContinueKind::StepInto { stack_id } => ContinueKind::StepInto {
						stack_id: position(stack_id)
					},
					ContinueKind::StepOut { stack_id } => ContinueKind::StepOut {
						stack_id: position(stack_id)
					},
					ContinueKind::StepOverInstruction { stack_id } => ContinueKind::StepOverInstruction {
						stack_id: position(stack_id)
					},
					ContinueKind::StepIntoInstruction { stack_id } => ContinueKind::StepIntoInstruction {
						stack_id: position(stack_id)
					}
				};
				self.answer(Response::Ack);
				self.broadcast(Response::Resumed, self.sessions.requester());
				self.pause.state = None;
				self.pause.paused = None;
				self.sessions.hold = None;
				self.sessions.hold_max = None;
				self.pause.stepper = if kind == ContinueKind::Continue {
					None
				} else {
					self.sessions.requester()
				};
				self.sessions.request_over();
				return Some(kind);
			}

			if let Request::Eval { frame_id, command, ending } = request {
				self.handle_eval_request(frame_id, &command, ending);
				self.sessions.request_over();
				if self.pause.ending() {
					self.broadcast(Response::Resumed, None);
					self.pause.state = None;
					self.pause.paused = None;
					self.sessions.hold = None;
					self.sessions.hold_max = None;
					self.pause.stepper = None;
					return Some(ContinueKind::Continue);
				}
				self.pause.state.as_mut().unwrap().invalidate_stacks();
				continue;
			}
			if let Request::Render { frame_id, expression, depth } = request {
				self.handle_render(frame_id, &expression, depth);
				self.sessions.request_over();
				self.pause.state.as_mut().unwrap().invalidate_stacks();
				continue;
			}
			if let Request::EvalOn { object, expression, depth } = request {
				self.handle_eval_on(&object, &expression, depth);
				self.sessions.request_over();
				self.pause.state.as_mut().unwrap().invalidate_stacks();
				continue;
			}

			let runs_dm = matches!(
				request,
				Request::HotReload(_) | Request::Skin(_) | Request::Script(_) | Request::Reconnect | Request::Spawn { .. }
			);
			self.handle_request(request);
			self.sessions.request_over();
			if runs_dm {
				if let Some(state) = self.pause.state.as_mut() {
					state.invalidate_stacks();
				}
			}
		}

		self.broadcast(Response::Resumed, None);
		self.pause.state = None;
		self.pause.paused = None;
		Some(ContinueKind::Continue)
	}

	pub(super) fn let_hold_go(&mut self) {
		let seconds = self.sessions.hold_max.map_or(0.0, |max| max.as_secs_f64());
		eprintln!("Debug Server: resumed after {} seconds without a debugger", seconds);
		self.sessions.hold = None;
		self.sessions.hold_max = None;
		self.pause.state = None;
		self.pause.paused = None;
		self.journal(JournalEvent::HoldEnded { seconds }, None);
		self.broadcast(Response::Resumed, None);
	}

	pub fn step_ended(&mut self) {
		let Some(id) = self.pause.stepper.take() else {
			return;
		};
		if let Some(client) = self.sessions.clients.get_mut(&id) {
			if !Sessions::deliver(client, &Response::StepEnded) {
				self.remove_client(id);
			}
		}
	}

	pub fn world_carries_on(&mut self) {
		self.pause.ending = None;
	}
}
