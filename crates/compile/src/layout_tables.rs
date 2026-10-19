//! The feature code's table blocks applied to the tables the compiler builds, and `OS/2`
//! `usMaxContext` from the compiled lookups (M8.8; rows `compile.feaTableOverrides`,
//! `compile.nameFeaOverrides`, `compile.os2MaxContext`, the `BASE` table, `fea.table.*`).
//!
//! The blocks are applied after the table builders, as fontc applies them (`fontbe/src/head.rs`,
//! `metrics_and_limits.rs`, `vertical_metrics.rs`, `os2.rs`, `name.rs`, `stat.rs` and
//! `features.rs`, 1.0.0, Apache-2.0, see `NOTICE`):
//!
//! | Block | What it changes |
//! |---|---|
//! | `head` | `fontRevision` |
//! | `hhea`, `vhea` | each metric the block writes; the others keep the computed value, as feaLib keeps them |
//! | `OS/2` | each field the block writes, an explicit zero included; an optical size raises the table to version 5 |
//! | `name` | each record replaces the record with the same platform, encoding, language and ID and is added otherwise; the records stay sorted |
//! | `BASE`, `STAT` | the whole table; the code's `STAT` replaces the one a variable font builds |
//!
//! The code's name IDs of 256 and above are moved before the compile to start above the highest
//! ID the source's names use ([`crate::info::first_free_name_id`]), so they add names and never
//! replace one of the source's.
//!
//! [`max_context`] is ported from fontTools' `otlLib/maxContextCalc.py` (fontTools 4.66.1, MIT
//! License, see `NOTICE`), which fontc ports too (`fontbe/src/os2/max_context.rs`).

use read_fonts::tables::gpos::{Gpos, PositionSubtables};
use read_fonts::tables::gsub::{Gsub, SubstitutionSubtables};
use read_fonts::tables::layout::{ChainedSequenceContext, ExtensionLookup, SequenceContext, Subtables};
use read_fonts::tables::name::Encoding;
use read_fonts::{FontData, FontRead, ReadError};
use write_fonts::tables::head::Head;
use write_fonts::tables::hhea::Hhea;
use write_fonts::tables::os2::Os2;
use write_fonts::tables::vhea::Vhea;
use write_fonts::types::{FWord, Fixed};

use crate::info::{MAX_NAME_CHARS, MAX_NAME_RECORDS, NameRecord};

/// The most units of work [`max_context`] spends: one per subtable, ligature and rule read.
pub const MAX_CONTEXT_WORK: usize = 50_000_000;

/// Why [`max_context`] could not measure the lookups.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ContextError {
    #[error("the {what} cannot be read: {message}")]
    Unreadable { what: &'static str, message: String },
    #[error("the layout tables need more than {0} units of work to read")]
    TooLarge(usize),
}

/// The work left for one [`max_context`] call.
struct Work {
    left: usize,
    limit: usize,
}

impl Work {
    fn charge(&mut self) -> Result<(), ContextError> {
        self.left = self.left.checked_sub(1).ok_or(ContextError::TooLarge(self.limit))?;
        Ok(())
    }
}

fn unreadable(what: &'static str) -> impl Fn(ReadError) -> ContextError {
    move |e| ContextError::Unreadable { what, message: e.to_string() }
}

/// The longest glyph sequence any lookup of `gsub` or `gpos` looks at (`OS/2` `usMaxContext`),
/// as fontTools computes it: 1 for single, multiple and alternate substitution and single
/// positioning, 2 for pair positioning, the component count of a ligature, the input length of a
/// contextual rule, the input plus look-ahead length of a chaining rule, 1 plus the look-ahead
/// length of a reverse chaining rule. Attachment lookups count nothing; no lookups give 0.
/// Reading stops with an error after `work` units (see [`MAX_CONTEXT_WORK`]).
pub fn max_context(gsub: Option<&[u8]>, gpos: Option<&[u8]>, work: usize) -> Result<u16, ContextError> {
    let mut work = Work { left: work, limit: work };
    let mut max = 0u32;
    if let Some(bytes) = gsub {
        let table = Gsub::read(FontData::new(bytes)).map_err(unreadable("GSUB table"))?;
        let list = table.lookup_list().map_err(unreadable("GSUB lookup list"))?;
        for lookup in list.lookups().iter() {
            let lookup = lookup.map_err(unreadable("GSUB lookup"))?;
            let subtables = lookup.subtables().map_err(unreadable("GSUB lookup"))?;
            max = max.max(gsub_context(&subtables, &mut work)?);
        }
    }
    if let Some(bytes) = gpos {
        let table = Gpos::read(FontData::new(bytes)).map_err(unreadable("GPOS table"))?;
        let list = table.lookup_list().map_err(unreadable("GPOS lookup list"))?;
        for lookup in list.lookups().iter() {
            let lookup = lookup.map_err(unreadable("GPOS lookup"))?;
            let subtables = lookup.subtables().map_err(unreadable("GPOS lookup"))?;
            max = max.max(gpos_context(&subtables, &mut work)?);
        }
    }
    Ok(u16::try_from(max).unwrap_or(u16::MAX))
}

