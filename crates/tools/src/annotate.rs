//! The Annotation tool (`A`) and its six sub-tools (`[[tool]] annotation…`, `docs/spec/03b`
//! question 11).
//!
//! Each sub-tool is a tool of its own (`annotationSelect`, `annotationText`, …), so
//! `annotation.setTool` is a tool switch; `annotation` names the Selection sub-tool.
//!
//! | Gesture | Result |
//! |---|---|
//! | click empty canvas (Text, Arrow, Circle, Plus, Minus) | `annotation.add` of that type at the pointer, with the command's defaults |
//! | click empty canvas (Selection) | clear the selection (`Shift`: keep it) |
//! | click an annotation | select it; `Shift` toggles it |
//! | drag an annotation | `annotation.move` of the selected annotations in one interaction |
//! | drag the handle of a selected annotation | `annotation.update` in one interaction: a note's box width, an arrow's angle, a circle's diameter |
//! | `Cmd+A` | `annotation.selectAll` |
//! | arrow keys | `annotation.move` by the font's increments (plain, `Shift`, `Cmd`) |
//! | `Delete` | `annotation.delete` of the selected annotations |
//! | `Escape` during a drag | cancel it |
//!
//! Geometry (our choices; the handbook shows the marks but gives no sizes): a note's position is
//! the top-left corner of its box, and its handle the box's top-right corner; an arrow's position
//! is its tip, its angle points from the tip to the tail, and its handle is the tail; a circle's
//! position is its centre, and its handle the bottom of the circle. Arrows, plus and minus signs
//! and a note's line height keep one size on screen at every zoom.

use fontcraft_geom::Point;
use fontcraft_model::Annotation;
use serde_json::{Map, Value, json};

use crate::event::{Key, KeyEvent, Modifiers, PointerEvent, PointerKind};
use crate::geometry::{dist, finite};
use crate::output::{Action, Invocation, Output};
use crate::selection::{Item, Selection};
use crate::tool::{Context, Tool};
use crate::view::View;

/// An arrow's length, in screen points.
pub const ARROW_LENGTH: f64 = 48.0;
/// Half the width of a plus or minus sign, in screen points.
pub const MARK_RADIUS: f64 = 8.0;
/// The height of one line of a note, in screen points.
pub const TEXT_LINE_HEIGHT: f64 = 16.0;
/// A note's box width when the file stores none (the `annotation.add` default).
pub const DEFAULT_TEXT_WIDTH: f64 = 200.0;
/// A circle's diameter when the file stores none (the `annotation.add` default).
pub const DEFAULT_CIRCLE_DIAMETER: f64 = 100.0;
/// Smallest box width or diameter a handle drag sets, in font units.
pub const MIN_SIZE: f64 = 1.0;
/// Most lines of a note counted for its box; more only make the box taller than any screen.
const MAX_LINES: usize = 1_000;

/// The kinds of annotation FontCraft draws. Files store them capitalised (`Text`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Arrow,
    Circle,
    Plus,
    Minus,
    /// A type FontCraft does not know; drawn as a dot.
    Other,
}

impl Kind {
    pub fn of(a: &Annotation) -> Kind {
        match a.kind.to_ascii_lowercase().as_str() {
            "text" => Kind::Text,
            "arrow" => Kind::Arrow,
            "circle" => Kind::Circle,
            "plus" => Kind::Plus,
            "minus" => Kind::Minus,
            _ => Kind::Other,
        }
    }
}

/// A note's box width in font units: the stored one, else the default.
pub fn text_width(a: &Annotation) -> f64 {
    if a.width.is_finite() && a.width > 0.0 { a.width } else { DEFAULT_TEXT_WIDTH }
}

/// A circle's diameter in font units: the stored one, else the default.
pub fn diameter(a: &Annotation) -> f64 {
    if a.width.is_finite() && a.width > 0.0 { a.width } else { DEFAULT_CIRCLE_DIAMETER }
}

