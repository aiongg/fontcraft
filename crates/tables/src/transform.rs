//! Transforming every glyph and scaling units per em (M12.24; rows `transform`,
//! `font.transform`). Oracles: AFDKO rotatefont, fontTools `ttLib.scaleUpem`.
//!
//! The units-per-em scaling is a port of fontTools' `ttLib/scaleUpem.py` (MIT License,
//! Copyright (c) 2017 Just van Rossum), table by table:
//!
//! | Table | What scales |
//! |---|---|
//! | `glyf` | point coordinates and component offsets, each rounded on its own; bounds worked out again |
//! | `hmtx`, `vmtx` | advances and side bearings |
//! | `head` | units per em; the bounds and `flags` bit 1 (every left side bearing at `xMin`) are worked out again from `glyf` |
//! | `maxp` | the point, contour and component maxima worked out again (`maxp.recalc`) |
//! | `hhea`, `vhea` | line metrics and caret offset; the largest advance and the extremes worked out again |
//! | `OS/2` | the average width, sub- and superscript sizes and offsets, strikeout, typographic and Windows metrics, x-height, cap height |
//! | `post` | underline position and thickness; the rest written again as fontTools writes a loaded `post` (`post::recompile`) |
//! | `VORG` | the vertical origins |
//! | `kern` | format 0 pair values |
//! | `gvar` | every delta |
//! | `GPOS` | value record placements and advances, anchors |
//! | `GDEF`, `BASE` | caret and baseline coordinates |
//! | item variation stores (`GDEF`, `BASE`, `HVAR`, `VVAR`, `MVAR`) | every delta |
//! | `cvt `, `cvar` | every value and delta (not in the oracle; see the row's notes) |
//!
//! Instructions are kept. Values round as fontTools' `otRound` rounds (halves up). Tables in
//! font units this port does not handle yet (`MATH`, `COLR`, the Apple layout tables) are kept
//! as they are with a warning.
//!
//! A matrix transform follows rotatefont: the named parts are applied in the order scale,
//! slant, rotate (clockwise), translate; a matrix `[a, b, c, d, e, f]` maps (x, y) to
//! (a·x + c·y + e, b·x + d·y + f). Composites keep their components: each component's 2×2
//! transform and offset are recomputed so the composite shows its transformed components
//! transformed once. Advance widths follow [`Widths`]. Every changed glyph loses its
//! instructions and gets its left side bearing set to its `xMin`; when no glyph keeps
//! instructions the font programs and `cvt ` go too, and the device metrics caches (`hdmx`,
//! `LTSH`, `VDMX`) go whenever a glyph changed. Whenever a glyph changed, `maxp` and the `head`
//! bounds and flags are recalculated as fontTools' `maxp.recalc` recalculates them on saving;
//! `maxp.maxSizeOfInstructions` keeps its stored value unless glyphs lost their instructions.
//!
//! `CFF ` and `CFF2` outlines take new units per em as scaleUpem gives them ([`cff`]): charstrings
//! and hint values scaled, the table written without subroutines, the `head` box and the `hhea`
//! and `vhea` extremes worked out again from the scaled outlines. A matrix on them follows
//! rotatefont ([`cff_matrix`]): points rounded on their own, hints kept, turned or dropped by the
//! matrix's linear part, the Private DICT hint values changed to match. The advance widths follow
//! [`Widths`] from the `hmtx` advances, and each changed glyph's left side bearing becomes its
//! `xMin`. A matrix on a variable TrueType font is refused for now.

mod cff;
mod cff_matrix;
mod fixed;
mod glyf;
mod layout;
pub(crate) mod post;
mod tuples;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use crate::limits::{MAX_COORDINATE, MAX_UNITS_PER_EM, MIN_UNITS_PER_EM};
use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};
use fixed::{i16_at, put_i16, put_u16, u16_at};
use glyf::{Args, Glyph, SCALED_COMPONENT_OFFSET, Shape, UNSCALED_COMPONENT_OFFSET, coord, ot_round};
use layout::Scaler;

/// What happens to the advance widths of transformed glyphs (rotatefont's `-row`, `-rtw`,
/// `-rew` and its default rule).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Widths {
    /// Transformed (a·w + e) when the matrix's `b` is 0 (no rotation but a half turn, no
    /// vertical shear), else set to the em.
    #[default]
    Auto,
    Keep,
    Transform,
    Em,
}

