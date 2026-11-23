use std::{
	cell::RefCell,
	collections::HashMap,
	convert::TryFrom,
	panic::{catch_unwind, AssertUnwindSafe}
};

use auxtools::{raw_types::procs::ProcId, *};

fn quiet<R>(work: impl FnOnce() -> R) -> std::thread::Result<R> {
	let hook = std::panic::take_hook();
	std::panic::set_hook(Box::new(|_| {}));
	let result = catch_unwind(AssertUnwindSafe(work));
	std::panic::set_hook(hook);
	result
}

fn must_panic(failures: &mut Vec<String>, what: &str, work: impl FnOnce()) {
	if catch_unwind(AssertUnwindSafe(work)).is_ok() {
		failures.push(format!("{} didn't panic", what));
	}
}

fn must_fail<T>(failures: &mut Vec<String>, what: &str, result: Result<T, Runtime>, expected: &str) {
	match result {
		Ok(_) => failures.push(format!("{} succeeded", what)),
		Err(e) if !e.message.contains(expected) => failures.push(format!("{} failed with {:?}, which doesn't say {:?}", what, e.message, expected)),
		Err(_) => {}
	}
}

fn no_hook(_: &Value, _: &Value, _: Vec<Value>) -> DMResult {
	Ok(Value::NULL)
}

fn no_handler(_: *mut raw_types::procs::ExecutionContext) {}

fn on_another_thread(raw_string: raw_types::values::Value) -> Vec<String> {
	let mut failures = vec![];
	let other_thread = "another thread";
	must_fail(&mut failures, "Value::from_string", Value::from_string("auxtest"), other_thread);
	must_fail(&mut failures, "StringRef::new", StringRef::new("auxtest"), other_thread);
	must_fail(&mut failures, "Value::turf", Value::turf(1, 1, 1), other_thread);
	must_fail(
		&mut failures,
		"Value::try_from",
		Value::try_from(&HashMap::<String, f32>::new()),
		other_thread
	);
	must_panic(&mut failures, "List::new", || drop(List::new()));
	must_panic(&mut failures, "Value::world", || drop(Value::world()));
	must_panic(&mut failures, "Value::globals", || drop(Value::globals()));
	must_panic(&mut failures, "Proc::find", || drop(Proc::find("/proc/do_tests")));
	must_panic(&mut failures, "Proc::from_id", || drop(Proc::from_id(ProcId(0))));
	must_panic(&mut failures, "Proc::overrides", || drop(Proc::overrides("/proc/do_tests")));
	must_panic(&mut failures, "byond_string!", || drop(byond_string!("auxtest_thread")));
	must_panic(&mut failures, "shut_down_with_world", shut_down_with_world);
	must_panic(&mut failures, "init_on_next_world_load", || init_on_next_world_load(None));
	must_panic(&mut failures, "instruction_hook::ticks_since_last", || {
		instruction_hook::ticks_since_last();
	});
	if !matches!(
		auxtools::hooks::hook("/proc/auxtest_hooked_global", no_hook),
		Err(HookFailure::NotInitialized)
	) {
		failures.push("hooks::hook didn't fail with NotInitialized".to_owned());
	}
	if instruction_hook::set_handler(no_handler).is_ok() {
		failures.push("instruction_hook::set_handler succeeded".to_owned());
	}
	if world_is_live() {
		failures.push("world_is_live is true".to_owned());
	}

	let printed = [format!("{}", raw_string), format!("{:?}", raw_string)];
	let expected = format!("Value(6, {})", unsafe { raw_string.data.id });
	if printed != [expected.clone(), expected.clone()] {
		failures.push(format!("a raw string value prints as {:?}, expected {:?} both ways", printed, expected));
	}

	let free = catch_unwind(|| {
		let null = Value::NULL;
		let number = Value::from(1.5);
		(
			List::is_list(&null.clone()),
			List::is_list(&number),
			number.clone().as_number().ok(),
			number.to_string().ok()
		)
	});
	let expected = (false, false, Some(1.5), Some("1.5".to_owned()));
	match free {
		Ok(free) if free == expected => {}
		Ok(free) => failures.push(format!("null and 1.5 gave {:?}, expected {:?}", free, expected)),
		Err(_) => failures.push("using null or a number panicked".to_owned())
	}
	failures
}

thread_local!(static KEPT_PROC: RefCell<Option<Proc>> = const { RefCell::new(None) });

#[hook("/proc/auxtest_threads")]
fn test_threads() {
	let string = Value::from_string("auxtest raw")?;
	let raw_string = string.raw();
	let failures = quiet(|| std::thread::spawn(move || on_another_thread(raw_string)).join())
		.ok()
		.and_then(Result::ok)
		.unwrap_or_else(|| vec!["the thread itself panicked".to_owned()]);
	if !failures.is_empty() {
		return Err(runtime!("test_threads: on another thread, {}", failures.join("; ")));
	}
	if format!("{}", string) != "\"auxtest raw\"" {
		return Err(runtime!("test_threads: a string value prints as {}, expected \"auxtest raw\"", string));
	}
	let kept = Proc::find("/proc/auxtest_hooked_global").ok_or_else(|| runtime!("test_threads: /proc/auxtest_hooked_global not defined"))?;
	KEPT_PROC.with(|proc| *proc.borrow_mut() = Some(kept));
	Ok(Value::from(true))
}

fn after_shutdown() -> Vec<String> {
	let mut failures = vec![];
	must_fail(&mut failures, "Value::from_string", Value::from_string("auxtest"), "shut down");
	must_panic(&mut failures, "byond_string!", || drop(byond_string!("auxtest_after_shutdown")));
	must_panic(&mut failures, "Proc::find", || drop(Proc::find("/proc/do_tests")));
	if catch_unwind(shut_down_with_world).is_err() {
		failures.push("shut_down_with_world panicked on BYOND's thread".to_owned());
	}
	match KEPT_PROC.with(|proc| proc.borrow_mut().take()) {
		None => failures.push("no proc was kept from the live world".to_owned()),
		Some(proc) => {
			if !proc.entry().is_null() {
				failures.push(format!("{} has an entry", proc.path));
			}
			if !proc.parameter_names().is_empty() || proc.locals_count() != 0 {
				failures.push(format!("{} has parameters or locals", proc.path));
			}
			must_fail(&mut failures, "Proc::call", proc.call(&[]), "world has ended");
			if !matches!(proc.hook(no_hook), Err(HookFailure::ProcNotFound)) {
				failures.push(format!("hooking {} didn't fail with ProcNotFound", proc.path));
			}
		}
	}
	failures
}

byond_ffi_fn! { auxtest_after_shutdown() {
	let failures = quiet(after_shutdown).unwrap_or_else(|_| vec!["the checks panicked".to_owned()]);
	Some(if failures.is_empty() {
		"ok".to_owned()
	} else {
		format!("after shutdown, {}", failures.join("; "))
	})
} }
