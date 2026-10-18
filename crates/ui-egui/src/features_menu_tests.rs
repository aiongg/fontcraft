//! Edit View shaping and the Features menu: a tab with smcp on shows `a.sc` and edits it,
//! kerning edits show without compiling again, feature code edits show after Compile, and the
//! menu's commands change only the tab.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::FontcraftApp;
use crate::canvas::view;

fn run(app: &mut FontcraftApp, id: &str, params: Value) -> Value {
    app.run(id, &params).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn shown(app: &FontcraftApp) -> Vec<Option<String>> {
    let tab = app.state.tab().unwrap();
    let layout = view::text_layout(&app.session, &app.layouts, tab).unwrap();
    layout.placed.iter().map(|p| p.glyph.as_deref().map(str::to_string)).collect()
}

/// A font with `a.sc`, `f_i` and smcp and liga features, compiled, and a tab showing `a f i b`
/// (glyphs typed by name).
fn compiled_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    for name in ["a.sc", "f_i"] {
        run(&mut app, "glyph.newGlyph", json!({"name": name, "width": 450}));
    }
    run(&mut app, "font.features.addFeature", json!({"tag": "smcp", "code": "sub a by a.sc;"}));
    run(&mut app, "font.features.addFeature", json!({"tag": "liga", "code": "sub f i by f_i;"}));
    let compiled = run(&mut app, "font.features.compile", json!({}));
    assert_eq!(compiled["preview"], json!(true), "{compiled}");
    run(&mut app, "view.newTab", json!({"glyphs": ["a", "f", "i", "b"]}));
    app
}

#[test]
fn a_tab_with_smcp_on_shows_and_edits_the_small_cap() {
    let mut app = compiled_app();
    let some = |n: &str| Some(n.to_string());
    assert_eq!(shown(&app), [some("a"), some("f"), some("i"), some("b")], "no feature is on by default");

    let reply = run(&mut app, "editView.toggleFeature", json!({"tag": "smcp"}));
    assert_eq!(reply["active"], json!(["smcp"]), "{reply}");
    assert_eq!(shown(&app), [some("a.sc"), some("f"), some("i"), some("b")]);

    // The ligature shows on its first component; the second takes no room.
    run(&mut app, "editView.toggleFeature", json!({"tag": "liga", "on": true}));
    assert_eq!(shown(&app), [some("a.sc"), some("f_i"), None, some("b")]);
    let tab = app.state.tab().unwrap().clone();
    let layout = view::text_layout(&app.session, &app.layouts, &tab).unwrap();
    assert_eq!(layout.placed[2].advance, 0.0);
    assert_eq!(layout.placed[3].x, layout.placed[1].x + 450.0, "b follows f_i's live width");

    // Double-clicking the small cap edits it (the gesture runs this command).
    run(&mut app, "editView.setCursor", json!({"index": 0, "glyph": "a.sc"}));
    let tab = app.state.tab().unwrap();
    assert_eq!(tab.edited_glyph(), Some("a.sc"));
    assert_eq!(tab.edited_index(), Some(0));

    // The dash turns everything off.
    let reply = run(&mut app, "editView.clearFeatures", json!({}));
    assert_eq!(reply["active"], json!([]), "{reply}");
    assert_eq!(shown(&app)[0], some("a"));
}

#[test]
fn kerning_edits_show_at_once_and_feature_edits_after_compile() {
    let mut app = compiled_app();
    run(&mut app, "editView.toggleFeature", json!({"tag": "smcp"}));
    run(&mut app, "kerning.setPair", json!({"left": "a.sc", "right": "f", "value": -50}));
    let tab = app.state.tab().unwrap().clone();
    let layout = view::text_layout(&app.session, &app.layouts, &tab).unwrap();
    assert_eq!(layout.placed[1].kern, -50.0, "kerning applies to the glyphs after substitution, without a compile");

    run(&mut app, "font.features.setCode", json!({"kind": "feature", "name": "smcp", "code": "sub b by a.sc;"}));
    assert_eq!(shown(&app)[0].as_deref(), Some("a.sc"), "the code changes nothing until it is compiled");
    run(&mut app, "font.features.compile", json!({}));
    let after = shown(&app);
    assert_eq!((after[0].as_deref(), after[3].as_deref()), (Some("a"), Some("a.sc")));
}

#[test]
fn the_menu_lists_compiled_features_and_its_commands_change_only_the_tab() {
    let mut app = FontcraftApp::default();
    run(&mut app, "view.newTab", json!({"glyphs": ["a"]}));
    let before = Arc::clone(&app.session.doc().unwrap().font);
    let reply = run(&mut app, "editView.getFeatures", json!({}));
    assert_eq!((reply["compiled"].clone(), reply["features"].clone()), (json!(false), json!([])), "{reply}");

    run(&mut app, "font.features.addPrefix", json!({"code": "languagesystem DFLT dflt;\nlanguagesystem latn dflt;\nlanguagesystem latn TRK;"}));
    run(&mut app, "font.features.addFeature", json!({"tag": "locl", "code": "script latn; language TRK exclude_dflt; sub i by a;"}));
    let reply = run(&mut app, "editView.getFeatures", json!({}));
    assert_eq!(reply["features"], json!([]), "a feature appears after the next compile");
    run(&mut app, "font.features.compile", json!({}));
    let reply = run(&mut app, "editView.getFeatures", json!({}));
    assert_eq!(reply["features"], json!([{"tag": "locl", "table": "GSUB", "active": false}]), "{reply}");
    assert_eq!(reply["languageSystems"], json!([["latn", "TRK"]]), "{reply}");

    let before_on = Arc::clone(&app.session.doc().unwrap().font);
    let reply = run(&mut app, "editView.setLocalizedForms", json!({}));
    assert_eq!((reply["localizedForms"].clone(), reply["script"].clone(), reply["language"].clone()), (json!(true), json!("latn"), json!("TRK")));
    run(&mut app, "editView.setText", json!({"text": "i"}));
    assert_eq!(shown(&app)[0].as_deref(), Some("a"), "Localized Forms applies the Turkish rule");
    assert!(Arc::ptr_eq(&before_on, &app.session.doc().unwrap().font), "the menu changes no font data");
    assert!(!Arc::ptr_eq(&before, &before_on));

    // A refused command changes nothing.
    let tab = app.state.tab().unwrap().features.clone();
    assert!(app.run("editView.toggleFeature", &json!({"tag": "toolong"})).is_err());
    assert!(app.run("editView.setLocalizedForms", &json!({"script": ""})).is_err());
    assert_eq!(app.state.tab().unwrap().features, tab);
}

/// A glyph without a code point, typed by name, reaches the shaper through its private-use code
/// point, so a feature substitutes it like a typed character.
#[test]
fn a_glyph_typed_by_name_without_a_code_point_is_substituted() {
    let mut app = compiled_app();
    run(&mut app, "font.features.addFeature", json!({"tag": "ss01", "code": "sub a.sc by f_i;"}));
    run(&mut app, "font.features.compile", json!({}));
    run(&mut app, "view.newTab", json!({"glyphs": ["a.sc", "b"]}));
    run(&mut app, "editView.toggleFeature", json!({"tag": "ss01"}));
    assert_eq!(shown(&app), [Some("f_i".to_string()), Some("b".to_string())]);
}
