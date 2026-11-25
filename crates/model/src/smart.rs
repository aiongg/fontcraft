//! Smart components drawn as outlines.
//!
//! Behaviour ported from glyphsLib `builder/smart_components.py` and `builder/components.py`
//! (Apache-2.0; see `NOTICE`). A smart glyph has properties (`GlyphInfo::parts`) and, per master,
//! pole layers that each draw one end of every property's range. A component of it with values
//! for those properties is drawn by interpolating the pole layers' nodes with a variation model
//! ([`fontcraft_geom::variation`]): the first pole layer of the component's master (its master
//! layer when that is a pole layer) is the default location, the others sit at −1 or 1 on each property where their end differs from the
//! default's. Smart components inside pole layers are drawn first. The component's transform is
//! applied, and contours are reversed when it flips them.
//!
//! One difference from glyphsLib: an ordinary component inside a pole layer keeps a transform
//! combined with the smart component's, where glyphsLib keeps the inner transform alone.
//!
//! [`crate::Font::outline`] draws a smart component through [`component_shapes`], so the editor,
//! measurements and metrics see the interpolated outline; [`draw_smart_components`] rewrites a
//! whole font the same way for export. [`component_anchors`] places a smart component's anchors
//! by the same interpolation, for anchor propagation.

use std::collections::HashMap;

use fontcraft_geom::variation::{VariationModel, normalize_value};

use crate::{Anchor, Component, Font, Glyph, Layer, MAX_COMPONENT_DEPTH, MAX_OUTLINE_ELEMENTS, Shape, Transform};

/// Most pole layers one smart glyph may have for one master.
pub const MAX_SMART_POLE_LAYERS: usize = 64;
/// Most smart components drawn, nested ones included, for one glyph layer. Nesting can multiply
/// the work (a pole layer with two smart components whose pole layers have two…).
pub const MAX_SMART_DRAWS_PER_LAYER: usize = 10_000;
/// Most smart components drawn, nested ones included, for one font.
pub const MAX_SMART_DRAWS: usize = 1_000_000;
/// Most anchors on one pole layer that [`component_anchors`] interpolates.
pub const MAX_SMART_ANCHORS: usize = 4096;

/// What [`draw_smart_components`] did.
#[derive(Debug, Default)]
pub struct SmartOutlines {
    /// Smart components of glyph layers drawn as outlines (not counting nested ones).
    pub drawn: usize,
    /// Smart components left as components, each with the reason.
    pub kept: Vec<String>,
}

/// `font` with every smart component of its glyph layers (not their backgrounds) drawn as
/// outlines, as glyphsLib draws them when it writes a UFO. A smart glyph with one pole layer for
/// the component's master stays a component, as in glyphsLib; one that cannot be interpolated
/// stays a component and is listed in [`SmartOutlines::kept`].
pub fn draw_smart_components(font: &Font) -> (Font, SmartOutlines) {
    let mut report = SmartOutlines::default();
    let mut changes: Vec<(usize, usize, Vec<Shape>)> = Vec::new();
    let mut draws_left = MAX_SMART_DRAWS;
    for (gi, glyph) in font.glyphs.iter().enumerate() {
        for (li, layer) in glyph.layers.iter().enumerate() {
            if !layer.shapes.iter().any(|s| matches!(s, Shape::Component(c) if is_smart(font, c))) {
                continue;
            }
            let draws = draws_left.min(MAX_SMART_DRAWS_PER_LAYER);
            let mut ctx = Ctx { font, chain: Vec::new(), nodes_left: MAX_OUTLINE_ELEMENTS, draws_left: draws, notes: Vec::new() };
            let (shapes, drawn) = ctx.draw_layer(layer);
            draws_left -= draws - ctx.draws_left;
            for n in ctx.notes {
                report.kept.push(format!("glyph `{}` layer `{}`: {n}", glyph.name, layer_label(layer)));
            }
            if drawn > 0 {
                report.drawn += drawn;
                changes.push((gi, li, shapes));
            }
        }
    }
    let mut out = font.clone();
    for (gi, li, shapes) in changes {
        if let Some(mut g) = out.glyphs.get_mut_at(gi)
            && let Some(l) = g.layers.get_mut(li)
        {
            l.shapes = shapes;
        }
    }
    (out, report)
}

