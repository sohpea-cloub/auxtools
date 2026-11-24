use std::convert::TryInto;

use dreammaker::ast::{Expression, Field, Follow, Term};

use crate::{
    compiler::{args, builtin_procs, follow, operands, strings, CompileError, Compiler, EvalKind},
    operands::{DMString, GlobalVar, Label, PickProbParams, StaticScope, Value, Variable},
    ByondVersion, Instruction,
};

enum RelativeCall {
    Parent,
    Itself,
}

pub(super) fn emit(compiler: &mut Compiler<'_>, term: Term) -> Result<EvalKind, CompileError> {
    let term_kind = match term {
        Term::ParentCall(_) => RelativeCall::Parent,
        _ => RelativeCall::Itself,
    };
    match term {
        Term::Expr(expr) => {
            compiler.regroup_root = true;
            let kind = compiler.emit_expr(*expr)?;
            compiler.emit_move_to_stack(kind)
        }

        Term::Null => Ok(EvalKind::Var(Variable::Null)),
        Term::Int(i) => {
            compiler.emit_number(i as f32);
            Ok(EvalKind::Stack)
        }
        Term::Float(f) => {
            compiler.emit_ins(Instruction::PushVal(Value::Number(f)));
            Ok(EvalKind::Stack)
        }
        Term::String(str) => {
            compiler.emit_ins(Instruction::PushVal(Value::DMString(strings::parse(&str)?)));
            Ok(EvalKind::Stack)
        }

        Term::Ident(ident) => {
            if super::procs::is_builtin_const(compiler, ident.as_str()) {
                let value = super::procs::const_value(compiler, ident.as_str()).unwrap();
                compiler.emit_constant(value)?;
                return Ok(EvalKind::Stack);
            }
            compiler.emit_find_var(ident)
        }
        Term::GlobalIdent(ident) => Ok(EvalKind::Var(Variable::Global(GlobalVar::named(
            ident.as_str().as_bytes(),
            Some(StaticScope::Global),
        )))),

        Term::Resource(resource) => {
            compiler.emit_ins(Instruction::PushVal(Value::Resource(resource)));
            Ok(EvalKind::Stack)
        }

        Term::As(val) => {
            compiler.emit_ins(Instruction::PushInt(val.bits() as i32));
            Ok(EvalKind::Stack)
        }

        Term::Prefab(prefab) => {
            if !prefab.vars.is_empty() {
                let value = prefab_value(compiler, &prefab)?;
                compiler.emit_ins(Instruction::PushVal(value));
                return Ok(EvalKind::Stack);
            }

            let mut path = prefab.path.to_string();
            if let Some(resolved) = super::procs::resolve_proc_path(compiler, &path) {
                path = resolved;
            }
            if let Some(found) = compiler
                .expr_scope
                .and_then(|scope| (scope.proc_path)(&path))
            {
                compiler.emit_found_proc(found);
                return Ok(EvalKind::Stack);
            }
            if path.ends_with("/proc") || path.ends_with("/verb") {
                compiler.emit_ins(Instruction::PushVal(Value::DMString(DMString(
                    path.into_bytes(),
                ))));
                return Ok(EvalKind::Stack);
            }

            compiler.emit_ins(Instruction::PushVal(Value::Path(path)));
            Ok(EvalKind::Stack)
        }

        Term::GlobalCall(ident, args) => {
            let path = operands::Proc(format!("/proc/{}", ident));
            let arg_count = args.len() as u32;
            match args::emit(compiler, args::ArgsContext::Proc, args.into_vec())? {
                args::ArgsResult::Normal => {
                    compiler.emit_ins(Instruction::CallGlob(arg_count, path));
                }
                args::ArgsResult::Assoc => {
                    compiler.emit_ins(Instruction::NewAssocList(arg_count));
                    compiler.emit_ins(Instruction::CallGlobalArgList(path));
                }
                args::ArgsResult::ArgList => {
                    compiler.emit_ins(Instruction::CallGlobalArgList(path));
                }
            }
            Ok(EvalKind::Stack)
        }

        Term::Call(ident, args) => {
            let args_vec = args.into_vec();
            match builtin_procs::emit(compiler, &ident, &args_vec)? {
                Some(kind) => Ok(kind),

                None => {
                    if let Some(kind) =
                        super::procs::emit_bare_call(compiler, &ident, args_vec.clone())?
                    {
                        return Ok(kind);
                    }
                    if compiler
                        .expr_scope
                        .is_some_and(|scope| (scope.has_src_proc)(ident.as_str()))
                    {
                        return compiler.emit_expr(super::src_member(Follow::Call(
                            dreammaker::ast::PropertyAccessKind::Dot,
                            ident.clone(),
                            args_vec.into(),
                        )));
                    }
                    let arg_count = args_vec.len() as u32;

                    match args::emit(compiler, args::ArgsContext::Proc, args_vec)? {
                        args::ArgsResult::Normal => {
                            compiler.emit_ins(Instruction::CallGlob(
                                arg_count,
                                operands::Proc(format!("/proc/{}", ident)),
                            ));
                        }

                        args::ArgsResult::Assoc => {
                            compiler.emit_ins(Instruction::NewAssocList(arg_count));
                            compiler.emit_ins(Instruction::CallGlobalArgList(operands::Proc(
                                format!("/proc/{}", ident),
                            )));
                        }

                        args::ArgsResult::ArgList => {
                            compiler.emit_ins(Instruction::CallGlobalArgList(operands::Proc(
                                format!("/proc/{}", ident),
                            )));
                        }
                    }

                    Ok(EvalKind::Stack)
                }
            }
        }

        Term::DynamicCall(lhs, rhs) => {
            let lhs_len = lhs.len();
            let rhs_len = rhs.len();

            if lhs.is_empty() {
                return Err(CompileError::MissingArgument {
                    proc: "call".to_owned(),
                    index: 1,
                });
            }

            if lhs_len > 2 {
                return Err(CompileError::TooManyArguments {
                    proc: "call".to_owned(),
                    expected: 2,
                });
            }

            for (i, expr) in lhs.iter().enumerate() {
                compiler.proc_macro_ok = lhs_len == 2 && i == 1;
                let kind = compiler.emit_expr(expr.clone());
                compiler.proc_macro_ok = false;
                compiler.emit_move_to_stack(kind?)?;
            }

            match args::emit(compiler, args::ArgsContext::Proc, rhs.into_vec())? {
                args::ArgsResult::Normal => match lhs_len {
                    1 => compiler.emit_ins(Instruction::CallPath(rhs_len as u32)),
                    2 => compiler.emit_ins(Instruction::CallName(rhs_len as u32)),

                    _ => unreachable!(),
                },

                args::ArgsResult::Assoc => {
                    compiler.emit_ins(Instruction::NewAssocList(rhs_len as u32));

                    match lhs_len {
                        1 => compiler.emit_ins(Instruction::CallPathArgList),
                        2 => compiler.emit_ins(Instruction::CallNameArgList),

                        _ => unreachable!(),
                    }
                }

                args::ArgsResult::ArgList => match lhs_len {
                    1 => compiler.emit_ins(Instruction::CallPathArgList),
                    2 => compiler.emit_ins(Instruction::CallNameArgList),

                    _ => unreachable!(),
                },
            }

            Ok(EvalKind::Stack)
        }

        Term::SelfCall(args) | Term::ParentCall(args) if !compiler.debugger => {
            let parent = matches!(term_kind, RelativeCall::Parent);
            let args = args.into_vec();
            if args.is_empty() {
                compiler.emit_ins(if parent {
                    Instruction::CallParent
                } else {
                    Instruction::CallSelf
                });
                return Ok(EvalKind::Stack);
            }
            let count = args.len() as u32;
            match args::emit(compiler, args::ArgsContext::Proc, args)? {
                args::ArgsResult::Normal => compiler.emit_ins(if parent {
                    Instruction::CallParentArgs(count)
                } else {
                    Instruction::CallSelfArgs(count)
                }),
                args::ArgsResult::Assoc => {
                    compiler.emit_ins(Instruction::NewAssocList(count));
                    compiler.emit_ins(if parent {
                        Instruction::CallParentArgList
                    } else {
                        Instruction::CallSelfArgList
                    });
                }
                args::ArgsResult::ArgList => compiler.emit_ins(if parent {
                    Instruction::CallParentArgList
                } else {
                    Instruction::CallSelfArgList
                }),
            }
            Ok(EvalKind::Stack)
        }

        Term::SelfCall { .. } | Term::ParentCall { .. } => {
            Err(CompileError::UnsupportedRelativeCall)
        }

        Term::NewImplicit { args } if compiler.implicit_type.is_some() => {
            let path = compiler.implicit_type.clone().unwrap();
            compiler.emit_ins(Instruction::PushVal(Value::Path(path)));
            emit_new(compiler, args)
        }
        Term::NewImplicit { .. } => Err(CompileError::UnsupportedImplicitNew),
        Term::NewMiniExpr { expr, args } => {
            if expr.ident.as_str() == "src" && !expr.fields.is_empty() {
                compiler.explicit_src = true;
            }
            let ty = super::procs::term_type(compiler, &Term::Ident(expr.ident.clone()));
            let var = compiler.emit_find_var(expr.ident.into())?;
            let follows: Vec<Follow> = expr
                .fields
                .iter()
                .map(|f: &Field| f.clone().into())
                .collect();

            let kind = follow::emit_typed(compiler, follows, var, ty)?;
            compiler.emit_move_to_stack(kind)?;

            emit_new(compiler, args)
        }
        Term::NewPrefab { prefab, args } => {
            let typeval = if prefab.vars.is_empty() {
                operands::Value::Path(prefab.path.to_string())
            } else {
                prefab_value(compiler, &prefab)?
            };
            compiler.emit_ins(Instruction::PushVal(typeval));

            emit_new(compiler, args)
        }

        Term::Locate { args, in_list } => {
            let args_len = args.len();

            args::emit_normal(compiler, args::ArgsContext::Proc, args.into_vec())?;

            match args_len {
                0 if compiler.implicit_type.is_some() => {
                    let path = compiler.implicit_type.clone().unwrap();
                    compiler.emit_ins(Instruction::PushVal(Value::Path(path)));
                    match in_list {
                        Some(list) => {
                            let kind = compiler.emit_expr(*list)?;
                            compiler.emit_move_to_stack(kind)?;
                            compiler.emit_ins(Instruction::LocateType);
                        }
                        None => compiler.emit_ins(Instruction::LocateRef),
                    }
                }
                0 => return Err(CompileError::UnsupportedImplicitLocate),

                1 if in_list.is_none() => {
                    compiler.emit_ins(Instruction::LocateRef);
                }

                1 if in_list.is_some() => {
                    let kind = compiler.emit_expr(*in_list.unwrap())?;
                    compiler.emit_move_to_stack(kind)?;

                    compiler.emit_ins(Instruction::LocateType);
                }

                2 if in_list.is_none() => {
                    compiler.emit_ins(Instruction::LocateType);
                }

                3 => {
                    compiler.emit_ins(Instruction::LocatePos);
                }

                _ => return Err(CompileError::InvalidLocateArgs),
            }

            Ok(EvalKind::Stack)
        }

        Term::Pick(args) => {
            match args.len() {
                0 => {
                    return Err(CompileError::MissingArgument {
                        proc: "pick".to_owned(),
                        index: 1,
                    })
                }

                1 => {
                    let (lhs, rhs) = args.into_vec().pop().unwrap();

                    if lhs.is_some() {
                        return Err(CompileError::UnexpectedProbability);
                    }

                    let rhs = super::builtin_procs::arglist_inner(&rhs)
                        .cloned()
                        .unwrap_or(rhs);
                    let kind = compiler.emit_expr(rhs)?;
                    compiler.emit_move_to_stack(kind)?;
                    compiler.emit_ins(Instruction::Pick);
                }

                _ if args.iter().all(|(w, _)| {
                    w.as_ref()
                        .map_or(true, |w| pick_weight(compiler, w).is_some())
                }) =>
                {
                    let weights: Vec<f32> = args
                        .iter()
                        .map(|(w, _)| {
                            w.as_ref()
                                .map_or(100.0, |w| pick_weight(compiler, w).unwrap())
                        })
                        .collect();
                    let total: f32 = weights.iter().sum();
                    let end = compiler.new_label();
                    let labels: Vec<String> =
                        (0..args.len()).map(|_| compiler.new_label()).collect();
                    let mut threshold = 0u32;
                    let mut cases = vec![];
                    let keeps_zero = compiler.byond.keeps_zero_pick_weight();
                    let zero: Vec<bool> = args
                        .iter()
                        .map(|(w, _)| {
                            !keeps_zero
                                && w.as_ref()
                                    .is_some_and(|w| super::fold::number(compiler, w) == Some(0.0))
                        })
                        .collect();
                    let mut last = args.len() - 1;
                    if zero[last] {
                        if let Some(live) = zero.iter().rposition(|z| !z) {
                            compiler.warn_bug(
                                crate::diagnostic::Id::ByondZeroWeightPick,
                                "pick() ends with a zero-weight entry, where DreamMaker's code jumps out of the proc",
                                "; dmasm selects the last entry whose weight is not a constant zero instead of reproducing DreamMaker's invalid jump",
                            );
                            last = live;
                        }
                    }
                    for ((weight, label), zero) in weights.iter().zip(&labels).zip(zero).take(last)
                    {
                        if zero {
                            continue;
                        }
                        threshold += (*weight as f64 / total as f64 * 65535.0).floor() as u32;
                        cases.push((threshold, Label::Named(label.clone())));
                    }
                    compiler.emit_ins(Instruction::PickSwitch(crate::PickSwitchParams {
                        default: Label::Named(labels[last].clone()),
                        cases,
                    }));
                    let count = args.len();
                    for (i, ((_, value), label)) in
                        args.into_vec().into_iter().zip(labels).enumerate()
                    {
                        compiler.emit_label(label);
                        let kind = compiler.emit_expr(value)?;
                        compiler.emit_move_to_stack(kind)?;
                        if i + 1 < count {
                            compiler.emit_ins(Instruction::Jmp(Label::Named(end.clone())));
                        }
                    }
                    compiler.emit_label(end);
                }

                _ => {
                    let label_end = format!("LAB_{:0>4X}", compiler.label_count);
                    compiler.label_count += 1;

                    let mut branches = vec![];

                    for (lhs, rhs) in args {
                        let lhs = lhs.map(|w| match &w {
                            Expression::Base { term, follow } if follow.is_empty() => match &term
                                .elem
                            {
                                Term::Call(name, a) if name.as_str() == "prob" && a.len() == 1 => {
                                    a[0].clone()
                                }
                                _ => w,
                            },
                            _ => w,
                        });
                        match lhs {
                            Some(lhs) => {
                                let kind = compiler.emit_expr(lhs)?;
                                compiler.emit_move_to_stack(kind)?;
                            }

                            None => compiler.emit_ins(Instruction::PushInt(100)),
                        }

                        branches.push((format!("LAB_{:0>4X}", compiler.label_count), rhs));
                        compiler.label_count += 1;
                    }

                    compiler.emit_ins(Instruction::PickProb(PickProbParams {
                        cases: branches
                            .iter()
                            .map(|(label, _)| Label::Named(label.to_owned()))
                            .collect(),
                    }));

                    let count = branches.len();
                    for (i, (label, expr)) in branches.into_iter().enumerate() {
                        compiler.emit_label(label);

                        let kind = compiler.emit_expr(expr)?;
                        compiler.emit_move_to_stack(kind)?;

                        if i + 1 < count {
                            compiler.emit_ins(Instruction::Jmp(Label::Named(label_end.clone())));
                        }
                    }

                    compiler.emit_label(label_end);
                }
            }

            Ok(EvalKind::Stack)
        }

        Term::Alist(args) => {
            super::versions::require(compiler.byond, ByondVersion::is_516, "alist()")?;
            emit_alist(compiler, args.into_vec())
        }

        Term::List(args) => {
            let arg_count = args.len();

            match args::emit(compiler, args::ArgsContext::List, args.into_vec())? {
                args::ArgsResult::Normal => {
                    compiler.emit_ins(Instruction::NewList(arg_count as u32));
                }

                args::ArgsResult::Assoc => {
                    compiler.emit_ins(Instruction::NewAssocList(arg_count as u32));
                }

                args::ArgsResult::ArgList => return Err(CompileError::UnexpectedArgList),
            }

            Ok(EvalKind::Stack)
        }

        Term::ExternalCall {
            library,
            function,
            args,
        } => {
            let Some(library) = library else {
                let kind = compiler.emit_expr(*function)?;
                compiler.emit_move_to_stack(kind)?;
                let args = args.into_vec();
                let count = args.len() as u32;
                return match args::emit(compiler, args::ArgsContext::Proc, args)? {
                    args::ArgsResult::Normal => {
                        compiler.emit_ins(Instruction::CallExtLoaded(count));
                        Ok(EvalKind::Stack)
                    }
                    args::ArgsResult::ArgList => {
                        compiler.emit_ins(Instruction::CallExtLoadedArgList);
                        Ok(EvalKind::Stack)
                    }
                    args::ArgsResult::Assoc => Err(CompileError::UnexpectedNamedArguments),
                };
            };
            for part in [*library, *function] {
                let kind = compiler.emit_expr(part)?;
                compiler.emit_move_to_stack(kind)?;
            }

            let args_vec = args.into_vec();
            let arg_amount: u32 = args_vec.len().try_into().unwrap();
            args::emit_normal(compiler, args::ArgsContext::Proc, args_vec)?;
            compiler.emit_ins(Instruction::CallLib(arg_amount));

            Ok(EvalKind::Stack)
        }

        Term::InterpString(first, parts) => {
            let mut text = strings::TextBuilder::new();
            text.push(first.as_str())?;
            let mut count = 0;
            for (expr, literal) in parts.into_vec() {
                if let Some(expr) = expr {
                    let kind = compiler.emit_expr(expr)?;
                    compiler.emit_move_to_stack(kind)?;
                    count += 1;
                }
                text.embed();
                text.push(literal.as_str())?;
            }
            if count == 0 {
                compiler.emit_ins(Instruction::PushVal(Value::DMString(DMString(text.buf))));
            } else {
                compiler.emit_ins(Instruction::Format(DMString(text.buf), count));
            }
            Ok(EvalKind::Stack)
        }
        Term::Input {
            args,
            input_type,
            in_list,
        } => emit_input(compiler, args.into_vec(), input_type, in_list.map(|l| *l)),
        Term::__TYPE__ if compiler.proc_macros_known() && !compiler.proc.ty.is_root() => {
            let path = compiler.proc.ty.get().path.clone();
            compiler.emit_ins(Instruction::PushVal(Value::Path(path)));
            Ok(EvalKind::Stack)
        }
        Term::__TYPE__ if compiler.proc_macros_known() => {
            compiler.emit_ins(Instruction::PushVal(Value::Null));
            Ok(EvalKind::Stack)
        }
        Term::__PROC__ if compiler.proc_macros_known() => {
            if !compiler.proc_macro_ok {
                super::versions::require(compiler.byond, ByondVersion::has_proc_macro, "__PROC__")?;
            }
            let path = compiler.proc.proc_path.clone();
            compiler.emit_ins(Instruction::PushVal(Value::Path(path)));
            Ok(EvalKind::Stack)
        }
        Term::__IMPLIED_TYPE__ if compiler.implicit_type.is_some() => {
            let path = compiler.implicit_type.clone().unwrap();
            compiler.emit_ins(Instruction::PushVal(Value::Path(path)));
            Ok(EvalKind::Stack)
        }
        Term::__TYPE__ => Err(CompileError::UnsupportedCompilerMacro {
            name: "__TYPE__".to_string(),
        }),
        Term::__IMPLIED_TYPE__ => Err(CompileError::UnsupportedCompilerMacro {
            name: "__IMPLIED_TYPE__".to_string(),
        }),
        Term::__PROC__ => Err(CompileError::UnsupportedCompilerMacro {
            name: "__PROC__".to_string(),
        }),
    }
}

