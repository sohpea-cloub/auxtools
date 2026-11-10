//! Evaluating expressions: in the debug console, as watches, in any frame

use serde_json::{json, Value};

use crate::{ok, world::World};

/// What the launcher offers to go on with, typed as far as `text` in the
/// paused frame (or in no frame): each name with its kind
fn offered(world: &mut World, text: &str, frame: Option<&serde_json::Value>) -> Vec<(String, String)> {
	let targets = world.request("completions", json!({ "frameId": frame, "text": text, "column": text.encode_utf16().count() + 1 }));
	targets["targets"]
		.as_array()
		.unwrap()
		.iter()
		.map(|t| (t["label"].as_str().unwrap().to_owned(), t["type"].as_str().unwrap().to_owned()))
		.collect()
}

/// Completions in the debug console, in the frame of a /datum/thing/sub's
/// inside(by), with a local local_list
fn completions(world: &mut World) {
	let frame = world.frame_id();
	let names = |world: &mut World, text: &str| -> Vec<String> { offered(world, text, Some(&frame)).into_iter().map(|(name, _)| name).collect() };
	let pair = |name: &str, kind: &str| (name.to_owned(), kind.to_owned());

	assert_eq!(offered(world, "by", Some(&frame)), [pair("by", "variable")]);
	assert_eq!(names(world, "loc"), ["local_list"]);
	// The frame's own, src's vars and procs, globals and global procs
	let all = offered(world, "", Some(&frame));
	for expected in [
		pair("by", "variable"),
		pair("local_list", "variable"),
		pair("src", "variable"),
		pair("usr", "variable"),
		pair("args", "variable"),
		pair("n", "variable"),
		pair("items", "variable"),
		pair("bump", "function"),
		pair("inside", "function"),
		pair("global_number", "variable"),
		pair("leaf", "function"),
		pair("middle", "function"),
		pair("world", "variable")
	] {
		assert!(all.contains(&expected), "{:?} isn't offered", expected);
	}
	assert!(!all.iter().any(|(name, _)| name.contains("auxtools") || name.contains('(')), "{:?}", all);
	ok("a name being typed is completed with what the frame can name: its own, src's vars and procs, globals and global procs");

	// After a dot: what that is, in the frame
	let members = offered(world, "src.", Some(&frame));
	for expected in [
		pair("n", "property"),
		pair("name", "property"),
		pair("other", "property"),
		pair("shared", "property"),
		pair("type", "property"),
		pair("bump", "method"),
		pair("inside", "method")
	] {
		assert!(members.contains(&expected), "{:?} isn't offered", expected);
	}
	assert!(!members.iter().any(|(name, _)| name == "by" || name == "leaf" || name == "hp"), "{:?}", members);
	assert_eq!(names(world, "src.o"), ["other"]);
	assert_eq!(names(world, "x + src.BU"), ["bump"]);
	assert_eq!(names(world, "world.max"), ["maxx", "maxy", "maxz"]);
	ok("after a dot, the vars and procs of what's before it, whatever the case");

	let mob = names(world, "new /mob/player(null).");
	assert_eq!(mob, Vec::<String>::new(), "a call isn't made to find out what it gives");
	assert_eq!(names(world, "src.other."), Vec::<String>::new());
	assert_eq!(names(world, "nowhere_at_all."), Vec::<String>::new());
	assert_eq!(names(world, "\"some te"), Vec::<String>::new());
	assert_eq!(names(world, "local_list[1]."), Vec::<String>::new());
	ok("nothing is offered for what would have to be called, isn't there, or has no vars");

	// In the middle of a line, and with letters that take two units
	let targets = world.request("completions", json!({ "frameId": frame, "text": "leaf(by) + n", "column": 8 }));
	assert_eq!(targets["targets"][0]["label"], "by");
	assert_eq!((&targets["targets"][0]["start"], &targets["targets"][0]["length"]), (&json!(6), &json!(2)));
	let targets = world.request("completions", json!({ "frameId": frame, "text": "\"\u{1F600}\" + src.na", "column": 14 }));
	assert_eq!(targets["targets"][0]["label"], "name");
	// The face is two units: quote, face, quote, " + ", "src." are 11
	assert_eq!((&targets["targets"][0]["start"], &targets["targets"][0]["length"]), (&json!(12), &json!(2)));
	ok("a completion says which text it takes the place of, wherever the cursor is");

	assert_eq!(offered(world, "#d", Some(&frame)), [pair("dis", "keyword"), pair("disassemble", "keyword")]);
	assert_eq!(offered(world, "#", Some(&frame)).len(), 5);
	ok("the debug server's commands are completed after a #");

	// No frame: the world's, and nothing of a frame's
	let global = offered(world, "glo", None);
	assert_eq!(global, [pair("global", "variable"), pair("global_list", "variable"), pair("global_number", "variable")]);
	assert_eq!(offered(world, "by", None), Vec::<(String, String)>::new());
	ok("with no frame, globals and global procs are completed");
}

