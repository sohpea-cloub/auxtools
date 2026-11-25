use dreammaker::ast::{AssignOp, Expression, Term};

use crate::{
    compiler::{args, CompileError, Compiler, EvalKind},
    operands::{self, DMString, Label, Variable},
    Instruction,
};

#[allow(non_camel_case_types)]
enum DefaultValue {
    null,
    Number(f32),
}

impl DefaultValue {
    fn into_value(self) -> operands::Value {
        match self {
            Self::null => operands::Value::Null,
            Self::Number(val) => operands::Value::Number(val),
        }
    }
}

impl From<i32> for DefaultValue {
    fn from(val: i32) -> Self {
        Self::Number(val as f32)
    }
}

impl From<f32> for DefaultValue {
    fn from(val: f32) -> Self {
        Self::Number(val)
    }
}

macro_rules! simple_stack_procs {
    (
        $(
            /proc/$proc_name:ident
            (
                $(
                    $param_name:ident
                    $(
                        = $param_default:tt
                    )?
                ),* $(,)?
            ) => $instruction:expr
        ),* $(,)?
    ) => {
        fn eval_simple_stack_procs(
            compiler: &mut Compiler<'_>,
            name: &str,
            args: &Vec<Expression>,
        ) -> Result<Option<EvalKind>, CompileError> {
            match name {
                $(
                    stringify!($proc_name) => {
                        #[allow(unused_mut)]
                        let mut arg_idx = 0;
                        let arg_count = args.len();

                        args::emit_normal(compiler, args::ArgsContext::Proc, args.to_owned())?;

                        $(
                            arg_idx += 1;

                            if arg_count < arg_idx {
                                #[allow(unused)]
                                let mut default: Option<DefaultValue> = None;

                                $(
                                    #[allow(unused_imports)]
                                    use DefaultValue::null;
                                    default = Some(DefaultValue::from($param_default));
                                )?

                                match default {
                                    Some(DefaultValue::Number(n)) if n.fract() == 0.0 && n.abs() < 1e9 => {
                                        compiler.emit_ins(Instruction::PushInt(n as i32));
                                    }
                                    Some(default) => {
                                        compiler.emit_ins(Instruction::PushVal(default.into_value()));
                                    }

                                    None => {
                                        return Err(CompileError::MissingArgument {
                                            proc: stringify!($proc_name).to_owned(),
                                            index: arg_idx as u32,
                                        });
                                    }
                                }
                            }
                        )*

                        if arg_count > arg_idx {
                            return Err(CompileError::TooManyArguments {
                                proc: stringify!($proc_name).to_owned(),
                                expected: arg_idx as u32,
                            });
                        }

                        compiler.emit_ins(super::versions::instruction(compiler.byond, $instruction));
                        Ok(Some(EvalKind::Stack))
                    }
                )*

                _ => Ok(None),
            }
        }
    }
}

