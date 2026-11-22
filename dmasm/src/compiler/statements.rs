use dreammaker::ast::{
    AssignOp, Block, Expression, Follow, Spanned, Statement, Term, UnaryOp, VarStatement,
};

use super::procs::{LoopLabels, ProcSource};
use super::{CompileError, Compiler, EvalKind};
use crate::operands::{Label, Value, Variable};
use crate::{ByondVersion, Instruction, Node};

pub(super) fn emit_proc_body(
    compiler: &mut Compiler,
    source: &ProcSource,
    code: &Block,
) -> Result<(), CompileError> {
    let first_statement = code.first().map(|stmt| stmt.location.line);
    let first_line = source
        .value
        .first_line
        .or(first_statement)
        .unwrap_or(source.value.location.line + 1);

    compiler.emit_ins(Instruction::DbgLine(first_line));
    super::known_bugs::check(compiler, source, code);
    let mut scan = LabelScan::default();
    label_iterators(code, 0, &mut scan);
    for (name, depth) in scan.bare {
        if scan.gotos.contains(&name) && !scan.labels.contains_key(&name) {
            scan.labels.insert(name.clone(), depth);
            ctx(compiler).bare_labels.insert(name);
        }
    }
    ctx(compiler).label_iterators = scan.labels;

    let params = source
        .value
        .parameters
        .iter()
        .filter(|p| !p.absolute && p.name.as_str() != "...");
    for (index, param) in params.enumerate() {
        let Some(default) = &param.default else {
            continue;
        };
        let arg = Variable::Arg(index as u32);
        let skip = compiler.new_label();
        compiler.emit_ins(Instruction::GetVar(arg.clone()));
        compiler.emit_ins(Instruction::IsNull);
        compiler.emit_ins(Instruction::Test);
        compiler.emit_ins(Instruction::Jz(Label::Named(skip.clone())));
        compiler.emit_ins(Instruction::DbgLine(param.location.line));
        if path_division(default) {
            compiler.warn_bug(
                &format!(
                    "parameter {}'s default divides a var by a number: written without spaces \
                     (`a.b/10`), DreamMaker reads it as a path and leaves the division out",
                    param.name
                ),
                "; dmasm divides",
            );
        }
        let default = match default {
            Expression::BinaryOp {
                op: dreammaker::ast::BinaryOp::In,
                lhs,
                ..
            } => &**lhs,
            other => other,
        };
        compiler.implicit_type = implicit_type_for(compiler, &param.var_type.type_path.to_string());
        let kind = compiler.emit_expr(default.clone());
        compiler.implicit_type = None;
        let kind = kind?;
        compiler.emit_move_to_stack(kind)?;
        compiler.emit_ins(Instruction::SetVar(arg));
        compiler.emit_label(skip);
    }

    let return_type = source
        .ty
        .get()
        .procs
        .get(source.name)
        .and_then(|p| p.declaration.as_ref())
        .is_some_and(|d| d.location == source.value.location && !d.return_type.is_empty());
    if return_type {
        compiler.emit_ins(Instruction::DbgLine(source.value.location.line));
    }

    let has_defaults = source.value.parameters.iter().any(|p| p.default.is_some());
    let skip_first_line = !has_defaults && !return_type && first_statement == Some(first_line);
    emit_block_inner(compiler, code, skip_first_line)
}

fn emit_block(compiler: &mut Compiler, block: &Block) -> Result<(), CompileError> {
    emit_block_inner(compiler, block, false)
}

#[derive(Default)]
struct LabelScan {
    labels: std::collections::HashMap<String, u32>,
    bare: std::collections::HashMap<String, u32>,
    gotos: std::collections::HashSet<String>,
}

fn label_iterators(block: &[Spanned<Statement>], depth: u32, out: &mut LabelScan) {
    for stmt in block.iter() {
        match &stmt.elem {
            Statement::Label { name, block } => {
                out.labels.insert(name.as_str().to_owned(), depth);
                label_iterators(block, depth, out);
            }
            Statement::ForList(f) => {
                let counts = !matches!(
                    &f.in_list,
                    Some(Expression::BinaryOp {
                        op: dreammaker::ast::BinaryOp::To,
                        ..
                    })
                );
                label_iterators(&f.block, depth + counts as u32, out);
            }
            Statement::ForKeyValue(f) => label_iterators(&f.block, depth + 1, out),
            Statement::ForRange(f) => label_iterators(&f.block, depth, out),
            Statement::ForLoop { block, .. }
            | Statement::ForInfinite { block }
            | Statement::While { block, .. }
            | Statement::DoWhile { block, .. } => label_iterators(block, depth, out),
            Statement::If { arms, else_arm } => {
                for (_, block) in arms.iter() {
                    label_iterators(block, depth, out);
                }
                if let Some(block) = else_arm {
                    label_iterators(block, depth, out);
                }
            }
            Statement::Switch { cases, default, .. } => {
                for (_, block) in cases.iter() {
                    label_iterators(block, depth, out);
                }
                if let Some(block) = default {
                    label_iterators(block, depth, out);
                }
            }
            Statement::TryCatch {
                try_block,
                catch_block,
                ..
            } => {
                label_iterators(try_block, depth, out);
                label_iterators(catch_block, depth, out);
            }
            Statement::Spawn { block, .. } => label_iterators(block, depth, out),
            Statement::Goto(name) => {
                out.gotos.insert(name.as_str().to_owned());
            }
            Statement::Expr(expr) => {
                if let Some(name) = bare_name(expr) {
                    out.bare.entry(name.to_owned()).or_insert(depth);
                }
            }
            _ => {}
        }
    }
}

pub(super) fn is_bare_label(block: &[Spanned<Statement>], expr: &Expression) -> bool {
    let Some(name) = bare_name(expr) else {
        return false;
    };
    let mut scan = LabelScan::default();
    label_iterators(block, 0, &mut scan);
    scan.gotos.contains(name) && !scan.labels.contains_key(name)
}

fn bare_name(expr: &Expression) -> Option<&str> {
    match expr {
        Expression::Base { term, follow } if follow.is_empty() => match &term.elem {
            Term::Ident(name) => Some(name.as_str()),
            _ => None,
        },
        _ => None,
    }
}

fn emit_block_inner(
    compiler: &mut Compiler,
    block: &Block,
    skip_first_line: bool,
) -> Result<(), CompileError> {
    compiler.proc.as_mut().unwrap().scopes.push(vec![]);
    let mut skip_next = false;
    for (i, stmt) in block.iter().enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }
        let line = stmt.location.line;
        compiler.read_to = None;
        let next_line = {
            let ctx = ctx(compiler);
            let found = ctx
                .next_lines
                .binary_search_by_key(&stmt.location, |(at, _)| *at);
            ctx.next_line = found.ok().map(|i| ctx.next_lines[i].1);
            ctx.next_line
        };
        match &stmt.elem {
            Statement::Expr(expr)
                if compiler.read_on_line(expr).is_some_and(|l| l > line)
                    && matches!(
                        expr,
                        Expression::BinaryOp { .. } | Expression::TernaryOp { .. }
                    ) => {}
            Statement::Return(Some(value)) => match after_leading_text(value, next_line) {
                Some(after) => {
                    compiler.emit_ins(Instruction::DbgLine(after));
                    compiler.emit_ins(Instruction::DbgLine(line));
                    compiler.skip_compound = true;
                }
                None if !(skip_first_line && i == 0) => {
                    compiler.emit_ins(Instruction::DbgLine(line))
                }
                None => {}
            },
            _ if !(skip_first_line && i == 0) => compiler.emit_ins(Instruction::DbgLine(line)),
            _ => {}
        }
        if let Statement::Spawn { delay, block: body } = &stmt.elem {
            if body.is_empty() && i + 1 < block.len() {
                let next = Statement::Spawn {
                    delay: delay.clone(),
                    block: block[i + 1..i + 2].to_vec().into(),
                };
                emit_statement(compiler, &next, stmt.location.line).map_err(|e| match e {
                    CompileError::AtLine(..) => e,
                    e => CompileError::AtLine(stmt.location.line, Box::new(e)),
                })?;
                skip_next = true;
                continue;
            }
        }
        emit_statement(compiler, &stmt.elem, stmt.location.line).map_err(|e| match e {
            CompileError::AtLine(..) => e,
            e => CompileError::AtLine(stmt.location.line, Box::new(e)),
        })?;
    }
    compiler.proc.as_mut().unwrap().scopes.pop();
    Ok(())
}

fn ctx<'c, 'a>(compiler: &'c mut Compiler<'a>) -> &'c mut super::procs::ProcCtx<'a> {
    compiler.proc.as_mut().unwrap()
}

fn emit_value(compiler: &mut Compiler, expr: &Expression) -> Result<(), CompileError> {
    let kind = compiler.emit_expr(expr.clone())?;
    compiler.emit_move_to_stack(kind)?;
    Ok(())
}

fn condition_end(compiler: &mut Compiler, condition: &Spanned<Expression>) {
    if let Some(end) = super::line_at(ctx(compiler).end_lines, condition.location) {
        compiler.read_on(end);
    }
}

fn emit_condition_jz(
    compiler: &mut Compiler,
    expr: &Expression,
    target: &str,
) -> Result<(), CompileError> {
    emit_condition(compiler, expr)?;
    compiler.emit_ins(Instruction::Jz(Label::Named(target.to_owned())));
    Ok(())
}

fn emit_condition(compiler: &mut Compiler, expr: &Expression) -> Result<(), CompileError> {
    compiler.condition_chain = matches!(
        expr,
        Expression::BinaryOp {
            op: dreammaker::ast::BinaryOp::And | dreammaker::ast::BinaryOp::Or,
            ..
        }
    );
    emit_value(compiler, expr)?;
    compiler.condition_chain = false;
    if let (false, Some(Node::Instruction(Instruction::GetFlag, _))) =
        (super::fold::is_logic(expr), compiler.nodes.last())
    {
        compiler.nodes.pop();
        return Ok(());
    }
    compiler.emit_ins(Instruction::Test);
    Ok(())
}

fn emit_statement(
    compiler: &mut Compiler,
    stmt: &Statement,
    line: u32,
) -> Result<(), CompileError> {
    if let Statement::Expr(expr) = stmt {
        if let Some(name) = bare_name(expr) {
            if ctx(compiler).bare_labels.remove(name) {
                ctx(compiler).clear_new_vars = true;
                compiler.emit_label(format!("USER_{}", name));
                return Ok(());
            }
        }
    }
    if matches!(
        stmt,
        Statement::ForLoop { .. }
            | Statement::ForList(_)
            | Statement::ForRange(_)
            | Statement::ForKeyValue(_)
            | Statement::Label { .. }
    ) {
        ctx(compiler).clear_new_vars = true;
    }
    let is_loop = matches!(
        stmt,
        Statement::ForLoop { .. }
            | Statement::ForInfinite { .. }
            | Statement::ForList(_)
            | Statement::ForRange(_)
            | Statement::ForKeyValue(_)
            | Statement::While { .. }
            | Statement::DoWhile { .. }
    );
    if is_loop {
        let res = emit_statement_inner(compiler, stmt, line);
        compiler.cache = None;
        return res;
    }
    emit_statement_inner(compiler, stmt, line)
}

