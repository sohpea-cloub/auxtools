use dreammaker::ast::{AssignOp, Expression, Term};

use super::builtins::{self, Param, Value};
use crate::{
    compiler::{args, CompileError, Compiler, EvalKind},
    operands::{self, DMString, Label, Variable},
    Instruction,
};

fn emit_stack(
    compiler: &mut Compiler<'_>,
    name: &str,
    params: &[Param],
    ins: &Instruction,
    args: &[Expression],
) -> Result<(), CompileError> {
    push_args(compiler, args)?;
    for (at, param) in params.iter().enumerate().skip(args.len()) {
        match param {
            Param::Int(default) => compiler.emit_ins(Instruction::PushInt(*default)),
            Param::Null => push_null(compiler),
            Param::Required => {
                return Err(CompileError::MissingArgument {
                    proc: name.to_owned(),
                    index: at as u32 + 1,
                })
            }
        }
    }
    if args.len() > params.len() {
        return Err(CompileError::TooManyArguments {
            proc: name.to_owned(),
            expected: params.len() as u32,
        });
    }
    compiler.emit_ins(super::versions::instruction(compiler.byond, ins.clone()));
    Ok(())
}

fn emit_counted(
    compiler: &mut Compiler<'_>,
    name: &str,
    (min, max): (u32, u32),
    ins: fn(u32) -> Instruction,
    args: &[Expression],
) -> Result<(), CompileError> {
    let count = args.len() as u32;
    if count < min || count > max {
        return Err(CompileError::IncorrectArgCount(name.to_owned()));
    }
    push_args(compiler, args)?;
    compiler.emit_ins(ins(count));
    Ok(())
}

pub(super) fn emit(
    compiler: &mut Compiler<'_>,
    name: &str,
    args: &Vec<Expression>,
) -> Result<Option<EvalKind>, CompileError> {
    let Some(builtin) = builtins::find(name).filter(|builtin| builtin.is_in(compiler.byond)) else {
        return Ok(None);
    };
    if builtin.code {
        if let Some(kind) = emit_code(compiler, name, args)? {
            return Ok(Some(kind));
        }
    }
    let n = args.len();
    match &builtin.value {
        Value::Stack { params, ins } => {
            emit_stack(compiler, name, params, ins, args)?;
            if matches!(name, "range" | "orange") {
                compiler.emit_ins(Instruction::CopyList);
            }
        }
        Value::Counted { min, max, ins } => emit_counted(compiler, name, (*min, *max), *ins, args)?,
        Value::ByCount { forms } => {
            let form = forms
                .iter()
                .find(|(fewest, most, _)| (*fewest..=*most).contains(&n));
            let Some((_, most, ins)) = form else {
                return Ok(None);
            };
            push_args(compiler, args)?;
            for _ in n..*most {
                push_null(compiler);
            }
            compiler.emit_ins(ins.clone());
        }
        Value::Test { ins } if n >= 1 => emit_tests(compiler, name, ins, args)?,
        Value::Test { .. } | Value::Code => return Ok(None),
        Value::None => {
            return Err(CompileError::UnsupportedBuiltin {
                proc: name.to_owned(),
            })
        }
    }
    Ok(Some(EvalKind::Stack))
}

fn emit_tests(
    compiler: &mut Compiler<'_>,
    name: &str,
    test: &Instruction,
    args: &[Expression],
) -> Result<(), CompileError> {
    let n = args.len();
    if n == 1 && super::versions::folds(compiler.byond, name) {
        let known = super::fold::number(compiler, &args[0])
            .and_then(|x| super::versions::nan_test(name, x));
        if let Some(result) = known {
            compiler.emit_ins(Instruction::PushInt(result as i32));
            if !compiler.byond.folds_nan_tests_cleanly() {
                compiler.warn_bug(
                    crate::diagnostic::Id::ByondFoldedTestFlag,
                    &format!("{name}() of a constant returns the previous test flag on this build"),
                    ", matching DreamMaker",
                );
                compiler.emit_ins(Instruction::GetFlag);
            }
            return Ok(());
        }
    }
    let end = compiler.new_label();
    for arg in args {
        push_args(compiler, std::slice::from_ref(arg))?;
        compiler.emit_ins(test.clone());
        if n > 1 {
            compiler.emit_ins(Instruction::Jz(Label::Named(end.clone())));
        }
    }
    if n > 1 {
        compiler.emit_label(end);
    }
    compiler.emit_ins(Instruction::GetFlag);
    Ok(())
}

