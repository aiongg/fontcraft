//! Width checks (M12.49): the advance widths of base glyphs, marks, spaces, mathematical signs,
//! figures, dashes, ligatures and accented letters in one compiled font, and the advances and
//! metrics keys of a source. Runners take the context of their scope ([`crate::BinaryCtx`],
//! [`crate::GlyphCtx`]), the thresholds and a [`crate::Sink`]; their registry entries are in
//! `registry/metrics_widths.rs`.
//!
//! The binary rules follow FontBakery 1.1.0 (https://github.com/fonttools/fontbakery, Apache
//! License 2.0, Copyright the FontBakery authors): `base_has_width`, `typenetwork/marks_width`,
//! `whitespace_widths`, `notofonts/hmtx/whitespace_advances`, `math_signs_width`,
//! `tnum_glyphs_equal_widths`, `notofonts/hmtx/encoded_latin_digits` and
//! `notofonts/hmtx/comma_period`; and the single-face tests 17, 22, 23 and 26 of comparefamily in
//! AFDKO 5.0.1 (https://github.com/adobe-type-tools/afdko, Apache License 2.0, Copyright Adobe).
//! Each check's row in `spec/inventory/21-qa-checks.toml` names its sources and where this port
//! departs from them.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use fontcraft_geom::{BezPath, Shape};
use fontcraft_model::glyphdata::GeneralCategory;
use read_fonts::tables::gsub::{SingleSubst, SubstitutionLookup, SubstitutionSubtables};
use read_fonts::types::{GlyphId, Tag};
use read_fonts::{FontRef, TableProvider};
use skrifa::MetadataProvider;
use skrifa::outline::{DrawSettings, OutlineGlyphCollection, OutlinePen};
use skrifa::prelude::{LocationRef, Size};
use skrifa::{GlyphNameSource, GlyphNames};

use crate::data::chars::MATH_SIGNS;
use crate::data::glyphs::general_category;
use crate::{BinaryCtx, GlyphCtx, Hit, Location, Params, Sink, Threshold, num};

/// Most cmap mappings one check walks: one per Unicode code point.
const MAX_CMAP_MAPPINGS: usize = 0x11_0000;
/// Most glyph names one message lists.
const MAX_NAMES_IN_MESSAGE: usize = 20;
/// Most ligature substitutions `check.metrics.ligatureWidths` reads.
const MAX_LIGATURES: usize = 100_000;
/// Most GSUB lookups one feature contributes.
const MAX_FEATURE_LOOKUPS: usize = 4_096;
/// Most subtables read from one lookup.
const MAX_SUBTABLES: usize = 4_096;
/// Most named instances `check.metrics.tabularWidths` measures the tabular figures at.
const MAX_INSTANCES: usize = 256;

/// `check.metrics.ligatureWidths` and `check.metrics.accentedWidths`: how far a width or
/// sidebearing may depart from the glyph it is compared with.
pub(crate) const TOLERANCE: Threshold = Threshold {
    name: "tolerance",
    default: 0.0,
    min: 0.0,
    max: 1_000.0,
    doc: "Font units a width or sidebearing may differ from the compared glyph's without a warning.",
};

// ---------------------------------------------------------------------------------------------
// Shared readers
// ---------------------------------------------------------------------------------------------

/// The advance of every glyph at the default location, as fontTools' `hmtx` lists them: glyphs
/// past the long metrics take the last long advance.
fn advances(font: &FontRef<'_>) -> Option<Vec<u16>> {
    let count = font.maxp().ok()?.num_glyphs();
    let hmtx = font.hmtx().ok()?;
    let long = hmtx.h_metrics();
    let last = long.last()?.advance();
    Some((0..usize::from(count)).map(|i| long.get(i).map_or(last, |m| m.advance())).collect())
}

fn advance(adv: &[u16], gid: GlyphId) -> Option<u16> {
    adv.get(usize::try_from(gid.to_u32()).ok()?).copied()
}

/// The glyph names of one compiled font: its own names (`post` or CFF), else the names of the
/// source it was compiled from, else `glyph N`.
struct Names {
    by_gid: Vec<String>,
    by_name: HashMap<String, GlyphId>,
}

impl Names {
    fn new(ctx: &BinaryCtx<'_>, count: usize) -> Self {
        let own = GlyphNames::new(&ctx.font);
        let use_own = own.source() != GlyphNameSource::Synthesized;
        let mut by_gid = Vec::with_capacity(count);
        let mut by_name = HashMap::with_capacity(count);
        for i in 0..count {
            let gid = GlyphId::new(u32::try_from(i).unwrap_or(u32::MAX));
            let name = if use_own {
                own.get(gid).map(|n| n.as_str().to_string())
            } else {
                ctx.source.and_then(|s| s.glyph(gid.to_u32())).map(str::to_string)
            };
            let name = name.unwrap_or_else(|| format!("glyph {i}"));
            by_name.entry(name.clone()).or_insert(gid);
            by_gid.push(name);
        }
        Self { by_gid, by_name }
    }

