use std::{
	collections::HashSet,
	path::{Path, PathBuf},
	sync::atomic::{AtomicBool, Ordering},
	time::{Duration, Instant}
};

use auxtools_protocol::*;
use dmasm::reload::{diff, Changes, ParseOptions, Snapshot};
use test_support::{
	debug::{breakpoint_id, Client, World},
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

fn paused_offset(client: &mut Client) -> (String, u32) {
	match client.request(Request::StackFrames {
		stack_id: 0,
		start_frame: Some(0),
		count: Some(1)
	}) {
		Response::StackFrames { frames, .. } => {
			let frame = frames.into_iter().next().expect("paused with no frames");
			(frame.instruction.proc.path, frame.instruction.offset)
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

	eval(&mut client, "sleeper_release = FALSE");
	eval(&mut client, "start_sleeper()");
	wait_for(&mut client, "sleeper_state", "\"sleeping\"");
	let changes = source.edit("reload.dm", "\"new code finished\"", "\"moved code finished\"");
	reload(&mut watcher, &changes);
	const SLEEPER: &str =
		"/proc/sleeper()\n\tsleeper_state = \"sleeping\"\n\twhile (!sleeper_release)\n\t\tsleep(1)\n\tsleeper_state = \"moved code finished\"\n";
	let changes = source.move_code(SLEEPER, "reload.dm", "main.dm");
	reload(&mut watcher, &changes);
	let response = client.request(Request::Pause);
	assert!(matches!(response, Response::Ack), "expected Ack for Pause, got {:?}", response);
	assert_eq!(client.expect_pause(), BreakpointReason::Pause);
	let Response::Stacks { stacks, .. } = client.request(Request::Stacks { sleeping: Some(1000) }) else {
		panic!("expected Stacks");
	};
	let mut sleeping = vec![];
	for stack in &stacks {
		let Response::StackFrames { frames, .. } = client.request(Request::StackFrames {
			stack_id: stack.id,
			start_frame: None,
			count: None
		}) else {
			panic!("expected StackFrames");
		};
		sleeping.extend(frames.into_iter().filter(|frame| frame.instruction.proc.path == "/sleeper"));
	}
	let [frame] = &sleeping[..] else {
		panic!("expected one frame of /sleeper, got {:?}", sleeping);
	};
	assert!(
		frame.old_code && frame.file.as_deref() == Some("reload.dm"),
		"expected /sleeper's frame in old code in reload.dm, got {:?}",
		frame
	);
	let Response::Disassemble(Ok(disassembly)) = client.request(Request::Disassemble { code: frame.code }) else {
		panic!("expected the disassembly of /sleeper's old code");
	};
	assert_eq!(disassembly.file.as_deref(), Some("reload.dm"));
	client.resume();
	eval(&mut client, "release_sleeper()");
	wait_for(&mut client, "sleeper_state", "\"new code finished\"");
	println!("ok: old code of a proc moved to another file is disassembled with its own file");
}

fn debugger(built: Built) {
	let (mut source, world, mut client, mut watcher) = start(built);

	let line = source.line_of("reload.dm", "\tx += 1");
	let old_offset = offset_of(&mut client, "/breakpoint_target", line);
	let old_at = InstructionRef {
		proc: ProcRef {
			path: "/breakpoint_target".to_owned(),
			override_id: 0
		},
		offset: old_offset
	};
	let response = client.request(Request::BreakpointSet {
		id: breakpoint_id(&old_at, false),
		instruction: old_at.clone(),
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
		id: breakpoint_id(&old_at, false)
	});
	assert!(matches!(response, Response::BreakpointUnset { success: true, .. }), "{:?}", response);
	eval(&mut client, "arm_breakpoint()");
	wait_for(&mut client, "breakpoint_armed", "0");
	println!("ok: remove a remapped breakpoint by its id");

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
		id: breakpoint_id(&at, false),
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
		id: breakpoint_id(&at, false)
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
		id: breakpoint_id(&at, false),
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
		id: breakpoint_id(&at, false)
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
			id: breakpoint_id(at, false),
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
			id: breakpoint_id(at, false)
		});
		assert!(matches!(response, Response::BreakpointUnset { success: true, .. }), "{:?}", response);
	};
	let line = source.line_of("reload.dm", "\tx += 1");
	assert_eq!(lines_with_code(&mut client, "reload.dm", line - 1, line + 3), [line - 1, line, line + 1]);
	let before = located(&mut client, "reload.dm", line);
	assert_eq!(before.len(), 1, "{:?}", before);
	stops_on(&mut client, &before[0], line);
	let response = client.request(Request::BreakpointSet {
		id: breakpoint_id(&before[0], false),
		instruction: before[0].clone(),
		condition: None,
		hit: None,
		log: None,
		exact: false
	});
	assert!(matches!(response, Response::BreakpointSet { .. }), "{:?}", response);
	let changes = source.edit("reload.dm", "\tvar/x = 1\n\tx += 1", "\tvar/x = 1\n\tx *= 3\n\tx += 1");
	reload(&mut watcher, &changes);
	eval(&mut client, "arm_breakpoint()");
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_line(&mut client), ("/breakpoint_target".to_owned(), Some(line + 1)));
	client.resume();
	let response = client.request(Request::BreakpointUnset {
		id: breakpoint_id(&before[0], false)
	});
	assert!(matches!(response, Response::BreakpointUnset { success: true, .. }), "{:?}", response);
	println!("ok: a breakpoint stays on its statement when a statement is inserted above it");
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
				id: breakpoint_id(&instruction, false),
				instruction,
				condition: None,
				hit: None,
				log: None,
				exact: false
			}
		} else {
			Request::BreakpointUnset {
				id: breakpoint_id(&instruction, false)
			}
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
			id: breakpoint_id(&last_in_file(offset), false),
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
		id: breakpoint_id(&last_in_file(old_offset), false)
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
		id: breakpoint_id(&last_in_file(after_dme_line), true),
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
		id: breakpoint_id(&last_in_file(after_dme_line), true)
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
		id: breakpoint_id(&instruction, false),
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
		"/datum/declared: var count was removed from source, but the running world still has it",
		"/datum/declared: var never_built was removed from source and isn't in the running world",
		"/datum/never_built: var count was removed from source and isn't in the running world",
		"/datum/declared/never_built: not found in the running world, so there is nothing to remove"
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
		"verb and built-in changes apply only to new instances. Existing instances keep their values"
	]);
	assert_eq!(eval(&mut client, "answer()"), "7");
	assert_eq!(eval(&mut client, "(new /obj/crate).density"), "0");
	println!("ok: a step that fails after others changed the world is a failure, not a note");

	twin_breakpoints(&mut client, &source, 1);
	println!("ok: procs that DreamMaker gave the same code have separate breakpoints");
	let changes = source.edit(
		"reload.dm",
		"twin_runs += 1; } /proc/second() { twin_waiting += 1; while (twin_hold) sleep(1); twin_runs += 1; }",
		"twin_runs += 2; } /proc/second() { twin_waiting += 1; while (twin_hold) sleep(1); twin_runs += 2; }"
	);
	assert_eq!(changes.plan.reloads.len(), 2, "{:?}", changes.plan);
	let line = source.line_of("reload.dm", "TWINS(twin_a, twin_b)");
	let offset = offset_of(&mut client, "/twin_a", line);
	twin_breakpoint(&mut client, "twin_a", offset, true);
	reload(&mut watcher, &changes);
	twin_run(&mut client, "twin_b", line, 2, false);
	twin_run(&mut client, "twin_a", line, 2, true);
	let offset = offset_of(&mut client, "/twin_a", line);
	twin_breakpoint(&mut client, "twin_a", offset, false);
	println!("ok: a breakpoint kept across a reload that gives procs the same code stops only its own proc");
	twin_breakpoints(&mut client, &source, 2);
	println!("ok: procs that a reload gave the same code have separate breakpoints");

	let sleeping = sleep_named_twins(&mut client);
	let changes = source.edit_many("reload.dm", &[
		("twin_runs += first_local; }", "twin_runs += first_local * 2; }"),
		("twin_runs += second_local; }", "twin_runs += second_local * 2; }")
	]);
	assert_eq!(changes.plan.reloads.len(), 2, "{:?}", changes.plan);
	reload(&mut watcher, &changes);
	let mut names = named_twin_locals(&mut client);
	wake_named_twins(&mut client, sleeping, 2);
	let sleeping = sleep_named_twins(&mut client);
	names.extend(named_twin_locals(&mut client));
	wake_named_twins(&mut client, sleeping, 4);
	let expected = ["c_local", "d_local", "c_local", "d_local"];
	assert_eq!(names, expected, "the locals' names of old and new frames of twin_c and twin_d");
	println!("ok: procs that a reload gave the same code keep their own local names");

	let sleeping = sleep_named_twins(&mut client);
	let changes = source.edit("reload.dm", "NAMED_TWINS(twin_c, c_local,", "NAMED_TWINS(twin_c, c_renamed,");
	assert_eq!(changes.plan.reloads.len(), 1, "{:?}", changes.plan);
	assert_eq!(eval(&mut client, "#fail-reload copy"), "the next reload fails at Copy");
	let result = expect_reload_result(watcher.request(reload_request(&changes)));
	assert!(!result.applied, "a reload whose code couldn't be copied was applied: {:?}", result);
	assert_eq!(result.errors, ["/twin_c: asked for with #fail-reload"]);
	let mut names = named_twin_locals(&mut client);
	reload(&mut watcher, &changes);
	names.extend(named_twin_locals(&mut client));
	wake_named_twins(&mut client, sleeping, 4);
	let sleeping = sleep_named_twins(&mut client);
	names.extend(named_twin_locals(&mut client));
	wake_named_twins(&mut client, sleeping, 4);
	let expected = ["c_local", "d_local", "c_local", "d_local", "c_renamed", "d_local"];
	assert_eq!(names, expected, "the locals' names of old and new frames of twin_c and twin_d");
	println!("ok: a reload that renames only a local keeps the old name in old frames");

	let line = source.line_of("reload.dm", "PARAM_TWINS(twin_e, e_param,");
	let twin_f = ProcRef {
		path: "/twin_f".to_owned(),
		override_id: 0
	};
	match client.request(Request::ProcInfo { proc: twin_f }) {
		Response::ProcInfo(Ok(info)) => assert_eq!(info.reloaded, None, "twin_f was reloaded before its test"),
		other => panic!("expected ProcInfo for /twin_f, got {:?}", other)
	}
	let waiting = twin_number(&mut client, "twin_waiting");
	eval(&mut client, "twin_hold = TRUE");
	eval(&mut client, "twin_armed = /proc/twin_f");
	wait_for(&mut client, "twin_waiting", &(waiting + 1).to_string());
	let offset = offset_of(&mut client, "/twin_e", line);
	twin_breakpoint(&mut client, "twin_e", offset, true);
	let changes = source.edit("reload.dm", "twin_f, f_param)", "twin_f, f_renamed)");
	assert_eq!(changes.plan.reloads.len(), 1, "{:?}", changes.plan);
	reload(&mut watcher, &changes);
	let names = frame_names(&mut client, &["/twin_f"], true);
	twin_breakpoint(&mut client, "twin_e", offset, false);
	let runs = twin_number(&mut client, "twin_runs");
	eval(&mut client, "twin_hold = FALSE");
	wait_for(&mut client, "twin_runs", &(runs + 1).to_string());
	assert_eq!(names, [["src,usr,f_param"]], "the arguments of a call of twin_f from before the reload");
	println!("ok: a call that shared its code keeps its parameter names after a breakpoint and a rename");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"var/claimed_step = 0\n\n/proc/claimed_target()\n\ttwin_runs += claimed_step\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	let line = source.line_of("reload.dm", "\ttwin_runs += claimed_step");
	let claimed = |offset: u32| InstructionRef {
		proc: ProcRef {
			path: "/claimed_target".to_owned(),
			override_id: 0
		},
		offset
	};
	let on_line: Vec<u32> = line_instructions(&mut client, "/claimed_target", line)
		.into_iter()
		.map(|(offset, _)| offset)
		.collect();
	assert!(on_line.len() > 1, "line {} has one instruction: {:?}", line, on_line);
	let mut other = world.connect();
	let set = |client: &mut Client, offset: u32, condition: &str| {
		let response = client.request(Request::BreakpointSet {
			id: breakpoint_id(&claimed(offset), true),
			instruction: claimed(offset),
			condition: Some(condition.to_owned()),
			hit: None,
			log: None,
			exact: true
		});
		assert!(
			matches!(response, Response::BreakpointSet {
				result: BreakpointSetResult::Success { .. }
			}),
			"couldn't set a breakpoint at {}: {:?}",
			offset,
			response
		);
	};
	set(&mut client, 0, "claimed_step == 3");
	set(&mut client, on_line[0], "claimed_step == 1");
	set(&mut other, on_line[on_line.len() - 1], "claimed_step == 2");
	let changes = source.edit("reload.dm", "\ttwin_runs += claimed_step\n", "\ttwin_runs = twin_runs + claimed_step\n");
	let result = reload(&mut watcher, &changes);
	assert!(result.warnings.is_empty(), "{:?}", result.warnings);
	for step in 1..=4 {
		let runs = twin_number(&mut client, "twin_runs");
		eval(&mut client, &format!("claimed_step = {}", step));
		eval(&mut client, "twin_armed = /proc/claimed_target");
		if step < 4 {
			assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint, "step {}", step);
			assert_eq!(other.expect_pause(), BreakpointReason::Breakpoint, "step {}", step);
			client.resume();
		}
		wait_for(&mut client, "twin_runs", &(runs + step).to_string());
	}
	let unset = |client: &mut Client, offset: u32| {
		let response = client.request(Request::BreakpointUnset {
			id: breakpoint_id(&claimed(offset), true)
		});
		assert!(
			matches!(response, Response::BreakpointUnset { success: true, .. }),
			"couldn't take away the breakpoint at {}: {:?}",
			offset,
			response
		);
	};
	unset(&mut client, 0);
	unset(&mut client, on_line[0]);
	unset(&mut other, on_line[on_line.len() - 1]);
	other.leave();
	println!("ok: instruction breakpoints keep their conditions across a reload, at the start or on one instruction");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"var/chained_step = 0\n\n/proc/chained_target()\n\tchained_step += 1\n\ttwin_runs++\n\ttwin_runs++\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	let first = source.line_of("reload.dm", "\tchained_step += 1\n") + 1;
	let lines = [first, first + 1];
	for unset_first in [true, false] {
		let before = lines.map(|line| offset_of(&mut client, "/chained_target", line));
		for offset in before {
			twin_breakpoint(&mut client, "chained_target", offset, true);
		}
		let changes = source.edit(
			"reload.dm",
			"chained_step += 1\n\ttwin_runs",
			"chained_step += 1; chained_step += 1\n\ttwin_runs"
		);
		reload(&mut watcher, &changes);
		let after = lines.map(|line| offset_of(&mut client, "/chained_target", line));
		assert_eq!(
			after[0], before[1],
			"the first line's code should start where the second's did: {:?} {:?}",
			before, after
		);
		let [gone, kept] = if unset_first { [0, 1] } else { [1, 0] };
		twin_breakpoint(&mut client, "chained_target", before[gone], false);
		twin_run(&mut client, "chained_target", lines[kept], 2, true);
		twin_breakpoint(&mut client, "chained_target", before[kept], false);
		twin_run(&mut client, "chained_target", lines[kept], 2, false);
	}
	println!("ok: breakpoints that a reload moves onto each other's old places can each be taken away");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"var/exact_step = 0\nvar/exact_value = 0\n\n/proc/exact_target()\n\texact_step += 1\n\texact_value = exact_step && exact_step; \
		 twin_runs++\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	let line = source.line_of("reload.dm", "\texact_value = exact_step &&");
	let after_assignment = |client: &mut Client| {
		let on_line = line_instructions(client, "/exact_target", line);
		(on_line.iter().find(|(_, text)| text.contains("twin_runs")))
			.unwrap_or_else(|| panic!("no instruction on line {} uses twin_runs: {:?}", line, on_line))
			.0
	};
	let before = after_assignment(&mut client);
	assert!(before > offset_of(&mut client, "/exact_target", line), "twin_runs++ starts line {}", line);
	let exact = |offset: u32| InstructionRef {
		proc: ProcRef {
			path: "/exact_target".to_owned(),
			override_id: 0
		},
		offset
	};
	let response = client.request(Request::BreakpointSet {
		id: breakpoint_id(&exact(before), true),
		instruction: exact(before),
		condition: Some("exact_value == exact_step".to_owned()),
		hit: None,
		log: None,
		exact: true
	});
	assert!(
		matches!(response, Response::BreakpointSet {
			result: BreakpointSetResult::Success { .. }
		}),
		"couldn't set a breakpoint at {}: {:?}",
		before,
		response
	);
	let changes = source.edit("reload.dm", "\texact_step += 1\n", "\texact_step = exact_step + 1\n");
	let result = reload(&mut watcher, &changes);
	assert!(result.warnings.is_empty(), "{:?}", result.warnings);
	let after = after_assignment(&mut client);
	let runs = twin_number(&mut client, "twin_runs");
	eval(&mut client, "exact_value = 0");
	eval(&mut client, "twin_armed = /proc/exact_target");
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_offset(&mut client), ("/exact_target".to_owned(), after));
	client.resume();
	wait_for(&mut client, "twin_runs", &(runs + 1).to_string());
	twin_breakpoint(&mut client, "exact_target", before, false);
	twin_run(&mut client, "exact_target", line, 1, false);
	println!("ok: an instruction breakpoint after a short-circuit on a line a reload keeps stays on its instruction");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/proc/loop_target()\n\tvar/i = 0\n\twhile (i < 1)\n\t\ti++\n\t\ttwin_runs++; continue\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	let line = source.line_of("reload.dm", "\t\ttwin_runs++; continue");
	let continue_jump = |client: &mut Client| {
		let on_line = line_instructions(client, "/loop_target", line);
		(on_line.iter().find(|(_, text)| text.starts_with("Jmp")))
			.unwrap_or_else(|| panic!("line {} has no jump: {:?}", line, on_line))
			.0
	};
	let before = continue_jump(&mut client);
	twin_breakpoint(&mut client, "loop_target", before, true);
	let changes = source.edit("reload.dm", "\t\ti++\n", "\t\ti = i + 1\n");
	let result = reload(&mut watcher, &changes);
	assert!(result.warnings.is_empty(), "{:?}", result.warnings);
	let after = continue_jump(&mut client);
	assert_ne!(after, before, "the reload didn't move line {}", line);
	let runs = twin_number(&mut client, "twin_runs");
	eval(&mut client, "twin_armed = /proc/loop_target");
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_offset(&mut client), ("/loop_target".to_owned(), after));
	client.resume();
	wait_for(&mut client, "twin_runs", &(runs + 1).to_string());
	twin_breakpoint(&mut client, "loop_target", before, false);
	twin_run(&mut client, "loop_target", line, 1, false);
	println!("ok: an instruction breakpoint on a moved line's jump back to a fixed loop stays on its instruction");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/proc/end_target()\n\tif (twin_runs >= 0)\n\t\ttwin_runs++\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	let line = source.line_of("reload.dm", "\tif (twin_runs >= 0)");
	let jump = |client: &mut Client| {
		let on_line = line_instructions(client, "/end_target", line);
		(on_line.iter().find(|(_, text)| text.starts_with('J')))
			.unwrap_or_else(|| panic!("line {} has no jump: {:?}", line, on_line))
			.0
	};
	let at = jump(&mut client);
	twin_breakpoint(&mut client, "end_target", at, true);
	for (from, to) in [
		("\t\ttwin_runs++\n", "\t\ttwin_runs = twin_runs + 1\n"),
		("\t\ttwin_runs = twin_runs + 1\n", "\t\ttwin_runs = twin_runs + 1\n\t\ttwin_runs += 0\n"),
		("\t\ttwin_runs += 0\n", "\t\ttwin_runs += 0\n\ttwin_waiting += 0\n")
	] {
		let changes = source.edit("reload.dm", from, to);
		let result = reload(&mut watcher, &changes);
		assert!(result.warnings.is_empty(), "{:?}", result.warnings);
		assert_eq!(jump(&mut client), at, "the reload moved the jump on line {}", line);
		let runs = twin_number(&mut client, "twin_runs");
		eval(&mut client, "twin_armed = /proc/end_target");
		assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
		assert_eq!(paused_offset(&mut client), ("/end_target".to_owned(), at), "after the edit to {:?}", to);
		client.resume();
		wait_for(&mut client, "twin_runs", &(runs + 1).to_string());
	}
	twin_breakpoint(&mut client, "end_target", at, false);
	twin_run(&mut client, "end_target", line, 1, false);
	println!("ok: an instruction breakpoint on a jump to a proc's end stays on its instruction when the body before the end changes");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/proc/slot_target()\n\tvar/slot_x = 1\n\ttwin_runs += slot_x\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	let adds = |client: &mut Client, line: u32| {
		let on_line = line_instructions(client, "/slot_target", line);
		let at = (on_line.iter().rev().find(|(_, text)| text.contains("twin_runs")))
			.unwrap_or_else(|| panic!("no instruction on line {} uses twin_runs: {:?}", line, on_line))
			.0;
		assert!(at > on_line[0].0, "line {} starts with twin_runs: {:?}", line, on_line);
		at
	};
	let before = adds(&mut client, source.line_of("reload.dm", "\ttwin_runs += slot_x"));
	twin_breakpoint(&mut client, "slot_target", before, true);
	let changes = source.edit("reload.dm", "\tvar/slot_x = 1\n", "\tvar/slot_w = 0\n\tvar/slot_x = 1 + slot_w\n");
	let result = reload(&mut watcher, &changes);
	assert!(result.warnings.is_empty(), "{:?}", result.warnings);
	let line = source.line_of("reload.dm", "\ttwin_runs += slot_x");
	let after = adds(&mut client, line);
	let runs = twin_number(&mut client, "twin_runs");
	eval(&mut client, "twin_armed = /proc/slot_target");
	assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
	assert_eq!(paused_offset(&mut client), ("/slot_target".to_owned(), after));
	client.resume();
	wait_for(&mut client, "twin_runs", &(runs + 1).to_string());
	twin_breakpoint(&mut client, "slot_target", before, false);
	twin_run(&mut client, "slot_target", line, 1, false);
	println!("ok: an instruction breakpoint after a local stays on its instruction when a new local renumbers it");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/proc/swap_target()\n\tvar/swapped = 0\n\ttwin_runs++\n\tswapped += 2\n\tswapped *= 3\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	let line = source.line_of("reload.dm", "\tswapped += 2");
	let swaps = [
		("\tswapped += 2\n\tswapped *= 3\n", "\tswapped *= 3\n\tswapped += 2\n"),
		("\tswapped *= 3\n\tswapped += 2\n", "\tswapped += 2\n\tswapped *= 3\n")
	];
	for (from, to) in swaps {
		let before = offset_of(&mut client, "/swap_target", line);
		twin_breakpoint(&mut client, "swap_target", before, true);
		let changes = source.edit("reload.dm", from, to);
		let result = reload(&mut watcher, &changes);
		assert!(result.warnings.is_empty(), "{:?}", result.warnings);
		twin_run(&mut client, "swap_target", line + 1, 1, true);
		twin_breakpoint(&mut client, "swap_target", before, false);
		twin_run(&mut client, "swap_target", line, 1, false);
	}
	println!("ok: breakpoints on two swapped statements move with them");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/proc/local_dropper()\n\tvar/dropped = 1\n\ttwin_waiting += 1\n\twhile (twin_hold) sleep(1)\n\ttwin_runs += \
		 dropped\n\n/proc/lone_local()\n\tvar/gone = 2\n\treturn gone\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	let waiting = twin_number(&mut client, "twin_waiting");
	eval(&mut client, "twin_hold = TRUE");
	eval(&mut client, "twin_armed = /proc/local_dropper");
	wait_for(&mut client, "twin_waiting", &(waiting + 1).to_string());
	let changes = source.edit_many("reload.dm", &[
		("\tvar/dropped = 1\n", ""),
		("\ttwin_runs += dropped\n", "\ttwin_runs += 1\n")
	]);
	assert_eq!(changes.plan.reloads.len(), 1, "{:?}", changes.plan);
	reload(&mut watcher, &changes);
	eval(&mut client, "twin_armed = /proc/local_dropper");
	wait_for(&mut client, "twin_waiting", &(waiting + 2).to_string());
	let mut names = frame_locals(&mut client, &["/local_dropper"]).remove(0);
	names.sort();
	assert_eq!(names, ["", "dropped"], "the locals of a call from before the reload and one after");
	let runs = twin_number(&mut client, "twin_runs");
	eval(&mut client, "twin_hold = FALSE");
	wait_for(&mut client, "twin_runs", &(runs + 2).to_string());
	let (table, references) = locals_references(&mut client, "/proc/local_dropper");
	let changes = source.edit("reload.dm", "\tvar/gone = 2\n\treturn gone\n", "\treturn 2\n");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "lone_local()"), "2");
	assert_eq!(
		locals_references(&mut client, "/proc/lone_local"),
		(table.clone(), references + 1),
		"the empty locals table that a second proc got"
	);
	assert_eq!(locals_references(&mut client, "/proc/local_dropper"), (table, references + 1));
	println!("ok: a reload that removes a proc's last local gives new calls no locals");

	let sleeping = sleep_named_twins(&mut client);
	let changes = source.edit("reload.dm", "twin_runs += second_local * 2; }", "twin_runs += second_local * 3; }");
	assert_eq!(changes.plan.reloads.len(), 1, "{:?}", changes.plan);
	reload(&mut watcher, &changes);
	let back = source.edit_many("reload.dm", &[
		("second_local * 3", "second_local * 2"),
		("twin_d, d_local)", "twin_d, d_again)")
	]);
	assert_eq!(back.plan.reloads.len(), 1, "{:?}", back.plan);
	assert_eq!(eval(&mut client, "#fail-reload copy"), "the next reload fails at Copy");
	let result = expect_reload_result(watcher.request(reload_request(&back)));
	assert!(!result.applied, "a reload back to code whose copy failed was applied: {:?}", result);
	assert_eq!(result.errors, ["/twin_d: asked for with #fail-reload"]);
	wake_named_twins(&mut client, sleeping, 4);
	let sleeping = sleep_named_twins(&mut client);
	let mut names = named_twin_locals(&mut client);
	wake_named_twins(&mut client, sleeping, 5);
	reload(&mut watcher, &back);
	let sleeping = sleep_named_twins(&mut client);
	names.extend(named_twin_locals(&mut client));
	wake_named_twins(&mut client, sleeping, 4);
	let expected = ["c_renamed", "d_local", "c_renamed", "d_again"];
	assert_eq!(
		names, expected,
		"the locals' names of twin_c and twin_d after the refused reload and after the retry"
	);
	println!("ok: a reload back to earlier code checks the copy for new local names first");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"#define SPLIT_TWINS(g_step, h_step) /proc/twin_g() { twin_waiting += 1; while (twin_hold) sleep(1); twin_runs += g_step; } /proc/twin_h() \
		 { twin_waiting += 1; while (twin_hold) sleep(1); twin_runs += h_step; }\nSPLIT_TWINS(1, 1)\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	let line = source.line_of("reload.dm", "SPLIT_TWINS(1, 1)");
	let waiting = twin_number(&mut client, "twin_waiting");
	eval(&mut client, "twin_hold = TRUE");
	eval(&mut client, "twin_armed = /proc/twin_h");
	wait_for(&mut client, "twin_waiting", &(waiting + 1).to_string());
	let offset = offset_of(&mut client, "/twin_g", line);
	twin_breakpoint(&mut client, "twin_g", offset, true);
	let changes = source.edit("reload.dm", "SPLIT_TWINS(1, 1)", "SPLIT_TWINS(1, 3)");
	assert_eq!(changes.plan.reloads.len(), 1, "{:?}", changes.plan);
	reload(&mut watcher, &changes);
	let (old_code, sleeping) = sleeping_listing(&mut client, "/twin_h");
	let twin_g = proc_listing(&mut client, "/twin_g");
	twin_breakpoint(&mut client, "twin_g", offset, false);
	let runs = twin_number(&mut client, "twin_runs");
	eval(&mut client, "twin_hold = FALSE");
	wait_for(&mut client, "twin_runs", &(runs + 1).to_string());
	assert!(old_code, "the call of twin_h from before the reload doesn't run old code");
	assert_eq!(sleeping, twin_g, "the code of a call of twin_h from before the reload");
	println!("ok: a call that shared its code shows that code after a breakpoint split it and a reload");

	let mut move_big = |from: &str, to: &str, code: bool| {
		let changes = source.edit("reload.dm", from, to);
		assert_eq!(changes.plan.reloads.len(), code as usize, "{:?}", changes.plan);
		changes
	};
	let changes = move_big("BIG1 BIG1 }\n", "BIG1 BIG1 }\nBIG_PROC(big_proc, big_param)\n", false);
	assert_eq!(changes.plan.new_procs.len(), 1, "{:?}", changes.plan);
	reload(&mut watcher, &changes);
	let changes = move_big("}\nBIG_PROC(big_proc, big_param)", "}\n\nBIG_PROC(big_proc, big_param)", false);
	reload(&mut watcher, &changes);
	let changes = move_big("}\n\nBIG_PROC(big_proc, big_param)", "}\n\n\nBIG_PROC(big_proc, big_renamed)", true);
	reload(&mut watcher, &changes);
	let changes = move_big("}\n\n\nBIG_PROC(big_proc, big_renamed)", "}\n\nBIG_PROC(big_proc, big_renamed)", false);
	let result = expect_reload_result(watcher.request(reload_request(&changes)));
	assert!(
		!result.applied,
		"a line move that needed a copy of 65,535 words was applied: {:?}",
		result
	);
	assert_eq!(result.errors, ["/big_proc: moving its lines gives back code that older calls run with \
	                            other names, and 65535 words leave no room for a copy with the new \
	                            names. Rebuild to move them"]);
	println!("ok: a line move refuses a 65,535-word proc's old code that has other names");
}

