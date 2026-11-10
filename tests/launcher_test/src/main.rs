//! Drives the launcher the way VS Code would, in the Debug Adapter Protocol,
//! against a world with none of auxtools' procs in it: it has to load the
//! debugger itself, stop at breakpoints, pick up again after the world
//! reboots, and go away when the world ends. Then a world launched without
//! debugging is found with `--list`, two more launchers attach to it at once,
//! and the first hot reloads it. Last, the launcher's command line pauses it
//! and looks around, a command at a time.
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

use std::{
	path::{Path, PathBuf},
	process::Command
};

use client::{launcher_command, library_path, test_args, Launcher};
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

	assert_eq!(run(&["status"], 0).trim(), "running");
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
	assert_eq!(run(&["status"], 0).trim(), "running");
	println!("ok: a wait that doesn't hold its pause leaves the world running");

	// Breakpoints with a message: each hit prints it, and nothing pauses
	let printed = run(&["wait", "--break", "/proc/value", "--log", "value() at {world.time > 0}", "--timeout", "3"], 2);
	assert!(printed.starts_with("value() at 1\n"), "wait --log said {}", printed);
	assert!(printed.contains("timed out"), "wait --log said {}", printed);
	assert_eq!(run(&["status"], 0).trim(), "running");
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
	assert_eq!(run(&["status"], 0).trim(), "running");
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
	// A copy, so the code can be edited
	let dir = std::env::temp_dir().join(format!("launcher_test_attach-{}", std::process::id()));
	std::fs::create_dir_all(&dir).unwrap();
	for file in ["attach_host.dme", "attach_host.dm"] {
		std::fs::copy(host_file(file), dir.join(file)).unwrap();
	}
	let dme = dir.join("attach_host.dme");
	let code = dir.join("attach_host.dm");
	let res = Command::new(paths::find_dm()).with_byond_paths().arg(&dme).status().unwrap();
	assert!(res.success(), "dreammaker build failed");

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

	std::fs::write(dir.join("stop"), "yes").unwrap();
	runner.event("terminated");
	assert!(runner.output.contains("attach test: done"), "the world didn't finish");
	runner.disconnect(json!({}));
	println!("ok: the world ended, and the launcher with it");
	let _ = std::fs::remove_dir_all(&dir);
}
