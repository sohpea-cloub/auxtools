//! The command line's commands for looking at a world: what `eval` says when
//! it fails, expressions from files, JSON, where `wait`'s breakpoints bound,
//! a proc's bytecode and whereabouts, and steps of one instruction

use std::{
	io::Write,
	path::Path,
	process::{Command, Stdio}
};

use serde_json::{json, Value};

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
	instruction_steps(pid, dir);
	as_json(pid, dir);
	on_an_object(pid);
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
	let ran = run(pid, &["trace", "/proc/value", "--log", &format!("@{}", message), "--count", "1"], 0);
	assert!(ran.out.contains("value() said \"2\"\n"), "trace --log said {}", ran.out);
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

/// The instruction a pause after a step of one is on: its offset and text,
/// as `dis` has them for the frame's proc
fn instruction_at(printed: &str, proc: &str, pid: &str) -> u32 {
	let mut lines = printed.lines();
	assert_eq!(lines.next(), Some("paused after a step"), "the step said {}", printed);
	let top = lines.next().unwrap_or_default();
	assert!(top.starts_with(&format!("#0 {}  ", proc)), "the step said {}", printed);
	let (offset, text) = lines
		.next()
		.and_then(|line| line.strip_prefix("   instruction "))
		.and_then(|line| line.split_once(": "))
		.unwrap_or_else(|| panic!("the step didn't say its instruction: {}", printed));
	let offset: u32 = offset.parse().unwrap();
	let listing = run(pid, &["dis", proc], 0).out;
	assert!(listing.contains(&format!("\n{:>6}  {}\n", offset, text)), "{} {} isn't in\n{}", offset, text, listing);
	offset
}

/// Steps of one instruction, over and into
fn instruction_steps(pid: &str, dir: &Path) {
	// The world calls it when it finds this file
	std::fs::write(dir.join("call"), "yes").unwrap();
	run(pid, &["wait", "--break", "/proc/fail_in", "--timeout", "60"], 0);
	let printed = run(pid, &["next", "--instruction"], 0).out;
	let mut last = instruction_at(&printed, "/fail_in", pid);
	// A breakpoint on a proc is on the instruction after its first DbgLine,
	// and this is the one after that
	let offsets: Vec<u32> = run(pid, &["dis", "/fail_in"], 0)
		.out
		.lines()
		.filter_map(|line| line.trim_start().split_once("  ").and_then(|(offset, text)| Some((offset.parse().ok()?, text))))
		.skip_while(|(_, text)| !text.starts_with("DbgLine"))
		.map(|(offset, _)| offset)
		.collect();
	assert_eq!(last, offsets[2], "next --instruction went to {} of {:?}", last, offsets);
	println!("ok: next --instruction goes one instruction on, and says which");
	// One at a time, into fail_deeper() when its call comes
	let mut printed = run(pid, &["step", "--instruction"], 0).out;
	for _ in 0..10 {
		if printed.lines().nth(1).is_some_and(|line| line.starts_with("#0 /fail_deeper  ")) {
			break;
		}
		let offset = instruction_at(&printed, "/fail_in", pid);
		assert!(offset > last, "a step went from {} to {}", last, offset);
		last = offset;
		printed = run(pid, &["step", "--instruction"], 0).out;
	}
	instruction_at(&printed, "/fail_deeper", pid);
	// A step of a line still says nothing of instructions
	let printed = run(pid, &["next"], 0).out;
	assert!(printed.starts_with("paused after a step\n#0 /fail_deeper  ") && !printed.contains("instruction"), "next said {}", printed);
	run(pid, &["continue"], 0);
	println!("ok: step --instruction goes into a proc that's called, an instruction at a time");
}

/// One JSON document on stdout, and nothing on stderr
fn json_of(pid: &str, args: &[&str]) -> Value {
	let ran = run(pid, args, 0);
	assert!(ran.err.is_empty(), "`{}` said {} on stderr", args.join(" "), ran.err);
	serde_json::from_str(&ran.out).unwrap_or_else(|e| panic!("`{}` printed {}: {}", args.join(" "), ran.out, e))
}

