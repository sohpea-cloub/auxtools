//! Breakpoints with conditions

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

	world.break_if(&[("bump_1", "src.n == 1 && global_number == 5")]);
	world.run("parent");
	world.stop_on("breakpoint", "/datum/thing/bump", "bump_1");
	world.resume_through("parent");
	world.break_if(&[("bump_1", "src.n == 2 || global_number == 6")]);
	world.run_through("parent");
	ok("a condition on a var of src and a global");

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
	ok("a condition that runtimes stops the world");

	world.break_on(&["leaf_1"]);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.resume_through("middle");
	ok("a breakpoint set again without its condition stops every time");
	world.end();
}
