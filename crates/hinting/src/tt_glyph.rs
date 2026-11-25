//! Glyph programs: a layer's TrueType instructions compiled to `glyf` bytecode against the
//! exported point numbers and the `cvt ` layout, calling the `fpgm` functions of
//! [`crate::tt_font`].
//!
//! Rows: `hinting.tt.snap.*`, `hinting.tt.stem.*` (bytecode), `hinting.tt.shift.rounding`,
//! `hinting.tt.interpolate.rounding`, `hinting.tt.iup`, `hinting.tt.rounding.modes`,
//! `hinting.tt.delta.restrictions`, `Ignore Vertical Hints`, `TTFDontPreserveDiagonals`.
//! Written from the TrueType instruction set specification and the handbook's description of
//! each instruction; nothing is taken from the reference's output.
//!
//! The program runs the instructions in list order, setting both vectors to the instruction's
//! axis (`SVTCA`) whenever the direction changes:
//!
//! | Instruction | Code |
//! |---|---|
//! | Snap, in a zone (vertical; Auto, or the named zone when the point lies in it) | [`FN_ZONE_SNAP`] |
//! | Snap, named zone the point lies outside | [`FN_ZONE_DISTANCE`] |
//! | Snap, no zone or horizontal | [`FN_POINT_ROUND`] with the rounding mode (No rounding touches the point) |
//! | Stem | an untouched origin first: [`FN_ZONE_SNAP`] in a zone, else rounded; then [`FN_STEM`] |
//! | Shift | `SRP1`, `SHP[1]`, then [`FN_POINT_ROUND`] unless No rounding |
//! | Interpolate | `SRP1`, `SRP2`, `IP`, then [`FN_POINT_ROUND`] unless No rounding |
//! | Delta | [`FN_POINT_DELTA`] per cell of the delta column, at its place in the list |
//! | the third Stem of a triple hint | [`FN_TRIPLE`] on the three stems sorted by position |
//!
//! Then `IUP` for each direction that has instructions. Deltas run before `IUP`, since a
//! rasterizer in backward-compatibility mode ignores moves after both `IUP`s.
//!
//! Diagonals: unless TTFDontPreserveDiagonals is set, a point moved by an instruction whose
//! contour neighbours are one flat line (across the direction of movement) and one slanted
//! straight line moves along the slanted line (`SFVTL`), so the slant keeps its angle where a
//! hinted stem crosses it; the freedom vector goes back to the axis after the move.
//!
//! Ignore Vertical Hints leaves out the horizontal Stem instructions that use a vertical TT
//! stem (named, or chosen by Auto).

use std::collections::BTreeMap;
use std::fmt::Write;

use fontcraft_ttasm::{FunctionTable, MAX_GLYPH_PROGRAM_BYTES, MAX_PROGRAM_TEXT, ProgramLimits, analyse, assemble};

use crate::replay::describe;
use crate::tt_font::{
    FN_POINT_DELTA, FN_POINT_ROUND, FN_STEM, FN_TRIPLE, FN_ZONE_DISTANCE, FN_ZONE_SNAP, S_GDI, STEM_NO_ROUNDING, STEM_ROUND, stem_mode,
};
use crate::{
    CvtLayout, DeltaDir, Direction, HintSettings, HintingError, MAX_DELTA_PPM, MAX_LAYER_HINTS, PointMap, Rounding, StemRef, TouchedState,
    TtInstruction, TtKind, TtPoint, ZoneRef,
};

/// A neighbour segment at most this steep across the direction of movement (as a fraction of
/// its length) counts as flat: about one degree.
const FLAT_SLOPE: f64 = 0.02;
/// A slanted neighbour must run at least this much along the direction of movement (as a
/// fraction of its length), which bounds the sideways move to four times the hinted one.
const MIN_DIAGONAL_RISE: f64 = 0.25;

/// One point of the compiled outline, in font units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OutlinePoint {
    pub x: f64,
    pub y: f64,
    pub on_curve: bool,
}

