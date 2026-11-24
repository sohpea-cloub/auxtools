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
}
