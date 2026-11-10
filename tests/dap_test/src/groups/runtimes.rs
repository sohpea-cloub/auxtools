//! Runtime errors

use serde_json::json;

use crate::{
	ok,
	world::{host_file, World}
};

/// Waits for the world to stop on a runtime error, and checks its message
/// and where it is
fn stopped_on(world: &mut World, message: &str, proc: &str, mark: &str) {
	let event = world.event("stopped");
	assert_eq!(event["body"]["reason"], "exception", "{}", event);
	assert_eq!(event["body"]["description"], message, "{}", event);
	let info = world.request("exceptionInfo", json!({ "threadId": 0 }));
	assert_eq!(info["exceptionId"], message, "{}", info);
	assert_eq!(info["breakMode"], "always", "{}", info);
	let frame = world.top_frame();
	assert_eq!(
		(frame["name"].as_str().unwrap(), frame["line"].as_u64().unwrap()),
		(proc, world.marks.line(mark)),
		"the runtime error's frame"
	);
}

pub fn run() {
	while_loading();

	let mut world = World::launch(json!({}));
	world.catch_runtimes(true);
	world.configure();

	world.run("fail_null");
	stopped_on(&mut world, "cannot read from list", "/fail_null", "fail_null");
	ok("a runtime error stops the world, with its message, on its line");

	let names: Vec<String> = world.frames(0).iter().map(|f| f["name"].as_str().unwrap().to_owned()).collect();
	assert_eq!(names[..3], ["/fail_null", "/fail_deep", "/sc_fail_null"]);
	assert_eq!(world.local("L"), "null");
	assert_eq!(world.eval("isnull(L)"), "1");
	ok("at a runtime error the stack, the locals and expressions are there");

	// The proc that failed is over, and so is its caller's line; the scenario
	// carries on
	let after = world.count("dap: after the failure");
	world.resume_through("fail_null");
	assert_eq!(world.count("dap: after the failure"), after + 1);
	assert!(world.output.contains("runtime error: cannot read from list"), "the world didn't log the error");
	ok("after a runtime error the world carries on, and has logged it");

	for (scenario, message, proc, mark) in [
		("fail_index", "list index out of bounds", "/fail_index", "fail_index"),
		("fail_divide", "Division by zero", "/fail_divide", "fail_divide"),
		("fail_crash", "crashed on purpose", "/fail_crash", "fail_crash")
	] {
		world.run(scenario);
		stopped_on(&mut world, message, proc, mark);
		world.resume_through(scenario);
	}
	ok("a bad index, a division by zero and a CRASH()");

	world.run("fail_spawned");
	stopped_on(&mut world, "cannot read from list", "/fail_null", "fail_null");
	let names: Vec<String> = world.frames(0).iter().map(|f| f["name"].as_str().unwrap().to_owned()).collect();
	assert_eq!(names, ["/fail_null", "/sc_fail_spawned"]);
	world.resume_through("fail_spawned");
	ok("a runtime error in a spawned block");

	// The scenario divides by zero; the block it spawned fails after
	world.run("fail_twice");
	stopped_on(&mut world, "Division by zero", "/fail_divide", "fail_divide");
	world.resume();
	stopped_on(&mut world, "list index out of bounds", "/fail_index", "fail_index");
	world.resume_through("fail_twice");
	ok("two runtime errors, one after the other");

	// An expression's own runtime error is the expression's
	world.run_to("middle", "/leaf", "leaf_1");
	let frame = world.frame_id();
	let response = world.try_request("evaluate", json!({ "expression": "fail_null()", "frameId": frame, "context": "watch" }));
	assert!(response["type"] == "response", "{}", response);
	assert!(!world.has_event("stopped"), "an expression's runtime error stopped the world again");
	assert_eq!(world.eval("x"), "3");
	world.resume_through("middle");
	ok("a runtime error in an expression doesn't stop the world again");

	world.catch_runtimes(false);
	let logged = world.count("runtime error: cannot read from list");
	world.run_through("fail_null");
	assert_eq!(world.count("runtime error: cannot read from list"), logged + 1);
	ok("with the filter off a runtime error doesn't stop the world, which logs it");

	world.catch_runtimes(true);
	world.run("fail_divide");
	stopped_on(&mut world, "Division by zero", "/fail_divide", "fail_divide");
	world.resume_through("fail_divide");
	ok("with the filter on again it stops");
	world.end();
}

/// A global var's initial value fails while the world loads
fn while_loading() {
	let flag = host_file("fail_early");
	std::fs::write(&flag, "yes").unwrap();
	let mut world = World::launch(json!({}));
	world.catch_runtimes(true);
	world.request("configurationDone", json!({}));
	stopped_on(&mut world, "cannot read from list", "/early_value", "early_failure");
	std::fs::remove_file(&flag).unwrap();
	world.resume();
	world.wait_output("dap: world/New");
	world.run_through("plain");
	ok("a runtime error while the world loads");
	world.end();
}
