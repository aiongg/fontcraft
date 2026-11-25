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
