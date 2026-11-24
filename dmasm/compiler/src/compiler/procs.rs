use std::collections::HashMap;

use dreammaker::ast::Block;
use dreammaker::objtree::{ObjectTree, ProcValue, TypeRef};

use super::{CompileError, Compiler, EvalKind};
use crate::operands::{DMString, GlobalVar, StaticScope, Variable};
use crate::{Instruction, Node};

pub struct ProcSource<'a> {
    pub tree: &'a ObjectTree,
    pub ty: TypeRef<'a>,
    pub name: &'a str,
    pub value: &'a ProcValue,
    pub file: &'a str,
    pub debug_info: bool,
    pub index: Option<usize>,
    pub byond: crate::ByondVersion,
    pub file_names: Option<&'a dyn Fn(dreammaker::FileId) -> String>,
}

#[derive(Debug)]
pub struct CompiledProc {
    pub nodes: Vec<Node>,
    pub locals: Vec<String>,
    pub params: Vec<String>,
    pub statics: Vec<StaticLocal>,
    pub warnings: Vec<crate::diagnostic::Diagnostic>,
    pub byond_bugs: Vec<ByondBug>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByondBug {
    pub id: crate::diagnostic::Id,
    pub line: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct StaticLocal {
    pub name: String,
    pub scope: StaticScope,
    pub value: Option<dreammaker::constants::Constant>,
    pub expression: Option<dreammaker::ast::Expression>,
    pub type_path: String,
    pub constant: bool,
}

const STATIC_LOCAL: u32 = 1 << 31;

#[derive(Clone)]
#[allow(dead_code)]
pub(super) struct ProcCtx<'a> {
    pub tree: &'a ObjectTree,
    pub ty: TypeRef<'a>,
    pub next_lines: &'a [(dreammaker::Location, u32)],
    pub next_line: Option<u32>,
    pub end_lines: &'a [(dreammaker::Location, u32)],
    pub operator_lines: &'a [(dreammaker::Location, u32)],
    pub locals: Vec<String>,
    pub local_types: Vec<String>,
    pub param_types: Vec<String>,
    pub scopes: Vec<Vec<(String, u32)>>,
    pub loops: Vec<LoopLabels>,
    pub iterators: u32,
    pub pending_loop_name: Option<(String, String, usize)>,
    pub static_locals: Vec<String>,
    pub statics: Vec<StaticLocal>,
    pub static_local_types: std::collections::HashMap<(String, u32), String>,
    pub clear_new_vars: bool,
    pub try_depth: u32,
    pub label_iterators: std::collections::HashMap<String, u32>,
    pub bare_labels: std::collections::HashSet<String>,
    pub cleanups: Vec<Instruction>,
    pub local_consts: std::collections::HashMap<(String, u32), dreammaker::constants::Constant>,
    pub proc_path: String,
    pub file: String,
    pub file_id: dreammaker::FileId,
    pub file_names: Option<&'a dyn Fn(dreammaker::FileId) -> String>,
    pub header_line: u32,
}

#[derive(Clone)]
pub(super) struct LoopLabels {
    pub name: Option<String>,
    pub cleanups: usize,
    pub break_cleanups: usize,
    pub continue_label: String,
    pub break_label: String,
    pub block: bool,
}

impl ProcCtx<'_> {
    fn find_declared(&self, name: &str) -> Option<u32> {
        self.scopes
            .iter()
            .rev()
            .flat_map(|scope| scope.iter().rev())
            .find(|(n, _)| n == name)
            .map(|(_, slot)| *slot)
    }

    pub fn find_local(&self, name: &str) -> Option<u32> {
        self.find_declared(name)
            .filter(|slot| slot & STATIC_LOCAL == 0)
    }

    pub fn find_static(&self, name: &str) -> Option<u32> {
        self.find_declared(name)
            .filter(|slot| slot & STATIC_LOCAL != 0)
            .map(|slot| slot & !STATIC_LOCAL)
    }

    pub fn declare_static(
        &mut self,
        name: &str,
        ty: String,
        value: Option<dreammaker::constants::Constant>,
        expression: Option<dreammaker::ast::Expression>,
        constant: bool,
    ) {
        let number = self.static_locals.iter().filter(|s| *s == name).count() as u32;
        self.static_locals.push(name.to_owned());
        self.statics.push(StaticLocal {
            name: name.to_owned(),
            scope: StaticScope::Proc {
                path: self.proc_path.clone(),
                nth: number,
            },
            value,
            expression,
            type_path: ty.clone(),
            constant,
        });
        self.static_local_types
            .insert((name.to_owned(), number), ty);
        if let Some(scope) = self.scopes.last_mut() {
            scope.push((name.to_owned(), number | STATIC_LOCAL));
        }
    }

    pub fn fold_last_static(&mut self, name: &str, value: dreammaker::constants::Constant) {
        let number = self.static_locals.iter().filter(|s| *s == name).count() as u32 - 1;
        self.local_consts.insert((name.to_owned(), number), value);
    }

    pub fn declare_typed_local(&mut self, name: &str, ty: String) -> u32 {
        let slot = self.locals.len() as u32;
        self.locals.push(name.to_owned());
        self.local_types.push(ty);
        self.scopes
            .last_mut()
            .expect("a scope is open")
            .push((name.to_owned(), slot));
        slot
    }
}

pub fn compile_proc(source: &ProcSource) -> Result<CompiledProc, CompileError> {
    compile_proc_as(source, None, &[])
}

