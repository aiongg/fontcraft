//! Outline repairs on `CFF ` and static `CFF2` outlines (M12.66; rows `outlines remove-overlaps`,
//! `direction`, `extremes`, `round` and their `outlines.*` commands). Oracles: `tx +V`,
//! checkoutlinesufo `-e` and fontTools `ttLib.removeOverlaps` on OTF input.
//!
//! Each selected glyph is drawn from its expanded charstring, repaired with the editor's geometry
//! (`fontcraft_pathops`, as for `glyf`), and replaced only when its outline changes. A replaced
//! glyph gets a new, unhinted charstring with the same width operand, as `tx +V` writes it; every
//! other charstring keeps its bytes, hints and subroutine calls, and the subroutines and DICTs are
//! copied, so a subroutinised input stays subroutinised. Every table but the outline table keeps
//! its bytes.
//!
//! | Action | What changes |
//! |---|---|
//! | Remove overlaps | The glyph's contours merged (nonzero rule, points on whole units), keeping the drawn winding; contours whose bounding box is smaller than `min_area` square units are dropped. The subroutines no charstring calls any more are removed unless `keep_unused_subroutines` |
//! | Correct direction | `path.correctPathDirection` (PostScript convention by default), the TrueType convention reversing its result, or every contour reversed |
//! | Add extremes | `path.addExtremes` on each contour, new points on whole units |
//! | Round | Point and control-point coordinates to multiples of `grid`; with `tolerance`, only values that close to a grid position. Widths stay |
//!
//! A `seac` glyph draws nothing of its own and is left alone. Decomposing is refused: CFF
//! outlines have no components.

use std::collections::{BTreeMap, HashMap};

use fontcraft_cff::PathOp;
use fontcraft_cff::subr::{SubrMode, TableSubset};
use fontcraft_geom::{BezPath, Point};
use fontcraft_pathops as po;

use super::{GlyphFailure, OutlineAction, OutlineOptions, OutlineReport, OutlineResult, Winding, limited, ot_round, selection, u16_at};
use crate::convert::cff_table::{Programs, path_charstring, subr_error};
use crate::convert::outline::{Contour, P, Seg, path_ops};
use crate::convert::overlaps::{from_bez, merged};
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

type Repaired = std::result::Result<Option<(Vec<PathOp>, usize)>, String>;

/// [`super::repair_report`] on a font whose outlines are in `CFF ` or `CFF2`.
pub(super) fn repair_report(font: &SfntFont, options: &OutlineOptions) -> Result<OutlineReport> {
    if options.action == OutlineAction::Decompose {
        return Err(TablesError::Unsupported("decomposing CFF outlines: they have no components".into()));
    }
    let (tag, cff2) = if font.get(*b"CFF ").is_some() { (*b"CFF ", false) } else { (*b"CFF2", true) };
    let table = font.get(tag).ok_or_else(|| TablesError::MissingTable("CFF or CFF2".into()))?;
    let maxp = font.get(*b"maxp").ok_or_else(|| TablesError::MissingTable("maxp".into()))?;
    let count = usize::from(u16_at(maxp, 4).ok_or_else(|| TablesError::invalid("the maxp table is cut short"))?);
    let programs = Programs::read(table, cff2)?;
    let n = programs.len();
    if n != count {
        return Err(TablesError::invalid(format!("the outline table has {n} glyphs, maxp {count}")));
    }
    let names = crate::reorder::glyph_names(font)?;
    let name = |gid: usize| names.get(gid).cloned().unwrap_or_else(|| format!("glyph{gid:05}"));
    let selected = selection(&names, n, options)?;

    let mut replace: HashMap<usize, Vec<u8>> = HashMap::new();
    let mut changed = Vec::new();
    let mut failed = Vec::new();
    let mut points_added = BTreeMap::new();
    let mut values_changed = 0usize;
    for gid in selected {
        if programs.is_accented(gid) {
            continue;
        }
        let outcome = match programs.draw(gid) {
            Ok(drawing) => {
                let contours: Vec<Contour> = drawing.contours.into_iter().filter(|c| !c.segs.is_empty()).collect();
                if contours.is_empty() { Ok(None) } else { repaired(&contours, options) }
            }
            Err(e) => Err(e.to_string()),
        };
        let code = outcome.and_then(|r| match r {
            Some((ops, amount)) => path_charstring(programs.width(gid), &ops, cff2).map(|c| Some((c, amount))).map_err(|e| e.to_string()),
            None => Ok(None),
        });
        match code {
            Ok(Some((code, amount))) => {
                replace.insert(gid, code);
                changed.push(name(gid));
                match options.action {
                    OutlineAction::AddExtremes => {
                        points_added.insert(name(gid), amount);
                    }
                    OutlineAction::Round => values_changed = values_changed.saturating_add(amount),
                    _ => {}
                }
            }
            Ok(None) => {}
            Err(reason) => {
                let glyph = name(gid);
                if !options.ignore_errors {
                    return Err(TablesError::Invalid(format!("glyph `{glyph}`: {reason}")));
                }
                failed.push(GlyphFailure { glyph, reason });
            }
        }
    }

    let mut out = font.clone();
    if !replace.is_empty() {
        let identity: Vec<u16> = (0..n).map(|g| u16::try_from(g).unwrap_or(u16::MAX)).collect();
        let mut rebuilt = if cff2 {
            crate::reorder::cff2::parse(table, n)?.rebuild(&identity, &replace)?
        } else {
            crate::reorder::cff::parse(table, n)?.rebuild_with(&identity, None, &replace, false)?
        };
        if options.action == OutlineAction::RemoveOverlaps && !options.keep_unused_subroutines {
            let subset = TableSubset { glyphs: None, emptied: &[], subroutines: SubrMode::Prune, max_expanded: 0 };
            rebuilt = fontcraft_cff::subr::subset_table(&rebuilt, cff2, &subset).map_err(subr_error)?;
        }
        out.insert(tag, rebuilt);
    }
    Ok(OutlineReport { result: OutlineResult { font: out, changed }, failed, points_added, values_changed })
}

