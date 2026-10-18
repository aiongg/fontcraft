//! Compiled layout put into an existing binary font (M12.14; rows `features compile`,
//! `layout.compileFeatures`, `features lint`, `layout.lintFeatures`). Oracle: fontTools feaLib.
//!
//! The feature compiler (`fontcraft-fea`) gives tables as bytes and the values the code's table
//! blocks set. This module puts them into a font the way feaLib's builder does (fontTools
//! `feaLib/builder.py`, MIT License, see `NOTICE`):
//! - `head`, `hhea`, `vhea` and `OS/2` keep every field the code does not set. A field that only
//!   a later `OS/2` version holds raises the table to that version, the fields it adds zero.
//! - The code's names replace the records with the same platform, encoding, language and name id
//!   and are added otherwise; records stay sorted.
//! - `usMaxContext` is the longest context of the `GSUB` and `GPOS` lookups, ported from
//!   fontTools' `otlLib/maxContextCalc.py` (MIT License).
//!
//! Also the glyph names a compiler sees for the font, and the lint's mark-attachment note.

use std::collections::BTreeSet;

use read_fonts::tables::gpos::PositionSubtables;
use read_fonts::tables::gsub::SubstitutionSubtables;
use read_fonts::tables::layout::{ChainedSequenceContext, ExtensionLookup, SequenceContext, Subtables};
use read_fonts::{FontData, FontRead, TableProvider};

use crate::features::{Budget, Provider, glyph_names, read_err};
use crate::sfnt::{SfntFont, Tag, tag_str};
use crate::{Result, TablesError};

const GSUB: Tag = *b"GSUB";
const GPOS: Tag = *b"GPOS";
const OS2: Tag = *b"OS/2";
const HEAD: Tag = *b"head";
const HHEA: Tag = *b"hhea";
const VHEA: Tag = *b"vhea";
const NAME: Tag = *b"name";

/// The first name id a font may give its own names.
const FIRST_FONT_NAME_ID: u16 = 256;
/// The last valid name id.
const LAST_NAME_ID: u16 = 32_767;
/// Most glyphs [`marks_used_as_bases`] reports.
pub const MAX_MARK_BASE_NOTES: usize = 1_000;

/// The glyph names a feature compiler sees for `font`, by glyph id: the names fontTools gives
/// the font when it loads it ([`crate::features`]), so code written against the font by
/// `layout.decompileFeatures` or by fontTools compiles against the same names.
pub fn compiler_glyph_names(font: &SfntFont) -> Result<Vec<String>> {
    glyph_names(&Provider(font), &mut Budget::new())
}

/// The values of a `table hhea` or `table vhea` block; `None` keeps the font's value. For
/// `vhea` the first three are the vertical typographic ascender, descender and line gap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MetricFields {
    pub ascender: Option<i16>,
    pub descender: Option<i16>,
    pub line_gap: Option<i16>,
    pub caret_offset: Option<i16>,
}

/// The values of a `table OS/2` block; `None` keeps the font's value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Os2Fields {
    pub weight_class: Option<u16>,
    pub width_class: Option<u16>,
    pub fs_type: Option<u16>,
    pub family_class: Option<i16>,
    pub panose: Option<[u8; 10]>,
    pub unicode_range: Option<[u32; 4]>,
    pub vendor_id: Option<Tag>,
    pub typo_ascender: Option<i16>,
    pub typo_descender: Option<i16>,
    pub typo_line_gap: Option<i16>,
    pub win_ascent: Option<u16>,
    pub win_descent: Option<u16>,
    pub code_page_range: Option<[u32; 2]>,
    pub x_height: Option<i16>,
    pub cap_height: Option<i16>,
    pub lower_optical_point_size: Option<u16>,
    pub upper_optical_point_size: Option<u16>,
}

/// What the code's `head`, `hhea`, `vhea` and `OS/2` blocks set.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlockValues {
    pub font_revision: Option<f64>,
    pub hhea: MetricFields,
    pub vhea: MetricFields,
    pub os2: Os2Fields,
}

