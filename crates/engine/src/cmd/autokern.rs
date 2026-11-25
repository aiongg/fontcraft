//! Auto-kerning, Kern to Distance, group building, pair rules, vertical kerning. Rows:
//! `kerning.autoKern`, `kerning.kernToDistance`, `kerning.buildGroups`,
//! `kerning.removePairsByRule`, `kerning.verticalFromHorizontal`. Plan: `docs/plans/M13.md` § 3,
//! decision D11.
//!
//! # Measuring a pair
//! Pairs are in logical order. The glyphs stand in visual order (in right-to-left kerning the
//! second glyph stands left), the right one shifted by the left one's advance, without kerning.
//! A group side is measured on one member: the member named like the group, else its first
//! member in glyph order. Outlines are the measured ones (slanted back in an italic master), as
//! auto-spacing takes them.
//!
//! - The **optical gap** is [`profile::optical_gap`] between the left glyph's right profile and
//!   the right glyph's left profile, each clipped to `depth` (30% of the x-height by default)
//!   inward from its extreme: a mean of the row distances that leans toward the closest rows.
//!   Both are taken over the lowercase band (baseline to x-height, overshoots included), or over
//!   the cap-height band when both glyphs are capitals or figures: the bands of
//!   `spacing.autoSpace`. The depth is deeper than auto-spacing's, as the corpus measure
//!   (`docs/plans/M13.md` § 5) shows kerning needs more of a counter or a diagonal's white to
//!   tell pairs apart.
//! - The **closest approach** is the smallest horizontal distance between the two outlines at a
//!   height both reach: the minimum over rows at most one unit apart, refined between the
//!   neighbouring rows by a golden-section search on the outlines' exact crossings.
//!
//! # Auto Kern
//! value = target − gap, rounded to `round` units. The target is `separation`, else the mean of
//! the two glyphs' reference gaps: the gap of n n (lowercase) or H H (capitals and figures)
//! without kerning; a group of glyphs without its reference takes the other's. With `touching`,
//! gap and reference gaps are closest approaches.
//!
//! # Build Kerning Groups
//! Per side, each glyph's clipped profile becomes, per row, its depth inward from the extreme. Two
//! glyphs differ by the root mean square of the depth differences: rows where both have ink
//! compare depths; rows between the baseline and the x-height (cap height) where only one has
//! ink count as the full clipping depth; rows in the overshoot zones where only one has ink are
//! left out. Over several masters the largest difference counts. In glyph order each glyph joins
//! the group whose members all lie within `maxDifference` (the closest such group), else starts
//! one; only glyphs of the same band are compared.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_geom::{BezPath, Line, ParamCurve, ParamCurveExtrema, PathSeg, Point, Rect, Shape as _};
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Font, Glyph, KernGroups, MAX_COORDINATE, Master, MetricValue, glyph_props, metric};
use fontcraft_spacing::kerning::pairs::stored_pairs;
use fontcraft_spacing::kerning::{Direction, GroupSide, KernIndex, Position, Side, Step, Table, check_group_name, facing};
use fontcraft_spacing::profile::{self, Band, SideProfile};
use fontcraft_spacing::{self as spacing, SIDEBEARING_RANGE, metrics_source, round_half_up};
use serde_json::{Map, Value, json};

use super::autospace::{default_glyphs, master_ids, opt_in};
use super::common::master_metrics;
use super::shared::{MAX_KERN_VALUE, direction as any_direction};
use super::*;
use crate::params::{Params, TargetKind};

/// Most pairs one call measures or changes, over all its masters.
pub const MAX_PAIRS: usize = 1_000_000;
/// Most profile comparisons one call of Build Kerning Groups makes, counted once per master.
pub const MAX_GROUP_COMPARISONS: usize = 20_000_000;
/// Sampling step of the optical profiles by default, in units.
const DEFAULT_STEP: f64 = 5.0;
/// Clipping depth of Auto Kern by default, as a share of the master's x-height. On the corpus
/// measure, sign agreement rose from 15% up to about 25% and changed little beyond.
const KERN_DEPTH_SHARE: f64 = 0.30;
/// Clipping depth of Build Kerning Groups by default, as a share of the master's x-height.
const GROUP_DEPTH_SHARE: f64 = 0.15;
/// Auto-kerning values are rounded to this many units by default.
const DEFAULT_ROUND: f64 = 5.0;
/// Most rows of the fine profiles the closest approach samples.
const FINE_ROWS: f64 = 4_000.0;
/// Golden-section steps that refine the closest approach between two sampled rows.
const REFINE_STEPS: usize = 40;
/// The reference glyph of each band: lowercase, then capitals and figures.
const REFERENCES: [&str; 2] = ["n", "H"];
const LOWER: usize = 0;
const CAPS: usize = 1;