simple_stack_procs! {
    /proc/abs(val) => Instruction::Abs,
    /proc/alert(param_1, param_2 = null, param_3 = null, param_4 = null, param_5 = null, param_6 = null) => Instruction::Alert,
    /proc/arccos(val) => Instruction::ArcCos,
    /proc/arcsin(val) => Instruction::ArcSin,
    /proc/ascii2text(val) => Instruction::Ascii2Text,
    /proc/bounds_dist(ref, target) => Instruction::BoundsDist,
    /proc/ckey(text) => Instruction::CKey,
    /proc/cKey(text) => Instruction::CKeyEx,
    /proc/ckeyEx(text) => Instruction::CKeyEx,
    /proc/clamp(val, low, high) => Instruction::Clamp,
    /proc/floor(val) => Instruction::Round,
    /proc/ceil(val) => Instruction::Ceil,
    /proc/trunc(val) => Instruction::Trunc,
    /proc/fract(val) => Instruction::Fract,
    /proc/sign(val) => Instruction::Sign,
    /proc/trimtext(text) => Instruction::TrimText,
    /proc/refcount(val) => Instruction::RefCount,
    /proc/lerp(a, b, t) => Instruction::Lerp,
    /proc/ftime(file, local = null) => Instruction::FTime,
    /proc/get_steps_to(ref, target, min = null) => Instruction::GetStepsTo,
    /proc/viewers(depth = null, center = null) => Instruction::Viewers,
    /proc/copytext(text, start, end = null) => Instruction::CopyText,
    /proc/copytext_char(text, start, end = null) => Instruction::CopyTextChar,
    /proc/cos(val) => Instruction::Cos2,
    /proc/fcopy(src, dst) => Instruction::FCopy,
    /proc/fcopy_rsc(path) => Instruction::FCopyRsc,
    /proc/fdel(path) => Instruction::FDel,
    /proc/fexists(path) => Instruction::FExists,
    /proc/file2text(path) => Instruction::File2Text,
    /proc/findlasttext(haystack, needle, start = 0, end = 1) => Instruction::FindLastText,
    /proc/findlasttextEx(haystack, needle, start = 0, end = 1) => Instruction::FindLastTextEx,
    /proc/findlasttext_char(haystack, needle, start = 0, end = 1) => Instruction::FindLastTextChar,
    /proc/findlasttextEx_char(haystack, needle, start = 0, end = 1) => Instruction::FindLastTextExChar,
    /proc/findtext(haystack, needle, start = 1, end = null) => Instruction::FindText,
    /proc/findtext_char(haystack, needle, start = 1, end = null) => Instruction::FindTextChar,
    /proc/findtextEx(haystack, needle, start = 1, end = null) => Instruction::FindTextEx,
    /proc/findtextEx_char(haystack, needle, start = 1, end = null) => Instruction::FindTextExChar,
    /proc/flist(path) => Instruction::FList,
    /proc/get_dir(loc1, loc2) => Instruction::GetDir,
    /proc/get_dist(loc1, loc2) => Instruction::GetDist,
    /proc/get_step(ref, dir) => Instruction::GetStep,
    /proc/get_step_away(ref, dir, max = null) => Instruction::GetStepAway,
    /proc/get_step_rand(ref) => Instruction::GetStepRand,
    /proc/get_step_to(ref, target, min = null) => Instruction::GetStepTo,
    /proc/get_step_towards(ref, target) => Instruction::GetStepTowards,
    /proc/hascall(obj, proc_name) => Instruction::HasCall,
    /proc/ispointer(val) => Instruction::IsPointer,
    /proc/hearers(depth = null, center = null) => Instruction::Hearers,
    /proc/html_decode(text) => Instruction::HtmlDecode,
    /proc/html_encode(text) => Instruction::HtmlEncode,
    /proc/isfile(val) => Instruction::IsFile,
    /proc/isicon(val) => Instruction::IsIcon,
    /proc/islist(val) => Instruction::IsList,
    /proc/isnull(val) => Instruction::IsNull,
    /proc/isnum(val) => Instruction::IsNum,
    /proc/istext(val) => Instruction::IsText,
    /proc/jointext(list, glue, start = 1, end = null) => Instruction::JoinText,
    /proc/length(val) => Instruction::Length,
    /proc/length_char(val) => Instruction::LengthChar,
    /proc/load_ext(library, function) => Instruction::LoadExt,
    /proc/lentext(text) => Instruction::Length,
    /proc/list2params(list) => Instruction::List2Params,
    /proc/lowertext(text) => Instruction::LowerText,
    /proc/missile(type, start, end) => Instruction::Missile,
    /proc/md5(text) => Instruction::Md5,
    /proc/nonspantext(haystack, needles, start = 1) => Instruction::NonSpanText,
    /proc/nonspantext_char(haystack, needles, start = 1) => Instruction::NonSpanTextChar,
    /proc/ohearers(depth = null, center = null) => Instruction::OHearers,
    /proc/orange(dist = null, center = null) => Instruction::ORange,
    /proc/oview(dist = null, center = null) => Instruction::OView,
    /proc/oviewers(depth = null, center = null) => Instruction::OViewers,
    /proc/params2list(params) => Instruction::Params2List,
    /proc/prob(val) => Instruction::Prob,
    /proc/range(dist = null, center = null) => Instruction::Range,
    /proc/ref(val) => Instruction::Ref,
    /proc/replacetext(haystack, needle, replacement, start = 1, end = null) => Instruction::ReplaceText,
    /proc/replacetext_char(haystack, needle, replacement, start = 1, end = null) => Instruction::ReplaceTextChar,
    /proc/replacetextEx(haystack, needle, replacement, start = 1, end = null) => Instruction::ReplaceTextEx,
    /proc/replacetextEx_char(haystack, needle, replacement, start = 1, end = null) => Instruction::ReplaceTextExChar,
    /proc/rgb2num(color, space = 0) => Instruction::Rgb2Num,
    /proc/sha1(arg) => Instruction::Sha1,
    /proc/sin(val) => Instruction::Sin2,
    /proc/spantext(haystack, needles, start = 1) => Instruction::SpanText,
    /proc/spantext_char(haystack, needles, start = 1) => Instruction::SpanTextChar,
    /proc/splicetext(text, start, end = null, insert = null) => Instruction::SpliceText,
    /proc/splicetext_char(text, start, end = null, insert = null) => Instruction::SpliceTextChar,
    /proc/splittext(text, delimiter, start = 1, end = null, include_delimiters = null) => Instruction::SplitText,
    /proc/splittext_char(text, delimiter, start = 1, end = null, include_delimiters = null) => Instruction::SplitTextChar,
    /proc/sqrt(val) => Instruction::Sqrt,
    /proc/tan(val) => Instruction::Tan2,
    /proc/text2ascii(text, pos = null) => Instruction::Text2Ascii,
    /proc/text2ascii_char(text, pos = null) => Instruction::Text2AsciiChar,
    /proc/text2file(text, file) => Instruction::Text2File,
    /proc/text2path(text) => Instruction::Text2Path,
    /proc/turn(var, angle) => Instruction::Turn,
    /proc/uppertext(text) => Instruction::UpperText,
    /proc/url_decode(text) => Instruction::UrlDecode,
    /proc/url_encode(text, format = null) => Instruction::UrlEncode,
    /proc/view(dist = null, center = null) => Instruction::View,
    /proc/winexists(player, control_id) => Instruction::WinExists,
    /proc/winget(player, control_id, params) => Instruction::WinGet,
    /proc/values_sum(list) => Instruction::ValuesSum,
    /proc/values_product(list) => Instruction::ValuesProduct,
    /proc/values_dot(a, b) => Instruction::ValuesDot,
    /proc/values_cut_under(list, min, inclusive = null) => Instruction::ValuesCutUnder,
    /proc/values_cut_over(list, max, inclusive = null) => Instruction::ValuesCutOver,
    /proc/bound_pixloc(atom, dir) => Instruction::BoundPixloc,
    /proc/_dm_db_new_query() => Instruction::DbNewQuery,
    /proc/_dm_db_execute(a, b, c, d, e) => Instruction::DbExecute,
    /proc/_dm_db_next_row(a, b, c) => Instruction::DbNextRow,
    /proc/_dm_db_rows_affected(a) => Instruction::DbRowsAffected,
    /proc/_dm_db_row_count(a) => Instruction::DbRowCount,
    /proc/_dm_db_error_msg(a) => Instruction::DbErrorMsg,
    /proc/_dm_db_columns(a, b) => Instruction::DbColumns,
    /proc/_dm_db_close(a) => Instruction::DbClose,
    /proc/_dm_db_new_con() => Instruction::DbNewConnection,
    /proc/_dm_db_connect(a, b, c, d, e, f) => Instruction::DbConnect,
    /proc/_dm_db_quote(a, b) => Instruction::DbQuote,
    /proc/_dm_db_is_connected(a) => Instruction::DbIsConnected,
}

macro_rules! simple_vararg_procs {
    (
        $(
            /proc/$proc_name:ident
            (
                $($min_args:literal)?
                $(..= $max_args:literal)?
            ) => $instruction:expr
        ),* $(,)?
    ) => {
        fn eval_simple_vararg_procs(
            compiler: &mut Compiler<'_>,
            name: &str,
            args: &[Expression],
        ) -> Result<Option<EvalKind>, CompileError> {
            let args_len = args.len() as u32;

            match name {
                $(
                    stringify!($proc_name) => {
                        $(
                            if args_len < $min_args {
                                return Err(CompileError::IncorrectArgCount(name.to_owned()));
                            }
                        )?

                        $(
                            if args_len > $max_args {
                                return Err(CompileError::IncorrectArgCount(name.to_owned()));
                            }
                        )?

                        args::emit_normal(compiler, args::ArgsContext::Proc, args.to_vec())?;
                        compiler.emit_ins($instruction(args_len));
                        Ok(Some(EvalKind::Stack))
                    }
                )*

                _ => Ok(None),
            }
        }
    }
}

