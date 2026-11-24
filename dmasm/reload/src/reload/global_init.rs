use std::collections::{BTreeMap, HashMap, HashSet};

use dreammaker::ast::{Expression, Follow, Spanned, Statement, Term, VarStatement};
use dreammaker::objtree::TypeRef;
use std::path::{Path, PathBuf};

use dreammaker::{FileId, Location};

use super::retargeted::Retargeted;
use super::{const_value, value_location, var_constant, ConstValue, ProcReload, Refusal, Snapshot};
use crate::diagnostic::{Diagnostic, Id};
use crate::operands::{DMString, GlobalVar, Label, StaticScope, Variable};
use crate::{syntax, Instruction, Node};

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GlobalInit {
    pub code: ProcReload,
    #[cfg_attr(feature = "serde", serde(default))]
    pub table: usize,
    #[cfg_attr(feature = "serde", serde(default))]
    pub proc_statics: BTreeMap<String, BTreeMap<String, usize>>,
}

fn proc_statics<'a>(
    set: impl Iterator<Item = &'a Initializer<'a>>,
) -> BTreeMap<String, BTreeMap<String, usize>> {
    let mut procs: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for init in set {
        let Some(path) = &init.proc_path else {
            continue;
        };
        let count = init
            .proc_static_names
            .iter()
            .filter(|s| **s == init.name)
            .count();
        procs
            .entry(path.clone())
            .or_default()
            .insert(init.name.clone(), count);
    }
    procs
}

struct Initializer<'a> {
    scope: StaticScope,
    name: String,
    proc: Option<super::ProcKey>,
    location: Location,
    ty: TypeRef<'a>,
    declared: String,
    expression: Option<&'a Expression>,
    constant: Option<ConstValue>,
    proc_path: Option<String>,
    proc_static_names: Vec<String>,
    visible: Vec<(String, u32)>,
    sized_list: bool,
    params: Vec<(String, String)>,
    scoped_late: bool,
    absolute: bool,
}

impl Initializer<'_> {
    fn key(&self) -> (&StaticScope, &str) {
        (&self.scope, &self.name)
    }

    fn computed(&self) -> Option<(&StaticScope, &str, &str, Option<&Expression>)> {
        if self.constant.is_some() {
            return None;
        }
        Some((&self.scope, &self.name, &self.declared, self.expression))
    }
}

impl Snapshot {
    fn initializers(&self, procs: bool) -> Vec<Initializer<'_>> {
        let mut statics: Statics<'_> = HashMap::new();
        for (key, (_, name, index)) in self.procs.iter().filter(|_| procs) {
            let Some((ty, value)) = self.find(key) else {
                continue;
            };
            let Some(code) = &value.code else {
                continue;
            };
            let mut found = vec![];
            static_statements(code, None, &mut found);
            if found.is_empty() {
                continue;
            }
            let proc_path = crate::compiler::proc_path(ty, name, *index == 0);
            let params: Vec<(String, String)> = value
                .parameters
                .iter()
                .map(|p| (p.name.to_string(), p.var_type.type_path.to_string()))
                .collect();
            let proc_static_names: Vec<String> = found
                .iter()
                .map(|(_, var)| var.name.as_str().to_owned())
                .collect();
            let declared = super::locals::declared(code);
            let mut seen: Vec<&str> = vec![];
            let proc_name = name;
            for (location, var) in found {
                let name = var.name.as_str();
                let number = seen.iter().filter(|n| **n == name).count();
                seen.push(name);
                if var.var_type.flags.is_const() {
                    continue;
                }
                let nowhere = Location {
                    file: FileId::UNKNOWN,
                    line: 0,
                    column: 0,
                };
                let constant = match &var.value {
                    None => Some(ConstValue::Null),
                    Some(value) => value
                        .simple_evaluate(nowhere)
                        .ok()
                        .as_ref()
                        .and_then(const_value),
                };
                statics
                    .entry((ty.index().index(), proc_name.to_owned(), *index))
                    .or_default()
                    .push(Initializer {
                        scope: StaticScope::Proc {
                            path: proc_path.clone(),
                            nth: number as u32,
                        },
                        name: name.to_owned(),
                        proc: Some(key.clone()),
                        location,
                        ty,
                        declared: var.var_type.type_path.to_string(),
                        expression: var.value.as_ref(),
                        constant,
                        proc_path: Some(proc_path.clone()),
                        proc_static_names: proc_static_names.clone(),
                        visible: super::visible_statics(&declared, name, number as u32),
                        sized_list: false,
                        params: params.clone(),
                        scoped_late: false,
                        absolute: false,
                    });
            }
        }
        let mut computed = HashSet::new();
        let mut declared: HashMap<FileId, Vec<u32>> = HashMap::new();
        for t in self.tree.iter_types() {
            for (name, var) in t.get().vars.iter() {
                let Some(declaration) = &var.declaration else {
                    continue;
                };
                if procs && declaration.location.file != FileId::BUILTINS {
                    declared
                        .entry(declaration.location.file)
                        .or_default()
                        .push(declaration.location.line);
                }
                let flags = declaration.var_type.flags;
                if declaration.location.file != FileId::BUILTINS
                    && !flags.is_const()
                    && (t.is_root() || flags.is_static())
                    && var_constant(var).is_none()
                {
                    let scope = if t.is_root() {
                        StaticScope::Global
                    } else {
                        StaticScope::Type(t.get().path.clone())
                    };
                    computed.insert((scope, name.to_string()));
                }
            }
        }
        for init in statics.values().flatten() {
            if init.constant.is_none() {
                computed.insert((init.scope.clone(), init.name.clone()));
            }
        }
        let mut walk = Walk {
            chains: self.include_chains(procs),
            computed,
            declared,
            track_defaults: procs,
            ..Walk::default()
        };
        walk.ty(self, self.tree.root(), &mut statics);
        walk.finish(self, &mut statics)
    }

    fn include_chains(&self, read: bool) -> HashMap<FileId, Vec<u32>> {
        let mut paths = vec![];
        self.ctx
            .files()
            .for_each(|path| paths.push(path.to_owned()));
        let unfound = |paths: &[PathBuf]| -> HashMap<FileId, Vec<u32>> {
            paths
                .iter()
                .enumerate()
                .filter_map(|(n, path)| {
                    Some((self.ctx.files().get_id(path)?, vec![u32::MAX, n as u32]))
                })
                .collect()
        };
        let Some(root) = self.root.as_ref().filter(|_| read) else {
            return unfound(&paths);
        };
        let by_path: HashMap<String, FileId> = paths
            .iter()
            .filter_map(|path| Some((normal(path), self.ctx.files().get_id(path)?)))
            .collect();
        let mut parents: HashMap<FileId, (FileId, u32)> = HashMap::new();
        let mut dme = None;
        for path in &paths {
            let name = normal(path);
            if !(name.ends_with(".dm") || name.ends_with(".dme")) {
                continue;
            }
            let Some(&id) = by_path.get(&name) else {
                continue;
            };
            if name.ends_with(".dme") && dme.is_none() {
                dme = Some(id);
            }
            let Ok(text) = std::fs::read(root.join(path)) else {
                continue;
            };
            let dir = path.parent().unwrap_or(Path::new(""));
            for (n, line) in String::from_utf8_lossy(&text).lines().enumerate() {
                let Some(included) = line
                    .trim_start()
                    .strip_prefix("#include")
                    .and_then(|rest| rest.trim_start().strip_prefix('"'))
                    .and_then(|rest| rest.split('"').next())
                else {
                    continue;
                };
                let included = included.replace('\\', "/");
                for candidate in [dir.join(&included), PathBuf::from(&included)] {
                    if let Some(&child) = by_path.get(&normal(&candidate)) {
                        if child != id && Some(child) != dme {
                            parents.entry(child).or_insert((id, n as u32 + 1));
                        }
                        break;
                    }
                }
            }
        }
        let mut chains = unfound(&paths);
        for &id in by_path.values() {
            let mut chain = vec![];
            let mut at = id;
            let mut seen = HashSet::new();
            while let Some(&(parent, line)) = parents.get(&at) {
                if !seen.insert(at) {
                    break;
                }
                chain.push(line);
                at = parent;
            }
            if Some(at) == dme {
                chain.reverse();
                chains.insert(id, chain);
            }
        }
        chains
    }

    pub fn global_init_proc(&self) -> Result<GlobalInit, String> {
        let all = self.initializers(true);
        let (nodes, locals) = build(
            self,
            all.iter()
                .filter(|i| i.constant.is_none())
                .map(|i| (i, i.expression)),
        )?;
        Ok(GlobalInit {
            code: ProcReload {
                proc: super::ProcKey {
                    path: "(global init)".to_owned(),
                    override_id: 0,
                },
                nodes,
                locals,
                params: vec![],
                settings: None,
                statics: vec![],
                warnings: vec![],
            },
            table: 0,
            proc_statics: proc_statics(all.iter().filter(|i| i.constant.is_none())),
        })
    }

    fn statics_read(&self, reads: impl Fn(syntax::Node) -> bool) -> bool {
        self.procs.keys().any(|key| {
            let Some(code) = self.find(key).and_then(|(_, value)| value.code.as_ref()) else {
                return false;
            };
            let mut statics = vec![];
            static_statements(code, None, &mut statics);
            statics.iter().any(|(_, var)| {
                var.value.as_ref().is_some_and(|value| {
                    let mut found = false;
                    syntax::walk_expression(value, &mut |node| found |= reads(node));
                    found
                })
            })
        })
    }

    fn has_statics(&self, key: &super::ProcKey) -> bool {
        let Some(code) = self.find(key).and_then(|(_, value)| value.code.as_ref()) else {
            return false;
        };
        let mut statics = vec![];
        static_statements(code, None, &mut statics);
        !statics.is_empty()
    }
}