pub fn compile_proc_as(
    source: &ProcSource,
    proc_path: Option<&str>,
    statics: &[(String, u32, String)],
) -> Result<CompiledProc, CompileError> {
    let declared: Vec<&dreammaker::ast::Parameter> = source
        .value
        .parameters
        .iter()
        .filter(|p| !p.absolute && p.name.as_str() != "...")
        .collect();
    let params: Vec<String> = declared
        .iter()
        .map(|p| p.name.as_str().to_owned())
        .collect();
    let param_types = declared
        .iter()
        .map(|p| p.var_type.type_path.to_string())
        .collect();

    let mut ctx = proc_ctx(source, param_types);
    if let Some(path) = proc_path {
        ctx.proc_path = path.to_owned();
    }
    for (name, number, ty) in statics {
        ctx.scopes[0].push((name.clone(), number | STATIC_LOCAL));
        ctx.static_local_types
            .insert((name.clone(), *number), ty.clone());
    }
    let mut compiler = Compiler::new(params.clone(), ctx);
    compiler.debug_info = source.debug_info;
    compiler.byond = source.byond;
    compiler.emit_ins(Instruction::DbgFile(DMString(
        source.file.as_bytes().to_vec(),
    )));

    let empty: Block = Box::new([]);
    let code = source.value.code.as_ref().unwrap_or(&empty);
    super::statements::emit_proc_body(&mut compiler, source, code)?;

    compiler.emit_ins(Instruction::End);
    let ctx = compiler.proc;
    Ok(CompiledProc {
        warnings: compiler.warnings.into_inner(),
        byond_bugs: compiler.byond_bugs.into_inner(),
        nodes: compiler.nodes,
        locals: ctx.locals,
        params,
        statics: ctx.statics,
    })
}

pub fn proc_path(ty: TypeRef, name: &str, first: bool) -> String {
    let ty_path = if ty.is_root() {
        ""
    } else {
        ty.get().path.as_str()
    };
    let declared_here = ty
        .get()
        .procs
        .get(name)
        .and_then(|p| p.declaration.as_ref());
    match declared_here {
        Some(decl) if first => format!("{}/{}/{}", ty_path, decl.kind.name(), name),
        _ => format!("{}/{}", ty_path, name),
    }
}

pub(super) fn proc_ctx<'a>(source: &ProcSource<'a>, param_types: Vec<String>) -> ProcCtx<'a> {
    ProcCtx {
        tree: source.tree,
        ty: source.ty,
        next_lines: &source.value.next_lines,
        next_line: None,
        end_lines: &source.value.end_lines,
        operator_lines: &source.value.operator_lines,
        locals: vec![],
        local_types: vec![],
        param_types,
        scopes: vec![vec![]],
        loops: vec![],
        iterators: 0,
        pending_loop_name: None,
        static_locals: vec![],
        statics: vec![],
        static_local_types: Default::default(),
        clear_new_vars: false,
        try_depth: 0,
        label_iterators: Default::default(),
        bare_labels: Default::default(),
        cleanups: vec![],
        local_consts: Default::default(),
        proc_path: {
            let first = source
                .ty
                .get()
                .procs
                .get(source.name)
                .map(|p| match source.index {
                    Some(index) => index == 0,
                    None => std::ptr::eq(&p.value[0], source.value),
                });
            proc_path(source.ty, source.name, first.unwrap_or(false))
        },
        file: source.file.to_owned(),
        file_id: source.value.location.file,
        file_names: source.file_names,
        header_line: source.value.location.line,
    }
}

pub(super) fn find_var(compiler: &Compiler, name: &str) -> Option<EvalKind> {
    let ctx = &compiler.proc;
    if let Some(slot) = ctx.find_local(name) {
        return Some(EvalKind::Var(Variable::Local(slot)));
    }
    if let Some(index) = compiler.params.iter().rposition(|x| x == name) {
        return Some(EvalKind::Var(Variable::Arg(index as u32)));
    }
    if let Some(number) = ctx.find_static(name) {
        let scope = StaticScope::Proc {
            path: ctx.proc_path.clone(),
            nth: number,
        };
        return Some(EvalKind::Var(Variable::Global(GlobalVar::named(
            name.as_bytes(),
            Some(scope),
        ))));
    }
    match name {
        "." | "usr" | "src" | "args" | "world" | "caller" | "callee" | "global" => return None,
        _ => {}
    }
    if compiler.debugger {
        return None;
    }
    let mut scope = String::new();
    if !ctx.ty.is_root() {
        if let Some(decl) = member_var(ctx.ty, name) {
            if !is_global_member(&decl) {
                return Some(EvalKind::Field(
                    super::chain_builder::ChainBuilder::begin(Variable::Src),
                    name.to_owned(),
                ));
            }
            scope = decl.ty().get().path.clone();
        }
    }
    Some(EvalKind::Var(Variable::Global(GlobalVar::named(
        name.as_bytes(),
        Some(StaticScope::of_type(&scope)),
    ))))
}

pub(super) fn call_var(ty: Option<TypeRef>, name: &str) -> Variable {
    let is_verb = ty
        .and_then(|t| t.get_proc_declaration(name))
        .is_some_and(|d| d.kind == dreammaker::ast::ProcDeclKind::Verb);
    let name = call_name(ty, name);
    if is_verb {
        Variable::DynamicVerb(name)
    } else {
        Variable::DynamicProc(name)
    }
}

pub(super) fn call_name(ty: Option<TypeRef>, name: &str) -> DMString {
    let mut current = ty;
    while let Some(t) = current {
        if let Some(proc) = t.get().procs.get(name) {
            for value in &proc.value {
                for stmt in value.code.iter().flat_map(|c| c.iter()) {
                    if let dreammaker::ast::Statement::Setting {
                        name: setting,
                        value,
                        ..
                    } = &stmt.elem
                    {
                        if setting.as_str() == "name" {
                            if let Some(s) = value.as_term().and_then(|t| match t {
                                dreammaker::ast::Term::String(s) => Some(s.clone()),
                                _ => None,
                            }) {
                                return super::strings::parse(s.as_str())
                                    .unwrap_or_else(|_| DMString(s.as_str().as_bytes().to_vec()));
                            }
                        }
                    }
                }
            }
            if proc.declaration.is_some() {
                break;
            }
        }
        current = t.parent_type();
    }
    super::dynamic_name(name)
}