simple_vararg_procs! {
    /proc/addtext(2) => Instruction::AddText,
    /proc/bounds(..=5) => Instruction::Bounds,
    /proc/matrix(..=6) => Instruction::MatrixNew,
    /proc/max(2) => Instruction::Max,
    /proc/min(2) => Instruction::Min,
    /proc/obounds(..=5) => Instruction::OBounds,
    /proc/regex() => Instruction::RegexNew,
    /proc/sorttext(2) => Instruction::SortText,
    /proc/sortText(2) => Instruction::SortTextEx,
    /proc/sorttextEx(2) => Instruction::SortTextEx,
    /proc/noise_hash(1) => Instruction::NoiseHash,
    /proc/startup(1) => Instruction::Startup,
    /proc/typesof(1) => Instruction::TypesOf,
}

macro_rules! unsupported_procs {
    (
        $(
            /proc/$proc_name:ident
        ),* $(,)?
    ) => {
        fn is_unsupported_proc(
            name: &str,
        ) -> bool {
            match name {
                $(
                    stringify!($proc_name) => true,
                )*

                _ => false,
            }
        }
    }
}

unsupported_procs! {
    /proc/list,
    /proc/locate,
    /proc/pick,
    /proc/input,

    /proc/shutdown,

    /proc/browse,
    /proc/browse_rsc,
    /proc/ftp,
    /proc/link,
    /proc/load_resource,
    /proc/output,
    /proc/rand_seed,
    /proc/run,
    /proc/CRASH,
}

pub(super) fn emit(
    compiler: &mut Compiler<'_>,
    name: &str,
    args: &Vec<Expression>,
) -> Result<Option<EvalKind>, CompileError> {
    if !super::versions::has_builtin(compiler.byond, name) {
        return Ok(None);
    }
    if matches!(name, "browse" | "browse_rsc" | "output" | "load_resource") && !args.is_empty() {
        push_args(compiler, &args[..1])?;
        for arg in &args[1..] {
            push_args(compiler, std::slice::from_ref(arg))?;
            compiler.emit_ins(Instruction::Pop);
        }
        return Ok(Some(EvalKind::Stack));
    }
    if name == "shutdown" && matches!(args.len(), 1 | 2) {
        push_args(compiler, args)?;
        for _ in args.len()..2 {
            push_null(compiler);
        }
        compiler.emit_ins(Instruction::ShutdownArgs);
        return Ok(Some(EvalKind::Stack));
    }
    if name == "animate" && !args.is_empty() {
        if emit_statement_builtin(compiler, name, args)? {
            let dropped = compiler.nodes.pop();
            debug_assert!(matches!(
                dropped,
                Some(crate::Node::Instruction(Instruction::Pop, _))
            ));
            return Ok(Some(EvalKind::Stack));
        }
    }
    if name == "statpanel" && args.len() == 1 {
        push_args(compiler, args)?;
        compiler.emit_ins(Instruction::StatPanelCheck);
        compiler.emit_ins(Instruction::GetFlag);
        return Ok(Some(EvalKind::Stack));
    }
    if is_unsupported_proc(name) {
        return Err(CompileError::UnsupportedBuiltin {
            proc: name.to_owned(),
        });
    }

    if name == "block" && (3..=5).contains(&args.len()) {
        push_args(compiler, args)?;
        for _ in args.len()..6 {
            compiler.emit_ins(Instruction::PushVal(operands::Value::Null));
        }
        compiler.emit_ins(Instruction::BlockXYZ);
        return Ok(Some(EvalKind::Stack));
    }
    if (name == "time2text" || name == "round") && args.is_empty() {
        push_null(compiler);
        if name == "time2text" {
            push_null(compiler);
        }
        compiler.emit_ins(if name == "time2text" {
            Instruction::Time2Text
        } else {
            Instruction::Round
        });
        return Ok(Some(EvalKind::Stack));
    }

    if name == "rgb2num" {
        if let Some(channels) = rgb2num_const(compiler, args) {
            for c in &channels {
                compiler.emit_ins(Instruction::PushInt(*c as i32));
            }
            compiler.emit_ins(Instruction::NewList(channels.len() as u32));
            return Ok(Some(EvalKind::Stack));
        }
    }

    if let Some(res) = eval_simple_stack_procs(compiler, name, args)? {
        if matches!(name, "range" | "orange") {
            compiler.emit_ins(Instruction::CopyList);
        }

        return Ok(Some(res));
    }

    if matches!(name, "min" | "max") && args.len() == 1 {
        let list = arglist_inner(&args[0]).unwrap_or(&args[0]).clone();
        args::emit_normal(compiler, args::ArgsContext::Proc, vec![list])?;
        compiler.emit_ins(if name == "min" {
            Instruction::MinList
        } else {
            Instruction::MaxList
        });
        return Ok(Some(EvalKind::Stack));
    }

    if let Some(res) = eval_simple_vararg_procs(compiler, name, args)? {
        return Ok(Some(res));
    }

    let arg_count = args.len() as u32;

    if let Some(kind) = emit_overloaded(compiler, name, args)? {
        return Ok(Some(kind));
    }

    match name {
        "arglist" => {
            if arg_count != 1 {
                return Err(CompileError::IncorrectArgCount(name.to_owned()));
            }

            args::emit_single_normal(compiler, args::ArgsContext::Proc, args[0].clone())?;
            Ok(Some(EvalKind::ArgList))
        }

        "initial" => {
            if arg_count != 1 {
                return Err(CompileError::IncorrectArgCount(name.to_owned()));
            }

            if let Some(var) = initial_static_field(compiler, &args[0])? {
                compiler.emit_ins(Instruction::GetVar(var));
                return Ok(Some(EvalKind::Stack));
            }
            if let Some(var) = initial_safe(compiler, &args[0])? {
                compiler.emit_ins(Instruction::GetVar(var));
                return Ok(Some(EvalKind::Stack));
            }
            let arg = args[0].clone();
            let var = match compiler.emit_expr(arg)? {
                EvalKind::Field(builder, field) => {
                    builder.get_initial_field(DMString(field.into()))
                }

                EvalKind::Var(global @ Variable::Global(_)) => Variable::Initial(Box::new(global)),

                EvalKind::Var(
                    var @ (Variable::Local(_)
                    | Variable::Arg(_)
                    | Variable::Src
                    | Variable::Usr
                    | Variable::Dot),
                ) => Variable::Initial(Box::new(var)),

                EvalKind::ListRef => {
                    compiler.emit_ins(Instruction::SetVar(Variable::CacheKey));
                    compiler.emit_ins(Instruction::SetVar(Variable::Cache));
                    Variable::Initial(Box::new(Variable::CacheIndex))
                }

                _ => return Err(CompileError::ExpectedFieldReference),
            };

            compiler.emit_ins(Instruction::GetVar(var));
            Ok(Some(EvalKind::Stack))
        }

        _ => Ok(None),
    }
}

