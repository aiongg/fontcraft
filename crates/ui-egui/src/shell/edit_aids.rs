//! Edit View display aids and their View menu switches: Tunni lines, curve tension, indexes, curve
//! length, details across glyphs, ghost outline, Lock Outline, the metrics beam (M13.31); the auto-
//! meter and suggestion switches (M13.34, M13.35). The drawing is in `crate::canvas::edit_aids`,
//! `crate::canvas::autometer` and `crate::canvas::suggestions`. (M13.31, M13.33 to M13.36 fill this
//! module.)
//!
//! Lock Outline acts on the pointer tools only: [`unless_locked`] drops the outline edits from
//! what a tool asks for before the tools glue carries it out, so commands run by name (menus,
//! the control channel, MCP, macros) still edit the outline.

use std::collections::BTreeMap;

use fontcraft_model::{Font, MAX_COORDINATE};
use fontcraft_tools::{Action, Output};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::state::Tab;

/// This module's view and window state, kept in `UiState::panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EditAidsState {
    /// `view.showTunniLines`.
    pub tunni_lines: bool,
    /// `view.showCurveTension`.
    pub curve_tension: bool,
    /// `view.showIndexes`.
    pub indexes: Indexes,
    /// `view.showCurveLength`.
    pub curve_length: bool,
    /// `view.showDetailsAcrossGlyphs`.
    pub details_across_glyphs: bool,
    /// `view.lockOutline`.
    pub lock_outline: bool,
    /// `view.showMetricsBeam`.
    pub beam: bool,
    /// The beam's height per document id, in font units. A document without an entry uses
    /// [`default_beam_height`].
    pub beam_heights: BTreeMap<u64, f64>,
    /// The snapping suggestions that are on (View › Suggest).
    pub suggest: fontcraft_tools::suggestions::Switches,
}

/// Which elements `view.showIndexes` labels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
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

    fn all(on: bool) -> Self {
        Self { points: on, segments: on, contours: on, components: on, anchors: on }
    }
}

pub const COMMANDS: &[UiCommand] = &[
    (
        "view.showTunniLines",
        "Tunni Lines",
        &["View"],
        "",
        "{on?: bool} draw, for every curve segment of the edited layer, a dotted line joining the ends of its two handles and a dot where the lines through its handles meet (omitted: toggle) → {on}",
    ),
    (
        "view.showCurveTension",
        "Show Curve Tension",
        &["View", "Show"],
        "",
        "{on?: bool} print beside each handle of every curve segment its tension: the handle's length as a percentage of the distance from its node to the point where the lines through both handles meet (omitted: toggle) → {on}",
    ),
    (
        "view.showIndexes",
        "Show Indexes",
        &["View"],
        "",
        "{points?: bool, segments?: bool, contours?: bool, components?: bool, anchors?: bool} label nodes with their index in their path, segments with their index in their path, paths and components with their shape index, and anchors with their index in the layer. The kinds given are set and the others kept; with none given, all five turn off when any is on, else all turn on → {points, segments, contours, components, anchors}",
    ),
    (
        "view.showCurveLength",
        "Show Curve Length",
        &["View"],
        "",
        "{on?: bool} print the arc length of every curve segment beside it, in font units (omitted: toggle) → {on}",
    ),
    (
        "view.showDetailsAcrossGlyphs",
        "Details Across Glyphs",
        &["View"],
        "",
        "{on?: bool} draw nodes, handles and the display aids that are on for every glyph on the edited glyph's line, not only for the edited glyph (omitted: toggle) → {on}",
    ),
    (
        "view.lockOutline",
        "Lock Outline",
        &["View", "Lock"],
        "",
        "{on?: bool} refuse pointer and key edits of paths, nodes and components in Edit View while guides, anchors and metrics stay editable, with a badge on the canvas; commands run by name still edit the outline (omitted: toggle) → {on}",
    ),
    (
        "view.showMetricsBeam",
        "Metrics Beam",
        &["View"],
        "",
        "{on?: bool, y?: number} show a horizontal line at height `y` (font units, kept per document; by default half the x-height) with the sidebearings measured where it meets the edited glyph's outline; giving `y` without `on` also shows it (omitted: toggle) → {on, y}",
    ),
    (
        "view.suggestHandles",
        "Suggest Handles",
        &["View", "Suggest"],
        "",
        "{on?: bool} while one handle is dragged, snap it flat, upright, parallel to the segment's other handle or square to it (omitted: toggle)",
    ),
    (
        "view.suggestLines",
        "Suggest Lines",
        &["View", "Suggest"],
        "",
        "{on?: bool} while nodes are dragged, snap to the continuations of nearby straight segments and handles, perpendiculars of diagonal guides, and centrelines between facing edges (omitted: toggle)",
    ),
    (
        "view.suggestDistance",
        "Suggest Distance",
        &["View", "Suggest"],
        "",
        "{on?: bool} while nodes are dragged, snap to outlines parallel to the other segments at the master's suggestDistance (omitted: toggle)",
    ),
    (
        "view.suggestStems",
        "Suggest Stems",
        &["View", "Suggest"],
        "",
        "{on?: bool} while a straight edge is dragged, snap it one of the master's declared stems away from a facing edge (omitted: toggle)",
    ),
];