pub(super) fn emit_bare_call(
    compiler: &mut Compiler,
    ident: &str,
    args: Vec<dreammaker::ast::Expression>,
) -> Result<Option<EvalKind>, CompileError> {
    let ctx = &compiler.proc;
    if compiler.debugger {
        return Ok(None);
    }
    let ty = ctx.ty;
    let tree = ctx.tree;
    if !ty.is_root() && ty.get_proc(ident).is_some_and(|p| !p.ty().is_root()) {
        let count = compiler.emit_call_args(args)?;
        let var = call_var(Some(ty), ident);
        compiler.emit_ins(Instruction::Call(
            Variable::SetCache(Box::new(Variable::Src), Box::new(var)),
            count,
        ));
        return Ok(Some(EvalKind::Stack));
    }
    if tree.root().get_proc(ident).is_some() {
        let path = crate::operands::Proc(format!("/proc/{}", ident));
        let count = args.len() as u32;
        match super::args::emit(compiler, super::args::ArgsContext::Proc, args)? {
            super::args::ArgsResult::Normal => {
                compiler.emit_ins(Instruction::CallGlob(count, path))
            }
            super::args::ArgsResult::Assoc => {
                compiler.emit_ins(Instruction::NewAssocList(count));
                compiler.emit_ins(Instruction::CallGlobalArgList(path));
            }
            super::args::ArgsResult::ArgList => {
                compiler.emit_ins(Instruction::CallGlobalArgList(path))
            }
        }
        return Ok(Some(EvalKind::Stack));
    }
    Ok(None)
}

pub(super) fn member_var<'a>(
    ty: TypeRef<'a>,
    name: &str,
) -> Option<dreammaker::objtree::VarDeclarationRef<'a>> {
    ty.get_var_declaration(name).filter(|d| !d.ty().is_root())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum After {
    Typed,
    Index,
    Call,
}

impl After {
    pub(super) fn of(follow: &dreammaker::ast::Follow) -> After {
        match follow {
            dreammaker::ast::Follow::Index(..) => After::Index,
            _ => After::Typed,
        }
    }
}

pub(super) fn field_var<'a>(
    compiler: &Compiler<'a>,
    ty: Option<TypeRef<'a>>,
    kind: dreammaker::ast::PropertyAccessKind,
    name: &str,
    after: After,
) -> Option<dreammaker::objtree::VarDeclarationRef<'a>> {
    use dreammaker::ast::PropertyAccessKind;
    match (kind, after) {
        (PropertyAccessKind::Scope, _) => ty.and_then(|t| member_var(t, name)),
        (_, After::Index) => colon_var(compiler, None, name),
        (PropertyAccessKind::Colon | PropertyAccessKind::SafeColon, After::Call) => {
            colon_var(compiler, None, name)
        }
        (PropertyAccessKind::Colon | PropertyAccessKind::SafeColon, After::Typed) => {
            colon_var(compiler, ty, name)
        }
        (PropertyAccessKind::Dot | PropertyAccessKind::SafeDot, After::Call) => match ty {
            Some(t) => member_var(t, name),
            None => colon_var(compiler, None, name),
        },
        (PropertyAccessKind::Dot | PropertyAccessKind::SafeDot, After::Typed) => {
            ty.and_then(|t| member_var(t, name))
        }
    }
}

pub(super) fn call_result<'a>(
    compiler: &Compiler<'a>,
    receiver: Option<TypeRef<'a>>,
    access: dreammaker::ast::PropertyAccessKind,
    name: &str,
) -> (Option<TypeRef<'a>>, After) {
    use dreammaker::ast::{ProcReturnType, PropertyAccessKind};
    let ty = match receiver {
        Some(t)
            if matches!(
                access,
                PropertyAccessKind::Colon | PropertyAccessKind::SafeColon
            ) =>
        {
            Some(colon_proc_type(t, name).unwrap_or(t))
        }
        Some(t) => Some(t),
        None => first_declaring_type(compiler.proc.tree, name),
    };
    let Some(declaration) = ty.and_then(|t| t.get_proc_declaration(name)) else {
        return (None, After::Typed);
    };
    if declaration.location.file == dreammaker::FileId::BUILTINS {
        return (None, After::Typed);
    }
    let returned = match &declaration.return_type {
        ProcReturnType::TypePath(path) => find_type(compiler, &path.to_string()),
        ProcReturnType::InputType(_) => None,
    };
    (returned, After::Call)
}

pub(super) fn start_type<'a>(
    compiler: &Compiler<'a>,
    term: &dreammaker::ast::Term,
) -> (Option<TypeRef<'a>>, After) {
    use dreammaker::ast::{PropertyAccessKind, Term};
    match term {
        Term::Call(name, _) => {
            let ctx = &compiler.proc;
            let receiver = match ctx.ty.get_proc(name.as_str()) {
                Some(_) => ctx.ty,
                None => ctx.tree.root(),
            };
            call_result(compiler, Some(receiver), PropertyAccessKind::Dot, name)
        }
        _ => (term_type(compiler, term), After::Typed),
    }
}

pub(super) fn colon_var<'a>(
    compiler: &Compiler<'a>,
    ty: Option<TypeRef<'a>>,
    name: &str,
) -> Option<dreammaker::objtree::VarDeclarationRef<'a>> {
    if let Some(decl) = ty.and_then(|t| member_var(t, name)) {
        return Some(decl);
    }
    ty.and_then(|t| declared_below(t, name))
        .or_else(|| declared_below(compiler.proc.tree.root(), name))
}

fn declared_below<'a>(
    ty: TypeRef<'a>,
    name: &str,
) -> Option<dreammaker::objtree::VarDeclarationRef<'a>> {
    let mut children: Vec<TypeRef<'a>> = ty.children().collect();
    children.sort_by_key(|child| child.index());
    children.into_iter().rev().find_map(|child| {
        declared_below(child, name).or_else(|| {
            child.get().vars.get(name)?.declaration.as_ref()?;
            member_var(child, name)
        })
    })
}