fn rgb_slots(args: &[Expression]) -> Option<([Option<Expression>; 5], Option<f32>)> {
    let mut slots: [Option<Expression>; 5] = Default::default();
    let mut names = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        match arg {
            Expression::AssignOp {
                op: AssignOp::Assign,
                lhs,
                rhs,
            } => {
                let name = ident_name(lhs)?;
                let slot = match name.as_str() {
                    "r" | "red" | "h" | "hue" => 0,
                    "g" | "green" | "s" | "saturation" | "c" | "chroma" => 1,
                    "b" | "blue" | "v" | "value" | "l" | "luminance" | "y" => 2,
                    "a" | "alpha" => 3,
                    "space" => 4,
                    _ => return None,
                };
                names.push(name);
                slots[slot] = Some((**rhs).clone());
            }
            _ if i < 5 && names.is_empty() => slots[i] = Some(arg.clone()),
            _ => return None,
        }
    }
    if slots[..3].iter().any(Option::is_none) {
        return None;
    }
    let has = |n: &str| names.iter().any(|x| x == n);
    let space = if has("h") || has("hue") {
        if has("v") || has("value") {
            Some(1.0)
        } else if has("l") || has("luminance") {
            Some(2.0)
        } else {
            Some(3.0)
        }
    } else {
        None
    };
    Some((slots, space))
}

pub(super) fn rgb_constant(compiler: &Compiler<'_>, args: &[Expression]) -> Option<String> {
    let ([r, g, b, a, sp], named_space) = rgb_slots(args)?;
    let space = match (named_space, &sp) {
        (Some(space), _) => space,
        (None, Some(sp)) => super::fold::number(compiler, sp)?,
        (None, None) => 0.0,
    };
    if space.fract() != 0.0 || !(0.0..=3.0).contains(&space) {
        return None;
    }
    let number = |e: &Option<Expression>| super::fold::number(compiler, e.as_ref()?);
    let channels = [number(&r)?, number(&g)?, number(&b)?];
    let alpha = match &a {
        None => None,
        Some(a) => Some(match super::fold::value(compiler, a)? {
            super::fold::Constant::Number(n) => n,
            super::fold::Constant::Null | super::fold::Constant::Text(_) => 0.0,
            _ => return None,
        }),
    };
    let (text, boundary) = super::fold::rgb_text(compiler.byond, space as u32, channels, alpha)?;
    if boundary {
        compiler.warn(&format!(
            "rgb() is folded to {text}, matching Linux DreamMaker; Windows may round a channel differently"
        ));
    }
    Some(text)
}

fn emit_rgb(
    compiler: &mut Compiler<'_>,
    args: &[Expression],
) -> Result<Option<EvalKind>, CompileError> {
    let Some((slots, space)) = rgb_slots(args) else {
        return Ok(None);
    };
    if let Some(text) = rgb_constant(compiler, args) {
        compiler.emit_ins(Instruction::PushVal(operands::Value::DMString(DMString(
            text.into_bytes(),
        ))));
        return Ok(Some(EvalKind::Stack));
    }
    let [r, g, b, a, sp] = slots;
    for e in [&r, &g, &b] {
        push_args(compiler, &[e.clone().unwrap()])?;
    }
    if sp.is_none() && space.is_none() {
        match a {
            Some(a) => {
                push_args(compiler, &[a])?;
                compiler.emit_ins(Instruction::Rgba);
            }
            None => compiler.emit_ins(Instruction::Rgb),
        }
        return Ok(Some(EvalKind::Stack));
    }
    let folds_to_zero = |compiler: &Compiler<'_>, a: &Expression| {
        let number =
            |e: &Option<Expression>| e.as_ref().and_then(|e| super::fold::number(compiler, e));
        [&r, &g, &b].iter().all(|e| number(e).is_some())
            && (space.is_some() || number(&sp).is_some())
            && matches!(
                super::fold::value(compiler, a),
                Some(super::fold::Constant::Null | super::fold::Constant::Text(_))
            )
    };
    match a {
        Some(a) if folds_to_zero(compiler, &a) => compiler.emit_number(0.0),
        Some(a) => push_args(compiler, &[a])?,
        None => push_null(compiler),
    }
    match (sp, space) {
        (Some(sp), _) => push_args(compiler, &[sp])?,
        (None, Some(n)) => compiler.emit_number(n),
        (None, None) => unreachable!(),
    }
    compiler.emit_ins(Instruction::RgbEx);
    Ok(Some(EvalKind::Stack))
}

fn is_list_literal(expr: &Expression) -> bool {
    matches!(expr, Expression::Base { term, follow } if follow.is_empty() && matches!(term.elem, Term::List(_)))
}

fn ident_name(expr: &Expression) -> Option<String> {
    match expr {
        Expression::Base { term, follow } if follow.is_empty() => match &term.elem {
            Term::Ident(name) => Some(name.to_string()),
            _ => None,
        },
        _ => None,
    }
}

fn push_args(compiler: &mut Compiler<'_>, args: &[Expression]) -> Result<(), CompileError> {
    args::emit_normal(compiler, args::ArgsContext::Proc, args.to_vec())
}

fn push_null(compiler: &mut Compiler<'_>) {
    compiler.emit_ins(Instruction::PushVal(operands::Value::Null));
}

