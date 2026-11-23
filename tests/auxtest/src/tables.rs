use auxtools::*;

#[hook("/proc/auxtest_tables")]
fn test_tables() {
	use runtime_tables::{type_field, type_fields};

	type_field(0, type_fields::PARENT).map_err(|e| runtime!("test_tables: couldn't read a type's parent: {}", e))?;

	for offset in [0x98, type_fields::MAPTEXT_Y] {
		if let Ok(word) = type_field(0, offset) {
			return Err(runtime!("test_tables: type_field read {:#x} at the bad offset +{:#x}", word, offset));
		}
	}

	#[cfg(target_os = "linux")]
	dropped_name_map_plans_free_their_memory()?;

	Ok(Value::from(true))
}

#[cfg(target_os = "linux")]
fn heap_in_use() -> usize {
	use std::os::raw::c_int;
	#[repr(C)]
	struct MallInfo {
		arena: c_int,
		ordblks: c_int,
		smblks: c_int,
		hblks: c_int,
		hblkhd: c_int,
		usmblks: c_int,
		fsmblks: c_int,
		uordblks: c_int,
		fordblks: c_int,
		keepcost: c_int
	}
	extern "C" {
		fn mallinfo() -> MallInfo;
	}
	unsafe { mallinfo().uordblks as usize }
}

#[cfg(target_os = "linux")]
fn dropped_name_map_plans_free_their_memory() -> DMResult<()> {
	use runtime_tables::{name_map_entries, plan_name_map};

	const PLANS: usize = 10_000;
	let entries = name_map_entries(0).map_err(|e| runtime!("test_tables: couldn't read a proc name map: {}", e))?;
	let before = heap_in_use();
	for _ in 0..PLANS {
		drop(plan_name_map(0, &entries).map_err(|e| runtime!("test_tables: couldn't plan a proc name map: {}", e))?);
	}
	let grown = heap_in_use().saturating_sub(before);
	if grown >= PLANS * 8 {
		return Err(runtime!(
			"test_tables: dropping {} proc name map plans left {} bytes allocated",
			PLANS,
			grown
		));
	}
	Ok(())
}
