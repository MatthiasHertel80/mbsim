//! FPv4-SP floating-point arithmetic with the ARM rules for rounding, flush-to-zero, default
//! NaN, NaN propagation and cumulative exception flags (DDI 0403E A2.5 "Floating-point data
//! types and arithmetic").
//!
//! Results are computed in `f64`, which is exact or carries an exact residual sign, and are then
//! rounded once to single precision by [`round`]. Computing +, -, *, / and sqrt in double and
//! rounding to single gives the correctly rounded result; the residual sign makes the directed
//! rounding modes and the inexact flag exact as well.

pub const FPSCR_N: u32 = 1 << 31;
pub const FPSCR_Z: u32 = 1 << 30;
pub const FPSCR_C: u32 = 1 << 29;
pub const FPSCR_V: u32 = 1 << 28;
pub const FPSCR_AHP: u32 = 1 << 26;
pub const FPSCR_DN: u32 = 1 << 25;
pub const FPSCR_FZ: u32 = 1 << 24;
pub const FPSCR_IDC: u32 = 1 << 7;
pub const FPSCR_IXC: u32 = 1 << 4;
pub const FPSCR_UFC: u32 = 1 << 3;
pub const FPSCR_OFC: u32 = 1 << 2;
pub const FPSCR_DZC: u32 = 1 << 1;
pub const FPSCR_IOC: u32 = 1 << 0;
/// Writable bits of FPSCR on the Cortex-M4.
pub const FPSCR_MASK: u32 = 0xF7C0_009F;

pub const DEFAULT_NAN: u32 = 0x7FC0_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    Nearest,
    PlusInf,
    MinusInf,
    Zero,
}

