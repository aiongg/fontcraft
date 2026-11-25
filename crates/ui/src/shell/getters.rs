//! Read-only getters for agents (`editView.getText`, `editView.getFeatures`, `fontView.getState`,
//! `transform.getSettings`, `view.getDisplayOptions`, `tool.list`) and the `ui.inspect` keys that report the same state (`display`,
//! `switches`, `selectedGlyphs`, `editing`, `interaction`, `dialog`, `transform`, `fontView`).
//!
//! A getter is a shell command that changes nothing and returns its reply: `FontCraftApp::apply` answers it through
//! [`find`] before any shell module runs, so `engine.execute` returns the reply. The replies keep the shapes the
//! earlier window gave them.

use craft::ui::runtime::commands::CommandRef;
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use super::palette::{OriginMode, TransformSettings};
use crate::app::FontCraftApp;
use crate::app::font_view::Column;

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Builds a getter's reply.
pub type Getter = fn(&FontCraftApp) -> Value;

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "editView.getText",
        label: "Get Tab Text",
        menu: &[],
        shortcut: None,
        params: "{} the edit view's text → {items: [glyph name | {placeholder} | {lineBreak} | {missing: char}], text (with /name escapes), cursor, selection: [start, end], current (the edited glyph), textMode, direction, kerning, …} as ui.inspect reports it under editText",
    },
    UiCommand {
        id: "editView.getFeatures",
        label: "Get Preview Features",
        menu: &[],
        shortcut: None,
        params: "{} the edit view's Features menu → {compiled: bool (the document has a feature compile), current: bool (the font has not changed since), features: [{tag, table: \"GSUB\"|\"GPOS\", active: bool}], active: [tag], localizedForms: bool, script, language, languageSystems: [[script, language]], shapingError?: string}",
    },
    UiCommand {
        id: "fontView.getState",
        label: "Get Font View State",
        menu: &[],
        shortcut: None,
        params: "{} → {showing, mode, cellSize, master, sort, columns, hidden, sidebarEntries, search, selected: [names], visible: [names in display order], counts: {selected, visible, total}, error}; visible and the counts are empty while Font View does not show",
    },
    UiCommand {
        id: "transform.getSettings",
        label: "Get Transformation Settings",
        menu: &[],
        shortcut: None,
        params: "{} the Transformations panel's settings → {origin: {mode, boxPoint, metric, point}, scale: {x, y, locked}, rotateAngle, slantAngle}",
    },
    UiCommand {
        id: "view.getDisplayOptions",
        label: "Get Display Options",
        menu: &[],
        shortcut: None,
        params: "{} every display toggle of the View menu, the showing tab, the zoom, the canvas centre, whether the Palette shows, the writing direction and the active preview features",
    },
    UiCommand {
        id: "tool.list",
        label: "List Tools",
        menu: &[],
        shortcut: None,
        params: "{} the pointer tools → {active: id, tools: [{id, label, shortcut, group}]}; tool.select makes one current",
    },
];

const GETTERS: &[(&str, Getter)] = &[
    ("editView.getText", crate::canvas::text::reply),
    ("editView.getFeatures", features),
    ("fontView.getState", font_view_state),
    ("transform.getSettings", transform_settings),
    ("view.getDisplayOptions", display_options),
    ("tool.list", tool_list),
];

/// The getter of the shell command `id`, if it is one.
pub fn find(id: &str) -> Option<Getter> {
    GETTERS.iter().find(|(g, _)| *g == id).map(|(_, getter)| *getter)
}

/// A getter fired from a key or the command palette shows nothing: its reply is for agents.
fn run(_: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    find(&command.id).map(|_| ()).ok_or_else(|| format!("no getter `{}`", command.id))
}

fn tool_list(app: &FontCraftApp) -> Value {
    app.tools.toolbox.list()
}

fn features(app: &FontCraftApp) -> Value {
    crate::canvas::text::features::reply(&app.models.features)
}

/// The View menu's display toggles, by the earlier window's names.
fn display(app: &FontCraftApp) -> Map<String, Value> {
    let d = &app.view.display;
    let mut out = Map::new();
    for (key, on) in [
        ("nodes", d.nodes),
        ("nodesInBackground", d.nodes_in_background),
        ("extraNodes", d.extra_nodes),
        ("metrics", d.metrics),
        ("metricsNames", d.metrics_names),
        ("info", d.info),
        ("background", d.background),
        ("guides", d.guides),
        ("boundingBox", d.bounding_box),
        ("annotations", d.annotations),
        ("fillPreview", d.fill_preview),
        ("anchors", d.anchors),
        ("groupMembers", app.view.text.group_members),
        ("hints", d.hints),
        ("masterCompatibility", d.master_compatibility),
        ("keepLayerSelectionInSync", d.keep_layer_selection_in_sync),
        ("matchWhenEditing", d.match_when_editing),
        ("showImage", d.image),
    ] {
        out.insert(key.into(), json!(on));
    }
    out
}

fn display_options(app: &FontCraftApp) -> Value {
    let canvas = &app.view.canvas;
    let mut out = display(app);
    out.insert("tab".into(), json!(app.view.tabs.active));
    out.insert("zoom".into(), json!(canvas.zoom));
    out.insert("center".into(), json!([canvas.center.x, canvas.center.y]));
    out.insert("palette".into(), json!(crate::shell::window::palette_shows(app)));
    out.insert("direction".into(), json!(app.view.text.direction.name()));
    out.insert("previewFeatures".into(), json!(app.models.features.active));
    Value::Object(out)
}

