//! The TrueType Instructor on the canvas: its view state and shell commands, the instruction
//! badges in both directions (the other direction dimmed), the nodes as gray discs, and the
//! point numbers.
//!
//! Rows: `hinting.tt.instructions.badgeColours`, `hinting.tt.tool.otherDirectionDimmed`,
//! `hinting.tt.tool.masterOutline`, `hint.tt.toggleDirection`, `hint.tt.increasePixelSize`,
//! `hint.tt.decreasePixelSize`, `hint.tt.showPointIndexes`, `hint.tt.selectInstructions`.
//!
//! The view state is [`InstructorState`], at `UiState::tt_instructor`; the tool reads its
//! direction through `ToolSettings::tt_direction`. What later packages build on:
//!
//! | Item | For |
//! |---|---|
//! | [`InstructorState::ppm`], [`MIN_PPM`], [`MAX_PPM`], [`clamp_ppm`] | the preview pixel size (`hint.tt.setPixelSize`, the grid-fitted outline, the pixel backdrop) |
//! | [`InstructorState::direction`] | the Info box's direction button, which runs `hint.tt.toggleDirection` |
//! | [`InstructorState::selected_in`] | the selected instructions, by list position, which the Info box edits |
//! | [`badges`] | where each badge is drawn, for hit tests and frame tests |
//!
//! Badges sit beside the point an instruction moves (Snap, Delta, Interpolate, with a leader line
//! to it; Interpolate also has lines to its two references) or halfway along the arrow from origin
//! to target (Stem, Shift). Badges on one spot stack away from the point. Clicking a badge of the
//! current direction runs `hint.tt.selectInstructions`; Shift+click toggles it.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Vec2};
use fontcraft_engine::compile::Options;
use fontcraft_engine::hinting::{Direction, MAX_LAYER_HINTS, NodeRef, TtInstruction, TtKind, TtPoint, tt_instructions};
use fontcraft_geom::Point;
use fontcraft_model::{Layer, NodeKind, Path, Shape as ModelShape, Target};
use fontcraft_tools::View;
use fontcraft_tools::tt_instructor::{ID as INSTRUCTOR, TtDirection};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Painting, Scene};
use crate::app::FontcraftApp;
use crate::panels::Pending;
use crate::shell::{Args, UiCommand};
use crate::theme::TtTokens;

/// Smallest preview pixel size, in pixels per em.
pub const MIN_PPM: u16 = 4;
/// Largest preview pixel size.
pub const MAX_PPM: u16 = 400;
/// The pixel size the Instructor starts with.
pub const DEFAULT_PPM: u16 = 12;

/// `ppm` brought into [`MIN_PPM`]..=[`MAX_PPM`].
pub fn clamp_ppm(ppm: i64) -> u16 {
    u16::try_from(ppm.clamp(i64::from(MIN_PPM), i64::from(MAX_PPM))).unwrap_or(DEFAULT_PPM)
}

/// The TrueType Instructor's view state. Nothing here is saved with the font.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InstructorState {
    /// The hinting direction new instructions take (`hint.tt.toggleDirection`).
    pub direction: TtDirection,
    /// The preview pixel size, [`MIN_PPM`] to [`MAX_PPM`].
    pub ppm: u16,
    /// Point numbers beside the on-curve nodes (`hint.tt.showPointIndexes`).
    pub point_indexes: bool,
    /// The selected instructions (`hint.tt.selectInstructions`).
    pub selected: SelectedInstructions,
}

impl Default for InstructorState {
    fn default() -> Self {
        Self { direction: TtDirection::Vertical, ppm: DEFAULT_PPM, point_indexes: false, selected: SelectedInstructions::default() }
    }
}

/// Instructions selected in one layer of one glyph, by their position in its instruction list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SelectedInstructions {
    pub glyph: String,
    pub layer: String,
    pub instructions: BTreeSet<usize>,
}

static NONE_SELECTED: BTreeSet<usize> = BTreeSet::new();

impl InstructorState {
    /// The selected instructions of `glyph`'s layer `layer`: empty when the selection belongs to
    /// another glyph or layer. Positions past the end of the list, after an edit, name nothing.
    pub fn selected_in(&self, glyph: &str, layer: &str) -> &BTreeSet<usize> {
        if self.selected.glyph == glyph && self.selected.layer == layer { &self.selected.instructions } else { &NONE_SELECTED }
    }
}

