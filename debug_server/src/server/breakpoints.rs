use super::*;

pub struct Claim {
	condition: Option<String>,
	hit: Option<HitCondition>,
	hits: u32,
	log: Option<Vec<LogPart>>,
	skipped: u32,
	told: bool
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Claimant {
	pub client: ClientId,
	pub id: u32,
	pub on_entry: bool
}

pub type Claims = HashMap<(raw_types::procs::ProcId, u16), BTreeMap<Claimant, Claim>>;

impl hot_reload::Claimant for Claimant {
	fn on_entry(&self) -> bool {
		self.on_entry
	}
}

pub(super) struct Breakpoints {
	pub(super) claims: Claims
}

impl Breakpoints {
	pub(super) fn new() -> Self {
		Self { claims: HashMap::new() }
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

	fn trigger_error(proc: &Proc, condition: Option<&str>, log: Option<&[LogPart]>) -> Option<String> {
		let condition_error = condition
			.and_then(|condition| Self::expression_error(proc, condition))
			.map(|e| format!("the condition doesn't compile: {}", e));
		condition_error.or_else(|| {
			log.into_iter().flatten().find_map(|part| match part {
				LogPart::Expr(expression) => {
					Self::expression_error(proc, expression).map(|e| format!("the message's {{{}}} doesn't compile: {}", expression, e))
				}
				LogPart::Text(_) => None
			})
		})
	}

	fn find(&self, client: ClientId, id: u32) -> Option<((raw_types::procs::ProcId, u16), Claimant)> {
		self.claims
			.iter()
			.find_map(|(&at, claims)| claims.keys().find(|who| who.client == client && who.id == id).map(|&who| (at, who)))
	}

	fn remove(&mut self, (key, who): ((raw_types::procs::ProcId, u16), Claimant)) -> (bool, u32) {
		let claims = self.claims.entry(key).or_default();
		let skipped = claims.remove(&who).map_or(0, |claim| claim.skipped);
		if !claims.is_empty() {
			return (true, skipped);
		}
		self.claims.remove(&key);
		let unhooked = Proc::from_id(key.0).is_some_and(|proc| unhook_instruction(&proc, key.1.into()).is_ok());
		(unhooked, skipped)
	}

	pub(super) fn forget_client(&mut self, id: ClientId) {
		let mut unclaimed = vec![];
		for (key, claims) in &mut self.claims {
			claims.retain(|who, _| who.client != id);
			if claims.is_empty() {
				unclaimed.push(*key);
			}
		}
		for (proc, offset) in unclaimed {
			self.claims.remove(&(proc, offset));
			if let Some(proc) = Proc::from_id(proc) {
				let _ = unhook_instruction(&proc, offset as u32);
			}
		}
	}

	pub(super) fn clear(&mut self) {
		self.claims.clear();
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
		exact: bool,
		id: u32
	) {
		let (Some(proc), Some(client)) = (
			unnamed::find(&instruction.proc.path, instruction.proc.override_id),
			self.sessions.requester()
		) else {
			self.answer(Response::BreakpointSet {
				result: BreakpointSetResult::Failed
			});
			return;
		};
		let on_entry = !exact && instruction.offset == 0;
		if !exact {
			instruction.offset = Breakpoints::breakpoint_offset(&proc, instruction.offset);
		}

		if let Some(reason) = Breakpoints::trigger_error(&proc, condition.as_deref(), log.as_deref()) {
			self.answer(Response::BreakpointSet {
				result: BreakpointSetResult::Refused { reason }
			});
			return;
		}
		if let Some(old) = self.breakpoints.find(client, id) {
			self.breakpoints.remove(old);
		}
		if hook_instruction(&proc, instruction.offset).is_err() {
			self.answer(Response::BreakpointSet {
				result: BreakpointSetResult::Failed
			});
			return;
		}
		let key = (proc.id, instruction.offset as u16);
		self.breakpoints
			.claims
			.entry(key)
			.or_default()
			.insert(Claimant { client, id, on_entry }, Claim {
				condition,
				hit,
				hits: 0,
				log,
				skipped: 0,
				told: false
			});
		self.answer(Response::BreakpointSet {
			result: Self::placed(&proc, key.1)
		});
	}

	fn placed(proc: &Proc, offset: u16) -> BreakpointSetResult {
		let (file, line) = Codes::breakpoint_place(proc, offset.into());
		BreakpointSetResult::Success {
			line,
			at: InstructionRef {
				proc: unnamed::proc_ref(proc),
				offset: offset.into()
			},
			file
		}
	}

	pub(super) fn handle_breakpoint_trigger(&mut self, id: u32, condition: Option<String>, hit: Option<HitCondition>, log: Option<Vec<LogPart>>) {
		let found = self.sessions.requester().and_then(|client| self.breakpoints.find(client, id));
		let Some(((proc_id, offset), who)) = found else {
			self.answer(Response::BreakpointSet {
				result: BreakpointSetResult::Failed
			});
			return;
		};
		let Some(proc) = Proc::from_id(proc_id) else {
			self.answer(Response::BreakpointSet {
				result: BreakpointSetResult::Failed
			});
			return;
		};
		if let Some(reason) = Breakpoints::trigger_error(&proc, condition.as_deref(), log.as_deref()) {
			self.answer(Response::BreakpointSet {
				result: BreakpointSetResult::Refused { reason }
			});
			return;
		}
		if let Some(claim) = self
			.breakpoints
			.claims
			.get_mut(&(proc_id, offset))
			.and_then(|claims| claims.get_mut(&who))
		{
			if claim.condition != condition || claim.hit != hit {
				claim.hits = 0;
			}
			claim.told &= claim.condition == condition;
			claim.condition = condition;
			claim.hit = hit;
			claim.log = log;
		}
		self.answer(Response::BreakpointSet {
			result: Self::placed(&proc, offset)
		});
	}

	pub(super) fn handle_breakpoint_unset(&mut self, id: u32) {
		let found = self.sessions.requester().and_then(|client| self.breakpoints.find(client, id));
		let Some((key, who)) = found else {
			self.answer(Response::BreakpointUnset { success: false, skipped: 0 });
			return;
		};
		let (success, skipped) = self.breakpoints.remove((key, who));
		self.answer(Response::BreakpointUnset { success, skipped });
	}

	pub(super) fn handle_breakpoints(&mut self) {
		let client = self.sessions.requester();
		let mut placed = vec![];
		for (&(proc, offset), claims) in &self.breakpoints.claims {
			let Some(proc) = Proc::from_id(proc) else {
				continue;
			};
			for who in claims.keys().filter(|who| Some(who.client) == client) {
				let (file, line) = Codes::breakpoint_place(&proc, offset.into());
				placed.push(PlacedBreakpoint {
					id: who.id,
					at: InstructionRef {
						proc: unnamed::proc_ref(&proc),
						offset: offset.into()
					},
					file,
					line
				});
			}
		}
		placed.sort_by_key(|placed| placed.id);
		self.answer(Response::Breakpoints(placed));
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

	pub(super) fn breakpoint_hit(&mut self, key: (raw_types::procs::ProcId, u16)) -> Option<Vec<Claimant>> {
		let (proc, offset) = key;
		let clients: Vec<Claimant> = self
			.breakpoints
			.claims
			.get(&key)
			.map(|claims| claims.keys().copied().collect())
			.unwrap_or_default();

		let unclaimed = clients.is_empty();
		let mut paused_by = vec![];
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
				paused_by.push(client);
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
			if let (Some(logged), Some(to)) = (logged, self.sessions.clients.get_mut(&client.client)) {
				if !Sessions::deliver(to, &logged) {
					self.remove_client(client.client);
				}
			}
		}
		(unclaimed || !paused_by.is_empty()).then_some(paused_by)
	}

	pub(super) fn condition_failed(&mut self, key: (raw_types::procs::ProcId, u16), client: Claimant, condition: &str, error: &str) {
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
			"Breakpoint{}: the condition `{}` failed: {}. Hits where it fails are skipped. This message is shown once.",
			place, condition, error
		);
		self.tell(client.client, Response::ConditionFailed { message });
	}
}
