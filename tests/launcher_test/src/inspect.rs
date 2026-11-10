//! The command line's commands for looking at a world: what `eval` says when
//! it fails, expressions from files, JSON, where `wait`'s breakpoints bound,
//! a proc's bytecode and whereabouts, and steps of one instruction

use std::{
	io::Write,
	path::Path,
	process::{Command, Stdio}
};

use super::{client::launcher_path, line_of};

/// What a command did: its exit code, and what it printed on stdout and on
/// stderr, apart
struct Ran {
	code: i32,
	out: String,
	err: String
}

/// `auxtools --pid PID ARGS...`, with `input` on its stdin
fn auxtools(pid: &str, args: &[&str], input: Option<&str>) -> Ran {
	let mut child = Command::new(launcher_path())
		.args(["--pid", pid])
		.args(args)
		.stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	if let Some(input) = input {
		child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
	}
	let out = child.wait_with_output().unwrap();
	Ran {
		code: out.status.code().unwrap_or(-1),
		out: String::from_utf8_lossy(&out.stdout).into_owned(),
		err: String::from_utf8_lossy(&out.stderr).into_owned()
	}
}

/// Runs it, and checks it exited with `code`
fn run(pid: &str, args: &[&str], code: i32) -> Ran {
	let ran = auxtools(pid, args, None);
	assert_eq!(ran.code, code, "`{}` gave {}, not {}: {}{}", args.join(" "), ran.code, code, ran.out, ran.err);
	ran
}

/// A failure: exit code 1, nothing on stdout, and what it said on stderr
fn fails(pid: &str, args: &[&str]) -> String {
	let ran = run(pid, args, 1);
	assert!(ran.out.is_empty(), "`{}` failed, and printed {} on stdout", args.join(" "), ran.out);
	assert!(!ran.err.is_empty(), "`{}` failed without saying why", args.join(" "));
	ran.err.trim_end().to_owned()
}

pub fn command_line_inspect(pid: u64, dir: &Path, code: &Path) {
	let pid = pid.to_string();
	let pid = pid.as_str();
	eval_failures(pid, code);
	from_files(pid, dir);
	procs(pid, code);
	where_breakpoints_bound(pid, code);
}

/// `eval` fails on stderr with exit code 1, whatever went wrong, and a
/// runtime error says where it happened
fn eval_failures(pid: &str, code: &Path) {
	let said = fails(pid, &["eval", "fail_in(3)"]);
	let lines: Vec<&str> = said.lines().collect();
	assert!(lines.len() == 4 && lines[0].starts_with("runtime error: "), "eval said {}", said);
	assert_eq!(lines[1], format!("  in /fail_deeper  attach_host.dm:{}", line_of(code, "\treturn L[n]")));
	assert_eq!(lines[2], format!("  called by /fail_in  attach_host.dm:{}", line_of(code, "\tvar/got = fail_deeper(n)")));
	assert_eq!(lines[3], "  called by the expression");
	// In the expression itself, there's nowhere else to say
	assert_eq!(fails(pid, &["eval", "1/0"]), "runtime error: Division by zero");
	// One the proc catches is no failure
	assert_eq!(run(pid, &["eval", "caught_inside()"], 0).out.trim(), "\"caught\"");
	println!("ok: eval's runtime error says the proc it was in, with its file and line, and the procs between it and the expression");

	// Paused, the same
	run(pid, &["pause"], 0);
	let said = fails(pid, &["eval", "fail_in(3)"]);
	assert!(said.contains("\n  in /fail_deeper  ") && said.ends_with("\n  called by the expression"), "eval said {}", said);
	for (args, said) in [
		(&["eval", "1 +"][..], "parser error: "),
		(&["eval", "nothing_called_this"][..], "undefined var: nothing_called_this"),
		(&["eval", "--frame", "9", "1"][..], "there's no frame 9"),
		(&["eval"][..], "eval what?")
	] {
		let got = fails(pid, args);
		assert!(got.starts_with(said), "`{}` said {}", args.join(" "), got);
	}
	run(pid, &["continue"], 0);
	println!("ok: every way eval fails is on stderr, with exit code 1 and nothing on stdout");
}