impl Widths {
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "auto" => Ok(Widths::Auto),
            "keep" => Ok(Widths::Keep),
            "transform" => Ok(Widths::Transform),
            "em" => Ok(Widths::Em),
            other => Err(TablesError::BadOptions(format!("`{other}` is not a widths policy: use auto, keep, transform or em"))),
        }
    }
}

/// What [`transform`] does: a matrix, or the named parts applied in the order scale, slant,
/// rotate, translate; then optionally new units per em.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TransformOptions {
    /// `[a, b, c, d, e, f]` as in PostScript; values finite. Excludes the named parts.
    pub matrix: Option<[f64; 6]>,
    pub scale: Option<(f64, f64)>,
    /// Slant angle in degrees (positive leans right), less than 90 either way.
    pub slant: Option<f64>,
    /// Rotation in degrees, clockwise.
    pub rotate: Option<f64>,
    /// Shift applied after the other parts.
    pub translate: Option<(f64, f64)>,
    /// With a matrix, also multiply units per em by its horizontal factor `a` (rotatefont `-fm`).
    pub scale_em: bool,
    pub widths: Widths,
    /// The glyphs the matrix applies to; `None` for every glyph. Units per em always scale the
    /// whole font.
    pub glyphs: Option<BTreeSet<u16>>,
    /// New units per em, from [`crate::limits::MIN_UNITS_PER_EM`] to
    /// [`crate::limits::MAX_UNITS_PER_EM`]; every metric scales with it.
    pub units_per_em: Option<u16>,
}

/// What [`transform_with_report`] did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TransformReport {
    /// The matrix applied to the selected glyphs, when there was one.
    pub matrix: Option<[f64; 6]>,
    /// Units per em before and after, when they changed.
    pub units_per_em: Option<(u16, u16)>,
    /// No instructions or hinting tables were removed.
    pub hints_kept: bool,
    pub glyphs_changed: usize,
    pub tables_changed: Vec<Tag>,
    pub tables_removed: Vec<Tag>,
    pub warnings: Vec<String>,
}

/// The transformed font. Refuses a result whose `glyf` coordinates pass
/// [`crate::limits::MAX_COORDINATE`].
pub fn transform(font: &SfntFont, options: &TransformOptions) -> Result<SfntFont> {
    transform_with_report(font, options).map(|(f, _)| f)
}

/// [`transform`], with what it did.
pub fn transform_with_report(font: &SfntFont, options: &TransformOptions) -> Result<(SfntFont, TransformReport)> {
    let matrix = options_matrix(options)?;
    if let Some(u) = options.units_per_em {
        check_upem(u)?;
    }
    if options.scale_em && options.units_per_em.is_some() {
        return Err(TablesError::BadOptions("scaling the em with the matrix and setting units per em exclude each other".into()));
    }
    if options.scale_em && matrix.is_none() {
        return Err(TablesError::BadOptions("scaling the em needs a matrix or a scale".into()));
    }
    if matrix.is_none() && options.units_per_em.is_none() {
        return Err(TablesError::BadOptions("nothing to do: give a matrix, a rotation, a shift, a scale, a slant or new units per em".into()));
    }
    let mut work = Work::read(font)?;
    let mut report = TransformReport { matrix, hints_kept: true, ..TransformReport::default() };
    if let Some(m) = matrix {
        if !font.is_cff() && (font.get(*b"fvar").is_some() || font.get(*b"gvar").is_some()) {
            return Err(TablesError::Unsupported("a matrix on a variable font: its deltas would need transforming too".into()));
        }
        work.apply_matrix(m, options, &mut report)?;
        if options.scale_em {
            let target = ot_round(f64::from(work.upem) * m[0].abs());
            let target = u16::try_from(target as i64).map_err(|_| TablesError::BadOptions(format!("units per em would be {target}")))?;
            check_upem(target)?;
            report.units_per_em = Some((work.upem, target));
            work.upem = target;
        }
    }
    if let Some(target) = options.units_per_em
        && target != work.upem
    {
        work.cff_changed = None;
        report.units_per_em = Some((work.upem, target));
        work.apply_upem(target, &mut report)?;
    }
    let out = work.finish(&mut report)?;
    for tag in out.tags() {
        if font.get(tag) != out.get(tag) {
            report.tables_changed.push(tag);
        }
    }
    report.tables_removed = font.tags().into_iter().filter(|t| out.get(*t).is_none()).collect();
    if !report.tables_removed.is_empty() {
        report.hints_kept = false;
    }
    Ok((out, report))
}

