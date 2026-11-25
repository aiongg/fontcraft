//! Higher-order interpolation (HOI) curves in a variable font (M9.33).
//!
//! Rows: `hoi.appliesToAllExports` (variable half).
//!
//! A variable font moves every point in straight lines between its sources, so a glyph whose
//! nodes keep curves gets extra sources along each curved span, drawn as a static instance there
//! is drawn ([`fontcraft_interp::hoi`]), the way an intermediate layer is compiled.
//!
//! - **Along a span**: [`SPAN_STEPS`] equal steps from the lower to the upper layer; the fewest of
//!   them are kept that leave every curved node within [`TOLERANCE`] of its curve when it moves in
//!   straight lines between them ([`fewest`]).
//! - **Beside a span**: a source's region covers every value of the axes it does not stand on, so
//!   the same axis values are added where the glyph has sources on the other axes, where the font
//!   would otherwise draw something else than the instance there.
//! - The result is checked against the instances and refined at most [`MAX_ROUNDS`] times.
//!
//! The added sources move only the outline: advances and vertical metrics vary with the glyph's
//! own layers. A glyph that needs more than the limits below keeps its straight lines, with a
//! warning.

use std::collections::{BTreeSet, HashMap, HashSet};

use fontcraft_geom::variation::VariationModel;
use fontcraft_interp::{Designspace, GlyphSources, InterpOptions, Location};
use fontcraft_model::{Font, Glyph, GlyphTable, Layer, Shape};

use crate::CompileError;

/// The steps a curved span is divided into; a span gets at most one source fewer than this.
pub const SPAN_STEPS: usize = 32;
/// How far, in font units, a node may be from its curve between two sources.
pub const TOLERANCE: f64 = 0.5;
/// Most sources added to one glyph.
pub const MAX_GLYPH_SAMPLES: usize = 192;
/// Most locations added to one font for its curves; the glyphs that need them are drawn whole at
/// each.
pub const MAX_SAMPLE_LOCATIONS: usize = 256;
/// Most curved spans of one glyph.
pub const MAX_SPANS: usize = 16;
/// Most sources of its own a glyph with curves has.
pub const MAX_OWN_SOURCES: usize = 128;
/// Most nodes with curves in one glyph.
pub const MAX_CURVED_NODES: usize = 4096;
/// Most locations one glyph is checked at.
pub const MAX_CHECKS: usize = 1024;
/// Most times the sources of one glyph are refined.
pub const MAX_ROUNDS: usize = 8;
/// Most planning done for the glyphs of one font, in [`WORK_UNIT`]s; the curves of the rest are drawn straight.
pub const MAX_PLAN_WORK: usize = 1_000_000_000;
/// Coordinates added up that count as one unit of planning.
const WORK_UNIT: usize = 16;
/// The shortest step, in normalized coordinates: sources stay apart once written in 14 fractional
/// bits. A span of less than two steps is drawn straight.
const SHORTEST_STEP: f64 = 1.0 / 1024.0;
/// Normalized coordinates this close are the same.
const SAME: f64 = 1e-9;

/// The sources a variable font gets for its glyphs' curves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Samples {
    /// The added locations (internal coordinates), each once.
    pub locations: Vec<Location>,
    /// Per glyph with curves, what it draws at which of `locations`.
    by_glyph: HashMap<String, Vec<(usize, Layer)>>,
}

impl Samples {
    /// The sources the curves of `font`'s glyphs need, `sources` holding each glyph's own.
    pub fn plan(
        font: &Font,
        designspace: &Designspace,
        sources: &HashMap<String, GlyphSources>,
        notes: &mut Vec<String>,
    ) -> Result<Samples, CompileError> {
        Samples::plan_within(font, designspace, sources, notes, MAX_PLAN_WORK)
    }