/// The hinting crate's direction for the Instructor's.
pub fn hinting_direction(d: TtDirection) -> Direction {
    match d {
        TtDirection::Vertical => Direction::Vertical,
        TtDirection::Horizontal => Direction::Horizontal,
    }
}

pub const COMMANDS: &[UiCommand] = &[
    (
        "hint.tt.toggleDirection",
        "Toggle Hinting Direction",
        &[],
        "X",
        "{direction?: \"vertical\" | \"horizontal\" (omitted: the other one)} the TrueType Instructor's hinting direction: vertical instructions move points in y (for horizontal stems), horizontal ones in x. New instructions take it; the instructions of the other direction are drawn dimmed and cannot be selected. A change clears the instruction selection → {direction}",
    ),
    (
        "hint.tt.increasePixelSize",
        "Increase Pixel Size",
        &[],
        ".",
        "{} raise the TrueType Instructor's preview pixel size by one, up to 400 PPM → {ppm}",
    ),
    (
        "hint.tt.decreasePixelSize",
        "Decrease Pixel Size",
        &[],
        ",",
        "{} lower the TrueType Instructor's preview pixel size by one, down to 4 PPM → {ppm}",
    ),
    (
        "hint.tt.showPointIndexes",
        "Show Point Indexes",
        &[],
        "",
        "{on?: bool} label each on-curve node of the edited glyph with its point number in the exported TrueType outline, the numbers TrueType instructions and autohinter control instructions use (omitted: toggle) → {on}",
    ),
    (
        "hint.tt.selectInstructions",
        "Select Instructions",
        &[],
        "",
        "{instructions: [index], mode?: \"replace\" | \"add\" | \"remove\" | \"toggle\" (default replace)} select TrueType instructions of the edited glyph's layer by their position in its instruction list, as clicking their badges in the TrueType Instructor does. Instructions of the other direction cannot be selected; an unknown position or another direction changes nothing → {instructions: [index]}",
    ),
];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "hint.tt.toggleDirection" => {
            let s = &mut app.state.tt_instructor;
            let direction = match args.string("direction")? {
                Some(name) => TtDirection::parse(name).ok_or_else(|| args.err("`direction` must be \"vertical\" or \"horizontal\""))?,
                None => s.direction.other(),
            };
            if direction != s.direction {
                s.selected.instructions.clear();
            }
            s.direction = direction;
            Ok(json!({"direction": direction.as_str()}))
        }
        "hint.tt.increasePixelSize" | "hint.tt.decreasePixelSize" => {
            let s = &mut app.state.tt_instructor;
            let step = if id == "hint.tt.increasePixelSize" { 1 } else { -1 };
            s.ppm = clamp_ppm(i64::from(s.ppm) + step);
            Ok(json!({"ppm": s.ppm}))
        }
        "hint.tt.showPointIndexes" => {
            let s = &mut app.state.tt_instructor;
            let on = args.toggle(s.point_indexes)?;
            s.point_indexes = on;
            Ok(json!({"on": on}))
        }
        "hint.tt.selectInstructions" => select_instructions(app, args),
        other => Err(format!("unknown command `{other}`")),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Replace,
    Add,
    Remove,
    Toggle,
}

