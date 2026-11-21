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

use std::{
	path::PathBuf,
	process::Command,
	time::{Duration, Instant}
};

use client::{Client, World};
use paths::ByondCommand;
use server_types::{
	BreakpointReason, BreakpointSetResult, ContinueKind, InstructionRef, ProcRef, ProfileReply, ProfileRequest, ProfileTotals, ProfiledCode, Request,
	Response, VariablesRef
};

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
		condition: condition.map(str::to_owned),
		hit: None,
		log: None
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
		Response::Eval(Ok(eval)) => eval.value,
		other => panic!("expected the result of {:?}, got {:?}", command, other)
	}
}

/// The proc the world is paused in.
fn paused_in(client: &mut Client) -> String {
	match client.request(Request::Stacks { sleeping: None }) {
		Response::Stacks { stacks, .. } => stacks.into_iter().next().expect("paused with no stacks").name,
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
	let Response::Eval(Ok(eval)) = response else {
		panic!("expected the result of {:?}, got {:?}", command, response);
	};
	let refs = eval.variables.unwrap_or_else(|| panic!("{} should be expandable", command));
	let vars = match client.request(Request::Variables {
		vars: refs,
		start: None,
		count: None
	}) {
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

/// How the values 516 brought show
fn values_516(client: &mut Client) {
	// numbers are keys in an alist, so walking it by position shows nonsense
	let response = client.request(Request::Eval {
		frame_id: Some(0),
		command: "alist_sample()".to_owned(),
		context: None
	});
	let Response::Eval(Ok(alist)) = response else {
		panic!("expected the result of alist_sample(), got {:?}", response);
	};
	assert_eq!(alist.value, "/alist {len = 3}");
	let vars = match client.request(Request::Variables {
		vars: alist.variables.expect("an alist should be expandable"),
		start: None,
		count: None
	}) {
		Response::Variables { vars } => vars,
		other => panic!("expected Variables, got {:?}", other)
	};
	let mut shown: Vec<_> = vars.into_iter().map(|var| var.value).collect();
	shown.sort();
	assert_eq!(shown, ["\"a\" = 1", "\"n\" = null", "7 = \"seven\""]);
	println!("ok: an alist shows its key/value pairs");

	// size is a float, so only its start is checked
	let (text, fields) = eval_fields(client, 0, "vector_sample()");
	assert_eq!(text, "vector(1,2,3)");
	assert_eq!(fields[..4], ["x = 1", "y = 2", "z = 3", "len = 3"]);
	assert!(fields[4].starts_with("size = 3.74"), "unexpected size: {}", fields[4]);
	let (text, fields) = eval_fields(client, 0, "vector2_sample()");
	assert_eq!(text, "vector(4,5)");
	assert_eq!(fields[..4], ["x = 4", "y = 5", "z = 0", "len = 2"]);
	println!("ok: a vector shows its components");

	let (text, fields) = eval_fields(client, 0, "pixloc_sample()");
	assert_eq!(text, "pixloc(1,1,1)");
	assert_eq!(fields[..5], ["x = 1", "y = 1", "z = 1", "step_x = 0", "step_y = 0"]);
	assert!(fields[5].starts_with("loc = "), "unexpected loc: {}", fields[5]);
	println!("ok: a pixloc shows its position");

	// frame 1 is main_loop, which holds a callee of itself
	let (text, fields) = eval_fields(client, 1, "this_proc");
	assert!(text.contains("/proc/main_loop"), "unexpected callee text: {}", text);
	assert_eq!(fields[..2], ["proc = /proc/main_loop", "file = \"debug_test_host.dm\""]);
	assert!(fields[2].starts_with("line = "), "unexpected line: {}", fields[2]);
	assert_eq!(fields[3..6], ["src = null", "usr = null", "args = /list {len = 0}"]);
	assert!(fields[6].starts_with("caller = "), "unexpected caller: {}", fields[6]);
	println!("ok: a callee shows its proc");
}

/// The line of the host's code that starts with `text`
fn host_line(text: &str) -> u32 {
	let source = std::fs::read_to_string(host_file("debug_test_host.dm")).unwrap();
	source.lines().position(|line| line.starts_with(text)).expect("no such line") as u32 + 1
}

fn status(client: &mut Client) -> Option<BreakpointReason> {
	match client.request(Request::Status) {
		Response::Status { paused } => paused.map(|stop| stop.reason),
		other => panic!("expected Status, got {:?}", other)
	}
}

fn locate(client: &mut Client, file: &str, line: u32) -> Vec<InstructionRef> {
	match client.request(Request::Locate { file: file.to_owned(), line }) {
		Response::Locate { instructions } => instructions,
		other => panic!("expected Locate, got {:?}", other)
	}
}

/// The requests the launcher makes that the checks above don't. It starts and
/// ends with the world paused in breakpoint_target, with no breakpoints.
fn protocol(world: &World, mut client: Client) -> Client {
	assert_eq!(status(&mut client), Some(BreakpointReason::Breakpoint));
	println!("ok: a paused world says why it's paused");

	// breakpoint_target, called by main_loop, which world/New spawned
	let response = client.request(Request::StackFrames {
		stack_id: 0,
		start_frame: Some(1),
		count: Some(1)
	});
	let Response::StackFrames { frames, total_count } = response else {
		panic!("expected StackFrames, got {:?}", response);
	};
	assert_eq!(total_count, 3);
	assert_eq!(frames.len(), 1);
	assert_eq!(frames[0].instruction.proc.path, "/main_loop");
	assert_eq!(frames[0].file.as_deref(), Some("debug_test_host.dm"));
	assert_eq!(frames[0].line, Some(host_line("\t\tbreakpoint_target()")));
	println!("ok: a page of the stack's frames, each with its file and line");

	// As many as there are, however many are asked for
	let response = client.request(Request::StackFrames {
		stack_id: 0,
		start_frame: Some(1),
		count: Some(u32::MAX)
	});
	assert!(
		matches!(&response, Response::StackFrames { frames, total_count: 3 } if frames.len() == 2),
		"expected the last two frames, got {:?}",
		response
	);
	println!("ok: a page of frames that would go past the most there can be");

	let returns = host_line("\treturn 2");
	let response = client.request(Request::Offset {
		proc: start_of(BREAKPOINT_TARGET).proc,
		line: returns
	});
	let Response::Offset { offset: Some(offset) } = response else {
		panic!("expected the offset of breakpoint_target's line, got {:?}", response);
	};
	let response = client.request(Request::Offset {
		proc: start_of(BREAKPOINT_TARGET).proc,
		line: returns + 100
	});
	assert!(
		matches!(response, Response::Offset { offset: None }),
		"a line the proc doesn't have got {:?}",
		response
	);
	println!("ok: where a proc's line is in its code");

	// However the client spells the file
	let expected = vec![InstructionRef {
		proc: start_of(BREAKPOINT_TARGET).proc,
		offset
	}];
	for file in [
		"debug_test_host.dm",
		"tests/debug_test_host/debug_test_host.dm",
		"C:\\somewhere\\tests\\debug_test_host\\debug_test_host.dm",
		"DEBUG_TEST_HOST.DM"
	] {
		assert_eq!(locate(&mut client, file, returns), expected, "{}", file);
	}
	// The proc's declaration has no code: its first line with code is the next
	assert_eq!(locate(&mut client, "debug_test_host.dm", returns - 1), expected);
	assert_eq!(locate(&mut client, "another_file.dm", returns), vec![]);
	assert_eq!(locate(&mut client, "debug_test_host.dm", 100_000), vec![]);
	println!("ok: the code on a file's line is found by the file's name, a longer path, backslashes and another case");

	let response = client.request(Request::StdDef);
	assert!(
		matches!(&response, Response::StdDef(Some(text)) if text.contains("Standard definitions")),
		"expected stddef.dm, got something else"
	);
	let response = client.request(Request::WorldInfo);
	let Response::WorldInfo { args, cwd, .. } = response else {
		panic!("expected WorldInfo, got {:?}", response);
	};
	assert!(
		args.iter().any(|arg| arg.ends_with("debug_test_host.dmb")),
		"the world's command line is {:?}",
		args
	);
	assert!(!cwd.is_empty());
	println!("ok: the world gives stddef.dm's text, and its own command line");

	client.resume();
	assert_eq!(status(&mut client), None);
	let response = client.request(Request::Scopes { frame_id: 0 });
	assert!(
		matches!(response, Response::Scopes {
			arguments: None,
			locals: None,
			globals: None
		}),
		"scopes of a running world: {:?}",
		response
	);
	let response = client.request(Request::Variables {
		vars: VariablesRef(1),
		start: None,
		count: None
	});
	assert!(
		matches!(&response, Response::Variables { vars } if vars.is_empty()),
		"variables of a running world: {:?}",
		response
	);
	let response = client.request(Request::StackFrames {
		stack_id: 0,
		start_frame: None,
		count: None
	});
	assert!(
		matches!(&response, Response::StackFrames { frames, total_count: 0 } if frames.is_empty()),
		"frames of a running world: {:?}",
		response
	);
	println!("ok: a running world has no scopes, variables or frames to give, and says so");

	// What can't be evaluated, or set, says why
	let response = client.request(Request::Eval {
		frame_id: None,
		command: "1 +".to_owned(),
		context: None
	});
	assert!(
		matches!(&response, Response::Eval(Err(why)) if why.starts_with("parser error: ")),
		"an expression that doesn't parse got {:?}",
		response
	);
	let response = client.request(Request::BreakpointSet {
		instruction: start_of(BREAKPOINT_TARGET),
		condition: Some("1 +".to_owned()),
		hit: None,
		log: None
	});
	assert!(
		matches!(&response, Response::BreakpointSet { result: BreakpointSetResult::Refused { reason } } if reason.starts_with("the condition doesn't compile: ")),
		"a condition that doesn't parse got {:?}",
		response
	);
	println!("ok: an expression and a condition that don't compile are refused, with why");

	// The world puts a breakpoint in the words of an instruction. One in
	// the middle of another's, `return 2`'s value, would make instructions
	// of the rest, with one there already or not.
	pause(&mut client);
	let set = |client: &mut Client, instruction: &InstructionRef| {
		client.request(Request::BreakpointSet {
			instruction: instruction.clone(),
			condition: None,
			hit: None,
			log: None
		})
	};
	let Response::BreakpointSet {
		result: BreakpointSetResult::Success { at, .. }
	} = set(&mut client, &start_of(BREAKPOINT_TARGET))
	else {
		panic!("couldn't set a breakpoint on {}", BREAKPOINT_TARGET);
	};
	let inside = InstructionRef {
		offset: at.offset + 1,
		..at.clone()
	};
	let response = set(&mut client, &inside);
	assert!(
		matches!(response, Response::BreakpointSet {
			result: BreakpointSetResult::Failed
		}),
		"a breakpoint inside an instruction with one got {:?}",
		response
	);
	let response = client.request(Request::BreakpointUnset {
		instruction: start_of(BREAKPOINT_TARGET)
	});
	assert!(matches!(response, Response::BreakpointUnset { success: true, .. }), "{:?}", response);
	let response = client.request(Request::InstructionBreakpointSet {
		instruction: inside,
		condition: None,
		hit: None,
		log: None
	});
	assert!(
		matches!(response, Response::BreakpointSet {
			result: BreakpointSetResult::Failed
		}),
		"a breakpoint inside an instruction got {:?}",
		response
	);
	client.resume();
	println!("ok: a breakpoint in the middle of an instruction isn't set, with a breakpoint at its start or not");

	// A breakpoint set on the instruction the world is paused at: the
	// instruction still runs, and the breakpoint stops the next pass. Step
	// out to main_loop first, so the pause is in a proc that only a bad
	// instruction would end.
	set_breakpoint(&mut client, BREAKPOINT_TARGET, None);
	expect_breakpoint_in(&mut client, BREAKPOINT_TARGET);
	let response = client.request(Request::BreakpointUnset {
		instruction: start_of(BREAKPOINT_TARGET)
	});
	assert!(matches!(response, Response::BreakpointUnset { success: true, .. }), "{:?}", response);
	let response = client.request(Request::Continue {
		kind: ContinueKind::StepOut { stack_id: 0 }
	});
	assert!(matches!(response, Response::Ack), "expected Ack for StepOut, got {:?}", response);
	assert_eq!(client.expect_pause(), BreakpointReason::Step);
	assert_eq!(paused_in(&mut client), "/main_loop");
	let response = client.request(Request::StackFrames {
		stack_id: 0,
		start_frame: Some(0),
		count: Some(1)
	});
	let Response::StackFrames { frames, .. } = response else {
		panic!("expected StackFrames, got {:?}", response);
	};
	let here = || InstructionRef {
		proc: ProcRef {
			path: frames[0].instruction.proc.path.clone(),
			override_id: frames[0].instruction.proc.override_id
		},
		offset: frames[0].instruction.offset
	};
	let response = client.request(Request::InstructionBreakpointSet {
		instruction: here(),
		condition: None,
		hit: None,
		log: None
	});
	assert!(
		matches!(response, Response::BreakpointSet {
			result: BreakpointSetResult::Success { .. }
		}),
		"a breakpoint on the paused instruction got {:?}",
		response
	);
	client.resume();
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_in(&mut client), "/main_loop");
	let response = client.request(Request::InstructionBreakpointUnset { instruction: here() });
	assert!(matches!(response, Response::BreakpointUnset { success: true, .. }), "{:?}", response);
	client.resume();
	assert_eq!(eval(&mut client, None, "1 + 1"), "2");
	println!("ok: a breakpoint set on the paused instruction");

	// Two clients want one breakpoint: it's there until both have let go
	let mut second = world.connect();
	pause(&mut client);
	assert_eq!(second.expect_pause(), BreakpointReason::Pause);
	set_breakpoint(&mut client, BREAKPOINT_TARGET, None);
	set_breakpoint(&mut second, BREAKPOINT_TARGET, None);
	client.resume();
	expect_breakpoint_in(&mut client, BREAKPOINT_TARGET);
	assert_eq!(second.expect_pause(), BreakpointReason::Breakpoint);
	let unset = |client: &mut Client, proc: &str| {
		let response = client.request(Request::BreakpointUnset { instruction: start_of(proc) });
		assert!(
			matches!(response, Response::BreakpointUnset { success: true, .. }),
			"couldn't unset {}: {:?}",
			proc,
			response
		);
	};
	unset(&mut client, BREAKPOINT_TARGET);
	client.resume();
	assert_eq!(second.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_in(&mut second), BREAKPOINT_TARGET);
	// tick_marker is called right after: the next stop is there
	unset(&mut second, BREAKPOINT_TARGET);
	set_breakpoint(&mut second, TICK_MARKER, None);
	second.resume();
	assert_eq!(second.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_in(&mut second), TICK_MARKER);
	second.resume();
	assert_eq!(second.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_in(&mut second), TICK_MARKER);
	println!("ok: a breakpoint two clients set stays until both have unset it");

	// A client that leaves takes its breakpoints, and the world carries on
	// when the last has gone
	second.leave();
	client.leave();
	let mut client = world.connect();
	assert_eq!(status(&mut client), None);
	println!("ok: the last client leaves a paused world, which carries on");

	// A client that's there for something else doesn't keep a pause going
	let watcher = world.connect_watcher();
	pause(&mut client);
	client.leave();
	let mut client = world.connect();
	assert_eq!(status(&mut client), None);
	watcher.leave();
	println!("ok: a pause doesn't wait for a client that says it isn't debugging");

	// A pause that's held outlasts its clients
	pause(&mut client);
	let response = client.request(Request::HoldPause);
	assert!(matches!(response, Response::Ack), "expected Ack for HoldPause, got {:?}", response);
	client.leave();
	let mut client = world.connect();
	assert_eq!(status(&mut client), Some(BreakpointReason::Pause));
	client.resume();
	assert_eq!(status(&mut client), None);
	println!("ok: a held pause is still there for the next client, until one lets the world carry on");

	// Asked for by a client that goes before there's a pause, it's forgotten
	let response = client.request(Request::HoldPause);
	assert!(matches!(response, Response::Ack), "expected Ack for HoldPause, got {:?}", response);
	client.leave();
	let mut client = world.connect();
	pause(&mut client);
	client.leave();
	let mut client = world.connect();
	assert_eq!(status(&mut client), None);
	println!("ok: a hold asked for by a client that left before the pause is forgotten");

	// Whatever arrives that isn't a request: that client is hung up on, and
	// nothing else notices
	let mut rubbish = world.connect_silent();
	rubbish.send_bytes(&[0xff, 0xff, 0xff, 0xff, 1, 2, 3]);
	rubbish.wait_closed();
	let mut rubbish = world.connect_silent();
	rubbish.send_bytes(&[3, 0, 0, 0, 0xfe, 0xfe, 0xfe]);
	rubbish.wait_closed();
	let quiet = world.connect_silent();
	assert_eq!(status(&mut client), None);
	assert_eq!(eval(&mut client, None, "1 + 1"), "2");

	// A proc BYOND loses its caller's registers in. (It was a release build's
	// trampoline that faulted on it, and this world alone crashes DreamDaemon
	// without us: the test is that it runs through here.) Not on Windows,
	// where BYOND checks its stack after the proc and ends the world itself
	// (exception c0000409), with or without a debugger.
	if cfg!(unix) {
		assert_eq!(eval(&mut client, None, "overrun()"), "7");
		println!("ok: a proc that overruns BYOND's buffer for its stack leaves the world running");
	}
	drop(quiet);
	println!("ok: bytes that aren't a request get that client hung up on, and a client that says nothing is left alone");

	set_breakpoint(&mut client, BREAKPOINT_TARGET, None);
	expect_breakpoint_in(&mut client, BREAKPOINT_TARGET);
	unset(&mut client, BREAKPOINT_TARGET);
	client
}

fn profile_request(client: &mut Client, request: ProfileRequest) -> Result<ProfileReply, String> {
	match client.request(Request::Profile(request)) {
		Response::Profile(reply) => reply,
		other => panic!("expected Profile, got {:?}", other)
	}
}

fn profile_totals(client: &mut Client, request: ProfileRequest) -> ProfileTotals {
	match profile_request(client, request) {
		Ok(ProfileReply::Totals(totals)) => totals,
		other => panic!("expected the totals of a profile, got {:?}", other)
	}
}

/// Find profile measurements by proc path.
fn profiled<'a>(totals: &'a ProfileTotals, path: &str) -> &'a ProfiledCode {
	(totals.codes.iter().find(|code| code.proc.path == path)).unwrap_or_else(|| panic!("{} isn't in {:?}", path, totals))
}

