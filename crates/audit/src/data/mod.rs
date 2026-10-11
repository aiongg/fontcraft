//! Data sets the checks read: registered OpenType tags, the OS/2 Unicode range and code page
//! bits, expected contour counts, registered vendor IDs, character lists, and Unicode
//! properties and production names from the glyph database. Each file names its source and
//! licence in its header; ported data also has a `NOTICE` entry.
//!
//! Lookups take code points as `u32` and tags as the four bytes stored in the font. They never
//! fail: an unknown value is simply not found.

pub mod chars;
pub mod code_pages;
pub mod contours;
pub mod glyphs;
pub mod tags;
pub mod unicode_ranges;
pub mod vendors;
