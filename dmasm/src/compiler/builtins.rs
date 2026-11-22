use crate::Instruction;

pub(super) struct Builtin {
    pub(super) name: &'static str,
    pub(super) value: Value,
}

pub(super) enum Value {
    Stack {
        params: &'static [Param],
        ins: Instruction,
    },
    Counted {
        min: u32,
        max: u32,
        ins: fn(u32) -> Instruction,
    },
    None,
}

pub(super) enum Param {
    Required,
    Null,
    Int(i32),
}

pub(super) fn find(name: &str) -> Option<&'static Builtin> {
    BUILTINS
        .binary_search_by(|builtin| builtin.name.cmp(name))
        .ok()
        .map(|at| &BUILTINS[at])
}

macro_rules! param {
    () => {
        Param::Required
    };
    (null) => {
        Param::Null
    };
    ($default:literal) => {
        Param::Int($default)
    };
}

macro_rules! stack {
    ($name:ident($($param:ident $(= $default:tt)?),*) => $ins:ident) => {
        Builtin {
            name: stringify!($name),
            value: Value::Stack {
                params: &[$(param!($($default)?)),*],
                ins: Instruction::$ins,
            },
        }
    };
}

macro_rules! counted {
    ($name:ident($min:literal..) => $ins:ident) => {
        counted!($name($min..=u32::MAX) => $ins)
    };
    ($name:ident($min:literal..=$max:expr) => $ins:ident) => {
        Builtin {
            name: stringify!($name),
            value: Value::Counted {
                min: $min,
                max: $max,
                ins: Instruction::$ins,
            },
        }
    };
}

macro_rules! none {
    ($name:ident) => {
        Builtin {
            name: stringify!($name),
            value: Value::None,
        }
    };
}