/// The shapes the smart component `c` draws in master `master` (or in the master it is pinned
/// to), its transform applied: paths,
/// and the plain components of its pole layers with the transform combined. `Ok(None)` when its
/// glyph is not a smart glyph or has one pole layer for the master (it is drawn as a plain
/// component). Each smart component drawn, nested ones included, takes one from `draws_left`.
pub fn component_shapes(font: &Font, c: &Component, master: &str, draws_left: &mut usize) -> Result<Option<Vec<Shape>>, String> {
    let Some(base) = font.glyphs.get(&c.glyph).filter(|g| !g.info.parts.is_empty()) else { return Ok(None) };
    let mut ctx = Ctx { font, chain: Vec::new(), nodes_left: MAX_OUTLINE_ELEMENTS, draws_left: *draws_left, notes: Vec::new() };
    let result = ctx.instantiate(base, c, master);
    *draws_left = ctx.draws_left;
    result
}

fn layer_label(l: &Layer) -> &str {
    l.info.name.as_deref().unwrap_or(&l.id)
}

fn is_smart(font: &Font, c: &Component) -> bool {
    font.glyphs.get(&c.glyph).is_some_and(|g| !g.info.parts.is_empty())
}

/// The drawing of one glyph layer's smart components.
struct Ctx<'a> {
    font: &'a Font,
    /// The smart glyphs being drawn, outermost first.
    chain: Vec<&'a str>,
    nodes_left: usize,
    draws_left: usize,
    notes: Vec<String>,
}

impl<'a> Ctx<'a> {
    /// The shapes of `layer` with its smart components drawn as outlines where they can be, and
    /// how many were drawn.
    ///
    /// The order follows glyphsLib. In a glyph's own layer the drawn outlines come after the
    /// layer's paths, as glyphsLib draws paths before components. In a pole layer they take the
    /// component's place among the shapes, and in a version 2 file a layer's components come
    /// before its paths (glyphsLib reads the `components` key before `paths`).
    fn draw_layer(&mut self, layer: &'a Layer) -> (Vec<Shape>, usize) {
        let nested = !self.chain.is_empty();
        let mut source: Vec<&Shape> = layer.shapes.iter().collect();
        if nested && self.font.source.glyphs_version == Some(2) {
            source.sort_by_key(|s| !matches!(s, Shape::Component(_)));
        }
        let mut out = Vec::with_capacity(layer.shapes.len());
        let mut tail = Vec::new();
        let mut drawn = 0;
        for shape in source {
            let Shape::Component(c) = shape else {
                out.push(shape.clone());
                continue;
            };
            let Some(base) = self.font.glyphs.get(&c.glyph).filter(|g| !g.info.parts.is_empty()) else {
                out.push(shape.clone());
                continue;
            };
            match self.instantiate(base, c, &layer.master) {
                Ok(Some(shapes)) => {
                    drawn += 1;
                    if nested { out.extend(shapes) } else { tail.extend(shapes) }
                }
                Ok(None) => out.push(shape.clone()),
                Err(reason) => {
                    self.notes.push(format!("the smart component `{}` stays a component: {reason}", c.glyph));
                    out.push(shape.clone());
                }
            }
        }
        out.extend(tail);
        (out, drawn)
    }

