//! Drives the launcher the way VS Code would, in the Debug Adapter Protocol,
//! against a world with none of auxtools' procs in it: it has to load the
//! debugger itself, stop at breakpoints, pick up again after the world
//! reboots, and go away when the world ends. Then a world launched without
//! debugging is found with `--list`, two more launchers attach to it at once,
//! and the first hot reloads it. Then the launcher's command line pauses it
//! and looks around, a command at a time. Last, the command line starts
//! worlds itself (`launch`, in the foreground and in the background), reboots
//! them and stops them.
//!
//! Build the launcher and the debug library first:
//! `cargo build -p auxtools-cli -p debug_server -p launcher_test`, then run this
//! from the repository's root with `BYOND_PATH` set. `LAUNCHER_TEST_SERVER`
//! picks the program on Windows (`dd` unless set: `dreamseeker` and
//! `dreamdaemon` need a display). `LAUNCHER_TEST_ARGS` replaces the program's
//! arguments (Windows' DreamDaemon asks before it runs a world `-trusted`).

// The debug client, which dap_test uses more of
#[allow(dead_code)]
#[path = "../../dap_test/src/client.rs"]
mod client;
#[allow(dead_code)]
#[path = "../../test_runner/src/paths.rs"]
mod paths;
mod journal;

use std::{
	io::{BufRead, BufReader, Read},
	path::{Path, PathBuf},
	process::{Child, Command, Stdio},
	sync::{Arc, Mutex},
	time::{Duration, Instant}
};

use client::{launcher_command, launcher_path, library_path, test_args, Launcher};
use paths::ByondCommand;
use serde_json::json;

/// The line `target` doubles its argument on
const TARGET_LINE: u64 = 28;

fn host_file(name: &str) -> PathBuf {
	std::env::current_dir().unwrap().join("tests/launcher_test_host").join(name)
}

/// What the client checks at the breakpoint in `target`, in each world
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
	println!("ok: stopped at the breakpoint in target(), n is 21");
	launcher.resume();
}

fn at_second_target(launcher: &mut Launcher) {
	launcher.stopped("breakpoint");
	let frame = launcher.top_frame();
	assert!(frame["name"].as_str().unwrap().ends_with("second_target"), "stopped in {}", frame);
	println!("ok: stopped at the function breakpoint");
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
	println!("ok: stopped at the runtime error ({})", message);
	launcher.resume();
}

fn main() {
	let res = Command::new(paths::find_dm())
		.with_byond_paths()
		.arg(host_file("launcher_test_host.dme"))
		.status()
		.unwrap();
	assert!(res.success(), "dreammaker build failed");
	let _ = std::fs::remove_file(host_file("rebooted"));

	command_line_shape();
	another_version();

	let mut launcher = Launcher::start();
	launcher.initialize();

	let server = client::server();
	let library = library_path();
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

	launcher.event("initialized");
	println!("ok: the world loaded the debugger and connected");
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

	// The world reboots, and the launcher gives the new one the breakpoints
	at_target(&mut launcher);
	assert!(
		launcher.output.contains("The world restarted"),
		"the launcher didn't see the world restart"
	);
	println!("ok: the breakpoints were set again after the reboot");
	at_second_target(&mut launcher);
	at_runtime(&mut launcher);

	// The world ends, and the launcher with it
	launcher.event("terminated");
	launcher.event("exited");
	// The programs with windows show world.log in them
	if server == "dd" || cfg!(unix) {
		assert!(
			launcher.output.contains("launcher test: world/New"),
			"the world's output wasn't passed on"
		);
		assert!(launcher.output.contains("launcher test: done"), "the world didn't finish");
		println!("ok: the world ended, and its output was passed on");
	} else {
		println!("ok: the world ended");
	}

	launcher.disconnect(json!({}));
	println!("ok: the launcher exited");
	let _ = std::fs::remove_file(host_file("rebooted"));

	attach_and_reload(&server, &library);
	launch_from_the_command_line(&server);
	println!("all ok");
}