/// The largest of `f` over the subtables, each charged to the work.
fn each<'a, T, E>(
    s: &Subtables<'a, T, E>,
    what: &'static str,
    work: &mut Work,
    mut f: impl FnMut(T, &mut Work) -> Result<u32, ContextError>,
) -> Result<u32, ContextError>
where
    T: FontRead<'a, Args = ()> + 'a,
    E: ExtensionLookup<'a, T> + 'a,
{
    let mut max = 0;
    for t in s.iter() {
        work.charge()?;
        let t = t.map_err(unreadable(what))?;
        max = max.max(f(t, work)?);
    }
    Ok(max)
}

fn gsub_context(s: &SubstitutionSubtables, work: &mut Work) -> Result<u32, ContextError> {
    let what = "GSUB subtable";
    match s {
        SubstitutionSubtables::Single(t) => each(t, what, work, |_, _| Ok(1)),
        SubstitutionSubtables::Multiple(t) => each(t, what, work, |_, _| Ok(1)),
        SubstitutionSubtables::Alternate(t) => each(t, what, work, |_, _| Ok(1)),
        SubstitutionSubtables::Ligature(t) => each(t, what, work, |st, work| {
            let mut max = 0;
            for set in st.ligature_sets().iter() {
                let set = set.map_err(unreadable("ligature set"))?;
                for lig in set.ligatures().iter() {
                    work.charge()?;
                    let lig = lig.map_err(unreadable("ligature"))?;
                    max = max.max(u32::from(lig.component_count()));
                }
            }
            Ok(max)
        }),
        SubstitutionSubtables::Contextual(t) => each(t, what, work, |st, work| sequence_context(&st, work)),
        SubstitutionSubtables::ChainContextual(t) => each(t, what, work, |st, work| chained_context(&st, work)),
        SubstitutionSubtables::Reverse(t) => each(t, what, work, |st, _| Ok(1 + u32::from(st.lookahead_glyph_count()))),
        SubstitutionSubtables::EmptyExtension => Ok(0),
    }
}

fn gpos_context(s: &PositionSubtables, work: &mut Work) -> Result<u32, ContextError> {
    let what = "GPOS subtable";
    match s {
        PositionSubtables::Single(t) => each(t, what, work, |_, _| Ok(1)),
        PositionSubtables::Pair(t) => each(t, what, work, |_, _| Ok(2)),
        PositionSubtables::Contextual(t) => each(t, what, work, |st, work| sequence_context(&st, work)),
        PositionSubtables::ChainContextual(t) => each(t, what, work, |st, work| chained_context(&st, work)),
        PositionSubtables::Cursive(_)
        | PositionSubtables::MarkToBase(_)
        | PositionSubtables::MarkToLig(_)
        | PositionSubtables::MarkToMark(_)
        | PositionSubtables::EmptyExtension => Ok(0),
    }
}

/// The longest input sequence of a contextual subtable.
fn sequence_context(st: &SequenceContext, work: &mut Work) -> Result<u32, ContextError> {
    let mut max = 0u32;
    match st {
        SequenceContext::Format1(t) => {
            for set in t.seq_rule_sets().iter().flatten() {
                let set = set.map_err(unreadable("contextual rule set"))?;
                for rule in set.seq_rules().iter() {
                    work.charge()?;
                    let rule = rule.map_err(unreadable("contextual rule"))?;
                    max = max.max(u32::from(rule.glyph_count()));
                }
            }
        }
        SequenceContext::Format2(t) => {
            for set in t.class_seq_rule_sets().iter().flatten() {
                let set = set.map_err(unreadable("contextual rule set"))?;
                for rule in set.class_seq_rules().iter() {
                    work.charge()?;
                    let rule = rule.map_err(unreadable("contextual rule"))?;
                    max = max.max(u32::from(rule.glyph_count()));
                }
            }
        }
        SequenceContext::Format3(t) => max = u32::from(t.glyph_count()),
    }
    Ok(max)
}

