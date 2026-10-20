//! The Select tool's PostScript hint gestures while View › Show Hints is on
//! (`hinting.ps.manual.select`, `hinting.ps.manual.dragMarkers`, `hinting.ps.manual.linkOnDrop`).
//!
//! | Gesture | Result |
//! |---|---|
//! | click a hint's badge or one of its markers | select that hint alone |
//! | `Shift`+click | toggle the hint in the selection |
//! | drag the circle (the origin edge) | `hint.ps.setHintValues` with the new origin, in one interaction; a stem's other edge stays where it is |
//! | drag the triangle (the other edge) | `hint.ps.setHintValues` with the new size |
//! | drop a marker on a node | `hint.ps.attachHintToNode`: that end follows the node |
//! | `Tab`, `Shift+Tab` with hints selected | `hint.ps.selectNextHint`, `hint.ps.selectPreviousHint` |
//! | `Delete` with hints selected | `hint.ps.deleteHints` |
//! | arrow keys with hints selected | `hint.ps.setHintValues` for each selected hint across the arrow (Up and Down move horizontal hints, Left and Right vertical ones) by the font's increments (plain, `Shift`, `Cmd`); the whole hint moves, so an end linked to a node becomes a number; the hints stay selected |
//!
//! Hints are addressed by their position in the layer's PostScript hint list, as
//! `hint.ps.listHints` returns it; nodes as `[path, node]` with `path` counting paths only. The
//! shell reads the list into [`HintMarks`] and puts it in the [`Scene`]; the canvas draws the
//! hints where [`HintMarks`] places them, so what is drawn is what is hit.
//!
//! Where things sit, in screen points: the markers of horizontal hints stand in a column
//! [`HintLayout::lane`] left of the glyph's origin, at the hint's edges; their badges stand left
//! of that column, beside the middle of the hint. The markers of vertical hints stand in a row
//! the same distance below the master's descender, their badges below that row.

use fontcraft_geom::Point;
use fontcraft_model::Shape;
use serde_json::{Map, Value, json};

use crate::event::{Key, KeyEvent, PointerEvent};
use crate::geometry::{dist, finite};
use crate::hit::{Hit, hit_test};
use crate::output::{Action, Invocation, Output, Overlay, OverlayKind};
use crate::scene::Scene;
use crate::selection::{Item, Selection};
use crate::tool::Context;

/// The label of a marker drag's undo step.
pub const DRAG_LABEL: &str = "Move Hint";

/// What a hint marks: a stem, or a ghost that marks a top or a bottom edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintKind {
    Stem,
    GhostTop,
    GhostBottom,
}

impl HintKind {
    /// The kind as `hint.ps.listHints` names it.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "stem" => Some(Self::Stem),
            "ghostTop" => Some(Self::GhostTop),
            "ghostBottom" => Some(Self::GhostBottom),
            _ => None,
        }
    }

    pub fn is_ghost(self) -> bool {
        self != Self::Stem
    }
}

/// One PostScript hint of the edited layer, as `hint.ps.listHints` reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct HintMark {
    /// Its position in the layer's hint list.
    pub index: usize,
    /// A horizontal hint's edges are y values, a vertical one's x values.
    pub horizontal: bool,
    pub kind: HintKind,
    /// The origin edge; `None` when it cannot be placed (linked to a node the layer lacks).
    pub origin: Option<f64>,
    /// The signed distance from the origin edge to the other edge; `None` for a ghost.
    pub size: Option<f64>,
    /// The node each end is linked to, as `[path, node]`.
    pub origin_node: Option<(usize, usize)>,
    pub target_node: Option<(usize, usize)>,
    /// What `hint.ps.listHints` finds doubtful about the hint.
    pub warnings: Vec<String>,
}

impl HintMark {
    /// The hint from one entry of `hint.ps.listHints`'s `hints`; `None` when the entry is not one.
    pub fn from_value(v: &Value) -> Option<Self> {
        let index = usize::try_from(v.get("index")?.as_u64()?).ok()?;
        let horizontal = match v.get("direction")?.as_str()? {
            "horizontal" => true,
            "vertical" => false,
            _ => return None,
        };
        let kind = HintKind::parse(v.get("kind")?.as_str()?)?;
        let number = |key: &str| v.get(key).and_then(Value::as_f64).filter(|n| n.is_finite());
        let node = |end: &str| {
            let pair = v.get("nodes")?.get(end)?.as_array()?;
            let [path, node] = pair.as_slice() else { return None };
            Some((usize::try_from(path.as_u64()?).ok()?, usize::try_from(node.as_u64()?).ok()?))
        };
        let warnings = v
            .get("warnings")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        Some(Self {
            index,
            horizontal,
            kind,
            origin: number("origin"),
            size: number("size").filter(|_| !kind.is_ghost()),
            origin_node: node("origin"),
            target_node: node("target"),
            warnings,
        })
    }

