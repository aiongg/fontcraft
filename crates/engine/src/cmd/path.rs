//! Path menu and the headless forms of the drawing tools.

use fontcraft_geom::round_point;
use fontcraft_model::{Layer, Node, NodeKind, Path, Shape};
use serde_json::{Value, json};

use super::*;
use crate::params::{Params, TargetKind, Targets, kind};

/// Most nodes one path may be given (a guard against hostile input).
const MAX_NODES: usize = 100_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "path.addPath",
        "Add Path",
        [],
        None,
        "{glyph: name, layer?: layer id (default: the first master's layer; a master's layer is added when the glyph has none), side?: \"foreground\" | \"background\", closed?: bool = true, nodes: [{x, y, type?: \"line\" | \"curve\" | \"qcurve\" | \"offcurve\" (default line), smooth?: bool}]} append a path, nodes in path direction: two offcurve nodes before each curve node, one or more before a qcurve node; coordinates within ±1,000,000, rounded to the grid, at most 100,000 nodes. A locked glyph refuses it → {glyph, layer, shape: index, nodes: count, selection: {glyph, layer, shapes: [index]}}",
        has_doc,
        add_path
    )
    .targets(&[TargetKind::Layer])]
}

fn node(p: &Params, v: &Value) -> Result<Node> {
    let Value::Object(o) = v else {
        return Err(p.bad(format!("each node must be an object {{x, y, type?}}, found {}", kind(v))));
    };
    let coord = |key: &str| {
        o.get(key).and_then(Value::as_f64).filter(|n| n.is_finite()).ok_or_else(|| p.bad(format!("a node's `{key}` must be a finite number")))
    };
    let node_kind = match o.get("type").and_then(Value::as_str).unwrap_or("line") {
        "line" => NodeKind::Line,
        "curve" => NodeKind::Curve,
        "qcurve" => NodeKind::QCurve,
        "offcurve" => NodeKind::OffCurve,
        other => return Err(p.bad(format!("a node's `type` must be line, curve, qcurve or offcurve, got \"{other}\""))),
    };
    let mut out = Node::new(coord("x")?, coord("y")?, node_kind);
    out.smooth = o.get("smooth").and_then(Value::as_bool).unwrap_or(false);
    Ok(out)
}

fn add_path(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("path.addPath", p)?;
    let glyph = p.str("glyph")?.to_string();
    let list = p.array("nodes")?;
    if list.is_empty() {
        return Err(p.bad("`nodes` is empty"));
    }
    if list.len() > MAX_NODES {
        return Err(p.bad(format!("`nodes` holds {} nodes; at most {MAX_NODES} are allowed", list.len())));
    }
    let step = super::node::grid(s.font()?);
    let mut nodes = list.iter().map(|v| node(&p, v)).collect::<Result<Vec<_>>>()?;
    for n in &mut nodes {
        let at = round_point(n.point(), step);
        (n.x, n.y) = (at.x, at.y);
    }
    let path = Path::new(p.bool_or("closed", true)?, nodes);
    path.to_bez().map_err(|why| p.bad(why))?;
    let count = path.nodes.len();
    let mut t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes no target lists: it adds a path to the layer"));
    }
    let layer = t.layer_id(s.font()?)?;
    t.layer = Some(layer.clone());
    let index = s.edit("Add Path", |font| {
        let is_master = font.master(&layer).is_ok();
        let mut g = font.glyph_mut(&glyph)?;
        if is_master && !g.layers.iter().any(|l| l.id == layer) {
            crate::check_unlocked(&g)?;
            g.layers.push(Layer::new(&layer, 600.0));
        }
        drop(g);
        t.edit_layer(font, |l| {
            let shapes = l.parts_mut(t.side).shapes;
            shapes.push(Shape::Path(path));
            Ok(shapes.len() - 1)
        })
    })?;
    t.shapes = vec![index];
    Ok(json!({ "glyph": glyph, "layer": layer, "shape": index, "nodes": count, "selection": t.to_json() }))
}
