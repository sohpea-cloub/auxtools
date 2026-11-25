use std::{path::PathBuf, time::Duration};

use serde_json::{json, Value};
use test_support::{
	dap::{named, value_of},
	temp::TempDir
};

use crate::{
	ok, paths,
	world::{host_file, World}
};

pub fn copy_of_the_world(name: &str) -> TempDir {
	TempDir::copy_of(&format!("dap_test_{}", name), &host_file(""), &["dm", "dme", "dmb", "rsc", "dmi"])
}

#[cfg(windows)]
pub fn long_name(dir: PathBuf) -> PathBuf {
	use std::os::windows::ffi::{OsStrExt, OsStringExt};
	#[link(name = "kernel32")]
	extern "system" {
		fn GetLongPathNameW(short: *const u16, long: *mut u16, len: u32) -> u32;
	}
	let short: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
	let mut long = vec![0u16; 1024];
	let len = unsafe { GetLongPathNameW(short.as_ptr(), long.as_mut_ptr(), long.len() as u32) } as usize;
	if len == 0 || len > long.len() {
		return dir;
	}
	PathBuf::from(std::ffi::OsString::from_wide(&long[..len]))
}

#[cfg(not(windows))]
pub fn long_name(dir: PathBuf) -> PathBuf {
	dir
}

fn reloads(output: &str) -> usize {
	output
		.lines()
		.filter(|line| line.starts_with("Hot reload: ") && line.ends_with(" ms"))
		.count()
}

fn last_reload_state(world: &mut World) -> Value {
	let mut last = world.event("auxtoolsReload");
	while let Some(next) = world.event_within("auxtoolsReload", Duration::from_millis(500)) {
		last = next;
	}
	last["body"].clone()
}

pub fn edit(world: &mut World, file: &str, old: &str, new: &str) {
	let before = reloads(&world.output);
	world.marks.edit(file, old, new);
	world.wait_until("the hot reload", |output| reloads(output) > before);
}

fn moved(world: &mut World) -> (u64, bool) {
	let event = world.event("breakpoint");
	assert_eq!(event["body"]["reason"], "changed", "{}", event);
	let breakpoint = &event["body"]["breakpoint"];
	(breakpoint["line"].as_u64().unwrap(), breakpoint["verified"].as_bool().unwrap())
}

fn reboot(world: &mut World, count: usize) {
	let restarts = world.output.matches("The world restarted").count();
	world.run("reboot");
	rebooted(world, count, restarts);
}

fn rebooted(world: &mut World, count: usize, restarts: usize) {
	world.wait_count("dap: world/New", count);
	world.wait_until("the reloads again", |output| {
		output.matches("The world restarted").count() > restarts
			&& output.rsplit("The world restarted").next().is_some_and(|after| reloads(after) > 0)
	});
}

fn watched_world(name: &str) -> (World, TempDir) {
	let dir = copy_of_the_world(name);
	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	(world, dir)
}

fn behind_the_clients_back() {
	let dir = copy_of_the_world("reloading");
	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	let at = world.marks.clone();
	assert_eq!(world.eval_global("TWICE(21)"), "42");
	for _ in 0..3 {
		let why = world.refused("evaluate", json!({ "expression": "CURRENT", "context": "hover" }));
		assert_eq!(why, "the expression may run code, so it is evaluated only when asked");
		let offered = world.request("completions", json!({ "text": "CURRENT.re", "column": 11 }));
		assert_eq!(offered["targets"], json!([]));
	}
	assert_eq!(world.eval_global("counted.reads"), "0");
	assert_eq!(world.eval_global("CURRENT.reads"), "1");
	ok("hover and completion run no macro's call");

	world.break_on(&["leaf_2"]);
	let before = at.line("leaf_2");
	edit(
		&mut world,
		"stepping.dm",
		"/proc/leaf(x)\n",
		"/proc/above()\n\treturn 1\n\n/proc/leaf(x)\n"
	);
	assert_eq!(moved(&mut world), (before + 3, true));
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	assert_eq!(world.local("y"), "4");
	assert_eq!(world.eval("TWICE(y)"), "8");
	ok("source macros in frame and global evaluation");
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	world.resume_through("middle");
	ok("breakpoint remapping after external edits");

	edit(
		&mut world,
		"stepping.dm",
		"\tvar/y = x + 1 // @leaf_1\n",
		"\tvar/y = x + 1 // @leaf_1\n\ty += 0\n"
	);
	assert_eq!(moved(&mut world), (at.line("leaf_2"), true));
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	world.resume_through("middle");
	ok("breakpoint remapping within a proc");

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
	ok("breakpoint on replacement text");

	edit(
		&mut world,
		"stepping.dm",
		"\tvar/doubled = y * 2 // @leaf_doubled\n\treturn doubled // @leaf_2\n",
		"\treturn y * 2 // @leaf_2\n"
	);
	assert_eq!(moved(&mut world), (at.line("leaf_2"), true));
	world.break_on(&[]);

	world.break_on(&["swap_add"]);
	edit(
		&mut world,
		"stepping.dm",
		"\tn += 2 // @swap_add\n\tn *= 3 // @swap_mul\n",
		"\tn *= 3 // @swap_mul\n\tn += 2 // @swap_add\n"
	);
	assert_eq!(moved(&mut world), (at.line("swap_add"), true));
	world.run("swap");
	world.stop_on("breakpoint", "/sc_swap", "swap_add");
	assert_eq!(world.local("n"), "3");
	world.resume_through("swap");
	world.break_on(&[]);
	ok("breakpoint on a statement swapped with the next");

	world.break_on(&["swap_mul"]);
	edit(
		&mut world,
		"stepping.dm",
		"\tn *= 3 // @swap_mul\n",
		"\tif (swap_guard)\n\t\tn *= 3 // @swap_mul\n"
	);
	assert_eq!(moved(&mut world), (at.line("swap_mul"), true));
	world.run_through("swap");
	assert_eq!(world.eval_global("swap_guard = 1"), "1");
	world.run("swap");
	world.stop_on("breakpoint", "/sc_swap", "swap_mul");
	world.resume_through("swap");
	world.break_on(&[]);
	ok("breakpoint on a statement put under a condition");

	world.break_on(&["notify_add"]);
	edit(
		&mut world,
		"stepping.dm",
		"\tn += 5 // @notify_add\n\tif (notify_ready)\n\t\tn *= 2\n",
		"\tif (notify_ready)\n\t\tn *= 2\n\t\tn += 5 // @notify_add\n"
	);
	assert_eq!(moved(&mut world), (at.line("notify_add"), true));
	world.run_through("notify");
	assert_eq!(world.eval_global("notify_ready = 1"), "1");
	world.run("notify");
	world.stop_on("breakpoint", "/sc_notify", "notify_add");
	assert_eq!(world.local("n"), "2");
	world.resume_through("notify");
	world.break_on(&[]);
	ok("breakpoint on a statement moved into the next block");

	let stepping = at.file("stepping.dm");
	world.break_on(&["leaf_1", "middle_2"]);
	let middle_2 = at.line("middle_2");
	world.drop_events("breakpoint");
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": true }));
	let before = reloads(&world.output);
	at.edit("stepping.dm", "/proc/middle(a)\n", "/proc/middle(a)\n\tvar/first = 1\n\tfirst++\n");
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": false }));
	let set = world.request(
		"setBreakpoints",
		json!({ "source": { "path": stepping }, "sourceModified": true, "breakpoints": [{ "line": middle_2 + 2 }] })
	);
	assert_eq!(reloads(&world.output), before, "the reload came before the client's breakpoints");
	assert_eq!(
		(
			&set["breakpoints"][0]["verified"],
			&set["breakpoints"][0]["line"],
			&set["breakpoints"][0]["message"]
		),
		(&json!(false), &json!(middle_2 + 2), &json!("pending hot reload")),
		"{}",
		set
	);
	world.wait_until("the hot reload", |output| reloads(output) > before);
	assert_eq!(moved(&mut world), (middle_2 + 2, true));
	assert_eq!(at.line("middle_2"), middle_2 + 2);
	world.run("middle");
	world.stop_on("breakpoint", "/middle", "middle_2");
	world.resume_through("middle");
	ok("defer sourceModified breakpoints until reload");

	world.break_on(&["middle_3"]);
	let middle_3 = at.line("middle_3");
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": true }));
	edit(&mut world, "stepping.dm", "\tfirst++\n", "\tfirst++\n\tfirst++\n");
	assert_eq!(moved(&mut world), (middle_3 + 1, true));
	let set = world.request(
		"setBreakpoints",
		json!({ "source": { "path": stepping }, "sourceModified": true, "breakpoints": [{ "line": middle_3 + 1 }] })
	);
	assert_eq!(
		(set["breakpoints"][0]["verified"].clone(), set["breakpoints"][0]["line"].clone()),
		(json!(true), json!(middle_3 + 1)),
		"{}",
		set
	);
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": false }));
	assert!(!world.has_event("breakpoint"), "the breakpoints were set again on being saved");
	world.run("middle");
	world.stop_on("breakpoint", "/middle", "middle_3");
	world.resume_through("middle");
	ok("sourceModified after reload");

	world.break_on(&[]);
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": true }));
	let buffer_line = at.line("middle_2") + 3;
	let set = world.request(
		"setBreakpoints",
		json!({ "source": { "path": stepping }, "breakpoints": [{ "line": buffer_line }] })
	);
	assert_eq!(set["breakpoints"][0]["verified"], false, "{}", set);
	assert_eq!(set["breakpoints"][0]["message"], "pending save and hot reload");
	ok("unsaved breakpoint waits for save");

	edit(&mut world, "main.dm", "var/ticks = 0\n", "var/ticks = 0\nvar/unrelated = 1\n");
	world.run_through("middle");
	assert!(
		!world.has_event("breakpoint"),
		"a reload of another file set the unsaved file's breakpoints"
	);
	ok("unrelated reload preserves pending breakpoint");

	let before = reloads(&world.output);
	at.edit(
		"stepping.dm",
		"\tvar/r = leaf(a) // @middle_1\n",
		"\tvar/r = leaf(a) // @middle_1\n\tr += 0\n\tr += 0\n\tr += 0\n"
	);
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": false }));
	assert_eq!(reloads(&world.output), before, "reload occurred before the save notification");
	let waiting = world.event("breakpoint");
	assert_eq!(
		(&waiting["body"]["breakpoint"]["verified"], &waiting["body"]["breakpoint"]["line"]),
		(&json!(false), &json!(buffer_line)),
		"{}",
		waiting
	);
	world.wait_until("the hot reload", |output| reloads(output) > before);
	assert_eq!(moved(&mut world), (buffer_line, true));
	assert_eq!(at.line("middle_2"), buffer_line);
	world.run("middle");
	world.stop_on("breakpoint", "/middle", "middle_2");
	world.resume_through("middle");
	ok("save and reload do not remap twice");

	world.set_breakpoints(&stepping, json!([]));
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": true }));
	let buffer_line = at.line("middle_3") + 1;
	world.request(
		"setBreakpoints",
		json!({ "source": { "path": stepping }, "breakpoints": [{ "line": buffer_line }] })
	);
	edit(
		&mut world,
		"stepping.dm",
		"\tr += 0\n\tr += 0\n\tr += 0\n",
		"\tr += 0\n\tr += 0\n\tr += 0\n\tr += 0\n"
	);
	world.run_through("middle");
	assert!(!world.has_event("breakpoint"), "a reload set the unsaved file's breakpoints");
	world.request("auxtoolsUnsaved", json!({ "path": stepping, "unsaved": false }));
	assert_eq!(moved(&mut world), (buffer_line, true));
	assert_eq!(at.line("middle_3"), buffer_line);
	world.run("middle");
	world.stop_on("breakpoint", "/middle", "middle_3");
	world.resume_through("middle");
	world.set_breakpoints(&stepping, json!([]));
	ok("reload before save notification");

	let pid = test_support::cli::listed_world("dap_test_host.dmb");
	let mut other = World::attach_in(dir.to_path_buf(), pid);
	other.catch_runtimes(false);
	other.break_on(&["sc_middle_2"]);
	let before = at.line("sc_middle_2");
	edit(
		&mut world,
		"stepping.dm",
		"/proc/sc_middle()\n",
		"/proc/before_it()\n\treturn 1\n\n/proc/sc_middle()\n"
	);
	assert_eq!(moved(&mut other), (before + 3, true));
	world.run("middle");
	world.stop_on("breakpoint", "/sc_middle", "sc_middle_2");
	other.stop_on("breakpoint", "/sc_middle", "sc_middle_2");
	assert_eq!(other.local("got"), "26");
	other.resume();
	world.finish("middle");
	other.end();
	ok("attached-session breakpoint remapping");

	world.end();
}

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
	assert_eq!(
		world.count("has changed since the world was built"),
		1,
		"expected exactly one changed-source warning"
	);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.set_breakpoints(&file, json!([]));
	world.resume_through("middle");
	ok("changed-source warning without hot reload");
	let said = world.refused("evaluate", json!({ "expression": "TWICE(21)", "context": "repl" }));
	assert!(said.contains("If it is a #define, evaluation can use it only after"), "{}", said);
	ok("missing-macro diagnostic without hot reload");
	world.end();
}

