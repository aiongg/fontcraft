//! The Higher-Order Interpolation tool's display (rows `higherOrderInterpolation`, `hoi.nodeMarkers`,
//! `hoi.knobOnDottedLine`) and its axis (`hoi.selectAxis`).
//!
//! | Item | Drawn as |
//! |---|---|
//! | a node of the edited layer, of the next layer down the axis, of the next layer up | a diamond: `here`, `below`, `above` |
//! | a straight path to a neighbouring layer | a dotted line with a knob in its middle |
//! | a curve | the curve, its knobs, and lines from the handles to their nodes or from the rotation's centre to both nodes |
//!
//! `app::models::hoi::marks` resolves the paths, for this layer (`CanvasModel::hoi`) and for the tool
//! (`Scene::hoi`). The picked knobs, the marquee and the drag's label are the tool's own overlays.

pub mod tokens;

use craft::ui::runtime::commands::CommandRef;
use egui::epaint::CircleShape;
use egui::{Pos2, Rect, Shape, Stroke};
use fontcraft_engine::model::Font;
use fontcraft_geom::Point;
use fontcraft_tools::hoi::{ID, Kind, Span};
use serde_json::{Map, Value, json};

use self::tokens::HoiTokens;
use super::layers::PaintCx;
use crate::app::{FontCraftApp, ViewState};
use crate::shell::commands::{Module, UiCommand};
use crate::shell::params::{quoted, string};

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Most dots drawn in one frame; the dotted lines past it are drawn solid.
const MAX_DOTS: usize = 20_000;

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "hoi.selectAxis",
    label: "HOI Axis",
    menu: &[],
    shortcut: None,
    params: "{axis: tag} the axis along which the Higher-Order Interpolation tool finds the neighbouring layers and edits their curves; the first axis until one is chosen. ui.inspect reports {axis, picked: [{shape, node, toward, index}]} under hoi",
}];

/// The tool's view state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HoiView {
    /// The chosen axis tag; `None`, or a tag the font has no axis of: the first axis.
    pub axis: Option<String>,
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let font = app.session.font().map_err(|e| e.to_string())?;
    let tag = string(&command.params, "axis")?.ok_or("`axis` (an axis tag) is required")?;
    if !font.axes.iter().any(|a| a.tag == tag) {
        return Err(format!("the font has no axis `{}`", quoted(tag)));
    }
    let tag = tag.to_string();
    if app.view.hoi.axis.as_deref() != Some(tag.as_str()) {
        app.view.hoi.axis = Some(tag);
        // The knobs belong to the axis they were picked on.
        let out = app.tools.toolbox.cancel();
        let _ = super::tools::apply::apply(app, out);
        app.view_changed();
    }
    Ok(())
}

/// Whether the tool is the current one.
pub fn active(view: &ViewState) -> bool {
    view.tool == ID
}

/// The tag of the axis the tool edits along.
pub fn axis<'a>(font: &'a Font, view: &ViewState) -> Option<&'a str> {
    let tags = || font.axes.iter().map(|a| a.tag.as_str());
    view.hoi.axis.as_deref().and_then(|tag| tags().find(|t| *t == tag)).or_else(|| tags().next())
}

/// What the axis pop-up shows: the font's axes as (tag, name) and the current one's tag.
#[derive(Clone, Debug, PartialEq)]
pub struct AxisChoice {
    pub axes: Vec<(String, String)>,
    pub current: String,
}

/// Most axes the pop-up lists.
const MAX_AXES: usize = 200;

/// The axis pop-up, while the tool is the current one in an Edit View tab and the font has an axis.
pub fn axis_choice(app: &FontCraftApp) -> Option<AxisChoice> {
    if !active(&app.view) || app.font_view_shows() {
        return None;
    }
    let font = app.session.font().ok()?;
    let current = axis(font, &app.view)?.to_string();
    let axes = font.axes.iter().take(MAX_AXES).map(|a| (a.tag.clone(), a.name.clone())).collect();
    Some(AxisChoice { axes, current })
}

/// `ui.inspect`'s `hoi` key: `null` while another tool is current.
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    let font = app.session.font().ok().filter(|_| active(&app.view));
    let value = font.map(|font| json!({"axis": axis(font, &app.view), "picked": app.tools.toolbox.hoi_picks()}));
    state.insert("hoi".into(), value.unwrap_or(Value::Null));
}

