use auxtools_protocol::Request;
use test_support::{dap::Launcher, standin::StandIn};

use super::world::{answers, attached, code, edit, held, procs, rebooted, reloaded, HELD};

#[track_caller]
fn edit_held(launcher: &mut Launcher, dir: &std::path::Path, file: &str, old: &str, new: &str) {
	let said = |output: &str| output.matches("Hot reload requires a reboot:").count() + output.matches("Hot reload requires a rebuild:").count();
	let before = said(&launcher.output);
	edit(dir, file, old, new);
	launcher.wait_until("the held reload", |output| said(output) > before);
}

#[track_caller]
fn edit_nothing_to_run(launcher: &mut Launcher, dir: &std::path::Path, file: &str, old: &str, new: &str) {
	let before = launcher.count("Hot reload: no changes to the running code");
	edit(dir, file, old, new);
	launcher.wait_count("Hot reload: no changes to the running code", before + 1);
}

fn held_one(message: &str, until: &str) -> Vec<(String, String)> {
	vec![(message.to_owned(), until.to_owned())]
}

#[test]
fn held_until_reboot() {
	let (dir, dme) = code("held_until_reboot", HELD);
	let mut stand_in = StandIn::new(answers(&dme));
	let (mut launcher, mut world) = attached(&mut stand_in, &dme);
	let is_reload = |request: &Request| matches!(request, Request::HotReload(_));

	edit_held(
		&mut launcher,
		&dir,
		"held.dm",
		"var/held_n = 1\nvar/held_k = HELD_K + 1\n\n/proc/held_value()\n\treturn 1\n",
		"var/held_n = 2\nvar/held_k = HELD_K + 1\n\n/proc/held_value()\n\treturn 2\n"
	);
	assert_eq!(held(&mut launcher), held_one("(global): held_n: changed initializer", "reboot"));
	assert!(world.next_within(super::SHORTLY, is_reload).is_none(), "a held edit was sent");

	edit(&dir, "held.dm", "var/held_n = 2\n", "var/held_n = 1\n");
	let reload = reloaded(&mut world);
	assert_eq!(
		(procs(&reload).len(), procs(&reload)[0].0.as_str(), reload.global_init.is_some()),
		(1, "/held_value", false),
		"{:?}",
		reload
	);
	assert!(procs(&reload)[0].1.contains("PushInt(2)"), "{:?}", procs(&reload));
	launcher.wait_output("Hot reload: reloaded /held_value");
	assert_eq!(held(&mut launcher), []);

	edit_held(&mut launcher, &dir, "held.dm", "var/held_n = 1\n", "var/held_n = 3\n");
	edit_held(&mut launcher, &dir, "held.dm", "\treturn 2\n", "\treturn 4\n");
	assert!(world.next_within(super::SHORTLY, is_reload).is_none(), "a held edit was sent");
	let mut world = rebooted(&stand_in, &mut launcher, world);
	let restored = reloaded(&mut world);
	assert_eq!(procs(&restored).len(), 1, "{:?}", restored);
	assert!(procs(&restored)[0].1.contains("PushInt(4)"), "{:?}", procs(&restored));
	let init = format!(
		"{:?}",
		restored.global_init.as_ref().expect("the reboot left the initializer out").code.nodes
	);
	assert!(init.contains("PushInt(3)"), "the world's initializer: {}", init);
	launcher.wait_output("The world restarted");
	assert_eq!(held(&mut launcher), []);

	edit_held(&mut launcher, &dir, "held.dm", "var/held_n = 3\n", "var/held_n = 5\n");
	edit_held(&mut launcher, &dir, "held_b.dm", "\treturn 1\n", "\treturn 6\n");
	assert!(world.next_within(super::SHORTLY, is_reload).is_none(), "a held edit was sent");
	assert_eq!(held(&mut launcher), held_one("(global): held_n: changed initializer", "reboot"));
	edit(&dir, "held.dm", "var/held_n = 5\n", "var/held_n = 3\n");
	let reload = reloaded(&mut world);
	assert_eq!(
		procs(&reload).iter().map(|(proc, _)| proc.as_str()).collect::<Vec<_>>(),
		["/held_b"],
		"{:?}",
		reload
	);
	assert!(procs(&reload)[0].1.contains("PushInt(6)"), "{:?}", procs(&reload));
	assert_eq!(held(&mut launcher), []);

	edit_held(&mut launcher, &dir, "held_b.dm", "#define HELD_K 1\n", "#define HELD_K 2\n");
	assert_eq!(held(&mut launcher), held_one("(global): held_k: changed initializer", "reboot"));
	edit_nothing_to_run(&mut launcher, &dir, "held_b.dm", "#define HELD_K 2\n", "#define HELD_K 1\n");
	assert_eq!(held(&mut launcher), []);

	edit_held(
		&mut launcher,
		&dir,
		"held.dm",
		"var/held_n = 3\n",
		"var/held_n = 3\n/mob/refused\n\tckey = \"someone\"\n"
	);
	let refused = held(&mut launcher);
	assert!(
		refused.len() == 1 && refused[0].0.contains("/mob/refused: ckey") && refused[0].1 == "rebuild",
		"{:?}",
		refused
	);
	edit_held(&mut launcher, &dir, "held_b.dm", "\treturn 6\n", "\treturn 7\n");
	assert_eq!(held(&mut launcher), refused);
	assert!(world.next_within(super::SHORTLY, is_reload).is_none(), "a held edit was sent");
	edit(&dir, "held.dm", "/mob/refused\n\tckey = \"someone\"\n", "");
	let reload = reloaded(&mut world);
	assert_eq!(
		procs(&reload).iter().map(|(proc, _)| proc.as_str()).collect::<Vec<_>>(),
		["/held_b"],
		"{:?}",
		reload
	);
	assert!(procs(&reload)[0].1.contains("PushInt(7)"), "{:?}", procs(&reload));
	assert_eq!(held(&mut launcher), []);

	edit_held(
		&mut launcher,
		&dir,
		"held.dm",
		"var/held_n = 3\n",
		"var/held_n = 3\n\n/world\n\tfps = 25\n"
	);
	assert_eq!(held(&mut launcher), held_one("/world: fps: changed setting", "rebuild"));
	assert!(launcher.output.contains("Hot reload requires a rebuild:"), "{}", launcher.output);
	edit_nothing_to_run(&mut launcher, &dir, "held.dm", "\n\n/world\n\tfps = 25\n", "\n");
	assert_eq!(held(&mut launcher), []);
	assert!(world.next_within(super::SHORTLY, is_reload).is_none(), "a held edit was sent");
	launcher.disconnect(serde_json::json!({}));
}
