use super::*;

pub fn statics(built: Built) {
	let (mut source, _world, mut client, mut watcher) = start(built);

	assert_eq!(eval(&mut client, "st_add_total()"), "1");
	let changes = source.edit(
		"statics.dm",
		"/proc/st_add_total()\n",
		"/proc/st_add_total()\n\tvar/static/st_total = 0\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "\"[st_add_total()] [st_total]\""), "\"1 1\"");
	println!("ok: a new static local beside a global of its name gets a slot of its own");

	let held = declared_references(&mut client, "/datum/st_holder");
	let changes = source.edit("statics.dm", "ST_LIMIT = 5", "ST_LIMIT = 10");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "\"[st_limit()] [st_static_limit()] [st_const_limit()]\""), "\"10 7 8\"");
	println!("ok: a global const changes beside a proc's static and const of its name");
	assert_eq!(
		declared_references(&mut client, "/datum/st_holder"),
		held,
		"the declared vars of a type that the change of a global doesn't touch"
	);
	println!("ok: looking for a type's var of a global's name leaves the type's tables as they were");

	assert_eq!(eval(&mut client, "st_mixed()"), "\"8 1\"");
	let changes = source.edit("statics.dm", "ST_MIXED++", "ST_MIXED += 2");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "\"[st_mixed()] [st_mixed_global()]\""), "\"10 1 1\"");
	println!("ok: a static that the old code read beside a global of its name keeps its slot");

	assert_eq!(eval(&mut client, "st_scoped()"), "1");
	let changes = source.edit("statics.dm", "return global.ST_SCOPED", "return ST_SCOPED");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "\"[st_scoped()] [st_scoped_global()]\""), "\"5 1\"");
	println!("ok: a local const that the old code didn't read leaves the global of its name as it was");

	assert_eq!(eval(&mut client, "\"[st_counter()] [st_counter()]\""), "\"1 2\"");
	let changes = source.edit("statics.dm", "\tif (/datum/st_twice::f())\n\t\treturn st_count", "\treturn st_count");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "st_counter()"), "3");
	println!("ok: a static keeps its value when its proc's old code doesn't compile");

	let both = "\"[st_both_root()] [st_both()[1]] [st_both()[2]]\"";
	assert_eq!(eval(&mut client, both), "\"5 7 5\"");
	let changes = source.edit("statics.dm", "ST_BOTH = 5", "ST_BOTH = 10");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, both), "\"10 7 10\"");
	println!("ok: a global const changes beside a proc that reads both it and a static of its name");

	assert_eq!(eval(&mut client, "\"[st_stateful()] [st_registered]\""), "\"2 1\"");
	let changes = source.edit("statics.dm", "global.st_state + 1", "global.st_state + 2");
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "\"[st_stateful()] [st_registered]\""),
		"\"3 1\"",
		"the proc's result and the runs of the initializer of a static that neither code reads"
	);
	println!("ok: an edit leaves alone a static that neither the old nor the new code reads");

	assert_eq!(eval(&mut client, "st_unknown()"), "1");
	let changes = source.edit(
		"statics.dm",
		"\tif (/datum/st_twice::f())\n\t\treturn global.ST_UNKNOWN",
		"\treturn ST_UNKNOWN"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "\"[st_unknown()] [st_unknown_global()]\""), "\"5 1\"");
	println!("ok: a local const leaves the global of its name as it was when its proc's old code doesn't compile");

	assert_eq!(eval(&mut client, "\"[st_defaulted()[1]] [st_defaulted()[2]]\""), "\"6 1\"");
	let changes = source.edit(
		"statics.dm",
		"\tif (/datum/st_twice::f())\n\t\treturn list(ST_DEFAULT + 1, x)",
		"\treturn ST_DEFAULT"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "\"[st_defaulted()] [st_default_global()]\""), "\"5 1\"");
	println!("ok: a local const leaves the global of its name as it was when a parameter default reads the global");

	assert_eq!(eval(&mut client, "\"[st_branch(1)] [st_branch(1)]\""), "\"1 2\"");
	let changes = source.edit(
		"statics.dm",
		"\tif (which == 1)\n\t\tvar/static/st_branch_count = 0",
		"\tif (which == 2)\n\t\tvar/static/st_branch_count = 10\n\t\tst_branch_count++\n\t\treturn st_branch_count\n\tif (which == \
		 1)\n\t\tvar/static/st_branch_count = 0"
	);
	assert!(
		changes.deferred.is_empty(),
		"the added branch's static waits for a reboot: {:?}",
		changes.deferred
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "\"[st_branch(1)] [st_branch(2)]\""),
		"\"3 11\"",
		"the counts of the old branch's static and of the added branch's"
	);
	println!("ok: a branch added above another with a static of the same name leaves the other's value with it");

	let renamed = "\"[st_renamed_root()] [st_renamed()]\"";
	assert_eq!(eval(&mut client, renamed), "\"5 8\"");
	let changes = source.edit(
		"statics.dm",
		"\tvar/static/ST_RENAMED = 7\n\tST_RENAMED++\n\treturn ST_RENAMED",
		"\tvar/static/st_renamed_count = 7\n\tst_renamed_count++\n\treturn st_renamed_count"
	);
	reload(&mut watcher, &changes);
	let changes = source.edit("statics.dm", "ST_RENAMED = 5", "ST_RENAMED = 10");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, renamed), "\"10 8\"");
	println!("ok: a global const changes after a static of its name was renamed");

	assert_eq!(eval(&mut client, "\"[st_seeded()] [st_seeded()]\""), "\"2 3\"");
	let changes = source.edit(
		"statics.dm",
		"\tst_seed++\n\treturn st_seed",
		"\tvar/static/list/st_cache = list(st_seed)\n\tst_seed++\n\treturn \"[st_seed] [st_cache[1]]\""
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "st_seeded()"), "\"4 3\"");
	println!("ok: a new static's initializer reads an earlier static of its proc");

	assert_eq!(eval(&mut client, "\"[st_gone_root()] [(new /datum/st_gone).count()]\""), "\"5 8\"");
	let changes = source.edit(
		"statics.dm",
		"\n/datum/st_gone/proc/count()\n\tvar/static/ST_GONE = 7\n\tST_GONE++\n\treturn ST_GONE\n",
		""
	);
	reload(&mut watcher, &changes);
	let changes = source.edit("statics.dm", "ST_GONE = 5", "ST_GONE = 10");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "st_gone_root()"), "10");
	println!("ok: a global const changes after a proc with a static of its name was removed");

	let tick = "(new /datum/st_over).tick()";
	assert_eq!(eval(&mut client, &format!("\"[{tick}] [{tick}]\"")), "\"2 4\"");
	let changes = source.edit_many("statics.dm", &[
		("ST_OVER_STEP = 1", "ST_OVER_STEP = 2"),
		(
			"\n/datum/st_over/tick()\n",
			"\n/datum/st_over/tick()\n\treturn ..()\n\n/datum/st_over/tick()\n"
		)
	]);
	assert!(changes.deferred.is_empty(), "the edit waits for a reboot: {:?}", changes.deferred);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, tick), "7");
	println!("ok: an override added above another that a changed const recompiles keeps the other's static");

	assert_eq!(eval(&mut client, "st_armed(1)"), "2");
	let changes = source.edit(
		"statics.dm",
		"\t\treturn st_arm\n\treturn 0",
		"\t\treturn st_arm\n\tvar/static/list/st_arm_cache = list(st_arm)\n\treturn st_arm_cache[1]"
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "\"[st_armed(0)] [st_armed(1)]\""),
		"\"20 3\"",
		"the new static's value from the global, and the count of the static in the block before it"
	);
	println!("ok: a new static's initializer reads a global when a static of its name is in a block that has ended");

	assert_eq!(eval(&mut client, "st_sown()"), "1");
	let changes = source.edit(
		"statics.dm",
		"\tvar/const/ST_SOWN = 1\n\treturn ST_SOWN",
		"\tvar/const/ST_SOWN = 2\n\tvar/static/list/st_sown_cache = list(ST_SOWN)\n\treturn st_sown_cache"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "st_sown()[1]"), "2", "the new static's value from the changed const");
	println!("ok: a new static's initializer reads the new value of a local const that the edit changes");
}

fn declared_references(client: &mut Client, path: &str) -> (u32, u32) {
	let answer = eval(client, &format!("#declared-refs {}", path));
	let parsed = answer
		.strip_prefix("table ")
		.and_then(|rest| rest.strip_suffix(" references"))
		.and_then(|rest| rest.split_once(": "))
		.and_then(|(table, references)| Some((table.parse().ok()?, references.parse().ok()?)));
	parsed.unwrap_or_else(|| panic!("expected the declared vars of {} and their references, got {}", path, answer))
}
