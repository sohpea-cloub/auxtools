use std::fmt;

use dreammaker::ast::Follow;
use dreammaker::{ast::Expression, Severity};

use crate::operands::{self, DMString, GlobalVar, Label, Variable};
use crate::syntax;
use crate::Instruction;
use crate::{ByondVersion, Node};

mod assignment;
mod fold;
mod known_bugs;
mod procs;
mod statements;

pub use procs::{compile_proc, ByondBug, CompiledProc, ProcSource, StaticLocal};
pub(crate) use procs::{compile_proc_as, labels_mut, new_tree, proc_path};

mod args;
mod binary_ops;
mod builtin_procs;
mod chain_builder;
mod follow;
mod strings;
mod term;
mod ternary;
mod unary;
mod versions;

pub(crate) fn text(raw: &str) -> Option<String> {
    String::from_utf8(strings::parse(raw).ok()?.0).ok()
}

use chain_builder::ChainBuilder;

fn is_writable(var: &Variable) -> bool {
    match var {
        Variable::Usr
        | Variable::Src
        | Variable::Args
        | Variable::Dot
        | Variable::CacheIndex
        | Variable::Arg { .. }
        | Variable::Local { .. }
        | Variable::Global { .. } => true,
        Variable::PtrDeref(_) => true,
        _ => false,
    }
}

fn dynamic_name(ident: &str) -> DMString {
    DMString(ident.replace('_', " ").into_bytes())
}

#[derive(Debug)]
pub enum CompileError {
    ParseError(dreammaker::DMError),
    StringError(strings::StringError),

    ExpectedLValue,
    ExpectedFieldReference,

    ExpectedEnd,

    UnexpectedRange,
    UnexpectedGlobal,
    UnexpectedArgList,
    UnexpectedProbability,
    UnexpectedNamedArguments,

    UnsupportedPrefabWithVars,
    UnsupportedBuiltin { proc: String },
    UnsupportedImplicitNew,
    UnsupportedRelativeCall,
    UnsupportedImplicitLocate,
    UnsupportedStringInterpolation,
    UnsupportedInput,
    UnsupportedCompilerMacro { name: String },
    UnsupportedStatement(&'static str),
    UndefinedVar(String),
    AtLine(u32, Box<CompileError>),

    AmbiguousListConstructor,
    NotInByond { what: String, byond: ByondVersion },
    InvalidLocateArgs,
    BadProcReference(String),

    IncorrectArgCount(String),
    MissingArgument { proc: String, index: u32 },
    TooManyArguments { proc: String, expected: u32 },
}

impl From<strings::StringError> for CompileError {
    fn from(err: strings::StringError) -> Self {
        Self::StringError(err)
    }
}

impl From<dreammaker::DMError> for CompileError {
    fn from(err: dreammaker::DMError) -> Self {
        Self::ParseError(err)
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompileError::ParseError(err) => {
                write!(f, "parser error: {}", err)
            }

            CompileError::StringError(err) => {
                write!(f, "string error: {}", err)
            }

            CompileError::UndefinedVar(name) => write!(f, "undefined var: {}", name),
            CompileError::ExpectedLValue => write!(f, "expected l-value"),
            CompileError::ExpectedFieldReference => write!(f, "expected field reference"),
            CompileError::ExpectedEnd => {
                write!(f, "expected end (received more code than expected)")
            }
            CompileError::UnexpectedRange => write!(f, "unexpected range"),
            CompileError::UnexpectedGlobal => write!(f, "unexpected global"),
            CompileError::UnexpectedArgList => write!(f, "unexpected arglist"),
            CompileError::UnexpectedProbability => write!(f, "unexpected prob()"),
            CompileError::UnexpectedNamedArguments => write!(f, "unexpected named arguments"),
            CompileError::UnsupportedPrefabWithVars => {
                write!(f, "prefabs with variable overrides are not supported")
            }
            CompileError::UnsupportedBuiltin { proc } => {
                write!(f, "unsupported built-in proc: {}", proc)
            }
            CompileError::UnsupportedImplicitNew => {
                write!(f, "implicit new() calls are not supported")
            }
            CompileError::UnsupportedRelativeCall => write!(f, "relative calls are not supported"),
            CompileError::UnsupportedImplicitLocate => {
                write!(f, "implicit locate() calls are not supported")
            }
            CompileError::UnsupportedStringInterpolation => {
                write!(f, "interpolated strings are not supported")
            }
            CompileError::UnsupportedInput => write!(f, "unsupported built-in proc: input"),
            CompileError::AmbiguousListConstructor => write!(
                f,
                "provided list constructor (or named parameters) are ambiguous"
            ),
            CompileError::InvalidLocateArgs => write!(f, "invalid arguments for locate()"),
            CompileError::IncorrectArgCount(proc) => {
                write!(f, "incorrect amount of arguments for: {}", proc)
            }
            CompileError::MissingArgument { proc, index: _ } => {
                write!(f, "missing argument(s) for: {}", proc)
            }
            CompileError::TooManyArguments { proc, expected } => write!(
                f,
                "too many argument(s) for: {} (expected {})",
                proc, expected
            ),
            CompileError::AtLine(line, err) => write!(f, "line {}: {}", line, err),
            CompileError::UnsupportedStatement(what) => {
                write!(f, "unsupported statement: {}", what)
            }
            CompileError::NotInByond { what, byond } => {
                write!(f, "BYOND {} does not support {}", byond, what)
            }
            CompileError::UnsupportedCompilerMacro { name } => {
                write!(f, "unsupported compiler macro: {}", name)
            }
            CompileError::BadProcReference(why) => write!(f, "bad proc reference: {}", why),
        }
    }
}

pub struct ExprScope<'a> {
    pub has_src_var: &'a dyn Fn(&str) -> bool,
    pub has_src_proc: &'a dyn Fn(&str) -> bool,
    pub global: &'a dyn Fn(&str) -> Option<ExprGlobal>,
    pub globals: &'a str,
    pub proc_ref: &'a dyn Fn(&str, &str) -> Result<FoundProc, String>,
    pub proc_path: &'a dyn Fn(&str) -> Option<FoundProc>,
}

pub struct FoundProc {
    pub tag: u8,
    pub data: u32,
    pub warning: Option<String>,
}

