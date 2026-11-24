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

	let changes = source.edit("dependents.dm", "DEP_LIMIT = 5", "DEP_LIMIT = 10");
	let chooser = changes.plan.reloads.iter().find(|r| r.proc.path == "/mob/dep_chooser/choose_dep_limit");
	let Some(InList::New(choices)) = chooser.map(|r| &r.params[0].in_list) else {
		panic!("the verb's unchanged `in` list isn't compiled again: {:?}", chooser);
	};
	let code: Vec<_> = (choices.nodes.iter())
		.filter_map(|node| match node {
			dmasm::Node::Instruction(instruction, _) => Some(instruction),
			_ => None
		})
		.collect();
	assert!(
		code.windows(2)
			.any(|pair| matches!(pair, [dmasm::Instruction::PushInt(11), dmasm::Instruction::NewList(1)])),
		"the verb's new `in` list isn't list(11): {}",
		dmasm::format(&choices.nodes)
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /datum/dep_limited).limits[1]"), "11");
	println!("ok: initializers and `in` lists that fold a changed const follow it");

	let changes = source.edit(
		"dependents.dm",
		"/datum/dep_copier\n",
		"/datum/dep_copier\n\tvar/copy = /datum/dep_config::handler\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "call(new /datum/dep_called, (new /datum/dep_config).handler)()"),
		"\"override\""
	);
	assert_eq!(
		eval(&mut client, "call(new /datum/dep_called, (new /datum/dep_copier).vars[\"copy\"])()"),
		"\"override\""
	);
	println!("ok: a new default that reads a type::proc() default names the same proc");

	assert_eq!(eval(&mut client, "dep_next()"), "6");
	let refused = refused_edit(&mut source, "dependents.dm", "DEP_STEP = 5", "DEP_STEP = 10");
	let shown: Vec<String> = refused.iter().map(|r| format!("{:#}", r)).collect();
	assert_eq!(
		shown,
		[
			"/dep_next: folds a const whose value changes, so it must be compiled again, but it doesn't compile: unsupported statement: \
			 type::proc() that names a type's second or later override"
		],
		"a proc dmasm can't compile that folds a changed const isn't refused"
	);
	println!("ok: a reload is refused for a proc dmasm can't compile that folds a changed const");

	let changes = source.edit("dependents.dm", "/datum/dep_searched/speak()\n\treturn \"searched first override\"\n", "");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "dep_calls_searched()"), "\"searched declaration\"");
	println!("ok: code dmasm can't compile that names a declaration by ancestor search stays");

	let changes = source.edit("dependents.dm", "/datum/dep_counted\n\n", "/datum/dep_counted\n\tvar/dep_total = 0\n\n");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /datum/dep_counted).add(5)"), "6");
	println!("ok: a new member var leaves code dmasm can't compile that reads a parameter of its name");
}

fn refused_edit(source: &mut Source, file: &str, from: &str, to: &str) -> Vec<dmasm::reload::Refusal> {
	let text = source.text(file);
	assert_eq!(text.matches(from).count(), 1, "{:?} should be in {} once", from, file);
	std::fs::write(source.dir.join(file), text.replace(from, to)).unwrap();
	let old = source.snapshot.take().expect("parsed before editing");
	source.parse();
	let edited: HashSet<PathBuf> = [Path::new(file).to_owned()].into_iter().collect();
	let changes = diff(&old, source.snapshot.as_ref().unwrap(), Some(&edited));
	std::fs::write(source.dir.join(file), text).unwrap();
	source.snapshot = Some(old);
	changes.refused
}
