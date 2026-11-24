use std::collections::BTreeSet;

use dreammaker::ast::{Expression, Follow, PathOp, Prefab, Term};

use super::*;
use crate::operands::{Proc, Value};

#[derive(Debug, PartialEq)]
enum Named<'a> {
    NoProc(&'a str),
    Old(ProcKey),
    Added(ProcKey),
}

pub(super) struct Retargeted<'s> {
    old: &'s Snapshot,
    new: &'s Snapshot,
    realigned: &'s Realigned,
    paths: HashSet<String>,
    names: HashSet<String>,
    global_names: HashSet<String>,
    defaults: HashSet<String>,
    consts: HashSet<String>,
}

fn resolve(s: &Snapshot, path: &str) -> Option<ProcKey> {
    if let Some((ty, name)) = path.split_once("::") {
        return last_definition(s, ty, name);
    }
    if s.tree.find(path).is_some() {
        return None;
    }
    let (ty, name) = path.rsplit_once('/')?;
    let (ty, declared) = match ty.rsplit_once('/') {
        Some((ty, "proc" | "verb")) => (ty, true),
        _ => (ty, false),
    };
    let t = s.find_type(ty)?;
    let values = real(t, name);
    let declaration = declaration_index(t, name, &values);
    let index = match declared {
        true => declaration?,
        false => (0..values.len()).find(|&i| Some(i) != declaration)?,
    };
    Some(ProcKey {
        path: proc_path(t, name),
        override_id: index as u32,
    })
}

fn last_definition(s: &Snapshot, ty: &str, name: &str) -> Option<ProcKey> {
    let t = s.find_type(ty)?;
    let values = real(t, name);
    let declaration = declaration_index(t, name, &values);
    let verb = t
        .get()
        .procs
        .get(name)?
        .declaration
        .as_ref()
        .is_some_and(|d| d.kind == dreammaker::ast::ProcDeclKind::Verb);
    let index = (0..values.len())
        .rev()
        .find(|&i| !verb || Some(i) != declaration)?;
    Some(ProcKey {
        path: proc_path(t, name),
        override_id: index as u32,
    })
}

fn forms(ty: &str, name: &str) -> Vec<String> {
    let mut forms = vec![
        format!("{ty}/{name}"),
        format!("{ty}/proc/{name}"),
        format!("{ty}/verb/{name}"),
    ];
    if !ty.is_empty() {
        forms.push(format!("{ty}::{name}"));
    }
    forms
}

pub(super) fn retargeted<'s>(
    old: &'s Snapshot,
    new: &'s Snapshot,
    realigned: &'s Realigned,
    changes: &Changes,
) -> Retargeted<'s> {
    let mut r = Retargeted {
        old,
        new,
        realigned,
        paths: HashSet::new(),
        names: HashSet::new(),
        global_names: HashSet::new(),
        defaults: HashSet::new(),
        consts: HashSet::new(),
    };
    let touched: BTreeSet<&str> = realigned
        .keys()
        .map(String::as_str)
        .chain(
            changes
                .plan
                .removed_procs
                .iter()
                .map(|r| r.proc.path.as_str()),
        )
        .collect();
    for path in touched {
        let Some((ty, name)) = path.rsplit_once('/') else {
            continue;
        };
        for form in forms(ty, name) {
            if r.before(&form) != r.after(&form) {
                r.names.insert(name.to_owned());
                if ty.is_empty() {
                    r.global_names.insert(name.to_owned());
                }
                r.paths.insert(form);
            }
        }
    }
    if !r.paths.is_empty() {
        r.find_defaults();
    }
    r
}