#[rustfmt::skip]
static BUILTINS: &[Builtin] = &[
    none!(CRASH),
    stack!(_dm_db_close(a) => DbClose),
    stack!(_dm_db_columns(a, b) => DbColumns),
    stack!(_dm_db_connect(a, b, c, d, e, f) => DbConnect),
    stack!(_dm_db_error_msg(a) => DbErrorMsg),
    stack!(_dm_db_execute(a, b, c, d, e) => DbExecute),
    stack!(_dm_db_is_connected(a) => DbIsConnected),
    stack!(_dm_db_new_con() => DbNewConnection),
    stack!(_dm_db_new_query() => DbNewQuery),
    stack!(_dm_db_next_row(a, b, c) => DbNextRow),
    stack!(_dm_db_quote(a, b) => DbQuote),
    stack!(_dm_db_row_count(a) => DbRowCount),
    stack!(_dm_db_rows_affected(a) => DbRowsAffected),
    stack!(abs(val) => Abs),
    counted!(addtext(2..) => AddText),
    stack!(alert(param_1, param_2 = null, param_3 = null, param_4 = null, param_5 = null, param_6 = null) => Alert),
    stack!(arccos(val) => ArcCos),
    stack!(arcsin(val) => ArcSin),
    stack!(ascii2text(val) => Ascii2Text),
    stack!(bound_pixloc(atom, dir) => BoundPixloc),
    counted!(bounds(0..=5) => Bounds),
    stack!(bounds_dist(ref, target) => BoundsDist),
    none!(browse),
    none!(browse_rsc),
    stack!(cKey(text) => CKeyEx),
    stack!(ceil(val) => Ceil),
    stack!(ckey(text) => CKey),
    stack!(ckeyEx(text) => CKeyEx),
    stack!(clamp(val, low, high) => Clamp),
    stack!(copytext(text, start, end = null) => CopyText),
    stack!(copytext_char(text, start, end = null) => CopyTextChar),
    stack!(cos(val) => Cos2),
    stack!(fcopy(src, dst) => FCopy),
    stack!(fcopy_rsc(path) => FCopyRsc),
    stack!(fdel(path) => FDel),
    stack!(fexists(path) => FExists),
    stack!(file2text(path) => File2Text),
    stack!(findlasttext(haystack, needle, start = 0, end = 1) => FindLastText),
    stack!(findlasttextEx(haystack, needle, start = 0, end = 1) => FindLastTextEx),
    stack!(findlasttextEx_char(haystack, needle, start = 0, end = 1) => FindLastTextExChar),
    stack!(findlasttext_char(haystack, needle, start = 0, end = 1) => FindLastTextChar),
    stack!(findtext(haystack, needle, start = 1, end = null) => FindText),
    stack!(findtextEx(haystack, needle, start = 1, end = null) => FindTextEx),
    stack!(findtextEx_char(haystack, needle, start = 1, end = null) => FindTextExChar),
    stack!(findtext_char(haystack, needle, start = 1, end = null) => FindTextChar),
    stack!(flist(path) => FList),
    stack!(floor(val) => Round),
    stack!(fract(val) => Fract),
    stack!(ftime(file, local = null) => FTime),
    none!(ftp),
    stack!(get_dir(loc1, loc2) => GetDir),
    stack!(get_dist(loc1, loc2) => GetDist),
    stack!(get_step(ref, dir) => GetStep),
    stack!(get_step_away(ref, dir, max = null) => GetStepAway),
    stack!(get_step_rand(ref) => GetStepRand),
    stack!(get_step_to(ref, target, min = null) => GetStepTo),
    stack!(get_step_towards(ref, target) => GetStepTowards),
    stack!(get_steps_to(ref, target, min = null) => GetStepsTo),
    stack!(hascall(obj, proc_name) => HasCall),
    stack!(hearers(depth = null, center = null) => Hearers),
    stack!(html_decode(text) => HtmlDecode),
    stack!(html_encode(text) => HtmlEncode),
    none!(input),
    stack!(isfile(val) => IsFile),
    stack!(isicon(val) => IsIcon),
    stack!(islist(val) => IsList),
    stack!(isnull(val) => IsNull),
    stack!(isnum(val) => IsNum),
    stack!(ispointer(val) => IsPointer),
    stack!(istext(val) => IsText),
    stack!(jointext(list, glue, start = 1, end = null) => JoinText),
    stack!(length(val) => Length),
    stack!(length_char(val) => LengthChar),
    stack!(lentext(text) => Length),
    stack!(lerp(a, b, t) => Lerp),
    none!(link),
    none!(list),
    stack!(list2params(list) => List2Params),
    stack!(load_ext(library, function) => LoadExt),
    none!(load_resource),
    none!(locate),
    stack!(lowertext(text) => LowerText),
    counted!(matrix(0..=6) => MatrixNew),
    counted!(max(2..) => Max),
    stack!(md5(text) => Md5),
    counted!(min(2..) => Min),
    stack!(missile(type, start, end) => Missile),
    counted!(noise_hash(1..) => NoiseHash),
    stack!(nonspantext(haystack, needles, start = 1) => NonSpanText),
    stack!(nonspantext_char(haystack, needles, start = 1) => NonSpanTextChar),
    counted!(obounds(0..=5) => OBounds),
    stack!(ohearers(depth = null, center = null) => OHearers),
    stack!(orange(dist = null, center = null) => ORange),
    none!(output),
    stack!(oview(dist = null, center = null) => OView),
    stack!(oviewers(depth = null, center = null) => OViewers),
    stack!(params2list(params) => Params2List),
    none!(pick),
    stack!(prob(val) => Prob),
    none!(rand_seed),
    stack!(range(dist = null, center = null) => Range),
    stack!(ref(val) => Ref),
    stack!(refcount(val) => RefCount),
    counted!(regex(0..) => RegexNew),
    stack!(replacetext(haystack, needle, replacement, start = 1, end = null) => ReplaceText),
    stack!(replacetextEx(haystack, needle, replacement, start = 1, end = null) => ReplaceTextEx),
    stack!(replacetextEx_char(haystack, needle, replacement, start = 1, end = null) => ReplaceTextExChar),
    stack!(replacetext_char(haystack, needle, replacement, start = 1, end = null) => ReplaceTextChar),
    stack!(rgb2num(color, space = 0) => Rgb2Num),
    none!(run),
    stack!(sha1(arg) => Sha1),
    none!(shutdown),
    stack!(sign(val) => Sign),
    stack!(sin(val) => Sin2),
    counted!(sortText(2..) => SortTextEx),
    counted!(sorttext(2..) => SortText),
    counted!(sorttextEx(2..) => SortTextEx),
    stack!(spantext(haystack, needles, start = 1) => SpanText),
    stack!(spantext_char(haystack, needles, start = 1) => SpanTextChar),
    stack!(splicetext(text, start, end = null, insert = null) => SpliceText),
    stack!(splicetext_char(text, start, end = null, insert = null) => SpliceTextChar),
    stack!(splittext(text, delimiter, start = 1, end = null, include_delimiters = null) => SplitText),
    stack!(splittext_char(text, delimiter, start = 1, end = null, include_delimiters = null) => SplitTextChar),
    stack!(sqrt(val) => Sqrt),
    counted!(startup(1..) => Startup),
    stack!(tan(val) => Tan2),
    stack!(text2ascii(text, pos = null) => Text2Ascii),
    stack!(text2ascii_char(text, pos = null) => Text2AsciiChar),
    stack!(text2file(text, file) => Text2File),
    stack!(text2path(text) => Text2Path),
    stack!(trimtext(text) => TrimText),
    stack!(trunc(val) => Trunc),
    stack!(turn(var, angle) => Turn),
    counted!(typesof(1..) => TypesOf),
    stack!(uppertext(text) => UpperText),
    stack!(url_decode(text) => UrlDecode),
    stack!(url_encode(text, format = null) => UrlEncode),
    stack!(values_cut_over(list, max, inclusive = null) => ValuesCutOver),
    stack!(values_cut_under(list, min, inclusive = null) => ValuesCutUnder),
    stack!(values_dot(a, b) => ValuesDot),
    stack!(values_product(list) => ValuesProduct),
    stack!(values_sum(list) => ValuesSum),
    stack!(view(dist = null, center = null) => View),
    stack!(viewers(depth = null, center = null) => Viewers),
    stack!(winexists(player, control_id) => WinExists),
    stack!(winget(player, control_id, params) => WinGet),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_in_order_and_finds_every_row() {
        for pair in BUILTINS.windows(2) {
            assert!(
                pair[0].name < pair[1].name,
                "{} is not before {}",
                pair[0].name,
                pair[1].name
            );
        }
        for builtin in BUILTINS {
            assert_eq!(
                find(builtin.name).map(|found| found.name),
                Some(builtin.name)
            );
        }
        assert!(find("no_such_proc").is_none());
    }
}
