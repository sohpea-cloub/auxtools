use serde_json::json;

use crate::{client, ok, world::World};

pub fn run() {
	let mut a = World::start();
	let pid = client::listed_world("dap_test_host.dmb");
	let mut b = World::attach(pid);
	b.catch_runtimes(false);

	a.break_on(&["leaf_1"]);
	a.run("middle");
	a.stop_on("breakpoint", "/leaf", "leaf_1");
	b.stop_on("breakpoint", "/leaf", "leaf_1");
	ok("breakpoint pause reaches both sessions");

	assert_eq!(b.eval("x"), "3");
	assert_eq!(b.eval("x = 4"), "4");
	assert_eq!(a.argument("x"), "4");
	ok("shared inspection and assignments");

	b.resume();
	a.event("continued");
	a.stop_on("breakpoint", "/leaf", "leaf_1");
	b.stop_on("breakpoint", "/leaf", "leaf_1");
	a.resume();
	b.event("continued");
	a.finish("middle");
	ok("resume events reach other sessions");

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
	ok("independent session breakpoints");

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
	ok("independent conditions at one location");

	a.break_on(&["middle_1"]);
	a.run("middle");
	a.stop_on("breakpoint", "/middle", "middle_1");
	b.stop_on("breakpoint", "/middle", "middle_1");
	a.break_on(&[]);
	a.step_on("stepIn", "/leaf", "leaf_1");
	b.event("continued");
	b.stop_on("step", "/leaf", "leaf_1");
	ok("step events reach both sessions");

	let mut c = World::attach(pid);
	c.catch_runtimes(false);
	c.stop_on("step", "/leaf", "leaf_1");
	assert_eq!(c.argument("x"), "3");
	c.resume();
	a.event("continued");
	b.event("continued");
	a.finish("middle");
	ok("attach to an existing pause");

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
	ok("any session can request runtime stops");

	b.break_on(&["leaf_1"]);
	c.break_on(&["middle_3"]);
	a.run("middle");
	for session in [&mut a, &mut b, &mut c] {
		session.stop_on("breakpoint", "/leaf", "leaf_1");
	}
	a.drop_events("continued");
	c.drop_events("continued");
	b.end();
	assert_eq!(c.eval("x"), "3");
	assert_eq!(a.frames(0).len(), 4);
	assert!(
		!a.has_event("continued") && !c.has_event("continued"),
		"the world carried on when one session left"
	);
	ok("remaining debugger retains pause");

	a.resume();
	c.event("continued");
	a.stop_on("breakpoint", "/middle", "middle_3");
	c.stop_on("breakpoint", "/middle", "middle_3");
	a.resume_through("middle");
	ok("disconnect removes owned breakpoints");

	c.launcher.child.kill().unwrap();
	c.launcher.child.wait().unwrap();
	for _ in 0..3 {
		a.eval_global("ticks");
	}
	a.run_through("middle");
	ok("killed launcher loses breakpoints");
	a.end();

	last_one_leaves();
}

fn last_one_leaves() {
	let mut runner = World::launch(json!({ "noDebug": true }));
	runner.configure();
	let pid = client::listed_world("dap_test_host.dmb");

	let mut session = World::attach(pid);
	session.request("pause", json!({ "threadId": 0 }));
	session.stopped("pause");
	assert!(!runner.has_event("stopped"), "noDebug session received a stop event");
	session.end();
	runner.run_through("plain");
	ok("last debugger disconnect resumes");

	let mut session = World::attach(pid);
	session.request("pause", json!({ "threadId": 0 }));
	session.stopped("pause");
	session.launcher.child.kill().unwrap();
	session.launcher.child.wait().unwrap();
	runner.run_through("plain");
	ok("last debugger death resumes");

	let mut first = World::attach(pid);
	let mut second = World::attach(pid);
	first.request("pause", json!({ "threadId": 0 }));
	first.stopped("pause");
	second.stopped("pause");
	second.end();
	assert!(!first.frames(0).is_empty());
	assert!(!first.has_event("continued"), "world resumed while a debugging session remained");
	first.end();
	runner.run_through("plain");
	ok("pause retained until all debuggers leave");

	let mut session = World::attach(pid);
	session.break_on(&["plain_1"]);
	runner.run("plain");
	session.stop_on("breakpoint", "/sc_plain", "plain_1");
	session.resume();
	runner.finish("plain");
	session.end();
	runner.run_through("plain");
	ok("attach to a noDebug launch");
	runner.end();
}
