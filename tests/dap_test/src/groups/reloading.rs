//! Debugging a world while its code is hot reloaded

use std::path::PathBuf;

use serde_json::json;

use crate::{
	client::value_of,
	ok,
	world::{host_file, World}
};

/// A copy of the world's code, to edit, in a directory of its own
pub fn copy_of_the_world(name: &str) -> PathBuf {
	let dir = std::env::temp_dir().join(format!("dap_test_{}-{}", name, std::process::id()));
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
pub fn edit(world: &mut World, file: &str, old: &str, new: &str) {
	let before = reloads(&world.output);
	world.marks.edit(file, old, new);
	world.wait_until("the hot reload", |output| reloads(output) > before);
}

/// Waits for the launcher to say where a breakpoint is now, and returns its
/// line and whether it's set
fn moved(world: &mut World) -> (u64, bool) {
	let event = world.event("breakpoint");
	assert_eq!(event["body"]["reason"], "changed", "{}", event);
	let breakpoint = &event["body"]["breakpoint"];
	(breakpoint["line"].as_u64().unwrap(), breakpoint["verified"].as_bool().unwrap())
}

/// Edits the client has no part in: an agent's, another editor's, a checkout
fn behind_the_clients_back() {
	let dir = copy_of_the_world("reloading");
	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload: watching");
	let at = world.marks.clone();

	// Three lines more above the breakpoint, which the client still has on
	// the old line
	world.break_on(&["leaf_2"]);
	let before = at.line("leaf_2");
	edit(&mut world, "stepping.dm", "/proc/leaf(x)\n", "/proc/above()\n\treturn 1\n\n/proc/leaf(x)\n");
	assert_eq!(moved(&mut world), (before + 3, true));
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	assert_eq!(world.local("y"), "4");
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	world.resume_through("middle");
	ok("lines added above a breakpoint, in a file the client didn't change: it stays on its statement, and the client is told its line");

	// A line more inside the proc, which is compiled again
	edit(&mut world, "stepping.dm", "\tvar/y = x + 1 // @leaf_1\n", "\tvar/y = x + 1 // @leaf_1\n\ty += 0\n");
	assert_eq!(moved(&mut world), (at.line("leaf_2"), true));
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	world.resume_through("middle");
	ok("a line added above a breakpoint in its own proc: it stays on its statement");

	// Its own line is written anew, as two: it's on the first of them
	edit(
		&mut world,
		"stepping.dm",
		"\treturn y * 2 // @leaf_2\n",
		"\tvar/doubled = y * 2 // @leaf_doubled\n\treturn doubled // @leaf_2\n"
	);
	assert_eq!(moved(&mut world), (at.line("leaf_doubled"), true));
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_doubled");
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_doubled");
	world.resume_through("middle");
	ok("a breakpoint whose line is rewritten is on what's written in its place");

	// Its line is taken away: the next line left is the proc's last
	edit(&mut world, "stepping.dm", "\tvar/doubled = y * 2 // @leaf_doubled\n\treturn doubled // @leaf_2\n", "\treturn y * 2 // @leaf_2\n");
	assert_eq!(moved(&mut world), (at.line("leaf_2"), true));
	world.break_on(&[]);

	// VS Code moves its breakpoints with the text while it's edited, and
	// once it's saved sends them again with `sourceModified`: lines of the
	// file as saved. Here they come before the world has the change, and
	// one of them was taken away meanwhile.
	let stepping = at.file("stepping.dm");
	world.break_on(&["leaf_1", "middle_2"]);
	let middle_2 = at.line("middle_2");
	world.drop_events("breakpoint");
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": true }));
	let before = reloads(&world.output);
	at.edit("stepping.dm", "/proc/middle(a)\n", "/proc/middle(a)\n\tvar/first = 1\n\tfirst++\n");
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": false }));
	world.request(
		"setBreakpoints",
		json!({ "source": { "path": stepping }, "sourceModified": true, "breakpoints": [{ "line": middle_2 + 2 }] })
	);
	assert_eq!(reloads(&world.output), before, "the reload came before the client's breakpoints");
	world.wait_until("the hot reload", |output| reloads(output) > before);
	assert_eq!(moved(&mut world), (middle_2 + 2, true));
	assert_eq!(at.line("middle_2"), middle_2 + 2);
	world.run("middle");
	world.stop_on("breakpoint", "/middle", "middle_2");
	world.resume_through("middle");
	ok("breakpoints sent with sourceModified after a save, before the world has the change: set on the lines given once it has, and the one taken away is gone");

	// The same with the world having the change first: the launcher has
	// moved them already. The client hasn't said it's saved yet.
	world.break_on(&["middle_3"]);
	let middle_3 = at.line("middle_3");
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": true }));
	edit(&mut world, "stepping.dm", "\tfirst++\n", "\tfirst++\n\tfirst++\n");
	assert_eq!(moved(&mut world), (middle_3 + 1, true));
	let set = world.request(
		"setBreakpoints",
		json!({ "source": { "path": stepping }, "sourceModified": true, "breakpoints": [{ "line": middle_3 + 1 }] })
	);
	assert_eq!((set["breakpoints"][0]["verified"].clone(), set["breakpoints"][0]["line"].clone()), (json!(true), json!(middle_3 + 1)), "{}", set);
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": false }));
	assert!(!world.has_event("breakpoint"), "the breakpoints were set again on being saved");
	world.run("middle");
	world.stop_on("breakpoint", "/middle", "middle_3");
	world.resume_through("middle");
	ok("breakpoints sent with sourceModified after the world has the change: set as given");

	// Lines added above, not saved, and then a breakpoint added below them:
	// its line is the editor's, which the file doesn't have yet
	world.break_on(&[]);
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": true }));
	let buffer_line = at.line("middle_2") + 3;
	let set = world.request("setBreakpoints", json!({ "source": { "path": stepping }, "breakpoints": [{ "line": buffer_line }] }));
	assert_eq!(set["breakpoints"][0]["verified"], false, "{}", set);
	assert_eq!(set["breakpoints"][0]["message"], "it's set when the file is saved and the world has the change");
	ok("a breakpoint given for a file with unsaved edits waits for the save");

	// Another file is reloaded meanwhile
	edit(&mut world, "main.dm", "var/ticks = 0\n", "var/ticks = 0\nvar/unrelated = 1\n");
	world.run_through("middle");
	assert!(!world.has_event("breakpoint"), "a reload of another file set the unsaved file's breakpoints");
	ok("a reload of another file leaves the unsaved file's breakpoints waiting");

	// Saved: the client says so before the world has the change
	let before = reloads(&world.output);
	at.edit("stepping.dm", "\tvar/r = leaf(a) // @middle_1\n", "\tvar/r = leaf(a) // @middle_1\n\tr += 0\n\tr += 0\n\tr += 0\n");
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": false }));
	assert_eq!(reloads(&world.output), before, "the reload came before the save was said");
	world.event("breakpoint");
	world.wait_until("the hot reload", |output| reloads(output) > before);
	assert_eq!(moved(&mut world), (buffer_line, true));
	assert_eq!(at.line("middle_2"), buffer_line);
	world.run("middle");
	world.stop_on("breakpoint", "/middle", "middle_2");
	world.resume_through("middle");
	ok("once the file is saved and reloaded the breakpoint is on the line the editor had, not moved again");

	// Again with the world having the change before it's said to be saved
	world.set_breakpoints(&stepping, json!([]));
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": true }));
	let buffer_line = at.line("middle_3") + 1;
	world.request("setBreakpoints", json!({ "source": { "path": stepping }, "breakpoints": [{ "line": buffer_line }] }));
	edit(&mut world, "stepping.dm", "\tr += 0\n\tr += 0\n\tr += 0\n", "\tr += 0\n\tr += 0\n\tr += 0\n\tr += 0\n");
	world.run_through("middle");
	assert!(!world.has_event("breakpoint"), "a reload set the unsaved file's breakpoints");
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": false }));
	assert_eq!(moved(&mut world), (buffer_line, true));
	assert_eq!(at.line("middle_3"), buffer_line);
	world.run("middle");
	world.stop_on("breakpoint", "/middle", "middle_3");
	world.resume_through("middle");
	world.set_breakpoints(&stepping, json!([]));
	ok("the same when the world has the change before the file is said to be saved");

	// Another launcher, attached, with a breakpoint of its own: it doesn't
	// reload anything, and is told the code changed
	let pid = crate::client::listed_world("dap_test_host.dmb");
	let mut other = World::attach_in(dir.clone(), pid);
	other.catch_runtimes(false);
	other.break_on(&["sc_middle_2"]);
	let before = at.line("sc_middle_2");
	edit(&mut world, "stepping.dm", "/proc/sc_middle()\n", "/proc/before_it()\n\treturn 1\n\n/proc/sc_middle()\n");
	assert_eq!(moved(&mut other), (before + 3, true));
	world.run("middle");
	world.stop_on("breakpoint", "/sc_middle", "sc_middle_2");
	other.stop_on("breakpoint", "/sc_middle", "sc_middle_2");
	assert_eq!(other.local("got"), "26");
	other.resume();
	world.finish("middle");
	other.end();
	ok("a session attached to a world that another reloads has its breakpoints follow their statements too");

	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}

/// Without hot reloading the world's code stays as it was built
fn saved_without_reloading() {
	let mut world = World::launch(json!({ "hotReload": false }));
	world.catch_runtimes(false);
	world.configure();
	let (file, line) = world.marks.place("leaf_1");
	for _ in 0..2 {
		let set = world.request(
			"setBreakpoints",
			json!({ "source": { "path": file }, "sourceModified": true, "breakpoints": [{ "line": line }] })
		);
		assert_eq!(set["breakpoints"][0]["verified"], true, "{}", set);
	}
	assert_eq!(world.count("has changed since the world was built"), 1, "the console didn't say it once");
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.set_breakpoints(&file, json!([]));
	world.resume_through("middle");
	ok("without hot reloading, breakpoints sent with sourceModified are set as given, and the console says once that the file changed");
	world.end();
}

pub fn run() {
	saved_without_reloading();
	behind_the_clients_back();

	let dir = copy_of_the_world("reloading");
	let mut world = World::launch_as(json!({ "supportsInvalidatedEvent": true }), dir.clone(), json!({ "hotReload": true }));
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
	assert!(!world.has_event("invalidated"), "a reload of a running world left something to ask for again");
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	assert_eq!(world.local("y"), "4");
	ok("a breakpoint the client moved with its line, after a reload that moved the code, stops on the same statement");

	// What the debug server has to say comes once, as the launcher's words
	assert_eq!(world.count("[auxtools] Hot reload: new code for"), 1, "the reload wasn't announced once");
	assert!(!world.output.contains("Debug Server:"), "the debug server's own output was passed on as well");
	assert!(!world.output.contains("Pausing execution"), "a stop was announced in the console");
	ok("the debug server's messages reach the console once, and a stop isn't one of them");

	// Paused in leaf(), whose code changes under it
	edit(
		&mut world,
		"stepping.dm",
		"\tvar/y = x + 1 // @leaf_1\n",
		"\tvar/y = x + 1 // @leaf_1\n\tvar/extra = 5 // @leaf_extra\n\ty += extra\n"
	);
	// The client is shown frames it asked for before the reload
	assert_eq!(world.event("invalidated")["body"]["areas"], json!(["stacks"]));
	let frames = world.frames(0);
	assert_eq!(frames[0]["name"], "/leaf (old code)");
	let locals = world.scope_values("Locals");
	assert_eq!(locals.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), [".", "y"]);
	ok("a reload while the world is paused is taken, the client is told to ask for the frames again, and the paused frame keeps the code and the locals it started with, and says so");

	// Its caller's code is as it was, two lines further down
	assert_eq!(
		(frames[1]["name"].as_str().unwrap(), frames[1]["line"].as_u64().unwrap()),
		("/middle", world.marks.line("middle_1"))
	);
	ok("a frame of a proc whose lines only moved is on its new line");

	// middle()'s r is what the old leaf() gives for 3
	world.break_on(&[]);
	world.step_on("stepOut", "/middle", "middle_1");
	world.step_on("next", "/middle", "middle_2");
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
	let stepping = world.marks.file("stepping.dm");
	let (fresh_1, fresh_3) = (world.marks.line("fresh_1"), world.marks.line("fresh_3"));
	let places = world.request("breakpointLocations", json!({ "source": { "path": stepping }, "line": fresh_1 - 1, "endLine": fresh_3 + 3 }));
	let lines: Vec<u64> = places["breakpoints"].as_array().unwrap().iter().map(|place| place["line"].as_u64().unwrap()).collect();
	assert_eq!(lines, [fresh_1, world.marks.line("fresh_2"), fresh_3, world.marks.line("sc_middle_1")]);
	ok("the places for breakpoints are those of the code the world runs after a reload: a new proc's lines, and the next one's moved down");
	world.run_to("fresh", "/sc_fresh", "fresh_2");
	assert_eq!(world.local("a"), "1");
	world.step_on("next", "/sc_fresh", "fresh_3");
	assert_eq!(world.local("b"), "2");
	world.resume_through("fresh");
	ok("a breakpoint in a proc that a reload added, and a step in it");

	// The world counts a breakpoint's hits from when it's set: a reload of
	// another file leaves it be
	let (file, line) = world.marks.place("recurse_leaf");
	world.set_breakpoints(&file, json!([{ "line": line, "hitCondition": "% 2" }]));
	world.run_through("recurse");
	world.drop_events("breakpoint");
	edit(&mut world, "errors.dm", "\tvar/zero = 0\n", "\tvar/zero = 0 * 2\n");
	assert!(!world.has_event("breakpoint"), "a reload of another file set the breakpoint again");
	world.run("recurse");
	world.stop_on("breakpoint", "/recurse", "recurse_leaf");
	world.set_breakpoints(&file, json!([]));
	world.resume_through("recurse");
	ok("a breakpoint's hit count goes on across a reload of another file");

	// BYOND's own list of the world's globals doesn't get one a reload adds
	edit(&mut world, "main.dm", "var/ticks = 0\n", "var/ticks = 0\nvar/added_later = 12\n");
	assert_eq!(world.eval_global("added_later"), "12");
	assert_eq!(world.eval_global("added_later = ticks > 0 ? 13 : 0"), "13");
	assert_eq!(world.eval_global("global.added_later + 1"), "14");
	ok("a global var that a reload added is read and assigned in expressions");

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
