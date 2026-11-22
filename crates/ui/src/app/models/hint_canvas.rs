//! The hint layers' view model and the Info box's hint variants.
//!
//! [`HintCanvas`] holds what `canvas::hinting` draws: the edited layer's PostScript hints while Show Hints is on, and
//! for the TrueType Instructor the on-curve nodes, the instructions in font units, the point numbers and the pixel
//! preview. [`PsHintBox`] and [`TtBox`] are the Info box while hints are selected and while the Instructor is the
//! chosen tool (`panels::info_box_ps_hint`, `panels::info_box_tt`).

use std::sync::Arc;

use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::compile::Options;
use fontcraft_engine::hinting::settings::main_master;
use fontcraft_engine::hinting::tt::delta_column_key;
use fontcraft_engine::hinting::{
    DeltaDir, Direction, HintSettings, NodeRef, Rounding, StemRef, TtInstruction, TtKind, TtPoint, ZoneRef, tt_instructions,
};
use fontcraft_engine::model::{Font, Layer, NodeKind, Path, Shape, Target};
use fontcraft_geom::Point;
use fontcraft_tools::Selection;
use fontcraft_tools::hint_drag::{HintKind, HintMarks};

use super::ProjectCx;
use crate::canvas::hinting::pixels::{self, PixelPreview};
use crate::canvas::hinting::{INSTRUCTOR, RenderingIntent, ps_marks};
use crate::dialogs::tt::deltas::{self, Cell, Column, DeltaTable};

/// What the hint layers draw.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct HintCanvas {
    /// The edited layer's PostScript hints; not `shown` while Show Hints is off.
    #[plain_data(assume = "plain values: numbers, strings and lists")]
    pub ps: HintMarks,
    /// The edited layer's TrueType picture; `None` without an edited glyph or while the background is edited.
    pub tt: Option<TtCanvas>,
}

/// The look of an instruction badge, by instruction type.
#[derive(PlainData, Clone, Copy, Debug, PartialEq, Eq)]
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
}

/// One TrueType instruction in font units.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct TtMark {
    /// Its position in the layer's instruction list.
    pub index: usize,
    pub kind: BadgeKind,
    /// The instruction moves points in x.
    pub horizontal: bool,
    /// The point it moves.
    pub target: [f64; 2],
    /// The node it moves, as `[path, node]`, when it moves a node.
    pub target_node: Option<(usize, usize)>,
    /// Its reference points: the origin of Stem and Shift, both references of Interpolate.
    pub references: Vec<[f64; 2]>,
}

/// One on-curve node of the edited layer.
#[derive(PlainData, Clone, Copy, Debug, PartialEq)]
pub struct TtNode {
    /// The shape and node index, as the selection names them.
    pub shape: usize,
    pub node: usize,
    pub at: [f64; 2],
}

/// The edited layer's TrueType picture.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct TtCanvas {
    pub glyph: String,
    pub layer: String,
    /// The Instructor is the chosen tool: nodes, badges and the pixel preview show.
    pub instructor: bool,
    pub nodes: Vec<TtNode>,
    /// Every node and handle of the layer's paths as `[path, node]` with its position: where a badge may drop.
    pub drop_points: Vec<((usize, usize), [f64; 2])>,
    pub instructions: Vec<TtMark>,
    /// Each on-curve node's point number, while Show Point Indexes is on.
    pub numbers: Vec<([f64; 2], u16)>,
    #[plain_data(assume = "an immutable preview behind an Arc: owned numbers and paths")]
    pub preview: Option<Arc<PixelPreview>>,
    /// Why the preview cannot be drawn.
    pub preview_error: Option<String>,
}

/// The point numbers of a layer: `paths[path][node]`, paths counted without components.
type PointNumbers = Vec<Vec<Option<u16>>>;

/// What the point numbers were computed for.
type NumbersKey = ((u64, u64), String, String);

/// What the projection keeps between runs: the pixel preview and the point numbers, both slow to compute.
#[derive(Default)]
pub struct HintCache {
    preview: pixels::Cache,
    numbers: Option<(NumbersKey, PointNumbers)>,
}

