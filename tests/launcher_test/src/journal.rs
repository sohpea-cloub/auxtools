use std::{
	path::Path,
	time::{Duration, Instant}
};

use test_support::cli::{self, Background, Cli, TIMEOUT};

#[track_caller]
pub fn run(pid: u64, args: &[&str], code: i32) -> String {
	Cli::new(pid).run(args, code)
}

#[track_caller]
pub fn wait_at(pid: u64, place: &str) -> Background {
	let mut waiting = Background::start(pid, &["wait", "pause", "--break", place, "--timeout", "15"]);
	waiting.error_with("breakpoint set at ");
	waiting
}

#[track_caller]
pub fn paused_at(pid: u64, place: &str, trigger: impl FnOnce()) -> String {
	let waiting = wait_at(pid, place);
	trigger();
	let (exit, printed) = waiting.finish();
	assert_eq!(exit, 0, "`wait pause --break {}` gave {}: {}", place, exit, printed);
	printed
}

#[track_caller]
pub fn rounds_pass(pid: u64, more: u64) {
	let rounds = || run(pid, &["eval", "rounds"], 0).trim().parse::<u64>().unwrap();
	let (from, deadline) = (rounds(), Instant::now() + TIMEOUT);
	while rounds() < from + more {
		assert!(Instant::now() < deadline, "the world's loop stopped at round {}", from);
		std::thread::sleep(Duration::from_millis(20));
	}
}

#[track_caller]
pub fn newest(pid: u64) -> u64 {
	let printed = run(pid, &["log", "--last", "1", "--json"], 0);
	printed.lines().last().map_or(0, |line| {
		serde_json::from_str::<serde_json::Value>(line).unwrap()["number"].as_u64().unwrap()
	})
}

pub fn runtimes(pid: u64, dir: &Path) {
	let before = newest(pid);
	let since = before.to_string();
	std::fs::write(dir.join("fail"), "yes").unwrap();
	let deadline = Instant::now() + TIMEOUT;
	let printed = loop {
		let printed = run(pid, &["log", "--runtimes", "--since", &since], 0);
		if !printed.is_empty() {
			break printed;
		}
		assert!(Instant::now() < deadline, "runtime error missing from journal");
		std::thread::sleep(Duration::from_millis(200));
	};
	let mut lines = printed.lines();
	let first = lines.next().unwrap();
	assert!(
		first.starts_with('#') && first.ends_with("runtime error: cannot read from list"),
		"log --runtimes said {}",
		printed
	);
	assert!(
		lines.next().is_some_and(|line| line.starts_with("  /fail  attach_host.dm:")),
		"log --runtimes said {}",
		printed
	);
	assert_eq!(run(pid, &["status"], 0).lines().next(), Some(format!("pid {}: running", pid).as_str()));
	println!("ok: runtime journal and stack");

	let printed = run(pid, &["log", "--since", &since, "--json"], 0);
	let entry: serde_json::Value = serde_json::from_str(printed.lines().find(|line| line.contains("\"runtime\"")).unwrap()).unwrap();
	assert_eq!(
		(&entry["kind"], &entry["caught"], &entry["proc"]),
		(&"runtime".into(), &false.into(), &"/fail".into()),
		"{}",
		entry
	);
	assert!(
		entry["number"].as_u64().unwrap() > before && entry["line"].as_u64().is_some(),
		"{}",
		entry
	);
	println!("ok: journal JSON Lines");

	let mut follow = Background::start(pid, &["log", "--runtimes", "--follow", "--json", "--since", &before.to_string()]);
	follow.line_with("cannot read from list");
	let now = newest(pid);
	std::fs::write(dir.join("fail"), "yes").unwrap();
	let line = follow.line_with("cannot read from list");
	let entry: serde_json::Value = serde_json::from_str(&line).unwrap();
	assert!(
		entry["number"].as_u64().unwrap() > now && entry["stack"].is_array(),
		"log --follow printed {}",
		follow.printed
	);
	follow.stop();
	println!("ok: follow runtime journal");
}

