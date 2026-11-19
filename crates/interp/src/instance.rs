//! Interpolated layers and static instance fonts (M9.4).
//!
//! Rows: `masters.metricsInterpolate`, `masters.originAutomaticForStatic`,
//! `instances.multiAxisCoordinates`, `instances.parametersStaticOnly`,
//! `alternateLayer.staticExport`, `fontInfo.customParameters.masterInterpolation`,
//! `fontInfo.stems.interpolateByName`, `fontInfo.instance.extrapolation`, `fontInfo.fontType`,
//! `instance.manualInterpolation`, `instance.instanceInterpolations`,
//! `compile.instanceInterpolation`, `compile.instanceLayoutInterpolation` (kerning),
//! `compile.kernGroupReconciliation`, `intermediateLayer.sparseInExport` (static half),
//! parameters `Local Interpolation`, `InterpolationWeightY`, `hoi.appliesToAllExports` (static
//! half).
//!
//! The kerning of an instance ([`interpolate_kerning`]) follows fontMath's kerning math
//! (mathKerning.py, MIT License, https://github.com/robotools/fontMath): the group fallback for
//! a pair a master lacks and the cleanup of pairs that come out zero.
//!
//! Contract:
//! - [`interpolate_layer`] interpolates paths, component transforms, anchors, the width and
//!   smart-component values at an internal location, through the glyph's own sub-model (its
//!   sources, without missing masters), switching alternate layers by inclusive ranges. The
//!   sources must be compatible ([`crate::compat`]); otherwise [`InterpError::Incompatible`].
//! - [`instance_font`] returns a copy of the font with a single master at the instance's location:
//!   every glyph's layer, the metrics, stems by name, number values, kerning (groups reconciled
//!   across masters) and the parameters present on every master, interpolated. Manual master
//!   weights, Local Interpolation and InterpolationWeightY apply. `compile` resolves that master
//!   as it resolves any other; `Font::resolve` keeps refusing instances between masters, so the
//!   model does not depend on this crate.
//! - Static instances outside the axis ranges extrapolate (`docs/plans/M9.md` § 7 question 6,
//!   decided: the handbook's rule) when [`InterpOptions::extrapolate`] is on; off, they are
//!   refused with [`InterpError::OutsideDesignspace`].
//! - At a master's location the master's own values come out unchanged (a weight of one is a
//!   copy), so an instance at a master differs from the master only where an alternate layer
//!   applies there.
//! - A glyph whose sources cannot interpolate fails the instance when it exports; one that does
//!   not export keeps the layer of its heaviest source, with a note ([`instance_font_with_notes`]).
//! - Nodes with higher-order interpolation curves follow them inside their span ([`crate::hoi`],
//!   `hoi.appliesToAllExports`, static half) when [`InterpOptions::hoi`] is on, as it is for
//!   static instances, the preview and `interpolation.interpolateGlyph`; the interpolated layer
//!   then keeps no curves. The variable compile's probes at intermediate locations turn it off.
//!   A glyph without curves interpolates exactly as without this step.
//! - A failure changes nothing and leaves the input untouched.
//! - A UFO or designspace source's instance also gets the font information fontmake's instancer
//!   gives it ([`ufo_instance_info`]): every number of the font information interpolated, as
//!   fontMath's `MathInfo` does, and the names of one style left out.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_geom::variation::VariationModel;
use fontcraft_model::plist::{Array, Number, Value};
use fontcraft_model::{
    Family, Font, Glyph, GlyphTable, KernContext, KernFirst, KernTable, Kerning, Layer, Master, MasterKerning, Param, Params, Shape, Transform,
};

use crate::InterpError;
use crate::compat::{self, MasterPairs};
use crate::designspace::{Designspace, Location, NormLocation, Placement};
use crate::hoi;
use crate::sources::{GlyphSources, Source};
use crate::{MAX_CACHED_MODELS, MAX_COORDINATE, MAX_GLYPHS, MAX_KERN_PAIRS, MAX_LAYER_NUMBERS};

/// Which variation model static instances use (the Font Type setting; a guess on
/// `fontInfo.fontType`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModelKind {
    /// The model anchored at the origin master, as variable fonts use it.
    #[default]
    Default,
    /// The variable font's model, with intermediate regions exactly as the font file holds them.
    Variable,
}

/// How an instance is interpolated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InterpOptions {
    /// Locations beyond the masters extrapolate; off, they are refused.
    pub extrapolate: bool,
    pub model: ModelKind,
    /// The InterpolationWeightY parameter: a separate weight-axis value for vertical coordinates.
    pub weight_y: Option<f64>,
    /// Nodes follow their higher-order interpolation curves ([`crate::hoi`]); off, every node
    /// moves in a straight line and keeps the curves its layer had.
    pub hoi: bool,
}

impl Default for InterpOptions {
    fn default() -> Self {
        Self { extrapolate: true, model: ModelKind::Default, weight_y: None, hoi: true }
    }
}

/// A weight below this, in size, counts as zero, and one this close to 1 as one.
const WEIGHT_EPSILON: f64 = 1e-12;

const WEIGHT_Y: &str = "InterpolationWeightY";
const LOCAL_INTERPOLATION: &str = "Local Interpolation";
/// Master parameters that name another master; the instance font has one master.
const MASTER_LINKS: [&str; 2] = ["Link Metrics With Master", "Link Metrics With First Master"];
const UFO_GROUPS: &str = "groups.plist";

/// A parameter value read as a number: a number, or text that is one.
fn param_number(value: &Value) -> Option<f64> {
    let v = match value {
        Value::Number(n) => n.value().as_f64(),
        Value::String(s) => s.as_str().trim().parse::<f64>().ok()?,
        _ => return None,
    };
    (v.is_finite() && v.abs() <= MAX_COORDINATE).then_some(v)
}

/// The options the instance at `instance` asks for through its parameters and the font's.
pub fn options_for(font: &Font, instance: usize) -> Result<InterpOptions, InterpError> {
    let inst = font.instances.get(instance).ok_or(InterpError::UnknownInstance(instance))?;
    let model = match font.info.settings.font_type.as_deref() {
        Some("variable") => ModelKind::Variable,
        _ => ModelKind::Default,
    };
    let weight_y = match inst.params.0.iter().find(|p| !p.disabled && p.name == WEIGHT_Y) {
        Some(p) => {
            Some(param_number(&p.value).ok_or_else(|| InterpError::BadParameter { name: WEIGHT_Y.into(), reason: "it is not a number".into() })?)
        }
        None => None,
    };
    Ok(InterpOptions { extrapolate: true, model, weight_y, hoi: true })
}

/// Whether static instance `instance` sets something only interpolation applies: manual master
/// weights, InterpolationWeightY, or Local Interpolation (on the instance or the font). Such an
/// instance goes through [`instance_font`] even at a master's location.
pub fn adjusts_interpolation(font: &Font, instance: usize) -> bool {
    let Some(inst) = font.instances.get(instance) else { return false };
    let named = |p: &fontcraft_model::Param, names: &[&str]| !p.disabled && names.contains(&p.name.as_str());
    (inst.manual_interpolation && !inst.interpolation.is_empty())
        || inst.params.0.iter().any(|p| named(p, &[WEIGHT_Y, LOCAL_INTERPOLATION]))
        || font.info.params.0.iter().any(|p| named(p, &[LOCAL_INTERPOLATION]))
}

/// The weight of each master (by id, in the designspace's order) at `location`, through the
/// variation model of the masters.
pub fn master_weights(designspace: &Designspace, location: &Location, options: &InterpOptions) -> Result<Vec<(String, f64)>, InterpError> {
    master_weights_cached(designspace, location, options, &mut ModelCache::new())
}

fn master_weights_cached(
    designspace: &Designspace,
    location: &Location,
    options: &InterpOptions,
    cache: &mut ModelCache,
) -> Result<Vec<(String, f64)>, InterpError> {
    if beyond_masters(designspace, location) && !options.extrapolate {
        return Err(InterpError::OutsideDesignspace { what: "the location".into() });
    }
    let locations = designspace.masters().iter().map(|m| designspace.to_normalized(&m.location)).collect::<Result<Vec<_>, _>>()?;
    let model = cache.get(&locations, options.extrapolate)?;
    let at = designspace.to_normalized(location)?;
    Ok(designspace.masters().iter().map(|m| m.id.clone()).zip(clean(model.master_scalars(&at.0))).collect())
}

/// Scalars with rounding noise (a weight of 1e-16 where the model means zero) set to zero.
fn clean(mut scalars: Vec<f64>) -> Vec<f64> {
    scalars.iter_mut().filter(|w| w.abs() <= WEIGHT_EPSILON).for_each(|w| *w = 0.0);
    scalars
}

