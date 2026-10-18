//! Text views of a font and the normalised dump (M12.10; row `dump` with its text views,
//! `@font`, `@glyphs` and `--normalize`). Oracles: AFDKO spot and `tx -dump`, ttxn.
//!
//! | View | Module | Oracle |
//! |---|---|---|
//! | the normalised dump ([`ViewOptions::normalize`]) | [`ttxn`], [`ttxn_otl`] | `ttxn` |
//! | `@font`, `@glyphs` | [`txdump`] | `tx -dump` |
//! | `GPOS=normalized`, `GDEF=normalized` | [`crate::normalize`] | otl-normalizer |
//! | `GSUB=fea`, `GPOS=fea`, `GDEF=fea` | [`crate::fea_write`] | feaLib round trip |
//!
//! The spot table views of `docs/spec/20-cli.md` › dump (`raw` and the per-table listings) are
//! not built: asking for one returns [`TablesError::NotBuilt`].
//!
//! Every function here reads untrusted bytes: counts are capped by the limits of
//! [`crate::limits`], the text by [`MAX_TEXT_OUTPUT`], and nothing panics.

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

pub mod ttxn;
pub mod ttxn_otl;
pub mod txdump;

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
    /// Glyphs the view covers (`@glyphs`); empty covers every glyph.
    pub glyphs: Vec<String>,
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
        GLYPHS_VIEW => txdump::glyphs(font, &options.glyphs, options.glyph_detail, !options.no_hints),
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
        _ => Err(TablesError::NotBuilt("this table view")),
    }
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
