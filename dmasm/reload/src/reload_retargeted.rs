use dreammaker::ast::{Follow, Term};

use super::*;
use crate::operands::Value;

pub(super) fn retargeted(
    old: &Snapshot,
    new: &Snapshot,
    realigned: &Realigned,
    changes: &Changes,
) -> HashSet<String> {
    let mut paths = HashSet::new();
    for (path, map) in realigned {
        let Some((ty, name)) = path.rsplit_once('/') else {
            continue;
        };
        let (Some(o), Some(n)) = (old.find_type(ty), new.find_type(ty)) else {
            continue;
        };
        let (old_values, new_values) = (real(o, name), real(n, name));
        let old_declaration = declaration_index(o, name, &old_values);
        let new_declaration = declaration_index(n, name, &new_values);
        let first_override =
            |count: usize, declaration: Option<usize>| (0..count).find(|&i| Some(i) != declaration);
        let old_index = |i: Option<usize>| i.and_then(|i| map.get(i).copied().flatten());
        let old_first = first_override(old_values.len(), old_declaration).map(|i| i as u32);
        if old_index(first_override(new_values.len(), new_declaration)) != old_first {
            paths.insert(path.clone());
        }
        if old_index(new_declaration) != old_declaration.map(|i| i as u32) {
            declaration_forms(ty, name, &mut paths);
        }
    }
    for removed in &changes.plan.removed_procs {
        if !realigned.contains_key(&removed.proc.path) {
            paths.insert(removed.proc.path.clone());
            declaration_forms(&removed.type_path, &removed.name, &mut paths);
        }
    }
    paths
}

fn declaration_forms(ty: &str, name: &str, paths: &mut HashSet<String>) {
    paths.insert(format!("{ty}/proc/{name}"));
    paths.insert(format!("{ty}/verb/{name}"));
}

fn names(retargeted: &HashSet<String>) -> HashSet<&str> {
    retargeted
        .iter()
        .filter_map(|path| path.rsplit('/').next())
        .collect()
}

fn mentions(node: syntax::Node, names: &HashSet<&str>) -> bool {
    match node {
        syntax::Node::Term(Term::Prefab(prefab)) => prefab
            .path
            .iter()
            .last()
            .is_some_and(|(_, last)| names.contains(last.as_str())),
        syntax::Node::Follow(Follow::ProcReference(name)) => names.contains(name.as_str()),
        _ => false,
    }
}

fn names_in(nodes: &[Node], retargeted: &HashSet<String>) -> bool {
    let named = |value: &Value| matches!(value, Value::Path(path) if retargeted.contains(path));
    nodes.iter().any(|node| match node {
        Node::Instruction(Instruction::PushVal(value), _) => named(value),
        Node::Instruction(Instruction::Switch(params), _) => {
            params.cases.iter().any(|(value, _)| named(value))
        }
        Node::Instruction(Instruction::SwitchRange(params), _) => {
            params.cases.iter().any(|(value, _)| named(value))
                || params
                    .range_cases
                    .iter()
                    .any(|(low, high, _)| named(low) || named(high))
        }
        _ => false,
    })
}

pub(super) fn code_names(code: &ProcReload, retargeted: &HashSet<String>) -> bool {
    names_in(&code.nodes, retargeted)
        || code
            .statics
            .iter()
            .filter_map(|s| s.init.as_deref())
            .any(|init| code_names(init, retargeted))
        || code.params.iter().any(|param| match &param.in_list {
            InList::New(init) => code_names(init, retargeted),
            _ => false,
        })
}

pub(super) fn path_users(
    old: &Snapshot,
    new: &Snapshot,
    realigned: &Realigned,
    retargeted: &HashSet<String>,
    changes: &mut Changes,
) {
    if retargeted.is_empty() {
        return;
    }
    let names = names(retargeted);
    let reloaded: HashSet<ProcKey> = changes
        .plan
        .reloads
        .iter()
        .map(|r| r.proc.clone())
        .collect();
    for key in new.procs.keys() {
        let Some(old_key) = old_key(realigned, key) else {
            continue;
        };
        if reloaded.contains(&old_key) || old.find(&old_key).is_none() {
            continue;
        }
        let Some((_, value)) = new.find(key) else {
            continue;
        };
        let mut found = false;
        syntax::walk_proc(value, &mut |node| found |= mentions(node, &names));
        if !found {
            continue;
        }
        let reload = new.compile_code(key).and_then(|mut reload| {
            new.compile_in_lists(key, &mut reload, |_, _| false)?;
            Ok(reload)
        });
        match reload {
            Ok(reload) if code_names(&reload, retargeted) => {
                changes.plan.line_moves.retain(|m| m.proc != old_key);
                changes.plan.reloads.push(ProcReload {
                    proc: old_key,
                    ..reload
                });
            }
            Ok(_) => {}
            Err(e) => {
                let refusal = new.uncompiled(key, &e);
                changes.refused.push(Refusal {
                    why: format!(
                        "names a proc whose definitions change, so it must be compiled again, but it {}",
                        refusal.why
                    ),
                    ..refusal
                });
            }
        }
    }
}

pub(super) fn inits_mention(inits: &[Init], retargeted: &HashSet<String>) -> bool {
    if retargeted.is_empty() {
        return false;
    }
    let names = names(retargeted);
    let mut found = false;
    for init in inits {
        syntax::walk_expression(&init.value, &mut |node| found |= mentions(node, &names));
    }
    found
}

pub(super) fn path_defaults(new: &Snapshot, retargeted: &HashSet<String>, changes: &mut Changes) {
    if retargeted.is_empty() {
        return;
    }
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
            if !retargeted.contains(&value) {
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
