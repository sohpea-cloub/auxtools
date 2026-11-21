//! Cheaper process CPU time for BYOND on Linux.
//!
//! BYOND's `TimeLib::CpuClock()` returns `clock()`, the process's CPU time,
//! which is a system call: about 0.7 µs on a virtual machine. The interpreter
//! reads it on proc calls and returns in some states, which can make each call
//! several times slower. The detour adds the monotonic time since the last
//! `clock()` reading instead, which the kernel serves without a system call,
//! and reads `clock()` again once a millisecond has passed, so time asleep or
//! descheduled is still left out.

use std::{
	cell::RefCell,
	ffi::{c_char, c_long, c_void},
	sync::Mutex,
	time::{Duration, Instant}
};

use retour::RawDetour;

/// `TimeLib::CpuClock()`'s result: no seconds, then the microseconds of CPU
/// time `clock()` gives
#[repr(C)]
struct CpuTime {
	seconds: u32,
	micros: c_long
}

extern "C" {
	fn clock() -> c_long;
	fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// glibc's `RTLD_DEFAULT`: search every loaded object
const RTLD_DEFAULT: *mut c_void = std::ptr::null_mut();

/// Time between `clock()` readings
const RESYNC: Duration = Duration::from_millis(1);

struct Clock {
	/// The last `clock()` reading and when it was taken
	synced: (c_long, Instant),
	/// The last value returned
	last: c_long
}

static CLOCK: Mutex<Option<Clock>> = Mutex::new(None);

thread_local!(static DETOUR: RefCell<Option<RawDetour>> = const { RefCell::new(None) });

impl Clock {
	fn new() -> Self {
		let real = unsafe { clock() };
		Self {
			synced: (real, Instant::now()),
			last: real
		}
	}

	fn read(&mut self) -> c_long {
		let now = Instant::now();
		let since = now.duration_since(self.synced.1);
		let value = if since < RESYNC {
			self.synced.0.wrapping_add(since.as_micros() as c_long)
		} else {
			let real = unsafe { clock() };
			self.synced = (real, now);
			real
		};
		// A reading after a pause can be behind the estimate; hold until it
		// catches up. `clock()` wraps, so compare the difference.
		if value.wrapping_sub(self.last) > 0 {
			self.last = value;
		}
		self.last
	}
}

extern "C" fn cpu_clock() -> CpuTime {
	let mut clock = CLOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
	CpuTime {
		seconds: 0,
		micros: clock.get_or_insert_with(Clock::new).read()
	}
}

/// Detour `TimeLib::CpuClock()`. Builds without it run as they are.
pub fn init() {
	let target = unsafe { dlsym(RTLD_DEFAULT, c"_ZN7TimeLib8CpuClockEv".as_ptr()) };
	if target.is_null() {
		return;
	}
	let Ok(detour) = (unsafe { RawDetour::new(target as *const (), cpu_clock as *const ()) }) else {
		return;
	};
	if unsafe { detour.enable() }.is_ok() {
		DETOUR.with(|cell| *cell.borrow_mut() = Some(detour));
	}
}

pub fn shutdown() {
	DETOUR.with(|cell| {
		if let Some(detour) = cell.borrow_mut().take() {
			let _ = unsafe { detour.disable() };
		}
	});
}
