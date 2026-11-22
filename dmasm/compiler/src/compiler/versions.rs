use super::CompileError;
use crate::{ByondVersion, Instruction};

pub(super) fn require(
    byond: ByondVersion,
    has: fn(ByondVersion) -> bool,
    what: &str,
) -> Result<(), CompileError> {
    if has(byond) {
        Ok(())
    } else {
        Err(CompileError::NotInByond {
            what: what.to_owned(),
            byond,
        })
    }
}

pub(super) fn instruction(byond: ByondVersion, ins: Instruction) -> Instruction {
    if byond.is_516() {
        return ins;
    }
    match ins {
        Instruction::Sin2 => Instruction::Sin,
        Instruction::Cos2 => Instruction::Cos,
        Instruction::Tan2 => Instruction::Tan,
        ins => ins,
    }
}

pub(super) fn nan_test(name: &str, x: f32) -> Option<bool> {
    match name {
        "isnan" => Some(x.is_nan()),
        "isinf" => Some(x.is_infinite()),
        _ => None,
    }
}

pub(super) fn log(byond: ByondVersion, x: f32, base: f32) -> f32 {
    if byond.folds_log_in_single() {
        x.ln() / base.ln()
    } else {
        ((x as f64).ln() / (base as f64).ln()) as f32
    }
}

pub(super) fn degrees(byond: ByondVersion, radians: f64) -> f64 {
    if byond.rounds_radians_to_single() {
        radians as f32 as f64 * 180.0 / std::f64::consts::PI
    } else {
        degrees_on_windows(radians)
    }
}

pub(super) fn degrees_on_windows(radians: f64) -> f64 {
    radians / (std::f64::consts::PI / 180.0)
}

pub(super) const DEFAULT_NAN: f64 = f64::from_bits(0xFFF8_0000_0000_0000);

pub(super) fn sin_cos(byond: ByondVersion, sin: bool, x: f64) -> f64 {
    if !byond.is_516() {
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
    if !byond.is_516() {
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
        "ceil" | "floor" | "round" | "trunc" | "fract" | "isnan" | "isinf" => byond.is_516(),
        _ => true,
    }
}
