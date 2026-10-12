//! The feature list (M12.12; rows `features list`, `layout.listFeatures`). Oracle: spot `-F`.
//!
//! Also the pieces the feature-code writer ([`crate::fea_write`]) shares: a read-fonts view of an
//! [`SfntFont`], the glyph names a compiler sees for the font, and a work budget that caps
//! everything read from layout tables.
//!
//! Glyph names follow the rules fontTools applies when it loads a font, so the names written
//! here are the ones feaLib finds when it compiles text against the same font: the CFF charset
//! for a CFF font (`cid00001` for a CID-keyed one), else `post` names, else names made from the
//! cmap (`uniXXXX`, `uXXXXX`, the ASCII names of the glyph list) and `glyphNNNNN` for the rest.
//! The naming rules follow fontTools' `TTFont.getGlyphOrder` (MIT License, see `NOTICE`).

use std::collections::HashMap;

use read_fonts::tables::gpos::PositionSubtables;
use read_fonts::tables::gsub::SubstitutionSubtables;
use read_fonts::tables::layout::{FeatureList, ScriptList, Subtables};
use read_fonts::types::GlyphId;
use read_fonts::{FontData, ReadError, TableProvider};

use crate::sfnt::{SfntFont, Tag, tag_str};
use crate::{Result, TablesError};

/// Most units of work (records, glyphs, rules) spent reading one font's layout tables.
pub const MAX_LAYOUT_WORK: usize = 50_000_000;

/// One feature of one language system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureEntry {
    /// `GSUB` or `GPOS`.
    pub table: Tag,
    pub script: Tag,
    /// `dflt` for the script's default language system.
    pub language: Tag,
    pub feature: Tag,
    /// The lookup indices the feature names.
    pub lookups: Vec<u16>,
}

/// What one lookup of a lookup list is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LookupKind {
    /// The lookup type, with an extension lookup's type read through (1 to 8 for `GSUB`, 1 to 8
    /// for `GPOS`); 0 for an extension lookup without subtables.
    pub lookup_type: u16,
    /// Stored through Extension lookups.
    pub extension: bool,
    /// The lookup flag.
    pub flag: u16,
    pub subtables: usize,
}

/// Every feature of every language system, in table, script, language and feature order.
pub fn list_features(font: &SfntFont) -> Result<Vec<FeatureEntry>> {
    let provider = Provider(font);
    let mut budget = Budget::new();
    let mut out = Vec::new();
    for table in [GSUB, GPOS] {
        let Some((scripts, features)) = layout_lists(&provider, table)? else { continue };
        for system in language_systems(&scripts, &mut budget)? {
            let indices = system.required.into_iter().chain(system.features.iter().copied());
            for index in indices {
                let (feature, lookups) = feature_at(&features, index, &mut budget)?;
                out.push(FeatureEntry { table, script: system.script, language: system.language, feature, lookups });
            }
        }
    }
    Ok(out)
}

/// The type, extension flag, lookup flag and subtable count of every lookup of `table` (`GSUB`
/// or `GPOS`), by lookup index. A font without the table gives an empty list.
pub fn lookup_kinds(font: &SfntFont, table: Tag) -> Result<Vec<LookupKind>> {
    let provider = Provider(font);
    if font.get(table).is_none() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    if table == GSUB {
        let gsub = provider.gsub().map_err(|e| read_err("GSUB", e))?;
        let list = gsub.lookup_list().map_err(|e| read_err("GSUB lookup list", e))?;
        for lookup in list.lookups().iter() {
            let lookup = lookup.map_err(|e| read_err("GSUB lookup", e))?;
            let subtables = lookup.subtables().map_err(|e| read_err("GSUB lookup", e))?;
            let (lookup_type, extension, count) = gsub_kind(&subtables);
            out.push(LookupKind { lookup_type, extension, flag: lookup.lookup_flag().to_bits(), subtables: count });
        }
    } else if table == GPOS {
        let gpos = provider.gpos().map_err(|e| read_err("GPOS", e))?;
        let list = gpos.lookup_list().map_err(|e| read_err("GPOS lookup list", e))?;
        for lookup in list.lookups().iter() {
            let lookup = lookup.map_err(|e| read_err("GPOS lookup", e))?;
            let subtables = lookup.subtables().map_err(|e| read_err("GPOS lookup", e))?;
            let (lookup_type, extension, count) = gpos_kind(&subtables);
            out.push(LookupKind { lookup_type, extension, flag: lookup.lookup_flag().to_bits(), subtables: count });
        }
    } else {
        return Err(TablesError::BadOptions(format!("`{}` has no lookups: give GSUB or GPOS", tag_str(table))));
    }
    Ok(out)
}

