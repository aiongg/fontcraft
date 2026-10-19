//! The subsetter (M12.17–M12.19; rows `subset`, `font.subset`, `compile.subsetOnExport`). A port
//! of fontTools' pyftsubset (MIT, Copyright 2013 Google, Inc.), table by table, with its option
//! names and defaults; HarfBuzz's subsetter is the second oracle.
//!
//! [`subset`] drops the tables the options remove, resolves the selection into a [`Plan`]
//! ([`closure`]), then each table module writes its tables into the output: [`core`] (M12.17),
//! [`layout`] (M12.18), [`var`] and [`other`] (M12.19), `varc` (M12.19c); [`core`] hands `CFF2`
//! and the subroutine steps of both CFF flavours to `cff2` (M12.19b). Each module has
//! `apply(plan, input, output) -> Result<()>` and leaves the tables it does not handle alone.
//! A table of the input that no module wrote is dropped, and [`SubsetResult::dropped`] says why:
//! no table that indexes glyphs is ever copied unchanged into a subset.

mod cff2;
pub mod closure;
pub mod core;
mod cs;
pub mod layout;
mod mirror;
pub mod other;
pub mod var;
mod varc;

use std::collections::{BTreeMap, BTreeSet};

use crate::Result;
use crate::sfnt::{SfntFont, Tag};

/// Tables pyftsubset drops unless told otherwise.
pub const DEFAULT_DROP_TABLES: [Tag; 12] =
    [*b"JSTF", *b"DSIG", *b"EBDT", *b"EBLC", *b"EBSC", *b"PCLT", *b"LTSH", *b"Feat", *b"Glat", *b"Gloc", *b"Silf", *b"Sill"];

/// Font-wide tables `hinting: false` drops (pyftsubset's default `--hinting-tables`).
pub const DEFAULT_HINTING_TABLES: [Tag; 6] = [*b"cvt ", *b"cvar", *b"fpgm", *b"prep", *b"hdmx", *b"VDMX"];

/// Name IDs kept by default.
pub const DEFAULT_NAME_IDS: [u16; 7] = [0, 1, 2, 3, 4, 5, 6];

/// Name language IDs kept by default (Windows English, United States).
pub const DEFAULT_NAME_LANGUAGES: [u16; 1] = [0x0409];

/// What to keep. The selections add up; an empty selection keeps `.notdef` only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubsetOptions {
    /// Characters to keep (within [`crate::limits::MAX_SUBSET_TEXT`]).
    pub text: String,
    /// Code points to keep (at most [`crate::limits::MAX_CODE_POINTS`]).
    pub unicodes: Vec<u32>,
    /// Keep every code point the `cmap` maps.
    pub all_unicodes: bool,
    /// Glyph names to keep.
    pub glyphs: Vec<String>,
    /// Glyph ids to keep.
    pub gids: Vec<u32>,
    /// Keep every glyph.
    pub all_glyphs: bool,
    /// A requested glyph name or id the font lacks is skipped instead of refused.
    pub ignore_missing_glyphs: bool,
    /// A requested code point the font does not map is skipped instead of refused.
    pub ignore_missing_unicodes: bool,
    /// Layout features to keep; `None` keeps [`layout::DEFAULT_LAYOUT_FEATURES`], a list
    /// holding `"*"` keeps all.
    pub layout_features: Option<Vec<String>>,
    /// Layout scripts to keep, each `script` or `script.LANG` (`script.dflt` keeps the default
    /// language system, `script.*` all of them); `None` or a list holding `"*"` keeps all.
    pub layout_scripts: Option<Vec<String>>,
    /// Add the glyphs the kept GSUB features can substitute in.
    pub layout_closure: bool,
    /// Keep glyph ids: dropped glyphs before the last kept one become empty instead of being
    /// removed.
    pub retain_gids: bool,
    /// Keep TrueType instructions and CFF hints.
    pub hinting: bool,
    /// Expand every CFF and CFF2 subroutine; otherwise only the unused ones are removed.
    pub desubroutinize: bool,
    /// Font-wide tables dropped when `hinting` is off.
    pub hinting_tables: Vec<Tag>,
    /// Keep glyph 0.
    pub notdef_glyph: bool,
    /// Keep the outline of glyph 0; otherwise it is emptied (its advance stays).
    pub notdef_outline: bool,
    /// Also keep glyphs 1 to 3 of a TrueType font.
    pub recommended_glyphs: bool,
    /// Add the mirrored partner of every requested bidirectional character.
    pub bidi_closure: bool,
    /// Keep glyph names in a TrueType font (`post` version 2); otherwise `post` becomes version 3.
    pub glyph_names: bool,
    /// Keep the `cmap` subtables that are neither Unicode nor symbol.
    pub legacy_cmap: bool,
    /// Keep the symbol `cmap` subtables (Windows platform, encoding 0).
    pub symbol_cmap: bool,
    /// Name IDs to keep; `None` keeps [`DEFAULT_NAME_IDS`], `Some([])` with `all_name_ids`
    /// keeps every ID below 256.
    pub name_ids: Option<Vec<u16>>,
    /// Keep every name ID below 256.
    pub all_name_ids: bool,
    /// Name language IDs to keep; `None` keeps [`DEFAULT_NAME_LANGUAGES`], `Some([])` keeps all.
    pub name_languages: Option<Vec<u16>>,
    /// Keep name records that are not Unicode.
    pub name_legacy: bool,
    /// Keep the `kern` table even when there is a `GPOS` table.
    pub legacy_kern: bool,
    /// Keep tables the subsetter knows nothing about.
    pub passthrough_tables: bool,
    /// Clear the `OS/2` Unicode range bits no kept code point falls in.
    pub prune_unicode_ranges: bool,
    /// Clear the `OS/2` code page bits the kept code points no longer support.
    pub prune_codepage_ranges: bool,
    /// Tables to drop besides [`DEFAULT_DROP_TABLES`].
    pub drop_tables: Vec<Tag>,
}

