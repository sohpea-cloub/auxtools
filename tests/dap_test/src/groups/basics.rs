//! What a session is before and around the debugging: what the launcher says
//! it can do, what it refuses, a world run without debugging, and the end.

use std::{
	path::PathBuf,
	time::{Duration, Instant}
};

use serde_json::{json, Value};

use crate::{
	client::{self, Launcher},
	ok, paths,
	world::{host_file, World}
};

pub fn run() {
	capabilities();
	refusals();
	without_debugging();
	output_and_exit();
	asked_to_end();
	told_along_the_way();
}

/// Waits for the world to be over, and returns how long that took
fn over(world: &mut World, since: std::time::Instant) -> std::time::Duration {
	world.event("terminated");
	let exited = world.event("exited");
	assert!(exited["body"]["exitCode"].is_number(), "no exit code: {}", exited);
	since.elapsed()
}

/// `terminate`: the world ends itself, with the shutdown code it has
fn asked_to_end() {
	let grace = std::time::Duration::from_secs(5);

	let mut world = World::start();
	let asked = std::time::Instant::now();
	world.request("terminate", json!({}));
	let took = over(&mut world, asked);
	assert_eq!(world.count("dap: world/Del\n"), 1, "the world's Del() didn't run once:\n{}", world.output);
	assert!(world.output.contains("Asked the world to end"), "the launcher didn't say what it did");
	assert!(
		took < grace && !world.output.contains("hasn't ended"),
		"the world was ended, {:?} on",
		took
	);
	ok("terminate has the world end itself: its world/Del() runs, and the client hears it's over");
	world.end();

	// Paused, it isn't going to run anything else
	let mut world = World::start();
	world.run_to("plain", "/sc_plain", "plain_1");
	let asked = std::time::Instant::now();
	world.request("terminate", json!({}));
	let took = over(&mut world, asked);
	assert_eq!(world.count("dap: world/Del\n"), 1, "the world's Del() didn't run once:\n{}", world.output);
	assert!(
		took < grace && !world.output.contains("hasn't ended"),
		"the world was ended, {:?} on",
		took
	);
	ok("terminate ends a paused world the same way");
	world.end();

	// A world that doesn't end when asked is ended
	let mut world = World::start();
	assert_eq!(world.eval_global("on_shutdown = \"refuse\""), "\"refuse\"");
	let asked = std::time::Instant::now();
	world.request("terminate", json!({}));
	world.wait_output("dap: world/Del\n");
	let took = over(&mut world, asked);
	assert!(took >= grace && took < grace * 2, "the world was ended after {:?}", took);
	assert!(
		!world.output.contains("connecting again"),
		"the launcher waited for the debugger of a world it was ending"
	);
	assert!(
		world.output.contains("The world hasn't ended in 5 seconds: ending its process"),
		"the launcher didn't say why:\n{}",
		world.output
	);
	ok("a world whose shutdown code doesn't end it is given five seconds, and then ended, which the console says");
	world.end();

	// Run without debugging, and attached to
	let mut world = World::launch(json!({ "noDebug": true }));
	world.configure();
	evaluate_when_connected(&mut world, "global_number");
	let mut other = World::attach(client::listed_world("dap_test_host.dmb"));
	other.request("configurationDone", json!({}));
	let asked = std::time::Instant::now();
	other.request("terminate", json!({}));
	let took = over(&mut other, asked);
	over(&mut world, asked);
	assert_eq!(world.count("dap: world/Del\n"), 1, "the world's Del() didn't run once:\n{}", world.output);
	assert!(took < grace, "the world was ended, {:?} on", took);
	ok("terminate from a session that attached ends the world for the one that started it without debugging too");
	other.end();
	world.end();
}

/// The ids of the progress events that have come, as `(start or end, id)`
fn progress(world: &mut World) -> Vec<(String, String)> {
	world.has_event("progressStart");
	let events: Vec<(String, String)> = world
		.events
		.iter()
		.filter(|e| e["event"] == "progressStart" || e["event"] == "progressEnd")
		.map(|e| {
			(
				e["event"].as_str().unwrap().to_owned(),
				e["body"]["progressId"].as_str().unwrap().to_owned()
			)
		})
		.collect();
	world.events.retain(|e| e["event"] != "progressStart" && e["event"] != "progressEnd");
	events
}