fn starting_at_world_new() {
	let dir = TempDir::new("dap_test_world_new");
	std::fs::write(dir.join("dap_test_host.dme"), "#include \"bare.dm\"\n").unwrap();
	std::fs::write(
		dir.join("bare.dm"),
		"/world/New()\n\tworld.log << \"dap: world/New\"\n\n/proc/icon_new()\n\treturn null\n"
	)
	.unwrap();
	std::fs::copy(host_file("reload_new.dmi"), dir.join("reload_new.dmi")).unwrap();
	paths::compile(&dir.join("dap_test_host.dme"));

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.configure();
	world.wait_output("Hot reload active");
	edit(&mut world, "bare.dm", "\treturn null\n", "\treturn 'reload_new.dmi'\n");
	assert_eq!(world.eval_global("jointext(icon_states(icon_new()), \",\")"), "\"new1\"");
	assert!(
		!world.output.contains("applies when world/New starts"),
		"the reload waited for world/New:\n{}",
		world.output
	);
	ok("resource reload after world/New startup");
	world.end();
}

fn completion_after_a_rename() {
	let dir = TempDir::new("dap_test_renamed");
	std::fs::write(dir.join("dap_test_host.dme"), "#define DEBUG\n#include \"bare.dm\"\n").unwrap();
	std::fs::write(
		dir.join("bare.dm"),
		"/world/New()\n\tworld.log << \"dap: world/New\"\n\n/proc/helper()\n\treturn 1\n"
	)
	.unwrap();
	paths::compile(&dir.join("dap_test_host.dme"));
	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.configure();
	world.wait_output("Hot reload active");
	let offered = |world: &mut World, text: &str| -> Vec<String> {
		let targets = world.request("completions", json!({ "text": text, "column": text.len() + 1 }));
		targets["targets"]
			.as_array()
			.unwrap()
			.iter()
			.map(|t| t["label"].as_str().unwrap().to_owned())
			.collect()
	};
	assert_eq!(offered(&mut world, "::hel"), ["helper"]);
	edit(&mut world, "bare.dm", "/proc/helper()\n", "/proc/helped()\n");
	assert_eq!(offered(&mut world, "::hel"), ["helped"]);
	assert_eq!(offered(&mut world, "hel"), ["helped"]);
	ok("completion after a reload renames a proc");
	world.end();
}

fn edited_during_a_reboot() {
	let (mut world, _dir) = watched_world("edited_during_a_reboot");
	let at = world.marks.clone();
	world.break_on(&["leaf_2"]);
	let before = at.line("leaf_2");
	edit(
		&mut world,
		"main.dm",
		"/proc/sc_reboot()\n",
		"/proc/sc_edit_and_reboot()\n\tvar/text = file2text(\"stepping.dm\")\n\tfdel(\"stepping.dm\")\n\ttext2file(replacetext(text, \
		 \"/proc/leaf(x)\\n\", \"/proc/above()\\n\\treturn 1\\n\\n/proc/leaf(x)\\n\"), \
		 \"stepping.dm\")\n\ttext2file(\"\\n/proc/appended()\\n\\treturn 1\\n\", \"main.dm\")\n\tworld.Reboot()\n\n/proc/sc_reboot()\n"
	);
	let restarts = world.output.matches("The world restarted").count();
	world.run("edit_and_reboot");
	rebooted(&mut world, 2, restarts);
	assert_eq!(at.line("leaf_2"), before + 3);
	assert_eq!(moved(&mut world), (before + 3, true));
	let set = world.try_break_if(&[("leaf_2", ""), ("early_value", "")]);
	assert_eq!((&set[0]["line"], &set[0]["verified"]), (&json!(before + 3), &json!(true)), "{}", set[0]);
	assert_eq!(set[1]["verified"], true, "{}", set[1]);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	world.resume_through("middle");
	ok("breakpoints follow an edit read after a reboot");
	world.end();
}

fn types_changed_before_reboot() {
	let (mut world, _dir) = watched_world("types_changed_before_reboot");
	edit(
		&mut world,
		"main.dm",
		"/proc/icon_built()\n",
		"/datum/rb_base\n\tvar/list/held = list(3)\n\tvar/x = 5\n\n/datum/rb_base/kid\n\n/datum/rb_base/New()\n\tx += \
		 1\n\t..()\n\n/proc/icon_built()\n"
	);
	edit(
		&mut world,
		"stepping.dm",
		"/datum/made\n",
		"/datum/made\n\tparent_type = /datum/rb_base\n"
	);
	edit(
		&mut world,
		"stepping.dm",
		"\tsize = n // @made_new\n",
		"\tsize = n // @made_new\n\t..()\n"
	);
	let check = |world: &mut World| {
		assert_eq!(world.eval_global("var/datum/rb_base/kid/k = new\n\"[k.x] [length(k.held)]\""), "\"6 1\"");
		assert_eq!(
			world.eval_global("var/datum/made/m = new(2)\n\"[istype(m, /datum/rb_base)] [m:x] [m.size]\""),
			"\"1 6 2\""
		);
	};
	check(&mut world);
	for count in 2..=4 {
		reboot(&mut world, count);
		check(&mut world);
	}
	ok("type and parent restoration after reboot");
	world.end();
}

fn override_restored_before_map() {
	let dir = copy_of_the_world("override_restored_before_map");
	std::fs::write(
		dir.join("placed.dm"),
		"var/placed_news = 0\n\n/obj/placed\n\n/obj/placed/New()\n\tplaced_news++\n\t..()\n\n/datum/rb_over\n"
	)
	.unwrap();
	std::fs::write(
		dir.join("placed.dmm"),
		"\"a\" = (/turf,/area)\n\"b\" = (/obj/placed,/turf,/area)\n\n(1,1,1) = {\"\naab\n\"}\n"
	)
	.unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"placed.dm\"\n#include \"placed.dmm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	edit(
		&mut world,
		"placed.dm",
		"/datum/rb_over\n",
		"/datum/rb_over\n\n/datum/rb_over/New()\n\tplaced_news += 10\n\t..()\n"
	);
	let made = |world: &mut World| world.eval_global("var/before = placed_news\nnew /datum/rb_over\nplaced_news - before");
	assert_eq!(made(&mut world), "10");
	for count in 2..=3 {
		reboot(&mut world, count);
		assert_eq!(world.eval_global("placed_news"), "1", "map atom was not constructed exactly once");
		assert_eq!(made(&mut world), "10");
	}
	ok("override restoration before map creation");
	world.end();
}

const INITIALIZERS: &str = "/proc/rb_late_set()
	return isnull(rb_late) ? \"null\" : \"set\"

var/rb_a = 5
var/rb_b = rb_a * 2
var/rb_news = 0
var/rb_seen = 0

