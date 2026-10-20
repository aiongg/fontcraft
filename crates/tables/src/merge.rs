//! Merging fonts (M12.23; rows `merge`, `font.merge`). Oracles: fontTools pyftmerge (font merge
//! with layout) and AFDKO mergefonts (glyph merge with a map file).
//!
//! The font merge ports fontTools' `merge` package (fontTools 4.66.1, MIT License, Copyright 2013
//! Google, Inc.; see `NOTICE`) table by table:
//!
//! | Table | Rule |
//! |---|---|
//! | glyph order | every glyph of every font, in font order, named as fontTools names them (the `CFF ` charset, `post` versions 1, 2 and 4, else the Unicode `cmap`); a name already used gets `.1`, `.2`…, and so does a name repeated inside one font, which `post` keeps storing under the repeated name |
//! | `glyf`, `loca` | glyphs concatenated, components renumbered; glyphs of every font but the first lose their instructions |
//! | `CFF ` | charstrings concatenated with their subroutines expanded and their widths moved to the first font's Private DICT, which is kept with its Top DICT; `FontBBox` recalculated; then subroutinised again ([`cff`]) |
//! | `hmtx`, `vmtx` | concatenated; `hhea` and `vhea` extents recomputed from the glyphs |
//! | `cmap` | Unicode mappings united, the earlier font keeping a code point; format 4, 12 when needed, 14 for variation sequences |
//! | `GSUB`, `GPOS` | lookups concatenated; features of a script several fonts share merged by tag; unused features and lookups dropped |
//! | `GDEF` | classes, attachment points, carets and mark sets united; variation stores concatenated, each font's variation indices moved to its part (FontCraft's rule: fontTools refuses them) |
//! | `MATH`, `BASE` | glyph-keyed data united, constants and baselines shared ([`base_math`]) |
//! | `head`, `hhea`, `vhea`, `maxp`, `OS/2`, `post` | field by field: extents take the largest value, bits are OR-ed, the rest comes from the first font |
//! | `post` names | the highest version of the fonts: 1 and 3 store none, 2 the merged names, 4 a code point for each merged name |
//! | `maxp`, `head` | then the point, contour and component maxima, the font bounds and `head.flags` bit 1 recalculated from the merged glyphs (`maxp.recalc`), in both forms of merge |
//! | `name`, `cvt `, `fpgm`, `prep`, `gasp` | the first font's |
//! | variation tables | variable TrueType fonts with equal axes stay variable ([`var`]; fontTools has no rule for them) |
//! | anything else | dropped, with a warning |
//!
//! Refused, as fontTools cannot merge them: CID-keyed CFF and `CFF2` fonts, `JSTF` tables
//! (unless dropped), and `BASE` with variation data. The fonts must all have TrueType outlines
//! or all CFF outlines. Variable fonts must all be variable, with the same axes; with `fvar`
//! dropped they merge as their default outlines instead.
//!
//! A code point several fonts map stays with the first; each later font gets a `locl` lookup
//! under its scripts that substitutes its own glyph (it needs a `GSUB` table).
//!
//! The glyph merge (a map for at least one font) takes, for each font, only the glyphs its map
//! lists, under their new names, and every font-wide value from the first font. Layout is
//! dropped unless asked for.

mod base_math;
mod cff;
mod cmap;
mod fixed;
mod glyphs;
mod layout;
mod plan;
mod var;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::limits::{MAX_GLYPHS, MAX_LIST_LINE, MAX_MERGE_FONTS};
use crate::reorder::be::Data;
use crate::sfnt::{Flavour, SfntFont, Tag, tag_str};
use crate::{Result, TablesError};

/// Which fonts' `cmap` and layout tables the result has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutMode {
    /// United `cmap`, `GSUB`, `GPOS` and `GDEF` of every font.
    Merge,
    /// The first font's only.
    Base,
    /// No layout tables; the `cmap` is still united.
    Drop,
}

