use serde_json::json;
use test_support::{cli, process};

use crate::{ok, world::World};

fn rebooted(world: &mut World, worlds: usize) {
	world.wait_count("dap: world/New", worlds);
	world.wait_count("The world restarted", worlds - 1);
}

fn told_after_reboot() {
	let dir = super::reloading::copy_of_the_world("reboots");
	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	let set = world.function_breakpoints(&["/proc/sc_added"]).remove(0);
	assert_eq!(set["verified"], false, "{}", set);
	super::reloading::edit(
		&mut world,
		"stepping.dm",
		"/proc/leaf(x)\n",
		"/proc/sc_added()\n\treturn 1 // @added_1\n\n/proc/leaf(x)\n"
	);

	world.run("reboot");
	rebooted(&mut world, 2);
	let told = world.event("breakpoint")["body"]["breakpoint"].clone();
	let line = world.marks.line("added_1");
	assert_eq!(
		(&told["id"], &told["verified"], &told["line"]),
		(&set["id"], &json!(true), &json!(line)),
		"{}",
		told
	);
	world.run("added");
	world.stop_on("breakpoint", "/sc_added", "added_1");
	world.function_breakpoints(&[]);
	world.resume_through("added");
	ok("breakpoint verification after reboot");
	world.end();
}

pub fn run() {
	let mut a = World::start();
	let pid = cli::listed_world("dap_test_host.dmb");
	let mut b = World::attach(pid);
	b.catch_runtimes(false);

	a.break_on(&["leaf_1"]);
	a.function_breakpoints(&["/proc/sc_middle"]);
	b.break_on(&["middle_3"]);

	a.run("reboot");
	rebooted(&mut a, 2);
	b.wait_output("The world restarted");
	a.catch_runtimes(true);
	a.run("middle");
	for (proc, mark) in [
		("/sc_middle", "sc_middle_1"),
		("/leaf", "leaf_1"),
		("/leaf", "leaf_1"),
		("/middle", "middle_3")
	] {
		a.stop_on("breakpoint", proc, mark);
		b.stop_on("breakpoint", proc, mark);
		a.resume();
		b.event("continued");
	}
	a.finish("middle");
	ok("restore line and function breakpoints");
	ok("reconnect attached sessions");

	a.run("fail_null");
	a.stopped("exception");
	b.stopped("exception");
	a.resume_through("fail_null");
	b.event("continued");
	a.catch_runtimes(false);
	ok("restore runtime error filters");

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
	assert_eq!(a.count("bad instruction"), 0, "a breakpoint was left in the old world's code");
	a.run("middle");
	a.stop_on("breakpoint", "/leaf", "leaf_1");
	b.stop_on("breakpoint", "/leaf", "leaf_1");
	a.resume();
	a.stop_on("breakpoint", "/leaf", "leaf_1");
	a.resume_through("middle");
	ok("reboot from paused evaluation");

	a.run("reboot");
	rebooted(&mut a, 4);
	a.run("reboot");
	rebooted(&mut a, 5);
	a.run("middle");
	a.stop_on("breakpoint", "/leaf", "leaf_1");
	a.break_on(&[]);
	a.resume_through("middle");
	ok("consecutive reboots");

	b.drop_events("continued");
	process::kill_process(pid);
	a.event("terminated");
	a.event("exited");
	b.event("terminated");
	ok("process-kill events reach all sessions");
	a.end();
	b.end();

	let mut a = World::start();
	let pid = cli::listed_world("dap_test_host.dmb");
	let mut b = World::attach(pid);
	a.run("quit");
	a.event("terminated");
	b.event("terminated");
	for world in [&a, &b] {
		assert!(
			!world.output.contains("Waiting to reconnect"),
			"a session took the world's end for a lost connection:\n{}",
			world.output
		);
	}
	ok("world-exit events reach attached sessions");
	a.end();
	b.end();

	told_after_reboot();
}