    fn name(&self, gid: GlyphId) -> String {
        usize::try_from(gid.to_u32()).ok().and_then(|i| self.by_gid.get(i)).cloned().unwrap_or_else(|| format!("glyph {}", gid.to_u32()))
    }

    fn gid(&self, name: &str) -> Option<GlyphId> {
        self.by_name.get(name).copied()
    }
}

/// The glyph a finding is about, named as the source names it when the font was compiled from one.
fn glyph_place(ctx: &BinaryCtx<'_>, names: &Names, gid: GlyphId) -> Location {
    let glyph = ctx.source.and_then(|s| s.glyph(gid.to_u32())).map_or_else(|| names.name(gid), str::to_string);
    Location { glyph, table: Some("hmtx".into()), ..Location::default() }
}

/// Names for a message: at most [`MAX_NAMES_IN_MESSAGE`], then how many more.
fn list(names: &[String]) -> String {
    let shown: Vec<&str> = names.iter().take(MAX_NAMES_IN_MESSAGE).map(String::as_str).collect();
    let mut s = shown.join(", ");
    if names.len() > MAX_NAMES_IN_MESSAGE {
        s.push_str(&format!(" and {} more", names.len() - MAX_NAMES_IN_MESSAGE));
    }
    s
}

/// The glyphs `GDEF` classes as marks (class 3).
fn gdef_marks(font: &FontRef<'_>) -> HashSet<u32> {
    font.gdef()
        .ok()
        .and_then(|g| g.glyph_class_def())
        .and_then(Result::ok)
        .map(|cd| cd.iter().filter(|(_, class)| *class == 3).map(|(g, _)| g.to_u32()).collect())
        .unwrap_or_default()
}

/// Every cmap mapping of the best subtable, in code point order.
fn mappings(font: &FontRef<'_>) -> Vec<(u32, GlyphId)> {
    font.charmap().mappings().take(MAX_CMAP_MAPPINGS).collect()
}

/// The advance of the glyph mapped to `cp`.
fn char_advance(font: &FontRef<'_>, adv: &[u16], cp: u32) -> Option<(GlyphId, u16)> {
    let gid = font.charmap().map(cp)?;
    Some((gid, advance(adv, gid)?))
}

/// Python's `round`: halves go to the even neighbour.
fn py_round(v: f64) -> f64 {
    v.round_ties_even()
}

// ---------------------------------------------------------------------------------------------
// Base glyphs, marks and spaces
// ---------------------------------------------------------------------------------------------

/// Characters that may draw nothing and take no width: separators, format and control
/// characters, non-spacing marks and private-use characters (FontBakery's `is_space`).
fn may_be_empty(cp: u32, category: Option<GeneralCategory>) -> bool {
    use GeneralCategory as G;
    matches!(category, Some(G::Zs | G::Zl | G::Zp | G::Cf | G::Mn | G::Cc))
        || (0xE000..=0xF8FF).contains(&cp)
        || (0xF_0000..=0xF_FFFD).contains(&cp)
        || (0x10_0000..=0x10_FFFD).contains(&cp)
}

/// `check.metrics.baseGlyphWidth`: an encoded glyph with a zero advance that is neither a mark in
/// `GDEF` nor a character that may draw nothing. A glyph mapped from several code points is
/// judged by the highest.
pub(crate) fn base_glyph_width(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(adv) = advances(&ctx.font) else {
        sink.skip("the font has no readable 'hmtx' or 'maxp' table");
        return;
    };
    if general_category(0x41).is_none() {
        sink.skip("the Unicode character data cannot be read");
        return;
    }
    let mut code_of: BTreeMap<u32, u32> = BTreeMap::new();
    for (cp, gid) in mappings(&ctx.font) {
        code_of.insert(gid.to_u32(), cp);
    }
    let marks = gdef_marks(&ctx.font);
    let names = Names::new(ctx, adv.len());
    for (&gid, &cp) in &code_of {
        let id = GlyphId::new(gid);
        if cp == 0 || advance(&adv, id) != Some(0) || marks.contains(&gid) || may_be_empty(cp, general_category(cp)) {
            continue;
        }
        let name = names.name(id);
        let msg = format!("{name} (U+{cp:04X}) is a base glyph with an advance width of 0.");
        sink.push(Hit::new("zero-width", msg).data("codePoint", f64::from(cp)).place(glyph_place(ctx, &names, id)));
    }
}

