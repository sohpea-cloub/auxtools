use std::{
	collections::HashSet,
	path::{Path, PathBuf},
	sync::atomic::{AtomicBool, Ordering},
	time::{Duration, Instant}
};

use auxtools_protocol::*;
use dmasm::reload::{diff, Changes, ParseOptions, Snapshot};
use test_support::{
	debug::{Client, World},
	paths,
	temp::TempDir
};

struct Source {
	dir: TempDir,
	options: ParseOptions,
	snapshot: Option<Snapshot>
}

impl Source {
	fn copy_host(section: &str) -> Source {
		let source = Source {
			dir: TempDir::new(&format!("hot_reload_test_{}", section)),
			options: ParseOptions::default(),
			snapshot: None
		};
		source.copy_files();
		source
	}

	fn copy_files(&self) {
		let host = std::env::current_dir().unwrap().join("tests/hot_reload_test_host");
		for file in [
			"hot_reload_test_host.dme",
			"main.dm",
			"reload.dm",
			"resources.dm",
			"lamp.dmi",
			"lamp_edited.dmi",
			"lamp_third.dmi",
			"lamp_built.dmi",
			"crate.dmi",
			"crate_edited.dmi",
			"beep.wav",
			"beep_edited.wav"
		] {
			std::fs::copy(host.join(file), self.dir.join(file)).unwrap();
		}
	}

	fn restore(&mut self) {
		self.copy_files();
		self.snapshot = None;
		self.parse();
	}

	fn dme(&self) -> PathBuf {
		self.dir.join("hot_reload_test_host.dme")
	}

	fn text(&self, file: &str) -> String {
		std::fs::read_to_string(self.dir.join(file)).unwrap()
	}

	fn line_of(&self, file: &str, needle: &str) -> u32 {
		let text = self.text(file);
		let at = text.find(needle).unwrap_or_else(|| panic!("{:?} isn't in {}", needle, file));
		text[..at].matches('\n').count() as u32 + 1
	}

	fn parse(&mut self) {
		self.snapshot = Some(Snapshot::parse(&self.dme(), &self.options).unwrap());
	}

	fn edit(&mut self, file: &str, from: &str, to: &str) -> Edit {
		self.edit_many(file, &[(from, to)])
	}

	fn edit_many(&mut self, file: &str, edits: &[(&str, &str)]) -> Edit {
		let mut text = self.text(file);
		for (from, to) in edits {
			assert_eq!(text.matches(from).count(), 1, "{:?} should be in {} once", from, file);
			text = text.replace(from, to);
		}
		std::fs::write(self.dir.join(file), text).unwrap();

		let old = self.snapshot.take().expect("parsed before editing");
		self.parse();
		let edited: HashSet<PathBuf> = [Path::new(file).to_owned()].into_iter().collect();
		let changes = diff(&old, self.snapshot.as_ref().unwrap(), Some(&edited));
		assert!(changes.refused.is_empty(), "the edit was refused: {:?}", changes.refused);
		Edit { changes, before: old }
	}

	fn edit_refused(&mut self, file: &str, from: &str, to: &str) -> Vec<String> {
		let text = self.text(file);
		assert_eq!(text.matches(from).count(), 1, "{:?} should be in {} once", from, file);
		std::fs::write(self.dir.join(file), text.replace(from, to)).unwrap();
		let old = self.snapshot.take().expect("parsed before editing");
		self.parse();
		let edited: HashSet<PathBuf> = [Path::new(file).to_owned()].into_iter().collect();
		let changes = diff(&old, self.snapshot.as_ref().unwrap(), Some(&edited));
		assert!(!changes.refused.is_empty(), "the edit wasn't refused");
		changes.refused.iter().map(|refusal| format!("{:#}", refusal)).collect()
	}

	fn move_code(&mut self, code: &str, from: &str, to: &str) -> Edit {
		let text = self.text(from);
		assert_eq!(text.matches(code).count(), 1, "{:?} should be in {} once", code, from);
		std::fs::write(self.dir.join(from), text.replace(code, "")).unwrap();
		std::fs::write(self.dir.join(to), format!("{}\n{}", self.text(to), code)).unwrap();
		let old = self.snapshot.take().expect("parsed before editing");
		self.parse();
		let edited: HashSet<PathBuf> = [Path::new(from).to_owned(), Path::new(to).to_owned()].into_iter().collect();
		let changes = diff(&old, self.snapshot.as_ref().unwrap(), Some(&edited));
		assert!(changes.refused.is_empty(), "the edit was refused: {:?}", changes.refused);
		Edit { changes, before: old }
	}
}

struct Edit {
	changes: Changes,
	before: Snapshot
}

impl std::ops::Deref for Edit {
	type Target = Changes;

	fn deref(&self) -> &Changes {
		&self.changes
	}
}

fn reload_request(edit: &Edit) -> Request {
	Request::HotReload(auxtools_cli::reload::request(&edit.changes, &edit.before, false))
}

fn resource_id(reference: &str) -> u32 {
	let hex = reference.trim_matches('"').trim_start_matches("[0x").trim_end_matches(']');
	u32::from_str_radix(hex, 16).unwrap_or_else(|_| panic!("{} isn't a resource reference", reference)) & 0xFF_FFFF
}

fn reload_resource(client: &mut Client, source: &Source, file: &str, edited: &str, ids: Vec<u32>) -> HotReloadResult {
	reload_resources(client, source, &[(file, edited, ids)])
}

fn reload_resources(client: &mut Client, source: &Source, files: &[(&str, &str, Vec<u32>)]) -> HotReloadResult {
	for (file, edited, _) in files {
		std::fs::copy(source.dir.join(edited), source.dir.join(file)).unwrap();
	}
	let names: Vec<String> = files.iter().map(|(file, ..)| file.to_string()).collect();
	let current = source.snapshot.as_ref().unwrap();
	let changes = current.resource_users(&names);
	assert!(changes.refused.is_empty(), "the resource reload was refused: {:?}", changes.refused);
	let mut request = auxtools_cli::reload::request(&changes, current, false);
	request.resources = files
		.iter()
		.map(|(file, _, ids)| HotReloadResource {
			path: file.to_string(),
			ids: ids.clone()
		})
		.collect();
	let result = expect_reload_result(client.request(Request::HotReload(request)));
	assert!(result.applied, "resource reload rejected: {:?}", result.errors);
	result
}

fn expect_reload_result(response: Response) -> HotReloadResult {
	match response {
		Response::HotReload(result) => result,
		other => panic!("expected the result of a hot reload, got {:?}", other)
	}
}

fn reload(client: &mut Client, changes: &Edit) -> HotReloadResult {
	let result = expect_reload_result(client.request(reload_request(changes)));
	assert!(result.applied, "reload rejected: {:?}", result.errors);
	result
}

fn eval(client: &mut Client, command: &str) -> String {
	match client.request(Request::Eval {
		frame_id: None,
		command: command.to_owned(),
		ending: false
	}) {
		Response::Eval(Ok(eval)) => eval.value,
		other => panic!("expected the result of {:?}, got {:?}", command, other)
	}
}

fn sleeps(client: &mut Client, command: &str) -> bool {
	match client.request(Request::Eval {
		frame_id: None,
		command: command.to_owned(),
		ending: false
	}) {
		Response::Eval(Ok(_)) => false,
		Response::Eval(Err(why)) if why.still_running && why.message.starts_with("evaluation aborted because the code slept") => true,
		other => panic!("expected the result of {:?}, got {:?}", command, other)
	}
}

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

fn procs_on(client: &mut Client, file: &str, line: u32) -> Vec<String> {
	located(client, file, line).into_iter().map(|at| at.proc.path).collect()
}

fn located(client: &mut Client, file: &str, line: u32) -> Vec<InstructionRef> {
	match client.request(Request::Locate { file: file.to_owned(), line }) {
		Response::Locate { instructions } => instructions,
		other => panic!("expected Locate for {}:{}, got {:?}", file, line, other)
	}
}

fn lines_with_code(client: &mut Client, file: &str, start: u32, end: u32) -> Vec<u32> {
	match client.request(Request::LinesWithCode {
		file: file.to_owned(),
		start,
		end
	}) {
		Response::LinesWithCode { lines } => lines,
		other => panic!("expected LinesWithCode for {}, got {:?}", file, other)
	}
}

const SECTIONS: &[(&str, fn(Built))] = &[
	("procs", procs),
	("debugger", debugger),
	("definitions", definitions),
	("types", types),
	("declarations", declarations),
	("resources", resources)
];

fn main() {
	let wanted: Vec<String> = std::env::args().skip(1).collect();
	if wanted == ["--list"] {
		for (section, _) in SECTIONS {
			println!("{}", section);
		}
		return;
	}
	for name in &wanted {
		assert!(
			SECTIONS.iter().any(|(section, _)| section == name),
			"there's no section {}; the sections are {:?}",
			name,
			SECTIONS.iter().map(|(section, _)| *section).collect::<Vec<_>>()
		);
	}
	let chosen: Vec<_> = SECTIONS
		.iter()
		.filter(|(name, _)| wanted.is_empty() || wanted.iter().any(|w| w == name))
		.collect();
	let mut next = chosen.first().map(|(name, _)| build(name));
	for (i, (name, run)) in chosen.iter().enumerate() {
		let built = next.take().unwrap();
		next = chosen.get(i + 1).map(|(name, _)| build(name));
		println!("--- {}", name);
		run(built);
	}
	println!("Hot reload tests succeeded");
}

struct Built {
	world: World,
	source: Source
}

fn build(section: &str) -> Built {
	let source = Source::copy_host(section);
	paths::compile(&source.dme());
	let world = World::start(&source.dir.join("hot_reload_test_host.dmb"), &[]);
	Built { world, source }
}

static VERSION_SHOWN: AtomicBool = AtomicBool::new(false);

fn start(built: Built) -> (Source, World, Client, Client) {
	let Built { world, mut source } = built;

	let mut client = world.connect();
	client.send(Request::CatchRuntimes {
		uncaught: Some(Default::default()),
		caught: None
	});
	let mut watcher = world.connect_watcher();

	let (version, build) = match watcher.request(Request::ByondVersion) {
		Response::ByondVersion { version, build } => (version, build),
		other => panic!("expected ByondVersion, got {:?}", other)
	};
	if !VERSION_SHOWN.swap(true, Ordering::Relaxed) {
		println!("ok: BYOND {}.{}", version, build);
	}
	source.options.byond = Some((version, build));
	source.parse();
	(source, world, client, watcher)
}

