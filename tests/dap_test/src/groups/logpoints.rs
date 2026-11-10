//! Breakpoints that say something and don't stop

use std::path::Path;

use serde_json::{json, Value};

use crate::{ok, world::World};

/// Makes the marked line's breakpoint the world's only one, with a message
/// and whatever else
fn log_on(world: &mut World, mark: &str, mut breakpoint: Value) -> Value {
	let (file, line) = world.marks.place(mark);
	breakpoint["line"] = line.into();
	world.set_breakpoints(&file, json!([breakpoint]))[0].clone()
}

/// The messages that have come since `from`, the launcher's own aside
fn logged_so_far(world: &World, from: usize) -> Vec<String> {
	world.outputs[from..]
		.iter()
		.filter(|output| output["category"] == "console" && !output["source"].is_null())
		.map(|output| output["output"].as_str().unwrap().to_owned())
		.collect()
}

/// The messages that came since `from`, once there are `count` of them: they
/// reach the client by another road than what the world prints, and can
/// come after it
fn logged(world: &mut World, from: usize, count: usize) -> Vec<String> {
	let deadline = std::time::Instant::now() + crate::client::TIMEOUT;
	while logged_so_far(world, from).len() < count {
		assert!(std::time::Instant::now() < deadline, "only {:?} was logged", logged_so_far(world, from));
		// Whatever has arrived is taken in
		world.has_event("output");
		std::thread::sleep(std::time::Duration::from_millis(10));
	}
	logged_so_far(world, from)
}

pub fn run() {
	let mut world = World::start();
	let at = world.marks.clone();

	// middle() calls leaf(3), then leaf(8)
	let set = log_on(&mut world, "leaf_1", json!({ "logMessage": "leaf got {x}, twice that is {x * 2}" }));
	assert_eq!(set["verified"], true, "{}", set);
	let from = world.outputs.len();
	world.run_through("middle");
	assert_eq!(logged(&mut world, from, 2), ["leaf got 3, twice that is 6\n", "leaf got 8, twice that is 16\n"]);
	ok("a breakpoint with a message says it at each hit, with its expressions' values, and the world doesn't stop");

	let first = world.outputs[from..].iter().find(|output| !output["source"].is_null()).unwrap();
	assert_eq!(first["line"], at.line("leaf_1"));
	assert_eq!(Path::new(first["source"]["path"].as_str().unwrap()), at.file("stepping.dm"));
	ok("a message comes with the file and line of its breakpoint");

	// A text's value is the text, an object's is its name
	log_on(&mut world, "bump_1", json!({ "logMessage": "{src} is called {name}; by is {by}, n is {n}, items are {src.items}" }));
	let from = world.outputs.len();
	world.run_through("parent");
	assert_eq!(logged(&mut world, from, 1), ["S is called S; by is 4, n is 1, items are null\n"]);
	ok("a text in a message isn't quoted; names mean what they do in the proc");

	log_on(&mut world, "leaf_1", json!({ "logMessage": "eight", "condition": "x == 8" }));
	let from = world.outputs.len();
	world.run_through("middle");
	assert_eq!(logged(&mut world, from, 1), ["eight\n"]);
	log_on(&mut world, "recurse_1", json!({ "logMessage": "depth {depth}", "hitCondition": "% 2" }));
	let from = world.outputs.len();
	world.run_through("recurse");
	assert_eq!(logged(&mut world, from, 2), ["depth 2\n", "depth 0\n"]);
	ok("a message with a condition, and one with a hit count");

	// Braces that pair up, and ones in a text, are the expression's
	log_on(&mut world, "leaf_1", json!({ "logMessage": "{length(list(1, 2))} {\"}\" + \"{\"} {x}{x} {not closed" }));
	let from = world.outputs.len();
	world.run_through("middle");
	assert_eq!(logged(&mut world, from, 2)[0], "2 }{ 33 {not closed\n");
	ok("braces inside an expression are its own, and one never closed is text");

	// An expression that fails when it's hit says so in its place
	log_on(&mut world, "leaf_1", json!({ "logMessage": "{x} has {x.len} and {nowhere_at_all}" }));
	let from = world.outputs.len();
	world.run_through("middle");
	assert_eq!(
		logged(&mut world, from, 2)[0],
		"3 has {x.len: runtime error: Cannot read 3.len} and {nowhere_at_all: undefined var: nowhere_at_all}\n"
	);
	ok("an expression of a message that fails says why in its place");

	let set = log_on(&mut world, "leaf_1", json!({ "logMessage": "x is {x +}" }));
	assert_eq!(set["verified"], false, "{}", set);
	assert!(
		set["message"].as_str().unwrap().starts_with("the message's {x +} doesn't compile: parser error: "),
		"{}",
		set
	);
	world.run_through("middle");
	ok("a message with an expression that doesn't compile is refused");

	// A step goes on past one, which still says its piece
	log_on(&mut world, "leaf_2", json!({ "logMessage": "y is {y}" }));
	world.function_breakpoints(&["/proc/middle"]);
	world.run("middle");
	world.stop_on("breakpoint", "/middle", "middle_1");
	world.function_breakpoints(&[]);
	let from = world.outputs.len();
	world.step_on("next", "/middle", "middle_2");
	assert_eq!(logged(&mut world, from, 1), ["y is 4\n"]);
	world.resume_through("middle");
	assert_eq!(logged(&mut world, from, 2)[1], "y is 9\n");
	ok("a step over a call goes past a breakpoint with a message in it");

	// The same line for two sessions: one stops there, the other is told
	let pid = crate::client::listed_world("dap_test_host.dmb");
	let mut other = World::attach(pid);
	other.catch_runtimes(false);
	other.break_on(&["leaf_2"]);
	let from = world.outputs.len();
	world.run("middle");
	for y in ["4", "9"] {
		other.stop_on("breakpoint", "/leaf", "leaf_2");
		world.stop_on("breakpoint", "/leaf", "leaf_2");
		assert_eq!(other.local("y"), y);
		other.resume();
	}
	world.finish("middle");
	assert_eq!(logged(&mut world, from, 2), ["y is 4\n", "y is 9\n"]);
	assert_eq!(logged_so_far(&other, 0), Vec::<String>::new());
	other.end();
	ok("a breakpoint with a message for one session and a plain one for another, on one line: it stops, and the first is told");

	// A step that stops on one: it says its piece there, and the world stops
	// for the step
	log_on(&mut world, "recurse_1", json!({ "logMessage": "depth {depth}" }));
	world.function_breakpoints(&["/proc/sc_recurse"]);
	world.run("recurse");
	world.stopped("breakpoint");
	world.function_breakpoints(&[]);
	let from = world.outputs.len();
	world.step_on("stepIn", "/recurse", "recurse_1");
	assert_eq!(logged(&mut world, from, 1), ["depth 3\n"]);
	world.resume_through("recurse");
	assert_eq!(logged(&mut world, from, 4), ["depth 3\n", "depth 2\n", "depth 1\n", "depth 0\n"]);
	ok("a step that stops on a breakpoint with a message says it, once");

	world.end();
}
