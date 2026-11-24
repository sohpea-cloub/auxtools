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
	assert_eq!(eval(&mut client, "(new /datum/dep_counted).add_default(5)"), "6");
	println!("ok: a new member var leaves code dmasm can't compile that reads a parameter of its name");

	assert_eq!(eval(&mut client, "dep_qualified() + dep_dotted()"), "12");
	let refused = refused_edit(&mut source, "dependents.dm", "DEP_QUALIFIED = 5", "DEP_QUALIFIED = 10");
	let mut paths: Vec<&str> = refused.iter().map(|r| r.path.as_str()).collect();
	paths.sort();
	assert_eq!(
		paths,
		["/dep_dotted", "/dep_qualified"],
		"a proc dmasm can't compile that folds a changed const as ::name or global.name isn't refused: {:?}",
		refused
	);
	println!("ok: a reload is refused for a proc dmasm can't compile that folds a changed const as ::name or global.name");

	assert_eq!(eval(&mut client, "dep_change_static_handler()"), "\"changed\"");
	let changes = source.edit(
		"dependents.dm",
		"/datum/dep_static_copier\n",
		"/datum/dep_static_copier\n\tvar/static/copy = /datum/dep_static_config::handler\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "/datum/dep_static_copier::copy"), "\"changed\"");
	println!("ok: a new static that reads a static with :: takes its value at run time");

	let changes = source.edit("dependents.dm", "var/datum/dep_choices_a/target", "var/datum/dep_choices_b/target");
	let chooser = changes.plan.reloads.iter().find(|r| r.proc.path == "/mob/dep_targeted/choose_dep_target");
	let Some(InList::New(choices)) = chooser.map(|r| &r.params[0].in_list) else {
		panic!("the verb's unchanged `in` list isn't compiled again: {:?}", chooser);
	};
	let code = format!("{:?}", choices.nodes);
	assert!(
		code.contains("/datum/dep_choices_b") && !code.contains("/datum/dep_choices_a"),
		"the verb's new `in` list doesn't read /datum/dep_choices_b's static: {}",
		code
	);
	reload(&mut watcher, &changes);
	println!("ok: an `in` list that reads a static through a retyped var reads the new type's");

	let changes = source.edit("dependents.dm", "DEP_OWN = 5", "DEP_OWN = 10");
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(
			&mut client,
			"\"[dep_own_global()] [(new /datum/dep_own).own()] [dep_own_global_alone()] [(new /datum/dep_own).own_alone()]\""
		),
		"\"11 3 10 2\""
	);
	println!("ok: a global const changes beside a member const of its name");
	assert_eq!(eval(&mut client, "(new /datum/dep_own).next()"), "3");
	println!("ok: a changed const leaves code dmasm can't compile that reads a member const of its name");

	assert_eq!(eval(&mut client, "dep_member_chain(new /datum/dep_member_config)"), "6");
	let refused = refused_edit(&mut source, "dependents.dm", "DEP_MEMBER = 5", "DEP_MEMBER = 10");
	let paths: Vec<&str> = refused.iter().map(|r| r.path.as_str()).collect();
	assert_eq!(
		paths,
		["/dep_member_chain"],
		"a proc dmasm can't compile that folds a changed member const through a typed var isn't refused: {:?}",
		refused
	);
	println!("ok: a reload is refused for a proc dmasm can't compile that folds a changed member const through a typed var");

	assert_eq!(eval(&mut client, "\"[dep_alone()] [dep_alone_sum()]\""), "\"5 6\"");
	let refused = refused_edit(&mut source, "dependents.dm", "DEP_ALONE = 5", "DEP_ALONE = 10");
	let paths: Vec<&str> = refused.iter().map(|r| r.path.as_str()).collect();
	assert_eq!(
		paths,
		["/dep_alone_sum"],
		"a changed const read alone in code dmasm can't compile refuses the reload, or one it folds doesn't: {:?}",
		refused
	);
	let changes = source.edit(
		"dependents.dm",
		"DEP_ALONE = 5\n\n/proc/dep_alone_sum()\n\tvar/last = /datum/dep_twice::f()\n\treturn last ? ::DEP_ALONE + 1 : 0",
		"DEP_ALONE = 10\n\n/proc/dep_alone_sum()\n\treturn ::DEP_ALONE + 1"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "\"[dep_alone()] [dep_alone_sum()]\""), "\"10 11\"");
	println!("ok: code dmasm can't compile that reads a changed const alone sees its new value");

	assert_eq!(eval(&mut client, "\"[dep_local_limit()] [dep_local_based()]\""), "\"5 6 6\"");
	let changes = source.edit("dependents.dm", "DEP_LOCAL = 5", "DEP_LOCAL = 10");
	reload(&mut watcher, &changes);
	let changes = source.edit("dependents.dm", "DEP_LOCAL_BASE = 5", "DEP_LOCAL_BASE = 10");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "\"[dep_local_limit()] [dep_local_based()]\""), "\"10 11 11\"");
	println!("ok: a changed local const is read alone and folded with its new value");

	assert_eq!(eval(&mut client, "dep_output()"), "5");
	let changes = source.edit("dependents.dm", "DEP_OUTPUT = 5", "DEP_OUTPUT = 10");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "dep_output()"), "10");
	println!("ok: code dmasm can't compile that outputs a changed const sees its new value");

	assert_eq!(eval(&mut client, "dep_global_chain()"), "6");
	let refused = refused_edit(&mut source, "dependents.dm", "DEP_GLOBAL_MEMBER = 5", "DEP_GLOBAL_MEMBER = 10");
	let paths: Vec<&str> = refused.iter().map(|r| r.path.as_str()).collect();
	assert_eq!(
		paths,
		["/dep_global_chain"],
		"a proc dmasm can't compile that folds a changed member const through a typed global isn't refused: {:?}",
		refused
	);
	println!("ok: a reload is refused for a proc dmasm can't compile that folds a changed member const through a typed global");

	assert_eq!(eval(&mut client, "\"[dep_local_offset()] [dep_local_default()]\""), "\"6 5\"");
	let changes = source.edit("dependents.dm", "DEP_OFFSET_BASE = 5", "DEP_OFFSET_BASE = 10");
	reload(&mut watcher, &changes);
	let changes = source.edit("dependents.dm", "dep_limit = 5", "dep_limit = 10");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "\"[dep_local_offset()] [dep_local_default()]\""), "\"11 10\"");
	println!("ok: a local const read alone follows the local consts and defaults it reads");

	assert_eq!(eval(&mut client, "dep_colon()"), "\"6 5\"");
	let changes = source.edit(
		"dependents.dm",
		"/datum/dep_colon_later\n",
		"/datum/dep_colon_later\n\tvar/const/DEP_COLON = 9\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "dep_colon()"), "\"10 9\"");
	println!("ok: an untyped x:name in an unchanged file finds a later type's new declaration");

	assert_eq!(eval(&mut client, "\"[dep_scoped_limit()] [dep_typed_limit()]\""), "\"6 6\"");
	let changes = source.edit("dependents.dm", "DEP_SCOPED_BASE = 5", "DEP_SCOPED_BASE = 10");
	reload(&mut watcher, &changes);
	let changes = source.edit("dependents.dm", "DEP_SCOPED = 5", "DEP_SCOPED = 10");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "\"[dep_scoped_limit()] [dep_typed_limit()]\""), "\"11 11\"");
	println!("ok: a local const in an unchanged file reads the names in scope where it is declared");

	const PARENT: &str = "\"[(new /datum/dep_parent/child).f()] [dep_self()]\"";
	assert_eq!(eval(&mut client, PARENT), "\"5 5\"");
	let changes = source.edit("dependents.dm", "DEP_PARENT = 5", "DEP_PARENT = 10");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, PARENT), "\"10 10\"");
	println!("ok: code dmasm can't compile that passes a changed const alone to ..() or .() sees its new value");

	let changes = source.edit("dependents.dm", "/datum/dep_colon_later\n\tvar/const/DEP_COLON = 9\n", "");
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "dep_colon()"), "\"6 5\"");
	println!("ok: an untyped x:name in an unchanged file finds an earlier type's declaration when a later type is removed");

	assert_eq!(eval(&mut client, "(new /datum/dep_colon_inits).limits[1]"), "6");
	assert_eq!(eval(&mut client, "dep_colon_local_const()"), "6");
	let changes = source.edit(
		"dependents.dm",
		"/datum/dep_colon_list_later\n",
		"/datum/dep_colon_list_later\n\tvar/const/DEP_COLON_LIST = 9\n"
	);
	let chooser = changes
		.plan
		.reloads
		.iter()
		.find(|r| r.proc.path == "/mob/dep_colon_chooser/choose_dep_colon_list");
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
			.any(|pair| matches!(pair, [dmasm::Instruction::PushInt(10), dmasm::Instruction::NewList(1)])),
		"the verb's new `in` list isn't list(10): {}",
		dmasm::format(&choices.nodes)
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "(new /datum/dep_colon_inits).limits[1]"), "10");
	println!("ok: initializers and `in` lists in an unchanged file follow the declaration an untyped x:name finds");
	assert_eq!(eval(&mut client, "dep_colon_local_const()"), "10");
	println!("ok: a local const in an unchanged file follows the declaration an untyped x:name finds");

	assert_eq!(eval(&mut client, "dep_colon_alone()"), "5");
	let refused = refused_edit(
		&mut source,
		"dependents.dm",
		"/datum/dep_colon_alone_later\n",
		"/datum/dep_colon_alone_later\n\tvar/const/DEP_COLON_ALONE = 9\n"
	);
	let shown: Vec<String> = refused.iter().map(|r| format!("{:#}", r)).collect();
	assert_eq!(
		shown,
		[
			"/dep_colon_alone: reads a var by `x:name` that another type declares now, so it must be compiled again, but it doesn't compile: \
			 unsupported statement: type::proc() that names a type's second or later override"
		],
		"a proc dmasm can't compile that reads an untyped x:name alone from another type after the reload isn't refused"
	);
	println!("ok: a reload is refused for a proc dmasm can't compile that reads an untyped x:name alone from another type");

	assert_eq!(eval(&mut client, "dep_retyped_local_const()"), "6");
	let changes = source.edit(
		"dependents.dm",
		"var/datum/dep_retype_first/dep_retype_target",
		"var/datum/dep_retype_later/dep_retype_target"
	);
	reload(&mut watcher, &changes);
	assert_eq!(eval(&mut client, "dep_retyped_local_const()"), "10");
	println!("ok: a local const in an unchanged file follows a var it reads through that is declared with another type");

	assert_eq!(eval(&mut client, "dep_colon_indexed()"), "5");
	let refused = refused_edit(
		&mut source,
		"dependents.dm",
		"/datum/dep_colon_indexed_later\n",
		"/datum/dep_colon_indexed_later\n\tvar/const/DEP_COLON_INDEXED = 9\n"
	);
	let shown: Vec<String> = refused.iter().map(|r| format!("{:#}", r)).collect();
	assert_eq!(
		shown,
		[
			"/dep_colon_indexed: reads a var by `x:name` that another type declares now, so it must be compiled again, but it doesn't compile: \
			 unsupported statement: type::proc() that names a type's second or later override"
		],
		"a proc dmasm can't compile that reads items[1]:name from another type after the reload isn't refused"
	);
	println!("ok: a reload is refused for a proc dmasm can't compile that reads items[1]:name from another type");

	assert_eq!(eval(&mut client, "dep_retyped_alone()"), "5");
	let refused = refused_edit(
		&mut source,
		"dependents.dm",
		"var/datum/dep_retype_alone_first/dep_retype_alone_target",
		"var/datum/dep_retype_alone_later/dep_retype_alone_target"
	);
	let shown: Vec<String> = refused.iter().map(|r| format!("{:#}", r)).collect();
	assert_eq!(
		shown,
		[
			"/dep_retyped_alone: reads a var through a var that is declared with another type now, so it must be compiled again, but it doesn't \
			 compile: unsupported statement: type::proc() that names a type's second or later override"
		],
		"a proc dmasm can't compile that reads a const through a retyped var isn't refused"
	);
	println!("ok: a reload is refused for a proc dmasm can't compile that reads a const through a var declared with another type");

	let read = "\"[dep_safe_index(new /datum/dep_safe_index_holder)] [isnull(dep_safe_index(null))]\"";
	assert_eq!(eval(&mut client, read), "\"9 1\"");
	let changes = source.edit(
		"dependents.dm",
		"\treturn holder?.items[1].DEP_SAFE_INDEX",
		"\treturn 0 || holder?.items[1].DEP_SAFE_INDEX"
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, read),
		"\"9 1\"",
		"recompiled holder?.items[1].name doesn't read the var DreamMaker reads"
	);
	println!("ok: recompiled holder?.items[1].name reads the var DreamMaker reads, and null for a null holder");

	assert_eq!(eval(&mut client, "dep_call_colon()"), "5");
	let refused = refused_edit(
		&mut source,
		"dependents.dm",
		"/datum/dep_call_colon_later\n",
		"/datum/dep_call_colon_later\n\tvar/const/DEP_CALL_COLON = 9\n"
	);
	let shown: Vec<String> = refused.iter().map(|r| format!("{:#}", r)).collect();
	assert_eq!(
		shown,
		[
			"/dep_call_colon: reads a var by `x:name` that another type declares now, so it must be compiled again, but it doesn't compile: \
			 unsupported statement: type::proc() that names a type's second or later override"
		],
		"a proc dmasm can't compile that reads make():name from another type after the reload isn't refused"
	);
	println!("ok: a reload is refused for a proc dmasm can't compile that reads make():name from another type");

	assert_eq!(eval(&mut client, "dep_returned()"), "9");
	let changes = source.edit(
		"dependents.dm",
		"/proc/dep_returned_make()\n",
		"/proc/dep_returned_make() as /datum/dep_returned_first\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "dep_returned()"),
		"5",
		"an unchanged proc that reads make().name doesn't follow make()'s new return type"
	);
	println!("ok: an unchanged proc that reads make().name follows make()'s new return type");

	assert_eq!(eval(&mut client, "dep_bare_call()"), "5");
	let changes = source.edit(
		"dependents.dm",
		"/datum/dep_bare_call_later\n",
		"/datum/dep_bare_call_later\n\tvar/const/DEP_BARE_CALL = 9\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "dep_bare_call()"),
		"9",
		"an unchanged proc that reads make().name doesn't read the var that untyped lookup finds now"
	);
	println!("ok: an unchanged proc that reads make().name reads the var that untyped lookup finds now");

	assert_eq!(eval(&mut client, "dep_safe_write(3)"), "\"1 3\"");
	let changes = source.edit(
		"dependents.dm",
		"factory.make()?.DEP_SAFE_WRITE = value\n",
		"factory.make()?.DEP_SAFE_WRITE = value + 1\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "dep_safe_write(5)"),
		"\"1 6\"",
		"recompiled make()?.name = value doesn't write the static that DreamMaker writes"
	);
	println!("ok: recompiled make()?.name = value writes the static that DreamMaker writes");

	let read = "\"[dep_safe_colon(new /datum/dep_safe_colon_base/sub)] [isnull(dep_safe_colon(null))]\"";
	assert_eq!(eval(&mut client, read), "\"5 1\"");
	let changes = source.edit(
		"dependents.dm",
		"\treturn base?:make().DEP_SAFE_COLON",
		"\treturn 0 || base?:make().DEP_SAFE_COLON"
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, read),
		"\"5 1\"",
		"recompiled base?:make().name doesn't read the var DreamMaker reads"
	);
	println!("ok: recompiled base?:make().name reads the var DreamMaker reads, and null for a null base");

	const MADE_AFTER_CALL: &str = "\"/datum/dep_new_child /datum/dep_new_child/special\"";
	assert_eq!(eval(&mut client, "dep_implicit_new()"), MADE_AFTER_CALL);
	let changes = source.edit(
		"dependents.dm",
		"\ttyped.me()?.dep_new_made = new\n",
		"\ttyped.me()?.dep_new_made = new()\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "dep_implicit_new()"),
		MADE_AFTER_CALL,
		"recompiled make()?.name = new doesn't make the type DreamMaker makes"
	);
	println!("ok: recompiled make()?.name = new makes the type DreamMaker makes");

	assert_eq!(eval(&mut client, "\"[dep_implicit_uncompiled()]\""), "\"/datum/dep_implicit_child\"");
	let refused = refused_edit(
		&mut source,
		"dependents.dm",
		"/proc/dep_implicit_make(holder) as /datum/dep_implicit_holder\n",
		"/proc/dep_implicit_make(holder) as /datum/dep_implicit_other\n"
	);
	let shown: Vec<String> = refused.iter().map(|r| format!("{:#}", r)).collect();
	assert_eq!(
		shown,
		[
			"/dep_implicit_uncompiled: takes the type of a `new`, `locate()`, `astype()` or `istype()` from a var whose type changes, so it must be \
			 compiled again, but it doesn't compile: unsupported statement: type::proc() that names a type's second or later override"
		],
		"a proc dmasm can't compile whose make()?.name = new makes another type after the reload isn't refused"
	);
	println!("ok: a reload is refused for a proc dmasm can't compile whose make()?.name = new makes another type");

	const GLOBAL_CALLED: &str = "\"/datum/dep_global_b 5\"";
	assert_eq!(eval(&mut client, "(new /datum/dep_global_caller).call_global()"), GLOBAL_CALLED);
	let changes = source.edit(
		"dependents.dm",
		"\treturn \"[::dep_global_make().type] ",
		"\treturn \"[::dep_global_make().type]  "
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "(new /datum/dep_global_caller).call_global()"),
		"\"/datum/dep_global_b  5\"",
		"recompiled ::make() in a type with its own make() doesn't call the global proc and read by its return type"
	);
	println!("ok: recompiled ::make() in a type with its own make() calls the global proc and reads by its return type");

	let read = "\"[dep_chain(new /datum/dep_chain_factory)] [isnull(dep_chain(null))]\"";
	assert_eq!(eval(&mut client, read), "\"5 1\"");
	let changes = source.edit(
		"dependents.dm",
		"\treturn factory?.builder().dep_chain_make().DEP_CHAIN_LIMIT",
		"\treturn 0 || factory?.builder().dep_chain_make().DEP_CHAIN_LIMIT"
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, read),
		"\"5 1\"",
		"recompiled factory?.builder().make().name doesn't read the var DreamMaker reads"
	);
	println!("ok: recompiled factory?.builder().make().name reads the var DreamMaker reads, and null for a null factory");

	assert_eq!(eval(&mut client, "dep_shared_bump()"), "\"0 1\"");
	let changes = source.edit(
		"dependents.dm",
		"\tdep_shared_make()?.DEP_SHARED.count += 1\n",
		"\tdep_shared_make()?.DEP_SHARED.count += 2\n"
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "dep_shared_bump()"),
		"\"0 3\"",
		"recompiled make()?.STATIC.name += value doesn't change the object that DreamMaker changes"
	);
	println!("ok: recompiled make()?.STATIC.name += value changes the object that DreamMaker changes");

	let read = "\"[dep_show_on(new /mob/dep_shower)] [isnull(dep_show_on(null))]\"";
	assert_eq!(eval(&mut client, read), "\"shown 1\"");
	let changes = source.edit("dependents.dm", "\treturn target?:dep_show()", "\treturn 0 || target?:dep_show()");
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, read),
		"\"shown 1\"",
		"recompiled untyped target?:verb() doesn't call the verb by its `set name`"
	);
	println!("ok: recompiled untyped target?:verb() calls the verb by its `set name`");

	const MAKE_ON: &str = "dep_make_on(new /datum/dep_maker_last)";
	assert_eq!(eval(&mut client, MAKE_ON), "\"last\"");
	let changes = source.edit("dependents.dm", "\treturn x:dep_make()", "\treturn 0 || x:dep_make()");
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, MAKE_ON),
		"\"last\"",
		"recompiled untyped x:make() doesn't call the proc of the last type made that declares make()"
	);
	println!("ok: recompiled untyped x:make() calls the proc of the last type made that declares make()");

	let changes = source.edit(
		"dependents.dm",
		"/proc/dep_make_on(x)",
		"/datum/dep_maker_later/proc/dep_make()\n\tset name = \"dep make later\"\n\treturn \"later\"\n\n/proc/dep_make_on(x)"
	);
	reload(&mut watcher, &changes);
	assert_eq!(
		eval(&mut client, "dep_make_on(new /datum/dep_maker_later)"),
		"\"later\"",
		"unchanged untyped x:make() doesn't call the make() of a type made later"
	);
	println!("ok: unchanged untyped x:make() calls the make() of a type made later");
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