static TREES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn new_tree() {
    TREES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

pub(super) fn first_declaring_type<'a>(tree: &'a ObjectTree, name: &str) -> Option<TypeRef<'a>> {
    thread_local! {
        static FIRST: std::cell::RefCell<((usize, usize), HashMap<String, String>)> =
            std::cell::RefCell::new(((0, 0), HashMap::new()));
    }
    let key = (
        tree as *const ObjectTree as usize,
        TREES.load(std::sync::atomic::Ordering::Relaxed),
    );
    let path = FIRST.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.0 != key {
            let mut first: HashMap<String, (dreammaker::objtree::SymbolId, String)> =
                HashMap::new();
            for ty in tree.iter_types() {
                for (proc_name, proc) in ty.get().procs.iter() {
                    let Some(decl) = &proc.declaration else {
                        continue;
                    };
                    let entry = first
                        .entry(proc_name.to_string())
                        .or_insert((decl.id, ty.get().path.clone()));
                    if decl.id < entry.0 {
                        *entry = (decl.id, ty.get().path.clone());
                    }
                }
            }
            *cell = (key, first.into_iter().map(|(k, (_, p))| (k, p)).collect());
        }
        cell.1.get(name).cloned()
    })?;
    tree.find(&path)
}

pub(super) fn list_elem_type<'a>(compiler: &Compiler<'a>, path: &str) -> Option<TypeRef<'a>> {
    let rest = path.strip_prefix("/list/")?;
    compiler.proc.tree.find(&format!("/{rest}"))
}

pub(super) fn colon_proc_type<'a>(ty: TypeRef<'a>, name: &str) -> Option<TypeRef<'a>> {
    if ty.get_proc(name).is_some() {
        return Some(ty);
    }
    ty.children().find_map(|child| colon_proc_type(child, name))
}

pub(super) fn is_global_member(decl: &dreammaker::objtree::VarDeclarationRef) -> bool {
    let flags = decl.get().var_type.flags;
    flags.is_static()
        || (flags.is_const() && decl.get().location.file != dreammaker::FileId::BUILTINS)
}

pub(super) fn static_chain<'a>(
    compiler: &Compiler<'a>,
    mut ty: Option<TypeRef<'a>>,
    names: &[&str],
) -> Option<dreammaker::objtree::VarDeclarationRef<'a>> {
    let (last, path) = names.split_last()?;
    for name in path {
        let decl = member_var(ty?, name)?;
        ty = declared_type(compiler, &decl.get().var_type);
    }
    member_var(ty?, last)
}

pub(super) fn declared_type<'a>(
    compiler: &Compiler<'a>,
    var_type: &dreammaker::ast::VarType,
) -> Option<TypeRef<'a>> {
    let path = var_type.type_path.to_string();
    find_type(compiler, &path)
}

fn find_type<'a>(compiler: &Compiler<'a>, path: &str) -> Option<TypeRef<'a>> {
    if path.is_empty() || path == "/" {
        return None;
    }
    let path = if path.starts_with("/list/") {
        "/list"
    } else {
        path
    };
    compiler.proc.tree.find(path)
}

pub(super) fn term_type<'a>(
    compiler: &Compiler<'a>,
    term: &dreammaker::ast::Term,
) -> Option<TypeRef<'a>> {
    use dreammaker::ast::Term;
    let ctx = &compiler.proc;
    let name = match term {
        Term::Ident(name) => name.as_str(),
        Term::GlobalIdent(name) => {
            let decl = ctx.tree.root().get_var_declaration(name.as_str())?;
            return declared_type(compiler, &decl.get().var_type);
        }
        _ => return None,
    };
    if let Some(slot) = ctx.find_local(name) {
        return find_type(compiler, &ctx.local_types[slot as usize]);
    }
    if let Some(index) = compiler.params.iter().rposition(|x| x == name) {
        return find_type(compiler, &ctx.param_types[index]);
    }
    if let Some(ty) = ctx
        .find_static(name)
        .and_then(|n| ctx.static_local_types.get(&(name.to_owned(), n)))
    {
        return find_type(compiler, ty);
    }
    match name {
        "src" => return if ctx.ty.is_root() { None } else { Some(ctx.ty) },
        "usr" => return ctx.tree.find("/mob"),
        "world" => return ctx.tree.find("/world"),
        "global" => return None,
        _ => {}
    }
    if !ctx.ty.is_root() {
        if let Some(decl) = member_var(ctx.ty, name) {
            return declared_type(compiler, &decl.get().var_type);
        }
    }
    let decl = ctx.tree.root().get_var_declaration(name)?;
    declared_type(compiler, &decl.get().var_type)
}

pub(super) fn expr_type<'a>(
    compiler: &Compiler<'a>,
    expr: &dreammaker::ast::Expression,
) -> Option<TypeRef<'a>> {
    use dreammaker::ast::{Expression, Follow, PropertyAccessKind, Term};
    let Expression::Base { term, follow } = expr else {
        return None;
    };
    if let Term::Expr(inner) = &term.elem {
        if follow.is_empty() {
            return expr_type(compiler, inner);
        }
    }
    let global = matches!(&term.elem, Term::Ident(i) if i.as_str() == "global")
        && matches!(follow.first().map(|f| &f.elem), Some(Follow::Field(..)));
    let mut ty = if global {
        let decl = compiler
            .proc
            .tree
            .root()
            .get_var_declaration(match &follow[0].elem {
                Follow::Field(_, name) => name.as_str(),
                _ => unreachable!(),
            })?;
        declared_type(compiler, &decl.get().var_type)
    } else {
        term_type(compiler, &term.elem)
    };
    for f in &follow[global as usize..] {
        let decl = match &f.elem {
            Follow::Field(PropertyAccessKind::Colon | PropertyAccessKind::SafeColon, name) => {
                colon_var(compiler, ty, name.as_str())?
            }
            Follow::Field(_, name) => member_var(ty?, name.as_str())?,
            _ => return None,
        };
        ty = declared_type(compiler, &decl.get().var_type);
    }
    ty
}