/// What [`apply_block_values`] changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockReport {
    /// The tables written, in `head`, `hhea`, `vhea`, `OS/2` order.
    pub changed: Vec<Tag>,
    /// Values left out because the font lacks their table.
    pub warnings: Vec<String>,
}

/// Writes the block values into the font's tables. A block whose table the font lacks is a
/// warning, as there is nothing to keep its other fields from; feaLib creates such tables only
/// for its own tests.
pub fn apply_block_values(font: &mut SfntFont, values: &BlockValues) -> Result<BlockReport> {
    let mut report = BlockReport::default();
    if let Some(revision) = values.font_revision {
        let fixed = (revision * 65_536.0).round();
        if !fixed.is_finite() || fixed < f64::from(i32::MIN) || fixed > f64::from(i32::MAX) {
            return Err(TablesError::BadOptions(format!("FontRevision {revision} does not fit the head table's 16.16 number")));
        }
        let fixed = fixed as i32;
        edit(font, HEAD, 54, &mut report, |data| put(data, 4, &fixed.to_be_bytes()))?;
    }
    for (tag, fields) in [(HHEA, &values.hhea), (VHEA, &values.vhea)] {
        if *fields != MetricFields::default() {
            edit(font, tag, 36, &mut report, |data| {
                let slots = [(4, fields.ascender), (6, fields.descender), (8, fields.line_gap), (22, fields.caret_offset)];
                for (at, value) in slots {
                    if let Some(v) = value {
                        put(data, at, &v.to_be_bytes());
                    }
                }
            })?;
        }
    }
    if values.os2 != Os2Fields::default() {
        edit(font, OS2, 2, &mut report, |data| patch_os2(data, &values.os2))?;
    }
    Ok(report)
}

/// Replaces `tag` with `change` applied to a copy of it; a missing table is a warning, one
/// shorter than `min_len` an error.
fn edit(font: &mut SfntFont, tag: Tag, min_len: usize, report: &mut BlockReport, change: impl FnOnce(&mut Vec<u8>)) -> Result<()> {
    let Some(old) = font.get(tag) else {
        report.warnings.push(format!("the font has no '{}' table: the code's values for it are not written", tag_str(tag).trim_end()));
        return Ok(());
    };
    if old.len() < min_len {
        return Err(TablesError::invalid(format!(
            "the '{}' table is {} bytes long, shorter than its fixed fields",
            tag_str(tag).trim_end(),
            old.len()
        )));
    }
    let mut data = old.to_vec();
    change(&mut data);
    font.insert(tag, data);
    report.changed.push(tag);
    Ok(())
}

/// Writes `bytes` at `at`, zero-filling the table up to there when it is shorter.
fn put(data: &mut Vec<u8>, at: usize, bytes: &[u8]) {
    let end = at.saturating_add(bytes.len());
    if data.len() < end {
        data.resize(end, 0);
    }
    if let Some(slot) = data.get_mut(at..end) {
        slot.copy_from_slice(bytes);
    }
}

/// The length of an `OS/2` table of `version`.
fn os2_len(version: u16) -> usize {
    match version {
        0 => 78,
        1 => 86,
        2..=4 => 96,
        _ => 100,
    }
}

