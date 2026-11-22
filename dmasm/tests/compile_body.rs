use dmasm::compiler::{compile_body_in, is_body, CompileError, ExprGlobal, ExprScope, FoundProc};
use dmasm::ByondVersion;

const PARAMS: [&str; 4] = [".", "src", "by", "globals"];

fn with_scope<R>(compile: impl FnOnce(&ExprScope) -> R) -> R {
    let scope = ExprScope {
        has_src_var: &|name| name == "hp",
        has_src_proc: &|name| name == "bump",
        global: &|name| (name == "ticks").then_some(ExprGlobal::Listed),
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
    compile(&scope)
}

fn lines(nodes: &[dmasm::Node]) -> Vec<String> {
    dmasm::format(nodes)
        .lines()
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect()
}

fn body(code: &str) -> Result<(Vec<String>, Vec<String>), CompileError> {
    let compiled =
        with_scope(|scope| compile_body_in(code, &PARAMS, Some(scope), ByondVersion::LATEST))?;
    Ok((lines(&compiled.nodes), compiled.locals))
}

fn until_return(code: Vec<String>) -> Vec<String> {
    let mut labels: Vec<String> = vec![];
    let mut out = vec![];
    for line in code {
        if line.starts_with("DbgLine") {
            continue;
        }
        let renamed = line
            .split_inclusive(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .map(|word| {
                let name =
                    word.trim_end_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '_'));
                if !name.starts_with("LAB_") {
                    return word.to_owned();
                }
                let index = labels.iter().position(|l| l == name).unwrap_or_else(|| {
                    labels.push(name.to_owned());
                    labels.len() - 1
                });
                format!("L{index}{}", &word[name.len()..])
            })
            .collect::<String>();
        let ret = renamed == "Ret";
        out.push(renamed);
        if ret {
            break;
        }
    }
    out
}

#[track_caller]
fn same_alone_and_in_a_body(code: &str) -> Vec<String> {
    let alone = with_scope(|scope| {
        dmasm::compiler::compile_expr_in(code, &PARAMS, Some(scope), ByondVersion::LATEST)
    })
    .unwrap_or_else(|e| panic!("{code} alone: {e}"));
    let (in_body, locals) =
        body(&format!("return {code}")).unwrap_or_else(|e| panic!("{code} in a body: {e}"));
    assert!(locals.is_empty(), "{code}: {locals:?}");
    let alone = until_return(lines(&alone));
    assert_eq!(alone, until_return(in_body), "{code}");
    alone
}

const RETURN: [&str; 6] = [
    "GetVar arg(0)",
    "GetVar arg(1)",
    "GetVar arg(2)",
    "GetVar arg(3)",
    "NewList 5",
    "Ret",
];

#[track_caller]
fn assert_has(code: &[String], lines: &[&str]) {
    assert!(
        code.windows(lines.len()).any(|w| w == lines),
        "expected\n{}\nin\n{}",
        lines.join("\n"),
        code.join("\n")
    );
}

#[test]
fn what_is_a_body() {
    for code in [
        "var/x = 1",
        "x = 1; x",
        "x\ny",
        "return 5",
        "for(var/i in 1 to 3) x += i",
        "if(x) 1",
        "var/n = 0; for(var/i in 1 to 3) n += i; n",
        "  var/x = 1\n  x",
    ] {
        assert!(is_body(code), "{:?} should be a body", code);
    }
    for code in [
        "x",
        "x;",
        "x = 1",
        "f(x)",
        "x + 1\n",
        "",
        "1 +",
        "x x",
        "\"(a = b)\"",
    ] {
        assert!(!is_body(code), "{:?} shouldn't be a body", code);
    }
}

#[test]
fn a_last_statement_that_is_an_expression_is_the_result() {
    let (code, locals) = body("var/n = 0; for(var/i in 1 to 3) n += i; n").unwrap();
    assert_eq!(locals, ["n", "i"]);
    let mut lines = vec!["GetVar local(0)"];
    lines.extend_from_slice(&RETURN[1..]);
    assert_has(
        &code,
        &[
            "GetVar local(0)",
            "GetVar arg(0)",
            "GetVar arg(1)",
            "GetVar arg(2)",
            "GetVar arg(3)",
            "NewList 5",
            "Ret",
        ],
    );
}