/// The longest input plus look-ahead sequence of a chaining subtable.
fn chained_context(st: &ChainedSequenceContext, work: &mut Work) -> Result<u32, ContextError> {
    let mut max = 0u32;
    match st {
        ChainedSequenceContext::Format1(t) => {
            for set in t.chained_seq_rule_sets().iter().flatten() {
                let set = set.map_err(unreadable("chaining rule set"))?;
                for rule in set.chained_seq_rules().iter() {
                    work.charge()?;
                    let rule = rule.map_err(unreadable("chaining rule"))?;
                    max = max.max(u32::from(rule.input_glyph_count()) + u32::from(rule.lookahead_glyph_count()));
                }
            }
        }
        ChainedSequenceContext::Format2(t) => {
            for set in t.chained_class_seq_rule_sets().iter().flatten() {
                let set = set.map_err(unreadable("chaining rule set"))?;
                for rule in set.chained_class_seq_rules().iter() {
                    work.charge()?;
                    let rule = rule.map_err(unreadable("chaining rule"))?;
                    max = max.max(u32::from(rule.input_glyph_count()) + u32::from(rule.lookahead_glyph_count()));
                }
            }
        }
        ChainedSequenceContext::Format3(t) => max = u32::from(t.input_glyph_count()) + u32::from(t.lookahead_glyph_count()),
    }
    Ok(max)
}

/// `usMaxContext` of the compiled layout tables. A table the compiler cannot read back stops the
/// export, as it would stop any reader of the font.
pub(crate) fn os2_max_context(compiled: &fontcraft_fea::Compiled) -> Result<u16, crate::CompileError> {
    max_context(compiled.gsub.as_deref(), compiled.gpos.as_deref(), MAX_CONTEXT_WORK)
        .map_err(|e| crate::CompileError::Table { table: "OS/2", message: format!("usMaxContext: {e}") })
}

/// The `table head` block: `fontRevision`. A revision that does not fit a 16.16 number is left
/// out with a warning.
pub(crate) fn apply_head(values: &fontcraft_fea::HeadValues, head: &mut Head, warnings: &mut Vec<String>) {
    let Some(revision) = values.font_revision else { return };
    let fits = revision.is_finite() && (-32_768.0..32_768.0).contains(&revision);
    if fits {
        head.font_revision = Fixed::from_f64(revision);
    } else {
        warnings.push(format!("feature code: the FontRevision {revision} of the head table block does not fit the table and was left out"));
    }
}

/// The `table hhea` block: each metric it writes.
pub(crate) fn apply_hhea(values: &fontcraft_fea::MetricsValues, hhea: &mut Hhea) {
    set_fword(&mut hhea.ascender, values.ascender);
    set_fword(&mut hhea.descender, values.descender);
    set_fword(&mut hhea.line_gap, values.line_gap);
    if let Some(v) = values.caret_offset {
        hhea.caret_offset = v;
    }
}

/// The `table vhea` block: each metric it writes.
pub(crate) fn apply_vhea(values: &fontcraft_fea::MetricsValues, vhea: &mut Vhea) {
    set_fword(&mut vhea.ascender, values.ascender);
    set_fword(&mut vhea.descender, values.descender);
    set_fword(&mut vhea.line_gap, values.line_gap);
    if let Some(v) = values.caret_offset {
        vhea.caret_offset = v;
    }
}

/// Whether a `table vhea` block sets anything: a font without vertical metrics then gets a
/// warning, as it has no `vhea` table to write the values into.
pub(crate) fn sets_vhea(values: &fontcraft_fea::MetricsValues) -> bool {
    *values != fontcraft_fea::MetricsValues::default()
}

fn set_fword(slot: &mut FWord, value: Option<i16>) {
    if let Some(v) = value {
        *slot = FWord::new(v);
    }
}