fn emit_input(
    compiler: &mut Compiler<'_>,
    args: Vec<Expression>,
    input_type: Option<dreammaker::ast::InputType>,
    in_list: Option<Expression>,
) -> Result<EvalKind, CompileError> {
    use dreammaker::ast::InputType;
    if args.is_empty() || args.len() > 4 {
        return Err(CompileError::IncorrectArgCount("input".to_owned()));
    }
    let builtin = in_list.as_ref().and_then(|l| input_source(compiler, l));
    let mut types = input_type.unwrap_or_default().bits();
    match (input_type, builtin) {
        (None, Some((_, 8))) => types = InputType::OBJ.bits(),
        (None, Some(_)) => types = InputType::ATOM.bits(),
        _ => {}
    }
    let wide = types > 0xFFFF;
    if wide {
        compiler.emit_ins(Instruction::PushVal(Value::Number(types as f32)));
    }
    let (distance, source) = match (builtin, in_list) {
        (Some(b), _) => b,
        (None, Some(list)) => {
            let one_arg_view = matches!(&list, Expression::Base { term, follow } if follow.is_empty()
                && matches!(&term.elem, Term::Call(name, args) if args.len() == 1
                    && matches!(name.as_str(), "view" | "oview" | "range" | "orange")));
            let kind = compiler.emit_expr(list)?;
            compiler.emit_move_to_stack(kind)?;
            (if one_arg_view { 125 } else { 0 }, 64)
        }
        (None, None) => (0, 0),
    };
    let n = args.len();
    args::emit_normal(compiler, args::ArgsContext::Proc, args)?;
    for _ in n..4 {
        compiler.emit_ins(Instruction::PushVal(Value::Null));
    }
    if wide {
        compiler.emit_ins(Instruction::InputColor(0, distance, source));
    } else {
        compiler.emit_ins(Instruction::Input(types, distance, source));
    }
    compiler.emit_ins(Instruction::PromptCheck);
    Ok(EvalKind::Stack)
}

