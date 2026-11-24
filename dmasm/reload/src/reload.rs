use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use dreammaker::ast::{Block, Parameter, Statement};
use dreammaker::objtree::{ObjectTree, ProcValue, TypeRef};
use dreammaker::preprocessor::DefineHistory;
use dreammaker::{Context, FileId, Severity};

use crate::compiler::{
    compile_proc, definition_order, definition_order_at, CompiledProc, ProcSource,
};
use crate::diagnostic::{Diagnostic, Id};
use crate::syntax;

#[path = "reload_moved.rs"]
mod moved;
#[path = "reload_reparent.rs"]
mod reparent;
#[path = "reload_retargeted.rs"]
mod retargeted;
use crate::{ByondVersion, Instruction, Node, StaticScope};
pub use reparent::{Reparented, ReparentedType};

mod global_init;
pub mod locals;
mod resources;
pub use global_init::GlobalInit;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ProcKey {
    pub path: String,
    pub override_id: u32,
}

impl std::fmt::Display for ProcKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.override_id == 0 {
            f.write_str(&self.path)
        } else {
            write!(f, "{} (override {})", self.path, self.override_id)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ProcReload {
    pub proc: ProcKey,
    pub nodes: Vec<Node>,
    pub locals: Vec<String>,
    pub params: Vec<Param>,
    pub settings: Option<ProcSettings>,
    pub statics: Vec<StaticVar>,
    #[cfg_attr(feature = "serde", serde(default, with = "warning_texts"))]
    pub warnings: Vec<Diagnostic>,
}

#[cfg(feature = "serde")]
mod warning_texts {
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::diagnostic::{Diagnostic, Id};

    pub fn serialize<S: Serializer>(warnings: &[Diagnostic], to: S) -> Result<S::Ok, S::Error> {
        to.collect_seq(warnings.iter().map(ToString::to_string))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(from: D) -> Result<Vec<Diagnostic>, D::Error> {
        let texts = Vec::<String>::deserialize(from)?;
        Ok(texts
            .into_iter()
            .map(|text| Diagnostic::warning(Id::Text, text))
            .collect())
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct StaticVar {
    pub name: String,
    pub scope: StaticScope,
    pub value: Option<ConstValue>,
    pub init: Option<Box<ProcReload>>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub constant: bool,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Param {
    pub name: String,
    pub input: u32,
    pub input_extra: u32,
    pub in_list: InList,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum InList {
    Builtin(u32),
    Kept,
    New(Box<ProcReload>),
}

fn builtin_in_list(in_list: Option<&dreammaker::ast::Expression>) -> Option<u32> {
    use dreammaker::ast::{Expression, Follow, Term};
    const NO_RANGE: u32 = 0x7D;
    const NOT_RANGED: u32 = 0x7F;
    let Some(expression) = in_list else {
        return Some(NO_RANGE << 8 | 1);
    };
    let Expression::Base { term, follow } = expression else {
        return None;
    };
    let fields: Vec<&str> = follow
        .iter()
        .map(|f| match &f.elem {
            Follow::Field(_, name) => Some(name.as_str()),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let is_usr = |e: &Expression| matches!(e, Expression::Base { term, follow } if follow.is_empty() && matches!(&term.elem, Term::Ident(i) if i.as_str() == "usr"));
    match (&term.elem, fields.as_slice()) {
        (Term::Ident(name), []) if name.as_str() == "world" => Some(NOT_RANGED << 8 | 0x10),
        (Term::Ident(name), [] | ["contents"]) if name.as_str() == "usr" => {
            Some(NOT_RANGED << 8 | 8)
        }
        (Term::Ident(name), ["loc"]) if name.as_str() == "usr" => Some(NOT_RANGED << 8 | 3),
        (Term::Call(name, args), []) => {
            let kind = match name.as_str() {
                "view" => 1,
                "oview" => 2,
                "range" => 5,
                "orange" => 6,
                _ => return None,
            };
            let mut range = NO_RANGE;
            for arg in args.iter() {
                match arg {
                    Expression::Base { term, follow } if follow.is_empty() => match term.elem {
                        Term::Int(n) if (0..NO_RANGE as i32).contains(&n) && range == NO_RANGE => {
                            range = n as u32
                        }
                        _ if is_usr(arg) => {}
                        _ => return None,
                    },
                    _ => return None,
                }
            }
            Some(range << 8 | kind)
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ProcSettings {
    pub waitfor: bool,
    pub background: bool,
    pub name: Option<String>,
    pub desc: Option<String>,
    pub category: Option<String>,
    pub no_category: bool,
    pub hidden: bool,
    pub popup_menu: bool,
    pub instant: bool,
    pub invisibility: Option<u8>,
    pub src: Option<(bool, u32)>,
}

impl Default for ProcSettings {
    fn default() -> Self {
        ProcSettings {
            waitfor: true,
            background: false,
            name: None,
            desc: None,
            category: None,
            no_category: false,
            hidden: false,
            popup_menu: true,
            instant: false,
            invisibility: None,
            src: None,
        }
    }
}

impl ProcSettings {
    pub fn flags(&self) -> u32 {
        let mut flags = 0;
        if self.hidden {
            flags |= 0x01;
        }
        if self.src.is_some_and(|(equals, _)| equals) {
            flags |= 0x02;
        }
        if self.waitfor {
            flags |= 0x04;
        }
        if self.no_category {
            flags |= 0x20;
        }
        if let Some(invisibility) = self.invisibility {
            flags |= 0x08;
            if invisibility != 0 {
                flags |= 0x10;
            }
        }
        if !self.popup_menu {
            flags |= 0x40;
        }
        if self.background {
            flags |= 0x100;
        }
        if self.instant {
            flags |= 0x200;
        }
        if flags & 0x318 != 0 {
            flags |= 0x80;
        }
        flags
    }

    pub fn packed(&self) -> u32 {
        let (range, kind) = match self.src {
            None => (0xFF, 0),
            Some((_, word)) => (word >> 8, word & 0xFF),
        };
        let invisibility = match self.invisibility {
            None => 0,
            Some(0) => 0xFF,
            Some(n) => n as u32,
        };
        range | kind << 8 | invisibility << 16
    }

    pub const FLAGS: u32 = 0x3FF;
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SettingsChange {
    pub proc: ProcKey,
    pub settings: ProcSettings,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NewProc {
    pub type_path: String,
    pub name: String,
    pub declared: bool,
    pub verb: bool,
    pub code: ProcReload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RemovedProc {
    pub proc: ProcKey,
    pub type_path: String,
    pub name: String,
    pub verb: bool,
}

impl RemovedProc {
    pub fn warning(&self) -> Option<Diagnostic> {
        self.type_path
            .is_empty()
            .then(|| definition_retained(&self.proc.path))
    }
}

fn definition_retained(path: &str) -> Diagnostic {
    note(
        Id::RemovedProcRetained,
        path,
        "a definition was removed from source but retained in the running world",
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RemovedVar {
    pub type_path: String,
    pub var: String,
}

impl RemovedVar {
    pub fn warning(&self) -> Diagnostic {
        var_retained(Id::RemovedVarRetained, &self.type_path, &self.var)
    }
}

fn var_retained(id: Id, shown: &str, name: &str) -> Diagnostic {
    note(
        id,
        shown,
        format!("var {name} was removed from source but retained in the running world"),
    )
}

fn note(id: Id, subject: impl Into<String>, message: impl Into<String>) -> Diagnostic {
    Diagnostic::warning(id, message).about(subject)
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ConstValue {
    Null,
    Number(f32),
    String(String),
    Path(String),
    Resource(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientScript {
    Text(String),
    File(String),
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VarDefault {
    pub var: String,
    pub value: ConstValue,
    pub types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BuiltinInherited {
    pub var: String,
    pub types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LineMove {
    pub proc: ProcKey,
    pub lines: Vec<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NewType {
    pub path: String,
    pub parent: String,
    pub fields: Vec<(String, ConstValue)>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NewVar {
    pub type_path: String,
    pub var: String,
    pub flags: u32,
    pub value: ConstValue,
    pub init: Option<Box<ProcReload>>,
    pub moved_from: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Redeclared {
    pub type_path: String,
    pub var: String,
    pub flags: u32,
    pub value: Option<ConstValue>,
    pub init: Option<Box<ProcReload>>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct InitProc {
    pub type_path: String,
    pub code: Option<ProcReload>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Until {
    Reboot,
    #[default]
    Rebuild,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Refusal {
    pub id: Id,
    pub path: String,
    pub name: Option<String>,
    pub why: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub until: Until,
}

impl Refusal {
    pub fn new(id: Id, path: impl Into<String>, why: impl Into<String>) -> Self {
        Refusal {
            id,
            path: path.into(),
            name: None,
            why: why.into(),
            file: None,
            line: None,
            until: Until::Rebuild,
        }
    }

    fn until(mut self, until: Until) -> Self {
        self.until = until;
        self
    }

    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    fn at(mut self, snapshot: &Snapshot, location: dreammaker::Location) -> Self {
        if let Some((file, line)) = snapshot.source_line(location) {
            self.file = Some(file);
            self.line = Some(line);
        }
        self
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !f.alternate() {
            match (&self.file, self.line) {
                (Some(file), Some(line)) => write!(f, "{file}:{line}: ")?,
                (Some(file), None) => write!(f, "{file}: ")?,
                _ => {}
            }
        }
        if !self.path.is_empty() {
            write!(f, "{}: ", self.path)?;
        }
        if let Some(name) = &self.name {
            write!(f, "{name}: ")?;
        }
        f.write_str(&self.why)
    }
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Plan {
    pub new_types: Vec<NewType>,
    pub reloads: Vec<ProcReload>,
    pub settings: Vec<SettingsChange>,
    pub new_procs: Vec<NewProc>,
    pub removed_procs: Vec<RemovedProc>,
    pub new_vars: Vec<NewVar>,
    pub redeclared: Vec<Redeclared>,
    pub var_defaults: Vec<VarDefault>,
    pub builtin_defaults: Vec<VarDefault>,
    pub builtin_inherited: Vec<BuiltinInherited>,
    pub init_procs: Vec<InitProc>,
    pub reparented: Vec<Reparented>,
    pub line_moves: Vec<LineMove>,
    pub removed_vars: Vec<RemovedVar>,
}

impl Plan {
    pub fn len(&self) -> usize {
        self.new_types.len()
            + self.reloads.len()
            + self.settings.len()
            + self.new_procs.len()
            + self.removed_procs.len()
            + self.new_vars.len()
            + self.redeclared.len()
            + self.var_defaults.len()
            + self.builtin_defaults.len()
            + self.builtin_inherited.len()
            + self.init_procs.len()
            + self.reparented.len()
            + self.line_moves.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Default)]
pub struct Changes {
    pub plan: Plan,
    pub consts: Vec<String>,
    pub retyped: Vec<String>,
    pub warnings: Vec<Diagnostic>,
    pub refused: Vec<Refusal>,
    pub deferred: Vec<Refusal>,
    pub global_init: Option<GlobalInit>,
    restart_settings: Vec<(String, String)>,
    defaults_changed: Vec<String>,
    defaults_moved: Vec<String>,
    colon_names: Vec<String>,
    moved_procs: Vec<String>,
    new_code: Vec<(ProcKey, Block)>,
}

impl Changes {
    pub fn settings_as_built(&mut self, built: &Snapshot, new: &Snapshot) {
        let value = |s: &Snapshot, path: &str, name: &str| {
            s.find_type(path)
                .and_then(|t| t.get_value(name))
                .map(|v| v.expression.clone())
        };
        let mut kept = vec![];
        for (path, name) in std::mem::take(&mut self.restart_settings) {
            if value(built, &path, &name) == value(new, &path, &name) {
                self.deferred.retain(|d| {
                    !(d.id == Id::SettingChanged
                        && d.path == path
                        && d.name.as_deref() == Some(&name))
                });
            } else {
                kept.push((path, name));
            }
        }
        self.restart_settings = kept;
    }

    pub fn new_code(&self, key: &ProcKey) -> Option<&Block> {
        let (_, code) = self.new_code.iter().find(|(had, _)| had == key)?;
        Some(code)
    }

    fn keep_new_code<'a>(&mut self, code: impl Fn(&ProcKey) -> Option<&'a Block>) {
        self.new_code = self
            .plan
            .reloads
            .iter()
            .filter(|reload| !reload.statics.is_empty())
            .filter_map(|reload| Some((reload.proc.clone(), code(&reload.proc)?.clone())))
            .collect();
    }

    fn collect_warnings(&mut self) {
        for reload in &self.plan.reloads {
            for warning in &reload.warnings {
                self.warnings
                    .push(warning.clone().about(reload.proc.to_string()));
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.plan.is_empty() && self.refused.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub file: String,
    pub line: u32,
    pub description: String,
}

impl ParseError {
    pub fn key(&self) -> String {
        format!("{}: {}", self.file, self.description)
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.description)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownBug {
    pub id: Id,
    pub file: String,
    pub line: Option<u32>,
    pub proc: ProcKey,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KnownBugs {
    pub bugs: Vec<KnownBug>,
    pub not_checked: Vec<(ProcKey, String, String)>,
}

#[derive(Debug, Clone, Default)]
pub struct ParseOptions {
    pub defines: Vec<(String, Option<String>)>,
    pub byond: Option<(u32, u32)>,
}

const PRELUDE: &str = "<hot reload prelude>";

pub struct Snapshot {
    ctx: Context,
    tree: ObjectTree,
    procs: BTreeMap<ProcKey, (String, String, usize)>,
    defines: Option<DefineHistory>,
    stopped: bool,
    patched: HashMap<ProcKey, ProcValue>,
    byond: ByondVersion,
    root: Option<PathBuf>,
    debug_names: HashMap<PathBuf, String>,
    default_lines: std::sync::Mutex<HashMap<ProcKey, Vec<(Vec<u8>, u32)>>>,
    folded: std::collections::BTreeSet<dreammaker::Location>,
    mended: HashSet<FileId>,
    moved_defaults: Vec<(FileId, String, u32)>,
    skin: Option<PathBuf>,
    maps: Vec<PathBuf>,
    resource_index: std::sync::OnceLock<resources::Index>,
}

pub struct FileReload {
    pub changes: Changes,
    patched: Vec<(ProcKey, ProcValue)>,
    files: Vec<FileId>,
    moved: Vec<(FileId, String, u32)>,
}

struct Reparsed {
    file: FileId,
    procs: Vec<(ProcKey, ProcValue, ProcValue)>,
    moved: Vec<(String, u32)>,
}

impl Snapshot {
    pub fn parse(dme: &Path, options: &ParseOptions) -> Result<Snapshot, String> {
        let ctx = Context::default();
        if let Some((major, build)) = options.byond {
            ByondVersion::new(major, build).read_as(&ctx);
        }
        let mut pp = dreammaker::Preprocessor::new(&ctx, dme.to_owned())
            .map_err(|e| format!("couldn't read {}: {}", dme.display(), e))?;

        let mut prelude = String::from("#undef SPACEMAN_DMM\n");
        if let Some((version, build)) = options.byond {
            prelude.push_str(&format!(
                "#undef DM_VERSION\n#define DM_VERSION {version}\n#undef DM_BUILD\n#define DM_BUILD {build}\n"
            ));
        }
        for (name, value) in &options.defines {
            prelude.push_str(&format!(
                "#define {} {}\n",
                name,
                value.as_deref().unwrap_or("")
            ));
        }
        pp.push_file(
            PathBuf::from(PRELUDE),
            std::io::Cursor::new(prelude.into_bytes()),
        )
        .map_err(|e| e.to_string())?;

        let (stopped, tree) = {
            let mut parser = dreammaker::Parser::new(&ctx, &mut pp);
            parser.enable_procs();
            parser.parse_object_tree_2()
        };
        let skin = pp.skins().last().cloned();
        let maps = pp.maps().to_vec();
        let defines = pp.finalize();
        let mut snapshot = Self::from_tree(ctx, tree);
        snapshot.defines = Some(defines);
        snapshot.stopped = stopped;
        snapshot.root = dme.parent().map(Path::to_owned);
        let root = snapshot.root.clone();
        let relative = |path: PathBuf| match &root {
            Some(root) => path.strip_prefix(root).map(Path::to_owned).unwrap_or(path),
            None => path,
        };
        snapshot.skin = skin.map(relative);
        snapshot.maps = maps.into_iter().map(relative).collect();
        if snapshot.root.is_some() {
            let read: HashSet<PathBuf> = snapshot
                .files()
                .iter()
                .map(|path| crate::debug_files::normalize(path))
                .collect();
            snapshot.debug_names = crate::debug_files::names(dme, &|path| read.contains(path))
                .into_iter()
                .filter(|(name, path)| Path::new(name) != path)
                .map(|(name, path)| (path, name))
                .collect();
        }
        if let Some((major, build)) = options.byond {
            snapshot.byond = ByondVersion::new(major, build);
        }
        snapshot.fold_const_initializers();
        Ok(snapshot)
    }

    pub fn reload_file(&mut self, root: &Path, file: &Path) -> Result<FileReload, String> {
        self.reload_files(root, &[file])
    }

    pub fn reload_files(&mut self, root: &Path, files: &[&Path]) -> Result<FileReload, String> {
        let mut reparsed: Vec<Reparsed> = vec![];
        for file in files {
            let file = self.reparse(root, file)?;
            if !reparsed.iter().any(|had| had.file == file.file) {
                reparsed.push(file);
            }
        }
        let mut pairs: Vec<(&ProcKey, &ProcValue, &ProcValue)> = reparsed
            .iter()
            .flat_map(|file| &file.procs)
            .map(|(key, old_value, new_value)| (key, old_value, new_value))
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(b.0));

        let mut changes = Changes::default();
        let mut before = HashMap::new();
        for (key, old_value, new_value) in &pairs {
            if old_value.code == new_value.code {
                before.insert((*key).clone(), self.compile(key));
            }
        }
        let previous: Vec<(ProcKey, Option<ProcValue>)> = pairs
            .iter()
            .map(|(key, _, new_value)| {
                (
                    (*key).clone(),
                    self.patched.insert((*key).clone(), (*new_value).clone()),
                )
            })
            .collect();
        let moved: Vec<(FileId, String, u32)> = reparsed
            .iter()
            .flat_map(|file| {
                file.moved
                    .iter()
                    .map(move |(name, line)| (file.file, name.clone(), *line))
            })
            .collect();
        let reader = {
            let all: Vec<&(FileId, String, u32)> =
                moved.iter().chain(&self.moved_defaults).collect();
            let changed = pairs
                .iter()
                .filter(|(_, old_value, new_value)| old_value.code != new_value.code)
                .map(|(key, ..)| *key);
            self.records_moved_default(&all, changed)
        };
        if reader.is_none() {
            for (key, old_value, new_value) in &pairs {
                let recompiled = |snapshot: &Snapshot| {
                    snapshot.compile_code(key).and_then(|mut reload| {
                        snapshot.compile_in_lists(key, &mut reload, |_, _| false)?;
                        Ok(reload)
                    })
                };
                let reload = if old_value.code != new_value.code {
                    recompiled(self)
                } else if let Some(old_code) = before.remove(*key) {
                    match line_change(old_code, self.compile(key), old_value, new_value) {
                        LineChange::Same => continue,
                        LineChange::Moved(lines) => {
                            changes.plan.line_moves.push(LineMove {
                                proc: (*key).clone(),
                                lines,
                            });
                            continue;
                        }
                        LineChange::Recompile => recompiled(self),
                    }
                } else {
                    continue;
                };
                match reload {
                    Ok(reload) => changes.plan.reloads.push(reload),
                    Err(e) => changes.refused.push(self.uncompiled(key, &e)),
                }
            }
        }
        for (key, value) in previous {
            match value {
                Some(value) => self.patched.insert(key, value),
                None => self.patched.remove(&key),
            };
        }
        if let Some(key) = reader {
            return Err(format!("{key} reads a var default with :: that moved"));
        }
        changes.keep_new_code(|key| {
            let (.., new_value) = pairs.iter().find(|(had, ..)| *had == key)?;
            new_value.code.as_ref()
        });
        changes.collect_warnings();
        let files = reparsed.iter().map(|file| file.file).collect();
        Ok(FileReload {
            changes,
            patched: reparsed
                .into_iter()
                .flat_map(|file| file.procs)
                .map(|(key, _, new_value)| (key, new_value))
                .collect(),
            files,
            moved,
        })
    }

    fn reparse(&self, root: &Path, file: &Path) -> Result<Reparsed, String> {
        let history = self
            .defines
            .as_ref()
            .ok_or("snapshot has no macro history")?;
        let id = self
            .ctx
            .files()
            .get_id(file)
            .ok_or("file is not part of the snapshot")?;
        let text = std::fs::read(root.join(file)).map_err(|e| e.to_string())?;
        if String::from_utf8_lossy(&text)
            .lines()
            .any(|line| line.trim_start().starts_with("#include"))
        {
            return Err("file contains includes".to_owned());
        }

        let ctx = Context::default();
        self.byond.read_as(&ctx);
        self.ctx.files().for_each(|path| {
            ctx.register_file(path);
        });
        let mut pp = history.branch_at_file(id, &ctx);
        pp.push_file(file.to_owned(), std::io::Cursor::new(text))
            .map_err(|e| e.to_string())?;
        let tree = {
            let mut parser = dreammaker::Parser::new(&ctx, &mut pp);
            parser.enable_procs();
            parser.parse_object_tree_without_builtins()
        };
        let defines = pp.finalize();
        if let Some(e) = ctx
            .errors()
            .iter()
            .find(|e| e.severity() == Severity::Error)
        {
            return Err(format!("{}: {}", e.location().line, e.description()));
        }
        if macros_in(history, id) != macros_in(&defines, id) {
            return Err("file macros changed".to_owned());
        }

        let old = self.contents_of(id)?;
        let mut new = FileContents::default();
        for ty in tree.iter_types() {
            let path = &ty.get().path;
            if !ty.is_root() && self.tree.find(path).is_none() {
                return Err(format!("{path} is a new type"));
            }
            new.add(ty, id);
        }
        if old.vars != new.vars {
            return Err("file vars changed".to_owned());
        }
        if old.declarations != new.declarations {
            return Err("proc declarations changed".to_owned());
        }
        let moved: Vec<(&String, u32)> = old
            .var_lines
            .iter()
            .filter_map(|(k, line)| {
                new.var_lines
                    .get(k)
                    .filter(|n| *n != line)
                    .map(|_| (&k.1, *line))
            })
            .collect();
        if !moved.is_empty() {
            if let Some(key) = self.reads_moved_default(id, &moved) {
                return Err(format!("{key} reads a var default with :: that moved"));
            }
        }
        let mut pairs = vec![];
        for (proc, old_values) in &old.procs {
            let new_values = new.procs.get(proc).ok_or("proc definitions were removed")?;
            if new_values.len() != old_values.len() {
                return Err("proc definitions were added or removed".to_owned());
            }
            for ((key, old_value), (_, new_value)) in old_values.iter().zip(new_values) {
                let key = key
                    .clone()
                    .ok_or("proc definition is missing from the baseline")?;
                let defaults = |v: &ProcValue| {
                    v.parameters
                        .iter()
                        .map(|p| p.default.clone())
                        .collect::<Vec<_>>()
                };
                if signature(old_value) != signature(new_value)
                    || defaults(old_value) != defaults(new_value)
                {
                    return Err(format!("{key}: parameters changed"));
                }
                if settings(old_value.code.as_ref()) != settings(new_value.code.as_ref()) {
                    return Err(format!("{key}: `set`s changed"));
                }
                fn statics(v: &ProcValue) -> Vec<(&str, Option<&dreammaker::ast::Expression>)> {
                    v.code
                        .as_deref()
                        .map(global_init::static_initializers)
                        .unwrap_or_default()
                }
                if old_value.code != new_value.code && statics(old_value) != statics(new_value) {
                    return Err(format!("{key}: static initializers changed"));
                }
                pairs.push((key, (*old_value).clone(), (*new_value).clone()));
            }
        }
        if new.procs.len() != old.procs.len() {
            return Err("proc definitions were added".to_owned());
        }
        let relocated: BTreeMap<dreammaker::Location, dreammaker::Location> = pairs
            .iter()
            .filter_map(|(key, _, new_value)| {
                let (ty, name, index) = self.procs.get(key)?;
                let built = self.find_type(ty)?.get().procs.get(name.as_str())?;
                Some((built.value.get(*index)?.location, new_value.location))
            })
            .collect();
        for (ty, name) in old.procs.keys() {
            let Some(ty) = self.find_type(ty) else {
                continue;
            };
            let moved = definition_order_at(ty, name, |l| relocated.get(&l).copied().unwrap_or(l));
            if moved != definition_order(ty, name) {
                return Err(format!(
                    "{}: the order of its definitions changed",
                    proc_path(ty, name)
                ));
            }
        }

        Ok(Reparsed {
            file: id,
            procs: pairs,
            moved: moved
                .into_iter()
                .map(|(name, line)| (name.clone(), line))
                .collect(),
        })
    }

    pub fn keep(&mut self, reload: FileReload) {
        let default_lines = self.default_lines.get_mut().unwrap();
        for (key, _) in &reload.patched {
            default_lines.remove(key);
        }
        self.patched.extend(reload.patched);
        self.mended.extend(reload.files);
        for moved in reload.moved {
            if !self.moved_defaults.contains(&moved) {
                self.moved_defaults.push(moved);
            }
        }
        self.resource_index = Default::default();
    }

    pub fn has_patches(&self) -> bool {
        !self.patched.is_empty() || !self.mended.is_empty()
    }

    pub fn without_patches<R>(&mut self, f: impl FnOnce(&Snapshot) -> R) -> R {
        let patched = std::mem::take(&mut self.patched);
        let result = f(self);
        self.patched = patched;
        result
    }

    pub fn clear_patches(&mut self) {
        self.patched.clear();
        self.mended.clear();
        self.moved_defaults.clear();
        self.resource_index = Default::default();
    }

    fn contents_of(&self, file: FileId) -> Result<FileContents<'_>, String> {
        let mut keys: HashMap<(&str, &str, usize), &ProcKey> = HashMap::new();
        for (key, (ty, name, index)) in &self.procs {
            keys.insert((ty.as_str(), name.as_str(), *index), key);
        }
        let mut contents = FileContents::default();
        for ty in self.tree.iter_types() {
            contents.add(ty, file);
            let path = &ty.get().path;
            for (name, proc) in ty.get().procs.iter() {
                let Some(values) = contents.procs.get_mut(&(path.clone(), name.to_string())) else {
                    continue;
                };
                for (key, value) in values.iter_mut() {
                    let index = proc.value.iter().position(|v| std::ptr::eq(v, *value));
                    let found =
                        index.and_then(|i| keys.get(&(path.as_str(), name.as_str(), i)).copied());
                    *key = found.cloned();
                    if let Some(patched) = found.and_then(|k| self.patched.get(k)) {
                        *value = patched;
                    }
                }
            }
        }
        Ok(contents)
    }

    pub fn from_tree(ctx: Context, mut tree: ObjectTree) -> Snapshot {
        crate::compiler::new_tree();
        crate::compiler::resolve_proc_references(&mut tree);
        let mut procs = BTreeMap::new();
        for ty in tree.iter_types() {
            for name in ty.get().procs.keys() {
                for (override_id, index) in
                    definition_order(ty, name.as_str()).into_iter().enumerate()
                {
                    let key = ProcKey {
                        path: proc_path(ty, name.as_str()),
                        override_id: override_id as u32,
                    };
                    procs.insert(
                        key,
                        (ty.get().path.clone(), name.as_str().to_owned(), index),
                    );
                }
            }
        }
        Snapshot {
            ctx,
            tree,
            procs,
            defines: None,
            stopped: false,
            patched: HashMap::new(),
            byond: ByondVersion::LATEST,
            root: None,
            debug_names: HashMap::new(),
            default_lines: std::sync::Mutex::new(HashMap::new()),
            folded: Default::default(),
            mended: Default::default(),
            moved_defaults: vec![],
            skin: None,
            maps: vec![],
            resource_index: Default::default(),
        }
    }

    fn fold_const_initializers(&mut self) {
        let described: Vec<(dreammaker::Location, String)> = self
            .ctx
            .errors()
            .iter()
            .filter(|e| e.severity() == Severity::Error)
            .map(|e| (e.location(), e.description().to_owned()))
            .collect();
        let errors: Vec<dreammaker::Location> = described.iter().map(|(at, _)| *at).collect();
        if errors.is_empty() {
            return;
        }
        let mut names: HashSet<String> = HashSet::new();
        loop {
            let mut found = vec![];
            for ty in self.tree.iter_types() {
                for (name, var) in ty.get().vars.iter() {
                    let location = var.value.location;
                    if var.value.constant.is_some()
                        || !errors.contains(&location)
                        || self.folded.contains(&location)
                    {
                        continue;
                    }
                    let Some(expression) = var.value.expression.as_ref() else {
                        continue;
                    };
                    let is_const = ty
                        .get_var_declaration(name.as_str())
                        .is_some_and(|d| d.var_type.flags.is_const());
                    if !is_const {
                        continue;
                    }
                    if let Some(constant) =
                        crate::compiler::fold_initializer(&self.tree, ty, expression, self.byond)
                    {
                        found.push((ty.index(), name.clone(), constant, location));
                    }
                }
            }
            if found.is_empty() {
                break;
            }
            for (ty, name, constant, location) in found {
                if let Some(var) = self.tree[ty].vars.get_mut(&name) {
                    var.value.constant = Some(constant);
                }
                self.folded.insert(location);
                names.insert(name.to_string());
            }
        }
        for (location, description) in described {
            let of = description.strip_prefix("recursive constant reference: ");
            if of.is_some_and(|name| names.contains(name)) {
                self.folded.insert(location);
            }
        }
    }

    pub fn byond(&self) -> ByondVersion {
        self.byond
    }

    pub fn set_byond(&mut self, byond: ByondVersion) {
        self.byond = byond;
    }

    pub fn tree(&self) -> &ObjectTree {
        &self.tree
    }

    pub fn context(&self) -> &Context {
        &self.ctx
    }

    pub fn macro_history(&self) -> Option<&DefineHistory> {
        self.defines.as_ref().filter(|_| !self.stopped)
    }

    pub fn files(&self) -> Vec<PathBuf> {
        let mut files = vec![];
        self.ctx.files().for_each(|path| {
            if path != Path::new(PRELUDE) {
                files.push(path.to_owned());
            }
        });
        files
    }

    pub fn skin(&self) -> Option<&Path> {
        self.skin.as_deref()
    }

    pub fn maps(&self) -> &[PathBuf] {
        &self.maps
    }

    pub fn client_script(&self) -> Result<Option<ClientScript>, String> {
        let Some(value) = self
            .find_type("/client")
            .and_then(|client| default_value(client, "script"))
        else {
            return Ok(None);
        };
        match value {
            Some(ConstValue::String(text)) => Ok(Some(ClientScript::Text(text))),
            Some(ConstValue::Resource(file)) => Ok(Some(ClientScript::File(file))),
            Some(ConstValue::Null) => Ok(None),
            _ => Err("/client: script must be constant text or a resource file".to_owned()),
        }
    }

    pub fn procs(&self) -> impl Iterator<Item = &ProcKey> {
        self.procs.keys()
    }

    pub fn proc_file(&self, key: &ProcKey) -> Option<String> {
        let (_, value) = self.find(key)?;
        Some(self.file_name(value.location.file))
    }

    pub fn proc_code(&self, key: &ProcKey) -> Option<&Block> {
        self.find(key)?.1.code.as_ref()
    }

    pub fn proc_parameters(&self, key: &ProcKey) -> Option<&[Parameter]> {
        Some(&self.find(key)?.1.parameters)
    }

    pub fn compile_reload(&self, key: &ProcKey) -> Result<ProcReload, Diagnostic> {
        let mut reload = self.compile_code(key)?;
        reload.settings = Some(
            self.settings(key)
                .map_err(|e| Diagnostic::error(Id::UnsupportedSetting, e))?,
        );
        Ok(reload)
    }

    fn compile_code(&self, key: &ProcKey) -> Result<ProcReload, Diagnostic> {
        let (_, value) = self.find(key).ok_or_else(|| no_proc(key))?;
        let compiled = self.compile(key)?;
        let (ty, _) = self.find(key).ok_or_else(|| no_proc(key))?;
        let mut statics = vec![];
        let mut earlier = vec![];
        for s in compiled.statics {
            let constant = s.value.as_ref().and_then(const_value);
            let init = match (&constant, &s.expression) {
                (None, Some(expression)) => Some(Box::new(self.compile_value(
                    ty,
                    expression,
                    &s.type_path,
                    value_location(expression, value.location),
                    static_proc_path(&s.scope),
                    &earlier,
                )?)),
                _ => None,
            };
            earlier.push((s.name.clone(), static_number(&s.scope)));
            statics.push(StaticVar {
                name: s.name,
                value: constant,
                scope: s.scope,
                init,
                constant: s.constant,
            });
        }
        Ok(ProcReload {
            proc: key.clone(),
            nodes: compiled.nodes,
            locals: compiled.locals,
            params: params(value),
            settings: None,
            statics,
            warnings: compiled.warnings,
        })
    }

    fn compile_in_lists(
        &self,
        key: &ProcKey,
        reload: &mut ProcReload,
        new: impl Fn(&str, &dreammaker::ast::Expression) -> bool,
    ) -> Result<(), Diagnostic> {
        let (ty, value) = self.find(key).ok_or_else(|| no_proc(key))?;
        for param in reload.params.iter_mut() {
            if param.in_list != InList::Kept {
                continue;
            }
            let Some(expression) = value
                .parameters
                .iter()
                .find(|p| p.name.as_str() == param.name)
                .and_then(|p| p.in_list.as_ref())
            else {
                continue;
            };
            if new(&param.name, expression) {
                let location = value_location(expression, value.location);
                let code = self
                    .compile_in_list(ty, expression, location)
                    .map_err(|e| {
                        Diagnostic::error(
                            e.id,
                            format!("parameter {}'s `in` list: {e}", param.name),
                        )
                    })?;
                param.in_list = InList::New(Box::new(code));
            }
        }
        Ok(())
    }

    fn compile_in_list(
        &self,
        ty: TypeRef,
        expression: &dreammaker::ast::Expression,
        location: dreammaker::Location,
    ) -> Result<ProcReload, Diagnostic> {
        use dreammaker::ast::{Spanned, Statement};
        let block: Block = Box::new([Spanned::new(
            location,
            Statement::Return(Some(expression.clone())),
        )]);
        self.compile_synthetic(ty, "(in list)", block, location, true, None, &[])
    }

    pub(crate) fn compile_value(
        &self,
        ty: TypeRef,
        expression: &dreammaker::ast::Expression,
        declared: &str,
        location: dreammaker::Location,
        proc_path: Option<&str>,
        statics: &[(String, u32)],
    ) -> Result<ProcReload, Diagnostic> {
        use dreammaker::ast::{Expression, Ident, PathOp, Prefab, RelativePath, Spanned, Term};
        let mut expression = expression.clone();
        if let Expression::Base { term, .. } = &mut expression {
            if let Term::NewImplicit { args } = &term.elem {
                let declared = if declared.trim_start_matches('/').starts_with("list/") {
                    "list"
                } else {
                    declared
                };
                let mut parts = declared.split('/').filter(|part| !part.is_empty());
                let Some(first) = parts.next() else {
                    return Err(Diagnostic::error(
                        Id::ImplicitNewUntyped,
                        "implicit new requires a declared variable type",
                    ));
                };
                let mut path = RelativePath::single(PathOp::Slash, Ident::from_nonstatic(first));
                for part in parts {
                    path.push((PathOp::Slash, Ident::from_nonstatic(part)));
                }
                term.elem = Term::NewPrefab {
                    prefab: Box::new(Prefab::from(path)),
                    args: args.clone(),
                };
            }
        }
        let block: Block = Box::new([Spanned::new(location, Statement::Return(Some(expression)))]);
        self.compile_synthetic(
            ty,
            "(value)",
            block,
            ty.get().location,
            false,
            proc_path,
            statics,
        )
    }

    fn compile_init(&self, ty: TypeRef, inits: &[Init]) -> Result<ProcReload, Diagnostic> {
        use dreammaker::ast::{
            AssignOp, Expression, Follow, Ident, PropertyAccessKind, Spanned, Term,
        };
        let block: Block = inits
            .iter()
            .map(|init| {
                let location = init.location;
                let lhs = Expression::Base {
                    term: Box::new(Spanned::new(
                        location,
                        Term::Ident(Ident::from_nonstatic("src")),
                    )),
                    follow: Box::new([Spanned::new(
                        location,
                        Follow::Field(PropertyAccessKind::Dot, Ident::from_nonstatic(&init.var)),
                    )]),
                };
                Spanned::new(
                    location,
                    Statement::Expr(Expression::AssignOp {
                        op: AssignOp::Assign,
                        lhs: Box::new(lhs),
                        rhs: Box::new(init.value.clone()),
                    }),
                )
            })
            .collect();
        self.compile_synthetic(ty, "(init)", block, ty.get().location, false, None, &[])
    }

    fn compile_synthetic(
        &self,
        ty: TypeRef,
        name: &str,
        block: Block,
        location: dreammaker::Location,
        debug_info: bool,
        proc_path: Option<&str>,
        statics: &[(String, u32)],
    ) -> Result<ProcReload, Diagnostic> {
        let value = ProcValue {
            location,
            parameters: Box::new([]),
            docs: Default::default(),
            code: Some(block),
            body_range: None,
            first_line: None,
            next_lines: Box::new([]),
            end_lines: Box::new([]),
            operator_lines: Box::new([]),
        };
        let file = self.debug_file_name(location.file);
        let source = ProcSource {
            tree: &self.tree,
            ty,
            name,
            value: &value,
            file: &file,
            debug_info,
            index: None,
            byond: self.byond,
            file_names: Some(&|id| self.debug_file_name(id)),
        };
        let path = format!("{}/{}", ty.get().path, name);
        let compiled = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::compiler::compile_proc_as(&source, proc_path, statics)
        })) {
            Ok(Ok(compiled)) => compiled,
            Ok(Err(e)) => return Err(e.diagnostic()),
            Err(_) => return Err(panicked()),
        };
        Ok(ProcReload {
            proc: ProcKey {
                path,
                override_id: 0,
            },
            nodes: compiled.nodes,
            locals: compiled.locals,
            params: vec![],
            settings: Some(ProcSettings::default()),
            statics: vec![],
            warnings: compiled.warnings,
        })
    }

    pub fn settings(&self, key: &ProcKey) -> Result<ProcSettings, String> {
        self.settings_with(key, &mut DeclaredSettings::new())
    }

    fn settings_with(
        &self,
        key: &ProcKey,
        declared: &mut DeclaredSettings,
    ) -> Result<ProcSettings, String> {
        let (ty, name, index) = self
            .procs
            .get(key)
            .ok_or_else(|| format!("no proc {key}"))?;
        let ty = self.find_type(ty).ok_or_else(|| format!("no proc {key}"))?;
        settings_of(ty, name, *index, declared)
    }

    pub fn compile(&self, key: &ProcKey) -> Result<CompiledProc, Diagnostic> {
        let (ty, value) = self.find(key).ok_or_else(|| no_proc(key))?;
        let (_, name, index) = &self.procs[key];
        let file = self.debug_file_name(value.location.file);
        let source = ProcSource {
            tree: &self.tree,
            ty,
            name,
            value,
            file: &file,
            debug_info: true,
            index: Some(*index),
            byond: self.byond,
            file_names: Some(&|id| self.debug_file_name(id)),
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compile_proc(&source))) {
            Ok(Ok(compiled)) => Ok(compiled),
            Ok(Err(e)) => Err(e.diagnostic()),
            Err(_) => Err(panicked()),
        }
    }

    pub fn known_bugs(&self) -> KnownBugs {
        let mut found = KnownBugs::default();
        for key in self.procs() {
            let file = self.proc_file(key).unwrap_or_default();
            match self.compile(key) {
                Ok(compiled) => found
                    .bugs
                    .extend(compiled.byond_bugs.into_iter().map(|bug| KnownBug {
                        id: bug.id,
                        file: file.clone(),
                        line: bug.line,
                        proc: key.clone(),
                        text: bug.text,
                    })),
                Err(error) => found
                    .not_checked
                    .push((key.clone(), file, error.to_string())),
            }
        }
        found
            .bugs
            .sort_by(|a, b| (&a.file, a.line, &a.proc).cmp(&(&b.file, b.line, &b.proc)));
        found.not_checked.sort();
        found
    }

    pub fn parse_errors(&self) -> Vec<ParseError> {
        self.unresolved_errors()
            .into_iter()
            .map(|(location, description)| ParseError {
                file: self.file_name(location.file),
                line: location.line,
                description,
            })
            .collect()
    }

    fn parse_refusals(&self) -> Vec<(String, Refusal)> {
        self.unresolved_errors()
            .into_iter()
            .map(|(location, description)| {
                let file = self.file_name(location.file);
                let why = format!("parse error: {description}");
                (
                    format!("{file}: {description}"),
                    Refusal::new(Id::ParseError, "", why).at(self, location),
                )
            })
            .collect()
    }

    fn unresolved_errors(&self) -> Vec<(dreammaker::Location, String)> {
        self.ctx
            .errors()
            .iter()
            .filter(|e| {
                e.severity() == Severity::Error
                    && !self.folded.contains(&e.location())
                    && !self.mended.contains(&e.location().file)
            })
            .map(|e| (e.location(), e.description().to_owned()))
            .collect()
    }

    fn source_line(&self, location: dreammaker::Location) -> Option<(String, u32)> {
        if location.file == FileId::BUILTINS
            || location.file == FileId::UNKNOWN
            || location.line == 0
        {
            return None;
        }
        let file = self.file_name(location.file);
        (file != PRELUDE).then_some((file, location.line))
    }

    fn type_location(&self, t: TypeRef) -> dreammaker::Location {
        let ty = t.get();
        let in_source = |l: &dreammaker::Location| self.source_line(*l).is_some();
        std::iter::once(ty.location)
            .chain(ty.vars.values().map(|v| v.value.location))
            .chain(
                ty.procs
                    .values()
                    .flat_map(|p| p.value.iter().map(|v| v.location)),
            )
            .find(in_source)
            .unwrap_or(ty.location)
    }

    fn var_location(&self, t: TypeRef, name: &str) -> dreammaker::Location {
        match t.get().vars.get(name) {
            Some(var) if self.source_line(var.value.location).is_some() => var.value.location,
            Some(var) => var
                .declaration
                .as_ref()
                .map(|d| d.location)
                .filter(|l| self.source_line(*l).is_some())
                .unwrap_or_else(|| self.type_location(t)),
            None => self.type_location(t),
        }
    }

    fn proc_refusal(&self, id: Id, key: &ProcKey, why: impl Into<String>) -> Refusal {
        let refusal = Refusal::new(id, key.to_string(), why);
        match self.find(key) {
            Some((_, value)) => refusal.at(self, value.location),
            None => refusal,
        }
    }

    fn uncompiled(&self, key: &ProcKey, e: &Diagnostic) -> Refusal {
        let refusal = self.proc_refusal(e.id, key, format!("doesn't compile: {e}"));
        match (&e.span, &e.subject) {
            (Some(span), None) if span.file.is_none() && refusal.file.is_some() => Refusal {
                why: format!("doesn't compile: {}", e.message),
                line: Some(span.line),
                ..refusal
            },
            _ => refusal,
        }
    }

    fn find(&self, key: &ProcKey) -> Option<(TypeRef<'_>, &ProcValue)> {
        let (ty, name, index) = self.procs.get(key)?;
        let ty = self.find_type(ty)?;
        if let Some(value) = self.patched.get(key) {
            return Some((ty, value));
        }
        let value = ty.get().procs.get(name.as_str())?.value.get(*index)?;
        Some((ty, value))
    }

    fn find_type(&self, path: &str) -> Option<TypeRef<'_>> {
        if path.is_empty() {
            Some(self.tree.root())
        } else {
            self.tree.find(path)
        }
    }

    fn reads_moved_default(&self, file: FileId, moved: &[(&String, u32)]) -> Option<ProcKey> {
        let file_name = self.debug_file_name(file).into_bytes();
        let names: HashSet<&str> = moved.iter().map(|(name, _)| name.as_str()).collect();
        let candidates: Vec<ProcKey> = self
            .procs
            .keys()
            .filter(|key| {
                self.find(key)
                    .is_some_and(|(_, value)| names_any(value, &HashSet::new(), &names))
            })
            .cloned()
            .collect();
        let mut default_lines = self.default_lines.lock().unwrap();
        for key in candidates {
            if !default_lines.contains_key(&key) {
                let lines = self.recorded_defaults(&key);
                default_lines.insert(key.clone(), lines);
            }
            if default_lines[&key]
                .iter()
                .any(|(f, line)| *f == file_name && moved.iter().any(|(_, old)| old == line))
            {
                return Some(key);
            }
        }
        None
    }

    fn records_moved_default<'k>(
        &self,
        moved: &[&(FileId, String, u32)],
        mut keys: impl Iterator<Item = &'k ProcKey>,
    ) -> Option<ProcKey> {
        if moved.is_empty() {
            return None;
        }
        let names: HashSet<&str> = moved.iter().map(|(_, name, _)| name.as_str()).collect();
        let places: Vec<(Vec<u8>, u32)> = moved
            .iter()
            .map(|(file, _, line)| (self.debug_file_name(*file).into_bytes(), *line))
            .collect();
        keys.find(|key| {
            let Some((_, value)) = self.find(key) else {
                return false;
            };
            if !names_any(value, &HashSet::new(), &names) {
                return false;
            }
            let Ok(code) = self.compile(key) else {
                return false;
            };
            let mut at = self.debug_file_name(value.location.file).into_bytes();
            code.nodes.iter().any(|node| match node {
                Node::Instruction(Instruction::DbgFile(name), _) => {
                    at = name.0.clone();
                    false
                }
                Node::Instruction(Instruction::DbgLine(line), _) => places
                    .iter()
                    .any(|(file, moved)| *file == at && moved == line),
                _ => false,
            })
        })
        .cloned()
    }

    fn recorded_defaults(&self, key: &ProcKey) -> Vec<(Vec<u8>, u32)> {
        let (Some((_, value)), Ok(code)) = (self.find(key), self.compile(key)) else {
            return vec![];
        };
        let own_file = self.debug_file_name(value.location.file).into_bytes();
        let own = proc_lines(value);
        let mut file = own_file.clone();
        let mut lines = vec![];
        for node in &code.nodes {
            match node {
                Node::Instruction(Instruction::DbgFile(name), _) => file = name.0.clone(),
                Node::Instruction(Instruction::DbgLine(line), _)
                    if !(file == own_file && own.contains(line)) =>
                {
                    lines.push((file.clone(), *line))
                }
                _ => {}
            }
        }
        lines
    }

    fn place(&self, location: dreammaker::Location) -> (String, u32, u16) {
        (
            self.file_name(location.file),
            location.line,
            location.column,
        )
    }

    fn file_name(&self, file: FileId) -> String {
        self.ctx
            .file_path(file)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn debug_file_name(&self, file: FileId) -> String {
        let path = self.ctx.file_path(file);
        match self.debug_names.get(&crate::debug_files::normalize(&path)) {
            Some(name) => name.clone(),
            None => self.file_name(file),
        }
    }
}

fn proc_path(ty: TypeRef, name: &str) -> String {
    format!("{}/{}", ty.get().path, name)
}

pub fn diff(old: &Snapshot, new: &Snapshot, changed_files: Option<&HashSet<PathBuf>>) -> Changes {
    let mut changes = Changes::default();

    let mut old_errors: HashMap<String, usize> = HashMap::new();
    for error in old.parse_errors() {
        *old_errors.entry(error.key()).or_default() += 1;
    }
    for (e, refusal) in new.parse_refusals() {
        match old_errors.get_mut(&e) {
            Some(n) if *n > 0 => *n -= 1,
            _ => changes.refused.push(refusal),
        }
    }
    if !changes.refused.is_empty() {
        return changes;
    }

    let mut realigned = Realigned::new();
    let new_procs = compare_types(old, new, &mut changes, &mut realigned);
    let old_key = |key: &ProcKey| old_key(&realigned, key);
    let depth = |path: &str| {
        let mut depth = 0;
        let mut at = new.find_type(path);
        while let Some(t) = at {
            depth += 1;
            at = t.parent_type();
        }
        depth
    };
    changes
        .plan
        .new_types
        .sort_by_cached_key(|t| depth(&t.path));
    let retargeted = retargeted::retargeted(old, new, &realigned, &changes);
    compare_inits(old, new, &retargeted, &mut changes);
    retargeted::path_defaults(&retargeted, &mut changes);
    for (key, type_path, name, declared, verb) in new_procs {
        let code = new.compile_reload(&key).and_then(|mut code| {
            new.compile_in_lists(&key, &mut code, |_, _| true)?;
            Ok(code)
        });
        match code {
            Ok(code) => changes.plan.new_procs.push(NewProc {
                type_path,
                name,
                declared,
                verb,
                code,
            }),
            Err(e) => changes.refused.push(new.uncompiled(&key, &e)),
        }
    }

    let in_changed_file = |key: &ProcKey| match changed_files {
        None => true,
        Some(files) => new
            .proc_file(key)
            .map_or(false, |f| files.contains(Path::new(&f))),
    };

    let (mut old_declared, mut new_declared) = (DeclaredSettings::new(), DeclaredSettings::new());
    for (key, _) in new.procs.iter() {
        let Some(old_key) = old_key(key) else {
            continue;
        };
        let (Some((_, old_value)), Some((_, new_value))) = (old.find(&old_key), new.find(key))
        else {
            continue;
        };

        let moved_file =
            old.file_name(old_value.location.file) != new.file_name(new_value.location.file);
        let defaults = |v: &ProcValue| {
            v.parameters
                .iter()
                .map(|p| p.default.clone())
                .collect::<Vec<_>>()
        };
        let params_changed = signature(old_value) != signature(new_value);
        let settings = new.settings_with(key, &mut new_declared);
        let settings = match (
            old.settings_with(&old_key, &mut old_declared) == settings,
            settings,
        ) {
            (true, _) => None,
            (false, Ok(settings)) => Some(settings),
            (false, Err(e)) => {
                changes
                    .refused
                    .push(new.proc_refusal(Id::UnsupportedSetting, key, e));
                continue;
            }
        };
        if old_value.code != new_value.code
            || defaults(old_value) != defaults(new_value)
            || params_changed
            || moved_file
        {
            let new_list = |name: &str, expression: &dreammaker::ast::Expression| {
                old_value
                    .parameters
                    .iter()
                    .find(|p| p.name.as_str() == name)
                    .and_then(|p| p.in_list.as_ref())
                    != Some(expression)
            };
            let reload = new.compile_code(key).and_then(|mut reload| {
                new.compile_in_lists(key, &mut reload, new_list)?;
                Ok(reload)
            });
            match reload {
                Ok(reload) => changes.plan.reloads.push(ProcReload {
                    settings,
                    proc: old_key,
                    ..reload
                }),
                Err(e) => changes.refused.push(new.uncompiled(key, &e)),
            }
            continue;
        }
        if let Some(settings) = settings {
            changes.plan.settings.push(SettingsChange {
                proc: old_key.clone(),
                settings,
            });
        }

        if in_changed_file(key) {
            match line_move(old, new, &old_key, key, old_value, new_value) {
                LineChange::Same => {}
                LineChange::Moved(lines) => changes.plan.line_moves.push(LineMove {
                    proc: old_key,
                    lines,
                }),
                LineChange::Recompile => match new.compile_code(key).and_then(|mut reload| {
                    new.compile_in_lists(key, &mut reload, |_, _| false)?;
                    Ok(reload)
                }) {
                    Ok(reload) => changes.plan.reloads.push(ProcReload {
                        settings: None,
                        proc: old_key,
                        ..reload
                    }),
                    Err(e) => changes.refused.push(new.uncompiled(key, &e)),
                },
            }
        }
    }

    const_users(old, new, &realigned, &mut changes);
    reparent::reparented_users(old, new, &realigned, &mut changes);
    moved::moved_users(old, new, &realigned, &mut changes);
    retargeted::path_users(&retargeted, &mut changes);
    local_const_users(old, new, &realigned, &mut changes);
    global_init::compare(old, new, &retargeted, &mut changes);
    changes.keep_new_code(|key| new.proc_code(&new_key(&realigned, key)?));
    changes.collect_warnings();
    changes
}

fn new_key(realigned: &Realigned, old: &ProcKey) -> Option<ProcKey> {
    match realigned.get(&old.path) {
        None => Some(old.clone()),
        Some(map) => map
            .iter()
            .position(|id| *id == Some(old.override_id))
            .map(|at| ProcKey {
                path: old.path.clone(),
                override_id: at as u32,
            }),
    }
}

type Realigned = HashMap<String, Vec<Option<u32>>>;

fn old_key(realigned: &Realigned, key: &ProcKey) -> Option<ProcKey> {
    match realigned.get(&key.path) {
        None => Some(key.clone()),
        Some(map) => map
            .get(key.override_id as usize)
            .copied()
            .flatten()
            .map(|override_id| ProcKey {
                path: key.path.clone(),
                override_id,
            }),
    }
}

fn names_any(value: &ProcValue, names: &HashSet<&str>, defaults: &HashSet<&str>) -> bool {
    use dreammaker::ast::Follow;
    let mut found = false;
    syntax::walk_proc(value, &mut |node| {
        found |= match node {
            syntax::Node::Name(name) => names.contains(name),
            syntax::Node::Follow(Follow::StaticField(name)) => defaults.contains(name.as_str()),
            _ => false,
        }
    });
    found
}

pub(crate) fn colon_fields(
    follow: &[dreammaker::ast::Spanned<dreammaker::ast::Follow>],
) -> impl Iterator<Item = &str> {
    use dreammaker::ast::{Follow, PropertyAccessKind};
    let after_index =
        std::iter::once(false).chain(follow.iter().map(|f| matches!(f.elem, Follow::Index(..))));
    follow
        .iter()
        .zip(after_index)
        .filter_map(|(f, after_index)| match &f.elem {
            Follow::Field(PropertyAccessKind::Colon | PropertyAccessKind::SafeColon, name) => {
                Some(name.as_str())
            }
            Follow::Field(kind, name) if after_index && *kind != PropertyAccessKind::Scope => {
                Some(name.as_str())
            }
            _ => None,
        })
}

fn colon_reads_any(value: &ProcValue, names: &HashSet<&str>) -> bool {
    use dreammaker::ast::Expression;
    if names.is_empty() {
        return false;
    }
    let mut found = false;
    syntax::walk_proc(value, &mut |node| {
        if let syntax::Node::Expression(Expression::Base { follow, .. }) = node {
            found |= colon_fields(follow).any(|name| names.contains(name));
        }
    });
    found
}

fn const_users(old: &Snapshot, new: &Snapshot, realigned: &Realigned, changes: &mut Changes) {
    if changes.consts.is_empty()
        && changes.retyped.is_empty()
        && changes.defaults_changed.is_empty()
        && changes.defaults_moved.is_empty()
        && changes.colon_names.is_empty()
    {
        return;
    }
    let names: HashSet<&str> = changes
        .consts
        .iter()
        .chain(&changes.retyped)
        .map(String::as_str)
        .collect();
    let colon_names: HashSet<&str> = changes.colon_names.iter().map(String::as_str).collect();
    let consts: HashSet<&str> = changes
        .consts
        .iter()
        .chain(&changes.colon_names)
        .map(String::as_str)
        .collect();
    let moved: HashSet<&str> = changes
        .defaults_changed
        .iter()
        .chain(&changes.defaults_moved)
        .map(String::as_str)
        .collect();
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
        if reloaded.contains(&old_key) {
            continue;
        }
        let (Some(_), Some((new_ty, new_value))) = (old.find(&old_key), new.find(key)) else {
            continue;
        };
        if !names_any(new_value, &names, &moved) && !colon_reads_any(new_value, &colon_names) {
            continue;
        }
        let compiled = match (old.compile(&old_key), new.compile(key)) {
            (Ok(before), Ok(after)) => Some((before, after)),
            (_, after) => {
                let path = &new_ty.get().path;
                let why = if folds_changed(old, new, path, new_value, &consts) {
                    "folds a const whose value changes"
                } else if colon_rebinds(old, new, path, new_value, &colon_names) {
                    "reads a var by `x:name` that another type declares now"
                } else {
                    continue;
                };
                if let Err(e) = after {
                    let refusal = new.uncompiled(key, &e);
                    changes.refused.push(Refusal {
                        why: format!(
                            "{why}, so it must be compiled again, but it {}",
                            refusal.why
                        ),
                        ..refusal
                    });
                    continue;
                }
                None
            }
        };
        let code = |nodes: &[Node]| -> Vec<Node> {
            nodes
                .iter()
                .filter(|n| !matches!(n, Node::Instruction(Instruction::DbgLine(_), _)))
                .cloned()
                .collect()
        };
        if let Some((before, after)) = compiled.filter(|(b, a)| code(&b.nodes) == code(&a.nodes)) {
            match line_map(&before.nodes, &after.nodes) {
                Some(lines) => {
                    let lines: Vec<(u32, u32)> = lines.into_iter().collect();
                    match changes
                        .plan
                        .line_moves
                        .iter_mut()
                        .find(|m| m.proc == old_key)
                    {
                        Some(found) => found.lines = lines,
                        None if lines.iter().any(|(o, n)| o != n) => {
                            changes.plan.line_moves.push(LineMove {
                                proc: old_key,
                                lines,
                            })
                        }
                        None => {}
                    }
                    continue;
                }
                None => {}
            }
        }
        let reload = new.compile_code(key).and_then(|mut reload| {
            new.compile_in_lists(key, &mut reload, |_, _| false)?;
            Ok(reload)
        });
        changes.plan.line_moves.retain(|m| m.proc != old_key);
        match reload {
            Ok(reload) => changes.plan.reloads.push(ProcReload {
                proc: old_key,
                ..reload
            }),
            Err(e) => changes.refused.push(new.uncompiled(key, &e)),
        }
    }
}

fn local_consts<'a>(
    s: &Snapshot,
    ty: TypeRef,
    value: &'a ProcValue,
) -> Vec<(&'a str, usize, Option<crate::compiler::Folded>)> {
    use crate::compiler::InScope;
    let mut statics: Vec<&str> = vec![];
    let mut folded: HashMap<usize, crate::compiler::Folded> = HashMap::new();
    let mut out = vec![];
    retargeted::walk_scopes(value, &mut |var, scopes| {
        let flags = var.var_type.flags;
        if !flags.is_static() && !flags.is_const() {
            return;
        }
        let name = var.name.as_str();
        let number = statics.iter().filter(|n| **n == name).count();
        statics.push(name);
        if !flags.is_const() {
            return;
        }
        let in_scope: Vec<InScope> = scopes
            .iter()
            .enumerate()
            .flat_map(|(depth, scope)| scope.iter().map(move |d| (depth, d)))
            .map(|(depth, d)| match d.var {
                None if depth == 0 => InScope::Param(d.name, &d.ty),
                Some(v) if v.var_type.flags.is_static() || v.var_type.flags.is_const() => {
                    let at = v as *const dreammaker::ast::VarStatement as usize;
                    InScope::Static(d.name, &d.ty, folded.get(&at).cloned())
                }
                _ => InScope::Local(d.name, &d.ty),
            })
            .collect();
        let value = var.value.as_ref().and_then(|value| {
            crate::compiler::fold_in_scope(&s.tree, ty, value, &in_scope, s.byond)
        });
        if let Some(value) = &value {
            folded.insert(
                var as *const dreammaker::ast::VarStatement as usize,
                value.clone(),
            );
        }
        out.push((name, number, value));
    });
    out
}

fn local_const_users(old: &Snapshot, new: &Snapshot, realigned: &Realigned, changes: &mut Changes) {
    if changes.consts.is_empty()
        && changes.defaults_changed.is_empty()
        && changes.retyped.is_empty()
        && changes.colon_names.is_empty()
    {
        return;
    }
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
        if reloaded.contains(&old_key) {
            continue;
        }
        let (Some((old_ty, old_value)), Some((new_ty, new_value))) =
            (old.find(&old_key), new.find(key))
        else {
            continue;
        };
        let now = local_consts(new, new_ty, new_value);
        if now.is_empty() || local_consts(old, old_ty, old_value) == now {
            continue;
        }
        let reload = new.compile_code(key).and_then(|mut reload| {
            new.compile_in_lists(key, &mut reload, |_, _| false)?;
            Ok(reload)
        });
        changes.plan.line_moves.retain(|m| m.proc != old_key);
        match reload {
            Ok(reload) => changes.plan.reloads.push(ProcReload {
                proc: old_key,
                ..reload
            }),
            Err(e) => {
                let refusal = new.uncompiled(key, &e);
                changes.refused.push(Refusal {
                    why: format!(
                        "has a local const whose value changes, so it must be compiled again, but it {}",
                        refusal.why
                    ),
                    ..refusal
                });
            }
        }
    }
}

fn read_alone(s: &Snapshot, value: &ProcValue) -> HashSet<usize> {
    use dreammaker::ast::{BinaryOp, Expression, Follow, Statement, Term};
    let mut alone: HashSet<usize> = HashSet::new();
    let mark = |alone: &mut HashSet<usize>, e: &Expression| {
        alone.insert(e as *const Expression as usize);
    };
    let local_value = |alone: &mut HashSet<usize>, var: &dreammaker::ast::VarStatement| {
        let flags = var.var_type.flags;
        if let Some(e) = var.value.as_ref() {
            if !flags.is_static() && !flags.is_const() {
                alone.insert(e as *const Expression as usize);
            }
        }
    };
    let builtin = |name: &str| {
        s.tree
            .root()
            .get_proc(name)
            .is_some_and(|p| p.location.file == FileId::BUILTINS)
    };
    syntax::walk_proc(value, &mut |node| match node {
        syntax::Node::Parameter(p) => {
            if let Some(default) = &p.default {
                mark(&mut alone, default);
            }
        }
        syntax::Node::Statement(statement) => match statement {
            Statement::Return(Some(e)) => mark(&mut alone, e),
            Statement::Spawn { delay: Some(e), .. } => mark(&mut alone, e),
            Statement::Expr(Expression::BinaryOp {
                op: BinaryOp::LShift,
                lhs,
                rhs,
            }) => {
                mark(&mut alone, lhs);
                mark(&mut alone, rhs);
            }
            Statement::Expr(Expression::BinaryOp {
                op: BinaryOp::RShift,
                lhs,
                ..
            }) => mark(&mut alone, lhs),
            Statement::Var(var) => local_value(&mut alone, var),
            Statement::Vars(vars) => vars.iter().for_each(|var| local_value(&mut alone, var)),
            _ => {}
        },
        syntax::Node::Expression(expression) => match expression {
            Expression::AssignOp { rhs, .. } => mark(&mut alone, rhs),
            Expression::TernaryOp { if_, else_, .. }
                if alone.contains(&(expression as *const Expression as usize)) =>
            {
                mark(&mut alone, if_);
                mark(&mut alone, else_);
            }
            Expression::Base { term, follow } => {
                if follow.is_empty() && alone.contains(&(expression as *const Expression as usize))
                {
                    alone.insert(&term.elem as *const Term as usize);
                    if let Term::Expr(inner) = &term.elem {
                        mark(&mut alone, inner);
                    }
                }
                let args: &[Expression] = match &term.elem {
                    Term::Call(name, args) if !builtin(name.as_str()) => args,
                    Term::ParentCall(args) | Term::SelfCall(args) => args,
                    Term::List(args) | Term::Alist(args) => args,
                    Term::NewImplicit { args: Some(args) }
                    | Term::NewPrefab {
                        args: Some(args), ..
                    }
                    | Term::NewMiniExpr {
                        args: Some(args), ..
                    } => args,
                    _ => &[],
                };
                for arg in args {
                    mark(&mut alone, arg);
                }
                if let Term::InterpString(_, parts) = &term.elem {
                    for part in parts.iter().filter_map(|(part, _)| part.as_ref()) {
                        mark(&mut alone, part);
                    }
                }
                for f in follow.iter() {
                    match &f.elem {
                        Follow::Call(_, _, args) => args.iter().for_each(|a| mark(&mut alone, a)),
                        Follow::Index(_, index) => mark(&mut alone, index),
                        _ => {}
                    }
                }
            }
            _ => {}
        },
        _ => {}
    });
    alone
}

fn folds_changed(
    old: &Snapshot,
    new: &Snapshot,
    ty: &str,
    value: &ProcValue,
    consts: &HashSet<&str>,
) -> bool {
    use dreammaker::ast::{Expression, Follow, PropertyAccessKind, Spanned, Term};
    let bound_value = |s: &Snapshot, name: &str| {
        let at = s.find_type(ty)?;
        let (at, declaration) = match at.get_var_declaration(name) {
            Some(declaration) => (at, declaration),
            None => (s.tree.root(), s.tree.root().get_var_declaration(name)?),
        };
        if !declaration.get().var_type.flags.is_const() {
            return None;
        }
        at.get_value(name)?.constant.clone()
    };
    let global_value = |s: &Snapshot, name: &str| {
        let root = s.tree.root();
        root.get_var_declaration(name)
            .filter(|d| d.get().var_type.flags.is_const())
            .and_then(|_| root.get_value(name))
            .and_then(|v| v.constant.clone())
    };
    let global_changed =
        |name: &str| consts.contains(name) && global_value(old, name) != global_value(new, name);
    let local_types = retargeted::local_read_types(value);
    let chain_changed = |term: &Spanned<Term>, chain: &[Spanned<Follow>]| {
        let Some(Follow::Field(_, name) | Follow::StaticField(name)) =
            chain.last().map(|f| &f.elem)
        else {
            return false;
        };
        if !consts.contains(name.as_str()) {
            return false;
        }
        let expression = Expression::Base {
            term: Box::new(term.clone()),
            follow: chain.to_vec().into_boxed_slice(),
        };
        let head = match &term.elem {
            Term::Ident(head) => {
                local_types.get(&(&term.elem as *const Term as usize, head.to_string()))
            }
            _ => None,
        };
        let locals: Vec<(&str, &str)> = match (&term.elem, head) {
            (Term::Ident(name), Some(path)) => vec![(name.as_str(), path.as_str())],
            _ => vec![],
        };
        let folded = |s: &Snapshot| {
            crate::compiler::fold_in_proc(&s.tree, s.find_type(ty)?, &expression, &locals, s.byond)
        };
        folded(old) != folded(new)
    };
    let locals: HashSet<(usize, String)> = local_types.keys().cloned().collect();
    let alone = read_alone(new, value);
    let read_alone = |e: &Expression, chain: usize| {
        matches!(e, Expression::Base { follow, .. } if follow.len() == chain)
            && alone.contains(&(e as *const Expression as usize))
    };
    let mut found = false;
    syntax::walk_proc(value, &mut |node| {
        found = found
            || match node {
                syntax::Node::Term(term @ Term::Ident(name)) => {
                    consts.contains(name.as_str())
                        && !locals.contains(&(term as *const Term as usize, name.to_string()))
                        && !alone.contains(&(term as *const Term as usize))
                        && bound_value(old, name) != bound_value(new, name)
                }
                syntax::Node::Term(term @ Term::GlobalIdent(name)) => {
                    !alone.contains(&(term as *const Term as usize)) && global_changed(name)
                }
                syntax::Node::Expression(expression @ Expression::Base { term, follow }) => {
                    let field = |f: &Spanned<Follow>| {
                        matches!(
                            f.elem,
                            Follow::Field(PropertyAccessKind::Dot | PropertyAccessKind::Colon, _)
                        )
                    };
                    let fields = follow.iter().take_while(|f| field(f)).count();
                    match (&term.elem, follow.first().map(|f| &f.elem)) {
                        (Term::Ident(global), Some(Follow::Field(_, name)))
                            if global.as_str() == "global" && fields < 2 =>
                        {
                            !read_alone(expression, 1) && global_changed(name)
                        }
                        (Term::Prefab(prefab), Some(Follow::StaticField(name)))
                            if prefab.vars.is_empty() =>
                        {
                            let path = prefab.path.to_string();
                            let default = |s: &Snapshot| {
                                s.find_type(&path)
                                    .and_then(|t| t.get_value(name.as_str()))
                                    .map(|v| v.constant.clone())
                            };
                            let is_const = |s: &Snapshot| {
                                s.find_type(&path)
                                    .and_then(|t| t.get_var_declaration(name.as_str()))
                                    .is_some_and(|d| d.get().var_type.flags.is_const())
                            };
                            !(read_alone(expression, 1) && is_const(new))
                                && default(old) != default(new)
                        }
                        _ => {
                            let statics = follow
                                .iter()
                                .take_while(|f| matches!(f.elem, Follow::StaticField(_)))
                                .count();
                            let chain = fields.max(statics);
                            !read_alone(expression, chain) && chain_changed(term, &follow[..chain])
                        }
                    }
                }
                _ => false,
            };
    });
    found
}

fn colon_rebinds(
    old: &Snapshot,
    new: &Snapshot,
    ty: &str,
    value: &ProcValue,
    names: &HashSet<&str>,
) -> bool {
    use dreammaker::ast::{Expression, Term};
    if names.is_empty() {
        return false;
    }
    let local_types = retargeted::local_read_types(value);
    let mut found = false;
    syntax::walk_proc(value, &mut |node| {
        let syntax::Node::Expression(expression @ Expression::Base { term, follow }) = node else {
            return;
        };
        if found || !colon_fields(follow).any(|name| names.contains(name)) {
            return;
        }
        let locals: Vec<(&str, &str)> = match &term.elem {
            Term::Ident(head) => local_types
                .get(&(&term.elem as *const Term as usize, head.to_string()))
                .map(|path| vec![(head.as_str(), path.as_str())])
                .unwrap_or_default(),
            _ => vec![],
        };
        let globals = |s: &Snapshot| {
            s.find_type(ty).map(|t| {
                crate::compiler::field_globals_in_proc(&s.tree, t, expression, &locals, s.byond)
            })
        };
        found = globals(old) != globals(new);
    });
    found
}

enum LineChange {
    Same,
    Moved(Vec<(u32, u32)>),
    Recompile,
}

fn line_move(
    old: &Snapshot,
    new: &Snapshot,
    old_key: &ProcKey,
    key: &ProcKey,
    old_value: &ProcValue,
    new_value: &ProcValue,
) -> LineChange {
    line_change(old.compile(old_key), new.compile(key), old_value, new_value)
}

fn line_change(
    old_code: Result<CompiledProc, Diagnostic>,
    new_code: Result<CompiledProc, Diagnostic>,
    old_value: &ProcValue,
    new_value: &ProcValue,
) -> LineChange {
    let mut lines = BTreeMap::new();
    match (old_code, new_code) {
        (Ok(old_code), Ok(new_code)) => match line_map(&old_code.nodes, &new_code.nodes) {
            Some(map) => lines.extend(map),
            None => return LineChange::Recompile,
        },
        _ => {}
    }
    if lines.is_empty() {
        let Some(range) = old_value.body_range.as_ref() else {
            return LineChange::Same;
        };
        let (o, n) = (old_value.location.line, new_value.location.line);
        for line in o..=range.end.line.max(o) {
            lines.insert(line, (line as i64 + n as i64 - o as i64) as u32);
        }
    }
    if lines.iter().all(|(o, n)| o == n) {
        return LineChange::Same;
    }
    LineChange::Moved(lines.into_iter().collect())
}

fn line_map(old: &[Node], new: &[Node]) -> Option<BTreeMap<u32, u32>> {
    let (old_lines, new_lines) = (dbg_lines(old), dbg_lines(new));
    let mut lines = BTreeMap::new();
    if old_lines.len() == new_lines.len() {
        for (o, n) in old_lines.into_iter().zip(new_lines) {
            if *lines.entry(o).or_insert(n) != n {
                return None;
            }
        }
    }
    Some(lines)
}

fn dbg_lines(nodes: &[Node]) -> Vec<u32> {
    nodes
        .iter()
        .filter_map(|node| match node {
            Node::Instruction(Instruction::DbgLine(line), _) => Some(*line),
            _ => None,
        })
        .collect()
}

fn signature(
    v: &ProcValue,
) -> Vec<(
    dreammaker::ast::Ident,
    dreammaker::ast::VarType,
    Option<dreammaker::ast::Expression>,
    bool,
)> {
    v.parameters
        .iter()
        .map(|p| {
            (
                p.name.clone(),
                p.var_type.clone(),
                p.in_list.clone(),
                p.absolute,
            )
        })
        .collect()
}

fn params(v: &ProcValue) -> Vec<Param> {
    use dreammaker::ast::InputType;
    v.parameters
        .iter()
        .filter(|p| !p.absolute && p.name.as_str() != "...")
        .map(|p| {
            let mut input = p.var_type.input_type;
            if input.is_empty() {
                let path: Vec<&str> = p.var_type.type_path.iter().map(|s| s.as_str()).collect();
                input = match path.as_slice() {
                    ["atom"] => InputType::ATOM,
                    ["atom", "movable", ..] => InputType::MOVABLE,
                    ["obj", ..] => InputType::OBJ,
                    ["mob", ..] => InputType::MOB,
                    ["turf", ..] => InputType::TURF,
                    ["area", ..] => InputType::AREA,
                    _ => InputType::empty(),
                };
            }
            Param {
                name: p.name.as_str().to_owned(),
                input: input.bits() & 0xFFFF,
                input_extra: (input.bits() >> 16) & 0x3,
                in_list: builtin_in_list(p.in_list.as_ref()).map_or(InList::Kept, InList::Builtin),
            }
        })
        .collect()
}

#[derive(Default)]
struct FileContents<'a> {
    vars: BTreeMap<
        (String, String),
        (
            Option<dreammaker::ast::VarType>,
            Option<dreammaker::ast::Expression>,
        ),
    >,
    var_lines: BTreeMap<(String, String), u32>,
    declarations:
        BTreeMap<(String, String), (dreammaker::ast::ProcDeclKind, dreammaker::ast::ProcFlags)>,
    procs: BTreeMap<(String, String), Vec<(Option<ProcKey>, &'a ProcValue)>>,
}

impl<'a> FileContents<'a> {
    fn add(&mut self, ty: TypeRef<'a>, file: FileId) {
        let path = &ty.get().path;
        for (name, var) in ty.get().vars.iter() {
            let declared = var.declaration.as_ref().filter(|d| d.location.file == file);
            if var.value.location.file == file && var.value.expression.is_some() {
                self.var_lines
                    .insert((path.clone(), name.to_string()), var.value.location.line);
            }
            if var.value.location.file == file || declared.is_some() {
                self.vars.insert(
                    (path.clone(), name.to_string()),
                    (
                        declared.map(|d| d.var_type.clone()),
                        var.value.expression.clone(),
                    ),
                );
            }
        }
        for (name, proc) in ty.get().procs.iter() {
            if let Some(d) = proc
                .declaration
                .as_ref()
                .filter(|d| d.location.file == file)
            {
                self.declarations
                    .insert((path.clone(), name.to_string()), (d.kind, d.flags));
            }
            let values: Vec<(Option<ProcKey>, &ProcValue)> = proc
                .value
                .iter()
                .filter(|v| v.location.file == file)
                .map(|v| (None, v))
                .collect();
            if !values.is_empty() {
                self.procs.insert((path.clone(), name.to_string()), values);
            }
        }
    }
}

fn macros_in(
    history: &DefineHistory,
    file: FileId,
) -> (
    Vec<(String, Vec<String>, bool, Vec<dreammaker::lexer::Token>)>,
    Vec<String>,
) {
    let mut defined = vec![];
    let mut undefined = vec![];
    for (range, (name, define)) in history.iter() {
        if range.start.file == file {
            defined.push((
                name.to_string(),
                define.params.iter().map(|p| p.to_string()).collect(),
                define.variadic,
                define.subst.clone(),
            ));
        }
        if range.end.file == file {
            undefined.push(name.to_string());
        }
    }
    undefined.sort();
    (defined, undefined)
}

type DeclaredSettings = HashMap<(usize, String), Result<ProcSettings, String>>;

fn no_proc(key: &ProcKey) -> Diagnostic {
    Diagnostic::error(Id::NoSuchProc, format!("no proc {key}"))
}

fn panicked() -> Diagnostic {
    Diagnostic::error(Id::CompilerPanicked, "the compiler panicked")
}

fn settings_of(
    ty: TypeRef,
    name: &str,
    index: usize,
    declared: &mut DeclaredSettings,
) -> Result<ProcSettings, String> {
    let mut base = ProcSettings::default();
    let mut at = Some(ty);
    while let Some(t) = at {
        if let Some(proc) = t.get().procs.get(name) {
            if proc.declaration.is_some() {
                let first = proc
                    .value
                    .iter()
                    .position(|v| v.location.file != FileId::BUILTINS);
                if let Some(first) = first {
                    if !(t.index() == ty.index() && first == index) {
                        base = declared
                            .entry((t.get() as *const _ as usize, name.to_owned()))
                            .or_insert_with(|| proc_settings(&proc.value[first], base))
                            .clone()?;
                    }
                }
                break;
            }
        }
        at = t.parent_type();
    }
    let value = &ty.get().procs[name].value[index];
    proc_settings(value, base)
}

fn proc_settings(v: &ProcValue, base: ProcSettings) -> Result<ProcSettings, String> {
    use dreammaker::ast::{Expression, SettingMode, Term};
    let mut out = base;
    for setting in settings(v.code.as_ref()) {
        let Statement::Setting { name, mode, value } = setting else {
            continue;
        };
        let name = name.as_str();
        let unclear = || format!("cannot resolve `set {name}` value");
        let number = || -> Result<f32, String> {
            match value {
                Expression::Base { term, follow } if follow.is_empty() => match &term.elem {
                    Term::Int(n) => Ok(*n as f32),
                    Term::Float(f) => Ok(*f),
                    _ => Err(unclear()),
                },
                _ => Err(unclear()),
            }
        };
        let text = || -> Result<Option<String>, String> {
            let nowhere = dreammaker::Location {
                file: FileId::UNKNOWN,
                line: 0,
                column: 0,
            };
            match value.simple_evaluate(nowhere) {
                Ok(dreammaker::constants::Constant::String(s)) => crate::compiler::text(s.as_str())
                    .map(Some)
                    .ok_or_else(unclear),
                Ok(dreammaker::constants::Constant::Null(_)) => Ok(None),
                _ => Err(unclear()),
            }
        };
        match name {
            "waitfor" => out.waitfor = number()? != 0.0,
            "background" => out.background = number()? != 0.0,
            "hidden" => out.hidden = number()? != 0.0,
            "popup_menu" => out.popup_menu = number()? != 0.0,
            "instant" => out.instant = number()? != 0.0,
            "invisibility" => out.invisibility = Some(number()?.clamp(0.0, 255.0) as u8),
            "name" => out.name = text()?,
            "desc" => out.desc = text()?,
            "category" => {
                out.category = text()?;
                out.no_category = out.category.is_none();
            }
            "src" => {
                let equals = matches!(mode, SettingMode::Assign);
                let word = verb_src(value, equals)
                    .ok_or_else(|| "unsupported `set src` expression for hot reload".to_owned())?;
                out.src = Some((equals, word));
            }
            other if other.starts_with("SpacemanDMM_") => {}
            other => return Err(format!("unsupported hot reload setting: `set {other}`")),
        }
    }
    Ok(out)
}

fn verb_src(value: &dreammaker::ast::Expression, equals: bool) -> Option<u32> {
    use dreammaker::ast::{Expression, Follow, Term};
    if equals {
        if let Expression::Base { term, follow } = value {
            if let Term::Ident(name) = &term.elem {
                if name.as_str() == "usr" {
                    let fields: Vec<&str> = follow
                        .iter()
                        .filter_map(|f| match &f.elem {
                            Follow::Field(_, name) => Some(name.as_str()),
                            _ => None,
                        })
                        .collect();
                    match (fields.as_slice(), follow.len()) {
                        ([], 0) => return Some(0xFF << 8 | 0x20),
                        (["loc"], 1) => return Some(0xFF << 8 | 0x03),
                        _ => {}
                    }
                }
            }
        }
    }
    builtin_in_list(Some(value))
}

fn settings(code: Option<&Block>) -> Vec<&Statement> {
    fn visit<'a>(block: &'a Block, out: &mut Vec<&'a Statement>) {
        for stmt in block.iter() {
            match &stmt.elem {
                s @ Statement::Setting { .. } => out.push(s),
                Statement::While { block, .. }
                | Statement::DoWhile { block, .. }
                | Statement::ForInfinite { block }
                | Statement::ForLoop { block, .. }
                | Statement::Spawn { block, .. }
                | Statement::Label { block, .. } => visit(block, out),
                Statement::ForList(f) => visit(&f.block, out),
                Statement::ForKeyValue(f) => visit(&f.block, out),
                Statement::ForRange(f) => visit(&f.block, out),
                Statement::If { arms, else_arm } => {
                    for (_, block) in arms {
                        visit(block, out);
                    }
                    if let Some(block) = else_arm {
                        visit(block, out);
                    }
                }
                Statement::Switch { cases, default, .. } => {
                    for (_, block) in cases.iter() {
                        visit(block, out);
                    }
                    if let Some(block) = default {
                        visit(block, out);
                    }
                }
                Statement::TryCatch {
                    try_block,
                    catch_block,
                    ..
                } => {
                    visit(try_block, out);
                    visit(catch_block, out);
                }
                _ => {}
            }
        }
    }
    let mut out = vec![];
    if let Some(code) = code {
        visit(code, &mut out);
    }
    out
}

fn value_location(
    expression: &dreammaker::ast::Expression,
    otherwise: dreammaker::Location,
) -> dreammaker::Location {
    match expression {
        dreammaker::ast::Expression::Base { term, .. } => term.location,
        _ => otherwise,
    }
}

#[derive(Clone)]
struct Init {
    var: String,
    on: String,
    declared: String,
    value: dreammaker::ast::Expression,
    location: dreammaker::Location,
}

impl PartialEq for Init {
    fn eq(&self, other: &Self) -> bool {
        self.var == other.var
            && self.on == other.on
            && self.declared == other.declared
            && self.value == other.value
    }
}

fn inits<'a>(t: TypeRef, memo: &'a mut HashMap<usize, Vec<Init>>) -> &'a [Init] {
    if !memo.contains_key(&t.index().index()) {
        let mut list = match t.parent_type() {
            Some(parent) if !t.is_root() => inits(parent, memo).to_vec(),
            _ => vec![],
        };
        let vars = &t.get().vars;
        list.retain(|init| !vars.contains_key(init.var.as_str()));
        if !t.is_root() {
            for (name, var) in vars.iter() {
                if var.value.location.file == FileId::BUILTINS {
                    continue;
                }
                let declaration = t.get_var_declaration(name.as_str());
                let flags = declaration
                    .map(|d| d.get().var_type.flags)
                    .unwrap_or_default();
                if flags.is_static() || flags.is_const() {
                    continue;
                }
                if let Some(value) = &var.value.expression {
                    if var_constant(var).is_none() {
                        let mut value = value.clone();
                        type_macro_is(&mut value, &t.get().path);
                        list.push(Init {
                            var: name.as_str().to_owned(),
                            on: t.get().path.clone(),
                            declared: declaration
                                .map(|d| d.get().var_type.type_path.to_string())
                                .unwrap_or_default(),
                            value,
                            location: var.value.location,
                        });
                    }
                }
            }
        }
        memo.insert(t.index().index(), list);
    }
    &memo[&t.index().index()]
}

fn type_macro_is(expression: &mut dreammaker::ast::Expression, path: &str) {
    use dreammaker::ast::{Expression, Follow, Ident, PathOp, Prefab, RelativePath, Term};
    fn term(term: &mut Term, path: &str) {
        let all = |list: &mut [Expression]| list.iter_mut().for_each(|e| type_macro_is(e, path));
        match term {
            Term::__TYPE__ => {
                let mut parts = path.split('/').filter(|part| !part.is_empty());
                let Some(first) = parts.next() else {
                    *term = Term::Null;
                    return;
                };
                let mut relative =
                    RelativePath::single(PathOp::Slash, Ident::from_nonstatic(first));
                for part in parts {
                    relative.push((PathOp::Slash, Ident::from_nonstatic(part)));
                }
                *term = Term::Prefab(Box::new(Prefab::from(relative)));
            }
            Term::Expr(e) => type_macro_is(e, path),
            Term::Prefab(prefab) => prefab
                .vars
                .iter_mut()
                .for_each(|(_, e)| type_macro_is(e, path)),
            Term::InterpString(_, parts) => parts
                .iter_mut()
                .filter_map(|(e, _)| e.as_mut())
                .for_each(|e| type_macro_is(e, path)),
            Term::Call(_, args)
            | Term::SelfCall(args)
            | Term::ParentCall(args)
            | Term::List(args)
            | Term::Alist(args)
            | Term::GlobalCall(_, args) => all(args),
            Term::NewImplicit { args } | Term::NewMiniExpr { args, .. } => {
                args.iter_mut().for_each(|args| all(args))
            }
            Term::NewPrefab { prefab, args } => {
                prefab
                    .vars
                    .iter_mut()
                    .for_each(|(_, e)| type_macro_is(e, path));
                args.iter_mut().for_each(|args| all(args));
            }
            Term::Input { args, in_list, .. } | Term::Locate { args, in_list } => {
                all(args);
                in_list.iter_mut().for_each(|e| type_macro_is(e, path));
            }
            Term::Pick(choices) => choices.iter_mut().for_each(|(weight, e)| {
                weight.iter_mut().for_each(|w| type_macro_is(w, path));
                type_macro_is(e, path);
            }),
            Term::DynamicCall(a, b) => {
                all(a);
                all(b);
            }
            Term::ExternalCall {
                library,
                function,
                args,
            } => {
                library.iter_mut().for_each(|e| type_macro_is(e, path));
                type_macro_is(function, path);
                all(args);
            }
            _ => {}
        }
    }
    match expression {
        Expression::Base { term: t, follow } => {
            term(&mut t.elem, path);
            for f in follow.iter_mut() {
                match &mut f.elem {
                    Follow::Index(_, e) => type_macro_is(e, path),
                    Follow::Call(_, _, args) => {
                        args.iter_mut().for_each(|e| type_macro_is(e, path))
                    }
                    _ => {}
                }
            }
        }
        Expression::BinaryOp { lhs, rhs, .. } | Expression::AssignOp { lhs, rhs, .. } => {
            type_macro_is(lhs, path);
            type_macro_is(rhs, path);
        }
        Expression::TernaryOp { cond, if_, else_ } => {
            type_macro_is(cond, path);
            type_macro_is(if_, path);
            type_macro_is(else_, path);
        }
    }
}

fn compare_inits(
    old: &Snapshot,
    new: &Snapshot,
    retargeted: &retargeted::Retargeted,
    changes: &mut Changes,
) {
    let (mut old_memo, mut new_memo) = (HashMap::new(), HashMap::new());
    for n in new.tree.iter_types() {
        if n.is_root() {
            continue;
        }
        let new_inits = inits(n, &mut new_memo).to_vec();
        let mut before = None;
        match old.find_type(&n.get().path) {
            Some(o) if inits(o, &mut old_memo) == new_inits.as_slice() => {
                if !retargeted.inits_might_name(&new_inits, &n.get().path) {
                    continue;
                }
                let old_inits = inits(o, &mut old_memo).to_vec();
                before = Some(old.compile_init(o, &old_inits));
            }
            Some(_) => {}
            None => {
                let mut at = n.parent_type();
                let mut copied = None;
                while let Some(t) = at {
                    if let Some(o) = old.find_type(&t.get().path) {
                        copied = Some(inits(o, &mut old_memo).to_vec());
                        break;
                    }
                    at = t.parent_type();
                }
                if new_inits.is_empty() && copied.is_none_or(|c| c.is_empty()) {
                    continue;
                }
            }
        }
        let code = if new_inits.is_empty() {
            None
        } else {
            match new.compile_init(n, &new_inits) {
                Ok(code)
                    if before.is_some_and(|before| {
                        before.is_ok_and(|before| retargeted.same_code(&before, &code))
                    }) =>
                {
                    continue
                }
                Ok(code) => Some(code),
                Err(e) => {
                    changes.refused.push(
                        Refusal::new(
                            Id::InitializerFailed,
                            n.get().path.clone(),
                            format!("initializer compilation failed: {e}"),
                        )
                        .at(new, new.type_location(n)),
                    );
                    continue;
                }
            }
        };
        changes.plan.init_procs.push(InitProc {
            type_path: n.get().path.clone(),
            code,
        });
    }
}

fn real<'a>(ty: TypeRef<'a>, name: &str) -> Vec<&'a ProcValue> {
    let values = ty.get().procs.get(name).map_or(&[][..], |p| &p.value[..]);
    definition_order(ty, name)
        .into_iter()
        .map(|i| &values[i])
        .collect()
}

fn declaration_index(ty: TypeRef, name: &str, values: &[&ProcValue]) -> Option<usize> {
    let declaration = ty.get().procs.get(name)?.declaration.as_ref()?;
    values
        .iter()
        .position(|v| v.location == declaration.location)
}

fn pair(
    old: &Snapshot,
    new: &Snapshot,
    (o, old_values): (TypeRef, &[&ProcValue]),
    (n, new_values): (TypeRef, &[&ProcValue]),
    name: &str,
) -> Vec<Option<usize>> {
    let (Some(old_declaration), Some(new_declaration)) = (
        declaration_index(o, name, old_values),
        declaration_index(n, name, new_values),
    ) else {
        return match old_values.len() == new_values.len() {
            true => (0..new_values.len()).map(Some).collect(),
            false => align(old, new, old_values, new_values),
        };
    };
    let old_overrides: Vec<usize> = (0..old_values.len())
        .filter(|&i| i != old_declaration)
        .collect();
    let new_overrides: Vec<usize> = (0..new_values.len())
        .filter(|&j| j != new_declaration)
        .collect();
    let overrides = match old_overrides.len() == new_overrides.len() {
        true => (0..new_overrides.len()).map(Some).collect(),
        false => {
            let old_rest: Vec<&ProcValue> = old_overrides.iter().map(|&i| old_values[i]).collect();
            let new_rest: Vec<&ProcValue> = new_overrides.iter().map(|&j| new_values[j]).collect();
            align(old, new, &old_rest, &new_rest)
        }
    };
    let mut map = vec![None; new_values.len()];
    map[new_declaration] = Some(old_declaration);
    for (j, i) in new_overrides.into_iter().zip(overrides) {
        map[j] = i.map(|i| old_overrides[i]);
    }
    map
}

fn align(
    old: &Snapshot,
    new: &Snapshot,
    old_values: &[&ProcValue],
    new_values: &[&ProcValue],
) -> Vec<Option<usize>> {
    let (n, m) = (old_values.len(), new_values.len());
    let weight = |i: usize, j: usize| -> u32 {
        if old.file_name(old_values[i].location.file) != new.file_name(new_values[j].location.file)
        {
            0
        } else if old_values[i].code == new_values[j].code {
            3
        } else {
            2
        }
    };
    let mut best = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let mut b = best[i + 1][j].max(best[i][j + 1]);
            let w = weight(i, j);
            if w > 0 {
                b = b.max(w + best[i + 1][j + 1]);
            }
            best[i][j] = b;
        }
    }
    let mut map = vec![None; m];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        let w = weight(i, j);
        if w > 0 && best[i][j] == w + best[i + 1][j + 1] {
            map[j] = Some(i);
            i += 1;
            j += 1;
        } else if best[i][j] == best[i + 1][j] {
            i += 1;
        } else {
            j += 1;
        }
    }
    map
}

fn removed(
    old: &Snapshot,
    n: TypeRef,
    name: &str,
    old_values: &[&ProcValue],
    changes: &mut Changes,
) {
    let path = proc_path(n, name);
    if n.is_root() {
        changes.warnings.push(definition_retained(&path));
    }
    let verb = old
        .find_type(&n.get().path)
        .and_then(|o| o.get_proc_declaration(name))
        .is_some_and(|d| d.kind == dreammaker::ast::ProcDeclKind::Verb);
    for override_id in 0..old_values.len() {
        changes.plan.removed_procs.push(RemovedProc {
            proc: ProcKey {
                path: path.clone(),
                override_id: override_id as u32,
            },
            type_path: n.get().path.clone(),
            name: name.to_owned(),
            verb,
        });
    }
}

fn is_builtin(t: TypeRef, name: &str) -> bool {
    t.get_var_declaration(name)
        .is_some_and(|d| d.get().location.file == FileId::BUILTINS)
}

fn sets_builtin(t: TypeRef, name: &str) -> bool {
    let mut at = Some(t);
    while let Some(t) = at {
        if t.get()
            .vars
            .get(name)
            .is_some_and(|v| v.value.location.file != FileId::BUILTINS)
        {
            return true;
        }
        at = t.parent_type();
    }
    false
}

const BUILTIN_VARS: &[&str] = &[
    "name",
    "desc",
    "icon",
    "icon_state",
    "text",
    "suffix",
    "dir",
    "layer",
    "maptext",
    "maptext_x",
    "maptext_y",
    "maptext_width",
    "maptext_height",
    "density",
    "opacity",
    "gender",
    "mouse_drop_zone",
    "mouse_opacity",
    "animate_movement",
    "appearance_flags",
    "icon_w",
    "icon_z",
    "bounds",
    "alpha",
    "blend_mode",
    "color",
    "invisibility",
    "infra_luminosity",
    "luminosity",
    "pixel_x",
    "pixel_y",
    "pixel_w",
    "pixel_z",
    "plane",
    "render_source",
    "render_target",
    "bound_x",
    "bound_y",
    "bound_width",
    "bound_height",
    "step_size",
    "step_x",
    "step_y",
    "glide_size",
    "screen_loc",
    "vis_flags",
    "mouse_drag_pointer",
    "mouse_drop_pointer",
    "mouse_over_pointer",
    "override",
    "fps",
    "view",
    "perspective",
    "tick_lag",
    "default_verb_category",
    "edge_limit",
    "mouse_pointer_icon",
    "key",
    "sight",
    "see_in_dark",
    "see_invisible",
];

fn new_type(new: &Snapshot, n: TypeRef, changes: &mut Changes) {
    let path = n.get().path.clone();
    let Some(parent) = n.parent_type() else {
        changes.refused.push(
            Refusal::new(Id::NewTypeWithoutParent, path, "new type with no parent")
                .at(new, new.type_location(n)),
        );
        return;
    };
    let mut fields = vec![];
    let mut refused = false;
    for (name, var) in n.get().vars.iter() {
        let name = name.as_str();
        if !is_builtin(n, name)
            || var.value.location.file == FileId::BUILTINS
            || builtin_in_var_table(n, name)
        {
            continue;
        }
        if name == "parent_type" {
            continue;
        }
        let Some(value) = var_constant(var) else {
            continue;
        };
        if !BUILTIN_VARS.contains(&name) {
            changes.refused.push(
                Refusal::new(
                    Id::UnsupportedBuiltinDefault,
                    path.clone(),
                    "unsupported built-in default on a new type",
                )
                .named(name)
                .at(new, var.value.location),
            );
            refused = true;
            continue;
        }
        fields.push((name.to_owned(), value));
    }
    if refused {
        return;
    }

    let mut name = fields
        .iter()
        .find(|(field, _)| field == "name")
        .map(|(_, value)| value.clone());
    if name.is_none() && !sets_builtin(n, "name") {
        let last = path
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .replace('_', " ");
        name = Some(ConstValue::String(last.clone()));
        fields.push(("name".to_owned(), ConstValue::String(last)));
    }
    if !sets_builtin(n, "text") {
        if let Some(ConstValue::String(name)) = name {
            let first: String = name.chars().take(1).collect();
            fields.push(("text".to_owned(), ConstValue::String(first)));
        }
    }

    changes.plan.new_types.push(NewType {
        path,
        parent: parent.get().path.clone(),
        fields,
    });
}

fn compare_types(
    old: &Snapshot,
    new: &Snapshot,
    changes: &mut Changes,
    realigned: &mut Realigned,
) -> Vec<(ProcKey, String, String, bool, bool)> {
    let mut new_procs = vec![];
    let by_path = |s: &'_ Snapshot| -> BTreeMap<String, ()> {
        s.tree
            .iter_types()
            .map(|t| (t.get().path.clone(), ()))
            .collect()
    };
    let old_types = by_path(old);
    let new_types = by_path(new);
    for path in new_types.keys().filter(|p| !old_types.contains_key(*p)) {
        if let Some(n) = new.find_type(path) {
            new_type(new, n, changes);
        }
    }
    for path in old_types.keys().filter(|p| !new_types.contains_key(*p)) {
        if let Some(o) = old.find_type(path) {
            for (name, var) in o.get().vars.iter() {
                if var.declaration.is_some()
                    && !changes.colon_names.iter().any(|c| c == name.as_str())
                {
                    changes.colon_names.push(name.to_string());
                }
            }
        }
        changes.warnings.push(note(
            Id::RemovedTypeRetained,
            path,
            "type removed from source but retained in the running world",
        ));
    }

    let no_vars = Default::default();
    let no_procs = Default::default();
    for path in new_types.keys() {
        let Some(n) = new.find_type(path) else {
            continue;
        };
        let o = old.find_type(path);
        let shown = if path.is_empty() {
            "(global)"
        } else {
            path.as_str()
        };

        let parent = |t: TypeRef| t.parent_type().map(|p| p.get().path.clone());
        if let Some(o) = o.filter(|o| parent(*o) != parent(n)) {
            reparent::reparent(old, new, o, n, changes);
        }

        let ov = o.map_or(&no_vars, |o| &o.get().vars);
        let nv = &n.get().vars;
        let declaration =
            |v: &dreammaker::objtree::TypeVar| v.declaration.as_ref().map(|d| d.var_type.clone());
        for (name, var) in nv.iter() {
            if o.is_none()
                && is_builtin(n, name.as_str())
                && !builtin_in_var_table(n, name.as_str())
            {
                continue;
            }
            match ov.get(name) {
                Some(old_var) if moved::var_moved_up(n, name.as_str(), old_var, Some(var)) => {
                    moved::var_moved(old, new, n, name.as_str(), old_var, Some(var), changes);
                }
                None if var.declaration.is_some() => {
                    if !n.is_root() && !changes.colon_names.iter().any(|c| c == name.as_str()) {
                        changes.colon_names.push(name.to_string());
                    }
                    if new_var(new, n, name.as_str(), var, o.is_none(), changes) {
                        if let Some(added) = changes.plan.new_vars.last_mut() {
                            added.moved_from = moved::moved_from(old, n, name.as_str());
                        }
                        var_default(old, new, path, name.as_str(), true, changes);
                    }
                }
                None => var_default(old, new, path, name.as_str(), false, changes),
                Some(old_var) if declaration(old_var) != declaration(var) => {
                    let storage = |v: &dreammaker::objtree::TypeVar| {
                        v.declaration.as_ref().map(|d| {
                            (
                                d.var_type.flags.is_static(),
                                d.var_type.flags.is_const(),
                                d.var_type.flags.is_tmp(),
                                d.var_type.input_type,
                            )
                        })
                    };
                    if !changes.retyped.iter().any(|r| r == name.as_str()) {
                        changes.retyped.push(name.to_string());
                    }
                    let kind =
                        |v: &dreammaker::objtree::TypeVar| storage(v).map(|s| (s.0, s.1, s.2));
                    let static_like =
                        |v: &dreammaker::objtree::TypeVar| kind(v).is_some_and(|k| k.0 || k.1);
                    let redeclared = kind(old_var) != kind(var) && !n.is_root();
                    if needs_constant(var) {
                        changes.refused.push(
                            Refusal::new(
                                Id::ConstInitializerRequired,
                                shown,
                                "constant initializer required for const var",
                            )
                            .named(name.as_str())
                            .at(new, var.value.location),
                        );
                        continue;
                    }
                    if let Some(why) = dm_refused_const(var) {
                        changes
                            .warnings
                            .push(refused_const_warning(&shown, name, why));
                    }
                    if redeclared {
                        redeclare(new, n, name.as_str(), old_var, var, changes);
                    }
                    let valued = redeclared
                        && (static_like(old_var) != static_like(var)
                            || kind(var).is_some_and(|k| k.1));
                    if !valued && old_var.value.expression != var.value.expression {
                        var_default(old, new, path, name.as_str(), false, changes);
                    }
                }
                Some(old_var)
                    if old_var.value.expression != var.value.expression
                        || old_var.value.constant != var.value.constant =>
                {
                    var_default(old, new, path, name.as_str(), false, changes);
                }
                Some(old_var)
                    if old.place(old_var.value.location) != new.place(var.value.location) =>
                {
                    if !changes.defaults_moved.iter().any(|c| c == name.as_str()) {
                        changes.defaults_moved.push(name.to_string());
                    }
                }
                Some(_) => {}
            }
        }
        for (name, var) in ov.iter().filter(|(name, _)| !nv.contains_key(*name)) {
            if moved::var_moved_up(n, name.as_str(), var, None) {
                moved::var_moved(old, new, n, name.as_str(), var, None, changes);
            } else if var.declaration.is_some() {
                let id = match n.is_root() {
                    true => Id::RemovedGlobalRetained,
                    false => Id::RemovedVarRetained,
                };
                changes
                    .warnings
                    .push(var_retained(id, shown, name.as_str()));
                if !n.is_root() {
                    changes.plan.removed_vars.push(RemovedVar {
                        type_path: path.to_owned(),
                        var: name.to_string(),
                    });
                    if !changes.colon_names.iter().any(|c| c == name.as_str()) {
                        changes.colon_names.push(name.to_string());
                    }
                }
            } else if n.get_var_declaration(name.as_str()).is_some() {
                var_default(old, new, path, name.as_str(), false, changes);
            }
        }

        let op = o.map_or(&no_procs, |o| &o.get().procs);
        let np = &n.get().procs;
        for (name, proc) in np.iter() {
            let old_values = o.map(|o| real(o, name.as_str())).unwrap_or_default();
            let new_values = real(n, name.as_str());
            let (old_count, new_count) = (old_values.len(), new_values.len());
            let path = proc_path(n, name.as_str());
            let verb = n
                .get_proc_declaration(name.as_str())
                .is_some_and(|d| d.kind == dreammaker::ast::ProcDeclKind::Verb);
            let map = match o {
                Some(o) => pair(old, new, (o, &old_values), (n, &new_values), name.as_str()),
                None => vec![None; new_count],
            };
            if !map.iter().flatten().is_sorted() {
                let at = proc
                    .declaration
                    .as_ref()
                    .map(|d| d.location)
                    .filter(|l| new.source_line(*l).is_some())
                    .unwrap_or_else(|| new.type_location(n));
                changes.refused.push(
                    Refusal::new(
                        Id::DefinitionOrderChanged,
                        path.clone(),
                        "the order of its definitions changed: DreamMaker puts a declaration at the type's first declaration",
                    )
                    .at(new, at),
                );
                continue;
            }
            if new_count != old_count {
                for override_id in (0..old_count).filter(|i| !map.contains(&Some(*i))) {
                    if n.is_root() {
                        changes.warnings.push(definition_retained(&path));
                    }
                    changes.plan.removed_procs.push(RemovedProc {
                        proc: ProcKey {
                            path: path.clone(),
                            override_id: override_id as u32,
                        },
                        type_path: n.get().path.clone(),
                        name: name.as_str().to_owned(),
                        verb,
                    });
                }
                for (override_id, _) in map.iter().enumerate().filter(|(_, old)| old.is_none()) {
                    let declared = old_count == 0
                        && proc
                            .declaration
                            .as_ref()
                            .is_some_and(|d| d.location == new_values[override_id].location);
                    if n.is_root() && !declared {
                        changes.warnings.push(note(
                            Id::UnreachableGlobalDefinition,
                            &path,
                            "additional global definition is unreachable; calls retain the original definition, matching DreamMaker",
                        ));
                    }
                    new_procs.push((
                        ProcKey {
                            path: path.clone(),
                            override_id: override_id as u32,
                        },
                        n.get().path.clone(),
                        name.as_str().to_owned(),
                        declared,
                        verb,
                    ));
                }
                realigned.insert(
                    path,
                    map.into_iter().map(|old| old.map(|i| i as u32)).collect(),
                );
                continue;
            }
            let declaration = |p: Option<&dreammaker::objtree::TypeProc>| {
                p.and_then(|p| p.declaration.as_ref())
                    .map(|d| (d.kind, d.flags))
            };
            if declaration(op.get(name)) != declaration(Some(proc)) {
                if moved::proc_moved_up(n, name.as_str(), op.get(name), proc) {
                    changes.moved_procs.push(path.clone());
                } else {
                    let at = proc
                        .declaration
                        .as_ref()
                        .map(|d| d.location)
                        .filter(|l| new.source_line(*l).is_some())
                        .unwrap_or_else(|| new.type_location(n));
                    changes.refused.push(
                        Refusal::new(
                            Id::ProcDeclarationChanged,
                            path.clone(),
                            "proc declaration kind or flags changed",
                        )
                        .at(new, at),
                    );
                }
            }
        }
        for name in op.keys().filter(|name| !np.contains_key(*name)) {
            let old_values = o.map(|o| real(o, name.as_str())).unwrap_or_default();
            if !old_values.is_empty() {
                removed(old, n, name.as_str(), &old_values, changes);
            }
        }
    }

    new_procs
}

fn new_var(
    snapshot: &Snapshot,
    n: TypeRef,
    name: &str,
    var: &dreammaker::objtree::TypeVar,
    new_type: bool,
    changes: &mut Changes,
) -> bool {
    let shown = if n.is_root() {
        "(global)".to_owned()
    } else {
        n.get().path.clone()
    };
    let flags = var
        .declaration
        .as_ref()
        .map(|d| d.var_type.flags)
        .unwrap_or_default();
    let at = var
        .declaration
        .as_ref()
        .map_or(var.value.location, |d| d.location);
    let place = format!("{}:{}", snapshot.file_name(at.file), at.line);
    if let Some(other) = declared_elsewhere(n, name) {
        changes.refused.push(
            Refusal::new(
                Id::DuplicateDefinition,
                &shown,
                format!("duplicate definition ({other} declares it too)"),
            )
            .named(name)
            .at(snapshot, at),
        );
        return false;
    }
    if needs_constant(var) {
        changes.refused.push(
            Refusal::new(
                Id::ConstInitializerRequired,
                &shown,
                "constant initializer required",
            )
            .named(name)
            .at(snapshot, at),
        );
        return false;
    }
    if let Some(why) = dm_refused_const(var) {
        changes
            .warnings
            .push(refused_const_warning(&place, name, why));
    }
    let static_like = flags.is_static() || flags.is_const() || n.is_root();
    let (value, init) = if static_like {
        match static_initial(snapshot, n, var) {
            Ok(value) => value,
            Err(e) => {
                changes.refused.push(
                    Refusal::new(
                        Id::InitializerFailed,
                        &shown,
                        format!("initializer compilation failed: {e}"),
                    )
                    .named(name)
                    .at(snapshot, at),
                );
                return false;
            }
        }
    } else {
        match var_constant(var) {
            Some(value) => (value, None),
            None => {
                if new_type {
                    changes.warnings.push(note(
                        Id::NewVarComputedDefault,
                        &shown,
                        format!("new var {name} has a computed default; initial() remains null until restart"),
                    ));
                } else {
                    changes.warnings.push(note(
                        Id::NewVarComputedDefault,
                        &shown,
                        format!("new var {name} has a computed default; existing instances and initial() remain null until restart"),
                    ));
                }
                (ConstValue::Null, None)
            }
        }
    };
    changes.plan.new_vars.push(NewVar {
        type_path: n.get().path.clone(),
        var: name.to_owned(),
        flags: byond_flags(flags, n.is_root()),
        value,
        init,
        moved_from: vec![],
    });
    true
}

fn declared_elsewhere(n: TypeRef, name: &str) -> Option<String> {
    let declares = |t: TypeRef| {
        t.get()
            .vars
            .get(name)
            .is_some_and(|v| v.declaration.is_some())
    };
    let mut at = n.parent_type().filter(|t| !t.is_root());
    while let Some(t) = at {
        if declares(t) {
            return Some(t.get().path.clone());
        }
        at = t.parent_type().filter(|t| !t.is_root());
    }
    if n.is_root() {
        return None;
    }
    let descends = |t: TypeRef| {
        let mut at = t.parent_type();
        while let Some(parent) = at {
            if parent == n {
                return true;
            }
            at = parent.parent_type();
        }
        false
    };
    n.tree()
        .iter_types()
        .find(|t| *t != n && declares(*t) && descends(*t))
        .map(|t| t.get().path.clone())
}

fn needs_constant(var: &dreammaker::objtree::TypeVar) -> bool {
    use dreammaker::constants::Constant;
    let is_const = var
        .declaration
        .as_ref()
        .is_some_and(|d| d.var_type.flags.is_const());
    let Some(expression) = var.value.expression.as_ref().filter(|_| is_const) else {
        return false;
    };
    let constant = var
        .value
        .constant
        .clone()
        .or_else(|| expression.simple_evaluate(var.value.location).ok());
    matches!(
        constant,
        None | Some(Constant::List(_)) | Some(Constant::New { .. }) | Some(Constant::Call(..))
    )
}

fn refused_const_value(new: &Snapshot, n: TypeRef, name: &str, changes: &mut Changes) -> bool {
    let Some(var) = n.get().vars.get(name).filter(|var| needs_constant(var)) else {
        return false;
    };
    let shown = if n.is_root() {
        "(global)"
    } else {
        &n.get().path
    };
    changes.refused.push(
        Refusal::new(
            Id::ConstInitializerRequired,
            shown,
            "constant initializer required",
        )
        .named(name)
        .at(new, var.value.location),
    );
    true
}

fn dm_refused_const(var: &dreammaker::objtree::TypeVar) -> Option<&'static str> {
    use dreammaker::ast::{BinaryOp, Expression, Term};
    fn why(expr: &Expression) -> Option<&'static str> {
        match expr {
            Expression::Base { term, .. } => match &term.elem {
                Term::Ident(name) if name.as_str() == "type" => Some("it reads type"),
                Term::Ident(name) if name.as_str() == "parent_type" => Some("it reads parent_type"),
                Term::Expr(inner) => why(inner),
                _ => None,
            },
            Expression::BinaryOp { op, lhs, rhs } => match op {
                BinaryOp::Eq
                | BinaryOp::NotEq
                | BinaryOp::Less
                | BinaryOp::LessEq
                | BinaryOp::Greater
                | BinaryOp::GreaterEq
                | BinaryOp::Equiv
                | BinaryOp::NotEquiv => Some("it compares"),
                _ => why(lhs).or_else(|| why(rhs)),
            },
            Expression::TernaryOp { cond, if_, else_ } => {
                why(cond).or_else(|| why(if_)).or_else(|| why(else_))
            }
            _ => None,
        }
    }
    let is_const = var
        .declaration
        .as_ref()
        .is_some_and(|d| d.var_type.flags.is_const());
    why(var.value.expression.as_ref().filter(|_| is_const)?)
}

fn refused_const_warning(place: &str, name: &str, why: &str) -> Diagnostic {
    note(
        Id::ConstRefusedByDreamMaker,
        place,
        format!("const {name}: DreamMaker refuses this initializer ({why}; constant initializer required). Accepted for hot reload, but a rebuild will fail"),
    )
}

fn byond_flags(flags: dreammaker::ast::VarTypeFlags, global: bool) -> u32 {
    let mut byond_flags = 0;
    if flags.is_static() || global {
        byond_flags |= 1;
    }
    if flags.is_const() {
        byond_flags |= 3;
    }
    if flags.is_tmp() {
        byond_flags |= 4;
    }
    byond_flags
}

fn static_proc_path(scope: &StaticScope) -> Option<&str> {
    match scope {
        StaticScope::Proc { path, .. } => Some(path),
        _ => None,
    }
}

fn static_number(scope: &StaticScope) -> u32 {
    match scope {
        StaticScope::Proc { nth, .. } => *nth,
        _ => 0,
    }
}

fn static_initial(
    snapshot: &Snapshot,
    n: TypeRef,
    var: &dreammaker::objtree::TypeVar,
) -> Result<(ConstValue, Option<Box<ProcReload>>), String> {
    if let Some(value) = var_constant(var) {
        return Ok((value, None));
    }
    let Some(expression) = &var.value.expression else {
        return Ok((ConstValue::Null, None));
    };
    let location = value_location(expression, var.value.location);
    let declared = var
        .declaration
        .as_ref()
        .map(|d| d.var_type.type_path.to_string())
        .unwrap_or_default();
    let code = snapshot
        .compile_value(n, expression, &declared, location, None, &[])
        .map_err(|e| e.to_string())?;
    Ok((ConstValue::Null, Some(Box::new(code))))
}

fn redeclare(
    new: &Snapshot,
    n: TypeRef,
    name: &str,
    old_var: &dreammaker::objtree::TypeVar,
    var: &dreammaker::objtree::TypeVar,
    changes: &mut Changes,
) {
    let path = n.get().path.clone();
    let flags_of = |v: &dreammaker::objtree::TypeVar| {
        v.declaration
            .as_ref()
            .map(|d| d.var_type.flags)
            .unwrap_or_default()
    };
    let (old_flags, flags) = (flags_of(old_var), flags_of(var));
    let is_static = |f: dreammaker::ast::VarTypeFlags| f.is_static() || f.is_const();
    let mut redeclared = Redeclared {
        type_path: path.clone(),
        var: name.to_owned(),
        flags: byond_flags(flags, false),
        value: None,
        init: None,
    };

    if is_static(flags) {
        if !is_static(old_flags) || flags.is_const() {
            match static_initial(new, n, var) {
                Ok((value, init)) => {
                    redeclared.value = Some(value);
                    redeclared.init = init;
                }
                Err(e) => {
                    changes.refused.push(
                        Refusal::new(
                            Id::InitializerFailed,
                            path,
                            format!("initializer compilation failed: {e}"),
                        )
                        .named(name)
                        .at(new, new.var_location(n, name)),
                    );
                    return;
                }
            }
        }
        changes.plan.redeclared.push(redeclared);
        return;
    }

    changes.plan.redeclared.push(redeclared);
    if !is_static(old_flags) {
        return;
    }
    let mut defaults: Vec<VarDefault> = vec![];
    let mut computed = false;
    for ty in new.tree.iter_types() {
        let mut at = Some(ty);
        while let Some(t) = at {
            if t.index() == n.index() {
                break;
            }
            at = t.parent_type();
        }
        if at.is_none() {
            continue;
        }
        match default_value(ty, name) {
            Some(Some(value)) => match defaults.iter_mut().find(|d| d.value == value) {
                Some(d) => d.types.push(ty.get().path.clone()),
                None => defaults.push(VarDefault {
                    var: name.to_owned(),
                    value,
                    types: vec![ty.get().path.clone()],
                }),
            },
            _ => computed = true,
        }
    }
    if computed {
        changes.warnings.push(note(
            Id::ComputedDefaultNotApplied,
            path,
            format!("var {name} has a computed default; existing instances retain null"),
        ));
    }
    changes.plan.var_defaults.extend(defaults);
}

fn defined_on<'a>(t: TypeRef<'a>, name: &str) -> Option<TypeRef<'a>> {
    let mut at = Some(t);
    while let Some(ty) = at {
        if ty.get().vars.contains_key(name) {
            return Some(ty);
        }
        at = ty.parent_type();
    }
    None
}

fn default_value(t: TypeRef, name: &str) -> Option<Option<ConstValue>> {
    let var = defined_on(t, name)?.get().vars.get(name)?;
    Some(var_constant(var))
}

fn var_constant(var: &dreammaker::objtree::TypeVar) -> Option<ConstValue> {
    match &var.value.expression {
        None => Some(ConstValue::Null),
        Some(expression) => match &var.value.constant {
            Some(constant) => const_value(constant),
            None => const_value(&expression.simple_evaluate(var.value.location).ok()?),
        },
    }
}

fn var_default(
    old: &Snapshot,
    new: &Snapshot,
    path: &str,
    name: &str,
    is_new: bool,
    changes: &mut Changes,
) {
    let shown = if path.is_empty() { "(global)" } else { path };
    let Some(n) = new.find_type(path) else { return };
    let Some(declaration) = n.get_var_declaration(name) else {
        changes.refused.push(
            Refusal::new(
                Id::ValueWithoutDeclaration,
                shown,
                "has a value but no declaration",
            )
            .named(name)
            .at(new, new.var_location(n, name)),
        );
        return;
    };
    let flags = declaration.get().var_type.flags;
    let in_var_table = builtin_in_var_table(n, name);
    let builtin = declaration.get().location.file == FileId::BUILTINS
        && (!in_var_table || !sets_own_builtin(n, name));
    if n.is_root() && flags.is_const() && !is_new {
        if refused_const_value(new, n, name, changes) {
            return;
        }
        if !changes.consts.iter().any(|c| c == name) {
            changes.consts.push(name.to_owned());
        }
        if let Some(why) = n.get().vars.get(name).and_then(dm_refused_const) {
            changes
                .warnings
                .push(refused_const_warning(shown, name, why));
        }
        if let Some(var) = n.get().vars.get(name) {
            match static_initial(new, n, var) {
                Ok((value, init)) => changes.plan.redeclared.push(Redeclared {
                    type_path: String::new(),
                    var: name.to_owned(),
                    flags: byond_flags(flags, true),
                    value: Some(value),
                    init,
                }),
                Err(e) => changes.refused.push(
                    Refusal::new(
                        Id::ConstValueFailed,
                        shown,
                        format!("const value compilation failed: {e}"),
                    )
                    .named(name)
                    .at(new, new.var_location(n, name)),
                ),
            }
        }
        return;
    }
    if n.is_root() || flags.is_static() {
        return;
    }
    if flags.is_const() {
        if is_new {
            return;
        }
        if refused_const_value(new, n, name, changes) {
            return;
        }
        if !changes.consts.iter().any(|c| c == name) {
            changes.consts.push(name.to_owned());
        }
        if let Some(why) = n.get().vars.get(name).and_then(dm_refused_const) {
            changes
                .warnings
                .push(refused_const_warning(shown, name, why));
        }
        if builtin {
            return;
        }
        let declaring = declaration.ty();
        let path = declaring.get().path.clone();
        if changes
            .plan
            .redeclared
            .iter()
            .any(|r| r.type_path == path && r.var == name)
        {
            return;
        }
        let Some(var) = declaring.get().vars.get(name) else {
            return;
        };
        match static_initial(new, declaring, var) {
            Ok((value, init)) => changes.plan.redeclared.push(Redeclared {
                type_path: path,
                var: name.to_owned(),
                flags: byond_flags(flags, false),
                value: Some(value),
                init,
            }),
            Err(e) => changes.refused.push(
                Refusal::new(
                    Id::ConstValueFailed,
                    shown,
                    format!("const value compilation failed: {e}"),
                )
                .named(name)
                .at(new, new.var_location(n, name)),
            ),
        }
        return;
    }

    if builtin && name == "script" && n.get().path == "/client" {
        return;
    }

    if builtin && in_world_record(n, name) {
        if !is_new {
            changes.deferred.push(
                Refusal::new(Id::SettingChanged, path, "changed setting")
                    .named(name)
                    .until(Until::Rebuild)
                    .at(new, new.var_location(n, name)),
            );
            changes
                .restart_settings
                .push((path.to_owned(), name.to_owned()));
        }
        return;
    }

    if !is_new && !changes.defaults_changed.iter().any(|c| c == name) {
        changes.defaults_changed.push(name.to_owned());
    }

    if builtin
        && !is_new
        && !sets_own_builtin(n, name)
        && (in_var_table || BUILTIN_VARS.contains(&name))
        && !matches!(name, "name" | "text")
    {
        if !in_var_table && reparent::builtin_base(n).as_deref() == Some(n.get().path.as_str()) {
            changes.refused.push(
                Refusal::new(
                    Id::BuiltinDefaultNotRestorable,
                    shown,
                    "cannot restore this built-in type's original default",
                )
                .named(name)
                .at(new, new.var_location(n, name)),
            );
            return;
        }
        if !in_var_table {
            changes.warnings.push(note(
                Id::BuiltinDefaultNewInstancesOnly,
                shown,
                format!("{name}: built-in default applies only to new instances"),
            ));
        }
        changes.plan.builtin_inherited.push(BuiltinInherited {
            var: name.to_owned(),
            types: inheriting(new, n, name),
        });
        return;
    }

    let Some(Some(value)) = default_value(n, name) else {
        if !is_new && !builtin {
            changes.warnings.push(note(
                Id::ComputedDefaultNotApplied,
                shown,
                format!("var {name} has a computed default; only new instances receive it"),
            ));
        }
        return;
    };
    if !is_new {
        let old_value = old.find_type(path).and_then(|o| default_value(o, name));
        if old_value == Some(Some(value.clone())) {
            return;
        }
    }

    let types = default_types(new, n, name);
    if builtin {
        if !BUILTIN_VARS.contains(&name) {
            changes.refused.push(
                Refusal::new(
                    Id::UnsupportedBuiltinDefault,
                    shown,
                    "unsupported built-in default change",
                )
                .named(name)
                .at(new, new.var_location(n, name)),
            );
            return;
        }
        changes.warnings.push(note(
            Id::BuiltinDefaultNewInstancesOnly,
            shown,
            format!("{name}: built-in default applies only to new instances"),
        ));
        if name == "name" {
            builtin_names(new, n, value, types, changes);
            return;
        }
        changes.plan.builtin_defaults.push(VarDefault {
            var: name.to_owned(),
            value,
            types,
        });
        return;
    }
    changes.plan.var_defaults.push(VarDefault {
        var: name.to_owned(),
        value,
        types,
    });
}

fn default_types(new: &Snapshot, n: TypeRef, name: &str) -> Vec<String> {
    let mut types = vec![];
    for ty in new.tree.iter_types() {
        let mut at = ty;
        loop {
            if at.index() == n.index() {
                types.push(ty.get().path.clone());
                break;
            }
            if at.get().vars.contains_key(name) {
                break;
            }
            match at.parent_type() {
                Some(parent) => at = parent,
                None => break,
            }
        }
    }
    types
}

const CLIENT_SETTINGS: &[&str] = &[
    "authenticate",
    "command_text",
    "control_freak",
    "dir",
    "lazy_eye",
    "preload_rsc",
    "show_map",
    "show_popup_menus",
    "show_verb_panel",
];

fn in_world_record(t: TypeRef, name: &str) -> bool {
    match t.get().path.as_str() {
        "/world" => true,
        "/client" => CLIENT_SETTINGS.contains(&name),
        _ => false,
    }
}

pub const VAR_TABLE_BUILTINS: &[&str] = &["/sound", "/particles", "/exception", "/regex", "/icon"];

fn builtin_in_var_table(t: TypeRef, name: &str) -> bool {
    t.get_var_declaration(name).is_some_and(|d| {
        d.get().location.file == FileId::BUILTINS
            && VAR_TABLE_BUILTINS.contains(&d.ty().get().path.as_str())
    })
}

fn sets_own_builtin(t: TypeRef, name: &str) -> bool {
    t.get()
        .vars
        .get(name)
        .is_some_and(|v| v.value.location.file != FileId::BUILTINS)
}

fn inheriting(new: &Snapshot, n: TypeRef, name: &str) -> Vec<String> {
    let mut types = vec![];
    for ty in new.tree.iter_types() {
        let mut at = ty;
        let mut depth = 0;
        loop {
            if at.index() == n.index() {
                types.push((depth, ty.get().path.clone()));
                break;
            }
            if at.get().vars.contains_key(name) {
                break;
            }
            depth += 1;
            match at.parent_type() {
                Some(parent) => at = parent,
                None => break,
            }
        }
    }
    types.sort();
    types.into_iter().map(|(_, path)| path).collect()
}

fn builtin_names(
    new: &Snapshot,
    n: TypeRef,
    value: ConstValue,
    types: Vec<String>,
    changes: &mut Changes,
) {
    let own_name = |path: &str| {
        ConstValue::String(
            path.rsplit('/')
                .next()
                .unwrap_or_default()
                .replace('_', " "),
        )
    };
    let named = sets_builtin(n, "name");
    let mut names = vec![];
    for path in types {
        let name = if named {
            value.clone()
        } else {
            own_name(&path)
        };
        names.push((path, name));
    }
    for (path, name) in &names {
        let push = |changes: &mut Changes, var: &str, value: ConstValue| match changes
            .plan
            .builtin_defaults
            .iter_mut()
            .find(|d| d.var == var && d.value == value)
        {
            Some(d) => d.types.push(path.clone()),
            None => changes.plan.builtin_defaults.push(VarDefault {
                var: var.to_owned(),
                value,
                types: vec![path.clone()],
            }),
        };
        push(changes, "name", name.clone());
        let sets_text = new.find_type(path).is_some_and(|t| sets_builtin(t, "text"));
        if let (ConstValue::String(name), false) = (name, sets_text) {
            push(
                changes,
                "text",
                ConstValue::String(name.chars().take(1).collect()),
            );
        }
    }
}

fn const_value(constant: &dreammaker::constants::Constant) -> Option<ConstValue> {
    use dreammaker::constants::Constant;
    Some(match constant {
        Constant::Null(_) => ConstValue::Null,
        Constant::Float(f) => ConstValue::Number(*f),
        Constant::String(s) => ConstValue::String(crate::compiler::text(s.as_str())?),
        Constant::Resource(s) => ConstValue::Resource(s.to_string()),
        Constant::Prefab(pop) if pop.vars.is_empty() => {
            let path: Vec<&str> = pop.path.iter().map(|s| s.as_str()).collect();
            ConstValue::Path(format!("/{}", path.join("/")))
        }
        _ => return None,
    })
}

fn proc_lines(value: &ProcValue) -> std::ops::RangeInclusive<u32> {
    let start = value.location.line;
    let end = match &value.body_range {
        Some(r) if r.end.file == value.location.file && r.end.line >= start => r.end.line,
        Some(_) => u32::MAX,
        None => start,
    };
    start..=end
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Project {
        dir: PathBuf,
    }

    impl Project {
        fn new(name: &str, files: &[(&str, &str)]) -> Project {
            let dir =
                std::env::temp_dir().join(format!("dmasm-files-{}-{}", name, std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let project = Project { dir };
            let mut dme = String::new();
            for (file, text) in files {
                dme += &format!("#include \"{file}\"\n");
                project.write(file, text);
            }
            project.write("world.dme", &dme);
            project
        }

        fn write(&self, file: &str, text: &str) {
            std::fs::write(self.dir.join(file), text).unwrap();
        }

        fn parse(&self) -> Snapshot {
            Snapshot::parse(&self.dir.join("world.dme"), &ParseOptions::default()).unwrap()
        }

        fn both(&self, old: &mut Snapshot, files: &[&str]) -> (Result<String, String>, String) {
            let paths: Vec<&Path> = files.iter().map(Path::new).collect();
            let shown = |changes: &Changes| {
                format!(
                    "{:#?}\n{:#?}\n{:#?}",
                    changes.plan, changes.refused, changes.warnings
                )
            };
            let own = old
                .reload_files(&self.dir, &paths)
                .map(|reload| shown(&reload.changes));
            let edited: HashSet<PathBuf> = files.iter().map(PathBuf::from).collect();
            let full = diff(old, &self.parse(), Some(&edited));
            (own, shown(&full))
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn reloaded(changes: &Changes) -> Vec<String> {
        changes
            .plan
            .reloads
            .iter()
            .map(|reload| reload.proc.to_string())
            .collect()
    }

    #[test]
    fn lines_that_move_under_a_header_that_stays_are_remapped_as_by_a_full_parse() {
        let a = "/proc/f()\n\tworld.log << 1\n\treturn 2\n/proc/g()\n\treturn 3\n";
        let project = Project::new("under", &[("a.dm", a)]);
        let mut old = project.parse();
        for (n, edited) in [
            a.replace("\treturn 2\n", "\t// two\n\treturn 2\n"),
            a.replace(
                "\treturn 2\n",
                "#ifdef ABSENT\n\tworld.log << 0\n#endif\n\treturn 2\n",
            ),
        ]
        .iter()
        .enumerate()
        {
            project.write("a.dm", edited);
            let (own, full) = project.both(&mut old, &["a.dm"]);
            assert_eq!(own.unwrap(), full);
            let reload = old.reload_file(&project.dir, Path::new("a.dm")).unwrap();
            assert!(reloaded(&reload.changes).is_empty());
            let moved: Vec<String> = reload
                .changes
                .plan
                .line_moves
                .iter()
                .map(|moved| format!("{} {:?}", moved.proc, moved.lines))
                .collect();
            let shift = if n == 0 { 1 } else { 3 };
            assert_eq!(
                moved,
                [
                    format!("/f [(2, 2), (3, {})]", 3 + shift),
                    format!("/g [(5, {})]", 5 + shift)
                ]
            );
        }
    }

    #[test]
    fn lines_that_clash_compile_the_proc_again_as_a_full_parse_does() {
        let (a, b) = (
            "/proc/g()\n\treturn 1\n/proc/f()\n\treturn /datum/u::x\n",
            "\n\n/datum/u\n\tvar/x = 3\n",
        );
        let project = Project::new("clash", &[("a.dm", a), ("b.dm", b)]);
        let mut old = project.parse();
        project.write(
            "a.dm",
            &a.replace("\treturn 1\n", "\tworld.log << 0\n\treturn 1\n"),
        );
        let (own, full) = project.both(&mut old, &["a.dm"]);
        assert_eq!(own.unwrap(), full);
        let reload = old.reload_file(&project.dir, Path::new("a.dm")).unwrap();
        assert_eq!(reloaded(&reload.changes), ["/f", "/g"]);
        assert!(reload.changes.plan.line_moves.is_empty());
    }

    #[test]
    fn a_body_that_starts_reading_a_moved_default_needs_the_full_parse() {
        let one = "/proc/g()\n\treturn 1\n\n/datum/u\n\tvar/x = 3\n";
        let project = Project::new("moved-in-one", &[("a.dm", one)]);
        let mut old = project.parse();
        project.write(
            "a.dm",
            &one.replace("\treturn 1\n", "\tworld.log << 0\n\treturn /datum/u::x\n"),
        );
        match old.reload_file(&project.dir, Path::new("a.dm")) {
            Ok(_) => panic!("reloaded on its own"),
            Err(e) => assert!(
                e.contains("/g reads a var default with :: that moved"),
                "{}",
                e
            ),
        }

        let (a, b) = (
            "/proc/g()\n\treturn 1\n/datum/u\n\tvar/x = 3\n",
            "/proc/f()\n\treturn 1\n/proc/h()\n\treturn 2\n",
        );
        let reads = b.replace("return 1", "return /datum/u::x");
        let project = Project::new("moved-kept", &[("a.dm", a), ("b.dm", b)]);
        let mut old = project.parse();
        project.write(
            "a.dm",
            &a.replace("\treturn 1\n", "\tworld.log << 0\n\treturn 1\n"),
        );
        let reload = old.reload_file(&project.dir, Path::new("a.dm")).unwrap();
        old.keep(reload);
        project.write("b.dm", &reads);
        match old.reload_file(&project.dir, Path::new("b.dm")) {
            Ok(_) => panic!("reloaded on its own"),
            Err(e) => assert!(
                e.contains("/f reads a var default with :: that moved"),
                "{}",
                e
            ),
        }
        old.clear_patches();
        project.write("a.dm", a);
        let (own, full) = project.both(&mut old, &["b.dm"]);
        assert_eq!(own.unwrap(), full);
    }

    const FILE_A: &str = "#define TWO 2
/datum/a
	var/x = 1
/datum/a/proc/z()
	return 1
/datum/a/b/proc/c()
	return TWO
/proc/below()
	return 3
";
    const FILE_B: &str = "/proc/first()
	return 1
/proc/second(n = 4)
	return n
/datum/a/proc/in_b()
	return x
";

    #[test]
    fn files_changed_together_reload_as_a_full_parse_has_them() {
        let project = Project::new("together", &[("a.dm", FILE_A), ("b.dm", FILE_B)]);
        let mut old = project.parse();
        assert!(!old.has_patches());

        let a = FILE_A
            .replace("\treturn 1\n", "\tworld.log << 1\n\treturn 1\n")
            .replace("return TWO", "return TWO + 1");
        let b = FILE_B
            .replace("return n", "return n + 1")
            .replace("return x", "return x + 1");
        project.write("a.dm", &a);
        project.write("b.dm", &b);
        for files in [["a.dm", "b.dm"], ["b.dm", "a.dm"]] {
            let (own, full) = project.both(&mut old, &files);
            assert_eq!(own.unwrap(), full);
        }
        let paths = [Path::new("a.dm"), Path::new("b.dm"), Path::new("a.dm")];
        let reload = old.reload_files(&project.dir, &paths).unwrap();
        assert_eq!(
            reloaded(&reload.changes),
            ["/datum/a/b/c", "/datum/a/in_b", "/datum/a/z", "/second"]
        );
        assert_eq!(reload.changes.plan.line_moves.len(), 1);
        assert_eq!(reload.changes.plan.line_moves[0].proc.to_string(), "/below");
        assert!(reload.changes.refused.is_empty());

        assert!(!old.has_patches());
        let (own, full) = project.both(&mut old, &["a.dm", "b.dm"]);
        assert_eq!(own.unwrap(), full);

        let reload = old.reload_files(&project.dir, &paths).unwrap();
        old.keep(reload);
        assert!(old.has_patches());
        project.write("b.dm", &b.replace("return 1", "return 11"));
        let reload = old.reload_files(&project.dir, &paths).unwrap();
        assert_eq!(reloaded(&reload.changes), ["/first"]);
        assert!(reload.changes.plan.line_moves.is_empty());
        old.clear_patches();
        assert!(!old.has_patches());
    }

    #[test]
    fn a_body_that_doesnt_compile_is_refused_among_files_as_by_a_full_parse() {
        let project = Project::new("refused", &[("a.dm", FILE_A), ("b.dm", FILE_B)]);
        let mut old = project.parse();
        project.write("a.dm", &FILE_A.replace("return 3", "return 33"));
        project.write(
            "b.dm",
            &FILE_B.replace("\treturn x", "\tswitch(x)\n\t\tif(x)\n\t\t\treturn 1"),
        );
        let (own, full) = project.both(&mut old, &["a.dm", "b.dm"]);
        let own = own.unwrap();
        assert!(own.contains("doesn't compile"), "{}", own);
        assert_eq!(own, full);
    }

    #[test]
    fn one_file_that_needs_the_full_parse_takes_the_others_with_it() {
        let a = FILE_A.replace("return 3", "return 33");
        for (name, b, why) in [
            ("var", format!("{FILE_B}/datum/a/var/y = 2\n"), "vars"),
            ("macro", format!("{FILE_B}#define THREE 3\n"), "macros"),
            (
                "proc",
                format!("{FILE_B}/datum/a/proc/more()\n\treturn 5\n"),
                "declarations",
            ),
            (
                "override",
                format!("{FILE_B}/datum/a/b/z()\n\treturn 5\n"),
                "added",
            ),
            (
                "removed",
                FILE_B.replace("/proc/first()\n\treturn 1\n", ""),
                "declarations",
            ),
            ("type", format!("{FILE_B}/datum/new_one\n"), "new type"),
            (
                "parameters",
                FILE_B.replace("second(n = 4)", "second(n = 5)"),
                "parameters",
            ),
            (
                "setting",
                FILE_B.replace("\treturn 1\n", "\tset waitfor = FALSE\n\treturn 1\n"),
                "`set`s",
            ),
            (
                "static",
                FILE_B.replace("\treturn 1\n", "\tvar/static/s = 1\n\treturn 1\n"),
                "static initializers",
            ),
            (
                "include",
                format!("{FILE_B}#include \"c.dm\"\n"),
                "includes",
            ),
            ("error", format!("{FILE_B})\n"), "expected"),
        ] {
            let project = Project::new(
                &format!("with-{name}"),
                &[("a.dm", FILE_A), ("b.dm", FILE_B)],
            );
            project.write("c.dm", "");
            let mut old = project.parse();
            project.write("a.dm", &a);
            project.write("b.dm", &b);
            for files in [["a.dm", "b.dm"], ["b.dm", "a.dm"]] {
                let paths: Vec<&Path> = files.iter().map(Path::new).collect();
                match old.reload_files(&project.dir, &paths) {
                    Ok(_) => panic!("{}: reloaded on their own", name),
                    Err(e) => assert!(e.contains(why), "{}: {}", name, e),
                }
            }
            let reload = old
                .reload_files(&project.dir, &[Path::new("a.dm")])
                .unwrap();
            assert_eq!(reloaded(&reload.changes), ["/below"], "{}", name);
            assert!(!old.has_patches());
        }

        let project = Project::new("with-unread", &[("a.dm", FILE_A), ("b.dm", FILE_B)]);
        let mut old = project.parse();
        project.write("a.dm", &a);
        project.write("c.dm", "/proc/unread()\n\treturn 1\n");
        match old.reload_files(&project.dir, &[Path::new("a.dm"), Path::new("c.dm")]) {
            Ok(_) => panic!("reloaded on their own"),
            Err(e) => assert!(e.contains("not part of the snapshot"), "{}", e),
        }
    }

    #[test]
    fn a_body_that_reads_a_default_another_file_moves_needs_the_full_parse() {
        let (a, b) = (
            "/proc/g()\n\treturn 1\n/datum/u\n\tvar/x = 3\n",
            "/proc/f()\n\treturn 1\n/proc/h()\n\treturn 2\n",
        );
        let moved = a.replace("\treturn 1\n", "\tworld.log << 0\n\treturn 1\n");
        let reads = b.replace("return 1", "return /datum/u::x");
        let project = Project::new("moved-across", &[("a.dm", a), ("b.dm", b)]);
        let mut old = project.parse();
        project.write("a.dm", &moved);
        project.write("b.dm", &reads);
        for files in [["a.dm", "b.dm"], ["b.dm", "a.dm"]] {
            let paths: Vec<&Path> = files.iter().map(Path::new).collect();
            match old.reload_files(&project.dir, &paths) {
                Ok(_) => panic!("reloaded on their own"),
                Err(e) => assert!(
                    e.contains("/f reads a var default with :: that moved"),
                    "{}",
                    e
                ),
            }
        }
        assert!(!old.has_patches());
        project.write("b.dm", b);
        let (own, full) = project.both(&mut old, &["a.dm"]);
        assert_eq!(own.unwrap(), full);
        project.write("a.dm", a);
        project.write("b.dm", &reads);
        let (own, full) = project.both(&mut old, &["b.dm"]);
        assert_eq!(own.unwrap(), full);
        project.write("a.dm", &moved);
        project.write("b.dm", &b.replace("return 2", "return 22"));
        let (own, full) = project.both(&mut old, &["a.dm", "b.dm"]);
        assert_eq!(own.unwrap(), full);
    }

    #[test]
    fn a_strings_text_is_not_a_use_of_a_name() {
        let code = "/proc/named()
	return LIMIT + 1
/proc/as_a_field(datum/d)
	return d.LIMIT
/proc/scoped()
	return /datum/thing::size
/proc/size()
	return 1
/proc/in_a_string()
	return \"LIMIT\"
/proc/in_a_strings_parts(x)
	return \"LIMIT[x]LIMIT\"
/proc/after_a_quote()
	return \"is \\\"LIMIT\"
/proc/in_a_resource()
	return 'LIMIT'
/proc/in_a_strings_code()
	return \"is [LIMIT]\"
";
        let ctx = Context::default();
        let pp = dreammaker::Preprocessor::from_buffer(&ctx, "code.dm".into(), code);
        let mut parser = dreammaker::Parser::new(&ctx, pp);
        parser.enable_procs();
        let tree = parser.parse_object_tree_without_builtins();
        let names: HashSet<&str> = vec!["LIMIT"].into_iter().collect();
        let defaults: HashSet<&str> = vec!["size"].into_iter().collect();
        let mut users: Vec<&str> = tree
            .root()
            .get()
            .procs
            .iter()
            .filter(|(_, proc)| names_any(&proc.value[0], &names, &defaults))
            .map(|(name, _)| name.as_str())
            .collect();
        users.sort();
        assert_eq!(
            users,
            ["as_a_field", "in_a_strings_code", "named", "scoped"]
        );
    }
}
