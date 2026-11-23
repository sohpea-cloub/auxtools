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
	name_map_plans_survive_new_types()?;

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

fn name_map_plans_survive_new_types() -> DMResult<()> {
	use runtime_tables::{add_type, name_map_entries, plan_name_map, type_count, type_field, type_fields};

	let failed = |what: &str, e: String| runtime!("test_tables: couldn't {}: {}", what, e);
	let original = name_map_entries(0).map_err(|e| failed("read a proc name map", e))?;
	let key = StringRef::new("auxtest_planned_name")?;
	let planned = vec![(key.get_id().0, vec![0], 0)];
	let plan = plan_name_map(0, &planned).map_err(|e| failed("plan a proc name map", e))?;

	let datum = StringRef::new("/datum")?.get_id().0;
	let parent = (0..type_count().map_err(|e| failed("count the types", e))?)
		.find(|&id| type_field(id, type_fields::PATH) == Ok(datum))
		.ok_or_else(|| runtime!("test_tables: there's no /datum type"))?;
	for i in 0..64 {
		let path = StringRef::new(&format!("/datum/auxtest_added_{}", i))?;
		add_type(parent, path.get_id().0).map_err(|e| failed("add a type", e))?;
		std::mem::forget(path);
	}

	plan.write().map_err(|e| failed("write a proc name map", e))?;
	let written = name_map_entries(0).map_err(|e| failed("read a proc name map", e))?;
	plan_name_map(0, &original)
		.and_then(|plan| plan.write())
		.map_err(|e| failed("restore a proc name map", e))?;
	if written != planned {
		return Err(runtime!(
			"test_tables: a proc name map planned before 64 new types reads {:?} after it was written, expected {:?}",
			written,
			planned
		));
	}
	Ok(())
}