pub fn specs() -> Vec<CommandSpec> {
    let g: &'static [TargetKind] = &[TargetKind::Glyphs];
    vec![
        cmd!(
            "kerning.autoKern",
            "Auto Kern",
            ["Font", "Kerning"],
            None,
            "{pairs?: [[left, right]] (glyph names or \"@group\", logical order), glyphsLeft?: [names], glyphsRight?: [names] (every pair of a first glyph from glyphsLeft and a second from glyphsRight; give pairs, both lists, or both), masters?: [master ids] (default: every master), direction?: \"ltr\" | \"rtl\" (default ltr), separation?: target gap in units (default: the gap of n n for lowercase and H H for capitals and figures, averaged over the pair's two glyphs), minKern?: values smaller than this in size are dropped (default 0), onlyCloser?: bool, drop values that would loosen a pair (default false), touching?: bool, measure gaps at the outlines' closest approach instead of optically (default false), keepExisting?: bool, leave pairs that are stored already (default true), round?: units the values are rounded to (default 5), step?: units between profile samples (default 5), depth?: units each profile is clipped to inward from its extreme (default 30% of the x-height), apply?: bool, store the values (default true; false only proposes)} propose and store kerning for each pair from the optical gap between the two glyphs: target minus the distance between the left glyph's right profile and the right glyph's left profile, averaged over the rows so the closest rows count most, over the band from the baseline to the x-height (cap height when both are capitals or figures), overshoots included. A group side is measured on the member named like the group, else its first member. A value of 0, below minKern, or positive with onlyCloser stores nothing, and removes the stored pair when keepExisting is false. Masters that take their kerning from another are skipped → {pairs: [{left, right, master, value, gap, target, previous, result: \"set\" | \"kept\" | \"dropped\" | \"removed\"}], targets: {master id: {lowercase, capitals}}, skipped: [{left?, right?, master, reason}]}",
            has_doc,
            auto_kern
        ),
        cmd!(
            "kerning.kernToDistance",
            "Kern to Distance",
            ["Font", "Kerning"],
            None,
            "{distance: units between the outlines at their closest, pairs?: [[left, right]] (glyph names or \"@group\", logical order), glyphsLeft?: [names], glyphsRight?: [names] (every pair of the two lists; default without pairs or lists: every pair stored in the master), masters?: [master ids] (default: every master), direction?: \"ltr\" | \"rtl\" (default ltr), apply?: bool (default true; false only proposes)} set each pair so the smallest horizontal distance between the two outlines, at any height both reach, equals the distance: the value is the distance minus that gap without kerning, rounded to a whole unit. A group side is measured on the member named like the group, else its first member. A value of 0 is stored only where another pair would otherwise apply. Masters that take their kerning from another are skipped → {pairs: [{left, right, master, value, gap, previous}], skipped: [{left?, right?, master, reason}]}",
            has_doc,
            kern_to_distance
        ),
        cmd!(
            "kerning.buildGroups",
            "Build Kerning Groups…",
            ["Font", "Kerning"],
            None,
            "{maxDifference: units, glyphs?: [names] (default: every exported letter and figure), masters?: [master ids] (default: every master; the largest difference counts), side?: \"left\" | \"right\" | \"both\" (default both), replaceExisting?: bool (default false: glyphs that have a group on a side keep it), step?: units between profile samples (default 5), depth?: units each profile is clipped to (default 15% of the x-height), apply?: bool (default false: only propose)} group glyphs whose side profiles match: per row, each profile's depth inward from its extreme (clipped); two glyphs differ by the root mean square of the depth differences over the band (rows inside the x-height or cap height where only one has ink count as the full depth). In glyph order each glyph joins the closest group whose members all differ from it by at most maxDifference, else starts one. Lowercase and capitals with figures are never mixed. Groups of two or more glyphs are named after their first member (with `.2`, `.3`… when that name is taken); a glyph left alone keeps what it has. Locked glyphs are skipped → {groups: {left?: [{name, glyphs}], right?: [{name, glyphs}]}, glyphs: [names changed], skipped: [{glyph, side?, reason}]}",
            has_doc,
            build_groups
        )
        .targets(g),
        cmd!(
            "kerning.removePairsByRule",
            "Remove Pairs…",
            ["Font", "Kerning"],
            None,
            "{smallerThan?: units, exceptionsWithin?: units (give one or both), glyphs?: [names] (only pairs naming one of them, or a group one of them belongs to on the side facing the pair), masters?: [master ids] (default: every master), direction?: \"ltr\" | \"rtl\" | \"vertical\" (default ltr)} delete stored pairs whose value is smaller in size than smallerThan, and exceptions whose value differs from their class value by at most exceptionsWithin. An exception names a glyph that has a group on the side facing the pair; its class value is what applies once it is gone (0 when nothing does). Every pair is judged on the kerning as it was before the call. Masters that take their kerning from another are skipped → {removed: [{master, left, right, value, rule: \"smallerThan\" | \"exceptionsWithin\", classValue?}], skipped: [{master, reason}]}",
            has_doc,
            remove_pairs_by_rule
        ),
        cmd!(
            "kerning.verticalFromHorizontal",
            "Vertical Kerning from Horizontal",
            ["Font", "Kerning"],
            None,
            "{masters?: [master ids] (default: every master)} delete each master's vertical kerning, then for every left-to-right pair whose glyphs both have a vertical alternate (the glyph named with the `.vert` suffix) store a vertical pair between the alternates, the first glyph's on top, with the kerning that applies between the two glyphs. Group sides stand for each of their members. Masters that take their kerning from another are skipped → {masters: {master id: {cleared, created}}, pairs: [{master, top, bottom, value}], skipped: [{master, reason}]}",
            has_doc,
            vertical_from_horizontal
        ),
    ]
}

// ── Shared ─────────────────────────────────────────────────────────────────────────────────────

/// Why `master` cannot have kerning of its own, when it takes it from another master.
pub(super) fn linked(font: &Font, m: &Master) -> Option<String> {
    let source = metrics_source(font, &m.id)?;
    (source.id != m.id).then(|| format!("master `{}` takes its kerning from master `{}`", m.name, source.name))
}

/// `direction` for the commands that measure outlines side by side.
pub(super) fn horizontal(p: &Params) -> Result<Direction> {
    let d = any_direction(p)?;
    if d == Direction::Vertical {
        return Err(p.bad("`direction` must be \"ltr\" or \"rtl\": outlines are measured side by side"));
    }
    Ok(d)
}

/// Capitals and figures use the cap-height band, the rest the x-height band.
fn band_of(g: &Glyph, data: &GlyphData) -> usize {
    let props = glyph_props(g, data, false);
    if props.case.as_deref() == Some("upper") || props.category.as_deref() == Some("Number") { CAPS } else { LOWER }
}

/// The sampled band from the baseline less its overshoot to `height` plus its overshoot, and
/// the core between the baseline and `height` themselves.
fn band(font: &Font, m: &Master, height: &str, default: f64) -> (Band, (f64, f64)) {
    let base = font.metric(m, metric::BASELINE).unwrap_or_default();
    let top = font.metric(m, height).unwrap_or(MetricValue { pos: default, over: 0.0 });
    let bottom = base.pos + base.over.min(0.0);
    let band = Band { bottom, top: (top.pos + top.over.max(0.0)).max(bottom) };
    (band, (base.pos.min(top.pos), base.pos.max(top.pos)))
}

/// Rounds `v` to a multiple of `unit`, halves upward.
fn round_to(v: f64, unit: f64) -> f64 {
    round_half_up(v / unit) * unit
}

/// A command side: a glyph of the font, or a group some glyph has on the side facing `position`.
fn read_side(p: &Params, font: &Font, index: &KernIndex, text: &str, at: &str, direction: Direction, position: Position) -> Result<Side> {
    let side = Side::parse(text).map_err(|e| p.bad(format!("{at}: {e}")))?;
    match &side {
        Side::Glyph(n) if font.glyph(n).is_none() => Err(p.bad(format!("{at}: there is no glyph `{n}`"))),
        Side::Group(g) if index.members(facing(direction, position), g).next().is_none() => {
            Err(p.bad(format!("{at}: no glyph has the {} kerning group `{g}`", facing(direction, position).name())))
        }
        _ => Ok(side),
    }
}

