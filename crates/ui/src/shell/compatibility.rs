//! Show Master Compatibility: the View menu toggle that draws every source layer of the edited glyph beside it
//! (`canvas::compatibility`), Edit › Keep Layer Selection in Sync, which carries the canvas selection to another
//! master of a compatible glyph ([`carried`]), and Edit › Match When Editing, which makes the window's structural edits
//! on every compatible layer ([`with_matching`]).

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::interp::{Designspace, GlyphSources, Source};
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
        params: "{on?: bool} while on, editing another master or intermediate layer of a compatible glyph, or another layer of the same alternate set, keeps the selected nodes, shapes and anchors, matched by index (omitted: toggle)",
    },
    UiCommand {
        id: "edit.toggleMatchWhenEditing",
        label: "Match When Editing",
        menu: &["Edit"],
        shortcut: None,
        params: "{on?: bool} while on, inserting and deleting nodes, setting a start node and reversing contours in the window also edit every compatible layer of the glyph, in the same undo step (the `matchLayers` parameter of those commands); layers that differ are left alone and named in the status line (omitted: toggle)",
    },
];

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let display = &mut app.view.display;
    let flag = match command.id.as_str() {
        "view.showMasterCompatibility" => &mut display.master_compatibility,
        "edit.keepLayerSelectionInSync" => &mut display.keep_layer_selection_in_sync,
        "edit.toggleMatchWhenEditing" => &mut display.match_when_editing,
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

/// The commands that take `matchLayers`.
pub const MATCHED: &[&str] = &["path.insertNode", "edit.delete", "path.makeNodeFirst", "path.reverseContours"];

/// The parameters of a command the person ran: while Match When Editing is on, the structural edits get `matchLayers`
/// unless they say.
pub fn with_matching(app: &FontCraftApp, id: &str, mut params: Value) -> Value {
    if app.view.display.match_when_editing
        && MATCHED.contains(&id)
        && let Some(map) = params.as_object_mut()
    {
        map.entry("matchLayers").or_insert(Value::Bool(true));
    }
    params
}

/// The selection the tools keep when the canvas moves from editing `old` to editing `new`, while Keep Layer Selection
/// in Sync is on: the same glyph and side, two layers that interpolate with each other (both among the master and
/// intermediate layers, or both of one alternate set), and a glyph whose layers interpolate. Items the new layer does
/// not have are dropped; guides, annotations, hints and intersections are not carried. `None`: the selection starts
/// empty.
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
    let layer = glyph.layers.iter().find(|l| l.id == new.layer)?.clone();
    let designspace = Designspace::new(font).ok()?;
    let sources = GlyphSources::of(font, &designspace, &new.glyph).ok()?;
    let together = |group: &[Source]| [&old.layer, &new.layer].iter().all(|id| group.iter().any(|s| s.layer_id == **id));
    if !together(&sources.main) && !sources.alternates.iter().any(|set| together(&set.sources)) {
        return None;
    }
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
    use fontcraft_engine::model::Shape;
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
        FontCraftApp::with_edit_tab(s, Arc::new(Host::builder(APP).build()))
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

    /// The selection also moves to an intermediate layer and between the layers of one alternate set, not from a master
    /// layer to an alternate one.
    #[test]
    fn the_selection_moves_among_the_layers_that_interpolate_with_each_other() {
        let mut app = two_masters();
        let ids: Vec<String> = app.session.font().unwrap().masters.iter().map(|m| m.id.clone()).collect();
        let mut run = |id: &str, params: Value| app.session.execute(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"));
        let brace = run("layers.addIntermediate", json!({"glyph": "n", "location": {"wght": 70}}))["layer"].as_str().unwrap().to_string();
        let set = run("layers.setAlternate", json!({"glyphs": ["n"], "allMasters": true, "ranges": {"wght": {"min": 100}}}));
        let brackets: Vec<String> = set["layers"].as_array().unwrap().iter().map(|r| r["layer"].as_str().unwrap().to_string()).collect();
        assert_eq!(brackets.len(), 2);
        app.sent.push(Msg::View(ViewMsg::SelectGlyph("n".into())));
        shell(&mut app, "edit.keepLayerSelectionInSync", json!({"on": true}));
        let mut moved = |from: &str, to: &str| -> bool {
            shell(&mut app, "layers.select", json!({"layer": from}));
            app.tools.selection = fontcraft_tools::Selection::new();
            app.tools.selection.points.insert((0, 2));
            shell(&mut app, "layers.select", json!({"layer": to}));
            app.tools.selection.points.contains(&(0, 2))
        };
        assert!(moved(&ids[0], &brace) && moved(&brace, &ids[1]), "master layers and intermediate layers");
        assert!(moved(&brackets[0], &brackets[1]), "two layers of one alternate set");
        assert!(!moved(&ids[0], &brackets[1]), "a master layer and an alternate layer");
    }

    /// While Match When Editing is on, an edit in the window changes every compatible master in one undo step; off, and
    /// for a request of the control channel, only the edited one.
    #[test]
    fn match_when_editing_edits_every_compatible_master() {
        let mut app = two_masters();
        let nodes = |app: &FontCraftApp| -> Vec<usize> {
            let g = app.session.font().unwrap().glyph("n").unwrap();
            g.layers.iter().filter_map(|l| if let Some(Shape::Path(p)) = l.shapes.first() { Some(p.nodes.len()) } else { None }).collect()
        };
        let before = nodes(&app);
        assert_eq!(before.len(), 2);
        let insert = json!({"glyph": "n", "segment": [0, 1]});
        app.run_now(Msg::run("path.insertNode", insert.clone()), false).unwrap();
        assert_eq!(nodes(&app), [before[0] + 1, before[1]], "off: the edited master only");
        app.run_now(Msg::run("edit.undo", json!({})), false).unwrap();

        shell(&mut app, "edit.toggleMatchWhenEditing", json!({}));
        app.run_request(Msg::run("path.insertNode", insert.clone())).unwrap();
        assert_eq!(nodes(&app), [before[0] + 1, before[1]], "a request says itself what it edits");
        app.run_now(Msg::run("edit.undo", json!({})), false).unwrap();
        app.run_now(Msg::run("path.insertNode", insert), false).unwrap();
        assert_eq!(nodes(&app), [before[0] + 1, before[1] + 1], "on: both masters");
        app.run_now(Msg::run("edit.undo", json!({})), false).unwrap();
        assert_eq!(nodes(&app), before, "one undo step");
    }
}
