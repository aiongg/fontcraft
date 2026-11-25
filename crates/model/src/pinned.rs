//! Components that do not follow the layer they are on: pinned to another master or layer of
//! their base glyph ([`Component::pinned_layer`]), drawn as outlines for export.
//!
//! A compiled composite refers to its base glyph at the same place of the design space, so it
//! cannot hold such a component. [`draw_pinned_components`] writes it as the paths it draws,
//! nested components included. The other layers of the glyph get the paths of their components
//! of the same base glyph, so the glyph still interpolates when only some masters are pinned.

use std::collections::BTreeSet;

use fontcraft_geom::{Affine, Point};

use crate::{Component, Font, Layer, MAX_COMPONENT_DEPTH, MAX_OUTLINE_ELEMENTS, Path, Shape, in_range};

/// Most components looked at, nested ones included, while one layer's pinned components are drawn.
pub const MAX_PINNED_VISITS: usize = 10_000;
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

/// Whether the component `c` of `layer` is drawn from another layer of its base glyph than the one
/// `layer` would give it.
pub fn is_redirected(font: &Font, c: &Component, layer: &Layer) -> bool {
    let Some(base) = font.glyph(&c.glyph) else { return false };
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
                    match drawing.component(c, &layer.master, 1, &mut paths) {
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
        let Some(layer) = c.pinned_layer(base).or_else(|| base.layer(master)) else { return Ok(()) };
        self.layer(layer, c.affine(), depth, out)
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

#[cfg(test)]
#[path = "pinned_tests.rs"]
mod tests;
