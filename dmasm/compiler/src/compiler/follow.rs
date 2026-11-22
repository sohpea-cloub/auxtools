use dreammaker::ast::{Follow, ListAccessKind, PropertyAccessKind, UnaryOp};

use crate::{
    compiler::{
        args, chain_builder::ChainBuilder, operands, unary, CompileError, Compiler, EvalKind,
    },
    operands::{DMString, GlobalVar, Label, StaticScope, Variable},
    Instruction,
};

pub(super) fn emit(
    compiler: &mut Compiler,
    follow: Vec<Follow>,
    kind: EvalKind,
) -> Result<EvalKind, CompileError> {
    emit_typed(compiler, follow, kind, None)
}

fn callee_var<'a>(
    compiler: &Compiler<'a>,
    ty: Option<dreammaker::objtree::TypeRef<'a>>,
    access: PropertyAccessKind,
    name: &str,
) -> Variable {
    let ty = match ty {
        Some(t) if access == PropertyAccessKind::Colon => {
            Some(super::procs::colon_proc_type(t, name).unwrap_or(t))
        }
        Some(t) => Some(t),
        None => super::procs::first_declaring_type(compiler.proc.tree, name),
    };
    super::procs::call_var(ty, name)
}

fn proc_known(
    compiler: &Compiler,
    ty: Option<dreammaker::objtree::TypeRef>,
    access: PropertyAccessKind,
    name: &str,
) -> bool {
    match ty {
        Some(t) if access == PropertyAccessKind::Colon => {
            super::procs::colon_proc_type(t, name).is_some()
        }
        Some(t) => t.get_proc(name).is_some(),
        None => super::procs::first_declaring_type(compiler.proc.tree, name).is_some(),
    }
}

