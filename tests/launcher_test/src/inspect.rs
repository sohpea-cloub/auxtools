use std::{
	io::Write,
	path::Path,
	process::{Command, Stdio}
};

use serde_json::{json, Value};

use super::{
	client::launcher_path,
	journal::{paused_at, Background},
	line_of
};

struct Ran {
	code: i32,
	out: String,
	err: String
}

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

#[track_caller]
fn run(pid: &str, args: &[&str], code: i32) -> Ran {
	let ran = auxtools(pid, args, None);
	assert_eq!(
		ran.code,
		code,
		"`{}` gave {}, not {}: {}{}",
		args.join(" "),
		ran.code,
		code,
		ran.out,
		ran.err
	);
	ran
}

#[track_caller]
fn fails(pid: &str, args: &[&str]) -> String {
	let ran = run(pid, args, 1);
	assert!(
		ran.out.is_empty(),
		"expected empty stdout from failed command `{}`: {}",
		args.join(" "),
		ran.out
	);
	match ran.err.strip_prefix("error: ") {
		Some(said) => said.trim_end().to_owned(),
		None => panic!("expected error prefix on stderr from `{}`: {:?}", args.join(" "), ran.err)
	}
}

#[track_caller]
fn paused_in_call(pid: &str, dir: &Path, place: &str) -> String {
	paused_at(pid.parse().unwrap(), place, || std::fs::write(dir.join("call"), "yes").unwrap())
}

pub fn evaluations(pid: u64, dir: &Path, code: &Path) {
	let pid = pid.to_string();
	let pid = pid.as_str();
	eval_failures(pid, code);
	from_files(pid, dir);
	procs(pid, code);
}

pub fn breakpoints(pid: u64, dir: &Path, code: &Path) {
	let pid = pid.to_string();
	let pid = pid.as_str();
	where_breakpoints_bound(pid, code);
	instruction_steps(pid, dir);
	as_json(pid, dir);
	on_an_object(pid);
	macros(pid, code);
}

pub fn spawned_and_steps(pid: u64, dir: &Path, code: &Path) {
	let pid = pid.to_string();
	let pid = pid.as_str();
	spawned(pid, code);
	steps_that_end_elsewhere(pid, dir);
}

fn eval_failures(pid: &str, code: &Path) {
	let ran = run(pid, &["eval", "fail_in(3)"], 0);
	assert_eq!(ran.out, "null\n", "unexpected eval output: {}", ran.out);
	let said = ran.err;
	let lines: Vec<&str> = said.lines().collect();
	assert!(
		lines.len() == 4 && lines[0].starts_with("runtime error in a called proc (the evaluation carried on): "),
		"unexpected eval output: {}",
		said
	);
	assert_eq!(lines[1], format!("  in /fail_deeper  attach_host.dm:{}", line_of(code, "\treturn L[n]")));
	assert_eq!(
		lines[2],
		format!("  called by /fail_in  attach_host.dm:{}", line_of(code, "\tvar/got = fail_deeper(n)"))
	);
	assert_eq!(lines[3], "  called by the expression");
	assert_eq!(fails(pid, &["eval", "1/0"]), "runtime error: Division by zero");
	assert_eq!(run(pid, &["eval", "caught_inside()"], 0).out.trim(), "\"caught\"");
	let caught = "try\n\tvar/list/L = null\n\tL.len = 1\ncatch(var/exception/e)\n\treturn \"caught: [e.name]\"";
	assert_eq!(run(pid, &["eval", caught], 0).out.trim(), "\"caught: Cannot modify null.len.\"");
	assert_eq!(
		run(pid, &["eval", "--spawn", "--wait", caught], 0).out.trim(),
		"\"caught: Cannot modify null.len.\""
	);
	assert_eq!(run(pid, &["eval", "/client == client_type()"], 0).out.trim(), "1");
	assert_eq!(run(pid, &["eval", "/datum/twice/proc/act"], 0).out, "/datum/twice/proc/act\n");
	assert_eq!(run(pid, &["eval", "/datum/twice/act"], 0).out, "/datum/twice/act\n");
	println!("ok: eval runtime error call stack");
	println!("ok: eval carries on after a runtime error in a called proc");

	assert_eq!(run(pid, &["eval", "/datum/twice/more::act() == twice_ref()"], 0).out, "1\n");
	assert_eq!(
		run(pid, &["eval", "call(new /datum/twice/more, /datum/twice/more::act())()"], 0).out,
		"\"second\"\n"
	);
	assert_eq!(run(pid, &["eval", "call(new /obj, /obj::atom_act())()"], 0).out, "\"atom\"\n");
	assert_eq!(
		run(pid, &["eval", "call(new /datum/twice, /datum/twice/proc/act)()"], 0).out,
		"\"first\"\n"
	);
	let said = fails(pid, &["eval", "/datum/twice::nope()"]);
	assert!(said.contains("/datum/twice has no proc called nope"), "unexpected eval output: {}", said);
	let said = fails(pid, &["eval", "new /datum/twice{tag = \"x\"}()"]);
	assert!(said.contains("use new and assign the vars"), "unexpected eval output: {}", said);
	println!("ok: eval's type proc references");

	let looped = "var/x = 0; for(var/i in 1 to 2000000) x++; return x";
	let said = fails(pid, &["eval", looped]);
	assert!(
		said.starts_with("evaluation aborted") && said.contains("background") && said.contains("eval --spawn --wait"),
		"unexpected eval output: {}",
		said
	);
	assert_eq!(run(pid, &["eval", "--spawn", "--wait", looped], 0).out, "2000000\n");
	println!("ok: background loop evaluation");

	assert_eq!(run(pid, &["eval", "\"\\improper AI \\bold Chamber\""], 0).out, "\"AI Chamber\"\n");
	let named = json_of(pid, &["eval", "--json", "new /obj/macro_named"]);
	assert!(named["name"] == "AI Chamber" && named["vars"]["name"] == "AI Chamber", "{}", named);
	println!("ok: text macro removal");

	run(pid, &["pause"], 0);
	let ran = run(pid, &["eval", "fail_in(3)"], 0);
	assert!(
		ran.out == "null\n" && ran.err.contains("\n  in /fail_deeper  ") && ran.err.ends_with("\n  called by the expression\n"),
		"unexpected eval output: {}{}",
		ran.out,
		ran.err
	);
	for (args, said) in [
		(&["eval", "1 +"][..], "parser error: "),
		(&["eval", "nothing_called_this"][..], "undefined var: nothing_called_this"),
		(&["eval", "--frame", "9", "1"][..], "frame 9 does not exist: the paused stack has "),
		(&["eval"][..], "eval requires an expression: EXPRESSION, --file FILE, or - for stdin.")
	] {
		let got = fails(pid, args);
		assert!(got.starts_with(said), "unexpected output from `{}`: {}", args.join(" "), got);
	}
	run(pid, &["continue"], 0);
	println!("ok: consistent eval failure output");
}