    /// [`Samples::plan`] with the most planning to do.
    pub(crate) fn plan_within(
        font: &Font,
        designspace: &Designspace,
        sources: &HashMap<String, GlyphSources>,
        notes: &mut Vec<String>,
        mut work: usize,
    ) -> Result<Samples, CompileError> {
        let mut out = Samples::default();
        for g in font.glyphs.iter() {
            crate::check_stop()?;
            if !g.layers.iter().any(fontcraft_interp::hoi::has_curves) {
                continue;
            }
            let Some(gs) = sources.get(&g.name) else { continue };
            let added = match plan_glyph(font, designspace, g, gs, &mut work) {
                Ok(added) => added,
                Err(Refused::Work) => {
                    notes.push(format!(
                        "glyph `{}`: the interpolation curves of the font take too much planning, so its nodes move in straight lines",
                        g.name
                    ));
                    continue;
                }
                Err(Refused::Limit(what, limit)) => {
                    notes.push(format!(
                        "glyph `{}`: its interpolation curves need more than {limit} {what}, so its nodes move in straight lines",
                        g.name
                    ));
                    continue;
                }
                // A glyph that does not export need not be compatible.
                Err(Refused::Error(_)) if !g.export => continue,
                Err(Refused::Error(e)) => return Err(e),
            };
            let new = added.iter().filter(|s| !out.locations.contains(&s.location)).count();
            if out.locations.len().saturating_add(new) > MAX_SAMPLE_LOCATIONS {
                notes.push(format!(
                    "glyph `{}`: the interpolation curves of the font need more than {MAX_SAMPLE_LOCATIONS} intermediate locations, so its nodes \
                     move in straight lines",
                    g.name
                ));
                continue;
            }
            let mut drawn = Vec::with_capacity(added.len());
            for s in added {
                let at = match out.locations.iter().position(|l| *l == s.location) {
                    Some(i) => i,
                    None => {
                        out.locations.push(s.location);
                        out.locations.len() - 1
                    }
                };
                drawn.push((at, s.layer));
            }
            if !drawn.is_empty() {
                out.by_glyph.insert(g.name.clone(), drawn);
            }
        }
        Ok(out)
    }

    /// Whether no glyph has curves that need sources.
    pub fn is_empty(&self) -> bool {
        self.by_glyph.is_empty()
    }

    /// The locations added to `glyph`.
    pub fn of(&self, glyph: &str) -> impl Iterator<Item = &Location> {
        self.by_glyph.get(glyph).into_iter().flatten().filter_map(move |(i, _)| self.locations.get(*i))
    }

    /// Puts what the glyphs with curves draw at `location` into `at`, the font interpolated
    /// there.
    pub fn place(&self, at: &mut Font, location: &Location) {
        let Some(here) = self.locations.iter().position(|l| l == location) else { return };
        let mut names: Vec<&String> = self.by_glyph.keys().collect();
        names.sort();
        for name in names {
            let Some((_, layer)) = self.by_glyph.get(name).and_then(|list| list.iter().find(|(i, _)| *i == here)) else { continue };
            let Some(mut glyph) = at.glyphs.get_mut(name) else { continue };
            if let Some(slot) = glyph.layers.iter_mut().find(|l| l.is_master_layer()) {
                let mut layer = layer.clone();
                layer.id.clone_from(&slot.id);
                layer.master.clone_from(&slot.master);
                *slot = layer;
            }
        }
    }

    /// `font` with only the glyphs drawn at the added locations: those with curves, the
    /// composites that reach them, and everything those are built from.
    pub fn reduced(&self, font: &Font) -> Font {
        let mut users: HashMap<String, Vec<&str>> = HashMap::new();
        for g in font.glyphs.iter() {
            for base in bases(g) {
                users.entry(base).or_default().push(g.name.as_str());
            }
        }
        let mut kept: HashSet<String> = self.by_glyph.keys().cloned().collect();
        let mut todo: Vec<String> = kept.iter().cloned().collect();
        while let Some(name) = todo.pop() {
            for user in users.get(&name).into_iter().flatten() {
                if kept.insert((*user).to_string()) {
                    todo.push((*user).to_string());
                }
            }
        }
        let mut todo: Vec<String> = kept.iter().cloned().collect();
        while let Some(name) = todo.pop() {
            for base in font.glyph(&name).map(|g| bases(g)).unwrap_or_default() {
                if kept.insert(base.clone()) {
                    todo.push(base);
                }
            }
        }
        let mut out = font.clone();
        out.glyphs = GlyphTable::from_glyphs(font.glyphs.iter().filter(|g| kept.contains(&g.name)).cloned().collect());
        out
    }
}

