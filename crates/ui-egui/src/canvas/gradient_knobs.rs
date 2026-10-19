//! Gradient knobs in Edit View (`color.fullColor.gradientCanvasKnobs`): while paths with a
//! gradient are selected on a full-colour layer, each shows knobs for its gradient's geometry,
//! two for a linear gradient (start and end, joined by a line) and one for a radial gradient
//! (its origin, the centre of its start circle). Conic gradients have no knob; they are placed
//! with `attributes.setGradientPosition` alone.
//!
//! Dragging a knob with the Select tool is one interaction whose previews and final step run
//! `attributes.setGradientPosition` on that path in font units, one undo step: a linear knob
//! moves its own end and keeps the other, the radial knob moves the start circle and the end
//! circle with it, as the command does without `end`. A knob lies above the path's nodes, so a
//! press on it drags the gradient.
//!
//! Positions come from the path's own bounds, curve extremes included, as the command converts
//! them; a path without width or height shows no knob.

use egui::{Pos2, Stroke};
use fontcraft_geom::{Point, Shape as _};
use fontcraft_model::{Font, GradientKind, Shape};
use fontcraft_tools::PointerKind;
use serde_json::{Value, json};

use super::Painting;
use crate::app::FontcraftApp;
use crate::state::Tab;
use crate::tools_glue::{self, RawPointer, Space};

/// Which point of a gradient a knob places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// A linear gradient's start, or a radial gradient's origin.
    Start,
    /// A linear gradient's end.
    End,
}

/// One knob: path `shape` of the edited layer, the point it places, where it is (font units),
/// and the gradient's start and, for a linear gradient, its end.
#[derive(Clone, Debug, PartialEq)]
pub struct Knob {
    pub shape: usize,
    pub end: End,
    pub at: Point,
    pub start: Point,
    pub finish: Option<Point>,
}

/// A knob drag between the press and the release.
#[derive(Clone, Debug)]
struct Dragging {
    knob: Knob,
    tab: u64,
    glyph: String,
    layer: String,
    /// The press in screen points, for the drag threshold.
    from: Point,
    began: bool,
}

/// The knob drag in progress.
#[derive(Default)]
pub struct KnobUi {
    drag: Option<Dragging>,
}

