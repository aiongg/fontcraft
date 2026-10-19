//! The `avar` planner, a port of fontTools 4.66 `varLib.avar.plan` (MIT).
//!
//! For each of the weight, width, slant and optical-size axes the font has, the planner measures
//! the glyphs at the axis limits, at the pins and at evenly spaced samples between them, then
//! places each requested user value where the measurement reaches the value interpolated between
//! the measurements at the ends of its stretch: logarithmically for weight and optical size,
//! linearly for width, through the tangent for slant. The planned axis is measured with its old
//! `avar` map removed; other axes stay at their defaults. A planned map whose normalised points
//! all lie within 0.01 of the identity is dropped.

use std::collections::HashMap;
use std::f64::consts::PI;

use write_fonts::tables::avar::{Avar, AxisValueMap, SegmentMaps};
use write_fonts::types::F2Dot14;

use super::fmt_g;
use super::glyphset::{GlyphSet, MAX_DRAW_WORK_PER_AXIS, Work};
use super::piecewise_linear_map;
use crate::metrics::{moment_statistics, moments};
use crate::sfnt::{SfntFont, Tag, tag_str};
use crate::variations::{FvarAxis, avar_version, fvar_axes, segment_maps};
use crate::{Result, TablesError};

/// Most target values, pins or glyphs given for one axis.
pub const MAX_PLAN_VALUES: usize = 10_000;
/// Most samples taken between two pins.
pub const MAX_SAMPLES: usize = 1_000;
/// The planner's default sample count.
pub const DEFAULT_SAMPLES: usize = 8;
/// Most entries of the measurement tables of one call, over every axis: each pin, and for each
/// stretch with a target value, its copy of the pins' measurements and its samples.
pub const MAX_PLAN_ENTRIES: usize = 1_000_000;

const WEIGHTS: [f64; 19] =
    [50.0, 100.0, 150.0, 200.0, 250.0, 300.0, 350.0, 400.0, 450.0, 500.0, 550.0, 600.0, 650.0, 700.0, 750.0, 800.0, 850.0, 900.0, 950.0];
const WIDTHS: [f64; 15] = [25.0, 37.5, 50.0, 62.5, 75.0, 87.5, 100.0, 112.5, 125.0, 137.5, 150.0, 162.5, 175.0, 187.5, 200.0];
const SIZES: [f64; 22] =
    [5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 18.0, 24.0, 30.0, 36.0, 48.0, 60.0, 72.0, 96.0, 120.0, 144.0, 192.0, 240.0, 288.0];

/// Python's `math.radians` and `math.degrees`, which divide where Rust multiplies.
fn radians(v: f64) -> f64 {
    v * (PI / 180.0)
}

fn degrees(v: f64) -> f64 {
    v / (PI / 180.0)
}

/// The default slant targets: the angles whose tangents are −1, −0.95, … 1.
fn slants() -> Vec<f64> {
    (-20..=20).map(|d| degrees((f64::from(d) / 20.0).atan())).collect()
}

/// How one axis is planned.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AxisPlan {
    /// The user values to place; `None` for the planner's defaults.
    pub values: Option<Vec<f64>>,
    /// The axis extent in design coordinates (minimum, default, maximum); the `fvar` range when
    /// `None`.
    pub design_limits: Option<(f64, f64, f64)>,
    /// `(user, design)` values that map exactly; the stretches between them are planned apart.
    pub pins: Vec<(f64, f64)>,
}

/// What [`plan_avar_with`] plans.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanOptions {
    pub weight: AxisPlan,
    pub width: AxisPlan,
    pub slant: AxisPlan,
    pub optical_size: AxisPlan,
    /// Samples between two pins; [`DEFAULT_SAMPLES`] when `None`.
    pub samples: Option<usize>,
    /// Glyph names and their weights in the averages; every glyph once when `None`.
    pub glyphs: Option<Vec<(String, f64)>>,
    /// Compare the measurements at the axis limits with the user values and warn when they
    /// disagree (planning goes on).
    pub sanitize: bool,
}