pub fn rounding_mode(fpscr: u32) -> Rounding {
    match (fpscr >> 22) & 3 {
        0 => Rounding::Nearest,
        1 => Rounding::PlusInf,
        2 => Rounding::MinusInf,
        _ => Rounding::Zero,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Zero,
    Normal,
    Inf,
    QNaN,
    SNaN,
}

#[derive(Clone, Copy, Debug)]
struct Unpacked {
    kind: Kind,
    neg: bool,
    /// Exact value for zero and finite numbers (input denormals flushed if FZ).
    value: f64,
}

fn unpack(x: u32, fpscr: &mut u32) -> Unpacked {
    let neg = x >> 31 != 0;
    let exp = (x >> 23) & 0xFF;
    let frac = x & 0x7F_FFFF;
    if exp == 0 {
        if frac == 0 || *fpscr & FPSCR_FZ != 0 {
            if frac != 0 {
                *fpscr |= FPSCR_IDC;
            }
            return Unpacked { kind: Kind::Zero, neg, value: if neg { -0.0 } else { 0.0 } };
        }
        return Unpacked { kind: Kind::Normal, neg, value: f32::from_bits(x) as f64 };
    }
    if exp == 0xFF {
        if frac == 0 {
            return Unpacked { kind: Kind::Inf, neg, value: if neg { f64::NEG_INFINITY } else { f64::INFINITY } };
        }
        let kind = if frac & 0x40_0000 != 0 { Kind::QNaN } else { Kind::SNaN };
        return Unpacked { kind, neg, value: f64::NAN };
    }
    Unpacked { kind: Kind::Normal, neg, value: f32::from_bits(x) as f64 }
}

fn process_nan(kind: Kind, x: u32, fpscr: &mut u32) -> u32 {
    if kind == Kind::SNaN {
        *fpscr |= FPSCR_IOC;
    }
    if *fpscr & FPSCR_DN != 0 {
        DEFAULT_NAN
    } else {
        x | 0x40_0000
    }
}

fn process_nans(a: (Unpacked, u32), b: (Unpacked, u32), fpscr: &mut u32) -> Option<u32> {
    if a.0.kind == Kind::SNaN {
        Some(process_nan(Kind::SNaN, a.1, fpscr))
    } else if b.0.kind == Kind::SNaN {
        Some(process_nan(Kind::SNaN, b.1, fpscr))
    } else if a.0.kind == Kind::QNaN {
        Some(process_nan(Kind::QNaN, a.1, fpscr))
    } else if b.0.kind == Kind::QNaN {
        Some(process_nan(Kind::QNaN, b.1, fpscr))
    } else {
        None
    }
}

fn process_nans3(a: (Unpacked, u32), b: (Unpacked, u32), c: (Unpacked, u32), fpscr: &mut u32) -> Option<u32> {
    for x in [a, b, c] {
        if x.0.kind == Kind::SNaN {
            return Some(process_nan(Kind::SNaN, x.1, fpscr));
        }
    }
    for x in [a, b, c] {
        if x.0.kind == Kind::QNaN {
            return Some(process_nan(Kind::QNaN, x.1, fpscr));
        }
    }
    None
}

fn invalid(fpscr: &mut u32) -> u32 {
    *fpscr |= FPSCR_IOC;
    DEFAULT_NAN
}

fn zero(neg: bool) -> u32 {
    (neg as u32) << 31
}

fn inf(neg: bool) -> u32 {
    ((neg as u32) << 31) | 0x7F80_0000
}

/// Target format of a rounding.
#[derive(Clone, Copy)]
struct Format {
    frac_bits: i32,
    exp_bits: i32,
}

const SINGLE: Format = Format { frac_bits: 23, exp_bits: 8 };
const HALF: Format = Format { frac_bits: 10, exp_bits: 5 };

/// `FPRound` of the exact value `v + residual * tiny` (residual: sign of the exact error
/// relative to `v`, i.e. -1, 0 or +1) to the format; `v` must be finite and nonzero.
/// With `ahp`, the half-precision alternative format has no infinities.
fn round_fmt(v: f64, residual: i32, fmt: Format, fpscr: &mut u32, ahp: bool) -> u32 {
    let neg = v < 0.0;
    let bits = v.abs().to_bits();
    let exp_field = (bits >> 52) as i32;
    let (m, e) = if exp_field == 0 { (bits & ((1 << 52) - 1), -1074) } else { ((bits & ((1 << 52) - 1)) | (1 << 52), exp_field - 1075) };
    // residual relative to the magnitude
    let r = if neg { -residual } else { residual };
    let m2: u128 = ((m as u128) << 2).wrapping_add(r as i128 as u128);
    let e2 = e - 2;
    let msb = 127 - m2.leading_zeros() as i32;
    let big_e = msb + e2;
    let bias = (1 << (fmt.exp_bits - 1)) - 1;
    let emin = 1 - bias;
    if *fpscr & FPSCR_FZ != 0 && big_e < emin {
        *fpscr |= FPSCR_UFC;
        return zero(neg);
    }
    let lsb_exp = (big_e - fmt.frac_bits).max(emin - fmt.frac_bits);
    let sh = lsb_exp - e2;
    let (mut q, rem, half) = if sh <= 0 {
        (m2 << (-sh) as u32, 0u128, 1u128)
    } else if sh >= 100 {
        // m2 has at most 56 bits: everything is below half an ulp
        (0, m2, u128::MAX)
    } else {
        (m2 >> sh, m2 & ((1u128 << sh) - 1), 1u128 << (sh - 1))
    };
    let inexact = rem != 0;
    let inc = match rounding_mode(*fpscr) {
        Rounding::Nearest => rem > half || (rem == half && q & 1 == 1),
        Rounding::PlusInf => !neg && inexact,
        Rounding::MinusInf => neg && inexact,
        Rounding::Zero => false,
    };
    if inc {
        q += 1;
    }
    let mut lsb = lsb_exp;
    let normal_min = 1u128 << fmt.frac_bits;
    while q >= normal_min << 1 {
        q >>= 1;
        lsb += 1;
    }
    let tiny = big_e < emin;
    if tiny && inexact {
        *fpscr |= FPSCR_UFC;
    }
    let max_exp_field = (1 << fmt.exp_bits) - 1;
    let (biased, frac) = if q >= normal_min { (lsb + fmt.frac_bits + bias, (q - normal_min) as u32) } else { (0, q as u32) };
    let sign_bit = (neg as u32) << (fmt.frac_bits + fmt.exp_bits);
    let max_normal = (((max_exp_field - 1) as u32) << fmt.frac_bits) | ((1 << fmt.frac_bits) - 1);
    if ahp {
        if biased > max_exp_field {
            *fpscr |= FPSCR_IOC;
            return sign_bit | ((max_exp_field as u32) << fmt.frac_bits) | ((1 << fmt.frac_bits) - 1);
        }
    } else if biased >= max_exp_field {
        *fpscr |= FPSCR_OFC | FPSCR_IXC;
        let to_inf = match rounding_mode(*fpscr) {
            Rounding::Nearest => true,
            Rounding::PlusInf => !neg,
            Rounding::MinusInf => neg,
            Rounding::Zero => false,
        };
        return sign_bit | if to_inf { (max_exp_field as u32) << fmt.frac_bits } else { max_normal };
    }
    if inexact {
        *fpscr |= FPSCR_IXC;
    }
    sign_bit | ((biased as u32) << fmt.frac_bits) | frac
}

fn round(v: f64, residual: i32, fpscr: &mut u32) -> u32 {
    round_fmt(v, residual, SINGLE, fpscr, false)
}

fn sign_of(x: f64) -> i32 {
    if x > 0.0 {
        1
    } else if x < 0.0 {
        -1
    } else {
        0
    }
}

/// Rounds an exact sum `s + err` (from TwoSum), handling an exact zero.
fn round_sum(s: f64, err: f64, fpscr: &mut u32) -> u32 {
    if s == 0.0 && err == 0.0 {
        return zero(rounding_mode(*fpscr) == Rounding::MinusInf);
    }
    round(s, sign_of(err), fpscr)
}

fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let bb = s - a;
    let err = (a - (s - bb)) + (b - bb);
    (s, err)
}