pub enum ExprGlobal {
    Listed,
    Id(u32),
}

pub fn compile_expr(code: &str, params: &[&str]) -> Result<Vec<Node>, CompileError> {
    compile_expr_for(code, params, ByondVersion::LATEST)
}

pub fn compile_expr_for(
    code: &str,
    params: &[&str],
    byond: ByondVersion,
) -> Result<Vec<Node>, CompileError> {
    compile_expr_in(code, params, None, byond)
}

pub fn compile_expr_in(
    code: &str,
    params: &[&str],
    scope: Option<&ExprScope>,
    byond: ByondVersion,
) -> Result<Vec<Node>, CompileError> {
    compile_expr_warned(code, params, scope, byond).map(|(nodes, _)| nodes)
}

pub fn compile_expr_warned(
    code: &str,
    params: &[&str],
    scope: Option<&ExprScope>,
    byond: ByondVersion,
) -> Result<(Vec<Node>, Vec<String>), CompileError> {
    let mut compiler = Compiler::new(params.iter().map(|x| x.to_string()).collect(), None);
    compiler.byond = byond;
    compiler.expr_scope = scope;
    compiler.emit_ins(Instruction::DbgFile(DMString(
        b"<dmasm expression>".to_vec(),
    )));

    let ctx = dreammaker::Context::default();
    byond.read_as(&ctx);

    let mut lexer =
        dreammaker::lexer::Lexer::new(&ctx, dreammaker::FileId::BUILTINS, code.as_bytes());
    let tokens: Vec<dreammaker::lexer::LocatedToken> =
        dreammaker::IndentProcessor::new(&ctx, &mut lexer).collect();
    let taken = std::cell::Cell::new(0usize);
    let expr = ctx.parse_expression(
        dreammaker::Location::BUILTINS,
        tokens
            .iter()
            .cloned()
            .inspect(|_| taken.set(taken.get() + 1)),
    )?;

    let rest = &tokens[taken.get().saturating_sub(1).min(tokens.len())..];
    let ends = |token: &dreammaker::lexer::LocatedToken| {
        use dreammaker::lexer::{Punctuation, Token};
        matches!(
            token.token,
            Token::Eof | Token::Punct(Punctuation::Newline | Punctuation::Semicolon)
        )
    };
    if taken.get() > 0 && !rest.iter().all(ends) {
        return Err(CompileError::ExpectedEnd);
    }

    for err in ctx.errors().iter() {
        if err.severity() == Severity::Error {
            return Err(err.clone().into());
        }
    }

    let kind = compiler.emit_expr(expr)?;
    compiler.emit_move_to_stack(kind)?;

    let param_amt = params.len() as u32;
    for arg_id in 0..param_amt {
        compiler.emit_ins(Instruction::GetVar(Variable::Arg(arg_id)));
    }

    compiler.emit_ins(Instruction::NewList(param_amt + 1));
    compiler.emit_ins(Instruction::Ret);
    let warnings = compiler.warnings.into_inner();
    Ok((compiler.nodes, warnings))
}

#[derive(Debug)]
pub struct CompiledBody {
    pub nodes: Vec<Node>,
    pub locals: Vec<String>,
    pub warnings: Vec<String>,
}

const BODY_PROC: &str = "dmasm_body";
const BODY_FILE: &str = "<dmasm expression>";

pub fn is_body(code: &str) -> bool {
    use dreammaker::ast::Statement;
    let Ok(tree) = parse_body(code, ByondVersion::LATEST) else {
        return false;
    };
    match body_statements(&tree) {
        [] => false,
        [only] => match &only.elem {
            Statement::Expr(expr) => is_statement_builtin(expr, ByondVersion::LATEST),
            _ => true,
        },
        _ => true,
    }
}

fn is_statement_builtin(expr: &Expression, byond: ByondVersion) -> bool {
    use dreammaker::ast::Term;
    let Expression::Base { term, follow } = expr else {
        return false;
    };
    let Term::Call(name, args) = &term.elem else {
        return false;
    };
    if !follow.is_empty() {
        return false;
    }
    let mut compiler = Compiler::new(vec![], None);
    compiler.byond = byond;
    if !matches!(
        builtin_procs::emit(&mut compiler, name.as_str(), &args.to_vec()),
        Ok(None)
    ) {
        return false;
    }
    !matches!(
        builtin_procs::emit_statement_builtin(&mut compiler, name.as_str(), args),
        Ok(false)
    )
}

