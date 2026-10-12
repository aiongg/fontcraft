//! Filter by Encoding File (`view.filterByEncodingFile`): an encoding file, or a built-in code
//! page, becomes a list filter in the sidebar's Filters section that lists the encoding's glyphs
//! in slot order, and the filter is selected. A filled slot whose glyph the font lacks stays in
//! the filter, so the filter's missing badge and Show Missing Glyphs report it; empty slots are
//! counted in the result and draw no cell.
//!
//! The file reader and the code pages are `fontcraft_engine::cmd::encodings`; the filter is
//! stored and selected through the sidebar's commands.

use fontcraft_engine::cmd::encodings::{BUILTIN_ENCODINGS, Encoding, builtin_encoding, parse_encoding, read_text_file, slot_names};
use serde_json::{Value, json};

use super::sidebar::{stored_filter, unique_filter_name};
use crate::app::FontcraftApp;
use crate::shell::{Args, UiCommand};

pub const COMMANDS: &[UiCommand] = &[(
    "view.filterByEncodingFile",
    "Filter by Encoding File",
    &["View"],
    "",
    "{path?: path, name?: string} shows an encoding's glyphs in slot order as a list filter in the Filters section, and selects it. path: an encoding file (lines `name slot`, `0xSlot 0xCodePoint`, or one name or code point per slot), and name the filter's name (default the file's title, else its file name). Without path, name is a built-in encoding: ascii, latin1, windows1252 or macRoman. A code point slot shows the font's glyph for it, or the glyph data's name for it when the font lacks one. A list filter of the same name is replaced → {filter: path, slots, filled, glyphs: [names in slot order]}",
)];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != "view.filterByEncodingFile" {
        return Err(format!("unknown command `{id}`"));
    }
    let (encoding, filter_name) = encoding_of(args)?;
    let font = app.session.font().map_err(|e| args.err(e))?;
    let data = app.session.glyph_data().map_err(|e| args.err(e))?;
    let names: Vec<String> = slot_names(font, &data, &encoding).into_iter().flatten().collect();
    if names.is_empty() {
        return Err(args.err("the encoding has no filled slots"));
    }
    let path = json!(["filters", filter_name]);
    let existing = stored_filter(app.session.settings(), &["filters".to_string(), filter_name.clone()]);
    let path = match existing {
        Some(f) if f.get("kind").and_then(Value::as_str) == Some("list") => {
            app.run("fontView.editFilter", &json!({"filter": path, "glyphs": names}))?;
            path
        }
        Some(_) => {
            // A smart filter or folder holds the name: the encoding gets a name of its own.
            let free = unique_filter_name(app.session.settings(), &filter_name);
            app.run("fontView.addListFilter", &json!({"name": free, "glyphs": names}))?;
            json!(["filters", free])
        }
        None => {
            app.run("fontView.addListFilter", &json!({"name": filter_name, "glyphs": names}))?;
            path
        }
    };
    app.run("fontView.selectSidebarEntries", &json!({"entries": [path]}))?;
    Ok(json!({"filter": path, "slots": encoding.slots.len(), "filled": encoding.filled(), "glyphs": names}))
}

/// The encoding the parameters name, and the filter name it gets.
fn encoding_of(args: &Args) -> Result<(Encoding, String), String> {
    let name = args.string("name")?.map(str::trim).filter(|n| !n.is_empty());
    match args.string("path")? {
        Some(path) => {
            let bytes = read_text_file(path).map_err(|e| args.err(e))?;
            let encoding = parse_encoding(&bytes).map_err(|e| args.err(format!("{path}: {e}")))?;
            let stem = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned());
            let filter_name = name.map(str::to_string).or_else(|| encoding.title.clone()).or(stem).unwrap_or_else(|| "Encoding".to_string());
            Ok((encoding, filter_name))
        }
        None => {
            let ids: Vec<&str> = BUILTIN_ENCODINGS.iter().map(|(id, _)| *id).collect();
            let name = name.ok_or_else(|| args.err(format!("give `path` (an encoding file) or `name` (a built-in encoding: {})", ids.join(", "))))?;
            let encoding =
                builtin_encoding(name).ok_or_else(|| args.err(format!("`{name}` is not a built-in encoding; use one of {}", ids.join(", "))))?;
            let title = encoding.title.clone().unwrap_or_else(|| name.to_string());
            Ok((encoding, title))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use serde_json::json;

    use crate::FontcraftApp;

    #[test]
    fn an_encoding_file_with_empty_slots_becomes_a_selected_list_filter() {
        let path = std::env::temp_dir().join(format!("fontcraft-encoding-{}.enc", std::process::id()));
        std::fs::write(&path, "%%FONTLAB ENCODING: 1; Tiny\nC 67\nA 65\n0x45 0x0045\nAgrave 70\n").unwrap();
        let mut app = FontcraftApp::new(fontcraft_engine::Session::with_document());
        let r = app.run("view.filterByEncodingFile", &json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!(r["filter"], json!(["filters", "Tiny"]));
        assert_eq!(r["slots"], 71, "slots 0 to 70, most of them empty");
        assert_eq!(r["filled"], 4);
        assert_eq!(r["glyphs"], json!(["A", "C", "E", "Agrave"]), "slot order, empty slots left out");

        // Font View lists the font's glyphs in slot order; Agrave is missing from the font.
        let shown = app.run("fontView.queryGlyphs", &json!({"entries": [["filters", "Tiny"]]})).unwrap();
        assert_eq!(shown["glyphs"], json!(["A", "C", "E"]));
        let missing = app.run("fontView.listMissingGlyphs", &json!({"entry": ["filters", "Tiny"]})).unwrap();
        assert_eq!(missing["missing"], json!(["Agrave"]));

        // Running it again replaces the filter instead of adding a second one.
        app.run("view.filterByEncodingFile", &json!({"path": path.to_string_lossy()})).unwrap();
        let filters = crate::shell::font_view::sidebar::stored_filters(app.session.settings());
        assert_eq!(filters.iter().filter(|f| f["name"] == "Tiny").count(), 1);
        let _ = std::fs::remove_file(&path);

        let r = app.run("view.filterByEncodingFile", &json!({"name": "windows1252"})).unwrap();
        assert_eq!(r["slots"], 256);
        assert!(app.run("view.filterByEncodingFile", &json!({})).is_err());
        assert!(app.run("view.filterByEncodingFile", &json!({"name": "klingon"})).is_err());
    }
}