impl Default for SubsetOptions {
    fn default() -> Self {
        Self {
            text: String::new(),
            unicodes: Vec::new(),
            all_unicodes: false,
            glyphs: Vec::new(),
            gids: Vec::new(),
            all_glyphs: false,
            ignore_missing_glyphs: false,
            ignore_missing_unicodes: true,
            layout_features: None,
            layout_scripts: None,
            layout_closure: true,
            retain_gids: false,
            hinting: true,
            desubroutinize: false,
            hinting_tables: DEFAULT_HINTING_TABLES.to_vec(),
            notdef_glyph: true,
            notdef_outline: false,
            recommended_glyphs: false,
            bidi_closure: true,
            glyph_names: false,
            legacy_cmap: false,
            symbol_cmap: false,
            name_ids: None,
            all_name_ids: false,
            name_languages: None,
            name_legacy: false,
            legacy_kern: false,
            passthrough_tables: false,
            prune_unicode_ranges: true,
            prune_codepage_ranges: true,
            drop_tables: Vec::new(),
        }
    }
}

/// The resolved selection the table modules work from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Glyph count of the input.
    pub num_glyphs: usize,
    /// Glyph ids kept, after the closure, in the input's numbering.
    pub glyphs: BTreeSet<u32>,
    /// The glyphs before the MATH closure: MATH keeps the variants and assemblies of these.
    pub pre_math_glyphs: BTreeSet<u32>,
    /// The glyphs after the MATH closure: the rest of MATH is subset to these.
    pub math_glyphs: BTreeSet<u32>,
    /// The glyphs after the GSUB closure, before components and accent parts join: GSUB, GPOS,
    /// GDEF and kern are subset to these.
    pub layout_glyphs: BTreeSet<u32>,
    /// The glyphs after the COLR closure, before components join: COLR is subset to these.
    pub colr_glyphs: BTreeSet<u32>,
    /// Glyphs requested by name or id (they keep every code point that maps to them).
    pub requested: BTreeSet<u32>,
    /// Code points requested, with their mirrored partners under `bidi_closure`.
    pub unicodes: BTreeSet<u32>,
    /// Glyphs dropped but kept in place, empty, under `retain_gids`.
    pub emptied: BTreeSet<u32>,
    /// Output glyph id → input glyph id.
    pub new_to_old: Vec<u32>,
    /// Input glyph id → output glyph id.
    pub glyph_map: BTreeMap<u32, u32>,
    /// Layout feature tags GSUB and GPOS keep before subsetting.
    pub features: BTreeSet<Tag>,
    /// Requested glyph names and ids the font lacks (with `ignore_missing_glyphs`).
    pub missing_glyphs: Vec<String>,
    /// Requested code points the font does not map (with `ignore_missing_unicodes`).
    pub missing_unicodes: Vec<u32>,
    /// Charstring operations left of the font's `MAX_FONT_CHARSTRING_OPS` after the
    /// closure; the `.notdef` width scan takes from it.
    pub charstring_ops_left: usize,
    pub options: SubsetOptions,
}

impl Plan {
    /// Input glyph `old` stays in the output (kept or emptied).
    pub fn in_output(&self, old: u32) -> bool {
        self.glyph_map.contains_key(&old)
    }
}

/// A table of the input missing from the subset, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DroppedTable {
    pub tag: Tag,
    pub reason: String,
}

