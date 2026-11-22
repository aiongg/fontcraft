//! Gradient knobs (`color.fullColor.gradientCanvasKnobs`): while paths with a gradient are selected on a full-colour
//! layer, each shows knobs for its gradient's geometry: two for a linear gradient (start and end, joined by a line),
//! one for a radial gradient (its origin, the centre of its start circle). Conic gradients have no knob; they are
//! placed with `attributes.setGradientPosition` alone.
//!
//! Dragging a knob with the Select tool is one interaction whose previews and final step run
//! `attributes.setGradientPosition` on that path in font units, one undo step: a linear knob moves its own end and
//! keeps the other; the radial knob moves the start circle and the end circle with it, as the command does without
//! `end`. A knob lies above the path's nodes: [`pointer`] sees the canvas's press before the toolbox does, so a press
//! on a knob drags the gradient.
//!
//! Positions come from the path's own bounds, curve extremes included, as the command converts them; a path without
//! width or height shows no knob.

use craft::ui::runtime::commands::CommandRef;
use egui::Stroke;
use fontcraft_geom::{Point, Shape as _};
use fontcraft_model::{GradientKind, Layer, Shape};
use fontcraft_tools::{PointerEvent, PointerKind, Selection};
use serde_json::{Value, json};

use super::tokens::ColorTokens;
use crate::app::{FontCraftApp, Msg, Preview};
use crate::canvas::layers::PaintCx;
use crate::canvas::tools::{apply, scene, tokens::ToolTokens};

/// The undo label of a knob drag.
pub const LABEL: &str = "Set Gradient Position";
/// The command a knob drag runs.
pub const COMMAND: &str = "attributes.setGradientPosition";

/// Which point of a gradient a knob places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// A linear gradient's start, or a radial gradient's origin.
    Start,
    /// A linear gradient's end.
    End,
}

/// One knob: path `shape` of the edited layer, the point it places, where it is (font units), and the gradient's start
/// and, for a linear gradient, its end.
#[derive(Clone, Debug, PartialEq)]
pub struct Knob {
    pub shape: usize,
    pub end: End,
    pub at: Point,
    pub start: Point,
    pub finish: Option<Point>,
}

/// A knob drag between the press and the release.
#[derive(Clone, Debug, PartialEq)]
struct Dragging {
    knob: Knob,
    glyph: String,
    layer: String,
    /// The press on screen, for the drag threshold.
    from: Point,
    began: bool,
}

/// The knob drag in progress.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KnobDrag {
    drag: Option<Dragging>,
}

/// The paths a selection names: those selected whole or with a selected node or handle.
pub fn selected_paths(selection: &Selection) -> Vec<usize> {
    let mut paths: Vec<usize> = selection.points.iter().map(|&(s, _)| s).chain(selection.shapes.iter().copied()).collect();
    paths.sort_unstable();
    paths.dedup();
    paths
}

/// The knobs of the paths `selected` of `layer`; none while the background is edited or on a layer that is not full
/// colour.
pub fn knobs(layer: &Layer, selected: &[usize], edit_background: bool) -> Vec<Knob> {
    if edit_background || !layer.info.attrs.full_color {
        return Vec::new();
    }
    let mut out = Vec::new();
    for &i in selected {
        let Some(Shape::Path(path)) = layer.shapes.get(i) else { continue };
        let Some(gradient) = path.info.attrs.gradient.as_ref() else { continue };
        let Ok(bez) = path.to_bez() else { continue };
        let b = bez.bounding_box();
        if !(b.width() > 0.0 && b.height() > 0.0) {
            continue;
        }
        let at = |p: Point| Point::new(b.x0 + b.width() * p.x, b.y0 + b.height() * p.y);
        match gradient.kind {
            GradientKind::Linear { start, end } => {
                let (s, e) = (at(start), at(end));
                out.push(Knob { shape: i, end: End::Start, at: s, start: s, finish: Some(e) });
                out.push(Knob { shape: i, end: End::End, at: e, start: s, finish: Some(e) });
            }
            GradientKind::Radial { start, .. } => {
                let s = at(start);
                out.push(Knob { shape: i, end: End::Start, at: s, start: s, finish: None });
            }
            GradientKind::Conic { .. } => {}
        }
    }
    out.retain(|k| k.at.x.is_finite() && k.at.y.is_finite());
    out
}

