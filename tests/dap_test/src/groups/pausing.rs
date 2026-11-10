//! Pausing, and the procs asleep while the world is paused: each is a thread

use serde_json::json;

use crate::{client::named, ok, world::World};

pub fn run() {
	let mut world = World::start();

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
	world.break_on(&["leaf_1"]);
	world.run("nap");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.break_on(&[]);
	let threads = world.threads();
	assert_eq!(threads[0], (0, "/leaf".to_owned()));
	let names: Vec<&str> = threads.iter().map(|(_, name)| name.as_str()).collect();
	assert!(names.contains(&"/nap") && names.contains(&"/main_loop"), "{:?}", names);
	assert!(!names.iter().any(|name| name.contains("auxtools")), "{:?}", names);
	ok("each sleeping proc is a thread, but for the debugger's own");

	let nap = threads.iter().find(|(_, name)| name == "/nap").unwrap().0;
	let frames = world.frames(nap);
	let names: Vec<&str> = frames.iter().map(|f| f["name"].as_str().unwrap()).collect();
	assert_eq!(names, ["/nap", "/sc_nap"]);
	ok("a sleeping proc's frames: itself, and who called it");

	let frame = frames[0]["id"].clone();
	let locals = world.scope(&frame, "Locals");
	assert_eq!(crate::client::value_of(&named(&world.variables(&locals)), "napped"), "10");
	let arguments = world.scope(&frame, "Arguments");
	assert_eq!(crate::client::value_of(&named(&world.variables(&arguments)), "ticks"), "10");
	ok("a sleeping proc's locals and arguments");

	assert_eq!(world.evaluate_in(&frame, "napped + ticks", "watch")["result"], "20");
	assert_eq!(world.evaluate_in(&frame, "napped = 11", "repl")["result"], "11");
	ok("an expression in a sleeping proc's frame, and an assignment to its local");

	world.resume_through("nap");

	world.break_on(&["leaf_1"]);
	world.run("many");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.break_on(&[]);
	let threads = world.threads();
	assert_eq!(threads.iter().filter(|(_, name)| name == "/sc_many").count(), 50, "{:?}", threads);
	let mut ids: Vec<u64> = threads.iter().map(|(id, _)| *id).collect();
	ids.dedup();
	assert_eq!(ids.len(), threads.len(), "threads share ids");
	world.resume_through("many");
	ok("fifty procs asleep at once are fifty threads");
	world.end();
}
