use std::{
	path::{Path, PathBuf},
	time::Duration
};

use auxtools_protocol::{HotReloadRequest, HotReloadResult, Request, Response};
use serde_json::{json, Value};
use test_support::{
	dap::Launcher,
	standin::{Answer, Answers, StandIn, World},
	temp::TempDir
};

pub const HELD: &[(&str, &str)] = &[
	("held.dme", "#include \"held_b.dm\"\n#include \"held.dm\"\n"),
	("held_b.dm", "#define HELD_K 1\n\n/proc/held_b()\n\treturn 1\n"),
	("held.dm", "var/held_n = 1\nvar/held_k = HELD_K + 1\n\n/proc/held_value()\n\treturn 1\n")
];

pub fn code(name: &str, files: &[(&str, &str)]) -> (TempDir, PathBuf) {
	let dir = TempDir::new(&format!("standin_{}", name));
	let built = std::time::SystemTime::now() - Duration::from_secs(60);
	for (file, text) in files {
		std::fs::write(dir.join(file), text).unwrap();
		std::fs::File::options()
			.write(true)
			.open(dir.join(file))
			.unwrap()
			.set_modified(built - Duration::from_secs(60))
			.unwrap();
	}
	let dme = dir.join(files[0].0);
	std::fs::File::create(dme.with_extension("dmb")).unwrap().set_modified(built).unwrap();
	(dir, dme)
}

#[track_caller]
pub fn edit(dir: &std::path::Path, file: &str, old: &str, new: &str) {
	let path = dir.join(file);
	let text = std::fs::read_to_string(&path).unwrap();
	assert!(text.contains(old), "{} has no {:?}", file, old);
	std::fs::write(&path, text.replacen(old, new, 1)).unwrap();
}

pub fn answers(dme: &Path) -> Answers {
	let dmb = dme.with_extension("dmb");
	Box::new(move |request| {
		Answer::Reply(match request {
			Request::ByondVersion => Response::ByondVersion { version: 516, build: 1689 },
			Request::Journal { .. } => Response::Journal {
				entries: vec![],
				first: 1,
				last: 1
			},
			Request::StdDef => Response::StdDef(None),
			Request::Reloader { .. } => Response::Reloader(Ok(())),
			Request::WorldInfo => Response::WorldInfo {
				args: vec!["DreamDaemon".to_owned(), dmb.to_string_lossy().into_owned(), "-trusted".to_owned()],
				cwd: dmb.parent().unwrap().to_string_lossy().into_owned(),
				port: 0
			},
			Request::Configured => Response::Ack,
			Request::Status => Response::Status { paused: None },
			Request::Stacks { .. } => Response::Stacks { stacks: vec![], sleeping: 0 },
			Request::CatchRuntimes { .. } | Request::Defines(_) | Request::JournalAdd(_) | Request::Disconnect => return Answer::Nothing,
			_ => return Answer::ToTest
		})
	})
}

pub fn applied() -> Response {
	Response::HotReload(HotReloadResult {
		applied: true,
		errors: vec![],
		warnings: vec![],
		skipped: vec![],
		resources: vec![]
	})
}

#[track_caller]
pub fn reloaded(world: &mut World) -> HotReloadRequest {
	let message = world.next("a reload", |request| matches!(request, Request::HotReload(_)));
	world.answer(&message, applied());
	match message.request {
		Request::HotReload(request) => request,
		_ => unreachable!()
	}
}

pub fn procs(reload: &HotReloadRequest) -> Vec<(String, String)> {
	reload
		.plan
		.reloads
		.iter()
		.map(|proc| (proc.proc.path.clone(), format!("{:?}", proc.nodes)))
		.collect()
}

#[track_caller]
pub fn reboots(stand_in: &StandIn, mut world: World) -> World {
	world.next(
		"world.Reboot()",
		|request| matches!(request, Request::Eval { command, ending: true, .. } if command == "world.Reboot()")
	);
	world.event(Response::Disconnect);
	world.close();
	stand_in.world()
}

#[track_caller]
pub fn rebooted(stand_in: &StandIn, launcher: &mut Launcher, world: World) -> World {
	launcher.request("auxtoolsReboot", json!({}));
	reboots(stand_in, world)
}

