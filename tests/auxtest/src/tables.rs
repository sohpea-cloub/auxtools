use auxtools::*;

#[hook("/proc/auxtest_tables")]
fn test_tables() {
	unsafe {
		offsets_are_checked()?;
		#[cfg(target_os = "linux")]
		dropped_name_map_plans_free_their_memory()?;
		name_map_plans_survive_new_types()?;
		name_maps_refuse_uncountable_lists()?;
		parent_plans_check_their_world()?;
		mob_edits_survive_new_types()?;
	}
	Ok(Value::from(true))
}

unsafe fn offsets_are_checked() -> DMResult<()> {
	use runtime_tables::{type_field, type_fields};

	type_field(0, type_fields::PARENT).map_err(|e| runtime!("test_tables: couldn't read a type's parent: {}", e))?;

	for offset in [0x98, type_fields::MAPTEXT_Y] {
		if let Ok(word) = type_field(0, offset) {
			return Err(runtime!("test_tables: type_field read {:#x} at the bad offset +{:#x}", word, offset));
		}
	}
	Ok(())
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
unsafe fn dropped_name_map_plans_free_their_memory() -> DMResult<()> {
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

unsafe fn name_map_plans_survive_new_types() -> DMResult<()> {
	use runtime_tables::{name_map_entries, plan_name_map};

	let failed = |what: &str, e: String| runtime!("test_tables: couldn't {}: {}", what, e);
	let original = name_map_entries(0).map_err(|e| failed("read a proc name map", e))?;
	let key = StringRef::new("auxtest_planned_name")?;
	let planned = vec![(key.get_id().0, vec![0], 0)];
	let plan = plan_name_map(0, &planned).map_err(|e| failed("plan a proc name map", e))?;

	add_types("/datum", 64)?;

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

unsafe fn name_maps_refuse_uncountable_lists() -> DMResult<()> {
	use runtime_tables::{plan_name_map, set_named_procs};

	const TOO_MANY: u32 = u16::MAX as u32 + 1;
	let key = StringRef::new("auxtest_too_many_procs")?.get_id().0;
	let mut accepted = vec![];
	if plan_name_map(0, &[(key, vec![0; TOO_MANY as usize], 0)]).is_ok() {
		accepted.push("a plan with 65,536 procs for one name");
	}
	let names: Vec<(u32, Vec<u32>, u8)> = (0..TOO_MANY).map(|key| (key, vec![0], 0)).collect();
	if plan_name_map(0, &names).is_ok() {
		accepted.push("a plan with 65,536 names");
	}
	if set_named_procs(0, key, &vec![0; TOO_MANY as usize]).is_ok() {
		accepted.push("65,536 procs for one name");
	}
	if !accepted.is_empty() {
		return Err(runtime!("test_tables: proc name maps accepted {}", accepted.join(", ")));
	}
	Ok(())
}

static EARLIER_PLAN: std::sync::Mutex<Option<runtime_tables::ParentPlan>> = std::sync::Mutex::new(None);

unsafe fn parent_plans_check_their_world() -> DMResult<()> {
	use runtime_tables::plan_parents;

	let failed = |what: &str, e: String| runtime!("test_tables: couldn't {}: {}", what, e);
	if let Some(earlier) = EARLIER_PLAN.lock().unwrap().take() {
		if earlier.write().is_ok() {
			return Err(runtime!("test_tables: a type hierarchy planned in an earlier world was written"));
		}
	}
	plan_parents(&[])
		.and_then(|plan| plan.write())
		.map_err(|e| failed("write the type hierarchy as it is", e))?;

	let plan = plan_parents(&[]).map_err(|e| failed("plan the type hierarchy", e))?;
	add_types("/datum", 1)?;
	if plan.write().is_ok() {
		return Err(runtime!("test_tables: a type hierarchy planned before a type was added was written"));
	}
	*EARLIER_PLAN.lock().unwrap() = Some(plan_parents(&[]).map_err(|e| failed("plan the type hierarchy", e))?);
	Ok(())
}

unsafe fn type_id(path: &str) -> DMResult<u32> {
	use runtime_tables::{type_count, type_field, type_fields};

	let path_id = StringRef::new(path)?.get_id().0;
	let count = type_count().map_err(|e| runtime!("test_tables: couldn't count the types: {}", e))?;
	(0..count)
		.find(|&id| type_field(id, type_fields::PATH) == Ok(path_id))
		.ok_or_else(|| runtime!("test_tables: there's no {} type", path))
}

unsafe fn add_types(parent: &str, count: usize) -> DMResult<()> {
	static ADDED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
	let parent_id = type_id(parent)?;
	for _ in 0..count {
		let n = ADDED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		let path = StringRef::new(&format!("{}/auxtest_added_{}", parent, n))?;
		runtime_tables::add_type(parent_id, path.get_id().0).map_err(|e| runtime!("test_tables: couldn't add a type: {}", e))?;
		std::mem::forget(path);
	}
	Ok(())
}

unsafe fn mob_edits_survive_new_types() -> DMResult<()> {
	use runtime_tables::set_mob_type;

	let failed = |what: &str, e: String| runtime!("test_tables: couldn't {}: {}", what, e);
	let mob = type_id("/mob")?;
	let mut added = Ok(());
	let mut old = 0;
	set_mob_type(mob, |entry| {
		old = entry.sight;
		added = add_types("/mob", 64);
		entry.sight = 0x1234;
	})
	.map_err(|e| failed("set a mob type's sight", e))?;
	added?;
	let mut sight = 0;
	set_mob_type(mob, |entry| {
		sight = entry.sight;
		entry.sight = old;
	})
	.map_err(|e| failed("read a mob type's sight", e))?;
	if sight != 0x1234 {
		return Err(runtime!(
			"test_tables: a mob type's sight set while 64 mob types were added reads {:#x}, expected 0x1234",
			sight
		));
	}
	Ok(())
}
