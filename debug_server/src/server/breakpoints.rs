use super::*;

pub struct Claim {
	condition: Option<String>,
	hit: Option<HitCondition>,
	hits: u32,
	log: Option<Vec<LogPart>>,
	skipped: u32,
	told: bool
}

pub type Claims = HashMap<(raw_types::procs::ProcId, u16), BTreeMap<(ClientId, Placed), Claim>>;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Placed {
	Line,
	Proc,
	Instruction
}

impl Placed {
	fn of(offset: u32, exactly: bool) -> Self {
		match offset {
			_ if exactly => Self::Instruction,
			0 => Self::Proc,
			_ => Self::Line
		}
	}
}

pub(super) struct Breakpoints {
	pub(super) claims: Claims,
	pub(super) moved: hot_reload::MovedBreakpoints<(ClientId, Placed)>
}

impl Breakpoints {
	pub(super) fn new() -> Self {
		Self {
			claims: HashMap::new(),
			moved: HashMap::new()
		}
	}

	pub(super) fn breakpoint_offset(proc: &Proc, offset: u32) -> u32 {
		if offset == 0 {
			code_index::of(proc).entry_offset()
		} else {
			offset
		}
	}

	pub(super) fn expression_error(proc: &Proc, condition: &str) -> Option<String> {
		let mut names = vec![".".to_owned(), "usr".to_owned(), "src".to_owned()];
		names.extend(proc.parameter_names().iter().map(String::from));
		names.extend(proc.local_names().iter().map(String::from));
		let names: Vec<&str> = names.iter().map(String::as_str).collect();
		let condition = match crate::macros::expand(condition) {
			Ok(condition) => condition,
			Err(e) => return Some(e)
		};
		dmasm::compiler::compile_expr_for(&condition, &names, crate::world_byond())
			.err()
			.map(|e| e.to_string())
	}

	pub(super) fn forget_client(&mut self, id: ClientId) {
		let mut unclaimed = vec![];
		for (key, claims) in &mut self.claims {
			claims.retain(|(client, _), _| *client != id);
			if claims.is_empty() {
				unclaimed.push(*key);
			}
		}
		self.moved.retain(|(.., (client, _)), _| *client != id);
		for (proc, offset) in unclaimed {
			self.claims.remove(&(proc, offset));
			if let Some(proc) = Proc::from_id(proc) {
				let _ = unhook_instruction(&proc, offset as u32);
			}
		}
	}

	pub(super) fn clear(&mut self) {
		self.claims.clear();
		self.moved.clear();
		unhook_all();
	}
}

impl Server {
	pub(super) fn handle_breakpoint_set(
		&mut self,
		mut instruction: InstructionRef,
		condition: Option<String>,
		hit: Option<HitCondition>,
		log: Option<Vec<LogPart>>,
		exactly: bool
	) {
		let (Some(proc), Some(client)) = (unnamed::find(&instruction.proc.path, instruction.proc.override_id), self.sessions.current) else {
			self.answer(Response::BreakpointSet {
				result: BreakpointSetResult::Failed
			});
			return;
		};
		let placed = Placed::of(instruction.offset, exactly);
		if !exactly {
			instruction.offset = Breakpoints::breakpoint_offset(&proc, instruction.offset);
		}
		let (file, line) = Codes::breakpoint_place(&proc, instruction.offset);

		let log_error = || {
			log.iter().flatten().find_map(|part| match part {
				LogPart::Expr(expression) => {
					Breakpoints::expression_error(&proc, expression).map(|e| format!("the message's {{{}}} doesn't compile: {}", expression, e))
				}
				LogPart::Text(_) => None
			})
		};
		let condition_error = condition
			.as_deref()
			.and_then(|condition| Breakpoints::expression_error(&proc, condition))
			.map(|e| format!("the condition doesn't compile: {}", e));
		if let Some(reason) = condition_error.or_else(log_error) {
			self.answer(Response::BreakpointSet {
				result: BreakpointSetResult::Refused { reason }
			});
			return;
		}

		match hook_instruction(&proc, instruction.offset) {
			Ok(()) => {
				let key = (proc.id, instruction.offset as u16);
				let claims = self.breakpoints.claims.entry(key).or_default();
				let told = claims.get(&(client, placed)).is_some_and(|old| old.told && old.condition == condition);
				claims.insert((client, placed), Claim {
					condition,
					hit,
					hits: 0,
					log,
					skipped: 0,
					told
				});

				let offset = instruction.offset;
				self.breakpoints
					.moved
					.retain(|&(id, from, who), &mut at| id != proc.id || who != (client, placed) || (at != offset && from != offset));

				instruction.proc = unnamed::proc_ref(&proc);
				self.answer(Response::BreakpointSet {
					result: BreakpointSetResult::Success { line, at: instruction, file }
				});
			}

			Err(_) => {
				self.answer(Response::BreakpointSet {
					result: BreakpointSetResult::Failed
				});
			}
		}
	}

