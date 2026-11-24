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

	let held = eval(&mut client, "#declared-refs /datum/st_holder");
	let changes = source.edit("statics.dm", "ST_LIMIT = 5", "ST_LIMIT = 10");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "\"[st_limit()] [st_static_limit()] [st_const_limit()]\""), "\"10 7 8\"");
	println!("ok: a global const changes beside a proc's static and const of its name");
	assert_eq!(
		eval(&mut client, "#declared-refs /datum/st_holder"),
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
}