fn locals_references(client: &mut Client, proc: &str) -> (String, u32) {
	let answer = eval(client, &format!("#locals-refs {}", proc));
	let parsed = answer
		.strip_suffix(" references")
		.and_then(|rest| rest.split_once(": 0 slots, "))
		.and_then(|(table, references)| Some((table.to_owned(), references.parse().ok()?)));
	parsed.unwrap_or_else(|| panic!("expected an empty locals table for {}, got {}", proc, answer))
}

fn line_instructions(client: &mut Client, proc: &str, line: u32) -> Vec<(u32, String)> {
	let proc = ProcRef {
		path: proc.to_owned(),
		override_id: 0
	};
	let code = match client.request(Request::ProcInfo { proc: proc.clone() }) {
		Response::ProcInfo(Ok(info)) => info.code,
		other => panic!("expected ProcInfo for {}, got {:?}", proc.path, other)
	};
	let Response::Disassemble(Ok(disassembly)) = client.request(Request::Disassemble { code }) else {
		panic!("expected the disassembly of {}", proc.path);
	};
	(disassembly.instructions.into_iter())
		.filter(|at| at.line == Some(line) && !at.text.starts_with("Dbg"))
		.map(|at| (at.offset as u32, at.text))
		.collect()
}

fn sleep_named_twins(client: &mut Client) -> u32 {
	let waiting = twin_number(client, "twin_waiting");
	eval(client, "twin_hold = TRUE");
	eval(client, "twin_armed = /proc/twin_c");
	wait_for(client, "twin_waiting", &(waiting + 1).to_string());
	eval(client, "twin_armed = /proc/twin_d");
	wait_for(client, "twin_waiting", &(waiting + 2).to_string());
	twin_number(client, "twin_runs")
}