/// Check profiling counts, timing and exclusions. Leave the world paused.
fn profile(world: &World, mut client: Client) -> Client {
	let proc = |path: &str| ProcRef {
		path: path.to_owned(),
		override_id: 0
	};
	assert_eq!(
		profile_request(&mut client, ProfileRequest::Totals),
		Err("no profile has been started".to_owned())
	);
	let missing = profile_request(&mut client, ProfileRequest::Start {
		procs: vec![proc("/profiled_missing")]
	});
	assert_eq!(missing, Err("the world has no proc /profiled_missing".to_owned()));
	assert_eq!(
		profile_request(&mut client, ProfileRequest::Start { procs: vec![] }),
		Ok(ProfileReply::Started)
	);
	assert_eq!(
		profile_request(&mut client, ProfileRequest::Start { procs: vec![] }),
		Err("another profile is running".to_owned())
	);
	println!("ok: profile validation");

	// The world's loop sleeps a tick after each turn. Were sleeps timed,
	// the profile's time would be about the time it ran.
	assert_eq!(eval(&mut client, None, "profiled_call()"), "500500");
	client.resume();
	std::thread::sleep(Duration::from_secs(1));
	let running = profile_totals(&mut client, ProfileRequest::Totals);
	assert!(running.running && running.seconds >= 1.0, "{:?}", running);
	let totals = profile_totals(&mut client, ProfileRequest::Stop);
	assert!(!totals.running && totals.seconds >= running.seconds, "{:?}", totals);
	let own: u64 = totals.codes.iter().map(|code| code.total.own_ns).sum();
	assert!((own as f64) < totals.seconds * 1e9 / 10.0, "the world slept most of {:?}", totals);
	let main_loop = profiled(&totals, "/main_loop");
	assert!(main_loop.total.hits > 0 && main_loop.total.calls == 0, "{:?}", main_loop);
	assert!(totals.codes.iter().all(|code| !code.proc.path.contains("auxtools_")), "{:?}", totals);
	println!("ok: sleep and debugger helper exclusions");

	// The loop's instructions ran once a turn, the rest once a call
	let looped = profiled(&totals, "/profiled_loop");
	assert_eq!(looped.total.calls, 1);
	assert_eq!(looped.file.as_deref(), Some("debug_test_host.dm"));
	let Ok(ProfileReply::Code(instructions)) = profile_request(&mut client, ProfileRequest::Code { code: looped.code }) else {
		panic!("expected the instructions of {:?}", looped);
	};
	let Response::Disassemble(Ok(disassembly)) = client.request(Request::Disassemble { code: looped.code }) else {
		panic!("expected the disassembly of {:?}", looped);
	};
	for instruction in &instructions {
		let text = (disassembly.instructions.iter().find(|at| at.offset == instruction.offset))
			.unwrap_or_else(|| panic!("no instruction starts at {:?}", instruction));
		let turns = if text.text.starts_with("AugAdd") { 1000 } else { instruction.hits };
		assert_eq!(instruction.hits, turns, "{}", text.text);
		assert!(
			matches!(instruction.hits, 1 | 1000 | 1001),
			"{} ran {} times",
			text.text,
			instruction.hits
		);
		assert_eq!(instruction.callee_ns, 0, "{} calls nothing", text.text);
	}
	assert_eq!(instructions.iter().map(|instruction| instruction.hits).sum::<u64>(), looped.total.hits);
	assert_eq!(
		instructions.iter().map(|instruction| instruction.own_ns).sum::<u64>(),
		looped.total.own_ns
	);
	assert!(instructions.iter().any(|instruction| instruction.hits == 1000), "{:?}", instructions);
	assert!(looped.total.own_ns > 0, "{:?}", looped);
	println!("ok: instruction hit counts");

	// Allow for differences in overhead correction between caller and callee.
	let calling = profiled(&totals, "/profiled_call");
	assert_eq!(calling.total.calls, 1);
	let near = |a: u64, b: u64| a.abs_diff(b) <= b / 4 + 2_000;
	assert!(near(calling.total.callee_ns, looped.total.own_ns), "{:?} called {:?}", calling, looped);
	println!("ok: callee timing");

	// Kept after it stops, until the next
	assert_eq!(profile_totals(&mut client, ProfileRequest::Totals), totals);
	let code = match client.request(Request::ProcInfo { proc: proc("/overrun") }) {
		Response::ProcInfo(Ok(info)) => info.code,
		other => panic!("expected ProcInfo, got {:?}", other)
	};
	let unprofiled = profile_request(&mut client, ProfileRequest::Code { code });
	assert_eq!(unprofiled, Err("/overrun wasn't profiled".to_owned()));

	// Of the procs asked for alone
	pause(&mut client);
	let only = ProfileRequest::Start {
		procs: vec![proc("/profiled_loop")]
	};
	assert_eq!(profile_request(&mut client, only), Ok(ProfileReply::Started));
	assert_eq!(eval(&mut client, None, "profiled_call()"), "500500");
	let totals = profile_totals(&mut client, ProfileRequest::Stop);
	assert_eq!(totals.codes.len(), 1, "{:?}", totals);
	assert_eq!(profiled(&totals, "/profiled_loop").total.calls, 1);
	println!("ok: proc filters");

	// It goes with the client that started it
	let mut second = world.connect();
	assert_eq!(
		profile_request(&mut second, ProfileRequest::Start { procs: vec![] }),
		Ok(ProfileReply::Started)
	);
	let theirs = Err("the profile belongs to another client".to_owned());
	assert_eq!(profile_request(&mut client, ProfileRequest::Totals), theirs);
	assert_eq!(profile_request(&mut client, ProfileRequest::Stop), theirs);
	assert_eq!(profile_request(&mut client, ProfileRequest::Code { code: looped.code }), theirs);
	assert!(profile_totals(&mut second, ProfileRequest::Totals).running);
	second.leave();
	// (the world sees it gone when it next looks)
	let none = Err("no profile has been started".to_owned());
	let mut gone = profile_request(&mut client, ProfileRequest::Totals);
	for _ in 0..50 {
		if gone == none {
			break;
		}
		std::thread::sleep(Duration::from_millis(100));
		gone = profile_request(&mut client, ProfileRequest::Totals);
	}
	assert_eq!(gone, none);
	println!("ok: profile client cleanup");
	client
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
	set_breakpoint(&mut client, BREAKPOINT_TARGET, None);
	expect_breakpoint_in(&mut client, BREAKPOINT_TARGET);
	println!("ok: breakpoint hit");

	// Not before the stop: the world answers while it's still setting its
	// globals, and the helper is one of them
	assert_eq!(eval(&mut client, None, "helper.greet(\"you\")"), "\"hi you\"");
	println!("ok: evaluating a verb call");

	// VS Code re-runs every watch expression on every stop, so this is the
	// easy one to hit by accident
	assert_eq!(eval(&mut client, Some(0), "breakpoint_target()"), "2");
	println!("ok: evaluating a proc that has a breakpoint in it");

	let byond = match client.request(Request::ByondVersion) {
		Response::ByondVersion { version, build } => (version, build),
		other => panic!("expected ByondVersion, got {:?}", other)
	};
	if byond.0 >= 516 {
		values_516(&mut client);
	} else {
		println!("(BYOND {}.{} has no alists, vectors, pixlocs or callees to show)", byond.0, byond.1);
	}

	assert_eq!(eval(&mut client, None, &dis_command), clean_disassembly);
	println!("ok: disassembly doesn't show the breakpoint");

	let response = client.request(Request::BreakpointUnset {
		instruction: start_of("/proc/does_not_exist")
	});
	assert!(
		matches!(response, Response::BreakpointUnset { success: false, .. }),
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
			matches!(response, Response::BreakpointUnset { success: true, .. }),
			"couldn't unset {}: {:?}",
			proc,
			response
		);
	}

	let client = protocol(&world, client);
	let mut client = profile(&world, client);

	// Runtime errors don't pause it when the launcher says not to
	client.send(Request::CatchRuntimes {
		uncaught: None,
		caught: None
	});
	eval(&mut client, None, "start_runtimes()");
	client.resume();
	// start_runtimes() gives twenty runtime errors, wait them out
	let start = Instant::now();
	while eval(&mut client, None, "runtimes_left") != "0" {
		assert!(start.elapsed() < Duration::from_secs(10), "the world's runtime errors never ended");
		std::thread::sleep(Duration::from_millis(20));
	}
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
