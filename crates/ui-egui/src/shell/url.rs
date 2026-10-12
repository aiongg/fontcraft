//! The URL scheme in the window (`url.show`): runs the engine's `url.open`, then adds to each
//! file's document one Edit View tab with the glyph layers the URL lists, in order. A file the URL
//! opened also gets the tabs its display strings name, as File › Open gives it. The last file's
//! document ends up in front.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::state::{MAX_TAB_GLYPHS, MAX_TABS, Tab, TabText};

pub const COMMANDS: &[UiCommand] = &[(
    "url.show",
    "Show Glyphs in New Tab",
    &[],
    "",
    "{url?: string, files?: [{path: string, glyphs?: [{glyph?: name, production?: name, layers?: [id]}]}]} open a fontcraft://show/ URL as url.open does (files that are already open are not opened again) and add to each file's document a new tab with the glyph layers it lists, in order. A tab shows each glyph in one layer: the first one the URL gives for it → {tabs: [{tab, position, glyphs, editing, master, layer}], warnings}",
)];

pub fn run(app: &mut FontcraftApp, _id: &str, args: &Args) -> Result<Value, String> {
    let reply = app.session.execute("url.open", &args.value()).map_err(|e| e.to_string())?;
    let mut warnings: Vec<String> =
        reply.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    let mut shown = Vec::new();
    for file in reply.get("tabs").and_then(Value::as_array).into_iter().flatten() {
        let Some(document) = file.get("document").and_then(Value::as_u64) else { continue };
        let Some(index) = app.session.documents().iter().position(|d| d.id == document) else { continue };
        let title = file.get("title").and_then(Value::as_str).unwrap_or_default().to_string();
        if let Err(e) = app.session.execute("document.activate", &json!({ "index": index })) {
            warnings.push(format!("{title}: {e}"));
            continue;
        }
        app.follow_document();
        if file.get("opened").and_then(Value::as_bool) == Some(true) {
            warnings.extend(app.open_display_strings());
        }
        let entries: Vec<(String, String, String)> = file
            .get("glyphs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|e| {
                let field = |k: &str| e.get(k).and_then(Value::as_str).map(str::to_string);
                Some((field("glyph")?, field("layer")?, field("master")?))
            })
            .collect();
        if entries.is_empty() {
            continue;
        }
        match add_tab(app, &entries, &title, &mut warnings) {
            Ok(tab) => shown.push(tab),
            Err(e) => warnings.push(e),
        }
    }
    if let Some(last) = warnings.last() {
        log::warn!("url.show: {last}");
    }
    Ok(json!({ "tabs": shown, "warnings": warnings }))
}