fn retargeted(
    (old, was): (&Snapshot, &Initializer<'_>),
    (new, init): (&Snapshot, &Initializer<'_>),
    r: &Retargeted,
) -> bool {
    if r.is_empty() {
        return false;
    }
    match (&was.constant, &init.constant) {
        (Some(ConstValue::Path(path)), Some(_)) => r.retargets(path),
        (None, None) => {
            let Some(expression) = init.expression else {
                return false;
            };
            if !r.expression_might_name(expression, &init.ty.get().path) {
                return false;
            }
            let compiled = |s: &Snapshot, init: &Initializer<'_>| {
                let expression = init.expression?;
                s.compile_value(
                    init.ty,
                    expression,
                    &init.declared,
                    value_location(expression, init.location),
                    init.proc_path.as_deref(),
                    &init.visible,
                )
                .ok()
            };
            match (compiled(old, was), compiled(new, init)) {
                (Some(before), Some(after)) => !r.same_code(&before, &after),
                (None, None) => false,
                _ => true,
            }
        }
        _ => false,
    }
}

#[allow(clippy::type_complexity)]
fn differences<'a, 'b>(
    (old, before): (&Snapshot, &'b [Initializer<'a>]),
    (new, after): (&Snapshot, &'b [Initializer<'a>]),
    r: &Retargeted,
) -> Option<Vec<(&'b Initializer<'a>, Option<&'a Expression>)>> {
    let was: std::collections::HashMap<_, _> = before.iter().map(|i| (i.key(), i)).collect();
    let mut table = vec![];
    let mut recompiled = false;
    for init in after {
        let Some(was) = was.get(&init.key()) else {
            continue;
        };
        let same_expression = was.expression == init.expression;
        match (&was.constant, &init.constant) {
            (Some(was_value), Some(now)) if was_value == now => {
                if same_expression && retargeted((old, was), (new, init), r) {
                    table.push((init, init.expression));
                }
            }
            (_, Some(_)) => table.push((init, init.expression)),
            (Some(ConstValue::Null), None) => {}
            (Some(_), None) => table.push((init, None)),
            (None, None) => {
                recompiled |= same_expression && retargeted((old, was), (new, init), r);
            }
        }
    }
    let computed = |list: &'b [Initializer<'a>]| {
        list.iter()
            .filter_map(Initializer::computed)
            .collect::<Vec<_>>()
    };
    if table.is_empty() && !recompiled && computed(before) == computed(after) {
        return None;
    }
    Some(table)
}

