//! Tables a compile adds beside the core ones (M12.42; rows `meta`, `VORG`, `DSIG`,
//! `roundtrip.binaryPassThroughTables` and the legacy table rows of `40-tables.toml`).
//!
//! | Table | When | Source |
//! |---|---|---|
//! | `meta` | the meta Table parameter names design or supported languages | [`meta_table`]; both reference compilers write it from the same parameter |
//! | `VORG` | a CFF font with vertical metrics | [`vorg_table`], as ufo2ft's `setupTable_VORG` builds it (MIT) |
//! | `DSIG` | [`crate::Options::stub_dsig`] | the empty table of a project compile ([`crate::makeotf::STUB_DSIG`]) |
//! | stored tables | [`crate::Options::pass_through_tables`] and a source that holds them | [`stored_tables`]: the bytes a compiled font had when it was opened, copied unchanged |
//!
//! **Stored tables.** A compiled font that is opened keeps the tables of [`STORED_TABLES`] in the
//! font's extra data (family [`Family::Binary`]): each under `table.<tag>` as data, and the
//! checksum of the glyph order they belong to under [`GLYPH_SET_KEY`] ([`store_tables`]). These
//! tables index glyphs by id, and none of them can be rebuilt from the model, so a compile copies
//! them only when asked and only when the built glyph order has the same checksum. A table the
//! compile built itself is never replaced. The editor's normal export leaves them out: stale tables
//! next to edited glyphs are worse than missing ones.
//!
//! The `VORG` and `meta` rules follow ufo2ft's `outlineCompiler.py` (MIT) and glyphsLib's meta Table
//! conversion (Apache-2.0); see `NOTICE`.

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_model::plist::{Data, Str, Value};
use fontcraft_model::{Family, Font, Params};
use read_fonts::FontRef;
use write_fonts::FontBuilder;
use write_fonts::tables::meta::{DataMapRecord, Meta, Metadata, ScriptLangTag};
use write_fonts::types::Tag;

use crate::{CompileError, Compiled, Options};

/// The custom parameter that holds the `meta` languages.
pub const META_PARAM: &str = "meta Table";
/// Longest language tag, in bytes, that the warning about malformed tags tests. BCP 47 tags in
/// use are far shorter.
pub const MAX_META_TAG_LEN: usize = 64;
/// Most entries of the meta Table parameter that are read; the rest are left out with a warning.
pub const MAX_META_ENTRIES: usize = 4_096;
/// Longest data text of one entry, in bytes; a longer entry is left out with a warning.
pub const MAX_META_DATA: usize = 4_096;

/// The tables a compiled font keeps when it is opened, for [`crate::Options::pass_through_tables`]:
/// the legacy hinting tables, justification, embedded bitmaps and the Apple layout tables (rows
/// `LTSH`, `VDMX`, `hdmx`, `JSTF`, `EBDT`, `morx`, `kerx`, `trak`).
pub const STORED_TABLES: [&str; 14] =
    ["LTSH", "VDMX", "hdmx", "JSTF", "EBDT", "EBLC", "EBSC", "morx", "mort", "feat", "ltag", "kerx", "ankr", "trak"];
/// The key of the stored tables' glyph-order checksum ([`glyph_set_checksum`]).
pub const GLYPH_SET_KEY: &str = "tableGlyphSet";
/// The key prefix of a stored table; the tag follows.
const TABLE_KEY_PREFIX: &str = "table.";
/// Most bytes of stored tables one compile copies.
pub const MAX_STORED_BYTES: usize = 256 << 20;

// ---------- meta ----------

