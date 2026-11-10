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

	hit_counts(&mut world);
	world.end();
}

/// Sets the one breakpoint, on the first line of recurse(), with a hit
/// condition and maybe a condition, runs recurse(3), which reaches it four
/// times (at depths 3, 2, 1 and 0), and returns the depths it stopped at
fn stops_with(world: &mut World, hit: &str, condition: Option<&str>) -> Vec<String> {
	let (file, line) = world.marks.place("recurse_1");
	let set = world.set_breakpoints(&file, json!([{ "line": line, "hitCondition": hit, "condition": condition }]));
	assert_eq!(set[0]["verified"], true, "{}", set[0]);
	stops_of(world)
}

/// Runs recurse(3) and returns the depths it stopped at
fn stops_of(world: &mut World) -> Vec<String> {
	world.run_collecting("recurse", |world| world.argument("depth"))
}

/// Lets the world carry on from a stop in `scenario`, which had finished
/// `finished` times when it was run, to its end, and returns the depths it
/// stopped at on the way
fn resume_collecting(world: &mut World, scenario: &str, finished: usize) -> Vec<String> {
	let over = format!("dap: finished {}\n", scenario);
	let deadline = std::time::Instant::now() + crate::client::TIMEOUT;
	let mut seen = vec![];
	world.resume();
	loop {
		if world.event_within("stopped", std::time::Duration::from_millis(20)).is_some() {
			seen.push(world.argument("depth"));
			world.resume();
		} else if world.count(&over) > finished {
			return seen;
		}
		assert!(std::time::Instant::now() < deadline, "{} never finished", scenario);
	}
}

fn hit_counts(world: &mut World) {
	assert_eq!(stops_with(world, "3", None), ["1", "0"]);
	assert_eq!(stops_with(world, ">= 2", None), ["2", "1", "0"]);
	ok("a hit count stops from that hit on");

	assert_eq!(stops_with(world, "== 2", None), ["2"]);
	assert_eq!(stops_with(world, "=4", None), ["0"]);
	assert_eq!(stops_with(world, "> 3", None), ["0"]);
	assert_eq!(stops_with(world, "< 3", None), ["3", "2"]);
	assert_eq!(stops_with(world, "<= 1", None), ["3"]);
	assert_eq!(stops_with(world, "% 2", None), ["2", "0"]);
	assert_eq!(stops_with(world, "9", None), Vec::<String>::new());
	ok("a hit count with ==, >, <, <= and %");

	// Only the hits where the condition holds are counted: depths 3, 1, 0
	assert_eq!(stops_with(world, "2", Some("depth != 2")), ["1", "0"]);
	assert_eq!(stops_with(world, "% 2", Some("depth < 3")), ["1"]);
	ok("a hit count with a condition counts the hits where the condition holds");

	// A step that stops on it is a hit too: depth 3's, so 2 and 0 stop
	let (file, line) = world.marks.place("recurse_1");
	world.set_breakpoints(&file, json!([{ "line": line, "hitCondition": "% 2" }]));
	world.function_breakpoints(&["/proc/sc_recurse"]);
	let finished = world.count("dap: finished recurse\n");
	world.run("recurse");
	world.stopped("breakpoint");
	world.function_breakpoints(&[]);
	world.step_on("stepIn", "/recurse", "recurse_1");
	assert_eq!(world.argument("depth"), "3");
	assert_eq!(resume_collecting(world, "recurse", finished), ["2", "0"]);
	ok("a step that stops on a breakpoint counts as its hit, and doesn't stop twice");

	// The count goes on until the breakpoint is set again
	assert_eq!(stops_with(world, "== 6", None), Vec::<String>::new());
	assert_eq!(stops_of(world), ["2"]);
	assert_eq!(stops_of(world), Vec::<String>::new());
	ok("hits are counted from when the breakpoint is set, across runs");

	let (file, line) = world.marks.place("recurse_1");
	for (hit, why) in [("soon", "\"soon\" isn't a hit count"), ("% 0", "every 0 hits is never"), ("== two", "\"== two\" isn't a hit count")] {
		let set = world.set_breakpoints(&file, json!([{ "line": line, "hitCondition": hit }]));
		assert_eq!(set[0]["verified"], false, "{}", set[0]);
		assert!(set[0]["message"].as_str().unwrap().starts_with(why), "{}", set[0]);
	}
	world.set_breakpoints(&file, json!([]));
	ok("a hit count that isn't one is refused, with what one looks like");

	// leaf(3), then leaf(8)
	let set = world.request(
		"setFunctionBreakpoints",
		json!({ "breakpoints": [{ "name": "/proc/leaf", "hitCondition": "2" }] })
	);
	assert_eq!(set["breakpoints"][0]["verified"], true);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	assert_eq!(world.argument("x"), "8");
	world.resume_through("middle");
	world.function_breakpoints(&[]);
	ok("a function breakpoint with a hit count");
}
