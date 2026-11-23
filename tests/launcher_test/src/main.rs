mod check;
mod inspect;
mod journal;
mod profile;

use std::{
	io::{BufRead, BufReader, Read},
	path::{Path, PathBuf},
	process::{Child, Command, Stdio},
	sync::{
		atomic::{AtomicBool, Ordering},
		Arc, Mutex
	},
	time::{Duration, Instant}
};

use serde_json::json;
use test_support::{
	cli::{auxtools, launcher_command, listed_world, listed_worlds, ran, Background, Cli},
	dap::{self, test_args, Launcher},
	paths::{self, library_path, ByondCommand},
	process,
	temp::TempDir
};

const TARGET_LINE: u64 = 28;

fn host_file(name: &str) -> PathBuf {
	std::env::current_dir().unwrap().join("tests/launcher_test_host").join(name)
}

fn at_target(launcher: &mut Launcher) {
	launcher.stopped("breakpoint");
	let frame = launcher.top_frame();
	assert!(frame["name"].as_str().unwrap().ends_with("target"), "stopped in {}", frame);
	assert_eq!(frame["line"], TARGET_LINE);
	let source = frame["source"]["path"].as_str().unwrap_or_default();
	assert!(source.ends_with("launcher_test_host.dm"), "the frame's source is {:?}", source);

	let scopes = launcher.request("scopes", json!({ "frameId": frame["id"] }));
	let arguments = scopes["scopes"]
		.as_array()
		.unwrap()
		.iter()
		.find(|s| s["name"] == "Arguments")
		.expect("no arguments scope")
		.clone();
	let vars = launcher.request("variables", json!({ "variablesReference": arguments["variablesReference"] }));
	let n = vars["variables"]
		.as_array()
		.unwrap()
		.iter()
		.find(|v| v["name"] == "n")
		.expect("no argument n")
		.clone();
	assert_eq!(n["value"], "21");
	assert_eq!(launcher.evaluate(&frame, "n + 1"), "22");
	println!("ok: line breakpoint and local value");
	launcher.resume();
}

fn at_second_target(launcher: &mut Launcher) {
	launcher.stopped("breakpoint");
	let frame = launcher.top_frame();
	assert!(frame["name"].as_str().unwrap().ends_with("second_target"), "stopped in {}", frame);
	println!("ok: function breakpoint");
	launcher.resume();
}

fn at_runtime(launcher: &mut Launcher) {
	launcher.stopped("exception");
	let info = launcher.request("exceptionInfo", json!({ "threadId": 0 }));
	assert_eq!(info["exceptionId"], "runtime error");
	let message = info["description"].as_str().unwrap();
	assert!(message.contains("cannot read from list"), "the runtime was {:?}", message);
	let frame = launcher.top_frame();
	assert!(frame["name"].as_str().unwrap().ends_with("fail"), "the runtime was in {}", frame);
	println!("ok: runtime stop ({})", message);
	launcher.resume();
}

const GROUPS: &[&str] = &[
	"launch",
	"attach",
	"inspect",
	"inspect_breakpoints",
	"inspect_spawned",
	"trace",
	"journal",
	"edited",
	"foreground",
	"detached",
	"reboots",
	"stops",
	"builds"
];

const ATTACHED: &[&str] = &["attach", "inspect", "inspect_breakpoints", "inspect_spawned", "trace", "journal"];

fn main() {
	let wanted: Vec<String> = std::env::args().skip(1).collect();
	if wanted == ["--list"] {
		for group in GROUPS {
			println!("{}", group);
		}
		return;
	}
	for name in &wanted {
		assert!(GROUPS.contains(&name.as_str()), "unknown group {}; available groups: {:?}", name, GROUPS);
	}
	std::env::set_var("AUXTOOLS_WATCH_INTERVAL", "50");
	paths::compile(&host_file("launcher_test_host.dme"));
	let _ = std::fs::remove_file(host_file("rebooted"));

	let server = dap::server();
	let library = library_path();
	let mut attached: Option<Attached> = None;
	let attach_checked = wanted.is_empty() || wanted.iter().any(|w| w == "attach");
	for group in GROUPS {
		if !wanted.is_empty() && !wanted.iter().any(|w| w == group) {
			continue;
		}
		println!("--- {}", group);
		if !ATTACHED.contains(group) {
			if let Some(world) = attached.take() {
				world.finish();
			}
		}
		if ATTACHED.contains(group) && attached.is_none() {
			attached = Some(attach_and_reload(&server, &library, attach_checked));
		}
		match (*group, &attached) {
			("launch", _) => launch(&server, &library),
			("attach", Some(world)) => {
				command_line(world.pid, &world.code);
				command_line_more(world.pid, &world.dir);
			}
			("inspect", Some(world)) => inspect::evaluations(world.pid, &world.dir, &world.code),
			("inspect_breakpoints", Some(world)) => inspect::breakpoints(world.pid, &world.dir, &world.code),
			("inspect_spawned", Some(world)) => inspect::spawned_and_steps(world.pid, &world.dir, &world.code),
			("trace", Some(world)) => {
				command_line_trace(world.pid, &world.code);
				profile::profile(world.pid, &world.dir, &world.code);
				journal::runtimes(world.pid, &world.dir);
			}
			("journal", Some(world)) => {
				journal::reloads(world.pid, &world.code);
				journal::hold_max(world.pid);
				journal::reboot(world.pid, &world.dir);
			}
			("edited", _) => edited_after_build(&server),
			("foreground", _) => foreground(&server),
			("detached", _) => detached(&server),
			("reboots", _) => reboots(&server),
			("stops", _) => stops(&server),
			("builds", _) => {
				without_debug_info(&server);
				unread_statics(&server);
				errored_baseline(&server);
				dmb_elsewhere(&server);
			}
			_ => unreachable!("group {} has no world", group)
		}
	}
	if let Some(world) = attached.take() {
		world.finish();
	}
	println!("all ok");
}

fn launch(server: &str, library: &Path) {
	command_line_shape();
	check::checks();
	another_version();
	no_debugger();

	let mut launcher = Launcher::start();
	launcher.initialize();

	launcher.request(
		"launch",
		json!({
			"dme": host_file("launcher_test_host.dme"),
			"byondPath": paths::find_byond(),
			"server": server,
			"debugLibrary": library,
			"args": test_args()
		})
	);

	launcher.started();
	println!("ok: debug library initialization");
	let set = launcher.request(
		"setBreakpoints",
		json!({
			"source": { "path": host_file("launcher_test_host.dm") },
			"breakpoints": [{ "line": TARGET_LINE }]
		})
	);
	assert_eq!(set["breakpoints"][0]["verified"], true, "the breakpoint wasn't set: {}", set);
	let set = launcher.request("setFunctionBreakpoints", json!({ "breakpoints": [{ "name": "/proc/second_target" }] }));
	assert_eq!(set["breakpoints"][0]["verified"], true, "the function breakpoint wasn't set: {}", set);
	launcher.request("setExceptionBreakpoints", json!({ "filters": ["runtimes"] }));
	launcher.request("configurationDone", json!({}));

	at_target(&mut launcher);
	at_second_target(&mut launcher);
	at_runtime(&mut launcher);

	at_target(&mut launcher);
	assert!(
		launcher.output.contains("The world restarted"),
		"the launcher didn't see the world restart"
	);
	println!("ok: breakpoint restoration after reboot");
	at_second_target(&mut launcher);
	at_runtime(&mut launcher);

	launcher.event("terminated");
	launcher.event("exited");
	if server == "dreamdaemon" || server == "dd" || cfg!(unix) {
		assert!(
			launcher.output.contains("launcher test: world/New"),
			"the world's output wasn't passed on"
		);
		assert!(launcher.output.contains("launcher test: done"), "world did not complete the test");
		println!("ok: world output before exit");
	} else {
		println!("ok: world exit");
	}

	launcher.disconnect(json!({}));
	println!("ok: launcher exit");
	let _ = std::fs::remove_file(host_file("rebooted"));
}