/datum/rb_counted
	var/n

/datum/rb_counted/New(n)
	src.n = n
	rb_news++

var/datum/rb_counted/rb_made = new(1)

/datum/rb_static
	var/static/s = rb_a + 1

/proc/rb_local()
	var/static/l = rb_a + 2
	return l

var/rb_late = rb_a

/obj/rb_placed/New()
	rb_seen = rb_b
	..()

/datum/rb_holder
	var/held = 'lamp.dmi'

/proc/rb_held()
	var/datum/rb_holder/H = new
	return list(H.held, 'lamp.dmi')
";

fn held_changes(world: &mut World) -> Vec<(String, String)> {
	let state = last_reload_state(world);
	let mut held: Vec<(String, String)> = state["problems"]
		.as_array()
		.unwrap()
		.iter()
		.map(|p| (p["message"].as_str().unwrap().to_owned(), p["until"].as_str().unwrap().to_owned()))
		.collect();
	held.sort();
	held
}

fn edit_held(world: &mut World, file: &str, old: &str, new: &str) {
	let said = |output: &str| output.matches("Hot reload requires a reboot:").count() + output.matches("Hot reload requires a rebuild:").count();
	let before = said(&world.output);
	world.marks.edit(file, old, new);
	world.wait_until("the held reload", |output| said(output) > before);
}

fn initializers_after_reboot() {
	let dir = copy_of_the_world("initializers_after_reboot");
	std::fs::write(dir.join("inits.dm"), INITIALIZERS).unwrap();
	std::fs::write(
		dir.join("inits.dmm"),
		"\"a\" = (/turf,/area)\n\"b\" = (/obj/rb_placed,/turf,/area)\n\n(1,1,1) = {\"\naab\n\"}\n"
	)
	.unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"inits.dm\"\n#include \"inits.dmm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	const VALUES: &str = "\"[rb_a] [rb_b] [rb_news] [rb_made.n] [(new /datum/rb_static).s] [rb_local()] [rb_c] [rb_seen]\"";
	edit_held(&mut world, "inits.dm", "var/rb_a = 5\n", "var/rb_a = 7\n");
	edit_held(&mut world, "inits.dm", "new(1)", "new(2)");
	edit_held(&mut world, "inits.dm", "\tvar/static/s = rb_a + 1\n", "\tvar/static/s = rb_a + 10\n");
	edit_held(&mut world, "inits.dm", "\tvar/static/l = rb_a + 2\n", "\tvar/static/l = rb_a + 20\n");
	edit_held(&mut world, "inits.dm", "var/rb_news = 0\n", "var/rb_news = 0\nvar/rb_c = rb_late_set()\n");
	let held: Vec<String> = held_changes(&mut world).into_iter().map(|(message, _)| message).collect();
	assert_eq!(
		held,
		[
			"(global): rb_a: changed initializer",
			"(global): rb_made: changed initializer",
			"/datum/rb_static: s: changed initializer",
			"/proc/rb_local: l: changed initializer"
		],
		"held"
	);
	assert_eq!(
		world.eval_global("\"[rb_a] [rb_b] [rb_news] [rb_made.n] [(new /datum/rb_static).s] [rb_local()] [rb_seen]\""),
		"\"5 10 1 1 6 7 10\""
	);
	ok("changed initializers held until reboot");
	for count in 2..=3 {
		reboot(&mut world, count);
		assert_eq!(world.eval_global(VALUES), "\"7 14 1 2 17 27 null 14\"", "after reboot {}", count - 1);
		assert_eq!(held_changes(&mut world), [], "after reboot {}", count - 1);
	}
	ok("updated global initialization after reboot");
	std::fs::copy(host_file("reload_new.dmi"), dir.join("rb_new.dmi")).unwrap();
	edit(
		&mut world,
		"inits.dm",
		"\treturn l\n",
		"\treturn l\n\n/proc/rb_icon()\n\tvar/static/rb_kept = rb_a + 1\n\treturn list(new /datum/rb_static, 'rb_new.dmi', rb_kept)\n"
	);
	reboot(&mut world, 4);
	assert!(
		world.output.contains("applies when world/New starts"),
		"the restore didn't wait:\n{}",
		world.output
	);
	assert_eq!(world.eval_global(VALUES), "\"7 14 1 2 17 27 null 14\"");
	ok("updated global initialization with deferred restore");
	assert_eq!(world.eval_global("var/list/I = rb_icon()\n\"[length(I)] [I[3]]\""), "\"3 8\"");
	resource_saved(&mut world, &dir, "lamp_edited.dmi");
	let held = "var/list/L = rb_held()\n\"[length(icon_states(L[1]))] [length(icon_states(L[2]))]\"";
	assert_eq!(world.eval_global(held), "\"3 3\"");
	reboot(&mut world, 5);
	assert_eq!(world.eval_global(held), "\"3 3\"");
	ok("type defaults in deferred restore");
	world.end();
}

fn initializer_added_to_a_world_without() {
	let dir = TempDir::new("dap_test_no_inits");
	std::fs::write(dir.join("dap_test_host.dme"), "#include \"bare.dm\"\n").unwrap();
	std::fs::write(
		dir.join("bare.dm"),
		"var/bare_n = 1\n\n/world/New()\n\tworld.log << \"dap: world/New\"\n\n/proc/bare_reboot()\n\tspawn(1)\n\t\tworld.Reboot()\n"
	)
	.unwrap();
	paths::compile(&dir.join("dap_test_host.dme"));

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	edit(
		&mut world,
		"bare.dm",
		"var/bare_n = 1\n",
		"var/bare_n = 1\nvar/list/bare_list = list(bare_n, 2)\n"
	);
	edit_held(&mut world, "bare.dm", "var/bare_n = 1\n", "var/bare_n = 3\n");
	assert_eq!(world.eval_global("\"[bare_n] [length(bare_list)] [bare_list[1]]\""), "\"1 2 1\"");
	for count in 2..=3 {
		let restarts = world.count("The world restarted");
		world.eval_global("bare_reboot()");
		rebooted(&mut world, count, restarts);
		assert_eq!(
			world.eval_global("\"[bare_n] [length(bare_list)] [bare_list[1]]\""),
			"\"3 2 3\"",
			"after reboot {}",
			count - 1
		);
	}
	ok("added global init proc after reboot");
	world.end();
}

const HANDLERS: &str = "/datum/rb_handled/proc/speak()
	return \"declaration\"

/datum/rb_handled/speak()
	return \"first override\"

/datum/rb_handled/speak()
	return \"second override\"

var/list/rb_handlers = list(/datum/rb_handled/speak)
";

fn initializer_naming_a_retargeted_path() {
	let dir = copy_of_the_world("initializer_naming_a_retargeted_path");
	std::fs::write(dir.join("handlers.dm"), HANDLERS).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"handlers.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	const SAYS: &str = "call(new /datum/rb_handled, rb_handlers[1])()";
	assert_eq!(world.eval_global(SAYS), "\"first override\"");
	edit(&mut world, "handlers.dm", "/datum/rb_handled/speak()\n\treturn \"first override\"\n", "");
	reboot(&mut world, 2);
	assert_eq!(world.eval_global(SAYS), "\"second override\"");
	ok("global initializer naming a path a reload retargets, after reboot");
	world.end();
}

const STATIC_HANDLERS: &str = "/datum/rb_static_handled/proc/speak()
	return \"declaration\"

/datum/rb_static_handled/speak()
	return \"first override\"

/datum/rb_static_handled/speak()
	return \"second override\"
";

const STATIC_CALLBACK: &str = "/proc/rb_static_callback()
	var/static/handler = /datum/rb_static_handled/speak
	return handler
";

fn static_naming_a_retargeted_path() {
	let dir = copy_of_the_world("static_naming_a_retargeted_path");
	std::fs::write(dir.join("static_handlers.dm"), STATIC_HANDLERS).unwrap();
	std::fs::write(dir.join("static_callback.dm"), STATIC_CALLBACK).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"static_handlers.dm\"\n#include \"static_callback.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	const SAYS: &str = "call(new /datum/rb_static_handled, rb_static_callback())()";
	assert_eq!(world.eval_global(SAYS), "\"first override\"");
	edit(
		&mut world,
		"static_handlers.dm",
		"/datum/rb_static_handled/speak()\n\treturn \"first override\"\n",
		""
	);
	reboot(&mut world, 2);
	assert_eq!(world.eval_global(SAYS), "\"second override\"");
	ok("proc static naming a path a reload retargets, after reboot");
	world.end();
}

const STATIC_LIMIT: &str = "var/const/RB_STATIC_LIMIT = 5
";

const STATIC_LIMITS: &str = "/proc/rb_static_limits()
	var/static/list/limits = list(RB_STATIC_LIMIT + 1)
	return limits
";

fn static_folding_a_changed_const() {
	let dir = copy_of_the_world("static_folding_a_changed_const");
	std::fs::write(dir.join("static_limit.dm"), STATIC_LIMIT).unwrap();
	std::fs::write(dir.join("static_limits.dm"), STATIC_LIMITS).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"static_limit.dm\"\n#include \"static_limits.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	assert_eq!(world.eval_global("rb_static_limits()[1]"), "6");
	edit(&mut world, "static_limit.dm", "RB_STATIC_LIMIT = 5", "RB_STATIC_LIMIT = 10");
	reboot(&mut world, 2);
	assert_eq!(world.eval_global("rb_static_limits()[1]"), "11");
	ok("proc static folding a const a reload changes, after reboot");
	world.end();
}

const STATIC_CONFIG: &str = "/datum/rb_static_config
	var/limit = 5
";

const STATIC_DEFAULTS: &str = "/proc/rb_static_defaults()
	var/static/list/limits = list(/datum/rb_static_config::limit)
	return limits
";