/// The glyphs the layers of `g` use as components.
fn bases(g: &Glyph) -> Vec<String> {
    let shapes = g.layers.iter().flat_map(|l| &l.shapes);
    shapes
        .filter_map(|s| match s {
            Shape::Component(c) => Some(c.glyph.clone()),
            _ => None,
        })
        .collect()
}

/// Why a glyph gets no sources for its curves.
enum Refused {
    /// More than so many of something.
    Limit(&'static str, usize),
    /// The planning of the font is used up ([`MAX_PLAN_WORK`]).
    Work,
    Error(CompileError),
}

impl From<CompileError> for Refused {
    fn from(e: CompileError) -> Self {
        Refused::Error(e)
    }
}

impl From<fontcraft_interp::InterpError> for Refused {
    fn from(e: fontcraft_interp::InterpError) -> Self {
        Refused::Error(e.into())
    }
}

/// A source added to a glyph, or one that could be.
#[derive(Clone)]
struct Sample {
    location: Location,
    normalized: Vec<f64>,
    /// The curved nodes' coordinates there.
    values: Vec<f64>,
    layer: Layer,
    /// The span it lies on; `None` beside one.
    span: Option<usize>,
}

/// One curved span of a glyph.
struct CurvedSpan<'a> {
    axis: usize,
    lower: &'a Location,
    upper: &'a Location,
}

/// Takes adding up `values` coordinates of each of `layers` off `work`.
fn spend(work: &mut usize, layers: usize, values: usize) -> Result<(), Refused> {
    *work = work.checked_sub(layers.saturating_mul(values) / WORK_UNIT + 1).ok_or(Refused::Work)?;
    Ok(())
}