fn from_files(pid: &str, dir: &Path) {
	let file = |name: &str, text: &str| {
		let path = dir.join(name);
		std::fs::write(&path, text).unwrap();
		path.to_str().unwrap().to_owned()
	};
	let expression = file("expression.txt", "\"[\"a b\"] /datum/x\" + \"!\"\n");
	assert_eq!(run(pid, &["eval", "-f", &expression], 0).out.trim(), "\"a b /datum/x!\"");
	assert_eq!(run(pid, &["eval", "--file", &expression], 0).out.trim(), "\"a b /datum/x!\"");
	let ran = auxtools(pid, &["eval", "-"], Some("6 * 7\n"));
	assert_eq!((ran.code, ran.out.trim()), (0, "42"), "unexpected eval - output: {}{}", ran.out, ran.err);
	let said = fails(pid, &["eval", "-f", "no such file.txt"]);
	assert!(said.starts_with("cannot read no such file.txt: "), "unexpected eval -f output: {}", said);
	assert_eq!(
		fails(pid, &["eval", "1", "-f", &expression]),
		"EXPRESSION and --file cannot be used together."
	);
	println!("ok: file and stdin evaluation");

	let never = file("never.txt", "1 == 2\n");
	let ran = run(
		pid,
		&[
			"wait",
			"pause",
			"--break",
			"/proc/value",
			"--if",
			&format!("@{}", never),
			"--timeout",
			"1"
		],
		2
	);
	assert!(ran.err.contains("timed out"), "unexpected wait output: {}{}", ran.out, ran.err);
	let message = file("message.txt", "value() said \"{1 + 1}\"\n");
	let ran = run(pid, &["trace", "/proc/value", "--log", &format!("@{}", message), "--count", "1"], 0);
	assert!(ran.out.contains("value() said \"2\"\n"), "unexpected trace --log output: {}", ran.out);
	let watched = file("watched.txt", "rounds");
	let ran = run(pid, &["wait", "pause", "--change", &format!("@{}", watched), "--timeout", "60"], 0);
	assert!(
		ran.out.starts_with("watching rounds, now "),
		"unexpected wait --change output: {}",
		ran.out
	);
	run(pid, &["continue"], 0);
	let said = fails(pid, &["wait", "pause", "--break", "/proc/value", "--if", &format!("@@{}", never)]);
	assert!(
		said.starts_with("cannot set a breakpoint at /proc/value: the condition doesn't compile"),
		"unexpected wait output: {}",
		said
	);
	let said = fails(pid, &["wait", "pause", "--break", "/proc/value", "--if", "@no such file.txt"]);
	assert!(said.starts_with("cannot read no such file.txt: "), "unexpected wait output: {}", said);
	println!("ok: file-backed wait arguments and @ escaping");
}