/// The plan of one axis.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AxisPlanResult {
    pub tag: Tag,
    /// The axis name the designspace fragment uses (`Weight`, `Width`, `Slant`, `OpticalSize`).
    pub name: &'static str,
    /// The `fvar` minimum, default and maximum.
    pub limits: (f64, f64, f64),
    /// User value to design value, in ascending order; empty when the plan is the identity.
    pub mapping: Vec<(f64, f64)>,
    /// The same map normalised, as written into `avar`.
    pub normalized: Vec<(f64, f64)>,
    /// The measurement at each pin, by design value.
    pub measurements: Vec<(f64, f64)>,
    /// The axis as a designspace `<axis>` element with its `<map>` children.
    pub designspace: String,
    /// What the sanitising check found.
    pub warnings: Vec<String>,
}

/// The result of [`plan_avar_with`].
#[derive(Clone, Debug, PartialEq)]
pub struct PlanResult {
    /// The font with its new `avar` table (unchanged when nothing was planned and it had none).
    pub font: SfntFont,
    /// One entry per planned axis the font has, in the planner's order.
    pub axes: Vec<AxisPlanResult>,
}

#[derive(Clone, Copy, PartialEq)]
enum Quantity {
    Weight,
    Width,
    Slant,
}

#[derive(Clone, Copy)]
enum Scale {
    Linear,
    Log,
    Degrees,
}

impl Scale {
    fn normalize(self, v: f64, lo: f64, hi: f64) -> Result<f64> {
        let t = match self {
            Scale::Linear => (v - lo) / (hi - lo),
            Scale::Log => {
                if v <= 0.0 || lo <= 0.0 || hi <= 0.0 {
                    return Err(TablesError::BadOptions("optical sizes must be greater than 0 to be planned".into()));
                }
                (v.ln() - lo.ln()) / (hi.ln() - lo.ln())
            }
            Scale::Degrees => {
                let (a, b) = (radians(lo).tan(), radians(hi).tan());
                (radians(v).tan() - a) / (b - a)
            }
        };
        finite(t, "a target value")
    }

    fn interpolate(self, t: f64, a: f64, b: f64) -> Result<f64> {
        let v = match self {
            Scale::Log => {
                if a <= 0.0 || b <= 0.0 {
                    return Err(TablesError::BadOptions("the measured weight is 0 at a pin: there is nothing to plan by".into()));
                }
                (a.ln() + t * (b.ln() - a.ln())).exp()
            }
            Scale::Linear | Scale::Degrees => a + t * (b - a),
        };
        finite(v, "an interpolated measurement")
    }
}

fn finite(v: f64, what: &str) -> Result<f64> {
    if v.is_finite() { Ok(v) } else { Err(TablesError::BadOptions(format!("{what} is not a finite number"))) }
}

/// An insertion-ordered map from numbers, compared with `==`, to numbers.
#[derive(Clone, Default)]
struct Table {
    pairs: Vec<(f64, f64)>,
    /// Each key's index in `pairs`, by its bits with −0 folded into 0. NaN keys equal nothing,
    /// so they are not indexed.
    index: HashMap<u64, usize>,
}

impl Table {
    /// `map[key] = value`.
    fn put(&mut self, key: f64, value: f64) {
        if key.is_nan() {
            self.pairs.push((key, value));
            return;
        }
        let bits = (key + 0.0).to_bits();
        match self.index.get(&bits).and_then(|&i| self.pairs.get_mut(i)) {
            Some(slot) => slot.1 = value,
            None => {
                self.index.insert(bits, self.pairs.len());
                self.pairs.push((key, value));
            }
        }
    }

