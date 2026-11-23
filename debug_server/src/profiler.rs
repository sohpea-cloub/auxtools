use std::{collections::HashMap, time::Instant};

use auxtools::{
	instruction_hook::{ticks_since_last, time_instructions, timestamp},
	*
};

use auxtools_protocol::{ProfiledInstruction, ProfiledTotal};

const MOST_CALLS: usize = 1024;

const CODE_CACHE_SLOTS: usize = 256;

const FLOOR_HITS: u64 = 256;

const SAMPLE_SLOTS: usize = 4096;

const SAMPLE_PATIENCE: u8 = 16;

#[derive(Clone, Copy)]
struct Sample {
	instruction: usize,
	timed: u64,
	ticks: u64,
	passed: u8
}

#[derive(Clone, Copy, Default)]
struct Counter {
	hits: u64,
	timed: u64,
	ticks: u64,
	callee_ticks: u64,
	callee_timed: u64
}

struct Code {
	proc: u32,
	bytecode: usize,
	index: Box<[u32]>,
	counters: Vec<Counter>,
	offsets: Vec<u16>
}

#[derive(Clone, Copy)]
struct At(*mut Code, u32);

impl At {
	const NONE: Self = Self(std::ptr::null_mut(), 0);

	unsafe fn counter<'a>(self) -> Option<&'a mut Counter> {
		unsafe { self.0.as_mut() }.and_then(|code| code.counters.get_mut(self.1 as usize))
	}
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Frame {
	ctx: usize,
	instance: usize
}

impl Frame {
	const NONE: Self = Self { ctx: 0, instance: 0 };

	unsafe fn of(ctx: *mut raw_types::procs::ExecutionContext) -> Self {
		if ctx.is_null() {
			return Self::NONE;
		}
		Self {
			ctx: ctx as usize,
			instance: unsafe { *(*ctx).proc_instance() } as usize
		}
	}
}

struct Call {
	caller: Frame,
	counter: At,
	ran: u64,
	timed: u64
}

pub struct Profiler {
	only: Option<Vec<u32>>,
	codes: HashMap<(u32, usize), Option<Box<Code>>>,
	cache: [(u32, usize, *mut Code); CODE_CACHE_SLOTS],
	last: Frame,
	last_parent: Frame,
	last_counter: At,
	last_opcode: u32,
	sleep: u32,
	evaluating: bool,
	last_instruction: usize,
	samples: Box<[Sample]>,
	replaced_floor: f64,
	calls: Vec<Call>,
	ran: u64,
	timed: u64,
	started: (Instant, u64),
	stopped: Option<(Instant, u64, f64)>
}

static mut PROFILER: Option<Box<Profiler>> = None;
static mut RUNNING: bool = false;

pub fn start(only: Option<Vec<u32>>) {
	unsafe {
		PROFILER = Some(Box::new(Profiler::new(only)));
		RUNNING = true;
	}
	time_instructions(true);
}

pub fn stop() {
	unsafe {
		if RUNNING {
			RUNNING = false;
			time_instructions(false);
			if let Some(profiler) = PROFILER.as_mut() {
				profiler.stopped = Some((Instant::now(), timestamp(), profiler.floor()));
			}
		}
	}
}

pub fn discard() {
	stop();
	unsafe {
		PROFILER = None;
	}
}

pub fn exists() -> bool {
	unsafe { PROFILER.is_some() }
}

pub fn running() -> bool {
	unsafe { RUNNING }
}

#[shutdown]
fn profiler_shutdown() {
	discard();
}

#[inline]
pub fn enter(ctx: *mut raw_types::procs::ExecutionContext, evaluating: bool) {
	unsafe {
		if RUNNING {
			if let Some(profiler) = PROFILER.as_mut() {
				profiler.enter(ctx, evaluating);
			}
		}
	}
}

pub(crate) fn can_jump(opcode: u32) -> bool {
	use dmasm::Opcode::*;
	[
		Jmp,
		Jnz,
		Jz,
		Spawn,
		Switch,
		PickSwitch,
		SwitchRange,
		JmpOr,
		JmpAnd,
		JmpLoop,
		JnzLoop,
		JzLoop,
		ForRange,
		ForRangeStep,
		Try,
		Catch,
		TryJmp,
		SetCacheJmpIfNull,
		SetCachePopJmpIfNull
	]
	.iter()
	.any(|op| op.word() == opcode)
}

impl Profiler {
	fn new(only: Option<Vec<u32>>) -> Self {
		Profiler {
			only,
			codes: HashMap::new(),
			cache: [(0, 0, std::ptr::null_mut()); CODE_CACHE_SLOTS],
			last: Frame::NONE,
			last_parent: Frame::NONE,
			last_counter: At::NONE,
			last_opcode: 0,
			sleep: dmasm::Opcode::Sleep.word(),
			evaluating: false,
			last_instruction: 0,
			samples: vec![
				Sample {
					instruction: 0,
					timed: 0,
					ticks: 0,
					passed: 0
				};
				SAMPLE_SLOTS
			]
			.into_boxed_slice(),
			replaced_floor: f64::INFINITY,
			calls: Vec::new(),
			ran: 0,
			timed: 0,
			started: (Instant::now(), timestamp()),
			stopped: None
		}
	}

