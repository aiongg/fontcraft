//! The Spiro tool: a contour drawn as on-curve control points, with a smooth spiral curve through
//! them (`[[tool]] spiro`). The curve comes from [`fontcraft_geom::spiro`].
//!
//! | Gesture | Result |
//! |---|---|
//! | click | adds a point of the latched kind ([`SpiroTool::set_kind`]); with none latched, a G4 curve point |
//! | Shift+click | a G2 curve point, when no kind is latched |
//! | Alt+click | a corner point, when no kind is latched |
//! | Cmd+click | a left point (curve before, straight line after), when no kind is latched |
//! | Cmd+Shift+click | a right point (straight line before, curve after), when no kind is latched |
//! | drag a point, or drag right after adding one | moves it; the whole curve is recomputed |
//! | click the first point | closes the contour and adds it |
//! | Return, Escape, a double-click, or leaving the tool | adds the open contour |
//! | Backspace or Delete | removes the last point |
//!
//! The contour is added with one `path.addPath`: the curve converted to cubic segments within
//! [`TOLERANCE`]. The spiro points are not kept with the path; once added it is an ordinary
//! outline.

use fontcraft_geom::spiro::{MAX_SPIRO_POINTS, SpiroKind, SpiroPoint, Spline};
use fontcraft_geom::{BezPath, Point};
use serde_json::{Map, Value, json};

use crate::draw::params;
use crate::event::{Key, KeyEvent, Modifiers, PointerEvent, PointerKind};
use crate::geometry::{dist, finite};
use crate::output::{Action, Invocation, Output, Overlay, OverlayKind};
use crate::pencil::nodes_of;
use crate::rapid::{node_value, preview_lines};
use crate::tool::{Context, Tool};

/// How near the added cubics stay to the spiral, in font units.
pub const TOLERANCE: f64 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Held {
    index: usize,
    /// A press on the first point, which closes the contour if the pointer does not move.
    close: bool,
    start: Point,
    moved: bool,
}

/// The Spiro tool.
#[derive(Clone, Debug, Default)]
pub struct SpiroTool {
    points: Vec<SpiroPoint>,
    latched: Option<SpiroKind>,
    held: Option<Held>,
    /// The curve through the points (open), while it can be solved.
    curve: Option<BezPath>,
    /// The command target, from the last event.
    target: Map<String, Value>,
}

impl SpiroTool {
    /// Latches the kind of point a plain click adds; `None` returns to modifier keys.
    pub fn set_kind(&mut self, kind: Option<SpiroKind>) {
        self.latched = kind;
    }

    pub fn kind(&self) -> Option<SpiroKind> {
        self.latched
    }

    /// The points placed so far.
    pub fn points(&self) -> &[SpiroPoint] {
        &self.points
    }

    fn kind_for(&self, m: Modifiers) -> SpiroKind {
        if let Some(kind) = self.latched {
            return kind;
        }
        match (m.cmd, m.shift, m.alt) {
            (true, true, _) => SpiroKind::Right,
            (true, false, _) => SpiroKind::Left,
            (false, _, true) => SpiroKind::Corner,
            (false, true, false) => SpiroKind::G2,
            (false, false, false) => SpiroKind::G4,
        }
    }

    fn refresh(&mut self) {
        self.curve = Spline::solve(&self.points, false).ok().map(|s| s.to_bez(TOLERANCE));
    }

    /// Adds the contour and starts a new one. A contour without a curve through it is kept so
    /// its points can be moved, unless `force`; too few points add nothing.
    fn finish(&mut self, closed: bool, force: bool) -> Output {
        self.held = None;
        let enough = if closed { self.points.len() >= 3 } else { self.points.len() >= 2 };
        if !enough {
            self.points.clear();
            self.curve = None;
            return Output::handled();
        }
        let invocation = Spline::solve(&self.points, closed).ok().and_then(|s| add_path(&self.target, &s.to_bez(TOLERANCE), closed));
        if invocation.is_none() && !force {
            return Output::handled();
        }
        self.points.clear();
        self.curve = None;
        match invocation {
            Some(invocation) => Output::with(Action::Run { invocation }),
            None => Output::handled(),
        }
    }
}

/// `path.addPath` for `path`, or `None` when it has too few nodes.
fn add_path(target: &Map<String, Value>, path: &BezPath, closed: bool) -> Option<Invocation> {
    let nodes = nodes_of(path, closed);
    if nodes.iter().filter(|n| n.on_curve()).count() < 2 {
        return None;
    }
    let list: Vec<Value> = nodes.iter().map(node_value).collect();
    Some(Invocation::new("path.addPath", params(target, json!({"closed": closed, "nodes": list}))))
}