/// The glyph's drawing steps after the repair and the points added or values moved; `None` when
/// the repair changes nothing.
fn repaired(contours: &[Contour], options: &OutlineOptions) -> Repaired {
    let before = path_ops(contours).map_err(|e| e.to_string())?;
    match options.action {
        OutlineAction::RemoveOverlaps => {
            let Some(mut result) = merged(contours)? else { return Ok(None) };
            let result_area = signed_area(&result);
            if result_area != 0.0 && (signed_area(contours) < 0.0) != (result_area < 0.0) {
                result = result.iter().map(|c| closed(c).reversed()).collect();
            }
            result.retain(|c| area_of_bounds(c) >= options.min_area);
            Ok(Some((path_ops(&result).map_err(|e| e.to_string())?, 0)))
        }
        OutlineAction::CorrectDirection => {
            let result: Vec<Contour> = match options.winding {
                Winding::Reverse => contours.iter().map(Contour::reversed).collect(),
                Winding::Auto | Winding::PostScript | Winding::TrueType => {
                    let input = po::contours_from_bez(&bez(contours)?);
                    let directed = limited(move || po::correct_direction(&input))?;
                    let out = back(&directed.contours)?;
                    if options.winding == Winding::TrueType { out.iter().map(Contour::reversed).collect() } else { out }
                }
            };
            let after = path_ops(&result).map_err(|e| e.to_string())?;
            Ok((key(&after) != key(&before)).then_some((after, 0)))
        }
        OutlineAction::AddExtremes => {
            let input = po::contours_from_bez(&bez(contours)?);
            let points_before = on_curve(&input);
            let extremes = po::ExtremesOptions { force: options.min_segment == 0.0, grid: 1.0 };
            let done = limited(move || {
                input
                    .iter()
                    .map(|c| if c.nodes.len() > 1 { po::add_extremes(c, None, extremes) } else { Ok(c.clone()) })
                    .collect::<std::result::Result<Vec<_>, _>>()
            })?;
            let added = on_curve(&done).saturating_sub(points_before);
            let after = path_ops(&back(&done)?).map_err(|e| e.to_string())?;
            Ok((key(&after) != key(&before)).then_some((after, added)))
        }
        OutlineAction::Round => {
            let (grid, tolerance) = (options.grid, options.tolerance);
            let mut moved = 0usize;
            let mut r = |p: P| {
                let one = |v: f64| {
                    let g = (v / grid).round() * grid;
                    if tolerance.is_none_or(|t| (v - g).abs() <= t) { ot_round(g) } else { v }
                };
                let q = (one(p.0), one(p.1));
                moved += usize::from(q.0 != p.0) + usize::from(q.1 != p.1);
                q
            };
            let after: Vec<PathOp> = before
                .iter()
                .map(|op| match *op {
                    PathOp::MoveTo(p) => PathOp::MoveTo(r(p)),
                    PathOp::LineTo(p) => PathOp::LineTo(r(p)),
                    PathOp::CurveTo(a, b, c) => PathOp::CurveTo(r(a), r(b), r(c)),
                })
                .collect();
            Ok((moved > 0).then_some((after, moved)))
        }
        OutlineAction::Decompose => Ok(None),
    }
}