pub fn project(model: &mut Tracked<HintCanvas>, cache: &mut HintCache, cx: &mut ProjectCx<'_>) {
    let ps = ps_marks(cx.session, cx.view);
    let tt = tt_canvas(cache, cx);
    super::set(model, HintCanvas { ps, tt });
}

fn tt_canvas(cache: &mut HintCache, cx: &mut ProjectCx<'_>) -> Option<TtCanvas> {
    if cx.view.edit_background {
        return None;
    }
    let instructor = cx.view.tool == INSTRUCTOR;
    let state = &cx.view.hinting.instructor;
    if !instructor && !state.point_indexes {
        return None;
    }
    let target = crate::canvas::tools::target(cx.session, cx.view)?;
    let doc = cx.session.active().map(|d| (d.id, d.revision)).unwrap_or_default();
    let options = pixels::options(cx.session);
    let font = cx.session.font().ok()?;
    let layer = font.glyph(&target.glyph)?.layers.iter().find(|l| l.id == target.layer)?;
    let mut out = TtCanvas {
        glyph: target.glyph.clone(),
        layer: target.layer.clone(),
        instructor,
        nodes: Vec::new(),
        drop_points: Vec::new(),
        instructions: Vec::new(),
        numbers: Vec::new(),
        preview: None,
        preview_error: None,
    };
    if state.point_indexes {
        out.numbers = point_numbers(cache, font, doc, &target.glyph, layer);
    }
    if !instructor {
        return Some(out);
    }
    for (s, shape) in layer.shapes.iter().enumerate() {
        let Shape::Path(path) = shape else { continue };
        for (n, node) in path.nodes.iter().enumerate() {
            if node.kind != NodeKind::OffCurve && node.x.is_finite() && node.y.is_finite() {
                out.nodes.push(TtNode { shape: s, node: n, at: [node.x, node.y] });
            }
        }
    }
    let paths = layer.shapes.iter().filter_map(|s| if let Shape::Path(p) = s { Some(p) } else { None });
    for (p, path) in paths.enumerate() {
        for (n, node) in path.nodes.iter().enumerate() {
            if node.x.is_finite() && node.y.is_finite() {
                out.drop_points.push(((p, n), [node.x, node.y]));
            }
        }
    }
    if let Ok(list) = tt_instructions(layer) {
        for (index, (_, ins)) in list.iter().enumerate() {
            let Some(at) = point_at(layer, &ins.target) else { continue };
            let references = [ins.origin, ins.other].iter().flatten().filter_map(|p| point_at(layer, p)).map(|p| [p.x, p.y]).collect();
            let target_node = match ins.target {
                TtPoint::Node(r) => Some((r.path, r.node)),
                _ => None,
            };
            out.instructions.push(TtMark {
                index,
                kind: BadgeKind::of(&ins.kind),
                horizontal: ins.direction == Direction::Horizontal,
                target: [at.x, at.y],
                target_node,
                references,
            });
        }
    }
    match cache.preview.preview(font, || options, doc, &layer.master, &target.glyph, state.instance.as_deref(), state.ppm, state.intent) {
        Ok(preview) => out.preview = Some(preview),
        Err(e) => out.preview_error = Some(e),
    }
    Some(out)
}

/// The point numbers of `layer`'s master outline, as `hint.tt.getPointIndexes` gives them, each beside its on-curve
/// node: empty for a composite or when the outline cannot be prepared.
fn point_numbers(cache: &mut HintCache, font: &Font, doc: (u64, u64), glyph: &str, layer: &Layer) -> Vec<([f64; 2], u16)> {
    let key = (doc, glyph.to_string(), layer.id.clone());
    let numbers = match &cache.numbers {
        Some((k, numbers)) if *k == key => numbers.clone(),
        _ => {
            let target = Target::Master(layer.master.clone());
            let options = Options { remove_overlap: true, ..Options::default() };
            let numbers =
                fontcraft_engine::compile::points::point_indexes(font, &target, glyph, &options).ok().flatten().map(|m| m.paths).unwrap_or_default();
            cache.numbers = Some((key, numbers.clone()));
            numbers
        }
    };
    let paths = layer.shapes.iter().filter_map(|s| if let Shape::Path(p) = s { Some(p) } else { None });
    let mut out = Vec::new();
    for (path, row) in paths.zip(&numbers) {
        for (node, number) in path.nodes.iter().zip(row) {
            if let Some(number) = number
                && node.kind != NodeKind::OffCurve
            {
                out.push(([node.x, node.y], *number));
            }
        }
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

// ── the Info box ────────────────────────────────────────────────────────────────────────────────

/// What a PostScript hint is, as the Info box names it.
#[derive(PlainData, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PsHintKind {
    Horizontal,
    Vertical,
    GhostTop,
    GhostBottom,
}

/// The Info box of the first selected PostScript hint.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct PsHintBox {
    pub glyph: String,
    pub layer: String,
    pub index: usize,
    pub kind: PsHintKind,
    pub origin: Option<f64>,
    /// `None` for a ghost.
    pub size: Option<f64>,
    /// How many hints are selected.
    pub count: usize,
    pub warnings: Vec<String>,
}