fn procs(pid: &str, code: &Path) {
	run(pid, &["eval", "new /datum/inits"], 0);
	let printed = run(pid, &["proc", "/datum/inits/(init)"], 0).out;
	assert!(
		printed.starts_with("/datum/inits/(init)") && printed.contains("definitions: 1"),
		"proc said {}",
		printed
	);
	println!("ok: procs named after startup");
	run(pid, &["eval", "auxtools_kept()[\"kept\"] = new /datum/twice"], 0);
	assert_eq!(run(pid, &["eval", "auxtools_kept()[\"kept\"]:type"], 0).out, "/datum/twice\n");
	println!("ok: values kept between evaluations");
	for (marker, value) in [("\t\ttotal += i // first loop", "1"), ("\t\ttotal += i // second loop", "10")] {
		let place = format!("attach_host.dm:{}", line_of(code, marker));
		paused_at(pid.parse().unwrap(), &place, || {
			run(pid, &["eval", "--spawn", "two_loops()"], 0);
		});
		let vars = run(pid, &["vars"], 0).out;
		assert_eq!(vars.matches("  i = ").count(), 1, "vars said {}", vars);
		assert!(vars.contains(&format!("  i = {}\n", value)), "vars said {}", vars);
		assert_eq!(run(pid, &["eval", "i"], 0).out.trim(), value);
		run(pid, &["continue"], 0);
	}
	println!("ok: a local's name means the one in use");
	let printed = run(pid, &["proc", "/proc/value"], 0).out;
	assert_eq!(
		printed,
		format!(
			"/value  attach_host.dm:{}\ndefinitions: 1\nreloaded: code (new code)\n",
			line_of(code, "\treturn 2")
		)
	);
	let printed = run(pid, &["proc", "/fail_in"], 0).out;
	assert!(printed.ends_with("\nreloaded: no\n"), "unexpected proc output: {}", printed);
	let printed = run(pid, &["proc", "/datum/twice/act#1"], 0).out;
	assert_eq!(
		printed,
		format!(
			"/datum/twice/act#1  attach_host.dm:{}\ndefinitions: 2\nreloaded: no\n",
			line_of(code, "\t. = ..()")
		)
	);
	let printed = run(pid, &["proc", "/datum/twice/proc/act"], 0).out;
	assert!(
		printed.starts_with(&format!("/datum/twice/act  attach_host.dm:{}\n", line_of(code, "\treturn \"first\""))),
		"unexpected proc output: {}",
		printed
	);
	for (args, said) in [
		(
			&["proc", "/proc/nothing_called_this"][..],
			"the world has no proc /proc/nothing_called_this"
		),
		(&["proc", "/value#3"][..], "/value has one definition, #0: there's no #3"),
		(
			&["proc", "/datum/twice/act#2"][..],
			"/datum/twice/act has 2 definitions, #0 to #1: there's no #2"
		),
		(&["proc", "value"][..], "invalid proc path 'value': expected /PATH[#N]."),
		(
			&["proc", "/value#x"][..],
			"invalid definition index 'x' in '/value#x': expected a non-negative integer."
		),
		(
			&["proc"][..],
			"missing required argument: /PATH[#N]. Run 'auxtools proc --help' for usage."
		),
		(
			&["disassemble", "/proc/nothing_called_this"][..],
			"the world has no proc /proc/nothing_called_this"
		)
	] {
		assert_eq!(fails(pid, args), said, "`{}`", args.join(" "));
	}
	println!("ok: proc metadata and reload status");

	let printed = run(pid, &["disassemble", "/datum/twice/act#1"], 0).out;
	let (first, second) = (line_of(code, "\t. = ..()"), line_of(code, "\treturn \"second\""));
	let expected = format!(
		"/datum/twice/act#1  attach_host.dm\nline {first}:\n     0  DbgFile \"attach_host.dm\"\n     2  DbgLine {first}\n     4  CallParent\n     5  SetVar \
		 dot\nline {second}:\n     7  DbgLine {second}\n     9  PushVal \"second\"\n    12  Ret\n    13  End\n"
	);
	assert_eq!(printed, expected);
	println!("ok: bytecode offsets and source lines");
}

fn where_breakpoints_bound(pid: &str, code: &Path) {
	let declared = line_of(code, "/proc/value()");
	let ran = run(
		pid,
		&["wait", "pause", "--break", &format!("attach_host.dm:{}", declared), "--timeout", "60"],
		0
	);
	assert!(
		ran.out.starts_with(&format!("pid {}: paused at a breakpoint\n#0 /value", pid)),
		"unexpected wait output: {}",
		ran.out
	);
	assert_eq!(
		ran.err,
		format!(
			"breakpoint set at /value  attach_host.dm:{} (line {} has no code; this is the next line that has)\n",
			declared + 1,
			declared
		)
	);
	run(pid, &["continue"], 0);
	let ran = run(
		pid,
		&[
			"wait",
			"pause",
			"--break",
			"/proc/fail_in",
			"--break",
			"/datum/twice/act#1",
			"--timeout",
			"1"
		],
		2
	);
	assert!(ran.out.is_empty(), "a wait that timed out printed {}", ran.out);
	assert_eq!(
		ran.err,
		format!(
			"breakpoint set at /fail_in  attach_host.dm:{}\nbreakpoint set at /datum/twice/act#1  attach_host.dm:{}\nerror: timed out after 1 \
			 second waiting for the world to pause.\n",
			line_of(code, "\tvar/got = fail_deeper(n)"),
			line_of(code, "\t. = ..()")
		)
	);
	println!("ok: resolved wait breakpoint locations");

	let info = run(pid, &["proc", "(global init)"], 0).out;
	assert!(
		info.starts_with("(global init)  attach_host.dm:") && info.ends_with("\ndefinitions: 1\nreloaded: no\n"),
		"unexpected proc output: {}",
		info
	);
	let cached = line_of(code, "\tvar/static/list/cache = list()");
	let ran = run(
		pid,
		&["wait", "pause", "--break", &format!("attach_host.dm:{}", cached), "--timeout", "1"],
		2
	);
	assert_eq!(
		ran.err,
		format!(
			"breakpoint set at (global init)  attach_host.dm:{}: this line initializes globals or static vars only at world startup\nerror: timed \
			 out after 1 second waiting for the world to pause.\n",
			cached
		)
	);
	println!("ok: initializer proc names");

	let last = line_of(code, "\treturn \"last [rounds]\"");
	let nowhere = [
		(
			"nowhere.dm:3",
			"cannot set a breakpoint at nowhere.dm:3: the world has no code from a file named nowhere.dm.".to_owned()
		),
		(
			"attach_host.dm:9999",
			format!(
				"cannot set a breakpoint at attach_host.dm:9999: attach_host.dm has no code after line {}.",
				last
			)
		),
		(
			"/proc/nothing_called_this",
			"cannot set a breakpoint at /proc/nothing_called_this: the world has no proc /proc/nothing_called_this".to_owned()
		),
		(
			"/value#2",
			"cannot set a breakpoint at /value#2: /value has one definition, #0: there's no #2".to_owned()
		)
	];
	for (place, said) in &nowhere {
		assert_eq!(&fails(pid, &["wait", "pause", "--break", place]), said);
	}
	run(pid, &["pause"], 0);
	for (place, said) in &nowhere {
		assert_eq!(&fails(pid, &["wait", "pause", "--break", place]), said);
	}
	run(pid, &["continue"], 0);
	println!("ok: invalid wait target diagnostics");
}