/// Adds a tab to the active document with `entries` (glyph, layer id, master id) in order. The
/// tab draws in the first entry's master; a glyph listed in another layer is drawn in the first
/// one listed for it, since a tab holds one layer per glyph.
fn add_tab(app: &mut FontcraftApp, entries: &[(String, String, String)], title: &str, warnings: &mut Vec<String>) -> Result<Value, String> {
    if app.state.tabs.len() >= MAX_TABS {
        return Err(format!("{title}: at most {MAX_TABS} tabs can be open, so no tab was added"));
    }
    let entries = match entries.get(..MAX_TAB_GLYPHS) {
        Some(head) => {
            warnings.push(format!("{title}: a tab holds at most {MAX_TAB_GLYPHS} glyphs; the rest were left out"));
            head
        }
        None => entries,
    };
    let master = entries.first().map(|(_, _, m)| m.clone());
    let mut first: BTreeMap<String, String> = BTreeMap::new();
    let mut warned = BTreeSet::new();
    for (glyph, layer, _) in entries {
        match first.get(glyph) {
            None => {
                first.insert(glyph.clone(), layer.clone());
            }
            Some(kept) if kept != layer && warned.insert(glyph.clone()) => {
                warnings.push(format!("{title}: `{glyph}` is shown in the first layer listed for it; a tab shows one layer per glyph"));
            }
            Some(_) => {}
        }
    }
    // A master layer's id is its master's id, so a glyph listed in the tab's master needs no entry.
    first.retain(|_, layer| Some(layer.as_str()) != master.as_deref());
    let names: Vec<String> = entries.iter().map(|(g, _, _)| g.clone()).collect();
    let text = TabText::from_glyphs(&names).map_err(|e| format!("{title}: {e}"))?;
    let id = app.state.next_tab_id;
    app.state.next_tab_id += 1;
    Ok(super::tabs::push(app, Tab { id, text, master, layers: first, ..Default::default() }))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use fontcraft_engine::Session;
    use serde_json::json;

    use crate::FontcraftApp;

    /// `path` percent-encoded for a URL query value.
    fn enc(path: &std::path::Path) -> String {
        let mut out = String::new();
        for b in path.to_string_lossy().bytes() {
            if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
                out.push(char::from(b));
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fontcraft-url-show-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A font with A–Z and a–z on one master, and a backup layer on A; returns the path, the
    /// master id and the backup layer's id.
    fn write_font(dir: &std::path::Path, family: &str) -> (PathBuf, String, String) {
        let mut s = Session::new();
        s.execute("file.new", &json!({ "familyName": family })).unwrap();
        let master = s.font().unwrap().masters[0].id.clone();
        let backup = s.execute("layers.addBackupLayer", &json!({ "glyphs": ["A"], "name": "Backup" })).unwrap();
        let backup = backup["layer"].as_str().unwrap().to_string();
        let path = dir.join(format!("{family}.fontcraft"));
        s.execute("file.saveAs", &json!({ "path": path })).unwrap();
        (path, master, backup)
    }

    #[test]
    fn url_show_adds_a_tab_per_file_with_the_listed_layers() {
        let dir = temp("tabs");
        let (one, master, backup) = write_font(&dir, "One");
        let (two, _, _) = write_font(&dir, "Two");
        let url =
            format!("fontcraft://show/?path={}&glyph=A&layer={backup}&layer={master}&glyph=B&path={}&glyph=C&glyph=missing", enc(&one), enc(&two));
        let mut app = FontcraftApp::new(Session::new());
        let reply = app.run("url.show", &json!({ "url": url })).unwrap();
        assert_eq!(app.session.documents().len(), 2);
        let tabs = reply["tabs"].as_array().unwrap();
        assert_eq!(tabs.len(), 2);
        // The last file is in front, with its tab showing.
        assert_eq!(app.session.doc().unwrap().title, "Two");
        let shown: Vec<&str> = app.state.tab().unwrap().shown_glyphs().collect();
        assert_eq!(shown, ["C"]);
        let warnings = reply["warnings"].to_string();
        assert!(warnings.contains("missing"), "{warnings}");
        assert!(warnings.contains("one layer per glyph"), "{warnings}");
        // The first file's tab: A twice and B, A drawn in its backup layer (listed first).
        app.run("document.activate", &json!({ "index": 0 })).unwrap();
        let tab = app.state.tabs.last().unwrap();
        let shown: Vec<&str> = tab.shown_glyphs().collect();
        assert_eq!(shown, ["A", "A", "B"]);
        assert_eq!(tab.layers.get("A"), Some(&backup));
        assert_eq!(tab.layers.get("B"), None);
        // Showing the same file again adds a tab and opens nothing.
        let before = app.state.tabs.len();
        let again = format!("fontcraft://show/?path={}&glyph=B", enc(&one));
        let reply = app.run("url.show", &json!({ "url": again })).unwrap();
        assert_eq!(app.session.documents().len(), 2);
        assert_eq!(app.state.tabs.len(), before + 1);
        assert_eq!(reply["tabs"].as_array().map(Vec::len), Some(1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn url_show_refuses_other_actions_and_opens_nothing() {
        let mut app = FontcraftApp::new(Session::new());
        let err = app.run("url.show", &json!({ "url": "fontcraft://run/?command=file.save" })).unwrap_err();
        assert!(err.contains("refused"), "{err}");
        assert!(app.session.documents().is_empty());
        assert!(app.state.tabs.is_empty());
    }
}