/// The compiled outline of a glyph, as `glyf` stores it.
#[derive(Clone, Copy, Debug, Default)]
pub struct GlyphOutline<'a> {
    /// The points in `glyf` order, phantom points not included.
    pub points: &'a [OutlinePoint],
    /// The index of each contour's last point (`endPtsOfContours`).
    pub contour_ends: &'a [u16],
    /// The advance width: the x of the right sidebearing phantom point.
    pub advance: f64,
}

/// Everything one glyph program is compiled from.
pub struct GlyphContext<'a> {
    pub glyph: &'a str,
    /// The main master's instructions in execution order ([`crate::tt_instructions`]).
    pub instructions: &'a [TtInstruction],
    /// Model node references to exported point numbers, for this outline.
    pub points: &'a dyn PointMap,
    pub outline: GlyphOutline<'a>,
    /// The `cvt ` layout of the master or instance being built ([`crate::FontPrograms::layout`]).
    pub layout: &'a CvtLayout,
    /// The functions of that build's `fpgm`, for the stack analysis.
    pub functions: &'a FunctionTable,
    pub settings: &'a HintSettings,
    /// The TT zones and stems that apply to the glyph ([`HintSettings::tt_zones_for`],
    /// [`HintSettings::tt_stems_for`]); `None`: all of them.
    pub zones: Option<&'a [usize]>,
    pub stems: Option<&'a [usize]>,
    /// The Delta column of the build, as the instructions store it (the instance's
    /// [`crate::tt::delta_column_key`], or the master's); `None`: no Delta applies.
    pub delta_column: Option<&'a str>,
    /// The instructions are the autohinter's and `fpgm` holds its stem function
    /// ([`crate::autohint_fpgm`]): Stem instructions with stem Auto take their width by the stem
    /// width mode.
    pub auto_stems: bool,
}

/// A compiled glyph program.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphProgram {
    /// At most [`MAX_GLYPH_PROGRAM_BYTES`]; empty for a glyph without instructions.
    pub bytes: Vec<u8>,
    /// What the program needs of `maxp`, from [`fontcraft_ttasm::analyse`]; merge with the font
    /// programs' limits.
    pub limits: ProgramLimits,
    /// Instructions left out, and why, for the export report.
    pub warnings: Vec<String>,
}

/// The glyph program of `cx.instructions`. An instruction that names a point the outline does
/// not have is left out with a warning; an outline that does not match its point map, or a
/// program past [`MAX_GLYPH_PROGRAM_BYTES`], is an error.
pub fn glyph_program(cx: &GlyphContext<'_>) -> Result<GlyphProgram, HintingError> {
    if cx.instructions.is_empty() {
        return Ok(GlyphProgram::default());
    }
    if cx.instructions.len() > MAX_LAYER_HINTS {
        return Err(HintingError::LimitExceeded { what: "TrueType instructions in a glyph", limit: MAX_LAYER_HINTS });
    }
    check_outline(cx)?;
    let mut c = Compiler { cx, text: String::new(), warnings: Vec::new(), touched: TouchedState::default(), axis: None };
    c.run()?;
    let Compiler { text, warnings, .. } = c;
    let bytes = assemble(&text)?;
    if bytes.len() > MAX_GLYPH_PROGRAM_BYTES {
        return Err(glyph_error(
            cx,
            format!("the TrueType instructions compile to {} bytes, more than the {MAX_GLYPH_PROGRAM_BYTES} a glyph program can hold", bytes.len()),
        ));
    }
    let limits = analyse(&bytes, cx.functions)?;
    Ok(GlyphProgram { bytes, limits, warnings })
}

fn glyph_error(cx: &GlyphContext<'_>, message: String) -> HintingError {
    HintingError::Glyph { glyph: cx.glyph.to_string(), message }
}