/// Most documents whose beam height is kept; past it, the oldest document's height is dropped.
const KEPT_BEAM_HEIGHTS: usize = 256;

/// Most levels of `command.batch` inside one another that [`edits_outline`] looks into; a deeper
/// step counts as an outline edit.
const MAX_BATCH_DEPTH: usize = 8;

/// The status bar message when Lock Outline refuses an edit.
pub const LOCKED: &str = "Lock Outline is on: the outline cannot be edited with the pointer or keys";

/// Command id prefixes that always edit the outline.
const OUTLINE_COMMANDS: &[&str] = &["path.", "nodes.", "tunni.", "smartComponent.", "filter."];

/// Command id prefixes that never do: the tools' selection, view and dialog commands, and the
/// guides, anchors, annotations and metrics that stay editable.
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

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if let Some(current) = app.state.panels.edit_aids.suggest.get_mut(id).map(|on| *on) {
        let on = args.toggle(current)?;
        if let Some(flag) = app.state.panels.edit_aids.suggest.get_mut(id) {
            *flag = on;
        }
        return Ok(json!({"on": on}));
    }
    let aids = &mut app.state.panels.edit_aids;
    match id {
        "view.showTunniLines" => toggle(args, &mut aids.tunni_lines),
        "view.showCurveTension" => toggle(args, &mut aids.curve_tension),
        "view.showCurveLength" => toggle(args, &mut aids.curve_length),
        "view.showDetailsAcrossGlyphs" => toggle(args, &mut aids.details_across_glyphs),
        "view.lockOutline" => toggle(args, &mut aids.lock_outline),
        "view.showIndexes" => indexes(args, &mut aids.indexes),
        "view.showMetricsBeam" => beam(app, args),
        _ => Err(args.err("unknown command")),
    }
}

fn toggle(args: &Args, flag: &mut bool) -> Result<Value, String> {
    *flag = args.toggle(*flag)?;
    Ok(json!({"on": *flag}))
}

fn indexes(args: &Args, shown: &mut Indexes) -> Result<Value, String> {
    let mut next = *shown;
    let mut given = false;
    for (key, slot) in [
        ("points", &mut next.points),
        ("segments", &mut next.segments),
        ("contours", &mut next.contours),
        ("components", &mut next.components),
        ("anchors", &mut next.anchors),
    ] {
        if let Some(on) = args.flag(key)? {
            *slot = on;
            given = true;
        }
    }
    if !given {
        next = Indexes::all(!shown.any());
    }
    *shown = next;
    Ok(json!({
        "points": next.points,
        "segments": next.segments,
        "contours": next.contours,
        "components": next.components,
        "anchors": next.anchors,
    }))
}

