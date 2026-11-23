use std::{
	collections::{BTreeSet, HashMap},
	sync::{Arc, LazyLock, Mutex, MutexGuard, TryLockError}
};

use auxtools::{raw_types::procs::ProcId, *};
use dmasm::disassembler::DisassembleEnv;

use crate::instruction_hooking::{is_breakpoint_word, original_code};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Mark {
	instruction: u16,
	file: bool,
	value: u32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step<'a> {
	File(&'a str),
	Line(u32),
	Other
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Code {
	offsets: Box<[u16]>,
	end: u32,
	marks: Box<[Mark]>,
	files: Box<[String]>,
	local_uses: Box<[(u32, u32, u32)]>
}

impl Code {
	pub fn read<E: DisassembleEnv>(bytecode: &[u32], env: &mut E) -> Code {
		fn locals_in(var: &dmasm::Variable, found: &mut Vec<u32>) {
			match var {
				dmasm::Variable::Local(slot) => found.push(*slot),
				dmasm::Variable::SetCache(a, b) => {
					locals_in(a, found);
					locals_in(b, found);
				}
				dmasm::Variable::Initial(inner) | dmasm::Variable::IsSaved(inner) => locals_in(inner, found),
				_ => {}
			}
		}

		let (nodes, _error) = dmasm::disassembler::disassemble(bytecode, env);
		let mut offsets = Vec::with_capacity(nodes.len());
		let mut end = 0;
		let mut marks = vec![];
		let mut files: Vec<String> = vec![];
		let mut local_uses: Vec<(u32, u32, u32)> = vec![];
		let mut found = vec![];
		for node in &nodes {
			let dmasm::Node::Instruction(ins, debug) = node else {
				continue;
			};
			let (Ok(offset), Ok(instruction)) = (u16::try_from(debug.offset), u16::try_from(offsets.len())) else {
				break;
			};
			match ins {
				dmasm::Instruction::DbgFile(name) => {
					let name = String::from_utf8_lossy(&name.0);
					let file = files.iter().position(|file| *file == name).unwrap_or_else(|| {
						files.push(name.into_owned());
						files.len() - 1
					});
					marks.push(Mark {
						instruction,
						file: true,
						value: file as u32
					});
				}
				dmasm::Instruction::DbgLine(line) => marks.push(Mark {
					instruction,
					file: false,
					value: *line
				}),
				_ => {}
			}
			found.clear();
			for var in ins.variables() {
				locals_in(var, &mut found);
			}
			for &slot in &found {
				match local_uses.binary_search_by_key(&slot, |(slot, ..)| *slot) {
					Ok(at) => local_uses[at].2 = debug.offset,
					Err(at) => local_uses.insert(at, (slot, debug.offset, debug.offset))
				}
			}
			offsets.push(offset);
			end = debug.offset + debug.bytecode.len() as u32;
		}
		Code {
			offsets: offsets.into(),
			end,
			marks: marks.into(),
			files: files.into(),
			local_uses: local_uses.into()
		}
	}

	fn steps(&self) -> impl Iterator<Item = (u32, Step<'_>)> {
		let mut marks = self.marks.iter().peekable();
		self.offsets.iter().enumerate().map(move |(at, &offset)| {
			let step = match marks.next_if(|mark| mark.instruction as usize == at) {
				Some(mark) if mark.file => Step::File(&self.files[mark.value as usize]),
				Some(mark) => Step::Line(mark.value),
				None => Step::Other
			};
			(offset as u32, step)
		})
	}

	pub fn file(&self) -> Option<&str> {
		match self.marks.first() {
			Some(mark) if mark.file && mark.instruction == 0 => Some(&self.files[mark.value as usize]),
			_ => None
		}
	}

	pub fn lines(&self) -> impl Iterator<Item = u32> + '_ {
		self.marks.iter().filter(|mark| !mark.file).map(|mark| mark.value)
	}

	pub fn lines_in(&self, wanted: impl Fn(&str) -> bool) -> Vec<u32> {
		let mut inside = false;
		let mut lines = vec![];
		for mark in self.marks.iter() {
			if mark.file {
				inside = wanted(&self.files[mark.value as usize]);
			} else if inside {
				lines.push(mark.value);
			}
		}
		lines
	}

	pub fn place(&self, offset: u32, before: bool) -> Option<(Option<&str>, u32)> {
		let mut line = None;
		let mut file = None;
		let mut reached = false;

		for (at, step) in self.steps() {
			if at > offset && !reached {
				reached = true;
				if line.is_some() {
					break;
				}
			}

			match step {
				Step::File(name) if !((reached || at == offset) && line.is_some()) => file = Some(name),
				Step::Line(_) if before && at == offset => {}
				Step::Line(here) => {
					line = Some(here);
					if reached {
						break;
					}
				}
				_ => {}
			}

			if at == offset {
				reached = true;
				if line.is_some() {
					break;
				}
			}
		}

		if reached {
			line.map(|line| (file, line))
		} else {
			None
		}
	}

	pub fn offset_of(&self, line: u32, wanted: Option<&dyn Fn(&str) -> bool>) -> Option<u32> {
		let mut at_line = false;
		let mut other_file = false;
		let mut inside = wanted.is_none();

		for (offset, step) in self.steps() {
			if at_line {
				match step {
					Step::File(_) => {
						other_file = true;
						continue;
					}
					Step::Line(_) if other_file => continue,
					_ => return Some(offset)
				}
			}

			match step {
				Step::File(name) => inside = wanted.is_none_or(|wanted| wanted(name)),
				Step::Line(here) if inside && here == line => at_line = true,
				_ => {}
			}
		}
		None
	}

	pub fn line_at(&self, offset: u32) -> Option<u32> {
		let mut line = None;
		for (at, step) in self.steps() {
			if at > offset {
				break;
			}
			if let Step::Line(here) = step {
				line = Some(here);
			}
		}
		line
	}

	pub fn entry_offset(&self) -> u32 {
		let mut has_line = false;
		for (offset, step) in self.steps() {
			match step {
				Step::File(_) => {}
				Step::Line(_) => has_line = true,
				Step::Other if has_line => return offset,
				Step::Other => return 0
			}
		}
		0
	}

	pub fn instruction_length(&self, offset: u32) -> Option<usize> {
		let at = self.offsets.binary_search_by(|start| (*start as u32).cmp(&offset)).ok()?;
		let next = self.offsets.get(at + 1).map_or(self.end, |next| *next as u32);
		Some((next - offset) as usize)
	}

	pub fn local_use(&self, slot: u32) -> Option<(u32, u32)> {
		let at = self.local_uses.binary_search_by_key(&slot, |(slot, ..)| *slot).ok()?;
		let (_, first, last) = self.local_uses[at];
		Some((first, last))
	}
}

fn alike_but_lines<E: DisassembleEnv>(a: &[u32], b: &[u32], env: &mut E) -> bool {
	fn instructions<E: DisassembleEnv>(bytecode: &[u32], env: &mut E) -> Vec<dmasm::Instruction> {
		let (nodes, _error) = dmasm::disassembler::disassemble(bytecode, env);
		nodes
			.into_iter()
			.filter_map(|node| match node {
				dmasm::Node::Instruction(dmasm::Instruction::DbgLine(_), _) => Some(dmasm::Instruction::DbgLine(0)),
				dmasm::Node::Instruction(ins, _) => Some(ins),
				_ => None
			})
			.collect()
	}
	a.len() == b.len() && instructions(a, env) == instructions(b, env)
}

struct Entry {
	at: usize,
	len: u16,
	code: Arc<Code>
}

pub type FileProcs = HashMap<String, Vec<u32>>;

#[derive(Default)]
struct Files {
	counted: u32,
	procs: Arc<FileProcs>,
	changed: BTreeSet<u32>
}

impl Files {
	fn update(&mut self, changed: &BTreeSet<u32>, read: Vec<(u32, Vec<String>)>, counted: u32) {
		let procs = Arc::make_mut(&mut self.procs);
		if !changed.is_empty() {
			procs.retain(|_, ids| {
				ids.retain(|id| !changed.contains(id));
				!ids.is_empty()
			});
		}
		for (id, files) in read {
			for file in files {
				let ids = procs.entry(file).or_default();
				if let Err(at) = ids.binary_search(&id) {
					ids.insert(at, id);
				}
			}
		}
		self.counted = counted;
	}
}

#[derive(Default)]
struct Index {
	procs: HashMap<u32, Entry>,
	old: HashMap<usize, Arc<Code>>,
	alike: HashMap<(usize, usize), bool>,
	files: Option<Files>
}

impl Index {
	fn get(&self, id: u32, at: usize, len: u16) -> Option<Arc<Code>> {
		self.procs
			.get(&id)
			.filter(|entry| entry.at == at && entry.len == len)
			.map(|entry| entry.code.clone())
	}

	fn keep(&mut self, id: u32, at: usize, len: u16, code: Arc<Code>) {
		let was = self.procs.insert(id, Entry { at, len, code });
		if was.is_some_and(|was| was.at != at || was.len != len) {
			if let Some(files) = &mut self.files {
				files.changed.insert(id);
			}
		}
	}

	fn proc_changed(&mut self, id: u32) {
		self.procs.remove(&id);
		if let Some(files) = &mut self.files {
			files.changed.insert(id);
		}
	}
}

static INDEX: LazyLock<Mutex<Index>> = LazyLock::new(Default::default);

fn index() -> Option<MutexGuard<'static, Index>> {
	match INDEX.try_lock() {
		Ok(index) => Some(index),
		Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
		Err(TryLockError::WouldBlock) => None
	}
}

