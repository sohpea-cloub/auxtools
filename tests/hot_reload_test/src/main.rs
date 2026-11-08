//! Edits the source of a running world and hot reloads it the way a watcher
//! would: dmasm compiles what changed, and the debug server swaps it in.

// test_runner already knows how to find BYOND, debug_test has a client, and
// the debug server's message types aren't exported from its cdylib. All of
// them are pulled in as source.
#[allow(dead_code)]
#[path = "../../debug_test/src/client.rs"]
mod client;
#[allow(dead_code)]
#[path = "../../test_runner/src/paths.rs"]
mod paths;
#[allow(dead_code)]
#[path = "../../../debug_server/src/server_types.rs"]
mod server_types;

use std::{
	collections::HashSet,
	net::{Ipv4Addr, TcpListener},
	path::{Path, PathBuf},
	process::{Child, Command},
	time::{Duration, Instant}
};

use client::Client;
use dmasm::reload::{diff, Changes, ParseOptions, Snapshot};
use paths::ByondCommand;
use server_types::*;

struct World(Child);

impl Drop for World {
	fn drop(&mut self) {
		let _ = self.0.kill();
		let _ = self.0.wait();
	}
}

fn find_debug_server() -> PathBuf {
	let mut path = std::env::current_exe().unwrap();
	path.pop();

	#[cfg(unix)]
	path.push("libdebug_server.so");

	#[cfg(windows)]
	path.push("debug_server.dll");

	assert!(path.is_file(), "couldn't find debug_server");
	path
}

fn free_port() -> u16 {
	TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port()
}

/// The world's source, in a copy the test can edit, and how it was last
/// parsed.
struct Source {
	dir: PathBuf,
	options: ParseOptions,
	snapshot: Option<Snapshot>
}

impl Source {
	fn copy_host() -> Source {
		let dir = std::env::temp_dir().join(format!("hot_reload_test_host-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();

		let host = std::env::current_dir().unwrap().join("tests/hot_reload_test_host");
		for file in ["hot_reload_test_host.dme", "main.dm", "reload.dm"] {
			std::fs::copy(host.join(file), dir.join(file)).unwrap();
		}

		Source {
			dir,
			options: ParseOptions::default(),
			snapshot: None
		}
	}

	fn dme(&self) -> PathBuf {
		self.dir.join("hot_reload_test_host.dme")
	}

	fn text(&self, file: &str) -> String {
		std::fs::read_to_string(self.dir.join(file)).unwrap()
	}

	/// The line `needle` is on, counting from 1.
	fn line_of(&self, file: &str, needle: &str) -> u32 {
		let text = self.text(file);
		let at = text.find(needle).unwrap_or_else(|| panic!("{:?} isn't in {}", needle, file));
		text[..at].matches('\n').count() as u32 + 1
	}

	fn parse(&mut self) {
		self.snapshot = Some(Snapshot::parse(&self.dme(), &self.options).unwrap());
	}

	/// Replaces `from` (which has to be there exactly once) with `to`, and
	/// returns what changed since the last parse.
	fn edit(&mut self, file: &str, from: &str, to: &str) -> Changes {
		let text = self.text(file);
		assert_eq!(text.matches(from).count(), 1, "{:?} should be in {} once", from, file);
		std::fs::write(self.dir.join(file), text.replace(from, to)).unwrap();

		let old = self.snapshot.take().expect("parsed before editing");
		self.parse();
		let edited: HashSet<PathBuf> = [Path::new(file).to_owned()].into_iter().collect();
		let changes = diff(&old, self.snapshot.as_ref().unwrap(), Some(&edited));
		assert!(changes.refused.is_empty(), "the edit was refused: {:?}", changes.refused);
		changes
	}
}

impl Drop for Source {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.dir);
	}
}

fn proc_ref(key: &dmasm::reload::ProcKey) -> ProcRef {
	ProcRef {
		path: key.path.clone(),
		override_id: key.override_id
	}
}

