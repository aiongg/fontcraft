//! Auto-spacing, Center in Width and Copy Metrics, on `fontcraft_spacing::profile` (M13.20 fills
//! it). Rows: `spacing.autoSpace`, `spacing.centerInWidth`, `edit.copyMetrics`, and the `beam`
//! parameter of the sidebearing commands (`view.showMetricsBeam`, engine half).

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M13.20.
    Vec::new()
}