    /// The other edge of a stem.
    pub fn target(&self) -> Option<f64> {
        let t = self.origin? + self.size?;
        t.is_finite().then_some(t)
    }

    /// The position of `end`.
    pub fn edge(&self, end: End) -> Option<f64> {
        match end {
            End::Origin => self.origin,
            End::Target => self.target(),
        }
    }
}

/// Sizes of the hint marks, in screen points; the shell takes them from its design tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HintLayout {
    /// Distance from the glyph's origin (horizontal hints) or descender (vertical hints) to the
    /// line the markers stand on.
    pub lane: f64,
    /// A badge's width and height.
    pub badge: (f64, f64),
    /// A marker's radius.
    pub marker: f64,
    /// Space between a marker and its badge.
    pub gap: f64,
}

impl Default for HintLayout {
    fn default() -> Self {
        Self { lane: 18.0, badge: (64.0, 16.0), marker: 5.0, gap: 6.0 }
    }
}

/// Which part of a hint is under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Badge,
    Marker(End),
}

/// One end of a hint: the origin (the circle) or the other edge (the triangle).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    Origin,
    Target,
}

impl End {
    /// The end as `hint.ps.attachHintToNode` names it.
    pub fn as_str(self) -> &'static str {
        match self {
            End::Origin => "origin",
            End::Target => "target",
        }
    }
}

/// A hint part under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HintHit {
    pub index: usize,
    pub part: Part,
}

/// A badge's box on screen: its least and greatest corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BadgeBox {
    pub min: Point,
    pub max: Point,
}

impl BadgeBox {
    pub fn contains(&self, p: Point) -> bool {
        (self.min.x..=self.max.x).contains(&p.x) && (self.min.y..=self.max.y).contains(&p.y)
    }

    pub fn center(&self) -> Point {
        Point::new((self.min.x + self.max.x) / 2.0, (self.min.y + self.max.y) / 2.0)
    }
}

/// The edited layer's PostScript hints as the canvas shows them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HintMarks {
    /// Show Hints is on and the layer's foreground is edited. When false, `hints` is empty and
    /// the hint selection is left alone.
    pub shown: bool,
    /// Every hint of the layer, in list order (`hints[i].index == i`).
    pub hints: Vec<HintMark>,
    /// The x of the glyph's origin and the y of the master's descender, in font units: where the
    /// markers' column and row are measured from.
    pub left: f64,
    pub bottom: f64,
    pub layout: HintLayout,
}

impl HintMarks {
    /// Hint `index`.
    pub fn get(&self, index: usize) -> Option<&HintMark> {
        self.hints.get(index).filter(|h| h.index == index)
    }

    /// Where `end`'s marker of `h` is on screen; `None` for a ghost's other end or an edge that
    /// cannot be placed.
    pub fn marker(&self, view: &crate::View, h: &HintMark, end: End) -> Option<Point> {
        let at = h.edge(end)?;
        let p = if h.horizontal {
            let column = view.to_screen(Point::new(self.left, at));
            Point::new(column.x - self.layout.lane, column.y)
        } else {
            let row = view.to_screen(Point::new(at, self.bottom));
            Point::new(row.x, row.y + self.layout.lane)
        };
        finite(p).then_some(p)
    }

    /// Where the badge of `h` is on screen.
    pub fn badge(&self, view: &crate::View, h: &HintMark) -> Option<BadgeBox> {
        let origin = self.marker(view, h, End::Origin)?;
        let other = self.marker(view, h, End::Target).unwrap_or(origin);
        let (w, hgt) = self.layout.badge;
        let reach = self.layout.marker + self.layout.gap;
        let (min, max) = if h.horizontal {
            let mid = (origin.y + other.y) / 2.0;
            let right = origin.x - reach;
            (Point::new(right - w, mid - hgt / 2.0), Point::new(right, mid + hgt / 2.0))
        } else {
            let mid = (origin.x + other.x) / 2.0;
            let top = origin.y + reach;
            (Point::new(mid - w / 2.0, top), Point::new(mid + w / 2.0, top + hgt))
        };
        (finite(min) && finite(max)).then_some(BadgeBox { min, max })
    }

