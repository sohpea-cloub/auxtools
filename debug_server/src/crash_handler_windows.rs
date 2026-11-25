use auxtools::*;
use winapi::{
	shared::{minwindef::HMODULE, ntdef::LONG},
	um::{
		errhandlingapi::SetUnhandledExceptionFilter,
		libloaderapi::{
			GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT
		},
		winnt::EXCEPTION_POINTERS
	},
	vc::excpt::EXCEPTION_EXECUTE_HANDLER
};

use crate::server_types::BreakpointReason;

extern "system" fn exception_filter(info: *mut EXCEPTION_POINTERS) -> LONG {
	let _ = auxtools::guard(|| unsafe { report(info) });
	crate::on_server(|dbg| {
		let ctx = unsafe { *raw_types::funcs::CURRENT_EXECUTION_CONTEXT };

		dbg.handle_breakpoint(ctx, BreakpointReason::Runtime {
			message: "native exception".to_owned(),
			caught: false
		});
	});

	EXCEPTION_EXECUTE_HANDLER
}

unsafe fn report(info: *mut EXCEPTION_POINTERS) {
	let Some(record) = info.as_ref().and_then(|info| info.ExceptionRecord.as_ref()) else {
		return;
	};
	let address = record.ExceptionAddress as usize;
	let mut module: HMODULE = std::ptr::null_mut();
	let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
	let place = if GetModuleHandleExW(flags, address as *const u16, &mut module) != 0 {
		let mut name = [0u16; 260];
		let length = GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) as usize;
		let path = String::from_utf16_lossy(&name[..length]);
		let file = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_owned();
		format!("{}+{:#x}", file, address - module as usize)
	} else {
		format!("{:#x}", address)
	};
	let access = match (record.ExceptionCode, record.NumberParameters >= 2) {
		(0xC000_0005, true) => {
			let what = match record.ExceptionInformation[0] {
				0 => "reading",
				1 => "writing",
				8 => "executing",
				_ => "accessing"
			};
			format!(" {} {:#x}", what, record.ExceptionInformation[1])
		}
		_ => String::new()
	};
	eprintln!("auxtools: native exception {:#010x} at {}{}", record.ExceptionCode, place, access);
	let ctx = *raw_types::funcs::CURRENT_EXECUTION_CONTEXT;
	if let Some(ctx) = ctx.as_ref() {
		let instance = *ctx.proc_instance();
		if let Some(proc) = instance.as_ref().and_then(|instance| Proc::from_id(instance.proc)) {
			eprintln!("auxtools: running {} at line {}", proc.path, ctx.line());
		}
	}
}

#[init(full)]
fn crash_handler_init() -> Result<(), String> {
	unsafe {
		SetUnhandledExceptionFilter(Some(exception_filter));
	}

	Ok(())
}
