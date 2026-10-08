//! Type 2 charstrings from cubic outlines.
//!
//! The pen part is a port of fontTools `pens/t2CharStringPen.py` (`T2CharStringPen`; MIT,
//! Copyright (c) 2009 Type Supply LLC, author Tal Leming): every point is rounded with the
//! tolerance rule, then stored relative to the previous rounded point; a contour's close adds
//! nothing. The width goes first when the glyph has one, `endchar` last. The bounds walk reads the
//! finished operators back the way fontTools' `T2OutlineExtractor` draws them.

use crate::bounds::{Bounds, BoundsPen};
use crate::number::{encode_t2_number, round_with_tolerance};
use crate::specialize::{Cat, Cmd, Op, RLINETO, RMOVETO, RRCURVETO, specialize};
use crate::{Error, PathOp};

/// The Type 2 operand stack limit.
pub(crate) const MAX_STACK: usize = 48;

/// One glyph's charstring and the bounds of what it draws.
pub(crate) struct Built {
    pub bytes: Vec<u8>,
    pub bounds: Option<Bounds>,
}

/// Encode `path` with an optional width argument already relative to `nominalWidthX`.
pub(crate) fn build(glyph: &str, path: &[PathOp], width: Option<f64>, tolerance: f64, specialise: bool) -> Result<Built, Error> {
    let commands = pen_commands(path, tolerance);
    let commands = if specialise { specialize(commands, MAX_STACK) } else { commands };
    let bounds = bounds(&commands);
    let what = format!("glyph `{glyph}`");
    let mut bytes = Vec::new();
    if let Some(w) = width {
        encode_t2_number(w, &what, &mut bytes)?;
    }
    for c in &commands {
        for &a in &c.args {
            encode_t2_number(a, &what, &mut bytes)?;
        }
        bytes.push(opcode(c.op).ok_or_else(|| Error::OutOfRange { what: what.clone(), value: f64::NAN })?);
    }
    bytes.push(14); // endchar
    Ok(Built { bytes, bounds })
}

/// The generic commands a pen records: relative moves, lines and curves between rounded points.
fn pen_commands(path: &[PathOp], tolerance: f64) -> Vec<Cmd> {
    let mut last = (0.0f64, 0.0f64);
    let mut rel = |p: (f64, f64), args: &mut Vec<f64>| {
        let r = (round_with_tolerance(p.0, tolerance), round_with_tolerance(p.1, tolerance));
        args.push(r.0 - last.0);
        args.push(r.1 - last.1);
        last = r;
    };
    path.iter()
        .map(|op| {
            let mut args = Vec::with_capacity(6);
            let op = match *op {
                PathOp::MoveTo(p) => {
                    rel(p, &mut args);
                    RMOVETO
                }
                PathOp::LineTo(p) => {
                    rel(p, &mut args);
                    RLINETO
                }
                PathOp::CurveTo(a, b, c) => {
                    rel(a, &mut args);
                    rel(b, &mut args);
                    rel(c, &mut args);
                    RRCURVETO
                }
            };
            Cmd { op, args }
        })
        .collect()
}

fn opcode(op: Op) -> Option<u8> {
    Some(match op {
        Op::Move(Cat::R) => 21,
        Op::Move(Cat::H) => 22,
        Op::Move(Cat::V) => 4,
        Op::Line(Cat::R) => 5,
        Op::Line(Cat::H) => 6,
        Op::Line(Cat::V) => 7,
        Op::Curve(Cat::R, Cat::R) => 8,
        Op::Curve(Cat::H, Cat::H) => 27,
        Op::Curve(Cat::V, Cat::V) => 26,
        Op::Curve(Cat::H, Cat::V) => 31,
        Op::Curve(Cat::V, Cat::H) => 30,
        Op::RCurveLine => 24,
        Op::RLineCurve => 25,
        _ => return None,
    })
}

/// Draw the finished operators into a bounds pen, accumulating points as the reference's
/// charstring reader does (each point is the previous one plus the delta).
fn bounds(commands: &[Cmd]) -> Option<Bounds> {
    let mut walker = Walker::default();
    for c in commands {
        walker.command(c);
    }
    walker.pen.bounds()
}