fn emit_statement_inner(
    compiler: &mut Compiler,
    stmt: &Statement,
    line: u32,
) -> Result<(), CompileError> {
    if matches!(
        stmt,
        Statement::ForLoop { .. }
            | Statement::ForInfinite { .. }
            | Statement::ForList(_)
            | Statement::ForRange(_)
            | Statement::ForKeyValue(_)
    ) {
        compiler.cache = None;
    }
    match stmt {
        Statement::Expr(expr) => emit_expr_statement(compiler, expr),

        Statement::Return(value) => {
            let is_dot = |e: &Expression| {
                matches!(strip_parens(e), Expression::Base { term, follow }
                if follow.is_empty() && matches!(&term.elem, Term::Ident(i) if i.as_str() == "."))
            };
            match value {
                Some(expr) if is_dot(expr) => compiler.emit_ins(Instruction::End),
                Some(expr) => {
                    compiler.regroup_root = true;
                    emit_value(compiler, expr)?;
                    compiler.emit_ins(Instruction::Ret);
                }
                None => compiler.emit_ins(Instruction::End),
            }
            Ok(())
        }

        Statement::If { arms, else_arm }
            if arms
                .iter()
                .any(|(c, _)| super::fold::value(compiler, &c.elem).is_some()) =>
        {
            let mut live: Vec<(Spanned<Expression>, Block)> = vec![];
            let mut else_block = else_arm.clone();
            let mut dropped_after = false;
            for (cond, block) in arms.iter() {
                match super::fold::value(compiler, &cond.elem) {
                    Some(c) if !c.truthy() => dropped_after = !live.is_empty(),
                    Some(_) => {
                        else_block = Some(block.clone());
                        break;
                    }
                    None => {
                        live.push((cond.clone(), block.clone()));
                        dropped_after = false;
                    }
                }
            }
            if dropped_after && else_block.is_none() {
                else_block = Some(Default::default());
            }
            if live.is_empty() {
                compiler.cache = None;
                if let Some(block) = &else_block {
                    emit_block(compiler, block)?;
                    compiler.cache = None;
                }
                return Ok(());
            }
            emit_statement(
                compiler,
                &Statement::If {
                    arms: live,
                    else_arm: else_block,
                },
                line,
            )
        }

        Statement::If { arms, else_arm } => {
            let end = compiler.new_label();
            let count = arms.len();
            for (i, (condition, block)) in arms.iter().enumerate() {
                if i > 0 {
                    compiler.emit_ins(Instruction::DbgLine(condition.location.line));
                }
                let next = compiler.new_label();
                condition_end(compiler, condition);
                emit_condition_jz(compiler, &condition.elem, &next)?;
                emit_block(compiler, block)?;
                let last = i + 1 == count && else_arm.is_none();
                if !last {
                    compiler.emit_ins(Instruction::Jmp(Label::Named(end.clone())));
                }
                compiler.emit_label(next);
            }
            if let Some(block) = else_arm {
                emit_block(compiler, block)?;
            }
            compiler.emit_label(end);
            Ok(())
        }

        Statement::While { condition, block } => {
            if super::fold::value(compiler, &condition.elem).is_some_and(|c| !c.truthy()) {
                return Ok(());
            }
            let head = compiler.new_label();
            let end = compiler.new_label();
            compiler.emit_loop_label(head.clone());
            if super::fold::value(compiler, &condition.elem).map_or(true, |c| !c.truthy()) {
                condition_end(compiler, condition);
                emit_condition_jz(compiler, &condition.elem, &end)?;
            }
            emit_loop_body(compiler, block, &head, &end)?;
            compiler.emit_ins(Instruction::JmpLoop(Label::Named(head)));
            compiler.emit_label(end);
            Ok(())
        }

        Statement::DoWhile { block, condition } => {
            let head = compiler.new_label();
            let cont = compiler.new_label();
            let end = compiler.new_label();
            compiler.emit_loop_label(head.clone());
            emit_loop_body(compiler, block, &cont, &end)?;
            compiler.emit_label(cont);
            match super::fold::value(compiler, &condition.elem).map(|c| c.truthy()) {
                Some(false) => {}
                Some(_) => compiler.emit_ins(Instruction::JmpLoop(Label::Named(head))),
                None => {
                    compiler.emit_ins(Instruction::DbgLine(condition.location.line));
                    emit_condition(compiler, &condition.elem)?;
                    compiler.emit_ins(Instruction::JnzLoop(Label::Named(head)));
                }
            }
            compiler.emit_label(end);
            Ok(())
        }

        Statement::ForLoop {
            init,
            test,
            inc,
            block,
        } => {
            ctx(compiler).scopes.push(vec![]);
            if let Some(init) = init {
                emit_for_init(compiler, init, line)?;
            }
            let head = compiler.new_label();
            let cont = compiler.new_label();
            let end = compiler.new_label();
            compiler.emit_loop_label(head.clone());
            if let Some(test) = test {
                emit_condition_jz(compiler, test, &end)?;
            }
            emit_loop_body(compiler, block, &cont, &end)?;
            compiler.emit_label(cont);
            if let Some(inc) = inc {
                compiler.emit_ins(Instruction::DbgLine(line));
                emit_statement(compiler, inc, line)?;
            }
            compiler.emit_ins(Instruction::JmpLoop(Label::Named(head)));
            compiler.emit_label(end);
            ctx(compiler).scopes.pop();
            Ok(())
        }

        Statement::ForList(for_list) => {
            if let Some(Expression::BinaryOp {
                op: dreammaker::ast::BinaryOp::To,
                lhs,
                rhs,
            }) = &for_list.in_list
            {
                let range = dreammaker::ast::ForRangeStatement {
                    var_type: for_list.var_type.clone(),
                    name: for_list.name.clone(),
                    start: (**lhs).clone(),
                    end: (**rhs).clone(),
                    step: None,
                    block: for_list.block.clone(),
                };
                return emit_statement(compiler, &Statement::ForRange(Box::new(range)), line);
            }
            ctx(compiler).scopes.push(vec![]);
            let (var, declared) = if let Some(var_type) = &for_list.var_type {
                let ty = var_type.type_path.to_string();
                let slot = ctx(compiler).declare_typed_local(for_list.name.as_str(), ty.clone());
                compiler.emit_ins(Instruction::PushVal(Value::Null));
                compiler.emit_ins(Instruction::SetVar(Variable::Local(slot)));
                (EvalKind::Var(Variable::Local(slot)), ty)
            } else {
                let name_expr = Expression::from(Term::Ident(for_list.name.clone()));
                let ty = super::procs::declared_type_path(compiler, &name_expr).unwrap_or_default();
                (compiler.emit_find_var(for_list.name.clone())?, ty)
            };
            let var = lvalue_var(compiler, var)?;
            compiler.cache = None;
            let kind = match &for_list.in_list {
                Some(list) => match iterator_kind(list) {
                    Some((kind, args)) => {
                        for arg in args.iter() {
                            emit_value(compiler, arg)?;
                        }
                        let wanted = if kind == 19 { 6 } else { 2 };
                        for _ in args.len().min(wanted)..wanted {
                            compiler.emit_ins(Instruction::PushVal(Value::Null));
                        }
                        kind
                    }
                    None => {
                        emit_value(compiler, list)?;
                        5
                    }
                },
                None => {
                    compiler.emit_ins(Instruction::GetVar(Variable::World));
                    5
                }
            };
            let nested = ctx(compiler).iterators > 0;
            let input = for_list.input_type.filter(|i| !i.is_empty()).or_else(|| {
                for_list
                    .var_type
                    .as_ref()
                    .map(|v| v.input_type)
                    .filter(|i| !i.is_empty())
            });
            let (mut filter, mut check) = match input {
                Some(input) => (crate::TypeFilter::from_bits_truncate(input.bits()), None),
                _ => loop_filter(compiler, &declared),
            };
            if for_list.in_list.is_none() && filter.is_empty() && input.is_none() {
                if let Some(path) = check.take() {
                    compiler.emit_ins(Instruction::PushVal(Value::Path(path)));
                    filter = crate::TypeFilter::DATUM_INSTANCES;
                }
            }
            if nested {
                compiler.emit_ins(Instruction::IterPush);
            }
            compiler.emit_ins(Instruction::IterLoad(kind, filter));
            let head = compiler.new_label();
            let end = compiler.new_label();
            compiler.emit_loop_label(head.clone());
            compiler.emit_ins(Instruction::IterNext);
            compiler.emit_ins(Instruction::SetVar(var.clone()));
            compiler.emit_ins(Instruction::Jz(Label::Named(end.clone())));
            if let Some(path) = check {
                compiler.emit_ins(Instruction::GetVar(var));
                compiler.emit_ins(Instruction::PushVal(Value::Path(path)));
                compiler.emit_ins(Instruction::IsType);
                compiler.emit_ins(Instruction::Test);
                compiler.emit_ins(Instruction::JzLoop(Label::Named(head.clone())));
            }
            ctx(compiler).iterators += 1;
            if nested {
                ctx(compiler).cleanups.push(Instruction::IterPop);
            }
            emit_loop_body(compiler, &for_list.block, &head, &end)?;
            ctx(compiler).iterators -= 1;
            if nested {
                ctx(compiler).cleanups.pop();
            }
            compiler.emit_ins(Instruction::JmpLoop(Label::Named(head)));
            compiler.emit_label(end);
            if nested {
                compiler.emit_ins(Instruction::IterPop);
            }
            ctx(compiler).scopes.pop();
            Ok(())
        }

        Statement::ForRange(for_range) => {
            ctx(compiler).scopes.push(vec![]);
            let var = if let Some(var_type) = &for_range.var_type {
                let ty = var_type.type_path.to_string();
                let slot = ctx(compiler).declare_typed_local(for_range.name.as_str(), ty);
                compiler.emit_ins(Instruction::PushVal(Value::Null));
                compiler.emit_ins(Instruction::SetVar(Variable::Local(slot)));
                EvalKind::Var(Variable::Local(slot))
            } else {
                compiler.emit_find_var(for_range.name.clone())?
            };
            let var = lvalue_var(compiler, var)?;
            emit_value(compiler, &for_range.start)?;
            emit_value(compiler, &for_range.end)?;
            let head = compiler.new_label();
            let end = compiler.new_label();
            match &for_range.step {
                Some(step) => {
                    emit_value(compiler, step)?;
                    compiler.emit_ins(Instruction::Check3Numbers);
                    compiler.emit_loop_label(head.clone());
                    compiler.emit_ins(Instruction::ForRangeStep(Label::Named(end.clone()), var));
                }
                None => {
                    compiler.emit_ins(Instruction::Check2Numbers);
                    compiler.emit_loop_label(head.clone());
                    compiler.emit_ins(Instruction::ForRange(Label::Named(end.clone()), var));
                }
            }
            let n = if for_range.step.is_some() { 3 } else { 2 };
            ctx(compiler).cleanups.push(Instruction::PopN(n));
            let res = emit_loop_body(compiler, &for_range.block, &head, &end);
            ctx(compiler).cleanups.pop();
            res?;
            compiler.emit_ins(Instruction::JmpLoop(Label::Named(head)));
            compiler.emit_label(end);
            compiler.emit_ins(Instruction::PopN(n));
            ctx(compiler).scopes.pop();
            Ok(())
        }

        Statement::Var(var) => {
            let next_line = ctx(compiler).next_line;
            text_read_ahead(compiler, var, next_line);
            emit_var_statement_then(compiler, var, false)
        }
        Statement::Vars(vars) => {
            let next_line = ctx(compiler).next_line;
            for (i, var) in vars.iter().enumerate() {
                let next = if i + 1 == vars.len() {
                    next_line
                } else {
                    Some(line)
                };
                text_read_ahead(compiler, var, next);
                emit_var_statement_then(compiler, var, false)?;
            }
            Ok(())
        }

        Statement::Switch {
            input,
            cases,
            default,
        } => {
            emit_value(compiler, input)?;
            let end = compiler.new_label();
            let default_label = if default.is_some() {
                compiler.new_label()
            } else {
                end.clone()
            };
            let mut exact = vec![];
            let mut ranges = vec![];
            let mut arm_labels = vec![];
            for (values, _) in cases.iter() {
                let label = compiler.new_label();
                for case in values.elem.iter().rev() {
                    match case {
                        dreammaker::ast::Case::Exact(e) => {
                            exact.push((constant_value(compiler, e)?, Label::Named(label.clone())))
                        }
                        dreammaker::ast::Case::Range(a, b) => ranges.push((
                            constant_value(compiler, a)?,
                            constant_value(compiler, b)?,
                            Label::Named(label.clone()),
                        )),
                    }
                }
                arm_labels.push(label);
            }
            if ranges.is_empty() {
                compiler.emit_ins(Instruction::Switch(crate::SwitchParams {
                    default: Label::Named(default_label.clone()),
                    cases: exact,
                }));
            } else {
                compiler.emit_ins(Instruction::SwitchRange(crate::SwitchRangeParams {
                    default: Label::Named(default_label.clone()),
                    cases: exact,
                    range_cases: ranges,
                }));
            }
            let count = cases.len();
            for (i, ((_, block), label)) in cases.iter().zip(arm_labels).enumerate() {
                compiler.emit_label(label);
                emit_block(compiler, block)?;
                if i + 1 < count || default.is_some() {
                    compiler.emit_ins(Instruction::Jmp(Label::Named(end.clone())));
                }
            }
            if let Some(block) = default {
                compiler.emit_label(default_label);
                emit_block(compiler, block)?;
            }
            compiler.emit_label(end);
            Ok(())
        }

        Statement::Setting { .. } => Ok(()),

        Statement::Break(label) | Statement::Continue(label) => {
            let target = find_loop(compiler, label.as_ref().map(|l| l.as_str()))?;
            let live = if matches!(stmt, Statement::Break(_)) {
                target.break_cleanups
            } else {
                target.cleanups
            };
            let cleanups: Vec<Instruction> = ctx(compiler).cleanups[live..]
                .iter()
                .rev()
                .cloned()
                .collect();
            for ins in cleanups {
                compiler.emit_ins(ins);
            }
            let in_try = ctx(compiler).try_depth > 0;
            compiler.emit_ins(match (matches!(stmt, Statement::Break(_)), in_try) {
                (true, false) => Instruction::Jmp(Label::Named(target.break_label)),
                (true, true) => Instruction::Catch(Label::Named(target.break_label)),
                (false, false) => Instruction::JmpLoop(Label::Named(target.continue_label)),
                (false, true) => Instruction::TryJmp(Label::Named(target.continue_label)),
            });
            Ok(())
        }

        Statement::Spawn { delay, block } => {
            match delay {
                Some(delay) => emit_value(compiler, delay)?,
                None => compiler.emit_ins(Instruction::PushInt(0)),
            }
            let after = compiler.new_label();
            compiler.emit_ins(Instruction::Spawn(Label::Named(after.clone())));
            let res = emit_block(compiler, block);
            res?;
            compiler.emit_ins(Instruction::End);
            compiler.emit_label(after);
            Ok(())
        }

        Statement::Throw(expr) => {
            emit_value(compiler, expr)?;
            compiler.emit_ins(Instruction::Throw);
            Ok(())
        }

        Statement::TryCatch {
            try_block,
            catch_params,
            catch_block,
        } => {
            let catch = compiler.new_label();
            let end = compiler.new_label();
            compiler.emit_ins(Instruction::Try(Label::Named(catch.clone())));
            ctx(compiler).try_depth += 1;
            let res = emit_block(compiler, try_block);
            ctx(compiler).try_depth -= 1;
            res?;
            compiler.emit_ins(Instruction::Catch(Label::Named(end.clone())));
            compiler.emit_label(catch);
            ctx(compiler).scopes.push(vec![]);
            match catch_params.first() {
                Some(param) => {
                    let path = param.to_string();
                    let mut parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
                    let name = parts.pop().unwrap_or("e").to_owned();
                    if parts.first() == Some(&"var") {
                        parts.remove(0);
                    }
                    let ty = if parts.is_empty() {
                        String::new()
                    } else {
                        format!("/{}", parts.join("/"))
                    };
                    let slot = ctx(compiler).declare_typed_local(&name, ty);
                    if !ctx(compiler).loops.is_empty() || ctx(compiler).clear_new_vars {
                        compiler.emit_ins(Instruction::PushVal(Value::Null));
                        compiler.emit_ins(Instruction::SetVar(Variable::Local(slot)));
                    }
                    compiler.emit_ins(Instruction::SetVar(Variable::Local(slot)));
                }
                None => compiler.emit_ins(Instruction::Pop),
            }
            let res = emit_block(compiler, catch_block);
            ctx(compiler).scopes.pop();
            res?;
            compiler.emit_label(end);
            Ok(())
        }

        Statement::Label { name, block } => {
            compiler.emit_label(format!("USER_{}", name));
            let is_loop = block.iter().any(|s| {
                matches!(
                    s.elem,
                    Statement::While { .. }
                        | Statement::DoWhile { .. }
                        | Statement::ForInfinite { .. }
                        | Statement::ForLoop { .. }
                        | Statement::ForList(_)
                        | Statement::ForKeyValue(_)
                        | Statement::ForRange(_)
                )
            });
            if is_loop {
                let end = compiler.new_label();
                let cleanups = ctx(compiler).cleanups.len();
                ctx(compiler).pending_loop_name =
                    Some((name.as_str().to_owned(), end.clone(), cleanups));
                let res = emit_block(compiler, block);
                ctx(compiler).pending_loop_name = None;
                res?;
                compiler.emit_label(end);
                return Ok(());
            }
            let end = compiler.new_label();
            let cleanups = ctx(compiler).cleanups.len();
            ctx(compiler).loops.push(LoopLabels {
                name: Some(name.as_str().to_owned()),
                cleanups,
                break_cleanups: cleanups,
                continue_label: end.clone(),
                break_label: end.clone(),
                block: true,
            });
            let res = emit_block(compiler, block);
            ctx(compiler).loops.pop();
            res?;
            compiler.emit_label(end);
            Ok(())
        }

        Statement::Goto(name) => {
            let depth = ctx(compiler)
                .label_iterators
                .get(name.as_str())
                .copied()
                .unwrap_or(0);
            for _ in depth.max(1)..ctx(compiler).iterators {
                compiler.emit_ins(Instruction::IterPop);
            }
            let label = Label::Named(format!("USER_{}", name));
            let in_try = ctx(compiler).try_depth > 0;
            compiler.emit_ins(if in_try {
                Instruction::TryJmp(label)
            } else {
                Instruction::JmpLoop(label)
            });
            Ok(())
        }

        Statement::ForInfinite { block } => {
            let head = compiler.new_label();
            let end = compiler.new_label();
            compiler.emit_loop_label(head.clone());
            emit_loop_body(compiler, block, &head, &end)?;
            compiler.emit_ins(Instruction::JmpLoop(Label::Named(head)));
            compiler.emit_label(end);
            Ok(())
        }

        Statement::ForKeyValue(kv) => {
            super::versions::require(compiler.byond, ByondVersion::is_516, "for (k, v in list)")?;
            ctx(compiler).scopes.push(vec![]);
            let (key, value) = if let Some(var_type) = &kv.var_type {
                let k = ctx(compiler)
                    .declare_typed_local(kv.key.as_str(), var_type.type_path.to_string());
                compiler.emit_ins(Instruction::PushVal(Value::Null));
                compiler.emit_ins(Instruction::SetVar(Variable::Local(k)));
                let body_line = kv
                    .block
                    .first()
                    .map(|s| s.location.line)
                    .or(ctx(compiler).next_line);
                if let Some(body_line) = body_line {
                    compiler.emit_ins(Instruction::DbgLine(body_line));
                }
                let v = ctx(compiler).declare_typed_local(kv.value.as_str(), String::new());
                compiler.emit_ins(Instruction::PushVal(Value::Null));
                compiler.emit_ins(Instruction::SetVar(Variable::Local(v)));
                compiler.emit_ins(Instruction::DbgLine(line));
                (Variable::Local(k), Variable::Local(v))
            } else {
                let k = compiler.emit_find_var(kv.key.clone())?;
                let k = lvalue_var(compiler, k)?;
                let v = compiler.emit_find_var(kv.value.clone())?;
                let v = lvalue_var(compiler, v)?;
                (k, v)
            };
            compiler.cache = None;
            match &kv.in_list {
                Some(list) => emit_value(compiler, list)?,
                None => compiler.emit_ins(Instruction::GetVar(Variable::World)),
            }
            let nested = ctx(compiler).iterators > 0;
            if nested {
                compiler.emit_ins(Instruction::IterPush);
            }
            let input = kv.key_input_type.filter(|i| !i.is_empty()).or_else(|| {
                kv.var_type
                    .as_ref()
                    .map(|v| v.input_type)
                    .filter(|i| !i.is_empty())
            });
            let declared = kv
                .var_type
                .as_ref()
                .map(|v| v.type_path.to_string())
                .unwrap_or_default();
            let (filter, check) = match input {
                Some(input) => (crate::TypeFilter::from_bits_truncate(input.bits()), None),
                _ => loop_filter(compiler, &declared),
            };
            compiler.emit_ins(Instruction::IterLoad(20, filter));
            let head = compiler.new_label();
            let end = compiler.new_label();
            compiler.emit_loop_label(head.clone());
            compiler.emit_ins(Instruction::IterNext);
            compiler.emit_ins(Instruction::KeyValueIter(value));
            compiler.emit_ins(Instruction::SetVar(key.clone()));
            compiler.emit_ins(Instruction::Jz(Label::Named(end.clone())));
            if let Some(path) = check {
                compiler.emit_ins(Instruction::GetVar(key));
                compiler.emit_ins(Instruction::PushVal(Value::Path(path)));
                compiler.emit_ins(Instruction::IsType);
                compiler.emit_ins(Instruction::Test);
                compiler.emit_ins(Instruction::JzLoop(Label::Named(head.clone())));
            }
            ctx(compiler).iterators += 1;
            if nested {
                ctx(compiler).cleanups.push(Instruction::IterPop);
            }
            emit_loop_body(compiler, &kv.block, &head, &end)?;
            ctx(compiler).iterators -= 1;
            if nested {
                ctx(compiler).cleanups.pop();
            }
            compiler.emit_ins(Instruction::JmpLoop(Label::Named(head)));
            compiler.emit_label(end);
            if nested {
                compiler.emit_ins(Instruction::IterPop);
            }
            ctx(compiler).scopes.pop();
            Ok(())
        }

        Statement::Del(expr) => {
            emit_value(compiler, expr)?;
            let target = strip_parens(expr);
            if is_deletable_place(target) {
                compiler.emit_ins(Instruction::PushVal(Value::Null));
                let kind = compiler.emit_inner_expr(target.clone())?;
                if kind == EvalKind::ListRef {
                    compiler.emit_ins(Instruction::ListSet);
                } else {
                    let var = lvalue_var(compiler, kind)?;
                    compiler.emit_ins(Instruction::SetVar(var));
                }
            }
            compiler.emit_ins(Instruction::Del);
            if is_src(target) {
                compiler.emit_ins(Instruction::End);
            }
            Ok(())
        }

        Statement::Crash(expr) => {
            match expr {
                Some(expr) => emit_value(compiler, expr)?,
                None => compiler.emit_ins(Instruction::PushVal(Value::Null)),
            }
            compiler.emit_ins(Instruction::Crash);
            Ok(())
        }
    }
}