fn static_reading_a_changed_default() {
	let dir = copy_of_the_world("static_reading_a_changed_default");
	std::fs::write(dir.join("static_config.dm"), STATIC_CONFIG).unwrap();
	std::fs::write(dir.join("static_defaults.dm"), STATIC_DEFAULTS).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"static_config.dm\"\n#include \"static_defaults.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	assert_eq!(world.eval_global("rb_static_defaults()[1]"), "5");
	edit(&mut world, "static_config.dm", "limit = 5", "limit = 10");
	reboot(&mut world, 2);
	assert_eq!(world.eval_global("rb_static_defaults()[1]"), "10");
	ok("proc static reading a default a reload changes, after reboot");
	world.end();
}

const STATIC_COLON_TYPES: &str = "/datum/rb_static_colon_first
	var/const/RB_STATIC_COLON = 5

/datum/rb_static_colon_later
";

const STATIC_COLON: &str = "var/rb_static_colon_holder

/proc/rb_static_colon_limits()
	var/static/list/limits = list(rb_static_colon_holder:RB_STATIC_COLON + 1)
	return limits
";

fn static_reading_a_changed_colon_name() {
	let dir = copy_of_the_world("static_reading_a_changed_colon_name");
	std::fs::write(dir.join("static_colon_types.dm"), STATIC_COLON_TYPES).unwrap();
	std::fs::write(dir.join("static_colon.dm"), STATIC_COLON).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"static_colon_types.dm\"\n#include \"static_colon.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	assert_eq!(world.eval_global("rb_static_colon_limits()[1]"), "6");
	edit(
		&mut world,
		"static_colon_types.dm",
		"/datum/rb_static_colon_later\n",
		"/datum/rb_static_colon_later\n\tvar/const/RB_STATIC_COLON = 9\n"
	);
	reboot(&mut world, 2);
	assert_eq!(world.eval_global("rb_static_colon_limits()[1]"), "10");
	ok("proc static reading a var by x:name that another type declares now, after reboot");
	world.end();
}

const STATIC_ITEMS: &str = "/datum/rb_static_item_a

/datum/rb_static_item_b
";

const STATIC_ITEM: &str = "/proc/rb_static_item()
	var/static/datum/rb_static_item_a/item = new
	return item.type
";

fn static_made_by_new_of_a_changed_type() {
	let dir = copy_of_the_world("static_made_by_new_of_a_changed_type");
	std::fs::write(dir.join("static_items.dm"), STATIC_ITEMS).unwrap();
	std::fs::write(dir.join("static_item.dm"), STATIC_ITEM).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"static_items.dm\"\n#include \"static_item.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	assert_eq!(world.eval_global("\"[rb_static_item()]\""), "\"/datum/rb_static_item_a\"");
	let held = |output: &str| output.matches("Hot reload requires a reboot:").count();
	let settled = |output: &str| reloads(output) + held(output);
	let (reloaded, waiting) = (reloads(&world.output), held(&world.output));
	world
		.marks
		.edit("static_item.dm", "datum/rb_static_item_a/item", "datum/rb_static_item_b/item");
	world.wait_until("the edit's reload", |output| settled(output) > reloaded + waiting);
	assert!(held(&world.output) > waiting, "the edit was reloaded without a reboot:\n{}", world.output);
	reboot(&mut world, 2);
	assert_eq!(world.eval_global("\"[rb_static_item()]\""), "\"/datum/rb_static_item_b\"");
	ok("proc static made by a bare new of a type a reload changes, after reboot");
	world.end();
}

const STATIC_RETYPED: &str = "/datum/rb_static_retyped_a
	var/const/RB_STATIC_RETYPED = 5

/datum/rb_static_retyped_b
	var/const/RB_STATIC_RETYPED = 9

var/datum/rb_static_retyped_a/rb_static_retyped_target

/proc/rb_static_retyped_limits()
	var/static/list/limits = list(rb_static_retyped_target.RB_STATIC_RETYPED + 1)
	return limits
";

fn static_reading_through_a_retyped_var() {
	let dir = copy_of_the_world("static_reading_through_a_retyped_var");
	std::fs::write(dir.join("static_retyped.dm"), STATIC_RETYPED).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"static_retyped.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	assert_eq!(world.eval_global("rb_static_retyped_limits()[1]"), "6");
	let unchanged = |output: &str| output.matches("Hot reload: no changes to the running code").count();
	let before = unchanged(&world.output);
	world.marks.edit(
		"static_retyped.dm",
		"var/datum/rb_static_retyped_a/rb_static_retyped_target",
		"var/datum/rb_static_retyped_b/rb_static_retyped_target"
	);
	world.wait_until("the reload", |output| unchanged(output) > before);
	reboot(&mut world, 2);
	assert_eq!(world.eval_global("rb_static_retyped_limits()[1]"), "10");
	ok("proc static reading a const through a var declared with another type now, after reboot");
	world.end();
}

const STATIC_TYPED: &str = "/datum/rb_static_typed_config
	var/const/RB_STATIC_TYPED = 5

/proc/rb_static_typed_limits()
	var/static/datum/rb_static_typed_config/config
	var/static/list/limits = list(config.RB_STATIC_TYPED)
	return limits
";

fn static_reading_through_a_typed_static() {
	let dir = copy_of_the_world("static_reading_through_a_typed_static");
	std::fs::write(dir.join("static_typed.dm"), STATIC_TYPED).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"static_typed.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	assert_eq!(world.eval_global("rb_static_typed_limits()[1]"), "5");
	let held = |output: &str| output.matches("Hot reload requires a reboot:").count();
	let settled = |output: &str| reloads(output) + held(output);
	let (reloaded, waiting) = (reloads(&world.output), held(&world.output));
	world
		.marks
		.edit("static_typed.dm", "list(config.RB_STATIC_TYPED)", "list(config.RB_STATIC_TYPED + 1)");
	world.wait_until("the edit's reload", |output| settled(output) > reloaded + waiting);
	assert!(held(&world.output) > waiting, "the edit was reloaded without a reboot:\n{}", world.output);
	reboot(&mut world, 2);
	assert_eq!(world.eval_global("json_encode(rb_static_typed_limits())"), "\"[6]\"");
	ok("proc static reading a const through a null static of its proc, after reboot");
	world.end();
}

const STATIC_MOVED: &str = "var/rb_static_seed = 20

/proc/rb_static_seeded(x)
	var/static/rb_static_seed = 1
	if (x)
		rb_static_seed++
	var/static/list/cache = list(rb_static_seed)
	return cache[1]
";

fn static_read_by_an_initializer_moved_into_a_block() {
	let dir = copy_of_the_world("static_read_by_an_initializer_moved_into_a_block");
	std::fs::write(dir.join("static_moved.dm"), STATIC_MOVED).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"static_moved.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	assert_eq!(world.eval_global("rb_static_seeded(0)"), "1");
	let held = |output: &str| output.matches("Hot reload requires a reboot:").count();
	let settled = |output: &str| reloads(output) + held(output);
	let (reloaded, waiting) = (reloads(&world.output), held(&world.output));
	world.marks.edit(
		"static_moved.dm",
		"\tvar/static/rb_static_seed = 1\n\tif (x)\n\t\trb_static_seed++",
		"\tif (x)\n\t\tvar/static/rb_static_seed = 1\n\t\trb_static_seed++"
	);
	world.wait_until("the edit's reload", |output| settled(output) > reloaded + waiting);
	assert!(held(&world.output) > waiting, "the edit was reloaded without a reboot:\n{}", world.output);
	reboot(&mut world, 2);
	assert_eq!(world.eval_global("\"[rb_static_seeded(0)] [rb_static_seeded(1)]\""), "\"20 20\"");
	ok("proc static whose initializer reads a global once a static of its name moves into a block, after reboot");
	world.end();
}

const STATIC_PAIR: &str = "/datum/rb_static_pair/proc/f()
	return 0

/datum/rb_static_pair/f()
	var/static/list/cache = list(1)
	return cache[1]

/datum/rb_static_pair/f()
	var/static/list/cache = list(2)
	return ..() * 10 + cache[1]
";

fn statics_of_two_overrides_with_the_same_name() {
	let dir = copy_of_the_world("statics_of_two_overrides_with_the_same_name");
	std::fs::write(dir.join("static_pair.dm"), STATIC_PAIR).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"static_pair.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	assert_eq!(world.eval_global("(new /datum/rb_static_pair).f()"), "12");
	let held = |output: &str| output.matches("Hot reload requires a reboot:").count();
	let settled = |output: &str| reloads(output) + held(output);
	let (reloaded, waiting) = (reloads(&world.output), held(&world.output));
	world.marks.edit("static_pair.dm", "list(2)", "list(3)");
	world.wait_until("the edit's reload", |output| settled(output) > reloaded + waiting);
	assert!(held(&world.output) > waiting, "the edit was reloaded without a reboot:\n{}", world.output);
	reboot(&mut world, 2);
	assert_eq!(world.eval_global("(new /datum/rb_static_pair).f()"), "13", "{}", world.output);
	ok("statics of the same name in two overrides, each with its own value after reboot");
	world.end();
}

const STATIC_GAINED: &str = "var/list/rb_gained_cache = list(5)

/proc/rb_gained()
	return rb_gained_cache[1]
";

fn static_gained_beside_a_global_of_its_name() {
	let dir = copy_of_the_world("static_gained_beside_a_global_of_its_name");
	std::fs::write(dir.join("static_gained.dm"), STATIC_GAINED).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"static_gained.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	let both = "\"[rb_gained_cache[1]] [rb_gained()]\"";
	assert_eq!(world.eval_global(both), "\"5 5\"");
	edit(
		&mut world,
		"static_gained.dm",
		"\treturn rb_gained_cache[1]",
		"\tvar/static/list/rb_gained_cache = list(1)\n\treturn rb_gained_cache[1]"
	);
	assert_eq!(world.eval_global(both), "\"5 1\"", "{}", world.output);
	reboot(&mut world, 2);
	assert_eq!(
		world.eval_global(both),
		"\"5 1\"",
		"the global's value and the new static's after reboot:\n{}",
		world.output
	);
	ok("proc static added beside a global of its name that the proc read, after reboot");
	world.end();
}