fn index_for_sure() -> MutexGuard<'static, Index> {
	INDEX.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn of(proc: &Proc) -> Arc<Code> {
	let (at, len) = unsafe { proc.bytecode_mut_ptr() };
	let at = at as usize;
	if let Some(code) = index().and_then(|index| index.get(proc.id().0, at, len)) {
		return code;
	}
	let code = Arc::new(Code::read(&original_code(proc), &mut crate::DisassembleEnv));
	if let Some(mut index) = index() {
		index.keep(proc.id().0, at, len, code.clone());
	}
	code
}

pub fn running(proc: &Proc, running: *const u32) -> Option<Arc<Code>> {
	let (current, _) = unsafe { proc.bytecode_mut_ptr() };
	if std::ptr::eq(running, current) {
		return None;
	}
	let key = (running as usize, current as usize);
	if let Some(index) = index() {
		match index.alike.get(&key) {
			Some(true) => return None,
			Some(false) => {
				if let Some(code) = index.old.get(&key.0) {
					return Some(code.clone());
				}
			}
			None => {}
		}
	}

	let old = auxtools::bytecode_at(running)?;
	let alike = alike_but_lines(&old, &original_code(proc), &mut crate::DisassembleEnv);
	let code = (!alike).then(|| Arc::new(Code::read(&old, &mut crate::DisassembleEnv)));
	if !old.iter().any(|&word| is_breakpoint_word(word)) {
		if let Some(mut index) = index() {
			index.alike.insert(key, alike);
			if let Some(code) = &code {
				index.old.insert(key.0, code.clone());
			}
		}
	}
	code
}

