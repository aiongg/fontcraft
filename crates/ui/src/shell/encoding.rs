//! View › Filter by Encoding File (`view.filterByEncodingFile`): an encoding file, or a built-in code page, becomes a
//! list filter in the sidebar's Filters section that lists the encoding's glyphs in slot order, and the filter is
//! selected. A filled slot whose glyph the font lacks stays in the filter, so the filter's missing badge reports it;
//! empty slots draw no cell.
//!
//! The file reader and the code pages are `fontcraft_engine::cmd::encodings`; the filter is stored through the engine's
//! filter commands and selected through `fontView.selectSidebarEntries`. Fired from the menu without a path, the
//! command asks for the file first (`app::files::PICKED`).

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::cmd::encodings::{BUILTIN_ENCODINGS, Encoding, builtin_encoding, parse_encoding, read_text_file, slot_names};
use serde_json::{Value, json};

use super::commands::{Module, UiCommand};
use super::params::{quoted, string};
use crate::app::FontCraftApp;
use crate::app::sidebar::unique_name;

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "view.filterByEncodingFile",
    label: "Filter by Encoding File",
    menu: &["View"],
    shortcut: None,
    params: "{path?: path, name?: string} show an encoding's glyphs in slot order as a list filter in the Filters section, and select it. path: an encoding file (lines `name slot`, `0xSlot 0xCodePoint`, or one name or code point per slot), and name the filter's name (default the file's title, else its file name). Without path, name is a built-in encoding: ascii, latin1, windows1252 or macRoman; with neither, the window asks for the file. A code point slot shows the font's glyph for it, or the glyph data's name for it when the font lacks one. A list filter of the same name is replaced",
}];

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    if command.id != "view.filterByEncodingFile" {
        return Err(format!("no encoding command `{}`", command.id));
    }
    let (encoding, filter_name) = encoding_of(&command.params)?;
    let font = app.session.font().map_err(|e| e.to_string())?;
    let data = app.session.glyph_data().map_err(|e| e.to_string())?;
    let names: Vec<String> = slot_names(font, &data, &encoding).into_iter().flatten().collect();
    if names.is_empty() {
        return Err("the encoding has no filled slots".into());
    }
    let filters = stored_filters(app);
    let kind_of = |name: &str| {
        filters.iter().find(|f| f.get("name").and_then(Value::as_str) == Some(name)).map(|f| f.get("kind").and_then(Value::as_str) == Some("list"))
    };
    let name = match kind_of(&filter_name) {
        Some(true) => {
            app.session.execute("fontView.editFilter", &json!({"filter": ["filters", filter_name], "glyphs": names})).map_err(|e| e.to_string())?;
            filter_name
        }
        found => {
            // A smart filter or a folder holds the name: the encoding gets a name of its own.
            let free = if found.is_some() {
                let taken: Vec<String> = filters.iter().filter_map(|f| f.get("name").and_then(Value::as_str)).map(str::to_string).collect();
                unique_name(&taken, &filter_name)
            } else {
                filter_name
            };
            app.session.execute("fontView.addListFilter", &json!({"name": free, "glyphs": names})).map_err(|e| e.to_string())?;
            free
        }
    };
    super::commands::run(app, &CommandRef::with_params("fontView.selectSidebarEntries", json!({"entries": [["filters", name]]})))
}

/// The top-level entries of the Filters section as stored; empty while the built-in filters are unchanged.
fn stored_filters(app: &FontCraftApp) -> Vec<Value> {
    app.session.settings().get("fontView.filters").ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

/// The encoding the parameters name, and the filter name it gets.
fn encoding_of(p: &Value) -> Result<(Encoding, String), String> {
    let name = string(p, "name")?.map(str::trim).filter(|n| !n.is_empty());
    match string(p, "path")? {
        Some(path) => {
            let bytes = read_text_file(path).map_err(|e| e.to_string())?;
            let encoding = parse_encoding(&bytes).map_err(|e| format!("{}: {e}", quoted(path)))?;
            let stem = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned());
            let filter_name = name.map(str::to_string).or_else(|| encoding.title.clone()).or(stem).unwrap_or_else(|| "Encoding".to_string());
            Ok((encoding, filter_name))
        }
        None => {
            let ids: Vec<&str> = BUILTIN_ENCODINGS.iter().map(|(id, _)| *id).collect();
            let name = name.ok_or_else(|| format!("give `path` (an encoding file) or `name` (a built-in encoding: {})", ids.join(", ")))?;
            let encoding =
                builtin_encoding(name).ok_or_else(|| format!("`{}` is not a built-in encoding; use one of {}", quoted(name), ids.join(", ")))?;
            let title = encoding.title.clone().unwrap_or_else(|| name.to_string());
            Ok((encoding, title))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use fontcraft_engine::Session;
    use serde_json::{Value, json};

    use crate::app::{APP, FontCraftApp, Msg};

    fn shell(app: &mut FontCraftApp, id: &str, params: Value) -> String {
        app.status.clear();
        app.sent.push(Msg::Shell(CommandRef::with_params(id, params)));
        app.drain();
        app.status.clone()
    }

    /// An encoding file with an empty slot becomes a selected list filter of its glyphs in slot order; running it again
    /// replaces that filter; a built-in code page works by name; a bad name changes nothing.
    #[test]
    fn an_encoding_file_becomes_a_selected_list_filter() {
        let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
        shell(&mut app, "fontView.show", json!({}));
        let dir = std::env::temp_dir().join(format!("fontcraft-ui-encoding-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Basic.enc");
        std::fs::write(&path, "B 0\nA 2\n").unwrap();
        let path = path.to_string_lossy().into_owned();

        assert_eq!(shell(&mut app, "view.filterByEncodingFile", json!({"path": path})), "");
        let filters = app.session.settings().get("fontView.filters").unwrap();
        let basic = filters.as_array().unwrap().iter().find(|f| f["name"] == "Basic").expect("a filter named after the file");
        assert_eq!((basic["kind"].clone(), basic["glyphs"].clone()), (json!("list"), json!(["B", "A"])), "slot order, the empty slot left out");
        assert_eq!(app.view.font_view.entries, Some(json!(["filters/Basic"])), "the filter is selected");

        std::fs::write(&path, "C 0\n").unwrap();
        assert_eq!(shell(&mut app, "view.filterByEncodingFile", json!({"path": path})), "");
        let filters = app.session.settings().get("fontView.filters").unwrap();
        let named: Vec<&Value> = filters.as_array().unwrap().iter().filter(|f| f["name"] == "Basic").collect();
        assert_eq!(named.len(), 1, "the list filter of the same name is replaced");
        assert_eq!(named[0]["glyphs"], json!(["C"]));

        assert_eq!(shell(&mut app, "view.filterByEncodingFile", json!({"name": "ascii"})), "");
        let before = app.session.settings().get("fontView.filters").unwrap();
        assert!(!shell(&mut app, "view.filterByEncodingFile", json!({"name": "klingon"})).is_empty());
        assert_eq!(app.session.settings().get("fontView.filters").unwrap(), before, "a refused run changes nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
