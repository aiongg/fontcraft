//! Per-node stroke widths (`thickness` row, M13.38): the command behind the Thickness tool's
//! per-node gestures. The whole-path gestures use `path.setStroke`. The widths live in each
//! node's user data ([`fontcraft_model::node_width`]) and the stroke outline reads them
//! (`fontcraft_compile::thickness`).

use fontcraft_model::node_width::{self, MAX_NODE_WIDTH, valid_side};
use fontcraft_model::{Node, Path};
use serde_json::{Value, json};

use super::stroke::{format_version, stroke_style};
use super::*;
use crate::params::{Params, TargetKind, Targets};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.setNodeWidths",
            "Set Node Widths",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\", points: [[shape, node]] (on-curve nodes of stroked paths), left?: units | [units] (one per point), right?: units | [units], clear?: bool} give each node its own stroke widths: `left` and `right` are the distances from the node to the stroke's left and right edges, looking along the path direction, from 0 to 10000 units. A side not given keeps the width the node is drawn with (its own, else the path's stroke width split by the stroke position). Between nodes the stroke's widths change smoothly; nodes without their own widths take the path's. `clear: true` removes the nodes' own widths and takes no `left` or `right`. A locked glyph refuses it → {glyph, layer, points: [[shape, node]], widths: [[left, right] | null]}",
            has_doc,
            set_node_widths
        )
        .targets(&[TargetKind::Layer, TargetKind::Points]),
    ]
}

/// One side's values from `left` or `right`: a number for every point, or one per point.
fn side_values(p: &Params, key: &str, count: usize) -> Result<Option<Vec<f64>>> {
    let Some(v) = p.get(key) else { return Ok(None) };
    let values: Vec<f64> = match v {
        Value::Number(_) => vec![p.f64(key)?; count],
        Value::Array(items) => {
            if items.len() != count {
                return Err(p.bad(format!("`{key}` holds {} values for {count} points; give one per point or a single number", items.len())));
            }
            items.iter().map(|i| i.as_f64().ok_or_else(|| p.bad(format!("`{key}` must hold numbers, found {i}")))).collect::<Result<_>>()?
        }
        other => return Err(p.bad(format!("`{key}` must be a number or a list of numbers, found {other}"))),
    };
    if let Some(bad) = values.iter().find(|w| !valid_side(**w)) {
        return Err(p.bad(format!("`{key}` must be from 0 to {MAX_NODE_WIDTH} units, got {bad}")));
    }
    Ok(Some(values))
}

/// The `[left, right]` widths `node` of `path` is drawn with, or why it takes none.
fn drawn_widths(path: &Path, node: &Node, version: u32) -> std::result::Result<[f64; 2], String> {
    let style = stroke_style(&path.info.attrs, version)?.ok_or("the path has no stroke")?;
    Ok(node_width::resolved(node, style.width, style.position))
}

fn set_node_widths(s: &mut Session, value: &Value) -> Result<Value> {
    let p = Params::new("path.setNodeWidths", value)?;
    let t = Targets::parse(&p)?;
    if t.points.is_empty() {
        return Err(p.bad("`points` is required: the nodes to change"));
    }
    if !(t.segments.is_empty() && t.shapes.is_empty() && t.anchors.is_empty() && t.guides.is_empty() && t.annotations.is_empty()) {
        return Err(p.bad("takes `points` only"));
    }
    let clear = p.bool_or("clear", false)?;
    let count = t.points.len();
    let (left, right) = (side_values(&p, "left", count)?, side_values(&p, "right", count)?);
    if clear && (left.is_some() || right.is_some()) {
        return Err(p.bad("`clear` takes no `left` or `right`"));
    }
    if !clear && left.is_none() && right.is_none() {
        return Err(p.bad("give `left`, `right` or both, or `clear: true`"));
    }
    let version = format_version(s.font()?);
    let widths = s.edit(if clear { "Clear Node Widths" } else { "Set Node Widths" }, |font| {
        t.edit_layer(font, |layer| {
            let mut parts = layer.parts_mut(t.side);
            let mut out = Vec::with_capacity(count);
            for (k, &(si, ni)) in t.points.iter().enumerate() {
                let path = parts.path_mut(si).ok_or_else(|| p.bad(format!("shape {si} is not a path")))?;
                let what = |e: String| p.bad(format!("`points`: [{si}, {ni}]: {e}"));
                let current = {
                    let node = path.nodes.get(ni).ok_or_else(|| what("does not resolve".into()))?;
                    if !node.on_curve() {
                        return Err(what("is not an on-curve node".into()));
                    }
                    drawn_widths(path, node, version).map_err(what)?
                };
                let new = if clear {
                    None
                } else {
                    let [l, r] = current;
                    let pick = |side: &Option<Vec<f64>>, old: f64| side.as_ref().and_then(|v| v.get(k).copied()).unwrap_or(old);
                    Some([pick(&left, l), pick(&right, r)])
                };
                let node = path.nodes.get_mut(ni).ok_or_else(|| what("does not resolve".into()))?;
                node_width::set(node, new).map_err(what)?;
                out.push(new.map_or(Value::Null, |[l, r]| json!([l, r])));
            }
            Ok(out)
        })
    })?;
    let layer = t.layer_id(s.font()?)?;
    let points: Vec<Value> = t.points.iter().map(|(a, b)| json!([a, b])).collect();
    Ok(json!({"glyph": t.glyph, "layer": layer, "points": points, "widths": widths}))
}