fn start_file(bytecode: &[u32]) -> Option<String> {
	match bytecode {
		[op, id, ..] if *op == dmasm::Opcode::DbgFile.word() => {
			let name = unsafe { StringRef::from_id(raw_types::strings::StringId(*id)) };
			Some(String::from_utf8_lossy(name.data()).into_owned())
		}
		_ => None
	}
}

pub fn file_of(proc: &Proc) -> Option<String> {
	start_file(&original_code(proc))
}

fn files_named(id: u32) -> Vec<String> {
	let Some(proc) = Proc::from_id(ProcId(id)) else {
		return vec![];
	};
	let bytecode = original_code(&proc);
	let Some(file) = start_file(&bytecode) else {
		return vec![];
	};
	let dbg_file = dmasm::Opcode::DbgFile.word();
	let mut files = match bytecode.iter().skip(2).any(|&word| word == dbg_file) {
		true => Code::read(&bytecode, &mut crate::DisassembleEnv).files.into_vec(),
		false => vec![]
	};
	if !files.contains(&file) {
		files.push(file);
	}
	files
}

pub fn by_file() -> Arc<FileProcs> {
	let count = unsafe { auxtools::runtime_tables::proc_count() };
	let (counted, changed) = {
		let mut index = index_for_sure();
		let files = index.files.get_or_insert_default();
		if files.counted == count && files.changed.is_empty() {
			return files.procs.clone();
		}
		(files.counted, files.changed.clone())
	};

	let moved: BTreeSet<u32> = changed.iter().copied().filter(|&id| id < counted).collect();
	let read = (moved.iter().copied().chain(counted..count)).map(|id| (id, files_named(id))).collect();

	let mut index = index_for_sure();
	let files = index.files.get_or_insert_default();
	files.changed.retain(|id| !changed.contains(id));
	files.update(&moved, read, count);
	files.procs.clone()
}

pub fn proc_changed(id: ProcId) {
	index_for_sure().proc_changed(id.0);
}

