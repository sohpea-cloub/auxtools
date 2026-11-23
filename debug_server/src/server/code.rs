use super::*;

const UNNUMBERED: u16 = u16::MAX;

pub(super) struct Codes {
	pub(super) old_codes: RefCell<Vec<usize>>
}

impl Codes {
	pub(super) fn new() -> Self {
		Self {
			old_codes: RefCell::new(vec![])
		}
	}

	pub(super) fn old_code_of(frame: &debug::StackFrame) -> Option<Arc<Code>> {
		Self::old_code_running(&frame.proc, frame.context)
	}

	pub(super) fn old_code_running(proc: &Proc, context: *mut raw_types::procs::ExecutionContext) -> Option<Arc<Code>> {
		code_index::running(proc, unsafe { *(*context).bytecode() })
	}

	pub(super) fn code_of(&self, frame: &debug::StackFrame, old: bool) -> CodeRef {
		let mut code = CodeRef {
			proc: frame.proc.id.0,
			old: 0
		};
		if old {
			code.old = self.old_code_number(unsafe { *(*frame.context).bytecode() } as usize);
		}
		code
	}

	pub(super) fn old_code_number(&self, running: usize) -> u16 {
		let mut old_codes = self.old_codes.borrow_mut();
		if let Some(at) = old_codes.iter().position(|&old| old == running) {
			return at as u16 + 1;
		}
		if old_codes.len() >= usize::from(UNNUMBERED - 1) {
			return UNNUMBERED;
		}
		old_codes.push(running);
		old_codes.len() as u16
	}

	pub(super) fn old_code(&self, path: &str, old: u16) -> Result<usize, String> {
		if old == UNNUMBERED {
			return Err(format!(
				"{} runs old code without an id: the debugger has given ids to {} old codes, the most it can",
				path,
				UNNUMBERED - 1
			));
		}
		let running = self.old_codes.borrow().get(usize::from(old).wrapping_sub(1)).copied();
		running.ok_or_else(|| format!("{} has no old code {}", path, old))
	}

	pub(super) fn disassemble(&self, code: CodeRef) -> Result<Disassembly, String> {
		let proc = Proc::from_id(raw_types::procs::ProcId(code.proc)).ok_or_else(|| format!("the world has no proc {:#x}", code.proc))?;
		let bytecode = if code.old == 0 {
			original_code(&proc)
		} else {
			let running = self.old_code(&proc.path, code.old)?;
			match auxtools::bytecode_at(running as *const u32) {
				Some(old) => Cow::Owned(crate::instruction_hooking::unpatched(running as *const u32, old)),
				None => return Err(format!("{} has no old code {}", proc.path, code.old))
			}
		};

		let mut env = disassemble_env::DisassembleEnv;
		let (nodes, error) = dmasm::disassembler::disassemble(&bytecode, &mut env);
		let first_line = nodes.iter().find_map(|node| match node {
			dmasm::Node::Instruction(dmasm::Instruction::DbgLine(line), _) => Some(*line),
			_ => None
		});
		let mut line = first_line;
		let mut instructions = vec![];
		let mut read = 0;
		for node in &nodes {
			let dmasm::Node::Instruction(ins, debug) = node else {
				continue;
			};
			if let dmasm::Instruction::DbgLine(at) = ins {
				line = Some(*at);
			}
			instructions.push(BytecodeInstruction {
				offset: debug.offset as u16,
				words: debug.bytecode.to_vec(),
				text: ins.to_string(),
				line
			});
			read = debug.offset as usize + debug.bytecode.len();
		}
		for (at, word) in bytecode.iter().enumerate().skip(read) {
			instructions.push(BytecodeInstruction {
				offset: at as u16,
				words: vec![*word],
				text: String::new(),
				line: None
			});
		}

		Ok(Disassembly {
			proc: unnamed::proc_ref(&proc),
			file: code_index::file_of(&proc),
			instructions,
			error: error.map(|error| format!("{:?}", error))
		})
	}