impl Retargeted<'_> {
    pub(super) fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    fn before<'a>(&self, path: &'a str) -> Named<'a> {
        resolve(self.old, path).map_or(Named::NoProc(path), Named::Old)
    }

    fn after<'a>(&self, path: &'a str) -> Named<'a> {
        let Some(key) = resolve(self.new, path) else {
            return Named::NoProc(path);
        };
        match old_key(self.realigned, &key).filter(|old| self.old.procs.contains_key(old)) {
            Some(old) => Named::Old(old),
            None => Named::Added(key),
        }
    }

    fn find_defaults(&mut self) {
        loop {
            let mut found = vec![];
            for s in [self.old, self.new] {
                for t in s.tree.iter_types() {
                    for (name, var) in t.get().vars.iter() {
                        let Some(expression) = &var.value.expression else {
                            continue;
                        };
                        let is_const = t
                            .get_var_declaration(name.as_str())
                            .is_some_and(|d| d.get().var_type.flags.is_const());
                        let known = self.defaults.contains(name.as_str())
                            && (!is_const || self.consts.contains(name.as_str()));
                        if !known && self.expressions_might_name([expression]) {
                            found.push((name.as_str().to_owned(), is_const));
                        }
                    }
                }
            }
            if found.is_empty() {
                return;
            }
            for (name, is_const) in found {
                if is_const {
                    self.consts.insert(name.clone());
                }
                self.defaults.insert(name);
            }
        }
    }

    fn reaches(&self, ty: &str, name: &str) -> bool {
        if !self.names.contains(name) {
            return false;
        }
        for s in [self.old, self.new] {
            let Some(mut at) = s.find_type(ty) else {
                return true;
            };
            loop {
                let path = if at.is_root() { "" } else { &at.get().path };
                if forms(path, name)
                    .iter()
                    .any(|form| self.paths.contains(form))
                {
                    return true;
                }
                match at.parent_type() {
                    Some(parent) => at = parent,
                    None => break,
                }
            }
        }
        forms("", name).iter().any(|form| self.paths.contains(form))
    }

    fn prefab_might_name(&self, prefab: &Prefab) -> bool {
        let parts: Vec<(PathOp, &str)> = prefab
            .path
            .iter()
            .map(|(op, part)| (*op, part.as_str()))
            .collect();
        let Some(&(_, name)) = parts.last() else {
            return false;
        };
        if !prefab.vars.is_empty() || !self.names.contains(name) {
            return false;
        }
        if parts.iter().any(|(op, _)| *op != PathOp::Slash) {
            return true;
        }
        let joined = |parts: &[(PathOp, &str)]| -> String {
            parts.iter().map(|(_, part)| format!("/{part}")).collect()
        };
        match parts
            .iter()
            .position(|(_, part)| *part == "proc" || *part == "verb")
        {
            Some(at) => self.reaches(&joined(&parts[..at]), name),
            None => self.paths.contains(&joined(&parts)),
        }
    }

    fn proc_might_name(&self, value: &ProcValue, own: &str) -> bool {
        let mut scan = Scan::new(self, Some(own));
        for parameter in value.parameters.iter() {
            if let Some(default) = &parameter.default {
                syntax::walk_expression(default, &mut |node| scan.visit(node));
            }
        }
        if let Some(code) = &value.code {
            syntax::walk_block(code, &mut |node| scan.visit(node));
        }
        scan.found
    }

    fn expressions_might_name<'e>(
        &self,
        expressions: impl IntoIterator<Item = &'e Expression>,
    ) -> bool {
        let mut scan = Scan::new(self, None);
        for expression in expressions {
            syntax::walk_expression(expression, &mut |node| scan.visit(node));
        }
        scan.found
    }

    pub(super) fn inits_might_name(&self, inits: &[Init]) -> bool {
        !self.is_empty() && self.expressions_might_name(inits.iter().map(|init| &init.value))
    }

    pub(super) fn same_targets(&self, before: &ProcReload, after: &ProcReload) -> bool {
        let (mut old, mut new) = (vec![], vec![]);
        code_paths(before, &mut old);
        code_paths(after, &mut new);
        old.len() == new.len()
            && old
                .iter()
                .zip(&new)
                .all(|(old, new)| self.before(old) == self.after(new))
    }

    fn retargets(&self, before: Result<ProcReload, Diagnostic>, after: &ProcReload) -> bool {
        before.map_or(true, |before| !self.same_targets(&before, after))
    }
}

struct Scan<'r, 's> {
    r: &'r Retargeted<'s>,
    own: Option<&'r str>,
    resolved: HashSet<*const Follow>,
    found: bool,
}

impl<'r, 's> Scan<'r, 's> {
    fn new(r: &'r Retargeted<'s>, own: Option<&'r str>) -> Self {
        Scan {
            r,
            own,
            resolved: HashSet::new(),
            found: false,
        }
    }

    fn visit(&mut self, node: syntax::Node) {
        if self.found {
            return;
        }
        let r = self.r;
        self.found = match node {
            syntax::Node::Expression(Expression::Base { term, follow }) => {
                match (&term.elem, follow.first().map(|f| &f.elem)) {
                    (Term::Prefab(prefab), Some(first @ Follow::ProcReference(name)))
                        if prefab.vars.is_empty()
                            && prefab.path.iter().all(|(op, _)| *op == PathOp::Slash) =>
                    {
                        self.resolved.insert(first);
                        let ty: String = prefab.path.iter().map(|(_, p)| format!("/{p}")).collect();
                        r.reaches(&ty, name.as_str())
                    }
                    _ => false,
                }
            }
            syntax::Node::Follow(follow @ Follow::ProcReference(name)) => {
                !self.resolved.contains(&(follow as *const Follow))
                    && r.names.contains(name.as_str())
            }
            syntax::Node::Follow(Follow::StaticField(name)) => r.defaults.contains(name.as_str()),
            syntax::Node::Term(Term::Prefab(prefab)) => r.prefab_might_name(prefab),
            syntax::Node::Term(Term::__PROC__) => self.own.is_some_and(|own| r.names.contains(own)),
            syntax::Node::Name(name) => r.consts.contains(name) || r.global_names.contains(name),
            _ => false,
        };
    }
}

fn code_paths<'c>(code: &'c ProcReload, out: &mut Vec<&'c str>) {
    nodes_paths(&code.nodes, out);
    for init in code.statics.iter().filter_map(|s| s.init.as_deref()) {
        code_paths(init, out);
    }
}

