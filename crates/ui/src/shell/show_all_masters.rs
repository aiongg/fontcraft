//! Edit › Show All Masters: the edited glyph once per master layer, then once per intermediate and alternate layer, in
//! the showing tab's text. Each item it inserts has its own layer (`Text::set_layer`), so the canvas draws, spaces and
//! edits each in that layer.

use craft::ui::runtime::commands::CommandRef;
use serde_json::{Value, json};

use super::commands::{Module, UiCommand};
use super::params::{quoted, string};
use crate::app::FontCraftApp;

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// Most layers one Show All Masters inserts.
const MAX_LAYERS: usize = 1_000;

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "edit.showAllMasters",
    label: "Show All Masters",
    menu: &["Edit"],
    shortcut: None,
    params: "{glyph?: name (default: the edited glyph)} insert the glyph into the Edit View tab's text after the edited glyph, once for each of its master layers and then once for each intermediate and alternate layer, in the Layers panel's order; each stands in its own layer, which is the active layer while the cursor is before it. ui.inspect lists the layers under editText.layers",
}];

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    if app.view.tabs.font_view_shows() {
        return Err("Show All Masters works in an Edit View tab: open a glyph first".into());
    }
    let font = app.session.font().map_err(|e| e.to_string())?;
    let glyph = match string(&command.params, "glyph")? {
        Some(name) => font.glyph(name).ok_or_else(|| format!("there is no glyph `{}`", quoted(name)))?,
        None => super::masters::shown_glyph(font, &app.view).ok_or("the font has no glyph")?,
    };
    let name = glyph.name.clone();
    let listed = app.session.query("layers.list", &json!({ "glyph": name })).map_err(|e| e.to_string())?;
    let layers = shown_layers(&listed);
    if layers.is_empty() {
        return Err(format!("glyph `{}` has no master layer", quoted(&name)));
    }
    if layers.len() > MAX_LAYERS {
        return Err(format!("glyph `{}` has {} layers to show; at most {MAX_LAYERS} are inserted", quoted(&name), layers.len()));
    }
    let font = app.session.font().map_err(|e| e.to_string())?;
    if !crate::canvas::text::shows_text(font, &app.view) {
        crate::canvas::text::hold_shown_glyph(app);
    }
    let font = app.session.font().map_err(|e| e.to_string())?;
    let mut next = app.view.text.clone();
    let edited = next.edited_index().ok_or("the edit view shows no glyph")?;
    let current = next.current().map(str::to_string);
    let at = edited.saturating_add(1);
    let item = crate::canvas::text::commands::glyph_item(font, &name);
    next.text.insert(at, vec![item; layers.len()]).map_err(|e| e.to_string())?;
    for (index, layer) in (at..).zip(&layers) {
        next.text.set_layer(index, Some(layer));
    }
    // The edited glyph stays the edited one.
    next.text.set_cursor(edited);
    next.text.set_current_glyph(current.as_deref());
    app.view.text = next;
    crate::canvas::text::follow(app);
    Ok(())
}