/// The command line, a command at a time: each connects, does it and leaves,
/// and the world stays paused between them
fn command_line(pid: u64, code: &Path) {
	let pid = pid.to_string();
	let run = |args: &[&str], code: i32| {
		let args: Vec<&str> = ["--pid", pid.as_str()].iter().chain(args).copied().collect();
		let (got, printed) = launcher_command(&args);
		assert_eq!(got, code, "`{}` gave {}, not {}: {}", args.join(" "), got, code, printed);
		printed
	};

	assert_eq!(run(&["status"], 0).lines().next().unwrap_or_default(), "running");
	let printed = run(&["wait", "--timeout", "1"], 2);
	assert!(printed.contains("timed out"), "wait said {}", printed);

	let line = line_of(code, "\treturn 2").to_string();
	let printed = run(&["wait", "--break", &format!("attach_host.dm:{}", line), "--timeout", "60"], 0);
	assert!(
		printed.starts_with("paused at a breakpoint\n#0 /value  attach_host.dm:") && printed.contains("#1 /run_until_stopped"),
		"wait said {}",
		printed
	);
	let printed = run(&["status"], 0);
	assert!(printed.starts_with("paused at a breakpoint"), "it didn't stay paused: {}", printed);
	run(&["continue"], 0);

	// A line with no code of its own breaks on the next that has
	let declared = line_of(code, "/proc/value()");
	let printed = run(&["wait", "--break", &format!("attach_host.dm:{}", declared), "--timeout", "60"], 0);
	assert!(
		printed.starts_with(&format!("paused at a breakpoint\n#0 /value  attach_host.dm:{}\n", declared + 1)),
		"wait said {}",
		printed
	);
	assert_eq!(run(&["eval", "6", "*", "7"], 0).trim(), "42");
	let printed = run(&["next"], 0);
	assert!(printed.starts_with("paused after a step\n#0 /run_until_stopped"), "next said {}", printed);
	let printed = run(&["vars"], 0);
	assert!(printed.contains("locals:"), "vars said {}", printed);
	assert_eq!(run(&["continue"], 0).trim(), "running");
	assert_eq!(
		run(&["continue"], 1).trim(),
		"the world is running: `pause` it, or `wait` for it to pause"
	);
	println!("ok: the command line waited for a breakpoint, looked around, stepped and let the world carry on");

	let printed = run(&["pause"], 0);
	assert!(
		printed.starts_with("paused where it was asked to\n#0 /run_until_stopped"),
		"pause said {}",
		printed
	);
	let printed = run(&["wait", "--break", "/proc/value"], 0);
	assert!(printed.starts_with("already paused"), "wait said {}", printed);
	run(&["continue"], 0);
	let printed = run(&["wait", "--break", "/proc/value", "--if", "1 == 2", "--timeout", "2"], 2);
	assert!(printed.contains("timed out"), "wait said {}", printed);
	let printed = run(&["wait", "--break", "nowhere.dm:3"], 1);
	assert!(printed.contains("no proc in nowhere.dm"), "wait said {}", printed);
	println!("ok: the command line paused the world, and its waits time out and say what's wrong");
}

/// What the command line is, before any world: a word for each command, and
/// help that says what each does
fn command_line_shape() {
	// Alone, it says how it's used, and doesn't wait to be spoken to
	let (code, alone) = launcher_command(&[]);
	assert_eq!(code, 0, "{}", alone);
	assert!(alone.starts_with("auxtools: debug BYOND worlds"), "auxtools said {}", alone);
	assert!(alone.contains("Usage: auxtools [--pid PID] COMMAND [OPTIONS]"), "auxtools said {}", alone);
	for command in [
		"launch", "stop", "reboot", "list", "status", "stack", "vars", "eval", "pause", "continue", "next", "step", "out", "wait", "dap", "help", "version"
	] {
		assert!(alone.lines().any(|line| line.starts_with(&format!("  {} ", command))), "{} isn't in the help:\n{}", command, alone);
	}
	for args in [&["help"][..], &["--help"][..], &["-h"][..]] {
		let (code, printed) = launcher_command(args);
		assert_eq!((code, printed == alone), (0, true), "`auxtools {}` said {}", args.join(" "), printed);
	}
	println!("ok: auxtools alone, and help, list every command with what it does, and exit");

	let (code, of_wait) = launcher_command(&["help", "wait"]);
	assert_eq!(code, 0);
	assert!(
		of_wait.starts_with("Usage: auxtools [--pid PID] wait [--break PLACE]...") && of_wait.contains("  --timeout SECONDS "),
		"help wait said {}",
		of_wait
	);
	assert_eq!(launcher_command(&["wait", "--help"]), (0, of_wait));
	let (code, printed) = launcher_command(&["help", "frobnicate"]);
	assert_eq!((code, printed.trim()), (1, "there's no command frobnicate: `auxtools help` lists them"));
	println!("ok: help COMMAND, and COMMAND --help, show a command's options");

	let (code, version) = launcher_command(&["version"]);
	assert_eq!(code, 0);
	assert!(version.starts_with("auxtools ") && protocol(&version) > 0, "version said {}", version);
	assert_eq!(launcher_command(&["--version"]), (0, version));
	println!("ok: version and --version say the launcher's version and its protocol's");

	let (code, list) = launcher_command(&["list"]);
	assert_eq!(code, 0, "{}", list);
	assert!(serde_json::from_str::<serde_json::Value>(&list).is_ok_and(|worlds| worlds.is_array()), "list said {}", list);
	for gone in ["--list", "--build"] {
		let (code, printed) = launcher_command(&[gone]);
		assert_eq!(code, 1, "`auxtools {}` said {}", gone, printed);
	}
	println!("ok: list is a command, and the old flags are gone");
}