/// The paint layer: a line from each linear gradient's start to its end, and a ringed dot per knob.
pub fn paint(cx: &PaintCx<'_>) {
    let Some(glyph) = cx.glyph() else { return };
    let list = knobs(&glyph.layer, &selected_paths(&cx.tools.selection), cx.edit_background);
    if list.is_empty() {
        return;
    }
    let t = ColorTokens::standard();
    for k in list.iter().filter(|k| k.end == End::Start) {
        if let Some(e) = k.finish {
            cx.painter.line_segment([cx.camera.to_screen(k.start), cx.camera.to_screen(e)], Stroke::new(t.knob_line_width, t.knob_line));
        }
    }
    for k in &list {
        cx.painter.circle(cx.camera.to_screen(k.at), t.knob_radius, t.knob, Stroke::new(t.knob_outline_width, t.knob_outline));
    }
}

/// The parameters of `attributes.setGradientPosition` that put `knob` at `to`.
pub fn invocation(glyph: &str, layer: &str, knob: &Knob, to: Point) -> Value {
    let (start, end) = match (knob.end, knob.finish) {
        (End::Start, Some(e)) => (to, Some(e)),
        (End::End, _) => (knob.start, Some(to)),
        (End::Start, None) => (to, None),
    };
    let mut params = json!({"glyph": glyph, "layer": layer, "shapes": [knob.shape], "start": [start.x, start.y]});
    if let (Some(e), Value::Object(o)) = (end, &mut params) {
        o.insert("end".into(), json!([e.x, e.y]));
    }
    params
}

/// A pointer event for the knobs, before the toolbox sees it: a press on a knob with the Select tool starts a drag,
/// and the drags and the release that follow move it. `None` when the knobs do not take the event; else the errors.
pub fn pointer(app: &mut FontCraftApp, ev: PointerEvent) -> Option<Vec<String>> {
    match ev.kind {
        PointerKind::Down => press(app, ev),
        PointerKind::Drag if app.knobs.drag.is_some() => Some(drag(app, ev, false)),
        PointerKind::Up if app.knobs.drag.is_some() => Some(drag(app, ev, true)),
        _ => None,
    }
}

fn screen(app: &FontCraftApp, p: Point) -> Point {
    let at = apply::camera(app).to_screen(p);
    Point::new(f64::from(at.x), f64::from(at.y))
}

fn press(app: &mut FontCraftApp, ev: PointerEvent) -> Option<Vec<String>> {
    if let Some(d) = app.knobs.drag.take()
        && d.began
    {
        let _ = app.apply_msg(Msg::Preview(Preview::Cancel));
    }
    if app.tools.toolbox.active_id() != "select" {
        return None;
    }
    apply::sync(app);
    let font = app.session.font().ok()?;
    let scene = scene(font, app.session.glyph_data().ok().as_deref(), &app.view, app.tools.focus.as_ref())?;
    let list = knobs(&scene.layer, &selected_paths(&app.tools.selection), app.view.edit_background);
    if list.is_empty() {
        return None;
    }
    let (glyph, layer) = (scene.glyph.clone(), scene.layer_id.clone());
    let tolerance = f64::from(ToolTokens::standard().hit_tolerance.max(ColorTokens::standard().knob_radius));
    let at = screen(app, ev.pos);
    let knob = list
        .into_iter()
        .map(|k| ((screen(app, k.at) - at).hypot(), k))
        .filter(|(d, _)| *d <= tolerance)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, k)| k)?;
    app.knobs.drag = Some(Dragging { knob, glyph, layer, from: at, began: false });
    Some(Vec::new())
}

fn drag(app: &mut FontCraftApp, ev: PointerEvent, up: bool) -> Vec<String> {
    let Some(mut d) = app.knobs.drag.take() else { return Vec::new() };
    if !d.began {
        let moved = (screen(app, ev.pos) - d.from).hypot();
        if moved < f64::from(ToolTokens::standard().drag_threshold) {
            if !up {
                app.knobs.drag = Some(d);
            }
            return Vec::new();
        }
        if let Err(e) = app.apply_msg(Msg::Preview(Preview::Begin(LABEL.into()))) {
            return vec![e];
        }
        d.began = true;
    }
    let mut errors = Vec::new();
    let step = CommandRef::with_params(COMMAND, invocation(&d.glyph, &d.layer, &d.knob, ev.pos));
    if let Err(e) = app.apply_msg(Msg::Preview(Preview::Update(step))) {
        errors.push(e);
    }
    if !up {
        app.knobs.drag = Some(d);
        return errors;
    }
    if let Err(e) = app.apply_msg(Msg::Preview(Preview::Commit)) {
        errors.push(e);
    }
    errors
}