fn reload_request(changes: &Changes) -> Request {
	Request::HotReload {
		procs: changes
			.reloads
			.iter()
			.map(|r| HotReloadProc {
				proc: proc_ref(&r.proc),
				nodes: r.nodes.clone(),
				locals: r.locals.clone(),
				params: r.params.clone()
			})
			.collect(),
		line_moves: changes
			.line_moves
			.iter()
			.map(|m| HotReloadLines {
				proc: proc_ref(&m.proc),
				lines: m.lines.clone()
			})
			.collect()
	}
}

fn expect_reload_result(response: Response) -> HotReloadResult {
	match response {
		Response::HotReload(result) => result,
		other => panic!("expected the result of a hot reload, got {:?}", other)
	}
}

fn reload(client: &mut Client, changes: &Changes) -> HotReloadResult {
	let result = expect_reload_result(client.request(reload_request(changes)));
	assert!(result.applied, "the reload wasn't applied: {:?}", result.errors);
	result
}

fn eval(client: &mut Client, command: &str) -> String {
	match client.request(Request::Eval {
		frame_id: None,
		command: command.to_owned(),
		context: None
	}) {
		Response::Eval(eval) => eval.value,
		other => panic!("expected the result of {:?}, got {:?}", command, other)
	}
}

/// Evaluates `command` until it gives `expected`.
fn wait_for(client: &mut Client, command: &str, expected: &str) {
	let start = Instant::now();
	loop {
		let value = eval(client, command);
		if value == expected {
			return;
		}
		assert!(
			start.elapsed() < Duration::from_secs(10),
			"{} is still {}, not {}",
			command,
			value,
			expected
		);
		std::thread::sleep(Duration::from_millis(100));
	}
}

/// The line the top frame of the paused world is on.
fn paused_line(client: &mut Client) -> (String, Option<u32>) {
	match client.request(Request::StackFrames {
		stack_id: 0,
		start_frame: Some(0),
		count: Some(1)
	}) {
		Response::StackFrames { frames, .. } => {
			let frame = frames.into_iter().next().expect("paused with no frames");
			(frame.instruction.proc.path, frame.line)
		}
		other => panic!("expected StackFrames, got {:?}", other)
	}
}

fn offset_of(client: &mut Client, proc: &str, line: u32) -> u32 {
	match client.request(Request::Offset {
		proc: ProcRef {
			path: proc.to_owned(),
			override_id: 0
		},
		line
	}) {
		Response::Offset { offset: Some(offset) } => offset,
		other => panic!("no offset for line {} of {}: {:?}", line, proc, other)
	}
}

