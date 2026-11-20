//! The system clipboard, app side.
//!
//! - Copy and Cut on the canvas run `edit.copy` and `edit.cut` on the selection and put the `json` text of the result
//!   on the clipboard; our own fragment pastes back without loss, and `edit.paste` also reads the SVG other apps put
//!   there.
//! - In Font View, Copy runs `edit.copy` with the selected glyphs and Font View's master and puts the copied glyphs
//!   (the `fontcraft-glyphs` text) on the clipboard; Paste runs `edit.paste` with that text, which adds the glyphs (a
//!   taken name gets the first free `.001` suffix), in this document or another. Text that holds no copied glyphs is
//!   not pasted in Font View.
//! - Copy Glyph Names (the glyph menus) puts the `text` of `glyph.copyGlyphNames` on the clipboard.
//! - Edit › Paste and Paste Special fired without text ask the window for the clipboard's text ([`Clipboard::request`]);
//!   the shell takes the text when it arrives and calls [`paste`]. Paste Special opens `dialog.pasteSpecial` on
//!   copied glyphs, with the selected glyphs as its `targets` and Font View's master, or on the canvas the edited glyph
//!   and the master of the layer it shows.
//! - Font View's master is the current master (`view.selectMaster`). A drawing pastes into the layer the canvas shows
//!   (the Layers panel's pick), as the tools edit it.
//! - Only a person's copy reaches the system clipboard: a copy the control channel names (`engine.execute`) returns
//!   its text in the reply and leaves the clipboard alone.
//!
//! - In Font Info's Features sidebar, Copy runs `font.features.copyEntries` on the selected entries and puts its `text`
//!   on the clipboard; Paste (`font.info.features.paste`) asks for the clipboard's text and runs
//!   `font.features.pasteEntries` with it. Copy and Paste fired while one of the sidebar's lists holds the keyboard
//!   focus (Cmd+C, Cmd+V) go to that list, which posts the same commands ([`to_focused_list`]).
//!
//! The text goes out through [`Clipboard::out`], which the shell hands to the window's clipboard.

use craft::ui::runtime::commands::CommandRef;
use craft_ext::ui::list::{EditCommand, deliver};
use serde_json::{Map, Value, json};

use super::{FontCraftApp, Msg};

const COPY: &str = "edit.copy";
const CUT: &str = "edit.cut";
const PASTE: &str = "edit.paste";
const PASTE_SPECIAL: &str = "edit.pasteSpecial";
const SELECT_ALL: &str = "edit.selectAll";
/// The `format` the engine writes into copied glyphs (`edit.copy` with `glyphs`).
const GLYPHS_FORMAT: &str = "fontcraft-glyphs";
/// Longest clipboard text a paste takes.
pub const MAX_PASTE_BYTES: usize = 16 * 1024 * 1024;

/// Which paste waits for the clipboard's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteKind {
    Paste,
    Special,
    /// Feature entries into the Features sidebar.
    FeatureEntries,
}

/// The clipboard's state on the app.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clipboard {
    /// Text for the system clipboard, put there by the shell in the next pass.
    pub out: Option<String>,
    /// A paste waits for the clipboard's text.
    pub request: Option<PasteKind>,
    /// The pass the window was asked for the clipboard's text in.
    pub asked: Option<u64>,
}

/// After a run of `id`: Copy and Cut put their `json` text on the clipboard, Copy Glyph Names its `text`.
pub fn after_run(app: &mut FontCraftApp, id: &str, result: &Value) {
    let key = match id {
        COPY | CUT => "json",
        "glyph.copyGlyphNames" | "font.features.copyEntries" => "text",
        _ => return,
    };
    if let Some(text) = result.get(key).and_then(Value::as_str) {
        app.clipboard.out = Some(text.to_string());
    }
}

fn is_empty(params: &Value) -> bool {
    params.is_null() || params.as_object().is_some_and(Map::is_empty)
}

/// A clipboard command the person fired (menu item, shortcut), after its targets were filled. Returns the command to
/// run, or `None` when it waits for the clipboard's text: Paste and Paste Special with nothing to paste ask the window
/// for it. Copy and Cut in Font View copy the selected glyphs.
pub fn fired(app: &mut FontCraftApp, command: CommandRef) -> Option<CommandRef> {
    match command.id.as_str() {
        PASTE | PASTE_SPECIAL if ["json", "svg", "pdf", "text"].iter().all(|k| command.params.get(k).is_none_or(Value::is_null)) => {
            app.clipboard.request = Some(if command.id == PASTE { PasteKind::Paste } else { PasteKind::Special });
            None
        }
        COPY if is_empty(&command.params) && in_font_view(app) => {
            let mut m = Map::new();
            m.insert("glyphs".into(), json!(app.view.font_view.selected));
            if let Some(master) = font_view_master(app) {
                m.insert("master".into(), json!(master));
            }
            Some(CommandRef::with_params(COPY, Value::Object(m)))
        }
        _ => Some(command),
    }
}