/// What a client is told without asking: which program it debugs, what's
/// taking a while, and where runtime errors were
fn told_along_the_way() {
	let pair = |event: &str, id: &str| (event.to_owned(), id.to_owned());

	// A client that shows progress
	let mut world = World::launch_as(json!({ "supportsProgressReporting": true }), host_file(""), json!({ "hotReload": true }));
	world.catch_runtimes(false);
	world.configure();
	world.wait_output("Hot reload: watching");
	let events = progress(&mut world);
	for id in ["starting", "reading"] {
		let (start, end) = (
			events.iter().position(|e| *e == pair("progressStart", id)),
			events.iter().position(|e| *e == pair("progressEnd", id))
		);
		assert!(start.is_some() && start < end, "{} wasn't shown from start to end: {:?}", id, events);
	}
	assert_eq!(events.len(), 4, "{:?}", events);
	ok("a client that shows progress is told while the world starts and while its code is read");

	let process = world.event("process");
	let pid = client::listed_world("dap_test_host.dmb");
	assert_eq!(process["body"]["systemProcessId"], pid);
	assert_eq!(process["body"]["startMethod"], "launch");
	assert!(process["body"]["name"].as_str().unwrap().ends_with("dap_test_host.dmb"), "{}", process);
	let mut other = Launcher::start();
	other.initialize();
	other.attach(pid, json!({}));
	let process = other.event("process");
	assert_eq!(
		(&process["body"]["systemProcessId"], &process["body"]["startMethod"]),
		(&json!(pid), &json!("attach"))
	);
	other.disconnect(json!({}));
	ok("a client is told which process it debugs, launched or attached to");

	world.run("reboot");
	world.wait_count("dap: world/New", 2);
	world.wait_output("The world restarted");
	let events = progress(&mut world);
	assert_eq!(
		events[..2],
		[pair("progressStart", "restarting"), pair("progressEnd", "restarting")],
		"{:?}",
		events
	);
	ok("a client that shows progress is told while the world reboots");

	// BYOND's report of a runtime error says where it was, which is passed
	// on as a place
	let at = world.marks.clone();
	world.run_through("fail_null");
	let place = world
		.outputs
		.iter()
		.find(|output| output["output"].as_str().unwrap().contains("source file: errors.dm,"))
		.unwrap_or_else(|| panic!("the world didn't say where its runtime error was:\n{}", world.output));
	assert_eq!(place["line"], at.line("fail_null"));
	assert_eq!(std::path::Path::new(place["source"]["path"].as_str().unwrap()), at.file("errors.dm"));
	ok("a runtime error the world logs comes with its file and line, for the client to make a link of");
	world.end();

	// A client that says nothing of progress isn't told of any
	let mut world = World::start();
	assert_eq!(progress(&mut world), Vec::<(String, String)>::new());
	world.end();
}

fn capabilities() {
	let mut launcher = Launcher::start();
	let capabilities = launcher.initialize();
	let mut offered: Vec<&str> = capabilities
		.as_object()
		.unwrap()
		.iter()
		.filter(|(_, value)| !value.is_null() && **value != json!(false))
		.map(|(name, _)| name.as_str())
		.collect();
	offered.sort_unstable();
	assert_eq!(offered, [
		"completionTriggerCharacters",
		"exceptionBreakpointFilters",
		"supportTerminateDebuggee",
		"supportsBreakpointLocationsRequest",
		"supportsCompletionsRequest",
		"supportsConditionalBreakpoints",
		"supportsConfigurationDoneRequest",
		"supportsDataBreakpoints",
		"supportsDisassembleRequest",
		"supportsEvaluateForHovers",
		"supportsExceptionFilterOptions",
		"supportsExceptionInfoRequest",
		"supportsFunctionBreakpoints",
		"supportsHitConditionalBreakpoints",
		"supportsInstructionBreakpoints",
		"supportsLoadedSourcesRequest",
		"supportsLogPoints",
		"supportsSetExpression",
		"supportsSetVariable",
		"supportsSteppingGranularity",
		"supportsTerminateRequest",
		"supportsValueFormattingOptions"
	]);
	let filters: Vec<&str> = capabilities["exceptionBreakpointFilters"]
		.as_array()
		.unwrap()
		.iter()
		.filter_map(|f| f["filter"].as_str())
		.collect();
	assert_eq!(filters, ["runtimes", "caught"]);
	let defaults: Vec<bool> = capabilities["exceptionBreakpointFilters"]
		.as_array()
		.unwrap()
		.iter()
		.map(|f| f["default"] == true && f["supportsCondition"] == true)
		.collect();
	assert_eq!(
		defaults,
		[true, false],
		"runtime errors nothing catches pause by default, caught ones don't"
	);
	ok("the launcher says what it can do, and nothing it can't");

	let why = launcher.refused("frobnicate", json!({}));
	assert_eq!(why, "frobnicate isn't supported");
	ok("a request it doesn't know is refused by name");
	launcher.disconnect(json!({}));
}

