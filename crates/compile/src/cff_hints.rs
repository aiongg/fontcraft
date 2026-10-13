//! PostScript hints at export: per glyph, manual hints when the hint-source layer has any,
//! resolved through node links in the master being built; else the autohinter, unless Autohint
//! is off, the glyph is in Disable autohinting for glyphs, or Export PostScript Hints is off
//! (`docs/plans/M10.md` § 3 › PostScript export). M10.15 adds the autohint stage.
//!
//! Manual hints:
//!
//! | Step | Rule |
//! |---|---|
//! | source layer | [`fontcraft_hinting::ps::ps_hint_layer`]: the main master's layer (Get Hints From Master, else the first master), else the first other master's layer with hints; a special layer that replaces the master layer uses its own |
//! | edges | a node reference is read on the glyph's layer in the master being built, so hints follow the outline into every master; a number is used as it is |
//! | stems | made positive (lower edge first); a top ghost written with width −20, a bottom ghost with −21; duplicates dropped; beyond [`MAX_STEM_HINTS`] the first are kept, with a warning |
//! | hint replacement | only when two stems of one direction overlap: masks are built along the outline (see [`masks`]) |
//! | autohinting | skipped for a glyph whose hint-source layer has any PostScript hint, even when none of them can be written |
//!
//! A hint whose node the built master's layer lacks takes the node's position on the layer the
//! hint is stored on, with a warning (guess on `hinting.ps.masters.transferByNodeLink`); one that
//! cannot be placed there either is left out. Rows:
//! `hinting.ps.export.*`, `hinting.ps.masters.*` (export), `export.autohintCff`,
//! `hinting.ps.export.autohintOption`, `hinting.ps.export.autohintParameter`.

use fontcraft_autohint::FdDict;
use fontcraft_cff::{CharstringHints, MAX_STEM_HINTS, Mask, PathOp, StemHint};
use fontcraft_hinting::ps::ps_hint_layer;
use fontcraft_hinting::{HintSettings, PsHintKind, ps_hints};
use fontcraft_model::{Font, ResolvedGlyph};

use crate::CompileError;

/// How far a node may lie from a stem edge and still count as on it, in font units: charstring
/// coordinates are rounded to whole units.
const EDGE_TOLERANCE: f64 = 0.5;

/// The zones and stems of `settings` in the autohinter's validated form.
pub fn autohint_dict(settings: &HintSettings) -> Result<FdDict, CompileError> {
    // M10.15: ZoneSet and StemSet from the settings, then FdDict::new.
    let _ = settings;
    Ok(FdDict::default())
}

/// The charstring hints of `glyph`, whose outline as prepared for CFF is `path`, or `None` for
/// an unhinted charstring. `font` is the source the glyph was resolved from and `settings` the
/// hint settings of the master or instance being built. Warnings go to `warnings`.
pub fn glyph_hints(
    font: &Font,
    settings: &HintSettings,
    glyph: &ResolvedGlyph,
    path: &[PathOp],
    autohint: Option<&FdDict>,
    warnings: &mut Vec<String>,
) -> Result<Option<CharstringHints>, CompileError> {
    if !settings.export_ps_hints {
        return Ok(None);
    }
    if let Some(manual) = manual_hints(font, settings, &glyph.name, path, warnings) {
        return Ok(Some(manual).filter(|h| !h.is_empty()));
    }
    // M10.15: the autohinter when `autohint` is given.
    let _ = autohint;
    Ok(None)
}

/// The glyph's manual hints resolved in the master being built, or `None` when its hint-source
/// layer has none (the autohinter may then hint it).
pub fn manual_hints(font: &Font, settings: &HintSettings, name: &str, path: &[PathOp], warnings: &mut Vec<String>) -> Option<CharstringHints> {
    let glyph = font.glyph(name)?;
    let built = glyph.layer(&settings.master)?;
    let source = ps_hint_layer(font, glyph, settings, built)?;
    let hints = match ps_hints(source) {
        Ok(h) => h,
        Err(e) => {
            warnings.push(format!("glyph `{name}`: its PostScript hints cannot be read ({e}); it was exported without hints"));
            return Some(CharstringHints::default());
        }
    };
    let mut out = CharstringHints::default();
    for (index, hint) in hints {
        let (a, b) = match hint.edges(built) {
            Ok(e) => e,
            Err(e) => match hint.edges(source) {
                Ok(fallback) => {
                    warnings.push(format!("glyph `{name}`: PostScript hint {index} took its position from the layer it is stored on: {e}"));
                    fallback
                }
                Err(e) => {
                    warnings.push(format!("glyph `{name}`: PostScript hint {index} was left out: {e}"));
                    continue;
                }
            },
        };
        let stem = match hint.kind {
            PsHintKind::Stem => StemHint::between(a, b),
            PsHintKind::GhostTop => StemHint::top_ghost(a),
            PsHintKind::GhostBottom => StemHint::bottom_ghost(a),
        };
        let list = if hint.horizontal { &mut out.h_stems } else { &mut out.v_stems };
        if !list.contains(&stem) {
            list.push(stem);
        }
    }
    let total = out.h_stems.len() + out.v_stems.len();
    if total > MAX_STEM_HINTS {
        warnings
            .push(format!("glyph `{name}`: {total} PostScript stem hints, more than the {MAX_STEM_HINTS} a charstring holds; the first were kept"));
        out.h_stems.truncate(MAX_STEM_HINTS);
        out.v_stems.truncate(MAX_STEM_HINTS - out.h_stems.len());
    }
    out.masks = masks(&out.h_stems, &out.v_stems, path);
    Some(out)
}