fn plan_glyph(font: &Font, designspace: &Designspace, glyph: &Glyph, sources: &GlyphSources, work: &mut usize) -> Result<Vec<Sample>, Refused> {
    // The sources the variable font has: those within its axes.
    let mut gs = GlyphSources { glyph: sources.glyph.clone(), ..GlyphSources::default() };
    let mut own: Vec<Vec<f64>> = Vec::new();
    for s in &sources.main {
        let n = designspace.to_normalized(&s.location)?.0;
        if n.iter().all(|v| (-1.0..=1.0).contains(v)) {
            own.push(n);
            gs.main.push(s.clone());
        }
    }
    let origin = designspace.origin().map(|m| m.id.as_str());
    if !gs.main.iter().any(|s| s.is_master && Some(s.master.as_str()) == origin) {
        return Ok(Vec::new());
    }
    let layer_of = |id: &str| glyph.layers.iter().find(|l| l.id == id);
    let spans = curved_spans(designspace, &gs, layer_of)?;
    if spans.is_empty() {
        return Ok(Vec::new());
    }
    if own.len() > MAX_OWN_SOURCES {
        return Err(Refused::Limit("sources in one glyph", MAX_OWN_SOURCES));
    }
    let nodes = curved_nodes(gs.main.iter().filter_map(|s| layer_of(&s.layer_id)));
    if nodes.len() > MAX_CURVED_NODES {
        return Err(Refused::Limit("nodes with curves in one glyph", MAX_CURVED_NODES));
    }
    let own_values: Vec<Vec<f64>> = gs.main.iter().map(|s| layer_of(&s.layer_id).map(|l| coordinates(l, &nodes)).unwrap_or_default()).collect();
    let options = InterpOptions::default();
    let size = gs.main.iter().filter_map(|s| layer_of(&s.layer_id)).map(layer_size).max().unwrap_or(0);
    let draw = |location: Location, span: Option<usize>, work: &mut usize| -> Result<Sample, Refused> {
        crate::check_stop()?;
        spend(work, gs.main.len(), size)?;
        let normalized = designspace.to_normalized(&location)?.0;
        let layer = fontcraft_interp::instance::interpolate_layer_in(font, designspace, &gs, &location, &options)?;
        Ok(Sample { values: coordinates(&layer, &nodes), location, normalized, layer, span })
    };

    // Along each span: the fewest steps, the others kept to check the result with.
    let mut added: Vec<Sample> = Vec::new();
    let mut checks: Vec<Sample> = Vec::new();
    for (k, span) in spans.iter().enumerate() {
        let ends = (designspace.to_normalized(span.lower)?.0, designspace.to_normalized(span.upper)?.0);
        let length = match (ends.0.get(span.axis), ends.1.get(span.axis)) {
            (Some(a), Some(b)) => (b - a).abs(),
            _ => continue,
        };
        let steps = ((length / SHORTEST_STEP).floor() as usize).min(SPAN_STEPS);
        if steps < 2 {
            continue;
        }
        // Twice as many positions, so a step's middle is measured too.
        let mut along = Vec::with_capacity(2 * steps + 1);
        for i in 0..=2 * steps {
            along.push(draw(between(span, i as f64 / (2 * steps) as f64), Some(k), work)?);
        }
        let parameters: Vec<f64> = along.iter().map(|s| s.normalized.get(span.axis).copied().unwrap_or(0.0)).collect();
        let values: Vec<&[f64]> = along.iter().map(|s| s.values.as_slice()).collect();
        let kept = fewest(&parameters, &values);
        for (i, sample) in along.into_iter().enumerate() {
            if i == 0 || i == 2 * steps || !i.is_multiple_of(2) {
                continue;
            }
            if kept.contains(&(i / 2)) { added.push(sample) } else { checks.push(sample) }
        }
    }

    // Beside each span: the same axis value where the glyph has sources on the other axes.
    let mut beside: Vec<Sample> = Vec::new();
    for s in &added {
        let Some(axis) = s.span.and_then(|k| spans.get(k)).map(|span| span.axis) else { continue };
        let Some(&value) = s.location.0.get(axis) else { continue };
        for (place, n) in gs.main.iter().zip(&own) {
            let elsewhere = n.iter().zip(&s.normalized).enumerate().any(|(b, (p, q))| b != axis && (p - q).abs() > SAME);
            if !elsewhere {
                continue;
            }
            let mut location = place.location.clone();
            let Some(slot) = location.0.get_mut(axis) else { continue };
            *slot = value;
            let normalized = designspace.to_normalized(&location)?.0;
            let known = own.iter().any(|o| same(o, &normalized))
                || [&added, &checks, &beside].into_iter().flatten().any(|o: &Sample| same(&o.normalized, &normalized));
            if known {
                continue;
            }
            if checks.len().saturating_add(beside.len()) >= MAX_CHECKS {
                return Err(Refused::Limit("locations to check in one glyph", MAX_CHECKS));
            }
            beside.push(draw(location, None, work)?);
        }
    }
    checks.append(&mut beside);

    for _ in 0..MAX_ROUNDS {
        if added.len() > MAX_GLYPH_SAMPLES {
            break;
        }
        let locations: Vec<Vec<f64>> = own.iter().chain(added.iter().map(|s| &s.normalized)).cloned().collect();
        let masters: Vec<Vec<f64>> = own_values.iter().chain(added.iter().map(|s| &s.values)).cloned().collect();
        let model = VariationModel::new(&locations, false).map_err(|e| model_error(glyph, &e))?;
        // Beside the spans every location that is off, along each span the one furthest off.
        let mut wanted: BTreeSet<usize> = BTreeSet::new();
        let mut worst: HashMap<usize, (f64, usize)> = HashMap::new();
        for (i, check) in checks.iter().enumerate() {
            crate::check_stop()?;
            spend(work, masters.len(), check.values.len())?;
            let drawn = model.interpolate(&check.normalized, &masters).map_err(|e| model_error(glyph, &e))?;
            let off = drawn.iter().zip(&check.values).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
            if off <= TOLERANCE {
                continue;
            }
            match check.span {
                None => {
                    wanted.insert(i);
                }
                Some(k) if worst.get(&k).is_none_or(|(w, _)| off > *w) => {
                    worst.insert(k, (off, i));
                }
                Some(_) => {}
            }
        }
        wanted.extend(worst.values().map(|(_, i)| *i));
        if wanted.is_empty() {
            break;
        }
        let (mut kept, mut rest) = (Vec::with_capacity(wanted.len()), Vec::with_capacity(checks.len()));
        for (i, check) in checks.into_iter().enumerate() {
            if wanted.contains(&i) { kept.push(check) } else { rest.push(check) }
        }
        added.append(&mut kept);
        checks = rest;
    }
    if added.len() > MAX_GLYPH_SAMPLES {
        return Err(Refused::Limit("added sources in one glyph", MAX_GLYPH_SAMPLES));
    }
    Ok(added)
}