fn input_source(compiler: &Compiler, list: &Expression) -> Option<(u32, u32)> {
    let Expression::Base { term, follow } = list else {
        return None;
    };
    match &term.elem {
        Term::Ident(name) if name.as_str() == "world" && follow.is_empty() => Some((127, 16)),
        Term::Ident(name) if name.as_str() == "usr" => {
            let contents = match &follow[..] {
                [] => true,
                [f] => matches!(&f.elem, Follow::Field(_, field) if field.as_str() == "contents"),
                _ => false,
            };
            contents.then_some((127, 8))
        }
        Term::Call(name, args) if follow.is_empty() => {
            let source = match name.as_str() {
                "view" => 1,
                "oview" => 2,
                "range" => 5,
                "orange" => 6,
                _ => return None,
            };
            let distance = match &args[..] {
                [] => 125,
                [arg] if is_usr(arg) => 125,
                [arg] => {
                    let n = super::fold::number(compiler, arg)?;
                    if n < 0.0 || n.fract() != 0.0 || n >= 125.0 {
                        return None;
                    }
                    n as u32
                }
                _ => return None,
            };
            Some((distance, source))
        }
        _ => None,
    }
}

fn is_usr(expr: &Expression) -> bool {
    matches!(expr, Expression::Base { term, follow } if follow.is_empty() && matches!(&term.elem, Term::Ident(n) if n.as_str() == "usr"))
}

