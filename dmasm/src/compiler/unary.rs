use dreammaker::ast::UnaryOp;

use crate::{
    compiler::{is_writable, CompileError, Compiler, EvalKind},
    operands::{DMString, Variable},
    Instruction,
};

pub(super) fn emit(
    compiler: &mut Compiler,
    unary: Vec<UnaryOp>,
    kind: EvalKind,
) -> Result<EvalKind, CompileError> {
    let mut kind = kind;
    for op in unary.into_iter().rev() {
        kind = emit_single(compiler, op, kind)?;
    }

    Ok(kind)
}

fn emit_single(
    compiler: &mut Compiler,
    op: UnaryOp,
    kind: EvalKind,
) -> Result<EvalKind, CompileError> {
    let mut return_kind = EvalKind::Stack;
    match op {
        UnaryOp::Neg | UnaryOp::Not | UnaryOp::BitNot => {
            compiler.emit_move_to_stack(kind)?;

            match op {
                UnaryOp::Neg => compiler.emit_ins(Instruction::UnaryNeg),
                UnaryOp::Not => compiler.emit_ins(Instruction::Not),
                UnaryOp::BitNot => compiler.emit_ins(Instruction::Bnot),
                _ => unreachable!(),
            }
        }

        UnaryOp::PreIncr
        | UnaryOp::PostIncr
        | UnaryOp::PreDecr
        | UnaryOp::PostDecr
        | UnaryOp::Reference
        | UnaryOp::Dereference => {
            let var = match kind {
                EvalKind::Stack if op == UnaryOp::Dereference => {
                    compiler.emit_ins(Instruction::SetVar(Variable::CacheKey));
                    Variable::CacheKey
                }

                EvalKind::Var(var) if is_writable(&var) => var,

                EvalKind::Var(var) if op == UnaryOp::Dereference => {
                    compiler.emit_ins(Instruction::GetVar(var));
                    compiler.emit_ins(Instruction::SetVar(Variable::CacheKey));
                    Variable::CacheKey
                }

                EvalKind::Field(builder, field) => builder.get_field(DMString(field.into())),

                EvalKind::ListRef => {
                    if op == UnaryOp::Dereference
                        && compiler
                            .byond
                            .at_least(super::versions::POINTER_INDEX_DROPPED)
                    {
                        compiler.warn_bug(
                            super::statements::POINTER_INDEX_BUG,
                            super::statements::POINTER_INDEX_OURS,
                        );
                    }
                    compiler.emit_ins(Instruction::SetVar(Variable::CacheKey));
                    compiler.emit_ins(Instruction::SetVar(Variable::Cache));
                    Variable::CacheIndex
                }

                _ => return Err(CompileError::ExpectedLValue),
            };

            match op {
                UnaryOp::PreIncr => compiler.emit_ins(Instruction::PreInc(var)),
                UnaryOp::PostIncr => compiler.emit_ins(Instruction::PostInc(var)),
                UnaryOp::PreDecr => compiler.emit_ins(Instruction::PreDec(var)),
                UnaryOp::PostDecr => compiler.emit_ins(Instruction::PostDec(var)),
                UnaryOp::Reference => return_kind = EvalKind::Var(reference(var)),
                UnaryOp::Dereference => {
                    return_kind = EvalKind::Var(Variable::PtrDeref(Box::new(var)))
                }
                _ => unreachable!(),
            }
        }
    }

    Ok(return_kind)
}

fn reference(var: Variable) -> Variable {
    match var {
        Variable::SetCache(holder, field) => {
            Variable::SetCache(holder, Box::new(reference(*field)))
        }
        var => Variable::PtrRef(Box::new(var)),
    }
}