impl Tool for SpiroTool {
    fn id(&self) -> &'static str {
        "spiro"
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        if !finite(ev.pos) {
            return Output::default();
        }
        self.target = cx.scene.target();
        match ev.kind {
            PointerKind::Down => {
                if cx.scene.locked {
                    return Output::handled();
                }
                let tolerance = cx.view.units(cx.settings.hit_tolerance);
                let hit = self.points.iter().position(|p| dist(p.at, ev.pos) <= tolerance);
                let (index, close) = match hit {
                    Some(0) if self.points.len() >= 3 => (0, true),
                    Some(i) => (i, false),
                    None if self.points.len() < MAX_SPIRO_POINTS => {
                        self.points.push(SpiroPoint { at: ev.pos, kind: self.kind_for(ev.mods) });
                        self.refresh();
                        (self.points.len() - 1, false)
                    }
                    None => return Output::handled(),
                };
                self.held = Some(Held { index, close, start: ev.pos, moved: false });
                Output::handled()
            }
            PointerKind::Drag => {
                let Some(mut held) = self.held else { return Output::default() };
                if !held.moved && dist(cx.view.to_screen(held.start), cx.view.to_screen(ev.pos)) < cx.settings.drag_threshold {
                    return Output::handled();
                }
                held.moved = true;
                self.held = Some(held);
                if let Some(p) = self.points.get_mut(held.index) {
                    p.at = ev.pos;
                }
                self.refresh();
                Output::handled()
            }
            PointerKind::Up => match self.held.take() {
                Some(Held { close: true, moved: false, .. }) => self.finish(true, false),
                Some(_) => Output::handled(),
                None => Output::default(),
            },
            PointerKind::DoubleClick if !self.points.is_empty() => self.finish(false, false),
            PointerKind::Move | PointerKind::DoubleClick => Output::default(),
        }
    }

    fn key(&mut self, _cx: &Context<'_>, ev: &KeyEvent) -> Output {
        if !ev.pressed || self.points.is_empty() {
            return Output::default();
        }
        match ev.key {
            Key::Escape | Key::Return => self.finish(false, true),
            Key::Backspace | Key::Delete if self.held.is_none() => {
                self.points.pop();
                self.refresh();
                Output::handled()
            }
            _ => Output::default(),
        }
    }

    /// Leaving the tool adds the contour drawn so far.
    fn cancel(&mut self) -> Output {
        let mut out = self.finish(false, true);
        out.handled = false;
        out
    }

    fn busy(&self) -> bool {
        self.held.is_some()
    }

    fn overlays(&self) -> Vec<Overlay> {
        let mut out = match &self.curve {
            Some(curve) => preview_lines(curve),
            // No curve yet (or none through these points): straight lines between the points.
            None => self
                .points
                .windows(2)
                .filter_map(|w| match w {
                    [a, b] => Some(Overlay::Line { kind: OverlayKind::DrawPreview, from: a.at, to: b.at }),
                    _ => None,
                })
                .collect(),
        };
        for (i, p) in self.points.iter().enumerate() {
            let held = self.held.is_some_and(|h| h.index == i);
            out.push(Overlay::Marker { kind: if held { OverlayKind::Hover } else { OverlayKind::DrawHandle }, at: p.at });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use fontcraft_model::Layer;

    use super::*;
    use crate::scene::Scene;
    use crate::{Selection, ToolSettings, View};

    fn event(tool: &mut SpiroTool, layer: &Layer, kind: PointerKind, x: f64, y: f64, mods: Modifiers) -> Output {
        let scene = Scene::new("A", "m01", layer);
        let selection = Selection::new();
        let cx = Context { scene: &scene, view: View::default(), selection: &selection, settings: ToolSettings::default() };
        tool.pointer(&cx, &PointerEvent::new(kind, x, y, mods))
    }

    fn click(tool: &mut SpiroTool, layer: &Layer, x: f64, y: f64, mods: Modifiers) -> Output {
        event(tool, layer, PointerKind::Down, x, y, mods);
        event(tool, layer, PointerKind::Up, x, y, mods)
    }

    fn key(tool: &mut SpiroTool, layer: &Layer, k: Key) -> Output {
        let scene = Scene::new("A", "m01", layer);
        let selection = Selection::new();
        let cx = Context { scene: &scene, view: View::default(), selection: &selection, settings: ToolSettings::default() };
        tool.key(&cx, &KeyEvent::press(k, Modifiers::NONE))
    }

    fn added(out: &Output) -> Value {
        match out.actions.as_slice() {
            [Action::Run { invocation }] if invocation.id == "path.addPath" => invocation.params.clone(),
            other => panic!("expected path.addPath, got {other:?}"),
        }
    }

    #[test]
    fn four_clicks_give_a_closed_g2_curve_as_cubics() {
        let layer = Layer::new("m01", 600.0);
        let mut tool = SpiroTool::default();
        for (x, y) in [(100.0, 0.0), (0.0, 100.0), (-100.0, 0.0), (0.0, -100.0)] {
            assert!(click(&mut tool, &layer, x, y, Modifiers::SHIFT).actions.is_empty());
        }
        assert!(tool.points().iter().all(|p| p.kind == SpiroKind::G2));
        let params = added(&click(&mut tool, &layer, 100.0, 0.0, Modifiers::NONE));
        assert!(tool.points().is_empty());
        assert_eq!(params["closed"], true);
        let nodes = params["nodes"].as_array().unwrap();
        let on: Vec<&Value> = nodes.iter().filter(|n| n["type"] != "offcurve").collect();
        // Cubic segments only, every on-curve node smooth, all of them on the circle.
        assert!(on.len() >= 4 && on.iter().all(|n| n["type"] == "curve" && n["smooth"] == true), "{nodes:?}");
        for n in nodes.iter().filter(|n| n["type"] != "offcurve") {
            let r = n["x"].as_f64().unwrap().hypot(n["y"].as_f64().unwrap());
            assert!((r - 100.0).abs() < 1e-6, "{n}");
        }
        for (x, y) in [(100.0, 0.0), (0.0, 100.0), (-100.0, 0.0), (0.0, -100.0)] {
            assert!(on.iter().any(|n| (n["x"].as_f64().unwrap() - x).abs() < 1e-9 && (n["y"].as_f64().unwrap() - y).abs() < 1e-9));
        }
    }

    #[test]
    fn dragging_a_point_recomputes_the_curve_and_escape_adds_it_open() {
        let layer = Layer::new("m01", 600.0);
        let mut tool = SpiroTool::default();
        click(&mut tool, &layer, 0.0, 0.0, Modifiers::NONE);
        click(&mut tool, &layer, 100.0, 50.0, Modifiers::NONE);
        click(&mut tool, &layer, 200.0, 0.0, Modifiers::NONE);
        let before = tool.overlays();
        event(&mut tool, &layer, PointerKind::Down, 100.0, 50.0, Modifiers::NONE);
        event(&mut tool, &layer, PointerKind::Drag, 100.0, 120.0, Modifiers::NONE);
        event(&mut tool, &layer, PointerKind::Up, 100.0, 120.0, Modifiers::NONE);
        assert_eq!(tool.points().len(), 3);
        assert_eq!(tool.points()[1].at, Point::new(100.0, 120.0));
        assert_ne!(tool.overlays(), before);
        let params = added(&key(&mut tool, &layer, Key::Escape));
        assert_eq!(params["closed"], false);
        let nodes = params["nodes"].as_array().unwrap();
        let x = |n: Option<&Value>| n.and_then(|n| n["x"].as_f64()).unwrap();
        assert!(x(nodes.first()).abs() < 1e-9 && (x(nodes.last()) - 200.0).abs() < 1e-9);
    }

    #[test]
    fn modifier_keys_and_the_latch_choose_the_point_kind() {
        let layer = Layer::new("m01", 600.0);
        let mut tool = SpiroTool::default();
        let mods = [Modifiers::NONE, Modifiers::SHIFT, Modifiers::ALT, Modifiers::CMD, Modifiers::CMD.with_shift()];
        for (i, m) in mods.into_iter().enumerate() {
            click(&mut tool, &layer, 100.0 * i as f64, 0.0, m);
        }
        let kinds: Vec<SpiroKind> = tool.points().iter().map(|p| p.kind).collect();
        assert_eq!(kinds, [SpiroKind::G4, SpiroKind::G2, SpiroKind::Corner, SpiroKind::Left, SpiroKind::Right]);
        tool.set_kind(Some(SpiroKind::Corner));
        click(&mut tool, &layer, 600.0, 50.0, Modifiers::SHIFT);
        assert_eq!(tool.points().last().unwrap().kind, SpiroKind::Corner);
    }

    #[test]
    fn backspace_leaving_the_tool_and_bad_input() {
        let layer = Layer::new("m01", 600.0);
        let mut tool = SpiroTool::default();
        click(&mut tool, &layer, 0.0, 0.0, Modifiers::NONE);
        click(&mut tool, &layer, 100.0, 0.0, Modifiers::NONE);
        click(&mut tool, &layer, 200.0, 50.0, Modifiers::NONE);
        assert!(key(&mut tool, &layer, Key::Backspace).handled);
        assert_eq!(tool.points().len(), 2);
        for kind in [PointerKind::Down, PointerKind::Drag, PointerKind::Up] {
            assert!(event(&mut tool, &layer, kind, f64::INFINITY, 0.0, Modifiers::NONE).actions.is_empty());
        }
        // Switching tools adds the open contour: two points give one segment.
        let params = added(&tool.cancel());
        assert_eq!(params["nodes"].as_array().unwrap().len(), 2);
        assert!(tool.points().is_empty() && tool.cancel().actions.is_empty());
    }
}