/// The Info box while hints are selected and shown; `None` otherwise.
pub fn ps_hint_box(cx: &mut ProjectCx<'_>, selection: &Selection) -> Option<PsHintBox> {
    if selection.hints.is_empty() {
        return None;
    }
    let marks = ps_marks(cx.session, cx.view);
    let target = crate::canvas::tools::target(cx.session, cx.view)?;
    let selected = marks.selected(selection);
    let h = marks.get(*selected.first()?)?;
    let kind = match (h.kind, h.horizontal) {
        (HintKind::GhostTop, _) => PsHintKind::GhostTop,
        (HintKind::GhostBottom, _) => PsHintKind::GhostBottom,
        (HintKind::Stem, true) => PsHintKind::Horizontal,
        (HintKind::Stem, false) => PsHintKind::Vertical,
    };
    Some(PsHintBox {
        glyph: target.glyph,
        layer: target.layer,
        index: h.index,
        kind,
        origin: h.origin,
        size: h.size,
        count: selected.len(),
        warnings: h.warnings.clone(),
    })
}

/// A choice in the zone or stem pop-up.
#[derive(PlainData, Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Auto,
    None,
    Named(String),
}

impl Choice {
    pub fn param(&self) -> &str {
        match self {
            Choice::Auto => "auto",
            Choice::None => "none",
            Choice::Named(name) => name,
        }
    }
}

/// The rounding modes as commands name them, in the order of the buttons.
pub const ROUNDINGS: [(Rounding, &str); 5] =
    [(Rounding::Round, "round"), (Rounding::Up, "up"), (Rounding::Down, "down"), (Rounding::NoRounding, "none"), (Rounding::GdiOnly, "gdiOnly")];

/// The Info box while the TrueType Instructor is the chosen tool.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct TtBox {
    pub glyph: String,
    pub layer: String,
    pub vertical: bool,
    pub ppm: u16,
    #[plain_data(assume = "a fieldless enum")]
    pub intent: RenderingIntent,
    /// Positions of the selected instructions by the commands that take them.
    pub snaps: Vec<usize>,
    pub stems: Vec<usize>,
    pub rounded: Vec<usize>,
    pub deltas: Vec<usize>,
    /// The shared zone, stem and rounding; `None` when the selection differs.
    pub zone: Option<Choice>,
    pub stem: Option<Choice>,
    /// The parameter of the shared rounding mode.
    pub rounding: Option<String>,
    /// The names the pop-ups offer.
    pub zones: Vec<String>,
    pub stem_names: Vec<String>,
    /// The Delta table of the selected Deltas.
    #[plain_data(assume = "plain values: strings, numbers and a map of fieldless enums")]
    pub table: Option<DeltaTable>,
}

/// The names of the TT stems that apply to `glyph` and suit Stem instructions of `direction`: a vertical Stem
/// instruction measures a horizontal stem, and the other way round.
pub fn stem_names_for(font: &Font, settings: &HintSettings, glyph: &str, direction: Direction) -> Vec<String> {
    settings
        .tt_stems_for(font, glyph)
        .into_iter()
        .filter_map(|i| settings.tt_stems.get(i))
        .filter(|st| st.horizontal == (direction == Direction::Vertical))
        .map(|st| st.name.clone())
        .collect()
}

