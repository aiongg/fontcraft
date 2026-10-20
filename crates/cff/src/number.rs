//! Number encodings: DICT integers and reals, Type 2 charstring integers and 16.16 fixed values,
//! and the rounding the reference uses.
//!
//! Ported from fontTools `misc/psCharStrings.py` (`encodeIntCFF`, `encodeIntT2`, `encodeFixed`,
//! `encodeFloat`) and `misc/roundTools.py` (`otRound`, `roundFunc`); fontTools is MIT-licensed,
//! Copyright (c) 2017 Just van Rossum and contributors. Matching its choices byte for byte is the
//! point: the reference compiler writes CFF through it.

use crate::Error;

/// A DICT value: an integer or a real. The two encode differently even when they are equal (a
/// real `-12.0` is written as a real number, an integer `-12` as an integer), so the caller says
/// which one it means.
#[derive(Clone, Copy, Debug)]
pub enum Number {
    Int(i64),
    Real(f64),
}

impl Number {
    pub(crate) fn as_f64(self) -> f64 {
        match self {
            // Values are checked against the i32 range before they are encoded; this conversion
            // only feeds comparisons and arithmetic.
            Number::Int(i) => i as f64,
            Number::Real(r) => r,
        }
    }

    /// Numeric equality, as the reference compares a value with its default (`0.0 == 0`).
    pub(crate) fn same_value(self, other: Number) -> bool {
        match (self, other) {
            (Number::Int(a), Number::Int(b)) => a == b,
            _ => self.as_f64() == other.as_f64(),
        }
    }

    /// `self - other`, an integer when both are integers.
    pub(crate) fn minus(self, other: Number) -> Number {
        match (self, other) {
            (Number::Int(a), Number::Int(b)) => Number::Int(a.saturating_sub(b)),
            _ => Number::Real(self.as_f64() - other.as_f64()),
        }
    }
}

impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        self.same_value(*other)
    }
}

impl From<i32> for Number {
    fn from(v: i32) -> Self {
        Number::Int(i64::from(v))
    }
}

impl From<f64> for Number {
    fn from(v: f64) -> Self {
        Number::Real(v)
    }
}

/// Round half up (towards +∞), as the OpenType specification and the reference round.
pub(crate) fn ot_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// The reference's coordinate rounding for a tolerance: none at 0, always at 0.5 or more, and
/// otherwise only when the rounded value is within the tolerance.
pub(crate) fn round_with_tolerance(v: f64, tolerance: f64) -> f64 {
    if tolerance == 0.0 {
        v
    } else if tolerance >= 0.5 {
        ot_round(v)
    } else {
        let rounded = ot_round(v);
        if (rounded - v).abs() <= tolerance { rounded } else { v }
    }
}

/// Append a DICT number: an integer in its shortest form, a real spelled with eight significant
/// digits as the reference spells it.
pub fn encode_dict_number(n: Number, what: &str, out: &mut Vec<u8>) -> Result<(), Error> {
    match n {
        Number::Int(i) => encode_dict_int(i, what, out),
        Number::Real(r) => encode_dict_real(r, what, out),
    }
}

/// Append a DICT integer in its shortest form (one, two, three or five bytes).
pub(crate) fn encode_dict_int(v: i64, what: &str, out: &mut Vec<u8>) -> Result<(), Error> {
    if encode_short_int(v, out) {
        return Ok(());
    }
    if let Ok(v) = i16::try_from(v) {
        out.push(28);
        out.extend_from_slice(&v.to_be_bytes());
        return Ok(());
    }
    let v32 = i32::try_from(v).map_err(|_| Error::OutOfRange { what: what.to_string(), value: v as f64 })?;
    out.push(29);
    out.extend_from_slice(&v32.to_be_bytes());
    Ok(())
}

/// The one- and two-byte integer forms shared by DICTs and charstrings; false when `v` needs a
/// longer form.
fn encode_short_int(v: i64, out: &mut Vec<u8>) -> bool {
    let byte = |x: i64| u8::try_from(x).unwrap_or(0);
    if (-107..=107).contains(&v) {
        out.push(byte(v + 139));
    } else if (108..=1131).contains(&v) {
        let w = v - 108;
        out.push(byte((w >> 8) + 247));
        out.push(byte(w & 0xFF));
    } else if (-1131..=-108).contains(&v) {
        let w = -v - 108;
        out.push(byte((w >> 8) + 251));
        out.push(byte(w & 0xFF));
    } else {
        return false;
    }
    true
}