fn command_line(pid: u64, code: &Path) {
	let pid = pid.to_string();
	let cli = Cli::new(&pid);

	assert_eq!(
		cli.run(&["status"], 0).lines().next().unwrap_or_default(),
		format!("pid {}: running", pid)
	);
	let printed = cli.run(&["wait", "pause", "--timeout", "1"], 2);
	assert_eq!(printed.trim(), "error: timed out after 1 second waiting for the world to pause.");

	let line = line_of(code, "\treturn 2").to_string();
	let printed = cli.run(&["wait", "pause", "--break", &format!("attach_host.dm:{}", line), "--timeout", "15"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused at a breakpoint\n#0 /value  attach_host.dm:", pid)) && printed.contains("#1 /run_until_stopped"),
		"unexpected wait output: {}",
		printed
	);
	let printed = cli.run(&["status"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused at a breakpoint\n#0 /value", pid)),
		"it didn't stay paused: {}",
		printed
	);
	let (got, after) = launcher_command(&["status", "--pid", &pid]);
	assert_eq!(
		(got, after.lines().next()),
		(0, printed.lines().next()),
		"unexpected status --pid output: {}",
		after
	);
	cli.run(&["continue"], 0);

	let declared = line_of(code, "/proc/value()");
	let printed = cli.run(
		&["wait", "pause", "--break", &format!("attach_host.dm:{}", declared), "--timeout", "15"],
		0
	);
	assert!(
		printed.starts_with(&format!(
			"pid {}: paused at a breakpoint\n#0 /value  attach_host.dm:{}\n",
			pid,
			declared + 1
		)),
		"unexpected wait output: {}",
		printed
	);
	assert_eq!(cli.run(&["eval", "6", "*", "7"], 0).trim(), "42");
	let printed = cli.run(&["step-over"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused after a step\n#0 /run_until_stopped", pid)),
		"step-over said {}",
		printed
	);
	let printed = cli.run(&["vars"], 0);
	assert!(printed.contains("locals:"), "unexpected vars output: {}", printed);
	assert_eq!(cli.run(&["continue"], 0).trim(), format!("pid {}: resumed", pid));
	assert_eq!(cli.run(&["continue"], 1).trim(), "error: the world is not paused.");
	assert_eq!(
		cli.run(&["stack"], 1).trim(),
		"error: the world is running. Pause it first with 'auxtools pause' or 'auxtools wait pause'."
	);
	println!("ok: CLI breakpoint inspection and stepping");

	let printed = cli.run(&["pause"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused by request\n#0 /run_until_stopped", pid)),
		"unexpected pause output: {}",
		printed
	);
	let printed = cli.run(&["wait", "pause", "--break", "/proc/value"], 0);
	assert!(
		printed.starts_with(&format!("already paused\npid {}: paused by request\n", pid)),
		"unexpected wait output: {}",
		printed
	);
	let resumed: serde_json::Value = serde_json::from_str(&cli.run(&["continue", "--json"], 0)).unwrap();
	assert_eq!(resumed, json!({ "pid": pid.parse::<u64>().unwrap(), "resumed": true }));
	let printed = cli.run(&["wait", "pause", "--break", "/proc/value", "--if", "1 == 2", "--timeout", "1.5"], 2);
	assert!(
		printed.contains("error: timed out after 1.5 seconds waiting for the world to pause."),
		"unexpected wait output: {}",
		printed
	);
	let printed = cli.run(&["wait", "pause", "--break", "nowhere.dm:3"], 1);
	assert!(
		printed.contains("the world has no code from a file named nowhere.dm"),
		"unexpected wait output: {}",
		printed
	);
	println!("ok: CLI pause and wait timeouts");
}

fn command_line_shape() {
	let (code, alone) = launcher_command(&[]);
	assert_eq!(code, 0, "{}", alone);
	assert!(
		alone.starts_with("auxtools - Launch, inspect, and debug BYOND worlds\n"),
		"auxtools said {}",
		alone
	);
	assert!(
		alone.contains("Usage:\n  auxtools [--pid PID] COMMAND [ARGS] [OPTIONS]\n"),
		"auxtools said {}",
		alone
	);
	for command in [
		"launch",
		"list",
		"status",
		"stop",
		"reboot",
		"log",
		"stack",
		"vars",
		"eval",
		"proc",
		"disassemble",
		"pause",
		"continue",
		"step-over",
		"step-in",
		"step-out",
		"wait",
		"trace",
		"profile",
		"check",
		"dap",
		"help",
		"version"
	] {
		assert!(
			alone.lines().any(|line| line.starts_with(&format!("  {} ", command))),
			"{} isn't in the help:\n{}",
			command,
			alone
		);
	}
	for args in [&["help"][..], &["--help"][..], &["-h"][..]] {
		let (code, printed) = launcher_command(args);
		assert_eq!(
			(code, printed == alone),
			(0, true),
			"unexpected output from `auxtools {}`: {}",
			args.join(" "),
			printed
		);
	}
	println!("ok: command listing and top-level help");

	let (code, of_wait) = launcher_command(&["help", "wait"]);
	assert_eq!(code, 0);
	assert!(
		of_wait.starts_with("Wait for a debugger or world lifecycle event.\n\nUsage:\n  auxtools [--pid PID] wait pause [OPTIONS]\n"),
		"help wait said {}",
		of_wait
	);
	assert_eq!(launcher_command(&["wait", "--help"]), (0, of_wait.clone()));
	let (code, bare) = launcher_command(&["wait"]);
	assert_eq!(code, 1, "{}", bare);
	assert_eq!(bare, format!("{}error: wait requires a command: pause, reload, or reboot.\n", of_wait));
	let (code, of_pause) = launcher_command(&["help", "wait", "pause"]);
	assert_eq!(code, 0);
	assert!(
		of_pause
			.starts_with("Wait for execution to pause and report the location and reason.\n\nUsage:\n  auxtools [--pid PID] wait pause [OPTIONS]\n")
			&& of_pause.contains("\n  --timeout SECONDS           Set the wait deadline. [default: unlimited]\n")
			&& of_pause.contains("\n  --pid PID                   Select the world process.\n")
			&& of_pause.contains("\n  --response-timeout SECONDS  Set the wait for each response from the world. [default: 30]\n")
			&& of_pause.contains("\n  -h, --help                  Show help for this command.\n")
			&& of_pause.contains("\nExit codes:\n  0  World paused, or was already paused.\n")
			&& !of_pause.contains("--log"),
		"help wait pause said {}",
		of_pause
	);
	assert_eq!(launcher_command(&["wait", "pause", "--help"]), (0, of_pause.clone()));
	assert_eq!(launcher_command(&["wait", "pause", "-h"]), (0, of_pause));
	let (code, of_trace) = launcher_command(&["help", "trace"]);
	assert_eq!(code, 0);
	assert!(
		of_trace
			.starts_with("Log or count breakpoint hits without pausing execution.\n\nUsage:\n  auxtools [--pid PID] trace LOCATION... [OPTIONS]\n")
			&& of_trace.contains("  --by caller "),
		"help trace said {}",
		of_trace
	);
	for (args, said) in [
		(
			&["help", "frobnicate"][..],
			"error: unknown command 'frobnicate'. Run 'auxtools --help' for available commands."
		),
		(
			&["help", "wait", "soon"],
			"error: unknown wait command 'soon'. Run 'auxtools wait --help' for available commands."
		),
		(
			&["frobnicate"],
			"error: unknown command 'frobnicate'. Run 'auxtools --help' for available commands."
		),
		(
			&["wait", "--break", "/proc/x"],
			"error: unexpected argument '--break' before a wait command: specify pause, reload, or reboot. Run 'auxtools wait --help' for usage."
		),
		(&["next"], "error: unknown command 'next'. Run 'auxtools --help' for available commands.")
	] {
		let (code, printed) = launcher_command(args);
		assert_eq!((code, printed.trim()), (1, said), "`auxtools {}`", args.join(" "));
	}
	println!("ok: command help and unknown-command diagnostics");

	let (code, version) = launcher_command(&["version"]);
	assert_eq!(code, 0);
	assert!(
		version.starts_with("auxtools ") && protocol(&version) > 0,
		"unexpected version output: {}",
		version
	);
	assert_eq!(launcher_command(&["--version"]), (0, version.clone()));
	assert_eq!(launcher_command(&["-V"]), (0, version.clone()));
	let (code, as_json) = launcher_command(&["version", "--json"]);
	let as_json: serde_json::Value = serde_json::from_str(&as_json).unwrap_or_else(|_| panic!("unexpected version --json output: {}", as_json));
	assert_eq!(code, 0);
	assert_eq!(as_json["protocol"], protocol(&version));
	assert_eq!(
		version.trim(),
		format!("auxtools {} (protocol {})", as_json["version"].as_str().unwrap(), as_json["protocol"])
	);
	println!("ok: version and protocol output");

	for (args, said) in [
		(&["vars", "--depth", "3"][..], "error: --depth requires --json."),
		(&["eval", "1", "--wait"], "error: --wait requires --spawn."),
		(
			&["eval", "1", "--frame", "0", "--src", "0x1"],
			"error: --frame and --src cannot be used together."
		),
		(
			&["eval", "1", "--frame", "0", "--spawn"],
			"error: --frame and --spawn cannot be used together."
		),
		(&["eval"], "error: eval requires an expression: EXPRESSION, --file FILE, or - for stdin."),
		(&["stop", "--kill-timeout", "1"], "error: --kill-timeout requires --kill."),
		(
			&["trace", "/proc/x", "--by", "n", "--log", "x"],
			"error: --by and --log cannot be used together."
		),
		(
			&["wait", "pause", "--no-hold", "--hold-max", "3"],
			"error: --no-hold and --hold-max cannot be used together."
		),
		(&["launch", "nothing.dmb", "--json"], "error: --json requires --detach."),
		(
			&["--pid", "1", "list"],
			"error: --pid cannot be used with 'list': it does not select an existing world."
		),
		(
			&["version", "--pid", "1"],
			"error: --pid cannot be used with 'version': it does not select an existing world."
		),
		(
			&["list", "--response-timeout", "1"],
			"error: --response-timeout cannot be used with 'list': it does not select an existing world."
		),
		(
			&["--response-timeout", "1", "stop"],
			"error: --response-timeout cannot be used with 'stop': it waits for the process to exit; use --timeout."
		),
		(
			&["trace"],
			"error: missing required argument: LOCATION. Run 'auxtools trace --help' for usage."
		),
		(
			&["trace", "/proc/x", "--count", "0"],
			"error: invalid value '0' for --count: expected a positive integer. Run 'auxtools trace --help' for usage."
		),
		(
			&["reboot", "--timeout", "soon"],
			"error: invalid value 'soon' for --timeout: expected a non-negative number of seconds. Run 'auxtools reboot --help' for usage."
		),
		(
			&["stack", "--sideways"],
			"error: unexpected argument '--sideways'. Run 'auxtools stack --help' for usage."
		),
		(
			&["vars", "--frame"],
			"error: --frame requires a value: N. Run 'auxtools vars --help' for usage."
		),
		(
			&["--pid", "soon", "status"],
			"error: invalid value 'soon' for --pid: expected a process ID. Run 'auxtools status --help' for usage."
		)
	] {
		let (code, printed) = launcher_command(args);
		assert_eq!((code, printed.trim()), (1, said), "`auxtools {}`", args.join(" "));
	}
	println!("ok: option validation before connection");

	let (code, list) = launcher_command(&["list", "--json"]);
	assert_eq!(code, 0, "{}", list);
	assert!(
		serde_json::from_str::<serde_json::Value>(&list).is_ok_and(|worlds| worlds.is_array()),
		"list --json said {}",
		list
	);
	let (code, list) = launcher_command(&["list"]);
	assert_eq!(code, 0, "{}", list);
	assert!(
		list.starts_with("PID ") || list == "No worlds found with the debug library loaded.\n",
		"list said {}",
		list
	);
	for gone in ["--list", "--build"] {
		let (code, printed) = launcher_command(&[gone]);
		assert_eq!(code, 1, "unexpected output from `auxtools {}`: {}", gone, printed);
	}
	println!("ok: world listing and legacy flag rejection");
}

fn protocol(version: &str) -> u32 {
	version
		.trim()
		.rsplit_once("(protocol ")
		.and_then(|(_, number)| number.strip_suffix(')'))
		.and_then(|number| number.parse().ok())
		.unwrap_or_else(|| panic!("unexpected version output: {}", version))
}

fn another_version() {
	use auxtools_cli::local;
	use auxtools_protocol::{old_socket_name, Hello, LAST_NAMED_PROTOCOL};

	let (_dir, dme) = attach_host_copy("another_version");
	let mut plain = KilledOnDrop(
		Command::new(paths::find_dreamdaemon())
			.with_byond_paths()
			.arg(dme.with_extension("dmb"))
			.args(test_args().unwrap_or_else(|| vec!["-trusted".to_owned()]))
			.stdout(std::process::Stdio::null())
			.stderr(std::process::Stdio::null())
			.spawn()
			.unwrap()
	);
	let pid = plain.id();
	let ours = protocol(&launcher_command(&["version"]).1);
	assert!(ours > LAST_NAMED_PROTOCOL, "this auxtools speaks protocol {}", ours);

	let stand_in = StandIn::listen(old_socket_name(LAST_NAMED_PROTOCOL, pid), drop);
	listed_and_refused(pid, LAST_NAMED_PROTOCOL, ours);
	drop(stand_in);
	println!("ok: incompatible protocol discovery and attach rejection");

	let stand_in = StandIn::listen(local::world_name(pid), |mut client| {
		let hello = Hello {
			protocol: auxtools_protocol::PROTOCOL_VERSION + 1,
			auxtools: "99.0.0".to_owned(),
			pid: std::process::id()
		};
		let _ = auxtools_protocol::write(&mut client, &hello);
		let mut sent = vec![];
		let _ = std::io::Read::read_to_end(&mut client, &mut sent);
		SENT_TO_NEWER.fetch_add(sent.len(), Ordering::SeqCst);
		GREETED.fetch_add(1, Ordering::SeqCst);
	});
	listed_and_refused(pid, ours + 1, ours);
	let deadline = Instant::now() + Duration::from_secs(10);
	while GREETED.load(Ordering::SeqCst) < 3 {
		assert!(
			Instant::now() < deadline,
			"the launcher connected {} times",
			GREETED.load(Ordering::SeqCst)
		);
		std::thread::sleep(Duration::from_millis(20));
	}
	drop(stand_in);
	assert_eq!(SENT_TO_NEWER.load(Ordering::SeqCst), 0, "a world of another version was sent requests");
	let _ = plain.kill();
	let _ = plain.wait();
	println!("ok: a hello with another protocol version is listed as incompatible and attach is rejected");
}

static SENT_TO_NEWER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static GREETED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn listed_and_refused(pid: u32, theirs: u32, ours: u32) {
	let (code, list) = launcher_command(&["list", "--json"]);
	assert_eq!(code, 0, "{}", list);
	let worlds: serde_json::Value = serde_json::from_str(&list).unwrap();
	let listed = worlds
		.as_array()
		.unwrap()
		.iter()
		.find(|world| world["pid"] == pid)
		.unwrap_or_else(|| panic!("the other version's world isn't listed: {}", list));
	assert_eq!(
		(&listed["protocol"], &listed["compatible"]),
		(&json!(theirs), &json!(false)),
		"{}",
		listed
	);
	let (code, table) = launcher_command(&["list"]);
	assert_eq!(code, 0, "{}", table);
	let header: Vec<&str> = table.lines().next().unwrap_or_default().split_whitespace().collect();
	assert_eq!(
		header,
		["PID", "PROGRAM", "PROTOCOL", "COMPATIBLE", "RESPONSIVE", "DMB", "CWD", "ARGS"],
		"list said {}",
		table
	);
	let row: Vec<&str> = table
		.lines()
		.find(|line| line.split_whitespace().next() == Some(pid.to_string().as_str()))
		.unwrap_or_else(|| panic!("the other version's world isn't in the table: {}", table))
		.split_whitespace()
		.collect();
	assert_eq!((row[2], row[3], row[4]), (theirs.to_string().as_str(), "no", "-"), "list said {}", table);
	assert_eq!(listed["responsive"], json!(null), "{}", listed);

	let pid_text = pid.to_string();
	let (code, printed) = launcher_command(&["--pid", &pid_text, "status"]);
	assert_eq!(code, 1, "{}", printed);
	assert!(
		printed.contains(&format!(
			"its debug library speaks protocol {}, and this auxtools speaks {}",
			theirs, ours
		)),
		"attaching to another version's world said {}",
		printed
	);
}

struct StandIn {
	name: String,
	done: Arc<AtomicBool>,
	listening: Option<std::thread::JoinHandle<()>>
}

impl StandIn {
	fn listen(name: String, answer: fn(interprocess::local_socket::Stream)) -> Self {
		use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions};
		let listener = ListenerOptions::new()
			.name(name.clone().to_ns_name::<GenericNamespaced>().unwrap())
			.create_sync()
			.unwrap();
		let done = Arc::new(AtomicBool::new(false));
		let stop = done.clone();
		let listening = std::thread::spawn(move || {
			for client in listener.incoming().flatten() {
				if stop.load(Ordering::SeqCst) {
					return;
				}
				std::thread::spawn(move || answer(client));
			}
		});
		StandIn {
			name,
			done,
			listening: Some(listening)
		}
	}
}

impl Drop for StandIn {
	fn drop(&mut self) {
		use interprocess::local_socket::{prelude::*, GenericNamespaced, Stream};
		self.done.store(true, Ordering::SeqCst);
		let Some(listening) = self.listening.take() else {
			return;
		};
		let deadline = Instant::now() + Duration::from_secs(10);
		while !listening.is_finished() && Instant::now() < deadline {
			if let Ok(name) = self.name.clone().to_ns_name::<GenericNamespaced>() {
				let _ = Stream::connect(name);
			}
			std::thread::sleep(Duration::from_millis(20));
		}
		if listening.is_finished() {
			let _ = listening.join();
		}
	}
}

struct KilledOnDrop(Child);

impl std::ops::Deref for KilledOnDrop {
	type Target = Child;

	fn deref(&self) -> &Child {
		&self.0
	}
}

impl std::ops::DerefMut for KilledOnDrop {
	fn deref_mut(&mut self) -> &mut Child {
		&mut self.0
	}
}

impl Drop for KilledOnDrop {
	fn drop(&mut self) {
		let _ = self.0.kill();
		let _ = self.0.wait();
	}
}

fn no_debugger() {
	use auxtools_cli::local;
	use auxtools_protocol::{Hello, Response, ServerMessage};

	const WHY: &str = "Couldn't find current_execution_context (BYOND 516.1600 is outside the checked range 1637..=1688)";
	let (_dir, dme) = attach_host_copy("no_debugger");
	let mut plain = KilledOnDrop(
		Command::new(paths::find_dreamdaemon())
			.with_byond_paths()
			.arg(dme.with_extension("dmb"))
			.args(test_args().unwrap_or_else(|| vec!["-trusted".to_owned()]))
			.stdout(std::process::Stdio::null())
			.stderr(std::process::Stdio::null())
			.spawn()
			.unwrap()
	);
	let pid = plain.id();
	let stand_in = StandIn::listen(local::world_name(pid), |mut client| {
		let _ = auxtools_protocol::write(&mut client, &Hello::new(env!("CARGO_PKG_VERSION")));
		let _ = auxtools_protocol::write(&mut client, &ServerMessage::event(Response::Unavailable { reason: WHY.to_owned() }));
		let _ = auxtools_protocol::write(&mut client, &ServerMessage::event(Response::Disconnect));
		let _ = std::io::copy(&mut client, &mut std::io::sink());
	});

	let mut launcher = Launcher::start();
	launcher.initialize();
	launcher.request("attach", json!({ "processId": pid }));
	launcher.event("terminated");
	std::thread::sleep(Duration::from_secs(1));
	launcher.has_event("output");
	let said = format!("Debugger initialization failed for process {}: {}", pid, WHY);
	assert!(
		launcher.count(&said) == 1 && launcher.output.contains("Detaching from this session") && !launcher.output.contains("reconnect"),
		"attaching said {}",
		launcher.output
	);
	launcher.disconnect(json!({}));
	assert!(plain.try_wait().unwrap().is_none(), "ending the session ended the world");
	println!("ok: initialization failure reported once on attach");

	let (code, list) = launcher_command(&["list", "--json"]);
	assert_eq!(code, 0, "{}", list);
	let worlds: serde_json::Value = serde_json::from_str(&list).unwrap();
	let listed = worlds
		.as_array()
		.unwrap()
		.iter()
		.find(|world| world["pid"] == pid)
		.unwrap_or_else(|| panic!("the world without the debugger isn't listed: {}", list));
	assert_eq!((&listed["compatible"], &listed["unavailable"]), (&json!(true), &json!(WHY)), "{}", listed);
	let pid_text = pid.to_string();
	let (code, printed) = launcher_command(&["--pid", &pid_text, "status"]);
	assert_eq!(code, 1, "{}", printed);
	assert!(
		printed.contains(&format!(
			"error: debugger initialization failed for pid {}: {}. The world is running; use 'auxtools stop' to terminate it.",
			pid, WHY
		)),
		"status said {}",
		printed
	);
	println!("ok: CLI reports debugger initialization failure");

	let (code, printed) = launcher_command(&["--pid", &pid_text, "stop", "--json"]);
	assert_eq!(code, 0, "{}", printed);
	assert!(
		printed.contains(r#"{"killed":true,"pid":"#) && printed.contains("world.Del() could not be called"),
		"stop said {}",
		printed
	);
	drop(stand_in);
	let deadline = Instant::now() + Duration::from_secs(10);
	while plain.try_wait().unwrap().is_none() {
		assert!(Instant::now() < deadline, "the world is still running after stop");
		std::thread::sleep(Duration::from_millis(100));
	}
	println!("ok: stop without debugger");
}

fn command_line_more(pid: u64, dir: &Path) {
	let pid = pid.to_string();
	let cli = Cli::new(&pid);

	let printed = cli.run(&["wait", "pause", "--break", "/proc/value", "--timeout", "15"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused at a breakpoint\n#0 /value", pid)),
		"unexpected wait output: {}",
		printed
	);
	let printed = cli.run(&["step-out", "--timeout", "15"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused after a step\n#0 /run_until_stopped", pid)),
		"step-out said {}",
		printed
	);
	let printed = cli.run(&["step-in"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused after a step\n#0 /run_until_stopped", pid)),
		"step-in said {}",
		printed
	);
	println!("ok: CLI Step Out and Step Into");

	let printed = cli.run(&["stack"], 0);
	assert!(
		printed.starts_with("#0 /run_until_stopped") && !printed.contains("stack 0"),
		"stack said {}",
		printed
	);
	let printed = cli.run(&["stack", "--all"], 0);
	let first = printed.split_once(": ").map(|(_, rest)| rest).unwrap_or_default();
	assert!(
		printed.starts_with("stack ")
			&& first.starts_with("/run_until_stopped\n#0 /run_until_stopped")
			&& printed.contains(": /doze (sleeping)\n#0 /doze  attach_host.dm:"),
		"stack --all said {}",
		printed
	);
	assert!(!printed.contains("auxtools_keepalive"), "stack --all said {}", printed);
	println!("ok: CLI running and sleeping stacks");

	let printed = cli.run(&["vars"], 0);
	assert!(
		printed.contains("arguments:\n  src = null") && printed.contains("locals:") && !printed.contains("globals:"),
		"unexpected vars output: {}",
		printed
	);
	let printed = cli.run(&["vars", "--frame", "0", "--globals"], 0);
	assert!(printed.contains("globals:"), "unexpected vars --globals output: {}", printed);
	let printed = cli.run(&["vars", "--frame", "7"], 1);
	assert!(
		printed.starts_with("error: frame 7 does not exist: the paused stack has "),
		"vars --frame 7 said {}",
		printed
	);
	assert_eq!(cli.run(&["eval", "--frame", "0", "6", "*", "7"], 0).trim(), "42");
	assert_eq!(cli.run(&["eval", "6", "*", "7", "--frame", "0"], 0).trim(), "42");
	assert_eq!(cli.run(&["eval", "-1"], 0).trim(), "-1");
	assert_eq!(cli.run(&["eval", "--", "-rounds", "<", "0"], 0).trim(), "1");
	let printed = cli.run(&["eval", "--frame", "7", "1"], 1);
	assert!(
		printed.starts_with("error: frame 7 does not exist: the paused stack has "),
		"eval --frame 7 said {}",
		printed
	);
	let printed = cli.run(&["eval", "1", "+"], 1);
	assert!(printed.starts_with("error: parser error: "), "unexpected eval output: {}", printed);
	assert_eq!(
		cli.run(&["eval", "nothing_called_this"], 1).trim(),
		"error: undefined var: nothing_called_this"
	);
	assert_eq!(cli.run(&["eval", "1", "/", "0"], 1).trim(), "error: runtime error: Division by zero");
	println!("ok: CLI frame variables, globals and evaluation");
	assert_eq!(cli.run(&["eval", "var/n = 0\nfor(var/i in 1 to 3)\n\tn += i\nn"], 0).trim(), "6");
	assert_eq!(
		cli.run(&["eval", "var/x = 1\nx = nothing_called_this"], 1).trim(),
		"error: line 2: undefined var: nothing_called_this"
	);
	println!("ok: CLI proc-body evaluation");
	cli.run(&["continue"], 0);

	std::fs::write(dir.join("fail"), "yes").unwrap();
	let printed = cli.run(&["wait", "pause", "--runtimes", "--timeout", "15"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused on a runtime error: cannot read from list\n#0 /fail", pid)),
		"wait --runtimes said {}",
		printed
	);
	cli.run(&["continue"], 0);
	println!("ok: CLI runtime wait");

	std::fs::write(dir.join("fail"), "yes").unwrap();
	let printed = cli.run(&["wait", "pause", "--runtime-filter", "!cannot read", "--timeout", "1.5"], 2);
	assert!(printed.contains("timed out"), "wait --runtime-filter said {}", printed);
	std::fs::write(dir.join("fail"), "yes").unwrap();
	let printed = cli.run(&["wait", "pause", "--runtime-filter", "bounds, FROM LIST", "--timeout", "15"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused on a runtime error: cannot read from list", pid)),
		"wait --runtime-filter said {}",
		printed
	);
	cli.run(&["continue"], 0);
	println!("ok: CLI runtime filter");
	let printed = cli.run(&["log", "--runtime-filter", "!cannot read", "--last", "3"], 0);
	assert!(!printed.contains("cannot read from list"), "log --runtime-filter said {}", printed);
	let printed = cli.run(&["log", "--runtime-filter", "FROM LIST", "--last", "1"], 0);
	assert!(
		printed.contains("runtime error: cannot read from list") && printed.matches(" UTC (world.time ").count() == 1,
		"log --runtime-filter said {}",
		printed
	);
	println!("ok: log runtime filter");

	let waiting = journal::wait_at(pid.parse().unwrap(), "/proc/fail");
	let printed = cli.run(&["wait", "pause", "--break", "/proc/tick", "--timeout", "15"], 0);
	assert!(printed.contains("#0 /tick"), "the other wait said {}", printed);
	cli.run(&["continue"], 0);
	std::fs::write(dir.join("fail"), "yes").unwrap();
	let (exit, printed) = waiting.finish();
	assert!(
		exit == 0 && printed.starts_with(&format!("pid {}: paused at a breakpoint\n#0 /fail", pid)),
		"the waiting client gave {}: {}",
		exit,
		printed
	);
	cli.run(&["continue"], 0);
	println!("ok: CLI wait passes over another client's pause");

	let printed = cli.run(&["wait", "pause", "--break", "/proc/value", "--no-hold", "--timeout", "15"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused at a breakpoint\n#0 /value", pid)),
		"wait --no-hold said {}",
		printed
	);
	assert_eq!(
		cli.run(&["status"], 0).lines().next().unwrap_or_default(),
		format!("pid {}: running", pid)
	);
	println!("ok: wait without retained pause");

	let printed = cli.run(
		&["wait", "pause", "--break", "/proc/value", "--if", "rounds.len > 0", "--timeout", "1"],
		2
	);
	assert_eq!(printed.matches("reported once").count(), 1, "unexpected wait --if output: {}", printed);
	assert!(
		printed.contains("failed; skipping affected hits (reported once): runtime error: Cannot read ") && printed.contains("timed out"),
		"unexpected wait --if output: {}",
		printed
	);
	println!("ok: wait condition error reported once");

	let printed = cli.run(&["wait", "pause", "--change", "rounds", "--timeout", "15"], 0);
	assert!(
		printed.starts_with("watching rounds, now ")
			&& printed.contains(&format!("\npid {}: paused after a change: rounds changed from ", pid))
			&& printed.contains("\n#0 /run_until_stopped"),
		"unexpected wait --change output: {}",
		printed
	);
	let printed = cli.run(&["wait", "pause", "--change", "said", "--if", "value > old", "--timeout", "15"], 0);
	assert!(
		printed.starts_with("watching said, now ")
			&& printed.contains(&format!("\npid {}: paused after a change: said changed from ", pid))
			&& printed.contains("\n#0 /run_until_stopped"),
		"wait --change from a pause said {}",
		printed
	);
	let printed = cli.run(&["wait", "pause", "--change", "1 + 1"], 1);
	assert!(
		printed.starts_with("error: cannot watch 1 + 1: "),
		"unexpected wait --change output: {}",
		printed
	);
	cli.run(&["continue"], 0);
	println!("ok: CLI global and frame watches");

	for (args, said) in [
		(&["frobnicate"][..], "error: unknown command 'frobnicate'."),
		(
			&["vars", "--frame", "top"][..],
			"error: invalid value 'top' for --frame: expected a non-negative integer."
		),
		(
			&["wait", "pause", "--break", "somewhere"][..],
			"error: invalid location 'somewhere': expected FILE:LINE or /proc/path[#N]."
		),
		(&["wait", "now"][..], "error: unknown wait command 'now'."),
		(
			&["wait", "pause", "--break", "/proc/value", "--log", "hit"][..],
			"error: unexpected argument '--log'."
		),
		(
			&["wait", "pause", "--break", "/proc/value", "--if", "1 =="][..],
			"error: cannot set a breakpoint at /proc/value: the condition doesn't compile: parser error: "
		)
	] {
		let printed = cli.run(args, 1);
		assert!(printed.starts_with(said), "unexpected output from `{}`: {}", args.join(" "), printed);
	}
	let (code, printed) = launcher_command(&["--pid", "4000000000", "status"]);
	assert_eq!(code, 1);
	assert!(
		printed.starts_with("error: cannot connect to process 4000000000: no debug server is listening in it: "),
		"status said {}",
		printed
	);
	assert_eq!(
		cli.run(&["status"], 0).lines().next().unwrap_or_default(),
		format!("pid {}: running", pid)
	);
	println!("ok: invalid command diagnostics");
}

fn command_line_trace(pid: u64, code: &Path) {
	let pid = pid.to_string();
	let cli = Cli::new(&pid);
	let value = line_of(code, "\treturn 2");
	let called = line_of(code, "\t\tworld.log << \"attach test: [value()]\"");
	let every = line_of(code, "\ttick(1)");
	let other = line_of(code, "\t\ttick(2)");

	let printed = cli.run(&["trace", "/proc/value", "--count", "3"], 0);
	let lines: Vec<&str> = printed.lines().collect();
	let hit = format!(
		"/value  attach_host.dm:{}, called by /run_until_stopped  attach_host.dm:{}",
		value, called
	);
	let summary = "summary: 3 hits, 0 skipped; ended: count".to_owned();
	assert_eq!(
		lines,
		[
			format!("tracing /value  attach_host.dm:{}", value),
			hit.clone(),
			hit.clone(),
			hit,
			summary
		],
		"unexpected trace output: {}",
		printed
	);
	assert_eq!(
		cli.run(&["status"], 0).lines().next().unwrap_or_default(),
		format!("pid {}: running", pid)
	);
	println!("ok: trace locations, callers and count limit");

	let printed = cli.run(
		&[
			"trace",
			&format!("attach_host.dm:{}", value),
			"--log",
			"rounds then: {rounds > 0}",
			"--count",
			"2"
		],
		0
	);
	assert_eq!(
		printed.lines().skip(1).collect::<Vec<_>>(),
		["rounds then: 1", "rounds then: 1", "summary: 2 hits, 0 skipped; ended: count"],
		"unexpected trace --log output: {}",
		printed
	);
	println!("ok: trace message interpolation");

	let started = Instant::now();
	let printed = cli.run(&["trace", "/proc/tick", "--by", "caller", "--for", "1.5"], 0);
	assert!(started.elapsed() >= Duration::from_millis(1500), "trace --for 1.5 ended early");
	let lines: Vec<&str> = printed.lines().collect();
	let count = |line: &str| line.split_whitespace().next().unwrap().parse::<u32>().unwrap();
	assert!(
		lines.len() == 4
			&& lines[1].ends_with(&format!("  /tally  attach_host.dm:{}", every))
			&& lines[2].ends_with(&format!("  /tally  attach_host.dm:{}", other))
			&& count(lines[1]) >= count(lines[2])
			&& count(lines[2]) >= 2
			&& lines[3] == format!("summary: {} hits, 0 skipped; ended: time", count(lines[1]) + count(lines[2])),
		"trace --by caller said {}",
		printed
	);
	let printed = cli.run(&["trace", "/proc/tick", "--by", "n", "--count", "6"], 0);
	let lines: Vec<&str> = printed.lines().collect();
	assert!(
		lines.len() == 4
			&& lines[1].ends_with("  1")
			&& lines[2].ends_with("  2")
			&& count(lines[1]) == 6 - count(lines[2])
			&& lines[3] == "summary: 6 hits, 0 skipped; ended: count",
		"trace --by n said {}",
		printed
	);
	println!("ok: trace grouping and limits");

	let printed = cli.run(&["trace", "/proc/tick", "--if", "n.len > 0", "--for", "1"], 0);
	assert_eq!(printed.matches("reported once").count(), 1, "unexpected trace --if output: {}", printed);
	assert!(
		printed.contains("failed; skipping affected hits (reported once): runtime error: Cannot read ")
			&& printed
				.lines()
				.any(|line| line.starts_with("summary: 0 hits, ") && !line.contains(", 0 skipped") && line.ends_with(" skipped; ended: time"))
			&& !printed.contains("called by"),
		"unexpected trace --if output: {}",
		printed
	);
	println!("ok: trace condition errors and skipped-hit count");

	let printed = cli.run(&["trace", "/proc/value", "--json", "--count", "1"], 0);
	let events: Vec<serde_json::Value> = printed.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
	assert!(
		events.len() == 3
			&& events[0]["event"] == "bound"
			&& events[0]["line"] == value
			&& events[1]["event"] == "hit"
			&& events[1]["caller"]["proc"] == "/run_until_stopped"
			&& events[1]["caller"]["line"] == called
			&& events[2]["event"] == "summary"
			&& events[2]["hits"] == 1
			&& events[2]["ended"] == "count",
		"trace --json said {}",
		printed
	);
	println!("ok: trace JSON Lines");

	let mut tracing = cli
		.command(&["trace", "/proc/value"])
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	let mut lines = BufReader::new(tracing.stdout.take().unwrap()).lines();
	assert!(lines.next().unwrap().unwrap().starts_with("tracing "));
	assert!(lines.next().unwrap().unwrap().starts_with("/value "));
	drop(lines);
	let traced = tracing.wait_with_output().unwrap();
	let err = String::from_utf8_lossy(&traced.stderr);
	assert!(
		err.is_empty() && traced.status.code().unwrap_or(0) == 0,
		"a trace into a closed pipe said {} ({})",
		err,
		traced.status
	);
	println!("ok: trace handles closed output pipe");

	let mut evaluating = cli
		.command(&["eval", "jointext(new/list(100000), \"xxxxxxxx\")"])
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	let mut out = evaluating.stdout.take().unwrap();
	let mut start = [0; 9];
	out.read_exact(&mut start).unwrap();
	assert_eq!(&start, b"\"xxxxxxxx");
	drop(out);
	let evaluated = evaluating.wait_with_output().unwrap();
	let err = String::from_utf8_lossy(&evaluated.stderr);
	assert!(
		err.is_empty() && evaluated.status.code().unwrap_or(0) == 0,
		"eval failed after stdout closed: {} ({})",
		err,
		evaluated.status
	);
	println!("ok: eval handles closed output pipe");

	#[cfg(unix)]
	{
		let mut tracing = Background::start(pid.parse().unwrap(), &["trace", "/proc/tick", "--by", "n"]);
		tracing.line_with("tracing /tick");
		journal::rounds_pass(pid.parse().unwrap(), 2);
		Command::new("kill").args(["-INT", &tracing.id().to_string()]).status().unwrap();
		let (code, out) = tracing.finish();
		assert!(
			code == 0 && out.contains(" skipped; ended: interrupted\n") && out.contains("  1\n"),
			"an interrupted trace said {} (exit {})",
			out,
			code
		);
		println!("ok: interrupted trace summary");
	}

	for (args, said) in [
		(
			&["trace", "nowhere.dm:3"][..],
			"error: cannot set a breakpoint at nowhere.dm:3: the world has no code from a file named nowhere.dm."
		),
		(
			&["trace", "/proc/value", "/proc/nothing_called_this"][..],
			"error: cannot set a breakpoint at /proc/nothing_called_this: "
		),
		(
			&["trace", "/proc/value", "--if", "1 =="][..],
			"error: cannot set a breakpoint at /proc/value: the condition doesn't compile: "
		),
		(
			&["trace", "/proc/value", "--for=-1"][..],
			"error: invalid value '-1' for --for: expected a non-negative number of seconds."
		)
	] {
		let printed = cli.run(args, 1);
		assert!(printed.contains(said), "unexpected output from `{}`: {}", args.join(" "), printed);
	}
	assert_eq!(
		cli.run(&["status"], 0).lines().next().unwrap_or_default(),
		format!("pid {}: running", pid)
	);
	let printed = cli.run(&["trace", "/proc/value", "--count", "1"], 0);
	assert_eq!(printed.lines().count(), 3, "unexpected trace output: {}", printed);
	println!("ok: invalid trace target diagnostics");
}

fn edited_after_build(server: &str) {
	let (dir, dme) = attach_host_copy("later");
	let dmb = dme.with_extension("dmb");
	let code = dir.join("attach_host.dm");
	let source = std::fs::read_to_string(&code).unwrap();
	let edit = |text: String| {
		std::fs::write(&code, text).unwrap();
		let later = std::fs::metadata(&dmb).unwrap().modified().unwrap() + Duration::from_secs(2);
		std::fs::File::options().write(true).open(&code).unwrap().set_modified(later).unwrap();
	};
	edit(format!("{}\n/proc/edited()\n\treturn 1\n", source));
	let mut started = Started::default();
	let args = launch_args(&dmb, server, &["--detach", "--json"]);
	let (got, printed) = launcher_command(&args.iter().map(String::as_str).collect::<Vec<_>>());
	assert_eq!(got, 0, "unexpected launch --detach output: {}", printed);
	let up: serde_json::Value = serde_json::from_str(&printed).unwrap_or_else(|_| panic!("unexpected launch --detach --json output: {}", printed));
	let (pid, launcher) = (up["pid"].as_u64().unwrap(), up["launcher"].as_u64().unwrap());
	started.pids.extend([pid, launcher]);
	let log = PathBuf::from(up["log"].as_str().unwrap());
	let read_log = || std::fs::read_to_string(&log).unwrap_or_default();
	let said = "Hot reload: files newer than the .dmb may have unapplied changes: attach_host.dm";
	assert!(read_log().contains(said), "launch said:\n{}", read_log());

	paths::compile(&dme);
	edit(format!("{}\n/proc/edited()\n\treturn 1\n\n/proc/edited_again()\n\treturn 2\n", source));
	wait_for_text("the launcher's log", "Hot reload: added /edited_again", read_log);
	let pid = pid.to_string();
	let (got, printed) = launcher_command(&["--pid", &pid, "reboot", "--timeout", "15"]);
	assert_eq!(got, 0, "reboot said {}", printed);
	let until = Instant::now() + Duration::from_secs(20);
	while read_log().matches(said).count() < 2 {
		assert!(Instant::now() < until, "the launcher said:\n{}", read_log());
		std::thread::sleep(Duration::from_millis(100));
	}
	let (got, printed) = launcher_command(&["--pid", &pid, "stop"]);
	assert_eq!(got, 0, "stop said {}", printed);
	drop(started);
	println!("ok: code changed after the .dmb was built");
}

#[track_caller]
fn line_of(file: &Path, text: &str) -> u64 {
	let source = std::fs::read_to_string(file).unwrap();
	source.lines().position(|line| line.starts_with(text)).expect("no such line") as u64 + 1
}

struct Attached {
	runner: Launcher,
	pid: u64,
	dir: TempDir,
	code: PathBuf,
	checks: bool
}

fn attach_and_reload(server: &str, library: &Path, checks: bool) -> Attached {
	let (dir, dme) = attach_host_copy("attach");
	let code = dir.join("attach_host.dm");

	let mut runner = Launcher::start();
	runner.initialize();
	runner.request(
		"launch",
		json!({
			"noDebug": true,
			"dme": dme,
			"byondPath": paths::find_byond(),
			"server": server,
			"debugLibrary": library,
			"args": test_args()
		})
	);
	runner.started();
	runner.request("configurationDone", json!({}));
	runner.wait_output("attach test: 1");
	runner.wait_output("Hot reload active");
	println!("ok: noDebug launch with source watching");

	let pid = listed_world("attach_host.dmb");
	println!("ok: world discovered with PID {}", pid);
	if !checks {
		let source = std::fs::read_to_string(&code).unwrap();
		std::fs::write(&code, source.replace("\treturn 1", "\treturn 2")).unwrap();
		runner.wait_output("Hot reload: reloaded /value");
		runner.wait_output("attach test: 2");
		return Attached {
			runner,
			pid,
			dir,
			code,
			checks
		};
	}

	let mut first = Launcher::start();
	first.initialize();
	first.request("attach", json!({ "processId": pid.to_string() }));
	first.event("initialized");
	first.request("configurationDone", json!({}));
	let mut second = Launcher::start();
	second.initialize();
	second.request("attach", json!({ "processId": pid, "dme": dme, "hotReload": true }));
	second.event("initialized");
	second.request("configurationDone", json!({}));
	second.wait_output("Hot reload disabled: the launcher (pid ");
	assert!(
		second.output.contains("already owns hot reload for this world"),
		"unexpected second-session output: {}",
		second.output
	);
	println!("ok: one reloader per world");

	let line = line_of(&code, "\treturn 1");
	let set = first.request("setBreakpoints", json!({ "source": { "path": code }, "breakpoints": [{ "line": line }] }));
	assert_eq!(set["breakpoints"][0]["verified"], true, "the breakpoint wasn't set: {}", set);
	first.stopped("breakpoint");
	second.stopped("breakpoint");
	let frame = second.top_frame();
	assert!(frame["name"].as_str().unwrap().ends_with("value"), "stopped in {}", frame);
	first.request("setBreakpoints", json!({ "source": { "path": code }, "breakpoints": [] }));
	second.resume();
	first.event("continued");
	println!("ok: concurrent launcher sessions");

	let source = std::fs::read_to_string(&code).unwrap();
	std::fs::write(&code, source.replace("\treturn 1", "\treturn 2")).unwrap();
	runner.wait_output("Hot reload: reloaded /value");
	runner.wait_output("attach test: 2");
	println!("ok: source reload");

	first.disconnect(json!({}));
	second.disconnect(json!({ "terminateDebuggee": false }));
	let seen = runner.output.matches("attach test: 2").count();
	runner.wait_count("attach test: 2", seen + 2);
	println!("ok: attached-session disconnect preserves world");
	Attached {
		runner,
		pid,
		dir,
		code,
		checks
	}
}

impl Attached {
	fn finish(self) {
		let Attached {
			mut runner,
			pid,
			dir,
			checks,
			..
		} = self;
		if !checks {
			std::fs::write(dir.join("stop"), "yes").unwrap();
			runner.event("terminated");
			runner.disconnect(json!({}));
			return;
		}
		let (code_given, printed) = launcher_command(&["--pid", &pid.to_string(), "pause"]);
		assert_eq!(code_given, 0, "unexpected pause output: {}", printed);
		let mut late = Launcher::start();
		late.initialize();
		late.request("attach", json!({ "processId": pid.to_string() }));
		late.event("initialized");
		late.request("configurationDone", json!({}));
		late.stopped("pause");
		late.resume();
		late.disconnect(json!({}));
		println!("ok: attach to a retained pause");
		let seen = runner.output.matches("attach test: 2").count();
		runner.wait_count("attach test: 2", seen + 1);
		println!("ok: CLI resume");

		let waiting = journal::wait_for_reboot(pid);
		let mut tracing = Background::start(pid, &["trace", "/proc/value", "--by", "caller"]);
		tracing.line_with("tracing /value");
		let seen = runner.output.matches("attach test: 2").count();
		runner.wait_count("attach test: 2", seen + 2);
		std::fs::write(dir.join("stop"), "yes").unwrap();
		let (code, printed) = tracing.finish();
		assert!(
			code == 3 && printed.contains("; ended: world\n") && printed.contains("\nerror: world connection ended before the operation completed"),
			"unexpected trace output ({}): {}",
			code,
			printed
		);
		println!("ok: world exit ends trace with code 3");
		runner.event("terminated");
		journal::world_went_away(waiting);
		assert!(runner.output.contains("attach test: done"), "world did not complete the test");
		runner.disconnect(json!({}));
		println!("ok: world and launcher exit");
	}
}

fn attach_host_copy(name: &str) -> (TempDir, PathBuf) {
	let dir = TempDir::new(&format!("launcher_test_{}", name));
	for left in [
		"stop",
		"fail",
		"reboot",
		"call",
		"refuse_del",
		"fail_del",
		"slow_del",
		"slow_reboot",
		"refuse_reboot"
	] {
		let _ = std::fs::remove_file(dir.join(left));
		assert!(
			!dir.join(left).exists(),
			"{} is left from another run and can't be removed",
			dir.join(left).display()
		);
	}
	for file in ["attach_host.dme", "attach_host.dm"] {
		std::fs::copy(host_file(file), dir.join(file)).unwrap();
	}
	let dme = dir.join("attach_host.dme");
	paths::compile(&dme);
	(dir, dme)
}

#[derive(Default)]
struct Started {
	child: Option<Child>,
	pids: Vec<u64>
}

impl Drop for Started {
	fn drop(&mut self) {
		if let Some(child) = &mut self.child {
			let _ = child.kill();
			let _ = child.wait();
		}
		if std::thread::panicking() {
			let list = auxtools(["list", "--json"]).output().ok();
			let worlds: serde_json::Value = list.and_then(|list| serde_json::from_slice(&list.stdout).ok()).unwrap_or_default();
			let ours = format!("launcher_test_launch-{}", std::process::id());
			for world in worlds.as_array().into_iter().flatten() {
				if world["dmb"].as_str().is_some_and(|dmb| dmb.contains(&ours)) {
					self.pids.extend(world["pid"].as_u64());
				}
			}
		}
		for pid in &self.pids {
			process::kill(*pid);
		}
	}
}

fn alive(pid: u64) -> bool {
	if cfg!(windows) {
		let listed = Command::new("tasklist")
			.args(["/FI", &format!("PID eq {}", pid), "/NH"])
			.output()
			.unwrap();
		String::from_utf8_lossy(&listed.stdout)
			.split_whitespace()
			.any(|word| word == pid.to_string())
	} else {
		std::fs::read_to_string(format!("/proc/{}/stat", pid))
			.is_ok_and(|stat| stat.rsplit_once(") ").is_some_and(|(_, rest)| !rest.starts_with('Z')))
	}
}

fn gone_within(pid: u64, seconds: u64) -> bool {
	let until = Instant::now() + Duration::from_secs(seconds);
	while alive(pid) {
		if Instant::now() > until {
			return false;
		}
		std::thread::sleep(Duration::from_millis(100));
	}
	true
}

#[track_caller]
fn wait_for_text(what: &str, text: &str, read: impl Fn() -> String) {
	let until = Instant::now() + Duration::from_secs(60);
	loop {
		let now = read();
		if now.contains(text) {
			return;
		}
		assert!(Instant::now() < until, "{} didn't say {:?}:\n{}", what, text, now);
		std::thread::sleep(Duration::from_millis(100));
	}
}

fn launch_args(dmb: &Path, server: &str, more: &[&str]) -> Vec<String> {
	let mut args: Vec<String> = vec!["launch".into(), dmb.to_string_lossy().into_owned()];
	args.extend([
		"--byond".into(),
		paths::find_byond().to_string_lossy().into_owned(),
		"--server".into(),
		server.to_owned()
	]);
	args.extend(more.iter().map(|arg| (*arg).to_owned()));
	if let Some(theirs) = test_args() {
		args.push("--".into());
		args.extend(theirs);
	}
	args
}

fn launch_here(dmb: &Path, server: &str, started: &mut Started) -> Arc<Mutex<String>> {
	let mut child = auxtools(launch_args(dmb, server, &[]))
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	let output = Arc::new(Mutex::new(String::new()));
	for stream in [
		Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
		Box::new(child.stderr.take().unwrap())
	] {
		let output = output.clone();
		std::thread::spawn(move || {
			for line in BufReader::new(stream).lines() {
				let Ok(line) = line else { return };
				let mut output = output.lock().unwrap();
				output.push_str(&line);
				output.push('\n');
			}
		});
	}
	started.child = Some(child);
	output
}

#[track_caller]
fn launcher_exit(started: &mut Started, output: &Mutex<String>) -> i32 {
	let child = started.child.as_mut().unwrap();
	let until = Instant::now() + Duration::from_secs(20);
	loop {
		if let Some(status) = child.try_wait().unwrap() {
			return status.code().unwrap_or(-1);
		}
		assert!(Instant::now() < until, "timed out waiting for launcher exit:\n{}", output.lock().unwrap());
		std::thread::sleep(Duration::from_millis(100));
	}
}

#[track_caller]
fn started_pid(output: &str) -> u64 {
	let pid = |line: &str| line.strip_prefix("Started ")?.rsplit_once("(process ")?.1.strip_suffix(')')?.parse().ok();
	match output.lines().find_map(pid) {
		Some(pid) => pid,
		None => panic!("launch output missing process ID:\n{}", output)
	}
}

fn errored_baseline(server: &str) {
	let (dir, dme) = attach_host_copy("errored");
	let code = dir.join("attach_host.dm");
	let source = std::fs::read_to_string(&code).unwrap();
	std::fs::write(&code, source.replace("\treturn 1", "\treturn (1")).unwrap();
	let mut started = Started::default();
	let output = launch_here(&dme.with_extension("dmb"), server, &mut started);
	let read = || output.lock().unwrap().clone();
	wait_for_text("launch", "Hot reload active", read);
	started.pids.push(started_pid(&read()));
	assert!(
		read().contains("The source has parse errors"),
		"launch didn't say the code has errors:\n{}",
		read()
	);
	std::fs::write(&code, &source).unwrap();
	wait_for_text("launch", "fixed parse error", read);
	let printed = read();
	assert!(
		printed.contains("Hot reload requires a rebuild") && !printed.contains("Hot reload: added"),
		"launch said:\n{}",
		printed
	);
	std::fs::write(dir.join("stop"), "yes").unwrap();
	let exit = launcher_exit(&mut started, &output);
	assert_eq!(exit, 0, "launch exited with {}:\n{}", exit, read());
	drop(started);
	println!("ok: repaired baseline errors do not duplicate definitions");
}

fn dmb_elsewhere(server: &str) {
	let (dir, dme) = attach_host_copy("elsewhere");
	let built = dme.with_extension("dmb");
	let dmb = dir.join("build").join("attach_host.dmb");
	std::fs::create_dir_all(dmb.parent().unwrap()).unwrap();
	std::fs::rename(&built, &dmb).unwrap();
	let mut started = Started::default();
	let args = launch_args(&dmb, server, &["--detach", "--json", "--dme", dme.to_str().unwrap()]);
	let (got, printed) = launcher_command(&args.iter().map(String::as_str).collect::<Vec<_>>());
	assert_eq!(got, 0, "unexpected launch --detach output: {}", printed);
	let up: serde_json::Value = serde_json::from_str(&printed).unwrap_or_else(|_| panic!("unexpected launch --detach --json output: {}", printed));
	let (pid, launcher) = (up["pid"].as_u64().unwrap(), up["launcher"].as_u64().unwrap());
	started.pids.extend([pid, launcher]);
	let pid = pid.to_string();
	let cli = Cli::new(&pid);
	let run = |args: &[&str]| cli.ran(args, None).printed();

	let code = dir.join("attach_host.dm");
	let source = std::fs::read_to_string(&code).unwrap();
	std::fs::write(&code, format!("{}\n/proc/added_here()\n\treturn 99\n", source)).unwrap();
	wait_for_text("the world", "99", || run(&["eval", "added_here()"]));
	paths::compile(&dme);
	std::fs::rename(&built, &dmb).unwrap();
	assert_eq!(run(&["reboot", "--timeout", "15"]).trim(), format!("pid {}: rebooted", pid));
	let printed = run(&["proc", "/added_here"]);
	assert!(printed.contains("definitions: 1\n"), "proc said {}", printed);
	assert_eq!(run(&["stop"]).trim(), format!("pid {}: stopped", pid));
	drop(started);
	println!("ok: a rebuilt .dmb that isn't beside its .dme");
}

fn foreground(server: &str) {
	let (dir, dme) = attach_host_copy("launch");
	let dmb = dme.with_extension("dmb");
	let code = dir.join("attach_host.dm");

	let missing = dir.join("nothing.dmb");
	for more in [&[][..], &["--detach"][..]] {
		let args = launch_args(&missing, server, more);
		let (got, printed) = launcher_command(&args.iter().map(String::as_str).collect::<Vec<_>>());
		assert_eq!(got, 1, "launch of a world that isn't there gave {}: {}", got, printed);
		assert!(
			printed.contains("nothing.dmb does not exist. Compile the world first"),
			"launch said {}",
			printed
		);
	}
	println!("ok: missing .dmb launch error");

	let source = std::fs::read_to_string(&code).unwrap();
	let unbuilt = "\n/datum/held/var/unbuilt = 1\n\n/datum/held/proc/unbuilt_proc()\n\treturn src\n";
	std::fs::write(&code, format!("{}{}", source, unbuilt)).unwrap();
	let mut started = Started::default();
	let output = launch_here(&dmb, server, &mut started);
	let read = || output.lock().unwrap().clone();
	wait_for_text("launch", "Hot reload active", read);
	wait_for_text("launch", "attach test: 1", read);
	started.pids.push(started_pid(&read()));
	std::fs::write(&code, source.replace("\treturn 1", "\treturn 2")).unwrap();
	wait_for_text("launch", "Hot reload: reloaded /value", read);
	wait_for_text("launch", "attach test: 2", read);
	wait_for_text("launch", "var unbuilt was removed from source and isn't in the running world", read);
	let printed = read();
	assert!(
		printed.contains("Hot reload: reloaded /value in ")
			&& !printed.contains("unlinked /")
			&& printed.contains("unbuilt_proc: not found in the running world; skipping removal")
			&& !printed.contains("retained in the running world"),
		"launch said:\n{}",
		printed
	);
	println!("ok: removing code the world never had");
	std::fs::write(dir.join("stop"), "yes").unwrap();
	let exit = launcher_exit(&mut started, &output);
	let printed = read();
	assert_eq!(exit, 0, "launch exited with {}:\n{}", exit, printed);
	assert!(
		printed.contains("attach test: done") && printed.contains("The world exited with code 0"),
		"launch said:\n{}",
		printed
	);
	assert!(
		!printed.contains("Debug server disconnected") && !printed.contains("waiting to reconnect"),
		"launch took the world's end for a lost connection:\n{}",
		printed
	);
	std::fs::remove_file(dir.join("stop")).unwrap();
	std::fs::write(&code, source).unwrap();
	drop(started);
	println!("ok: foreground output and reloads");

	if cfg!(unix) {
		let mut started = Started::default();
		let output = launch_here(&dmb, server, &mut started);
		let read = || output.lock().unwrap().clone();
		wait_for_text("launch", "Hot reload active", read);
		started.pids.push(started_pid(&read()));
		let launcher = started.child.as_ref().unwrap().id().to_string();
		assert!(Command::new("kill").args(["-TERM", &launcher]).status().unwrap().success());
		let exit = launcher_exit(&mut started, &output);
		let printed = read();
		assert_eq!(exit, 0, "launch exited with {}:\n{}", exit, printed);
		assert!(
			printed.contains("Interrupted: requesting graceful shutdown") && printed.contains("attach test: world/Del"),
			"launch said:\n{}",
			printed
		);
		println!("ok: interrupt requests graceful shutdown");
	}
}

fn detached(server: &str) {
	let (dir, dme) = attach_host_copy("launch");
	let dmb = dme.with_extension("dmb");
	let code = dir.join("attach_host.dm");

	let mut started = Started::default();
	let args = launch_args(&dmb, server, &["--detach", "--json"]);
	let (got, printed) = launcher_command(&args.iter().map(String::as_str).collect::<Vec<_>>());
	assert_eq!(got, 0, "unexpected launch --detach output: {}", printed);
	let up: serde_json::Value = serde_json::from_str(&printed).unwrap_or_else(|_| panic!("unexpected launch --detach --json output: {}", printed));
	let (pid, launcher) = (up["pid"].as_u64().unwrap(), up["launcher"].as_u64().unwrap());
	started.pids.extend([pid, launcher]);
	let log = PathBuf::from(up["log"].as_str().unwrap());
	assert_eq!(log, dir.join("attach_host.auxtools.log"));
	let read_log = || std::fs::read_to_string(&log).unwrap_or_default();
	assert!(
		read_log().contains("Hot reload active"),
		"the world was up before its code was watched:\n{}",
		read_log()
	);
	let listed = listed_worlds();
	assert!(
		listed
			.iter()
			.any(|world| world["pid"] == pid && world["dmb"].as_str().is_some_and(|dmb| dmb.contains("launcher_test_launch-"))),
		"list doesn't show the world: {:?}",
		listed
	);
	let pid_text = pid.to_string();
	let cli = Cli::new(&pid_text);
	assert_eq!(
		cli.run(&["status"], 0).lines().next().unwrap_or_default(),
		format!("pid {}: running", pid)
	);
	assert_eq!(cli.run(&["eval", "6", "*", "7"], 0).trim(), "42");
	wait_for_text("the background launcher's log", "attach test: 1", read_log);
	println!("ok: detached launch readiness and commands");

	#[cfg(unix)]
	{
		let mine = |listed: Vec<serde_json::Value>| listed.into_iter().find(|world| world["pid"] == pid);
		let listed = mine(listed_worlds()).unwrap_or_else(|| panic!("list doesn't show the world"));
		assert_eq!(listed["responsive"], json!(true), "{}", listed);
		assert!(Command::new("kill").args(["-STOP", &pid_text]).status().unwrap().success());
		let asked: Vec<_> = [
			&["list"][..],
			&["list", "--json"],
			&["--pid", &pid_text, "status", "--response-timeout", "1"],
			&["--pid", &pid_text, "--response-timeout", "1.5", "pause"]
		]
		.iter()
		.map(|args| {
			std::thread::spawn({
				let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
				move || {
					let ran = ran(auxtools(&args).env("AUXTOOLS_TEST_PATIENCE", "1000"), None);
					(ran.code, ran.printed())
				}
			})
		})
		.collect();
		let mut answers = asked.into_iter().map(|asking| asking.join().unwrap());
		let (code, table) = answers.next().unwrap();
		let stopped = answers.next().unwrap().1;
		let (status, paused) = (answers.next().unwrap(), answers.next().unwrap());
		assert!(Command::new("kill").args(["-CONT", &pid_text]).status().unwrap().success());
		let worlds: serde_json::Value = serde_json::from_str(&stopped).unwrap_or_else(|_| panic!("list --json said {}", stopped));
		let listed = worlds.as_array().unwrap().iter().find(|world| world["pid"] == pid);
		assert!(
			listed.is_some_and(|world| world["responsive"] == false && world["compatible"] == true && world["dmb"].is_null()),
			"list --json said {}",
			stopped
		);
		let row = table.lines().find(|line| line.split_whitespace().next() == Some(pid_text.as_str()));
		assert!(
			code == 0 && row.is_some_and(|row| row.split_whitespace().nth(4) == Some("no")),
			"list said {}",
			table
		);
		println!("ok: list shows unresponsive worlds");
		for ((code, printed), seconds) in [(status, "1 second"), (paused, "1.5 seconds")] {
			assert_eq!(
				(code, printed.as_str()),
				(
					2,
					format!(
						"error: timed out after {} waiting for the world to respond; it is busy or hung. Set --response-timeout to wait longer.\n",
						seconds
					)
					.as_str()
				),
				"a command to a stopped world"
			);
		}
		println!("ok: response timeouts for stalled worlds");
	}

	let printed = cli.run(&["reboot"], 0);
	assert_eq!(printed.trim(), format!("pid {}: rebooted", pid));
	assert_eq!(
		cli.run(&["status"], 0).lines().next().unwrap_or_default(),
		format!("pid {}: running", pid)
	);
	let again: serde_json::Value = serde_json::from_str(&cli.run(&["reboot", "--json", "--timeout", "15"], 0)).unwrap();
	assert_eq!(again, json!({ "pid": pid, "rebooted": true }));
	wait_for_text("the background launcher's log", "The world restarted", read_log);
	println!("ok: reboot waits for reconnection");

	let source = std::fs::read_to_string(&code).unwrap();
	std::fs::write(&code, format!("{}\n/proc/added_by_reload()\n\treturn 99\n", source)).unwrap();
	wait_for_text("the background launcher's log", "Hot reload: added /added_by_reload", read_log);
	cli.run(&["reboot", "--timeout", "15"], 0);
	assert_eq!(cli.run(&["eval", "added_by_reload()"], 0).trim(), "99");
	let loaded: serde_json::Value = serde_json::from_str(&cli.run(&["log", "--world", "--last", "1", "--json"], 0)).unwrap();
	let status: serde_json::Value = serde_json::from_str(&cli.run(&["status", "--json"], 0)).unwrap();
	assert!(
		loaded["event"] == "loaded"
			&& status["last_reload"]["applied"] == true
			&& status["last_reload"]["number"].as_u64() > loaded["number"].as_u64(),
		"after reboot, the world loaded at {} and the last reload is {}",
		loaded,
		status["last_reload"]
	);
	println!("ok: reboot waits for hot reloads to be reapplied");

	#[cfg(unix)]
	{
		let rebaselined = || read_log().matches("Hot reload: using the rebuilt .dmb").count();
		let before = rebaselined();
		let launcher_text = launcher.to_string();
		let loads = || {
			let (_, printed) = launcher_command(&["--pid", &pid_text, "log", "--world", "--json"]);
			let entries = printed.lines().filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok());
			entries.filter(|entry| entry["event"] == "loaded").count()
		};
		let loaded = loads();
		assert!(Command::new("kill").args(["-STOP", &launcher_text]).status().unwrap().success());
		let edited = std::fs::read_to_string(&code).unwrap();
		std::fs::write(&code, format!("{}\n/proc/zz_agent_probe()\n\treturn 7\n", edited)).unwrap();
		let built = paths::dreammaker().arg(&dme).output().unwrap();
		std::fs::write(dir.join("reboot"), "yes").unwrap();
		let until = Instant::now() + Duration::from_secs(20);
		while loads() == loaded && Instant::now() < until {
			std::thread::sleep(Duration::from_millis(100));
		}
		let rebooted = loads() > loaded;
		assert!(Command::new("kill").args(["-CONT", &launcher_text]).status().unwrap().success());
		assert!(rebooted, "the world didn't reboot:\n{}", read_log());
		assert!(built.status.success(), "dreammaker build failed");
		let started = Instant::now();
		while rebaselined() == before {
			assert!(
				started.elapsed() < Duration::from_secs(20),
				"the launcher did not adopt the rebuilt .dmb:\n{}",
				read_log()
			);
			std::thread::sleep(Duration::from_millis(100));
		}
		assert_eq!(cli.run(&["eval", "zz_agent_probe()"], 0).trim(), "7");
		let printed = cli.run(&["proc", "/zz_agent_probe"], 0);
		assert!(
			printed.contains("definitions: 1\n") && printed.contains("reloaded: no\n"),
			"proc already included in the rebuilt .dmb was reloaded: {}\nlauncher log:\n{}",
			printed,
			read_log()
		);
		println!("ok: rebuilt worlds skip reloads already included in the build");
	}
	drop(started);
}

fn reboots(server: &str) {
	let (dir, dme) = attach_host_copy("launch");
	let dmb = dme.with_extension("dmb");
	let mut started = Started::default();
	let args = launch_args(&dmb, server, &["--detach", "--json"]);
	let (got, printed) = launcher_command(&args.iter().map(String::as_str).collect::<Vec<_>>());
	assert_eq!(got, 0, "unexpected launch --detach output: {}", printed);
	let up: serde_json::Value = serde_json::from_str(&printed).unwrap_or_else(|_| panic!("unexpected launch --detach --json output: {}", printed));
	let (pid, launcher) = (up["pid"].as_u64().unwrap(), up["launcher"].as_u64().unwrap());
	started.pids.extend([pid, launcher]);
	let log = PathBuf::from(up["log"].as_str().unwrap());
	let read_log = || std::fs::read_to_string(&log).unwrap_or_default();
	let pid_text = pid.to_string();
	let cli = Cli::new(&pid_text);

	for moment in 0..8u64 {
		std::thread::sleep(Duration::from_millis(moment * 13));
		cli.run(&["reboot", "--timeout", "15"], 0);
	}
	assert_eq!(
		cli.run(&["status"], 0).lines().next().unwrap_or_default(),
		format!("pid {}: running", pid)
	);
	assert_eq!(cli.run(&["eval", "6", "*", "7"], 0).trim(), "42");
	let said = read_log();
	assert!(
		!said.contains("ID mismatch") && !said.contains("panicked"),
		"the launcher's log has:\n{}",
		said
	);
	println!("ok: reboot throughout world execution");

	std::fs::write(dir.join("slow_reboot"), "yes").unwrap();
	cli.run(&["wait", "pause", "--break", "/proc/value", "--timeout", "15"], 0);
	cli.run(&["reboot", "--timeout", "15"], 0);
	std::fs::remove_file(dir.join("slow_reboot")).unwrap();
	cli.run(&["wait", "pause", "--break", "/proc/value", "--timeout", "15"], 0);
	println!("ok: paused reboot with sleeping world/Reboot");

	for (files, command, over) in [
		(["slow_reboot", "refuse_reboot"], "reboot", "attach test: reboot refused"),
		(["slow_del", "fail_del"], "stop", "attach test: world/Del failed")
	] {
		for file in files {
			std::fs::write(dir.join(file), "yes").unwrap();
		}
		cli.run(&[command, "--timeout", "1"], 2);
		wait_for_text("the background launcher's log", over, read_log);
		for file in files {
			std::fs::remove_file(dir.join(file)).unwrap();
		}
		cli.run(&["wait", "pause", "--break", "/proc/value", "--timeout", "20"], 0);
	}
	println!("ok: pauses after declined reboot and failed shutdown");

	let before = read_log().len();
	std::fs::write(dir.join("slow_del"), "yes").unwrap();
	let printed = cli.run(&["stop"], 0);
	assert_eq!(printed.trim(), format!("pid {}: stopped", pid));
	assert!(!alive(pid), "world process is still running");
	assert!(gone_within(launcher, 10), "the background launcher didn't go with the world");
	let said = read_log();
	assert!(
		said.contains("attach test: world/Del") && said.contains("The world exited with code 0"),
		"the log has:\n{}",
		said
	);
	assert!(
		!said[before..].contains("waiting to reconnect"),
		"the launcher took the world's end for a lost connection:\n{}",
		said
	);
	let _ = std::fs::remove_file(dir.join("slow_del"));
	drop(started);
	println!("ok: paused shutdown with sleeping world/Del");
}

fn stops(server: &str) {
	let (dir, dme) = attach_host_copy("launch");
	let dmb = dme.with_extension("dmb");
	let log = dir.join("attach_host.auxtools.log");

	let mut started = Started::default();
	let args = launch_args(&dmb, server, &["--detach", "--no-hot-reload"]);
	let (got, printed) = launcher_command(&args.iter().map(String::as_str).collect::<Vec<_>>());
	assert_eq!(got, 0, "unexpected launch --detach output: {}", printed);
	let lines: Vec<&str> = printed.lines().collect();
	let pid: u64 = lines
		.first()
		.and_then(|line| line.strip_prefix("pid ")?.strip_suffix(": started")?.parse().ok())
		.unwrap_or_else(|| panic!("unexpected launch --detach output: {}", printed));
	let launcher: u64 = lines
		.get(1)
		.and_then(|line| line.strip_prefix("launcher: pid ")?.parse().ok())
		.unwrap_or_else(|| panic!("unexpected launch --detach output: {}", printed));
	assert_eq!(
		lines.get(2).copied(),
		Some(format!("log: {}", dir.join("attach_host.auxtools.log").display()).as_str()),
		"unexpected launch --detach output: {}",
		printed
	);
	started.pids.extend([pid, launcher]);
	std::fs::write(dir.join("refuse_del"), "yes").unwrap();
	let pid_text = pid.to_string();
	let cli = Cli::new(&pid_text);
	let printed = cli.run(&["eval", "DOUBLE_IT(1)"], 1);
	assert!(
		printed.starts_with("error: proc not found: /proc/DOUBLE_IT (if that's a #define: source macros unavailable"),
		"unexpected eval output: {}",
		printed
	);
	println!("ok: missing source-macro diagnostic");
	std::fs::remove_file(dir.join("refuse_del")).unwrap();
	std::fs::write(dir.join("fail_del"), "yes").unwrap();
	let printed = cli.run(&["stop", "--timeout", "10"], 1);
	assert!(
		printed.starts_with(&format!(
			"error: world.Del() failed with a runtime error (see the world's log), so the world (pid {}) is still running.",
			pid
		)),
		"stop said {}",
		printed
	);
	assert!(alive(pid), "the world ended");
	let printed = cli.run(&["eval", "1 + 1"], 0);
	assert_eq!(printed.trim(), "2", "eval after the failed stop said {}", printed);
	std::fs::remove_file(dir.join("fail_del")).unwrap();
	std::fs::write(dir.join("refuse_del"), "yes").unwrap();
	println!("ok: stop reports a failing world/Del");
	let printed = cli.run(&["stop", "--timeout", "1"], 2);
	assert!(
		printed.starts_with(&format!(
			"error: timed out after 1 second waiting for the world process (pid {}) to exit.",
			pid
		)),
		"stop said {}",
		printed
	);
	assert!(alive(pid), "the world ended");
	let printed = cli.run(&["stop", "--timeout", "0", "--kill", "--kill-timeout", "10", "--json"], 0);
	let stopped: serde_json::Value =
		serde_json::from_str(printed.lines().next().unwrap_or_default()).unwrap_or_else(|_| panic!("stop --json said {}", printed));
	assert_eq!(stopped, json!({ "pid": pid, "stopped": true, "killed": true }), "stop said {}", printed);
	assert!(
		printed.contains("graceful shutdown timed out after 0 seconds; the process was terminated."),
		"stop said {}",
		printed
	);
	assert!(!alive(pid), "world process is still running");
	let ended = if cfg!(windows) {
		"The world exited with code 1"
	} else {
		"The world was terminated"
	};
	wait_for_text("the background launcher's log", ended, || {
		std::fs::read_to_string(&log).unwrap_or_default()
	});
	assert!(gone_within(launcher, 10), "the background launcher didn't go with the world");
	drop(started);
	println!("ok: stop timeout and forced termination");

	let _ = std::fs::remove_file(dir.join("refuse_del"));
}

fn without_debug_info(server: &str) {
	let (dir, dme) = attach_host_copy("nolines");
	let text = std::fs::read_to_string(&dme).unwrap().replace("#define DEBUG\n", "");
	std::fs::write(&dme, text).unwrap();
	paths::compile(&dme);
	let dmb = dme.with_extension("dmb");

	let mut started = Started::default();
	let args = launch_args(&dmb, server, &["--detach", "--no-hot-reload", "--json"]);
	let (got, printed) = launcher_command(&args.iter().map(String::as_str).collect::<Vec<_>>());
	assert_eq!(got, 0, "unexpected launch --detach output: {}", printed);
	let launched: serde_json::Value = serde_json::from_str(printed.lines().next().unwrap_or_default())
		.unwrap_or_else(|_| panic!("unexpected launch --detach --json output: {}", printed));
	let pid = launched["pid"].as_u64().unwrap();
	started.pids.extend([pid, launched["launcher"].as_u64().unwrap()]);
	let pid_text = pid.to_string();
	let line = line_of(&dir.join("attach_host.dm"), "\ttick(1)").to_string();
	let place = format!("attach_host.dm:{}", line);
	let (got, printed) = launcher_command(&["--pid", &pid_text, "wait", "pause", "--break", &place, "--timeout", "5"]);
	assert_eq!(got, 1, "a line breakpoint without debug info gave {}: {}", got, printed);
	assert!(
		printed.contains("the world was compiled without debug information"),
		"a line breakpoint without debug info said {}",
		printed
	);
	let (got, printed) = launcher_command(&["--pid", &pid_text, "stop", "--timeout", "10", "--kill"]);
	assert_eq!(got, 0, "stop gave {}: {}", got, printed);
	drop(started);
	println!("ok: line breakpoints in a world without debug info");
}

fn unread_statics(server: &str) {
	let (dir, dme) = attach_host_copy("statics");
	let code = dir.join("attach_host.dm");
	let mut source = std::fs::read_to_string(&code).unwrap();
	source.push_str(
		"\nvar/inits = 0\n\n/proc/count_init(x)\n\tinits++\n\treturn x\n\n/proc/unread_static()\n\tvar/static/kept = count_init(1)\n\treturn \
		 1\n\n// __PROC__ needs BYOND 515.1644.\n#if DM_VERSION > 515 || DM_BUILD >= 1644\n#define STATIC_NAME __PROC__\n#else\n#define STATIC_NAME \
		 2\n#endif\n\n/proc/unread_static_proc()\n\tvar/static/named = count_init(STATIC_NAME)\n\treturn 1\n"
	);
	std::fs::write(&code, &source).unwrap();
	paths::compile(&dme);
	let dmb = dme.with_extension("dmb");

	let mut started = Started::default();
	let args = launch_args(&dmb, server, &["--detach", "--json"]);
	let (got, printed) = launcher_command(&args.iter().map(String::as_str).collect::<Vec<_>>());
	assert_eq!(got, 0, "unexpected launch --detach output: {}", printed);
	let launched: serde_json::Value = serde_json::from_str(printed.lines().next().unwrap_or_default())
		.unwrap_or_else(|_| panic!("unexpected launch --detach --json output: {}", printed));
	let pid = launched["pid"].as_u64().unwrap();
	started.pids.extend([pid, launched["launcher"].as_u64().unwrap()]);
	let pid_text = pid.to_string();
	let cli = Cli::new(&pid_text);
	assert_eq!(cli.run(&["eval", "inits"], 0).trim(), "2");

	for (n, edited) in ["1 + 1", "3"].iter().enumerate() {
		let before = if n == 0 { "1".to_owned() } else { "1 + 1".to_owned() };
		source = source.replace(
			&format!("count_init(1)\n\treturn {}\n", before),
			&format!("count_init(1)\n\treturn {}\n", edited)
		);
		source = source.replace(
			&format!("count_init(STATIC_NAME)\n\treturn {}\n", before),
			&format!("count_init(STATIC_NAME)\n\treturn {}\n", edited)
		);
		let since = journal::newest(pid).to_string();
		std::fs::write(&code, &source).unwrap();
		let printed = cli.run(&["wait", "reload", "--since", &since, "--timeout", "15"], 0);
		assert!(printed.contains("hot reload applied"), "the reload said {}", printed);
		assert_eq!(cli.run(&["eval", "inits"], 0).trim(), "2", "an initializer ran again");
	}
	assert_eq!(cli.run(&["eval", "unread_static() + unread_static_proc()"], 0).trim(), "6");

	source = source.replace("count_init(1)\n\treturn 3\n", "count_init(1)\n\treturn kept + 10\n");
	let since = journal::newest(pid).to_string();
	std::fs::write(&code, &source).unwrap();
	let printed = cli.run(&["wait", "reload", "--since", &since, "--timeout", "15"], 0);
	assert!(printed.contains("hot reload applied"), "the reload said {}", printed);
	assert_eq!(cli.run(&["eval", "unread_static()"], 0).trim(), "11");
	assert_eq!(cli.run(&["eval", "inits"], 0).trim(), "2", "an initializer ran again");
	cli.run(&["stop", "--timeout", "10", "--kill"], 0);
	drop(started);
	println!("ok: reloads keep static locals the code doesn't read");
}