/// The pairs `pairs`, `glyphsLeft` and `glyphsRight` name, repeats dropped; `None` when none of
/// them is given.
pub(super) fn read_pairs(p: &Params, font: &Font, index: &KernIndex, direction: Direction) -> Result<Option<Vec<(Side, Side)>>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut given = false;
    if let Some(list) = p.opt_array("pairs")? {
        given = true;
        if list.len() > MAX_PAIRS {
            return Err(p.bad(format!("`pairs` holds {} pairs; at most {MAX_PAIRS} are allowed", list.len())));
        }
        for (i, item) in list.iter().enumerate() {
            let Some([Value::String(l), Value::String(r)]) = item.as_array().map(Vec::as_slice) else {
                return Err(p.bad(format!("`pairs` item {i} must be a list of two sides, each a glyph name or \"@group\"")));
            };
            let at = format!("`pairs` item {i}");
            let first = read_side(p, font, index, l, &at, direction, Position::First)?;
            let second = read_side(p, font, index, r, &at, direction, Position::Second)?;
            if seen.insert((first.clone(), second.clone())) {
                out.push((first, second));
            }
        }
    }
    match (p.opt_glyphs("glyphsLeft", font)?, p.opt_glyphs("glyphsRight", font)?) {
        (None, None) => {}
        (Some(left), Some(right)) => {
            given = true;
            if left.len().saturating_mul(right.len()).saturating_add(out.len()) > MAX_PAIRS {
                return Err(p.bad(format!("`glyphsLeft` and `glyphsRight` make more than {MAX_PAIRS} pairs")));
            }
            for a in &left {
                for b in &right {
                    let pair = (Side::Glyph(a.clone()), Side::Glyph(b.clone()));
                    if seen.insert(pair.clone()) {
                        out.push(pair);
                    }
                }
            }
        }
        _ => return Err(p.bad("give both `glyphsLeft` and `glyphsRight`")),
    }
    Ok(given.then_some(out))
}

/// The glyph a side is measured on: the glyph itself, or the group member named like the group,
/// else the group's first member in glyph order.
fn representative(index: &KernIndex, side: &Side, direction: Direction, position: Position) -> Option<String> {
    match side {
        Side::Glyph(n) => Some(n.clone()),
        Side::Group(g) => {
            let facing = facing(direction, position);
            index.members(facing, g).find(|m| m == g).or_else(|| index.members(facing, g).next()).map(str::to_string)
        }
    }
}

/// One pair to measure: its sides in logical order, the glyphs they are measured on in logical
/// order, and the same two glyphs in visual order (`left`, `right`).
struct Job {
    first: Side,
    second: Side,
    first_glyph: String,
    second_glyph: String,
    left: String,
    right: String,
}

fn job(index: &KernIndex, first: Side, second: Side, direction: Direction) -> std::result::Result<Job, (Side, Side, String)> {
    let f = representative(index, &first, direction, Position::First);
    let s = representative(index, &second, direction, Position::Second);
    let (Some(f), Some(s)) = (f, s) else {
        return Err((first, second, "a group of the pair has no members".into()));
    };
    let (left, right) = if direction == Direction::Rtl { (s.clone(), f.clone()) } else { (f.clone(), s.clone()) };
    Ok(Job { first, second, first_glyph: f, second_glyph: s, left, right })
}

fn pair_skip(first: &Side, second: &Side, master: &str, reason: impl Into<String>) -> Value {
    json!({"left": first.command(), "right": second.command(), "master": master, "reason": reason.into()})
}

/// The pairs `master`'s own table stores in `direction`: by sides, their stored spelling and
/// value (the first in stored order when two spellings name the same sides).
/// Stored pairs by their sides: stored spelling of each side, and the value.
type Stored = HashMap<(Side, Side), (String, String, f64)>;

fn stored_by_sides(font: &Font, direction: Direction, master: &str) -> Stored {
    let mut out = HashMap::new();
    for lp in stored_pairs(font, direction, master) {
        out.entry((lp.left(), lp.right())).or_insert_with(|| (lp.first.to_string(), lp.second.to_string(), lp.value));
    }
    out
}

/// A change to one master's table, in stored spelling: a value to set, or `None` to remove.
struct Change {
    master: String,
    first: String,
    second: String,
    value: Option<f64>,
}

impl Change {
    /// Sets `first`, `second` in the spelling already stored, else in the spelling for their
    /// positions.
    fn set(stored: &Stored, direction: Direction, master: &str, first: &Side, second: &Side, value: f64) -> Change {
        let (f, s) = match stored.get(&(first.clone(), second.clone())) {
            Some((f, s, _)) => (f.clone(), s.clone()),
            None => (first.stored(direction, Position::First), second.stored(direction, Position::Second)),
        };
        Change { master: master.to_string(), first: f, second: s, value: Some(value) }
    }
}

fn apply_changes(font: &mut Font, direction: Direction, changes: &[Change]) {
    let table = direction.table_mut(Arc::make_mut(&mut font.kerning));
    for c in changes {
        match c.value {
            Some(v) => table.set(&c.master, &c.first, &c.second, v),
            None => {
                table.remove(&c.master, &c.first, &c.second);
            }
        }
    }
}

// ── Measuring ──────────────────────────────────────────────────────────────────────────────────

/// One glyph layer prepared for measuring.
struct Outline {
    path: BezPath,
    /// Each segment with its bounds, so a height tests only the segments that reach it.
    segs: Vec<(PathSeg, Rect)>,
    width: f64,
    y0: f64,
    y1: f64,
}

impl Outline {
    fn of(font: &Font, g: &Glyph, master: &str) -> std::result::Result<Outline, String> {
        let l = g.layers.iter().find(|l| l.id == master).ok_or("the glyph has no layer for this master")?;
        let path = spacing::measure::measured_outline(font, l).map_err(|e| e.to_string())?;
        if path.elements().is_empty() {
            return Err("the layer is empty".into());
        }
        let b = path.bounding_box();
        let inside = |v: f64| v.is_finite() && v.abs() <= MAX_COORDINATE;
        if ![b.x0, b.y0, b.x1, b.y1, l.width].into_iter().all(inside) {
            return Err(format!("the outline or width reaches beyond ±{MAX_COORDINATE}"));
        }
        let segs = path
            .segments()
            .filter_map(|seg| {
                let b = ParamCurveExtrema::bounding_box(&seg);
                [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()).then_some((seg, b))
            })
            .collect();
        Ok(Outline { path, segs, width: l.width, y0: b.y0, y1: b.y1 })
    }

    /// The leftmost and rightmost ink on the horizontal line at `y`.
    fn edges_at(&self, y: f64) -> Option<(f64, f64)> {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for (seg, b) in &self.segs {
            if !(b.y0..=b.y1).contains(&y) {
                continue;
            }
            let line = Line::new(Point::new(b.x0 - 1.0, y), Point::new(b.x1 + 1.0, y));
            for hit in seg.intersect_line(line) {
                let x = seg.eval(hit.segment_t).x;
                if x.is_finite() {
                    lo = lo.min(x);
                    hi = hi.max(x);
                }
            }
        }
        (lo <= hi).then_some((lo, hi))
    }
}

/// A glyph's left and right profiles.
type Profiles = (SideProfile, SideProfile);

struct Measured {
    outline: std::result::Result<Outline, String>,
    /// Clipped profiles per band: `None` until taken, `Some(None)` without ink in the band.
    clipped: [Option<Option<Profiles>>; 2],
    /// Profiles over the master's fine band, for closest approaches.
    fine: Option<Profiles>,
}

/// What one master needs to measure a set of glyphs.
pub(super) struct Kit {
    cores: [(f64, f64); 2],
    depth: f64,
    glyphs: HashMap<String, Measured>,
}

