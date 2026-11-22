use dmasm_bytecode::{Instruction, Node};
use dmasm_compiler::compiler::{compile_expr, compile_expr_for};

fn instructions(code: &str, params: &[&str]) -> Vec<Instruction> {
    compile_expr(code, params)
        .expect("compiles")
        .into_iter()
        .filter_map(|node| match node {
            Node::Instruction(ins, _) => Some(ins),
            _ => None,
        })
        .collect()
}

#[test]
fn calls_on_a_value_go_by_name_to_find_verbs_too() {
    let code = instructions("x.greet(1, 2)", &["x"]);
    assert!(code.contains(&Instruction::CallName(2)), "{:?}", code);
    let code = instructions("x:greet(a = 1)", &["x"]);
    assert!(code.contains(&Instruction::CallNameArgList), "{:?}", code);
}

#[test]
fn what_follows_an_expression_is_an_error() {
    for code in [
        "x x",
        "x 1",
        "1 x",
        "x) + (x",
        "x + 1 2",
        "f(x) y",
        "x[1] x",
        "\"a\" \"b\"",
        "x, x",
        "x x x x",
    ] {
        let error = compile_expr(code, &["x"]).unwrap_err();
        assert!(
            matches!(error, CompileError::ExpectedEnd),
            "{}: {:?}",
            code,
            error
        );
    }
}

#[test]
fn an_expression_ends_where_it_ends() {
    for code in [
        "x",
        "1",
        "\"a\"",
        "x.y",
        "x.y.z",
        "f(x)",
        "x.f()",
        "x[1]",
        "x++",
        "++x",
        "-x",
        "!x",
        "x + 1",
        "(x)",
        "x ? 1 : 2",
        "x = 1",
        "x += 1",
        "list(1, 2)",
        "list(\"a\" = 1)",
        "new /datum",
        "new /datum(x)",
        "/datum/thing",
        "\"[x]\"",
        "\"[x] and [x.y]\"",
        "x in list(1)",
        "x?.y",
        "x:y",
        "null",
        "x && x",
        "x || x",
        "istype(x, /datum)",
        "x;",
        "x\n",
        " x ",
        "'file.dmi'",
        "1e3",
    ] {
        match compile_expr(code, &["x"]) {
            Err(CompileError::ExpectedEnd) => {
                panic!("{} was taken to have something after it", code)
            }
            _ => {}
        }
    }
}

use dmasm_compiler::compiler::{compile_expr_in, CompileError, ExprGlobal, ExprScope, FoundProc};
use dmasm_compiler::ByondVersion;

fn scoped(code: &str, params: &[&str]) -> Result<Vec<Instruction>, CompileError> {
    let scope = ExprScope {
        has_src_var: &|name| matches!(name, "hp" | "name"),
        has_src_proc: &|name| name == "bump",
        global: &|name| match name {
            "ticks" | "name" => Some(ExprGlobal::Listed),
            "added" => Some(ExprGlobal::Id(77)),
            _ => None,
        },
        globals: "globals",
        proc_ref: &|ty, name| match (ty, name) {
            ("/datum/foo", "double") => Ok(FoundProc {
                tag: 0x26,
                data: 5,
                warning: None,
            }),
            ("/datum/foo/bar" | "/datum/foo/bar/baz", "double") => Ok(FoundProc {
                tag: 0x26,
                data: 6,
                warning: None,
            }),
            _ => Err(format!("{ty} has no proc called {name}")),
        },
        proc_path: &|path| match path {
            "/datum/foo/proc/double" => Some(FoundProc {
                tag: 0x26,
                data: 5,
                warning: None,
            }),
            "/datum/foo/bar/proc/twice" => Some(FoundProc {
                tag: 0x26,
                data: 7,
                warning: Some("not declared on /datum/foo/bar".to_owned()),
            }),
            _ => None,
        },
    };
    let mut params = params.to_vec();
    params.push("globals");
    Ok(
        compile_expr_in(code, &params, Some(&scope), ByondVersion::LATEST)?
            .into_iter()
            .filter_map(|node| match node {
                Node::Instruction(ins, _) => Some(ins),
                _ => None,
            })
            .collect(),
    )
}

fn written_out(code: &str, params: &[&str]) -> Vec<Instruction> {
    let mut params = params.to_vec();
    params.push("globals");
    instructions(code, &params)
}