fn check_outline(cx: &GlyphContext<'_>) -> Result<(), HintingError> {
    let o = &cx.outline;
    let n = o.points.len();
    if n != usize::from(cx.points.point_count()) {
        return Err(glyph_error(cx, format!("the outline has {n} points but its point map counts {}", cx.points.point_count())));
    }
    let mut start = 0usize;
    for &end in o.contour_ends {
        let end = usize::from(end);
        if end < start || end >= n {
            return Err(glyph_error(cx, "the contour ends do not match the outline's points".to_string()));
        }
        start = end + 1;
    }
    if start != n {
        return Err(glyph_error(cx, "the contour ends do not match the outline's points".to_string()));
    }
    if !o.advance.is_finite() || o.points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return Err(HintingError::BadNumber { what: format!("a point of `{}`", cx.glyph) });
    }
    Ok(())
}

/// A resolved point: its number and original position.
#[derive(Clone, Copy, Debug)]
struct Pt {
    index: u16,
    x: f64,
    y: f64,
}

impl Pt {
    fn along(&self, d: Direction) -> f64 {
        match d {
            Direction::Vertical => self.y,
            Direction::Horizontal => self.x,
        }
    }
}

/// A Stem of a triple hint once compiled: its lower and upper point along the direction.
#[derive(Clone, Copy, Debug)]
struct Member {
    lo: Pt,
    hi: Pt,
    direction: Direction,
}

struct Compiler<'a, 'b> {
    cx: &'b GlyphContext<'a>,
    text: String,
    warnings: Vec<String>,
    touched: TouchedState,
    /// The axis both vectors are set to.
    axis: Option<Direction>,
}

fn axis_bit(d: Direction) -> u8 {
    match d {
        Direction::Vertical => 0,
        Direction::Horizontal => 1,
    }
}

impl Compiler<'_, '_> {
    fn line(&mut self, s: std::fmt::Arguments<'_>) {
        // `write!` to a String cannot fail.
        let _ = self.text.write_fmt(s);
        self.text.push('\n');
    }

    fn warn(&mut self, index: usize, why: &str) {
        self.warnings.push(format!("`{}`: TrueType instruction {index} is left out: {why}", self.cx.glyph));
    }

    fn run(&mut self) -> Result<(), HintingError> {
        let mut sizes: BTreeMap<u32, usize> = BTreeMap::new();
        for t in self.cx.instructions {
            if let TtKind::Stem { triple: Some(g), .. } = t.kind {
                *sizes.entry(g).or_default() += 1;
            }
        }
        let mut groups: BTreeMap<u32, Vec<Member>> = BTreeMap::new();
        let mut used = [false; 2];
        for (i, t) in self.cx.instructions.iter().enumerate() {
            if self.text.len() > MAX_PROGRAM_TEXT {
                return Err(glyph_error(self.cx, format!("the TrueType instructions need more than {MAX_PROGRAM_TEXT} bytes of program text")));
            }
            let Some(member) = self.instruction(i, t)? else { continue };
            if let Some(slot) = used.get_mut(usize::from(axis_bit(t.direction))) {
                *slot = true;
            }
            if let (TtKind::Stem { triple: Some(g), .. }, Some(m)) = (&t.kind, member) {
                let list = groups.entry(*g).or_default();
                list.push(m);
                if list.len() == 3 && sizes.get(g) == Some(&3) {
                    let list = list.clone();
                    self.triple(&list);
                }
            }
        }
        for (g, n) in &sizes {
            let emitted = groups.get(g).map_or(0, Vec::len);
            if *n != 3 || emitted != 3 {
                self.warnings.push(format!(
                    "`{}`: a triple hint has {emitted} of its Stem instructions compiled, not 3; its stems are kept apart by nothing",
                    self.cx.glyph
                ));
            }
        }
        for d in [Direction::Vertical, Direction::Horizontal] {
            if used.get(usize::from(axis_bit(d))).copied().unwrap_or(false) {
                self.line(format_args!("IUP[{}]", axis_bit(d)));
            }
        }
        Ok(())
    }

    fn resolve(&self, p: &TtPoint) -> Option<Pt> {
        let index = self.cx.points.resolve(p)?;
        let o = &self.cx.outline;
        let n = o.points.len();
        let (x, y) = match usize::from(index) {
            k if k < n => o.points.get(k).map(|q| (q.x, q.y))?,
            k if k == n => (0.0, 0.0),
            k if k == n + 1 => (o.advance, 0.0),
            _ => return None,
        };
        Some(Pt { index, x, y })
    }

