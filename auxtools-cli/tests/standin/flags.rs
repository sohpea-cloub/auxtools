use auxtools_protocol::Request;
use serde_json::json;
use test_support::{dap::Launcher, standin::StandIn};

use super::world::{answers, code, edit, held, launched, launcher, procs, rebooted, reboots, reloaded};

const FLAGGED: &[(&str, &str)] = &[
	("flags.dme", "#include \"flags.dm\"\n"),
	(
		"flags.dm",
		"/proc/flag_value()
	return 1

#ifdef FLAG_A
/proc/flagged()
	return \"a\"

/datum/flag_only
	var/n = 3
#else
/proc/flagged()
	return \"none\"
#endif

#ifdef FLAG_MAP
#include \"flag_map.dmm\"
#endif
"
	),
	("flag_map.dmm", "\"a\" = (/turf,/area)\n\n(1,1,1) = {\"\na\n\"}\n")
];

#[track_caller]
fn flags_held(launcher: &mut Launcher, flags: &[&str]) {
	let said = |output: &str| output.matches("Hot reload requires a reboot:").count() + output.matches("Hot reload requires a rebuild:").count();
	let before = said(&launcher.output);
	launcher.request("auxtoolsBuildFlags", json!({ "flags": flags }));
	launcher.wait_until("the held build flags", |output| said(output) > before);
}

fn added(flag: &str) -> Vec<(String, String)> {
	vec![(format!("build flags changed: added {}", flag), "reboot".to_owned())]
}

fn reloaded_procs(reload: &auxtools_protocol::HotReloadRequest) -> Vec<String> {
	procs(reload).into_iter().map(|(proc, _)| proc).collect()
}

#[test]
fn held_for_build_flags() {
	let (dir, dme) = code("held_for_build_flags", FLAGGED);
	let mut stand_in = StandIn::new(answers(&dme));
	let more = json!({ "buildFlags": [], "hotReload": true });
	let (mut launcher, mut world) = launched(&mut stand_in, launcher(), &dme, more.clone());
	let is_reload = |request: &Request| matches!(request, Request::HotReload(_));

	flags_held(&mut launcher, &["FLAG_A"]);
	assert_eq!(held(&mut launcher), added("FLAG_A"));
	let before = launcher.count("Hot reload requires a reboot:");
	edit(&dir, "flags.dm", "\treturn 1\n", "\treturn 2\n");
	launcher.wait_count("Hot reload requires a reboot:", before + 1);
	assert!(world.next_within(super::SHORTLY, is_reload).is_none(), "a held edit was sent");

	launcher.request("auxtoolsBuildFlags", json!({ "flags": [] }));
	let reload = reloaded(&mut world);
	assert_eq!(reloaded_procs(&reload), ["/flag_value"], "{:?}", reload);
	assert!(procs(&reload)[0].1.contains("PushInt(2)"), "{:?}", procs(&reload));
	assert_eq!(held(&mut launcher), []);

	flags_held(&mut launcher, &["FLAG_A"]);
	let mut world = rebooted(&stand_in, &mut launcher, world);
	let restored = reloaded(&mut world);
	let (with_a, types) = (procs(&restored), format!("{:?}", restored.plan.new_types));
	assert!(
		with_a.iter().any(|(proc, code)| proc == "/flagged" && code.contains("\"a\"")) && types.contains("/datum/flag_only"),
		"the reboot restored {:?}",
		restored
	);
	assert_eq!(held(&mut launcher), []);
	edit(&dir, "flags.dm", "\treturn \"a\"\n", "\treturn \"b\"\n");
	let reload = reloaded(&mut world);
	assert_eq!(reloaded_procs(&reload), ["/flagged"], "{:?}", reload);
	assert!(procs(&reload)[0].1.contains("\"b\""), "{:?}", procs(&reload));

	flags_held(&mut launcher, &[]);
	assert_eq!(held(&mut launcher), [(
		"build flags changed: removed FLAG_A".to_owned(),
		"reboot".to_owned()
	)]);
	let mut world = rebooted(&stand_in, &mut launcher, world);
	let restored = reloaded(&mut world);
	assert_eq!(
		(reloaded_procs(&restored), restored.plan.new_types.len()),
		(vec!["/flag_value".to_owned()], 0),
		"the reboot restored {:?}",
		restored
	);
	assert_eq!(held(&mut launcher), []);

	let said = |output: &str| output.matches("no changes to the running code").count();
	let before = said(&launcher.output);
	launcher.request("auxtoolsBuildFlags", json!({ "flags": ["FLAG_UNUSED"] }));
	launcher.wait_until("the unchanged code", |output| said(output) > before);
	assert_eq!(held(&mut launcher), []);

	flags_held(&mut launcher, &["FLAG_UNUSED", "FLAG_MAP"]);
	assert_eq!(held(&mut launcher), [
		("build flags changed: added FLAG_MAP".to_owned(), "reboot".to_owned()),
		("compiled maps changed: added flag_map.dmm".to_owned(), "rebuild".to_owned())
	]);

	launcher.request("auxtoolsBuildFlags", json!({ "flags": ["FLAG_A"] }));
	let dmb = dme.with_extension("dmb");
	std::fs::File::options()
		.write(true)
		.open(&dmb)
		.unwrap()
		.set_modified(std::time::SystemTime::now())
		.unwrap();
	launcher.drop_events("process");
	launcher.drop_events("auxtoolsReload");
	launcher.request("restart", json!({ "arguments": stand_in.launch_arguments(&dme, more) }));
	let mut world = reboots(&stand_in, world);
	launcher.wait_output("using the rebuilt .dmb");
	while launcher.event("auxtoolsReload")["body"]["state"] != "active" {}
	assert!(!launcher.has_event("process"), "the restart relaunched the world");
	assert!(
		world.next_within(super::SHORTLY, is_reload).is_none(),
		"the rebuilt world was sent its own code"
	);
	edit(&dir, "flags.dm", "\treturn \"b\"\n", "\treturn \"c\"\n");
	let reload = reloaded(&mut world);
	assert_eq!(reloaded_procs(&reload), ["/flagged"], "{:?}", reload);
	assert!(procs(&reload)[0].1.contains("\"c\""), "{:?}", procs(&reload));
	assert_eq!(held(&mut launcher), []);
	launcher.disconnect(json!({}));
}