fn ext<'a, T, E>(s: &Subtables<'a, T, E>) -> (bool, usize)
where
    T: read_fonts::FontRead<'a, Args = ()> + 'a,
    E: read_fonts::tables::layout::ExtensionLookup<'a, T> + 'a,
{
    (matches!(s, Subtables::Extension(_)), s.len())
}

/// The effective type, whether it is stored through extensions, and the subtable count.
pub(crate) fn gsub_kind(s: &SubstitutionSubtables) -> (u16, bool, usize) {
    let (t, (e, n)) = match s {
        SubstitutionSubtables::Single(s) => (1, ext(s)),
        SubstitutionSubtables::Multiple(s) => (2, ext(s)),
        SubstitutionSubtables::Alternate(s) => (3, ext(s)),
        SubstitutionSubtables::Ligature(s) => (4, ext(s)),
        SubstitutionSubtables::Contextual(s) => (5, ext(s)),
        SubstitutionSubtables::ChainContextual(s) => (6, ext(s)),
        SubstitutionSubtables::Reverse(s) => (8, ext(s)),
        SubstitutionSubtables::EmptyExtension => (0, (true, 0)),
    };
    (t, e, n)
}

/// The effective type, whether it is stored through extensions, and the subtable count.
pub(crate) fn gpos_kind(s: &PositionSubtables) -> (u16, bool, usize) {
    let (t, (e, n)) = match s {
        PositionSubtables::Single(s) => (1, ext(s)),
        PositionSubtables::Pair(s) => (2, ext(s)),
        PositionSubtables::Cursive(s) => (3, ext(s)),
        PositionSubtables::MarkToBase(s) => (4, ext(s)),
        PositionSubtables::MarkToLig(s) => (5, ext(s)),
        PositionSubtables::MarkToMark(s) => (6, ext(s)),
        PositionSubtables::Contextual(s) => (7, ext(s)),
        PositionSubtables::ChainContextual(s) => (8, ext(s)),
        PositionSubtables::EmptyExtension => (0, (true, 0)),
    };
    (t, e, n)
}

// ── shared with the feature-code writer ──────────────────────────────────────────────────────

pub(crate) const GSUB: Tag = *b"GSUB";
pub(crate) const GPOS: Tag = *b"GPOS";
pub(crate) const GDEF: Tag = *b"GDEF";
pub(crate) const BASE: Tag = *b"BASE";
pub(crate) const DFLT_LANG: Tag = *b"dflt";

/// A read-fonts view of the tables of an [`SfntFont`].
pub(crate) struct Provider<'a>(pub(crate) &'a SfntFont);

impl<'a> TableProvider<'a> for Provider<'a> {
    fn data_for_tag(&self, tag: read_fonts::types::Tag) -> Option<FontData<'a>> {
        self.0.get(tag.into_bytes()).map(FontData::new)
    }
}

/// A read error in a table, as a message that names where it happened.
pub(crate) fn read_err(what: &str, e: ReadError) -> TablesError {
    TablesError::Invalid(format!("the {what} cannot be read: {e}"))
}

/// The work left for one run; every record, glyph and rule read is charged before it is used.
pub(crate) struct Budget {
    left: usize,
}

impl Budget {
    pub(crate) fn new() -> Self {
        Self { left: MAX_LAYOUT_WORK }
    }

    pub(crate) fn charge(&mut self, units: usize) -> Result<()> {
        match self.left.checked_sub(units) {
            Some(left) => {
                self.left = left;
                Ok(())
            }
            None => {
                self.left = 0;
                Err(TablesError::TooLarge { what: "the layout tables (units of work to read them)", limit: MAX_LAYOUT_WORK })
            }
        }
    }
}

/// One language system of a script list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LanguageSystem {
    pub(crate) script: Tag,
    pub(crate) language: Tag,
    /// The required feature's index.
    pub(crate) required: Option<u16>,
    /// The other feature indices, in stored order.
    pub(crate) features: Vec<u16>,
}

/// The script and feature lists of `GSUB` or `GPOS`; `None` when the font lacks the table.
pub(crate) fn layout_lists<'a>(p: &Provider<'a>, table: Tag) -> Result<Option<(ScriptList<'a>, FeatureList<'a>)>> {
    if p.0.get(table).is_none() {
        return Ok(None);
    }
    let name = tag_str(table);
    let lists = if table == GSUB {
        let t = p.gsub().map_err(|e| read_err(&name, e))?;
        (t.script_list(), t.feature_list())
    } else {
        let t = p.gpos().map_err(|e| read_err(&name, e))?;
        (t.script_list(), t.feature_list())
    };
    let scripts = lists.0.map_err(|e| read_err(&format!("{name} script list"), e))?;
    let features = lists.1.map_err(|e| read_err(&format!("{name} feature list"), e))?;
    Ok(Some((scripts, features)))
}

