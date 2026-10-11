//! Colour palette commands: list, add and remove colours and palettes, set a colour. Removing a
//! colour renumbers the palette layers that use later rows in the same undo step and refuses while
//! a layer uses the row.
//!
//! Package M11.2 (`docs/plans/M11.md` § 4 › M11.2) fills [`specs`]; rows: `colorPalette.list`,
//! `colorPalette.addColor`, `colorPalette.removeColor`, `colorPalette.setColor`,
//! `colorPalette.addPalette`, `colorPalette.removePalette`.

use super::CommandSpec;

pub fn specs() -> Vec<CommandSpec> {
    // M11.2
    Vec::new()
}