    /// The outlines of the smart component `c` of `base` in master `master`; `None` when the
    /// glyph has one pole layer there (it stays a plain component).
    fn instantiate(&mut self, base: &'a Glyph, c: &Component, master: &str) -> Result<Option<Vec<Shape>>, String> {
        if self.chain.contains(&base.name.as_str()) {
            return Err("the smart glyph uses itself".into());
        }
        if self.chain.len() >= MAX_COMPONENT_DEPTH {
            return Err("smart components nest too deeply".into());
        }
        self.draws_left = self.draws_left.checked_sub(1).ok_or_else(|| {
            format!("too many smart components to draw (at most {MAX_SMART_DRAWS_PER_LAYER} for one glyph layer and {MAX_SMART_DRAWS} for the font)")
        })?;
        // A pinned component is drawn from its own master, and so are the components in it.
        let read = c.pinned_layer(base).map_or(master, |l| l.master.as_str());
        let Some(Interpolation { poles, model, at }) = interpolation(base, c, read)? else { return Ok(None) };

        // Nested smart components first; then the nodes of every path, in order.
        self.chain.push(&base.name);
        let drawn_poles: Vec<Vec<Shape>> = poles.iter().map(|l| self.draw_layer(l).0).collect();
        self.chain.pop();
        let coordinates: Vec<Vec<f64>> = drawn_poles
            .iter()
            .map(|shapes| {
                shapes
                    .iter()
                    .filter_map(|s| if let Shape::Path(p) = s { Some(p) } else { None })
                    .flat_map(|p| p.nodes.iter().flat_map(|n| [n.x, n.y]))
                    .collect()
            })
            .collect();
        let count = coordinates.first().map_or(0, Vec::len);
        self.nodes_left = self.nodes_left.checked_sub(count / 2).ok_or_else(too_large)?;
        let values = model.interpolate(&at, &coordinates).map_err(|_| "its pole layers do not have the same number of nodes".to_string())?;

        let affine = c.affine();
        let flip = affine.determinant() < 0.0;
        let mut shapes = drawn_poles.into_iter().next().unwrap_or_default();
        let mut next = values.as_chunks::<2>().0.iter();
        for shape in &mut shapes {
            match shape {
                Shape::Path(p) => {
                    for n in &mut p.nodes {
                        if let Some(&[x, y]) = next.next() {
                            let moved = affine * fontcraft_geom::Point::new(x, y);
                            (n.x, n.y) = (moved.x, moved.y);
                        }
                    }
                    if !p.nodes.iter().all(|n| crate::in_range(n.x) && crate::in_range(n.y)) {
                        return Err("the interpolated outline lies beyond the coordinate limit".into());
                    }
                    if flip {
                        p.reverse();
                    }
                }
                Shape::Component(inner) => {
                    inner.transform = Transform::Matrix((affine * inner.affine()).as_coeffs());
                    if read != master && inner.info.master_id.is_none() {
                        inner.info.master_id = Some(read.to_string());
                    }
                }
                _ => {}
            }
        }
        Ok(Some(shapes))
    }
}

/// The pole layers of the smart glyph `base` for `master` (every master when it is empty), in
/// the glyph's layer order.
pub fn pole_layers<'a, 'm>(base: &'a Glyph, master: &'m str) -> impl Iterator<Item = &'a Layer> + use<'a, 'm> {
    base.layers.iter().filter(move |l| !l.info.attrs.smart_poles.is_empty() && (master.is_empty() || l.master == master))
}

/// An upper bound on the steps [`interpolate_anchors`] takes for the component `c` of `base` in
/// `master` when no pole layer has more than `anchors` anchors, for callers that budget their
/// work before calling it: placing each pole layer on the properties, building the variation
/// model, reading the component's values, and reading and weighing the pole layers' anchors.
pub fn anchor_interpolation_cost(base: &Glyph, c: &Component, master: &str, anchors: usize) -> usize {
    let (poles, ends) = pole_layers(base, master).fold((0usize, 0usize), |(n, e), l| (n.saturating_add(1), e.max(l.info.attrs.smart_poles.len())));
    let parts = base.info.parts.len();
    // Each pole layer looks up every property among its own ends and the default layer's.
    let locations = poles.saturating_mul(parts).saturating_mul(ends.saturating_mul(2));
    let model = poles.saturating_mul(poles).saturating_mul(parts.saturating_add(1));
    let at = parts.saturating_mul(c.info.smart_values.len().saturating_add(ends));
    // Per pole layer and anchor: index it by name, look up the default's, weigh two coordinates.
    let read = poles.saturating_mul(anchors.saturating_mul(4));
    locations.saturating_add(model).saturating_add(at).saturating_add(read)
}