/// Append a Type 2 charstring number: an integer when the value is one in 16.16 fixed point,
/// otherwise the five-byte 16.16 form. The reference writes larger integers with a known-wrong
/// fallback; here they are refused.
pub(crate) fn encode_t2_number(v: f64, what: &str, out: &mut Vec<u8>) -> Result<(), Error> {
    if !v.is_finite() {
        return Err(Error::NotFinite { what: what.to_string() });
    }
    let fixed = ot_round(v * 65536.0);
    if !(-2_147_483_648.0..=2_147_483_647.0).contains(&fixed) {
        return Err(Error::OutOfRange { what: what.to_string(), value: v });
    }
    let fixed = fixed as i64;
    if fixed & 0xFFFF == 0 {
        let int = fixed >> 16;
        if encode_short_int(int, out) {
            return Ok(());
        }
        let short = i16::try_from(int).map_err(|_| Error::OutOfRange { what: what.to_string(), value: v })?;
        out.push(28);
        out.extend_from_slice(&short.to_be_bytes());
        return Ok(());
    }
    let fixed = i32::try_from(fixed).map_err(|_| Error::OutOfRange { what: what.to_string(), value: v })?;
    out.push(255);
    out.extend_from_slice(&fixed.to_be_bytes());
    Ok(())
}

/// Append a DICT real number as packed nibbles, spelled as the reference spells it (eight
/// significant digits, then a few rewrites that shorten the nibble string).
pub(crate) fn encode_dict_real(f: f64, what: &str, out: &mut Vec<u8>) -> Result<(), Error> {
    if !f.is_finite() {
        return Err(Error::NotFinite { what: what.to_string() });
    }
    out.push(30);
    if f == 0.0 {
        out.push(0x0F);
        return Ok(());
    }
    let s = real_spelling(f);
    let mut nibbles: Vec<u8> = Vec::with_capacity(s.len() + 2);
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        let nibble = match c {
            '0'..='9' => c as u8 - b'0',
            '.' => 0xA,
            '-' => 0xE,
            'E' => {
                let n = match chars.peek() {
                    Some('-') => {
                        chars.next();
                        0xC
                    }
                    Some('+') => {
                        chars.next();
                        0xB
                    }
                    _ => 0xB,
                };
                if chars.peek() == Some(&'0') {
                    chars.next();
                }
                n
            }
            // `real_spelling` produces only the characters above.
            _ => return Err(Error::NotFinite { what: what.to_string() }),
        };
        nibbles.push(nibble);
    }
    nibbles.push(0xF);
    if nibbles.len() % 2 == 1 {
        nibbles.push(0xF);
    }
    for pair in nibbles.chunks(2) {
        let hi = pair.first().copied().unwrap_or(0xF);
        let lo = pair.get(1).copied().unwrap_or(0xF);
        out.push((hi << 4) | lo);
    }
    Ok(())
}

/// The string the reference turns into nibbles for a non-zero finite `f`.
fn real_spelling(f: f64) -> String {
    let mut s = format_g8(f);
    if let Some(rest) = s.strip_prefix("0.") {
        s = format!(".{rest}");
    } else if let Some(rest) = s.strip_prefix("-0.") {
        s = format!("-.{rest}");
    } else if s.ends_with("000") {
        let significant = s.trim_end_matches('0').to_string();
        s = format!("{significant}E{}", s.len() - significant.len());
    } else if let (Some(dot), Some(e)) = (s.find('.'), s.find('E')) {
        let integer = s.get(..dot).unwrap_or("").to_string();
        let fraction = s.get(dot + 1..e).unwrap_or("").to_string();
        let exponent: i64 = s.get(e + 1..).and_then(|x| x.parse().ok()).unwrap_or(0);
        let new_exponent = exponent - fraction.len() as i64;
        s = if new_exponent == 1 { format!("{integer}{fraction}0") } else { format!("{integer}{fraction}E{new_exponent}") };
    }
    if (s.starts_with(".0") || s.starts_with("-.0"))
        && let Some((sign, rest)) = s.split_once('.')
    {
        s = format!("{sign}{}E-{}", rest.trim_start_matches('0'), rest.len());
    }
    s
}