const STATIC_CONFIGS: &str = "/datum/rb_static_config_a
	var/const/RB_CONFIG_LIMIT = 5

/datum/rb_static_config_b
	var/const/RB_CONFIG_LIMIT = 9

/proc/rb_static_config_limits()
	var/static/datum/rb_static_config_a/config
	var/static/list/limits = list(config.RB_CONFIG_LIMIT)
	return limits
";

fn static_reading_through_a_retyped_static() {
	let dir = copy_of_the_world("static_reading_through_a_retyped_static");
	std::fs::write(dir.join("static_configs.dm"), STATIC_CONFIGS).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"static_configs.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	assert_eq!(world.eval_global("json_encode(rb_static_config_limits())"), "\"[5]\"");
	let held = |output: &str| output.matches("Hot reload requires a reboot:").count();
	let unchanged = |output: &str| output.matches("Hot reload: no changes to the running code").count();
	let settled = |output: &str| reloads(output) + held(output) + unchanged(output);
	let before = settled(&world.output);
	world.marks.edit(
		"static_configs.dm",
		"var/static/datum/rb_static_config_a/config",
		"var/static/datum/rb_static_config_b/config"
	);
	world.wait_until("the edit's reload", |output| settled(output) > before);
	reboot(&mut world, 2);
	assert_eq!(
		world.eval_global("json_encode(rb_static_config_limits())"),
		"\"[9]\"",
		"the const read through the static's new type after reboot:\n{}",
		world.output
	);
	ok("proc static reading a const through a static of its proc declared with another type now, after reboot");
	world.end();
}

const STATIC_KINDS: &str = "var/rb_static_kind = /datum/rb_static_kind_b

/datum/rb_static_kind_a

/datum/rb_static_kind_b

/proc/rb_static_kinded(x)
	var/static/rb_static_kind = /datum/rb_static_kind_a
	if (x)
		rb_static_kind = null
	var/static/datum/item = new rb_static_kind()
	return item.type
";

fn static_made_by_new_of_a_static_moved_into_a_block() {
	let dir = copy_of_the_world("static_made_by_new_of_a_static_moved_into_a_block");
	std::fs::write(dir.join("static_kinds.dm"), STATIC_KINDS).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"static_kinds.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	assert_eq!(world.eval_global("\"[rb_static_kinded(0)]\""), "\"/datum/rb_static_kind_a\"");
	let held = |output: &str| output.matches("Hot reload requires a reboot:").count();
	let settled = |output: &str| reloads(output) + held(output);
	let before = settled(&world.output);
	world.marks.edit(
		"static_kinds.dm",
		"\tvar/static/rb_static_kind = /datum/rb_static_kind_a\n\tif (x)\n\t\trb_static_kind = null",
		"\tif (x)\n\t\tvar/static/rb_static_kind = /datum/rb_static_kind_a\n\t\trb_static_kind = null"
	);
	world.wait_until("the edit's reload", |output| settled(output) > before);
	reboot(&mut world, 2);
	assert_eq!(
		world.eval_global("\"[rb_static_kinded(0)]\""),
		"\"/datum/rb_static_kind_b\"",
		"the type that `new` made from the global after reboot:\n{}",
		world.output
	);
	ok("proc static made by new of a global once a static of its name moves into a block, after reboot");
	world.end();
}

const COMPUTED_KINDS: &str = "/proc/rb_computed_kind_made()
	var/static/rb_computed_kind = rb_computed_kind_chosen()
	var/static/rb_computed_item = new rb_computed_kind()
	return \"[rb_computed_item:type]\"

/proc/rb_computed_kind_chosen()
	return /datum/rb_computed_kind

/datum/rb_computed_kind
";

fn static_made_by_new_of_a_computed_static() {
	let dir = copy_of_the_world("static_made_by_new_of_a_computed_static");
	std::fs::write(dir.join("computed_kinds.dm"), COMPUTED_KINDS).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"computed_kinds.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	assert_eq!(world.eval_global("rb_computed_kind_made()"), "\"/datum/rb_computed_kind\"");
	let held = |output: &str| output.matches("Hot reload requires a reboot:").count();
	let settled = |output: &str| reloads(output) + held(output);
	let before = settled(&world.output);
	world.marks.edit("computed_kinds.dm", "new rb_computed_kind()", "new rb_computed_kind(1)");
	world.wait_until("the edit's reload", |output| settled(output) > before);
	reboot(&mut world, 2);
	assert_eq!(
		world.eval_global("rb_computed_kind_made()"),
		"\"/datum/rb_computed_kind\"",
		"the type that `new` made from a static set by a proc call after reboot:\n{}",
		world.output
	);
	ok("proc static made by new of a static that a proc call sets, after reboot");
	world.end();
}

const CASTS: &str = "/datum/rb_cast_item

/datum/rb_cast_other

var/datum/rb_cast_source = new /datum/rb_cast_other

/proc/rb_cast()
	return 0
";

fn global_cast_to_its_declared_type() {
	let dir = copy_of_the_world("global_cast_to_its_declared_type");
	std::fs::write(dir.join("casts.dm"), CASTS).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"casts.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	if world.eval_global("world.byond_version").parse::<u32>().unwrap() < 516 {
		world.end();
		return;
	}
	edit(
		&mut world,
		"casts.dm",
		"/proc/rb_cast()\n\treturn 0",
		"var/datum/rb_cast_item/rb_cast_global = astype(rb_cast_source)\n\n/proc/rb_cast()\n\treturn isnull(rb_cast_global)"
	);
	assert_eq!(world.eval_global("rb_cast()"), "1", "whether the new global is null:\n{}", world.output);
	reboot(&mut world, 2);
	assert_eq!(
		world.eval_global("rb_cast()"),
		"1",
		"whether the new global is null after reboot:\n{}",
		world.output
	);
	ok("new global cast by `astype()` to its declared type, live and after reboot");
	world.end();
}

const COMPUTED_CONFIG: &str = "var/datum/rb_computed_config/rb_computed_config

var/rb_computed_config_global = new rb_computed_config.item_type()

/proc/rb_computed_config_made()
	var/static/rb_computed_config_item = new rb_computed_config.item_type()
	var/datum/global_item = rb_computed_config_global
	return \"[rb_computed_config_item && rb_computed_config_item:type] [global_item && global_item.type]\"

/datum/rb_computed_config
	var/static/item_type = rb_computed_config_kind

var/rb_computed_config_kind = rb_computed_config_chosen()

/proc/rb_computed_config_chosen()
	return /datum/rb_computed_config_kind

/datum/rb_computed_config_kind
";

fn made_by_new_of_a_computed_static_through_a_var() {
	let dir = copy_of_the_world("made_by_new_of_a_computed_static_through_a_var");
	std::fs::write(dir.join("computed_config.dm"), COMPUTED_CONFIG).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"computed_config.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	let made = "\"/datum/rb_computed_config_kind /datum/rb_computed_config_kind\"";
	assert_eq!(world.eval_global("rb_computed_config_made()"), made);
	let held = |output: &str| output.matches("Hot reload requires a reboot:").count();
	let settled = |output: &str| reloads(output) + held(output);
	for made_by in ["global = new", "item = new"] {
		let before = settled(&world.output);
		world.marks.edit(
			"computed_config.dm",
			&format!("{made_by} rb_computed_config.item_type()"),
			&format!("{made_by} rb_computed_config.item_type(1)")
		);
		world.wait_until("the edit's reload", |output| settled(output) > before);
	}
	reboot(&mut world, 2);
	assert_eq!(
		world.eval_global("rb_computed_config_made()"),
		made,
		"the types that `new` made from a static read through a var, after reboot:\n{}",
		world.output
	);
	ok("global and proc static made by new of a computed static read through a var, after reboot");
	world.end();
}

const LOCAL_LIMIT: &str = "/proc/rb_local_limit()
	var/const/LIMIT = 5
	return \"[LIMIT] [LIMIT + 1]\"
";

fn local_const_changed() {
	let dir = copy_of_the_world("local_const_changed");
	std::fs::write(dir.join("local_limit.dm"), LOCAL_LIMIT).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"local_limit.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	assert_eq!(world.eval_global("rb_local_limit()"), "\"5 6\"");
	edit(&mut world, "local_limit.dm", "LIMIT = 5", "LIMIT = 10");
	assert_eq!(world.eval_global("rb_local_limit()"), "\"10 11\"");
	reboot(&mut world, 2);
	assert_eq!(world.eval_global("rb_local_limit()"), "\"10 11\"");
	ok("local const a reload changes, read alone and folded, after reboot");
	world.end();
}

#[track_caller]
fn finished_with(world: &mut World, what: &str) {
	let deadline = std::time::Instant::now() + test_support::dap::TIMEOUT;
	loop {
		world.has_event("progressEnd");
		let reloaded = |e: &Value| {
			e["event"] == "auxtoolsReload"
				&& e["body"]["state"] == "reloaded"
				&& e["body"]["summary"].as_str().is_some_and(|said| said.contains(what))
		};
		let ended = |e: &Value| e["event"] == "progressEnd" && e["body"]["progressId"] == "reloading";
		if world.events.iter().skip_while(|e| !reloaded(e)).any(ended) {
			return;
		}
		assert!(
			std::time::Instant::now() < deadline,
			"the launcher didn't finish with the reload of {}: {:?}",
			what,
			world.events
		);
		std::thread::sleep(std::time::Duration::from_millis(10));
	}
}