/// The protocol version in what `auxtools version` prints
fn protocol(version: &str) -> u32 {
	version
		.trim()
		.rsplit_once("(protocol ")
		.and_then(|(_, number)| number.strip_suffix(')'))
		.and_then(|number| number.parse().ok())
		.unwrap_or_else(|| panic!("version said {}", version))
}

/// A world whose debug library speaks another version of the protocol: it's
/// listed as one that can't be attached to, and attaching says why. The
/// stand-in is a BYOND server with no debug library, and this test listening
/// where the library of the version before ours would.
fn another_version() {
	use interprocess::local_socket::{prelude::*, GenericNamespaced, ListenerOptions};

	let mut plain = Command::new(paths::find_dreamdaemon())
		.with_byond_paths()
		.arg(host_file("launcher_test_host.dmb"))
		.args(test_args().unwrap_or_else(|| vec!["-trusted".to_owned()]))
		.stdout(std::process::Stdio::null())
		.stderr(std::process::Stdio::null())
		.spawn()
		.unwrap();
	let pid = plain.id();
	let ours = protocol(&launcher_command(&["version"]).1);
	let theirs = ours - 1;
	let name = format!("auxtools-debug-v{}-{}", theirs, pid);
	let listener = ListenerOptions::new().name(name.to_ns_name::<GenericNamespaced>().unwrap()).create_sync().unwrap();
	// Whoever connects is let in, and told nothing
	std::thread::spawn(move || {
		for client in listener.incoming() {
			drop(client);
		}
	});

	let (code, list) = launcher_command(&["list"]);
	assert_eq!(code, 0, "{}", list);
	let worlds: serde_json::Value = serde_json::from_str(&list).unwrap();
	let listed = worlds
		.as_array()
		.unwrap()
		.iter()
		.find(|world| world["pid"] == pid)
		.unwrap_or_else(|| panic!("the other version's world isn't listed: {}", list));
	assert_eq!((&listed["protocol"], &listed["compatible"]), (&json!(theirs), &json!(false)), "{}", listed);

	let pid_text = pid.to_string();
	let (code, printed) = launcher_command(&["--pid", &pid_text, "status"]);
	assert_eq!(code, 1, "{}", printed);
	assert!(
		printed.contains(&format!("its debug library speaks protocol {}, and this auxtools speaks {}", theirs, ours)),
		"attaching to another version's world said {}",
		printed
	);
	let _ = plain.kill();
	let _ = plain.wait();
	println!("ok: a world whose debug library speaks another protocol version is listed as incompatible, and attaching to it says why");
}