/// The knobs of the paths `selected` of the tab's edited layer; none in text mode, while the
/// background is edited, or on a layer that is not full colour.
pub fn knobs(font: &Font, tab: &Tab, selected: &[usize]) -> Vec<Knob> {
    if tab.text_mode || tab.edit_background {
        return Vec::new();
    }
    let Some(layer) = super::view::active_layer(font, tab) else { return Vec::new() };
    if !layer.info.attrs.full_color {
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

/// The knobs of the selected paths of the showing tab.
pub fn current(app: &FontcraftApp) -> Vec<Knob> {
    let (Ok(font), Some(tab)) = (app.session.font(), app.state.tab()) else { return Vec::new() };
    knobs(font, tab, &crate::panels::stroke::selected_paths(app))
}

/// Draws `knobs`: a line from each linear gradient's start to its end, and a ringed dot per knob.
pub fn draw(c: &Painting, knobs: &[Knob]) {
    let t = &c.tokens.color;
    let screen = |p: Point| {
        let q = c.view.to_screen(p);
        Pos2::new(q.x as f32, q.y as f32)
    };
    for k in knobs.iter().filter(|k| k.end == End::Start) {
        if let Some(e) = k.finish {
            c.painter.line_segment([screen(k.start), screen(e)], Stroke::new(t.knob_line_width, t.knob_line));
        }
    }
    for k in knobs {
        let at = screen(k.at);
        c.painter.circle(at, t.knob_radius, t.knob, Stroke::new(t.knob_outline_width, t.knob_outline));
    }
}

/// The parameters of `attributes.setGradientPosition` that put `knob` at `to`.
pub fn invocation(glyph: &str, layer: &str, knob: &Knob, to: Point) -> Value {
    let mut params = json!({"glyph": glyph, "layer": layer, "shapes": [knob.shape]});
    match (knob.end, knob.finish) {
        (End::Start, Some(e)) => {
            params["start"] = json!([to.x, to.y]);
            params["end"] = json!([e.x, e.y]);
        }
        (End::End, _) => {
            params["start"] = json!([knob.start.x, knob.start.y]);
            params["end"] = json!([to.x, to.y]);
        }
        (End::Start, None) => params["start"] = json!([to.x, to.y]),
    }
    params
}

fn font_point(view: &fontcraft_tools::View, ev: &RawPointer) -> Point {
    match ev.space {
        Space::Font => ev.at,
        Space::Screen => view.to_font(ev.at),
    }
}

/// A pointer event for the gradient knobs, before the placed image and the tools see it: a press
/// on a knob with the Select tool starts a drag, and the drag and release that follow move it.
/// `None` when the knobs do not take the event.
pub fn pointer(app: &mut FontcraftApp, ev: RawPointer) -> Option<Vec<String>> {
    match ev.kind {
        PointerKind::Down => press(app, ev),
        PointerKind::Drag if app.color_ui.knobs.drag.is_some() => Some(drag(app, ev, false)),
        PointerKind::Up if app.color_ui.knobs.drag.is_some() => Some(drag(app, ev, true)),
        _ => None,
    }
}

fn press(app: &mut FontcraftApp, ev: RawPointer) -> Option<Vec<String>> {
    if let Some(d) = app.color_ui.knobs.drag.take()
        && d.began
        && app.session.interaction().is_some()
    {
        let _ = app.session.cancel();
    }
    if app.tools.toolbox.active_id() != "select" {
        return None;
    }
    let knobs = current(app);
    if knobs.is_empty() {
        return None;
    }
    let view = tools_glue::current_view(app)?;
    let font = app.session.font().ok()?;
    let tolerance = tools_glue::settings(&app.tokens, font).hit_tolerance.max(f64::from(app.tokens.color.knob_radius));
    let tab = app.state.tab()?;
    let glyph = tab.edited_glyph()?.to_string();
    let layer = super::view::active_layer(font, tab)?.id.clone();
    let at = view.to_screen(font_point(&view, &ev));
    let knob = knobs
        .into_iter()
        .map(|k| ((view.to_screen(k.at) - at).hypot(), k))
        .filter(|(d, _)| *d <= tolerance)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, k)| k)?;
    app.color_ui.knobs.drag = Some(Dragging { knob, tab: tab.id, glyph, layer, from: at, began: false });
    Some(Vec::new())
}

fn drag(app: &mut FontcraftApp, ev: RawPointer, up: bool) -> Vec<String> {
    let Some(mut d) = app.color_ui.knobs.drag.take() else { return Vec::new() };
    let same_tab = app.state.tab().is_some_and(|t| t.id == d.tab);
    let Some(view) = tools_glue::current_view(app).filter(|_| same_tab) else {
        if d.began && app.session.interaction().is_some() {
            let _ = app.session.cancel();
        }
        return Vec::new();
    };
    let pos = font_point(&view, &ev);
    let mut errors = Vec::new();
    if !d.began {
        let threshold = app.session.font().map_or(0.0, |f| tools_glue::settings(&app.tokens, f).drag_threshold);
        if (view.to_screen(pos) - d.from).hypot() < threshold {
            if !up {
                app.color_ui.knobs.drag = Some(d);
            }
            return errors;
        }
        if let Err(e) = app.session.begin("Set Gradient Position") {
            let e = e.to_string();
            app.state.status = e.clone();
            return vec![e];
        }
        d.began = true;
    }
    let params = invocation(&d.glyph, &d.layer, &d.knob, pos);
    if let Err(e) = app.session.preview("attributes.setGradientPosition", &params) {
        let e = e.to_string();
        app.state.status = e.clone();
        errors.push(e);
    }
    if !up {
        app.color_ui.knobs.drag = Some(d);
        return errors;
    }
    match app.session.commit() {
        Ok(true) => app.state.status = "Set Gradient Position: done".to_string(),
        Ok(false) => {}
        Err(e) => {
            let e = e.to_string();
            app.state.status = e.clone();
            errors.push(e);
        }
    }
    errors
}
