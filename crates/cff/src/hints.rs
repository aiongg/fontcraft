//! Charstring hints: stem hints, hint masks, counter masks and flex, as plain data the encoder
//! writes into a Type 2 charstring (`hstem`/`hstemhm`, `vstem`/`vstemhm`, `hintmask`,
//! `cntrmask`, `flex`, `hflex`, `hflex1`, `flex1`; Adobe Technical Note 5177, sections 4.1 to
//! 4.3).
//!
//! What the encoder does with them (`charstring.rs`):
//!
//! | Step | Rule |
//! |---|---|
//! | stems | edges rounded as the outline's coordinates are; a negative width that is not a ghost flipped to a positive one; then sorted by lower edge (masks follow the new order) |
//! | ghosts | width −20: a top edge at `edge`; width −21: a bottom edge at `edge − 21` (the format's reserved widths) |
//! | operators | `hstem`/`vstem` without masks, `hstemhm`/`vstemhm` with hint or counter masks; at most 23 stems per operator so the width and the arguments fit the 48-operand stack, each operator counting from 0 as the format reads it |
//! | masks | `cntrmask` after the stems, then the mask at operation 0 before the first moveto; later masks before their operation, one bit per stem, horizontal stems first, most significant bit first |
//! | outline | specialised between masks and flex pairs, never across them |
//! | flex | the shortest of `hflex`, `hflex1`, `flex1` and `flex` that draws the two curves exactly, flex depth 50 (the value otfautohint writes) |
//!
//! Rows: `hinting.ps.export.stemHintsMadePositive`, `hinting.ps.export.hintReplacement`,
//! `hinting.ps.flex.*`.

use crate::number::round_with_tolerance;
use crate::{Error, MAX_COORDINATE, PathOp};

/// Most stem hints in one charstring, both directions together (Type 2 limit).
pub const MAX_STEM_HINTS: usize = 96;
/// Most counter masks in one charstring. The format names no limit; otfautohint writes at most
/// two (one per direction), so this only bounds hostile input.
pub const MAX_COUNTER_MASKS: usize = 32;
/// Width that marks a ghost hint at a top edge.
pub const TOP_GHOST: f64 = -20.0;
/// Width that marks a ghost hint at a bottom edge.
pub const BOTTOM_GHOST: f64 = -21.0;
/// The flex depth written with every flex (otfautohint's value, from the C autohinter's
/// `gDMin`): flex curves render flat below that many hundredths of a device pixel.
pub(crate) const FLEX_DEPTH: f64 = 50.0;
/// Stems per stem operator: 23 pairs plus a width argument fill 47 of the 48 stack slots.
pub(crate) const STEMS_PER_OPERATOR: usize = 23;

/// One stem hint in absolute font units: the lower (or left) edge and the width. A ghost hint
/// has width [`TOP_GHOST`] with `edge` at the top edge, or [`BOTTOM_GHOST`] with `edge` 21 units
/// above the bottom edge, as the format stores them ([`StemHint::top_ghost`],
/// [`StemHint::bottom_ghost`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StemHint {
    pub edge: f64,
    pub width: f64,
}

impl StemHint {
    /// A stem between two edges, in either order.
    pub fn between(a: f64, b: f64) -> Self {
        if a <= b { Self { edge: a, width: b - a } } else { Self { edge: b, width: a - b } }
    }

    /// A ghost hint at the top edge `y`.
    pub fn top_ghost(y: f64) -> Self {
        Self { edge: y, width: TOP_GHOST }
    }

    /// A ghost hint at the bottom edge `y`.
    pub fn bottom_ghost(y: f64) -> Self {
        Self { edge: y - BOTTOM_GHOST, width: BOTTOM_GHOST }
    }

    pub fn is_ghost(&self) -> bool {
        self.width == TOP_GHOST || self.width == BOTTOM_GHOST
    }

    /// The edges the hint controls, lower first; a ghost's one edge twice.
    pub fn edges(&self) -> (f64, f64) {
        if self.width == TOP_GHOST {
            (self.edge, self.edge)
        } else if self.width == BOTTOM_GHOST {
            (self.edge + BOTTOM_GHOST, self.edge + BOTTOM_GHOST)
        } else if self.width < 0.0 {
            (self.edge + self.width, self.edge)
        } else {
            (self.edge, self.edge + self.width)
        }
    }

