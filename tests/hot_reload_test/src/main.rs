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

fn proc_code(r: &dmasm::reload::ProcReload) -> HotReloadProc {
	HotReloadProc {
		proc: proc_ref(&r.proc),
		nodes: r.nodes.clone(),
		locals: r.locals.clone(),
		params: r.params.clone(),
		settings: r.settings,
		statics: r.statics.clone()
	}
}

fn reload_request(changes: &Changes) -> Request {
	Request::HotReload(HotReloadRequest {
		procs: changes.reloads.iter().map(proc_code).collect(),
		new_procs: changes
			.new_procs
			.iter()
			.map(|p| HotReloadNewProc {
				type_path: p.type_path.clone(),
				name: p.name.clone(),
				declared: p.declared,
				code: proc_code(&p.code)
			})
			.collect(),
		new_vars: changes.new_vars.clone(),
		var_defaults: changes
			.var_defaults
			.iter()
			.map(|d| HotReloadVarDefault {
				var: d.var.clone(),
				value: d.value.clone(),
				types: d.types.clone()
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
	})
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
	let reload_port = free_port();
	let _world = World(
		Command::new(paths::find_dreamdaemon())
			.with_byond_paths()
			.env("AUXTOOLS_DEBUG_DLL", find_debug_server())
			.env("AUXTOOLS_DEBUG_PORT", port.to_string())
			.env("AUXTOOLS_RELOAD_PORT", reload_port.to_string())
			.arg(source.dir.join("hot_reload_test_host.dmb"))
			.arg("-trusted")
			.arg("-close")
			.spawn()
			.unwrap()
	);

	// A debugger and a watcher, attached at the same time. The watcher sends
	// the reloads, except where the debugger's own connection is tested.
	let mut client = Client::connect(port);
	let mut watcher = Client::connect(reload_port);

	// Parsed as DreamMaker does for the BYOND version the world runs on
	let (version, build) = match watcher.request(Request::ByondVersion) {
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
	reload(&mut watcher, &changes);
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
	reload(&mut watcher, &changes);
	eval(&mut client, "release_sleeper()");
	wait_for(&mut client, "sleeper_state", "\"old code finished\"");
	eval(&mut client, "start_sleeper()");
	wait_for(&mut client, "sleeper_state", "\"new code finished\"");
	println!("ok: a sleeping call finishes on the old code, new calls run the new code");

	// Recursion into the new code
	assert_eq!(eval(&mut client, "factorial(4)"), "24");
	let changes = source.edit("reload.dm", "return n * factorial(n - 1)", "return n * factorial(n - 1) + 1");
	reload(&mut watcher, &changes);
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
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "few_locals()"), "21");
	println!("ok: growing a proc's locals");

	// A static local keeps its value
	assert_eq!(eval(&mut client, "counter()"), "1");
	assert_eq!(eval(&mut client, "counter()"), "2");
	let changes = source.edit("reload.dm", "\tcount++", "\tcount += 10");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "counter()"), "12");
	println!("ok: a static local keeps its value");

	// The watcher doesn't need a debugger there
	client.send(Request::Disconnect);
	client.wait_closed();
	let changes = source.edit("reload.dm", "/proc/answer()\n\treturn 2", "/proc/answer()\n\treturn 20");
	reload(&mut watcher, &changes);
	let mut client = Client::connect(port);
	assert_eq!(eval(&mut client, "answer()"), "20");
	let changes = source.edit("reload.dm", "/proc/answer()\n\treturn 20", "/proc/answer()\n\treturn 2");
	reload(&mut watcher, &changes);
	println!("ok: reloading with no debugger attached");

	// A breakpoint stays on its line, and the watcher can reload while the
	// debugger has the world paused.: with new code it stays on the same line,
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
	let result = reload(&mut watcher, &changes);
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
	reload(&mut watcher, &changes);
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
	let result = reload(&mut watcher, &changes);
	assert!(
		result.warnings.iter().any(|w| w.contains("hooked")),
		"no warning about the hook: {:?}",
		result.warnings
	);
	println!("ok: a warning for a hooked proc");

	// The debugger's own connection takes reloads too. Requests sent back to
	// back, the way VS Code sends them.
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

	// A new global proc, called from a changed one
	let changes = source.edit(
		"reload.dm",
		"/proc/uses_helper()\n\treturn 0",
		"/proc/uses_helper()\n\treturn helper() + 1\n\n/proc/helper()\n\treturn 7"
	);
	assert_eq!(changes.new_procs.len(), 1);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "uses_helper()"), "8");
	assert_eq!(eval(&mut client, "helper()"), "7");
	// With its lines, as the world was built with DEBUG
	let response = client.request(Request::Offset {
		proc: ProcRef {
			path: "/helper".to_owned(),
			override_id: 0
		},
		line: source.line_of("reload.dm", "\treturn 7")
	});
	assert!(matches!(response, Response::Offset { offset: Some(_) }), "{:?}", response);
	println!("ok: a new global proc");

	// A new proc on a type: existing instances of it and its subtypes have it
	let changes = source.edit(
		"reload.dm",
		"/datum/animal/proc/speak()\n\treturn sound",
		"/datum/animal/proc/speak()\n\treturn sound\n\n/datum/animal/proc/describe()\n\treturn \"[sound] on [legs] legs\""
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.describe()"), "\"woof on 4 legs\"");
	println!("ok: a new proc on a type");

	// A new override between a proc and a subtype's override of it: the
	// subtype's ..() goes through it now
	assert_eq!(eval(&mut client, "old_puppy.speak()"), "\"woof?\"");
	let changes = source.edit(
		"reload.dm",
		"/datum/animal/dog\n\tsound = \"woof\"\n",
		"/datum/animal/dog\n\tsound = \"woof\"\n\n/datum/animal/dog/speak()\n\treturn \"[..()]!\"\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.speak()"), "\"woof!\"");
	assert_eq!(eval(&mut client, "old_puppy.speak()"), "\"woof!?\"");
	println!("ok: a new override, and a subtype's ..() through it");

	// A new var default reaches instances that never set the var, new ones,
	// and subtypes that inherit it, but not a subtype that sets its own
	eval(&mut client, "write_legs()");
	let changes = source.edit("reload.dm", "\tvar/legs = 4", "\tvar/legs = 3");
	assert_eq!(changes.var_defaults.len(), 1);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.legs"), "3");
	assert_eq!(eval(&mut client, "new_dog_legs()"), "3");
	assert_eq!(eval(&mut client, "legs_written.legs"), "9");
	assert_eq!(eval(&mut client, "new_bird_legs()"), "2");
	assert_eq!(eval(&mut client, "initial(old_dog.legs)"), "3");
	println!("ok: a new var default");

	// New vars: an instance var, overridden by a subtype, a static var, a tmp
	// var and a global, used by new procs. Existing instances have them.
	let changes = source.edit(
		"reload.dm",
		"\tvar/legs = 3\n",
		"\tvar/legs = 3\n\tvar/tail = 1\n\tvar/static/herd = 0\n\tvar/tmp/mood = \"calm\"\n\n/datum/animal/bird\n\ttail = 0\n\nvar/flock_size = \
		 12\n\n/datum/animal/proc/join_herd()\n\therd++\n\treturn \"[herd] [mood] [tail] \
		 [flock_size]\"\n\n/proc/new_bird_tail()\n\tvar/datum/animal/bird/b = new\n\treturn b.tail\n"
	);
	assert_eq!(changes.new_vars.len(), 4, "{:?}", changes.new_vars);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.tail"), "1");
	assert_eq!(eval(&mut client, "initial(old_dog.tail)"), "1");
	assert_eq!(eval(&mut client, "new_bird_tail()"), "0");
	assert_eq!(eval(&mut client, "old_dog.join_herd()"), "\"1 calm 1 12\"");
	assert_eq!(eval(&mut client, "old_puppy.join_herd()"), "\"2 calm 1 12\"");
	assert_eq!(eval(&mut client, "old_dog.herd"), "2");
	assert_eq!(eval(&mut client, "flock_size"), "12");
	assert_eq!(eval(&mut client, "(\"tail\" in old_dog.vars)"), "1");
	println!("ok: new vars");

	// The default of a var a reload added
	let changes = source.edit("reload.dm", "\tvar/tail = 1\n", "\tvar/tail = 2\n");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.tail"), "2");
	assert_eq!(eval(&mut client, "new_bird_tail()"), "0");
	println!("ok: the default of a new var");

	// A new parameter, by name too
	let changes = source.edit(
		"reload.dm",
		"/proc/greet(n)\n\treturn \"hi [n]\"\n\n/proc/greet_named()\n\treturn \"unchanged\"",
		"/proc/greet(n, sound = \"!\")\n\treturn \"hi [n][sound]\"\n\n/proc/greet_named()\n\treturn greet(\"bo\", sound = \"?\")"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "greet(\"al\")"), "\"hi al!\"");
	assert_eq!(eval(&mut client, "greet_named()"), "\"hi bo?\"");
	println!("ok: a new parameter");

	// A parameter whose name nothing in the world had
	let changes = source.edit(
		"reload.dm",
		"/proc/greet(n, sound = \"!\")\n\treturn \"hi [n][sound]\"\n\n/proc/greet_named()\n\treturn greet(\"bo\", sound = \"?\")",
		"/proc/greet(n, sound = \"!\", unheard_of = \"\")\n\treturn \"hi [n][sound][unheard_of]\"\n\n/proc/greet_named()\n\treturn greet(\"bo\", \
		 unheard_of = \"~\")"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "greet_named()"), "\"hi bo!~\"");
	println!("ok: a parameter with a name the world didn't have");

	// New static locals: one in a proc that has another, and one in a new proc
	let before: u32 = eval(&mut client, "counter()").parse().unwrap();
	let changes = source.edit(
		"reload.dm",
		"\tvar/static/count = 0\n\tcount += 10\n\treturn count",
		"\tvar/static/count = 0\n\tvar/static/bonus = 100\n\tcount += 10\n\treturn count + bonus\n\n/proc/visits()\n\tvar/static/v = \
		 10\n\tv++\n\treturn v"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "counter()"), (before + 110).to_string());
	assert_eq!(eval(&mut client, "visits()"), "11");
	assert_eq!(eval(&mut client, "visits()"), "12");
	println!("ok: new static locals");

	// More definitions of a proc on a type that has one: one more of the dog's
	// speak(), which the puppy's ..() goes through now, and two for the cat
	// in one reload, the second calling the first
	let changes = source.edit(
		"reload.dm",
		"/proc/caller_sees()\n\tvar/x = slow()\n\treturn isnull(x)\n",
		"/proc/caller_sees()\n\tvar/x = slow()\n\treturn isnull(x)\n\n/datum/animal/dog/speak()\n\treturn \
		 \"[..()]!!\"\n\n/datum/animal/cat/speak()\n\treturn \"[..()]~\"\n\n/datum/animal/cat/speak()\n\treturn \
		 uppertext(..())\n\n/proc/cat_says()\n\tvar/datum/animal/cat/c = new\n\treturn c.speak()\n"
	);
	assert_eq!(
		changes.new_procs.len(),
		4,
		"{:?}",
		changes.new_procs.iter().map(|p| p.code.proc.to_string()).collect::<Vec<_>>()
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.speak()"), "\"woof!!!\"");
	assert_eq!(eval(&mut client, "old_puppy.speak()"), "\"woof!!!?\"");
	assert_eq!(eval(&mut client, "cat_says()"), "\"MEOW~\"");
	println!("ok: more definitions of a proc on a type");

	// A new parameter next to `in` lists: one BYOND has built in, and an
	// expression it keeps
	let changes = source.edit(
		"reload.dm",
		"/proc/in_lists(mob/m in world, o in global.vars)\n\treturn 1",
		"/proc/in_lists(mob/m in world, o in global.vars, n = 5)\n\treturn n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "in_lists()"), "5");
	assert_eq!(eval(&mut client, "in_lists(n = 6)"), "6");
	println!("ok: a new parameter next to `in` lists");

	// set waitfor = 0: the caller no longer waits for the sleep
	let before = eval(&mut client, "caller_sees()");
	let changes = source.edit("reload.dm", "/proc/slow()\n\tsleep(1)", "/proc/slow()\n\tset waitfor = 0\n\tsleep(1)");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "caller_sees()"), "1", "before the change it was {}", before);
	println!("ok: set waitfor");

	// Something that can't be done leaves everything as it was
	let mut request = reload_request(&source.edit("reload.dm", "\treturn 4", "\treturn 5"));
	if let Request::HotReload(HotReloadRequest { procs, .. }) = &mut request {
		let mut bad = procs[0].clone();
		bad.proc.path = "/a_proc_the_world_doesnt_have".to_owned();
		procs.push(bad);
	}
	let result = expect_reload_result(client.request(request));
	assert!(!result.applied);
	assert!(
		result.errors.iter().any(|e| e.contains("a_proc_the_world_doesnt_have")),
		"{:?}",
		result.errors
	);
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
	assert_eq!(eval(&mut client, "old_dog.legs"), "4");
	println!("ok: a world reboot after reloads");

	println!("Hot reload tests succeeded");
}
