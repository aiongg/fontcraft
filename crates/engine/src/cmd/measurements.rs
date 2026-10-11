//! Auto-meter edits and stored measurements, on `fontcraft_pathops::stems` (M13.34 fills it). Rows:
//! `autometer.editStemNumerically`, `font.recalculateMeasurements`, and the engine queries behind
//! `view.showStemWidths`, `view.showCounterWidths`, `view.showCornerAngles`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M13.34.
    Vec::new()
}