	pub(super) fn handle_disassemble(&mut self, path: &str, id: u32) -> String {
		match unnamed::find(path, id) {
			Some(proc) => {
				let bytecode = original_code(&proc);

				let mut env = crate::DisassembleEnv;
				let (nodes, error) = dmasm::disassembler::disassemble(&bytecode, &mut env);
				let dism = dmasm::format_disassembly(&nodes, None);

				match error {
					Some(error) => {
						format!("Dism for {:?}\n{}\n\tError: {:?}", proc, dism, error)
					}

					None => {
						format!("Dism for {:?}\n{}", proc, dism)
					}
				}
			}

			None => "Proc not found".to_owned()
		}
	}

	pub(super) fn proc_info(wanted: &ProcRef) -> Result<ProcInfo, String> {
		let definitions = (0..).take_while(|&n| unnamed::find(&wanted.path, n).is_some()).count() as u32;
		let proc = match unnamed::find(&wanted.path, wanted.override_id) {
			Some(proc) => proc,
			None if definitions == 0 => return Err(format!("the world has no proc {}", wanted.path)),
			None if definitions == 1 => return Err(format!("{} has one definition, #0. There is no #{}", wanted.path, wanted.override_id)),
			None => {
				return Err(format!(
					"{} has {} definitions, #0 to #{}. There is no #{}",
					wanted.path,
					definitions,
					definitions - 1,
					wanted.override_id
				))
			}
		};
		Ok(ProcInfo {
			proc: unnamed::proc_ref(&proc),
			code: CodeRef { proc: proc.id.0, old: 0 },
			file: code_index::file_of(&proc),
			line: code_index::of(&proc).lines().next(),
			definitions,
			reloaded: hot_reload::reloaded(proc.id.0)
		})
	}

	pub(super) fn frame_place(&self, frame: &debug::StackFrame, asleep: bool) -> (Option<String>, Option<u32>) {
		Self::place_running(&frame.proc, frame.context, frame.offset, asleep)
	}

	pub(super) fn place_running(
		proc: &Proc,
		context: *mut raw_types::procs::ExecutionContext,
		offset: u16,
		asleep: bool
	) -> (Option<String>, Option<u32>) {
		let offset = if asleep { (offset as u32).saturating_sub(1) } else { offset as u32 };

		if let Some(code) = Self::old_code_running(proc, context) {
			return match code.place(offset, false) {
				Some((file, line)) => (file.map(str::to_owned).or_else(|| code_index::file_of(proc)), Some(line)),
				None => (code_index::file_of(proc), None)
			};
		}
		Self::place_of(proc, offset)
	}

	pub(super) fn frame_info(&self, frame: &debug::StackFrame, id: u32, asleep: bool) -> StackFrame {
		let old_code = Self::old_code_of(frame).is_some();
		let (file, line) = self.frame_place(frame, asleep);
		StackFrame {
			id,
			instruction: InstructionRef {
				proc: unnamed::proc_ref(&frame.proc),
				offset: frame.offset as u32
			},
			line,
			file,
			old_code,
			code: self.code_of(frame, old_code)
		}
	}

	pub(super) fn normalized(path: &str) -> String {
		path.replace('\\', "/").to_ascii_lowercase()
	}

	pub(super) fn procs_in(file: &str) -> (Vec<String>, Vec<(ProcRef, Vec<u32>)>) {
		let normalize = Self::normalized;
		let wanted = normalize(file);
		let same = |file: &str| file == wanted;
		let alike = |file: &str| file.ends_with(&format!("/{}", wanted)) || wanted.ends_with(&format!("/{}", file));

		let index = code_index::by_file();
		let matching = |test: &dyn Fn(&str) -> bool| -> (Vec<String>, Vec<u32>) {
			let files: Vec<(&String, &Vec<u32>)> = index.iter().filter(|(file, _)| test(&normalize(file))).collect();
			let mut ids: Vec<u32> = files.iter().flat_map(|(_, ids)| ids.iter().copied()).collect();
			ids.sort_unstable();
			ids.dedup();
			(files.iter().map(|(file, _)| normalize(file)).collect(), ids)
		};
		let (mut files, mut ids) = matching(&same);
		if ids.is_empty() {
			(files, ids) = matching(&alike);
		}
		let procs = ids
			.into_iter()
			.filter_map(|id| Proc::from_id(raw_types::procs::ProcId(id)))
			.map(|proc| {
				let lines = code_index::of(&proc).lines_in(|file| files.contains(&Self::normalized(file)));
				let proc = unnamed::proc_ref(&proc);
				(proc, lines)
			})
			.collect();
		(files, procs)
	}

