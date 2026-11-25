//! Edit › Copy Attributes and Paste Attributes (rows `edit.copyAttributes`, `edit.pasteAttributes`): how a path is
//! drawn, or what a node is, from the selection onto the selected paths or nodes.
//!
//! - Copying a path takes its stroke (width, height, position, join, caps, cap axis), fill, colours, gradient, shadows
//!   and mask. Copying one node of a path takes its type and its name.
//! - Pasting replaces what was copied: a path's attributes on every named path (a point names its path), colours,
//!   gradient and shadows on full-colour layers only; a node's type on every named on-curve node, and its name when it
//!   had one. No point moves.
//! - The text is untrusted: its numbers, colours, stroke and name are checked before anything changes.

use std::collections::BTreeSet;

use fontcraft_model::plist::{Str, Value as Plist};
use fontcraft_model::{Family, LayerParts, MAX_NAME_LEN, Node, Path, Shape, ShapeAttrs};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::attributes::is_full_color;
use super::stroke::{CAP_AXIS_KEY, cap_axis, stroke_style};
use super::*;
use crate::params::{Params, TargetKind as K, Targets};

/// The `format` of the clipboard text.
pub const ATTRIBUTES_FORMAT: &str = "fontcraft-attributes";
/// The newest version of that text this version writes and reads.
const ATTRIBUTES_VERSION: u32 = 1;
/// Longest clipboard text read as copied attributes, in bytes.
const MAX_ATTRIBUTES_TEXT: usize = 1 << 20;

pub fn specs() -> Vec<CommandSpec> {
    const TARGETS: &[K] = &[K::Layer, K::Points, K::Shapes];
    vec![
        cmd!(
            query "edit.copyAttributes",
            "Copy Attributes",
            ["Edit"],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\", points?: [[shape, node]], shapes?: [index]} the attributes of the first path in `shapes`, or of the first path whose on-curve nodes are all in `points`: stroke, fill, colours, gradient, shadows and mask. Else, with one on-curve node in `points`, the node's type and name; else the attributes of the first point's path → {text: string (for edit.pasteAttributes), kind: \"path\" | \"node\"}",
            has_doc,
            copy
        )
        .targets(TARGETS),
        cmd!(
            "edit.pasteAttributes",
            "Paste Attributes",
            ["Edit"],
            None,
            "{glyph: name, layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\", points?: [[shape, node]], shapes?: [index], text: string (from edit.copyAttributes)} gives the copied attributes to what the targets name, replacing what it has: a path's to every path in `shapes` and of `points` (colours, gradient and shadows on a full-colour layer only), a node's type to every on-curve node in `points`, with its name when it has one. No point moves → {glyph, kind: \"path\" | \"node\", changed: count, selection}",
            has_doc,
            paste
        )
        .targets(TARGETS),
    ]
}

/// Copied attributes as clipboard text.
#[derive(Serialize, Deserialize)]
struct Clip {
    format: String,
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<ShapeAttrs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    node: Option<NodeAttrs>,
}

#[derive(Clone, Serialize, Deserialize)]
struct NodeAttrs {
    smooth: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Deserialize)]
struct Header {
    format: Option<String>,
    version: Option<u32>,
}

/// Puts the copied attributes of `from` in `to`; colours, gradient and shadows with `color` only.
fn put(to: &mut ShapeAttrs, from: &ShapeAttrs, color: bool) {
    to.stroke_width.clone_from(&from.stroke_width);
    to.stroke_height.clone_from(&from.stroke_height);
    to.stroke_pos.clone_from(&from.stroke_pos);
    to.line_cap_start = from.line_cap_start;
    to.line_cap_end = from.line_cap_end;
    to.line_join = from.line_join;
    to.fill = from.fill;
    to.mask = from.mask;
    if from.extra.get(Family::Native, CAP_AXIS_KEY).is_some() {
        to.extra.insert(Family::Native, CAP_AXIS_KEY, Plist::String(Str::from(cap_axis(from).name())));
    } else {
        to.extra.remove(Family::Native, CAP_AXIS_KEY);
    }
    if color {
        to.fill_color.clone_from(&from.fill_color);
        to.stroke_color.clone_from(&from.stroke_color);
        to.gradient.clone_from(&from.gradient);
        to.shadow.clone_from(&from.shadow);
        to.shadow_in.clone_from(&from.shadow_in);
    }
}

/// `points` and `shapes` only, and at least one of them.
fn targets(p: &Params) -> Result<Targets> {
    let t = Targets::parse(p)?;
    if !(t.segments.is_empty() && t.anchors.is_empty() && t.guides.is_empty() && t.annotations.is_empty()) {
        return Err(p.bad("takes only `points` and `shapes`"));
    }
    if t.points.is_empty() && t.shapes.is_empty() {
        return Err(p.bad("give paths in `shapes`, or nodes in `points`"));
    }
    Ok(t)
}