#[test]
fn every_way_out_returns_the_params() {
    let (code, _) = body("if(by)\n\treturn 1\nby = 2").unwrap();
    assert_has(
        &code,
        &[
            "PushInt 1",
            "GetVar arg(0)",
            "GetVar arg(1)",
            "GetVar arg(2)",
            "GetVar arg(3)",
            "NewList 5",
            "Ret",
        ],
    );
    assert_has(&code, &["PushInt 2", "SetVar arg(2)"]);
    assert_has(&code, &RETURN);
    for code in ["by = 2", "by += 2", "by++", "--by"] {
        let (text, _) = body(code).unwrap();
        assert_has(&text, &RETURN);
    }
    let (text, _) = body("by = 2\nby + 1").unwrap();
    let (dot, _) = body("by = 2\nreturn").unwrap();
    assert_ne!(text, dot);
    let (code, _) = body("var/x = 1\nreturn").unwrap();
    assert_has(&code, &RETURN);
    assert!(!code.iter().any(|l| l == "End"), "{}", code.join("\n"));
}

#[test]
fn names_are_the_body_s_then_the_frame_s_then_src_s_then_globals() {
    let (code, _) = body("var/x = hp + by + ticks\nx").unwrap();
    assert_has(&code, &["GetVar cache = arg(1); cache[\"hp\"]"]);
    assert_has(&code, &["GetVar arg(2)"]);
    assert_has(&code, &["GetVar arg(3)", "PushVal \"ticks\"", "ListGet"]);
    let (code, _) = body("var/x = bump(1)\nx").unwrap();
    assert_has(&code, &["GetVar arg(1)", "PushVal \"bump\""]);
    assert!(
        code.iter().any(|l| l.starts_with("CallName")),
        "{}",
        code.join("\n")
    );
    let error = body("var/x = 1\nx = nowhere").unwrap_err();
    assert_eq!(error.to_string(), "line 2: undefined var: nowhere");
}

#[test]
fn errors_say_the_line_in_the_code() {
    let error = body("var/x = 1\nx +").unwrap_err();
    assert!(matches!(error, CompileError::ParseError(_)), "{:?}", error);
    assert!(
        error.to_string().starts_with("parser error: 2:"),
        "{}",
        error
    );
}

#[test]
fn a_pointer_and_a_type_s_var() {
    let (code, _) = body("var/x = 5\n*(&x)").unwrap();
    assert_has(
        &code,
        &["GetVar &local(0)", "SetVar cache_key", "GetVar *cache_key"],
    );
    let (code, _) = body("var/x = /obj/item::throwforce\nx").unwrap();
    assert_has(
        &code,
        &[
            "PushVal /obj/item",
            "SetVar cache",
            "GetVar initial(cache[\"throwforce\"])",
        ],
    );
    let (code, _) = body("var/x = initial(/obj::name)\nx").unwrap();
    assert_has(
        &code,
        &[
            "PushVal /obj",
            "SetVar cache",
            "GetVar initial(cache[\"name\"])",
        ],
    );
}

#[test]
fn the_debug_info_names_lines_in_the_code() {
    let (code, _) = body("var/x = 1\n\nx++\nx").unwrap();
    assert_eq!(code[0], "DbgFile \"<dmasm expression>\"");
    assert_has(&code, &["DbgLine 1"]);
    assert_has(&code, &["DbgLine 3"]);
}

#[test]
fn a_call_the_builtin_types_dont_have_goes_by_name() {
    let (code, _) = body("var/mob/M = new\nM.plain_verb()\nM:examinate(M)").expect("compiles");
    let joined = code.join("\n");
    assert!(joined.contains("CallName 0"), "{}", joined);
    assert!(joined.contains("CallName 1"), "{}", joined);
    assert!(!joined.contains("dynamic_proc"), "{}", joined);
    let (code, _) = body("var/mob/M = new\nM.Move(null)").expect("compiles");
    let joined = code.join("\n");
    assert!(!joined.contains("CallName"), "{}", joined);
}

