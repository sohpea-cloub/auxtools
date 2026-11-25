use std::{
	collections::VecDeque,
	ffi::CStr,
	sync::{
		atomic::{AtomicUsize, Ordering},
		Mutex
	},
	time::{Duration, Instant}
};

use winapi::um::{
	libloaderapi::{GetModuleHandleA, GetProcAddress},
	winnt::{IMAGE_DIRECTORY_ENTRY_IMPORT, IMAGE_DOS_HEADER, IMAGE_IMPORT_DESCRIPTOR, IMAGE_NT_HEADERS32}
};

pub const HOLD_VAR: &str = "AUXTOOLS_HOLD_SOCKETS";

const HOLD: Duration = Duration::from_secs(90);

type Close = unsafe extern "system" fn(usize) -> i32;
type Shutdown = unsafe extern "system" fn(usize, i32) -> i32;

static CLOSE: AtomicUsize = AtomicUsize::new(0);
static SHUTDOWN: AtomicUsize = AtomicUsize::new(0);
static HELD: Mutex<VecDeque<(Instant, usize)>> = Mutex::new(VecDeque::new());

pub fn install() -> Result<usize, String> {
	unsafe {
		let wsock = GetModuleHandleA(c"wsock32.dll".as_ptr());
		let core = GetModuleHandleA(c"byondcore.dll".as_ptr());
		if wsock.is_null() || core.is_null() {
			return Err("byondcore.dll or wsock32.dll not loaded".to_owned());
		}
		let close = GetProcAddress(wsock, c"closesocket".as_ptr()) as usize;
		let shutdown = GetProcAddress(wsock, c"shutdown".as_ptr()) as usize;
		if close == 0 || shutdown == 0 {
			return Err("wsock32.dll lacks closesocket or shutdown".to_owned());
		}
		CLOSE.store(close, Ordering::Relaxed);
		SHUTDOWN.store(shutdown, Ordering::Relaxed);

		let base = core as usize;
		let dos = &*(base as *const IMAGE_DOS_HEADER);
		let nt = &*((base + dos.e_lfanew as usize) as *const IMAGE_NT_HEADERS32);
		let imports = nt.OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT as usize];
		if imports.VirtualAddress == 0 {
			return Err("byondcore.dll has no imports".to_owned());
		}
		let mut replaced = 0;
		let mut descriptor = (base + imports.VirtualAddress as usize) as *const IMAGE_IMPORT_DESCRIPTOR;
		while (*descriptor).Name != 0 {
			let name = CStr::from_ptr((base + (*descriptor).Name as usize) as *const i8);
			if name.to_bytes().eq_ignore_ascii_case(b"wsock32.dll") {
				let mut slot = (base + (*descriptor).FirstThunk as usize) as *mut usize;
				while *slot != 0 {
					if *slot == close {
						region::protect(slot as *const u8, 4, region::Protection::READ_WRITE)
							.map_err(|e| format!("cannot write socket import: {}", e))?;
						*slot = held_close as *const () as usize;
						replaced += 1;
					}
					slot = slot.add(1);
				}
			}
			descriptor = descriptor.add(1);
		}
		match replaced {
			0 => Err("byondcore.dll has no wsock32.dll closesocket import".to_owned()),
			n => Ok(n)
		}
	}
}

unsafe extern "system" fn held_close(socket: usize) -> i32 {
	let close: Close = std::mem::transmute(CLOSE.load(Ordering::Relaxed));
	let shutdown: Shutdown = std::mem::transmute(SHUTDOWN.load(Ordering::Relaxed));
	let now = Instant::now();
	let mut held = HELD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
	if held.iter().any(|&(_, at)| at == socket) {
		return 0;
	}
	shutdown(socket, 2);
	held.push_back((now, socket));
	while let Some(&(since, old)) = held.front() {
		if now.duration_since(since) < HOLD {
			break;
		}
		held.pop_front();
		close(old);
	}
	0
}
