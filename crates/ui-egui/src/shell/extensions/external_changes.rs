//! The External Changes dialog (`dialog.externalChanges`) and the check that raises it
//! (`io.watchSourceOnDisk`).
//!
//! When the window regains focus, [`on_focus`] compares the stamps of the open documents' files
//! with the ones they had when last read or written (no file is read). For each document whose
//! files changed, and that was not already asked about for the same state of its files, the
//! dialog asks first: Review, Reject All or Update All. Review lists the selected font's changed
//! items grouped by layer, each with a tick; Accept Selected takes the ticked items as one undo
//! step, Reject All keeps the document as it is. Double-clicking an item shows its glyph's stored
//! text, the file's lines marked as additions and the document's as removals.
//!
//! Nothing is reloaded without asking, whether or not the document has unsaved edits. Every
//! action runs `file.reviewExternalChanges`, which never lists what only the document changed.

use std::collections::BTreeSet;

use egui::{Align2, RichText, Sense};
use fontcraft_engine::cmd::external_changes::{DiskChange, changed_on_disk};
use serde_json::{Value, json};

use super::super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::panels::Pending;

/// The engine command every action runs.
const REVIEW: &str = "file.reviewExternalChanges";
/// Most lines of a glyph's text the dialog shows.
const MAX_TEXT_ROWS: usize = 5_000;
/// Rows of the item list and the text view visible without scrolling.
const VISIBLE_ROWS: f32 = 16.0;

pub const COMMAND: UiCommand = (
    "dialog.externalChanges",
    "Review External Changes…",
    &["File"],
    "",
    "{font?: number, action?: \"review\" | \"updateAll\" | \"rejectAll\" | \"acceptSelected\" | \"showText\" | \"close\", glyph?: name} without action: show the External Changes dialog for the open documents whose files changed on disk (else the active document), or for document font (its position among the open documents); refused when nothing changed. The actions answer the dialog as its buttons do: review lists the changed items, updateAll takes every change of every listed font, rejectAll and close keep the documents as they are, acceptSelected takes the ticked items of the selected font, showText shows the stored text of glyph → {fonts: [title]}",
);

/// The dialog's state, kept in [`super::ExtensionsState`] for the session only.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExternalChangesState {
    pub open: bool,
    /// Past the first prompt: the item list shows.
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
    /// The file states already asked about, one per document, so returning to the window does
    /// not ask again about the same change.
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
pub fn on_focus(app: &mut FontcraftApp, ctx: &egui::Context) {
    if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(true)))) {
        check(app);
    }
}

/// Opens the dialog for the documents whose files changed since they were last read or written
/// and that were not asked about in that state already. Returns how many fonts it lists.
pub fn check(app: &mut FontcraftApp) -> usize {
    let fresh: Vec<DiskChange> = changed_on_disk(&app.session).into_iter().filter(|c| !state(app).seen.contains(c)).collect();
    if fresh.is_empty() {
        return 0;
    }
    let docs: Vec<u64> = fresh.iter().map(|c| c.doc).collect();
    let st = state(app);
    for change in fresh {
        st.seen.retain(|s| s.doc != change.doc);
        st.seen.push(change);
    }
    open_for(app, &docs)
}

fn state(app: &mut FontcraftApp) -> &mut ExternalChangesState {
    &mut app.state.panels.extensions.external
}

/// Reviews each of `docs` and shows the dialog for those whose files changed something the
/// document does not have. A file that was only touched refreshes its stamp in the review and
/// is not listed.
fn open_for(app: &mut FontcraftApp, docs: &[u64]) -> usize {
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
            Err(e) => state(app).note = Some(e),
        }
    }
    if found.is_empty() {
        return 0;
    }
    let st = state(app);
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
fn review(app: &mut FontcraftApp, doc: u64, mut params: Value) -> Result<Value, String> {
    let index = app.session.documents().iter().position(|d| d.id == doc).ok_or("the document was closed")?;
    if let Some(o) = params.as_object_mut() {
        o.insert("font".into(), json!(index));
    }
    app.run(REVIEW, &params)
}

