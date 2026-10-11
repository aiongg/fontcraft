//! Variation utilities (M12.33, M12.34; `fontcraft_tables::variations`, `fontcraft_tables::variations_model`).
//!
//! Each package fills its own function below, so packages that share this module do not edit
//! the same lines.

use super::CommandSpec;

/// The commands of this module, in package order.
pub fn specs() -> Vec<CommandSpec> {
    let mut v = map_specs();
    v.extend(model_specs());
    v
}

/// `variations.mapCoordinates`, `variations.buildAvar` (M12.33).
fn map_specs() -> Vec<CommandSpec> {
    // M12.33
    Vec::new()
}

/// `variations.buildHvar`, `.describeModel`, `.plot`, `.planAvar` (M12.34).
fn model_specs() -> Vec<CommandSpec> {
    // M12.34
    Vec::new()
}
