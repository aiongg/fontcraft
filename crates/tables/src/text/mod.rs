//! Text views of a font and the normalised dump (M12.10; row `dump` with its text views,
//! `@font`, `@glyphs` and `--normalize`). Oracles: AFDKO spot and `tx -dump`, ttxn.
//!
//! | View | Module | Oracle |
//! |---|---|---|
//! | the normalised dump ([`ViewOptions::normalize`]) | [`ttxn`], [`ttxn_otl`] | `ttxn` |
//! | `@font`, `@glyphs` | [`txdump`] | `tx -dump` |
//! | `GPOS=normalized`, `GDEF=normalized` | [`crate::normalize`] | otl-normalizer |
//! | `GSUB=fea`, `GPOS=fea`, `GDEF=fea` | [`crate::fea_write`] | feaLib round trip |
//! | the spot listings (`cmap=summary`, `name=lines`, `GPOS=features`…) | [`spot`] | AFDKO spot |
//! | `CFF=literal` | [`cff_literal`] | `tx -dcf` |
//!
//! Not built, and refused with [`TablesError::NotBuilt`]: `raw` for tables other than `name`,
//! and the listings `BASE=list`, `BASE=friendly`, `GSUB=names`, `GPOS=pairs`, `kern=headers`,
//! `kern=indices`, `kern=names` and `avar=designspace`.
//!
//! Every function here reads untrusted bytes: counts are capped by the limits of
//! [`crate::limits`], the text by [`MAX_TEXT_OUTPUT`], and nothing panics.

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

pub mod cff_literal;
mod spot;
mod spot_desc;
pub mod ttxn;
pub mod ttxn_otl;
pub mod txdump;

pub use cff_literal::CffLevel;

#[cfg(test)]
mod tests;

/// Largest text one view produces, in bytes (the limit of the TTX dump).
pub const MAX_TEXT_OUTPUT: usize = crate::limits::MAX_TTX_OUTPUT;

/// The pseudo-table of the font program's global data.
pub const FONT_VIEW: &str = "@font";
/// The pseudo-table of the per-glyph data.
pub const GLYPHS_VIEW: &str = "@glyphs";

/// How much `@glyphs` writes per glyph (`tx -dump` levels 4, 5 and 6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlyphDetail {
    /// Index, name and encoding.
    #[default]
    Short,
    /// Also the path with hints, wrapped into lines of at most 78 characters.
    Medium,
    /// Also the path with hints, one operator per line.
    Long,
}

/// What [`view`] prints.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewOptions {
    /// The view, by the name `docs/spec/20-cli.md` › dump gives it, or [`FONT_VIEW`] or
    /// [`GLYPHS_VIEW`]. Empty with `normalize` is the normalised dump.
    pub view: String,
    /// Tables the view covers; empty covers every table the view knows.
    pub tables: Vec<Tag>,
    /// Glyphs the view covers (`@glyphs`, `glyf=points`); empty covers every glyph.
    pub glyphs: Vec<String>,
    /// Glyphs `@glyphs` leaves out (`tx -gx`).
    pub exclude_glyphs: Vec<String>,
    /// Print the normalised dump (ttxn's form): glyph ids as names, offsets and counts left out,
    /// lookups flattened.
    pub normalize: bool,
    /// With `normalize`: blank `head.fontRevision` and the version name records (ttxn `-nv`).
    pub no_version: bool,
    /// Leave stem hints out of outline dumps (ttxn `-nh`, `tx -dump -n`).
    pub no_hints: bool,
    /// With `normalize`: say which lookups are stored through Extension lookups (ttxn `-se`).
    pub show_extensions: bool,
    /// What `@glyphs` writes per glyph.
    pub glyph_detail: GlyphDetail,
    /// The cmap encoding record the mapping listings write (spot `-C`); `None` writes all.
    pub cmap_subtable: Option<usize>,
    /// Scale the `hmtx` listings to 1000 units per em.
    pub units_1000: bool,
    /// Where each table starts in the font file, for the byte ranges and headers the listings
    /// print; a table not listed starts at 0.
    pub table_offsets: Vec<(Tag, u32)>,
    /// The structures of the CFF literal view, by their `tx -dcf -T` letters; empty for all.
    pub cff_parts: String,
    /// How much the CFF literal view writes.
    pub cff_level: CffLevel,
    /// Keep the flowed data of the CFF literal view on one line (`tx -dcf -b`).
    pub no_wrap: bool,
}