pub fn run(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let action = args.string("action")?;
    match action {
        None => {
            let docs: Vec<u64> = match args.index("font")? {
                Some(i) => {
                    let i = usize::try_from(i).map_err(|_| args.err("`font` is out of range"))?;
                    vec![app.session.documents().get(i).ok_or_else(|| args.err(format!("there is no open document {i}")))?.id]
                }
                None => {
                    let changed: Vec<u64> = changed_on_disk(&app.session).iter().map(|c| c.doc).collect();
                    if changed.is_empty() { app.session.active().filter(|d| d.path.is_some()).map(|d| d.id).into_iter().collect() } else { changed }
                }
            };
            if open_for(app, &docs) == 0 {
                return Err(args.err("no open document changed on disk since it was read or saved"));
            }
            state(app).reviewing = args.index("font")?.is_some();
        }
        Some("review") => state(app).reviewing = true,
        Some("rejectAll" | "close") => close(state(app)),
        Some("updateAll") => {
            let fonts: Vec<u64> = state(app).fonts.iter().map(|f| f.doc).collect();
            let mut failed = Vec::new();
            for doc in fonts {
                if let Err(e) = review(app, doc, json!({ "accept": "all" })) {
                    failed.push(e);
                }
            }
            if let Some(e) = failed.into_iter().next() {
                state(app).note = Some(e.clone());
                return Err(e);
            }
            close(state(app));
        }
        Some("acceptSelected") => {
            let st = state(app);
            let Some(font) = st.fonts.get(st.selected) else { return Err(args.err("no font is selected")) };
            let doc = font.doc;
            let accept: Vec<Value> = st
                .lines
                .iter()
                .filter_map(|l| match l {
                    Line::Item(item) if !st.unticked.contains(&item.key) => Some(item.accept.clone()),
                    _ => None,
                })
                .collect();
            if let Err(e) = review(app, doc, json!({ "accept": accept })) {
                state(app).note = Some(e.clone());
                return Err(e);
            }
            let st = state(app);
            st.fonts.retain(|f| f.doc != doc);
            if st.fonts.is_empty() {
                close(st);
            } else {
                st.selected = 0;
                refresh_lines(st);
            }
        }
        Some("showText") => {
            let glyph = args.string("glyph")?.ok_or_else(|| args.err("showText needs `glyph`"))?.to_string();
            let st = state(app);
            let Some(doc) = st.fonts.get(st.selected).map(|f| f.doc) else { return Err(args.err("no font is selected")) };
            let report = review(app, doc, json!({ "glyph": glyph }))?;
            let lines: Vec<(String, String)> = report
                .get("text")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .take(MAX_TEXT_ROWS)
                        .map(|l| {
                            let field = |k: &str| l.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                            (field("op"), field("text"))
                        })
                        .collect()
                })
                .unwrap_or_default();
            state(app).text = Some((glyph, lines));
        }
        Some(other) => return Err(args.err(format!("unknown action `{other}`"))),
    }
    let titles: Vec<String> = state(app).fonts.iter().map(|f| f.title.clone()).collect();
    Ok(json!({ "fonts": titles }))
}

/// Closes the dialog; the files' states stay in `seen`, so they are not asked about again.
fn close(st: &mut ExternalChangesState) {
    let seen = std::mem::take(&mut st.seen);
    *st = ExternalChangesState { seen, ..Default::default() };
}

/// Rebuilds the item list of the selected font: a section per layer, then the glyphs' own data,
/// added and removed glyphs, and the font-level parts. Every item starts ticked.
fn refresh_lines(st: &mut ExternalChangesState) {
    st.unticked.clear();
    st.text = None;
    st.lines = st.fonts.get(st.selected).map(|f| lines(&f.report)).unwrap_or_default();
}