pub fn add(a: u32, b: u32, fpscr: &mut u32) -> u32 {
    let ua = unpack(a, fpscr);
    let ub = unpack(b, fpscr);
    if let Some(n) = process_nans((ua, a), (ub, b), fpscr) {
        return n;
    }
    match (ua.kind, ub.kind) {
        (Kind::Inf, Kind::Inf) if ua.neg != ub.neg => invalid(fpscr),
        (Kind::Inf, _) => inf(ua.neg),
        (_, Kind::Inf) => inf(ub.neg),
        (Kind::Zero, Kind::Zero) if ua.neg == ub.neg => zero(ua.neg),
        _ => {
            let (s, err) = two_sum(ua.value, ub.value);
            round_sum(s, err, fpscr)
        }
    }
}

pub fn neg(a: u32) -> u32 {
    a ^ 0x8000_0000
}

pub fn abs(a: u32) -> u32 {
    a & 0x7FFF_FFFF
}

pub fn sub(a: u32, b: u32, fpscr: &mut u32) -> u32 {
    let ua = unpack(a, fpscr);
    let ub = unpack(b, fpscr);
    if let Some(n) = process_nans((ua, a), (ub, b), fpscr) {
        return n;
    }
    add_unpacked_after_nan(ua, Unpacked { neg: !ub.neg, value: -ub.value, ..ub }, fpscr)
}

fn add_unpacked_after_nan(ua: Unpacked, ub: Unpacked, fpscr: &mut u32) -> u32 {
    match (ua.kind, ub.kind) {
        (Kind::Inf, Kind::Inf) if ua.neg != ub.neg => invalid(fpscr),
        (Kind::Inf, _) => inf(ua.neg),
        (_, Kind::Inf) => inf(ub.neg),
        (Kind::Zero, Kind::Zero) if ua.neg == ub.neg => zero(ua.neg),
        _ => {
            let (s, err) = two_sum(ua.value, ub.value);
            round_sum(s, err, fpscr)
        }
    }
}

