//! The Command Console's view model (`panels::command_console`).
//!
//! The log grows on failed commands too, which move no document revision, so this model is not part of the
//! stamped projection: `FontCraftApp::project` calls [`project`] every time, and it rebuilds only when the log, the
//! pane's start, the input line, the history or the document moved, and only while the panel shows.

use std::sync::Arc;

use craft::behavior::dock::PanelState;
use craft::ui::contract::{PlainData, Tracked};

use super::ProjectCx;
use crate::app::console::{self, MAX_LINE_BYTES};
use crate::shell::dock::PanelId;

/// Most log lines the pane holds; older ones stay in the session's log.
pub const MAX_PANE_LINES: usize = 2_000;
/// Longest pane line drawn, in characters; Copy takes the whole line.
pub const MAX_DRAWN_CHARS: usize = 400;

/// What the console panel shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ConsoleModel {
    /// The pane's lines, oldest first, each cut at [`MAX_DRAWN_CHARS`].
    pub lines: Vec<String>,
    /// The input line's text.
    pub input: String,
    /// The command the input line names and its parameters doc.
    pub doc: Option<String>,
    /// Every command id with its parameters doc, sorted by id: what the input line completes.
    pub docs: Arc<[(&'static str, &'static str)]>,
    /// The active font's glyph names, for completion inside parameters.
    pub glyphs: Arc<[String]>,
    /// The lines run, oldest first.
    pub history: Arc<[String]>,
    /// The Macro Panel's editor text (`shell::macro_editor`).
    pub editor: String,
    /// A macro is being recorded (`script.recordMacro`).
    pub recording: bool,
    /// What the model was made from: the log's next entry, the pane's start, the active document and its revision,
    /// and the history's length.
    pub made_from: (u64, u64, Option<(u64, u64)>, usize),
}

/// Refreshes `model` when what it shows moved, while the console panel shows.
pub fn project(model: &mut Tracked<ConsoleModel>, cx: &mut ProjectCx<'_>) {
    if cx.view.workspace.layout().panel_state(&PanelId::CommandConsole) == PanelState::Hidden {
        return;
    }
    let state = &cx.view.commands.console;
    let log = &cx.session.extensions().log;
    let document = cx.session.active().map(|d| (d.id, d.revision));
    let made_from = (log.next_seq(), state.shown_from, document, state.history.len());
    let recording = cx.session.extensions().recording.is_some();
    if model.made_from == made_from
        && model.input == state.input
        && model.editor == state.editor
        && model.recording == recording
        && !model.docs.is_empty()
    {
        return;
    }
    let skip = log.from_seq(state.shown_from).count().saturating_sub(MAX_PANE_LINES);
    let lines: Vec<String> = log.from_seq(state.shown_from).skip(skip).map(|entry| drawn(&entry.to_line())).collect();
    let docs: Arc<[(&'static str, &'static str)]> = if model.docs.is_empty() { console::command_docs().into() } else { Arc::clone(&model.docs) };
    let glyphs: Arc<[String]> = if model.made_from.2 == document && !model.glyphs.is_empty() {
        Arc::clone(&model.glyphs)
    } else {
        cx.session.font().map(|font| font.glyphs.iter().map(|g| g.name.clone()).collect()).unwrap_or_default()
    };
    let same_history = model.history.len() == state.history.len() && model.history.last() == state.history.last();
    let history: Arc<[String]> = if same_history { Arc::clone(&model.history) } else { state.history.clone().into() };
    let input = state.input.chars().take(MAX_LINE_BYTES).collect::<String>();
    let doc = console::complete(&docs, &[], &[], &input).doc.filter(|_| !input.trim().is_empty()).map(|(id, doc)| drawn(&format!("{id} {doc}")));
    let editor = state.editor.clone();
    super::set(model, ConsoleModel { lines, input, doc, docs, glyphs, history, editor, recording, made_from });
}

/// `text` cut at [`MAX_DRAWN_CHARS`] characters, with an ellipsis when cut.
fn drawn(text: &str) -> String {
    let mut out: String = text.chars().take(MAX_DRAWN_CHARS).collect();
    if out.len() < text.len() {
        out.push('…');
    }
    out
}
