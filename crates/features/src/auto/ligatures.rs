//! Ligatures, ordinals and ornaments (M8.11).
//!
//! Rows: `autoFeature.liga`, `dlig`, `hlig`, `rlig`, `afrc`, `ornm`, `ordn`.

use super::Generator;

/// This module's generators.
pub(super) fn generators() -> Vec<&'static dyn Generator> {
    // M8.11
    Vec::new()
}