/// The edit command a fired Edit menu command stands for.
fn edit_command(id: &str) -> Option<EditCommand> {
    match id {
        COPY => Some(EditCommand::Copy),
        CUT => Some(EditCommand::Cut),
        PASTE => Some(EditCommand::Paste),
        SELECT_ALL => Some(EditCommand::SelectAll),
        _ => None,
    }
}

/// Hands an Edit command the person fired with no parameters to the list that holds the keyboard focus and takes it
/// (the Features sidebar takes Copy and Paste); `true` when the list took it, and the document is left alone.
pub fn to_focused_list(app: &FontCraftApp, command: &CommandRef) -> bool {
    let Some(edit) = edit_command(&command.id) else { return false };
    is_empty(&command.params) && app.window.as_ref().is_some_and(|ctx| deliver(ctx, edit))
}

/// Font View shows with glyphs selected: copying and pasting act on whole glyphs.
fn in_font_view(app: &FontCraftApp) -> bool {
    !app.view.font_view.selected.is_empty() && app.font_view_shows()
}

/// Font View's master: the current master (`view.selectMaster`).
fn font_view_master(app: &FontCraftApp) -> Option<String> {
    let font = app.session.font().ok()?;
    crate::shell::masters::current_master(font, &app.view.layers).map(|m| m.id.clone())
}

/// The master of the layer the canvas shows (the Layers panel's pick), else the current master.
fn canvas_master(app: &FontCraftApp) -> Option<String> {
    let font = app.session.font().ok()?;
    crate::canvas::tools::active_master(font, &app.view).map(|m| m.id.clone())
}

/// Whether clipboard text holds copied glyphs rather than a drawing.
fn holds_glyphs(text: &str) -> bool {
    text.trim_start().starts_with('{') && text.contains(GLYPHS_FORMAT)
}

/// The message that pastes `text` as `kind` asks: `edit.paste` into the edited glyph (on the canvas) or the copied
/// glyphs into the font (in Font View), or Paste Special's dialog. Errors when there is nothing to paste, or no glyph
/// to paste a drawing into ([`FontCraftApp::targets_chosen`]).
pub fn paste_msg(app: &mut FontCraftApp, text: &str, kind: PasteKind) -> Result<Msg, String> {
    if text.len() > MAX_PASTE_BYTES {
        return Err(format!("Paste: the clipboard's text is larger than {} MiB", MAX_PASTE_BYTES >> 20));
    }
    if text.trim().is_empty() {
        return Err("Paste: the clipboard holds no text".into());
    }
    if kind == PasteKind::FeatureEntries {
        return Ok(Msg::run("font.features.pasteEntries", json!({ "text": text })));
    }
    let glyphs = holds_glyphs(text);
    if kind == PasteKind::Special {
        if !glyphs {
            return Err("Paste Special: the clipboard holds no copied glyphs".into());
        }
        // With no glyph chosen the copied glyphs go over the glyphs of their names, not over the font's first glyph.
        let targets: Vec<String> = if in_font_view(app) {
            app.view.font_view.selected.clone()
        } else if app.targets_chosen() {
            crate::canvas::tools::target(&app.session, &app.view).map(|t| t.glyph).into_iter().collect()
        } else {
            Vec::new()
        };
        let mut params = Map::new();
        params.insert("text".into(), json!(text));
        if !targets.is_empty() {
            params.insert("targets".into(), json!(targets));
        }
        let master = if !in_font_view(app) && app.targets_chosen() { canvas_master(app) } else { font_view_master(app) };
        if let Some(master) = master {
            params.insert("master".into(), json!(master));
        }
        return Ok(Msg::Shell(CommandRef::with_params("dialog.open", json!({"dialog": "dialog.pasteSpecial", "params": params}))));
    }
    if app.font_view_shows() && glyphs {
        return Ok(Msg::run(PASTE, json!({ "text": text })));
    }
    if app.font_view_shows() && in_font_view(app) {
        return Err("Paste: the clipboard holds no copied glyphs to add to the font".into());
    }
    if !app.targets_chosen() {
        return Err(super::select_glyph_first(PASTE));
    }
    let mut command = crate::canvas::tools::targets::with_targets(app, CommandRef::new(PASTE));
    let mut params = match command.params {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    params.insert("text".into(), json!(text));
    command.params = Value::Object(params);
    Ok(Msg::Run(command))
}

/// Pastes `text` as `kind` asks, after what views posted: the shell calls it when the clipboard's text arrives.
pub fn paste(app: &mut FontCraftApp, text: &str, kind: PasteKind) {
    match paste_msg(app, text, kind) {
        Ok(msg) => app.sent.push(msg),
        Err(e) => app.status = e,
    }
}