pub fn mul(a: u32, b: u32, fpscr: &mut u32) -> u32 {
    let ua = unpack(a, fpscr);
    let ub = unpack(b, fpscr);
    if let Some(n) = process_nans((ua, a), (ub, b), fpscr) {
        return n;
    }
    let sign = ua.neg != ub.neg;
    match (ua.kind, ub.kind) {
        (Kind::Inf, Kind::Zero) | (Kind::Zero, Kind::Inf) => invalid(fpscr),
        (Kind::Inf, _) | (_, Kind::Inf) => inf(sign),
        (Kind::Zero, _) | (_, Kind::Zero) => zero(sign),
        _ => round(ua.value * ub.value, 0, fpscr),
    }
}

pub fn div(a: u32, b: u32, fpscr: &mut u32) -> u32 {
    let ua = unpack(a, fpscr);
    let ub = unpack(b, fpscr);
    if let Some(n) = process_nans((ua, a), (ub, b), fpscr) {
        return n;
    }
    let sign = ua.neg != ub.neg;
    match (ua.kind, ub.kind) {
        (Kind::Inf, Kind::Inf) | (Kind::Zero, Kind::Zero) => invalid(fpscr),
        (Kind::Inf, _) => inf(sign),
        (_, Kind::Zero) => {
            *fpscr |= FPSCR_DZC;
            inf(sign)
        }
        (Kind::Zero, _) | (_, Kind::Inf) => zero(sign),
        _ => {
            let q = ua.value / ub.value;
            let rem = (-q).mul_add(ub.value, ua.value);
            round(q, sign_of(rem) * sign_of(ub.value), fpscr)
        }
    }
}

pub fn sqrt(a: u32, fpscr: &mut u32) -> u32 {
    let ua = unpack(a, fpscr);
    match ua.kind {
        Kind::QNaN | Kind::SNaN => process_nan(ua.kind, a, fpscr),
        Kind::Zero => zero(ua.neg),
        Kind::Inf if !ua.neg => inf(false),
        _ if ua.neg => invalid(fpscr),
        _ => {
            let r = ua.value.sqrt();
            let rem = (-r).mul_add(r, ua.value);
            round(r, sign_of(rem), fpscr)
        }
    }
}

/// `FPMulAdd(addend, op1, op2)`: addend + op1 * op2 with one rounding.
pub fn mul_add(addend: u32, op1: u32, op2: u32, fpscr: &mut u32) -> u32 {
    let ua = unpack(addend, fpscr);
    let u1 = unpack(op1, fpscr);
    let u2 = unpack(op2, fpscr);
    if let Some(n) = process_nans3((ua, addend), (u1, op1), (u2, op2), fpscr) {
        return n;
    }
    let inf_prod = u1.kind == Kind::Inf || u2.kind == Kind::Inf;
    let zero_prod = u1.kind == Kind::Zero || u2.kind == Kind::Zero;
    let sign_p = u1.neg != u2.neg;
    if inf_prod && zero_prod {
        return invalid(fpscr);
    }
    if ua.kind == Kind::Inf && inf_prod && ua.neg != sign_p {
        return invalid(fpscr);
    }
    if ua.kind == Kind::Inf {
        return inf(ua.neg);
    }
    if inf_prod {
        return inf(sign_p);
    }
    if ua.kind == Kind::Zero && zero_prod && ua.neg == sign_p {
        return zero(ua.neg);
    }
    let p = u1.value * u2.value; // exact
    let (s, err) = two_sum(ua.value, p);
    round_sum(s, err, fpscr)
}

/// `FPCompare`: returns the NZCV flags in bits 31..28.
pub fn compare(a: u32, b: u32, quiet_nan_exc: bool, fpscr: &mut u32) -> u32 {
    let ua = unpack(a, fpscr);
    let ub = unpack(b, fpscr);
    let nan = |k| k == Kind::QNaN || k == Kind::SNaN;
    if nan(ua.kind) || nan(ub.kind) {
        if ua.kind == Kind::SNaN || ub.kind == Kind::SNaN || quiet_nan_exc {
            *fpscr |= FPSCR_IOC;
        }
        return 0b0011 << 28;
    }
    if ua.value == ub.value {
        0b0110 << 28
    } else if ua.value < ub.value {
        0b1000 << 28
    } else {
        0b0010 << 28
    }
}