/// Proc bodies in the paused frame of `inside`, and in none
fn bodies(world: &mut World) {
	let mut gives = |world: &mut World, cases: &[(&str, &str)]| {
		for (body, expected) in cases {
			assert_eq!(world.eval(body), *expected, "{}", body);
		}
	};
	gives(world, &[
		("var/n = 0; for(var/i in 1 to 3) n += i; n", "6"),
		("var/total = 0\nfor(var/x in local_list)\n\ttotal += x\ntotal", "61"),
		("return by * 2", "8"),
		("if(by > 3)\n\treturn \"big\"\nreturn \"small\"", "\"big\""),
		("var/list/L = list()\nfor(var/i in 1 to by)\n\tL += i\nL.Join(\",\")", "\"1,2,3,4\""),
		("var/i = 0\nwhile(i < 10)\n\ti++\n\tif(i == 5) break\ni", "5"),
		// Indented as a whole, as code copied from a proc is
		("\t\tvar/x = by\n\t\tx * 3", "12")
	]);
	ok("a body: vars, loops, return, and a last line that's an expression is the result");

	gives(world, &[
		// src's var, the frame's argument, a global
		("var/x = n + by + global_number\nx", "10"),
		// a global proc, and src's
		("var/r = leaf(1)\nr", "4"),
		("var/got = bump(1)\nn = 1\ngot", "4"),
		// the frame's own vars come before src's
		("var/list/got = local_list\ngot.len", "3")
	]);
	ok("in a body a name is its own, then the frame's, src's var and a global; a call is src's proc or a global one");

	// (local_list is list(11, 20, 30) by now)
	// Changes to the frame's vars are written back; the body's own aren't
	// anywhere afterwards. Falling off the end gives `.`, the frame's.
	gives(world, &[("by = 9\nlocal_list[1] = 12\nvar/unused = 0", "null")]);
	assert_eq!(world.argument("by"), "9");
	assert_eq!(world.eval("local_list[1]"), "12");
	gives(world, &[("by = 4; local_list[1] = 11; by", "4"), ("var/q = 3", "null")]);
	assert_eq!(world.argument("by"), "4");
	let frame = world.frame_id();
	let why = world.refused("evaluate", json!({ "expression": "q", "frameId": frame, "context": "repl" }));
	assert_eq!(why, "undefined var: q");
	ok("a body's changes to the frame's vars are written back, and its own vars go with it");

	let frame = world.frame_id();
	let mut fails = |world: &mut World, cases: &[(&str, &str)]| {
		for (body, expected) in cases {
			for context in ["repl", "watch"] {
				let why = world.refused("evaluate", json!({ "expression": body, "frameId": frame, "context": context }));
				assert!(why.starts_with(expected), "{} in a {} said {:?}", body, context, why);
			}
		}
	};
	fails(world, &[
		("var/x = 1\nx = nowhere", "line 2: undefined var: nowhere"),
		("var/x = 1\nx +", "parser error: 2:"),
		("var/x = 0\nreturn by / x", "runtime error: Division by zero"),
		("var/x = 1\nreturn src.other.name", "runtime error: Cannot read null.name"),
		// A body is over when it's answered too
		("var/x = by\nsleep(1)\nreturn x", "the expression didn't finish: something it called went to sleep")
	]);
	assert_eq!(world.eval("by"), "4");
	assert_eq!(world.frames(0).len(), 3);
	ok("a body's compile errors say the line, and its runtime errors and sleeping fail as an expression's do");

	// The same in a watch; a result that opens doesn't say how to reach
	// what's in it, which would run the body again
	let result = world.evaluate_in(&frame, "var/datum/thing/t = src\nt", "watch");
	assert_eq!(result["result"], "S");
	let inside = world.variables(&result["variablesReference"].clone());
	let n = inside.iter().find(|v| v["name"] == "n").unwrap();
	assert_eq!((&n["value"], &n["evaluateName"]), (&json!("1"), &Value::Null), "{}", n);
	// A hover reads one expression, and runs no body: this one would never end
	for body in ["again:\ngoto again", "var/x = n\nx", "return n"] {
		let why = world.refused("evaluate", json!({ "expression": body, "frameId": frame, "context": "hover" }));
		assert_eq!(why, "a hover evaluates one expression, not a proc's body", "{}", body);
	}
	ok("a body in a watch, whose result opens; a hover refuses bodies");

	assert_eq!(world.eval_global("var/t = 0\nfor(var/i in 1 to 4) t += i\nt"), "10");
	assert_eq!(world.eval_global("global_number = 7\nvar/was = global_number\nglobal_number = 5\nwas"), "7");
	let why = world.refused("evaluate", json!({ "expression": "var/x = by\nx", "context": "repl" }));
	assert_eq!(why, "line 1: undefined var: by");
	ok("a body in no frame has the globals and nothing of the frames");
}

