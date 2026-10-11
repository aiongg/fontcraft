//! General fields, settings, the note and the row clipboard: `font.info.listFields`, `.addField`, `.removeField`, `.setLocalizedValue`, `.removeLocalizedValue`, `.copyRows`, `.pasteRows`, `.stepVersion`, `.updateDate`, `font.settings.get/set`, `font.note.get/set`. `font.info.get/set` stay in `font.rs`.
//!
//! Package M7.3 (`docs/plans/M7.md` § 4) fills [`specs`].

use super::*;

pub fn specs() -> Vec<CommandSpec> {
    // M7.3: the commands.
    Vec::new()
}