fn check_upem(u: u16) -> Result<()> {
    if (MIN_UNITS_PER_EM..=MAX_UNITS_PER_EM).contains(&u) {
        Ok(())
    } else {
        Err(TablesError::BadOptions(format!("units per em must be from {MIN_UNITS_PER_EM} to {MAX_UNITS_PER_EM}, not {u}")))
    }
}

/// Applies `first`, then `then`.
fn compose(first: [f64; 6], then: [f64; 6]) -> [f64; 6] {
    let [a1, b1, c1, d1, e1, f1] = first;
    let [a2, b2, c2, d2, e2, f2] = then;
    [a2 * a1 + c2 * b1, b2 * a1 + d2 * b1, a2 * c1 + c2 * d1, b2 * c1 + d2 * d1, a2 * e1 + c2 * f1 + e2, b2 * e1 + d2 * f1 + f2]
}

/// The matrix the options describe, `None` when they describe none. Values within 1e-12 of a
/// whole number are snapped to it, so a quarter turn is exactly one.
pub fn options_matrix(o: &TransformOptions) -> Result<Option<[f64; 6]>> {
    let named = o.scale.is_some() || o.slant.is_some() || o.rotate.is_some() || o.translate.is_some();
    let m = if let Some(m) = o.matrix {
        if named {
            return Err(TablesError::BadOptions("a matrix excludes rotate, translate, scale and slant".into()));
        }
        m
    } else if named {
        let mut m = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        if let Some((sx, sy)) = o.scale {
            m = compose(m, [sx, 0.0, 0.0, sy, 0.0, 0.0]);
        }
        if let Some(angle) = o.slant {
            if !angle.is_finite() || angle.abs() >= 90.0 {
                return Err(TablesError::BadOptions(format!("a slant of {angle}° is not between -90° and 90°")));
            }
            m = compose(m, [1.0, 0.0, angle.to_radians().tan(), 1.0, 0.0, 0.0]);
        }
        if let Some(angle) = o.rotate {
            if !angle.is_finite() {
                return Err(TablesError::BadOptions("the rotation must be a finite number of degrees".into()));
            }
            let (sin, cos) = angle.to_radians().sin_cos();
            m = compose(m, [cos, -sin, sin, cos, 0.0, 0.0]);
        }
        if let Some((tx, ty)) = o.translate {
            m = compose(m, [1.0, 0.0, 0.0, 1.0, tx, ty]);
        }
        m
    } else {
        return Ok(None);
    };
    if m.iter().any(|v| !v.is_finite() || v.abs() > 1.0e6) {
        return Err(TablesError::BadOptions("matrix values must be finite and at most 1,000,000 in size".into()));
    }
    let m = m.map(|v| if (v - v.round()).abs() < 1e-12 { v.round() + 0.0 } else { v });
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-9 {
        return Err(TablesError::BadOptions("the matrix flattens the outlines (its determinant is zero)".into()));
    }
    Ok(Some(m))
}

/// A 2×2 matrix as `[xx, yx, xy, yy]`.
type Linear = [f64; 4];

fn linear(m: [f64; 6]) -> Linear {
    [m[0], m[1], m[2], m[3]]
}

fn mul(p: Linear, q: Linear) -> Linear {
    let [pxx, pyx, pxy, pyy] = p;
    let [qxx, qyx, qxy, qyy] = q;
    [pxx * qxx + pxy * qyx, pyx * qxx + pyy * qyx, pxx * qxy + pxy * qyy, pyx * qxy + pyy * qyy]
}

fn apply(p: Linear, (x, y): (f64, f64)) -> (f64, f64) {
    (p[0] * x + p[2] * y, p[1] * x + p[3] * y)
}

fn inverse(p: Linear) -> Option<Linear> {
    let [xx, yx, xy, yy] = p;
    let det = xx * yy - xy * yx;
    (det.abs() > 1e-12).then(|| [yy / det, -yx / det, -xy / det, xx / det])
}