pub(super) fn field_globals(
    compiler: &Compiler,
    expr: &dreammaker::ast::Expression,
) -> Vec<Option<String>> {
    use dreammaker::ast::{Expression, Follow, Term};
    let Expression::Base { term, follow } = expr else {
        return vec![];
    };
    let global = matches!(&term.elem, Term::Ident(i) if i.as_str() == "global");
    let (mut ty, mut after) = match global {
        true => (None, After::Typed),
        false => start_type(compiler, &term.elem),
    };
    let mut found = vec![];
    for (i, f) in follow.iter().enumerate() {
        let before = std::mem::replace(&mut after, After::of(&f.elem));
        let (kind, name) = match &f.elem {
            Follow::Field(kind, name) => (kind, name),
            Follow::Index(..) => {
                found.push(None);
                ty = None;
                continue;
            }
            Follow::Call(access, name, _) => {
                found.push(None);
                let receiver = match global && i == 0 {
                    true => Some(compiler.proc.tree.root()),
                    false => ty,
                };
                (ty, after) = call_result(compiler, receiver, *access, name.as_str());
                continue;
            }
            _ => break,
        };
        let decl = match kind {
            _ if global && i == 0 => compiler.proc.tree.root().get_var_declaration(name),
            _ => field_var(compiler, ty, *kind, name.as_str(), before),
        };
        found.push(
            decl.filter(|d| !d.ty().is_root() && is_global_member(d))
                .map(|d| d.ty().get().path.clone()),
        );
        ty = decl.and_then(|d| declared_type(compiler, &d.get().var_type));
    }
    found
}

pub(super) fn resolve_proc_path(compiler: &Compiler, path: &str) -> Option<String> {
    let ctx = &compiler.proc;
    let (type_part, sep, kind, name) = ["proc", "verb"].iter().find_map(|kind| {
        for sep in ['.', '/', ':'] {
            for end in ['/', '.'] {
                let marker = format!("{}{}{}", sep, kind, end);
                if let Some(i) = path.find(&marker) {
                    return Some((&path[..i], sep, *kind, &path[i + marker.len()..]));
                }
            }
        }
        None
    })?;
    let written_on_type = sep == '/' && !type_part.is_empty();
    let warn_elsewhere = |found: &str| {
        if written_on_type {
            compiler.warn(crate::diagnostic::Id::ProcPathNotDeclared, &format!(
                "{path}: DreamMaker refuses this path, since {name} isn't declared on {type_part}; this is {found}"
            ));
        }
    };
    let ty = if type_part.is_empty() {
        if path.starts_with('/') {
            ctx.tree.root()
        } else {
            ctx.ty
        }
    } else {
        ctx.tree.find(type_part)?
    };
    let mut current = Some(ty);
    while let Some(t) = current {
        if let Some(proc) = t.get().procs.get(name) {
            if proc.declaration.is_some() {
                let prefix = if t.is_root() {
                    ""
                } else {
                    t.get().path.as_str()
                };
                let found = format!("{}/{}/{}", prefix, kind, name);
                if t != ty {
                    warn_elsewhere(&found);
                }
                return Some(found);
            }
        }
        current = t.parent_type();
    }
    if ctx.tree.root().get().procs.contains_key(name) {
        let found = format!("/{}/{}", kind, name);
        if ty != ctx.tree.root() {
            warn_elsewhere(&found);
        }
        return Some(found);
    }
    None
}

pub(super) fn declared_type_path(
    compiler: &Compiler,
    expr: &dreammaker::ast::Expression,
) -> Option<String> {
    use dreammaker::ast::{Expression, Follow, Term};
    let Expression::Base { term, follow } = expr else {
        return None;
    };
    let ctx = &compiler.proc;
    match follow.last().map(|f| &f.elem) {
        None => {
            let Term::Ident(name) = &term.elem else {
                return None;
            };
            let name = name.as_str();
            if let Some(slot) = ctx.find_local(name) {
                return Some(ctx.local_types[slot as usize].clone());
            }
            if let Some(i) = compiler.params.iter().rposition(|x| x == name) {
                return Some(ctx.param_types[i].clone());
            }
            if let Some(ty) = ctx
                .find_static(name)
                .and_then(|n| ctx.static_local_types.get(&(name.to_owned(), n)))
            {
                return Some(ty.clone());
            }
            let decl = if ctx.ty.is_root() {
                None
            } else {
                member_var(ctx.ty, name)
            }
            .or_else(|| ctx.tree.root().get_var_declaration(name))?;
            Some(decl.get().var_type.type_path.to_string())
        }
        Some(Follow::Field(_, name)) => {
            let base = Expression::Base {
                term: term.clone(),
                follow: follow[..follow.len() - 1].to_vec().into_boxed_slice(),
            };
            let ty = expr_type(compiler, &base)?;
            let decl = member_var(ty, name.as_str())?;
            Some(decl.get().var_type.type_path.to_string())
        }
        _ => None,
    }
}

pub(super) fn label_targets(ins: &Instruction) -> Vec<String> {
    use crate::operands::Label;
    use Instruction::*;
    let named = |l: &Label| match l {
        Label::Named(n) => Some(n.clone()),
        Label::Offset(_) => None,
    };
    match ins {
        Jmp(l)
        | Jz(l)
        | Jnz(l)
        | JmpLoop(l)
        | JzLoop(l)
        | JnzLoop(l)
        | JmpOr(l)
        | JmpAnd(l)
        | SetCacheJmpIfNull(l)
        | SetCachePopJmpIfNull(l)
        | Spawn(l)
        | Try(l)
        | Catch(l)
        | TryJmp(l)
        | ForRange(l, _)
        | ForRangeStep(l, _) => named(l).into_iter().collect(),
        Switch(p) => p
            .cases
            .iter()
            .map(|c| &c.1)
            .chain([&p.default])
            .filter_map(named)
            .collect(),
        SwitchRange(p) => p
            .cases
            .iter()
            .map(|c| &c.1)
            .chain(p.range_cases.iter().map(|c| &c.2))
            .chain([&p.default])
            .filter_map(named)
            .collect(),
        PickSwitch(p) => p
            .cases
            .iter()
            .map(|c| &c.1)
            .chain([&p.default])
            .filter_map(named)
            .collect(),
        PickProb(p) => p.cases.iter().filter_map(named).collect(),
        _ => vec![],
    }
}