pub fn compile_body_in(
    code: &str,
    params: &[&str],
    scope: Option<&ExprScope>,
    byond: ByondVersion,
) -> Result<CompiledBody, CompileError> {
    use dreammaker::ast::{BinaryOp, Statement};

    let parsed = parse_body(code, byond)?;
    let mut statements = body_statements(&parsed).to_vec();
    let ends_with_label = matches!(
        statements.last().map(|last| &last.elem),
        Some(Statement::Expr(expr)) if statements::is_bare_label(&statements, expr)
    );
    if let Some(last) = statements.last_mut().filter(|_| !ends_with_label) {
        if let Statement::Expr(expr) = &last.elem {
            let io = matches!(
                expr,
                Expression::BinaryOp {
                    op: BinaryOp::LShift | BinaryOp::RShift,
                    ..
                }
            );
            let assigns = match expr {
                Expression::AssignOp { .. } => true,
                Expression::Base { follow, .. } => matches!(
                    follow.last().map(|f| &f.elem),
                    Some(dreammaker::ast::Follow::Unary(
                        dreammaker::ast::UnaryOp::PreIncr
                            | dreammaker::ast::UnaryOp::PostIncr
                            | dreammaker::ast::UnaryOp::PreDecr
                            | dreammaker::ast::UnaryOp::PostDecr
                    ))
                ),
                _ => false,
            };
            if !io && !assigns && !is_statement_builtin(expr, byond) {
                last.elem = Statement::Return(Some(expr.clone()));
            }
        }
    }
    let value = dreammaker::objtree::ProcValue {
        code: Some(statements.into_boxed_slice()),
        ..parsed
            .root()
            .get()
            .procs
            .get(BODY_PROC)
            .expect("the body's proc")
            .value[0]
            .clone()
    };
    let tree = builtins_tree();
    let root = tree.root();
    let source = procs::ProcSource {
        tree: &tree,
        ty: root,
        name: BODY_PROC,
        value: &value,
        file: BODY_FILE,
        debug_info: true,
        index: Some(0),
        byond,
        file_names: None,
    };

    let ctx = procs::proc_ctx(&source, vec![String::new(); params.len()]);
    let mut compiler = Compiler::new(params.iter().map(|x| x.to_string()).collect(), Some(ctx));
    compiler.byond = byond;
    compiler.expr_scope = scope;
    compiler.emit_ins(Instruction::DbgFile(DMString(
        BODY_FILE.as_bytes().to_vec(),
    )));
    statements::emit_proc_body(&mut compiler, &source, value.code.as_ref().unwrap()).map_err(
        |e| match e {
            CompileError::AtLine(line, e) => CompileError::AtLine(line.saturating_sub(1), e),
            e => e,
        },
    )?;
    compiler.emit_ins(Instruction::End);
    let locals = compiler.proc.take().unwrap().locals;
    let warnings = compiler
        .warnings
        .take()
        .into_iter()
        .map(|warning| {
            let line = warning
                .strip_prefix("line ")
                .and_then(|rest| rest.split_once(": "))
                .and_then(|(line, text)| Some((line.parse::<u32>().ok()?, text)));
            match line {
                Some((line, text)) => format!("line {}: {text}", line.saturating_sub(1)),
                None => warning,
            }
        })
        .collect();

    let count = params.len() as u32;
    let dot = match params.iter().rposition(|p| *p == ".") {
        Some(index) => Variable::Arg(index as u32),
        None => Variable::Dot,
    };
    let mut nodes = vec![];
    for node in compiler.nodes {
        let ins = match node {
            Node::Instruction(ins, ()) => ins,
            other => {
                nodes.push(other);
                continue;
            }
        };
        match ins {
            Instruction::DbgLine(line) => nodes.push(Node::Instruction(
                Instruction::DbgLine(line.saturating_sub(1)),
                (),
            )),
            Instruction::Ret | Instruction::End => {
                if ins == Instruction::End {
                    nodes.push(Node::Instruction(Instruction::GetVar(dot.clone()), ()));
                }
                for arg in 0..count {
                    nodes.push(Node::Instruction(
                        Instruction::GetVar(Variable::Arg(arg)),
                        (),
                    ));
                }
                nodes.push(Node::Instruction(Instruction::NewList(count + 1), ()));
                nodes.push(Node::Instruction(Instruction::Ret, ()));
            }
            ins => nodes.push(Node::Instruction(ins, ())),
        }
    }
    Ok(CompiledBody {
        nodes,
        locals,
        warnings,
    })
}

fn parse_body(
    code: &str,
    byond: ByondVersion,
) -> Result<dreammaker::objtree::ObjectTree, CompileError> {
    let lines: Vec<&str> = code.lines().collect();
    let indent = |line: &str| line.len() - line.trim_start_matches([' ', '\t']).len();
    let common = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| &line[..indent(line)])
        .reduce(|a, b| {
            let same = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
            &a[..same]
        })
        .unwrap_or("");
    let lines: Vec<&str> = lines
        .iter()
        .map(|line| line.strip_prefix(common).unwrap_or(line.trim_start()))
        .collect();
    let unit = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| &line[..indent(line)])
        .find(|prefix| !prefix.is_empty())
        .unwrap_or("\t");
    let mut text = format!("/proc/{}()\n", BODY_PROC);
    for line in &lines {
        text.push_str(unit);
        text.push_str(line);
        text.push('\n');
    }

    let ctx = dreammaker::Context::default();
    byond.read_as(&ctx);
    let pp = dreammaker::Preprocessor::from_buffer(&ctx, BODY_FILE.into(), text);
    let mut parser = dreammaker::Parser::new(&ctx, pp);
    parser.enable_procs();
    let tree = parser.parse_object_tree_without_builtins();
    if let Some(err) = ctx
        .errors()
        .iter()
        .find(|e| e.severity() == Severity::Error)
    {
        let mut at = err.location();
        at.line = at
            .line
            .saturating_sub(1)
            .clamp(1, lines.len().max(1) as u32);
        at.column = (at.column.saturating_sub(unit.len() as u16) + common.len() as u16).max(1);
        return Err(CompileError::ParseError(dreammaker::DMError::new(
            at,
            err.description(),
        )));
    }
    if tree
        .root()
        .get()
        .procs
        .get(BODY_PROC)
        .is_none_or(|p| p.value.is_empty())
    {
        return Err(CompileError::UnsupportedStatement("code outside the body"));
    }
    Ok(tree)
}

fn builtins_tree() -> dreammaker::objtree::ObjectTree {
    let ctx = dreammaker::Context::default();
    let pp = dreammaker::Preprocessor::from_buffer(&ctx, BODY_FILE.into(), "");
    dreammaker::Parser::new(&ctx, pp).parse_object_tree()
}

fn body_statements(
    tree: &dreammaker::objtree::ObjectTree,
) -> &[dreammaker::ast::Spanned<dreammaker::ast::Statement>] {
    tree.root()
        .get()
        .procs
        .get(BODY_PROC)
        .and_then(|p| p.value[0].code.as_deref())
        .unwrap_or(&[])
}

fn src_member(member: Follow) -> Expression {
    let location = dreammaker::Location::BUILTINS;
    Expression::Base {
        term: Box::new(dreammaker::ast::Spanned::new(
            location,
            dreammaker::ast::Term::Ident("src".into()),
        )),
        follow: vec![dreammaker::ast::Spanned::new(location, member)].into(),
    }
}

#[derive(Debug, PartialEq)]
enum EvalKind {
    Stack,

    ListRef,

    Range,

    Global,

    ArgList,

    Var(Variable),

    Field(ChainBuilder, String),
}

impl fmt::Display for EvalKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvalKind::Stack => write!(f, "stack value"),
            EvalKind::ListRef => write!(f, "list access"),
            EvalKind::Range => write!(f, "range"),
            EvalKind::Global => write!(f, "global"),
            EvalKind::ArgList => write!(f, "arglist"),
            EvalKind::Var(_) => write!(f, "variable"),
            EvalKind::Field(_, _) => write!(f, "field access"),
        }
    }
}