fn emit_overloaded(
    compiler: &mut Compiler<'_>,
    name: &str,
    args: &[Expression],
) -> Result<Option<EvalKind>, CompileError> {
    let n = args.len();
    let by_count =
        |ins: &[(usize, Instruction)]| ins.iter().find(|(c, _)| *c == n).map(|(_, i)| i.clone());
    let simple = match name {
        "round" => by_count(&[(1, Instruction::Round), (2, Instruction::RoundN)]),
        "text2num" => by_count(&[(1, Instruction::Text2Num), (2, Instruction::Text2NumRadix)]),
        "num2text" => by_count(&[
            (1, Instruction::Num2Text),
            (2, Instruction::Num2TextSigFigs),
            (3, Instruction::Num2TextRadix),
        ]),
        "log" => by_count(&[(1, Instruction::Log10), (2, Instruction::Log)]),
        "arctan" => by_count(&[(1, Instruction::ArcTan), (2, Instruction::ArcTan2)]),
        "ispath" => by_count(&[(1, Instruction::IsPath), (2, Instruction::IsSubPath)]),
        "roll" => by_count(&[(1, Instruction::RollStr), (2, Instruction::Roll)]),
        "block" => by_count(&[(2, Instruction::Block), (6, Instruction::BlockXYZ)]),
        "shell" => by_count(&[(1, Instruction::Shell)]),
        "icon_states" => by_count(&[
            (1, Instruction::IconStates),
            (2, Instruction::IconStatesMode),
        ]),
        "json_encode" => by_count(&[
            (1, Instruction::JsonEncode),
            (2, Instruction::JsonEncodeFlags(2)),
        ]),
        "json_decode" => by_count(&[
            (1, Instruction::JsonDecode),
            (2, Instruction::JsonDecodeFlags(2)),
        ]),
        "astype" => by_count(&[(2, Instruction::AsType)]),
        "time2text" if n == 3 => Some(Instruction::Time2TextTZ(3)),
        "istype" if n == 2 => Some(Instruction::IsType),
        _ => None,
    };
    if let Some(ins) = simple {
        push_args(compiler, args)?;
        compiler.emit_ins(ins);
        return Ok(Some(EvalKind::Stack));
    }

    match name {
        "time2text" if n == 1 || n == 2 => {
            push_args(compiler, args)?;
            if n == 1 {
                push_null(compiler);
            }
            compiler.emit_ins(Instruction::Time2Text);
        }
        "rand" if n <= 2 => {
            push_args(compiler, args)?;
            match n {
                0 => {
                    push_null(compiler);
                    compiler.emit_ins(Instruction::Rand);
                }
                1 => compiler.emit_ins(Instruction::Rand),
                _ => compiler.emit_ins(Instruction::RandRange),
            }
        }
        "isturf" | "ismob" | "isarea" | "isobj" | "ismovable" | "isloc" | "isnan" | "isinf"
            if n >= 1 =>
        {
            let test = match name {
                "isturf" => Instruction::IsTurf,
                "ismob" => Instruction::IsMob,
                "isarea" => Instruction::IsArea,
                "isobj" => Instruction::IsObj,
                "ismovable" => Instruction::IsMovable,
                "isloc" => Instruction::IsLoc,
                "isnan" => Instruction::IsNaN,
                _ => Instruction::IsInf,
            };
            if n == 1 && super::versions::folds(compiler.byond, name) {
                let known = super::fold::number(compiler, &args[0])
                    .and_then(|x| super::versions::nan_test(name, x));
                if let Some(result) = known {
                    compiler.emit_ins(Instruction::PushInt(result as i32));
                    if !super::versions::clean_nan_fold(compiler.byond) {
                        compiler.warn_bug(
                            &format!(
                                "{name}() of a constant returns the previous test flag on this build"
                            ),
                            ", matching DreamMaker",
                        );
                        compiler.emit_ins(Instruction::GetFlag);
                    }
                    return Ok(Some(EvalKind::Stack));
                }
            }
            let end = compiler.new_label();
            for arg in args {
                push_args(compiler, std::slice::from_ref(arg))?;
                compiler.emit_ins(test.clone());
                if n > 1 {
                    compiler.emit_ins(Instruction::Jz(Label::Named(end.clone())));
                }
            }
            if n > 1 {
                compiler.emit_label(end);
            }
            compiler.emit_ins(Instruction::GetFlag);
        }
        "text"
            if n == 1
                && !matches!(&args[0], Expression::Base { term, .. } if matches!(&term.elem, Term::String(_))) =>
        {
            let kind = compiler.emit_expr(args[0].clone())?;
            compiler.emit_move_to_stack(kind)?;
        }
        "text" if n >= 1 => {
            let mut text = super::strings::TextBuilder::new();
            let mut values: Vec<Expression> = vec![];
            let mut rest = args[1..].iter();
            match &args[0] {
                Expression::Base { term, follow } if follow.is_empty() => match &term.elem {
                    Term::String(s) => text.push(s.as_str())?,
                    Term::InterpString(first, parts) => {
                        text.push(first.as_str())?;
                        for (e, literal) in parts.iter() {
                            text.embed();
                            match e {
                                Some(e) => values.push(e.clone()),
                                None => match rest.next() {
                                    Some(arg) => values.push(arg.clone()),
                                    None => {
                                        return Err(CompileError::IncorrectArgCount(
                                            "text".to_owned(),
                                        ))
                                    }
                                },
                            }
                            text.push(literal.as_str())?;
                        }
                    }
                    _ => return Ok(None),
                },
                _ => return Ok(None),
            }
            if rest.next().is_some() {
                return Err(CompileError::IncorrectArgCount("text".to_owned()));
            }
            let embeds = values.len();
            push_args(compiler, &values)?;
            compiler.emit_ins(Instruction::Format(DMString(text.buf), embeds as u32));
        }
        "REGEX_QUOTE" | "REGEX_QUOTE_REPLACEMENT" if n == 1 => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::PushInt(if name == "REGEX_QUOTE" {
                1
            } else {
                2
            }));
            compiler.emit_ins(Instruction::RegexNew(2));
        }
        "rgb" => return emit_rgb(compiler, args),
        "astype" if n == 1 && compiler.implicit_type.is_some() => {
            let path = compiler.implicit_type.take().unwrap();
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::PushVal(operands::Value::Path(path)));
            compiler.emit_ins(Instruction::AsType);
        }
        "astype" if n == 1 && compiler.static_type_name(&args[0]).is_some() => {
            let path = compiler.static_type_name(&args[0]).unwrap();
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::PushVal(operands::Value::Path(path)));
            compiler.emit_ins(Instruction::AsType);
        }
        "astype" if n == 1 && compiler.expr_scope.is_some() => {
            let path = declared_world_type(compiler, &args[0]).ok_or(
                CompileError::UnsupportedBuiltin {
                    proc: "astype with an untyped value".to_owned(),
                },
            )?;
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::PushVal(operands::Value::Path(path)));
            compiler.emit_ins(Instruction::AsType);
        }
        "shell" if n == 0 => compiler.emit_ins(Instruction::ShellAllowed),
        "issaved" if n == 1 => {
            let var = match compiler.emit_inner_expr(args[0].clone())? {
                EvalKind::ListRef => {
                    compiler.emit_ins(Instruction::SetVar(Variable::CacheKey));
                    compiler.emit_ins(Instruction::SetVar(Variable::Cache));
                    Variable::IsSaved(Box::new(Variable::CacheIndex))
                }
                EvalKind::Field(builder, field) => builder.get_leaf(Variable::IsSaved(Box::new(
                    Variable::Field(DMString(field.into())),
                ))),
                EvalKind::Var(var) => Variable::IsSaved(Box::new(var)),
                _ => return Err(CompileError::ExpectedFieldReference),
            };
            compiler.emit_ins(Instruction::GetVar(var));
        }
        "vector" | "pixloc" if (1..=3).contains(&n) => {
            push_args(compiler, args)?;
            let count = n as u32;
            compiler.emit_ins(if name == "vector" {
                Instruction::NewVector(count)
            } else {
                Instruction::NewPixloc(count)
            });
        }
        _ if name.starts_with("step") && movement(name, n).is_some() => {
            let (base, ins) = movement(name, n).unwrap();
            push_args(compiler, args)?;
            for _ in n..base {
                push_null(compiler);
            }
            compiler.emit_ins(ins);
            compiler.emit_ins(Instruction::GetFlag);
        }
        "cmptext" | "cmptextEx" if n >= 2 => {
            let compare = if name == "cmptext" {
                Instruction::CmpText
            } else {
                Instruction::Teq
            };
            let end = compiler.new_label();
            push_args(compiler, &args[..1])?;
            for (i, arg) in args[1..].iter().enumerate() {
                if i > 0 {
                    compiler.emit_ins(Instruction::Jz(Label::Named(end.clone())));
                }
                push_args(compiler, std::slice::from_ref(arg))?;
                compiler.emit_ins(compare.clone());
            }
            if n > 2 {
                compiler.emit_label(end);
            }
            compiler.emit_ins(Instruction::Pop);
            compiler.emit_ins(Instruction::GetFlag);
        }
        "icon" | "sound" | "generator" => {
            compiler.emit_ins(Instruction::PushVal(operands::Value::DMString(DMString(
                format!("/{}", name).into_bytes(),
            ))));
            let count = args.len() as u32;
            match args::emit(compiler, args::ArgsContext::Proc, args.to_vec())? {
                args::ArgsResult::Normal => compiler.emit_ins(Instruction::New(count)),
                args::ArgsResult::Assoc => {
                    compiler.emit_ins(Instruction::NewAssocList(count));
                    compiler.emit_ins(Instruction::NewArgList);
                }
                args::ArgsResult::ArgList => compiler.emit_ins(Instruction::NewArgList),
            }
        }
        "file" if n == 1 => {
            compiler.emit_ins(Instruction::PushVal(operands::Value::File));
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::New(1));
        }
        "image" => {
            let count = args.len() as u32;
            match args::emit(compiler, args::ArgsContext::Proc, args.to_vec())? {
                args::ArgsResult::Normal if count == 2 => compiler.emit_ins(Instruction::NewImage),
                args::ArgsResult::Normal => compiler.emit_ins(Instruction::NewImageArgs(count)),
                args::ArgsResult::Assoc => {
                    compiler.emit_ins(Instruction::NewAssocList(count));
                    compiler.emit_ins(Instruction::NewImageArgList);
                }
                args::ArgsResult::ArgList => compiler.emit_ins(Instruction::NewImageArgList),
            }
        }
        "filter" => {
            let count = args.len() as u32;
            match args::emit(compiler, args::ArgsContext::Proc, args.to_vec())? {
                args::ArgsResult::Assoc => compiler.emit_ins(Instruction::NewAssocList(count)),
                args::ArgsResult::ArgList => {}
                args::ArgsResult::Normal => {
                    return Err(CompileError::IncorrectArgCount(name.to_owned()))
                }
            }
            compiler.emit_ins(Instruction::FilterNewArgList);
        }
        "gradient" => {
            let named = args.iter().any(|a| {
                matches!(
                    a,
                    Expression::AssignOp {
                        op: AssignOp::Assign,
                        ..
                    }
                )
            });
            if named {
                for (i, arg) in args.iter().enumerate() {
                    match arg {
                        Expression::AssignOp {
                            op: AssignOp::Assign,
                            lhs,
                            rhs,
                        } => {
                            let name =
                                ident_name(lhs).ok_or(CompileError::UnexpectedNamedArguments)?;
                            compiler.emit_ins(Instruction::PushVal(operands::Value::DMString(
                                DMString(name.into_bytes()),
                            )));
                            push_args(compiler, &[(**rhs).clone()])?;
                        }
                        _ => {
                            compiler.emit_number((i + 1) as f32);
                            push_args(compiler, std::slice::from_ref(arg))?;
                        }
                    }
                }
                compiler.emit_ins(Instruction::NewAssocList(n as u32));
            } else if let Some(list) = (n == 1).then(|| arglist_inner(&args[0])).flatten() {
                push_args(compiler, std::slice::from_ref(list))?;
            } else if n == 2 && is_list_literal(&args[0]) {
                push_args(compiler, args)?;
                compiler.emit_ins(Instruction::GradientIndex);
                return Ok(Some(EvalKind::Stack));
            } else {
                push_args(compiler, args)?;
                compiler.emit_ins(Instruction::NewList(n as u32));
            }
            compiler.emit_ins(Instruction::Gradient);
        }
        "newlist" => {
            for arg in args {
                push_args(compiler, std::slice::from_ref(arg))?;
                compiler.emit_ins(Instruction::New(0));
            }
            compiler.emit_ins(Instruction::NewList(n as u32));
        }
        "nameof" if n == 1 => {
            let mut name = last_identifier(&args[0]).ok_or(CompileError::ExpectedFieldReference)?;
            if name == "__PROC__" {
                if !compiler.proc_macros_known() {
                    return Err(CompileError::UnsupportedCompilerMacro {
                        name: "__PROC__".to_string(),
                    });
                }
                let ctx = compiler
                    .proc
                    .as_ref()
                    .ok_or(CompileError::ExpectedFieldReference)?;
                name = ctx
                    .proc_path
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_owned();
            }
            compiler.emit_ins(Instruction::PushVal(operands::Value::DMString(DMString(
                name.into_bytes(),
            ))));
        }
        "istype" if n == 1 => {
            let path = compiler
                .static_type_name(&args[0])
                .or_else(|| declared_world_type(compiler, &args[0]))
                .ok_or(CompileError::UnsupportedBuiltin {
                    proc: "istype with an untyped value".to_owned(),
                })?;
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::PushVal(operands::Value::Path(path)));
            compiler.emit_ins(Instruction::IsType);
        }
        _ => return Ok(None),
    }
    Ok(Some(EvalKind::Stack))
}