const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// The tables every transform rewrites, decoded.
struct Work {
    font: SfntFont,
    upem: u16,
    /// The glyph count `maxp` gives.
    count: usize,
    /// The `glyf` glyphs; empty for CFF outlines.
    glyphs: Vec<Glyph>,
    /// CFF outlines: whether they are in `CFF2`, and each glyph's exact bounds once changed.
    cff: Option<(bool, Option<Vec<Option<[f64; 4]>>>)>,
    /// How many CFF glyphs a matrix changed; `None` when every glyph changed.
    cff_changed: Option<usize>,
    hmtx: fixed::Metrics,
    vmtx: Option<fixed::Metrics>,
    metrics_changed: bool,
}

impl Work {
    fn read(font: &SfntFont) -> Result<Self> {
        let need = |tag: &[u8; 4]| font.get(*tag).ok_or_else(|| TablesError::MissingTable(String::from_utf8_lossy(tag).trim_end().to_string()));
        let head = need(b"head")?;
        let maxp = need(b"maxp")?;
        let hhea = need(b"hhea")?;
        let hmtx = need(b"hmtx")?;
        let upem = u16_at(head, 18).ok_or_else(|| TablesError::invalid("the head table is cut short"))?;
        if upem == 0 {
            return Err(TablesError::invalid("the head table gives 0 units per em"));
        }
        let long = i16_at(head, 50).ok_or_else(|| TablesError::invalid("the head table is cut short"))? != 0;
        let count = usize::from(u16_at(maxp, 4).ok_or_else(|| TablesError::invalid("the maxp table is cut short"))?);
        let cff = font.is_cff().then(|| (font.get(*b"CFF2").is_some(), None));
        let glyphs = if cff.is_some() {
            Vec::new()
        } else {
            let glyf = need(b"glyf")?;
            let offsets = glyf::read_loca(need(b"loca")?, long, count, glyf.len())?;
            glyf::read_glyphs(glyf, &offsets)?
        };
        let long_h = usize::from(u16_at(hhea, 34).ok_or_else(|| TablesError::invalid("the hhea table is cut short"))?);
        let hmtx = fixed::read_metrics(hmtx, long_h, count)?;
        let vmtx = match (font.get(*b"vhea"), font.get(*b"vmtx")) {
            (Some(vhea), Some(vmtx)) => {
                let long_v = usize::from(u16_at(vhea, 34).ok_or_else(|| TablesError::invalid("the vhea table is cut short"))?);
                Some(fixed::read_metrics(vmtx, long_v, count)?)
            }
            _ => None,
        };
        Ok(Work { font: font.clone(), upem, count, glyphs, cff, cff_changed: None, hmtx, vmtx, metrics_changed: false })
    }