impl Kit {
    /// Takes the outlines of every glyph in `needs`; with `step`, their clipped profiles over
    /// the band each is paired with; with `fine`, profiles at most a unit apart over a band that
    /// holds every outline. `Err` when the bands cannot be sampled at `step`.
    pub(super) fn new(
        font: &Font,
        m: &Master,
        step: Option<f64>,
        depth: Option<f64>,
        depth_share: f64,
        needs: &[(String, usize)],
        fine: bool,
    ) -> std::result::Result<Kit, String> {
        let metrics = master_metrics(font, m);
        let (lower, lower_core) = band(font, m, metric::X_HEIGHT, metrics.x_height);
        let (caps, caps_core) = band(font, m, metric::CAP_HEIGHT, metrics.cap_height);
        let bands = [lower, caps];
        if let Some(step) = step {
            for b in bands {
                profile::profiles(&BezPath::new(), b, step).map_err(|e| format!("master `{}`: {e}", m.name))?;
            }
        }
        let depth = depth.unwrap_or((depth_share * metrics.x_height).max(0.0));
        let mut glyphs: HashMap<String, Measured> = HashMap::new();
        for (name, band_index) in needs {
            let entry = glyphs.entry(name.clone()).or_insert_with(|| Measured {
                outline: font.glyph(name).ok_or_else(|| "there is no such glyph".to_string()).and_then(|g| Outline::of(font, g, &m.id)),
                clipped: [None, None],
                fine: None,
            });
            let (Some(step), Ok(outline), Some(slot), Some(band)) =
                (step, &entry.outline, entry.clipped.get_mut(*band_index), bands.get(*band_index))
            else {
                continue;
            };
            if slot.is_some() {
                continue;
            }
            let (l, r) = profile::profiles(&outline.path, *band, step).map_err(|e| format!("master `{}`: {e}", m.name))?;
            *slot = Some((l.inked_rows() > 0).then(|| (profile::clip_depth(&l, depth), profile::clip_depth(&r, depth))));
        }
        if fine {
            let (lo, hi) = glyphs
                .values()
                .filter_map(|g| g.outline.as_ref().ok())
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), o| (lo.min(o.y0), hi.max(o.y1)));
            if lo <= hi {
                let band = Band { bottom: lo, top: hi };
                let step = ((hi - lo) / FINE_ROWS).max(profile::MIN_PROFILE_STEP);
                for g in glyphs.values_mut() {
                    if let Ok(o) = &g.outline {
                        g.fine = profile::profiles(&o.path, band, step).ok();
                    }
                }
            }
        }
        Ok(Kit { cores: [lower_core, caps_core], depth, glyphs })
    }

    fn outline(&self, name: &str) -> std::result::Result<(&Measured, &Outline), String> {
        let m = self.glyphs.get(name).ok_or_else(|| format!("glyph `{name}` was not measured"))?;
        let o = m.outline.as_ref().map_err(|e| format!("glyph `{name}`: {e}"))?;
        Ok((m, o))
    }

    fn clipped(&self, name: &str, band: usize) -> std::result::Result<(&Profiles, &Outline), String> {
        let (m, o) = self.outline(name)?;
        match m.clipped.get(band) {
            Some(Some(Some(p))) => Ok((p, o)),
            Some(Some(None)) => Err(format!("glyph `{name}` has no ink between the band's heights")),
            _ => Err(format!("glyph `{name}` was not measured over this band")),
        }
    }

    /// The optical gap between `left` and `right` standing side by side without kerning.
    fn optical(&self, left: &str, right: &str, band: usize) -> std::result::Result<f64, String> {
        let ((_, r), lo) = self.clipped(left, band)?;
        let ((l, _), _) = self.clipped(right, band)?;
        profile::optical_gap(r, l, lo.width).ok_or_else(|| format!("`{left}` and `{right}` have no height in common within the band"))
    }

    /// The smallest horizontal distance between `left` and `right` standing side by side
    /// without kerning, at a height both reach.
    pub(super) fn closest(&self, left: &str, right: &str) -> std::result::Result<f64, String> {
        let (ma, oa) = self.outline(left)?;
        let (mb, ob) = self.outline(right)?;
        let (Some((_, ra)), Some((lb, _))) = (&ma.fine, &mb.fine) else {
            return Err(format!("`{left}` and `{right}` were not sampled finely"));
        };
        let offset = oa.width;
        let mut best: Option<(f64, usize)> = None;
        for (i, (r, l)) in ra.samples.iter().zip(&lb.samples).enumerate() {
            if let (Some(r), Some(l)) = (r, l) {
                let d = l + offset - r;
                match best {
                    Some((b, _)) if b <= d => {}
                    _ => best = Some((d, i)),
                }
            }
        }
        let (d0, i) = best.ok_or_else(|| format!("`{left}` and `{right}` have no height in common"))?;
        let y = ra.height(i);
        let lo = (y - ra.step).max(oa.y0.max(ob.y0));
        let hi = (y + ra.step).min(oa.y1.min(ob.y1));
        let gap = |y: f64| match (oa.edges_at(y), ob.edges_at(y)) {
            (Some((_, r)), Some((l, _))) => l + offset - r,
            _ => f64::INFINITY,
        };
        Ok(if lo < hi { golden_min(gap, lo, hi, d0) } else { d0 })
    }

    /// The gap of each band's reference pair (n n, H H) without kerning; a band without its
    /// reference glyph takes the other's.
    fn references(&self, touching: bool) -> [Option<f64>; 2] {
        let mut out = [None, None];
        for (i, name) in REFERENCES.iter().enumerate() {
            let v = if touching { self.closest(name, name) } else { self.optical(name, name, i) };
            if let Some(slot) = out.get_mut(i) {
                *slot = v.ok();
            }
        }
        let [lower, caps] = out;
        [lower.or(caps), caps.or(lower)]
    }
}

/// The smallest value of `f` found by a golden-section search over `lo..hi`, or `start` when
/// that is smaller.
fn golden_min(f: impl Fn(f64) -> f64, lo: f64, hi: f64, start: f64) -> f64 {
    const G: f64 = 0.618_033_988_749_895;
    let (mut a, mut b) = (lo, hi);
    let (mut c, mut d) = (b - G * (b - a), a + G * (b - a));
    let (mut fc, mut fd) = (f(c), f(d));
    let mut best = start.min(fc).min(fd);
    for _ in 0..REFINE_STEPS {
        if fc < fd {
            b = d;
            d = c;
            fd = fc;
            c = b - G * (b - a);
            fc = f(c);
            best = best.min(fc);
        } else {
            a = c;
            c = d;
            fc = fd;
            d = a + G * (b - a);
            fd = f(d);
            best = best.min(fd);
        }
    }
    best
}