fn file_named_by_a_reload() {
	let dir = copy_of_the_world("file_named_by_a_reload");
	std::fs::create_dir_all(dir.join("fzdir")).unwrap();
	std::fs::copy(host_file("reload_new.dmi"), dir.join("fzdir/named.dmi")).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = "#define FILE_DIR fzdir\n".to_owned() + &std::fs::read_to_string(&dme).unwrap();
	std::fs::write(&dme, text).unwrap();
	let mut world = World::launch_as(
		json!({ "supportsProgressReporting": true }),
		dir.to_path_buf(),
		json!({ "hotReload": true })
	);
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	edit(
		&mut world,
		"main.dm",
		"/proc/icon_built()\n",
		"/proc/icon_named()\n\treturn 'named.dmi'\n\n/proc/icon_built()\n"
	);
	let shown = |world: &mut World| world.eval_global("\"[icon_named()] [jointext(icon_states(icon_named()), \",\")]\"");
	assert_eq!(shown(&mut world), "\"fzdir/named.dmi new1\"");
	ok("new resource lookup through FILE_DIR");
	finished_with(&mut world, "/icon_named");

	let before = world.count("Hot reload: reloaded fzdir/named.dmi");
	std::fs::copy(host_file("lamp_edited.dmi"), dir.join("fzdir/named.dmi")).unwrap();
	let saved = std::fs::File::options().write(true).open(dir.join("fzdir/named.dmi")).unwrap();
	saved.set_modified(std::time::SystemTime::now()).unwrap();
	world.wait_until("the resource reload", |output| {
		output.matches("Hot reload: reloaded fzdir/named.dmi").count() > before
	});
	assert_eq!(shown(&mut world), "\"fzdir/named.dmi on,off,new\"");
	ok("watching newly referenced resources");
	world.end();
}

fn resource_saved(world: &mut World, dir: &std::path::Path, version: &str) {
	let before = world.count("Hot reload: reloaded lamp.dmi");
	std::fs::copy(dir.join(version), dir.join("lamp.dmi")).unwrap();
	let saved = std::fs::File::options().write(true).open(dir.join("lamp.dmi")).unwrap();
	saved.set_modified(std::time::SystemTime::now()).unwrap();
	world.wait_until("the resource reload", |output| {
		output.matches("Hot reload: reloaded lamp.dmi").count() > before
	});
}

fn resource_files() {
	let dir = copy_of_the_world("resources");
	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.configure();
	world.wait_output("Hot reload active");
	assert_eq!(world.eval_global("make_lamps()"), "\"made\"");
	assert_eq!(world.eval_global("lamp_states()"), "\"on,off\"");

	resource_saved(&mut world, &dir, "lamp_edited.dmi");
	let new = "\"on,off,new\"";
	assert_eq!(world.eval_global("lamp_states()"), new);
	assert_eq!(world.eval_global("states_of(early_lamp.icon)"), new);
	ok("icon reload of the code and appearances that use it");

	world.drop_events("auxtoolsReload");
	world.run("reboot");
	world.wait_count("dap: world/New", 2);
	world.reloaded("lamp");
	assert_eq!(world.eval_global("states_of(early_lamp.icon)"), new);
	assert_eq!(world.eval_global("lamp_states()"), new);
	ok("saved icon restored after reboot");

	resource_saved(&mut world, &dir, "lamp_third.dmi");
	let third = "\"on,off,third\"";
	assert_eq!(world.eval_global("states_of(early_lamp.icon)"), third);
	assert_eq!(world.eval_global("lamp_states()"), third);
	ok("icon saved again after reboot");
	world.end();
}

fn read_ahead_stop(world: &mut World, file: &std::path::Path, line: u64) -> (u64, String) {
	world.set_breakpoints(file, json!([line]));
	world.run("nothing_to_run");
	let stop = world.stop();
	let text = world.local("text");
	world.resume_through("nothing_to_run");
	world.set_breakpoints(file, json!([]));
	(stop.line, text)
}

fn nothing_to_run() {
	let (mut world, _dir) = watched_world("nothing_to_run");
	let at = world.marks.clone();
	let file = at.file("stepping.dm");
	let names = ["nothing_macro", "nothing_var", "nothing_set", "nothing_code"];
	let lines: Vec<u64> = names.iter().map(|name| at.line(name)).collect();
	let placed = |world: &mut World| -> Vec<Value> {
		let set = world.set_breakpoints(&file, json!(lines));
		set.iter().map(|b| b["line"].clone()).collect()
	};
	let expected: Vec<Value> = lines.iter().map(|l| json!(l)).collect();
	assert_eq!(placed(&mut world), expected, "breakpoints moved off lines DreamMaker's code has");
	world.set_breakpoints(&file, json!([lines[0]]));
	world.run("nothing_to_run");
	world.stop_on("breakpoint", "/nothing_to_run", "nothing_macro");
	world.resume_through("nothing_to_run");
	let read_ahead = at.line("read_ahead_next");
	let before = read_ahead_stop(&mut world, &file, read_ahead);

	edit(&mut world, "stepping.dm", "\tn = 1 // @nothing_code\n", "\tn = 2 // @nothing_code\n");
	assert_eq!(world.eval_global("nothing_to_run()"), "2");
	assert_eq!(placed(&mut world), expected, "breakpoints moved off lines after the reload");
	world.set_breakpoints(&file, json!([lines[0]]));
	world.run("nothing_to_run");
	world.stop_on("breakpoint", "/nothing_to_run", "nothing_macro");
	world.resume_through("nothing_to_run");
	ok("breakpoints on nonexecutable lines after reload");
	let after = read_ahead_stop(&mut world, &file, read_ahead);
	assert_eq!(after, before, "a breakpoint DreamMaker records ahead stopped elsewhere after the reload");
	ok("breakpoint on a lookahead line after reload");
	world.end();
}

fn saved(world: &mut World, file: &str, old: &str, new: &str, outcome: &str) {
	let before = world.count(outcome);
	world.marks.edit(file, old, new);
	world.wait_until(outcome, |output| output.matches(outcome).count() > before);
}

const NOTHING: &str = "Hot reload: no changes to the running code";
const REFUSED: &str = "Hot reload requires a rebuild";

fn saves_with_nothing_to_reload() {
	let (mut world, _dir) = watched_world("nothing_to_reload");
	world.break_on(&["leaf_1"]);
	saved(&mut world, "stepping.dm", "/proc/leaf(x)\n", "/proc/leaf(x) // doubles x + 1\n", NOTHING);
	world.break_on(&["leaf_2"]);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	world.break_on(&[]);
	world.resume_through("middle");
	ok("breakpoint set after a save that changes only a comment");

	saved(
		&mut world,
		"errors.dm",
		"\tvar/zero = 0\n",
		"\tvar/zero = 0\n\tvar/list/broken = (\n",
		REFUSED
	);
	saved(&mut world, "errors.dm", "\tvar/list/broken = (\n", "", NOTHING);
	world.break_on(&["fail_caught_here"]);
	world.run("fail_caught_here");
	world.stop_on("breakpoint", "/fail_caught_here", "fail_caught_here");
	world.break_on(&[]);
	world.resume_through("fail_caught_here");
	ok("breakpoint set after a refused save is undone");
	world.end();
}

fn retry_after_a_refused_save() {
	let (mut world, _dir) = watched_world("retry_after_refusal");
	let at = world.marks.clone();
	let stepping = at.file("stepping.dm");
	world.break_on(&["middle_2"]);
	let before = world.count(REFUSED);
	at.edit("stepping.dm", "/proc/middle(a)\n", "// Calls leaf twice\n/proc/middle(a)\n");
	at.edit("errors.dm", "\tvar/zero = 0\n", "\tvar/zero = 0\n\tvar/list/broken = (\n");
	world.wait_until(REFUSED, |output| output.matches(REFUSED).count() > before);
	let middle_2 = at.line("middle_2");
	world.drop_events("breakpoint");
	let set = world.request(
		"setBreakpoints",
		json!({ "source": { "path": stepping }, "sourceModified": true, "breakpoints": [{ "line": middle_2 }] })
	);
	assert_eq!(set["breakpoints"][0]["message"], "pending hot reload", "{}", set);
	at.edit("errors.dm", "\tvar/list/broken = (\n", "");
	assert_eq!(moved(&mut world), (middle_2, true));
	world.run("middle");
	world.stop_on("breakpoint", "/middle", "middle_2");
	world.set_breakpoints(&stepping, json!([]));
	world.resume_through("middle");
	ok("breakpoint in a file saved with a refused one, after the refusal is fixed");
	world.end();
}

fn reboot_refusing_the_edits() {
	let (mut world, _dir) = watched_world("reboot_refusing_the_edits");
	let at = world.marks.clone();
	world.break_on(&["leaf_1"]);
	let leaf_1 = at.line("leaf_1");
	edit(&mut world, "stepping.dm", "/proc/leaf(x)\n", "/proc/leaf(x)\n\tvar/first = 1\n");
	assert_eq!(moved(&mut world), (leaf_1 + 1, true));
	saved(
		&mut world,
		"errors.dm",
		"\tvar/zero = 0\n",
		"\tvar/zero = 0\n\tvar/list/broken = (\n",
		REFUSED
	);
	let refusals = world.count(REFUSED);
	world.drop_events("breakpoint");
	world.run("reboot");
	world.wait_count("dap: world/New", 2);
	world.wait_until("the refusal after the reboot", |output| output.matches(REFUSED).count() > refusals);
	let told = world.event("breakpoint")["body"]["breakpoint"].clone();
	assert_eq!(
		(&told["verified"], &told["line"], &told["message"]),
		(&json!(false), &json!(leaf_1 + 1), &json!("pending hot reload")),
		"{}",
		told
	);
	world.run_through("middle");
	ok("breakpoint in an edit a reboot refuses waits");

	saved(&mut world, "errors.dm", "\tvar/list/broken = (\n", "", " ms");
	assert_eq!(moved(&mut world), (leaf_1 + 1, true));
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.break_on(&[]);
	world.resume_through("middle");
	ok("breakpoint in an edit a reboot refused, once a reload has it");
	world.end();
}

pub fn edits() {
	saved_without_reloading();
	behind_the_clients_back();
	starting_at_world_new();
	types_changed_before_reboot();
	saves_with_nothing_to_reload();
	retry_after_a_refused_save();
	edited_during_a_reboot();
	completion_after_a_rename();
	reboot_refusing_the_edits();
}