fn main() {
	let mut source = Source::copy_host();

	let res = Command::new(paths::find_dm()).with_byond_paths().arg(source.dme()).status().unwrap();
	assert!(res.success(), "dreammaker build failed");

	let port = free_port();
	let _world = World(
		Command::new(paths::find_dreamdaemon())
			.with_byond_paths()
			.env("AUXTOOLS_DEBUG_DLL", find_debug_server())
			.env("AUXTOOLS_DEBUG_PORT", port.to_string())
			.arg(source.dir.join("hot_reload_test_host.dmb"))
			.arg("-trusted")
			.arg("-close")
			.spawn()
			.unwrap()
	);

	let mut client = Client::connect(port);

	// Parsed as DreamMaker does for the BYOND version the world runs on
	let (version, build) = match client.request(Request::ByondVersion) {
		Response::ByondVersion { version, build } => (version, build),
		other => panic!("expected ByondVersion, got {:?}", other)
	};
	println!("ok: the world runs on BYOND {}.{}", version, build);
	source.options.byond = Some((version, build));
	source.parse();

	// New code for a proc
	assert_eq!(eval(&mut client, "answer()"), "1");
	let changes = source.edit("reload.dm", "/proc/answer()\n\treturn 1", "/proc/answer()\n\treturn 2");
	assert_eq!(changes.reloads.len(), 1);
	reload(&mut client, &changes);
	assert_eq!(eval(&mut client, "answer()"), "2");
	println!("ok: new code for a proc");

	// A call that was sleeping when its proc changed finishes on the code it
	// started with. New calls run the new code.
	assert_eq!(eval(&mut client, "sleeper_state"), "\"sleeping\"");
	let changes = source.edit(
		"reload.dm",
		"sleeper_state = \"old code finished\"",
		"sleeper_state = \"new code finished\""
	);
	reload(&mut client, &changes);
	eval(&mut client, "release_sleeper()");
	wait_for(&mut client, "sleeper_state", "\"old code finished\"");
	eval(&mut client, "start_sleeper()");
	wait_for(&mut client, "sleeper_state", "\"new code finished\"");
	println!("ok: a sleeping call finishes on the old code, new calls run the new code");

	// Recursion into the new code
	assert_eq!(eval(&mut client, "factorial(4)"), "24");
	let changes = source.edit("reload.dm", "return n * factorial(n - 1)", "return n * factorial(n - 1) + 1");
	reload(&mut client, &changes);
	assert_eq!(eval(&mut client, "factorial(4)"), "41");
	println!("ok: a recursive proc");

	// More locals than the proc's frames had room for
	assert_eq!(eval(&mut client, "few_locals()"), "1");
	let changes = source.edit(
		"reload.dm",
		"/proc/few_locals()\n\tvar/a = 1\n\treturn a",
		"/proc/few_locals()\n\tvar/a = 1\n\tvar/b = 2\n\tvar/c = 3\n\tvar/d = 4\n\tvar/e = 5\n\tvar/f = 6\n\treturn a + b + c + d + e + f"
	);
	let few_locals = changes.reloads.iter().find(|r| r.proc.path == "/few_locals").unwrap();
	assert_eq!(few_locals.locals.len(), 6);
	reload(&mut client, &changes);
	assert_eq!(eval(&mut client, "few_locals()"), "21");
	println!("ok: growing a proc's locals");

	// A static local keeps its value
	assert_eq!(eval(&mut client, "counter()"), "1");
	assert_eq!(eval(&mut client, "counter()"), "2");
	let changes = source.edit("reload.dm", "\tcount++", "\tcount += 10");
	reload(&mut client, &changes);
	assert_eq!(eval(&mut client, "counter()"), "12");
	println!("ok: a static local keeps its value");

	// A breakpoint stays on its line: with new code it stays on the same line,
	// and when the proc moves it moves with it
	let line = source.line_of("reload.dm", "\tx += 1");
	let old_offset = offset_of(&mut client, "/breakpoint_target", line);
	let response = client.request(Request::BreakpointSet {
		instruction: InstructionRef {
			proc: ProcRef {
				path: "/breakpoint_target".to_owned(),
				override_id: 0
			},
			offset: old_offset
		},
		condition: None
	});
	assert!(matches!(response, Response::BreakpointSet { .. }), "{:?}", response);
	eval(&mut client, "arm_breakpoint()");
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_line(&mut client), ("/breakpoint_target".to_owned(), Some(line)));

	// Two more lines in answer() move everything below it, and the world is
	// paused while it happens
	let changes = source.edit(
		"reload.dm",
		"/proc/answer()\n\treturn 2",
		"/proc/answer()\n\tvar/unused = 1\n\tunused++\n\treturn 3"
	);
	assert!(changes.line_moves.iter().any(|m| m.proc.path == "/breakpoint_target"));
	let result = reload(&mut client, &changes);
	assert!(result.warnings.is_empty(), "{:?}", result.warnings);
	// The paused frame runs the old code, and still knows its line
	assert_eq!(paused_line(&mut client), ("/breakpoint_target".to_owned(), Some(line)));
	client.resume();

	eval(&mut client, "arm_breakpoint()");
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_line(&mut client), ("/breakpoint_target".to_owned(), Some(line + 2)));
	assert_eq!(source.line_of("reload.dm", "\tx += 1"), line + 2);
	client.resume();
	assert_eq!(eval(&mut client, "answer()"), "3");
	println!("ok: a breakpoint moves with its line");

	// The debug-client still knows it by the offset it set it at
	let response = client.request(Request::BreakpointUnset {
		instruction: InstructionRef {
			proc: ProcRef {
				path: "/breakpoint_target".to_owned(),
				override_id: 0
			},
			offset: old_offset
		}
	});
	assert!(matches!(response, Response::BreakpointUnset { success: true }), "{:?}", response);
	eval(&mut client, "arm_breakpoint()");
	wait_for(&mut client, "breakpoint_armed", "0");
	println!("ok: a moved breakpoint can be removed by its old offset");

	// A runtime in new code reports the new code's line
	let changes = source.edit(
		"reload.dm",
		"/proc/runtime_target()\n\treturn 1",
		"/proc/runtime_target()\n\tvar/list/nothing\n\treturn nothing.len"
	);
	reload(&mut client, &changes);
	eval(&mut client, "arm_runtime()");
	match client.expect_pause() {
		BreakpointReason::Runtime(message) => assert!(message.contains("null.len"), "unexpected runtime: {}", message),
		other => panic!("expected a runtime, got {:?}", other)
	}
	let line = source.line_of("reload.dm", "\treturn nothing.len");
	assert_eq!(paused_line(&mut client), ("/runtime_target".to_owned(), Some(line)));
	client.resume();
	println!("ok: a runtime in new code");

	// A proc auxtools hooked never runs its bytecode
	let changes = source.edit(
		"main.dm",
		"/proc/enable_debugging(mode, port)\n\tCRASH(\"auxtools not loaded\")",
		"/proc/enable_debugging(mode, port)\n\tCRASH(\"auxtools still not loaded\")"
	);
	let result = reload(&mut client, &changes);
	assert!(
		result.warnings.iter().any(|w| w.contains("hooked")),
		"no warning about the hook: {:?}",
		result.warnings
	);
	println!("ok: a warning for a hooked proc");

	// Requests sent back to back, the way VS Code sends them
	let changes = source.edit("reload.dm", "\treturn 3", "\treturn 4");
	client.send(reload_request(&changes));
	client.send(Request::Eval {
		frame_id: None,
		command: "answer()".to_owned(),
		context: None
	});
	assert!(expect_reload_result(client.recv()).applied);
	match client.recv() {
		Response::Eval(eval) => assert_eq!(eval.value, "4"),
		other => panic!("expected the result of answer(), got {:?}", other)
	}
	println!("ok: a reload and an eval sent back to back");

	// Something that can't be done leaves everything as it was
	let mut request = reload_request(&source.edit("reload.dm", "\treturn 4", "\treturn 5"));
	if let Request::HotReload { procs, .. } = &mut request {
		procs.push(HotReloadProc {
			params: vec!["x".to_owned()],
			..procs[0].clone()
		});
	}
	let result = expect_reload_result(client.request(request));
	assert!(!result.applied);
	assert!(result.errors.iter().any(|e| e.contains("parameters")), "{:?}", result.errors);
	assert_eq!(eval(&mut client, "answer()"), "4");
	println!("ok: a reload that can't be done in full changes nothing");

	// Shutting auxtools down puts back what the world had, which BYOND frees
	// itself. The world then starts again from its .dmb.
	eval(&mut client, "request_reboot()");
	client.wait_closed();
	// The old world hangs up a moment before it stops listening
	std::thread::sleep(Duration::from_millis(500));
	let mut client = Client::connect(port);
	assert_eq!(eval(&mut client, "answer()"), "1");
	assert_eq!(eval(&mut client, "few_locals()"), "1");
	assert_eq!(eval(&mut client, "counter()"), "1");
	println!("ok: a world reboot after reloads");

	println!("Hot reload tests succeeded");
}