/// `FPToFixed`: float to a signed or unsigned integer with `frac` fraction bits, saturating to
/// `bits` (16 or 32). `round_zero` forces round towards zero (VCVT); otherwise FPSCR.RMode.
pub fn to_fixed(a: u32, bits: u32, frac: u32, unsigned: bool, round_zero: bool, fpscr: &mut u32) -> u32 {
    let ua = unpack(a, fpscr);
    if ua.kind == Kind::QNaN || ua.kind == Kind::SNaN {
        *fpscr |= FPSCR_IOC;
        return 0;
    }
    let (min, max): (i64, i64) = if unsigned { (0, (1i64 << bits) - 1) } else { (-(1i64 << (bits - 1)), (1i64 << (bits - 1)) - 1) };
    // the result is zero-extended (unsigned) or sign-extended (signed) to 32 bits
    let to_bits = |v: i64| {
        if bits == 16 && unsigned {
            (v as u32) & 0xFFFF
        } else if bits == 16 {
            v as i16 as i32 as u32
        } else {
            v as u32
        }
    };
    let sat = |v: i64, fpscr: &mut u32| -> u32 {
        *fpscr |= FPSCR_IOC;
        to_bits(v)
    };
    if ua.kind == Kind::Inf {
        return sat(if ua.neg { min } else { max }, fpscr);
    }
    let v = ua.value * (1u64 << frac) as f64; // exact scaling
    let mode = if round_zero { Rounding::Zero } else { rounding_mode(*fpscr) };
    let fl = v.floor();
    let err = v - fl;
    let mut i = fl;
    let up = match mode {
        Rounding::Nearest => err > 0.5 || (err == 0.5 && (fl / 2.0).fract() != 0.0),
        Rounding::PlusInf => err != 0.0,
        Rounding::MinusInf => false,
        Rounding::Zero => err != 0.0 && v < 0.0,
    };
    if up {
        i += 1.0;
    }
    if i > max as f64 {
        return sat(max, fpscr);
    }
    if i < min as f64 {
        return sat(min, fpscr);
    }
    if err != 0.0 {
        *fpscr |= FPSCR_IXC;
    }
    to_bits(i as i64)
}

/// `FixedToFP`: integer with `frac` fraction bits to float, rounded with `round_nearest` or
/// FPSCR.RMode.
pub fn from_fixed(x: u32, bits: u32, frac: u32, unsigned: bool, round_nearest: bool, fpscr: &mut u32) -> u32 {
    let int: i64 = match (bits, unsigned) {
        (16, true) => (x & 0xFFFF) as i64,
        (16, false) => (x as u16 as i16) as i64,
        (_, true) => x as i64,
        (_, false) => x as i32 as i64,
    };
    if int == 0 {
        return 0;
    }
    let v = int as f64 / (1u64 << frac) as f64; // exact
    if round_nearest {
        let saved = *fpscr;
        *fpscr &= !(3 << 22);
        let r = round(v, 0, fpscr);
        *fpscr = (*fpscr & !(3 << 22)) | (saved & (3 << 22));
        r
    } else {
        round(v, 0, fpscr)
    }
}

