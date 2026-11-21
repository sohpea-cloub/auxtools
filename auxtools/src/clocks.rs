//! Cheaper clocks for BYOND on Linux.
//!
//! While a world's profiler is on (`world.Profile`), BYOND reads two clocks on
//! every proc call and return: `TimeLib::CpuClock()`, three times a call, and
//! `TimeLib::Now()`, twice. `CpuClock` returns `clock()`, the process's CPU
//! time, which is a system call: about 0.7 µs on a virtual machine, enough to
//! make a call ten times slower. `Now` is `gettimeofday()`, which the kernel
//! serves without one but not for nothing: 55 ns on the same machine.
//!
//! Both are detoured to read the real clock once a millisecond and in
//! between add the time the processor's timestamp counter says has passed,
//! which costs a quarter of `gettimeofday()`. For the CPU clock that takes
//! the process to have been running since the last reading; a reading after
//! it slept or was descheduled is behind what was returned, and the clock
//! holds until it catches up. A counter that jumps, stops or changes pace
//! costs readings their accuracy for a millisecond at most: a jump or a
//! thousand readings bring the next real one, and the pace is measured again
//! at each.
//!
//! On Windows BYOND's `CpuClock` is the C runtime's `clock()`, which reads
//! the performance counter, and `Now` is `_ftime64()`: neither needs this.

use std::{
	cell::RefCell,
	ffi::{c_char, c_long, c_void},
	sync::{
		atomic::{AtomicU32, Ordering},
		Mutex
	},
	time::Instant
};

use retour::RawDetour;

/// What `TimeLib::CpuClock()` and `TimeLib::Now()` return: a `timeval`.
/// `CpuClock` leaves the seconds at 0 and gives the microseconds of CPU
/// time `clock()` does, which wrap.
#[repr(C)]
struct TimeVal {
	seconds: c_long,
	micros: c_long
}

