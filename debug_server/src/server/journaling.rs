use super::*;

const JOURNAL_FRAMES: usize = 64;

impl Server {
	pub(super) fn journal(&mut self, event: JournalEvent, from: Option<ClientId>) {
		let entry = journal::add(event);
		let mut gone = vec![];
		for (&id, client) in &mut self.sessions.clients {
			let follows = client.follow.as_ref().is_some_and(|kinds| journal::wanted(&entry, kinds));
			if follows && Some(id) != from && !Sessions::deliver(client, &Response::Journaled(entry.clone())) {
				gone.push(id);
			}
		}
		for id in gone {
			self.remove_client(id);
		}
	}

	pub(super) fn journal_stack(&self) -> Vec<JournalFrame> {
		type Known = HashMap<(raw_types::procs::ProcId, u16, usize), JournalFrame>;
		static KNOWN: Mutex<(u64, Option<Known>)> = Mutex::new((0, None));
		const MAX_KNOWN_FRAMES: usize = 16384;

		let mut uncached: (u64, Option<Known>) = (0, None);
		let mut guard = match KNOWN.try_lock() {
			Ok(guard) => Some(guard),
			Err(std::sync::TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
			Err(std::sync::TryLockError::WouldBlock) => None
		};
		let (changes, known) = match guard.as_mut() {
			Some(guard) => &mut **guard,
			None => &mut uncached
		};
		let now = auxtools::bytecode_changes();
		if *changes != now || known.as_ref().is_some_and(|known| known.len() >= MAX_KNOWN_FRAMES) {
			*changes = now;
			*known = None;
		}
		let known = known.get_or_insert_with(HashMap::new);

		let mut frames: Vec<(JournalFrame, bool)> = vec![];
		let mut ctx = unsafe { *raw_types::funcs::CURRENT_EXECUTION_CONTEXT };
		while !ctx.is_null() {
			let (id, offset, running) = unsafe { ((**(*ctx).proc_instance()).proc, *(*ctx).bytecode_offset(), *(*ctx).bytecode()) };
			let Some(proc) = Proc::from_id(id) else {
				break;
			};
			let helper = crate::launched::runs_spawns(&proc);
			let (current, length) = unsafe { proc.bytecode_mut_ptr() };
			let breakpoint =
				(offset as u32) < length as u32 && crate::instruction_hooking::is_breakpoint_word(unsafe { *current.add(offset as usize) });
			let plain = std::ptr::eq(running, current) && !breakpoint;
			let key = (id, offset, running as usize);
			let frame = match known.get(&key).filter(|_| plain) {
				Some(frame) => frame.clone(),
				None => {
					let (file, line) = Codes::place_running(&proc, ctx, offset, false);
					let frame = JournalFrame {
						proc: unnamed::proc_name(&proc),
						file,
						line
					};
					if plain && !frame.proc.starts_with('(') {
						known.insert(key, frame.clone());
					}
					frame
				}
			};
			frames.push((frame, helper));
			ctx = unsafe { *(*ctx).parent_context() };
		}

		let shown = match frames.iter().rposition(|(_, helper)| !helper) {
			Some(last) => last + 1,
			None => frames.len()
		};
		frames.truncate(shown.min(JOURNAL_FRAMES));
		frames.into_iter().map(|(frame, _)| frame).collect()
	}

	pub(super) fn handle_journal(&mut self, after: u64, kinds: Vec<JournalKind>, limit: Option<u32>, follow: bool) {
		let (entries, first, last) = journal::read(after, &kinds, limit);
		if let Some(client) = self.sessions.requester().and_then(|id| self.sessions.clients.get_mut(&id)) {
			client.follow = follow.then_some(kinds);
		}
		self.answer(Response::Journal { entries, first, last });
	}
}