fn wake_named_twins(client: &mut Client, runs: u32, added: u32) {
	eval(client, "twin_hold = FALSE");
	wait_for(client, "twin_runs", &(runs + added).to_string());
}

fn named_twin_locals(client: &mut Client) -> Vec<String> {
	let twins = ["/twin_c", "/twin_d"];
	frame_locals(client, &twins)
		.into_iter()
		.zip(twins)
		.map(|(frames, twin)| frames.into_iter().next().unwrap_or_else(|| panic!("no frame of {}", twin)))
		.collect()
}

fn frame_locals(client: &mut Client, procs: &[&str]) -> Vec<Vec<String>> {
	frame_names(client, procs, false)
}

fn frame_names(client: &mut Client, procs: &[&str], arguments: bool) -> Vec<Vec<String>> {
	let response = client.request(Request::Pause);
	assert!(matches!(response, Response::Ack), "expected Ack for Pause, got {:?}", response);
	assert_eq!(client.expect_pause(), BreakpointReason::Pause);
	let Response::Stacks { stacks, .. } = client.request(Request::Stacks { sleeping: Some(1000) }) else {
		panic!("expected Stacks");
	};
	let mut found = vec![vec![]; procs.len()];
	for stack in &stacks {
		let Response::StackFrames { frames, .. } = client.request(Request::StackFrames {
			stack_id: stack.id,
			start_frame: None,
			count: None
		}) else {
			panic!("expected StackFrames");
		};
		for frame in frames {
			let Some(at) = procs.iter().position(|proc| *proc == frame.instruction.proc.path) else {
				continue;
			};
			let scope = match client.request(Request::Scopes { frame_id: frame.id }) {
				Response::Scopes { arguments: Some(scope), .. } if arguments => scope,
				Response::Scopes { locals: Some(scope), .. } if !arguments => scope,
				other => panic!("expected the scopes of {}, got {:?}", procs[at], other)
			};
			let Response::Variables { vars } = client.request(Request::Variables {
				vars: scope,
				start: None,
				count: None
			}) else {
				panic!("expected the locals of {}", procs[at]);
			};
			let locals: Vec<String> = vars.into_iter().map(|var| var.name).filter(|name| name != ".").collect();
			found[at].push(locals.join(","));
		}
	}
	client.resume();
	found
}