fn renumber_old_statics(
    old: &Snapshot,
    new: &Snapshot,
    r: &Retargeted,
    before: &mut [Initializer<'_>],
) {
    let mut numbers: HashMap<super::ProcKey, Vec<(String, u32, u32)>> = HashMap::new();
    for key in new.procs() {
        let Some(old_key) = r.old_key(key) else {
            continue;
        };
        let (Some(was), Some(now)) = (old.proc_code(&old_key), new.proc_code(key)) else {
            continue;
        };
        let declared = super::locals::declared(was);
        if declared.is_empty() || was == now {
            continue;
        }
        let (pairs, _) = super::locals::matched(&declared, &super::locals::declared(now));
        let pairs = pairs
            .into_iter()
            .map(|(name, new, old)| (name.to_owned(), new, old))
            .collect();
        numbers.insert(old_key, pairs);
    }
    for Initializer {
        proc, scope, name, ..
    } in before
    {
        let Some(pairs) = proc.as_ref().and_then(|proc| numbers.get(proc)) else {
            continue;
        };
        let StaticScope::Proc { nth, .. } = scope else {
            continue;
        };
        *nth = pairs
            .iter()
            .find(|(had, _, old)| had == name && old == nth)
            .map_or(u32::MAX - *nth, |(_, new, _)| *new);
    }
}

fn changed(new: &Snapshot, before: &[Initializer<'_>], after: &[Initializer<'_>]) -> Vec<Refusal> {
    let was: HashMap<_, _> = before.iter().map(|i| (i.key(), i)).collect();
    after
        .iter()
        .filter(|init| {
            was.get(&init.key()).is_some_and(|was| {
                was.constant != init.constant || was.expression != init.expression
            })
        })
        .map(|init| {
            let shown = match &init.scope {
                StaticScope::Proc { path, .. } => path,
                StaticScope::Global => "(global)",
                StaticScope::Type(path) => path,
            };
            Refusal::new(Id::ChangedInitializer, shown, "changed initializer")
                .named(init.name.clone())
                .until(super::Until::Reboot)
                .at(new, init.location)
        })
        .collect()
}

type Statics<'a> = HashMap<(usize, String, usize), Vec<Initializer<'a>>>;

enum Waiting<'a> {
    Initializer(Initializer<'a>),
    Type(TypeRef<'a>),
    Default(Initializer<'a>),
}

const LIST_PROCS: &[&str] = &[
    "Add",
    "Copy",
    "Cut",
    "Find",
    "Insert",
    "Join",
    "Remove",
    "RemoveAll",
    "Splice",
    "Swap",
];

const CORE_TYPES: &[&str] = &[
    "/datum", "/atom", "/area", "/turf", "/obj", "/mob", "/image", "/client", "/world",
];

const LATE_TYPES: &[&str] = &[
    "/sound",
    "/icon",
    "/matrix",
    "/database",
    "/exception",
    "/regex",
    "/mutable_appearance",
    "/dm_filter",
    "/generator",
    "/particles",
];

fn visited_late(byond: crate::ByondVersion, t: TypeRef) -> bool {
    let path = t.get().path.as_str();
    if CORE_TYPES.contains(&path) || super::reparent::builtin_base(t).as_deref() != Some(path) {
        return false;
    }
    if path == "/mutable_appearance" {
        return !byond.visits_mutable_appearance_at_root();
    }
    t.parent_type()
        .filter(|p| !p.is_root())
        .is_none_or(|p| p.get().path == "/datum")
}

#[derive(Default)]
struct Walk<'a> {
    now: Vec<Initializer<'a>>,
    later: Vec<Waiting<'a>>,
    finishing: bool,
    last_pass: bool,
    track_defaults: bool,
    defaults_waiting: HashMap<usize, usize>,
    computed: HashSet<(StaticScope, String)>,
    set: HashSet<(StaticScope, String)>,
    passed: HashSet<(StaticScope, String)>,
    declared: HashMap<FileId, Vec<u32>>,
    blocks: HashMap<FileId, HashMap<u32, Vec<u32>>>,
    types: HashSet<usize>,
    procs: HashSet<(usize, String)>,
    first: HashMap<usize, Option<Vec<u32>>>,
    chains: HashMap<FileId, Vec<u32>>,
    texts: HashMap<FileId, Vec<String>>,
}

impl<'a> Walk<'a> {
    fn at(&self, location: Location) -> Vec<u32> {
        let mut at = match self.chains.get(&location.file) {
            Some(chain) => chain.clone(),
            None => vec![u32::MAX],
        };
        at.extend([location.line, location.column as u32]);
        at
    }

    fn var_at(&mut self, snapshot: &Snapshot, location: Location) -> Vec<u32> {
        if !self.blocks.contains_key(&location.file) {
            self.line(snapshot, location);
            let blocks = match (
                self.texts.get(&location.file),
                self.declared.get(&location.file),
            ) {
                (Some(lines), Some(declared)) => var_blocks(lines, declared),
                _ => HashMap::new(),
            };
            self.blocks.insert(location.file, blocks);
        }
        let mut at = match self.chains.get(&location.file) {
            Some(chain) => chain.clone(),
            None => vec![u32::MAX],
        };
        if let Some(place) = self.blocks[&location.file].get(&location.line) {
            at.extend(place);
        }
        at.extend([location.line, location.column as u32]);
        at
    }

    fn absolute_order(&mut self, snapshot: &Snapshot, root: TypeRef<'a>) -> HashMap<&'a str, u32> {
        enum Entry<'n> {
            Node(String, Vec<Entry<'n>>),
            Var(&'n str),
        }
        fn number<'n>(entries: &[Entry<'n>], order: &mut HashMap<&'n str, u32>) {
            for entry in entries {
                match entry {
                    Entry::Node(_, below) => number(below, order),
                    Entry::Var(name) => {
                        let place = order.len() as u32;
                        order.insert(*name, place);
                    }
                }
            }
        }
        let mut declared: Vec<(Vec<u32>, Vec<String>, &'a str)> = vec![];
        for (name, var) in root.get().vars.iter() {
            let Some(declaration) = &var.declaration else {
                continue;
            };
            if declaration.location.file == FileId::BUILTINS
                || !self.absolute(snapshot, declaration.location)
            {
                continue;
            }
            let line = self
                .line(snapshot, declaration.location)
                .unwrap_or_default();
            let text = line.trim_start();
            let end = text
                .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '/'))
                .unwrap_or(text.len());
            let mut parts: Vec<String> = text[..end]
                .split('/')
                .filter(|part| !part.is_empty())
                .skip(1)
                .map(str::to_owned)
                .collect();
            parts.pop();
            declared.push((self.at(declaration.location), parts, name.as_str()));
        }
        declared.sort();
        let mut tree: Vec<Entry<'a>> = vec![];
        for (_, parts, name) in declared {
            let mut level = &mut tree;
            for part in parts {
                let found = level
                    .iter()
                    .position(|entry| matches!(entry, Entry::Node(name, _) if *name == part));
                let at = found.unwrap_or_else(|| {
                    level.push(Entry::Node(part, vec![]));
                    level.len() - 1
                });
                level = match &mut level[at] {
                    Entry::Node(_, below) => below,
                    Entry::Var(_) => unreachable!(),
                };
            }
            level.push(Entry::Var(name));
        }
        let mut order = HashMap::new();
        number(&tree, &mut order);
        order
    }

    fn ty(&mut self, snapshot: &'a Snapshot, t: TypeRef<'a>, statics: &mut Statics<'a>) {
        enum Item<'a> {
            Var(&'a str, &'a dreammaker::objtree::TypeVar),
            Default(&'a str, &'a dreammaker::objtree::TypeVar),
            Proc(&'a str),
            Statics(&'a str, usize),
            Declared(dreammaker::ast::ProcDeclKind),
            Type(TypeRef<'a>),
        }
        let mut items: Vec<(Vec<u32>, Item<'a>)> = vec![];
        let absolute = if t.is_root() {
            self.absolute_order(snapshot, t)
        } else {
            HashMap::new()
        };
        for (name, var) in t.get().vars.iter() {
            if let Some(declaration) = &var.declaration {
                if declaration.location.file != FileId::BUILTINS {
                    let at = match absolute.get(name.as_str()) {
                        Some(place) => vec![0, *place],
                        None => self.var_at(snapshot, declaration.location),
                    };
                    items.push((at, Item::Var(name.as_str(), var)));
                }
            } else if self.track_defaults
                && var.value.location.file != FileId::BUILTINS
                && var.value.expression.as_ref().is_some_and(|e| !literal(e))
            {
                let at = self.at(var.value.location);
                items.push((at, Item::Default(name.as_str(), var)));
            }
        }
        let mut declared: HashMap<dreammaker::ast::ProcDeclKind, Vec<u32>> = HashMap::new();
        for (name, p) in t.get().procs.iter() {
            for (index, value) in p
                .value
                .iter()
                .enumerate()
                .filter(|(_, v)| v.location.file != FileId::BUILTINS)
            {
                let at = self.at(value.location);
                match &p.declaration {
                    Some(declaration) if index == 0 => {
                        let first = declared.entry(declaration.kind).or_insert(at.clone());
                        if at < *first {
                            *first = at;
                        }
                    }
                    _ => {
                        items.push((at.clone(), Item::Proc(name.as_str())));
                        items.push((at, Item::Statics(name.as_str(), index)));
                    }
                }
            }
        }
        for (kind, at) in declared {
            items.push((at, Item::Declared(kind)));
        }
        for child in t.children() {
            if let Some(at) = self.first(child) {
                items.push((at, Item::Type(child)));
            }
        }
        items.sort_by(|(a, _), (b, _)| a.cmp(b));
        for (_, item) in items {
            match item {
                Item::Var(name, var) => {
                    let declaration = var.declaration.as_ref().unwrap();
                    let flags = declaration.var_type.flags;
                    let scope = if t.is_root() {
                        StaticScope::Global
                    } else {
                        StaticScope::Type(t.get().path.clone())
                    };
                    self.passed.insert((scope.clone(), name.to_owned()));
                    if !flags.is_const() && (t.is_root() || flags.is_static()) {
                        let init = Initializer {
                            scope,
                            name: name.to_owned(),
                            proc: None,
                            location: var.value.location,
                            ty: t,
                            declared: declaration.var_type.type_path.to_string(),
                            expression: var.value.expression.as_ref(),
                            constant: var_constant(var),
                            proc_path: None,
                            proc_static_names: vec![],
                            visible: vec![],
                            sized_list: false,
                            params: vec![],
                            scoped_late: false,
                            absolute: t.is_root() && self.absolute(snapshot, declaration.location),
                        };
                        self.add(snapshot, init);
                    } else {
                        self.var_default(snapshot, t, name, var);
                    }
                }
                Item::Default(name, var) => self.var_default(snapshot, t, name, var),
                Item::Proc(name) => {
                    self.procs.insert((t.index().index(), name.to_owned()));
                }
                Item::Statics(name, index) => {
                    let mut own = statics
                        .remove(&(t.index().index(), name.to_owned(), index))
                        .unwrap_or_default();
                    own.sort_by_cached_key(|init| self.at(init.location));
                    for init in own {
                        self.add(snapshot, init);
                    }
                }
                Item::Declared(kind) => {
                    let mut procs: Vec<(Vec<u32>, &str)> = t
                        .get()
                        .procs
                        .iter()
                        .filter(|(_, p)| p.declaration.as_ref().is_some_and(|d| d.kind == kind))
                        .filter_map(|(name, p)| {
                            let first = p.value.first()?;
                            (first.location.file != FileId::BUILTINS)
                                .then(|| (self.at(first.location), name.as_str()))
                        })
                        .collect();
                    procs.sort();
                    for (_, name) in procs {
                        self.procs.insert((t.index().index(), name.to_owned()));
                        let mut own = statics
                            .remove(&(t.index().index(), name.to_owned(), 0))
                            .unwrap_or_default();
                        own.sort_by_cached_key(|init| self.at(init.location));
                        for init in own {
                            self.add(snapshot, init);
                        }
                    }
                }
                Item::Type(child) => self.enter(snapshot, child, statics),
            }
        }
        self.types.insert(t.index().index());
    }

    fn enter(&mut self, snapshot: &'a Snapshot, t: TypeRef<'a>, statics: &mut Statics<'a>) {
        let path = t.get().path.as_str();
        let at_root = t.parent_path().is_some_and(|p| p.is_root());
        if self.finishing || !at_root || CORE_TYPES.contains(&path) {
            return self.ty(snapshot, t, statics);
        }
        if visited_late(snapshot.byond, t) {
            return;
        }
        match t.parent_type().filter(|p| !p.is_root()) {
            Some(parent) if !self.types.contains(&parent.index().index()) => {
                self.later.push(Waiting::Type(t));
            }
            _ => self.ty(snapshot, t, statics),
        }
    }

    fn add(&mut self, snapshot: &'a Snapshot, mut init: Initializer<'a>) {
        init.sized_list = self
            .line(snapshot, init.location)
            .is_some_and(|line| line.contains(&format!("{}[", init.name)));
        if init.constant.is_some() || self.is_ready(snapshot, &init) {
            self.run(init);
        } else {
            if let Some(expression) = init.expression {
                let mut scoped = vec![];
                terms(expression, &mut |seen| {
                    if let Seen::StaticFieldOf(Term::Prefab(prefab)) = seen {
                        scoped.extend(snapshot.tree.find(&prefab.path.to_string()));
                    }
                });
                for found in scoped
                    .into_iter()
                    .filter(|_| snapshot.byond.runs_scoped_reads_last())
                {
                    let mut above = found.parent_type();
                    while let Some(t) = above.filter(|t| !t.is_root()) {
                        init.scoped_late |= self.first(t).is_none();
                        above = t.parent_type();
                    }
                }
            }
            self.later.push(Waiting::Initializer(init));
        }
    }

    fn is_ready(&self, snapshot: &Snapshot, init: &Initializer) -> bool {
        if init.scoped_late && !self.last_pass {
            return false;
        }
        self.declared_ready(snapshot, &init.declared)
            && init
                .expression
                .is_none_or(|e| self.ready(snapshot, init, e))
    }

    fn var_default(
        &mut self,
        snapshot: &'a Snapshot,
        t: TypeRef<'a>,
        name: &str,
        var: &'a dreammaker::objtree::TypeVar,
    ) {
        let Some(expression) = var.value.expression.as_ref().filter(|e| !literal(e)) else {
            return;
        };
        if !self.track_defaults {
            return;
        }
        let mut scoped = false;
        terms(expression, &mut |seen| {
            scoped |= matches!(seen, Seen::StaticFieldOf(_))
        });
        if scoped {
            return;
        }
        let init = Initializer {
            scope: StaticScope::Type(t.get().path.clone()),
            name: name.to_owned(),
            proc: None,
            location: var.value.location,
            ty: t,
            declared: String::new(),
            expression: Some(expression),
            constant: None,
            proc_path: None,
            proc_static_names: vec![],
            visible: vec![],
            sized_list: false,
            params: vec![],
            scoped_late: false,
            absolute: false,
        };
        if !self.is_ready(snapshot, &init) {
            *self.defaults_waiting.entry(t.index().index()).or_default() += 1;
            self.later.push(Waiting::Default(init));
        }
    }

    fn defaults_wait(&self, t: TypeRef) -> bool {
        let mut at = Some(t);
        while let Some(t) = at.filter(|t| !t.is_root()) {
            if self
                .defaults_waiting
                .get(&t.index().index())
                .is_some_and(|n| *n > 0)
            {
                return true;
            }
            at = t.parent_type();
        }
        false
    }

    fn run(&mut self, init: Initializer<'a>) {
        self.set.insert((init.scope.clone(), init.name.clone()));
        self.now.push(init);
    }

    fn finish(mut self, snapshot: &'a Snapshot, statics: &mut Statics<'a>) -> Vec<Initializer<'a>> {
        self.finishing = true;
        let unnamed: Vec<TypeRef<'a>> = snapshot.tree.iter_types().collect();
        for t in unnamed {
            if self.first(t).is_none() {
                self.types.insert(t.index().index());
            }
        }
        let mut waiting = std::mem::take(&mut self.later);
        let lead = waiting
            .iter()
            .take_while(|entry| matches!(entry, Waiting::Initializer(init) if init.absolute))
            .count();
        let rest = waiting.split_off(lead);
        let mut progress = false;
        for entry in waiting {
            progress |= self.turn(snapshot, statics, entry);
        }
        let root = snapshot.tree.root();
        let mut late: Vec<(usize, Vec<u32>, TypeRef<'a>)> = vec![];
        let children: Vec<TypeRef<'a>> = root.children().collect();
        for child in children {
            let path = child.get().path.as_str();
            if !visited_late(snapshot.byond, child) {
                continue;
            }
            if let Some(at) = self.first(child) {
                let rank = LATE_TYPES
                    .iter()
                    .position(|late| *late == path)
                    .unwrap_or(LATE_TYPES.len());
                late.push((rank, at, child));
            }
        }
        late.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        for (_, _, t) in late {
            self.ty(snapshot, t, statics);
        }
        for entry in rest {
            progress |= self.turn(snapshot, statics, entry);
        }

        loop {
            let waiting = std::mem::take(&mut self.later);
            if waiting.is_empty() {
                break;
            }
            if !progress && !self.last_pass {
                self.last_pass = true;
            } else if !progress {
                for entry in waiting {
                    if let Waiting::Initializer(init) = entry {
                        self.run(init);
                    }
                }
                break;
            }
            progress = false;
            for entry in waiting {
                progress |= self.turn(snapshot, statics, entry);
            }
        }
        self.now
    }

    fn turn(
        &mut self,
        snapshot: &'a Snapshot,
        statics: &mut Statics<'a>,
        entry: Waiting<'a>,
    ) -> bool {
        match entry {
            Waiting::Initializer(init) if self.is_ready(snapshot, &init) => self.run(init),
            Waiting::Default(init) if self.is_ready(snapshot, &init) => {
                if let Some(n) = self.defaults_waiting.get_mut(&init.ty.index().index()) {
                    *n -= 1;
                }
            }
            Waiting::Type(t) => self.ty(snapshot, t, statics),
            entry => {
                self.later.push(entry);
                return false;
            }
        }
        true
    }

    fn value_set(&self, init: &Initializer, name: &str, global: bool, finishing: bool) -> bool {
        let set = |scope: StaticScope| {
            let key = (scope, name.to_owned());
            (finishing || self.passed.contains(&key))
                && (!self.computed.contains(&key) || self.set.contains(&key))
        };
        let t = init.ty;
        if !global {
            if let Some(proc_path) = &init.proc_path {
                if let Some((_, number)) = init.visible.iter().rev().find(|(n, _)| n == name) {
                    let scope = StaticScope::Proc {
                        path: proc_path.clone(),
                        nth: *number,
                    };
                    let key = (scope, name.to_owned());
                    return !self.computed.contains(&key) || self.set.contains(&key);
                }
                if init.proc_static_names.iter().any(|s| s == name) {
                    return finishing && !self.computed.iter().any(|(scope, n)| {
                        n == name
                            && matches!(scope, StaticScope::Proc { path, .. } if path == proc_path)
                            && !self.set.contains(&(scope.clone(), n.clone()))
                    });
                }
            }
            if let Some(declaration) = t
                .get_var_declaration(name)
                .filter(|d| !t.is_root() && !d.ty().is_root())
            {
                let flags = declaration.get().var_type.flags;
                return !flags.is_static()
                    || flags.is_const()
                    || set(StaticScope::Type(declaration.ty().get().path.clone()));
            }
        }
        match t.tree().root().get().vars.get(name) {
            Some(var)
                if var.declaration.is_some() && var.value.location.file != FileId::BUILTINS =>
            {
                set(StaticScope::Global)
            }
            _ => true,
        }
    }

    fn absolute(&mut self, snapshot: &Snapshot, location: Location) -> bool {
        self.line(snapshot, location)
            .is_some_and(|line| line.trim_start().starts_with("/var/"))
    }

    fn line(&mut self, snapshot: &Snapshot, location: Location) -> Option<&str> {
        let lines = self.texts.entry(location.file).or_insert_with(|| {
            let Some(root) = &snapshot.root else {
                return vec![];
            };
            std::fs::read(root.join(snapshot.file_name(location.file)))
                .map(|text| {
                    String::from_utf8_lossy(&text)
                        .lines()
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        });
        lines
            .get(location.line.saturating_sub(1) as usize)
            .map(String::as_str)
    }

    fn first(&mut self, t: TypeRef<'a>) -> Option<Vec<u32>> {
        if let Some(at) = self.first.get(&t.index().index()) {
            return at.clone();
        }
        let own = std::iter::once(t.get().location)
            .chain(t.get().vars.values().map(|v| v.value.location))
            .chain(
                t.get()
                    .procs
                    .values()
                    .flat_map(|p| p.value.iter())
                    .map(|v| v.location),
            )
            .filter(|at| at.file != FileId::BUILTINS)
            .map(|at| self.at(at))
            .min();
        let children: Vec<_> = t.children().collect();
        let at = children
            .into_iter()
            .filter_map(|c| self.first(c))
            .chain(own)
            .min();
        self.first.insert(t.index().index(), at.clone());
        at
    }

    fn declared_ready(&self, snapshot: &Snapshot, declared: &str) -> bool {
        let mut path = format!("/{}", declared.trim_start_matches('/'));
        while let Some(of) = path.strip_prefix("/list/") {
            path = format!("/{of}");
        }
        if path == "/" || path == "/list" {
            return true;
        }
        snapshot
            .tree
            .find(&path)
            .is_some_and(|t| self.types.contains(&t.index().index()))
    }

    fn ready(&self, snapshot: &Snapshot, init: &Initializer, expression: &Expression) -> bool {
        let finishing = self.finishing;
        let (t, declared) = (init.ty, init.declared.as_str());
        let before =
            |ty: TypeRef, name: &str| self.procs.contains(&(ty.index().index(), name.to_owned()));
        let mut ready = true;
        let root = snapshot.tree.root();
        let path = |path: &str| {
            let proc_at = ["/proc/", ".proc/", "/verb/", ".verb/"]
                .iter()
                .filter_map(|p| path.find(p))
                .min();
            let (ty, proc) = match proc_at {
                Some(at) => (&path[..at], true),
                None => (path, false),
            };
            if ty.is_empty() && path.starts_with('.') {
                let name = path.rsplit('/').next().unwrap_or_default();
                let mut at = Some(t);
                while let Some(ty) = at {
                    if ty.get().procs.get(name).is_some_and(|p| {
                        p.value.iter().any(|v| v.location.file != FileId::BUILTINS)
                    }) {
                        return before(ty, name);
                    }
                    at = ty.parent_type();
                }
                return true;
            }
            if !proc && ty == "/list" && init.sized_list {
                return true;
            }
            match snapshot.tree.find(ty.trim_end_matches('/')) {
                Some(found) if proc => {
                    let name = path.rsplit('/').next().unwrap_or_default();
                    before(found, name)
                }
                Some(found) => self.types.contains(&found.index().index()),
                None => false,
            }
        };
        let call = |name: &str, global: bool| {
            let mut at = if global { Some(root) } else { Some(t) };
            while let Some(ty) = at {
                if let Some(p) = ty.get().procs.get(name) {
                    if p.value.iter().any(|v| v.location.file != FileId::BUILTINS) {
                        return before(ty, name);
                    }
                }
                at = if ty.is_root() {
                    None
                } else {
                    ty.parent_type().or(Some(root))
                };
            }
            true
        };
        terms(expression, &mut |seen| {
            let term = match seen {
                Seen::Term(term) => term,
                Seen::GlobalCall(name) => {
                    ready &= call(name, true);
                    return;
                }
                Seen::GlobalVar(name) => {
                    ready &= self.value_set(init, name, true, finishing);
                    return;
                }
                Seen::StaticFieldOf(Term::Ident(name))
                    if ["type", "src"].contains(&name.as_str()) =>
                {
                    ready &= self.types.contains(&t.index().index()) && !self.defaults_wait(t);
                    return;
                }
                Seen::StaticFieldOf(Term::Prefab(prefab)) => {
                    let found = snapshot.tree.find(&prefab.path.to_string());
                    ready &= found.is_none_or(|found| {
                        self.types.contains(&found.index().index()) && !self.defaults_wait(found)
                    });
                    return;
                }
                Seen::StaticFieldOf(Term::Ident(name)) => {
                    let declared = |ty: TypeRef| {
                        ty.get_var_declaration(name)
                            .map(|d| d.get().var_type.type_path.to_string())
                    };
                    let found = declared(t)
                        .or_else(|| declared(root))
                        .and_then(|path| {
                            snapshot
                                .tree
                                .find(&format!("/{}", path.trim_start_matches('/')))
                        })
                        .filter(|found| !found.is_root());
                    ready &= found.is_none_or(|found| {
                        self.types.contains(&found.index().index()) && !self.defaults_wait(found)
                    });
                    return;
                }
                Seen::StaticFieldOf(_) => return,
                Seen::MemberOf(name, proc) => {
                    let declared = |ty: TypeRef| {
                        ty.get_var_declaration(name)
                            .map(|d| d.get().var_type.type_path.to_string())
                    };
                    let param = init.params.iter().find(|(param, _)| param == name);
                    let path = match param
                        .map(|(_, path)| path.clone())
                        .or_else(|| declared(t))
                        .or_else(|| declared(root))
                    {
                        Some(path) => path,
                        None if name == "world" => "world".to_owned(),
                        None => String::new(),
                    };
                    let path = format!("/{}", path.trim_start_matches('/'));
                    let list = path == "/list" || path.starts_with("/list/");
                    if proc.is_some_and(|proc| LIST_PROCS.contains(&proc))
                        && (list || path == "/")
                        && snapshot.byond.defers_list_proc_initializers()
                    {
                        ready &= finishing;
                    } else if path != "/" && !list {
                        ready &= snapshot
                            .tree
                            .find(&path)
                            .is_none_or(|found| self.types.contains(&found.index().index()));
                    }
                    return;
                }
            };
            ready &= match term {
                Term::__TYPE__ => self.types.contains(&t.index().index()),
                Term::Ident(name) => self.value_set(init, name, false, finishing),
                Term::GlobalIdent(name) => self.value_set(init, name, true, finishing),
                Term::Call(name, _) => call(name, false),
                Term::GlobalCall(name, _) => call(name, true),
                Term::Prefab(prefab) | Term::NewPrefab { prefab, .. } => {
                    let text = prefab.path.to_string();
                    let changed = !prefab.vars.is_empty()
                        && snapshot
                            .tree
                            .find(&text)
                            .is_some_and(|found| self.defaults_wait(found));
                    !changed
                        && (!(text.starts_with('/')
                            || text.starts_with(".proc/")
                            || text.starts_with(".verb/"))
                            || path(&text))
                }
                Term::NewImplicit { .. } => self.declared_ready(snapshot, declared),
                _ => true,
            };
        });
        ready
    }
}

fn var_blocks(lines: &[String], declared: &[u32]) -> HashMap<u32, Vec<u32>> {
    let indent = |line: &str| line.len() - line.trim_start().len();
    let path = |line: &str| -> (Vec<String>, bool) {
        let text = line.trim_start();
        let end = text
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '/'))
            .unwrap_or(text.len());
        let rest = text[end..].trim_start();
        let parts = text[..end]
            .split('/')
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect();
        (parts, rest.is_empty() || rest.starts_with("//"))
    };
    let mut found: Vec<(u32, u32, Vec<(String, u32)>)> = vec![];
    for &line in declared {
        let Some(own) = lines.get(line.saturating_sub(1) as usize) else {
            continue;
        };
        let (mut parts, _) = path(own);
        parts.pop();
        if parts.iter().any(|part| part == "var") {
            continue;
        }
        let mut parts: Vec<(String, u32)> = parts.into_iter().map(|part| (part, line)).collect();
        let mut within = indent(own);
        let mut root = None;
        for above in (0..line.saturating_sub(1) as usize).rev() {
            if within == 0 {
                break;
            }
            let text = lines[above].trim();
            if text.is_empty()
                || text.starts_with('#')
                || text.starts_with("//")
                || text.starts_with("/*")
                || text.starts_with('*')
                || indent(&lines[above]) >= within
            {
                continue;
            }
            let (header, alone) = path(&lines[above]);
            if !alone || header.is_empty() {
                continue;
            }
            within = indent(&lines[above]);
            let number = above as u32 + 1;
            let var = header.iter().position(|part| part == "var");
            let below = match var {
                Some(var) => &header[var + 1..],
                None => &header[..],
            };
            parts.splice(0..0, below.iter().map(|part| (part.clone(), number)));
            if var.is_some() {
                root = Some(number);
                break;
            }
        }
        if let Some(root) = root {
            found.push((line, root, parts));
        }
    }
    let mut first: HashMap<(u32, Vec<&str>), u32> = HashMap::new();
    for (_, root, parts) in &found {
        for n in 0..parts.len() {
            let node: Vec<&str> = parts[..=n].iter().map(|(part, _)| part.as_str()).collect();
            let at = first.entry((*root, node)).or_insert(parts[n].1);
            *at = (*at).min(parts[n].1);
        }
    }
    found
        .iter()
        .map(|(line, root, parts)| {
            let mut place = vec![*root];
            for n in 0..parts.len() {
                let node: Vec<&str> = parts[..=n].iter().map(|(part, _)| part.as_str()).collect();
                place.push(first[&(*root, node)]);
            }
            (*line, place)
        })
        .collect()
}

fn literal(expression: &Expression) -> bool {
    match expression {
        Expression::Base { term, follow } => {
            follow.is_empty()
                && matches!(
                    term.elem,
                    Term::Null
                        | Term::Int(_)
                        | Term::Float(_)
                        | Term::String(_)
                        | Term::Resource(_)
                )
        }
        _ => false,
    }
}

enum Seen<'e> {
    Term(&'e Term),
    GlobalCall(&'e str),
    GlobalVar(&'e str),
    StaticFieldOf(&'e Term),
    MemberOf(&'e str, Option<&'e str>),
}

fn terms<'e>(expression: &'e Expression, each: &mut dyn FnMut(Seen<'e>)) {
    syntax::walk_expression(expression, &mut |node| match node {
        syntax::Node::Expression(Expression::Base { term, follow }) => {
            let (name, first) = match &term.elem {
                Term::Ident(name) => (Some(name.as_str()), follow.first().map(|f| &f.elem)),
                _ => (None, None),
            };
            match first {
                Some(Follow::Call(_, proc, _)) if name == Some("global") => {
                    each(Seen::GlobalCall(proc.as_str()))
                }
                Some(Follow::Field(_, field)) if name == Some("global") => {
                    each(Seen::GlobalVar(field.as_str()))
                }
                _ => {}
            }
            if follow
                .iter()
                .any(|f| matches!(f.elem, Follow::StaticField(_) | Follow::ProcReference(_)))
            {
                each(Seen::StaticFieldOf(&term.elem));
            }
            match (name, first) {
                (Some(name), Some(Follow::Field(..))) => each(Seen::MemberOf(name, None)),
                (Some(name), Some(Follow::Call(_, proc, _))) => {
                    each(Seen::MemberOf(name, Some(proc.as_str())))
                }
                _ => {}
            }
        }
        syntax::Node::Term(term) => each(Seen::Term(term)),
        _ => {}
    });
}

fn normal(path: &Path) -> String {
    let mut parts: Vec<&str> = vec![];
    let text = path.to_string_lossy().replace('\\', "/");
    for part in text.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/").to_ascii_lowercase()
}

pub(super) fn static_initializers(code: &[Spanned<Statement>]) -> Vec<(&str, Option<&Expression>)> {
    let mut found = vec![];
    static_statements(code, None, &mut found);
    found
        .into_iter()
        .map(|(_, var)| (var.name.as_str(), var.value.as_ref()))
        .collect()
}

pub(super) fn static_statements<'a>(
    block: &'a [Spanned<Statement>],
    at: Option<Location>,
    out: &mut Vec<(Location, &'a VarStatement)>,
) {
    for statement in block {
        one_statement(&statement.elem, at.unwrap_or(statement.location), out);
    }
}

fn one_statement<'a>(
    statement: &'a Statement,
    location: Location,
    out: &mut Vec<(Location, &'a VarStatement)>,
) {
    let is_static =
        |var: &VarStatement| var.var_type.flags.is_static() || var.var_type.flags.is_const();
    match statement {
        Statement::Var(var) if is_static(var) => out.push((location, var)),
        Statement::Vars(vars) => {
            out.extend(vars.iter().filter(|v| is_static(v)).map(|v| (location, v)))
        }
        Statement::While { block, .. }
        | Statement::DoWhile { block, .. }
        | Statement::ForInfinite { block }
        | Statement::Spawn { block, .. }
        | Statement::Label { block, .. } => static_statements(block, None, out),
        Statement::If { arms, else_arm } => {
            for (_, block) in arms {
                static_statements(block, None, out);
            }
            if let Some(block) = else_arm {
                static_statements(block, None, out);
            }
        }
        Statement::ForLoop {
            init, inc, block, ..
        } => {
            if let Some(init) = init {
                one_statement(init, location, out);
            }
            static_statements(block, None, out);
            if let Some(inc) = inc {
                one_statement(inc, location, out);
            }
        }
        Statement::ForList(each) => static_statements(&each.block, None, out),
        Statement::ForKeyValue(each) => static_statements(&each.block, None, out),
        Statement::ForRange(each) => static_statements(&each.block, None, out),
        Statement::Switch { cases, default, .. } => {
            for (_, block) in cases.iter() {
                static_statements(block, None, out);
            }
            if let Some(block) = default {
                static_statements(block, None, out);
            }
        }
        Statement::TryCatch {
            try_block,
            catch_block,
            ..
        } => {
            static_statements(try_block, None, out);
            static_statements(catch_block, None, out);
        }
        _ => {}
    }
}

#[allow(clippy::type_complexity)]
fn build<'a, 'b>(
    new: &'a Snapshot,
    runs: impl Iterator<Item = (&'b Initializer<'a>, Option<&'a Expression>)>,
) -> Result<(Vec<Node>, Vec<String>), String>
where
    'a: 'b,
{
    let mut nodes = vec![];
    let mut locals: Vec<String> = vec![];
    let mut file = None;
    for (n, (init, expression)) in runs.enumerate() {
        let shown = match &init.scope {
            StaticScope::Global => format!("(global): {}", init.name),
            StaticScope::Type(path) => format!("{}: {}", path, init.name),
            StaticScope::Proc { path, nth } => format!("{}#{}: {}", path, nth, init.name),
        };
        let null;
        let expression = match expression {
            Some(expression) => expression,
            None => {
                null = Expression::Base {
                    term: Box::new(Spanned::new(init.location, Term::Null)),
                    follow: Box::new([]),
                };
                &null
            }
        };
        let location = value_location(expression, init.location);
        let code = match new.compile_value(
            init.ty,
            expression,
            &init.declared,
            location,
            init.proc_path.as_deref(),
            &init.visible,
        ) {
            Ok(code) => code,
            Err(e) => {
                return Err(format!("{shown}: initializer compilation failed: {e}"));
            }
        };
        let mut value: Vec<Node> = code
            .nodes
            .into_iter()
            .filter(|node| {
                !matches!(
                    node,
                    Node::Instruction(Instruction::DbgFile(_) | Instruction::DbgLine(_), _)
                )
            })
            .collect();
        if !matches!(
            value[..],
            [
                ..,
                Node::Instruction(Instruction::Ret, _),
                Node::Instruction(Instruction::End, _)
            ]
        ) {
            return Err(format!(
                "{shown}: initializer compilation failed: its code doesn't end in a return"
            ));
        }
        value.truncate(value.len() - 2);
        let own = |label: &mut Label| {
            if let Label::Named(name) = label {
                *name = format!("{name}_{n}");
            }
        };
        for node in &mut value {
            match node {
                Node::Label(label) => own(label),
                Node::Instruction(ins, _) => {
                    crate::compiler::labels_mut(ins).into_iter().for_each(own)
                }
                Node::Comment(_) => {}
            }
        }
        let places = [location, init.location];
        let place = places
            .iter()
            .copied()
            .find(|at| at.file != FileId::UNKNOWN && at.file != FileId::BUILTINS);
        if let Some(place) = place {
            if file != Some(place.file) {
                file = Some(place.file);
                nodes.push(Node::Instruction(
                    Instruction::DbgFile(DMString(new.debug_file_name(place.file).into_bytes())),
                    (),
                ));
            }
            nodes.push(Node::Instruction(Instruction::DbgLine(place.line), ()));
        }
        nodes.extend(value);
        nodes.push(Node::Instruction(
            Instruction::SetVar(Variable::Global(GlobalVar::named(
                init.name.as_bytes(),
                Some(init.scope.clone()),
            ))),
            (),
        ));
        if code.locals.len() > locals.len() {
            locals = code.locals;
        }
    }
    nodes.push(Node::Instruction(Instruction::End, ()));
    Ok((nodes, locals))
}

pub(super) fn compare(
    old: &Snapshot,
    new: &Snapshot,
    r: &Retargeted,
    changes: &mut super::Changes,
) {
    let defaults: HashSet<&str> = changes
        .defaults_changed
        .iter()
        .map(String::as_str)
        .collect();
    let colon_names: HashSet<&str> = changes.colon_names.iter().map(String::as_str).collect();
    let statics = !changes.consts.is_empty()
        || !defaults.is_empty()
            && new.statics_read(|node| {
                matches!(node, syntax::Node::Follow(Follow::StaticField(name))
                    if defaults.contains(name.as_str()))
            })
        || !colon_names.is_empty()
            && new.statics_read(|node| {
                matches!(node, syntax::Node::Expression(Expression::Base { follow, .. })
                    if super::colon_fields(follow).any(|name| colon_names.contains(name)))
            })
        || changes
            .plan
            .reloads
            .iter()
            .map(|r| &r.proc)
            .chain(changes.plan.new_procs.iter().map(|p| &p.code.proc))
            .chain(changes.plan.removed_procs.iter().map(|r| &r.proc))
            .any(|key| old.has_statics(key) || new.has_statics(key));
    if !statics
        && differences(
            (old, &old.initializers(false)),
            (new, &new.initializers(false)),
            r,
        )
        .is_none()
    {
        return;
    }
    let (mut before, after) = (old.initializers(true), new.initializers(true));
    renumber_old_statics(old, new, r, &mut before);
    let Some(table) = differences((old, &before), (new, &after), r) else {
        return;
    };
    let deferred = changed(new, &before, &after);
    let table_vars = table.len();
    let without = |why: String| {
        Diagnostic::warning(
            Id::GlobalInitKept,
            format!("{why}: reboot will use the build's global and static initializers"),
        )
        .about("(global init)")
    };

    let proc_statics = proc_statics(
        table
            .iter()
            .map(|(init, _)| *init)
            .chain(after.iter().filter(|i| i.constant.is_none())),
    );
    let runs = table.into_iter().chain(
        after
            .iter()
            .filter(|i| i.constant.is_none())
            .map(|i| (i, i.expression)),
    );
    let (nodes, locals) = match build(new, runs) {
        Ok(code) => code,
        Err(why) => {
            changes.warnings.push(without(why));
            changes
                .deferred
                .extend(deferred.into_iter().map(|d| d.until(super::Until::Rebuild)));
            return;
        }
    };
    changes.deferred.extend(deferred);
    changes.global_init = Some(GlobalInit {
        code: ProcReload {
            proc: super::ProcKey {
                path: "(global init)".to_owned(),
                override_id: 0,
            },
            nodes,
            locals,
            params: vec![],
            settings: None,
            statics: vec![],
            warnings: vec![],
        },
        table: table_vars,
        proc_statics,
    });
}

impl GlobalInit {
    pub fn without(&self, mut leave: impl FnMut(&GlobalVar) -> bool) -> GlobalInit {
        let mut out = self.clone();
        out.code.nodes = vec![];
        out.table = 0;
        let (mut span, mut sets) = (vec![], 0);
        for node in &self.code.nodes {
            span.push(node.clone());
            if let Node::Instruction(Instruction::SetVar(Variable::Global(var)), _) = node {
                if leave(var) {
                    span.retain(|n| matches!(n, Node::Instruction(Instruction::DbgFile(_), _)));
                } else {
                    out.table += usize::from(sets < self.table);
                }
                out.code.nodes.append(&mut span);
                sets += 1;
            }
        }
        out.code.nodes.append(&mut span);
        out
    }
}
