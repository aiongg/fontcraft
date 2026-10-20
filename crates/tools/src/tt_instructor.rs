//! The TrueType Instructor tool (shortcut I): plain keys add instructions, the pointer selects
//! nodes and handles of the edited layer without moving them.
//!
//! Rows: `trueTypeInstructor`, `hinting.tt.tool.plainKeysAreInstructions`, `hint.tt.toggleDirection`,
//! `hint.tt.increasePixelSize`, `hint.tt.decreasePixelSize`, `hinting.tt.instructions.addFromSelection`.
//!
//! | Key (no Cmd, Ctrl or Alt) | Runs |
//! |---|---|
//! | A | `hint.tt.snapPoint` on the selected points |
//! | S | `hint.tt.addStemHint` on the selected points, paired by [`stem_pairs`] |
//! | F | `hint.tt.shiftPoints` on the selected points |
//! | G | `hint.tt.interpolate` on the selected points |
//! | E | `hint.tt.addDelta` on the selected points |
//! | X | `hint.tt.toggleDirection` |
//! | Period, Plus, Equals | `hint.tt.increasePixelSize` |
//! | Comma, Minus | `hint.tt.decreasePixelSize` |
//!
//! Every other character key is taken and does nothing, so letters never switch tools while the
//! Instructor is active (the toolbar does). The arrow keys, Return, Delete and Backspace are taken
//! too: the outline is there to pick points, not to edit (with instructions selected, the shell
//! takes Delete and Backspace first and deletes them). Tab, Escape (outside a gesture), Space
//! and keys with Cmd, Ctrl or Alt go on to the shell, so Tab steps through the nodes and Autohint
//! (Cmd+Ctrl+Alt+Shift+I) runs as a menu shortcut.
//!
//! The instruction commands get `glyph`, `layer` (the edited layer), `nodes` as `[path, node]`
//! with `path` counting paths only, and `direction` from [`ToolSettings::tt_direction`]
//! (crate::ToolSettings), which the shell fills from the Instructor's view state. With nothing
//! selected, or while the background is edited, the keys add nothing.
//!
//! Pointer: a click on a node or handle selects it (Shift toggles it); a drag from anywhere else
//! draws a rectangle that selects the nodes and handles inside it (Shift toggles them into the
//! selection); a click on nothing clears the selection. Instruction badges belong to the canvas
//! (`canvas::tt_badges` in the egui shell), which takes clicks and drags on them before the tool
//! sees them.

use fontcraft_geom::Point;
use fontcraft_model::{LayerSide, Shape};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::event::{Key, KeyEvent, PointerEvent, PointerKind};
use crate::geometry::{dist, rect_corners};
use crate::hit::{Hit, hit_test};
use crate::output::{Action, Invocation, Output, Overlay, OverlayKind};
use crate::scene::Scene;
use crate::selection::{Item, Selection, in_region};
use crate::tool::{Context, Tool};

/// The tool's id, as `tool.select` names it.
pub const ID: &str = "trueTypeInstructor";

/// The Instructor's hinting direction (`hint.tt.toggleDirection`): vertical instructions move
/// points in y (for horizontal stems), horizontal ones in x.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TtDirection {
    #[default]
    Vertical,
    Horizontal,
}

impl TtDirection {
    /// The name the hinting commands' `direction` parameter takes.
    pub fn as_str(self) -> &'static str {
        match self {
            TtDirection::Vertical => "vertical",
            TtDirection::Horizontal => "horizontal",
        }
    }

    /// A direction from its parameter name.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "vertical" => Some(TtDirection::Vertical),
            "horizontal" => Some(TtDirection::Horizontal),
            _ => None,
        }
    }

    pub fn other(self) -> Self {
        match self {
            TtDirection::Vertical => TtDirection::Horizontal,
            TtDirection::Horizontal => TtDirection::Vertical,
        }
    }
}

/// The command a plain key runs in the Instructor, or `None` when the key adds nothing (an
/// unbound character, or an instruction key with no point to act on). The key is taken either
/// way; see the module doc.
pub fn key_invocation(cx: &Context<'_>, c: char) -> Option<Invocation> {
    let id = match c.to_ascii_lowercase() {
        'a' => "hint.tt.snapPoint",
        's' => "hint.tt.addStemHint",
        'f' => "hint.tt.shiftPoints",
        'g' => "hint.tt.interpolate",
        'e' => "hint.tt.addDelta",
        'x' => return Some(Invocation::new("hint.tt.toggleDirection", json!({}))),
        '.' | '+' | '=' => return Some(Invocation::new("hint.tt.increasePixelSize", json!({}))),
        ',' | '-' => return Some(Invocation::new("hint.tt.decreasePixelSize", json!({}))),
        _ => return None,
    };
    let scene = cx.scene;
    if scene.side != LayerSide::Foreground {
        return None;
    }
    let direction = cx.settings.tt_direction;
    let mut points = selected_points(scene, cx.selection);
    if points.is_empty() {
        return None;
    }
    if id == "hint.tt.addStemHint" {
        points = stem_pairs(points, direction);
    }
    let nodes: Vec<[usize; 2]> = points.iter().map(|p| [p.path, p.node]).collect();
    let params = json!({
        "glyph": scene.glyph,
        "layer": scene.layer_id,
        "nodes": nodes,
        "direction": direction.as_str(),
    });
    Some(Invocation::new(id, params))
}