/// The glyphs a kit must measure for `jobs`, each with the band its pairs use, plus the
/// reference glyphs the font has.
fn needs_of(font: &Font, data: &GlyphData, jobs: &[Job]) -> (Vec<(String, usize)>, HashMap<String, usize>) {
    let mut bands: HashMap<String, usize> = HashMap::new();
    let mut band = |name: &str| -> usize {
        if let Some(b) = bands.get(name) {
            return *b;
        }
        let b = font.glyph(name).map_or(LOWER, |g| band_of(g, data));
        bands.insert(name.to_string(), b);
        b
    };
    let mut seen = HashSet::new();
    let mut needs = Vec::new();
    for (i, name) in REFERENCES.iter().enumerate() {
        if font.glyph(name).is_some() && seen.insert((name.to_string(), i)) {
            needs.push((name.to_string(), i));
        }
    }
    for j in jobs {
        let shared = if band(&j.left) == CAPS && band(&j.right) == CAPS { CAPS } else { LOWER };
        for name in [&j.left, &j.right] {
            if seen.insert((name.clone(), shared)) {
                needs.push((name.clone(), shared));
            }
        }
    }
    (needs, bands)
}

// ── Auto Kern ──────────────────────────────────────────────────────────────────────────────────

fn auto_kern(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("kerning.autoKern", v)?;
    let direction = horizontal(&p)?;
    let step = opt_in(&p, "step", profile::MIN_PROFILE_STEP..=MAX_COORDINATE)?.unwrap_or(DEFAULT_STEP);
    let depth = opt_in(&p, "depth", 0.0..=MAX_COORDINATE)?;
    let separation = opt_in(&p, "separation", SIDEBEARING_RANGE)?;
    let min_kern = opt_in(&p, "minKern", 0.0..=MAX_KERN_VALUE)?.unwrap_or(0.0);
    let unit = opt_in(&p, "round", 1.0..=1_000.0)?.unwrap_or(DEFAULT_ROUND);
    let only_closer = p.bool_or("onlyCloser", false)?;
    let touching = p.bool_or("touching", false)?;
    let keep_existing = p.bool_or("keepExisting", true)?;
    let apply = p.bool_or("apply", true)?;
    let data = s.glyph_data()?;
    let font = s.font()?;
    let index = KernIndex::new(font);
    let pairs = read_pairs(&p, font, &index, direction)?.ok_or_else(|| p.bad("give `pairs`, or `glyphsLeft` and `glyphsRight`"))?;
    if pairs.is_empty() {
        return Err(p.bad("there are no pairs to kern"));
    }
    let masters = master_ids(&p, font)?;
    if pairs.len().saturating_mul(masters.len()) > MAX_PAIRS {
        return Err(p.bad(format!("{} pairs in {} masters are more than {MAX_PAIRS}", pairs.len(), masters.len())));
    }
    let mut skipped = Vec::new();
    let mut jobs = Vec::new();
    for (first, second) in pairs {
        match job(&index, first, second, direction) {
            Ok(j) => jobs.push(j),
            Err((f, sd, reason)) => skipped.push(json!({"left": f.command(), "right": sd.command(), "reason": reason})),
        }
    }
    let (needs, bands) = needs_of(font, &data, &jobs);
    let band_of_glyph = |name: &str| bands.get(name).copied().unwrap_or(LOWER);
    let mut out = Vec::new();
    let mut targets = Map::new();
    let mut changes = Vec::new();
    for id in &masters {
        let m = font.master(id)?;
        if let Some(reason) = linked(font, m) {
            skipped.push(json!({"master": id, "reason": reason}));
            continue;
        }
        let kit = Kit::new(font, m, Some(step), depth, KERN_DEPTH_SHARE, &needs, touching).map_err(|e| p.bad(e))?;
        let refs = kit.references(touching);
        let [lower, caps] = refs;
        targets.insert(id.clone(), json!({"lowercase": lower, "capitals": caps}));
        if separation.is_none() && refs.iter().any(Option::is_none) {
            return Err(p.bad(format!("master `{}` has neither n nor H to measure the target on; give `separation`", m.name)));
        }
        let stored = stored_by_sides(font, direction, id);
        for j in &jobs {
            crate::work::check()?;
            let (bl, br) = (band_of_glyph(&j.left), band_of_glyph(&j.right));
            let shared = if bl == CAPS && br == CAPS { CAPS } else { LOWER };
            let gap = if touching { kit.closest(&j.left, &j.right) } else { kit.optical(&j.left, &j.right, shared) };
            let gap = match gap {
                Ok(g) => g,
                Err(reason) => {
                    skipped.push(pair_skip(&j.first, &j.second, id, reason));
                    continue;
                }
            };
            let target = match separation {
                Some(t) => t,
                None => (refs.get(bl).copied().flatten().unwrap_or(0.0) + refs.get(br).copied().flatten().unwrap_or(0.0)) / 2.0,
            };
            let value = round_to(target - gap, unit);
            if !(value.is_finite() && value.abs() <= MAX_KERN_VALUE) {
                skipped.push(pair_skip(&j.first, &j.second, id, format!("the value would be {value}, beyond ±{MAX_KERN_VALUE}")));
                continue;
            }
            let previous = stored.get(&(j.first.clone(), j.second.clone()));
            let dropped = value == 0.0 || value.abs() < min_kern || (only_closer && value > 0.0);
            let result = match (previous, keep_existing, dropped) {
                (Some(_), true, _) => "kept",
                (_, _, false) => {
                    changes.push(Change::set(&stored, direction, id, &j.first, &j.second, value));
                    "set"
                }
                (Some((f, s, _)), false, true) => {
                    changes.push(Change { master: id.clone(), first: f.clone(), second: s.clone(), value: None });
                    "removed"
                }
                (None, _, true) => "dropped",
            };
            out.push(json!({
                "left": j.first.command(), "right": j.second.command(), "master": id, "value": value,
                "gap": gap, "target": target, "previous": previous.map(|(_, _, v)| *v), "result": result,
            }));
        }
    }
    if apply && !changes.is_empty() {
        s.edit("Auto Kern", |font| {
            apply_changes(font, direction, &changes);
            Ok(())
        })?;
    }
    Ok(json!({"pairs": out, "targets": targets, "skipped": skipped}))
}

// ── Kern to Distance ───────────────────────────────────────────────────────────────────────────