impl LayoutMode {
    /// The mode named `merge`, `base` or `drop`.
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "merge" => Ok(LayoutMode::Merge),
            "base" => Ok(LayoutMode::Base),
            "drop" => Ok(LayoutMode::Drop),
            other => Err(TablesError::BadOptions(format!("`{other}` is not a layout mode: give merge, base or drop"))),
        }
    }
}

/// What happens when two fonts supply a glyph of the same name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Duplicates {
    /// The earlier glyph is kept; the later font's references point at it.
    First,
    /// The later glyph replaces the earlier one in its place.
    Last,
    /// Refused, unless the two glyphs are identical (then they are shared).
    Error,
    /// Both are kept; the later one gets `.1`, `.2`… appended.
    Rename,
}

impl Duplicates {
    /// The policy named `first`, `last`, `error` or `rename`.
    pub fn parse(text: &str) -> Result<Self> {
        match text {
            "first" => Ok(Duplicates::First),
            "last" => Ok(Duplicates::Last),
            "error" => Ok(Duplicates::Error),
            "rename" => Ok(Duplicates::Rename),
            other => Err(TablesError::BadOptions(format!("`{other}` is not a duplicates policy: give first, last, error or rename"))),
        }
    }
}

/// What [`merge`] does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MergeOptions {
    /// Glyph map (mergefonts' form): per input font, new name → name in that font. Empty merges
    /// every glyph of every font (pyftmerge's form). A font whose own list is empty contributes
    /// every glyph under its own name.
    pub glyph_maps: Vec<Vec<(String, String)>>,
    /// Tables to drop from the result.
    pub drop_tables: Vec<Tag>,
    /// `None`: [`LayoutMode::Merge`] without maps, [`LayoutMode::Drop`] with them.
    pub layout: Option<LayoutMode>,
    /// `None`: [`Duplicates::Rename`] without maps (as pyftmerge), [`Duplicates::First`] with
    /// them (as mergefonts).
    pub duplicates: Option<Duplicates>,
    /// Seconds since 1970 for `head.created` and `head.modified`. `None` keeps the first font's
    /// creation time and the latest modification time of the inputs.
    pub timestamp: Option<i64>,
}

/// One glyph of the result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergedGlyph {
    /// Its name in the result.
    pub name: String,
    /// The input font it comes from (an index into the fonts given).
    pub font: usize,
    /// Its name in that font.
    pub source: String,
}

/// The merged font and what happened on the way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeResult {
    pub font: SfntFont,
    /// The glyph order of the result.
    pub glyphs: Vec<MergedGlyph>,
    /// Clashes that were resolved: renamed or shared glyphs, code points kept by an earlier
    /// font.
    pub conflicts: Vec<String>,
    /// Data left out: dropped tables and `cmap` subtables, substitutions that could not be
    /// added.
    pub warnings: Vec<String>,
}

/// One font made of `fonts` (2 to [`crate::limits::MAX_MERGE_FONTS`]; one is enough with a
/// glyph map).
pub fn merge(fonts: &[SfntFont], options: &MergeOptions) -> Result<SfntFont> {
    merge_with_report(fonts, options).map(|r| r.font)
}

/// A glyph map file: an optional header line `mergefonts [PostScriptName [LanguageGroup]]`
/// (any case), then one `new-name source-name` pair per line. A line with one name takes that
/// glyph under its own name; `#` starts a comment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlyphMap {
    pub ps_name: Option<String>,
    pub language_group: Option<u8>,
    /// (new name, name in the font), in file order.
    pub entries: Vec<(String, String)>,
}