/// A selected point as the hinting commands name it, with its position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HintPoint {
    /// Index among the layer's paths, components skipped.
    pub path: usize,
    pub node: usize,
    pub at: Point,
}

/// The selected nodes and handles of the scene's paths, in selection order, as the hinting
/// commands address them.
pub fn selected_points(scene: &Scene, selection: &Selection) -> Vec<HintPoint> {
    let mut path_index = Vec::with_capacity(scene.layer.shapes.len());
    let mut paths = 0usize;
    for shape in &scene.layer.shapes {
        if matches!(shape, Shape::Path(_)) {
            path_index.push(Some(paths));
            paths += 1;
        } else {
            path_index.push(None);
        }
    }
    selection
        .points
        .iter()
        .filter_map(|&(s, n)| {
            let path = (*path_index.get(s)?)?;
            let Some(Shape::Path(p)) = scene.layer.shapes.get(s) else { return None };
            let node = p.nodes.get(n)?;
            Some(HintPoint { path, node: n, at: Point::new(node.x, node.y) })
        })
        .collect()
}

/// The points in the order `hint.tt.addStemHint` pairs them: two by two along the stems. For the
/// vertical direction a stem is measured in y, so its two points sit one above the other: the
/// points are sorted by x, then y, and each pair runs from the lower point (the origin) to the
/// upper one. The horizontal direction sorts by y, then x, and runs from left to right.
pub fn stem_pairs(mut points: Vec<HintPoint>, direction: TtDirection) -> Vec<HintPoint> {
    let key = |p: &HintPoint| match direction {
        TtDirection::Vertical => (p.at.x, p.at.y),
        TtDirection::Horizontal => (p.at.y, p.at.x),
    };
    points.sort_by(|a, b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal));
    points
}

#[derive(Clone, Debug, Default)]
enum State {
    #[default]
    Idle,
    /// The button went down at `at` (font units); `on` is the point it went down on.
    Pressed { at: Point, on: Option<(usize, usize)>, shift: bool, before: Selection },
    /// A selection rectangle from `start` to `current`.
    Marquee { start: Point, current: Point, shift: bool, before: Selection },
}

/// The TrueType Instructor.
#[derive(Clone, Debug, Default)]
pub struct TtInstructorTool {
    state: State,
}

impl TtInstructorTool {
    fn down(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        let before = cx.selection.clone();
        let shift = ev.mods.shift;
        let on = match hit_test(cx.scene, &cx.view, ev.pos, cx.settings.hit_tolerance) {
            Some(h) => match h.hit {
                Hit::Node { shape, node } | Hit::Handle { shape, node } => Some((shape, node)),
                _ => None,
            },
            None => None,
        };
        self.state = State::Pressed { at: ev.pos, on, shift, before: before.clone() };
        let Some((s, n)) = on else { return Output::handled() };
        let item = Item::Point(s, n);
        let selection = if shift {
            let mut sel = before;
            sel.toggle(item);
            sel
        } else if before.contains(&item) {
            // A press on a selected point keeps the others until the button comes up, as with the
            // Select tool.
            return Output::handled();
        } else {
            Selection::from_items([item])
        };
        Output::with(Action::Select { selection })
    }

    fn drag(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        match std::mem::take(&mut self.state) {
            State::Pressed { at, on, shift, before } => {
                let far = dist(cx.view.to_screen(at), cx.view.to_screen(ev.pos)) >= cx.settings.drag_threshold;
                if !far {
                    self.state = State::Pressed { at, on, shift, before };
                    return Output::handled();
                }
                self.state = State::Marquee { start: at, current: ev.pos, shift, before };
                self.marquee_selection(cx)
            }
            State::Marquee { start, shift, before, .. } => {
                self.state = State::Marquee { start, current: ev.pos, shift, before };
                self.marquee_selection(cx)
            }
            State::Idle => Output::default(),
        }
    }

