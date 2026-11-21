//! The URL scheme in the window (`url.show`): runs the engine's `url.open`, then shows each file's document and the
//! glyphs the URL lists for it in a tab of their own. The last file's document ends up in front.

use craft::ui::runtime::commands::CommandRef;
use serde_json::{Value, json};

use super::commands::{Module, UiCommand};
use crate::app::{FontCraftApp, Msg};

pub const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "url.show",
    label: "Show Glyphs in New Tab",
    menu: &[],
    shortcut: None,
    params: "{url?: string, files?: [{path: string, glyphs?: [{glyph?: name, production?: name, layers?: [id]}]}]} open a fontcraft://show/ URL as url.open does (files that are already open are not opened again) and show each file's document with the first glyph it lists; the last file's document ends up in front. Warnings go to the status line",
}];

pub const MODULE: Module = Module { commands: COMMANDS, run };

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let params = if command.params.is_null() { json!({}) } else { command.params.clone() };
    let reply = app.run_command("url.open", params)?;
    let mut warnings: Vec<String> =
        reply.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    for file in reply.get("tabs").and_then(Value::as_array).into_iter().flatten() {
        let Some(document) = file.get("document").and_then(Value::as_u64) else { continue };
        let Some(index) = app.session.documents().iter().position(|d| d.id == document) else { continue };
        let title = file.get("title").and_then(Value::as_str).unwrap_or_default().to_string();
        if let Err(e) = app.apply_msg(Msg::run("document.activate", json!({ "index": index }))) {
            warnings.push(format!("{title}: {e}"));
            continue;
        }
        // A file the URL opened brings back its tabs from its display strings, before the URL's own tab.
        if file.get("opened").and_then(Value::as_bool) == Some(true) {
            warnings.extend(super::tabs::upkeep::open_display_strings(app));
        }
        // The file's glyphs open in a tab of their own.
        let glyphs: Vec<String> = file
            .get("glyphs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|e| e.get("glyph").and_then(Value::as_str))
            .take(fontcraft_textlayout::MAX_ITEMS)
            .map(str::to_string)
            .collect();
        if !glyphs.is_empty() {
            super::tabs::open_glyphs(app, &glyphs)?;
        }
    }
    if let Some(last) = warnings.last() {
        app.status = format!("url.show: {last}");
    }
    Ok(())
}
