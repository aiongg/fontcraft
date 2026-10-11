//! Name records of a variable font: the names of axes and named instances, and the family,
//! style, PostScript and file names the variable font parameters set (M9.6).
//!
//! Rows: `compile.nameIdAllocation`, `instances.namedInstancesInVariable` (records), parameters
//! `Variable Font Family Name`, `Variable Font File Name`, `Variable Font Style Name`,
//! `Variable SubfamilyName`, `variablePostscriptFontName`, `variableStyleName`,
//! `variableStyleNames`, `variationsPostScriptNamePrefix`.
//!
//! Contract: name IDs for `fvar` and `STAT` are allocated from [`FIRST_FONT_NAME_ID`] upwards in
//! the order the builders ask, the same text getting the same ID, as fontc allocates them; the
//! static name records (IDs below 256) keep their IDs. At most 32,767 IDs are allocated.

use fontcraft_model::Font;
use write_fonts::types::NameId;

use crate::CompileError;

/// The first name ID free for font-specific names.
pub const FIRST_FONT_NAME_ID: u16 = 256;
/// The last name ID allocated (IDs above 32,767 are reserved).
pub const LAST_FONT_NAME_ID: u16 = 32_767;

pub const VARIABLE_FAMILY_NAME: &str = "Variable Font Family Name";
pub const VARIABLE_FILE_NAME: &str = "Variable Font File Name";
pub const VARIABLE_STYLE_NAME: &str = "Variable Font Style Name";
pub const VARIABLE_SUBFAMILY_NAME: &str = "Variable SubfamilyName";
pub const VARIABLE_POSTSCRIPT_NAME: &str = "variablePostscriptFontName";
pub const VARIATIONS_POSTSCRIPT_PREFIX: &str = "variationsPostScriptNamePrefix";

/// Hands out name IDs for the variable tables.
#[derive(Clone, Debug, PartialEq)]
pub struct NameAllocator {
    next: u16,
    records: Vec<(NameId, String)>,
}

impl Default for NameAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl NameAllocator {
    /// An allocator starting at [`FIRST_FONT_NAME_ID`].
    pub fn new() -> Self {
        Self { next: FIRST_FONT_NAME_ID, records: Vec::new() }
    }

    /// The ID of `text`: the one it already has, else the next free one. Fails past
    /// [`LAST_FONT_NAME_ID`].
    pub fn id_for(&mut self, text: &str) -> Result<NameId, CompileError> {
        // M9.6
        let _ = (text, &mut self.next);
        Err(CompileError::NotYetImplemented("name IDs for variable fonts"))
    }

    /// The records allocated so far, in ID order.
    pub fn records(&self) -> &[(NameId, String)] {
        &self.records
    }
}

/// The names of one variable font.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VariableNames {
    pub family: String,
    /// The default instance's style name.
    pub style: String,
    pub postscript_name: Option<String>,
    /// The prefix of the named instances' PostScript names.
    pub postscript_prefix: Option<String>,
    /// The file name without extension.
    pub file_stem: String,
}

/// The names of the variable font of `setting` (`None`: the default font), from the setting's
/// and the font's parameters.
pub fn variable_names(font: &Font, setting: Option<usize>) -> Result<VariableNames, CompileError> {
    // M9.6
    let _ = (font, setting);
    Err(CompileError::NotYetImplemented("variable font names"))
}
