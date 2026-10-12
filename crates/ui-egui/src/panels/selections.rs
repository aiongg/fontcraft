//! Palette › Selections: the edited glyph's stored selections (`palette.selections`). Typing a
//! name and pressing Return stores the Edit View selection under it (`edit.storeSelection`);
//! clicking a name selects it again on the edited layer (`edit.recallSelection`, this module's
//! shell command); Update stores the current selection under that entry's name; Remove deletes the
//! entry (`edit.removeStoredSelection`). The entries come from `edit.getStoredSelections`, mapped
//! onto the edited layer, so an entry that does not fully resolve there shows how much is missing.

use fontcraft_tools::Selection;
use serde_json::{Value, json};

use super::{Pending, edit_targets, palette, widgets, with};
use crate::app::FontcraftApp;
use crate::shell::{Args, UiCommand};
use crate::tools_glue;

/// Entries the section lists (a glyph keeps at most 256).
const MAX_ROWS: usize = 256;

pub const COMMANDS: &[UiCommand] = &[(
    "edit.recallSelection",
    "Recall Selection",
    &["Edit", "Selection"],
    "",
    "{name: string, glyph?: name, layer?: id (both must name what Edit View edits)} replace the Edit View selection with the stored selection `name` of the edited glyph, mapped onto the edited layer: the nodes with the same indexes, the components at the same indexes, the anchors with the same names. What does not resolve there is left out and counted; when nothing resolves the command is refused → {name, selection, missing: count}",
)];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != "edit.recallSelection" {
        return Err(format!("unknown command `{id}`"));
    }
    let name = args.string("name")?.ok_or_else(|| args.err("`name` is required"))?;
    tools_glue::sync(app);
    let query = {
        let font = app.session.font().map_err(|e| e.to_string())?;
        let tab = app.state.tab().ok_or_else(|| args.err("no Edit View tab is showing (open a glyph first)"))?;
        let scene = tools_glue::shown_scene(&app.session, font, tab).ok_or_else(|| args.err("the edited glyph has no layer in the active master"))?;
        if let Some(glyph) = args.string("glyph")?
            && glyph != scene.glyph
        {
            return Err(args.err(format!("`{glyph}` is not the glyph being edited (`{}`); open it first", scene.glyph)));
        }
        if let Some(layer) = args.string("layer")?
            && layer != scene.layer_id
        {
            return Err(args.err(format!("layer `{layer}` is not the layer being edited (`{}`)", scene.layer_id)));
        }
        json!({"glyph": scene.glyph, "layer": scene.layer_id, "side": scene.side.as_str(), "name": name})
    };
    let found = app.session.execute("edit.getStoredSelections", &query).map_err(|e| args.err(e))?;
    let entry =
        found.get("selections").and_then(Value::as_array).and_then(|l| l.first()).ok_or_else(|| args.err(format!("no stored selection `{name}`")))?;
    let mut lists = serde_json::Map::new();
    for key in ["points", "shapes", "anchors"] {
        if let Some(v) = entry.get(key).filter(|v| v.as_array().is_some_and(|a| !a.is_empty())) {
            lists.insert(key.into(), v.clone());
        }
    }
    if lists.is_empty() {
        return Err(args.err(format!("nothing of `{name}` resolves on the edited layer")));
    }
    let selection = Selection::from_value(&Value::Object(lists.clone())).map_err(|e| args.err(e))?;
    let missing = entry.get("missing").cloned().unwrap_or(json!(0));
    if let Some(tab) = app.state.tab_mut() {
        tab.selection = selection;
    }
    Ok(json!({"name": name, "selection": Value::Object(lists), "missing": missing}))
}

/// One listed entry: its name and how many of its items do not resolve on the edited layer.
fn entries(app: &mut FontcraftApp, targets: &serde_json::Map<String, Value>) -> Vec<(String, u64)> {
    let mut query = serde_json::Map::new();
    for key in ["glyph", "layer", "side"] {
        if let Some(v) = targets.get(key) {
            query.insert(key.into(), v.clone());
        }
    }
    let Ok(found) = app.session.execute("edit.getStoredSelections", &Value::Object(query)) else { return Vec::new() };
    let Some(list) = found.get("selections").and_then(Value::as_array) else { return Vec::new() };
    list.iter()
        .take(MAX_ROWS)
        .filter_map(|e| Some((e.get("name")?.as_str()?.to_string(), e.get("missing").and_then(Value::as_u64).unwrap_or(0))))
        .collect()
}

