//! Auto-spacing, Center in Width and Copy Metrics. Rows: `spacing.autoSpace`,
//! `spacing.centerInWidth`, `edit.copyMetrics`; the `beam` parameter of the left and right
//! sidebearing setters (`view.showMetricsBeam`, engine half) lives with the setters in
//! `cmd/spacing.rs`. Plan: `docs/plans/M13.md` § 3, decision D11.
//!
//! # Auto-spacing, method `optical`
//! Per master, each glyph belongs to one of two groups: capitals and figures (case `upper` or
//! category `Number`), measured over the band from the baseline less its overshoot to the cap
//! height plus its overshoot; and everything else, over the band up to the x-height. Each side's
//! profile ([`fontcraft_spacing::profile`]) is clipped to `depth` inward from its extreme.
//!
//! Model `classes` (the default, [`fontcraft_spacing::space_model`]): a side takes the spacing of
//! the control side whose shape it resembles most, corrected for the difference between the two
//! shapes. The controls are the sides of the font's own `n`, `o` and `v` (capitals and figures:
//! `H`, `O` and `V`) as they are spaced, then those of the glyphs in `references`; they keep their
//! spacing. `area` moves the whole group so a straight side gets that white. A group without
//! controls takes the other group's; a master without any is spaced by the area model, which needs
//! `area`.
//!
//! Model `area`: a side's white has two parts, the distance from the advance edge to the side's
//! extreme, and the mean depth of the clipped profile inside it, which counts `weight` times. The
//! sidebearing is set so the two add up to `area`; a straight side gets `area` itself, and round
//! or open sides less, by the weight times their mean depth. The group's area and weight are
//! calibrated on the control glyphs: a least-squares line through their sides, the white outside
//! the extreme against the mean depth, whose slope gives the weight (held between 0 and
//! [`MAX_WEIGHT`]) and whose value at depth 0 gives the area. Without depth to compare (only
//! straight controls), the weight is 1 and the area their mean white. `area` replaces the
//! calibrated area and keeps the weight; a group without its control glyphs takes the other
//! group's; a font with none of them has weight 1 and needs `area`.
//!
//! Sidebearings are rounded to whole units, then held between `min` and `max`.
//!
//! Profiles are taken on the measured outline (slanted back in an italic master), so the values
//! agree with the sidebearings the Info box shows. A side whose field has a metrics key keeps its
//! value; a layer with a width key or under automatic alignment is skipped.

use std::collections::{BTreeMap, HashSet};

use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::guide_follow::{BothGuides, follow_width};
use fontcraft_model::{Font, Glyph, Layer, MAX_COORDINATE, Master, MetricValue, glyph_props, metric};
use fontcraft_spacing::profile::{self, Band, Side};
use fontcraft_spacing::space_model::{ClassModel, Control, SideShape};
use fontcraft_spacing::{self as spacing, Field, SIDEBEARING_RANGE, Scope, WIDTH_RANGE};
use serde_json::{Map, Value, json};

use super::common::{self, in_range, master_metrics};
use super::shared::{shift_layer, shift_with_right_edge};
use super::*;
use crate::EngineError;
use crate::params::{Params, TargetKind};

/// Sampling step of the profiles by default, in units.
const DEFAULT_STEP: f64 = 5.0;
/// Clipping depth of the class model by default, as a share of the master's x-height.
const CLASS_DEPTH_SHARE: f64 = 0.15;
/// Clipping depth of the area model by default, as a share of the master's x-height. On the corpus
/// measure (`docs/plans/M13.md` § 5) shares from 5% to 10% did about equally well, deeper ones
/// worse.
const AREA_DEPTH_SHARE: f64 = 0.10;
/// Most glyphs `references` may name.
pub const MAX_REFERENCES: usize = 256;
/// Largest weight calibration gives the depth inside a profile.
pub const MAX_WEIGHT: f64 = 2.0;
/// Sidebearing of method `custom` by default, in percent of the em.
const DEFAULT_PERCENT: f64 = 5.0;
/// The control glyphs that calibrate each group, lowercase first: a straight, a round and a
/// diagonal side.
const CONTROLS: [[&str; 3]; 2] = [["n", "o", "v"], ["H", "O", "V"]];