    /// The hint part at the font point `pos`: markers first, then badges, later hints over
    /// earlier ones, as they are drawn.
    pub fn hit(&self, view: &crate::View, pos: Point, tolerance: f64) -> Option<HintHit> {
        if !self.shown || !finite(pos) {
            return None;
        }
        let at = view.to_screen(pos);
        let reach = self.layout.marker.max(0.0) + tolerance.max(0.0);
        for h in self.hints.iter().rev() {
            for end in [End::Target, End::Origin] {
                if self.marker(view, h, end).is_some_and(|m| dist(m, at) <= reach) {
                    return Some(HintHit { index: h.index, part: Part::Marker(end) });
                }
            }
        }
        self.hints.iter().rev().find(|h| self.badge(view, h).is_some_and(|b| b.contains(at))).map(|h| HintHit { index: h.index, part: Part::Badge })
    }

    /// The selected hints that the layer has, in list order.
    pub fn selected(&self, selection: &Selection) -> Vec<usize> {
        selection.hints.iter().copied().filter(|&i| self.get(i).is_some()).collect()
    }
}

/// The path index (paths only, components skipped) of the path at `shape`.
pub fn path_index(scene: &Scene, shape: usize) -> Option<usize> {
    match scene.layer.shapes.get(shape)? {
        Shape::Path(_) => Some(scene.layer.shapes.iter().take(shape).filter(|s| matches!(s, Shape::Path(_))).count()),
        _ => None,
    }
}

/// The shape index of the `path`-th path.
pub fn shape_index(scene: &Scene, path: usize) -> Option<usize> {
    scene.layer.shapes.iter().enumerate().filter(|(_, s)| matches!(s, Shape::Path(_))).nth(path).map(|(i, _)| i)
}

/// The command target of hint `index`: `{glyph, layer, hint}`.
fn hint_target(scene: &Scene, index: usize) -> Map<String, Value> {
    let mut m = scene.target();
    m.insert("hint".into(), json!(index));
    m
}

/// What a press at `ev.pos` does when it lands on a hint: the selection change, and the drag a
/// marker press may start. `None` when no hint is there.
pub fn press(cx: &Context<'_>, ev: &PointerEvent) -> Option<(Output, Option<HintDrag>)> {
    let marks = &cx.scene.ps_hints;
    let hit = marks.hit(&cx.view, ev.pos, cx.settings.hit_tolerance)?;
    let mut out = Output::handled();
    if ev.mods.shift {
        let mut selection = cx.selection.clone();
        selection.toggle(Item::Hint(hit.index));
        out.push(Action::Select { selection });
        return Some((out, None));
    }
    let only = Selection::from_items([Item::Hint(hit.index)]);
    if *cx.selection != only {
        out.push(Action::Select { selection: only });
    }
    let drag = match hit.part {
        Part::Marker(end) => HintDrag::press(cx, hit.index, end, ev.pos),
        Part::Badge => None,
    };
    Some((out, drag))
}

/// The keys that act on selected hints: `Tab` and `Shift+Tab` step through them, `Delete`
/// removes them, the arrow keys move them. `None` when no hint is selected or the key is not
/// one of these.
pub fn key(cx: &Context<'_>, ev: &KeyEvent) -> Option<Output> {
    let marks = &cx.scene.ps_hints;
    let arrow = matches!(ev.key, Key::Left | Key::Right | Key::Up | Key::Down);
    if !ev.pressed || !marks.shown || ev.mods.ctrl || ev.mods.alt || (ev.mods.cmd && !arrow) {
        return None;
    }
    let selected = marks.selected(cx.selection);
    if selected.is_empty() {
        return None;
    }
    if arrow {
        return Some(nudge(cx, ev, &selected));
    }
    let invocation = match ev.key {
        Key::Tab => Invocation::new(if ev.mods.shift { "hint.ps.selectPreviousHint" } else { "hint.ps.selectNextHint" }, json!({})),
        Key::Delete | Key::Backspace if !ev.mods.shift => {
            let mut m = cx.scene.target();
            m.insert("hints".into(), json!(selected));
            Invocation::new("hint.ps.deleteHints", Value::Object(m))
        }
        _ => return None,
    };
    Some(Output::with(Action::Run { invocation }))
}

