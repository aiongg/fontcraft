//! The sources a glyph interpolates from (M9.3).
//!
//! Rows: `intermediateLayer.purpose`, `intermediateLayer.inheritsEmptyCoordinates`,
//! `alternateLayer.definition`, `alternateLayer.onePerMaster`,
//! `alternateLayer.combinedWithIntermediate`, `alternateLayer.onMasterLayer`.
//!
//! Contract:
//! - The main sources are the glyph's master layers (of the designspace's masters) and its
//!   intermediate layers. An intermediate layer's location is its `coordinates`, a missing
//!   coordinate taking its master's value. Layers of masters a setting disabled are left out.
//! - Alternate layers are grouped by their distinct range sets (at most
//!   [`crate::MAX_RANGE_SETS`]). A set's sources are its alternate layers, at their masters'
//!   locations or, for a layer that is both alternate and intermediate, at its own coordinates.
//!   The reverse partners (`]…[` written as a range that excludes the interval) are separate sets.
//! - A master that has no layer in the glyph is listed in [`GlyphSources::missing_masters`];
//!   interpolation then uses a sub-model without it.
//! - A master layer may carry a range itself (`alternateLayer.onMasterLayer`). When a layer under
//!   the same master is an alternate layer with every bound empty (the `[]` partner), the partner
//!   is the master's main source and the master layer joins the alternate set of its range.
//!   Without a partner the master layer is both. A `[]` layer whose master layer has no range is
//!   no source at all.
//! - Range sets are told apart by the box they cover, an empty bound standing for the axis end,
//!   as both oracles group alternate layers.
//! - An alternate set that has no layer at some master's location takes that master's main
//!   source there, as both oracles do; the handbook asks for one alternate layer under every
//!   master (`alternateLayer.onePerMaster`) and does not say what happens without one.
//! - Colour, SVG, bitmap and smart-glyph pole layers are no sources; backup layers are ignored.
//! - A layer marked inactive ([`is_inactive`]) is no source: it is listed in
//!   [`GlyphSources::inactive`], and interpolation uses a sub-model without it. The mark has no
//!   effect on the origin master's main layer, which every interpolation starts from.
//! - Two sources at one location, a location that is not finite, and more than
//!   [`crate::MAX_SOURCES`] sources are refused.

use std::collections::HashSet;
use std::sync::Arc;

use fontcraft_model::fc_keys::SOURCE_INACTIVE;
use fontcraft_model::plist::{Number, Value};
use fontcraft_model::{Font, Glyph, GlyphTable, Layer};

use crate::designspace::{Designspace, Location, MasterInfo};
use crate::instance::{InterpOptions, interpolate_layer};
use crate::rules::RangeSet;
use crate::{InterpError, MAX_AXES, MAX_COORDINATE, MAX_RANGE_SETS, MAX_SOURCES};

/// One source of a glyph: a layer at a location.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    /// The layer's id.
    pub layer_id: String,
    /// The master the layer belongs to.
    pub master: String,
    /// Internal coordinates, one per designspace axis.
    pub location: Location,
    /// The layer stands at its master's location: a master layer, the `[]` partner standing in
    /// for one, or an alternate layer without coordinates of its own. `false` for an intermediate
    /// layer.
    pub is_master: bool,
}

/// The alternate layers of one range set.
#[derive(Clone, Debug, PartialEq)]
pub struct AlternateSet {
    pub ranges: RangeSet,
    /// The set's layers in glyph order, then the main sources taken for masters the set has no
    /// layer for, in master order.
    pub sources: Vec<Source>,
}

/// Everything one glyph interpolates from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphSources {
    pub glyph: String,
    /// Master layers and intermediate layers, master layers first in master order.
    pub main: Vec<Source>,
    /// The alternate sets, in the order their first layer appears in the glyph.
    pub alternates: Vec<AlternateSet>,
    /// Ids of the designspace's masters the glyph has no layer for.
    pub missing_masters: Vec<String>,
    /// The sources an inactive mark leaves out, main and alternate ones, in glyph order.
    pub inactive: Vec<Source>,
}