    fn set_axis(&mut self, d: Direction) {
        if self.axis != Some(d) {
            self.line(format_args!("SVTCA[{}]", axis_bit(d)));
            self.axis = Some(d);
        }
    }

    /// Compiles one instruction; `Ok(None)` when it is left out. A compiled Stem gives its
    /// points for a triple hint.
    fn instruction(&mut self, i: usize, t: &TtInstruction) -> Result<Option<Option<Member>>, HintingError> {
        let mut pts = Vec::with_capacity(3);
        for p in t.points() {
            match self.resolve(&p) {
                Some(q) => pts.push(q),
                None => {
                    self.warn(i, &format!("{} has no point in the exported outline", describe(&p)));
                    return Ok(None);
                }
            }
        }
        let d = t.direction;
        let needs = match t.kind {
            TtKind::Snap { .. } | TtKind::Delta { .. } => 1,
            TtKind::Stem { .. } | TtKind::Shift => 2,
            TtKind::Interpolate => 3,
        };
        if pts.len() != needs {
            self.warn(i, "it does not name the points its type needs");
            return Ok(None);
        }
        let (Some(&target), first, second) = (pts.last(), pts.first().copied(), pts.get(1).copied()) else { return Ok(None) };
        match &t.kind {
            TtKind::Snap { zone } => {
                self.set_axis(d);
                self.snap(i, target, zone, t.rounding, d);
            }
            TtKind::Stem { stem, .. } => {
                let Some(origin) = first else { return Ok(None) };
                let auto = self.cx.auto_stems && *stem == StemRef::Auto;
                let stem = self.stem_for(i, stem, origin, target, d);
                let vertical_stem = stem.and_then(|k| self.cx.settings.tt_stems.get(k)).is_some_and(|s| !s.horizontal);
                if self.cx.settings.ignore_vertical_hints && d == Direction::Horizontal && vertical_stem {
                    return Ok(None);
                }
                self.set_axis(d);
                if t.origin.is_some_and(|o| !self.touched.is_touched(&o, d)) {
                    match self.zone_at(origin, d) {
                        Some(z) => self.moved(origin, d, |c| c.zone_call(origin, z, FN_ZONE_SNAP)),
                        None => self.moved(origin, d, |c| c.point_round(origin, STEM_ROUND)),
                    }
                }
                let (w, f) = stem.and_then(|k| self.cx.layout.stem(k)).map_or((-1, -1), |(w, f)| (i32::from(w), i32::from(f)));
                let mode = stem_mode(t.rounding);
                self.moved(target, d, |c| {
                    if auto {
                        c.line(format_args!("PUSH {} {} {w} {FN_AUTO_STEM}\nCALL[ ]", origin.index, target.index));
                    } else {
                        c.line(format_args!("PUSH {} {} {w} {f} {mode} {FN_STEM}\nCALL[ ]", origin.index, target.index));
                    }
                });
                self.touched.run(t);
                let (lo, hi) = if origin.along(d) <= target.along(d) { (origin, target) } else { (target, origin) };
                return Ok(Some(Some(Member { lo, hi, direction: d })));
            }
            TtKind::Shift => {
                let Some(origin) = first else { return Ok(None) };
                self.set_axis(d);
                self.moved(target, d, |c| {
                    c.line(format_args!("PUSH {} {}\nSRP1[ ]\nSHP[1]", target.index, origin.index));
                    c.round_after(target, t.rounding);
                });
            }
            TtKind::Interpolate => {
                let (Some(a), Some(b)) = (first, second) else { return Ok(None) };
                self.set_axis(d);
                self.moved(target, d, |c| {
                    c.line(format_args!("PUSH {} {} {}\nSRP1[ ]\nSRP2[ ]\nIP[ ]", target.index, b.index, a.index));
                    c.round_after(target, t.rounding);
                });
            }
            TtKind::Delta { cells } => {
                if d != Direction::Vertical {
                    self.warn(i, "a Delta instruction works in the vertical direction only");
                    return Ok(None);
                }
                let Some(column) = self.cx.delta_column.and_then(|k| cells.get(k)) else { return Ok(Some(None)) };
                self.set_axis(d);
                for (&ppm, dir) in column {
                    if ppm == 0 || ppm > MAX_DELTA_PPM {
                        self.warnings.push(format!("`{}`: a Delta cell at {ppm} PPM is left out (sizes 1 to {MAX_DELTA_PPM})", self.cx.glyph));
                        continue;
                    }
                    let amount = match dir {
                        DeltaDir::Up => 64,
                        DeltaDir::Down => -64,
                    };
                    self.line(format_args!("PUSH {} {ppm} {amount} {FN_POINT_DELTA}\nCALL[ ]", target.index));
                }
            }
        }
        self.touched.run(t);
        Ok(Some(None))
    }