fn listing(client: &mut Client, code: CodeRef) -> Vec<String> {
	match client.request(Request::Disassemble { code }) {
		Response::Disassemble(Ok(disassembly)) => disassembly.instructions.into_iter().map(|at| at.text).collect(),
		other => panic!("expected the disassembly of {:?}, got {:?}", code, other)
	}
}

fn proc_listing(client: &mut Client, proc: &str) -> Vec<String> {
	let code = match client.request(Request::ProcInfo {
		proc: ProcRef {
			path: proc.to_owned(),
			override_id: 0
		}
	}) {
		Response::ProcInfo(Ok(info)) => info.code,
		other => panic!("expected ProcInfo for {}, got {:?}", proc, other)
	};
	listing(client, code)
}

fn sleeping_listing(client: &mut Client, proc: &str) -> (bool, Vec<String>) {
	let response = client.request(Request::Pause);
	assert!(matches!(response, Response::Ack), "expected Ack for Pause, got {:?}", response);
	assert_eq!(client.expect_pause(), BreakpointReason::Pause);
	let Response::Stacks { stacks, .. } = client.request(Request::Stacks { sleeping: Some(1000) }) else {
		panic!("expected Stacks");
	};
	let mut found = None;
	for stack in &stacks {
		let Response::StackFrames { frames, .. } = client.request(Request::StackFrames {
			stack_id: stack.id,
			start_frame: None,
			count: None
		}) else {
			panic!("expected StackFrames");
		};
		found = found.or(frames.into_iter().find(|frame| frame.instruction.proc.path == proc));
	}
	let frame = found.unwrap_or_else(|| panic!("no sleeping call of {}", proc));
	let code = listing(client, frame.code);
	client.resume();
	(frame.old_code, code)
}