/// The lines of a note's text (at least one).
pub fn text_lines(a: &Annotation) -> usize {
    a.text.as_deref().map_or(1, |t| t.lines().take(MAX_LINES).count().max(1))
}

/// A note's box, top-left and bottom-right corners in font units.
pub fn text_box(a: &Annotation, view: &View) -> (Point, Point) {
    let height = view.units(TEXT_LINE_HEIGHT) * text_lines(a) as f64;
    (Point::new(a.x, a.y), Point::new(a.x + text_width(a), a.y - height))
}

/// An arrow's tail in font units: [`ARROW_LENGTH`] screen points from its tip, along its angle.
pub fn arrow_tail(a: &Annotation, view: &View) -> Point {
    let angle = if a.angle.is_finite() { a.angle.to_radians() } else { 0.0 };
    let len = view.units(ARROW_LENGTH);
    Point::new(a.x + len * angle.cos(), a.y + len * angle.sin())
}

/// The point a handle drag starts from, in font units: a note's top-right corner, an arrow's
/// tail, a circle's bottom. Plus, minus and unknown types have none.
pub fn handle(a: &Annotation, view: &View) -> Option<Point> {
    match Kind::of(a) {
        Kind::Text => Some(Point::new(a.x + text_width(a), a.y)),
        Kind::Arrow => Some(arrow_tail(a, view)),
        Kind::Circle => Some(Point::new(a.x, a.y - diameter(a) / 2.0)),
        Kind::Plus | Kind::Minus | Kind::Other => None,
    }
}

/// Distance from `p` to the segment `a`–`b`.
fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    if len2.is_nan() || len2 <= 0.0 {
        return dist(p, a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    dist(p, Point::new(a.x + t * dx, a.y + t * dy))
}

/// Whether `p` (font units) is on annotation `a`, within `tol` font units.
pub fn hits(a: &Annotation, view: &View, p: Point, tol: f64) -> bool {
    let at = Point::new(a.x, a.y);
    if !finite(at) || !finite(p) {
        return false;
    }
    match Kind::of(a) {
        Kind::Text => {
            let (lo, hi) = text_box(a, view);
            (lo.x - tol..=lo.x.max(hi.x) + tol).contains(&p.x) && (hi.y - tol..=lo.y + tol).contains(&p.y)
        }
        Kind::Arrow => segment_distance(p, at, arrow_tail(a, view)) <= tol,
        Kind::Circle => dist(p, at) <= diameter(a) / 2.0 + tol,
        Kind::Plus | Kind::Minus | Kind::Other => dist(p, at) <= view.units(MARK_RADIUS) + tol,
    }
}

/// The annotation sub-tools, in palette order: `annotation.setTool` name, tool id, the type it adds.
pub const SUB_TOOLS: &[(&str, &str, Option<&str>)] = &[
    ("select", "annotationSelect", None),
    ("text", "annotationText", Some("text")),
    ("arrow", "annotationArrow", Some("arrow")),
    ("circle", "annotationCircle", Some("circle")),
    ("plus", "annotationPlus", Some("plus")),
    ("minus", "annotationMinus", Some("minus")),
];

/// The tool id of an `annotation.setTool` name.
pub fn sub_tool_id(name: &str) -> Option<&'static str> {
    SUB_TOOLS.iter().find(|(n, _, _)| *n == name).map(|(_, id, _)| *id)
}

/// What a press grabbed.
#[derive(Clone, Debug, PartialEq)]
enum Grab {
    /// The handle of annotation `index`.
    Handle(usize),
    /// Annotation `index`; dragging moves `selection`'s annotations.
    Body {
        index: usize,
        reduce_to: Option<Selection>,
    },
    Empty,
}

#[derive(Clone, Debug)]
struct Press {
    down: Point,
    grab: Grab,
    mods: Modifiers,
    selection: Selection,
    /// The drag began an interaction.
    began: bool,
}