const STARTS_AT_AN_AREA: &str = "/area
	var/list/things = list()

var/global/list/rb_base = list(\"x\")

/datum/rb_reader/proc/tgm()
	var/static/list/default_list = rb_base
	var/static/list/wrapped_default_list = list(default_list)
	return wrapped_default_list[1] == default_list

/datum/rb_reader/proc/dmm()
	var/static/list/default_list = list(rb_base)
	return length(default_list)

/proc/rb_other(index)
	var/static/list/default_list = rb_base
	if (index == default_list)
		return 1
	return index == default_list

/datum/rb_flags
	var/static/auto_reset = TRUE
";

fn initializers_after_reboot_at_an_area() {
	let dir = copy_of_the_world("initializers_after_reboot_at_an_area");
	std::fs::write(dir.join("area_inits.dm"), STARTS_AT_AN_AREA).unwrap();
	std::fs::write(dir.join("area_inits.dmm"), "\"a\" = (/turf,/area)\n\n(1,1,1) = {\"\na\n\"}\n").unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"area_inits.dm\"\n#include \"area_inits.dmm\"\n";
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);

	let mut world = World::launch_in(dir.to_path_buf(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	const VALUES: &str = "\"[(new /datum/rb_flags).auto_reset] [(new /datum/rb_reader).tgm()] [(new /datum/rb_reader).dmm()] [rb_other(rb_base)]\"";
	edit_held(&mut world, "area_inits.dm", "auto_reset = TRUE", "auto_reset = FALSE");
	assert_eq!(world.eval_global(VALUES), "\"1 1 1 1\"");
	reboot(&mut world, 2);
	assert_eq!(world.eval_global(VALUES), "\"0 1 1 1\"");
	assert_eq!(held_changes(&mut world), []);
	assert!(!world.output.contains("global initialization has already run"), "{}", world.output);
	ok("held initializers after area startup");
	world.end();
}

fn proc_moved_to_another_file() {
	let (mut world, dir) = watched_world("proc_moved");
	let (tool, line) = world.marks.place("sub_tool");
	let set = world.set_breakpoints(&tool, json!([line]));
	assert_eq!(set[0]["verified"], true, "{}", set[0]);
	const SUB_TOOL: &str = "/proc/sub_tool(n)\n\treturn n + 1 // @sub_tool\n";
	std::fs::write(dir.join("sub/moved.dm"), SUB_TOOL).unwrap();
	world.drop_events("breakpoint");
	edit(&mut world, "sub/tool.dm", SUB_TOOL, "#include \"moved.dm\"\n");
	let (moved, line) = world.marks.place("sub_tool");
	let told = world.event("breakpoint")["body"]["breakpoint"].clone();
	let path = told["source"]["path"].as_str().map(PathBuf::from);
	assert_eq!(
		(
			&told["id"],
			&told["verified"],
			&told["line"],
			path.as_deref().and_then(|path| path.file_name())
		),
		(&set[0]["id"], &json!(true), &json!(line), moved.file_name()),
		"{}",
		told
	);
	world.run_through("declares");
	world.run("tools");
	let stop = world.stop();
	assert_eq!((stop.proc.as_str(), stop.line), ("/sub_tool", line));
	world.resume_through("tools");
	ok("source breakpoint on a proc moved to another file");
	world.end();
}

pub fn files() {
	proc_moved_to_another_file();
	override_restored_before_map();
	initializers_after_reboot();
	initializers_after_reboot_at_an_area();
	initializer_added_to_a_world_without();
	initializer_naming_a_retargeted_path();
	static_naming_a_retargeted_path();
	static_folding_a_changed_const();
	static_reading_a_changed_default();
	static_reading_a_changed_colon_name();
	static_made_by_new_of_a_changed_type();
	static_reading_through_a_retyped_var();
	static_reading_through_a_typed_static();
	static_read_by_an_initializer_moved_into_a_block();
	statics_of_two_overrides_with_the_same_name();
	static_gained_beside_a_global_of_its_name();
	static_reading_through_a_retyped_static();
	static_made_by_new_of_a_static_moved_into_a_block();
	static_made_by_new_of_a_computed_static();
	global_cast_to_its_declared_type();
	made_by_new_of_a_computed_static_through_a_var();
	local_const_changed();
	file_named_by_a_reload();
	resource_files();
	nothing_to_run();
}

fn swapped_parameters(world: &mut World) {
	world.run_through("pair_sleeps");
	edit(world, "stepping.dm", "/proc/pair(first, second)\n", "/proc/pair(second, first)\n");
	world.run_to("pair", "/pair", "pair_return");
	assert_eq!((world.argument("first"), world.argument("second")), ("2".to_owned(), "1".to_owned()));
	let threads = world.threads();
	let (old, _) = (threads.iter())
		.find(|(_, name)| name.starts_with("/pair") && name.ends_with(", sleeping"))
		.unwrap_or_else(|| panic!("no sleeping /pair in {:?}", threads));
	let frame = world.frames(*old)[0]["id"].clone();
	let arguments = world.scope(&frame, "Arguments");
	let arguments = named(&world.variables(&arguments));
	assert_eq!((value_of(&arguments, "first"), value_of(&arguments, "second")), ("1", "2"));
	assert_eq!(world.evaluate_in(&frame, "first - second", "watch")["result"], "-1");
	assert_eq!(world.evaluate_in(&frame, "second = 5", "repl")["result"], "5");
	world.resume_through("pair");
	world.wait_output("dap: the old pair returned -4");
	assert!(world.output.contains("dap: the new pair returned 1\n"), "{}", world.output);
	ok("parameter names of a sleeping call after a reload swaps them");
}

fn renamed_parameter(world: &mut World) {
	world.run_through("lone_sleeps");
	edit(
		world,
		"stepping.dm",
		"(value)\n\twhile (lone_held)\n\t\tsleep(1)\n\treturn value //",
		"(amount)\n\twhile (lone_held)\n\t\tsleep(1)\n\treturn amount //"
	);
	world.run_to("lone", "/lone", "lone_return");
	assert_eq!(world.argument("amount"), "2");
	let threads = world.threads();
	let (old, _) = (threads.iter())
		.find(|(_, name)| name.starts_with("/lone") && name.ends_with(", sleeping"))
		.unwrap_or_else(|| panic!("no sleeping /lone in {:?}", threads));
	let frame = world.frames(*old)[0]["id"].clone();
	let arguments = world.scope(&frame, "Arguments");
	let arguments = named(&world.variables(&arguments));
	assert_eq!(value_of(&arguments, "value"), "1", "the sleeping call's arguments: {:?}", arguments);
	assert_eq!(world.evaluate_in(&frame, "value", "watch")["result"], "1");
	world.resume_through("lone");
	world.wait_output("dap: the old lone returned 1");
	assert!(world.output.contains("dap: the new lone returned 2\n"), "{}", world.output);
	ok("parameter names of a sleeping call after a reload renames one and keeps the code");
}

pub fn run() {
	let dir = copy_of_the_world("reloading");
	let mut world = World::launch_as(
		json!({ "supportsInvalidatedEvent": true }),
		dir.to_path_buf(),
		json!({ "hotReload": true })
	);
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");

	world.break_on(&["leaf_2"]);
	let before = world.marks.line("leaf_2");
	edit(
		&mut world,
		"stepping.dm",
		"/proc/leaf(x)\n",
		"/proc/above()\n\treturn 1\n\n/proc/leaf(x)\n"
	);
	assert_eq!(world.marks.line("leaf_2"), before + 3);
	world.break_on(&["leaf_2"]);
	assert!(
		!world.has_event("invalidated"),
		"a reload of a running world left something to ask for again"
	);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	assert_eq!(world.local("y"), "4");
	ok("client-remapped breakpoint stays on its statement");

	assert_eq!(world.count("new code for"), 0, "duplicate reload summary from the debug server");
	assert!(
		!world.output.contains("Debug Server:"),
		"the debug server's own output was passed on as well"
	);
	assert!(!world.output.contains("Pausing execution"), "a stop was announced in the console");
	ok("single delivery of server notifications");

	edit(
		&mut world,
		"stepping.dm",
		"\tvar/y = x + 1 // @leaf_1\n",
		"\tvar/y = x + 1 // @leaf_1\n\tvar/extra = 5 // @leaf_extra\n\ty += extra\n"
	);
	assert_eq!(world.event("invalidated")["body"]["areas"], json!(["stacks"]));
	let frames = world.frames(0);
	assert_eq!(frames[0]["name"], "/leaf (old code)");
	let locals = world.scope_values("Locals");
	assert_eq!(locals.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), [".", "y"]);
	ok("paused reload preserves old code and locals");

	assert_eq!(
		(frames[1]["name"].as_str().unwrap(), frames[1]["line"].as_u64().unwrap()),
		("/middle", world.marks.line("middle_1"))
	);
	ok("line-only reload updates frame locations");

	world.break_on(&[]);
	world.step_on("stepOut", "/middle", "middle_1");
	world.step_on("next", "/middle", "middle_2");
	assert_eq!(world.local("r"), "8");
	ok("active frame finishes old code");

	world.step_on("stepIn", "/leaf", "leaf_1");
	world.step_on("next", "/leaf", "leaf_extra");
	world.step("next");
	world.step_on("next", "/leaf", "leaf_2");
	let locals = world.scope_values("Locals");
	assert_eq!((value_of(&locals, "y"), value_of(&locals, "extra")), ("14", "5"));
	world.resume_through("middle");
	ok("new calls use reloaded code and locals");

	edit(
		&mut world,
		"stepping.dm",
		"/proc/sc_middle()\n",
		"/proc/sc_fresh()\n\tvar/a = 1 // @fresh_1\n\tvar/b = a + 1 // @fresh_2\n\treturn b // @fresh_3\n\n/proc/sc_middle()\n"
	);
	let stepping = world.marks.file("stepping.dm");
	let (fresh_1, fresh_3) = (world.marks.line("fresh_1"), world.marks.line("fresh_3"));
	let places = world.request(
		"breakpointLocations",
		json!({ "source": { "path": stepping }, "line": fresh_1 - 1, "endLine": fresh_3 + 3 })
	);
	let lines: Vec<u64> = places["breakpoints"]
		.as_array()
		.unwrap()
		.iter()
		.map(|place| place["line"].as_u64().unwrap())
		.collect();
	assert_eq!(lines, [fresh_1, world.marks.line("fresh_2"), fresh_3, world.marks.line("sc_middle_1")]);
	ok("breakpoint locations after proc additions");
	world.run_to("fresh", "/sc_fresh", "fresh_2");
	assert_eq!(world.local("a"), "1");
	world.step_on("next", "/sc_fresh", "fresh_3");
	assert_eq!(world.local("b"), "2");
	world.resume_through("fresh");
	ok("stepping into an added proc");

	let added = dir.join("added.dm");
	std::fs::write(&added, "/proc/sc_added()\n\tvar/a = 1\n\treturn a + 1\n").unwrap();
	std::fs::File::options()
		.write(true)
		.open(&added)
		.unwrap()
		.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
		.unwrap();
	let set = world.set_breakpoints(&added, json!([{ "line": 3 }]));
	assert_eq!(set[0]["verified"], false, "{}", set[0]);
	world.drop_events("breakpoint");
	edit(
		&mut world,
		"dap_test_host.dme",
		"#include \"main.dm\"\n",
		"#include \"main.dm\"\n#include \"added.dm\"\n"
	);
	let told = world.event("breakpoint")["body"]["breakpoint"].clone();
	assert_eq!(
		(&told["id"], &told["verified"], &told["line"]),
		(&set[0]["id"], &json!(true), &json!(3)),
		"{}",
		told
	);
	world.run("added");
	world.launcher.stop_at("breakpoint", "/sc_added", 3);
	world.set_breakpoints(&added, json!([]));
	world.resume_through("added");
	ok("breakpoint in a file a reload adds");

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
	ok("unrelated reload preserves hit counts");

	let set = world.function_breakpoints(&["/proc/leaf"]);
	assert_eq!(set[0]["verified"], true);
	edit(
		&mut world,
		"stepping.dm",
		"\tvar/extra = 5 // @leaf_extra\n",
		"\tvar/extra = 6 // @leaf_extra\n"
	);
	world.run("middle");
	for _ in 0..2 {
		world.stop_on("breakpoint", "/leaf", "leaf_1");
		world.resume();
	}
	world.finish("middle");
	edit(
		&mut world,
		"stepping.dm",
		"/proc/leaf(x)\n",
		"/proc/leaf(x)\n\tvar/first = 1 // @leaf_first\n\tfirst++\n"
	);
	world.run("middle");
	for _ in 0..2 {
		world.stop_on("breakpoint", "/leaf", "leaf_first");
		world.resume();
	}
	world.finish("middle");
	world.function_breakpoints(&[]);
	world.run_through("middle");
	ok("function breakpoint follows reloaded entry");

	let set = world.function_breakpoints(&["/proc/later"]);
	assert_eq!(set[0]["message"], "proc not found: /proc/later", "{}", set[0]);
	world.drop_events("breakpoint");
	edit(
		&mut world,
		"stepping.dm",
		"/proc/sc_middle()\n",
		"/proc/sc_later()\n\treturn later()\n\n/proc/later()\n\treturn 1 // @later_1\n\n/proc/sc_middle()\n"
	);
	let told = world.event("breakpoint")["body"]["breakpoint"].clone();
	assert_eq!(
		(&told["id"], &told["verified"], &told["line"]),
		(&set[0]["id"], &json!(true), &json!(world.marks.line("later_1"))),
		"{}",
		told
	);
	world.run("later");
	world.stop_on("breakpoint", "/later", "later_1");
	world.function_breakpoints(&[]);
	world.resume_through("later");
	ok("function breakpoint on a proc a reload adds");

	let set = world.function_breakpoints(&["/datum/thing/twice/bump#1"]);
	assert_eq!(set[0]["verified"], true, "{}", set[0]);
	const SECOND: &str = "/datum/thing/twice/bump(by)\n\treturn ..(by) + 100 // @twice_second\n\n";
	const AFTER: &str = "/proc/sc_twice()\n";
	world.drop_events("breakpoint");
	edit(&mut world, "stepping.dm", SECOND, "");
	let told = world.event("breakpoint")["body"]["breakpoint"].clone();
	assert_eq!((&told["id"], &told["verified"]), (&set[0]["id"], &json!(false)), "{}", told);
	edit(&mut world, "stepping.dm", AFTER, &format!("{}{}", SECOND, AFTER));
	let told = world.event("breakpoint")["body"]["breakpoint"].clone();
	assert_eq!((&told["id"], &told["verified"]), (&set[0]["id"], &json!(true)), "{}", told);
	world.run("twice");
	let stop = world.stop();
	assert_eq!(stop.line, world.marks.line("twice_second"), "stopped in {}", stop.proc);
	world.function_breakpoints(&[]);
	world.resume_through("twice");
	ok("function breakpoint on a proc a reload removes and adds again");

	let set = world.function_breakpoints(&["/datum/thing/twice/bump#1"]);
	assert_eq!(set[0]["verified"], true, "{}", set[0]);
	const INSERTED: &str = "/datum/thing/twice/bump(by)\n\treturn ..(by) + 10 // @twice_inserted\n\n";
	edit(&mut world, "stepping.dm", SECOND, &format!("{}{}", INSERTED, SECOND));
	world.run("twice");
	let stop = world.stop();
	assert_eq!(
		stop.line,
		world.marks.line("twice_inserted"),
		"stopped in {} at line {}",
		stop.proc,
		stop.line
	);
	world.function_breakpoints(&[]);
	world.resume_through("twice");
	ok("function breakpoint on an override number a reload gives another definition");
	edit(
		&mut world,
		"stepping.dm",
		"\tvar/y = x + 1 // @leaf_1\n\tvar/extra = 6 // @leaf_extra\n\ty += extra\n\treturn y * 2 // @leaf_2\n",
		"\tvar/renamed = x + 1 // @leaf_1\n\tvar/extra = 6 // @leaf_extra\n\trenamed += extra\n\treturn renamed * 2 // @leaf_2\n"
	);
	world.run_to("middle", "/leaf", "leaf_2");
	let locals = world.scope_values("Locals");
	assert_eq!(locals.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), [
		".", "first", "renamed", "extra"
	]);
	assert_eq!(world.eval("renamed + extra"), "16");
	world.resume_through("middle");
	ok("renamed local inspection and evaluation");

	swapped_parameters(&mut world);
	renamed_parameter(&mut world);

	world.catch_runtimes(true);
	edit(
		&mut world,
		"errors.dm",
		"\tvar/list/L = null\n\treturn L[1] // @fail_null\n",
		"\tvar/list/L = null\n\tvar/unused = 1\n\treturn L[1] // @fail_null\n"
	);
	world.run("fail_null");
	let stop = world.stop();
	assert_eq!(
		(stop.reason.as_str(), stop.proc.as_str(), stop.line),
		("exception", "/fail_null", world.marks.line("fail_null"))
	);
	world.resume_through("fail_null");
	world.catch_runtimes(false);
	ok("runtime error location after reload");

	world.break_on(&["leaf_2"]);
	world.drop_events("auxtoolsReload");
	world.marks.edit("stepping.dm", "\tvar/n = 1\n", "\tvar/n = 1\n\tvar/list/broken = (\n");
	world.wait_output("Hot reload requires a rebuild");
	let refused = last_reload_state(&mut world);
	assert_eq!(refused["state"], "refused", "{}", refused);
	assert!(!refused["problems"].as_array().unwrap().is_empty(), "{}", refused);
	world.drop_events("breakpoint");
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	world.break_on(&[]);
	world.resume_through("middle");
	world.marks.edit("stepping.dm", "\tvar/list/broken = (\n", "");
	world.wait_output("Hot reload: no changes to the running code");
	let fixed = last_reload_state(&mut world);
	assert_eq!((&fixed["state"], &fixed["problems"]), (&json!("reloaded"), &json!([])), "{}", fixed);
	world.drop_events("breakpoint");
	ok("rejected reload preserves code and breakpoints");

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
	ok("restore reloads and breakpoints after reboot");

	let states = |world: &mut World, proc: &str| world.eval_global(&format!("jointext(icon_states({}()), \",\")", proc));
	edit(
		&mut world,
		"main.dm",
		"/proc/icon_reloaded()\n\treturn null\n",
		"/proc/icon_reloaded()\n\treturn 'reload_built_2.dmi'\n"
	);
	assert_eq!(states(&mut world, "icon_reloaded"), "\"second1\"");
	reboot(&mut world, 3);
	assert_eq!(states(&mut world, "icon_reloaded"), "\"second1\"");
	let after = world.output.rsplit("The world restarted").next().unwrap_or_default().to_owned();
	assert!(
		!after.contains("applies when world/New starts"),
		"the reload waited for world/New:\n{}",
		after
	);
	ok("restored reload names a built resource");

	edit(
		&mut world,
		"main.dm",
		"/proc/icon_built()\n",
		"/proc/icon_new()\n\treturn 'reload_new.dmi'\n\n/proc/icon_built()\n"
	);
	assert_eq!(states(&mut world, "icon_new"), "\"new1\"");
	reboot(&mut world, 4);
	let after = world.output.rsplit("The world restarted").next().unwrap_or_default().to_owned();
	assert!(
		after.contains("these resource files need the loaded cache: reload_new.dmi. The reload applies when world/New starts"),
		"the launcher said:\n{}",
		after
	);
	assert_eq!(states(&mut world, "icon_new"), "\"new1\"");
	assert_eq!(states(&mut world, "icon_reloaded"), "\"second1\"");
	assert_eq!(states(&mut world, "icon_built"), "\"built1,built2\"");
	ok("restored reload naming a new resource waits for world/New");
	world.end();
}
