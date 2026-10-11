//! Path attribute commands of full-colour layers: stroke, fill, gradient and the image-only effects
//! of the given paths. Refused on other layers.
//!
//! Package M11.4 (`docs/plans/M11.md` § 4 › M11.4) fills [`specs`]; rows: the 13 `attributes.*`
//! commands and `attributes.removeGradient`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M11.4
    Vec::new()
}