pub fn specs() -> Vec<CommandSpec> {
    let g: &'static [TargetKind] = &[TargetKind::Glyphs];
    vec![
        cmd!(
            "spacing.autoSpace",
            "Auto Space",
            ["Glyph"],
            None,
            "{glyphs?: [names] (default: every exported letter and figure), masters?: [master ids] (default: every master), method?: 'optical' (default) | 'tight' | 'custom', model?: 'classes' (default) | 'area' (optical), references?: [names] of glyphs that are spaced already, to take spacing from besides n, o and v (capitals and figures: H, O and V; model classes, at most 256), step?: units between profile samples (optical, default 5, at least 1), depth?: units the profile is clipped to inward from its extreme (optical, default 15% of the x-height, 10% with model area), area?: the white beside a straight side in units (optical, default what the font's n, or H for capitals and figures, has; with model area calibrated with the depth weight on n, o and v, or H, O and V), separation?: white between two straight neighbours in units, twice `area` (give one of the two), percentOfUpm?: both sidebearings in percent of the em (custom, default 5), min?: smallest sidebearing, max?: largest sidebearing, beam?: height to measure at (tight and custom)} set the left and right sidebearings of each glyph's master layers from their shapes. Optical measures the side profiles over the band from the baseline to the x-height (cap height for capitals and figures), overshoots included. Model classes gives each side the spacing of the control side of the most similar shape, corrected for the difference; n, o and v (H, O and V) and the references keep their spacing. Without any of them in the master it uses model area, which needs `area`. Model area makes each side's white equal to the area: the distance from the advance edge to the side's extreme plus the weight times the mean depth of its profile clipped to `depth`. Tight sets both sidebearings to 0, custom to a share of the em. Sidebearings are whole units. A side with a metrics key keeps its value; layers with a width key, under automatic alignment, empty, or without ink in the band are skipped, as are locked glyphs and masters that take their widths from another → {glyphs: [names changed], values: {glyph: {master id: {left, right}}}, models: {master id: 'classes' | 'area'}, areas: {master id: {lowercase, capitals}} (the white beside a straight side), weights: {master id: {lowercase, capitals}} (model area), classes: {glyph: {master id: {left, right}}} (model classes: the control side taken, as `n left`; a side with a metrics key has none), skipped: [{glyph?, layer, reason}]}",
            has_doc,
            auto_space
        )
        .targets(g),
        cmd!(
            "spacing.centerInWidth",
            "Center in Width",
            ["Glyph"],
            None,
            "{glyph | glyphs: [names], layer? | layers?: [ids] (default: the first master), ratio?: left ÷ right as a number, or text `a:b` (default 1; `1:2` puts a third of the space left)} move each layer's contents sideways inside its unchanged width so the left and right sidebearings share their sum in the ratio; guides, anchors and the background move along. The new left sidebearing is rounded to the font's grid. Metrics keys stay and may then be out of sync. Empty layers and layers under automatic alignment are skipped, as are locked glyphs when several are given → {glyphs: [names changed], values: {glyph: {layer id: {left, right}}}, skipped: [{glyph, layer?, reason}]}",
            has_doc,
            center_in_width
        )
        .targets(g),
        cmd!(
            "edit.copyMetrics",
            "Copy Width / Sidebearings",
            ["Edit"],
            None,
            "{from: glyph, what: ['width' | 'lsb' | 'rsb' | 'verticalAdvance'], to?: [names], layers?: [ids] (default: the first master)} give each glyph in `to` the chosen metrics of `from`, layer by layer (the same layer id in both), without changing outlines: width sets the advance, lsb moves the contents and the width with them, rsb sets the width, verticalAdvance the vertical width; applied in that order, so lsb and rsb together decide the width. A copied value replaces the metrics key of its field. Without `to`, only the values are given. Layers that are empty (for lsb and rsb), under automatic alignment (for width, lsb and rsb) or missing are skipped, as are locked glyphs → {from, values: {layer id: {width?, lsb?, rsb?, verticalAdvance?}}, glyphs: [names changed], skipped: [{glyph, layer?, reason}]}",
            has_doc,
            copy_metrics
        )
        .targets(g),
    ]
}

// ── Shared ─────────────────────────────────────────────────────────────────────────────────────

fn skip(list: &mut Vec<Value>, glyph: Option<&str>, layer: Option<&str>, reason: impl Into<String>) {
    let mut v = Map::new();
    if let Some(g) = glyph {
        v.insert("glyph".into(), json!(g));
    }
    if let Some(l) = layer {
        v.insert("layer".into(), json!(l));
    }
    v.insert("reason".into(), json!(reason.into()));
    list.push(Value::Object(v));
}

/// A number parameter that must lie in `range` when given.
pub(super) fn opt_in(p: &Params, key: &str, range: std::ops::RangeInclusive<f64>) -> Result<Option<f64>> {
    match p.opt_f64(key)? {
        Some(v) if !(v.is_finite() && range.contains(&v)) => {
            Err(p.bad(format!("`{key}` must be between {} and {}, got {v}", range.start(), range.end())))
        }
        other => Ok(other),
    }
}

/// Rounds `v` to the font's grid; a grid of 0 leaves it.
fn to_grid(font: &Font, v: f64) -> f64 {
    let g = common::grid(font);
    if g > 0.0 { (v / g).round() * g } else { v }
}

fn check_width(w: f64) -> std::result::Result<(), String> {
    if WIDTH_RANGE.contains(&w) { Ok(()) } else { Err(format!("the width would be {w}, outside {} to {}", WIDTH_RANGE.start(), WIDTH_RANGE.end())) }
}

// ── Auto Space ─────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    Optical,
    Tight,
    Custom,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Model {
    Classes,
    Area,
}

impl Model {
    fn name(self) -> &'static str {
        match self {
            Model::Classes => "classes",
            Model::Area => "area",
        }
    }
}

struct Options {
    method: Method,
    model: Model,
    references: Vec<String>,
    step: f64,
    depth: Option<f64>,
    area: Option<f64>,
    percent: f64,
    min: Option<f64>,
    max: Option<f64>,
    beam: Option<f64>,
}

