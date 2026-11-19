//! Filter › Shape Order (`dialog.shapeOrder`): the layer's shapes as a list in stacking order, rows dragged into a new
//! order, previewed with `filter.shapeOrder`.
//!
//! The first row is shape 0, drawn first, at the bottom of the stack. Each row names the shape's index when the dialog
//! opened and what it is (from `layer.inspect`). Agents give the order as a list of those indices with
//! `ui.dialog.set {field: "order"}`.

use fontcraft_engine::Session;
use serde_json::{Map, Value};

use super::fields::Field;
use super::spec::{DialogSpec, Fields};

pub const SPECS: &[DialogSpec] = &[DialogSpec::new("dialog.shapeOrder", "Shape Order", "filter.shapeOrder", Fields::Targeted(fields))];

/// Most shapes the dialog lists; a layer with more is reordered with `filter.shapeOrder` directly.
const MAX_ORDER_ROWS: usize = 4096;

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