fn forget_everything() {
	*index_for_sure() = Index::default();
}

#[shutdown]
fn world_ended() {
	forget_everything();
}

#[init(partial)]
fn world_loaded() -> Result<(), String> {
	forget_everything();
	Ok(())
}

#[cfg(test)]
mod tests {
	use std::{collections::BTreeSet, sync::Arc};

	use dmasm::Opcode;

	use super::{alike_but_lines, Code, Files, Index, Step};

	struct Env;

	impl dmasm::disassembler::DisassembleEnv for Env {
		fn get_string_data(&mut self, index: u32) -> Option<Vec<u8>> {
			Some(format!("file{}.dm", index).into_bytes())
		}

		fn get_variable_name(&mut self, index: u32) -> Option<Vec<u8>> {
			Some(format!("var{}", index).into_bytes())
		}

		fn get_proc_name(&mut self, index: u32) -> Option<String> {
			Some(format!("/proc/p{}", index))
		}

		fn value_to_string_data(&mut self, _tag: u32, _data: u32) -> Option<Vec<u8>> {
			None
		}
	}

	fn file(id: u32) -> [u32; 2] {
		[Opcode::DbgFile.word(), id]
	}

	fn line(line: u32) -> [u32; 2] {
		[Opcode::DbgLine.word(), line]
	}

	fn push(value: u32) -> [u32; 2] {
		[Opcode::PushInt.word(), value]
	}

	const POP: [u32; 1] = [0x51];

	fn ret() -> [u32; 1] {
		[Opcode::Ret.word()]
	}

	fn read(parts: &[&[u32]]) -> (Vec<u32>, Code) {
		let bytecode = parts.concat();
		let code = Code::read(&bytecode, &mut Env);
		(bytecode, code)
	}

	fn two_lines() -> Code {
		read(&[&file(1), &line(10), &push(1), &POP, &line(11), &push(2), &ret()]).1
	}

	fn two_files() -> Code {
		read(&[
			&file(1),
			&line(5),
			&push(1),
			&file(2),
			&line(30),
			&push(2),
			&file(1),
			&line(6),
			&file(9),
			&line(1),
			&file(1),
			&line(6),
			&push(3),
			&ret()
		])
		.1
	}

	mod by_the_book {
		use dmasm::{disassembler::disassemble, Instruction, Node};

		use super::Env;

		pub fn place(bytecode: &[u32], offset: u32, before: bool) -> Option<(Option<String>, u32)> {
			let (nodes, _error) = disassemble(bytecode, &mut Env);
			let mut current_line_number = None;
			let mut current_file = None;
			let mut reached_offset = false;

			for node in &nodes {
				let Node::Instruction(ins, debug) = node else { continue };
				let at = debug.offset;
				if at > offset && !reached_offset {
					reached_offset = true;
					if current_line_number.is_some() {
						break;
					}
				}

				match ins {
					Instruction::DbgFile(name) if !((reached_offset || at == offset) && current_line_number.is_some()) => {
						current_file = Some(String::from_utf8_lossy(&name.0).into_owned());
					}
					Instruction::DbgLine(_) if before && at == offset => {}
					Instruction::DbgLine(line) => {
						current_line_number = Some(*line);
						if reached_offset {
							break;
						}
					}
					_ => {}
				}

				if at == offset {
					reached_offset = true;
					if current_line_number.is_some() {
						break;
					}
				}
			}

			if reached_offset {
				current_line_number.map(|line| (current_file, line))
			} else {
				None
			}
		}

		pub fn offset_of(bytecode: &[u32], line: u32, files: Option<&[String]>) -> Option<u32> {
			let mut offset = None;
			let mut at_offset = false;
			let mut other_file = false;
			let mut inside = files.is_none();
			let (nodes, _error) = disassemble(bytecode, &mut Env);

			for node in nodes {
				if let Node::Instruction(ins, debug) = node {
					if at_offset {
						match ins {
							Instruction::DbgFile(_) => {
								other_file = true;
								continue;
							}
							Instruction::DbgLine(_) if other_file => continue,
							_ => {
								offset = Some(debug.offset);
								break;
							}
						}
					}

					match ins {
						Instruction::DbgFile(name) => {
							inside = files.is_none_or(|files| files.contains(&String::from_utf8_lossy(&name.0).into_owned()));
						}
						Instruction::DbgLine(current_line) if inside && current_line == line => at_offset = true,
						_ => {}
					}
				}
			}

			offset
		}