fn options(p: &Params, font: &Font) -> Result<Options> {
    let method = match p.opt_str("method")? {
        None | Some("optical") => Method::Optical,
        Some("tight") => Method::Tight,
        Some("custom") => Method::Custom,
        Some(other) => return Err(p.bad(format!("`method` must be optical, tight or custom, got `{other}`"))),
    };
    let model = match p.opt_str("model")? {
        None | Some("classes") => Model::Classes,
        Some("area") => Model::Area,
        Some(other) => return Err(p.bad(format!("`model` must be classes or area, got `{other}`"))),
    };
    if method != Method::Optical && p.get("model").is_some() {
        return Err(p.bad("`model` applies to method optical"));
    }
    if let Some(Value::Array(a)) = p.get("references")
        && a.len() > MAX_REFERENCES
    {
        return Err(p.bad(format!("`references` holds {} names; at most {MAX_REFERENCES} are allowed", a.len())));
    }
    let references = p.opt_glyphs("references", font)?.unwrap_or_default();
    if !references.is_empty() && (method != Method::Optical || model != Model::Classes) {
        return Err(p.bad("`references` applies to method optical with model classes"));
    }
    let step = opt_in(p, "step", profile::MIN_PROFILE_STEP..=MAX_COORDINATE)?.unwrap_or(DEFAULT_STEP);
    let depth = opt_in(p, "depth", 0.0..=MAX_COORDINATE)?;
    let area = opt_in(p, "area", SIDEBEARING_RANGE)?;
    let separation = opt_in(p, "separation", SIDEBEARING_RANGE)?;
    let area = match (area, separation) {
        (Some(_), Some(_)) => return Err(p.bad("give `area` or `separation`, not both")),
        (Some(a), None) => Some(a),
        (None, s) => s.map(|s| s / 2.0),
    };
    let percent = opt_in(p, "percentOfUpm", -100.0..=100.0)?.unwrap_or(DEFAULT_PERCENT);
    let min = opt_in(p, "min", SIDEBEARING_RANGE)?;
    let max = opt_in(p, "max", SIDEBEARING_RANGE)?;
    if let (Some(lo), Some(hi)) = (min, max)
        && lo > hi
    {
        return Err(p.bad(format!("`min` ({lo}) is larger than `max` ({hi})")));
    }
    let beam = opt_in(p, "beam", -MAX_COORDINATE..=MAX_COORDINATE)?;
    if beam.is_some() && method == Method::Optical {
        return Err(p.bad("`beam` applies to methods tight and custom; optical measures the whole band"));
    }
    Ok(Options { method, model, references, step, depth, area, percent, min, max, beam })
}

/// Capitals and figures measure up to the cap height, the rest up to the x-height.
fn is_capital(g: &Glyph, data: &GlyphData) -> bool {
    let props = glyph_props(g, data, false);
    props.case.as_deref() == Some("upper") || props.category.as_deref() == Some("Number")
}

/// Every exported letter and figure, in font order.
pub(super) fn default_glyphs(font: &Font, data: &GlyphData) -> Vec<String> {
    font.glyphs
        .iter()
        .filter(|g| g.export)
        .filter(|g| matches!(glyph_props(g, data, false).category.as_deref(), Some("Letter" | "Number")))
        .map(|g| g.name.clone())
        .collect()
}

