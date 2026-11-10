//! Pausing, and the procs asleep while the world is paused: each is a thread

use serde_json::json;

use crate::{client::named, ok, world::World};

/// The threads in `proc`, whatever line they're on
fn threads_in<'a>(threads: &'a [(u64, String)], proc: &str) -> Vec<&'a (u64, String)> {
	let with_place = format!("{}  ", proc);
	threads.iter().filter(|(_, name)| name == proc || name.starts_with(&with_place)).collect()
}

/// The id of the one thread in `proc`
fn thread_in(threads: &[(u64, String)], proc: &str) -> u64 {
	match threads_in(threads, proc).as_slice() {
		[(id, _)] => *id,
		found => panic!("{} threads in {}: {:?}", found.len(), proc, threads)
	}
}

pub fn run() {
	let mut world = World::start();
	let at = world.marks.clone();

	assert_eq!(world.threads(), [(0, "Main".to_owned())]);
	let trace = world.request("stackTrace", json!({ "threadId": 0 }));
	assert_eq!(trace["stackFrames"].as_array().unwrap().len(), 0);
	assert_eq!(trace["totalFrames"], 0);
	ok("a running world has one thread, with no frames to show");

	// A proc that doesn't sleep
	world.run("busy");
	world.request("pause", json!({ "threadId": 0 }));
	let stop = world.stop();
	assert_eq!((stop.reason.as_str(), stop.proc.as_str()), ("pause", "/sc_busy"));
	ok("pause stops the world in the code that's running");

	world.request("pause", json!({ "threadId": 0 }));
	assert_eq!(world.eval("stop_busy"), "0");
	assert!(!world.has_event("stopped"), "a pause while paused stopped again");
	ok("pause while paused changes nothing");

	let stop = world.step("next");
	assert_eq!((stop.reason.as_str(), stop.proc.as_str()), ("step", "/sc_busy"));
	ok("a step after a pause");
	world.eval("stop_busy = TRUE");
	world.resume_through("busy");

	// Nothing of the world's runs for three seconds: it pauses in the
	// debugger's own loop, which isn't shown as a thread
	world.eval_global("idle_ticks = 30");
	world.wait_output("dap: running");
	world.request("pause", json!({ "threadId": 0 }));
	world.stopped("pause");
	let frames = world.frames(0);
	assert_eq!(frames.len(), 1, "{:?}", frames);
	assert_eq!(frames[0]["presentationHint"], "subtle", "{}", frames[0]);
	assert!(frames[0]["source"].is_null(), "{}", frames[0]);
	world.resume();
	world.wait_output("dap: idle over");
	ok("pause in a world with nothing running stops it all the same, in a frame with no source");

	// nap(10) sleeps for a second while leaf() stops
	let set = world.try_break_if(&[("leaf_1", "")]);
	world.run("nap");
	let stop = world.stop();
	assert_eq!((stop.reason.as_str(), stop.proc.as_str(), stop.line), ("breakpoint", "/leaf", at.line("leaf_1")));
	assert_eq!(stop.hit, [set[0]["id"].as_u64().unwrap()], "the breakpoints the stop was for");
	world.break_on(&[]);
	let threads = world.threads();
	assert_ne!(stop.thread, 0);
	assert_eq!(threads[0], (stop.thread, format!("/leaf  stepping.dm:{}", at.line("leaf_1"))));
	ok("a stop says which thread it's in, the first listed, and which breakpoint it was for");

	let names: Vec<&str> = threads.iter().map(|(_, name)| name.as_str()).collect();
	let asleep = format!("/nap  stepping.dm:{}, sleeping", at.line("nap_2"));
	assert!(names.contains(&asleep.as_str()), "{:?}", names);
	assert!(names.iter().any(|name| name.starts_with("/main_loop  main.dm:") && name.ends_with(", sleeping")), "{:?}", names);
	assert!(!names.iter().any(|name| name.contains("auxtools")), "{:?}", names);
	ok("each sleeping proc is a thread, named with where it sleeps, but for the debugger's own");

	let nap = thread_in(&threads, "/nap");
	let frames = world.frames(nap);
	let places: Vec<(&str, u64)> = frames.iter().map(|f| (f["name"].as_str().unwrap(), f["line"].as_u64().unwrap())).collect();
	assert_eq!(places[0], ("/nap", at.line("nap_2")));
	assert_eq!(places[1].0, "/sc_nap");
	assert_eq!(places.len(), 2);
	ok("a sleeping proc's frames: on the line it sleeps on, and who called it");

	let frame = frames[0]["id"].clone();
	let locals = world.scope(&frame, "Locals");
	assert_eq!(crate::client::value_of(&named(&world.variables(&locals)), "napped"), "10");
	let arguments = world.scope(&frame, "Arguments");
	assert_eq!(crate::client::value_of(&named(&world.variables(&arguments)), "ticks"), "10");
	ok("a sleeping proc's locals and arguments");

	assert_eq!(world.evaluate_in(&frame, "napped + ticks", "watch")["result"], "20");
	assert_eq!(world.evaluate_in(&frame, "napped = 11", "repl")["result"], "11");
	ok("an expression in a sleeping proc's frame, and an assignment to its local");

	// A step later, each is the thread it was
	let stop_again = world.step("next");
	assert_eq!((stop_again.reason.as_str(), stop_again.thread), ("step", stop.thread));
	assert!(stop_again.hit.is_empty(), "a step stopped for breakpoints {:?}", stop_again.hit);
	let threads_again = world.threads();
	assert_eq!(thread_in(&threads_again, "/nap"), nap);
	assert_eq!(thread_in(&threads_again, "/main_loop"), thread_in(&threads, "/main_loop"));
	ok("threads keep their ids from one stop to the next");

	// It wakes a second later, the same thread
	world.request("next", json!({ "threadId": nap }));
	let woke = world.stop();
	assert_eq!((woke.reason.as_str(), woke.proc.as_str(), woke.line), ("step", "/nap", at.line("nap_3")));
	assert_eq!(woke.thread, nap, "the proc that woke is another thread than when it slept");
	assert_eq!(world.local("napped"), "11");
	// Back in the block that called it, which is all there is
	assert_eq!(world.step("next").proc, "/sc_nap");
	world.request("next", json!({ "threadId": 0 }));
	world.finish("nap");
	assert!(!world.has_event("stopped"), "a step off the end of a sleeping proc's caller stopped somewhere");
	ok("a step in a sleeping proc stops in it when it wakes");

	world.break_on(&["leaf_1"]);
	world.run("nap");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.break_on(&[]);
	let nap = thread_in(&world.threads(), "/nap");
	world.request("stepOut", json!({ "threadId": nap }));
	assert_eq!(world.stop().proc, "/sc_nap");
	world.resume_through("nap");
	ok("stepOut of a sleeping proc stops in its caller once it has woken and returned");

	world.break_on(&["leaf_1"]);
	world.run("many");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.break_on(&[]);
	let threads = world.threads();
	assert_eq!(threads_in(&threads, "/sc_many").len(), 50, "{:?}", threads);
	let mut ids: Vec<u64> = threads.iter().map(|(id, _)| *id).collect();
	ids.sort_unstable();
	ids.dedup();
	assert_eq!(ids.len(), threads.len(), "threads share ids");
	assert!(!world.output.contains("procs are asleep"), "the launcher said the list was cut short");
	world.resume_through("many");
	ok("fifty procs asleep at once are fifty threads");

	// More than the list is given room for
	world.eval_global("many = 150");
	world.break_on(&["leaf_1"]);
	world.run("many");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.break_on(&[]);
	let threads = world.threads();
	assert!(threads[0].1.starts_with("/leaf  "), "{:?}", threads[0]);
	assert_eq!(threads.iter().filter(|(_, name)| name.ends_with(", sleeping")).count(), 100, "{:?}", threads);
	world.threads();
	// Fifty from before, a hundred and fifty now, and the world's loop
	assert_eq!(
		world.count("201 procs are asleep: the list of threads has the first 100 (\"sleepingThreads\" in the launch configuration sets how many)"),
		1,
		"what the launcher said of the list:\n{}",
		world.output
	);
	world.end();
	ok("the list of threads has the proc that's running and the first hundred asleep, and the console says so once");

	let mut world = World::launch(json!({ "sleepingThreads": 500 }));
	world.catch_runtimes(false);
	world.configure();
	world.eval_global("many = 150");
	world.break_on(&["leaf_1"]);
	world.run("many");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.break_on(&[]);
	let threads = world.threads();
	assert_eq!(threads_in(&threads, "/sc_many").len(), 150);
	assert!(!world.output.contains("procs are asleep"), "the launcher said the list was cut short");
	// The last of them is a thread like any other
	let last = threads.last().unwrap().0;
	assert_eq!(world.frames(last)[0]["line"], at.line("many_sleep"));
	world.end();
	ok("sleepingThreads in the launch configuration sets how many sleeping procs are listed");
}
