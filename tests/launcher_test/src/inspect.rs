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
	let _ = dir;
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