fn procs(built: Built) {
	let (mut source, _world, mut client, mut watcher) = start(built);

	assert_eq!(eval(&mut client, "answer()"), "1");
	let changes = source.edit("reload.dm", "/proc/answer()\n\treturn 1", "/proc/answer()\n\treturn 2");
	assert_eq!(changes.plan.reloads.len(), 1);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "answer()"), "2");
	println!("ok: proc bytecode replacement");

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
	println!("ok: active calls retain old bytecode");

	assert_eq!(eval(&mut client, "factorial(4)"), "24");
	let changes = source.edit("reload.dm", "return n * factorial(n - 1)", "return n * factorial(n - 1) + 1");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "factorial(4)"), "41");
	println!("ok: recursive proc reload");

	assert_eq!(eval(&mut client, "few_locals()"), "1");
	let changes = source.edit(
		"reload.dm",
		"/proc/few_locals()\n\tvar/a = 1\n\treturn a",
		"/proc/few_locals()\n\tvar/a = 1\n\tvar/b = 2\n\tvar/c = 3\n\tvar/d = 4\n\tvar/e = 5\n\tvar/f = 6\n\treturn a + b + c + d + e + f"
	);
	let few_locals = changes.plan.reloads.iter().find(|r| r.proc.path == "/few_locals").unwrap();
	assert_eq!(few_locals.locals.len(), 6);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "few_locals()"), "21");
	println!("ok: expanded locals table");

	assert_eq!(eval(&mut client, "counter()"), "1");
	assert_eq!(eval(&mut client, "counter()"), "2");
	let changes = source.edit("reload.dm", "\tcount++", "\tcount += 10");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "counter()"), "12");
	println!("ok: static local value preservation");
}