/// The `OS/2` fields of `o` written into `data`, the version raised as feaLib raises it: code
/// pages need version 1, x-height and cap height 2, optical sizes 5.
fn patch_os2(data: &mut Vec<u8>, o: &Os2Fields) {
    let mut needed = 0u16;
    let u16s = [(4, o.weight_class), (6, o.width_class), (8, o.fs_type), (74, o.win_ascent), (76, o.win_descent)];
    for (at, value) in u16s {
        if let Some(v) = value {
            put(data, at, &v.to_be_bytes());
        }
    }
    let i16s = [(30, o.family_class), (68, o.typo_ascender), (70, o.typo_descender), (72, o.typo_line_gap)];
    for (at, value) in i16s {
        if let Some(v) = value {
            put(data, at, &v.to_be_bytes());
        }
    }
    if let Some(panose) = o.panose {
        put(data, 32, &panose);
    }
    if let Some(ranges) = o.unicode_range {
        for (i, r) in ranges.iter().enumerate() {
            put(data, 42 + 4 * i, &r.to_be_bytes());
        }
    }
    if let Some(vendor) = o.vendor_id {
        put(data, 58, &vendor);
    }
    if let Some(pages) = o.code_page_range {
        for (i, r) in pages.iter().enumerate() {
            put(data, 78 + 4 * i, &r.to_be_bytes());
        }
        needed = needed.max(1);
    }
    for (at, value) in [(86, o.x_height), (88, o.cap_height)] {
        if let Some(v) = value {
            put(data, at, &v.to_be_bytes());
            needed = needed.max(2);
        }
    }
    for (at, value) in [(96, o.lower_optical_point_size), (98, o.upper_optical_point_size)] {
        if let Some(v) = value {
            put(data, at, &v.to_be_bytes());
            needed = needed.max(5);
        }
    }
    let current = data.first_chunk::<2>().map_or(0, |b| u16::from_be_bytes(*b));
    let version = current.max(needed);
    let len = os2_len(version);
    if data.len() < len {
        data.resize(len, 0);
    }
    put(data, 0, &version.to_be_bytes());
}

/// One name record and its string bytes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct NameRecord {
    platform: u16,
    encoding: u16,
    language: u16,
    name_id: u16,
    bytes: Vec<u8>,
}

/// The records and language tags (as their string bytes) of a `name` table.
fn name_records(data: &[u8], what: &str) -> Result<(Vec<NameRecord>, Vec<Vec<u8>>)> {
    let name = read_fonts::tables::name::Name::read(FontData::new(data)).map_err(|e| read_err(what, e))?;
    let storage = name.string_data();
    let string = |offset: usize, len: u16| -> Result<Vec<u8>> {
        storage
            .as_bytes()
            .get(offset..offset.saturating_add(usize::from(len)))
            .map(<[u8]>::to_vec)
            .ok_or_else(|| TablesError::invalid(format!("a string of the {what} lies outside the table")))
    };
    let mut records = Vec::with_capacity(name.name_record().len());
    for r in name.name_record() {
        records.push(NameRecord {
            platform: r.platform_id(),
            encoding: r.encoding_id(),
            language: r.language_id(),
            name_id: r.name_id().to_u16(),
            bytes: string(r.string_offset().to_u32() as usize, r.length())?,
        });
    }
    let mut tags = Vec::new();
    for t in name.lang_tag_record().unwrap_or_default() {
        tags.push(string(t.lang_tag_offset().to_u32() as usize, t.length())?);
    }
    Ok((records, tags))
}

/// The first name id above every id of 256 and above that `font` uses (256 when it uses none):
/// where the code's own name ids are moved so they cannot replace the font's names.
pub fn first_free_name_id(font: &SfntFont) -> Result<u16> {
    let Some(data) = font.get(NAME) else { return Ok(FIRST_FONT_NAME_ID) };
    let (records, _) = name_records(data, "name table")?;
    let highest = records.iter().map(|r| r.name_id).filter(|id| (FIRST_FONT_NAME_ID..=LAST_NAME_ID).contains(id)).max();
    match highest {
        None => Ok(FIRST_FONT_NAME_ID),
        Some(LAST_NAME_ID) => Err(TablesError::BadOptions(format!("the font uses name id {LAST_NAME_ID}, so the code's names have no free id"))),
        Some(id) => Ok(id.saturating_add(1)),
    }
}

/// The font's `name` table (or none) with the records of `added` merged in: a record with the
/// same platform, encoding, language and name id is replaced, others are added, and the result
/// is sorted as fontTools sorts it. Language tags of a version 1 table are kept.
pub fn merge_names(existing: Option<&[u8]>, added: &[u8]) -> Result<Vec<u8>> {
    let (mut records, tags) = match existing {
        Some(data) => name_records(data, "font's name table")?,
        None => (Vec::new(), Vec::new()),
    };
    let (new, _) = name_records(added, "compiled name table")?;
    for record in new {
        let key = (record.platform, record.encoding, record.language, record.name_id);
        records.retain(|r| (r.platform, r.encoding, r.language, r.name_id) != key);
        records.push(record);
    }
    records.sort();
    write_name(&records, &tags)
}