fn nodes_paths<'c>(nodes: &'c [Node], out: &mut Vec<&'c str>) {
    let value = |value: &'c Value, out: &mut Vec<&'c str>| match value {
        Value::Path(path) => out.push(path),
        Value::Prefab { vars, .. } => vars.iter().for_each(|(_, nodes)| nodes_paths(nodes, out)),
        _ => {}
    };
    for node in nodes {
        match node {
            Node::Instruction(Instruction::PushVal(v), _) => value(v, out),
            Node::Instruction(Instruction::Switch(params), _) => {
                params.cases.iter().for_each(|(v, _)| value(v, out))
            }
            Node::Instruction(Instruction::SwitchRange(params), _) => {
                params.cases.iter().for_each(|(v, _)| value(v, out));
                for (low, high, _) in &params.range_cases {
                    value(low, out);
                    value(high, out);
                }
            }
            Node::Instruction(
                Instruction::CallGlob(_, Proc(path)) | Instruction::CallGlobalArgList(Proc(path)),
                _,
            ) => out.push(path),
            _ => {}
        }
    }
}

pub(super) fn path_users(r: &Retargeted, changes: &mut Changes) {
    if r.is_empty() {
        return;
    }
    let (old, new) = (r.old, r.new);
    let refuse = |changes: &mut Changes, key: &ProcKey, e: &Diagnostic| {
        let refusal = new.uncompiled(key, e);
        changes.refused.push(Refusal {
            why: format!(
                "names a proc whose definitions change, so it must be compiled again, but it {}",
                refusal.why
            ),
            ..refusal
        });
    };
    'procs: for (key, (_, name, _)) in new.procs.iter() {
        let Some(old_key) = old_key(r.realigned, key) else {
            continue;
        };
        let (Some((old_ty, old_value)), Some((new_ty, value))) =
            (old.find(&old_key), new.find(key))
        else {
            continue;
        };
        let reloaded = changes
            .plan
            .reloads
            .iter()
            .position(|reload| reload.proc == old_key);

        let mut lists = HashSet::new();
        for parameter in value.parameters.iter() {
            let Some(after) = &parameter.in_list else {
                continue;
            };
            let Some(before) = old_value
                .parameters
                .iter()
                .find(|p| p.name == parameter.name)
                .and_then(|p| p.in_list.as_ref())
                .filter(|before| *before == after)
            else {
                continue;
            };
            if !r.expressions_might_name([after]) {
                continue;
            }
            let after =
                match new.compile_in_list(new_ty, after, value_location(after, value.location)) {
                    Ok(after) => after,
                    Err(e) => {
                        let e = Diagnostic::error(
                            e.id,
                            format!("parameter {}'s `in` list: {e}", parameter.name),
                        );
                        refuse(changes, key, &e);
                        continue 'procs;
                    }
                };
            let before =
                old.compile_in_list(old_ty, before, value_location(before, old_value.location));
            if r.retargets(before, &after) {
                lists.insert(parameter.name.as_str());
            }
        }

        let body = if reloaded.is_none() && r.proc_might_name(value, name) {
            match new.compile_code(key) {
                Ok(after) => r
                    .retargets(old.compile_code(&old_key), &after)
                    .then_some(after),
                Err(e) => {
                    refuse(changes, key, &e);
                    continue;
                }
            }
        } else {
            None
        };
        if body.is_none() && lists.is_empty() {
            continue;
        }
        let in_lists = |name: &str, _: &Expression| lists.contains(name);
        let reload = match reloaded {
            Some(at) => new
                .compile_in_lists(key, &mut changes.plan.reloads[at], in_lists)
                .map(|()| None),
            None => body
                .map_or_else(|| new.compile_code(key), Ok)
                .and_then(|mut reload| {
                    new.compile_in_lists(key, &mut reload, in_lists)?;
                    Ok(Some(reload))
                }),
        };
        match reload {
            Ok(Some(reload)) => {
                changes.plan.line_moves.retain(|m| m.proc != old_key);
                changes.plan.reloads.push(ProcReload {
                    proc: old_key,
                    ..reload
                });
            }
            Ok(None) => {}
            Err(e) => refuse(changes, key, &e),
        }
    }
}

pub(super) fn path_defaults(r: &Retargeted, changes: &mut Changes) {
    if r.is_empty() {
        return;
    }
    let new = r.new;
    for n in new.tree.iter_types() {
        if n.is_root() {
            continue;
        }
        let path = &n.get().path;
        for (name, var) in n.get().vars.iter() {
            let Some(declaration) = n.get_var_declaration(name.as_str()) else {
                continue;
            };
            let flags = declaration.get().var_type.flags;
            if var.value.location.file == FileId::BUILTINS
                || declaration.get().location.file == FileId::BUILTINS
                || flags.is_static()
                || flags.is_const()
            {
                continue;
            }
            let Some(ConstValue::Path(value)) = var_constant(var) else {
                continue;
            };
            if !r.paths.contains(&value) {
                continue;
            }
            if changes
                .plan
                .var_defaults
                .iter()
                .any(|d| d.var == name.as_str() && d.types.contains(path))
            {
                continue;
            }
            changes.plan.var_defaults.push(VarDefault {
                var: name.as_str().to_owned(),
                value: ConstValue::Path(value),
                types: default_types(new, n, name.as_str()),
            });
        }
    }
}