#[track_caller]
fn instruction_at(printed: &str, proc: &str, pid: &str) -> u32 {
	let mut lines = printed.lines();
	assert_eq!(
		lines.next(),
		Some(format!("pid {}: paused after a step", pid).as_str()),
		"unexpected step output: {}",
		printed
	);
	let top = lines.next().unwrap_or_default();
	assert!(top.starts_with(&format!("#0 {}  ", proc)), "unexpected step output: {}", printed);
	let instruction = lines.next().and_then(|line| line.strip_prefix("   instruction "));
	let Some((offset, text)) = instruction.and_then(|line| line.split_once(": ")) else {
		panic!("missing instruction in step output: {}", printed)
	};
	let offset: u32 = offset.parse().unwrap();
	let listing = run(pid, &["disassemble", proc], 0).out;
	assert!(
		listing.contains(&format!("\n{:>6}  {}\n", offset, text)),
		"{} {} isn't in\n{}",
		offset,
		text,
		listing
	);
	offset
}

fn instruction_steps(pid: &str, dir: &Path) {
	paused_in_call(pid, dir, "/proc/fail_in");
	let printed = run(pid, &["step-over", "--instruction"], 0).out;
	let mut last = instruction_at(&printed, "/fail_in", pid);
	let offsets: Vec<u32> = run(pid, &["disassemble", "/fail_in"], 0)
		.out
		.lines()
		.filter_map(|line| {
			line.trim_start()
				.split_once("  ")
				.and_then(|(offset, text)| Some((offset.parse().ok()?, text)))
		})
		.skip_while(|(_, text)| !text.starts_with("DbgLine"))
		.map(|(offset, _)| offset)
		.collect();
	assert_eq!(last, offsets[2], "step-over --instruction went to {} of {:?}", last, offsets);
	println!("ok: instruction Step Over");
	let mut printed = run(pid, &["step-in", "--instruction"], 0).out;
	for _ in 0..10 {
		if printed.lines().nth(1).is_some_and(|line| line.starts_with("#0 /fail_deeper  ")) {
			break;
		}
		let offset = instruction_at(&printed, "/fail_in", pid);
		assert!(offset > last, "a step went from {} to {}", last, offset);
		last = offset;
		printed = run(pid, &["step-in", "--instruction"], 0).out;
	}
	instruction_at(&printed, "/fail_deeper", pid);
	let printed = run(pid, &["step-over"], 0).out;
	assert!(
		printed.starts_with(&format!("pid {}: paused after a step\n#0 /fail_deeper  ", pid)) && !printed.contains("instruction"),
		"step-over said {}",
		printed
	);
	run(pid, &["continue"], 0);
	println!("ok: instruction Step Into");
}

#[track_caller]
fn json_of(pid: &str, args: &[&str]) -> Value {
	let ran = run(pid, args, 0);
	assert!(ran.err.is_empty(), "unexpected stderr from `{}`: {}", args.join(" "), ran.err);
	match serde_json::from_str(&ran.out) {
		Ok(value) => value,
		Err(e) => panic!("`{}` printed {}: {}", args.join(" "), ran.out, e)
	}
}

