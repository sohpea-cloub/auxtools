use dmasm_bytecode::{Instruction, Node, Variable};
use dmasm_compiler::compiler::{compile_proc, ProcSource};

fn compile(code: &str, ty: &str, name: &str) -> String {
    compile_on(code, ty, name, dmasm_compiler::ByondVersion::LATEST)
}

fn compile_on(code: &str, ty: &str, name: &str, byond: dmasm_compiler::ByondVersion) -> String {
    let ctx = dreammaker::Context::default();
    let pp = dreammaker::Preprocessor::from_buffer(&ctx, "test.dm".into(), code.to_owned());
    let mut parser = dreammaker::Parser::new(&ctx, pp);
    parser.enable_procs();
    let tree = parser.parse_object_tree();
    let ty = if ty.is_empty() {
        tree.root()
    } else {
        tree.find(ty).expect("type")
    };
    let value = ty
        .get()
        .procs
        .get(name)
        .expect("proc")
        .value
        .last()
        .expect("proc value");
    let source = ProcSource {
        tree: &tree,
        ty,
        name,
        value,
        file: "test.dm",
        debug_info: false,
        index: None,
        byond,
        file_names: None,
    };
    let compiled = compile_proc(&source).expect("compiles");
    dmasm_bytecode::format(&compiled.nodes)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[track_caller]
fn assert_has(code: &str, lines: &[&str]) {
    let want = lines.join("\n");
    assert!(code.contains(&want), "expected\n{}\nin\n{}", want, code);
}

#[test]
fn findlasttext_defaults() {
    let code = compile("/proc/f(a)\n\treturn findlasttext(a, \"/\")\n", "", "f");
    assert_has(
        &code,
        &["PushVal \"/\"", "PushInt 0", "PushInt 1", "FindLastText"],
    );
}

#[test]
fn empty_argument_is_pushed_null() {
    let code = compile("/proc/g(x, y, z)\n/proc/f(a, b)\n\tg(a, , b)\n", "", "f");
    assert_has(&code, &["GetVar arg(0)", "PushVal null", "GetVar arg(1)"]);
}

#[test]
fn const_alone_is_its_global_but_folds_in_expressions() {
    let src = "var/const/RC = 20\n/proc/f()\n\tworld << RC\n\tworld << RC + 1\n\tworld << NORTH\n";
    let code = compile(src, "", "f");
    assert_has(&code, &["GetVar world", "GetVar global(\"RC\")", "Output"]);
    assert_has(&code, &["GetVar world", "PushInt 21", "Output"]);
    assert_has(&code, &["GetVar world", "PushInt 1", "Output"]);
}

#[test]
fn colon_reads_the_last_type_that_declares_its_var() {
    let src = "/datum/q\n/datum/p/a\n\tvar/const/LIMIT = 5\n/datum/p/b\n\tvar/const/LIMIT = 9\n/datum/p/b/c\n\
               /datum/q\n\tvar/const/LIMIT = 20\n\tvar/static/S = 4\n\
               /proc/f(x, datum/q/y, list/z)\n\treturn list(x:LIMIT + 1, y:LIMIT + 1, z:LIMIT + 1, x:LIMIT, -y:LIMIT, x:S, x?:S)\n";
    let code = compile(src, "", "f");
    assert_has(
        &code,
        &[
            "PushInt 10",
            "PushInt 21",
            "PushInt 10",
            "GetVar global(\"LIMIT\")",
            "PushVal -20",
            "GetVar global(\"S\")",
        ],
    );
    assert_eq!(code.matches("GetVar global(\"S\")").count(), 2, "{}", code);
}

#[test]
fn colon_chain_reads_the_type_of_each_var_it_goes_through() {
    let src = "/datum/config\n\tvar/const/LIMIT = 5\n/datum/holder\n\tvar/datum/config/config\n\
               /datum/other\n\tvar/const/LIMIT = 9\n\
               /proc/f(x)\n\treturn list(x:config:LIMIT + 1, x:config.LIMIT + 2)\n";
    let code = compile(src, "", "f");
    assert_has(&code, &["PushInt 6", "PushInt 7", "NewList 2"]);
}

#[test]
fn a_safe_chain_finds_a_var_after_an_index_as_untyped_colon_finds_it() {
    let types = "/datum/a\n\tvar/const/LIMIT = 5\n\tvar/static/S = 1\n\tvar/datum/a/link\n\
                 /datum/b\n\tvar/const/LIMIT = 9\n\tvar/static/S = 2\n\tvar/datum/b/link\n\
                 /datum/holder\n\tvar/list/datum/a/items\n\tvar/list/plain\n";
    for (body, read) in [
        ("return holder?.items[1].LIMIT", "GetVar global(\"LIMIT\")"),
        ("return holder?.plain[1].LIMIT", "GetVar global(\"LIMIT\")"),
        ("return holder?.items[1]:LIMIT", "GetVar global(\"LIMIT\")"),
        ("return holder.items?[1].LIMIT", "GetVar global(\"LIMIT\")"),
        (
            "return holder?.plain[1].link.LIMIT",
            "GetVar global(\"LIMIT\")",
        ),
        ("return holder?.items[1]?.LIMIT", "GetVar global(\"LIMIT\")"),
        ("return holder.items[1]?.LIMIT", "GetVar global(\"LIMIT\")"),
        (
            "return holder?.items[1].link?.LIMIT",
            "GetVar global(\"LIMIT\")",
        ),
        ("return holder?.items[1].S", "GetVar global(\"S\")"),
        ("holder?.items[1].S = 3", "SetVar global(\"S\")"),
        ("holder?.items[1].S++", "Inc global(\"S\")"),
        ("holder.items[1]?.S = 3", "SetVar global(\"S\")"),
    ] {
        let src = format!("{types}/proc/f(datum/holder/holder)\n\t{body}\n");
        let ctx = dreammaker::Context::default();
        let pp = dreammaker::Preprocessor::from_buffer(&ctx, "test.dm".into(), src);
        let mut parser = dreammaker::Parser::new(&ctx, pp);
        parser.enable_procs();
        let tree = parser.parse_object_tree();
        let value = tree
            .root()
            .get()
            .procs
            .get("f")
            .unwrap()
            .value
            .last()
            .unwrap();
        let source = ProcSource {
            tree: &tree,
            ty: tree.root(),
            name: "f",
            value,
            file: "test.dm",
            debug_info: false,
            index: None,
            byond: dmasm_compiler::ByondVersion::LATEST,
            file_names: None,
        };
        let nodes = compile_proc(&source).expect("compiles").nodes;
        let code = dmasm_bytecode::format(&nodes);
        let lines: Vec<&str> = code.lines().map(str::trim).collect();
        assert!(
            lines.contains(&"ListGet") && lines.contains(&read),
            "{}: no ListGet and {} in {:?}",
            body,
            read,
            lines
        );
        let scopes: Vec<String> = nodes
            .iter()
            .filter_map(|node| match node {
                Node::Instruction(
                    Instruction::GetVar(Variable::Global(g))
                    | Instruction::SetVar(Variable::Global(g))
                    | Instruction::Inc(Variable::Global(g)),
                    _,
                ) => Some(format!("{:?}", g.scope)),
                _ => None,
            })
            .collect();
        assert_eq!(
            scopes,
            ["Some(Type(\"/datum/b\"))"],
            "{}: {:?}",
            body,
            lines
        );
    }
}

#[test]
fn a_var_after_a_call_is_found_by_the_procs_return_type() {
    let types = "/datum/a\n\tvar/const/LIMIT = 5\n\tvar/static/S = 1\n\tvar/datum/a/link\n\
                 /datum/b\n\tvar/const/LIMIT = 9\n\tvar/static/S = 2\n\tvar/datum/b/link\n\
                 /datum/factory/proc/make()\n\treturn new /datum/a\n\
                 /datum/factory/proc/typed() as /datum/a\n\treturn new /datum/a\n\
                 /proc/gmake()\n\treturn new /datum/a\n\
                 /proc/gtyped() as /datum/a\n\treturn new /datum/a\n";
    for (body, scope) in [
        ("return factory.make().LIMIT", "/datum/b"),
        ("return factory.make():LIMIT", "/datum/b"),
        ("return factory.typed().LIMIT", "/datum/a"),
        ("return factory.typed():LIMIT", "/datum/b"),
        ("return factory.typed().link.LIMIT", "/datum/a"),
        ("return factory.make().link.LIMIT", "/datum/b"),
        ("return factory.make().S", "/datum/b"),
        ("return factory?.make().LIMIT", "/datum/b"),
        ("return factory?.typed().LIMIT", "/datum/a"),
        ("return factory.typed()?.LIMIT", "/datum/a"),
        ("return factory.make()?.LIMIT", "/datum/b"),
        ("return gmake().LIMIT", "/datum/b"),
        ("return gtyped().LIMIT", "/datum/a"),
    ] {
        let src = format!("{types}/proc/f(datum/factory/factory)\n\t{body}\n");
        let ctx = dreammaker::Context::default();
        let pp = dreammaker::Preprocessor::from_buffer(&ctx, "test.dm".into(), src);
        let mut parser = dreammaker::Parser::new(&ctx, pp);
        parser.enable_procs();
        let tree = parser.parse_object_tree();
        let value = tree
            .root()
            .get()
            .procs
            .get("f")
            .unwrap()
            .value
            .last()
            .unwrap();
        let source = ProcSource {
            tree: &tree,
            ty: tree.root(),
            name: "f",
            value,
            file: "test.dm",
            debug_info: false,
            index: None,
            byond: dmasm_compiler::ByondVersion::LATEST,
            file_names: None,
        };
        let nodes = compile_proc(&source).expect("compiles").nodes;
        let scopes: Vec<String> = nodes
            .iter()
            .filter_map(|node| match node {
                Node::Instruction(Instruction::GetVar(Variable::Global(g)), _) => {
                    Some(format!("{:?}", g.scope))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            scopes,
            [format!("Some(Type(\"{}\"))", scope)],
            "{}: {}",
            body,
            dmasm_bytecode::format(&nodes)
        );
    }
}

fn global_scopes(src: &str) -> Vec<String> {
    global_scopes_in(src, "")
}

fn global_scopes_in(src: &str, ty: &str) -> Vec<String> {
    let ctx = dreammaker::Context::default();
    let pp = dreammaker::Preprocessor::from_buffer(&ctx, "test.dm".into(), src.to_owned());
    let mut parser = dreammaker::Parser::new(&ctx, pp);
    parser.enable_procs();
    let tree = parser.parse_object_tree();
    let ty = match ty {
        "" => tree.root(),
        path => tree.find(path).expect("type"),
    };
    let value = ty.get().procs.get("f").unwrap().value.last().unwrap();
    let source = ProcSource {
        tree: &tree,
        ty,
        name: "f",
        value,
        file: "test.dm",
        debug_info: false,
        index: None,
        byond: dmasm_compiler::ByondVersion::LATEST,
        file_names: None,
    };
    let nodes = compile_proc(&source).expect("compiles").nodes;
    let mut scopes = vec![];
    for node in &nodes {
        let Node::Instruction(ins, _) = node else {
            continue;
        };
        let mut vars = ins.variables();
        while let Some(var) = vars.pop() {
            match var {
                Variable::Global(g) => {
                    let scope = format!("{:?}", g.scope);
                    if !scopes.contains(&scope) {
                        scopes.push(scope);
                    }
                }
                Variable::SetCache(holder, field) => vars.extend([&**field, &**holder]),
                _ => {}
            }
        }
    }
    scopes
}

#[test]
fn a_safe_write_after_a_call_finds_its_static_as_a_read_does() {
    let types = "/datum/a\n\tvar/static/S = 1\n/datum/b\n\tvar/static/S = 2\n\
                 /datum/factory/proc/make()\n\treturn new /datum/a\n\
                 /datum/factory/proc/typed() as /datum/a\n\treturn new /datum/a\n\
                 /proc/gmake()\n\treturn new /datum/a\n\
                 /proc/gtyped() as /datum/a\n\treturn new /datum/a\n";
    for (body, scope) in [
        ("factory.make()?.S = 3", "/datum/b"),
        ("factory.typed()?.S = 3", "/datum/a"),
        ("gmake()?.S = 3", "/datum/b"),
        ("gtyped()?.S = 3", "/datum/a"),
        ("factory?.make().S = 3", "/datum/b"),
        ("factory?.typed().S = 3", "/datum/a"),
        ("factory.make()?.S += 3", "/datum/b"),
        ("factory.typed()?.S++", "/datum/a"),
        ("factory.make()?.S--", "/datum/b"),
        ("factory.make()?.S ||= 3", "/datum/b"),
        ("return (factory.typed()?.S = 4)", "/datum/a"),
        ("return factory.make()?.S++", "/datum/b"),
    ] {
        let src = format!("{types}/proc/f(datum/factory/factory)\n\t{body}\n");
        assert_eq!(
            global_scopes(&src),
            [format!("Some(Type(\"{}\"))", scope)],
            "{}",
            body
        );
    }
}

#[test]
fn a_global_call_calls_the_global_proc_and_reads_by_its_return_type() {
    let types = "/datum/a\n\tvar/const/LIMIT = 5\n\tvar/static/S = 1\n\
                 /datum/b\n\tvar/const/LIMIT = 9\n\tvar/static/S = 2\n\
                 /proc/make() as /datum/a\n\treturn new /datum/a\n\
                 /datum/thing/proc/make() as /datum/b\n\treturn new /datum/b\n";
    for (body, call) in [
        ("return ::make()", "CallGlob 0 /proc/make"),
        ("return ::make(1, 2)", "CallGlob 2 /proc/make"),
        (
            "return ::make(arglist(list(1)))",
            "CallGlobalArgList /proc/make",
        ),
        ("return ::make(a = 1)", "CallGlobalArgList /proc/make"),
        ("return ::make().LIMIT", "CallGlob 0 /proc/make"),
    ] {
        let code = compile(
            &format!("{types}/datum/thing/proc/f()\n\t{body}\n"),
            "/datum/thing",
            "f",
        );
        assert_has(&code, &[call]);
        assert!(!code.contains("dynamic_proc"), "{}: {}", body, code);
    }
    for (ty, body) in [
        ("/datum/thing", "return ::make().LIMIT"),
        ("/datum/thing", "return ::make().S"),
        ("/datum/thing", "return ::make()?.LIMIT"),
        ("/datum/thing", "::make()?.S = 3"),
        ("", "return ::make().LIMIT"),
    ] {
        let src = match ty {
            "" => format!("{types}/proc/f()\n\t{body}\n"),
            ty => format!("{types}{ty}/proc/f()\n\t{body}\n"),
        };
        assert_eq!(
            global_scopes_in(&src, ty),
            ["Some(Type(\"/datum/a\"))"],
            "{}",
            body
        );
    }
}

#[test]
fn a_safe_chain_finds_each_calls_proc_and_value_by_the_type_before_it() {
    let types = "/datum/a\n\tvar/const/LIMIT = 5\n\tvar/static/S = 1\n\
                 /datum/b\n\tvar/const/LIMIT = 9\n\tvar/static/S = 2\n\
                 /datum/decoy/proc/make() as /datum/b\n\treturn new /datum/b\n\
                 /datum/decoy/proc/named()\n\treturn new /datum/b\n\
                 /datum/decoy/verb/shown()\n\treturn new /datum/b\n\
                 /datum/builder/proc/make() as /datum/a\n\treturn new /datum/b\n\
                 /datum/builder/proc/named()\n\tset name = \"build it\"\n\treturn new /datum/b\n\
                 /datum/builder/proc/me() as /datum/builder\n\treturn src\n\
                 /datum/builder/proc/shown()\n\treturn new /datum/b\n\
                 /datum/factory/proc/builder() as /datum/builder\n\treturn new /datum/builder\n";
    for body in [
        "return factory?.builder().make().LIMIT",
        "return factory?.builder().make().S",
        "return factory?.builder().make()?.LIMIT",
        "return factory?.builder().me().make().LIMIT",
        "return factory?.builder():make().LIMIT",
        "return factory?.builder().make().LIMIT + 1",
    ] {
        let src = format!("{types}/proc/f(datum/factory/factory)\n\t{body}\n");
        assert_eq!(
            global_scopes(&src),
            ["Some(Type(\"/datum/a\"))"],
            "{}",
            body
        );
    }
    for (body, call) in [
        (
            "return factory?.builder().named()",
            "Call dynamic_proc(\"build it\") 0",
        ),
        (
            "return factory?.builder().me().named()",
            "Call dynamic_proc(\"build it\") 0",
        ),
        (
            "return factory?.builder().shown()",
            "Call dynamic_proc(\"shown\") 0",
        ),
    ] {
        let code = compile(
            &format!("{types}/proc/f(datum/factory/factory)\n\t{body}\n"),
            "",
            "f",
        );
        assert_has(&code, &[call]);
    }
}

#[test]
fn a_safe_write_finds_the_statics_it_goes_through_as_a_read_does() {
    let types = "/datum/counter\n\tvar/count = 0\n\tvar/datum/counter/next\n\
                 \tvar/static/datum/counter/INNER\n\
                 /datum/a\n\tvar/static/datum/counter/SHARED = new\n\
                 \tvar/datum/counter/plain\n\tvar/static/S = 1\n\
                 /datum/b\n\tvar/static/datum/counter/SHARED = new\n\
                 \tvar/datum/counter/plain\n\tvar/static/S = 2\n\
                 /datum/holder\n\tvar/datum/a/a\n\
                 /proc/make()\n\treturn new /datum/a\n\
                 /proc/typed() as /datum/a\n\treturn new /datum/b\n";
    for (body, scopes) in [
        ("make()?.SHARED.count += 1", &["/datum/b"][..]),
        ("typed()?.SHARED.count += 1", &["/datum/a"]),
        ("make()?.SHARED.count = 1", &["/datum/b"]),
        ("make()?.SHARED?.count = 1", &["/datum/b"]),
        ("make()?.SHARED?.count += 1", &["/datum/b"]),
        ("make()?.SHARED.next.count++", &["/datum/b"]),
        ("make()?.SHARED.count ||= 2", &["/datum/b"]),
        ("return (make()?.SHARED.count ||= 2)", &["/datum/b"]),
        ("return (typed()?.S ||= 2)", &["/datum/a"]),
        ("x?.SHARED.count += 1", &["/datum/a"]),
        ("x?.SHARED.count++", &["/datum/a"]),
        ("return x?.SHARED.count++", &["/datum/a"]),
        ("return (x?.SHARED.count += 1)", &["/datum/a"]),
        ("x?.SHARED.next?.count = 3", &["/datum/a"]),
        ("h?.a.SHARED.count += 1", &["/datum/a"]),
        ("h?.a?.SHARED.count = 1", &["/datum/a"]),
        ("x?.plain.INNER.count = 3", &["/datum/counter"]),
        ("x?.SHARED.INNER.count = 3", &["/datum/a", "/datum/counter"]),
        ("y?:SHARED.count = 3", &["/datum/b"]),
    ] {
        let src = format!("{types}/proc/f(datum/a/x, datum/holder/h, y)\n\t{body}\n");
        let want: Vec<String> = scopes
            .iter()
            .map(|s| format!("Some(Type(\"{s}\"))"))
            .collect();
        assert_eq!(global_scopes(&src), want, "{}", body);
    }
}

#[test]
fn a_safe_colon_call_finds_its_proc_and_value_as_a_colon_call_does() {
    let types = "/datum/a\n\tvar/const/LIMIT = 5\n\tvar/static/S = 1\n\
                 /datum/b\n\tvar/const/LIMIT = 9\n\tvar/static/S = 2\n\
                 /datum/base\n\
                 /datum/base/sub/proc/make() as /datum/a\n\treturn new /datum/a\n\
                 /datum/base/sub/proc/plain()\n\treturn new /datum/a\n\
                 /datum/base/sub/proc/named()\n\tset name = \"build\"\n\treturn new /datum/a\n\
                 /datum/base/sub/verb/vmake()\n\treturn new /datum/a\n";
    for (body, scope) in [
        ("return base?:make().LIMIT", "/datum/a"),
        ("return base?:plain().LIMIT", "/datum/b"),
        ("return base?:make().S", "/datum/a"),
        ("return base?:make()?.LIMIT", "/datum/a"),
        ("base?:make().S = 3", "/datum/a"),
    ] {
        let src = format!("{types}/proc/f(datum/base/base)\n\t{body}\n");
        assert_eq!(
            global_scopes(&src),
            [format!("Some(Type(\"{}\"))", scope)],
            "{}",
            body
        );
    }
    for (body, call) in [
        ("return base?:named()", "Call dynamic_proc(\"build\") 0"),
        (
            "return base?:vmake().LIMIT",
            "Call dynamic_verb(\"vmake\") 0",
        ),
    ] {
        let code = compile(
            &format!("{types}/proc/f(datum/base/base)\n\t{body}\n"),
            "",
            "f",
        );
        assert_has(&code, &[call]);
    }
}

#[test]
fn a_safe_colon_call_on_an_untyped_value_finds_its_proc_as_a_colon_call_does() {
    let types = "/mob/verb/show()\n\tset name = \"Display\"\n\
                 /datum/thing/proc/show_proc()\n\tset name = \"Show It\"\n\
                 /mob/verb/plain_verb()\n";
    for (body, call) in [
        ("target?:show()", "Call dynamic_verb(\"Display\") 0"),
        (
            "target:show()",
            "CallStatement cache = arg(0); dynamic_verb(\"Display\") 0",
        ),
        ("target?:show_proc()", "Call dynamic_proc(\"Show It\") 0"),
        (
            "return target?:show_proc()",
            "Call dynamic_proc(\"Show It\") 0",
        ),
        (
            "target?:plain_verb()",
            "Call dynamic_verb(\"plain verb\") 0",
        ),
        ("target?:show().x = 1", "Call dynamic_verb(\"Display\") 0"),
    ] {
        let code = compile(&format!("{types}/proc/f(target)\n\t{body}\n"), "", "f");
        assert_has(&code, &[call]);
    }
}

#[test]
fn a_colon_call_finds_its_proc_as_the_last_type_made_declares_it() {
    let types = "/datum/t1\n\tvar/const/LIMIT = 1\n\tvar/x\n\
                 /datum/t2\n\tvar/LIMIT = 2\n\tvar/x\n\
                 /datum/zz\n/datum/aa\n\
                 /datum/zz/proc/make() as /datum/t1\n\tset name = \"zz\"\n\
                 /datum/aa/proc/make() as /datum/t2\n\tset name = \"aa\"\n\
                 /datum/holder/proc/act()\n\tset name = \"holder act\"\n\
                 /mob/verb/act()\n\tset name = \"mob act\"\n\
                 /datum/base\n/datum/base/s2\n/datum/base/s1\n\
                 /datum/base/s1/proc/build()\n\tset name = \"s1\"\n\
                 /datum/base/s2/proc/build()\n\tset name = \"s2\"\n\
                 /datum/other/proc/build()\n\tset name = \"other\"\n\
                 /datum/base/proc/own()\n\tset name = \"base own\"\n\
                 /datum/other/proc/own()\n\tset name = \"other own\"\n";
    for (body, call) in [
        ("return x:make()", "dynamic_proc(\"aa\") 0"),
        ("return x?:make()", "dynamic_proc(\"aa\") 0"),
        ("return items[1].make()", "dynamic_proc(\"aa\") 0"),
        ("return x:act()", "dynamic_verb(\"mob act\") 0"),
        ("return base:build()", "dynamic_proc(\"s1\") 0"),
        ("return base?:build()", "dynamic_proc(\"other\") 0"),
        ("return base:own()", "dynamic_proc(\"base own\") 0"),
        ("return base?:own()", "dynamic_proc(\"other own\") 0"),
        ("return items:make()", "dynamic_proc(\"aa\") 0"),
        ("base?:own().x = 1", "dynamic_proc(\"other own\") 0"),
        ("x:make()?:own().x = 1", "dynamic_proc(\"other own\") 0"),
    ] {
        let code = compile(
            &format!("{types}/proc/f(x, datum/base/base, list/items)\n\t{body}\n"),
            "",
            "f",
        );
        assert!(
            code.contains(call),
            "{}: expected {} in\n{}",
            body,
            call,
            code
        );
    }
    for body in ["return x:make().LIMIT", "return x?:make()?.LIMIT"] {
        let code = compile(&format!("{types}/proc/f(x)\n\t{body}\n"), "", "f");
        assert!(
            code.contains("cache[\"LIMIT\"]"),
            "{}: expected /datum/t2's LIMIT, a field, in\n{}",
            body,
            code
        );
    }
}

#[test]
fn a_safe_colon_field_finds_its_var_as_on_an_untyped_value() {
    let types = "/datum/base\n\tvar/LIMIT = 1\n\
                 /datum/later\n\tvar/static/LIMIT = 2\n\
                 /datum/base/sub\n";
    for (body, var) in [
        ("return base?:LIMIT", "GetVar global(\"LIMIT\""),
        ("base?:LIMIT = 5", "SetVar global(\"LIMIT\""),
        ("base?:LIMIT += 1", "global(\"LIMIT\""),
        ("return base:LIMIT", "cache[\"LIMIT\"]"),
        ("return base?.LIMIT", "GetVar cache[\"LIMIT\"]"),
    ] {
        let code = compile(
            &format!("{types}/proc/f(datum/base/sub/base)\n\t{body}\n"),
            "",
            "f",
        );
        assert!(
            code.contains(var),
            "{}: expected {} in\n{}",
            body,
            var,
            code
        );
    }
}

#[test]
fn a_call_after_a_safe_index_keeps_the_indexed_item_as_its_receiver() {
    let types = "/datum/item/proc/refresh(a)\n\treturn a\n";
    for (body, call) in [
        (
            "return items?[1]:refresh()",
            "Call dynamic_proc(\"refresh\") 0",
        ),
        (
            "items?[1]:refresh(x)",
            "GetVar arg(1)\nPopCache\nCall dynamic_proc(\"refresh\") 1",
        ),
        (
            "return items?[1].refresh(items.len)",
            "PopCache\nCall dynamic_proc(\"refresh\") 1",
        ),
    ] {
        let code = compile(
            &format!("{types}/proc/f(list/items, x)\n\t{body}\n"),
            "",
            "f",
        );
        assert_has(&code, &["ListGet", "SetVar cache", "PushCache"]);
        assert_has(&code, &[call, "PopCache"]);
    }
}

#[test]
fn new_of_a_field_chain_reads_it_by_the_declared_type() {
    let types = "/datum/item\n/datum/item/special\n\
                 /datum/config\n\tvar/static/item_type = /datum/item/special\n\
                 \tvar/const/CONST_TYPE = /datum/item/special\n\
                 \tvar/plain_type = /datum/item\n\tvar/datum/config/inner\n";
    for body in [
        "return new config.item_type()",
        "return new config.item_type(1)",
        "return new config.inner.item_type()",
        "return new config.CONST_TYPE()",
    ] {
        let src = format!("{types}/proc/f(datum/config/config)\n\t{body}\n");
        assert_eq!(
            global_scopes(&src),
            ["Some(Type(\"/datum/config\"))"],
            "{}",
            body
        );
    }
    let code = compile(
        &format!("{types}/datum/config/proc/f()\n\treturn new src.item_type()\n"),
        "/datum/config",
        "f",
    );
    assert_has(&code, &["GetVar global(\"item_type\")", "New 0"]);
    let code = compile(
        &format!("{types}/proc/f(datum/config/config)\n\treturn new config.plain_type()\n"),
        "",
        "f",
    );
    assert_has(
        &code,
        &["GetVar cache = arg(0); cache[\"plain_type\"]", "New 0"],
    );
}

#[test]
fn a_safe_place_after_a_call_is_typed_as_a_read_finds_it() {
    let types = "/datum/child\n/datum/child/special\n\
                 /datum/holder\n\tvar/datum/child/child\n\tvar/datum/holder/inner\n\
                 /datum/other\n\tvar/datum/child/special/child\n\
                 /datum/maker/proc/untyped_holder()\n\treturn new /datum/holder\n\
                 /proc/get_holder() as /datum/holder\n\treturn new /datum/holder\n";
    for (body, made) in [
        ("get_holder()?.child = new", "/datum/child"),
        (
            "maker.untyped_holder()?.child = new",
            "/datum/child/special",
        ),
        ("get_holder()?:child = new", "/datum/child/special"),
        ("get_holder().inner?.child = new", "/datum/child"),
        ("get_holder()?.child = astype(x)", "/datum/child"),
        ("return astype(get_holder().child)", "/datum/child"),
    ] {
        let code = compile(
            &format!("{types}/proc/f(datum/maker/maker, x)\n\t{body}\n"),
            "",
            "f",
        );
        assert_has(&code, &[&format!("PushVal {made}")]);
    }
    for body in [
        "get_holder().child = new",
        "get_holder().inner.child = new",
        "maker?.untyped_holder().child = new",
    ] {
        assert_eq!(
            compile_error(&format!("{types}/proc/g(datum/maker/maker)\n\t{body}\n")),
            "line 13: implicit new() calls are not supported",
            "{}",
            body
        );
    }
}

#[test]
fn a_var_after_an_index_is_found_as_untyped_colon_finds_it() {
    let types = "/datum/a\n\tvar/const/LIMIT = 5\n\tvar/datum/a/link\n\
                 /datum/b\n\tvar/const/LIMIT = 9\n\tvar/datum/b/link\n";
    for read in [
        "var/list/datum/a/items = list()\n\treturn items[1]:LIMIT",
        "var/list/datum/a/items = list()\n\treturn items[1].LIMIT",
        "var/list/items = list()\n\treturn items[1].LIMIT",
        "var/list/items = list()\n\treturn items[1]:link:LIMIT",
        "var/list/items = list()\n\treturn items[1].link.LIMIT",
    ] {
        let src = format!("{types}/proc/f()\n\t{read}\n");
        let ctx = dreammaker::Context::default();
        let pp = dreammaker::Preprocessor::from_buffer(&ctx, "test.dm".into(), src);
        let mut parser = dreammaker::Parser::new(&ctx, pp);
        parser.enable_procs();
        let tree = parser.parse_object_tree();
        let value = tree
            .root()
            .get()
            .procs
            .get("f")
            .unwrap()
            .value
            .last()
            .unwrap();
        let source = ProcSource {
            tree: &tree,
            ty: tree.root(),
            name: "f",
            value,
            file: "test.dm",
            debug_info: false,
            index: None,
            byond: dmasm_compiler::ByondVersion::LATEST,
            file_names: None,
        };
        let nodes = compile_proc(&source).expect("compiles").nodes;
        let code = dmasm_bytecode::format(&nodes);
        let code: Vec<&str> = code.lines().map(str::trim).collect();
        assert!(
            code.ends_with(&[
                "ListGet",
                "SetVar cache",
                "GetVar global(\"LIMIT\")",
                "Ret",
                "End"
            ]),
            "{}: {:?}",
            read,
            code
        );
        let read_from: Vec<String> = nodes
            .iter()
            .filter_map(|node| match node {
                Node::Instruction(Instruction::GetVar(Variable::Global(g)), _) => {
                    Some(format!("{:?}", g.scope))
                }
                _ => None,
            })
            .collect();
        assert_eq!(read_from, ["Some(Type(\"/datum/b\"))"], "{}", read);
    }
}

#[test]
fn static_after_double_colon_chain() {
    let src = "/datum/a\n\tvar/datum/b/b\n/datum/b\n\tvar/const/C = 3\nvar/datum/a/A\n/proc/f()\n\treturn A::b::C\n";
    let code = compile(src, "", "f");
    assert_has(&code, &["GetVar global(\"C\")", "Ret"]);
    assert!(!code.contains("global(\"A\")"), "{}", code);
}

#[test]
fn del_clears_its_target() {
    let src = "/datum/q/proc/f()\n\tvar/datum/x = new\n\tdel(x)\n\tdel src\n";
    let code = compile(src, "/datum/q", "f");
    assert_has(
        &code,
        &["GetVar local(0)", "PushVal null", "SetVar local(0)", "Del"],
    );
    assert_has(
        &code,
        &["GetVar src", "PushVal null", "SetVar src", "Del", "End"],
    );
}

#[test]
fn vars_after_a_switch_start_null() {
    let src =
        "/proc/f(a)\n\tvar/x\n\tswitch(a)\n\t\tif(1)\n\t\t\treturn\n\tvar/y\n\treturn list(x, y)\n";
    let code = compile(src, "", "f");
    assert_eq!(code.matches("PushVal null").count(), 1, "{}", code);
    assert_has(&code, &["PushVal null", "SetVar local(1)"]);
}

#[test]
fn alist_makes_an_alist() {
    let code = compile("/proc/f(a)\n\treturn alist(1 = a)\n", "", "f");
    assert_has(&code, &["PushInt 1", "GetVar arg(0)", "NewAlist 1"]);
}

#[test]
fn input_from_builtin_sources() {
    let src = "/proc/f(a, L)\n\t. = input(a) as num\n\t. = input(a) as mob in world\n\t. = input(a) in L\n";
    let code = compile(src, "", "f");
    assert_has(&code, &["Input 8 0 0", "PromptCheck"]);
    assert_has(&code, &["Input 1 127 16", "PromptCheck"]);
    assert_has(
        &code,
        &[
            "GetVar arg(1)",
            "GetVar arg(0)",
            "PushVal null",
            "PushVal null",
            "PushVal null",
            "Input 0 0 64",
        ],
    );
}

#[test]
fn safe_access_assignment() {
    let src = "/datum/q\n\tvar/n\n/proc/f(datum/q/a, x)\n\ta?.n = x\n";
    let code = compile(src, "", "f");
    assert_has(
        &code,
        &[
            "GetVar arg(0)",
            "SetCachePopJmpIfNull LAB_0001",
            "PushCache",
            "GetVar arg(1)",
            "PopCache",
            "SetVar cache[\"n\"]",
        ],
    );
}

#[test]
fn or_assign_field_saves_cache_around_value() {
    let src = "/datum/q\n\tvar/x\n\tvar/y\n/proc/f(datum/q/o)\n\to.y ||= o.x\n";
    let code = compile(src, "", "f");
    assert_has(
        &code,
        &[
            "Test",
            "Jnz LAB_0000",
            "PushCache",
            "GetVar cache[\"x\"]",
            "PopCache",
        ],
    );
}

#[test]
fn sized_list_var() {
    let code = compile("/proc/f(n)\n\tvar/list/L[n]\n\treturn L\n", "", "f");
    assert_has(&code, &["GetVar arg(0)", "EmptyList", "SetVar local(0)"]);
}

#[test]
fn verbs_are_called_by_display_name() {
    let src = "/mob/verb/stop_pulling()\n\tset name = \"Stop Pulling\"\n/proc/f(mob/m)\n\tm.stop_pulling()\n";
    let code = compile(src, "", "f");
    assert!(code.contains("dynamic_verb(\"Stop Pulling\")"), "{}", code);
}

#[test]
fn savefile_input_statement() {
    let code = compile("/proc/f(savefile/S, x)\n\tS[\"k\"] >> x\n", "", "f");
    assert_has(
        &code,
        &[
            "GetVar arg(0)",
            "PushVal \"k\"",
            "Index",
            "Read",
            "SetVar arg(1)",
        ],
    );
}

#[test]
fn a_block_local_const_is_folded_only_in_its_block() {
    let src = "var/const/LIMIT = 10\n/proc/f(x)\n\tif(x)\n\t\tvar/const/LIMIT = 2\n\t\tx = LIMIT + 1\n\treturn LIMIT + 1\n";
    let code = compile(src, "", "f");
    assert_has(&code, &["PushInt 3", "SetVar arg(0)"]);
    assert_has(&code, &["PushInt 11", "Ret"]);
}

#[test]
fn same_named_static_locals_in_one_proc_are_different_globals() {
    let src = "/proc/f(a)\n\tif(a)\n\t\tvar/static/x = 1\n\t\tworld << x\n\telse\n\t\tvar/static/x = 2\n\t\tworld << x\n";
    let ctx = dreammaker::Context::default();
    let pp = dreammaker::Preprocessor::from_buffer(&ctx, "test.dm".into(), src.to_owned());
    let mut parser = dreammaker::Parser::new(&ctx, pp);
    parser.enable_procs();
    let tree = parser.parse_object_tree();
    let value = tree
        .root()
        .get()
        .procs
        .get("f")
        .unwrap()
        .value
        .last()
        .unwrap();
    let source = ProcSource {
        tree: &tree,
        ty: tree.root(),
        name: "f",
        value,
        file: "test.dm",
        debug_info: false,
        index: None,
        byond: dmasm_compiler::ByondVersion::LATEST,
        file_names: None,
    };
    let compiled = compile_proc(&source).unwrap();
    let scopes: Vec<Option<dmasm_bytecode::StaticScope>> = compiled
        .nodes
        .iter()
        .filter_map(|n| match n {
            dmasm_bytecode::Node::Instruction(
                dmasm_bytecode::Instruction::GetVar(dmasm_bytecode::Variable::Global(g)),
                _,
            ) => Some(g.scope.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        scopes,
        vec![
            Some(dmasm_bytecode::StaticScope::Proc {
                path: "/proc/f".to_owned(),
                nth: 0
            }),
            Some(dmasm_bytecode::StaticScope::Proc {
                path: "/proc/f".to_owned(),
                nth: 1
            })
        ]
    );
}

#[test]
fn chain_reloaded_after_call() {
    let code = compile(
        "/datum/var/datum/m\n/datum/var/list/L\n/datum/proc/g()\n/proc/f(datum/a)\n\ta.m.g()\n\tfor (var/x in a.m.L)\n\t\treturn x\n",
        "",
        "f",
    );
    assert_has(
        &code,
        &["GetVar cache = arg(0); cache = cache[\"m\"]; cache[\"L\"]"],
    );
}

#[test]
fn type_proc_reference_override() {
    let code = compile(
        "/atom/proc/p()\n/obj/a/p()\n/obj/a/b\n/proc/f()\n\treturn /obj/a/b::p()\n",
        "",
        "f",
    );
    assert_has(&code, &["PushVal /obj/a/p"]);
}

#[test]
fn type_proc_reference_is_the_types_last_definition() {
    for (code, pushed) in [
        (
            "/datum/t/proc/f()\n/proc/g()\n\treturn /datum/t::f()\n",
            "PushVal /datum/t/proc/f",
        ),
        (
            "/datum/t/proc/f()\n/datum/t/f()\n/datum/t/c\n/proc/g()\n\treturn /datum/t/c::f()\n",
            "PushVal /datum/t/f",
        ),
        (
            "/datum/t/f()\n/datum/t/proc/f()\n/proc/g()\n\treturn /datum/t::f()\n",
            "PushVal /datum/t/proc/f",
        ),
        (
            "/datum/t/proc/h()\n/datum/t/f()\n/datum/t/proc/f()\n/proc/g()\n\treturn /datum/t::f()\n",
            "PushVal /datum/t/f",
        ),
        (
            "/icon/f()\n/icon/proc/f()\n/proc/g()\n\treturn /icon::f()\n",
            "PushVal /icon/f",
        ),
    ] {
        assert_has(&compile(code, "", "g"), &[pushed]);
    }
}

#[test]
fn type_proc_reference_to_a_verb_is_its_override() {
    for code in [
        "/mob/verb/choose()\n/mob/choose()\n/mob/m\n/proc/g()\n\treturn /mob/m::choose()\n",
        "/mob/choose()\n/mob/verb/choose()\n/proc/g()\n\treturn /mob::choose()\n",
    ] {
        assert_has(&compile(code, "", "g"), &["PushVal /mob/choose"]);
    }
}

#[test]
fn type_proc_reference_to_a_later_override_is_refused() {
    let code = "/datum/t/proc/f()\n/datum/t/f()\n/datum/t/f()\n/proc/g()\n\treturn /datum/t::f()\n";
    assert_eq!(
        compile_error(code),
        "line 5: unsupported statement: type::proc() that names a type's second or later override"
    );
}

#[test]
fn type_proc_reference_to_a_verb_without_an_override_is_refused() {
    let code = "/mob/verb/choose()\n/mob/m\n/proc/g()\n\treturn /mob/m::choose()\n";
    assert_eq!(
        compile_error(code),
        "line 4: bad proc reference: choose is a verb with no override on /mob"
    );
}

fn compile_error(code: &str) -> String {
    let ctx = dreammaker::Context::default();
    let pp = dreammaker::Preprocessor::from_buffer(&ctx, "test.dm".into(), code.to_owned());
    let mut parser = dreammaker::Parser::new(&ctx, pp);
    parser.enable_procs();
    let tree = parser.parse_object_tree();
    let value = &tree.root().get().procs["g"].value[0];
    let source = ProcSource {
        tree: &tree,
        ty: tree.root(),
        name: "g",
        value,
        file: "test.dm",
        debug_info: false,
        index: None,
        byond: dmasm_compiler::ByondVersion::LATEST,
        file_names: None,
    };
    compile_proc(&source).unwrap_err().to_string()
}

#[test]
fn var_in_clause() {
    let code = compile(
        "/proc/f(a)\n\tvar/x = a in list(1, 2)\n\treturn x\n",
        "",
        "f",
    );
    assert_has(&code, &["GetVar arg(0)", "SetVar local(0)"]);
}

#[test]
fn bit_not_of_huge_number() {
    let code = compile("/proc/f()\n\treturn ~(65536 ** 2)\n", "", "f");
    assert_has(&code, &["PushVal 16777215"]);
}

#[test]
fn builtin_const_not_folded() {
    let code = compile(
        "/atom/proc/f()\n\treturn loc?.type || \"x\"\n",
        "/atom",
        "f",
    );
    assert!(code.contains("SetCacheJmpIfNull"), "{}", code);
}

#[test]
fn chain_kept_after_call() {
    let code = compile(
        "/datum/var/datum/m\n/datum/var/x\n/datum/proc/g()\n/proc/f(datum/a)\n\ta.m.g()\n\ta.m.g()\n\t. = a.m.x\n\t. = a.m.x\n",
        "",
        "f",
    );
    assert_has(
        &code,
        &[
            "CallStatement cache = arg(0); cache = cache[\"m\"]; dynamic_proc(\"g\") 0",
            "Pop",
            "CallStatement dynamic_proc(\"g\") 0",
            "Pop",
            "GetVar cache[\"x\"]",
            "SetVar dot",
            "GetVar cache = arg(0); cache = cache[\"m\"]; cache[\"x\"]",
        ],
    );
}

#[test]
fn safe_src_is_not_an_explicit_src_chain() {
    let code = compile(
        "/mob/var/mob/m\n/mob/var/x\n/mob/var/y\n/mob/var/list/t\n/mob/proc/f()\n\tif(src?.t)\n\t\treturn 0\n\tvar/g = m.x\n\treturn m.y\n",
        "/mob",
        "f",
    );
    assert_has(
        &code,
        &[
            "GetVar cache = src; cache = cache[\"m\"]; cache[\"x\"]",
            "SetVar local(0)",
            "GetVar cache[\"y\"]",
        ],
    );
}

#[test]
fn a_logical_operator_folded_to_its_first_value_forgets_the_cache() {
    let code = |value: &str| {
        compile(
            &format!("/datum/var/datum/m\n/datum/var/x\n/datum/var/y\n/proc/f(datum/a, b)\n\tif(a.m.x)\n\t\tvar/c = ({value})\n\t\treturn a.m.y\n"),
            "",
            "f",
        )
    };
    for kept in ["5", "\"s\"", "b"] {
        assert_has(&code(kept), &["GetVar cache[\"y\"]", "Ret"]);
    }
    for forgotten in [
        "1 || b",
        "\"s\" || b",
        "\"\" && b",
        "null && b",
        "/datum || 1",
    ] {
        assert_has(
            &code(forgotten),
            &[
                "GetVar cache = arg(0); cache = cache[\"m\"]; cache[\"y\"]",
                "Ret",
            ],
        );
    }
}

#[test]
fn a_for_with_an_empty_first_part_is_read_past_it() {
    let code = |head: &str| {
        compile(
            &format!("/proc/f(a, b)\n\tfor ({head})\n\t\tb++\n\treturn b\n"),
            "",
            "f",
        )
    };
    let shifted = code("; a < 5; a++");
    assert_has(&shifted, &["GetVar arg(0)", "PushInt 5", "Tl", "Pop"]);
    assert_has(&shifted, &["PostInc arg(0)", "Test", "Jz"]);
    let plain = code(", a < 5, a++");
    assert_has(&plain, &["GetVar arg(0)", "PushInt 5", "Tl", "Test", "Jz"]);
    assert!(plain.contains("\nInc arg(0)\nJmpLoop"), "{plain}");
}

#[test]
fn a_global_const_named_through_global_folds() {
    let code = |body: &str| {
        compile(
            &format!(
                "var/const/gc = 7\n/datum/d\n\tvar/const/gc = 9\n/datum/d/proc/f(a)\n\t{body}\n"
            ),
            "/datum/d",
            "f",
        )
    };
    assert_has(&code("return global.gc + 1"), &["PushInt 8", "Ret"]);
    assert_has(&code("return ::gc + 1"), &["PushInt 8", "Ret"]);
    assert_has(&code("return gc + 1"), &["PushInt 10", "Ret"]);
    assert_has(&code("return global.gc ? a : 2"), &["GetVar arg(0)", "Ret"]);
    assert_has(&code("return global.gc"), &["GetVar global(\"gc\")", "Ret"]);
    assert!(code("switch (a)\n\t\tif (global.gc)\n\t\t\treturn 1").contains("Switch default => "));
}

#[test]
fn an_implicit_new_for_a_list_of_a_type() {
    let code = compile(
        "/datum/d/var/datum/d/list/dl\n/proc/f(datum/d/o)\n\tvar/datum/d/list/v = new\n\tvar/datum/d/list/w = new(3)\n\to.dl = new\n\treturn v\n",
        "",
        "f",
    );
    assert_has(&code, &["PushVal /list", "New 0", "SetVar local(0)"]);
    assert_has(
        &code,
        &["PushVal /list", "PushInt 3", "New 1", "SetVar local(1)"],
    );
    assert_has(
        &code,
        &[
            "PushVal /list",
            "New 0",
            "SetVar cache = arg(0); cache[\"dl\"]",
        ],
    );
}

#[test]
fn rarer_forms_of_builtins_on_their_own_line() {
    let code = |body: &str| {
        compile(
            &format!("/proc/f(a, b, c)\n\t{body}\n\treturn a\n"),
            "",
            "f",
        )
    };
    assert_has(
        &code("statpanel(a, b)"),
        &[
            "GetVar arg(0)",
            "PushVal null",
            "GetVar arg(1)",
            "StatPanel",
            "GetVar arg(0)",
            "Ret",
        ],
    );
    assert_has(
        &code("statpanel(a, b, c)"),
        &[
            "GetVar arg(0)",
            "GetVar arg(1)",
            "GetVar arg(2)",
            "StatPanel",
            "GetVar arg(0)",
            "Ret",
        ],
    );
    assert_has(
        &code("output(a, b)"),
        &[
            "GetVar arg(0)",
            "Pop",
            "GetVar arg(1)",
            "Pop",
            "GetVar arg(0)",
            "Ret",
        ],
    );
    assert_has(
        &code("animate(a, b, c)"),
        &[
            "GetVar arg(0)",
            "NullAnimate",
            "Pop",
            "GetVar arg(0)",
            "Ret",
        ],
    );
}

#[test]
fn a_line_that_does_nothing() {
    let code = |body: &str| {
        compile(
            &format!("/datum/var/x\n/proc/f(datum/a, b)\n\t{body}\n\treturn b\n"),
            "",
            "f",
        )
    };
    for nothing in ["a", "(a)", "a.x", "usr", "null"] {
        assert_eq!(code(nothing).trim(), "GetVar arg(1)\nRet\nEnd", "{nothing}");
    }
    assert_has(
        &code("isarea(a)"),
        &["GetVar arg(0)", "IsArea", "GetVar arg(1)", "Ret"],
    );
    assert_has(
        &code("a == b"),
        &["Teq", "Pop", "GetFlag", "Pop", "GetVar arg(1)", "Ret"],
    );
    assert_has(
        &code("isnull(a)"),
        &["GetVar arg(0)", "IsNull", "Pop", "GetVar arg(1)", "Ret"],
    );
}

#[test]
fn a_variable_times_itself_is_squared() {
    let code = |expr: &str| compile(&format!("var/g\n/proc/f(a)\n\treturn {expr}\n"), "", "f");
    assert_has(&code("a * (a)"), &["GetVar arg(0)", "PowSquare", "Ret"]);
    assert_has(
        &code("g * global.g"),
        &["GetVar global(\"g\")", "PowSquare", "Ret"],
    );
    assert_has(
        &code("global.g * g"),
        &["GetVar global(\"g\")", "PowSquare", "Ret"],
    );
    let shadowed = compile(
        "var/g\n/proc/f(a)\n\tvar/g = a\n\treturn g * global.g\n",
        "",
        "f",
    );
    assert!(!shadowed.contains("PowSquare"), "{}", shadowed);
}

#[test]
fn a_square_left_of_a_product() {
    let code = |expr: &str| {
        compile(
            &format!("/proc/g(x)\n/proc/f(a, b, list/L)\n\tvar/v = {expr}\n\treturn v\n"),
            "",
            "f",
        )
    };
    for expr in ["a * a * b", "(a * a) * b", "((a * a)) * b"] {
        let code = code(expr);
        assert!(!code.contains("PowSquare"), "{}:\n{}", expr, code);
    }
    for expr in [
        "g(a * a) * b",
        "L[a * a] * b",
        "list(a * a)[1] * b",
        "(b ? a * a : 1) * b",
        "-(a * a) * b",
    ] {
        let code = code(expr);
        assert!(
            code.contains("GetVar arg(0)\nPowSquare"),
            "{}: expected a square of a in\n{}",
            expr,
            code
        );
    }
}

#[test]
fn a_pick_of_weight_0_and_a_case_of_minus_0() {
    let v = |build| dmasm_compiler::ByondVersion::new(516, build);
    let middle = "/proc/f(a, b)\n\treturn pick(1; a, 0; b, 2; \"c\")\n";
    let last = "/proc/f(a, b)\n\treturn pick(1; a, 0; b)\n";
    let table = |code: &str| {
        code.lines()
            .find(|l| l.starts_with("PickSwitch"))
            .unwrap()
            .to_owned()
    };

    let code = compile_on(middle, "", "f", v(1689));
    let cases = table(&code);
    assert_eq!(cases.matches("=>").count(), 3, "{}", cases);
    assert_eq!(cases.matches("21845 =>").count(), 2, "{}", cases);
    let code = compile_on(last, "", "f", v(1689));
    let cases = table(&code);
    assert_eq!(cases.matches("=>").count(), 2, "{}", cases);
    assert!(cases.contains("65535 =>"), "{}", cases);
    let target = cases
        .split("default => ")
        .nth(1)
        .unwrap()
        .split(',')
        .next()
        .unwrap()
        .trim();
    assert_has(&code, &[&format!("{target}:"), "GetVar arg(1)"]);
    assert_eq!(warnings_for(last, v(1689)), [] as [String; 0]);
    for build in [1688, 1689] {
        let code = compile_on(
            "/proc/f(a, b)\n\treturn pick(\"x\"; a, 3; b)\n",
            "",
            "f",
            v(build),
        );
        assert!(table(&code).contains(" 0 =>"), "{}", code);
    }

    let code = compile_on(middle, "", "f", v(1688));
    let cases = table(&code);
    assert_eq!(cases.matches("=>").count(), 2, "{}", cases);
    assert!(cases.contains("21845 =>"), "{}", cases);
    assert!(code.contains("GetVar arg(1)"), "{}", code);
    let code = compile_on(last, "", "f", v(1688));
    let cases = table(&code);
    assert_eq!(cases.matches("=>").count(), 1, "{}", cases);
    let target = cases
        .split("default => ")
        .nth(1)
        .unwrap()
        .trim_end_matches([',', ' ']);
    assert_has(&code, &[&format!("{target}:"), "GetVar arg(0)"]);
    let warnings = warnings_for(last, v(1688));
    assert!(
        warnings.iter().any(|w| w.contains("zero-weight entry")),
        "{:?}",
        warnings
    );
    let code = compile(
        "/proc/f(a)\n\tswitch (a)\n\t\tif (-0)\n\t\t\treturn 1\n\treturn 2\n",
        "",
        "f",
    );
    let table = code.lines().find(|l| l.starts_with("Switch")).unwrap();
    assert!(
        table.contains(" 0 =>") && !table.contains("-0"),
        "{}",
        table
    );
}

#[test]
fn a_parameter_default_stops_at_in() {
    let code = compile(
        "/proc/f(a, b = a in list(1), c = (a in list(1)))\n\treturn b\n",
        "",
        "f",
    );
    assert_has(&code, &["GetVar arg(0)", "SetVar arg(1)"]);
    assert!(code.contains("IsIn"), "{code}");
}

#[test]
fn a_chain_through_fields_isnt_reused_after_a_change_as_a_value() {
    let code = |body: &str| {
        compile(
            &format!("/datum/var/datum/m\n/datum/var/x\n/proc/f(datum/a, b)\n\t{body}\n"),
            "",
            "f",
        )
    };
    let whole = "cache = arg(0); cache = cache[\"m\"]; cache[\"x\"]";
    assert_has(
        &code("return a.m.x++ + a.m.x"),
        &[
            &format!("PostInc {whole}"),
            &format!("GetVar {whole}"),
            "Add",
        ],
    );
    assert_has(
        &code("return (a.m.x += b) + a.m.x"),
        &["PushEval", &format!("GetVar {whole}"), "Add"],
    );
    assert_has(
        &code("return (a.m.x ||= b)"),
        &[
            &format!("GetVar {whole}"),
            "JmpOr LAB_0002",
            "GetVar arg(1)",
            &format!("SetVarExpr {whole}"),
        ],
    );
    assert_has(
        &code("return a.m.x + (a.m.x = b)"),
        &["GetVar arg(1)", "SetVarExpr cache[\"x\"]", "Add"],
    );
}

#[test]
fn a_logical_assignment_writes_its_field_as_it_read_it() {
    let code = |body: &str| {
        compile(
            &format!("/datum/d\n\tvar/x\n\tvar/y\n/datum/d/proc/f(b)\n\t{body}\n"),
            "/datum/d",
            "f",
        )
    };
    assert_has(
        &code("src.y = 1\n\tsrc.x &&= b"),
        &[
            "GetVar cache[\"x\"]",
            "Test",
            "Jz LAB_0001",
            "GetVar arg(0)",
            "SetVar cache[\"x\"]",
        ],
    );
    assert_has(
        &code("y = 1\n\tsrc.x &&= b"),
        &[
            "GetVar cache = src; cache[\"x\"]",
            "Test",
            "Jz LAB_0001",
            "GetVar arg(0)",
            "SetVar cache = src; cache[\"x\"]",
        ],
    );
    assert_has(
        &code("src.y = 1\n\tx ||= b"),
        &[
            "GetVar cache = src; cache[\"x\"]",
            "Test",
            "Jnz LAB_0001",
            "GetVar arg(0)",
            "SetVar cache = src; cache[\"x\"]",
        ],
    );
}

#[test]
fn dropped_constant_forgets_cache() {
    let code = compile(
        "/datum/var/datum/m\n/datum/var/x\n/datum/var/y\n/proc/f(datum/a)\n\tif(a.m.x)\n\t\treturn (0 || a.m.y)\n",
        "",
        "f",
    );
    assert_has(
        &code,
        &[
            "GetVar cache = arg(0); cache = cache[\"m\"]; cache[\"y\"]",
            "Ret",
        ],
    );
}

#[test]
fn datum_loop_in_world_goes_over_atoms() {
    let code = compile(
        "/datum/foo\n/proc/f()\n\tfor (var/datum/foo/x in world)\n\t\tworld.log << x\n\tfor (var/datum/foo/y)\n\t\tworld.log << y\n",
        "",
        "f",
    );
    assert_has(&code, &["GetVar world", "IterLoad 5 ()"]);
    assert_has(
        &code,
        &["PushVal /datum/foo", "IterLoad 5 (datum_instances | )"],
    );
}

#[test]
fn text_squared_is_pow_square() {
    let code = compile(
        "/proc/f(x)\n\t. = \"a\" * \"a\"\n\treturn \"[x]\" * \"[x]\"\n",
        "",
        "f",
    );
    assert_has(&code, &["PushVal \"a\"", "PowSquare"]);
    assert_has(&code, &["Format \"[]\" 1", "Mul"]);
}

#[test]
fn bit_ops_fold_like_dreammaker() {
    let code = compile(
        "/proc/f()\n\t. = ~(65536**3)\n\t. = ~(2**31+256)\n\t. = ~(-1)\n\t. = (2**33) & 16777215\n\t. = (-5) | 0\n\treturn (2**31+256) >> 8\n",
        "",
        "f",
    );
    assert_has(
        &code,
        &[
            "PushInt 0",
            "SetVar dot",
            "PushVal 16776959",
            "SetVar dot",
            "PushVal 16777215",
            "SetVar dot",
            "PushVal 16777215",
            "SetVar dot",
            "PushVal 16777211",
            "SetVar dot",
            "PushInt 65535",
        ],
    );
}

#[test]
fn cmptext_with_more_than_two() {
    let code = compile(
        "/proc/f()\n\treturn cmptextEx(\"a\", \"A\", \"b\")\n",
        "",
        "f",
    );
    assert_has(
        &code,
        &[
            "PushVal \"a\"",
            "PushVal \"A\"",
            "Teq",
            "Jz LAB_0001",
            "PushVal \"b\"",
            "Teq",
            "LAB_0001:",
            "Pop",
            "GetFlag",
        ],
    );
    let code = compile("/proc/f(a)\n\treturn cmptext(a, \"A\", \"b\")\n", "", "f");
    assert_has(
        &code,
        &["CmpText", "Jz LAB_0001", "PushVal \"b\"", "CmpText"],
    );
}

#[test]
fn call_ext_of_a_loaded_function() {
    let src = "/proc/f()\n\tvar/h = load_ext(\"lib.so\", \"func\")\n\treturn call_ext(h)(1, 2)\n";
    let code = compile(src, "", "f");
    assert_has(
        &code,
        &["PushVal \"lib.so\"", "PushVal \"func\"", "LoadExt"],
    );
    assert_has(
        &code,
        &[
            "GetVar local(0)",
            "PushInt 1",
            "PushInt 2",
            "CallExtLoaded 2",
        ],
    );
}

#[test]
fn type_macro_in_a_global_proc_is_null() {
    let code = compile("/proc/f()\n\treturn __TYPE__\n", "", "f");
    assert_has(&code, &["PushVal null", "Ret"]);
}

#[test]
fn missile_leaves_nothing() {
    let code = compile("/proc/f()\n\tvar/atom/a\n\tmissile(/obj, a, a)\n", "", "f");
    assert_has(&code, &["GetVar local(0)", "Missile", "End"]);
}

#[test]
fn proc_argument_keys_worked_out() {
    let src = "/proc/g(...)\n/proc/f()\n\tvar/a = 0\n\treturn g(a + 1 = 2, 7)\n";
    let code = compile(src, "", "f");
    assert_has(
        &code,
        &[
            "Add",
            "PushInt 2",
            "PushInt 2",
            "PushInt 7",
            "NewAssocList 2",
            "CallGlobalArgList /proc/g",
        ],
    );
    let src =
        "/datum/t\n\tvar/b\n/proc/g(...)\n/proc/f()\n\tvar/datum/t/a = new\n\treturn g(a?.b = 1)\n";
    let code = compile(src, "", "f");
    assert_has(
        &code,
        &["SetVarExpr cache[\"b\"]", "LAB_0004:", "CallGlob 1 /proc/g"],
    );
}

#[test]
fn switch_with_no_cases() {
    let code = compile("/proc/f(a)\n\tswitch(a)\n\treturn 1\n", "", "f");
    assert_has(
        &code,
        &[
            "GetVar arg(0)",
            "Switch default => LAB_0001,",
            "LAB_0001:",
            "PushInt 1",
            "Ret",
        ],
    );
}

#[test]
fn parenthesised_assignment_is_a_list_key() {
    let code = compile("/proc/f(a, b, c)\n\treturn list(a, (b = 1), c)\n", "", "f");
    assert_has(
        &code,
        &[
            "PushInt 1",
            "GetVar arg(0)",
            "GetVar arg(1)",
            "PushInt 1",
            "PushInt 3",
            "GetVar arg(2)",
            "NewAssocList 3",
        ],
    );
    let code = compile("/proc/f(a, b, c)\n\treturn list((\"x\" = 1))\n", "", "f");
    assert_has(&code, &["PushVal \"x\"", "PushInt 1", "NewAssocList 1"]);
    let code = compile(
        "/proc/g(a, b)\n\treturn b\n/proc/f(a, b, c)\n\treturn g(a, (b = 1))\n",
        "",
        "f",
    );
    assert_has(
        &code,
        &[
            "GetVar arg(0)",
            "PushInt 1",
            "SetVarExpr arg(1)",
            "CallGlob 2 /proc/g",
        ],
    );
}

#[test]
fn writing_through_a_pointer() {
    let code = compile("/proc/f(a)\n\tvar/p = &a\n\t*p = 2\n\treturn a\n", "", "f");
    assert_has(&code, &["PushInt 2", "SetVar *local(0)", "GetVar arg(0)"]);
    let code = compile("/proc/f(a)\n\tvar/p = &a\n\t*p += 2\n\treturn a\n", "", "f");
    assert_has(&code, &["PushInt 2", "AugAdd *local(0)", "GetVar arg(0)"]);
    let code = compile("/proc/f(a)\n\tvar/p = &a\n\treturn (*p = 3)\n", "", "f");
    assert_has(&code, &["PushInt 3", "PushTop", "SetVar *local(0)", "Ret"]);
    let code = compile(
        "/proc/f(a)\n\tvar/p = &a\n\t(*p)++\n\t(*p) = 6\n\treturn a\n",
        "",
        "f",
    );
    assert_has(&code, &["Inc *local(0)", "PushInt 6", "SetVar *local(0)"]);
}

#[test]
fn pointer_to_a_field() {
    let src =
        "/datum/t\n\tvar/v = 1\n/proc/f()\n\tvar/datum/t/T = new\n\tvar/p = &T.v\n\treturn *p\n";
    assert_has(
        &compile(src, "", "f"),
        &["GetVar cache = local(0); &cache[\"v\"]", "SetVar local(1)"],
    );
    let src = "/datum/t\n\tvar/v = 1\n\tproc/m()\n\t\tvar/p = &src.v\n\t\treturn *p\n";
    assert_has(
        &compile(src, "/datum/t", "m"),
        &["GetVar cache = src; &cache[\"v\"]", "SetVar local(0)"],
    );
    let src = "/datum/t\n\tvar/v = 1\n/proc/f()\n\tvar/datum/t/T = new\n\tvar/p = &(T.v)\n\treturn *(&T.v)\n";
    assert_has(
        &compile(src, "", "f"),
        &[
            "GetVar cache = local(0); &cache[\"v\"]",
            "SetVar local(1)",
            "GetVar &cache[\"v\"]",
            "SetVar cache_key",
            "GetVar *cache_key",
        ],
    );
    assert_has(
        &compile("/proc/f(a)\n\tvar/p = &a\n\treturn *(p)\n", "", "f"),
        &["GetVar *local(0)", "Ret"],
    );
}

#[test]
fn reading_a_pointer_it_makes_warns_before_516_1668() {
    let src = "/datum/t\n\tvar/v = 1\n\tvar/datum/t/w\nvar/g = 1\n\
               /datum/t/proc/m()\n\treturn *(&src.v)\n\
               /proc/f(list/L)\n\tvar/datum/t/T = new\n\tvar/l = 1\n\
               \tT.v = *&T.w.v\n\treturn list(*(&l), *(&L[1]), *(&global.g))\n\
               /proc/w()\n\tvar/datum/t/T = new\n\t*(&T.v) = 3\n";
    let bugs = |ty: &str, name: &str, build: u32| -> Vec<String> {
        compiled_of(src, ty, name, dmasm_compiler::ByondVersion::new(516, build))
            .byond_bugs
            .into_iter()
            .map(|bug| bug.text)
            .collect()
    };
    let bug = "`*(&x)`: before 516.1668 DreamMaker reads `*x`, which dereferences x's value, a \
               \"bad pointer\" error unless x holds a pointer";
    assert_eq!(bugs("", "f", 1667), [bug]);
    assert_eq!(bugs("/datum/t", "m", 1667), [bug]);
    assert_eq!(bugs("", "w", 1667), [bug]);
    assert!(bugs("", "f", 1668).is_empty());
    assert!(bugs("/datum/t", "m", 1668).is_empty());
    assert_eq!(
        compile_on(src, "", "f", dmasm_compiler::ByondVersion::new(516, 1667)),
        compile(src, "", "f")
    );
    let held = "/proc/f(a)\n\tvar/p = &a\n\treturn *(p) + *p\n";
    assert!(
        compiled_of(held, "", "f", dmasm_compiler::ByondVersion::new(516, 1667))
            .byond_bugs
            .is_empty()
    );
}

#[test]
fn new_without_arguments_is_true_in_a_test() {
    let src = "/datum/t\n/proc/f(a, b)\n\treturn (new /datum/t ? a : b)\n";
    assert_eq!(compile(src, "", "f"), "GetVar arg(0)\nRet\nEnd");
    let src = "/datum/t\n/proc/f(a, b)\n\treturn !(new /datum/t()) || (new /datum/t && a)\n";
    assert_eq!(compile(src, "", "f"), "GetVar arg(0)\nRet\nEnd");
    let src = "/datum/t\n/proc/f(a, b)\n\treturn (new /datum/t || a)\n";
    assert_has(
        &compile(src, "", "f"),
        &["PushVal /datum/t", "New 0", "Ret"],
    );
    let src = "/datum/t\n/proc/f(a, b)\n\treturn (new /datum/t(a) ? a : b)\n";
    assert_has(
        &compile(src, "", "f"),
        &["PushVal /datum/t", "GetVar arg(0)", "New 1", "Test"],
    );
}

#[test]
fn writing_a_static_var_through_safe_access() {
    let src = "/datum/t\n\tvar/static/s = 2\n\tvar/datum/t/n\n/proc/f(datum/t/a, b)\n\ta?.s = b\n\ta?.n.s += b\n\tb = a?.s++\n";
    assert_has(
        &compile(src, "", "f"),
        &[
            "GetVar arg(0)",
            "SetCachePopJmpIfNull LAB_0001",
            "PushCache",
            "GetVar arg(1)",
            "PopCache",
            "SetVar global(\"s\")",
            "LAB_0001:",
            "GetVar arg(0)",
            "SetCachePopJmpIfNull LAB_0004",
            "PushCache",
            "GetVar arg(1)",
            "PopCache",
            "AugAdd global(\"s\")",
            "LAB_0004:",
            "GetVar arg(0)",
            "SetCacheJmpIfNull LAB_0007",
            "PushCache",
            "PopCache",
            "PostInc global(\"s\")",
        ],
    );
}

#[test]
fn logical_assignment_through_safe_access() {
    let src =
        "/datum/t\n\tvar/v\n\tvar/datum/t/n\n/proc/f(datum/t/a, b)\n\ta?.n?.v ||= b\n\treturn a\n";
    assert_eq!(
        compile(src, "", "f"),
        [
            "GetVar arg(0)",
            "SetCachePopJmpIfNull LAB_0001",
            "PushCache",
            "GetVar cache[\"n\"]",
            "SetCachePopJmpIfNull LAB_0002",
            "PushCache",
            "PopCache",
            "GetVar cache[\"v\"]",
            "Test",
            "Jnz LAB_0002",
            "GetVar arg(1)",
            "SetVar cache[\"v\"]",
            "LAB_0002:",
            "PopCache",
            "LAB_0001:",
            "GetVar arg(0)",
            "Ret",
            "End",
        ]
        .join("\n")
    );
    let src = "/datum/t\n\tvar/v\n\tvar/datum/t/n\n/proc/f(datum/t/a, b)\n\ta?.v &&= a.n\n";
    assert_has(
        &compile(src, "", "f"),
        &[
            "Jz LAB_0001",
            "PushCache",
            "GetVar cache = arg(0); cache[\"n\"]",
            "PopCache",
            "SetVar cache[\"v\"]",
        ],
    );
}

#[test]
fn writing_what_a_call_returns_through_safe_access() {
    let types =
        "/datum/d\n\tvar/datum/d/n\n\tvar/x\n\tvar/list/L\n/datum/d/proc/f(a)\n\treturn n\n";
    let body = |line: &str| {
        compile(
            &format!("{types}/proc/g(datum/d/a, b)\n\t{line}\n"),
            "",
            "g",
        )
    };
    assert_eq!(
        body("a?.f(1).x = b"),
        [
            "GetVar arg(0)",
            "SetCachePopJmpIfNull LAB_0001",
            "PushCache",
            "GetVar arg(1)",
            "PushInt 1",
            "Call dynamic_proc(\"f\") 1",
            "SetVar cache",
            "SetVar cache[\"x\"]",
            "PopCache",
            "LAB_0001:",
            "End",
        ]
        .join("\n")
    );
    assert_has(
        &body("a?.f(1).x = a.n.x"),
        &[
            "GetVar cache = arg(0); cache = cache[\"n\"]; cache[\"x\"]",
            "PopCache",
            "PushCache",
            "PushInt 1",
            "Call dynamic_proc(\"f\") 1",
        ],
    );
    assert_has(
        &body("return (a?.f(1).n.x = b)"),
        &[
            "GetVar arg(1)",
            "PushTop",
            "PushInt 1",
            "Call dynamic_proc(\"f\") 1",
            "SetVar cache",
            "SetVar cache = cache[\"n\"]; cache[\"x\"]",
            "PopCache",
        ],
    );
    assert_has(
        &body("return a?.f(1).x++"),
        &["SetVar cache", "PostInc cache[\"x\"]", "PopCache"],
    );
    assert_has(
        &body("a?.f(1).L[2] = b"),
        &[
            "SetVar cache",
            "GetVar cache[\"L\"]",
            "PushInt 2",
            "ListSet",
            "PopCache",
        ],
    );
    assert_has(
        &body("a?.f(1).L[2] += b"),
        &[
            "SetVar cache",
            "GetVar cache[\"L\"]",
            "PushInt 2",
            "PopCache",
            "SetVar cache_key",
            "SetVar cache",
            "AugAdd cache[cache_key]",
        ],
    );
    assert_has(
        &body("a?.L[1].x ||= a.x"),
        &[
            "GetVar cache[\"L\"]",
            "PushInt 1",
            "ListGet",
            "SetVar cache",
            "GetVar cache[\"x\"]",
            "Test",
            "Jnz LAB_0003",
            "PushCache",
            "GetVar cache = arg(0); cache[\"x\"]",
            "PopCache",
            "SetVar cache[\"x\"]",
            "LAB_0003:",
            "PopCache",
            "LAB_0001:",
        ],
    );
    assert_has(
        &body("return (a?.f(1)[2] &&= b)"),
        &[
            "Call dynamic_proc(\"f\") 1",
            "PushInt 2",
            "PopCache",
            "SetVar cache_key",
            "SetVar cache",
            "GetVar cache[cache_key]",
            "JmpAnd LAB_0006",
            "GetVar arg(1)",
            "SetVarExpr cache[cache_key]",
        ],
    );
}

#[test]
fn or_assign_saves_the_cache_for_some_values() {
    let types =
        "var/gx = 1\n/datum/t\n\tvar/n = 1\n\tvar/m = 2\n\tvar/static/st = 3\n\tvar/const/k = 4\n";
    let value = |v: &str| {
        compile(
            &format!("{types}/proc/f(datum/t/o, list/L)\n\to.n ||= {v}\n"),
            "",
            "f",
        )
    };
    for v in ["o.st", "o.k", "global.gx", "gx"] {
        assert!(!value(v).contains("PushCache"), "{v}: {}", value(v));
    }
    for v in ["o.m", "L[1]", "/datum/t::st", "o::st"] {
        assert!(value(v).contains("PushCache"), "{v}: {}", value(v));
    }
}

#[test]
fn safe_access_to_a_const_is_not_folded() {
    let src =
        "/datum/t\n\tvar/const/k = 256\n/proc/f(datum/t/a)\n\tif (a?.k) return 1\n\treturn 2\n";
    assert_has(
        &compile(src, "", "f"),
        &[
            "GetVar arg(0)",
            "SetCacheJmpIfNull LAB_0003",
            "GetVar global(\"k\")",
            "LAB_0003:",
            "Test",
        ],
    );
}

#[test]
fn hex_escapes_are_characters() {
    let code = compile("/proc/f()\n\treturn \"\\xe9t\\x41\"\n", "", "f");
    assert_has(&code, &["PushVal \"étA\"", "Ret"]);
}

#[test]
fn hashes_of_text_fold() {
    let code = compile("/proc/f()\n\treturn md5(\"x\")\n", "", "f");
    assert_has(
        &code,
        &["PushVal \"9dd4e461268c8034f5c8564e155c67a6\"", "Ret"],
    );
    let code = compile(
        "/proc/f()\n\treturn sha1(\"k\" + \"UPPER lower\")\n",
        "",
        "f",
    );
    assert_has(
        &code,
        &[
            "PushVal \"7806b65f635265368f10c2abc91db9eae1753a54\"",
            "Ret",
        ],
    );
    let code = compile("/proc/f()\n\treturn md5(\"\\xff\")\n", "", "f");
    assert_has(
        &code,
        &["PushVal \"f3f7437e8c1f303fc6742d9368b36cf6\"", "Ret"],
    );
    let code = compile("/proc/f()\n\treturn md5(1)\n", "", "f");
    assert_has(&code, &["PushInt 1", "Md5"]);
}

#[test]
fn assert_gives_numbers_as_written() {
    let code = compile("/proc/f(n)\n\tASSERT(n > 1e3 + 0.50)\n", "", "f");
    assert!(code.contains("PushVal \"n > 1e3 + 0.50\""), "{code}");
}

#[test]
fn folded_var_defaults_have_their_escapes() {
    let src = "/datum/t\n\tvar/v = \"q\\\"q\"\n\tvar/const/k = \"a\\tb\"\n/proc/f()\n\treturn (\"a\" + /datum/t::v) + /datum/t::k\n";
    let code = compile(src, "", "f");
    assert_has(&code, &["PushVal \"aq\\\"qa\tb\"", "Ret"]);
}

#[test]
fn constant_lists_are_true_in_a_test() {
    for v in [
        "list(1, \"k\" = 2)",
        "list()",
        "new /list(2)",
        "newlist(/datum)",
        "list(k = new /datum)",
    ] {
        let code = compile(&format!("/proc/f(a, b)\n\treturn ({v} ? a : b)\n"), "", "f");
        assert_eq!(code, "GetVar arg(0)\nRet\nEnd", "{v}");
    }
    let code = compile("/proc/f(a, b)\n\treturn (list(a) ? a : b)\n", "", "f");
    assert_has(&code, &["GetVar arg(0)", "NewList 1", "Test"]);
}

#[test]
fn a_const_chosen_by_a_constant_test_is_read_alone() {
    let types = "/datum/t\n\tvar/const/k = 1000\n";
    for v in [
        "(7 && /datum/t::k)",
        "(/datum/t::k || \"abc\")",
        "(1 ? /datum/t::k : 2)",
    ] {
        let code = compile(&format!("{types}/proc/f()\n\treturn {v}\n"), "", "f");
        assert_eq!(code, "GetVar global(\"k\")\nRet\nEnd", "{v}");
    }
    let code = compile(
        &format!("{types}/proc/f()\n\treturn (0 || /datum/t::k + 1)\n"),
        "",
        "f",
    );
    assert_eq!(code, "PushInt 1001\nRet\nEnd");
}

#[test]
fn goto_in_a_try_block_uses_try_jmp() {
    let src = "/proc/f(a)\n\ttry\n\t\tif (a) goto l1\n\t\ta = 2\n\t\tl1:\n\t\ta += 1\n\tcatch\n\t\tif (a) goto l2\n\t\tl2:\n\treturn a\n";
    let code = compile(src, "", "f");
    assert!(code.contains("TryJmp USER_l1"), "{code}");
    assert!(code.contains("JmpLoop USER_l2"), "{code}");
}

#[test]
fn type_and_proc_macros_fold_in_a_test() {
    let code = compile(
        "/datum/t/proc/f(a, b)\n\treturn (__TYPE__ ? a : b)\n",
        "/datum/t",
        "f",
    );
    assert_eq!(code, "GetVar arg(0)\nRet\nEnd");
    let code = compile("/proc/f(a, b)\n\treturn (__TYPE__ ? a : b)\n", "", "f");
    assert_eq!(code, "GetVar arg(1)\nRet\nEnd");
    let code = compile("/proc/f(a, b)\n\treturn (__PROC__ ? a : b)\n", "", "f");
    assert_eq!(code, "GetVar arg(0)\nRet\nEnd");
}

#[test]
fn call_arguments_are_a_procs() {
    let code = compile(
        "/proc/f(o, n)\n\treturn call(o, \"f\")((n = 1), 2)\n",
        "",
        "f",
    );
    assert_has(
        &code,
        &[
            "PushVal \"f\"",
            "PushInt 1",
            "SetVarExpr arg(1)",
            "PushInt 2",
            "CallName 2",
        ],
    );
    let code = compile(
        "/proc/f(o, n)\n\treturn call(o, \"f\")((n = 1), x = 2)\n",
        "",
        "f",
    );
    assert_has(
        &code,
        &[
            "GetVar arg(1)",
            "PushInt 1",
            "PushVal \"x\"",
            "PushInt 2",
            "NewAssocList 2",
            "CallNameArgList",
        ],
    );
}

#[test]
fn a_false_else_if_leaves_its_jump() {
    let code = compile(
        "/proc/f(a, b)\n\tif (a) b = 1\n\telse if (0) b = 2\n\treturn b\n",
        "",
        "f",
    );
    assert_has(&code, &["SetVar arg(1)", "Jmp LAB_0000", "LAB_0001:"]);
    let code = compile(
        "/proc/f(a, b)\n\tif (0) b = 1\n\telse if (a) b = 2\n\treturn b\n",
        "",
        "f",
    );
    assert!(!code.contains("Jmp "), "{code}");
}

#[test]
fn or_assign_goes_by_the_source_of_a_folded_value() {
    let types = "/datum/t\n\tvar/n = 1\n\tvar/m = 2\n\tvar/static/st = 3\n";
    let code = compile(
        &format!("{types}/proc/f(datum/t/o)\n\to.n &&= (0 && o.m)\n"),
        "",
        "f",
    );
    assert_has(&code, &["PushCache", "PushInt 0", "PopCache"]);
    let code = compile(
        &format!("{types}/proc/f(datum/t/o)\n\to.n &&= (0 && o.st)\n"),
        "",
        "f",
    );
    assert!(!code.contains("PushCache"), "{code}");
}

#[test]
fn a_dropped_if_forgets_the_cache() {
    let src = "/datum/t\n\tvar/list/l = list(1)\n/proc/f(datum/t/o, a)\n\to.l.Remove(a)\n\tif (0)\n\t\ta = 2\n\treturn o.l.len\n";
    assert_has(
        &compile(src, "", "f"),
        &[
            "Pop",
            "GetVar cache = arg(0); cache = cache[\"l\"]; cache[\"len\"]",
        ],
    );
}

#[test]
fn folding_and_or_forgets_the_cache() {
    let types = "/datum/t\n\tvar/a = 1\n\tvar/list/l\n";
    let code = compile(
        &format!("{types}/proc/f(datum/t/p, n)\n\tp.l = list()\n\tn = (1 || 2)\n\treturn p.a\n"),
        "",
        "f",
    );
    assert_has(
        &code,
        &["SetVar arg(1)", "GetVar cache = arg(0); cache[\"a\"]"],
    );
    let code = compile(
        &format!("{types}/proc/f(datum/t/p, n)\n\tp.l = list()\n\tn = (1 ? 2 : 3)\n\treturn p.a\n"),
        "",
        "f",
    );
    assert_has(&code, &["SetVar arg(1)", "GetVar cache[\"a\"]"]);
}

#[test]
fn an_if_decided_when_compiled_forgets_the_cache_after_it() {
    let src = "/datum/t\n\tvar/a = 1\n\tvar/b = 2\n/proc/f(datum/t/p, n)\n\tif (1) n = p.a\n\treturn p.b\n";
    assert_has(
        &compile(src, "", "f"),
        &["SetVar arg(1)", "GetVar cache = arg(0); cache[\"b\"]"],
    );
}

#[test]
fn initial_of_a_static_or_const_is_its_globals() {
    let types = "/datum/t\n\tvar/n = 1\n\tvar/static/s = 2\n\tvar/const/k = 3\n\tvar/datum/t/o\n";
    let code = compile(
        &format!("{types}/proc/f(datum/t/a)\n\treturn initial(a.o.s) + initial(a.k)\n"),
        "",
        "f",
    );
    assert_has(
        &code,
        &[
            "GetVar initial(global(\"s\"))",
            "GetVar initial(global(\"k\"))",
            "Add",
        ],
    );
    let code = compile(
        &format!("{types}/proc/f(datum/t/a)\n\treturn initial(a?.o?.n)\n"),
        "",
        "f",
    );
    assert_has(
        &code,
        &[
            "GetVar arg(0)",
            "SetVar cache",
            "PushCache",
            "GetVar cache[\"o\"]",
            "SetVar cache",
            "PopCache",
            "GetVar initial(cache[\"n\"])",
        ],
    );
}

fn compile_with_lines(src: &str) -> Vec<String> {
    compile_with_lines_of(src, "", "f")
}

fn compile_with_lines_of(src: &str, ty: &str, name: &str) -> Vec<String> {
    let ctx = dreammaker::Context::default();
    let pp = dreammaker::Preprocessor::from_buffer(&ctx, "test.dm".into(), src.to_owned());
    let mut parser = dreammaker::Parser::new(&ctx, pp);
    parser.enable_procs();
    let tree = parser.parse_object_tree();
    let ty = if ty.is_empty() {
        tree.root()
    } else {
        tree.find(ty).expect("type")
    };
    let value = ty.get().procs.get(name).unwrap().value.last().unwrap();
    let source = ProcSource {
        tree: &tree,
        ty,
        name,
        value,
        file: "test.dm",
        debug_info: true,
        index: None,
        byond: dmasm_compiler::ByondVersion::LATEST,
        file_names: Some(&|_| "other.dm".to_owned()),
    };
    let code = dmasm_bytecode::format(&compile_proc(&source).unwrap().nodes);
    code.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("LAB_"))
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_type_scope_default_records_where_it_is_written() {
    let code =
        compile_with_lines("/datum/u\n\tvar/x = 3\n/proc/f(a)\n\ta = /datum/u::x\n\treturn a\n");
    assert_eq!(
        code,
        [
            "DbgFile \"test.dm\"",
            "DbgLine 4",
            "DbgLine 2",
            "PushInt 3",
            "SetVar arg(0)",
            "DbgLine 5",
            "GetVar arg(0)",
            "Ret",
            "End"
        ]
    );
}

#[test]
fn only_compound_terms_record_a_later_line() {
    let code = compile_with_lines("/proc/g()\n/proc/f(datum/a)\n\ta = list(1,\n\t\ta,\n\t\ta.type,\n\t\t-1,\n\t\t1 + 2,\n\t\t\"[a]\",\n\t\tg())\n");
    let lines: Vec<&str> = code
        .iter()
        .map(String::as_str)
        .filter(|l| l.starts_with("DbgLine"))
        .collect();
    assert_eq!(lines, ["DbgLine 3", "DbgLine 7", "DbgLine 8", "DbgLine 9"]);
}

#[test]
fn after_a_default_the_next_compound_term_records_its_line() {
    let code = compile_with_lines(
        "/proc/f(a)\n\tif (text(\"[]\", /datum/u::x) || \"[a]\")\n\t\ta = 1\n/datum/u\n\tvar/x = 3\n",
    );
    let at = code.iter().position(|l| l.starts_with("JmpOr")).unwrap();
    assert_eq!(
        code[at + 1..at + 3],
        ["DbgLine 2", "GetVar arg(0)"],
        "{code:?}"
    );
    let code = compile_with_lines(
        "/proc/f(a)\n\ta = list(/datum/u::x, a, a.type)\n/datum/u\n\tvar/x = 3\n",
    );
    assert_eq!(
        code.iter().filter(|l| l.starts_with("DbgLine")).count(),
        2,
        "{code:?}"
    );
}

#[test]
fn locate_in_a_list_in_another() {
    let code = compile(
        "/datum/t\n/proc/f(a, b)\n\treturn locate(/datum/t) in a in b\n",
        "",
        "f",
    );
    assert_eq!(
        code,
        "GetVar arg(1)\nPushVal /datum/t\nGetVar arg(0)\nLocateType\nIsIn Value\nGetFlag\nRet\nEnd"
    );
}

#[test]
fn for_loop_separators_can_be_mixed() {
    let body = |head: &str| {
        compile(
            &format!("/proc/f(n)\n\tvar/i\n\tfor({head})\n\t\tworld.log << i\n"),
            "",
            "f",
        )
    };
    let want = body("i = 1; i <= n; i++");
    assert!(want.contains("Inc local(0)"), "{want}");
    assert_eq!(body("i = 1; i <= n, i++"), want);
    assert_eq!(body("i = 1, i <= n; i++"), want);
}

#[test]
fn assert_keeps_raw_strings_as_written() {
    let code = compile(
        "/proc/f(a)\n\tASSERT(a == @{\"brace\"} || a == @'one')\n",
        "",
        "f",
    );
    assert!(
        code.contains(r#"PushVal "a == @{\"brace\"} || a == @'one'""#),
        "{code}"
    );
}

#[test]
fn the_place_of_an_augmented_assignment_records_no_line() {
    let lines = |code: Vec<String>| {
        code.into_iter()
            .filter(|l| l.starts_with("DbgLine"))
            .collect::<Vec<_>>()
    };
    let src =
        |op: &str| format!("/proc/f(list/L)\n\tL[\"b\"] {op} /datum/u::x\n/datum/u\n\tvar/x = 3\n");
    assert_eq!(
        lines(compile_with_lines(&src("-="))),
        ["DbgLine 2", "DbgLine 4"]
    );
    assert_eq!(
        lines(compile_with_lines(&src("="))),
        ["DbgLine 2", "DbgLine 4", "DbgLine 2"]
    );
}

#[test]
fn a_folded_default_from_another_file_still_names_the_file_again() {
    let ctx = dreammaker::Context::default();
    let mut pp = dreammaker::Preprocessor::from_buffer(
        &ctx,
        "test.dm".into(),
        "/proc/f(a)\n\ta = /datum/u::x + 1\n\ta = 2\n".to_owned(),
    );
    pp.push_file(
        "other.dm".into(),
        std::io::Cursor::new(b"/datum/u\n\tvar/x = 3\n".to_vec()),
    )
    .unwrap();
    let mut parser = dreammaker::Parser::new(&ctx, pp);
    parser.enable_procs();
    let tree = parser.parse_object_tree();
    let value = tree
        .root()
        .get()
        .procs
        .get("f")
        .unwrap()
        .value
        .last()
        .unwrap();
    let source = ProcSource {
        tree: &tree,
        ty: tree.root(),
        name: "f",
        value,
        file: "test.dm",
        debug_info: true,
        index: None,
        byond: dmasm_compiler::ByondVersion::LATEST,
        file_names: Some(&|_| "other.dm".to_owned()),
    };
    let code = dmasm_bytecode::format(&compile_proc(&source).unwrap().nodes);
    let code: Vec<&str> = code
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(
        code,
        [
            "DbgFile \"test.dm\"",
            "DbgLine 2",
            "PushInt 4",
            "SetVar arg(0)",
            "DbgFile \"test.dm\"",
            "DbgLine 3",
            "PushInt 2",
            "SetVar arg(0)",
            "End"
        ]
    );
}

fn records_with_default_from_other_file(expr: &str) -> Vec<String> {
    let ctx = dreammaker::Context::default();
    let mut pp = dreammaker::Preprocessor::from_buffer(
        &ctx,
        "test.dm".into(),
        format!("/proc/f(a)\n\ta = {expr}\n\ta = 2\n"),
    );
    pp.push_file(
        "other.dm".into(),
        std::io::Cursor::new(b"/datum/u\n\tvar/x = -4\n".to_vec()),
    )
    .unwrap();
    let mut parser = dreammaker::Parser::new(&ctx, pp);
    parser.enable_procs();
    let tree = parser.parse_object_tree();
    let value = tree
        .root()
        .get()
        .procs
        .get("f")
        .unwrap()
        .value
        .last()
        .unwrap();
    let source = ProcSource {
        tree: &tree,
        ty: tree.root(),
        name: "f",
        value,
        file: "test.dm",
        debug_info: true,
        index: None,
        byond: dmasm_compiler::ByondVersion::LATEST,
        file_names: Some(&|_| "other.dm".to_owned()),
    };
    dmasm_bytecode::format(&compile_proc(&source).unwrap().nodes)
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("Dbg"))
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_default_from_another_file_in_an_operand_folded_away_names_nothing() {
    let files = |expr: &str| {
        records_with_default_from_other_file(expr)
            .iter()
            .filter(|l| l.starts_with("DbgFile"))
            .count()
    };
    for expr in [
        "(1 ? -1 : /datum/u::x)",
        "(0 ? /datum/u::x : 2)",
        "(1 || /datum/u::x)",
        "(0 && /datum/u::x)",
        "(/datum/u::x && 0)",
        "((~(\"A,b,c\" ? -1 : /datum/u::x)) || 3)",
        "min(1, (1 ? 2 : /datum/u::x))",
    ] {
        assert_eq!(files(expr), 1, "{expr}");
    }
    for expr in [
        "/datum/u::x + 1",
        "(/datum/u::x ? 1 : 2)",
        "(/datum/u::x || 3)",
    ] {
        assert!(files(expr) > 1, "{expr}");
    }
}

#[test]
fn a_constant_condition_from_another_file_names_the_line_again() {
    assert_eq!(
        records_with_default_from_other_file("(/datum/u::x ? a : 2)"),
        [
            "DbgFile \"test.dm\"",
            "DbgLine 2",
            "DbgFile \"test.dm\"",
            "DbgLine 2",
            "DbgLine 3"
        ]
    );
}

#[test]
fn a_fold_that_differs_by_platform_is_warned_about() {
    let latest = dmasm_compiler::ByondVersion::LATEST;
    let warnings =
        |expr: &str, byond| warnings_for(&format!("/proc/f(a)\n\treturn {expr}\n"), byond);
    let warned = |expr: &str, what: &str| {
        let w = warnings(expr, latest);
        assert!(w.len() == 1 && w[0].contains(what), "{expr}: {w:?}");
    };
    let quiet = |expr: &str, byond| {
        let w = warnings(expr, byond);
        assert!(w.is_empty(), "{expr} on {byond:?}: {w:?}");
    };
    warned(
        "arctan(2)",
        "arctan(2) is folded to 63.434948, as DreamMaker does on Windows",
    );
    warned(
        "arctan(2)",
        "with glibc 2.41 or later folds it to 63.43495.",
    );
    warned(
        "arccos(0.7)",
        "with glibc 2.41 or later folds it to 45.572994.",
    );
    warned("arctan(1, 2)", "arctan(1, 2) is folded to 63.434948");
    warned("arctan(16777215)", "folds it to 89.99999.");
    quiet("arctan(1)", latest);
    quiet("arcsin(0.5)", latest);
    quiet("arctan(a)", latest);
    quiet("arctan(2)", dmasm_compiler::ByondVersion::new(515, 1637));
    warned(
        "rgb(0, 60, 25, space = COLORSPACE_HSV)",
        "rgb() is folded to #401919, matching Linux DreamMaker",
    );
    warned(
        "rgb(h = 180, s = 60, l = 25)",
        "Windows may round a channel differently",
    );
    quiet("rgb(0, 60, 26, space = COLORSPACE_HSV)", latest);
    quiet("rgb(127.5, 2, 3)", latest);
    quiet("rgb(0, 60, a, space = COLORSPACE_HSV)", latest);
    quiet(
        "rgb(0, 60, 25, space = COLORSPACE_HSV)",
        dmasm_compiler::ByondVersion::new(515, 1637),
    );
}

fn compiled_of(
    code: &str,
    ty: &str,
    name: &str,
    byond: dmasm_compiler::ByondVersion,
) -> dmasm_compiler::compiler::CompiledProc {
    let ctx = dreammaker::Context::default();
    let pp = dreammaker::Preprocessor::from_buffer(&ctx, "test.dm".into(), code.to_owned());
    let mut parser = dreammaker::Parser::new(&ctx, pp);
    parser.enable_procs();
    let tree = parser.parse_object_tree();
    let ty = if ty.is_empty() {
        tree.root()
    } else {
        tree.find(ty).unwrap()
    };
    let value = ty.get().procs.get(name).unwrap().value.last().unwrap();
    let source = ProcSource {
        tree: &tree,
        ty,
        name,
        value,
        file: "test.dm",
        debug_info: false,
        index: None,
        byond,
        file_names: None,
    };
    compile_proc(&source).unwrap()
}

fn warnings_of(
    code: &str,
    ty: &str,
    name: &str,
    byond: dmasm_compiler::ByondVersion,
) -> Vec<String> {
    let compiled = compiled_of(code, ty, name, byond);
    let warnings: Vec<String> = compiled.warnings.iter().map(ToString::to_string).collect();
    for bug in &compiled.byond_bugs {
        let text = match bug.line {
            Some(line) => format!("line {}: {}", line, bug.text),
            None => bug.text.clone(),
        };
        assert!(
            compiled
                .warnings
                .iter()
                .any(|w| w.id == bug.id && w.to_string().starts_with(&text)),
            "{:?}",
            warnings
        );
    }
    warnings
}

#[test]
fn warnings_about_byonds_bugs_are_told_from_the_rest() {
    let bugs = |code: &str, build| -> Vec<String> {
        let compiled = compiled_of(code, "", "f", dmasm_compiler::ByondVersion::new(516, build));
        compiled
            .byond_bugs
            .into_iter()
            .map(|bug| bug.text)
            .collect()
    };
    let pointer = "/proc/f(list/L)\n\treturn *L[1]\n";
    assert_eq!(
        bugs(pointer, 1688),
        [
            "`*p[i]`: from 516.1668 DreamMaker leaves the index out, a \"bad index\" error for a \
          list of pointers"
        ]
    );
    let weights = "/proc/f(a, b)\n\treturn pick(1; a, 0; b)\n";
    assert_eq!(
        bugs(weights, 1688),
        ["pick() ends with a zero-weight entry, where DreamMaker's code jumps out of the proc"]
    );
    let divided = "/datum/proc/f(a, b = a.len/10)\n\treturn b\n";
    let compiled = compiled_of(
        divided,
        "/datum",
        "f",
        dmasm_compiler::ByondVersion::new(516, 1688),
    );
    assert!(compiled.warnings[0]
        .to_string()
        .ends_with("leaves the division out; dmasm divides"));
    assert!(compiled.byond_bugs[0]
        .text
        .ends_with("leaves the division out"));
    assert_eq!(bugs(weights, 1689), [] as [String; 0]);
    let wide = "/proc/f(a)\n\treturn (-1e10) ^ a\n";
    assert_eq!(bugs(wide, 1688), [] as [String; 0]);
    let compiled = compiled_of(wide, "", "f", dmasm_compiler::ByondVersion::new(516, 1688));
    assert!(!compiled.warnings.is_empty());
}

#[test]
fn code_that_byond_gets_wrong_is_warned_about() {
    let latest = dmasm_compiler::ByondVersion::LATEST;
    let v = |build| dmasm_compiler::ByondVersion::new(516, build);
    let warned = |code: &str, ty: &str, name: &str, byond, what: &str| {
        let w = warnings_of(code, ty, name, byond);
        assert!(
            w.iter().any(|w| w.contains(what)),
            "{name} of {code:?} on {byond:?}: {w:?}"
        );
    };
    let quiet = |code: &str, ty: &str, name: &str, byond| {
        let w = warnings_of(code, ty, name, byond);
        assert!(w.is_empty(), "{name} of {code:?} on {byond:?}: {w:?}");
    };

    let labelled = |outer: &str, go_on: &str| {
        format!(
            "/proc/f()\n\t{outer}\n\t\tvar/i = 0\n\t\tinner:\n\t\t\tdo\n\t\t\t\ti++\n\
             \t\t\t\tif (i == 1)\n\t\t\t\t\t{go_on}\n\t\t\twhile (i < 2)\n"
        )
    };
    let counted = "for (var/j in 1 to 2)";
    warned(
        &labelled(counted, "continue inner"),
        "",
        "f",
        latest,
        "line 8: `continue inner`",
    );
    warned(
        &labelled("for (var/j = 1 to 2)", "continue inner"),
        "",
        "f",
        latest,
        "crashes the server",
    );
    quiet(&labelled(counted, "continue"), "", "f", latest);
    quiet(&labelled(counted, "break inner"), "", "f", latest);
    quiet(
        &labelled("for (var/j in list(1, 2))", "continue inner"),
        "",
        "f",
        latest,
    );
    quiet(
        "/proc/f()\n\tfor (var/j in 1 to 2)\n\t\tvar/i = 0\n\t\tinner:\n\t\t\twhile (i < 2)\n\
         \t\t\t\ti++\n\t\t\t\tcontinue inner\n",
        "",
        "f",
        latest,
    );

    let nested = |outer: &str, between: &str, inner: &str| {
        format!(
            "/proc/f()\n\t{outer}\n\t\ttry\n\t\t\t{between}\n\t\t\t\t{inner}\n\
             \t\t\t\t\tthrow 1\n\t\tcatch\n\t\t\treturn\n"
        )
    };
    let list = "for (var/a in list(1, 2))";
    warned(
        &nested(list, "if (1)", "for (var/b in list(1))"),
        "",
        "f",
        latest,
        "line 5: nested list loops separated by `try`",
    );
    warned(
        &nested(list, "for (var/c in 1 to 2)", "for (var/k, v in list(1))"),
        "",
        "f",
        latest,
        "may end early, loop forever",
    );
    quiet(
        &nested(list, "if (1)", "for (var/b in 1 to 2)"),
        "",
        "f",
        latest,
    );
    quiet(
        &nested("for (var/a in 1 to 2)", "if (1)", "for (var/b in list(1))"),
        "",
        "f",
        latest,
    );
    quiet(
        "/proc/f()\n\tfor (var/a in list(1))\n\t\tfor (var/b in list(1))\n\t\t\ttry\n\
         \t\t\t\tthrow 1\n\t\t\tcatch\n\t\t\t\treturn\n",
        "",
        "f",
        latest,
    );
    quiet(
        "/proc/f()\n\tfor (var/a in list(1))\n\t\tfor (var/b in list(1))\n\t\t\treturn\n",
        "",
        "f",
        latest,
    );

    let pointer = "/proc/f()\n\tvar/v\n\tvar/list/L = list(&v)\n\treturn *L[1]\n";
    warned(pointer, "", "f", v(1668), "leaves the index out");
    quiet(pointer, "", "f", v(1667));

    let types = "/datum/t/proc/operator<<=(b)\n\treturn 1\n/datum/t/proc/operator>>=(b)\n\treturn 1\n\
                 /datum/t/proc/operator++()\n\treturn 1\n/datum/t/proc/operator--()\n\treturn 1\n\
                 /datum/t/proc/operator**(b)\n\treturn 1\n/datum/t/proc/operator*(b)\n\treturn 1\n\
                 /datum/t/proc/operator<=>(b)\n\treturn 0\n/datum/t/proc/operator~=(b)\n\treturn 0\n\
                 /datum/t/proc/operator~!(b)\n\treturn 1\n\
                 /datum/t/proc/operator+=(b)\n\treturn 1\n\
                 /datum/u/proc/operator**(b)\n\treturn 1\n\
                 /area/x/Uncross(atom/movable/O)\n\treturn 0\n/turf/x/Uncross(atom/movable/O)\n\treturn 0\n";
    for name in ["operator<<=", "operator>>="] {
        warned(types, "/datum/t", name, latest, "discards the result");
    }
    warned(types, "/datum/t", "operator++", latest, "`a++` and `++a`");
    warned(types, "/datum/t", "operator--", latest, "`a--` and `--a`");
    warned(
        types,
        "/datum/t",
        "operator**",
        latest,
        "calls `operator*(a)`",
    );
    quiet(types, "/datum/u", "operator**", latest);
    quiet(types, "/datum/t", "operator*", latest);
    quiet(types, "/datum/t", "operator+=", latest);
    warned(types, "/datum/t", "operator<=>", v(1674), "list.Find(x)");
    quiet(types, "/datum/t", "operator<=>", v(1673));
    for name in ["operator~=", "operator~!"] {
        warned(types, "/datum/t", name, v(1674), "discard the result");
        warned(types, "/datum/t", name, v(1688), "discard the result");
        quiet(types, "/datum/t", name, v(1673));
        quiet(types, "/datum/t", name, v(1689));
    }
    warned(types, "/area/x", "Uncross", v(1683), "ignores the result");
    quiet(types, "/area/x", "Uncross", v(1682));
    quiet(types, "/turf/x", "Uncross", latest);
}

fn warnings_for(code: &str, byond: dmasm_compiler::ByondVersion) -> Vec<String> {
    let ctx = dreammaker::Context::default();
    let pp = dreammaker::Preprocessor::from_buffer(&ctx, "test.dm".into(), code.to_owned());
    let mut parser = dreammaker::Parser::new(&ctx, pp);
    parser.enable_procs();
    let tree = parser.parse_object_tree();
    let value = tree
        .root()
        .get()
        .procs
        .get("f")
        .unwrap()
        .value
        .last()
        .unwrap();
    let source = ProcSource {
        tree: &tree,
        ty: tree.root(),
        name: "f",
        value,
        file: "test.dm",
        debug_info: false,
        index: None,
        byond,
        file_names: None,
    };
    compile_proc(&source)
        .unwrap()
        .warnings
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn deliberate_differences_and_copied_quirks_are_warned_about() {
    let latest = dmasm_compiler::ByondVersion::LATEST;
    let v = |build| dmasm_compiler::ByondVersion::new(516, build);
    let warned = |code: &str, byond, what: &str| {
        let w = warnings_for(code, byond);
        assert!(
            w.iter().any(|w| w.contains(what)),
            "{code:?} on {byond:?}: {w:?}"
        );
    };
    let quiet = |code: &str, byond| {
        let w = warnings_for(code, byond);
        assert!(w.is_empty(), "{code:?} on {byond:?}: {w:?}");
    };
    let types = "/datum/t\n\tvar/n = 1\n\tvar/list/L = list(1)\n\tvar/datum/t/o\n\tproc/g(x)\n";
    warned(
        "/proc/f(t = world.fps/10)\n\treturn t\n",
        latest,
        "reads it as a path",
    );
    quiet("/proc/f(t = 1/10)\n\treturn t\n", latest);
    warned(
        &format!("{types}/proc/f(datum/t/a, datum/t/b)\n\ta?.g(b?.n)?.n = 1\n"),
        latest,
        "dmasm uses a",
    );
    for line in [
        "a?.g().n = b.n",
        "a?.g(b.n).n = 1",
        "a?.L[1].n = b.n",
        "return a?.g(b.n).n",
    ] {
        warned(
            &format!("{types}/proc/f(datum/t/a, datum/t/b)\n\t{line}\n"),
            latest,
            "(b); dmasm uses a",
        );
    }
    for line in ["a?.g().n = b", "return a?.g(b.n)", "return a?.g(b).n"] {
        quiet(&format!("{types}/proc/f(datum/t/a, b)\n\t{line}\n"), latest);
    }
    warned(
        &format!("{types}/proc/f(datum/t/a)\n\ta?.n ||= 1\n"),
        v(1672),
        "before 516.1673",
    );
    quiet(
        &format!("{types}/proc/f(datum/t/a)\n\ta?.n ||= 1\n"),
        v(1673),
    );
    let pointer = "/proc/f()\n\tvar/list/L = list(1, 2)\n\tvar/p = &L\n\t*p[1] = 5\n";
    warned(pointer, v(1668), "leaves the index out");
    quiet(pointer, v(1667));
    warned("/proc/f(x)\n\treturn (-1e10) ^ x\n", latest, "32-bit range");
    warned(
        "/proc/f()\n\treturn isnan(1)\n",
        v(1659),
        "the previous test flag",
    );
    quiet("/proc/f()\n\treturn isnan(1)\n", v(1672));
    warned(
        &format!("{types}/proc/f(datum/t/a, datum/t/b)\n\ta?.L[1] += b.n\n"),
        v(1684),
        "before 516.1685",
    );
    quiet(
        &format!("{types}/proc/f(datum/t/a, datum/t/b)\n\ta?.L[1] += b.n\n"),
        v(1685),
    );
    quiet(
        &format!("{types}/proc/f(datum/t/a)\n\ta?.L[1] += 2\n"),
        v(1684),
    );
}

#[test]
fn initial_of_a_static_field() {
    let src = "/obj/item\n\tvar/throwforce = 7\n\tvar/static/sv = 3\n\tvar/obj/item/other\n\
        /proc/a()\n\treturn initial(/obj/item::throwforce)\n\
        /proc/b()\n\treturn initial(/obj/item::sv)\n\
        /proc/c()\n\tvar/obj/item/I = new\n\treturn initial(I.other::throwforce)\n\
        /obj/item/proc/m()\n\treturn initial(src::throwforce)\n";
    assert_has(
        &compile(src, "", "a"),
        &[
            "PushVal /obj/item",
            "SetVar cache",
            "GetVar initial(cache[\"throwforce\"])",
            "Ret",
        ],
    );
    assert_has(
        &compile(src, "", "b"),
        &[
            "PushVal /obj/item",
            "SetVar cache",
            "GetVar initial(global(\"sv\"))",
            "Ret",
        ],
    );
    assert_has(
        &compile(src, "", "c"),
        &[
            "GetVar cache = local(0); cache[\"other\"]",
            "SetVar cache",
            "GetVar initial(cache[\"throwforce\"])",
        ],
    );
    assert_has(
        &compile(src, "/obj/item", "m"),
        &[
            "GetVar src",
            "SetVar cache",
            "GetVar initial(cache[\"throwforce\"])",
        ],
    );
}

#[test]
fn a_name_alone_on_its_line_that_a_goto_names_is_a_label() {
    let src = "/proc/f()\n\tvar/i = 0\n\tagain\n\ti++\n\tif (i < 5)\n\t\tgoto again\n\treturn i\n";
    let code = compile(src, "", "f");
    assert_eq!(
        code,
        "PushInt 0\nSetVar local(0)\nUSER_again:\nInc local(0)\nGetVar local(0)\nPushInt 5\nTl\nTest\nJz LAB_0002\nJmpLoop USER_again\nLAB_0002:\nLAB_0001:\nGetVar local(0)\nRet\nEnd"
    );
    let src = "/proc/f(L)\n\tout\n\tfor (var/x in L)\n\t\tif (x) goto out\n";
    let code = compile(src, "", "f");
    assert!(code.contains("USER_out:"), "{code}");
    let code = compile("/proc/f(a)\n\ta\n\treturn 1\n", "", "f");
    assert!(!code.contains("USER_"), "{code}");
}

#[test]
fn a_proc_of_the_world_s_never_calls_by_name() {
    let src = "/mob/proc/known()\n/proc/f(mob/M, x)\n\tM.known()\n\tM.unknown()\n\tx:unknown()\n";
    let code = compile(src, "", "f");
    assert!(!code.contains("CallName"), "{code}");
    assert!(code.contains("dynamic_proc(\"known\")"), "{code}");
    assert!(code.contains("dynamic_proc(\"unknown\")"), "{code}");
}

fn dbg_lines(src: &str) -> Vec<String> {
    compile_with_lines(src)
        .into_iter()
        .filter(|l| l.starts_with("DbgLine"))
        .collect()
}

#[test]
fn the_first_line_is_the_first_with_anything_on_it() {
    let empty_macro = "#define EMPTY\n/proc/f()\n\tEMPTY\n\tvar/x\n\treturn x\n";
    assert_eq!(
        dbg_lines(empty_macro),
        ["DbgLine 3", "DbgLine 4", "DbgLine 5"]
    );
    let semicolon = "/proc/f()\n\t;\n\treturn 1\n";
    assert_eq!(dbg_lines(semicolon), ["DbgLine 2", "DbgLine 3"]);
    let doc_comment = "/proc/f()\n\t/// doc\n\treturn 1\n";
    assert_eq!(dbg_lines(doc_comment), ["DbgLine 3"]);
    let call_macro = "#define CALL(a, b) world.log << list(a, b)\n/proc/f()\n\tCALL(1,\n\t\t2)\n";
    assert_eq!(dbg_lines(call_macro), ["DbgLine 3", "DbgLine 4"]);
    let empty = "/proc/f()\n\t// nothing\n\n/proc/g()\n";
    assert_eq!(dbg_lines(empty), ["DbgLine 4"]);
    let before_a_type = "/proc/f()\n\n/datum\n\tvar/v\n";
    assert_eq!(dbg_lines(before_a_type), ["DbgLine 3"]);
}

#[test]
fn a_return_type_records_the_header_after_the_first_line() {
    let src = "/proc/f() as num\n\t// comment\n\treturn 1\n";
    assert_eq!(dbg_lines(src), ["DbgLine 3", "DbgLine 1", "DbgLine 3"]);
}

#[test]
fn an_assignment_operator_begins_with_dot_src() {
    let assigning = [
        "+=", "-=", "*=", "/=", "%=", "%%=", "&=", "|=", "^=", "<<=", ">>=", ":=", "++", "--",
    ];
    let others = [
        "+", "-", "*", "/", "%", "%%", "**", "&", "|", "^", "~", "<<", ">>", "<", ">", "<=", ">=",
        "~=", "~!", "[]", "[]=", "\"\"",
    ];
    let mut code = "/datum/d\n\tvar/n\n".to_owned();
    for op in assigning.iter().chain(&others) {
        code += &format!("/datum/d/proc/operator{op}(b = 1)\n\tn = b\n");
    }
    for op in assigning {
        let compiled = compile(&code, "/datum/d", &format!("operator{op}"));
        assert_has(
            &compiled,
            &[
                "SetVar arg(0)",
                "LAB_0000:",
                "GetVar src",
                "SetVar dot",
                "GetVar arg(0)",
            ],
        );
    }
    for op in others {
        let compiled = compile(&code, "/datum/d", &format!("operator{op}"));
        assert!(
            !compiled.contains("SetVar dot"),
            "operator{op}:\n{compiled}"
        );
    }
}

#[test]
fn an_assignment_operator_records_the_header_for_dot_src() {
    let lines = |header: &str, body: &str| {
        let src = format!("/datum/d\n\tvar/n\n/datum/d/proc/{header}\n{body}/datum/d/proc/g()\n");
        let name = header.split('(').next().unwrap();
        compile_with_lines_of(&src, "/datum/d", name)
            .into_iter()
            .filter(|l| l.starts_with("DbgLine") || l.starts_with("SetVar dot"))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        lines("operator+=(b)", "\tn = b\n"),
        ["DbgLine 4", "DbgLine 3", "SetVar dot", "DbgLine 4"]
    );
    assert_eq!(
        lines("operator+=(b = 1)", "\tn = b\n"),
        [
            "DbgLine 4",
            "DbgLine 3",
            "DbgLine 4",
            "DbgLine 3",
            "SetVar dot",
            "DbgLine 4"
        ]
    );
    assert_eq!(
        lines("operator+=(b = 1) as num", "\tn = b\n"),
        ["DbgLine 4", "DbgLine 3", "SetVar dot", "DbgLine 4"]
    );
    assert_eq!(
        lines("operator-=(b) as num", "\tn = b\n"),
        ["DbgLine 4", "DbgLine 3", "SetVar dot", "DbgLine 4"]
    );
    assert_eq!(
        lines("operator++()", ""),
        ["DbgLine 4", "DbgLine 3", "SetVar dot"]
    );
}

#[test]
fn lines_read_on_are_recorded_as_dreammaker_does() {
    let lines = |src: &str| dbg_lines(&format!("/proc/f(a, b)\n{src}"));
    assert_eq!(
        lines("\tf(1)\n\tvar/s = \"[a]\"\n\treturn s\n"),
        [
            "DbgLine 2",
            "DbgLine 3",
            "DbgLine 4",
            "DbgLine 3",
            "DbgLine 4"
        ]
    );
    assert_eq!(
        lines("\tif(a)\n\t\treturn \"[a]\"\n\treturn 1\n"),
        ["DbgLine 2", "DbgLine 4", "DbgLine 3", "DbgLine 4"]
    );
    assert_eq!(
        lines("\tfor(var/k, v in a)\n\t\tf(v)\n"),
        ["DbgLine 2", "DbgLine 3", "DbgLine 2", "DbgLine 3"]
    );
    assert_eq!(lines("\tf(1,\n\t\tb = 2)\n"), ["DbgLine 2", "DbgLine 3"]);
    assert_eq!(
        lines("\tif(f(1,\n\t\t2))\n\t\tf(3)\n"),
        ["DbgLine 2", "DbgLine 3", "DbgLine 2", "DbgLine 4"]
    );
    assert_eq!(lines("\ta = {\"x\n[a]\ny\"}\n"), ["DbgLine 2", "DbgLine 4"]);
    assert_eq!(lines("\ta = a \\\n\t\t+ 1\n"), ["DbgLine 2", "DbgLine 3"]);
    assert_eq!(
        lines("\ta = f(1,\n\t\t2) + 3\n"),
        ["DbgLine 2", "DbgLine 3", "DbgLine 2"]
    );
}