	unsafe fn enter(&mut self, ctx: *mut raw_types::procs::ExecutionContext, evaluating: bool) {
		if evaluating != self.evaluating {
			self.evaluating = evaluating;
			self.last = Frame::NONE;
			self.last_parent = Frame::NONE;
			self.last_counter = At::NONE;
		}
		let spent = ticks_since_last();
		let (frame, parent, bytecode, offset) = unsafe {
			(
				Frame::of(ctx),
				Frame::of(*(*ctx).parent_context()),
				*(*ctx).bytecode(),
				*(*ctx).bytecode_offset() as usize
			)
		};

		let mut returned = None;
		let followed = if frame == self.last {
			offset != 0 || can_jump(self.last_opcode)
		} else if parent == self.last && self.last != Frame::NONE {
			true
		} else if let Some(depth) = self.calls.iter().rposition(|call| call.caller == frame) {
			returned = Some(depth);
			true
		} else {
			frame == self.last_parent
		};
		if let Some(spent) = spent.filter(|_| followed && self.last_opcode != self.sleep) {
			self.ran += spent;
			self.timed += 1;
			self.sample(spent);
			if let Some(counter) = unsafe { self.last_counter.counter() } {
				counter.ticks += spent;
				counter.timed += 1;
			}
		}

		if !followed {
			self.calls.clear();
		} else if let Some(depth) = returned {
			self.calls.truncate(depth + 1);
			if let Some(call) = self.calls.pop() {
				if let Some(counter) = unsafe { call.counter.counter() } {
					counter.callee_ticks += self.ran - call.ran;
					counter.callee_timed += self.timed - call.timed;
				}
			}
		} else if frame != self.last && parent == self.last && self.calls.len() < MOST_CALLS {
			self.calls.push(Call {
				caller: self.last,
				counter: self.last_counter,
				ran: self.ran,
				timed: self.timed
			});
		}

		let code = self.code(bytecode, unsafe { (*(frame.instance as *mut raw_types::procs::ProcInstance)).proc });
		self.last_counter = match unsafe { code.as_mut() }.and_then(|code| code.hit(offset)) {
			Some(at) => At(code, at),
			None => At::NONE
		};
		self.last = frame;
		self.last_parent = parent;
		self.last_opcode = unsafe { *bytecode.add(offset) };
		self.last_instruction = bytecode as usize + offset * std::mem::size_of::<u32>();
	}

	fn sample(&mut self, spent: u64) {
		let sample = &mut self.samples[(self.last_instruction >> 2) % SAMPLE_SLOTS];
		if sample.instruction == self.last_instruction {
			sample.passed = 0;
		} else if sample.passed < SAMPLE_PATIENCE && sample.instruction != 0 {
			sample.passed += 1;
			return;
		} else {
			if sample.timed >= FLOOR_HITS {
				self.replaced_floor = self.replaced_floor.min(sample.ticks as f64 / sample.timed as f64);
			}
			*sample = Sample {
				instruction: self.last_instruction,
				timed: 0,
				ticks: 0,
				passed: 0
			};
		}
		sample.timed += 1;
		sample.ticks += spent;
	}

	fn code(&mut self, bytecode: *mut u32, proc: raw_types::procs::ProcId) -> *mut Code {
		let address = bytecode as usize;
		let slot = (address >> 3 ^ address >> 11 ^ proc.0 as usize) % CODE_CACHE_SLOTS;
		let (cached_proc, cached_address, cached) = self.cache[slot];
		if cached_address == address && cached_proc == proc.0 {
			return cached;
		}
		let only = &self.only;
		let code = self
			.codes
			.entry((proc.0, address))
			.or_insert_with(|| Self::measured(only.as_deref(), bytecode, proc));
		let code = code.as_deref_mut().map_or(std::ptr::null_mut(), std::ptr::from_mut);
		self.cache[slot] = (proc.0, address, code);
		code
	}

