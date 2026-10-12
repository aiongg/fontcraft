//! Stroked paths whose nodes carry their own widths (the Thickness tool, `thickness` row): the
//! step between a model path and `fontcraft_pathops::thickness`.
//!
//! A path where no node has its own width is stroked by the uniform stroker, as before; one
//! where some node has is stroked by the variable one, with the nodes' widths in the order of
//! the path's on-curve points ([`fontcraft_model::node_width`]).

use fontcraft_geom::BezPath;
use fontcraft_model::{Node, NodeKind, Path, node_width};
use fontcraft_pathops::thickness::{Widths, stroke_outline_widths};
use fontcraft_pathops::{StrokeStyle, stroke_outline};

/// The outline and stroke a path adds to a layer's composite: a path with per-node widths comes
/// back already stroked, with no stroke left to apply; any other path as it is, with `stroke`.
pub fn path_shape(path: &Path, stroke: Option<StrokeStyle>) -> Result<(Vec<BezPath>, Option<StrokeStyle>), String> {
    match stroke {
        Some(style) if node_width::has_any(path) => Ok((stroke_path(path, &style)?, None)),
        _ => Ok((vec![path.to_bez()?], stroke)),
    }
}

/// The stroke outline of `path` drawn with `style` and the nodes' own widths.
pub fn stroke_path(path: &Path, style: &StrokeStyle) -> Result<Vec<BezPath>, String> {
    let bez = path.to_bez()?;
    let result = if node_width::has_any(path) { stroke_outline_widths(&bez, style, &knot_widths(path)) } else { stroke_outline(&bez, style) };
    result.map_err(|e| e.to_string())
}

/// The nodes' own widths in the order `Path::to_bez` emits on-curve points: the first on-curve
/// node, then every later one, wrapping round a closed path. The points a run of quadratic
/// off-curve nodes implies between them have none, and a closed path's final return to its
/// first node takes that node's.
pub fn knot_widths(path: &Path) -> Vec<Option<Widths>> {
    let nodes = &path.nodes;
    let Some(first) = nodes.iter().position(Node::on_curve) else { return Vec::new() };
    let count = nodes.len();
    let mut out = vec![nodes.get(first).and_then(node_width::get)];
    let steps = if path.closed { count } else { count.saturating_sub(first + 1) };
    let mut pending = 0usize;
    for step in 1..=steps {
        let i = (first + step) % count;
        let Some(node) = nodes.get(i) else { continue };
        match node.kind {
            NodeKind::OffCurve => pending += 1,
            // The closing line back to the start is implied by the close.
            NodeKind::Line if path.closed && step == steps => {}
            NodeKind::Line | NodeKind::Curve => {
                out.push(node_width::get(node));
                pending = 0;
            }
            NodeKind::QCurve => {
                out.extend(std::iter::repeat_n(None, pending.saturating_sub(1)));
                out.push(node_width::get(node));
                pending = 0;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use fontcraft_geom::Shape as _;
    use fontcraft_model::Node;

    use super::*;

    #[test]
    fn widths_follow_the_on_curve_order_of_the_path() {
        let mut path = Path::rect(0.0, 0.0, 100.0, 100.0);
        node_width::set(&mut path.nodes[2], Some([5.0, 6.0])).unwrap();
        let knots = knot_widths(&path);
        let on_curves = path.to_bez().unwrap().elements().iter().filter(|e| !matches!(e, fontcraft_geom::PathEl::ClosePath)).count();
        assert_eq!(knots.len(), on_curves);
        assert_eq!(knots.iter().flatten().count(), 1);
        assert_eq!(knots.get(2).copied().flatten(), Some([5.0, 6.0]));
    }

    #[test]
    fn a_path_with_node_widths_is_stroked_by_them() {
        let mut path = Path::new(false, vec![Node::new(0.0, 0.0, NodeKind::Line), Node::new(100.0, 0.0, NodeKind::Line)]);
        let style = StrokeStyle::new(20.0);
        let (plain, kept) = path_shape(&path, Some(style)).unwrap();
        assert_eq!((plain.len(), kept), (1, Some(style)), "without node widths the stroke is applied later, as before");
        node_width::set(&mut path.nodes[1], Some([30.0, 0.0])).unwrap();
        let (outline, kept) = path_shape(&path, Some(style)).unwrap();
        assert_eq!(kept, None);
        let b = outline.iter().map(|p| p.bounding_box()).reduce(|a, b| a.union(b)).unwrap();
        assert!((b.y1 - 30.0).abs() < 1e-6 && (b.y0 + 10.0).abs() < 1e-6, "{b:?}");
    }
}
