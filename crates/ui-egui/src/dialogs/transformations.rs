//! Path › Transformations: the origin, slant and Cursivy settings of `path.transformations`,
//! previewed on the selected points or, with none selected, on the whole layer.
//!
//! Translate and Scale take `{x, y}` objects, which a field cannot hold yet; agents pass them to
//! the command directly.

use serde_json::json;

use super::form::{Field, Kind};
use super::{Dialog, Fields};

pub const DIALOG: Dialog =
    Dialog { id: "dialog.transformations", title: "Transformations", command: "path.transformations", fields: Fields::Fixed(fields) };

/// The command refuses ±90° and beyond, where the shear has no finite factor.
const MAX_SLANT: f64 = 89.9;

/// The origins in the order of their export-string codes (0 cap height … 4 baseline).
const ORIGINS: [&str; 5] = ["capHeight", "halfCapHeight", "xHeight", "halfXHeight", "baseline"];

fn fields(_: &fontcraft_engine::Session) -> Vec<Field> {
    vec![
        Field {
            key: "origin".into(),
            label: "Origin".into(),
            kind: Kind::Choice { choices: ORIGINS.iter().map(|o| o.to_string()).collect() },
            default: json!("baseline"),
            doc: "The fixed point of the scale and slant: a metric height of the master, at half the advance width".into(),
        },
        Field::number("slant", "Slant", -MAX_SLANT, MAX_SLANT, 0.0, "Degrees of a plain shear about the origin"),
        Field::bool("cursivy", "Cursivy", false, "Slant with optical correction of the curves; needs the master's horizontal and vertical stems"),
    ]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::Font;
    use serde_json::{Value, json};

    use crate::{FontcraftApp, control};

    fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
        let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["result"].clone()
    }

    fn fails(app: &mut FontcraftApp, method: &str, params: Value) -> String {
        let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        assert_eq!(r["ok"], false, "{method} should fail: {r}");
        r["error"].as_str().unwrap_or_default().to_string()
    }

    /// Glyph A with a 200-unit square, edited in a tab.
    fn square_app() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        app
    }

    fn font(app: &FontcraftApp) -> Arc<Font> {
        app.session.doc().unwrap().font.clone()
    }

    fn open(app: &mut FontcraftApp, params: Value) -> Value {
        call(app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": "dialog.transformations", "params": params}}))
    }

    #[test]
    fn the_dialog_previews_slants_the_layer_and_copies_the_filter_parameter() {
        let mut app = square_app();
        let original = font(&app);
        let opened = open(&mut app, json!({}));
        assert_eq!(opened["title"], "Transformations");
        assert_eq!(opened["params"]["glyph"], "A", "no points selected: the whole layer");
        assert_eq!(opened["params"]["origin"], "baseline");
        assert!(font(&app).same_as(&original), "the opening values change nothing");

        call(&mut app, "ui.dialog.set", json!({"field": "slant", "value": 10}));
        assert!(!font(&app).same_as(&original), "the canvas shows the slanted square");
        let error = fails(&mut app, "ui.dialog.set", json!({"field": "slant", "value": 90}));
        assert!(error.contains("from -89.9 to 89.9"), "{error}");
        fails(&mut app, "ui.dialog.set", json!({"field": "origin", "value": "middle"}));

        // The "…" menu's first item copies the dialog's values as an export filter.
        let shown = call(&mut app, "ui.dialog.inspect", json!({}));
        let copied = call(&mut app, "engine.execute", json!({"command": "path.transformations.copyFilterParameter", "params": shown["params"]}));
        assert_eq!(copied["text"], "Transformations;Slant:10;SlantCorrection:0;Origin:4;");

        call(&mut app, "ui.dialog.set", json!({"field": "origin", "value": "xHeight"}));
        call(&mut app, "ui.dialog.confirm", json!({}));
        let mut direct = square_app();
        direct.run("path.transformations", &json!({"glyph": "A", "slant": 10, "origin": "xHeight"})).unwrap();
        assert!(font(&app).same_as(&font(&direct)), "the dialog and the command give the same font");
    }

    #[test]
    fn selected_points_move_alone_and_cancel_puts_the_font_back() {
        let mut app = square_app();
        let original = font(&app);
        let points = json!([[0, 0], [0, 1]]);
        open(&mut app, json!({"glyph": "A", "points": points, "slant": 20}));
        call(&mut app, "ui.click", json!({"id": "ok"}));
        let mut direct = square_app();
        direct.run("path.transformations", &json!({"glyph": "A", "points": points, "slant": 20})).unwrap();
        assert!(font(&app).same_as(&font(&direct)));
        assert!(!font(&app).same_as(&original));

        let before = font(&app);
        open(&mut app, json!({"glyph": "A", "slant": -15, "origin": "capHeight"}));
        assert!(!font(&app).same_as(&before), "the preview shows the slant");
        call(&mut app, "ui.click", json!({"id": "cancel"}));
        assert!(Arc::ptr_eq(&font(&app), &before), "Cancel puts back the very font the dialog opened on");

        // Cursivy is refused with a message, and OK keeps nothing.
        open(&mut app, json!({"glyph": "A", "slant": 10}));
        let shown = call(&mut app, "ui.click", json!({"id": "cursivy"}));
        assert!(shown["error"].as_str().is_some_and(|e| e.contains("Cursivy")), "{shown}");
        fails(&mut app, "ui.dialog.confirm", json!({}));
        call(&mut app, "ui.dialog.cancel", json!({}));
        assert!(Arc::ptr_eq(&font(&app), &before));
    }
}