    fn get(&self, key: f64) -> Result<f64> {
        (!key.is_nan())
            .then(|| self.index.get(&(key + 0.0).to_bits()))
            .flatten()
            .and_then(|&i| self.pairs.get(i))
            .map(|(_, v)| *v)
            .ok_or_else(|| TablesError::invalid("a pin has no measurement"))
    }
}

fn get(map: &[(f64, f64)], key: f64) -> Result<f64> {
    map.iter().find(|(k, _)| *k == key).map(|(_, v)| *v).ok_or_else(|| TablesError::invalid("a pin has no measurement"))
}

/// [`piecewise_linear_map`] over a mapping whose keys ascend, each once, found by bisection.
fn piecewise_sorted(v: f64, mapping: &[(f64, f64)]) -> f64 {
    let i = mapping.partition_point(|(k, _)| *k < v);
    if let Some(&(k, to)) = mapping.get(i)
        && k == v
    {
        return to;
    }
    let (Some(&(lowest, at_lowest)), Some(&(highest, at_highest))) = (mapping.first(), mapping.last()) else { return v };
    if v < lowest {
        return v + at_lowest - lowest;
    }
    if v > highest {
        return v + at_highest - highest;
    }
    match (i.checked_sub(1).and_then(|j| mapping.get(j)), mapping.get(i)) {
        (Some(&(a, va)), Some(&(b, vb))) if b != a => va + (vb - va) * (v - a) / (b - a),
        _ => v,
    }
}

/// fontTools' `normalizeValue` with clamping.
fn normalize(v: f64, (lo, de, hi): (f64, f64, f64)) -> Result<f64> {
    fontcraft_geom::variation::normalize_value(v, lo, de, hi, false)
        .ok_or_else(|| TablesError::BadOptions(format!("the limits {}:{}:{} are not in order", fmt_g(lo), fmt_g(de), fmt_g(hi))))
}

struct Measurer<'s, 'a> {
    set: &'s GlyphSet<'a>,
    axis: usize,
    limits: (f64, f64, f64),
    glyphs: &'s [(u32, f64)],
    quantity: Quantity,
}

impl Measurer<'_, '_> {
    /// The averaged measurement of the glyphs at user value `value` of the axis.
    fn at(&self, value: f64, work: &mut Work) -> Result<f64> {
        let mut coords = vec![0.0; self.set.axis_count];
        if let Some(slot) = coords.get_mut(self.axis) {
            *slot = normalize(value, self.limits)?;
        }
        let at = self.set.at(&coords)?;
        let (mut sum, mut total) = (0.0, 0.0);
        for &(gid, frequency) in self.glyphs {
            if frequency == 0.0 {
                continue;
            }
            let glyph = at.glyph(gid, work)?;
            match self.quantity {
                Quantity::Weight => {
                    let mult = glyph.width * frequency;
                    sum += mult * moments(&glyph.contours).area.abs();
                    total += mult;
                }
                Quantity::Width => {
                    sum += glyph.width * frequency;
                    total += frequency;
                }
                Quantity::Slant => {
                    let mult = glyph.width * frequency;
                    sum += mult * moment_statistics(&glyph.contours).slant;
                    total += mult;
                }
            }
        }
        if total == 0.0 {
            return Err(TablesError::BadOptions("the measured glyphs have no advance width".into()));
        }
        let mean = sum / total;
        finite(if self.quantity == Quantity::Slant { -degrees(mean.atan()) } else { mean }, "a measurement")
    }
}

struct AxisKind {
    tag: Tag,
    name: &'static str,
    quantity: Quantity,
    normalize: Scale,
    interpolate: Scale,
    defaults: Vec<f64>,
    /// Whether `sanitize` checks this axis (fontTools has no check for optical size).
    sanitized: bool,
}

