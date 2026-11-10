//! Scopes and variables: how each kind of value shows, and what's inside it

use serde_json::{json, Value};

use crate::{
	client::{named, value_of},
	ok,
	world::World
};

/// The world's BYOND version
fn byond_version(world: &mut World) -> u64 {
	world.eval_global("world.byond_version").parse().unwrap()
}

pub fn run() {
	let mut world = World::start();
	let version = byond_version(&mut world);
	world.run_to("values", "/values", "values_stop");

	let frame = world.frame_id();
	let scopes = world.request("scopes", json!({ "frameId": frame }));
	let names: Vec<&str> = scopes["scopes"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap()).collect();
	assert_eq!(names, ["Locals", "Arguments", "Globals"]);
	ok("a frame's scopes: its locals, its arguments, the globals");

	let locals_reference = world.scope(&frame, "Locals");
	let variables = world.variables(&locals_reference);
	let locals = named(&variables);
	let local = |name: &str| value_of(&locals, name).to_owned();
	let variable = |name: &str| -> Value { variables.iter().find(|v| v["name"] == name).unwrap().clone() };
	let opens = |name: &str| variable(name)["variablesReference"] != 0;

	assert_eq!(locals[0], (".".to_owned(), "\"dot\"".to_owned()));
	assert_eq!(locals[1].0, "whole");
	ok("the locals start with the proc's return value, then come as they're declared");

	assert_eq!(local("whole"), "42");
	assert_eq!(local("fraction"), "1.5");
	assert_eq!(local("negative"), "-0.25");
	// BYOND's numbers are 32-bit floats
	assert_eq!(local("past_24_bits"), "16777216");
	assert_eq!(local("huge"), "1000000000000000000000000000000");
	assert_eq!(local("infinite"), "inf");
	ok("numbers: whole, fractional, negative, past 24 bits, huge and infinite");

	assert_eq!(local("text"), r#""hello \"quoted\"\nline two""#);
	assert_eq!(local("empty_text"), r#""""#);
	ok("text is quoted, with its quotes and newlines escaped");

	assert_eq!(local("nothing"), "null");
	assert_eq!(local("deleted"), "null");
	assert!(!opens("nothing") && !opens("whole") && !opens("text"));
	ok("null, and a var whose object was deleted; plain values don't open");

	assert_eq!(local("path"), "/datum/thing");
	assert_eq!(local("proc_path"), "/proc/leaf");
	assert_eq!(local("file"), "values.dm");
	assert!(!opens("path") && !opens("proc_path") && !opens("file"));
	ok("a typepath, a proc's path and a file, none of which opens");

	// A datum shows as its name, if it has one
	assert_eq!(local("named"), "A");
	assert_eq!(local("unnamed"), "/datum/made");
	let inside = world.children(&variable("named"));
	assert_eq!(inside[0], ("type".to_owned(), "/datum/thing".to_owned()));
	let children = world.variables(&variable("named")["variablesReference"].clone());
	assert_eq!(children[0]["variablesReference"], 0, "an object's type opens");
	let names: Vec<String> = inside[1..].iter().map(|(name, _)| name.to_lowercase()).collect();
	let mut sorted = names.clone();
	sorted.sort();
	assert_eq!(names, sorted, "an object's vars aren't sorted");
	assert_eq!(value_of(&inside, "name"), "\"A\"");
	assert_eq!(value_of(&inside, "n"), "1");
	assert_eq!(value_of(&inside, "other"), "B");
	assert_eq!(value_of(&inside, "items"), "/list {len = 3}");
	assert_eq!(value_of(&inside, "K"), "4");
	ok("a datum opens to its vars: its type first, the rest by name");

	// Down through an object's var to another object
	let children = world.variables(&variable("named")["variablesReference"].clone());
	let other = children.iter().find(|v| v["name"] == "other").unwrap();
	let inside = world.children(other);
	assert_eq!(inside[0], ("type".to_owned(), "/datum/thing/sub".to_owned()));
	assert_eq!(value_of(&inside, "name"), "\"B\"");
	ok("an object inside an object opens too");

	for (name, text, type_path) in [("T", "turf", "/turf"), ("O", "obj", "/obj"), ("M", "player", "/mob/player"), ("A", "area", "/area")] {
		assert_eq!(local(name), text);
		let inside = world.children(&variable(name));
		assert_eq!(inside[0], ("type".to_owned(), type_path.to_owned()));
	}
	let mob = world.children(&variable("M"));
	assert_eq!(value_of(&mob, "hp"), "10");
	assert_eq!(value_of(&mob, "loc"), "turf");
	ok("a turf, an obj, a mob and an area, with their built-in vars and their own");

	let inside = world.children(&variable("the_world"));
	assert_eq!(value_of(&inside, "maxx"), "3");
	assert_eq!(value_of(&inside, "byond_version"), version.to_string());
	ok("the world opens to its vars");

	assert_eq!(local("empty"), "/list {len = 0}");
	assert_eq!(world.children(&variable("empty")).len(), 0);
	assert_eq!(local("plain"), "/list {len = 4}");
	let plain: Vec<(&str, &str)> = vec![("[1]", "1"), ("[2]", "\"b\""), ("[3]", "null"), ("[4]", "A")];
	let inside = world.children(&variable("plain"));
	assert_eq!(inside.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect::<Vec<_>>(), plain);
	ok("a list opens to its elements, numbered from 1");

	let children = world.variables(&variable("assoc")["variablesReference"].clone());
	assert_eq!(named(&children), [
		("[1]".to_owned(), "\"a\" = 1".to_owned()),
		("[2]".to_owned(), "\"b\" = A".to_owned()),
		// A key with no value is no different from an element
		("[3]".to_owned(), "\"c\"".to_owned())
	]);
	assert_eq!(world.children(&children[1]), [
		("key".to_owned(), "\"b\"".to_owned()),
		("value".to_owned(), "A".to_owned())
	]);
	ok("an associative list shows key = value, and each entry opens to its key and its value");

	let children = world.variables(&variable("nested")["variablesReference"].clone());
	assert_eq!(world.children(&children[0]), [("[1]".to_owned(), "1".to_owned()), ("[2]".to_owned(), "2".to_owned())]);
	let inner = world.variables(&children[1]["variablesReference"].clone());
	assert_eq!(named(&inner), [("[1]".to_owned(), "\"x\" = /list {len = 1}".to_owned())]);
	ok("lists inside lists open");

	let inside = world.children(&variable("dupes"));
	assert_eq!(inside.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>(), ["\"a\"", "\"a\"", "1", "1"]);
	let inside = world.children(&variable("big"));
	assert_eq!(inside.len(), 3000);
	assert_eq!(inside[2999], ("[3000]".to_owned(), "null".to_owned()));
	ok("a list with the same element twice, and one of 3,000");

	let inside = world.children(&variable("var_list"));
	assert!(inside.iter().any(|(_, value)| value == "\"n\" = 1"), "{:?}", inside);
	assert!(inside.iter().any(|(_, value)| value == "\"name\" = \"A\""), "{:?}", inside);
	let inside = world.children(&variable("contents"));
	assert_eq!(inside.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>(), ["obj", "player"]);
	assert_eq!(local("overlays"), "/list {len = 0}");
	let inside = world.children(&variable("verbs"));
	assert_eq!(inside, [("[1]".to_owned(), "/mob/player/verb/shout".to_owned())]);
	assert_eq!(local("world_contents"), "/list {len = 12}");
	ok("BYOND's own lists: an object's vars, a turf's contents, a mob's overlays and verbs, the world's contents");

	let inside = world.children(&variable("mx"));
	assert_eq!(local("mx"), "/matrix");
	assert_eq!(
		["a", "b", "c", "d", "e", "f"].map(|name| value_of(&inside, name).to_owned()),
		["1", "0", "0", "0", "1", "0"]
	);
	assert_eq!(local("re"), "a+");
	assert_eq!(value_of(&world.children(&variable("re")), "name"), "\"a+\"");
	assert_eq!(local("error"), "boom");
	let inside = world.children(&variable("error"));
	assert_eq!((value_of(&inside, "file"), value_of(&inside, "line")), ("\"file.dm\"", "3"));
	assert_eq!(local("save"), "dap_test.sav");
	assert_eq!(value_of(&world.children(&variable("img")), "type"), "/image");
	ok("a matrix, a regex, an exception, a savefile and an image");

	assert_eq!(local("pointer"), "&whole");
	ok("a pointer shows what it points at");

	if version >= 516 {
		assert_eq!(local("pairs"), "/alist {len = 2}");
		let mut inside: Vec<String> = world.children(&variable("pairs")).into_iter().map(|(_, value)| value).collect();
		inside.sort();
		assert_eq!(inside, ["\"a\" = 1", "7 = \"seven\""]);
		assert_eq!(local("vec"), "vector(1,2,3)");
		let inside = world.children(&variable("vec"));
		assert_eq!(inside[..4], [("x", "1"), ("y", "2"), ("z", "3"), ("len", "3")].map(|(n, v)| (n.to_owned(), v.to_owned())));
		assert_eq!(local("pix"), "pixloc(1,1,1)");
		assert_eq!(value_of(&world.children(&variable("pix")), "loc"), "turf");
		let inside = world.children(&variable("this_proc"));
		assert_eq!(value_of(&inside, "proc"), "/proc/values");
		assert_eq!(value_of(&inside, "file"), "\"values.dm\"");
		assert_eq!(local("filters"), "/list {len = 0}");
		ok("516's values: an alist, a vector, a pixloc, a callee, and filters as a list");
	} else {
		println!("(BYOND {} has no alists, vectors, pixlocs or callees to show)", version);
	}

	assert_eq!(local("twin"), "1");
	assert_eq!(local("twin #1"), "2");
	ok("two locals of one name are told apart");

	assert_eq!(world.scope_values("Arguments"), [
		("src".to_owned(), "null".to_owned()),
		("usr".to_owned(), "null".to_owned()),
		("first".to_owned(), "1".to_owned()),
		("second".to_owned(), "\"two\"".to_owned()),
		// values() was called with one more than it declares
		("undefined argument #1".to_owned(), "3".to_owned())
	]);
	ok("the arguments: src, usr, the proc's own, and one more than it declares");

	let globals = world.scope_values("Globals");
	assert_eq!(value_of(&globals, "global_number"), "5");
	assert_eq!(value_of(&globals, "global_list"), "/list {len = 3}");
	assert_eq!(value_of(&globals, "early_global"), "5");
	// A type's static and const vars are globals
	assert_eq!(value_of(&globals, "shared"), "9");
	assert_eq!(value_of(&globals, "K"), "4");
	assert_eq!(value_of(&globals, "NORTH"), "1");
	ok("the globals: the world's own, types' static vars, BYOND's constants");

	for reference in [-5, 0, 99_999] {
		let vars = world.request("variables", json!({ "variablesReference": reference }));
		assert_eq!(vars["variables"].as_array().unwrap().len(), 0);
	}
	assert_eq!(world.local("whole"), "42");
	ok("a reference that was never given, 0 among them, answers nothing");

	// whole += length(args)
	world.step("next");
	assert_eq!(world.local("whole"), "45");
	assert_eq!(world.eval("whole = 7"), "7");
	assert_eq!(world.local("whole"), "7");
	ok("a local's value after a step, and after an expression assigned to it");
	world.resume_through("values");

	world.run_to("inside", "/datum/thing/inside", "inside_stop");
	let arguments = world.scope_values("Arguments");
	assert_eq!((value_of(&arguments, "src"), value_of(&arguments, "by")), ("S", "4"));
	let src = world.variable("Arguments", "src");
	let inside = world.children(&src);
	assert_eq!(inside[0], ("type".to_owned(), "/datum/thing/sub".to_owned()));
	assert_eq!(value_of(&inside, "items"), "/list {len = 2}");
	ok("src, in a datum's proc, opens to the datum");

	let list = world.variable("Locals", "local_list");
	assert_eq!(world.children(&list).iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>(), ["10", "20", "30"]);
	world.eval("local_list += 40");
	let list = world.variable("Locals", "local_list");
	assert_eq!(list["value"], "/list {len = 4}");
	assert_eq!(world.children(&list)[3], ("[4]".to_owned(), "40".to_owned()));
	world.resume_through("inside");
	ok("a list that an expression added to shows its new element");
	world.end();
}