fn as_json(pid: &str, dir: &Path) {
	let eval = |expression: &str, more: &[&str]| {
		let args: Vec<&str> = ["eval", "--json", expression].iter().chain(more).copied().collect();
		json_of(pid, &args)
	};
	let number: u64 = pid.parse().unwrap();
	let status = json_of(pid, &["status", "--json"]);
	for (key, value) in [
		("pid", json!(number)),
		("paused", json!(false)),
		("reason", json!(null)),
		("message", json!(null)),
		("caught", json!(null)),
		("changes", json!([])),
		("stack", json!([])),
		("instruction", json!(null))
	] {
		assert_eq!(status[key], value, "{} of {}", key, status);
	}
	assert!(
		status["uptime_ms"].as_u64().is_some_and(|up| up > 0) && status["world_time"].is_number() && status["loads"] == 1,
		"{}",
		status
	);
	assert_eq!(status["last_reload"]["kind"], "reload", "{}", status);
	assert_eq!(status.as_object().unwrap().len(), 12, "{}", status);
	assert_eq!(
		eval("list(1, \"two\", null, list(3, list(4)))", &[]),
		json!([1, "two", null, [3, { "list": true, "len": 1 }]])
	);
	assert_eq!(eval("list(\"a\" = 1, \"b\")", &[]), json!({ "a": 1, "b": null }));
	assert_eq!(eval("list(1, 2)", &["--depth", "0"]), json!({ "list": true, "len": 2 }));
	assert_eq!(eval("0.1", &[]).to_string(), "0.1");
	assert_eq!(eval("\"text\"", &[]), json!("text"));
	assert_eq!(eval("/datum/twice", &[]), json!({ "path": "/datum/twice" }));
	let made = eval("new /datum/twice", &[]);
	assert!(
		made["ref"].as_str().is_some_and(|r| r.starts_with("[0x")) && made["type"] == "/datum/twice" && made["name"].is_null(),
		"{}",
		made
	);
	assert!(
		made["vars"]["type"] == json!({ "path": "/datum/twice" }) && made["vars"].get("vars").is_none(),
		"{}",
		made
	);
	let shut = eval("new /datum/twice", &["--depth", "1"]);
	assert!(shut.get("vars").is_none(), "{}", shut);
	let in_list = eval("list(new /datum/twice)", &[]);
	assert!(in_list[0]["type"] == "/datum/twice" && in_list[0].get("vars").is_none(), "{}", in_list);
	assert_eq!(fails(pid, &["eval", "--json", "1/0"]), "runtime error: Division by zero");
	assert_eq!(fails(pid, &["eval", "--depth", "1", "1"]), "--depth requires --json.");
	let printed = json_of(pid, &["proc", "--json", "/datum/twice/act#1"]);
	assert!(
		printed["proc"] == "/datum/twice/act#1" && printed["definitions"] == 2 && printed["reloaded"].is_null(),
		"{}",
		printed
	);
	assert_eq!(json_of(pid, &["proc", "--json", "/value"])["reloaded"], "code");
	let printed = json_of(pid, &["disassemble", "--json", "/value"]);
	assert!(
		printed["instructions"][0]["text"].as_str().is_some_and(|t| t.starts_with("DbgFile")),
		"{}",
		printed
	);
	println!("ok: recursive JSON rendering");

	let mut waiting = Background::start(number, &["wait", "pause", "--json", "--break", "/proc/fail_in", "--timeout", "60"]);
	waiting.error_with("breakpoint set at ");
	std::fs::write(dir.join("call"), "yes").unwrap();
	let (code, out, err) = waiting.finish_apart();
	assert_eq!(code, 0, "unexpected wait output: {}{}", out, err);
	assert!(
		err.starts_with("breakpoint set at /fail_in  attach_host.dm:") && err.lines().count() == 1,
		"unexpected wait output: {}",
		err
	);
	let paused: Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("wait pause --json printed {}: {}", out, e));
	assert!(
		paused["paused"] == true && paused["reason"] == "breakpoint" && paused["already"] == false && paused["pid"] == number,
		"{}",
		paused
	);
	assert_eq!(paused.as_object().unwrap().len(), 11, "{}", paused);
	assert_eq!(paused["breakpoints"][0]["proc"], "/fail_in");
	assert_eq!(paused["breakpoints"][0]["place"], "/proc/fail_in");
	assert_eq!(paused["stack"][0]["proc"], "/fail_in");
	assert!(paused["instruction"].is_null() && paused["changes"] == json!([]), "{}", paused);
	let again = json_of(pid, &["wait", "pause", "--json"]);
	assert!(again["already"] == true && again["stack"] == paused["stack"], "{}", again);
	let status = json_of(pid, &["status", "--json"]);
	assert!(
		status["reason"] == "breakpoint" && status["stack"] == paused["stack"] && status["uptime_ms"].is_number(),
		"{}",
		status
	);
	let held = json_of(pid, &["pause", "--json"]);
	assert!(
		held["paused"] == true && held["reason"] == "breakpoint" && held.as_object().unwrap().len() == 8,
		"{}",
		held
	);
	let stack = json_of(pid, &["stack", "--json"]);
	assert!(
		stack[0]["proc"] == "/fail_in" && stack[0]["line"].is_number() && stack[0]["offset"].is_number(),
		"{}",
		stack
	);
	let all = json_of(pid, &["stack", "--json", "--all"]);
	assert!(all[0]["frames"] == stack && all[0]["sleeping"] == false, "{}", all);
	let vars = json_of(pid, &["vars", "--json", "--globals"]);
	assert!(
		vars["arguments"]["n"] == 1 && vars["locals"]["got"].is_null() && vars["globals"]["rounds"].is_number(),
		"{}",
		vars
	);
	assert_eq!(eval("n + 1", &[]), json!(2));
	let stepped = json_of(pid, &["step-over", "--instruction", "--json"]);
	assert!(
		stepped["reason"] == "step" && stepped["instruction"]["text"].is_string() && stepped["pid"] == number,
		"{}",
		stepped
	);
	run(pid, &["continue"], 0);
	println!("ok: JSON results while paused");
}

