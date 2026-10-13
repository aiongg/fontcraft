//! A Type 2 charstring scanner for the subsetter: it finds the advance width operand and an
//! `endchar` that builds an accented glyph (`seac`), following subroutine calls. It draws
//! nothing. The `seac` reading follows fontTools' `_ClosureGlyphsT2Decompiler` (MIT).

use crate::limits::{MAX_CFF2_STACK, MAX_SUBR_NESTING};
use crate::{Result, TablesError};

/// Most operators and operands one glyph may execute, subroutine calls included.
pub const MAX_CHARSTRING_OPS: usize = 1 << 20;

/// What a charstring declares.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scan {
    /// The width operand (the advance minus `nominalWidthX`), when the charstring has one.
    pub width: Option<f64>,
    /// The base and accent character codes (Standard Encoding) of a `seac` `endchar`.
    pub seac: Option<(u8, u8)>,
}

struct Scanner<'a> {
    local: &'a [&'a [u8]],
    global: &'a [&'a [u8]],
    stack: Vec<f64>,
    stems: usize,
    seen_first: bool,
    ops: usize,
    out: Scan,
}

/// Scans `program` with its local and global subroutines.
pub fn scan(program: &[u8], local: &[&[u8]], global: &[&[u8]]) -> Result<Scan> {
    let mut s = Scanner { local, global, stack: Vec::new(), stems: 0, seen_first: false, ops: 0, out: Scan::default() };
    s.run(program, 0)?;
    Ok(s.out)
}

fn bias(count: usize) -> i64 {
    if count < 1240 {
        107
    } else if count < 33900 {
        1131
    } else {
        32768
    }
}

fn bad(what: &str) -> TablesError {
    TablesError::invalid(format!("a CFF charstring {what}"))
}

/// How a charstring stopped.
enum Stop {
    Return,
    End,
}

impl Scanner<'_> {
    /// The width is the extra operand of the first stack-clearing operator.
    fn first(&mut self, expected_even: bool) {
        if !self.seen_first {
            self.seen_first = true;
            let even = self.stack.len().is_multiple_of(2);
            let extra = if expected_even { !even } else { even && !self.stack.is_empty() };
            if extra {
                self.out.width = self.stack.first().copied();
                if !self.stack.is_empty() {
                    self.stack.remove(0);
                }
            }
        }
    }

    fn run(&mut self, code: &[u8], depth: usize) -> Result<Stop> {
        if depth > MAX_SUBR_NESTING {
            return Err(bad("nests subroutines too deeply"));
        }
        let mut i = 0usize;
        let byte = |at: usize| code.get(at).copied().ok_or_else(|| bad("is cut short"));
        while i < code.len() {
            self.ops += 1;
            if self.ops > MAX_CHARSTRING_OPS {
                return Err(TablesError::TooLarge { what: "the operations of one CFF charstring", limit: MAX_CHARSTRING_OPS });
            }
            if self.stack.len() > MAX_CFF2_STACK {
                return Err(bad("overflows its stack"));
            }
            let b0 = byte(i)?;
            match b0 {
                32..=246 => {
                    self.stack.push(f64::from(i16::from(b0) - 139));
                    i += 1;
                }
                247..=250 => {
                    let b1 = i16::from(byte(i + 1)?);
                    self.stack.push(f64::from((i16::from(b0) - 247) * 256 + b1 + 108));
                    i += 2;
                }
                251..=254 => {
                    let b1 = i16::from(byte(i + 1)?);
                    self.stack.push(f64::from(-(i16::from(b0) - 251) * 256 - b1 - 108));
                    i += 2;
                }
                28 => {
                    let v = i16::from_be_bytes([byte(i + 1)?, byte(i + 2)?]);
                    self.stack.push(f64::from(v));
                    i += 3;
                }
                255 => {
                    let v = i32::from_be_bytes([byte(i + 1)?, byte(i + 2)?, byte(i + 3)?, byte(i + 4)?]);
                    self.stack.push(f64::from(v) / 65536.0);
                    i += 5;
                }
                1 | 3 | 18 | 23 => {
                    self.first(true);
                    self.stems += self.stack.len() / 2;
                    self.stack.clear();
                    i += 1;
                }
                19 | 20 => {
                    self.first(true);
                    self.stems += self.stack.len() / 2;
                    self.stack.clear();
                    i += 1 + self.stems.div_ceil(8);
                }
                10 | 29 => {
                    let index = self.stack.pop().ok_or_else(|| bad("calls a subroutine without an index"))?;
                    let subrs = if b0 == 10 { self.local } else { self.global };
                    let at = (index as i64).checked_add(bias(subrs.len())).ok_or_else(|| bad("calls a subroutine out of range"))?;
                    let sub = usize::try_from(at).ok().and_then(|at| subrs.get(at)).ok_or_else(|| bad("calls a missing subroutine"))?;
                    i += 1;
                    if let Stop::End = self.run(sub, depth + 1)? {
                        return Ok(Stop::End);
                    }
                }
                11 => return Ok(Stop::Return),
                14 => {
                    self.first(true);
                    if self.stack.len() >= 4 {
                        let code = |v: Option<&f64>| v.map(|v| *v as i64).and_then(|v| u8::try_from(v).ok());
                        let len = self.stack.len();
                        if let (Some(base), Some(accent)) = (code(self.stack.get(len - 2)), code(self.stack.get(len - 1))) {
                            self.out.seac = Some((base, accent));
                        }
                    }
                    return Ok(Stop::End);
                }
                21 => {
                    self.first(true);
                    self.stack.clear();
                    i += 1;
                }
                4 | 22 => {
                    self.first(false);
                    self.stack.clear();
                    i += 1;
                }
                12 => {
                    self.stack.clear();
                    i += 2;
                }
                _ => {
                    self.stack.clear();
                    i += 1;
                }
            }
        }
        Ok(Stop::Return)
    }
}

