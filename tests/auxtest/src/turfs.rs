use auxtools::*;

#[hook("/proc/auxtest_turfs")]
fn test_turfs(first: Value, last: Value) {
	let world = Value::WORLD;
	let max_x = world.get_number(byond_string!("maxx"))? as u32;
	let max_y = world.get_number(byond_string!("maxy"))? as u32;
	let max_z = world.get_number(byond_string!("maxz"))? as u32;
	let count = max_x * max_y * max_z;

	let checks = [
		("turf(1, 1, 1)", Value::turf(1, 1, 1)?, &first),
		("turf_by_id(0)", Value::turf_by_id(0)?, &first),
		("turf(maxx, maxy, maxz)", Value::turf(max_x, max_y, max_z)?, &last),
		("turf_by_id(the last id)", Value::turf_by_id(count - 1)?, &last)
	];
	for (name, got, expected) in checks {
		if got != *expected {
			return Err(runtime!("test_turfs: {} gave {}, expected {}", name, got, expected));
		}
	}

	if let Ok(past_end) = Value::turf_by_id(count) {
		return Err(runtime!("test_turfs: turf_by_id({}), one past the last turf, gave {}", count, past_end));
	}
	if let Ok(past_end) = Value::turf(max_x + 1, max_y, max_z) {
		return Err(runtime!("test_turfs: turf(maxx + 1, maxy, maxz) gave {}", past_end));
	}

	Ok(Value::from(true))
}