/// How many coordinates of nodes a layer has.
fn layer_size(layer: &Layer) -> usize {
    layer.shapes.iter().map(|s| if let Shape::Path(p) = s { p.nodes.len().saturating_mul(2) } else { 1 }).sum()
}

fn model_error(glyph: &Glyph, e: &fontcraft_geom::variation::VariationError) -> CompileError {
    CompileError::Glyph { glyph: glyph.name.clone(), message: e.to_string() }
}

fn same(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(p, q)| (p - q).abs() <= SAME)
}

/// The location at `s` (0 to 1) from the span's lower to its upper end.
fn between(span: &CurvedSpan<'_>, s: f64) -> Location {
    let mut out = span.lower.clone();
    if let (Some(slot), Some(upper)) = (out.0.get_mut(span.axis), span.upper.0.get(span.axis)) {
        *slot += (upper - *slot) * s;
    }
    out
}

/// The spans of `sources` on whose lower layer some node keeps a curve.
fn curved_spans<'a, 'l>(
    designspace: &Designspace,
    sources: &'a GlyphSources,
    layer_of: impl Fn(&str) -> Option<&'l Layer>,
) -> Result<Vec<CurvedSpan<'a>>, Refused> {
    let mut out = Vec::new();
    for lower in &sources.main {
        let Some(layer) = layer_of(&lower.layer_id) else { continue };
        for (axis, info) in designspace.axes().iter().enumerate() {
            let curved = layer.shapes.iter().any(|s| match s {
                Shape::Path(p) => p.nodes.iter().any(|n| n.info.hoi.get(&info.tag).is_some()),
                _ => false,
            });
            if !curved {
                continue;
            }
            let Some(id) = fontcraft_interp::hoi::neighbour_layer(designspace, sources, &lower.layer_id, &info.tag, true)? else { continue };
            let Some(upper) = sources.main.iter().find(|s| s.layer_id == id) else { continue };
            if out.len() >= MAX_SPANS {
                return Err(Refused::Limit("curved spans in one glyph", MAX_SPANS));
            }
            out.push(CurvedSpan { axis, lower: &lower.location, upper: &upper.location });
        }
    }
    Ok(out)
}

/// The nodes (shape and node index) that keep a curve on some layer of `layers`.
fn curved_nodes<'l>(layers: impl Iterator<Item = &'l Layer>) -> Vec<(usize, usize)> {
    let mut out: BTreeSet<(usize, usize)> = BTreeSet::new();
    for layer in layers {
        for (k, shape) in layer.shapes.iter().enumerate() {
            if let Shape::Path(p) = shape {
                out.extend(p.nodes.iter().enumerate().filter(|(_, n)| !n.info.hoi.is_empty()).map(|(i, _)| (k, i)));
            }
        }
    }
    out.into_iter().collect()
}

