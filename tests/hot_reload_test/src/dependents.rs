use super::*;

pub fn dependents(built: Built) {
	let (mut source, _world, mut client, mut watcher) = start(built);

	const MADE: &str = "\"[(new /datum/dep_holder).item.type]\"";
	assert_eq!(eval(&mut client, MADE), "\"/datum/dep_made\"");
	let changes = source.edit("dependents.dm", "var/datum/dep_made/item = new", "var/datum/dep_made/other/item = new");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, MADE), "\"/datum/dep_made/other\"");
	println!("ok: an implicit new makes the var's new declared type");
}