fn constant_value(compiler: &mut Compiler, expr: &Expression) -> Result<Value, CompileError> {
    match super::fold::value(compiler, expr) {
        Some(super::fold::Constant::Number(n)) => return Ok(Value::Number(n + 0.0)),
        Some(super::fold::Constant::Text(t)) => {
            return Ok(Value::DMString(crate::operands::DMString(t)))
        }
        Some(super::fold::Constant::Null) => return Ok(Value::Null),
        Some(super::fold::Constant::Path(p)) => return Ok(Value::Path(p)),
        Some(super::fold::Constant::New) | None => {}
    }
    if let Expression::Base { term, follow } = strip_parens(expr) {
        if follow.is_empty() {
            match &term.elem {
                Term::Null => return Ok(Value::Null),
                Term::String(s) => return Ok(Value::DMString(super::strings::parse(s)?)),
                Term::Prefab(prefab) if prefab.vars.is_empty() => {
                    return Ok(Value::Path(prefab.path.to_string()))
                }
                Term::Resource(r) => return Ok(Value::Resource(r.to_string())),
                _ => {}
            }
        }
    }
    let _ = compiler;
    Err(CompileError::UnsupportedStatement(
        "switch case that isn't a simple constant",
    ))
}

fn emit_for_init(compiler: &mut Compiler, init: &Statement, line: u32) -> Result<(), CompileError> {
    match init {
        Statement::Expr(Expression::Base { term, follow })
            if follow.is_empty() && matches!(term.elem, Term::Ident(_)) =>
        {
            Ok(())
        }
        Statement::Var(var) => emit_var_statement_then(compiler, var, true),
        Statement::Vars(vars) => {
            for var in vars {
                emit_var_statement_then(compiler, var, true)?;
            }
            Ok(())
        }
        other => emit_statement(compiler, other, line),
    }
}