    /// Whether two hints of one direction may not be active together: their edge spans share a
    /// point.
    pub fn overlaps(&self, other: &StemHint) -> bool {
        let (a0, a1) = self.edges();
        let (b0, b1) = other.edges();
        a0 <= b1 && b0 <= a1
    }

    /// The hint as the encoder writes it: edges rounded with `tolerance` (the font's
    /// [`crate::Font::round_tolerance`]), a negative stem width made positive.
    pub fn normalised(&self, tolerance: f64) -> Self {
        if self.is_ghost() {
            return Self { edge: round_with_tolerance(self.edge, tolerance), width: self.width };
        }
        let (a, b) = self.edges();
        let (a, b) = (round_with_tolerance(a, tolerance), round_with_tolerance(b, tolerance));
        Self::between(a, b)
    }
}

/// Which stems are on: parallel to [`CharstringHints::h_stems`] and
/// [`CharstringHints::v_stems`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mask {
    pub h: Vec<bool>,
    pub v: Vec<bool>,
}

impl Mask {
    /// The mask bytes: one bit per stem, horizontal stems first, most significant bit first,
    /// padded with zeros to whole bytes.
    pub fn bytes(&self) -> Vec<u8> {
        let bits = self.h.len() + self.v.len();
        let mut out = vec![0u8; bits.div_ceil(8)];
        for (i, on) in self.h.iter().chain(&self.v).enumerate() {
            if *on && let Some(b) = out.get_mut(i / 8) {
                *b |= 0x80 >> (i % 8);
            }
        }
        out
    }
}

/// The hints of one glyph. Masks and flex name path operations by their index in
/// [`crate::Glyph::path`]; the encoder sorts the stems.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CharstringHints {
    pub h_stems: Vec<StemHint>,
    pub v_stems: Vec<StemHint>,
    /// Masks in path order, each taking over before path operation `op` (a `hintmask`). The
    /// first is at op 0 and is written before the first moveto. Empty: no hint replacement
    /// (`hstem`/`vstem`), every stem on throughout.
    pub masks: Vec<(usize, Mask)>,
    /// Counter masks, written after the stems (`cntrmask`).
    pub counters: Vec<Mask>,
    /// Index of the first of two `CurveTo` operations that form a flex.
    pub flex: Vec<usize>,
}

impl CharstringHints {
    pub fn is_empty(&self) -> bool {
        self.h_stems.is_empty() && self.v_stems.is_empty()
    }

    /// Checks the hints against the glyph's `path`: the stem and mask limits, finite numbers,
    /// mask lengths, masks in path order from op 0, no two overlapping stems in one mask, flex on
    /// two curves with no mask between them.
    pub fn validate(&self, glyph: &str, path: &[PathOp]) -> Result<(), Error> {
        let bad = |message: String| Error::BadHints { glyph: glyph.to_string(), message };
        let stems = self.h_stems.len() + self.v_stems.len();
        if stems > MAX_STEM_HINTS {
            return Err(bad(format!("{stems} stem hints, more than {MAX_STEM_HINTS}")));
        }
        if self.counters.len() > MAX_COUNTER_MASKS {
            return Err(bad(format!("{} counter masks, more than {MAX_COUNTER_MASKS}", self.counters.len())));
        }
        if self.h_stems.iter().chain(&self.v_stems).any(|s| !s.edge.is_finite() || !s.width.is_finite()) {
            return Err(bad("a stem hint is not a finite number".into()));
        }
        for m in self.masks.iter().map(|(_, m)| m).chain(&self.counters) {
            if m.h.len() != self.h_stems.len() || m.v.len() != self.v_stems.len() {
                return Err(bad("a mask does not match the stems".into()));
            }
        }
        let ops = path.len().max(1);
        if self.masks.iter().map(|(op, _)| *op).chain(self.flex.iter().copied()).any(|op| op >= ops) {
            return Err(bad("a mask or flex names a path operation the glyph does not have".into()));
        }
        if self.masks.first().is_some_and(|(op, _)| *op != 0) {
            return Err(bad("the first hint mask must apply from the start of the outline".into()));
        }
        if self.masks.windows(2).any(|w| matches!(w, [(a, _), (b, _)] if a >= b)) {
            return Err(bad("hint masks are not in path order".into()));
        }
        for (op, m) in &self.masks {
            if overlapping(&self.h_stems, &m.h) || overlapping(&self.v_stems, &m.v) {
                return Err(bad(format!("the hint mask at path operation {op} holds two overlapping stems")));
            }
        }
        let mut flex = self.flex.clone();
        flex.sort_unstable();
        if flex.windows(2).any(|w| matches!(w, [a, b] if b.saturating_sub(*a) < 2)) {
            return Err(bad("two flex hints share a curve".into()));
        }
        for &f in &flex {
            let curve = |i: usize| matches!(path.get(i), Some(PathOp::CurveTo(..)));
            if !curve(f) || !curve(f + 1) {
                return Err(bad(format!("the flex at path operation {f} is not on two curves")));
            }
            if self.masks.iter().any(|(op, _)| *op == f + 1) {
                return Err(bad(format!("a hint mask falls inside the flex at path operation {f}")));
            }
        }
        Ok(())
    }

