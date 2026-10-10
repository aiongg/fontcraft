//! The selection's bounding box with its knobs, and the transformation origin.
//!
//! | Item | Drawn as | When |
//! |---|---|---|
//! | bounding box | rectangle a few points outside the selection, `bounding_box` | Select tool, `view.showBoundingBox` on, more than one item or a whole shape selected |
//! | knobs | eight filled squares of side `knob_size` on its corners and side centres | with the box |
//! | origin | ring with a crosshair, `transform_origin` | Rotate and Scale tools: the point a drag turns or scales about |
//! | reference point | crosshair, `transform_origin` | other tools, while the origin mode is the reference point |
//!
//! The box and knob positions come from [`fontcraft_tools::bbox`], so they are where the Select
//! tool finds them.

use egui::{Pos2, Rect, Shape, Stroke, Vec2};
use fontcraft_geom::Point;
use fontcraft_model::Font;
use fontcraft_tools::View;
use fontcraft_tools::bbox::{center, drawn_box, knob_points, selection_box, shown_box};

use crate::shell::transform::{OriginMode, TransformSettings};
use crate::state::Tab;
use crate::theme::Tokens;

fn pos(view: View, p: Point) -> Option<Pos2> {
    let s = view.to_screen(p);
    let at = Pos2::new(s.x as f32, s.y as f32);
    (at.x.is_finite() && at.y.is_finite()).then_some(at)
}

/// The shapes for the box of `tab`'s selection and the origin, for the active tool `tool`.
pub fn shapes(font: &Font, tab: &Tab, tool: &str, show_box: bool, settings: &TransformSettings, view: View, tokens: &Tokens) -> Vec<Shape> {
    let mut out = Vec::new();
    let freehand = matches!(tool, "rotate" | "scale");
    let box_wanted = show_box && tool == "select";
    let reference = settings.reference().filter(|_| settings.mode == OriginMode::Reference);
    if !(box_wanted || freehand || reference.is_some()) {
        return out;
    }
    let Some(scene) = crate::tools_glue::scene(font, tab) else { return out };
    let units = view.units(1.0);
    if box_wanted && let Some(r) = shown_box(&scene, &tab.selection) {
        let outer = drawn_box(r, units);
        let corners =
            [Point::new(outer.x0, outer.y0), Point::new(outer.x1, outer.y0), Point::new(outer.x1, outer.y1), Point::new(outer.x0, outer.y1)];
        let line: Vec<Pos2> = corners.iter().filter_map(|p| pos(view, *p)).collect();
        if line.len() == corners.len() {
            out.push(Shape::closed_line(line, Stroke::new(1.0, tokens.bounding_box)));
            for (_, at) in knob_points(r, units) {
                if let Some(at) = pos(view, at) {
                    out.push(Shape::rect_filled(Rect::from_center_size(at, Vec2::splat(tokens.knob_size)), 0.0, tokens.bounding_box));
                }
            }
        }
    }
    let stroke = Stroke::new(tokens.selection_width, tokens.transform_origin);
    let radius = tokens.transform_origin_radius;
    let origin = if freehand { settings.reference().or_else(|| selection_box(&scene, &tab.selection).map(center)) } else { reference };
    if let Some(at) = origin.and_then(|p| pos(view, p)) {
        let arm = if freehand { radius * 1.6 } else { radius };
        out.push(Shape::line_segment([at - Vec2::new(arm, 0.0), at + Vec2::new(arm, 0.0)], stroke));
        out.push(Shape::line_segment([at - Vec2::new(0.0, arm), at + Vec2::new(0.0, arm)], stroke));
        if freehand {
            out.push(Shape::circle_stroke(at, radius, stroke));
        }
    }
    out
}
