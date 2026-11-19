//! Overlap removal on `CFF ` and static `CFF2` outlines (`tx +V`, fontTools `CFF2ToCFF
//! --remove-overlaps`), for `font.convert` (M12.30d).
//!
//! Each glyph is drawn from its expanded charstring ([`Programs`]), its contours are merged with
//! the editor's Remove Overlap (`fontcraft_pathops`, nonzero fill, result points on the
//! one-unit grid), and a glyph whose outline changes gets a new, unhinted charstring with the
//! same width operand. Every other charstring, the subroutines and the DICTs are copied as they
//! are. A `seac` glyph draws nothing of its own and is left alone; its base and accent are merged
//! as glyphs of their own.

use std::collections::HashMap;

use fontcraft_geom::{BezPath, PathEl, Point};
use fontcraft_pathops as po;

use super::cff_table::{Programs, path_charstring};
use super::outline::{Contour, P, Seg, path_ops};
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// `font` with the overlapping contours of its `CFF ` or `CFF2` glyphs merged. A glyph that
/// cannot be merged fails the call, or with `ignore_errors` stays as it was with a note.
pub(super) fn remove_ps_overlaps(font: &SfntFont, ignore_errors: bool, notes: &mut Vec<String>) -> Result<SfntFont> {
    let (tag, cff2) = if font.get(*b"CFF ").is_some() { (*b"CFF ", false) } else { (*b"CFF2", true) };
    let table = font.get(tag).ok_or_else(|| TablesError::MissingTable("glyf, CFF or CFF2".into()))?;
    let n = super::num_glyphs(font)?;
    let programs = Programs::read(table, cff2)?;
    if programs.len() != n {
        return Err(TablesError::invalid(format!("the outline table has {} glyphs, maxp {n}", programs.len())));
    }
    let names = crate::reorder::glyph_names(font)?;
    let mut replace: HashMap<usize, Vec<u8>> = HashMap::new();
    for gid in 0..n {
        if programs.is_accented(gid) {
            continue;
        }
        let drawing = programs.draw(gid)?;
        let contours: Vec<Contour> = drawing.contours.into_iter().filter(|c| !c.segs.is_empty()).collect();
        if contours.is_empty() {
            continue;
        }
        match merged(&contours) {
            Ok(None) => {}
            Ok(Some(result)) => {
                replace.insert(gid, path_charstring(programs.width(gid), &path_ops(&result)?, cff2)?);
            }
            Err(why) => {
                let name = names.get(gid).cloned().unwrap_or_else(|| format!("glyph {gid}"));
                if !ignore_errors {
                    return Err(TablesError::Unsupported(format!("the overlaps of `{name}` cannot be merged: {why}")));
                }
                notes.push(format!("the overlaps of `{name}` ({why})"));
            }
        }
    }
    if replace.is_empty() {
        return Ok(font.clone());
    }
    let identity: Vec<u16> = (0..n).map(|g| u16::try_from(g).unwrap_or(u16::MAX)).collect();
    let rebuilt = if cff2 {
        crate::reorder::cff2::parse(table, n)?.rebuild(&identity, &replace)?
    } else {
        crate::reorder::cff::parse(table, n)?.rebuild_with(&identity, None, &replace, false)?
    };
    notes.push(format!("the stem hints of the glyphs whose overlaps were merged ({})", replace.len()));
    let mut out = font.clone();
    out.insert(tag, rebuilt);
    Ok(out)
}

/// The glyph's contours merged; `None` when merging changes nothing.
pub(crate) fn merged(contours: &[Contour]) -> std::result::Result<Option<Vec<Contour>>, String> {
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
    let input = po::contours_from_bez(&path);
    let options = po::OverlapOptions { grid: 1.0, open_paths: po::OpenPaths::Remove };
    let result = po::run_with_time_limit(po::TIME_LIMIT, move || po::remove_overlap(&input, options))
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(result.len());
    for c in &result {
        let bez = c.to_bez().map_err(|e| e.to_string())?;
        out.extend(from_bez(&bez));
    }
    let rounded: Vec<Contour> = contours.iter().map(round).collect();
    Ok((canonical(&rounded) != canonical(&out)).then_some(out))
}

fn pt(p: P) -> Point {
    Point::new(p.0, p.1)
}

pub(crate) fn from_bez(path: &BezPath) -> Vec<Contour> {
    let mut out: Vec<Contour> = Vec::new();
    let p = |q: Point| (q.x, q.y);
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(a) => out.push(Contour { start: p(a), segs: Vec::new() }),
            PathEl::LineTo(a) => {
                if let Some(c) = out.last_mut() {
                    c.segs.push(Seg::Line(p(a)));
                }
            }
            PathEl::QuadTo(q, a) => {
                if let Some(c) = out.last_mut() {
                    // Raised exactly; cubic input never gives one, but the type allows it.
                    let from = c.segs.last().map_or(c.start, seg_end);
                    let c1 = (from.0 + 2.0 / 3.0 * (q.x - from.0), from.1 + 2.0 / 3.0 * (q.y - from.1));
                    let c2 = (a.x + 2.0 / 3.0 * (q.x - a.x), a.y + 2.0 / 3.0 * (q.y - a.y));
                    c.segs.push(Seg::Cubic(c1, c2, p(a)));
                }
            }
            PathEl::CurveTo(a, b, c2) => {
                if let Some(c) = out.last_mut() {
                    c.segs.push(Seg::Cubic(p(a), p(b), p(c2)));
                }
            }
            PathEl::ClosePath => {}
        }
    }
    out.retain(|c| !c.segs.is_empty());
    out
}

fn seg_end(s: &Seg) -> P {
    match s {
        Seg::Line(p) | Seg::Cubic(_, _, p) | Seg::Quad(_, p) => *p,
    }
}

fn round(c: &Contour) -> Contour {
    let r = |p: &P| (p.0.round(), p.1.round());
    let segs = c
        .segs
        .iter()
        .map(|s| match s {
            Seg::Line(p) => Seg::Line(r(p)),
            Seg::Cubic(a, b, p) => Seg::Cubic(r(a), r(b), r(p)),
            Seg::Quad(offs, p) => Seg::Quad(offs.iter().map(r).collect(), r(p)),
        })
        .collect();
    Contour { start: r(&c.start), segs }
}

/// The contours as sorted point lists, sorted: equal for the same outline whatever the start
/// points, directions and contour order, with a closing line back to the start not counted.
fn canonical(contours: &[Contour]) -> Vec<Vec<(i64, i64)>> {
    let key = |p: &P| ((p.0 * 64.0).round() as i64, (p.1 * 64.0).round() as i64);
    let mut out: Vec<Vec<(i64, i64)>> = contours
        .iter()
        .map(|c| {
            let mut points = vec![key(&c.start)];
            let n = c.segs.len();
            for (i, s) in c.segs.iter().enumerate() {
                match s {
                    Seg::Line(p) if i + 1 == n && *p == c.start => {}
                    Seg::Line(p) => points.push(key(p)),
                    Seg::Cubic(a, b, p) => points.extend([key(a), key(b), key(p)]),
                    Seg::Quad(offs, p) => {
                        points.extend(offs.iter().map(key));
                        points.push(key(p));
                    }
                }
            }
            if points.len() > 1 && points.last() == points.first() {
                points.pop();
            }
            points.sort_unstable();
            points
        })
        .collect();
    out.sort();
    out
}
