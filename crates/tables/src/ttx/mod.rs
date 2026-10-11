//! TTX: the XML form of a font's tables, written and read as fontTools' `ttx` does (M12.4–M12.9;
//! rows `dump`, `font.dumpTables`, `tables import`, `tables.importTtx`, format `ttx`).
//!
//! [`dump`] walks the tables in file order and hands each to the writer module that knows it;
//! a table no module knows is written as hex, as `ttx` writes it. [`import`] parses the text into
//! an [`Element`] tree and hands each table element to the reader module that knows it.
//!
//! | Module | Tables | Package |
//! |---|---|---|
//! | [`core`] | `head hhea maxp OS/2 name post cmap hmtx loca glyf gasp vhea vmtx fpgm prep cvt hdmx` | M12.4 |
//! | [`cff`] | `CFF ` `CFF2` `VORG` | M12.5 |
//! | [`otdata`] | GSUB, GPOS, GDEF, BASE, JSTF, MATH, STAT, fvar, avar, meta and other structured tables | M12.6 |
//! | [`var`], [`color`], [`misc`] | variations; COLR, CPAL, SVG, sbix and bitmaps; kern, DSIG, the Apple tables | M12.7 |
//! | [`read_core`], [`read_cff`] | reading the tables of `core` and `cff` | M12.8 |
//! | [`read_otdata`], [`read_var`], [`read_color`] | reading the rest | M12.9 |
//!
//! Every writer module has `write_table(tag, data, font, w) -> Result<bool>` (false: not mine);
//! every reader module has `read_table(element, font) -> Result<Option<(Tag, Vec<u8>)>>`
//! (`None`: not mine). The JSON and hex forms of `dump` belong to M12.4.

pub mod cff;
pub mod color;
pub mod core;
pub mod misc;
pub mod otdata;
pub mod read_cff;
pub mod read_color;
pub mod read_core;
pub mod read_otdata;
pub mod read_var;
pub mod var;

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// The form [`dump`] writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DumpFormat {
    /// TTX XML.
    #[default]
    Ttx,
    /// The same tree as JSON.
    Json,
    /// Every table as hex.
    Hex,
}

/// What [`dump`] writes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DumpOptions {
    pub format: DumpFormat,
    /// Tables to write; empty writes every table.
    pub tables: Vec<Tag>,
    /// Tables to leave out.
    pub skip: Vec<Tag>,
    /// Tables written as hex even when a module knows them.
    pub hex: Vec<Tag>,
    /// Leave out the `ttLibVersion` attribute of the root, for stable comparisons.
    pub no_version: bool,
}

/// The font's tables as text.
pub fn dump(font: &SfntFont, options: &DumpOptions) -> Result<String> {
    // M12.4: the root element, file order, the per-module dispatch, hex fallback, JSON and hex.
    let _ = (font, options);
    Err(TablesError::NotBuilt("dump"))
}

/// How [`import`] merges the tables it reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportOptions {
    /// Tables to take from the text; empty takes every table it holds.
    pub tables: Vec<Tag>,
}

/// `font` with the tables of the TTX `text` added or replaced. `font` may be empty
/// ([`SfntFont::new`]) to build a font from text alone.
pub fn import(font: SfntFont, text: &str, options: &ImportOptions) -> Result<SfntFont> {
    // M12.8: parse within the TTX limits, then the per-module dispatch.
    let _ = (font, text, options);
    Err(TablesError::NotBuilt("tables import"))
}

/// One XML element of a TTX document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Element {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub children: Vec<Element>,
    /// The text directly inside the element, whitespace kept.
    pub text: String,
}

impl Element {
    /// The value of one attribute.
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    /// The first child with this name.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }
}

/// Parses TTX text into its root element within [`crate::limits::MAX_TTX_BYTES`],
/// [`crate::limits::MAX_TTX_DEPTH`] and [`crate::limits::MAX_TTX_ELEMENTS`].
pub fn parse(text: &str) -> Result<Element> {
    // M12.8
    let _ = text;
    Err(TablesError::NotBuilt("TTX reading"))
}

/// Builds TTX text: indentation of two spaces per level, attributes in the order given, text and
/// attribute values escaped.
#[derive(Clone, Debug, Default)]
pub struct XmlWriter {
    out: String,
    depth: usize,
}

impl XmlWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// `<name a="v" …>` and one level deeper.
    pub fn open(&mut self, name: &str, attributes: &[(&str, &str)]) {
        self.start(name, attributes);
        self.out.push_str(">\n");
        self.depth = self.depth.saturating_add(1);
    }

    /// `</name>` one level up.
    pub fn close(&mut self, name: &str) {
        self.depth = self.depth.saturating_sub(1);
        self.indent();
        self.out.push_str("</");
        self.out.push_str(name);
        self.out.push_str(">\n");
    }

    /// `<name a="v" …/>`.
    pub fn simple(&mut self, name: &str, attributes: &[(&str, &str)]) {
        self.start(name, attributes);
        self.out.push_str("/>\n");
    }

    /// A line of text at the current depth.
    pub fn text(&mut self, text: &str) {
        self.indent();
        escape_into(&mut self.out, text, false);
        self.out.push('\n');
    }

    /// `<!-- text -->` at the current depth.
    pub fn comment(&mut self, text: &str) {
        self.indent();
        self.out.push_str("<!-- ");
        self.out.push_str(&text.replace("--", "- -"));
        self.out.push_str(" -->\n");
    }

    /// The text written so far.
    pub fn finish(self) -> String {
        self.out
    }

    fn start(&mut self, name: &str, attributes: &[(&str, &str)]) {
        self.indent();
        self.out.push('<');
        self.out.push_str(name);
        for (key, value) in attributes {
            self.out.push(' ');
            self.out.push_str(key);
            self.out.push_str("=\"");
            escape_into(&mut self.out, value, true);
            self.out.push('"');
        }
    }

    fn indent(&mut self) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
    }
}

fn escape_into(out: &mut String, text: &str, attribute: bool) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}