pub fn reloads(pid: u64, code: &Path) {
	let source = std::fs::read_to_string(code).unwrap();
	assert!(source.contains("\treturn 2\n"), "unexpected value() source after test setup");
	let printed = run(pid, &["wait", "reload", "--timeout", "1"], 2);
	assert_eq!(printed.trim(), "error: timed out after 1 second waiting for a hot reload attempt.");

	let since = newest(pid).to_string();
	let looped = "\tfor (var/a in list(1)) { try { for (var/b in list(2)) { a += b } } catch (var/e) { a = e } }\n\treturn 3\n";
	assert!(source.contains("\treturn 2\n\n"), "unexpected value() source after test setup");
	std::fs::write(code, source.replace("\treturn 2\n\n", looped)).unwrap();
	let printed = run(pid, &["wait", "reload", "--since", &since, "--timeout", "15"], 0);
	assert!(
		printed.starts_with('#') && printed.contains(" hot reload applied: reloaded /value"),
		"unexpected wait reload output: {}",
		printed
	);
	assert!(
		printed.contains("\n  warning: /value: line ") && printed.contains("nested list loops separated by `try`"),
		"no warning about the loop: {}",
		printed
	);
	println!("ok: reload result includes BYOND bug warning");

	let since = newest(pid).to_string();
	std::fs::write(code, source.replace("\treturn 2\n", "\treturn nothing_called_this\n")).unwrap();
	let printed = run(pid, &["wait", "reload", "--since", &since, "--timeout", "15", "--json"], 1);
	let entry: serde_json::Value = serde_json::from_str(printed.lines().next().unwrap()).unwrap();
	assert_eq!((&entry["kind"], &entry["applied"]), (&"reload".into(), &false.into()), "{}", printed);
	let errors = entry["errors"].to_string();
	assert!(
		errors.contains("/value") && errors.contains("nothing_called_this"),
		"the errors were {}",
		errors
	);
	assert!(
		printed.ends_with("\nerror: the hot reload was rejected.\n"),
		"unexpected wait reload output: {}",
		printed
	);
	println!("ok: rejected reload diagnostics ({})", errors);

	let since = newest(pid).to_string();
	std::fs::write(code, source.replace("\treturn 2\n", "\treturn ]\n")).unwrap();
	let printed = run(pid, &["wait", "reload", "--since", &since, "--timeout", "15"], 1);
	let error = printed
		.lines()
		.find(|line| line.starts_with("  error: "))
		.unwrap_or_else(|| panic!("unexpected wait reload output: {}", printed));
	let place = error.trim_start_matches("  error: ").split(": ").next().unwrap();
	assert!(
		place
			.rsplit_once(':')
			.is_some_and(|(file, line)| file.ends_with("attach_host.dm") && line.parse::<u32>().is_ok()),
		"the error isn't at a line of the file: {}",
		printed
	);
	println!(
		"ok: wait reload says a hot reload with a compile error wasn't applied, and where the error is ({})",
		error.trim()
	);

	let mut attached = test_support::dap::Launcher::start();
	attached.initialize();
	attached.request("attach", serde_json::json!({ "processId": pid, "hotReload": false }));
	attached.event("initialized");
	attached.request("configurationDone", serde_json::json!({}));

	let since = newest(pid).to_string();
	let waiting = Background::start(pid, &["wait", "reload", "--since", &since, "--timeout", "15"]);
	std::fs::write(code, &source).unwrap();
	let (exit, printed) = waiting.finish();
	assert!(
		exit == 0 && printed.contains("hot reload applied: reloaded /value"),
		"wait reload gave {}: {}",
		exit,
		printed
	);
	attached.wait_output("Hot reload by another launcher: reloaded /value");
	attached.disconnect(serde_json::json!({}));
	println!("ok: attached launcher reload reports");

	let printed = run(pid, &["log", "--reloads", "--last", "4"], 0);
	let headers: Vec<&str> = printed.lines().filter(|line| line.starts_with('#')).collect();
	assert!(
		headers.len() == 4
			&& headers[0].contains("applied: reloaded /value")
			&& headers[1].contains("not applied")
			&& headers[2].contains("not applied")
			&& headers[3].contains("applied: reloaded /value"),
		"log --reloads said {}",
		printed
	);
	assert!(
		printed.contains("\n  error: ") && printed.contains("nothing_called_this"),
		"log --reloads said {}",
		printed
	);
	println!("ok: reload history and errors");

	let printed = run(pid, &["log", "--output", "--last", "5"], 0);
	assert!(
		printed.contains(" output\n  attach test: "),
		"unexpected log --output output: {}",
		printed
	);
	println!("ok: world output journal");

	let printed = run(pid, &["status"], 0);
	let lines: Vec<&str> = printed.lines().collect();
	assert!(
		lines.len() == 3
			&& lines[0] == format!("pid {}: running", pid)
			&& lines[1].starts_with("uptime: ")
			&& lines[1].contains(" UTC), world.time ")
			&& lines[1].ends_with(", world loads: 1")
			&& lines[2].starts_with("last reload: #")
			&& lines[2].ends_with(" hot reload applied: reloaded /value"),
		"status said {}",
		printed
	);
	println!("ok: uptime and latest reload status");

	let place = format!("attach_host.dm:{}", crate::line_of(code, "\t\ttotal += i // first loop"));
	let waiting = wait_at(pid, &place);
	let since = newest(pid).to_string();
	std::fs::write(code, source.replace("/proc/two_loops()\n", "/proc/two_loops()\n\tvar/moved = 1\n")).unwrap();
	run(pid, &["wait", "reload", "--since", &since, "--timeout", "15"], 0);
	run(pid, &["eval", "--spawn", "two_loops()"], 0);
	let (exit, printed) = waiting.finish();
	assert_eq!(exit, 0, "wait pause after a reload gave: {}", printed);
	run(pid, &["continue"], 0);
	let since = newest(pid).to_string();
	std::fs::write(code, &source).unwrap();
	run(pid, &["wait", "reload", "--since", &since, "--timeout", "15"], 0);
	println!("ok: wait pause keeps its breakpoint through a reload");

	let guard = code.with_file_name("guard");
	let place = format!("attach_host.dm:{}", crate::line_of(code, "\t\ttotal += i // second loop"));
	let waiting = wait_at(pid, &place);
	let since = newest(pid).to_string();
	let guarded = "\t\tif (fexists(\"guard\"))\n\t\t\ttotal += i // second loop\n";
	std::fs::write(code, source.replace("\t\ttotal += i // second loop\n", guarded)).unwrap();
	run(pid, &["wait", "reload", "--since", &since, "--timeout", "15"], 0);
	run(pid, &["eval", "--spawn", "two_loops()"], 0);
	rounds_pass(pid, 2);
	std::fs::write(&guard, "yes").unwrap();
	run(pid, &["eval", "--spawn", "two_loops()"], 0);
	let (exit, printed) = waiting.finish();
	let line = crate::line_of(code, "\t\t\ttotal += i // second loop");
	assert!(
		exit == 0 && printed.contains(&format!("#0 /two_loops  attach_host.dm:{}\n", line)),
		"wait pause on a statement put under a condition gave {}: {}",
		exit,
		printed
	);
	run(pid, &["continue"], 0);
	std::fs::remove_file(&guard).unwrap();
	let since = newest(pid).to_string();
	std::fs::write(code, &source).unwrap();
	run(pid, &["wait", "reload", "--since", &since, "--timeout", "15"], 0);
	println!("ok: wait pause keeps its breakpoint on a statement put under a condition");

	let before =
		"/proc/notified()\n\tvar/total = 1\n\ttotal += 2 // notify\n\tif (fexists(\"guard\"))\n\t\ttotal *= 3\n\treturn total\n\n/proc/two_loops()\n";
	let after = "/proc/notified()\n\tvar/total = 1\n\tif (fexists(\"guard\"))\n\t\ttotal *= 3\n\t\ttotal += 2 // notify\n\treturn \
	             total\n\n/proc/two_loops()\n";
	let since = newest(pid).to_string();
	std::fs::write(code, source.replace("/proc/two_loops()\n", before)).unwrap();
	run(pid, &["wait", "reload", "--since", &since, "--timeout", "15"], 0);
	let place = format!("attach_host.dm:{}", crate::line_of(code, "\ttotal += 2 // notify"));
	let waiting = wait_at(pid, &place);
	let since = newest(pid).to_string();
	std::fs::write(code, source.replace("/proc/two_loops()\n", after)).unwrap();
	run(pid, &["wait", "reload", "--since", &since, "--timeout", "15"], 0);
	run(pid, &["eval", "--spawn", "notified()"], 0);
	rounds_pass(pid, 2);
	std::fs::write(&guard, "yes").unwrap();
	run(pid, &["eval", "--spawn", "notified()"], 0);
	let (exit, printed) = waiting.finish();
	let line = crate::line_of(code, "\t\ttotal += 2 // notify");
	assert!(
		exit == 0 && printed.contains(&format!("#0 /notified  attach_host.dm:{}\n", line)),
		"wait pause on a statement moved into the next block gave {}: {}",
		exit,
		printed
	);
	run(pid, &["continue"], 0);
	std::fs::remove_file(&guard).unwrap();
	let since = newest(pid).to_string();
	std::fs::write(code, &source).unwrap();
	run(pid, &["wait", "reload", "--since", &since, "--timeout", "15"], 0);
	println!("ok: wait pause keeps its breakpoint on a statement moved into the next block");

	let before = "/proc/looped()\n\tvar/total = 1\n\tfor (var/i in list(1, 2))\n\t\ttotal += i * 2 // notify\n\t\tif \
	              (fexists(\"guard\"))\n\t\t\ttotal *= 3\n\treturn total\n\n/proc/two_loops()\n";
	let after = "/proc/looped()\n\tvar/total = 1\n\tfor (var/i in list(1, 2))\n\t\tif (fexists(\"guard\"))\n\t\t\ttotal *= 3\n\t\t\ttotal += i * 2 \
	             // notify\n\treturn total\n\n/proc/two_loops()\n";
	let since = newest(pid).to_string();
	std::fs::write(code, source.replace("/proc/two_loops()\n", before)).unwrap();
	run(pid, &["wait", "reload", "--since", &since, "--timeout", "15"], 0);
	let place = format!("attach_host.dm:{}", crate::line_of(code, "\t\ttotal += i * 2 // notify"));
	let waiting = wait_at(pid, &place);
	let since = newest(pid).to_string();
	std::fs::write(code, source.replace("/proc/two_loops()\n", after)).unwrap();
	run(pid, &["wait", "reload", "--since", &since, "--timeout", "15"], 0);
	run(pid, &["eval", "--spawn", "looped()"], 0);
	rounds_pass(pid, 2);
	std::fs::write(&guard, "yes").unwrap();
	run(pid, &["eval", "--spawn", "looped()"], 0);
	let (exit, printed) = waiting.finish();
	let line = crate::line_of(code, "\t\t\ttotal += i * 2 // notify");
	assert!(
		exit == 0 && printed.contains(&format!("#0 /looped  attach_host.dm:{}\n", line)),
		"wait pause on a statement moved to the end of a loop's body gave {}: {}",
		exit,
		printed
	);
	run(pid, &["continue"], 0);
	std::fs::remove_file(&guard).unwrap();
	let since = newest(pid).to_string();
	std::fs::write(code, &source).unwrap();
	run(pid, &["wait", "reload", "--since", &since, "--timeout", "15"], 0);
	println!("ok: wait pause keeps its breakpoint on a statement moved to the end of a loop's body");

	let before = "/proc/branched()\n\tvar/total = 1\n\ttotal += 2 // notify\n\tif (fexists(\"guard\"))\n\t\ttotal *= 3\n\telse\n\t\ttotal -= \
	              1\n\treturn total\n\n/proc/two_loops()\n";
	let after = "/proc/branched()\n\tvar/total = 1\n\tif (fexists(\"guard\"))\n\t\ttotal *= 3\n\t\ttotal += 2 // notify\n\telse\n\t\ttotal -= \
	             1\n\treturn total\n\n/proc/two_loops()\n";
	let since = newest(pid).to_string();
	std::fs::write(code, source.replace("/proc/two_loops()\n", before)).unwrap();
	run(pid, &["wait", "reload", "--since", &since, "--timeout", "15"], 0);
	let place = format!("attach_host.dm:{}", crate::line_of(code, "\ttotal += 2 // notify"));
	let waiting = wait_at(pid, &place);
	let since = newest(pid).to_string();
	std::fs::write(code, source.replace("/proc/two_loops()\n", after)).unwrap();
	run(pid, &["wait", "reload", "--since", &since, "--timeout", "15"], 0);
	run(pid, &["eval", "--spawn", "branched()"], 0);
	rounds_pass(pid, 2);
	std::fs::write(&guard, "yes").unwrap();
	run(pid, &["eval", "--spawn", "branched()"], 0);
	let (exit, printed) = waiting.finish();
	let line = crate::line_of(code, "\t\ttotal += 2 // notify");
	assert!(
		exit == 0 && printed.contains(&format!("#0 /branched  attach_host.dm:{}\n", line)),
		"wait pause on a statement moved to the end of an `if` with an `else` gave {}: {}",
		exit,
		printed
	);
	run(pid, &["continue"], 0);
	std::fs::remove_file(&guard).unwrap();
	let since = newest(pid).to_string();
	std::fs::write(code, &source).unwrap();
	run(pid, &["wait", "reload", "--since", &since, "--timeout", "15"], 0);
	println!("ok: wait pause keeps its breakpoint on a statement moved to the end of an `if` with an `else`");

	let before = "/proc/spawned()\n\tvar/total = 1\n\ttotal += 2 // notify\n\ttotal -= 1\n\treturn total\n\n/proc/two_loops()\n";
	let after = "/proc/spawned()\n\tvar/total = 1\n\tspawn(10)\n\t\ttotal += 2 // notify\n\ttotal -= 1\n\treturn total\n\n/proc/two_loops()\n";
	let since = newest(pid).to_string();
	std::fs::write(code, source.replace("/proc/two_loops()\n", before)).unwrap();
	run(pid, &["wait", "reload", "--since", &since, "--timeout", "15"], 0);
	let place = format!("attach_host.dm:{}", crate::line_of(code, "\ttotal += 2 // notify"));
	let waiting = wait_at(pid, &place);
	let since = newest(pid).to_string();
	std::fs::write(code, source.replace("/proc/two_loops()\n", after)).unwrap();
	run(pid, &["wait", "reload", "--since", &since, "--timeout", "15"], 0);
	run(pid, &["eval", "--spawn", "spawned()"], 0);
	let (exit, printed) = waiting.finish();
	let line = crate::line_of(code, "\t\ttotal += 2 // notify");
	assert!(
		exit == 0 && printed.contains(&format!("#0 /spawned  attach_host.dm:{}\n", line)),
		"wait pause on a statement moved to the end of a `spawn` block gave {}: {}",
		exit,
		printed
	);
	run(pid, &["continue"], 0);
	let since = newest(pid).to_string();
	std::fs::write(code, &source).unwrap();
	run(pid, &["wait", "reload", "--since", &since, "--timeout", "15"], 0);
	println!("ok: wait pause keeps its breakpoint on a statement moved to the end of a `spawn` block");
}