		pub fn lines_in(bytecode: &[u32], files: &[String]) -> Vec<u32> {
			let (nodes, _error) = disassemble(bytecode, &mut Env);
			let mut inside = false;
			let mut lines = vec![];
			for node in nodes {
				match node {
					Node::Instruction(Instruction::DbgFile(name), _) => {
						inside = files.contains(&String::from_utf8_lossy(&name.0).into_owned());
					}
					Node::Instruction(Instruction::DbgLine(line), _) if inside => lines.push(line),
					_ => {}
				}
			}
			lines
		}

		pub fn files_of(bytecode: &[u32]) -> Vec<String> {
			let (nodes, _error) = disassemble(bytecode, &mut Env);
			let mut files: Vec<String> = vec![];
			for node in nodes {
				if let Node::Instruction(Instruction::DbgFile(name), _) = node {
					let name = String::from_utf8_lossy(&name.0).into_owned();
					if !files.contains(&name) {
						files.push(name);
					}
				}
			}
			files
		}

		pub fn entry_offset(bytecode: &[u32]) -> u32 {
			let (nodes, _error) = disassemble(bytecode, &mut Env);
			let mut has_line = false;
			for node in nodes {
				match node {
					Node::Instruction(Instruction::DbgFile(_), _) => {}
					Node::Instruction(Instruction::DbgLine(_), _) => has_line = true,
					Node::Instruction(_, debug) if has_line => return debug.offset,
					Node::Instruction(..) => return 0,
					_ => {}
				}
			}
			0
		}

		pub fn line_at(bytecode: &[u32], offset: u32) -> Option<u32> {
			let mut line = None;
			for ins in dmasm::scan(bytecode).ok()?.instructions {
				if ins.offset > offset {
					break;
				}
				if ins.opcode == dmasm::Opcode::DbgLine {
					line = ins.bytecode.get(1).copied();
				}
			}
			line
		}

		pub fn first_line(bytecode: &[u32]) -> Option<u32> {
			dmasm::scan(bytecode)
				.ok()?
				.instructions
				.iter()
				.find(|ins| ins.opcode == dmasm::Opcode::DbgLine)
				.and_then(|ins| ins.bytecode.get(1).copied())
		}

		pub fn instruction_length(bytecode: &[u32], offset: u32) -> Option<usize> {
			let (nodes, _error) = disassemble(bytecode, &mut Env);
			nodes.into_iter().find_map(|node| match node {
				Node::Instruction(_, debug) if debug.offset == offset => Some(debug.bytecode.len()),
				_ => None
			})
		}
	}

	#[test]
	fn reads_instructions_marks_and_files() {
		let code = two_lines();
		assert_eq!(*code.offsets, [0, 2, 4, 6, 7, 9, 11]);
		assert_eq!(code.end, 12);
		assert_eq!(code.file(), Some("file1.dm"));
		assert_eq!(*code.files, ["file1.dm"]);
		assert_eq!(code.lines().collect::<Vec<_>>(), [10, 11]);
		let steps: Vec<_> = code.steps().collect();
		assert_eq!(steps, [
			(0, Step::File("file1.dm")),
			(2, Step::Line(10)),
			(4, Step::Other),
			(6, Step::Other),
			(7, Step::Line(11)),
			(9, Step::Other),
			(11, Step::Other)
		]);

		let code = two_files();
		assert_eq!(code.file(), Some("file1.dm"));
		assert_eq!(*code.files, ["file1.dm", "file2.dm", "file9.dm"]);
		assert_eq!(code.lines().collect::<Vec<_>>(), [5, 30, 6, 1, 6]);
	}

	#[test]
	fn code_without_debug_records_has_no_file_or_lines() {
		let (_, code) = read(&[&push(1), &ret()]);
		assert_eq!(code.file(), None);
		assert!(code.files.is_empty());
		assert_eq!(code.lines().count(), 0);
		assert_eq!(code.place(0, false), None);
		assert_eq!(code.offset_of(1, None), None);
		assert_eq!(code.entry_offset(), 0);

		let (_, code) = code_of_line_only();
		assert_eq!(code.file(), None);
		assert_eq!(code.place(2, false), Some((None, 3)));
		assert_eq!(code.entry_offset(), 2);

		let empty = Code::read(&[], &mut Env);
		assert_eq!(empty.place(0, false), None);
		assert_eq!(empty.entry_offset(), 0);
		assert_eq!(empty.instruction_length(0), None);
	}

