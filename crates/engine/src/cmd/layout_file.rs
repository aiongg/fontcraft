//! Layout commands on compiled fonts and feature files: `layout.normalize`, `.listFeatures`, `.decompileFeatures`, `.convertSyntax`, `.compileFeatures`, `.lintFeatures` (M12.11–M12.14).
//!
//! Each package fills its own function below, so packages that share this module do not edit
//! the same lines.

use super::CommandSpec;

/// The commands of this module, in package order.
pub fn specs() -> Vec<CommandSpec> {
    let mut v = normalize_specs();
    v.extend(list_specs());
    v.extend(decompile_specs());
    v.extend(convert_specs());
    v.extend(compile_specs());
    v.extend(lint_specs());
    v
}

/// `layout.normalize` (M12.11).
fn normalize_specs() -> Vec<CommandSpec> {
    // M12.11
    Vec::new()
}

/// `layout.listFeatures` (M12.12).
fn list_specs() -> Vec<CommandSpec> {
    // M12.12
    Vec::new()
}

/// `layout.decompileFeatures` (M12.12).
fn decompile_specs() -> Vec<CommandSpec> {
    // M12.12
    Vec::new()
}

/// `layout.convertSyntax` (M12.13).
fn convert_specs() -> Vec<CommandSpec> {
    // M12.13
    Vec::new()
}

/// `layout.compileFeatures` (M12.14).
fn compile_specs() -> Vec<CommandSpec> {
    // M12.14
    Vec::new()
}

/// `layout.lintFeatures` (M12.14).
fn lint_specs() -> Vec<CommandSpec> {
    // M12.14
    Vec::new()
}