/// Encodes `v` as a charstring operand: an integer in the shortest form, otherwise a 16.16
/// fixed number.
pub fn encode_number(v: f64) -> Vec<u8> {
    if v.fract() == 0.0 && (-32768.0..=32767.0).contains(&v) {
        let n = v as i32;
        return match n {
            -107..=107 => vec![(n + 139) as u8],
            108..=1131 => {
                let m = n - 108;
                vec![(m / 256 + 247) as u8, (m % 256) as u8]
            }
            -1131..=-108 => {
                let m = -n - 108;
                vec![(m / 256 + 251) as u8, (m % 256) as u8]
            }
            _ => {
                let [a, b] = (n as i16).to_be_bytes();
                vec![28, a, b]
            }
        };
    }
    let fixed = (v * 65536.0).round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    let mut out = vec![255];
    out.extend_from_slice(&fixed.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn width_and_seac_through_a_subroutine() {
        // 50 (width) 0 0 65 194 endchar: width 50, seac of A (65) and acute (194).
        let mut program = encode_number(50.0);
        for v in [0.0, 0.0, 65.0, 194.0] {
            program.extend(encode_number(v));
        }
        program.push(14);
        assert_eq!(scan(&program, &[], &[]).unwrap(), Scan { width: Some(50.0), seac: Some((65, 194)) });

        // The same endchar inside local subroutine 0 (index -107 with the bias of 107).
        let sub = program.clone();
        let call = [encode_number(-107.0), vec![10]].concat();
        assert_eq!(scan(&call, &[&sub], &[]).unwrap().seac, Some((65, 194)));
    }

    #[test]
    fn numbers_round_trip_and_hmoveto_width() {
        for v in [0.0, 107.0, -107.0, 108.0, 1131.0, -1131.0, 2000.0, -32768.0, 0.5] {
            let mut p = encode_number(v);
            p.extend(encode_number(10.0));
            p.push(22); // width hmoveto
            p.push(14);
            assert_eq!(scan(&p, &[], &[]).unwrap().width, Some(v), "{v}");
        }
    }

    #[test]
    fn self_calling_subroutine_is_refused() {
        let sub = [encode_number(-107.0), vec![10]].concat();
        assert!(scan(&sub, &[&sub], &[]).is_err());
    }
}