fn refusals() {
	let byond = paths::find_byond();
	let mut launcher = Launcher::start();
	launcher.initialize();

	let why = launcher.refused("launch", json!({ "byondPath": byond }));
	assert!(why.contains("say which world to run"), "launching nothing said {:?}", why);
	let why = launcher.refused("launch", json!({ "dme": host_file("dap_test_host.dme"), "byondPath": "/nowhere/byond" }));
	assert!(why.contains("there's no BYOND in"), "launching with no BYOND said {:?}", why);
	let why = launcher.refused("launch", json!({ "dme": host_file("not_built.dme"), "byondPath": byond }));
	assert!(
		why.contains("there's no") && why.contains("not_built.dmb"),
		"launching a world that isn't built said {:?}",
		why
	);
	ok("launches with no world named, no BYOND and no .dmb are refused, each with why");

	let why = launcher.refused("attach", json!({}));
	assert!(why.contains("processId"), "attaching to nothing said {:?}", why);
	let why = launcher.refused("attach", json!({ "processId": 4_000_000_000u32 }));
	assert!(why.contains("there's no process"), "attaching to no process said {:?}", why);
	let why = launcher.refused("attach", json!({ "processId": std::process::id() }));
	assert!(
		why.contains("isn't a world with this version of auxtools' debugger"),
		"attaching to this test said {:?}",
		why
	);
	ok("attaching to nothing, to no process and to a program that isn't a world is refused");
	launcher.disconnect(json!({}));

	let mut world = World::start();
	let why = world.refused("launch", json!({ "dme": host_file("dap_test_host.dme"), "byondPath": byond }));
	assert_eq!(why, "there's a world already");
	let why = world.refused("attach", json!({ "processId": std::process::id() }));
	assert_eq!(why, "there's a world already");
	ok("a session has one world");

	let stddef = world.request("source", json!({ "sourceReference": 1 }));
	assert!(
		stddef["content"].as_str().unwrap().contains("Standard definitions"),
		"stddef.dm's source is something else"
	);
	let why = world.refused("source", json!({ "sourceReference": 7 }));
	assert_eq!(why, "there's no such source");
	ok("stddef.dm's text is the one source the launcher has");

	loaded_sources(&mut world);
	files_another_environment_names(&mut world);
	world.end();
}

/// `loadedSources`: the world's files, each once, then stddef.dm
fn loaded_sources(world: &mut World) {
	let sources = world.request("loadedSources", json!({}))["sources"].as_array().unwrap().clone();
	let (files, references): (Vec<&Value>, Vec<&Value>) = sources.iter().partition(|source| source["path"].is_string());
	let paths: Vec<PathBuf> = files.iter().map(|source| PathBuf::from(source["path"].as_str().unwrap())).collect();
	// (lib/tool.dm is just "tool.dm" to the world: where it is is worked out)
	let expected: Vec<PathBuf> = [
		"errors.dm",
		"lib/tool.dm",
		"main.dm",
		"stepping.dm",
		"sub/tool.dm",
		"values.dm",
		"watching.dm"
	]
	.iter()
	.map(|name| world.marks.file(name))
	.collect();
	assert_eq!(paths, expected, "{:?}", sources);
	for (source, path) in files.iter().zip(&paths) {
		assert!(path.is_absolute() && path.is_file(), "{}", path.display());
		assert_eq!(source["name"].as_str(), path.file_name().and_then(|name| name.to_str()));
	}
	ok("loadedSources lists the world's files, sorted and each once, by name and an absolute path that's there");

	assert_eq!(references.len(), 1, "{:?}", sources);
	assert_eq!(
		(&references[0]["name"], &references[0]["sourceReference"]),
		(&json!("stddef.dm"), &json!(1))
	);
	assert_eq!(sources.last(), Some(references[0]));
	ok("and stddef.dm last, by the reference its text is given for");
}