/// A smart component's pole layers for one master, default first, with the variation model
/// over them and the component's location in it.
struct Interpolation<'a> {
    poles: Vec<&'a Layer>,
    model: VariationModel,
    at: Vec<f64>,
}

/// The interpolation of the smart component `c` of `base` in master `master`; `None` when the
/// glyph has one pole layer there.
fn interpolation<'a>(base: &'a Glyph, c: &Component, master: &str) -> Result<Option<Interpolation<'a>>, String> {
    // glyphsLib lists a glyph's master layers before its other layers.
    let mut poles: Vec<&'a Layer> = pole_layers(base, master).collect();
    poles.sort_by_key(|l| !l.is_master_layer());
    let Some(&default) = poles.first() else {
        return Err("the smart glyph has no pole layers for this master".into());
    };
    if poles.len() == 1 {
        return Ok(None);
    }
    if poles.len() > MAX_SMART_POLE_LAYERS {
        return Err(format!("the smart glyph has more than {MAX_SMART_POLE_LAYERS} pole layers for one master"));
    }
    let parts = &base.info.parts;
    let pole = |l: &Layer, name: &str| l.info.attrs.smart_poles.iter().find(|p| p.property == name).map(|p| p.top);
    let default_pole = |name: &str| pole(default, name).ok_or_else(|| format!("the first pole layer does not say which end of `{name}` it draws"));
    // Each pole layer's location: 0 where it draws the same end as the default, else −1 or 1.
    let mut locations = Vec::with_capacity(poles.len());
    for l in &poles {
        let mut loc = vec![0.0; parts.len()];
        for (slot, part) in loc.iter_mut().zip(parts.iter()) {
            let Some(top) = pole(l, &part.name) else { continue };
            *slot = match (default_pole(&part.name)?, top) {
                (a, b) if a == b => 0.0,
                (false, true) => 1.0,
                _ => -1.0,
            };
        }
        locations.push(loc);
    }
    let model = VariationModel::new(&locations, true).map_err(|e| format!("its pole layers cannot be interpolated ({e})"))?;
    let mut at = vec![0.0; parts.len()];
    for (slot, part) in at.iter_mut().zip(parts.iter()) {
        let Some(v) = c.info.smart_values.iter().find(|v| v.property == part.name) else { continue };
        let default_value = if default_pole(&part.name)? { part.top_value } else { part.bottom_value };
        *slot = normalize_value(v.value, part.bottom_value, default_value, part.top_value, true)
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("the range of `{}` runs from {} down to {}", part.name, part.bottom_value, part.top_value))?;
    }
    Ok(Some(Interpolation { poles, model, at }))
}

/// The anchors of the smart component `c` in master `master`: the anchors of its glyph's first
/// pole layer (the master layer when that is a pole layer), each placed by interpolating the
/// pole layers' anchors of the same name at the component's values, as its outline is. The
/// positions are in the smart glyph's coordinates; the component's transform is not applied.
/// The pole layers' own anchors are read; [`interpolate_anchors`] reads others.
///
/// `Ok(None)` when the glyph is not a smart glyph or has one pole layer for the master (its
/// layer's anchors apply unchanged). An error when the pole layers cannot be interpolated, when
/// a pole layer lacks an anchor the first one has, or when a pole layer has more than
/// [`MAX_SMART_ANCHORS`] anchors.
pub fn component_anchors(font: &Font, c: &Component, master: &str) -> Result<Option<Vec<Anchor>>, String> {
    let Some(base) = font.glyphs.get(&c.glyph) else { return Ok(None) };
    interpolate_anchors(base, c, master, |l| l.anchors.as_slice())
}