	fn code_of_line_only() -> (Vec<u32>, Code) {
		read(&[&line(3), &push(1), &ret()])
	}

	#[test]
	fn stops_reading_where_the_code_does_not_disassemble() {
		let (bytecode, code) = read(&[&file(1), &line(10), &push(1), &[0xFFFF_FFF0], &line(11), &ret()]);
		assert_eq!(bytecode.len(), 10);
		assert_eq!(*code.offsets, [0, 2, 4]);
		assert_eq!(code.end, 6);
		assert_eq!(code.lines().collect::<Vec<_>>(), [10]);
		assert_eq!(code.offset_of(11, None), None);
		assert_eq!(code.instruction_length(4), Some(2));
		assert_eq!(code.instruction_length(6), None);
		assert_eq!(code.place(8, false), None);
		assert_eq!(code.place(4, false), Some((Some("file1.dm"), 10)));
	}

	#[test]
	fn a_line_starts_after_its_record() {
		let code = two_lines();
		assert_eq!(code.offset_of(10, None), Some(4));
		assert_eq!(code.offset_of(11, None), Some(9));
		assert_eq!(code.offset_of(12, None), None);
		assert_eq!(code.entry_offset(), 4);

		let (_, code) = read(&[&file(1), &line(10), &push(1), &line(11)]);
		assert_eq!(code.offset_of(11, None), None);

		let (_, code) = read(&[&file(1), &line(10), &line(11), &push(1), &ret()]);
		assert_eq!(code.offset_of(10, None), Some(4));
		assert_eq!(code.offset_of(11, None), Some(6));
	}

	#[test]
	fn a_line_is_of_the_file_asked_for() {
		let code = two_files();
		let first = |name: &str| name == "file1.dm";
		let second = |name: &str| name == "file2.dm";
		let dme = |name: &str| name == "file9.dm";
		assert_eq!(code.lines_in(first), [5, 6, 6]);
		assert_eq!(code.lines_in(second), [30]);
		assert_eq!(code.lines_in(dme), [1]);
		assert_eq!(code.lines_in(|_| false), Vec::<u32>::new());

		assert_eq!(code.offset_of(5, Some(&first)), Some(4));
		assert_eq!(code.offset_of(30, Some(&second)), Some(10));
		assert_eq!(code.offset_of(30, Some(&first)), None);
		assert_eq!(code.offset_of(30, None), Some(10));
		assert_eq!(code.offset_of(6, Some(&first)), Some(24));
		assert_eq!(code.offset_of(6, None), Some(24));
		assert_eq!(code.offset_of(1, Some(&dme)), Some(24));
		assert_eq!(code.offset_of(5, Some(&second)), None);
	}

	#[test]
	fn an_offset_is_on_the_line_and_in_the_file_before_it() {
		let code = two_lines();
		let at = |offset, before| code.place(offset, before).map(|(file, line)| (file.map(str::to_owned), line));
		let first = Some((Some("file1.dm".to_owned()), 10));
		let second = Some((Some("file1.dm".to_owned()), 11));
		assert_eq!(at(0, false), first);
		assert_eq!(at(2, false), first);
		assert_eq!(at(4, false), first);
		assert_eq!(at(6, false), first);
		assert_eq!(at(7, false), second);
		assert_eq!(at(7, true), first);
		assert_eq!(at(9, false), second);
		assert_eq!(at(11, false), second);
		assert_eq!(at(5, false), first);
		assert_eq!(at(10, false), second);
		assert_eq!(at(12, false), None);

		let code = two_files();
		let at = |offset, before| code.place(offset, before).map(|(file, line)| (file.map(str::to_owned), line));
		assert_eq!(at(4, false), Some((Some("file1.dm".to_owned()), 5)));
		assert_eq!(at(6, false), Some((Some("file1.dm".to_owned()), 5)));
		assert_eq!(at(10, false), Some((Some("file2.dm".to_owned()), 30)));
		assert_eq!(at(24, false), Some((Some("file1.dm".to_owned()), 6)));
		assert_eq!(at(18, false), Some((Some("file9.dm".to_owned()), 1)));
	}