pub(super) fn emit_typed<'a>(
    compiler: &mut Compiler<'a>,
    follow: Vec<Follow>,
    kind: EvalKind,
    ty: Option<dreammaker::objtree::TypeRef<'a>>,
) -> Result<EvalKind, CompileError> {
    let mut kind = kind;
    let mut ty = ty;
    let mut elem_ty = compiler.list_elem_type.take();
    let safe_place = std::mem::take(&mut compiler.safe_place);

    let mut field_buffer = vec![];

    let mut follow = follow.into_iter();
    while let Some(sub_expr) = follow.next() {
        let more_fields = matches!(
            follow.as_slice().first(),
            Some(
                Follow::Field(PropertyAccessKind::Dot | PropertyAccessKind::Colon, _)
                    | Follow::Call(PropertyAccessKind::Dot | PropertyAccessKind::Colon, _, _)
            )
        );
        if is_safe(&sub_expr) && !safe_place {
            kind = commit_field_buffer(compiler, kind, &mut field_buffer)?;
            compiler.emit_move_to_stack(kind)?;
            return emit_safe(compiler, sub_expr, follow.collect(), ty);
        }
        let global_field = compiler.global_field;
        if matches!(
            sub_expr,
            Follow::Field(..) | Follow::Call(..) | Follow::StaticField(_)
        ) && !(matches!(kind, EvalKind::Global) && matches!(sub_expr, Follow::Field(..)))
        {
            compiler.global_field = true;
        }
        if !matches!(sub_expr, Follow::Field(..) | Follow::Index(..)) {
            elem_ty = None;
        }
        match sub_expr {
            Follow::Field(access_kind, ident) => {
                let field_ty = ty.and_then(|t| match access_kind {
                    PropertyAccessKind::Colon => super::procs::colon_var(t, ident.as_str()),
                    _ => super::procs::member_var(t, ident.as_str()),
                });
                ty =
                    field_ty.and_then(|d| super::procs::declared_type(compiler, &d.get().var_type));
                elem_ty = field_ty.and_then(|d| {
                    super::procs::list_elem_type(compiler, &d.get().var_type.type_path.to_string())
                });
                if matches!(kind, EvalKind::Global) && field_buffer.is_empty() && field_ty.is_none()
                {
                    ty = compiler
                        .proc
                        .tree
                        .root()
                        .get_var_declaration(ident.as_str())
                        .and_then(|d| super::procs::declared_type(compiler, &d.get().var_type));
                }
                if let Some(decl) = field_ty {
                    let is_static = super::procs::is_global_member(&decl)
                        && matches!(
                            access_kind,
                            PropertyAccessKind::Dot | PropertyAccessKind::Colon
                        );
                    let scope = decl.ty().get().path.clone();
                    if is_static && more_fields {
                        let mut name = super::chain_builder::STATIC_MARKER.to_vec();
                        name.extend_from_slice(ident.as_str().as_bytes());
                        name.push(0);
                        name.extend_from_slice(scope.as_bytes());
                        field_buffer.push(String::from_utf8(name).unwrap());
                        continue;
                    }
                    if is_static
                        && matches!(
                            kind,
                            EvalKind::Var(_) | EvalKind::Field(..) | EvalKind::Global
                        )
                    {
                        compiler.global_field = global_field;
                        field_buffer.clear();
                        kind = EvalKind::Var(Variable::Global(GlobalVar::named(
                            ident.as_str().as_bytes(),
                            Some(StaticScope::of_type(&scope)),
                        )));
                        continue;
                    }
                    if is_static && matches!(kind, EvalKind::Stack | EvalKind::ListRef) {
                        compiler.emit_move_to_stack(kind)?;
                        compiler.emit_ins(Instruction::SetVar(Variable::Cache));
                        kind = EvalKind::Var(Variable::Global(GlobalVar::named(
                            ident.as_str().as_bytes(),
                            Some(StaticScope::of_type(&scope)),
                        )));
                        continue;
                    }
                }
                match access_kind {
                    PropertyAccessKind::Dot
                    | PropertyAccessKind::Colon
                    | PropertyAccessKind::Scope => {
                        field_buffer.push(ident.into_owned());
                    }

                    PropertyAccessKind::SafeDot | PropertyAccessKind::SafeColon => {
                        kind = commit_field_buffer(compiler, kind, &mut field_buffer)?;
                        compiler.emit_move_to_stack(kind)?;

                        let short_circuit = compiler.short_circuit();
                        compiler
                            .emit_ins(Instruction::SetCacheJmpIfNull(Label::Named(short_circuit)));

                        kind = EvalKind::Field(
                            ChainBuilder::begin(Variable::Cache),
                            ident.into_owned(),
                        );
                    }
                }
            }

            Follow::Index(access_kind, expr) => {
                ty = elem_ty.take();
                kind = commit_field_buffer(compiler, kind, &mut field_buffer)?;

                match access_kind {
                    ListAccessKind::Normal => {
                        compiler.emit_move_to_stack(kind)?;

                        let expr = compiler.emit_expr(*expr)?;
                        compiler.emit_move_to_stack(expr)?;

                        kind = EvalKind::ListRef;
                    }

                    ListAccessKind::Safe => {
                        compiler.emit_move_to_stack(kind)?;

                        let short_circuit = compiler.short_circuit();
                        compiler
                            .emit_ins(Instruction::SetCacheJmpIfNull(Label::Named(short_circuit)));
                        compiler.emit_ins(Instruction::GetVar(Variable::Cache));

                        let expr = compiler.emit_expr(*expr)?;
                        compiler.emit_move_to_stack(expr)?;

                        kind = EvalKind::ListRef;
                    }
                }
            }

            Follow::Call(index_kind, ident, args) => {
                let receiver_ty = ty.take();
                let arg_count = args.len() as u32;

                match index_kind {
                    PropertyAccessKind::Dot
                    | PropertyAccessKind::Colon
                    | PropertyAccessKind::Scope
                        if matches!(kind, EvalKind::Global) && field_buffer.is_empty() =>
                    {
                        match args::emit(compiler, args::ArgsContext::Proc, args.into_vec())? {
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
                    }

                    access @ (PropertyAccessKind::Dot | PropertyAccessKind::Colon)
                        if compiler.debugger
                            && !proc_known(compiler, receiver_ty, access, &ident) =>
                    {
                        kind = commit_field_buffer(compiler, kind, &mut field_buffer)?;
                        compiler.emit_move_to_stack(kind)?;
                        compiler.emit_ins(Instruction::PushVal(crate::Value::DMString(DMString(
                            ident.as_bytes().to_vec(),
                        ))));
                        match args::emit(compiler, args::ArgsContext::Proc, args.into_vec())? {
                            args::ArgsResult::Normal => {
                                compiler.emit_ins(Instruction::CallName(arg_count))
                            }
                            args::ArgsResult::Assoc => {
                                compiler.emit_ins(Instruction::NewAssocList(arg_count));
                                compiler.emit_ins(Instruction::CallNameArgList);
                            }
                            args::ArgsResult::ArgList => {
                                compiler.emit_ins(Instruction::CallNameArgList)
                            }
                        }
                    }

                    access @ (PropertyAccessKind::Dot
                    | PropertyAccessKind::Colon
                    | PropertyAccessKind::Scope)
                        if !field_buffer.is_empty()
                            || matches!(kind, EvalKind::Var(_) | EvalKind::Field(..)) =>
                    {
                        let callee = callee_var(compiler, receiver_ty, access, &ident);
                        let parked = matches!(kind, EvalKind::Stack | EvalKind::ListRef);
                        kind = commit_field_buffer(compiler, kind, &mut field_buffer)?;
                        let builder = match kind {
                            EvalKind::Var(var) => ChainBuilder::begin(var),
                            EvalKind::Field(mut builder, field) => {
                                builder.append(DMString(field.into()));
                                builder
                            }
                            _ => unreachable!(),
                        };
                        if parked {
                            compiler.emit_ins(Instruction::PushCache);
                        }
                        let explicit_src = std::mem::take(&mut compiler.explicit_src);
                        let count = compiler.emit_call_args(args.into_vec())?;
                        compiler.explicit_src = explicit_src;
                        if parked {
                            compiler.emit_ins(Instruction::PopCache);
                        }
                        compiler.emit_ins(Instruction::Call(builder.get_leaf(callee), count));
                    }

                    access @ (PropertyAccessKind::Dot
                    | PropertyAccessKind::Colon
                    | PropertyAccessKind::Scope) => {
                        let callee = callee_var(compiler, receiver_ty, access, &ident);
                        kind = commit_field_buffer(compiler, kind, &mut field_buffer)?;
                        compiler.emit_move_to_stack(kind)?;

                        compiler.emit_ins(Instruction::SetVar(Variable::Cache));
                        compiler.emit_ins(Instruction::PushCache);

                        match args::emit(compiler, args::ArgsContext::Proc, args.into_vec())? {
                            args::ArgsResult::Normal => {
                                compiler.emit_ins(Instruction::PopCache);

                                compiler.emit_ins(Instruction::Call(callee.clone(), arg_count));
                            }

                            args::ArgsResult::Assoc => {
                                compiler.emit_ins(Instruction::NewAssocList(arg_count));

                                compiler.emit_ins(Instruction::PopCache);

                                compiler.emit_ins(Instruction::Call(callee.clone(), 65535));
                            }

                            args::ArgsResult::ArgList => {
                                compiler.emit_ins(Instruction::PopCache);

                                compiler.emit_ins(Instruction::Call(callee.clone(), 65535));
                            }
                        }
                    }

                    PropertyAccessKind::SafeDot | PropertyAccessKind::SafeColon => {
                        kind = commit_field_buffer(compiler, kind, &mut field_buffer)?;
                        compiler.emit_move_to_stack(kind)?;

                        let short_circuit = compiler.short_circuit();
                        compiler
                            .emit_ins(Instruction::SetCacheJmpIfNull(Label::Named(short_circuit)));

                        compiler.emit_ins(Instruction::PushCache);

                        match args::emit(compiler, args::ArgsContext::Proc, args.into_vec())? {
                            args::ArgsResult::Normal => {
                                compiler.emit_ins(Instruction::PopCache);

                                compiler.emit_ins(Instruction::Call(
                                    Variable::DynamicProc(super::dynamic_name(&ident)),
                                    arg_count,
                                ));
                            }

                            args::ArgsResult::Assoc => {
                                compiler.emit_ins(Instruction::NewAssocList(arg_count));

                                compiler.emit_ins(Instruction::PopCache);

                                compiler.emit_ins(Instruction::Call(
                                    Variable::DynamicProc(super::dynamic_name(&ident)),
                                    65535,
                                ));
                            }

                            args::ArgsResult::ArgList => {
                                compiler.emit_ins(Instruction::PopCache);

                                compiler.emit_ins(Instruction::Call(
                                    Variable::DynamicProc(super::dynamic_name(&ident)),
                                    65535,
                                ));
                            }
                        }
                    }
                }

                kind = EvalKind::Stack;
            }

            Follow::Unary(op) => {
                ty = None;
                kind = commit_field_buffer(compiler, kind, &mut field_buffer)?;
                kind = unary::emit(compiler, vec![op], kind)?;
            }

            Follow::StaticField(ident) => {
                let decl = ty.and_then(|t| super::procs::member_var(t, ident.as_str()));
                let static_scope = decl
                    .filter(super::procs::is_global_member)
                    .map(|d| d.ty().get().path.clone());
                let is_static = static_scope.is_some();
                let mut names = vec![ident.as_str()];
                for f in follow.as_slice() {
                    match f {
                        Follow::StaticField(name) => names.push(name.as_str()),
                        _ => break,
                    }
                }
                let leads_to_static = !is_static
                    && (2..=names.len()).any(|n| {
                        super::procs::static_chain(compiler, ty, &names[..n])
                            .is_some_and(|d| super::procs::is_global_member(&d))
                    });
                if leads_to_static {
                    ty =
                        decl.and_then(|d| super::procs::declared_type(compiler, &d.get().var_type));
                    continue;
                }
                kind = commit_field_buffer(compiler, kind, &mut field_buffer)?;
                if is_static {
                    if kind == EvalKind::Stack {
                        compiler.emit_ins(Instruction::Pop);
                    }
                    kind = EvalKind::Var(Variable::Global(GlobalVar::named(
                        ident.as_str().as_bytes(),
                        static_scope.as_deref().map(StaticScope::of_type),
                    )));
                } else {
                    compiler.emit_move_to_stack(kind)?;
                    compiler.emit_ins(Instruction::SetVar(Variable::Cache));
                    compiler.emit_ins(Instruction::GetVar(Variable::Initial(Box::new(
                        Variable::Field(DMString(ident.as_str().into())),
                    ))));
                    kind = EvalKind::Stack;
                }
                ty = decl.and_then(|d| super::procs::declared_type(compiler, &d.get().var_type));
            }

            Follow::ProcReference(_) => {
                return Err(CompileError::UnsupportedStatement(
                    "type::procname() must have a constant type path",
                ))
            }
        }
    }

    kind = commit_field_buffer(compiler, kind, &mut field_buffer)?;
    Ok(kind)
}

fn commit_field_buffer(
    compiler: &mut Compiler,
    kind: EvalKind,
    field_chain: &mut Vec<String>,
) -> Result<EvalKind, CompileError> {
    if field_chain.is_empty() {
        return Ok(kind);
    }

    let mut builder = match kind {
        EvalKind::Stack => {
            compiler.emit_ins(Instruction::SetVar(Variable::Cache));
            ChainBuilder::begin(Variable::Cache)
        }

        EvalKind::ListRef => {
            compiler.emit_ins(Instruction::ListGet);
            compiler.emit_ins(Instruction::SetVar(Variable::Cache));
            ChainBuilder::begin(Variable::Cache)
        }

        EvalKind::Range => return Err(CompileError::UnexpectedRange),
        EvalKind::ArgList => return Err(CompileError::UnexpectedArgList),

        EvalKind::Global => {
            let name = field_chain.remove(0);
            if let Some(scope) = compiler.expr_scope {
                if name == "vars" && (scope.global)("vars").is_none() {
                    if let Some(index) = compiler.params.iter().rposition(|x| *x == scope.globals) {
                        compiler.emit_ins(Instruction::GetVar(Variable::Arg(index as u32)));
                        return commit_field_buffer(compiler, EvalKind::Stack, field_chain);
                    }
                }
                let kind = compiler.emit_global_by_name(&name)?;
                return commit_field_buffer(compiler, kind, field_chain);
            }
            let var =
                Variable::Global(GlobalVar::named(name.as_bytes(), Some(StaticScope::Global)));
            return commit_field_buffer(compiler, EvalKind::Var(var), field_chain);
        }

        EvalKind::Field(mut builder, field) => {
            builder.append(DMString(field.into()));
            builder
        }

        EvalKind::Var(var) => ChainBuilder::begin(var),
    };

    let last_field = field_chain.pop().unwrap();

    for field in field_chain.iter() {
        builder.append(DMString(field.clone().into()));
    }

    field_chain.clear();
    Ok(EvalKind::Field(builder, last_field))
}

fn is_safe(follow: &Follow) -> bool {
    matches!(
        follow,
        Follow::Field(
            PropertyAccessKind::SafeDot | PropertyAccessKind::SafeColon,
            _
        ) | Follow::Call(
            PropertyAccessKind::SafeDot | PropertyAccessKind::SafeColon,
            _,
            _
        ) | Follow::Index(ListAccessKind::Safe, _)
    )
}

fn emit_safe<'a>(
    compiler: &mut Compiler<'a>,
    first: Follow,
    mut rest: Vec<Follow>,
    ty: Option<dreammaker::objtree::TypeRef<'a>>,
) -> Result<EvalKind, CompileError> {
    let mut unary = vec![];
    while let Some(Follow::Unary(_)) = rest.last() {
        unary.insert(0, rest.pop().unwrap());
    }
    if let [Follow::Unary(
        op @ (UnaryOp::PreIncr | UnaryOp::PostIncr | UnaryOp::PreDecr | UnaryOp::PostDecr),
    )] = unary.as_slice()
    {
        let op = *op;
        let mut follows = vec![first.clone()];
        follows.extend(rest.iter().cloned());
        let step = move |var: Variable| {
            vec![match op {
                UnaryOp::PreIncr => Instruction::PreInc(var),
                UnaryOp::PostIncr => Instruction::PostInc(var),
                UnaryOp::PreDecr => Instruction::PreDec(var),
                _ => Instruction::PostDec(var),
            }]
        };
        let write = super::statements::SafeWrite::Change(&step);
        if let Some(chain) = super::statements::safe_chain(&follows) {
            if chain.writable(&write, true) {
                let indexed = matches!(follows.last(), Some(Follow::Index(..)));
                let static_var = super::statements::static_member(compiler, ty, &follows, indexed);
                super::statements::emit_safe_chain_write(
                    compiler, chain, None, static_var, None, write, true,
                )?;
                return Ok(EvalKind::Stack);
            }
        }
    }
    let kind = emit_safe_inner(compiler, first, rest, ty)?;
    if unary.is_empty() {
        return Ok(kind);
    }
    emit(compiler, unary, kind)
}