fn text_read_ahead(compiler: &mut Compiler, var: &VarStatement, next_line: Option<u32>) {
    if var.var_type.flags.is_static() || var.var_type.flags.is_const() {
        return;
    }
    if let Some(after) = var
        .value
        .as_ref()
        .and_then(|v| after_leading_text(v, next_line))
    {
        compiler.emit_ins(Instruction::DbgLine(after));
        compiler.read_to = Some(after);
        compiler.line_pending = true;
    }
}

fn after_leading_text(expr: &Expression, next_line: Option<u32>) -> Option<u32> {
    match expr {
        Expression::Base { term, follow } => match (&term.elem, follow.first()) {
            (Term::InterpString(..), Some(f)) => Some(f.location.line),
            (Term::InterpString(..), None) => next_line,
            _ => None,
        },
        Expression::BinaryOp { lhs, rhs, .. } => {
            after_leading_text(lhs, Some(super::first_line(rhs)))
        }
        Expression::TernaryOp { cond, if_, .. } => {
            after_leading_text(cond, Some(super::first_line(if_)))
        }
        Expression::AssignOp { .. } => None,
    }
}

fn emit_var_statement_then(
    compiler: &mut Compiler,
    var: &VarStatement,
    null_first: bool,
) -> Result<(), CompileError> {
    emit_var_statement(compiler, var, null_first)?;
    if matches!(&var.value, Some(Expression::Base { term, follow })
        if follow.is_empty() && matches!(term.elem, Term::NewImplicit { .. }))
        && implicit_type_for(compiler, &var.var_type.type_path.to_string()).as_deref()
            == Some("/list")
    {
        compiler.end_assignment_statement();
    }
    Ok(())
}

fn emit_var_statement(
    compiler: &mut Compiler,
    var: &VarStatement,
    null_first: bool,
) -> Result<(), CompileError> {
    let in_clause;
    let var = match &var.value {
        Some(Expression::BinaryOp {
            op: dreammaker::ast::BinaryOp::In,
            lhs,
            ..
        }) => {
            let mut v = var.clone();
            v.value = Some((**lhs).clone());
            in_clause = v;
            &in_clause
        }
        _ => var,
    };
    if var.var_type.flags.is_const() {
        use dreammaker::constants::Constant as C;
        let value = match var
            .value
            .as_ref()
            .and_then(|v| super::fold::value(compiler, v))
        {
            Some(super::fold::Constant::Number(n)) => Some(C::Float(n)),
            Some(super::fold::Constant::Text(t))
                if !t.iter().any(|b| matches!(b, 0xFF | b'\\' | b'[')) =>
            {
                String::from_utf8(t).ok().map(|t| C::String(t.into()))
            }
            _ => None,
        };
        if let Some(value) = value {
            ctx(compiler)
                .local_consts
                .insert(var.name.as_str().to_owned(), value);
        }
    }
    if var.var_type.flags.is_static() || var.var_type.flags.is_const() {
        let ty = var.var_type.type_path.to_string();
        let value = match &var.value {
            None => Some(dreammaker::constants::Constant::Null(None)),
            Some(value) => {
                let nowhere = dreammaker::Location {
                    file: dreammaker::FileId::UNKNOWN,
                    line: 0,
                    column: 0,
                };
                value.simple_evaluate(nowhere).ok()
            }
        };
        ctx(compiler).declare_static(var.name.as_str(), ty, value, var.value.clone());
        return Ok(());
    }
    let ty = var.var_type.type_path.to_string();
    compiler.implicit_type = implicit_type_for(compiler, &ty);
    let slot_var = |compiler: &mut Compiler| {
        let slot = ctx(compiler).declare_typed_local(var.name.as_str(), ty.clone());
        Variable::Local(slot)
    };
    if null_first {
        let local = slot_var(compiler);
        compiler.emit_ins(Instruction::PushVal(Value::Null));
        compiler.emit_ins(Instruction::SetVar(local.clone()));
        if let Some(value) = &var.value {
            emit_value(compiler, value)?;
            compiler.emit_ins(Instruction::SetVar(local));
        }
        return Ok(());
    }
    let sized_list = matches!(&var.value, Some(Expression::Base { term, .. })
        if term.location.file == dreammaker::FileId::UNKNOWN && matches!(term.elem, Term::NewPrefab { .. }));
    match &var.value {
        Some(value)
            if sized_list && (!ctx(compiler).loops.is_empty() || ctx(compiler).clear_new_vars) =>
        {
            compiler.implicit_type = None;
            let local = slot_var(compiler);
            compiler.emit_ins(Instruction::PushVal(Value::Null));
            compiler.emit_ins(Instruction::SetVar(local.clone()));
            emit_value(compiler, value)?;
            compiler.emit_ins(Instruction::SetVar(local));
        }
        Some(value) => {
            emit_value(compiler, value)?;
            compiler.implicit_type = None;
            let local = slot_var(compiler);
            compiler.emit_ins(Instruction::SetVar(local));
        }
        None => {
            compiler.implicit_type = None;
            let local = slot_var(compiler);
            if !ctx(compiler).loops.is_empty() || ctx(compiler).clear_new_vars {
                compiler.emit_ins(Instruction::PushVal(Value::Null));
                compiler.emit_ins(Instruction::SetVar(local));
            }
        }
    }
    Ok(())
}

fn iterator_kind(list: &Expression) -> Option<(u32, &[Expression])> {
    let Expression::Base { term, follow } = strip_parens(list) else {
        return None;
    };
    if !follow.is_empty() {
        return None;
    }
    let Term::Call(name, args) = &term.elem else {
        return None;
    };
    let kind = match name.as_str() {
        "block" if args.len() == 2 => 6,
        "block" if (3..=6).contains(&args.len()) => 19,
        "view" => 7,
        "oview" => 8,
        "range" => 13,
        "orange" => 14,
        "viewers" => 15,
        "oviewers" => 16,
        "hearers" => 17,
        "ohearers" => 18,
        _ => return None,
    };
    if args.len() > 2 && kind != 19 {
        return None;
    }
    Some((kind, args))
}