pub fn held(launcher: &mut Launcher) -> Vec<(String, String)> {
	let mut last = launcher.event("auxtoolsReload");
	while let Some(next) = launcher.event_within("auxtoolsReload", Duration::from_millis(300)) {
		last = next;
	}
	let mut held: Vec<(String, String)> = last["body"]["problems"]
		.as_array()
		.unwrap()
		.iter()
		.map(|p| (p["message"].as_str().unwrap().to_owned(), p["until"].as_str().unwrap().to_owned()))
		.collect();
	held.sort();
	held
}

pub fn launcher() -> Launcher {
	launcher_with(&[])
}

pub fn launcher_with(env: &[(&str, &str)]) -> Launcher {
	test_support::paths::use_launcher(env!("CARGO_BIN_EXE_auxtools"));
	let mut env = env.to_vec();
	env.push(("AUXTOOLS_WATCH_INTERVAL", "50"));
	let mut launcher = Launcher::start_with(&env);
	launcher.echo = false;
	launcher.initialize();
	launcher
}

#[track_caller]
pub fn attached(stand_in: &mut StandIn, dme: &Path) -> (Launcher, World) {
	let pid = stand_in.spawn();
	let mut launcher = launcher();
	launcher.request("attach", json!({ "processId": pid, "dme": dme, "hotReload": true }));
	let world = stand_in.world();
	launcher.event("initialized");
	launcher.request("configurationDone", json!({}));
	launcher.wait_output("Hot reload active");
	(launcher, world)
}

#[track_caller]
pub fn launched(stand_in: &mut StandIn, mut launcher: Launcher, dme: &Path, more: Value) -> (Launcher, World) {
	let reloads = more["hotReload"] == true;
	launcher.request("launch", stand_in.launch_arguments(dme, more));
	stand_in.started();
	let world = stand_in.world();
	launcher.event("initialized");
	launcher.request("configurationDone", json!({}));
	if reloads {
		launcher.wait_output("Hot reload active");
	}
	(launcher, world)
}

#[test]
#[ignore]
fn record() {
	use auxtools_cli::game::{Game, Launch};
	use test_support::paths;

	let (dir, dme) = code("record", HELD);
	paths::compile(&dme);
	let library = PathBuf::from(std::env::var("STANDIN_DEBUG_LIBRARY").unwrap());
	let log = std::fs::File::create(std::env::var("STANDIN_RECORD").unwrap()).unwrap();
	let world = Game::start(
		Launch {
			program: paths::find_dreamdaemon(),
			dmb: &dme.with_extension("dmb"),
			args: &["-trusted".to_owned()],
			env: &Default::default(),
			debug: Some((library, false))
		},
		|_, line| print!("  | {}", line),
		|_| {}
	)
	.unwrap();
	std::thread::sleep(Duration::from_secs(2));

	let mut stand_in = StandIn::new(Box::new(|_| Answer::ToTest));
	let pid = stand_in.relaying(world.pid(), log);
	let mut launcher = launcher();
	launcher.request("attach", json!({ "processId": pid, "dme": dme, "hotReload": true }));
	launcher.event("initialized");
	launcher.request("configurationDone", json!({}));
	launcher.wait_output("Hot reload active");
	launcher.request("threads", json!({}));
	launcher.request("evaluate", json!({ "expression": "held_value()", "context": "repl" }));

	let said = |output: &str| output.matches("Hot reload requires a reboot:").count();
	edit(&dir, "held.dm", "var/held_n = 1\n", "var/held_n = 2\n");
	edit(&dir, "held.dm", "\treturn 1\n", "\treturn 2\n");
	launcher.wait_until("the held reload", |output| said(output) > 0);
	edit(&dir, "held.dm", "var/held_n = 2\n", "var/held_n = 1\n");
	launcher.wait_until("the reload", |output| output.contains("Hot reload: ") && output.contains(" ms"));
	edit(&dir, "held.dm", "var/held_n = 1\n", "var/held_n = 3\n");
	launcher.wait_until("the held reload", |output| said(output) > 1);
	launcher.request("auxtoolsReboot", json!({}));
	launcher.wait_until("the reboot", |output| output.contains("The world restarted"));
	launcher.wait_until("the restored reloads", |output| {
		output.rsplit("The world restarted").next().unwrap().contains(" ms")
	});
	launcher.request("evaluate", json!({ "expression": "\"[held_n] [held_value()]\"", "context": "repl" }));
	launcher.request("disconnect", json!({ "terminateDebuggee": false }));
	world.kill();
}