/// A `name` table: version 0, or 1 when there are language tags.
fn write_name(records: &[NameRecord], tags: &[Vec<u8>]) -> Result<Vec<u8>> {
    let too_large = || TablesError::TooLarge { what: "the name table", limit: usize::from(u16::MAX) };
    let count = u16::try_from(records.len()).map_err(|_| too_large())?;
    let tag_count = u16::try_from(tags.len()).map_err(|_| too_large())?;
    let mut header = 6usize.saturating_add(12usize.saturating_mul(records.len()));
    if !tags.is_empty() {
        header = header.saturating_add(2).saturating_add(4usize.saturating_mul(tags.len()));
    }
    let storage_offset = u16::try_from(header).map_err(|_| too_large())?;
    let mut out = Vec::with_capacity(header);
    let mut storage: Vec<u8> = Vec::new();
    let mut place = |bytes: &[u8]| -> Result<(u16, u16)> {
        let offset = u16::try_from(storage.len()).map_err(|_| too_large())?;
        let len = u16::try_from(bytes.len()).map_err(|_| too_large())?;
        storage.extend_from_slice(bytes);
        Ok((offset, len))
    };
    let mut record_bytes = Vec::with_capacity(records.len().saturating_mul(12));
    for r in records {
        let (offset, len) = place(&r.bytes)?;
        for v in [r.platform, r.encoding, r.language, r.name_id, len, offset] {
            record_bytes.extend_from_slice(&v.to_be_bytes());
        }
    }
    let mut tag_bytes = Vec::with_capacity(tags.len().saturating_mul(4));
    for t in tags {
        let (offset, len) = place(t)?;
        tag_bytes.extend_from_slice(&len.to_be_bytes());
        tag_bytes.extend_from_slice(&offset.to_be_bytes());
    }
    let version: u16 = if tags.is_empty() { 0 } else { 1 };
    for v in [version, count, storage_offset] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.extend_from_slice(&record_bytes);
    if !tags.is_empty() {
        out.extend_from_slice(&tag_count.to_be_bytes());
        out.extend_from_slice(&tag_bytes);
    }
    out.extend_from_slice(&storage);
    Ok(out)
}

/// The longest glyph sequence any `GSUB` or `GPOS` lookup of `font` looks at (`OS/2`
/// `usMaxContext`), as fontTools computes it: 1 for single, multiple and alternate
/// substitution and single positioning, 2 for pair positioning, the component count of a
/// ligature, the input plus look-ahead length of a chaining rule, the input length of a
/// contextual rule, 1 plus the look-ahead length of a reverse chaining rule. Attachment lookups
/// count nothing.
pub fn max_context(font: &SfntFont) -> Result<u16> {
    let p = Provider(font);
    let mut budget = Budget::new();
    let mut max = 0u32;
    if font.get(GSUB).is_some() {
        let gsub = p.gsub().map_err(|e| read_err("GSUB", e))?;
        let list = gsub.lookup_list().map_err(|e| read_err("GSUB lookup list", e))?;
        for lookup in list.lookups().iter() {
            let lookup = lookup.map_err(|e| read_err("GSUB lookup", e))?;
            let subtables = lookup.subtables().map_err(|e| read_err("GSUB lookup", e))?;
            max = max.max(gsub_context(&subtables, &mut budget)?);
        }
    }
    if font.get(GPOS).is_some() {
        let gpos = p.gpos().map_err(|e| read_err("GPOS", e))?;
        let list = gpos.lookup_list().map_err(|e| read_err("GPOS lookup list", e))?;
        for lookup in list.lookups().iter() {
            let lookup = lookup.map_err(|e| read_err("GPOS lookup", e))?;
            let subtables = lookup.subtables().map_err(|e| read_err("GPOS lookup", e))?;
            max = max.max(gpos_context(&subtables, &mut budget)?);
        }
    }
    Ok(u16::try_from(max).unwrap_or(u16::MAX))
}

