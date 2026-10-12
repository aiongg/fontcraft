//! Per-node stroke widths, the data the Thickness tool edits (`thickness` row,
//! `docs/plans/M13.md` § 4 › M13.38).
//!
//! A stroked path draws a band of the path's stroke width along it. An on-curve node may carry
//! its own width instead, as `[left, right]` under [`crate::fc_keys::NODE_WIDTH`] in its user
//! data: the distances, in font units, from the node to the band's left and right edges, looking
//! along the path direction. Between two nodes the band changes smoothly from one node's widths
//! to the next one's. A node without its own value takes the path's: the stroke width split by
//! the stroke position ([`path_sides`]).
//!
//! The stroke outline reads these values (`fontcraft_compile::thickness`); `path.setNodeWidths`
//! writes them.

use fontcraft_plist::{Array, Number, Value};

use crate::fc_keys::NODE_WIDTH;
use crate::{Dimension, Node, Path};

/// Widest distance from a node to one edge of its stroke, in font units (the widest stroke).
pub const MAX_NODE_WIDTH: f64 = 10_000.0;

/// Whether `w` can be one side of a node's width: a finite number from 0 to [`MAX_NODE_WIDTH`].
pub fn valid_side(w: f64) -> bool {
    w.is_finite() && (0.0..=MAX_NODE_WIDTH).contains(&w)
}

/// The node's own `[left, right]` widths, or `None` when it has none or the stored value is not
/// two usable numbers (such a value is ignored, as if absent).
pub fn get(node: &Node) -> Option<[f64; 2]> {
    let items = &node.info.user_data.get(NODE_WIDTH)?.as_array()?.items;
    let [l, r] = items.as_slice() else { return None };
    let (l, r) = (l.as_num()?.as_f64(), r.as_num()?.as_f64());
    (valid_side(l) && valid_side(r)).then_some([l, r])
}

/// Stores `widths` as the node's own value, or removes it with `None`. Values outside
/// [`valid_side`] are refused and change nothing.
pub fn set(node: &mut Node, widths: Option<[f64; 2]>) -> Result<(), String> {
    match widths {
        Some([l, r]) => {
            if !(valid_side(l) && valid_side(r)) {
                return Err(format!("a node's widths must be from 0 to {MAX_NODE_WIDTH} units, got [{l}, {r}]"));
            }
            let num = |v: f64| Number::float(v).map(Value::Number).ok_or_else(|| format!("the width {v} is not a finite number"));
            let value = Value::Array(Array { items: vec![num(l)?, num(r)?], ..Array::default() });
            node.info.user_data.0.insert(NODE_WIDTH, value);
        }
        None => {
            if node.info.user_data.get(NODE_WIDTH).is_none() {
                return Ok(());
            }
            let dict = &mut node.info.user_data.0;
            if let Some(i) = dict.entries.iter().position(|(k, _)| k.as_str() == NODE_WIDTH) {
                dict.remove_at(i);
            }
            node.info.tidy();
        }
    }
    Ok(())
}

/// Whether any node of `path` has its own widths.
pub fn has_any(path: &Path) -> bool {
    path.nodes.iter().any(|n| get(n).is_some())
}

/// The `[left, right]` widths of a stroke `width` units wide at `position` (−1 wholly left of the
/// path direction, 0 centred, 1 wholly right): what a node without its own value takes.
pub fn path_sides(width: f64, position: f64) -> [f64; 2] {
    let half = width / 2.0;
    let p = position.clamp(-1.0, 1.0);
    [half * (1.0 - p), half * (1.0 + p)]
}

/// The stroke width and position (−1 to 1) of `path` in a font of `.glyphs` format version
/// `glyphs_version`, or `None` when the path has no stroke (no width, a width of 0 or less, or
/// one that is not a plain number). Versions before 4 store the position as 0 right, 0.5
/// centred, 1 left; version 4 stores it as −1 left to 1 right. No stored position is centred.
pub fn path_stroke(path: &Path, glyphs_version: u32) -> Option<(f64, f64)> {
    let number = |d: &Option<Dimension>| match d {
        Some(Dimension::Number(n)) => Some(*n),
        Some(Dimension::Expression(e)) => e.trim().parse::<f64>().ok(),
        None => None,
    };
    let attrs = &path.info.attrs;
    let width = number(&attrs.stroke_width).filter(|w| w.is_finite() && *w > 0.0)?;
    let position = match number(&attrs.stroke_pos).filter(|p| p.is_finite()) {
        None => 0.0,
        Some(s) if glyphs_version < 4 => 1.0 - 2.0 * s,
        Some(p) => p,
    };
    Some((width, position.clamp(-1.0, 1.0)))
}

/// The widths `node` is drawn with on a path stroked `width` wide at `position`.
pub fn resolved(node: &Node, width: f64, position: f64) -> [f64; 2] {
    get(node).unwrap_or_else(|| path_sides(width, position))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeKind;

    #[test]
    fn node_widths_round_trip_through_user_data_and_clear_cleanly() {
        let mut node = Node::new(10.0, 20.0, NodeKind::Line);
        assert_eq!(get(&node), None);
        set(&mut node, Some([12.0, 30.5])).unwrap();
        assert_eq!(get(&node), Some([12.0, 30.5]));
        assert!(set(&mut node, Some([f64::NAN, 1.0])).is_err());
        assert!(set(&mut node, Some([-1.0, 1.0])).is_err());
        assert_eq!(get(&node), Some([12.0, 30.5]), "a refused value changes nothing");
        set(&mut node, None).unwrap();
        assert_eq!(get(&node), None);
        assert_eq!(node, Node::new(10.0, 20.0, NodeKind::Line), "clearing leaves no empty user data behind");
    }

    #[test]
    fn a_path_value_splits_by_position() {
        assert_eq!(path_sides(40.0, 0.0), [20.0, 20.0]);
        assert_eq!(path_sides(40.0, -1.0), [40.0, 0.0]);
        assert_eq!(path_sides(40.0, 0.5), [10.0, 30.0]);
    }
}
