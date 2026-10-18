//! A check module: reports open paths, and nodes whose coordinates are not multiples of the grid
//! step.
//!
//! To start a module of your own: rename the crate in `Cargo.toml`, give `manifest.json` your
//! own `id`, `name` and parameters, and rewrite [`check`]. `abi.rs` stays as it is.
//!
//! Input: `{"glyph", "unicodes", "layer", "metrics"}`. Output: `{"findings": [{"name",
//! "severity", "message", "position"?}]}`, or `{"error": "…"}`. The audit lists each finding
//! under the check id `check.plugin.<module id>.<name>`; `severity` is `info`, `warning` or
//! `error`; `position` is `[x, y]` in font units, and the canvas marks it.
//! `docs/plugins.md` describes the format in full.

mod abi;

use serde_json::{Value, json};

/// The manifest `fc_manifest` returns.
pub const MANIFEST: &str = include_str!("../manifest.json");

/// Most findings reported for one layer; the host refuses more than 10,000.
const MAX_FINDINGS: usize = 1_000;

/// One run: the input and parameter JSON in, the output JSON out. Called by `fc_run`.
pub fn run(input: &[u8], params: &[u8]) -> Vec<u8> {
    let output = check(input, params).unwrap_or_else(|message| json!({ "error": message }));
    serde_json::to_vec(&output).unwrap_or_default()
}

fn check(input: &[u8], params: &[u8]) -> Result<Value, String> {
    let input: Value = serde_json::from_slice(input).map_err(|e| format!("the input is not JSON: {e}"))?;
    let params: Value = serde_json::from_slice(params).map_err(|e| format!("the parameters are not JSON: {e}"))?;
    let grid = params.get("grid").and_then(Value::as_f64).filter(|g| *g >= 1.0).unwrap_or(1.0);
    let open_paths = params.get("openPaths").and_then(Value::as_bool).unwrap_or(true);
    let layer = input.get("layer").ok_or("the input has no layer")?;

    let mut findings = Vec::new();
    let shapes = layer.get("shapes").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    for (s, shape) in shapes.iter().enumerate() {
        let Some(path) = shape.get("path") else { continue };
        let nodes = path.get("nodes").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        let point = |node: &Value| Some([node.get("x")?.as_f64()?, node.get("y")?.as_f64()?]);
        // `closed` is left out of the native form when it is true.
        if open_paths && path.get("closed").and_then(Value::as_bool) == Some(false) {
            let mut finding = json!({ "name": "openPath", "severity": "warning", "message": format!("path {s} is open") });
            if let (Some(at), Some(object)) = (nodes.first().and_then(point), finding.as_object_mut()) {
                object.insert("position".into(), json!(at));
            }
            findings.push(finding);
        }
        for (n, node) in nodes.iter().enumerate() {
            let Some([x, y]) = point(node) else { continue };
            if findings.len() >= MAX_FINDINGS {
                break;
            }
            if x % grid != 0.0 || y % grid != 0.0 {
                findings.push(json!({
                    "name": "offGrid",
                    "severity": "info",
                    "message": format!("node {n} of path {s} at ({x}, {y}) is off the {grid}-unit grid"),
                    "position": [x, y],
                }));
            }
        }
    }
    Ok(json!({ "findings": findings }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_path_and_off_grid_node() {
        let input = json!({
            "glyph": "l", "unicodes": [108], "metrics": {},
            "layer": {"id": "m01", "master": "m01", "width": 300.0, "shapes": [
                {"path": {"closed": false, "nodes": [{"x": 100.0, "y": 0.0, "type": "line"}, {"x": 100.5, "y": 700.0, "type": "line"}]}},
                {"path": {"nodes": [{"x": 0.0, "y": 0.0, "type": "line"}, {"x": 10.0, "y": 0.0, "type": "line"}, {"x": 10.0, "y": 10.0, "type": "line"}]}}
            ]}
        });
        let out: Value = serde_json::from_slice(&run(input.to_string().as_bytes(), br#"{"grid": 1, "openPaths": true}"#)).unwrap_or(Value::Null);
        let names: Vec<&str> = out.get("findings").and_then(Value::as_array).into_iter().flatten().filter_map(|f| f.get("name")?.as_str()).collect();
        assert_eq!(names, ["openPath", "offGrid"]);
        assert_eq!(out.pointer("/findings/0/position"), Some(&json!([100.0, 0.0])));
    }
}