/// The `table OS/2` block: each field it writes. An optical size raises the table to version 5
/// (the writer picks the version from the fields present); the other size, when the block leaves
/// it out, is written as 0, as feaLib leaves the fields a raised version adds.
pub(crate) fn apply_os2(values: &fontcraft_fea::Os2Values, os2: &mut Os2) {
    let o = values;
    if let Some(v) = o.weight_class {
        os2.us_weight_class = v;
    }
    if let Some(v) = o.width_class {
        os2.us_width_class = v;
    }
    if let Some(v) = o.fs_type {
        os2.fs_type = v;
    }
    if let Some(v) = o.family_class {
        os2.s_family_class = v;
    }
    if let Some(v) = o.panose {
        os2.panose_10 = v;
    }
    if let Some([r1, r2, r3, r4]) = o.unicode_range {
        os2.ul_unicode_range_1 = r1;
        os2.ul_unicode_range_2 = r2;
        os2.ul_unicode_range_3 = r3;
        os2.ul_unicode_range_4 = r4;
    }
    if let Some(v) = o.vendor_id {
        os2.ach_vend_id = v;
    }
    if let Some(v) = o.win_ascent {
        os2.us_win_ascent = v;
    }
    if let Some(v) = o.win_descent {
        os2.us_win_descent = v;
    }
    if let Some([c1, c2]) = o.code_page_range {
        os2.ul_code_page_range_1 = Some(c1);
        os2.ul_code_page_range_2 = Some(c2);
    }
    if let Some(v) = o.x_height {
        os2.sx_height = Some(v);
    }
    if let Some(v) = o.cap_height {
        os2.s_cap_height = Some(v);
    }
    if let Some(v) = o.typo_ascender {
        os2.s_typo_ascender = v;
    }
    if let Some(v) = o.typo_descender {
        os2.s_typo_descender = v;
    }
    if let Some(v) = o.typo_line_gap {
        os2.s_typo_line_gap = v;
    }
    if o.lower_optical_point_size.is_some() || o.upper_optical_point_size.is_some() {
        os2.us_lower_optical_point_size = Some(o.lower_optical_point_size.or(os2.us_lower_optical_point_size).unwrap_or(0));
        os2.us_upper_optical_point_size = Some(o.upper_optical_point_size.or(os2.us_upper_optical_point_size).unwrap_or(0));
    }
}