#[test]
fn an_implicit_new_of_a_world_type_makes_the_declared_type() {
    let (code, _) = body("var/datum/apiModel/Error/m = new\nm").expect("compiles");
    assert!(
        code.iter().any(|l| l == "PushVal /datum/apiModel/Error"),
        "{}",
        code.join("\n")
    );
}

#[test]
fn a_body_may_be_indented_with_spaces() {
    let tabs = body("var/n = 0\nfor(var/i in 1 to 3)\n\tn++\nn").unwrap();
    for code in [
        "var/n = 0\nfor(var/i in 1 to 3)\n    n++\nn",
        "var/n = 0\nfor(var/i in 1 to 3)\n  n++\nn",
        "    var/n = 0\n    for(var/i in 1 to 3)\n        n++\n    n",
    ] {
        assert!(is_body(code), "{:?} should be a body", code);
        assert_eq!(body(code).unwrap(), tabs, "{:?}", code);
    }
    let (code, _) = body("var/n = 0\nfor(var/i in 1 to 3)\n    if(i > 1)\n        n++\nn").unwrap();
    assert_has(&code, &["Inc local(0)"]);
    for (code, at) in [
        ("var/x = 1\nif(x)\n    x x", "3:7:"),
        ("var/x = 1\nif(x)\n\tx x", "3:4:"),
        ("  var/x = 1\n  if(x)\n      x x", "3:9:"),
    ] {
        let error = body(code).unwrap_err().to_string();
        assert!(
            error.starts_with(&format!("parser error: {} error: got `x`", at)),
            "{:?}: {}",
            code,
            error
        );
    }
}

#[test]
fn a_body_refuses_the_procs_own_macros_as_an_expression_does() {
    for code in [
        "return __TYPE__",
        "var/t = __TYPE__\nreturn \"[t]\"",
        "return __PROC__",
        "return nameof(__PROC__)",
        "return __TYPE__ ? 1 : 2",
    ] {
        let error = body(code).unwrap_err();
        assert!(
            format!("{}", error).contains("unsupported compiler macro"),
            "{}: {}",
            code,
            error
        );
    }
}

#[test]
fn global_vars_is_the_globals_list() {
    let (code, _) = body("var/n = length(global.vars)\nn").unwrap();
    assert_has(&code, &["GetVar arg(3)", "Length", "SetVar local(0)"]);
}

#[test]
fn bare_vars_is_a_name_like_any_other() {
    let error = body("return vars").unwrap_err().to_string();
    assert_eq!(error, "line 1: undefined var: vars");
    let error = body("var/n = length(vars)\nn").unwrap_err().to_string();
    assert_eq!(error, "line 1: undefined var: vars");
    let unscoped = compile_body_in("return vars", &[], None, ByondVersion::LATEST).unwrap();
    assert!(
        dmasm::format(&unscoped.nodes).contains("GetVar global(\"vars\")"),
        "{}",
        dmasm::format(&unscoped.nodes)
    );
}

#[test]
fn a_builtin_that_is_only_a_statement_is_a_body() {
    for code in [
        "sleep(1)",
        "sleep()",
        "sleep(by)",
        "flick(\"x\", src)",
        "walk(src, 1)",
    ] {
        assert!(is_body(code), "{:?} should be a body", code);
    }
    for code in [
        "sleep",
        "sleep(1) + 1",
        "animate(src, alpha = 0)",
        "rand(1)",
    ] {
        assert!(!is_body(code), "{:?} shouldn't be a body", code);
    }
    let (code, _) = body("sleep(1)").unwrap();
    assert_has(&code, &["PushInt 1", "Sleep"]);
    assert!(
        !code.iter().any(|l| l.contains("sleep")),
        "{}",
        code.join("\n")
    );
    assert_has(&code, &RETURN);
    let (code, _) = body("by = 2; sleep(by)").unwrap();
    assert_has(&code, &["GetVar arg(2)", "Sleep"]);
    assert_has(&code, &RETURN);
}

