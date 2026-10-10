//! Filter › Shape Order: the layer's shapes as a list in stacking order, rows dragged into a new
//! order, previewed with `filter.shapeOrder`.
//!
//! The first row is shape 0, drawn first, at the bottom of the stack. Each row names the shape's
//! index when the dialog opened and what it is (from `layer.inspect`). Agents give the order as
//! a list of those indices with `ui.dialog.set {field: "order"}`.

use fontcraft_engine::Session;
use serde_json::{Map, Value};

use super::form::{Field, MAX_ORDER_ROWS};
use super::{Dialog, Fields};

pub const DIALOG: Dialog = Dialog::new("dialog.shapeOrder", "Shape Order", "filter.shapeOrder", Fields::Targeted(fields));

/// One field: the order of the target layer's shapes.
fn fields(session: &mut Session, targets: &Map<String, Value>) -> Result<Vec<Field>, String> {
    let layer: Map<String, Value> = ["glyph", "layer", "side"].iter().filter_map(|k| targets.get(*k).map(|v| (k.to_string(), v.clone()))).collect();
    if !layer.contains_key("glyph") {
        return Err("Shape Order: select a glyph first".into());
    }
    let shown = session.execute("layer.inspect", &Value::Object(layer)).map_err(|e| e.to_string())?;
    let shapes = shown.get("shapes").and_then(Value::as_array).ok_or("Shape Order: the layer lists no shapes")?;
    if shapes.is_empty() {
        return Err("Shape Order: the layer has no shapes".into());
    }
    if shapes.len() > MAX_ORDER_ROWS {
        return Err(format!(
            "Shape Order: the layer has {} shapes; the dialog lists at most {MAX_ORDER_ROWS}, so give `order` to filter.shapeOrder",
            shapes.len()
        ));
    }
    let labels = shapes.iter().enumerate().map(|(i, s)| row(i, s)).collect();
    Ok(vec![Field::order("order", "Shapes", labels, "Bottom of the stack first. Drag a row onto another to move it there")])
}

/// The text of a shape's row, from its `layer.inspect` entry.
fn row(index: usize, shape: &Value) -> String {
    let text = |key: &str| shape.get(key).and_then(Value::as_str).unwrap_or_default();
    match text("kind") {
        "path" => {
            let nodes = shape.get("nodes").and_then(Value::as_array).map_or(0, Vec::len);
            let open = if shape.get("closed").and_then(Value::as_bool) == Some(false) { ", open" } else { "" };
            format!("{index}: Path ({nodes} nodes{open})")
        }
        "component" => format!("{index}: Component {}", text("component")),
        "image" => format!("{index}: Image"),
        _ => format!("{index}: Other shape"),
    }
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

    /// Glyph A with three squares, edited in a tab.
    fn three_shapes() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        for (x, size) in [(100, 200), (150, 100), (400, 50)] {
            app.run("path.addRectangle", &json!({"glyph": "A", "x": x, "y": 100, "width": size, "height": size})).unwrap();
        }
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        app
    }

    fn font(app: &FontcraftApp) -> Arc<Font> {
        app.session.doc().unwrap().font.clone()
    }

    #[test]
    fn the_rows_are_the_layers_shapes_and_the_order_previews_and_matches_the_command() {
        let mut app = three_shapes();
        let original = font(&app);
        let opened = call(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"command": "filter.shapeOrder"}}));
        assert_eq!(opened["dialog"], "dialog.shapeOrder");
        let field = &opened["fields"][0];
        assert_eq!((field["id"].clone(), field["kind"].clone()), (json!("order"), json!("order")));
        assert_eq!(field["labels"], json!(["0: Path (4 nodes)", "1: Path (4 nodes)", "2: Path (4 nodes)"]), "{field}");
        assert_eq!(field["value"], json!([0, 1, 2]));
        assert_eq!(opened["params"]["glyph"], "A");

        for bad in [json!([0, 1]), json!([0, 0, 1]), json!([0, 1, 3]), json!("2,1,0")] {
            fails(&mut app, "ui.dialog.set", json!({"field": "order", "value": bad}));
        }
        call(&mut app, "ui.dialog.set", json!({"field": "order", "value": [2, 0, 1]}));
        assert!(!font(&app).same_as(&original), "the canvas shows the new order");
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| app.ui(ui)).textures_delta.clear();
        call(&mut app, "ui.dialog.confirm", json!({}));

        let mut direct = three_shapes();
        direct.run("filter.shapeOrder", &json!({"glyph": "A", "order": [2, 0, 1]})).unwrap();
        assert!(font(&app).same_as(&font(&direct)), "the dialog and the command give the same font");

        let before = font(&app);
        call(
            &mut app,
            "engine.execute",
            json!({"command": "dialog.open", "params": {"dialog": "dialog.shapeOrder", "params": {"glyph": "A", "order": [1, 0, 2]}}}),
        );
        assert!(!font(&app).same_as(&before));
        call(&mut app, "ui.dialog.cancel", json!({}));
        assert!(Arc::ptr_eq(&font(&app), &before), "Cancel puts back the very font the dialog opened on");

        let empty =
            fails(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": "dialog.shapeOrder", "params": {"glyph": "C"}}}));
        assert!(!empty.is_empty());
    }

    #[test]
    fn a_dropped_row_takes_the_place_it_is_dropped_on() {
        use super::super::form::moved;
        assert_eq!(moved(&[0, 1, 2, 3], 0, 2), Some(vec![1, 2, 0, 3]));
        assert_eq!(moved(&[0, 1, 2, 3], 3, 0), Some(vec![3, 0, 1, 2]));
        assert_eq!(moved(&[0, 1, 2], 1, 1), None);
        assert_eq!(moved(&[0, 1, 2], 5, 0), None);
    }
}
