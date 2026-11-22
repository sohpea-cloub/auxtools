use dreammaker::ast::{Block, Statement};

use super::{procs::ProcSource, Compiler};
use crate::diagnostic::Id;

pub(super) fn check(compiler: &Compiler, source: &ProcSource, code: &Block) {
    let line = source.value.location.line;
    if let Some((id, text)) = proc_warning(compiler, source) {
        compiler.warn_at(id, line, &text);
    }
    let mut walk = Walk {
        compiler,
        loops: vec![],
    };
    walk.block(code, None);
}

fn proc_warning(compiler: &Compiler, source: &ProcSource) -> Option<(Id, String)> {
    let byond = compiler.byond;
    let name = source.name;
    let found = match name {
        "operator<<=" | "operator>>=" => (
            Id::ByondShiftAssignDiscarded,
            format!(
                "`{name}`: BYOND discards the result, leaving the variable unchanged; \
             modify src instead"
            ),
        ),
        "operator++" | "operator--" => (
            Id::ByondIncrementOperator,
            format!(
            "`{name}`: statements `a{op}` and `{op}a` pass no argument and discard the result; \
             in `b = a{op}`, `a{op}` passes 1 and assigns the result to `a`",
            op = &name["operator".len()..]
            ),
        ),
        "operator**" if source.ty.get_proc("operator*").is_some() => (
            Id::ByondPowerCallsMultiply,
            "`operator**`: `a ** 2` calls `operator*(a)` when both overloads exist".to_owned(),
        ),
        "operator<=>" if byond.find_calls_compare_operator() => (
            Id::ByondCompareOperatorInFind,
            "`operator<=>`: from 516.1674 `list.Find(x)` and `list ^ x` call it for each item, \
             and from 516.1689 treat a result of 0 as a match"
                .to_owned(),
        ),
        "operator~=" | "operator~!" if byond.discards_equivalence_result() => (
            Id::ByondEquivalenceDiscarded,
            format!(
                "`{name}`: 516.1674–516.1688 discard the result and call the opposite \
                 equivalence overload when both exist"
            ),
        ),
        "Uncross" if byond.area_exit_calls_uncross() && source.ty.get().is_subpath_of("/area") => (
            Id::ByondAreaUncross,
            "`Uncross()` of an area: from 516.1683 BYOND calls it and ignores the result; \
             return 0 from `Exit()` to stop a move"
                .to_owned(),
        ),
        _ => return None,
    };
    Some(found)
}

#[derive(Clone, Copy, PartialEq)]
enum LoopKind {
    Counted,
    List,
    DoWhile,
    Other,
}

struct Loop {
    kind: LoopKind,
    label: Option<String>,
    try_inside: bool,
}

struct Walk<'c, 'a> {
    compiler: &'c Compiler<'a>,
    loops: Vec<Loop>,
}

impl Walk<'_, '_> {
    fn block(&mut self, block: &Block, label: Option<&str>) {
        let mut label = label.map(str::to_owned);
        for stmt in block.iter() {
            let line = stmt.location.line;
            match &stmt.elem {
                Statement::Label { name, block } => self.block(block, Some(name.as_str())),
                Statement::ForRange(f) => {
                    self.looped(LoopKind::Counted, label.take(), &f.block, line)
                }
                Statement::ForList(f) => self.looped(LoopKind::List, label.take(), &f.block, line),
                Statement::ForKeyValue(f) => {
                    self.looped(LoopKind::List, label.take(), &f.block, line)
                }
                Statement::DoWhile { block, .. } => {
                    self.looped(LoopKind::DoWhile, label.take(), block, line)
                }
                Statement::While { block, .. }
                | Statement::ForInfinite { block }
                | Statement::ForLoop { block, .. } => {
                    self.looped(LoopKind::Other, label.take(), block, line)
                }
                Statement::If { arms, else_arm } => {
                    for (_, block) in arms.iter() {
                        self.block(block, None);
                    }
                    if let Some(block) = else_arm {
                        self.block(block, None);
                    }
                }
                Statement::Switch { cases, default, .. } => {
                    for (_, block) in cases.iter() {
                        self.block(block, None);
                    }
                    if let Some(block) = default {
                        self.block(block, None);
                    }
                }
                Statement::TryCatch {
                    try_block,
                    catch_block,
                    ..
                } => {
                    let outer = self.loops.last_mut().map(|l| {
                        let was = l.try_inside;
                        l.try_inside = true;
                        was
                    });
                    self.block(try_block, None);
                    if let (Some(was), Some(l)) = (outer, self.loops.last_mut()) {
                        l.try_inside = was;
                    }
                    self.block(catch_block, None);
                }
                Statement::Spawn { block, .. } => {
                    let loops = std::mem::take(&mut self.loops);
                    self.block(block, None);
                    self.loops = loops;
                }
                Statement::Continue(Some(name)) => self.continue_to(name.as_str(), line),
                _ => {}
            }
        }
    }

    fn looped(&mut self, kind: LoopKind, label: Option<String>, block: &Block, line: u32) {
        if kind == LoopKind::List {
            let caught_outside = self
                .loops
                .iter()
                .rev()
                .scan(false, |seen_try, l| {
                    *seen_try |= l.try_inside;
                    Some((l.kind, *seen_try))
                })
                .any(|(kind, seen_try)| kind == LoopKind::List && seen_try);
            if caught_outside {
                self.compiler.warn_at(
                    Id::ByondListLoopsAcrossTry,
                    line,
                    "nested list loops separated by `try`: catching an inner-loop error makes \
                     the outer loop use the inner iterator; it may end early, loop forever or \
                     crash the server. Use a counted loop or catch inside the inner loop",
                );
            }
        }
        self.loops.push(Loop {
            kind,
            label,
            try_inside: false,
        });
        self.block(block, None);
        self.loops.pop();
    }

    fn continue_to(&mut self, name: &str, line: u32) {
        let Some(at) = self
            .loops
            .iter()
            .rposition(|l| l.label.as_deref() == Some(name))
        else {
            return;
        };
        if self.loops[at].kind == LoopKind::DoWhile
            && self.loops[..at].iter().any(|l| l.kind == LoopKind::Counted)
        {
            self.compiler.warn_at(
                Id::ByondContinueToDoWhile,
                line,
                &format!(
                    "`continue {name}` to a labelled `do` ... `while` inside a counted `for` \
                     loop crashes the server; use a plain `continue`, or a `while` loop"
                ),
            );
        }
    }
}
