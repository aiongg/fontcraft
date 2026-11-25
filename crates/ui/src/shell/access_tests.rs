//! What assistive technology finds in the window.

use craft::ui::runtime::commands::CommandRef;
use craft::ui::testing::access::{is_interactive, name};
use serde_json::json;

use crate::testing::TestApp;

/// The interactive nodes of the window without a name, as role and place.
fn unnamed(t: &mut TestApp) -> Vec<String> {
    let tree = t.frames.tree();
    let nodes = tree.keys().into_iter().filter(|key| !tree.is_hidden(*key) && is_interactive(&tree, *key) && name(&tree, *key).is_none());
    nodes.map(|key| format!("{:?} at {:?}", tree.role_of(key), tree.bounds_of(key))).collect()
}

/// Every control of the window has a name: in Edit View, in Font View, and with the panels open that draw their own.
#[test]
fn every_control_of_the_window_has_a_name() {
    let mut t = TestApp::new();
    t.frames(2);
    assert_eq!(unnamed(&mut t), Vec::<String>::new(), "Edit View");
    t.app.fire(CommandRef::with_params("view.showTab", json!({"index": 1})));
    t.frames(3);
    assert_eq!(unnamed(&mut t), Vec::<String>::new(), "Font View");
    for panel in ["textPreview", "bitmapEditor", "spaceCenter"] {
        t.app.fire(CommandRef::with_params("window.togglePanel", json!({ "panel": panel })));
        t.frames(3);
        assert_eq!(unnamed(&mut t), Vec::<String>::new(), "{panel}");
    }
}

/// The children of the nodes called `parent`, each as its name, value and whether it has a place.
fn children(t: &mut TestApp, parent: &str) -> Vec<(String, Option<String>, bool)> {
    let tree = t.frames.tree();
    let owned = |key| tree.parent_of_key(key).is_some_and(|owner| name(&tree, owner) == Some(parent));
    let keys = tree.keys().into_iter().filter(|key| owned(*key));
    keys.map(|key| {
        let placed = tree.bounds_of(key).is_some_and(|r| r.is_positive());
        (name(&tree, key).unwrap_or_default().to_string(), tree.value_of(key).map(str::to_string), placed)
    })
    .collect()
}

/// What the canvas, the Text Preview and the Space Center draw can be found by name (M14.9b): the edited glyph's nodes,
/// components and anchors while they show, and the glyphs of the two previews.
#[test]
fn drawn_glyphs_and_nodes_are_named_children_of_their_views() {
    let mut t = TestApp::new();
    let execute = |t: &mut TestApp, command: &str, params: serde_json::Value| {
        t.ok("engine.execute", json!({"command": command, "params": params}));
    };
    for glyph in ["A", "V"] {
        execute(&mut t, "path.addRectangle", json!({"glyph": glyph, "x": 100, "y": 0, "width": 400, "height": 700}));
    }
    execute(&mut t, "anchor.add", json!({"glyph": "A", "name": "top", "x": 300, "y": 700}));
    execute(&mut t, "glyph.addComponent", json!({"glyph": "A", "component": "V"}));
    t.app.sent.push(crate::app::Msg::View(crate::app::ViewMsg::SelectGlyph("A".into())));
    t.frames(4);
    let canvas = canvas_name();
    let shown = children(&mut t, &canvas);
    let has = |name: &str, value: Option<&str>| shown.iter().any(|(n, v, placed)| n == name && v.as_deref() == value && *placed);
    let corner = shown.iter().find(|(_, value, _)| value.as_deref() == Some("100, 0"));
    assert!(corner.is_some_and(|(n, _, placed)| n.starts_with("Path 1, node ") && *placed), "{shown:?}");
    assert!(has("Anchor top", Some("300, 700")), "{shown:?}");
    assert!(has("Component V", None), "{shown:?}");
    execute(&mut t, "view.showNodes", json!({"on": false}));
    t.frames(3);
    let hidden = children(&mut t, &canvas);
    assert!(!hidden.iter().any(|(n, ..)| n.starts_with("Path 1")), "hidden nodes are not listed: {hidden:?}");

    t.app.fire(CommandRef::with_params("window.togglePanel", json!({ "panel": "textPreview" })));
    t.frames(3);
    execute(&mut t, "textPreview.setText", json!({"text": "AV"}));
    t.frames(6);
    let page = children(&mut t, crate::panels::text_preview::TextPreviewWords::default().page.as_str());
    assert!(page.iter().any(|(n, _, placed)| n == "A" && *placed) && page.iter().any(|(n, ..)| n == "V"), "{page:?}");

    execute(&mut t, "window.spaceCenter", json!({"text": "AV"}));
    t.frames(6);
    let spaced = children(&mut t, &space_center_name());
    assert!(spaced.iter().any(|(n, _, placed)| n == "A" && *placed) && spaced.iter().any(|(n, ..)| n == "V"), "{spaced:?}");
}

fn canvas_name() -> String {
    super::ChromeWords::default().document.as_str().to_string()
}

fn space_center_name() -> String {
    crate::panels::space_center::SpaceCenterWords::default().preview.as_str().to_string()
}
