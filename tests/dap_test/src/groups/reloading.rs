use std::{path::PathBuf, process::Command, time::Duration};

use serde_json::{json, Value};

use crate::{
	client::{launch_arguments, value_of},
	ok,
	paths::{self, ByondCommand},
	world::{host_file, World}
};

pub fn copy_of_the_world(name: &str) -> PathBuf {
	fn copy(from: &std::path::Path, to: &std::path::Path) {
		std::fs::create_dir_all(to).unwrap();
		for entry in std::fs::read_dir(from).unwrap() {
			let path = entry.unwrap().path();
			let copy_to = to.join(path.file_name().unwrap());
			if path.is_dir() {
				copy(&path, &copy_to);
			} else if path
				.extension()
				.is_some_and(|e| ["dm", "dme", "dmb", "rsc", "dmi"].contains(&e.to_str().unwrap_or_default()))
			{
				std::fs::copy(&path, copy_to).unwrap();
			}
		}
	}
	let dir = std::env::temp_dir().join(format!("dap_test_{}-{}", name, std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	copy(&host_file(""), &dir);
	dir
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

fn watched_world(name: &str) -> (World, PathBuf) {
	let dir = copy_of_the_world(name);
	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	(world, dir)
}

fn built_in_taken_away() {
	let (mut world, dir) = watched_world("built_in_taken_away");
	edit(
		&mut world,
		"main.dm",
		"/proc/icon_built()\n",
		"/obj/layered\n\tlayer = 7\n\n/proc/icon_built()\n"
	);
	assert_eq!(world.eval_global("(new /obj/layered).layer"), "7");
	edit(&mut world, "main.dm", "\tlayer = 7\n", "\t// layer = 7\n");
	assert_eq!(world.eval_global("(new /obj/layered).layer"), "3");
	ok("built-in-only override removal");
	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}

fn behind_the_clients_back() {
	let dir = copy_of_the_world("reloading");
	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	let at = world.marks.clone();
	assert_eq!(world.eval_global("TWICE(21)"), "42");

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

	let pid = crate::client::listed_world("dap_test_host.dmb");
	let mut other = World::attach_in(dir.clone(), pid);
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
	let _ = std::fs::remove_dir_all(&dir);
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
	assert!(said.contains("source macros unavailable"), "{}", said);
	ok("missing-macro diagnostic without hot reload");
	world.end();
}

fn starting_at_world_new() {
	let dir = std::env::temp_dir().join(format!("dap_test_world_new-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	std::fs::write(dir.join("dap_test_host.dme"), "#include \"bare.dm\"\n").unwrap();
	std::fs::write(
		dir.join("bare.dm"),
		"/world/New()\n\tworld.log << \"dap: world/New\"\n\n/proc/icon_new()\n\treturn null\n"
	)
	.unwrap();
	std::fs::copy(host_file("reload_new.dmi"), dir.join("reload_new.dmi")).unwrap();
	let built = Command::new(crate::paths::find_dm())
		.with_byond_paths()
		.arg(dir.join("dap_test_host.dme"))
		.status()
		.unwrap();
	assert!(built.success(), "dreammaker build of {} failed", dir.display());

	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
	world.configure();
	world.wait_output("Hot reload active");
	edit(&mut world, "bare.dm", "\treturn null\n", "\treturn 'reload_new.dmi'\n");
	assert_eq!(world.eval_global("jointext(icon_states(icon_new()), \",\")"), "\"new1\"");
	assert!(
		!world.output.contains("applies as world/New starts"),
		"the reload waited for world/New:\n{}",
		world.output
	);
	ok("resource reload after world/New startup");
	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}

fn parent_changed_alone() {
	let (mut world, dir) = watched_world("parent_changed_alone");
	edit(
		&mut world,
		"main.dm",
		"/proc/icon_built()\n",
		"/datum/pa\n\tvar/x = 1\n\n/datum/pb\n\tvar/x = 2\n\n/datum/pt\n\tparent_type = /datum/pa\n\n/proc/icon_built()\n"
	);
	assert_eq!(world.eval_global("(new /datum/pt).x"), "1");
	edit(&mut world, "main.dm", "\tparent_type = /datum/pa\n", "\tparent_type = /datum/pb\n");
	assert_eq!(world.eval_global("(new /datum/pt).x"), "2");
	assert_eq!(world.eval_global("istype(new /datum/pt, /datum/pb)"), "1");
	ok("parent-only reload");
	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}

fn types_changed_before_reboot() {
	let (mut world, dir) = watched_world("types_changed_before_reboot");
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
	let _ = std::fs::remove_dir_all(&dir);
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
	let built = Command::new(crate::paths::find_dm()).with_byond_paths().arg(&dme).status().unwrap();
	assert!(built.success(), "dreammaker build of {} failed", dir.display());

	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
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
	let _ = std::fs::remove_dir_all(&dir);
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

fn held_until_reboot() {
	let dir = copy_of_the_world("held_until_reboot");
	std::fs::write(dir.join("held_b.dm"), "#define HELD_K 1\n\n/proc/held_b()\n\treturn 1\n").unwrap();
	std::fs::write(
		dir.join("held.dm"),
		"var/held_n = 1\nvar/held_k = HELD_K + 1\n\n/proc/held_value()\n\treturn 1\n"
	)
	.unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"held_b.dm\"\n#include \"held.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	let built = Command::new(crate::paths::find_dm()).with_byond_paths().arg(&dme).status().unwrap();
	assert!(built.success(), "dreammaker build of {} failed", dir.display());
	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	let values = "\"[held_n] [held_value()]\"";
	let held = |message: &str, until: &str| vec![(message.to_owned(), until.to_owned())];

	edit_held(
		&mut world,
		"held.dm",
		"var/held_n = 1\nvar/held_k = HELD_K + 1\n\n/proc/held_value()\n\treturn 1\n",
		"var/held_n = 2\nvar/held_k = HELD_K + 1\n\n/proc/held_value()\n\treturn 2\n"
	);
	assert_eq!(held_changes(&mut world), held("(global): held_n: changed initializer", "reboot"));
	assert!(world.output.contains("Hot reload requires a reboot:"), "{}", world.output);
	assert_eq!(world.eval_global(values), "\"1 1\"");
	ok("held save preserves running code");

	edit(&mut world, "held.dm", "var/held_n = 2\n", "var/held_n = 1\n");
	assert_eq!(world.eval_global(values), "\"1 2\"");
	assert_eq!(held_changes(&mut world), []);
	ok("reverted initializer releases pending edits");

	edit_held(&mut world, "held.dm", "var/held_n = 1\n", "var/held_n = 3\n");
	edit_held(&mut world, "held.dm", "\treturn 2\n", "\treturn 4\n");
	assert_eq!(world.eval_global(values), "\"1 2\"");
	reboot(&mut world, 2);
	assert_eq!(world.eval_global(values), "\"3 4\"");
	assert_eq!(held_changes(&mut world), []);
	ok("reboot applies held changes");

	edit_held(&mut world, "held.dm", "var/held_n = 3\n", "var/held_n = 5\n");
	edit_held(&mut world, "held_b.dm", "\treturn 1\n", "\treturn 6\n");
	assert_eq!(world.eval_global("held_b()"), "1");
	assert_eq!(held_changes(&mut world), held("(global): held_n: changed initializer", "reboot"));
	edit(&mut world, "held.dm", "var/held_n = 5\n", "var/held_n = 3\n");
	assert_eq!(world.eval_global("held_b()"), "6");
	assert_eq!(held_changes(&mut world), []);
	ok("held edits span files");

	edit_held(&mut world, "held_b.dm", "#define HELD_K 1\n", "#define HELD_K 2\n");
	assert_eq!(held_changes(&mut world), held("(global): held_k: changed initializer", "reboot"));
	edit_nothing_to_run(&mut world, "held_b.dm", "#define HELD_K 2\n", "#define HELD_K 1\n");
	assert_eq!(held_changes(&mut world), []);
	ok("reverted macro clears dependent problems");

	edit_held(&mut world, "held.dm", "var/held_n = 3\n", "var/held_n = 3\n\n/world\n\tfps = 25\n");
	assert_eq!(held_changes(&mut world), held("/world: fps: changed setting", "rebuild"));
	assert!(world.output.contains("Hot reload requires a rebuild:"), "{}", world.output);
	edit_nothing_to_run(&mut world, "held.dm", "\n\n/world\n\tfps = 25\n", "\n");
	assert_eq!(held_changes(&mut world), []);
	ok("world setting requires rebuild");
	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}

fn edit_nothing_to_run(world: &mut World, file: &str, old: &str, new: &str) {
	let before = world.count("Hot reload: no changes to the running code");
	world.marks.edit(file, old, new);
	world.wait_count("Hot reload: no changes to the running code", before + 1);
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
	let built = Command::new(crate::paths::find_dm()).with_byond_paths().arg(&dme).status().unwrap();
	assert!(built.success(), "dreammaker build of {} failed", dir.display());

	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
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
		world.output.contains("applies as world/New starts"),
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
	let _ = std::fs::remove_dir_all(&dir);
}

fn initializer_added_to_a_world_without() {
	let dir = std::env::temp_dir().join(format!("dap_test_no_inits-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	std::fs::write(dir.join("dap_test_host.dme"), "#include \"bare.dm\"\n").unwrap();
	std::fs::write(
		dir.join("bare.dm"),
		"var/bare_n = 1\n\n/world/New()\n\tworld.log << \"dap: world/New\"\n\n/proc/bare_reboot()\n\tspawn(1)\n\t\tworld.Reboot()\n"
	)
	.unwrap();
	let built = Command::new(crate::paths::find_dm())
		.with_byond_paths()
		.arg(dir.join("dap_test_host.dme"))
		.status()
		.unwrap();
	assert!(built.success(), "dreammaker build of {} failed", dir.display());

	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
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
	let _ = std::fs::remove_dir_all(&dir);
}

fn file_named_by_a_reload() {
	let dir = copy_of_the_world("file_named_by_a_reload");
	std::fs::create_dir_all(dir.join("fzdir")).unwrap();
	std::fs::copy(host_file("reload_new.dmi"), dir.join("fzdir/named.dmi")).unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = "#define FILE_DIR fzdir\n".to_owned() + &std::fs::read_to_string(&dme).unwrap();
	std::fs::write(&dme, text).unwrap();
	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
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
	let _ = std::fs::remove_dir_all(&dir);
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
	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
	world.configure();
	world.wait_output("Hot reload active");
	assert_eq!(world.eval_global("make_lamps()"), "\"made\"");
	assert_eq!(world.eval_global("lamp_states()"), "\"on,off\"");

	resource_saved(&mut world, &dir, "lamp_edited.dmi");
	let new = "\"on,off,new\"";
	assert_eq!(world.eval_global("lamp_states()"), new);
	assert_eq!(world.eval_global("states_of(early_lamp.icon)"), new);
	assert_eq!(world.eval_global("states_of(placed_lamp.icon)"), new);
	assert_eq!(world.eval_global("overlay_states()"), new);
	assert_eq!(world.eval_global("var/obj/lamp/L = new\nstates_of(L.icon)"), new);
	ok("icon reload on existing and new instances");

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
	let _ = std::fs::remove_dir_all(&dir);
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
	let (mut world, dir) = watched_world("nothing_to_run");
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
	let _ = std::fs::remove_dir_all(&dir);
}

const FLAGGED: &str = "/proc/flag_value()
	return 1

#ifdef FLAG_A
/proc/flagged()
	return \"a\"

/datum/flag_only
	var/n = 3
#else
/proc/flagged()
	return \"none\"
#endif

#ifdef FLAG_MAP
#include \"flag_map.dmm\"
#endif
";

fn flags_held(world: &mut World, flags: &[&str]) {
	let said = |output: &str| output.matches("Hot reload requires a reboot:").count() + output.matches("Hot reload requires a rebuild:").count();
	let before = said(&world.output);
	world.request("auxtoolsBuildFlags", json!({ "flags": flags }));
	world.wait_until("the held build flags", |output| said(output) > before);
}

fn held_for_build_flags() {
	let dir = copy_of_the_world("held_for_build_flags");
	std::fs::write(dir.join("flags.dm"), FLAGGED).unwrap();
	std::fs::write(dir.join("flag_map.dmm"), "\"a\" = (/turf,/area)\n\n(1,1,1) = {\"\na\n\"}\n").unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"flags.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	let build = |flags: &[&str]| {
		let built = Command::new(paths::find_dm()).with_byond_paths().args(flags).arg(&dme).status().unwrap();
		assert!(built.success(), "dreammaker build of {} failed", dir.display());
	};
	build(&[]);
	let more = json!({ "hotReload": true, "buildFlags": [] });
	let mut world = World::launch_in(dir.clone(), more.clone());
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	let added = |flag: &str| vec![(format!("build flags changed (added {})", flag), "reboot".to_owned())];

	flags_held(&mut world, &["FLAG_A"]);
	assert_eq!(held_changes(&mut world), added("FLAG_A"));
	assert_eq!(world.eval_global("flagged()"), "\"none\"");
	edit_held(&mut world, "flags.dm", "\treturn 1\n", "\treturn 2\n");
	assert_eq!(world.eval_global("flag_value()"), "1");
	ok("changed build flags hold reloads");

	let before = reloads(&world.output);
	world.request("auxtoolsBuildFlags", json!({ "flags": [] }));
	world.wait_until("the released reload", |output| reloads(output) > before);
	assert_eq!(world.eval_global("flag_value()"), "2");
	assert_eq!(held_changes(&mut world), []);
	ok("reverted build flags release held edits");

	flags_held(&mut world, &["FLAG_A"]);
	reboot(&mut world, 2);
	assert_eq!(
		world.eval_global("\"[flagged()] [(new /datum/flag_only).n] [flag_value()]\""),
		"\"a 3 2\""
	);
	assert_eq!(held_changes(&mut world), []);
	edit(&mut world, "flags.dm", "\treturn \"a\"\n", "\treturn \"b\"\n");
	assert_eq!(world.eval_global("flagged()"), "\"b\"");
	ok("reboot applies selected flags");

	flags_held(&mut world, &[]);
	assert_eq!(held_changes(&mut world), [(
		"build flags changed (removed FLAG_A)".to_owned(),
		"reboot".to_owned()
	)]);
	assert_eq!(world.eval_global("flagged()"), "\"b\"");
	reboot(&mut world, 3);
	assert_eq!(world.eval_global("flagged()"), "\"none\"");
	assert_eq!(held_changes(&mut world), []);
	ok("reboot applies removed flags");

	let said = |output: &str| output.matches("no changes to the running code").count();
	let before = said(&world.output);
	world.request("auxtoolsBuildFlags", json!({ "flags": ["FLAG_UNUSED"] }));
	world.wait_until("the unchanged code", |output| said(output) > before);
	assert_eq!(held_changes(&mut world), []);
	ok("unused flags apply immediately");

	flags_held(&mut world, &["FLAG_UNUSED", "FLAG_MAP"]);
	assert_eq!(held_changes(&mut world), [
		("build flags changed (added FLAG_MAP)".to_owned(), "reboot".to_owned()),
		("compiled maps changed (added flag_map.dmm)".to_owned(), "rebuild".to_owned())
	]);
	ok("map flags require rebuild");

	world.request("auxtoolsBuildFlags", json!({ "flags": ["FLAG_A"] }));
	build(&["-DFLAG_A"]);
	world.drop_events("process");
	world.drop_events("auxtoolsReload");
	world.request("restart", json!({ "arguments": launch_arguments(&paths::find_byond(), &dme, more) }));
	world.wait_count("dap: world/New", 4);
	world.wait_output("using the rebuilt .dmb");
	while world.event("auxtoolsReload")["body"]["state"] != "active" {}
	assert!(!world.has_event("process"), "the restart relaunched the world");
	assert_eq!(world.eval_global("flagged()"), "\"b\"");
	edit(&mut world, "flags.dm", "\treturn \"b\"\n", "\treturn \"c\"\n");
	assert_eq!(world.eval_global("flagged()"), "\"c\"");
	assert_eq!(held_changes(&mut world), []);
	ok("restart uses selected build flags");
	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}

fn default_names_a_new_proc() {
	let dir = copy_of_the_world("default_names_a_new_proc");
	std::fs::write(dir.join("named.dm"), "/datum/rl_named\n\tvar/verb_path\n").unwrap();
	let dme = dir.join("dap_test_host.dme");
	let text = std::fs::read_to_string(&dme).unwrap() + "#include \"named.dm\"\n";
	std::fs::write(&dme, text).unwrap();
	let built = Command::new(paths::find_dm()).with_byond_paths().arg(&dme).status().unwrap();
	assert!(built.success(), "dreammaker build of {} failed", dir.display());

	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload active");
	edit(
		&mut world,
		"named.dm",
		"\tvar/verb_path\n",
		"\tvar/verb_path\n\n/datum/rl_named/one\n\tverb_path = /client/proc/rl_named_verb\n\n/client/proc/rl_named_verb()\n\tset name = \
		 \"Named\"\n\treturn 5\n"
	);
	assert!(!world.output.contains("Hot reload failed"), "{}", world.output);
	assert_eq!(
		world.eval_global("\"[(new /datum/rl_named/one).verb_path == /client/proc/rl_named_verb]\""),
		"\"1\""
	);
	ok("new proc references in type defaults");
	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}

pub fn edits() {
	held_until_reboot();
	default_names_a_new_proc();
	held_for_build_flags();
	saved_without_reloading();
	built_in_taken_away();
	behind_the_clients_back();
	starting_at_world_new();
	parent_changed_alone();
	types_changed_before_reboot();
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
	let built = Command::new(crate::paths::find_dm()).with_byond_paths().arg(&dme).status().unwrap();
	assert!(built.success(), "dreammaker build of {} failed", dir.display());

	let mut world = World::launch_in(dir.clone(), json!({ "hotReload": true }));
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
	let _ = std::fs::remove_dir_all(&dir);
}

pub fn files() {
	override_restored_before_map();
	initializers_after_reboot();
	initializers_after_reboot_at_an_area();
	initializer_added_to_a_world_without();
	file_named_by_a_reload();
	resource_files();
	nothing_to_run();
}

pub fn run() {
	let dir = copy_of_the_world("reloading");
	let mut world = World::launch_as(json!({ "supportsInvalidatedEvent": true }), dir.clone(), json!({ "hotReload": true }));
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

	edit(&mut world, "main.dm", "var/ticks = 0\n", "var/ticks = 0\nvar/added_later = 12\n");
	assert_eq!(world.eval_global("added_later"), "12");
	assert_eq!(world.eval_global("added_later = ticks > 0 ? 13 : 0"), "13");
	assert_eq!(world.eval_global("global.added_later + 1"), "14");
	ok("new global evaluation and assignment");

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

	let before = world.count("Hot reload requires a rebuild");
	world.drop_events("auxtoolsReload");
	world
		.marks
		.edit("stepping.dm", "/mob/player\n", "/mob/refused\n\tckey = \"someone\"\n\n/mob/player\n");
	world.wait_count("Hot reload requires a rebuild", before + 1);
	let refused = last_reload_state(&mut world);
	assert_eq!(refused["state"], "refused", "{}", refused);
	edit_held(&mut world, "errors.dm", "return fail_null() + 1", "return fail_null() + 2");
	let still = last_reload_state(&mut world);
	assert_eq!(
		(&still["state"], &still["problems"]),
		(&refused["state"], &refused["problems"]),
		"{}",
		still
	);
	let reloaded = reloads(&world.output);
	world.marks.edit("stepping.dm", "/mob/refused\n\tckey = \"someone\"\n\n", "");
	world.wait_until("the held reload", |output| reloads(output) > reloaded);
	let last = world
		.output
		.lines()
		.rfind(|line| line.starts_with("Hot reload: ") && line.ends_with(" ms"))
		.unwrap()
		.to_owned();
	assert!(last.contains("/fail_deep"), "the fix's reload: {}", last);
	let fixed = last_reload_state(&mut world);
	assert_eq!(fixed["problems"], json!([]), "{}", fixed);
	edit(&mut world, "errors.dm", "return fail_null() + 2", "return fail_null() + 1");
	ok("refusal holds edits in other files");

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
		!after.contains("applies as world/New starts"),
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
		after.contains("resource files need the loaded cache (reload_new.dmi); reload applies as world/New starts"),
		"the launcher said:\n{}",
		after
	);
	assert_eq!(states(&mut world, "icon_new"), "\"new1\"");
	assert_eq!(states(&mut world, "icon_reloaded"), "\"second1\"");
	assert_eq!(states(&mut world, "icon_built"), "\"built1,built2\"");
	ok("restored reload naming a new resource waits for world/New");
	world.end();
	let _ = std::fs::remove_dir_all(&dir);
}