/// The x and y of each of `nodes` on `layer`, 0 for one it does not have.
fn coordinates(layer: &Layer, nodes: &[(usize, usize)]) -> Vec<f64> {
    nodes
        .iter()
        .flat_map(|&(k, i)| {
            let node = match layer.shapes.get(k) {
                Some(Shape::Path(p)) => p.nodes.get(i).map(|n| [n.x, n.y]),
                _ => None,
            };
            node.unwrap_or_default()
        })
        .collect()
}

/// The fewest steps between the ends of a span that leave every value within [`TOLERANCE`] of
/// the straight lines between them. `values` are measured at the axis values `parameters`, two
/// to a step, the ends included; step `i` is at `parameters[2 * i]`. Steps next to each other are
/// always joined, so a curve that bends more than [`SPAN_STEPS`] lines can follow gets them all.
fn fewest(parameters: &[f64], values: &[&[f64]]) -> Vec<usize> {
    let steps = parameters.len().min(values.len()).saturating_sub(1) / 2;
    // Per step: the number of lines that reach it and the step the last of them starts at.
    let mut best: Vec<Option<(usize, usize)>> = vec![Some((0, 0))];
    for j in 1..=steps {
        let mut here: Option<(usize, usize)> = None;
        for i in 0..j {
            let Some((lines, _)) = best.get(i).copied().flatten() else { continue };
            if here.is_some_and(|(n, _)| lines + 1 >= n) {
                continue;
            }
            if i + 1 == j || straight(parameters, values, 2 * i, 2 * j) {
                here = Some((lines + 1, i));
            }
        }
        best.push(here);
    }
    let mut out = Vec::new();
    let mut at = steps;
    while let Some((_, from)) = best.get(at).copied().flatten() {
        if from == 0 || from >= at {
            break;
        }
        out.push(from);
        at = from;
    }
    out.reverse();
    out
}

/// Whether the values between `from` and `to` lie within [`TOLERANCE`] of the line between them.
fn straight(parameters: &[f64], values: &[&[f64]], from: usize, to: usize) -> bool {
    let (Some(a), Some(b), Some(va), Some(vb)) = (parameters.get(from), parameters.get(to), values.get(from), values.get(to)) else { return false };
    let length = b - a;
    if length == 0.0 || !length.is_finite() {
        return false;
    }
    parameters.iter().zip(values).take(to).skip(from + 1).all(|(p, v)| {
        let f = (p - a) / length;
        v.iter().zip(va.iter().zip(*vb)).all(|(v, (x, y))| (x + (y - x) * f - v).abs() <= TOLERANCE)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_span_gets_the_fewest_steps_its_curve_needs() {
        // A bulge of `height` over 32 steps, measured twice to a step.
        let at =
            |height: f64| -> Vec<Vec<f64>> { (0..=64).map(|i| f64::from(i) / 64.0).map(|s| vec![100.0 * s, height * 4.0 * s * (1.0 - s)]).collect() };
        let parameters: Vec<f64> = (0..=64).map(|i| f64::from(i) / 64.0).collect();
        let count = |height: f64| {
            let v = at(height);
            fewest(&parameters, &v.iter().map(Vec::as_slice).collect::<Vec<_>>()).len()
        };
        // A step of width h is off by height × h² at its middle.
        assert_eq!(count(0.0), 0, "a straight line needs none");
        assert_eq!(count(0.4), 0, "nor does a curve within the tolerance");
        assert_eq!(count(8.0), 3, "four equal parts of a bulge of 8 are off by exactly 0.5");
        assert_eq!(count(512.0), SPAN_STEPS - 1);
        assert_eq!(count(5000.0), SPAN_STEPS - 1, "never more than the steps");
    }
}