/// The arrow key `ev` on the `selected` hints: one `hint.ps.moveHints` for all of them (one undo
/// step), then the selection again, since the command answers with a selection of no hints.
/// Hints along the arrow stay where they are; the key is used either way, so it does not move
/// the hidden node selection.
fn nudge(cx: &Context<'_>, ev: &KeyEvent, selected: &[usize]) -> Output {
    let [plain, big, huge] = cx.settings.increments;
    let step = if ev.mods.cmd {
        huge
    } else if ev.mods.shift {
        big
    } else {
        plain
    };
    let (horizontal, by) = match ev.key {
        Key::Up => (true, step),
        Key::Down => (true, -step),
        Key::Right => (false, step),
        _ => (false, -step),
    };
    let mut out = Output::handled();
    if !by.is_finite() || by == 0.0 {
        return out;
    }
    let moving: Vec<usize> = selected.iter().copied().filter(|&i| cx.scene.ps_hints.get(i).is_some_and(|h| h.horizontal == horizontal)).collect();
    if moving.is_empty() {
        return out;
    }
    let mut m = cx.scene.target();
    m.insert("hints".into(), json!(moving));
    m.insert(if horizontal { "dy" } else { "dx" }.into(), json!(by));
    out.push(Action::Run { invocation: Invocation::new("hint.ps.moveHints", Value::Object(m)) });
    out.push(Action::Select { selection: cx.selection.clone() });
    out
}

/// What a drag last asked for.
#[derive(Clone, Debug, PartialEq)]
enum Sent {
    Value(f64),
    Node(usize, usize),
}

/// A hint marker drag, from the press to the release.
#[derive(Clone, Debug)]
pub struct HintDrag {
    index: usize,
    end: End,
    horizontal: bool,
    ghost: bool,
    origin: f64,
    /// The other edge, when it is a number rather than a node link.
    numeric_target: Option<f64>,
    down: Point,
    begun: bool,
    pub last_pos: Point,
    last_sent: Option<Sent>,
    /// The node under the pointer, as `[path, node]`, and where it is.
    over: Option<((usize, usize), Point)>,
}

impl HintDrag {
    /// The drag a press at `pos` on `end`'s marker of hint `index` starts. `None` when the glyph
    /// is locked or the hint cannot be placed.
    pub fn press(cx: &Context<'_>, index: usize, end: End, pos: Point) -> Option<Self> {
        let h = cx.scene.ps_hints.get(index)?;
        if cx.scene.locked || !finite(pos) {
            return None;
        }
        let origin = h.origin?;
        h.edge(end)?;
        Some(Self {
            index,
            end,
            horizontal: h.horizontal,
            ghost: h.kind.is_ghost(),
            origin,
            numeric_target: if h.target_node.is_none() { h.target() } else { None },
            down: pos,
            begun: false,
            last_pos: pos,
            last_sent: None,
            over: None,
        })
    }

    /// The hint being dragged.
    pub fn index(&self) -> usize {
        self.index
    }

    /// Takes the drag to the pointer: the interaction starts once the pointer has moved past the
    /// drag threshold.
    pub fn drag(&mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        let mut out = Output::handled();
        if !self.begun {
            if dist(cx.view.to_screen(self.down), cx.view.to_screen(ev.pos)) < cx.settings.drag_threshold {
                return out;
            }
            self.begun = true;
            out.push(Action::Begin { label: DRAG_LABEL.into() });
        }
        if let Some(invocation) = self.update(cx, ev.pos) {
            out.push(Action::Preview { invocation });
        }
        out
    }

    /// Ends the drag: one undo step, and the hint selected again (the commands' results replace
    /// the selection).
    pub fn release(mut self, cx: &Context<'_>, ev: &PointerEvent) -> Output {
        let mut out = Output::handled();
        if !self.begun {
            return out;
        }
        if let Some(invocation) = self.update(cx, ev.pos) {
            out.push(Action::Preview { invocation });
        }
        out.push(Action::Commit);
        out.push(Action::Select { selection: Selection::from_items([Item::Hint(self.index)]) });
        out
    }

