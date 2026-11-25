//! The canvas overlays: guides and grids, the Measurement tool's labels, annotations, the display aids, the auto-meter
//! and audit markers (`docs/spec/03a` § 20, `docs/spec/03b`).
//!
//! | Module | Paints | Layer |
//! |---|---|---|
//! | [`guides`] | the grid, metric lines that parameters set, CJK and design grids | `grid` |
//! | [`guides`] | local and global guides, their knobs and names, measurement guides' crossings | `guides` |
//! | [`power_guides`] | guide thickness edges, guide markers, power guide rhombi and link lines | `powerGuides` |
//! | [`audit`] | the segments, paths and areas audit results report | `auditHighlights` |
//! | [`annotations`] | the edited layer's annotations | `annotations` |
//! | [`aids`] | ghost outline, Tunni lines, tension, curve lengths, indexes, the metrics beam, the Lock Outline badge | `editAids` |
//! | [`autometer`] | stem and counter widths, corner angles, the quick measurement | `autometer` |
//! | [`guides`] | the Measurement tool's coordinates, deltas, overlaps and ruler crossings | `measurement` |
//! | [`audit`] | the rings and badges of audit results | `auditMarkers` |
//! | [`suggest`] | Suggest Distance's parallel outlines near the pointer while a drag runs | `suggestOutlines` |
//!
//! The switches are view state ([`Switches`], `shell::overlays`). What the layers paint comes from engine queries
//! (`guide.displayLines`, `measure.ruler`, `measure.coordinates`, `audit.run`, `audit.results`) and the auto-meter,
//! run by [`refresh`] after `drain` and only when what they read changed; painting reads [`OverlayData`] alone.

pub mod aids;
pub mod annotations;
pub mod audit;
pub mod autometer;
pub mod guides;
pub mod power_guides;
pub mod suggest;
pub mod tokens;

use std::collections::BTreeMap;

use egui::{Align2, Color32, FontId, Pos2};
use fontcraft_engine::Session;
use fontcraft_engine::model::{Font, Guide, LayerSide, Master, Shape};
use fontcraft_geom::{BezPath, Point};
use fontcraft_tools::{Action, Output, Overlay, OverlayKind, PointerKind};
use serde_json::{Value, json};

use super::tools::{Target, ToolInput};
use crate::app::{FontCraftApp, ViewState};

craft::ui::words! {
    /// The words the overlays paint.
    pub struct OverlayWords {
        /// The badge in the canvas's corner while Lock Outline is on.
        locked: "fontcraftOverlays.locked" = "Outline locked";
        /// The name of the field that edits a text note on the canvas.
        note: "fontcraftOverlays.note" = "Note";
        /// The name of the field that names an anchor on the canvas.
        anchor: "fontcraftOverlays.anchor" = "Anchor Name";
        /// Under that field: no name was typed.
        anchor_empty: "fontcraftOverlays.anchorEmpty" = "Type a name, or press Escape to keep the old one";
        /// Under that field: the layer has an anchor of the typed name.
        anchor_taken: "fontcraftOverlays.anchorTaken" = "Another anchor of this layer has this name";
    }
}

/// One label a layer paints.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub at: Pos2,
    pub align: Align2,
    pub text: String,
    pub size: f32,
    pub color: Color32,
}

/// What one overlay layer paints: shapes, then labels over them.
#[derive(Default)]
pub struct Drawn {
    pub shapes: Vec<egui::Shape>,
    pub labels: Vec<Label>,
}

impl Drawn {
    pub fn label(&mut self, at: Pos2, align: Align2, text: String, size: f32, color: Color32) {
        if at.x.is_finite() && at.y.is_finite() {
            self.labels.push(Label { at, align, text, size, color });
        }
    }

    pub fn paint(self, cx: &super::layers::PaintCx<'_>) {
        cx.painter.extend(self.shapes);
        for l in self.labels {
            cx.painter.text(l.at, l.align, l.text, FontId::proportional(l.size), l.color);
        }
    }
}

/// What a paint layer of this module reads, beside the rest of `PaintCx`.
pub struct Painted<'a> {
    pub data: &'a OverlayData,
    pub words: &'a OverlayWords,
}

/// The labels `view.showIndexes` shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Indexes {
    pub points: bool,
    pub segments: bool,
    pub contours: bool,
    pub components: bool,
    pub anchors: bool,
}