/// `masters`: master ids, repeats dropped; every master by default.
pub(super) fn master_ids(p: &Params, font: &Font) -> Result<Vec<String>> {
    let Some(ids) = p.opt_strings("masters")? else {
        return Ok(font.masters.iter().map(|m| m.id.clone()).collect());
    };
    if ids.is_empty() {
        return Err(p.bad("`masters` is empty"));
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for id in ids {
        if !font.masters.iter().any(|m| m.id == id) {
            return Err(p.bad(format!("`masters`: there is no master `{id}`")));
        }
        if seen.insert(id) {
            out.push(id.to_string());
        }
    }
    Ok(out)
}

/// How one group of glyphs is spaced.
#[derive(Clone)]
enum GroupModel {
    /// White = area − weight × mean clipped depth.
    Area { area: f64, weight: f64 },
    /// The class model, its whites moved by `shift`.
    Classes { model: ClassModel, shift: f64 },
}

impl GroupModel {
    /// The white beside a straight side.
    fn area(&self) -> f64 {
        match self {
            GroupModel::Area { area, .. } => *area,
            GroupModel::Classes { model, shift } => model.straight() + shift,
        }
    }

    fn weight(&self) -> Option<f64> {
        match self {
            GroupModel::Area { weight, .. } => Some(*weight),
            GroupModel::Classes { .. } => None,
        }
    }
}

/// What one master needs to space its layers: the band and the model of each group (lowercase,
/// capitals), and the clipping depth. Methods other than optical have no models.
struct MasterSpacing {
    bands: [Band; 2],
    depth: f64,
    models: Option<[GroupModel; 2]>,
}

/// A side of a control glyph: its clipped profile, the white outside its extreme, and how far the
/// glyph's bounds reach past it.
struct ControlSide {
    side: Side,
    profile: profile::SideProfile,
    white: f64,
    overhang: f64,
}

fn band(font: &Font, m: &Master, height: &str, default: f64) -> Band {
    let base = font.metric(m, metric::BASELINE).unwrap_or_default();
    let top = font.metric(m, height).unwrap_or(MetricValue { pos: default, over: 0.0 });
    let bottom = base.pos + base.over.min(0.0);
    Band { bottom, top: (top.pos + top.over.max(0.0)).max(bottom) }
}

impl MasterSpacing {
    /// `references` are the reference glyphs of each group. Reference glyphs without a usable
    /// layer are added to `skipped`. `Err` names the master when its bands cannot be sampled, or
    /// when an area is needed and neither group has a control glyph.
    fn new(
        font: &Font,
        m: &Master,
        opts: &Options,
        references: &[Vec<String>; 2],
        skipped: &mut Vec<Value>,
    ) -> std::result::Result<MasterSpacing, String> {
        let metrics = master_metrics(font, m);
        let bands = [band(font, m, metric::X_HEIGHT, metrics.x_height), band(font, m, metric::CAP_HEIGHT, metrics.cap_height)];
        let depth = |model: Model| {
            let share = if model == Model::Classes { CLASS_DEPTH_SHARE } else { AREA_DEPTH_SHARE };
            opts.depth.unwrap_or((share * metrics.x_height).max(0.0))
        };
        let mut ms = MasterSpacing { bands, depth: depth(opts.model), models: None };
        if opts.method != Method::Optical {
            return Ok(ms);
        }
        if opts.model == Model::Classes {
            ms.models = ms.classes(font, m, opts, references, skipped)?;
        }
        if ms.models.is_none() {
            ms.depth = depth(Model::Area);
            ms.models = ms.areas(font, m, opts)?;
        }
        if ms.models.is_none() {
            return Err(format!("master `{}` has none of n, o, v, H, O and V to calibrate on; give `area` or `separation`", m.name));
        }
        Ok(ms)
    }

    /// The model this master's glyphs are spaced by.
    fn model(&self) -> Option<Model> {
        match self.models.as_ref()? {
            [GroupModel::Classes { .. }, _] => Some(Model::Classes),
            [GroupModel::Area { .. }, _] => Some(Model::Area),
        }
    }

    /// The sides with ink of glyph `name` over the band of `group`, as they are spaced.
    fn control_sides(&self, font: &Font, m: &Master, group: usize, name: &str, step: f64) -> std::result::Result<Vec<ControlSide>, String> {
        let mut sides = Vec::new();
        let Some(band) = self.bands.get(group) else { return Ok(sides) };
        let Some(l) = font.glyph(name).and_then(|g| g.layers.iter().find(|l| l.id == m.id)) else { return Ok(sides) };
        let Ok(outline) = spacing::measure::measured_outline(font, l) else { return Ok(sides) };
        let (left, right) = profile::profiles(&outline, *band, step).map_err(|e| format!("master `{}`: {e}", m.name))?;
        let bounds = fontcraft_geom::Shape::bounding_box(&outline);
        for p in [left, right] {
            let Some(x) = p.extreme() else { continue };
            let (white, overhang) = match p.side {
                Side::Left => (x, x - bounds.x0),
                Side::Right => (l.width - x, bounds.x1 - x),
            };
            sides.push(ControlSide { side: p.side, profile: profile::clip_depth(&p, self.depth), white, overhang });
        }
        Ok(sides)
    }

    /// The class models of both groups; `None` when the master has no control at all.
    fn classes(
        &self,
        font: &Font,
        m: &Master,
        opts: &Options,
        references: &[Vec<String>; 2],
        skipped: &mut Vec<Value>,
    ) -> std::result::Result<Option<[GroupModel; 2]>, String> {
        let mut found = [None, None];
        for (group, ((names, refs), slot)) in CONTROLS.iter().zip(references).zip(&mut found).enumerate() {
            let mut controls = Vec::new();
            let refs = refs.iter().map(String::as_str).filter(|r| !names.contains(r));
            for (name, reference) in names.iter().map(|n| (*n, false)).chain(refs.map(|r| (r, true))) {
                let before = controls.len();
                for c in self.control_sides(font, m, group, name, opts.step)? {
                    let Some(shape) = SideShape::new(&c.profile, self.depth) else { continue };
                    controls.push(Control { glyph: name.to_string(), side: c.side, shape, white: c.white, overhang: c.overhang });
                }
                if reference && controls.len() == before {
                    skip(skipped, Some(name), Some(&m.id), "the reference glyph has no ink between the band's heights");
                }
            }
            *slot = ClassModel::new(controls);
        }
        let [lower, upper] = found;
        let (Some(lower), Some(upper)) = (lower.clone().or_else(|| upper.clone()), upper.or(lower)) else { return Ok(None) };
        Ok(Some([lower, upper].map(|model| {
            let shift = opts.area.map_or(0.0, |a| a - model.straight());
            GroupModel::Classes { model, shift }
        })))
    }

    /// The area models of both groups: the weights, and the areas the options left open,
    /// calibrated on the control glyphs. `None` when an area is open and neither group has a
    /// control glyph.
    fn areas(&self, font: &Font, m: &Master, opts: &Options) -> std::result::Result<Option<[GroupModel; 2]>, String> {
        let mut found = [None, None];
        for (group, (names, slot)) in CONTROLS.iter().zip(&mut found).enumerate() {
            // Per control side: the white outside its extreme, and its clipped mean depth.
            let mut sides = Vec::new();
            for name in names {
                for c in self.control_sides(font, m, group, name, opts.step)? {
                    if let Some(d) = profile::mean_depth(&c.profile) {
                        sides.push((c.white, d));
                    }
                }
            }
            *slot = fit(&sides);
        }
        let [lower, upper] = found;
        let model = |fitted: Option<(f64, f64)>| {
            let area = opts.area.or(fitted.map(|(a, _)| a))?;
            Some(GroupModel::Area { area, weight: fitted.map_or(1.0, |(_, w)| w) })
        };
        Ok(model(lower.or(upper)).zip(model(upper.or(lower))).map(|(l, u)| [l, u]))
    }
}

/// The area and depth weight that fit `sides` (white outside the extreme, mean depth) best:
/// the least-squares line white = area − weight × depth, the weight held between 0 and
/// [`MAX_WEIGHT`]. Sides that all have about the same depth give weight 1 and their mean white.
fn fit(sides: &[(f64, f64)]) -> Option<(f64, f64)> {
    if sides.is_empty() {
        return None;
    }
    let n = sides.len() as f64;
    let mean_out = sides.iter().map(|s| s.0).sum::<f64>() / n;
    let mean_depth = sides.iter().map(|s| s.1).sum::<f64>() / n;
    let cov: f64 = sides.iter().map(|(o, d)| (o - mean_out) * (d - mean_depth)).sum();
    let var: f64 = sides.iter().map(|(_, d)| (d - mean_depth).powi(2)).sum();
    // Below a unit of spread in depth the slope means nothing.
    let weight = if var >= n { (-cov / var).clamp(0.0, MAX_WEIGHT) } else { 1.0 };
    let area = mean_out + weight * mean_depth;
    (area.is_finite() && weight.is_finite()).then_some((area, weight))
}

/// One layer's new spacing: move the contents by `shift`, then set the width.
struct Plan {
    glyph: String,
    layer: String,
    shift: f64,
    width: f64,
    left: f64,
    right: f64,
    /// The control side each side took, where the class model set it.
    classes: [Option<String>; 2],
}

/// The white the model wants outside the extreme of each side, and the control sides taken.
fn optical_whites(
    ms: &MasterSpacing,
    group: usize,
    glyph: &str,
    (left, right): (&profile::SideProfile, &profile::SideProfile),
    overhangs: [f64; 2],
) -> std::result::Result<([f64; 2], [Option<String>; 2]), String> {
    const NO_INK: &str = "it has no ink between the band's heights";
    let model = ms.models.as_ref().and_then(|g| g.get(group)).ok_or("no model")?;
    let mut whites = [0.0; 2];
    let mut classes = [None, None];
    for (((p, overhang), white), class) in [left, right].into_iter().zip(overhangs).zip(&mut whites).zip(&mut classes) {
        match model {
            GroupModel::Area { area, weight } => {
                *white = area - weight * profile::mean_depth(&profile::clip_depth(p, ms.depth)).ok_or(NO_INK)?;
            }
            GroupModel::Classes { model, shift } => {
                let shape = SideShape::new(p, ms.depth).ok_or(NO_INK)?;
                let found = model.predict(glyph, p.side, &shape, overhang).ok_or("its spacing is not a number")?;
                *white = found.white + shift;
                *class = Some(format!("{} {}", found.control.glyph, if found.control.side == Side::Left { "left" } else { "right" }));
            }
        }
    }
    Ok((whites, classes))
}

/// The new sidebearings of one layer, or why it is skipped.
fn plan_layer(font: &Font, g: &Glyph, l: &Layer, capital: bool, ms: &MasterSpacing, opts: &Options) -> std::result::Result<Plan, String> {
    if spacing::derives_metrics(font, l) {
        return Err("automatic alignment sets its metrics".into());
    }
    if spacing::effective_key(g, l, Field::Width).is_some() {
        return Err("its width has a metrics key".into());
    }
    let keep_left = spacing::effective_key(g, l, Field::Left).is_some();
    let keep_right = spacing::effective_key(g, l, Field::Right).is_some();
    if keep_left && keep_right {
        return Err("both sidebearings have metrics keys".into());
    }
    let outline = spacing::measure::measured_outline(font, l).map_err(|e| e.to_string())?;
    if outline.elements().is_empty() {
        return Err("the layer is empty".into());
    }
    let b = fontcraft_geom::Shape::bounding_box(&outline);
    if ![b.x0, b.x1].iter().all(|v| in_range(*v)) {
        return Err(format!("the outline reaches beyond ±{MAX_COORDINATE}"));
    }
    // The current sidebearings and the wanted ones, before keys, limits and rounding.
    let mut classes = [None, None];
    let (current, wanted) = match opts.method {
        Method::Optical => {
            let group = usize::from(capital);
            let band = ms.bands.get(group).copied().ok_or("no band")?;
            let (left, right) = profile::profiles(&outline, band, opts.step).map_err(|e| e.to_string())?;
            let (Some(el), Some(er)) = (left.extreme(), right.extreme()) else {
                return Err("it has no ink between the band's heights".into());
            };
            // The outline's bounds may reach past the band's extremes (an ascender, a descender).
            let (past_left, past_right) = (el - b.x0, b.x1 - er);
            let ([wl, wr], found) = optical_whites(ms, group, &g.name, (&left, &right), [past_left, past_right])?;
            classes = found;
            ((b.x0, l.width - b.x1), (wl - past_left, wr - past_right))
        }
        Method::Tight | Method::Custom => {
            let v = if opts.method == Method::Tight { 0.0 } else { opts.percent / 100.0 * f64::from(font.units_per_em) };
            let current = match opts.beam {
                Some(y) => spacing::sidebearings_at_height(font, l, y).map_err(|e| e.to_string())?.ok_or("the outline does not reach the beam")?,
                None => (b.x0, l.width - b.x1),
            };
            (current, (v, v))
        }
    };
    let settle = |keep: bool, now: f64, want: f64| -> f64 {
        if keep {
            return now;
        }
        let mut v = want.round();
        if let Some(lo) = opts.min {
            v = v.max(lo);
        }
        if let Some(hi) = opts.max {
            v = v.min(hi);
        }
        v
    };
    for (class, keep) in classes.iter_mut().zip([keep_left, keep_right]) {
        if keep {
            *class = None;
        }
    }
    let left = settle(keep_left, current.0, wanted.0);
    let right = settle(keep_right, current.1, wanted.1);
    if ![left, right].iter().all(|v| SIDEBEARING_RANGE.contains(v)) {
        return Err(format!("the sidebearings would be {left} and {right}, outside {} to {}", SIDEBEARING_RANGE.start(), SIDEBEARING_RANGE.end()));
    }
    let shift = left - current.0;
    let width = l.width + shift + (right - current.1);
    check_width(width)?;
    Ok(Plan { glyph: g.name.clone(), layer: l.id.clone(), shift, width, left, right, classes })
}

fn auto_space(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("spacing.autoSpace", v)?;
    let data = s.glyph_data()?;
    let font = s.font()?;
    let opts = options(&p, font)?;
    let names = match p.opt_glyphs("glyphs", font)? {
        Some(n) if n.is_empty() => return Err(p.bad("`glyphs` is empty")),
        Some(n) => n,
        None => default_glyphs(font, &data),
    };
    let masters = master_ids(&p, font)?;
    // The group of each glyph, looked up once for every master.
    let glyphs: Vec<(&Glyph, bool)> = names.iter().filter_map(|n| font.glyph(n)).map(|g| (g.as_ref(), is_capital(g, &data))).collect();
    let mut plans = Vec::new();
    let mut skipped = Vec::new();
    let mut models = Map::new();
    let mut areas = Map::new();
    let mut weights = Map::new();
    let mut references = [Vec::new(), Vec::new()];
    for name in &opts.references {
        let capital = font.glyph(name).is_some_and(|g| is_capital(g, &data));
        if let Some(list) = references.get_mut(usize::from(capital)) {
            list.push(name.clone());
        }
    }
    for id in &masters {
        let m = font.master(id)?;
        if let Some(source) = spacing::metrics_source(font, id)
            && source.id != *id
        {
            skip(&mut skipped, None, Some(id), format!("master `{}` takes its widths from master `{}`", m.name, source.name));
            continue;
        }
        let ms = MasterSpacing::new(font, m, &opts, &references, &mut skipped).map_err(|e| p.bad(e))?;
        if let (Some(model), Some([lower, upper])) = (ms.model(), &ms.models) {
            models.insert(id.clone(), json!(model.name()));
            areas.insert(id.clone(), json!({"lowercase": lower.area(), "capitals": upper.area()}));
            if let (Some(l), Some(u)) = (lower.weight(), upper.weight()) {
                weights.insert(id.clone(), json!({"lowercase": l, "capitals": u}));
            }
        }
        for (g, capital) in &glyphs {
            crate::work::check()?;
            if g.info.locked {
                skip(&mut skipped, Some(&g.name), Some(id), "the glyph is locked");
                continue;
            }
            let Some(l) = g.layers.iter().find(|l| l.id == *id) else {
                skip(&mut skipped, Some(&g.name), Some(id), "the glyph has no layer for this master");
                continue;
            };
            match plan_layer(font, g, l, *capital, &ms, &opts) {
                Ok(plan) => plans.push(plan),
                Err(reason) => skip(&mut skipped, Some(&g.name), Some(id), reason),
            }
        }
    }
    let mut values: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    let mut classes: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    let mut changed = Vec::new();
    let mut seen = HashSet::new();
    s.edit("Auto Space", |font| {
        let both = BothGuides::of(font);
        for plan in &plans {
            values.entry(plan.glyph.clone()).or_default().insert(plan.layer.clone(), json!({"left": plan.left, "right": plan.right}));
            let [left, right] = &plan.classes;
            if left.is_some() || right.is_some() {
                let mut taken = Map::new();
                for (side, class) in [("left", left), ("right", right)] {
                    if let Some(class) = class {
                        taken.insert(side.into(), json!(class));
                    }
                }
                classes.entry(plan.glyph.clone()).or_default().insert(plan.layer.clone(), Value::Object(taken));
            }
            let mut gm = font.glyph_mut(&plan.glyph)?;
            let Some(l) = gm.layers.iter_mut().find(|l| l.id == plan.layer) else { continue };
            if plan.shift == 0.0 && plan.width == l.width {
                continue;
            }
            let before = l.width + plan.shift;
            shift_with_right_edge(l, plan.shift, both).map_err(|e| p.bad(format!("glyph `{}`: {e}", plan.glyph)))?;
            l.width = plan.width;
            follow_width(l, before, both);
            if seen.insert(plan.glyph.as_str()) {
                changed.push(plan.glyph.clone());
            }
        }
        Ok(())
    })?;
    Ok(json!({"glyphs": changed, "values": values, "models": models, "areas": areas, "weights": weights, "classes": classes, "skipped": skipped}))
}

// ── Center in Width ────────────────────────────────────────────────────────────────────────────

/// `ratio`: the share of the space that goes left, from a number (left ÷ right) or `a:b`.
fn left_share(p: &Params) -> Result<f64> {
    let bad = || p.bad("`ratio` must be a positive number (left ÷ right) or text `a:b` with two numbers of at least 0, not both 0");
    let (a, b) = match p.get("ratio") {
        None => (1.0, 1.0),
        Some(Value::String(text)) => {
            let (a, b) = text.split_once(':').ok_or_else(bad)?;
            let a: f64 = a.trim().parse().map_err(|_| bad())?;
            let b: f64 = b.trim().parse().map_err(|_| bad())?;
            (a, b)
        }
        Some(_) => (p.f64("ratio")?, 1.0),
    };
    let ok = a.is_finite() && b.is_finite() && a >= 0.0 && b >= 0.0 && a + b > 0.0 && a <= MAX_COORDINATE && b <= MAX_COORDINATE;
    if ok { Ok(a / (a + b)) } else { Err(bad()) }
}

fn center_in_width(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("spacing.centerInWidth", v)?;
    let font = s.font()?;
    let scope = common::scope(&p, font)?;
    let share = left_share(&p)?;
    let mut plans: Vec<(String, String, f64, f64, f64)> = Vec::new();
    let mut skipped = Vec::new();
    for name in &scope.glyphs {
        let Some(g) = font.glyph(name) else { continue };
        if g.info.locked {
            if scope.single {
                return Err(EngineError::Locked(name.clone()));
            }
            skip(&mut skipped, Some(name), None, "the glyph is locked");
            continue;
        }
        for id in &scope.layers {
            let Some(l) = g.layers.iter().find(|l| l.id == *id) else {
                if scope.single {
                    return Err(p.bad(format!("glyph `{name}` has no layer `{id}`")));
                }
                skip(&mut skipped, Some(name), Some(id), "the glyph has no such layer");
                continue;
            };
            if spacing::derives_metrics(font, l) {
                skip(&mut skipped, Some(name), Some(id), "automatic alignment sets its metrics");
                continue;
            }
            let lsb = spacing::metric(font, l, Field::Left).map_err(|e| p.bad(format!("glyph `{name}`: {e}")))?;
            let rsb = spacing::metric(font, l, Field::Right).map_err(|e| p.bad(format!("glyph `{name}`: {e}")))?;
            let (Some(lsb), Some(rsb)) = (lsb, rsb) else {
                skip(&mut skipped, Some(name), Some(id), "the layer is empty");
                continue;
            };
            let left = to_grid(font, (lsb + rsb) * share);
            plans.push((name.clone(), id.clone(), left - lsb, left, lsb + rsb - left));
        }
    }
    let mut values: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    let mut changed: Vec<String> = Vec::new();
    s.edit("Center in Width", |font| {
        let both = BothGuides::of(font);
        for (name, id, shift, left, right) in &plans {
            values.entry(name.clone()).or_default().insert(id.clone(), json!({"left": left, "right": right}));
            if *shift == 0.0 {
                continue;
            }
            let mut gm = font.glyph_mut(name)?;
            let Some(l) = gm.layers.iter_mut().find(|l| l.id == *id) else { continue };
            shift_layer(l, *shift, both).map_err(|e| p.bad(format!("glyph `{name}`: {e}")))?;
            if !changed.contains(name) {
                changed.push(name.clone());
            }
        }
        Ok(())
    })?;
    Ok(json!({"glyphs": changed, "values": values, "skipped": skipped}))
}

// ── Copy Metrics ───────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum What {
    Width,
    Lsb,
    Rsb,
    VerticalAdvance,
}

