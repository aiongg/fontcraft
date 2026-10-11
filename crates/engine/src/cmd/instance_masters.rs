//! Instance and master commands (M9.18).
//!
//! Rows: `font.instance.addForEachMaster`, `font.instance.toMaster`,
//! `font.instance.addVariableFontSetting`, `font.master.addFromOtherFont`,
//! `file.generateInstances`, `open.fontsAsMasters`, `switching.renameGlyphsAtExport` (static
//! half). Generate Instances creates at most `fontcraft_interp::MAX_GENERATED_INSTANCES`
//! documents per run.

use super::*;

pub fn specs() -> Vec<CommandSpec> {
    Vec::new()
}
