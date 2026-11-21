//! Compare Fonts (M9.24): two masters, from one open document or from two, side by side. This module holds the
//! panel's view state and its commands; the rows are the projection's (`app::models::compare`), asked of
//! `compareFonts.diff` while the panel shows and again whenever a side or one of the two documents changes.
//!
//! | Command | Does |
//! |---|---|
//! | `edit.compareFonts {left?, right?}` | shows the panel on two masters; a side not given keeps the last one |
//! | `compareFonts.setView {selected?, collapsed?}` | selects a row, and collapses or expands the category headings |

use craft::ui::runtime::commands::CommandRef;
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use super::dock::PanelId;
use super::params::quoted;
use crate::app::FontCraftApp;

/// The categories in the panel's order: the `category` of `compareFonts.diff`.
pub const CATEGORIES: [&str; 4] = ["fontInfo", "masterMetrics", "glyph", "kerning"];
/// Longest category or row key `compareFonts.setView` takes, in bytes.
const MAX_KEY_BYTES: usize = 1024;

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "edit.compareFonts",
        label: "Compare Fonts…",
        menu: &["Edit"],
        shortcut: None,
        params: "{left?: {document?: index|title|path, master?: id|name}, right?: {document?, master?}} show the Compare Fonts panel on two masters of one or two open documents. A side not given keeps the panel's last one (the first time: the active document on the left, the next open document on the right, or the active document's second master when no other document is open); a document given without a master takes its first master. The rows are compareFonts.diff's for the two sides; ui.inspect reports the panel under compareFonts",
    },
    UiCommand {
        id: "compareFonts.setView",
        label: "Compare Fonts: Select Row",
        menu: &[],
        shortcut: None,
        params: "{selected?: {category: fontInfo|masterMetrics|glyph|kerning, key: string}|null, collapsed?: [category]} select a row of the Compare Fonts panel (null: none), which Use Left and Use Right act on; collapsed lists the category headings whose rows are hidden",
    },
];

/// One side: an open document, by session id, and one of its masters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompareSide {
    pub document: u64,
    /// A master id; empty for the document's first master.
    pub master: String,
}

/// The selected row, by the `category` and `key` `compareFonts.diff` gives it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectedRow {
    pub category: String,
    pub key: String,
}

/// The panel's state, in `ViewState::compare_fonts`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompareFontsState {
    pub left: Option<CompareSide>,
    pub right: Option<CompareSide>,
    /// The categories whose rows are hidden under their heading.
    pub collapsed: Vec<String>,
    pub selected: Option<SelectedRow>,
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    match command.id.as_str() {
        "edit.compareFonts" => open(app, &command.params),
        "compareFonts.setView" => set_view(app, &command.params),
        other => Err(format!("no Compare Fonts command `{other}`")),
    }
}

/// Whether the Compare Fonts panel shows.
pub fn shows(app: &FontCraftApp) -> bool {
    app.view.workspace.layout.panel_state(&PanelId::CompareFonts) != craft::behavior::dock::PanelState::Hidden
}

/// The position of the open document `document` (a session id).
pub fn position(app: &FontCraftApp, document: u64) -> Option<usize> {
    app.session.documents().iter().position(|d| d.id == document)
}

/// `side` while its document is open; a master the document no longer has becomes its first.
fn still_open(app: &FontCraftApp, side: Option<&CompareSide>) -> Option<CompareSide> {
    let side = side?;
    let doc = app.session.documents().iter().find(|d| d.id == side.document)?;
    let master = if doc.font.masters.iter().any(|m| m.id == side.master) { side.master.clone() } else { String::new() };
    Some(CompareSide { document: side.document, master })
}

/// The sides the panel first opens on.
fn default_sides(app: &FontCraftApp) -> Option<(CompareSide, CompareSide)> {
    let docs = app.session.documents();
    let active = app.session.active().or_else(|| docs.first())?;
    let left = CompareSide { document: active.id, master: String::new() };
    let right = match docs.iter().find(|d| d.id != active.id) {
        Some(other) => CompareSide { document: other.id, master: String::new() },
        None => CompareSide { document: active.id, master: active.font.masters.get(1).map(|m| m.id.clone()).unwrap_or_default() },
    };
    Some((left, right))
}

