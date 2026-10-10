//! Path › Interpolate with Background: the blend percentage of `path.interpolateWithBackground`.

use super::form::Field;
use super::{Dialog, Fields};

/// The engine's cap on the amount, in percent.
const MAX_AMOUNT: f64 = 100_000.0;

pub const DIALOG: Dialog =
    Dialog::new("dialog.interpolateWithBackground", "Interpolate with Background", "path.interpolateWithBackground", Fields::Fixed(fields));

fn fields(_: &fontcraft_engine::Session) -> Vec<Field> {
    vec![Field::number(
        "amount",
        "Percentage",
        -MAX_AMOUNT,
        MAX_AMOUNT,
        50.0,
        "0 keeps the foreground, 100 gives the background; below 0 or above 100 extrapolates",
    )]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};

    use crate::{FontcraftApp, control};

    fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
        let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["result"].clone()
    }

    /// Glyph A, edited in a tab: a 200-unit square in the background and the same square moved
    /// by (100, 40) in the foreground.
    fn app() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
        app.run("path.selectionToBackground", &json!({"glyphs": ["A"]})).unwrap();
        app.run("selection.move", &json!({"glyph": "A", "shapes": [0], "dx": 100, "dy": 40})).unwrap();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        app
    }

    fn font(app: &FontcraftApp) -> Arc<fontcraft_model::Font> {
        app.session.doc().unwrap().font.clone()
    }

    #[test]
    fn the_dialog_gives_the_commands_font_and_cancel_puts_it_back() {
        let mut app = app();
        let original = font(&app);
        let opened = call(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": "dialog.interpolateWithBackground"}}));
        assert_eq!(opened["fields"][0]["id"], "amount");
        assert_eq!(opened["fields"][0]["value"], 50.0);
        assert_eq!(opened["params"]["glyphs"], json!(["A"]), "the glyph in the tab");
        // Below 0 extrapolates away from the background.
        call(&mut app, "ui.dialog.set", json!({"field": "amount", "value": -50}));
        assert!(!font(&app).same_as(&original), "the preview moves the square");
        call(&mut app, "ui.dialog.confirm", json!({}));

        let mut direct = self::app();
        direct.run("path.interpolateWithBackground", &json!({"glyphs": ["A"], "amount": -50})).unwrap();
        assert!(font(&app).same_as(&font(&direct)), "the dialog and the command give the same font");

        let mut app = self::app();
        let original = font(&app);
        call(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": "dialog.interpolateWithBackground"}}));
        call(&mut app, "ui.dialog.set", json!({"field": "amount", "value": 100}));
        call(&mut app, "ui.dialog.cancel", json!({}));
        assert!(Arc::ptr_eq(&font(&app), &original), "Cancel puts back the very font the dialog opened on");
    }
}
