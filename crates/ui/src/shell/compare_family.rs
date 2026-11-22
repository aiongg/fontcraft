//! Compare Family (M9.24): one attribute of every glyph across the open documents of a family, one column per
//! document and one row per glyph. This module holds the panel's view state and its commands; the table is the
//! projection's (`app::models::compare`), read with `compareFamily.report` while the panel shows: when it opens, when
//! the attribute or the filter changes and on Update. Editing a document does not read it again.

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::cmd::encodings::resolve_documents;
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use super::dock::PanelId;
use super::params::{flag, quoted, string};
use crate::app::FontCraftApp;

/// The attributes the table can show: the `attribute` of `compareFamily.report`.
pub const ATTRIBUTES: [&str; 5] = ["componentNames", "anchors", "leftKerningGroup", "rightKerningGroup", "glyphs"];
/// The row filters: the `filter` of `compareFamily.report`.
pub const FILTERS: [&str; 3] = ["all", "missingInOne", "ignoreIfMissingInOne"];

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "edit.compareFamily",
        label: "Compare Family…",
        menu: &["Edit"],
        shortcut: None,
        params: "{} show the Compare Family panel: one column per open document (its first master) and one row per glyph, showing the chosen attribute. The table is read now, when the attribute or filter changes and on Update, not after every edit. ui.inspect reports the panel under compareFamily",
    },
    UiCommand {
        id: "compareFamily.setAttribute",
        label: "Compare Family: Attribute",
        menu: &[],
        shortcut: None,
        params: "{attribute: \"componentNames\"|\"anchors\"|\"leftKerningGroup\"|\"rightKerningGroup\"|\"glyphs\"} show this glyph attribute in the Compare Family table and read the table again",
    },
    UiCommand {
        id: "compareFamily.setFilter",
        label: "Compare Family: Filter",
        menu: &[],
        shortcut: None,
        params: "{filter: \"all\"|\"missingInOne\"|\"ignoreIfMissingInOne\"} all: glyphs with a value in some document; missingInOne: glyphs absent from at least one document; ignoreIfMissingInOne: glyphs present in every document. Reads the table again",
    },
    UiCommand {
        id: "compareFamily.togglePin",
        label: "Compare Family: Keep on Top",
        menu: &[],
        shortcut: None,
        params: "{on?: bool (default: the opposite of now)} keep the Compare Family panel above the document windows",
    },
    UiCommand {
        id: "compareFamily.update",
        label: "Update",
        menu: &[],
        shortcut: None,
        params: "{} read the Compare Family table again from the open documents",
    },
    UiCommand {
        id: "compareFamily.openGlyph",
        label: "Open Glyph from Compare Family",
        menu: &[],
        shortcut: None,
        params: "{document: index|title|path, glyph: name} make the open document active and show the glyph there; refused when the document has no such glyph",
    },
];

/// The panel's state, in `ViewState::compare_family`.
#[derive(Clone, Debug, PartialEq)]
pub struct CompareFamilyState {
    /// An [`ATTRIBUTES`] id.
    pub attribute: String,
    /// A [`FILTERS`] id.
    pub filter: String,
    /// The panel stays above the document windows.
    pub pinned: bool,
    /// Moves with every read the table needs: the projection reads it again only then.
    pub reads: u64,
}

impl Default for CompareFamilyState {
    fn default() -> Self {
        Self { attribute: "componentNames".into(), filter: "all".into(), pinned: false, reads: 0 }
    }
}

/// Whether the Compare Family panel shows.
pub fn shows(app: &FontCraftApp) -> bool {
    app.view.workspace.layout.panel_state(&PanelId::CompareFamily) != craft::behavior::dock::PanelState::Hidden
}

fn one_of(p: &Value, key: &str, ids: &[&str]) -> Result<String, String> {
    let v = string(p, key)?.ok_or_else(|| format!("`{key}` is required"))?;
    if !ids.contains(&v) {
        return Err(format!("unknown {key} `{}`: use {}", quoted(v), ids.join(", ")));
    }
    Ok(v.to_string())
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = &command.params;
    let st = &mut app.view.compare_family;
    match command.id.as_str() {
        "edit.compareFamily" => {
            st.reads = st.reads.wrapping_add(1);
            crate::shell::dock::show_window(&mut app.view.workspace, PanelId::CompareFamily, true, app.view.canvas.rect);
        }
        "compareFamily.update" => st.reads = st.reads.wrapping_add(1),
        "compareFamily.setAttribute" => {
            st.attribute = one_of(p, "attribute", &ATTRIBUTES)?;
            st.reads = st.reads.wrapping_add(1);
        }
        "compareFamily.setFilter" => {
            st.filter = one_of(p, "filter", &FILTERS)?;
            st.reads = st.reads.wrapping_add(1);
        }
        "compareFamily.togglePin" => st.pinned = flag(p, "on")?.unwrap_or(!st.pinned),
        "compareFamily.openGlyph" => return open_glyph(app, p),
        other => return Err(format!("no Compare Family command `{other}`")),
    }
    Ok(())
}

/// Activates the document and shows the glyph; a refused document or glyph changes nothing.
fn open_glyph(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let document = p.get("document").filter(|v| !v.is_null()).ok_or("`document` is required: an open document's position, title or path")?;
    let glyph = string(p, "glyph")?.ok_or("`glyph` is required")?;
    let found = resolve_documents(&app.session, Some(&Value::Array(vec![document.clone()])))?;
    let index = found.first().and_then(|id| app.session.documents().iter().position(|d| d.id == *id)).ok_or("no such open document")?;
    let doc = app.session.documents().get(index).ok_or("no such open document")?;
    if doc.font.glyph(glyph).is_none() {
        return Err(format!("`{}` has no glyph `{}`", doc.title, quoted(glyph)));
    }
    let glyph = glyph.to_string();
    // Through the app, so the newly active document's tab set shows before the glyph goes into it.
    app.apply_msg(FontCraftApp::route(CommandRef::with_params("document.activate", json!({ "index": index }))))?;
    super::tabs::show_glyph(app, glyph)
}

/// The panel as `ui.inspect` reports it, under `compareFamily`.
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    let st = &app.view.compare_family;
    let model = &app.models.compare_family;
    let columns: Vec<Value> = model.columns.iter().map(|c| json!({"document": c.document, "title": c.title, "master": c.master})).collect();
    state.insert(
        "compareFamily".into(),
        json!({
            "open": shows(app),
            "attribute": st.attribute,
            "filter": st.filter,
            "pinned": st.pinned,
            "columns": columns,
            "total": model.total,
            "rows": model.rows.len(),
            "error": model.error,
        }),
    );
}
