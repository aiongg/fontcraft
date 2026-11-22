//! Components attached to a path through a layer hint (corner components now; caps, segments and
//! brushes share the code), as the Edit View draws and selects them.
//!
//! Each attachment is fitted into its host path on its own; what fitting put in is its band (the
//! highlight the canvas draws), and the stretch between two inserted nodes is what a click hits
//! (`corner.highlightAndSelection`). A selected attachment is [`Item::Hint`] with its position in
//! the layer's hint list, the address `edit.delete`, `edit.copy` (`corners`) and the corner
//! commands take.

use fontcraft_geom::{BezPath, ParamCurve, ParamCurveExtrema, ParamCurveNearest, Point};
use fontcraft_model::attach::{Attachment, attachment, fit_cap};
use fontcraft_model::corner::{CornerTarget, corner_hint, insert_corners, is_corner};
use fontcraft_model::{Font, Hint, Layer, Path, Shape};

use crate::geometry::finite;
use crate::scene::Scene;
use crate::selection::{Item, Selection};

/// Most attachments one layer fits for drawing and hit testing: each fits its glyph once.
pub const MAX_ATTACHMENTS: usize = 64;
/// Most nodes a fitted path may have for its band to be worked out.
const MAX_BAND_NODES: usize = 4096;
/// Two node positions closer than this, in font units, are the same node.
const SAME_NODE: f64 = 0.5;
/// How closely the nearest point of a curve is searched for, in font units.
const NEAREST_ACCURACY: f64 = 1e-3;

/// What an attachment hint attaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentKind {
    Corner,
    Cap,
}

/// The kind of attachment `hint` is; `None` for any other hint (PostScript and TrueType hints).
pub fn kind(hint: &Hint) -> Option<AttachmentKind> {
    if is_corner(hint) {
        return Some(AttachmentKind::Corner);
    }
    match attachment(hint) {
        Ok(Some(Attachment::Cap(_))) => Some(AttachmentKind::Cap),
        _ => None,
    }
}

/// Whether the Edit View selects attachments of `kind` by a click. Caps, segments and brushes
/// join with their own Info box and clipboard support.
pub fn selectable_kind(kind: AttachmentKind) -> bool {
    match kind {
        AttachmentKind::Corner => true,
        AttachmentKind::Cap => false,
    }
}

/// Whether hint `index` of `layer` is an attachment the Edit View selects.
pub fn is_selectable(layer: &Layer, index: usize) -> bool {
    layer.info.hints.get(index).and_then(kind).is_some_and(selectable_kind)
}

/// One fitted attachment of a layer.
#[derive(Clone, Debug, PartialEq)]
pub struct AttachmentMark {
    /// The hint's position in the layer's hint list.
    pub hint: usize,
    pub kind: AttachmentKind,
    /// The host path's shape index.
    pub shape: usize,
    /// The segments of the fitted host path that touch an inserted node, in font units.
    pub band: BezPath,
    /// The segments between two inserted nodes: the attachment's own outline, which a click hits.
    pub outline: BezPath,
}

/// The fitted attachments of `layer` whose kind `wanted` accepts, in hint order, at most
/// [`MAX_ATTACHMENTS`]. An attachment that does not fit (its glyph is missing, it sits on an
/// extra node) has no mark.
pub fn marks(font: &Font, layer: &Layer, wanted: impl Fn(AttachmentKind) -> bool) -> Vec<AttachmentMark> {
    let mut out = Vec::new();
    for (i, hint) in layer.info.hints.iter().enumerate() {
        if out.len() >= MAX_ATTACHMENTS {
            break;
        }
        let Some(k) = kind(hint).filter(|k| wanted(*k)) else { continue };
        let fitted = match k {
            AttachmentKind::Corner => corner_hint(hint).ok().flatten().and_then(|corner| match corner.target {
                CornerTarget::Node { shape, .. } => insert_corners(font, &with_one_hint(layer, i)).ok().flatten().map(|shapes| (shape, shapes)),
                CornerTarget::Intersection { .. } => None,
            }),
            AttachmentKind::Cap => match attachment(hint) {
                Ok(Some(Attachment::Cap(cap))) => fit_cap(font, &layer.master, &layer.shapes, &cap).ok().map(|shapes| (cap.shape, shapes)),
                _ => None,
            },
        };
        let Some((shape, shapes)) = fitted else { continue };
        if let (Some(Shape::Path(before)), Some(Shape::Path(after))) = (layer.shapes.get(shape), shapes.get(shape))
            && let Some((band, outline)) = inserted(before, after)
        {
            out.push(AttachmentMark { hint: i, kind: k, shape, band, outline });
        }
    }
    out
}