pub(super) fn emit_alist(
    compiler: &mut Compiler<'_>,
    args: Vec<Expression>,
) -> Result<EvalKind, CompileError> {
    let n = args.len() as u32;
    let positional = |arg: &Expression| {
        !matches!(
            arg,
            Expression::AssignOp {
                op: dreammaker::ast::AssignOp::Assign,
                ..
            }
        ) && super::builtin_procs::arglist_inner(arg).is_none()
    };
    if args.iter().any(positional) {
        for arg in args {
            let lone = positional(&arg);
            match args::emit(compiler, args::ArgsContext::List, vec![arg])? {
                args::ArgsResult::Assoc if !lone => {}
                args::ArgsResult::Normal if lone => {
                    compiler.emit_ins(Instruction::PushVal(crate::operands::Value::Null));
                }
                _ => return Err(CompileError::AmbiguousListConstructor),
            }
        }
        compiler.emit_ins(Instruction::NewAlist(n));
        return Ok(EvalKind::Stack);
    }
    match args::emit(compiler, args::ArgsContext::List, args)? {
        args::ArgsResult::Assoc => {}
        args::ArgsResult::Normal if n == 0 => {}
        _ => return Err(CompileError::AmbiguousListConstructor),
    }
    compiler.emit_ins(Instruction::NewAlist(n));
    Ok(EvalKind::Stack)
}

