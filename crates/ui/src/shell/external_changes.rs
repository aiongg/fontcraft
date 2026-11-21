//! The External Changes dialog's state and command (`dialog.externalChanges`), and the check that raises it
//! (`io.watchSourceOnDisk`).
//!
//! When the window regains focus, [`on_focus`] compares the stamps of the open documents' files with the ones they
//! had when last read or written (no file is read). For each document whose files changed, and that was not already
//! asked about for the same state of its files, the dialog asks first: Review, Reject All or Update All. Review lists
//! the selected font's changed items grouped by layer, each ticked; Accept Selected takes the ticked items as one undo
//! step, Reject All keeps the document as it is. Activating an item (double-click, Enter) shows its glyph's stored
//! text: the file's lines marked as additions, the document's as removals.
//!
//! Nothing is reloaded without asking, whether or not the document has unsaved edits. Every action runs
//! `file.reviewExternalChanges`, which never lists what only the document changed. The dialog itself is
//! `dialogs::external_changes`.

use std::collections::BTreeSet;

use fontcraft_engine::cmd::external_changes::{DiskChange, changed_on_disk};
use serde_json::{Value, json};

use super::commands::UiCommand;
use crate::app::FontCraftApp;

/// The engine command every action runs.
const REVIEW: &str = "file.reviewExternalChanges";
/// Most lines of a glyph's text the dialog shows.
pub const MAX_TEXT_ROWS: usize = 5_000;
/// Most items one font lists.
pub const MAX_ITEMS: usize = 10_000;

pub const COMMAND: UiCommand = UiCommand {
    id: "dialog.externalChanges",
    label: "Review External Changes…",
    menu: &["File"],
    shortcut: None,
    params: "{font?: number, action?: \"review\" | \"updateAll\" | \"rejectAll\" | \"acceptSelected\" | \"showText\" | \"selectFont\" | \"tick\" | \"close\", glyph?: name, index?: number, keys?: [item key], on?: bool} without action: show the External Changes dialog for the open documents whose files changed on disk (else the active document), or for document font (its position among the open documents); refused when nothing changed. The actions answer the dialog as its buttons do: review lists the changed items, updateAll takes every change of every listed font, rejectAll and close keep the documents as they are, acceptSelected takes the ticked items of the selected font, showText shows the stored text of glyph, selectFont shows the items of the listed font at index, tick ticks (on, the default) or unticks the items with these keys",
};

/// The dialog's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExternalChanges {
    pub open: bool,
    /// Past the first question: the item list shows.
    pub reviewing: bool,
    /// The fonts the dialog is about, with their reports.
    pub fonts: Vec<ChangedFont>,
    /// The font whose items show (its position in `fonts`).
    pub selected: usize,
    /// The selected font's items, grouped.
    pub lines: Vec<Line>,
    /// Keys of the items the person unticked; everything else is accepted.
    pub unticked: BTreeSet<String>,
    /// The glyph whose stored text shows, and its lines as (op, text).
    pub text: Option<(String, Vec<(String, String)>)>,
    /// The last action's error.
    pub note: Option<String>,
    /// Moves with every change, so the dialog knows to show it again.
    pub revision: u64,
    /// The file states already asked about, one per document, so returning to the window does not ask again about
    /// the same change.
    seen: Vec<DiskChange>,
}

/// An open document whose files changed, with what `file.reviewExternalChanges` reported.
#[derive(Clone, Debug, PartialEq)]
pub struct ChangedFont {
    /// The document's id; its position can change while the dialog is open.
    pub doc: u64,
    pub title: String,
    pub modified: bool,
    pub report: Value,
}

/// One line of the item list.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    Header(String),
    Item(Item),
}

/// One reviewable change.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub key: String,
    pub label: String,
    /// What `accept` names for it.
    pub accept: Value,
    pub glyph: Option<String>,
}

/// The focus hook: when the window has just regained focus, look for files changed on disk.
pub fn on_focus(app: &mut FontCraftApp, ctx: &egui::Context) {
    if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(true)))) {
        check(app);
    }
}