fn select_instructions(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let Some(Value::Array(given)) = args.get("instructions") else {
        return Err(args.err("`instructions` must be a list of positions in the instruction list"));
    };
    if given.len() > MAX_LAYER_HINTS {
        return Err(args.err(format!("`instructions` names more than {MAX_LAYER_HINTS} instructions")));
    }
    let wanted: Vec<usize> = given
        .iter()
        .map(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()))
        .collect::<Option<_>>()
        .ok_or_else(|| args.err("`instructions` must hold whole numbers"))?;
    let mode = match args.string("mode")? {
        None | Some("replace") => Mode::Replace,
        Some("add") => Mode::Add,
        Some("remove") => Mode::Remove,
        Some("toggle") => Mode::Toggle,
        Some(other) => return Err(args.err(format!("unknown mode `{other}`; the modes are replace, add, remove and toggle"))),
    };
    let font = app.session.font().map_err(|e| e.to_string())?;
    let tab = app.state.tab().ok_or("no Edit View tab is showing (open a glyph first)")?;
    if tab.edit_background {
        return Err(args.err("instructions belong to the foreground; the background is being edited"));
    }
    let glyph = tab.edited_glyph().ok_or("no glyph is being edited")?.to_string();
    let layer = super::view::active_layer(font, tab).ok_or("the edited glyph has no layer in the active master")?;
    let list = tt_instructions(layer).map_err(|e| args.err(e))?;
    let direction = app.state.tt_instructor.direction;
    for &i in &wanted {
        let Some((_, t)) = list.get(i) else {
            return Err(args.err(format!("the layer has no instruction {i} (it has {})", list.len())));
        };
        if t.direction != hinting_direction(direction) {
            return Err(args.err(format!("instruction {i} is {}: switch the direction to select it", direction.other().as_str())));
        }
    }
    let layer = layer.id.clone();
    let state = &mut app.state.tt_instructor;
    let mut current = state.selected_in(&glyph, &layer).clone();
    current.retain(|&i| i < list.len());
    match mode {
        Mode::Replace => current = wanted.into_iter().collect(),
        Mode::Add => current.extend(wanted),
        Mode::Remove => current.retain(|i| !wanted.contains(i)),
        Mode::Toggle => {
            for i in wanted {
                if !current.remove(&i) {
                    current.insert(i);
                }
            }
        }
    }
    state.selected = SelectedInstructions { glyph, layer, instructions: current };
    Ok(json!({"instructions": state.selected.instructions}))
}

/// The look of a badge, by instruction type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeKind {
    Snap,
    Stem,
    /// A Stem instruction of a triple hint.
    TripleStem,
    Shift,
    Interpolate,
    Delta,
}

impl BadgeKind {
    pub fn of(kind: &TtKind) -> Self {
        match kind {
            TtKind::Snap { .. } => BadgeKind::Snap,
            TtKind::Stem { triple: Some(_), .. } => BadgeKind::TripleStem,
            TtKind::Stem { .. } => BadgeKind::Stem,
            TtKind::Shift => BadgeKind::Shift,
            TtKind::Interpolate => BadgeKind::Interpolate,
            TtKind::Delta { .. } => BadgeKind::Delta,
        }
    }

    pub fn colour(self, t: &TtTokens) -> Color32 {
        match self {
            BadgeKind::Snap => t.snap,
            BadgeKind::Stem => t.stem,
            BadgeKind::TripleStem => t.triple,
            BadgeKind::Shift => t.shift,
            BadgeKind::Interpolate => t.interpolate,
            BadgeKind::Delta => t.delta,
        }
    }
}

/// One instruction as drawn, in screen points.
#[derive(Clone, Debug, PartialEq)]
pub struct Badge {
    /// The instruction's position in the layer's instruction list.
    pub index: usize,
    pub kind: BadgeKind,
    /// The instruction has the current direction: drawn in colour and selectable.
    pub current: bool,
    /// The badge's centre.
    pub at: Pos2,
    /// The point the instruction moves.
    pub target: Pos2,
    /// Its reference points: the origin of Stem and Shift, both references of Interpolate.
    pub references: Vec<Pos2>,
}

/// Where the badges of `list` (the layer's instructions, as `tt_instructions` gives them) go in
/// `view`. Instructions whose points do not resolve on `layer` get no badge.
pub fn badges(layer: &Layer, list: &[(usize, TtInstruction)], view: View, t: &TtTokens, current: TtDirection) -> Vec<Badge> {
    let screen = |p: Point| {
        let s = view.to_screen(p);
        Pos2::new(s.x as f32, s.y as f32)
    };
    let mut stacked: BTreeMap<(i64, i64, bool), usize> = BTreeMap::new();
    let mut out = Vec::with_capacity(list.len());
    for (index, (_, ins)) in list.iter().enumerate() {
        let Some(target) = point_at(layer, &ins.target).map(screen) else { continue };
        let references: Vec<Pos2> = [ins.origin, ins.other].iter().flatten().filter_map(|p| point_at(layer, p)).map(screen).collect();
        let kind = BadgeKind::of(&ins.kind);
        // Away from the point across the direction the instruction moves it.
        let side = match ins.direction {
            Direction::Vertical => Vec2::new(-1.0, 0.0),
            Direction::Horizontal => Vec2::new(0.0, 1.0),
        };
        let anchor = match (kind, references.first()) {
            (BadgeKind::Stem | BadgeKind::TripleStem | BadgeKind::Shift, Some(origin)) => origin.lerp(target, 0.5),
            _ => target + side * t.badge_offset,
        };
        if !anchor.x.is_finite() || !anchor.y.is_finite() {
            continue;
        }
        let horizontal = ins.direction == Direction::Horizontal;
        let slot = stacked.entry((anchor.x.round() as i64, anchor.y.round() as i64, horizontal)).or_insert(0);
        let at = anchor + side * (t.badge_radius * 2.0 + t.selected_gap) * *slot as f32;
        *slot += 1;
        out.push(Badge { index, kind, current: ins.direction == hinting_direction(current), at, target, references });
    }
    out
}