fn lines(report: &Value) -> Vec<Line> {
    let mut sections: Vec<(String, Vec<Item>)> = Vec::new();
    let mut add = |section: String, item: Item| match sections.iter_mut().find(|(s, _)| *s == section) {
        Some((_, items)) => items.push(item),
        None => sections.push((section, vec![item])),
    };
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let empty = Vec::new();
    let mut glyph_data = Vec::new();
    let mut whole = Vec::new();
    for g in report.get("glyphs").and_then(Value::as_array).unwrap_or(&empty) {
        let name = s(g, "glyph");
        let mut mark = String::new();
        if g.get("conflict").and_then(Value::as_bool) == Some(true) {
            mark.push_str(", also changed here");
        }
        if g.get("locked").and_then(Value::as_bool) == Some(true) {
            mark.push_str(", locked");
        }
        let change = s(g, "change");
        if change != "changed" {
            let label = format!("{name} ({change}{mark})");
            whole.push(Item { key: format!("g:{name}"), label, accept: json!({ "glyph": name }), glyph: Some(name.clone()) });
            continue;
        }
        let suffix = if mark.is_empty() { String::new() } else { format!(" ({})", mark.trim_start_matches(", ")) };
        if g.get("attributes").and_then(Value::as_bool) == Some(true) {
            glyph_data.push(Item {
                key: format!("a:{name}"),
                label: format!("{name}{suffix}"),
                accept: json!({ "glyph": name, "attributes": true }),
                glyph: Some(name.clone()),
            });
        }
        for l in g.get("layers").and_then(Value::as_array).unwrap_or(&empty) {
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
    for p in report.get("parts").and_then(Value::as_array).unwrap_or(&empty) {
        let part = s(p, "part");
        let conflict = if p.get("conflict").and_then(Value::as_bool) == Some(true) { " (also changed here)" } else { "" };
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

enum Event {
    Toggle(String),
    Select(usize),
    Run(Value),
}

/// Draws the dialog while it is open. Its buttons queue `dialog.externalChanges` actions.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !state(app).open {
        return;
    }
    let FontcraftApp { state: ui_state, tokens, .. } = app;
    let st = &mut ui_state.panels.extensions.external;
    let mut events = Vec::new();
    let mut open = true;
    let row_height = tokens.list_row_height;
    egui::Window::new("External Changes")
        .id(egui::Id::new("dialog.externalChanges"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .pivot(Align2::CENTER_TOP)
        .default_pos(egui::pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            let action = |name: &str| Event::Run(json!({ "action": name }));
            if !st.reviewing {
                let titles: Vec<&str> = st.fonts.iter().map(|f| f.title.as_str()).collect();
                ui.label(match titles.as_slice() {
                    [one] => format!("{one} was changed on disk by another program."),
                    _ => format!("{} open fonts were changed on disk by another program: {}.", titles.len(), titles.join(", ")),
                });
                if st.fonts.iter().any(|f| f.modified) {
                    ui.label(RichText::new("Unsaved edits are kept; only the changes made on disk are offered.").color(tokens.text_dim));
                }
                ui.add_space(tokens.dialog_spacing);
                ui.horizontal(|ui| {
                    if ui.button("Review…").clicked() {
                        events.push(action("review"));
                    }
                    if ui.button("Reject All").clicked() {
                        events.push(action("rejectAll"));
                    }
                    if ui.button("Update All").clicked() {
                        events.push(action("updateAll"));
                    }
                });
            } else {
                if st.fonts.len() > 1 {
                    ui.horizontal_wrapped(|ui| {
                        for (i, f) in st.fonts.iter().enumerate() {
                            if ui.selectable_label(i == st.selected, f.title.as_str()).clicked() {
                                events.push(Event::Select(i));
                            }
                        }
                    });
                    ui.add_space(tokens.dialog_spacing);
                }
                egui::ScrollArea::vertical()
                    .id_salt("externalChanges.items")
                    .max_height(row_height * VISIBLE_ROWS)
                    .auto_shrink([false, true])
                    .show_rows(ui, row_height, st.lines.len(), |ui, range| {
                        for line in st.lines.get(range).unwrap_or_default() {
                            match line {
                                Line::Header(title) => {
                                    ui.strong(title.as_str());
                                }
                                Line::Item(item) => {
                                    ui.horizontal(|ui| {
                                        let mut ticked = !st.unticked.contains(&item.key);
                                        if ui.checkbox(&mut ticked, "").changed() {
                                            events.push(Event::Toggle(item.key.clone()));
                                        }
                                        let label = ui.label(item.label.as_str()).interact(Sense::click());
                                        if label.double_clicked()
                                            && let Some(glyph) = &item.glyph
                                        {
                                            events.push(Event::Run(json!({ "action": "showText", "glyph": glyph })));
                                        }
                                    });
                                }
                            }
                        }
                    });
                if let Some((glyph, text)) = &st.text {
                    ui.add_space(tokens.dialog_spacing);
                    ui.strong(format!("{glyph}: the file's lines (+) and the document's (−)"));
                    egui::ScrollArea::both().id_salt("externalChanges.text").max_height(row_height * VISIBLE_ROWS).show_rows(
                        ui,
                        row_height,
                        text.len(),
                        |ui, range| {
                            for (op, line) in text.get(range).unwrap_or_default() {
                                let color = match op.as_str() {
                                    "+" => tokens.accent,
                                    "-" => tokens.error_text,
                                    _ => tokens.text_dim,
                                };
                                ui.label(RichText::new(format!("{op} {line}")).monospace().color(color));
                            }
                        },
                    );
                }
                ui.add_space(tokens.dialog_spacing);
                ui.horizontal(|ui| {
                    if ui.button("Reject All").clicked() {
                        events.push(action("rejectAll"));
                    }
                    if ui.button("Accept Selected").clicked() {
                        events.push(action("acceptSelected"));
                    }
                });
            }
            if let Some(note) = &st.note {
                ui.add_space(tokens.dialog_spacing);
                ui.label(RichText::new(note.as_str()).color(tokens.error_text));
            }
        });
    if !open {
        events.push(Event::Run(json!({ "action": "close" })));
    }
    for event in events {
        match event {
            Event::Toggle(key) => {
                if !st.unticked.remove(&key) {
                    st.unticked.insert(key);
                }
            }
            Event::Select(i) => {
                st.selected = i;
                refresh_lines(st);
            }
            Event::Run(params) => pending.push((COMMAND.0, params)),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use fontcraft_engine::Session;

    #[test]
    fn a_file_changed_on_disk_opens_the_dialog_once_and_accepts_the_ticked_items() {
        let dir = std::env::temp_dir().join(format!("fontcraft-ui-external-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Test.glyphspackage");
        let mut app = FontcraftApp::default();
        app.run("file.new", &json!({})).unwrap();
        app.run("file.saveAs", &json!({ "path": path })).unwrap();
        assert_eq!(check(&mut app), 0, "nothing changed yet");
        let mut other = Session::new();
        other.execute("file.open", &json!({ "path": path })).unwrap();
        other.execute("spacing.setWidth", &json!({ "glyphs": ["A", "B"], "value": 1234 })).unwrap();
        other.execute("glyph.newGlyph", &json!({ "name": "A.alt" })).unwrap();
        other.execute("file.save", &json!({})).unwrap();

        assert_eq!(check(&mut app), 1);
        let st = &app.state.panels.extensions.external;
        assert!(st.open && !st.reviewing);
        let keys: Vec<&str> = st.lines.iter().filter_map(|l| if let Line::Item(i) = l { Some(i.key.as_str()) } else { None }).collect();
        assert!(keys.iter().any(|k| k.starts_with("l:") && k.ends_with(":A")), "{keys:?}");
        assert!(keys.contains(&"g:A.alt"), "{keys:?}");
        // The same state of the file is not asked about twice.
        app.run("dialog.externalChanges", &json!({ "action": "close" })).unwrap();
        assert_eq!(check(&mut app), 0);

        app.run("dialog.externalChanges", &json!({})).unwrap();
        app.run("dialog.externalChanges", &json!({ "action": "review" })).unwrap();
        let st = &mut app.state.panels.extensions.external;
        let untick: Vec<String> =
            st.lines.iter().filter_map(|l| if let Line::Item(i) = l { Some(i.key.clone()) } else { None }).filter(|k| !k.ends_with(":A")).collect();
        st.unticked.extend(untick);
        app.run("dialog.externalChanges", &json!({ "action": "acceptSelected" })).unwrap();
        assert!(!app.state.panels.extensions.external.open);
        let font = app.session.font().unwrap();
        let width = |name: &str| font.glyphs.get(name).unwrap().layers.first().unwrap().width;
        assert_eq!(width("A"), 1234.0);
        assert_ne!(width("B"), 1234.0);
        assert!(font.glyphs.get("A.alt").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
