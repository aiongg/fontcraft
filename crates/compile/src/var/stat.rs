//! The `STAT` table (M9.7).
//!
//! Rows: `stat.purpose`, `stat.elidableNames`, `stat.automaticFromInstances`,
//! `stat.manualOverrides`, `compile.statDefaults`, the `STAT` table row, parameters
//! `Export STAT Table`, `Elidable STAT Axis Value Name`, `Style Name as STAT entry`,
//! `Italic Style Linking`.
//!
//! Contract: [`stat_input`] derives the design axes and axis values as fontc's
//! `glyphs2fontir/src/stat.rs` does (Apache-2.0; ported with credit in this file's header and in
//! `NOTICE`): from the axes' labels when the font states them, else from the instances' names and
//! locations, with the elidable names and the italic linking the parameters set. [`build_stat`]
//! writes it with the shared name allocator. Axis value format 4 is never written. No table when
//! every variable font setting turns Export STAT Table off.

use fontcraft_interp::Designspace;
use fontcraft_model::Font;
use write_fonts::tables::stat::Stat;
use write_fonts::types::Tag;

use super::names::NameAllocator;
use crate::CompileError;

/// One axis value: a name for a value, a range of values, or a value linked to another.
#[derive(Clone, Debug, PartialEq)]
pub struct StatValue {
    pub name: String,
    /// External coordinate.
    pub value: f64,
    /// `(min, max)` for a range value (format 2).
    pub range: Option<(f64, f64)>,
    /// The value it links to (format 3).
    pub linked_value: Option<f64>,
    pub elidable: bool,
    pub older_sibling: bool,
}

/// One design axis and its values.
#[derive(Clone, Debug, PartialEq)]
pub struct StatAxis {
    pub tag: Tag,
    pub name: String,
    pub ordering: u16,
    pub values: Vec<StatValue>,
}

/// What `STAT` holds, before names get IDs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatInput {
    pub axes: Vec<StatAxis>,
    /// The name used when every axis value is elided (name ID 2 when `None`).
    pub elided_fallback_name: Option<String>,
}

/// The `STAT` contents of the variable font of `designspace`; `None` when the font turns the
/// table off.
pub fn stat_input(font: &Font, designspace: &Designspace) -> Result<Option<StatInput>, CompileError> {
    // M9.7
    let _ = (font, designspace);
    Err(CompileError::NotYetImplemented("the STAT derivation"))
}

/// The `STAT` table of `input`.
pub fn build_stat(input: &StatInput, names: &mut NameAllocator) -> Result<Stat, CompileError> {
    // M9.7
    let _ = (input, names);
    Err(CompileError::NotYetImplemented("the STAT table"))
}