/// The rest of the command line: the other steps, the stacks, a frame's
/// variables, runtime errors, and what it says to what it doesn't understand
fn command_line_more(pid: u64, dir: &Path) {
	let pid = pid.to_string();
	let run = |args: &[&str], code: i32| {
		let args: Vec<&str> = ["--pid", pid.as_str()].iter().chain(args).copied().collect();
		let (got, printed) = launcher_command(&args);
		assert_eq!(got, code, "`{}` gave {}, not {}: {}", args.join(" "), got, code, printed);
		printed
	};

	let printed = run(&["wait", "--break", "/proc/value", "--timeout", "60"], 0);
	assert!(printed.starts_with("paused at a breakpoint\n#0 /value"), "wait said {}", printed);
	let printed = run(&["out"], 0);
	assert!(printed.starts_with("paused after a step\n#0 /run_until_stopped"), "out said {}", printed);
	let printed = run(&["step"], 0);
	assert!(printed.starts_with("paused after a step\n#0 /run_until_stopped"), "step said {}", printed);
	println!("ok: the command line steps out of a proc, and into a line");

	let printed = run(&["stack"], 0);
	assert!(printed.starts_with("#0 /run_until_stopped") && !printed.contains("stack 0"), "stack said {}", printed);
	let printed = run(&["stack", "--all"], 0);
	// The running proc's first, then the sleeping ones, each with the id it keeps
	let first = printed.split_once(": ").map(|(_, rest)| rest).unwrap_or_default();
	assert!(
		printed.starts_with("stack ") && first.starts_with("/run_until_stopped\n#0 /run_until_stopped") && printed.contains(" (sleeping)\n#0 "),
		"stack --all said {}",
		printed
	);
	println!("ok: the command line shows the paused stack, or every stack");

	let printed = run(&["vars"], 0);
	assert!(printed.contains("arguments:\n  src = null") && printed.contains("locals:") && !printed.contains("globals:"), "vars said {}", printed);
	let printed = run(&["vars", "--frame", "0", "--globals"], 0);
	assert!(printed.contains("globals:"), "vars --globals said {}", printed);
	let printed = run(&["vars", "--frame", "7"], 1);
	assert_eq!(printed.trim(), "there's no frame 7");
	assert_eq!(run(&["eval", "--frame", "0", "6", "*", "7"], 0).trim(), "42");
	let printed = run(&["eval", "--frame", "7", "1"], 1);
	assert_eq!(printed.trim(), "there's no frame 7");
	let printed = run(&["eval", "1", "+"], 1);
	assert!(printed.starts_with("parser error: "), "eval said {}", printed);
	assert_eq!(run(&["eval", "nothing_called_this"], 1).trim(), "undefined var: nothing_called_this");
	assert_eq!(run(&["eval", "1", "/", "0"], 1).trim(), "runtime error: Division by zero");
	println!("ok: the command line shows a frame's variables, with the globals when asked, and evaluates in a frame");
	run(&["continue"], 0);

	// The world fails when it finds this file
	std::fs::write(dir.join("fail"), "yes").unwrap();
	let printed = run(&["wait", "--runtimes", "--timeout", "60"], 0);
	assert!(
		printed.starts_with("paused on a runtime error: cannot read from list\n#0 /fail"),
		"wait --runtimes said {}",
		printed
	);
	run(&["continue"], 0);
	println!("ok: the command line waits for a runtime error");

	// A wait that doesn't hold its pause: the world carries on when it's gone
	let printed = run(&["wait", "--break", "/proc/value", "--no-hold", "--timeout", "60"], 0);
	assert!(printed.starts_with("paused at a breakpoint\n#0 /value"), "wait --no-hold said {}", printed);
	assert_eq!(run(&["status"], 0).lines().next().unwrap_or_default(), "running");
	println!("ok: a wait that doesn't hold its pause leaves the world running");

	// Breakpoints with a message: each hit prints it, and nothing pauses
	let printed = run(&["wait", "--break", "/proc/value", "--log", "value() at {world.time > 0}", "--timeout", "3"], 2);
	assert!(printed.starts_with("value() at 1\n"), "wait --log said {}", printed);
	assert!(printed.contains("timed out"), "wait --log said {}", printed);
	assert_eq!(run(&["status"], 0).lines().next().unwrap_or_default(), "running");
	println!("ok: the command line prints a breakpoint's message at each hit, and the world runs on");

	// A global, watched from a running world; then a var of the proc it
	// paused in, until that changes
	let printed = run(&["wait", "--change", "rounds", "--timeout", "60"], 0);
	assert!(
		printed.starts_with("watching rounds, now ") && printed.contains("\npaused after a change: rounds changed from ") && printed.contains("\n#0 /run_until_stopped"),
		"wait --change said {}",
		printed
	);
	let printed = run(&["wait", "--change", "said", "--if", "value > old", "--timeout", "60"], 0);
	assert!(
		printed.starts_with("watching said, now ") && printed.contains("\npaused after a change: said changed from ") && printed.contains("\n#0 /run_until_stopped"),
		"wait --change from a pause said {}",
		printed
	);
	let printed = run(&["wait", "--change", "1 + 1"], 1);
	assert!(printed.starts_with("couldn't watch 1 + 1: "), "wait --change said {}", printed);
	run(&["continue"], 0);
	println!("ok: the command line waits for a var to change: a global from a running world, a proc's own from a pause");

	for (args, said) in [
		(&["frobnicate"][..], "there's no command frobnicate"),
		(&["stack", "--sideways"][..], "there's no --sideways here"),
		(&["vars", "--frame"][..], "--frame needs a value"),
		(&["vars", "--frame", "top"][..], "--frame top isn't a number"),
		(&["eval"][..], "eval what?"),
		(&["wait", "--break", "somewhere"][..], "somewhere is neither FILE:LINE nor /proc/path"),
		(&["wait", "now"][..], "wait doesn't take now"),
		(
			&["wait", "--break", "/proc/value", "--if", "1 =="][..],
			"couldn't break at /proc/value: the condition doesn't compile: parser error: "
		)
	] {
		let printed = run(args, 1);
		assert!(printed.starts_with(said), "`{}` said {}", args.join(" "), printed);
	}
	let (code, printed) = launcher_command(&["--pid", "soon", "status"]);
	assert_eq!((code, printed.trim()), (1, "soon isn't a process id"));
	let (code, printed) = launcher_command(&["--pid", "4000000000", "status"]);
	assert_eq!(code, 1);
	assert!(printed.starts_with("couldn't reach the world in process 4000000000"), "status said {}", printed);
	assert_eq!(run(&["status"], 0).lines().next().unwrap_or_default(), "running");
	println!("ok: the command line says what it doesn't understand, and exits with 1");
}