fn rgb_slots(args: &[Expression]) -> Option<([Option<Expression>; 5], Option<f32>)> {
    let mut slots: [Option<Expression>; 5] = Default::default();
    let mut names = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        match arg {
            Expression::AssignOp {
                op: AssignOp::Assign,
                lhs,
                rhs,
            } => {
                let name = ident_name(lhs)?;
                let slot = match name.as_str() {
                    "r" | "red" | "h" | "hue" => 0,
                    "g" | "green" | "s" | "saturation" | "c" | "chroma" => 1,
                    "b" | "blue" | "v" | "value" | "l" | "luminance" | "y" => 2,
                    "a" | "alpha" => 3,
                    "space" => 4,
                    _ => return None,
                };
                names.push(name);
                slots[slot] = Some((**rhs).clone());
            }
            _ if i < 5 && names.is_empty() => slots[i] = Some(arg.clone()),
            _ => return None,
        }
    }
    if slots[..3].iter().any(Option::is_none) {
        return None;
    }
    let has = |n: &str| names.iter().any(|x| x == n);
    let space = if has("h") || has("hue") {
        if has("v") || has("value") {
            Some(1.0)
        } else if has("l") || has("luminance") {
            Some(2.0)
        } else {
            Some(3.0)
        }
    } else {
        None
    };
    Some((slots, space))
}

pub(super) fn rgb_constant(compiler: &Compiler<'_>, args: &[Expression]) -> Option<String> {
    let ([r, g, b, a, sp], named_space) = rgb_slots(args)?;
    let space = match (named_space, &sp) {
        (Some(space), _) => space,
        (None, Some(sp)) => super::fold::number(compiler, sp)?,
        (None, None) => 0.0,
    };
    if space.fract() != 0.0 || !(0.0..=3.0).contains(&space) {
        return None;
    }
    let number = |e: &Option<Expression>| super::fold::number(compiler, e.as_ref()?);
    let channels = [number(&r)?, number(&g)?, number(&b)?];
    let alpha = match &a {
        None => None,
        Some(a) => Some(match super::fold::value(compiler, a)? {
            super::fold::Constant::Number(n) => n,
            super::fold::Constant::Null | super::fold::Constant::Text(_) => 0.0,
            _ => return None,
        }),
    };
    let (text, boundary) = super::fold::rgb_text(compiler.byond, space as u32, channels, alpha)?;
    if boundary {
        compiler.warn(crate::diagnostic::Id::FoldedRgb, &format!(
            "rgb() is folded to {text}, matching Linux DreamMaker; Windows may round a channel differently"
        ));
    }
    Some(text)
}

fn emit_rgb(
    compiler: &mut Compiler<'_>,
    args: &[Expression],
) -> Result<Option<EvalKind>, CompileError> {
    let Some((slots, space)) = rgb_slots(args) else {
        return Ok(None);
    };
    if let Some(text) = rgb_constant(compiler, args) {
        compiler.emit_ins(Instruction::PushVal(operands::Value::DMString(DMString(
            text.into_bytes(),
        ))));
        return Ok(Some(EvalKind::Stack));
    }
    let [r, g, b, a, sp] = slots;
    for e in [&r, &g, &b] {
        push_args(compiler, &[e.clone().unwrap()])?;
    }
    if sp.is_none() && space.is_none() {
        match a {
            Some(a) => {
                push_args(compiler, &[a])?;
                compiler.emit_ins(Instruction::Rgba);
            }
            None => compiler.emit_ins(Instruction::Rgb),
        }
        return Ok(Some(EvalKind::Stack));
    }
    let folds_to_zero = |compiler: &Compiler<'_>, a: &Expression| {
        let number =
            |e: &Option<Expression>| e.as_ref().and_then(|e| super::fold::number(compiler, e));
        [&r, &g, &b].iter().all(|e| number(e).is_some())
            && (space.is_some() || number(&sp).is_some())
            && matches!(
                super::fold::value(compiler, a),
                Some(super::fold::Constant::Null | super::fold::Constant::Text(_))
            )
    };
    match a {
        Some(a) if folds_to_zero(compiler, &a) => compiler.emit_number(0.0),
        Some(a) => push_args(compiler, &[a])?,
        None => push_null(compiler),
    }
    match (sp, space) {
        (Some(sp), _) => push_args(compiler, &[sp])?,
        (None, Some(n)) => compiler.emit_number(n),
        (None, None) => unreachable!(),
    }
    compiler.emit_ins(Instruction::RgbEx);
    Ok(Some(EvalKind::Stack))
}