    /// The hints as the encoder writes them: stems normalised and sorted, masks reordered with
    /// them, flex sorted.
    pub(crate) fn normalised(&self, tolerance: f64) -> CharstringHints {
        let (h_stems, h_order) = sorted(&self.h_stems, tolerance);
        let (v_stems, v_order) = sorted(&self.v_stems, tolerance);
        let remap = |m: &Mask| Mask {
            h: h_order.iter().map(|&i| m.h.get(i).copied().unwrap_or(false)).collect(),
            v: v_order.iter().map(|&i| m.v.get(i).copied().unwrap_or(false)).collect(),
        };
        let mut flex = self.flex.clone();
        flex.sort_unstable();
        CharstringHints {
            h_stems,
            v_stems,
            masks: self.masks.iter().map(|(op, m)| (*op, remap(m))).collect(),
            counters: self.counters.iter().map(remap).collect(),
            flex,
        }
    }
}

/// Whether two stems that `on` selects overlap.
fn overlapping(stems: &[StemHint], on: &[bool]) -> bool {
    let active: Vec<&StemHint> = stems.iter().zip(on).filter(|(_, on)| **on).map(|(s, _)| s).collect();
    active.iter().enumerate().any(|(i, a)| active.iter().skip(i + 1).any(|b| a.overlaps(b)))
}

/// `stems` normalised and sorted by lower edge, then width; with each new position's old index.
fn sorted(stems: &[StemHint], tolerance: f64) -> (Vec<StemHint>, Vec<usize>) {
    let mut v: Vec<(usize, StemHint)> = stems.iter().map(|s| s.normalised(tolerance)).enumerate().collect();
    v.sort_by(|(_, a), (_, b)| a.edge.total_cmp(&b.edge).then(a.width.total_cmp(&b.width)));
    let order = v.iter().map(|(i, _)| *i).collect();
    (v.into_iter().map(|(_, s)| s).collect(), order)
}

/// Which of `stems`, the hints of one direction, the stem operators can write once rounded with
/// `tolerance`; parallel to `stems`. A stem is not writable when a number is not finite, an edge
/// lies beyond [`MAX_COORDINATE`], or its width or its offset from the stem written before it
/// leaves the range of a Type 2 integer. Stems are taken in the encoder's order, so a stem left
/// out does not count as the one written before the next.
pub fn writable_stems(stems: &[StemHint], tolerance: f64) -> Vec<bool> {
    let fits = |v: f64| (-32_768.0..=32_767.0).contains(&v);
    let (sorted_stems, order) = sorted(stems, tolerance);
    let mut ok = vec![false; stems.len()];
    let (mut written, mut last) = (0usize, 0.0);
    for (s, i) in sorted_stems.iter().zip(order) {
        let (a, b) = s.edges();
        if !(s.edge.is_finite() && s.width.is_finite() && a.abs() <= MAX_COORDINATE && b.abs() <= MAX_COORDINATE) {
            continue;
        }
        let base = if written % STEMS_PER_OPERATOR == 0 { 0.0 } else { last };
        if !fits(s.edge - base) || !fits(s.width) {
            continue;
        }
        if let Some(o) = ok.get_mut(i) {
            *o = true;
        }
        written += 1;
        last = s.edge + s.width;
    }
    ok
}

