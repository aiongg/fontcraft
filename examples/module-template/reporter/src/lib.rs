//! A reporter module: a dot on every on-curve node, a line at the x-height behind the outline,
//! and the node count below the glyph being edited.
//!
//! To start a module of your own: rename the crate in `Cargo.toml`, give `manifest.json` your
//! own `id`, `name` and parameters, and rewrite [`report`]. `abi.rs` stays as it is.
//!
//! Input: `{"glyph", "unicodes", "layer", "metrics", "scale", "active"}`. Output:
//! `{"background": [primitive…], "foreground": [primitive…]}`, or `{"error": "…"}`.
//! Coordinates and radii are font units; stroke widths and text sizes are screen pixels, so a
//! size meant to look the same at every zoom is divided by `scale` (screen pixels per font unit)
//! when it becomes font units. The app runs a reporter again only when the layer, the
//! parameters or the zoom change. `docs/plugins.md` lists every primitive.

mod abi;

use serde_json::{Value, json};

/// The manifest `fc_manifest` returns.
pub const MANIFEST: &str = include_str!("../manifest.json");

/// Most dots drawn; the host refuses a drawing of more than 100,000 primitives.
const MAX_DOTS: usize = 50_000;

/// One run: the input and parameter JSON in, the output JSON out. Called by `fc_run`.
pub fn run(input: &[u8], params: &[u8]) -> Vec<u8> {
    let output = report(input, params).unwrap_or_else(|message| json!({ "error": message }));
    serde_json::to_vec(&output).unwrap_or_default()
}

fn report(input: &[u8], params: &[u8]) -> Result<Value, String> {
    let input: Value = serde_json::from_slice(input).map_err(|e| format!("the input is not JSON: {e}"))?;
    let params: Value = serde_json::from_slice(params).map_err(|e| format!("the parameters are not JSON: {e}"))?;
    let size = params.get("size").and_then(Value::as_f64).unwrap_or(5.0);
    let color = match params.get("color").and_then(Value::as_str) {
        Some("red") => [220, 40, 40, 255],
        Some("grey") => [120, 120, 120, 255],
        _ => [30, 110, 230, 255],
    };
    let count = params.get("count").and_then(Value::as_bool).unwrap_or(true);
    let scale = input.get("scale").and_then(Value::as_f64).filter(|s| s.is_finite() && *s > 0.0).unwrap_or(1.0);
    let active = input.get("active").and_then(Value::as_bool).unwrap_or(false);
    let layer = input.get("layer").ok_or("the input has no layer")?;

    let mut foreground = Vec::new();
    let mut nodes = 0usize;
    for shape in layer.get("shapes").and_then(Value::as_array).into_iter().flatten() {
        for node in shape.get("path").and_then(|p| p.get("nodes")).and_then(Value::as_array).into_iter().flatten() {
            nodes += 1;
            if node.get("type").and_then(Value::as_str) == Some("offcurve") || foreground.len() >= MAX_DOTS {
                continue;
            }
            let (Some(x), Some(y)) = (node.get("x").and_then(Value::as_f64), node.get("y").and_then(Value::as_f64)) else { continue };
            foreground.push(json!({ "type": "circle", "center": [x, y], "radius": size / 2.0 / scale, "fill": color }));
        }
    }
    if count && active {
        // 3 dot sizes below the baseline, whatever the zoom.
        foreground.push(json!({ "type": "text", "at": [0.0, -3.0 * size / scale], "text": format!("{nodes} nodes"), "color": color, "size": 12.0 }));
    }

    let mut background = Vec::new();
    let width = layer.get("width").and_then(Value::as_f64).unwrap_or(0.0);
    if let Some(x_height) = input.get("metrics").and_then(|m| m.get("x-height")).and_then(Value::as_f64) {
        let faint = [color[0], color[1], color[2], 90];
        background.push(json!({ "type": "line", "from": [0.0, x_height], "to": [width, x_height], "stroke": { "color": faint, "width": 1.0 } }));
    }
    Ok(json!({ "background": background, "foreground": foreground }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dots_on_curve_nodes_sized_for_the_zoom() {
        let input = json!({
            "glyph": "o", "unicodes": [111], "metrics": {"x-height": 500.0}, "scale": 0.5, "active": true,
            "layer": {"id": "m01", "master": "m01", "width": 600.0, "shapes": [{"path": {"closed": true, "nodes": [
                {"x": 0.0, "y": 0.0, "type": "line"}, {"x": 50.0, "y": 50.0, "type": "offcurve"},
                {"x": 60.0, "y": 50.0, "type": "offcurve"}, {"x": 100.0, "y": 100.0, "type": "curve"}
            ]}}]}
        });
        let out: Value =
            serde_json::from_slice(&run(input.to_string().as_bytes(), br#"{"size": 4, "color": "red", "count": true}"#)).unwrap_or(Value::Null);
        let foreground = out.get("foreground").and_then(Value::as_array).cloned().unwrap_or_default();
        let dots: Vec<&Value> = foreground.iter().filter(|p| p.get("type") == Some(&json!("circle"))).collect();
        assert_eq!(dots.len(), 2, "off-curve nodes get no dot");
        assert_eq!(dots[0].get("radius"), Some(&json!(4.0)), "4 pixels across at half size is 8 units across");
        assert!(foreground.iter().any(|p| p.get("text") == Some(&json!("4 nodes"))));
        assert_eq!(out.pointer("/background/0/to"), Some(&json!([600.0, 500.0])));
    }
}
