//! Components that do not follow the layer they are on: pinned to another master or layer of
//! their base glyph ([`Component::pinned_layer`]) or to a location on the font's axes
//! ([`Component::location`]), drawn as outlines for export.
//!
//! A location is an amount on each axis it names, in design coordinates; on the other axes the
//! component stays at its layer's master. The base glyph is interpolated there from its master
//! layers and intermediate layers, each drawn with its attachments, corners and components. A
//! component with both a location and a pin is drawn at the location, and a smart component by its
//! values.
//!
//! A compiled composite refers to its base glyph at the same place of the design space, so it
//! cannot hold such a component. [`draw_pinned_components`] writes it as the paths it draws,
//! nested components included. The other layers of the glyph get the paths of their components
//! of the same base glyph, so the glyph still interpolates when only some masters are pinned.

use std::collections::{BTreeMap, BTreeSet};

use fontcraft_geom::variation::{MAX_AXES, VariationModel, normalize_value};
use fontcraft_geom::{Affine, Point};
use fontcraft_plist::{Dict, Num, Value};

use crate::{Component, Font, Glyph, Layer, MAX_COMPONENT_DEPTH, MAX_OUTLINE_ELEMENTS, Path, Shape, axes, fc_keys, in_range};

/// Most components looked at, nested ones included, while one layer's pinned components are drawn.
pub const MAX_PINNED_VISITS: usize = 4_096;
/// Most layers a base glyph is interpolated from.
pub const MAX_LOCATION_SOURCES: usize = 64;
/// Most nodes drawing the pinned components of one font may add.
pub const MAX_PINNED_NODES: usize = 5_000_000;

/// What [`draw_pinned_components`] did.
#[derive(Debug, Default)]
pub struct PinnedOutlines {
    /// Components written as paths.
    pub drawn: usize,
    /// Components left as components, each with the reason.
    pub kept: Vec<String>,
}

impl Component {
    /// The location the component is drawn at, by axis tag in design coordinates; `None` when it
    /// has none or when what is stored cannot be read.
    pub fn location(&self) -> Option<BTreeMap<String, f64>> {
        let d = self.info.user_data.get(fc_keys::COMPONENT_LOCATION)?.as_dict()?;
        if d.is_empty() || d.len() > MAX_AXES {
            return None;
        }
        d.entries.iter().map(|(tag, v)| Some((tag.as_str().to_string(), v.as_num().map(Num::as_f64).filter(|v| in_range(*v))?))).collect()
    }

    /// Sets the location the component is drawn at; an empty one takes it away. Values that are
    /// not numbers or beyond the coordinate limit are left out.
    pub fn set_location(&mut self, location: &BTreeMap<String, f64>) {
        let mut d = Dict::new();
        for (tag, v) in location.iter().take(MAX_AXES) {
            if let Some(v) = Some(*v).filter(|v| in_range(*v)).and_then(axes::number_value) {
                d.insert(tag.as_str(), v);
            }
        }
        let data = &mut self.info.user_data.0;
        if !d.is_empty() {
            data.insert(fc_keys::COMPONENT_LOCATION, Value::Dict(d));
        } else if let Some(i) = data.entries.iter().position(|(k, _)| k.as_str() == fc_keys::COMPONENT_LOCATION) {
            data.remove_at(i);
        }
    }
}

/// Whether the component `c` of `layer` is drawn from another layer of its base glyph than the one
/// `layer` would give it.
pub fn is_redirected(font: &Font, c: &Component, layer: &Layer) -> bool {
    let Some(base) = font.glyph(&c.glyph) else { return false };
    if c.location().is_some() {
        return true;
    }
    let Some(pinned) = c.pinned_layer(base) else { return false };
    // An intermediate layer draws its components at its own place, not at its master's.
    pinned.id != layer.master || !layer.info.attrs.coordinates.is_empty()
}

/// The shapes of `layer` with its caps, other attachments and corners fitted; `None` when it has
/// none. A layer whose attachments cannot be fitted has its corners only, and one whose corners
/// cannot be fitted is as it was.
pub(crate) fn fitted(font: &Font, layer: &Layer) -> Option<Vec<Shape>> {
    crate::attach::insert_attachments(font, layer).ok().flatten().or_else(|| crate::corner::insert_corners(font, layer).ok().flatten())
}