    /// Ends the drag without a change.
    pub fn cancel(&self) -> Output {
        if self.begun { Output::with(Action::Cancel) } else { Output::default() }
    }

    /// Where the dragged marker is going: the node it would link to, else the pointer.
    pub fn overlays(&self) -> Vec<Overlay> {
        if !self.begun {
            return Vec::new();
        }
        match self.over {
            Some((_, at)) => vec![Overlay::Marker { kind: OverlayKind::SnapTarget, at }],
            None => vec![Overlay::Marker { kind: OverlayKind::Hover, at: self.last_pos }],
        }
    }

    /// The command for the pointer at `pos`, when it differs from the last one: a link to the
    /// node under the pointer, else the edge at the pointer, rounded to whole units.
    fn update(&mut self, cx: &Context<'_>, pos: Point) -> Option<Invocation> {
        if !finite(pos) {
            return None;
        }
        self.last_pos = pos;
        self.over = match hit_test(cx.scene, &cx.view, pos, cx.settings.hit_tolerance).map(|h| (h.hit, h.at)) {
            Some((Hit::Node { shape, node }, at)) => path_index(cx.scene, shape).map(|path| ((path, node), at)),
            _ => None,
        };
        let mut params = hint_target(cx.scene, self.index);
        let (id, sent) = match self.over {
            Some(((path, node), _)) => {
                params.insert("end".into(), json!(self.end.as_str()));
                params.insert("node".into(), json!([path, node]));
                ("hint.ps.attachHintToNode", Sent::Node(path, node))
            }
            None => {
                let v = (if self.horizontal { pos.y } else { pos.x }).round();
                if !v.is_finite() {
                    return None;
                }
                match self.end {
                    End::Origin => {
                        params.insert("origin".into(), json!(v));
                        // A numeric other edge stays put; a linked one keeps its node.
                        if let Some(t) = self.numeric_target.filter(|_| !self.ghost) {
                            params.insert("size".into(), json!(t - v));
                        }
                    }
                    End::Target => {
                        params.insert("size".into(), json!(v - self.origin));
                    }
                }
                ("hint.ps.setHintValues", Sent::Value(v))
            }
        };
        if self.last_sent.as_ref() == Some(&sent) {
            return None;
        }
        self.last_sent = Some(sent);
        Some(Invocation::new(id, Value::Object(params)))
    }
}

#[cfg(test)]
mod tests {
    use fontcraft_model::{Component, Layer, Path};

    use super::*;
    use crate::View;
    use crate::event::PointerKind::{Down, Drag, Up};
    use crate::event::{Modifiers, PointerKind};
    use crate::select::SelectTool;
    use crate::tool::{Tool, ToolSettings};

    const NONE: Modifiers = Modifiers::NONE;
    const SHIFT: Modifiers = Modifiers { shift: true, alt: false, cmd: false, ctrl: false };

    /// Shape 0: a component; shape 1: the path (100,0) (300,0) (300,700) (100,700). Two hints:
    /// 0, a horizontal stem from y 0 to 80 with numeric edges; 1, a vertical stem from x 100,
    /// linked to node [0, 0], 200 wide.
    fn layer() -> Layer {
        let mut layer = Layer::new("m01", 600.0);
        layer.shapes.push(Shape::Component(Component::new("base", 0.0, 0.0)));
        layer.shapes.push(Shape::Path(Path::rect(100.0, 0.0, 300.0, 700.0)));
        layer
    }

    fn marks() -> HintMarks {
        let stem = |index, horizontal, origin, size, origin_node| HintMark {
            index,
            horizontal,
            kind: HintKind::Stem,
            origin: Some(origin),
            size: Some(size),
            origin_node,
            target_node: None,
            warnings: Vec::new(),
        };
        HintMarks {
            shown: true,
            hints: vec![stem(0, true, 0.0, 80.0, None), stem(1, false, 100.0, 200.0, Some((0, 0)))],
            left: 0.0,
            bottom: -200.0,
            layout: HintLayout::default(),
        }
    }

    struct Shell {
        layer: Layer,
        marks: HintMarks,
        selection: Selection,
        view: View,
        actions: Vec<Action>,
    }

    impl Shell {
        fn new() -> Self {
            Self { layer: layer(), marks: marks(), selection: Selection::new(), view: View::new(1.0, Point::new(300.0, 800.0)), actions: Vec::new() }
        }