/// Where an instruction's point is on `layer`, in font units.
fn point_at(layer: &Layer, p: &TtPoint) -> Option<Point> {
    let at = match p {
        TtPoint::Node(r) => r.resolve(layer).map(|n| Point::new(n.x, n.y))?,
        TtPoint::Intersection { a, b } => {
            let end = |r: &NodeRef| r.resolve(layer).map(|n| Point::new(n.x, n.y));
            let (pa, pb) = (end(a)?, end(b)?);
            let start = |r: &NodeRef| r.path(layer).and_then(|path| previous_on_curve(path, r.node));
            match (start(a), start(b)) {
                (Some(sa), Some(sb)) => crossing(sa, pa, sb, pb).unwrap_or_else(|| pa.midpoint(pb)),
                _ => pa.midpoint(pb),
            }
        }
        TtPoint::Lsb => Point::new(0.0, 0.0),
        TtPoint::Rsb => Point::new(if layer.width.is_finite() { layer.width } else { 0.0 }, 0.0),
    };
    (at.x.is_finite() && at.y.is_finite()).then_some(at)
}

/// The on-curve node before `node` on `path`, where the segment that ends at `node` starts.
fn previous_on_curve(path: &Path, node: usize) -> Option<Point> {
    let n = path.nodes.len();
    if node >= n {
        return None;
    }
    let before: Vec<usize> = if path.closed { (1..n).map(|k| (node + n - k) % n).collect() } else { (0..node).rev().collect() };
    before.into_iter().filter_map(|i| path.nodes.get(i)).find(|nd| nd.kind != NodeKind::OffCurve).map(|nd| Point::new(nd.x, nd.y))
}

/// Where the line through `a0` and `a1` crosses the line through `b0` and `b1`.
fn crossing(a0: Point, a1: Point, b0: Point, b1: Point) -> Option<Point> {
    let (ax, ay, bx, by) = (a1.x - a0.x, a1.y - a0.y, b1.x - b0.x, b1.y - b0.y);
    let d = ax * by - ay * bx;
    if !d.is_finite() || d.abs() < 1e-9 {
        return None;
    }
    let u = ((b0.x - a0.x) * by - (b0.y - a0.y) * bx) / d;
    let p = Point::new(a0.x + u * ax, a0.y + u * ay);
    (p.x.is_finite() && p.y.is_finite()).then_some(p)
}

/// Draws this overlay for the edited layer: the point numbers when they are on; with the
/// Instructor active, the nodes as gray discs and the instruction badges.
pub fn draw(ui: &egui::Ui, c: &Painting, scene: &Scene, pending: &mut Pending) {
    if scene.tab.edit_background {
        return;
    }
    let state = &scene.state.tt_instructor;
    if state.point_indexes {
        draw_point_indexes(c, scene);
    }
    if scene.tool != INSTRUCTOR {
        return;
    }
    draw_nodes(c, scene);
    let Ok(list) = tt_instructions(scene.layer) else { return };
    let glyph = scene.tab.edited_glyph().unwrap_or_default();
    let selected = state.selected_in(glyph, &scene.layer.id);
    let marks = badges(scene.layer, &list, c.view, &c.tokens.tt, state.direction);
    // The other direction behind, for reference only.
    for b in marks.iter().filter(|b| !b.current) {
        paint(c, b, false);
    }
    for b in marks.iter().filter(|b| b.current) {
        paint(c, b, selected.contains(&b.index));
        if !c.rect.contains(b.at) {
            continue;
        }
        let area = Rect::from_center_size(b.at, Vec2::splat(c.tokens.tt.badge_radius * 2.0));
        // Not focusable: a click on a badge must leave the keyboard to the tool.
        let response = ui.interact(area, ui.id().with(("tt-badge", b.index)), Sense::CLICK | Sense::DRAG);
        if response.clicked() {
            let shift = ui.input(|i| i.modifiers.shift);
            pending.push(("hint.tt.selectInstructions", json!({"instructions": [b.index], "mode": if shift { "toggle" } else { "replace" }})));
        }
    }
}