fn on_an_object(pid: &str) {
	let reference = run(pid, &["eval", "ref(held)"], 0).out.trim().trim_matches('"').to_owned();
	assert!(reference.starts_with("[0x") && reference.ends_with(']'), "ref(held) gave {}", reference);
	let bare = reference.trim_start_matches('[').trim_end_matches(']').to_owned();
	for object in [&reference, &bare] {
		assert_eq!(run(pid, &["eval", "--src", object, "count + double(2) + src.count"], 0).out, "10\n");
	}
	assert_eq!(
		json_of(pid, &["eval", "--src", &reference, "--json", "list(count, type)"]),
		json!([3, { "path": "/datum/held" }])
	);
	run(pid, &["pause"], 0);
	assert_eq!(run(pid, &["eval", "--src", &bare, "double(count)"], 0).out, "6\n");
	run(pid, &["continue"], 0);
	println!("ok: object-scoped evaluation");

	let gone = run(pid, &["eval", "ref(new /datum/held)"], 0).out.trim().trim_matches('"').to_owned();
	assert_eq!(fails(pid, &["eval", "--src", &gone, "count"]), format!("object not found: {}", gone));
	assert_eq!(
		fails(pid, &["eval", "--src", "held", "count"]),
		"invalid object reference held; expected a ref() value such as [0x2000001]"
	);
	assert_eq!(
		fails(pid, &["eval", "--src", &bare, "--frame", "0", "count"]),
		"--frame and --src cannot be used together."
	);
	assert!(fails(pid, &["eval", "--src", &bare, "nothing_called_this"]).starts_with("undefined var: nothing_called_this"));
	println!("ok: invalid object reference diagnostics");
}

#[track_caller]
fn eval_becomes(pid: &str, expression: &str, wanted: &str) {
	let mut got = String::new();
	for _ in 0..100 {
		got = auxtools(pid, &["eval", expression], None).out;
		if got.trim() == wanted {
			return;
		}
		std::thread::sleep(std::time::Duration::from_millis(100));
	}
	panic!("eval {} gave {}, not {}", expression, got, wanted);
}

fn macros(pid: &str, code: &Path) {
	assert_eq!(run(pid, &["eval", "DOUBLE_IT(ANSWER) + 1"], 0).out, "85\n");
	assert_eq!(run(pid, &["eval", "\"[ANSWER]!\""], 0).out, "\"42!\"\n");
	assert!(fails(pid, &["eval", "GONE"]).starts_with("undefined var: GONE"));
	let reference = run(pid, &["eval", "ref(held)"], 0).out.trim().trim_matches('"').to_owned();
	assert_eq!(run(pid, &["eval", "--src", &reference, "DOUBLE_IT(count)"], 0).out, "6\n");
	println!("ok: source macro expansion and undefinition");

	let ran = run(
		pid,
		&[
			"wait",
			"pause",
			"--break",
			"/proc/tick",
			"--if",
			"n == DOUBLE_IT(1) && ANSWER == 42",
			"--timeout",
			"60"
		],
		0
	);
	assert!(
		ran.out.starts_with(&format!("pid {}: paused at a breakpoint\n#0 /tick", pid)),
		"unexpected wait output: {}",
		ran.out
	);
	assert_eq!(run(pid, &["eval", "DOUBLE_IT(n)"], 0).out, "4\n");
	run(pid, &["continue"], 0);
	let ran = run(
		pid,
		&[
			"trace",
			"/proc/tick",
			"--if",
			"n == 1",
			"--log",
			"doubled: {DOUBLE_IT(n)}",
			"--count",
			"1"
		],
		0
	);
	assert!(ran.out.contains("doubled: 2\n"), "unexpected trace output: {}", ran.out);
	println!("ok: macros in conditions, logpoints and frame evaluation");

	let source = std::fs::read_to_string(code).unwrap();
	std::fs::write(code, source.replace("#define ANSWER 42", "#define ANSWER 43\n#define GONE 2")).unwrap();
	eval_becomes(pid, "ANSWER + GONE", "45");
	std::fs::write(code, &source).unwrap();
	eval_becomes(pid, "ANSWER", "42");
	assert!(fails(pid, &["eval", "GONE"]).starts_with("undefined var: GONE"));
	println!("ok: macro updates after reload");
}