/// Sets `OS/2.usMaxContext` from the layout tables when the font has `GSUB` or `GPOS` and an
/// `OS/2` table of version 2 or later (earlier versions have no such field), as feaLib does
/// after every compile. Answers whether the table was written.
pub fn update_max_context(font: &mut SfntFont) -> Result<bool> {
    if font.get(GSUB).is_none() && font.get(GPOS).is_none() {
        return Ok(false);
    }
    let Some(os2) = font.get(OS2) else { return Ok(false) };
    let version = os2.first_chunk::<2>().map_or(0, |b| u16::from_be_bytes(*b));
    if version < 2 || os2.len() < 96 {
        return Ok(false);
    }
    let mut data = os2.to_vec();
    let value = max_context(font)?;
    put(&mut data, 94, &value.to_be_bytes());
    font.insert(OS2, data);
    Ok(true)
}

/// The largest of `f` over the subtables, each charged to the budget.
fn each<'a, T, E>(s: &Subtables<'a, T, E>, what: &str, budget: &mut Budget, mut f: impl FnMut(T, &mut Budget) -> Result<u32>) -> Result<u32>
where
    T: FontRead<'a, Args = ()> + 'a,
    E: ExtensionLookup<'a, T> + 'a,
{
    let mut max = 0;
    for t in s.iter() {
        budget.charge(1)?;
        let t = t.map_err(|e| read_err(what, e))?;
        max = max.max(f(t, budget)?);
    }
    Ok(max)
}

fn gsub_context(s: &SubstitutionSubtables, budget: &mut Budget) -> Result<u32> {
    let what = "GSUB subtable";
    match s {
        SubstitutionSubtables::Single(t) => each(t, what, budget, |_, _| Ok(1)),
        SubstitutionSubtables::Multiple(t) => each(t, what, budget, |_, _| Ok(1)),
        SubstitutionSubtables::Alternate(t) => each(t, what, budget, |_, _| Ok(1)),
        SubstitutionSubtables::Ligature(t) => each(t, what, budget, |st, budget| {
            let mut max = 0;
            for set in st.ligature_sets().iter() {
                let set = set.map_err(|e| read_err("ligature set", e))?;
                for lig in set.ligatures().iter() {
                    budget.charge(1)?;
                    let lig = lig.map_err(|e| read_err("ligature", e))?;
                    max = max.max(u32::from(lig.component_count()));
                }
            }
            Ok(max)
        }),
        SubstitutionSubtables::Contextual(t) => each(t, what, budget, |st, budget| sequence_context(&st, budget)),
        SubstitutionSubtables::ChainContextual(t) => each(t, what, budget, |st, budget| chained_context(&st, budget)),
        SubstitutionSubtables::Reverse(t) => each(t, what, budget, |st, _| Ok(1 + u32::from(st.lookahead_glyph_count()))),
        SubstitutionSubtables::EmptyExtension => Ok(0),
    }
}

fn gpos_context(s: &PositionSubtables, budget: &mut Budget) -> Result<u32> {
    let what = "GPOS subtable";
    match s {
        PositionSubtables::Single(t) => each(t, what, budget, |_, _| Ok(1)),
        PositionSubtables::Pair(t) => each(t, what, budget, |_, _| Ok(2)),
        PositionSubtables::Contextual(t) => each(t, what, budget, |st, budget| sequence_context(&st, budget)),
        PositionSubtables::ChainContextual(t) => each(t, what, budget, |st, budget| chained_context(&st, budget)),
        PositionSubtables::Cursive(_)
        | PositionSubtables::MarkToBase(_)
        | PositionSubtables::MarkToLig(_)
        | PositionSubtables::MarkToMark(_)
        | PositionSubtables::EmptyExtension => Ok(0),
    }
}