pub fn paint(cx: &PaintCx<'_>) {
    cx.painter.extend(shapes(cx));
}

/// The paths, then the diamonds, then the knobs; nothing outside the canvas body.
pub fn shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let Some(marks) = &cx.model.hoi else { return Vec::new() };
    let t = HoiTokens::standard();
    let body = cx.rect.expand(t.diamond_radius.max(t.knob_radius));
    let mut out = Vec::new();
    let mut dots = MAX_DOTS;
    for span in &marks.spans {
        path(cx, &t, body, span, &mut dots, &mut out);
    }
    for (nodes, color) in [(&marks.below, t.below), (&marks.above, t.above), (&marks.here, t.here)] {
        let diamond = |c: Pos2| {
            let r = t.diamond_radius;
            let corners = vec![Pos2::new(c.x, c.y - r), Pos2::new(c.x + r, c.y), Pos2::new(c.x, c.y + r), Pos2::new(c.x - r, c.y)];
            Shape::convex_polygon(corners, color, Stroke::new(t.line_width, t.diamond_edge))
        };
        out.extend(nodes.iter().filter_map(|p| at(cx, *p)).filter(|p| body.contains(*p)).map(diamond));
    }
    for span in &marks.spans {
        let color = if span.kind == Kind::Linear { t.linear_knob } else { t.knob };
        out.extend(span.knobs.iter().filter_map(|p| at(cx, *p)).filter(|p| body.contains(*p)).map(|center| {
            Shape::Circle(CircleShape { center, radius: t.knob_radius, fill: color, stroke: Stroke::new(t.line_width, t.diamond_edge) })
        }));
    }
    out
}

/// A font point on screen, when it is finite there.
fn at(cx: &PaintCx<'_>, p: Point) -> Option<Pos2> {
    Some(cx.camera.to_screen(p)).filter(|p| p.x.is_finite() && p.y.is_finite())
}

/// One span's path and the lines of its knobs.
fn path(cx: &PaintCx<'_>, t: &HoiTokens, body: Rect, span: &Span, dots: &mut usize, out: &mut Vec<Shape>) {
    let line = |a: Point, b: Point| -> Option<(Pos2, Pos2)> { clip(at(cx, a)?, at(cx, b)?, body) };
    if span.kind == Kind::Linear {
        let Some((a, b)) = line(span.lower, span.upper) else { return };
        let count = (a.distance(b) / t.dot_spacing).ceil().max(0.0) as usize + 1;
        if count <= *dots {
            *dots -= count;
            out.extend(Shape::dotted_line(&[a, b], t.line, t.dot_spacing, t.dot_radius));
        } else {
            out.push(Shape::line_segment([a, b], Stroke::new(t.line_width, t.line)));
        }
        return;
    }
    let arms: Vec<(Point, Point)> = match (span.kind, span.knobs.as_slice()) {
        (Kind::Handles, [h1, h2]) => vec![(span.lower, *h1), (span.upper, *h2)],
        (Kind::Rotation, [centre]) => vec![(*centre, span.lower), (*centre, span.upper)],
        _ => Vec::new(),
    };
    for (a, b) in arms {
        if let Some((a, b)) = line(a, b) {
            out.push(Shape::line_segment([a, b], Stroke::new(t.line_width, t.arm)));
        }
    }
    let points: Vec<Pos2> = span.curve.iter().filter_map(|p| at(cx, *p)).collect();
    if points.len() == span.curve.len() && points.len() > 1 && Rect::from_points(&points).intersects(body) {
        out.push(Shape::line(points, Stroke::new(t.curve_width, t.curve)));
    }
}

/// The part of the segment from `a` to `b` inside `rect` (Liang–Barsky); `None` when it lies outside.
fn clip(a: Pos2, b: Pos2, rect: Rect) -> Option<(Pos2, Pos2)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
    for (step, room) in [(-d.x, a.x - rect.left()), (d.x, rect.right() - a.x), (-d.y, a.y - rect.top()), (d.y, rect.bottom() - a.y)] {
        if step == 0.0 {
            if room < 0.0 {
                return None;
            }
            continue;
        }
        let t = room / step;
        if step < 0.0 { t0 = t0.max(t) } else { t1 = t1.min(t) }
    }
    let (from, to) = (a + d * t0, a + d * t1);
    (t0 <= t1 && from.x.is_finite() && from.y.is_finite() && to.x.is_finite() && to.y.is_finite()).then_some((from, to))
}