fn spawned(pid: &str, code: &Path) {
	let number = run(pid, &["eval", "--spawn", "slow_double(4)"], 0).out;
	let number: u64 = number
		.trim()
		.strip_prefix("spawn ")
		.and_then(|number| number.parse().ok())
		.unwrap_or_else(|| panic!("eval --spawn printed {}", number));
	assert_eq!(json_of(pid, &["eval", "--spawn", "--json", "1"]), json!({ "spawn": number + 1 }));
	let mut entry = Value::Null;
	for _ in 0..100 {
		let printed = run(pid, &["log", "--spawned", "--last", "2", "--json"], 0).out;
		entry = printed
			.lines()
			.filter_map(|line| serde_json::from_str::<Value>(line).ok())
			.find(|entry| entry["spawn"] == number)
			.unwrap_or_default();
		if entry["spawn"] == number {
			break;
		}
		std::thread::sleep(std::time::Duration::from_millis(100));
	}
	assert!(
		entry["spawn"] == number && entry["value"] == 8 && entry["expression"] == "slow_double(4)",
		"{}",
		entry
	);
	let printed = run(pid, &["log", "--spawned", "--last", "2"], 0).out;
	assert!(
		printed.contains(&format!(" spawn {}: slow_double(4) returned 8\n", number)),
		"log said {}",
		printed
	);
	println!("ok: spawn ID and journal result");

	assert_eq!(run(pid, &["eval", "--spawn", "--wait", "slow_double(5)"], 0).out, "10\n");
	let reference = run(pid, &["eval", "ref(held)"], 0).out.trim().trim_matches('"').to_owned();
	assert_eq!(
		run(
			pid,
			&["eval", "--spawn", "--wait", "--src", &reference, "double(count) + DOUBLE_IT(1)"],
			0
		)
		.out,
		"8\n"
	);
	let both = json_of(pid, &["eval", "--spawn", "--wait", "--json", "list(slow_double(1), held)"]);
	assert!(
		both[0] == 2 && both[1]["type"] == "/datum/held" && both[1].get("vars").is_none(),
		"{}",
		both
	);
	println!("ok: wait for spawned evaluation result");

	let body = "var/n = slow_double(2)\nsleep(1)\nvar/list/both = list(n, n + 1)\nboth[2]";
	assert_eq!(run(pid, &["eval", "--spawn", "--wait", body], 0).out, "5\n");
	assert_eq!(run(pid, &["eval", "--spawn", "--wait", "slow_double(5)"], 0).out, "10\n");
	println!("ok: spawned body variables survive sleep");

	let build: u32 = run(pid, &["eval", "world.byond_build"], 0).out.trim().parse().unwrap();
	let ran = run(pid, &["eval", "var/a = 1\nreturn pick(1; a, 0; a)"], 0);
	assert_eq!(ran.out, "1\n", "unexpected eval output: {}", ran.out);
	if build < 1689 {
		assert!(
			ran.err.starts_with("warning: line 2: pick() ends with a zero-weight entry"),
			"unexpected eval output: {}",
			ran.err
		);
	} else {
		assert_eq!(ran.err, "", "unexpected eval output");
	}
	println!("ok: evaluation compiler warnings");

	let ran = run(pid, &["eval", "--spawn", "--wait", "fail_in(3)"], 0);
	assert_eq!(ran.out, "null\n", "unexpected eval output: {}", ran.out);
	assert!(
		ran.err.starts_with("runtime error in a called proc (the evaluation carried on): ")
			&& ran.err.contains("\n  in /fail_deeper  ")
			&& ran.err.ends_with("\n  called by the expression\n"),
		"unexpected eval output: {}",
		ran.err
	);
	let ran = run(pid, &["eval", "--spawn", "--wait", "slow_fail()"], 0);
	assert_eq!(ran.out, "null\n", "unexpected eval output: {}", ran.out);
	let said = ran.err;
	let lines: Vec<&str> = said.lines().collect();
	assert!(
		lines.len() == 4 && lines[0].starts_with("runtime error in a called proc (the evaluation carried on): "),
		"unexpected eval output: {}",
		said
	);
	assert_eq!(lines[1], format!("  in /fail_deeper  attach_host.dm:{}", line_of(code, "\treturn L[n]")));
	assert_eq!(
		lines[2],
		format!("  called by /slow_fail  attach_host.dm:{}", line_of(code, "\treturn fail_deeper(5)"))
	);
	let printed = run(pid, &["log", "--spawned", "--last", "1"], 0).out;
	assert!(printed.contains(": slow_fail() returned null"), "log said {}", printed);
	let printed = run(pid, &["log", "--runtimes", "--last", "1"], 0).out;
	let frames: Vec<&str> = printed.lines().skip(1).collect();
	assert!(
		frames.len() == 3
			&& frames[0].starts_with("  /fail_deeper  ")
			&& frames[1].starts_with("  /slow_fail  ")
			&& frames[2].starts_with("  /auxtools_spawn_stub  "),
		"log said {}",
		printed
	);
	assert_eq!(
		fails(pid, &["eval", "--spawn", "--wait", "sleep(1)\nvar/n = 0\nreturn 1/n"]),
		"runtime error: Division by zero"
	);
	println!("ok: spawned runtime error stacks");

	let printed = paused_at(pid.parse().unwrap(), "/proc/slow_double", || {
		run(pid, &["eval", "--spawn", "slow_double(6)"], 0);
	});
	assert!(
		printed.contains(&format!("pid {}: paused at a breakpoint\n#0 /slow_double  ", pid)),
		"unexpected wait output: {}",
		printed
	);
	let stack = run(pid, &["stack"], 0).out;
	assert!(
		stack.lines().count() == 2 && stack.lines().nth(1).is_some_and(|line| line.starts_with("#1 /auxtools_spawn_stub")),
		"stack said {}",
		stack
	);
	let stacks = run(pid, &["stack", "--all"], 0).out;
	assert!(
		!stacks.contains("auxtools_keepalive") && !stacks.contains("auxtools_spawned"),
		"stack --all said {}",
		stacks
	);
	assert_eq!(run(pid, &["eval", "n"], 0).out, "6\n");
	let stepped = run(pid, &["step-out", "--timeout", "10"], 0).out;
	assert!(stepped.contains("#0 /auxtools_spawn_stub  "), "step-out said {}", stepped);
	let stepped = run(pid, &["step-out", "--timeout", "10"], 0);
	assert!(
		stepped.err.contains("no frame remained to pause in") && stepped.out.ends_with(": running\n"),
		"step-out said {}{}",
		stepped.out,
		stepped.err
	);
	println!("ok: breakpoints in spawned calls");

	let line = line_of(code, "\treturn \"last [rounds]\"");
	let printed = run(pid, &["disassemble", "/proc/last_in_file"], 0).out;
	assert!(printed.contains("attach_host.dme\""), "missing .dme debug marker: {}", printed);
	let place = format!("/last_in_file  attach_host.dm:{}", line);
	let printed = paused_at(pid.parse().unwrap(), &format!("attach_host.dm:{}", line), || {
		run(pid, &["eval", "--spawn", "last_in_file()"], 0);
	});
	assert!(
		printed.contains(&format!("breakpoint set at {}\n", place)) && printed.contains(&format!("paused at a breakpoint\n#0 {}\n", place)),
		"unexpected wait output: {}",
		printed
	);
	assert!(run(pid, &["stack"], 0).out.starts_with(&format!("#0 {}\n", place)));
	let stepped = run(pid, &["step-over", "--timeout", "10"], 0).out;
	assert!(
		stepped.contains("paused after a step\n#0 /auxtools_spawn_stub  "),
		"step-over said {}",
		stepped
	);
	run(pid, &["continue"], 0);
	println!("ok: breakpoints in the last proc of an included file");

	let ran = run(pid, &["eval", "--spawn", "--wait", "--timeout", "0.5", "slow_double(1, 50)"], 2);
	assert!(
		ran.out.is_empty()
			&& ran.err.starts_with("error: timed out after 0.5 seconds waiting for spawn ")
			&& ran.err.trim_end().ends_with(" to finish. It keeps running."),
		"unexpected eval output: {}{}",
		ran.out,
		ran.err
	);
	assert!(fails(pid, &["eval", "--spawn", "1 +"]).starts_with("parser error: "));
	assert!(fails(pid, &["eval", "--spawn", "--src", "[0x2100ffff]", "1"]).starts_with("object not found: [0x2100ffff]"));
	assert_eq!(fails(pid, &["eval", "--wait", "1"]), "--wait requires --spawn.");
	assert_eq!(fails(pid, &["eval", "--spawn", "--timeout", "1", "1"]), "--timeout requires --wait.");
	assert_eq!(
		fails(pid, &["eval", "--spawn", "--frame", "0", "1"]),
		"--frame and --spawn cannot be used together."
	);
	println!("ok: spawn timeout and rejection diagnostics");
}