#[derive(Clone)]
struct Compiler<'a> {
    params: Vec<String>,
    nodes: Vec<Node>,
    label_count: u32,
    short_circuit_labels: Vec<(String, bool)>,
    proc: Option<procs::ProcCtx<'a>>,
    expr_scope: Option<&'a ExprScope<'a>>,
    cache: Option<Vec<Variable>>,
    last_line: Option<u32>,
    implicit_type: Option<String>,
    explicit_src: bool,
    debug_info: bool,
    byond: ByondVersion,
    label_states: std::collections::HashMap<String, (u32, Option<Vec<Variable>>)>,
    condition_chain: bool,
    mul_lhs: bool,
    regroup_root: bool,
    regroup_chain: bool,
    global_field: bool,
    other_file: bool,
    line_pending: bool,
    pub(super) read_on_pending: bool,
    pub(super) read_to: Option<u32>,
    pub(super) skip_compound: bool,
    pub(super) place: bool,
    proc_macro_ok: bool,
    no_fold: bool,
    live_only: bool,
    warnings: std::cell::RefCell<Vec<String>>,
    byond_bugs: std::cell::RefCell<Vec<ByondBug>>,
    list_elem_type: Option<dreammaker::objtree::TypeRef<'a>>,
}

impl<'a> Compiler<'a> {
    fn new(params: Vec<String>, proc: Option<procs::ProcCtx<'a>>) -> Self {
        Compiler {
            params,
            nodes: vec![],
            label_count: 0,
            short_circuit_labels: vec![],
            proc,
            expr_scope: None,
            cache: None,
            last_line: None,
            implicit_type: None,

            explicit_src: false,
            debug_info: true,
            byond: ByondVersion::LATEST,
            label_states: Default::default(),
            condition_chain: false,
            mul_lhs: false,
            regroup_root: false,
            regroup_chain: false,
            global_field: false,
            other_file: false,
            line_pending: false,
            read_on_pending: false,
            read_to: None,
            skip_compound: false,
            place: false,
            warnings: Default::default(),
            byond_bugs: Default::default(),
            proc_macro_ok: false,
            no_fold: false,
            live_only: false,
            list_elem_type: None,
        }
    }

    fn emit_ins(&mut self, mut ins: Instruction) {
        if !self.debug_info && matches!(ins, Instruction::DbgLine(_) | Instruction::DbgFile(_)) {
            return;
        }
        if let (Instruction::DbgLine(_), true) = (&ins, self.other_file) {
            self.other_file = false;
            self.last_line = None;
            let file = self
                .proc
                .as_ref()
                .map(|p| p.file.clone())
                .unwrap_or_default();
            self.emit_ins(Instruction::DbgFile(DMString(file.into_bytes())));
        }
        if let Instruction::DbgLine(line) = ins {
            self.line_pending = false;
            self.read_on_pending = false;
            if self.last_line == Some(line) {
                return;
            }
            self.last_line = Some(line);
        }
        procs::track_cache(&mut self.cache, &mut ins, &mut self.explicit_src);
        if matches!(
            ins,
            Instruction::Switch(_)
                | Instruction::SwitchRange(_)
                | Instruction::PickSwitch(_)
                | Instruction::PickProb(_)
        ) {
            if let Some(ctx) = self.proc.as_mut() {
                ctx.clear_new_vars = true;
            }
        }
        let mut targets = procs::label_targets(&ins);
        targets.sort();
        targets.dedup();
        for target in targets {
            let state = if matches!(ins, Instruction::Spawn(_)) {
                None
            } else {
                self.cache.clone()
            };
            self.label_states
                .entry(target)
                .and_modify(|(n, s)| {
                    *n += 1;
                    if *s != state {
                        *s = None
                    }
                })
                .or_insert((1, state));
        }
        self.nodes.push(Node::Instruction(ins, ()));
    }

    fn emit_label(&mut self, label: String) {
        let falls_through = !matches!(
            self.nodes.last(),
            Some(Node::Instruction(
                Instruction::Jmp(_)
                    | Instruction::JmpLoop(_)
                    | Instruction::Switch(_)
                    | Instruction::SwitchRange(_)
                    | Instruction::PickSwitch(_)
                    | Instruction::PickProb(_),
                _
            ))
        );
        let jumped = self.label_states.remove(&label);
        self.cache = match (falls_through, jumped) {
            (false, Some((1, state))) => state,
            _ => None,
        };
        self.nodes.push(Node::Label(Label::Named(label)));
    }

    fn emit_loop_label(&mut self, label: String) {
        self.cache = None;
        self.nodes.push(Node::Label(Label::Named(label)));
    }

    fn emit_call_args(&mut self, args: Vec<Expression>) -> Result<u32, CompileError> {
        let count = args.len() as u32;
        Ok(match args::emit(self, args::ArgsContext::Proc, args)? {
            args::ArgsResult::Normal => count,
            args::ArgsResult::Assoc => {
                self.emit_ins(Instruction::NewAssocList(count));
                65535
            }
            args::ArgsResult::ArgList => 65535,
        })
    }

    fn proc_macros_known(&self) -> bool {
        self.proc.is_some() && self.expr_scope.is_none()
    }

    fn static_type_name(&self, expr: &Expression) -> Option<String> {
        procs::expr_type(self, expr).map(|t| t.get().path.clone())
    }

    pub(crate) fn end_assignment_statement(&mut self) {
        if self.cache.as_ref().is_some_and(|h| h.len() > 1) {
            self.cache = None;
        }
    }

    fn new_label(&mut self) -> String {
        let label = format!("LAB_{:0>4X}", self.label_count);
        self.label_count += 1;
        label
    }