fn debugger(built: Built) {
	let (mut source, world, mut client, mut watcher) = start(built);

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
		condition: None,
		hit: None,
		log: None,
		exact: false
	});
	assert!(matches!(response, Response::BreakpointSet { .. }), "{:?}", response);
	eval(&mut client, "arm_breakpoint()");
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_line(&mut client), ("/breakpoint_target".to_owned(), Some(line)));

	let changes = source.edit(
		"reload.dm",
		"/proc/answer()\n\treturn 1",
		"/proc/answer()\n\tvar/unused = 1\n\tunused++\n\treturn 3"
	);
	assert!(changes.plan.line_moves.iter().any(|m| m.proc.path == "/breakpoint_target"));
	let result = reload(&mut watcher, &changes);
	assert!(result.warnings.is_empty(), "{:?}", result.warnings);
	assert_eq!(paused_line(&mut client), ("/breakpoint_target".to_owned(), Some(line + 2)));
	fn sleeping_stacks(client: &mut Client) -> (usize, u32) {
		match client.request(Request::Stacks { sleeping: Some(1000) }) {
			Response::Stacks { stacks, sleeping } => (stacks.iter().filter(|stack| stack.sleeping).count(), sleeping),
			other => panic!("expected Stacks, got {:?}", other)
		}
	}
	let (listed, asleep) = sleeping_stacks(&mut client);
	assert_eq!(listed, asleep as usize);
	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n\treturn \"first\"",
		"/proc/first_def()\n\treturn \"first\"\n\nvar/global/spawned_by_reload = \
		 sleep_from_reload()\n\n/proc/sleep_from_reload()\n\tspawn(0)\n\t\tsleep(6000)\n\treturn 1"
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		sleeping_stacks(&mut client),
		(listed + 1, asleep + 1),
		"the stacks shown after a reload while paused aren't the world's"
	);
	println!("ok: stacks after a paused reload");
	client.resume();

	eval(&mut client, "arm_breakpoint()");
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_line(&mut client), ("/breakpoint_target".to_owned(), Some(line + 2)));
	assert_eq!(source.line_of("reload.dm", "\tx += 1"), line + 2);
	client.resume();
	assert_eq!(eval(&mut client, "answer()"), "3");
	println!("ok: breakpoint line remapping");

	let response = client.request(Request::BreakpointUnset {
		instruction: InstructionRef {
			proc: ProcRef {
				path: "/breakpoint_target".to_owned(),
				override_id: 0
			},
			offset: old_offset
		},
		exact: false
	});
	assert!(matches!(response, Response::BreakpointUnset { success: true, .. }), "{:?}", response);
	eval(&mut client, "arm_breakpoint()");
	wait_for(&mut client, "breakpoint_armed", "0");
	println!("ok: remove remapped breakpoint by original offset");

	let line = source.line_of("reload.dm", "\ty += 1");
	let offset = offset_of(&mut client, "/caught_target", line);
	let at = InstructionRef {
		proc: ProcRef {
			path: "/caught_target".to_owned(),
			override_id: 0
		},
		offset
	};
	let response = client.request(Request::BreakpointSet {
		instruction: at.clone(),
		condition: None,
		hit: None,
		log: None,
		exact: false
	});
	assert!(matches!(response, Response::BreakpointSet { .. }), "{:?}", response);
	eval(&mut client, "caught_armed = TRUE");
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	let changes = source.edit(
		"reload.dm",
		"var/caught_done = 0",
		"var/caught_done = 0\nvar/init_fails = fails_now()\n\n/proc/fails_now()\n\tCRASH(\"initializer failed\")"
	);
	let result = reload(&mut watcher, &changes);
	assert!(result.applied, "{:?}", result);
	let response = client.request(Request::BreakpointUnset {
		instruction: at,
		exact: false
	});
	assert!(matches!(response, Response::BreakpointUnset { success: true, .. }), "{:?}", response);
	client.resume();
	wait_for(&mut client, "caught_done", "2");
	assert_eq!(eval(&mut client, "caught_by_tick"), "null");
	println!("ok: initializer error isolation");

	let line = source.line_of("reload.dm", "\tx += 1");
	assert_eq!(procs_on(&mut client, "reload.dm", line), ["/breakpoint_target"]);
	let offset = offset_of(&mut client, "/breakpoint_target", line);
	let at = InstructionRef {
		proc: ProcRef {
			path: "/breakpoint_target".to_owned(),
			override_id: 0
		},
		offset
	};
	let response = client.request(Request::BreakpointSet {
		instruction: at.clone(),
		condition: None,
		hit: None,
		log: None,
		exact: false
	});
	assert!(matches!(response, Response::BreakpointSet { .. }), "{:?}", response);
	const TARGET: &str = "/proc/breakpoint_target()\n\tvar/x = 1\n\tx += 1\n\treturn x\n";
	let changes = source.move_code(TARGET, "reload.dm", "main.dm");
	let result = reload(&mut watcher, &changes);
	assert!(result.warnings.is_empty(), "{:?}", result.warnings);
	eval(&mut client, "arm_breakpoint()");
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(
		paused_line(&mut client),
		("/breakpoint_target".to_owned(), Some(source.line_of("main.dm", "\tx += 1")))
	);
	client.resume();
	let response = client.request(Request::BreakpointUnset {
		instruction: at,
		exact: false
	});
	assert!(matches!(response, Response::BreakpointUnset { success: true, .. }), "{:?}", response);
	println!("ok: breakpoints move with a proc to another file");

	let moved = source.line_of("main.dm", "\tx += 1");
	assert_eq!(procs_on(&mut client, "main.dm", moved), ["/breakpoint_target"]);
	assert_eq!(lines_with_code(&mut client, "main.dm", moved, moved), [moved]);
	let left = procs_on(&mut client, "reload.dm", line);
	assert!(!left.iter().any(|proc| proc == "/breakpoint_target"), "{:?}", left);
	let changes = source.move_code(TARGET, "main.dm", "reload.dm");
	reload(&mut watcher, &changes);
	let back = source.line_of("reload.dm", "\tx += 1");
	assert_eq!(procs_on(&mut client, "reload.dm", back), ["/breakpoint_target"]);
	let left = procs_on(&mut client, "main.dm", moved);
	assert!(!left.iter().any(|proc| proc == "/breakpoint_target"), "{:?}", left);
	println!("ok: a proc moved to another file is located there");

	let stops_on = |client: &mut Client, at: &InstructionRef, line: u32| {
		let response = client.request(Request::BreakpointSet {
			instruction: at.clone(),
			condition: None,
			hit: None,
			log: None,
			exact: false
		});
		assert!(matches!(response, Response::BreakpointSet { .. }), "{:?}", response);
		eval(client, "arm_breakpoint()");
		assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
		assert_eq!(paused_line(client), ("/breakpoint_target".to_owned(), Some(line)));
		client.resume();
		let response = client.request(Request::BreakpointUnset {
			instruction: at.clone(),
			exact: false
		});
		assert!(matches!(response, Response::BreakpointUnset { success: true, .. }), "{:?}", response);
	};
	let line = source.line_of("reload.dm", "\tx += 1");
	assert_eq!(lines_with_code(&mut client, "reload.dm", line - 1, line + 3), [line - 1, line, line + 1]);
	let before = located(&mut client, "reload.dm", line);
	assert_eq!(before.len(), 1, "{:?}", before);
	stops_on(&mut client, &before[0], line);
	let changes = source.edit("reload.dm", "\tvar/x = 1\n\tx += 1", "\tvar/x = 1\n\tx *= 3\n\tx += 1");
	reload(&mut watcher, &changes);
	assert_eq!(lines_with_code(&mut client, "reload.dm", line - 1, line + 3), [
		line - 1,
		line,
		line + 1,
		line + 2
	]);
	assert_eq!(located(&mut client, "reload.dm", line), before);
	let after = located(&mut client, "reload.dm", line + 1);
	assert_eq!(after.len(), 1, "{:?}", after);
	assert_eq!(after[0].proc, before[0].proc);
	assert!(after[0].offset > before[0].offset, "{:?} isn't after {:?}", after, before);
	stops_on(&mut client, &after[0], line + 1);
	println!("ok: lines are located in a proc's new code");

	let changes = source.edit(
		"reload.dm",
		"/proc/breakpoint_target()\n",
		"// two lines that\n// move the proc down\n/proc/breakpoint_target()\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(lines_with_code(&mut client, "reload.dm", line - 1, line + 5), [
		line + 1,
		line + 2,
		line + 3,
		line + 4
	]);
	assert_eq!(located(&mut client, "reload.dm", line + 3), after);
	stops_on(&mut client, &after[0], line + 3);
	let changes = source.edit_many("reload.dm", &[
		(
			"// two lines that\n// move the proc down\n/proc/breakpoint_target()\n",
			"/proc/breakpoint_target()\n"
		),
		("\tvar/x = 1\n\tx *= 3\n\tx += 1", "\tvar/x = 1\n\tx += 1")
	]);
	reload(&mut watcher, &changes);
	assert_eq!(located(&mut client, "reload.dm", line), before);
	assert_eq!(lines_with_code(&mut client, "reload.dm", line - 1, line + 5), [line - 1, line, line + 1]);
	println!("ok: lines are located where a reload moved them");

	let breakpoint = |client: &mut Client, offset: u32, set: bool| {
		let instruction = InstructionRef {
			proc: ProcRef {
				path: "/breakpoint_target".to_owned(),
				override_id: 0
			},
			offset
		};
		match client.request(if set {
			Request::BreakpointSet {
				instruction,
				condition: None,
				hit: None,
				log: None,
				exact: false
			}
		} else {
			Request::BreakpointUnset { instruction, exact: false }
		}) {
			Response::BreakpointSet {
				result: BreakpointSetResult::Success { .. }
			}
			| Response::BreakpointUnset { success: true, .. } => {}
			other => panic!(
				"couldn't {} the breakpoint at {}: {:?}",
				if set { "set" } else { "take away" },
				offset,
				other
			)
		}
	};
	let changes = source.edit("reload.dm", "\tvar/x = 1\n\tx += 1", "\tvar/x = 1\n\n\tx += 1");
	reload(&mut watcher, &changes);
	let mut other = world.connect();
	let line = source.line_of("reload.dm", "\tx += 1");
	let old_offset = offset_of(&mut client, "/breakpoint_target", line);
	breakpoint(&mut client, old_offset, true);
	breakpoint(&mut other, old_offset, true);
	let changes = source.edit("reload.dm", "\tvar/x = 1\n\n\tx += 1", "\tvar/x = 1\n\tx *= 1\n\tx += 1");
	let result = reload(&mut watcher, &changes);
	assert!(result.warnings.is_empty(), "{:?}", result.warnings);
	let new_offset = offset_of(&mut client, "/breakpoint_target", line);
	assert_eq!(
		offset_of(&mut client, "/breakpoint_target", line - 1),
		old_offset,
		"the new line isn't where the old one was"
	);
	assert_ne!(new_offset, old_offset);
	breakpoint(&mut client, old_offset, false);
	breakpoint(&mut client, new_offset, true);
	breakpoint(&mut client, old_offset, true);
	breakpoint(&mut client, old_offset, false);
	eval(&mut client, "arm_breakpoint()");
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(other.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_line(&mut client), ("/breakpoint_target".to_owned(), Some(line)));
	client.resume();
	breakpoint(&mut other, old_offset, false);
	other.leave();
	breakpoint(&mut client, new_offset, false);
	eval(&mut client, "arm_breakpoint()");
	wait_for(&mut client, "breakpoint_armed", "0");
	let changes = source.edit("reload.dm", "\tvar/x = 1\n\tx *= 1\n\tx += 1", "\tvar/x = 1\n\tx += 1");
	reload(&mut watcher, &changes);
	println!("ok: breakpoint aliases are client-specific");

	let line = source.line_of("resources.dm", "\treturn \"last [");
	let old_offset = offset_of(&mut client, "/last_in_file", line);
	let last_in_file = |offset: u32| InstructionRef {
		proc: ProcRef {
			path: "/last_in_file".to_owned(),
			override_id: 0
		},
		offset
	};
	let set = |client: &mut Client, offset: u32| {
		let response = client.request(Request::BreakpointSet {
			instruction: last_in_file(offset),
			condition: None,
			hit: None,
			log: None,
			exact: false
		});
		assert!(
			matches!(response, Response::BreakpointSet {
				result: BreakpointSetResult::Success { .. }
			}),
			"{:?}",
			response
		);
	};
	set(&mut client, old_offset);
	let dme_lines = lines_with_code(&mut client, "hot_reload_test_host.dme", 1, 10_000);
	assert!(!dme_lines.is_empty(), "the .dme's place isn't among /last_in_file's lines");
	let changes = source.edit(
		"resources.dm",
		"/proc/last_in_file()\n",
		"// two lines that\n// move the proc down\n/proc/last_in_file()\n"
	);
	assert!(changes.plan.line_moves.iter().any(|m| m.proc.path == "/last_in_file"));
	let result = reload(&mut watcher, &changes);
	assert!(result.warnings.is_empty(), "{:?}", result.warnings);
	assert_eq!(
		lines_with_code(&mut client, "hot_reload_test_host.dme", 1, 10_000),
		dme_lines,
		"the .dme's place moved with resources.dm's lines"
	);
	let mut other = world.connect();
	let new_offset = offset_of(&mut other, "/last_in_file", line + 2);
	set(&mut other, new_offset);
	match client.request(Request::Spawn {
		expression: "last_in_file()".to_owned(),
		object: None
	}) {
		Response::Spawned(Ok(_)) => {}
		other => panic!("expected last_in_file() to be spawned, got {:?}", other)
	}
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(other.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_line(&mut client), ("/last_in_file".to_owned(), Some(line + 2)));
	client.resume();
	wait_for(&mut client, "last_calls", "1");
	match client.request(Request::BreakpointUnset {
		instruction: last_in_file(old_offset),
		exact: false
	}) {
		Response::BreakpointUnset { success: true, .. } => {}
		other => panic!("couldn't take away the moved breakpoint: {:?}", other)
	}
	other.leave();
	println!("ok: a moved breakpoint is where one set anew is, past the .dme's place");

	let code = match client.request(Request::ProcInfo { proc: last_in_file(0).proc }) {
		Response::ProcInfo(Ok(info)) => info.code,
		other => panic!("expected ProcInfo for /last_in_file, got {:?}", other)
	};
	let Response::Disassemble(Ok(disassembly)) = client.request(Request::Disassemble { code }) else {
		panic!("expected the disassembly of /last_in_file");
	};
	let instructions = &disassembly.instructions;
	let dme_at = instructions
		.iter()
		.position(|at| at.text.starts_with("DbgFile") && at.text.contains(".dme"))
		.unwrap_or_else(|| panic!("/last_in_file has no .dme record: {:?}", instructions));
	let after_dme_line = instructions[dme_at + 2].offset as u32;
	match client.request(Request::BreakpointSet {
		instruction: last_in_file(after_dme_line),
		condition: None,
		hit: None,
		log: None,
		exact: true
	}) {
		Response::BreakpointSet {
			result: BreakpointSetResult::Success { .. }
		} => {}
		other => panic!("couldn't set a breakpoint after the .dme's line: {:?}", other)
	}
	let changes = source.edit("resources.dm", "// two lines that\n// move the proc down\n", "");
	assert!(changes.plan.line_moves.iter().any(|m| m.proc.path == "/last_in_file"));
	let result = reload(&mut watcher, &changes);
	assert!(result.warnings.is_empty(), "{:?}", result.warnings);
	assert_eq!(lines_with_code(&mut client, "hot_reload_test_host.dme", 1, 10_000), dme_lines);
	match client.request(Request::BreakpointUnset {
		instruction: last_in_file(after_dme_line),
		exact: true
	}) {
		Response::BreakpointUnset { success: true, .. } => {}
		other => panic!("the breakpoint after the .dme's line went with the reload: {:?}", other)
	}
	println!("ok: a reload moving an included file's lines leaves the .dme's lines");

	let changes = source.edit(
		"reload.dm",
		"/proc/runtime_target()\n\treturn 1",
		"/proc/runtime_target()\n\tvar/list/nothing\n\treturn nothing.len"
	);
	reload(&mut watcher, &changes);
	eval(&mut client, "arm_runtime()");
	match client.expect_pause() {
		BreakpointReason::Runtime { message, .. } => assert!(message.contains("null.len"), "unexpected runtime: {}", message),
		other => panic!("expected a runtime, got {:?}", other)
	}
	let line = source.line_of("reload.dm", "\treturn nothing.len");
	assert_eq!(paused_line(&mut client), ("/runtime_target".to_owned(), Some(line)));
	client.resume();
	println!("ok: runtime error in reloaded code");

	let changes = source.edit(
		"reload.dm",
		"\tvar/list/nothing\n\treturn nothing.len",
		"\tthrow EXCEPTION(new /datum/animal/cat)"
	);
	reload(&mut watcher, &changes);
	eval(&mut client, "arm_runtime()");
	match client.expect_pause() {
		BreakpointReason::Runtime { message, .. } => assert_eq!(message, "/datum/animal/cat"),
		other => panic!("expected a runtime, got {:?}", other)
	}
	client.resume();
	println!("ok: exception named with an object");
	std::fs::write(source.dir.join("Case_Probe.txt"), "found").unwrap();
	let changes = source.edit(
		"reload.dm",
		"/proc/runtime_target()\n\tthrow EXCEPTION(new /datum/animal/cat)",
		"/proc/runtime_target()\n\treturn file2text('case_probe.txt')"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "runtime_target()"), "\"found\"");
	println!("ok: resource named in another case");
	let changes = source.edit(
		"reload.dm",
		"/proc/runtime_target()\n",
		"/datum/cache_probe/proc/probe()\n\treturn 1\n\n/datum/cache_probe/sub\n\tvar/extra = 5\n\n/proc/runtime_target()\n"
	);
	reload(&mut watcher, &changes);
	let line = source.line_of("reload.dm", "/datum/cache_probe/proc/probe()") + 1;
	let instruction = InstructionRef {
		proc: ProcRef {
			path: "/datum/cache_probe/probe".to_owned(),
			override_id: 0
		},
		offset: offset_of(&mut client, "/datum/cache_probe/probe", line)
	};
	let response = client.request(Request::BreakpointSet {
		instruction,
		condition: None,
		hit: None,
		log: Some(vec![
			LogPart::Text("extra ".to_owned()),
			LogPart::Expr("extra".to_owned()),
			LogPart::Text(", global ".to_owned()),
			LogPart::Expr("cache_global".to_owned()),
		]),
		exact: false
	});
	assert!(
		matches!(response, Response::BreakpointSet {
			result: BreakpointSetResult::Success { .. }
		}),
		"{:?}",
		response
	);
	let mut logged = |client: &mut Client, object: &str| {
		eval(client, &format!("spawn(0) (new {}).probe(); 1", object));
		match client.recv() {
			Response::Log { message, .. } => message,
			other => panic!("expected a log message, got {:?}", other)
		}
	};
	let sub = logged(&mut client, "/datum/cache_probe/sub");
	assert!(sub.starts_with("extra 5, global {cache_global: "), "{}", sub);
	let base = logged(&mut client, "/datum/cache_probe");
	assert!(base.starts_with("extra {extra: "), "{}", base);
	let changes = source.edit(
		"reload.dm",
		"/datum/cache_probe/sub\n",
		"var/cache_global = 7\n\n/datum/cache_probe/sub\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(logged(&mut client, "/datum/cache_probe/sub"), "extra 5, global 7");
	println!("ok: logpoint expressions compiled once, for each type of src and reload");

	let changes = source.edit(
		"main.dm",
		"/proc/auxtools_launched_world_del()\n\tCRASH(\"auxtools not loaded\")",
		"/proc/auxtools_launched_world_del()\n\tCRASH(\"auxtools still not loaded\")"
	);
	let result = reload(&mut watcher, &changes);
	assert!(
		result.warnings.iter().any(|w| w.contains("hooked")),
		"no warning about the hook: {:?}",
		result.warnings
	);
	println!("ok: hooked-proc warning");

	let changes = source.edit("reload.dm", "\treturn 3", "\treturn 4");
	client.send(reload_request(&changes));
	client.send(Request::Eval {
		frame_id: None,
		command: "answer()".to_owned(),
		ending: false
	});
	assert!(expect_reload_result(client.recv()).applied);
	match client.recv() {
		Response::Eval(Ok(eval)) => assert_eq!(eval.value, "4"),
		other => panic!("expected the result of answer(), got {:?}", other)
	}
	println!("ok: ordered reload and evaluation");

	let mut request = reload_request(&source.edit("reload.dm", "\treturn 4", "\treturn 5"));
	if let Request::HotReload(HotReloadRequest { plan, .. }) = &mut request {
		let mut bad = plan.reloads[0].clone();
		bad.proc.path = "/a_proc_the_world_doesnt_have".to_owned();
		plan.reloads.push(bad);
	}
	let result = expect_reload_result(client.request(request));
	assert!(!result.applied);
	assert!(
		result.errors.iter().any(|e| e.contains("a_proc_the_world_doesnt_have")),
		"{:?}",
		result.errors
	);
	assert_eq!(eval(&mut client, "answer()"), "4");
	println!("ok: rejected reload preserves existing code");

	let request = Request::HotReload(HotReloadRequest {
		plan: dmasm::reload::Plan {
			removed_procs: vec![dmasm::reload::RemovedProc {
				proc: dmasm::reload::ProcKey {
					path: "/datum/declared/never_built".to_owned(),
					override_id: 0
				},
				type_path: "/datum/declared".to_owned(),
				name: "never_built".to_owned(),
				verb: false
			}],
			removed_vars: [
				("/datum/declared", "count"),
				("/datum/declared", "never_built"),
				("/datum/never_built", "count")
			]
			.map(|(type_path, var)| dmasm::reload::RemovedVar {
				type_path: type_path.to_owned(),
				var: var.to_owned()
			})
			.to_vec(),
			..Default::default()
		},
		..Default::default()
	});
	let result = expect_reload_result(client.request(request));
	assert!(result.applied, "{:?}", result.errors);
	assert_eq!(result.skipped, ["/datum/declared/never_built"]);
	assert_eq!(result.warnings, [
		"/datum/declared: var count was removed from source but retained in the running world",
		"/datum/declared: var never_built was removed from source and isn't in the running world",
		"/datum/never_built: var count was removed from source and isn't in the running world",
		"/datum/declared/never_built: not found in the running world; skipping removal"
	]);
	println!("ok: removals the world never had");

	assert_eq!(eval(&mut client, "#fail-reload append"), "the next reload fails at Append");
	let changes = source.edit(
		"reload.dm",
		"\treturn 5",
		"\treturn unadded()\n\nvar/unadded_var = 6\n\n/proc/unadded()\n\treturn unadded_var"
	);
	assert_eq!(
		(changes.plan.new_vars.len(), changes.plan.new_procs.len(), changes.plan.reloads.len()),
		(1, 1, 1),
		"{:?}",
		changes.plan
	);
	let result = expect_reload_result(client.request(reload_request(&changes)));
	assert!(!result.applied, "a reload whose proc wasn't added was applied: {:?}", result);
	assert_eq!(result.errors, [
		"couldn't add a proc: asked for with #fail-reload",
		"new entries remain allocated but unused: 1 var"
	]);
	assert_eq!(eval(&mut client, "answer()"), "4");
	assert_eq!(eval(&mut client, "(\"unadded_var\" in global.vars)"), "0");
	reload(&mut client, &changes);
	assert_eq!(eval(&mut client, "answer()"), "6");
	assert_eq!(eval(&mut client, "global.vars[\"unadded_var\"]"), "6");
	println!("ok: a reload stops before changing anything when an addition fails");

	assert_eq!(eval(&mut client, "#fail-reload change"), "the next reload fails at Change");
	let changes = source.edit_many("reload.dm", &[
		("\treturn unadded()", "\treturn 7"),
		("/obj/crate\n\tname = \"crate\"\n", "/obj/crate\n\tname = \"crate\"\n\tdensity = 1\n")
	]);
	assert_eq!(changes.plan.builtin_defaults.len(), 1, "{:?}", changes.plan.builtin_defaults);
	let result = reload(&mut client, &changes);
	assert_eq!(result.failures, ["couldn't set a built-in var: asked for with #fail-reload"]);
	assert_eq!(result.warnings, [
		"verb and built-in changes apply only to new instances; existing instances retain their values"
	]);
	assert_eq!(eval(&mut client, "answer()"), "7");
	assert_eq!(eval(&mut client, "(new /obj/crate).density"), "0");
	println!("ok: a step that fails after others changed the world is a failure, not a note");
}