#[test]
fn istype_of_a_var_with_a_world_type_checks_that_type() {
    let (code, _) =
        body("var/datum/round_event/vent_clog/E = by\nreturn istype(E)").expect("compiles");
    assert!(
        code.iter()
            .any(|l| l == "PushVal /datum/round_event/vent_clog"),
        "{}",
        code.join("\n")
    );
    let (code, _) = body("var/list/L = by\nreturn istype(L)").expect("compiles");
    assert!(
        code.iter().any(|l| l == "PushVal /list"),
        "{}",
        code.join("\n")
    );
    let error = body("var/x = by\nreturn istype(x)").unwrap_err();
    assert!(
        error.to_string().contains("istype with an untyped value"),
        "{}",
        error
    );
}

#[test]
fn a_parser_warning_does_not_refuse_a_body() {
    let (text, _) = body("var/x = 4294967295\nreturn x").expect("compiles");
    assert!(
        text.iter().any(|l| l.starts_with("PushVal 4294967")),
        "{:?}",
        text
    );
}

#[test]
fn a_bare_label_in_a_body_can_be_jumped_to() {
    let (text, _) =
        body("var/i = 0\nagain\ni++\nif (i < 5)\n\tgoto again\nreturn i").expect("compiles");
    assert!(text.iter().any(|l| l == "USER_again:"), "{:?}", text);
    assert!(text.iter().any(|l| l == "JmpLoop USER_again"), "{:?}", text);
}

#[test]
fn astype_of_a_var_with_a_world_type_casts_to_that_type() {
    let (text, _) = body("var/datum/foo/x = null\nreturn astype(x)").expect("compiles");
    let at = text.iter().position(|l| l == "AsType").expect("AsType");
    assert_eq!(text[at - 1], "PushVal /datum/foo", "{:?}", text);
    let (text, _) = body("var/list/datum/foo/L = null\nreturn astype(L)").expect("compiles");
    let at = text.iter().position(|l| l == "AsType").expect("AsType");
    assert_eq!(text[at - 1], "PushVal /list", "{:?}", text);
    let error = body("var/x = null\nreturn astype(x)").unwrap_err();
    assert!(
        format!("{:?}", error).contains("astype with an untyped value"),
        "{:?}",
        error
    );
}

#[test]
fn a_bit_operand_beyond_32_bits_is_left_to_the_world() {
    for code in [
        "return -1e10 | 0",
        "return 2**31 | 0",
        "var/x = 1\nreturn 1e10 & x",
    ] {
        let (text, _) = body(code).expect("compiles");
        assert!(
            text.iter().any(|l| l == "Bor" || l == "Band"),
            "{}: {:?}",
            code,
            text
        );
    }
    let (text, _) = body("return 6 | 1").expect("compiles");
    assert!(text.iter().any(|l| l == "PushInt 7"), "{:?}", text);
}

#[test]
fn a_body_gives_its_warnings_with_its_own_lines() {
    let body = dmasm::compiler::compile_body_in(
        "var/x = 1\nvar/list/L = list(1)\nL.Cut()\nreturn L.len",
        &[],
        None,
        dmasm::ByondVersion::LATEST,
    )
    .unwrap();
    assert!(body.warnings.is_empty(), "{:?}", body.warnings);
    let (_, warnings) = dmasm::compiler::compile_expr_warned(
        "isnan(1)",
        &[],
        None,
        dmasm::ByondVersion::new(516, 1659),
    )
    .unwrap();
    assert!(
        warnings.iter().any(|w| w.contains("previous test flag")),
        "{:?}",
        warnings
    );
    let body = dmasm::compiler::compile_body_in(
        "var/x = 1\nreturn isnan(x) + isnan(1)",
        &[],
        None,
        dmasm::ByondVersion::new(516, 1659),
    )
    .unwrap();
    assert_eq!(
        body.warnings
            .iter()
            .filter(|w| w.starts_with("line 2: ") && w.contains("previous test flag"))
            .count(),
        1,
        "{:?}",
        body.warnings
    );
}

#[test]
fn a_body_can_end_with_a_bare_label() {
    let (code, _) = body("var/y = 1\nif (y) goto done\ny = 2\ndone").expect("compiles");
    let joined = code.join("\n");
    assert!(joined.contains("USER_done:"), "{}", joined);
    let (code, _) = body("var/y = 1\ny").expect("compiles");
    assert_has(&code, &["GetVar local(0)"]);
}

