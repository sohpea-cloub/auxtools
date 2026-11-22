mod groups;
mod world;

use std::sync::atomic::{AtomicUsize, Ordering};

use test_support::paths;

static CHECKS: AtomicUsize = AtomicUsize::new(0);

pub fn ok(what: &str) {
	CHECKS.fetch_add(1, Ordering::Relaxed);
	println!("ok: {}", what);
}

const GROUPS: &[(&str, fn())] = &[
	("basics", groups::basics::run),
	("breakpoints", groups::breakpoints::run),
	("functions", groups::functions::run),
	("conditions", groups::conditions::run),
	("logpoints", groups::logpoints::run),
	("stepping", groups::stepping::run),
	("pausing", groups::pausing::run),
	("frames", groups::frames::run),
	("variables", groups::variables::run),
	("watches", groups::watches::run),
	("expressions", groups::expressions::run),
	("runtimes", groups::runtimes::run),
	("sessions", groups::sessions::run),
	("reboots", groups::reboots::run),
	("restarts", groups::restarts::run),
	("reload_edits", groups::reloading::edits),
	("reload_files", groups::reloading::files),
	("reloading", groups::reloading::run),
	("bytecode", groups::bytecode::run),
	("skins", groups::skins::run),
	("scripts", groups::scripts::run)
];

fn main() {
	let wanted: Vec<String> = std::env::args().skip(1).collect();
	if wanted == ["--list"] {
		for (group, _) in GROUPS {
			println!("{}", group);
		}
		return;
	}
	for name in &wanted {
		assert!(
			GROUPS.iter().any(|(group, _)| group == name),
			"there's no group {}; the groups are {:?}",
			name,
			GROUPS.iter().map(|(group, _)| *group).collect::<Vec<_>>()
		);
	}

	std::env::set_var("AUXTOOLS_WATCH_INTERVAL", "50");

	let _ = std::fs::remove_file(world::host_file("fail_early"));
	paths::compile(&world::host_file("dap_test_host.dme"));

	for (name, run) in GROUPS {
		if wanted.is_empty() || wanted.iter().any(|w| w == name) {
			println!("--- {}", name);
			run();
		}
	}
	println!("all ok: {} checks", CHECKS.load(Ordering::Relaxed));
}