/// [`component_anchors`] for a component `c` of the smart glyph `base`, with each pole layer's
/// anchors given by `anchors` (anchor propagation passes the anchors a pole layer's components
/// give it).
pub fn interpolate_anchors<'a, 's>(
    base: &'a Glyph,
    c: &Component,
    master: &str,
    anchors: impl Fn(&'a Layer) -> &'s [Anchor],
) -> Result<Option<Vec<Anchor>>, String> {
    if base.info.parts.is_empty() {
        return Ok(None);
    }
    let Some(Interpolation { poles, model, at }) = interpolation(base, c, master)? else { return Ok(None) };
    let lists: Vec<&'s [Anchor]> = poles.iter().map(|&l| anchors(l)).collect();
    let Some(&default) = lists.first() else { return Ok(None) };
    if lists.iter().any(|l| l.len() > MAX_SMART_ANCHORS) {
        return Err(format!("a pole layer has more than {MAX_SMART_ANCHORS} anchors"));
    }
    let mut coordinates = Vec::with_capacity(poles.len());
    for (l, list) in poles.iter().zip(&lists) {
        // The first anchor of a name, as in the default layer's own order.
        let mut by_name: HashMap<&str, (f64, f64)> = HashMap::with_capacity(list.len());
        for b in *list {
            by_name.entry(b.name.as_str()).or_insert((b.x, b.y));
        }
        let mut row = Vec::with_capacity(default.len() * 2);
        for a in default {
            let &(x, y) = by_name.get(a.name.as_str()).ok_or_else(|| format!("the pole layer `{}` has no anchor `{}`", layer_label(l), a.name))?;
            row.extend([x, y]);
        }
        coordinates.push(row);
    }
    let values = model.interpolate(&at, &coordinates).map_err(|e| format!("its pole layers' anchors cannot be interpolated ({e})"))?;
    let mut out = Vec::with_capacity(default.len());
    for (a, xy) in default.iter().zip(values.as_chunks::<2>().0) {
        let &[x, y] = xy;
        if !(crate::in_range(x) && crate::in_range(y)) {
            return Err("an interpolated anchor lies beyond the coordinate limit".into());
        }
        let mut moved = a.clone();
        (moved.x, moved.y) = (x, y);
        out.push(moved);
    }
    Ok(Some(out))
}

fn too_large() -> String {
    format!("the outline with its smart components drawn has more than {MAX_OUTLINE_ELEMENTS} nodes")
}

#[cfg(test)]
#[path = "smart_outline_tests.rs"]
mod outline_tests;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::{Node, NodeKind, Path};

    fn kinds(p: &Path) -> Vec<(f64, NodeKind)> {
        p.nodes.iter().map(|n| (n.x, n.kind)).collect()
    }

    /// A smart glyph whose pole layers use it again, twice each: each use is drawn once, and the
    /// uses inside it stay components and are reported, instead of recursing.
    #[test]
    fn a_smart_glyph_that_uses_itself_is_drawn_once() {
        use crate::{
            ComponentInfo, ComponentInfoBox, GlyphInfo, GlyphInfoBox, LayerAttrs, LayerInfo, LayerInfoBox, PartProperty, SmartPole, SmartValue,
        };
        let mut font = Font::new("Loop");
        let mut part = Glyph::new("_loop");
        part.info = GlyphInfoBox::new(GlyphInfo {
            parts: vec![PartProperty {
                name: "W".into(),
                bottom_value: 0.0,
                top_value: 1.0,
                bottom_name: None,
                top_name: None,
                extra: Default::default(),
            }],
            ..Default::default()
        });
        let mut me = Component::new("_loop", 0.0, 0.0);
        me.info = ComponentInfoBox::new(ComponentInfo { smart_values: vec![SmartValue { property: "W".into(), value: 0.5 }], ..Default::default() });
        for (id, top) in [("m01", false), ("top", true)] {
            let mut l = Layer::new("m01", 100.0);
            l.id = id.into();
            l.shapes = vec![Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0)), Shape::Component(me.clone()), Shape::Component(me.clone())];
            l.info = LayerInfoBox::new(LayerInfo {
                attrs: LayerAttrs { smart_poles: vec![SmartPole { property: "W".into(), top }], ..Default::default() },
                ..Default::default()
            });
            part.layers.push(l);
        }
        font.glyphs.push(part);
        let mut user = Glyph::new("user");
        let mut l = Layer::new("m01", 100.0);
        l.shapes.push(Shape::Component(me));
        user.layers.push(l);
        font.glyphs.push(user);
        let (out, report) = draw_smart_components(&font);
        assert!(!report.kept.is_empty());
        assert!(report.kept.iter().all(|k| k.contains("uses itself")), "{:?}", report.kept);
        // Once in `user`, twice in each of the smart glyph's own two layers.
        assert_eq!(report.drawn, 5);
        let shapes = &out.glyphs.get("user").unwrap().layers[0].shapes;
        assert!(matches!(shapes.as_slice(), [Shape::Path(_), Shape::Component(_), Shape::Component(_)]), "{shapes:?}");
    }

    /// Twelve smart glyphs, each pole layer of one using the next twice: four times as many
    /// draws per level. The draws stop at the limit.
    #[test]
    fn nesting_that_multiplies_the_work_stops_at_the_limit() {
        use crate::{GlyphInfo, GlyphInfoBox, LayerAttrs, LayerInfo, LayerInfoBox, PartProperty, SmartPole};
        let mut font = Font::new("Fan");
        for k in 0..12 {
            let mut g = Glyph::new(format!("_p{k}"));
            g.info = GlyphInfoBox::new(GlyphInfo {
                parts: vec![PartProperty {
                    name: "W".into(),
                    bottom_value: 0.0,
                    top_value: 1.0,
                    bottom_name: None,
                    top_name: None,
                    extra: Default::default(),
                }],
                ..Default::default()
            });
            for (id, top) in [("m01", false), ("top", true)] {
                let mut l = Layer::new("m01", 100.0);
                l.id = id.into();
                l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 10.0, if top { 20.0 } else { 10.0 })));
                if k < 11 {
                    let next = Component::new(format!("_p{}", k + 1), 0.0, 0.0);
                    l.shapes.extend([Shape::Component(next.clone()), Shape::Component(next)]);
                }
                l.info = LayerInfoBox::new(LayerInfo {
                    attrs: LayerAttrs { smart_poles: vec![SmartPole { property: "W".into(), top }], ..Default::default() },
                    ..Default::default()
                });
                g.layers.push(l);
            }
            font.glyphs.push(g);
        }
        let mut user = Glyph::new("user");
        let mut l = Layer::new("m01", 100.0);
        l.shapes.push(Shape::Component(Component::new("_p0", 0.0, 0.0)));
        user.layers.push(l);
        font.glyphs.push(user);
        let (_, report) = draw_smart_components(&font);
        assert!(report.kept.iter().any(|k| k.contains("too many smart components")), "{:?}", report.kept.first());
    }

    #[test]
    fn reversing_keeps_the_start_and_moves_the_segment_kinds() {
        use NodeKind::*;
        // A(line) c1 c2 B(curve) C(line), closed: A(curve) C(line) B(line) c2 c1 (glyphsLib's nodes, which end with
        // the start: C(line) B(line) c2 c1 A(curve)).
        let n = |x: f64, k| Node::new(x, 0.0, k);
        let mut p = Path::new(true, vec![n(1.0, Line), n(2.0, OffCurve), n(3.0, OffCurve), n(4.0, Curve), n(5.0, Line)]);
        p.reverse();
        assert_eq!(kinds(&p), [(1.0, Curve), (5.0, Line), (4.0, Line), (3.0, OffCurve), (2.0, OffCurve)]);
        // Its handles may stand first in the list: c1 c2 B(curve) C(line) A(line).
        let mut p = Path::new(true, vec![n(2.0, OffCurve), n(3.0, OffCurve), n(4.0, Curve), n(5.0, Line), n(1.0, Line)]);
        p.reverse();
        assert_eq!(kinds(&p), [(4.0, Line), (3.0, OffCurve), (2.0, OffCurve), (1.0, Curve), (5.0, Line)]);
        // Open: P0 c1 c2 P1(curve) P2(line) gives P2 P1(line) c2 c1 P0(curve).
        let mut p = Path::new(false, vec![n(0.0, Line), n(1.0, OffCurve), n(2.0, OffCurve), n(3.0, Curve), n(4.0, Line)]);
        p.reverse();
        assert_eq!(kinds(&p), [(4.0, Line), (3.0, Line), (2.0, OffCurve), (1.0, OffCurve), (0.0, Curve)]);
    }
}