fn loop_filter(compiler: &Compiler, ty: &str) -> (crate::TypeFilter, Option<String>) {
    use crate::TypeFilter as F;
    if ty.is_empty() {
        return (F::empty(), None);
    }
    let ty = match ty.strip_prefix("/list") {
        Some(rest) if rest.starts_with('/') => {
            if compiler
                .proc
                .as_ref()
                .and_then(|c| c.tree.find(rest))
                .is_some()
            {
                rest
            } else {
                "/list"
            }
        }
        _ => ty,
    };
    let path = compiler
        .proc
        .as_ref()
        .and_then(|c| c.tree.find(ty))
        .map(|t| t.get().path.clone())
        .unwrap_or_else(|| ty.to_owned());
    let under = |root: &str| path == root || path.starts_with(&format!("{}/", root));
    let filter = if under("/obj") {
        F::OBJ
    } else if under("/mob") {
        F::MOB
    } else if under("/turf") {
        F::TURF
    } else if under("/area") {
        F::AREA
    } else if under("/atom/movable") {
        F::MOB | F::OBJ
    } else if under("/atom") {
        F::MOB | F::OBJ | F::TURF | F::AREA
    } else {
        F::empty()
    };
    let exact = matches!(path.as_str(), "/mob" | "/turf" | "/area");
    (filter, if exact { None } else { Some(path) })
}

fn implicit_type_for(compiler: &Compiler, ty: &str) -> Option<String> {
    if ty.is_empty() {
        return None;
    }
    if is_list_path(ty) {
        return Some("/list".to_owned());
    }
    let found = compiler
        .proc
        .as_ref()?
        .tree
        .find(ty)
        .map(|t| t.get().path.clone());
    match found {
        None if compiler.debugger => Some(ty.to_owned()),
        found => found,
    }
}

fn is_list_path(ty: &str) -> bool {
    ty.split('/').any(|part| part == "list")
}

fn list_type_of(compiler: &Compiler, expr: &Expression) -> Option<String> {
    super::procs::declared_type_path(compiler, expr)
        .filter(|p| is_list_path(p))
        .map(|_| "/list".to_owned())
}

fn find_loop(compiler: &mut Compiler, name: Option<&str>) -> Result<LoopLabels, CompileError> {
    let loops = &ctx(compiler).loops;
    let found = match name {
        None => loops.iter().rev().find(|l| !l.block),
        Some(name) => loops.iter().rev().find(|l| l.name.as_deref() == Some(name)),
    };
    found.cloned().ok_or(CompileError::UnsupportedStatement(
        "break/continue outside a loop",
    ))
}

fn emit_loop_body(
    compiler: &mut Compiler,
    block: &Block,
    continue_label: &str,
    break_label: &str,
) -> Result<(), CompileError> {
    let pending = ctx(compiler).pending_loop_name.take();
    let cleanups = ctx(compiler).cleanups.len();
    let (name, break_label, break_cleanups) = match pending {
        Some((name, end, at)) => (Some(name), end, at),
        None => (None, break_label.to_owned(), cleanups),
    };
    ctx(compiler).loops.push(LoopLabels {
        name,
        cleanups,
        break_cleanups,
        continue_label: continue_label.to_owned(),
        break_label,
        block: false,
    });
    let res = emit_block(compiler, block);
    ctx(compiler).loops.pop();
    res
}

fn saves_cache(compiler: &Compiler, start: usize) -> bool {
    compiler.nodes[start..].iter().any(|n| {
        matches!(n, Node::Instruction(i, _) if super::procs::touches_cache(i)
            || matches!(i, Instruction::CallGlob(..) | Instruction::CallGlobalArgList(_) | Instruction::ListGet))
    })
}

fn unfolded_saves_cache(compiler: &mut Compiler, value: &Expression) -> bool {
    unfolded_has(compiler, value, |compiler, start| {
        compiler.global_field || saves_cache(compiler, start)
    })
}

pub(super) fn unfolded_has(
    compiler: &mut Compiler,
    value: &Expression,
    found: impl Fn(&Compiler, usize) -> bool,
) -> bool {
    let start = compiler.nodes.len();
    let saved = (
        compiler.label_count,
        compiler.cache.clone(),
        compiler.last_line,
        compiler.label_states.clone(),
        compiler.global_field,
        compiler.other_file,
        compiler.line_pending,
    );
    compiler.no_fold = true;
    compiler.global_field = false;
    let ok = emit_value(compiler, value).is_ok();
    compiler.no_fold = false;
    let saves = ok && found(compiler, start);
    compiler.nodes.truncate(start);
    (
        compiler.label_count,
        compiler.cache,
        compiler.last_line,
        compiler.label_states,
        compiler.global_field,
        compiler.other_file,
        compiler.line_pending,
    ) = saved;
    saves
}

fn unparenthesised(expr: &Expression) -> Expression {
    let mut expr = expr;
    while let Expression::Base { term, follow } = expr {
        match &term.elem {
            Term::Expr(inner) if follow.is_empty() => expr = inner,
            _ => break,
        }
    }
    expr.clone()
}

fn lvalue_var(compiler: &mut Compiler, kind: EvalKind) -> Result<Variable, CompileError> {
    match kind {
        EvalKind::Var(var) if super::is_writable(&var) => Ok(var),
        EvalKind::Field(builder, field) => {
            Ok(builder.get_field(crate::operands::DMString(field.into())))
        }
        EvalKind::ListRef => {
            compiler.emit_ins(Instruction::SetVar(Variable::CacheKey));
            compiler.emit_ins(Instruction::SetVar(Variable::Cache));
            Ok(Variable::CacheIndex)
        }
        _ => Err(CompileError::ExpectedLValue),
    }
}

fn is_deletable_place(expr: &Expression) -> bool {
    let Expression::Base { term, follow } = expr else {
        return false;
    };
    match follow.last().map(|f| &f.elem) {
        None => match &term.elem {
            Term::Ident(name) => name.as_str() != "world",
            Term::GlobalIdent(_) => true,
            _ => false,
        },
        Some(Follow::Field(
            dreammaker::ast::PropertyAccessKind::Dot | dreammaker::ast::PropertyAccessKind::Colon,
            name,
        )) => {
            !matches!(name.as_str(), "type" | "parent_type")
                && follow[..follow.len() - 1]
                    .iter()
                    .all(|f| matches!(f.elem, Follow::Field(..)))
        }
        Some(Follow::Index(dreammaker::ast::ListAccessKind::Normal, _)) => true,
        _ => false,
    }
}

fn is_src(expr: &Expression) -> bool {
    matches!(expr, Expression::Base { term, follow }
        if follow.is_empty() && matches!(&term.elem, Term::Ident(i) if i.as_str() == "src"))
}

fn strip_parens(expr: &Expression) -> &Expression {
    if let Expression::Base { term, follow } = expr {
        if follow.is_empty() {
            if let Term::Expr(inner) = &term.elem {
                return strip_parens(inner);
            }
        }
    }
    expr
}

pub(super) struct SafeChain {
    segments: Vec<Vec<Follow>>,
    last: Vec<Follow>,
    index: Option<Expression>,
}

pub(super) enum SafeWrite<'w> {
    Assign,
    Change(&'w dyn Fn(Variable) -> Vec<Instruction>),
    Logical { or: bool },
}

fn is_safe_follow(f: &Follow) -> bool {
    use dreammaker::ast::{ListAccessKind as L, PropertyAccessKind as K};
    matches!(
        f,
        Follow::Field(K::SafeDot | K::SafeColon, _)
            | Follow::Call(K::SafeDot | K::SafeColon, _, _)
            | Follow::Index(L::Safe, _)
    )
}

pub(super) fn safe_chain(follow: &[Follow]) -> Option<SafeChain> {
    use dreammaker::ast::{ListAccessKind as L, PropertyAccessKind as K};
    let mut segments: Vec<Vec<Follow>> = vec![];
    let mut index = None;
    for (i, f) in follow.iter().enumerate() {
        let last = i + 1 == follow.len();
        match f {
            Follow::Field(K::SafeDot | K::SafeColon, name) => {
                segments.push(vec![Follow::Field(K::Dot, name.clone())])
            }
            Follow::Call(K::SafeDot | K::SafeColon, name, args) => {
                segments.push(vec![Follow::Call(K::Dot, name.clone(), args.clone())])
            }
            Follow::Field(K::Dot | K::Colon, _) | Follow::Call(K::Dot | K::Colon, _, _) => {
                segments.last_mut()?.push(f.clone())
            }
            Follow::Index(L::Normal, e) if last => index = Some((**e).clone()),
            Follow::Index(L::Normal, _) => segments.last_mut()?.push(f.clone()),
            Follow::Index(L::Safe, e) if last => {
                segments.push(vec![]);
                index = Some((**e).clone())
            }
            Follow::Index(L::Safe, e) => segments.push(vec![Follow::Index(L::Normal, e.clone())]),
            _ => return None,
        }
    }
    let last = segments.pop()?;
    let calls = |s: &Vec<Follow>| s.iter().filter(|f| matches!(f, Follow::Call(..))).count();
    if segments.iter().any(|s| calls(s) > 1) {
        return None;
    }
    Some(SafeChain {
        segments,
        last,
        index,
    })
}

fn safe_place(expr: &Expression) -> Option<(Expression, SafeChain)> {
    let Expression::Base { term, follow } = expr else {
        return None;
    };
    let first = follow.iter().position(|f| is_safe_follow(&f.elem))?;
    let rest: Vec<Follow> = follow[first..].iter().map(|f| f.elem.clone()).collect();
    let chain = safe_chain(&rest)?;
    let base = Expression::Base {
        term: term.clone(),
        follow: follow[..first].to_vec().into_boxed_slice(),
    };
    Some((base, chain))
}

impl SafeChain {
    fn fields(&self) -> Option<Vec<String>> {
        self.last
            .iter()
            .map(|f| match f {
                Follow::Field(_, name) => Some(name.to_string()),
                _ => None,
            })
            .collect()
    }

    fn computed(&self) -> (Vec<Follow>, Vec<String>) {
        let split = self
            .last
            .iter()
            .rposition(|f| !matches!(f, Follow::Field(..)))
            .map_or(0, |i| i + 1);
        let fields = self.last[split..]
            .iter()
            .map(|f| match f {
                Follow::Field(_, name) => name.to_string(),
                _ => unreachable!(),
            })
            .collect();
        (self.last[..split].to_vec(), fields)
    }

    pub fn writable(&self, write: &SafeWrite, value_form: bool) -> bool {
        match (self.fields(), write) {
            (Some(_), SafeWrite::Assign) => self.index.is_some(),
            (Some(_), SafeWrite::Change(_)) => true,
            (Some(_), SafeWrite::Logical { .. }) => !value_form,
            (None, SafeWrite::Assign) => self.index.is_some(),
            (None, _) => {
                self.index.is_some() || matches!(self.last.last(), Some(Follow::Field(..)))
            }
        }
    }
}

fn call_after_cache_use(compiler: &Compiler, start: usize) -> Option<usize> {
    let call = start
        + compiler.nodes[start..]
            .iter()
            .position(|n| matches!(n, Node::Instruction(Instruction::Call(..), _)))?;
    compiler.nodes[start..call]
        .iter()
        .any(|n| matches!(n, Node::Instruction(i, _) if super::procs::touches_cache(i)))
        .then_some(call)
}

