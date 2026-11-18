//! `auxtools optimise`: an optimised world runs as it did, but for what the
//! command says it changes.

use std::{
	path::{Path, PathBuf},
	process::{Command, Stdio},
	time::{Duration, Instant}
};

use serde_json::{json, Value};

use crate::{
	client::launcher_path,
	paths::{self, ByondCommand}
};

const NAMES: &str = include_str!("optimise/names.dm");
const REFS: &str = include_str!("optimise/refs.dm");

/// What the world writes, as DreamMaker compiled it.
const STOCK: &str = "\
plain: typed cba1 untyped cba2 implicit cba3cba3cba3
plain on another type: typed other1 untyped other2
plain by name: cba7 hascall 1
settings: nowait1 background2 nowait3
text: text1 text2 text7 text7 hascall 1
verbs: verb1 category2 desc3 hidden4 category5
named: custom1 custom2 custom7
value: value1 value2 value3
shared: shared1 sharedverb2 shared3
world: world1 global: global2
verb names: added later, held verb; added1 not added2
error: proc name: will fail (/datum/a/proc/will_fail)
global error: proc name: global fails (/proc/global_fails)
done
";

/// Create a project of one file, compiled with `defines`.
fn project(name: &str, code: &str, defines: &[&str]) -> PathBuf {
	let dir = std::env::temp_dir().join(format!("launcher_test_optimise-{}-{}", name, std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let dme = dir.join("world.dme");
	std::fs::write(&dme, code).unwrap();
	let built = Command::new(paths::find_dm())
		.with_byond_paths()
		.args(defines)
		.arg(&dme)
		.output()
		.unwrap();
	assert!(
		built.status.success() && dir.join("world.dmb").is_file(),
		"DreamMaker did not build {}: {}",
		dme.display(),
		String::from_utf8_lossy(&built.stdout)
	);
	dir
}

/// Run the world until it shuts itself down, and return what it wrote.
fn results(dmb: &Path) -> String {
	let dir = dmb.parent().unwrap();
	let _ = std::fs::remove_file(dir.join("result.txt"));
	let mut world = Command::new(paths::find_dreamdaemon())
		.with_byond_paths()
		.arg(dmb)
		.args(["-trusted", "-close"])
		.current_dir(dir)
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.spawn()
		.unwrap();
	let started = Instant::now();
	loop {
		if world.try_wait().unwrap().is_some() {
			break;
		}
		if started.elapsed() > Duration::from_secs(120) {
			let _ = world.kill();
			let _ = world.wait();
			panic!(
				"{} did not shut down; it wrote {:?}",
				dmb.display(),
				std::fs::read_to_string(dir.join("result.txt"))
			);
		}
		std::thread::sleep(Duration::from_millis(50));
	}
	// (text2file ends a line as the platform does)
	std::fs::read_to_string(dir.join("result.txt"))
		.unwrap_or_else(|e| panic!("{} wrote no results: {}", dmb.display(), e))
		.replace("\r\n", "\n")
}

/// Run optimise and return its exit code, stdout and stderr.
fn optimise(dir: &Path, args: &[&str]) -> (i32, String, String) {
	let ran = Command::new(launcher_path())
		.arg("optimise")
		.args(args)
		.current_dir(dir)
		.output()
		.unwrap();
	let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
	(ran.status.code().unwrap_or(-1), text(&ran.stdout), text(&ran.stderr))
}

fn names() {
	let dir = project("names", NAMES, &[]);
	let stock = std::fs::read(dir.join("world.dmb")).unwrap();
	assert_eq!(
		results(&dir.join("world.dmb")),
		STOCK,
		"unexpected results from the world as DreamMaker built it"
	);

	// What BYOND's own procs add to the counts: every world has them
	let empty = project("empty", "/world/New()\n\t..()\n\tshutdown()\n", &[]);
	let (code, out, err) = optimise(&empty, &["world.dmb", "--skip", "refs", "--dry-run", "--json"]);
	assert_eq!((code, err.as_str()), (0, ""), "unexpected optimise --dry-run output: {}", out);
	let own: Value = serde_json::from_str(&out).unwrap_or_else(|_| panic!("unexpected optimise --dry-run --json output: {}", out));
	let count = |report: &Value, what: &str| report["names"][what].as_u64().unwrap_or_else(|| panic!("no {} in {}", what, report));
	let (identifiers, procs) = (count(&own, "identifiers") + 7, count(&own, "procs") + 10);

	// A dry run reports and writes nothing
	let (code, out, err) = optimise(&dir, &["world.dmb", "--skip", "refs", "--dry-run", "--json"]);
	assert_eq!((code, err.as_str()), (0, ""), "unexpected optimise --dry-run output: {}", out);
	let dry: Value = serde_json::from_str(&out).unwrap_or_else(|_| panic!("unexpected optimise --dry-run --json output: {}", out));
	assert_eq!(dry["output"], Value::Null, "{}", out);
	assert_eq!((count(&dry, "identifiers"), count(&dry, "procs")), (identifiers, procs), "{}", out);
	let mut kept = dry["names"]["kept"].clone();
	for (reason, listed) in kept.as_object_mut().unwrap() {
		listed
			.as_array_mut()
			.unwrap()
			.retain(|identifier| !own["names"]["kept"][reason.as_str()].as_array().unwrap().contains(identifier));
	}
	assert_eq!(
		kept,
		json!({
			"verb": ["a_verb", "held_verb", "is_hidden", "with_category", "with_desc"],
			"named": [],
			"value": ["added_later", "used_as_value"],
			"text": ["has_text"],
			"shared": ["shared_name"],
			"operator": [],
			"unread": [],
			"full": []
		}),
		"{}",
		out
	);
	assert!(
		std::fs::read(dir.join("world.dmb")).unwrap() == stock,
		"optimise --dry-run changed the world"
	);
	println!("ok: optimise --dry-run");

	// To another file: the world is left, and the copy behaves the same but
	// for the name in the error
	let (code, out, err) = optimise(&dir, &["world.dmb", "--skip", "refs", "--output", "optimised.dmb"]);
	assert_eq!((code, err.as_str()), (0, ""), "unexpected optimise --output output: {}", out);
	assert!(
		out.starts_with(&format!("names: renamed {} procs of {} identifiers, changed ", procs, identifiers))
			&& out.ends_with(&format!("wrote optimised.dmb: {0} to {0}\n", "0.0 MB")),
		"unexpected optimise --output output: {}",
		out
	);
	for kept in [
		" with a verb or a proc with verb settings\n",
		"  kept: 2 identifiers with a proc whose path is used as a value\n",
		"  kept: 1 identifier with a name that is also text in the code\n",
		"  kept: 1 identifier with a name another proc goes by\n"
	] {
		assert!(out.contains(kept), "optimise did not print {:?}: {}", kept, out);
	}
	assert!(
		std::fs::read(dir.join("world.dmb")).unwrap() == stock,
		"optimise --output changed the world it read"
	);
	let renamed = STOCK.replace("proc name: will fail", "proc name: will_fail");
	assert_eq!(
		results(&dir.join("optimised.dmb")),
		renamed,
		"unexpected results from the optimised world"
	);
	println!("ok: optimised world runs as before");

	// The same as DreamMaker's world with each renamed proc named in source
	let named = project("named", NAMES, &["-DNAMED"]);
	assert_eq!(
		results(&named.join("world.dmb")),
		renamed,
		"unexpected results from the world named in source"
	);
	let (code, out, _) = optimise(&named, &["world.dmb", "--skip", "refs", "--dry-run", "--json"]);
	let left: Value = serde_json::from_str(&out).unwrap_or_else(|_| panic!("unexpected optimise --json output: {}", out));
	assert_eq!(
		(code, count(&left, "identifiers")),
		(0, count(&own, "identifiers")),
		"optimise found procs to rename in a world named in source: {}",
		out
	);
	assert_eq!(left["names"]["second_names"]["after"], dry["names"]["second_names"]["after"], "{}", out);
	println!("ok: optimised world matches set name in source");

	// In place, and again: nothing left to do
	let (code, out, err) = optimise(&dir, &["world.dmb", "--skip", "refs"]);
	assert_eq!((code, err.as_str()), (0, ""), "unexpected optimise output: {}", out);
	let once = std::fs::read(dir.join("world.dmb")).unwrap();
	assert!(
		once == std::fs::read(dir.join("optimised.dmb")).unwrap(),
		"optimise in place and --output wrote different worlds"
	);
	let (code, out, err) = optimise(&dir, &["world.dmb", "--skip", "refs", "--json"]);
	assert_eq!((code, err.as_str()), (0, ""), "unexpected optimise output: {}", out);
	let again: Value = serde_json::from_str(&out).unwrap_or_else(|_| panic!("unexpected optimise --json output: {}", out));
	assert_eq!(
		(&again["names"]["identifiers"], &again["names"]["calls"]),
		(&json!(0), &json!(0)),
		"{}",
		out
	);
	assert!(
		std::fs::read(dir.join("world.dmb")).unwrap() == once,
		"optimising an optimised world changed it"
	);
	println!("ok: optimise in place, twice");

	for dir in [dir, named, empty] {
		let _ = std::fs::remove_dir_all(dir);
	}
}

fn refusals() {
	let dir = std::env::temp_dir().join(format!("launcher_test_optimise-refused-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	std::fs::write(dir.join("text.dmb"), "not a world\n").unwrap();
	std::fs::write(dir.join("old.dmb"), "world bin v514\nmin compatibility v514 514\n").unwrap();
	for (file, why) in [
		("text.dmb", "error: cannot read text.dmb: not a .dmb file.\n"),
		(
			"old.dmb",
			"error: cannot read old.dmb: a .dmb of BYOND 514 (compatible down to 514) isn't supported: 515 and 516 are.\n"
		)
	] {
		let (code, out, err) = optimise(&dir, &[file]);
		assert_eq!(
			(code, out.as_str(), err.as_str()),
			(1, "", why),
			"unexpected optimise output for {}",
			file
		);
	}
	let (code, out, err) = optimise(&dir, &["missing.dmb"]);
	assert!(
		code == 1 && out.is_empty() && err.starts_with("error: cannot read missing.dmb: "),
		"unexpected optimise output for a missing file: {} {} {}",
		code,
		out,
		err
	);
	println!("ok: optimise refuses what is not a world");
	let _ = std::fs::remove_dir_all(&dir);
}

/// The strings of the refs pass are the refs this BYOND build makes.
fn refs() {
	let dir = project("refs", REFS, &[]);
	let made = results(&dir.join("world.dmb"));
	let lines: Vec<&str> = made.lines().collect();
	// `[0x`, the kind, then which one in six hex digits
	for (line, kind) in lines
		.iter()
		.zip(["turf [0x1", "obj [0x2", "mob [0x3", "image [0xd", "list [0xf", "datum [0x21"])
	{
		let digits = line.strip_prefix(kind).and_then(|rest| rest.strip_suffix(']')).unwrap_or("?");
		assert!(
			digits.len() == 6 && digits.bytes().all(|digit| matches!(digit, b'0'..=b'9' | b'a'..=b'f')),
			"a ref is not as the refs pass writes them: {:?}, expected {}000000]",
			line,
			kind
		);
	}
	assert_eq!(lines.len(), 7, "unexpected results from the refs world: {}", made);

	let (code, out, err) = optimise(&dir, &["world.dmb", "--skip", "names", "--json"]);
	assert_eq!((code, err.as_str()), (0, ""), "unexpected optimise output: {}", out);
	let report: Value = serde_json::from_str(&out).unwrap_or_else(|_| panic!("unexpected optimise --json output: {}", out));
	assert_eq!(report["names"], Value::Null, "{}", out);
	assert_eq!(
		report["refs"],
		json!({ "added": 64512, "present": 0, "wanted": 64512, "spacing": 256, "full": false }),
		"{}",
		out
	);
	assert_eq!(
		results(&dir.join("world.dmb")),
		made,
		"unexpected results from the world with ref strings added"
	);

	// Again: they are there. Fewer are some of the same.
	let (code, out, err) = optimise(&dir, &["world.dmb", "--skip", "names", "--ref-seeds", "1000"]);
	assert_eq!(
		(code, out.as_str(), err.as_str()),
		(
			0,
			"refs: added 0 strings spaced 256 refs apart, 504 already there\nwrote world.dmb: 0.8 MB to 0.8 MB\n",
			""
		),
		"unexpected optimise output"
	);
	let (code, out, err) = optimise(&dir, &["world.dmb", "--skip", "names", "--skip", "refs", "--json"]);
	assert_eq!((code, err.as_str()), (0, ""), "unexpected optimise output: {}", out);
	let report: Value = serde_json::from_str(&out).unwrap_or_else(|_| panic!("unexpected optimise --json output: {}", out));
	assert_eq!((&report["names"], &report["refs"]), (&Value::Null, &Value::Null), "{}", out);
	println!("ok: ref strings added once, in the form of this build's refs");
	let _ = std::fs::remove_dir_all(&dir);
}

pub fn checks() {
	refusals();
	names();
	refs();
}
