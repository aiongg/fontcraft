//! Primitive Measurements: the size of the rectangle or ellipse a click with the Primitives tool
//! places. The tool's click names `dialog.primitiveMeasurements` with the click as `x`, `y` and
//! the mode as `shape`; [`for_click`] picks the dialog of that shape's command.
//!
//! One spec row, two dialogs: the rectangle's (the one `dialog.open {dialog}` finds) and the
//! ellipse's (`dialog.open {command: "path.addEllipse"}`). The click is the shape's bottom-left
//! corner, as in `path.addRectangle` and `path.addEllipse` without `fromCenter`.

use fontcraft_model::MAX_COORDINATE;
use serde_json::{Map, Value};

use super::form::Field;
use super::{Dialog, Fields};

/// The size the fields open with, in font units.
const DEFAULT_SIZE: f64 = 100.0;

pub const RECTANGLE: Dialog =
    Dialog { id: "dialog.primitiveMeasurements", title: "Add Rectangle", command: "path.addRectangle", fields: Fields::Fixed(fields) };

pub const ELLIPSE: Dialog =
    Dialog { id: "dialog.primitiveMeasurements", title: "Add Ellipse", command: "path.addEllipse", fields: Fields::Fixed(fields) };

fn fields(_: &fontcraft_engine::Session) -> Vec<Field> {
    let max = 2.0 * MAX_COORDINATE;
    vec![
        Field::number("width", "Width", 1.0, max, DEFAULT_SIZE, "The shape's width in font units, to the right of the click"),
        Field::number("height", "Height", 1.0, max, DEFAULT_SIZE, "The shape's height in font units, above the click"),
    ]
}

/// The dialog a click with the Primitives tool opens, with the parameters to open it with: the
/// click's `{glyph, layer?, side?, x, y, shape: "rectangle" | "ellipse"}` without `shape`.
pub fn for_click(params: &Value) -> Option<(Dialog, Map<String, Value>)> {
    let mut m = params.as_object()?.clone();
    let dialog = match m.remove("shape").as_ref().and_then(Value::as_str) {
        Some("ellipse") => ELLIPSE,
        _ => RECTANGLE,
    };
    Some((dialog, m))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};

    use super::for_click;
    use crate::{FontcraftApp, control, dialogs};

    fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
        let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["result"].clone()
    }

    /// Glyph A with a square, edited in a tab.
    fn app() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        app
    }

    fn font(app: &FontcraftApp) -> Arc<fontcraft_model::Font> {
        app.session.doc().unwrap().font.clone()
    }

    #[test]
    fn the_typed_size_places_the_shape_at_the_click() {
        for (shape, command) in [("rectangle", "path.addRectangle"), ("ellipse", "path.addEllipse")] {
            let click = json!({"glyph": "A", "x": 400, "y": -20, "shape": shape});
            let (dialog, params) = for_click(&click).unwrap();
            assert_eq!((dialog.id, dialog.command), ("dialog.primitiveMeasurements", command));
            assert!(!params.contains_key("shape"));

            let mut app = app();
            let opened = call(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"command": command, "params": params}}));
            assert_eq!(opened["dialog"], "dialog.primitiveMeasurements");
            assert_eq!((opened["fields"][0]["id"].clone(), opened["fields"][1]["id"].clone()), (json!("width"), json!("height")));
            call(&mut app, "ui.dialog.set", json!({"field": "width", "value": 50}));
            call(&mut app, "ui.dialog.set", json!({"field": "height", "value": 300}));
            assert!(call(&mut app, "ui.dialog.inspect", json!({}))["error"].is_null());
            call(&mut app, "ui.dialog.confirm", json!({}));

            let mut direct = self::app();
            direct.run(command, &json!({"glyph": "A", "x": 400, "y": -20, "width": 50, "height": 300})).unwrap();
            assert!(font(&app).same_as(&font(&direct)), "{shape}: the dialog and the command give the same font");
        }
        assert_eq!(dialogs::find("dialog.primitiveMeasurements").map(|d| d.command), Some("path.addRectangle"));

        let mut app = app();
        let original = font(&app);
        call(
            &mut app,
            "engine.execute",
            json!({"command": "dialog.open", "params": {"dialog": "dialog.primitiveMeasurements", "params": {"glyph": "A", "x": 0, "y": 0}}}),
        );
        assert!(!font(&app).same_as(&original), "the preview shows the rectangle");
        let r = control::handle(&mut app, &json!({"id": 1, "method": "ui.dialog.set", "params": {"field": "width", "value": 0}}));
        assert_eq!(r["ok"], false, "a size of 0 is refused: {r}");
        call(&mut app, "ui.dialog.cancel", json!({}));
        assert!(Arc::ptr_eq(&font(&app), &original), "Cancel puts back the very font the dialog opened on");
    }
}