fn steps_that_end_elsewhere(pid: &str, dir: &Path) {
	let number: u64 = pid.parse().unwrap();
	paused_in_call(pid, dir, "/proc/fail_in");
	let out = run(pid, &["step-out", "--timeout", "30"], 0).out;
	assert!(
		out.starts_with(&format!("pid {}: paused after a step\n#0 /run_until_stopped  ", pid)),
		"step-out said {}",
		out
	);
	let ran = run(pid, &["step-over", "--timeout", "30", "--json"], 0);
	let state: Value = serde_json::from_str(&ran.out).unwrap_or_else(|e| panic!("step-over --json printed {}: {}", ran.out, e));
	assert!(
		state["pid"] == number && state["paused"] == false && state["reason"].is_null() && state["stack"] == json!([]),
		"{}",
		state
	);
	assert_eq!(ran.err, "step completed with the world running: no frame remained to pause in.\n");
	assert_eq!(json_of(pid, &["status", "--json"])["paused"], false);
	println!("ok: step exhaustion succeeds with world running");

	paused_in_call(pid, dir, "/proc/fail_deeper");
	let ran = run(pid, &["step-over", "--timeout", "30"], 0);
	assert!(ran.err.is_empty(), "step-over said {}", ran.err);
	let ran = run(pid, &["step-over", "--timeout", "30"], 0);
	assert!(
		ran.out.starts_with(&format!("pid {}: paused after a step\n#0 /fail_in  ", pid)),
		"step-over said {}",
		ran.out
	);
	assert_eq!(ran.err, "the step ended /fail_deeper with a runtime error: list index out of bounds\n");
	run(pid, &["continue"], 0);
	println!("ok: a step reports the runtime error that ended its proc");

	paused_at(number, "/proc/slow_double", || {
		run(pid, &["eval", "--spawn", "slow_double(6, 40)"], 0);
	});
	let on_sleep = run(pid, &["step-over", "--timeout", "30", "--json"], 0).out;
	assert!(
		on_sleep.contains("\"reason\":\"step\"") && on_sleep.contains("\"proc\":\"/slow_double\""),
		"step-over said {}",
		on_sleep
	);
	let started = std::time::Instant::now();
	let ran = run(pid, &["step-over", "--timeout", "1", "--json"], 2);
	assert!(
		started.elapsed() < std::time::Duration::from_secs(4),
		"a step with --timeout 1 took {:?}",
		started.elapsed()
	);
	let state: Value = serde_json::from_str(&ran.out).unwrap_or_else(|e| panic!("step-over --json printed {}: {}", ran.out, e));
	assert!(state["pid"] == number && state["paused"] == false, "{}", state);
	assert!(
		ran.err.starts_with("error: timed out after 1 second waiting for the step to complete."),
		"step-over said {}",
		ran.err
	);
	paused_at(number, "/proc/slow_double", || {
		run(pid, &["eval", "--spawn", "slow_double(7, 20)"], 0);
	});
	run(pid, &["step-over", "--timeout", "30"], 0);
	let ran = run(pid, &["step-over", "--timeout", "0.5"], 2);
	assert_eq!(ran.out, format!("pid {}: running\n", pid));
	assert!(
		ran.err
			.starts_with("error: timed out after 0.5 seconds waiting for the step to complete."),
		"step-over said {}",
		ran.err
	);
	std::thread::sleep(std::time::Duration::from_millis(3500));
	assert_eq!(json_of(pid, &["status", "--json"])["paused"], false);
	println!("ok: step timeout preserves execution state");
}
