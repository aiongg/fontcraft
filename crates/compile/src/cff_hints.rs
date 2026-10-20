//! PostScript hints at export: per glyph, manual hints when the hint-source layer has any,
//! resolved through node links in the master being built; else the autohinter, unless Autohint
//! is off, the glyph is in Disable autohinting for glyphs, or Export PostScript Hints is off
//! (`docs/plans/M10.md` § 3 › PostScript export).
//!
//! Manual hints:
//!
//! | Step | Rule |
//! |---|---|
//! | source layer | [`fontcraft_hinting::ps::ps_hint_layer`]: the main master's layer (Get Hints From Master, else the first master), else the first other master's layer with hints; a special layer that replaces the master layer uses its own |
//! | edges | a node reference is read on the glyph's layer in the master being built, so hints follow the outline into every master; a number is used as it is |
//! | stems | made positive (lower edge first) and rounded as the charstring stores them; a top ghost written with width −20, a bottom ghost with −21; duplicates dropped; beyond [`MAX_STEM_HINTS`] the first are kept, with a warning; then a stem whose numbers a charstring cannot hold ([`writable_stems`]) left out with a warning |
//! | hint replacement | only when two stems of one direction overlap: masks are built along the outline (see [`masks`]) |
//! | autohinting | skipped for a glyph whose hint-source layer has any PostScript hint, even when none of them can be written |
//! | flex | what the autohinter finds on the outline, unless one of the glyph's masks falls between the two curves |
//!
//! The autohinter ([`autohinted`]) runs on the outline as the charstring draws it, against the
//! zones and stems of the Private dict being written ([`autohint_dict`]), with otfautohint's
//! defaults: what otfautohint gives on the unhinted font. Its stems, hint masks, counter masks
//! and flex are written as it gives them; a result the charstring cannot hold leaves the glyph
//! unhinted, with a warning.
//! Where its masks go in the charstring follows otfautohint's charstring writer (AFDKO 5.0.1
//! `otfautohint/glyphData.py`, Apache-2.0, Copyright 2021 Adobe; see `NOTICE`).
//!
//! A hint whose node the built master's layer lacks takes the node's position on the layer the
//! hint is stored on, with a warning (guess on `hinting.ps.masters.transferByNodeLink`); one that
//! cannot be placed there either is left out. Rows:
//! `hinting.ps.export.*`, `hinting.ps.masters.*` (export), `export.autohintCff`,
//! `hinting.ps.export.autohintOption`, `hinting.ps.export.autohintParameter`.

use fontcraft_autohint::{ElementPos, FdDict, GlyphHints, HintInput, HintMask, Options, StemSet, Zone, ZoneSet, hint_glyph};
use fontcraft_cff::{CharstringHints, MAX_STEM_HINTS, Mask, Number, PathOp, PrivateDict, StemHint, writable_stems};
use fontcraft_geom::{BezPath, PathEl, Point};
use fontcraft_hinting::ps::ps_hint_layer;
use fontcraft_hinting::{HintSettings, PsHintKind, ps_hints};
use fontcraft_model::{Font, ResolvedGlyph, Target};

use crate::cff::ROUND_TOLERANCE;
use crate::{CompileError, ot_round};

/// How far a node may lie from a stem edge and still count as on it, in font units: charstring
/// coordinates are rounded to whole units.
const EDGE_TOLERANCE: f64 = 0.5;