/// Whether `tag` is a well-formed ScriptLangTag: subtags of 1 to 8 ASCII letters and digits
/// joined by hyphens (BCP 47's shape; only the script subtag is required, so `Latn` is one).
pub fn is_script_lang_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= MAX_META_TAG_LEN
        && tag.split('-').all(|s| (1..=8).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// The `meta` table of the meta Table parameter in `params`: a `dlng` and an `slng` record, each
/// the data of every entry with that tag joined by commas, in entry order, as both reference
/// compilers write it (`compile.meta`). The text is written as it is given; a comma-separated
/// piece that is not a well-formed tag ([`is_script_lang_tag`]) gives a warning. Entries with
/// another tag are left out with a warning. `None` when no entry is left.
pub fn meta_table(params: &Params, warnings: &mut Vec<String>) -> Result<Option<Vec<u8>>, CompileError> {
    let Some(value) = params.get(META_PARAM) else { return Ok(None) };
    let Some(entries) = value.as_array() else {
        warnings.push(format!("the {META_PARAM} parameter is not a list of entries, so no meta table was written"));
        return Ok(None);
    };
    let mut lists: [(Tag, Vec<String>); 2] = [(Tag::new(b"dlng"), Vec::new()), (Tag::new(b"slng"), Vec::new())];
    let mut bad: Vec<String> = Vec::new();
    for entry in entries.items.iter().take(MAX_META_ENTRIES) {
        let Some(dict) = entry.as_dict() else { continue };
        let (Some(tag), Some(data)) = (dict.get("tag").and_then(Value::as_str), dict.get("data").and_then(Value::as_str)) else { continue };
        let Some((_, list)) = lists.iter_mut().find(|(t, _)| t.to_be_bytes().as_slice() == tag.as_bytes()) else {
            warnings
                .push(format!("the {META_PARAM} entry `{}` was left out: only dlng and slng are written", tag.chars().take(8).collect::<String>()));
            continue;
        };
        if data.len() > MAX_META_DATA {
            warnings.push(format!("a {META_PARAM} entry longer than {MAX_META_DATA} bytes was left out"));
            continue;
        }
        for piece in data.split(',').map(str::trim) {
            if !is_script_lang_tag(piece) && bad.len() < 10 {
                bad.push(format!("`{}`", piece.chars().take(MAX_META_TAG_LEN).collect::<String>()));
            }
        }
        list.push(data.to_string());
    }
    if entries.items.len() > MAX_META_ENTRIES {
        warnings.push(format!("the {META_PARAM} parameter has more than {MAX_META_ENTRIES} entries; the rest were left out"));
    }
    if !bad.is_empty() {
        warnings.push(format!("the {META_PARAM} parameter holds text that is not a language tag, written as it is: {}", bad.join(", ")));
    }
    let records: Vec<DataMapRecord> = lists
        .into_iter()
        .filter(|(_, list)| !list.is_empty())
        .map(|(tag, list)| {
            // write-fonts joins the items with commas.
            let items = list.into_iter().filter_map(|t| ScriptLangTag::new(t).ok()).collect();
            DataMapRecord::new(tag, Metadata::ScriptLangTags(items))
        })
        .collect();
    if records.is_empty() {
        return Ok(None);
    }
    write_fonts::dump_table(&Meta::new(records)).map(Some).map_err(|e| CompileError::Table { table: "meta", message: e.to_string() })
}

// ---------- VORG ----------

/// The `VORG` table of a CFF font whose glyphs, in glyph order, have the vertical origins
/// `origins` (y of the origin, in font units): the most frequent origin as the default (the first
/// in glyph order among equally frequent ones, as Python's `Counter.most_common` picks it), and a
/// record for every glyph whose origin differs, by glyph id. `None` for a font without glyphs.
pub fn vorg_table(origins: &[i16]) -> Option<Vec<u8>> {
    let mut counts: HashMap<i16, (usize, usize)> = HashMap::new();
    for (i, o) in origins.iter().enumerate() {
        counts.entry(*o).or_insert((0, i)).0 += 1;
    }
    let (&default, _) = counts.iter().max_by(|a, b| a.1.0.cmp(&b.1.0).then(b.1.1.cmp(&a.1.1)))?;
    let records: Vec<(u16, i16)> =
        origins.iter().enumerate().filter(|(_, o)| **o != default).filter_map(|(i, o)| Some((u16::try_from(i).ok()?, *o))).collect();
    let count = u16::try_from(records.len()).ok()?;
    let mut out = Vec::with_capacity(8 + 4 * records.len());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&default.to_be_bytes());
    out.extend_from_slice(&count.to_be_bytes());
    for (gid, y) in records {
        out.extend_from_slice(&gid.to_be_bytes());
        out.extend_from_slice(&y.to_be_bytes());
    }
    Some(out)
}

// ---------- stored tables ----------