/// The layer of `sources`' glyph at `location`, interpolated; its id and master are the origin
/// master's.
pub fn interpolate_layer(font: &Font, sources: &GlyphSources, location: &Location, options: &InterpOptions) -> Result<Layer, InterpError> {
    let designspace = Designspace::new(font)?;
    let origin = designspace.origin().map(|m| m.id.clone()).ok_or_else(|| InterpError::BadMasterSetup("the font has no masters".into()))?;
    let glyph = font.glyph(&sources.glyph).ok_or_else(|| InterpError::UnknownGlyph(sources.glyph.clone()))?;
    let at = At { x: location.clone(), y: y_location(&designspace, location, options) };
    let mut cache = ModelCache::new();
    let (layers, locations, weights) = glyph_inputs(&designspace, glyph, sources, &at, None, options, &mut cache)?;
    match combine(&layers, &weights, false)? {
        Some(mut layer) => {
            if options.hoi {
                apply_hoi(&designspace, &layers, &locations, &weights, &mut layer)?;
            }
            Ok(as_master_layer(layer, &origin))
        }
        None => Err(incompatible(font, sources)),
    }
}

/// `font` reduced to one master at the location of the instance at `instance` (an index into
/// `Font::instances`), everything master-bound interpolated.
pub fn instance_font(font: &Font, instance: usize, options: &InterpOptions) -> Result<Font, InterpError> {
    instance_font_with_notes(font, instance, options).map(|(f, _)| f)
}

/// [`instance_font`], with what the export report should say: that the instance was
/// extrapolated, and each glyph that could not be interpolated.
pub fn instance_font_with_notes(font: &Font, instance: usize, options: &InterpOptions) -> Result<(Font, Vec<String>), InterpError> {
    let inst = font.instances.get(instance).ok_or(InterpError::UnknownInstance(instance))?;
    let label = if inst.name.is_empty() { format!("#{instance}") } else { inst.name.clone() };
    let designspace = Designspace::new(font)?;
    let origin_info = designspace.origin().ok_or_else(|| InterpError::BadMasterSetup("the font has no masters".into()))?;
    let origin = font.master(&origin_info.id).map_err(|_| InterpError::UnknownMaster(origin_info.id.clone()))?;
    if font.glyphs.len() > MAX_GLYPHS {
        return Err(InterpError::LimitExceeded { what: "glyphs", limit: MAX_GLYPHS });
    }
    let location = designspace.instance_location(font, instance)?;
    let mut notes = Vec::new();
    if beyond_masters(&designspace, &location) {
        if !options.extrapolate {
            return Err(InterpError::OutsideDesignspace { what: format!("instance `{label}`") });
        }
        notes.push(format!("instance `{label}` lies outside the masters and was extrapolated"));
    }
    let manual: Option<HashMap<&str, f64>> = (inst.manual_interpolation && !inst.interpolation.is_empty())
        .then(|| inst.interpolation.iter().filter(|(_, w)| w.is_finite()).map(|(id, w)| (id.as_str(), *w)).collect());
    let mut cache = ModelCache::new();
    let at = At { x: location.clone(), y: y_location(&designspace, &location, options) };

    // Master-bound values.
    let weights = match &manual {
        Some(m) => {
            let w: Vec<(String, f64)> = designspace.masters().iter().map(|d| (d.id.clone(), m.get(d.id.as_str()).copied().unwrap_or(0.0))).collect();
            MasterWeights { y: w.clone(), x: w }
        }
        None => {
            let x = master_weights_cached(&designspace, &at.x, options, &mut cache)?;
            let y = match &at.y {
                Some(y) => master_weights_cached(&designspace, y, options, &mut cache)?,
                None => x.clone(),
            };
            MasterWeights { x, y }
        }
    };
    let master = interpolate_master(font, origin, &weights, &inst.location)?;

    // Glyphs.
    let local = local_interpolation(font, &designspace, inst, &location)?;
    let mut glyphs = Vec::with_capacity(font.glyphs.len());
    for g in font.glyphs.iter() {
        let sources = GlyphSources::of(font, &designspace, &g.name)?;
        let mut out = Glyph::clone(g);
        if sources.main.is_empty() {
            out.layers = g.layers.iter().filter(|l| l.master == origin.id && l.is_master_layer()).cloned().collect();
            glyphs.push(Arc::new(out));
            continue;
        }
        // As fontmake's instancer: the origin master's glyphs make the instance.
        if !sources.main.iter().any(|s| s.is_master && s.master == origin.id) {
            notes.push(format!("glyph `{}` has no layer in the origin master, so the instance leaves it out", g.name));
            continue;
        }
        let at = match &local {
            Some((names, local_at)) if names.contains(g.name.as_str()) => {
                At { x: local_at.clone(), y: local_at_y(&designspace, local_at, &location, &at) }
            }
            _ => at.clone(),
        };
        let layer = match glyph_inputs(&designspace, g, &sources, &at, manual.as_ref(), options, &mut cache).and_then(|(layers, locations, w)| {
            let heaviest = w.x.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |(i, _)| i);
            let mut combined = combine(&layers, &w, false)?;
            if let Some(layer) = combined.as_mut()
                && options.hoi
            {
                apply_hoi(&designspace, &layers, &locations, &w, layer)?;
            }
            Ok((combined, layers.get(heaviest).map(|l| Layer::clone(l))))
        }) {
            Ok((Some(layer), _)) => layer,
            Ok((None, fallback)) => {
                if g.export {
                    return Err(incompatible(font, &sources));
                }
                notes.push(format!("glyph `{}` does not export and its layers are not compatible; the instance takes its nearest layer", g.name));
                fallback.ok_or_else(|| incompatible(font, &sources))?
            }
            Err(e) if !g.export && !matches!(e, InterpError::LimitExceeded { .. }) => {
                notes.push(format!("glyph `{}` does not export and cannot be interpolated ({e}); the instance takes its first layer", g.name));
                match sources.main.first().and_then(|s| g.layers.iter().find(|l| l.id == s.layer_id)) {
                    Some(l) => l.clone(),
                    None => continue,
                }
            }
            Err(e) => return Err(e),
        };
        out.layers = vec![as_master_layer(layer, &origin.id)];
        glyphs.push(Arc::new(out));
    }

    // A designspace's own rules swap glyphs in a static instance, as fontmake's instancer does.
    if fontcraft_model::is_ufo_family(font.source.format)
        && let Some(rules) = crate::doc_rules::document_rules(font)
    {
        if rules.truncated {
            notes.push(crate::doc_rules::cut_note());
        }
        let names: HashSet<String> = glyphs.iter().map(|g| g.name.clone()).collect();
        let swaps = crate::doc_rules::swaps_at(&rules, &inst.location, |n| names.contains(n));
        for (old, new) in crate::doc_rules::apply_swaps(&mut glyphs, &swaps) {
            notes
                .push(format!("the designspace rule that substitutes `{new}` for `{old}` names a glyph the instance does not have and was left out"));
        }
    }

    let mut out = font.clone();
    let mut master = master;
    if fontcraft_model::is_ufo_family(font.source.format) {
        ufo_instance_info(font, &inst.location, &weights.x, &mut out, &mut master);
    }
    out.kerning = Arc::new(interpolate_kerning(font, &weights.x, &origin.id)?);
    out.masters = Arc::new(vec![master]);
    out.glyphs = GlyphTable::from_glyphs(glyphs);
    Ok((out, notes))
}

/// Whether `location` lies beyond the masters on some axis: outside the axis ranges, or inside a
/// range an axis mapping widened past the masters. Either way the model extrapolates there.
fn beyond_masters(designspace: &Designspace, location: &Location) -> bool {
    designspace.classify(location) == Placement::Outside || !axes_beyond_masters(designspace, location).is_empty()
}

/// The smallest and the largest of `values`; the minimum is above the maximum when there are none.
fn span(values: impl Iterator<Item = f64>) -> (f64, f64) {
    values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| (lo.min(x), hi.max(x)))
}