fn pick_weight(compiler: &Compiler, expr: &Expression) -> Option<f32> {
    if let Some(n) = super::fold::number(compiler, expr) {
        return Some(n);
    }
    if let Some(super::fold::Constant::Text(_)) = super::fold::value(compiler, expr) {
        return Some(0.0);
    }
    if let Expression::Base { term, follow } = expr {
        if follow.is_empty() {
            if let Term::Call(name, args) = &term.elem {
                if name.as_str() == "prob" && args.len() == 1 {
                    return super::fold::number(compiler, &args[0]);
                }
            }
        }
    }
    None
}

fn emit_new(
    compiler: &mut Compiler<'_>,
    args: Option<Box<[Expression]>>,
) -> Result<EvalKind, CompileError> {
    let args = args.unwrap_or_else(|| Box::new([])).into_vec();
    let arg_count = args.len() as u32;

    match args::emit(compiler, args::ArgsContext::Proc, args)? {
        args::ArgsResult::Normal => {
            compiler.emit_ins(Instruction::New(arg_count));
        }

        args::ArgsResult::Assoc => {
            compiler.emit_ins(Instruction::NewAssocList(arg_count));
            compiler.emit_ins(Instruction::NewArgList);
        }

        args::ArgsResult::ArgList => {
            compiler.emit_ins(Instruction::NewArgList);
        }
    }

    Ok(EvalKind::Stack)
}