	fn measured(only: Option<&[u32]>, bytecode: *mut u32, proc: raw_types::procs::ProcId) -> Option<Box<Code>> {
		if only.is_some_and(|only| !only.contains(&proc.0)) {
			return None;
		}
		let proc = Proc::from_id(proc)?;
		if crate::launched::is_helper(&proc) {
			return None;
		}
		let (current, len) = unsafe { proc.bytecode_mut_ptr() };
		let len = if current == bytecode {
			len as usize
		} else {
			auxtools::bytecode_at(bytecode)?.len()
		};
		Some(Box::new(Code {
			proc: proc.id.0,
			bytecode: bytecode as usize,
			index: vec![0; len].into_boxed_slice(),
			counters: vec![],
			offsets: vec![]
		}))
	}

	fn clock(&self) -> (f64, f64) {
		let (ended, ticks) = self
			.stopped
			.map_or_else(|| (Instant::now(), timestamp()), |(ended, ticks, _)| (ended, ticks));
		let elapsed = ended.duration_since(self.started.0);
		let ticks = ticks.wrapping_sub(self.started.1).max(1);
		(elapsed.as_nanos() as f64 / ticks as f64, elapsed.as_secs_f64())
	}

	fn floor(&self) -> f64 {
		if let Some((.., floor)) = self.stopped {
			return floor;
		}
		let floor = (self.samples.iter())
			.filter(|sample| sample.timed >= FLOOR_HITS)
			.map(|sample| sample.ticks as f64 / sample.timed as f64)
			.fold(self.replaced_floor, f64::min);
		if floor.is_finite() {
			floor
		} else {
			0.0
		}
	}
}

impl Code {
	fn hit(&mut self, offset: usize) -> Option<u32> {
		let index = self.index.get_mut(offset)?;
		if *index == 0 {
			self.counters.push(Counter::default());
			self.offsets.push(offset as u16);
			*index = self.counters.len() as u32;
		}
		let at = *index - 1;
		self.counters[at as usize].hits += 1;
		Some(at)
	}
}

impl Counter {
	fn took(&self, floor: f64, ns: f64) -> (u64, u64) {
		let own = match self.timed {
			0 => 0.0,
			timed => (self.ticks as f64 / timed as f64 - floor).max(0.0) * self.hits as f64
		};
		let callee = (self.callee_ticks as f64 - floor * self.callee_timed as f64).max(0.0);
		((own * ns) as u64, (callee * ns) as u64)
	}
}

pub struct Totals {
	pub seconds: f64,
	pub running: bool,
	pub overhead: f64,
	pub codes: Vec<(u32, usize, ProfiledTotal)>
}

pub fn totals() -> Option<Totals> {
	let profiler = unsafe { PROFILER.as_ref() }?;
	let (ns, seconds) = profiler.clock();
	let floor = profiler.floor();
	let mut codes = vec![];
	for code in profiler.codes.values().flatten() {
		let first = code.index.first().and_then(|first| code.counters.get((*first as usize).checked_sub(1)?));
		let mut total = ProfiledTotal {
			calls: first.map_or(0, |first| first.hits),
			hits: 0,
			own_ns: 0,
			callee_ns: 0
		};
		for counter in &code.counters {
			let (own, callee) = counter.took(floor, ns);
			total.hits += counter.hits;
			total.own_ns += own;
			total.callee_ns += callee;
		}
		if total.hits != 0 {
			codes.push((code.proc, code.bytecode, total));
		}
	}
	Some(Totals {
		seconds,
		running: running(),
		overhead: floor * ns,
		codes
	})
}

pub fn instructions(proc: u32, bytecode: usize) -> Option<Vec<ProfiledInstruction>> {
	let profiler = unsafe { PROFILER.as_ref() }?;
	let code = profiler.codes.get(&(proc, bytecode))?.as_ref()?;
	let (ns, _) = profiler.clock();
	let floor = profiler.floor();
	let mut instructions: Vec<ProfiledInstruction> = (code.counters.iter().zip(&code.offsets))
		.map(|(counter, offset)| {
			let (own_ns, callee_ns) = counter.took(floor, ns);
			ProfiledInstruction {
				offset: *offset,
				hits: counter.hits,
				own_ns,
				callee_ns
			}
		})
		.collect();
	instructions.sort_by_key(|instruction| instruction.offset);
	Some(instructions)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn floor_keeps_samples_that_another_instruction_replaced() {
		let mut profiler = Profiler::new(None);
		profiler.last_instruction = 0x1000;
		for _ in 0..FLOOR_HITS {
			profiler.sample(100);
		}
		profiler.last_instruction = 0x1000 + SAMPLE_SLOTS * 4;
		for _ in 0..=SAMPLE_PATIENCE {
			profiler.sample(500);
		}
		assert_eq!(profiler.samples[0x1000 >> 2].instruction, profiler.last_instruction);
		assert_eq!(profiler.floor(), 100.0);
	}
}
