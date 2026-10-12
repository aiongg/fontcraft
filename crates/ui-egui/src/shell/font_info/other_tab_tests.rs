//! The Other Settings tab in headless frames: each field and checkbox queues `font.settings.set`
//! with its key and gives the font that command gives.

use serde_json::json;

use super::super::font_tab::tests::{Tab, app, font, same_as_command};
use super::{control_id, set_command};

#[test]
fn other_settings_fields_give_the_font_their_commands_give() {
    let mut app = app();
    let tab = Tab::new(super::show);
    for (key, typed, value) in [
        ("gridSpacing", "5", json!(5)),
        ("gridSubdivision", "10", json!(10)),
        ("keyboardIncrementShift", "20", json!(20)),
        ("keyboardIncrementCmd", "200", json!(200)),
    ] {
        let pending = tab.enter(&mut app, control_id(key), typed);
        let (id, params) = set_command(key, value.clone());
        same_as_command(&mut app, pending, id, params);
        let shown = app.run("font.settings.get", &json!({"keys": [key]})).unwrap();
        assert_eq!(shown.get(key).and_then(|v| v.as_f64()), value.as_f64(), "{key}");
    }
    let f = font(&app);
    assert_eq!(f.info.settings.grid_length, Some(5.0));
}

#[test]
fn other_settings_checkboxes_toggle_through_the_command() {
    let mut app = app();
    let tab = Tab::new(super::show);
    for key in ["useCustomNaming", "disableAutomaticAlignment", "keepAlternatesNextToBase"] {
        let pending = tab.click(&mut app, control_id(key));
        let (id, params) = set_command(key, json!(true));
        same_as_command(&mut app, pending, id, params);
        let pending = tab.click(&mut app, control_id(key));
        let (id, params) = set_command(key, json!(false));
        same_as_command(&mut app, pending, id, params);
    }
    let shown = app.run("font.settings.get", &json!({"keys": ["useCustomNaming"]})).unwrap();
    assert_eq!(shown, json!({"useCustomNaming": false}));
}

#[test]
fn an_out_of_range_grid_spacing_is_refused_and_changes_nothing() {
    let mut app = app();
    let tab = Tab::new(super::show);
    let before = font(&app);
    let pending = tab.enter(&mut app, control_id("gridSpacing"), "-3");
    assert_eq!(pending, vec![set_command("gridSpacing", json!(-3))]);
    for (id, p) in pending {
        assert!(app.run(id, &p).is_err(), "a negative grid is refused");
    }
    assert_eq!(font(&app), before);
}