/// The arguments of the stem operators for `stems` (sorted), split so each operator's fit the
/// stack: each list is relative from 0, a stem's edge after the previous stem's `edge + width`.
pub(crate) fn stem_args(stems: &[StemHint]) -> Vec<Vec<f64>> {
    stems
        .chunks(STEMS_PER_OPERATOR)
        .map(|chunk| {
            let mut last = 0.0;
            let mut args = Vec::with_capacity(chunk.len() * 2);
            for s in chunk {
                args.push(s.edge - last);
                args.push(s.width);
                last = s.edge + s.width;
            }
            args
        })
        .collect()
}

/// The flex operator and its arguments for two curves given as relative vectors (`a` the
/// first curve's three, `b` the second's): the shortest form that draws them exactly.
pub(crate) fn flex_args(a: [(f64, f64); 3], b: [(f64, f64); 3]) -> (u8, Vec<f64>) {
    let [(dx1, dy1), (dx2, dy2), (dx3, dy3)] = a;
    let [(dx4, dy4), (dx5, dy5), (dx6, dy6)] = b;
    if dy1 == 0.0 && dy3 == 0.0 && dy4 == 0.0 && dy6 == 0.0 && dy5 == -dy2 {
        return (34, vec![dx1, dx2, dy2, dx3, dx4, dx5, dx6]);
    }
    if dy3 == 0.0 && dy4 == 0.0 && dy1 + dy2 + dy5 + dy6 == 0.0 {
        return (36, vec![dx1, dy1, dx2, dy2, dx3, dx4, dx5, dy5, dx6]);
    }
    let first_five = vec![dx1, dy1, dx2, dy2, dx3, dy3, dx4, dy4, dx5, dy5];
    let (sx, sy) = (dx1 + dx2 + dx3 + dx4 + dx5, dy1 + dy2 + dy3 + dy4 + dy5);
    if sx.abs() > sy.abs() && sy + dy6 == 0.0 {
        let mut args = first_five;
        args.push(dx6);
        return (37, args);
    }
    if sx.abs() <= sy.abs() && sx + dx6 == 0.0 {
        let mut args = first_five;
        args.push(dy6);
        return (37, args);
    }
    let mut args = first_five;
    args.extend([dx6, dy6, FLEX_DEPTH]);
    (35, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_bytes_are_msb_first_and_padded() {
        let m = Mask { h: vec![true, false, true], v: vec![false, false, false, false, false, true] };
        assert_eq!(m.bytes(), vec![0b1010_0000, 0b1000_0000]);
        assert_eq!(Mask::default().bytes(), Vec::<u8>::new());
    }

    #[test]
    fn ghosts_keep_their_widths_and_stems_turn_positive() {
        let h = [StemHint { edge: 700.4, width: -80.0 }, StemHint::top_ghost(700.0), StemHint::bottom_ghost(0.0)];
        let n = CharstringHints { h_stems: h.to_vec(), ..Default::default() }.normalised(0.5);
        assert_eq!(
            n.h_stems,
            vec![StemHint { edge: 21.0, width: -21.0 }, StemHint { edge: 620.0, width: 80.0 }, StemHint { edge: 700.0, width: -20.0 }]
        );
        assert_eq!(stem_args(&n.h_stems), vec![vec![21.0, -21.0, 620.0, 80.0, 0.0, -20.0]]);
    }

    #[test]
    fn writable_stems_are_judged_in_the_encoders_order() {
        let stems = [
            StemHint::between(100.0, 200.0),
            // More than 32 767 above the end of 32 968..32 980.
            StemHint::between(70_000.0, 70_020.0),
            StemHint::between(-20_000.0, 20_000.0),
            StemHint::between(0.0, 80.0),
            // 32 767 above the end of 100..200, the last stem written before it.
            StemHint::between(32_967.0, 32_970.0),
            StemHint::between(32_968.4, 32_980.0),
            StemHint { edge: f64::NAN, width: 1.0 },
        ];
        assert_eq!(writable_stems(&stems, 0.5), vec![true, false, false, true, true, true, false]);
    }
}