/// One side's parameters for the compare commands: `{document: position, master?}`.
pub fn side_value(index: usize, master: &str) -> Value {
    if master.is_empty() { json!({ "document": index }) } else { json!({ "document": index, "master": master }) }
}

/// One side's parameters for `compareFonts.diff`: what the command was given, the rest from `side`.
fn side_params(app: &FontCraftApp, given: Option<&Value>, side: &CompareSide) -> Value {
    let given = match given.filter(|v| !v.is_null()) {
        None => None,
        Some(Value::Object(m)) => Some(m),
        // `compareFonts.diff` says what is wrong with it.
        Some(other) => return other.clone(),
    };
    let field = |name: &str| given.and_then(|m| m.get(name)).filter(|v| !v.is_null());
    let mut out = Map::new();
    let document = field("document");
    match document {
        Some(d) => {
            out.insert("document".into(), d.clone());
        }
        None => {
            if let Some(i) = position(app, side.document) {
                out.insert("document".into(), json!(i));
            }
        }
    }
    match field("master") {
        Some(m) => {
            out.insert("master".into(), m.clone());
        }
        None if document.is_none() && !side.master.is_empty() => {
            out.insert("master".into(), json!(side.master));
        }
        None => {}
    }
    Value::Object(out)
}

/// A side as `compareFonts.diff` resolved it (`{document: position, master: id}`).
fn resolved(app: &FontCraftApp, v: Option<&Value>) -> Option<CompareSide> {
    let v = v?;
    let index = usize::try_from(v.get("document")?.as_u64()?).ok()?;
    let doc = app.session.documents().get(index)?;
    Some(CompareSide { document: doc.id, master: v.get("master").and_then(Value::as_str).unwrap_or_default().to_string() })
}

/// Resolves the sides with `compareFonts.diff`, which refuses a side it cannot find; a refused side changes nothing.
fn open(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let (left_default, right_default) = default_sides(app).ok_or("no document is open")?;
    let st = &app.view.compare_fonts;
    let left = still_open(app, st.left.as_ref()).unwrap_or(left_default);
    let right = still_open(app, st.right.as_ref()).unwrap_or(right_default);
    let params = json!({
        "left": side_params(app, p.get("left"), &left),
        "right": side_params(app, p.get("right"), &right),
        "categories": ["fontInfo"],
    });
    let out = app.session.query("compareFonts.diff", &params).map_err(|e| e.to_string())?;
    let (Some(left), Some(right)) = (resolved(app, out.get("left")), resolved(app, out.get("right"))) else {
        return Err("the compared documents are not open".into());
    };
    let st = &mut app.view.compare_fonts;
    if st.left.as_ref() != Some(&left) || st.right.as_ref() != Some(&right) {
        st.selected = None;
    }
    st.left = Some(left);
    st.right = Some(right);
    // A refused move (the layout has no room) leaves the panel where it was.
    let _ = app.view.workspace.set_visible(PanelId::CompareFonts, true, None, false, |panel| panel.permissions());
    Ok(())
}

fn key_text(v: &Value, what: &str) -> Result<String, String> {
    let s = v.as_str().ok_or_else(|| format!("`{what}` must be a string"))?;
    if s.len() > MAX_KEY_BYTES {
        return Err(format!("`{what}` is longer than {MAX_KEY_BYTES} bytes"));
    }
    Ok(s.to_string())
}

fn category(v: &Value, what: &str) -> Result<String, String> {
    let c = key_text(v, what)?;
    if !CATEGORIES.contains(&c.as_str()) {
        return Err(format!("unknown category `{}`: use {}", quoted(&c), CATEGORIES.join(", ")));
    }
    Ok(c)
}