fn kinds() -> [AxisKind; 4] {
    [
        AxisKind {
            tag: *b"wght",
            name: "Weight",
            quantity: Quantity::Weight,
            normalize: Scale::Linear,
            interpolate: Scale::Log,
            defaults: WEIGHTS.to_vec(),
            sanitized: true,
        },
        AxisKind {
            tag: *b"wdth",
            name: "Width",
            quantity: Quantity::Width,
            normalize: Scale::Linear,
            interpolate: Scale::Linear,
            defaults: WIDTHS.to_vec(),
            sanitized: true,
        },
        AxisKind {
            tag: *b"slnt",
            name: "Slant",
            quantity: Quantity::Slant,
            normalize: Scale::Degrees,
            interpolate: Scale::Linear,
            defaults: slants(),
            sanitized: true,
        },
        AxisKind {
            tag: *b"opsz",
            name: "OpticalSize",
            quantity: Quantity::Weight,
            normalize: Scale::Log,
            interpolate: Scale::Log,
            defaults: SIZES.to_vec(),
            sanitized: false,
        },
    ]
}

fn check_values(values: &[f64], what: &str) -> Result<()> {
    if values.len() > MAX_PLAN_VALUES {
        return Err(TablesError::TooLarge { what: "the values given for one axis", limit: MAX_PLAN_VALUES });
    }
    if values.iter().any(|v| !v.is_finite()) {
        return Err(TablesError::BadOptions(format!("a {what} is not a finite number")));
    }
    Ok(())
}

/// Plans the `avar` maps of the font's weight, width, slant and optical-size axes.
pub fn plan_avar_with(font: &SfntFont, options: &PlanOptions) -> Result<PlanResult> {
    let axes = fvar_axes(font)?;
    if avar_version(font)?.is_some_and(|v| v >= 2) {
        return Err(TablesError::Unsupported("planning a font whose avar table is version 2".into()));
    }
    let samples = options.samples.unwrap_or(DEFAULT_SAMPLES);
    if samples > MAX_SAMPLES {
        return Err(TablesError::TooLarge { what: "the sample count", limit: MAX_SAMPLES });
    }
    let set = GlyphSet::new(font, axes.len())?;
    let glyphs: Vec<(u32, f64)> = match &options.glyphs {
        None => (0..set.count).filter_map(|g| u32::try_from(g).ok()).map(|g| (g, 1.0)).collect(),
        Some(list) => {
            if list.len() > crate::limits::MAX_GLYPHS {
                return Err(TablesError::TooLarge { what: "the glyphs to measure", limit: crate::limits::MAX_GLYPHS });
            }
            let order = crate::ttx::glyph_order(font)?;
            let mut by_name: HashMap<&str, usize> = HashMap::with_capacity(order.len());
            for (i, n) in order.iter().enumerate() {
                by_name.entry(n.as_str()).or_insert(i);
            }
            let mut out = Vec::with_capacity(list.len());
            for (name, frequency) in list {
                let gid = *by_name.get(name.as_str()).ok_or_else(|| TablesError::BadOptions(format!("the font has no glyph `{name}`")))?;
                if !frequency.is_finite() {
                    return Err(TablesError::BadOptions(format!("the frequency of `{name}` is not a finite number")));
                }
                out.push((u32::try_from(gid).unwrap_or(u32::MAX), *frequency));
            }
            out
        }
    };

    let had_avar = font.get(*b"avar").is_some();
    let mut segments: Vec<(Tag, Vec<(f64, f64)>)> = if had_avar { segment_maps(font)? } else { Vec::new() };
    if segments.len() != axes.len() {
        segments = axes.iter().map(|a| (a.tag, Vec::new())).collect();
    }
    let planned = kinds().iter().filter(|k| axes.iter().any(|a| a.tag == k.tag)).count().max(1);
    let mut budget = Budget { work: Work::new(MAX_DRAW_WORK_PER_AXIS.saturating_mul(planned)), entries: MAX_PLAN_ENTRIES };
    let mut results = Vec::new();
    let mut any = false;
    for (kind, plan) in kinds().iter().zip([&options.weight, &options.width, &options.slant, &options.optical_size]) {
        let Some(index) = axes.iter().position(|a| a.tag == kind.tag) else { continue };
        let Some(axis) = axes.get(index) else { continue };
        let result = plan_axis(&set, index, axis, kind, plan, samples, &glyphs, options.sanitize, &mut budget)?;
        if let Some(slot) = segments.iter_mut().find(|(t, _)| *t == kind.tag) {
            slot.1 = result.normalized.clone();
        }
        any |= !result.normalized.is_empty();
        results.push(result);
    }

    let mut out = font.clone();
    if any || had_avar {
        let maps: Vec<SegmentMaps> = segments
            .iter()
            .map(|(_, pairs)| {
                let identity = [(-1.0, -1.0), (0.0, 0.0), (1.0, 1.0)];
                let mut pairs: Vec<(f64, f64)> = if pairs.is_empty() { identity.to_vec() } else { pairs.clone() };
                pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
                SegmentMaps::new(pairs.iter().map(|&(k, v)| AxisValueMap::new(f2dot14(k), f2dot14(v))).collect())
            })
            .collect();
        let bytes = write_fonts::dump_table(&Avar::new(maps)).map_err(|e| TablesError::invalid(format!("the avar table cannot be written: {e}")))?;
        out.insert(*b"avar", bytes);
    }
    Ok(PlanResult { font: out, axes: results })
}