fn is_list_literal(expr: &Expression) -> bool {
    matches!(expr, Expression::Base { term, follow } if follow.is_empty() && matches!(term.elem, Term::List(_)))
}

fn ident_name(expr: &Expression) -> Option<String> {
    match expr {
        Expression::Base { term, follow } if follow.is_empty() => match &term.elem {
            Term::Ident(name) => Some(name.to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn push_args(compiler: &mut Compiler<'_>, args: &[Expression]) -> Result<(), CompileError> {
    args::emit_normal(compiler, args::ArgsContext::Proc, args.to_vec())
}

fn push_null(compiler: &mut Compiler<'_>) {
    compiler.emit_ins(Instruction::PushVal(operands::Value::Null));
}

fn emit_code(
    compiler: &mut Compiler<'_>,
    name: &str,
    args: &[Expression],
) -> Result<Option<EvalKind>, CompileError> {
    let n = args.len();
    match name {
        "browse" | "browse_rsc" | "output" | "load_resource" if n >= 1 => {
            push_args(compiler, &args[..1])?;
            for arg in &args[1..] {
                push_args(compiler, std::slice::from_ref(arg))?;
                compiler.emit_ins(Instruction::Pop);
            }
        }
        "shutdown" if matches!(n, 1 | 2) => {
            push_args(compiler, args)?;
            for _ in n..2 {
                push_null(compiler);
            }
            compiler.emit_ins(Instruction::ShutdownArgs);
        }
        "animate" if n >= 1 => {
            if !emit_statement_builtin(compiler, name, args)? {
                return Ok(None);
            }
            let dropped = compiler.nodes.pop();
            debug_assert!(matches!(
                dropped,
                Some(crate::Node::Instruction(Instruction::Pop, _))
            ));
        }
        "statpanel" if n == 1 => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::StatPanelCheck);
            compiler.emit_ins(Instruction::GetFlag);
        }
        "rgb2num" => {
            let Some(channels) = rgb2num_const(compiler, args) else {
                return Ok(None);
            };
            for c in &channels {
                compiler.emit_ins(Instruction::PushInt(*c as i32));
            }
            compiler.emit_ins(Instruction::NewList(channels.len() as u32));
        }
        "min" | "max" if n == 1 => {
            let list = arglist_inner(&args[0]).unwrap_or(&args[0]).clone();
            args::emit_normal(compiler, args::ArgsContext::Proc, vec![list])?;
            compiler.emit_ins(if name == "min" {
                Instruction::MinList
            } else {
                Instruction::MaxList
            });
        }
        "text"
            if n == 1
                && !matches!(&args[0], Expression::Base { term, .. } if matches!(&term.elem, Term::String(_))) =>
        {
            let kind = compiler.emit_expr(args[0].clone())?;
            compiler.emit_move_to_stack(kind)?;
        }
        "text" if n >= 1 => {
            let mut text = super::strings::TextBuilder::new();
            let mut values: Vec<Expression> = vec![];
            let mut rest = args[1..].iter();
            match &args[0] {
                Expression::Base { term, follow } if follow.is_empty() => match &term.elem {
                    Term::String(s) => text.push(s.as_str())?,
                    Term::InterpString(first, parts) => {
                        text.push(first.as_str())?;
                        for (e, literal) in parts.iter() {
                            text.embed();
                            match e {
                                Some(e) => values.push(e.clone()),
                                None => match rest.next() {
                                    Some(arg) => values.push(arg.clone()),
                                    None => {
                                        return Err(CompileError::IncorrectArgCount(
                                            "text".to_owned(),
                                        ))
                                    }
                                },
                            }
                            text.push(literal.as_str())?;
                        }
                    }
                    _ => return Ok(None),
                },
                _ => return Ok(None),
            }
            if rest.next().is_some() {
                return Err(CompileError::IncorrectArgCount("text".to_owned()));
            }
            let embeds = values.len();
            push_args(compiler, &values)?;
            compiler.emit_ins(Instruction::Format(DMString(text.buf), embeds as u32));
        }
        "REGEX_QUOTE" | "REGEX_QUOTE_REPLACEMENT" if n == 1 => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::PushInt(if name == "REGEX_QUOTE" {
                1
            } else {
                2
            }));
            compiler.emit_ins(Instruction::RegexNew(2));
        }
        "rgb" => return emit_rgb(compiler, args),
        "astype" if n == 1 && compiler.implied_type()?.is_some() => {
            let path = compiler.implied_type()?.unwrap();
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::PushVal(operands::Value::Path(path)));
            compiler.emit_ins(Instruction::AsType);
        }
        "astype" if n == 1 && compiler.static_type_name(&args[0]).is_some() => {
            let path = compiler.static_type_name(&args[0]).unwrap();
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::PushVal(operands::Value::Path(path)));
            compiler.emit_ins(Instruction::AsType);
        }
        "astype" if n == 1 && compiler.debugger => {
            let path = declared_world_type(compiler, &args[0]).ok_or(
                CompileError::UnsupportedBuiltin {
                    proc: "astype with an untyped value".to_owned(),
                },
            )?;
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::PushVal(operands::Value::Path(path)));
            compiler.emit_ins(Instruction::AsType);
        }
        "issaved" if n == 1 => {
            let var = match compiler.emit_inner_expr(args[0].clone())? {
                EvalKind::ListRef => {
                    compiler.emit_ins(Instruction::SetVar(Variable::CacheKey));
                    compiler.emit_ins(Instruction::SetVar(Variable::Cache));
                    Variable::IsSaved(Box::new(Variable::CacheIndex))
                }
                EvalKind::Field(builder, field) => builder.get_leaf(Variable::IsSaved(Box::new(
                    Variable::Field(DMString(field.into())),
                ))),
                EvalKind::Var(var) => Variable::IsSaved(Box::new(var)),
                _ => return Err(CompileError::ExpectedFieldReference),
            };
            compiler.emit_ins(Instruction::GetVar(var));
        }
        "vector" | "pixloc" if (1..=3).contains(&n) => {
            push_args(compiler, args)?;
            let count = n as u32;
            compiler.emit_ins(if name == "vector" {
                Instruction::NewVector(count)
            } else {
                Instruction::NewPixloc(count)
            });
        }
        _ if name.starts_with("step") && movement(name, n).is_some() => {
            let (base, ins) = movement(name, n).unwrap();
            push_args(compiler, args)?;
            for _ in n..base {
                push_null(compiler);
            }
            compiler.emit_ins(ins);
            compiler.emit_ins(Instruction::GetFlag);
        }
        "cmptext" | "cmptextEx" if n >= 2 => {
            let compare = if name == "cmptext" {
                Instruction::CmpText
            } else {
                Instruction::Teq
            };
            let end = compiler.new_label();
            push_args(compiler, &args[..1])?;
            for (i, arg) in args[1..].iter().enumerate() {
                if i > 0 {
                    compiler.emit_ins(Instruction::Jz(Label::Named(end.clone())));
                }
                push_args(compiler, std::slice::from_ref(arg))?;
                compiler.emit_ins(compare.clone());
            }
            if n > 2 {
                compiler.emit_label(end);
            }
            compiler.emit_ins(Instruction::Pop);
            compiler.emit_ins(Instruction::GetFlag);
        }
        "icon" | "sound" | "generator" => {
            compiler.emit_ins(Instruction::PushVal(operands::Value::DMString(DMString(
                format!("/{}", name).into_bytes(),
            ))));
            let count = args.len() as u32;
            match args::emit(compiler, args::ArgsContext::Proc, args.to_vec())? {
                args::ArgsResult::Normal => compiler.emit_ins(Instruction::New(count)),
                args::ArgsResult::Assoc => {
                    compiler.emit_ins(Instruction::NewAssocList(count));
                    compiler.emit_ins(Instruction::NewArgList);
                }
                args::ArgsResult::ArgList => compiler.emit_ins(Instruction::NewArgList),
            }
        }
        "file" if n == 1 => {
            compiler.emit_ins(Instruction::PushVal(operands::Value::File));
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::New(1));
        }
        "image" => {
            let count = args.len() as u32;
            match args::emit(compiler, args::ArgsContext::Proc, args.to_vec())? {
                args::ArgsResult::Normal if count == 2 => compiler.emit_ins(Instruction::NewImage),
                args::ArgsResult::Normal => compiler.emit_ins(Instruction::NewImageArgs(count)),
                args::ArgsResult::Assoc => {
                    compiler.emit_ins(Instruction::NewAssocList(count));
                    compiler.emit_ins(Instruction::NewImageArgList);
                }
                args::ArgsResult::ArgList => compiler.emit_ins(Instruction::NewImageArgList),
            }
        }
        "filter" => {
            let count = args.len() as u32;
            match args::emit(compiler, args::ArgsContext::Proc, args.to_vec())? {
                args::ArgsResult::Assoc => compiler.emit_ins(Instruction::NewAssocList(count)),
                args::ArgsResult::ArgList => {}
                args::ArgsResult::Normal => {
                    return Err(CompileError::IncorrectArgCount(name.to_owned()))
                }
            }
            compiler.emit_ins(Instruction::FilterNewArgList);
        }
        "gradient" => {
            let named = args.iter().any(|a| {
                matches!(
                    a,
                    Expression::AssignOp {
                        op: AssignOp::Assign,
                        ..
                    }
                )
            });
            if named {
                for (i, arg) in args.iter().enumerate() {
                    match arg {
                        Expression::AssignOp {
                            op: AssignOp::Assign,
                            lhs,
                            rhs,
                        } => {
                            let name =
                                ident_name(lhs).ok_or(CompileError::UnexpectedNamedArguments)?;
                            compiler.emit_ins(Instruction::PushVal(operands::Value::DMString(
                                DMString(name.into_bytes()),
                            )));
                            push_args(compiler, &[(**rhs).clone()])?;
                        }
                        _ => {
                            compiler.emit_number((i + 1) as f32);
                            push_args(compiler, std::slice::from_ref(arg))?;
                        }
                    }
                }
                compiler.emit_ins(Instruction::NewAssocList(n as u32));
            } else if let Some(list) = (n == 1).then(|| arglist_inner(&args[0])).flatten() {
                push_args(compiler, std::slice::from_ref(list))?;
            } else if n == 2 && is_list_literal(&args[0]) {
                push_args(compiler, args)?;
                compiler.emit_ins(Instruction::GradientIndex);
                return Ok(Some(EvalKind::Stack));
            } else {
                push_args(compiler, args)?;
                compiler.emit_ins(Instruction::NewList(n as u32));
            }
            compiler.emit_ins(Instruction::Gradient);
        }
        "newlist" => {
            for arg in args {
                push_args(compiler, std::slice::from_ref(arg))?;
                compiler.emit_ins(Instruction::New(0));
            }
            compiler.emit_ins(Instruction::NewList(n as u32));
        }
        "nameof" if n == 1 => {
            let mut name = last_identifier(&args[0]).ok_or(CompileError::ExpectedFieldReference)?;
            if name == "__PROC__" {
                if !compiler.proc_macros_known() {
                    return Err(CompileError::UnsupportedCompilerMacro {
                        name: "__PROC__".to_string(),
                    });
                }
                name = compiler
                    .proc
                    .proc_path
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_owned();
            }
            compiler.emit_ins(Instruction::PushVal(operands::Value::DMString(DMString(
                name.into_bytes(),
            ))));
        }
        "istype" if n == 1 => {
            let path = compiler
                .static_type_name(&args[0])
                .or_else(|| declared_world_type(compiler, &args[0]))
                .ok_or(CompileError::UnsupportedBuiltin {
                    proc: "istype with an untyped value".to_owned(),
                })?;
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::PushVal(operands::Value::Path(path)));
            compiler.emit_ins(Instruction::IsType);
        }
        "arglist" => {
            if n != 1 {
                return Err(CompileError::IncorrectArgCount(name.to_owned()));
            }

            args::emit_single_normal(compiler, args::ArgsContext::Proc, args[0].clone())?;
            return Ok(Some(EvalKind::ArgList));
        }

        "initial" => {
            if n != 1 {
                return Err(CompileError::IncorrectArgCount(name.to_owned()));
            }

            if let Some(var) = initial_static_field(compiler, &args[0])? {
                compiler.emit_ins(Instruction::GetVar(var));
                return Ok(Some(EvalKind::Stack));
            }
            if let Some(var) = initial_safe(compiler, &args[0])? {
                compiler.emit_ins(Instruction::GetVar(var));
                return Ok(Some(EvalKind::Stack));
            }
            let arg = args[0].clone();
            let var = match compiler.emit_expr(arg)? {
                EvalKind::Field(builder, field) => {
                    builder.get_initial_field(DMString(field.into()))
                }

                EvalKind::Var(global @ Variable::Global(_)) => Variable::Initial(Box::new(global)),

                EvalKind::Var(
                    var @ (Variable::Local(_)
                    | Variable::Arg(_)
                    | Variable::Src
                    | Variable::Usr
                    | Variable::Dot),
                ) => Variable::Initial(Box::new(var)),

                EvalKind::ListRef => {
                    compiler.emit_ins(Instruction::SetVar(Variable::CacheKey));
                    compiler.emit_ins(Instruction::SetVar(Variable::Cache));
                    Variable::Initial(Box::new(Variable::CacheIndex))
                }

                _ => return Err(CompileError::ExpectedFieldReference),
            };

            compiler.emit_ins(Instruction::GetVar(var));
        }
        _ => return Ok(None),
    }
    Ok(Some(EvalKind::Stack))
}