/// The line of `file` that `text` starts
fn line_of(file: &Path, text: &str) -> u64 {
	let source = std::fs::read_to_string(file).unwrap();
	source.lines().position(|line| line.starts_with(text)).expect("no such line") as u64 + 1
}

/// A world launched without debugging, found with `--list`, attached to by
/// two launchers at once, and hot reloaded by the one that launched it
fn attach_and_reload(server: &str, library: &Path) {
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
	runner.event("initialized");
	runner.request("configurationDone", json!({}));
	runner.wait_output("attach test: 1");
	runner.wait_output("Hot reload: watching");
	println!("ok: a world launched without debugging runs, and its code is watched");

	let pid = client::listed_world("attach_host.dmb");
	println!("ok: --list found the world, process {}", pid);

	// Two debuggers: one finds the code from the world's command line, the
	// other is told where it is
	let mut first = Launcher::start();
	first.initialize();
	first.request("attach", json!({ "processId": pid.to_string() }));
	first.event("initialized");
	first.request("configurationDone", json!({}));
	let mut second = Launcher::start();
	second.initialize();
	second.request("attach", json!({ "processId": pid, "dme": dme }));
	second.event("initialized");
	second.request("configurationDone", json!({}));

	// Either one's breakpoint stops the world for both, and either can let it
	// carry on
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
	println!("ok: two launchers attached, both stopped at one's breakpoint, the other let it carry on");

	// The launcher that started it reloads it as the code changes
	let source = std::fs::read_to_string(&code).unwrap();
	std::fs::write(&code, source.replace("\treturn 1", "\treturn 2")).unwrap();
	runner.wait_output("Hot reload: reloaded /value");
	runner.wait_output("attach test: 2");
	assert!(
		!runner.output.contains("reading all the code"),
		"a change to a proc's code read all the code again"
	);
	println!("ok: the world was hot reloaded, from the changed file read on its own");

	// Leaving doesn't end a world that was attached to
	first.disconnect(json!({}));
	second.disconnect(json!({ "terminateDebuggee": false }));
	let seen = runner.output.matches("attach test: 2").count();
	runner.wait_count("attach test: 2", seen + 2);
	println!("ok: the attached launchers left, and the world carries on");

	command_line(pid, &code);
	command_line_more(pid, &dir);
	journal::runtimes(pid, &dir);
	journal::reloads(pid, &code);
	journal::reboot(pid, &dir);

	// A launcher that attaches to a world that's paused hears it is
	let (code_given, printed) = launcher_command(&["--pid", &pid.to_string(), "pause"]);
	assert_eq!(code_given, 0, "pause said {}", printed);
	let mut late = Launcher::start();
	late.initialize();
	late.request("attach", json!({ "processId": pid.to_string() }));
	late.event("initialized");
	late.request("configurationDone", json!({}));
	late.stopped("pause");
	late.resume();
	late.disconnect(json!({}));
	println!("ok: a launcher that attaches to a paused world hears it's paused");
	let seen = runner.output.matches("attach test: 2").count();
	runner.wait_count("attach test: 2", seen + 1);
	println!("ok: the world carries on after the command line let it");

	let waiting = journal::wait_for_reboot(pid);
	std::fs::write(dir.join("stop"), "yes").unwrap();
	runner.event("terminated");
	journal::world_went_away(waiting);
	assert!(runner.output.contains("attach test: done"), "the world didn't finish");
	runner.disconnect(json!({}));
	println!("ok: the world ended, and the launcher with it");
	let _ = std::fs::remove_dir_all(&dir);
}