    fn up(&mut self, cx: &Context<'_>) -> Output {
        match std::mem::take(&mut self.state) {
            State::Pressed { on: None, shift: false, .. } => Output::with(Action::Select { selection: Selection::new() }),
            State::Pressed { on: Some((s, n)), shift: false, before, .. } if before.contains(&Item::Point(s, n)) => {
                Output::with(Action::Select { selection: Selection::from_items([Item::Point(s, n)]) })
            }
            State::Pressed { .. } => Output::handled(),
            State::Marquee { start, current, shift, before } => {
                self.state = State::Marquee { start, current, shift, before };
                let out = self.marquee_selection(cx);
                self.state = State::Idle;
                out
            }
            State::Idle => Output::default(),
        }
    }

    fn marquee_selection(&self, cx: &Context<'_>) -> Output {
        let State::Marquee { start, current, shift, before } = &self.state else { return Output::default() };
        let corners = rect_corners(*start, 0.0, current.x - start.x, current.y - start.y);
        let region = in_region(cx.scene, &corners, false, false);
        let mut selection = if *shift { before.clone() } else { Selection::new() };
        for &(s, n) in &region.points {
            if *shift {
                selection.toggle(Item::Point(s, n));
            } else {
                selection.insert(Item::Point(s, n));
            }
        }
        Output::with(Action::Select { selection })
    }
}