    fn emit_find_var(&mut self, ident: dreammaker::ast::Ident) -> Result<EvalKind, CompileError> {
        if let Some(kind) = procs::find_var(self, ident.as_str()) {
            return Ok(kind);
        }

        if let Some(index) = self.params.iter().rposition(|x| *x == ident.as_str()) {
            return Ok(EvalKind::Var(Variable::Arg(index as u32)));
        }

        Ok(match ident.as_str() {
            "." => EvalKind::Var(Variable::Dot),
            "usr" => EvalKind::Var(Variable::Usr),
            "src" => EvalKind::Var(Variable::Src),
            "args" => EvalKind::Var(Variable::Args),
            "world" => EvalKind::Var(Variable::World),
            "caller" => EvalKind::Var(Variable::Caller),
            "callee" => EvalKind::Var(Variable::Callee),
            "global" => EvalKind::Global,

            name => match self.expr_scope {
                Some(scope) => {
                    if (scope.has_src_var)(name) {
                        return self.emit_expr(src_member(Follow::Field(
                            dreammaker::ast::PropertyAccessKind::Dot,
                            ident.clone(),
                        )));
                    }
                    return self.emit_global_by_name(name);
                }

                None => EvalKind::Var(Variable::Global(GlobalVar::named(name.as_bytes(), None))),
            },
        })
    }

    fn emit_global_by_name(&mut self, name: &str) -> Result<EvalKind, CompileError> {
        let scope = self
            .expr_scope
            .expect("a global by name outside a debugger's expression");
        match (scope.global)(name) {
            None => return Err(CompileError::UndefinedVar(name.to_owned())),
            Some(ExprGlobal::Id(id)) => {
                let mut global = GlobalVar::named(name.as_bytes(), Some(String::new()));
                global.id = Some(id);
                return Ok(EvalKind::Var(Variable::Global(global)));
            }
            Some(ExprGlobal::Listed) => {}
        }
        let Some(index) = self.params.iter().rposition(|x| *x == scope.globals) else {
            return Err(CompileError::UndefinedVar(scope.globals.to_owned()));
        };
        self.emit_ins(Instruction::GetVar(Variable::Arg(index as u32)));
        self.emit_ins(Instruction::PushVal(operands::Value::DMString(DMString(
            name.as_bytes().to_vec(),
        ))));
        Ok(EvalKind::ListRef)
    }

    fn emit_move_to_stack(&mut self, kind: EvalKind) -> Result<EvalKind, CompileError> {
        match kind {
            EvalKind::Stack => {}

            EvalKind::ListRef => {
                self.emit_ins(Instruction::ListGet);
            }

            EvalKind::Range => return Err(CompileError::UnexpectedRange),
            EvalKind::Global => return Err(CompileError::UnexpectedGlobal),
            EvalKind::ArgList => return Err(CompileError::UnexpectedArgList),

            EvalKind::Var(var) => {
                self.emit_ins(Instruction::GetVar(var));
            }

            EvalKind::Field(builder, field) => {
                let var = builder.get_field(DMString(field.into()));
                self.emit_ins(Instruction::GetVar(var));
            }
        }

        Ok(EvalKind::Stack)
    }

    fn short_circuit(&mut self) -> String {
        let label = self.short_circuit_labels.last_mut().unwrap();
        label.1 = true;
        label.0.to_owned()
    }

    fn emit_constant(
        &mut self,
        value: dreammaker::constants::Constant,
    ) -> Result<(), CompileError> {
        use dreammaker::constants::Constant;
        match value {
            Constant::Float(n) => self.emit_number(n),
            Constant::Null(_) => self.emit_ins(Instruction::PushVal(operands::Value::Null)),
            Constant::String(s) => self.emit_ins(Instruction::PushVal(operands::Value::DMString(
                strings::parse(s.as_str())?,
            ))),
            Constant::Resource(r) => self.emit_ins(Instruction::PushVal(
                operands::Value::Resource(r.as_str().to_owned()),
            )),
            Constant::Prefab(pop) if pop.vars.is_empty() => self.emit_ins(Instruction::PushVal(
                operands::Value::Path(pop.path.to_string()),
            )),
            _ => {
                return Err(CompileError::UnsupportedStatement(
                    "a const var with a complex value",
                ))
            }
        }
        Ok(())
    }

    fn emit_number(&mut self, value: f32) {
        if value.fract() == 0.0 && (0.0..=65535.0).contains(&value) {
            self.emit_ins(Instruction::PushInt(value as i32));
        } else {
            self.emit_ins(Instruction::PushVal(operands::Value::Number(value)));
        }
    }

    fn emit_folded(&mut self, folded: Option<fold::Constant>) -> bool {
        match folded {
            Some(fold::Constant::Number(value)) => self.emit_number(value),
            Some(fold::Constant::Text(text)) => self.emit_ins(Instruction::PushVal(
                operands::Value::DMString(DMString(text)),
            )),
            Some(fold::Constant::Null) => {
                self.emit_ins(Instruction::PushVal(operands::Value::Null))
            }
            Some(fold::Constant::Path(path)) => {
                self.emit_ins(Instruction::PushVal(operands::Value::Path(path)))
            }
            Some(fold::Constant::New) | None => return false,
        }
        true
    }

