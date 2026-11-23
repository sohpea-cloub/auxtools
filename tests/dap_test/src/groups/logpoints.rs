use std::path::Path;

use serde_json::{json, Value};

use crate::{ok, world::World};

fn log_on(world: &mut World, mark: &str, mut breakpoint: Value) -> Value {
	let (file, line) = world.marks.place(mark);
	breakpoint["line"] = line.into();
	world.set_breakpoints(&file, json!([breakpoint]))[0].clone()
}

fn logged_so_far(world: &World, from: usize) -> Vec<String> {
	world.outputs[from..]
		.iter()
		.filter(|output| output["category"] == "console" && !output["source"].is_null())
		.map(|output| output["output"].as_str().unwrap().to_owned())
		.collect()
}

pub fn logged(world: &mut World, from: usize, count: usize) -> Vec<String> {
	let deadline = std::time::Instant::now() + test_support::dap::TIMEOUT;
	while logged_so_far(world, from).len() < count {
		assert!(std::time::Instant::now() < deadline, "only {:?} was logged", logged_so_far(world, from));
		world.has_event("output");
		std::thread::sleep(std::time::Duration::from_millis(10));
	}
	logged_so_far(world, from)
}

pub fn run() {
	let mut world = World::start();
	let at = world.marks.clone();

	let set = log_on(&mut world, "leaf_1", json!({ "logMessage": "leaf got {x}, twice that is {x * 2}" }));
	assert_eq!(set["verified"], true, "{}", set);
	let from = world.outputs.len();
	world.run_through("middle");
	assert_eq!(logged(&mut world, from, 2), [
		"leaf got 3, twice that is 6\n",
		"leaf got 8, twice that is 16\n"
	]);
	ok("logpoint interpolation without pausing");

	let first = world.outputs[from..].iter().find(|output| !output["source"].is_null()).unwrap();
	assert_eq!(first["line"], at.line("leaf_1"));
	assert_eq!(Path::new(first["source"]["path"].as_str().unwrap()), at.file("stepping.dm"));
	ok("logpoint source location");

	log_on(
		&mut world,
		"bump_1",
		json!({ "logMessage": "{src} is called {name}; by is {by}, n is {n}, items are {src.items}" })
	);
	let from = world.outputs.len();
	world.run_through("parent");
	assert_eq!(logged(&mut world, from, 1), ["S is called S; by is 4, n is 1, items are null\n"]);
	ok("unquoted text and scoped names");

	log_on(&mut world, "leaf_1", json!({ "logMessage": "eight", "condition": "x == 8" }));
	let from = world.outputs.len();
	world.run_through("middle");
	assert_eq!(logged(&mut world, from, 1), ["eight\n"]);
	log_on(&mut world, "recurse_1", json!({ "logMessage": "depth {depth}", "hitCondition": "% 2" }));
	let from = world.outputs.len();
	world.run_through("recurse");
	assert_eq!(logged(&mut world, from, 2), ["depth 2\n", "depth 0\n"]);
	ok("logpoint conditions and hit counts");

	log_on(
		&mut world,
		"leaf_1",
		json!({ "logMessage": "{length(list(1, 2))} {\"}\" + \"{\"} {x}{x} {not closed" })
	);
	let from = world.outputs.len();
	world.run_through("middle");
	assert_eq!(logged(&mut world, from, 2)[0], "2 }{ 33 {not closed\n");
	ok("nested and unmatched message braces");

	log_on(&mut world, "leaf_1", json!({ "logMessage": "{x} has {x.len} and {nowhere_at_all}" }));
	let from = world.outputs.len();
	world.run_through("middle");
	assert_eq!(
		logged(&mut world, from, 2)[0],
		"3 has {x.len: runtime error: Cannot read 3.len} and {nowhere_at_all: undefined var: nowhere_at_all}\n"
	);
	ok("inline evaluation errors");

	let set = log_on(&mut world, "leaf_1", json!({ "logMessage": "x is {x +}" }));
	assert_eq!(set["verified"], false, "{}", set);
	assert!(
		set["message"]
			.as_str()
			.unwrap()
			.starts_with("the message's {x +} doesn't compile: parser error: "),
		"{}",
		set
	);
	world.run_through("middle");
	ok("logpoint compile error rejection");

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
	ok("Step Over ignores logpoint stops");

	let pid = test_support::cli::listed_world("dap_test_host.dmb");
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
	ok("logpoints and breakpoints across sessions");

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
	ok("single log message at a step stop");

	world.end();
}