/// Whether `layer` is marked as left out of interpolation.
pub fn is_inactive(layer: &Layer) -> bool {
    match layer.info.user_data.get(SOURCE_INACTIVE) {
        Some(Value::Number(n)) => n.value().as_f64() != 0.0,
        Some(Value::Bool(on)) => *on,
        Some(Value::String(s)) => s.as_str() == "1",
        _ => false,
    }
}

/// Marks `layer` as left out of interpolation, or takes the mark off.
pub fn set_inactive(layer: &mut Layer, inactive: bool) {
    let data = &mut layer.info.user_data;
    if inactive {
        data.0.insert(SOURCE_INACTIVE, Value::Number(Number::int(1)));
    } else {
        data.0.entries.retain(|(k, _)| k.as_str() != SOURCE_INACTIVE);
    }
}

/// Whether any layer of `font` is marked as left out of interpolation.
pub fn has_inactive(font: &Font) -> bool {
    font.glyphs.iter().any(|g| g.layers.iter().any(is_inactive))
}

/// `font` with the drawing of every inactive master layer replaced by the interpolation of the
/// glyph's other sources there; `None` when nothing is inactive. The layers stay marked, so they
/// are still no sources of their own glyphs; what uses a glyph at a master (a decomposed
/// component, the mark positions) then uses the interpolated drawing. A layer whose glyph cannot
/// be interpolated keeps its drawing.
pub fn with_inactive_interpolated(font: &Font, designspace: &Designspace, options: &InterpOptions) -> Option<Font> {
    if !has_inactive(font) {
        return None;
    }
    let mut changed = false;
    let glyphs = font
        .glyphs
        .iter()
        .map(|g| {
            if !g.layers.iter().any(is_inactive) {
                return Arc::clone(g);
            }
            let Ok(sources) = GlyphSources::of(font, designspace, &g.name) else { return Arc::clone(g) };
            let mut out = Glyph::clone(g);
            for s in sources.inactive.iter().filter(|s| s.is_master) {
                let Some(layer) = out.layers.iter_mut().find(|l| l.id == s.layer_id && l.is_master_layer()) else { continue };
                let Ok(new) = interpolate_layer(font, &sources, &s.location, options) else { continue };
                layer.width = new.width;
                layer.shapes = new.shapes;
                layer.anchors = new.anchors;
                layer.info.vert_width = new.info.vert_width;
                layer.info.vert_origin = new.info.vert_origin;
                changed = true;
            }
            Arc::new(out)
        })
        .collect();
    let mut out = font.clone();
    out.glyphs = GlyphTable::from_glyphs(glyphs);
    changed.then_some(out)
}