/// The axes (indexes into the designspace's axes) on which `location` lies beyond the masters:
/// outside the axis range, or inside a range an axis mapping widened past the masters.
pub fn axes_beyond_masters(designspace: &Designspace, location: &Location) -> Vec<usize> {
    // One axis's range is tested by moving the origin along that axis alone, which needs an
    // origin inside every range.
    let origin = designspace.origin().map(|m| &m.location).filter(|o| designspace.classify(o) == Placement::Inside);
    location
        .0
        .iter()
        .enumerate()
        .filter(|&(i, &v)| {
            let off_range = origin.is_some_and(|o| {
                let mut probe = o.clone();
                let Some(slot) = probe.0.get_mut(i) else { return false };
                *slot = v;
                designspace.classify(&probe) == Placement::Outside
            });
            let (lo, hi) = span(designspace.masters().iter().filter_map(|m| m.location.0.get(i).copied()));
            off_range || (lo <= hi && (v < lo || v > hi))
        })
        .map(|(i, _)| i)
        .collect()
}

/// For each of `instances` (indexes into `Font::instances`), the axes (indexes into `Font::axes`)
/// on which it lies beyond the masters, so that a static export extrapolates it; the test
/// [`instance_font_with_notes`] makes. A variable font setting and an unknown index have none.
/// When the font's designspace cannot be built (two axes with one tag, an unusable mapping) or
/// the instance's location cannot be read, each coordinate is compared with the masters' range
/// on its axis.
pub fn instances_beyond_masters(font: &Font, instances: &[usize]) -> Vec<Vec<usize>> {
    let designspace = Designspace::new(font).ok();
    instances
        .iter()
        .map(|&i| {
            let Some(inst) = font.instances.get(i).filter(|inst| inst.kind.as_deref() != Some("variable")) else { return Vec::new() };
            let modelled = designspace.as_ref().and_then(|ds| {
                let location = ds.instance_location(font, i).ok()?;
                Some(axes_beyond_masters(ds, &location).into_iter().filter_map(|k| ds.axes().get(k).map(|a| a.font_axis)).collect())
            });
            modelled.unwrap_or_else(|| {
                (0..font.axes.len())
                    .filter(|&k| {
                        let (lo, hi) = span(font.masters.iter().map(|m| fontcraft_model::axes::design_value(&m.location, k)));
                        let v = fontcraft_model::axes::design_value(&inst.location, k);
                        lo <= hi && (v < lo || v > hi)
                    })
                    .collect()
            })
        })
        .collect()
}

/// An internal location for horizontal values and, with InterpolationWeightY, another for
/// vertical ones.
#[derive(Clone, Debug)]
struct At {
    x: Location,
    y: Option<Location>,
}

/// The weight of each master for horizontal and for vertical values.
struct MasterWeights {
    x: Vec<(String, f64)>,
    y: Vec<(String, f64)>,
}

/// The weight of each source for horizontal and for vertical values.
#[derive(Clone, Debug, PartialEq)]
struct Weights {
    x: Vec<f64>,
    y: Vec<f64>,
}

/// The weight axis is the axis tagged `wght`.
fn weight_axis(designspace: &Designspace) -> Option<usize> {
    designspace.axis("wght").map(|(i, _)| i)
}

/// `location` with InterpolationWeightY's value on the weight axis; `None` without the
/// parameter, without a weight axis, or when the value is the instance's own.
fn y_location(designspace: &Designspace, location: &Location, options: &InterpOptions) -> Option<Location> {
    let value = options.weight_y?;
    let axis = weight_axis(designspace)?;
    let mut y = location.clone();
    let slot = y.0.get_mut(axis)?;
    if (*slot - value).abs() <= f64::EPSILON {
        return None;
    }
    *slot = value;
    Some(y)
}

/// The vertical location of a glyph Local Interpolation moves: InterpolationWeightY shifts its
/// weight by as much as it shifts the instance's.
fn local_at_y(designspace: &Designspace, local: &Location, instance: &Location, at: &At) -> Option<Location> {
    let y = at.y.as_ref()?;
    let axis = weight_axis(designspace)?;
    let shift = y.0.get(axis)? - instance.0.get(axis)?;
    let mut out = local.clone();
    *out.0.get_mut(axis)? += shift;
    Some(out)
}

/// The Local Interpolation parameter of `inst`: the glyphs it names and their location. The
/// value is one coordinate per axis, then `include:` and a comma-separated list of glyph names;
/// axes without a coordinate keep the instance's.
fn local_interpolation<'f>(
    font: &'f Font,
    designspace: &Designspace,
    inst: &'f fontcraft_model::Instance,
    location: &Location,
) -> Result<Option<(HashSet<&'f str>, Location)>, InterpError> {
    let Some(p) = inst.params.0.iter().chain(font.info.params.0.iter()).find(|p| !p.disabled && p.name == LOCAL_INTERPOLATION) else {
        return Ok(None);
    };
    let bad = |reason: &str| InterpError::BadParameter { name: LOCAL_INTERPOLATION.into(), reason: reason.into() };
    let text = p.value.as_str().ok_or_else(|| bad("it is not text"))?;
    let mut at = location.clone();
    let mut names = HashSet::new();
    let mut axis = 0;
    for part in text.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(list) = part.strip_prefix("include:") {
            names.extend(list.split(',').map(str::trim).filter(|s| !s.is_empty()));
            continue;
        }
        let v: f64 = part.parse().map_err(|_| bad("an axis value is not a number"))?;
        if !v.is_finite() || v.abs() > MAX_COORDINATE {
            return Err(bad("an axis value is not a usable number"));
        }
        let Some(font_axis) = designspace.axes().iter().position(|a| a.font_axis == axis) else {
            return Err(bad("it has more values than the font has axes"));
        };
        if let Some(slot) = at.0.get_mut(font_axis) {
            *slot = v;
        }
        axis += 1;
    }
    Ok((!names.is_empty()).then_some((names, at)))
}

