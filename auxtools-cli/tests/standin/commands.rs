use std::{
	process::{Command, Output, Stdio},
	time::{Duration, Instant}
};

use auxtools_protocol::{Request, Response};
use test_support::standin::{Answer, Answers, StandIn};

const LATE: Duration = Duration::from_secs(10);

fn command(pid: u32, args: &[&str]) -> std::process::Child {
	Command::new(env!("CARGO_BIN_EXE_auxtools"))
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