/// The code's `name` table (records already moved above the source's IDs) merged into
/// `records`: a record with the same platform, encoding, language and ID is replaced, others are
/// added, and the list is sorted. `protected` IDs (a variable font's axis and instance names,
/// which are allocated after the layout stage) are kept: a code record with one of them is left
/// out with a warning. A record in an encoding the writer cannot store is left out too.
pub(crate) fn merge_names(records: &mut Vec<NameRecord>, code: &[u8], protected: &[u16], warnings: &mut Vec<String>) {
    let name = match read_fonts::tables::name::Name::read(FontData::new(code)) {
        Ok(n) => n,
        Err(e) => {
            warnings.push(format!("feature code: the names of the name table block were left out: {e}"));
            return;
        }
    };
    let storage = name.string_data();
    let mut clashed = Vec::new();
    for r in name.name_record() {
        let (platform, encoding, language, id) = (r.platform_id(), r.encoding_id(), r.language_id(), r.name_id().to_u16());
        if Encoding::new(platform, encoding) == Encoding::Unknown {
            warnings
                .push(format!("feature code: name {id} for platform {platform}, encoding {encoding} was left out: the encoding is not supported"));
            continue;
        }
        if protected.contains(&id) {
            if !clashed.contains(&id) {
                clashed.push(id);
            }
            continue;
        }
        let Ok(text) = r.string(storage) else {
            warnings.push(format!("feature code: name {id} was left out: its string lies outside the table"));
            continue;
        };
        let mut text: String = text.chars().collect();
        if text.chars().count() > MAX_NAME_CHARS {
            text = text.chars().take(MAX_NAME_CHARS).collect();
            warnings.push(format!("feature code: name {id} is longer than {MAX_NAME_CHARS} characters and was cut"));
        }
        let key = (platform, encoding, language, id);
        records.retain(|n| (n.platform, n.encoding, n.language, n.id) != key);
        if records.len() >= MAX_NAME_RECORDS {
            warnings.push(format!("feature code: the font has {MAX_NAME_RECORDS} name records, so name {id} was left out"));
            continue;
        }
        records.push(NameRecord { platform, encoding, language, id, text });
    }
    if !clashed.is_empty() {
        let ids: Vec<String> = clashed.iter().map(u16::to_string).collect();
        warnings.push(format!(
            "feature code: the names with ID {} were left out: the variable font's axis and instance names use those IDs",
            ids.join(", ")
        ));
    }
    records.sort_by_key(|r| (r.platform, r.encoding, r.language, r.id));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: u16, text: &str) -> NameRecord {
        NameRecord { platform: 3, encoding: 1, language: 0x409, id, text: text.into() }
    }

    fn name_table(records: &[(u16, &str)]) -> Vec<u8> {
        // A compiled name table holds its records sorted.
        let mut records = records.to_vec();
        records.sort_by_key(|(id, _)| *id);
        let name = write_fonts::tables::name::Name::new(
            records
                .iter()
                .map(|(id, text)| {
                    write_fonts::tables::name::NameRecord::new(3, 1, 0x409, write_fonts::types::NameId::new(*id), text.to_string().into())
                })
                .collect(),
        );
        write_fonts::dump_table(&name).unwrap()
    }

    #[test]
    fn code_names_replace_matching_records_keep_the_others_and_sort() {
        let mut records = vec![record(1, "Family"), record(4, "Family Regular"), record(9, "Designer")];
        let code = name_table(&[(9, "Joachim Müller-Lancé"), (4, "set from FEA name table"), (300, "Swash")]);
        let mut warnings = Vec::new();
        merge_names(&mut records, &code, &[], &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        let got: Vec<(u16, &str)> = records.iter().map(|r| (r.id, r.text.as_str())).collect();
        assert_eq!(got, [(1, "Family"), (4, "set from FEA name table"), (9, "Joachim Müller-Lancé"), (300, "Swash")]);
    }

    #[test]
    fn code_names_never_replace_a_variable_fonts_axis_names() {
        let mut records = vec![record(256, "Weight")];
        let code = name_table(&[(256, "Swash")]);
        let mut warnings = Vec::new();
        merge_names(&mut records, &code, &[256], &mut warnings);
        assert_eq!(records, [record(256, "Weight")]);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    #[test]
    fn metrics_blocks_change_only_the_fields_they_write() {
        let mut hhea = Hhea { ascender: FWord::new(800), descender: FWord::new(-200), line_gap: FWord::new(90), ..Default::default() };
        let values = fontcraft_fea::MetricsValues { ascender: Some(950), caret_offset: Some(20), ..Default::default() };
        apply_hhea(&values, &mut hhea);
        assert_eq!((hhea.ascender.to_i16(), hhea.descender.to_i16(), hhea.line_gap.to_i16(), hhea.caret_offset), (950, -200, 90, 20));
    }

    #[test]
    fn an_optical_size_alone_writes_the_other_as_zero() {
        // The compiler's OS/2 is version 4, so every field below version 5 is present.
        let mut os2 = Os2 {
            ul_code_page_range_1: Some(0),
            ul_code_page_range_2: Some(0),
            sx_height: Some(0),
            s_cap_height: Some(0),
            us_default_char: Some(0),
            us_break_char: Some(32),
            us_max_context: Some(0),
            ..Default::default()
        };
        let values = fontcraft_fea::Os2Values { upper_optical_point_size: Some(240), fs_type: Some(0), ..Default::default() };
        os2.fs_type = 8;
        apply_os2(&values, &mut os2);
        assert_eq!((os2.us_lower_optical_point_size, os2.us_upper_optical_point_size, os2.fs_type), (Some(0), Some(240), 0));
        assert!(write_fonts::dump_table(&os2).is_ok(), "a version 5 table with both sizes");
    }

    #[test]
    fn a_revision_out_of_range_is_left_out() {
        let mut head = Head { font_revision: Fixed::from_f64(1.0), ..Default::default() };
        let mut warnings = Vec::new();
        apply_head(&fontcraft_fea::HeadValues { font_revision: Some(1e9) }, &mut head, &mut warnings);
        assert_eq!((head.font_revision, warnings.len()), (Fixed::from_f64(1.0), 1));
        apply_head(&fontcraft_fea::HeadValues { font_revision: Some(2.5) }, &mut head, &mut warnings);
        assert_eq!(head.font_revision, Fixed::from_f64(2.5));
    }

    /// A font with a three-glyph ligature, a stylistic set with a UI name, a source name with
    /// ID 300 and, when `blocks`, the table blocks of the reference compilers' test source.
    fn block_font(blocks: bool) -> fontcraft_model::Font {
        use fontcraft_model::plist::Value;
        use fontcraft_model::{Font, Param, Snippet};
        let mut f = Font::new("Block Sans");
        for (name, cp) in [("a", Some(0x61)), ("f", Some(0x66)), ("i", Some(0x69)), ("f_f_i", None), ("a.ss01", None)] {
            if f.glyph(name).is_none() {
                f.add_glyph(name, cp.into_iter().collect(), 500.0).unwrap();
            }
        }
        std::sync::Arc::make_mut(&mut f.info).params.0.push(Param::new("Name Table Entry", Value::String("300; Source name".into())));
        let features = std::sync::Arc::make_mut(&mut f.features);
        let mut prefix = String::from("languagesystem DFLT dflt;\n");
        if blocks {
            prefix.push_str(concat!(
                "table head { FontRevision 2.5; } head;\n",
                "table hhea { Ascender 950; CaretOffset 20; } hhea;\n",
                "table OS/2 { FSType 0; TypoAscender 950; winAscent 1100; WeightClass 700; } OS/2;\n",
                "table name { nameid 9 \"Joachim Müller-Lancé\"; nameid 4 \"set from FEA name table\"; } name;\n",
            ));
        }
        features.prefixes.push(Snippet { name: "Blocks".into(), code: prefix, ..Default::default() });
        features.features.push(Snippet { name: "liga".into(), code: "sub f f i by f_f_i;".into(), ..Default::default() });
        features.features.push(Snippet {
            name: "ss01".into(),
            code: "featureNames { name \"Alternate a\"; };\nsub a by a.ss01;".into(),
            ..Default::default()
        });
        f
    }

    #[test]
    fn table_blocks_and_the_longest_context_reach_the_compiled_font() {
        use skrifa::raw::TableProvider;
        use skrifa::string::StringId;
        use skrifa::{FontRef, MetadataProvider};
        let names = |font: &FontRef, id: u16| -> Vec<String> { font.localized_strings(StringId::new(id)).map(|s| s.to_string()).collect() };

        let plain = crate::compile_ttf(&block_font(false), &crate::Options::default()).unwrap();
        let plain = FontRef::new(&plain.bytes).unwrap();
        let out = crate::compile_ttf(&block_font(true), &crate::Options::default()).unwrap();
        let font = FontRef::new(&out.bytes).unwrap();

        assert_eq!(font.head().unwrap().font_revision(), Fixed::from_f64(2.5));
        let (hhea, before) = (font.hhea().unwrap(), plain.hhea().unwrap());
        assert_eq!((hhea.ascender().to_i16(), hhea.caret_offset()), (950, 20));
        assert_eq!((hhea.descender(), hhea.line_gap()), (before.descender(), before.line_gap()), "fields the block leaves out");
        let (os2, before) = (font.os2().unwrap(), plain.os2().unwrap());
        assert_eq!((os2.fs_type(), os2.s_typo_ascender(), os2.us_win_ascent(), os2.us_weight_class()), (0, 950, 1100, 700));
        assert_eq!(os2.s_typo_descender(), before.s_typo_descender());
        assert_eq!(os2.us_max_context(), Some(3), "the three-glyph ligature");
        assert_eq!(plain.os2().unwrap().us_max_context(), Some(3));

        assert_eq!(names(&font, 9), ["Joachim Müller-Lancé"]);
        assert_eq!(names(&font, 4), ["set from FEA name table"]);
        // The stylistic set's name goes above the source's ID 300 and leaves it alone.
        assert_eq!(names(&font, 300), ["Source name"]);
        assert_eq!(names(&font, 301), ["Alternate a"]);
        assert!(names(&font, 256).is_empty());
    }

    #[test]
    fn unreadable_lookups_are_an_error_and_none_give_zero() {
        assert_eq!(max_context(None, None, MAX_CONTEXT_WORK), Ok(0));
        assert!(matches!(max_context(Some(&[0, 1, 0]), None, MAX_CONTEXT_WORK), Err(ContextError::Unreadable { .. })));
    }
}
