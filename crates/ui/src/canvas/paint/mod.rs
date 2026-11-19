//! The edit view's drawing, one module per paint layer (`docs/spec/03a` § 4, `03b`).
//!
//! | Module | Draws |
//! |---|---|
//! | [`status`] | the glyph status tint behind the edited glyph |
//! | [`background`] | the darker canvas while the background is edited; the other visible layers and the other side of the layer |
//! | [`metrics`] | alignment zones, metric lines and their names, the edited glyph's side lines |
//! | [`outline`] | the edited side's composite outline, over the paths as drawn when strokes or masks change it |
//! | [`components`] | components by alignment state, placeholders, corner components |
//! | [`nodes`] | nodes and handles, start and open-end marks, duplicate nodes, metric highlights, Extra Nodes |
//! | [`anchors`] | anchors by metric position |

pub mod anchors;
pub mod background;
pub mod components;
pub mod metrics;
pub mod nodes;
pub mod outline;
pub mod status;

use craft::behavior::kurbo::Affine;
use egui::{Pos2, Shape, Stroke};
use fontcraft_geom::{BezPath, PathEl, Point};

use super::layers::PaintCx;

craft::ui::words! {
    /// The words the canvas paints.
    pub struct CanvasWords {
        /// A component placeholder's label after the base glyph's name: its base glyph has no outline.
        empty: "fontcraftCanvas.empty" = "empty";
        /// A component placeholder's label: no glyph has the base glyph's name.
        missing: "fontcraftCanvas.missing" = "no such glyph";
        /// A component placeholder's label: the base glyph contains the glyph the component is in.
        circular: "fontcraftCanvas.circular" = "circular reference";
    }
}

/// Most points one outline is flattened to. The model bounds coordinates and outline size, so real glyphs stay far
/// below this; it keeps a pathological one from filling memory.
const MAX_FLATTEN_POINTS: usize = 200_000;

/// The contours of `path` mapped by `affine`, as polylines (curves flattened to a quarter of a screen point). A
/// closed contour ends where it began.
pub fn flatten(path: &BezPath, affine: Affine) -> Vec<Vec<Pos2>> {
    let mapped = affine * path.clone();
    let mut contours: Vec<Vec<Pos2>> = Vec::new();
    let mut count = 0usize;
    fontcraft_geom::flatten(&mapped, 0.25, |el| {
        count += 1;
        if count > MAX_FLATTEN_POINTS {
            return;
        }
        match el {
            PathEl::MoveTo(p) => contours.push(vec![pos(p)]),
            PathEl::LineTo(p) => {
                if let Some(c) = contours.last_mut() {
                    c.push(pos(p));
                }
            }
            PathEl::ClosePath => {
                if let Some(c) = contours.last_mut()
                    && let Some(first) = c.first().copied()
                {
                    c.push(first);
                }
            }
            _ => {}
        }
    });
    contours.retain(|c| c.len() > 1 && c.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
    contours
}

/// The polylines that stroke `path` (font units) with `stroke` through the camera.
pub fn outline_shapes(cx: &PaintCx<'_>, path: &BezPath, stroke: Stroke) -> Vec<Shape> {
    flatten(path, cx.camera.affine()).into_iter().map(|contour| Shape::line(contour, stroke)).collect()
}

/// The screen point of the font point `[x, y]`, when it is finite.
pub fn screen(cx: &PaintCx<'_>, [x, y]: [f64; 2]) -> Option<Pos2> {
    let at = cx.camera.to_screen(Point::new(x, y));
    (at.x.is_finite() && at.y.is_finite()).then_some(at)
}

fn pos(p: Point) -> Pos2 {
    Pos2::new(p.x as f32, p.y as f32)
}