	#[test]
	fn answers_are_what_a_fresh_disassembly_gave() {
		let mut seed = 0x2545_F491_4F6C_DD1Du64;
		let mut next = move |below: u64| {
			seed ^= seed << 13;
			seed ^= seed >> 7;
			seed ^= seed << 17;
			(seed % below) as u32
		};
		let name = |id: u32| format!("file{}.dm", id);
		for _ in 0..400 {
			let mut bytecode = vec![];
			for _ in 0..next(14) {
				match next(13) {
					0 | 1 => bytecode.extend(file(1 + next(3))),
					2..=5 => bytecode.extend(line(1 + next(5))),
					6 | 7 => bytecode.extend(POP),
					8 => bytecode.push(0xFFFF_FFF0),
					_ => bytecode.extend(push(next(9)))
				}
			}
			let code = Code::read(&bytecode, &mut Env);
			let said = format!("{:?}", bytecode);
			if dmasm::scan(&bytecode).is_ok() {
				assert_eq!(code.lines().next(), by_the_book::first_line(&bytecode), "{}", said);
				for offset in 0..bytecode.len() as u32 + 2 {
					assert_eq!(code.line_at(offset), by_the_book::line_at(&bytecode, offset), "{} at {}", said, offset);
				}
			}
			assert_eq!(*code.files, by_the_book::files_of(&bytecode), "{}", said);
			assert_eq!(code.entry_offset(), by_the_book::entry_offset(&bytecode), "{}", said);
			for offset in 0..bytecode.len() as u32 + 2 {
				for before in [false, true] {
					let place = code.place(offset, before).map(|(file, line)| (file.map(str::to_owned), line));
					assert_eq!(
						place,
						by_the_book::place(&bytecode, offset, before),
						"{} at {} ({})",
						said,
						offset,
						before
					);
				}
				assert_eq!(
					code.instruction_length(offset),
					by_the_book::instruction_length(&bytecode, offset),
					"{} at {}",
					said,
					offset
				);
			}
			for files in [vec![], vec![name(1)], vec![name(2), name(3)]] {
				let wanted = |file: &str| files.iter().any(|wanted| wanted == file);
				assert_eq!(code.lines_in(wanted), by_the_book::lines_in(&bytecode, &files), "{} in {:?}", said, files);
				for line in 1..=5 {
					assert_eq!(
						code.offset_of(line, Some(&wanted)),
						by_the_book::offset_of(&bytecode, line, Some(&files)),
						"{} line {} in {:?}",
						said,
						line,
						files
					);
				}
			}
			for line in 1..=5 {
				assert_eq!(
					code.offset_of(line, None),
					by_the_book::offset_of(&bytecode, line, None),
					"{} line {}",
					said,
					line
				);
			}
		}
	}

	#[test]
	fn hot_reload_finds_lines_whatever_their_file() {
		let code = two_files();
		assert_eq!(code.line_at(0), None);
		assert_eq!(code.line_at(2), Some(5));
		assert_eq!(code.line_at(9), Some(30));
		assert_eq!(code.line_at(19), Some(1));
		assert_eq!(code.line_at(40), Some(6));
		assert_eq!(code.offset_of(5, None), Some(4));
		assert_eq!(code.offset_of(6, None), Some(24));
		assert_eq!(code.offset_of(30, None), Some(10));
		assert_eq!(code.offset_of(7, None), None);
		let (_, code) = read(&[&file(1), &line(10)]);
		assert_eq!(code.offset_of(10, None), None);
	}

	#[test]
	fn a_proc_breakpoint_goes_after_the_first_records() {
		assert_eq!(two_lines().entry_offset(), 4);
		assert_eq!(two_files().entry_offset(), 4);
		let (_, code) = read(&[&file(1), &line(10)]);
		assert_eq!(code.entry_offset(), 0);
		let (_, code) = read(&[&file(1), &push(1), &ret()]);
		assert_eq!(code.entry_offset(), 0);
	}

	#[test]
	fn an_instruction_has_the_words_up_to_the_next() {
		let code = two_lines();
		assert_eq!(code.instruction_length(0), Some(2));
		assert_eq!(code.instruction_length(6), Some(1));
		assert_eq!(code.instruction_length(9), Some(2));
		assert_eq!(code.instruction_length(11), Some(1));
		assert_eq!(code.instruction_length(1), None);
		assert_eq!(code.instruction_length(12), None);
	}