fn beam(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let y = args.number("y")?;
    if y.is_some_and(|y| y.abs() > MAX_COORDINATE) {
        return Err(args.err(format!("`y` must lie within ±{MAX_COORDINATE}")));
    }
    let doc = app.session.active().map(|d| d.id);
    if y.is_some() && doc.is_none() {
        return Err(args.err("no font is open"));
    }
    let aids = &mut app.state.panels.edit_aids;
    let on = if args.get("on").is_none() && y.is_some() { true } else { args.toggle(aids.beam)? };
    if let (Some(y), Some(doc)) = (y, doc) {
        if aids.beam_heights.len() >= KEPT_BEAM_HEIGHTS && !aids.beam_heights.contains_key(&doc) {
            aids.beam_heights.pop_first();
        }
        aids.beam_heights.insert(doc, y);
    }
    aids.beam = on;
    let shown = match (doc, app.session.font().ok(), app.state.tab()) {
        (Some(doc), Some(font), Some(tab)) => json!(beam_height(&app.state.panels.edit_aids, font, doc, tab)),
        (Some(doc), _, _) => app.state.panels.edit_aids.beam_heights.get(&doc).map_or(Value::Null, |y| json!(y)),
        _ => Value::Null,
    };
    Ok(json!({"on": on, "y": shown}))
}

/// The metrics beam's height for document `doc` shown in `tab`, in font units: the height set
/// with `view.showMetricsBeam`, else [`default_beam_height`]. Edit View and Space Center share it.
pub fn beam_height(aids: &EditAidsState, font: &Font, doc: u64, tab: &Tab) -> f64 {
    aids.beam_heights.get(&doc).copied().filter(|y| y.is_finite()).unwrap_or_else(|| default_beam_height(font, tab))
}

/// Half the x-height of the tab's master, like the text-mode measurement line.
pub fn default_beam_height(font: &Font, tab: &Tab) -> f64 {
    super::measurement::default_height(font, tab)
}

/// `out` without the outline edits while Lock Outline is on; the status bar says why they were
/// dropped. The gesture itself goes on: a drag whose previews are all dropped commits nothing.
pub fn unless_locked(app: &mut FontcraftApp, mut out: Output) -> Output {
    if !app.state.panels.edit_aids.lock_outline {
        return out;
    }
    let before = out.actions.len();
    out.actions.retain(|a| !refused(a));
    if out.actions.len() != before {
        app.state.status = LOCKED.to_string();
    }
    out
}

fn refused(action: &Action) -> bool {
    match action {
        Action::Preview { invocation } | Action::Run { invocation } => edits_outline(&invocation.id, &invocation.params, 0),
        _ => false,
    }
}

/// Whether running `id` with `params` changes paths, nodes or components, as far as Lock Outline
/// is concerned.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_edits_are_told_apart_from_guide_anchor_and_metric_edits() {
        assert!(edits_outline("selection.move", &json!({"glyph": "A", "points": [[0, 1]], "dx": 5}), 0));
        assert!(!edits_outline("selection.move", &json!({"glyph": "A", "anchors": ["top"], "dx": 5}), 0));
        assert!(!edits_outline("selection.move", &json!({"glyph": "A", "points": [], "anchors": ["top"]}), 0));
        assert!(edits_outline("path.addPath", &json!({}), 0));
        assert!(!edits_outline("guide.setProperties", &json!({"points": [[0, 0]]}), 0));
        let batch = json!({"steps": [{"command": "selection.move", "params": {"points": [[0, 1]]}}, {"command": "path.connectNodes", "params": {}}]});
        assert!(edits_outline("command.batch", &batch, 0));
        let guides = json!({"steps": [{"command": "guide.add", "params": {}}]});
        assert!(!edits_outline("command.batch", &guides, 0));
        let mut deep = json!({"steps": []});
        for _ in 0..20 {
            deep = json!({"steps": [{"command": "command.batch", "params": deep}]});
        }
        assert!(edits_outline("command.batch", &deep, 0), "too deep counts as an edit");
    }
}
