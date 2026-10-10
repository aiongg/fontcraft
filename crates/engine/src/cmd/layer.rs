//! Queries on one layer: `layer.inspect`, the outline with the address of everything in it.

use fontcraft_model::{NodeKind, Shape};
use serde_json::{Value, json};

use super::*;
use crate::params::{GuideScope, Params, TargetKind, Targets};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "layer.inspect",
            "Inspect Layer",
            [],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\"} one layer with the address of every shape, node, anchor, guide and annotation, in the form the editing commands' targets use → {glyph, layer, master, side, locked, width, shapes: [{index, kind: \"path\" | \"component\" | \"image\" | \"other\", closed?, locked?, nodes?: [{index, x, y, type: \"line\" | \"curve\" | \"qcurve\" | \"offcurve\", smooth}], component?: name, transform?: [xx, xy, yx, yy, dx, dy]}], anchors: [{name, x, y}], guides: [{scope: \"local\" | \"global\", index, x, y, angle, locked}], annotations: [{index, type, x, y, text?}]}",
            has_doc,
            inspect
        )
        .targets(&[TargetKind::Layer]),
    ]
}

fn node_type(k: NodeKind) -> &'static str {
    match k {
        NodeKind::Line => "line",
        NodeKind::Curve => "curve",
        NodeKind::QCurve => "qcurve",
        NodeKind::OffCurve => "offcurve",
    }
}

fn inspect(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("layer.inspect", p)?;
    let t = Targets::parse(&p)?;
    if t.has_lists() {
        return Err(p.bad("takes no target lists: it returns the whole layer"));
    }
    let r = t.resolve(s.font()?)?;
    let shapes: Vec<Value> = r
        .parts
        .shapes
        .iter()
        .enumerate()
        .map(|(index, shape)| match shape {
            Shape::Path(path) => {
                let nodes: Vec<Value> = path
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(i, n)| json!({"index": i, "x": n.x, "y": n.y, "type": node_type(n.kind), "smooth": n.smooth}))
                    .collect();
                json!({"index": index, "kind": "path", "closed": path.closed, "locked": path.info.locked, "nodes": nodes})
            }
            Shape::Component(c) => {
                json!({"index": index, "kind": "component", "component": c.glyph, "locked": c.info.locked, "transform": c.affine().as_coeffs()})
            }
            Shape::Image(_) => json!({"index": index, "kind": "image"}),
            Shape::Raw(_) => json!({"index": index, "kind": "other"}),
        })
        .collect();
    let anchors: Vec<Value> = r.parts.anchors.iter().map(|a| json!({"name": a.name, "x": a.x, "y": a.y})).collect();
    let guide = |scope: GuideScope, (index, g): (usize, &fontcraft_model::Guide)| json!({"scope": scope.as_str(), "index": index, "x": g.x, "y": g.y, "angle": g.angle, "locked": g.locked});
    let guides: Vec<Value> = r
        .parts
        .guides
        .iter()
        .enumerate()
        .map(|g| guide(GuideScope::Local, g))
        .chain(r.master.guides.iter().enumerate().map(|g| guide(GuideScope::Global, g)))
        .collect();
    let annotations: Vec<Value> = r
        .parts
        .annotations
        .iter()
        .enumerate()
        .map(|(index, a)| {
            let mut v = json!({"index": index, "type": a.kind, "x": a.x, "y": a.y});
            if let (Some(text), Some(o)) = (&a.text, v.as_object_mut()) {
                o.insert("text".into(), json!(text));
            }
            v
        })
        .collect();
    Ok(json!({
        "glyph": r.glyph.name,
        "layer": r.layer.id,
        "master": r.layer.master,
        "side": t.side.as_str(),
        "locked": r.glyph.info.locked,
        "width": r.layer.width,
        "shapes": shapes,
        "anchors": anchors,
        "guides": guides,
        "annotations": annotations,
    }))
}
