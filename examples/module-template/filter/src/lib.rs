//! A filter module: moves a layer's outline, components and anchors by `dx` and `dy` font units.
//! When the filter runs from Edit View with nodes selected, only the selected nodes move.
//!
//! To start a module of your own: rename the crate in `Cargo.toml`, give `manifest.json` your
//! own `id`, `name` and parameters, and rewrite [`filter`]. `abi.rs` stays as it is.
//!
//! Input: `{"glyph", "unicodes", "layer", "metrics", "selection"?}`, where `layer` is the layer
//! in FontCraft's native layer form. Output: `{"shapes", "anchors"?, "width"?, "log"?}`, or
//! `{"error": "…"}`. The shapes replace the layer's shapes, so every shape the module does not
//! change must be returned as it came. `docs/plugins.md` describes both in full.

mod abi;

use serde_json::{Value, json};

/// The manifest `fc_manifest` returns.
pub const MANIFEST: &str = include_str!("../manifest.json");

/// One run: the input and parameter JSON in, the output JSON out. Called by `fc_run`.
pub fn run(input: &[u8], params: &[u8]) -> Vec<u8> {
    let output = filter(input, params).unwrap_or_else(|message| json!({ "error": message }));
    serde_json::to_vec(&output).unwrap_or_default()
}

fn filter(input: &[u8], params: &[u8]) -> Result<Value, String> {
    let input: Value = serde_json::from_slice(input).map_err(|e| format!("the input is not JSON: {e}"))?;
    // The host hands over every declared parameter, checked and with defaults filled in.
    let params: Value = serde_json::from_slice(params).map_err(|e| format!("the parameters are not JSON: {e}"))?;
    let dx = params.get("dx").and_then(Value::as_f64).unwrap_or(0.0);
    let dy = params.get("dy").and_then(Value::as_f64).unwrap_or(0.0);
    let move_anchors = params.get("anchors").and_then(Value::as_bool).unwrap_or(true);

    let mut layer = input.get("layer").cloned().ok_or("the input has no layer")?;
    let selection: Vec<(u64, u64)> = input
        .get("selection")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(|n| Some((n.get("shape")?.as_u64()?, n.get("node")?.as_u64()?))).collect())
        .unwrap_or_default();
    let whole_layer = selection.is_empty();

    let mut moved = 0usize;
    let mut shapes = take_list(&mut layer, "shapes");
    for (s, shape) in shapes.iter_mut().enumerate() {
        if let Some(nodes) = shape.get_mut("path").and_then(|p| p.get_mut("nodes")).and_then(Value::as_array_mut) {
            for (n, node) in nodes.iter_mut().enumerate() {
                if whole_layer || selection.contains(&(s as u64, n as u64)) {
                    add(node, "x", dx);
                    add(node, "y", dy);
                    moved += 1;
                }
            }
        } else if let Some(component) = shape.get_mut("component")
            && whole_layer
        {
            shift_component(component, dx, dy);
        }
        // Images and shapes FontCraft keeps as read are returned unchanged.
    }

    let mut output = json!({
        "shapes": shapes,
        "log": [format!("{}: moved {moved} nodes by ({dx}, {dy})", input.get("glyph").and_then(Value::as_str).unwrap_or("?"))],
    });
    if whole_layer && move_anchors {
        let mut anchors = take_list(&mut layer, "anchors");
        for anchor in &mut anchors {
            add(anchor, "x", dx);
            add(anchor, "y", dy);
        }
        // Without `anchors` in the output the layer keeps its own.
        if let Some(out) = output.as_object_mut() {
            out.insert("anchors".into(), Value::Array(anchors));
        }
    }
    Ok(output)
}

/// The list under `key` of `object`, taken out of it; empty when it is missing (the native form
/// leaves out empty lists).
fn take_list(object: &mut Value, key: &str) -> Vec<Value> {
    match object.get_mut(key).map(Value::take) {
        Some(Value::Array(items)) => items,
        _ => Vec::new(),
    }
}

/// Adds `d` to the number under `key`, if there is one.
fn add(object: &mut Value, key: &str, d: f64) {
    if let Some(v) = object.get_mut(key)
        && let Some(n) = v.as_f64()
    {
        *v = json!(n + d);
    }
}

/// Moves a component. Its transform is either `{"decomposed": {"pos": [x, y], …}}` or
/// `{"matrix": [xx, yx, xy, yy, dx, dy]}`, and is left out when it is the identity.
fn shift_component(component: &mut Value, dx: f64, dy: f64) {
    let Some(component) = component.as_object_mut() else { return };
    let transform = component
        .entry("transform")
        .or_insert_with(|| json!({ "decomposed": { "pos": [0.0, 0.0], "scale": [1.0, 1.0], "angle": 0.0, "slant": [0.0, 0.0] } }));
    if let Some(pos) = transform.get_mut("decomposed").and_then(|d| d.get_mut("pos")).and_then(Value::as_array_mut) {
        add_at(pos, 0, dx);
        add_at(pos, 1, dy);
    } else if let Some(matrix) = transform.get_mut("matrix").and_then(Value::as_array_mut) {
        add_at(matrix, 4, dx);
        add_at(matrix, 5, dy);
    }
}

fn add_at(list: &mut [Value], index: usize, d: f64) {
    if let Some(v) = list.get_mut(index)
        && let Some(n) = v.as_f64()
    {
        *v = json!(n + d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(input: Value, params: Value) -> Value {
        serde_json::from_slice(&run(input.to_string().as_bytes(), params.to_string().as_bytes())).unwrap_or(Value::Null)
    }

    #[test]
    fn moves_the_whole_layer_or_only_the_selection() {
        let layer = json!({
            "id": "m01", "master": "m01", "width": 600.0,
            "shapes": [
                {"path": {"closed": true, "nodes": [{"x": 0.0, "y": 0.0, "type": "line"}, {"x": 100.0, "y": 0.0, "type": "line"}]}},
                {"component": {"glyph": "acutecomb"}}
            ],
            "anchors": [{"name": "top", "x": 50.0, "y": 700.0}]
        });
        let params = json!({"dx": 10.0, "dy": -5.0, "anchors": true});

        let all = output(json!({"glyph": "a", "unicodes": [97], "layer": layer.clone(), "metrics": {}}), params.clone());
        assert_eq!(all.pointer("/shapes/0/path/nodes/1/x"), Some(&json!(110.0)));
        assert_eq!(all.pointer("/shapes/1/component/transform/decomposed/pos"), Some(&json!([10.0, -5.0])));
        assert_eq!(all.pointer("/anchors/0/y"), Some(&json!(695.0)));

        let some = output(json!({"glyph": "a", "unicodes": [], "layer": layer, "metrics": {}, "selection": [{"shape": 0, "node": 1}]}), params);
        assert_eq!(some.pointer("/shapes/0/path/nodes/0/x"), Some(&json!(0.0)));
        assert_eq!(some.pointer("/shapes/0/path/nodes/1/x"), Some(&json!(110.0)));
        assert_eq!(some.pointer("/shapes/1/component/transform"), None, "components stay with a selection");
        assert_eq!(some.get("anchors"), None, "the layer keeps its anchors");
    }

    #[test]
    fn bad_input_is_an_error_not_a_trap() {
        let out = output(json!("not a layer input"), json!({}));
        assert!(out.get("error").and_then(Value::as_str).is_some());
    }
}