#[test]
fn a_bare_name_is_a_var_of_src() {
    assert_eq!(scoped("hp", &[]).unwrap(), written_out("src.hp", &[]));
    assert_eq!(
        scoped("hp + 1", &[]).unwrap(),
        written_out("src.hp + 1", &[])
    );
    assert_eq!(
        scoped("hp.len", &[]).unwrap(),
        written_out("src.hp.len", &[])
    );
    assert_eq!(scoped("hp[2]", &[]).unwrap(), written_out("src.hp[2]", &[]));
    assert_eq!(
        scoped("x.hp + hp", &["x"]).unwrap(),
        written_out("x.hp + src.hp", &["x"])
    );
}

#[test]
fn a_bare_name_is_assigned_as_a_var_of_src() {
    assert_eq!(
        scoped("hp = 5", &[]).unwrap(),
        written_out("src.hp = 5", &[])
    );
    assert_eq!(
        scoped("hp += x", &["x"]).unwrap(),
        written_out("src.hp += x", &["x"])
    );
    assert_eq!(scoped("hp++", &[]).unwrap(), written_out("src.hp++", &[]));
}

#[test]
fn a_param_comes_before_a_var_of_src() {
    assert_eq!(scoped("hp", &["hp"]).unwrap(), written_out("hp", &["hp"]));
    assert_eq!(
        scoped("src.hp", &["hp"]).unwrap(),
        written_out("src.hp", &["hp"])
    );
}

#[test]
fn a_var_of_src_comes_before_a_global() {
    assert_eq!(scoped("name", &[]).unwrap(), written_out("src.name", &[]));
}

#[test]
fn a_global_is_read_and_written_through_the_globals_list() {
    assert_eq!(
        scoped("ticks", &[]).unwrap(),
        written_out("globals[\"ticks\"]", &[])
    );
    assert_eq!(
        scoped("ticks + hp", &[]).unwrap(),
        written_out("globals[\"ticks\"] + src.hp", &[])
    );
    assert_eq!(
        scoped("ticks = 3", &[]).unwrap(),
        written_out("globals[\"ticks\"] = 3", &[])
    );
    assert_eq!(
        scoped("ticks.len", &[]).unwrap(),
        written_out("globals[\"ticks\"].len", &[])
    );
}

#[test]
fn a_global_that_isnt_listed_is_read_by_its_id() {
    for code in ["added", "global.added"] {
        let ins = scoped(code, &[]).unwrap();
        assert!(
            matches!(&ins[1], Instruction::GetVar(dmasm_bytecode::Variable::Global(global)) if global.id == Some(77)),
            "{}: {:?}",
            code,
            ins
        );
    }
    let ins = scoped("added = 3", &[]).unwrap();
    assert!(
        ins.iter().any(|i| matches!(i, Instruction::SetVarExpr(dmasm_bytecode::Variable::Global(global)) if global.id == Some(77))),
        "{:?}",
        ins
    );
}

#[test]
fn a_name_nothing_has_is_an_error() {
    let error = scoped("nowhere + 1", &[]).unwrap_err();
    assert!(
        matches!(&error, CompileError::UndefinedVar(name) if name == "nowhere"),
        "{:?}",
        error
    );
    assert_eq!(error.to_string(), "undefined var: nowhere");
    assert!(scoped("hp.nowhere", &[]).is_ok());
    let error = scoped("global.nowhere", &[]).unwrap_err();
    assert!(
        matches!(&error, CompileError::UndefinedVar(name) if name == "nowhere"),
        "{:?}",
        error
    );
}

#[test]
fn a_bare_call_is_src_s_proc_if_it_has_one() {
    assert_eq!(
        scoped("bump(1, hp)", &[]).unwrap(),
        written_out("src.bump(1, src.hp)", &[])
    );
    assert_eq!(
        scoped("other(1)", &[]).unwrap(),
        written_out("other(1)", &[])
    );
    assert_eq!(
        scoped("length(hp)", &[]).unwrap(),
        written_out("length(src.hp)", &[])
    );
}

#[test]
fn what_the_frame_has_keeps_its_meaning() {
    for name in ["src", "usr", ".", "world"] {
        assert_eq!(
            scoped(name, &[]).unwrap(),
            written_out(name, &[]),
            "{}",
            name
        );
    }
}

#[test]
fn global_dot_is_a_global_whatever_src_has() {
    assert_eq!(
        scoped("global.ticks", &[]).unwrap(),
        written_out("globals[\"ticks\"]", &[])
    );
    assert_eq!(
        scoped("global.name", &[]).unwrap(),
        written_out("globals[\"name\"]", &[])
    );
    assert_eq!(
        scoped("global.ticks.len", &[]).unwrap(),
        written_out("globals[\"ticks\"].len", &[])
    );
    assert_eq!(
        scoped("global.ticks = hp", &[]).unwrap(),
        written_out("globals[\"ticks\"] = src.hp", &[])
    );
}