fn definitions(built: Built) {
	let (mut source, _world, mut client, mut watcher) = start(built);

	let changes = source.edit(
		"reload.dm",
		"/proc/uses_helper()\n\treturn 0",
		"/proc/uses_helper()\n\treturn helper() + 1\n\n/proc/helper()\n\treturn 7"
	);
	assert_eq!(changes.plan.new_procs.len(), 1);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "uses_helper()"), "8");
	assert_eq!(eval(&mut client, "helper()"), "7");
	let response = client.request(Request::Offset {
		proc: ProcRef {
			path: "/helper".to_owned(),
			override_id: 0
		},
		line: source.line_of("reload.dm", "\treturn 7")
	});
	assert!(matches!(response, Response::Offset { offset: Some(_) }), "{:?}", response);
	println!("ok: new global proc");

	let changes = source.edit(
		"reload.dm",
		"/datum/animal/proc/speak()\n\treturn sound",
		"/datum/animal/proc/speak()\n\treturn sound\n\n/datum/animal/proc/describe()\n\treturn \"[sound] on [legs] legs\""
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.describe()"), "\"woof on 4 legs\"");
	println!("ok: new type proc");

	assert_eq!(eval(&mut client, "old_puppy.speak()"), "\"woof?\"");
	let changes = source.edit(
		"reload.dm",
		"/datum/animal/dog\n\tsound = \"woof\"\n",
		"/datum/animal/dog\n\tsound = \"woof\"\n\n/datum/animal/dog/speak()\n\treturn \"[..()]!\"\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.speak()"), "\"woof!\"");
	assert_eq!(eval(&mut client, "old_puppy.speak()"), "\"woof!?\"");
	println!("ok: new override and subtype parent calls");

	eval(&mut client, "write_legs()");
	let changes = source.edit("reload.dm", "\tvar/legs = 4", "\tvar/legs = 3");
	assert_eq!(changes.plan.var_defaults.len(), 1);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.legs"), "3");
	assert_eq!(eval(&mut client, "new_dog_legs()"), "3");
	assert_eq!(eval(&mut client, "legs_written.legs"), "9");
	assert_eq!(eval(&mut client, "new_bird_legs()"), "2");
	assert_eq!(eval(&mut client, "initial(old_dog.legs)"), "3");
	println!("ok: variable default update");

	let changes = source.edit(
		"reload.dm",
		"\tvar/legs = 3\n",
		"\tvar/legs = 3\n\tvar/tail = 1\n\tvar/static/herd = 0\n\tvar/tmp/mood = \"calm\"\n\n/datum/animal/bird\n\ttail = 0\n\nvar/flock_size = \
		 12\n\n/datum/animal/proc/join_herd()\n\therd++\n\treturn \"[herd] [mood] [tail] \
		 [flock_size]\"\n\n/proc/new_bird_tail()\n\tvar/datum/animal/bird/b = new\n\treturn b.tail\n"
	);
	assert_eq!(changes.plan.new_vars.len(), 4, "{:?}", changes.plan.new_vars);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.tail"), "1");
	assert_eq!(eval(&mut client, "initial(old_dog.tail)"), "1");
	assert_eq!(eval(&mut client, "new_bird_tail()"), "0");
	assert_eq!(eval(&mut client, "old_dog.join_herd()"), "\"1 calm 1 12\"");
	assert_eq!(eval(&mut client, "old_puppy.join_herd()"), "\"2 calm 1 12\"");
	assert_eq!(eval(&mut client, "old_dog.herd"), "2");
	assert_eq!(eval(&mut client, "flock_size"), "12");
	assert_eq!(eval(&mut client, "(\"tail\" in old_dog.vars)"), "1");
	println!("ok: new variables");

	assert_eq!(eval(&mut client, "(\"flock_size\" in global.vars)"), "1");
	assert_eq!(eval(&mut client, "global.vars[\"flock_size\"]"), "12");
	assert_eq!(eval(&mut client, "global.vars[\"flock_size\"] = 13"), "13");
	assert_eq!(eval(&mut client, "flock_size"), "13");
	assert_eq!(eval(&mut client, "(\"herd\" in global.vars)"), "0");
	println!("ok: new global in global.vars");

	let changes = source.edit("reload.dm", "\tvar/tail = 1\n", "\tvar/tail = 2\n");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.tail"), "2");
	assert_eq!(eval(&mut client, "new_bird_tail()"), "0");
	println!("ok: new variable default");

	let changes = source.edit(
		"reload.dm",
		"/proc/greet(n)\n\treturn \"hi [n]\"\n\n/proc/greet_named()\n\treturn \"unchanged\"",
		"/proc/greet(n, sound = \"!\")\n\treturn \"hi [n][sound]\"\n\n/proc/greet_named()\n\treturn greet(\"bo\", sound = \"?\")"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "greet(\"al\")"), "\"hi al!\"");
	assert_eq!(eval(&mut client, "greet_named()"), "\"hi bo?\"");
	println!("ok: new parameter");

	let changes = source.edit(
		"reload.dm",
		"/proc/greet(n, sound = \"!\")\n\treturn \"hi [n][sound]\"\n\n/proc/greet_named()\n\treturn greet(\"bo\", sound = \"?\")",
		"/proc/greet(n, sound = \"!\", unheard_of = \"\")\n\treturn \"hi [n][sound][unheard_of]\"\n\n/proc/greet_named()\n\treturn greet(\"bo\", \
		 unheard_of = \"~\")"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "greet_named()"), "\"hi bo!~\"");
	println!("ok: new parameter name");

	let before: u32 = eval(&mut client, "counter()").parse().unwrap();
	let changes = source.edit(
		"reload.dm",
		"\tvar/static/count = 0\n\tcount++\n\treturn count",
		"\tvar/static/count = 0\n\tvar/static/bonus = 100\n\tcount += 10\n\treturn count + bonus\n\n/proc/visits()\n\tvar/static/v = \
		 10\n\tv++\n\treturn v"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "counter()"), (before + 110).to_string());
	assert_eq!(eval(&mut client, "visits()"), "11");
	assert_eq!(eval(&mut client, "visits()"), "12");
	println!("ok: new static locals");

	const CAT: &str = "/datum/animal/cat\n\tsound = \"meow\"\n";
	const DUPE: &str = "/datum/animal\n\tvar/dupe = 1\n\n/datum/animal/cat\n\tsound = \"meow\"\n\tvar/dupe = 2\n";
	let refused = source.edit_refused("reload.dm", CAT, DUPE);
	assert!(
		refused
			.iter()
			.any(|e| e.contains("dupe: duplicate definition (/datum/animal declares it too)")),
		"{:?}",
		refused
	);
	source.edit("reload.dm", DUPE, CAT);
	println!("ok: duplicate var definition refused by type");

	let changes = source.edit("reload.dm", "\tvar/spare = 1\n", "");
	reload(&mut watcher, &changes);
	let changes = source.edit("reload.dm", "/datum/animal\n", "/datum/animal\n\tvar/spare = 3\n");
	let result = reload(&mut watcher, &changes);
	assert!(
		result
			.warnings
			.iter()
			.any(|w| w.contains("var spare reuses the declaration retained by the running world")),
		"{:?}",
		result.warnings
	);
	assert_eq!(eval(&mut client, "var/datum/animal/a = new /datum/animal\nreturn a.spare"), "3");
	println!("ok: a var removed and declared again");

	let changes = source.edit(
		"reload.dm",
		"/proc/caller_sees()\n\tvar/x = slow()\n\treturn isnull(x)\n",
		"/proc/caller_sees()\n\tvar/x = slow()\n\treturn isnull(x)\n\n/datum/animal/dog/speak()\n\treturn \
		 \"[..()]!!\"\n\n/datum/animal/cat/speak()\n\treturn \"[..()]~\"\n\n/datum/animal/cat/speak()\n\treturn \
		 uppertext(..())\n\n/proc/cat_says()\n\tvar/datum/animal/cat/c = new\n\treturn c.speak()\n"
	);
	assert_eq!(
		changes.plan.new_procs.len(),
		4,
		"{:?}",
		changes.plan.new_procs.iter().map(|p| p.code.proc.to_string()).collect::<Vec<_>>()
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.speak()"), "\"woof!!!\"");
	assert_eq!(eval(&mut client, "old_puppy.speak()"), "\"woof!!!?\"");
	assert_eq!(eval(&mut client, "cat_says()"), "\"MEOW~\"");
	println!("ok: additional proc definitions");

	let changes = source.edit("reload.dm", "/datum/animal/dog/speak()\n\treturn \"[..()]!!\"\n", "");
	assert_eq!(changes.plan.removed_procs.len(), 1, "{:?}", changes.plan.removed_procs);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.speak()"), "\"woof!\"");
	assert_eq!(eval(&mut client, "old_puppy.speak()"), "\"woof!?\"");
	let changes = source.edit(
		"reload.dm",
		"/proc/cat_says()\n",
		"/datum/animal/dog/speak()\n\treturn \"[..()]!!\"\n\n/proc/cat_says()\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "old_dog.speak()"), "\"woof!!!\"");
	assert_eq!(eval(&mut client, "old_puppy.speak()"), "\"woof!!!?\"");
	println!("ok: remove and restore definition");

	let without = "/proc/cat_says()\n";
	let with = "/proc/again()\n\treturn 7\n\n/proc/cat_says()\n";
	let mut removal = None;
	for round in 0..2 {
		let changes = source.edit("reload.dm", without, with);
		assert_eq!(changes.plan.new_procs.len(), 1, "round {}", round);
		let result = reload(&mut watcher, &changes);
		assert!(result.warnings.is_empty(), "round {}: {:?}", round, result.warnings);
		assert_eq!(eval(&mut client, "again()"), "7");
		let changes = source.edit("reload.dm", with, without);
		assert_eq!(changes.plan.removed_procs.len(), 1, "round {}", round);
		let result = reload(&mut watcher, &changes);
		assert!(result.warnings.is_empty(), "round {}: {:?}", round, result.warnings);
		removal = Some(changes);
	}
	let result = reload(&mut watcher, &removal.unwrap());
	assert_eq!(result.warnings.len(), 1, "{:?}", result.warnings);
	println!("ok: repeated proc addition and removal");

	let changes = source.edit(
		"reload.dm",
		"/proc/in_lists(mob/m in world, o in global.vars)\n\treturn 1",
		"/proc/in_lists(mob/m in world, o in global.vars, n = 5)\n\treturn n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "in_lists()"), "5");
	assert_eq!(eval(&mut client, "in_lists(n = 6)"), "6");
	println!("ok: parameter addition preserves input lists");

	let changes = source.edit("reload.dm", "\tvar/list/items = list(\"a\")", "\tvar/list/items = list(\"a\", \"b\")");
	assert_eq!(
		changes.plan.init_procs.len(),
		2,
		"{:?}",
		changes.plan.init_procs.iter().map(|i| &i.type_path).collect::<Vec<_>>()
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "new_items(/datum/bag)"), "\"a,b\"");
	assert_eq!(eval(&mut client, "new_items(/datum/bag/big)"), "\"a,b\"");
	assert_eq!(eval(&mut client, "new_items(/datum/bag/fixed)"), "\"z\"");
	assert_eq!(eval(&mut client, "jointext(old_bag.items, \",\")"), "\"a\"");
	println!("ok: computed default update");

	let changes = source.edit(
		"reload.dm",
		"\tvar/list/items = list(\"a\", \"b\")\n",
		"\tvar/list/items = list(\"a\", \"b\")\n\tvar/list/tags = list(1, 2)\n\n/datum/animal\n\tvar/list/toys = \
		 list(\"ball\")\n\n/proc/new_tags()\n\tvar/datum/bag/b = new\n\treturn length(b.tags)\n\n/proc/new_toys()\n\tvar/datum/animal/dog/d = \
		 new\n\treturn d.toys[1]\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "new_tags()"), "2");
	assert_eq!(eval(&mut client, "new_toys()"), "\"ball\"");
	assert_eq!(eval(&mut client, "isnull(old_bag.tags)"), "1");
	println!("ok: new computed variables");

	let changes = source.edit("reload.dm", "\tvar/list/items = list(\"a\", \"b\")\n", "\tvar/list/items = null\n");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "new_items(/datum/bag)"), "\"none\"");
	assert_eq!(eval(&mut client, "new_items(/datum/bag/fixed)"), "\"z\"");
	assert_eq!(eval(&mut client, "new_tags()"), "2");
	println!("ok: computed default becomes constant");
}