/// fontTools' `fl2fi(v, 14)`: rounded half up, clamped to the F2Dot14 range.
fn f2dot14(v: f64) -> F2Dot14 {
    F2Dot14::from_bits((v * 16384.0 + 0.5).floor().clamp(-32768.0, 32767.0) as i16)
}

/// The values of `sorted` (ascending, each once) strictly between `lo` and `hi`.
fn targets_of(sorted: &[f64], lo: f64, hi: f64) -> &[f64] {
    let from = sorted.partition_point(|w| *w <= lo);
    let to = sorted.partition_point(|w| *w < hi).max(from);
    sorted.get(from..to).unwrap_or_default()
}

/// What one call of [`plan_avar_with`] may still spend, over all its axes.
struct Budget {
    work: Work,
    entries: usize,
}

#[allow(clippy::too_many_arguments)]
fn plan_axis(
    set: &GlyphSet<'_>,
    index: usize,
    axis: &FvarAxis,
    kind: &AxisKind,
    plan: &AxisPlan,
    samples: usize,
    glyphs: &[(u32, f64)],
    sanitize: bool,
    budget: &mut Budget,
) -> Result<AxisPlanResult> {
    let name = tag_str(kind.tag);
    let triple = (axis.minimum, axis.default, axis.maximum);
    let values = plan.values.clone().unwrap_or_else(|| kind.defaults.clone());
    check_values(&values, "target value")?;
    if plan.pins.len() > MAX_PLAN_VALUES {
        return Err(TablesError::TooLarge { what: "the pins of one axis", limit: MAX_PLAN_VALUES });
    }
    if plan.pins.iter().any(|(a, b)| !a.is_finite() || !b.is_finite()) {
        return Err(TablesError::BadOptions(format!("a pin of the `{}` axis is not a finite number", name.trim_end())));
    }
    let design = match plan.design_limits {
        Some(d) => {
            if !(d.0.is_finite() && d.1.is_finite() && d.2.is_finite() && d.0 <= d.1 && d.1 <= d.2) {
                return Err(TablesError::BadOptions(format!("the design limits of the `{}` axis must be finite and in order", name.trim_end())));
            }
            d
        }
        None => triple,
    };
    let mut pin_table = Table::default();
    for &(before, after) in &plan.pins {
        pin_table.put(before, after);
    }
    pin_table.put(triple.0, design.0);
    pin_table.put(triple.1, design.1);
    pin_table.put(triple.2, design.2);
    let mut pins = pin_table.pairs;
    pins.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut sorted_values = values;
    sorted_values.sort_by(f64::total_cmp);
    sorted_values.dedup();
    let stretches = pins.windows(2).filter(|w| matches!(w, [(lo, _), (hi, _)] if !targets_of(&sorted_values, *lo, *hi).is_empty())).count();
    let entries = stretches.saturating_mul(pins.len().saturating_add(samples)).saturating_add(pins.len());
    budget.entries =
        budget.entries.checked_sub(entries).ok_or(TablesError::TooLarge { what: "the measurements of one plan", limit: MAX_PLAN_ENTRIES })?;

    let measurer = Measurer { set, axis: index, limits: triple, glyphs, quantity: kind.quantity };
    let mut measurements = Table::default();
    for &(value, design_value) in &pins {
        measurements.put(design_value, measurer.at(value, &mut budget.work)?);
    }
    let warnings = if sanitize && kind.sanitized { sanitize_axis(kind.quantity, triple, design, &measurements.pairs)? } else { Vec::new() };

    let mut out = Table::default();
    let mut out_normalized = Table::default();
    for pair in pins.windows(2) {
        let [(range_min, target_min), (range_max, target_max)] = pair else { continue };
        let (range_min, target_min, range_max, target_max) = (*range_min, *target_min, *range_max, *target_max);
        let targets = targets_of(&sorted_values, range_min, range_max);
        if targets.is_empty() {
            continue;
        }
        if target_max == target_min {
            return Err(TablesError::BadOptions(format!("two pins of the `{}` axis map to the same design value", name.trim_end())));
        }
        let normalized_min = normalize(range_min, triple)?;
        let normalized_max = normalize(range_max, triple)?;
        let normalized_target_min = normalize(target_min, design)?;
        let normalized_target_max = normalize(target_max, design)?;

        let mut value_measurements = measurements.clone();
        for sample in 1..=samples {
            let value = range_min + (range_max - range_min) * sample as f64 / (samples + 1) as f64;
            let design_value = piecewise_sorted(value, &pins);
            value_measurements.put(design_value, measurer.at(value, &mut budget.work)?);
        }
        let mut sorted = value_measurements.pairs.clone();
        sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut measurement_value = Table::default();
        for (design_value, measurement) in sorted {
            measurement_value.put(measurement, design_value);
        }

        out.put(range_min, target_min);
        out_normalized.put(normalized_min, normalized_target_min);
        let at_min = value_measurements.get(target_min)?;
        let at_max = value_measurements.get(target_max)?;
        for &value in targets {
            let t = kind.normalize.normalize(value, range_min, range_max)?;
            let target_measurement = kind.interpolate.interpolate(t, at_min, at_max)?;
            let target_value = piecewise_linear_map(target_measurement, &measurement_value.pairs);
            out.put(value, target_value);
            let value_normalized = normalized_min + (value - range_min) / (range_max - range_min) * (normalized_max - normalized_min);
            let mapped =
                normalized_target_min + (target_value - target_min) / (target_max - target_min) * (normalized_target_max - normalized_target_min);
            out_normalized.put(value_normalized, finite(mapped, "a planned value")?);
        }
        out.put(range_max, target_max);
        out_normalized.put(normalized_max, normalized_target_max);
    }
    let (mut out, mut out_normalized) = (out.pairs, out_normalized.pairs);
    if out_normalized.iter().all(|(k, v)| (k - v).abs() < 0.01) {
        out.clear();
        out_normalized.clear();
    }
    out_normalized.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut snippet = format!(
        "    <axis tag=\"{}\" name=\"{}\" minimum=\"{}\" default=\"{}\" maximum=\"{}\"",
        name.trim_end(),
        kind.name,
        fmt_g(triple.0),
        fmt_g(triple.1),
        fmt_g(triple.2)
    );
    if out.is_empty() {
        snippet.push_str("/>");
    } else {
        snippet.push_str(">\n");
        for (k, v) in &out {
            snippet.push_str(&format!("      <map input=\"{}\" output=\"{}\"/>\n", fmt_g(*k), fmt_g(*v)));
        }
        snippet.push_str("    </axis>");
    }
    let mut measurements = measurements.pairs;
    measurements.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(AxisPlanResult {
        tag: kind.tag,
        name: kind.name,
        limits: triple,
        mapping: out,
        normalized: out_normalized,
        measurements,
        designspace: snippet,
        warnings,
    })
}