fn set_view(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let selected = match p.get("selected") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::Object(row)) => {
            let category = category(row.get("category").unwrap_or(&Value::Null), "selected.category")?;
            let key = key_text(row.get("key").unwrap_or(&Value::Null), "selected.key")?;
            Some(Some(SelectedRow { category, key }))
        }
        Some(_) => return Err("`selected` must be {category, key} or null".into()),
    };
    let collapsed = match p.get("collapsed").filter(|v| !v.is_null()) {
        None => None,
        Some(Value::Array(list)) if list.len() <= CATEGORIES.len() => {
            let mut out = Vec::new();
            for c in list {
                let c = category(c, "collapsed")?;
                if !out.contains(&c) {
                    out.push(c);
                }
            }
            Some(out)
        }
        Some(_) => return Err("`collapsed` must be a list of categories".into()),
    };
    if selected.is_none() && collapsed.is_none() {
        return Err("give `selected` or `collapsed`".into());
    }
    let st = &mut app.view.compare_fonts;
    if let Some(s) = selected {
        st.selected = s;
    }
    if let Some(c) = collapsed {
        st.collapsed = c;
    }
    Ok(())
}

/// The panel as `ui.inspect` reports it, under `compareFonts`.
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    let st = &app.view.compare_fonts;
    let model = &app.models.compare_fonts;
    let side = |s: &Option<CompareSide>| s.as_ref().and_then(|s| position(app, s.document).map(|i| side_value(i, &s.master)));
    state.insert(
        "compareFonts".into(),
        json!({
            "open": shows(app),
            "left": side(&st.left),
            "right": side(&st.right),
            "rows": model.rows.len(),
            "truncated": model.truncated,
            "collapsed": st.collapsed,
            "selected": st.selected.as_ref().map(|s| json!({"category": s.category, "key": s.key})),
            "error": model.error,
        }),
    );
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::sync::Arc;

    use craft::host::Host;
    use fontcraft_engine::Session;
    use fontcraft_engine::model::{Font, Layer, Master};

    use super::*;
    use crate::app::{APP, Msg};

    fn shell(id: &str, params: Value) -> Msg {
        Msg::Shell(CommandRef::with_params(id, params))
    }

    /// "Untitled" (A 600 wide) and "Other" (A 640 wide, x-height 520), Untitled active.
    fn two_documents() -> FontCraftApp {
        let mut session = Session::with_document();
        let mut other = Font::new("Other");
        other.glyph_mut("A").unwrap().layer_mut("m01").unwrap().width = 640.0;
        other.set_metric_pos("m01", "x-height", 520.0).unwrap();
        session.add_document(other, None).unwrap();
        session.execute("document.activate", &json!({"index": 0})).unwrap();
        FontCraftApp::with_edit_tab(session, Arc::new(Host::builder(APP).build()))
    }

    #[test]
    fn compare_fonts_lists_compare_fonts_diff_and_use_left_runs_on_the_selected_row() {
        let mut app = two_documents();
        app.run_now(shell("edit.compareFonts", json!({})), false).unwrap();
        assert!(shows(&app));
        let diff = app.session.query("compareFonts.diff", &json!({"left": {"document": 0}, "right": {"document": 1}})).unwrap();
        let model = &app.models.compare_fonts;
        assert_eq!(model.rows.len(), diff["rows"].as_array().unwrap().len(), "the panel lists what compareFonts.diff returns");
        let a = model.rows.iter().find(|r| r.category == "glyph" && r.key == "A").unwrap();
        assert_eq!(a.label, "A (width)");
        assert!(a.left.as_deref().unwrap().starts_with("width 600"), "{a:?}");
        assert!(crate::panels::compare_fonts::use_command(model, true).is_none(), "nothing is selected");

        // Use Left on the selected row writes the left master's width to the right document; the rows follow.
        app.run_now(shell("compareFonts.setView", json!({"selected": {"category": "glyph", "key": "A"}})), false).unwrap();
        let msg = crate::panels::compare_fonts::use_command(&app.models.compare_fonts, true).unwrap();
        app.run_now(msg, false).unwrap();
        assert_eq!(app.session.documents()[1].font.glyph("A").unwrap().layer("m01").unwrap().width, 600.0);
        let model = &app.models.compare_fonts;
        assert!(!model.rows.iter().any(|r| r.category == "glyph" && r.key == "A"), "the difference is gone from the list");
        assert_eq!(model.selected, None, "the selected row is no longer listed");

        // A collapsed heading hides its rows.
        app.run_now(shell("compareFonts.setView", json!({"collapsed": ["masterMetrics"]})), false).unwrap();
        let model = &app.models.compare_fonts;
        assert!(
            model.lines.iter().all(|l| !matches!(l, crate::app::models::compare::DiffLine::Row(i) if model.rows[*i].category == "masterMetrics"))
        );

        // A refused side changes nothing.
        let before = app.view.compare_fonts.clone();
        assert!(app.run_now(shell("edit.compareFonts", json!({"right": {"master": "Nonesuch"}})), false).is_err());
        assert!(app.run_now(shell("compareFonts.setView", json!({"collapsed": ["colour"]})), false).is_err());
        assert_eq!(app.view.compare_fonts, before);
    }

    #[test]
    fn compare_fonts_with_one_document_compares_its_first_two_masters() {
        let mut f = Font::new("Two");
        Arc::make_mut(&mut f.masters).push(Master { id: "m02".into(), name: "Bold".into(), ..Default::default() });
        f.glyph_mut("A").unwrap().layers.push(Layer::new("m02", 700.0));
        let mut session = Session::new();
        session.add_document(f, None).unwrap();
        let mut app = FontCraftApp::with_edit_tab(session, Arc::new(Host::builder(APP).build()));
        app.run_now(shell("edit.compareFonts", json!({})), false).unwrap();
        let st = &app.view.compare_fonts;
        assert_eq!((st.left.as_ref().unwrap().master.as_str(), st.right.as_ref().unwrap().master.as_str()), ("m01", "m02"));
        assert!(app.models.compare_fonts.rows.iter().any(|r| r.category == "glyph" && r.key == "A"));

        // A side given by master name keeps the other side.
        app.run_now(shell("edit.compareFonts", json!({"left": {"master": "Bold"}})), false).unwrap();
        assert_eq!(app.view.compare_fonts.left.as_ref().unwrap().master, "m02");
        assert!(!app.models.compare_fonts.rows.iter().any(|r| r.category == "glyph"));
    }

    #[test]
    fn compare_family_reads_the_report_on_open_and_update_and_opens_a_glyph() {
        let mut app = two_documents();
        app.run_now(shell("edit.compareFamily", json!({})), false).unwrap();
        assert!(crate::shell::compare_family::shows(&app));
        let report = app.session.query("compareFamily.report", &json!({"attribute": "componentNames", "filter": "all"})).unwrap();
        let model = &app.models.compare_family;
        assert_eq!(model.columns.len(), report["columns"].as_array().unwrap().len());
        assert_eq!(model.rows.len(), report["rows"].as_array().unwrap().len());

        // Editing a document does not read the table again; Update does.
        let read = model.read;
        app.run_now(Msg::run("glyph.newGlyph", json!({"name": "B.alt"})), false).unwrap();
        assert_eq!(app.models.compare_family.read, read);
        app.run_now(shell("compareFamily.setAttribute", json!({"attribute": "anchors"})), false).unwrap();
        assert_eq!(app.models.compare_family.attribute, "anchors");
        assert!(app.run_now(shell("compareFamily.setFilter", json!({"filter": "some"})), false).is_err());
        assert_eq!(app.view.compare_family.filter, "all", "a refused filter changes nothing");

        // Opening a glyph activates its document and shows the glyph.
        app.run_now(shell("compareFamily.openGlyph", json!({"document": 1, "glyph": "A"})), false).unwrap();
        assert_eq!(app.session.active().unwrap().id, app.session.documents()[1].id);
        assert_eq!(app.view.glyph.as_deref(), Some("A"));
        assert!(app.run_now(shell("compareFamily.openGlyph", json!({"document": 1, "glyph": "Nonesuch"})), false).is_err());
    }
}