    fn apply_matrix(&mut self, m: [f64; 6], o: &TransformOptions, report: &mut TransformReport) -> Result<()> {
        if self.cff.is_some() {
            return self.apply_matrix_cff(m, o, report);
        }
        let selected = |g: usize| o.glyphs.as_ref().is_none_or(|set| u16::try_from(g).is_ok_and(|g| set.contains(&g)));
        let matrix_of = |g: usize| if selected(g) { m } else { IDENTITY };
        let mut negative_widths = 0usize;
        for gid in 0..self.glyphs.len() {
            let own = matrix_of(gid);
            let Some(glyph) = self.glyphs.get_mut(gid) else { continue };
            match &mut glyph.shape {
                Shape::Empty => {}
                Shape::Simple(s) if selected(gid) => {
                    for p in s.points.iter_mut() {
                        let (x, y) = (f64::from(p.x), f64::from(p.y));
                        p.x = coord(own[0] * x + own[2] * y + own[4], &format!("glyph {gid}: a point's x"))?;
                        p.y = coord(own[1] * x + own[3] * y + own[5], &format!("glyph {gid}: a point's y"))?;
                    }
                    glyph.changed = true;
                }
                Shape::Simple(_) => {}
                Shape::Composite(c) => {
                    for comp in c.components.iter_mut() {
                        let child = matrix_of(usize::from(comp.glyph));
                        if own == IDENTITY && child == IDENTITY {
                            continue;
                        }
                        let child_inverse = inverse(linear(child)).ok_or_else(|| TablesError::BadOptions("the matrix cannot be inverted".into()))?;
                        let new_matrix = mul(mul(linear(own), comp.matrix), child_inverse);
                        if let Args::Offset(x, y) = comp.args {
                            let d = (f64::from(x), f64::from(y));
                            let actual = if comp.flags & SCALED_COMPONENT_OFFSET != 0 { apply(comp.matrix, d) } else { d };
                            let moved = apply(linear(own), actual);
                            let shift = apply(new_matrix, (child[4], child[5]));
                            let what = format!("glyph {gid}: a component offset");
                            let nx = coord(moved.0 + own[4] - shift.0, &what)?;
                            let ny = coord(moved.1 + own[5] - shift.1, &what)?;
                            comp.args = Args::Offset(nx, ny);
                            if comp.flags & SCALED_COMPONENT_OFFSET != 0 {
                                comp.flags = (comp.flags & !SCALED_COMPONENT_OFFSET) | UNSCALED_COMPONENT_OFFSET;
                            }
                        }
                        comp.matrix = new_matrix;
                        glyph.changed = true;
                    }
                }
            }
            if glyph.changed && glyph.strip_instructions() {
                report.hints_kept = false;
            }
        }
        let bounds = glyf::all_bounds(&self.glyphs)?;
        for (gid, glyph) in self.glyphs.iter().enumerate() {
            if selected(gid) {
                let Some(w) = self.hmtx.advances.get_mut(gid) else { continue };
                let new = ot_round(new_advance(o.widths, m, *w, self.upem));
                if new < 0.0 {
                    negative_widths += 1;
                }
                let new =
                    u16::try_from(new.max(0.0) as i64).map_err(|_| TablesError::Invalid(format!("glyph {gid}: the advance width would be {new}")))?;
                if new != *w {
                    *w = new;
                    self.metrics_changed = true;
                }
            }
            if glyph.changed
                && let (Some(b), Some(lsb)) = (bounds.get(gid).copied().flatten(), self.hmtx.bearings.get_mut(gid))
            {
                let x_min = i16::try_from(b[0]).map_err(|_| TablesError::Invalid(format!("glyph {gid}: xMin would be {}", b[0])))?;
                if *lsb != x_min {
                    *lsb = x_min;
                    self.metrics_changed = true;
                }
            }
        }
        if negative_widths > 0 {
            report.warnings.push(format!("{negative_widths} advance widths would be negative and are set to 0"));
        }
        if self.vmtx.is_some() && self.glyphs.iter().any(|g| g.changed) {
            report.warnings.push("the vertical metrics (vmtx) are left as they were".into());
        }
        let any_changed = self.glyphs.iter().any(|g| g.changed);
        if any_changed || self.metrics_changed {
            for tag in [*b"hdmx", *b"LTSH", *b"VDMX"] {
                self.font.remove(tag);
            }
        }
        if any_changed && self.glyphs.iter().all(|g| g.instructions().is_empty()) {
            for tag in [*b"fpgm", *b"prep", *b"cvt ", *b"cvar"] {
                self.font.remove(tag);
            }
        }
        Ok(())
    }

