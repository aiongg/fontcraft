//! Cap components, segment components and brushes fitted into outlines (M13.40 fills the caps,
//! M13.41 the segments and brushes; neither is called by outline resolution yet: M13.40 adds the
//! one hook in `font.rs` next to the corner components). Behaviour: `docs/spec/09-reusing-shapes.md`
//! § 8 to § 10; the fitting is provisional, from the rows' guesses, until the reference app's
//! exports are in (`docs/plans/M13.md` § 3 and § 8).
//!
//! Each is a hint on the layer, as a corner component is ([`crate::corner`]):
//! - a **cap** (`_cap.<name>`) sits on two neighbouring nodes of one path, the end of a stroke,
//!   with a scale, an alignment and a Fit flag that sizes it to the stroke's width. Its glyph
//!   carries an `origin` anchor, or `node1` and `node2` anchors for both stem nodes;
//! - a **segment component** (`_segment.<name>`, one open path from anchor `start` to `end`)
//!   replaces one segment of a host path, bent along it without adding nodes;
//! - a **brush** (`_brush.<name>`, one closed path, anchors `start` and `end`) is drawn along one
//!   segment: the outline between the anchors stretches and bends, the rest keeps its shape.
//!
//! Inserted nodes are rounded to whole units, as corner components are.

use fontcraft_geom::Vec2;

use crate::corner::CornerAlignment;
use crate::{Font, Hint, Layer, Shape};

/// The hint type of a cap component.
pub const CAP_HINT: &str = "Cap";
/// The hint type of a segment component.
pub const SEGMENT_HINT: &str = "Segment";
/// The hint type of a brush.
pub const BRUSH_HINT: &str = "Brush";
/// Largest cap scale, in either direction, as a factor.
pub const MAX_CAP_SCALE: f64 = 100.0;
/// Most node moves fitting the attachments of one layer may cost.
pub const MAX_ATTACH_WORK: usize = 50_000_000;

/// A cap component as its hint stores it.
#[derive(Clone, Debug, PartialEq)]
pub struct CapHint {
    /// The cap glyph (`_cap.<name>`).
    pub name: String,
    /// The path (shape index) and its first stem node; the second is the next node.
    pub shape: usize,
    pub node: usize,
    /// Horizontal and vertical scale as factors.
    pub scale: Vec2,
    pub alignment: CornerAlignment,
    /// Sized to the stroke's width (`capComponent.setFit`).
    pub fit: bool,
}

/// A segment component or a brush as its hint stores it.
#[derive(Clone, Debug, PartialEq)]
pub struct SegmentHint {
    /// The segment or brush glyph.
    pub name: String,
    /// The path (shape index) and the node the replaced segment ends at.
    pub shape: usize,
    pub node: usize,
}

/// One attachment on a layer.
#[derive(Clone, Debug, PartialEq)]
pub enum Attachment {
    Cap(CapHint),
    Segment(SegmentHint),
    Brush(SegmentHint),
}

impl Attachment {
    /// The hint that stores this attachment.
    pub fn to_hint(&self) -> Hint {
        // M13.40 / M13.41: the stored keys, confirmed against the reference's files.
        Hint { family: crate::Family::Glyphs, value: fontcraft_plist::Value::Dict(fontcraft_plist::Dict::new()) }
    }
}

/// Why an attachment could not be fitted.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("`{glyph}`: {message}")]
pub struct AttachError {
    /// The cap, segment or brush glyph.
    pub glyph: String,
    pub message: String,
}

/// The attachment `hint` stores; `Ok(None)` for a hint of another kind.
pub fn attachment(hint: &Hint) -> Result<Option<Attachment>, String> {
    let _ = hint;
    // M13.40 / M13.41.
    Ok(None)
}

/// `shapes` (a layer's shapes in master `master`) with `cap` fitted onto its two nodes.
pub fn fit_cap(font: &Font, master: &str, shapes: &[Shape], cap: &CapHint) -> Result<Vec<Shape>, AttachError> {
    let _ = (font, master);
    // M13.40.
    Err(AttachError { glyph: cap.name.clone(), message: format!("cap components are not supported yet ({} shapes)", shapes.len()) })
}

/// `shapes` with the segment `segment` replaced by the segment glyph, bent along it.
pub fn fit_segment(font: &Font, master: &str, shapes: &[Shape], segment: &SegmentHint) -> Result<Vec<Shape>, AttachError> {
    let _ = (font, master);
    // M13.41.
    Err(AttachError { glyph: segment.name.clone(), message: format!("segment components are not supported yet ({} shapes)", shapes.len()) })
}

/// `shapes` with the brush drawn along its segment.
pub fn fit_brush(font: &Font, master: &str, shapes: &[Shape], brush: &SegmentHint) -> Result<Vec<Shape>, AttachError> {
    let _ = (font, master);
    // M13.41.
    Err(AttachError { glyph: brush.name.clone(), message: format!("brushes are not supported yet ({} shapes)", shapes.len()) })
}

/// The shapes of `layer` with every attachment fitted in; `Ok(None)` when the layer has none.
/// The hook outline resolution will call (M13.40).
pub fn insert_attachments(font: &Font, layer: &Layer) -> Result<Option<Vec<Shape>>, AttachError> {
    let _ = (font, layer);
    // M13.40.
    Ok(None)
}

/// `layer` with the attachments at `indices` (positions in the layer's hints) fitted into its
/// paths and their hints removed (Decompose Cap, Decompose Segment Component, Decompose Brush).
pub fn decompose_attachments(font: &Font, layer: &Layer, indices: &[usize]) -> Result<Layer, AttachError> {
    let _ = (font, indices);
    // M13.40 / M13.41.
    Err(AttachError { glyph: String::new(), message: format!("decomposing attachments on layer `{}` is not supported yet", layer.id) })
}
