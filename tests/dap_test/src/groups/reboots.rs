//! The world rebooting, and ending, under the debugger

use serde_json::json;

use crate::{client, ok, world::World};

/// Waits for the world after a reboot to be running
fn rebooted(world: &mut World, worlds: usize) {
	world.wait_count("dap: world/New", worlds);
	world.wait_count("The world restarted", worlds - 1);
}

pub fn run() {
	let mut a = World::start();
	let pid = client::listed_world("dap_test_host.dmb");
	let mut b = World::attach(pid);
	b.catch_runtimes(false);

	a.break_on(&["leaf_1"]);
	a.function_breakpoints(&["/proc/sc_middle"]);
	b.break_on(&["middle_3"]);

	// The scenario calls world.Reboot(): the world is loaded again in the
	// same process, and each launcher connects to it again
	a.run("reboot");
	rebooted(&mut a, 2);
	b.wait_output("The world restarted");
	a.catch_runtimes(true);
	a.run("middle");
	for (proc, mark) in [("/sc_middle", "sc_middle_1"), ("/leaf", "leaf_1"), ("/leaf", "leaf_1"), ("/middle", "middle_3")] {
		a.stop_on("breakpoint", proc, mark);
		b.stop_on("breakpoint", proc, mark);
		a.resume();
		b.event("continued");
	}
	a.finish("middle");
	ok("after a reboot the launcher's breakpoints, on lines and on procs, are set in the new world");
	ok("a session that attached is connected to the new world too, with its breakpoints");

	a.run("fail_null");
	a.stopped("exception");
	b.stopped("exception");
	a.resume_through("fail_null");
	b.event("continued");
	a.catch_runtimes(false);
	ok("after a reboot runtime errors are caught as before");

	// An expression reboots the world it's paused in
	a.function_breakpoints(&[]);
	b.break_on(&[]);
	a.run("middle");
	a.stop_on("breakpoint", "/leaf", "leaf_1");
	b.stop_on("breakpoint", "/leaf", "leaf_1");
	let frame = a.frame_id();
	let why = a.refused("evaluate", json!({ "expression": "world.Reboot()", "frameId": frame, "context": "repl" }));
	assert_eq!(why, "the world disconnected");
	a.event("continued");
	b.event("continued");
	rebooted(&mut a, 3);
	a.run("middle");
	a.stop_on("breakpoint", "/leaf", "leaf_1");
	b.stop_on("breakpoint", "/leaf", "leaf_1");
	a.resume();
	a.stop_on("breakpoint", "/leaf", "leaf_1");
	a.resume_through("middle");
	ok("an expression that reboots the world while it's paused: the sessions hear it carry on, and debug the next world");

	a.run("reboot");
	rebooted(&mut a, 4);
	a.run("reboot");
	rebooted(&mut a, 5);
	a.run("middle");
	a.stop_on("breakpoint", "/leaf", "leaf_1");
	a.break_on(&[]);
	a.resume_through("middle");
	ok("two reboots in a row");

	// The process is killed
	b.drop_events("continued");
	client::kill_process(pid);
	a.event("terminated");
	a.event("exited");
	b.event("terminated");
	ok("a world whose process is killed: both sessions are told it's over");
	a.end();
	b.end();

	let mut a = World::start();
	let pid = client::listed_world("dap_test_host.dmb");
	let mut b = World::attach(pid);
	a.run("quit");
	a.event("terminated");
	b.event("terminated");
	ok("a world that ends itself: the session that attached is told too");
	a.end();
	b.end();
}