/// `font` with the pinned components of its layers (not the backgrounds) written as paths, and
/// the components of the same base glyphs on the other layers of those glyphs.
pub fn draw_pinned_components(font: &Font) -> (Font, PinnedOutlines) {
    let mut report = PinnedOutlines::default();
    let mut changes: Vec<(usize, usize, Vec<Shape>)> = Vec::new();
    let mut nodes_left = MAX_PINNED_NODES;
    for (gi, glyph) in font.glyphs.iter().enumerate() {
        let bases: BTreeSet<&str> = glyph
            .layers
            .iter()
            .flat_map(|l| l.shapes.iter().filter_map(move |s| if let Shape::Component(c) = s { Some((l, c)) } else { None }))
            .filter(|(l, c)| is_redirected(font, c, l))
            .map(|(_, c)| c.glyph.as_str())
            .collect();
        if bases.is_empty() {
            continue;
        }
        for (li, layer) in glyph.layers.iter().enumerate() {
            let budget = nodes_left.min(MAX_OUTLINE_ELEMENTS);
            let mut drawing = Drawing { font, nodes_left: budget, visits_left: MAX_PINNED_VISITS };
            let mut shapes = Vec::with_capacity(layer.shapes.len());
            let mut drawn = 0;
            for shape in &layer.shapes {
                if let Shape::Component(c) = shape
                    && bases.contains(c.glyph.as_str())
                {
                    let mut paths = Vec::new();
                    // An unpinned component of an intermediate layer is drawn at that layer's place.
                    let own = (!is_redirected(font, c, layer)).then(|| at_layer(font, c, layer)).flatten();
                    match drawing.component(own.as_ref().unwrap_or(c), &layer.master, 1, &mut paths) {
                        Ok(()) => {
                            drawn += 1;
                            shapes.extend(paths.into_iter().map(Shape::Path));
                            continue;
                        }
                        Err(reason) => report.kept.push(format!(
                            "glyph `{}` layer `{}`: the component `{}` stays a component: {reason}",
                            glyph.name,
                            layer.info.name.as_deref().unwrap_or(&layer.id),
                            c.glyph
                        )),
                    }
                }
                shapes.push(shape.clone());
            }
            nodes_left = nodes_left.saturating_sub(budget.saturating_sub(drawing.nodes_left));
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

/// `c` at the location of the intermediate layer `layer`; `None` for any other layer.
fn at_layer(font: &Font, c: &Component, layer: &Layer) -> Option<Component> {
    let coordinates = &layer.info.attrs.coordinates;
    if coordinates.is_empty() {
        return None;
    }
    let location =
        font.axes.iter().zip(coordinates).filter_map(|(a, v)| v.filter(|v| v.is_finite()).map(|v| (a.tag.clone(), v))).collect::<BTreeMap<_, _>>();
    let mut c = c.clone();
    c.set_location(&location);
    Some(c)
}

/// The paths of `base` interpolated at `location` for a component on a layer of `master`, in
/// `base`'s coordinates. Takes one from `visits` for each component looked at.
pub(crate) fn located_paths(
    font: &Font,
    base: &Glyph,
    location: &BTreeMap<String, f64>,
    master: &str,
    depth: usize,
    visits: &mut usize,
) -> Result<Vec<Path>, String> {
    let mut drawing = Drawing { font, nodes_left: MAX_OUTLINE_ELEMENTS, visits_left: *visits };
    let result = drawing.located(base, location, master, depth);
    *visits = drawing.visits_left;
    result
}

/// The paths the component `c` draws on a layer of `master`, nested components included.
pub fn component_paths(font: &Font, c: &Component, master: &str) -> Result<Vec<Path>, String> {
    let mut out = Vec::new();
    let mut drawing = Drawing { font, nodes_left: MAX_OUTLINE_ELEMENTS, visits_left: MAX_PINNED_VISITS };
    drawing.component(c, master, 1, &mut out)?;
    Ok(out)
}

/// Turns components into paths, within the limits of one layer.
struct Drawing<'f> {
    font: &'f Font,
    nodes_left: usize,
    visits_left: usize,
}

impl Drawing<'_> {
    /// Appends the paths the component `c` draws on a layer of `master`; `depth` is 1 for a
    /// component of the layer drawn.
    fn component(&mut self, c: &Component, master: &str, depth: usize, out: &mut Vec<Path>) -> Result<(), String> {
        if depth > MAX_COMPONENT_DEPTH {
            return Err(format!("components nest more than {MAX_COMPONENT_DEPTH} deep below `{}`, or a glyph uses itself", c.glyph));
        }
        self.visits_left = self.visits_left.checked_sub(1).ok_or_else(|| format!("drawing it visits more than {MAX_PINNED_VISITS} components"))?;
        if c.transform.numbers().iter().any(|v| !v.is_finite()) {
            return Err(format!("the transform of the component `{}` is not a number", c.glyph));
        }
        let font = self.font;
        let base = font.glyph(&c.glyph).ok_or_else(|| format!("the component's glyph `{}` does not exist", c.glyph))?;
        // Each smart component drawn counts as a visit.
        if let Ok(Some(drawn)) = crate::smart::component_shapes(font, c, master, &mut self.visits_left) {
            for shape in &drawn {
                self.shape(shape, Affine::IDENTITY, master, depth, out)?;
            }
            return Ok(());
        }
        if let Some(location) = c.location() {
            for path in self.located(base, &location, master, depth)? {
                self.shape(&Shape::Path(path), c.affine(), master, depth, out)?;
            }
            return Ok(());
        }
        let Some(layer) = c.pinned_layer(base).or_else(|| base.layer(master)) else { return Ok(()) };
        self.layer(layer, c.affine(), depth, out)
    }

    /// The paths of `base` at `location`, interpolated from what its sources draw.
    fn located(&mut self, base: &Glyph, location: &BTreeMap<String, f64>, master: &str, depth: usize) -> Result<Vec<Path>, String> {
        self.visits_left = self.visits_left.checked_sub(1).ok_or_else(|| format!("drawing it visits more than {MAX_PINNED_VISITS} components"))?;
        let sources = Sources::of(self.font, base, location, master)?;
        let mut drawn: Vec<Vec<Path>> = Vec::with_capacity(sources.layers.len());
        for layer in &sources.layers {
            let mut paths = Vec::new();
            self.layer(layer, Affine::IDENTITY, depth, &mut paths)?;
            drawn.push(paths);
        }
        let shape = |paths: &[Path]| paths.iter().map(|p| (p.closed, p.nodes.iter().map(|n| n.on_curve()).collect::<Vec<_>>())).collect::<Vec<_>>();
        let mut shapes = drawn.iter().map(|p| shape(p));
        if let Some(first) = shapes.next()
            && shapes.any(|s| s != first)
        {
            return Err(format!("the masters of `{}` are not compatible", base.name));
        }
        let rows: Vec<Vec<f64>> = drawn.iter().map(|paths| paths.iter().flat_map(|p| p.nodes.iter().flat_map(|n| [n.x, n.y])).collect()).collect();
        let values = sources.model.interpolate(&sources.at, &rows).map_err(|e| format!("`{}` cannot be interpolated: {e}", base.name))?;
        let mut out = drawn.into_iter().next().unwrap_or_default();
        let nodes = out.iter_mut().flat_map(|p| p.nodes.iter_mut());
        for (n, &[x, y]) in nodes.zip(values.as_chunks::<2>().0) {
            (n.x, n.y) = (x, y);
        }
        Ok(out)
    }

    /// Appends the paths of `layer`, moved by `affine`.
    fn layer(&mut self, layer: &Layer, affine: Affine, depth: usize, out: &mut Vec<Path>) -> Result<(), String> {
        let fitted = fitted(self.font, layer);
        for shape in fitted.as_deref().unwrap_or(&layer.shapes) {
            self.shape(shape, affine, &layer.master, depth, out)?;
        }
        Ok(())
    }

    fn shape(&mut self, shape: &Shape, affine: Affine, master: &str, depth: usize, out: &mut Vec<Path>) -> Result<(), String> {
        match shape {
            Shape::Path(path) => {
                self.nodes_left = self.nodes_left.checked_sub(path.nodes.len()).ok_or_else(|| "the drawing would hold too many nodes".to_string())?;
                let mut path = path.clone();
                for n in &mut path.nodes {
                    let at = affine * Point::new(n.x, n.y);
                    (n.x, n.y) = (at.x, at.y);
                }
                if !path.nodes.iter().all(|n| in_range(n.x) && in_range(n.y)) {
                    return Err("the drawing lies beyond the coordinate limit".into());
                }
                if affine.determinant() < 0.0 {
                    path.reverse();
                }
                out.push(path);
                Ok(())
            }
            Shape::Component(inner) => {
                let mut inner = inner.clone();
                if affine != Affine::IDENTITY {
                    inner.transform = crate::Transform::Matrix((affine * inner.affine()).as_coeffs());
                }
                self.component(&inner, master, depth + 1, out)
            }
            Shape::Image(_) | Shape::Raw(_) => Ok(()),
        }
    }
}

/// The layers a glyph is interpolated from, with the variation model over their locations and
/// the wanted location in it.
struct Sources<'a> {
    layers: Vec<&'a Layer>,
    model: VariationModel,
    at: Vec<f64>,
}