/// `check.metrics.markWidths`: encoded non-spacing marks (Mn, Me) must have no advance, spacing
/// modifiers (Sk, Lm) must have one.
pub(crate) fn mark_widths(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(adv) = advances(&ctx.font) else {
        sink.skip("the font has no readable 'hmtx' or 'maxp' table");
        return;
    };
    if general_category(0x41).is_none() {
        sink.skip("the Unicode character data cannot be read");
        return;
    }
    let names = Names::new(ctx, adv.len());
    let mut seen: HashSet<u32> = HashSet::new();
    for (cp, gid) in mappings(&ctx.font) {
        let Some(width) = advance(&adv, gid) else { continue };
        let category = general_category(cp);
        let non_spacing = matches!(category, Some(GeneralCategory::Mn | GeneralCategory::Me));
        let spacing = matches!(category, Some(GeneralCategory::Sk | GeneralCategory::Lm));
        if !((non_spacing && width != 0) || (spacing && width == 0)) || !seen.insert(gid.to_u32()) {
            continue;
        }
        let name = names.name(gid);
        let place = glyph_place(ctx, &names, gid);
        if non_spacing {
            let msg = format!("{name} (U+{cp:04X}) is a combining mark but has an advance width of {width}.");
            sink.push(Hit::new("non-spacing-not-zero", msg).data("width", f64::from(width)).place(place));
        } else {
            let msg = format!("{name} (U+{cp:04X}) is a spacing modifier but has no advance width.");
            sink.push(Hit::new("spacing-zero", msg).without_fix().place(place));
        }
    }
}

/// The monospace test of FontBakery's `glyph_metrics_stats`: with more than 80% of printable
/// ASCII encoded, 80% of those glyphs (zero widths left out) share one advance; otherwise the
/// encoded letters, marks, numbers, punctuation, symbols and spaces that are not `GDEF` marks
/// use at most two non-zero advances.
fn seems_monospaced(font: &FontRef<'_>, adv: &[u16], map: &[(u32, GlyphId)]) -> bool {
    let ascii: BTreeSet<u32> = map.iter().filter(|(cp, _)| (0x20..0x7F).contains(cp)).map(|(_, g)| g.to_u32()).collect();
    let encoded_ascii = map.iter().filter(|(cp, _)| (0x20..0x7F).contains(cp)).count();
    if encoded_ascii as f64 > 0.8 * 95.0 {
        let widths: Vec<u16> = ascii.iter().filter_map(|g| advance(adv, GlyphId::new(*g))).filter(|w| *w != 0).collect();
        let mut counts: BTreeMap<u16, usize> = BTreeMap::new();
        for w in &widths {
            *counts.entry(*w).or_default() += 1;
        }
        let top = counts.values().copied().max().unwrap_or(0);
        return top > 0 && top as f64 >= widths.len() as f64 * 0.8;
    }
    use GeneralCategory as G;
    let marks = gdef_marks(font);
    let mut widths = BTreeSet::new();
    for (cp, gid) in map {
        let relevant = match general_category(*cp) {
            Some(G::Zs) => true,
            Some(G::Zl | G::Zp | G::Cc | G::Cf | G::Cs | G::Co | G::Cn) | None => false,
            Some(_) => true,
        };
        if !relevant || marks.contains(&gid.to_u32()) {
            continue;
        }
        if let Some(w) = advance(adv, *gid).filter(|w| *w != 0) {
            widths.insert(w);
            if widths.len() > 2 {
                return false;
            }
        }
    }
    true
}

/// The advance a space character should have (Noto's rules), or the range it should fall in.
enum Expected {
    Exact(f64),
    Range(f64, f64),
}

/// `check.metrics.whitespaceWidths`: the space has an advance and the no-break space the same
/// one. In a proportional font the other encoded spaces also get their conventional advances
/// (a warning): tab and no-break space as the space, en and em quads and spaces, the third-,
/// fourth- and sixth-em spaces, the figure space as the zero, the punctuation space as the
/// period, the thin and hair spaces within their ranges, and the zero width space 0.
pub(crate) fn whitespace_widths(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (Some(adv), Ok(head)) = (advances(&ctx.font), ctx.font.head()) else {
        sink.skip("the font has no readable 'hmtx', 'maxp' or 'head' table");
        return;
    };
    let (Some((space_gid, space)), Some((nbsp_gid, nbsp))) = (char_advance(&ctx.font, &adv, 0x20), char_advance(&ctx.font, &adv, 0xA0)) else {
        sink.skip("the font does not map both the space and the no-break space");
        return;
    };
    let names = Names::new(ctx, adv.len());
    if space == 0 {
        let msg = format!("The space ({}) has no advance width.", names.name(space_gid));
        sink.push(Hit::new("zero-space", msg).without_fix().place(glyph_place(ctx, &names, space_gid)));
    } else if space != nbsp {
        let msg = format!(
            "The no-break space ({}) is {nbsp} units wide but the space ({}) is {space}; both should have the same advance.",
            names.name(nbsp_gid),
            names.name(space_gid)
        );
        sink.push(
            Hit::new("different-widths", msg)
                .data("space", f64::from(space))
                .data("noBreakSpace", f64::from(nbsp))
                .place(glyph_place(ctx, &names, nbsp_gid)),
        );
    }
    let map = mappings(&ctx.font);
    if general_category(0x41).is_none() || seems_monospaced(&ctx.font, &adv, &map) {
        return;
    }
    let em = f64::from(head.units_per_em());
    let of = |cp: u32| char_advance(&ctx.font, &adv, cp).map(|(_, w)| f64::from(w));
    let expectations: [(u32, Option<Expected>); 14] = [
        (0x0009, of(0x20).map(Expected::Exact)),
        (0x00A0, of(0x20).map(Expected::Exact)),
        (0x2000, Some(Expected::Exact(em / 2.0))),
        (0x2001, Some(Expected::Exact(em))),
        (0x2002, Some(Expected::Exact(em / 2.0))),
        (0x2003, Some(Expected::Exact(em))),
        (0x2004, Some(Expected::Exact(em / 3.0))),
        (0x2005, Some(Expected::Exact(em / 4.0))),
        (0x2006, Some(Expected::Exact(em / 6.0))),
        (0x2007, of(0x30).map(Expected::Exact)),
        (0x2008, of(0x2E).map(Expected::Exact)),
        (0x2009, Some(Expected::Range(em / 6.0, em / 5.0))),
        (0x200A, Some(Expected::Range(em / 16.0, em / 10.0))),
        (0x200B, Some(Expected::Exact(0.0))),
    ];
    for (cp, expected) in expectations {
        let (Some((gid, got)), Some(expected)) = (char_advance(&ctx.font, &adv, cp), expected) else { continue };
        let got = f64::from(got);
        let wanted = match expected {
            Expected::Exact(w) if got != py_round(w) => num(py_round(w)),
            Expected::Range(lo, hi) if got < lo.floor() || got > hi.ceil() => format!("{} to {}", num(lo.floor()), num(hi.ceil())),
            _ => continue,
        };
        let msg = format!("{} (U+{cp:04X}) is {} units wide; it should be {wanted}.", names.name(gid), num(got));
        sink.push(Hit::new("bad-whitespace-advance", msg).data("width", got).without_fix().place(glyph_place(ctx, &names, gid)));
    }
}

