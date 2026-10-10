//! Pasted outline is outside the glyph. Asked after `edit.paste` returns `outOfBounds`: OK moves
//! what was pasted (the result's `selection`) by the result's `correction` with
//! `selection.move`, which puts the pasted box's bottom left at the origin; Cancel keeps the
//! pasted position. The fields show the move and can change it. [`follow_up`] gives the
//! parameters to open it with from the paste's result.

use fontcraft_model::MAX_COORDINATE;
use serde_json::{Map, Value, json};

use super::form::Field;
use super::{After, Dialog, Fields};

pub const DIALOG: Dialog = Dialog {
    follow_up: true,
    after: Some(After { command: "edit.paste", params: follow_up }),
    buttons: ["Move", "Keep Position"],
    message: "The pasted outline lies outside the glyph box.",
    ..Dialog::new("dialog.pastedOutlineBounds", "Move Pasted Outline", "selection.move", Fields::Fixed(fields))
};

fn fields(_: &fontcraft_engine::Session) -> Vec<Field> {
    let max = 2.0 * MAX_COORDINATE;
    vec![
        Field::number("dx", "Move X", -max, max, 0.0, "Horizontal move in font units that brings the pasted outline into the glyph box"),
        Field::number("dy", "Move Y", -max, max, 0.0, "Vertical move in font units that brings the pasted outline into the glyph box"),
    ]
}

/// The parameters to open the dialog with after `edit.paste` returned `result`: its
/// `selection` and its `correction` as `dx`, `dy`. `None` unless the paste is out of bounds.
pub fn follow_up(_request: &Value, result: &Value) -> Option<Map<String, Value>> {
    if result.get("outOfBounds").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let mut m = result.get("selection")?.as_object()?.clone();
    let correction = result.get("correction")?.as_array()?;
    let (dx, dy) = (correction.first()?.as_f64()?, correction.get(1)?.as_f64()?);
    m.insert("dx".into(), json!(dx));
    m.insert("dy".into(), json!(dy));
    Some(m)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};

    use super::follow_up;
    use crate::{FontcraftApp, control};

    fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
        let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["result"].clone()
    }

    fn font(app: &FontcraftApp) -> Arc<fontcraft_model::Font> {
        app.session.doc().unwrap().font.clone()
    }

    /// Glyph A, edited in a tab, with a square pasted 5000 units to the right; the paste's result.
    fn pasted(svg_x: f64) -> (FontcraftApp, Value) {
        let mut app = FontcraftApp::default();
        app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        let svg =
            format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1000"><rect x="{svg_x}" y="0" width="100" height="100"/></svg>"#);
        let result = app.run("edit.paste", &json!({"glyph": "A", "svg": svg})).unwrap();
        (app, result)
    }

    #[test]
    fn ok_moves_the_pasted_outline_into_the_glyph_and_cancel_keeps_it() {
        let (mut app, result) = pasted(5000.0);
        let params = follow_up(&Value::Null, &result).unwrap();
        assert_eq!((params["dx"].clone(), params["dy"].clone()), (json!(-5000.0), json!(100.0)), "{result}");
        assert_eq!(params["shapes"], json!([1]), "the pasted square");
        let opened =
            call(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": "dialog.pastedOutlineBounds", "params": params}}));
        assert_eq!(opened["fields"][0]["value"], -5000.0);
        assert!(opened["error"].is_null(), "{opened}");
        call(&mut app, "ui.dialog.confirm", json!({}));

        let (mut direct, _) = pasted(5000.0);
        direct.run("selection.move", &Value::Object(params.clone())).unwrap();
        assert!(font(&app).same_as(&font(&direct)), "the dialog and the command give the same font");

        let (mut app, _) = pasted(5000.0);
        let original = font(&app);
        call(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": "dialog.pastedOutlineBounds", "params": params}}));
        assert!(!font(&app).same_as(&original), "the preview shows the moved outline");
        call(&mut app, "ui.dialog.cancel", json!({}));
        assert!(Arc::ptr_eq(&font(&app), &original), "Cancel keeps the pasted position");

        let (_, near) = pasted(0.0);
        assert!(follow_up(&Value::Null, &near).is_none(), "a paste inside the glyph box asks nothing");
    }
}