fn types(built: Built) {
	let (mut source, _world, mut client, mut watcher) = start(built);

	let changes = source.edit(
		"reload.dm",
		"/obj/crate/proc/describe_crate()\n",
		"/obj/crate/big_crate\n\tdesc = \"roomy\"\n\ticon_state = \"big\"\n\tdensity = 1\n\tcapacity = 8\n\tvar/list/stickers = \
		 list(\"fragile\")\n\n/obj/crate/big_crate/describe_crate()\n\treturn \"[..()] with [stickers[1]]\"\n\n/obj/crate/big_crate/huge\n\tname = \
		 \"huge crate\"\n\tcapacity = 16\n\n/datum/ledger\n\tvar/entries = 0\n\n/datum/ledger/proc/add()\n\tentries++\n\treturn \
		 entries\n\n/proc/ledger_test()\n\tvar/datum/ledger/l = new\n\tl.add()\n\treturn \"[l.add()] [istype(l, /datum/ledger)] [istype(l, /datum)] \
		 [l.type]\"\n\n/obj/crate/proc/describe_crate()\n"
	);
	assert_eq!(changes.plan.new_types.len(), 3, "{:?}", changes.plan.new_types);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "crate_report(/obj/crate)"), "\"crate|||0|4|crate holds 4|empty|c\"");
	assert_eq!(
		eval(&mut client, "crate_report(/obj/crate/big_crate)"),
		"\"crate|roomy|big|1|8|crate holds 8 with fragile|empty|c\""
	);
	assert_eq!(
		eval(&mut client, "crate_report(/obj/crate/big_crate/huge)"),
		"\"huge crate|roomy|big|1|16|huge crate holds 16 with fragile|empty|h\""
	);
	assert_eq!(eval(&mut client, "ledger_test()"), "\"2 1 1 /datum/ledger\"");
	assert_eq!(eval(&mut client, "text2path(\"/obj/crate/big_crate/huge\")"), "/obj/crate/big_crate/huge");
	assert_eq!(eval(&mut client, "length(typesof(/obj/crate))"), "3");
	assert_eq!(eval(&mut client, "istype(new /obj/crate/big_crate/huge, /obj/crate/big_crate)"), "1");
	assert_eq!(eval(&mut client, "istype(new /obj/crate, /obj/crate/big_crate)"), "0");
	assert_eq!(eval(&mut client, "length(typesof(/datum/animal))"), "5");
	assert_eq!(eval(&mut client, "istype(old_puppy, /datum/animal/dog)"), "1");
	println!("ok: new types");

	assert_eq!(eval(&mut client, "text2path(\"/datum/sought\")"), "null");
	assert_eq!(eval(&mut client, "text2path(\"/proc/sought_maker\")"), "null");
	let changes = source.edit(
		"reload.dm",
		"/proc/crate_report(type)\n",
		"/datum/sought\n\tvar/found = 1\n\n/proc/sought_maker()\n\tvar/datum/sought/s = new /datum/sought\n\treturn \
		 s.found\n\n/proc/crate_report(type)\n"
	);
	assert_eq!(changes.plan.new_types.len(), 1, "{:?}", changes.plan.new_types);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "sought_maker()"), "1");
	assert_eq!(eval(&mut client, "istype(new /datum/sought, /datum/sought)"), "1");
	let build: u32 = eval(&mut client, "world.byond_build").parse().unwrap();
	if build >= if cfg!(windows) { 1641 } else { 1644 } {
		assert_eq!(eval(&mut client, "text2path(\"/datum/sought\")"), "/datum/sought");
		assert_eq!(eval(&mut client, "text2path(\"/proc/sought_maker\")"), "/proc/sought_maker");
		assert_eq!(eval(&mut client, "call(text2path(\"/proc/sought_maker\"))()"), "1");
	}
	println!("ok: reload after cached path lookup failures");

	let changes = source.edit(
		"reload.dm",
		"/proc/crate_report(type)\n",
		"var/list/once_global = list(\"g\", \"h\")\n\n/obj/crate\n\tvar/static/list/once_static = list(1, 2, \
		 3)\n\n/proc/once_local()\n\tvar/static/datum/ledger/l = new\n\treturn l.add()\n\n/proc/once_report()\n\tvar/obj/crate/c = new\n\treturn \
		 \"[length(once_global)] [length(c.once_static)]\"\n\n/proc/crate_report(type)\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "once_report()"), "\"2 3\"");
	assert_eq!(eval(&mut client, "once_local()"), "1");
	assert_eq!(eval(&mut client, "once_local()"), "2");
	println!("ok: computed global and static initializers");

	assert!(sleeps(&mut client, "caller_sees()"));
	let changes = source.edit("reload.dm", "/proc/slow()\n\tsleep(1)", "/proc/slow()\n\tset waitfor = 0\n\tsleep(1)");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "caller_sees()"), "1");
	println!("ok: waitfor setting");

	assert_eq!(eval(&mut client, "nap_skipped(/datum/napper/cat)"), "1");
	let changes = source.edit(
		"reload.dm",
		"\tset waitfor = FALSE\n\tsleep(1)\n\treturn 1",
		"\tset waitfor = TRUE\n\tsleep(1)\n\treturn 1"
	);
	assert_eq!((changes.plan.reloads.len(), changes.plan.settings.len()), (1, 1));
	reload(&mut watcher, &changes);
	assert!(sleeps(&mut client, "nap_skipped(/datum/napper/cat)"));
	println!("ok: declaration settings inheritance");

	assert_eq!(eval(&mut client, "verb_report(old_helper)"), "\"Greet:Helpers\"");
	let changes = source.edit(
		"reload.dm",
		"/mob/helper/small\n",
		"/mob/helper/verb/wave()\n\tset name = \"Wave Hello\"\n\treturn \"wave\"\n\n/mob/helper/small\n\n/mob/helper/small/greet()\n\treturn \
		 \"[..()]!\"\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "verb_report(new /mob/helper/small)"),
		"\"Greet:Helpers,Wave Hello:,Greet:Helpers\""
	);
	assert_eq!(eval(&mut client, "verb_report(new /mob/helper)"), "\"Wave Hello:,Greet:Helpers\"");
	assert_eq!(eval(&mut client, "helper_greets(new /mob/helper/small)"), "\"hi!\"");
	assert_eq!(eval(&mut client, "verb_report(old_helper)"), "\"Greet:Helpers\"");
	assert_eq!(eval(&mut client, "helper_greets(old_helper)"), "\"hi\"");
	assert_eq!(eval(&mut client, "call(old_helper, \"wave\")()"), "\"wave\"");

	let changes = source.edit("reload.dm", "\tset category = \"Helpers\"\n", "\tset category = \"Friends\"\n");
	assert_eq!((changes.plan.reloads.len(), changes.plan.settings.len()), (1, 1));
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "verb_report(old_helper)"), "\"Greet:Friends\"");
	assert_eq!(
		eval(&mut client, "verb_report(new /mob/helper/small)"),
		"\"Greet:Friends,Wave Hello:,Greet:Friends\""
	);

	let changes = source.edit("reload.dm", "/mob/helper/small/greet()\n\treturn \"[..()]!\"\n", "");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "verb_report(new /mob/helper/small)"), "\"Wave Hello:,Greet:Friends\"");
	assert_eq!(eval(&mut client, "helper_greets(new /mob/helper/small)"), "\"hi\"");
	println!("ok: verb changes");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/mob/helper/tiny\n\tname = \"tiny\"\n\tsee_in_dark = 6\n\n/proc/mob_report(type)\n\tvar/mob/m = new type\n\treturn \"[m.type] [m.name] \
		 [m.sight] [m.see_in_dark] [istype(m, /mob/helper)] [ismob(m)] [verb_report(m)]\"\n\n/proc/first_def()\n"
	);
	assert_eq!(changes.plan.new_types.len(), 1, "{:?}", changes.refused);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "mob_report(/mob/helper/tiny)"),
		"\"/mob/helper/tiny tiny 0 6 1 1 Wave Hello:,Greet:Friends\""
	);
	assert_eq!(eval(&mut client, "helper_greets(new /mob/helper/tiny)"), "\"hi\"");
	assert_eq!(eval(&mut client, "text2path(\"/mob/helper/tiny\")"), "/mob/helper/tiny");
	assert_eq!(eval(&mut client, "length(typesof(/mob/helper))"), "3");
	let changes = source.edit("reload.dm", "/mob/helper/small\n", "/mob/helper/small\n\tsight = 4\n");
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "mob_report(/mob/helper/small)"),
		"\"/mob/helper/small small 4 2 1 1 Wave Hello:,Greet:Friends\""
	);
	println!("ok: new mob types");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n\treturn \"first\"\n",
		"/proc/first_def()\n\treturn \"first\"\n\n/first_def()\n\treturn \"second\"\n"
	);
	assert_eq!(changes.plan.new_procs.len(), 1);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "first_def()"), "\"first\"");
	println!("ok: additional global definition");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/proc/pick_from(n in list(7, 8, 9))\n\treturn n\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "pick_from(8)"), "8");
	println!("ok: new parameter input lists");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/proc/prefab_report()\n\tvar/obj/crate/a = new /obj/crate{capacity = 11; name = \"custom\"}()\n\tvar/obj/crate/b = new /obj/crate{capacity \
		 = 11; name = \"custom\"}()\n\tvar/obj/crate/c = new /obj/crate{capacity = 3}\n\treturn \"[a.name] [a.capacity] [b.capacity] [c.name] \
		 [c.capacity] [istype(c, /obj/crate)]\"\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "prefab_report()"), "\"custom 11 11 crate 3 1\"");
	println!("ok: new prefabs");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/obj/crate/fancy\n\talpha = 100\n\tpixel_x = 4\n\tmaptext = \"hi\"\n\tmaptext_width = 64\n\tgender = \"female\"\n\tappearance_flags = \
		 2049\n\tmouse_opacity = 0\n\tcolor = \"#ff0000\"\n\n/proc/builtin_report(type)\n\tvar/obj/o = new type\n\treturn \"[o.name] [o.alpha] \
		 [o.pixel_x] [o.pixel_y] [o.maptext] [o.maptext_width] [o.gender] [o.appearance_flags] [o.mouse_opacity] [o.color]\"\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "builtin_report(/obj/crate/fancy)"),
		"\"crate 100 4 0 hi 64 female 2049 0 #ff0000\""
	);
	let changes = source.edit(
		"reload.dm",
		"/obj/crate\n\tname = \"crate\"\n",
		"/obj/crate\n\tname = \"box\"\n\tpixel_y = 3\n"
	);
	assert_eq!(changes.plan.builtin_defaults.len(), 3, "{:?}", changes.plan.builtin_defaults);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "builtin_report(/obj/crate)"), "\"box 255 0 3  32 neuter 0 1 \"");
	assert_eq!(eval(&mut client, "(new /obj/crate).text"), "\"b\"");
	assert_eq!(
		eval(&mut client, "builtin_report(/obj/crate/big_crate)"),
		"\"box 255 0 3  32 neuter 0 1 \""
	);
	assert_eq!(
		eval(&mut client, "builtin_report(/obj/crate/fancy)"),
		"\"box 100 4 3 hi 64 female 2049 0 #ff0000\""
	);
	assert_eq!(
		eval(&mut client, "builtin_report(/obj/crate/big_crate/huge)"),
		"\"huge crate 255 0 3  32 neuter 0 1 \""
	);
	println!("ok: built-in variable defaults");

	let changes = source.edit(
		"reload.dm",
		"\tname = \"box\"\n\tpixel_y = 3\n",
		"\tname = \"box\"\n\tpixel_y = 3\n\tlayer = 7\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /obj/crate/big_crate).layer"), "7");
	let changes = source.edit("reload.dm", "\tpixel_y = 3\n\tlayer = 7\n", "");
	assert_eq!(changes.plan.builtin_inherited.len(), 2, "{:?}", changes.plan.builtin_inherited);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /obj/crate).layer"), "3");
	assert_eq!(eval(&mut client, "(new /obj/crate/big_crate).layer"), "3");
	assert_eq!(eval(&mut client, "(new /obj/crate/big_crate).pixel_y"), "0");
	assert_eq!(eval(&mut client, "(new /obj/crate/fancy).pixel_y"), "0");
	println!("ok: built-in override removal");

	let changes = source.edit("reload.dm", "/proc/first_def()\n", "/area\n\tlayer = 9\n\n/proc/first_def()\n");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /area).layer"), "9");
	let refused = source.edit_refused("reload.dm", "/area\n\tlayer = 9\n\n", "");
	assert!(
		refused
			.iter()
			.any(|why| why.starts_with("/area: layer: cannot restore this built-in type's original default")),
		"{:?}",
		refused
	);
	let changes = source.edit("reload.dm", "/proc/first_def()\n", "/area\n\tlayer = 9\n\n/proc/first_def()\n");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /area).layer"), "9");
	println!("ok: built-in type default reset rejection");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/exception/hot_reload\n\tname = \"one\"\n\n/sound/hot_reload\n\trepeat = 1\n\n/obj/crate/bounded\n\tbounds = \"1,1 to \
		 16,16\"\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	let changes = source.edit_many("reload.dm", &[
		("\tname = \"one\"\n", "\tname = \"two\"\n"),
		("\trepeat = 1\n", "\trepeat = 1\n\tchannel = 5\n\tfalloff = 2\n"),
		("\"1,1 to 16,16\"", "\"1,1 to 8,24\"")
	]);
	assert_eq!(changes.plan.var_defaults.len(), 3, "{:?}", changes.plan.var_defaults);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "initial(/exception/hot_reload::name)"), "\"two\"");
	assert_eq!(eval(&mut client, "initial(/sound/hot_reload::channel)"), "5");
	assert_eq!(eval(&mut client, "initial(/sound/hot_reload::falloff)"), "2");
	assert_eq!(eval(&mut client, "hr_named_falloff(new /sound/hot_reload)"), "\"2 7 9\"");
	assert_eq!(eval(&mut client, "(new /obj/crate/bounded).bound_width"), "8");
	assert_eq!(eval(&mut client, "(new /obj/crate/bounded).bound_height"), "24");
	println!("ok: var-table built-ins and bounds");
}