	pub(super) fn handle_breakpoint_unset(&mut self, instruction: InstructionRef, exactly: bool) {
		let (Some(proc), Some(client)) = (unnamed::find(&instruction.proc.path, instruction.proc.override_id), self.sessions.current) else {
			self.answer(Response::BreakpointUnset { success: false, skipped: 0 });
			return;
		};

		let placed = Placed::of(instruction.offset, exactly);
		let asked = if exactly {
			instruction.offset
		} else {
			Breakpoints::breakpoint_offset(&proc, instruction.offset)
		};
		let who = (client, placed);
		let offset = self.breakpoints.moved.get(&(proc.id, asked, who)).copied().unwrap_or(asked);

		self.breakpoints
			.moved
			.retain(|(id, _, other), at| *id != proc.id || *at != offset || *other != who);

		let key = (proc.id, offset as u16);
		let claims = self.breakpoints.claims.entry(key).or_default();
		let had = claims.remove(&who);
		let skipped = had.as_ref().map_or(0, |claim| claim.skipped);
		let success = if claims.is_empty() {
			self.breakpoints.claims.remove(&key);
			unhook_instruction(&proc, offset).is_ok()
		} else {
			had.is_some()
		};
		self.answer(Response::BreakpointUnset { success, skipped });
	}

	pub(super) fn log_message(&mut self, parts: &[LogPart]) -> String {
		let mut message = String::new();
		for part in parts {
			match part {
				LogPart::Text(text) => message.push_str(text),
				LogPart::Expr(expression) => {
					match self.eval_expr_plain(EvalRequest::new(Some(0), expression)).value {
						Ok(value) => message.push_str(&value.as_string().unwrap_or_else(|_| values::stringify(&value))),
						Err(error) => message.push_str(&format!("{{{}: {}}}", expression, error))
					}
					self.pause.state.as_mut().unwrap().invalidate_stacks();
				}
			}
		}
		message
	}

	pub(super) fn breakpoint_hit(&mut self, key: (raw_types::procs::ProcId, u16)) -> bool {
		let (proc, offset) = key;
		let clients: Vec<(ClientId, Placed)> = self
			.breakpoints
			.claims
			.get(&key)
			.map(|claims| claims.keys().copied().collect())
			.unwrap_or_default();

		let mut pause = clients.is_empty();
		for client in clients {
			let Some(condition) = self
				.breakpoints
				.claims
				.get(&key)
				.and_then(|claims| claims.get(&client))
				.map(|claim| claim.condition.clone())
			else {
				continue;
			};
			if let Some(condition) = condition {
				let passed = self
					.eval_expr(EvalRequest::new(Some(0), &condition))
					.value
					.map(|result| result.is_truthy());

				self.pause.state.as_mut().unwrap().invalidate_stacks();
				match passed {
					Ok(true) => {}
					Ok(false) => continue,
					Err(error) => {
						self.condition_failed(key, client, &condition, &error);
						continue;
					}
				}
			}

			let Some(claim) = self.breakpoints.claims.get_mut(&key).and_then(|claims| claims.get_mut(&client)) else {
				continue;
			};
			claim.hits += 1;
			if claim.hit.is_some_and(|hit| !hit.takes(claim.hits)) {
				continue;
			}

			let Some(log) = claim.log.clone() else {
				pause = true;
				continue;
			};
			let message = self.log_message(&log);
			let caller = self
				.pause
				.state
				.as_ref()
				.and_then(|state| state.active().get(1))
				.map(|frame| self.codes.frame_info(frame, 1, false));
			let logged = Proc::from_id(proc).map(|proc| {
				let place = unnamed::proc_ref(&proc);
				let (file, line) = Codes::place_of(&proc, offset as u32);
				Response::Log {
					message,
					file,
					line,
					proc: place,
					caller
				}
			});
			if let (Some(logged), Some(to)) = (logged, self.sessions.clients.get_mut(&client.0)) {
				if !Sessions::deliver(to, &logged) {
					self.remove_client(client.0);
				}
			}
		}
		pause
	}

	pub(super) fn condition_failed(&mut self, key: (raw_types::procs::ProcId, u16), client: (ClientId, Placed), condition: &str, error: &str) {
		let Some(claim) = self.breakpoints.claims.get_mut(&key).and_then(|claims| claims.get_mut(&client)) else {
			return;
		};
		claim.skipped += 1;
		if std::mem::replace(&mut claim.told, true) {
			return;
		}
		let place = match Proc::from_id(key.0) {
			Some(proc) => match Codes::place_of(&proc, key.1 as u32) {
				(Some(file), Some(line)) => format!(" at {}:{}", file, line),
				_ => format!(" in {}", unnamed::proc_name(&proc))
			},
			None => String::new()
		};
		let message = format!(
			"The condition ({}) of the breakpoint{} failed; skipping affected hits (reported once): {}",
			condition, place, error
		);
		self.tell(client.0, Response::ConditionFailed { message });
	}
}