/// The view as text.
pub fn view(font: &SfntFont, options: &ViewOptions) -> Result<String> {
    if options.normalize {
        if !options.view.is_empty() && options.view != "raw" {
            return Err(TablesError::BadOptions(format!("the normalised dump has no views: `{}` cannot be combined with normalize", options.view)));
        }
        return ttxn::dump(font, options);
    }
    match options.view.as_str() {
        FONT_VIEW => txdump::font_dict(font),
        GLYPHS_VIEW => txdump::glyphs(font, &options.glyphs, &options.exclude_glyphs, options.glyph_detail, !options.no_hints),
        "normalized" => {
            check_tables(options, &[*b"GPOS", *b"GDEF"], "normalized")?;
            let opts = crate::normalize::NormalizeOptions { tables: options.tables.clone(), location: Vec::new() };
            crate::normalize::normalize(font, &opts)
        }
        "fea" => {
            check_tables(options, &[*b"GSUB", *b"GPOS", *b"GDEF"], "fea")?;
            let opts = crate::fea_write::DecompileOptions {
                tables: options.tables.clone(),
                named_classes: true,
                show_extensions: options.show_extensions,
                ..Default::default()
            };
            crate::fea_write::decompile(font, &opts)
        }
        "literal" => {
            check_tables(options, &[*b"CFF "], "literal")?;
            let data = font.get(*b"CFF ").ok_or_else(|| TablesError::MissingTable("CFF ".into()))?;
            if !options.glyphs.is_empty() {
                return Err(TablesError::NotBuilt("the flattened charstrings of selected glyphs in the CFF literal view"));
            }
            let base = options.table_offsets.iter().find(|(t, _)| t == b"CFF ").map_or(0, |(_, o)| *o);
            cff_literal::dump(data, base, &options.cff_parts, options.cff_level, !options.no_wrap)
        }
        name => {
            let [tag] = options.tables.as_slice() else {
                return Err(TablesError::BadOptions(format!("the `{name}` view needs one table")));
            };
            spot::listing(font, *tag, name, options)?.ok_or(TablesError::NotBuilt("this table view"))
        }
    }
}

/// Where each table starts in a font file (an sfnt or a face of a collection), for
/// [`ViewOptions::table_offsets`]; empty for other files.
pub fn table_offsets(bytes: &[u8], face: u32) -> Vec<(Tag, u32)> {
    let rd = |at: usize| -> Option<u32> { Some(u32::from_be_bytes(bytes.get(at..at.checked_add(4)?)?.try_into().ok()?)) };
    let start = if bytes.get(..4) == Some(b"ttcf") {
        let Some(n) = rd(8) else { return Vec::new() };
        if face >= n {
            return Vec::new();
        }
        let Some(at) = rd(12 + 4 * face as usize) else { return Vec::new() };
        at as usize
    } else {
        0
    };
    if !matches!(rd(start), Some(0x0001_0000 | 0x4F54_544F | 0x7472_7565 | 0x7479_7031)) {
        return Vec::new();
    }
    let Some(count) = rd(start + 4).map(|v| (v >> 16) as usize) else { return Vec::new() };
    let mut out = Vec::with_capacity(count.min(1024));
    for i in 0..count {
        let rec = start + 12 + 16 * i;
        let (Some(tag), Some(offset)) = (bytes.get(rec..rec + 4), rd(rec + 8)) else { break };
        let Ok(tag) = <[u8; 4]>::try_from(tag) else { break };
        out.push((tag, offset));
    }
    out
}

/// Refuses a view asked for a table it does not cover.
fn check_tables(options: &ViewOptions, allowed: &[Tag], view: &str) -> Result<()> {
    for tag in &options.tables {
        if !allowed.contains(tag) {
            let names: Vec<String> = allowed.iter().map(|t| crate::sfnt::tag_str(*t).trim_end().to_string()).collect();
            return Err(TablesError::BadOptions(format!(
                "the `{view}` view covers {}; '{}' has no such view",
                names.join(", "),
                crate::sfnt::tag_str(*tag).trim_end()
            )));
        }
    }
    Ok(())
}

/// Text that refuses to grow past [`MAX_TEXT_OUTPUT`].
pub(crate) struct Out {
    pub(crate) text: String,
    limit: usize,
}

impl Out {
    pub(crate) fn new() -> Self {
        Self { text: String::new(), limit: MAX_TEXT_OUTPUT }
    }

    #[cfg(test)]
    pub(crate) fn with_limit(limit: usize) -> Self {
        Self { text: String::new(), limit }
    }

    pub(crate) fn push(&mut self, s: &str) -> Result<()> {
        if self.text.len().saturating_add(s.len()) > self.limit {
            return Err(TablesError::TooLarge { what: "the text view", limit: self.limit });
        }
        self.text.push_str(s);
        Ok(())
    }

    /// `s` and a newline.
    pub(crate) fn line(&mut self, s: &str) -> Result<()> {
        self.push(s)?;
        self.push("\n")
    }
}