// ---------------------------------------------------------------------------------------------
// Signs and figures
// ---------------------------------------------------------------------------------------------

/// `check.metrics.mathSignWidths`: the encoded mathematical signs of FontBakery's list share one
/// advance. The most common width wins, the first in list order on a tie; a most common width of
/// 0 passes, as in FontBakery. Each glyph of another width is reported.
pub(crate) fn math_sign_widths(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(adv) = advances(&ctx.font) else {
        sink.skip("the font has no readable 'hmtx' or 'maxp' table");
        return;
    };
    let charmap = ctx.font.charmap();
    // Widths in the order they are first met, with their glyphs.
    let mut groups: Vec<(u16, Vec<GlyphId>)> = Vec::new();
    let mut seen: HashSet<u32> = HashSet::new();
    for &cp in MATH_SIGNS {
        let Some(gid) = charmap.map(cp) else { continue };
        let Some(width) = advance(&adv, gid) else { continue };
        if !seen.insert(gid.to_u32()) {
            continue;
        }
        match groups.iter_mut().find(|(w, _)| *w == width) {
            Some((_, glyphs)) => glyphs.push(gid),
            None => groups.push((width, vec![gid])),
        }
    }
    let mut common: Option<(u16, usize)> = None;
    for (w, glyphs) in &groups {
        if common.is_none_or(|(_, n)| glyphs.len() > n) {
            common = Some((*w, glyphs.len()));
        }
    }
    let Some((common, count)) = common.filter(|(w, _)| *w != 0) else { return };
    if groups.len() < 2 {
        return;
    }
    let names = Names::new(ctx, adv.len());
    for (width, glyphs) in groups.iter().filter(|(w, _)| *w != common) {
        for gid in glyphs {
            let msg = format!("{} is {width} units wide; {count} of the math signs are {common}.", names.name(*gid));
            let hit = Hit::new("width-outliers", msg).data("width", f64::from(*width)).data("expected", f64::from(common));
            sink.push(hit.place(glyph_place(ctx, &names, *gid)));
        }
    }
}

/// The lookups of the GSUB features with one of `tags`, in lookup-list order; `None` when the
/// font has no such feature.
fn feature_lookups<'a>(font: &FontRef<'a>, tags: &[Tag]) -> Option<Vec<SubstitutionLookup<'a>>> {
    let gsub = font.gsub().ok()?;
    let features = gsub.feature_list().ok()?;
    let mut indices: BTreeSet<u16> = BTreeSet::new();
    let mut found = false;
    for rec in features.feature_records() {
        if !tags.contains(&rec.feature_tag()) {
            continue;
        }
        found = true;
        if let Ok(feature) = rec.feature(features.offset_data()) {
            indices.extend(feature.lookup_list_indices().iter().take(MAX_FEATURE_LOOKUPS).map(|i| i.get()));
        }
    }
    if !found {
        return None;
    }
    let Ok(list) = gsub.lookup_list() else { return Some(Vec::new()) };
    Some(indices.into_iter().filter_map(|i| list.lookups().get(usize::from(i)).ok()).collect())
}

