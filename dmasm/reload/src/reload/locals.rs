use dreammaker::ast::{Case, Expression, Spanned, Statement, VarStatement};

pub struct Declared<'a> {
    pub name: &'a str,
    pub nth: u32,
    place: Vec<Step<'a>>,
    blocks: Vec<u32>,
    pub line: u32,
}

#[derive(Clone, PartialEq)]
enum Step<'a> {
    If(&'a Expression),
    Else(Vec<&'a Expression>),
    While(&'a Expression),
    DoWhile(&'a Expression),
    ForInfinite,
    ForLoop(
        Option<&'a Statement>,
        Option<&'a Expression>,
        Option<&'a Statement>,
    ),
    ForList(&'a str, Option<&'a Expression>),
    ForKeyValue(&'a str, &'a str, Option<&'a Expression>),
    ForRange(
        &'a str,
        &'a Expression,
        &'a Expression,
        Option<&'a Expression>,
    ),
    Spawn(Option<&'a Expression>),
    Case(&'a Expression, &'a [Case]),
    Default(&'a Expression),
    Try,
    Catch,
    Label(&'a str),
}

pub fn declared(block: &[Spanned<Statement>]) -> Vec<Declared<'_>> {
    let mut walk = Walk::default();
    walk.block(block);
    walk.out
}

pub fn visible_at(declared: &[Declared], at: usize) -> Vec<(String, u32)> {
    let here = &declared[at].blocks;
    declared[..at]
        .iter()
        .filter(|local| here.starts_with(&local.blocks))
        .map(|local| (local.name.to_owned(), local.nth))
        .collect()
}

#[derive(Default)]
struct Walk<'a> {
    place: Vec<Step<'a>>,
    blocks: Vec<u32>,
    opened: u32,
    out: Vec<Declared<'a>>,
}

impl<'a> Walk<'a> {
    fn block(&mut self, block: &'a [Spanned<Statement>]) {
        for statement in block {
            self.statement(&statement.elem, statement.location.line);
        }
    }

    fn open(&mut self) {
        self.blocks.push(self.opened);
        self.opened += 1;
    }

    fn nested(&mut self, step: Step<'a>, block: &'a [Spanned<Statement>]) {
        self.place.push(step);
        self.open();
        self.block(block);
        self.blocks.pop();
        self.place.pop();
    }

    fn var(&mut self, var: &'a VarStatement, line: u32) {
        if var.var_type.flags.is_static() || var.var_type.flags.is_const() {
            let name = var.name.as_str();
            self.out.push(Declared {
                name,
                nth: self.out.iter().filter(|had| had.name == name).count() as u32,
                place: self.place.clone(),
                blocks: self.blocks.clone(),
                line,
            });
        }
    }

    fn statement(&mut self, statement: &'a Statement, line: u32) {
        match statement {
            Statement::Var(one) => self.var(one, line),
            Statement::Vars(all) => all.iter().for_each(|one| self.var(one, line)),
            Statement::While { condition, block } => {
                self.nested(Step::While(&condition.elem), block)
            }
            Statement::DoWhile { block, condition } => {
                self.nested(Step::DoWhile(&condition.elem), block)
            }
            Statement::ForInfinite { block } => self.nested(Step::ForInfinite, block),
            Statement::Spawn { delay, block } => self.nested(Step::Spawn(delay.as_ref()), block),
            Statement::Label { name, block } => self.nested(Step::Label(name.as_str()), block),
            Statement::ForList(each) => self.nested(
                Step::ForList(each.name.as_str(), each.in_list.as_ref()),
                &each.block,
            ),
            Statement::ForKeyValue(each) => self.nested(
                Step::ForKeyValue(
                    each.key.as_str(),
                    each.value.as_str(),
                    each.in_list.as_ref(),
                ),
                &each.block,
            ),
            Statement::ForRange(each) => self.nested(
                Step::ForRange(
                    each.name.as_str(),
                    &each.start,
                    &each.end,
                    each.step.as_ref(),
                ),
                &each.block,
            ),
            Statement::ForLoop {
                init,
                test,
                inc,
                block,
            } => {
                self.place.push(Step::ForLoop(
                    init.as_deref(),
                    test.as_deref(),
                    inc.as_deref(),
                ));
                self.open();
                for part in init.iter().chain(inc) {
                    self.statement(part, line);
                }
                self.open();
                self.block(block);
                self.blocks.pop();
                self.blocks.pop();
                self.place.pop();
            }
            Statement::If { arms, else_arm } => {
                for (condition, block) in arms {
                    self.nested(Step::If(&condition.elem), block);
                }
                if let Some(block) = else_arm {
                    self.nested(
                        Step::Else(arms.iter().map(|(condition, _)| &condition.elem).collect()),
                        block,
                    );
                }
            }
            Statement::Switch {
                input,
                cases,
                default,
            } => {
                for (values, block) in cases.iter() {
                    self.nested(Step::Case(input, &values.elem), block);
                }
                if let Some(block) = default {
                    self.nested(Step::Default(input), block);
                }
            }
            Statement::TryCatch {
                try_block,
                catch_block,
                ..
            } => {
                self.nested(Step::Try, try_block);
                self.nested(Step::Catch, catch_block);
            }
            _ => {}
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Unmatched {
    pub name: String,
    pub line: u32,
    pub old: usize,
    pub new: usize,
}

pub fn matched<'a>(
    old: &[Declared<'a>],
    new: &[Declared<'a>],
) -> (Vec<(&'a str, u32, u32)>, Vec<Unmatched>) {
    let mut names: Vec<&str> = new.iter().map(|local| local.name).collect();
    names.sort_unstable();
    names.dedup();
    let mut pairs = vec![];
    let mut unmatched = vec![];
    for name in names {
        let mut olds: Vec<&Declared> = old.iter().filter(|local| local.name == name).collect();
        let mut news: Vec<&Declared> = new.iter().filter(|local| local.name == name).collect();
        let mut at = 0;
        while let Some(first) = news.get(at) {
            let place = first.place.clone();
            let new_here: Vec<u32> = news
                .iter()
                .filter(|local| local.place == place)
                .map(|local| local.nth)
                .collect();
            let old_here: Vec<u32> = olds
                .iter()
                .filter(|local| local.place == place)
                .map(|local| local.nth)
                .collect();
            if new_here.len() != old_here.len() || old_here.is_empty() {
                at += 1;
                continue;
            }
            pairs.extend(
                new_here
                    .into_iter()
                    .zip(old_here)
                    .map(|(new, old)| (name, new, old)),
            );
            news.retain(|local| local.place != place);
            olds.retain(|local| local.place != place);
        }
        let kinds = |local: &Declared<'a>| -> Vec<std::mem::Discriminant<Step<'a>>> {
            local.place.iter().map(std::mem::discriminant).collect()
        };
        while let Some(first) = news.first() {
            let these = kinds(first);
            let new_here: Vec<&Declared> = news
                .iter()
                .copied()
                .filter(|local| kinds(local) == these)
                .collect();
            let old_here: Vec<&Declared> = olds
                .iter()
                .copied()
                .filter(|local| kinds(local) == these)
                .collect();
            match (&old_here[..], &new_here[..]) {
                ([], _) => {}
                ([old], [new]) => pairs.push((name, new.nth, old.nth)),
                _ => unmatched.push(Unmatched {
                    name: name.to_owned(),
                    line: new_here[0].line,
                    old: old_here.len(),
                    new: new_here.len(),
                }),
            }
            news.retain(|local| kinds(local) != these);
            olds.retain(|local| kinds(local) != these);
        }
    }
    (pairs, unmatched)
}