    fn emit_inner_expr(&mut self, expr: Expression) -> Result<EvalKind, CompileError> {
        let root = std::mem::take(&mut self.regroup_root);
        let place = std::mem::take(&mut self.place);
        if self.proc.is_some() {
            if let Some(line) = self.read_on_line(&expr).filter(|_| !place) {
                if self.read_to.is_some_and(|r| line < r) && self.last_line == self.read_to {
                    self.emit_ins(Instruction::DbgLine(line));
                    self.read_on_pending = true;
                } else {
                    self.read_on(line);
                }
            }
            let ends = self.proc.as_ref().map_or(&[][..], |p| p.end_lines);
            let skip = std::mem::take(&mut self.skip_compound);
            if let Some(line) = compound_line(&expr, ends).filter(|_| !place && !skip) {
                let term = matches!(expr, Expression::Base { .. });
                if self.line_pending
                    || (term && self.read_on_pending)
                    || self.last_line.is_some_and(|l| line > l)
                {
                    self.emit_ins(Instruction::DbgLine(line));
                }
            }
            if let Some(line) = self.named_call_end(&expr).filter(|_| !place) {
                self.read_on(line);
            }
            let folded = fold::constant(self, &expr);
            if folded.as_ref().is_some_and(|c| *c != fold::Constant::New) && fold::has_logic(&expr)
            {
                self.cache = None;
            }
            if folded.as_ref().is_some_and(|c| *c != fold::Constant::New)
                && self.debug_info
                && !self.other_file
                && syntax::has_static_field(&expr)
                && self.names_other_file(&expr)
            {
                self.other_file = true;
            }
            if self.emit_folded(folded) {
                return Ok(EvalKind::Stack);
            }
        } else if !place {
            let folded = fold::constant(self, &expr);
            if self.emit_folded(folded) {
                return Ok(EvalKind::Stack);
            }
        }
        match expr {
            Expression::TernaryOp { cond, if_, else_ } if fold::value(self, &cond).is_some() => {
                let chosen = if fold::value(self, &cond).unwrap().truthy() {
                    if_
                } else {
                    else_
                };
                if self.debug_info
                    && !self.other_file
                    && syntax::has_static_field(&cond)
                    && self.names_other_file(&cond)
                {
                    if let Some(line) = self.last_line {
                        self.other_file = true;
                        self.emit_ins(Instruction::DbgLine(line));
                    }
                }
                let kind = self.emit_expr(*chosen)?;
                self.emit_move_to_stack(kind)
            }
            Expression::TernaryOp { cond, if_, else_ } => ternary::emit(self, *cond, *if_, *else_),
            Expression::BinaryOp { op, lhs, rhs } => {
                self.regroup_chain = root;
                binary_ops::emit(self, op, *lhs, *rhs)
            }
            Expression::AssignOp { op, lhs, rhs } => assignment::emit(self, op, *lhs, *rhs),

            Expression::Base { term, follow }
                if follow.is_empty()
                    && matches!(&term.elem, dreammaker::ast::Term::Locate { in_list: Some(list), .. }
                        if matches!(**list, Expression::BinaryOp { op: dreammaker::ast::BinaryOp::In, .. })) =>
            {
                let dreammaker::ast::Term::Locate {
                    args,
                    in_list: Some(list),
                } = term.elem
                else {
                    unreachable!()
                };
                let Expression::BinaryOp { lhs, rhs, .. } = *list else {
                    unreachable!()
                };
                let locate = Expression::Base {
                    term: Box::new(dreammaker::ast::Spanned::new(
                        term.location,
                        dreammaker::ast::Term::Locate {
                            args,
                            in_list: Some(lhs),
                        },
                    )),
                    follow,
                };
                self.emit_expr(Expression::BinaryOp {
                    op: dreammaker::ast::BinaryOp::In,
                    lhs: Box::new(locate),
                    rhs,
                })
            }
            Expression::Base { term, follow }
                if self.proc.is_some()
                    && match follow.first().map(|f| &f.elem) {
                        Some(Follow::Unary(dreammaker::ast::UnaryOp::Reference)) => true,
                        Some(Follow::Unary(dreammaker::ast::UnaryOp::Dereference)) => matches!(
                            &term.elem,
                            dreammaker::ast::Term::Expr(inner) if matches!(&**inner,
                                Expression::Base { follow, .. }
                                    if !follow.iter().any(|f| matches!(f.elem, Follow::Unary(_))))
                        ),
                        _ => false,
                    }
                    && matches!(&term.elem, dreammaker::ast::Term::Expr(inner)
                        if matches!(**inner, Expression::Base { .. })) =>
            {
                let dreammaker::ast::Term::Expr(inner) = term.elem else {
                    unreachable!()
                };
                let Expression::Base {
                    term: inner_term,
                    follow: inner_follow,
                } = *inner
                else {
                    unreachable!()
                };
                let follow = inner_follow.iter().chain(follow.iter()).cloned().collect();
                self.emit_expr(Expression::Base {
                    term: inner_term,
                    follow,
                })
            }
            Expression::Base { term, follow } => {
                let unspanned_follows: Vec<Follow> =
                    follow.iter().map(|f| f.elem.clone()).collect();
                if self.proc.is_some()
                    && !follow.is_empty()
                    && matches!(&term.elem, dreammaker::ast::Term::Ident(i) if i.as_str() == "src")
                {
                    self.explicit_src = true;
                }
                let mut type_term = &term.elem;
                while let dreammaker::ast::Term::Expr(inner) = type_term {
                    match &**inner {
                        Expression::Base { term, follow } if follow.is_empty() => {
                            type_term = &term.elem
                        }
                        _ => break,
                    }
                }
                if let (
                    dreammaker::ast::Term::Prefab(prefab),
                    Some(Follow::StaticField(_) | Follow::ProcReference(_)),
                ) = (type_term, unspanned_follows.first())
                {
                    if self.proc.is_some() && prefab.vars.is_empty() && self.expr_scope.is_none() {
                        self.global_field = true;
                        let path = prefab.path.to_string();
                        let kind = procs::emit_type_scope(self, &path, &unspanned_follows[0])?;
                        let ty = self.proc.as_ref().unwrap().tree.find(&path);
                        return follow::emit_typed(self, unspanned_follows[1..].to_vec(), kind, ty);
                    }
                    if let (Some(scope), Follow::ProcReference(name)) =
                        (self.expr_scope, &unspanned_follows[0])
                    {
                        if prefab.vars.is_empty() {
                            let path = prefab.path.to_string();
                            let found = (scope.proc_ref)(&path, name.as_str())
                                .map_err(CompileError::BadProcReference)?;
                            self.emit_found_proc(found);
                            return follow::emit(
                                self,
                                unspanned_follows[1..].to_vec(),
                                EvalKind::Stack,
                            );
                        }
                    }
                }
                let ty = procs::term_type(self, &term.elem);
                let elem_ty = match &term.elem {
                    dreammaker::ast::Term::Ident(_) | dreammaker::ast::Term::GlobalIdent(_) => {
                        procs::declared_type_path(
                            self,
                            &dreammaker::ast::Expression::Base {
                                term: term.clone(),
                                follow: Default::default(),
                            },
                        )
                        .and_then(|path| procs::list_elem_type(self, &path))
                    }
                    _ => None,
                };
                if matches!(term.elem, dreammaker::ast::Term::Null)
                    && term.location.file == dreammaker::FileId::UNKNOWN
                    && unspanned_follows.is_empty()
                    && self.proc.is_some()
                {
                    self.emit_ins(Instruction::PushVal(operands::Value::Null));
                    return Ok(EvalKind::Stack);
                }
                if let dreammaker::ast::Term::NewPrefab {
                    prefab,
                    args: Some(args),
                } = &term.elem
                {
                    if term.location.file == dreammaker::FileId::UNKNOWN
                        && args.len() == 1
                        && prefab.path.to_string() == "/list"
                        && self.proc.is_some()
                    {
                        let kind = self.emit_expr(args[0].clone())?;
                        self.emit_move_to_stack(kind)?;
                        self.emit_ins(Instruction::EmptyList);
                        return follow::emit_typed(self, unspanned_follows, EvalKind::Stack, ty);
                    }
                }
                let is_ident = matches!(term.elem, dreammaker::ast::Term::Ident(_));
                let mut kind = term::emit(self, term.elem)?;
                if is_ident
                    && self.proc.as_ref().is_some_and(|c| !c.ty.is_root())
                    && matches!(
                        unspanned_follows.first(),
                        Some(
                            Follow::Field(
                                dreammaker::ast::PropertyAccessKind::Dot
                                    | dreammaker::ast::PropertyAccessKind::Colon,
                                _
                            ) | Follow::Call(
                                dreammaker::ast::PropertyAccessKind::Dot
                                    | dreammaker::ast::PropertyAccessKind::Colon,
                                _,
                                _
                            )
                        )
                    )
                {
                    if let EvalKind::Var(Variable::Global(g)) = &kind {
                        if let Some(scope) = g
                            .scope
                            .as_deref()
                            .filter(|s| !s.is_empty() && !s.starts_with("proc:"))
                        {
                            let mut marker = chain_builder::STATIC_MARKER.to_vec();
                            marker.extend_from_slice(&g.name.0);
                            marker.push(0);
                            marker.extend_from_slice(scope.as_bytes());
                            kind = EvalKind::Field(
                                chain_builder::ChainBuilder::begin(Variable::Src),
                                String::from_utf8(marker).unwrap(),
                            );
                        }
                    }
                }
                self.list_elem_type = elem_ty;
                let kind = follow::emit_typed(self, unspanned_follows, kind, ty)?;
                Ok(kind)
            }
        }
    }

