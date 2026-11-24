use super::*;

pub fn dependents(built: Built) {
	let (mut source, _world, mut client, mut watcher) = start(built);

	const MADE: &str = "\"[(new /datum/dep_holder).item.type]\"";
	assert_eq!(eval(&mut client, MADE), "\"/datum/dep_made\"");
	let changes = source.edit("dependents.dm", "var/datum/dep_made/item = new", "var/datum/dep_made/other/item = new");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, MADE), "\"/datum/dep_made/other\"");
	println!("ok: an implicit new makes the var's new declared type");

	const FACTORY_MAKES: &str = "\"[(new /datum/dep_factory).make().type]\"";
	assert_eq!(eval(&mut client, FACTORY_MAKES), "\"/datum/dep_made\"");
	let changes = source.edit(
		"dependents.dm",
		"/datum/dep_factory\n\n",
		"/datum/dep_factory\n\tvar/dep_item_type = /datum/dep_made/other\n\tvar/dep_count = 0\n\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, FACTORY_MAKES), "\"/datum/dep_made/other\"");
	assert_eq!(eval(&mut client, "dep_factory_counts()"), "\"7 100\"");
	println!("ok: new member vars take an unchanged proc's new and loop uses of globals' names");
}