fn restore_receiver(compiler: &mut Compiler, call: usize) {
    compiler.warn_bug(SAFE_RECEIVER_BUG, SAFE_RECEIVER_OURS);
    compiler
        .nodes
        .insert(call, Node::Instruction(Instruction::PushCache, ()));
    compiler
        .nodes
        .insert(call, Node::Instruction(Instruction::PopCache, ()));
}

pub(super) fn emit_safe_chain_write(
    compiler: &mut Compiler,
    chain: SafeChain,
    place: Option<&Expression>,
    static_var: Option<Variable>,
    value: Option<&Expression>,
    write: SafeWrite,
    value_form: bool,
) -> Result<(), CompileError> {
    let fields = chain.fields();
    let computed = chain.computed();
    let jump = |end: &str| {
        let end = Label::Named(end.to_owned());
        if value_form {
            Instruction::SetCacheJmpIfNull(end)
        } else {
            Instruction::SetCachePopJmpIfNull(end)
        }
    };
    let mut ends = vec![];
    for segment in chain.segments.into_iter().chain([vec![]]) {
        let end = compiler.new_label();
        compiler.emit_ins(jump(&end));
        compiler.emit_ins(Instruction::PushCache);
        ends.push(end);
        if segment.is_empty() {
            break;
        }
        let start = compiler.nodes.len();
        let kind =
            super::follow::emit_typed(compiler, segment, EvalKind::Var(Variable::Cache), None)?;
        compiler.emit_move_to_stack(kind)?;
        if let Some(call) = call_after_cache_use(compiler, start) {
            restore_receiver(compiler, call);
        }
    }
    let implicit = place.and_then(|p| compiler.static_type_name(p));
    let value_start = compiler.nodes.len();
    if let (Some(value), false) = (value, matches!(write, SafeWrite::Logical { .. })) {
        compiler.implicit_type = implicit.clone();
        emit_value(compiler, value)?;
        compiler.implicit_type = None;
    }
    let field_var = |fields: &[String]| {
        if let Some(var) = &static_var {
            return var.clone();
        }
        if fields.is_empty() {
            return Variable::Cache;
        }
        let mut builder = super::chain_builder::ChainBuilder::begin(Variable::Cache);
        for f in &fields[..fields.len() - 1] {
            builder.append(crate::operands::DMString(f.clone().into_bytes()));
        }
        builder.get_field(crate::operands::DMString(
            fields[fields.len() - 1].clone().into_bytes(),
        ))
    };
    let Some(fields) = fields else {
        let (computed, fields) = computed;
        let value_used_cache = compiler.nodes[value_start..]
            .iter()
            .any(|n| matches!(n, Node::Instruction(i, _) if super::procs::touches_cache(i)));
        let copies = match &write {
            SafeWrite::Assign => true,
            SafeWrite::Change(write) => {
                matches!(write(Variable::Cache)[..], [Instruction::SetVarExpr(_)])
            }
            SafeWrite::Logical { .. } => false,
        };
        if value_form && copies {
            compiler.emit_ins(Instruction::PushTop);
        }
        if value_used_cache {
            compiler.warn_bug(SAFE_RECEIVER_BUG, SAFE_RECEIVER_OURS);
            compiler.emit_ins(Instruction::PopCache);
            compiler.emit_ins(Instruction::PushCache);
        }
        let start = compiler.nodes.len();
        let kind =
            super::follow::emit_typed(compiler, computed, EvalKind::Var(Variable::Cache), None)?;
        compiler.emit_move_to_stack(kind)?;
        if let Some(call) = call_after_cache_use(compiler, start) {
            restore_receiver(compiler, call);
        }
        let mut var = None;
        if !fields.is_empty() {
            compiler.emit_ins(Instruction::SetVar(Variable::Cache));
            var = Some(field_var(&fields));
        }
        let mut holder_saved = true;
        if let Some(index) = chain.index {
            if let Some(var) = var {
                compiler.emit_ins(Instruction::GetVar(var));
            }
            emit_value(compiler, &index)?;
            if matches!(write, SafeWrite::Assign) {
                compiler.emit_ins(Instruction::ListSet);
            } else {
                compiler.emit_ins(Instruction::PopCache);
                holder_saved = false;
                compiler.emit_ins(Instruction::SetVar(Variable::CacheKey));
                compiler.emit_ins(Instruction::SetVar(Variable::Cache));
            }
            var = Some(Variable::CacheIndex);
        }
        let var = var.expect("a field or an element");
        match write {
            SafeWrite::Assign => {}
            SafeWrite::Change(write) => {
                for ins in write(var) {
                    compiler.emit_ins(match ins {
                        Instruction::SetVarExpr(var) => Instruction::SetVar(var),
                        ins => ins,
                    });
                }
            }
            SafeWrite::Logical { or } => {
                if !compiler.byond.keeps_safe_logical_null_tests() {
                    compiler.warn_bug(SAFE_LOGICAL_BUG, "; dmasm keeps them");
                }
                let end = Label::Named(compiler.new_label());
                compiler.emit_ins(Instruction::GetVar(var.clone()));
                match (value_form, or) {
                    (true, true) => compiler.emit_ins(Instruction::JmpOr(end.clone())),
                    (true, false) => compiler.emit_ins(Instruction::JmpAnd(end.clone())),
                    (false, _) => {
                        compiler.emit_ins(Instruction::Test);
                        compiler.emit_ins(if or {
                            Instruction::Jnz(end.clone())
                        } else {
                            Instruction::Jz(end.clone())
                        });
                    }
                }
                let start = compiler.nodes.len();
                compiler.implicit_type = implicit;
                emit_value(compiler, value.expect("a value"))?;
                compiler.implicit_type = None;
                if compiler.nodes[start..]
                    .iter()
                    .any(|n| matches!(n, Node::Instruction(i, _) if super::procs::touches_cache(i)))
                {
                    compiler
                        .nodes
                        .insert(start, Node::Instruction(Instruction::PushCache, ()));
                    compiler.emit_ins(Instruction::PopCache);
                }
                compiler.emit_ins(if value_form {
                    Instruction::SetVarExpr(var)
                } else {
                    Instruction::SetVar(var)
                });
                let Label::Named(end) = end else {
                    unreachable!()
                };
                compiler.emit_label(end);
            }
        }
        if !holder_saved {
            ends.pop().map(|end| compiler.emit_label(end));
        }
        for end in ends.into_iter().rev() {
            compiler.emit_ins(Instruction::PopCache);
            compiler.emit_label(end);
        }
        return Ok(());
    };
    match (fields, write, chain.index) {
        (fields, SafeWrite::Change(write), None) => {
            compiler.emit_ins(Instruction::PopCache);
            for ins in write(field_var(&fields)) {
                compiler.emit_ins(ins);
            }
            ends.pop().map(|end| compiler.emit_label(end));
        }
        (fields, SafeWrite::Assign, Some(index)) => {
            compiler.emit_ins(Instruction::PopCache);
            if value_form {
                compiler.emit_ins(Instruction::PushTop);
            }
            compiler.emit_ins(Instruction::GetVar(field_var(&fields)));
            emit_value(compiler, &index)?;
            compiler.emit_ins(Instruction::ListSet);
            ends.pop().map(|end| compiler.emit_label(end));
        }
        (fields, SafeWrite::Change(write), Some(index)) => {
            let restore_first = compiler.byond.restores_cache_for_safe_element();
            if !restore_first {
                warn_list_from_cache(compiler);
            }
            if restore_first {
                compiler.emit_ins(Instruction::PopCache);
            }
            compiler.emit_ins(Instruction::GetVar(field_var(&fields)));
            emit_value(compiler, &index)?;
            if !restore_first {
                compiler.emit_ins(Instruction::PopCache);
            }
            compiler.emit_ins(Instruction::SetVar(Variable::CacheKey));
            compiler.emit_ins(Instruction::SetVar(Variable::Cache));
            for ins in write(Variable::CacheIndex) {
                compiler.emit_ins(ins);
            }
            ends.pop().map(|end| compiler.emit_label(end));
        }
        (fields, SafeWrite::Logical { or }, index) => {
            if !compiler.byond.keeps_safe_logical_null_tests() {
                compiler.warn_bug(SAFE_LOGICAL_BUG, "; dmasm keeps them");
            }
            let restore_first = index.is_none() || compiler.byond.restores_cache_for_safe_element();
            if !restore_first {
                warn_list_from_cache(compiler);
            }
            if restore_first {
                compiler.emit_ins(Instruction::PopCache);
            }
            let mut var = field_var(&fields);
            if let Some(index) = index {
                compiler.emit_ins(Instruction::GetVar(var));
                emit_value(compiler, &index)?;
                if !restore_first {
                    compiler.emit_ins(Instruction::PopCache);
                }
                compiler.emit_ins(Instruction::SetVar(Variable::CacheKey));
                compiler.emit_ins(Instruction::SetVar(Variable::Cache));
                var = Variable::CacheIndex;
            }
            compiler.emit_ins(Instruction::GetVar(var.clone()));
            compiler.emit_ins(Instruction::Test);
            let end = ends.pop().expect("a segment");
            compiler.emit_ins(if or {
                Instruction::Jnz(Label::Named(end.clone()))
            } else {
                Instruction::Jz(Label::Named(end.clone()))
            });
            let start = compiler.nodes.len();
            compiler.implicit_type = implicit;
            emit_value(compiler, value.expect("a value"))?;
            compiler.implicit_type = None;
            if compiler.nodes[start..]
                .iter()
                .any(|n| matches!(n, Node::Instruction(i, _) if super::procs::touches_cache(i)))
            {
                compiler
                    .nodes
                    .insert(start, Node::Instruction(Instruction::PushCache, ()));
                compiler.emit_ins(Instruction::PopCache);
            }
            compiler.emit_ins(Instruction::SetVar(var));
            compiler.emit_label(end);
        }
        _ => unreachable!("not writable"),
    }
    for end in ends.into_iter().rev() {
        compiler.emit_ins(Instruction::PopCache);
        compiler.emit_label(end);
    }
    Ok(())
}

pub(super) fn static_place(
    compiler: &Compiler,
    place: &Expression,
    indexed: bool,
) -> Option<Variable> {
    let Expression::Base { term, follow } = place else {
        return None;
    };
    let base = Expression::Base {
        term: term.clone(),
        follow: Default::default(),
    };
    let follow: Vec<Follow> = follow.iter().map(|f| f.elem.clone()).collect();
    static_member(
        compiler,
        super::procs::expr_type(compiler, &base),
        &follow,
        indexed,
    )
}