fn kern_to_distance(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("kerning.kernToDistance", v)?;
    let direction = horizontal(&p)?;
    let distance = opt_in(&p, "distance", SIDEBEARING_RANGE)?.ok_or_else(|| p.bad("`distance` is required"))?;
    let apply = p.bool_or("apply", true)?;
    let data = s.glyph_data()?;
    let font = s.font()?;
    let index = KernIndex::new(font);
    let given = read_pairs(&p, font, &index, direction)?;
    if given.as_ref().is_some_and(Vec::is_empty) {
        return Err(p.bad("there are no pairs to kern"));
    }
    let masters = master_ids(&p, font)?;
    let mut skipped = Vec::new();
    let mut out = Vec::new();
    let mut changes = Vec::new();
    let mut total = 0usize;
    for id in &masters {
        let m = font.master(id)?;
        if let Some(reason) = linked(font, m) {
            skipped.push(json!({"master": id, "reason": reason}));
            continue;
        }
        let stored = stored_by_sides(font, direction, id);
        let pairs: Vec<(Side, Side)> = match &given {
            Some(list) => list.clone(),
            None => stored_pairs(font, direction, id).iter().map(|lp| (lp.left(), lp.right())).collect(),
        };
        total = total.saturating_add(pairs.len());
        if total > MAX_PAIRS {
            return Err(p.bad(format!("the call would measure more than {MAX_PAIRS} pairs")));
        }
        let mut jobs = Vec::new();
        let mut seen = HashSet::new();
        for (first, second) in pairs {
            if !seen.insert((first.clone(), second.clone())) {
                continue;
            }
            match job(&index, first, second, direction) {
                Ok(j) if font.glyph(&j.left).is_some() && font.glyph(&j.right).is_some() => jobs.push(j),
                Ok(j) => skipped.push(pair_skip(&j.first, &j.second, id, "the pair names a glyph the font does not have")),
                Err((f, sd, reason)) => skipped.push(pair_skip(&f, &sd, id, reason)),
            }
        }
        let (needs, _) = needs_of(font, &data, &jobs);
        let kit = Kit::new(font, m, None, None, KERN_DEPTH_SHARE, &needs, true).map_err(|e| p.bad(e))?;
        let table = index.table(direction, id).map_err(|e| p.bad(e.to_string()))?;
        for j in &jobs {
            crate::work::check()?;
            let gap = match kit.closest(&j.left, &j.right) {
                Ok(g) => g,
                Err(reason) => {
                    skipped.push(pair_skip(&j.first, &j.second, id, reason));
                    continue;
                }
            };
            let value = round_half_up(distance - gap);
            if !(value.is_finite() && value.abs() <= MAX_KERN_VALUE) {
                skipped.push(pair_skip(&j.first, &j.second, id, format!("the value would be {value}, beyond ±{MAX_KERN_VALUE}")));
                continue;
            }
            let previous = stored.get(&(j.first.clone(), j.second.clone())).map(|(_, _, v)| *v);
            // A 0 changes nothing unless the pair is stored or another pair would apply.
            let other_applies = || match (index.glyph_id(&j.first_glyph), index.glyph_id(&j.second_glyph)) {
                (Some(f), Some(s)) => table.lookup(f, s).is_some(),
                _ => false,
            };
            if (value != 0.0 || previous.is_some() || other_applies()) && previous != Some(value) {
                changes.push(Change::set(&stored, direction, id, &j.first, &j.second, value));
            }
            out.push(json!({"left": j.first.command(), "right": j.second.command(), "master": id, "value": value, "gap": gap, "previous": previous}));
        }
    }
    if apply && !changes.is_empty() {
        s.edit("Kern to Distance", |font| {
            apply_changes(font, direction, &changes);
            Ok(())
        })?;
    }
    Ok(json!({"pairs": out, "skipped": skipped}))
}

// ── Build Kerning Groups ───────────────────────────────────────────────────────────────────────

/// One glyph's side, ready to compare: per master, each row's depth inward from the extreme
/// (`None` without ink) and whether the row lies inside the band's core.
struct Candidate {
    name: String,
    band: usize,
    rows: Vec<Vec<(Option<f64>, bool)>>,
}

/// The difference between two candidates: the root mean square of their depth differences,
/// the largest over the masters.
fn difference(a: &Candidate, b: &Candidate, depths: &[f64]) -> f64 {
    let mut worst: f64 = 0.0;
    for ((ra, rb), depth) in a.rows.iter().zip(&b.rows).zip(depths) {
        let mut sum = 0.0;
        let mut n = 0usize;
        for ((da, core), (db, _)) in ra.iter().zip(rb) {
            let d = match (da, db) {
                (Some(x), Some(y)) => (x - y).abs(),
                (None, None) => continue,
                _ if *core => *depth,
                _ => continue,
            };
            sum += d * d;
            n += 1;
        }
        let rms = if n == 0 { *depth } else { (sum / n as f64).sqrt() };
        worst = worst.max(rms);
    }
    worst
}

fn group_slot(groups: &mut KernGroups, side: GroupSide) -> &mut Option<String> {
    match side {
        GroupSide::Left => &mut groups.left,
        GroupSide::Right => &mut groups.right,
        GroupSide::Top => &mut groups.top,
        GroupSide::Bottom => &mut groups.bottom,
    }
}

/// A group name made from a glyph name: characters a group name may not hold become `_`.
fn group_name(glyph: &str) -> String {
    glyph.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') { c } else { '_' }).collect()
}