/// A checksum of a glyph order (FNV-1a, 64 bits, over each name followed by a line feed), as
/// sixteen hexadecimal digits.
pub fn glyph_set_checksum<S: AsRef<str>>(order: &[S]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for name in order {
        for b in name.as_ref().bytes().chain(std::iter::once(b'\n')) {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

/// Keeps the tables of `tables` that [`STORED_TABLES`] names in `font`'s extra data, with the
/// checksum of `order`, the glyph order they index; returns how many were kept. Tables stored
/// before are replaced. For the reader of compiled fonts.
pub fn store_tables(font: &mut Font, tables: &[(Tag, &[u8])], order: &[String]) -> usize {
    let extra = Arc::make_mut(&mut font.extra);
    for tag in STORED_TABLES {
        extra.remove(Family::Binary, &format!("{TABLE_KEY_PREFIX}{tag}"));
    }
    let mut kept = 0usize;
    let mut bytes = 0usize;
    for (tag, data) in tables {
        let name = tag.to_string();
        if !STORED_TABLES.contains(&name.as_str()) {
            continue;
        }
        bytes = bytes.saturating_add(data.len());
        if bytes > MAX_STORED_BYTES {
            break;
        }
        extra.insert(Family::Binary, format!("{TABLE_KEY_PREFIX}{name}"), Value::Data(Data::new(data.to_vec())));
        kept += 1;
    }
    if kept > 0 {
        extra.insert(Family::Binary, GLYPH_SET_KEY, Value::String(Str::new(glyph_set_checksum(order))));
    } else {
        extra.remove(Family::Binary, GLYPH_SET_KEY);
    }
    kept
}

/// The tables `font` keeps from the compiled font it was opened from, in [`STORED_TABLES`]
/// order, and the checksum of their glyph order.
pub fn stored_tables(font: &Font) -> (Vec<(Tag, &[u8])>, Option<&str>) {
    let tables = STORED_TABLES
        .iter()
        .filter_map(|tag| match font.extra.get(Family::Binary, &format!("{TABLE_KEY_PREFIX}{tag}")) {
            Some(Value::Data(d)) => Some((Tag::new_checked(tag.as_bytes()).ok()?, d.bytes())),
            _ => None,
        })
        .collect();
    let checksum = font.extra.get(Family::Binary, GLYPH_SET_KEY).and_then(Value::as_str);
    (tables, checksum)
}

/// The tags of the stored tables a build of `font` with glyph order `order` copies
/// ([`crate::Options::pass_through_tables`]), for the compile plan.
pub fn passed_through(font: &Font, order: &[String]) -> Vec<String> {
    let (tables, checksum) = stored_tables(font);
    if checksum != Some(glyph_set_checksum(order).as_str()) {
        return Vec::new();
    }
    tables.iter().map(|(tag, _)| tag.to_string()).collect()
}

/// `compiled` with the stored tables of `font` added when their glyph order is the build's.
/// Nothing changes, with a warning, when it is not.
fn pass_through(font: &Font, compiled: &mut Compiled) -> Result<(), CompileError> {
    let (tables, checksum) = stored_tables(font);
    if tables.is_empty() {
        compiled.warnings.push("no tables were passed through: the source keeps none from a compiled font".into());
        return Ok(());
    }
    let names = || tables.iter().map(|(t, _)| t.to_string()).collect::<Vec<_>>().join(", ");
    if checksum != Some(glyph_set_checksum(&compiled.glyph_order).as_str()) {
        compiled.warnings.push(format!(
            "the tables {} kept from the opened font were left out: the font's glyph order is no longer the one they were made for",
            names()
        ));
        return Ok(());
    }
    let table_error = |e: String| CompileError::Table { table: "the font directory", message: e };
    let built = FontRef::new(&compiled.bytes).map_err(|e| table_error(e.to_string()))?;
    let mut builder = FontBuilder::new();
    let mut have = Vec::new();
    for record in built.table_directory().table_records() {
        let tag = record.tag();
        if let Some(data) = built.table_data(tag) {
            builder.add_raw(tag, data.as_bytes().to_vec());
            have.push(tag);
        }
    }
    let mut bytes = 0usize;
    for (tag, data) in &tables {
        if have.contains(tag) {
            compiled.warnings.push(format!("the kept `{tag}` table was left out: the compile wrote its own"));
            continue;
        }
        bytes = bytes.saturating_add(data.len());
        if bytes > MAX_STORED_BYTES {
            return Err(CompileError::Table {
                table: "the font directory",
                message: format!("the kept tables hold more than {MAX_STORED_BYTES} bytes"),
            });
        }
        builder.add_raw(*tag, data.to_vec());
    }
    compiled.bytes = builder.build();
    Ok(())
}

/// What the options add to a built font after assembly: the stored tables, then a stub `DSIG`.
/// `font` is the source (`None` for a resolved font compiled on its own, which keeps no tables).
pub(crate) fn finish(font: Option<&Font>, compiled: &mut Compiled, options: &Options) -> Result<(), CompileError> {
    if options.pass_through_tables {
        match font {
            Some(font) => pass_through(font, compiled)?,
            None => compiled.warnings.push("no tables were passed through: the compile was given no source".into()),
        }
    }
    if options.stub_dsig {
        crate::makeotf::finish(compiled, &crate::makeotf::Finish { stub_dsig: true, ..Default::default() })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