/// Reads a glyph map file ([`GlyphMap`]).
pub fn parse_glyph_map(text: &str) -> Result<GlyphMap> {
    let mut map = GlyphMap::default();
    for (i, raw) in text.lines().enumerate() {
        let line_no = i + 1;
        if raw.len() > MAX_LIST_LINE {
            return Err(TablesError::BadOptions(format!("line {line_no} of the glyph map is longer than {MAX_LIST_LINE} bytes")));
        }
        let line = raw.split('#').next().unwrap_or_default();
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some(first) = words.first() else { continue };
        if map.entries.is_empty() && map.ps_name.is_none() && first.eq_ignore_ascii_case("mergefonts") {
            map.ps_name = words.get(1).map(|s| s.to_string());
            if let Some(group) = words.get(2) {
                map.language_group = match *group {
                    "0" => Some(0),
                    "1" => Some(1),
                    _ => return Err(TablesError::BadOptions(format!("line {line_no} of the glyph map: the language group must be 0 or 1"))),
                };
            }
            continue;
        }
        let entry = match words.as_slice() {
            [one] => (one.to_string(), one.to_string()),
            [new, source] => (new.to_string(), source.to_string()),
            _ => return Err(TablesError::BadOptions(format!("line {line_no} of the glyph map: give a new name and a source name"))),
        };
        if map.entries.len() >= MAX_GLYPHS {
            return Err(TablesError::TooLarge { what: "the glyph map", limit: MAX_GLYPHS });
        }
        map.entries.push(entry);
    }
    Ok(map)
}

