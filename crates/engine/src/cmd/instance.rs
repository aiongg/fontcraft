//! `font.instantiate` (M12.20–M12.22; `fontcraft_tables::instance`).
//!
//! Each package fills its own function below, so packages that share this module do not edit
//! the same lines.

use super::CommandSpec;

/// The commands of this module, in package order.
pub fn specs() -> Vec<CommandSpec> {
    let mut v = binary_specs();
    v.extend(source_specs());
    v
}

/// `font.instantiate` on variable fonts: full instances (M12.20), then partial ones (M12.21) (M12.20).
fn binary_specs() -> Vec<CommandSpec> {
    // M12.20
    Vec::new()
}

/// `font.instantiate` on designspace and multi-master sources (M12.22).
fn source_specs() -> Vec<CommandSpec> {
    // M12.22
    Vec::new()
}