/// One badge: its lines, the badge, and the ring when it is selected.
fn paint(c: &Painting, b: &Badge, selected: bool) {
    let t = &c.tokens.tt;
    let colour = if b.current { b.kind.colour(t) } else { t.dimmed };
    let line = Stroke::new(t.line_width, colour);
    match b.kind {
        BadgeKind::Stem | BadgeKind::TripleStem | BadgeKind::Shift => match b.references.first() {
            Some(&origin) => {
                c.painter.line_segment([origin, b.target], line);
                arrowhead(c, origin, b.target, colour);
            }
            None => {
                c.painter.line_segment([b.target, b.at], line);
            }
        },
        BadgeKind::Interpolate => {
            for &r in &b.references {
                c.painter.line_segment([r, b.target], Stroke::new(t.line_width * 0.5, colour));
            }
            c.painter.line_segment([b.target, b.at], line);
        }
        BadgeKind::Snap | BadgeKind::Delta => {
            c.painter.line_segment([b.target, b.at], line);
        }
    }
    c.painter.circle(b.at, t.badge_radius, colour, Stroke::new(1.0, c.tokens.canvas));
    if selected {
        c.painter.circle_stroke(b.at, t.badge_radius + t.selected_gap, Stroke::new(t.line_width, c.tokens.selection));
    }
}

/// A filled arrowhead at `to`, pointing away from `from`.
fn arrowhead(c: &Painting, from: Pos2, to: Pos2, colour: Color32) {
    let size = c.tokens.tt.arrow_size;
    let d = to - from;
    let len = d.length();
    if !len.is_finite() || len <= size {
        return;
    }
    let dir = d / len;
    let normal = Vec2::new(-dir.y, dir.x);
    let base = to - dir * size;
    c.painter.add(Shape::convex_polygon(vec![to, base + normal * size * 0.5, base - normal * size * 0.5], colour, Stroke::NONE));
}

/// The on-curve nodes of the edited layer as gray discs, the selected ones in the selection
/// colour (`hinting.tt.tool.masterOutline`).
fn draw_nodes(c: &Painting, scene: &Scene) {
    let t = &c.tokens.tt;
    let mut shapes = Vec::new();
    for (s, shape) in scene.layer.shapes.iter().enumerate() {
        let ModelShape::Path(path) = shape else { continue };
        for (n, node) in path.nodes.iter().enumerate() {
            if node.kind == NodeKind::OffCurve {
                continue;
            }
            let p = c.view.to_screen(Point::new(node.x, node.y));
            let at = Pos2::new(p.x as f32, p.y as f32);
            if !at.x.is_finite() || !at.y.is_finite() {
                continue;
            }
            let fill = if scene.tab.selection.points.contains(&(s, n)) { c.tokens.selection } else { t.node };
            shapes.push(Shape::circle_filled(at, t.node_radius, fill));
        }
    }
    c.painter.extend(shapes);
}

/// The point numbers of a glyph's layer: `paths[path][node]`, paths counted without components.
type PointNumbers = Vec<Vec<Option<u16>>>;

thread_local! {
    /// The numbers last computed, with the document revision, glyph and layer they belong to:
    /// numbering runs the export's outline preparation, too slow for every frame.
    static NUMBERS: RefCell<Option<((u64, u64), String, String, PointNumbers)>> = const { RefCell::new(None) };
}

/// The point numbers of the edited layer's master outline, as `hint.tt.getPointIndexes` gives
/// them: empty for a composite or when the outline cannot be prepared.
fn point_numbers(scene: &Scene, glyph: &str) -> PointNumbers {
    let fresh = NUMBERS.with_borrow(|cached| match cached {
        Some((doc, g, l, numbers)) if *doc == scene.doc && g == glyph && *l == scene.layer.id => Some(numbers.clone()),
        _ => None,
    });
    if let Some(numbers) = fresh {
        return numbers;
    }
    let target = Target::Master(scene.layer.master.clone());
    let options = Options { remove_overlap: true, ..Options::default() };
    let numbers =
        fontcraft_engine::compile::points::point_indexes(scene.font, &target, glyph, &options).ok().flatten().map(|m| m.paths).unwrap_or_default();
    NUMBERS.with_borrow_mut(|cached| *cached = Some((scene.doc, glyph.to_string(), scene.layer.id.clone(), numbers.clone())));
    numbers
}

