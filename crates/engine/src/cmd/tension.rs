//! Curve tension and harmonizing (M13.25 fills it). Rows: `path.setCurveTension`,
//! `path.harmonizeNodes`, `path.harmonizeHandles`, `path.selectTunniLines`, `curveTension`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M13.25.
    Vec::new()
}