fn build_groups(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("kerning.buildGroups", v)?;
    let max_difference = opt_in(&p, "maxDifference", 0.0..=MAX_COORDINATE)?.ok_or_else(|| p.bad("`maxDifference` is required"))?;
    let sides: Vec<GroupSide> = match p.opt_str("side")? {
        None | Some("both") => vec![GroupSide::Left, GroupSide::Right],
        Some("left") => vec![GroupSide::Left],
        Some("right") => vec![GroupSide::Right],
        Some(other) => return Err(p.bad(format!("`side` must be \"left\", \"right\" or \"both\", got `{other}`"))),
    };
    let replace = p.bool_or("replaceExisting", false)?;
    let apply = p.bool_or("apply", false)?;
    let step = opt_in(&p, "step", profile::MIN_PROFILE_STEP..=MAX_COORDINATE)?.unwrap_or(DEFAULT_STEP);
    let depth = opt_in(&p, "depth", 0.0..=MAX_COORDINATE)?;
    let data = s.glyph_data()?;
    let font = s.font()?;
    let mut names = match p.opt_glyphs("glyphs", font)? {
        Some(n) if n.is_empty() => return Err(p.bad("`glyphs` is empty")),
        Some(n) => n,
        None => default_glyphs(font, &data),
    };
    let order: HashMap<&str, usize> = font.glyphs.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
    names.sort_by_key(|n| order.get(n.as_str()).copied().unwrap_or(usize::MAX));
    let masters = master_ids(&p, font)?;
    let mut skipped = Vec::new();
    // Each glyph with its band; locked glyphs are left out.
    let mut glyphs: Vec<(String, usize)> = Vec::new();
    for name in &names {
        let Some(g) = font.glyph(name) else { continue };
        if g.info.locked {
            skipped.push(json!({"glyph": name, "reason": "the glyph is locked"}));
            continue;
        }
        glyphs.push((name.clone(), band_of(g, &data)));
    }
    let mut kits = Vec::new();
    for id in &masters {
        let m = font.master(id)?;
        kits.push(Kit::new(font, m, Some(step), depth, GROUP_DEPTH_SHARE, &glyphs, false).map_err(|e| p.bad(e))?);
    }
    let depths: Vec<f64> = kits.iter().map(|k| k.depth).collect();
    // Glyphs that cannot be measured in some master are left out of every side.
    let mut usable = Vec::new();
    for (name, band) in &glyphs {
        match kits.iter().map(|k| k.clipped(name, *band).map(|_| ())).find_map(std::result::Result::err) {
            Some(reason) => skipped.push(json!({"glyph": name, "reason": reason})),
            None => usable.push((name.clone(), *band)),
        }
    }
    let mut groups_json = Map::new();
    let mut assign: Vec<(String, GroupSide, String)> = Vec::new();
    let mut comparisons = 0usize;
    for side in &sides {
        let mut candidates: Vec<Candidate> = Vec::new();
        for (name, band) in &usable {
            crate::work::check()?;
            if !replace && let Some(existing) = font.glyph(name).and_then(|g| side.of(&g.kern_groups)) {
                skipped.push(json!({"glyph": name, "side": side.name(), "reason": format!("it keeps its {} group `{existing}`", side.name())}));
                continue;
            }
            let mut rows: Vec<Vec<(Option<f64>, bool)>> = Vec::with_capacity(kits.len());
            for k in &kits {
                let Ok(((l, r), _)) = k.clipped(name, *band) else { continue };
                let (core0, core1) = k.cores.get(*band).copied().unwrap_or((0.0, 0.0));
                let prof = if *side == GroupSide::Left { l } else { r };
                let extreme = prof.extreme().unwrap_or(0.0);
                let depth_of = |x: f64| if *side == GroupSide::Left { x - extreme } else { extreme - x };
                rows.push(
                    prof.samples
                        .iter()
                        .enumerate()
                        .map(|(i, x)| {
                            let h = prof.height(i);
                            (x.map(depth_of), (core0..=core1).contains(&h))
                        })
                        .collect(),
                );
            }
            candidates.push(Candidate { name: name.clone(), band: *band, rows });
        }
        // Clusters of candidate indices, in glyph order.
        let mut clusters: Vec<(usize, Vec<usize>)> = Vec::new();
        for (ci, cand) in candidates.iter().enumerate() {
            crate::work::check()?;
            let mut best: Option<(f64, usize)> = None;
            for (k, (band, members)) in clusters.iter().enumerate() {
                if *band != cand.band {
                    continue;
                }
                let mut worst: f64 = 0.0;
                for other in members.iter().filter_map(|mi| candidates.get(*mi)) {
                    comparisons = comparisons.saturating_add(kits.len());
                    if comparisons > MAX_GROUP_COMPARISONS {
                        return Err(p.bad(format!(
                            "grouping these glyphs needs more than {MAX_GROUP_COMPARISONS} profile comparisons; give fewer glyphs or masters"
                        )));
                    }
                    worst = worst.max(difference(cand, other, &depths));
                    if worst > max_difference {
                        break;
                    }
                }
                if worst <= max_difference && best.is_none_or(|(b, _)| worst < b) {
                    best = Some((worst, k));
                }
            }
            match best.and_then(|(_, k)| clusters.get_mut(k)) {
                Some((_, members)) => members.push(ci),
                None => clusters.push((cand.band, vec![ci])),
            }
        }
        let grouped: HashSet<&str> = clusters
            .iter()
            .filter(|(_, m)| m.len() > 1)
            .flat_map(|(_, m)| m.iter().filter_map(|i| candidates.get(*i)).map(|c| c.name.as_str()))
            .collect();
        // Names held by glyphs that keep their group on this side.
        let mut taken: HashSet<String> =
            font.glyphs.iter().filter(|g| !grouped.contains(g.name.as_str())).filter_map(|g| side.of(&g.kern_groups).map(str::to_string)).collect();
        let mut list = Vec::new();
        for (_, members) in clusters.iter().filter(|(_, m)| m.len() > 1) {
            let names: Vec<&str> = members.iter().filter_map(|i| candidates.get(*i)).map(|c| c.name.as_str()).collect();
            let Some(seed) = names.first() else { continue };
            let base = group_name(seed);
            let mut name = base.clone();
            let mut n = 2usize;
            while taken.contains(&name) && n <= taken.len() + 2 {
                name = format!("{base}.{n}");
                n += 1;
            }
            if taken.contains(&name) || check_group_name(&name).is_err() {
                for g in &names {
                    skipped.push(json!({"glyph": g, "side": side.name(), "reason": format!("no usable group name could be made from `{seed}`")}));
                }
                continue;
            }
            taken.insert(name.clone());
            for g in &names {
                assign.push((g.to_string(), *side, name.clone()));
            }
            list.push(json!({"name": name, "glyphs": names}));
        }
        groups_json.insert(side.name().to_string(), Value::Array(list));
    }
    let mut changed: Vec<String> = Vec::new();
    if apply && !assign.is_empty() {
        s.edit("Build Kerning Groups", |font| {
            for (glyph, side, name) in &assign {
                let mut gm = font.glyph_mut(glyph)?;
                let slot = group_slot(&mut gm.kern_groups, *side);
                if slot.as_deref() != Some(name.as_str()) {
                    *slot = Some(name.clone());
                    if !changed.contains(glyph) {
                        changed.push(glyph.clone());
                    }
                }
            }
            Ok(())
        })?;
    }
    Ok(json!({"groups": groups_json, "glyphs": changed, "skipped": skipped}))
}

// ── Remove Pairs ───────────────────────────────────────────────────────────────────────────────

/// The value that applies to an exception's glyphs once it is gone: `None` when the pair is not
/// an exception (no glyph side whose glyph has a group facing the pair).
fn class_value(index: &KernIndex, table: &Table<'_>, direction: Direction, first: &Side, second: &Side) -> Option<f64> {
    let (ff, fs) = (facing(direction, Position::First), facing(direction, Position::Second));
    const AFTER_GLYPHS: [Step; 3] = [Step::GlyphGroup, Step::GroupGlyph, Step::GroupGroup];
    const GROUPS: [Step; 1] = [Step::GroupGroup];
    let (f, s, steps): (usize, usize, &[Step]) = match (first, second) {
        (Side::Glyph(a), Side::Glyph(b)) => {
            if index.group(a, ff).is_none() && index.group(b, fs).is_none() {
                return None;
            }
            (index.glyph_id(a)?, index.glyph_id(b)?, &AFTER_GLYPHS)
        }
        (Side::Glyph(a), Side::Group(g)) => {
            index.group(a, ff)?;
            let member = index.members(fs, g).next()?;
            (index.glyph_id(a)?, index.glyph_id(member)?, &GROUPS)
        }
        (Side::Group(g), Side::Glyph(b)) => {
            index.group(b, fs)?;
            let member = index.members(ff, g).next()?;
            (index.glyph_id(member)?, index.glyph_id(b)?, &GROUPS)
        }
        (Side::Group(_), Side::Group(_)) => return None,
    };
    Some(steps.iter().find_map(|step| table.at_step(f, s, *step)).map_or(0.0, |a| a.value))
}

