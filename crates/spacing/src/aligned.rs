//! Metrics of layers under automatic alignment (M6.15).
//!
//! Rows: `metricsKey.autoAlignedDisplay`, `metricsKey.autoAlignedOffset`,
//! `components.autoAlignMetricsOffsetKeys`, `components.autoAlignMetricsFromBase`.
//!
//! A layer that holds only components, at least one of them aligned automatically, takes its
//! width and its left and right sidebearings from its base components. The alignment itself
//! lives in `fontcraft_components` (`align`, `metrics`), which this crate does not depend on;
//! [`derives_metrics`] repeats its rule for when a layer's width is derived, and an engine test
//! keeps the two in step.
//!
//! On such a layer the left, right and width keys do not set the metric:
//! - a left or right offset key (`=+18`, `=-12`) is applied by alignment itself, so it is in
//!   sync by construction;
//! - every other key in those three fields is ignored: [`crate::evaluate`] gives no value for it,
//!   the out-of-sync scan does not list it, and Update Metrics leaves the metric alone.
//!
//! The top and bottom sidebearings are not derived; their keys work as on any layer.

use fontcraft_model::{Alignment, Font, Layer, Shape};

use crate::Field;
use crate::keys::{Body, Key};

/// The font-wide Disable Automatic Alignment option is on.
pub fn alignment_off(font: &Font) -> bool {
    font.info.settings.disables_automatic_alignment == Some(true)
}

/// Whether `layer`'s width and left and right sidebearings come from automatic alignment: the
/// font option is off, the layer holds no path (nor a drawn shape kept raw), and at least one of
/// its components is not set to Disabled.
pub fn derives_metrics(font: &Font, layer: &Layer) -> bool {
    if alignment_off(font) {
        return false;
    }
    let mut aligned = false;
    for shape in &layer.shapes {
        match shape {
            Shape::Path(_) | Shape::Raw(_) => return false,
            Shape::Component(c) => aligned |= c.info.alignment != Alignment::Disabled,
            Shape::Image(_) => {}
        }
    }
    aligned
}

/// The fields automatic alignment derives: left, right and width.
pub fn is_derived_field(field: Field) -> bool {
    matches!(field, Field::Left | Field::Right | Field::Width)
}

/// Whether `field` of `layer` is set by automatic alignment rather than by a key or a number.
pub fn derived_field(font: &Font, layer: &Layer, field: Field) -> bool {
    is_derived_field(field) && derives_metrics(font, layer)
}

/// Whether `key` is an offset that alignment applies in `field`: `=+N` or `=-N` (layer-local or
/// glyph-wide) in the left or right field, without `@`.
pub fn is_offset(key: &Key, field: Field) -> bool {
    matches!(key.body, Body::Signed(_)) && key.at.is_none() && matches!(field, Field::Left | Field::Right)
}

/// The amount of an offset key; `None` for any other key.
pub fn offset_amount(key: &Key, field: Field) -> Option<f64> {
    match key.body {
        Body::Signed(v) if is_offset(key, field) => Some(v),
        _ => None,
    }
}