/// Draws the section.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let Some(targets) = edit_targets(app, "edit.storeSelection") else {
        palette::note(app, ui, "Open a glyph to store selections");
        return;
    };
    let Some(glyph) = targets.get("glyph").cloned() else { return };
    ui.horizontal(|ui| {
        ui.label("Name");
        if let Some(name) = widgets::text(ui, widgets::id("selections.add"), "", 140.0)
            && !name.trim().is_empty()
        {
            pending.push(("edit.storeSelection", with(targets.clone(), json!({ "name": name.trim() }))));
        }
    });
    let list = entries(app, &targets);
    if list.is_empty() {
        palette::note(app, ui, "No stored selections");
    }
    for (name, missing) in list {
        ui.horizontal(|ui| {
            if widgets::button(ui, widgets::id(("selections.recall", &name)), &name, "Select these nodes again", true, false) {
                pending.push(("edit.recallSelection", json!({ "name": name })));
            }
            if missing > 0 {
                ui.label(egui::RichText::new(format!("{missing} missing")).color(app.tokens.text_dim));
            }
            if widgets::flat_button(ui, widgets::id(("selections.update", &name)), "Update", "Store the current selection under this name") {
                pending.push(("edit.storeSelection", with(targets.clone(), json!({ "name": name }))));
            }
            if widgets::flat_button(ui, widgets::id(("selections.remove", &name)), "Remove", "Delete this stored selection") {
                pending.push(("edit.removeStoredSelection", json!({ "glyph": glyph, "name": name })));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use fontcraft_model::Layer;
    use serde_json::json;

    use crate::FontcraftApp;

    /// Glyph A with a square in master `m01` and a wider rectangle in `m02`, both drawn the same
    /// way (four nodes), edited in `m01`.
    fn two_masters() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
        app.session
            .edit("setup", |font| {
                let mut m = font.default_master()?.clone();
                m.id = "m02".into();
                m.name = "Bold".into();
                Arc::make_mut(&mut font.masters).push(m);
                font.glyph_mut("A")?.layers.push(Layer::new("m02", 700.0));
                Ok(())
            })
            .unwrap();
        app.run("path.addRectangle", &json!({"glyph": "A", "layer": "m02", "x": 50, "y": 50, "width": 400, "height": 300})).unwrap();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        app
    }

    fn selected(app: &FontcraftApp) -> BTreeSet<(usize, usize)> {
        app.state.tab().unwrap().selection.points.clone()
    }

    #[test]
    fn a_stored_selection_is_recalled_on_another_master() {
        let mut app = two_masters();
        app.run("selection.set", &json!({"points": [[0, 0], [0, 2]]})).unwrap();
        let targets = super::super::edit_targets(&app, "edit.storeSelection").unwrap();
        assert_eq!(targets["points"], json!([[0, 0], [0, 2]]), "the shell fills the stored lists from the selection");
        let r = app.run("edit.storeSelection", &super::super::with(targets, json!({"name": "corners"}))).unwrap();
        assert_eq!((r["replaced"].as_bool(), r["count"].as_u64()), (Some(false), Some(1)), "{r}");

        app.run("edit.deselectAll", &json!({})).unwrap();
        app.run("view.selectMaster", &json!({"master": 1})).unwrap();
        let r = app.run("edit.recallSelection", &json!({"name": "corners"})).unwrap();
        assert_eq!(r["missing"], json!(0), "{r}");
        assert_eq!(selected(&app), BTreeSet::from([(0, 0), (0, 2)]), "the same node indexes in the second master");

        // Refusals change nothing: an unknown name, another glyph than the edited one.
        app.run("edit.deselectAll", &json!({})).unwrap();
        assert!(app.run("edit.recallSelection", &json!({"name": "nope"})).is_err());
        assert!(app.run("edit.recallSelection", &json!({"name": "corners", "glyph": "B"})).is_err());
        assert!(selected(&app).is_empty());
    }
}