/// Opens the dialog for the documents whose files changed since they were last read or written and that were not
/// asked about in that state already. Returns how many fonts it lists.
pub fn check(app: &mut FontCraftApp) -> usize {
    let fresh: Vec<DiskChange> = changed_on_disk(&app.session).into_iter().filter(|c| !app.view.extensions.external.seen.contains(c)).collect();
    if fresh.is_empty() {
        return 0;
    }
    let docs: Vec<u64> = fresh.iter().map(|c| c.doc).collect();
    let st = &mut app.view.extensions.external;
    for change in fresh {
        st.seen.retain(|s| s.doc != change.doc);
        st.seen.push(change);
    }
    let found = open_for(app, &docs);
    app.view_changed();
    found
}

/// Reviews each of `docs` and opens the dialog for those whose files changed something the document does not have.
/// A file that was only touched refreshes its stamp in the review and is not listed.
fn open_for(app: &mut FontCraftApp, docs: &[u64]) -> usize {
    let mut found = Vec::new();
    for &doc in docs {
        match review(app, doc, json!({})) {
            Ok(report) if report.get("changed").and_then(Value::as_bool) == Some(true) => found.push(ChangedFont {
                doc,
                title: report.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
                modified: report.get("modified").and_then(Value::as_bool).unwrap_or(false),
                report,
            }),
            Ok(_) => {}
            Err(e) => app.view.extensions.external.note = Some(e),
        }
    }
    if found.is_empty() {
        return 0;
    }
    let st = &mut app.view.extensions.external;
    for font in found {
        st.fonts.retain(|f| f.doc != font.doc);
        st.fonts.push(font);
    }
    st.open = true;
    st.selected = st.selected.min(st.fonts.len().saturating_sub(1));
    refresh_lines(st);
    st.fonts.len()
}

/// Runs `file.reviewExternalChanges` on the document with this id.
fn review(app: &mut FontCraftApp, doc: u64, mut params: Value) -> Result<Value, String> {
    let index = app.session.documents().iter().position(|d| d.id == doc).ok_or("the document was closed")?;
    if let Some(o) = params.as_object_mut() {
        o.insert("font".into(), json!(index));
    }
    app.session.execute(REVIEW, &params).map_err(|e| e.to_string())
}

fn index(params: &Value, key: &str) -> Result<Option<usize>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().and_then(|n| usize::try_from(n).ok()).map(Some).ok_or_else(|| format!("`{key}` is a position from 0")),
    }
}

/// Runs one `dialog.externalChanges` request.
pub fn run(app: &mut FontCraftApp, params: &Value) -> Result<(), String> {
    let result = act(app, params);
    if let Err(e) = &result {
        app.view.extensions.external.note = Some(e.clone());
    }
    let st = &mut app.view.extensions.external;
    st.revision = st.revision.wrapping_add(1);
    result
}