impl GlyphSources {
    /// The sources of `glyph` in `designspace`.
    pub fn of(font: &Font, designspace: &Designspace, glyph: &str) -> Result<GlyphSources, InterpError> {
        let g = font.glyph(glyph).ok_or_else(|| InterpError::UnknownGlyph(glyph.to_string()))?;
        let mut out = GlyphSources { glyph: glyph.to_string(), ..GlyphSources::default() };
        let origin = designspace.origin().map(|m| m.id.as_str());

        for m in designspace.masters() {
            let Some(layer) = g.layer(&m.id) else {
                out.missing_masters.push(m.id.clone());
                continue;
            };
            let main = if has_range(layer) { partner(g, &m.id).unwrap_or(layer) } else { layer };
            let source = Source { layer_id: main.id.clone(), master: m.id.clone(), location: m.location.clone(), is_master: true };
            let list = if is_inactive(main) && origin != Some(m.id.as_str()) { &mut out.inactive } else { &mut out.main };
            push_limited(list, source)?;
        }
        for layer in &g.layers {
            let a = &layer.info.attrs;
            if layer.is_master_layer() || !is_outline_layer(layer) || !a.axis_rules.is_empty() || a.coordinates.is_empty() {
                continue;
            }
            let Some(m) = designspace.master(&layer.master) else { continue };
            let list = if is_inactive(layer) { &mut out.inactive } else { &mut out.main };
            push_limited(
                list,
                Source { layer_id: layer.id.clone(), master: m.id.clone(), location: own_location(designspace, m, layer)?, is_master: false },
            )?;
        }
        check_distinct(glyph, &out.main)?;

        let mut boxes: Vec<Vec<(u64, u64)>> = Vec::new();
        for layer in &g.layers {
            if !has_range(layer) || (!layer.is_master_layer() && !is_outline_layer(layer)) {
                continue;
            }
            let Some(m) = designspace.master(&layer.master) else { continue };
            let ranges = RangeSet::from_rules(designspace, &layer.info.attrs.axis_rules)?;
            let key = range_box(designspace, &ranges);
            let (location, is_master) =
                if layer.info.attrs.coordinates.is_empty() { (m.location.clone(), true) } else { (own_location(designspace, m, layer)?, false) };
            let source = Source { layer_id: layer.id.clone(), master: m.id.clone(), location, is_master };
            // The origin's main layer stays a source in spite of its mark.
            if is_inactive(layer) && !out.main.iter().any(|s| s.layer_id == source.layer_id) {
                if !out.inactive.iter().any(|s| s.layer_id == source.layer_id) {
                    push_limited(&mut out.inactive, source)?;
                }
                continue;
            }
            let set = match boxes.iter().position(|b| *b == key) {
                Some(i) => out.alternates.get_mut(i),
                None => {
                    if boxes.len() >= MAX_RANGE_SETS {
                        return Err(InterpError::LimitExceeded { what: "alternate range sets in one glyph", limit: MAX_RANGE_SETS });
                    }
                    boxes.push(key);
                    out.alternates.push(AlternateSet { ranges, sources: Vec::new() });
                    out.alternates.last_mut()
                }
            };
            if let Some(set) = set {
                push_limited(&mut set.sources, source)?;
            }
        }
        for set in &mut out.alternates {
            for s in out.main.iter().filter(|s| s.is_master) {
                if !set.sources.iter().any(|x| x.location == s.location) {
                    push_limited(&mut set.sources, s.clone())?;
                }
            }
            check_distinct(glyph, &set.sources)?;
        }
        Ok(out)
    }

    /// The sources that apply at `location`: the main sources with each alternate set that
    /// contains the location put in place of the master layers it replaces.
    ///
    /// Bounds are inclusive. When several sets contain the location, every one applies, the sets
    /// with more bounds last, so the most specific set wins for a master both replace.
    pub fn at(&self, location: &Location) -> Vec<&Source> {
        let mut out: Vec<&Source> = self.main.iter().collect();
        let mut sets: Vec<&AlternateSet> = self.alternates.iter().filter(|a| a.ranges.contains(location)).collect();
        sets.sort_by_key(|a| a.ranges.axes.iter().map(|r| usize::from(r.min.is_some()) + usize::from(r.max.is_some())).sum::<usize>());
        for set in sets {
            for slot in out.iter_mut().filter(|s| s.is_master) {
                if let Some(alt) = set.sources.iter().find(|a| a.is_master && a.master == slot.master) {
                    *slot = alt;
                }
            }
        }
        out
    }

    /// Whether the glyph has intermediate layers (a source that is not at a master's location).
    pub fn has_intermediates(&self) -> bool {
        self.main.iter().any(|s| !s.is_master)
    }
}

/// A layer that draws the glyph's outline, as opposed to a colour, SVG, bitmap or smart-glyph
/// pole layer.
fn is_outline_layer(layer: &Layer) -> bool {
    let a = &layer.info.attrs;
    a.color_palette.is_none() && !a.full_color && !a.svg && a.sbix_size.is_none() && a.smart_poles.is_empty()
}

/// An alternate layer with at least one bound.
fn has_range(layer: &Layer) -> bool {
    layer.info.attrs.axis_rules.iter().any(|r| r.min.is_some() || r.max.is_some())
}