pub fn labels_mut(ins: &mut Instruction) -> Vec<&mut crate::operands::Label> {
    use Instruction::*;
    match ins {
        Jmp(l)
        | Jz(l)
        | Jnz(l)
        | JmpLoop(l)
        | JzLoop(l)
        | JnzLoop(l)
        | JmpOr(l)
        | JmpAnd(l)
        | SetCacheJmpIfNull(l)
        | SetCachePopJmpIfNull(l)
        | Spawn(l)
        | Try(l)
        | Catch(l)
        | TryJmp(l)
        | ForRange(l, _)
        | ForRangeStep(l, _) => vec![l],
        Switch(p) => p
            .cases
            .iter_mut()
            .map(|c| &mut c.1)
            .chain([&mut p.default])
            .collect(),
        SwitchRange(p) => p
            .cases
            .iter_mut()
            .map(|c| &mut c.1)
            .chain(p.range_cases.iter_mut().map(|c| &mut c.2))
            .chain([&mut p.default])
            .collect(),
        PickSwitch(p) => p
            .cases
            .iter_mut()
            .map(|c| &mut c.1)
            .chain([&mut p.default])
            .collect(),
        PickProb(p) => p.cases.iter_mut().collect(),
        _ => vec![],
    }
}

pub(super) fn local_const_value(
    folded: super::fold::Constant,
) -> Option<dreammaker::constants::Constant> {
    use dreammaker::constants::Constant as C;
    match folded {
        super::fold::Constant::Number(n) => Some(C::Float(n)),
        super::fold::Constant::Text(t) if !t.iter().any(|b| matches!(b, 0xFF | b'\\' | b'[')) => {
            String::from_utf8(t).ok().map(|t| C::String(t.into()))
        }
        _ => None,
    }
}

pub(super) fn const_value(
    compiler: &Compiler,
    name: &str,
) -> Option<dreammaker::constants::Constant> {
    const_lookup(compiler, name).map(|(value, _)| value)
}

pub(super) fn global_const_value(
    compiler: &Compiler,
    name: &str,
) -> Option<dreammaker::constants::Constant> {
    let root = compiler.proc.tree.root();
    let decl = root.get_var_declaration(name)?;
    if !decl.get().var_type.flags.is_const()
        || decl.get().location.file == dreammaker::FileId::BUILTINS
    {
        return None;
    }
    root.get_value(name)?.constant.clone()
}

pub(super) fn is_builtin_const(compiler: &Compiler, name: &str) -> bool {
    const_lookup(compiler, name).is_some_and(|(_, builtin)| builtin)
}

fn const_lookup(
    compiler: &Compiler,
    name: &str,
) -> Option<(dreammaker::constants::Constant, bool)> {
    let ctx = &compiler.proc;
    if ctx.find_local(name).is_some() || compiler.params.iter().any(|p| p == name) {
        return None;
    }
    if compiler.debugger && name == "vars" {
        return None;
    }
    if let Some(number) = ctx.find_static(name) {
        let folded = ctx.local_consts.get(&(name.to_owned(), number));
        return folded.map(|c| (c.clone(), false));
    }
    let ty = if !ctx.ty.is_root() && member_var(ctx.ty, name).is_some() {
        ctx.ty
    } else {
        ctx.tree.root()
    };
    let decl = ty.get_var_declaration(name)?;
    if !decl.get().var_type.flags.is_const() {
        return None;
    }
    let builtin = decl.get().location.file == dreammaker::FileId::BUILTINS;
    if !ty.is_root() && builtin {
        return None;
    }
    Some((ty.get_value(name)?.constant.clone()?, builtin))
}

pub(super) fn emit_type_scope(
    compiler: &mut Compiler,
    path: &str,
    follow: &dreammaker::ast::Follow,
) -> Result<EvalKind, CompileError> {
    use dreammaker::ast::Follow;
    let tree = compiler.proc.tree;
    let ty = tree
        .find(path)
        .ok_or(CompileError::UnsupportedStatement("::"))?;
    match follow {
        Follow::StaticField(name) if name.as_str() == "type" || name.as_str() == "parent_type" => {
            let path = if name.as_str() == "type" {
                ty.get().path.clone()
            } else {
                ty.parent_type()
                    .map(|p| p.get().path.clone())
                    .unwrap_or_default()
            };
            compiler.emit_ins(Instruction::PushVal(crate::operands::Value::Path(path)));
            Ok(EvalKind::Stack)
        }
        Follow::StaticField(name) => {
            let decl =
                member_var(ty, name.as_str()).ok_or(CompileError::UnsupportedStatement("::"))?;
            if is_global_member(&decl) {
                let scope = decl.ty().get().path.clone();
                return Ok(EvalKind::Var(Variable::Global(GlobalVar::named(
                    name.as_str().as_bytes(),
                    Some(StaticScope::of_type(&scope)),
                ))));
            }
            let value = ty.get_value(name.as_str());
            if let Some(at) = value.filter(|v| v.expression.is_some()).map(|v| v.location) {
                let ctx = &compiler.proc;
                if at.file == ctx.file_id {
                    compiler.emit_ins(Instruction::DbgLine(at.line));
                    compiler.line_pending = true;
                } else if let Some(names) = ctx.file_names {
                    let file = names(at.file);
                    compiler.emit_ins(Instruction::DbgFile(DMString(file.into_bytes())));
                    compiler.last_line = None;
                    compiler.emit_ins(Instruction::DbgLine(at.line));
                    compiler.other_file = true;
                    compiler.line_pending = true;
                }
            }
            use dreammaker::constants::Constant as C;
            match value.and_then(|v| v.constant.clone()) {
                Some(
                    constant @ (C::Null(_)
                    | C::Float(_)
                    | C::String(_)
                    | C::Resource(_)
                    | C::Prefab(_)),
                ) => compiler.emit_constant(constant)?,
                _ => match value.and_then(|v| v.expression.clone()) {
                    Some(expr) => {
                        let kind = compiler.emit_expr(expr)?;
                        compiler.emit_move_to_stack(kind)?;
                    }
                    None => compiler.emit_constant(dreammaker::constants::Constant::Null(None))?,
                },
            }
            Ok(EvalKind::Stack)
        }
        Follow::ProcReference(name) => {
            let proc_path = type_proc_path(ty, name.as_str())?;
            compiler.emit_ins(Instruction::PushVal(crate::operands::Value::Path(
                proc_path,
            )));
            Ok(EvalKind::Stack)
        }
        _ => unreachable!(),
    }
}