fn act(app: &mut FontCraftApp, params: &Value) -> Result<(), String> {
    let action = params.get("action").and_then(Value::as_str);
    match action {
        None => {
            let font = index(params, "font")?;
            let docs: Vec<u64> = match font {
                Some(i) => vec![app.session.documents().get(i).ok_or_else(|| format!("there is no open document {i}"))?.id],
                None => {
                    let changed: Vec<u64> = changed_on_disk(&app.session).iter().map(|c| c.doc).collect();
                    if changed.is_empty() { app.session.active().filter(|d| d.path.is_some()).map(|d| d.id).into_iter().collect() } else { changed }
                }
            };
            if open_for(app, &docs) == 0 {
                return Err("no open document changed on disk since it was read or saved".into());
            }
            app.view.extensions.external.reviewing = font.is_some();
        }
        Some("review") => app.view.extensions.external.reviewing = true,
        Some("rejectAll" | "close") => close(&mut app.view.extensions.external),
        Some("updateAll") => {
            let fonts: Vec<u64> = app.view.extensions.external.fonts.iter().map(|f| f.doc).collect();
            let mut failed = None;
            for doc in fonts {
                if let Err(e) = review(app, doc, json!({ "accept": "all" })) {
                    failed.get_or_insert(e);
                }
            }
            if let Some(e) = failed {
                return Err(e);
            }
            close(&mut app.view.extensions.external);
        }
        Some("acceptSelected") => {
            let st = &app.view.extensions.external;
            let doc = st.fonts.get(st.selected).map(|f| f.doc).ok_or("no font is selected")?;
            let accept: Vec<Value> = st
                .lines
                .iter()
                .filter_map(|l| match l {
                    Line::Item(item) if !st.unticked.contains(&item.key) => Some(item.accept.clone()),
                    _ => None,
                })
                .collect();
            review(app, doc, json!({ "accept": accept }))?;
            let st = &mut app.view.extensions.external;
            st.fonts.retain(|f| f.doc != doc);
            if st.fonts.is_empty() {
                close(st);
            } else {
                st.selected = 0;
                refresh_lines(st);
            }
        }
        Some("showText") => {
            let glyph = params.get("glyph").and_then(Value::as_str).ok_or("showText needs `glyph`")?.to_string();
            let st = &app.view.extensions.external;
            let doc = st.fonts.get(st.selected).map(|f| f.doc).ok_or("no font is selected")?;
            let report = review(app, doc, json!({ "glyph": glyph }))?;
            let lines: Vec<(String, String)> = report
                .get("text")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(MAX_TEXT_ROWS)
                .map(|l| {
                    let field = |k: &str| l.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                    (field("op"), field("text"))
                })
                .collect();
            app.view.extensions.external.text = Some((glyph, lines));
        }
        Some("selectFont") => {
            let at = index(params, "index")?.ok_or("selectFont needs `index`")?;
            let st = &mut app.view.extensions.external;
            if at >= st.fonts.len() {
                return Err(format!("the dialog lists no font {at}"));
            }
            st.selected = at;
            refresh_lines(st);
        }
        Some("tick") => {
            let on = params.get("on").and_then(Value::as_bool).unwrap_or(true);
            let keys = params.get("keys").and_then(Value::as_array).ok_or("tick needs `keys`")?;
            let st = &mut app.view.extensions.external;
            for key in keys.iter().filter_map(Value::as_str).take(MAX_ITEMS) {
                if on {
                    st.unticked.remove(key);
                } else {
                    st.unticked.insert(key.to_string());
                }
            }
        }
        Some(other) => return Err(format!("unknown action `{other}`")),
    }
    app.view.extensions.external.note = None;
    Ok(())
}

/// Closes the dialog; the files' states stay in `seen`, so they are not asked about again.
fn close(st: &mut ExternalChanges) {
    let seen = std::mem::take(&mut st.seen);
    let revision = st.revision;
    *st = ExternalChanges { seen, revision, ..Default::default() };
}

/// Rebuilds the item list of the selected font. Every item starts ticked.
fn refresh_lines(st: &mut ExternalChanges) {
    st.unticked.clear();
    st.text = None;
    st.lines = st.fonts.get(st.selected).map(|f| lines(&f.report)).unwrap_or_default();
}

/// The items of a report: a section per layer, then the glyphs' own data, added and removed glyphs, and the
/// font-level parts.
pub fn lines(report: &Value) -> Vec<Line> {
    let mut sections: Vec<(String, Vec<Item>)> = Vec::new();
    let mut count = 0usize;
    let mut add = |section: String, item: Item| {
        if count >= MAX_ITEMS {
            return;
        }
        count += 1;
        match sections.iter_mut().find(|(s, _)| *s == section) {
            Some((_, items)) => items.push(item),
            None => sections.push((section, vec![item])),
        }
    };
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let flag = |v: &Value, k: &str| v.get(k).and_then(Value::as_bool) == Some(true);
    let list = |v: &Value, k: &str| v.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    let mut glyph_data = Vec::new();
    let mut whole = Vec::new();
    for g in list(report, "glyphs").iter().take(MAX_ITEMS) {
        let name = s(g, "glyph");
        let mut mark = String::new();
        if flag(g, "conflict") {
            mark.push_str(", also changed here");
        }
        if flag(g, "locked") {
            mark.push_str(", locked");
        }
        let change = s(g, "change");
        if change != "changed" {
            let label = format!("{name} ({change}{mark})");
            whole.push(Item { key: format!("g:{name}"), label, accept: json!({ "glyph": name }), glyph: Some(name.clone()) });
            continue;
        }
        let suffix = if mark.is_empty() { String::new() } else { format!(" ({})", mark.trim_start_matches(", ")) };
        if flag(g, "attributes") {
            glyph_data.push(Item {
                key: format!("a:{name}"),
                label: format!("{name}{suffix}"),
                accept: json!({ "glyph": name, "attributes": true }),
                glyph: Some(name.clone()),
            });
        }
        for l in list(g, "layers").iter().take(MAX_ITEMS) {
            let (id, layer_change) = (s(l, "layer"), s(l, "change"));
            let label = if layer_change == "changed" { format!("{name}{suffix}") } else { format!("{name} (layer {layer_change}{mark})") };
            add(
                format!("Layer {}", s(l, "name")),
                Item { key: format!("l:{id}:{name}"), label, accept: json!({ "glyph": name, "layer": id }), glyph: Some(name.clone()) },
            );
        }
    }
    for item in glyph_data {
        add("Glyph data".into(), item);
    }
    for item in whole {
        add("Glyphs added or removed".into(), item);
    }
    for p in list(report, "parts").iter().take(MAX_ITEMS) {
        let part = s(p, "part");
        let conflict = if flag(p, "conflict") { " (also changed here)" } else { "" };
        add(
            "Font data".into(),
            Item { key: format!("p:{part}"), label: format!("{}{conflict}", part_label(&part)), accept: json!({ "part": part }), glyph: None },
        );
    }
    sections.into_iter().flat_map(|(title, items)| std::iter::once(Line::Header(title)).chain(items.into_iter().map(Line::Item))).collect()
}