/// The `[]` partner under `master`: an alternate layer with every bound empty.
fn partner<'a>(g: &'a Glyph, master: &str) -> Option<&'a Layer> {
    g.layers.iter().find(|l| {
        let a = &l.info.attrs;
        l.master == master && !l.is_master_layer() && is_outline_layer(l) && a.coordinates.is_empty() && !a.axis_rules.is_empty() && !has_range(l)
    })
}

/// An intermediate layer's location: its coordinates matched to the designspace's axes, a
/// missing one taking its master's value.
fn own_location(designspace: &Designspace, master: &MasterInfo, layer: &Layer) -> Result<Location, InterpError> {
    let coords = &layer.info.attrs.coordinates;
    if coords.len() > MAX_AXES {
        return Err(InterpError::LimitExceeded { what: "coordinates on one layer", limit: MAX_AXES });
    }
    let mut out = Vec::with_capacity(designspace.axes().len());
    for (i, axis) in designspace.axes().iter().enumerate() {
        let inherited = master.location.0.get(i).copied().unwrap_or(axis.internal.default);
        let v = coords.get(axis.font_axis).copied().flatten().unwrap_or(inherited);
        if !v.is_finite() || v.abs() > MAX_COORDINATE {
            return Err(InterpError::BadNumber { what: format!("the {} coordinate of layer `{}`", axis.tag, layer.id) });
        }
        out.push(v);
    }
    Ok(Location(out))
}

fn push_limited(sources: &mut Vec<Source>, source: Source) -> Result<(), InterpError> {
    if sources.len() >= MAX_SOURCES {
        return Err(InterpError::LimitExceeded { what: "sources in one glyph", limit: MAX_SOURCES });
    }
    sources.push(source);
    Ok(())
}

/// The bits of a coordinate, with −0 and 0 the same.
fn bits(v: f64) -> u64 {
    if v == 0.0 { 0 } else { v.to_bits() }
}

/// The box a range set covers, an empty bound standing for the axis end.
fn range_box(designspace: &Designspace, ranges: &RangeSet) -> Vec<(u64, u64)> {
    designspace
        .axes()
        .iter()
        .zip(&ranges.axes)
        .map(|(axis, r)| (bits(r.min.unwrap_or(axis.internal.minimum)), bits(r.max.unwrap_or(axis.internal.maximum))))
        .collect()
}

