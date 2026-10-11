//! What Edit View draws behind the edited layer (`docs/spec/03b` › Background, Palette › Layers).
//!
//! | Item | Drawn as | Shown when |
//! |---|---|---|
//! | the other side of the layer | thin outline, `background_outline` (`background.outlineDisplay`) | Show Background |
//! | its nodes and handles | as the edited side's nodes | Show Background and Show Nodes › In Background |
//! | another layer of the glyph whose eye is on | thin outline, `visible_layer_outline` (`layers.visibleLayerOutline`) | always |
//! | editing the background | the canvas a little darker, `background_tint` (`background.darkenedCanvas`) | [`tint`] |

use egui::Stroke;
use fontcraft_geom::{Affine, BezPath};
use fontcraft_model::{Font, Glyph, Layer, Shape};

use super::Painting;
use super::nodes::{self, Metrics};
use crate::state::DisplayOptions;

/// Most outline elements drawn for one background or one other layer. Each component is already
/// bounded by the model; this bounds a layer with very many of them.
const MAX_ELEMENTS: usize = 200_000;

/// The outline of `shapes` on `master`, components resolved. Shapes that cannot be resolved
/// (a missing component glyph, a cycle, bad coordinates) are left out.
pub(crate) fn outline(font: &Font, shapes: &[Shape], master: &str) -> BezPath {
    let mut out = BezPath::new();
    for shape in shapes {
        let part = match shape {
            Shape::Path(p) => p.to_bez().ok(),
            Shape::Component(c) => font.component_outline(c, master).ok(),
            _ => None,
        };
        let Some(part) = part else { continue };
        if out.elements().len().saturating_add(part.elements().len()) > MAX_ELEMENTS {
            break;
        }
        out.extend(part);
    }
    out
}

fn stroke_outline(c: &Painting, path: BezPath, stroke: Stroke) {
    let v = c.view;
    let to_screen = Affine::translate(v.origin.to_vec2()) * Affine::scale_non_uniform(v.scale(), -v.scale());
    for contour in super::flatten(&(to_screen * path)) {
        c.painter.add(egui::Shape::line(contour, stroke));
    }
}

/// The background of `layer` and the other visible layers of `glyph`, behind the outline. While
/// the background is edited, `layer` is the side view whose background is the foreground.
pub fn draw(c: &Painting, font: &Font, glyph: &Glyph, layer: &Layer, display: &DisplayOptions) {
    let tokens = c.tokens;
    for other in glyph.layers.iter().filter(|l| l.id != layer.id && l.info.visible) {
        let path = outline(font, &other.shapes, &other.master);
        stroke_outline(c, path, Stroke::new(tokens.visible_layer_outline_width, tokens.visible_layer_outline));
    }
    if !display.background {
        return;
    }
    let Some(bg) = layer.info.background.as_ref() else { return };
    let path = outline(font, &bg.shapes, &layer.master);
    stroke_outline(c, path, Stroke::new(tokens.background_outline_width, tokens.background_outline));
    if display.nodes_in_background {
        let shown = Layer { shapes: bg.shapes.clone(), ..Layer::new(layer.master.clone(), layer.width) };
        let none = Metrics { lines: Vec::new(), zones: Vec::new() };
        c.painter.extend(nodes::shapes(&shown, c.view, tokens, &none, None));
    }
}

/// Darkens the canvas a little: the sign that the background is the side being edited
/// (`background.darkenedCanvas`). Drawn first, under everything else.
pub fn tint(c: &Painting) {
    c.painter.rect_filled(c.rect, 0.0, c.tokens.background_tint);
}