fn emit_safe_inner<'a>(
    compiler: &mut Compiler<'a>,
    first: Follow,
    rest: Vec<Follow>,
    ty: Option<dreammaker::objtree::TypeRef<'a>>,
) -> Result<EvalKind, CompileError> {
    let end = compiler.new_label();
    compiler.emit_ins(Instruction::SetCacheJmpIfNull(Label::Named(end.clone())));

    let mut follows = vec![match first {
        Follow::Field(_, ident) => Follow::Field(PropertyAccessKind::Dot, ident),
        Follow::Call(_, ident, args) => Follow::Call(PropertyAccessKind::Dot, ident, args),
        Follow::Index(_, expr) => Follow::Index(ListAccessKind::Normal, expr),
        other => other,
    }];
    follows.extend(rest);

    let plain = follows.iter().all(|f| {
        matches!(
            f,
            Follow::Field(PropertyAccessKind::Dot | PropertyAccessKind::Colon, _)
        )
    });
    if plain {
        let kind = emit_typed(compiler, follows, EvalKind::Var(Variable::Cache), ty)?;
        compiler.emit_move_to_stack(kind)?;
        compiler.emit_label(end);
        return Ok(EvalKind::Stack);
    }

    compiler.emit_ins(Instruction::PushCache);
    let mut popped = false;
    let mut called = false;
    let mut kind = EvalKind::Var(Variable::Cache);
    let mut field_buffer: Vec<String> = vec![];
    let mut ty = ty;
    let mut it = follows.into_iter();
    while let Some(f) = it.next() {
        if is_safe(&f) {
            kind = commit_field_buffer(compiler, kind, &mut field_buffer)?;
            compiler.emit_move_to_stack(kind)?;
            kind = emit_safe(compiler, f, it.by_ref().collect(), ty)?;
            break;
        }
        let receiver_ty = ty;
        ty = None;
        match f {
            Follow::Field(_, ident) => {
                let decl = receiver_ty.and_then(|t| super::procs::member_var(t, ident.as_str()));
                ty = decl.and_then(|d| super::procs::declared_type(compiler, &d.get().var_type));
                match decl {
                    Some(decl) if super::procs::is_global_member(&decl) => {
                        field_buffer.clear();
                        let scope = decl.ty().get().path.clone();
                        kind = EvalKind::Var(Variable::Global(GlobalVar::named(
                            ident.as_str().as_bytes(),
                            Some(StaticScope::of_type(&scope)),
                        )));
                    }
                    _ => field_buffer.push(ident.into_owned()),
                }
            }
            Follow::Call(_, ident, args) if !called => {
                called = true;
                kind = commit_field_buffer(compiler, kind, &mut field_buffer)?;
                let builder = match kind {
                    EvalKind::Var(var) => ChainBuilder::begin(var),
                    EvalKind::Field(mut builder, field) => {
                        builder.append(DMString(field.into()));
                        builder
                    }
                    other => {
                        compiler.emit_move_to_stack(other)?;
                        compiler.emit_ins(Instruction::SetVar(Variable::Cache));
                        ChainBuilder::begin(Variable::Cache)
                    }
                };
                let start = compiler.nodes.len();
                let count = compiler.emit_call_args(args.into_vec())?;
                let touched = compiler.nodes[start..].iter().any(|n| {
                    matches!(n, crate::Node::Instruction(i, _) if super::procs::touches_cache(i))
                });
                let pop = touched || it.as_slice().is_empty();
                if touched && !it.as_slice().is_empty() {
                    compiler.warn_bug(
                        crate::diagnostic::Id::ByondSafeChainReceiver,
                        super::statements::SAFE_RECEIVER_BUG,
                        super::statements::SAFE_RECEIVER_OURS,
                    );
                }
                if pop {
                    compiler.emit_ins(Instruction::PopCache);
                    popped = true;
                }
                let var = super::procs::call_var(receiver_ty, &ident);
                compiler.emit_ins(Instruction::Call(builder.get_leaf(var), count));
                kind = EvalKind::Stack;
            }
            other => {
                kind = commit_field_buffer(compiler, kind, &mut field_buffer)?;
                kind = emit(compiler, vec![other], kind)?;
            }
        }
    }
    kind = commit_field_buffer(compiler, kind, &mut field_buffer)?;
    compiler.emit_move_to_stack(kind)?;
    if !popped {
        compiler.emit_ins(Instruction::PopCache);
    }
    compiler.emit_label(end);
    Ok(EvalKind::Stack)
}