    /// A matrix on `CFF ` or `CFF2` outlines ([`cff_matrix`]), with the advance widths and left
    /// side bearings that follow.
    fn apply_matrix_cff(&mut self, m: [f64; 6], o: &TransformOptions, report: &mut TransformReport) -> Result<()> {
        let Some((cff2, _)) = self.cff else { return Ok(()) };
        let tag = if cff2 { *b"CFF2" } else { *b"CFF " };
        let data = self.font.get(tag).ok_or_else(|| TablesError::MissingTable(crate::sfnt::tag_str(tag).trim_end().to_string()))?.to_vec();
        let selected = |g: usize| o.glyphs.as_ref().is_none_or(|set| u16::try_from(g).is_ok_and(|g| set.contains(&g)));
        let matrices: Vec<Option<[f64; 6]>> = (0..self.count).map(|g| selected(g).then_some(m)).collect();
        let mut negative_widths = 0usize;
        let mut widths = Vec::with_capacity(self.count);
        for (gid, own) in matrices.iter().enumerate() {
            let width = match (own, self.hmtx.advances.get(gid)) {
                (Some(own), Some(old)) => {
                    let new = ot_round(new_advance(o.widths, *own, *old, self.upem));
                    if new < 0.0 {
                        negative_widths += 1;
                    }
                    Some(new.max(0.0))
                }
                _ => None,
            };
            widths.push(width);
        }
        let em_factor = o.scale_em.then(|| ot_round(f64::from(self.upem) * m[0].abs()) / f64::from(self.upem));
        let job = cff_matrix::MatrixJob { matrix: m, glyphs: &matrices, widths: &widths, em_factor };
        let out = cff_matrix::apply_matrix(&data, cff2, &job)?;
        self.font.insert(tag, out.table);
        for (gid, (own, width)) in matrices.iter().zip(&widths).enumerate() {
            if let (Some(w), Some(slot)) = (width, self.hmtx.advances.get_mut(gid)) {
                let w = u16::try_from(*w as i64).map_err(|_| TablesError::Invalid(format!("glyph {gid}: the advance width would be {w}")))?;
                if *slot != w {
                    *slot = w;
                    self.metrics_changed = true;
                }
            }
            if own.is_some()
                && let (Some(Some(b)), Some(lsb)) = (out.bounds.get(gid), self.hmtx.bearings.get_mut(gid))
            {
                let x_min = i16::try_from(b[0].floor() as i64).map_err(|_| TablesError::Invalid(format!("glyph {gid}: xMin would be {}", b[0])))?;
                if *lsb != x_min {
                    *lsb = x_min;
                    self.metrics_changed = true;
                }
            }
        }
        if negative_widths > 0 {
            report.warnings.push(format!("{negative_widths} advance widths would be negative and are set to 0"));
        }
        if self.vmtx.is_some() && out.glyphs_changed > 0 {
            report.warnings.push("the vertical metrics (vmtx) are left as they were".into());
        }
        report.hints_kept &= out.hints_kept;
        report.warnings.extend(out.warnings);
        self.cff = Some((cff2, Some(out.bounds)));
        self.cff_changed = Some(out.glyphs_changed);
        Ok(())
    }