/// The ids of the layers a `layers.list` reply lists that Show All Masters inserts: the masters, then the intermediate
/// and alternate layers.
fn shown_layers(listed: &Value) -> Vec<String> {
    let rows = listed.get("layers").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let of = |kinds: &'static [&'static str]| {
        rows.iter()
            .filter(move |row| row.get("kind").and_then(Value::as_str).is_some_and(|k| kinds.contains(&k)))
            .filter_map(|row| row.get("id").and_then(Value::as_str))
            .map(str::to_string)
    };
    of(&["master"]).chain(of(&["intermediate", "alternate"])).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use fontcraft_engine::Session;
    use fontcraft_engine::model::Shape;
    use fontcraft_geom::Shape as _;
    use fontcraft_tools::{Key, KeyEvent, Modifiers};
    use serde_json::{Value, json};

    use super::*;
    use crate::app::{APP, Msg, ViewMsg};
    use crate::canvas::tools::ToolInput;

    fn try_shell(app: &mut FontCraftApp, id: &str, params: Value) -> Result<(), String> {
        app.status.clear();
        app.sent.push(Msg::Shell(CommandRef::with_params(id, params)));
        app.drain();
        if app.status.is_empty() { Ok(()) } else { Err(app.status.clone()) }
    }

    fn shell(app: &mut FontCraftApp, id: &str, params: Value) {
        try_shell(app, id, params).unwrap_or_else(|e| panic!("{id}: {e}"));
    }

    /// Glyph `n` edited; its layer of the first master is 600 wide with a stem of 100, of the second 900 with a stem of
    /// 300; the second master also has an intermediate layer `mid` and a backup.
    fn app() -> (FontCraftApp, String, String) {
        let mut s = Session::with_document();
        let mut run = |id: &str, params: Value| s.execute(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"));
        run("font.axes.add", json!({"name": "Weight", "tag": "wght"}));
        run("font.master.add", json!({}));
        run("font.master.setAxisLocation", json!({"masters": [0], "location": [30]}));
        run("font.master.setAxisLocation", json!({"masters": [1], "location": [120]}));
        let ids: Vec<String> = s.font().unwrap().masters.iter().map(|m| m.id.clone()).collect();
        for (id, stem) in ids.iter().zip([100, 300]) {
            s.execute("path.addRectangle", &json!({"glyph": "n", "layer": id, "x": 50, "y": 0, "width": stem, "height": 500})).unwrap();
        }
        let second = ids[1].clone();
        s.edit("setup", |font| {
            let mut glyph = font.glyph_mut("n")?;
            let layer = glyph.layers.iter_mut().find(|l| l.id == second).unwrap();
            layer.width = 900.0;
            let mut backup = layer.clone();
            backup.id = "bk".into();
            let mut mid = layer.clone();
            mid.id = "mid".into();
            mid.width = 700.0;
            glyph.layers.extend([backup, mid]);
            Ok(())
        })
        .unwrap();
        s.execute("layers.setIntermediate", &json!({"glyph": "n", "layer": "mid", "coordinates": {"wght": 70}})).unwrap();
        let mut app = FontCraftApp::with_edit_tab(s, Arc::new(Host::builder(APP).build()));
        app.sent.push(Msg::View(ViewMsg::SelectGlyph("n".into())));
        app.drain();
        (app, ids[0].clone(), ids[1].clone())
    }

    fn own(app: &FontCraftApp) -> Vec<Option<String>> {
        let text = &app.view.text.text;
        (0..text.len()).map(|i| text.layer(i).map(str::to_string)).collect()
    }

    fn active(app: &FontCraftApp) -> String {
        app.tools.target().unwrap().layer.clone()
    }

    #[test]
    fn each_master_layer_is_drawn_and_spaced_beside_the_edited_glyph() {
        let (mut app, first, second) = app();
        shell(&mut app, "edit.showAllMasters", json!({}));
        assert_eq!(own(&app), [None, Some(first.clone()), Some(second), Some("mid".into())], "masters, then the intermediate layer, no backup");
        assert_eq!(app.view.text.text.cursor(), 0);
        assert_eq!(active(&app), first, "the edited glyph stays edited");

        let model = &app.models.text;
        let advances: Vec<f64> = model.layout.as_ref().unwrap().placed.iter().map(|p| p.advance).collect();
        assert_eq!(advances, [600.0, 600.0, 900.0, 700.0]);
        let drawn: Vec<(usize, f64)> = model.glyphs.iter().map(|g| (g.item, g.outline.bounding_box().width())).collect();
        assert_eq!(drawn, [(1, 100.0), (2, 300.0), (3, 300.0)], "each item's outline is its layer's");

        // Another glyph's layers go after the edited glyph; a refused command changes nothing.
        let before = own(&app);
        assert!(try_shell(&mut app, "edit.showAllMasters", json!({"glyph": "nothing"})).is_err());
        assert_eq!(own(&app), before);
        shell(&mut app, "fontView.show", json!({}));
        assert!(try_shell(&mut app, "edit.showAllMasters", json!({})).is_err(), "Font View shows");
    }

    #[test]
    fn the_item_after_the_cursor_is_edited_in_its_own_layer() {
        let (mut app, first, second) = app();
        shell(&mut app, "edit.showAllMasters", json!({}));
        shell(&mut app, "editView.setCursor", json!({"index": 2}));
        assert_eq!(active(&app), second);
        assert_eq!(app.models.canvas.glyph.as_ref().unwrap().width, 900.0, "the canvas draws the second master's layer");

        app.tools.selection.points.insert((0, 0));
        app.sent.push(Msg::Tool(ToolInput::Key(KeyEvent::press(Key::Right, Modifiers::default()))));
        app.drain();
        let node_x = |layer: &str| {
            let font = app.session.font().unwrap();
            let layer = font.glyph("n").unwrap().layers.iter().find(|l| l.id == layer).unwrap();
            let Some(Shape::Path(path)) = layer.shapes.first() else { panic!("the rectangle is a path") };
            path.nodes[0].x
        };
        assert!(node_x(&second) != node_x("bk"), "the arrow key moved the node of the second master's layer");
        assert_eq!(node_x("mid"), node_x("bk"), "no other layer changed");

        // The Layers panel and the master icons change this item alone; text edits keep the others' layers.
        shell(&mut app, "layers.select", json!({"layer": "bk"}));
        assert_eq!((active(&app), own(&app)[2].clone()), ("bk".to_string(), Some("bk".to_string())));
        assert!(app.view.layers.layers.is_empty(), "the glyph's other items follow the current master");
        shell(&mut app, "view.selectMaster", json!({"master": 0}));
        assert_eq!(own(&app), [None, Some(first.clone()), Some(first.clone()), Some("mid".into())]);
        shell(&mut app, "editView.insertText", json!({"glyphs": ["o"], "at": 0}));
        assert_eq!(own(&app), [None, None, Some(first.clone()), Some(first.clone()), Some("mid".into())]);
        shell(&mut app, "editView.setCursor", json!({"index": 4}));
        assert_eq!(active(&app), "mid");

        // Another tab, and back: the items keep their layers.
        shell(&mut app, "view.newTab", json!({"glyphs": ["n"]}));
        assert_eq!(active(&app), first, "the new tab's item has no layer of its own");
        shell(&mut app, "view.showPreviousTab", json!({}));
        assert_eq!((active(&app), own(&app)[4].clone()), ("mid".to_string(), Some("mid".to_string())));

        // A layer that is gone gives way to the glyph's shown layer.
        app.session.execute("layers.delete", &json!({"glyphs": ["n"], "layers": ["mid"]})).unwrap();
        app.drain();
        assert_eq!(active(&app), first);
    }
}
