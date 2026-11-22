use dreammaker::objtree::{TypeProc, TypeRef, TypeVar};

use super::*;

fn storage(flags: dreammaker::ast::VarTypeFlags) -> (bool, bool, bool) {
    (flags.is_static(), flags.is_const(), flags.is_tmp())
}

pub(super) fn var_moved_up(
    n: TypeRef,
    name: &str,
    old_var: &TypeVar,
    var: Option<&TypeVar>,
) -> bool {
    old_var.declaration.is_some()
        && var.is_none_or(|v| v.declaration.is_none())
        && n.parent_type()
            .and_then(|p| p.get_var_declaration(name))
            .is_some_and(|d| d.get().location.file != FileId::BUILTINS)
}

pub(super) fn moved_from(old: &Snapshot, n: TypeRef, name: &str) -> Vec<String> {
    let below = |t: TypeRef| {
        let mut at = t.parent_type();
        while let Some(p) = at {
            if p == n {
                return true;
            }
            at = p.parent_type();
        }
        false
    };
    n.tree()
        .iter_types()
        .filter(|t| below(*t))
        .filter_map(|t| {
            let old_var = old.find_type(&t.get().path)?.get().vars.get(name)?;
            var_moved_up(t, name, old_var, t.get().vars.get(name)).then(|| t.get().path.clone())
        })
        .collect()
}

pub(super) fn var_moved(
    old: &Snapshot,
    new: &Snapshot,
    n: TypeRef,
    name: &str,
    old_var: &TypeVar,
    var: Option<&TypeVar>,
    changes: &mut Changes,
) {
    let path = n.get().path.clone();
    let Some(declaration) = n.parent_type().and_then(|p| p.get_var_declaration(name)) else {
        return;
    };
    let ancestor = declaration.ty().get().path.clone();
    let (old_flags, flags) = (
        old_var
            .declaration
            .as_ref()
            .map(|d| d.var_type.flags)
            .unwrap_or_default(),
        declaration.get().var_type.flags,
    );
    let (old_storage, new_storage) = (storage(old_flags), storage(flags));
    if new_storage.0 || new_storage.1 {
        let why = format!(
            "moved to {ancestor} as a static or const var; global slot conflicts with {path}'s"
        );
        changes.refused.push(
            Refusal::new(path, why)
                .named(name)
                .at(new, declaration.get().location),
        );
        return;
    }
    if old_storage != new_storage {
        if let Some(ancestor_var) = declaration.ty().get().vars.get(name) {
            redeclare(new, n, name, old_var, ancestor_var, changes);
        }
    }
    let _ = var;
    var_default(old, new, &path, name, false, changes);
}

pub(super) fn proc_moved_up(
    n: TypeRef,
    name: &str,
    old_proc: Option<&TypeProc>,
    proc: &TypeProc,
) -> bool {
    let Some(old_declaration) = old_proc.and_then(|p| p.declaration.as_ref()) else {
        return false;
    };
    proc.declaration.is_none()
        && n.parent_type()
            .and_then(|p| p.get_proc_declaration(name))
            .is_some_and(|d| d.kind == old_declaration.kind && d.flags == old_declaration.flags)
}

pub(super) fn moved_users(
    old: &Snapshot,
    new: &Snapshot,
    realigned: &Realigned,
    changes: &mut Changes,
) {
    if changes.moved_procs.is_empty() {
        return;
    }
    let reloaded: std::collections::HashSet<ProcKey> = changes
        .plan
        .reloads
        .iter()
        .map(|r| r.proc.clone())
        .collect();
    let mut reloads = vec![];
    let moved = changes.moved_procs.clone();
    for key in new.procs.keys().filter(|key| moved.contains(&key.path)) {
        let Some(old_key) = old_key(realigned, key) else {
            continue;
        };
        if reloaded.contains(&old_key) {
            continue;
        }
        let (Ok(before), Ok(after)) = (old.compile(&old_key), new.compile(key)) else {
            continue;
        };
        if before.nodes == after.nodes {
            continue;
        }
        match new.compile_code(key).and_then(|mut reload| {
            new.compile_in_lists(key, &mut reload, |_, _| false)?;
            Ok(reload)
        }) {
            Ok(reload) => reloads.push(ProcReload {
                proc: old_key,
                ..reload
            }),
            Err(e) => changes.refused.push(new.uncompiled(key, &e)),
        }
    }
    changes
        .plan
        .line_moves
        .retain(|m| !reloads.iter().any(|r| r.proc == m.proc));
    changes.plan.reloads.extend(reloads);
}
