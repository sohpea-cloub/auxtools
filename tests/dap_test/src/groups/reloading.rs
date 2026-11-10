//! Debugging a world while its code is hot reloaded

use std::path::PathBuf;

use serde_json::json;

use crate::{
	client::value_of,
	ok,
	world::{host_file, World}
};

/// A copy of the world's code, to edit
fn copy_of_the_world() -> PathBuf {
	let dir = std::env::temp_dir().join(format!("dap_test_reloading-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	for entry in std::fs::read_dir(host_file("")).unwrap() {
		let path = entry.unwrap().path();
		if path.extension().is_some_and(|e| e == "dm" || e == "dme" || e == "dmb" || e == "rsc") {
			std::fs::copy(&path, dir.join(path.file_name().unwrap())).unwrap();
		}
	}
	dir
}

/// How many reloads the launcher has said were done
fn reloads(output: &str) -> usize {
	output
		.lines()
		.filter(|line| line.starts_with("Hot reload: ") && line.ends_with(" ms"))
		.count()
}

/// Changes a file, and waits for the world to have the change
fn edit(world: &mut World, file: &str, old: &str, new: &str) {
	let before = reloads(&world.output);
	world.marks.edit(file, old, new);
	world.wait_until("the hot reload", |output| reloads(output) > before);
	// The launcher has told the client where its breakpoints are now
	world.drop_events("breakpoint");
}

pub fn run() {
	let dir = copy_of_the_world();
	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload: watching");

	// Three lines more above the breakpoint: the client moves its mark with
	// the text, and says where its breakpoints are now
	world.break_on(&["leaf_2"]);
	let before = world.marks.line("leaf_2");
	edit(&mut world, "stepping.dm", "/proc/leaf(x)\n", "/proc/above()\n\treturn 1\n\n/proc/leaf(x)\n");
	assert_eq!(world.marks.line("leaf_2"), before + 3);
	world.break_on(&["leaf_2"]);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	assert_eq!(world.local("y"), "4");
	ok("a breakpoint the client moved with its line, after a reload that moved the code, stops on the same statement");

	// Paused in leaf(), whose code changes under it
	edit(
		&mut world,
		"stepping.dm",
		"\tvar/y = x + 1 // @leaf_1\n",
		"\tvar/y = x + 1 // @leaf_1\n\tvar/extra = 5 // @leaf_extra\n\ty += extra\n"
	);
	let frame = world.top_frame();
	assert_eq!(frame["name"], "/leaf");
	let locals = world.scope_values("Locals");
	assert_eq!(locals.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), [".", "y"]);
	ok("a reload while the world is paused is taken, and the paused frame keeps the code and the locals it started with");

	// middle()'s r is what the old leaf() gives for 3
	world.break_on(&[]);
	assert_eq!(world.step("stepOut").proc, "/middle");
	assert_eq!(world.step("next").proc, "/middle");
	assert_eq!(world.local("r"), "8");
	ok("the frame that was paused finishes on its old code");

	world.step_on("stepIn", "/leaf", "leaf_1");
	world.step_on("next", "/leaf", "leaf_extra");
	world.step("next");
	world.step_on("next", "/leaf", "leaf_2");
	let locals = world.scope_values("Locals");
	assert_eq!((value_of(&locals, "y"), value_of(&locals, "extra")), ("14", "5"));
	world.resume_through("middle");
	ok("the next call runs the new code: stepped into, with its new local");

	edit(
		&mut world,
		"stepping.dm",
		"/proc/sc_middle()\n",
		"/proc/sc_fresh()\n\tvar/a = 1 // @fresh_1\n\tvar/b = a + 1 // @fresh_2\n\treturn b // @fresh_3\n\n/proc/sc_middle()\n"
	);
	world.run_to("fresh", "/sc_fresh", "fresh_2");
	assert_eq!(world.local("a"), "1");
	world.step_on("next", "/sc_fresh", "fresh_3");
	assert_eq!(world.local("b"), "2");
	world.resume_through("fresh");
	ok("a breakpoint in a proc that a reload added, and a step in it");

	// The proc's code changes, and then its first line too
	let set = world.function_breakpoints(&["/proc/leaf"]);
	assert_eq!(set[0]["verified"], true);
	edit(&mut world, "stepping.dm", "\tvar/extra = 5 // @leaf_extra\n", "\tvar/extra = 6 // @leaf_extra\n");
	world.run("middle");
	for _ in 0..2 {
		world.stop_on("breakpoint", "/leaf", "leaf_1");
		world.resume();
	}
	world.finish("middle");
	edit(&mut world, "stepping.dm", "/proc/leaf(x)\n", "/proc/leaf(x)\n\tvar/first = 1 // @leaf_first\n\tfirst++\n");
	world.run("middle");
	for _ in 0..2 {
		world.stop_on("breakpoint", "/leaf", "leaf_first");
		world.resume();
	}
	world.finish("middle");
	world.function_breakpoints(&[]);
	world.run_through("middle");
	ok("a function breakpoint stays on a proc whose code is reloaded, on whatever its first line is now");
	edit(
		&mut world,
		"stepping.dm",
		"\tvar/y = x + 1 // @leaf_1\n\tvar/extra = 6 // @leaf_extra\n\ty += extra\n\treturn y * 2 // @leaf_2\n",
		"\tvar/renamed = x + 1 // @leaf_1\n\tvar/extra = 6 // @leaf_extra\n\trenamed += extra\n\treturn renamed * 2 // @leaf_2\n"
	);
	world.run_to("middle", "/leaf", "leaf_2");
	let locals = world.scope_values("Locals");
	assert_eq!(locals.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), [".", "first", "renamed", "extra"]);
	assert_eq!(world.eval("renamed + extra"), "16");
	world.resume_through("middle");
	ok("a local a reload renamed is shown, and evaluated, by its new name");

	// The runtime error is a line further down now
	world.catch_runtimes(true);
	edit(&mut world, "errors.dm", "\tvar/list/L = null\n\treturn L[1] // @fail_null\n", "\tvar/list/L = null\n\tvar/unused = 1\n\treturn L[1] // @fail_null\n");
	world.run("fail_null");
	let stop = world.stop();
	assert_eq!(
		(stop.reason.as_str(), stop.proc.as_str(), stop.line),
		("exception", "/fail_null", world.marks.line("fail_null"))
	);
	world.resume_through("fail_null");
	world.catch_runtimes(false);
	ok("a runtime error in reloaded code stops on its new line");

	// An edit the world can't take: nothing changes, breakpoints included
	world.break_on(&["leaf_2"]);
	world.marks.edit("stepping.dm", "\tvar/n = 1\n", "\tvar/n = 1\n\tvar/list/broken = (\n");
	world.wait_output("Hot reload: not reloaded");
	world.drop_events("breakpoint");
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	world.break_on(&[]);
	world.resume_through("middle");
	world.marks.edit("stepping.dm", "\tvar/list/broken = (\n", "");
	world.wait_output("Hot reload: nothing the world runs changed");
	world.drop_events("breakpoint");
	ok("an edit that can't be reloaded leaves the world and its breakpoints as they were");

	// The next world is loaded from the .dmb, and gets the reloads again
	world.break_on(&["leaf_2"]);
	world.run("reboot");
	world.wait_count("dap: world/New", 2);
	world.wait_until("the reloads again", |output| {
		output.rsplit("The world restarted").next().is_some_and(|after| reloads(after) > 0) && output.contains("The world restarted")
	});
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	assert_eq!(world.local("extra"), "6");
	world.break_on(&[]);
	world.resume_through("middle");
	ok("after a reboot the world gets the reloads again, and the breakpoints with them");
	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}