/// Each on-curve node's point number beside it (`hint.tt.showPointIndexes`).
fn draw_point_indexes(c: &Painting, scene: &Scene) {
    let Some(glyph) = scene.tab.edited_glyph() else { return };
    let numbers = point_numbers(scene, glyph);
    let t = &c.tokens.tt;
    let paths = scene.layer.shapes.iter().filter_map(|s| match s {
        ModelShape::Path(p) => Some(p),
        _ => None,
    });
    for (path, row) in paths.zip(&numbers) {
        for (node, number) in path.nodes.iter().zip(row) {
            let Some(number) = number else { continue };
            if node.kind == NodeKind::OffCurve {
                continue;
            }
            let p = c.view.to_screen(Point::new(node.x, node.y));
            let at = Pos2::new(p.x as f32 + t.index_gap, p.y as f32 - t.index_gap);
            if c.rect.contains(at) {
                c.painter.text(at, Align2::LEFT_BOTTOM, number.to_string(), FontId::proportional(t.index_size), t.index_text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, PointerButton};

    use super::*;
    use crate::{control, tools_glue};

    /// A headless window: one egui context kept across frames.
    struct Window {
        ctx: egui::Context,
    }

    impl Window {
        fn new() -> Self {
            Self { ctx: egui::Context::default() }
        }

        /// One frame with these events; the shapes it drew, nested lists flattened.
        fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) -> Vec<Shape> {
            let input =
                egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
            let output = self.ctx.run_ui(input, |ui| app.ui(ui));
            let mut out: Vec<Shape> = output.shapes.into_iter().map(|c| c.shape).collect();
            let mut flat = Vec::new();
            while let Some(shape) = out.pop() {
                match shape {
                    Shape::Vec(list) => out.extend(list),
                    other => flat.push(other),
                }
            }
            flat
        }
    }

    /// Glyph A with three squares (paths 0, 1, 2), edited at actual size.
    fn squares() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        for x in [100, 250, 400] {
            app.run("path.addRectangle", &json!({"glyph": "A", "x": x, "y": 100, "width": 100, "height": 100})).unwrap();
        }
        app
    }

    fn edit(app: &mut FontcraftApp) {
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        app.run("view.setZoom", &json!({"size": 1000, "center": [300, 150]})).unwrap();
        app.run("tool.select", &json!({"tool": INSTRUCTOR})).unwrap();
    }

    fn add(app: &mut FontcraftApp, id: &str, nodes: Value, direction: &str) {
        app.run(id, &json!({"glyph": "A", "nodes": nodes, "direction": direction})).unwrap();
    }

    /// Every instruction type in each direction it exists in: vertical Snap 0, Stem 1, Shift 2,
    /// Interpolate 3, Delta 4, a triple hint 5–7; horizontal Snap 8, Stem 9, Shift 10,
    /// Interpolate 11.
    fn hinted() -> FontcraftApp {
        let mut app = squares();
        for direction in ["vertical", "horizontal"] {
            add(&mut app, "hint.tt.snapPoint", json!([[0, 0]]), direction);
            add(&mut app, "hint.tt.addStemHint", json!([[0, 1], [0, 2]]), direction);
            add(&mut app, "hint.tt.shiftPoints", json!([[0, 0], [0, 3]]), direction);
            add(&mut app, "hint.tt.interpolate", json!([[0, 0], [0, 2], [1, 0]]), direction);
            if direction == "vertical" {
                add(&mut app, "hint.tt.addDelta", json!([[1, 1]]), direction);
                add(&mut app, "hint.tt.addStemHint", json!([[1, 2], [1, 3], [2, 0], [2, 1], [2, 2], [2, 3]]), direction);
                app.run("hint.tt.makeTripleHint", &json!({"glyph": "A", "instructions": [5, 6, 7]})).unwrap();
            }
        }
        edit(&mut app);
        app
    }

    fn layer(app: &FontcraftApp) -> Layer {
        let font = app.session.font().unwrap();
        font.glyph("A").unwrap().layer(&font.masters[0].id).unwrap().clone()
    }

    /// Badges drawn with each fill.
    fn count(shapes: &[Shape], t: &TtTokens, fill: Color32) -> usize {
        shapes.iter().filter(|s| matches!(s, Shape::Circle(c) if c.fill == fill && c.radius == t.badge_radius)).count()
    }

    #[test]
    fn badges_of_every_type_are_coloured_in_the_current_direction_and_dimmed_in_the_other() {
        let mut app = hinted();
        let w = Window::new();
        w.frame(&mut app, Vec::new());
        let t = app.tokens.tt.clone();
        let shapes = w.frame(&mut app, Vec::new());
        let by_kind = |shapes: &[Shape]| [t.snap, t.stem, t.shift, t.interpolate, t.delta, t.dimmed].map(|fill| count(shapes, &t, fill)).to_vec();
        // Snap and triple share purple: 1 Snap + 3 triple Stems.
        assert_eq!(t.snap, t.triple);
        assert_eq!(by_kind(&shapes), vec![4, 1, 1, 1, 1, 4], "vertical: the vertical instructions in colour, the 4 horizontal ones dimmed");
        let discs = shapes.iter().filter(|s| matches!(s, Shape::Circle(c) if c.fill == t.node && c.radius == t.node_radius)).count();
        assert_eq!(discs, 12, "every node a gray disc");

        app.run("hint.tt.toggleDirection", &json!({})).unwrap();
        let shapes = w.frame(&mut app, Vec::new());
        assert_eq!(by_kind(&shapes), vec![1, 1, 1, 1, 0, 8], "horizontal: no Delta, the 8 vertical instructions dimmed");

        // Another tool draws no badges.
        app.run("tool.select", &json!({"tool": "select"})).unwrap();
        let shapes = w.frame(&mut app, Vec::new());
        assert_eq!(count(&shapes, &t, t.dimmed) + count(&shapes, &t, t.stem), 0);
    }

    #[test]
    fn badge_layout_stacks_badges_on_one_point_and_marks_the_direction() {
        let app = hinted();
        let list = tt_instructions(&layer(&app)).unwrap();
        let view = tools_glue::current_view(&app).unwrap();
        let t = &app.tokens.tt;
        let marks = badges(&layer(&app), &list, view, t, TtDirection::Vertical);
        assert_eq!(marks.len(), 12);
        let kinds: Vec<BadgeKind> = marks.iter().map(|b| b.kind).collect();
        let (snap, stem, shift, interpolate) = (BadgeKind::Snap, BadgeKind::Stem, BadgeKind::Shift, BadgeKind::Interpolate);
        let triple = BadgeKind::TripleStem;
        assert_eq!(kinds, vec![snap, stem, shift, interpolate, BadgeKind::Delta, triple, triple, triple, snap, stem, shift, interpolate]);
        assert!(marks.iter().take(8).all(|b| b.current) && marks.iter().skip(8).all(|b| !b.current));
        // Interpolate has two references; Stem and Shift badges sit on their arrows.
        assert_eq!(marks[3].references.len(), 2);
        assert_eq!(marks[1].at, marks[1].references[0].lerp(marks[1].target, 0.5));
        // No two badges of one direction on one spot.
        for (i, a) in marks.iter().enumerate() {
            for b in marks.iter().skip(i + 1) {
                assert!(a.current != b.current || a.at.distance(b.at) > 1.0, "{a:?} and {b:?} overlap");
            }
        }
    }

    #[test]
    fn a_click_on_a_badge_selects_its_instruction_and_shift_click_toggles() {
        let mut app = hinted();
        let w = Window::new();
        w.frame(&mut app, Vec::new());
        let list = tt_instructions(&layer(&app)).unwrap();
        let view = tools_glue::current_view(&app).unwrap();
        let marks = badges(&layer(&app), &list, view, &app.tokens.tt, TtDirection::Vertical);
        let points = app.state.tab().unwrap().selection.clone();
        let click = |app: &mut FontcraftApp, at: Pos2, modifiers: Modifiers| {
            w.frame(app, vec![Event::PointerMoved(at)]);
            w.frame(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers }]);
            w.frame(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers }]);
            w.frame(app, Vec::new());
        };
        click(&mut app, marks[0].at, Modifiers::NONE);
        assert_eq!(app.state.tt_instructor.selected_in("A", &layer(&app).id), &BTreeSet::from([0]));
        click(&mut app, marks[2].at, Modifiers::SHIFT);
        assert_eq!(app.state.tt_instructor.selected_in("A", &layer(&app).id), &BTreeSet::from([0, 2]));
        assert_eq!(app.state.tab().unwrap().selection, points, "the node selection stays");
        // The other direction cannot be selected.
        assert!(app.run("hint.tt.selectInstructions", &json!({"instructions": [8]})).is_err());
        assert_eq!(app.state.tt_instructor.selected_in("A", &layer(&app).id), &BTreeSet::from([0, 2]));
        // Switching the direction clears the selection.
        app.run("hint.tt.toggleDirection", &json!({"direction": "horizontal"})).unwrap();
        assert!(app.state.tt_instructor.selected_in("A", &layer(&app).id).is_empty());
        app.run("hint.tt.selectInstructions", &json!({"instructions": [8, 9]})).unwrap();
        app.run("hint.tt.selectInstructions", &json!({"instructions": [9], "mode": "remove"})).unwrap();
        assert_eq!(app.state.tt_instructor.selected_in("A", &layer(&app).id), &BTreeSet::from([8]));
    }

    fn key(app: &mut FontcraftApp, name: &str) -> Value {
        control::handle(app, &json!({"id": 1, "method": "ui.key", "params": {"key": name}}))
    }

    #[test]
    fn each_instructor_key_equals_its_command_and_letters_do_not_switch_tools() {
        let mut app = squares();
        edit(&mut app);
        let mut headless = squares();
        app.run("selection.set", &json!({"points": [[0, 0], [0, 3]]})).unwrap();
        key(&mut app, "a");
        headless.run("hint.tt.snapPoint", &json!({"glyph": "A", "nodes": [[0, 0], [0, 3]]})).unwrap();
        assert_eq!(layer(&app), layer(&headless), "A adds what Snap Point adds");
        assert_eq!(app.tools.toolbox.active_id(), INSTRUCTOR, "A does not switch to the Annotation tool");

        key(&mut app, "x");
        assert_eq!(app.state.tt_instructor.direction, TtDirection::Horizontal);
        app.run("selection.set", &json!({"points": [[1, 0], [1, 1]]})).unwrap();
        key(&mut app, "s");
        headless.run("hint.tt.addStemHint", &json!({"glyph": "A", "nodes": [[1, 0], [1, 1]], "direction": "horizontal"})).unwrap();
        assert_eq!(layer(&app), layer(&headless), "S in the horizontal direction adds a horizontal Stem");

        for (name, ppm) in [(".", 13), ("+", 14), (",", 13), ("-", 12)] {
            key(&mut app, name);
            assert_eq!(app.state.tt_instructor.ppm, ppm, "{name}");
        }
        for name in ["v", "h", "t", "z", "l"] {
            key(&mut app, name);
        }
        assert_eq!(app.tools.toolbox.active_id(), INSTRUCTOR);
        assert_eq!(layer(&app), layer(&headless), "unbound letters change nothing");
    }

    #[test]
    fn pixel_size_stays_within_its_limits_and_point_numbers_toggle() {
        let mut app = squares();
        edit(&mut app);
        app.state.tt_instructor.ppm = MAX_PPM;
        assert_eq!(app.run("hint.tt.increasePixelSize", &json!({})).unwrap(), json!({"ppm": MAX_PPM}));
        app.state.tt_instructor.ppm = MIN_PPM;
        assert_eq!(app.run("hint.tt.decreasePixelSize", &json!({})).unwrap(), json!({"ppm": MIN_PPM}));
        assert_eq!(clamp_ppm(i64::MIN), MIN_PPM);
        assert!(app.run("hint.tt.toggleDirection", &json!({"direction": "diagonal"})).is_err());
        assert_eq!(app.state.tt_instructor.direction, TtDirection::Vertical);

        assert_eq!(app.run("hint.tt.showPointIndexes", &json!({})).unwrap(), json!({"on": true}));
        let w = Window::new();
        w.frame(&mut app, Vec::new());
        let shapes = w.frame(&mut app, Vec::new());
        let t = app.tokens.tt.clone();
        let labels = shapes.iter().filter(|s| matches!(s, Shape::Text(text) if text.fallback_color == t.index_text)).count();
        assert_eq!(labels, 12, "one number per on-curve node");
        assert_eq!(app.run("hint.tt.showPointIndexes", &json!({"on": false})).unwrap(), json!({"on": false}));
    }
}