impl Indexes {
    pub fn any(&self) -> bool {
        self.points || self.segments || self.contours || self.components || self.anchors
    }

    pub fn all(on: bool) -> Indexes {
        Indexes { points: on, segments: on, contours: on, components: on, anchors: on }
    }
}

/// The overlays' switches: the View menu's display aids and Measure items, the snapping suggestions, Lock Outline and
/// the audit switches.
#[derive(Clone, Debug, PartialEq)]
pub struct Switches {
    pub tunni_lines: bool,
    pub curve_tension: bool,
    pub indexes: Indexes,
    pub curve_length: bool,
    pub details_across_glyphs: bool,
    pub lock_outline: bool,
    /// `view.showMetricsBeam`.
    pub beam: bool,
    /// The beam's height per document id, in font units; a document without one uses half the x-height.
    pub beam_heights: BTreeMap<u64, f64>,
    pub suggest: fontcraft_tools::suggestions::Switches,
    pub stem_widths: bool,
    pub counter_widths: bool,
    pub corner_angles: bool,
    pub quick_measurement: bool,
    /// Live audit: the outlines profile runs on the edited layer after each change.
    pub live_audit: bool,
    pub audit_markers: bool,
}

impl Default for Switches {
    fn default() -> Self {
        Switches {
            tunni_lines: false,
            curve_tension: false,
            indexes: Indexes::default(),
            curve_length: false,
            details_across_glyphs: false,
            lock_outline: false,
            beam: false,
            beam_heights: BTreeMap::new(),
            suggest: Default::default(),
            stem_widths: false,
            counter_widths: false,
            corner_angles: false,
            quick_measurement: false,
            live_audit: false,
            audit_markers: true,
        }
    }
}

/// What the overlay layers paint, in the edited glyph's font units.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverlayData {
    pub switches: Switches,
    /// `guide.displayLines` of the edited layer.
    pub lines: Option<guides::DisplayLines>,
    /// The global guides of the edited layer's master.
    pub global_guides: Vec<Guide>,
    /// The global guides whose filter leaves the edited glyph out.
    pub out_of_scope: Vec<usize>,
    /// The crossings of each measurement guide with the outline.
    pub guide_rulers: Vec<(fontcraft_tools::GuideRef, guides::Ruler)>,
    /// The Measurement tool's coordinates of the edited layer.
    pub coordinates: Option<guides::Coordinates>,
    /// The measurement ruler's ends and crossings.
    pub ruler: Option<(Point, Point, guides::Ruler)>,
    /// The auto-meter's stems, counters and corners (`autometer.measure`).
    pub meter: Option<Value>,
    /// The quick measurement under the pointer: from, to and the distance.
    pub quick: Option<([f64; 2], [f64; 2], f64)>,
    /// The audit results of the edited layer.
    pub markers: Vec<audit::Marker>,
    /// The shapes the running interaction changed, as they were when it began.
    pub ghost: Option<BezPath>,
    /// The metrics beam's height, while it shows.
    pub beam: Option<f64>,
    /// The text note whose field is open: its text is the field's.
    pub editing: Option<usize>,
    /// The shown reporter modules' drawings of the edited layer (`canvas::reporters`).
    pub reporters: Vec<super::reporters::ReporterDrawing>,
    /// Suggest Distance's parallel outlines near the pointer while a drag runs, as polylines ([`suggest::outlines`]).
    pub suggest_outlines: Vec<Vec<Point>>,
    /// The reference font's glyph in the edited glyph's font units (`canvas::reference_font`).
    pub reference: Option<std::sync::Arc<BezPath>>,
}

/// The overlays' side of the app.
#[derive(Debug, Default)]
pub struct Overlays {
    pub switches: Switches,
    /// The text note being typed on the canvas.
    pub note: Option<super::annotation_editor::NoteEdit>,
    /// The anchor name being typed on the canvas.
    pub anchor: Option<super::anchor_editor::AnchorEdit>,
    /// The pointer over the canvas in font units, and whether Shift was held.
    pub hover: Option<(Point, bool)>,
    pub data: OverlayData,
    /// How many times live audit has run.
    pub audit_runs: u64,
    /// The reference font's outline and what it was built from (`canvas::reference_font`).
    pub reference: super::reference_font::ReferenceCache,
    base: Option<BaseKey>,
    ruler: Option<(BaseKey, Point, Point)>,
    quick: Option<(BaseKey, Point, bool)>,
    /// What Suggest Distance's outlines were last found for: the base, the pointer and the zoom.
    suggest: Option<(BaseKey, Point, f64)>,
}

