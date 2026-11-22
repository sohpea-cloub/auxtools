use std::ffi::c_void;

use crate::{
	raw_types::{funcs, lists, procs},
	registry::{self, builds, find}
};

pub(crate) fn resolve_full() -> Result<(), String> {
	let build = registry::byond_build()?;
	macro_rules! function {
		($name:literal) => {
			find!($name)? as *const c_void
		};
	}

	unsafe {
		funcs::CURRENT_EXECUTION_CONTEXT = find!("current_execution_context")? as *mut *mut procs::ExecutionContext;
		let queue = find!("suspended_procs")?;
		let (front, back) = if cfg!(unix) && build >= builds::LINUX_FRAMELESS {
			(queue + 4, queue)
		} else {
			(queue, queue + 4)
		};
		funcs::SUSPENDED_FRONT = front as *const usize;
		funcs::SUSPENDED_BACK = back as *const usize;
		funcs::SUSPENDED_PROCS_BUFFER = find!("suspended_procs_buffer")? as *mut procs::SuspendedProcsBuffer;
		if build >= builds::FIRST_516 {
			funcs::ALIST_TABLE = find!("alist_table_ptr")? as *const *const *const lists::Alist;
			funcs::ALIST_TABLE_COUNT = find!("alist_table_count")? as *const u32;
		} else {
			static NO_ALISTS: u32 = 0;
			funcs::ALIST_TABLE = std::ptr::null();
			funcs::ALIST_TABLE_COUNT = &NO_ALISTS;
		}
		funcs::call_proc_by_id_byond = function!("call_proc_by_id");
		funcs::call_datum_proc_by_name_byond = function!("call_datum_proc_by_name");
		funcs::get_proc_array_entry_byond = function!("get_proc_array_entry");
		funcs::get_string_id_byond = function!("get_string_id");
		funcs::get_variable_byond = function!("get_variable");
		funcs::set_variable_byond = function!("set_variable");
		funcs::get_string_table_entry_byond = function!("get_string_table_entry");
		funcs::inc_ref_count_byond = function!("inc_ref_count");
		funcs::dec_ref_count_byond = function!("dec_ref_count");
		funcs::get_assoc_element_byond = function!("get_assoc_element");
		funcs::set_assoc_element_byond = function!("set_assoc_element");
		funcs::create_list_byond = function!("create_list");
		funcs::append_to_list_byond = function!("append_to_list");
		funcs::remove_from_list_byond = function!("remove_from_list");
		#[cfg(unix)]
		{
			funcs::remove_from_list_in_registers = build < builds::LINUX_REMOVE_FROM_LIST_ON_STACK;
			funcs::get_length_returns_count = build < builds::LINUX_FRAMELESS;
		}
		funcs::get_length_byond = function!("get_length");
		funcs::value_is_list_byond = function!("value_is_list");
		funcs::get_misc_by_id_byond = function!("get_misc_by_id");
		funcs::to_string_byond = function!("to_string");
		funcs::runtime_byond = function!("runtime");
	}

	Ok(())
}

pub(crate) fn resolve_partial() -> Result<(), String> {
	let build = registry::byond_build()?;
	let names = find!("variable_names")?;
	let count = if cfg!(unix) && build >= builds::LINUX_FRAMELESS {
		names - 4
	} else {
		names + 4
	};
	unsafe {
		funcs::VARIABLE_NAME_ENTRIES = names as *const *const crate::raw_types::strings::StringId;
		funcs::VARIABLE_NAME_COUNT = count as *const u32;
	}
	Ok(())
}
