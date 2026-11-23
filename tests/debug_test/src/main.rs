use std::{
	path::PathBuf,
	time::{Duration, Instant}
};

use auxtools_protocol::{
	BreakpointReason, BreakpointSetResult, ContinueKind, DataChange, InstructionRef, JournalEvent, JournalKind, LogPart, ProcRef, ProfileReply,
	ProfileRequest, ProfileTotals, ProfiledCode, Request, Response, RuntimeFilter, VariablesRef, WatchPlace
};
use test_support::{
	debug::{breakpoint_id, Client, World},
	paths
};

const BUSY: &str = "var/start = world.timeofday
var/turns = 0
while (turns < 300000 && world.timeofday >= start && world.timeofday - start < 3)
	turns++
return turns";

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
		id: breakpoint_id(&start_of(proc), false),
		instruction: start_of(proc),
		condition: condition.map(str::to_owned),
		hit: None,
		log: None,
		exact: false
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
		ending: false
	});

	match response {
		Response::Eval(Ok(eval)) => eval.value,
		other => panic!("expected the result of {:?}, got {:?}", command, other)
	}
}

fn paused_in(client: &mut Client) -> String {
	match client.request(Request::Stacks { sleeping: None }) {
		Response::Stacks { stacks, .. } => stacks.into_iter().next().expect("paused with no stacks").name,
		other => panic!("expected Stacks, got {:?}", other)
	}
}