/// fontTools' `sanitizeWeight`, `sanitizeWidth` and `sanitizeSlant`: the warnings they log.
fn sanitize_axis(quantity: Quantity, user: (f64, f64, f64), design: (f64, f64, f64), measurements: &[(f64, f64)]) -> Result<Vec<String>> {
    let (min, default, max) = (get(measurements, design.0)?, get(measurements, design.1)?, get(measurements, design.2)?);
    let triple = |a: f64, b: f64, c: f64| format!("{}:{}:{}", fmt_g(a), fmt_g(b), fmt_g(c));
    let mut warnings = Vec::new();
    match quantity {
        Quantity::Width => {
            let lo = user.1 * (min / default);
            let hi = user.1 * (max / default);
            if (lo - user.0).abs() / user.1 > 0.05 || (hi - user.2).abs() / user.1 > 0.05 {
                warnings.push("Calculated width axis min/max do not match user input.".into());
                warnings.push(format!("  Current width axis limits: {}", triple(user.0, user.1, user.2)));
                warnings.push(format!("  Suggested width axis limits: {}", triple(lo, user.1, hi)));
            }
        }
        Quantity::Weight => {
            if user.0 == user.1 || user.1 == user.2 || user.0 == user.2 {
                return Ok(warnings);
            }
            if min <= 0.0 || default <= 0.0 || max <= 0.0 {
                warnings.push("The weight axis cannot be checked: a measured weight is 0.".into());
                return Ok(warnings);
            }
            let (log_min, log_default, log_max) = (min.ln(), default.ln(), max.ln());
            let t = (user.1 - user.0) / (user.2 - user.0);
            let y = (log_min + t * (log_max - log_min)).exp();
            let t = (y - min) / (max - min);
            let calculated_default = user.0 + t * (user.2 - user.0);
            if (calculated_default - user.1).abs() / user.1 > 0.05 {
                warnings.push("Calculated weight axis default does not match user input.".into());
                warnings.push(format!("  Current weight axis limits: {}", triple(user.0, user.1, user.2)));
                warnings.push(format!("  Suggested weight axis limits, changing default: {}", triple(user.0, calculated_default, user.2)));
                let t = (user.2 - user.0) / (user.1 - user.0);
                let y = (log_min + t * (log_default - log_min)).exp();
                let t = (y - min) / (default - min);
                let calculated_max = user.0 + t * (user.1 - user.0);
                warnings.push(format!("  Suggested weight axis limits, changing maximum: {}", triple(user.0, user.1, calculated_max)));
                let t = (user.0 - user.2) / (user.1 - user.2);
                let y = (log_max + t * (log_default - log_max)).exp();
                let t = (y - max) / (default - max);
                let calculated_min = user.2 + t * (user.1 - user.2);
                warnings.push(format!("  Suggested weight axis limits, changing minimum: {}", triple(calculated_min, user.1, user.2)));
            }
        }
        Quantity::Slant => {
            if (min - user.0).abs() > 1.0 || (default - user.1).abs() > 1.0 || (max - user.2).abs() > 1.0 {
                warnings.push("Calculated slant axis min/default/max do not match user input.".into());
                warnings.push(format!("  Current slant axis limits: {}", triple(user.0, user.1, user.2)));
                warnings.push(format!("  Suggested slant axis limits: {}", triple(min, default, max)));
            }
        }
    }
    Ok(warnings)
}
