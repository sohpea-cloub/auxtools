use dreammaker::ast::{Case, Expression, Spanned, Statement, VarStatement};

pub struct Declared<'a> {
    pub name: &'a str,
    pub nth: u32,
    place: Vec<Step<'a>>,
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
    let mut out = vec![];
    declared_in(block, &mut vec![], &mut out);
    out
}

fn declared_in<'a>(
    block: &'a [Spanned<Statement>],
    place: &mut Vec<Step<'a>>,
    out: &mut Vec<Declared<'a>>,
) {
    for statement in block {
        declared_by(&statement.elem, statement.location.line, place, out);
    }
}

fn nested<'a>(
    step: Step<'a>,
    block: &'a [Spanned<Statement>],
    place: &mut Vec<Step<'a>>,
    out: &mut Vec<Declared<'a>>,
) {
    place.push(step);
    declared_in(block, place, out);
    place.pop();
}

fn declared_by<'a>(
    statement: &'a Statement,
    line: u32,
    place: &mut Vec<Step<'a>>,
    out: &mut Vec<Declared<'a>>,
) {
    let mut var = |var: &'a VarStatement, place: &[Step<'a>]| {
        if var.var_type.flags.is_static() || var.var_type.flags.is_const() {
            let name = var.name.as_str();
            out.push(Declared {
                name,
                nth: out.iter().filter(|had| had.name == name).count() as u32,
                place: place.to_vec(),
                line,
            });
        }
    };
    match statement {
        Statement::Var(one) => var(one, place),
        Statement::Vars(all) => all.iter().for_each(|one| var(one, place)),
        Statement::While { condition, block } => {
            nested(Step::While(&condition.elem), block, place, out)
        }
        Statement::DoWhile { block, condition } => {
            nested(Step::DoWhile(&condition.elem), block, place, out)
        }
        Statement::ForInfinite { block } => nested(Step::ForInfinite, block, place, out),
        Statement::Spawn { delay, block } => nested(Step::Spawn(delay.as_ref()), block, place, out),
        Statement::Label { name, block } => nested(Step::Label(name.as_str()), block, place, out),
        Statement::ForList(each) => nested(
            Step::ForList(each.name.as_str(), each.in_list.as_ref()),
            &each.block,
            place,
            out,
        ),
        Statement::ForKeyValue(each) => nested(
            Step::ForKeyValue(
                each.key.as_str(),
                each.value.as_str(),
                each.in_list.as_ref(),
            ),
            &each.block,
            place,
            out,
        ),
        Statement::ForRange(each) => nested(
            Step::ForRange(
                each.name.as_str(),
                &each.start,
                &each.end,
                each.step.as_ref(),
            ),
            &each.block,
            place,
            out,
        ),
        Statement::ForLoop {
            init,
            test,
            inc,
            block,
        } => {
            place.push(Step::ForLoop(
                init.as_deref(),
                test.as_deref(),
                inc.as_deref(),
            ));
            for part in init.iter().chain(inc) {
                declared_by(part, line, place, out);
            }
            declared_in(block, place, out);
            place.pop();
        }
        Statement::If { arms, else_arm } => {
            for (condition, block) in arms {
                nested(Step::If(&condition.elem), block, place, out);
            }
            if let Some(block) = else_arm {
                nested(
                    Step::Else(arms.iter().map(|(condition, _)| &condition.elem).collect()),
                    block,
                    place,
                    out,
                );
            }
        }
        Statement::Switch {
            input,
            cases,
            default,
        } => {
            for (values, block) in cases.iter() {
                nested(Step::Case(input, &values.elem), block, place, out);
            }
            if let Some(block) = default {
                nested(Step::Default(input), block, place, out);
            }
        }
        Statement::TryCatch {
            try_block,
            catch_block,
            ..
        } => {
            nested(Step::Try, try_block, place, out);
            nested(Step::Catch, catch_block, place, out);
        }
        _ => {}
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