fn check_distinct(glyph: &str, sources: &[Source]) -> Result<(), InterpError> {
    let mut seen: HashSet<Vec<u64>> = HashSet::with_capacity(sources.len());
    for (i, s) in sources.iter().enumerate() {
        if !seen.insert(s.location.0.iter().map(|&v| bits(v)).collect()) {
            let other = sources.iter().take(i).find(|o| o.location == s.location).map_or("another layer", |o| o.layer_id.as_str());
            return Err(InterpError::BadMasterSetup(format!("layers `{other}` and `{}` of `{glyph}` are at the same location", s.layer_id)));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use fontcraft_model::{Axis, AxisRule, Font, Glyph, GlyphTable, Instance, Layer, Master, Path, Shape};

    use super::*;

    pub(crate) fn master(id: &str, name: &str, at: &[f64]) -> Master {
        Master { id: id.into(), name: name.into(), location: at.to_vec(), ..Master::default() }
    }

    fn axis(name: &str, tag: &str) -> Axis {
        Axis { name: name.into(), tag: tag.into(), ..Axis::default() }
    }

    /// Masters at `masters` (id, name, location) on the axes `axes` (name, tag), one static
    /// instance at `instance`, and `glyphs`.
    pub(crate) fn font_with(axes: &[(&str, &str)], masters: Vec<Master>, instances: Vec<Instance>, glyphs: Vec<Glyph>) -> Font {
        let mut f = Font::new("Test");
        f.axes = Arc::new(axes.iter().map(|(n, t)| axis(n, t)).collect());
        f.masters = Arc::new(masters);
        f.instances = Arc::new(instances);
        f.glyphs = GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect());
        f
    }

    /// Thin (wght 100) and Black (900) with an instance at 400.
    pub(crate) fn two_masters(glyphs: Vec<Glyph>) -> Font {
        let regular = Instance { name: "Regular".into(), location: vec![400.0], ..Instance::default() };
        font_with(&[("Weight", "wght")], vec![master("thin", "Thin", &[100.0]), master("black", "Black", &[900.0])], vec![regular], glyphs)
    }

    pub(crate) fn layer(master: &str, id: &str, shapes: Vec<Shape>) -> Layer {
        let mut l = Layer::new(master, 500.0);
        l.id = id.into();
        l.shapes = shapes;
        l
    }

    pub(crate) fn rect(w: f64) -> Shape {
        Shape::Path(Path::rect(0.0, 0.0, w, 700.0))
    }

    pub(crate) fn glyph(name: &str, layers: Vec<Layer>) -> Glyph {
        let mut g = Glyph::new(name);
        g.layers = layers;
        g
    }

    fn ranged(mut l: Layer, rules: &[(Option<f64>, Option<f64>)]) -> Layer {
        l.info.attrs.axis_rules = rules.iter().map(|&(min, max)| AxisRule { min, max }).collect();
        l
    }

    fn at(mut l: Layer, coordinates: &[Option<f64>]) -> Layer {
        l.info.attrs.coordinates = coordinates.to_vec();
        l
    }

    fn ids(sources: &[Source]) -> Vec<&str> {
        sources.iter().map(|s| s.layer_id.as_str()).collect()
    }

    fn sources(font: &Font, name: &str) -> Result<GlyphSources, InterpError> {
        GlyphSources::of(font, &Designspace::new(font).unwrap(), name)
    }

    #[test]
    fn intermediate_layer_inherits_empty_coordinates_from_its_master() {
        let e = glyph(
            "e",
            vec![
                layer("thin", "thin", vec![rect(100.0)]),
                layer("black", "black", vec![rect(300.0)]),
                layer("narrow", "narrow", vec![rect(80.0)]),
                at(layer("black", "brace", vec![rect(200.0)]), &[Some(400.0), None]),
                layer("thin", "backup", vec![rect(10.0)]),
            ],
        );
        let masters =
            vec![master("thin", "Thin", &[100.0, 100.0]), master("black", "Black", &[900.0, 100.0]), master("narrow", "Narrow", &[100.0, 50.0])];
        let font = font_with(&[("Weight", "wght"), ("Width", "wdth")], masters, Vec::new(), vec![e]);
        let s = sources(&font, "e").unwrap();
        assert_eq!(ids(&s.main), ["thin", "black", "narrow", "brace"], "masters first, the backup layer ignored");
        let brace = s.main.last().unwrap();
        assert_eq!((brace.location.clone(), brace.is_master, brace.master.as_str()), (Location(vec![400.0, 100.0]), false, "black"));
        assert!(s.has_intermediates() && s.alternates.is_empty() && s.missing_masters.is_empty());
    }

    #[test]
    fn alternate_layers_group_by_range_and_take_master_layers_where_missing() {
        let dollar = glyph(
            "dollar",
            vec![
                layer("thin", "thin", vec![rect(100.0)]),
                layer("black", "black", vec![rect(300.0)]),
                ranged(layer("thin", "alt-thin", vec![rect(110.0)]), &[(Some(600.0), None)]),
                // The axis end written out is the same range as an empty bound.
                ranged(layer("black", "alt-black", vec![rect(310.0)]), &[(Some(600.0), Some(900.0))]),
                ranged(layer("thin", "low-thin", vec![rect(90.0)]), &[(None, Some(300.0))]),
            ],
        );
        let font = two_masters(vec![dollar]);
        let s = sources(&font, "dollar").unwrap();
        assert_eq!(ids(&s.main), ["thin", "black"]);
        assert_eq!(s.alternates.len(), 2);
        let first = s.alternates.first().unwrap();
        assert_eq!(ids(&first.sources), ["alt-thin", "alt-black"]);
        assert_eq!(first.ranges.axes, [AxisRule { min: Some(600.0), max: None }], "the first layer's ranges");
        let second = s.alternates.get(1).unwrap();
        assert_eq!(ids(&second.sources), ["low-thin", "black"], "Black has no layer for this range: its master layer stands in");
        assert!(second.sources.iter().all(|x| x.is_master));
    }

    #[test]
    fn master_layer_with_a_range_trades_places_with_its_empty_partner() {
        let g = glyph(
            "g",
            vec![
                ranged(layer("thin", "thin", vec![rect(110.0)]), &[(Some(600.0), None)]),
                ranged(layer("thin", "partner", vec![rect(100.0)]), &[(None, None)]),
                layer("black", "black", vec![rect(300.0)]),
                ranged(layer("black", "alt-black", vec![rect(310.0)]), &[(Some(600.0), None)]),
                // A `[]` layer under a master whose layer has no range is no source.
                ranged(layer("black", "stray", vec![rect(1.0)]), &[(None, None)]),
            ],
        );
        let font = two_masters(vec![g]);
        let s = sources(&font, "g").unwrap();
        assert_eq!(ids(&s.main), ["partner", "black"]);
        assert!(s.main.iter().all(|x| x.is_master));
        assert_eq!(s.alternates.len(), 1);
        assert_eq!(ids(&s.alternates.first().unwrap().sources), ["thin", "alt-black"]);
    }

    #[test]
    fn layer_both_intermediate_and_alternate_serves_the_alternate_shape_at_its_coordinates() {
        let g = glyph(
            "g",
            vec![
                layer("thin", "thin", vec![rect(100.0)]),
                layer("black", "black", vec![rect(300.0)]),
                ranged(at(layer("thin", "both", vec![rect(210.0)]), &[Some(400.0)]), &[(Some(300.0), None)]),
            ],
        );
        let font = two_masters(vec![g]);
        let s = sources(&font, "g").unwrap();
        assert_eq!(ids(&s.main), ["thin", "black"], "not a main source");
        let set = s.alternates.first().unwrap();
        assert_eq!(ids(&set.sources), ["both", "thin", "black"]);
        let both = set.sources.first().unwrap();
        assert_eq!((both.location.clone(), both.is_master), (Location(vec![400.0]), false));
    }

    #[test]
    fn sources_refuse_two_layers_at_one_location_and_list_missing_masters() {
        let twice =
            glyph("a", vec![layer("thin", "thin", vec![]), layer("black", "black", vec![]), at(layer("thin", "brace", vec![]), &[Some(100.0)])]);
        let half = glyph("b", vec![layer("thin", "thin", vec![rect(100.0)])]);
        let nan = glyph("c", vec![layer("thin", "thin", vec![]), at(layer("thin", "brace", vec![]), &[Some(f64::NAN)])]);
        let font = two_masters(vec![twice, half, nan]);
        assert!(matches!(sources(&font, "a"), Err(InterpError::BadMasterSetup(m)) if m.contains("brace")));
        assert_eq!(sources(&font, "b").unwrap().missing_masters, ["black"]);
        assert!(matches!(sources(&font, "c"), Err(InterpError::BadNumber { .. })));
        assert!(matches!(sources(&font, "zz"), Err(InterpError::UnknownGlyph(_))));
    }

    #[test]
    fn inactive_sources_leave_interpolation_but_the_origin_stays() {
        let off = |mut l: Layer| {
            set_inactive(&mut l, true);
            l
        };
        let g = glyph(
            "g",
            vec![
                off(layer("thin", "thin", vec![rect(100.0)])),
                layer("black", "black", vec![rect(900.0)]),
                off(at(layer("thin", "brace", vec![rect(700.0)]), &[Some(500.0)])),
                off(ranged(layer("black", "alt", vec![rect(1.0)]), &[(Some(600.0), None)])),
            ],
        );
        let mut font = two_masters(vec![g]);
        let s = sources(&font, "g").unwrap();
        assert_eq!(ids(&s.main), ["thin", "black"], "the origin's mark has no effect");
        assert_eq!(ids(&s.inactive), ["brace", "alt"]);
        assert!(s.alternates.is_empty(), "a set without an active layer is no set");
        let width = |font: &Font| {
            let l = interpolate_layer(font, &sources(font, "g").unwrap(), &Location(vec![500.0]), &InterpOptions::default()).unwrap();
            match l.shapes.first() {
                Some(Shape::Path(p)) => p.nodes.iter().map(|n| n.x).fold(0.0, f64::max),
                _ => 0.0,
            }
        };
        assert_eq!(width(&font), 500.0, "halfway between the masters, not the intermediate layer's 700");
        let glyphs: Vec<Glyph> = font
            .glyphs
            .iter()
            .map(|g| {
                let mut g = Glyph::clone(g);
                g.layers.iter_mut().for_each(|l| set_inactive(l, false));
                g
            })
            .collect();
        font.glyphs = GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect());
        assert!(!has_inactive(&font));
        assert_eq!(width(&font), 700.0);
    }

    #[test]
    fn inactive_master_layers_draw_the_interpolation() {
        let mut black = layer("black", "black", vec![rect(900.0)]);
        set_inactive(&mut black, true);
        let g = glyph("g", vec![layer("thin", "thin", vec![rect(100.0)]), at(layer("thin", "brace", vec![rect(200.0)]), &[Some(500.0)]), black]);
        let font = two_masters(vec![g]);
        let ds = Designspace::new(&font).unwrap();
        let filled = with_inactive_interpolated(&font, &ds, &InterpOptions::default()).unwrap();
        let g = filled.glyph("g").unwrap();
        let black = g.layers.iter().find(|l| l.id == "black").unwrap();
        assert!(is_inactive(black), "still no source");
        assert_ne!(black.shapes, vec![rect(900.0)], "the inactive drawing is gone");
        assert!(with_inactive_interpolated(&two_masters(Vec::new()), &ds, &InterpOptions::default()).is_none());
    }

    #[test]
    fn alternate_ranges_include_both_bounds() {
        let font = two_masters(Vec::new());
        let ds = Designspace::new(&font).unwrap();
        let set = RangeSet::from_rules(&ds, &[AxisRule { min: Some(600.0), max: Some(800.0) }]).unwrap();
        for (v, inside) in [(600.0, true), (700.0, true), (800.0, true), (599.9, false), (800.1, false), (f64::NAN, false)] {
            assert_eq!(set.contains(&Location(vec![v])), inside, "{v}");
        }
        let open = RangeSet::from_rules(&ds, &[AxisRule { min: Some(600.0), max: None }]).unwrap();
        assert!(open.contains(&Location(vec![10_000.0])), "an empty bound is open, also beyond the axis");
        assert!(!open.contains(&Location(vec![600.0, 1.0])), "a location of the wrong length");
        let bounded_beyond = [AxisRule::default(), AxisRule { min: Some(1.0), max: None }];
        assert!(RangeSet::from_rules(&ds, &bounded_beyond).is_err(), "a bound on an axis the font lacks");
        assert!(RangeSet::from_rules(&ds, &[AxisRule::default(), AxisRule::default()]).is_ok(), "open rules beyond the axes are harmless");
        assert!(RangeSet::from_rules(&ds, &[AxisRule { min: Some(f64::INFINITY), max: None }]).is_err());
        let reversed = RangeSet::from_rules(&ds, &[AxisRule { min: Some(800.0), max: Some(600.0) }]);
        assert_eq!(reversed, Err(InterpError::ReversedRange { axis: 1, min: 800.0, max: 600.0 }), "a range whose ends are swapped");
        assert!(RangeSet::from_rules(&ds, &vec![AxisRule::default(); MAX_AXES + 1]).is_err());
    }
}
