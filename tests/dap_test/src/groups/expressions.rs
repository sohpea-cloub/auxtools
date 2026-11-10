//! Evaluating expressions: in the debug console, as watches, in any frame

use serde_json::json;

use crate::{ok, world::World};

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

	// What can't be evaluated fails, and says why: a client shows that where
	// the value would be
	let mut fails = |world: &mut World, cases: &[(&str, &str)]| {
		let frame = world.frame_id();
		for (expression, expected) in cases {
			for context in ["repl", "watch", "hover"] {
				let why = world.refused("evaluate", json!({ "expression": expression, "frameId": frame, "context": context }));
				assert!(why.starts_with(expected), "{} in a {} said {:?}", expression, context, why);
			}
		}
	};
	fails(&mut world, &[
		("1 +", "parser error: "),
		("", "parser error: "),
		("by by", "expected end"),
		// (read as `var` divided by `q`)
		("var/q = 3", "undefined var: var"),
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
		("CRASH(\"no\")", "unsupported built-in proc: CRASH"),
		("..()", "relative calls are not supported")
	]);
	ok("a name, a proc or a type the world doesn't have, and what expressions can't do, fail by name");

	fails(&mut world, &[
		("src.other.name", "runtime error: Cannot read null.name"),
		("1 / 0", "runtime error: Division by zero"),
		("local_list[9]", "runtime error: list index out of bounds"),
		("fail_null()", "runtime error: cannot read from list"),
		("src.shared.len", "runtime error: Cannot read 9.len"),
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
