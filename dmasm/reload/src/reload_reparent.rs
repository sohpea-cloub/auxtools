use std::collections::HashSet;

use dreammaker::objtree::TypeRef;
use dreammaker::FileId;

use super::*;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Reparented {
    pub type_path: String,
    pub parent: String,
    pub types: Vec<ReparentedType>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReparentedType {
    pub path: String,
    pub declares: Vec<String>,
    pub sets: Vec<String>,
    pub builtins: Vec<String>,
    pub name: ConstValue,
    pub text: ConstValue,
}

pub(super) fn builtin_base(t: TypeRef) -> Option<String> {
    static BUILTIN_TYPES: std::sync::LazyLock<HashSet<String>> = std::sync::LazyLock::new(|| {
        dreammaker::objtree::ObjectTree::with_builtins()
            .iter_types()
            .filter(|t| !t.is_root())
            .map(|t| t.get().path.clone())
            .collect()
    });
    let mut at = Some(t);
    while let Some(t) = at {
        if BUILTIN_TYPES.contains(&t.get().path) {
            return Some(t.get().path.clone());
        }
        at = t.parent_type();
    }
    None
}

fn descends_from(t: TypeRef, ancestor: TypeRef) -> bool {
    let mut at = Some(t);
    while let Some(t) = at {
        if t == ancestor {
            return true;
        }
        at = t.parent_type();
    }
    false
}

fn record_name(t: TypeRef) -> (ConstValue, ConstValue) {
    let set = |name: &str| {
        let mut at = Some(t);
        while let Some(ty) = at {
            if let Some(var) = ty
                .get()
                .vars
                .get(name)
                .filter(|v| v.value.location.file != FileId::BUILTINS)
            {
                return Some(var_constant(var).unwrap_or(ConstValue::Null));
            }
            at = ty.parent_type();
        }
        None
    };
    let name = set("name").unwrap_or_else(|| {
        ConstValue::String(
            t.get()
                .path
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .replace('_', " "),
        )
    });
    let text = set("text").unwrap_or_else(|| match &name {
        ConstValue::String(name) => ConstValue::String(name.chars().take(1).collect()),
        _ => ConstValue::Null,
    });
    (name, text)
}

pub(super) fn reparent(
    old: &Snapshot,
    new: &Snapshot,
    o: TypeRef,
    n: TypeRef,
    changes: &mut Changes,
) {
    let path = n.get().path.clone();
    let at = n
        .get()
        .vars
        .get("parent_type")
        .map(|v| v.value.location)
        .filter(|l| new.source_line(*l).is_some())
        .unwrap_or_else(|| new.type_location(n));
    let Some(parent) = n.parent_type().filter(|p| !p.is_root()) else {
        changes.refused.push(
            Refusal::new(
                Id::ReparentedToRoot,
                path,
                "cannot change a parent type to the root",
            )
            .at(new, at),
        );
        return;
    };
    let parent_path = parent.get().path.clone();
    let refusal = |why: String| {
        Refusal::new(
            Id::ParentChanged,
            path.clone(),
            format!("parent type changed: {why}"),
        )
        .at(new, at)
    };
    let new_parent = changes.plan.new_types.iter().any(|t| t.path == parent_path);
    let mut refuse = |why: String| changes.refused.push(refusal(why));
    if builtin_base(n).as_deref() == Some(path.as_str()) {
        return refuse("built-in types cannot change parents".to_owned());
    }
    if builtin_base(o) != builtin_base(n) {
        return refuse(format!(
            "its instances would be a {} instead of a {}",
            builtin_base(n).unwrap_or_default(),
            builtin_base(o).unwrap_or_default()
        ));
    }
    if descends_from(parent, n) {
        return refuse(format!("parent {parent_path} is a subtype of this type"));
    }
    if old.find_type(&parent_path).is_none() && !new_parent {
        return refuse(format!("parent {parent_path} does not exist"));
    }

    let mut subtree: Vec<TypeRef> = n
        .tree()
        .iter_types()
        .filter(|t| descends_from(*t, n))
        .collect();
    let depth = |t: &TypeRef| {
        let mut depth = 0;
        let mut at = t.parent_type();
        while let Some(p) = at {
            depth += 1;
            at = p.parent_type();
        }
        depth
    };
    subtree.sort_by_key(depth);

    let mut errors = vec![];
    let mut types = vec![];
    for t in &subtree {
        let mut declares = vec![];
        let mut sets = vec![];
        let mut builtins = vec![];
        for (name, var) in t.get().vars.iter() {
            if var.value.location.file == FileId::BUILTINS || name.as_str() == "parent_type" {
                continue;
            }
            if var.declaration.is_some() {
                if let Some(above) = parent.get_var_declaration(name.as_str()) {
                    errors.push(format!(
                        "{} declares var {name}, which {} declares too",
                        t.get().path,
                        above.ty().get().path
                    ));
                }
                declares.push(name.to_string());
            } else if is_builtin(*t, name.as_str()) {
                builtins.push(name.to_string());
            } else if t.get_var_declaration(name.as_str()).is_none() {
                errors.push(format!(
                    "{}: var {name} is no longer declared",
                    t.get().path
                ));
            } else {
                sets.push(name.to_string());
            }
        }
        for (name, proc) in t.get().procs.iter() {
            if real(proc).is_empty() {
                continue;
            }
            if proc.declaration.is_some() {
                if parent.get_proc_declaration(name.as_str()).is_some() {
                    errors.push(format!(
                        "{} declares proc {name}, which its new ancestors declare too",
                        t.get().path
                    ));
                }
            } else if t.get_proc_declaration(name.as_str()).is_none() {
                errors.push(format!(
                    "{}: proc {name} is no longer declared",
                    t.get().path
                ));
            }
        }
        let (name, text) = record_name(*t);
        types.push(ReparentedType {
            path: t.get().path.clone(),
            declares,
            sets,
            builtins,
            name,
            text,
        });
    }
    if !errors.is_empty() {
        changes.refused.push(refusal(errors.join("; ")));
        return;
    }
    changes.plan.reparented.push(Reparented {
        type_path: path,
        parent: parent_path,
        types,
    });
}

fn inherited_changes(
    old: &Snapshot,
    new: &Snapshot,
    types: &HashSet<&str>,
) -> (HashSet<String>, HashSet<String>) {
    let mut declared = HashSet::new();
    let mut defaults = HashSet::new();
    for path in types {
        let (Some(o), Some(n)) = (old.find_type(path), new.find_type(path)) else {
            continue;
        };
        let mut names = HashSet::new();
        for t in [o, n] {
            let mut at = Some(t);
            while let Some(t) = at {
                names.extend(t.get().vars.keys().map(|name| name.as_str()));
                at = t.parent_type();
            }
        }
        for name in names {
            let before = o.get_var_declaration(name).map(|d| &d.get().var_type);
            let after = n.get_var_declaration(name).map(|d| &d.get().var_type);
            let value = |t: TypeRef| {
                t.get_value(name)
                    .map(|v| format!("{:?}{:?}", v.constant, v.expression))
            };
            let is_const = after.is_some_and(|var_type| var_type.flags.is_const());
            if before != after {
                declared.insert(name.to_owned());
            } else if value(o) != value(n) {
                if is_const {
                    declared.insert(name.to_owned());
                } else {
                    defaults.insert(name.to_owned());
                }
            }
        }
    }
    (declared, defaults)
}

pub(super) fn reparented_users(
    old: &Snapshot,
    new: &Snapshot,
    realigned: &Realigned,
    changes: &mut Changes,
) {
    let types: HashSet<&str> = changes
        .plan
        .reparented
        .iter()
        .flat_map(|r| r.types.iter().map(|t| t.path.as_str()))
        .collect();
    if types.is_empty() {
        return;
    }
    let (declared, defaults) = inherited_changes(old, new, &types);
    let named: HashSet<&str> = declared.iter().map(String::as_str).collect();
    let fields: HashSet<&str> = defaults.iter().map(String::as_str).collect();
    let reloaded: HashSet<ProcKey> = changes
        .plan
        .reloads
        .iter()
        .map(|r| r.proc.clone())
        .collect();
    let mut reloads = vec![];
    for key in new.procs.keys() {
        let Some((type_path, _)) = key.path.rsplit_once('/') else {
            continue;
        };
        let own = types.contains(type_path);
        if !own {
            if named.is_empty() && fields.is_empty() {
                continue;
            }
            let Some((_, value)) = new.find(key) else {
                continue;
            };
            if !names_any(value, &named, &fields) {
                continue;
            }
        }
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
        let code = |nodes: &[Node]| -> Vec<Node> {
            nodes
                .iter()
                .filter(|n| !matches!(n, Node::Instruction(Instruction::DbgLine(_), _)))
                .cloned()
                .collect()
        };
        if !own && code(&before.nodes) == code(&after.nodes) {
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
