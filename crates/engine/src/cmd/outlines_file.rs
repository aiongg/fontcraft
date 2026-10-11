//! Whole-font outline commands on sources and compiled fonts (M12.28, M12.29; `fontcraft_tables::outlines`, `fontcraft_tables::curves`). On a source each runs the document command on every glyph (one implementation per action).
//!
//! Each package fills its own function below, so packages that share this module do not edit
//! the same lines.

use super::CommandSpec;

/// The commands of this module, in package order.
pub fn specs() -> Vec<CommandSpec> {
    let mut v = repair_specs();
    v.extend(curve_specs());
    v
}

/// `outlines.removeOverlaps`, `.correctDirection`, `.addExtremes`, `.roundCoordinates`, `.decompose` (M12.28).
fn repair_specs() -> Vec<CommandSpec> {
    // M12.28
    Vec::new()
}

/// `outlines.convertCurves`, `outlines.componentize` (M12.29).
fn curve_specs() -> Vec<CommandSpec> {
    // M12.29
    Vec::new()
}