pub fn type_proc_path(ty: TypeRef, name: &str) -> Result<String, CompileError> {
    let mut current = Some(ty);
    while let Some(t) = current {
        if t.get().procs.contains_key(name) {
            if !t.is_root() {
                if let Some(found) = last_definition(t, name)? {
                    return Ok(found);
                }
            }
            break;
        }
        current = t.parent_type();
    }
    let mut current = Some(ty);
    while let Some(t) = current {
        if t.get()
            .procs
            .get(name)
            .is_some_and(|p| p.declaration.is_some())
        {
            let prefix = if t.is_root() {
                ""
            } else {
                t.get().path.as_str()
            };
            return Ok(format!("{}/proc/{}", prefix, name));
        }
        current = t.parent_type();
    }
    match ty.tree().root().get().procs.contains_key(name) {
        true => Ok(format!("/proc/{}", name)),
        false => Err(CompileError::UnsupportedStatement("::")),
    }
}

pub fn resolve_proc_references(tree: &mut ObjectTree) {
    use dreammaker::ast::Follow;
    let mut found = vec![];
    let mut reads = vec![];
    for ty in tree.iter_types() {
        for (var_name, var) in ty.get().vars.iter() {
            let Some((prefab, follow)) = var.value.expression.as_ref().and_then(type_scope_read)
            else {
                continue;
            };
            let Some(on) = tree.find(&prefab.path.to_string()) else {
                continue;
            };
            match follow {
                Follow::ProcReference(name) => {
                    let constant = type_proc_path(on, name.as_str()).ok().map(|path| {
                        dreammaker::constants::Constant::Prefab(Box::new(
                            dreammaker::ast::AbsolutePath::from(path.as_str()).into(),
                        ))
                    });
                    found.push((ty.index(), var_name.clone(), constant));
                }
                Follow::StaticField(name)
                    if !on.get_var_declaration(name.as_str()).is_some_and(|d| {
                        let flags = d.get().var_type.flags;
                        flags.is_static() && !flags.is_const()
                    }) =>
                {
                    reads.push((
                        ty.index(),
                        var_name.clone(),
                        on.get().path.clone(),
                        name.clone(),
                    ))
                }
                _ => {}
            }
        }
    }
    let mut changed: std::collections::HashSet<dreammaker::ast::Ident> =
        found.iter().map(|(_, name, _)| name.clone()).collect();
    for (ty, name, constant) in found {
        if let Some(var) = tree[ty].vars.get_mut(&name) {
            var.value.constant = constant;
        }
    }
    for _ in 0..reads.len() {
        let mut again = false;
        for (ty, name, on, read) in &reads {
            if !changed.contains(read) {
                continue;
            }
            let constant = tree
                .find(on)
                .and_then(|on| on.get_value(read.as_str()))
                .and_then(|value| value.constant.clone());
            let Some(var) = tree[*ty].vars.get_mut(name) else {
                continue;
            };
            if var.value.constant != constant {
                var.value.constant = constant;
                changed.insert(name.clone());
                again = true;
            }
        }
        if !again {
            break;
        }
    }
}

fn type_scope_read(
    expression: &dreammaker::ast::Expression,
) -> Option<(&dreammaker::ast::Prefab, &dreammaker::ast::Follow)> {
    use dreammaker::ast::{Expression, Term};
    let mut expression = expression;
    while let Expression::Base { term, follow } = expression {
        match (&term.elem, &follow[..]) {
            (Term::Expr(inner), []) => expression = inner,
            _ => break,
        }
    }
    let Expression::Base { term, follow } = expression else {
        return None;
    };
    let [follow] = &follow[..] else {
        return None;
    };
    let mut term = &term.elem;
    while let Term::Expr(inner) = term {
        match &**inner {
            Expression::Base {
                term: inner,
                follow,
            } if follow.is_empty() => term = &inner.elem,
            _ => break,
        }
    }
    match term {
        Term::Prefab(prefab) if prefab.vars.is_empty() => Some((prefab, &follow.elem)),
        _ => None,
    }
}

fn last_definition(t: TypeRef, name: &str) -> Result<Option<String>, CompileError> {
    let Some(proc) = t.get().procs.get(name) else {
        return Ok(None);
    };
    let is_declaration = |v: &ProcValue| {
        proc.declaration
            .as_ref()
            .is_some_and(|d| d.location == v.location)
    };
    let mut order = definition_order(t, name);
    if proc
        .declaration
        .as_ref()
        .is_some_and(|d| d.kind == dreammaker::ast::ProcDeclKind::Verb)
    {
        order.retain(|&i| !is_declaration(&proc.value[i]));
        if order.is_empty() {
            return Err(CompileError::BadProcReference(format!(
                "{} is a verb with no override on {}",
                name,
                t.get().path
            )));
        }
    }
    let overrides = order
        .iter()
        .filter(|&&i| !is_declaration(&proc.value[i]))
        .count();
    match order.last().map(|&i| &proc.value[i]) {
        Some(last) if !is_declaration(last) => match overrides {
            1 => Ok(Some(format!("{}/{}", t.get().path, name))),
            _ => Err(CompileError::UnsupportedStatement(
                "type::proc() that names a type's second or later override",
            )),
        },
        _ => Ok(None),
    }
}

const STDDEF_PROC_BLOCKS: &[&str] = &[
    "/sound",
    "/icon",
    "/matrix",
    "/database",
    "/database/query",
    "/regex",
    "/generator",
];

pub fn definition_order(ty: TypeRef, name: &str) -> Vec<usize> {
    definition_order_at(ty, name, |location| location)
}