#[test]
fn a_body_without_a_scope_is_the_debugger_s_code_too() {
    let unscoped = |code: &str| {
        compile_body_in(code, &["x"], None, ByondVersion::LATEST).map(|compiled| {
            dmasm::format(&compiled.nodes)
                .lines()
                .map(|l| l.trim().to_owned())
                .collect::<Vec<_>>()
        })
    };
    for code in ["return __PROC__", "return __TYPE__"] {
        let error = unscoped(code).unwrap_err().to_string();
        assert!(
            error.contains("unsupported compiler macro"),
            "{code}: {error}"
        );
    }
    assert_has(
        &unscoped("return x.greet(1)").unwrap(),
        &[
            "GetVar arg(0)",
            "PushVal \"greet\"",
            "PushInt 1",
            "CallName 1",
        ],
    );
    assert_has(
        &unscoped("return /datum/thing::size").unwrap(),
        &[
            "PushVal /datum/thing",
            "SetVar cache",
            "GetVar initial(cache[\"size\"])",
        ],
    );
    let error = unscoped("return /obj{name = \"a\"}")
        .unwrap_err()
        .to_string();
    assert!(error.contains("modified types"), "{error}");
}

#[test]
fn a_body_refuses_calls_to_the_proc_itself_and_its_parent() {
    for code in ["..()", "x = .(1)\nreturn x", "return ..(by)", "if (by) .()"] {
        let error = body(code).unwrap_err().to_string();
        assert!(
            error.ends_with("relative calls are not supported"),
            "{code}: {error}"
        );
    }
}

#[test]
fn an_assignment_to_a_global_is_a_value_too() {
    let written = [
        "PushInt 3",
        "PushTop",
        "GetVar arg(3)",
        "PushVal \"ticks\"",
        "ListSet",
    ];
    for code in [
        "var/a = (ticks = 3)\na",
        "by = ticks = 3",
        "return (global.ticks = 3)",
    ] {
        let (compiled, _) = body(code).unwrap_or_else(|e| panic!("{code}: {e}"));
        assert_has(&compiled, &written);
    }
}

#[test]
fn null_is_read_alone_as_in_a_body() {
    for code in ["null", "by == null", "by ? null : 1", "bump(null, , 1)"] {
        let compiled = same_alone_and_in_a_body(code);
        assert!(
            compiled.iter().any(|l| l == "GetVar null"),
            "{code}: {compiled:?}"
        );
    }
    assert_has(
        &same_alone_and_in_a_body("bump(null, , 1)"),
        &["GetVar null", "PushVal null", "PushInt 1"],
    );
}

#[test]
fn parentheses_give_a_value_alone_as_in_a_body() {
    for code in [
        "(by)",
        "(by).hp",
        "(by.hp).len",
        "&(by)",
        "*(by)",
        "*(&by)",
        "-(by)",
        "bump((by), (hp))",
    ] {
        same_alone_and_in_a_body(code);
    }
    assert_has(
        &same_alone_and_in_a_body("(by).hp"),
        &["GetVar arg(2)", "SetVar cache", "GetVar cache[\"hp\"]"],
    );
}

#[test]
fn a_result_is_regrouped_alone_as_in_a_body() {
    assert_has(
        &same_alone_and_in_a_body("by + (hp + 1) + 2"),
        &["PushInt 1", "Add", "PushInt 2", "Add", "Add"],
    );
    for code in [
        "(by + (hp + 1) + 2)",
        "bump(by + (hp + 1) + 2)",
        "by * (hp * 2) * 3",
        "by | (hp | 1) | 2",
        "by - (hp - 1) - 2",
        "by ? by + (1 + hp) + 2 : 0",
    ] {
        same_alone_and_in_a_body(code);
    }
}