fn declarations(built: Built) {
	let (mut source, _world, mut client, mut watcher) = start(built);

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/datum/hr_a\n\tvar/x = 1\n\n/datum/hr_a/proc/f()\n\treturn \"a\"\n\n/datum/hr_b\n\tvar/x = 2\n\tvar/y = 3\n\n/datum/hr_b/New()\n\ty = \
		 4\n\n/datum/hr_b/proc/f()\n\treturn \"b\"\n\n/datum/hr_t\n\tparent_type = /datum/hr_a\n\n/datum/hr_t/f()\n\treturn \"t\" + \
		 ..()\n\n/datum/hr_t/sub\n\n/obj/hr_oa\n\ticon_state = \"a\"\n\tlayer = 7\n\n/obj/hr_ob\n\tpixel_x = 2\n\tbound_width = 16\n\tbound_height \
		 = 48\n\n/obj/hr_ot\n\tparent_type = /obj/hr_oa\n\tbound_width = 8\n\tstep_x = \
		 2\n\nvar/datum/hr_t/sub/hr_kept\n\n/proc/hr_keep()\n\thr_kept = new\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	eval(&mut client, "hr_keep()");
	assert_eq!(eval(&mut client, "hr_kept.f()"), "\"ta\"");
	let changes = source.edit_many("reload.dm", &[
		("\tparent_type = /datum/hr_a\n", "\tparent_type = /datum/hr_b\n"),
		("\tparent_type = /obj/hr_oa\n", "\tparent_type = /obj/hr_ob\n")
	]);
	assert_eq!(changes.plan.reparented.len(), 2, "{:?}", changes.plan.reparented);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /datum/hr_t/sub).f()"), "\"tb\"");
	assert_eq!(eval(&mut client, "istype(new /datum/hr_t/sub, /datum/hr_b)"), "1");
	assert_eq!(eval(&mut client, "istype(new /datum/hr_t/sub, /datum/hr_a)"), "0");
	assert_eq!(eval(&mut client, "length(typesof(/datum/hr_b))"), "3");
	assert_eq!(eval(&mut client, "length(typesof(/datum/hr_a))"), "1");
	assert_eq!(eval(&mut client, "(new /datum/hr_t).y"), "4");
	assert_eq!(eval(&mut client, "(new /datum/hr_t).x"), "2");
	assert_eq!(
		eval(
			&mut client,
			"\"[(new /obj/hr_ot).icon_state] [(new /obj/hr_ot).layer] [(new /obj/hr_ot).pixel_x]\""
		),
		"\" 3 2\""
	);
	assert_eq!(
		eval(
			&mut client,
			"\"[(new /obj/hr_ot).bound_width] [(new /obj/hr_ot).bound_height] [(new /obj/hr_ot).step_x]\""
		),
		"\"8 48 2\""
	);
	assert_eq!(eval(&mut client, "hr_kept.f()"), "\"tb\"");
	assert_eq!(
		eval(&mut client, "\"[istype(hr_kept, /datum/hr_b)] [hr_kept.x] [hr_kept.y]\""),
		"\"1 2 3\""
	);
	let changes = source.edit("reload.dm", "\tparent_type = /datum/hr_b\n", "\tparent_type = /datum/hr_a\n");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /datum/hr_t/sub).f()"), "\"ta\"");
	assert_eq!(eval(&mut client, "length(typesof(/datum/hr_a))"), "3");
	assert_eq!(eval(&mut client, "(new /datum/hr_t).x"), "1");
	println!("ok: parent type change");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/datum/mv_hud\n\n/datum/mv_hud/human\n\tvar/tmp/list/sue = list(\"human\")\n\n/datum/mv_hud/human/sub\n\n/datum/mv_hud/robot\n\tvar/sue = \
		 2\n\n/datum/mv_hud/plain\n\n/datum/mv_hud/human/proc/upd()\n\treturn \
		 \"human\"\n\nvar/datum/mv_hud/human/mv_kept\n\n/proc/mv_keep()\n\tmv_kept = new\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	eval(&mut client, "mv_keep()");
	let changes = source.edit_many("reload.dm", &[
		(
			"/datum/mv_hud\n\n",
			"/datum/mv_hud\n\tvar/sue = 5\n\n/datum/mv_hud/proc/upd()\n\treturn \"hud\"\n\n/proc/mv_error()\n\ttry\n\t\t(new \
			 /datum/mv_hud/human).upd(TRUE)\n\tcatch(var/exception/e)\n\t\treturn e.desc\n\n"
		),
		("\tvar/tmp/list/sue = list(\"human\")\n", ""),
		("\tvar/sue = 2\n", "\tsue = 3\n"),
		(
			"/datum/mv_hud/human/proc/upd()\n\treturn \"human\"\n",
			"/datum/mv_hud/human/upd(fail)\n\tif(fail)\n\t\tvar/list/nothing\n\t\treturn nothing.len\n\treturn \"human>\" + ..()\n"
		)
	]);
	assert!(changes.refused.is_empty(), "{:?}", changes.refused);
	let mut unmarked = reload_request(&changes);
	if let Request::HotReload(request) = &mut unmarked {
		for var in &mut request.plan.new_vars {
			var.moved_from.clear();
		}
	}
	let refused = expect_reload_result(watcher.request(unmarked));
	assert!(!refused.applied, "duplicate var was accepted: {:?}", refused);
	assert!(refused.errors.iter().any(|e| e.contains("duplicate definition")), "{:?}", refused.errors);
	let result = reload(&mut watcher, &changes);
	assert!(
		result
			.warnings
			.iter()
			.any(|w| w.contains("retain runtime declarations, values and var IDs")),
		"{:?}",
		result.warnings
	);
	let report = "\"[(new /datum/mv_hud).sue] [(new /datum/mv_hud/human).sue] [(new /datum/mv_hud/human/sub).sue] [(new /datum/mv_hud/robot).sue] \
	              [(new /datum/mv_hud/plain).sue]\"";
	assert_eq!(eval(&mut client, report), "\"5 5 5 3 5\"");
	assert_eq!(eval(&mut client, "issaved((new /datum/mv_hud/human).sue)"), "1");
	assert_eq!(eval(&mut client, "initial((new /datum/mv_hud/human/sub).sue)"), "5");
	assert_eq!(eval(&mut client, "(new /datum/mv_hud/human/sub).upd()"), "\"human>hud\"");
	assert_eq!(eval(&mut client, "(new /datum/mv_hud/plain).upd()"), "\"hud\"");
	assert_eq!(eval(&mut client, "hascall(new /datum/mv_hud/robot, \"upd\")"), "1");
	let error = eval(&mut client, "mv_error()");
	assert!(error.contains("proc name: upd (/datum/mv_hud/human/upd)"), "{}", error);
	assert_eq!(eval(&mut client, "json_encode(mv_kept.sue)"), "\"[\\\"human\\\"]\"");
	assert_eq!(eval(&mut client, "mv_kept.upd()"), "\"human>hud\"");
	println!("ok: declarations moved to an ancestor");

	let sound = "\"[(new /sound/hr_st).pan] [(new /sound/hr_st).frequency] [(new /sound/hr_st).priority]\"";
	assert_eq!(eval(&mut client, sound), "\"10 3 0\"");
	let changes = source.edit("main.dm", "\tparent_type = /sound/hr_sa\n", "\tparent_type = /sound/hr_sb\n");
	assert_eq!(changes.plan.reparented.len(), 1, "{:?}", changes.plan.reparented);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, sound), "\"0 3 5\"");
	println!("ok: sound parent change");

	assert_eq!(eval(&mut client, "kept_limits.max_doubled()"), "6");
	let changes = source.edit("main.dm", "\tvar/const/MAX = 3\n", "\tvar/const/MAX = 7\n");
	assert_eq!(changes.consts, ["MAX"]);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "kept_limits.max_doubled()"), "14");
	assert_eq!(eval(&mut client, "kept_limits.MAX"), "7");
	println!("ok: type const update");

	let changes = source.edit("main.dm", "\tvar/const/MAX = 7\n", "\tvar/const/MAX = 7\n\tvar/const/MIN = 2\n");
	assert!(changes.refused.is_empty(), "{:?}", changes.refused);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "kept_limits.MIN"), "2");
	println!("ok: new type const");

	assert_eq!(eval(&mut client, "json_encode(limit_all())"), "\"[10,11]\"");
	let changes = source.edit("main.dm", "var/const/LIMIT_ALL = 10\n", "var/const/LIMIT_ALL = 20\n");
	assert!(changes.warnings.is_empty(), "{:?}", changes.warnings);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "json_encode(limit_all())"), "\"[20,21]\"");
	println!("ok: global const update");

	let changes = source.edit("main.dm", "/datum/limits/var/list/things = null\n", "/datum/limits/var/things = null\n");
	assert_eq!(changes.retyped, ["things"]);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "kept_limits.thing_count()"), "0");
	println!("ok: variable type change");

	assert_eq!(eval(&mut client, "call(kept_limits, \"Label\")()"), "\"labelled\"");
	let changes = source.edit("main.dm", "\tset name = \"Label\"\n", "\tset name = \"Tag\"\n");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "call(kept_limits, \"Tag\")()"), "\"labelled\"");
	assert_eq!(eval(&mut client, "call(kept_limits, \"label\")()"), "\"labelled\"");
	println!("ok: proc rename");

	let changes = source.edit(
		"main.dm",
		"/datum/chain/say()\n\treturn ..() + \"3\"",
		"/datum/chain/say()\n\treturn ..() + \"2\"\n\n/datum/chain/say()\n\treturn ..() + \"3\""
	);
	assert_eq!(changes.plan.new_procs.len(), 1);
	let result = reload(&mut watcher, &changes);
	assert!(result.warnings.is_empty(), "{:?}", result.warnings);
	assert_eq!(eval(&mut client, "the_chain.say()"), "\"123\"");
	let definition_at = |client: &mut Client, override_id: u32, line: u32| {
		matches!(
			client.request(Request::Offset {
				proc: ProcRef {
					path: "/datum/chain/say".to_owned(),
					override_id
				},
				line
			}),
			Response::Offset { offset: Some(_) }
		)
	};
	assert!(definition_at(&mut client, 1, source.line_of("main.dm", "\treturn ..() + \"2\"")));
	assert!(definition_at(&mut client, 2, source.line_of("main.dm", "\treturn ..() + \"3\"")));
	let changes = source.edit("main.dm", "\treturn ..() + \"3\"", "\treturn ..() + \"4\"");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "the_chain.say()"), "\"124\"");
	let changes = source.edit("main.dm", "/datum/chain/say()\n\treturn ..() + \"2\"\n\n", "");
	assert_eq!(changes.plan.removed_procs.len(), 1);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "the_chain.say()"), "\"14\"");
	assert!(definition_at(&mut client, 1, source.line_of("main.dm", "\treturn ..() + \"4\"")));
	println!("ok: insert and remove type proc definitions");

	assert_eq!(eval(&mut client, "declared_one.read_count()"), "1");
	let changes = source.edit("main.dm", "\tvar/count = 1", "\tvar/static/count = 7");
	assert_eq!(changes.plan.redeclared.len(), 1);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "declared_one.read_count()"), "7");
	eval(&mut client, "declared_one.count = 8");
	assert_eq!(eval(&mut client, "(new /datum/declared/sub).read_count()"), "8");
	let changes = source.edit_many("main.dm", &[
		("\tvar/static/count = 7", "\tvar/count = 2"),
		("/datum/declared/sub\n", "/datum/declared/sub\n\tcount = 3\n")
	]);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /datum/declared).read_count()"), "2");
	assert_eq!(eval(&mut client, "(new /datum/declared/sub).read_count()"), "3");
	assert_eq!(eval(&mut client, "issaved(declared_one.count)"), "1");
	let changes = source.edit_many("main.dm", &[
		("\tvar/shared = 10", "\tvar/const/shared = 11"),
		("\tvar/count = 2", "\tvar/tmp/count = 2")
	]);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "declared_one.read_shared()"), "11");
	assert_eq!(eval(&mut client, "issaved(declared_one.count)"), "0");
	let changes = source.edit("main.dm", "\tvar/many = null", "\tvar/static/list/many = list(1, 2, 3)");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "declared_one.many_count()"), "3");
	println!("ok: static, const and tmp declaration transitions");

	let changes = source.edit(
		"main.dm",
		"/proc/compiled_late()\n\treturn 0",
		concat!(
			"/proc/compiled_late()\n",
			"\tvar/list/got = list(cmptextEx(\"a\", \"a\", \"a\"), cmptext(\"a\", \"A\", \"b\"),",
			" findlasttext_char(\"abcabc\", \"b\"), isnull(__TYPE__), keyed(lowertext(\"B\") = 3))\n",
			"\treturn jointext(got, \",\")"
		)
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "compiled_late()"), "\"1,0,5,1,3\"");
	println!("ok: additional compiler forms");

	let definition_at = |client: &mut Client, path: &str, override_id: u32, line: u32| {
		matches!(
			client.request(Request::Offset {
				proc: ProcRef {
					path: path.to_owned(),
					override_id
				},
				line
			}),
			Response::Offset { offset: Some(_) }
		)
	};
	assert_eq!(eval(&mut client, "verb_chain.speak()"), "\"ac\"");
	assert_eq!(eval(&mut client, "world.chant()"), "\"xz\"");
	let changes = source.edit_many("main.dm", &[
		(
			"/mob/chain_verbs/speak()\n\treturn ..() + \"c\"",
			"/mob/chain_verbs/speak()\n\treturn ..() + \"b\"\n\n/mob/chain_verbs/speak()\n\treturn ..() + \"c\""
		),
		(
			"/world/chant()\n\treturn ..() + \"z\"",
			"/world/chant()\n\treturn ..() + \"y\"\n\n/world/chant()\n\treturn ..() + \"z\""
		),
		(
			"/solo()\n\treturn \"another\"",
			"/solo()\n\treturn \"between\"\n\n/solo()\n\treturn \"another\""
		)
	]);
	assert_eq!(changes.plan.new_procs.len(), 3);
	let result = reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /mob/chain_verbs).speak()"), "\"abc\"");
	assert_eq!(eval(&mut client, "verb_names(new /mob/chain_verbs)"), "\"speak,speak,other,speak\"");
	assert_eq!(eval(&mut client, "world.chant()"), "\"xyz\"");
	assert_eq!(eval(&mut client, "solo()"), "\"declared\"");
	assert!(result.warnings.iter().all(|w| w.contains("existing instances")), "{:?}", result.warnings);
	for (path, needle) in [
		("/mob/chain_verbs/speak", "\treturn ..() + \"c\""),
		("/world/chant", "\treturn ..() + \"z\""),
		("/solo", "\treturn \"another\"")
	] {
		let line = source.line_of("main.dm", needle);
		assert!(definition_at(&mut client, path, 2, line), "{} isn't numbered 2", path);
	}
	let changes = source.edit_many("main.dm", &[
		("/mob/chain_verbs/speak()\n\treturn ..() + \"b\"\n\n", ""),
		("/world/chant()\n\treturn ..() + \"y\"\n\n", ""),
		("/solo()\n\treturn \"between\"\n\n", "")
	]);
	assert_eq!(changes.plan.removed_procs.len(), 3);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /mob/chain_verbs).speak()"), "\"ac\"");
	assert_eq!(eval(&mut client, "world.chant()"), "\"xz\"");
	assert_eq!(eval(&mut client, "solo()"), "\"declared\"");
	for (path, needle) in [
		("/mob/chain_verbs/speak", "\treturn ..() + \"c\""),
		("/world/chant", "\treturn ..() + \"z\""),
		("/solo", "\treturn \"another\"")
	] {
		let line = source.line_of("main.dm", needle);
		assert!(definition_at(&mut client, path, 1, line), "{} isn't numbered 1", path);
	}
	println!("ok: verb, world and global definition edits");
}

