//! FontCraft binary font tools (L3): everything the command line does to a compiled font.
//!
//! The crate reads an sfnt font into [`SfntFont`] (flavour, tables by tag, table order), changes
//! the tables a tool is about, and writes the font back with [`write`]. Contract shared by every
//! tool (rows `cli.global.sourceOrBinary`, `roundtrip.binaryPassThroughTables`):
//!
//! - Tables a tool does not touch are copied byte for byte; unknown tables pass through.
//! - Touched tables are read with read-fonts, changed through write-fonts' owned types and
//!   written again.
//! - [`write`] always recomputes the table directory, every checksum and
//!   `head.checkSumAdjustment`. The table order is the recommended one unless the tool asks to
//!   keep the input order ([`sfnt::TableOrder`]).
//! - Every input is untrusted: counts and sizes are capped by the limits in [`limits`], and no
//!   function here panics. A tool that fails returns [`TablesError`] and produces nothing.
//!
//! One module per tool, so the packages that build them own disjoint files. Each tool has an
//! options type, a result type and one entry function; until its package lands the entry
//! function returns [`TablesError::NotBuilt`].
//!
//! | Module | Tool | Package |
//! |---|---|---|
//! | [`sfnt`] | read and write, pass-through | M12.1a |
//! | [`edit`], [`show`] | table actions, `tables show` | M12.2 |
//! | [`collection`] | collections | M12.3 |
//! | [`ttx`] | TTX writer and reader | M12.4–M12.9 |
//! | [`text`] | text views and the normalised dump | M12.10 |
//! | [`normalize`] | layout normaliser | M12.11 |
//! | [`features`], [`fea_write`] | feature list, decompile | M12.12 |
//! | [`volt`], [`fontdame`] | feature syntax conversion | M12.13 |
//! | [`rename`], [`reorder`], [`goadb`] | glyph rename and reorder | M12.15 |
//! | [`metrics`] | metrics report | M12.16 |
//! | [`subset`] | subsetter | M12.17–M12.19 |
//! | [`instance`] | instancer | M12.20, M12.21 |
//! | [`merge`] | merge | M12.23 |
//! | [`transform`] | transform, units per em | M12.24 |
//! | [`optimize`] | optimize | M12.26 |
//! | [`dehint`] | hint removal | M12.27 |
//! | [`outlines`], [`curves`] | outline repairs, curve conversion | M12.28, M12.29 |
//! | [`convert`] | format conversion | M12.30 |
//! | [`diff`] | font diff | M12.32 |
//! | [`variations`], [`variations_model`] | variation utilities | M12.33, M12.34 |
//! | [`patch`] | table patches for audit fixes | M12.61 |
#![forbid(unsafe_code)]

pub mod collection;
pub mod convert;
pub mod curves;
pub mod dehint;
pub mod diff;
pub mod edit;
mod error;
pub mod fea_write;
pub mod features;
pub mod fontdame;
pub mod goadb;
pub mod instance;
mod layout_budget;
pub mod limits;
pub mod merge;
pub mod metrics;
pub mod normalize;
pub mod optimize;
pub mod outlines;
pub mod patch;
pub mod rename;
pub mod reorder;
pub mod sfnt;
pub mod show;
pub mod subset;
pub mod text;
pub mod transform;
pub mod ttx;
pub mod variations;
pub mod variations_model;
pub mod volt;

#[cfg(test)]
mod tests;

pub use error::TablesError;
pub use sfnt::{Flavour, SfntFont, TableOrder, Tag, WriteOptions, read, write};

/// Shorthand for this crate's results.
pub type Result<T, E = TablesError> = std::result::Result<T, E>;
