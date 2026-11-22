use std::{
	path::Path,
	time::{Duration, Instant}
};

use test_support::cli::{launcher_command, Background};

use crate::{journal::run, line_of};

pub fn profile(pid: u64, dir: &Path, code: &Path) {
	let tally = line_of(code, "\ttick(1)");
	let tick = line_of(code, "/proc/tick(n)") + 1;

	let started = Instant::now();
	let printed = run(pid, &["profile", "/proc/tick", "/proc/tally", "--for", "1.5"], 0);
	assert!(started.elapsed() >= Duration::from_millis(1500), "profile --for 1.5 ended early");
	let lines: Vec<&str> = printed.lines().collect();
	let row = |end: &str| {
		let found = lines.iter().find(|line| line.ends_with(end));
		found
			.unwrap_or_else(|| panic!("no line ends with {:?} in {}", end, printed))
			.split_whitespace()
			.collect::<Vec<_>>()
	};
	let number = |text: &str| text.parse::<u64>().unwrap_or_else(|_| panic!("{} isn't a count in {}", text, printed));
	assert!(
		lines[0].starts_with("profiled 1.") && lines[0].ends_with(" in 2 procs"),
		"unexpected profile output: {}",
		printed
	);
	assert_eq!(lines[2].split_whitespace().collect::<Vec<_>>(), [
		"time",
		"%",
		"with",
		"calls",
		"calls",
		"instructions",
		"proc"
	]);
	let of_tick = row(&format!("  /tick  attach_host.dm:{}", tick));
	let of_tally = row(&format!("  /tally  attach_host.dm:{}", tally));
	let (ticks, tallies) = (number(of_tick[5]), number(of_tally[5]));
	assert!(
		tallies >= 4 && ticks > tallies && ticks <= tallies * 2,
		"unexpected profile output: {}",
		printed
	);
	assert!(
		lines.contains(&"/tick  attach_host.dm") && lines.contains(&"/tally  attach_host.dm"),
		"{}",
		printed
	);
	assert!(
		lines.contains(&format!("line {}:  return n", tick).as_str()) && lines.contains(&format!("line {}:  tick(1)", tally).as_str()),
		"unexpected profile output: {}",
		printed
	);
	let returned = row("  Ret");
	assert_eq!(number(returned[1]), ticks, "{}", printed);
	let called = lines.iter().find(|line| line.ends_with("  CallGlob 1 /tick")).unwrap();
	let called: Vec<&str> = called.split_whitespace().collect();
	assert!(
		number(called[1]) == tallies && (called.len() == 7 || (called.len() == 9 && ["ns", "us"].contains(&called[5]))),
		"unexpected profile output: {}",
		printed
	);
	assert_eq!(
		run(pid, &["status"], 0).lines().next().unwrap_or_default(),
		format!("pid {}: running", pid)
	);
	println!("ok: named proc listings");

	let printed = run(pid, &["profile", "--for", "1", "--top", "3"], 0);
	let lines: Vec<&str> = printed.lines().collect();
	let tables: Vec<usize> = (0..lines.len()).filter(|at| lines[*at].trim_start().starts_with("time ")).collect();
	assert!(
		tables.len() == 2
			&& lines[tables[1]].ends_with("  line")
			&& tables[1] - tables[0] == 5
			&& lines.len() == tables[1] + 4
			&& lines[tables[1] + 1..].iter().all(|line| line.contains("  attach_host.dm:"))
			&& !printed.contains("auxtools_"),
		"unexpected profile output: {}",
		printed
	);
	println!("ok: proc and source line rankings");

	let page = dir.join("profile.html");
	let printed = run(
		pid,
		&["profile", "--for", "1", "--top", "2", "--json", "--html", page.to_str().unwrap()],
		0
	);
	let report: serde_json::Value = serde_json::from_str(&printed).unwrap_or_else(|e| panic!("{}: {}", e, printed));
	let procs = report["procs"].as_array().unwrap();
	let names: Vec<&str> = procs.iter().map(|proc| proc["proc"].as_str().unwrap()).collect();
	let times: Vec<u64> = procs.iter().map(|proc| proc["ns"].as_u64().unwrap()).collect();
	assert!(
		report["ended"] == "time"
			&& report["seconds"].as_f64().unwrap() >= 1.0
			&& ["/run_until_stopped", "/value", "/tally", "/tick"]
				.iter()
				.all(|name| names.contains(name))
			&& times.windows(2).all(|pair| pair[0] >= pair[1])
			&& report["ns"] == times.iter().sum::<u64>()
			&& procs.iter().enumerate().all(|(at, proc)| proc["listing"].is_array() == (at < 2)),
		"unexpected profile JSON: {}",
		printed
	);
	for proc in &procs[..2] {
		let listing = proc["listing"].as_array().unwrap();
		let hits: u64 = listing.iter().map(|row| row["hits"].as_u64().unwrap()).sum();
		assert_eq!(proc["instructions"], hits, "{}", proc);
		assert!(listing.iter().any(|row| row["source"].is_string()), "{}", proc);
	}
	let written = std::fs::read_to_string(&page).unwrap();
	assert!(
		written.contains("<title>Instruction timing</title>") && written.contains(&format!(r#""proc":"{}""#, names[0])),
		"the page doesn't hold the report"
	);
	println!("ok: JSON and HTML profiles");

	let busy = "error: cannot start a profile: another profile is running.\n";
	let left_running = |args: &[&str]| {
		let mut running = Background::start(pid, args);
		running.error_with("profiling until interrupted");
		running
	};
	let running = left_running(&["profile"]);
	for (args, said) in [
		(&["profile", "--for", "1"][..], busy),
		(
			&["profile", "/proc/nothing_called_this"],
			"error: cannot start a profile: the world has no proc /proc/nothing_called_this.\n"
		),
		(&["profile", "tick"], "error: invalid proc path 'tick': expected /PATH[#N].\n")
	] {
		assert_eq!(run(pid, args, 1), said, "{:?}", args);
	}
	running.stop();
	let deadline = Instant::now() + Duration::from_secs(20);
	loop {
		let pid = pid.to_string();
		let (code, printed) = launcher_command(&["--pid", &pid, "profile", "--for", "1"]);
		if code == 0 {
			break;
		}
		assert!(Instant::now() < deadline, "profile remained after client exit: {}", printed);
		std::thread::sleep(Duration::from_millis(200));
	}
	println!("ok: profile errors and client cleanup");

	#[cfg(unix)]
	{
		let running = left_running(&["profile", "/proc/tick"]);
		crate::journal::rounds_pass(pid, 2);
		std::process::Command::new("kill")
			.args(["-INT", &running.id().to_string()])
			.status()
			.unwrap();
		let (code, printed) = running.finish();
		assert!(
			code == 0 && printed.contains("profiled ") && printed.contains(" in 1 procs\n") && printed.contains("  Ret\n"),
			"unexpected interrupted profile output: {} (exit {})",
			printed,
			code
		);
		println!("ok: interrupted profile report");
	}
}