/// `f` formatted as C's `%.8G` would format it: eight significant digits, fixed notation for
/// decimal exponents from −4 to 7, scientific otherwise, trailing zeros removed.
pub(crate) fn format_g8(f: f64) -> String {
    let sci = format!("{:.7e}", f.abs());
    let (mantissa, exponent) = sci.split_once('e').unwrap_or((sci.as_str(), "0"));
    let exponent: i64 = exponent.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let sign = if f.is_sign_negative() { "-" } else { "" };
    if !(-4..8).contains(&exponent) {
        let first = digits.get(..1).unwrap_or("0");
        let rest = digits.get(1..).unwrap_or("").trim_end_matches('0');
        let dot = if rest.is_empty() { String::new() } else { format!(".{rest}") };
        let esign = if exponent < 0 { '-' } else { '+' };
        return format!("{sign}{first}{dot}E{esign}{:02}", exponent.abs());
    }
    let (integer, fraction) = if exponent >= 0 {
        let split = (exponent + 1) as usize;
        (digits.get(..split).unwrap_or("").to_string(), digits.get(split..).unwrap_or("").to_string())
    } else {
        ("0".to_string(), format!("{}{digits}", "0".repeat((-exponent - 1) as usize)))
    };
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() { format!("{sign}{integer}") } else { format!("{sign}{integer}.{fraction}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real(f: f64) -> Vec<u8> {
        let mut out = Vec::new();
        encode_dict_real(f, "test", &mut out).unwrap();
        out
    }

    fn dict_int(v: i64) -> Vec<u8> {
        let mut out = Vec::new();
        encode_dict_int(v, "test", &mut out).unwrap();
        out
    }

    fn t2(v: f64) -> Vec<u8> {
        let mut out = Vec::new();
        encode_t2_number(v, "test", &mut out).unwrap();
        out
    }

    #[test]
    fn g8_matches_printf() {
        // Expected strings are what `"%.8G" % value` gives.
        let cases: &[(f64, &str)] = &[
            (0.001, "0.001"),
            (0.00048828125, "0.00048828125"),
            (0.039625, "0.039625"),
            (-12.0, "-12"),
            (100.0, "100"),
            (12_300_000.0, "12300000"),
            (123_456_789.0, "1.2345679E+08"),
            (0.000012, "1.2E-05"),
            (1.0 / 3.0, "0.33333333"),
            (-0.5, "-0.5"),
            (1e100, "1E+100"),
        ];
        for (v, want) in cases {
            assert_eq!(format_g8(*v), *want, "{v}");
        }
    }

    #[test]
    fn dict_reals_match_the_reference() {
        // Expected bytes are fontTools' `encodeFloat` output for the same values.
        assert_eq!(real(0.0), [0x1E, 0x0F]);
        assert_eq!(real(-0.0), [0x1E, 0x0F]);
        // .001 → "1E-3"
        assert_eq!(real(0.001), [0x1E, 0x1C, 0x3F]);
        // -12.0 → "-12"
        assert_eq!(real(-12.0), [0x1E, 0xE1, 0x2F]);
        // 0.039625 → ".039625" → "39625E-6"
        assert_eq!(real(0.039625), [0x1E, 0x39, 0x62, 0x5C, 0x6F]);
        // 1/2048 → ".00048828125" → "48828125E-11"
        assert_eq!(real(1.0 / 2048.0), [0x1E, 0x48, 0x82, 0x81, 0x25, 0xC1, 0x1F]);
        // 12300000 → "123E5"
        assert_eq!(real(12_300_000.0), [0x1E, 0x12, 0x3B, 0x5F]);
        // 1.5 → "1.5"
        assert_eq!(real(1.5), [0x1E, 0x1A, 0x5F]);
        // 123456789 → "1.2345679E+08" → "123456790" (an exponent of 1 becomes a zero)
        assert_eq!(real(123_456_789.0), [0x1E, 0x12, 0x34, 0x56, 0x79, 0x0F]);
        // 15 → "%.8G" gives "15": no dot, no exponent.
        assert_eq!(real(15.0), [0x1E, 0x15, 0xFF]);
        // 0.000012 → "1.2E-05" → "12E-6"
        assert_eq!(real(0.000012), [0x1E, 0x12, 0xC6, 0xFF]);
    }

    #[test]
    fn dict_integers_take_the_shortest_form() {
        assert_eq!(dict_int(0), [139]);
        assert_eq!(dict_int(-107), [32]);
        assert_eq!(dict_int(107), [246]);
        assert_eq!(dict_int(108), [247, 0]);
        assert_eq!(dict_int(1131), [250, 255]);
        assert_eq!(dict_int(-108), [251, 0]);
        assert_eq!(dict_int(-1131), [254, 255]);
        assert_eq!(dict_int(1132), [28, 0x04, 0x6C]);
        assert_eq!(dict_int(-32768), [28, 0x80, 0x00]);
        assert_eq!(dict_int(32768), [29, 0, 0, 0x80, 0]);
        assert_eq!(dict_int(-100_000), [29, 0xFF, 0xFE, 0x79, 0x60]);
        let mut out = Vec::new();
        assert!(encode_dict_int(1 << 40, "x", &mut out).is_err());
    }

    #[test]
    fn charstring_numbers_use_fixed_only_for_fractions() {
        assert_eq!(t2(0.0), [139]);
        assert_eq!(t2(-5.0), [134]);
        assert_eq!(t2(2000.0), [28, 0x07, 0xD0]);
        assert_eq!(t2(0.5), [255, 0, 0, 0x80, 0]);
        assert_eq!(t2(-1.25), [255, 0xFF, 0xFE, 0xC0, 0]);
        // A value that is an integer at 16.16 precision is written as the integer.
        assert_eq!(t2(3.000001), [142]);
        let mut out = Vec::new();
        assert!(encode_t2_number(40_000.0, "x", &mut out).is_err());
        assert!(encode_t2_number(f64::NAN, "x", &mut out).is_err());
    }

    #[test]
    fn rounding_follows_the_tolerance() {
        assert_eq!(ot_round(2.5), 3.0);
        assert_eq!(ot_round(-2.5), -2.0);
        assert_eq!(round_with_tolerance(1.3, 0.5), 1.0);
        assert_eq!(round_with_tolerance(1.3, 0.0), 1.3);
        assert_eq!(round_with_tolerance(1.3, 0.2), 1.3);
        assert_eq!(round_with_tolerance(1.1, 0.2), 1.0);
    }
}