/// The longest input sequence of a contextual subtable.
fn sequence_context(st: &SequenceContext, budget: &mut Budget) -> Result<u32> {
    let mut max = 0u32;
    match st {
        SequenceContext::Format1(t) => {
            for set in t.seq_rule_sets().iter().flatten() {
                let set = set.map_err(|e| read_err("contextual rule set", e))?;
                for rule in set.seq_rules().iter() {
                    budget.charge(1)?;
                    let rule = rule.map_err(|e| read_err("contextual rule", e))?;
                    max = max.max(u32::from(rule.glyph_count()));
                }
            }
        }
        SequenceContext::Format2(t) => {
            for set in t.class_seq_rule_sets().iter().flatten() {
                let set = set.map_err(|e| read_err("contextual rule set", e))?;
                for rule in set.class_seq_rules().iter() {
                    budget.charge(1)?;
                    let rule = rule.map_err(|e| read_err("contextual rule", e))?;
                    max = max.max(u32::from(rule.glyph_count()));
                }
            }
        }
        SequenceContext::Format3(t) => max = u32::from(t.glyph_count()),
    }
    Ok(max)
}

/// The longest input plus look-ahead sequence of a chaining subtable.
fn chained_context(st: &ChainedSequenceContext, budget: &mut Budget) -> Result<u32> {
    let mut max = 0u32;
    match st {
        ChainedSequenceContext::Format1(t) => {
            for set in t.chained_seq_rule_sets().iter().flatten() {
                let set = set.map_err(|e| read_err("chaining rule set", e))?;
                for rule in set.chained_seq_rules().iter() {
                    budget.charge(1)?;
                    let rule = rule.map_err(|e| read_err("chaining rule", e))?;
                    max = max.max(u32::from(rule.input_glyph_count()) + u32::from(rule.lookahead_glyph_count()));
                }
            }
        }
        ChainedSequenceContext::Format2(t) => {
            for set in t.chained_class_seq_rule_sets().iter().flatten() {
                let set = set.map_err(|e| read_err("chaining rule set", e))?;
                for rule in set.chained_class_seq_rules().iter() {
                    budget.charge(1)?;
                    let rule = rule.map_err(|e| read_err("chaining rule", e))?;
                    max = max.max(u32::from(rule.input_glyph_count()) + u32::from(rule.lookahead_glyph_count()));
                }
            }
        }
        ChainedSequenceContext::Format3(t) => max = u32::from(t.input_glyph_count()) + u32::from(t.lookahead_glyph_count()),
    }
    Ok(max)
}

/// Glyphs that one mark-to-base or mark-to-ligature subtable of `gpos` lists both as a mark and
/// as a base or ligature, as (lookup index, glyph id), each pair once, at most
/// [`MAX_MARK_BASE_NOTES`]. Usually harmless (a spacing mark that also takes marks), so the lint
/// reports them only when asked for every note.
pub fn marks_used_as_bases(gpos: &[u8]) -> Result<Vec<(u16, u16)>> {
    let table = read_fonts::tables::gpos::Gpos::read(FontData::new(gpos)).map_err(|e| read_err("GPOS", e))?;
    let list = table.lookup_list().map_err(|e| read_err("GPOS lookup list", e))?;
    let mut budget = Budget::new();
    let mut found: BTreeSet<(u16, u16)> = BTreeSet::new();
    for (index, lookup) in list.lookups().iter().enumerate() {
        let index = u16::try_from(index).unwrap_or(u16::MAX);
        let lookup = lookup.map_err(|e| read_err("GPOS lookup", e))?;
        let subtables = lookup.subtables().map_err(|e| read_err("GPOS lookup", e))?;
        let mut pairs: Vec<(BTreeSet<u16>, Vec<u16>)> = Vec::new();
        match &subtables {
            PositionSubtables::MarkToBase(t) => {
                for st in t.iter() {
                    let st = st.map_err(|e| read_err("mark-to-base subtable", e))?;
                    let marks = coverage(st.mark_coverage(), &mut budget)?;
                    let bases = coverage(st.base_coverage(), &mut budget)?;
                    pairs.push((marks.into_iter().collect(), bases));
                }
            }
            PositionSubtables::MarkToLig(t) => {
                for st in t.iter() {
                    let st = st.map_err(|e| read_err("mark-to-ligature subtable", e))?;
                    let marks = coverage(st.mark_coverage(), &mut budget)?;
                    let ligatures = coverage(st.ligature_coverage(), &mut budget)?;
                    pairs.push((marks.into_iter().collect(), ligatures));
                }
            }
            _ => {}
        }
        for (marks, bases) in pairs {
            for gid in bases.into_iter().filter(|g| marks.contains(g)) {
                found.insert((index, gid));
                if found.len() >= MAX_MARK_BASE_NOTES {
                    return Ok(found.into_iter().collect());
                }
            }
        }
    }
    Ok(found.into_iter().collect())
}