/// What the data read from the engine depends on.
#[derive(Clone, Debug, PartialEq)]
struct BaseKey {
    doc: Option<(u64, u64)>,
    target: Target,
    guides: bool,
    measuring: bool,
    switches: Switches,
    corners: bool,
    interaction: bool,
}

/// Corner angles show from this many screen points per font unit.
pub const CORNER_ZOOM: f64 = 1.5;

/// Brings [`Overlays::data`] up to date and finishes a text note the editor closed. Runs after `drain`.
pub fn refresh(app: &mut FontCraftApp) {
    super::annotation_editor::finish(app);
    let Some(target) = super::tools::target(&app.session, &app.view) else {
        app.overlays.data = OverlayData { switches: app.overlays.switches.clone(), ..OverlayData::default() };
        app.overlays.base = None;
        return;
    };
    let ruler = app.tools.view.overlays.iter().find_map(|o| match o {
        Overlay::Line { kind: OverlayKind::Ruler, from, to } => Some((*from, *to)),
        _ => None,
    });
    let sw = app.overlays.switches.clone();
    let base = BaseKey {
        doc: app.session.active().map(|d| (d.id, d.revision)),
        target: target.clone(),
        guides: app.view.display.guides,
        measuring: app.tools.toolbox.base_id() == "measurement" || ruler.is_some(),
        corners: sw.corner_angles && app.view.canvas.zoom >= CORNER_ZOOM,
        switches: sw,
        interaction: app.session.interaction().is_some(),
    };
    let fresh = app.overlays.base.as_ref() != Some(&base);
    if fresh {
        let mut data = OverlayData { switches: base.switches.clone(), ..OverlayData::default() };
        read_base(&mut app.session, &base, &mut data, &mut app.overlays.audit_runs);
        data.beam = beam_height(&app.session, &app.view, &base.switches);
        app.overlays.data = data;
        app.overlays.base = Some(base.clone());
    }
    match ruler.filter(|_| base.measuring) {
        Some((from, to)) => {
            let key = (base.clone(), from, to);
            if fresh || app.overlays.ruler.as_ref() != Some(&key) {
                let params = with(&target_params(&base.target), &[("from", json!([from.x, from.y])), ("to", json!([to.x, to.y]))]);
                app.overlays.data.ruler = query(&mut app.session, "measure.ruler", params).map(|v| (from, to, guides::Ruler::read(&v)));
                app.overlays.ruler = Some(key);
            }
        }
        None => {
            app.overlays.data.ruler = None;
            app.overlays.ruler = None;
        }
    }
    app.overlays.data.editing = app.overlays.note.as_ref().map(|n| n.index);
    match app.overlays.hover.filter(|_| base.switches.quick_measurement) {
        Some((at, shift)) => {
            let key = (base.clone(), at, shift);
            if fresh || app.overlays.quick.as_ref() != Some(&key) {
                app.overlays.data.quick = app.session.font().ok().and_then(|font| autometer::quick(font, &base.target, at, shift));
                app.overlays.quick = Some(key);
            }
        }
        None => {
            app.overlays.data.quick = None;
            app.overlays.quick = None;
        }
    }
    match app.overlays.hover.filter(|_| base.switches.suggest.distance && base.interaction) {
        Some((at, _)) => {
            let key = (base.clone(), at, app.view.canvas.zoom);
            if fresh || app.overlays.suggest.as_ref() != Some(&key) {
                app.overlays.data.suggest_outlines = suggest::outlines(app);
                app.overlays.suggest = Some(key);
            }
        }
        None => {
            app.overlays.data.suggest_outlines = Vec::new();
            app.overlays.suggest = None;
        }
    }
}