/// --json, on a running world and a paused one
fn as_json(pid: &str, dir: &Path) {
	let eval = |expression: &str, more: &[&str]| {
		let args: Vec<&str> = ["eval", "--json", expression].iter().chain(more).copied().collect();
		json_of(pid, &args)
	};
	assert_eq!(json_of(pid, &["status", "--json"]), json!({ "paused": false }));
	assert_eq!(eval("list(1, \"two\", null, list(3, list(4)))", &[]), json!([1, "two", null, [3, { "list": true, "len": 1 }]]));
	assert_eq!(eval("list(\"a\" = 1, \"b\")", &[]), json!({ "a": 1, "b": null }));
	assert_eq!(eval("list(1, 2)", &["--depth", "0"]), json!({ "list": true, "len": 2 }));
	assert_eq!(eval("0.1", &[]).to_string(), "0.1");
	assert_eq!(eval("\"text\"", &[]), json!("text"));
	assert_eq!(eval("/datum/twice", &[]), json!({ "path": "/datum/twice" }));
	let made = eval("new /datum/twice", &[]);
	assert!(made["ref"].as_str().is_some_and(|r| r.starts_with("[0x")) && made["type"] == "/datum/twice" && made["name"].is_null(), "{}", made);
	assert!(made["vars"]["type"] == json!({ "path": "/datum/twice" }) && made["vars"].get("vars").is_none(), "{}", made);
	let shut = eval("new /datum/twice", &["--depth", "1"]);
	assert!(shut.get("vars").is_none(), "{}", shut);
	let in_list = eval("list(new /datum/twice)", &[]);
	assert!(in_list[0]["type"] == "/datum/twice" && in_list[0].get("vars").is_none(), "{}", in_list);
	assert_eq!(fails(pid, &["eval", "--json", "1/0"]), "runtime error: Division by zero");
	assert_eq!(fails(pid, &["eval", "--depth", "1", "1"]), "--depth goes with --json");
	let printed = json_of(pid, &["proc", "--json", "/datum/twice/act#1"]);
	assert!(printed["proc"] == "/datum/twice/act#1" && printed["definitions"] == 2 && printed["reloaded"].is_null(), "{}", printed);
	assert_eq!(json_of(pid, &["proc", "--json", "/value"])["reloaded"], "code");
	let printed = json_of(pid, &["dis", "--json", "/value"]);
	assert!(printed["instructions"][0]["text"].as_str().is_some_and(|t| t.starts_with("DbgFile")), "{}", printed);
	println!("ok: eval --json writes values out as JSON on a running world: lists in full to --depth, objects by ref, type and name");

	std::fs::write(dir.join("call"), "yes").unwrap();
	let paused = json_of(pid, &["wait", "--json", "--break", "/proc/fail_in", "--timeout", "60"]);
	assert!(paused["paused"] == true && paused["reason"] == "breakpoint" && paused["already"] == false, "{}", paused);
	assert_eq!(paused["breakpoints"][0]["proc"], "/fail_in");
	assert_eq!(paused["breakpoints"][0]["place"], "/proc/fail_in");
	assert_eq!(paused["stack"][0]["proc"], "/fail_in");
	assert!(paused["instruction"].is_null() && paused["changes"] == json!([]), "{}", paused);
	let again = json_of(pid, &["wait", "--json"]);
	assert!(again["already"] == true && again["stack"] == paused["stack"], "{}", again);
	assert_eq!(json_of(pid, &["status", "--json"])["reason"], "breakpoint");
	let stack = json_of(pid, &["stack", "--json"]);
	assert!(stack[0]["proc"] == "/fail_in" && stack[0]["line"].is_number() && stack[0]["offset"].is_number(), "{}", stack);
	let all = json_of(pid, &["stack", "--json", "--all"]);
	assert!(all[0]["frames"] == stack && all[0]["sleeping"] == false, "{}", all);
	let vars = json_of(pid, &["vars", "--json", "--globals"]);
	assert!(vars["arguments"]["n"] == 1 && vars["locals"]["got"].is_null() && vars["globals"]["rounds"].is_number(), "{}", vars);
	assert_eq!(eval("n + 1", &[]), json!(2));
	let stepped = json_of(pid, &["next", "--instruction", "--json"]);
	assert!(stepped["reason"] == "step" && stepped["instruction"]["text"].is_string(), "{}", stepped);
	run(pid, &["continue"], 0);
	println!("ok: wait, status, stack, vars, eval and next print one JSON document with --json, paused");
}

/// eval --src: an object's vars and procs by their bare names, with no frame
fn on_an_object(pid: &str) {
	let reference = run(pid, &["eval", "ref(held)"], 0).out.trim().trim_matches('"').to_owned();
	assert!(reference.starts_with("[0x") && reference.ends_with(']'), "ref(held) gave {}", reference);
	let bare = reference.trim_start_matches('[').trim_end_matches(']').to_owned();
	for object in [&reference, &bare] {
		assert_eq!(run(pid, &["eval", "--src", object, "count + double(2) + src.count"], 0).out, "10\n");
	}
	assert_eq!(json_of(pid, &["eval", "--src", &reference, "--json", "list(count, type)"]), json!([3, { "path": "/datum/held" }]));
	// Paused, the same, and not in the paused frame
	run(pid, &["pause"], 0);
	assert_eq!(run(pid, &["eval", "--src", &bare, "double(count)"], 0).out, "6\n");
	run(pid, &["continue"], 0);
	println!("ok: eval --src names an object's vars and procs by their bare names, running or paused");

	// One that's gone: the datum is deleted once nothing holds it
	let gone = run(pid, &["eval", "ref(new /datum/held)"], 0).out.trim().trim_matches('"').to_owned();
	assert_eq!(fails(pid, &["eval", "--src", &gone, "count"]), format!("there's no object {}: it was deleted, or never was", gone));
	assert_eq!(
		fails(pid, &["eval", "--src", "held", "count"]),
		"held isn't an object's reference: that's what ref() gives, like [0x2000001]"
	);
	assert_eq!(fails(pid, &["eval", "--src", &bare, "--frame", "0", "count"]), "--src and --frame don't go together: --src is in no frame");
	assert!(fails(pid, &["eval", "--src", &bare, "nothing_called_this"]).starts_with("undefined var: nothing_called_this"));
	println!("ok: eval --src fails on an object that's gone, or a ref that isn't one");
}