/// One annotation sub-tool.
#[derive(Clone, Debug)]
pub struct AnnotateTool {
    id: &'static str,
    /// The type a click on empty canvas adds; `None` for the Selection sub-tool.
    adds: Option<&'static str>,
    press: Option<Press>,
}

impl AnnotateTool {
    /// The sub-tool with this tool id; `annotation` is the Selection sub-tool.
    pub fn new(id: &str) -> Option<Self> {
        let id = if id == "annotation" { "annotationSelect" } else { id };
        SUB_TOOLS.iter().find(|(_, t, _)| *t == id).map(|&(_, id, adds)| Self { id, adds, press: None })
    }

    fn grab(cx: &Context<'_>, p: Point) -> Grab {
        let list = &cx.scene.layer.info.annotations;
        let tol = cx.view.units(cx.settings.hit_tolerance);
        for &i in cx.selection.annotations.iter().rev() {
            let Some(h) = list.get(i).and_then(|a| handle(a, &cx.view)) else { continue };
            if dist(h, p) <= tol {
                return Grab::Handle(i);
            }
        }
        // The last annotation is drawn on top.
        match list.iter().enumerate().rev().find(|(_, a)| hits(a, &cx.view, p, tol)) {
            Some((index, _)) => Grab::Body { index, reduce_to: None },
            None => Grab::Empty,
        }
    }
}

/// `glyph` and `layer` of the edited layer.
fn layer_params(cx: &Context<'_>) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("glyph".into(), json!(cx.scene.glyph));
    m.insert("layer".into(), json!(cx.scene.layer_id));
    m
}

fn annotations_of(selection: &Selection) -> Value {
    json!(selection.annotations.iter().collect::<Vec<_>>())
}

/// `annotation.update` parameters for dragging the handle of `a` (index `index`) to `p`.
fn handle_params(cx: &Context<'_>, index: usize, a: &Annotation, p: Point) -> Option<Value> {
    let mut m = layer_params(cx);
    m.insert("annotation".into(), json!(index));
    match Kind::of(a) {
        Kind::Text => m.insert("width".into(), json!((p.x - a.x).max(MIN_SIZE))),
        Kind::Arrow => {
            let (dx, dy) = (p.x - a.x, p.y - a.y);
            if dx == 0.0 && dy == 0.0 {
                return None;
            }
            m.insert("angle".into(), json!(dy.atan2(dx).to_degrees().rem_euclid(360.0)))
        }
        Kind::Circle => m.insert("diameter".into(), json!((2.0 * dist(p, Point::new(a.x, a.y))).max(MIN_SIZE))),
        Kind::Plus | Kind::Minus | Kind::Other => return None,
    };
    Some(Value::Object(m))
}

