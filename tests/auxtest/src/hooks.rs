use auxtools::*;

const DISCARDED: &str = "a hook result nobody is going to look at";

#[hook("/datum/auxtest_discard/New")]
fn discard_new() {
	Value::from_string(DISCARDED)
}

#[hook("/proc/auxtest_hooked_global")]
fn hooked_global(x: Value) {
	Ok(Value::from(x.as_number()? + 1.0))
}

#[hook("/datum/auxtest_hooked/proc/hooked_method")]
fn hooked_method(x: Value) {
	Ok(Value::from(x.as_number()? * 2.0))
}

#[hook("/proc/auxtest_panics")]
fn panics() {
	panic!("auxtest asked for this panic");
}

byond_ffi_fn! { auxtest_ffi_panics() {
	if true {
		panic!("auxtest asked for this panic");
	}
	Some("unreachable")
} }

#[hook("/proc/auxtest_hooks")]
fn test_hooks() {
	use raw_types::{funcs, strings};

	let held = StringRef::new(DISCARDED)?;

	unsafe {
		let mut entry: *mut strings::StringEntry = std::ptr::null_mut();
		assert_eq!(funcs::get_string_table_entry(&mut entry, held.get_id()), 1);

		let before = (*entry).ref_count;
		Proc::find("/proc/auxtest_new_discard").unwrap().call(&[])?;
		if (*entry).ref_count != before {
			return Err(runtime!("test_hooks: a discarded hook result kept its reference"));
		}
	}

	let by_id = Proc::find("/proc/auxtest_hooked_global").unwrap().call(&[&Value::from(41.0)])?;
	if by_id.as_number()? != 42.0 {
		return Err(runtime!("test_hooks: calling a hooked global proc from Rust did not reach the hook"));
	}

	let datum = Proc::find("/proc/auxtest_new_hooked").unwrap().call(&[])?;
	let by_name = datum.call("hooked_method", &[&Value::from(21.0)])?;
	if by_name.as_number()? != 42.0 {
		return Err(runtime!("test_hooks: calling a hooked datum proc from Rust did not reach the hook"));
	}

	Ok(Value::from(true))
}