/// Reads what the overlays need from the engine for `key`.
fn read_base(session: &mut Session, key: &BaseKey, data: &mut OverlayData, audit_runs: &mut u64) {
    let target = target_params(&key.target);
    if key.guides {
        if let Ok(font) = session.font() {
            data.out_of_scope = guides::out_of_scope(session, font, &key.target);
            let master = font.glyph(&key.target.glyph).and_then(|g| g.layer(&key.target.layer)).map(|l| l.master.clone());
            data.global_guides = font.masters.iter().find(|m| Some(&m.id) == master.as_ref()).map(|m| m.guides.clone()).unwrap_or_default();
        }
        data.lines = query(session, "guide.displayLines", Value::Object(target.clone())).map(|v| guides::DisplayLines::read(&v));
        for guide in guides::measured(session, &key.target, &data.out_of_scope) {
            let params = with(&target, &[("guide", json!({"scope": guide.scope.as_str(), "index": guide.index}))]);
            if let Some(r) = query(session, "measure.ruler", params).map(|v| guides::Ruler::read(&v)) {
                data.guide_rulers.push((guide, r));
            }
        }
    }
    if key.measuring {
        let mut placement = json!({"glyph": key.target.glyph, "layer": key.target.layer});
        if key.target.side == LayerSide::Background
            && let Some(o) = placement.as_object_mut()
        {
            o.insert("side".into(), json!("background"));
        }
        let answer = session.query("measure.coordinates", &with(&target, &[("placements", json!([placement]))])).ok();
        data.coordinates = answer.as_ref().and_then(|v| v.get("placements")?.get(0)).map(guides::Coordinates::read);
    }
    let sw = &key.switches;
    if sw.stem_widths || sw.counter_widths || key.corners {
        data.meter = session.font().ok().and_then(|font| autometer::measure(font, &key.target, [sw.stem_widths, sw.counter_widths, key.corners]));
    }
    if sw.audit_markers || sw.live_audit {
        if key.interaction {
            // A drag runs an edit at every step: the audit waits for it to end and the markers hide meanwhile.
            data.markers = Vec::new();
        } else {
            data.markers = audit::read(session, &key.target, sw.live_audit, audit_runs);
        }
    }
    if key.interaction {
        data.ghost = session.font().ok().zip(session.interaction_start()).and_then(|(now, before)| ghost(now, before, &key.target));
    }
}

/// The shapes of the target's layer that differ from how they were in `before`, as one outline.
fn ghost(now: &Font, before: &Font, target: &Target) -> Option<BezPath> {
    let shapes = |font: &Font| -> Option<Vec<Shape>> {
        let layer = font.glyph(&target.glyph)?.layer(&target.layer)?;
        Some(match target.side {
            LayerSide::Foreground => layer.shapes.clone(),
            LayerSide::Background => layer.info.background.as_ref().map(|b| b.shapes.clone()).unwrap_or_default(),
        })
    };
    let (was, is) = (shapes(before)?, shapes(now)?);
    let master = before.glyph(&target.glyph)?.layer(&target.layer)?.master.clone();
    let mut out = BezPath::new();
    for (i, shape) in was.iter().enumerate() {
        if is.get(i) == Some(shape) {
            continue;
        }
        let part = match shape {
            Shape::Path(p) => p.to_bez().ok(),
            Shape::Component(c) => before.component_outline(c, &master).ok(),
            _ => None,
        };
        if let Some(part) = part {
            if out.elements().len().saturating_add(part.elements().len()) > aids::MAX_GHOST_ELEMENTS {
                break;
            }
            out.extend(part);
        }
    }
    (!out.elements().is_empty()).then_some(out)
}

/// The metrics beam's height while it shows: the height kept for the active document, else half the x-height of the
/// active layer's master (the current master without an edited glyph).
fn beam_height(session: &Session, view: &ViewState, sw: &Switches) -> Option<f64> {
    if !sw.beam {
        return None;
    }
    let doc = session.active()?;
    if let Some(y) = sw.beam_heights.get(&doc.id).copied().filter(|y| y.is_finite()) {
        return Some(y);
    }
    let master = super::tools::active_master(&doc.font, view)?;
    default_beam_height(&doc.font, master)
}

/// Half the x-height of `master`.
pub fn default_beam_height(font: &Font, master: &Master) -> Option<f64> {
    let x_height = font.metric(master, "x-height").map_or(500.0, |m| m.pos);
    let y = x_height / 2.0;
    y.is_finite().then_some(y)
}

/// The address of the target's layer, as the queries take it.
fn target_params(target: &Target) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    m.insert("glyph".into(), json!(target.glyph));
    m.insert("layer".into(), json!(target.layer));
    if target.side == LayerSide::Background {
        m.insert("side".into(), json!("background"));
    }
    m
}

