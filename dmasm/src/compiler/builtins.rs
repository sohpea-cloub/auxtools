use crate::{ByondVersion, Instruction};

pub(super) struct Builtin {
    pub(super) name: &'static str,
    since: Option<fn(ByondVersion) -> bool>,
    pub(super) code: bool,
    pub(super) value: Value,
}

impl Builtin {
    const fn new(name: &'static str, value: Value) -> Builtin {
        Builtin {
            name,
            since: None,
            code: matches!(value, Value::Code),
            value,
        }
    }

    const fn since(mut self, has: fn(ByondVersion) -> bool) -> Builtin {
        self.since = Some(has);
        self
    }

    const fn with_code(mut self) -> Builtin {
        self.code = true;
        self
    }

    pub(super) fn is_in(&self, byond: ByondVersion) -> bool {
        self.since.is_none_or(|has| has(byond))
    }
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
    ByCount {
        forms: &'static [(usize, usize, Instruction)],
    },
    Test {
        ins: Instruction,
    },
    Code,
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
        Builtin::new(
            stringify!($name),
            Value::Stack {
                params: &[$(param!($($default)?)),*],
                ins: Instruction::$ins,
            },
        )
    };
}

macro_rules! counted {
    ($name:ident($min:literal..) => $ins:ident) => {
        counted!($name($min..=u32::MAX) => $ins)
    };
    ($name:ident($min:literal..=$max:expr) => $ins:ident) => {
        Builtin::new(
            stringify!($name),
            Value::Counted {
                min: $min,
                max: $max,
                ins: Instruction::$ins,
            },
        )
    };
}

macro_rules! by_count {
    (@most $fewest:literal) => {
        $fewest
    };
    (@most $fewest:literal $most:literal) => {
        $most
    };
    ($name:ident: $($fewest:literal $(..=$most:literal)? => $ins:ident $(($operand:literal))?),+) => {
        Builtin::new(
            stringify!($name),
            Value::ByCount {
                forms: &[$((
                    $fewest,
                    by_count!(@most $fewest $($most)?),
                    Instruction::$ins $(($operand))?,
                )),+],
            },
        )
    };
}

macro_rules! test {
    ($name:ident => $ins:ident) => {
        Builtin::new(
            stringify!($name),
            Value::Test {
                ins: Instruction::$ins,
            },
        )
    };
}

macro_rules! code {
    ($name:ident) => {
        Builtin::new(stringify!($name), Value::Code)
    };
}

macro_rules! none {
    ($name:ident) => {
        Builtin::new(stringify!($name), Value::None)
    };
}