    fn apply_upem(&mut self, target: u16, report: &mut TransformReport) -> Result<()> {
        let s = Scaler { factor: f64::from(target) / f64::from(self.upem) };
        // gvar counts each glyph's points before the glyphs are rewritten (scaling keeps them).
        let points: Vec<usize> = self.glyphs.iter().map(Glyph::variation_points).collect();
        for (gid, glyph) in self.glyphs.iter_mut().enumerate() {
            match &mut glyph.shape {
                Shape::Empty => continue,
                Shape::Simple(simple) => {
                    for p in simple.points.iter_mut() {
                        p.x = scaled_coord(s, p.x, gid)?;
                        p.y = scaled_coord(s, p.y, gid)?;
                    }
                }
                Shape::Composite(c) => {
                    for comp in c.components.iter_mut() {
                        if let Args::Offset(x, y) = comp.args {
                            comp.args = Args::Offset(scaled_coord(s, x, gid)?, scaled_coord(s, y, gid)?);
                        }
                    }
                }
            }
            glyph.changed = true;
        }
        for m in std::iter::once(&mut self.hmtx).chain(self.vmtx.as_mut()) {
            for a in m.advances.iter_mut() {
                let r = s.i32(i32::from(*a));
                *a = u16::try_from(r).map_err(|_| TablesError::Invalid(format!("an advance would be {r}, outside 0 to 65,535")))?;
            }
            for b in m.bearings.iter_mut() {
                *b = s.i16(*b, "a side bearing")?;
            }
        }
        self.metrics_changed = true;
        self.upem = target;

        let font = &mut self.font;
        for tag in [*b"hhea", *b"vhea"] {
            patch(font, tag, &mut |t| fixed::HHEA_SCALED.iter().try_for_each(|&(at, what)| fixed::scale_i16(t, at, s, what)))?;
        }
        patch(font, *b"OS/2", &mut |t| {
            fixed::OS2_SIGNED.iter().try_for_each(|&(at, what)| fixed::scale_i16(t, at, s, what))?;
            fixed::OS2_UNSIGNED.iter().try_for_each(|&(at, what)| fixed::scale_u16(t, at, s, what))
        })?;
        let glyph_count = self.count;
        patch(font, *b"post", &mut |t| {
            fixed::scale_i16(t, 8, s, "the underline position")?;
            fixed::scale_i16(t, 10, s, "the underline thickness")?;
            *t = post::recompile(t, glyph_count)?;
            Ok(())
        })?;
        patch(font, *b"VORG", &mut |t| fixed::scale_vorg(t, s))?;
        let mut kern_skipped = 0;
        patch(font, *b"kern", &mut |t| {
            kern_skipped = fixed::scale_kern(t, s)?;
            Ok(())
        })?;
        if kern_skipped > 0 {
            report.warnings.push(format!("{kern_skipped} kern subtables of formats other than 0 are left unscaled"));
        }
        let cvt_count = font.get(*b"cvt ").map_or(0, |t| t.len() / 2);
        patch(font, *b"cvt ", &mut |t| fixed::scale_cvt(t, s))?;
        let axes = font.get(*b"fvar").and_then(|t| u16_at(t, 8)).map_or(0, usize::from);
        let mut deltas = |lists: &mut [Vec<i32>]| -> Result<()> {
            for v in lists.iter_mut().flat_map(|l| l.iter_mut()) {
                *v = s.i32(*v);
            }
            Ok(())
        };
        patch(font, *b"cvar", &mut |t| {
            *t = tuples::rewrite_cvar(t, axes, cvt_count, &mut deltas)?;
            Ok(())
        })?;
        patch(font, *b"gvar", &mut |t| {
            *t = tuples::rewrite_gvar(t, &points, &mut deltas)?;
            Ok(())
        })?;
        patch(font, *b"GPOS", &mut |t| {
            *t = layout::scale_gpos(t, s)?;
            Ok(())
        })?;
        patch(font, *b"GDEF", &mut |t| {
            if let Some(n) = layout::scale_gdef(t, s)? {
                *t = n;
            }
            Ok(())
        })?;
        patch(font, *b"BASE", &mut |t| {
            *t = layout::scale_base(t, s)?;
            Ok(())
        })?;
        patch(font, *b"HVAR", &mut |t| {
            *t = layout::scale_hvar(t, s)?;
            Ok(())
        })?;
        patch(font, *b"VVAR", &mut |t| {
            *t = layout::scale_vvar(t, s)?;
            Ok(())
        })?;
        patch(font, *b"MVAR", &mut |t| {
            *t = layout::scale_mvar(t, s)?;
            Ok(())
        })?;
        let unscaled: Vec<String> = [*b"MATH", *b"COLR", *b"VARC", *b"kerx", *b"trak", *b"ankr", *b"opbd", *b"lcar", *b"Silf", *b"Glat"]
            .into_iter()
            .filter(|t| font.get(*t).is_some())
            .map(|t| crate::sfnt::tag_str(t).trim_end().to_string())
            .collect();
        if !unscaled.is_empty() {
            report.warnings.push(format!("these tables hold values in font units and are left unscaled: {}", unscaled.join(", ")));
        }
        if let Some((cff2, bounds)) = &mut self.cff {
            let tag = if *cff2 { *b"CFF2" } else { *b"CFF " };
            let data = font.get(tag).ok_or_else(|| TablesError::MissingTable(crate::sfnt::tag_str(tag).trim_end().to_string()))?;
            let scaled = cff::scale_upem(data, *cff2, s.factor)?;
            font.insert(tag, scaled.table);
            *bounds = Some(scaled.bounds);
            report.warnings.extend(scaled.warnings);
        }
        let hinted = font.get(*b"fpgm").is_some() || font.get(*b"prep").is_some() || self.glyphs.iter().any(|g| !g.instructions().is_empty());
        if hinted {
            report.warnings.push("the TrueType instructions are kept but rarely stay correct at a new units per em: hint the font again".into());
        }
        Ok(())
    }