/// The zones and stems of the Private dict `private` in the autohinter's validated form, read
/// as otfautohint reads them from a compiled font (StemSnap first, then StdHW/StdVW). Identical
/// zones are merged first: the Private dict keeps the duplicates the source has, which the
/// autohinter would reject as overlapping. Zones or stems the autohinter cannot use leave it
/// running without them, with a warning; `None` (with a warning) only when even that fails.
pub fn autohint_dict(private: &PrivateDict, name: &str, warnings: &mut Vec<String>) -> Option<FdDict> {
    let value = |n: &Number| match *n {
        Number::Int(i) => i as f64,
        Number::Real(r) => r,
    };
    let zones = |list: &[Number]| -> Vec<Zone> {
        let mut out: Vec<Zone> = Vec::new();
        for [a, b] in list.as_chunks::<2>().0 {
            let zone = Zone { bottom: value(a), top: value(b), is_bottom: false, name: String::new() };
            if !out.iter().any(|z| z.bottom == zone.bottom && z.top == zone.top) {
                out.push(zone);
            }
        }
        out
    };
    let set = ZoneSet {
        blue_values: zones(&private.blue_values),
        other_blues: zones(&private.other_blues),
        family_blues: zones(&private.family_blues),
        family_other_blues: zones(&private.family_other_blues),
        blue_fuzz: value(&private.blue_fuzz),
    };
    let stems = |snap: &[Number], std: Option<&Number>| -> Vec<f64> {
        if snap.is_empty() { std.map(value).into_iter().collect() } else { snap.iter().map(value).collect() }
    };
    let stems = StemSet { h: stems(&private.stem_snap_h, private.std_hw.as_ref()), v: stems(&private.stem_snap_v, private.std_vw.as_ref()) };
    let first = match FdDict::new(name, &set, &stems, false) {
        Ok(d) => return Some(d),
        Err(e) => e,
    };
    // As otfautohint's allowNoBlues: missing zones or stems replaced by inactive ones, then the
    // zones left out, then the stems too.
    let fallbacks = [(&set, &stems), (&ZoneSet::default(), &stems), (&ZoneSet::default(), &StemSet::default())];
    for (zones, stems) in fallbacks {
        if let Ok(d) = FdDict::new(name, zones, stems, true) {
            warnings.push(format!("PostScript autohinting: {first}; the glyphs were autohinted without what cannot be used"));
            return Some(d);
        }
    }
    warnings.push(format!("PostScript autohinting is not possible: {first}"));
    None
}

/// The autohinter's zones and stems for master `master` of `font`: those of the Private dict an
/// export of the master writes ([`autohint_dict`]). Warnings go to `warnings`; the error says why
/// the master's settings cannot be read.
pub fn master_autohint_dict(font: &Font, master: &str, warnings: &mut Vec<String>) -> Result<Option<FdDict>, String> {
    let resolved = font.resolve(&Target::Master(master.to_string())).map_err(|e| e.to_string())?;
    let settings = HintSettings::for_master(font, master).map_err(|e| e.to_string())?;
    let private = crate::cff::private_dict(&resolved, Some(&settings), warnings);
    Ok(autohint_dict(&private, master, warnings))
}

/// The charstring hints of `glyph`, whose outline as prepared for CFF is `path`, or `None` for
/// an unhinted charstring. `font` is the source the glyph was resolved from and `settings` the
/// hint settings of the master or instance being built; `dict` is the autohinter's zones and
/// stems, and `autohint` says whether autohinting is on. A hand-hinted glyph gets the flex the
/// autohinter finds on its outline, where no mask of its own falls inside one. Warnings go to
/// `warnings`.
pub fn glyph_hints(
    font: &Font,
    settings: &HintSettings,
    glyph: &ResolvedGlyph,
    path: &mut Vec<PathOp>,
    dict: Option<&FdDict>,
    autohint: bool,
    warnings: &mut Vec<String>,
) -> Result<Option<CharstringHints>, CompileError> {
    if !settings.export_ps_hints {
        return Ok(None);
    }
    if let Some(mut manual) = manual_hints(font, settings, &glyph.name, path, warnings) {
        if manual.is_empty() {
            return Ok(None);
        }
        if let Some(dict) = dict {
            let found = autohinted(&glyph.name, path, dict, &Options::default(), &mut Vec::new()).map(|h| h.flex).unwrap_or_default();
            manual.flex = found.into_iter().filter(|f| !manual.masks.iter().any(|(op, _)| *op == f + 1)).collect();
            if manual.validate(&glyph.name, path).is_err() {
                manual.flex.clear();
            }
        }
        return Ok(Some(manual));
    }
    let Some(dict) = dict.filter(|_| autohint) else { return Ok(None) };
    if settings.disable_autohint_glyphs.contains(&glyph.name) {
        return Ok(None);
    }
    let drawn = charstring_path(path);
    let hints = autohinted(&glyph.name, &drawn, dict, &Options::default(), warnings);
    if hints.is_some() {
        *path = drawn;
    }
    Ok(hints)
}

