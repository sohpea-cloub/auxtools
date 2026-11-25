use super::CompileError;
use crate::{ByondVersion, Instruction};

pub(super) const BYOND_516: u32 = 1648;

const NEW_IN_516: &[&str] = &[
    "astype",
    "bound_pixloc",
    "lerp",
    "load_ext",
    "sign",
    "values_cut_over",
    "values_cut_under",
    "values_dot",
    "values_product",
    "values_sum",
    "vector",
    "pixloc",
];

pub(super) const SAFE_ELEMENT_CACHE: u32 = 1685;

pub(super) const SAFE_LOGICAL_ASSIGN: u32 = 1673;

pub(super) const POINTER_INDEX_DROPPED: u32 = 1668;
pub(super) const PICK_ZERO_WEIGHT_KEPT: u32 = 1689;

pub(super) const PROC_MACRO: u32 = 1644;

pub(super) fn has_builtin(byond: ByondVersion, name: &str) -> bool {
    byond.at_least(BYOND_516) || !NEW_IN_516.contains(&name)
}

pub(super) fn require(byond: ByondVersion, first: u32, what: &str) -> Result<(), CompileError> {
    if byond.at_least(first) {
        Ok(())
    } else {
        Err(CompileError::NotInByond {
            what: what.to_owned(),
            byond,
        })
    }
}

pub(super) fn instruction(byond: ByondVersion, ins: Instruction) -> Instruction {
    if byond.at_least(BYOND_516) {
        return ins;
    }
    match ins {
        Instruction::Sin2 => Instruction::Sin,
        Instruction::Cos2 => Instruction::Cos,
        Instruction::Tan2 => Instruction::Tan,
        ins => ins,
    }
}

pub(super) fn clean_nan_fold(byond: ByondVersion) -> bool {
    byond.build == 1670 || byond.at_least(1672)
}

pub(super) fn nan_test(name: &str, x: f32) -> Option<bool> {
    match name {
        "isnan" => Some(x.is_nan()),
        "isinf" => Some(x.is_infinite()),
        _ => None,
    }
}

pub(super) const SINGLE_LOG: u32 = 1644;

pub(super) fn log(byond: ByondVersion, x: f32, base: f32) -> f32 {
    if byond.at_least(SINGLE_LOG) {
        x.ln() / base.ln()
    } else {
        ((x as f64).ln() / (base as f64).ln()) as f32
    }
}

pub(super) const SINGLE_RADIANS: u32 = 1644;

pub(super) fn degrees(byond: ByondVersion, radians: f64) -> f64 {
    if byond.at_least(SINGLE_RADIANS) {
        radians as f32 as f64 * 180.0 / std::f64::consts::PI
    } else {
        degrees_on_windows(radians)
    }
}

pub(super) fn degrees_on_windows(radians: f64) -> f64 {
    radians / (std::f64::consts::PI / 180.0)
}

pub(super) const LINUX_COLOURS: u32 = 1647;

pub(super) const DEFAULT_NAN: f64 = f64::from_bits(0xFFF8_0000_0000_0000);

pub(super) fn sin_cos(byond: ByondVersion, sin: bool, x: f64) -> f64 {
    if !byond.at_least(BYOND_516) {
        if x.is_infinite() {
            return DEFAULT_NAN;
        }
        let r = x.to_radians();
        return if sin { r.sin() } else { r.cos() };
    }
    if !x.is_finite() {
        let nan = if x.is_nan() { x } else { DEFAULT_NAN };
        return match (sin, x < 0.0) {
            (true, true) => -nan,
            (true, false) => nan,
            (false, _) if x.is_nan() => nan.abs(),
            (false, _) => nan,
        };
    }
    let mut a = x % 360.0;
    if a < 0.0 {
        a += 360.0;
    }
    let quarter = (a / 90.0).floor();
    let r = ((a - 90.0 * quarter) * (std::f64::consts::PI / 180.0)).sin_cos();
    let (s, c) = (r.0, r.1);
    match (quarter as u32 % 4, sin) {
        (0, true) => s,
        (0, false) => c,
        (1, true) => c,
        (1, false) => -s,
        (2, true) => -s,
        (2, false) => -c,
        (_, true) => -c,
        (_, false) => s,
    }
}

pub(super) fn tan(byond: ByondVersion, x: f32) -> f32 {
    if !byond.at_least(BYOND_516) {
        return (x as f64).to_radians().tan() as f32;
    }
    let mut a = x as f64 % 360.0;
    if a > 180.0 {
        a -= 360.0;
    } else if a <= -180.0 {
        a += 360.0;
    }
    let negative = a < 0.0;
    let a = a.abs();
    let quarter = (a / 90.0).floor();
    let mut rest = a - 90.0 * quarter;
    let mut inverse = false;
    if rest > 45.0 {
        rest = 90.0 - rest;
        inverse = true;
    }
    let mut t = rest.to_radians().tan() as f32;
    let odd = quarter as u32 % 2 == 1;
    if odd != inverse {
        t = 1.0 / t;
    }
    if odd != negative {
        t = -t;
    }
    t
}

pub(super) fn folds(byond: ByondVersion, name: &str) -> bool {
    match name {
        "ceil" | "floor" | "round" | "trunc" | "fract" | "isnan" | "isinf" => {
            byond.at_least(BYOND_516)
        }
        _ => true,
    }
}