/// `*(&x)`, a type's var by `::`, and `initial()` of one
fn smaller_things(world: &mut World) {
	let mut gives = |world: &mut World, cases: &[(&str, &str)]| {
		for (expression, expected) in cases {
			assert_eq!(world.eval(expression), *expected, "{}", expression);
		}
	};
	gives(world, &[("*(&by)", "4"), ("*(&by) = 6", "6"), ("by", "6"), ("var/p = &by\n*p = 4\n*(&by)", "4")]);
	assert_eq!(world.argument("by"), "4");
	ok("*(&x) reads and writes what the pointer it makes points at");

	// As DreamMaker has them: a type's initial value, a static var's value
	gives(world, &[
		("/datum/thing::name", "\"thing\""),
		("/datum/thing::n + 1", "2"),
		("/datum/thing/sub::name", "\"thing\""),
		("initial(/datum/thing::n)", "1"),
		("src::name", "\"thing\""),
		("initial(src::n)", "1"),
		("/datum/thing::shared", "9"),
		("initial(/datum/thing::shared)", "9"),
		("var/x = /datum/thing::n\nreturn x + initial(src::n)", "2")
	]);
	ok("a type's var by ::, and initial() of one, in an expression and a body");
}

pub fn run() {
	let mut world = World::start();
	world.run_to("inside", "/datum/thing/inside", "inside_stop");
	// Each expression and what it gives, in the stopped proc: src is a
	// /datum/thing/sub called "S" with n = 1, by is 4, local_list is
	// list(10, 20, 30)
	let mut gives = |world: &mut World, cases: &[(&str, &str)]| {
		for (expression, expected) in cases {
			assert_eq!(world.eval(expression), *expected, "{}", expression);
		}
	};

	gives(&mut world, &[
		("1 + 2 * 3", "7"),
		("7 % 4", "3"),
		("2 ** 10", "1024"),
		("\"a\" + \"b\"", "\"ab\""),
		("1 && 0", "0"),
		("!0", "1"),
		("by > 3 ? \"big\" : \"small\"", "\"big\""),
		("5 & 3", "1")
	]);
	ok("arithmetic, text, logic, a conditional and bit operations");

	gives(&mut world, &[("by", "4"), ("by * 2", "8"), ("local_list[2]", "20"), ("length(local_list)", "3"), ("src", "S"), ("usr", "null")]);
	ok("the frame's arguments and locals, src and usr");

	gives(&mut world, &[
		("src.n", "1"),
		("src.name", "\"S\""),
		("src.items[1]", "\"x\""),
		("src.other", "null"),
		("src.type", "/datum/thing/sub"),
		("src.vars[\"n\"]", "1"),
		("initial(src.n)", "1")
	]);
	ok("an object's vars, through a list, by vars[] and by initial()");

	gives(&mut world, &[
		("global_number", "5"),
		("global.global_number", "5"),
		("global_list[2]", "\"two\""),
		("src.shared", "9"),
		("src.K", "4"),
		("world.maxx", "3")
	]);
	ok("globals, a type's static and const vars, the world's vars");

	gives(&mut world, &[
		("leaf(4)", "10"),
		("middle(1)", "14"),
		("recurse(3)", "5"),
		("text2num(\"12\") + 1", "13"),
		("istype(src, /datum/thing)", "1"),
		("locate(1, 1, 1)", "turf"),
		("\"[src.name]-[by]\"", "\"S-4\"")
	]);
	ok("the world's procs, BYOND's own, and text with expressions in it");

	// A name the frame doesn't have is src's var, as it is in the proc's code
	gives(&mut world, &[
		("n", "1"),
		("name", "\"S\""),
		("items[1]", "\"x\""),
		("length(items)", "2"),
		("other", "null"),
		("type", "/datum/thing/sub"),
		("vars[\"n\"]", "1"),
		("n + by", "5"),
		("\"[name] has [n]\"", "\"S has 1\"")
	]);
	gives(&mut world, &[("n = 50", "50"), ("src.n", "50"), ("n++", "50"), ("n", "51"), ("n -= 50", "1")]);
	ok("a var of src by its bare name, read and assigned");

	// Then a global, and a type's static and const vars are those
	gives(&mut world, &[
		("ticks > 0", "1"),
		("early_global", "5"),
		("global_number = 6", "6"),
		("global.global_number", "6"),
		("global_number -= 1", "5"),
		("shared", "9"),
		("K", "4")
	]);
	ok("a global by its bare name, read and assigned");

	gives(&mut world, &[("bump(1)", "4"), ("n", "3"), ("n = 1", "1"), ("inside(0)", "/list {len = 3}")]);
	ok("a proc of src called without src.");

	// The frame's arguments, not the expression's own
	gives(&mut world, &[("args[1]", "4"), ("length(args)", "1"), ("args", "/list {len = 1}")]);
	ok("args is the frame's");

	// A sub's bump() doubles its argument for the parent's, and adds one
	gives(&mut world, &[("src.bump(1)", "4"), ("src.n", "3"), ("src.n = 1", "1")]);
	ok("a call to an object's proc, which changes the object");

	gives(&mut world, &[
		("new /datum/thing(\"E\")", "E"),
		("list(1, 2, 3)", "/list {len = 3}"),
		("/datum/thing", "/datum/thing"),
		("src?.other?.n", "null"),
		("isnull(src.other)", "1"),
		("null", "null")
	]);
	ok("new, list(), a typepath, ?. and null");

	gives(&mut world, &[("by = 9", "9"), ("by", "9")]);
	assert_eq!(world.argument("by"), "9");
	gives(&mut world, &[("by++", "9"), ("by", "10"), ("by -= 6", "4")]);
	assert_eq!(world.argument("by"), "4");
	ok("assigning to an argument, seen in the frame's arguments; ++ and -=");

	gives(&mut world, &[("local_list[1] = 11", "11"), ("local_list[1]", "11")]);
	let list = world.variable("Locals", "local_list");
	assert_eq!(world.children(&list)[0].1, "11");
	gives(&mut world, &[("src.n = 50", "50"), ("src.n", "50"), ("src.n = 1", "1")]);
	gives(&mut world, &[("global_number = 6", "6"), ("global_number", "6"), ("global_number = 5", "5")]);
	ok("assigning to a list's element, an object's var and a global");

	// In the debug console, a watch or a hover
	let frame = world.frame_id();
	for context in ["repl", "watch", "hover"] {
		let watched = world.evaluate_in(&frame, "src", context);
		assert_eq!(watched["result"], "S");
		let inside = crate::client::named(&world.variables(&watched["variablesReference"].clone()));
		assert_eq!(crate::client::value_of(&inside, "n"), "1", "{}", context);
		let watched = world.evaluate_in(&frame, "local_list", context);
		assert_eq!(world.variables(&watched["variablesReference"].clone()).len(), 3);
		let watched = world.evaluate_in(&frame, "by", context);
		assert_eq!((&watched["result"], &watched["variablesReference"]), (&json!("4"), &json!(0)));
	}
	ok("a result opens when it's an object or a list, in the console, a watch or a hover");

	let result = world.evaluate_in(&frame, "src", "watch");
	assert_eq!(result["type"], "/datum/thing/sub");
	let inside = world.variables(&result["variablesReference"].clone());
	let n = inside.iter().find(|v| v["name"] == "n").unwrap();
	assert_eq!(n["evaluateName"], "(src).n");
	assert_eq!(world.evaluate_in(&frame, "(src).n + 1", "watch")["result"], "2");
	let result = world.evaluate_in(&frame, "local_list", "watch");
	assert_eq!((&result["type"], &result["indexedVariables"]), (&json!("/list"), &json!(3)));
	assert_eq!(world.evaluate_in(&frame, "by", "watch")["type"], "num");
	assert_eq!(world.evaluate_in(&frame, "name", "watch")["type"], "text");
	ok("a result says what kind of value it is, and what's inside it is reached through the expression");

	// The mouse on some code reads it, and that's all
	for (expression, shown) in [("n", "1"), ("src.items[2]", "\"y\""), ("by == 4 && n <= 1", "1"), ("\"(a = b)\"", "\"(a = b)\"")] {
		assert_eq!(world.evaluate_in(&frame, expression, "hover")["result"], shown, "{}", expression);
	}
	for expression in ["bump(1)", "src.bump(1)", "n = 5", "n += 1", "n++", "leaf(n)", "\"[bump(1)]\""] {
		let why = world.refused("evaluate", json!({ "expression": expression, "frameId": frame, "context": "hover" }));
		assert_eq!(why, "a hover doesn't call procs or assign", "{}", expression);
	}
	assert_eq!(world.eval("n"), "1");
	ok("a hover evaluates what reads, and refuses what would call a proc or assign");

	completions(&mut world);

	// sc_inside called inside(), and has a local of its own
	let caller = world.frames(0)[1]["id"].clone();
	assert_eq!(world.evaluate_in(&caller, "in_caller", "watch")["result"], "77");
	assert_eq!(world.evaluate_in(&caller, "in_caller = 78", "repl")["result"], "78");
	assert_eq!(world.evaluate_in(&caller, "s.name", "watch")["result"], "\"S\"");
	// It's a global proc's frame: there's no src to have an n
	let why = world.refused("evaluate", json!({ "expression": "n", "frameId": caller, "context": "watch" }));
	assert_eq!(why, "undefined var: n");
	ok("an expression in a caller's frame sees the caller's locals, and not the callee's src");

	assert_eq!(world.eval_global("global_number"), "5");
	assert_eq!(world.eval_global("leaf(1)"), "4");
	let why = world.refused("evaluate", json!({ "expression": "by", "context": "repl" }));
	assert_eq!(why, "undefined var: by");
	ok("an expression in no frame, while paused, has the globals and nothing of the frames");

	world.break_on(&["leaf_1"]);
	assert_eq!(world.eval("leaf(1)"), "4");
	assert_eq!(world.eval("middle(1)"), "14");
	assert!(!world.has_event("stopped"), "a breakpoint in an evaluated proc stopped the world");
	world.break_on(&[]);
	ok("a proc with a breakpoint in it, called by an expression, doesn't stop");

	bodies(&mut world);
	smaller_things(&mut world);

	// What can't be evaluated fails, and says why: a client shows that where
	// the value would be
	let mut fails = |world: &mut World, cases: &[(&str, &str)]| {
		let frame = world.frame_id();
		for (expression, expected) in cases {
			for context in ["repl", "watch"] {
				let why = world.refused("evaluate", json!({ "expression": expression, "frameId": frame, "context": context }));
				assert!(why.starts_with(expected), "{} in a {} said {:?}", expression, context, why);
			}
		}
	};
	fails(&mut world, &[
		("1 +", "parser error: "),
		("", "parser error: "),
		("by by", "expected end"),
		("5 = by", "expected l-value")
	]);
	ok("an expression that doesn't parse fails with the parser's words");

	fails(&mut world, &[
		("not_a_name_anywhere", "undefined var: not_a_name_anywhere"),
		// A var of another type: src doesn't have it, and it's no global
		("hp", "undefined var: hp"),
		("hp = 3", "undefined var: hp"),
		("not_a_proc_anywhere(1)", "there's no proc /proc/not_a_proc_anywhere"),
		("src.not_a_name_anywhere", "runtime error: undefined variable /datum/thing/sub/var/not_a_name_anywhere"),
		("/datum/not_a_type", "there's no type /datum/not_a_type"),
		("sleep(1)", "there's no proc /proc/sleep"),
		("..()", "relative calls are not supported")
	]);
	ok("a name, a proc or a type the world doesn't have, and what expressions can't do, fail by name");

	fails(&mut world, &[
		("src.other.name", "runtime error: Cannot read null.name"),
		("1 / 0", "runtime error: Division by zero"),
		("local_list[9]", "runtime error: list index out of bounds"),
		("fail_null()", "runtime error: cannot read from list"),
		("src.shared.len", "runtime error: Cannot read 9.len"),
		// (a statement, which is a body)
		("CRASH(\"no\")", "runtime error: no"),
		// An expression is over when it's answered: it can't wait
		("sleeper(1)", "the expression didn't finish: something it called went to sleep")
	]);
	assert_eq!(world.eval("by"), "4");
	assert_eq!(world.frames(0).len(), 3);
	ok("an expression whose code fails gives the runtime error, and the stop is as it was");

	let stale = world.try_request("evaluate", json!({ "expression": "by", "frameId": 99, "context": "watch" }));
	assert_eq!(stale["success"], false, "{}", stale);
	assert!(stale["message"].as_str().unwrap().starts_with("there's no frame 99"), "{}", stale);
	assert!(!world.output.contains("[auxtools]"), "the debug server had something to say in the console:\n{}", world.output);
	ok("an expression in a frame that isn't there fails; none of it is said in the console");

	let disassembly = world.eval("#dis");
	assert!(
		disassembly.starts_with("Dism for Proc(\"/datum/thing/proc/inside\")") && disassembly.contains("DbgLine"),
		"#dis said {}",
		disassembly
	);
	let disassembly = world.eval("#dis /proc/leaf");
	assert!(disassembly.contains("/proc/leaf") && disassembly.contains("DbgFile \"stepping.dm\""), "#dis said {}", disassembly);
	assert!(world.eval("#help").contains("disassemble"));
	ok("#dis shows the frame's proc's bytecode, or a named proc's");

	// As a client sends its watches: all at once
	let requests: Vec<(&str, serde_json::Value)> = (0..100)
		.map(|i| ("evaluate", json!({ "expression": format!("by + {}", i), "frameId": frame, "context": "watch" })))
		.collect();
	let responses = world.pipeline(&requests);
	for (i, response) in responses.iter().enumerate() {
		assert_eq!(response["body"]["result"], (4 + i).to_string(), "{}", response);
	}
	ok("a hundred expressions sent back to back are answered in order");
	world.resume_through("inside");

	assert_eq!(world.eval_global("global_number + 1"), "6");
	assert_eq!(world.eval_global("global_number = 8"), "8");
	assert_eq!(world.eval_global("global_number"), "8");
	assert_eq!(world.eval_global("leaf(2)"), "6");
	assert!(!world.has_event("stopped"));
	world.run_through("plain");
	ok("expressions while the world runs, which doesn't pause for them");
	world.end();
}