    fn snap(&mut self, i: usize, p: Pt, zone: &ZoneRef, rounding: Rounding, d: Direction) {
        let zone = match (zone, d) {
            (_, Direction::Horizontal) | (ZoneRef::NoZone, _) => None,
            (ZoneRef::Auto, _) => self.zone_at(p, d).map(|z| (z, FN_ZONE_SNAP)),
            (ZoneRef::Index(k), _) if self.cx.layout.zone(*k).is_some() => {
                let inside = self.cx.layout.spans.get(*k).is_some_and(|&(lo, hi)| f64::from(lo) <= p.y && p.y <= f64::from(hi));
                Some((*k, if inside { FN_ZONE_SNAP } else { FN_ZONE_DISTANCE }))
            }
            (ZoneRef::Index(k), _) => {
                self.warnings
                    .push(format!("`{}`: TrueType instruction {i} names TT zone {k}, which the master does not have; it uses Auto", self.cx.glyph));
                self.zone_at(p, d).map(|z| (z, FN_ZONE_SNAP))
            }
        };
        match (zone, rounding) {
            (_, Rounding::NoRounding) | (None, _) => {
                let mode = stem_mode(rounding);
                self.moved(p, d, |c| c.point_round(p, mode));
            }
            (Some((z, call)), Rounding::GdiOnly) => self.moved(p, d, |c| {
                c.line(format_args!("PUSH {S_GDI}\nRS[ ]\nIF[ ]"));
                c.zone_call(p, z, call);
                c.line(format_args!("ELSE[ ]"));
                c.point_round(p, STEM_NO_ROUNDING);
                c.line(format_args!("EIF[ ]"));
            }),
            (Some((z, call)), _) => self.moved(p, d, |c| c.zone_call(p, z, call)),
        }
    }

    fn zone_call(&mut self, p: Pt, z: usize, call: u8) {
        if let Some((pos, fit)) = self.cx.layout.zone(z) {
            self.line(format_args!("PUSH {} {pos} {fit} {call}\nCALL[ ]", p.index));
        }
    }

    fn point_round(&mut self, p: Pt, mode: u8) {
        self.line(format_args!("PUSH {} {mode} {FN_POINT_ROUND}\nCALL[ ]", p.index));
    }

    fn round_after(&mut self, p: Pt, rounding: Rounding) {
        if rounding != Rounding::NoRounding {
            self.point_round(p, stem_mode(rounding));
        }
    }

    /// The zone among the glyph's whose span holds `p`, vertical only.
    fn zone_at(&self, p: Pt, d: Direction) -> Option<usize> {
        if d != Direction::Vertical || usize::from(p.index) >= self.cx.outline.points.len() {
            return None;
        }
        let allowed = |k: usize| self.cx.zones.is_none_or(|z| z.contains(&k));
        self.cx.layout.spans.iter().enumerate().find(|&(k, &(lo, hi))| allowed(k) && f64::from(lo) <= p.y && p.y <= f64::from(hi)).map(|(k, _)| k)
    }

