//! Primitive Measurements (row `dialog.primitiveMeasurements`): the size of the rectangle or ellipse a click with the
//! Primitives tool places. The tool's click names `dialog.primitiveMeasurements` with the click as `x`, `y` and the
//! mode as `shape`; `canvas::tools::apply` hands it to [`open_for_click`], which opens the dialog of that shape's
//! command.
//!
//! One spec row, two dialogs: the rectangle's (the one `dialog.open {dialog}` finds) and the ellipse's
//! (`dialog.open {command: "path.addEllipse"}`). The click is the shape's bottom-left corner, as in `path.addRectangle`
//! and `path.addEllipse` without `fromCenter`.

use fontcraft_model::MAX_COORDINATE;
use serde_json::{Map, Value};

use super::fields::Field;
use super::spec::{DialogSpec, Fields};
use crate::app::FontCraftApp;

/// What the Primitives tool's click asks for.
pub const CLICK: &str = "dialog.primitiveMeasurements";

/// The size the fields open with, in font units.
const DEFAULT_SIZE: f64 = 100.0;

pub const RECTANGLE: DialogSpec = DialogSpec::new(CLICK, "Add Rectangle", "path.addRectangle", Fields::Fixed(fields));

pub const ELLIPSE: DialogSpec = DialogSpec::new(CLICK, "Add Ellipse", "path.addEllipse", Fields::Fixed(fields));

pub const SPECS: &[DialogSpec] = &[RECTANGLE, ELLIPSE];

fn fields(_: &fontcraft_engine::Session) -> Vec<Field> {
    let max = 2.0 * MAX_COORDINATE;
    vec![
        Field::number("width", "Width", 1.0, max, DEFAULT_SIZE, "The shape's width in font units, to the right of the click"),
        Field::number("height", "Height", 1.0, max, DEFAULT_SIZE, "The shape's height in font units, above the click"),
    ]
}

/// The dialog a click with the Primitives tool opens, with the parameters to open it with: the click's
/// `{glyph, layer?, side?, x, y, shape: "rectangle" | "ellipse"}` without `shape`.
pub fn for_click(params: &Value) -> Option<(DialogSpec, Map<String, Value>)> {
    let mut m = params.as_object()?.clone();
    let dialog = match m.remove("shape").as_ref().and_then(Value::as_str) {
        Some("ellipse") => ELLIPSE,
        _ => RECTANGLE,
    };
    Some((dialog, m))
}

/// Opens the dialog the Primitives tool's click asks for.
pub fn open_for_click(app: &mut FontCraftApp, params: &Value) -> Result<(), String> {
    let (dialog, given) = for_click(params).ok_or_else(|| format!("{CLICK}: the click names no glyph"))?;
    super::form::open(app, &dialog, &given).map(drop)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use fontcraft_engine::Session;
    use serde_json::{Value, json};

    use super::{CLICK, for_click};
    use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
    use crate::control;
    use crate::dialogs;

    fn call(app: &mut FontCraftApp, method: &str, params: Value) -> Value {
        control::dispatch(app, method, &params).unwrap_or_else(|e| panic!("{method}: {e}"))
    }

    /// Glyph A with a square, shown in the glyph panels.
    fn app() -> FontCraftApp {
        let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
        app.session.execute("glyph.newGlyph", &json!({"name": "A"})).ok();
        app.session.execute("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
        app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
        app.drain();
        app
    }

    fn font(app: &FontCraftApp) -> Arc<fontcraft_model::Font> {
        app.session.doc().unwrap().font.clone()
    }

    #[test]
    fn the_typed_size_places_the_shape_at_the_click() {
        for (shape, command) in [("rectangle", "path.addRectangle"), ("ellipse", "path.addEllipse")] {
            let click = json!({"glyph": "A", "x": 400, "y": -20, "shape": shape});
            let (dialog, params) = for_click(&click).unwrap();
            assert_eq!((dialog.id, dialog.command), (CLICK, command));
            assert!(!params.contains_key("shape"));

            // The tool's click opens the dialog as the canvas hands it over.
            let mut app = app();
            super::open_for_click(&mut app, &click).unwrap();
            let opened = call(&mut app, "ui.dialog.inspect", json!({}));
            assert_eq!(opened["dialog"], CLICK);
            assert_eq!((opened["fields"][0]["id"].clone(), opened["fields"][1]["id"].clone()), (json!("width"), json!("height")));
            call(&mut app, "ui.dialog.set", json!({"field": "width", "value": 50}));
            call(&mut app, "ui.dialog.set", json!({"field": "height", "value": 300}));
            assert!(call(&mut app, "ui.dialog.inspect", json!({}))["error"].is_null());
            call(&mut app, "ui.dialog.confirm", json!({}));

            let mut direct = self::app();
            direct.session.execute(command, &json!({"glyph": "A", "x": 400, "y": -20, "width": 50, "height": 300})).unwrap();
            assert!(font(&app).same_as(&font(&direct)), "{shape}: the dialog and the command give the same font");
        }
        assert_eq!(dialogs::find(CLICK).map(|d| d.command), Some("path.addRectangle"));

        let mut app = app();
        let original = font(&app);
        call(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": CLICK, "params": {"glyph": "A", "x": 0, "y": 0}}}));
        assert!(!font(&app).same_as(&original), "the preview shows the rectangle");
        assert!(control::dispatch(&mut app, "ui.dialog.set", &json!({"field": "width", "value": 0})).is_err(), "a size of 0 is refused");
        call(&mut app, "ui.dialog.cancel", json!({}));
        assert!(Arc::ptr_eq(&font(&app), &original), "Cancel puts back the very font the dialog opened on");
    }

    /// A click with the Primitives tool on the canvas opens the dialog instead of running an unknown command.
    #[test]
    fn a_primitives_click_asks_for_the_size() {
        let mut app = app();
        let out = fontcraft_tools::Output::with(fontcraft_tools::Action::Run {
            invocation: fontcraft_tools::Invocation::new(CLICK, json!({"glyph": "A", "x": 10, "y": 20, "shape": "ellipse"})),
        });
        let errors = crate::canvas::tools::apply::apply(&mut app, out);
        assert!(errors.is_empty(), "{errors:?}");
        let opened = call(&mut app, "ui.dialog.inspect", json!({}));
        assert_eq!((opened["dialog"].clone(), opened["command"].clone()), (json!(CLICK), json!("path.addEllipse")), "{opened}");
    }
}