/// `path` as the charstring draws it, which is what the autohinter must see to hint as the
/// reference does on a compiled font: coordinates rounded as the encoder rounds them, then the
/// steps the specialiser removes (`fontcraft_cff`'s pass 3): a curve whose first and last
/// handles lie on its ends becomes a line, a zero-length line goes, and a horizontal (or
/// vertical) step right after a line of the same direction is joined to it. Successive movetos keep the last.
pub fn charstring_path(path: &[PathOp]) -> Vec<PathOp> {
    charstring_path_mapped(path).0
}

/// [`charstring_path`], with, per step of the result, the index in `path` of the first step it
/// was made from (a dropped step counts with the step after it).
pub fn charstring_path_mapped(path: &[PathOp]) -> (Vec<PathOp>, Vec<usize>) {
    let r = |(x, y): (f64, f64)| (ot_round(x), ot_round(y));
    let mut out: Vec<PathOp> = Vec::with_capacity(path.len());
    let mut starts: Vec<usize> = Vec::with_capacity(path.len());
    // The first step of `path` not yet part of a step of `out`.
    let mut pending = 0usize;
    let mut current = (0.0, 0.0);
    // The direction of the last line drawn, when the step before this one was that line.
    let mut last_line: Option<bool> = None;
    for (i, op) in path.iter().enumerate() {
        let step = match *op {
            PathOp::MoveTo(p) => {
                let p = r(p);
                if let Some(PathOp::MoveTo(prev)) = out.last_mut() {
                    *prev = p;
                } else {
                    out.push(PathOp::MoveTo(p));
                    starts.push(pending);
                }
                pending = i + 1;
                current = p;
                last_line = None;
                continue;
            }
            PathOp::LineTo(p) => PathOp::LineTo(r(p)),
            PathOp::CurveTo(a, b, p) => {
                let (a, b, p) = (r(a), r(b), r(p));
                if a == current && b == p { PathOp::LineTo(p) } else { PathOp::CurveTo(a, b, p) }
            }
        };
        let was_line = matches!(op, PathOp::LineTo(_));
        let PathOp::LineTo(p) = step else {
            out.push(step);
            starts.push(pending);
            pending = i + 1;
            current = end(&step);
            last_line = None;
            continue;
        };
        if p == current {
            // Dropped; the line before it is no longer right before the next step.
            last_line = None;
            continue;
        }
        let horizontal = p.1 == current.1;
        let axis = (horizontal || p.0 == current.0).then_some(horizontal);
        match (axis, last_line, out.last_mut()) {
            (Some(h), Some(prev), Some(PathOp::LineTo(prev_end))) if h == prev => *prev_end = p,
            _ => {
                out.push(PathOp::LineTo(p));
                starts.push(pending);
            }
        }
        pending = i + 1;
        current = p;
        last_line = if was_line { axis } else { None };
    }
    (out, starts)
}

fn end(op: &PathOp) -> (f64, f64) {
    match *op {
        PathOp::MoveTo(p) | PathOp::LineTo(p) | PathOp::CurveTo(_, _, p) => p,
    }
}

/// [`autohinted`] on `path` as the static charstring draws it ([`charstring_path_mapped`]), with
/// each mask moved before the first step of `path` that the drawn step it names was made from, so
/// that it names a step of `path`. Flex is left out: it names curves of the drawn path.
pub fn autohinted_on_steps(name: &str, path: &[PathOp], dict: &FdDict, options: &Options, warnings: &mut Vec<String>) -> Option<CharstringHints> {
    let (drawn, starts) = charstring_path_mapped(path);
    let mut hints = autohinted(name, &drawn, dict, options, warnings)?;
    hints.flex.clear();
    for (op, _) in &mut hints.masks {
        *op = if *op == 0 { 0 } else { starts.get(*op).copied().unwrap_or(path.len()) };
    }
    hints.masks.retain(|(op, _)| *op < path.len().max(1));
    Some(hints)
}