    /// Writes `glyf`, `loca`, the metrics tables and the values worked out from them.
    fn finish(mut self, report: &mut TransformReport) -> Result<SfntFont> {
        let cff_bounds = self.cff.as_ref().and_then(|(_, b)| b.as_ref());
        let (bounds, outlines_changed) = match cff_bounds {
            Some(b) => {
                report.glyphs_changed = self.cff_changed.unwrap_or(b.len());
                let whole: Vec<Option<[i32; 4]>> =
                    b.iter().map(|b| b.map(|[x0, y0, x1, y1]| [x0.floor() as i32, y0.floor() as i32, x1.ceil() as i32, y1.ceil() as i32])).collect();
                (whole, true)
            }
            None => {
                report.glyphs_changed = self.glyphs.iter().filter(|g| g.changed).count();
                (glyf::all_bounds(&self.glyphs)?, report.glyphs_changed > 0)
            }
        };
        let any_changed = report.glyphs_changed > 0 && cff_bounds.is_none();
        let font = &mut self.font;
        if any_changed {
            let (glyf, loca, long) = glyf::build(&self.glyphs, &bounds)?;
            font.insert(*b"glyf", glyf);
            font.insert(*b"loca", loca);
            if let Some(mut head) = font.get(*b"head").map(<[u8]>::to_vec) {
                put_i16(&mut head, 50, i16::from(long));
                font.insert(*b"head", head);
            }
            // fontTools' `maxp.recalc` keeps `maxSizeOfInstructions` as stored; only a transform
            // that removed glyph programs lowers it to the largest one left.
            if !report.hints_kept
                && let Some(mut maxp) = font.get(*b"maxp").map(<[u8]>::to_vec)
                && maxp.len() >= 32
            {
                let largest = self.glyphs.iter().map(|g| g.instructions().len()).max().unwrap_or(0);
                if let Some(old) = u16_at(&maxp, 26)
                    && usize::from(old) > largest
                {
                    put_u16(&mut maxp, 26, largest as u16);
                }
                font.insert(*b"maxp", maxp);
            }
        }
        if let Some(mut head) = font.get(*b"head").map(<[u8]>::to_vec) {
            put_u16(&mut head, 18, self.upem);
            if outlines_changed {
                let mut all: Option<[i32; 4]> = None;
                for b in bounds.iter().flatten() {
                    all = Some(match all {
                        None => *b,
                        Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
                    });
                }
                let all = all.unwrap_or_default();
                for (i, v) in all.iter().enumerate() {
                    let v = i16::try_from(*v).map_err(|_| TablesError::Invalid(format!("the font bounds would reach {v}")))?;
                    put_i16(&mut head, 36 + 2 * i, v);
                }
            }
            font.insert(*b"head", head);
        }
        if outlines_changed || self.metrics_changed {
            let (hmtx, long) = fixed::write_metrics(&self.hmtx);
            font.insert(*b"hmtx", hmtx);
            if let Some(mut hhea) = font.get(*b"hhea").map(<[u8]>::to_vec) {
                put_u16(&mut hhea, 34, long);
                let extents: Vec<Option<(i32, i32)>> = bounds.iter().map(|b| b.map(|b| (b[0], b[2]))).collect();
                fixed::recalc_extremes(&mut hhea, &self.hmtx, &extents)?;
                font.insert(*b"hhea", hhea);
            }
            if let Some(vmtx) = &self.vmtx {
                let (table, long) = fixed::write_metrics(vmtx);
                font.insert(*b"vmtx", table);
                if let Some(mut vhea) = font.get(*b"vhea").map(<[u8]>::to_vec) {
                    put_u16(&mut vhea, 34, long);
                    let extents: Vec<Option<(i32, i32)>> = bounds.iter().map(|b| b.map(|b| (b[1], b[3]))).collect();
                    fixed::recalc_extremes(&mut vhea, vmtx, &extents)?;
                    font.insert(*b"vhea", vhea);
                }
            }
        }
        if any_changed || report.units_per_em.is_some() {
            crate::ttx::read_core::recalc_compiled_maxp(font)?;
        }
        Ok(self.font)
    }
}

/// A transformed glyph's advance width before rounding, from its old advance `old`.
fn new_advance(widths: Widths, m: [f64; 6], old: u16, upem: u16) -> f64 {
    let old = f64::from(old);
    match widths {
        Widths::Keep => old,
        Widths::Em => f64::from(upem),
        Widths::Transform => m[0] * old + m[4],
        Widths::Auto if m[1] == 0.0 => m[0] * old + m[4],
        Widths::Auto => f64::from(upem),
    }
}

/// Runs `f` on a copy of the table and puts the result back; nothing when the font lacks it.
fn patch(font: &mut SfntFont, tag: Tag, f: &mut dyn FnMut(&mut Vec<u8>) -> Result<()>) -> Result<()> {
    if let Some(mut t) = font.get(tag).map(<[u8]>::to_vec) {
        f(&mut t)?;
        font.insert(tag, t);
    }
    Ok(())
}

fn scaled_coord(s: Scaler, v: i32, gid: usize) -> Result<i32> {
    let r = s.i32(v);
    if f64::from(r).abs() > MAX_COORDINATE {
        return Err(TablesError::Invalid(format!("glyph {gid}: a coordinate would be {r}, outside ±32,767")));
    }
    Ok(r)
}