/// The first path of `t` that is named whole.
fn whole_path<'f>(parts: LayerParts<'f>, t: &Targets) -> Option<&'f Path> {
    let named = t.shapes.iter().find_map(|&i| match parts.shapes.get(i) {
        Some(Shape::Path(path)) => Some(path),
        _ => None,
    });
    named.or_else(|| {
        super::node::group(&t.points).into_iter().find_map(|(i, set)| {
            let path = match parts.shapes.get(i) {
                Some(Shape::Path(path)) => path,
                _ => return None,
            };
            path.nodes.iter().enumerate().all(|(k, n)| !n.on_curve() || set.contains(&k)).then_some(path)
        })
    })
}

fn copy(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("edit.copyAttributes", v)?;
    let t = targets(&p)?;
    let r = t.resolve(s.font()?)?;
    let on_curve: Vec<&Node> = t.points.iter().filter_map(|&(i, k)| r.parts.node(i, k)).filter(|n| n.on_curve()).collect();
    let mut clip = Clip { format: ATTRIBUTES_FORMAT.into(), version: ATTRIBUTES_VERSION, path: None, node: None };
    if let (None, [node]) = (whole_path(r.parts, &t), on_curve.as_slice()) {
        clip.node = Some(NodeAttrs { smooth: node.smooth, name: node.info.name.clone() });
    } else {
        let first = || {
            t.points.first().and_then(|&(i, _)| match r.parts.shapes.get(i) {
                Some(Shape::Path(path)) => Some(path),
                _ => None,
            })
        };
        let path = whole_path(r.parts, &t).or_else(first).ok_or_else(|| p.bad("none of the targets is a path or a node"))?;
        let mut attrs = ShapeAttrs::default();
        put(&mut attrs, &path.info.attrs, true);
        clip.path = Some(attrs);
    }
    let kind = if clip.node.is_some() { "node" } else { "path" };
    let text = serde_json::to_string(&clip).map_err(|e| p.bad(format!("the attributes cannot be written as text: {e}")))?;
    Ok(json!({"text": text, "kind": kind}))
}

/// The attributes in `text`, checked.
fn read(p: &Params, text: &str) -> Result<Clip> {
    if text.len() > MAX_ATTRIBUTES_TEXT {
        return Err(p.bad(format!("`text` is larger than {} KiB", MAX_ATTRIBUTES_TEXT >> 10)));
    }
    let none = || p.bad("`text` holds no copied attributes: copy them with Copy Attributes first");
    let header: Header = serde_json::from_str(text).map_err(|_| none())?;
    if header.format.as_deref() != Some(ATTRIBUTES_FORMAT) {
        return Err(none());
    }
    if header.version.is_none_or(|v| v > ATTRIBUTES_VERSION) {
        return Err(p.bad("the copied attributes are from a newer version"));
    }
    let clip: Clip = serde_json::from_str(text).map_err(|e| p.bad(format!("the copied attributes cannot be read: {e}")))?;
    match (&clip.path, &clip.node) {
        (Some(attrs), None) => {
            attrs.check().map_err(|m| p.bad(m))?;
            if let Some(style) = stroke_style(attrs).map_err(|m| p.bad(m))? {
                style.check().map_err(|e| p.bad(e.to_string()))?;
            }
        }
        (None, Some(node)) => {
            let bad = |n: &String| n.trim().is_empty() || n.len() > MAX_NAME_LEN || n.chars().any(char::is_control);
            if node.name.as_ref().is_some_and(bad) {
                return Err(p.bad(format!("the copied node name is empty, longer than {MAX_NAME_LEN} bytes or holds control characters")));
            }
        }
        _ => return Err(none()),
    }
    Ok(clip)
}

fn paste(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("edit.pasteAttributes", v)?;
    let t = targets(&p)?;
    let clip = read(&p, p.str("text")?)?;
    let changed = s.edit("Paste Attributes", |font| {
        t.edit_layer(font, |layer| {
            let color = is_full_color(layer);
            let mut parts = layer.parts_mut(t.side);
            let mut count = 0usize;
            if let Some(attrs) = &clip.path {
                let paths: BTreeSet<usize> = t.shapes.iter().copied().chain(t.points.iter().map(|&(i, _)| i)).collect();
                for i in paths {
                    let Some(path) = parts.path_mut(i) else { continue };
                    put(&mut path.info.attrs, attrs, color);
                    path.info.tidy();
                    count += 1;
                }
            }
            if let Some(node) = &clip.node {
                for &(i, k) in &t.points {
                    let Some(n) = parts.path_mut(i).and_then(|path| path.nodes.get_mut(k)).filter(|n| n.on_curve()) else { continue };
                    n.smooth = node.smooth;
                    n.tangent = n.tangent && node.smooth;
                    if node.name.is_some() && n.info.name != node.name {
                        n.info.name.clone_from(&node.name);
                    }
                    count += 1;
                }
            }
            if count == 0 {
                return Err(p.bad(if clip.node.is_some() { "none of `points` is an on-curve node" } else { "none of the targets is a path" }));
            }
            Ok(count)
        })
    })?;
    let kind = if clip.node.is_some() { "node" } else { "path" };
    Ok(json!({"glyph": t.glyph, "kind": kind, "changed": changed, "selection": t.to_json()}))
}