pub fn definition_order_at(
    ty: TypeRef,
    name: &str,
    at: impl Fn(dreammaker::Location) -> dreammaker::Location,
) -> Vec<usize> {
    let Some(proc) = ty.get().procs.get(name) else {
        return vec![];
    };
    let declaration = proc
        .declaration
        .as_ref()
        .filter(|d| d.location.file != dreammaker::FileId::BUILTINS);
    let place = |v: &ProcValue| match declaration {
        Some(d) if v.location == d.location => {
            if d.kind == dreammaker::ast::ProcDeclKind::Proc
                && STDDEF_PROC_BLOCKS.contains(&ty.get().path.as_str())
            {
                return None;
            }
            ty.get()
                .procs
                .values()
                .filter_map(|p| p.declaration.as_ref())
                .filter(|other| {
                    other.kind == d.kind && other.location.file != dreammaker::FileId::BUILTINS
                })
                .map(|other| at(other.location))
                .min()
        }
        _ => Some(at(v.location)),
    };
    let mut order: Vec<usize> = (0..proc.value.len())
        .filter(|&i| proc.value[i].location.file != dreammaker::FileId::BUILTINS)
        .collect();
    order.sort_by_key(|&i| place(&proc.value[i]));
    order
}

pub(super) fn touches_cache(ins: &Instruction) -> bool {
    use Instruction::*;
    if matches!(
        ins,
        PushCache
            | PopCache
            | PushCacheKey
            | PopCacheKey
            | SetCacheJmpIfNull(_)
            | SetCachePopJmpIfNull(_)
    ) {
        return true;
    }
    fn uses(var: &Variable) -> bool {
        match var {
            Variable::Null
            | Variable::World
            | Variable::Usr
            | Variable::Src
            | Variable::Args
            | Variable::Dot
            | Variable::Caller
            | Variable::Callee
            | Variable::Arg(_)
            | Variable::Local(_)
            | Variable::Global(_)
            | Variable::StaticProc(_)
            | Variable::StaticVerb(_) => false,
            Variable::Initial(v)
            | Variable::IsSaved(v)
            | Variable::PtrRef(v)
            | Variable::PtrDeref(v) => uses(v),
            _ => true,
        }
    }
    let mut ins = ins.clone();
    var_operand(&mut ins).is_some_and(|v| uses(v))
}

pub(super) fn var_operand(ins: &mut Instruction) -> Option<&mut Variable> {
    use Instruction::*;
    match ins {
        GetVar(v)
        | SetVar(v)
        | SetVarExpr(v)
        | AugAdd(v)
        | AugSub(v)
        | AugMul(v)
        | AugDiv(v)
        | AugMod(v)
        | AugBand(v)
        | AugBor(v)
        | AugXor(v)
        | AugLShift(v)
        | AugRShift(v)
        | AugFloatMod(v)
        | AssignInto(v)
        | PreInc(v)
        | PostInc(v)
        | PreDec(v)
        | PostDec(v)
        | Inc(v)
        | Dec(v)
        | Call(v, _)
        | CallStatement(v, _)
        | KeyValueIter(v) => Some(v),
        ForRange(_, v) | ForRangeStep(_, v) => Some(v),
        _ => None,
    }
}

pub(super) fn track_cache(
    cache: &mut Option<Vec<Variable>>,
    ins: &mut Instruction,
    explicit_src: &mut bool,
) {
    track_cache_inner(cache, ins, explicit_src);
    *explicit_src = false;
    use Instruction::*;
    let call = matches!(
        ins,
        CallGlob(..)
            | CallGlobalArgList(_)
            | CallPath(_)
            | CallName(_)
            | CallNameArgList
            | CallPathArgList
            | CallSelf
            | CallSelfArgs(_)
            | CallSelfArgList
            | Del
            | Sleep
    ) || matches!(ins, Call(v, _) | CallStatement(v, _) if !uses_cache(v))
        || matches!(
            ins,
            PreInc(_) | PostInc(_) | PreDec(_) | PostDec(_) | PushEval
        );
    if call && cache.as_ref().is_some_and(|held| held.len() > 1) {
        *cache = None;
    }
}

pub(super) fn uses_cache(var: &Variable) -> bool {
    match var {
        Variable::Cache | Variable::CacheKey | Variable::CacheIndex => true,
        Variable::SetCache(..) | Variable::Field(_) => true,
        Variable::DynamicProc(_)
        | Variable::DynamicVerb(_)
        | Variable::StaticProc(_)
        | Variable::StaticVerb(_) => true,
        _ => false,
    }
}

fn track_cache_inner(
    cache: &mut Option<Vec<Variable>>,
    ins: &mut Instruction,
    explicit_src: &mut bool,
) {
    match ins {
        Instruction::SetVar(Variable::Cache)
        | Instruction::PopCache
        | Instruction::SetCacheJmpIfNull(_)
        | Instruction::SetCachePopJmpIfNull(_) => {
            *cache = None;
            return;
        }
        _ => {}
    }

    let writes = !matches!(
        ins,
        Instruction::GetVar(_) | Instruction::Call(..) | Instruction::CallStatement(..)
    );
    let Some(var) = var_operand(ins) else {
        return;
    };

    if writes
        && cache
            .as_ref()
            .is_some_and(|held| held.first() == Some(&*var))
    {
        *cache = None;
    }

    let mut bases: Vec<Variable> = vec![];
    let mut cursor = &*var;
    while let Variable::SetCache(base, rest) = cursor {
        bases.push((**base).clone());
        cursor = rest;
    }

    if bases.is_empty() {
        return;
    }

    if bases[0] == Variable::Src && *explicit_src {
        bases[0] = Variable::PtrRef(Box::new(Variable::Src));
        *explicit_src = false;
    }
    if let Some(held) = cache.as_ref() {
        if *held == bases {
            for _ in 0..held.len() {
                let Variable::SetCache(_, rest) = std::mem::replace(var, Variable::Null) else {
                    unreachable!()
                };
                *var = *rest;
            }
        }
    }

    *cache = Some(bases);
}
