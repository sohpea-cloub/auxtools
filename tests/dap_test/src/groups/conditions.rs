use serde_json::json;

use crate::{ok, world::World};

pub fn run() {
	let mut world = World::start();

	world.break_if(&[("leaf_1", "x == 8")]);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	assert_eq!(world.argument("x"), "8");
	world.resume_through("middle");
	ok("argument condition");

	world.break_if(&[("leaf_1", "x == 999")]);
	world.run_through("middle");
	ok("always-false condition");

	world.break_if(&[("leaf_2", "y == 4")]);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	assert_eq!(world.local("y"), "4");
	world.resume_through("middle");
	ok("local condition");

	world.break_if(&[("bump_1", "n == 1 && global_number == 5")]);
	world.run("parent");
	world.stop_on("breakpoint", "/datum/thing/bump", "bump_1");
	world.resume_through("parent");
	world.break_if(&[("bump_1", "src.n == 2 || global.global_number == 6")]);
	world.run_through("parent");
	ok("src member and global conditions");

	world.break_if(&[("leaf_2", ""), ("middle_3", "leaf(1) == 4")]);
	world.run("middle");
	for _ in 0..2 {
		world.stop_on("breakpoint", "/leaf", "leaf_2");
		assert_eq!(world.frames(0).len(), 4, "stopped in the condition's own call");
		world.resume();
	}
	world.stop_on("breakpoint", "/middle", "middle_3");
	world.resume_through("middle");
	ok("no recursive stops during condition evaluation");

	world.break_if(&[("leaf_1", "global_number++ && 0")]);
	world.run_through("middle");
	assert_eq!(world.eval_global("global_number"), "7");
	world.eval_global("global_number = 5");
	ok("condition side effects run once per hit");

	let failed = "` failed: ";
	world.break_if(&[("leaf_1", "x.len > 3")]);
	for _ in 0..3 {
		world.run_through("middle");
	}
	let (_, line) = world.marks.place("leaf_1");
	let said = format!(
		"Breakpoint at stepping.dm:{}: the condition `x.len > 3` failed: runtime error: Cannot read 3.len. Hits where it fails are skipped. This \
		 message is shown once.",
		line
	);
	assert_eq!(world.count(&said), 1, "expected exactly one diagnostic matching {:?}", said);
	assert_eq!(world.count("This message is shown once."), 1, "diagnostic was reported more than once");
	ok("condition runtime errors reported once");

	world.break_if(&[("leaf_1", "x == 8 || x.len")]);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	assert_eq!(world.argument("x"), "8");
	world.resume_through("middle");
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.resume_through("middle");
	assert_eq!(world.count(failed), 2);
	ok("conditions that fail on some hits");

	let set = world.try_break_if(&[("leaf_1", "x =="), ("leaf_2", "y == 4"), ("middle_3", "nowhere_at_all == 1")]);
	assert_eq!(set[0]["verified"], false, "{}", set[0]);
	let why = set[0]["message"].as_str().unwrap();
	assert!(why.starts_with("the condition doesn't compile: parser error: "), "{}", why);
	assert_eq!((&set[1]["verified"], &set[2]["verified"]), (&json!(true), &json!(true)));
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_2");
	world.resume_through("middle");
	let undefined = format!("{}undefined var: nowhere_at_all", failed);
	world.wait_count(&undefined, 1);
	assert_eq!(world.count(&undefined), 1);
	let set = world.request(
		"setFunctionBreakpoints",
		json!({ "breakpoints": [{ "name": "/proc/leaf", "condition": "x x" }, { "name": "/proc/middle" }] })
	);
	assert_eq!(set["breakpoints"][0]["verified"], false);
	assert!(set["breakpoints"][0]["message"]
		.as_str()
		.unwrap()
		.starts_with("the condition doesn't compile: "));
	assert_eq!(set["breakpoints"][1]["verified"], true);
	world.function_breakpoints(&[]);
	ok("condition compile errors at installation");

	world.break_on(&["leaf_1"]);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.resume();
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	world.resume_through("middle");
	ok("removing a condition");

	hit_counts(&mut world);
	world.end();
}

fn stops_with(world: &mut World, hit: &str, condition: Option<&str>) -> Vec<String> {
	let (file, line) = world.marks.place("recurse_1");
	let set = world.set_breakpoints(&file, json!([{ "line": line, "hitCondition": hit, "condition": condition }]));
	assert_eq!(set[0]["verified"], true, "{}", set[0]);
	stops_of(world)
}

fn stops_of(world: &mut World) -> Vec<String> {
	world.run_collecting("recurse", |world| world.argument("depth"))
}

