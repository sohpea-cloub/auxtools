//! What a session is before and around the debugging: what the launcher says
//! it can do, what it refuses, a world run without debugging, and the end.

use std::time::{Duration, Instant};

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
		"exceptionBreakpointFilters",
		"supportTerminateDebuggee",
		"supportsConditionalBreakpoints",
		"supportsConfigurationDoneRequest",
		"supportsExceptionFilterOptions",
		"supportsExceptionInfoRequest",
		"supportsFunctionBreakpoints",
		"supportsHitConditionalBreakpoints",
		"supportsLogPoints",
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
	assert_eq!(defaults, [true, false], "runtime errors nothing catches pause by default, caught ones don't");
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
	assert!(why.contains("there's no") && why.contains("not_built.dmb"), "launching a world that isn't built said {:?}", why);
	ok("launches with no world named, no BYOND and no .dmb are refused, each with why");

	let why = launcher.refused("attach", json!({}));
	assert!(why.contains("processId"), "attaching to nothing said {:?}", why);
	let why = launcher.refused("attach", json!({ "processId": 4_000_000_000u32 }));
	assert!(why.contains("there's no process"), "attaching to no process said {:?}", why);
	let why = launcher.refused("attach", json!({ "processId": std::process::id() }));
	assert!(why.contains("isn't a world with this version of auxtools' debugger"), "attaching to this test said {:?}", why);
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
	world.end();
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

	let why = world.refused("pause", json!({ "threadId": 0 }));
	assert_eq!(why, "the world runs without debugging");
	world.run_through("middle");
	ok("a world run without debugging isn't paused: the client would never hear it had been");
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
	let list: Value = serde_json::from_str(&client::launcher_command(&["--list"]).1).unwrap();
	assert!(
		!list
			.as_array()
			.unwrap()
			.iter()
			.any(|w| w["dmb"].as_str().is_some_and(|dmb| dmb.ends_with("dap_test_host.dmb"))),
		"the world outlived its session: {}",
		list
	);
	ok("disconnecting ends the world the launcher started");

	// Asked to leave it running, the launcher can't: it says so
	let mut world = World::start();
	world.try_request("disconnect", json!({ "terminateDebuggee": false }));
	assert!(
		world.output.contains("The world was started by this session, and ends with it"),
		"the launcher didn't say the world would end"
	);
	assert!(world.launcher.child.wait().unwrap().success());
	let list: Value = serde_json::from_str(&client::launcher_command(&["--list"]).1).unwrap();
	assert_eq!(list.as_array().unwrap().len(), 0, "the world outlived its session: {}", list);
	ok("a world the launcher started ends with its session even when asked not to, and the console says why");
}