fn prefab_value(
    compiler: &mut Compiler,
    prefab: &dreammaker::ast::Prefab,
) -> Result<Value, CompileError> {
    if compiler.debugger && !prefab.vars.is_empty() {
        return Err(CompileError::UnsupportedStatement(
            "modified types (/type{var = value}) are unsupported in debugger code; use new and assign the vars",
        ));
    }
    let saved = std::mem::take(&mut compiler.nodes);
    let saved_cache = compiler.cache.take();
    let saved_line = compiler.last_line.take();
    let saved_pending = std::mem::take(&mut compiler.line_pending);
    let mut vars = vec![];
    let mut res = Ok(());
    for (name, value) in prefab.vars.iter() {
        compiler.cache = None;
        compiler.last_line = None;
        let mut emit = || -> Result<(), CompileError> {
            let literal = matches!(value, Expression::Base { term, follow }
                if follow.is_empty()
                    && matches!(term.elem, Term::Resource(_) | Term::Prefab(_) | Term::String(_) | Term::Null));
            if !literal
                && super::fold::value(compiler, value)
                    .is_none_or(|v| v == super::fold::Constant::New)
            {
                let line = value_line(value);
                let file = compiler.proc.file.clone();
                compiler.emit_ins(Instruction::DbgLine(line));
                compiler.emit_ins(Instruction::DbgFile(crate::operands::DMString(
                    file.into_bytes(),
                )));
            }
            let kind = compiler.emit_expr(value.clone())?;
            compiler.emit_move_to_stack(kind)?;
            let var = super::chain_builder::ChainBuilder::begin(Variable::Src)
                .get_field(crate::operands::DMString(name.as_str().as_bytes().to_vec()));
            compiler.cache = None;
            compiler.emit_ins(Instruction::SetVar(var));
            Ok(())
        };
        res = emit();
        vars.push((name.to_string(), std::mem::take(&mut compiler.nodes)));
        if res.is_err() {
            break;
        }
    }
    compiler.nodes = saved;
    compiler.cache = saved_cache;
    compiler.last_line = saved_line;
    compiler.line_pending = saved_pending;
    res?;
    Ok(Value::Prefab {
        path: prefab.path.to_string(),
        vars,
    })
}

fn value_line(expr: &Expression) -> u32 {
    match expr {
        Expression::Base { term, .. } => term.location.line,
        Expression::BinaryOp { lhs, .. } => value_line(lhs),
        Expression::AssignOp { lhs, .. } => value_line(lhs),
        Expression::TernaryOp { cond, .. } => value_line(cond),
    }
}