        fn run(&mut self, f: impl FnOnce(&Context<'_>) -> Output) {
            let mut scene = Scene::new("H", "m01", &self.layer);
            scene.ps_hints = self.marks.clone();
            let cx = Context { scene: &scene, view: self.view, selection: &self.selection, settings: ToolSettings::default() };
            let out = f(&cx);
            for a in out.actions {
                if let Action::Select { selection } = &a {
                    self.selection = selection.clone();
                }
                self.actions.push(a);
            }
        }

        fn pointer(&mut self, tool: &mut SelectTool, kind: PointerKind, at: Point, mods: Modifiers) {
            let pos = self.view.to_font(at);
            self.run(|cx| tool.pointer(cx, &PointerEvent { kind, pos, mods }));
        }

        fn marker(&self, index: usize, end: End) -> Point {
            let h = self.marks.get(index).unwrap();
            self.marks.marker(&self.view, h, end).unwrap()
        }

        fn badge(&self, index: usize) -> Point {
            self.marks.badge(&self.view, self.marks.get(index).unwrap()).unwrap().center()
        }

        fn runs(&self) -> Vec<(String, Value)> {
            self.actions
                .iter()
                .filter_map(|a| match a {
                    Action::Preview { invocation } | Action::Run { invocation } => Some((invocation.id.clone(), invocation.params.clone())),
                    _ => None,
                })
                .collect()
        }
    }

    #[test]
    fn a_badge_click_selects_the_hint_and_shift_click_adds_another() {
        let mut shell = Shell::new();
        let mut tool = SelectTool::default();
        let at = shell.badge(0);
        shell.pointer(&mut tool, Down, at, NONE);
        shell.pointer(&mut tool, Up, at, NONE);
        assert_eq!(shell.selection, Selection::from_items([Item::Hint(0)]));
        let at = shell.badge(1);
        shell.pointer(&mut tool, Down, at, SHIFT);
        shell.pointer(&mut tool, Up, at, SHIFT);
        assert_eq!(shell.selection, Selection::from_items([Item::Hint(0), Item::Hint(1)]));
        shell.pointer(&mut tool, Down, at, SHIFT);
        assert_eq!(shell.selection, Selection::from_items([Item::Hint(0)]));
        assert!(shell.runs().is_empty(), "selecting changes nothing in the font");
    }

    #[test]
    fn dragging_the_origin_moves_that_edge_and_keeps_a_numeric_other_edge() {
        let mut shell = Shell::new();
        let mut tool = SelectTool::default();
        let at = shell.marker(0, End::Origin);
        shell.pointer(&mut tool, Down, at, NONE);
        // To y = 20 in font units, far from any node in x.
        shell.pointer(&mut tool, Drag, Point::new(at.x, at.y - 20.0), NONE);
        shell.pointer(&mut tool, Up, Point::new(at.x, at.y - 20.0), NONE);
        let runs = shell.runs();
        let (id, params) = runs.last().unwrap();
        assert_eq!(id, "hint.ps.setHintValues");
        assert_eq!(params["hint"], 0);
        assert_eq!((params["origin"].as_f64(), params["size"].as_f64()), (Some(20.0), Some(60.0)));
        assert!(matches!(shell.actions.first(), Some(Action::Select { .. })));
        assert!(shell.actions.iter().any(|a| matches!(a, Action::Begin { label } if label == DRAG_LABEL)));
        assert!(matches!(shell.actions.iter().rev().nth(1), Some(Action::Commit)));
        assert_eq!(shell.selection, Selection::from_items([Item::Hint(0)]), "the dragged hint stays selected");
    }

    #[test]
    fn dragging_the_triangle_sets_the_size_and_a_drop_on_a_node_links_the_end() {
        let mut shell = Shell::new();
        let mut tool = SelectTool::default();
        let at = shell.marker(1, End::Target);
        shell.pointer(&mut tool, Down, at, NONE);
        shell.pointer(&mut tool, Drag, Point::new(at.x - 50.0, at.y), NONE);
        let runs = shell.runs();
        let (id, params) = runs.last().unwrap();
        assert_eq!((id.as_str(), params["size"].as_f64()), ("hint.ps.setHintValues", Some(150.0)));
        assert!(params.get("origin").is_none(), "the linked origin is left alone");
        // Onto the node at (300, 700): shape 1 is the first path, so the node is [0, 2].
        let node = shell.view.to_screen(Point::new(300.0, 700.0));
        shell.pointer(&mut tool, Drag, node, NONE);
        shell.pointer(&mut tool, Up, node, NONE);
        let runs = shell.runs();
        let (id, params) = runs.last().unwrap();
        assert_eq!(id, "hint.ps.attachHintToNode");
        assert_eq!((params["hint"].clone(), params["end"].clone(), params["node"].clone()), (json!(1), json!("target"), json!([0, 2])));
    }