#[test]
fn without_a_scope_a_bare_name_is_a_global() {
    let code = instructions("ticks", &[]);
    assert!(
        matches!(&code[1], Instruction::GetVar(dmasm_bytecode::Variable::Global(global)) if global.name.0 == b"ticks"),
        "{:?}",
        code
    );
}

#[test]
fn a_pointer_made_in_place_is_read_through() {
    use dmasm_bytecode::Variable;
    let code = instructions("*(&x)", &["x"]);
    assert_eq!(
        &code[1..4],
        &[
            Instruction::GetVar(Variable::PtrRef(Box::new(Variable::Arg(0)))),
            Instruction::SetVar(Variable::CacheKey),
            Instruction::GetVar(Variable::PtrDeref(Box::new(Variable::CacheKey))),
        ],
        "{:?}",
        code
    );
    let code = instructions("*(&x) = 5", &["x"]);
    assert!(
        code.iter().any(|i| matches!(i, Instruction::SetVarExpr(Variable::PtrDeref(p)) if **p == Variable::CacheKey)
            || matches!(i, Instruction::SetVar(Variable::PtrDeref(p)) if **p == Variable::CacheKey)),
        "{:?}",
        code
    );
    let code = instructions("*p", &["p"]);
    assert_eq!(
        code[1],
        Instruction::GetVar(Variable::PtrDeref(Box::new(Variable::Arg(0)))),
        "{:?}",
        code
    );
}

#[test]
fn a_type_s_var_by_double_colon_is_its_initial_value() {
    use dmasm_bytecode::Variable;
    let read = |base: Instruction| {
        vec![
            base,
            Instruction::SetVar(Variable::Cache),
            Instruction::GetVar(Variable::Initial(Box::new(Variable::Field(
                dmasm_bytecode::DMString(b"throwforce".to_vec()),
            )))),
        ]
    };
    let path = Instruction::PushVal(dmasm_bytecode::Value::Path("/obj/item".to_owned()));
    for code in [
        "/obj/item::throwforce",
        "initial(/obj/item::throwforce)",
        "(/obj/item)::throwforce",
    ] {
        assert_eq!(
            &instructions(code, &[])[1..4],
            &read(path.clone())[..],
            "{}",
            code
        );
    }
    let arg = Instruction::GetVar(Variable::Arg(0));
    for code in ["x::throwforce", "initial(x::throwforce)"] {
        assert_eq!(
            &instructions(code, &["x"])[1..4],
            &read(arg.clone())[..],
            "{}",
            code
        );
    }
    let ins = scoped("initial(hp::throwforce)", &[]).unwrap();
    assert_eq!(
        &ins[2..4],
        &read(Instruction::GetVar(Variable::Null))[1..3],
        "{:?}",
        ins
    );
    assert!(compile_expr("/obj/item::f()", &[]).is_err());
}

#[test]
fn integers_too_big_for_push_int_are_pushed_whole() {
    use dmasm_bytecode::Value;
    for (code, pushed) in [
        ("65535", Instruction::PushInt(65535)),
        ("65536", Instruction::PushVal(Value::Number(65536.0))),
        ("70000", Instruction::PushVal(Value::Number(70000.0))),
        ("0x11170", Instruction::PushVal(Value::Number(70000.0))),
    ] {
        let code_of = instructions(code, &[]);
        assert_eq!(code_of.get(1), Some(&pushed), "{}: {:?}", code, code_of);
    }
    let code = instructions("-70000", &[]);
    assert!(
        !code
            .iter()
            .any(|ins| matches!(ins, Instruction::PushInt(_))),
        "{:?}",
        code
    );
}

#[test]
fn global_vars_is_the_globals_list() {
    assert_eq!(
        scoped("length(global.vars)", &[]).unwrap(),
        written_out("length(globals)", &[])
    );
    assert_eq!(
        scoped("global.vars[\"ticks\"]", &[]).unwrap(),
        written_out("globals[\"ticks\"]", &[])
    );
    assert_eq!(
        scoped("global.vars[\"ticks\"] = 2", &[]).unwrap(),
        written_out("globals[\"ticks\"] = 2", &[])
    );
    let code = scoped("global.vars.len", &[]).unwrap();
    assert!(
        matches!(
            &code[1..3],
            [
                Instruction::GetVar(dmasm_bytecode::Variable::Arg(0)),
                Instruction::SetVar(dmasm_bytecode::Variable::Cache)
            ]
        ),
        "{:?}",
        code
    );
    assert!(matches!(
        scoped("global.vars = 1", &[]),
        Err(CompileError::ExpectedLValue)
    ));
    assert!(scoped("vars", &[]).is_err());
}

