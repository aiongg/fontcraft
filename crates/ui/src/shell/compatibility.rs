//! Show Master Compatibility: the View menu toggle that draws every source layer of the edited glyph beside it
//! (`canvas::compatibility`), and Edit › Keep Layer Selection in Sync, which carries the canvas selection to another
//! master of a compatible glyph ([`carried`]).

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::model::{Layer, Shape};
use fontcraft_engine::prelude::*;
use fontcraft_tools::Selection;
use serde_json::Value;

use super::commands::{Module, UiCommand};
use crate::app::FontCraftApp;
use crate::canvas::tools::Target;

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "view.showMasterCompatibility",
        label: "Show Master Compatibility",
        menu: &["View"],
        shortcut: Some("Cmd+Ctrl+Alt+N"),
        params: "{on?: bool} draw every master, intermediate and alternate layer of the edited glyph beside it: shapes tinted by their index, lines joining start nodes, component centres, anchors and selected points across the layers, segments green (compatible), yellow (turning more than 20°) or red (incompatible) (omitted: toggle)",
    },
    UiCommand {
        id: "edit.keepLayerSelectionInSync",
        label: "Keep Layer Selection in Sync",
        menu: &["Edit"],
        shortcut: None,
        params: "{on?: bool} while on, editing another master of a compatible glyph keeps the selected nodes, shapes and anchors, matched by index (omitted: toggle)",
    },
];

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let display = &mut app.view.display;
    let flag = match command.id.as_str() {
        "view.showMasterCompatibility" => &mut display.master_compatibility,
        "edit.keepLayerSelectionInSync" => &mut display.keep_layer_selection_in_sync,
        other => return Err(format!("unknown command `{other}`")),
    };
    let on = match command.params.get("on") {
        None | Some(Value::Null) => !*flag,
        Some(Value::Bool(on)) => *on,
        Some(other) => return Err(format!("{}: `on` is true or false, not {other}", command.id)),
    };
    *flag = on;
    Ok(())
}

/// The selection the tools keep when the canvas moves from editing `old` to editing `new`, while Keep Layer Selection
/// in Sync is on: the same glyph and side, both master layers, and a glyph whose layers interpolate. Items the new
/// layer does not have are dropped; guides, annotations, hints and intersections are not carried. `None`: the
/// selection starts empty.
pub fn carried(app: &mut FontCraftApp, old: Option<&Target>, new: Option<&Target>) -> Option<Selection> {
    let (old, new) = (old?, new?);
    if !app.view.display.keep_layer_selection_in_sync
        || old.glyph != new.glyph
        || old.side != new.side
        || old.layer == new.layer
        || app.tools.selection.is_empty()
    {
        return None;
    }
    let font = app.session.font().ok()?;
    let glyph = font.glyph(&new.glyph)?;
    let master_layer = |id: &str| glyph.layers.iter().find(|l| l.id == id && l.is_master_layer());
    master_layer(&old.layer)?;
    let layer = master_layer(&new.layer)?.clone();
    if !app.session.glyph_compat(&new.glyph).is_some_and(|c| c.ok) {
        return None;
    }
    Some(carried_selection(&app.tools.selection, &layer))
}

/// The part of `selection` that resolves on `layer`: nodes and shapes by index, anchors by name.
pub fn carried_selection(selection: &Selection, layer: &Layer) -> Selection {
    Selection {
        points: selection.points.iter().copied().filter(|&(s, n)| matches!(layer.shapes.get(s), Some(Shape::Path(p)) if n < p.nodes.len())).collect(),
        shapes: selection.shapes.iter().copied().filter(|&s| s < layer.shapes.len()).collect(),
        anchors: selection.anchors.iter().filter(|a| layer.anchors.iter().any(|x| x.name == **a)).cloned().collect(),
        ..Selection::default()
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

    use crate::app::{APP, FontCraftApp, Msg, ViewMsg};

    fn shell(app: &mut FontCraftApp, id: &str, params: Value) {
        app.sent.push(Msg::Shell(CommandRef::with_params(id, params)));
        app.drain();
        assert!(app.status.is_empty(), "{id}: {}", app.status);
    }

    /// Masters Light (wght 30) and Bold (wght 120); glyph `n` with a rectangle on each, and `o` with a rectangle on
    /// Light only, so its layers do not interpolate.
    fn two_masters() -> FontCraftApp {
        let mut s = Session::with_document();
        let mut run = |id: &str, params: Value| {
            s.execute(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"));
        };
        run("font.axes.add", json!({"name": "Weight", "tag": "wght"}));
        run("font.master.add", json!({}));
        run("font.master.setAxisLocation", json!({"masters": [0], "location": [30]}));
        run("font.master.setAxisLocation", json!({"masters": [1], "location": [120]}));
        let ids: Vec<String> = s.font().unwrap().masters.iter().map(|m| m.id.clone()).collect();
        for (id, width) in ids.iter().zip([100, 300]) {
            s.execute("path.addRectangle", &json!({"glyph": "n", "layer": id, "x": 50, "y": 0, "width": width, "height": 500})).unwrap();
        }
        s.execute("path.addRectangle", &json!({"glyph": "o", "layer": ids[0], "x": 50, "y": 0, "width": 100, "height": 500})).unwrap();
        FontCraftApp::new(s, Arc::new(Host::builder(APP).build()))
    }

    /// Edits `glyph` on the first master, selects node 2 of path 0, and shows the second master: the selection then.
    fn select_then_switch(app: &mut FontCraftApp, glyph: &str) -> Vec<(usize, usize)> {
        app.sent.push(Msg::View(ViewMsg::SelectGlyph(glyph.into())));
        shell(app, "view.selectMaster", json!({"master": 0}));
        app.tools.selection = fontcraft_tools::Selection::new();
        app.tools.selection.points.insert((0, 2));
        shell(app, "view.selectMaster", json!({"master": 1}));
        app.tools.selection.points.iter().copied().collect()
    }

    #[test]
    fn keep_layer_selection_in_sync_carries_the_selection_to_another_master_of_a_compatible_glyph() {
        let mut app = two_masters();
        assert!(select_then_switch(&mut app, "n").is_empty(), "off: another master starts with no selection");
        shell(&mut app, "edit.keepLayerSelectionInSync", json!({}));
        assert!(app.view.display.keep_layer_selection_in_sync, "omitted: toggle");
        assert_eq!(select_then_switch(&mut app, "n"), [(0, 2)], "on: node 2 of path 0 stays selected on Bold");
        assert!(select_then_switch(&mut app, "o").is_empty(), "o cannot interpolate: nothing is carried");
        let checks = crate::app::menu_states::checks(&app);
        assert!(checks.contains(&("edit.keepLayerSelectionInSync", true)), "the Edit menu row is checked");
    }
}
