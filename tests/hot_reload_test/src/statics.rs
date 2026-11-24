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
}