/// `layer` with only the hint at `index`.
fn with_one_hint(layer: &Layer, index: usize) -> Layer {
    let mut one = layer.clone();
    one.info.hints = layer.info.hints.get(index).cloned().into_iter().collect();
    one
}

/// What fitting an attachment into `before` put into `after`: the segments touching a node
/// `before` lacks, and those of them whose two end nodes are both new. `None` when nothing was
/// put in.
fn inserted(before: &Path, after: &Path) -> Option<(BezPath, BezPath)> {
    if after.nodes.len() > MAX_BAND_NODES || before.nodes.len() > MAX_BAND_NODES {
        return None;
    }
    let fresh: Vec<bool> =
        after.nodes.iter().map(|n| !before.nodes.iter().any(|o| (o.x - n.x).abs() < SAME_NODE && (o.y - n.y).abs() < SAME_NODE)).collect();
    let is_fresh = |i: usize| fresh.get(i).copied().unwrap_or(false);
    let node = |i: usize| after.nodes.get(i).map(|n| Point::new(n.x, n.y));
    let mut band = BezPath::new();
    let mut outline = BezPath::new();
    let (mut last_band, mut last_outline): (Option<usize>, Option<usize>) = (None, None);
    for seg in after.segments() {
        let touches = std::iter::once(seg.start).chain(seg.handles.iter().copied()).chain(std::iter::once(seg.end));
        if !touches.into_iter().any(is_fresh) {
            continue;
        }
        let own = is_fresh(seg.start) && is_fresh(seg.end);
        let handles = seg.handles.as_slice();
        append(&mut band, &mut last_band, seg.start, handles, seg.end, &node)?;
        if own {
            append(&mut outline, &mut last_outline, seg.start, handles, seg.end, &node)?;
        }
    }
    if band.elements().is_empty() {
        return None;
    }
    // A corner whose inserted run has no segment of its own is hit along its whole band.
    if outline.elements().is_empty() {
        outline = band.clone();
    }
    Some((band, outline))
}

/// Adds the segment from node `start` through `handles` to node `end` to `path`, starting a new
/// subpath unless the last segment added ended at `start`.
fn append(
    path: &mut BezPath,
    last: &mut Option<usize>,
    start: usize,
    handles: &[usize],
    end: usize,
    node: &impl Fn(usize) -> Option<Point>,
) -> Option<()> {
    let (from, to) = (node(start)?, node(end)?);
    if *last != Some(start) {
        path.move_to(from);
    }
    match handles {
        [h] => path.quad_to(node(*h)?, to),
        [h1, h2] => path.curve_to(node(*h1)?, node(*h2)?, to),
        _ => path.line_to(to),
    }
    *last = Some(end);
    Some(())
}

/// The selectable attachment of `scene` whose own outline passes nearest `pos`, within `tol`
/// font units: its hint index, the distance and the nearest point.
pub fn nearest(scene: &Scene, pos: Point, tol: f64) -> Option<(usize, f64, Point)> {
    if !finite(pos) || tol.is_nan() {
        return None;
    }
    let mut best: Option<(usize, f64, Point)> = None;
    for m in scene.attachments.iter().filter(|m| selectable_kind(m.kind) && scene.shape_editable(m.shape)) {
        for seg in m.outline.segments() {
            if !seg.bounding_box().inflate(tol, tol).contains(pos) {
                continue;
            }
            let near = seg.nearest(pos, NEAREST_ACCURACY);
            let d = near.distance_sq.sqrt();
            if d.is_finite() && d <= tol && best.is_none_or(|(_, bd, _)| d < bd) {
                best = Some((m.hint, d, seg.eval(near.t)));
            }
        }
    }
    best
}

/// The selected hints that are selectable attachments of `kind` in `scene`'s layer, in hint
/// order.
pub fn selected(scene: &Scene, selection: &Selection, kind: AttachmentKind) -> Vec<usize> {
    let hints = &scene.layer.info.hints;
    selection.hints.iter().copied().filter(|&i| hints.get(i).and_then(self::kind) == Some(kind)).collect()
}

/// The selected corner components, by hint index.
pub fn selected_corners(scene: &Scene, selection: &Selection) -> Vec<usize> {
    selected(scene, selection, AttachmentKind::Corner)
}

/// The selection items of the selected attachments, for keeping them through a change that
/// rebuilds the rest of the selection.
pub fn items(scene: &Scene, selection: &Selection) -> Vec<Item> {
    selection.hints.iter().copied().filter(|&i| is_selectable(&scene.layer, i)).map(Item::Hint).collect()
}