/// Hint masks for `h` and `v` along `path`, empty when no two stems of one direction overlap.
///
/// Each path operation turns on the stems an edge of which its end point lies on. The first mask
/// applies from the start; a new mask starts at the first operation that turns on a stem
/// overlapping one the current mask holds, and holds what the operations from there turn on.
/// Where one point turns on two overlapping stems, the first in source order wins. A stem no
/// point touches goes into every mask it fits.
pub fn masks(h: &[StemHint], v: &[StemHint], path: &[PathOp]) -> Vec<(usize, Mask)> {
    let conflicts = |stems: &[StemHint]| -> Vec<Vec<bool>> {
        stems.iter().enumerate().map(|(i, a)| stems.iter().enumerate().map(|(j, b)| i != j && a.overlaps(b)).collect()).collect()
    };
    let (ch, cv) = (conflicts(h), conflicts(v));
    if !ch.iter().chain(&cv).flatten().any(|c| *c) {
        return Vec::new();
    }
    let empty = || Mask { h: vec![false; h.len()], v: vec![false; v.len()] };
    // Whether stem `k` clashes with a stem `on` holds.
    let clash = |c: &[Vec<bool>], on: &[bool], k: usize| c.get(k).is_some_and(|row| row.iter().zip(on).any(|(x, o)| *x && *o));
    let mut touched_any = (vec![false; h.len()], vec![false; v.len()]);
    let mut out: Vec<(usize, Mask)> = vec![(0, empty())];
    for (i, op) in path.iter().enumerate() {
        let (x, y) = match *op {
            PathOp::MoveTo(p) | PathOp::LineTo(p) | PathOp::CurveTo(_, _, p) => p,
        };
        let th = first_wins(on_edge(h, y), &ch);
        let tv = first_wins(on_edge(v, x), &cv);
        for (list, touched) in [(&th, &mut touched_any.0), (&tv, &mut touched_any.1)] {
            for &k in list {
                if let Some(t) = touched.get_mut(k) {
                    *t = true;
                }
            }
        }
        let Some((at, current)) = out.last() else { break };
        let needs_new = th.iter().any(|&k| !current.h.get(k).copied().unwrap_or(false) && clash(&ch, &current.h, k))
            || tv.iter().any(|&k| !current.v.get(k).copied().unwrap_or(false) && clash(&cv, &current.v, k));
        if needs_new && i > *at {
            out.push((i, empty()));
        }
        let Some((_, m)) = out.last_mut() else { break };
        for k in th {
            if !clash(&ch, &m.h, k)
                && let Some(b) = m.h.get_mut(k)
            {
                *b = true;
            }
        }
        for k in tv {
            if !clash(&cv, &m.v, k)
                && let Some(b) = m.v.get_mut(k)
            {
                *b = true;
            }
        }
    }
    for (_, m) in &mut out {
        for (touched, c, on) in [(&touched_any.0, &ch, &mut m.h), (&touched_any.1, &cv, &mut m.v)] {
            for (k, t) in touched.iter().enumerate() {
                if !t
                    && !clash(c, on, k)
                    && let Some(b) = on.get_mut(k)
                {
                    *b = true;
                }
            }
        }
    }
    out.dedup_by(|later, earlier| later.1 == earlier.1);
    out
}

/// `stems` without those that clash with an earlier one of them.
fn first_wins(stems: Vec<usize>, conflicts: &[Vec<bool>]) -> Vec<usize> {
    let mut kept: Vec<usize> = Vec::with_capacity(stems.len());
    for k in stems {
        if !kept.iter().any(|&j| conflicts.get(j).and_then(|row| row.get(k)).copied().unwrap_or(false)) {
            kept.push(k);
        }
    }
    kept
}

/// The stems with an edge at `at`.
fn on_edge(stems: &[StemHint], at: f64) -> Vec<usize> {
    stems
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            let (a, b) = s.edges();
            (a - at).abs() <= EDGE_TOLERANCE || (b - at).abs() <= EDGE_TOLERANCE
        })
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<PathOp> {
        vec![PathOp::MoveTo((x0, y0)), PathOp::LineTo((x1, y0)), PathOp::LineTo((x1, y1)), PathOp::LineTo((x0, y1))]
    }

    #[test]
    fn overlapping_stems_are_split_into_masks_along_the_outline() {
        // Two boxes one above the other whose horizontal stems overlap: 0..300 and 200..500.
        let mut path = rect(0.0, 0.0, 100.0, 300.0);
        path.extend(rect(0.0, 200.0, 100.0, 500.0));
        let h = [StemHint::between(0.0, 300.0), StemHint::between(200.0, 500.0), StemHint::top_ghost(900.0)];
        let v = [StemHint::between(0.0, 100.0)];
        let m = masks(&h, &v, &path);
        let on = |h: [bool; 3]| Mask { h: h.to_vec(), v: vec![true] };
        assert_eq!(m, vec![(0, on([true, false, true])), (4, on([false, true, true]))]);
        // Two ghosts at one point: the first wins and no second, equal mask follows.
        let ghosts = [StemHint::top_ghost(300.0), StemHint::bottom_ghost(300.0)];
        assert_eq!(masks(&ghosts, &[], &path), vec![(0, Mask { h: vec![true, false], v: vec![] })]);
        // Without an overlap: no masks.
        assert!(masks(&h[..1], &v, &path).is_empty());
    }
}