/// The stem choices of the instruction menu's Set Stem Reference for instructions `chosen` of layer `layer` of
/// `glyph`: Auto, No Stem, then the TT stems their direction takes (none when the Stem instructions differ in
/// direction), each with whether every chosen Stem instruction has it. Empty when a chosen instruction is not a Stem.
pub fn stem_choices(font: &Font, glyph: &str, layer: &str, chosen: &std::collections::BTreeSet<usize>) -> Vec<(Choice, bool)> {
    let Some(l) = font.glyph(glyph).and_then(|g| g.layers.iter().find(|l| l.id == layer)) else { return Vec::new() };
    let list: Vec<TtInstruction> = tt_instructions(l).map(|v| v.into_iter().map(|(_, t)| t).collect()).unwrap_or_default();
    let selected: Vec<&TtInstruction> = chosen.iter().filter_map(|&i| list.get(i)).collect();
    if selected.is_empty() || !selected.iter().all(|t| matches!(t.kind, TtKind::Stem { .. })) {
        return Vec::new();
    }
    let settings = HintSettings::for_master(font, &main_master(font)).ok();
    let stem_name = |i: usize| settings.as_ref().and_then(|s| s.tt_stems.get(i)).map(|z| z.name.clone());
    let current = shared(selected.iter().filter_map(|t| match &t.kind {
        TtKind::Stem { stem: StemRef::Auto, .. } => Some(Choice::Auto),
        TtKind::Stem { stem: StemRef::NoStem, .. } => Some(Choice::None),
        TtKind::Stem { stem: StemRef::Index(i), .. } => stem_name(*i).map(Choice::Named),
        _ => None,
    }));
    let names = match (&settings, shared(selected.iter().map(|t| t.direction))) {
        (Some(s), Some(d)) => stem_names_for(font, s, glyph, d),
        _ => Vec::new(),
    };
    [Choice::Auto, Choice::None]
        .into_iter()
        .chain(names.into_iter().map(Choice::Named))
        .map(|c| {
            let on = current.as_ref() == Some(&c);
            (c, on)
        })
        .collect()
}

/// The value the selected instructions share, if they share one.
fn shared<T: PartialEq>(mut values: impl Iterator<Item = T>) -> Option<T> {
    let first = values.next()?;
    let mut rest = values;
    rest.all(|v| v == first).then_some(first)
}