pub(super) fn static_member(
    compiler: &Compiler,
    ty: Option<dreammaker::objtree::TypeRef>,
    follow: &[Follow],
    indexed: bool,
) -> Option<Variable> {
    let follow = &follow[..follow.len().checked_sub(indexed as usize)?];
    let (Follow::Field(_, name), holder) = (follow.last()?, &follow[..follow.len() - 1]) else {
        return None;
    };
    let mut ty = ty?;
    for f in holder {
        let Follow::Field(_, field) = f else {
            return None;
        };
        ty = super::procs::declared_type(
            compiler,
            &super::procs::member_var(ty, field.as_str())?.get().var_type,
        )?;
    }
    let decl = super::procs::member_var(ty, name.as_str())?;
    if !super::procs::is_global_member(&decl) {
        return None;
    }
    Some(Variable::Global(crate::operands::GlobalVar::named(
        name.as_str().as_bytes(),
        Some(crate::operands::StaticScope::of_type(&decl.ty().get().path)),
    )))
}

pub(super) fn emit_safe_write(
    compiler: &mut Compiler,
    place: &Expression,
    value: Option<&Expression>,
    write: SafeWrite,
    value_form: bool,
) -> Result<bool, CompileError> {
    let Some((base, chain)) = safe_place(place) else {
        return Ok(false);
    };
    if !chain.writable(&write, value_form) {
        return Ok(false);
    }
    emit_value(compiler, &base)?;
    let static_var = static_place(compiler, place, chain.index.is_some());
    emit_safe_chain_write(
        compiler,
        chain,
        Some(place),
        static_var,
        value,
        write,
        value_form,
    )?;
    Ok(true)
}

fn emit_expr_statement(compiler: &mut Compiler, expr: &Expression) -> Result<(), CompileError> {
    if matches!(expr, Expression::Base { term, follow }
        if follow.is_empty() && matches!(&term.elem, Term::Prefab(p) if p.vars.is_empty()))
    {
        return Ok(());
    }
    if let Expression::Base { term, follow } = strip_parens(expr) {
        let plain = matches!(term.elem, Term::Ident(_) | Term::Null)
            && follow.iter().all(|f| {
                matches!(
                    f.elem,
                    Follow::Field(
                        dreammaker::ast::PropertyAccessKind::Dot
                            | dreammaker::ast::PropertyAccessKind::Colon,
                        _
                    )
                )
            });
        if plain {
            return Ok(());
        }
    }
    emit_expr_statement_inner(compiler, expr)?;
    let n = compiler.nodes.len();
    if n >= 3
        && matches!(
            compiler.nodes[n - 1],
            Node::Instruction(Instruction::Pop, _)
        )
        && matches!(
            compiler.nodes[n - 2],
            Node::Instruction(Instruction::GetFlag, _)
        )
        && !matches!(
            compiler.nodes[n - 3],
            Node::Instruction(Instruction::Pop, _)
        )
    {
        compiler.nodes.truncate(n - 2);
    }
    let assignment = match strip_parens(expr) {
        Expression::BinaryOp {
            op: dreammaker::ast::BinaryOp::RShift,
            ..
        } => true,
        Expression::AssignOp { lhs, .. } => !matches!(&**lhs, Expression::Base { follow, .. }
            if matches!(follow.last().map(|f| &f.elem), Some(Follow::Index(..)))),
        Expression::Base { follow, .. } => matches!(
            follow.last().map(|f| &f.elem),
            Some(Follow::Unary(
                UnaryOp::PreIncr | UnaryOp::PostIncr | UnaryOp::PreDecr | UnaryOp::PostDecr
            ))
        ),
        _ => false,
    };
    let n = compiler.nodes.len();
    let value_set = n >= 2
        && matches!(
            compiler.nodes[n - 1],
            Node::Instruction(Instruction::Pop, _)
        )
        && matches!(
            compiler.nodes[n - 2],
            Node::Instruction(Instruction::SetVarExpr(_), _)
        );
    if assignment && !value_set {
        compiler.end_assignment_statement();
    }
    Ok(())
}

fn emit_expr_statement_inner(
    compiler: &mut Compiler,
    expr: &Expression,
) -> Result<(), CompileError> {
    match expr {
        Expression::BinaryOp {
            op: dreammaker::ast::BinaryOp::In,
            lhs,
            rhs,
        } if matches!(&**lhs, Expression::AssignOp { .. }) => {
            emit_value(compiler, rhs)?;
            emit_value(compiler, lhs)?;
            compiler.emit_ins(Instruction::IsIn(crate::operands::IsInParams::Value));
            Ok(())
        }

        Expression::AssignOp { op, lhs, rhs }
            if !matches!(op, AssignOp::AndAssign | AssignOp::OrAssign)
                && safe_place(lhs).is_some() =>
        {
            let op = *op;
            let element = safe_place(lhs).is_some_and(|(_, c)| c.index.is_some());
            let to_field = move |var: Variable| {
                vec![if op == AssignOp::Assign {
                    Instruction::SetVar(var)
                } else {
                    aug_instruction(op, var)
                }]
            };
            let write = if element && op == AssignOp::Assign {
                SafeWrite::Assign
            } else {
                SafeWrite::Change(&to_field)
            };
            if !emit_safe_write(compiler, lhs, Some(rhs), write, false)? {
                return Err(CompileError::ExpectedLValue);
            }
            Ok(())
        }

        Expression::AssignOp {
            op: op @ (AssignOp::OrAssign | AssignOp::AndAssign),
            lhs,
            rhs,
        } if safe_place(lhs)
            .is_some_and(|(_, c)| c.writable(&SafeWrite::Logical { or: true }, false)) =>
        {
            let write = SafeWrite::Logical {
                or: *op == AssignOp::OrAssign,
            };
            emit_safe_write(compiler, lhs, Some(rhs), write, false)?;
            Ok(())
        }

        Expression::AssignOp { op, lhs, rhs }
            if !matches!(op, AssignOp::AndAssign | AssignOp::OrAssign) =>
        {
            if *op == AssignOp::Assign {
                compiler.implicit_type = compiler
                    .static_type_name(lhs)
                    .or_else(|| list_type_of(compiler, lhs));
            }
            emit_value(compiler, rhs)?;
            compiler.implicit_type = None;
            warn_pointer_index(compiler, lhs);
            compiler.place = *op != AssignOp::Assign;
            let kind = compiler.emit_inner_expr(unparenthesised(lhs))?;
            if *op == AssignOp::Assign {
                if kind == EvalKind::ListRef {
                    compiler.emit_ins(Instruction::ListSet);
                    return Ok(());
                }
                let var = lvalue_var(compiler, kind)?;
                compiler.emit_ins(Instruction::SetVar(var));
                return Ok(());
            }
            let var = lvalue_var(compiler, kind)?;
            compiler.emit_ins(aug_instruction(*op, var));
            Ok(())
        }

        Expression::Base { term, follow }
            if matches!(
                follow.last().map(|f| &f.elem),
                Some(Follow::Unary(
                    UnaryOp::PreIncr | UnaryOp::PostIncr | UnaryOp::PreDecr | UnaryOp::PostDecr
                ))
            ) =>
        {
            let Some(Follow::Unary(op)) = follow.last().map(|f| f.elem.clone()) else {
                unreachable!()
            };
            let inner = unparenthesised(&Expression::Base {
                term: term.clone(),
                follow: follow[..follow.len() - 1].to_vec().into_boxed_slice(),
            });
            let inc = matches!(op, UnaryOp::PreIncr | UnaryOp::PostIncr);
            let step = move |var: Variable| {
                vec![if inc {
                    Instruction::Inc(var)
                } else {
                    Instruction::Dec(var)
                }]
            };
            if emit_safe_write(compiler, &inner, None, SafeWrite::Change(&step), false)? {
                return Ok(());
            }
            let kind = compiler.emit_inner_expr(inner)?;
            let var = lvalue_var(compiler, kind)?;
            match op {
                UnaryOp::PreIncr | UnaryOp::PostIncr => compiler.emit_ins(Instruction::Inc(var)),
                _ => compiler.emit_ins(Instruction::Dec(var)),
            }
            Ok(())
        }

        Expression::AssignOp {
            op: op @ (AssignOp::OrAssign | AssignOp::AndAssign),
            lhs,
            rhs,
        } => {
            let end = compiler.new_label();
            let jump = |end: &str| {
                if *op == AssignOp::OrAssign {
                    Instruction::Jnz(Label::Named(end.to_owned()))
                } else {
                    Instruction::Jz(Label::Named(end.to_owned()))
                }
            };
            let implicit = compiler
                .static_type_name(lhs)
                .or_else(|| list_type_of(compiler, lhs));
            let kind = compiler.emit_inner_expr((**lhs).clone())?;
            if matches!(kind, EvalKind::ListRef | EvalKind::Field(..)) {
                let field = matches!(kind, EvalKind::Field(..));
                let var = lvalue_var(compiler, kind)?;
                let before = compiler.cache.clone();
                let explicit_src = compiler.explicit_src;
                compiler.emit_ins(Instruction::GetVar(var.clone()));
                if field && compiler.cache.as_ref().is_some_and(|h| h.len() > 1) {
                    compiler.cache = before.clone();
                }
                compiler.emit_ins(Instruction::Test);
                compiler.emit_ins(jump(&end));
                let start = compiler.nodes.len();
                compiler.implicit_type = implicit.clone();
                compiler.global_field = false;
                emit_value(compiler, rhs)?;
                compiler.implicit_type = None;
                let saves = std::mem::take(&mut compiler.global_field)
                    || saves_cache(compiler, start)
                    || unfolded_saves_cache(compiler, rhs);
                let saves_key = !field
                    && compiler.nodes[start..]
                        .iter()
                        .any(|n| matches!(n, Node::Instruction(Instruction::ListGet, _)));
                if saves {
                    if saves_key {
                        compiler
                            .nodes
                            .insert(start, Node::Instruction(Instruction::PushCacheKey, ()));
                    }
                    compiler
                        .nodes
                        .insert(start, Node::Instruction(Instruction::PushCache, ()));
                    if saves_key {
                        compiler.emit_ins(Instruction::PopCacheKey);
                    }
                    compiler.emit_ins(Instruction::PopCache);
                }
                if field {
                    compiler.cache = before;
                    compiler.explicit_src = explicit_src;
                }
                compiler.emit_ins(Instruction::SetVar(var));
                compiler.emit_label(end);
                return Ok(());
            }
            let var = lvalue_var(compiler, kind)?;
            compiler.emit_ins(Instruction::GetVar(var.clone()));
            compiler.emit_ins(Instruction::Test);
            compiler.emit_ins(jump(&end));
            compiler.end_assignment_statement();
            compiler.implicit_type = implicit.clone();
            emit_value(compiler, rhs)?;
            compiler.implicit_type = None;
            compiler.emit_ins(Instruction::SetVar(var));
            compiler.emit_label(end);
            Ok(())
        }

        Expression::BinaryOp {
            op: dreammaker::ast::BinaryOp::LShift,
            lhs,
            rhs,
        } => emit_output(compiler, lhs, rhs),

        Expression::BinaryOp {
            op: dreammaker::ast::BinaryOp::RShift,
            lhs,
            rhs,
        } => emit_input(compiler, lhs, rhs),

        Expression::Base { term, follow }
            if follow.is_empty()
                && matches!(&term.elem, Term::Call(name, _) if name.as_str() == "animate") =>
        {
            if let Some(end) = compiler.named_call_end(expr) {
                compiler.read_on(end);
            }
            let Term::Call(name, args) = &term.elem else {
                unreachable!()
            };
            if !super::builtin_procs::emit_statement_builtin(compiler, name.as_str(), args)? {
                emit_value(compiler, expr)?;
            }
            Ok(())
        }

        Expression::Base { term, follow }
            if follow.is_empty()
                && matches!(&term.elem, Term::Call(name, args)
                    if super::builtin_procs::emit_statement_builtin(compiler, name.as_str(), args)?) =>
        {
            Ok(())
        }

        other => {
            emit_value(compiler, other)?;
            let n = compiler.nodes.len();
            let parenthesized = matches!(other, Expression::Base { term, follow }
                if follow.is_empty() && matches!(term.elem, Term::Expr(_)));
            let keep_call = parenthesized
                || n >= 2
                    && matches!(
                        compiler.nodes[n - 2],
                        Node::Instruction(Instruction::PopCache, _)
                    );
            if let Some(Node::Instruction(ins @ Instruction::Call(..), _)) =
                compiler.nodes.last_mut().filter(|_| !keep_call)
            {
                if let Instruction::Call(var, count) = ins.clone() {
                    *ins = Instruction::CallStatement(var, count);
                }
            }
            if !matches!(
                compiler.nodes.last(),
                Some(Node::Instruction(Instruction::Missile, _))
            ) {
                compiler.emit_ins(Instruction::Pop);
            }
            Ok(())
        }
    }
}