/// Expressions and messages from files and stdin, which no shell gets to
/// change on the way
fn from_files(pid: &str, dir: &Path) {
	let file = |name: &str, text: &str| {
		let path = dir.join(name);
		std::fs::write(&path, text).unwrap();
		path.to_str().unwrap().to_owned()
	};
	let expression = file("expression.txt", "\"[\"a b\"] /datum/x\" + \"!\"\n");
	assert_eq!(run(pid, &["eval", "-f", &expression], 0).out.trim(), "\"a b /datum/x!\"");
	let ran = auxtools(pid, &["eval", "-"], Some("6 * 7\n"));
	assert_eq!((ran.code, ran.out.trim()), (0, "42"), "eval - said {}{}", ran.out, ran.err);
	let said = fails(pid, &["eval", "-f", "no such file.txt"]);
	assert!(said.starts_with("couldn't read no such file.txt: "), "eval -f said {}", said);
	println!("ok: eval reads its expression from a file, or stdin");

	let never = file("never.txt", "1 == 2\n");
	let ran = run(pid, &["wait", "--break", "/proc/value", "--if", &format!("@{}", never), "--timeout", "2"], 2);
	assert!(ran.err.contains("timed out"), "wait said {}{}", ran.out, ran.err);
	let message = file("message.txt", "value() said \"{1 + 1}\"\n");
	let ran = run(pid, &["wait", "--break", "/proc/value", "--log", &format!("@{}", message), "--timeout", "2"], 2);
	assert!(ran.out.starts_with("value() said \"2\"\n"), "wait --log said {}", ran.out);
	let watched = file("watched.txt", "rounds");
	let ran = run(pid, &["wait", "--change", &format!("@{}", watched), "--timeout", "60"], 0);
	assert!(ran.out.starts_with("watching rounds, now "), "wait --change said {}", ran.out);
	run(pid, &["continue"], 0);
	// Two @s are one, and no file
	let said = fails(pid, &["wait", "--break", "/proc/value", "--if", &format!("@@{}", never)]);
	assert!(said.starts_with("couldn't break at /proc/value: the condition doesn't compile"), "wait said {}", said);
	let said = fails(pid, &["wait", "--break", "/proc/value", "--if", "@no such file.txt"]);
	assert!(said.starts_with("couldn't read no such file.txt: "), "wait said {}", said);
	println!("ok: wait's --if, --log and --change read their text from @FILE, and @@ is a plain @");
}