#[derive(Default)]
struct Walker {
    pen: BoundsPen,
    current: (f64, f64),
    saw_move: bool,
}

impl Walker {
    fn next(&mut self, d: (f64, f64)) -> (f64, f64) {
        self.current = (self.current.0 + d.0, self.current.1 + d.1);
        self.current
    }

    fn move_to(&mut self, d: (f64, f64)) {
        let p = self.next(d);
        self.pen.move_to(p);
        self.saw_move = true;
    }

    fn line_to(&mut self, d: (f64, f64)) {
        if !self.saw_move {
            self.move_to((0.0, 0.0));
        }
        let p = self.next(d);
        self.pen.line_to(p);
    }

    fn curve_to(&mut self, a: (f64, f64), b: (f64, f64), c: (f64, f64)) {
        if !self.saw_move {
            self.move_to((0.0, 0.0));
        }
        let a = self.next(a);
        let b = self.next(b);
        let c = self.next(c);
        self.pen.curve_to(a, b, c);
    }

    fn command(&mut self, c: &Cmd) {
        let a = |i: usize| c.args.get(i).copied().unwrap_or(0.0);
        let n = c.args.len();
        match c.op {
            Op::Move(Cat::R) => self.move_to((a(0), a(1))),
            Op::Move(Cat::H) => self.move_to((a(0), 0.0)),
            Op::Move(Cat::V) => self.move_to((0.0, a(0))),
            Op::Line(Cat::R) => {
                for i in (0..n.saturating_sub(1)).step_by(2) {
                    self.line_to((a(i), a(i + 1)));
                }
            }
            Op::Line(Cat::H | Cat::V) => {
                let mut horizontal = c.op == Op::Line(Cat::H);
                for i in 0..n {
                    self.line_to(if horizontal { (a(i), 0.0) } else { (0.0, a(i)) });
                    horizontal = !horizontal;
                }
            }
            Op::Curve(Cat::R, Cat::R) => {
                for i in (0..n.saturating_sub(5)).step_by(6) {
                    self.curve_to((a(i), a(i + 1)), (a(i + 2), a(i + 3)), (a(i + 4), a(i + 5)));
                }
            }
            Op::RCurveLine => {
                for i in (0..n.saturating_sub(7)).step_by(6) {
                    self.curve_to((a(i), a(i + 1)), (a(i + 2), a(i + 3)), (a(i + 4), a(i + 5)));
                }
                self.line_to((a(n.saturating_sub(2)), a(n.saturating_sub(1))));
            }
            Op::RLineCurve => {
                let lines = n.saturating_sub(6);
                for i in (0..lines.saturating_sub(1)).step_by(2) {
                    self.line_to((a(i), a(i + 1)));
                }
                let k = lines;
                self.curve_to((a(k), a(k + 1)), (a(k + 2), a(k + 3)), (a(k + 4), a(k + 5)));
            }
            Op::Curve(Cat::V, Cat::V) => {
                let (mut dx1, mut i) = if n % 2 == 1 { (a(0), 1) } else { (0.0, 0) };
                while i + 3 < n {
                    self.curve_to((dx1, a(i)), (a(i + 1), a(i + 2)), (0.0, a(i + 3)));
                    dx1 = 0.0;
                    i += 4;
                }
            }
            Op::Curve(Cat::H, Cat::H) => {
                let (mut dy1, mut i) = if n % 2 == 1 { (a(0), 1) } else { (0.0, 0) };
                while i + 3 < n {
                    self.curve_to((a(i), dy1), (a(i + 1), a(i + 2)), (a(i + 3), 0.0));
                    dy1 = 0.0;
                    i += 4;
                }
            }
            Op::Curve(Cat::H, Cat::V) | Op::Curve(Cat::V, Cat::H) => {
                let mut horizontal = c.op == Op::Curve(Cat::H, Cat::V);
                let mut i = 0;
                while i + 3 < n {
                    let last = if n - (i + 4) == 1 { a(i + 4) } else { 0.0 };
                    if horizontal {
                        self.curve_to((a(i), 0.0), (a(i + 1), a(i + 2)), (last, a(i + 3)));
                    } else {
                        self.curve_to((0.0, a(i)), (a(i + 1), a(i + 2)), (a(i + 3), last));
                    }
                    horizontal = !horizontal;
                    i += 4;
                }
            }
            // Intermediate forms never reach the walker: `specialize` resolves them all.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<PathOp> {
        vec![PathOp::MoveTo((100.0, 0.0)), PathOp::LineTo((400.0, 0.0)), PathOp::LineTo((400.0, 700.0)), PathOp::LineTo((100.0, 700.0))]
    }

    #[test]
    fn a_square_with_and_without_width() {
        let b = build("a", &square(), None, 0.5, true).unwrap();
        // 100 hmoveto 300 700 -300 hlineto endchar
        assert_eq!(b.bytes, vec![239, 22, 247, 192, 249, 80, 251, 192, 6, 14]);
        assert_eq!(b.bounds.unwrap().round_out(), [100, 0, 400, 700]);
        let b = build("a", &square(), Some(-20.0), 0.5, true).unwrap();
        assert_eq!(&b.bytes[..2], &[119, 239]);
    }

    #[test]
    fn unspecialised_output_keeps_generic_operators() {
        let b = build("a", &square(), None, 0.5, false).unwrap();
        // 100 0 rmoveto 300 0 rlineto 0 700 rlineto -300 0 rlineto endchar
        assert_eq!(b.bytes, vec![239, 139, 21, 247, 192, 139, 5, 139, 249, 80, 5, 251, 192, 139, 5, 14]);
    }

    #[test]
    fn coordinates_round_half_up_before_differencing() {
        let path = [PathOp::MoveTo((10.5, -0.5)), PathOp::LineTo((20.4, -0.5))];
        let b = build("a", &path, None, 0.5, false).unwrap();
        // (11, 0) then (20, 0): 11 0 rmoveto 9 0 rlineto
        assert_eq!(b.bytes, vec![150, 139, 21, 148, 139, 5, 14]);
        // With rounding off the fractions survive as 16.16 numbers.
        let b = build("a", &path, None, 0.0, false).unwrap();
        assert_eq!(&b.bytes[..5], &[255, 0, 10, 0x80, 0]);
    }

    #[test]
    fn an_empty_glyph_is_endchar_alone() {
        let b = build("space", &[], Some(100.0), 0.5, true).unwrap();
        assert_eq!(b.bytes, vec![239, 14]);
        assert_eq!(b.bounds, None);
    }

    #[test]
    fn the_walker_reads_back_every_specialised_form() {
        // A glyph whose specialised form uses hv/vh, hh, vv, rcurveline and rlinecurve; its bounds
        // from the specialised operators equal those from the generic ones.
        let path = vec![
            PathOp::MoveTo((0.0, 100.0)),
            PathOp::CurveTo((0.0, 155.0), (45.0, 200.0), (100.0, 200.0)),
            PathOp::CurveTo((155.0, 200.0), (200.0, 155.0), (200.0, 100.0)),
            PathOp::CurveTo((200.0, 45.0), (155.0, 0.0), (100.0, 0.0)),
            PathOp::LineTo((50.0, -20.0)),
            PathOp::MoveTo((300.0, 300.0)),
            PathOp::CurveTo((310.0, 300.0), (330.0, 310.0), (340.0, 310.0)),
            PathOp::CurveTo((350.0, 310.0), (370.0, 310.0), (380.0, 310.0)),
            PathOp::CurveTo((380.0, 320.0), (390.0, 330.0), (390.0, 340.0)),
            PathOp::CurveTo((390.0, 350.0), (390.0, 360.0), (390.0, 370.0)),
            PathOp::LineTo((395.0, 380.0)),
            PathOp::CurveTo((400.0, 390.0), (410.0, 395.0), (420.0, 397.0)),
        ];
        let generic = build("g", &path, None, 0.5, false).unwrap();
        let special = build("g", &path, None, 0.5, true).unwrap();
        assert_eq!(generic.bounds, special.bounds);
        assert!(special.bytes.len() < generic.bytes.len());
    }
}