#[test]
fn a_chain_of_and_or_or_is_the_same_alone_as_in_a_body() {
    for code in [
        "by || (hp || ticks)",
        "(by && hp) && (ticks && by)",
        "by && (hp || ticks)",
        "0 || by",
        "\"a\" || by",
        "null && by",
        "by || 0 || hp",
        "by.hp || by.hp.len || 0",
        "bump(1 && by)",
    ] {
        same_alone_and_in_a_body(code);
    }
    let chain = same_alone_and_in_a_body("(by || hp) || ticks");
    assert_eq!(
        chain.iter().filter(|l| l.ends_with(':')).count(),
        1,
        "{chain:?}"
    );
    let folded = same_alone_and_in_a_body("1 && by");
    assert!(!folded.iter().any(|l| l.starts_with("Jmp")), "{folded:?}");
}

#[test]
fn squares_and_bit_constants_are_the_same_alone_as_in_a_body() {
    assert_has(
        &same_alone_and_in_a_body("by ** 2"),
        &["GetVar arg(2)", "PowSquare"],
    );
    assert_has(
        &same_alone_and_in_a_body("by ** -1"),
        &["GetVar arg(2)", "PowNegativeOne"],
    );
    assert_has(
        &same_alone_and_in_a_body("by * by"),
        &["GetVar arg(2)", "PowSquare"],
    );
    assert_has(
        &same_alone_and_in_a_body("hp * hp"),
        &["GetVar cache = arg(1); cache[\"hp\"]", "PowSquare"],
    );
    assert_has(
        &same_alone_and_in_a_body("16777217 & by"),
        &["PushInt 0", "GetVar arg(2)", "Band"],
    );
    for code in [
        "by ** 3",
        "by * by * hp",
        "(by + 1) * (by + 1)",
        "255 & by",
        "by | 255",
        "-1 ^ by",
        "3 * by",
    ] {
        same_alone_and_in_a_body(code);
    }
}

#[test]
fn a_condition_that_sets_the_test_flag_is_not_tested_again_alone_either() {
    assert_has(
        &same_alone_and_in_a_body("by == 1 ? 2 : 3"),
        &["Teq", "Pop", "Jz L0", "PushInt 2"],
    );
    for code in [
        "by in hp ? 1 : 2",
        "isnull(by) ? 1 : 2",
        "(by == 1) ? 2 : 3",
        "by == 1 && hp ? 2 : 3",
        "by == 1 ? by == 2 ? 1 : 2 : 3",
    ] {
        same_alone_and_in_a_body(code);
    }
}

#[test]
fn a_pick_with_weights_ends_the_same_alone_as_in_a_body() {
    let picked = same_alone_and_in_a_body("pick(by; 1, hp; 2)");
    assert_eq!(
        picked.iter().filter(|l| l.starts_with("Jmp ")).count(),
        1,
        "{picked:?}"
    );
    for code in ["pick(by; 1, 2)", "pick(by; 1, hp; pick(ticks; 2, 3))"] {
        same_alone_and_in_a_body(code);
    }
}

#[test]
fn a_safe_chain_is_the_same_alone_as_in_a_body() {
    assert_has(
        &same_alone_and_in_a_body("by?.hp?.len"),
        &[
            "GetVar arg(2)",
            "SetCacheJmpIfNull L0",
            "PushCache",
            "GetVar cache[\"hp\"]",
            "SetCacheJmpIfNull L1",
            "GetVar cache[\"len\"]",
            "L1:",
            "PopCache",
            "L0:",
        ],
    );
    for code in [
        "by?[1]",
        "by?.hp[1]",
        "by?.hp++",
        "--by?.hp",
        "by?.bump().hp",
        "by?.bump()?.bump(hp)",
        "by?.hp?.len == null",
        "!by?.hp",
        "by?.hp = 1",
        "by?.hp += 1",
        "by?.hp ||= 1",
    ] {
        same_alone_and_in_a_body(code);
    }
}

#[test]
fn a_logical_assignment_to_a_safe_place_is_a_value_in_a_body_too() {
    for code in [
        "by?.hp?.len ||= 1",
        "by?[1] ||= 2",
        "by?.hp[1] &&= 2",
        "by?.bump()?.hp ||= 1",
    ] {
        let compiled = same_alone_and_in_a_body(code);
        assert!(
            compiled.iter().any(|l| l.starts_with("SetVarExpr cache")),
            "{code}: {compiled:?}"
        );
    }
}