    fn names_other_file(&mut self, expr: &Expression) -> bool {
        let start = self.nodes.len();
        let saved = (
            self.label_count,
            self.cache.clone(),
            self.last_line,
            self.label_states.clone(),
            self.global_field,
            self.line_pending,
            self.short_circuit_labels.len(),
        );
        self.no_fold = true;
        self.live_only = true;
        let ok = self.emit_expr(expr.clone()).is_ok();
        self.no_fold = false;
        self.live_only = false;
        let found = ok && self.other_file;
        self.nodes.truncate(start);
        self.short_circuit_labels.truncate(saved.6);
        (
            self.label_count,
            self.cache,
            self.last_line,
            self.label_states,
            self.global_field,
            self.line_pending,
        ) = (saved.0, saved.1, saved.2, saved.3, saved.4, saved.5);
        self.other_file = false;
        found
    }

    fn emit_found_proc(&mut self, found: FoundProc) {
        if let Some(warning) = &found.warning {
            self.warn(warning);
        }
        self.emit_ins(Instruction::PushVal(operands::Value::Raw {
            tag: found.tag,
            data: found.data,
        }));
    }

    fn warn(&self, text: &str) {
        self.record(self.last_line, text, None);
    }

    fn warn_bug(&self, bug: &str, ours: &str) {
        self.record(self.last_line, &format!("{bug}{ours}"), Some(bug));
    }

    fn warn_at(&self, line: u32, text: &str) {
        self.record(Some(line), text, Some(text));
    }

    fn record(&self, line: Option<u32>, text: &str, byond_bug: Option<&str>) {
        if let Some(bug) = byond_bug {
            let bug = ByondBug {
                line,
                text: bug.to_owned(),
            };
            let mut bugs = self.byond_bugs.borrow_mut();
            if !bugs.contains(&bug) {
                bugs.push(bug);
            }
        }
        let text = match line {
            Some(line) => format!("line {line}: {text}"),
            None => text.to_owned(),
        };
        let mut warnings = self.warnings.borrow_mut();
        if !warnings.contains(&text) {
            warnings.push(text);
        }
    }

    fn emit_place(
        &mut self,
        op: dreammaker::ast::AssignOp,
        expr: Expression,
    ) -> Result<EvalKind, CompileError> {
        self.place = op != dreammaker::ast::AssignOp::Assign;
        self.emit_expr(expr)
    }

    fn emit_expr(&mut self, expr: Expression) -> Result<EvalKind, CompileError> {
        let label = format!("LAB_{:0>4X}", self.label_count);
        self.label_count += 1;
        self.short_circuit_labels.push((label, false));

        let kind = self.emit_inner_expr(expr)?;

        let (label, used) = self.short_circuit_labels.pop().unwrap();

        if used {
            self.emit_move_to_stack(kind)?;
            self.emit_label(label);
            return Ok(EvalKind::Stack);
        }

        Ok(kind)
    }
}

#[test]
fn compiles_safe_field_increment() {
    let nodes = compile_expr("a?.b++", &["a"]).unwrap();
    let bytecode = crate::assembler::assemble(&nodes, &mut crate::TestAssembleEnv).unwrap();

    assert_eq!(
        bytecode,
        vec![0x84, 1337, 0x33, 0xFFD9, 0, 0x13D, 9, 0x63, 1337, 0x33, 0xFFD9, 0, 0x1A, 2, 0x12,]
    );
}

pub(super) fn line_at(
    lines: &[(dreammaker::Location, u32)],
    at: dreammaker::Location,
) -> Option<u32> {
    lines
        .binary_search_by_key(&at, |(l, _)| *l)
        .ok()
        .map(|i| lines[i].1)
}

fn first_token(expr: &Expression) -> dreammaker::Location {
    match expr {
        Expression::Base { term, follow } => follow
            .iter()
            .map(|f| f.location)
            .chain(std::iter::once(term.location))
            .min()
            .unwrap(),
        Expression::BinaryOp { lhs, .. } | Expression::AssignOp { lhs, .. } => first_token(lhs),
        Expression::TernaryOp { cond, .. } => first_token(cond),
    }
}