/// [`merge`] with the glyph order and the clashes it resolved.
pub fn merge_with_report(fonts: &[SfntFont], options: &MergeOptions) -> Result<MergeResult> {
    let map_form = options.glyph_maps.iter().any(|m| !m.is_empty());
    let least = if map_form { 1 } else { 2 };
    if fonts.len() < least {
        return Err(TablesError::BadOptions(format!("give at least {least} fonts to merge")));
    }
    if fonts.len() > MAX_MERGE_FONTS {
        return Err(TablesError::TooLarge { what: "the number of fonts to merge", limit: MAX_MERGE_FONTS });
    }
    if options.glyph_maps.len() > fonts.len() {
        return Err(TablesError::BadOptions(format!("{} glyph maps were given for {} fonts", options.glyph_maps.len(), fonts.len())));
    }
    let layout = options.layout.unwrap_or(if map_form { LayoutMode::Drop } else { LayoutMode::Merge });
    let duplicates = options.duplicates.unwrap_or(if map_form { Duplicates::First } else { Duplicates::Rename });
    let drop: BTreeSet<Tag> = options.drop_tables.iter().copied().collect();
    let mut kinds: Vec<bool> = Vec::with_capacity(fonts.len());
    for (i, font) in fonts.iter().enumerate() {
        kinds.push(check_input(font, i)?);
    }
    let is_cff = kinds.first().copied().unwrap_or(false);
    if kinds.iter().any(|k| *k != is_cff) {
        return Err(TablesError::BadOptions("the fonts mix TrueType and CFF outlines: convert them to one kind first".into()));
    }
    if layout == LayoutMode::Merge
        && !drop.contains(b"JSTF")
        && let Some(i) = fonts.iter().position(|f| f.get(*b"JSTF").is_some())
    {
        return Err(TablesError::Unsupported(format!("merging 'JSTF' tables (font {}); leave 'JSTF' out with dropTables", i + 1)));
    }
    units_per_em(fonts)?;
    // Leaving `fvar` out merges variable fonts as their default outlines.
    let axes = if drop.contains(b"fvar") { None } else { var::check(fonts)? };
    let variations = axes.as_ref().map(|a| var::Glyphs::read(fonts, a.len())).transpose()?;

    let (names, stored_names): (Vec<Vec<String>>, Vec<_>) = fonts.iter().map(plan::font_glyph_names).collect::<Result<Vec<_>>>()?.into_iter().unzip();
    let expanded: Vec<Vec<u8>> = if is_cff { fonts.iter().enumerate().map(|(i, f)| cff::expand(f, i)).collect::<Result<_>>()? } else { Vec::new() };
    let cff_fonts: Vec<cff::Expanded<'_>> =
        fonts.iter().zip(&expanded).enumerate().map(|(i, (f, e))| cff::Expanded::read(f, e, i)).collect::<Result<_>>()?;
    let sources: Vec<glyphs::Source<'_>> = if is_cff {
        fonts.iter().zip(&cff_fonts).map(|(f, e)| glyphs::Source::read_cff(f, e.charstrings())).collect::<Result<_>>()?
    } else {
        fonts.iter().enumerate().map(|(i, f)| glyphs::Source::read(f, i)).collect::<Result<_>>()?
    };
    let same = |a: (usize, u16), b: (usize, u16)| -> bool {
        match (sources.get(a.0), sources.get(b.0)) {
            (Some(x), Some(y)) => x.glyph(a.1) == y.glyph(b.1) && x.metric(a.1) == y.metric(b.1) && variations.as_ref().is_none_or(|v| v.same(a, b)),
            _ => false,
        }
    };
    let plan = plan::plan(&names, &options.glyph_maps, duplicates, &same)?;
    let mut warnings: Vec<String> = Vec::new();
    let mut conflicts = plan.conflicts.clone();
    for (i, font) in fonts.iter().enumerate() {
        if variations.is_none() && font.get(*b"fvar").is_some() {
            warnings.push(format!("font {} is variable: the result is static, with its default outlines", i + 1));
        }
    }

    let first = fonts.first().ok_or_else(|| TablesError::BadOptions("give fonts to merge".into()))?;
    let mut out = SfntFont::new(if is_cff { Flavour::Cff } else { Flavour::TrueType });
    let mut built = glyphs::build(&sources, &plan, is_cff, &mut warnings)?;
    if is_cff {
        let names: Vec<String> = plan.slots.iter().map(|s| s.name.clone()).collect();
        let table = cff::build(&cff_fonts, &plan, &names, &mut warnings)?;
        built.bounds = table.bounds;
        // fontTools keeps the first font's indexToLocFormat when there is no `loca`.
        built.long_loca = Data::new(*b"head", first.get(*b"head").unwrap_or_default()).i16(50)? != 0;
        out.insert(*b"CFF ", table.table);
    } else {
        out.insert(*b"glyf", built.glyf.clone());
        out.insert(*b"loca", built.loca.clone());
    }

    // Font-wide tables.
    let tables_of = |tag: Tag| -> Vec<&[u8]> { fonts.iter().filter_map(|f| f.get(tag)).collect() };
    let (head, hhea, maxp) = if map_form {
        let one = |tag: Tag| first.get(tag).map(<[u8]>::to_vec).ok_or_else(|| TablesError::MissingTable(tag_str(tag)));
        (one(*b"head")?, one(*b"hhea")?, fixed::maxp(&tables_of(*b"maxp"))?)
    } else {
        (fixed::head(&tables_of(*b"head"))?, fixed::hhea(&tables_of(*b"hhea"))?, fixed::maxp(&tables_of(*b"maxp"))?)
    };
    let head = fixed::finish_head(head, &built, timestamp(fonts, options.timestamp)?)?;
    out.insert(*b"head", head);
    out.insert(*b"hhea", fixed::finish_hea(hhea, &built.h_metrics, &built.bounds, false)?);
    out.insert(*b"maxp", fixed::finish_maxp(maxp, built.count)?);
    out.insert(*b"hmtx", built.hmtx.clone());
    if let Some((vmtx, v_metrics)) = &built.vertical {
        let vhea = if map_form { first.get(*b"vhea").map(<[u8]>::to_vec) } else { Some(fixed::hhea(&tables_of(*b"vhea"))?) };
        if let Some(vhea) = vhea {
            out.insert(*b"vhea", fixed::finish_hea(vhea, v_metrics, &built.bounds, true)?);
            out.insert(*b"vmtx", vmtx.clone());
        }
    }
    if map_form {
        if let Some(os2) = first.get(*b"OS/2") {
            out.insert(*b"OS/2", os2.to_vec());
        }
    } else if fonts.iter().any(|f| f.get(*b"OS/2").is_some()) {
        out.insert(*b"OS/2", fixed::os2(&tables_of(*b"OS/2"), &built.h_metrics)?);
    }
    let post_headers = if map_form { first.get(*b"post").into_iter().collect() } else { tables_of(*b"post") };
    if !post_headers.is_empty() {
        // fontTools stores each glyph under the name its own font's `post` had for it (the
        // mappings of later fonts win), and under its name in the result otherwise.
        let mut stored: HashMap<&str, &str> = HashMap::new();
        if !map_form {
            for (font, mapping) in fonts.iter().zip(&stored_names) {
                if font.get(*b"post").is_some() {
                    stored.extend(mapping.iter().map(|(k, v)| (k.as_str(), v.as_str())));
                }
            }
        }
        let order: Vec<String> = plan.slots.iter().map(|s| s.name.clone()).collect();
        let names: Vec<String> = order.iter().map(|name| stored.get(name.as_str()).map_or_else(|| name.clone(), |n| n.to_string())).collect();
        out.insert(*b"post", fixed::post(&post_headers, &order, &names)?);
    }
    for tag in [*b"name", *b"cvt ", *b"fpgm", *b"prep", *b"gasp"] {
        if let Some(bytes) = first.get(tag) {
            out.insert(tag, bytes.to_vec());
        }
    }

    // cmap and layout.
    let cmap_fonts: Vec<usize> = if layout == LayoutMode::Base { vec![0] } else { (0..fonts.len()).collect() };
    let united = cmap::unite(fonts, &plan, &cmap_fonts, &mut warnings)?;
    for (font, mapped) in &united.kept_by_earlier {
        conflicts.push(format!("{mapped} code points of font {} stay with an earlier font's glyphs", font + 1));
    }
    if fonts.iter().any(|f| f.get(*b"cmap").is_some()) {
        out.insert(*b"cmap", cmap::write(&united)?);
    }
    match layout {
        LayoutMode::Drop => {}
        LayoutMode::Base => {
            layout::base(fonts, &plan, &mut out)?;
            base_math::merge(fonts, &[0], &plan, &drop, &mut out)?;
        }
        LayoutMode::Merge => {
            layout::merge(fonts, &plan, &united.duplicates, &drop, &mut out, &mut warnings)?;
            let all: Vec<usize> = (0..fonts.len()).collect();
            base_math::merge(fonts, &all, &plan, &drop, &mut out)?;
        }
    }
    if let Some(variations) = &variations {
        var::write(fonts, &plan, variations, &mut out, &mut warnings)?;
    }
    // Everything else is dropped, as fontTools drops tables it has no rule for.
    let var_tables: &[Tag] = if variations.is_some() { &var::TABLES } else { &[] };
    let handled: BTreeSet<Tag> = out.tables.keys().copied().chain(HANDLED.iter().copied()).chain(var_tables.iter().copied()).collect();
    let mut dropped: BTreeMap<Tag, Vec<usize>> = BTreeMap::new();
    for (i, font) in fonts.iter().enumerate() {
        for tag in font.tags() {
            if !handled.contains(&tag) && !drop.contains(&tag) {
                dropped.entry(tag).or_default().push(i + 1);
            }
        }
    }
    for (tag, from) in dropped {
        let list: Vec<String> = from.iter().map(ToString::to_string).collect();
        warnings.push(format!("'{}' (font {}) is left out: merge has no rule for it", tag_str(tag), list.join(", ")));
    }
    crate::ttx::read_core::recalc_compiled_maxp(&mut out)?;
    for tag in &drop {
        out.remove(*tag);
    }
    out.order = first.tags().into_iter().filter(|t| out.tables.contains_key(t)).collect();

    let glyphs = plan
        .slots
        .iter()
        .map(|s| MergedGlyph {
            name: s.name.clone(),
            font: s.font,
            source: names.get(s.font).and_then(|n| n.get(usize::from(s.gid))).cloned().unwrap_or_default(),
        })
        .collect();
    Ok(MergeResult { font: out, glyphs, conflicts, warnings })
}