/// What one lookup's single substitutions turn `gid` into; `gid` itself when none covers it.
fn substitute(lookup: &SubstitutionLookup<'_>, gid: GlyphId) -> GlyphId {
    let Ok(SubstitutionSubtables::Single(subtables)) = lookup.subtables() else { return gid };
    for sub in subtables.iter().filter_map(Result::ok).take(MAX_SUBTABLES) {
        match sub {
            SingleSubst::Format1(t) => {
                if t.coverage().ok().and_then(|c| c.get(gid)).is_none() {
                    continue;
                }
                let out = (i64::from(gid.to_u32()) + i64::from(t.delta_glyph_id())).rem_euclid(65_536);
                return GlyphId::new(u32::try_from(out).unwrap_or(0));
            }
            SingleSubst::Format2(t) => {
                let Some(index) = t.coverage().ok().and_then(|c| c.get(gid)) else { continue };
                if let Some(s) = t.substitute_glyph_ids().get(usize::from(index)) {
                    return GlyphId::from(s.get());
                }
            }
        }
    }
    gid
}

/// The width groups of `glyphs` (name, width), most popular last, as comparefamily's
/// `MakeWidthTable` sorts them: by count, then width.
fn width_table(glyphs: &[(String, u16)]) -> Vec<(usize, u16, Vec<String>)> {
    let mut by_width: BTreeMap<u16, Vec<String>> = BTreeMap::new();
    for (name, width) in glyphs {
        by_width.entry(*width).or_default().push(name.clone());
    }
    let mut table: Vec<(usize, u16, Vec<String>)> = by_width.into_iter().map(|(w, n)| (n.len(), w, n)).collect();
    table.sort();
    table
}

