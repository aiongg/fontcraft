//! The selection's bounding box with its knobs, and the point transformations are about.
//!
//! | Item | Drawn as | When |
//! |---|---|---|
//! | bounding box | a rectangle a few points outside the selection, `bounding_box` | the Select tool, View › Show Bounding Box on, more than one item or a whole shape selected |
//! | knobs | eight filled squares on its corners and side centres | with the box, unless the glyph is locked |
//! | origin | a ring with a crosshair, `transform_origin` | the Rotate and Scale tools: the point a drag turns or scales about |
//! | reference point | a crosshair, `transform_origin` | other tools, while the Transformations section works about the reference point |
//!
//! [`refresh`] works out what to draw when the document, the selection, the tool or the origin changed; positions come
//! from [`fontcraft_tools::bbox`], so the box and its knobs are where the Select tool finds them. Text mode draws none.
//! A selected image has a box of its own (`canvas::images`).

use egui::{Pos2, Rect as ScreenRect, Shape, Stroke, Vec2};
use fontcraft_geom::{Point, Rect};
use fontcraft_tools::Selection;
use fontcraft_tools::bbox::{center, drawn_box, knob_points, selection_box, shown_box};

use super::layers::PaintCx;
use super::paint::screen;
use super::tools::tokens::ToolTokens;
use super::tools::{Target, scene, target};
use crate::app::FontCraftApp;
use crate::shell::palette::OriginMode;

/// The point a transformation is about.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Origin {
    pub at: Point,
    /// The Rotate or Scale tool is chosen: a drag is about this point.
    pub active: bool,
}

/// What the canvas marks around the selection, in font units.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marks {
    /// The selection's box, as measured; it is drawn a few points outside.
    pub frame: Option<Rect>,
    pub knobs: bool,
    pub origin: Option<Origin>,
    /// The selected smart components' handles (`canvas::smart`).
    pub handles: Vec<Point>,
}

/// What the marks were worked out from.
#[derive(Clone, Debug, PartialEq)]
pub struct MarksKey {
    doc: Option<(u64, u64)>,
    target: Target,
    selection: Selection,
    focus: Option<Vec<usize>>,
    tool: &'static str,
    boxed: bool,
    image: bool,
    point: Option<[f64; 2]>,
    mode: OriginMode,
    interaction: bool,
}

/// Brings `ToolView::marks` up to date. Runs after each drain.
pub fn refresh(app: &mut FontCraftApp) {
    let target = target(&app.session, &app.view).filter(|_| !super::text::text_mode(&app.view));
    let Some(target) = target else {
        app.tools.marks_key = None;
        if app.tools.view.marks != Marks::default() {
            app.tools.view.marks = Marks::default();
        }
        return;
    };
    let transform = &app.view.palette.transform;
    let key = MarksKey {
        doc: app.session.active().map(|d| (d.id, d.revision)),
        target,
        selection: app.tools.selection.clone(),
        focus: app.tools.focus.clone(),
        tool: app.tools.toolbox.base_id(),
        boxed: app.view.display.bounding_box,
        image: app.tools.image,
        point: transform.point.filter(|p| p.iter().all(|v| v.is_finite())),
        mode: transform.mode,
        interaction: app.session.interaction().is_some(),
    };
    if app.tools.marks_key.as_ref() == Some(&key) {
        return;
    }
    let marks = marks(app, &key);
    app.tools.view.marks = marks;
    app.tools.marks_key = Some(key);
}

fn marks(app: &mut FontCraftApp, key: &MarksKey) -> Marks {
    app.tools.smart.update(&app.session, &app.view, &app.tools.selection);
    let mut out = Marks { handles: app.tools.smart.handles.iter().map(|h| h.at).collect(), ..Marks::default() };
    let freehand = matches!(key.tool, "rotate" | "scale");
    let boxed = key.boxed && key.tool == "select" && !key.image && (key.selection.len() > 1 || !key.selection.shapes.is_empty());
    let placed = key.point.map(|[x, y]| Point::new(x, y));
    let centred = freehand && placed.is_none() && !key.image;
    if (boxed || centred)
        && let Ok(font) = app.session.font()
        && let Some(scene) = scene(font, app.session.glyph_data().ok().as_deref(), &app.view, app.tools.focus.as_ref())
    {
        if boxed {
            out.frame = shown_box(&scene, &key.selection);
            out.knobs = !scene.locked;
        }
        if centred {
            out.origin = selection_box(&scene, &key.selection).map(|r| Origin { at: center(r), active: true });
        }
    }
    match placed {
        Some(at) if freehand => out.origin = Some(Origin { at, active: true }),
        Some(at) if key.mode == OriginMode::Reference => out.origin = Some(Origin { at, active: false }),
        // A drag is about the centre the selection had when it began.
        None if centred && key.interaction && app.tools.view.marks.origin.is_some() => out.origin = app.tools.view.marks.origin,
        _ => {}
    }
    out
}

/// The paint layer of the box, its knobs and the origin.
pub fn paint(cx: &PaintCx<'_>) {
    cx.painter.extend(shapes(cx));
}

pub fn shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let t = ToolTokens::standard();
    let marks = &cx.tools.marks;
    let mut out = Vec::new();
    let units = 1.0 / cx.camera.scale();
    if let Some(frame) = marks.frame.filter(|_| units.is_finite() && units > 0.0) {
        let r = drawn_box(frame, units);
        let corners: Option<Vec<Pos2>> = [[r.x0, r.y0], [r.x1, r.y0], [r.x1, r.y1], [r.x0, r.y1]].iter().map(|p| screen(cx, *p)).collect();
        if let Some(corners) = corners {
            out.push(Shape::closed_line(corners, Stroke::new(t.bounding_box_width, t.bounding_box)));
        }
        if marks.knobs {
            let knobs = knob_points(frame, units).into_iter().filter_map(|(_, p)| screen(cx, [p.x, p.y]));
            out.extend(knobs.map(|at| Shape::rect_filled(ScreenRect::from_center_size(at, Vec2::splat(t.knob_size)), 0.0, t.bounding_box)));
        }
    }
    if let Some(origin) = marks.origin
        && let Some(at) = screen(cx, [origin.at.x, origin.at.y])
    {
        let stroke = Stroke::new(t.selection_width, t.transform_origin);
        let arm = if origin.active { t.transform_origin_radius * 1.6 } else { t.transform_origin_radius };
        out.push(Shape::line_segment([at - Vec2::new(arm, 0.0), at + Vec2::new(arm, 0.0)], stroke));
        out.push(Shape::line_segment([at - Vec2::new(0.0, arm), at + Vec2::new(0.0, arm)], stroke));
        if origin.active {
            out.push(Shape::circle_stroke(at, t.transform_origin_radius, stroke));
        }
    }
    out
}