fn remove_pairs_by_rule(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("kerning.removePairsByRule", v)?;
    let direction = any_direction(&p)?;
    let smaller = opt_in(&p, "smallerThan", 0.0..=MAX_KERN_VALUE + 1.0)?;
    let within = opt_in(&p, "exceptionsWithin", 0.0..=2.0 * MAX_KERN_VALUE)?;
    if smaller.is_none() && within.is_none() {
        return Err(p.bad("give `smallerThan`, `exceptionsWithin` or both"));
    }
    let font = s.font()?;
    let index = KernIndex::new(font);
    let masters = master_ids(&p, font)?;
    let scope = p.opt_glyphs("glyphs", font)?;
    if scope.as_ref().is_some_and(Vec::is_empty) {
        return Err(p.bad("`glyphs` is empty"));
    }
    // The glyphs in scope, and the groups they belong to on each side facing a pair.
    let scope = scope.map(|names| {
        let groups =
            |position| -> HashSet<String> { names.iter().filter_map(|n| index.group(n, facing(direction, position))).map(str::to_string).collect() };
        let (firsts, seconds) = (groups(Position::First), groups(Position::Second));
        (names.into_iter().collect::<HashSet<String>>(), firsts, seconds)
    });
    let in_scope = |side: &Side, position: Position| match (&scope, side) {
        (None, _) => true,
        (Some((names, _, _)), Side::Glyph(n)) => names.contains(n),
        (Some((_, firsts, seconds)), Side::Group(g)) => {
            if position == Position::First {
                firsts.contains(g)
            } else {
                seconds.contains(g)
            }
        }
    };
    let mut skipped = Vec::new();
    let mut removed = Vec::new();
    let mut changes = Vec::new();
    for id in &masters {
        let m = font.master(id)?;
        if let Some(reason) = linked(font, m) {
            skipped.push(json!({"master": id, "reason": reason}));
            continue;
        }
        let table = index.table(direction, id).map_err(|e| p.bad(e.to_string()))?;
        for lp in stored_pairs(font, direction, id) {
            let (first, second) = (lp.left(), lp.right());
            if !(in_scope(&first, Position::First) || in_scope(&second, Position::Second)) {
                continue;
            }
            let entry = |rule: &str| json!({"master": id, "left": first.command(), "right": second.command(), "value": lp.value, "rule": rule});
            let hit = if smaller.is_some_and(|limit| lp.value.abs() < limit) {
                Some(entry("smallerThan"))
            } else {
                match (within, class_value(&index, &table, direction, &first, &second)) {
                    (Some(w), Some(class)) if (lp.value - class).abs() <= w => {
                        let mut e = entry("exceptionsWithin");
                        if let Some(o) = e.as_object_mut() {
                            o.insert("classValue".into(), json!(class));
                        }
                        Some(e)
                    }
                    _ => None,
                }
            };
            if let Some(e) = hit {
                removed.push(e);
                changes.push(Change { master: id.clone(), first: lp.first.to_string(), second: lp.second.to_string(), value: None });
            }
        }
    }
    if !changes.is_empty() {
        s.edit("Remove Pairs", |font| {
            apply_changes(font, direction, &changes);
            Ok(())
        })?;
    }
    Ok(json!({"removed": removed, "skipped": skipped}))
}

/// A vertical kerning pair to store: top glyph, bottom glyph, value.
type VerticalPair = (String, String, f64);

// ── Vertical from horizontal ───────────────────────────────────────────────────────────────────

fn vertical_from_horizontal(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("kerning.verticalFromHorizontal", v)?;
    let font = s.font()?;
    let index = KernIndex::new(font);
    let masters = master_ids(&p, font)?;
    // Each glyph that has a vertical alternate, with the alternate's name.
    let alternates: HashMap<&str, &str> = font
        .glyphs
        .iter()
        .filter_map(|g| g.name.strip_suffix(".vert").filter(|base| font.glyph(base).is_some()).map(|base| (base, g.name.as_str())))
        .collect();
    let expand = |side: &Side, position: Position| match side {
        Side::Glyph(n) => alternates.get_key_value(n.as_str()).map(|(k, _)| *k).into_iter().collect::<Vec<&str>>(),
        Side::Group(g) => index.members(facing(Direction::Ltr, position), g).filter(|m| alternates.contains_key(m)).collect::<Vec<&str>>(),
    };
    let mut skipped = Vec::new();
    let mut counts = Map::new();
    // Per master, the vertical pairs to store: top, bottom, value.
    let mut created: Vec<(String, Vec<VerticalPair>)> = Vec::new();
    let mut out = Vec::new();
    let mut total = 0usize;
    for id in &masters {
        let m = font.master(id)?;
        if let Some(reason) = linked(font, m) {
            skipped.push(json!({"master": id, "reason": reason}));
            continue;
        }
        let table = index.table(Direction::Ltr, id).map_err(|e| p.bad(e.to_string()))?;
        let cleared: usize = font.kerning.vertical.0.iter().filter(|e| e.master == *id).flat_map(|e| &e.firsts).map(|f| f.seconds.len()).sum();
        let mut seen = HashSet::new();
        let mut pairs = Vec::new();
        for lp in stored_pairs(font, Direction::Ltr, id) {
            let tops = expand(&lp.left(), Position::First);
            let bottoms = expand(&lp.right(), Position::Second);
            total = total.saturating_add(tops.len().saturating_mul(bottoms.len()));
            if total > MAX_PAIRS {
                return Err(p.bad(format!("the vertical pairs would be more than {MAX_PAIRS}")));
            }
            for a in &tops {
                for b in &bottoms {
                    if !seen.insert((*a, *b)) {
                        continue;
                    }
                    let value = match (index.glyph_id(a), index.glyph_id(b)) {
                        (Some(f), Some(s)) => table.lookup(f, s).map_or(0.0, |x| x.value),
                        _ => 0.0,
                    };
                    let (Some(top), Some(bottom)) = (alternates.get(a), alternates.get(b)) else { continue };
                    if value != 0.0 {
                        out.push(json!({"master": id, "top": top, "bottom": bottom, "value": value}));
                        pairs.push((top.to_string(), bottom.to_string(), value));
                    }
                }
            }
        }
        counts.insert(id.clone(), json!({"cleared": cleared, "created": pairs.len()}));
        created.push((id.clone(), pairs));
    }
    s.edit("Vertical Kerning from Horizontal", |font| {
        let table = &mut Arc::make_mut(&mut font.kerning).vertical;
        for (id, pairs) in &created {
            table.0.retain(|e| e.master != *id);
            for (top, bottom, value) in pairs {
                table.set(id, top, bottom, *value);
            }
        }
        Ok(())
    })?;
    Ok(json!({"masters": counts, "pairs": out, "skipped": skipped}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_min_finds_the_tip_of_a_one_sided_minimum() {
        // Falls toward 500, then no ink above it.
        let f = |y: f64| if y > 500.0 { f64::INFINITY } else { 50.0 + (500.0 - y) * 0.4 };
        let best = golden_min(f, 498.5, 500.5, f(499.5));
        assert!((best - 50.0).abs() < 1e-3, "{best}");
    }

    #[test]
    fn group_names_keep_only_allowed_characters() {
        assert_eq!(group_name("a.sc"), "a.sc");
        assert_eq!(group_name("a+b"), "a_b");
        assert!(check_group_name(&group_name("é")).is_ok());
    }
}