/// The name groups comparefamily's test 17 expects to be tabular.
const TABULAR_GROUPS: [(&str, &[&str]); 3] = [
    ("tabular lining", &["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "zero.slash"]),
    (
        "tabular old-style",
        &[
            "zero.taboldstyle",
            "one.taboldstyle",
            "two.taboldstyle",
            "three.taboldstyle",
            "four.taboldstyle",
            "five.taboldstyle",
            "six.taboldstyle",
            "seven.taboldstyle",
            "eight.taboldstyle",
            "nine.taboldstyle",
        ],
    ),
    (
        "tabular numerator",
        &[
            "zero.numerator",
            "one.numerator",
            "two.numerator",
            "three.numerator",
            "four.numerator",
            "five.numerator",
            "six.numerator",
            "seven.numerator",
            "eight.numerator",
            "nine.numerator",
        ],
    ),
];

/// `check.metrics.tabularWidths`, three rules:
/// - with a GSUB `tnum` feature, the encoded figures 0-9 after its single substitutions share one
///   advance, at the default location of a static font and at every named instance of a variable
///   one (FontBakery `tnum_glyphs_equal_widths`, which shapes the figures instead);
/// - in each name group of comparefamily's test 17 with ten or more glyphs present, once more than
///   half the group shares a width, every glyph of another width is reported;
/// - the glyph `figurespace` has the width of `one.tab`, else of `one.taboldstyle` (test 26).
pub(crate) fn tabular_widths(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(adv) = advances(&ctx.font) else {
        sink.skip("the font has no readable 'hmtx' or 'maxp' table");
        return;
    };
    let names = Names::new(ctx, adv.len());
    tnum_widths(ctx, &names, sink);
    for (group, members) in TABULAR_GROUPS {
        let glyphs: Vec<(String, u16)> = names
            .by_gid
            .iter()
            .enumerate()
            .filter(|(_, n)| members.contains(&n.as_str()))
            .filter_map(|(i, n)| Some((n.clone(), *adv.get(i)?)))
            .collect();
        let table = width_table(&glyphs);
        let Some((top, common, _)) = table.last() else { continue };
        if glyphs.len() <= 9 || (*top as f64) <= glyphs.len() as f64 * 0.5 || table.len() < 2 {
            continue;
        }
        for (_, width, outliers) in table.iter().take(table.len().saturating_sub(1)) {
            for name in outliers {
                let Some(gid) = names.gid(name) else { continue };
                let msg = format!("{name} is {width} units wide; {top} of the {} {group} figures are {common}.", glyphs.len());
                let hit = Hit::new("group-unequal", msg).data("width", f64::from(*width)).data("expected", f64::from(*common));
                sink.push(hit.place(glyph_place(ctx, &names, gid)));
            }
        }
    }
    if let Some(space) = names.gid("figurespace") {
        let tab = ["one.tab", "one.taboldstyle"].into_iter().find_map(|t| names.gid(t).map(|g| (t, g)));
        if let (Some(width), Some((tab, tab_gid))) = (advance(&adv, space), tab)
            && let Some(tab_width) = advance(&adv, tab_gid)
            && width != tab_width
        {
            let msg = format!("figurespace is {width} units wide but the tabular figure {tab} is {tab_width}.");
            let hit = Hit::new("figure-space", msg).data("width", f64::from(width)).data("expected", f64::from(tab_width));
            sink.push(hit.place(glyph_place(ctx, &names, space)));
        }
    }
}

/// The `tnum` rule of [`tabular_widths`].
fn tnum_widths(ctx: &BinaryCtx<'_>, names: &Names, sink: &mut Sink) {
    let Some(lookups) = feature_lookups(&ctx.font, &[Tag::new(b"tnum")]) else { return };
    let charmap = ctx.font.charmap();
    let figures: Vec<GlyphId> = (0x30u32..=0x39).filter_map(|cp| charmap.map(cp)).map(|g| lookups.iter().fold(g, |g, l| substitute(l, g))).collect();
    if figures.is_empty() {
        return;
    }
    let axes = ctx.font.axes();
    let instances = ctx.font.named_instances();
    let mut places: Vec<(Option<String>, skrifa::instance::Location)> = Vec::new();
    if axes.is_empty() || instances.is_empty() {
        places.push((None, skrifa::instance::Location::default()));
    }
    for instance in instances.iter().take(MAX_INSTANCES) {
        let coords: Vec<String> = axes.iter().zip(instance.user_coords()).map(|(a, v)| format!("{}={}", a.tag(), num(f64::from(v)))).collect();
        places.push((Some(coords.join(", ")), instance.location()));
    }
    for (label, location) in &places {
        let metrics = ctx.font.glyph_metrics(Size::unscaled(), LocationRef::from(location));
        let mut by_width: BTreeMap<i64, Vec<String>> = BTreeMap::new();
        for gid in &figures {
            let Some(w) = metrics.advance_width(*gid).filter(|w| w.is_finite()) else { continue };
            by_width.entry(f64::from(w).round() as i64).or_default().push(names.name(*gid));
        }
        if by_width.len() < 2 {
            continue;
        }
        let groups: Vec<String> = by_width.iter().map(|(w, n)| format!("{w}: {}", list(n))).collect();
        let at = label.as_ref().map(|l| format!(" at the instance {l}")).unwrap_or_default();
        let msg = format!("The figures of the tnum feature do not share one advance{at}: {}.", groups.join("; "));
        sink.push(Hit::new("tnum-unequal", msg).place(Location { table: Some("GSUB".into()), feature: Some("tnum".into()), ..Location::default() }));
    }
}

/// `check.metrics.digitWidths`: the encoded figures 0-9 have the advance of the zero, and the
/// comma that of the period. Skipped when the font maps neither a zero nor both comma and period.
pub(crate) fn digit_widths(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(adv) = advances(&ctx.font) else {
        sink.skip("the font has no readable 'hmtx' or 'maxp' table");
        return;
    };
    let names = Names::new(ctx, adv.len());
    let zero = char_advance(&ctx.font, &adv, 0x30);
    let comma = char_advance(&ctx.font, &adv, 0x2C);
    let period = char_advance(&ctx.font, &adv, 0x2E);
    if zero.is_none() && (comma.is_none() || period.is_none()) {
        sink.skip("the font maps neither the figure zero nor both comma and period");
        return;
    }
    if let Some((_, zero)) = zero {
        for d in 1..=9u32 {
            let cp = 0x30 + d;
            match char_advance(&ctx.font, &adv, cp) {
                None => sink.push(Hit::new("missing-digit", format!("The font maps the figure 0 but not the figure {d}."))),
                Some((gid, w)) if w != zero => {
                    let msg = format!("The figure {d} ({}) is {w} units wide but the zero is {zero}.", names.name(gid));
                    sink.push(
                        Hit::new("bad-digit-width", msg)
                            .data("width", f64::from(w))
                            .data("expected", f64::from(zero))
                            .place(glyph_place(ctx, &names, gid)),
                    );
                }
                Some(_) => {}
            }
        }
    }
    if let (Some((comma_gid, c)), Some((_, p))) = (comma, period)
        && c != p
    {
        let msg = format!("The comma ({}) is {c} units wide but the period is {p}.", names.name(comma_gid));
        sink.push(
            Hit::new("comma-period", msg).data("width", f64::from(c)).data("expected", f64::from(p)).place(glyph_place(ctx, &names, comma_gid)),
        );
    }
}

/// Glyphs whose names give their width in ems (comparefamily test 26): the en dash and the dot
/// leaders are half an em, the em dash one em. comparefamily's list spells the en dash with a
/// trailing space, so it never checks it; this port does.
const EM_FRACTION_NAMES: [(&str, f64); 5] =
    [("endash", 0.5), ("onedotenleader", 0.5), ("twodotenleader", 0.5), ("threedotenleader", 0.5), ("emdash", 1.0)];

/// `check.metrics.fixedFractionWidths`: the glyphs of [`EM_FRACTION_NAMES`] have their width.
pub(crate) fn fixed_fraction_widths(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (Some(adv), Ok(head)) = (advances(&ctx.font), ctx.font.head()) else {
        sink.skip("the font has no readable 'hmtx', 'maxp' or 'head' table");
        return;
    };
    let em = f64::from(head.units_per_em());
    let names = Names::new(ctx, adv.len());
    for (name, fraction) in EM_FRACTION_NAMES {
        let Some(gid) = names.gid(name) else { continue };
        let Some(width) = advance(&adv, gid).map(f64::from) else { continue };
        let expected = em * fraction;
        if width == expected {
            continue;
        }
        let (code, what) = if fraction < 1.0 { ("not-half-em", "half the em") } else { ("not-em", "the em") };
        let msg = format!("{name} is {} units wide rather than {what} ({}).", num(width), num(expected));
        sink.push(Hit::new(code, msg).data("width", width).data("expected", expected).place(glyph_place(ctx, &names, gid)));
    }
}

// ---------------------------------------------------------------------------------------------
// Ligatures and accented glyphs
// ---------------------------------------------------------------------------------------------

/// Collects an outline into a path.
struct PathPen(BezPath);

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), f64::from(y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), f64::from(y)));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to((f64::from(cx0), f64::from(cy0)), (f64::from(x), f64::from(y)));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to((f64::from(cx0), f64::from(cy0)), (f64::from(cx1), f64::from(cy1)), (f64::from(x), f64::from(y)));
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}