impl What {
    fn name(self) -> &'static str {
        match self {
            What::Width => "width",
            What::Lsb => "lsb",
            What::Rsb => "rsb",
            What::VerticalAdvance => "verticalAdvance",
        }
    }

    fn field(self) -> Option<Field> {
        match self {
            What::Width => Some(Field::Width),
            What::Lsb => Some(Field::Left),
            What::Rsb => Some(Field::Right),
            What::VerticalAdvance => None,
        }
    }
}

/// `what`, in the order the values are applied, repeats dropped.
fn what_list(p: &Params) -> Result<Vec<What>> {
    let mut out = Vec::new();
    for w in p.strings("what")? {
        let w = match w {
            "width" => What::Width,
            "lsb" => What::Lsb,
            "rsb" => What::Rsb,
            "verticalAdvance" => What::VerticalAdvance,
            other => return Err(p.bad(format!("`what`: `{other}` is not one of width, lsb, rsb and verticalAdvance"))),
        };
        if !out.contains(&w) {
            out.push(w);
        }
    }
    if out.is_empty() {
        return Err(p.bad("`what` is empty"));
    }
    out.sort();
    Ok(out)
}

/// The layer's vertical advance: its own vertical width, else the master's ascender to its
/// descender.
fn vertical_advance(font: &Font, l: &Layer) -> f64 {
    l.info.vert_width.unwrap_or_else(|| match font.masters.iter().find(|m| m.id == l.master) {
        Some(m) => {
            let metrics = master_metrics(font, m);
            metrics.ascender - metrics.descender
        }
        None => f64::from(font.units_per_em),
    })
}