/// Half precision to single precision (VCVTB/VCVTT to single).
pub fn half_to_single(h: u16, fpscr: &mut u32) -> u32 {
    let neg = h >> 15 != 0;
    let exp = ((h >> 10) & 0x1F) as u32;
    let frac = (h & 0x3FF) as u32;
    let ahp = *fpscr & FPSCR_AHP != 0;
    if exp == 0x1F && !ahp {
        if frac == 0 {
            return inf(neg);
        }
        if frac & 0x200 == 0 {
            *fpscr |= FPSCR_IOC;
        }
        if *fpscr & FPSCR_DN != 0 {
            return DEFAULT_NAN;
        }
        return ((neg as u32) << 31) | 0x7FC0_0000 | (frac << 13);
    }
    if exp == 0 {
        if frac == 0 {
            return zero(neg);
        }
        // half-precision denormals are not flushed (FZ applies to single precision only)
        let v = frac as f64 * 2f64.powi(-24);
        let bits = (v as f32).to_bits();
        return bits | ((neg as u32) << 31);
    }
    let v = (1024 + frac) as f64 * 2f64.powi(exp as i32 - 25);
    (v as f32).to_bits() | ((neg as u32) << 31)
}

/// Single precision to half precision (VCVTB/VCVTT to half).
pub fn single_to_half(a: u32, fpscr: &mut u32) -> u16 {
    let ua = unpack(a, fpscr);
    let ahp = *fpscr & FPSCR_AHP != 0;
    let sign = (ua.neg as u16) << 15;
    match ua.kind {
        Kind::QNaN | Kind::SNaN => {
            if ahp {
                *fpscr |= FPSCR_IOC;
                return sign;
            }
            if ua.kind == Kind::SNaN {
                *fpscr |= FPSCR_IOC;
            }
            if *fpscr & FPSCR_DN != 0 {
                return 0x7E00;
            }
            sign | 0x7E00 | ((a >> 13) & 0x3FF) as u16
        }
        Kind::Inf => {
            if ahp {
                *fpscr |= FPSCR_IOC;
                sign | 0x7FFF
            } else {
                sign | 0x7C00
            }
        }
        Kind::Zero => sign,
        Kind::Normal => {
            // FZ does not apply to the half-precision result
            let saved_fz = *fpscr & FPSCR_FZ;
            *fpscr &= !FPSCR_FZ;
            let r = round_fmt(ua.value, 0, HALF, fpscr, ahp) as u16;
            *fpscr |= saved_fz;
            r
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(x: f32) -> u32 {
        x.to_bits()
    }

    #[test]
    fn basic() {
        let mut s = 0;
        assert_eq!(add(f(1.0), f(2.0), &mut s), f(3.0));
        assert_eq!(s, 0);
        assert_eq!(mul(f(1.5), f(-2.0), &mut s), f(-3.0));
        assert_eq!(div(f(1.0), f(3.0), &mut s), f(1.0 / 3.0));
        assert_eq!(s, FPSCR_IXC);
        let mut s = 0;
        assert_eq!(div(f(1.0), f(0.0), &mut s), f(f32::INFINITY));
        assert_eq!(s, FPSCR_DZC);
        let mut s = 0;
        assert_eq!(sqrt(f(-1.0), &mut s), DEFAULT_NAN);
        assert_eq!(s, FPSCR_IOC);
    }

    #[test]
    fn rounding_matches_native() {
        let mut seed = 0x1234_5678_9ABC_DEF0u64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u32
        };
        for _ in 0..200_000 {
            let a = rnd();
            let b = rnd();
            let (fa, fb) = (f32::from_bits(a), f32::from_bits(b));
            if !fa.is_finite() || !fb.is_finite() {
                continue;
            }
            let mut s = 0;
            let r = add(a, b, &mut s);
            let n = fa + fb;
            if !n.is_nan() {
                assert_eq!(r, n.to_bits(), "{fa:e} + {fb:e}");
            }
            let mut s = 0;
            let r = mul(a, b, &mut s);
            let n = fa * fb;
            if !n.is_nan() {
                assert_eq!(r, n.to_bits(), "{fa:e} * {fb:e}");
            }
            let mut s = 0;
            let r = div(a, b, &mut s);
            let n = fa / fb;
            if !n.is_nan() {
                assert_eq!(r, n.to_bits(), "{fa:e} / {fb:e}");
            }
            let mut s = 0;
            let r = mul_add(a, b, a, &mut s);
            let n = fb.mul_add(fa, fa);
            if !n.is_nan() {
                assert_eq!(r, n.to_bits(), "fma");
            }
        }
    }

    #[test]
    fn flush_to_zero() {
        let mut s = FPSCR_FZ;
        let tiny = f32::from_bits(1);
        assert_eq!(add(f(tiny), f(0.0), &mut s), 0);
        assert_eq!(s & FPSCR_IDC, FPSCR_IDC);
        let mut s = FPSCR_FZ;
        // product below the normal range flushes to zero with UFC
        assert_eq!(mul(f(1e-30), f(1e-10), &mut s), 0);
        assert_eq!(s & FPSCR_UFC, FPSCR_UFC);
    }

    #[test]
    fn directed_rounding() {
        // 1/3 and 2/3 round up in round-to-nearest
        let mut s = 1 << 22; // RP
        assert_eq!(div(f(1.0), f(3.0), &mut s), f(1.0 / 3.0));
        let mut s = 3 << 22; // RZ
        assert_eq!(div(f(1.0), f(3.0), &mut s), f(1.0 / 3.0) - 1);
        let mut s = 2 << 22; // RM
        assert_eq!(div(f(2.0), f(3.0), &mut s), f(2.0 / 3.0) - 1);
        let mut s = 2 << 22; // RM, negative
        assert_eq!(div(f(-2.0), f(3.0), &mut s), f(-2.0 / 3.0));
        let mut s = 1 << 22; // RP, negative
        assert_eq!(div(f(-2.0), f(3.0), &mut s), f(-2.0 / 3.0) - 1);
        let mut s = 3 << 22;
        assert_eq!(add(f(f32::MAX), f(f32::MAX), &mut s), f(f32::MAX));
        assert_eq!(s & (FPSCR_OFC | FPSCR_IXC), FPSCR_OFC | FPSCR_IXC);
        // exact zero sum is -0 in round towards minus infinity
        let mut s = 2 << 22;
        assert_eq!(add(f(1.0), f(-1.0), &mut s), 0x8000_0000);
    }

    #[test]
    fn conversions() {
        let mut s = 0;
        assert_eq!(to_fixed(f(-1.5), 32, 0, false, true, &mut s), (-1i32) as u32);
        assert_eq!(s, FPSCR_IXC);
        let mut s = 0;
        assert_eq!(to_fixed(f(2.5), 32, 0, false, false, &mut s), 2);
        let mut s = 0;
        assert_eq!(to_fixed(f(3e9), 32, 0, false, true, &mut s), 0x7FFF_FFFF);
        assert_eq!(s, FPSCR_IOC);
        let mut s = 0;
        assert_eq!(to_fixed(f(-1.0), 32, 0, true, true, &mut s), 0);
        assert_eq!(s, FPSCR_IOC);
        let mut s = 0;
        assert_eq!(from_fixed(16_777_217, 32, 0, false, false, &mut s), f(16_777_216.0));
        assert_eq!(s, FPSCR_IXC);
        let mut s = 0;
        assert_eq!(single_to_half(f(1.0), &mut s), 0x3C00);
        assert_eq!(half_to_single(0x3C00, &mut s), f(1.0));
        assert_eq!(single_to_half(f(65520.0), &mut s), 0x7C00);
    }

    #[test]
    fn nan_propagation() {
        let mut s = 0;
        let snan = 0x7F80_0001;
        assert_eq!(add(f(1.0), snan, &mut s), 0x7FC0_0001);
        assert_eq!(s, FPSCR_IOC);
        let mut s = FPSCR_DN;
        assert_eq!(add(0xFFC0_1234, f(1.0), &mut s), DEFAULT_NAN);
        let mut s = 0;
        assert_eq!(compare(f(1.0), f(2.0), false, &mut s), 0b1000 << 28);
        assert_eq!(compare(DEFAULT_NAN, f(2.0), true, &mut s), 0b0011 << 28);
        assert_eq!(s, FPSCR_IOC);
    }
}