fn eval_fields(client: &mut Client, frame_id: u32, command: &str) -> (String, Vec<String>) {
	let response = client.request(Request::Eval {
		frame_id: Some(frame_id),
		command: command.to_owned(),
		ending: false
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

fn values_516(client: &mut Client) {
	let response = client.request(Request::Eval {
		frame_id: Some(0),
		command: "alist_sample()".to_owned(),
		ending: false
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

	let (text, fields) = eval_fields(client, 1, "this_proc");
	assert!(text.contains("/proc/main_loop"), "unexpected callee text: {}", text);
	assert_eq!(fields[..2], ["proc = /proc/main_loop", "file = \"debug_test_host.dm\""]);
	assert!(fields[2].starts_with("line = "), "unexpected line: {}", fields[2]);
	assert_eq!(fields[3..6], ["src = null", "usr = null", "args = /list {len = 0}"]);
	assert!(fields[6].starts_with("caller = "), "unexpected caller: {}", fields[6]);
	println!("ok: a callee shows its proc");
}

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

fn protocol(world: &World, mut client: Client) -> Client {
	assert_eq!(status(&mut client), Some(BreakpointReason::Breakpoint));
	println!("ok: a paused world says why it's paused");

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
	assert_eq!(
		locate(&mut client, "debug_test_host.dm", returns - 1),
		expected,
		"the line of a proc's declaration finds the code on the next"
	);
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

	let response = client.request(Request::Eval {
		frame_id: None,
		command: "1 +".to_owned(),
		ending: false
	});
	assert!(
		matches!(&response, Response::Eval(Err(why)) if why.message.starts_with("parser error: ") && !why.still_running),
		"an expression that doesn't parse got {:?}",
		response
	);
	let response = client.request(Request::BreakpointSet {
		id: breakpoint_id(&start_of(BREAKPOINT_TARGET), false),
		instruction: start_of(BREAKPOINT_TARGET),
		condition: Some("1 +".to_owned()),
		hit: None,
		log: None,
		exact: false
	});
	assert!(
		matches!(&response, Response::BreakpointSet { result: BreakpointSetResult::Refused { reason } } if reason.starts_with("the condition doesn't compile: ")),
		"a condition that doesn't parse got {:?}",
		response
	);
	println!("ok: an expression and a condition that don't compile are refused, with why");

	pause(&mut client);
	let set = |client: &mut Client, instruction: &InstructionRef| {
		client.request(Request::BreakpointSet {
			id: breakpoint_id(instruction, false),
			instruction: instruction.clone(),
			condition: None,
			hit: None,
			log: None,
			exact: false
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
		id: breakpoint_id(&start_of(BREAKPOINT_TARGET), false)
	});
	assert!(matches!(response, Response::BreakpointUnset { success: true, .. }), "{:?}", response);
	let response = client.request(Request::BreakpointSet {
		id: breakpoint_id(&inside, true),
		instruction: inside,
		condition: None,
		hit: None,
		log: None,
		exact: true
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

	set_breakpoint(&mut client, BREAKPOINT_TARGET, None);
	expect_breakpoint_in(&mut client, BREAKPOINT_TARGET);
	let response = client.request(Request::BreakpointUnset {
		id: breakpoint_id(&start_of(BREAKPOINT_TARGET), false)
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
	let response = client.request(Request::BreakpointSet {
		id: breakpoint_id(&here(), true),
		instruction: here(),
		condition: None,
		hit: None,
		log: None,
		exact: true
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
	let response = client.request(Request::BreakpointUnset {
		id: breakpoint_id(&here(), true)
	});
	assert!(matches!(response, Response::BreakpointUnset { success: true, .. }), "{:?}", response);
	client.resume();
	assert_eq!(eval(&mut client, None, "1 + 1"), "2");
	println!("ok: a breakpoint set on the paused instruction");

	let mut second = world.connect();
	pause(&mut client);
	assert_eq!(second.expect_pause(), BreakpointReason::Pause);
	set_breakpoint(&mut client, BREAKPOINT_TARGET, None);
	set_breakpoint(&mut second, BREAKPOINT_TARGET, None);
	client.resume();
	expect_breakpoint_in(&mut client, BREAKPOINT_TARGET);
	assert_eq!(second.expect_pause(), BreakpointReason::Breakpoint);
	let unset = |client: &mut Client, proc: &str| {
		let response = client.request(Request::BreakpointUnset {
			id: breakpoint_id(&start_of(proc), false)
		});
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

	second.leave();
	client.leave();
	let mut client = world.connect();
	assert_eq!(status(&mut client), None);
	println!("ok: the last client leaves a paused world, which carries on");

	let watcher = world.connect_watcher();
	pause(&mut client);
	client.leave();
	let mut client = world.connect();
	assert_eq!(status(&mut client), None);
	watcher.leave();
	println!("ok: a pause doesn't wait for a client that says it isn't debugging");

	pause(&mut client);
	let response = client.request(Request::HoldPause);
	assert!(matches!(response, Response::Ack), "expected Ack for HoldPause, got {:?}", response);
	client.leave();
	let mut client = world.connect();
	assert_eq!(status(&mut client), Some(BreakpointReason::Pause));
	client.resume();
	assert_eq!(status(&mut client), None);
	println!("ok: a held pause is still there for the next client, until one lets the world carry on");

	let response = client.request(Request::HoldPause);
	assert!(matches!(response, Response::Ack), "expected Ack for HoldPause, got {:?}", response);
	client.leave();
	let mut client = world.connect();
	pause(&mut client);
	client.leave();
	let mut client = world.connect();
	assert_eq!(status(&mut client), None);
	println!("ok: a hold asked for by a client that left before the pause is forgotten");

	let mut rubbish = world.connect_silent();
	rubbish.send_bytes(&[0xff, 0xff, 0xff, 0xff, 1, 2, 3]);
	rubbish.wait_closed();
	let mut rubbish = world.connect_silent();
	rubbish.send_bytes(&[3, 0, 0, 0, 0xfe, 0xfe, 0xfe]);
	rubbish.wait_closed();
	let quiet = world.connect_silent();
	assert_eq!(status(&mut client), None);
	assert_eq!(eval(&mut client, None, "1 + 1"), "2");

	if cfg!(unix) {
		assert_eq!(eval(&mut client, None, "overrun()"), "7");
		println!("ok: a proc that overruns BYOND's buffer for its stack leaves the world running");
	}
	drop(quiet);
	println!("ok: bytes that aren't a request get that client hung up on, and a client that says nothing is left alone");

	let late = client.late;
	let started = Instant::now();
	let busy = Request::Eval {
		frame_id: None,
		command: BUSY.to_owned(),
		ending: false
	};
	let answer = client.request_within(busy, Duration::from_millis(20));
	assert!(
		answer.is_none(),
		"the world wasn't busy for long enough: {:?} after {:?}",
		answer,
		started.elapsed()
	);
	assert_eq!(status(&mut client), None);
	assert_eq!(client.late, late + 1, "the evaluation's answer didn't come before the next request's");
	assert_eq!(eval(&mut client, None, "1 + 1"), "2");
	println!("ok: an answer that comes after its request timed out isn't taken for the next request's");

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

fn profiled<'a>(totals: &'a ProfileTotals, path: &str) -> &'a ProfiledCode {
	(totals.codes.iter().find(|code| code.proc.path == path)).unwrap_or_else(|| panic!("{} isn't in {:?}", path, totals))
}

fn most_hits(client: &mut Client, code: &ProfiledCode) -> u64 {
	match profile_request(client, ProfileRequest::Code { code: code.code }) {
		Ok(ProfileReply::Code(instructions)) => instructions.iter().map(|instruction| instruction.hits).max().unwrap_or(0),
		other => panic!("expected the instructions of {:?}, got {:?}", code, other)
	}
}

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

	let calling = profiled(&totals, "/profiled_call");
	assert_eq!(calling.total.calls, 1);
	let near = |a: u64, b: u64| a.abs_diff(b) <= b / 4 + 2_000;
	assert!(near(calling.total.callee_ns, looped.total.own_ns), "{:?} called {:?}", calling, looped);
	println!("ok: callee timing");

	assert_eq!(
		profile_totals(&mut client, ProfileRequest::Totals),
		totals,
		"a stopped profile's totals are kept until the next"
	);
	let code = match client.request(Request::ProcInfo { proc: proc("/overrun") }) {
		Response::ProcInfo(Ok(info)) => info.code,
		other => panic!("expected ProcInfo, got {:?}", other)
	};
	let unprofiled = profile_request(&mut client, ProfileRequest::Code { code });
	assert_eq!(unprofiled, Err("/overrun wasn't profiled".to_owned()));

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

	for order in ["profiled_twin_b(10) + profiled_twin_a(100)", "profiled_twin_a(100) + profiled_twin_b(10)"] {
		let only = ProfileRequest::Start {
			procs: vec![proc("/profiled_twin_a")]
		};
		assert_eq!(profile_request(&mut client, only), Ok(ProfileReply::Started));
		assert_eq!(eval(&mut client, None, order), "5105");
		let totals = profile_totals(&mut client, ProfileRequest::Stop);
		assert_eq!(totals.codes.len(), 1, "after {}: {:?}", order, totals);
		let twin = profiled(&totals, "/profiled_twin_a");
		assert_eq!((twin.total.calls, most_hits(&mut client, twin)), (1, 101), "after {}: {:?}", order, twin);
	}
	assert_eq!(
		profile_request(&mut client, ProfileRequest::Start { procs: vec![] }),
		Ok(ProfileReply::Started)
	);
	assert_eq!(eval(&mut client, None, "profiled_twin_a(100) + profiled_twin_b(10)"), "5105");
	let totals = profile_totals(&mut client, ProfileRequest::Stop);
	for (path, hits) in [("/profiled_twin_a", 101), ("/profiled_twin_b", 11)] {
		let twin = profiled(&totals, path);
		assert_eq!((twin.total.calls, most_hits(&mut client, twin)), (1, hits), "{:?}", twin);
	}
	println!("ok: procs that share code are profiled apart");

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

fn loop_back_to_the_start() {
	let world = World::start(&host_file("debug_test_plain.dmb"), &[]);
	let mut client = world.connect();
	pause(&mut client);
	let only = ProfileRequest::Start {
		procs: vec![ProcRef {
			path: "/profiled_countdown".to_owned(),
			override_id: 0
		}]
	};
	let count_down = |client: &mut Client| {
		assert_eq!(profile_request(client, only.clone()), Ok(ProfileReply::Started));
		assert_eq!(eval(client, None, "profiled_countdown(5)"), "0");
		let totals = profile_totals(client, ProfileRequest::Stop);
		let counted = profiled(&totals, "/profiled_countdown");
		let Ok(ProfileReply::Code(instructions)) = profile_request(client, ProfileRequest::Code { code: counted.code }) else {
			panic!("expected the instructions of {:?}", counted);
		};
		assert_eq!(
			instructions.first().map(|first| (first.offset, first.hits)),
			Some((0, 6)),
			"{:?}",
			instructions
		);
		assert_eq!(counted.total.calls, 1, "{:?}", counted);
		counted.clone()
	};
	let counted = count_down(&mut client);
	println!("ok: a loop back to the first instruction isn't counted as calls");

	let Response::Disassemble(Ok(disassembly)) = client.request(Request::Disassemble { code: counted.code }) else {
		panic!("expected the disassembly of {:?}", counted);
	};
	let back = (disassembly.instructions.iter().rev())
		.find(|instruction| instruction.text.starts_with("Jmp"))
		.unwrap_or_else(|| panic!("no jump back in {:?}", disassembly.instructions));
	let back_at = InstructionRef {
		proc: counted.proc.clone(),
		offset: back.offset as u32
	};
	let break_back = |client: &mut Client, condition: Option<&str>| {
		let response = client.request(Request::BreakpointSet {
			id: breakpoint_id(&back_at, true),
			instruction: back_at.clone(),
			condition: condition.map(str::to_owned),
			hit: None,
			log: None,
			exact: true
		});
		assert!(
			matches!(response, Response::BreakpointSet {
				result: BreakpointSetResult::Success { .. }
			}),
			"a breakpoint on `{}` got {:?}",
			back.text,
			response
		);
	};
	break_back(&mut client, None);
	count_down(&mut client);
	println!("ok: a loop back to the first instruction through a breakpoint isn't counted as calls");

	let step_back = |client: &mut Client| {
		eval(client, None, "spawn()\n\tprofiled_countdown(3)");
		client.resume();
		assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
		assert_eq!(paused_at(client), (counted.proc.path.clone(), back.offset as u32));
		let response = client.request(Request::Continue {
			kind: ContinueKind::StepOverInstruction { stack_id: 0 }
		});
		assert!(
			matches!(response, Response::Ack),
			"expected Ack for StepOverInstruction, got {:?}",
			response
		);
		assert_eq!(client.expect_pause(), BreakpointReason::Step);
		assert_eq!(paused_at(client), (counted.proc.path.clone(), 0));
	};
	step_back(&mut client);
	println!("ok: a step from a breakpoint on a loop back to the first instruction stops there");

	let run_out = |client: &mut Client| {
		break_back(client, Some("n < 0"));
		client.resume();
		std::thread::sleep(Duration::from_millis(200));
		pause(client);
	};
	run_out(&mut client);
	break_back(&mut client, Some("n < 10"));
	step_back(&mut client);
	println!("ok: a step from a conditional breakpoint on a loop back to the first instruction stops there");

	run_out(&mut client);
	assert_eq!(profile_request(&mut client, only.clone()), Ok(ProfileReply::Started));
	eval(&mut client, None, "spawn()\n\tprofiled_countdown(5)");
	client.resume();
	let start = Instant::now();
	let totals = loop {
		let totals = profile_totals(&mut client, ProfileRequest::Totals);
		if totals.codes.first().is_some_and(|code| code.total.hits == counted.total.hits) {
			break profile_totals(&mut client, ProfileRequest::Stop);
		}
		assert!(start.elapsed() < Duration::from_secs(10), "the countdown never ended: {:?}", totals);
		std::thread::sleep(Duration::from_millis(20));
	};
	let looped = profiled(&totals, "/profiled_countdown");
	let Ok(ProfileReply::Code(instructions)) = profile_request(&mut client, ProfileRequest::Code { code: looped.code }) else {
		panic!("expected the instructions of {:?}", looped);
	};
	assert_eq!(
		(looped.total.calls, instructions.first().map(|first| (first.offset, first.hits))),
		(1, Some((0, 6))),
		"{:?} {:?}",
		looped,
		instructions
	);
	println!("ok: a loop back to the first instruction through a conditional breakpoint isn't counted as calls");

	pause(&mut client);
	break_back(&mut client, None);
	eval(&mut client, None, "spawn()\n\tprofiled_countdown(3)");
	client.resume();
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	let response = client.request(Request::BreakpointUnset {
		id: breakpoint_id(&back_at, true)
	});
	assert!(matches!(response, Response::BreakpointUnset { success: true, .. }), "{:?}", response);
	let response = client.request(Request::WatchSet {
		place: WatchPlace::Expression {
			frame_id: Some(0),
			expression: "n".to_owned()
		},
		condition: None,
		hit: None
	});
	let Response::WatchSet(Ok(watch)) = response else {
		panic!("expected a watch on n, got {:?}", response);
	};
	client.resume();
	for (old, new) in [("2", "1"), ("1", "0")] {
		let changed = BreakpointReason::Data {
			changes: vec![DataChange {
				id: watch.id,
				name: "n".to_owned(),
				old: old.to_owned(),
				new: new.to_owned()
			}]
		};
		assert_eq!(client.expect_pause(), changed);
		if old == "2" {
			assert_eq!(paused_at(&mut client), (counted.proc.path.clone(), back.offset as u32));
			let response = client.request(Request::Continue {
				kind: ContinueKind::StepOverInstruction { stack_id: 0 }
			});
			assert!(
				matches!(response, Response::Ack),
				"expected Ack for StepOverInstruction, got {:?}",
				response
			);
			assert_eq!(client.expect_pause(), BreakpointReason::Step);
			assert_eq!(paused_at(&mut client), (counted.proc.path.clone(), 0));
			println!("ok: an instruction step from a watch's stop runs the instruction");
		}
		client.resume();
	}
	let ended = client.recv();
	assert!(
		matches!(&ended, Response::WatchEnded { id, reason } if *id == watch.id && reason == "watched proc returned"),
		"expected the watch to end when the proc returned, got {:?}",
		ended
	);
	println!("ok: a watch on an argument lasts through a loop back to the first instruction");
}

fn break_exactly(client: &mut Client, at: &InstructionRef, log: Option<&str>) {
	let response = client.request(Request::BreakpointSet {
		id: breakpoint_id(at, true),
		instruction: at.clone(),
		condition: None,
		hit: None,
		log: log.map(LogPart::parse),
		exact: true
	});
	assert!(
		matches!(response, Response::BreakpointSet {
			result: BreakpointSetResult::Success { .. }
		}),
		"a breakpoint on {:?} got {:?}",
		at,
		response
	);
}

fn watch_steps(client: &mut Client, expression: &str) -> u32 {
	eval(client, None, "spawn()\n\twatched_steps()");
	client.resume();
	expect_breakpoint_in(client, "/watched_steps");
	let response = client.request(Request::WatchSet {
		place: WatchPlace::Expression {
			frame_id: Some(0),
			expression: expression.to_owned()
		},
		condition: None,
		hit: None
	});
	let Response::WatchSet(Ok(watch)) = response else {
		panic!("expected a watch on {}, got {:?}", expression, response);
	};
	client.resume();
	watch.id
}

fn changed(id: u32, name: &str, old: &str, new: &str) -> BreakpointReason {
	BreakpointReason::Data {
		changes: vec![DataChange {
			id,
			name: name.to_owned(),
			old: old.to_owned(),
			new: new.to_owned()
		}]
	}
}

fn expect_returned(client: &mut Client, watch: u32) {
	let ended = client.recv();
	assert!(
		matches!(&ended, Response::WatchEnded { id, reason } if *id == watch && reason == "watched proc returned"),
		"expected the watch to end when the proc returned, got {:?}",
		ended
	);
}

fn watches_at_the_end() {
	let world = World::start(&host_file("debug_test_plain.dmb"), &[]);
	let mut client = world.connect();
	pause(&mut client);
	set_breakpoint(&mut client, "/watched_steps", None);
	let code = match client.request(Request::ProcInfo {
		proc: start_of("/watched_steps").proc
	}) {
		Response::ProcInfo(Ok(info)) => info.code,
		other => panic!("expected ProcInfo, got {:?}", other)
	};
	let Response::Disassemble(Ok(disassembly)) = client.request(Request::Disassemble { code }) else {
		panic!("expected the disassembly of /watched_steps");
	};
	let last = disassembly.instructions.last().expect("no instructions in /watched_steps");
	let end = InstructionRef {
		offset: last.offset as u32,
		..start_of("/watched_steps")
	};

	let dot = watch_steps(&mut client, ".");
	assert_eq!(client.expect_pause(), changed(dot, ".", "null", "2"));
	assert_eq!(paused_at(&mut client), ("/watched_steps".to_owned(), end.offset));
	client.resume();
	expect_returned(&mut client, dot);
	println!("ok: a frame watch stops for a change right before its proc returns");

	pause(&mut client);
	break_exactly(&mut client, &end, None);
	let steps = watch_steps(&mut client, "steps");
	assert_eq!(client.expect_pause(), changed(steps, "steps", "null", "1"));
	let first_change = InstructionRef {
		offset: paused_at(&mut client).1,
		..start_of("/watched_steps")
	};
	client.resume();
	assert_eq!(client.expect_pause(), changed(steps, "steps", "1", "2"));
	client.resume();
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_at(&mut client), ("/watched_steps".to_owned(), end.offset));
	client.resume();
	expect_returned(&mut client, steps);
	pause(&mut client);
	println!("ok: a frame watch ends when its proc returns through a breakpoint");

	let response = client.request(Request::BreakpointUnset {
		id: breakpoint_id(&end, true)
	});
	assert!(matches!(response, Response::BreakpointUnset { success: true, .. }), "{:?}", response);
	break_exactly(&mut client, &first_change, Some("logged"));
	let steps = watch_steps(&mut client, "steps");
	let logged = client.recv();
	assert!(
		matches!(&logged, Response::Log { message, .. } if message == "logged"),
		"expected the logpoint's message, got {:?}",
		logged
	);
	assert_eq!(client.expect_pause(), changed(steps, "steps", "null", "1"));
	client.resume();
	assert_eq!(client.expect_pause(), changed(steps, "steps", "1", "2"));
	client.resume();
	expect_returned(&mut client, steps);
	println!("ok: a logpoint where a watch stops logs once, with one stop");

	pause(&mut client);
	let response = client.request(Request::BreakpointUnset {
		id: breakpoint_id(&first_change, true)
	});
	assert!(matches!(response, Response::BreakpointUnset { success: true, .. }), "{:?}", response);
	break_exactly(&mut client, &end, None);
	eval(&mut client, None, "spawn()\n\twatched_steps()");
	client.resume();
	expect_breakpoint_in(&mut client, "/watched_steps");
	client.resume();
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_at(&mut client), ("/watched_steps".to_owned(), end.offset));
	let response = client.request(Request::WatchSet {
		place: WatchPlace::Expression {
			frame_id: Some(0),
			expression: "steps".to_owned()
		},
		condition: None,
		hit: None
	});
	let Response::WatchSet(Ok(watch)) = response else {
		panic!("expected a watch on steps, got {:?}", response);
	};
	client.resume();
	expect_returned(&mut client, watch.id);
	println!("ok: a frame watch set at its proc's last instruction ends when the proc returns");

	pause(&mut client);
	let response = client.request(Request::BreakpointUnset {
		id: breakpoint_id(&end, true)
	});
	assert!(matches!(response, Response::BreakpointUnset { success: true, .. }), "{:?}", response);
	set_breakpoint(&mut client, "/watched_throw", None);
	eval(&mut client, None, "spawn()\n\tcatches_watched()");
	client.resume();
	expect_breakpoint_in(&mut client, "/watched_throw");
	let mut watch = |frame_id: u32, expression: &str| {
		let response = client.request(Request::WatchSet {
			place: WatchPlace::Expression {
				frame_id: Some(frame_id),
				expression: expression.to_owned()
			},
			condition: None,
			hit: None
		});
		match response {
			Response::WatchSet(Ok(watch)) => watch.id,
			other => panic!("expected a watch on {}, got {:?}", expression, other)
		}
	};
	let (thrown, caught) = (watch(0, "thrown"), watch(1, "caught"));
	client.resume();
	assert_eq!(client.expect_pause(), changed(thrown, "thrown", "null", "1"));
	client.resume();
	assert_eq!(client.expect_pause(), changed(thrown, "thrown", "1", "2"));
	client.resume();
	let ended = client.recv();
	assert!(
		matches!(&ended, Response::WatchEnded { id, reason } if *id == thrown && reason == "watched proc ended"),
		"expected the watch to end when a throw left its proc, got {:?}",
		ended
	);
	assert_eq!(client.expect_pause(), changed(caught, "caught", "0", "1"));
	client.resume();
	expect_returned(&mut client, caught);
	println!("ok: a frame watch ends when a throw leaves its proc, and the catching frame's watch stays");
}

fn false_condition_on_a_caught_throw() {
	let world = World::start(&host_file("debug_test_host.dmb"), &[]);
	let mut client = world.connect();
	pause(&mut client);
	let throw_caught = start_of("/throw_caught").proc;
	let code = match client.request(Request::ProcInfo { proc: throw_caught.clone() }) {
		Response::ProcInfo(Ok(info)) => info.code,
		other => panic!("expected ProcInfo, got {:?}", other)
	};
	let Response::Disassemble(Ok(disassembly)) = client.request(Request::Disassemble { code }) else {
		panic!("expected the disassembly of /throw_caught");
	};
	let throw = (disassembly.instructions.iter())
		.find(|instruction| instruction.text.starts_with("Throw"))
		.unwrap_or_else(|| panic!("no throw in {:?}", disassembly.instructions));
	let throw_at = InstructionRef {
		proc: throw_caught.clone(),
		offset: throw.offset as u32
	};
	let break_throw = |client: &mut Client, condition: Option<&str>, log: Option<&str>| {
		let response = client.request(Request::BreakpointSet {
			id: breakpoint_id(&throw_at, true),
			instruction: throw_at.clone(),
			condition: condition.map(str::to_owned),
			hit: None,
			log: log.map(LogPart::parse),
			exact: true
		});
		assert!(
			matches!(response, Response::BreakpointSet {
				result: BreakpointSetResult::Success { .. }
			}),
			"a breakpoint on `{}` got {:?}",
			throw.text,
			response
		);
	};
	let caught = BreakpointReason::Runtime {
		message: "thrown and caught".to_owned(),
		caught: true
	};
	break_throw(&mut client, Some("0"), None);
	client.send(Request::CatchRuntimes {
		uncaught: None,
		caught: Some(RuntimeFilter::default())
	});
	eval(&mut client, None, "spawn()\n\tthrow_caught()");
	client.resume();
	assert_eq!(client.expect_pause(), caught);
	assert_eq!(paused_at(&mut client), ("/throw_caught".to_owned(), throw.offset as u32));
	println!("ok: a caught throw under a breakpoint with a false condition stops for caught errors");

	break_throw(&mut client, None, Some("logged"));
	eval(&mut client, None, "spawn()\n\tthrow_caught()");
	client.resume();
	let logged = client.recv();
	assert!(
		matches!(&logged, Response::Log { message, .. } if message == "logged"),
		"expected the logpoint's message, got {:?}",
		logged
	);
	assert_eq!(client.expect_pause(), caught);
	println!("ok: a logpoint on a caught throw logs when the throw stops");

	let response = client.request(Request::BreakpointUnset {
		id: breakpoint_id(&throw_at, true)
	});
	assert!(matches!(response, Response::BreakpointUnset { success: true, .. }), "{:?}", response);
	let before = (disassembly.instructions.iter().rev())
		.find(|instruction| instruction.offset < throw.offset)
		.unwrap_or_else(|| panic!("nothing before the throw in {:?}", disassembly.instructions));
	let before_at = InstructionRef {
		proc: throw_caught.clone(),
		offset: before.offset as u32
	};
	let response = client.request(Request::BreakpointSet {
		id: breakpoint_id(&before_at, true),
		instruction: before_at,
		condition: None,
		hit: None,
		log: None,
		exact: true
	});
	assert!(
		matches!(response, Response::BreakpointSet {
			result: BreakpointSetResult::Success { .. }
		}),
		"a breakpoint on `{}` got {:?}",
		before.text,
		response
	);
	eval(&mut client, None, "spawn()\n\tthrow_caught()");
	client.resume();
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_at(&mut client), ("/throw_caught".to_owned(), before.offset as u32));
	let runtimes = |client: &mut Client| match client.request(Request::Journal {
		after: 0,
		kinds: vec![JournalKind::Runtime],
		limit: None,
		follow: false
	}) {
		Response::Journal { entries, .. } => entries,
		other => panic!("expected the journal, got {:?}", other)
	};
	let seen = runtimes(&mut client).len();
	let response = client.request(Request::Continue {
		kind: ContinueKind::StepOverInstruction { stack_id: 0 }
	});
	assert!(
		matches!(response, Response::Ack),
		"expected Ack for StepOverInstruction, got {:?}",
		response
	);
	assert_eq!(client.expect_pause(), BreakpointReason::Step);
	assert_eq!(paused_at(&mut client), ("/throw_caught".to_owned(), throw.offset as u32));
	client.resume();
	let journaled: Vec<_> = (runtimes(&mut client).into_iter().skip(seen))
		.map(|entry| match entry.event {
			JournalEvent::Runtime { message, caught, .. } => (message, caught),
			other => panic!("expected a runtime error, got {:?}", other)
		})
		.collect();
	assert_eq!(journaled, [("thrown and caught".to_owned(), true)]);
	println!("ok: a caught throw an instruction step stops on is journaled once, with no second stop");
}

fn paused_at(client: &mut Client) -> (String, u32) {
	let response = client.request(Request::StackFrames {
		stack_id: 0,
		start_frame: Some(0),
		count: Some(1)
	});
	let Response::StackFrames { frames, .. } = response else {
		panic!("expected StackFrames, got {:?}", response);
	};
	(frames[0].instruction.proc.path.clone(), frames[0].instruction.offset)
}

fn waiting_world_ends() {
	let world = World::start(&host_file("debug_test_host.dmb"), &[]);
	let mut client = world.connect_silent();
	client.send(Request::Eval {
		frame_id: None,
		command: "world.Del()".to_owned(),
		ending: true
	});
	client.wait_closed();
	let code = world.exit_code();
	let output = world.output();
	assert!(code.is_some() && !output.contains("panic"), "ended with {:?} after {}", code, output);
	println!("ok: an evaluation ends a world that waits for its debugger");
}

fn panics() {
	let panic = || Request::Eval {
		frame_id: None,
		command: "#panic".to_owned(),
		ending: false
	};
	let world = World::start(&host_file("debug_test_host.dmb"), &[]);
	let mut client = world.connect_silent();
	client.send(panic());
	client.wait_closed();
	let mut client = world.connect();
	assert_eq!(status(&mut client), None);
	assert_eq!(eval(&mut client, None, "1 + 1"), "2");
	println!("ok: a request that panics while the world waits for its debugger drops the client, and the world starts");

	client.send(panic());
	client.wait_closed();
	let mut client = world.connect();
	assert_eq!(eval(&mut client, None, "1 + 1"), "2");
	pause(&mut client);
	client.send(panic());
	client.wait_closed();
	let mut client = world.connect();
	assert_eq!(status(&mut client), None);
	assert_eq!(eval(&mut client, None, "1 + 1"), "2");
	let output = world.output();
	assert_eq!(output.matches("asked for with #panic").count(), 3, "{}", output);
	println!("ok: a request that panics while the world runs or is paused drops the client, and the world carries on");
}

fn main() {
	paths::compile(&host_file("debug_test_host.dme"));
	paths::compile(&host_file("debug_test_plain.dme"));

	let world = World::start(&host_file("debug_test_host.dmb"), &[]);

	let dis_command = format!("#dis {}", BREAKPOINT_TARGET);

	let mut client = world.connect();
	let clean_disassembly = eval(&mut client, None, &dis_command);
	set_breakpoint(&mut client, BREAKPOINT_TARGET, None);
	expect_breakpoint_in(&mut client, BREAKPOINT_TARGET);
	println!("ok: breakpoint hit");

	assert_eq!(eval(&mut client, None, "helper.greet(\"you\")"), "\"hi you\"");
	println!("ok: evaluating a verb call");

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
		id: breakpoint_id(&start_of("/proc/does_not_exist"), false)
	});
	assert!(
		matches!(response, Response::BreakpointUnset { success: false, .. }),
		"unsetting a breakpoint on a missing proc got {:?}",
		response
	);
	println!("ok: unsetting a breakpoint on a missing proc");

	set_breakpoint(&mut client, TICK_MARKER, None);
	set_breakpoint(&mut client, BREAKPOINT_TARGET, Some("tick_marker() == 0"));
	client.resume();
	expect_breakpoint_in(&mut client, TICK_MARKER);
	client.resume();
	expect_breakpoint_in(&mut client, TICK_MARKER);
	println!("ok: conditional breakpoint that calls a proc with a breakpoint");

	set_breakpoint(&mut client, BREAKPOINT_TARGET, None);
	client.resume();
	expect_breakpoint_in(&mut client, BREAKPOINT_TARGET);
	println!("ok: re-setting a breakpoint drops its old condition");

	for proc in [BREAKPOINT_TARGET, TICK_MARKER] {
		let response = client.request(Request::BreakpointUnset {
			id: breakpoint_id(&start_of(proc), false)
		});
		assert!(
			matches!(response, Response::BreakpointUnset { success: true, .. }),
			"couldn't unset {}: {:?}",
			proc,
			response
		);
	}

	let client = protocol(&world, client);
	let mut client = profile(&world, client);

	client.send(Request::CatchRuntimes {
		uncaught: None,
		caught: None
	});
	eval(&mut client, None, "start_runtimes()");
	client.resume();
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
	drop(world);

	loop_back_to_the_start();
	watches_at_the_end();
	false_condition_on_a_caught_throw();
	waiting_world_ends();
	panics();

	println!("Debug server tests succeeded");
}