/// A glyph's horizontal metrics as comparefamily reads them from `tx -mtx`: advance, left edge and
/// right edge of the outline's bounds, rounded; an empty glyph has edges at 0.
#[derive(Clone, Copy)]
struct HMetrics {
    width: f64,
    left: f64,
    right: f64,
}

impl HMetrics {
    fn rsb(self) -> f64 {
        self.width - self.right
    }
}

fn h_metrics(adv: &[u16], outlines: &OutlineGlyphCollection<'_>, gid: GlyphId) -> Option<HMetrics> {
    let width = f64::from(advance(adv, gid)?);
    let glyph = outlines.get(gid)?;
    let mut pen = PathPen(BezPath::new());
    glyph.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).ok()?;
    if pen.0.elements().is_empty() {
        return Some(HMetrics { width, left: 0.0, right: 0.0 });
    }
    let r = pen.0.bounding_box();
    if !(r.x0.is_finite() && r.x1.is_finite()) {
        return None;
    }
    Some(HMetrics { width, left: r.x0.round(), right: r.x1.round() })
}

/// The ligature substitutions of the GSUB `liga`, `clig`, `dlig`, `hlig` and `rlig` features with
/// two or more components: (ligature, components), each pair once.
fn ligatures(font: &FontRef<'_>) -> Vec<(GlyphId, Vec<GlyphId>)> {
    let tags = [Tag::new(b"clig"), Tag::new(b"dlig"), Tag::new(b"hlig"), Tag::new(b"rlig"), Tag::new(b"liga")];
    let Some(lookups) = feature_lookups(font, &tags) else { return Vec::new() };
    let mut out: BTreeSet<(GlyphId, Vec<GlyphId>)> = BTreeSet::new();
    let mut records = 0usize;
    for lookup in &lookups {
        let Ok(SubstitutionSubtables::Ligature(subtables)) = lookup.subtables() else { continue };
        for subtable in subtables.iter().filter_map(Result::ok).take(MAX_SUBTABLES) {
            let Ok(coverage) = subtable.coverage() else { continue };
            for (first, set) in coverage.iter().zip(subtable.ligature_sets().iter()) {
                let Ok(set) = set else { continue };
                for ligature in set.ligatures().iter().filter_map(Result::ok) {
                    records += 1;
                    if records > MAX_LIGATURES {
                        return out.into_iter().collect();
                    }
                    let mut components = vec![GlyphId::from(first)];
                    components.extend(ligature.component_glyph_ids().iter().map(|g| GlyphId::from(g.get())));
                    if components.len() >= 2 {
                        out.insert((GlyphId::from(ligature.ligature_glyph()), components));
                    }
                }
            }
        }
    }
    out.into_iter().collect()
}

/// `check.metrics.ligatureWidths` (comparefamily test 22): a ligature is wider than its first
/// component, its left sidebearing is that of its first component and its right sidebearing that
/// of its last, each within `tolerance`.
pub(crate) fn ligature_widths(ctx: &BinaryCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    let Some(adv) = advances(&ctx.font) else {
        sink.skip("the font has no readable 'hmtx' or 'maxp' table");
        return;
    };
    let tolerance = params.get(&TOLERANCE);
    let names = Names::new(ctx, adv.len());
    let outlines = ctx.font.outline_glyphs();
    for (lig, components) in ligatures(&ctx.font) {
        let (Some(first), Some(last)) = (components.first(), components.last()) else { continue };
        let (Some(l), Some(f), Some(z)) = (h_metrics(&adv, &outlines, lig), h_metrics(&adv, &outlines, *first), h_metrics(&adv, &outlines, *last))
        else {
            continue;
        };
        let lig_name = names.name(lig);
        let place = || glyph_place(ctx, &names, lig);
        if l.width <= f.width - tolerance {
            let msg = format!(
                "The ligature {lig_name} is {} units wide, no wider than its first component {} ({}).",
                num(l.width),
                names.name(*first),
                num(f.width)
            );
            sink.push(Hit::new("not-wider", msg).data("width", l.width).data("expected", f.width).place(place()));
        }
        if (l.left - f.left).abs() > tolerance {
            let msg = format!(
                "The left sidebearing of the ligature {lig_name} is {} but that of its first component {} is {}.",
                num(l.left),
                names.name(*first),
                num(f.left)
            );
            sink.push(Hit::new("left-sidebearing", msg).data("value", l.left).data("expected", f.left).place(place()));
        }
        if (z.rsb() - l.rsb()).abs() > tolerance {
            let msg = format!(
                "The right sidebearing of the ligature {lig_name} is {} but that of its last component {} is {}.",
                num(l.rsb()),
                names.name(*last),
                num(z.rsb())
            );
            sink.push(Hit::new("right-sidebearing", msg).data("value", l.rsb()).data("expected", z.rsb()).place(place()));
        }
    }
}