fn resume_collecting(world: &mut World, scenario: &str, finished: usize) -> Vec<String> {
	let over = format!("dap: finished {}\n", scenario);
	let deadline = std::time::Instant::now() + test_support::dap::TIMEOUT;
	let mut seen = vec![];
	world.resume();
	loop {
		if world.event_within("stopped", std::time::Duration::from_millis(20)).is_some() {
			seen.push(world.argument("depth"));
			world.resume();
		} else if world.count(&over) > finished {
			return seen;
		}
		assert!(std::time::Instant::now() < deadline, "timed out waiting for {}", scenario);
	}
}

fn hit_counts(world: &mut World) {
	assert_eq!(stops_with(world, "3", None), ["1", "0"]);
	assert_eq!(stops_with(world, ">= 2", None), ["2", "1", "0"]);
	ok("hit-count threshold");

	assert_eq!(stops_with(world, "== 2", None), ["2"]);
	assert_eq!(stops_with(world, "=4", None), ["0"]);
	assert_eq!(stops_with(world, "> 3", None), ["0"]);
	assert_eq!(stops_with(world, "< 3", None), ["3", "2"]);
	assert_eq!(stops_with(world, "<= 1", None), ["3"]);
	assert_eq!(stops_with(world, "% 2", None), ["2", "0"]);
	assert_eq!(stops_with(world, "9", None), Vec::<String>::new());
	ok("hit-count operators");

	assert_eq!(stops_with(world, "2", Some("depth != 2")), ["1", "0"]);
	assert_eq!(stops_with(world, "% 2", Some("depth < 3")), ["1"]);
	ok("count only hits passing the condition");

	let (file, line) = world.marks.place("recurse_1");
	world.set_breakpoints(&file, json!([{ "line": line, "hitCondition": "% 2" }]));
	world.function_breakpoints(&["/proc/sc_recurse"]);
	let finished = world.count("dap: finished recurse\n");
	world.run("recurse");
	world.stopped("breakpoint");
	world.function_breakpoints(&[]);
	world.step_on("stepIn", "/recurse", "recurse_1");
	assert_eq!(world.argument("depth"), "3");
	assert_eq!(resume_collecting(world, "recurse", finished), ["2", "0"]);
	ok("step and breakpoint share one stop");

	assert_eq!(stops_with(world, "== 6", None), Vec::<String>::new());
	assert_eq!(stops_of(world), ["2"]);
	assert_eq!(stops_of(world), Vec::<String>::new());
	ok("hit counts persist between runs");

	let (file, line) = world.marks.place("recurse_1");
	world.set_breakpoints(&file, json!([]));
	assert_eq!(stops_with(world, "== 6", None), Vec::<String>::new());
	let plain = world.marks.line("plain_1");
	let set = world.set_breakpoints(&file, json!([{ "line": line, "hitCondition": "== 6" }, { "line": plain }]));
	assert_eq!((&set[0]["verified"], &set[1]["verified"]), (&json!(true), &json!(true)), "{:?}", set);
	assert_eq!(stops_of(world), ["2"]);
	world.set_breakpoints(&file, json!([]));
	let on_recurse = json!({ "name": "/proc/recurse", "hitCondition": "== 6" });
	world.request("setFunctionBreakpoints", json!({ "breakpoints": [on_recurse] }));
	assert_eq!(stops_of(world), Vec::<String>::new());
	world.request(
		"setFunctionBreakpoints",
		json!({ "breakpoints": [on_recurse, { "name": "/proc/sc_plain" }] })
	);
	assert_eq!(stops_of(world), ["2"]);
	world.function_breakpoints(&[]);
	ok("adding a breakpoint keeps the others' hit counts");

	let (file, line) = world.marks.place("recurse_1");
	for (hit, why) in [
		("soon", "invalid hit count \"soon\"; expected"),
		("% 0", "hit interval must be greater than zero"),
		("== two", "invalid hit count \"== two\"; expected")
	] {
		let set = world.set_breakpoints(&file, json!([{ "line": line, "hitCondition": hit }]));
		assert_eq!(set[0]["verified"], false, "{}", set[0]);
		assert!(set[0]["message"].as_str().unwrap().starts_with(why), "{}", set[0]);
	}
	world.set_breakpoints(&file, json!([]));
	ok("invalid hit-count diagnostic");

	let set = world.request(
		"setFunctionBreakpoints",
		json!({ "breakpoints": [{ "name": "/proc/leaf", "hitCondition": "2" }] })
	);
	assert_eq!(set["breakpoints"][0]["verified"], true);
	world.run("middle");
	world.stop_on("breakpoint", "/leaf", "leaf_1");
	assert_eq!(world.argument("x"), "8");
	world.resume_through("middle");
	world.function_breakpoints(&[]);
	ok("function breakpoint hit count");
}