pub fn reboot(pid: u64, dir: &Path) {
	let before = newest(pid);
	let waiting = Background::start(pid, &["wait", "reboot", "--since", &before.to_string(), "--timeout", "15"]);
	std::fs::write(dir.join("reboot"), "yes").unwrap();
	let (exit, printed) = waiting.finish();
	assert!(
		exit == 0 && printed.contains(" world rebooted (load 2 in this process)"),
		"wait reboot gave {}: {}",
		exit,
		printed
	);
	let number: u64 = printed.trim_start_matches('#').split(' ').next().unwrap().parse().unwrap();
	assert!(number > before, "the reboot is entry {}, before {}", number, before);
	let printed = run(pid, &["status"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: running\nuptime: ", pid)) && printed.contains(", world loads: 2\n"),
		"status said {}",
		printed
	);
	let printed = run(pid, &["wait", "reboot", "--since", &before.to_string(), "--timeout", "5", "--json"], 0);
	let entry: serde_json::Value = serde_json::from_str(&printed).unwrap_or_else(|_| panic!("unexpected wait reboot --json output: {}", printed));
	assert!(
		entry["kind"] == "world" && entry["event"] == "loaded" && entry["load"] == 2 && entry["number"] == number,
		"{}",
		entry
	);
	println!("ok: wait for reboot readiness");

	let printed = run(pid, &["log", "--world", "--json"], 0);
	let loads: Vec<serde_json::Value> = printed
		.lines()
		.map(|line| serde_json::from_str(line).unwrap())
		.filter(|entry: &serde_json::Value| entry["event"] == "loaded")
		.collect();
	assert!(
		loads.len() == 2
			&& loads[0]["load"] == 1
			&& loads[1]["load"] == 2
			&& loads[1]["number"] == number
			&& loads[0]["number"].as_u64() < Some(number),
		"log --world said {}",
		printed
	);
	let printed = run(pid, &["wait", "reload", "--since", &number.to_string(), "--timeout", "15"], 0);
	assert!(printed.contains("hot reload applied: "), "unexpected wait reload output: {}", printed);
	println!("ok: journal continuity across reboot");
}

#[track_caller]
pub fn wait_for_reboot(pid: u64) -> Background {
	let waiting = Background::start(pid, &["wait", "reboot", "--timeout", "15"]);
	std::thread::sleep(Duration::from_secs(1));
	waiting
}

#[track_caller]
pub fn world_went_away(waiting: Background) {
	let (exit, printed) = waiting.finish();
	assert!(
		exit == 3 && printed.contains("error: world connection ended before the operation completed"),
		"wait reboot gave {}: {}",
		exit,
		printed
	);
	println!("ok: world exit during reboot wait");
}

#[track_caller]
fn until_running(pid: u64) {
	let started = Instant::now();
	loop {
		if run(pid, &["status"], 0).starts_with(&format!("pid {}: running\n", pid)) {
			return;
		}
		assert!(started.elapsed() < TIMEOUT, "the world stayed paused");
		std::thread::sleep(Duration::from_millis(200));
	}
}

pub fn hold_max(pid: u64) {
	let asked = Instant::now();
	let printed = run(pid, &["pause", "--hold-max", "1.5"], 0);
	assert!(
		printed.starts_with(&format!("pid {}: paused by request", pid)),
		"unexpected pause output: {}",
		printed
	);
	until_running(pid);
	let took = asked.elapsed();
	assert!(took >= Duration::from_millis(1500), "the pause was let go after {:?}", took);
	let printed = run(pid, &["log", "--world", "--last", "1"], 0);
	assert!(
		printed.contains("the world resumed after 1.5 seconds with no debugger connected, as --hold-max allows"),
		"log --world said {}",
		printed
	);
	println!("ok: held-pause expiry journal");

	let paused = cli::ran(Cli::new(pid).command(&["pause"]).env("AUXTOOLS_HOLD_MAX", "0.5"), None);
	assert_eq!(paused.code, 0, "unexpected pause output: {}", paused.err);
	until_running(pid);
	run(pid, &["pause"], 0);
	std::thread::sleep(Duration::from_secs(2));
	assert!(
		run(pid, &["status"], 0).starts_with(&format!("pid {}: paused", pid)),
		"a pause without --hold-max was let go"
	);
	run(pid, &["continue"], 0);
	println!("ok: environment pause limit and unlimited holds");
}