/// A copy of the world that runs until told to stop, built, in a directory of
/// its own named after `name`, so its code can be edited
fn attach_host_copy(name: &str) -> (PathBuf, PathBuf) {
	let dir = std::env::temp_dir().join(format!("launcher_test_{}-{}", name, std::process::id()));
	std::fs::create_dir_all(&dir).unwrap();
	for file in ["attach_host.dme", "attach_host.dm"] {
		std::fs::copy(host_file(file), dir.join(file)).unwrap();
	}
	let dme = dir.join("attach_host.dme");
	let res = Command::new(paths::find_dm()).with_byond_paths().arg(&dme).status().unwrap();
	assert!(res.success(), "dreammaker build failed");
	(dir, dme)
}

/// Processes the test started, ended when it's done with them, whether it
/// passes or fails: a world, a launcher in the background, a launcher in the
/// foreground (`child`)
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
		// A check that failed may not have got as far as hearing which
		// processes there were: the worlds of this run's copies are found, and
		// their launchers go with them
		if std::thread::panicking() {
			let list = Command::new(launcher_path()).arg("list").output().ok();
			let worlds: serde_json::Value = list.and_then(|list| serde_json::from_slice(&list.stdout).ok()).unwrap_or_default();
			let ours = format!("launcher_test_launch-{}", std::process::id());
			for world in worlds.as_array().into_iter().flatten() {
				if world["dmb"].as_str().is_some_and(|dmb| dmb.contains(&ours)) {
					self.pids.extend(world["pid"].as_u64());
				}
			}
		}
		for pid in &self.pids {
			let _ = if cfg!(windows) {
				Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output()
			} else {
				Command::new("kill").args(["-9", &pid.to_string()]).output()
			};
		}
	}
}