fn resources(built: Built) {
	let (mut source, world, mut client, mut watcher) = start(built);

	let built_lines = [
		lines_with_code(&mut client, "reload.dm", 1, 10_000),
		lines_with_code(&mut client, "main.dm", 1, 10_000)
	];
	let built_line = source.line_of("reload.dm", "\tx += 1");
	let built_place = located(&mut client, "reload.dm", built_line);
	assert_eq!(built_place.len(), 1, "{:?}", built_place);

	let changes = source.edit(
		"main.dm",
		"var/datum/limits/kept_limits = new\n",
		"var/datum/limits/kept_limits = new\n\n/world/Del()\n\tworld.log << \"world/Del says bye\"\n\t..()\n"
	);
	assert_eq!(changes.plan.new_procs.len(), 1);
	assert_eq!(changes.plan.new_procs[0].code.proc.override_id, 0);
	let result = reload(&mut watcher, &changes);
	assert!(result.warnings.is_empty(), "{:?}", result.warnings);
	offset_of(
		&mut client,
		"/world/Del",
		source.line_of("main.dm", "\tworld.log << \"world/Del says bye\"")
	);
	println!("ok: world/Del source override numbering");

	assert_eq!(eval(&mut client, "make_lamps()"), "\"made\"");
	assert_eq!(eval(&mut client, "lamp_states()"), "\"on,off\"");
	let built_lamp = resource_id(&eval(&mut client, "lamp_ref()"));
	let result = reload_resource(&mut watcher, &source, "lamp.dmi", "lamp_edited.dmi", vec![built_lamp]);
	let lamp = result.resources.first().unwrap_or_else(|| panic!("no resource in {:?}", result));
	assert_ne!(lamp.id, built_lamp, "the edited file kept its resource");
	assert!(lamp.appearances >= 3, "only {} appearances changed", lamp.appearances);
	assert_eq!(resource_id(&eval(&mut client, "lamp_ref()")), lamp.id);
	assert_eq!(eval(&mut client, "lamp_states()"), "\"on,off,new\"");
	assert_eq!(eval(&mut client, "states_of(placed_lamp.icon)"), "\"on,off,new\"");
	assert_eq!(eval(&mut client, "new_lamp_states()"), "\"on,off,new\"");
	assert_eq!(eval(&mut client, "overlay_states()"), "\"on,off,new\"");
	assert_eq!(resource_id(&eval(&mut client, "\"\\ref[placed_lamp.mouse_over_pointer]\"")), lamp.id);
	println!("ok: resource reload of an icon");

	let built_beep = resource_id(&eval(&mut client, "beep_ref()"));
	let result = reload_resource(&mut watcher, &source, "beep.wav", "beep_edited.wav", vec![built_beep]);
	let beep = result.resources.first().unwrap_or_else(|| panic!("no resource in {:?}", result));
	assert_ne!(beep.id, built_beep, "the edited file kept its resource");
	assert_eq!(resource_id(&eval(&mut client, "beep_ref()")), beep.id);
	assert_eq!(resource_id(&eval(&mut client, "default_sound_ref()")), beep.id);
	println!("ok: resource reload of a sound");

	assert_eq!(eval(&mut client, "make_crate()"), "\"made\"");
	let built_crate = resource_id(&eval(&mut client, "crate_ref()"));
	let held = "\"\\ref[held_lamp.icon]\"";
	assert_eq!(resource_id(&eval(&mut client, held)), built_lamp);
	let result = reload_resources(&mut watcher, &source, &[
		("lamp.dmi", "lamp_third.dmi", vec![built_lamp, lamp.id]),
		("crate.dmi", "crate_edited.dmi", vec![built_crate])
	]);
	assert_eq!(result.resources.len(), 2, "{:?}", result);
	assert!(result.resources.iter().all(|r| r.appearances >= 1), "{:?}", result);
	let crate_id = result.resources[1].id;
	assert_eq!(eval(&mut client, "states_of(placed_lamp.icon)"), "\"on,off,third\"");
	assert_eq!(eval(&mut client, "overlay_states()"), "\"on,off,third\"");
	assert_eq!(eval(&mut client, "states_of(placed_crate.icon)"), "\"new1\"");
	assert_eq!(resource_id(&eval(&mut client, held)), built_lamp);
	println!("ok: resource reload of two files");

	let result = reload_resource(&mut watcher, &source, "crate.dmi", "lamp_built.dmi", vec![crate_id]);
	assert_eq!(result.resources[0].id, built_lamp, "{:?}", result);
	assert!(
		result.warnings.iter().any(|w| w.contains("crate.dmi shares existing resource")),
		"{:?}",
		result.warnings
	);
	assert_eq!(eval(&mut client, "states_of(placed_crate.icon)"), "\"on,off\"");
	reload_resource(&mut watcher, &source, "crate.dmi", "crate_edited.dmi", vec![]);
	assert_eq!(resource_id(&eval(&mut client, held)), built_lamp);
	assert_eq!(eval(&mut client, "states_of(placed_crate.icon)"), "\"on,off\"");
	println!("ok: resource reload with shared content");

	let third = resource_id(&eval(&mut client, "lamp_ref()"));
	let result = reload_resource(&mut watcher, &source, "lamp.dmi", "lamp_built.dmi", vec![third]);
	assert_eq!(result.resources[0].id, built_lamp, "{:?}", result);
	assert!(!result.warnings.iter().any(|w| w.contains("same bytes")), "{:?}", result.warnings);
	assert_eq!(eval(&mut client, "states_of(placed_lamp.icon)"), "\"on,off\"");
	let result = reload_resource(&mut watcher, &source, "lamp.dmi", "lamp_edited.dmi", vec![built_lamp]);
	assert!(result.warnings.iter().all(|w| !w.contains("same bytes")), "{:?}", result.warnings);
	assert_eq!(eval(&mut client, "states_of(placed_lamp.icon)"), "\"on,off,new\"");
	println!("ok: resource reload with restored built content");

	let changes = source.edit_many("reload.dm", &[
		("/proc/answer()\n\treturn 1", "/proc/answer()\n\treturn 2"),
		(
			"/proc/few_locals()\n\tvar/a = 1\n\treturn a",
			"/proc/few_locals()\n\tvar/a = 1\n\tvar/b = 2\n\treturn a + b"
		),
		("\tcount++", "\tcount += 10"),
		("\tvar/legs = 4", "\tvar/legs = 3")
	]);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "answer()"), "2");
	assert_eq!(eval(&mut client, "few_locals()"), "3");
	assert_eq!(eval(&mut client, "counter()"), "10");
	assert_eq!(eval(&mut client, "old_dog.legs"), "3");

	assert_ne!(lines_with_code(&mut client, "reload.dm", 1, 10_000), built_lines[0]);
	eval(&mut client, "request_reboot()");
	client.wait_closed();
	let mut client = world.connect();
	assert!(world.output().contains("world/Del says bye"), "world/Del override was not called");
	assert_eq!(eval(&mut client, "answer()"), "1");
	assert_eq!(eval(&mut client, "few_locals()"), "1");
	assert_eq!(eval(&mut client, "counter()"), "1");
	assert_eq!(eval(&mut client, "old_dog.legs"), "4");
	assert_eq!(lines_with_code(&mut client, "reload.dm", 1, 10_000), built_lines[0]);
	assert_eq!(lines_with_code(&mut client, "main.dm", 1, 10_000), built_lines[1]);
	assert_eq!(located(&mut client, "reload.dm", built_line), built_place);
	println!("ok: reboot after reload");

	assert_eq!(eval(&mut client, "make_lamps()"), "\"made\"");
	assert_eq!(eval(&mut client, "lamp_states()"), "\"on,off\"");
	assert_eq!(resource_id(&eval(&mut client, "lamp_ref()")), built_lamp);
	let mut watcher = world.connect_watcher();
	let result = reload_resource(&mut watcher, &source, "lamp.dmi", "lamp_edited.dmi", vec![built_lamp]);
	assert!(result.resources.first().is_some_and(|lamp| lamp.appearances >= 3), "{:?}", result);
	assert_eq!(eval(&mut client, "lamp_states()"), "\"on,off,new\"");
	assert_eq!(eval(&mut client, "states_of(placed_lamp.icon)"), "\"on,off,new\"");
	assert_eq!(eval(&mut client, "overlay_states()"), "\"on,off,new\"");
	println!("ok: resource reload after a reboot");

	let lamp = result.resources[0].id;
	assert_eq!(eval(&mut client, "make_many_appearances()"), "\"made\"");
	let result = reload_resource(&mut watcher, &source, "lamp.dmi", "lamp_third.dmi", vec![lamp]);
	assert!(result.resources.first().is_some_and(|lamp| lamp.appearances >= 3), "{:?}", result);
	assert_eq!(eval(&mut client, "states_of(placed_lamp.icon)"), "\"on,off,third\"");
	eval(&mut client, "many_appearances = null");
	watcher.send(Request::Disconnect);
	watcher.wait_closed();
	println!("ok: resource reload past 65,535 appearances");

	client.send(Request::Disconnect);
	client.wait_closed();
	let mut watcher = world.connect_watcher();
	source.restore();
	let changes = source.edit("reload.dm", "/proc/tick()\n", "/proc/tick()\n\tworld.log << \"tick says [answer()]\"\n");
	reload(&mut watcher, &changes);
	let start = Instant::now();
	while !world.output().contains("tick says 1") {
		assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for reloaded tick()");
		std::thread::sleep(Duration::from_millis(100));
	}
	println!("ok: reload without a debugger");
}