    #[test]
    fn tab_and_delete_act_on_the_selected_hints() {
        let mut shell = Shell::new();
        shell.selection = Selection::from_items([Item::Hint(1), Item::Hint(0)]);
        let mut tool = SelectTool::default();
        shell.run(|cx| tool.key(cx, &KeyEvent::press(Key::Tab, SHIFT)));
        shell.run(|cx| tool.key(cx, &KeyEvent::press(Key::Delete, NONE)));
        let runs = shell.runs();
        assert_eq!(runs.first().map(|r| r.0.as_str()), Some("hint.ps.selectPreviousHint"));
        let (id, params) = runs.last().unwrap();
        assert_eq!((id.as_str(), params["hints"].clone(), params["layer"].clone()), ("hint.ps.deleteHints", json!([0, 1]), json!("m01")));
    }

    #[test]
    fn arrow_keys_move_the_selected_hints_across_the_arrow_and_keep_them_selected() {
        let mut shell = Shell::new();
        let both = Selection::from_items([Item::Hint(0), Item::Hint(1)]);
        shell.selection = both.clone();
        let mut tool = SelectTool::default();
        // Up moves the horizontal hint 0 only, by the plain increment, in one command.
        shell.run(|cx| tool.key(cx, &KeyEvent::press(Key::Up, NONE)));
        let runs = shell.runs();
        assert_eq!(runs.len(), 1, "{runs:?}");
        let (id, params) = &runs[0];
        assert_eq!((id.as_str(), params["hints"].clone(), params["layer"].clone()), ("hint.ps.moveHints", json!([0]), json!("m01")));
        assert_eq!((params["dy"].as_f64(), params.get("dx")), (Some(1.0), None));
        assert_eq!(shell.selection, both, "the hints stay selected");
        // Shift+Left moves the vertical hint 1 by the big increment.
        shell.actions.clear();
        shell.run(|cx| tool.key(cx, &KeyEvent::press(Key::Left, SHIFT)));
        let runs = shell.runs();
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!((runs[0].1["hints"].clone(), runs[0].1["dx"].as_f64()), (json!([1]), Some(-10.0)));
        // Without a selected hint the arrows are not the hints'.
        shell.selection = Selection::new();
        let scene_marks = shell.marks.clone();
        let scene = {
            let mut scene = Scene::new("H", "m01", &shell.layer);
            scene.ps_hints = scene_marks;
            scene
        };
        let cx = Context { scene: &scene, view: shell.view, selection: &shell.selection, settings: ToolSettings::default() };
        assert!(key(&cx, &KeyEvent::press(Key::Up, NONE)).is_none());
    }

    #[test]
    fn hidden_hints_are_not_hit_and_a_locked_glyph_selects_without_dragging() {
        let mut shell = Shell::new();
        let at = shell.badge(0);
        shell.marks.shown = false;
        assert!(shell.marks.hit(&shell.view, shell.view.to_font(at), 6.0).is_none());
        let shell = Shell::new();
        let marker = shell.marker(0, End::Origin);
        let mut scene = Scene::new("H", "m01", &shell.layer);
        scene.ps_hints = shell.marks.clone();
        scene.locked = true;
        let cx = Context { scene: &scene, view: shell.view, selection: &shell.selection, settings: ToolSettings::default() };
        let pos = shell.view.to_font(marker);
        let (out, drag) = press(&cx, &PointerEvent { kind: Down, pos, mods: NONE }).unwrap();
        assert!(drag.is_none());
        assert!(matches!(out.actions.as_slice(), [Action::Select { .. }]));
    }

    #[test]
    fn path_indexes_skip_components() {
        let layer = layer();
        let scene = Scene::new("H", "m01", &layer);
        assert_eq!((path_index(&scene, 1), path_index(&scene, 0), shape_index(&scene, 0), shape_index(&scene, 1)), (Some(0), None, Some(1), None));
    }
}