fn part_label(part: &str) -> &str {
    match part {
        "info" => "Font info",
        "axes" => "Axes",
        "masters" => "Masters and metrics",
        "instances" => "Instances",
        "kerning" => "Kerning",
        "features" => "Features",
        "groups" => "Groups",
        "glyphOrder" => "Glyph order",
        "other" => "Other font data",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use fontcraft_engine::Session;

    use super::*;
    use crate::app::Msg;

    fn shell(app: &mut FontCraftApp, params: Value) -> Result<Value, String> {
        app.run_now(Msg::Shell(CommandRef::with_params(COMMAND.id, params)), false)
    }

    fn keys(app: &FontCraftApp) -> Vec<String> {
        app.view.extensions.external.lines.iter().filter_map(|l| if let Line::Item(i) = l { Some(i.key.clone()) } else { None }).collect()
    }

    #[test]
    fn a_file_changed_on_disk_opens_the_dialog_once_and_accepts_the_ticked_items() {
        let dir = std::env::temp_dir().join(format!("fontcraft-ui-next-external-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Test.glyphspackage");
        let mut app = FontCraftApp::with_edit_tab(Session::new(), Arc::new(Host::builder(crate::app::APP).build()));
        app.run_now(Msg::run("file.new", json!({})), false).unwrap();
        app.run_now(Msg::run("file.saveAs", json!({ "path": path })), false).unwrap();
        assert_eq!(check(&mut app), 0, "nothing changed yet");
        let mut other = Session::new();
        other.execute("file.open", &json!({ "path": path })).unwrap();
        other.execute("spacing.setWidth", &json!({ "glyphs": ["A", "B"], "value": 1234 })).unwrap();
        other.execute("glyph.newGlyph", &json!({ "name": "A.alt" })).unwrap();
        other.execute("file.save", &json!({})).unwrap();

        assert_eq!(check(&mut app), 1);
        let st = &app.view.extensions.external;
        assert!(st.open && !st.reviewing);
        let listed = keys(&app);
        assert!(listed.iter().any(|k| k.starts_with("l:") && k.ends_with(":A")), "{listed:?}");
        assert!(listed.iter().any(|k| k == "g:A.alt"), "{listed:?}");
        // The same state of the file is not asked about twice.
        shell(&mut app, json!({ "action": "close" })).unwrap();
        assert_eq!(check(&mut app), 0);

        shell(&mut app, json!({})).unwrap();
        shell(&mut app, json!({ "action": "review" })).unwrap();
        let untick: Vec<String> = keys(&app).into_iter().filter(|k| !k.ends_with(":A")).collect();
        shell(&mut app, json!({ "action": "tick", "on": false, "keys": untick })).unwrap();
        shell(&mut app, json!({ "action": "acceptSelected" })).unwrap();
        assert!(!app.view.extensions.external.open);
        let font = app.session.font().unwrap();
        let width = |name: &str| font.glyphs.get(name).unwrap().layers.first().unwrap().width;
        assert_eq!(width("A"), 1234.0);
        assert_ne!(width("B"), 1234.0);
        assert!(font.glyphs.get("A.alt").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