/// Every language system: per script in stored order, the default one (as `dflt`) first.
pub(crate) fn language_systems(scripts: &ScriptList, budget: &mut Budget) -> Result<Vec<LanguageSystem>> {
    let data = scripts.offset_data();
    let mut out = Vec::new();
    for record in scripts.script_records() {
        budget.charge(1)?;
        let script_tag = record.script_tag().into_bytes();
        let script = record.script(data).map_err(|e| read_err(&format!("script '{}'", tag_str(script_tag)), e))?;
        let mut push = |language: Tag, sys: read_fonts::tables::layout::LangSys, budget: &mut Budget| -> Result<()> {
            let indices = sys.feature_indices();
            budget.charge(indices.len().saturating_add(1))?;
            let required = Some(sys.required_feature_index()).filter(|i| *i != 0xFFFF);
            out.push(LanguageSystem { script: script_tag, language, required, features: indices.iter().map(|i| i.get()).collect() });
            Ok(())
        };
        if let Some(sys) = script.default_lang_sys() {
            let sys = sys.map_err(|e| read_err(&format!("default language system of '{}'", tag_str(script_tag)), e))?;
            push(DFLT_LANG, sys, budget)?;
        }
        let sdata = script.offset_data();
        for lang in script.lang_sys_records() {
            budget.charge(1)?;
            let language = lang.lang_sys_tag().into_bytes();
            let sys =
                lang.lang_sys(sdata).map_err(|e| read_err(&format!("language system '{}' of '{}'", tag_str(language), tag_str(script_tag)), e))?;
            push(language, sys, budget)?;
        }
    }
    Ok(out)
}

/// The tag and lookup indices of feature `index`.
pub(crate) fn feature_at(features: &FeatureList, index: u16, budget: &mut Budget) -> Result<(Tag, Vec<u16>)> {
    let record = features
        .feature_records()
        .get(usize::from(index))
        .ok_or_else(|| TablesError::invalid(format!("a language system names feature {index}, which the feature list does not have")))?;
    let tag = record.feature_tag().into_bytes();
    let feature = record.feature(features.offset_data()).map_err(|e| read_err(&format!("feature '{}'", tag_str(tag)), e))?;
    let lookups = feature.lookup_list_indices();
    budget.charge(lookups.len().saturating_add(1))?;
    Ok((tag, lookups.iter().map(|i| i.get()).collect()))
}

/// The number of glyphs: `maxp`, else the CFF charstring count, else 0.
pub(crate) fn glyph_count(p: &Provider) -> usize {
    p.maxp().map(|m| usize::from(m.num_glyphs())).unwrap_or(0)
}

/// The glyph names a compiler sees for this font, by glyph id (see the module notes).
pub(crate) fn glyph_names(p: &Provider, budget: &mut Budget) -> Result<Vec<String>> {
    let count = glyph_count(p);
    budget.charge(count)?;
    let mut names: Vec<Option<String>> = vec![None; count];
    let mut from_cff = false;
    if let Ok(cff) = p.cff() {
        if let Some(charset) = cff.charset(0) {
            for (gid, slot) in names.iter_mut().enumerate() {
                let Ok(gid) = u32::try_from(gid) else { break };
                if let Some(sid) = charset.string_id(GlyphId::new(gid)) {
                    *slot = cff.string(sid).map(|b| String::from_utf8_lossy(b).into_owned());
                }
            }
            from_cff = true;
        } else if let Some(cids) = cid_charset(&cff) {
            for (gid, slot) in names.iter_mut().enumerate() {
                let Ok(g) = u32::try_from(gid) else { break };
                if gid == 0 {
                    *slot = Some(".notdef".into());
                } else if let Some(cid) = cids.string_id(GlyphId::new(g)) {
                    *slot = Some(format!("cid{:05}", cid.to_u16()));
                }
            }
            from_cff = true;
        }
    }
    if !from_cff {
        match p.post() {
            Ok(post) if post.version() != read_fonts::types::Version16Dot16::VERSION_3_0 => {
                for (gid, slot) in names.iter_mut().enumerate() {
                    let Ok(g) = u16::try_from(gid) else { break };
                    *slot = post.glyph_name(read_fonts::types::GlyphId16::new(g)).map(str::to_string);
                }
            }
            _ => cmap_names(p, &mut names, budget)?,
        }
    }
    // Missing names, then repeated ones, as fontTools makes them.
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::with_capacity(count);
    for (gid, name) in names.into_iter().enumerate() {
        let name = match name.filter(|n| !n.is_empty()) {
            Some(n) => n,
            None if gid == 0 => ".notdef".to_string(),
            None => format!("glyph{gid:05}"),
        };
        let uses = seen.entry(name.clone()).or_insert(0);
        *uses += 1;
        out.push(if *uses > 1 { format!("{name}#{}", *uses - 1) } else { name });
    }
    Ok(out)
}