#[test]
fn a_constant_nan_test_is_its_result_on_builds_that_fold_it_cleanly() {
    for (code, result) in [
        ("isnan(1.#INF - 1.#INF)", 1),
        ("isnan(1)", 0),
        ("isinf(-1.#INF)", 1),
    ] {
        let code = instructions(code, &[]);
        assert_eq!(code[1], Instruction::PushInt(result), "{:?}", code);
        assert!(!code.contains(&Instruction::GetFlag), "{:?}", code);
    }
    let old = compile_expr_for("isnan(1)", &[], ByondVersion::new(516, 1659)).unwrap();
    assert!(
        old.iter()
            .any(|n| matches!(n, Node::Instruction(Instruction::GetFlag, _))),
        "{:?}",
        old
    );
}

#[test]
fn a_parser_warning_does_not_refuse_an_expression() {
    for (code, value) in [
        ("4294967295", 4294967295.0f32),
        ("-2147483648", -2147483648.0),
    ] {
        let code = instructions(code, &[]);
        assert!(
            code.contains(&Instruction::PushVal(dmasm_bytecode::Value::Number(value)))
                || code.contains(&Instruction::PushInt(value as i32)),
            "{:?}",
            code
        );
    }
}

#[test]
fn a_types_proc_reference_is_the_one_the_world_finds() {
    use dmasm_bytecode::Value;
    let pushed = |code: &str| {
        scoped(code, &[])
            .unwrap()
            .into_iter()
            .find_map(|ins| match ins {
                Instruction::PushVal(Value::Raw { tag, data }) => Some((tag, data)),
                _ => None,
            })
    };
    assert_eq!(pushed("/datum/foo/bar/baz::double()"), Some((0x26, 6)));
    assert_eq!(pushed("/datum/foo::double()"), Some((0x26, 5)));
    assert_eq!(pushed("call(/datum/foo/bar::double())(1)"), Some((0x26, 6)));
    assert_eq!(pushed("/datum/foo/proc/double"), Some((0x26, 5)));
    let error = scoped("/datum/foo::triple()", &[]).unwrap_err().to_string();
    assert!(
        error.contains("/datum/foo has no proc called triple"),
        "{}",
        error
    );
    assert!(scoped("src::double()", &[]).is_err());
}

#[test]
fn a_proc_path_dreammaker_refuses_is_taken_with_a_warning() {
    let (_, warnings) = dmasm_compiler::compiler::compile_expr_warned(
        "/datum/foo/bar/proc/twice",
        &["globals"],
        Some(&ExprScope {
            has_src_var: &|_| false,
            has_src_proc: &|_| false,
            global: &|_| None,
            globals: "globals",
            proc_ref: &|_, _| Err(String::new()),
            proc_path: &|_| {
                Some(FoundProc {
                    tag: 0x26,
                    data: 7,
                    warning: Some("not declared on /datum/foo/bar".to_owned()),
                })
            },
        }),
        ByondVersion::LATEST,
    )
    .unwrap();
    let warnings: Vec<String> = warnings.iter().map(ToString::to_string).collect();
    assert_eq!(warnings, ["not declared on /datum/foo/bar"]);
}

#[test]
fn a_modified_type_is_refused_in_debugger_code() {
    for code in ["new /datum/foo{val = 8}()", "/datum/foo{val = 8}"] {
        let error = scoped(code, &[]).unwrap_err().to_string();
        assert!(
            error.contains("use new and assign the vars"),
            "{code}: {error}"
        );
    }
}

#[test]
fn pick_draws_as_a_procs_pick_does() {
    let code = instructions("pick(1, 2, x)", &["x"]);
    assert!(
        code.iter()
            .any(|ins| matches!(ins, Instruction::PickSwitch(_))),
        "{:?}",
        code
    );
    let code = instructions("pick(prob(10); 1, 2)", &[]);
    assert!(
        code.iter()
            .any(|ins| matches!(ins, Instruction::PickSwitch(_))),
        "{:?}",
        code
    );
    let code = instructions("pick(x; 1, 2)", &["x"]);
    assert!(
        code.iter()
            .any(|ins| matches!(ins, Instruction::PickProb(_))),
        "{:?}",
        code
    );
}

