use std::time::{Duration, Instant};

use auxtools_protocol::{Request, Response};
use serde_json::json;
use test_support::standin::{StandIn, World};

use super::world::{answers, code, launched, launcher_with, HELD};

const GRACE: Duration = Duration::from_secs(1);

#[track_caller]
fn refuses_to_end(world: &mut World) {
	world.next(
		"world.Del()",
		|request| matches!(request, Request::Eval { command, ending: true, .. } if command == "world.Del()")
	);
}

#[test]
fn shutdown_timeout_terminates_the_process() {
	let (_dir, dme) = code("shutdown_timeout", HELD);
	let mut stand_in = StandIn::new(answers(&dme));
	let launcher = launcher_with(&[("AUXTOOLS_TEST_PATIENCE", "1000")]);
	let (mut launcher, mut world) = launched(&mut stand_in, launcher, &dme, json!({}));
	let asked = Instant::now();
	launcher.request("terminate", json!({}));
	refuses_to_end(&mut world);
	launcher.event("terminated");
	let exited = launcher.event("exited");
	let took = asked.elapsed();
	assert!(exited["body"]["exitCode"].is_number(), "no exit code: {}", exited);
	assert!(
		took >= GRACE && took < GRACE + Duration::from_secs(5),
		"the world was ended after {:?}",
		took
	);
	assert!(launcher.output.contains("Stopping the world"), "{}", launcher.output);
	assert!(
		launcher.output.contains("Shutdown timed out after 1 seconds. Terminating process"),
		"the launcher didn't say why:\n{}",
		launcher.output
	);
	assert!(
		!launcher.output.contains("Waiting to reconnect"),
		"the launcher waited for the debugger of a world it was ending:\n{}",
		launcher.output
	);
	launcher.disconnect(json!({}));
}

#[test]
fn a_process_that_outlives_its_world_is_terminated() {
	let (_dir, dme) = code("outlives_its_world", HELD);
	let mut stand_in = StandIn::new(answers(&dme));
	let launcher = launcher_with(&[("AUXTOOLS_TEST_PATIENCE", "1000")]);
	let (mut launcher, world) = launched(&mut stand_in, launcher, &dme, json!({}));
	let ended = Instant::now();
	world.event(Response::ShuttingDown);
	world.close();
	launcher.event("terminated");
	let exited = launcher.event("exited");
	let took = ended.elapsed();
	assert!(exited["body"]["exitCode"].is_number(), "no exit code: {}", exited);
	assert!(
		took >= GRACE && took < GRACE + Duration::from_secs(5),
		"the process was ended after {:?}",
		took
	);
	assert!(
		launcher
			.output
			.contains("The world ended, but its process is still running after 1 seconds. Terminating process"),
		"the launcher didn't say why:\n{}",
		launcher.output
	);
	assert!(
		!launcher.output.contains("The world restarted"),
		"the launcher took a world that had ended for one that came back:\n{}",
		launcher.output
	);
	launcher.disconnect(json!({}));
}

#[test]
fn stop_cancels_a_pending_relaunch() {
	let (_dir, dme) = code("stop_cancels_a_pending_relaunch", HELD);
	let mut stand_in = StandIn::new(answers(&dme));
	let launcher = launcher_with(&[("AUXTOOLS_TEST_PATIENCE", "1000")]);
	let (mut launcher, mut world) = launched(&mut stand_in, launcher, &dme, json!({}));
	launcher.event("process");
	let changed = stand_in.launch_arguments(&dme, json!({ "sleepingThreads": 5 }));
	launcher.request("restart", json!({ "arguments": changed }));
	refuses_to_end(&mut world);
	launcher.request("terminate", json!({}));
	launcher.wait_output("Restart cancelled: the world is stopping");
	launcher.event("terminated");
	assert!(!launcher.has_event("process"), "the world started again after Stop");
	assert!(
		launcher.output.contains("Shutdown timed out after 1 seconds"),
		"the launcher didn't end the world:\n{}",
		launcher.output
	);
	launcher.disconnect(json!({}));
}
