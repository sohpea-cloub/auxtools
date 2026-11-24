use std::{
	process::{Command, Output, Stdio},
	time::{Duration, Instant}
};

use auxtools_protocol::{JournalEntry, JournalEvent, Request, Response};
use test_support::standin::{Answer, Answers, StandIn};

const LATE: Duration = Duration::from_secs(10);

fn command(pid: u32, args: &[&str]) -> std::process::Child {
	test_support::paths::use_launcher(env!("CARGO_BIN_EXE_auxtools"));
	Command::new(test_support::paths::launcher_path())
		.arg("--pid")
		.arg(pid.to_string())
		.args(args)
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap()
}

fn printed(output: &Output) -> String {
	format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn watchers_answered(mut answered: usize) -> Answers {
	Box::new(move |request| match request {
		Request::Watcher if answered > 0 => {
			answered -= 1;
			Answer::Reply(Response::Ack)
		}
		Request::Journal { .. } => Answer::Reply(Response::Journal {
			entries: vec![],
			first: 1,
			last: 1
		}),
		Request::Disconnect => Answer::Nothing,
		_ => Answer::ToTest
	})
}

#[test]
fn a_wait_for_a_reboot_gives_up_at_its_timeout_while_the_world_doesnt_answer() {
	let mut stand_in = StandIn::new(watchers_answered(0));
	let pid = stand_in.spawn();
	let started = Instant::now();
	let waiting = command(pid, &["wait", "reboot", "--timeout", "2"]);
	let mut world = stand_in.world();
	world.next("Watcher", |request| matches!(request, Request::Watcher));
	let output = waiting.wait_with_output().unwrap();
	let took = started.elapsed();
	assert!(
		output.status.code() == Some(2) && took < LATE,
		"wait reboot --timeout 2 ended after {:?} with {:?}: {}",
		took,
		output.status.code(),
		printed(&output)
	);
	assert_eq!(printed(&output), "error: timed out after 2 seconds waiting for the world to reboot.\n");
}

#[test]
fn a_wait_for_a_reboot_gives_up_at_its_timeout_while_the_next_world_doesnt_answer() {
	let mut stand_in = StandIn::new(watchers_answered(1));
	let pid = stand_in.spawn();
	let started = Instant::now();
	let waiting = command(pid, &["wait", "reboot", "--timeout", "2"]);
	let first = stand_in.world();
	while !first.requests().iter().any(|request| request.starts_with("Journal")) {
		assert!(started.elapsed() < LATE, "the command didn't follow the journal: {:?}", first.requests());
		std::thread::sleep(Duration::from_millis(20));
	}
	first.event(Response::Disconnect);
	first.close();
	let mut next = stand_in.world();
	next.next("Watcher again", |request| matches!(request, Request::Watcher));
	let output = waiting.wait_with_output().unwrap();
	let took = started.elapsed();
	assert!(
		output.status.code() == Some(2) && took < LATE,
		"wait reboot --timeout 2 ended after {:?} with {:?}: {}",
		took,
		output.status.code(),
		printed(&output)
	);
	assert_eq!(
		printed(&output),
		"error: timed out waiting for the world to respond after its connection ended.\n"
	);
}

#[test]
fn a_wait_for_a_reboot_from_a_journal_entry_survives_a_reboot_while_it_registers() {
	let loaded = JournalEntry {
		number: 8,
		time: 0,
		world_time: 0.0,
		event: JournalEvent::Loaded { load: 2 }
	};
	let mut watchers = 0;
	let mut stand_in = StandIn::new(Box::new(move |request| match request {
		Request::Watcher => {
			watchers += 1;
			match watchers {
				1 => Answer::ToTest,
				_ => Answer::Reply(Response::Ack)
			}
		}
		Request::Journal { after, .. } => Answer::Reply(Response::Journal {
			entries: if *after == 5 { vec![loaded.clone()] } else { vec![] },
			first: 1,
			last: 8
		}),
		Request::Uptime => Answer::Reply(Response::Uptime {
			loaded: 0,
			up: 1.0,
			world_time: 0.0,
			loads: 2,
			last_reload: None,
			started: true
		}),
		_ => Answer::Nothing
	}));
	let pid = stand_in.spawn();
	let started = Instant::now();
	let waiting = command(pid, &["wait", "reboot", "--since", "5", "--timeout", "5"]);
	let mut first = stand_in.world();
	first.next("Watcher", |request| matches!(request, Request::Watcher));
	first.event(Response::Disconnect);
	first.close();
	let output = waiting.wait_with_output().unwrap();
	let took = started.elapsed();
	assert!(
		output.status.code() == Some(0) && printed(&output).starts_with("#8 ") && took < LATE,
		"wait reboot --since 5 ended after {:?} with {:?}: {}",
		took,
		output.status.code(),
		printed(&output)
	);
}

#[test]
fn a_wait_for_a_reboot_keeps_its_timeout_through_a_reboot_while_it_registers() {
	let mut stand_in = StandIn::new(watchers_answered(0));
	let pid = stand_in.spawn();
	let waiting = command(pid, &["wait", "reboot", "--since", "5", "--timeout", "3"]);
	let mut first = stand_in.world();
	first.next("Watcher", |request| matches!(request, Request::Watcher));
	let asked = Instant::now();
	std::thread::sleep(Duration::from_secs(2));
	first.event(Response::Disconnect);
	first.close();
	let mut next = stand_in.world();
	next.next("Watcher again", |request| matches!(request, Request::Watcher));
	let output = waiting.wait_with_output().unwrap();
	let took = asked.elapsed();
	assert!(
		output.status.code() == Some(2) && took < Duration::from_secs(4),
		"wait reboot --timeout 3, cut 2 seconds after it registered, ended {:?} after it registered with {:?}: {}",
		took,
		output.status.code(),
		printed(&output)
	);
	assert_eq!(printed(&output), "error: timed out after 3 seconds waiting for the world to reboot.\n");
}