fn declared_world_type(compiler: &Compiler, arg: &Expression) -> Option<String> {
    compiler.expr_scope?;
    let path = super::procs::declared_type_path(compiler, arg).filter(|path| !path.is_empty())?;
    Some(if path.split('/').any(|part| part == "list") {
        "/list".to_owned()
    } else {
        path
    })
}

pub(super) fn last_identifier(expr: &Expression) -> Option<String> {
    use dreammaker::ast::{Follow, Term};
    match expr {
        Expression::Base { term, follow } => match follow.last().map(|f| &f.elem) {
            Some(
                Follow::Field(_, ident) | Follow::StaticField(ident) | Follow::ProcReference(ident),
            ) => Some(ident.as_str().to_owned()),
            Some(_) => None,
            None => match &term.elem {
                Term::Ident(ident) => Some(ident.as_str().to_owned()),
                Term::Prefab(prefab) => prefab
                    .path
                    .to_string()
                    .rsplit('/')
                    .next()
                    .map(str::to_owned),
                Term::Expr(inner) => last_identifier(inner),
                Term::__PROC__ => Some("__PROC__".to_owned()),
                _ => None,
            },
        },
        _ => None,
    }
}

fn movement(name: &str, n: usize) -> Option<(usize, Instruction)> {
    use Instruction::*;
    let (min, base, plain, speed) = match name {
        "step" => (2, 2, Step, StepSpeed),
        "step_to" => (2, 3, StepTo, StepToSpeed),
        "step_away" => (2, 3, StepAway, StepAwaySpeed),
        "step_towards" => (2, 2, StepTowards, StepTowardsSpeed),
        "step_rand" => (1, 1, StepRand, StepRandSpeed),
        "walk" => (2, 3, Walk, WalkSpeed),
        "walk_to" => (2, 4, WalkTo, WalkToSpeed),
        "walk_away" => (2, 4, WalkAway, WalkAwaySpeed),
        "walk_towards" => (2, 3, WalkTowards, WalkTowardsSpeed),
        "walk_rand" => (1, 2, WalkRand, WalkRandSpeed),
        _ => return None,
    };
    match n {
        _ if n < min => None,
        _ if n <= base => Some((base, plain)),
        _ if n == base + 1 => Some((n, speed)),
        _ => None,
    }
}