#[rustfmt::skip]
static BUILTINS: &[Builtin] = &[
    none!(CRASH),
    code!(REGEX_QUOTE),
    code!(REGEX_QUOTE_REPLACEMENT),
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
    code!(animate),
    stack!(arccos(val) => ArcCos),
    stack!(arcsin(val) => ArcSin),
    by_count!(arctan: 1 => ArcTan, 2 => ArcTan2),
    code!(arglist),
    stack!(ascii2text(val) => Ascii2Text),
    by_count!(astype: 2 => AsType).with_code().since(ByondVersion::is_516),
    by_count!(block: 2 => Block, 3..=6 => BlockXYZ),
    stack!(bound_pixloc(atom, dir) => BoundPixloc).since(ByondVersion::is_516),
    counted!(bounds(0..=5) => Bounds),
    stack!(bounds_dist(ref, target) => BoundsDist),
    none!(browse).with_code(),
    none!(browse_rsc).with_code(),
    stack!(cKey(text) => CKeyEx),
    stack!(ceil(val) => Ceil),
    stack!(ckey(text) => CKey),
    stack!(ckeyEx(text) => CKeyEx),
    stack!(clamp(val, low, high) => Clamp),
    code!(cmptext),
    code!(cmptextEx),
    stack!(copytext(text, start, end = null) => CopyText),
    stack!(copytext_char(text, start, end = null) => CopyTextChar),
    stack!(cos(val) => Cos2),
    stack!(fcopy(src, dst) => FCopy),
    stack!(fcopy_rsc(path) => FCopyRsc),
    stack!(fdel(path) => FDel),
    stack!(fexists(path) => FExists),
    code!(file),
    stack!(file2text(path) => File2Text),
    code!(filter),
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
    code!(generator),
    stack!(get_dir(loc1, loc2) => GetDir),
    stack!(get_dist(loc1, loc2) => GetDist),
    stack!(get_step(ref, dir) => GetStep),
    stack!(get_step_away(ref, dir, max = null) => GetStepAway),
    stack!(get_step_rand(ref) => GetStepRand),
    stack!(get_step_to(ref, target, min = null) => GetStepTo),
    stack!(get_step_towards(ref, target) => GetStepTowards),
    stack!(get_steps_to(ref, target, min = null) => GetStepsTo),
    code!(gradient),
    stack!(hascall(obj, proc_name) => HasCall),
    stack!(hearers(depth = null, center = null) => Hearers),
    stack!(html_decode(text) => HtmlDecode),
    stack!(html_encode(text) => HtmlEncode),
    code!(icon),
    by_count!(icon_states: 1 => IconStates, 2 => IconStatesMode),
    code!(image),
    code!(initial),
    none!(input),
    test!(isarea => IsArea),
    stack!(isfile(val) => IsFile),
    stack!(isicon(val) => IsIcon),
    test!(isinf => IsInf),
    stack!(islist(val) => IsList),
    test!(isloc => IsLoc),
    test!(ismob => IsMob),
    test!(ismovable => IsMovable),
    test!(isnan => IsNaN),
    stack!(isnull(val) => IsNull),
    stack!(isnum(val) => IsNum),
    test!(isobj => IsObj),
    by_count!(ispath: 1 => IsPath, 2 => IsSubPath),
    stack!(ispointer(val) => IsPointer),
    code!(issaved),
    stack!(istext(val) => IsText),
    test!(isturf => IsTurf),
    by_count!(istype: 2 => IsType).with_code(),
    stack!(jointext(list, glue, start = 1, end = null) => JoinText),
    by_count!(json_decode: 1 => JsonDecode, 2 => JsonDecodeFlags(2)),
    by_count!(json_encode: 1 => JsonEncode, 2 => JsonEncodeFlags(2)),
    stack!(length(val) => Length),
    stack!(length_char(val) => LengthChar),
    stack!(lentext(text) => Length),
    stack!(lerp(a, b, t) => Lerp).since(ByondVersion::is_516),
    none!(link),
    none!(list),
    stack!(list2params(list) => List2Params),
    stack!(load_ext(library, function) => LoadExt).since(ByondVersion::is_516),
    none!(load_resource).with_code(),
    none!(locate),
    by_count!(log: 1 => Log10, 2 => Log),
    stack!(lowertext(text) => LowerText),
    counted!(matrix(0..=6) => MatrixNew),
    counted!(max(2..) => Max).with_code(),
    stack!(md5(text) => Md5),
    counted!(min(2..) => Min).with_code(),
    stack!(missile(type, start, end) => Missile),
    code!(nameof),
    code!(newlist),
    counted!(noise_hash(1..) => NoiseHash),
    stack!(nonspantext(haystack, needles, start = 1) => NonSpanText),
    stack!(nonspantext_char(haystack, needles, start = 1) => NonSpanTextChar),
    by_count!(num2text: 1 => Num2Text, 2 => Num2TextSigFigs, 3 => Num2TextRadix),
    counted!(obounds(0..=5) => OBounds),
    stack!(ohearers(depth = null, center = null) => OHearers),
    stack!(orange(dist = null, center = null) => ORange),
    none!(output).with_code(),
    stack!(oview(dist = null, center = null) => OView),
    stack!(oviewers(depth = null, center = null) => OViewers),
    stack!(params2list(params) => Params2List),
    none!(pick),
    code!(pixloc).since(ByondVersion::is_516),
    stack!(prob(val) => Prob),
    by_count!(rand: 0..=1 => Rand, 2 => RandRange),
    none!(rand_seed),
    stack!(range(dist = null, center = null) => Range),
    stack!(ref(val) => Ref),
    stack!(refcount(val) => RefCount),
    counted!(regex(0..) => RegexNew),
    stack!(replacetext(haystack, needle, replacement, start = 1, end = null) => ReplaceText),
    stack!(replacetextEx(haystack, needle, replacement, start = 1, end = null) => ReplaceTextEx),
    stack!(replacetextEx_char(haystack, needle, replacement, start = 1, end = null) => ReplaceTextExChar),
    stack!(replacetext_char(haystack, needle, replacement, start = 1, end = null) => ReplaceTextChar),
    code!(rgb),
    stack!(rgb2num(color, space = 0) => Rgb2Num).with_code(),
    by_count!(roll: 1 => RollStr, 2 => Roll),
    by_count!(round: 0..=1 => Round, 2 => RoundN),
    none!(run),
    stack!(sha1(arg) => Sha1),
    by_count!(shell: 0 => ShellAllowed, 1 => Shell),
    none!(shutdown).with_code(),
    stack!(sign(val) => Sign).since(ByondVersion::is_516),
    stack!(sin(val) => Sin2),
    counted!(sortText(2..) => SortTextEx),
    counted!(sorttext(2..) => SortText),
    counted!(sorttextEx(2..) => SortTextEx),
    code!(sound),
    stack!(spantext(haystack, needles, start = 1) => SpanText),
    stack!(spantext_char(haystack, needles, start = 1) => SpanTextChar),
    stack!(splicetext(text, start, end = null, insert = null) => SpliceText),
    stack!(splicetext_char(text, start, end = null, insert = null) => SpliceTextChar),
    stack!(splittext(text, delimiter, start = 1, end = null, include_delimiters = null) => SplitText),
    stack!(splittext_char(text, delimiter, start = 1, end = null, include_delimiters = null) => SplitTextChar),
    stack!(sqrt(val) => Sqrt),
    counted!(startup(1..) => Startup),
    code!(statpanel),
    code!(step),
    code!(step_away),
    code!(step_rand),
    code!(step_to),
    code!(step_towards),
    stack!(tan(val) => Tan2),
    code!(text),
    stack!(text2ascii(text, pos = null) => Text2Ascii),
    stack!(text2ascii_char(text, pos = null) => Text2AsciiChar),
    stack!(text2file(text, file) => Text2File),
    by_count!(text2num: 1 => Text2Num, 2 => Text2NumRadix),
    stack!(text2path(text) => Text2Path),
    by_count!(time2text: 0..=2 => Time2Text, 3 => Time2TextTZ(3)),
    stack!(trimtext(text) => TrimText),
    stack!(trunc(val) => Trunc),
    stack!(turn(var, angle) => Turn),
    counted!(typesof(1..) => TypesOf),
    stack!(uppertext(text) => UpperText),
    stack!(url_decode(text) => UrlDecode),
    stack!(url_encode(text, format = null) => UrlEncode),
    stack!(values_cut_over(list, max, inclusive = null) => ValuesCutOver).since(ByondVersion::is_516),
    stack!(values_cut_under(list, min, inclusive = null) => ValuesCutUnder).since(ByondVersion::is_516),
    stack!(values_dot(a, b) => ValuesDot).since(ByondVersion::is_516),
    stack!(values_product(list) => ValuesProduct).since(ByondVersion::is_516),
    stack!(values_sum(list) => ValuesSum).since(ByondVersion::is_516),
    code!(vector).since(ByondVersion::is_516),
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