/// Removes the metrics key that `field` of layer `id` shows, wherever it is stored.
fn clear_key(g: &mut Glyph, id: &str, field: Field) {
    let scope = g.layers.iter().find(|l| l.id == id).and_then(|l| spacing::effective_key(g, l, field)).map(|(scope, _)| scope);
    let slot = |keys: &mut fontcraft_model::MetricsKeys| match field {
        Field::Left => keys.left = None,
        Field::Right => keys.right = None,
        Field::Width => keys.width = None,
        Field::Top => keys.top = None,
        Field::Bottom => keys.bottom = None,
        Field::VertWidth => keys.vert_width = None,
        Field::VertOrigin => keys.vert_origin = None,
    };
    match scope {
        Some(Scope::Local) => {
            if let Some(l) = g.layers.iter_mut().find(|l| l.id == id) {
                slot(&mut l.info.metrics_keys);
            }
        }
        Some(Scope::Glyph) => slot(&mut g.info.metrics_keys),
        None => {}
    }
}

fn copy_metrics(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("edit.copyMetrics", v)?;
    let font = s.font()?;
    let from = p.str("from")?;
    let source = font.glyph(from).ok_or_else(|| p.bad(format!("`from`: there is no glyph `{from}`")))?;
    let what = what_list(&p)?;
    let to = p.opt_glyphs("to", font)?;
    let layers = match p.opt_strings("layers")? {
        None => vec![font.default_master()?.id.clone()],
        Some(ids) if ids.is_empty() => return Err(p.bad("`layers` is empty")),
        Some(ids) => {
            let mut seen = HashSet::new();
            ids.into_iter().filter(|id| seen.insert(*id)).map(str::to_string).collect()
        }
    };
    // The values to copy, per layer id.
    let mut copied: Vec<(String, Vec<(What, f64)>)> = Vec::new();
    let mut values = Map::new();
    for id in &layers {
        let l = source.layers.iter().find(|l| l.id == *id).ok_or_else(|| p.bad(format!("`from`: glyph `{from}` has no layer `{id}`")))?;
        let mut list = Vec::new();
        for w in &what {
            let value = match w.field() {
                Some(field) => spacing::metric(font, l, field)
                    .map_err(|e| p.bad(format!("glyph `{from}`: {e}")))?
                    .ok_or_else(|| p.bad(format!("glyph `{from}` draws nothing on layer `{id}`, so it has no sidebearings")))?,
                None => vertical_advance(font, l),
            };
            list.push((*w, value));
        }
        values.insert(id.clone(), Value::Object(list.iter().map(|(w, v)| (w.name().to_string(), json!(v))).collect()));
        copied.push((id.clone(), list));
    }
    let Some(to) = to else {
        return Ok(json!({"from": from, "values": values, "glyphs": [], "skipped": []}));
    };
    if to.is_empty() {
        return Err(p.bad("`to` is empty"));
    }
    let horizontal = what.iter().any(|w| *w != What::VerticalAdvance);
    let sides = what.iter().any(|w| matches!(w, What::Lsb | What::Rsb));
    let mut skipped = Vec::new();
    let mut changed = Vec::new();
    s.edit("Copy Width / Sidebearings", |font| {
        let both = BothGuides::of(font);
        for name in &to {
            let Some(g) = font.glyph(name) else { continue };
            if g.info.locked {
                skip(&mut skipped, Some(name), None, "the glyph is locked");
                continue;
            }
            let mut touched = false;
            for (id, list) in &copied {
                let Some(l) = font.glyph(name).and_then(|g| g.layers.iter().find(|l| l.id == *id)) else {
                    skip(&mut skipped, Some(name), Some(id), "the glyph has no such layer");
                    continue;
                };
                if horizontal && spacing::derives_metrics(font, l) {
                    skip(&mut skipped, Some(name), Some(id), "automatic alignment sets its metrics");
                    continue;
                }
                if sides && spacing::metric(font, l, Field::Left).map_err(|e| p.bad(format!("glyph `{name}`: {e}")))?.is_none() {
                    skip(&mut skipped, Some(name), Some(id), "the layer is empty");
                    continue;
                }
                for (w, value) in list {
                    // Each step measures the layer as the previous one left it.
                    let current = match w.field() {
                        Some(field) => font.glyph(name).and_then(|g| g.layers.iter().find(|l| l.id == *id)).map(|l| spacing::metric(font, l, field)),
                        None => None,
                    };
                    let current = match current {
                        Some(m) => m.map_err(|e| p.bad(format!("glyph `{name}`: {e}")))?,
                        None => None,
                    };
                    let mut gm = font.glyph_mut(name)?;
                    let g: &mut Glyph = &mut gm;
                    if let Some(field) = w.field() {
                        clear_key(g, id, field);
                    }
                    let Some(l) = g.layers.iter_mut().find(|l| l.id == *id) else { continue };
                    let moved = match w {
                        What::Width => check_width(*value).map(|()| {
                            let before = l.width;
                            l.width = *value;
                            follow_width(l, before, both);
                        }),
                        What::Lsb => {
                            let d = value - current.unwrap_or(*value);
                            check_width(l.width + d).and_then(|()| shift_with_right_edge(l, d, both)).map(|()| l.width += d)
                        }
                        What::Rsb => {
                            let width = l.width + value - current.unwrap_or(*value);
                            check_width(width).map(|()| {
                                let before = l.width;
                                l.width = width;
                                follow_width(l, before, both);
                            })
                        }
                        What::VerticalAdvance => check_width(*value).map(|()| l.info.vert_width = Some(*value)),
                    };
                    moved.map_err(|e| p.bad(format!("glyph `{name}`, layer `{id}`: {e}")))?;
                    touched = true;
                }
            }
            if touched {
                changed.push(name.clone());
            }
        }
        Ok(())
    })?;
    Ok(json!({"from": from, "values": values, "glyphs": changed, "skipped": skipped}))
}