/// DreamMaker names a file that a .dme other than the world's includes as
/// that .dme does: lib/tool.dm, which lib/lib.dme includes, is "tool.dm" to
/// the world. sub/tool.dm is "sub/tool.dm", with code on the same lines.
fn files_another_environment_names(world: &mut World) {
	assert_eq!(world.marks.line("lib_tool"), world.marks.line("sub_tool"));
	let source_path = |world: &mut World| PathBuf::from(world.top_frame()["source"]["path"].as_str().unwrap());

	// lib_tool() is called first: a breakpoint taken for the end of the
	// file's name would stop there
	world.run_to("tools", "/sub_tool", "sub_tool");
	assert_eq!(source_path(world), world.marks.file("sub/tool.dm"));
	world.resume_through("tools");
	ok("a breakpoint in a file isn't one in a file whose name in the world its path ends with");

	world.run_to("tools", "/lib_tool", "lib_tool");
	assert_eq!(source_path(world), world.marks.file("lib/tool.dm"));
	world.resume_through("tools");
	ok("a file another .dme includes takes breakpoints, and a frame in it is in the file where it is");
}

/// Evaluates in a world that may not be connected yet
fn evaluate_when_connected(launcher: &mut Launcher, expression: &str) -> String {
	let deadline = Instant::now() + client::TIMEOUT;
	loop {
		let response = launcher.try_request("evaluate", json!({ "expression": expression, "context": "repl" }));
		if response["success"] == true {
			return response["body"]["result"].as_str().unwrap().to_owned();
		}
		assert!(Instant::now() < deadline, "the world never connected: {}", response);
		std::thread::sleep(Duration::from_millis(100));
	}
}

fn without_debugging() {
	let mut world = World::launch(json!({ "noDebug": true }));
	let set = world.try_break_if(&[("leaf_1", "")]);
	assert_eq!(set[0]["verified"], false);
	assert_eq!(set[0]["message"], "the world runs without debugging");
	let set = world.function_breakpoints(&["/proc/leaf"]);
	assert_eq!(set[0]["verified"], false);
	ok("a world run without debugging takes no breakpoints, and says so");

	world.configure();
	assert_eq!(evaluate_when_connected(&mut world, "global_number + 1"), "6");
	world.run_through("middle");
	ok("a world run without debugging runs, and still evaluates expressions");

	let leaf_1 = world.marks.line("leaf_1");
	let stepping = world.marks.file("stepping.dm");
	let places = world.request("breakpointLocations", json!({ "source": { "path": stepping }, "line": leaf_1 }));
	assert_eq!(places["breakpoints"], json!([]));
	let sources = world.request("loadedSources", json!({}));
	assert_eq!(sources["sources"], json!([]));
	ok("a world run without debugging has no places for breakpoints and no sources to show, as it takes no breakpoints");

	let why = world.refused("pause", json!({ "threadId": 0 }));
	assert_eq!(why, "the world runs without debugging");
	let watched = world.request("dataBreakpointInfo", json!({ "name": "global_number" }));
	assert_eq!(
		(&watched["dataId"], &watched["description"]),
		(&json!(null), &json!("the world runs without debugging"))
	);
	world.run_through("middle");
	ok("a world run without debugging isn't paused or watched: the client would never hear it had been");
	world.end();
}

fn output_and_exit() {
	let mut world = World::start();
	world.run_through("stderr");
	let category = |world: &World, text: &str| -> String {
		world
			.categories
			.iter()
			.find(|(_, output)| output.contains(text))
			.map(|(category, _)| category.clone())
			.unwrap_or_else(|| panic!("{:?} wasn't passed on", text))
	};
	let log = category(&world, "dap: to the log");
	assert!(log == "stdout" || log == "stderr", "the world's log came as {:?}", log);
	ok("what the world prints comes as its output");

	let pid = client::listed_world("dap_test_host.dmb");
	world.run("quit");
	world.event("terminated");
	let exited = world.event("exited");
	assert!(exited["body"]["exitCode"].is_number(), "no exit code: {}", exited);
	assert_eq!(category(&world, "The world exited"), "console");
	ok("a world that ends is terminated and exited, with its exit code, and the launcher says so in the console");
	world.end();

	// A world that's still running ends with its session
	let world = World::start();
	let pid_again = client::listed_world("dap_test_host.dmb");
	assert_ne!(pid, pid_again, "the first world is still there");
	world.end();
	let list = client::listed_worlds();
	assert!(list.is_empty(), "the world outlived its session: {:?}", list);
	ok("disconnecting ends the world the launcher started");

	// Asked to leave it running, the launcher can't: it says so
	let mut world = World::start();
	world.try_request("disconnect", json!({ "terminateDebuggee": false }));
	assert!(
		world.output.contains("The world was started by this session, and ends with it"),
		"the launcher didn't say the world would end"
	);
	assert!(world.launcher.child.wait().unwrap().success());
	let list = client::listed_worlds();
	assert!(list.is_empty(), "the world outlived its session: {:?}", list);
	ok("a world the launcher started ends with its session even when asked not to, and the console says why");
}