fn coverage(table: std::result::Result<read_fonts::tables::layout::CoverageTable, read_fonts::ReadError>, budget: &mut Budget) -> Result<Vec<u16>> {
    let table = table.map_err(|e| read_err("coverage table", e))?;
    let mut out = Vec::new();
    for gid in table.iter() {
        budget.charge(1)?;
        out.push(gid.to_u16());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sfnt::Flavour;

    fn name_table(records: &[(u16, u16, u16, u16, &str)]) -> Vec<u8> {
        let records: Vec<NameRecord> = records
            .iter()
            .map(|&(platform, encoding, language, name_id, text)| NameRecord {
                platform,
                encoding,
                language,
                name_id,
                bytes: text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
            })
            .collect();
        write_name(&records, &[]).unwrap()
    }

    #[test]
    fn code_names_replace_matching_records_and_ids_move_above_the_fonts() {
        let font_names = name_table(&[(3, 1, 0x409, 1, "Family"), (3, 1, 0x409, 256, "Old"), (3, 1, 0x409, 300, "Axis")]);
        let mut font = SfntFont::new(Flavour::TrueType);
        font.insert(NAME, font_names.clone());
        assert_eq!(first_free_name_id(&font).unwrap(), 301, "above the highest id the font uses");
        assert_eq!(first_free_name_id(&SfntFont::new(Flavour::TrueType)).unwrap(), 256);

        let code = name_table(&[(3, 1, 0x409, 256, "New"), (1, 0, 0, 257, "Mac")]);
        let merged = merge_names(Some(&font_names), &code).unwrap();
        let (records, tags) = name_records(&merged, "merged").unwrap();
        assert!(tags.is_empty());
        let keys: Vec<(u16, u16, u16, u16)> = records.iter().map(|r| (r.platform, r.encoding, r.language, r.name_id)).collect();
        assert_eq!(keys, vec![(1, 0, 0, 257), (3, 1, 0x409, 1), (3, 1, 0x409, 256), (3, 1, 0x409, 300)], "sorted, 256 replaced");
        let text =
            |r: &NameRecord| String::from_utf16(&r.bytes.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect::<Vec<_>>()).unwrap();
        assert_eq!(text(&records[2]), "New");
    }

    #[test]
    fn os2_fields_raise_the_version_and_keep_the_rest() {
        let mut v1 = vec![0u8; 86];
        v1[1] = 1;
        v1[4..6].copy_from_slice(&400u16.to_be_bytes());
        let mut font = SfntFont::new(Flavour::TrueType);
        font.insert(OS2, v1);
        let values =
            BlockValues { os2: Os2Fields { cap_height: Some(700), typo_ascender: Some(812), ..Os2Fields::default() }, ..BlockValues::default() };
        let report = apply_block_values(&mut font, &values).unwrap();
        assert_eq!(report.changed, vec![OS2]);
        assert_eq!(report.warnings.len(), 0);
        let os2 = font.get(OS2).unwrap();
        assert_eq!(os2.len(), 96, "version 2 length");
        assert_eq!(u16::from_be_bytes([os2[0], os2[1]]), 2);
        assert_eq!(u16::from_be_bytes([os2[4], os2[5]]), 400, "weight kept");
        assert_eq!(i16::from_be_bytes([os2[68], os2[69]]), 812);
        assert_eq!(i16::from_be_bytes([os2[88], os2[89]]), 700);

        let revision = BlockValues { font_revision: Some(2.5), ..BlockValues::default() };
        let report = apply_block_values(&mut font, &revision).unwrap();
        assert!(report.changed.is_empty());
        assert_eq!(report.warnings.len(), 1, "no head table: a warning, not an error");
    }
}