/// Whether process `pid` is running
fn alive(pid: u64) -> bool {
	if cfg!(windows) {
		let listed = Command::new("tasklist").args(["/FI", &format!("PID eq {}", pid), "/NH"]).output().unwrap();
		String::from_utf8_lossy(&listed.stdout).split_whitespace().any(|word| word == pid.to_string())
	} else {
		// One that's ended and not waited for yet is a zombie, `Z`
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

/// Waits for `text` to be in what `read` gives
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

/// `auxtools launch DMB` and the options every launch here has
fn launch_args(dmb: &Path, server: &str, more: &[&str]) -> Vec<String> {
	let mut args: Vec<String> = vec!["launch".into(), dmb.to_string_lossy().into_owned()];
	args.extend(["--byond".into(), paths::find_byond().to_string_lossy().into_owned(), "--server".into(), server.to_owned()]);
	args.extend(more.iter().map(|arg| (*arg).to_owned()));
	if let Some(theirs) = test_args() {
		args.push("--".into());
		args.extend(theirs);
	}
	args
}

/// `auxtools launch` in the foreground, its output collected as it comes
fn launch_here(dmb: &Path, server: &str, started: &mut Started) -> Arc<Mutex<String>> {
	let mut child = Command::new(launcher_path())
		.args(launch_args(dmb, server, &[]))
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	let output = Arc::new(Mutex::new(String::new()));
	for stream in [Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>, Box::new(child.stderr.take().unwrap())] {
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

/// The exit code of the foreground launcher, once it's gone
fn launcher_exit(started: &mut Started, output: &Mutex<String>) -> i32 {
	let child = started.child.as_mut().unwrap();
	let until = Instant::now() + Duration::from_secs(30);
	loop {
		if let Some(status) = child.try_wait().unwrap() {
			return status.code().unwrap_or(-1);
		}
		assert!(Instant::now() < until, "the launcher didn't exit:\n{}", output.lock().unwrap());
		std::thread::sleep(Duration::from_millis(100));
	}
}

/// The process id in `launch`'s line about the program it started
fn started_pid(output: &str) -> u64 {
	output
		.lines()
		.find_map(|line| line.strip_prefix("Started ")?.rsplit_once("(process ")?.1.strip_suffix(')')?.parse().ok())
		.unwrap_or_else(|| panic!("launch didn't say which process it started:\n{}", output))
}

/// `auxtools launch`, `stop` and `reboot`: worlds the command line starts and
/// ends itself, in the foreground and in the background
fn launch_from_the_command_line(server: &str) {
	let (dir, dme) = attach_host_copy("launch");
	let dmb = dme.with_extension("dmb");
	let code = dir.join("attach_host.dm");

	// The launcher never builds: a world that isn't built isn't launched
	let missing = dir.join("nothing.dmb");
	for more in [&[][..], &["--detach"][..]] {
		let args = launch_args(&missing, server, more);
		let (got, printed) = launcher_command(&args.iter().map(String::as_str).collect::<Vec<_>>());
		assert_eq!(got, 1, "launch of a world that isn't there gave {}: {}", got, printed);
		assert!(printed.contains("nothing.dmb: build the world first"), "launch said {}", printed);
	}
	println!("ok: launch says a world that isn't built isn't there, and exits with 1");

	// In the foreground: the world's output, its hot reloads, and its end
	let mut started = Started::default();
	let output = launch_here(&dmb, server, &mut started);
	let read = || output.lock().unwrap().clone();
	wait_for_text("launch", "Hot reload: watching", read);
	wait_for_text("launch", "attach test: 1", read);
	started.pids.push(started_pid(&read()));
	let source = std::fs::read_to_string(&code).unwrap();
	std::fs::write(&code, source.replace("\treturn 1", "\treturn 2")).unwrap();
	wait_for_text("launch", "Hot reload: reloaded /value", read);
	wait_for_text("launch", "attach test: 2", read);
	std::fs::write(dir.join("stop"), "yes").unwrap();
	let exit = launcher_exit(&mut started, &output);
	let printed = read();
	assert_eq!(exit, 0, "launch exited with {}:\n{}", exit, printed);
	assert!(printed.contains("attach test: done") && printed.contains("The world exited with code 0"), "launch said:\n{}", printed);
	std::fs::remove_file(dir.join("stop")).unwrap();
	std::fs::write(&code, source).unwrap();
	drop(started);
	println!("ok: launch in the foreground prints the world's output and its hot reloads, and ends with it");

	// Interrupted, it has the world end itself
	if cfg!(unix) {
		let mut started = Started::default();
		let output = launch_here(&dmb, server, &mut started);
		let read = || output.lock().unwrap().clone();
		wait_for_text("launch", "Hot reload: watching", read);
		started.pids.push(started_pid(&read()));
		let launcher = started.child.as_ref().unwrap().id().to_string();
		assert!(Command::new("kill").args(["-TERM", &launcher]).status().unwrap().success());
		let exit = launcher_exit(&mut started, &output);
		let printed = read();
		assert_eq!(exit, 0, "launch exited with {}:\n{}", exit, printed);
		assert!(
			printed.contains("Interrupted: asking the world to end") && printed.contains("attach test: world/Del"),
			"launch said:\n{}",
			printed
		);
		println!("ok: launch, interrupted, has the world end itself through its world/Del()");
	}

	// In the background: it returns once the world is up, and other
	// commands work with it
	let mut started = Started::default();
	let args = launch_args(&dmb, server, &["--detach", "--json"]);
	let (got, printed) = launcher_command(&args.iter().map(String::as_str).collect::<Vec<_>>());
	assert_eq!(got, 0, "launch --detach said {}", printed);
	let up: serde_json::Value = serde_json::from_str(&printed).unwrap_or_else(|_| panic!("launch --detach --json said {}", printed));
	let (pid, launcher) = (up["pid"].as_u64().unwrap(), up["launcher"].as_u64().unwrap());
	started.pids.extend([pid, launcher]);
	let log = PathBuf::from(up["log"].as_str().unwrap());
	assert_eq!(log, dir.join("attach_host.auxtools.log"));
	let read_log = || std::fs::read_to_string(&log).unwrap_or_default();
	assert!(read_log().contains("Hot reload: watching"), "the world was up before its code was watched:\n{}", read_log());
	let listed = client::listed_worlds();
	assert!(
		listed.iter().any(|world| world["pid"] == pid && world["dmb"].as_str().is_some_and(|dmb| dmb.contains("launcher_test_launch-"))),
		"list doesn't show the world: {:?}",
		listed
	);
	let pid_text = pid.to_string();
	let run = |args: &[&str], code: i32| {
		let args: Vec<&str> = ["--pid", pid_text.as_str()].iter().chain(args).copied().collect();
		let (got, printed) = launcher_command(&args);
		assert_eq!(got, code, "`{}` gave {}, not {}: {}", args.join(" "), got, code, printed);
		printed
	};
	assert_eq!(run(&["status"], 0).trim(), "running");
	assert_eq!(run(&["eval", "6", "*", "7"], 0).trim(), "42");
	wait_for_text("the background launcher's log", "attach test: 1", read_log);
	println!("ok: launch --detach returns when the world is up, with its process, and commands work with it");

	let printed = run(&["reboot"], 0);
	assert!(printed.starts_with("The world rebooted and is answering again, "), "reboot said {}", printed);
	assert_eq!(run(&["status"], 0).trim(), "running");
	wait_for_text("the background launcher's log", "The world restarted", read_log);
	println!("ok: reboot reboots the world, and returns when it answers again; the launcher connects again");

	let printed = run(&["stop"], 0);
	assert_eq!(printed.trim(), format!("The world has ended (process {})", pid));
	assert!(!alive(pid), "the world is still there");
	assert!(gone_within(launcher, 10), "the background launcher didn't go with the world");
	let said = read_log();
	assert!(said.contains("attach test: world/Del") && said.contains("The world exited with code 0"), "the log has:\n{}", said);
	drop(started);
	println!("ok: stop has the world end itself, and the background launcher goes with it");

	// A world whose world/Del() doesn't end it
	let mut started = Started::default();
	let args = launch_args(&dmb, server, &["--detach"]);
	let (got, printed) = launcher_command(&args.iter().map(String::as_str).collect::<Vec<_>>());
	assert_eq!(got, 0, "launch --detach said {}", printed);
	let pid: u64 = printed
		.strip_prefix("The world is up: process ")
		.and_then(|rest| rest.lines().next()?.parse().ok())
		.unwrap_or_else(|| panic!("launch --detach said {}", printed));
	let launcher: u64 = printed
		.split_once("Its launcher is process ")
		.and_then(|(_, rest)| rest.split_once(',')?.0.parse().ok())
		.unwrap_or_else(|| panic!("launch --detach said {}", printed));
	started.pids.extend([pid, launcher]);
	std::fs::write(dir.join("refuse_del"), "yes").unwrap();
	let pid_text = pid.to_string();
	let run = |args: &[&str], code: i32| {
		let args: Vec<&str> = ["--pid", pid_text.as_str()].iter().chain(args).copied().collect();
		let (got, printed) = launcher_command(&args);
		assert_eq!(got, code, "`{}` gave {}, not {}: {}", args.join(" "), got, code, printed);
		printed
	};
	let printed = run(&["stop", "--timeout", "2"], 1);
	assert!(
		printed.starts_with(&format!("the world (process {}) is still running 2 s on: its world/Del() didn't end it", pid)),
		"stop said {}",
		printed
	);
	assert!(alive(pid), "the world ended");
	let printed = run(&["stop", "--timeout", "0", "--kill"], 0);
	assert_eq!(printed.trim(), format!("The world didn't end in 0 s, so its process ({}) was ended", pid));
	assert!(!alive(pid), "the world is still there");
	wait_for_text("the background launcher's log", "The world was ended", || std::fs::read_to_string(&log).unwrap_or_default());
	assert!(gone_within(launcher, 10), "the background launcher didn't go with the world");
	drop(started);
	println!("ok: stop says when the world's Del() doesn't end it, and --kill ends its process");

	let _ = std::fs::remove_dir_all(&dir);
}
