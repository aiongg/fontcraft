//! Typing a text note on the canvas (`annotationText`, `annotation.update`).
//!
//! A click with the Text Annotation tool adds a note (`annotation.add`) and opens a multi-line field at its box; a
//! double-click on a text note with any annotation tool opens the same field. Return adds a line. `Cmd+Return` or a
//! click outside commits the text through `annotation.update`; `Escape` cancels. A new note is added and typed as one
//! undo step: committing undoes the bare add and adds the note again with its text, and a new note left empty or
//! cancelled is removed by that undo.

use craft::ui::contract::{Bind, Show as _, ShowContainer as _};
use craft::ui::popover::Popover;
use craft::ui::runtime::env::Env;
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::text::TextField;
use egui::{Pos2, Ui};
use fontcraft_engine::model::LayerSide;
use fontcraft_geom::Point;
use fontcraft_tools::annotate::{Kind, hits};
use serde_json::{Value, json};

use super::overlays::OverlayWords;
use crate::app::{FontCraftApp, Msg};

/// Most lines the field shows at once.
const MAX_LINES: usize = 12;
/// Most characters a note takes (the engine's limit is per layer).
const MAX_TEXT: usize = 65_536;

/// The note being typed.
#[derive(Clone, Debug, PartialEq)]
pub struct NoteEdit {
    pub glyph: String,
    pub layer: String,
    pub side: LayerSide,
    pub index: usize,
    pub text: String,
    /// The text when the field opened.
    pub original: String,
    /// The note was added by the click that opened the field.
    pub new: bool,
    /// The document and its revision right after the add: undoing the add is safe only while it holds.
    pub added_at: Option<(u64, u64)>,
    /// The note's position and box width, for adding it again with its text.
    pub position: [f64; 2],
    pub width: f64,
    /// The field is open; the popover writes `false` when it closes.
    pub open: bool,
    /// The field closed: `Some(true)` commits, `Some(false)` cancels.
    pub done: Option<bool>,
}

/// After a tool's command ran: a text note the Text Annotation tool added opens its field.
pub fn after_tool_run(app: &mut FontCraftApp, id: &str, params: &Value) {
    if id != "annotation.add" || params.get("type").and_then(Value::as_str) != Some("text") || app.tools.toolbox.base_id() != "annotationText" {
        return;
    }
    let Some(target) = super::tools::target(&app.session, &app.view) else { return };
    let Ok(font) = app.session.font() else { return };
    let Some(layer) = font.glyph(&target.glyph).and_then(|g| g.layer(&target.layer)) else { return };
    let annotations = layer.parts(target.side).annotations;
    let index = match app.tools.selection.annotations.iter().copied().collect::<Vec<_>>().as_slice() {
        [one] => *one,
        _ => annotations.len().saturating_sub(1),
    };
    let Some(a) = annotations.get(index) else { return };
    app.overlays.note = Some(NoteEdit {
        glyph: target.glyph.clone(),
        layer: target.layer.clone(),
        side: target.side,
        index,
        text: a.text.clone().unwrap_or_default(),
        original: a.text.clone().unwrap_or_default(),
        new: true,
        added_at: app.session.active().map(|d| (d.id, d.revision)),
        position: [a.x, a.y],
        width: fontcraft_tools::annotate::text_width(a),
        open: true,
        done: None,
    });
}

/// A double-click at `pos` (font units) with an annotation tool on a text note opens its field.
pub fn double_click(app: &mut FontCraftApp, pos: Point) {
    if !app.tools.toolbox.base_id().starts_with("annotation") {
        return;
    }
    let Some(target) = super::tools::target(&app.session, &app.view) else { return };
    let view = super::tools::apply::tools_view(&super::tools::apply::camera(app));
    let tol = view.units(f64::from(super::tools::tokens::ToolTokens::standard().hit_tolerance));
    let Ok(font) = app.session.font() else { return };
    let Some(layer) = font.glyph(&target.glyph).and_then(|g| g.layer(&target.layer)) else { return };
    let annotations = layer.parts(target.side).annotations;
    let hit = annotations.iter().enumerate().rev().find(|(_, a)| Kind::of(a) == Kind::Text && hits(a, &view, pos, tol));
    let Some((index, a)) = hit else { return };
    let text = a.text.clone().unwrap_or_default();
    app.overlays.note = Some(NoteEdit {
        glyph: target.glyph,
        layer: target.layer,
        side: target.side,
        index,
        original: text.clone(),
        text,
        new: false,
        added_at: None,
        position: [a.x, a.y],
        width: fontcraft_tools::annotate::text_width(a),
        open: true,
        done: None,
    });
}

/// The commands that finish `note`: `commit` takes its text, else it is cancelled. `revision` is the active
/// document's now.
pub fn plan(note: &NoteEdit, commit: bool, revision: Option<(u64, u64)>) -> Vec<Msg> {
    let mut address = json!({"glyph": note.glyph, "layer": note.layer});
    if note.side == LayerSide::Background
        && let Some(o) = address.as_object_mut()
    {
        o.insert("side".into(), json!("background"));
    }
    let with = |extra: Value| {
        let mut p = address.clone();
        if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        p
    };
    let undoable = note.new && note.added_at.is_some() && note.added_at == revision;
    let empty = note.text.trim().is_empty();
    match (commit, note.new) {
        (false, false) => Vec::new(),
        (false, true) | (true, true) if empty || !commit => {
            if undoable {
                vec![Msg::run("edit.undo", json!({}))]
            } else {
                vec![Msg::run("annotation.delete", with(json!({"annotations": [note.index]})))]
            }
        }
        (true, true) if undoable => vec![
            Msg::run("edit.undo", json!({})),
            Msg::run("annotation.add", with(json!({"type": "text", "position": note.position, "width": note.width, "text": note.text}))),
        ],
        _ if note.text == note.original && !note.new => Vec::new(),
        _ => vec![Msg::run("annotation.update", with(json!({"annotation": note.index, "text": note.text})))],
    }
}

/// Finishes the note whose field closed. Runs in `drain`'s pass, after the messages.
pub fn finish(app: &mut FontCraftApp) {
    let Some(commit) = app.overlays.note.as_ref().and_then(|n| n.done) else { return };
    let Some(note) = app.overlays.note.take() else { return };
    let revision = app.session.active().map(|d| (d.id, d.revision));
    for msg in plan(&note, commit, revision) {
        if let Err(error) = app.apply_msg(msg) {
            app.status = error;
            break;
        }
    }
}

/// What the field reads and changes.
pub struct EditorProps<'a> {
    pub note: &'a mut NoteEdit,
    /// The note's top-left corner on screen.
    pub at: Pos2,
}

/// Shows the field over the note.
pub fn show(ui: &mut Ui, props: EditorProps<'_>) {
    let words = Env::of(ui).words::<OverlayWords>();
    let note = props.note;
    let lines = note.text.lines().count().clamp(1, MAX_LINES) + 1;
    let (text, open) = (&mut note.text, &mut note.open);
    let out = Popover::new(words.note.clone(), Bind::bound(open)).at_point(props.at).show(ui, |ui, _| {
        let edit = TextField::new(words.note.clone(), Bind::bound(&mut *text)).lines(lines).limit(MAX_TEXT).submits().show(ui);
        if edit.escaped {
            Some(false)
        } else if edit.submitted {
            Some(true)
        } else {
            None
        }
    });
    let done = out.inner.flatten().or(match out.output.closed {
        Some(CloseReason::Escape) => Some(false),
        Some(_) => Some(true),
        None => None,
    });
    if done.is_some() {
        note.done = done;
    }
}