pub(super) fn emit_statement_builtin(
    compiler: &mut Compiler<'_>,
    name: &str,
    args: &[Expression],
) -> Result<bool, CompileError> {
    let n = args.len();
    let padded = |compiler: &mut Compiler<'_>, total: usize| -> Result<(), CompileError> {
        push_args(compiler, args)?;
        for _ in n..total {
            push_null(compiler);
        }
        Ok(())
    };
    match name {
        "sleep" if n <= 1 => {
            if n == 0 {
                compiler.emit_ins(Instruction::PushInt(0));
            } else {
                push_args(compiler, args)?;
            }
            compiler.emit_ins(Instruction::Sleep);
        }
        "flick" if n == 2 => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::Flick);
        }
        _ if movement(name, n).is_some() => {
            let (base, ins) = movement(name, n).unwrap();
            padded(compiler, base)?;
            compiler.emit_ins(ins);
        }
        "winset" if n == 3 => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::WinSet);
        }
        "rand_seed" if n == 1 => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::RandSeed);
        }
        "shutdown" if n == 0 => compiler.emit_ins(Instruction::Shutdown),
        "shutdown" => {
            push_args(compiler, args)?;
            for _ in n..2 {
                compiler.emit_ins(Instruction::PushVal(operands::Value::Null));
            }
            compiler.emit_ins(Instruction::ShutdownArgs);
            compiler.emit_ins(Instruction::Pop);
        }
        "shell" if n == 1 => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::Shell);
            compiler.emit_ins(Instruction::Pop);
        }
        "animate"
            if n >= 2
                && args.iter().all(|a| {
                    !matches!(a, Expression::AssignOp { .. }) && arglist_inner(a).is_none()
                }) =>
        {
            push_args(compiler, &args[..1])?;
            compiler.emit_ins(Instruction::NullAnimate);
            compiler.emit_ins(Instruction::Pop);
        }
        "animate" if n == 1 && !matches!(args[0], Expression::AssignOp { .. }) => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::NullAnimate);
            compiler.emit_ins(Instruction::Pop);
        }
        "animate" => {
            let count = args.len() as u32;
            match args::emit(compiler, args::ArgsContext::Proc, args.to_vec())? {
                args::ArgsResult::Assoc => compiler.emit_ins(Instruction::NewAssocList(count)),
                args::ArgsResult::ArgList => {}
                args::ArgsResult::Normal => compiler.emit_ins(Instruction::NewList(count)),
            }
            compiler.emit_ins(Instruction::Animate);
            compiler.emit_ins(Instruction::Pop);
        }
        "winshow" if n == 2 || n == 3 => {
            push_args(compiler, args)?;
            if n == 2 {
                compiler.emit_ins(Instruction::PushInt(1));
            }
            compiler.emit_ins(Instruction::WinShow);
        }
        "winclone" if n == 3 => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::WinClone);
        }
        "stat" if n == 2 => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::Stat);
        }
        "stat" if n == 1 => {
            compiler.emit_ins(Instruction::PushVal(operands::Value::Null));
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::Stat);
        }
        "statpanel" if n == 1 => {
            push_args(compiler, args)?;
            compiler.emit_ins(Instruction::StatPanelCheck);
        }
        "statpanel" if n == 2 || n == 3 => {
            push_args(compiler, &args[..1])?;
            if n == 2 {
                push_null(compiler);
            }
            push_args(compiler, &args[1..])?;
            compiler.emit_ins(Instruction::StatPanel);
        }
        "browse" | "browse_rsc" | "output" | "load_resource" if n >= 1 => {
            for arg in args {
                push_args(compiler, std::slice::from_ref(arg))?;
                compiler.emit_ins(Instruction::Pop);
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

pub(super) fn arglist_inner(expr: &Expression) -> Option<&Expression> {
    match expr {
        Expression::Base { term, follow } if follow.is_empty() => match &term.elem {
            Term::Call(name, args) if name.as_str() == "arglist" && args.len() == 1 => {
                Some(&args[0])
            }
            _ => None,
        },
        _ => None,
    }
}

fn initial_static_field(
    compiler: &mut Compiler<'_>,
    expr: &Expression,
) -> Result<Option<Variable>, CompileError> {
    use dreammaker::ast::{Follow, Term};
    let Expression::Base { term, follow } = expr else {
        return Ok(None);
    };
    let Some((last, rest)) = follow.split_last() else {
        return Ok(None);
    };
    let Follow::StaticField(name) = &last.elem else {
        return Ok(None);
    };
    let base = Expression::Base {
        term: term.clone(),
        follow: rest.to_vec().into_boxed_slice(),
    };
    let ty = match &term.elem {
        Term::Prefab(prefab) if rest.is_empty() && prefab.vars.is_empty() => compiler
            .proc
            .as_ref()
            .and_then(|ctx| ctx.tree.find(&prefab.path.to_string())),
        _ => super::procs::expr_type(compiler, &base),
    };
    let static_scope = ty
        .and_then(|ty| super::procs::member_var(ty, name.as_str()))
        .filter(super::procs::is_global_member)
        .map(|decl| decl.ty().get().path.clone());
    let kind = compiler.emit_expr(base)?;
    compiler.emit_move_to_stack(kind)?;
    compiler.emit_ins(Instruction::SetVar(Variable::Cache));
    Ok(Some(match static_scope {
        Some(scope) => Variable::Initial(Box::new(Variable::Global(
            crate::operands::GlobalVar::named(name.as_str().as_bytes(), Some(scope)),
        ))),
        None => Variable::Initial(Box::new(Variable::Field(DMString(
            name.as_str().as_bytes().to_vec(),
        )))),
    }))
}

fn initial_safe(
    compiler: &mut Compiler<'_>,
    expr: &Expression,
) -> Result<Option<Variable>, CompileError> {
    use dreammaker::ast::{Follow, PropertyAccessKind as K};
    let Expression::Base { term, follow } = expr else {
        return Ok(None);
    };
    let safe = |f: &Follow| matches!(f, Follow::Field(K::SafeDot | K::SafeColon, _));
    let Some(first) = follow.iter().position(|f| safe(&f.elem)) else {
        return Ok(None);
    };
    if !follow[first..]
        .iter()
        .all(|f| matches!(f.elem, Follow::Field(..)))
    {
        return Ok(None);
    }
    let base = Expression::Base {
        term: term.clone(),
        follow: follow[..first].to_vec().into_boxed_slice(),
    };
    let kind = compiler.emit_expr(base)?;
    compiler.emit_move_to_stack(kind)?;
    let mut segments: Vec<Vec<String>> = vec![];
    for f in &follow[first..] {
        let Follow::Field(_, name) = &f.elem else {
            unreachable!()
        };
        if safe(&f.elem) {
            segments.push(vec![]);
        }
        segments.last_mut().unwrap().push(name.to_string());
    }
    let last = segments.pop().unwrap();
    let saved = segments.len();
    let chain = |fields: &[String]| {
        let mut builder = super::chain_builder::ChainBuilder::begin(Variable::Cache);
        for f in &fields[..fields.len() - 1] {
            builder.append(DMString(f.clone().into_bytes()));
        }
        builder
    };
    for fields in segments {
        compiler.emit_ins(Instruction::SetVar(Variable::Cache));
        compiler.emit_ins(Instruction::PushCache);
        let field = DMString(fields[fields.len() - 1].clone().into_bytes());
        compiler.emit_ins(Instruction::GetVar(chain(&fields).get_field(field)));
    }
    compiler.emit_ins(Instruction::SetVar(Variable::Cache));
    for _ in 0..saved {
        compiler.emit_ins(Instruction::PopCache);
    }
    if let Some(global) = super::statements::static_place(compiler, expr, false) {
        return Ok(Some(Variable::Initial(Box::new(global))));
    }
    let field = DMString(last[last.len() - 1].clone().into_bytes());
    Ok(Some(chain(&last).get_initial_field(field)))
}

const NAMED_COLOURS: &[(&str, &[u8])] = &[
    ("aqua", &[0, 255, 255]),
    ("black", &[0, 0, 0]),
    ("blue", &[0, 0, 255]),
    ("cyan", &[0, 255, 255]),
    ("fuchsia", &[255, 0, 255]),
    ("gold", &[128, 128, 0]),
    ("gray", &[128, 128, 128]),
    ("grey", &[128, 128, 128]),
    ("green", &[0, 192, 0]),
    ("lime", &[0, 255, 0]),
    ("magenta", &[255, 0, 255]),
    ("maroon", &[128, 0, 0]),
    ("navy", &[0, 0, 128]),
    ("olive", &[128, 128, 0]),
    ("purple", &[128, 0, 128]),
    ("red", &[255, 0, 0]),
    ("silver", &[192, 192, 192]),
    ("teal", &[0, 128, 128]),
    ("white", &[255, 255, 255]),
    ("yellow", &[255, 255, 0]),
    ("transparent", &[0, 0, 0, 0]),
    ("none", &[0, 0, 0, 0]),
];

fn colour_const(compiler: &Compiler, expr: &Expression) -> Option<Vec<u8>> {
    let text = match super::fold::value(compiler, expr)? {
        super::fold::Constant::Text(t) => t,
        super::fold::Constant::Null => return Some(vec![255, 255, 255]),
        _ => return None,
    };
    let text = std::str::from_utf8(&text).ok()?;
    let Some(hex) = text.strip_prefix('#') else {
        return NAMED_COLOURS
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(text))
            .map(|(_, channels)| channels.to_vec());
    };
    if !hex.is_ascii() {
        return None;
    }
    match hex.len() {
        3 | 4 => hex
            .chars()
            .map(|c| Some(c.to_digit(16)? as u8 * 17))
            .collect(),
        6 | 8 => (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
            .collect(),
        _ => None,
    }
}

fn rgb2num_space(compiler: &Compiler, args: &[Expression]) -> Option<u32> {
    match args {
        [_] => Some(0),
        [_, space] => match super::fold::value(compiler, space)? {
            super::fold::Constant::Number(n) if (1.0..4.0).contains(&n) => Some(n as u32),
            super::fold::Constant::Number(_)
            | super::fold::Constant::Null
            | super::fold::Constant::Text(_) => Some(0),
            _ => None,
        },
        _ => None,
    }
}

fn rgb2num_const(compiler: &Compiler, args: &[Expression]) -> Option<Vec<u8>> {
    if rgb2num_space(compiler, args)? != 0 {
        return None;
    }
    colour_const(compiler, &args[0])
}

pub(super) fn rgb2num_is_constant(compiler: &Compiler, args: &[Expression]) -> bool {
    rgb2num_space(compiler, args).is_some() && colour_const(compiler, &args[0]).is_some()
}