impl<'a> Sources<'a> {
    fn of(font: &Font, base: &'a Glyph, location: &BTreeMap<String, f64>, master: &str) -> Result<Self, String> {
        let count = font.axes.len();
        if count > MAX_AXES {
            return Err(format!("the font has more than {MAX_AXES} axes"));
        }
        let place = |m: &crate::Master| (0..count).map(|i| axes::design_value(&m.location, i)).collect::<Vec<f64>>();
        let origin = axes::origin(font).ok_or("the font has no masters")?;
        let home = font.master(master).unwrap_or(origin);
        let masters: Vec<(&str, Vec<f64>)> = font.masters.iter().map(|m| (m.id.as_str(), place(m))).collect();
        // The origin's layer first: its paths give the result its nodes' kinds.
        let mut found: Vec<(&'a Layer, Vec<f64>)> = Vec::new();
        let mut add = |layer: &'a Layer, at: Vec<f64>| {
            if found.len() < MAX_LOCATION_SOURCES && !found.iter().any(|(_, l)| *l == at) {
                found.push((layer, at));
            }
        };
        let no_layer = || format!("`{}` has no layer for master `{}`", base.name, origin.name);
        add(base.layer(&origin.id).ok_or_else(no_layer)?, place(origin));
        for (id, at) in &masters {
            if let Some(layer) = base.layer(id) {
                add(layer, at.clone());
            }
        }
        for layer in base.layers.iter().filter(|l| !l.is_master_layer() && !l.info.attrs.coordinates.is_empty()) {
            let a = &layer.info.attrs;
            let outline = a.color_palette.is_none() && !a.full_color && !a.svg && a.sbix_size.is_none() && a.smart_poles.is_empty();
            let Some((_, at)) = masters.iter().find(|(id, _)| *id == layer.master).filter(|_| outline && a.axis_rules.is_empty()) else { continue };
            let own = |i: usize, v: f64| a.coordinates.get(i).copied().flatten().filter(|v| v.is_finite()).unwrap_or(v);
            add(layer, at.iter().enumerate().map(|(i, v)| own(i, *v)).collect());
        }
        let default = place(origin);
        let wanted: Vec<f64> = place(home)
            .into_iter()
            .enumerate()
            .map(|(i, v)| font.axes.get(i).and_then(|a| location.get(&a.tag)).copied().filter(|v| in_range(*v)).unwrap_or(v))
            .collect();
        // Each axis runs from −1 at its lowest source to 1 at its highest, 0 at the origin.
        let range = |i: usize| {
            let values = || found.iter().filter_map(move |(_, l)| l.get(i).copied());
            (values().fold(f64::INFINITY, f64::min), values().fold(f64::NEG_INFINITY, f64::max))
        };
        let ranges: Vec<(f64, f64)> = (0..count).map(range).collect();
        let normalized = |at: &[f64]| -> Result<Vec<f64>, String> {
            at.iter()
                .zip(&ranges)
                .zip(&default)
                .map(|((v, (low, high)), d)| normalize_value(*v, *low, *d, *high, true).filter(|v| v.is_finite()))
                .collect::<Option<Vec<f64>>>()
                .ok_or_else(|| "a location is not a usable number".to_string())
        };
        let locations = found.iter().map(|(_, l)| normalized(l)).collect::<Result<Vec<_>, _>>()?;
        let model = VariationModel::new(&locations, true).map_err(|e| format!("`{}` cannot be interpolated: {e}", base.name))?;
        Ok(Self { at: normalized(&wanted)?, layers: found.into_iter().map(|(l, _)| l).collect(), model })
    }
}

#[cfg(test)]
#[path = "pinned_tests.rs"]
mod tests;
