//! Variable fonts opened into axes, masters and intermediate layers (M9.14).
//!
//! Rows: `variable-ttf.open`, `cff2.open`.
//!
//! Contract:
//! - Axes come from `fvar` (tags, names, ranges, hidden flags) with their `avar` maps written as
//!   the Axis Mappings parameter; named instances become instances, STAT labels axis labels.
//! - One master per peak region that every varying glyph uses at a corner of the designspace;
//!   regions that only some glyphs use become intermediate layers of those glyphs. Each source's
//!   outlines and advances are the font's at that location (skrifa draws them, `HVAR` or phantom
//!   points give the advances). CFF2 blends are read through read-fonts.
//! - What is left out (hinting, `cvar`, layout variations beyond kerning) is listed in
//!   [`crate::Decompiled::warnings`].
//! - Counts read from the file are checked before allocation: at most [`MAX_NAMED_INSTANCES`]
//!   named instances, [`MAX_SOURCES`] masters and intermediate locations, and per glyph the tuple
//!   and point counts the tables state checked against the glyph itself.
//! - A static font passed here is opened as [`crate::decompile`] opens it.

use crate::{DecompileError, Decompiled};

/// Most named instances read from `fvar` (its 16-bit count).
pub const MAX_NAMED_INSTANCES: usize = 65_535;
/// Most masters and intermediate locations a variable font opens into.
pub const MAX_SOURCES: usize = 1_024;

/// Reads a variable font into a document with its axes, masters and intermediate layers. `face`
/// chooses a member of a collection.
pub fn open_variable(bytes: &[u8], face: Option<u32>) -> Result<Decompiled, DecompileError> {
    // M9.14
    let _ = (bytes, face);
    Err(DecompileError::NotYetImplemented("opening variable fonts"))
}
