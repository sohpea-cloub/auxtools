use dreammaker::ast::{AssignOp, Expression, Follow, ListAccessKind, PropertyAccessKind, Term};

use crate::{
    compiler::{CompileError, Compiler, EvalKind},
    operands::{DMString, Value},
    Instruction,
};

fn unroll(expr: &Expression) -> &Expression {
    match expr {
        Expression::Base { term, follow } => {
            if follow.is_empty() {
                if let Term::Expr(inner) = &term.elem {
                    return unroll(inner.as_ref());
                }
            }

            expr
        }

        _ => expr,
    }
}

fn can_short_circuit(expr: &Expression) -> bool {
    match expr {
        Expression::Base { term: _, follow } => {
            for follow in follow {
                match follow.elem {
                    Follow::Index(ListAccessKind::Safe, _) => return true,
                    Follow::Field(PropertyAccessKind::SafeDot, _) => return true,
                    Follow::Field(PropertyAccessKind::SafeColon, _) => return true,
                    Follow::Call(PropertyAccessKind::SafeDot, _, _) => return true,
                    Follow::Call(PropertyAccessKind::SafeColon, _, _) => return true,

                    _ => {}
                }
            }

            false
        }

        Expression::AssignOp { op: _, lhs, rhs: _ } => can_short_circuit(lhs),
        Expression::BinaryOp { .. } => false,
        Expression::TernaryOp { .. } => false,
    }
}

fn has_assoc_argument(context: ArgsContext, args: &[Expression]) -> Result<bool, CompileError> {
    let mut found_associative = false;

    for arg in args {
        match context {
            ArgsContext::Proc => {
                if let Expression::AssignOp {
                    op: AssignOp::Assign,
                    lhs,
                    rhs: _,
                } = arg
                {
                    if can_short_circuit(unroll(lhs)) {
                        continue;
                    }

                    if !matches!(lhs.as_ref(), Expression::Base { term, .. } if matches!(term.elem, Term::Expr(_)))
                    {
                        found_associative = true;
                    }
                }
            }

            ArgsContext::List => {
                if let Expression::AssignOp {
                    op: AssignOp::Assign,
                    lhs,
                    rhs: _,
                } = unroll(arg)
                {
                    if can_short_circuit(unroll(lhs)) {
                        continue;
                    }

                    found_associative = true;
                }
            }
        }
    }

    Ok(found_associative)
}

#[derive(PartialEq, Clone, Copy)]
pub(super) enum ArgsContext {
    Proc,
    List,
}

pub(super) enum ArgsResult {
    Normal,

    Assoc,

    ArgList,
}

pub(super) fn emit(
    compiler: &mut Compiler,
    context: ArgsContext,
    args: Vec<Expression>,
) -> Result<ArgsResult, CompileError> {
    if has_assoc_argument(context, &args)? {
        return do_assoc(compiler, context, args);
    }

    do_normal(compiler, args)
}

pub(super) fn emit_single_normal(
    compiler: &mut Compiler,
    context: ArgsContext,
    arg: Expression,
) -> Result<(), CompileError> {
    emit_normal(compiler, context, vec![arg])
}

pub(super) fn emit_normal(
    compiler: &mut Compiler,
    context: ArgsContext,
    args: Vec<Expression>,
) -> Result<(), CompileError> {
    let result = emit(compiler, context, args)?;

    match result {
        ArgsResult::Normal => Ok(()),
        ArgsResult::Assoc => Err(CompileError::UnexpectedNamedArguments),
        ArgsResult::ArgList => Err(CompileError::UnexpectedArgList),
    }
}

fn do_normal(compiler: &mut Compiler, args: Vec<Expression>) -> Result<ArgsResult, CompileError> {
    let args_len = args.len();

    for arg in args {
        compiler.regroup_root = true;
        let expr = compiler.emit_expr(arg)?;

        if let EvalKind::ArgList = expr {
            if args_len == 1 {
                return Ok(ArgsResult::ArgList);
            }
        }

        compiler.emit_move_to_stack(expr)?;
    }

    Ok(ArgsResult::Normal)
}

fn do_assoc(
    compiler: &mut Compiler,
    context: ArgsContext,
    args: Vec<Expression>,
) -> Result<ArgsResult, CompileError> {
    for (idx, arg) in args.into_iter().enumerate() {
        let idx = idx + 1;

        if let Expression::AssignOp {
            op: AssignOp::Assign,
            lhs,
            rhs,
        } = &arg
        {
            if let Expression::Base { term, follow } = lhs.as_ref() {
                if follow.is_empty() {
                    if let Term::Null = &term.elem {
                        compiler.emit_ins(Instruction::PushVal(Value::DMString(DMString(
                            b"null".to_vec(),
                        ))));
                        compiler.regroup_root = true;
                        let kind = compiler.emit_expr((**rhs).to_owned())?;
                        compiler.emit_move_to_stack(kind)?;
                        continue;
                    }

                    if let Term::Ident(ident) = &term.elem {
                        if context == ArgsContext::List
                            && super::procs::is_builtin_const(compiler, ident.as_str())
                        {
                            let value =
                                super::procs::const_value(compiler, ident.as_str()).unwrap();
                            compiler.emit_constant(value)?;
                        } else {
                            compiler.emit_ins(Instruction::PushVal(Value::DMString(DMString(
                                ident.as_str().as_bytes().to_vec(),
                            ))));
                        }

                        compiler.regroup_root = true;
                        let kind = compiler.emit_expr((**rhs).to_owned())?;
                        compiler.emit_move_to_stack(kind)?;
                        continue;
                    }
                }
            }
        }

        let arg = unroll(&arg);

        match arg {
            Expression::AssignOp {
                op: AssignOp::Assign,
                lhs,
                rhs,
            } => {
                compiler.regroup_root = true;
                let kind = compiler.emit_expr((**lhs).to_owned())?;
                compiler.emit_move_to_stack(kind)?;

                compiler.regroup_root = true;
                let implicit = compiler.destination_type(lhs);
                let outer = std::mem::replace(&mut compiler.implicit_type, implicit);
                let kind = compiler.emit_expr((**rhs).to_owned());
                compiler.implicit_type = outer;
                compiler.emit_move_to_stack(kind?)?;
            }

            other => {
                compiler.emit_ins(Instruction::PushInt(idx as i32));

                compiler.regroup_root = true;
                let kind = compiler.emit_expr(other.to_owned())?;
                compiler.emit_move_to_stack(kind)?;
            }
        }
    }

    Ok(ArgsResult::Assoc)
}
