//! Variable TrueType outlines: one glyph's sources made point-compatible and quadratic (M9.5).
//!
//! Rows: `compile.cu2quAcrossMasters`, `intermediateLayer.sparseInExport`.
//!
//! Contract: every cubic segment is converted in all sources of the glyph at once
//! (`fontcraft_geom::cu2qu_multi`), so each source gets the same points; `glyf` is then written
//! from the origin's outlines by the static writer. Sources that are not point-compatible after
//! conversion are refused.

use fontcraft_geom::cu2qu_multi::curves_to_quadratic;
use fontcraft_geom::{BezPath, CubicBez, PathEl, Point};

use crate::CompileError;

/// `sources` (one list of contours per source, the same structure in each) with every cubic
/// segment replaced by the same number of quadratic segments in each source, within `tolerance`
/// font units.
pub fn quadratic_sources(glyph: &str, sources: &[Vec<BezPath>], tolerance: f64) -> Result<Vec<Vec<BezPath>>, CompileError> {
    let bad = |message: String| CompileError::Glyph { glyph: glyph.to_string(), message };
    let contours = sources.first().map_or(0, Vec::len);
    if sources.iter().any(|s| s.len() != contours) {
        return Err(bad("its sources do not have the same number of contours".into()));
    }
    let mut out: Vec<Vec<BezPath>> = vec![Vec::with_capacity(contours); sources.len()];
    for c in 0..contours {
        let paths: Vec<&BezPath> = sources.iter().filter_map(|s| s.get(c)).collect();
        let len = paths.first().map_or(0, |p| p.elements().len());
        if paths.iter().any(|p| p.elements().len() != len) {
            return Err(bad(format!("contour {c} does not have the same segments in every source")));
        }
        let mut built: Vec<BezPath> = vec![BezPath::new(); paths.len()];
        let mut last: Vec<Point> = vec![Point::ZERO; paths.len()];
        let mut start: Vec<Point> = vec![Point::ZERO; paths.len()];
        for e in 0..len {
            let els: Vec<PathEl> = paths.iter().filter_map(|p| p.elements().get(e).copied()).collect();
            let same_kind = els.windows(2).all(|w| {
                matches!(
                    (w.first(), w.get(1)),
                    (Some(PathEl::MoveTo(_)), Some(PathEl::MoveTo(_)))
                        | (Some(PathEl::LineTo(_)), Some(PathEl::LineTo(_)))
                        | (Some(PathEl::QuadTo(..)), Some(PathEl::QuadTo(..)))
                        | (Some(PathEl::CurveTo(..)), Some(PathEl::CurveTo(..)))
                        | (Some(PathEl::ClosePath), Some(PathEl::ClosePath))
                )
            });
            if !same_kind {
                return Err(bad(format!("contour {c} has a curve in one source where another has a line")));
            }
            if let Some(PathEl::CurveTo(..)) = els.first() {
                let cubics: Vec<CubicBez> = els
                    .iter()
                    .zip(&last)
                    .filter_map(|(el, &p0)| match *el {
                        PathEl::CurveTo(a, b, p) => Some(CubicBez::new(p0, a, b, p)),
                        _ => None,
                    })
                    .collect();
                let quads = curves_to_quadratic(&cubics, tolerance)
                    .ok_or_else(|| bad(format!("a curve of contour {c} cannot be made quadratic in every source at once")))?;
                for ((path, spline), (l, cubic)) in built.iter_mut().zip(quads).zip(last.iter_mut().zip(&cubics)) {
                    spline.iter().for_each(|q| path.quad_to(q.p1, q.p2));
                    *l = cubic.p3;
                }
                continue;
            }
            for ((path, el), (l, s)) in built.iter_mut().zip(&els).zip(last.iter_mut().zip(start.iter_mut())) {
                path.push(*el);
                match *el {
                    PathEl::MoveTo(p) => (*l, *s) = (p, p),
                    PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => *l = p,
                    PathEl::ClosePath => *l = *s,
                }
            }
        }
        for (slot, path) in out.iter_mut().zip(built) {
            slot.push(path);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn bowl(size: f64) -> BezPath {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.curve_to((0.0, size * 0.55), (size * 0.45, size), (size, size));
        p.line_to((size, 0.0));
        p.close_path();
        p
    }

    #[test]
    fn every_source_gets_the_same_points() {
        let out = quadratic_sources("o", &[vec![bowl(100.0)], vec![bowl(1000.0)]], 1.0).unwrap();
        let kinds = |p: &BezPath| p.elements().iter().map(std::mem::discriminant).collect::<Vec<_>>();
        assert_eq!(kinds(&out[0][0]), kinds(&out[1][0]));
        assert!(out[1][0].elements().iter().filter(|e| matches!(e, PathEl::QuadTo(..))).count() > 1);
        let mut line = BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((0.0, 100.0));
        line.line_to((100.0, 100.0));
        line.close_path();
        assert!(quadratic_sources("o", &[vec![bowl(100.0)], vec![line]], 1.0).is_err());
    }
}