fn twin_breakpoints(client: &mut Client, source: &Source, step: u32) {
	let line = source.line_of("reload.dm", "TWINS(twin_a, twin_b)");
	let offset = offset_of(client, "/twin_a", line);
	assert_eq!(offset_of(client, "/twin_b", line), offset);
	let twin_a = ProcRef {
		path: "/twin_a".to_owned(),
		override_id: 0
	};
	let code = match client.request(Request::ProcInfo { proc: twin_a.clone() }) {
		Response::ProcInfo(Ok(info)) => info.code,
		other => panic!("expected ProcInfo for /twin_a, got {:?}", other)
	};
	let Response::Disassemble(Ok(disassembly)) = client.request(Request::Disassemble { code }) else {
		panic!("expected the disassembly of /twin_a");
	};
	let counts = disassembly
		.instructions
		.iter()
		.rev()
		.find(|at| at.text.contains("twin_runs"))
		.unwrap_or_else(|| panic!("/twin_a has no instruction with twin_runs: {:?}", disassembly.instructions))
		.offset as u32;

	let waiting = twin_number(client, "twin_waiting");
	eval(client, "twin_hold = TRUE");
	eval(client, "twin_armed = /proc/twin_b");
	wait_for(client, "twin_waiting", &(waiting + 1).to_string());
	twin_breakpoint(client, "twin_a", counts, true);
	let runs = twin_number(client, "twin_runs");
	eval(client, "twin_hold = FALSE");
	wait_for(client, "twin_runs", &(runs + step).to_string());
	twin_run(client, "twin_a", line, step, true);
	twin_breakpoint(client, "twin_a", counts, false);

	twin_breakpoint(client, "twin_a", offset, true);
	twin_run(client, "twin_b", line, step, false);
	twin_run(client, "twin_a", line, step, true);
	twin_breakpoint(client, "twin_b", offset, true);
	twin_breakpoint(client, "twin_a", offset, false);
	twin_run(client, "twin_b", line, step, true);
	twin_run(client, "twin_a", line, step, false);
	twin_breakpoint(client, "twin_b", offset, false);
	twin_run(client, "twin_b", line, step, false);
}

fn twin_breakpoint(client: &mut Client, twin: &str, offset: u32, set: bool) {
	let instruction = InstructionRef {
		proc: ProcRef {
			path: format!("/{}", twin),
			override_id: 0
		},
		offset
	};
	match client.request(if set {
		Request::BreakpointSet {
			id: breakpoint_id(&instruction, true),
			instruction,
			condition: None,
			hit: None,
			log: None,
			exact: true
		}
	} else {
		Request::BreakpointUnset {
			id: breakpoint_id(&instruction, true)
		}
	}) {
		Response::BreakpointSet {
			result: BreakpointSetResult::Success { .. }
		}
		| Response::BreakpointUnset { success: true, .. } => {}
		other => panic!(
			"couldn't {} the breakpoint at {} in {}: {:?}",
			if set { "set" } else { "take away" },
			offset,
			twin,
			other
		)
	}
}

fn twin_number(client: &mut Client, var: &str) -> u32 {
	eval(client, var).parse().unwrap()
}

