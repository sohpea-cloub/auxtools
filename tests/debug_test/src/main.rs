//! Runs a tiny world with the debug server loaded into it, as the launcher
//! does, and pokes it the way the launcher would, to make sure the debugger
//! can't take the game down.

// Shared with hot_reload_test, which uses the parts this doesn't
#[allow(dead_code)]
mod client;

// test_runner already knows how to find BYOND, and the debug server's message
// types aren't exported from its cdylib. Both are pulled in as source, and
// each has bits this test never touches.
#[allow(dead_code)]
#[path = "../../test_runner/src/paths.rs"]
mod paths;
#[allow(dead_code)]
#[path = "../../../debug_server/src/server_types.rs"]
mod server_types;

use std::{path::PathBuf, process::Command, time::Duration};

use client::{Client, World};
use paths::ByondCommand;
use server_types::{BreakpointReason, BreakpointSetResult, InstructionRef, ProcRef, Request, Response};

// The debug server names global procs without the /proc part
const BREAKPOINT_TARGET: &str = "/breakpoint_target";
const TICK_MARKER: &str = "/tick_marker";

fn host_file(name: &str) -> PathBuf {
	let mut path = std::env::current_dir().unwrap();
	path.push("tests/debug_test_host");
	path.push(name);
	path
}

fn start_of(proc: &str) -> InstructionRef {
	InstructionRef {
		proc: ProcRef {
			path: proc.to_owned(),
			override_id: 0
		},
		offset: 0
	}
}

fn set_breakpoint(client: &mut Client, proc: &str, condition: Option<&str>) {
	let response = client.request(Request::BreakpointSet {
		instruction: start_of(proc),
		condition: condition.map(str::to_owned)
	});

	assert!(
		matches!(response, Response::BreakpointSet {
			result: BreakpointSetResult::Success { .. }
		}),
		"couldn't set a breakpoint on {}: {:?}",
		proc,
		response
	);
}

fn eval(client: &mut Client, frame_id: Option<u32>, command: &str) -> String {
	let response = client.request(Request::Eval {
		frame_id,
		command: command.to_owned(),
		context: None
	});

	match response {
		Response::Eval(eval) => eval.value,
		other => panic!("expected the result of {:?}, got {:?}", command, other)
	}
}

/// The proc the world is paused in.
fn paused_in(client: &mut Client) -> String {
	match client.request(Request::Stacks) {
		Response::Stacks { stacks } => stacks.into_iter().next().expect("paused with no stacks").name,
		other => panic!("expected Stacks, got {:?}", other)
	}
}

/// Evals `command` in `frame_id` and returns its one-line text and its expanded
/// fields as "name = value" strings.
fn eval_fields(client: &mut Client, frame_id: u32, command: &str) -> (String, Vec<String>) {
	let response = client.request(Request::Eval {
		frame_id: Some(frame_id),
		command: command.to_owned(),
		context: None
	});
	let Response::Eval(eval) = response else {
		panic!("expected the result of {:?}, got {:?}", command, response);
	};
	let refs = eval.variables.unwrap_or_else(|| panic!("{} should be expandable", command));
	let vars = match client.request(Request::Variables { vars: refs }) {
		Response::Variables { vars } => vars,
		other => panic!("expected Variables, got {:?}", other)
	};
	let fields = vars.into_iter().map(|var| format!("{} = {}", var.name, var.value)).collect();
	(eval.value, fields)
}

fn expect_breakpoint_in(client: &mut Client, proc: &str) {
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_in(client), proc);
}

fn pause(client: &mut Client) {
	let response = client.request(Request::Pause);
	assert!(matches!(response, Response::Ack), "expected Ack for Pause, got {:?}", response);
	assert_eq!(client.expect_pause(), BreakpointReason::Pause);
}

