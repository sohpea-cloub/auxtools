//! Several launchers debugging one world at once

use serde_json::json;

use crate::{client, ok, world::World};

pub fn run() {
	let mut a = World::start();
	let pid = client::listed_world("dap_test_host.dmb");
	let mut b = World::attach(pid);
	b.catch_runtimes(false);

	// One's breakpoint stops the world for both
	a.break_on(&["leaf_1"]);
	a.run("middle");
	a.stop_on("breakpoint", "/leaf", "leaf_1");
	b.stop_on("breakpoint", "/leaf", "leaf_1");
	ok("a breakpoint of one session stops the world for both, in the same place");

	assert_eq!(b.eval("x"), "3");
	assert_eq!(b.eval("x = 4"), "4");
	assert_eq!(a.argument("x"), "4");
	ok("the other session looks around in the stop, and what it changes the first sees");

	b.resume();
	a.event("continued");
	a.stop_on("breakpoint", "/leaf", "leaf_1");
	b.stop_on("breakpoint", "/leaf", "leaf_1");
	a.resume();
	b.event("continued");
	a.finish("middle");
	ok("either session lets the world carry on, and the other hears it has");

	// Each has its own, and setting one's leaves the other's alone
	b.break_on(&["middle_3"]);
	a.run("middle");
	for (proc, mark) in [("/leaf", "leaf_1"), ("/leaf", "leaf_1"), ("/middle", "middle_3")] {
		a.stop_on("breakpoint", proc, mark);
		b.stop_on("breakpoint", proc, mark);
		a.resume();
		b.event("continued");
	}
	a.finish("middle");
	a.break_on(&[]);
	a.run("middle");
	a.stop_on("breakpoint", "/middle", "middle_3");
	b.stop_on("breakpoint", "/middle", "middle_3");
	b.resume();
	a.event("continued");
	a.finish("middle");
	b.break_on(&[]);
	a.run_through("middle");
	ok("each session has its own breakpoints: clearing one's leaves the other's");

	// Both on one line, each with a condition: leaf(3), then leaf(8)
	a.break_if(&[("leaf_1", "x == 3")]);
	b.break_if(&[("leaf_1", "x == 8")]);
	a.run("middle");
	for x in ["3", "8"] {
		a.stop_on("breakpoint", "/leaf", "leaf_1");
		b.stop_on("breakpoint", "/leaf", "leaf_1");
		assert_eq!(a.argument("x"), x);
		a.resume();
		b.event("continued");
	}
	a.finish("middle");
	b.break_on(&[]);
	a.run("middle");
	a.stop_on("breakpoint", "/leaf", "leaf_1");
	b.stop_on("breakpoint", "/leaf", "leaf_1");
	assert_eq!(a.argument("x"), "3");
	a.resume();
	a.finish("middle");
	assert!(!a.has_event("stopped"), "the cleared session's condition still stops the world");
	b.event("continued");
	ok("two sessions' breakpoints on one line, each with its condition: either stops the world");

	// One steps, the other follows
	a.break_on(&["middle_1"]);
	a.run("middle");
	a.stop_on("breakpoint", "/middle", "middle_1");
	b.stop_on("breakpoint", "/middle", "middle_1");
	a.break_on(&[]);
	a.step_on("stepIn", "/leaf", "leaf_1");
	b.event("continued");
	b.stop_on("step", "/leaf", "leaf_1");
	ok("one session's step is seen by the other: the world carries on, then stops after a step");

	// A third arrives in the middle of it
	let mut c = World::attach(pid);
	c.stop_on("step", "/leaf", "leaf_1");
	assert_eq!(c.argument("x"), "3");
	c.resume();
	a.event("continued");
	b.event("continued");
	a.finish("middle");
	ok("a session that attaches to a paused world is told it's paused, and why");

	// Whether to pause on runtime errors is each session's own say, but a
	// pause is everyone's
	a.catch_runtimes(true);
	a.run("fail_null");
	for session in [&mut a, &mut b, &mut c] {
		let stopped = session.stopped("exception");
		assert_eq!(stopped["body"]["description"], "cannot read from list");
	}
	b.resume();
	a.finish("fail_null");
	a.catch_runtimes(false);
	a.run_through("fail_null");
	ok("a runtime error stops the world when any session wants it to, for all of them");

	// A session that leaves takes its breakpoints with it
	b.break_on(&["leaf_1"]);
	c.break_on(&["middle_3"]);
	b.end();
	a.run("middle");
	a.stop_on("breakpoint", "/middle", "middle_3");
	c.stop_on("breakpoint", "/middle", "middle_3");
	a.resume_through("middle");
	ok("a session that disconnects takes its breakpoints with it, and leaves the world running");

	// So does one whose launcher dies
	c.launcher.child.kill().unwrap();
	c.launcher.child.wait().unwrap();
	for _ in 0..3 {
		a.eval_global("ticks");
	}
	a.run_through("middle");
	ok("a session whose launcher is killed loses its breakpoints too");
	a.end();

	last_one_leaves();
}

/// A world run without debugging, and one session that debugs it for a while
fn last_one_leaves() {
	let mut runner = World::launch(json!({ "noDebug": true }));
	runner.configure();
	let pid = client::listed_world("dap_test_host.dmb");

	let mut session = World::attach(pid);
	session.request("pause", json!({ "threadId": 0 }));
	session.stopped("pause");
	assert!(!runner.has_event("stopped"), "a launcher that isn't debugging passed a stop on");
	session.end();
	runner.run_through("plain");
	ok("the only debugging session leaves a paused world, which carries on");

	let mut session = World::attach(pid);
	session.break_on(&["plain_1"]);
	runner.run("plain");
	session.stop_on("breakpoint", "/sc_plain", "plain_1");
	session.resume();
	runner.finish("plain");
	session.end();
	runner.run_through("plain");
	ok("a world run without debugging is debugged by a session that attaches, and runs on when it's gone");
	runner.end();
}