	#[test]
	fn locals_are_used_from_their_first_instruction_to_their_last() {
		let get_var = Opcode::GetVar.word();
		let set_var = Opcode::SetVar.word();
		let local_marker = 0xFFDA;
		let (_, code) = read(&[
			&line(1),
			&[get_var, local_marker, 1],
			&[set_var, local_marker, 0],
			&[get_var, local_marker, 1],
			&POP,
			&ret()
		]);
		assert_eq!(*code.offsets, [0, 2, 5, 8, 11, 12]);
		assert_eq!(code.local_use(0), Some((5, 5)));
		assert_eq!(code.local_use(1), Some((2, 8)));
		assert_eq!(code.local_use(2), None);
	}

	#[test]
	fn code_is_alike_when_only_its_lines_differ() {
		let (a, _) = read(&[&file(1), &line(10), &push(1), &ret()]);
		let (moved, _) = read(&[&file(1), &line(14), &push(1), &ret()]);
		let (edited, _) = read(&[&file(1), &line(10), &push(2), &ret()]);
		let (longer, _) = read(&[&file(1), &line(10), &push(1), &POP, &ret()]);
		let (other_file, _) = read(&[&file(2), &line(10), &push(1), &ret()]);
		assert!(alike_but_lines(&a, &a, &mut Env));
		assert!(alike_but_lines(&a, &moved, &mut Env));
		assert!(!alike_but_lines(&a, &edited, &mut Env));
		assert!(!alike_but_lines(&a, &longer, &mut Env));
		assert!(!alike_but_lines(&a, &other_file, &mut Env));
	}

	#[test]
	fn an_entry_is_of_the_code_the_proc_had_when_it_was_read() {
		let mut index = Index::default();
		let read = Arc::new(two_lines());
		index.keep(7, 0x1000, 12, read.clone());
		assert!(index.get(7, 0x1000, 12).is_some_and(|code| Arc::ptr_eq(&code, &read)));
		assert!(index.get(7, 0x2000, 12).is_none());
		assert!(index.get(7, 0x1000, 13).is_none());
		assert!(index.get(8, 0x1000, 12).is_none());

		index.proc_changed(7);
		assert!(index.get(7, 0x1000, 12).is_none());
	}

	#[test]
	fn the_files_follow_procs_that_change_and_take_in_new_ones() {
		let names = |files: &[&str]| files.iter().map(|file| file.to_string()).collect::<Vec<_>>();
		let mut files = Files::default();
		files.update(
			&BTreeSet::new(),
			vec![(0, names(&["a.dm"])), (1, names(&["b.dm", "a.dm"])), (2, vec![]), (3, names(&["b.dm"]))],
			4
		);
		assert_eq!(files.procs["a.dm"], [0, 1]);
		assert_eq!(files.procs["b.dm"], [1, 3]);
		assert_eq!(files.procs.len(), 2);
		assert_eq!(files.counted, 4);

		let changed: BTreeSet<u32> = [0, 3].into_iter().collect();
		files.update(&changed, vec![(0, vec![]), (3, names(&["c.dm"])), (4, names(&["a.dm"]))], 5);
		assert_eq!(files.procs["a.dm"], [1, 4]);
		assert_eq!(files.procs["b.dm"], [1]);
		assert_eq!(files.procs["c.dm"], [3]);
		let changed: BTreeSet<u32> = [3].into_iter().collect();
		files.update(&changed, vec![(3, names(&["b.dm"]))], 5);
		assert_eq!(files.procs["b.dm"], [1, 3]);
		assert!(!files.procs.contains_key("c.dm"));

		let held = files.procs.clone();
		let changed: BTreeSet<u32> = [1].into_iter().collect();
		files.update(&changed, vec![(1, names(&["c.dm"]))], 5);
		assert_eq!(held["a.dm"], [1, 4]);
		assert_eq!(files.procs["a.dm"], [4]);
		assert_eq!(files.procs["c.dm"], [1]);
	}

	#[test]
	fn a_proc_read_again_unasked_has_its_files_read_again() {
		let mut index = Index::default();
		index.files = Some(Files::default());
		index.keep(7, 0x1000, 12, Arc::new(two_lines()));
		index.keep(7, 0x1000, 12, Arc::new(two_lines()));
		assert!(index.files.as_ref().unwrap().changed.is_empty());
		index.keep(7, 0x2000, 30, Arc::new(two_files()));
		assert_eq!(index.files.as_ref().unwrap().changed.iter().copied().collect::<Vec<_>>(), [7]);
		index.proc_changed(9);
		assert_eq!(index.files.as_ref().unwrap().changed.iter().copied().collect::<Vec<_>>(), [7, 9]);
	}
}
