//! Breakpoints with conditions

use serde_json::json;

use crate::{ok, world::World};

pub fn run() {
	let mut world = World::start();

	// middle() calls leaf(3), then leaf(8)
	world.break_if(&[("leaf_1", "x == 8")]);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	assert_eq!(world.argument("x"), "8");
	world.resume_through("middle");
	ok("a condition on an argument: stops when it's true, not when it's false");

	world.break_if(&[("leaf_1", "x == 999")]);
	world.run_through("middle");
	ok("a condition that's never true never stops");

	world.break_if(&[("leaf_2", "y == 4")]);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	assert_eq!(world.local("y"), "4");
	world.resume_through("middle");
	ok("a condition on a local");

	world.break_if(&[("bump_1", "n == 1 && global_number == 5")]);
	world.run("parent");
	world.stop_on("breakpoint", "/datum/thing/bump", "bump_1");
	world.resume_through("parent");
	world.break_if(&[("bump_1", "src.n == 2 || global.global_number == 6")]);
	world.run_through("parent");
	ok("a condition on a var of src, by its bare name and with src., and a global");

	// The condition calls a proc with a breakpoint of its own in it, which
	// doesn't stop there
	world.break_if(&[("leaf_2", ""), ("middle_3", "leaf(1) == 4")]);
	world.run("middle");
	for _ in 0..2 {
		world.stop_on("breakpoint", "/leaf", "leaf_2");
		assert_eq!(world.frames(0).len(), 4, "stopped in the condition's own call");
		world.resume();
	}
	world.stop_on("breakpoint", "/middle", "middle_3");
	world.resume_through("middle");
	ok("a condition that calls a proc with a breakpoint in it");

	world.break_if(&[("leaf_1", "global_number++ && 0")]);
	world.run_through("middle");
	assert_eq!(world.eval_global("global_number"), "7");
	world.eval_global("global_number = 5");
	ok("a condition's side effects happen once a hit");

	// The condition can't be worked out: better a stop too many than one
	// missed
	world.break_if(&[("leaf_1", "x.len > 3")]);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.resume_through("middle");
	// x is 3, then 8
	for x in [3, 8] {
		let said = format!(
			"[auxtools] A breakpoint's condition (x.len > 3) failed, so the world pauses there: runtime error: Cannot read {}.len",
			x
		);
		assert_eq!(world.count(&said), 1, "the console didn't say {:?}", said);
	}
	ok("a condition that runtimes stops the world, and the console says why");

	// One that doesn't compile isn't set, the others in the file are
	let set = world.try_break_if(&[("leaf_1", "x =="), ("leaf_2", "y == 4"), ("middle_3", "nowhere_at_all == 1")]);
	assert_eq!(set[0]["verified"], false, "{}", set[0]);
	let why = set[0]["message"].as_str().unwrap();
	assert!(why.starts_with("the condition doesn't compile: parser error: "), "{}", why);
	assert_eq!((&set[1]["verified"], &set[2]["verified"]), (&json!(true), &json!(true)));
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	world.resume();
	// A name the world doesn't have is only found out when it's hit
	world.stop_on("breakpoint", "/middle", "middle_3");
	world.resume_through("middle");
	assert_eq!(world.count("failed, so the world pauses there: undefined var: nowhere_at_all"), 1);
	let set = world.request(
		"setFunctionBreakpoints",
		json!({ "breakpoints": [{ "name": "/proc/leaf", "condition": "x x" }, { "name": "/proc/middle" }] })
	);
	assert_eq!(set["breakpoints"][0]["verified"], false);
	assert!(set["breakpoints"][0]["message"].as_str().unwrap().starts_with("the condition doesn't compile: "));
	assert_eq!(set["breakpoints"][1]["verified"], true);
	world.function_breakpoints(&[]);
	ok("a condition that doesn't compile is refused when it's set, with the parser's words, on a line or on a proc");

	world.break_on(&["leaf_1"]);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.resume_through("middle");
	ok("a breakpoint set again without its condition stops every time");
	world.end();
}