/// The layers a glyph interpolates from at `at`, their locations and their weights.
fn glyph_inputs<'g>(
    designspace: &Designspace,
    glyph: &'g Glyph,
    sources: &GlyphSources,
    at: &At,
    manual: Option<&HashMap<&str, f64>>,
    options: &InterpOptions,
    cache: &mut ModelCache,
) -> Result<(Vec<&'g Layer>, Vec<Location>, Weights), InterpError> {
    let mut chosen: Vec<&Source> = sources.at(&at.x);
    // As fontmake's instancer: when the glyph is drawn at the origin, sources left empty elsewhere
    // are skipped.
    let is_empty = |s: &Source| glyph.layers.iter().find(|l| l.id == s.layer_id).is_none_or(|l| l.shapes.is_empty());
    let origin = designspace.origin().map(|m| m.id.as_str());
    if chosen.iter().any(|s| s.is_master && Some(s.master.as_str()) == origin && !is_empty(s)) {
        chosen.retain(|s| !is_empty(s));
    }
    let layers = chosen
        .iter()
        .map(|s| {
            glyph
                .layers
                .iter()
                .find(|l| l.id == s.layer_id)
                .ok_or_else(|| InterpError::UnknownLayer { glyph: glyph.name.clone(), layer: s.layer_id.clone() })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let weights = match manual {
        Some(m) => {
            let w: Vec<f64> = chosen.iter().map(|s| if s.is_master { m.get(s.master.as_str()).copied().unwrap_or(0.0) } else { 0.0 }).collect();
            Weights { y: w.clone(), x: w }
        }
        None => {
            if beyond_masters(designspace, &at.x) && !options.extrapolate {
                return Err(InterpError::OutsideDesignspace { what: format!("glyph `{}` at the instance", glyph.name) });
            }
            let locations = chosen.iter().map(|s| designspace.to_normalized(&s.location)).collect::<Result<Vec<_>, _>>()?;
            let model = cache.get(&locations, options.extrapolate)?;
            let x = clean(model.master_scalars(&designspace.to_normalized(&at.x)?.0));
            let y = match &at.y {
                Some(y) => clean(model.master_scalars(&designspace.to_normalized(y)?.0)),
                None => x.clone(),
            };
            Weights { x, y }
        }
    };
    Ok((layers, chosen.iter().map(|s| s.location.clone()).collect(), weights))
}

/// `out`, the linear interpolation of `layers` (at `locations`, internal coordinates) with
/// `weights`, with its nodes bent along the layers' higher-order interpolation curves
/// ([`crate::hoi::apply`]). Layers without curves leave it untouched.
fn apply_hoi(designspace: &Designspace, layers: &[&Layer], locations: &[Location], weights: &Weights, out: &mut Layer) -> Result<(), InterpError> {
    if !layers.iter().any(|l| hoi::has_curves(l)) {
        return Ok(());
    }
    let normalized = locations.iter().map(|l| designspace.to_normalized(l).map(|n| n.0)).collect::<Result<Vec<_>, _>>()?;
    let tags: Vec<&str> = designspace.axes().iter().map(|a| a.tag.as_str()).collect();
    hoi::apply(out, layers, &normalized, &weights.x, &weights.y, &tags)
}

fn incompatible(font: &Font, sources: &GlyphSources) -> InterpError {
    let scope = MasterPairs { masters: font.masters.iter().map(|m| m.id.clone()).collect(), indicators: true };
    InterpError::Incompatible { glyph: sources.glyph.clone(), mismatches: compat::check_glyph(font, sources, &scope).first_mismatch }
}

/// `layer` as the instance font's master layer of `master`.
fn as_master_layer(mut layer: Layer, master: &str) -> Layer {
    layer.id = master.to_string();
    layer.master = master.to_string();
    layer.info.name = None;
    layer.info.attrs.coordinates.clear();
    layer.info.attrs.axis_rules.clear();
    layer
}

/// Which numbers of a layer interpolate, read from every layer at once.
struct Plan {
    vert_width: bool,
    vert_origin: bool,
    /// Per shape: for a component, whether every layer stores it decomposed with the same scale,
    /// rotation and slant (only the position then interpolates) and the smart properties every
    /// layer sets.
    components: Vec<Option<(bool, Vec<String>)>>,
    /// Anchor names every layer has, in the first layer's order.
    anchors: Vec<String>,
}

/// The plan for `layers`, or `None` when their shapes do not correspond.
fn plan(layers: &[&Layer]) -> Option<Plan> {
    let first = layers.first()?;
    let mut components = Vec::with_capacity(first.shapes.len());
    for l in layers {
        if l.shapes.len() != first.shapes.len() {
            return None;
        }
    }
    for (i, shape) in first.shapes.iter().enumerate() {
        let others = layers.iter().map(|l| l.shapes.get(i));
        match shape {
            Shape::Path(p) => {
                for o in others {
                    match o {
                        Some(Shape::Path(q)) if q.nodes.len() == p.nodes.len() && q.closed == p.closed => {}
                        _ => return None,
                    }
                }
                components.push(None);
            }
            Shape::Component(c) => {
                let mut same_frame = true;
                let mut smart: Vec<String> = c.info.smart_values.iter().map(|v| v.property.clone()).collect();
                for o in others {
                    let Some(Shape::Component(d)) = o else { return None };
                    if d.glyph != c.glyph {
                        return None;
                    }
                    same_frame &= match (c.transform, d.transform) {
                        (Transform::Decomposed { scale, angle, slant, .. }, Transform::Decomposed { scale: s2, angle: a2, slant: sl2, .. }) => {
                            scale == s2 && angle == a2 && slant == sl2
                        }
                        _ => false,
                    };
                    smart.retain(|name| d.info.smart_values.iter().any(|v| &v.property == name));
                }
                components.push(Some((same_frame, smart)));
            }
            other => {
                for o in others {
                    if o.map(std::mem::discriminant) != Some(std::mem::discriminant(other)) {
                        return None;
                    }
                }
                components.push(None);
            }
        }
    }
    let mut anchors: Vec<String> = Vec::new();
    for a in &first.anchors {
        if !anchors.contains(&a.name) && layers.iter().all(|l| l.anchors.iter().any(|b| b.name == a.name)) {
            anchors.push(a.name.clone());
        }
    }
    Some(Plan {
        vert_width: layers.iter().all(|l| l.info.vert_width.is_some()),
        vert_origin: layers.iter().all(|l| l.info.vert_origin.is_some()),
        components,
        anchors,
    })
}

/// The numbers of `layer` in the plan's order, each with whether it is vertical.
fn numbers(layer: &Layer, plan: &Plan, out: &mut Vec<(f64, bool)>) -> Result<(), InterpError> {
    out.push((layer.width, false));
    if plan.vert_width {
        out.push((layer.info.vert_width.unwrap_or(0.0), true));
    }
    if plan.vert_origin {
        out.push((layer.info.vert_origin.unwrap_or(0.0), true));
    }
    for (shape, how) in layer.shapes.iter().zip(&plan.components) {
        match (shape, how) {
            (Shape::Path(p), _) => out.extend(p.nodes.iter().flat_map(|n| [(n.x, false), (n.y, true)])),
            (Shape::Component(c), Some((same_frame, smart))) => {
                if *same_frame {
                    if let Transform::Decomposed { pos, .. } = c.transform {
                        out.extend([(pos.0, false), (pos.1, true)]);
                    }
                } else {
                    let [xx, yx, xy, yy, dx, dy] = c.affine().as_coeffs();
                    out.extend([(xx, false), (yx, true), (xy, false), (yy, true), (dx, false), (dy, true)]);
                }
                for name in smart {
                    out.push((c.info.smart_values.iter().find(|v| &v.property == name).map_or(0.0, |v| v.value), false));
                }
            }
            _ => {}
        }
        if out.len() > MAX_LAYER_NUMBERS {
            return Err(InterpError::LimitExceeded { what: "numbers in one layer", limit: MAX_LAYER_NUMBERS });
        }
    }
    for name in &plan.anchors {
        if let Some(a) = layer.anchors.iter().find(|a| &a.name == name) {
            out.extend([(a.x, false), (a.y, true)]);
        }
    }
    Ok(())
}

/// `v` rounded half up, as fontTools' `otRound`.
fn ot_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// A weight that is one (within rounding).
fn is_one(w: f64) -> bool {
    (w - 1.0).abs() <= WEIGHT_EPSILON
}

/// `layers` combined with `weights`; `None` when they are not compatible. A source with weight
/// one and all others zero is copied. With `round`, the width, coordinates, component positions
/// and anchors are rounded as fontmake rounds an instance's glyphs; scales, angles and smart
/// values are not.
fn combine(layers: &[&Layer], weights: &Weights, round: bool) -> Result<Option<Layer>, InterpError> {
    if weights.x.len() != layers.len() || weights.y.len() != layers.len() {
        return Err(InterpError::Variation(fontcraft_geom::variation::VariationError::BadValues));
    }
    let only = |w: &[f64]| {
        let ones: Vec<usize> = w.iter().enumerate().filter(|(_, v)| is_one(**v)).map(|(i, _)| i).collect();
        (ones.len() == 1 && w.iter().filter(|v| v.abs() > WEIGHT_EPSILON).count() == 1).then(|| ones.first().copied()).flatten()
    };
    if let (Some(i), Some(j)) = (only(&weights.x), only(&weights.y))
        && i == j
        && let Some(l) = layers.get(i)
    {
        return Ok(Some(Layer::clone(l)));
    }
    let Some(plan) = plan(layers) else { return Ok(None) };
    let heaviest = weights.x.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |(i, _)| i);
    let Some(template) = layers.get(heaviest) else { return Ok(None) };
    let mut sum: Vec<(f64, bool)> = Vec::new();
    let mut scratch: Vec<(f64, bool)> = Vec::new();
    for (k, layer) in layers.iter().enumerate() {
        let (wx, wy) = (weights.x.get(k).copied().unwrap_or(0.0), weights.y.get(k).copied().unwrap_or(0.0));
        if wx == 0.0 && wy == 0.0 {
            continue;
        }
        scratch.clear();
        numbers(layer, &plan, &mut scratch)?;
        if sum.is_empty() {
            sum = scratch.iter().map(|&(v, vertical)| (v * if vertical { wy } else { wx }, vertical)).collect();
        } else if scratch.len() != sum.len() {
            return Ok(None);
        } else {
            for (s, &(v, vertical)) in sum.iter_mut().zip(&scratch) {
                s.0 += v * if vertical { wy } else { wx };
            }
        }
    }
    if sum.is_empty() {
        numbers(template, &plan, &mut sum)?;
        sum.iter_mut().for_each(|s| s.0 = 0.0);
    }
    if let Some((v, _)) = sum.iter().find(|(v, _)| !v.is_finite() || v.abs() > MAX_COORDINATE) {
        return Err(InterpError::BadNumber { what: format!("the interpolated value {v}") });
    }
    let mut out = Layer::clone(template);
    let mut values = sum.into_iter().map(|(v, _)| v);
    let mut next = || values.next().unwrap_or(0.0);
    let r = |v: f64| if round { ot_round(v) } else { v };
    out.width = r(next());
    if plan.vert_width {
        out.info.vert_width = Some(r(next()));
    }
    if plan.vert_origin {
        out.info.vert_origin = Some(r(next()));
    }
    for (shape, how) in out.shapes.iter_mut().zip(&plan.components) {
        match (shape, how) {
            (Shape::Path(p), _) => {
                for n in &mut p.nodes {
                    n.x = r(next());
                    n.y = r(next());
                }
            }
            (Shape::Component(c), Some((same_frame, smart))) => {
                if *same_frame {
                    if let Transform::Decomposed { pos, .. } = &mut c.transform {
                        *pos = (r(next()), r(next()));
                    }
                } else {
                    let [xx, yx, xy, yy] = [next(), next(), next(), next()];
                    c.transform = Transform::Matrix([xx, yx, xy, yy, r(next()), r(next())]);
                }
                for name in smart {
                    let v = next();
                    if let Some(s) = c.info.smart_values.iter_mut().find(|s| &s.property == name) {
                        s.value = v;
                    }
                }
            }
            _ => {}
        }
    }
    let mut seen = HashSet::new();
    for name in &plan.anchors {
        let (x, y) = (r(next()), r(next()));
        if let Some(a) = out.anchors.iter_mut().find(|a| &a.name == name && !seen.contains(name)) {
            a.x = x;
            a.y = y;
        }
        seen.insert(name.clone());
    }
    Ok(Some(out))
}

/// `Σ weight · value` over the masters with a weight; `None` when a weighted master has no value.
fn weighted(weights: &[(String, f64)], mut value: impl FnMut(&str) -> Option<f64>) -> Option<f64> {
    let mut sum = 0.0;
    for (id, w) in weights {
        if *w == 0.0 {
            continue;
        }
        sum += w * value(id)?;
    }
    (sum.is_finite() && sum.abs() <= MAX_COORDINATE).then_some(sum)
}

/// The instance font's master: the origin master with every value that is a number on each
/// weighted master interpolated, at the instance's location.
fn interpolate_master(font: &Font, origin: &Master, weights: &MasterWeights, location: &[f64]) -> Result<Master, InterpError> {
    let masters: HashMap<&str, &Master> = font.masters.iter().map(|m| (m.id.as_str(), m)).collect();
    let get = |id: &str| masters.get(id).copied();
    let mut out = origin.clone();
    out.location = location.to_vec();
    let metric_count = font.metrics.len().max(origin.metric_values.len());
    out.metric_values.resize(metric_count, Default::default());
    for (i, mv) in out.metric_values.iter_mut().enumerate() {
        let value = |id: &str| get(id).map(|m| m.metric_values.get(i).copied().unwrap_or_default());
        if let Some(pos) = weighted(&weights.y, |id| value(id).map(|v| v.pos)) {
            mv.pos = pos;
        }
        if let Some(over) = weighted(&weights.y, |id| value(id).map(|v| v.over)) {
            mv.over = over;
        }
    }
    for (i, v) in out.stem_values.iter_mut().enumerate() {
        // A horizontal stem is measured vertically.
        let horizontal = font.stems.get(i).is_some_and(|s| s.horizontal);
        let w = if horizontal { &weights.y } else { &weights.x };
        if let Some(sum) = weighted(w, |id| get(id).and_then(|m| m.stem_values.get(i).copied())) {
            *v = sum;
        }
    }
    for (i, v) in out.number_values.iter_mut().enumerate() {
        if let Some(sum) = weighted(&weights.x, |id| get(id).and_then(|m| m.number_values.get(i).copied())) {
            *v = sum;
        }
    }
    let zone_count = origin.zones.len();
    let same_zones = weights.y.iter().filter(|(_, w)| *w != 0.0).all(|(id, _)| get(id).is_some_and(|m| m.zones.len() == zone_count));
    if same_zones {
        for (i, z) in out.zones.iter_mut().enumerate() {
            let zone = |id: &str| get(id).and_then(|m| m.zones.get(i).copied());
            if let (Some(pos), Some(size)) = (weighted(&weights.y, |id| zone(id).map(|z| z.pos)), weighted(&weights.y, |id| zone(id).map(|z| z.size)))
            {
                z.pos = pos;
                z.size = size;
            }
        }
    }
    out.params = interpolate_params(&origin.params, &weights.x, |id| get(id).map(|m| &m.params));
    // The masters' Axis Location parameters map design to user coordinates; the one master left
    // carries the instance's user location, read through the whole font's map.
    if crate::params::axis_location(&origin.params)?.is_some() {
        let entries: Vec<crate::params::AxisLocationEntry> = font
            .axes
            .iter()
            .enumerate()
            .map(|(i, a)| crate::params::AxisLocationEntry {
                axis: a.name.clone(),
                location: fontcraft_model::axes::mapping(font, i).to_user(fontcraft_model::axes::design_value(location, i)),
            })
            .collect();
        out.params = crate::params::set_axis_location(&out.params, &entries)?;
    }
    Ok(out)
}

/// Font-information numbers fontMath does not interpolate: ufo2ft copies them from the default
/// source.
const NOT_INTERPOLATED: [&str; 6] =
    ["postscriptWindowsCharacterSet", "year", "postscriptUniqueID", "macintoshFONDFamilyID", "woffMajorVersion", "woffMinorVersion"];

/// The font information of a UFO-family source's instance (`out`, whose one master is `master`):
/// each number of the font's information that a master does not set itself is interpolated over
/// the masters, each master taking the font's value where it has none (a designspace's masters
/// hold only what differs from the default source), into the master's parameters. Unless the
/// source has one master and the instance sits at it, the names of one style go
/// ([`fontcraft_model::strip_instance_info`]).
fn ufo_instance_info(font: &Font, location: &[f64], weights: &[(String, f64)], out: &mut Font, master: &mut Master) {
    let masters: HashMap<&str, &Master> = font.masters.iter().map(|m| (m.id.as_str(), m)).collect();
    for p in font.info.params.0.iter().filter(|p| !p.disabled && !NOT_INTERPOLATED.contains(&p.name.as_str())) {
        let Value::Number(own) = &p.value else { continue };
        if master.params.get(&p.name).is_some() {
            continue;
        }
        let mut whole = true;
        let sum = weighted(weights, |id| {
            let n = masters.get(id).and_then(|m| m.params.get(&p.name)).and_then(Value::as_num).unwrap_or_else(|| own.value());
            whole &= matches!(n, fontcraft_model::plist::Num::Int(_));
            Some(n.as_f64())
        });
        let Some(sum) = sum else { continue };
        #[allow(clippy::cast_possible_truncation)]
        let number = if whole { Some(Number::int(ot_round(sum) as i64)) } else { Number::float(sum) };
        if let Some(n) = number.filter(|n| n.value().as_f64() != own.value().as_f64()) {
            master.params.0.push(Param::new(p.name.clone(), Value::Number(n)));
        }
    }
    // A number the font does not set and only some masters do: fontMath counts it as 0 where
    // a master lacks it, so the instance has it wherever a weighted master sets it.
    let mut names: Vec<&str> = Vec::new();
    for (id, _) in weights.iter().filter(|(_, w)| *w != 0.0) {
        let Some(m) = masters.get(id.as_str()) else { continue };
        for p in m.params.0.iter().filter(|p| !p.disabled && matches!(p.value, Value::Number(_))) {
            if !names.contains(&p.name.as_str()) {
                names.push(&p.name);
            }
        }
    }
    for name in names {
        if NOT_INTERPOLATED.contains(&name) || MASTER_LINKS.contains(&name) || font.info.params.get(name).is_some() {
            continue;
        }
        let of = |id: &str| masters.get(id).and_then(|m| m.params.get(name)).and_then(Value::as_num);
        let mut whole = true;
        let sum = weighted(weights, |id| {
            Some(of(id).map_or(0.0, |n| {
                whole &= matches!(n, fontcraft_model::plist::Num::Int(_));
                n.as_f64()
            }))
        });
        let Some(sum) = sum else { continue };
        #[allow(clippy::cast_possible_truncation)]
        let number = if whole { Some(Number::int(ot_round(sum) as i64)) } else { Number::float(sum) };
        master.params.0.retain(|p| p.name != name);
        // A class of 0 is not a class: the instance has none, as ufoLib2 refuses it.
        let class = matches!(name, "weightClass" | "widthClass" | "openTypeOS2WeightClass" | "openTypeOS2WidthClass");
        if let Some(n) = number.filter(|n| !(class && n.value().as_f64() < 1.0)) {
            master.params.0.push(Param::new(name.to_string(), Value::Number(n)));
        }
    }
    if font.masters.len() == 1 && font.master_at(location).is_some() {
        return;
    }
    let info = Arc::make_mut(&mut out.info);
    fontcraft_model::strip_instance_info(&mut info.properties, &mut info.params, false);
    fontcraft_model::strip_instance_info(&mut master.properties, &mut master.params, false);
}

/// `params` with each parameter that is a number, or a list of numbers of one length, on every
/// weighted master interpolated; the rest as they are. Parameters linking to another master are
/// dropped.
fn interpolate_params<'a>(params: &Params, weights: &[(String, f64)], of: impl Fn(&str) -> Option<&'a Params>) -> Params {
    let find = |id: &str, name: &str| of(id).and_then(|p| p.0.iter().find(|q| !q.disabled && q.name == name)).map(|q| &q.value);
    let mut out = params.clone();
    out.0.retain(|p| !MASTER_LINKS.contains(&p.name.as_str()));
    for p in out.0.iter_mut().filter(|p| !p.disabled) {
        match &mut p.value {
            Value::Number(n) => {
                // Whole numbers on every master stay whole, rounded as fontmake rounds them.
                let mut whole = true;
                let sum = weighted(weights, |id| {
                    let n = find(id, &p.name).and_then(Value::as_num)?;
                    whole &= matches!(n, fontcraft_model::plist::Num::Int(_));
                    Some(n.as_f64())
                });
                if let Some(sum) = sum {
                    #[allow(clippy::cast_possible_truncation)]
                    let number = if whole { Some(Number::int(ot_round(sum) as i64)) } else { Number::float(sum) };
                    if let Some(number) = number {
                        *n = number;
                    }
                }
            }
            Value::Array(Array { items, .. }) if !items.is_empty() && items.iter().all(|v| v.as_num().is_some()) => {
                let len = items.len();
                let element = |id: &str, i: usize| {
                    find(id, &p.name)
                        .and_then(Value::as_array)
                        .filter(|a| a.items.len() == len)
                        .and_then(|a| a.items.get(i))
                        .and_then(Value::as_num)
                        .map(|n| n.as_f64())
                };
                let sums: Option<Vec<Number>> = (0..len).map(|i| weighted(weights, |id| element(id, i)).and_then(Number::float)).collect();
                if let Some(sums) = sums {
                    for (item, n) in items.iter_mut().zip(sums) {
                        *item = Value::Number(n);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// One master's kerning: (first, second) in the reference's spelling → value.
type PairValues = HashMap<(String, String), f64>;

/// A master's kerning groups: glyph → group name, for first sides and for second sides. A master
/// read from its own UFO keeps that file's groups; others use the glyphs' groups.
struct Groups {
    first: HashMap<String, String>,
    second: HashMap<String, String>,
}

fn groups_of(font: &Font, master: Option<&Master>) -> Groups {
    let mut out = Groups { first: HashMap::new(), second: HashMap::new() };
    if let Some(dict) = master.and_then(|m| m.extra.get(Family::Ufo, UFO_GROUPS)).and_then(Value::as_dict) {
        for (name, members) in &dict.entries {
            let name = name.as_str();
            let (map, group) = match (name.strip_prefix("public.kern1."), name.strip_prefix("public.kern2.")) {
                (Some(g), _) => (&mut out.first, g),
                (_, Some(g)) => (&mut out.second, g),
                _ => continue,
            };
            for m in members.as_array().map(|a| a.items.as_slice()).unwrap_or_default() {
                if let Some(glyph) = m.as_str() {
                    map.entry(glyph.to_string()).or_insert_with(|| group.to_string());
                }
            }
        }
        return out;
    }
    for g in font.glyphs.iter() {
        if let Some(r) = &g.kern_groups.right {
            out.first.insert(g.name.clone(), r.clone());
        }
        if let Some(l) = &g.kern_groups.left {
            out.second.insert(g.name.clone(), l.clone());
        }
    }
    out
}

/// Kerning interpolated over the weighted masters, stored for `target`.
///
/// Left-to-right pairs: a pair one master lacks takes the value its groups give it there, as in
/// fontMath's kerning math: the pair itself, then the second side's group, the first side's
/// group, and both groups; otherwise zero. The order is the compiled kern lookup's
/// (`compile.kernValueLookup`); fontMath tries the first side's group before the second. Right-to-left and vertical pairs, and kerning
/// contexts, take the pair itself or zero.
fn interpolate_kerning(font: &Font, weights: &[(String, f64)], target: &str) -> Result<Kerning, InterpError> {
    let instance_groups = groups_of(font, None);
    let weighted_masters: Vec<(&str, f64)> = weights.iter().filter(|(_, w)| *w != 0.0).map(|(id, w)| (id.as_str(), *w)).collect();
    let table = |t: &KernTable, with_groups: bool| -> Result<KernTable, InterpError> {
        // Per master: (first, second) in the reference's spelling → value.
        let mut values: Vec<(PairValues, Groups, f64)> = Vec::new();
        let mut keys: Vec<(String, String)> = Vec::new();
        let mut seen: HashSet<(String, String)> = HashSet::new();
        for &(id, w) in &weighted_masters {
            let mut map = HashMap::new();
            if let Some(mk) = t.0.iter().find(|m| m.master == id) {
                for f in &mk.firsts {
                    for (second, v) in &f.seconds {
                        let key = (f.first.clone(), second.clone());
                        if seen.insert(key.clone()) {
                            keys.push(key);
                            if keys.len() > MAX_KERN_PAIRS {
                                return Err(InterpError::LimitExceeded { what: "kerning pairs", limit: MAX_KERN_PAIRS });
                            }
                        }
                        map.insert((fontcraft_model::glyphs_side(&f.first).into_owned(), fontcraft_model::glyphs_side(second).into_owned()), *v);
                    }
                }
            }
            let groups = if with_groups {
                groups_of(font, font.masters.iter().find(|m| m.id == id))
            } else {
                Groups { first: HashMap::new(), second: HashMap::new() }
            };
            values.push((map, groups, w));
        }
        let mut firsts: Vec<KernFirst> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for (first, second) in keys {
            let (a, b) = (fontcraft_model::glyphs_side(&first).into_owned(), fontcraft_model::glyphs_side(&second).into_owned());
            let mut sum = 0.0;
            for (map, groups, w) in &values {
                let ga = (fontcraft_model::side_group(&a).is_none()).then(|| groups.first.get(&a).map(|g| format!("@MMK_L_{g}"))).flatten();
                let gb = (fontcraft_model::side_group(&b).is_none()).then(|| groups.second.get(&b).map(|g| format!("@MMK_R_{g}"))).flatten();
                let mut candidates = vec![(a.clone(), b.clone())];
                if let Some(gb) = &gb {
                    candidates.push((a.clone(), gb.clone()));
                }
                if let Some(ga) = &ga {
                    candidates.push((ga.clone(), b.clone()));
                    if let Some(gb) = &gb {
                        candidates.push((ga.clone(), gb.clone()));
                    }
                }
                let v = candidates.iter().find_map(|k| map.get(k)).copied().unwrap_or(0.0);
                sum += w * v;
            }
            if !sum.is_finite() || sum.abs() > MAX_COORDINATE {
                return Err(InterpError::BadNumber { what: format!("the kerning of {first} {second}") });
            }
            // As fontMath's kerning cleanup: a pair that comes out zero is dropped unless one side
            // is a glyph that belongs to a group (an exception).
            let exception = (fontcraft_model::side_group(&a).is_none() && instance_groups.first.contains_key(&a))
                || (fontcraft_model::side_group(&b).is_none() && instance_groups.second.contains_key(&b));
            if sum == 0.0 && !exception {
                continue;
            }
            let sum = ot_round(sum);
            let i = *index.entry(first.clone()).or_insert_with(|| {
                firsts.push(KernFirst { first: first.clone(), seconds: Vec::new() });
                firsts.len() - 1
            });
            if let Some(f) = firsts.get_mut(i) {
                f.seconds.push((second, sum));
            }
        }
        Ok(if firsts.is_empty() { KernTable::default() } else { KernTable(vec![MasterKerning { master: target.to_string(), firsts }]) })
    };
    let k = &font.kerning;
    let context = k
        .context
        .iter()
        .filter_map(|c| {
            let v = weighted(weights, |id| c.values.iter().find(|(m, _)| m == id).map(|(_, v)| *v))?;
            Some(KernContext { values: vec![(target.to_string(), ot_round(v))], ..c.clone() })
        })
        .collect();
    Ok(Kerning { ltr: table(&k.ltr, true)?, rtl: table(&k.rtl, false)?, vertical: table(&k.vertical, false)?, context })
}

/// Variation models of a designspace's glyphs, cached by their sources' normalized locations, so
/// glyphs with the same sources share one model.
#[derive(Clone, Debug, Default)]
pub struct ModelCache {
    /// Keyed by the locations' bits and the extrapolation flag, so equal locations share an
    /// entry.
    models: HashMap<Vec<u64>, Arc<VariationModel>>,
}

impl ModelCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The model of sources at `locations`, built once. Past [`MAX_CACHED_MODELS`] entries the
    /// cache is emptied first.
    pub fn get(&mut self, locations: &[NormLocation], extrapolate: bool) -> Result<Arc<VariationModel>, InterpError> {
        let mut key: Vec<u64> = Vec::with_capacity(locations.iter().map(|l| l.0.len() + 1).sum::<usize>() + 1);
        key.push(u64::from(extrapolate));
        for l in locations {
            key.push(l.0.len() as u64);
            // −0 and 0 are the same location.
            key.extend(l.0.iter().map(|v| if *v == 0.0 { 0 } else { v.to_bits() }));
        }
        if let Some(m) = self.models.get(&key) {
            return Ok(Arc::clone(m));
        }
        let raw: Vec<Vec<f64>> = locations.iter().map(|l| l.0.clone()).collect();
        let model = Arc::new(VariationModel::new(&raw, extrapolate)?);
        if self.models.len() >= MAX_CACHED_MODELS {
            self.models.clear();
        }
        self.models.insert(key, Arc::clone(&model));
        Ok(model)
    }

    pub fn clear(&mut self) {
        self.models.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::plist::{Str, Value};
    use fontcraft_model::{
        AxisRule, Font, Glyph, Hoi, HoiAxis, HoiCurve, Instance, KernFirst, KernTable, Layer, MasterKerning, MetricValue, Node, NodeKind, Param,
        Path, Rare, Shape,
    };

    use super::*;
    use crate::sources::tests::{font_with, glyph, layer, master, rect};

    /// The width of the first rectangle of `name`'s only layer in `font`.
    fn rect_width(font: &Font, name: &str) -> f64 {
        let g = font.glyph(name).unwrap();
        assert_eq!(g.layers.len(), 1, "one layer per glyph");
        let Some(Shape::Path(p)) = g.layers[0].shapes.first() else { panic!("no path") };
        p.nodes.iter().map(|n| n.x).fold(f64::MIN, f64::max)
    }

    fn rect_height(font: &Font, name: &str) -> f64 {
        let Some(Shape::Path(p)) = font.glyph(name).unwrap().layers[0].shapes.first() else { panic!("no path") };
        p.nodes.iter().map(|n| n.y).fold(f64::MIN, f64::max)
    }

    fn instance(name: &str, at: f64) -> Instance {
        Instance { name: name.into(), location: vec![at], ..Instance::default() }
    }

    fn with_rules(mut l: Layer, min: Option<f64>, max: Option<f64>) -> Layer {
        l.info.attrs.axis_rules = vec![AxisRule { min, max }];
        l
    }

    fn kern(master: &str, pairs: &[(&str, &str, f64)]) -> MasterKerning {
        MasterKerning {
            master: master.into(),
            firsts: pairs.iter().map(|(a, b, v)| KernFirst { first: (*a).into(), seconds: vec![((*b).into(), *v)] }).collect(),
        }
    }

    /// Thin (100) and Black (900); `n` is 100 wide in Thin and 300 in Black, its advance 500
    /// and 700; instances at `at`.
    fn two(at: &[f64], extra: Vec<Glyph>) -> Font {
        let mut black = layer("black", "black", vec![rect(300.0)]);
        black.width = 700.0;
        let mut glyphs = vec![glyph("n", vec![layer("thin", "thin", vec![rect(100.0)]), black])];
        glyphs.extend(extra);
        let mut thin = master("thin", "Thin", &[100.0]);
        thin.metric_values = vec![MetricValue { pos: 500.0, over: 10.0 }];
        let mut bold = master("black", "Black", &[900.0]);
        bold.metric_values = vec![MetricValue { pos: 540.0, over: 20.0 }];
        let instances = at.iter().map(|&v| instance(&format!("I{v}"), v)).collect();
        font_with(&[("Weight", "wght")], vec![thin, bold], instances, glyphs)
    }

    #[test]
    fn instances_between_and_beyond_two_masters() {
        let mut font = two(&[400.0, 1000.0], Vec::new());
        Arc::make_mut(&mut font.kerning).ltr = KernTable::new(vec![kern("thin", &[("n", "n", -100.0)]), kern("black", &[("n", "n", -200.0)])]);
        let (regular, notes) = instance_font_with_notes(&font, 0, &InterpOptions::default()).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(regular.masters.len(), 1);
        assert_eq!(regular.masters[0].location, [400.0], "the master sits at the instance");
        assert_eq!(regular.masters[0].metric_values[0], MetricValue { pos: 515.0, over: 13.75 });
        assert_eq!(rect_width(&regular, "n"), 175.0);
        assert_eq!(regular.glyph("n").unwrap().layers[0].width, 575.0);
        assert_eq!(regular.glyph("n").unwrap().layers[0].id, "thin", "the origin master's layer");
        assert_eq!(regular.kerning.ltr.get("thin", "n", "n"), Some(-137.0), "rounded half up");
        assert!(regular.resolve(&fontcraft_model::Target::Instance(0)).is_ok(), "the instance resolves at its master");

        let (black, notes) = instance_font_with_notes(&font, 1, &InterpOptions::default()).unwrap();
        assert_eq!(rect_width(&black, "n"), 325.0, "extrapolated");
        assert!(notes.iter().any(|n| n.contains("`I1000`") && n.contains("extrapolated")), "{notes:?}");
        let refused = instance_font(&font, 1, &InterpOptions { extrapolate: false, ..InterpOptions::default() });
        assert!(matches!(refused, Err(InterpError::OutsideDesignspace { .. })), "{refused:?}");
    }

    #[test]
    fn alternate_layers_switch_at_inclusive_bounds() {
        let alt = glyph(
            "a",
            vec![
                layer("thin", "thin", vec![rect(100.0)]),
                layer("black", "black", vec![rect(300.0)]),
                with_rules(layer("thin", "thin-alt", vec![rect(1100.0)]), Some(600.0), None),
                with_rules(layer("black", "black-alt", vec![rect(1300.0)]), Some(600.0), None),
            ],
        );
        let font = two(&[599.0, 600.0, 900.0], vec![alt]);
        let width = |i: usize| rect_width(&instance_font(&font, i, &InterpOptions::default()).unwrap(), "a");
        assert_eq!(width(0), 224.75, "below the bound: the master layers");
        assert_eq!(width(1), 1225.0, "on the bound: the alternates");
        assert_eq!(width(2), 1300.0, "at the Black master: its alternate");
    }

    #[test]
    fn an_intermediate_layer_adds_a_source_and_a_third_master_narrows_the_span() {
        let mut brace = layer("thin", "brace", vec![rect(600.0)]);
        brace.info.attrs.coordinates = vec![Some(500.0)];
        let e = glyph("e", vec![layer("thin", "thin", vec![rect(100.0)]), layer("black", "black", vec![rect(300.0)]), brace]);
        let font = two(&[500.0, 300.0], vec![e]);
        let at = |i: usize| rect_width(&instance_font(&font, i, &InterpOptions::default()).unwrap(), "e");
        assert_eq!(at(0), 600.0, "at the intermediate layer");
        assert_eq!(at(1), 350.0, "between the Thin master and the intermediate layer");

        let three = font_with(
            &[("Weight", "wght")],
            vec![master("thin", "Thin", &[100.0]), master("regular", "Regular", &[400.0]), master("black", "Black", &[900.0])],
            vec![instance("SemiBold", 650.0)],
            vec![glyph(
                "n",
                vec![
                    layer("thin", "thin", vec![rect(100.0)]),
                    layer("regular", "regular", vec![rect(150.0)]),
                    layer("black", "black", vec![rect(350.0)]),
                ],
            )],
        );
        assert_eq!(rect_width(&instance_font(&three, 0, &InterpOptions::default()).unwrap(), "n"), 250.0, "Regular and Black only");
    }

    #[test]
    fn kerning_pairs_one_master_lacks_take_its_group_value() {
        let mut font = two(&[500.0], vec![glyph("o", vec![layer("thin", "thin", vec![rect(100.0)]), layer("black", "black", vec![rect(300.0)])])]);
        {
            let glyphs: Vec<Arc<Glyph>> = font
                .glyphs
                .iter()
                .map(|g| {
                    let mut g = Glyph::clone(g);
                    g.kern_groups.right = Some(g.name.clone());
                    g.kern_groups.left = Some(format!("{}_round", g.name));
                    Arc::new(g)
                })
                .collect();
            font.glyphs = fontcraft_model::GlyphTable::from_glyphs(glyphs);
        }
        Arc::make_mut(&mut font.kerning).ltr = KernTable::new(vec![
            kern("thin", &[("@MMK_L_n", "@MMK_R_o_round", -100.0)]),
            kern("black", &[("n", "o", -300.0), ("@MMK_L_n", "@MMK_R_o_round", -200.0)]),
        ]);
        let out = instance_font(&font, 0, &InterpOptions::default()).unwrap();
        assert_eq!(out.kerning.ltr.get("thin", "n", "o"), Some(-200.0), "Thin's value comes from its groups");
        assert_eq!(out.kerning.ltr.get("thin", "@MMK_L_n", "@MMK_R_o_round"), Some(-150.0));
    }

    #[test]
    fn manual_factors_weight_y_and_local_interpolation() {
        let mut font = two(&[100.0], Vec::new());
        let mut manual = instance("Manual", 100.0);
        manual.manual_interpolation = true;
        manual.interpolation = vec![("thin".into(), 0.25), ("black".into(), 0.75)];
        Arc::make_mut(&mut font.instances).push(manual);
        assert_eq!(rect_width(&instance_font(&font, 1, &InterpOptions::default()).unwrap(), "n"), 250.0, "the stored factors, not the location");

        // Black's rectangle is taller: InterpolationWeightY moves only y coordinates.
        let mut tall = layer("black", "black", vec![Shape::Path(fontcraft_model::Path::rect(0.0, 0.0, 300.0, 900.0))]);
        tall.width = 700.0;
        let mut font = two(&[100.0], vec![]);
        let mut glyphs: Vec<Arc<Glyph>> = font.glyphs.iter().cloned().collect();
        glyphs[0] = Arc::new(glyph("n", vec![layer("thin", "thin", vec![rect(100.0)]), tall]));
        glyphs.push(Arc::new(glyph("o", vec![layer("thin", "thin", vec![rect(100.0)]), layer("black", "black", vec![rect(300.0)])])));
        font.glyphs = fontcraft_model::GlyphTable::from_glyphs(glyphs);
        let inst = &mut Arc::make_mut(&mut font.instances)[0];
        inst.params.0.push(Param { name: WEIGHT_Y.into(), value: Value::Number(Number::int(900)), disabled: false, extra: Default::default() });
        inst.params.0.push(Param {
            name: LOCAL_INTERPOLATION.into(),
            value: Value::String(Str::new("900; include: o")),
            disabled: false,
            extra: Default::default(),
        });
        let options = options_for(&font, 0).unwrap();
        assert_eq!(options.weight_y, Some(900.0));
        let out = instance_font(&font, 0, &options).unwrap();
        assert_eq!((rect_width(&out, "n"), rect_height(&out, "n")), (100.0, 900.0), "x at Thin, y at Black");
        assert_eq!(rect_width(&out, "o"), 300.0, "o is drawn at 900");
    }

    #[test]
    fn a_glyph_without_an_origin_layer_is_left_out_and_an_incompatible_one_is_refused() {
        let sparse = glyph("bonus", vec![layer("black", "black", vec![rect(300.0)])]);
        let (out, notes) = instance_font_with_notes(&two(&[500.0], vec![sparse]), 0, &InterpOptions::default()).unwrap();
        assert!(out.glyph("bonus").is_none());
        assert!(notes.iter().any(|n| n.contains("`bonus`")), "{notes:?}");

        let odd = glyph("odd", vec![layer("thin", "thin", vec![rect(100.0)]), layer("black", "black", vec![rect(300.0), rect(10.0)])]);
        let refused = instance_font(&two(&[500.0], vec![odd]), 0, &InterpOptions::default());
        assert!(matches!(&refused, Err(InterpError::Incompatible { glyph, .. }) if glyph == "odd"), "{refused:?}");
    }

    /// A 700-high rectangle `right` wide whose bottom right node has `curve` on the weight axis.
    fn curved(right: f64, curve: Option<HoiCurve>) -> Shape {
        let mut nodes = vec![
            Node::new(0.0, 0.0, NodeKind::Line),
            Node::new(right, 0.0, NodeKind::Line),
            Node::new(right, 700.0, NodeKind::Line),
            Node::new(0.0, 700.0, NodeKind::Line),
        ];
        if let Some(curve) = curve {
            let mut info = (*nodes[1].info).clone();
            info.hoi = Hoi(vec![HoiAxis { axis: "wght".into(), curve }]);
            nodes[1].info = Rare::new(info);
        }
        Shape::Path(Path::new(true, nodes))
    }

    /// The nodes of the first path of `name` in the instance font `font`.
    fn nodes_of(font: &Font, name: &str) -> Vec<(f64, f64)> {
        let Some(Shape::Path(p)) = font.glyph(name).unwrap().layers[0].shapes.first() else { panic!("no path") };
        p.nodes.iter().map(|n| (n.x, n.y)).collect()
    }

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() <= 1e-9 && (a.1 - b.1).abs() <= 1e-9
    }

    #[test]
    fn a_node_with_a_curve_follows_it_in_instances() {
        let ip = HoiCurve::Point { x: 200.0, y: 100.0 };
        let o = glyph("o", vec![layer("thin", "thin", vec![curved(100.0, Some(ip))]), layer("black", "black", vec![curved(300.0, None)])]);
        let font = two(&[500.0, 300.0, 100.0], vec![o]);
        let on = InterpOptions::default();
        let off = InterpOptions { hoi: false, ..InterpOptions::default() };

        let halfway = instance_font(&font, 0, &on).unwrap();
        let nodes = nodes_of(&halfway, "o");
        assert!(close(nodes[1], (200.0, 100.0)), "through the intermediate point halfway: {nodes:?}");
        assert!(close(nodes[2], (200.0, 700.0)), "nodes without a curve move in straight lines: {nodes:?}");
        let Some(Shape::Path(p)) = halfway.glyph("o").unwrap().layers[0].shapes.first() else { panic!("no path") };
        assert!(p.nodes.iter().all(|n| n.info.hoi.is_empty()), "the instance keeps no curves");

        assert!(close(nodes_of(&instance_font(&font, 1, &on).unwrap(), "o")[1], (150.0, 75.0)), "a quarter of the way");
        assert!(close(nodes_of(&instance_font(&font, 1, &off).unwrap(), "o")[1], (150.0, 0.0)), "straight with curves off");
        assert!(close(nodes_of(&instance_font(&font, 2, &on).unwrap(), "o")[1], (100.0, 0.0)), "at the lower master");

        let designspace = Designspace::new(&font).unwrap();
        let sources = GlyphSources::of(&font, &designspace, "o").unwrap();
        let one = interpolate_layer(&font, &sources, &Location(vec![500.0]), &on).unwrap();
        let Some(Shape::Path(p)) = one.shapes.first() else { panic!("no path") };
        assert!(close((p.nodes[1].x, p.nodes[1].y), (200.0, 100.0)), "one layer interpolated the same way");

        // A glyph without curves comes out the same either way.
        for i in 0..3 {
            let (a, b) = (instance_font(&font, i, &on).unwrap(), instance_font(&font, i, &off).unwrap());
            assert_eq!(a.glyph("n").unwrap().layers, b.glyph("n").unwrap().layers, "instance {i}");
        }
    }

    #[test]
    fn a_curve_bends_only_its_own_span_on_other_axes() {
        let ip = HoiCurve::Point { x: 200.0, y: 100.0 };
        let o = glyph(
            "o",
            vec![
                layer("tn", "tn", vec![curved(100.0, Some(ip))]),
                layer("bn", "bn", vec![curved(300.0, None)]),
                layer("tw", "tw", vec![curved(100.0, None)]),
                layer("bw", "bw", vec![curved(300.0, None)]),
            ],
        );
        let masters = vec![
            master("tn", "Thin Narrow", &[100.0, 50.0]),
            master("bn", "Black Narrow", &[900.0, 50.0]),
            master("tw", "Thin Wide", &[100.0, 100.0]),
            master("bw", "Black Wide", &[900.0, 100.0]),
        ];
        let at = |wdth: f64| Instance { name: format!("W{wdth}"), location: vec![500.0, wdth], ..Instance::default() };
        let font = font_with(&[("Weight", "wght"), ("Width", "wdth")], masters, vec![at(50.0), at(75.0), at(100.0)], vec![o]);
        let node = |i: usize| nodes_of(&instance_font(&font, i, &InterpOptions::default()).unwrap(), "o")[1];
        assert!(close(node(0), (200.0, 100.0)), "on the curve's span: {:?}", node(0));
        assert!(close(node(1), (200.0, 50.0)), "halfway to the straight span: {:?}", node(1));
        assert!(close(node(2), (200.0, 0.0)), "on the span without a curve: {:?}", node(2));
    }
}