fn pt(p: P) -> Point {
    Point::new(p.0, p.1)
}

/// The contours as one path for the editor's geometry, each closed.
fn bez(contours: &[Contour]) -> std::result::Result<BezPath, String> {
    let mut path = BezPath::new();
    for c in contours {
        path.move_to(pt(c.start));
        for s in &c.segs {
            match s {
                Seg::Line(p) => path.line_to(pt(*p)),
                Seg::Cubic(a, b, p) => path.curve_to(pt(*a), pt(*b), pt(*p)),
                Seg::Quad(..) => return Err("a quadratic curve in a PostScript outline".into()),
            }
        }
        path.close_path();
    }
    Ok(path)
}

/// The editor's contours as closed contours.
fn back(contours: &[po::Contour]) -> std::result::Result<Vec<Contour>, String> {
    let mut out = Vec::with_capacity(contours.len());
    for c in contours {
        let bez = c.to_bez().map_err(|e| e.to_string())?;
        out.extend(from_bez(&bez).iter().map(closed));
    }
    Ok(out)
}

fn on_curve(contours: &[po::Contour]) -> usize {
    contours.iter().map(|c| c.nodes.iter().filter(|n| n.kind != po::NodeKind::OffCurve).count()).sum()
}

fn seg_end(s: &Seg) -> P {
    match s {
        Seg::Line(p) | Seg::Cubic(_, _, p) | Seg::Quad(_, p) => *p,
    }
}

/// The contour with its closing line written out, which [`Contour::reversed`] needs.
fn closed(c: &Contour) -> Contour {
    let mut c = c.clone();
    if c.segs.last().map(seg_end).is_some_and(|end| end != c.start) {
        c.segs.push(Seg::Line(c.start));
    }
    c
}

fn points(c: &Contour) -> impl Iterator<Item = P> + '_ {
    std::iter::once(c.start).chain(c.segs.iter().flat_map(|s| match s {
        Seg::Line(p) => vec![*p],
        Seg::Cubic(a, b, p) => vec![*a, *b, *p],
        Seg::Quad(offs, p) => offs.iter().copied().chain(std::iter::once(*p)).collect(),
    }))
}

/// Twice the signed area of the contours' point polygons: negative when they run clockwise.
fn signed_area(contours: &[Contour]) -> f64 {
    let mut sum = 0.0;
    for c in contours {
        let pts: Vec<P> = points(c).collect();
        for (i, p) in pts.iter().enumerate() {
            if let Some(q) = pts.get((i + 1) % pts.len()) {
                sum += p.0 * q.1 - q.0 * p.1;
            }
        }
    }
    sum
}

fn area_of_bounds(c: &Contour) -> f64 {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in points(c) {
        (x0, y0, x1, y1) = (x0.min(p.0), y0.min(p.1), x1.max(p.0), y1.max(p.1));
    }
    let area = (x1 - x0) * (y1 - y0);
    if area.is_finite() { area } else { 0.0 }
}

/// The drawing steps on a 1/64-unit grid, for telling whether a repair changed anything.
fn key(ops: &[PathOp]) -> Vec<i64> {
    let q = |v: f64| (v * 64.0).round() as i64;
    let mut out = Vec::with_capacity(ops.len().saturating_mul(4));
    for op in ops {
        match *op {
            PathOp::MoveTo(p) => out.extend([0, q(p.0), q(p.1)]),
            PathOp::LineTo(p) => out.extend([1, q(p.0), q(p.1)]),
            PathOp::CurveTo(a, b, c) => out.extend([2, q(a.0), q(a.1), q(b.0), q(b.1), q(c.0), q(c.1)]),
        }
    }
    out
}

#[cfg(test)]
#[path = "outlines_cff_tests.rs"]
mod tests;