/// The name parts comparefamily strips from a glyph name to find its base letter, in sorted
/// order: the parts are removed one after the other, wherever they occur.
const ACCENT_NAME_PARTS: [&str; 30] = [
    "accent",
    "acute",
    "acutecomb",
    "bar",
    "breve",
    "caron",
    "cedilla",
    "circumflex",
    "comb",
    "croat",
    "dieresis",
    "dot",
    "dotaccent",
    "grave",
    "gravecomb",
    "hook",
    "hookabove",
    "hookabovecomb",
    "hungarumlaut",
    "macron",
    "ogonek",
    "ring",
    "ringacute",
    "slash",
    "slashacute",
    "tilde",
    "tildecomb",
    "umlaut",
    "uni0302",
    "uni0313",
];

/// The base glyph name comparefamily derives from an accented glyph's name, or `None` when the
/// name has no accent part (or is one).
fn accent_base(name: &str) -> Option<String> {
    let mut base = name.to_string();
    for part in ACCENT_NAME_PARTS {
        if base == part {
            break;
        }
        base = base.replace(part, "");
    }
    if base == name {
        return None;
    }
    if let Some(stripped) = base.strip_suffix('.') {
        base = stripped.to_string();
    }
    (!base.is_empty()).then_some(base)
}

/// `check.metrics.accentedWidths` (comparefamily test 23): a glyph whose name is a base glyph's
/// name with accent parts added has that glyph's advance, within `tolerance`.
pub(crate) fn accented_widths(ctx: &BinaryCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    let Some(adv) = advances(&ctx.font) else {
        sink.skip("the font has no readable 'hmtx' or 'maxp' table");
        return;
    };
    let tolerance = params.get(&TOLERANCE);
    let names = Names::new(ctx, adv.len());
    for (i, name) in names.by_gid.iter().enumerate() {
        let Some(base) = accent_base(name) else { continue };
        let Some(base_gid) = names.gid(&base) else { continue };
        let (Some(width), Some(base_width)) = (adv.get(i).copied(), advance(&adv, base_gid)) else { continue };
        let (width, base_width) = (f64::from(width), f64::from(base_width));
        if (width - base_width).abs() <= tolerance {
            continue;
        }
        let gid = GlyphId::new(u32::try_from(i).unwrap_or(u32::MAX));
        let msg = format!("{name} is {} units wide but its base glyph {base} is {}.", num(width), num(base_width));
        sink.push(Hit::new("width-differs", msg).data("width", width).data("expected", base_width).place(glyph_place(ctx, &names, gid)));
    }
}

// ---------------------------------------------------------------------------------------------
// Source checks
// ---------------------------------------------------------------------------------------------

/// The layers whose widths are compiled or shown: master layers and layers with attributes
/// (intermediate, alternate, colour); backup layers are left out.
fn live_layers<'a>(ctx: &'a GlyphCtx<'_>) -> impl Iterator<Item = &'a fontcraft_model::Layer> {
    ctx.glyph.layers.iter().filter(|l| l.is_master_layer() || !l.info.attrs.is_empty())
}

/// `check.metrics.metricsKeyOutOfSync`: reports through the spacing crate's out-of-sync scan,
/// which the audit cannot reach yet; until it can, the check skips.
pub(crate) fn metrics_key_out_of_sync(ctx: &GlyphCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let has_keys = !ctx.glyph.info.metrics_keys.is_empty() || ctx.glyph.layers.iter().any(|l| !l.info.metrics_keys.is_empty());
    if has_keys {
        sink.skip("the metrics-key scan is not available to the audit");
    }
}

/// `check.metrics.negativeAdvance`: a master or attribute layer with a negative width.
pub(crate) fn negative_advance(ctx: &GlyphCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    for layer in live_layers(ctx) {
        if layer.width.is_finite() && layer.width < 0.0 {
            let msg = format!("{} has a width of {} on this layer; advance widths cannot be negative.", ctx.glyph.name, num(layer.width));
            let place = Location { layer: layer.id.clone(), master: Some(layer.master.clone()), ..Location::default() };
            sink.push(Hit::new("negative-width", msg).data("width", layer.width).place(place));
        }
    }
}

#[cfg(test)]
mod tests;