/// The Info box of the Instructor; `None` while another tool is chosen or no glyph is edited.
pub fn tt_box(cx: &mut ProjectCx<'_>) -> Option<TtBox> {
    if cx.view.tool != INSTRUCTOR {
        return None;
    }
    let target = crate::canvas::tools::target(cx.session, cx.view)?;
    let state = &cx.view.hinting.instructor;
    let font = cx.session.font().ok()?;
    let layer = font.glyph(&target.glyph)?.layers.iter().find(|l| l.id == target.layer)?;
    let list: Vec<TtInstruction> = tt_instructions(layer).map(|l| l.into_iter().map(|(_, t)| t).collect()).unwrap_or_default();
    let chosen = state.selected_in(&target.glyph, &target.layer);
    let selected: Vec<(usize, &TtInstruction)> = chosen.iter().filter_map(|&i| list.get(i).map(|t| (i, t))).collect();
    let of = |pick: fn(&TtKind) -> bool| selected.iter().filter(|(_, t)| pick(&t.kind)).map(|(i, _)| *i).collect::<Vec<_>>();
    let snaps = of(|k| matches!(k, TtKind::Snap { .. }));
    let stems = of(|k| matches!(k, TtKind::Stem { .. }));
    let deltas = of(|k| matches!(k, TtKind::Delta { .. }));
    let rounded = of(|k| !matches!(k, TtKind::Delta { .. }));
    let settings = HintSettings::for_master(font, &main_master(font)).ok();
    let zone_name = |i: usize| settings.as_ref().and_then(|s| s.tt_zones.get(i)).map_or_else(|| format!("{}", i + 1), |z| z.name.clone());
    let stem_name = |i: usize| settings.as_ref().and_then(|s| s.tt_stems.get(i)).map_or_else(|| format!("{}", i + 1), |z| z.name.clone());
    let zone = shared(selected.iter().filter_map(|(_, t)| match &t.kind {
        TtKind::Snap { zone } => Some(match zone {
            ZoneRef::Auto => Choice::Auto,
            ZoneRef::NoZone => Choice::None,
            ZoneRef::Index(i) => Choice::Named(zone_name(*i)),
        }),
        _ => None,
    }));
    let stem = shared(selected.iter().filter_map(|(_, t)| match &t.kind {
        TtKind::Stem { stem, .. } => Some(match stem {
            StemRef::Auto => Choice::Auto,
            StemRef::NoStem => Choice::None,
            StemRef::Index(i) => Choice::Named(stem_name(*i)),
        }),
        _ => None,
    }));
    let rounding = shared(selected.iter().filter(|(_, t)| !matches!(t.kind, TtKind::Delta { .. })).map(|(_, t)| t.rounding))
        .and_then(|r| ROUNDINGS.iter().find(|(m, _)| *m == r).map(|(_, p)| p.to_string()));
    let direction_of = |pick: fn(&TtKind) -> bool| shared(selected.iter().filter(|(_, t)| pick(&t.kind)).map(|(_, t)| t.direction));
    let zones = match (&settings, direction_of(|k| matches!(k, TtKind::Snap { .. }))) {
        (Some(s), Some(Direction::Vertical)) => {
            s.tt_zones_for(font, &target.glyph).into_iter().filter_map(|i| s.tt_zones.get(i)).map(|z| z.name.clone()).collect()
        }
        _ => Vec::new(),
    };
    let stem_names = match (&settings, direction_of(|k| matches!(k, TtKind::Stem { .. }))) {
        (Some(s), Some(d)) => stem_names_for(font, s, &target.glyph, d),
        _ => Vec::new(),
    };
    let table = (!deltas.is_empty()).then(|| delta_table(font, &selected));
    Some(TtBox {
        glyph: target.glyph,
        layer: target.layer,
        vertical: state.direction == fontcraft_tools::tt_instructor::TtDirection::Vertical,
        ppm: state.ppm,
        intent: state.intent,
        snaps,
        stems,
        rounded,
        deltas,
        zone,
        stem,
        rounding,
        zones,
        stem_names,
        table,
    })
}

/// The Delta table of the selected Delta instructions: a column per static instance (per master without one, instances
/// or masters at one location sharing a column), then any column a selected Delta stores that matches neither.
fn delta_table(font: &Font, selected: &[(usize, &TtInstruction)]) -> DeltaTable {
    fn add(key: String, name: &str, columns: &mut Vec<Column>) {
        if let Some(c) = columns.iter_mut().find(|c| c.key == key) {
            c.label = format!("{}, {name}", c.label);
        } else if columns.len() < deltas::MAX_COLUMNS {
            columns.push(Column { key, label: name.to_string() });
        }
    }
    let mut columns: Vec<Column> = Vec::new();
    let statics: Vec<_> = font.instances.iter().filter(|i| i.kind.as_deref() != Some("variable")).collect();
    if statics.is_empty() {
        for m in font.masters.iter() {
            add(delta_column_key(&m.location), &m.name, &mut columns);
        }
    } else {
        for i in statics {
            add(delta_column_key(&i.location), &i.name, &mut columns);
        }
    }
    let cells_of: Vec<_> = selected
        .iter()
        .filter_map(|(_, t)| match &t.kind {
            TtKind::Delta { cells } => Some(cells),
            _ => None,
        })
        .collect();
    for cells in &cells_of {
        for key in cells.keys() {
            if !columns.iter().any(|c| &c.key == key) && columns.len() < deltas::MAX_COLUMNS {
                columns.push(Column { key: key.clone(), label: key.clone() });
            }
        }
    }
    let mut table = DeltaTable { title: String::new(), columns, ..DeltaTable::default() };
    for (index, column) in table.columns.iter().enumerate() {
        for ppm in deltas::FIRST_PPM..=deltas::LAST_PPM {
            let values = cells_of.iter().map(|c| c.get(&column.key).and_then(|sizes| sizes.get(&ppm)).copied());
            let cell = match shared(values) {
                Some(None) => continue,
                Some(Some(DeltaDir::Up)) => Cell::Up,
                Some(Some(DeltaDir::Down)) => Cell::Down,
                None => Cell::Mixed,
            };
            table.cells.insert((index, ppm), cell);
        }
    }
    table
}
