//! Breakpoints on lines

use serde_json::json;

use crate::{ok, world::World};

pub fn run() {
	before_the_world_runs();
	placing();
	hitting();
}

/// The world waits for the client's breakpoints, so ones in the code that
/// runs as it loads are hit
fn before_the_world_runs() {
	let mut world = World::launch(json!({}));
	world.catch_runtimes(false);
	world.break_on(&["early_value", "world_new"]);
	world.request("configurationDone", json!({}));

	world.stop_on("breakpoint", "/early_value", "early_value");
	ok("a breakpoint in the code of a global var's initial value, set before the world runs");
	world.resume();
	world.stop_on("breakpoint", "/world/New", "world_new");
	ok("a breakpoint in world/New");
	world.resume();
	world.end();
}

fn placing() {
	let mut world = World::start();
	let at = world.marks.clone();
	let stepping = at.file("stepping.dm");
	let (leaf_1, leaf_2) = (at.line("leaf_1"), at.line("leaf_2"));

	let set = world.set_breakpoints(&stepping, json!([leaf_2]));
	assert_eq!((&set[0]["verified"], &set[0]["line"]), (&json!(true), &json!(leaf_2)), "{}", set[0]);
	assert!(set[0]["id"].is_number(), "a breakpoint with no id: {}", set[0]);
	ok("a breakpoint on a line with code is verified, there");

	// The proc's declaration, the blank line before it and the comment that
	// starts the file have no code: the next line that has is leaf's first
	let declaration = at.line_starting("stepping.dm", "/proc/leaf(x)");
	let set = world.set_breakpoints(&stepping, json!([declaration, declaration - 1, 1]));
	for breakpoint in &set {
		assert_eq!(
			(&breakpoint["verified"], &breakpoint["line"]),
			(&json!(true), &json!(leaf_1)),
			"{}",
			breakpoint
		);
	}
	ok("a breakpoint on a declaration, a blank line or a comment moves to the next line with code, and says where");

	let set = world.set_breakpoints(&stepping, json!([99_999]));
	assert_eq!(set[0]["verified"], false);
	assert_eq!(set[0]["message"], "no code on or after this line in the world's code");
	let elsewhere = at.dir.join("not_in_the_world.dm");
	let set = world.set_breakpoints(&elsewhere, json!([3]));
	assert_eq!(set[0]["verified"], false);
	assert_eq!(set[0]["message"], "no code on or after this line in the world's code");
	ok("a breakpoint past the end of a file, or in a file the world doesn't have, is unverified, with why");

	let first = world.set_breakpoints(&stepping, json!([leaf_1, leaf_2]));
	let second = world.set_breakpoints(&at.file("errors.dm"), json!([at.line("fail_null")]));
	let mut ids: Vec<u64> = first.iter().chain(&second).map(|b| b["id"].as_u64().unwrap()).collect();
	ids.sort_unstable();
	ids.dedup();
	assert_eq!(ids.len(), 3, "breakpoints share ids");
	ok("each breakpoint has an id of its own");

	let why = world.refused(
		"setBreakpoints",
		json!({ "source": { "path": stepping }, "sourceModified": true, "breakpoints": [{ "line": leaf_1 }] })
	);
	assert_eq!(why, "the file has changes that haven't been saved");
	let why = world.refused("setBreakpoints", json!({ "source": { "name": "stepping.dm" }, "breakpoints": [{ "line": leaf_1 }] }));
	assert_eq!(why, "the breakpoints' source has no path");
	ok("breakpoints for a file with unsaved changes, or with no path, are refused");
	world.end();
}

fn hitting() {
	let mut world = World::start();

	// middle() calls leaf() twice, then returns
	world.break_on(&["leaf_1", "middle_3"]);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.resume();
	world.stop_on("breakpoint", "/middle", "middle_3");
	world.resume_through("middle");
	ok("breakpoints set while the world runs are hit, in the order the code reaches them");

	world.break_on(&["middle_3"]);
	world.run("middle");
	world.stop_on("breakpoint", "/middle", "middle_3");
	ok("a breakpoint left out of the next set is gone, the other stays");

	// Paused: one more, further on
	world.break_on(&["middle_3", "sc_middle_2"]);
	world.resume();
	world.stop_on("breakpoint", "/sc_middle", "sc_middle_2");
	world.resume_through("middle");
	ok("a breakpoint set while the world is paused is hit");

	world.break_on(&[]);
	world.run_through("middle");
	ok("with every breakpoint cleared the world doesn't stop");

	world.break_on(&["spawner_body"]);
	world.run("spawner");
	world.stop_on("breakpoint", "/spawner", "spawner_body");
	assert_eq!(world.local("count"), "0", "the spawned block has its own copy of the proc's locals");
	world.resume_through("spawner");
	ok("a breakpoint in a spawned block");

	world.break_on(&["shout"]);
	world.run("verb");
	world.stop_on("breakpoint", "/mob/player/shout", "shout");
	world.resume_through("verb");
	ok("a breakpoint in a verb");

	world.break_on(&["sub_bump_1", "bump_1"]);
	world.run("parent");
	world.stop_on("breakpoint", "/datum/thing/sub/bump", "sub_bump_1");
	world.resume();
	world.stop_on("breakpoint", "/datum/thing/bump", "bump_1");
	world.resume_through("parent");
	ok("breakpoints in an override and in the proc it calls with ..()");

	// The same line, once for each call deeper
	world.break_on(&["recurse_1"]);
	world.run("recurse");
	for depth in ["3", "2", "1", "0"] {
		world.stop_on("breakpoint", "/recurse", "recurse_1");
		assert_eq!(world.argument("depth"), depth);
		world.resume();
	}
	world.finish("recurse");
	ok("a breakpoint in a proc that calls itself is hit at each depth");
	world.end();
}
