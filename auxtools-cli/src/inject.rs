use std::{ffi::OsStr, mem::size_of, os::windows::ffi::OsStrExt, path::Path, ptr::null_mut};

use winapi::{
	ctypes::c_void,
	shared::{minwindef::FALSE, ntdef::NT_SUCCESS},
	um::{
		handleapi::{CloseHandle, INVALID_HANDLE_VALUE},
		libloaderapi::{GetModuleHandleA, GetProcAddress},
		memoryapi::{ReadProcessMemory, VirtualAllocEx, VirtualProtectEx, WriteProcessMemory},
		processthreadsapi::{FlushInstructionCache, OpenThread, ResumeThread},
		tlhelp32::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32},
		winnt::{HANDLE, MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, THREAD_SUSPEND_RESUME}
	}
};

const ATTACH_EXPORT: &[u8] = b"auxtools_debug_attach\0";

const PATCH_LEN: usize = 6;

fn last_error(what: &str) -> String {
	format!("{}: {}", what, std::io::Error::last_os_error())
}

fn read<T: Copy>(process: HANDLE, address: usize) -> Result<T, String> {
	let mut value = std::mem::MaybeUninit::<T>::uninit();
	let ok = unsafe {
		ReadProcessMemory(
			process,
			address as *const c_void,
			value.as_mut_ptr() as *mut c_void,
			size_of::<T>(),
			null_mut()
		)
	};
	if ok == FALSE {
		return Err(last_error("couldn't read the program's memory"));
	}
	Ok(unsafe { value.assume_init() })
}

fn write(process: HANDLE, address: usize, bytes: &[u8]) -> Result<(), String> {
	let ok = unsafe { WriteProcessMemory(process, address as *mut c_void, bytes.as_ptr() as *const c_void, bytes.len(), null_mut()) };
	if ok == FALSE {
		return Err(last_error("couldn't write to the program's memory"));
	}
	Ok(())
}

#[repr(C)]
struct ProcessBasicInformation {
	exit_status: i32,
	peb_base_address: usize,
	affinity_mask: usize,
	base_priority: i32,
	unique_process_id: usize,
	inherited_from_unique_process_id: usize
}

type NtQueryInformationProcess = unsafe extern "system" fn(HANDLE, u32, *mut c_void, u32, *mut u32) -> i32;

fn entry_point(process: HANDLE) -> Result<usize, String> {
	let query: NtQueryInformationProcess = unsafe {
		let function = GetProcAddress(GetModuleHandleA(c"ntdll.dll".as_ptr()), c"NtQueryInformationProcess".as_ptr());
		if function.is_null() {
			return Err(last_error("couldn't find NtQueryInformationProcess"));
		}
		std::mem::transmute(function)
	};

	let mut info: ProcessBasicInformation = unsafe { std::mem::zeroed() };
	let status = unsafe {
		query(
			process,
			0,
			&mut info as *mut _ as *mut c_void,
			size_of::<ProcessBasicInformation>() as u32,
			null_mut()
		)
	};
	if !NT_SUCCESS(status) {
		return Err(format!("couldn't find the program's PEB (status {:#x})", status));
	}

	let image_base: u32 = read(process, info.peb_base_address + 8)?;
	let image_base = image_base as usize;
	let pe_header: u32 = read(process, image_base + 0x3C)?;
	let entry_rva: u32 = read(process, image_base + pe_header as usize + 0x28)?;
	Ok(image_base + entry_rva as usize)
}

fn kernel32(name: &[u8]) -> Result<u32, String> {
	unsafe {
		let module = GetModuleHandleA(c"kernel32.dll".as_ptr());
		let function = GetProcAddress(module, name.as_ptr() as *const i8);
		if function.is_null() {
			return Err(last_error("couldn't find kernel32's functions"));
		}
		Ok(function as usize as u32)
	}
}

pub fn attach_on_entry(process: HANDLE, library: &Path) -> Result<(), String> {
	let entry = entry_point(process)?;
	let original: [u8; PATCH_LEN] = read(process, entry)?;

	let load_library = kernel32(b"LoadLibraryW\0")?;
	let get_proc_address = kernel32(b"GetProcAddress\0")?;

	let path: Vec<u8> = OsStr::new(library).encode_wide().chain([0]).flat_map(u16::to_le_bytes).collect();

	const CODE_LEN: usize = 64;
	let size = CODE_LEN + PATCH_LEN + path.len() + ATTACH_EXPORT.len();
	let stub = unsafe { VirtualAllocEx(process, null_mut(), size, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) } as usize;
	if stub == 0 {
		return Err(last_error("couldn't allocate memory in the program"));
	}
	let saved = stub + CODE_LEN;
	let path_at = saved + PATCH_LEN;
	let name_at = path_at + path.len();

	let le = |v: usize| (v as u32).to_le_bytes();
	let mut code = vec![];
	code.extend([0x60, 0x9C]);
	code.push(0x68);
	code.extend(le(path_at));
	code.push(0xB8);
	code.extend(le(load_library as usize));
	code.extend([0xFF, 0xD0]);
	code.extend([0x85, 0xC0, 0x74, 0x13]);
	code.push(0x68);
	code.extend(le(name_at));
	code.push(0x50);
	code.push(0xB8);
	code.extend(le(get_proc_address as usize));
	code.extend([0xFF, 0xD0]);
	code.extend([0x85, 0xC0, 0x74, 0x02]);
	code.extend([0xFF, 0xD0]);
	code.push(0xBE);
	code.extend(le(saved));
	code.push(0xBF);
	code.extend(le(entry));
	code.push(0xB9);
	code.extend(le(PATCH_LEN));
	code.extend([0xFC, 0xF3, 0xA4]);
	code.extend([0x9D, 0x61]);
	code.push(0x68);
	code.extend(le(entry));
	code.push(0xC3);
	assert!(code.len() <= CODE_LEN);
	code.resize(CODE_LEN, 0xCC);
	code.extend(original);
	code.extend(&path);
	code.extend(ATTACH_EXPORT);
	write(process, stub, &code)?;

	let mut old = 0;
	if unsafe { VirtualProtectEx(process, entry as *mut c_void, PATCH_LEN, PAGE_EXECUTE_READWRITE, &mut old) } == FALSE {
		return Err(last_error("couldn't make the program's entry point writable"));
	}
	let mut patch = vec![0x68];
	patch.extend(le(stub));
	patch.push(0xC3);
	write(process, entry, &patch)?;

	unsafe {
		FlushInstructionCache(process, entry as *const c_void, PATCH_LEN);
		FlushInstructionCache(process, stub as *const c_void, size);
	}
	Ok(())
}

pub fn resume(pid: u32) -> Result<(), String> {
	unsafe {
		let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
		if snapshot == INVALID_HANDLE_VALUE {
			return Err(last_error("couldn't list the program's threads"));
		}

		let mut entry: THREADENTRY32 = std::mem::zeroed();
		entry.dwSize = size_of::<THREADENTRY32>() as u32;
		let mut resumed = 0;
		let mut more = Thread32First(snapshot, &mut entry) != FALSE;
		while more {
			if entry.th32OwnerProcessID == pid {
				let thread = OpenThread(THREAD_SUSPEND_RESUME, FALSE, entry.th32ThreadID);
				if !thread.is_null() {
					if ResumeThread(thread) != u32::MAX {
						resumed += 1;
					}
					CloseHandle(thread);
				}
			}
			more = Thread32Next(snapshot, &mut entry) != FALSE;
		}
		CloseHandle(snapshot);

		if resumed == 0 {
			return Err(last_error("couldn't start the program"));
		}
	}
	Ok(())
}