impl Compiler<'_> {
    pub(super) fn read_on(&mut self, line: u32) {
        if self.last_line.is_some_and(|l| line > l) && self.read_to.is_none_or(|r| line > r) {
            self.emit_ins(Instruction::DbgLine(line));
            self.read_on_pending = true;
            self.read_to = Some(line);
        }
    }

    pub(super) fn named_call_end(&self, expr: &Expression) -> Option<u32> {
        use dreammaker::ast::Term;
        let ctx = self.proc.as_ref()?;
        let named = |args: &[Expression]| {
            args.iter().any(|a| {
                matches!(
                    a,
                    Expression::AssignOp {
                        op: dreammaker::ast::AssignOp::Assign,
                        ..
                    }
                )
            })
        };
        let Expression::Base { term, follow } = expr else {
            return None;
        };
        let call = match &term.elem {
            Term::Call(_, args) | Term::ParentCall(args) | Term::SelfCall(args) => named(args),
            Term::NewImplicit { args: Some(args) }
            | Term::NewPrefab {
                args: Some(args), ..
            }
            | Term::NewMiniExpr {
                args: Some(args), ..
            } => named(args),
            _ => false,
        };
        let mut line = call
            .then(|| line_at(ctx.end_lines, term.location))
            .flatten();
        for f in follow.iter() {
            if let Follow::Call(_, _, args) = &f.elem {
                if named(args) {
                    line = line.max(line_at(ctx.end_lines, f.location));
                }
            }
        }
        line
    }

    pub(super) fn read_on_line(&self, expr: &Expression) -> Option<u32> {
        let ctx = self.proc.as_ref()?;
        let operator = |after: &Expression| {
            let at = first_token(after);
            line_at(ctx.operator_lines, at).unwrap_or(at.line)
        };
        match expr {
            Expression::BinaryOp { rhs, .. } => Some(operator(rhs)),
            Expression::TernaryOp { if_, .. } => Some(operator(if_)),
            Expression::AssignOp { .. } => None,
            Expression::Base { term, follow } => follow
                .iter()
                .filter(|f| f.location > term.location)
                .map(|f| f.location.line)
                .find(|l| *l > term.location.line),
        }
    }
}

fn compound_line(expr: &Expression, ends: &[(dreammaker::Location, u32)]) -> Option<u32> {
    use dreammaker::ast::{PropertyAccessKind, Term, UnaryOp};
    match expr {
        Expression::Base { term, follow } => {
            let number = matches!(term.elem, Term::Int(_) | Term::Float(_));
            let leaf_term = match &term.elem {
                Term::Null
                | Term::Int(_)
                | Term::Float(_)
                | Term::Ident(_)
                | Term::String(_)
                | Term::Resource(_)
                | Term::Prefab(_)
                | Term::GlobalIdent(_) => true,
                Term::Expr(inner) => compound_line(inner, ends).is_none(),
                _ => false,
            };
            let leaf_follow = follow.iter().all(|f| match &f.elem {
                Follow::Field(PropertyAccessKind::Dot | PropertyAccessKind::Colon, _) => true,
                Follow::Unary(UnaryOp::Neg) => number && follow.len() == 1,
                _ => false,
            });
            let line = match term.elem {
                Term::InterpString(..) => {
                    line_at(ends, term.location).unwrap_or(term.location.line)
                }
                _ => term.location.line,
            };
            (!(leaf_term && leaf_follow)).then_some(line)
        }
        Expression::BinaryOp { lhs, .. } | Expression::AssignOp { lhs, .. } => {
            Some(first_line(lhs))
        }
        Expression::TernaryOp { cond, .. } => Some(first_line(cond)),
    }
}

pub(super) fn first_line(expr: &Expression) -> u32 {
    match expr {
        Expression::Base { term, .. } => term.location.line,
        Expression::BinaryOp { lhs, .. } | Expression::AssignOp { lhs, .. } => first_line(lhs),
        Expression::TernaryOp { cond, .. } => first_line(cond),
    }
}

pub(crate) fn fold_initializer(
    tree: &dreammaker::objtree::ObjectTree,
    ty: dreammaker::objtree::TypeRef,
    expr: &Expression,
    byond: ByondVersion,
) -> Option<dreammaker::constants::Constant> {
    use dreammaker::constants::Constant as C;
    let value = dreammaker::objtree::ProcValue {
        location: dreammaker::Location::BUILTINS,
        parameters: Box::new([]),
        docs: Default::default(),
        code: Some(Box::new([])),
        body_range: None,
        first_line: None,
        next_lines: Box::new([]),
        end_lines: Box::new([]),
        operator_lines: Box::new([]),
    };
    let source = procs::ProcSource {
        tree,
        ty,
        name: "(init)",
        value: &value,
        file: "",
        debug_info: false,
        index: None,
        byond,
        file_names: None,
    };
    let ctx = procs::proc_ctx(&source, vec![]);
    let mut compiler = Compiler::new(vec![], Some(ctx));
    compiler.byond = byond;
    if let Expression::BinaryOp {
        op: dreammaker::ast::BinaryOp::In,
        lhs,
        rhs,
    } = expr
    {
        let item = fold::value(&compiler, lhs)?;
        let Expression::Base { term, follow } = &**rhs else {
            return None;
        };
        let dreammaker::ast::Term::List(args) = &term.elem else {
            return None;
        };
        if !follow.is_empty() {
            return None;
        }
        let mut found = false;
        for arg in args.iter() {
            let key = match arg {
                Expression::AssignOp { lhs, .. } => &**lhs,
                arg => arg,
            };
            found |= fold::value(&compiler, key)? == item;
        }
        return Some(C::Float(found as i32 as f32));
    }
    Some(match fold::value(&compiler, expr)? {
        fold::Constant::Null => C::Null(None),
        fold::Constant::Number(n) => C::Float(n),
        fold::Constant::Text(text) => {
            let text = String::from_utf8(text).ok()?;
            if text.contains(['\\', '"', '[', ']']) {
                return None;
            }
            C::String(text.into())
        }
        fold::Constant::Path(_) | fold::Constant::New => return None,
    })
}