/// Tables merge reads or rebuilds, so they are never reported as dropped.
const HANDLED: [Tag; 22] = [
    *b"head", *b"hhea", *b"maxp", *b"hmtx", *b"glyf", *b"loca", *b"CFF ", *b"vhea", *b"vmtx", *b"OS/2", *b"post", *b"name", *b"cvt ", *b"fpgm",
    *b"prep", *b"gasp", *b"cmap", *b"GSUB", *b"GPOS", *b"GDEF", *b"BASE", *b"MATH",
];

/// Whether the font has CFF outlines (else TrueType ones). `CFF2` fonts are refused: fontTools
/// has no rule for `CFF2`, so its merge of them has no outlines.
fn check_input(font: &SfntFont, index: usize) -> Result<bool> {
    if font.get(*b"CFF2").is_some() {
        return Err(TablesError::Unsupported(format!("merging CFF2 fonts (font {})", index + 1)));
    }
    let cff = font.get(*b"CFF ").is_some();
    let needed: &[Tag] = if cff { &[*b"head", *b"maxp", *b"hhea", *b"hmtx"] } else { &[*b"head", *b"maxp", *b"hhea", *b"hmtx", *b"glyf", *b"loca"] };
    for tag in needed {
        if font.get(*tag).is_none() {
            return Err(TablesError::BadOptions(format!(
                "font {} has no '{}' table; merge needs TrueType or CFF outlines",
                index + 1,
                tag_str(*tag)
            )));
        }
    }
    for tag in [*b"GSUB", *b"GPOS"] {
        if let Some(bytes) = font.get(tag)
            && Data::new(tag, bytes).u16(2)? >= 1
            && Data::new(tag, bytes).u32(10).unwrap_or(0) != 0
        {
            return Err(TablesError::Unsupported(format!("merging '{}' feature variations (font {})", tag_str(tag), index + 1)));
        }
    }
    Ok(cff)
}

