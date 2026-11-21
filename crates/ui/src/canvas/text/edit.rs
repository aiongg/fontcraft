//! Edit › Copy, Cut, Paste, Delete and Select All in text mode (row `text.clipboard`): they act on the edit view's text
//! as the Text tool's keys do. Copy puts the selected items on the clipboard as text: characters, a glyph without one
//! as `/name`, as the display strings write them. Paste reads `/name` back as the glyph name. Paste Special and
//! Delete in All Masters work on outlines, so text mode refuses them.
//!
//! A menu item or shortcut reaches [`fired`] before its targets are filled; Paste without text asks the window for
//! the clipboard's text, which [`shell`] takes when it arrives.

use craft::ui::contract::BodyCx;
use craft::ui::runtime::commands::{ClipItem, CommandRef, Commands as _};
use egui::Ui;
use fontcraft_textlayout::Text;
use serde_json::{Value, json};

use crate::app::{FontCraftApp, Msg};

/// Longest clipboard text a paste takes, in bytes.
pub const MAX_PASTE_BYTES: usize = 1 << 20;
/// Passes a paste waits for the clipboard's text before it gives up.
const PASTE_WAIT: u64 = 30;
/// Why Paste Special does nothing in text mode.
pub const NO_PASTE_SPECIAL: &str = "Paste Special pastes into glyphs: leave text mode (Escape) to use it";

/// The text side of the system clipboard.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextClipboard {
    /// Text for the system clipboard, put there by the shell in the next pass.
    pub out: Option<String>,
    /// A paste waits for the clipboard's text: `Some(None)` before the window was asked, then the pass it was asked in.
    pub paste: Option<Option<u64>>,
}

/// An Edit command fired from a menu or a shortcut in text mode: the messages that do it on the text, or why it does
/// nothing. `None` outside text mode or for another command, which then runs as usual.
pub fn fired(app: &mut FontCraftApp, command: &CommandRef) -> Option<Result<Vec<Msg>, String>> {
    if !super::text_mode(&app.view) {
        return None;
    }
    Some(match command.id.as_str() {
        "edit.copy" | "edit.cut" => {
            if let Some(text) = selected_text(app) {
                app.view.text.clipboard.out = Some(text);
            }
            Ok(if command.id == "edit.cut" { remove(app, false).into_iter().collect() } else { Vec::new() })
        }
        "edit.delete" => Ok(remove(app, true).into_iter().collect()),
        "edit.selectAll" => Ok(vec![shell("editView.setCursor", json!({"index": app.view.text.text.len(), "anchor": 0}))]),
        "edit.paste" => match command.params.get("text").and_then(Value::as_str) {
            Some(text) => paste(text).map(|m| m.into_iter().collect()),
            None => {
                app.view.text.clipboard.paste = Some(None);
                Ok(Vec::new())
            }
        },
        "edit.pasteSpecial" => Err(NO_PASTE_SPECIAL.to_string()),
        "edit.deleteInAllMasters" => Err("Delete in All Masters removes outline points: leave text mode (Escape) to use it".to_string()),
        _ => return None,
    })
}

/// The selected items as clipboard text; `None` when nothing is selected.
pub fn selected_text(app: &FontCraftApp) -> Option<String> {
    let state = &app.view.text;
    let font = app.session.font().ok()?;
    let items = state.text.items().get(state.text.selection())?.to_vec();
    if items.is_empty() {
        return None;
    }
    let mut text = Text::new();
    text.replace(0..0, items).ok()?;
    text.set_current_glyph(state.current());
    Some(text.to_display_string(font))
}

/// The message that removes the selection; with nothing selected and `forward`, the item after the cursor.
fn remove(app: &FontCraftApp, forward: bool) -> Option<Msg> {
    let text = &app.view.text.text;
    let (selection, cursor, len) = (text.selection(), text.cursor(), text.len());
    let range = if !selection.is_empty() {
        selection
    } else if forward {
        cursor..cursor.saturating_add(1).min(len)
    } else {
        return None;
    };
    (!range.is_empty()).then(|| shell("editView.insertText", json!({"chars": "", "replace": [range.start, range.end]})))
}

/// The message that pastes clipboard `text` at the cursor, in place of the selection.
pub fn paste(text: &str) -> Result<Option<Msg>, String> {
    if text.len() > MAX_PASTE_BYTES {
        return Err(format!("Paste: the clipboard holds more than {MAX_PASTE_BYTES} bytes"));
    }
    if text.trim_start().starts_with('{') && text.contains("\"format\"") {
        return Err("Paste: the clipboard holds copied glyphs, not text; leave text mode to paste them".into());
    }
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    Ok((!text.is_empty()).then(|| shell("editView.insertText", json!({"text": text}))))
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

/// The shell's part, every pass: hands copied text to the system clipboard, and takes the clipboard's text for a
/// paste that waits for it (from the system clipboard where the app reads it; else asks the window once, takes the
/// paste event that brings it, gives up when none comes).
pub fn shell_pass(ui: &Ui, cx: &mut BodyCx<'_>, app: &mut FontCraftApp) {
    if let Some(text) = app.view.text.clipboard.out.take() {
        cx.clipboard().put(&[ClipItem::Text(text.into())]);
    }
    let Some(asked) = app.view.text.clipboard.paste else { return };
    // Where the app reads the system clipboard itself, the paste takes its text at once (`clipboard::read_waiting`).
    if let Some(read) = app.clipboard_reader {
        app.view.text.clipboard.paste = None;
        match read().map(|text| text.map(|text| paste(&text))) {
            Ok(Some(Ok(msg))) => app.sent.extend(msg),
            Ok(Some(Err(e))) => app.status = e,
            Ok(None) => app.status = "Paste: the clipboard holds no text".into(),
            Err(e) => app.status = format!("Paste: {e}"),
        }
        return;
    }
    let pasted = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Paste(text) => Some(text.clone()),
            _ => None,
        })
    });
    let pass = ui.ctx().cumulative_pass_nr();
    match (pasted, asked) {
        (Some(text), _) => {
            app.view.text.clipboard.paste = None;
            match paste(&text) {
                Ok(msg) => app.sent.extend(msg),
                Err(e) => app.status = e,
            }
        }
        (None, None) => {
            app.view.text.clipboard.paste = Some(Some(pass));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            ui.ctx().request_repaint();
        }
        (None, Some(at)) if pass.saturating_sub(at) > PASTE_WAIT => {
            app.view.text.clipboard.paste = None;
            app.status = "Paste: the clipboard holds no text".into();
        }
        (None, Some(_)) => ui.ctx().request_repaint(),
    }
}