impl Tool for AnnotateTool {
    fn id(&self) -> &'static str {
        self.id
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        if !finite(ev.pos) {
            return Output::default();
        }
        match ev.kind {
            PointerKind::Down => {
                let mut grab = Self::grab(cx, ev.pos);
                let mut selection = cx.selection.clone();
                let mut out = Output::handled();
                if let Grab::Body { index, reduce_to } = &mut grab {
                    let item = Item::Annotation(*index);
                    if ev.mods.shift {
                        selection.toggle(item);
                    } else if selection.contains(&item) {
                        // A click without a drag keeps only this one; a drag moves them all.
                        *reduce_to = Some(Selection::from_items([item]));
                    } else {
                        selection = Selection::from_items([item]);
                    }
                    if selection != *cx.selection {
                        out.push(Action::Select { selection: selection.clone() });
                    }
                }
                self.press = Some(Press { down: ev.pos, grab, mods: ev.mods, selection, began: false });
                out
            }
            PointerKind::Drag => {
                let Some(press) = self.press.as_mut() else { return Output::default() };
                let moved = dist(cx.view.to_screen(press.down), cx.view.to_screen(ev.pos)) >= cx.settings.drag_threshold;
                let params = match &press.grab {
                    Grab::Handle(i) => cx.scene.layer.info.annotations.get(*i).and_then(|a| handle_params(cx, *i, a, ev.pos)),
                    Grab::Body { .. } if !press.mods.shift && !press.selection.annotations.is_empty() => {
                        let mut m = layer_params(cx);
                        m.insert("annotations".into(), annotations_of(&press.selection));
                        m.insert("dx".into(), json!(ev.pos.x - press.down.x));
                        m.insert("dy".into(), json!(ev.pos.y - press.down.y));
                        Some(Value::Object(m))
                    }
                    _ => None,
                };
                let Some(params) = params.filter(|_| press.began || moved) else { return Output::handled() };
                let (id, label) = if matches!(press.grab, Grab::Handle(_)) {
                    ("annotation.update", "Edit Annotation")
                } else {
                    ("annotation.move", "Move Annotations")
                };
                let mut out = Output::handled();
                if !press.began {
                    press.began = true;
                    out.push(Action::Begin { label: label.into() });
                }
                out.push(Action::Preview { invocation: Invocation::new(id, params) });
                out
            }
            PointerKind::Up => {
                let Some(press) = self.press.take() else { return Output::default() };
                if press.began {
                    return Output::with(Action::Commit);
                }
                match press.grab {
                    Grab::Body { reduce_to: Some(only), .. } => Output::with(Action::Select { selection: only }),
                    Grab::Empty => match self.adds {
                        Some(kind) => {
                            let mut m = layer_params(cx);
                            m.insert("type".into(), json!(kind));
                            m.insert("position".into(), json!([press.down.x, press.down.y]));
                            Output::with(Action::Run { invocation: Invocation::new("annotation.add", Value::Object(m)) })
                        }
                        None if !press.mods.shift && !cx.selection.is_empty() => Output::with(Action::Select { selection: Selection::new() }),
                        None => Output::handled(),
                    },
                    _ => Output::handled(),
                }
            }
            PointerKind::Move | PointerKind::DoubleClick => Output::default(),
        }
    }

    fn key(&mut self, cx: &Context<'_>, ev: &KeyEvent) -> Output {
        if !ev.pressed {
            return Output::default();
        }
        if ev.key == Key::Escape && self.busy() {
            let mut out = self.cancel();
            out.handled = true;
            return out;
        }
        if self.busy() {
            return Output::default();
        }
        let m = ev.mods;
        let selected = &cx.selection.annotations;
        let run = |id: &str, params: Map<String, Value>| Output::with(Action::Run { invocation: Invocation::new(id, Value::Object(params)) });
        match ev.key {
            Key::Char('a') if m.cmd && !m.alt && !m.shift => run("annotation.selectAll", layer_params(cx)),
            Key::Delete | Key::Backspace if !selected.is_empty() && !m.cmd && !m.alt => {
                let mut params = layer_params(cx);
                params.insert("annotations".into(), annotations_of(cx.selection));
                run("annotation.delete", params)
            }
            Key::Left | Key::Right | Key::Up | Key::Down if !selected.is_empty() && !m.alt => {
                let [plain, big, huge] = cx.settings.increments;
                let step = if m.cmd {
                    huge
                } else if m.shift {
                    big
                } else {
                    plain
                };
                let (dx, dy) = match ev.key {
                    Key::Left => (-step, 0.0),
                    Key::Right => (step, 0.0),
                    Key::Up => (0.0, step),
                    _ => (0.0, -step),
                };
                let mut params = layer_params(cx);
                params.insert("annotations".into(), annotations_of(cx.selection));
                params.insert("dx".into(), json!(dx));
                params.insert("dy".into(), json!(dy));
                run("annotation.move", params)
            }
            _ => Output::default(),
        }
    }

    fn cancel(&mut self) -> Output {
        match self.press.take() {
            Some(p) if p.began => Output::with(Action::Cancel),
            _ => Output::default(),
        }
    }

    fn busy(&self) -> bool {
        self.press.is_some()
    }
}