fn declared_world_type(compiler: &Compiler, arg: &Expression) -> Option<String> {
    if !compiler.debugger {
        return None;
    }
    let path = super::procs::declared_type_path(compiler, arg).filter(|path| !path.is_empty())?;
    Some(if path.split('/').any(|part| part == "list") {
        "/list".to_owned()
    } else {
        path
    })
}

pub(super) fn last_identifier(expr: &Expression) -> Option<String> {
    use dreammaker::ast::{Follow, Term};
    match expr {
        Expression::Base { term, follow } => match follow.last().map(|f| &f.elem) {
            Some(
                Follow::Field(_, ident) | Follow::StaticField(ident) | Follow::ProcReference(ident),
            ) => Some(ident.as_str().to_owned()),
            Some(_) => None,
            None => match &term.elem {
                Term::Ident(ident) => Some(ident.as_str().to_owned()),
                Term::Prefab(prefab) => prefab
                    .path
                    .to_string()
                    .rsplit('/')
                    .next()
                    .map(str::to_owned),
                Term::Expr(inner) => last_identifier(inner),
                Term::__PROC__ => Some("__PROC__".to_owned()),
                _ => None,
            },
        },
        _ => None,
    }
}

fn movement(name: &str, n: usize) -> Option<(usize, Instruction)> {
    use Instruction::*;
    let (min, base, plain, speed) = match name {
        "step" => (2, 2, Step, StepSpeed),
        "step_to" => (2, 3, StepTo, StepToSpeed),
        "step_away" => (2, 3, StepAway, StepAwaySpeed),
        "step_towards" => (2, 2, StepTowards, StepTowardsSpeed),
        "step_rand" => (1, 1, StepRand, StepRandSpeed),
        "walk" => (2, 3, Walk, WalkSpeed),
        "walk_to" => (2, 4, WalkTo, WalkToSpeed),
        "walk_away" => (2, 4, WalkAway, WalkAwaySpeed),
        "walk_towards" => (2, 3, WalkTowards, WalkTowardsSpeed),
        "walk_rand" => (1, 2, WalkRand, WalkRandSpeed),
        _ => return None,
    };
    match n {
        _ if n < min => None,
        _ if n <= base => Some((base, plain)),
        _ if n == base + 1 => Some((n, speed)),
        _ => None,
    }
}