/// The subset font and what it kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubsetResult {
    pub font: SfntFont,
    /// Input glyph id → output glyph id.
    pub glyph_map: BTreeMap<u32, u32>,
    /// Names of the output glyphs in output order (emptied glyphs included).
    pub retained: Vec<String>,
    /// Tables of the input the subset does not have, in tag order.
    pub dropped: Vec<DroppedTable>,
    pub missing_glyphs: Vec<String>,
    pub missing_unicodes: Vec<u32>,
}

/// The layout tables ([`layout`]).
const LAYOUT_TABLES: [Tag; 3] = [*b"GDEF", *b"GSUB", *b"GPOS"];
/// Variation tables ([`var`], `CFF2` in [`core`], `VARC` in `varc`).
const VARIATION_TABLES: [Tag; 10] = [*b"fvar", *b"avar", *b"gvar", *b"cvar", *b"HVAR", *b"VVAR", *b"MVAR", *b"STAT", *b"CFF2", *b"VARC"];
/// Other tables pyftsubset subsets ([`other`]).
const OTHER_TABLES: [Tag; 13] =
    [*b"COLR", *b"CPAL", *b"SVG ", *b"CBDT", *b"CBLC", *b"sbix", *b"MATH", *b"BASE", *b"kern", *b"EBLC", *b"EBDT", *b"VORG", *b"hdmx"];

/// Subsets `font`.
pub fn subset(font: &SfntFont, options: &SubsetOptions) -> Result<SubsetResult> {
    let mut input = font.clone();
    let mut dropped: Vec<DroppedTable> = Vec::new();
    let mut drop = |input: &mut SfntFont, tag: Tag, reason: &str| {
        if input.remove(tag).is_some() {
            dropped.push(DroppedTable { tag, reason: reason.to_string() });
        }
    };
    for tag in font.tags() {
        if DEFAULT_DROP_TABLES.contains(&tag) || options.drop_tables.contains(&tag) {
            drop(&mut input, tag, "dropped by the options");
        } else if !options.hinting && options.hinting_tables.contains(&tag) {
            drop(&mut input, tag, "a hinting table");
        } else if tag == *b"kern" && !options.legacy_kern && font.tables.contains_key(b"GPOS") {
            drop(&mut input, tag, "the font has GPOS");
        }
    }

    let plan = closure::plan(&input, options)?;
    let mut output = SfntFont::new(input.flavour);
    core::apply(&plan, &input, &mut output)?;
    layout::apply(&plan, &input, &mut output)?;
    var::apply(&plan, &input, &mut output)?;
    varc::apply(&plan, &input, &mut output)?;
    other::apply(&plan, &input, &mut output)?;
    // The name table also keeps the names the other output tables refer to.
    let referenced = var::name_ids_in_use(&output);
    if let Some(name) = input.get(*b"name")
        && !referenced.is_empty()
    {
        output.insert(*b"name", core::subset_name(&plan, name, &referenced)?);
    }

    for tag in input.tags() {
        if output.tables.contains_key(&tag) {
            continue;
        }
        let known = LAYOUT_TABLES.contains(&tag) || VARIATION_TABLES.contains(&tag) || OTHER_TABLES.contains(&tag) || core::is_core(tag);
        if options.passthrough_tables && !known {
            if let Some(bytes) = input.get(tag) {
                output.insert(tag, bytes.to_vec());
            }
            continue;
        }
        let reason = if [*b"CBDT", *b"CBLC", *b"EBDT", *b"EBLC"].contains(&tag) {
            "its bitmap location or data table is missing"
        } else if tag == *b"CPAL" {
            "no color table uses its palettes"
        } else if core::is_core(tag) || LAYOUT_TABLES.contains(&tag) || VARIATION_TABLES.contains(&tag) || OTHER_TABLES.contains(&tag) {
            "empty after subsetting"
        } else {
            "the subsetter does not know this table"
        };
        dropped.push(DroppedTable { tag, reason: reason.to_string() });
    }
    // Keep the input's table order for the tables that remain.
    output.order = font.tags().into_iter().filter(|t| output.tables.contains_key(t)).collect();
    dropped.sort_by_key(|d| d.tag);

    let names = crate::reorder::glyph_names(font)?;
    let retained = plan.new_to_old.iter().map(|o| names.get(*o as usize).cloned().unwrap_or_else(|| format!("glyph{o:05}"))).collect();
    Ok(SubsetResult {
        font: output,
        glyph_map: plan.glyph_map.clone(),
        retained,
        dropped,
        missing_glyphs: plan.missing_glyphs.clone(),
        missing_unicodes: plan.missing_unicodes.clone(),
    })
}

#[cfg(test)]
mod tests;