fn cid_charset<'a>(cff: &read_fonts::tables::cff::Cff<'a>) -> Option<read_fonts::ps::cff::charset::Charset<'a>> {
    use read_fonts::ps::cff::dict::{Entry, entries};
    let top = cff.top_dicts().get(0)?;
    let mut charset = None;
    let mut glyphs = None;
    let mut cid = false;
    for entry in entries(top, None).flatten() {
        match entry {
            Entry::Charset(offset) => charset = Some(offset),
            Entry::CharstringsOffset(offset) => {
                let data = cff.offset_data().split_off(offset)?;
                glyphs = read_fonts::ps::cff::index::Index::new(data.as_bytes(), false).ok().map(|i| i.count());
            }
            Entry::Ros { .. } => cid = true,
            _ => {}
        }
    }
    if !cid {
        return None;
    }
    read_fonts::ps::cff::charset::Charset::new(cff.offset_data(), charset?, glyphs?)
}

/// Names from the Unicode cmap: the lowest code point of each glyph.
fn cmap_names(p: &Provider, names: &mut [Option<String>], budget: &mut Budget) -> Result<()> {
    let Ok(cmap) = p.cmap() else { return Ok(()) };
    let mut lowest: HashMap<u32, u32> = HashMap::new();
    let data = cmap.offset_data();
    for record in cmap.encoding_records() {
        let unicode = matches!((record.platform_id() as u16, record.encoding_id()), (0, _) | (3, 1) | (3, 10));
        if !unicode {
            continue;
        }
        let Ok(subtable) = record.subtable(data) else { continue };
        for (cp, gid) in subtable.iter() {
            budget.charge(1)?;
            let low = lowest.entry(gid.to_u32()).or_insert(cp);
            *low = (*low).min(cp);
        }
    }
    let mut uses: HashMap<String, usize> = HashMap::new();
    for (gid, slot) in names.iter_mut().enumerate() {
        let Some(cp) = u32::try_from(gid).ok().and_then(|g| lowest.get(&g)) else { continue };
        let base = match ascii_name(*cp) {
            Some(n) => n.to_string(),
            None if *cp <= 0xFFFF => format!("uni{cp:04X}"),
            None => format!("u{cp:X}"),
        };
        let n = uses.entry(base.clone()).or_insert(0);
        *n += 1;
        *slot = Some(if *n > 1 { format!("{base}.alt{}", *n - 1) } else { base });
    }
    Ok(())
}

/// The glyph-list name of a printable ASCII character.
fn ascii_name(cp: u32) -> Option<&'static str> {
    const PUNCT: [&str; 32] = [
        "space",
        "exclam",
        "quotedbl",
        "numbersign",
        "dollar",
        "percent",
        "ampersand",
        "quotesingle",
        "parenleft",
        "parenright",
        "asterisk",
        "plus",
        "comma",
        "hyphen",
        "period",
        "slash",
        "colon",
        "semicolon",
        "less",
        "equal",
        "greater",
        "question",
        "at",
        "bracketleft",
        "backslash",
        "bracketright",
        "asciicircum",
        "underscore",
        "grave",
        "braceleft",
        "bar",
        "braceright",
    ];
    const DIGITS: [&str; 10] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];
    const UPPER: [&str; 26] =
        ["A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z"];
    const LOWER: [&str; 26] =
        ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z"];
    let i = |base: u32| usize::try_from(cp.wrapping_sub(base)).ok();
    match cp {
        0x20..=0x2F => PUNCT.get(i(0x20)?).copied(),
        0x30..=0x39 => DIGITS.get(i(0x30)?).copied(),
        0x3A..=0x40 => PUNCT.get(i(0x3A)? + 16).copied(),
        0x41..=0x5A => UPPER.get(i(0x41)?).copied(),
        0x5B..=0x60 => PUNCT.get(i(0x5B)? + 23).copied(),
        0x61..=0x7A => LOWER.get(i(0x61)?).copied(),
        0x7B..=0x7D => PUNCT.get(i(0x7B)? + 29).copied(),
        0x7E => Some("asciitilde"),
        _ => None,
    }
}
