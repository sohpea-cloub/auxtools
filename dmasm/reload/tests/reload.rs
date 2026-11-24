use std::collections::HashSet;
use std::path::PathBuf;

use dmasm_bytecode::{Instruction, Node};
use dmasm_compiler::diagnostic::{Diagnostic, Id};
use dmasm_reload::reload::{
    diff, Changes, ClientScript, ConstValue, InList, NewVar, ParseOptions, ProcKey, ProcSettings,
    Refusal, Snapshot, Until,
};

struct World {
    dir: PathBuf,
}

impl World {
    fn new(name: &str, code: &str) -> World {
        let dir =
            std::env::temp_dir().join(format!("dmasm-reload-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("code")).unwrap();
        std::fs::write(dir.join("world.dme"), "#include \"code/code.dm\"\n").unwrap();
        let world = World { dir };
        world.write(code);
        world
    }

    fn write(&self, code: &str) {
        std::fs::write(self.dir.join("code/code.dm"), code).unwrap();
    }

    fn parse(&self) -> Snapshot {
        Snapshot::parse(&self.dir.join("world.dme"), &ParseOptions::default()).unwrap()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn changes_of(name: &str, before: &str, after: &str) -> Changes {
    changes(name, before, after)
}

fn changes(name: &str, before: &str, after: &str) -> Changes {
    let world = World::new(name, before);
    let old = world.parse();
    world.write(after);
    let new = world.parse();
    let edited: HashSet<PathBuf> = vec![PathBuf::from("code/code.dm")].into_iter().collect();
    diff(&old, &new, Some(&edited))
}

fn texts(diagnostics: &[Diagnostic]) -> Vec<String> {
    diagnostics.iter().map(ToString::to_string).collect()
}

fn key(path: &str, override_id: u32) -> ProcKey {
    ProcKey {
        path: path.to_owned(),
        override_id,
    }
}

fn reloaded(changes: &Changes) -> Vec<ProcKey> {
    assert!(changes.refused.is_empty(), "refused: {:?}", changes.refused);
    changes
        .plan
        .reloads
        .iter()
        .map(|r| r.proc.clone())
        .collect()
}

#[test]
fn nothing_changed() {
    let code = "/proc/a()\n\treturn 1\n";
    assert!(changes("same", code, code).is_empty());
}

#[test]
fn a_changed_body_is_compiled() {
    let changes = changes(
        "body",
        "/proc/a()\n\treturn 1\n/proc/b()\n\treturn 2\n",
        "/proc/a()\n\treturn 1\n/proc/b()\n\tvar/x = 3\n\treturn x\n",
    );
    assert_eq!(reloaded(&changes), [key("/b", 0)]);
    assert!(changes.plan.line_moves.is_empty());
    let reload = &changes.plan.reloads[0];
    assert_eq!(reload.locals, ["x"]);
    assert_eq!(
        reload.nodes.first(),
        Some(&Node::Instruction(
            Instruction::DbgFile(dmasm_bytecode::DMString(b"code/code.dm".to_vec())),
            ()
        ))
    );
}

#[test]
fn a_reloaded_assignment_operator_returns_src_by_default() {
    let changes = changes(
        "assignment-operator",
        "/datum/d/proc/operator+=(b)\n\treturn src\n/datum/d/proc/operator+(b)\n\treturn src\n",
        "/datum/d/proc/operator+=(b)\n\tworld.log << b\n/datum/d/proc/operator+(b)\n\tworld.log << b\n",
    );
    let mut reloaded = reloaded(&changes);
    reloaded.sort_by(|a, b| a.path.cmp(&b.path));
    assert_eq!(
        reloaded,
        [key("/datum/d/operator+", 0), key("/datum/d/operator+=", 0)]
    );
    let prologue = |path: &str| -> Vec<Instruction> {
        let reload = changes
            .plan
            .reloads
            .iter()
            .find(|r| r.proc.path == path)
            .unwrap();
        reload
            .nodes
            .iter()
            .filter_map(|node| match node {
                Node::Instruction(Instruction::DbgFile(_) | Instruction::DbgLine(_), ()) => None,
                Node::Instruction(ins, ()) => Some(ins.clone()),
                _ => None,
            })
            .take(2)
            .collect()
    };
    let dot_src = [
        Instruction::GetVar(dmasm_bytecode::Variable::Src),
        Instruction::SetVar(dmasm_bytecode::Variable::Dot),
    ];
    assert_eq!(prologue("/datum/d/operator+="), dot_src);
    assert_ne!(prologue("/datum/d/operator+"), dot_src);
}

#[test]
fn files_of_an_included_environment_are_named_as_it_names_them() {
    let world = World::new("names", "/proc/a()\n\treturn 1\n");
    let dir = &world.dir;
    std::fs::create_dir_all(dir.join("lib/sub")).unwrap();
    std::fs::write(
        dir.join("world.dme"),
        "#include \"code/code.dm\"\n#include \"lib\\lib.dme\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("lib/lib.dme"),
        "#include \"std.dm\"\n#include \"sub\\more.dm\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("lib/std.dm"), "/proc/b()\n\treturn 2\n").unwrap();
    std::fs::write(dir.join("lib/sub/more.dm"), "/proc/c()\n\treturn 3\n").unwrap();
    let old = world.parse();

    let file_of = |snapshot: &Snapshot, proc: &str| match snapshot
        .compile(&key(proc, 0))
        .unwrap()
        .nodes
        .first()
    {
        Some(Node::Instruction(Instruction::DbgFile(name), ())) => {
            String::from_utf8(name.0.clone()).unwrap()
        }
        other => panic!("{proc} starts with {other:?}"),
    };
    assert_eq!(file_of(&old, "/a"), "code/code.dm");
    assert_eq!(file_of(&old, "/b"), "std.dm");
    assert_eq!(file_of(&old, "/c"), "sub/more.dm");
    assert_eq!(old.proc_file(&key("/b", 0)).as_deref(), Some("lib/std.dm"));

    std::fs::write(dir.join("lib/std.dm"), "/proc/b()\n\treturn 4\n").unwrap();
    let changes = diff(&old, &world.parse(), None);
    assert_eq!(reloaded(&changes), [key("/b", 0)]);
    assert_eq!(
        changes.plan.reloads[0].nodes.first(),
        Some(&Node::Instruction(
            Instruction::DbgFile(dmasm_bytecode::DMString(b"std.dm".to_vec())),
            ()
        ))
    );
}

#[test]
fn a_file_included_earlier_moves_no_defaults() {
    let world = World::new(
        "renumbered",
        "/obj/thing\n\tname = \"thing\"\n\n/proc/a()\n\treturn /obj/thing::name\n",
    );
    let old = world.parse();
    std::fs::write(world.dir.join("code/first.dm"), "/proc/b()\n\treturn 2\n").unwrap();
    std::fs::write(
        world.dir.join("world.dme"),
        "#include \"code/first.dm\"\n#include \"code/code.dm\"\n",
    )
    .unwrap();
    let edited: HashSet<PathBuf> = ["world.dme", "code/first.dm"]
        .iter()
        .map(PathBuf::from)
        .collect();
    let changes = diff(&old, &world.parse(), Some(&edited));
    assert!(
        changes.plan.line_moves.is_empty(),
        "{:?}",
        changes.plan.line_moves
    );
    assert!(reloaded(&changes).is_empty(), "{:?}", reloaded(&changes));
}

#[test]
fn procs_below_an_edit_move_lines() {
    let changes = changes(
        "lines",
        "/proc/a()\n\treturn 1\n\n/proc/b()\n\tworld.log << 1\n\treturn 2\n",
        "/proc/a()\n\tworld.log << 0\n\treturn 1\n\n/proc/b()\n\tworld.log << 1\n\treturn 2\n",
    );
    assert_eq!(reloaded(&changes), [key("/a", 0)]);
    assert_eq!(changes.plan.line_moves.len(), 1);
    assert_eq!(changes.plan.line_moves[0].proc, key("/b", 0));
    assert_eq!(changes.plan.line_moves[0].lines, [(5, 6), (6, 7)]);
}

#[test]
fn types_and_overrides_are_named_like_auxtools_does() {
    let before = "/datum/thing/proc/f()\n\treturn 1\n/datum/thing/f()\n\treturn ..() + 1\n/datum/thing/verb/v()\n\treturn 3\n";
    let after = "/datum/thing/proc/f()\n\treturn 1\n/datum/thing/f()\n\treturn ..() + 2\n/datum/thing/verb/v()\n\treturn 4\n";
    let changes = changes("overrides", before, after);
    let mut keys = reloaded(&changes);
    keys.sort();
    assert_eq!(keys, [key("/datum/thing/f", 1), key("/datum/thing/v", 0)]);
}

#[test]
fn a_define_change_recompiles_its_users() {
    let mut keys = reloaded(&changes(
        "define",
        "#define N 1\n/proc/a()\n\treturn N\n/proc/b()\n\treturn 2\n",
        "#define N 5\n/proc/a()\n\treturn N\n/proc/b()\n\treturn 2\n",
    ));
    keys.sort();
    assert_eq!(keys, [key("/a", 0)]);
}

#[test]
fn a_const_change_recompiles_its_users() {
    let before = "var/const/LIMIT = 3\n/datum/thing\n\tvar/const/SIZE = 2\n/proc/a()\n\treturn LIMIT\n/proc/b()\n\treturn 2\n/datum/thing/proc/c()\n\treturn SIZE * 2\n/datum/thing/proc/d()\n\treturn \"SIZE\"\n";
    let after = before
        .replace("LIMIT = 3", "LIMIT = 4")
        .replace("SIZE = 2", "SIZE = 5");
    let changes = changes("const", before, &after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert_eq!(changes.consts, ["LIMIT", "SIZE"]);
    let set: Vec<(&str, &str, Option<ConstValue>)> = changes
        .plan
        .redeclared
        .iter()
        .map(|r| (r.type_path.as_str(), r.var.as_str(), r.value.clone()))
        .collect();
    assert_eq!(
        set,
        [
            ("", "LIMIT", Some(ConstValue::Number(4.0))),
            ("/datum/thing", "SIZE", Some(ConstValue::Number(5.0)))
        ]
    );
    assert_eq!(reloaded(&changes), [key("/datum/thing/c", 0)]);
    let changes = changes_of(
        "const-moved",
        before,
        &after.replace("/datum/thing\n", "/datum/thing\n\n"),
    );
    assert_eq!(reloaded(&changes), [key("/datum/thing/c", 0)]);
    let moved: Vec<ProcKey> = changes
        .plan
        .line_moves
        .iter()
        .map(|m| m.proc.clone())
        .collect();
    assert_eq!(
        moved,
        [key("/a", 0), key("/b", 0), key("/datum/thing/d", 0)]
    );
    assert!(changes.warnings.is_empty(), "{:?}", changes.warnings);
}

#[test]
fn a_changed_const_is_found_wherever_a_proc_names_it() {
    let before = "var/const/LIMIT = 3
/datum/thing
	var/const/SIZE = 2
/proc/in_default(x = LIMIT + 1)
	return x
/proc/in_condition(x)
	if(x > LIMIT + 1)
		return 1
	else if(x < LIMIT - 9)
		return 2
/proc/in_loops(x)
	while(x < LIMIT + 1)
		x++
	do
		x--
	while(x > LIMIT + 2)
	for(x = LIMIT + 3, x < LIMIT + 9, x += LIMIT + 4)
		world.log << x
/proc/in_switch(x)
	switch(x)
		if(LIMIT)
			return 1
		if(10 to LIMIT + 20)
			return 2
/proc/in_range()
	for(var/i in 1 to LIMIT + 1)
		world.log << i
/proc/in_text()
	return \"up to [LIMIT + 1]\"
/proc/in_list()
	return list(1, \"key\" = LIMIT + 1)
/proc/in_argument()
	return in_condition(LIMIT + 1)
/proc/in_new()
	return new /datum/thing(LIMIT + 1)
/proc/in_field(datum/thing/t)
	return t.SIZE * 2
/proc/in_index(list/l)
	return l[LIMIT + 1]
/proc/in_call_of(datum/thing/t)
	return t.in_method(LIMIT + 1)
/datum/thing/proc/in_method(x)
	return x ? SIZE * 2 : 0
/proc/in_spawn()
	spawn(LIMIT + 1)
		return
/proc/in_local()
	var/x = LIMIT + 1
	return x
/proc/in_try()
	try
		throw LIMIT + 1
	catch(var/e)
		return e
/proc/in_label()
	again:
		return LIMIT + 1
/proc/unrelated(x)
	return x + 3
";
    let after = before
        .replace("LIMIT = 3", "LIMIT = 4")
        .replace("SIZE = 2", "SIZE = 5");
    let changes = changes("const-places", before, &after);
    assert_eq!(changes.consts, ["LIMIT", "SIZE"]);
    let mut procs: Vec<String> = reloaded(&changes).iter().map(|k| k.to_string()).collect();
    procs.sort();
    assert_eq!(
        procs,
        [
            "/datum/thing/in_method",
            "/in_argument",
            "/in_call_of",
            "/in_condition",
            "/in_default",
            "/in_field",
            "/in_index",
            "/in_label",
            "/in_list",
            "/in_local",
            "/in_loops",
            "/in_new",
            "/in_range",
            "/in_spawn",
            "/in_switch",
            "/in_text",
            "/in_try",
        ]
    );
}

#[test]
fn a_new_const_is_a_global() {
    let before = "/datum/thing
	var/x = 1
/datum/thing/sub
";
    let after = "/datum/thing
	var/x = 1
	var/const/K = 7
/datum/thing/sub
/datum/other
	var/const/J = 2
";
    let changes = changes("new-const", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let new: Vec<(&str, &str, u32)> = changes
        .plan
        .new_vars
        .iter()
        .map(|v| (v.type_path.as_str(), v.var.as_str(), v.flags))
        .collect();
    assert_eq!(new, [("/datum/other", "J", 3), ("/datum/thing", "K", 3)]);
    assert!(
        changes.plan.var_defaults.is_empty(),
        "{:?}",
        changes.plan.var_defaults
    );
}

#[test]
fn a_var_declared_with_another_type_recompiles_its_users() {
    let before = "/datum/holder\n\tvar/list/items = null\n/datum/holder/proc/count()\n\treturn items.len\n/datum/holder/proc/other()\n\treturn 1\n";
    let after = before.replace("var/list/items", "var/items");
    let changes = changes("retyped", before, &after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert_eq!(changes.retyped, ["items"]);
    assert!(!reloaded(&changes).contains(&key("/datum/holder/other", 0)));
}

#[test]
fn vars_declared_another_way() {
    let base = "/datum/a\n\tvar/x = 1\n/datum/a/b\n/proc/f()\n\tvar/datum/a/a = new\n\treturn a.x\n/datum/a/proc/g()\n\treturn x\n";
    let redeclared = |changes: &Changes| -> Vec<(String, u32, Option<ConstValue>)> {
        assert!(changes.refused.is_empty(), "{:?}", changes.refused);
        changes
            .plan
            .redeclared
            .iter()
            .map(|r| (r.var.clone(), r.flags, r.value.clone()))
            .collect()
    };

    let changes = changes_of(
        "to-static",
        base,
        &base.replace("var/x = 1", "var/static/x = 2"),
    );
    assert_eq!(
        redeclared(&changes),
        [("x".to_owned(), 1, Some(ConstValue::Number(2.0)))]
    );
    assert_eq!(reloaded(&changes), [key("/datum/a/g", 0), key("/f", 0)]);
    assert!(changes.plan.var_defaults.is_empty());

    let changes = changes_of(
        "to-const",
        base,
        &base.replace("var/x = 1", "var/const/x = 1"),
    );
    assert_eq!(
        redeclared(&changes),
        [("x".to_owned(), 3, Some(ConstValue::Number(1.0)))]
    );
    assert_eq!(reloaded(&changes), [key("/datum/a/g", 0), key("/f", 0)]);

    let before = base.replace("var/x = 1", "var/static/x = 1");
    let after = base.replace("/datum/a/b\n", "/datum/a/b\n\tx = 5\n");
    let changes = changes_of("from-static", &before, &after);
    assert!(
        changes.plan.line_moves.is_empty(),
        "{:?}",
        changes.plan.line_moves
    );
    assert_eq!(redeclared(&changes), [("x".to_owned(), 0, None)]);
    assert_eq!(reloaded(&changes), [key("/datum/a/g", 0), key("/f", 0)]);
    let mut defaults: Vec<(ConstValue, Vec<String>)> = changes
        .plan
        .var_defaults
        .iter()
        .map(|d| (d.value.clone(), d.types.clone()))
        .collect();
    defaults.sort_by_key(|d| d.1.clone());
    defaults.dedup();
    assert_eq!(
        defaults,
        [
            (ConstValue::Number(1.0), vec!["/datum/a".to_owned()]),
            (ConstValue::Number(5.0), vec!["/datum/a/b".to_owned()])
        ]
    );

    let changes = changes_of("to-tmp", base, &base.replace("var/x = 1", "var/tmp/x = 1"));
    assert_eq!(redeclared(&changes), [("x".to_owned(), 4, None)]);
    assert!(reloaded(&changes).is_empty());
    let changes = changes_of(
        "static-tmp",
        &before,
        &before.replace("var/static/x", "var/static/tmp/x"),
    );
    assert_eq!(redeclared(&changes), [("x".to_owned(), 5, None)]);

    let changes = changes_of("as", base, &base.replace("var/x = 1", "var/x = 1 as num"));
    assert!(redeclared(&changes).is_empty());
}

#[test]
fn type_macro_in_an_inherited_default_is_the_declaring_type() {
    let before = "/datum/a\n/datum/a/b\n";
    let after = "/datum/a\n\tvar/list/l = list(__TYPE__)\n/datum/a/b\n";
    let changes = changes("type-macro", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert_eq!(changes.plan.init_procs.len(), 2);
    for init in &changes.plan.init_procs {
        let code = init.code.as_ref().unwrap();
        assert!(
            code.nodes.iter().any(|n| matches!(n, Node::Instruction(Instruction::PushVal(v), _) if format!("{:?}", v) == "Path(\"/datum/a\")")),
            "{}: {:?}",
            init.type_path,
            code.nodes
        );
    }
}

#[test]
fn spaceman_dmm_is_not_defined() {
    let code = |n: u32| {
        format!("/proc/a()\n#ifdef SPACEMAN_DMM\n\treturn 0\n#else\n\treturn {n}\n#endif\n")
    };
    let changes = changes("spaceman", &code(1), &code(2));
    assert_eq!(reloaded(&changes), [key("/a", 0)]);
    let code = dmasm_bytecode::format(&changes.plan.reloads[0].nodes);
    assert!(code.contains("PushInt 2"), "{}", code);
}

#[test]
fn default_arguments_can_change() {
    let changes = changes(
        "default-args",
        "/proc/a(x = 1)\n\treturn x\n",
        "/proc/a(x = 2)\n\treturn x\n",
    );
    assert_eq!(reloaded(&changes), [key("/a", 0)]);
}

fn refusal(name: &str, before: &str, after: &str) -> String {
    let changes = changes(name, before, after);
    assert!(
        !changes.refused.is_empty(),
        "nothing refused: {:?}",
        changes
    );
    shown(&changes.refused).join("\n")
}

fn shown(refused: &[Refusal]) -> Vec<String> {
    refused.iter().map(Refusal::to_string).collect()
}

#[test]
fn refuses_what_a_reload_cant_do() {
    let base = "/datum/thing\n\tvar/a = 1\n\tvar/list/l = list()\n/datum/thing/proc/f(x)\n\tset waitfor = FALSE\n\treturn x\n/obj/verb/v()\n\treturn\n";
    let cases = [
        (
            "new-type-builtin",
            format!("{base}/mob/other\n\tckey = \"someone\"\n"),
            "/mob/other: ckey: unsupported built-in default on a new type",
        ),
        (
            "setting",
            base.replace(
                "/obj/verb/v()\n\treturn",
                "/obj/verb/v()\n\tset src in usr.loc.contents\n\treturn",
            ),
            "unsupported `set src` expression",
        ),
        (
            "parse-error",
            base.replace("return x", "return (x"),
            "parse error",
        ),
    ];
    for (name, after, why) in cases.iter() {
        let refused = refusal(name, base, after);
        assert!(
            refused.contains(*why),
            "{}: expected {:?} in {:?}",
            name,
            why,
            refused
        );
    }
}

#[test]
fn builtin_var_defaults() {
    let before = "/obj/thing\n\tname = \"a\"\n/obj/thing/sub\n/obj/thing/own\n\tpixel_x = 2\n";
    let after = before.replace("\"a\"", "\"b\"").replace(
        "/obj/thing\n",
        "/obj/thing\n\tpixel_x = 5\n\tgender = \"male\"\n",
    );
    let changes = changes("builtin", before, &after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let mut defaults: Vec<(String, ConstValue, Vec<String>)> = changes
        .plan
        .builtin_defaults
        .iter()
        .map(|d| (d.var.clone(), d.value.clone(), d.types.clone()))
        .collect();
    defaults.sort_by(|a, b| a.0.cmp(&b.0));
    let both = vec!["/obj/thing".to_owned(), "/obj/thing/sub".to_owned()];
    let all = vec![
        "/obj/thing".to_owned(),
        "/obj/thing/own".to_owned(),
        "/obj/thing/sub".to_owned(),
    ];
    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };
    let defaults: Vec<_> = defaults
        .into_iter()
        .map(|(var, value, types)| (var, value, sorted(types)))
        .collect();
    assert_eq!(
        defaults,
        [
            (
                "gender".to_owned(),
                ConstValue::String("male".to_owned()),
                all.clone()
            ),
            (
                "name".to_owned(),
                ConstValue::String("b".to_owned()),
                all.clone()
            ),
            ("pixel_x".to_owned(), ConstValue::Number(5.0), both),
            ("text".to_owned(), ConstValue::String("b".to_owned()), all),
        ]
    );
    assert!(changes.plan.var_defaults.is_empty());

    let refused = refusal("builtin-mob", "/mob/a\n", "/mob/a\n\tckey = \"someone\"\n");
    assert!(refused.contains("built-in default"), "{}", refused);
}

#[test]
fn new_procs_are_compiled() {
    let before = "/datum/thing/proc/f()\n\treturn 1\n/datum/thing/sub\n";
    let after = "/datum/thing/proc/f()\n\treturn 1\n/datum/thing/sub\n/proc/g()\n\treturn 2\n/datum/thing/proc/h(a, obj/b, c as num)\n\treturn a\n/datum/thing/sub/f()\n\treturn ..() + 1\n";
    let changes = changes("new-procs", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let mut new: Vec<(String, String, bool, String)> = changes
        .plan
        .new_procs
        .iter()
        .map(|p| {
            (
                p.type_path.clone(),
                p.name.clone(),
                p.declared,
                p.code.proc.path.clone(),
            )
        })
        .collect();
    new.sort();
    assert_eq!(
        new,
        [
            ("".to_owned(), "g".to_owned(), true, "/g".to_owned()),
            (
                "/datum/thing".to_owned(),
                "h".to_owned(),
                true,
                "/datum/thing/h".to_owned()
            ),
            (
                "/datum/thing/sub".to_owned(),
                "f".to_owned(),
                false,
                "/datum/thing/sub/f".to_owned()
            ),
        ]
    );
    let h = changes
        .plan
        .new_procs
        .iter()
        .find(|p| p.name == "h")
        .unwrap();
    let params: Vec<(String, u32)> = h
        .code
        .params
        .iter()
        .map(|p| (p.name.clone(), p.input))
        .collect();
    assert_eq!(
        params,
        [
            ("a".to_owned(), 0),
            ("b".to_owned(), 2),
            ("c".to_owned(), 8)
        ]
    );
}

#[test]
fn removed_procs_and_vars_stay() {
    let before =
        "/datum/thing\n\tvar/a = 1\n/datum/thing/proc/f()\n\treturn 1\n/proc/g()\n\treturn f()\n";
    let after = "/datum/thing\n/proc/g()\n\treturn 2\n";
    let changes = changes("removed", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let warnings = texts(&changes.warnings).join("\n");
    assert!(warnings.contains("var a was removed"), "{}", warnings);
    assert_eq!(changes.plan.removed_procs.len(), 1);
    assert_eq!(changes.plan.removed_procs[0].proc, key("/datum/thing/f", 0));
    assert_eq!(reloaded(&changes), [key("/g", 0)]);
}

#[test]
fn what_was_removed_is_listed_with_its_warning() {
    let before = "var/total = 1\n/datum/thing\n\tvar/a = 1\n\tvar/b = 2\n\
                  /proc/g()\n\treturn 1\n/proc/h()\n\treturn 2\n";
    let after = "/datum/thing\n\tvar/b = 2\n/proc/h()\n\treturn 2\n";
    let changes = changes("removed-listed", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let vars: Vec<(&str, &str)> = changes
        .plan
        .removed_vars
        .iter()
        .map(|v| (v.type_path.as_str(), v.var.as_str()))
        .collect();
    assert_eq!(vars, [("/datum/thing", "a")]);
    let procs: Vec<Option<String>> = changes
        .plan
        .removed_procs
        .iter()
        .map(|p| p.warning().as_ref().map(ToString::to_string))
        .collect();
    assert_eq!(procs.len(), 1, "{:?}", changes.plan.removed_procs);
    let mut expected = vec![
        "(global): var total was removed from source but retained in the running world".to_owned(),
        changes.plan.removed_vars[0].warning().to_string(),
        procs[0].clone().unwrap(),
    ];
    expected.sort();
    let mut warnings = texts(&changes.warnings);
    warnings.sort();
    assert_eq!(warnings, expected);
    for (id, subject) in [
        (Id::RemovedGlobalRetained, "(global)"),
        (Id::RemovedVarRetained, "/datum/thing"),
        (Id::RemovedProcRetained, "/g"),
    ] {
        assert!(
            changes
                .warnings
                .iter()
                .any(|w| w.id == id && w.subject.as_deref() == Some(subject)),
            "{id:?} {subject}: {:?}",
            changes.warnings
        );
    }
    assert_eq!(
        changes.plan.removed_vars[0].warning().to_string(),
        "/datum/thing: var a was removed from source but retained in the running world"
    );
    assert_eq!(
        procs[0].as_deref(),
        Some("/g: a definition was removed from source but retained in the running world")
    );

    let before = "/datum/thing/proc/f()\n\treturn 1\n/datum/thing/proc/k()\n\treturn 1\n";
    let changes = changes_of(
        "removed-type-proc",
        before,
        "/datum/thing/proc/k()\n\treturn 1\n",
    );
    assert_eq!(changes.plan.removed_procs.len(), 1);
    assert_eq!(changes.plan.removed_procs[0].warning(), None);
    assert!(changes.warnings.is_empty(), "{:?}", changes.warnings);
}

#[test]
fn a_removed_var_a_subtype_overrode_stays() {
    let before = "/datum/thing\n\tvar/a = 1\n/datum/thing/sub\n\ta = 2\n";
    let after = "/datum/thing\n/datum/thing/sub\n";
    let changes = changes("removed-override", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert!(
        texts(&changes.warnings)
            .join("\n")
            .contains("var a was removed"),
        "{:?}",
        changes.warnings
    );
    assert!(
        changes.plan.var_defaults.is_empty(),
        "{:?}",
        changes.plan.var_defaults
    );
}

#[test]
fn removed_overrides_are_unlinked() {
    let before = "/datum/a/b\n\tvar/x = 1\n/datum/a/proc/f()\n\treturn 1\n/datum/a/b/f()\n\treturn 2\n/datum/a/b/f()\n\treturn 3\n/proc/g()\n\treturn 1\n";
    let changes = changes(
        "removed-last",
        before,
        &before.replace("/datum/a/b/f()\n\treturn 3\n", ""),
    );
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let removed: Vec<(ProcKey, &str)> = changes
        .plan
        .removed_procs
        .iter()
        .map(|r| (r.proc.clone(), r.type_path.as_str()))
        .collect();
    assert_eq!(removed, [(key("/datum/a/b/f", 1), "/datum/a/b")]);

    let changes = changes_of(
        "removed-both",
        before,
        &before.replace(
            "/datum/a/b/f()\n\treturn 2\n/datum/a/b/f()\n\treturn 3\n",
            "",
        ),
    );
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert_eq!(changes.plan.removed_procs.len(), 2);

    let changes = changes_of(
        "removed-first",
        before,
        &before.replace("/datum/a/b/f()\n\treturn 2\n", ""),
    );
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let removed: Vec<ProcKey> = changes
        .plan
        .removed_procs
        .iter()
        .map(|r| r.proc.clone())
        .collect();
    assert_eq!(removed, [key("/datum/a/b/f", 0)]);
    assert!(reloaded(&changes).is_empty(), "{:?}", reloaded(&changes));

    let changes = changes_of(
        "removed-global",
        before,
        &before.replace("/proc/g()\n\treturn 1\n", ""),
    );
    let removed: Vec<(ProcKey, &str)> = changes
        .plan
        .removed_procs
        .iter()
        .map(|r| (r.proc.clone(), r.type_path.as_str()))
        .collect();
    assert_eq!(removed, [(key("/g", 0), "")]);
    assert!(texts(&changes.warnings)
        .join("\n")
        .contains("/g: a definition was removed"));
}

#[test]
fn definitions_come_among_the_others() {
    let before =
        "/datum/a/proc/f()\n\treturn 1\n/datum/a/f()\n\treturn 2\n/datum/a/f()\n\treturn 3\n";

    let after = before.replace(
        "/datum/a/f()\n\treturn 2\n",
        "/datum/a/f()\n\treturn 9\n/datum/a/f()\n\treturn 2\n",
    );
    let changes = changes_of("inserted", before, &after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let added: Vec<ProcKey> = changes
        .plan
        .new_procs
        .iter()
        .map(|p| p.code.proc.clone())
        .collect();
    assert_eq!(added, [key("/datum/a/f", 1)]);
    assert!(changes.plan.removed_procs.is_empty());
    assert!(reloaded(&changes).is_empty(), "{:?}", reloaded(&changes));
    let moved: Vec<ProcKey> = changes
        .plan
        .line_moves
        .iter()
        .map(|m| m.proc.clone())
        .collect();
    assert_eq!(moved, [key("/datum/a/f", 1), key("/datum/a/f", 2)]);

    let changes = changes_of(
        "inserted-and-changed",
        before,
        &after.replace("return 3", "return 4"),
    );
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let added: Vec<ProcKey> = changes
        .plan
        .new_procs
        .iter()
        .map(|p| p.code.proc.clone())
        .collect();
    assert_eq!(added, [key("/datum/a/f", 1)]);
    assert_eq!(reloaded(&changes), [key("/datum/a/f", 2)]);

    let after = before.replace("/datum/a/f()\n\treturn 2\n", "").replace(
        "/datum/a/f()\n\treturn 3\n",
        "/datum/a/f()\n\treturn 3\n/datum/a/f()\n\treturn 5\n/datum/a/f()\n\treturn 6\n",
    );
    let changes = changes_of("swapped", before, &after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert!(changes.plan.removed_procs.is_empty());
    let added: Vec<ProcKey> = changes
        .plan
        .new_procs
        .iter()
        .map(|p| p.code.proc.clone())
        .collect();
    assert_eq!(added, [key("/datum/a/f", 3)]);
    assert_eq!(
        reloaded(&changes),
        [key("/datum/a/f", 1), key("/datum/a/f", 2)]
    );

    let cases = [
        (
            "verb-between",
            "/obj/verb/v()\n\treturn 1\n/obj/v()\n\treturn 2\n",
            "/obj/v()",
            "/obj/v",
            true,
        ),
        (
            "world-between",
            "/world/New()\n\treturn 1\n/world/New()\n\treturn 2\n",
            "/world/New()",
            "/world/New",
            false,
        ),
        (
            "global-between",
            "/proc/g()\n\treturn 1\n/g()\n\treturn 2\n",
            "/g()",
            "/g",
            false,
        ),
    ];
    for (name, before, head, path, verb) in cases {
        let second = format!("{head}\n\treturn 2\n");
        let after = before.replace(&second, &format!("{head}\n\treturn 9\n{second}"));
        let changes = changes_of(name, before, &after);
        assert!(
            changes.refused.is_empty(),
            "{}: {:?}",
            name,
            changes.refused
        );
        let added: Vec<(ProcKey, bool, bool)> = changes
            .plan
            .new_procs
            .iter()
            .map(|p| (p.code.proc.clone(), p.declared, p.verb))
            .collect();
        assert_eq!(added, [(key(path, 1), false, verb)], "{}", name);
        assert!(reloaded(&changes).is_empty(), "{}", name);

        let changes = changes_of(
            name,
            &after,
            &after.replacen("\treturn 9\n", "\treturn 9\n\treturn 8\n", 1),
        );
        assert_eq!(reloaded(&changes), [key(path, 1)], "{}", name);

        let changes = changes_of(name, &after, before);
        let removed: Vec<(ProcKey, bool)> = changes
            .plan
            .removed_procs
            .iter()
            .map(|r| (r.proc.clone(), r.verb))
            .collect();
        assert_eq!(removed, [(key(path, 1), verb)], "{}", name);
        assert!(reloaded(&changes).is_empty(), "{}", name);
    }
}

#[test]
fn var_defaults_go_to_the_subtypes_that_inherit_them() {
    let before =
        "/datum/a\n\tvar/x = 1\n\tvar/y = \"one\"\n/datum/a/b\n/datum/a/c\n\tx = 5\n/datum/a/c/d\n";
    let after = before
        .replace("var/x = 1", "var/x = 2")
        .replace("var/y = \"one\"", "var/y = /datum/a/b");
    let changes = changes("defaults", before, &after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let mut defaults: Vec<(String, ConstValue, Vec<String>)> = changes
        .plan
        .var_defaults
        .iter()
        .map(|d| (d.var.clone(), d.value.clone(), d.types.clone()))
        .collect();
    defaults.sort_by(|a, b| a.0.cmp(&b.0));
    for d in &mut defaults {
        d.2.sort();
    }
    assert_eq!(
        defaults,
        [
            (
                "x".to_owned(),
                ConstValue::Number(2.0),
                vec!["/datum/a".to_owned(), "/datum/a/b".to_owned()]
            ),
            (
                "y".to_owned(),
                ConstValue::Path("/datum/a/b".to_owned()),
                vec![
                    "/datum/a".to_owned(),
                    "/datum/a/b".to_owned(),
                    "/datum/a/c".to_owned(),
                    "/datum/a/c/d".to_owned()
                ]
            ),
        ]
    );
}

#[test]
fn global_var_values_are_left_for_a_reboot() {
    let changes = changes("globals", "var/g = 1\n", "var/g = 2\n");
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert!(changes.plan.var_defaults.is_empty());
    assert_eq!(
        deferred(&changes),
        ["(global): g: changed initializer (reboot)"]
    );
}

#[test]
fn parameters_and_waitfor_can_change() {
    let before = "/proc/f(a)\n\treturn a\n";
    let after = "/proc/f(a, mob/b)\n\tset waitfor = 0\n\tset background = 1\n\treturn a\n";
    let changes = changes("signature", before, after);
    assert_eq!(reloaded(&changes), [key("/f", 0)]);
    let reload = &changes.plan.reloads[0];
    let params: Vec<(String, u32)> = reload
        .params
        .iter()
        .map(|p| (p.name.clone(), p.input))
        .collect();
    assert_eq!(params, [("a".to_owned(), 0), ("b".to_owned(), 1)]);
    assert_eq!(
        reload.settings,
        Some(ProcSettings {
            waitfor: false,
            background: true,
            ..ProcSettings::default()
        })
    );
}

#[test]
fn new_vars_and_their_overrides() {
    let before = "/datum/a\n/datum/a/b\n/datum/a/c\n/datum/a/b/d\n";
    let after = "var/g = 3\n/datum/a\n\tvar/x = 1\n\tvar/static/s = \"t\"\n\tvar/tmp/t\n/datum/a/b\n\tx = 2\n/datum/a/c\n/datum/a/b/d\n";
    let changes = changes("new-vars", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let mut vars = changes.plan.new_vars.clone();
    vars.sort_by(|a, b| a.var.cmp(&b.var));
    let var = |type_path: &str, var: &str, flags: u32, value: ConstValue| NewVar {
        type_path: type_path.to_owned(),
        var: var.to_owned(),
        flags,
        value,
        init: None,
        moved_from: vec![],
    };
    assert_eq!(
        vars,
        [
            var("", "g", 1, ConstValue::Number(3.0)),
            var("/datum/a", "s", 1, ConstValue::String("t".to_owned())),
            var("/datum/a", "t", 4, ConstValue::Null),
            var("/datum/a", "x", 0, ConstValue::Number(1.0)),
        ]
    );
    let mut defaults: Vec<(String, ConstValue, Vec<String>)> = changes
        .plan
        .var_defaults
        .iter()
        .map(|d| (d.var.clone(), d.value.clone(), d.types.clone()))
        .collect();
    defaults.sort_by_key(|d| (d.0.clone(), d.2.clone()));
    for d in &mut defaults {
        d.2.sort();
    }
    assert_eq!(
        defaults,
        [
            (
                "t".to_owned(),
                ConstValue::Null,
                vec![
                    "/datum/a".to_owned(),
                    "/datum/a/b".to_owned(),
                    "/datum/a/b/d".to_owned(),
                    "/datum/a/c".to_owned()
                ]
            ),
            (
                "x".to_owned(),
                ConstValue::Number(1.0),
                vec!["/datum/a".to_owned(), "/datum/a/c".to_owned()]
            ),
            (
                "x".to_owned(),
                ConstValue::Number(2.0),
                vec!["/datum/a/b".to_owned(), "/datum/a/b/d".to_owned()]
            ),
        ]
    );
}

#[test]
fn a_new_var_names_the_proc_dreammaker_gives_for_type_proc() {
    let before = "/datum/t/proc/f()\n/datum/t/f()\n/datum/t/c\n/datum/u\n";
    let after = "/datum/t/proc/f()\n/datum/t/f()\n/datum/t/c\n/datum/u\n\tvar/handler = /datum/t::f()\n\tvar/child = /datum/t/c::f()\n";
    let changes = changes("new-var-proc-reference", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let mut values: Vec<(String, ConstValue)> = changes
        .plan
        .new_vars
        .iter()
        .map(|v| (v.var.clone(), v.value.clone()))
        .collect();
    values.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        values,
        [
            (
                "child".to_owned(),
                ConstValue::Path("/datum/t/f".to_owned())
            ),
            (
                "handler".to_owned(),
                ConstValue::Path("/datum/t/f".to_owned())
            ),
        ]
    );
}

#[test]
fn removing_an_override_goes_back_to_the_inherited_default() {
    let before = "/datum/a\n\tvar/x = 1\n/datum/a/b\n\tx = 2\n";
    let after = "/datum/a\n\tvar/x = 1\n/datum/a/b\n";
    let changes = changes("unoverride", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert_eq!(changes.plan.var_defaults.len(), 1);
    assert_eq!(changes.plan.var_defaults[0].value, ConstValue::Number(1.0));
    assert_eq!(changes.plan.var_defaults[0].types, ["/datum/a/b"]);
}

#[test]
fn static_locals_come_with_their_initial_values() {
    let before = "/datum/a/proc/f()\n\treturn 1\n";
    let after = "/datum/a/proc/f()\n\tvar/static/n = 3\n\tvar/static/list/l = list()\n\tif (n)\n\t\tvar/static/n = \"x\"\n\t\treturn n\n\treturn l\n";
    let changes = changes("statics", before, after);
    let in_f = |nth| dmasm_bytecode::StaticScope::Proc {
        path: "/datum/a/proc/f".to_owned(),
        nth,
    };
    let statics: Vec<(String, dmasm_bytecode::StaticScope, Option<ConstValue>)> =
        changes.plan.reloads[0]
            .statics
            .iter()
            .map(|s| (s.name.clone(), s.scope.clone(), s.value.clone()))
            .collect();
    assert_eq!(
        statics,
        [
            ("n".to_owned(), in_f(0), Some(ConstValue::Number(3.0))),
            ("l".to_owned(), in_f(0), None),
            (
                "n".to_owned(),
                in_f(1),
                Some(ConstValue::String("x".to_owned()))
            ),
        ]
    );
}

#[test]
fn another_definition_after_the_others_is_a_new_proc() {
    let before = "/datum/a/proc/f()\n\treturn 1\n";
    let after = "/datum/a/proc/f()\n\treturn 1\n/datum/a/f()\n\treturn ..() + 1\n/datum/a/f()\n\treturn ..() * 10\n";
    let changes = changes("second-definition", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let added: Vec<(ProcKey, bool)> = changes
        .plan
        .new_procs
        .iter()
        .map(|p| (p.code.proc.clone(), p.declared))
        .collect();
    assert_eq!(
        added,
        [(key("/datum/a/f", 1), false), (key("/datum/a/f", 2), false)]
    );
}

#[test]
fn another_definition_of_a_global_proc_is_a_proc_of_its_own() {
    let before = "/proc/g()\n\treturn 1\n";
    let changes = changes(
        "global-redefinition",
        before,
        &format!("{before}/g()\n\treturn ..() + 1\n"),
    );
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let added: Vec<(ProcKey, &str, bool)> = changes
        .plan
        .new_procs
        .iter()
        .map(|p| (p.code.proc.clone(), p.type_path.as_str(), p.declared))
        .collect();
    assert_eq!(added, [(key("/g", 1), "", false)]);
    assert!(
        texts(&changes.warnings)
            .iter()
            .any(|w| w.contains("unreachable")),
        "{:?}",
        changes.warnings
    );
}

fn pushed_ints(nodes: &[Node]) -> Vec<String> {
    nodes
        .iter()
        .filter_map(|node| match node {
            Node::Instruction(instruction @ Instruction::PushInt(_), ()) => {
                Some(format!("{instruction:?}"))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn definitions_are_numbered_in_the_order_dreammaker_gives_them() {
    let cases = [
        (
            "override-first",
            "/datum/p/f()\n\treturn 1\n/datum/p/proc/f()\n\treturn 2\n",
            "/datum/p/f",
            1,
            Some("/datum/p/proc/f"),
        ),
        (
            "after-a-declared-proc",
            "/datum/p/proc/g()\n/datum/p/f()\n\treturn 1\n/datum/p/proc/f()\n\treturn 2\n",
            "/datum/p/f",
            0,
            Some("/datum/p/f"),
        ),
        (
            "between-overrides",
            "/datum/p/f()\n\treturn 1\n/datum/p/proc/g()\n/datum/p/f()\n\treturn 3\n/datum/p/proc/f()\n\treturn 2\n",
            "/datum/p/f",
            1,
            None,
        ),
        (
            "after-a-declared-verb",
            "/datum/p/verb/g()\n/datum/p/f()\n\treturn 1\n/datum/p/proc/f()\n\treturn 2\n",
            "/datum/p/f",
            1,
            Some("/datum/p/proc/f"),
        ),
        (
            "verb-after-a-declared-verb",
            "/datum/p/verb/g()\n/datum/p/f()\n\treturn 1\n/datum/p/verb/f()\n\treturn 2\n",
            "/datum/p/f",
            0,
            Some("/datum/p/f"),
        ),
        (
            "after-a-parent-proc",
            "/datum/proc/g()\n/datum/p/f()\n\treturn 1\n/datum/p/proc/f()\n\treturn 2\n",
            "/datum/p/f",
            1,
            Some("/datum/p/proc/f"),
        ),
        (
            "global",
            "/g()\n\treturn 1\n/proc/g()\n\treturn 2\n",
            "/g",
            1,
            None,
        ),
        (
            "type-with-procs-in-stddef",
            "/icon/f()\n\treturn 1\n/icon/proc/f()\n\treturn 2\n",
            "/icon/f",
            0,
            Some("/icon/f"),
        ),
        (
            "type-without-procs-in-stddef",
            "/mob/f()\n\treturn 1\n/mob/proc/f()\n\treturn 2\n",
            "/mob/f",
            1,
            Some("/mob/proc/f"),
        ),
    ];
    for (name, code, path, declaration, target) in cases {
        let changes = changes_of(name, code, &code.replace("return 2", "return 5"));
        assert_eq!(reloaded(&changes), [key(path, declaration)], "{name}");
        let Some(target) = target else { continue };
        let ty = &path[..path.rfind('/').unwrap()];
        let after = format!(
            "{}/proc/calls()\n\treturn {}::f()\n",
            code.replace("return 2", "return 5"),
            ty
        );
        let changes = changes_of(name, code, &after);
        let calls = changes
            .plan
            .new_procs
            .iter()
            .find(|p| p.code.proc == key("/calls", 0))
            .unwrap_or_else(|| panic!("{}: no new /calls in {:?}", name, changes.refused));
        let code = dmasm_bytecode::format(&calls.code.nodes);
        assert!(
            code.contains(&format!("PushVal {}\n", target)),
            "{}: expected PushVal {} in\n{}",
            name,
            target,
            code
        );
    }
}

#[test]
fn an_override_in_an_earlier_file_comes_before_the_declaration() {
    let world = World::new(
        "override-in-earlier-file",
        "/datum/p/proc/f()\n\treturn 2\n",
    );
    std::fs::write(world.dir.join("code/a.dm"), "/datum/p/f()\n\treturn 1\n").unwrap();
    std::fs::write(
        world.dir.join("world.dme"),
        "#include \"code/a.dm\"\n#include \"code/code.dm\"\n",
    )
    .unwrap();
    let old = world.parse();
    world.write("/datum/p/proc/f()\n\treturn 5\n");
    let new = world.parse();
    let edited: HashSet<PathBuf> = vec![PathBuf::from("code/code.dm")].into_iter().collect();
    assert_eq!(
        reloaded(&diff(&old, &new, Some(&edited))),
        [key("/datum/p/f", 1)]
    );
}

#[test]
fn an_override_and_its_later_declaration_are_added_in_order() {
    let cases = [
        ("/datum/p\n", "/datum/p/f", "/datum/p/", false),
        ("var/x = 1\n", "/g", "/", true),
    ];
    for (before, path, prefix, global) in cases {
        let name = &path[path.rfind('/').unwrap() + 1..];
        let after =
            format!("{before}{prefix}{name}()\n\treturn 1\n{prefix}proc/{name}()\n\treturn 2\n");
        let changes = changes("added-override-first", before, &after);
        assert!(changes.refused.is_empty(), "{:?}", changes.refused);
        let added: Vec<(ProcKey, bool, Vec<String>)> = changes
            .plan
            .new_procs
            .iter()
            .map(|p| (p.code.proc.clone(), p.declared, pushed_ints(&p.code.nodes)))
            .collect();
        assert_eq!(
            added,
            [
                (key(path, 0), false, vec!["PushInt(1)".to_owned()]),
                (key(path, 1), true, vec!["PushInt(2)".to_owned()])
            ],
            "{path}"
        );
        let unreachable = texts(&changes.warnings)
            .iter()
            .filter(|w| w.contains("unreachable"))
            .count();
        assert_eq!(
            unreachable, global as usize,
            "{path}: {:?}",
            changes.warnings
        );
    }

    let changes = changes(
        "added-override-before-declaration",
        "/datum/p/proc/f()\n\treturn 2\n",
        "/datum/p/f()\n\treturn 1\n/datum/p/proc/f()\n\treturn 2\n",
    );
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let added: Vec<(ProcKey, bool)> = changes
        .plan
        .new_procs
        .iter()
        .map(|p| (p.code.proc.clone(), p.declared))
        .collect();
    assert_eq!(added, [(key("/datum/p/f", 0), false)]);
    assert!(reloaded(&changes).is_empty(), "{:?}", reloaded(&changes));
}

#[test]
fn an_override_added_before_an_edited_declaration_is_the_new_proc() {
    let changes = changes(
        "added-override-before-edited-declaration",
        "/datum/p/proc/f()\n\treturn 2\n",
        "/datum/p/f()\n\treturn 1\n/datum/p/proc/f()\n\treturn 3\n",
    );
    let added: Vec<(ProcKey, bool, Vec<String>)> = changes
        .plan
        .new_procs
        .iter()
        .map(|p| (p.code.proc.clone(), p.declared, pushed_ints(&p.code.nodes)))
        .collect();
    assert_eq!(
        added,
        [(key("/datum/p/f", 0), false, vec!["PushInt(1)".to_owned()])]
    );
    let reloads: Vec<(ProcKey, Vec<String>)> = changes
        .plan
        .reloads
        .iter()
        .map(|r| (r.proc.clone(), pushed_ints(&r.nodes)))
        .collect();
    assert_eq!(
        reloads,
        [(key("/datum/p/f", 0), vec!["PushInt(3)".to_owned()])]
    );
}

#[test]
fn a_declaration_placed_past_an_override_is_refused() {
    let before = "/datum/p/f()\n\treturn 1\n/datum/p/proc/f()\n\treturn 2\n";
    let why = "/datum/p/f: the order of its definitions changed";
    for (name, after) in [
        ("declaration-before", format!("/datum/p/proc/g()\n{before}")),
        (
            "declaration-and-override-before",
            format!("/datum/p/proc/g()\n{before}/datum/p/f()\n\treturn 3\n"),
        ),
    ] {
        let refused = refusal(name, before, &after);
        assert!(refused.contains(why), "{}: {}", name, refused);
    }

    let before = format!("{before}/datum/p/proc/g()\n");
    let after = format!(
        "/datum/p/proc/g()\n{}",
        before.replace("/datum/p/proc/g()\n", "")
    );
    let world = World::new("declaration-moved", &before);
    let mut old = world.parse();
    world.write(&after);
    match old.reload_file(&world.dir, std::path::Path::new("code/code.dm")) {
        Ok(_) => panic!("reloaded on its own"),
        Err(e) => assert!(e.contains(why), "{}", e),
    }
    let edited: HashSet<PathBuf> = vec![PathBuf::from("code/code.dm")].into_iter().collect();
    let refused = shown(&diff(&old, &world.parse(), Some(&edited)).refused).join("\n");
    assert!(refused.contains(why), "{}", refused);
}

#[test]
fn in_lists_are_the_words_dreammaker_writes() {
    let cases = [
        ("mob/a in view()", Some(0x7D01)),
        ("mob/a in view(3)", Some(0x0301)),
        ("mob/a in oview()", Some(0x7D02)),
        ("mob/a in oview(1, usr)", Some(0x0102)),
        ("mob/a in range()", Some(0x7D05)),
        ("mob/a in range(2)", Some(0x0205)),
        ("mob/a in orange()", Some(0x7D06)),
        ("mob/a in orange(2)", Some(0x0206)),
        ("mob/a in world", Some(0x7F10)),
        ("obj/a in usr", Some(0x7F08)),
        ("obj/a in usr.loc", Some(0x7F03)),
        ("obj/a in usr.contents", Some(0x7F08)),
        ("mob/a in view(usr)", Some(0x7D01)),
        ("a", Some(0x7D01)),
        ("obj/a in src", None),
        ("mob/a in view(src)", None),
        ("a in list(1, 2)", None),
    ];
    let code: String = cases
        .iter()
        .enumerate()
        .map(|(i, (param, _))| format!("/mob/proc/p{i}({param})\n\treturn\n"))
        .collect();
    let world = World::new("in-lists", &code);
    let snapshot = world.parse();
    for (i, (param, word)) in cases.iter().enumerate() {
        let reload = snapshot
            .compile_reload(&key(&format!("/mob/p{i}"), 0))
            .unwrap();
        let expected = match word {
            Some(word) => InList::Builtin(*word),
            None => InList::Kept,
        };
        assert_eq!(reload.params[0].in_list, expected, "{}", param);
    }
}

#[test]
fn in_list_expressions_are_kept_or_compiled() {
    let before = "/mob/proc/f(obj/a in src.contents, b)\n\treturn 1\n";
    let changes = changes(
        "in-list-kept",
        before,
        "/mob/proc/f(obj/a in src.contents, b, c)\n\treturn 2\n",
    );
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert_eq!(changes.plan.reloads[0].params[0].in_list, InList::Kept);

    let changes = changes_of(
        "in-list-changed",
        before,
        "/mob/proc/f(obj/a in src.loc, b)\n\treturn 1\n/mob/proc/g(n in list(1, 2))\n\treturn n\n",
    );
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let InList::New(code) = &changes.plan.reloads[0].params[0].in_list else {
        panic!("{:?}", changes.plan.reloads[0].params[0].in_list);
    };
    assert!(
        dmasm_bytecode::format(&code.nodes).contains("cache = src; cache[\"loc\"]"),
        "{}",
        dmasm_bytecode::format(&code.nodes)
    );
    let InList::New(code) = &changes.plan.new_procs[0].code.params[0].in_list else {
        panic!("{:?}", changes.plan.new_procs[0].code.params[0].in_list);
    };
    assert!(
        dmasm_bytecode::format(&code.nodes).contains("NewList 2"),
        "{}",
        dmasm_bytecode::format(&code.nodes)
    );
    let text = dmasm_bytecode::format(&code.nodes);
    assert!(
        text.starts_with("DbgFile") && text.contains("DbgLine 3"),
        "{}",
        text
    );
}

#[test]
fn defaults_that_arent_constants_change_the_init_procs() {
    let before = "/datum/a\n\tvar/list/l = list(1)\n\tvar/n = 2\n/datum/a/b\n/datum/a/c\n\tl = list(3)\n/datum/a/d\n\tvar/list/m = list()\n/datum/e\n";
    let after = before.replace("list(1)", "list(1, 2)");
    let first = changes("init-procs", before, &after);
    assert!(first.refused.is_empty(), "{:?}", first.refused);
    let mut types: Vec<&str> = first
        .plan
        .init_procs
        .iter()
        .map(|i| i.type_path.as_str())
        .collect();
    types.sort();
    assert_eq!(types, ["/datum/a", "/datum/a/b", "/datum/a/d"]);
    assert!(first.plan.init_procs.iter().all(|i| i.code.is_some()));
    assert!(first.plan.var_defaults.is_empty());

    let after = before
        .replace("\tvar/n = 2\n", "\tvar/n = 2\n\tvar/list/k = list(\"k\")\n")
        .replace("l = list(3)", "l = null");
    let second = changes("init-procs-2", before, &after);
    assert!(second.refused.is_empty(), "{:?}", second.refused);
    assert_eq!(second.plan.new_vars.len(), 1);
    assert_eq!(second.plan.new_vars[0].value, ConstValue::Null);
    let mut types: Vec<&str> = second
        .plan
        .init_procs
        .iter()
        .map(|i| i.type_path.as_str())
        .collect();
    types.sort();
    assert_eq!(
        types,
        ["/datum/a", "/datum/a/b", "/datum/a/c", "/datum/a/d"]
    );
    assert_eq!(
        second.plan.var_defaults.len(),
        1,
        "{:?}",
        second.plan.var_defaults
    );
    assert_eq!(second.plan.var_defaults[0].types, ["/datum/a/c"]);
    assert!(
        texts(&second.warnings)
            .iter()
            .any(|w| w.contains("new var k has a computed default")
                && w.contains("initial() remain")),
        "{:?}",
        second.warnings
    );
}

#[test]
fn new_types_with_their_vars_and_procs() {
    let before = "/obj/thing\n\tvar/a = 1\n/obj/thing/proc/f()\n\treturn 1\n/obj/named\n\tname = \"Named\"\n";
    let after = format!(
        "{before}/obj/thing/my_new\n\tvar/b = 2\n\tvar/list/c = list(3)\n\ta = 5\n\ticon_state = \"s\"\n\tdensity = 1\n\
         /obj/thing/my_new/f()\n\treturn 2\n/obj/thing/my_new/proc/g()\n\treturn 3\n/obj/thing/my_new/deeper\n/obj/named/kid\n/datum/d\n"
    );
    let changes = changes("new-types", before, &after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    type Fields = Vec<(String, ConstValue)>;
    let types: Vec<(&str, &str, Fields)> = changes
        .plan
        .new_types
        .iter()
        .map(|t| {
            (t.path.as_str(), t.parent.as_str(), {
                let mut f = t.fields.clone();
                f.sort_by(|a, b| a.0.cmp(&b.0));
                f
            })
        })
        .collect();
    let s = |v: &str| ConstValue::String(v.to_owned());
    assert_eq!(
        types,
        [
            (
                "/datum/d",
                "/datum",
                vec![("name".to_owned(), s("d")), ("text".to_owned(), s("d"))]
            ),
            ("/obj/named/kid", "/obj/named", vec![]),
            (
                "/obj/thing/my_new",
                "/obj/thing",
                vec![
                    ("density".to_owned(), ConstValue::Number(1.0)),
                    ("icon_state".to_owned(), s("s")),
                    ("name".to_owned(), s("my new")),
                    ("text".to_owned(), s("m")),
                ]
            ),
            (
                "/obj/thing/my_new/deeper",
                "/obj/thing/my_new",
                vec![
                    ("name".to_owned(), s("deeper")),
                    ("text".to_owned(), s("d"))
                ]
            ),
        ]
    );
    let mut vars: Vec<&str> = changes
        .plan
        .new_vars
        .iter()
        .map(|v| v.var.as_str())
        .collect();
    vars.sort();
    assert_eq!(vars, ["b", "c"]);
    let mut defaults: Vec<(&str, Vec<String>)> = changes
        .plan
        .var_defaults
        .iter()
        .map(|d| (d.var.as_str(), d.types.clone()))
        .collect();
    defaults.sort();
    assert_eq!(
        defaults,
        [
            (
                "a",
                vec![
                    "/obj/thing/my_new".to_owned(),
                    "/obj/thing/my_new/deeper".to_owned()
                ]
            ),
            (
                "b",
                vec![
                    "/obj/thing/my_new".to_owned(),
                    "/obj/thing/my_new/deeper".to_owned()
                ]
            ),
        ]
    );
    let mut procs: Vec<(String, bool)> = changes
        .plan
        .new_procs
        .iter()
        .map(|p| (p.code.proc.path.clone(), p.declared))
        .collect();
    procs.sort();
    assert_eq!(
        procs,
        [
            ("/obj/thing/my_new/f".to_owned(), false),
            ("/obj/thing/my_new/g".to_owned(), true)
        ]
    );
    let mut inits: Vec<&str> = changes
        .plan
        .init_procs
        .iter()
        .map(|i| i.type_path.as_str())
        .collect();
    inits.sort();
    assert_eq!(inits, ["/obj/thing/my_new", "/obj/thing/my_new/deeper"]);
}

#[test]
fn globals_and_statics_that_arent_constants_come_with_code_to_run_once() {
    let before = "/datum/a\n/datum/a/proc/f()\n\treturn 1\n";
    let after = "var/list/g = list(1, 2)\n/datum/a\n\tvar/static/list/s = list(3)\n/datum/a/proc/f()\n\tvar/static/datum/a/l = new\n\treturn l\n";
    let changes = changes("run-once", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    for var in &changes.plan.new_vars {
        assert_eq!(var.value, ConstValue::Null);
        let init = var
            .init
            .as_ref()
            .unwrap_or_else(|| panic!("no init for {}", var.var));
        assert!(init
            .nodes
            .iter()
            .any(|n| matches!(n, Node::Instruction(Instruction::Ret, _))));
    }
    assert_eq!(changes.plan.new_vars.len(), 2);
    let statics = &changes.plan.reloads[0].statics;
    assert_eq!(statics.len(), 1);
    assert!(statics[0].value.is_none() && statics[0].init.is_some());
}

#[test]
fn verb_settings_are_the_words_dreammaker_writes() {
    let cases: &[(&str, u32, u32)] = &[
        ("", 0x4, 0xFF),
        ("set src = usr", 0x6, 0x20FF),
        ("set src = view()", 0x6, 0x017D),
        ("set src in oview(1)", 0x4, 0x0201),
        ("set src in world", 0x4, 0x107F),
        ("set src = usr.loc", 0x6, 0x03FF),
        ("set src in usr.contents", 0x4, 0x087F),
        ("set invisibility = 0", 0x8C, 0xFF00FF),
        ("set invisibility = 1", 0x9C, 0x0100FF),
        ("set invisibility = 101", 0x9C, 0x6500FF),
        ("set popup_menu = 0", 0x44, 0xFF),
        ("set instant = 1", 0x284, 0xFF),
        ("set hidden = 1\n\tset popup_menu = 0", 0x45, 0xFF),
        ("set background = 1", 0x184, 0xFF),
        ("set waitfor = 0", 0x0, 0xFF),
        ("set src = usr.contents", 0x6, 0x087F),
        ("set src in usr.loc", 0x4, 0x037F),
        ("set src in view(usr, 1)", 0x4, 0x0101),
        ("set src in view(0)", 0x4, 0x0100),
        ("set category = null", 0x24, 0xFF),
    ];
    let code: String = cases
        .iter()
        .enumerate()
        .map(|(i, (set, _, _))| format!("/mob/verb/v{i}()\n\t{set}\n\treturn\n"))
        .collect();
    let world = World::new("verb-settings", &format!("{code}/mob/verb/named()\n\tset name = \"Named Verb\"\n\tset category = \"Cat\"\n\tset desc = \"a desc\"\n\treturn\n"));
    let snapshot = world.parse();
    for (i, (set, flags, packed)) in cases.iter().enumerate() {
        let settings = snapshot.settings(&key(&format!("/mob/v{i}"), 0)).unwrap();
        assert_eq!(
            (settings.flags(), settings.packed()),
            (*flags, *packed),
            "{}",
            set
        );
    }
    let named = snapshot.settings(&key("/mob/named", 0)).unwrap();
    assert_eq!(
        (
            named.name.as_deref(),
            named.desc.as_deref(),
            named.category.as_deref()
        ),
        (Some("Named Verb"), Some("a desc"), Some("Cat"))
    );
}

#[test]
fn new_verbs_and_changed_settings() {
    let before = "/mob/sub\n/mob/verb/a()\n\treturn\n/mob/proc/b()\n\tset name = \"B\"\n\treturn\n";
    let after = "/mob/sub\n/mob/verb/a()\n\treturn\n/mob/proc/b()\n\tset name = \"Bee\"\n\tset category = \"Admin\"\n\treturn\n/mob/verb/c()\n\treturn\n/mob/sub/a()\n\treturn ..()\n";
    let changes = changes("new-verbs", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let mut added: Vec<(String, bool, bool)> = changes
        .plan
        .new_procs
        .iter()
        .map(|p| (p.code.proc.path.clone(), p.declared, p.verb))
        .collect();
    added.sort();
    assert_eq!(
        added,
        [
            ("/mob/c".to_owned(), true, true),
            ("/mob/sub/a".to_owned(), false, true)
        ]
    );
    assert_eq!(changes.plan.reloads.len(), 1);
    assert_eq!(
        changes.plan.reloads[0]
            .settings
            .as_ref()
            .unwrap()
            .name
            .as_deref(),
        Some("Bee")
    );
}

#[test]
fn settings_come_from_the_declaration() {
    let code = |set: &str| {
        format!("/mob/verb/a()\n\tset name = \"A \\\"verb\\\"\"\n\tset category = \"Cat\"\n\t{set}\n\treturn\n/mob/sub/a()\n\tset background = 1\n\treturn\n/mob/sub/sub/a()\n\treturn\n")
    };
    let world = World::new("settings-inherited", &code("set waitfor = 0"));
    let snapshot = world.parse();
    let settings = |path: &str| snapshot.settings(&key(path, 0)).unwrap();
    assert_eq!(settings("/mob/a").name.as_deref(), Some("A \"verb\""));
    let sub = settings("/mob/sub/a");
    assert_eq!(
        (
            sub.name.as_deref(),
            sub.category.as_deref(),
            sub.waitfor,
            sub.background
        ),
        (Some("A \"verb\""), Some("Cat"), false, true)
    );
    let subsub = settings("/mob/sub/sub/a");
    assert_eq!((subsub.waitfor, subsub.background), (false, false));

    let changes = changes(
        "settings-inherited",
        &code("set waitfor = 0"),
        &code("set waitfor = 1"),
    );
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert_eq!(reloaded(&changes), [key("/mob/a", 0)]);
    assert!(changes.plan.reloads[0].settings.as_ref().unwrap().waitfor);
    let mut changed: Vec<(String, bool)> = changes
        .plan
        .settings
        .iter()
        .map(|c| (c.proc.path.clone(), c.settings.waitfor))
        .collect();
    changed.sort();
    assert_eq!(
        changed,
        [
            ("/mob/sub/a".to_owned(), true),
            ("/mob/sub/sub/a".to_owned(), true)
        ]
    );

    let changes = changes_of(
        "settings-inherited",
        &code(""),
        &code("").replace("/mob/sub/sub/a()\n\treturn", "/mob/sub/sub/a()\n\treturn 1"),
    );
    assert_eq!(reloaded(&changes), [key("/mob/sub/sub/a", 0)]);
    assert_eq!(changes.plan.reloads[0].settings, None);
}

#[test]
fn one_file_parsed_again_on_its_own() {
    let before = "#define TWO 2\n/datum/a\n\tvar/x = 1\n/datum/a/proc/f()\n\treturn TWO\n\n/datum/a/proc/g()\n\treturn 3\n";
    let world = World::new("one-file", before);
    let file = std::path::Path::new("code/code.dm");
    let edited: HashSet<PathBuf> = vec![PathBuf::from("code/code.dm")].into_iter().collect();
    let mut old = world.parse();

    let after = before.replace("\treturn TWO\n", "\tvar/y = TWO\n\treturn y\n");
    world.write(&after);
    let reload = old.reload_file(&world.dir, file).unwrap();
    let full = diff(&old, &world.parse(), Some(&edited));
    assert!(
        reload.changes.refused.is_empty(),
        "{:?}",
        reload.changes.refused
    );
    assert_eq!(reloaded(&reload.changes), [key("/datum/a/f", 0)]);
    assert_eq!(
        reload.changes.plan.reloads[0].nodes,
        full.plan.reloads[0].nodes
    );
    assert_eq!(reload.changes.plan.line_moves, full.plan.line_moves);
    assert_eq!(reload.changes.plan.line_moves[0].proc, key("/datum/a/g", 0));
    old.keep(reload);

    let again = after.replace("return 3", "return 4");
    world.write(&again);
    let reload = old.reload_file(&world.dir, file).unwrap();
    assert_eq!(reloaded(&reload.changes), [key("/datum/a/g", 0)]);
    assert!(reload.changes.plan.line_moves.is_empty());
    old.keep(reload);

    for (name, code, why) in [
        ("var", again.replace("var/x = 1", "var/x = 2"), "vars"),
        (
            "macro",
            again.replace("#define TWO 2", "#define TWO 3"),
            "macros",
        ),
        (
            "proc",
            format!("{}/datum/a/proc/h()\n\treturn 5\n", again),
            "declarations",
        ),
        (
            "override",
            format!("{}/datum/a/f()\n\treturn 5\n", again),
            "added",
        ),
        (
            "verb",
            again.replace("/datum/a/proc/g()", "/datum/a/verb/g()"),
            "declarations",
        ),
        (
            "final",
            again.replace("/datum/a/proc/g()", "/datum/a/proc/final/g()"),
            "declarations",
        ),
        ("type", format!("{}/datum/b\n", again), "new type"),
        ("parameters", again.replace("g()", "g(z)"), "parameters"),
        (
            "setting",
            again.replace("\treturn 4", "\tset waitfor = FALSE\n\treturn 4"),
            "`set`s",
        ),
        (
            "static",
            again.replace("\treturn 4", "\tvar/static/s = 1\n\treturn 4"),
            "static initializers",
        ),
    ] {
        world.write(&code);
        match old.reload_file(&world.dir, file) {
            Ok(_) => panic!("{}: reloaded on its own", name),
            Err(e) => assert!(e.contains(why), "{}: {}", name, e),
        }
    }
}

#[test]
fn a_changed_default_recompiles_type_scope_users() {
    let before = "/datum/thing\n\tvar/size = 2\n/datum/thing/big\n/proc/a()\n\treturn /datum/thing/big::size\n/proc/b()\n\treturn 2\n";
    let after = before.replace("/datum/thing/big\n", "/datum/thing/big\n\tsize = 9\n");
    let changes = changes("type-scope-default", before, &after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert_eq!(reloaded(&changes), [key("/a", 0)]);
}

#[test]
fn a_changed_default_is_found_wherever_a_proc_reads_it_with_scope() {
    let before = "/datum/thing
	var/size = 2
/datum/thing/big
/proc/in_default(x = /datum/thing/big::size)
	return x
/proc/in_argument()
	return max(1, /datum/thing/big::size)
/proc/in_block(x)
	if(x)
		for(var/i in 1 to 2)
			return list(i, /datum/thing/big::size)
/proc/in_text()
	return \"is [/datum/thing/big::size]\"
/proc/named_alike(datum/thing/big/b)
	var/size = b.size
	return size
";
    let after = before.replace("/datum/thing/big\n", "/datum/thing/big\n\tsize = 9\n");
    let changes = changes("type-scope-places", before, &after);
    let mut procs: Vec<String> = reloaded(&changes).iter().map(|k| k.to_string()).collect();
    procs.sort();
    assert_eq!(
        procs,
        ["/in_argument", "/in_block", "/in_default", "/in_text"]
    );
}

#[test]
fn a_changed_default_recompiles_a_proc_that_reads_it_through_a_macro() {
    let before = "#define DEFAULT_OF(T, V) T::V\n/datum/thing\n\tvar/size = 2\n/datum/thing/big\n/proc/a()\n\treturn DEFAULT_OF(/datum/thing/big, size)\n/proc/b()\n\treturn 2\n";
    let after = before.replace("/datum/thing/big\n", "/datum/thing/big\n\tsize = 9\n");
    let changes = changes("type-scope-macro", before, &after);
    assert_eq!(reloaded(&changes), [key("/a", 0)]);
    let before = "#define DEFAULT_OF(T, V) T::V\n/datum/a\n\tvar/size = 1\n/datum/b\n\tvar/size = 2\n/datum/a/c\n/mob/proc/sized()\n\treturn DEFAULT_OF(/datum/a/c, size)\n";
    let after = before.replace("/datum/a/c\n", "/datum/a/c\n\tparent_type = /datum/b\n");
    let changes = changes_of("parent-type-macro", before, &after);
    assert_eq!(
        changes.plan.reparented.len(),
        1,
        "{:?}",
        changes.plan.reparented
    );
    assert_eq!(reloaded(&changes), [key("/mob/sized", 0)]);
}

#[test]
fn a_new_parent_type_is_a_reparent_and_nothing_else() {
    let before = "/datum/a\n/datum/b\n/datum/a/c\n\tvar/x = 1\n";
    let after = "/datum/a\n/datum/b\n/datum/a/c\n\tparent_type = /datum/b\n\tvar/x = 1\n";
    let changes = changes("parent-type", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert_eq!(
        changes.plan.reparented.len(),
        1,
        "{:?}",
        changes.plan.reparented
    );
    assert!(
        changes.plan.var_defaults.is_empty() && changes.plan.builtin_defaults.is_empty(),
        "{:?}",
        changes
    );
    assert!(
        changes.plan.redeclared.is_empty(),
        "{:?}",
        changes.plan.redeclared
    );
}

#[test]
fn a_new_parent_type_recompiles_other_procs_that_hold_what_it_inherits() {
    let before = "/datum/a\n\tvar/size = 1\n\tvar/const/limit = 3\n\
                  /datum/b\n\tvar/size = 2\n\tvar/const/limit = 7\n\
                  /datum/a/c\n\
                  /mob/proc/sized()\n\treturn /datum/a/c::size\n\
                  /mob/proc/capped(datum/a/c/thing)\n\treturn thing.limit\n\
                  /mob/proc/other(datum/a/thing)\n\treturn thing.limit + /datum/a::size\n";
    let after = before.replace("/datum/a/c\n", "/datum/a/c\n\tparent_type = /datum/b\n");
    let changes = changes("parent-type-users", before, &after);
    assert_eq!(
        changes.plan.reparented.len(),
        1,
        "{:?}",
        changes.plan.reparented
    );
    let mut procs = reloaded(&changes);
    procs.sort();
    assert_eq!(procs, [key("/mob/capped", 0), key("/mob/sized", 0)]);
}

#[test]
fn what_a_new_parent_changes_is_found_wherever_a_proc_names_it() {
    let before = "/datum/x
	var/const/n = 1
/datum/y
	var/const/n = 2
/datum/a
	var/size = 1
	var/const/limit = 3
	var/datum/x/held
/datum/b
	var/size = 2
	var/const/limit = 7
	var/datum/y/held
/datum/a/c
/mob/proc/in_default(x = /datum/a/c::size)
	return x
/mob/proc/in_argument(datum/a/c/thing)
	return max(thing.limit, 1)
/mob/proc/in_block(datum/a/c/thing, x)
	if(x)
		return list(thing.limit, /datum/a/c::size)
/mob/proc/through_its_type(datum/a/c/thing)
	return thing.held.n
/mob/proc/other(datum/a/thing)
	return thing.limit + /datum/a::size + thing.held.n
";
    let after = before.replace("/datum/a/c\n", "/datum/a/c\n\tparent_type = /datum/b\n");
    let changes = changes("parent-type-places", before, &after);
    assert_eq!(
        changes.plan.reparented.len(),
        1,
        "{:?}",
        changes.plan.reparented
    );
    let mut procs: Vec<String> = reloaded(&changes).iter().map(|k| k.to_string()).collect();
    procs.sort();
    assert_eq!(
        procs,
        [
            "/mob/in_argument",
            "/mob/in_block",
            "/mob/in_default",
            "/mob/through_its_type"
        ]
    );
}

#[test]
fn a_new_parent_with_the_same_default_written_elsewhere_names_its_file() {
    let changes = types_changes(
        "parent-type-same-default",
        "/datum/a\n\tvar/x = 5\n/proc/f()\n\treturn /datum/a/c::x\n/proc/g()\n\treturn 5\n",
        "/datum/b\n\tvar/x = 5\n/datum/a/c\n",
        "/datum/b\n\tvar/x = 5\n/datum/a/c\n\tparent_type = /datum/b\n",
    );
    assert_eq!(
        changes.plan.reparented.len(),
        1,
        "{:?}",
        changes.plan.reparented
    );
    assert_eq!(reloaded(&changes), [key("/f", 0)]);
}

#[test]
fn a_removed_name_goes_back_to_each_types_own() {
    let before =
        "/obj/thing\n\tname = \"box\"\n/obj/thing/big_one\n/obj/thing/small\n\ttext = \"s\"\n";
    let after = "/obj/thing\n/obj/thing/big_one\n/obj/thing/small\n\ttext = \"s\"\n";
    let changes = changes("removed-name", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let mut set: Vec<(String, String, ConstValue)> = changes
        .plan
        .builtin_defaults
        .iter()
        .flat_map(|d| {
            d.types
                .iter()
                .map(move |t| (t.clone(), d.var.clone(), d.value.clone()))
        })
        .collect();
    set.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    let s = |t: &str| ConstValue::String(t.to_owned());
    assert_eq!(
        set,
        [
            ("/obj/thing".to_owned(), "name".to_owned(), s("thing")),
            ("/obj/thing".to_owned(), "text".to_owned(), s("t")),
            (
                "/obj/thing/big_one".to_owned(),
                "name".to_owned(),
                s("big one")
            ),
            ("/obj/thing/big_one".to_owned(), "text".to_owned(), s("b")),
            ("/obj/thing/small".to_owned(), "name".to_owned(), s("small")),
        ]
    );
}

fn types_changes(name: &str, code: &str, before: &str, after: &str) -> Changes {
    let world = World::new(name, &format!("{code}#include \"types.dm\"\n"));
    std::fs::write(world.dir.join("code/types.dm"), before).unwrap();
    let old = world.parse();
    std::fs::write(world.dir.join("code/types.dm"), after).unwrap();
    let new = world.parse();
    let edited: HashSet<PathBuf> = vec![PathBuf::from("code/types.dm")].into_iter().collect();
    diff(&old, &new, Some(&edited))
}

#[test]
fn a_type_scope_default_that_moves_moves_its_users_lines() {
    let code = "\n\n\n/proc/f()\n\treturn /datum/u::x\n";
    let changes = types_changes(
        "moved-default",
        code,
        "/datum/u\n\tvar/x = 3\n",
        "\n/datum/u\n\tvar/x = 3\n",
    );
    assert!(changes.plan.reloads.is_empty(), "{:?}", reloaded(&changes));
    assert_eq!(
        changes.plan.line_moves.len(),
        1,
        "{:?}",
        changes.plan.line_moves
    );
    assert_eq!(changes.plan.line_moves[0].proc, key("/f", 0));
    assert_eq!(changes.plan.line_moves[0].lines, [(2, 3), (5, 5)]);
}

#[test]
fn a_type_scope_default_whose_lines_clash_is_compiled_again() {
    let changes = types_changes(
        "clashing-default",
        "/proc/f()\n\treturn /datum/u::x\n",
        "/datum/u\n\tvar/x = 3\n",
        "\n/datum/u\n\tvar/x = 3\n",
    );
    assert_eq!(reloaded(&changes), [key("/f", 0)]);
    assert!(
        changes.plan.line_moves.is_empty(),
        "{:?}",
        changes.plan.line_moves
    );
}

#[test]
fn a_type_scope_default_moved_to_another_file_is_compiled_again() {
    let world = World::new(
        "default-to-code",
        "#include \"types.dm\"\n/proc/f()\n\treturn /datum/u::x\n",
    );
    std::fs::write(world.dir.join("code/types.dm"), "/datum/u\n\tvar/x = 3\n").unwrap();
    let old = world.parse();
    std::fs::write(world.dir.join("code/types.dm"), "").unwrap();
    world.write("#include \"types.dm\"\n/proc/f()\n\treturn /datum/u::x\n/datum/u\n\tvar/x = 3\n");
    let new = world.parse();
    let changes = diff(&old, &new, None);
    assert_eq!(reloaded(&changes), [key("/f", 0)]);
}

#[test]
fn a_moved_default_read_with_scope_needs_the_full_read() {
    let world = World::new(
        "moved-default-one-file",
        "/proc/g()\n\treturn 1\n/datum/u\n\tvar/x = 3\n/datum/w\n\tvar/name = \"w\"\n/proc/f()\n\treturn /datum/u::x\n",
    );
    let file = std::path::Path::new("code/code.dm");
    let mut old = world.parse();
    world.write("/proc/g()\n\tworld.log << 0\n\treturn 1\n/datum/u\n\tvar/x = 3\n/datum/w\n\tvar/name = \"w\"\n/proc/f()\n\treturn /datum/u::x\n");
    match old.reload_file(&world.dir, file) {
        Ok(_) => panic!("reloaded on its own"),
        Err(e) => assert!(
            e.contains("/f reads a var default with :: that moved"),
            "{e}"
        ),
    }
    let world = World::new(
        "moved-default-macro",
        "#define DEFAULT_OF(T, V) T::V\n/proc/g()\n\treturn 1\n/datum/u\n\tvar/x = 3\n/proc/f()\n\treturn DEFAULT_OF(/datum/u, x)\n",
    );
    let mut old = world.parse();
    world.write("#define DEFAULT_OF(T, V) T::V\n/proc/g()\n\tworld.log << 0\n\treturn 1\n/datum/u\n\tvar/x = 3\n/proc/f()\n\treturn DEFAULT_OF(/datum/u, x)\n");
    match old.reload_file(&world.dir, file) {
        Ok(_) => panic!("reloaded on its own"),
        Err(e) => assert!(
            e.contains("/f reads a var default with :: that moved"),
            "{}",
            e
        ),
    }
    let world = World::new(
        "moved-unread-default",
        "/proc/g()\n\treturn 1\n/datum/w\n\tvar/name = \"w\"\n/proc/f(obj/o)\n\treturn o::name\n",
    );
    let mut old = world.parse();
    world.write("/proc/g()\n\tworld.log << 0\n\treturn 1\n/datum/w\n\tvar/name = \"w\"\n/proc/f(obj/o)\n\treturn o::name\n");
    let reload = old.reload_file(&world.dir, file).unwrap();
    assert_eq!(reloaded(&reload.changes), [key("/g", 0)]);
}

#[test]
fn a_constant_beyond_32_bits_at_a_bit_operator_is_warned_about() {
    let changes = changes(
        "big-bits",
        "/proc/f(x)\n\treturn x\n",
        "/proc/f(x)\n\treturn (-1e10) ^ x\n",
    );
    assert_eq!(reloaded(&changes), [key("/f", 0)]);
    assert!(
        texts(&changes.warnings)
            .iter()
            .any(|w| w.starts_with("/f: line 2: bit operand -10000000000")),
        "{:?}",
        changes.warnings
    );
    let changes = changes_of(
        "small-bits",
        "/proc/f(x)\n\treturn x\n",
        "/proc/f(x)\n\treturn (-1e6) ^ x\n",
    );
    assert!(changes.warnings.is_empty(), "{:?}", changes.warnings);
}

#[test]
fn a_static_local_s_initializer_names_its_own_proc() {
    let before = "/datum/a\n/datum/a/proc/f()\n\treturn 1\n";
    let after = "/proc/reg(p)\n\treturn p\n/datum/a\n/datum/a/proc/f()\n\tvar/static/r = reg(__PROC__)\n\treturn 1\n";
    let changes = changes("static-proc", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let reload = changes
        .plan
        .reloads
        .iter()
        .find(|r| r.proc.path == "/datum/a/proc/f" || r.proc.path == "/datum/a/f")
        .expect("f is reloaded");
    let init = reload.statics[0].init.as_ref().expect("code to run once");
    let pushed: Vec<String> = init
        .nodes
        .iter()
        .filter_map(|n| match n {
            Node::Instruction(Instruction::PushVal(dmasm_bytecode::Value::Path(p)), _) => {
                Some(p.clone())
            }
            _ => None,
        })
        .collect();
    assert!(
        pushed.iter().any(|p| p == "/datum/a/proc/f"),
        "__PROC__ gave {:?}",
        pushed
    );
}

#[test]
fn what_dreammaker_refuses_to_build_is_refused() {
    let before = "var/x = 1\n/datum/a\n/datum/a/b\n/datum/c\n\tvar/list/l = list(1)\n";
    for (name, after) in [(
        "dup-down",
        before
            .replace("/datum/a\n", "/datum/a\n\tvar/v = 1\n")
            .replace("/datum/a/b\n", "/datum/a/b\n\tvar/v = 2\n"),
    )] {
        let changes = changes(name, before, &after);
        assert!(
            changes
                .refused
                .iter()
                .any(|r| r.to_string().contains("v: duplicate definition")),
            "{:?}",
            changes.refused
        );
    }
    let obj = "/obj/item\n\tvar/foo = 1\n";
    let refused = changes(
        "dup-parents",
        obj,
        &format!("{obj}/atom/movable\n\tvar/foo = 2\n"),
    )
    .refused;
    assert!(
        refused.iter().any(|r| r
            .to_string()
            .contains("foo: duplicate definition (/obj/item declares it too)")),
        "{:?}",
        refused
    );
    let changes_ok = changes(
        "dup-global",
        before,
        &before.replace("/datum/a\n", "/datum/a\n\tvar/x = 2\n"),
    );
    assert!(changes_ok.refused.is_empty(), "{:?}", changes_ok.refused);
    let new_const = before.replace("/datum/a\n", "/datum/a\n\tvar/const/list/k = list(1, 2)\n");
    let refused = changes("const-new", before, &new_const).refused;
    assert!(
        refused
            .iter()
            .any(|r| r.to_string().contains("constant initializer required")),
        "{:?}",
        refused
    );
    let to_const = before.replace("var/list/l", "var/const/list/l");
    let refused = changes("const-to", before, &to_const).refused;
    assert!(
        refused
            .iter()
            .any(|r| r.to_string().contains("constant initializer required")),
        "{:?}",
        refused
    );
    let had_const = "var/const/g = 1\n/datum/a\n\tvar/const/k = 1\n";
    for (name, after) in [
        ("const-list", had_const.replace("k = 1", "k = list(1, 2)")),
        ("const-new", had_const.replace("k = 1", "k = new /datum")),
        ("const-global", had_const.replace("g = 1", "g = list(1, 2)")),
    ] {
        let refused = changes(name, had_const, &after).refused;
        assert!(
            refused
                .iter()
                .any(|r| r.file.as_deref() == Some("code/code.dm")
                    && r.line.is_some()
                    && r.to_string().contains("constant initializer required")),
            "{name}: {:?}",
            refused
        );
    }
    let had_const = format!("{had_const}\tvar/const/k2 = k + 1\n");
    for after in [
        had_const.replace("k = 1", "k = 2"),
        had_const.replace("k + 1", "k + 2"),
        had_const.replace("g = 1", "g = 2"),
    ] {
        let refused = changes("const-other", &had_const, &after).refused;
        assert!(refused.is_empty(), "{:?}", refused);
    }
    for value in [
        "matrix()",
        "icon('x.dmi')",
        "sound('x.ogg')",
        "file(\"x\")",
        "newlist(/datum)",
        "vector(1, 2)",
        "generator(\"num\", 0, 1)",
        "filter(type = \"blur\")",
    ] {
        let new_const = before.replace(
            "/datum/a\n",
            &format!("/datum/a\n\tvar/const/k = {value}\n"),
        );
        let refused = changes("const-call", before, &new_const).refused;
        assert!(
            refused
                .iter()
                .any(|r| r.to_string().contains("constant initializer required")),
            "{value}: {:?}",
            refused
        );
        let plain = before.replace("/datum/a\n", &format!("/datum/a\n\tvar/k = {value}\n"));
        let changes_ok = changes("plain-call", before, &plain);
        assert!(
            !changes_ok
                .refused
                .iter()
                .any(|r| r.to_string().contains("constant initializer")),
            "{value}: {:?}",
            changes_ok.refused
        );
    }
    let fine = before.replace(
        "/datum/a\n",
        "/datum/a\n\tvar/const/n = 1 << 3\n\tvar/const/t = \"text\"\n\tvar/const/p = /datum/c\n",
    );
    let changes_ok = changes("const-ok", before, &fine);
    assert!(changes_ok.refused.is_empty(), "{:?}", changes_ok.refused);
}

#[test]
fn consts_dreammaker_folds_are_taken_and_those_it_refuses_warned_about() {
    let before = "var/const/gconst = 3\n/datum/a\n/datum/c\n\tvar/const/c1 = 1\n";
    for value in [
        "sqrt(4)",
        "round(1.5)",
        "abs(-1)",
        "md5(\"x\")",
        "global.gconst",
        "1 in list(1)",
        "2 in list(1, \"a\" = 3)",
        "floor(sqrt(4)) * 2",
    ] {
        let after = before.replace(
            "/datum/a\n",
            &format!("/datum/a\n\tvar/const/k = {value}\n"),
        );
        let got = changes("const-folded", before, &after);
        assert!(got.refused.is_empty(), "{value}: {:?}", got.refused);
    }
    let after = before.replace(
        "/datum/a\n",
        "/datum/a\n\tvar/const/k = sqrt(16)\n\tvar/y = k + 1\n",
    );
    let got = changes("const-used", before, &after);
    assert!(got.refused.is_empty(), "{:?}", got.refused);
    let world = World::new(
        "const-values",
        &before.replace(
            "/datum/a\n",
            "/datum/a\n\tvar/const/k = sqrt(16)\n\tvar/const/k2 = k + 1\n\tvar/const/h = md5(\"x\")\n\tvar/const/i = 3 in list(1, 3)\n",
        ),
    );
    let snapshot = world.parse();
    let a = snapshot.tree().find("/datum/a").unwrap();
    let value = |name: &str| {
        a.get()
            .vars
            .get(name)
            .and_then(|v| v.value.constant.clone())
            .map(|c| c.to_string())
    };
    assert_eq!(value("k").as_deref(), Some("4"));
    assert_eq!(value("k2").as_deref(), Some("5"));
    assert_eq!(
        value("h").as_deref(),
        Some("\"9dd4e461268c8034f5c8564e155c67a6\"")
    );
    assert_eq!(value("i").as_deref(), Some("1"));
    let after = before.replace(
        "/datum/a\n",
        "/datum/a\n\tvar/const/k = sqrt(16)\n\tvar/const/k2 = k + 1\n",
    );
    let got = changes("const-chain", before, &after);
    assert!(got.refused.is_empty(), "{:?}", got.refused);
    for (value, why) in [
        ("type", "it reads type"),
        ("parent_type", "it reads parent_type"),
        ("\"a\" == \"a\"", "it compares"),
        ("1 < 2", "it compares"),
        ("(1 + (2 != 3))", "it compares"),
    ] {
        let after = before.replace(
            "/datum/a\n",
            &format!("/datum/a\n\tvar/const/k = {value}\n"),
        );
        let got = changes("const-refused", before, &after);
        assert!(got.refused.is_empty(), "{value}: {:?}", got.refused);
        assert!(
            texts(&got.warnings)
                .iter()
                .any(|w| w.contains("const k: DreamMaker refuses") && w.contains(why)),
            "{value}: {:?}",
            got.warnings
        );
    }
    let after = before.replace("/datum/a\n", "/datum/a\n\tvar/const/k = 1 + 2\n");
    let got = changes("const-plain", before, &after);
    assert!(
        !texts(&got.warnings)
            .iter()
            .any(|w| w.contains("DreamMaker refuses")),
        "{:?}",
        got.warnings
    );
    let after = before.replace("/datum/a\n", "/datum/a\n\tvar/const/k = min(1, 2)\n");
    let got = changes("const-unfolded", before, &after);
    assert!(
        !got.refused.is_empty(),
        "non-constant min() initializer was accepted"
    );
}

#[test]
fn refusals_are_placed_in_the_new_source() {
    let before =
        "/datum/a\n/datum/a/proc/f()\n\treturn 1\n/datum/b\n/obj/o\n\tparent_type = /obj\n";
    let placed = |name: &str, after: &str| -> Vec<(String, Option<u32>, String)> {
        changes(name, before, after)
            .refused
            .iter()
            .map(|r| (r.file.clone().unwrap_or_default(), r.line, format!("{r:#}")))
            .collect()
    };
    assert_eq!(
        placed(
            "placed-compile",
            &before.replace("\treturn 1", "\tvar/x = 1\n\tswitch(x)\n\t\tif(x)\n\t\t\treturn 1"),
        ),
        [(
            "code/code.dm".to_owned(),
            Some(4),
            "/datum/a/f: doesn't compile: unsupported statement: switch case that isn't a simple constant".to_owned()
        )]
    );
    assert_eq!(
        placed("placed-reparent", &before.replace("= /obj", "= /datum/b")),
        [(
            "code/code.dm".to_owned(),
            Some(6),
            "/obj/o: parent type changed: its instances would be a /datum instead of a /obj"
                .to_owned()
        )]
    );
}

#[test]
fn a_var_declared_twice_on_one_type_is_refused_where_it_is() {
    let before = "var/x = 1\n/datum/a\n/datum/t\n";
    let after = before.replace("/datum/t\n", "/datum/t\n\tvar/w = 1\n\tvar/w = 2\n");
    let refused = changes("dup-same", before, &after).refused;
    assert_eq!(
        shown(&refused),
        ["code/code.dm:5: parse error: duplicate definition of var/w"]
    );
    let after = before.replace("/datum/t\n", "/datum/t\n\tvar/vars\n");
    let refused = changes("dup-builtin", before, &after).refused;
    assert_eq!(
        refused,
        [Refusal {
            id: Id::DuplicateDefinition,
            path: "/datum/t".to_owned(),
            name: Some("vars".to_owned()),
            why: "duplicate definition (/datum declares it too)".to_owned(),
            file: Some("code/code.dm".to_owned()),
            line: Some(4),
            until: Until::Rebuild,
        }]
    );
}

#[test]
fn an_error_mended_by_reading_its_file_again_is_gone() {
    let broken =
        "/datum/a\n\tvar/const/k = sqrt(16)\n/datum/a/proc/f()\n\tvar/x = 1 +\n\treturn x\n";
    let world = World::new("mended", broken);
    let mut old = world.parse();
    let errors = old.parse_errors();
    assert_eq!(errors.len(), 1, "{:?}", errors);
    assert!(
        errors[0].to_string().starts_with("code/code.dm:4: "),
        "{:?}",
        errors
    );

    world.write(&broken.replace("1 +", "1 + 2"));
    let reload = old
        .reload_file(&world.dir, std::path::Path::new("code/code.dm"))
        .unwrap();
    old.keep(reload);
    assert!(old.parse_errors().is_empty(), "{:?}", old.parse_errors());
    world.write(&format!(
        "{}/proc/g()\n\treturn 2\n",
        broken.replace("1 +", "1 + 2")
    ));
    let changes = diff(&old, &world.parse(), None);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
}

#[test]
fn a_new_type_doesnt_keep_the_init_proc_its_parent_loses() {
    let before = "/datum/a\n\tvar/list/l = list(1)\n/datum/a/b\n";
    let after = "/datum/a\n\tvar/static/list/l = list(1)\n/datum/a/b\n/datum/a/b/c\n";
    let reloaded = changes("new-type-init", before, after);
    assert!(reloaded.refused.is_empty(), "{:?}", reloaded.refused);
    let mut inits: Vec<(String, bool)> = reloaded
        .plan
        .init_procs
        .iter()
        .map(|i| (i.type_path.clone(), i.code.is_some()))
        .collect();
    inits.sort();
    assert_eq!(
        inits,
        [
            ("/datum/a".to_owned(), false),
            ("/datum/a/b".to_owned(), false),
            ("/datum/a/b/c".to_owned(), false),
        ]
    );

    let fresh = changes(
        "new-type-no-init",
        "/datum/a\n\tvar/x = 1\n",
        "/datum/a\n\tvar/x = 1\n/datum/a/b\n",
    );
    assert!(
        fresh.plan.init_procs.is_empty(),
        "{:?}",
        fresh.plan.init_procs
    );
}

#[test]
fn a_built_in_a_type_stops_setting_comes_from_its_parent() {
    let before = "/mob/m\n\tlayer = 7\n\tsee_in_dark = 3\n/mob/m/sub\n/mob/m/own\n\tlayer = 2\n";
    let after = "/mob/m\n/mob/m/sub\n/mob/m/own\n\tlayer = 2\n";
    let changes = changes("builtin-inherited", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert!(
        changes.plan.builtin_defaults.is_empty(),
        "{:?}",
        changes.plan.builtin_defaults
    );
    let mut inherited: Vec<(String, Vec<String>)> = changes
        .plan
        .builtin_inherited
        .iter()
        .map(|i| (i.var.clone(), i.types.clone()))
        .collect();
    inherited.sort();
    let types = |t: &[&str]| t.iter().map(|t| t.to_string()).collect::<Vec<_>>();
    assert_eq!(
        inherited,
        [
            ("layer".to_owned(), types(&["/mob/m", "/mob/m/sub"])),
            (
                "see_in_dark".to_owned(),
                types(&["/mob/m", "/mob/m/own", "/mob/m/sub"])
            ),
        ]
    );
}

#[test]
fn a_built_in_a_built_in_type_stops_setting_needs_a_rebuild() {
    let own = changes(
        "builtin-inherited-builtin-type",
        "/mob\n\tlayer = 9\n/mob/m\n",
        "/mob\n/mob/m\n",
    );
    assert_eq!(
        shown(&own.refused),
        ["code/code.dm:1: /mob: layer: cannot restore this built-in type's original default"]
    );
    assert!(
        own.plan.builtin_inherited.is_empty(),
        "{:?}",
        own.plan.builtin_inherited
    );
    let table = changes(
        "builtin-inherited-var-table",
        "/sound\n\tpan = 10\n",
        "/sound\n",
    );
    assert!(table.refused.is_empty(), "{:?}", table.refused);
}

#[test]
fn a_setting_back_as_built_waits_for_nothing() {
    let world = World::new("settings-back", "/world\n\tfps = 10\n");
    let built = world.parse();
    world.write("/world\n\tfps = 20\n");
    let changed = world.parse();
    let edited: HashSet<PathBuf> = vec![PathBuf::from("code/code.dm")].into_iter().collect();
    let mut changes = diff(&built, &changed, Some(&edited));
    changes.settings_as_built(&built, &changed);
    assert_eq!(
        deferred(&changes),
        ["/world: fps: changed setting (rebuild)"]
    );
    world.write("/world\n\tfps = 10\n");
    let back = world.parse();
    let mut changes = diff(&changed, &back, Some(&edited));
    assert_eq!(
        deferred(&changes),
        ["/world: fps: changed setting (rebuild)"]
    );
    changes.settings_as_built(&built, &back);
    assert!(changes.deferred.is_empty(), "{:?}", changes.deferred);
}

fn deferred(changes: &Changes) -> Vec<String> {
    let mut shown: Vec<String> = changes
        .deferred
        .iter()
        .map(|d| {
            let until = match d.until {
                Until::Reboot => "reboot",
                Until::Rebuild => "rebuild",
            };
            format!("{d:#} ({until})")
        })
        .collect();
    shown.sort();
    shown
}

#[test]
fn world_and_client_settings_apply_after_a_restart() {
    let before = "/world\n\tfps = 10\n\tname = \"a\"\n/client\n\tcontrol_freak = 0\n\tdir = 1\n";
    let after = "/world\n\tfps = 20\n\tname = \"b\"\n\ticon_size = 64\n/client\n\tcontrol_freak = 1\n\tdir = 4\n";
    let changes = changes("world-settings", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert!(
        changes.plan.builtin_defaults.is_empty(),
        "{:?}",
        changes.plan.builtin_defaults
    );
    assert_eq!(
        deferred(&changes),
        [
            "/client: control_freak: changed setting (rebuild)",
            "/client: dir: changed setting (rebuild)",
            "/world: fps: changed setting (rebuild)",
            "/world: icon_size: changed setting (rebuild)",
            "/world: name: changed setting (rebuild)",
        ]
    );
    let fps = changes
        .deferred
        .iter()
        .find(|d| d.name.as_deref() == Some("fps"))
        .unwrap();
    assert_eq!(
        (fps.file.as_deref(), fps.line),
        (Some("code/code.dm"), Some(2))
    );
    assert!(
        !texts(&changes.warnings)
            .iter()
            .any(|w| w.contains("setting")),
        "{:?}",
        changes.warnings
    );
}

#[test]
fn built_ins_a_sound_keeps_in_its_var_table_are_var_defaults() {
    let before = "/sound/s\n\tvolume = 50\n/sound/s/sub\n/obj/o\n\tbounds = \"1,1 to 16,16\"\n/client\n\tfps = 20\n";
    let after = "/sound/s\n\tvolume = 60\n\trepeat = 1\n/sound/s/sub\n/sound/n\n\tchannel = 3\n/obj/o\n\tbounds = \"1,1 to 8,8\"\n/client\n\tfps = 40\n";
    let changes = changes("sound-defaults", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let mut defaults: Vec<(String, Vec<String>)> = changes
        .plan
        .var_defaults
        .iter()
        .map(|d| {
            let mut types = d.types.clone();
            types.sort();
            (d.var.clone(), types)
        })
        .collect();
    defaults.sort();
    let types = |t: &[&str]| t.iter().map(|t| t.to_string()).collect::<Vec<_>>();
    assert_eq!(
        defaults,
        [
            ("channel".to_owned(), types(&["/sound/n"])),
            ("repeat".to_owned(), types(&["/sound/s", "/sound/s/sub"])),
            ("volume".to_owned(), types(&["/sound/s", "/sound/s/sub"])),
        ]
    );
    let mut builtins: Vec<&str> = changes
        .plan
        .builtin_defaults
        .iter()
        .map(|d| d.var.as_str())
        .collect();
    builtins.sort();
    assert_eq!(builtins, ["bounds", "fps"]);
    assert_eq!(changes.plan.new_types.len(), 1);
    assert!(
        changes.plan.new_types[0]
            .fields
            .iter()
            .all(|(f, _)| f != "channel"),
        "{:?}",
        changes.plan.new_types
    );
}

#[test]
fn a_parent_change_comes_with_its_subtree() {
    let before = "/datum/a\n\tvar/x = 1\n/datum/a/proc/f()\n\treturn 1\n/datum/b\n\tvar/x = 2\n/datum/b/proc/f()\n\treturn 2\n/datum/t\n\tparent_type = /datum/a\n\tvar/own = 1\n\tx = 3\n/datum/t/f()\n\treturn ..()\n/datum/t/s\n\town = 2\n/obj/a\n\tname = \"ay\"\n/obj/b\n/obj/t\n\tparent_type = /obj/a\n\ticon_state = \"t\"\n";
    let after = before
        .replace("parent_type = /datum/a", "parent_type = /datum/b")
        .replace("parent_type = /obj/a", "parent_type = /obj/b");
    let changes = changes("reparent", before, &after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let shown: Vec<(
        String,
        String,
        Vec<(String, Vec<String>, Vec<String>, Vec<String>, ConstValue)>,
    )> = changes
        .plan
        .reparented
        .iter()
        .map(|r| {
            (
                r.type_path.clone(),
                r.parent.clone(),
                r.types
                    .iter()
                    .map(|t| {
                        (
                            t.path.clone(),
                            t.declares.clone(),
                            t.sets.clone(),
                            t.builtins.clone(),
                            t.name.clone(),
                        )
                    })
                    .collect(),
            )
        })
        .collect();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(
        shown,
        vec![
            (
                "/datum/t".to_owned(),
                "/datum/b".to_owned(),
                vec![
                    (
                        "/datum/t".to_owned(),
                        s(&["own"]),
                        s(&["x"]),
                        s(&[]),
                        ConstValue::String("t".to_owned())
                    ),
                    (
                        "/datum/t/s".to_owned(),
                        s(&[]),
                        s(&["own"]),
                        s(&[]),
                        ConstValue::String("s".to_owned())
                    ),
                ]
            ),
            (
                "/obj/t".to_owned(),
                "/obj/b".to_owned(),
                vec![(
                    "/obj/t".to_owned(),
                    s(&[]),
                    s(&[]),
                    s(&["icon_state"]),
                    ConstValue::String("t".to_owned())
                )]
            ),
        ]
    );
}

#[test]
fn parent_changes_a_reload_refuses() {
    let base = "/datum/a\n\tvar/x = 1\n/datum/a/proc/f()\n\treturn 1\n/datum/b\n/datum/t\n\tparent_type = /datum/a\n\tx = 2\n/datum/u\n\tparent_type = /datum/a\n\tvar/y = 1\n/datum/u/proc/g()\n\treturn 1\n/datum/b/var/y = 2\n/datum/b/proc/g()\n\treturn 2\n/obj/o\n\tparent_type = /obj\n";
    let cases = [
        (
            "undeclared",
            base.replace(
                "/datum/t\n\tparent_type = /datum/a",
                "/datum/t\n\tparent_type = /datum/b",
            ),
            "undefined var 'x'",
        ),
        (
            "duplicate",
            base.replace(
                "/datum/u\n\tparent_type = /datum/a",
                "/datum/u\n\tparent_type = /datum/b",
            ),
            "declares var y",
        ),
        (
            "kind",
            base.replace(
                "/obj/o\n\tparent_type = /obj",
                "/obj/o\n\tparent_type = /datum/b",
            ),
            "would be a /datum instead of a /obj",
        ),
    ];
    for (name, after, why) in cases.iter() {
        let refused = refusal(name, base, after);
        assert!(
            refused.contains(*why),
            "{}: expected {:?} in {:?}",
            name,
            why,
            refused
        );
    }
}

#[test]
fn the_skin_is_the_last_interface_file_included() {
    let world = World::new("skin", "/proc/a()\n\treturn 1\n");
    assert_eq!(world.parse().skin(), None);
    std::fs::create_dir_all(world.dir.join("ui")).unwrap();
    for name in ["a.dmf", "b.dmf", "c.dmf"] {
        std::fs::write(world.dir.join("ui").join(name), "window \"w\"\n").unwrap();
    }
    std::fs::write(
        world.dir.join("world.dme"),
        "#include \"ui\\a.dmf\"\n#include \"code/code.dm\"\n#include \"ui/b.dmf\"\n#if 0\n#include \"ui/c.dmf\"\n#endif\n",
    )
    .unwrap();
    let snapshot = world.parse();
    assert_eq!(snapshot.skin(), Some(std::path::Path::new("ui/b.dmf")));
    assert!(!snapshot
        .files()
        .iter()
        .any(|file| file.extension() == Some("dmf".as_ref())));
}

#[test]
fn maps_are_the_map_files_included() {
    let world = World::new("maps", "/proc/a()\n\treturn 1\n");
    assert!(world.parse().maps().is_empty());
    std::fs::create_dir_all(world.dir.join("maps")).unwrap();
    for name in ["a.dmm", "b.dmm"] {
        std::fs::write(
            world.dir.join("maps").join(name),
            "\"a\" = (/turf,/area)\n\n(1,1,1) = {\"\na\n\"}\n",
        )
        .unwrap();
    }
    std::fs::write(
        world.dir.join("world.dme"),
        "#include \"code/code.dm\"\n#include \"maps\\a.dmm\"\n#ifdef BIG\n#include \"maps/b.dmm\"\n#endif\n",
    )
    .unwrap();
    let parse = |defines: Vec<(String, Option<String>)>| {
        Snapshot::parse(
            &world.dir.join("world.dme"),
            &ParseOptions {
                defines,
                byond: None,
            },
        )
        .unwrap()
    };
    assert_eq!(
        parse(vec![]).maps(),
        [std::path::PathBuf::from("maps/a.dmm")]
    );
    assert_eq!(
        parse(vec![("BIG".to_owned(), None)]).maps(),
        [
            std::path::PathBuf::from("maps/a.dmm"),
            std::path::PathBuf::from("maps/b.dmm")
        ]
    );
}

#[test]
fn a_resource_file_names_the_code_and_defaults_that_use_it() {
    let world = World::new(
        "resource-users",
        "obj/lamp\n\
         \ticon = 'icons/lamp.dmi'\n\
         obj/lamp/red\n\
         obj/lamp/blue\n\
         \ticon = 'other.dmi'\n\
         datum/holder\n\
         \tvar/sound_file = 'beep.ogg'\n\
         \tvar/list/files = list('icons/lamp.dmi')\n\
         datum/holder/sub\n\
         /proc/states()\n\
         \treturn icon_states('icons/lamp.dmi')\n\
         /proc/play()\n\
         \tworld << sound('beep.ogg')\n\
         /proc/unrelated()\n\
         \treturn 1\n",
    );
    let snapshot = world.parse();

    let lamp = snapshot.resource_users(&["icons\\LAMP.dmi".to_owned()]);
    assert!(lamp.refused.is_empty(), "{:?}", lamp.refused);
    assert_eq!(reloaded(&lamp), vec![key("/states", 0)]);
    assert_eq!(
        lamp.plan.builtin_defaults.len(),
        1,
        "{:?}",
        lamp.plan.builtin_defaults
    );
    assert_eq!(lamp.plan.builtin_defaults[0].var, "icon");
    assert_eq!(
        lamp.plan.builtin_defaults[0].value,
        ConstValue::Resource("icons/lamp.dmi".to_owned())
    );
    let mut types = lamp.plan.builtin_defaults[0].types.clone();
    types.sort();
    assert_eq!(types, vec!["/obj/lamp", "/obj/lamp/red"]);
    let mut inits: Vec<&str> = lamp
        .plan
        .init_procs
        .iter()
        .map(|i| i.type_path.as_str())
        .collect();
    inits.sort();
    assert_eq!(inits, vec!["/datum/holder", "/datum/holder/sub"]);
    assert!(lamp.plan.init_procs.iter().all(|i| i.code.is_some()));

    let beep = snapshot.resource_users(&["beep.ogg".to_owned()]);
    assert_eq!(reloaded(&beep), vec![key("/play", 0)]);
    assert_eq!(
        beep.plan.var_defaults.len(),
        1,
        "{:?}",
        beep.plan.var_defaults
    );
    assert_eq!(beep.plan.var_defaults[0].var, "sound_file");
    assert!(beep.plan.builtin_defaults.is_empty() && beep.plan.init_procs.is_empty());

    assert!(snapshot
        .resource_users(&["missing.dmi".to_owned()])
        .is_empty());
}

#[test]
fn a_resource_is_found_wherever_code_or_a_default_names_it() {
    let world = World::new(
        "resource-places",
        "/obj/lamp
/datum/holder
	var/list/by_name = list(\"lamp\" = 'a.dmi')
	var/shown = icon('a.dmi')
	var/lamp = new /obj/lamp{icon = 'a.dmi'}
	var/other = 'b.dmi'
/proc/in_default(i = 'a.dmi')
	return i
/proc/in_block(x)
	if(x)
		for(var/i in 1 to 2)
			world << 'a.dmi'
/proc/in_switch(x)
	switch(x)
		if(1)
			return 2
		else
			return x ? 'a.dmi' : null
/proc/in_text()
	return \"\\icon['a.dmi']\"
/proc/in_list()
	return list(\"lamp\" = 'a.dmi')
/proc/in_new()
	return new /icon('a.dmi', \"on\")
/proc/in_call(datum/holder/h)
	return h.by_name.Find('a.dmi')
/proc/with_backslash()
	return 'icons\\\\A.dmi'
/proc/in_a_string()
	return \"'a.dmi'\"
/proc/unrelated()
	return 'b.dmi'
",
    );
    let snapshot = world.parse();
    let users = snapshot.resource_users(&["a.dmi".to_owned()]);
    let mut procs: Vec<String> = reloaded(&users).iter().map(|k| k.to_string()).collect();
    procs.sort();
    assert_eq!(
        procs,
        [
            "/in_block",
            "/in_call",
            "/in_default",
            "/in_list",
            "/in_new",
            "/in_switch",
            "/in_text"
        ]
    );
    let inits: Vec<&str> = users
        .plan
        .init_procs
        .iter()
        .map(|i| i.type_path.as_str())
        .collect();
    assert_eq!(inits, ["/datum/holder"]);
    for var in ["by_name", "shown", "lamp"] {
        let code = "/obj/lamp\n/datum/holder\n\tvar/VAR = VALUE\n"
            .replace("VAR", var)
            .replace(
                "VALUE",
                match var {
                    "by_name" => "list(\"lamp\" = 'a.dmi')",
                    "shown" => "icon('a.dmi')",
                    _ => "new /obj/lamp{icon = 'a.dmi'}",
                },
            );
        let world = World::new("resource-default", &code);
        let users = world.parse().resource_users(&["a.dmi".to_owned()]);
        assert_eq!(users.plan.init_procs.len(), 1, "{}: {:?}", var, users);
    }
    let world = World::new(
        "resource-evaluated",
        "/datum/keeper
	var/const/ICON = 'a.dmi'
	var/kept = ICON
	var/list/kept_list = list(ICON, 2)
",
    );
    let users = world.parse().resource_users(&["a.dmi".to_owned()]);
    assert_eq!(
        users.plan.var_defaults.len(),
        1,
        "{:?}",
        users.plan.var_defaults
    );
    assert_eq!(users.plan.var_defaults[0].var, "kept");
    assert_eq!(
        users.plan.init_procs.len(),
        1,
        "{:?}",
        users.plan.init_procs
    );
    assert_eq!(users.warnings.len(), 1, "{:?}", users.warnings);
    assert!(
        users.warnings[0].to_string().contains("const ICON"),
        "{:?}",
        users.warnings
    );
    let users = snapshot.resource_users(&["icons/a.dmi".to_owned()]);
    assert_eq!(reloaded(&users), [key("/with_backslash", 0)]);
}

#[test]
fn a_resource_named_with_a_combining_character_is_found() {
    let name = "sounds/cafe\u{301}.ogg";
    let world = World::new(
        "resource-accent",
        &format!(
            "/proc/play()\n\tworld << sound('{}')\n/datum/d\n\tvar/s = '{}'\n",
            name, name
        ),
    );
    let users = world.parse().resource_users(&[name.to_owned()]);
    assert_eq!(reloaded(&users), [key("/play", 0)]);
    assert_eq!(
        users.plan.var_defaults.len(),
        1,
        "{:?}",
        users.plan.var_defaults
    );
}

#[test]
fn the_client_script_is_left_to_the_launcher() {
    for (before, after) in [
        (
            "/client/script = \"<style>a {}</style>\"\n",
            "/client/script = \"<style>b {}</style>\"\n",
        ),
        (
            "/client/script = \"<style>a {}</style>\"\n",
            "/client/script = 'a.css'\n",
        ),
        ("/client/script = 'a.css'\n", "/client\n"),
    ] {
        let changes = changes("client-script", before, after);
        assert!(
            changes.is_empty() && changes.warnings.is_empty(),
            "{:?} -> {:?}: {:?}",
            before,
            after,
            changes
        );
    }

    let world = World::new(
        "client-script-value",
        "/client/script = {\"<style>\n.a {}\n</style>\"}\n",
    );
    assert_eq!(
        world.parse().client_script(),
        Ok(Some(ClientScript::Text(
            "<style>\n.a {}\n</style>".to_owned()
        )))
    );
    world.write("/client/script = 'style.css'\n");
    assert_eq!(
        world.parse().client_script(),
        Ok(Some(ClientScript::File("style.css".to_owned())))
    );
    world.write("/client/script = null\n");
    assert_eq!(world.parse().client_script(), Ok(None));
    world.write("var/x = 1\n/client/script = \"<style>[x] {}</style>\"\n");
    assert!(world.parse().client_script().is_err());
}

#[test]
fn a_declaration_moved_up_to_a_parent_is_taken() {
    let before = "/datum/hud\n/datum/hud/human\n\tvar/tmp/list/sue = list()\n/datum/hud/robot\n\tvar/list/sue = list(1)\n/datum/open\n/datum/open/river/proc/upd()\n\treturn 1\n";
    let after = "/datum/hud\n\tvar/list/sue = null\n/datum/hud/human\n/datum/hud/robot\n\tsue = list(1)\n/datum/open/proc/upd()\n\treturn 0\n/datum/open/river/upd()\n\treturn ..() + 1\n";
    let changes = changes("moved-up", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert_eq!(
        changes.plan.new_vars.len(),
        1,
        "{:?}",
        changes.plan.new_vars
    );
    assert_eq!(changes.plan.new_vars[0].type_path, "/datum/hud");
    let mut moved_from = changes.plan.new_vars[0].moved_from.clone();
    moved_from.sort();
    assert_eq!(moved_from, ["/datum/hud/human", "/datum/hud/robot"]);
    let redeclared: Vec<&str> = changes
        .plan
        .redeclared
        .iter()
        .map(|r| r.type_path.as_str())
        .collect();
    assert_eq!(redeclared, ["/datum/hud/human"]);
    assert!(
        texts(&changes.warnings)
            .iter()
            .all(|w| !w.contains("removed from source")),
        "{:?}",
        changes.warnings
    );
    assert_eq!(changes.plan.new_procs.len(), 1);
    assert_eq!(changes.plan.new_procs[0].type_path, "/datum/open");
    assert!(changes
        .plan
        .reloads
        .iter()
        .any(|r| r.proc.path == "/datum/open/river/upd"));

    let refused = refusal(
        "moved-up-static",
        "/datum/a\n/datum/a/b\n\tvar/static/s = 1\n",
        "/datum/a\n\tvar/static/s = 1\n/datum/a/b\n",
    );
    assert!(refused.contains("as a static or const var"), "{}", refused);
    let unstatic = changes_of(
        "moved-up-unstatic",
        "/datum/a\n/datum/a/b\n\tvar/static/s = 1\n",
        "/datum/a\n\tvar/s = 1\n/datum/a/b\n",
    );
    assert!(unstatic.refused.is_empty(), "{:?}", unstatic.refused);
    assert_eq!(
        unstatic.plan.redeclared.len(),
        1,
        "{:?}",
        unstatic.plan.redeclared
    );
    assert_eq!(
        (
            unstatic.plan.redeclared[0].type_path.as_str(),
            unstatic.plan.redeclared[0].flags
        ),
        ("/datum/a/b", 0)
    );
    let refused = refusal(
        "moved-up-verb",
        "/mob/a\n/mob/a/b/proc/v()\n\treturn 1\n",
        "/mob/a/verb/v()\n\treturn 0\n/mob/a/b/v()\n\treturn 1\n",
    );
    assert!(refused.contains("declaration kind or flags"), "{}", refused);
}

#[test]
fn a_moved_up_proc_that_names_itself_is_compiled_again() {
    let before = "/datum/a\n/datum/a/b/proc/f()\n\treturn __PROC__\n";
    let after = "/datum/a/proc/f()\n\treturn 0\n/datum/a/b/f()\n\treturn __PROC__\n";
    let changes = changes("moved-up-proc-name", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let reload = changes
        .plan
        .reloads
        .iter()
        .find(|r| r.proc.path == "/datum/a/b/f")
        .expect("moved proc was not recompiled");
    assert!(
        format!("{:?}", reload.nodes).contains("/datum/a/b/f\""),
        "{:?}",
        reload.nodes
    );
}

fn scope(var: &dmasm_bytecode::GlobalVar) -> String {
    match &var.scope {
        None => "?".to_owned(),
        Some(dmasm_bytecode::StaticScope::Global) => String::new(),
        Some(dmasm_bytecode::StaticScope::Type(path)) => path.clone(),
        Some(dmasm_bytecode::StaticScope::Proc { path, nth }) => format!("{path}#{nth}"),
    }
}

fn global_init_sets(changes: &Changes) -> Vec<String> {
    let init = changes.global_init.as_ref().expect("no global init proc");
    let mut line = 0;
    let mut sets = vec![];
    for node in &init.code.nodes {
        match node {
            Node::Instruction(Instruction::DbgLine(at), ()) => line = *at,
            Node::Instruction(Instruction::SetVar(dmasm_bytecode::Variable::Global(var)), ()) => {
                sets.push(format!(
                    "{}:{} @{line}",
                    scope(var),
                    String::from_utf8_lossy(&var.name.0)
                ))
            }
            _ => {}
        }
    }
    sets
}

#[test]
fn global_init_without_some_vars() {
    let before = "var/a = 5\nvar/b = a * 2\n/proc/f()\n\tvar/static/y = rand(1, 2)\n\treturn y\nvar/c = b + 1\n";
    let changes = changes_of(
        "global_init_without",
        before,
        &before.replace("var/a = 5", "var/a = 7"),
    );
    let mut changes = changes;
    let init = changes.global_init.take().unwrap();
    assert_eq!(init.table, 1);
    changes.global_init = Some(init.without(|var| var.name.0 == b"y" || var.name.0 == b"a"));
    assert_eq!(global_init_sets(&changes), [":b @2", ":c @6"]);
    assert_eq!(changes.global_init.as_ref().unwrap().table, 0);
}

#[test]
fn global_init_counts_a_procs_statics_of_a_name() {
    let before = "/proc/f()\n\tif (prob(50))\n\t\tvar/static/z = rand(1, 2)\n\t\treturn z\n\telse\n\t\tvar/static/z = rand(3, 4)\n\t\treturn z\n\
                  /proc/g()\n\tvar/static/y = rand(1, 2)\n\tvar/static/w = 1\n\treturn y + w\n";
    let changes = changes_of(
        "global_init_counts",
        before,
        &before.replace("rand(1, 2)", "rand(5, 6)"),
    );
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let counts = &changes.global_init.as_ref().unwrap().proc_statics;
    assert_eq!(
        format!("{:?}", counts),
        r#"{"/proc/f": {"z": 2}, "/proc/g": {"y": 1}}"#
    );
}

#[test]
fn global_init_reads_a_procs_earlier_statics() {
    let before = "var/list/base = list(1)\n/proc/f()\n\tvar/static/list/d = base\n\tvar/static/list/w = list(d)\n\treturn w\n\
                  /proc/g()\n\tvar/static/list/d = list(base)\n\treturn d\n/datum/t\n\tvar/static/flag = list(1)\n";
    let changes = changes_of(
        "global_init_earlier",
        before,
        &before.replace("flag = list(1)", "flag = list(2)"),
    );
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    let init = changes.global_init.as_ref().expect("no global init proc");
    let mut read: Vec<String> = init
        .code
        .nodes
        .iter()
        .filter_map(|node| match node {
            Node::Instruction(Instruction::GetVar(dmasm_bytecode::Variable::Global(var)), ()) => {
                Some(format!(
                    "{}:{}",
                    scope(var),
                    String::from_utf8_lossy(&var.name.0)
                ))
            }
            _ => None,
        })
        .collect();
    read.sort();
    assert_eq!(read, ["/proc/f#0:d", ":base", ":base"]);
}

#[test]
fn changed_initializers_give_a_rebuilds_global_init_proc() {
    let before = "var/a = 5\nvar/b = a * 2\n/datum/thing\n/datum/s\n\tvar/static/x = list(1)\n\
                  var/datum/thing/t = new(1)\n/proc/f()\n\tvar/static/y = rand(1, 2)\n\treturn y\n";
    let changes = changes_of(
        "global_init_constant",
        before,
        &before.replace("var/a = 5", "var/a = 7"),
    );
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert_eq!(
        global_init_sets(&changes),
        [":a @1", ":b @2", "/datum/s:x @5", ":t @6", "/proc/f#0:y @8"]
    );
    assert_eq!(
        deferred(&changes),
        ["(global): a: changed initializer (reboot)"]
    );

    let changes = changes_of(
        "global_init_new",
        before,
        &before.replace("new(1)", "new(2)"),
    );
    assert_eq!(
        global_init_sets(&changes),
        [":b @2", "/datum/s:x @5", ":t @6", "/proc/f#0:y @8"]
    );
    let nodes = &changes.global_init.as_ref().unwrap().code.nodes;
    let made: Vec<_> = nodes
        .windows(2)
        .filter(|pair| matches!(pair[1], Node::Instruction(Instruction::New(_), ())))
        .map(|pair| &pair[0])
        .collect();
    assert_eq!(made, [&Node::Instruction(Instruction::PushInt(2), ())]);

    let changes = changes_of(
        "global_init_added",
        before,
        &before
            .replace("var/b = a * 2\n", "var/b = a * 2\nvar/c = b + 1\n")
            .replace("rand(1, 2)", "rand(3, 4)")
            .replace("list(1)", "3"),
    );
    assert_eq!(
        global_init_sets(&changes),
        ["/datum/s:x @6", ":b @2", ":c @3", ":t @7", "/proc/f#0:y @9"]
    );

    let changes = changes_of(
        "global_init_now_computed",
        before,
        &before.replace("var/a = 5", "var/a = rand(5)"),
    );
    assert_eq!(
        global_init_sets(&changes),
        [
            ":a @1",
            ":a @1",
            ":b @2",
            "/datum/s:x @5",
            ":t @6",
            "/proc/f#0:y @8"
        ]
    );
    assert!(matches!(
        changes.global_init.as_ref().unwrap().code.nodes[2],
        Node::Instruction(Instruction::GetVar(dmasm_bytecode::Variable::Null), ())
    ));

    let changes = changes_of(
        "global_init_unchanged",
        before,
        &before.replace("\treturn y\n", "\treturn y + 1\n"),
    );
    assert!(changes.global_init.is_none());
}

#[test]
fn global_init_left_out_with_a_warning() {
    let changes = changes(
        "global_init_uncompiled",
        "var/a = 5\nvar/b = a\n",
        "var/a = 5\nvar/b = locate()\n",
    );
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert!(changes.global_init.is_none());
    assert!(
        texts(&changes.warnings).iter().any(|w| w
            .starts_with("(global init): (global): b: initializer compilation failed")
            && w.ends_with("reboot will use the build's global and static initializers")),
        "{:?}",
        changes.warnings
    );
    assert_eq!(
        deferred(&changes),
        ["(global): b: changed initializer (rebuild)"]
    );

    let world = World::new(
        "global_init_after_include",
        "var/a = 5\n#include \"more.dm\"\nvar/b = a + 1\n",
    );
    std::fs::write(world.dir.join("code/more.dm"), "var/c = a + 2\n").unwrap();
    let old = world.parse();
    world.write("var/a = 6\n#include \"more.dm\"\nvar/b = a + 1\n");
    let changes = diff(&old, &world.parse(), None);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert_eq!(global_init_sets(&changes), [":a @1", ":c @1", ":b @3"]);
}

#[test]
fn global_init_in_the_order_files_are_included() {
    let world = World::new("global_init_two_files", "var/a = 5\nvar/b = c + 1\n");
    std::fs::write(world.dir.join("code/zz.dm"), "var/c = rand(1, 2)\n").unwrap();
    std::fs::write(world.dir.join("code/aa.dm"), "var/d = b + c\n").unwrap();
    std::fs::write(
        world.dir.join("world.dme"),
        "#include \"code/zz.dm\"\n#include \"code/code.dm\"\n#include \"code/aa.dm\"\n",
    )
    .unwrap();
    let old = world.parse();
    world.write("var/a = 6\nvar/b = c + 1\n");
    let changes = diff(&old, &world.parse(), None);
    assert_eq!(
        global_init_sets(&changes),
        [":a @1", ":c @1", ":b @2", ":d @1"]
    );
    assert_eq!(changes.global_init.as_ref().unwrap().table, 1);
}

#[test]
fn global_init_in_dreammakers_order() {
    let cases = [
        (
            "var/a0 = list(0)\n/proc/f()\n\tvar/static/pf = list(1)\nvar/a1 = list(1)\n/proc/g()\n\tvar/static/pg = list(1)\nvar/a2 = list(1)\n",
            "a0 pf pg a1 a2",
        ),
        (
            "/datum/x\n\tvar/static/s = list(5)\nvar/a0 = list(0)\n/datum/y/proc/g()\n\tvar/static/p1 = list(1)\nvar/a1 = list(1)\n",
            "s p1 a0 a1",
        ),
        (
            "/datum/y\n/obj/o\n/datum/x\n\tvar/static/s0 = list(0)\n\tvar/static/s1 = new /datum/y\n\tvar/static/s2 = new /obj/o\n\tvar/static/s3 = new /datum\n\tvar/static/s4 = new /datum/x\n\tvar/static/s5 = new /datum/x/k\n\tvar/static/s6 = list(/datum/y)\n/datum/x/k\nvar/a0 = new /datum/x\nvar/a1 = new /datum\n",
            "s0 s1 s6 a0 a1 s2 s3 s4 s5",
        ),
        (
            "/datum/x\n\tvar/static/s0 = list(0)\n/datum/x/proc/f()\n\tvar/static/p0 = new /datum/x\n\tvar/static/p1 = new /datum\n\tvar/static/p2 = list(9)\n/datum/z\n\tvar/static/z0 = list(1)\nvar/a0 = list(0)\n",
            "s0 p2 z0 a0 p0 p1",
        ),
        (
            "var/a0 = list(0)\nvar/a1 = f()\n/datum/x\n\tvar/static/s1 = f()\n\tvar/static/s2 = new /datum\n\tvar/static/s3 = list(3)\n/proc/g()\n\tvar/static/p1 = f()\n\tvar/static/p2 = new /datum\nvar/a2 = new /datum\n/proc/f()\n\treturn 1\nvar/a3 = list(3)\n",
            "a0 s3 p2 a2 a3 a1 s1 s2 p1",
        ),
        (
            r#"/proc/first()
var/g0 = list(0)
var/regex/g1 = regex("a")
var/list/g2 = list(2)
var/datum/g3 = list(3)
var/datum/later/g4 = list(4)
var/icon/g5 = icon('a.dmi')
var/g6 = list(6)
var/obj/g7 = list(7)
var/g8 = 'a.dmi'
var/sound/g9 = list(9)
/proc/f()
	var/static/regex/s1 = regex("a")
	var/static/s2 = regex("a")
	var/static/datum/s3 = list(3)
/datum/later
var/datum/later/h4 = list(4)
var/datum/h3 = list(3)
"#,
            "s2 g0 g2 g6 h4 h3 s1 s3 g1 g3 g4 g5 g7 g9",
        ),
        (
            "var/a = list(1)\n/proc/f()\n\tvar/static/s = list(0)\n/var/b = list(2)\nvar/c = list(3)\n/datum/x\n\tvar/static/xs = list(9)\n/var/global/d = list(4)\nvar/global/e = list(5)\n/var/f2 = list(6)\n",
            "b d f2 a s c xs e",
        ),
        (
            "/datum/s/proc/first()\n/datum/s\n\tvar/static/c = list(1)\n/datum/s/proc/fire()\n\tvar/static/h = list(2)\n",
            "h c",
        ),
        (
            "/datum/s\n\tvar/static/c = list(1)\n/datum/s/New()\n\tvar/static/n = list(3)\n/datum/s\n\tvar/static/d = list(4)\n/datum/s/proc/late()\n\tvar/static/l = list(5)\n/datum/s\n\tvar/static/e = list(6)\n",
            "c n d l e",
        ),
        (
            "/datum/b\n/obj/o\n/datum/a\n\tvar/static/s1 = list(/obj/o)\n\tvar/static/s2 = list(/datum/b)\n\tvar/static/s3 = list(/datum/c)\n\tvar/static/s4 = new /obj/o\n\tvar/static/s5 = new /datum/b\n\tvar/static/list/obj/s6 = list(1)\n\tvar/static/list/datum/b/s7 = list(2)\n\tvar/static/list/datum/c/s8 = list(3)\n\tvar/static/obj/s9 = list(4)\n/datum/c\n",
            "s2 s5 s7 s1 s3 s4 s6 s8 s9",
        ),
    ];
    for (n, (source, order)) in cases.iter().enumerate() {
        let world = World::new(&format!("global_init_order_{n}"), source);
        let nodes = world.parse().global_init_proc().unwrap().code.nodes;
        let set: Vec<String> = nodes
            .iter()
            .filter_map(|node| match node {
                Node::Instruction(
                    Instruction::SetVar(dmasm_bytecode::Variable::Global(var)),
                    (),
                ) => Some(String::from_utf8_lossy(&var.name.0).into_owned()),
                _ => None,
            })
            .collect();
        assert_eq!(
            set.join(" "),
            *order,
            "unexpected initializer order for source:\n{source}"
        );
    }
}

#[test]
fn global_init_orders_dreammaker_gave() {
    let cases: Vec<(&str, &str, &str)> = include_str!("global_init_orders.txt")
        .split("=== ")
        .filter(|case| !case.is_empty())
        .map(|case| {
            let (head, source) = case.split_once('\n').unwrap();
            let (name, order) = head.split_once(": ").unwrap();
            (name, order, source)
        })
        .collect();
    let order_on = |name: &str, source: &str, byond: Option<(u32, u32)>| {
        let world = World::new(&format!("global_init_given_{name}"), source);
        let options = ParseOptions {
            byond,
            ..ParseOptions::default()
        };
        let snapshot = Snapshot::parse(&world.dir.join("world.dme"), &options).unwrap();
        let nodes = snapshot.global_init_proc().unwrap().code.nodes;
        let set: Vec<String> = nodes
            .iter()
            .filter_map(|node| match node {
                Node::Instruction(
                    Instruction::SetVar(dmasm_bytecode::Variable::Global(var)),
                    (),
                ) => Some(String::from_utf8_lossy(&var.name.0).into_owned()),
                _ => None,
            })
            .collect();
        set.join(" ")
    };
    let mut wrong = vec![];
    for (name, order, source) in &cases {
        let ours = order_on(name, source, Some((516, 1688)));
        if ours != *order {
            wrong.push(format!("{name}: {ours} for {order}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));

    let source = |name: &str| cases.iter().find(|case| case.0 == name).unwrap().2;
    assert_eq!(
        order_on("gg1", source("gg1"), Some((515, 1647))),
        "colors hair z x"
    );
    assert_eq!(
        order_on("gg1", source("gg1"), Some((516, 1648))),
        "colors z x hair"
    );
    assert_eq!(
        order_on("jj1", source("jj1"), Some((516, 1684))),
        "x y g a h"
    );
    assert_eq!(
        order_on("jj1", source("jj1"), Some((516, 1685))),
        "x y g h a"
    );
    assert_eq!(
        order_on("y8", source("y8"), Some((515, 1642))),
        "i ar t o m am a d c y s ic ma db dq e r mu df ge p z"
    );
    assert_eq!(
        order_on("y8", source("y8"), Some((515, 1643))),
        "i ar t o m am a d c y s ic ma db dq e r df ge p z mu"
    );
}

#[test]
fn global_init_after_a_reload_is_in_dmasms_order() {
    let sets = |changes: &Changes| -> Vec<String> {
        let init = changes.global_init.as_ref().expect("no global init proc");
        assert!(matches!(
            init.code.nodes.last(),
            Some(Node::Instruction(Instruction::End, ()))
        ));
        init.code
            .nodes
            .iter()
            .filter_map(|node| match node {
                Node::Instruction(
                    Instruction::SetVar(dmasm_bytecode::Variable::Global(var)),
                    (),
                ) => Some(String::from_utf8_lossy(&var.name.0).into_owned()),
                _ => None,
            })
            .collect()
    };
    let added = changes(
        "global_init_own_order",
        "var/a = list(1)\nvar/b = list(2)\nvar/c = list(3)\n",
        "var/a = list(1)\nvar/x = list(9)\nvar/b = list(2)\nvar/c = list(3)\n",
    );
    assert_eq!(sets(&added), ["a", "x", "b", "c"]);

    let reads_later = changes_of(
        "global_init_own_order_reads_later",
        "var/a = 10\nvar/b = a\nvar/c = -3.25\n",
        "var/a = c + 1\nvar/b = a\nvar/c = -3.25\n",
    );
    assert_eq!(sets(&reads_later), ["a", "a", "b"]);
}

#[test]
fn changed_initializers_are_deferred_to_a_reboot() {
    let before = "var/a = 1\nvar/b = list()\n/datum/s\n\tvar/static/x = 2\n\tvar/y = 3\n\
                  /proc/f()\n\tvar/static/z = 4\n\treturn z\n";
    let after = "\nvar/a = 5\nvar/b = list(1)\nvar/c = 6\n/datum/s\n\tvar/static/x = 7\n\
                 \tvar/static/y = 3\n/proc/f()\n\tvar/static/z = 8\n\treturn z\n";
    let changes = changes("deferred-initializers", before, after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert!(changes.global_init.is_some());
    assert_eq!(
        deferred(&changes),
        [
            "(global): a: changed initializer (reboot)",
            "(global): b: changed initializer (reboot)",
            "/datum/s: x: changed initializer (reboot)",
            "/proc/f: z: changed initializer (reboot)",
        ]
    );
    let lines: Vec<(String, Option<u32>)> = changes
        .deferred
        .iter()
        .map(|d| (d.name.clone().unwrap(), d.line))
        .collect();
    assert!(lines.contains(&("a".to_owned(), Some(2))), "{:?}", lines);
    assert!(lines.contains(&("z".to_owned(), Some(9))), "{:?}", lines);

    let changes = changes_of("deferred-moved", before, &format!("\n\n{}", before));
    assert!(changes.deferred.is_empty(), "{:?}", changes.deferred);
}

#[test]
fn unchanged_code_naming_a_retargeted_proc_path_is_compiled_again() {
    let before =
        "/datum/t/proc/f()\n\treturn 0\n\n/datum/t/f()\n\treturn 1\n\n/datum/t/f()\n\treturn 2\n\n\
                  /proc/calls()\n\treturn call(new /datum/t, /datum/t/f)()\n\n\
                  /proc/declared()\n\treturn call(new /datum/t, /datum/t/proc/f)()\n\n\
                  /datum/names\n\tvar/said = /datum/t/f\n\tvar/list/all_said = list(/datum/t/f)\n\n\
                  /datum/names/more\n";
    let removed = before.replace("/datum/t/f()\n\treturn 1\n\n", "");
    let changes = changes("retargeted-removed", before, &removed);
    assert_eq!(reloaded(&changes), [key("/calls", 0)]);
    let defaults: Vec<(&str, &[String])> = changes
        .plan
        .var_defaults
        .iter()
        .map(|d| (d.var.as_str(), d.types.as_slice()))
        .collect();
    assert_eq!(
        defaults,
        [(
            "said",
            &["/datum/names".to_owned(), "/datum/names/more".to_owned()][..]
        )]
    );
    let inits: Vec<&str> = changes
        .plan
        .init_procs
        .iter()
        .map(|i| i.type_path.as_str())
        .collect();
    assert_eq!(inits, ["/datum/names", "/datum/names/more"]);

    let inserted = before.replace(
        "/datum/t/f()\n\treturn 1\n",
        "/datum/t/f()\n\treturn 3\n\n/datum/t/f()\n\treturn 1\n",
    );
    let changes = changes_of("retargeted-inserted", before, &inserted);
    assert_eq!(reloaded(&changes), [key("/calls", 0)]);
    assert_eq!(
        changes.plan.var_defaults.len(),
        1,
        "{:?}",
        changes.plan.var_defaults
    );

    let appended = format!("{before}\n/datum/t/f()\n\treturn 4\n");
    let changes = changes_of("retargeted-appended", before, &appended);
    assert_eq!(reloaded(&changes), []);
    assert!(
        changes.plan.var_defaults.is_empty(),
        "{:?}",
        changes.plan.var_defaults
    );
    assert!(changes.plan.init_procs.is_empty());
}

#[test]
fn uncompiled_code_naming_a_retargeted_proc_path_is_refused() {
    let before = "/datum/t/proc/f()\n\treturn 0\n\n/datum/t/f()\n\treturn 1\n\n/datum/t/f()\n\treturn 2\n\n\
                  /proc/calls(x)\n\tswitch(x)\n\t\tif(x)\n\t\t\treturn call(new /datum/t, /datum/t/f)()\n";
    let removed = before.replace("/datum/t/f()\n\treturn 1\n\n", "");
    let changes = changes("retargeted-uncompiled", before, &removed);
    let refused: Vec<String> = changes.refused.iter().map(|r| format!("{r:#}")).collect();
    assert_eq!(
        refused,
        [
            "/calls: names a proc whose definitions change, so it must be compiled again, but it \
          doesn't compile: unsupported statement: switch case that isn't a simple constant"
        ]
    );
}

#[test]
fn unchanged_code_naming_a_child_path_follows_its_first_override() {
    let before = "/datum/t/proc/f()\n\treturn 0\n\n/datum/t/middle/f()\n\treturn 1\n\n\
                  /datum/t/middle/child\n\n\
                  /proc/calls()\n\treturn call(new /datum/t/middle/child, /datum/t/middle/child::f())()\n";
    let added = format!("{before}\n/datum/t/middle/child/f()\n\treturn 2\n");
    let changes = changes("retargeted-child-added", before, &added);
    assert_eq!(reloaded(&changes), [key("/calls", 0)]);

    let changes = changes_of("retargeted-child-removed", &added, before);
    assert_eq!(reloaded(&changes), [key("/calls", 0)]);
}

#[test]
fn unchanged_code_reading_a_default_that_names_a_retargeted_path_is_compiled_again() {
    let before =
        "/datum/t/proc/f()\n\treturn 0\n\n/datum/t/f()\n\treturn 1\n\n/datum/t/f()\n\treturn 2\n\n\
                  /datum/config\n\tvar/handler = /datum/t/f\n\n\
                  /proc/reads()\n\treturn /datum/config::handler\n\n\
                  /proc/unrelated()\n\treturn 3\n\n\
                  /datum/holder\n\tvar/list/handlers = list(/datum/config::handler)\n";
    let removed = before.replace("/datum/t/f()\n\treturn 1\n\n", "");
    let changes = changes("retargeted-folded", before, &removed);
    assert_eq!(reloaded(&changes), [key("/reads", 0)]);
    let inits: Vec<&str> = changes
        .plan
        .init_procs
        .iter()
        .map(|i| i.type_path.as_str())
        .collect();
    assert_eq!(inits, ["/datum/holder"]);
}

#[test]
fn a_verbs_unchanged_in_list_naming_a_retargeted_path_is_compiled_again() {
    let before = "/datum/t/proc/f()\n\treturn list(0)\n\n/datum/t/f()\n\treturn list(1)\n\n\
                  /datum/t/f()\n\treturn list(2)\n\n\
                  /mob/verb/choose(x in call(new /datum/t, /datum/t/f)())\n\treturn x\n";
    for (name, body) in [
        ("retargeted-in-list", "return x"),
        ("retargeted-in-list-body", "return 1"),
    ] {
        let after = before
            .replace("/datum/t/f()\n\treturn list(1)\n\n", "")
            .replace("return x", body);
        let changes = changes_of(name, before, &after);
        assert_eq!(reloaded(&changes), [key("/mob/choose", 0)], "{}", name);
        let in_list = &changes.plan.reloads[0].params[0].in_list;
        assert!(matches!(in_list, InList::New(_)), "{}: {:?}", name, in_list);
    }
}

#[test]
fn uncompiled_code_naming_another_types_path_of_the_same_name_is_not_refused() {
    let before = "/datum/a/proc/f()\n\treturn 0\n\n/datum/a/f()\n\treturn 1\n\n/datum/a/f()\n\treturn 2\n\n\
                  /datum/b/proc/f()\n\treturn 0\n\n/datum/b/f()\n\treturn 3\n\n\
                  /proc/other(x)\n\tswitch(x)\n\t\tif(x)\n\t\t\treturn call(new /datum/b, /datum/b/f)()\n";
    let removed = before.replace("/datum/a/f()\n\treturn 1\n\n", "");
    let changes = changes("retargeted-other-uncompiled", before, &removed);
    assert_eq!(reloaded(&changes), []);
}

#[test]
fn unchanged_code_naming_a_types_last_definition_follows_it() {
    let before =
        "/datum/t/proc/f()\n\treturn 0\n\n/datum/t/f()\n\treturn 1\n\n/datum/t/f()\n\treturn 2\n\n\
                  /proc/calls()\n\treturn call(new /datum/t, /datum/t::f())()\n\n\
                  /datum/names\n\tvar/said = /datum/t::f()\n";
    let removed = before.replace("/datum/t/f()\n\treturn 2\n\n", "");
    let changes = changes("last-removed", before, &removed);
    assert_eq!(reloaded(&changes), [key("/calls", 0)]);
    let code = dmasm_bytecode::format(&changes.plan.reloads[0].nodes);
    assert!(code.contains("PushVal /datum/t/f\n"), "{}", code);
    let defaults: Vec<(&str, &ConstValue, &[String])> = changes
        .plan
        .var_defaults
        .iter()
        .map(|d| (d.var.as_str(), &d.value, d.types.as_slice()))
        .collect();
    assert_eq!(
        defaults,
        [(
            "said",
            &ConstValue::Path("/datum/t/f".to_owned()),
            &["/datum/names".to_owned()][..]
        )]
    );

    let changes = changes_of("last-appended", &removed, before);
    let refused: Vec<String> = changes.refused.iter().map(|r| format!("{r:#}")).collect();
    assert_eq!(
        refused,
        [
            "/datum/names: initializer compilation failed: line 14: unsupported statement: type::proc() \
          that names a type's second or later override",
            "/calls: names a proc whose definitions change, so it must be compiled again, but it \
          doesn't compile: unsupported statement: type::proc() that names a type's second or later override"
        ]
    );
}

#[test]
fn uncompiled_code_naming_a_declaration_that_stays_is_not_refused() {
    let before = "/datum/t/proc/f()\n\treturn 0\n\n/datum/t/f()\n\treturn 1\n\n/datum/t/f()\n\treturn 2\n\n\
                  /proc/declared(x)\n\tswitch(x)\n\t\tif(x)\n\t\t\treturn call(new /datum/t, /datum/t/proc/f)()\n";
    let removed = before.replace("/datum/t/f()\n\treturn 1\n\n", "");
    let changes = changes("declared-uncompiled", before, &removed);
    assert_eq!(reloaded(&changes), []);
}

fn sorted_reloads(changes: &Changes) -> Vec<String> {
    let mut reloads: Vec<String> = reloaded(changes).iter().map(ToString::to_string).collect();
    reloads.sort();
    reloads
}

fn init_types(changes: &Changes) -> Vec<&str> {
    let mut types: Vec<&str> = changes
        .plan
        .init_procs
        .iter()
        .map(|i| i.type_path.as_str())
        .collect();
    types.sort();
    types
}

#[test]
fn a_member_var_added_or_removed_takes_unqualified_uses_from_a_global() {
    let before = "var/count = 100\n\n\
                  /datum/t\n\tvar/list/counts = list(count)\n\n\
                  /datum/t/proc/bump()\n\tcount++\n\n\
                  /datum/t/proc/local()\n\tvar/count = 1\n\treturn count\n\n\
                  /datum/t/child/proc/bump_more()\n\tcount += 2\n\n\
                  /datum/u/proc/bump()\n\tcount++\n";
    let added = before.replace("/datum/t\n", "/datum/t\n\tvar/count = 0\n");
    for (name, before, after) in [
        ("member-var-added", before, added.as_str()),
        ("member-var-removed", added.as_str(), before),
    ] {
        let changes = changes_of(name, before, after);
        assert_eq!(
            sorted_reloads(&changes),
            ["/datum/t/bump", "/datum/t/child/bump_more"],
            "{}",
            name
        );
        assert_eq!(
            init_types(&changes),
            ["/datum/t", "/datum/t/child"],
            "{}",
            name
        );
    }

    let unshadowed = before.replace("var/count = 100\n", "");
    let added = unshadowed.replace("/datum/t\n", "/datum/t\n\tvar/count = 0\n");
    let changes = changes_of("member-var-without-global", &unshadowed, &added);
    assert_eq!(sorted_reloads(&changes), Vec::<String>::new());
}

#[test]
fn a_member_proc_added_or_removed_takes_unqualified_calls_from_a_global() {
    let before = "/proc/helper()\n\treturn 1\n\n\
                  /datum/t/proc/ask()\n\treturn helper()\n\n\
                  /datum/t/child/proc/ask_more()\n\treturn helper() + 1\n\n\
                  /datum/u/proc/ask()\n\treturn helper()\n";
    let added = format!("{before}\n/datum/t/proc/helper()\n\treturn 2\n");
    for (name, before, after) in [
        ("member-proc-added", before, added.as_str()),
        ("member-proc-removed", added.as_str(), before),
    ] {
        let changes = changes_of(name, before, after);
        assert_eq!(
            sorted_reloads(&changes),
            ["/datum/t/ask", "/datum/t/child/ask_more"],
            "{}",
            name
        );
    }

    let overridden = format!("{added}\n/datum/t/child/helper()\n\treturn 3\n");
    let changes = changes_of("member-proc-overridden", &added, &overridden);
    assert_eq!(sorted_reloads(&changes), Vec::<String>::new());
}

#[test]
fn a_changed_default_recompiles_the_initializers_and_in_lists_that_read_it() {
    let before = "/datum/config\n\tvar/limit = 5\n\n\
                  /datum/holder\n\tvar/list/limits = list(/datum/config::limit)\n\tvar/copied = /datum/config::limit\n\n\
                  /mob/verb/choose(x in list(/datum/config::limit))\n\treturn x\n";
    let after = before.replace("limit = 5", "limit = 10");
    let changes = changes("default-read", before, &after);
    assert_eq!(sorted_reloads(&changes), ["/mob/choose"]);
    let in_list = &changes.plan.reloads[0].params[0].in_list;
    assert!(matches!(in_list, InList::New(_)), "{:?}", in_list);
    assert_eq!(init_types(&changes), ["/datum/holder"]);
    let mut defaults: Vec<(&str, &ConstValue)> = changes
        .plan
        .var_defaults
        .iter()
        .map(|d| (d.var.as_str(), &d.value))
        .collect();
    defaults.sort_by_key(|d| d.0);
    assert_eq!(
        defaults,
        [
            ("copied", &ConstValue::Number(10.0)),
            ("limit", &ConstValue::Number(10.0))
        ]
    );
}

#[test]
fn a_global_initializer_naming_a_retargeted_path_is_given_to_a_reboot() {
    let before = "/datum/t/proc/f()\n\treturn 0\n\n/datum/t/f()\n\treturn 1\n\n/datum/t/f()\n\treturn 2\n\n\
                  var/list/handlers = list(/datum/t/f)\nvar/handler = /datum/t/f\nvar/other = list(1)\n\n\
                  /datum/s\n\tvar/static/list/more = list(/datum/t/f)\n\n\
                  /proc/kept()\n\tvar/static/list/l = list(/datum/t/f)\n\treturn l\n";
    let removed = before.replace("/datum/t/f()\n\treturn 1\n\n", "");
    let changes = changes("global-init-retargeted", before, &removed);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert!(changes.deferred.is_empty(), "{:?}", changes.deferred);
    assert_eq!(
        global_init_sets(&changes),
        [
            ":handler @8",
            ":other @9",
            "/datum/s:more @12",
            ":handlers @7",
            "/proc/kept#0:l @15"
        ]
    );
    assert_eq!(changes.global_init.as_ref().unwrap().table, 1);

    let appended = format!("{before}\n/datum/t/f()\n\treturn 3\n");
    let changes = changes_of("global-init-appended", before, &appended);
    assert!(changes.global_init.is_none());
}

#[test]
fn an_implicit_new_follows_the_vars_declared_type() {
    let before = "/datum/a\n/datum/b\n\n\
                  /datum/holder\n\tvar/datum/a/item = new\n\n\
                  /datum/holder/child\n\n\
                  /datum/other\n\tvar/datum/a/item = new\n";
    let after = before.replacen("var/datum/a/item", "var/datum/b/item", 1);
    let changes = changes("implicit-new-retyped", before, &after);
    assert!(changes.refused.is_empty(), "{:?}", changes.refused);
    assert_eq!(
        init_types(&changes),
        ["/datum/holder", "/datum/holder/child"]
    );
    for init in &changes.plan.init_procs {
        let code = dmasm_bytecode::format(&init.code.as_ref().unwrap().nodes);
        assert!(code.contains("/datum/b"), "{}: {}", init.type_path, code);
    }
}
