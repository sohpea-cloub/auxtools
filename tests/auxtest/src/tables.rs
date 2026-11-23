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

	Ok(Value::from(true))
}