#[test]
fn a_unary_operator_applies_to_a_safe_chain_that_stops() {
    for (code, op) in [
        ("!a?.f()", Instruction::Not),
        ("-a?.f()", Instruction::UnaryNeg),
        ("!a?.b", Instruction::Not),
    ] {
        let nodes = compile_expr(code, &["a"]).expect("expression compilation failed");
        let target = nodes
            .iter()
            .find_map(|node| match node {
                Node::Instruction(Instruction::SetCacheJmpIfNull(label), _) => Some(label.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{}: no null check in {:?}", code, nodes));
        let at = |want: &dyn Fn(&Node) -> bool| nodes.iter().position(|node| want(node)).unwrap();
        let landing = at(&|node| matches!(node, Node::Label(label) if *label == target));
        let applied = at(&|node| matches!(node, Node::Instruction(ins, _) if *ins == op));
        assert!(landing < applied, "{}: {:?}", code, nodes);
    }
}

#[test]
fn an_assignment_in_parentheses_is_a_methods_argument_not_a_name() {
    let code = instructions("o.f((x = y), 5)", &["o", "x", "y"]);
    assert!(code.contains(&Instruction::CallName(2)), "{:?}", code);
    let code = instructions("o.f(x = y)", &["o", "x", "y"]);
    assert!(code.contains(&Instruction::CallNameArgList), "{:?}", code);
}

#[test]
fn an_expression_folds_constants_as_a_proc_does() {
    let code = instructions("~-4", &[]);
    assert!(
        code.contains(&Instruction::PushInt(16777215))
            || code.contains(&Instruction::PushVal(dmasm_bytecode::Value::Number(
                16777215.0
            ))),
        "{:?}",
        code
    );
    let code = instructions("new /datum ? 1 : 2", &[]);
    assert!(
        !code.iter().any(|ins| matches!(ins, Instruction::New(_))),
        "{:?}",
        code
    );
    let code = instructions("~x", &["x"]);
    assert!(code.contains(&Instruction::Bnot), "{:?}", code);
}

#[test]
fn without_a_scope_an_expression_is_still_the_debugger_s() {
    let error = compile_expr("/obj{name = \"a\"}", &[]).unwrap_err();
    assert!(error.to_string().contains("modified types"), "{error}");
    let error = compile_expr("astype(x)", &["x"]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "unsupported built-in proc: astype with an untyped value"
    );
    let code = instructions("4294967296 | 1", &[]);
    assert!(code.contains(&Instruction::Bor), "{:?}", code);
}

#[test]
fn a_constant_of_byond_s_own_is_its_value() {
    assert_eq!(scoped("NORTH", &[]).unwrap()[1], Instruction::PushInt(1));
    assert_eq!(
        scoped("NORTH | SOUTH", &[]).unwrap()[1],
        Instruction::PushInt(3)
    );
    assert_eq!(scoped("TRUE", &[]).unwrap()[1], Instruction::PushInt(1));
    assert_eq!(instructions("FALSE", &[])[1], Instruction::PushInt(0));
    assert_eq!(
        scoped("NORTH", &["NORTH"]).unwrap()[1],
        Instruction::GetVar(dmasm_bytecode::Variable::Arg(0))
    );
}

#[test]
fn a_proc_of_byond_s_own_types_is_called_as_a_proc_calls_it() {
    let code = instructions("x.Copy(1, 2)", &["x"]);
    assert!(
        matches!(
            &code[..],
            [
                _,
                Instruction::PushInt(1),
                Instruction::PushInt(2),
                Instruction::Call(_, 2),
                ..
            ]
        ),
        "{:?}",
        code
    );
}

#[test]
fn an_expression_records_no_lines_and_names_no_line_in_an_error() {
    let code = compile_expr("x ? \"[x]\" : f(x)", &["x"]).unwrap();
    assert!(
        !code
            .iter()
            .any(|node| matches!(node, Node::Instruction(Instruction::DbgLine(_), _))),
        "{:?}",
        code
    );
    assert_eq!(
        scoped("f(nowhere)", &[]).unwrap_err().to_string(),
        "undefined var: nowhere"
    );
    assert_eq!(
        compile_expr("..()", &[]).unwrap_err().to_string(),
        "relative calls are not supported"
    );
}