fn emit_io_target(compiler: &mut Compiler, target: &Expression) -> Result<(), CompileError> {
    if let Expression::Base { term, follow } = target {
        if let Some(Follow::Index(dreammaker::ast::ListAccessKind::Normal, key)) =
            follow.last().map(|f| &f.elem)
        {
            let base = Expression::Base {
                term: term.clone(),
                follow: follow[..follow.len() - 1].to_vec().into_boxed_slice(),
            };
            emit_value(compiler, &base)?;
            emit_value(compiler, key)?;
            compiler.emit_ins(Instruction::Index);
            return Ok(());
        }
    }
    emit_value(compiler, target)
}

fn emit_input(
    compiler: &mut Compiler,
    source: &Expression,
    place: &Expression,
) -> Result<(), CompileError> {
    emit_io_target(compiler, source)?;
    compiler.emit_ins(Instruction::Read);
    let kind = compiler.emit_inner_expr(place.clone())?;
    if kind == EvalKind::ListRef {
        compiler.emit_ins(Instruction::ListSet);
    } else {
        let var = lvalue_var(compiler, kind)?;
        compiler.emit_ins(Instruction::SetVar(var));
    }
    Ok(())
}

fn emit_output(
    compiler: &mut Compiler,
    target: &Expression,
    value: &Expression,
) -> Result<(), CompileError> {
    emit_io_target(compiler, target)?;
    if let Expression::Base { term, follow } = strip_parens(value) {
        if follow.is_empty() {
            match &term.elem {
                Term::Call(name, args) => {
                    let n = args.len();
                    let ins = match (name.as_str(), n) {
                        ("browse", 1) => Some(Instruction::Browse),
                        ("browse", 2) => Some(Instruction::BrowseOpt),
                        ("browse_rsc", 1 | 2) => Some(Instruction::BrowseRsc),
                        ("output", 2) => Some(Instruction::WinOutput),
                        ("link", 1) => Some(Instruction::Link),
                        ("ftp", 1 | 2) => Some(Instruction::OutputFtp),
                        ("run", 1) => Some(Instruction::OutputRun),
                        ("load_resource", _) => Some(Instruction::LoadResource(n as u32 + 1)),
                        _ => None,
                    };
                    if let Some(ins) = ins {
                        for arg in args.iter() {
                            emit_value(compiler, arg)?;
                        }
                        let pad = match (name.as_str(), n) {
                            ("ftp", 1) | ("browse_rsc", 1) => 1,
                            _ => 0,
                        };
                        for _ in 0..pad {
                            compiler.emit_ins(Instruction::PushVal(Value::Null));
                        }
                        compiler.emit_ins(ins);
                        return Ok(());
                    }
                }
                Term::InterpString(..) => {
                    emit_value(compiler, value)?;
                    if let Some(Node::Instruction(Instruction::Format(pattern, count), _)) =
                        compiler.nodes.last().cloned()
                    {
                        compiler.nodes.pop();
                        compiler.emit_ins(Instruction::OutputFormat(pattern, count));
                        return Ok(());
                    }
                    compiler.emit_ins(Instruction::Output);
                    return Ok(());
                }
                _ => {}
            }
        }
    }
    emit_value(compiler, value)?;
    compiler.emit_ins(Instruction::Output);
    Ok(())
}

pub(super) fn aug_instruction(op: AssignOp, var: Variable) -> Instruction {
    match op {
        AssignOp::AddAssign => Instruction::AugAdd(var),
        AssignOp::SubAssign => Instruction::AugSub(var),
        AssignOp::MulAssign => Instruction::AugMul(var),
        AssignOp::DivAssign => Instruction::AugDiv(var),
        AssignOp::ModAssign => Instruction::AugMod(var),
        AssignOp::FloatModAssign => Instruction::AugFloatMod(var),
        AssignOp::AssignInto => Instruction::AssignInto(var),
        AssignOp::BitAndAssign => Instruction::AugBand(var),
        AssignOp::BitOrAssign => Instruction::AugBor(var),
        AssignOp::BitXorAssign => Instruction::AugXor(var),
        AssignOp::LShiftAssign => Instruction::AugLShift(var),
        AssignOp::RShiftAssign => Instruction::AugRShift(var),
        AssignOp::Assign | AssignOp::AndAssign | AssignOp::OrAssign => unreachable!(),
    }
}

pub(super) fn emit_safe_logical_assign(
    compiler: &mut Compiler,
    place: &Expression,
    rhs: &Expression,
    or: bool,
) -> Result<bool, CompileError> {
    let Some((base, chain)) = safe_place(place) else {
        return Ok(false);
    };
    let Some(fields) = chain.fields() else {
        return Ok(false);
    };
    if !chain.segments.is_empty() || chain.index.is_some() || fields.is_empty() {
        return Ok(false);
    }
    if !compiler.byond.keeps_safe_logical_null_tests() {
        compiler.warn_bug(SAFE_LOGICAL_BUG, "; dmasm keeps them");
    }
    let mut builder = super::chain_builder::ChainBuilder::begin(Variable::Cache);
    for f in &fields[..fields.len() - 1] {
        builder.append(crate::operands::DMString(f.clone().into_bytes()));
    }
    let var = builder.get_field(crate::operands::DMString(
        fields[fields.len() - 1].clone().into_bytes(),
    ));
    emit_value(compiler, &base)?;
    let end = compiler.new_label();
    compiler.emit_ins(Instruction::SetCacheJmpIfNull(Label::Named(end.clone())));
    compiler.emit_ins(Instruction::PushCache);
    compiler.emit_ins(Instruction::PopCache);
    compiler.emit_ins(Instruction::GetVar(var.clone()));
    compiler.emit_ins(if or {
        Instruction::JmpOr(Label::Named(end.clone()))
    } else {
        Instruction::JmpAnd(Label::Named(end.clone()))
    });
    let start = compiler.nodes.len();
    compiler.implicit_type = compiler.static_type_name(place);
    emit_value(compiler, rhs)?;
    compiler.implicit_type = None;
    if compiler.nodes[start..]
        .iter()
        .any(|n| matches!(n, Node::Instruction(i, _) if super::procs::touches_cache(i)))
    {
        compiler
            .nodes
            .insert(start, Node::Instruction(Instruction::PushCache, ()));
        compiler.emit_ins(Instruction::PopCache);
    }
    compiler.emit_ins(Instruction::SetVarExpr(var));
    compiler.emit_label(end);
    Ok(true)
}

fn path_division(default: &Expression) -> bool {
    let Expression::BinaryOp {
        op: dreammaker::ast::BinaryOp::Div,
        lhs,
        rhs,
    } = strip_parens(default)
    else {
        return false;
    };
    let name_chain = matches!(&**lhs, Expression::Base { term, follow }
        if matches!(term.elem, Term::Ident(_))
            && follow.iter().all(|f| matches!(f.elem, Follow::Field(dreammaker::ast::PropertyAccessKind::Dot, _))));
    let number = matches!(&**rhs, Expression::Base { term, follow }
        if matches!(term.elem, Term::Int(_) | Term::Float(_))
            && follow.iter().all(|f| matches!(f.elem, Follow::Unary(UnaryOp::Neg))));
    name_chain && number
}

fn warn_list_from_cache(compiler: &Compiler) {
    let since = compiler
        .nodes
        .iter()
        .rposition(|n| matches!(n, Node::Instruction(Instruction::PushCache, _)))
        .map_or(0, |i| i + 1);
    if compiler.nodes[since..]
        .iter()
        .any(|n| matches!(n, Node::Instruction(i, _) if super::procs::touches_cache(i)))
    {
        compiler.warn_bug(
            "before 516.1685, safe list assignment resolves the list using the RHS cache value",
            ", matching DreamMaker",
        );
    }
}

pub(super) const POINTER_INDEX_BUG: &str =
    "`*p[i]`: from 516.1668 DreamMaker leaves the index out, a \"bad index\" error for a list of \
     pointers";
pub(super) const POINTER_INDEX_OURS: &str = "; dmasm dereferences the indexed element";

pub(super) const SAFE_RECEIVER_BUG: &str =
    "`?.` chains that continue after a call or list access (`a?.f(b.x).v = b.y`): DreamMaker \
     uses the receiver left by the arguments or assigned value (b)";
pub(super) const SAFE_RECEIVER_OURS: &str = "; dmasm uses a";

const SAFE_LOGICAL_BUG: &str =
    "`?.` in `||=` or `&&=`: before 516.1673 DreamMaker leaves out its null tests (a runtime \
     error on null, and the first object's var with two `?.`)";

pub(super) fn warn_pointer_index(compiler: &Compiler, place: &Expression) {
    if let Expression::Base { follow, .. } = strip_parens(place) {
        if compiler.byond.drops_pointer_index()
            && follow.iter().any(|f| matches!(f.elem, Follow::Index(..)))
            && follow
                .iter()
                .any(|f| matches!(f.elem, Follow::Unary(UnaryOp::Dereference)))
        {
            compiler.warn_bug(POINTER_INDEX_BUG, POINTER_INDEX_OURS);
        }
    }
}