fn main() {
	let res = Command::new(paths::find_dm())
		.with_byond_paths()
		.arg(host_file("debug_test_host.dme"))
		.status()
		.unwrap();
	assert!(res.success(), "dreammaker build failed");

	let world = World::start(&host_file("debug_test_host.dmb"), &[]);

	let dis_command = format!("#dis {}", BREAKPOINT_TARGET);

	let mut client = world.connect();
	let clean_disassembly = eval(&mut client, None, &dis_command);
	assert_eq!(eval(&mut client, None, "helper.greet(\"you\")"), "\"hi you\"");
	println!("ok: evaluating a verb call");
	set_breakpoint(&mut client, BREAKPOINT_TARGET, None);
	expect_breakpoint_in(&mut client, BREAKPOINT_TARGET);
	println!("ok: breakpoint hit");

	// VS Code re-runs every watch expression on every stop, so this is the
	// easy one to hit by accident
	assert_eq!(eval(&mut client, Some(0), "breakpoint_target()"), "2");
	println!("ok: evaluating a proc that has a breakpoint in it");

	// numbers are keys in an alist, so walking it by position shows nonsense
	let response = client.request(Request::Eval {
		frame_id: Some(0),
		command: "alist_sample()".to_owned(),
		context: None
	});
	let Response::Eval(alist) = response else {
		panic!("expected the result of alist_sample(), got {:?}", response);
	};
	assert_eq!(alist.value, "/alist {len = 3}");
	let vars = match client.request(Request::Variables {
		vars: alist.variables.expect("an alist should be expandable")
	}) {
		Response::Variables { vars } => vars,
		other => panic!("expected Variables, got {:?}", other)
	};
	let mut shown: Vec<_> = vars.into_iter().map(|var| var.value).collect();
	shown.sort();
	assert_eq!(shown, ["\"a\" = 1", "\"n\" = null", "7 = \"seven\""]);
	println!("ok: an alist shows its key/value pairs");

	// size is a float, so only its start is checked
	let (text, fields) = eval_fields(&mut client, 0, "vector_sample()");
	assert_eq!(text, "vector(1,2,3)");
	assert_eq!(fields[..4], ["x = 1", "y = 2", "z = 3", "len = 3"]);
	assert!(fields[4].starts_with("size = 3.74"), "unexpected size: {}", fields[4]);
	let (text, fields) = eval_fields(&mut client, 0, "vector2_sample()");
	assert_eq!(text, "vector(4,5)");
	assert_eq!(fields[..4], ["x = 4", "y = 5", "z = 0", "len = 2"]);
	println!("ok: a vector shows its components");

	let (text, fields) = eval_fields(&mut client, 0, "pixloc_sample()");
	assert_eq!(text, "pixloc(1,1,1)");
	assert_eq!(fields[..5], ["x = 1", "y = 1", "z = 1", "step_x = 0", "step_y = 0"]);
	assert!(fields[5].starts_with("loc = "), "unexpected loc: {}", fields[5]);
	println!("ok: a pixloc shows its position");

	// frame 1 is main_loop, which holds a callee of itself
	let (text, fields) = eval_fields(&mut client, 1, "this_proc");
	assert!(text.contains("/proc/main_loop"), "unexpected callee text: {}", text);
	assert_eq!(fields[..2], ["proc = /proc/main_loop", "file = \"debug_test_host.dm\""]);
	assert!(fields[2].starts_with("line = "), "unexpected line: {}", fields[2]);
	assert_eq!(fields[3..6], ["src = null", "usr = null", "args = /list {len = 0}"]);
	assert!(fields[6].starts_with("caller = "), "unexpected caller: {}", fields[6]);
	println!("ok: a callee shows its proc");

	assert_eq!(eval(&mut client, None, &dis_command), clean_disassembly);
	println!("ok: disassembly doesn't show the breakpoint");

	let response = client.request(Request::BreakpointUnset {
		instruction: start_of("/proc/does_not_exist")
	});
	assert!(
		matches!(response, Response::BreakpointUnset { success: false }),
		"unsetting a breakpoint on a missing proc got {:?}",
		response
	);
	println!("ok: unsetting a breakpoint on a missing proc");

	// The condition is false, and working that out means calling a proc that
	// has its own breakpoint. The second stop in tick_marker is the one that
	// shows breakpoint_target got skipped.
	set_breakpoint(&mut client, TICK_MARKER, None);
	set_breakpoint(&mut client, BREAKPOINT_TARGET, Some("tick_marker() == 0"));
	client.resume();
	expect_breakpoint_in(&mut client, TICK_MARKER);
	client.resume();
	expect_breakpoint_in(&mut client, TICK_MARKER);
	println!("ok: conditional breakpoint that calls a proc with a breakpoint");

	// A debugger can re-send a breakpoint without unsetting it first, so
	// this is how a condition gets removed
	set_breakpoint(&mut client, BREAKPOINT_TARGET, None);
	client.resume();
	expect_breakpoint_in(&mut client, BREAKPOINT_TARGET);
	println!("ok: re-setting a breakpoint drops its old condition");

	for proc in [BREAKPOINT_TARGET, TICK_MARKER] {
		let response = client.request(Request::BreakpointUnset { instruction: start_of(proc) });
		assert!(
			matches!(response, Response::BreakpointUnset { success: true }),
			"couldn't unset {}: {:?}",
			proc,
			response
		);
	}

	// Runtime errors don't pause it when the launcher says not to
	client.send(Request::CatchRuntimes { should_catch: false });
	eval(&mut client, None, "start_runtimes()");
	client.resume();
	// start_runtimes() gives two seconds of runtime errors, wait them out
	std::thread::sleep(Duration::from_secs(3));
	pause(&mut client);
	println!("ok: survived runtime errors, and still answers");
	client.resume();

	eval(&mut client, None, "request_reboot()");
	client.wait_closed();

	let mut client = world.connect();
	pause(&mut client);
	println!("ok: the world connects again after a reboot");

	println!("Debug server tests succeeded");
}