/// The autohinter's hints for the outline `path` of glyph `name`, as charstring hints, or `None`
/// (with a warning when the autohinter failed or its result cannot be written).
pub fn autohinted(name: &str, path: &[PathOp], dict: &FdDict, options: &Options, warnings: &mut Vec<String>) -> Option<CharstringHints> {
    let (contours, starts) = contours(path);
    let input = HintInput { name, contours: &contours, without_overlap: None, dict, options };
    let hints = match hint_glyph(&input) {
        Ok(h) => h,
        Err(e) => {
            warnings.push(format!("glyph `{name}`: the PostScript autohinter failed ({e}); it was exported without hints"));
            return None;
        }
    };
    if hints.is_empty() {
        return None;
    }
    let out = charstring_hints(&hints, &starts, path.len());
    let only = |s: &[StemHint]| writable_stems(s, ROUND_TOLERANCE).into_iter().all(|ok| ok);
    if let Err(e) = out.validate(name, path).map_err(|e| e.to_string()).and_then(|()| {
        if only(&out.h_stems) && only(&out.v_stems) { Ok(()) } else { Err("a stem lies beyond the numbers a charstring can hold".into()) }
    }) {
        warnings.push(format!("glyph `{name}`: the PostScript autohinter's hints cannot be written ({e}); it was exported without hints"));
        return None;
    }
    Some(out)
}

/// The closed contours `path` draws, as the autohinter reads a charstring, and the index in
/// `path` of each contour's moveto. A moveto with nothing drawn after it starts no contour.
pub fn contours(path: &[PathOp]) -> (Vec<BezPath>, Vec<usize>) {
    let (mut out, mut starts) = (Vec::new(), Vec::new());
    let mut current: Vec<PathEl> = Vec::new();
    let mut at = 0;
    let mut finish = |current: &mut Vec<PathEl>, at: usize| {
        if current.len() > 1 {
            current.push(PathEl::ClosePath);
            out.push(BezPath::from_vec(std::mem::take(current)));
            starts.push(at);
        }
        current.clear();
    };
    let pt = |(x, y): (f64, f64)| Point::new(x, y);
    for (i, op) in path.iter().enumerate() {
        match *op {
            PathOp::MoveTo(p) => {
                finish(&mut current, at);
                current.push(PathEl::MoveTo(pt(p)));
                at = i;
            }
            PathOp::LineTo(p) => current.push(PathEl::LineTo(pt(p))),
            PathOp::CurveTo(a, b, p) => current.push(PathEl::CurveTo(pt(a), pt(b), pt(p))),
        }
    }
    finish(&mut current, at);
    (out, starts)
}