fn with(target: &serde_json::Map<String, Value>, extra: &[(&str, Value)]) -> Value {
    let mut m = target.clone();
    for (k, v) in extra {
        m.insert((*k).into(), v.clone());
    }
    Value::Object(m)
}

fn query(session: &mut Session, id: &str, params: Value) -> Option<Value> {
    session.query(id, &params).ok()
}

/// What the overlays take from the tools' input: the pointer for the quick measurement, and a double-click on a text
/// note for its editor. Runs after the toolbox saw the input.
pub fn after_input(app: &mut FontCraftApp, input: &ToolInput) {
    match input {
        ToolInput::Pointer(ev) => {
            match ev.kind {
                PointerKind::Move | PointerKind::Drag => app.overlays.hover = Some((ev.pos, ev.mods.shift)),
                PointerKind::DoubleClick => super::annotation_editor::double_click(app, ev.pos),
                _ => {}
            }
            if ev.kind == PointerKind::Down {
                app.overlays.hover = Some((ev.pos, ev.mods.shift));
            }
        }
        ToolInput::Modifiers(mods) => {
            if let Some((_, shift)) = app.overlays.hover.as_mut() {
                *shift = mods.shift;
            }
        }
        ToolInput::Key(_) => {}
    }
}

/// The status line when Lock Outline refuses an edit.
pub const LOCKED: &str = "Lock Outline is on: the outline cannot be edited with the pointer or keys";

/// Most levels of `command.batch` inside one another that [`edits_outline`] looks into; a deeper step counts as an
/// outline edit.
const MAX_BATCH_DEPTH: usize = 8;

/// Command id prefixes that always edit the outline.
const OUTLINE_COMMANDS: &[&str] = &["path.", "nodes.", "tunni.", "smartComponent.", "filter."];

/// Command id prefixes that never do: the tools' selection, view and dialog commands, and the guides, anchors,
/// annotations and metrics that stay editable.
const FREE_COMMANDS: &[&str] = &[
    "tool.",
    "editView.",
    "view.",
    "dialog.",
    "selection.select",
    "selection.set",
    "guide.",
    "anchor.",
    "annotation.",
    "glyph.",
    "transform.setOrigin",
];

/// Parameters whose presence makes any other command an outline edit.
const OUTLINE_TARGETS: &[&str] = &["points", "shapes", "paths", "segments", "components", "component"];

/// `out` without its outline edits while Lock Outline is on; the status line says why they were dropped. Commands run
/// by name (menus, the control channel, agents) do not pass here and still edit the outline.
pub fn unless_locked(app: &mut FontCraftApp, mut out: Output) -> Output {
    if !app.overlays.switches.lock_outline {
        return out;
    }
    let before = out.actions.len();
    out.actions.retain(|a| match a {
        Action::Preview { invocation } | Action::Run { invocation } => !edits_outline(&invocation.id, &invocation.params, 0),
        _ => true,
    });
    if out.actions.len() != before {
        app.status = LOCKED.to_string();
    }
    out
}

/// Whether running `id` with `params` changes paths, nodes or components, as far as Lock Outline is concerned.
pub fn edits_outline(id: &str, params: &Value, depth: usize) -> bool {
    if id == "command.batch" {
        if depth >= MAX_BATCH_DEPTH {
            return true;
        }
        let Some(steps) = params.get("steps").and_then(Value::as_array) else { return false };
        return steps.iter().any(|step| {
            let command = step.get("command").and_then(Value::as_str).unwrap_or_default();
            edits_outline(command, step.get("params").unwrap_or(&Value::Null), depth + 1)
        });
    }
    if OUTLINE_COMMANDS.iter().any(|p| id.starts_with(p)) {
        return true;
    }
    if FREE_COMMANDS.iter().any(|p| id.starts_with(p)) {
        return false;
    }
    OUTLINE_TARGETS.iter().any(|key| match params.get(*key) {
        None | Some(Value::Null) => false,
        Some(Value::Array(list)) => !list.is_empty(),
        Some(_) => true,
    })
}

/// Formats a number with at most two decimals and no trailing zeros.
pub fn num(v: f64) -> String {
    if !v.is_finite() {
        return "?".into();
    }
    let r = (v * 100.0).round() / 100.0;
    let text = format!("{r:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".into() } else { text.to_string() }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests;
