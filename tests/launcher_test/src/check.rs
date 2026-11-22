use std::path::PathBuf;

use serde_json::{json, Value};
use test_support::cli;

const CODE: &str = "\
#ifdef LOOPS
/proc/caught(list/a, list/b)
\tfor (var/x in a)
\t\ttry
\t\t\tfor (var/y in b)
\t\t\t\tx += y
\t\tcatch
#endif
/datum/num/proc/operator~=(b)
\treturn 1
/proc/refused()
\treturn &5
";

fn project() -> PathBuf {
	let dir = std::env::temp_dir().join(format!("launcher_test_check-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	std::fs::write(dir.join("world.dme"), CODE).unwrap();
	dir
}

fn check(dir: &PathBuf, args: &[&str]) -> (i32, String, String) {
	let ran = cli::ran(cli::auxtools(["check"]).args(args).current_dir(dir), None);
	(ran.code, ran.out, ran.err)
}

pub fn checks() {
	let dir = project();

	let (code, out, err) = check(&dir, &["--byond-version", "516.1689"]);
	assert_eq!(
		(code, out.as_str(), err.as_str()),
		(
			0,
			"nothing found in 1 proc for BYOND 516.1689; 1 proc not checked\n",
			"not checked: world.dme: /refused: line 12: expected l-value\n"
		)
	);

	let (code, out, err) = check(&dir, &["-DLOOPS", "--byond-version=516.1688", "world.dme"]);
	assert_eq!(code, 5, "{}", err);
	let lines: Vec<&str> = out.lines().collect();
	assert_eq!(lines.len(), 2, "{}", out);
	assert!(
		lines[0].starts_with("world.dme:5: /caught: nested list loops separated by `try`"),
		"{}",
		out
	);
	assert!(lines[1].starts_with("world.dme:9: /datum/num/operator~=: `operator~=`: "), "{}", out);
	assert_eq!(
		err,
		"not checked: world.dme: /refused: line 12: expected l-value\nerror: 2 issues found in 2 procs for BYOND 516.1688; 1 proc not checked.\n"
	);
	println!("ok: check filters known bugs by build");

	let (code, out, err) = check(&dir, &["-DLOOPS", "--byond-version", "516.1689", "--json"]);
	assert_eq!(code, 5, "{}", err);
	assert_eq!(err, "error: 1 issue found in 2 procs for BYOND 516.1689; 1 proc not checked.\n");
	let found: Value = serde_json::from_str(&out).unwrap_or_else(|_| panic!("unexpected check --json output: {}", out));
	assert_eq!(
		(&found["byond"], &found["procs"], &found["parse_errors"]),
		(&json!("516.1689"), &json!(2), &json!([]))
	);
	assert_eq!(
		found["not_checked"],
		json!([{ "file": "world.dme", "proc": "/refused", "override": 0, "error": "line 12: expected l-value" }])
	);
	let places = found["found"].as_array().unwrap();
	assert_eq!(places.len(), 1, "{}", out);
	assert_eq!(
		(&places[0]["file"], &places[0]["line"], &places[0]["proc"], &places[0]["override"]),
		(&json!("world.dme"), &json!(5), &json!("/caught"), &json!(0))
	);
	assert!(
		places[0]["text"].as_str().unwrap().contains("nested list loops separated by `try`"),
		"{}",
		out
	);
	println!("ok: check --json");

	let (code, out, err) = check(&dir, &["other.dme"]);
	assert_eq!((code, out.as_str()), (1, ""), "{}", err);
	assert!(err.starts_with("error: ") && err.ends_with("other.dme does not exist.\n"), "{}", err);
	for (args, said) in [
		(
			&["--strict"][..],
			"error: unexpected argument '--strict'. Run 'auxtools check --help' for usage.\n"
		),
		(
			&["--byond-version", "516"],
			"error: invalid value '516' for --byond-version: expected a BYOND build such as 516.1689. Run 'auxtools check --help' for usage.\n"
		),
		(
			&["--pid", "1"],
			"error: --pid cannot be used with 'check': it does not select an existing world.\n"
		)
	] {
		let (code, out, err) = check(&dir, args);
		assert_eq!((code, out.as_str(), err.as_str()), (1, "", said), "`auxtools check {}`", args.join(" "));
	}
	std::fs::write(dir.join("second.dme"), CODE).unwrap();
	let (code, _, err) = check(&dir, &[]);
	assert_eq!(
		(code, err.as_str()),
		(1, "error: more than one .dme in the current directory: specify DME.\n")
	);
	println!("ok: check errors");
	let _ = std::fs::remove_dir_all(&dir);
}