    /// The TT stem a Stem instruction uses: the named one, else the glyph's stem of the
    /// direction closest to the measured distance.
    fn stem_for(&mut self, i: usize, stem: &StemRef, origin: Pt, target: Pt, d: Direction) -> Option<usize> {
        match stem {
            StemRef::NoStem => return None,
            StemRef::Index(k) if self.cx.layout.stem(*k).is_some() => return Some(*k),
            StemRef::Index(k) => {
                self.warnings
                    .push(format!("`{}`: TrueType instruction {i} names TT stem {k}, which the master does not have; it uses Auto", self.cx.glyph));
            }
            StemRef::Auto => {}
        }
        let measured = (target.along(d) - origin.along(d)).abs();
        let wanted = d == Direction::Vertical;
        self.cx
            .settings
            .tt_stems
            .iter()
            .enumerate()
            .filter(|(k, s)| s.horizontal == wanted && s.size.is_finite() && self.cx.stems.is_none_or(|a| a.contains(k)))
            .min_by(|(_, a), (_, b)| (a.size.abs() - measured).abs().total_cmp(&(b.size.abs() - measured).abs()))
            .map(|(k, _)| k)
    }

    /// Emits `body`, which moves `p`, with the freedom vector along `p`'s slanted neighbour when
    /// diagonals are preserved.
    fn moved(&mut self, p: Pt, d: Direction, body: impl FnOnce(&mut Self)) {
        let along = if self.cx.settings.tt_dont_preserve_diagonals { None } else { self.diagonal_neighbour(p.index, d) };
        if let Some(n) = along {
            self.line(format_args!("PUSH {} {n}\nSFVTL[0]", p.index));
        }
        body(self);
        if along.is_some() {
            self.line(format_args!("SFVTCA[{}]", axis_bit(d)));
        }
    }

    /// The neighbour of `index` on its contour along which it should move: exactly one
    /// neighbour flat across `d`, the other a slanted straight line.
    fn diagonal_neighbour(&self, index: u16, d: Direction) -> Option<u16> {
        let o = &self.cx.outline;
        let k = usize::from(index);
        let p = o.points.get(k)?;
        let mut start = 0usize;
        let mut end = None;
        for &e in o.contour_ends {
            let e = usize::from(e);
            if k <= e {
                end = Some(e);
                break;
            }
            start = e + 1;
        }
        let end = end?;
        if end < start + 2 {
            return None;
        }
        let prev = if k == start { end } else { k - 1 };
        let next = if k == end { start } else { k + 1 };
        let shape = |n: usize| -> Option<(bool, bool)> {
            let q = o.points.get(n)?;
            let (dx, dy) = (q.x - p.x, q.y - p.y);
            let len = dx.hypot(dy);
            if len <= f64::EPSILON {
                return None;
            }
            let (rise, run) = match d {
                Direction::Vertical => (dy.abs() / len, dx.abs() / len),
                Direction::Horizontal => (dx.abs() / len, dy.abs() / len),
            };
            Some((rise <= FLAT_SLOPE, q.on_curve && rise >= MIN_DIAGONAL_RISE && run > FLAT_SLOPE))
        };
        let (a, b) = (shape(prev)?, shape(next)?);
        let n = match (a, b) {
            ((true, _), (_, true)) => next,
            ((_, true), (true, _)) => prev,
            _ => return None,
        };
        u16::try_from(n).ok()
    }

    /// Keeps a pixel between the three stems of a triple hint, sorted by position.
    fn triple(&mut self, members: &[Member]) {
        let mut m = members.to_vec();
        let Some(d) = m.first().map(|x| x.direction) else { return };
        if m.iter().any(|x| x.direction != d) {
            self.warnings.push(format!("`{}`: a triple hint mixes directions; its stems are kept apart by nothing", self.cx.glyph));
            return;
        }
        m.sort_by(|a, b| (a.lo.along(d) + a.hi.along(d)).total_cmp(&(b.lo.along(d) + b.hi.along(d))));
        if let [a, b, c] = m.as_slice() {
            self.set_axis(d);
            self.line(format_args!("PUSH {} {} {} {} {} {FN_TRIPLE}\nCALL[ ]", a.hi.index, b.lo.index, b.hi.index, c.lo.index, c.hi.index));
        }
    }
}

#[cfg(test)]
mod tests;