impl Tool for TtInstructorTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn pointer(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        match ev.kind {
            PointerKind::Down => self.down(cx, ev),
            PointerKind::Drag => self.drag(cx, ev),
            PointerKind::Up => self.up(cx),
            PointerKind::Move | PointerKind::DoubleClick => Output::default(),
        }
    }

    fn key(&mut self, cx: &Context<'_>, ev: &KeyEvent) -> Output {
        if !ev.pressed {
            return Output::default();
        }
        if ev.key == Key::Escape {
            if !self.busy() {
                return Output::default();
            }
            let mut out = self.cancel();
            out.handled = true;
            return out;
        }
        let m = ev.mods;
        if m.cmd || m.ctrl || m.alt {
            return Output::default();
        }
        match ev.key {
            Key::Char(c) => {
                let mut out = Output::handled();
                if let Some(invocation) = key_invocation(cx, c) {
                    out.push(Action::Run { invocation });
                }
                out
            }
            Key::Left | Key::Right | Key::Up | Key::Down | Key::Return | Key::Delete | Key::Backspace => Output::handled(),
            Key::Tab | Key::Escape | Key::Space | Key::Home | Key::End => Output::default(),
        }
    }

    fn cancel(&mut self) -> Output {
        match std::mem::take(&mut self.state) {
            State::Marquee { before, .. } | State::Pressed { before, .. } => Output::with(Action::Select { selection: before }),
            State::Idle => Output::default(),
        }
    }

    fn busy(&self) -> bool {
        !matches!(self.state, State::Idle)
    }

    fn overlays(&self) -> Vec<Overlay> {
        match &self.state {
            State::Marquee { start, current, .. } => {
                let corners = rect_corners(*start, 0.0, current.x - start.x, current.y - start.y);
                vec![Overlay::Polygon { kind: OverlayKind::Marquee, points: corners.to_vec() }]
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use fontcraft_model::{Component, Layer, Path};

    use serde_json::Value;

    use super::*;
    use crate::event::Modifiers;
    use crate::tool::ToolSettings;
    use crate::toolbox::Toolbox;
    use crate::view::View;

    const NONE: Modifiers = Modifiers::NONE;

    /// Shape 0: a component; shape 1: a closed square (0,0) (100,0) (100,100) (0,100); shape 2: a
    /// closed square (300,0) (400,0) (400,100) (300,100).
    fn layer() -> Layer {
        let mut layer = Layer::new("m01", 600.0);
        layer.shapes.push(Shape::Component(Component::new("B", 0.0, 0.0)));
        layer.shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        layer.shapes.push(Shape::Path(Path::rect(300.0, 0.0, 400.0, 100.0)));
        layer
    }

    fn run<R>(layer: &Layer, selection: &Selection, direction: TtDirection, f: impl FnOnce(&Context<'_>) -> R) -> R {
        let scene = Scene::new("A", "m01", layer);
        let settings = ToolSettings { tt_direction: direction, ..ToolSettings::default() };
        let cx = Context { scene: &scene, view: View::default(), selection, settings };
        f(&cx)
    }

    fn select(points: &[(usize, usize)]) -> Selection {
        Selection::from_items(points.iter().map(|&(s, n)| Item::Point(s, n)))
    }

    fn press(c: char) -> KeyEvent {
        KeyEvent::press(Key::Char(c), NONE)
    }

    fn runs(out: &Output) -> Vec<(String, Value)> {
        out.actions
            .iter()
            .filter_map(|a| match a {
                Action::Run { invocation } => Some((invocation.id.clone(), invocation.params.clone())),
                _ => None,
            })
            .collect()
    }

    /// The `[path, node]` pairs an invocation names.
    fn invocation_nodes(params: &Value) -> Vec<[usize; 2]> {
        params
            .get("nodes")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| {
                        let pair = v.as_array()?;
                        Some([usize::try_from(pair.first()?.as_u64()?).ok()?, usize::try_from(pair.get(1)?.as_u64()?).ok()?])
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn each_instruction_key_runs_its_command_on_the_selected_points_in_the_current_direction() {
        let layer = layer();
        let sel = select(&[(1, 0), (2, 3)]);
        for (c, id) in [
            ('a', "hint.tt.snapPoint"),
            ('s', "hint.tt.addStemHint"),
            ('f', "hint.tt.shiftPoints"),
            ('g', "hint.tt.interpolate"),
            ('e', "hint.tt.addDelta"),
        ] {
            for direction in [TtDirection::Vertical, TtDirection::Horizontal] {
                let out = run(&layer, &sel, direction, |cx| TtInstructorTool::default().key(cx, &press(c)));
                assert!(out.handled);
                let r = runs(&out);
                assert_eq!(r.len(), 1, "{c}");
                let (got, params) = &r[0];
                assert_eq!(got, id);
                assert_eq!(params["glyph"], "A");
                assert_eq!(params["layer"], "m01");
                assert_eq!(params["direction"], direction.as_str());
                // Paths are counted without the component: shape 1 is path 0, shape 2 path 1.
                let mut nodes = invocation_nodes(params);
                nodes.sort_unstable();
                assert_eq!(nodes, vec![[0, 0], [1, 3]], "{c}");
            }
        }
    }

    #[test]
    fn view_keys_run_the_view_commands_and_other_letters_are_taken_without_switching_tools() {
        let layer = layer();
        let sel = Selection::new();
        for (c, id) in [
            ('x', "hint.tt.toggleDirection"),
            ('.', "hint.tt.increasePixelSize"),
            ('+', "hint.tt.increasePixelSize"),
            (',', "hint.tt.decreasePixelSize"),
            ('-', "hint.tt.decreasePixelSize"),
        ] {
            let out = run(&layer, &sel, TtDirection::Vertical, |cx| TtInstructorTool::default().key(cx, &press(c)));
            assert_eq!(runs(&out).first().map(|r| r.0.as_str()), Some(id), "{c}");
        }
        // The window names the punctuation keys; they reach the tool as their characters.
        for (name, c) in [("Period", '.'), ("Comma", ','), ("Minus", '-'), ("Plus", '+'), ("Equals", '=')] {
            assert_eq!(Key::parse(name), Some(Key::Char(c)));
        }
        // Through the toolbox: V, H, T and A would switch tools elsewhere; here they are taken.
        let mut toolbox = Toolbox::new();
        assert!(toolbox.select(ID).is_ok());
        for c in ['v', 'h', 't', 'l', 'a', 'z', 'q'] {
            for mods in [NONE, Modifiers::SHIFT] {
                let out = run(&layer, &sel, TtDirection::Vertical, |cx| toolbox.key(cx, &KeyEvent::press(Key::Char(c), mods)));
                assert!(out.handled, "{c}");
                assert!(runs(&out).iter().all(|r| r.0 != "tool.select"), "{c} switched tools");
            }
        }
        assert_eq!(toolbox.active_id(), ID);
        // Keys with Cmd go on to the menu shortcuts; arrows and Delete do not edit the outline.
        let out = run(&layer, &sel, TtDirection::Vertical, |cx| toolbox.key(cx, &KeyEvent::press(Key::Char('a'), Modifiers::CMD)));
        assert!(runs(&out).is_empty());
        let one = select(&[(1, 0)]);
        for key in [Key::Left, Key::Up, Key::Return, Key::Delete, Key::Backspace] {
            let out = run(&layer, &one, TtDirection::Vertical, |cx| toolbox.key(cx, &KeyEvent::press(key, NONE)));
            assert!(out.handled && out.actions.is_empty(), "{key:?}");
        }
    }

    #[test]
    fn instruction_keys_add_nothing_without_a_selection_or_in_the_background() {
        let layer = layer();
        let out = run(&layer, &Selection::new(), TtDirection::Vertical, |cx| TtInstructorTool::default().key(cx, &press('a')));
        assert!(out.handled && out.actions.is_empty());
        let sel = select(&[(1, 0)]);
        let mut scene = Scene::new("A", "m01", &layer);
        scene.side = LayerSide::Background;
        let cx = Context { scene: &scene, view: View::default(), selection: &sel, settings: ToolSettings::default() };
        assert!(TtInstructorTool::default().key(&cx, &press('a')).actions.is_empty());
    }

    #[test]
    fn stem_pairs_run_across_the_stem_from_the_lower_or_left_point() {
        let p = |path, node, x, y| HintPoint { path, node, at: Point::new(x, y) };
        let points = vec![p(0, 2, 100.0, 100.0), p(1, 0, 300.0, 0.0), p(0, 1, 100.0, 0.0), p(1, 3, 300.0, 100.0)];
        let v: Vec<(usize, usize)> = stem_pairs(points.clone(), TtDirection::Vertical).iter().map(|p| (p.path, p.node)).collect();
        assert_eq!(v, vec![(0, 1), (0, 2), (1, 0), (1, 3)]);
        let h: Vec<(usize, usize)> = stem_pairs(points, TtDirection::Horizontal).iter().map(|p| (p.path, p.node)).collect();
        assert_eq!(h, vec![(0, 1), (1, 0), (0, 2), (1, 3)]);
    }

    #[test]
    fn clicks_and_rectangles_select_points_without_moving_them() {
        let layer = layer();
        let mut tool = TtInstructorTool::default();
        let mut sel = Selection::new();
        let apply = |out: Output, sel: &mut Selection| {
            for a in out.actions {
                match a {
                    Action::Select { selection } => *sel = selection,
                    other => panic!("the Instructor asked for {other:?}"),
                }
            }
        };
        let ev = |kind, x, y, mods| PointerEvent::new(kind, x, y, mods);
        // A click on a node selects it; Shift+click adds another.
        for (x, y, mods) in [(0.0, 0.0, NONE), (100.0, 100.0, Modifiers::SHIFT)] {
            for kind in [PointerKind::Down, PointerKind::Up] {
                let out = run(&layer, &sel.clone(), TtDirection::Vertical, |cx| tool.pointer(cx, &ev(kind, x, y, mods)));
                apply(out, &mut sel);
            }
        }
        assert_eq!(sel, select(&[(1, 0), (1, 2)]));
        // A drag over the second square selects its four nodes; nothing moves.
        for (kind, x, y) in [(PointerKind::Down, 250.0, -50.0), (PointerKind::Drag, 450.0, 150.0)] {
            let out = run(&layer, &sel.clone(), TtDirection::Vertical, |cx| tool.pointer(cx, &ev(kind, x, y, NONE)));
            apply(out, &mut sel);
        }
        assert_eq!(tool.overlays().len(), 1, "the rectangle is drawn");
        let out = run(&layer, &sel.clone(), TtDirection::Vertical, |cx| tool.pointer(cx, &ev(PointerKind::Up, 450.0, 150.0, NONE)));
        apply(out, &mut sel);
        assert_eq!(sel, select(&[(2, 0), (2, 1), (2, 2), (2, 3)]));
        assert!(!tool.busy());
        // A click on nothing clears the selection.
        for kind in [PointerKind::Down, PointerKind::Up] {
            let out = run(&layer, &sel.clone(), TtDirection::Vertical, |cx| tool.pointer(cx, &ev(kind, 200.0, 300.0, NONE)));
            apply(out, &mut sel);
        }
        assert!(sel.is_empty());
    }

    #[test]
    fn escape_during_a_rectangle_restores_the_selection() {
        let layer = layer();
        let before = select(&[(1, 0)]);
        let mut tool = TtInstructorTool::default();
        run(&layer, &before, TtDirection::Vertical, |cx| {
            tool.pointer(cx, &PointerEvent::new(PointerKind::Down, 250.0, -50.0, NONE));
            tool.pointer(cx, &PointerEvent::new(PointerKind::Drag, 450.0, 150.0, NONE));
        });
        let out = run(&layer, &before, TtDirection::Vertical, |cx| tool.key(cx, &KeyEvent::press(Key::Escape, NONE)));
        assert!(out.handled);
        assert_eq!(out.actions, vec![Action::Select { selection: before }]);
        assert!(!tool.busy());
    }

    #[test]
    fn the_tool_is_listed_with_its_letter() {
        let info = crate::TOOLS.iter().find(|t| t.id == ID);
        assert_eq!(info.map(|t| (t.label, t.shortcut)), Some(("TrueType Instructor", "I")));
        assert_eq!(crate::make_tool(ID).map(|t| t.id()), Some(ID));
    }
}