fn twin_run(client: &mut Client, twin: &str, line: u32, step: u32, stops: bool) {
	let runs = twin_number(client, "twin_runs");
	eval(client, &format!("twin_armed = /proc/{}", twin));
	if stops {
		assert_eq!(client.expect_pause(), BreakpointReason::Breakpoint);
		assert_eq!(paused_line(client), (format!("/{}", twin), Some(line)));
		client.resume();
	}
	wait_for(client, "twin_runs", &(runs + step).to_string());
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
	assert_eq!(eval(&mut client, "flock_size = old_dog ? flock_size + 1 : 0"), "14");
	assert_eq!(eval(&mut client, "global.flock_size + 1"), "15");
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

	let changes = source.edit("reload.dm", "\tvar/spare = 1\n", "");
	reload(&mut watcher, &changes);
	let changes = source.edit("reload.dm", "/datum/animal\n", "/datum/animal\n\tvar/spare = 3\n");
	let result = reload(&mut watcher, &changes);
	assert!(
		result
			.warnings
			.iter()
			.any(|w| w.contains("var spare uses the declaration that the running world still has")),
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

	let changes = source.edit(
		"reload.dm",
		"/proc/cat_says()\n",
		"/datum/animal/named\n\tsound = /client/proc/named_verb\n\n/client/proc/named_verb()\n\tset name = \"Named\"\n\treturn \
		 5\n\n/proc/cat_says()\n"
	);
	let result = reload(&mut watcher, &changes);
	assert!(result.failures.is_empty(), "{:?}", result.failures);
	assert_eq!(eval(&mut client, "(new /datum/animal/named).sound == /client/proc/named_verb"), "1");
	println!("ok: new proc references in type defaults");

	assert_eq!(eval(&mut client, "(new /datum/early).speak()"), "\"declaration, then override\"");
	assert_eq!(eval(&mut client, "\"[/datum/early/proc/speak]\""), "\"/datum/early/proc/speak\"");
	let changes = source.edit("reload.dm", "\"declaration, then [..()]\"", "\"edited declaration, then [..()]\"");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /datum/early).speak()"), "\"edited declaration, then override\"");
	println!("ok: a declaration after its override is edited");

	assert_eq!(
		eval(&mut client, "(new /datum/early_apart).speak()"),
		"\"declaration apart, then override apart\""
	);
	let changes = source.edit("main.dm", "\"override apart\"", "\"edited override apart\"");
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "(new /datum/early_apart).speak()"),
		"\"declaration apart, then edited override apart\""
	);
	println!("ok: an override in a file before its declaration's is edited");

	assert_eq!(eval(&mut client, "calls_early_global()"), "\"early declared\"");
	assert_eq!(eval(&mut client, "early_global()"), "\"early declared\"");
	let changes = source.edit("reload.dm", "\"early declared\"", "\"edited early declared\"");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "calls_early_global()"), "\"edited early declared\"");
	assert_eq!(eval(&mut client, "early_global()"), "\"edited early declared\"");
	println!("ok: a global declaration after another definition is edited");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/datum/early_new/speak()\n\treturn \"new override\"\n\n/datum/early_new/proc/speak()\n\treturn \"new declaration, then \
		 [..()]\"\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "(new /datum/early_new).speak()"),
		"\"new declaration, then new override\""
	);
	println!("ok: an override and its later declaration added together");

	assert_eq!(eval(&mut client, "(new /datum/late).speak()"), "\"late declaration, then \"");
	let changes = source.edit(
		"reload.dm",
		"/datum/late/proc/speak()\n",
		"/datum/late/speak()\n\treturn \"late override\"\n\n/datum/late/proc/speak()\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /datum/late).speak()"), "\"late declaration, then late override\"");
	assert_eq!(eval(&mut client, "\"[/datum/late/proc/speak]\""), "\"/datum/late/proc/speak\"");
	println!("ok: an override added before its declaration");

	assert_eq!(eval(&mut client, "call(new /datum/twice, /datum/twice/speak)()"), "\"first override\"");
	assert_eq!(eval(&mut client, "call(new /datum/early, /datum/early/speak)()"), "\"override\"");
	println!("ok: a proc path without proc names the first override");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/proc/added_twice()\n\treturn \"declared\"\n\n/added_twice()\n\treturn \"unreachable\"\n\n/proc/calls_added_twice()\n\treturn \
		 added_twice()\n\n/datum/added_thrice/proc/speak()\n\treturn \"declaration\"\n\n/datum/added_thrice/speak()\n\treturn \"first \
		 override\"\n\n/datum/added_thrice/speak()\n\treturn \"second override\"\n\n/proc/calls_added_thrice()\n\treturn call(new \
		 /datum/added_thrice, /datum/added_thrice/speak)()\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "calls_added_twice()"), "\"declared\"");
	assert_eq!(eval(&mut client, "calls_added_thrice()"), "\"first override\"");
	println!("ok: new code names the definitions added with it");
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

	let verb_refs = |client: &mut Client| ["Greet", "Friends", "Pals"].map(|text| eval(client, &format!("#string-refs {}", text)));
	let greet = verb_refs(&mut client)[0].clone();
	let changes = source.edit("reload.dm", "\tset category = \"Helpers\"\n", "\tset category = \"Friends\"\n");
	assert_eq!((changes.plan.reloads.len(), changes.plan.settings.len()), (1, 1));
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "verb_report(old_helper)"), "\"Greet:Friends\"");
	assert_eq!(
		eval(&mut client, "verb_report(new /mob/helper/small)"),
		"\"Greet:Friends,Wave Hello:,Greet:Friends\""
	);
	assert_eq!(
		verb_refs(&mut client),
		[greet.as_str(), "2", "0"],
		"after the category changed to Friends"
	);
	for (from, to, refs) in [("Friends", "Pals", ["0", "2"]), ("Pals", "Friends", ["2", "0"])] {
		let category = |name: &str| format!("\tset category = \"{}\"\n", name);
		let changes = source.edit("reload.dm", &category(from), &category(to));
		reload(&mut watcher, &changes);
		assert_eq!(
			verb_refs(&mut client),
			[greet.as_str(), refs[0], refs[1]],
			"after the category changed to {}",
			to
		);
	}
	println!("ok: a verb's setting holds one reference to its new string and releases the old one");

	let changes = source.edit("reload.dm", "/mob/helper/small/greet()\n\treturn \"[..()]!\"\n", "");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "verb_report(new /mob/helper/small)"), "\"Wave Hello:,Greet:Friends\"");
	assert_eq!(eval(&mut client, "helper_greets(new /mob/helper/small)"), "\"hi\"");
	println!("ok: verb changes");

	let changes = source.edit(
		"reload.dm",
		"/proc/crate_report(type)\n",
		"/obj/crate/verb/shout()\n\tset name = \"Shout\"\n\tset desc = \"Loudly\"\n\tset category = \"Noises\"\n\treturn \
		 \"shout\"\n\n/proc/crate_report(type)\n"
	);
	reload(&mut watcher, &changes);
	let refs = |client: &mut Client| ["Shout", "Yell", "Holler", "Loudly", "Noises"].map(|text| eval(client, &format!("#string-refs {}", text)));
	assert_eq!(refs(&mut client), ["1", "0", "0", "1", "1"], "references to the new verb's settings");
	for (from, to, expected) in [
		("Shout", "Yell", ["0", "1", "0", "1", "1"]),
		("Yell", "Holler", ["0", "0", "1", "1", "1"])
	] {
		let name = |name: &str| format!("\tset name = \"{}\"\n", name);
		let changes = source.edit("reload.dm", &name(from), &name(to));
		reload(&mut watcher, &changes);
		assert_eq!(refs(&mut client), expected, "references after the new verb was renamed to {}", to);
	}
	assert_eq!(eval(&mut client, "call(new /obj/crate, \"Holler\")()"), "\"shout\"");
	println!("ok: a new verb's settings hold one reference to their strings and a rename releases the old name");

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
		"/mob/helper/small\n\tsight = 4\n",
		"/mob/helper\n\tkey = \"helper_key\"\n\n/mob/helper/small\n\tsight = 4\n"
	);
	assert_eq!(changes.plan.builtin_defaults.len(), 1, "{:?}", changes.plan.builtin_defaults);
	assert_eq!(changes.plan.builtin_defaults[0].types.len(), 3, "{:?}", changes.plan.builtin_defaults);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "#string-refs helper_key"),
		"3",
		"the references to a key that three mob types got"
	);
	let changes = source.edit(
		"reload.dm",
		"/mob/helper/small\n\tsight = 4\n",
		"/mob/helper/small\n\tsight = 4\n\n/mob/helper/later\n"
	);
	assert_eq!(changes.plan.new_types.len(), 1, "{:?}", changes.plan.new_types);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "#string-refs helper_key"),
		"4",
		"the references to a key that a new subtype inherited"
	);
	let changes = source.edit("reload.dm", "\tkey = \"helper_key\"\n", "\tkey = null\n");
	assert_eq!(changes.plan.builtin_defaults[0].types.len(), 4, "{:?}", changes.plan.builtin_defaults);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "#string-refs helper_key"),
		"0",
		"the references to a key that no mob type has"
	);
	assert_eq!(eval(&mut client, "isnull((new /mob/helper/small).key)"), "1");
	println!("ok: a mob key default gives each type a reference");

	let refs = |client: &mut Client, texts: [&str; 2]| texts.map(|text| eval(client, &format!("#string-refs {}", text)));
	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/obj/crate/labelled\n\tdesc = \"label_one\"\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	let changes = source.edit("reload.dm", "/proc/first_def()\n", "/obj/crate/labelled/small\n\n/proc/first_def()\n");
	reload(&mut watcher, &changes);
	assert_eq!(
		refs(&mut client, ["label_one", "label_two"]),
		["2", "0"],
		"after a subtype inherited a desc"
	);
	let changes = source.edit("reload.dm", "\"label_one\"", "\"label_two\"");
	reload(&mut watcher, &changes);
	assert_eq!(refs(&mut client, ["label_one", "label_two"]), ["0", "2"], "after the desc changed");
	let changes = source.edit(
		"reload.dm",
		"/obj/crate/labelled/small\n",
		"/obj/crate/labelled/small\n\tdesc = \"label_small\"\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		refs(&mut client, ["label_two", "label_small"]),
		["1", "1"],
		"after the subtype got its own desc"
	);
	let changes = source.edit("reload.dm", "\tdesc = \"label_small\"\n", "");
	assert_eq!(changes.plan.builtin_inherited.len(), 1, "{:?}", changes.plan);
	reload(&mut watcher, &changes);
	assert_eq!(
		refs(&mut client, ["label_two", "label_small"]),
		["2", "0"],
		"after the subtype inherited the desc again"
	);
	assert_eq!(eval(&mut client, "initial(/obj/crate/labelled/small::desc)"), "\"label_two\"");
	println!("ok: a type releases the strings it no longer has");

	let target_name: u32 = eval(&mut client, "#string-refs render_target").parse().unwrap();
	assert_eq!(eval(&mut client, "#fail-reload append"), "the next reload fails at Append");
	let changes = source.edit(
		"reload.dm",
		"\tkey = null\n",
		"\tkey = null\n\tdesc = \"unwritten_desc\"\n\trender_target = \"unwritten_target\"\n\n/proc/unappended()\n\treturn 1\n"
	);
	let result = expect_reload_result(watcher.request(reload_request(&changes)));
	assert!(!result.applied, "a reload whose proc wasn't added was applied: {:?}", result);
	assert_eq!(
		refs(&mut client, ["unwritten_desc", "unwritten_target"]),
		["0", "0"],
		"the references to a desc and a render_target that a failed reload didn't write"
	);
	assert_eq!(
		eval(&mut client, "#string-refs render_target"),
		target_name.to_string(),
		"the references to render_target's name after a failed reload"
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		refs(&mut client, ["unwritten_desc", "unwritten_target"]),
		["4", "4"],
		"the references to a desc and a render_target that four mob types got"
	);
	assert_eq!(
		eval(&mut client, "#string-refs render_target"),
		(target_name + 4).to_string(),
		"the references to render_target's name after four mob types got it"
	);
	println!("ok: a failed reload holds no reference to the strings it didn't write");

	let small_target = (
		"/mob/helper/small\n\tsight = 4\n",
		"/mob/helper/small\n\tsight = 4\n\trender_target = \"unwritten_target\"\n"
	);
	let changes = source.edit_many("reload.dm", &[
		("\trender_target = \"unwritten_target\"\n", "\trender_target = \"second_target\"\n"),
		small_target
	]);
	reload(&mut watcher, &changes);
	let targets = |client: &mut Client| {
		let [unwritten, second] = refs(client, ["unwritten_target", "second_target"]);
		[unwritten, second, eval(client, "#string-refs render_target")]
	};
	let counts = |unwritten: u32, second: u32, names: u32| [unwritten, second, target_name + names].map(|count| count.to_string());
	assert_eq!(
		targets(&mut client),
		counts(1, 3, 4),
		"after a render_target changed and a subtype kept the old one"
	);
	let changes = source.edit_many("reload.dm", &[
		(small_target.1, small_target.0),
		("/mob/helper/later\n", "/mob/helper/later\n\n/mob/helper/latest\n")
	]);
	assert_eq!(changes.plan.builtin_inherited.len(), 1, "{:?}", changes.plan);
	reload(&mut watcher, &changes);
	assert_eq!(
		targets(&mut client),
		counts(0, 5, 5),
		"after a subtype inherited a render_target and a new one got it"
	);
	let changes = source.edit_many("reload.dm", &[("\trender_target = \"second_target\"\n", ""), small_target]);
	reload(&mut watcher, &changes);
	assert_eq!(
		targets(&mut client),
		counts(1, 0, 1),
		"after a render_target was cleared and a subtype kept its own"
	);
	assert_eq!(eval(&mut client, "(new /mob/helper/small).render_target"), "\"unwritten_target\"");
	assert_eq!(eval(&mut client, "(new /mob/helper/latest).render_target"), "null");
	println!("ok: a list-backed default releases the strings a type no longer has");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/datum/labelled\n\tvar/label\n\n/datum/labelled/kid\n\n/proc/first_def()\n"
	);
	reload(&mut watcher, &changes);
	let labels = |client: &mut Client| ["default_one", "default_two", "default_three"].map(|text| eval(client, &format!("#string-refs {}", text)));
	let report = "\"[(new /datum/labelled).label] [(new /datum/labelled/kid).label]\"";
	assert_eq!(eval(&mut client, "#fail-reload append"), "the next reload fails at Append");
	let changes = source.edit(
		"reload.dm",
		"\tvar/label\n",
		"\tvar/label = \"default_one\"\n\n/proc/label_unappended()\n\treturn 1\n"
	);
	let result = expect_reload_result(watcher.request(reload_request(&changes)));
	assert!(!result.applied, "a reload whose proc wasn't added was applied: {:?}", result);
	assert_eq!(labels(&mut client), ["0", "0", "0"], "after a failed reload");
	reload(&mut watcher, &changes);
	assert_eq!(labels(&mut client), ["2", "0", "0"], "after a default reached a type and its subtype");
	let changes = source.edit_many("reload.dm", &[
		("\"default_one\"", "\"default_two\""),
		("/datum/labelled/kid\n", "/datum/labelled/kid\n\tlabel = \"default_one\"\n")
	]);
	reload(&mut watcher, &changes);
	assert_eq!(
		labels(&mut client),
		["1", "1", "0"],
		"after the default changed and the subtype kept the old one"
	);
	let changes = source.edit("reload.dm", "\"default_two\"", "\"default_three\"");
	reload(&mut watcher, &changes);
	assert_eq!(labels(&mut client), ["1", "0", "1"], "after the default changed again");
	assert_eq!(eval(&mut client, report), "\"default_three default_one\"");
	let changes = source.edit_many("reload.dm", &[
		("\tlabel = \"default_one\"\n", ""),
		("/datum/labelled/kid\n", "/datum/labelled/kid\n\n/datum/labelled/later\n")
	]);
	reload(&mut watcher, &changes);
	assert_eq!(
		labels(&mut client),
		["0", "0", "3"],
		"after the subtype took the default and a new one got it"
	);
	let changes = source.edit("reload.dm", "\tvar/label = \"default_three\"\n", "\tvar/label\n");
	reload(&mut watcher, &changes);
	assert_eq!(labels(&mut client), ["0", "0", "0"], "after the default was cleared");
	assert_eq!(eval(&mut client, report), "\" \"");
	println!("ok: a var's default holds one reference to its string for each type that has it");

	let added = |client: &mut Client| ["added_label", "added_text"].map(|text| eval(client, &format!("#string-refs {}", text)));
	assert_eq!(added(&mut client), ["0", "0"], "before the var was added");
	let changes = source.edit(
		"reload.dm",
		"/datum/labelled/kid\n",
		"/datum/labelled/kid\n\tvar/added_label = \"added_text\"\n"
	);
	let Request::HotReload(mut refused) = reload_request(&changes) else {
		unreachable!()
	};
	let mut missing = refused.plan.new_vars[0].clone();
	missing.var = "added_missing".to_owned();
	missing.value = dmasm::reload::ConstValue::Path("/datum/no_such_type".to_owned());
	refused.plan.new_vars.push(missing);
	let result = expect_reload_result(watcher.request(Request::HotReload(refused)));
	assert!(!result.applied, "a reload with a var of a missing type was applied: {:?}", result);
	assert_eq!(added(&mut client), ["0", "0"], "after a refused reload");
	reload(&mut watcher, &changes);
	assert_eq!(added(&mut client), ["1", "2"], "after the var was added to a type");
	assert_eq!(eval(&mut client, "(new /datum/labelled/kid).added_label"), "\"added_text\"");
	println!("ok: a refused new var holds no reference to its name or its string");

	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/proc/static_label()\n\tvar/static/label = \"static_text\"\n\treturn label\n\n/proc/first_def()\n"
	);
	let Request::HotReload(mut refused) = reload_request(&changes) else {
		unreachable!()
	};
	refused.plan.new_vars.push(dmasm::reload::NewVar {
		type_path: "/datum/labelled".to_owned(),
		var: "static_missing".to_owned(),
		flags: 0,
		value: dmasm::reload::ConstValue::Path("/datum/no_such_type".to_owned()),
		init: None,
		moved_from: vec![]
	});
	let result = expect_reload_result(watcher.request(Request::HotReload(refused)));
	assert!(!result.applied, "a reload with a var of a missing type was applied: {:?}", result);
	for text in ["static_text", "/proc/static_label", "static_label"] {
		let references = eval(&mut client, &format!("#string-refs {}", text));
		assert_eq!(references, "0", "{} after a refused reload", text);
	}
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "#string-refs static_text"), "1", "after the static was added");
	assert_eq!(eval(&mut client, "static_label()"), "\"static_text\"");
	println!("ok: a refused new proc and static local hold no reference to their strings");

	let changes = source.edit_many("reload.dm", &[
		(
			"\tvar/added_label = \"added_text\"\n",
			"\tvar/static/added_label = \"kid_text\"\n\tvar/list/kid_list = list()\n"
		),
		(
			"/proc/first_def()\n",
			"/proc/prefab_kid()\n\treturn new /datum/labelled/kid{kid_list = 5}\n\n/proc/first_def()\n"
		)
	]);
	let Request::HotReload(mut refused) = reload_request(&changes) else {
		unreachable!()
	};
	assert_eq!(changes.plan.redeclared.len(), 1, "{:?}", changes.plan);
	assert_eq!(changes.plan.init_procs.len(), 1, "{:?}", changes.plan);
	refused.plan.new_vars.push(dmasm::reload::NewVar {
		type_path: "/datum/labelled".to_owned(),
		var: "kid_missing".to_owned(),
		flags: 0,
		value: dmasm::reload::ConstValue::Path("/datum/no_such_type".to_owned()),
		init: None,
		moved_from: vec![]
	});
	let result = expect_reload_result(watcher.request(Request::HotReload(refused)));
	assert!(!result.applied, "a reload with a var of a missing type was applied: {:?}", result);
	let kid_strings = ["kid_text", "/datum/labelled/kid/proc/(init)", "/datum/labelled/kid/proc/(map_init)"];
	let references = kid_strings.map(|text| eval(&mut client, &format!("#string-refs {}", text)));
	assert_eq!(references, ["0", "0", "0"], "references to {:?} after a refused reload", kid_strings);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "#string-refs kid_text"), "1", "after the static was set");
	let kid = "\"[(new /datum/labelled/kid).added_label] [length((new /datum/labelled/kid).kid_list)] [prefab_kid():kid_list]\"";
	assert_eq!(eval(&mut client, kid), "\"kid_text 0 5\"");
	println!("ok: a refused static value, init proc and prefab hold no reference to their strings");

	assert_eq!(eval(&mut client, "#fail-reload append"), "the next reload fails at Append");
	let changes = source.edit(
		"reload.dm",
		"/proc/first_def()\n",
		"/obj/crate/retried\n\tdesc = \"first try\"\n\n/proc/retried_maker()\n\treturn new /obj/crate/retried\n\n/proc/first_def()\n"
	);
	let result = expect_reload_result(watcher.request(reload_request(&changes)));
	assert!(!result.applied, "a reload whose proc wasn't added was applied: {:?}", result);
	source.snapshot = Some(changes.before);
	let changes = source.edit("reload.dm", "\"first try\"", "\"second try\"");
	assert_eq!(changes.plan.new_types.len(), 1, "{:?}", changes.plan);
	let result = expect_reload_result(watcher.request(reload_request(&changes)));
	assert!(!result.applied, "a reload that kept a failed reload's desc was applied: {:?}", result);
	assert_eq!(result.errors, ["/obj/crate/retried: a reload that failed added this type with another \
	                            parent or other built-in values. Undo the change to them, or rebuild \
	                            the .dmb and restart the world"]);
	source.snapshot = Some(changes.before);
	let changes = source.edit("reload.dm", "\"second try\"", "\"first try\"");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "retried_maker().desc"), "\"first try\"");
	println!("ok: a retried reload refuses other values for a type that a failed reload added");

	let orphan = "/obj/crate/orphan\n\n/proc/orphan_maker()\n\treturn new /obj/crate/orphan\n\n";
	assert_eq!(eval(&mut client, "#fail-reload append"), "the next reload fails at Append");
	let changes = source.edit("reload.dm", "/proc/first_def()\n", &format!("{}/proc/first_def()\n", orphan));
	let result = expect_reload_result(watcher.request(reload_request(&changes)));
	assert!(!result.applied, "a reload whose proc wasn't added was applied: {:?}", result);
	source.snapshot = Some(changes.before);
	let crate_desc = (
		"/obj/crate\n\tname = \"crate\"\n",
		"/obj/crate\n\tname = \"crate\"\n\tdesc = \"parent_desc\"\n"
	);
	let changes = source.edit_many("reload.dm", &[(orphan, ""), crate_desc]);
	assert!(changes.plan.new_types.is_empty(), "{:?}", changes.plan);
	reload(&mut watcher, &changes);
	let changes = source.edit("reload.dm", "/proc/first_def()\n", &format!("{}/proc/first_def()\n", orphan));
	assert_eq!(changes.plan.new_types.len(), 1, "{:?}", changes.plan);
	let result = expect_reload_result(watcher.request(reload_request(&changes)));
	assert!(
		!result.applied,
		"a reload that kept a desc the type inherited before its parent's changed was applied: {:?}",
		result
	);
	assert_eq!(result.errors, ["/obj/crate/orphan: a reload that failed added this type, and the \
	                            values it inherits have changed since. Undo the change to them, or \
	                            rebuild the .dmb and restart the world"]);
	source.snapshot = Some(changes.before);
	let changes = source.edit_many("reload.dm", &[(orphan, ""), (crate_desc.1, crate_desc.0)]);
	reload(&mut watcher, &changes);
	let changes = source.edit("reload.dm", "/proc/first_def()\n", &format!("{}/proc/first_def()\n", orphan));
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "orphan_maker().desc"), "null");
	println!("ok: a retried reload refuses a type whose parent changed after a failed reload added it");

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
	let changes = source.edit("reload.dm", "\tpixel_y = 3\n\tlayer = 7\n", "\t// pixel_y = 3\n\t// layer = 7\n");
	assert_eq!(changes.plan.builtin_inherited.len(), 2, "{:?}", changes.plan.builtin_inherited);
	assert_eq!(changes.plan.len(), 2, "{:?}", changes.plan);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /obj/crate).layer"), "3");
	assert_eq!(eval(&mut client, "(new /obj/crate/big_crate).layer"), "3");
	assert_eq!(eval(&mut client, "(new /obj/crate/big_crate).pixel_y"), "0");
	assert_eq!(eval(&mut client, "(new /obj/crate/fancy).pixel_y"), "0");
	println!("ok: built-in override removal");

	let changes = source.edit("reload.dm", "/proc/first_def()\n", "/area\n\tlayer = 9\n\n/proc/first_def()\n");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /area).layer"), "9");
	println!("ok: built-in type default");

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
		 4\n\n/datum/hr_b/proc/f()\n\treturn \"b\"\n\n/datum/hr_t\n\tparent_type = /datum/hr_a\n\tvar/hr_own = 1\n\n/datum/hr_t/f()\n\treturn \"t\" \
		 + ..()\n\n/datum/hr_t/sub\n\n/obj/hr_oa\n\ticon_state = \"a\"\n\tlayer = 7\n\n/obj/hr_ob\n\tpixel_x = 2\n\tbound_width = \
		 16\n\tbound_height = 48\n\n/obj/hr_ot\n\tparent_type = /obj/hr_oa\n\tbound_width = 8\n\tstep_x = \
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
	let names = |client: &mut Client| ["hr_own", "step_x"].map(|name| eval(client, &format!("#string-refs {}", name)));
	let before = names(&mut client);
	reload(&mut watcher, &changes);
	let after_one = names(&mut client);
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
	let [own_after, _] = names(&mut client);
	assert_eq!(
		[own_after, after_one[1].clone()],
		before,
		"the references to the names of a var and a built-in after their types changed parents"
	);
	assert_eq!(eval(&mut client, "(new /datum/hr_t/sub).f()"), "\"ta\"");
	assert_eq!(eval(&mut client, "length(typesof(/datum/hr_a))"), "3");
	assert_eq!(eval(&mut client, "(new /datum/hr_t).x"), "1");
	assert_eq!(eval(&mut client, "(new /datum/hr_t).hr_own"), "1");
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
	let upd_paths =
		|client: &mut Client| ["/datum/mv_hud/human/proc/upd", "/datum/mv_hud/human/upd"].map(|path| eval(client, &format!("#string-refs {}", path)));
	assert_eq!(upd_paths(&mut client), ["1", "0"], "references to the new proc's path");
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
			.any(|w| w.contains("subtypes keep their declarations, values and var IDs")),
		"{:?}",
		result.warnings
	);
	assert_eq!(
		upd_paths(&mut client),
		["0", "1"],
		"references to the proc's paths after its parent declared it"
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

	let pan = |client: &mut Client| eval(client, "#string-refs pan");
	let before = pan(&mut client);
	for _ in 0..2 {
		let changes = source.edit("main.dm", "\tfrequency = 3\n", "\tfrequency = 3\n\tpan = 4\n");
		reload(&mut watcher, &changes);
		let changes = source.edit("main.dm", "\tfrequency = 3\n\tpan = 4\n", "\tfrequency = 3\n");
		assert_eq!(changes.plan.builtin_inherited.len(), 1, "{:?}", changes.plan);
		reload(&mut watcher, &changes);
	}
	assert_eq!(
		pan(&mut client),
		before,
		"references to the var name pan after a sound type inherited it twice"
	);
	assert_eq!(eval(&mut client, "(new /sound/hr_st).pan"), "0");
	println!("ok: a built-in var a sound type inherits again takes no reference to its name");

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

	let world_calls = |client: &mut Client| {
		eval(
			client,
			"\"[world_call(\"Hail\")] [world_call(\"Salute\")] [world_call(\"Bow\")] [world_call(\"hail\")]\""
		)
	};
	assert_eq!(world_calls(&mut client), "\"hailed missing missing hailed\"");
	let changes = source.edit("main.dm", "\tset name = \"Hail\"\n", "\tset name = \"Salute\"\n");
	reload(&mut watcher, &changes);
	assert_eq!(world_calls(&mut client), "\"missing hailed missing hailed\"");
	let changes = source.edit("main.dm", "\tset name = \"Salute\"\n", "\tset name = \"Bow\"\n");
	reload(&mut watcher, &changes);
	assert_eq!(world_calls(&mut client), "\"missing missing hailed hailed\"");
	println!("ok: world proc rename");

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
	assert!(result.warnings.iter().all(|w| w.contains("Existing instances")), "{:?}", result.warnings);
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
		result.warnings.iter().any(|w| w.contains("crate.dmi has the same bytes as resource")),
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