extern "C" {
	fn clock() -> c_long;
	fn gettimeofday(time: *mut TimeVal, zone: *mut c_void) -> i32;
	fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// glibc's `RTLD_DEFAULT`: search every loaded object
const RTLD_DEFAULT: *mut c_void = std::ptr::null_mut();

/// Microseconds between real readings
const PERIOD: u32 = 1000;

/// Estimates served between real readings whatever the counter says
const MOST_ESTIMATES: u32 = 1000;

fn counter() -> u64 {
	unsafe { core::arch::x86::_rdtsc() }
}

/// A clock's last real reading: the counter then, the reading's seconds and
/// microseconds, and the counter's pace. Read without a lock: `sequence` is
/// odd while the rest is being written and moves on when it has been.
struct Anchor {
	sequence: AtomicU32,
	counter_low: AtomicU32,
	counter_high: AtomicU32,
	seconds: AtomicU32,
	micros: AtomicU32,
	/// Microseconds a counter tick, in 2^-32ths; 0 until a pace is known
	pace: AtomicU32,
	/// Counter ticks in `PERIOD`
	period: AtomicU32,
	/// Estimates served since the reading. Counted without care: a lost
	/// count delays the next real reading by one estimate.
	estimates: AtomicU32
}

impl Anchor {
	const fn new() -> Self {
		Self {
			sequence: AtomicU32::new(0),
			counter_low: AtomicU32::new(0),
			counter_high: AtomicU32::new(0),
			seconds: AtomicU32::new(0),
			micros: AtomicU32::new(0),
			pace: AtomicU32::new(0),
			period: AtomicU32::new(0),
			estimates: AtomicU32::new(0)
		}
	}

	/// The seconds and microseconds of the last reading plus the time the
	/// counter says has passed, or `None` when a real reading is due
	fn estimate(&self) -> Option<(u32, u32)> {
		loop {
			let sequence = self.sequence.load(Ordering::Acquire);
			let low = self.counter_low.load(Ordering::Relaxed);
			let high = self.counter_high.load(Ordering::Relaxed);
			let seconds = self.seconds.load(Ordering::Relaxed);
			let micros = self.micros.load(Ordering::Relaxed);
			let pace = self.pace.load(Ordering::Relaxed);
			let period = self.period.load(Ordering::Relaxed);
			if sequence & 1 == 1 || self.sequence.load(Ordering::Acquire) != sequence {
				std::hint::spin_loop();
				continue;
			}
			let estimates = self.estimates.load(Ordering::Relaxed);
			if pace == 0 || estimates >= MOST_ESTIMATES {
				return None;
			}
			// A counter that went back wraps to far more than a period
			let ticks = counter().wrapping_sub((high as u64) << 32 | low as u64);
			if ticks >= period as u64 {
				return None;
			}
			self.estimates.store(estimates + 1, Ordering::Relaxed);
			return Some((seconds, micros.wrapping_add(((ticks * pace as u64) >> 32) as u32)));
		}
	}

	/// Record a real reading taken at `counter`
	fn set(&self, counter: u64, seconds: u32, micros: u32, pace: u32, period: u32) {
		self.sequence.fetch_add(1, Ordering::AcqRel);
		self.counter_low.store(counter as u32, Ordering::Relaxed);
		self.counter_high.store((counter >> 32) as u32, Ordering::Relaxed);
		self.seconds.store(seconds, Ordering::Relaxed);
		self.micros.store(micros, Ordering::Relaxed);
		self.pace.store(pace, Ordering::Relaxed);
		self.period.store(period, Ordering::Relaxed);
		self.estimates.store(0, Ordering::Relaxed);
		self.sequence.fetch_add(1, Ordering::AcqRel);
	}
}

/// What a real reading is taken with: when the last was, on the monotonic
/// clock and the counter, for the counter's pace, and what that reading was,
/// for the latest time an estimate can have returned since
struct Reading {
	last: Option<(Instant, u64)>,
	pace: u32,
	period: u32,
	/// The counter, seconds and microseconds of the last reading
	anchor: Option<(u64, u32, u32)>
}

impl Reading {
	const fn new() -> Self {
		Self {
			last: None,
			pace: 0,
			period: 0,
			anchor: None
		}
	}

	/// The microseconds an estimate made at `counter` adds to the last
	/// reading: no estimate since can have added more
	fn ahead(&self, counter: u64) -> Option<u32> {
		let (then, ..) = self.anchor?;
		if self.pace == 0 {
			return Some(0);
		}
		let ticks = counter.wrapping_sub(then).min(self.period as u64);
		Some(((ticks * self.pace as u64) >> 32) as u32)
	}

	/// Measure the counter's pace over the time since the last reading, if
	/// that is long enough to measure it by, and note this one
	fn pace(&mut self, now: Instant, counter: u64) {
		let Some((then, counter_then)) = self.last else {
			self.last = Some((now, counter));
			return;
		};
		let micros = now.duration_since(then).as_micros();
		// Half a period at least
		if micros < (PERIOD / 2) as u128 {
			return;
		}
		self.last = Some((now, counter));
		let ticks = counter.wrapping_sub(counter_then);
		// Too long ago, or a counter that stood still, went back or ticks
		// less than once a microsecond: no estimates until the next reading
		if micros > u32::MAX as u128 || ticks <= micros as u64 || ticks > u64::MAX >> 33 {
			self.pace = 0;
			return;
		}
		self.pace = (((micros as u64) << 32) / ticks) as u32;
		self.period = (ticks * PERIOD as u64 / micros as u64).min(u32::MAX as u64) as u32;
	}
}

static CPU: Anchor = Anchor::new();
static CPU_READING: Mutex<Reading> = Mutex::new(Reading::new());
static WALL: Anchor = Anchor::new();
static WALL_READING: Mutex<Reading> = Mutex::new(Reading::new());

thread_local!(static DETOURS: RefCell<Vec<RawDetour>> = const { RefCell::new(Vec::new()) });

extern "C" fn cpu_clock() -> TimeVal {
	if let Some((_, micros)) = CPU.estimate() {
		return TimeVal {
			seconds: 0,
			micros: micros as c_long
		};
	}
	let mut reading = CPU_READING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
	let (now, counter) = (Instant::now(), counter());
	let mut micros = unsafe { clock() } as u32;
	// Behind what an estimate has returned, when the process didn't run all
	// the time since the last reading: hold there until it catches up.
	// `clock()` wraps, so it is the difference that says which is ahead.
	if let (Some(ahead), Some((_, _, last))) = (reading.ahead(counter), reading.anchor) {
		let returned = last.wrapping_add(ahead);
		let behind = returned.wrapping_sub(micros);
		if behind as i32 > 0 && behind <= PERIOD {
			micros = returned;
		}
	}
	reading.pace(now, counter);
	reading.anchor = Some((counter, 0, micros));
	CPU.set(counter, 0, micros, reading.pace, reading.period);
	TimeVal {
		seconds: 0,
		micros: micros as c_long
	}
}

extern "C" fn now() -> TimeVal {
	let (mut seconds, mut micros) = match WALL.estimate() {
		Some(estimate) => estimate,
		None => {
			let mut reading = WALL_READING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
			let (now, counter) = (Instant::now(), counter());
			let mut real = TimeVal { seconds: 0, micros: 0 };
			unsafe { gettimeofday(&mut real, std::ptr::null_mut()) };
			let mut read = (real.seconds as u32, real.micros as u32);
			// A little behind what an estimate has returned: hold there. A
			// clock that was set back by more is followed.
			if let (Some(ahead), Some((_, last_seconds, last_micros))) = (reading.ahead(counter), reading.anchor) {
				let returned = last_seconds as i64 * 1_000_000 + (last_micros + ahead) as i64;
				let behind = returned - (read.0 as i64 * 1_000_000 + read.1 as i64);
				if behind > 0 && behind <= PERIOD as i64 {
					read = ((returned / 1_000_000) as u32, (returned % 1_000_000) as u32);
				}
			}
			reading.pace(now, counter);
			reading.anchor = Some((counter, read.0, read.1));
			WALL.set(counter, read.0, read.1, reading.pace, reading.period);
			read
		}
	};
	// An estimate adds a period at most to microseconds below a million
	if micros >= 1_000_000 {
		micros -= 1_000_000;
		seconds += 1;
	}
	TimeVal {
		seconds: seconds as c_long,
		micros: micros as c_long
	}
}

/// Detour `TimeLib::CpuClock()` and `TimeLib::Now()`. Builds without them
/// run as they are.
pub fn init() {
	let clocks: [(&std::ffi::CStr, *const ()); 2] = [
		(c"_ZN7TimeLib8CpuClockEv", cpu_clock as *const ()),
		(c"_ZN7TimeLib3NowEv", now as *const ())
	];
	for (symbol, ours) in clocks {
		let target = unsafe { dlsym(RTLD_DEFAULT, symbol.as_ptr()) };
		if target.is_null() {
			continue;
		}
		let Ok(detour) = (unsafe { RawDetour::new(target as *const (), ours) }) else {
			continue;
		};
		if unsafe { detour.enable() }.is_ok() {
			DETOURS.with(|cell| cell.borrow_mut().push(detour));
		}
	}
}

pub fn shutdown() {
	DETOURS.with(|cell| {
		for detour in cell.borrow_mut().drain(..) {
			let _ = unsafe { detour.disable() };
		}
	});
}