	pub(super) fn lines_with_code(file: &str, start: u32, end: u32) -> Vec<u32> {
		let mut lines: Vec<u32> = Self::procs_in(file)
			.1
			.into_iter()
			.flat_map(|(_, lines)| lines)
			.filter(|line| (start..=end).contains(line))
			.collect();
		lines.sort_unstable();
		lines.dedup();
		lines
	}

	pub(super) fn locate(&self, file: &str, line: u32) -> Vec<InstructionRef> {
		let (files, procs) = Self::procs_in(file);
		let Some(line) = procs.iter().flat_map(|(_, lines)| lines.iter().copied()).filter(|&l| l >= line).min() else {
			return vec![];
		};

		let mut found = vec![];
		for (proc, lines) in procs {
			if !lines.contains(&line) {
				continue;
			}
			if let Some(offset) = self.offset_of(proc.clone(), line, Some(&files)) {
				found.push(InstructionRef { proc, offset });
			}
		}
		found
	}

	pub(super) fn place_of(proc: &Proc, offset: u32) -> (Option<String>, Option<u32>) {
		let breakpoint_here = crate::instruction_hooking::hooked_at(proc, offset as usize);
		Self::place_at(proc, offset, breakpoint_here)
	}

	pub(super) fn breakpoint_place(proc: &Proc, offset: u32) -> (Option<String>, Option<u32>) {
		Self::place_at(proc, offset, true)
	}

	pub(super) fn place_at(proc: &Proc, offset: u32, before: bool) -> (Option<String>, Option<u32>) {
		let code = code_index::of(proc);
		match code.place(offset, before) {
			Some((file, line)) => (file.or_else(|| code.file()).map(str::to_owned), Some(line)),
			None => (code.file().map(str::to_owned), None)
		}
	}

	pub(super) fn get_offset(&self, proc: ProcRef, line: u32) -> Option<u32> {
		self.offset_of(proc, line, None)
	}

	pub(super) fn offset_of(&self, proc: ProcRef, line: u32, files: Option<&[String]>) -> Option<u32> {
		let proc = unnamed::find(&proc.path, proc.override_id)?;
		let wanted = |file: &str| files.is_some_and(|files| files.contains(&Self::normalized(file)));
		code_index::of(&proc).offset_of(line, files.map(|_| &wanted as &dyn Fn(&str) -> bool))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn old_code_past_the_last_id_is_refused() {
		let codes = Codes::new();
		codes.old_codes.borrow_mut().extend(1..usize::from(UNNUMBERED - 1));
		let (last, past, further) = (0x1000_0000, 0x2000_0000, 0x3000_0000);
		assert_eq!(codes.old_code_number(last), UNNUMBERED - 1);
		assert_eq!(codes.old_code_number(past), UNNUMBERED);
		assert_eq!(codes.old_code_number(further), UNNUMBERED);
		assert_eq!(codes.old_code_number(last), UNNUMBERED - 1);
		assert_eq!(codes.old_code("/f", UNNUMBERED - 1), Ok(last));
		assert_eq!(
			codes.old_code("/f", UNNUMBERED),
			Err("/f runs old code without an id: the debugger has given ids to 65534 old codes, the most it can".to_owned())
		);
		assert_eq!(codes.old_code("/f", 0), Err("/f has no old code 0".to_owned()));
	}
}