fn units_per_em(fonts: &[SfntFont]) -> Result<()> {
    let mut seen: Option<u16> = None;
    for font in fonts {
        let upem = Data::new(*b"head", font.get(*b"head").unwrap_or_default()).u16(18)?;
        match seen {
            Some(u) if u != upem => {
                return Err(TablesError::BadOptions(format!(
                    "the fonts have different units per em ({u} and {upem}); rescale them to one value first"
                )));
            }
            _ => seen = Some(upem),
        }
    }
    Ok(())
}

/// Seconds between 1904 (the sfnt epoch) and 1970.
const EPOCH_1904: i64 = 2_082_844_800;

/// (created, modified) for `head`, as seconds since 1904.
fn timestamp(fonts: &[SfntFont], given: Option<i64>) -> Result<(i64, i64)> {
    if let Some(t) = given {
        let t = t.checked_add(EPOCH_1904).ok_or_else(|| TablesError::BadOptions("the timestamp is out of range".into()))?;
        return Ok((t, t));
    }
    let time = |font: &SfntFont, at: usize| -> Result<i64> {
        let d = Data::new(*b"head", font.get(*b"head").unwrap_or_default());
        Ok((i64::from(d.u32(at)?) << 32) | i64::from(d.u32(at + 4)?))
    };
    let created = fonts.first().map(|f| time(f, 20)).transpose()?.unwrap_or(0);
    let mut modified = 0i64;
    for font in fonts {
        modified = modified.max(time(font, 28)?);
    }
    Ok((created, modified))
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod var_tests;
