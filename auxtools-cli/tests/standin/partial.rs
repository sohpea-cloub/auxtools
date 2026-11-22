use auxtools_protocol::{HotReloadResult, JournalEvent, Request, Response};
use test_support::standin::{Answer, StandIn};

use super::world::{answers, attached, code, edit, HELD};

#[test]
fn partly_applied() {
	let (dir, dme) = code("partly_applied", HELD);
	let mut routine = answers(&dme);
	let mut stand_in = StandIn::new(Box::new(move |request| match request {
		Request::JournalAdd(JournalEvent::Reload { .. }) => Answer::ToTest,
		_ => routine(request)
	}));
	let (mut launcher, mut world) = attached(&mut stand_in, &dme);

	edit(&dir, "held.dm", "\treturn 1\n", "\treturn 2\n");
	let reload = world.next("a reload", |request| matches!(request, Request::HotReload(_)));
	world.answer(
		&reload,
		Response::HotReload(HotReloadResult {
			applied: true,
			errors: vec![],
			warnings: vec!["a note".to_owned()],
			failures: vec!["couldn't set a built-in var: no".to_owned()],
			skipped: vec![],
			resources: vec![]
		})
	);
	launcher.wait_output("    warning: a note\n");
	let said = launcher.output.rsplit("Hot reload partly applied: ").next().unwrap();
	assert!(
		said.starts_with("reloaded /held_value in ")
			&& said.contains(
				" ms; restart the world to be sure it runs the new code\n    failed: couldn't set a built-in var: no\n    warning: a note\n"
			),
		"the launcher said: {}",
		launcher.output
	);

	let record = world.next("the reload's record", |request| matches!(request, Request::JournalAdd(_)));
	let Request::JournalAdd(JournalEvent::Reload {
		applied, failures, warnings, ..
	}) = record.request
	else {
		unreachable!()
	};
	assert_eq!(
		(applied, failures, warnings),
		(true, vec!["couldn't set a built-in var: no".to_owned()], vec!["a note".to_owned()])
	);

	let mut state = launcher.event("auxtoolsReload");
	while state["body"]["state"] != "failed" {
		state = launcher.event("auxtoolsReload");
	}
	assert_eq!(state["body"]["summary"], "partly applied: couldn't set a built-in var: no");
}