/// Where a proc is, and its bytecode
fn procs(pid: &str, code: &Path) {
	// value() was hot reloaded before this
	let printed = run(pid, &["proc", "/proc/value"], 0).out;
	assert_eq!(
		printed,
		format!("/value  attach_host.dm:{}\ndefinitions: 1 (#0 to #0)\nhot reload: new code\n", line_of(code, "\treturn 2"))
	);
	let printed = run(pid, &["proc", "/fail_in"], 0).out;
	assert!(printed.ends_with("\nhot reload: not since the world started\n"), "proc said {}", printed);
	let printed = run(pid, &["proc", "/datum/twice/act#1"], 0).out;
	assert_eq!(
		printed,
		format!(
			"/datum/twice/act#1  attach_host.dm:{}\ndefinitions: 2 (#0 to #1)\nhot reload: not since the world started\n",
			line_of(code, "\t. = ..()")
		)
	);
	let printed = run(pid, &["proc", "/datum/twice/proc/act"], 0).out;
	assert!(printed.starts_with(&format!("/datum/twice/act  attach_host.dm:{}\n", line_of(code, "\treturn \"first\""))), "proc said {}", printed);
	for (args, said) in [
		(&["proc", "/proc/nothing_called_this"][..], "the world has no proc /proc/nothing_called_this"),
		(&["proc", "/value#3"][..], "/value has one definition, #0: there's no #3"),
		(&["proc", "/datum/twice/act#2"][..], "/datum/twice/act has 2 definitions, #0 to #1: there's no #2"),
		(&["proc", "value"][..], "value isn't a proc's path: it starts with /"),
		(&["proc", "/value#x"][..], "x isn't a definition's number"),
		(&["proc"][..], "proc which proc?"),
		(&["dis", "/proc/nothing_called_this"][..], "the world has no proc /proc/nothing_called_this")
	] {
		assert_eq!(fails(pid, args), said, "`{}`", args.join(" "));
	}
	println!("ok: proc says where a proc is, how many definitions it has, and whether a hot reload changed it");

	let printed = run(pid, &["dis", "/datum/twice/act#1"], 0).out;
	let (first, second) = (line_of(code, "\t. = ..()"), line_of(code, "\treturn \"second\""));
	let expected = format!(
		"/datum/twice/act#1  attach_host.dm\nline {first}:\n     0  DbgFile \"attach_host.dm\"\n     2  DbgLine {first}\n     4  CallParent\n     5  SetVar \
		 dot\nline {second}:\n     7  DbgLine {second}\n     9  PushVal \"second\"\n    12  Ret\n    13  End\n"
	);
	assert_eq!(printed, expected);
	println!("ok: dis prints a proc's instructions with their offsets, and where each line starts");
}

/// wait says where each of its breakpoints went, and fails before waiting
/// on a place the world doesn't have
fn where_breakpoints_bound(pid: &str, code: &Path) {
	let declared = line_of(code, "/proc/value()");
	let ran = run(pid, &["wait", "--break", &format!("attach_host.dm:{}", declared), "--timeout", "60"], 0);
	assert!(ran.out.starts_with("paused at a breakpoint\n#0 /value"), "wait said {}", ran.out);
	assert_eq!(
		ran.err,
		format!("breakpoint at /value  attach_host.dm:{} (line {} has no code: this is the next that has)\n", declared + 1, declared)
	);
	run(pid, &["continue"], 0);
	let ran = run(pid, &["wait", "--break", "/proc/fail_in", "--break", "/datum/twice/act#1", "--timeout", "1"], 2);
	assert_eq!(
		ran.err,
		format!(
			"breakpoint at /fail_in  attach_host.dm:{}\nbreakpoint at /datum/twice/act#1  attach_host.dm:{}\ntimed out: the world didn't pause\n",
			line_of(code, "\tvar/got = fail_deeper(n)"),
			line_of(code, "\t. = ..()")
		)
	);
	println!("ok: wait says where each breakpoint went, on the next line with code when its own has none");

	let last = line_of(code, "\treturn \"second\"");
	let nowhere = [
		("nowhere.dm:3", "couldn't break at nowhere.dm:3: the world has no code from a file called nowhere.dm".to_owned()),
		("attach_host.dm:9999", format!("couldn't break at attach_host.dm:9999: attach_host.dm has no code after line {}", last)),
		("/proc/nothing_called_this", "couldn't break at /proc/nothing_called_this: the world has no proc /proc/nothing_called_this".to_owned()),
		("/value#2", "couldn't break at /value#2: /value has one definition, #0: there's no #2".to_owned())
	];
	// With no time limit: it would wait for ever if it didn't fail first
	for (place, said) in &nowhere {
		assert_eq!(&fails(pid, &["wait", "--break", place]), said);
	}
	run(pid, &["pause"], 0);
	for (place, said) in &nowhere {
		assert_eq!(&fails(pid, &["wait", "--break", place]), said);
	}
	run(pid, &["continue"], 0);
	println!("ok: wait fails at once on a file the world doesn't have, a line past its code, and a proc it doesn't have, paused or not");
}