fn origin_mode(mode: OriginMode) -> &'static str {
    match mode {
        OriginMode::Box => "box",
        OriginMode::Reference => "reference",
        OriginMode::Metrics => "metrics",
        OriginMode::Layer => "layer",
    }
}

fn transform_json(s: &TransformSettings) -> Value {
    let [x, y] = s.scale;
    json!({
        "origin": {"mode": origin_mode(s.mode), "boxPoint": s.box_point, "metric": s.metric, "point": s.point},
        "scale": {"x": x, "y": y, "locked": s.scale_locked},
        "rotateAngle": s.rotate_angle,
        "slantAngle": s.slant_angle,
    })
}

fn transform_settings(app: &FontCraftApp) -> Value {
    transform_json(&app.view.palette.transform)
}

fn font_view_state(app: &FontCraftApp) -> Value {
    let fv = &app.view.font_view;
    let model = &app.models.font_view;
    let visible: Vec<&str> = model.visible.iter().filter_map(|&i| model.name(i)).collect();
    let ids = |columns: &[Column]| columns.iter().map(|c| c.id()).collect::<Vec<_>>();
    json!({
        "showing": app.font_view_shows(),
        "mode": fv.mode.as_str(),
        "cellSize": fv.cell_size,
        "master": model.master,
        "sort": fv.sort.as_ref().map(|s| json!({"column": s.column.id(), "ascending": s.ascending})),
        "columns": ids(&fv.shown_columns()),
        "hidden": ids(&fv.hidden),
        "sidebarEntries": fv.entries.clone().unwrap_or_else(|| json!([])),
        "search": fv.search.to_json(),
        "selected": fv.selected,
        "counts": {"selected": fv.selected.len(), "visible": visible.len(), "total": model.rows.len()},
        "visible": visible,
        "error": model.error,
    })
}

/// `ui.inspect`'s keys for the state the getters report, and what the earlier window's reply held besides:
/// `display` (the View toggles), `switches` (every on/off menu row and whether it is on), `selectedGlyphs` (Font
/// View's selection), `editing` (the edited glyph while an edit view tab shows), `interaction` (the label of the
/// interaction in progress), `dialog` (the open form dialog's id), `transform` and `fontView`.
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    let switches: Map<String, Value> = crate::app::menu_states::checks(app).into_iter().map(|(id, on)| (id.to_string(), json!(on))).collect();
    let editing = if app.font_view_shows() {
        None
    } else {
        app.session.font().ok().and_then(|f| crate::canvas::tools::edited_glyph(f, &app.view)).map(str::to_string)
    };
    state.insert("display".into(), Value::Object(display(app)));
    state.insert("switches".into(), Value::Object(switches));
    state.insert("selectedGlyphs".into(), json!(app.view.font_view.selected));
    state.insert("editing".into(), json!(editing));
    state.insert("interaction".into(), json!(app.session.interaction()));
    state.insert("dialog".into(), json!(app.dialogs.open.as_ref().map(|d| d.id.as_str())));
    state.insert("transform".into(), transform_settings(app));
    state.insert("fontView".into(), font_view_state(app));
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use fontcraft_engine::Session;
    use serde_json::json;

    use crate::testing::TestApp;

    fn get(t: &mut TestApp, id: &str) -> serde_json::Value {
        t.ok("engine.execute", json!({"command": id, "params": {}}))
    }

    /// The getters answer through `engine.execute` without changing anything, and `ui.inspect` carries the same state.
    #[test]
    fn getters_and_inspect_report_the_window_state() {
        let mut t = TestApp::with(Session::with_document());
        t.ok("engine.execute", json!({"command": "view.showBoundingBox", "params": {}}));
        let display = get(&mut t, "view.getDisplayOptions");
        assert_eq!((display["boundingBox"].clone(), display["nodes"].clone()), (json!(true), json!(true)), "{display}");
        assert_eq!(display["direction"], "ltr");

        let transform = get(&mut t, "transform.getSettings");
        assert_eq!(transform["origin"]["mode"], "box");
        assert_eq!(transform["scale"], json!({"x": 110.0, "y": 110.0, "locked": true}));

        t.ok("engine.execute", json!({"command": "editView.setText", "params": {"glyphs": ["A", "V"]}}));
        assert_eq!(get(&mut t, "editView.getText")["items"], json!(["A", "V"]));
        assert_eq!(get(&mut t, "editView.getFeatures")["compiled"], false);

        t.ok("engine.execute", json!({"command": "fontView.show", "params": {}}));
        t.ok("engine.execute", json!({"command": "fontView.setSelection", "params": {"glyphs": ["B", "A"]}}));
        let state = get(&mut t, "fontView.getState");
        assert_eq!((state["showing"].clone(), state["selected"].clone()), (json!(true), json!(["B", "A"])));
        assert_eq!(state["counts"]["visible"], state["counts"]["total"]);

        let inspect = t.ok("ui.inspect", json!({}));
        assert_eq!(inspect["selectedGlyphs"], json!(["B", "A"]));
        assert_eq!(inspect["display"]["boundingBox"], true);
        assert_eq!(inspect["switches"]["view.showBoundingBox"], true);
        assert_eq!((inspect["editing"].clone(), inspect["interaction"].clone(), inspect["dialog"].clone()), (json!(null), json!(null), json!(null)));
        assert_eq!(inspect["fontView"]["mode"], "grid");
        assert_eq!(inspect["transform"], transform);
    }
}