pub(super) fn emit_statement_builtin(
    compiler: &mut Compiler<'_>,
    name: &str,
    args: &[Expression],
) -> Result<bool, CompileError> {
    let n = args.len();
    let padded = |compiler: &mut Compiler<'_>, total: usize| -> Result<(), CompileError> {
        push_args(compiler, args)?;
        for _ in n..total {
            push_null(compiler);
        }
        Ok(())
    };
    match name {
        "sleep" if n <= 1 => {
            if n == 0 {
                compiler.emit_ins(Instruction::PushInt(0));
            } else {
                push_args(compiler, args)?;
            }
            compiler.emit_ins(Instruction::Sleep);
        }
        "flick" if n == 2 => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::Flick);
        }
        _ if movement(name, n).is_some() => {
            let (base, ins) = movement(name, n).unwrap();
            padded(compiler, base)?;
            compiler.emit_ins(ins);
        }
        "winset" if n == 3 => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::WinSet);
        }
        "rand_seed" if n == 1 => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::RandSeed);
        }
        "shutdown" if n == 0 => compiler.emit_ins(Instruction::Shutdown),
        "shutdown" => {
            push_args(compiler, args)?;
            for _ in n..2 {
                compiler.emit_ins(Instruction::PushVal(operands::Value::Null));
            }
            compiler.emit_ins(Instruction::ShutdownArgs);
            compiler.emit_ins(Instruction::Pop);
        }
        "shell" if n == 1 => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::Shell);
            compiler.emit_ins(Instruction::Pop);
        }
        "animate"
            if n >= 2
                && args.iter().all(|a| {
                    !matches!(a, Expression::AssignOp { .. }) && arglist_inner(a).is_none()
                }) =>
        {
            push_args(compiler, &args[..1])?;
            compiler.emit_ins(Instruction::NullAnimate);
            compiler.emit_ins(Instruction::Pop);
        }
        "animate" if n == 1 && !matches!(args[0], Expression::AssignOp { .. }) => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::NullAnimate);
            compiler.emit_ins(Instruction::Pop);
        }
        "animate" => {
            let count = args.len() as u32;
            match args::emit(compiler, args::ArgsContext::Proc, args.to_vec())? {
                args::ArgsResult::Assoc => compiler.emit_ins(Instruction::NewAssocList(count)),
                args::ArgsResult::ArgList => {}
                args::ArgsResult::Normal => compiler.emit_ins(Instruction::NewList(count)),
            }
            compiler.emit_ins(Instruction::Animate);
            compiler.emit_ins(Instruction::Pop);
        }
        "winshow" if n == 2 || n == 3 => {
            push_args(compiler, args)?;
            if n == 2 {
                compiler.emit_ins(Instruction::PushInt(1));
            }
            compiler.emit_ins(Instruction::WinShow);
        }
        "winclone" if n == 3 => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::WinClone);
        }
        "stat" if n == 2 => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::Stat);
        }
        "stat" if n == 1 => {
            compiler.emit_ins(Instruction::PushVal(operands::Value::Null));
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::Stat);
        }
        "statpanel" if n == 1 => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::StatPanelCheck);
        }
        "statpanel" if n == 2 || n == 3 => {
            push_args(compiler, &args[..1])?;
            if n == 2 {
                push_null(compiler);
            }
            push_args(compiler, &args[1..])?;
            compiler.emit_ins(Instruction::StatPanel);
        }
        "browse" | "browse_rsc" | "output" | "load_resource" if n >= 1 => {
            for arg in args {
                push_args(compiler, std::slice::from_ref(arg))?;
                compiler.emit_ins(Instruction::Pop);
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

pub(super) fn arglist_inner(expr: &Expression) -> Option<&Expression> {
    match expr {
        Expression::Base { term, follow } if follow.is_empty() => match &term.elem {
            Term::Call(name, args) if name.as_str() == "arglist" && args.len() == 1 => {
                Some(&args[0])
            }
            _ => None,
        },
        _ => None,
    }
}

fn initial_static_field(
    compiler: &mut Compiler<'_>,
    expr: &Expression,
) -> Result<Option<Variable>, CompileError> {
    use dreammaker::ast::{Follow, Term};
    let Expression::Base { term, follow } = expr else {
        return Ok(None);
    };
    let Some((last, rest)) = follow.split_last() else {
        return Ok(None);
    };
    let Follow::StaticField(name) = &last.elem else {
        return Ok(None);
    };
    let base = Expression::Base {
        term: term.clone(),
        follow: rest.to_vec().into_boxed_slice(),
    };
    let ty = match &term.elem {
        Term::Prefab(prefab) if rest.is_empty() && prefab.vars.is_empty() => {
            compiler.proc.tree.find(&prefab.path.to_string())
        }
        _ => super::procs::expr_type(compiler, &base),
    };
    let static_scope = ty
        .and_then(|ty| super::procs::member_var(ty, name.as_str()))
        .filter(super::procs::is_global_member)
        .map(|decl| decl.ty().get().path.clone());
    let kind = compiler.emit_expr(base)?;
    compiler.emit_move_to_stack(kind)?;
    compiler.emit_ins(Instruction::SetVar(Variable::Cache));
    Ok(Some(match static_scope {
        Some(scope) => Variable::Initial(Box::new(Variable::Global(
            crate::operands::GlobalVar::named(
                name.as_str().as_bytes(),
                Some(crate::operands::StaticScope::of_type(&scope)),
            ),
        ))),
        None => Variable::Initial(Box::new(Variable::Field(DMString(
            name.as_str().as_bytes().to_vec(),
        )))),
    }))
}

fn initial_safe(
    compiler: &mut Compiler<'_>,
    expr: &Expression,
) -> Result<Option<Variable>, CompileError> {
    use dreammaker::ast::{Follow, PropertyAccessKind as K};
    let Expression::Base { term, follow } = expr else {
        return Ok(None);
    };
    let safe = |f: &Follow| matches!(f, Follow::Field(K::SafeDot | K::SafeColon, _));
    let Some(first) = follow.iter().position(|f| safe(&f.elem)) else {
        return Ok(None);
    };
    if !follow[first..]
        .iter()
        .all(|f| matches!(f.elem, Follow::Field(..)))
    {
        return Ok(None);
    }
    let base = Expression::Base {
        term: term.clone(),
        follow: follow[..first].to_vec().into_boxed_slice(),
    };
    let kind = compiler.emit_expr(base)?;
    compiler.emit_move_to_stack(kind)?;
    let mut segments: Vec<Vec<String>> = vec![];
    for f in &follow[first..] {
        let Follow::Field(_, name) = &f.elem else {
            unreachable!()
        };
        if safe(&f.elem) {
            segments.push(vec![]);
        }
        segments.last_mut().unwrap().push(name.to_string());
    }
    let last = segments.pop().unwrap();
    let saved = segments.len();
    let chain = |fields: &[String]| {
        let mut builder = super::chain_builder::ChainBuilder::begin(Variable::Cache);
        for f in &fields[..fields.len() - 1] {
            builder.append(DMString(f.clone().into_bytes()));
        }
        builder
    };
    for fields in segments {
        compiler.emit_ins(Instruction::SetVar(Variable::Cache));
        compiler.emit_ins(Instruction::PushCache);
        let field = DMString(fields[fields.len() - 1].clone().into_bytes());
        compiler.emit_ins(Instruction::GetVar(chain(&fields).get_field(field)));
    }
    compiler.emit_ins(Instruction::SetVar(Variable::Cache));
    for _ in 0..saved {
        compiler.emit_ins(Instruction::PopCache);
    }
    if let Some(global) = super::statements::static_place(compiler, expr, false) {
        return Ok(Some(Variable::Initial(Box::new(global))));
    }
    let field = DMString(last[last.len() - 1].clone().into_bytes());
    Ok(Some(chain(&last).get_initial_field(field)))
}

const NAMED_COLOURS: &[(&str, &[u8])] = &[
    ("aqua", &[0, 255, 255]),
    ("black", &[0, 0, 0]),
    ("blue", &[0, 0, 255]),
    ("cyan", &[0, 255, 255]),
    ("fuchsia", &[255, 0, 255]),
    ("gold", &[128, 128, 0]),
    ("gray", &[128, 128, 128]),
    ("grey", &[128, 128, 128]),
    ("green", &[0, 192, 0]),
    ("lime", &[0, 255, 0]),
    ("magenta", &[255, 0, 255]),
    ("maroon", &[128, 0, 0]),
    ("navy", &[0, 0, 128]),
    ("olive", &[128, 128, 0]),
    ("purple", &[128, 0, 128]),
    ("red", &[255, 0, 0]),
    ("silver", &[192, 192, 192]),
    ("teal", &[0, 128, 128]),
    ("white", &[255, 255, 255]),
    ("yellow", &[255, 255, 0]),
    ("transparent", &[0, 0, 0, 0]),
    ("none", &[0, 0, 0, 0]),
];

fn colour_const(compiler: &Compiler, expr: &Expression) -> Option<Vec<u8>> {
    let text = match super::fold::value(compiler, expr)? {
        super::fold::Constant::Text(t) => t,
        super::fold::Constant::Null => return Some(vec![255, 255, 255]),
        _ => return None,
    };
    let text = std::str::from_utf8(&text).ok()?;
    let Some(hex) = text.strip_prefix('#') else {
        return NAMED_COLOURS
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(text))
            .map(|(_, channels)| channels.to_vec());
    };
    if !hex.is_ascii() {
        return None;
    }
    match hex.len() {
        3 | 4 => hex
            .chars()
            .map(|c| Some(c.to_digit(16)? as u8 * 17))
            .collect(),
        6 | 8 => (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
            .collect(),
        _ => None,
    }
}

fn rgb2num_space(compiler: &Compiler, args: &[Expression]) -> Option<u32> {
    match args {
        [_] => Some(0),
        [_, space] => match super::fold::value(compiler, space)? {
            super::fold::Constant::Number(n) if (1.0..4.0).contains(&n) => Some(n as u32),
            super::fold::Constant::Number(_)
            | super::fold::Constant::Null
            | super::fold::Constant::Text(_) => Some(0),
            _ => None,
        },
        _ => None,
    }
}

fn rgb2num_const(compiler: &Compiler, args: &[Expression]) -> Option<Vec<u8>> {
    if rgb2num_space(compiler, args)? != 0 {
        return None;
    }
    colour_const(compiler, &args[0])
}

pub(super) fn rgb2num_is_constant(compiler: &Compiler, args: &[Expression]) -> bool {
    rgb2num_space(compiler, args).is_some() && colour_const(compiler, &args[0]).is_some()
}
