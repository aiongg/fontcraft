//! The node commands "in All Masters" (M3.52, `docs/spec/03a` § 11 to § 14): each is its one-layer
//! command on the layer whose addresses are given and, by [`super::matching`], on every layer of
//! the glyph that interpolates with it. They are the Alt alternates of the one-layer commands'
//! rows in the menus.

use serde_json::Value;

use super::super::{pathops, transform};
use super::*;

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.toggleNodeTypeInAllMasters",
            "Toggle Node Type in All Masters",
            [],
            None,
            "{glyph: name, layer?: id (whose addresses are given; default the first master's), side?, points: [[shape, node]], to?: \"smooth\" | \"corner\", keepOutline?: bool} path.toggleNodeType in that layer and, by the same addresses, in every master, intermediate and alternate layer of the glyph with the same shapes and node types, in one undo step; a master or intermediate layer that differs is left alone and listed in `skipped`, and an edit one of them cannot take, or that would leave one incompatible, is refused. Each node gets on every layer the type it gets on that one → {selection, matched: [{glyph, layer}], skipped: [{glyph, layer, reason}]}",
            has_doc,
            |s, v| super::toggle(s, v, "path.toggleNodeTypeInAllMasters", false)
        )
        .targets(&[K::Layer, K::Points])
        .context(&["canvas.node"]),
        cmd!(
            "path.convertSegmentToCurveInAllMasters",
            "Convert Segment to Curve in All Masters",
            [],
            None,
            "{glyph: name, layer?: id (whose addresses are given; default the first master's), side?, segments: [[shape, node]] (or segment: [shape, node]), orthogonal?: bool} path.convertSegmentToCurve in that layer and, by the same addresses, in every master, intermediate and alternate layer of the glyph with the same shapes and node types, in one undo step; a master or intermediate layer that differs is left alone and listed in `skipped`, and an edit one of them cannot take, or that would leave one incompatible, is refused. The curves are cubic or quadratic as they are on that layer → {selection, matched: [{glyph, layer}], skipped: [{glyph, layer, reason}]}",
            has_doc,
            |s, v| super::curve_segments(s, v, "path.convertSegmentToCurveInAllMasters")
        )
        .targets(&[K::Layer, K::Segments])
        .context(&["canvas.segment"]),
        cmd!(
            "path.insertNodeInAllMasters",
            "Insert Node in All Masters",
            [],
            None,
            "{glyph: name, layer?: id (whose addresses are given; default the first master's), side?, segment: [shape, node], t?: number strictly between 0 and 1 (default 0.5), near?: [x, y], snap?: \"extremum\" | \"inflection\"} path.insertNode in that layer and, by the same addresses, in every master, intermediate and alternate layer of the glyph with the same shapes and node types, in one undo step; a master or intermediate layer that differs is left alone and listed in `skipped`, and an edit one of them cannot take, or that would leave one incompatible, is refused. The segment is split at the same t on every layer → {node: [shape, node], selection, matched: [{glyph, layer}], skipped: [{glyph, layer, reason}]}",
            has_doc,
            |s, v| super::split_segment(s, v, "path.insertNodeInAllMasters")
        )
        .targets(&[K::Layer, K::Segments])
        .context(&["canvas.segment"]),
        cmd!(
            "path.connectNodesInAllMasters",
            "Connect Nodes in All Masters",
            [],
            None,
            "{glyph: name, layer?: id (whose addresses are given; default the first master's), side?, points: [[shape, node], [shape, node]]} path.connectNodes in that layer and, by the same addresses, in every master, intermediate and alternate layer of the glyph with the same shapes and node types, in one undo step; a master or intermediate layer that differs is left alone and listed in `skipped`, and an edit one of them cannot take, or that would leave one incompatible, is refused. The ends become one node where they lie on each other on every layer; otherwise a straight segment joins them on every layer → {selection, matched: [{glyph, layer}], skipped: [{glyph, layer, reason}]}",
            has_doc,
            |s, v| super::connect(s, v, "path.connectNodesInAllMasters")
        )
        .targets(&[K::Layer, K::Points])
        .context(&["canvas.node"]),
        cmd!(
            "path.closeOpenPathsInAllMasters",
            "Close Open Paths in All Masters",
            [],
            None,
            "{glyph: name, layer?: id (whose addresses are given; default the first master's), side?, shapes?: [index], points?: [[shape, node]] (their paths)} path.closeOpenPaths in that layer and, by the same addresses, in every master, intermediate and alternate layer of the glyph with the same shapes and node types, in one undo step; a master or intermediate layer that differs is left alone and listed in `skipped`, and an edit one of them cannot take, or that would leave one incompatible, is refused. The ends of a path become one node where they lie on each other on every layer; otherwise a straight segment joins them on every layer → {selection, matched: [{glyph, layer}], skipped: [{glyph, layer, reason}]}",
            has_doc,
            |s, v| super::close_paths(s, v, "path.closeOpenPathsInAllMasters")
        )
        .targets(&[K::Layer, K::Shapes, K::Points])
        .context(&["canvas.node", "canvas.path"]),
        cmd!(
            "path.openCornerInAllMasters",
            "Open Corner in All Masters",
            [],
            None,
            "{glyph: name, layer?: id (whose addresses are given; default the first master's), side?, points?: [[shape, node]] (corner nodes), segments?: [[shape, node]] (both end nodes), overlap?: number | [x, y] (default from each layer's master, as path.openCorner)} path.openCorner in that layer and, by the same addresses, in every master, intermediate and alternate layer of the glyph with the same shapes and node types, in one undo step; a master or intermediate layer that differs is left alone and listed in `skipped`, and an edit one of them cannot take, or that would leave one incompatible, is refused. The corners that open on that layer open on every layer → {selection, matched: [{glyph, layer}], skipped: [{glyph, layer, reason}]}",
            has_doc,
            |s, v| pathops::open_corner_as(s, v, "path.openCornerInAllMasters")
        )
        .targets(&[K::Layer, K::Points, K::Segments])
        .context(&["canvas.node", "canvas.segment"]),
        cmd!(
            "path.reconnectNodesInAllMasters",
            "Reconnect Nodes in All Masters",
            [],
            None,
            "{glyph: name, layer?: id (whose addresses are given; default the first master's), side?, points: [[shape, node]] (an even number of on-curve nodes)} path.reconnectNodes in that layer and, by the same addresses, in every master, intermediate and alternate layer of the glyph with the same shapes and node types, in one undo step; a master or intermediate layer that differs is left alone and listed in `skipped`, and an edit one of them cannot take, or that would leave one incompatible, is refused. The nodes pair up on every layer as they do on that one → {selection, matched: [{glyph, layer}], skipped: [{glyph, layer, reason}]}",
            has_doc,
            |s, v| pathops::reconnect_nodes_as(s, v, "path.reconnectNodesInAllMasters")
        )
        .targets(&[K::Layer, K::Points])
        .context(&["canvas.node"]),
        cmd!(
            "path.makeNodeFirstInAllMasters",
            "Make Node First in All Masters",
            [],
            None,
            "{glyph: name, layer?: id (whose addresses are given; default the first master's), side?, point: [shape, node]} path.makeNodeFirst in that layer and, by the same addresses, in every master, intermediate and alternate layer of the glyph with the same shapes and node types, in one undo step; a master or intermediate layer that differs is left alone and listed in `skipped`, and an edit one of them cannot take, or that would leave one incompatible, is refused → {selection, matched: [{glyph, layer}], skipped: [{glyph, layer, reason}]}",
            has_doc,
            |s, v| super::start_at(s, v, "path.makeNodeFirstInAllMasters")
        )
        .targets(&[K::Layer, K::Points])
        .context(&["canvas.node"]),
        cmd!(
            "path.alignSelectionInAllMasters",
            "Align Selection in All Masters",
            ["Path"],
            None,
            "{glyph: name, layer?: id (whose addresses are given; default the first master's), side?, points?: [[shape, node]], segments?: [[shape, node]], shapes?: [index] (paths, or one component), anchors?: [name], axis?: \"auto\" (default) | \"vertical\" | \"horizontal\", origin?: as path.alignSelection} path.alignSelection in that layer and, by the same addresses, in every master, intermediate and alternate layer of the glyph with the same shapes and node types, in one undo step; a master or intermediate layer that differs is left alone and listed in `skipped`, and an edit one of them cannot take, or that would leave one incompatible, is refused. Every layer lines up along the axis that layer does, on a coordinate of its own; a layer without one of the anchors is left alone and listed → {glyph, moved: count (on that layer), axis?, coordinate?, anchor?, x?, component?, origin?, message?, matched: [{glyph, layer}], skipped: [{glyph, layer, reason}]}",
            has_doc,
            |s, v| transform::align_selection_as(s, v, "path.alignSelectionInAllMasters")
        )
        .targets(&[K::Layer, K::Points, K::Segments, K::Shapes, K::Anchors]),
        cmd!(
            "path.cutWithLineInAllMasters",
            "Cut with Line in All Masters",
            [],
            None,
            "{glyph: name, layer?: id (whose addresses are given; default the first master's), side?, from: [x, y], to: [x, y], shapes?: [index] (default every path)} path.cutWithLine in that layer and, by the same addresses, in every master, intermediate and alternate layer of the glyph with the same shapes and node types, in one undo step; a master or intermediate layer that differs is left alone and listed in `skipped`, and an edit one of them cannot take, or that would leave one incompatible, is refused. Every layer is cut at the places on its paths where the line cuts that layer: the same segments at the same t → {selection, matched: [{glyph, layer}], skipped: [{glyph, layer, reason}]}",
            has_doc,
            |s, v| pathops::cut_with_line_as(s, v, "path.cutWithLineInAllMasters")
        )
        .targets(&[K::Layer, K::Shapes]),
    ]
}

/// `edit.deleteInAllMasters`.
pub(super) fn delete(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("edit.deleteInAllMasters", v)?;
    let t = Targets::parse(&p)?;
    let other = !(t.segments.is_empty() && t.anchors.is_empty() && t.guides.is_empty() && t.annotations.is_empty());
    if other || p.get("hints").is_some() || p.get("image").is_some() {
        return Err(p.bad("takes only `points` and `shapes`"));
    }
    if t.points.is_empty() && t.shapes.is_empty() {
        return Err(p.bad("give what to delete: `points` or `shapes`"));
    }
    remove_targets(s, v, "edit.deleteInAllMasters")
}

/// `path.deleteAndBreakInAllMasters`.
pub(super) fn delete_and_break(s: &mut Session, v: &Value) -> Result<Value> {
    break_at(s, v, "path.deleteAndBreakInAllMasters")
}