/// The autohinter's result as charstring hints for a path of `ops` operations whose contours
/// start at `starts`. An element names the operation after its contour's moveto; a mask on a
/// contour's implicit closing line goes before that contour's own moveto (the hinter walks each
/// contour from its closing line), after the start mask for the first contour. The mask on the
/// second curve of a flex is dropped, as the reference does.
fn charstring_hints(hints: &GlyphHints, starts: &[usize], ops: usize) -> CharstringHints {
    let stem = |s: &fontcraft_autohint::Stem| StemHint { edge: s.lb, width: s.rt - s.lb };
    let mask = |m: &HintMask| Mask { h: m.h.clone(), v: m.v.clone() };
    let op_of = |pos: &ElementPos| -> Option<usize> {
        let start = *starts.get(pos.subpath)?;
        let end = starts.get(pos.subpath + 1).copied().unwrap_or(ops);
        let op = start.checked_add(1)?.checked_add(pos.element)?;
        if op < end { Some(op) } else { Some(start) }
    };
    let mut out = CharstringHints {
        h_stems: hints.h.iter().map(stem).collect(),
        v_stems: hints.v.iter().map(stem).collect(),
        counters: hints.counters.iter().map(mask).collect(),
        flex: hints.flex.iter().filter_map(op_of).collect(),
        masks: Vec::new(),
    };
    if !hints.masks.is_empty() || hints.start_mask.is_some() {
        let all = Mask { h: vec![true; out.h_stems.len()], v: vec![true; out.v_stems.len()] };
        out.masks.push((0, hints.start_mask.as_ref().map(mask).unwrap_or(all)));
        let mut placed: Vec<(usize, Mask)> = hints
            .masks
            .iter()
            .filter_map(|(pos, m)| op_of(pos).map(|op| (op, mask(m))))
            .filter(|(op, _)| !out.flex.iter().any(|f| f + 1 == *op))
            .collect();
        placed.sort_by_key(|(op, _)| *op);
        for (op, m) in placed {
            match out.masks.last_mut() {
                Some((last, slot)) if *last == op => *slot = m,
                _ => out.masks.push((op, m)),
            }
        }
    }
    out
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
    // Each stem with the index of the hint it comes from, for warnings.
    let (mut h, mut v) = (Vec::<(usize, StemHint)>::new(), Vec::<(usize, StemHint)>::new());
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
        // Rounded as the encoder writes it, so duplicates and the overlaps that decide the masks
        // are those of the charstring.
        let stem = match hint.kind {
            PsHintKind::Stem => StemHint::between(a, b),
            PsHintKind::GhostTop => StemHint::top_ghost(a),
            PsHintKind::GhostBottom => StemHint::bottom_ghost(a),
        }
        .normalised(ROUND_TOLERANCE);
        let list = if hint.horizontal { &mut h } else { &mut v };
        if !list.iter().any(|(_, s)| *s == stem) {
            list.push((index, stem));
        }
    }
    // Truncated first: leaving stems out changes the offsets the stems after them are written at,
    // so the stems written are the ones checked.
    let total = h.len() + v.len();
    if total > MAX_STEM_HINTS {
        warnings
            .push(format!("glyph `{name}`: {total} PostScript stem hints, more than the {MAX_STEM_HINTS} a charstring holds; the first were kept"));
        h.truncate(MAX_STEM_HINTS);
        v.truncate(MAX_STEM_HINTS - h.len());
    }
    let mut out = CharstringHints::default();
    for (list, stems) in [(h, &mut out.h_stems), (v, &mut out.v_stems)] {
        let only: Vec<StemHint> = list.iter().map(|(_, s)| *s).collect();
        for ((index, stem), ok) in list.into_iter().zip(writable_stems(&only, ROUND_TOLERANCE)) {
            if ok {
                stems.push(stem);
            } else {
                warnings.push(format!("glyph `{name}`: PostScript hint {index} lies beyond the numbers a charstring can hold; it was left out"));
            }
        }
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

    #[test]
    fn autohinted_masks_move_to_the_steps_they_were_made_from() {
        use PathOp::{LineTo, MoveTo};
        // Two boxes whose horizontal stems overlap (0..80 and 40..120), so the hints need
        // replacing along the outline. `split` draws the first box's bottom in two lines and adds
        // a zero-length line, which the charstring joins and drops.
        let clean = [rect(0.0, 0.0, 200.0, 80.0), rect(400.0, 40.0, 600.0, 120.0)].concat();
        let mut split = vec![MoveTo((0.0, 0.0)), LineTo((100.0, 0.0)), LineTo((200.0, 0.0)), LineTo((200.0, 0.0)), LineTo((200.0, 80.0))];
        split.push(LineTo((0.0, 80.0)));
        split.extend(rect(400.0, 40.0, 600.0, 120.0));
        let (drawn, starts) = charstring_path_mapped(&split);
        assert_eq!(drawn, clean);
        // The joined line starts at step 1; the dropped step goes with the line after it.
        assert_eq!(starts, vec![0, 1, 3, 5, 6, 7, 8, 9]);

        let zones = ZoneSet {
            blue_values: vec![Zone { bottom: -10.0, top: 0.0, ..Default::default() }, Zone { bottom: 700.0, top: 710.0, ..Default::default() }],
            blue_fuzz: 1.0,
            ..Default::default()
        };
        let dict = FdDict::new("t", &zones, &StemSet { h: vec![80.0], v: vec![80.0] }, false).unwrap();
        let mut warnings = Vec::new();
        let direct = autohinted("boxes", &clean, &dict, &Options::default(), &mut warnings).unwrap();
        assert!(direct.masks.len() >= 2, "the overlap needs hint replacement: {direct:?}");
        let moved = autohinted_on_steps("boxes", &split, &dict, &Options::default(), &mut warnings).unwrap();
        assert_eq!((&moved.h_stems, &moved.v_stems), (&direct.h_stems, &direct.v_stems));
        let expected: Vec<(usize, Mask)> = direct.masks.iter().map(|(op, m)| (starts[*op], m.clone())).collect();
        assert_eq!(moved.masks, expected);
        assert!(moved.masks.iter().all(|(op, _)| *op < split.len()));
    }
}
