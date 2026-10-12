//! Family consistency checks (M12.58). Runners take the context of their scope ([`crate::GlyphCtx`],
//! [`crate::SourceCtx`], [`crate::BinaryCtx`], [`crate::FamilyCtx`]), the thresholds and a
//! [`crate::Sink`]; their registry entries go in `registry/`.
//!
//! The rules are ported from two Apache-2.0 tools, read rule by rule: the family tests 1 to 21 of
//! AFDKO's comparefamily (5.0.1, `python/afdko/comparefamily.py`, Copyright 2015 Adobe Systems
//! Incorporated) and the family checks of FontBakery (1.1.0, `Lib/fontbakery/checks/**`,
//! Copyright the FontBakery authors). Messages and structure are our own.
//!
//! Every check groups the fonts it is given in one of three ways (`audit.familyGrouping`):
//!
//! | Grouping | Fonts that belong together | Checks |
//! |---|---|---|
//! | style-linked group | the same family name, ID 1 (Windows English, else Macintosh) | `maxFourFontsPerFamilyName`, `uniqueStyleLinking`, `nameRecordSets`, `styleLinkedGroupShape`, `boldLinkWeights`, `widthClass`, `macWindowsMenuNames`, `blueValueArraySizes`, `blueScaleAndShift`, the strict part of `consistentFamilyName` |
//! | typographic family | the same typographic family name, ID 16, falling back to ID 1 | every other check |
//! | all fonts given | everything in the run | `consistentFamilyName`, `duplicateFullNames`; `superfamilyVerticalMetrics` compares the typographic families with each other |
//!
//! Bold and italic come from OS/2 fsSelection (bits 5 and 0), or from head macStyle when a font
//! has no OS/2 table.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use read_fonts::ps::cff::dict::{self, Operator, Token};
use read_fonts::ps::cff::stack::Number;
use read_fonts::tables::layout::{FeatureList, FeatureParams, ScriptList};
use read_fonts::types::{GlyphId, Tag};
use read_fonts::{FontRef, TableProvider};
use skrifa::MetadataProvider;

use crate::{BinaryCtx, FamilyCtx, Hit, Location, Params, Sink};

/// Most fonts one family check reads; the caller caps families at the same number.
pub(crate) const MAX_FACES: usize = 256;
/// Most items (name keys, code points, fonts) listed in one message.
const MAX_LISTED: usize = 20;
/// Most script, language and feature entries read from one layout table.
const MAX_LAYOUT_ENTRIES: usize = 100_000;
/// Most operands kept for one DICT operator (the CFF2 argument stack limit).
const MAX_DICT_OPERANDS: usize = 513;
/// Most operators read from one DICT.
const MAX_DICT_ENTRIES: usize = 4_096;
/// The number of Unicode code points.
const MAX_CODEPOINTS: usize = 1_114_112;
/// A glyph wider than this many times its narrowest width in the family is reported.
const WIDTH_RATIO: u32 = 3;
/// BlueScale when a Private DICT does not set it.
const DEFAULT_BLUE_SCALE: f64 = 0.039625;
/// BlueShift when a Private DICT does not set it.
const DEFAULT_BLUE_SHIFT: f64 = 7.0;
/// BlueScale values closer than this are equal: 16.16 fixed point keeps about this much.
const BLUE_SCALE_EPSILON: f64 = 2e-5;
/// Name IDs a style-linked group need not share: the typographic and Macintosh compatible names
/// and the variation and WWS names.
const EXEMPT_NAME_IDS: [u16; 6] = [16, 17, 18, 20, 21, 22];
/// Suffixes that mark a tabular glyph.
const TABULAR_SUFFIXES: [&str; 6] = [".tnum", ".tf", ".tosf", ".tsc", ".tab", ".tabular"];

// ---------------------------------------------------------------------------------------------
// What the checks read from each font
// ---------------------------------------------------------------------------------------------

struct NameRec {
    platform: u16,
    encoding: u16,
    language: u16,
    id: u16,
    text: String,
}

#[derive(Clone, Copy)]
struct Os2 {
    version: u16,
    weight: u16,
    width: u16,
    fs_type: u16,
    fs_selection: u16,
    panose: [u8; 10],
    /// sTypoAscender, sTypoDescender, sTypoLineGap.
    typo: [i16; 3],
    /// usWinAscent, usWinDescent.
    win: [u16; 2],
}

/// One font as the family checks see it.
struct Face<'a> {
    id: &'a str,
    font: &'a FontRef<'a>,
    names: Vec<NameRec>,
    mac_names: bool,
    /// The style-linked group: family name ID 1.
    family: String,
    /// The typographic family: ID 16, else ID 1.
    typo_family: String,
    /// The name messages use: the PostScript name, else the font id.
    label: String,
    bold: bool,
    italic: bool,
    os2: Option<Os2>,
    upm: Option<u16>,
    /// head.fontRevision as 16.16 bits.
    revision: Option<i32>,
    /// post underlinePosition and underlineThickness.
    underline: Option<(i16, i16)>,
    /// hhea ascender, descender and line gap.
    hhea: Option<[i16; 3]>,
    variable: bool,
}

impl<'a> Face<'a> {
    fn new(ctx: &'a BinaryCtx<'a>) -> Self {
        let font = &ctx.font;
        let names = read_names(font);
        let mac_names = names.iter().any(|n| n.platform == 1);
        let os2 = font.os2().ok().map(|t| {
            let mut panose = [0u8; 10];
            for (slot, v) in panose.iter_mut().zip(t.panose_10()) {
                *slot = *v;
            }
            Os2 {
                version: t.version(),
                weight: t.us_weight_class(),
                width: t.us_width_class(),
                fs_type: t.fs_type(),
                fs_selection: t.fs_selection().bits(),
                panose,
                typo: [t.s_typo_ascender(), t.s_typo_descender(), t.s_typo_line_gap()],
                win: [t.us_win_ascent(), t.us_win_descent()],
            }
        });
        let head = font.head().ok();
        let mac_style = head.as_ref().map(|h| h.mac_style().bits()).unwrap_or(0);
        let (bold, italic) = match &os2 {
            Some(o) => (o.fs_selection & 0x20 != 0, o.fs_selection & 1 != 0),
            None => (mac_style & 1 != 0, mac_style & 2 != 0),
        };
        let mut face = Face {
            id: ctx.id,
            font,
            names,
            mac_names,
            family: String::new(),
            typo_family: String::new(),
            label: String::new(),
            bold,
            italic,
            os2,
            upm: head.as_ref().map(|h| h.units_per_em()),
            revision: head.as_ref().map(|h| h.font_revision().to_bits()),
            underline: font.post().ok().map(|p| (p.underline_position().to_i16(), p.underline_thickness().to_i16())),
            hhea: font.hhea().ok().map(|h| [h.ascender().to_i16(), h.descender().to_i16(), h.line_gap().to_i16()]),
            variable: font.fvar().is_ok(),
        };
        face.family = face.win(1).or_else(|| face.mac(1)).unwrap_or_default().to_string();
        face.typo_family = face.win(16).or_else(|| face.win(1)).or_else(|| face.mac(16)).or_else(|| face.mac(1)).unwrap_or_default().to_string();
        face.label = face.win(6).or_else(|| face.mac(6)).filter(|s| !s.is_empty()).unwrap_or(ctx.id).to_string();
        face
    }

    fn name(&self, platform: u16, encoding: u16, language: u16, id: u16) -> Option<&str> {
        self.names.iter().find(|n| n.platform == platform && n.encoding == encoding && n.language == language && n.id == id).map(|n| n.text.as_str())
    }

    /// A Windows Unicode English (United States) name.
    fn win(&self, id: u16) -> Option<&str> {
        self.name(3, 1, 0x409, id)
    }

    /// A Macintosh Roman English name.
    fn mac(&self, id: u16) -> Option<&str> {
        self.name(1, 0, 0, id)
    }

    /// The copyright (0) or trademark (7) string the comparison reads: the Macintosh one when the
    /// font keeps Macintosh names, the Windows one otherwise.
    fn legal(&self, id: u16) -> &str {
        if self.mac_names { self.mac(id) } else { self.win(id) }.unwrap_or_default()
    }

    fn here(&self) -> Location {
        Location { font: Some(self.id.to_string()), fonts: vec![self.id.to_string()], ..Location::default() }
    }

    fn field(&self, table: &str, field: &str) -> Location {
        Location { table: Some(table.into()), field: Some(field.into()), ..self.here() }
    }
}

fn read_names(font: &FontRef<'_>) -> Vec<NameRec> {
    let Ok(name) = font.name() else { return Vec::new() };
    let data = name.string_data();
    name.name_record()
        .iter()
        .filter_map(|r| {
            let text = r.string(data).ok()?.to_string();
            Some(NameRec { platform: r.platform_id(), encoding: r.encoding_id(), language: r.language_id(), id: r.name_id().to_u16(), text })
        })
        .collect()
}

fn faces<'a>(fonts: &'a [BinaryCtx<'a>]) -> Vec<Face<'a>> {
    fonts.iter().take(MAX_FACES).map(Face::new).collect()
}

/// Splits `faces` by `key`, keeping the order in which each key first appears.
fn group_by<'f, 'a>(faces: &'f [Face<'a>], key: impl for<'x> Fn(&'x Face<'a>) -> &'x str) -> Vec<(String, Vec<&'f Face<'a>>)> {
    let mut groups: Vec<(String, Vec<&'f Face<'a>>)> = Vec::new();
    for face in faces {
        let k = key(face);
        match groups.iter_mut().find(|(g, _)| g == k) {
            Some((_, members)) => members.push(face),
            None => groups.push((k.to_string(), vec![face])),
        }
    }
    groups
}

fn style_linked<'f, 'a>(faces: &'f [Face<'a>]) -> Vec<(String, Vec<&'f Face<'a>>)> {
    group_by(faces, |f| f.family.as_str())
}

fn typographic<'f, 'a>(faces: &'f [Face<'a>]) -> Vec<(String, Vec<&'f Face<'a>>)> {
    group_by(faces, |f| f.typo_family.as_str())
}

fn among(group: &[&Face<'_>]) -> Location {
    Location { fonts: group.iter().map(|f| f.id.to_string()).collect(), ..Location::default() }
}

fn labels(group: &[&Face<'_>]) -> String {
    list(group.iter().map(|f| f.label.clone()))
}

/// `a, b, c`, at most [`MAX_LISTED`] items, then how many more.
fn list(items: impl IntoIterator<Item = String>) -> String {
    let all: Vec<String> = items.into_iter().collect();
    let mut text = all.iter().take(MAX_LISTED).cloned().collect::<Vec<_>>().join(", ");
    if all.len() > MAX_LISTED {
        text.push_str(&format!(" and {} more", all.len() - MAX_LISTED));
    }
    text
}

fn strict(params: &Params<'_>, check: &str) -> bool {
    crate::find(check).and_then(|c| c.threshold("strict")).is_some_and(|t| params.get(t) >= 0.5)
}

// ---------------------------------------------------------------------------------------------
// Names and style linking
// ---------------------------------------------------------------------------------------------

/// FontBakery `opentype/family/max_4_fonts_per_family_name`; comparefamily family test 1.
pub(crate) fn max_four_fonts_per_family_name(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    let named: Vec<&Face<'_>> = faces.iter().filter(|f| f.win(1).is_some()).collect();
    let mut groups: Vec<(&str, Vec<&Face<'_>>)> = Vec::new();
    for face in named {
        let name = face.win(1).unwrap_or_default();
        match groups.iter_mut().find(|(g, _)| *g == name) {
            Some((_, m)) => m.push(face),
            None => groups.push((name, vec![face])),
        }
    }
    for (name, group) in groups {
        if group.len() > 4 {
            let n = group.len();
            sink.push(
                Hit::new(
                    "too-many",
                    format!("{n} fonts share the family name '{name}' ({}); a style-linked group holds four at most.", labels(&group)),
                )
                .data("fonts", n as f64)
                .place(among(&group)),
            );
        }
    }
}

/// FontBakery `opentype/family/bold_italic_unique_for_nameid1`; comparefamily family test 5.
pub(crate) fn unique_style_linking(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (name, group) in style_linked(&faces) {
        let mut seen: Vec<((bool, bool), Vec<&Face<'_>>)> = Vec::new();
        for face in group {
            let key = (face.bold, face.italic);
            match seen.iter_mut().find(|(k, _)| *k == key) {
                Some((_, m)) => m.push(face),
                None => seen.push((key, vec![face])),
            }
        }
        for ((bold, italic), same) in seen {
            if same.len() > 1 {
                sink.push(
                    Hit::new(
                        "same-style-bits",
                        format!(
                            "{} in family '{name}' all have bold {} and italic {}: applications cannot tell them apart.",
                            labels(&same),
                            yes(bold),
                            yes(italic)
                        ),
                    )
                    .place(among(&same)),
                );
            }
        }
    }
}

fn yes(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

/// FontBakery `opentype/family/consistent_family_name` and `typographic_family_name`;
/// comparefamily family test 3 (with `strict`).
pub(crate) fn consistent_family_name(ctx: &FamilyCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    let by_name = group_by(&faces, |f| f.win(16).or_else(|| f.win(1)).unwrap_or_default());
    if by_name.len() > 1 {
        let detail = list(by_name.iter().map(|(name, group)| format!("'{name}' in {}", labels(group))));
        sink.push(
            Hit::new("inconsistent-family-name", format!("{} different family names: {detail}.", by_name.len())).data("names", by_name.len() as f64),
        );
    }
    if !strict(params, "check.family.consistentFamilyName") {
        return;
    }
    for (name, group) in style_linked(&faces) {
        let mut languages: Vec<u16> = Vec::new();
        for face in &group {
            for n in &face.names {
                if n.platform == 3 && n.encoding == 1 && n.id == 16 && !languages.contains(&n.language) {
                    languages.push(n.language);
                }
            }
        }
        let Some((first, rest)) = group.split_first() else { continue };
        for language in languages {
            let base = first.name(3, 1, language, 16);
            for face in rest {
                let value = face.name(3, 1, language, 16);
                if value != base {
                    let place = Location {
                        name_id: Some(16),
                        platform: Some(3),
                        language: Some(language.to_string()),
                        fonts: vec![first.id.into(), face.id.into()],
                        ..Location::default()
                    };
                    sink.push(
                        Hit::new(
                            "differs-in-language",
                            format!(
                                "In group '{name}', the typographic family name for language {language} is {} in {} but {} in {}.",
                                quoted(value),
                                face.label,
                                quoted(base),
                                first.label
                            ),
                        )
                        .place(place),
                    );
                }
            }
        }
    }
}

fn quoted(s: Option<&str>) -> String {
    s.map(|s| format!("'{s}'")).unwrap_or_else(|| "missing".into())
}

/// comparefamily family test 2.
pub(crate) fn name_record_sets(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (name, group) in style_linked(&faces) {
        let mut counts: BTreeMap<(u16, u16, u16, u16), usize> = BTreeMap::new();
        for face in &group {
            let keys: BTreeSet<(u16, u16, u16, u16)> = face
                .names
                .iter()
                .filter(|n| !EXEMPT_NAME_IDS.contains(&n.id) && n.id <= 255)
                .map(|n| (n.platform, n.encoding, n.language, n.id))
                .collect();
            for key in keys {
                *counts.entry(key).or_default() += 1;
            }
        }
        let (Some(min), Some(max)) = (counts.values().min(), counts.values().max()) else { continue };
        if min == max {
            continue;
        }
        let partial = list(counts.iter().filter(|(_, n)| **n < group.len()).map(|((p, e, l, id), n)| format!("({p}, {e}, {l}, {id}) in {n}")));
        sink.push(
            Hit::new(
                "differs",
                format!(
                    "The fonts of group '{name}' ({}) do not carry the same name records; records missing from some fonts: {partial}.",
                    labels(&group)
                ),
            )
            .place(among(&group)),
        );
    }
}

/// The base (regular, else bold) and bold fonts of a style-linked group, as comparefamily family
/// test 6 picks them: the last upright plain font and the last upright bold font.
fn base_and_bold<'f, 'a>(group: &[&'f Face<'a>]) -> (Option<&'f Face<'a>>, Option<&'f Face<'a>>) {
    let regular = group.iter().rev().find(|f| !f.bold && !f.italic).copied();
    let bold = group.iter().rev().find(|f| f.bold && !f.italic).copied();
    (regular.or(bold), bold)
}

/// comparefamily family test 6 (base font).
pub(crate) fn style_linked_group_shape(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (name, group) in style_linked(&faces) {
        let (base, bold) = base_and_bold(&group);
        let Some(base) = base else {
            sink.push(
                Hit::new("no-base-font", format!("Group '{name}' ({}) has no upright regular or bold font.", labels(&group))).place(among(&group)),
            );
            continue;
        };
        if bold.is_some_and(|b| std::ptr::eq(b, base)) && group.len() > 2 {
            sink.push(
                Hit::new(
                    "base-not-regular",
                    format!("Group '{name}' has {} fonts but no upright regular one: its base would be the bold font {}.", group.len(), base.label),
                )
                .place(among(&group)),
            );
        }
    }
}

/// The bold weight classes a regular weight class allows before applications embolden it.
fn bold_range(regular: u16) -> Option<(u16, u16)> {
    match regular / 50 * 50 {
        250 => Some((600, 1000)),
        300 => Some((600, 950)),
        350 => Some((600, 900)),
        400 => Some((600, 850)),
        450 => Some((600, 800)),
        500 => Some((650, 750)),
        550 => Some((600, 800)),
        _ => None,
    }
}

/// comparefamily family tests 6 (weight ranges) and 8.
pub(crate) fn bold_link_weights(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (name, group) in style_linked(&faces) {
        // Family test 6.
        if let (Some(base), Some(bold)) = base_and_bold(&group)
            && !std::ptr::eq(base, bold)
            && let (Some(b), Some(w)) = (base.os2, bold.os2)
        {
            match bold_range(b.weight) {
                None => sink.push(
                    Hit::new(
                        "regular-weight-out-of-range",
                        format!(
                            "The regular font {} has weight class {}, outside 250 to 599: choosing Bold will embolden it instead of using {}.",
                            base.label, b.weight, bold.label
                        ),
                    )
                    .data("weight", f64::from(b.weight))
                    .place(base.field("OS/2", "usWeightClass")),
                ),
                Some((lo, hi)) if w.weight < lo || w.weight > hi => sink.push(
                    Hit::new(
                        "bold-weight-out-of-range",
                        format!(
                            "The bold font {} has weight class {}, outside {lo} to {hi}, the range for a regular weight of {} in group '{name}'.",
                            bold.label, w.weight, b.weight
                        ),
                    )
                    .data("weight", f64::from(w.weight))
                    .data("min", f64::from(lo))
                    .data("max", f64::from(hi))
                    .place(bold.field("OS/2", "usWeightClass")),
                ),
                Some(_) if w.weight % 50 != 0 => sink.push(
                    Hit::new(
                        "bold-weight-not-round",
                        format!("The bold font {} has weight class {}, which is not a multiple of 50.", bold.label, w.weight),
                    )
                    .data("weight", f64::from(w.weight))
                    .place(bold.field("OS/2", "usWeightClass")),
                ),
                Some(_) => {}
            }
        }
        // Family test 8.
        if group.first().is_none_or(|f| f.os2.is_none()) {
            continue;
        }
        let mut weights: Vec<u16> = Vec::new();
        for w in group.iter().filter_map(|f| f.os2.map(|o| o.weight)) {
            if !weights.contains(&w) {
                weights.push(w);
            }
        }
        let bold = group.iter().rev().find(|f| f.bold);
        let base = group.iter().rev().find(|f| !f.bold && !f.italic);
        match weights.as_slice() {
            [_, _, _, ..] => sink.push(
                Hit::new(
                    "too-many-weights",
                    format!(
                        "Group '{name}' uses {} weight classes ({}); a style-linked group uses two at most.",
                        weights.len(),
                        list(weights.iter().map(u16::to_string))
                    ),
                )
                .data("weights", weights.len() as f64)
                .place(among(&group)),
            ),
            [_] => {
                if let (Some(bold), Some(_)) = (bold, base) {
                    sink.push(
                        Hit::new(
                            "bold-same-weight",
                            format!("The bold font {} has the same weight class as the regular font of group '{name}'.", bold.label),
                        )
                        .place(bold.field("OS/2", "usWeightClass")),
                    );
                }
            }
            [a, b] => {
                let (low, high) = (*a.min(b), *a.max(b));
                for face in &group {
                    let Some(o) = face.os2 else { continue };
                    if o.weight == low && face.bold {
                        sink.push(
                            Hit::new(
                                "bold-lighter",
                                format!("{} is marked bold but has the lower weight class of group '{name}' ({low}).", face.label),
                            )
                            .place(face.field("OS/2", "usWeightClass")),
                        );
                    }
                    if o.weight == high && !face.bold {
                        sink.push(
                            Hit::new(
                                "regular-heavier",
                                format!("{} is not marked bold but has the higher weight class of group '{name}' ({high}).", face.label),
                            )
                            .place(face.field("OS/2", "usWeightClass")),
                        );
                    }
                }
            }
            [] => {}
        }
    }
}

/// comparefamily family test 9.
pub(crate) fn width_class(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (name, group) in style_linked(&faces) {
        let Some((first, rest)) = group.split_first() else { continue };
        let Some(base) = first.os2 else { continue };
        for face in rest {
            let Some(o) = face.os2 else { continue };
            if o.width != base.width {
                sink.push(
                    Hit::new(
                        "differs",
                        format!("In group '{name}', {} has width class {} but {} has {}.", face.label, o.width, first.label, base.width),
                    )
                    .place(Location { fonts: vec![first.id.into(), face.id.into()], ..face.field("OS/2", "usWidthClass") }),
                );
            }
        }
    }
}

/// Removes four-digit years (with the spaces and commas around them) and collapses white space,
/// as comparefamily family test 7 does before it compares two legal strings.
fn without_years(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let at = |i: usize| chars.get(i).copied();
    let mut stripped = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let mut j = i;
        while at(j) == Some(' ') {
            j += 1;
        }
        let year = matches!(at(j), Some('1' | '2'))
            && matches!(at(j + 1), Some('9' | '8' | '0'))
            && at(j + 2).is_some_and(|c| c.is_ascii_digit())
            && at(j + 3).is_some_and(|c| c.is_ascii_digit());
        if year {
            let mut k = j + 4;
            while at(k) == Some(',') {
                k += 1;
            }
            while at(k) == Some(' ') {
                k += 1;
            }
            i = k;
            continue;
        }
        if let Some(c) = at(i) {
            stripped.push(c);
        }
        i += 1;
    }
    let mut out = String::with_capacity(stripped.len());
    let mut space = false;
    for c in stripped.chars() {
        if c.is_whitespace() {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    out
}

/// comparefamily family test 7.
pub(crate) fn copyright_and_trademark(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (name, group) in typographic(&faces) {
        let Some((first, rest)) = group.split_first() else { continue };
        for (id, what, differs, years) in
            [(0u16, "copyright", "copyright-differs", "copyright-years-only"), (7, "trademark", "trademark-differs", "trademark-years-only")]
        {
            let base = first.legal(id);
            for face in rest {
                let value = face.legal(id);
                if value == base {
                    continue;
                }
                let only_years = without_years(value) == without_years(base);
                let (code, message) = if only_years {
                    (years, format!("In family '{name}', the {what} strings of {} and {} differ only in their years.", first.label, face.label))
                } else {
                    (differs, format!("In family '{name}', {} has the {what} string '{value}' but {} has '{base}'.", face.label, first.label))
                };
                sink.push(Hit::new(code, message).place(Location {
                    table: Some("name".into()),
                    name_id: Some(id),
                    fonts: vec![first.id.into(), face.id.into()],
                    ..Location::default()
                }));
            }
        }
    }
}

/// comparefamily family test 11: the two names are the same when they have the same length and
/// agree on every character the Macintosh name has below 128.
fn same_menu_name(win: &str, mac: &str) -> bool {
    let (w, m): (Vec<char>, Vec<char>) = (win.chars().collect(), mac.chars().collect());
    w.len() == m.len() && w.iter().zip(&m).all(|(a, b)| u32::from(*b) > 127 || a == b)
}

/// comparefamily family test 11, for fonts that keep Macintosh names.
pub(crate) fn mac_windows_menu_names(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (_, group) in style_linked(&faces) {
        for face in group {
            if !face.mac_names || ps_info(face.font).is_cid() {
                continue;
            }
            let mac = face.mac(18).filter(|s| !s.is_empty()).or_else(|| face.mac(4)).unwrap_or_default();
            let win = face.win(1).unwrap_or_default();
            let same = same_menu_name(win, mac);
            let place = Location { table: Some("name".into()), name_id: Some(1), ..face.here() };
            if (face.bold || face.italic) && same {
                sink.push(
                    Hit::new(
                        "bold-italic-names-equal",
                        format!("{} is a bold or italic member of its group, so its Macintosh name '{mac}' should differ from its Windows family name '{win}'.", face.label),
                    )
                    .place(place),
                );
            } else if !face.bold && !face.italic && !same {
                sink.push(
                    Hit::new(
                        "regular-names-differ",
                        format!(
                            "{} is the regular member of its group, so its Macintosh name '{mac}' should equal its Windows family name '{win}'.",
                            face.label
                        ),
                    )
                    .place(place),
                );
            }
        }
    }
}

/// FontBakery `typenetwork/family/duplicated_names`.
pub(crate) fn duplicate_full_names(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (id, code, what) in [(4u16, "duplicate-full-names", "full name"), (6, "duplicate-postscript-names", "PostScript name")] {
        let mut seen: Vec<(&str, Vec<&Face<'_>>)> = Vec::new();
        for face in &faces {
            let Some(name) = face.win(id) else { continue };
            match seen.iter_mut().find(|(n, _)| *n == name) {
                Some((_, m)) => m.push(face),
                None => seen.push((name, vec![face])),
            }
        }
        for (name, same) in seen {
            if same.len() > 1 {
                sink.push(
                    Hit::new(code, format!("{} fonts have the {what} '{name}': {}.", same.len(), list(same.iter().map(|f| f.id.to_string()))))
                        .place(Location { table: Some("name".into()), name_id: Some(id), ..among(&same) }),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Metrics and OS/2 values
// ---------------------------------------------------------------------------------------------

/// FontBakery `opentype/family/equal_font_versions`.
pub(crate) fn equal_versions(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (name, group) in typographic(&faces) {
        let versions: BTreeSet<i32> = group.iter().filter_map(|f| f.revision).collect();
        if versions.len() > 1 {
            let detail = list(group.iter().filter_map(|f| Some(format!("{} {}", f.label, revision_text(f.revision?)))));
            sink.push(Hit::new("mismatch", format!("The fonts of family '{name}' have different font revisions: {detail}.")).place(Location {
                table: Some("head".into()),
                field: Some("fontRevision".into()),
                ..among(&group)
            }));
        }
    }
}

fn revision_text(bits: i32) -> String {
    format!("{:.3}", f64::from(bits) / 65536.0)
}

/// FontBakery `opentype/family/panose_familytype`; comparefamily family test 10 (with `strict`).
pub(crate) fn panose_family_type(ctx: &FamilyCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    let strict = strict(params, "check.family.panoseFamilyType");
    let faces = faces(ctx.fonts);
    for (name, group) in typographic(&faces) {
        let mut types: BTreeMap<u8, Vec<&Face<'_>>> = BTreeMap::new();
        for face in &group {
            let Some(o) = face.os2 else {
                sink.push(
                    Hit::new("lacks-os2", format!("{} has no OS/2 table, so it has no PANOSE classification.", face.label))
                        .place(Location { table: Some("OS/2".into()), ..face.here() }),
                );
                continue;
            };
            types.entry(o.panose.first().copied().unwrap_or(0)).or_default().push(*face);
            if !strict {
                continue;
            }
            let (head, tail) = o.panose.split_at(4);
            if tail.iter().all(|v| *v == 0) {
                if head.iter().all(|v| *v == 0) {
                    sink.push(
                        Hit::new("all-zero", format!("{} has a PANOSE classification of all zeros.", face.label)).place(face.field("OS/2", "panose")),
                    );
                } else {
                    sink.push(
                        Hit::new(
                            "first-four-only",
                            format!("{} sets only the first four PANOSE digits, which may be defaults a compiler filled in.", face.label),
                        )
                        .place(face.field("OS/2", "panose")),
                    );
                }
            }
        }
        if types.len() > 1 {
            let detail = list(types.iter().map(|(t, fonts)| format!("{t} in {}", labels(fonts))));
            sink.push(Hit::new("inconsistency", format!("The fonts of family '{name}' differ in PANOSE family type: {detail}.")).place(Location {
                table: Some("OS/2".into()),
                field: Some("panose".into()),
                ..among(&group)
            }));
        }
    }
}

/// FontBakery `opentype/family/underline_thickness`; comparefamily family test 15.
pub(crate) fn underline_consistency(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (name, group) in typographic(&faces) {
        let Some((first, rest)) = group.split_first() else { continue };
        let Some((pos, thick)) = first.underline else { continue };
        for face in rest {
            let Some((p, t)) = face.underline else { continue };
            for (code, field, what, value, base) in [
                ("position-differs", "underlinePosition", "underline position", p, pos),
                ("thickness-differs", "underlineThickness", "underline thickness", t, thick),
            ] {
                if value != base {
                    sink.push(
                        Hit::new(code, format!("In family '{name}', {} has {what} {value} but {} has {base}.", face.label, first.label))
                            .data("value", f64::from(value))
                            .data("expected", f64::from(base))
                            .place(Location { fonts: vec![first.id.into(), face.id.into()], ..face.field("post", field) }),
                    );
                }
            }
        }
    }
}

/// The eight vertical metrics: table, field and value.
fn vertical(face: &Face<'_>) -> Option<[(&'static str, &'static str, i32); 8]> {
    let (o, [ha, hd, hg]) = (face.os2?, face.hhea?);
    let ([ta, td, tg], [wa, wd]) = (o.typo, o.win);
    Some([
        ("OS/2", "sTypoAscender", i32::from(ta)),
        ("OS/2", "sTypoDescender", i32::from(td)),
        ("OS/2", "sTypoLineGap", i32::from(tg)),
        ("OS/2", "usWinAscent", i32::from(wa)),
        ("OS/2", "usWinDescent", i32::from(wd)),
        ("hhea", "ascender", i32::from(ha)),
        ("hhea", "descender", i32::from(hd)),
        ("hhea", "lineGap", i32::from(hg)),
    ])
}

/// A metric that differs: table, field, and each font's value.
type MetricDifference<'f> = (&'static str, &'static str, Vec<(&'f str, i32)>);
/// The eight vertical metrics of one font, each with its table and field.
type VerticalMetrics = [(&'static str, &'static str, i32); 8];
/// For each of the eight metrics that differ between `group`'s fonts: table, field, and the
/// value of each font.
fn metric_differences<'f>(group: &[&'f Face<'_>]) -> Vec<MetricDifference<'f>> {
    let rows: Vec<(&str, VerticalMetrics)> = group.iter().filter_map(|f| Some((f.label.as_str(), vertical(f)?))).collect();
    let mut out = Vec::new();
    for i in 0..8 {
        let values: Vec<(&str, (&'static str, &'static str, i32))> = rows.iter().filter_map(|(label, r)| Some((*label, *r.get(i)?))).collect();
        let distinct: BTreeSet<i32> = values.iter().map(|(_, (_, _, v))| *v).collect();
        if let (true, Some((_, (table, field, _)))) = (distinct.len() > 1, values.first()) {
            out.push((*table, *field, values.iter().map(|(l, (_, _, v))| (*l, *v)).collect()));
        }
    }
    out
}

/// FontBakery `family/vertical_metrics`.
pub(crate) fn vertical_metrics(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (name, group) in typographic(&faces) {
        let mut missing = false;
        for face in &group {
            if face.os2.is_none() {
                missing = true;
                sink.push(
                    Hit::new("lacks-os2", format!("{} has no OS/2 table.", face.label))
                        .without_fix()
                        .place(Location { table: Some("OS/2".into()), ..face.here() }),
                );
            } else if face.hhea.is_none() {
                missing = true;
                sink.push(
                    Hit::new("lacks-hhea", format!("{} has no hhea table.", face.label))
                        .without_fix()
                        .place(Location { table: Some("hhea".into()), ..face.here() }),
                );
            }
        }
        if missing {
            continue;
        }
        for (table, field, values) in metric_differences(&group) {
            let detail = list(values.iter().map(|(l, v)| format!("{l} {v}")));
            sink.push(Hit::new("mismatch", format!("{table} {field} is not the same across family '{name}': {detail}.")).place(Location {
                table: Some(table.into()),
                field: Some(field.into()),
                ..among(&group)
            }));
        }
    }
}

/// FontBakery `superfamily/vertical_metrics` and `superfamily/list`. The sibling families are
/// the typographic families among the fonts given.
pub(crate) fn superfamily_vertical_metrics(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    let families = typographic(&faces);
    if families.len() < 2 {
        return;
    }
    sink.push(
        Hit::new("families", format!("{} sibling families: {}.", families.len(), list(families.iter().map(|(n, _)| format!("'{n}'")))))
            .data("families", families.len() as f64),
    );
    let all: Vec<&Face<'_>> = faces.iter().collect();
    for (table, field, values) in metric_differences(&all) {
        let detail = list(values.iter().map(|(l, v)| format!("{l} {v}")));
        sink.push(
            Hit::new("superfamily-vertical-metrics", format!("{table} {field} is not the same across the sibling families: {detail}."))
                .place(Location { table: Some(table.into()), field: Some(field.into()), ..Location::default() }),
        );
    }
}

/// FontBakery `adobefonts/family/consistent_upm`; comparefamily single-face test 16.
pub(crate) fn units_per_em(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (name, group) in typographic(&faces) {
        let values: BTreeSet<u16> = group.iter().filter_map(|f| f.upm).collect();
        if values.len() > 1 {
            sink.push(
                Hit::new(
                    "inconsistent-upem",
                    format!("The fonts of family '{name}' use different units per em: {}.", list(values.iter().map(u16::to_string))),
                )
                .place(Location { table: Some("head".into()), field: Some("unitsPerEm".into()), ..among(&group) }),
            );
        }
    }
}

/// comparefamily family test 13.
pub(crate) fn wws_bit(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    const WWS: u16 = 1 << 8;
    let faces = faces(ctx.fonts);
    for (name, group) in typographic(&faces) {
        let with: Vec<(&Face<'_>, Os2)> = group.iter().filter_map(|f| Some((*f, f.os2?))).collect();
        let set: Vec<&Face<'_>> = with.iter().filter(|(_, o)| o.fs_selection & WWS != 0).map(|(f, _)| *f).collect();
        if set.is_empty() {
            continue;
        }
        if set.len() != with.len() {
            sink.push(
                Hit::new("mixed", format!("In family '{name}', the WWS bit is set in {} but not in the other fonts.", labels(&set)))
                    .place(Location { table: Some("OS/2".into()), field: Some("fsSelection".into()), ..among(&group) }),
            );
            continue;
        }
        let mut old = false;
        for (face, o) in &with {
            if o.version < 4 {
                old = true;
                sink.push(
                    Hit::new(
                        "old-os2-version",
                        format!("{} sets the WWS bit, but its OS/2 table is version {}; the bit needs version 4.", face.label, o.version),
                    )
                    .place(face.field("OS/2", "version")),
                );
            }
        }
        if old {
            continue;
        }
        let mut seen: Vec<((u16, u16, bool), &Face<'_>)> = Vec::new();
        for (face, o) in &with {
            let key = (o.width, o.weight, face.italic);
            match seen.iter().find(|(k, _)| *k == key) {
                Some((_, other)) => sink.push(
                    Hit::new(
                        "duplicate-style",
                        format!("{} has the same weight, width and italic values as {} although the WWS bit is set.", face.label, other.label),
                    )
                    .place(Location { fonts: vec![other.id.into(), face.id.into()], ..face.field("OS/2", "fsSelection") }),
                ),
                None => seen.push((key, *face)),
            }
        }
    }
}

/// comparefamily family test 14.
pub(crate) fn embedding_permissions(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (name, group) in typographic(&faces) {
        let Some((first, rest)) = group.split_first() else { continue };
        let Some(base) = first.os2 else { continue };
        for face in rest {
            let Some(o) = face.os2 else { continue };
            if o.fs_type != base.fs_type {
                sink.push(
                    Hit::new(
                        "differs",
                        format!(
                            "In family '{name}', {} has embedding permissions (fsType) {} but {} has {}.",
                            face.label, o.fs_type, first.label, base.fs_type
                        ),
                    )
                    .place(Location { fonts: vec![first.id.into(), face.id.into()], ..face.field("OS/2", "fsType") }),
                );
            }
        }
    }
}

/// FontBakery `googlefonts/family/italics_have_roman_counterparts`, read from the style bits and
/// weight and width classes instead of file names.
pub(crate) fn italics_have_uprights(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (name, group) in typographic(&faces) {
        for face in group.iter().filter(|f| f.italic) {
            let partner = group.iter().any(|u| {
                !u.italic
                    && u.variable == face.variable
                    && (face.variable || matches!((u.os2, face.os2), (Some(a), Some(b)) if a.weight == b.weight && a.width == b.width))
            });
            if !partner {
                let weight = face.os2.map(|o| o.weight.to_string()).unwrap_or_else(|| "unknown".into());
                sink.push(
                    Hit::new(
                        "missing-upright",
                        format!("The italic font {} (weight class {weight}) has no upright partner in family '{name}'.", face.label),
                    )
                    .place(face.here()),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Glyphs, widths and characters
// ---------------------------------------------------------------------------------------------

/// Glyph names in glyph order, and whether they come from the font (not made up from ids).
fn glyph_names(font: &FontRef<'_>) -> (Vec<String>, bool) {
    let names = font.glyph_names();
    let mut real = false;
    let list = names
        .iter()
        .map(|(_, n)| {
            real |= !n.is_synthesized();
            n.as_str().to_string()
        })
        .collect();
    (list, real)
}

/// Advance width per glyph name.
fn widths(font: &FontRef<'_>, names: &[String]) -> HashMap<String, u16> {
    let Ok(hmtx) = font.hmtx() else { return HashMap::new() };
    names.iter().enumerate().filter_map(|(gid, name)| Some((name.clone(), hmtx.advance(GlyphId::new(u32::try_from(gid).ok()?))?))).collect()
}

/// comparefamily family test 16.
pub(crate) fn glyph_width_ratio(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (name, group) in typographic(&faces) {
        if group.len() < 2 {
            continue;
        }
        let data: Vec<(&Face<'_>, HashMap<String, u16>, Vec<String>)> = group
            .iter()
            .map(|f| {
                let (mut names, _) = glyph_names(f.font);
                let w = widths(f.font, &names);
                names.sort();
                (*f, w, names)
            })
            .collect();
        // Fonts with the same glyph set are compared with each other; a font whose glyph set no
        // other font shares joins the largest shared set.
        let mut sets: BTreeMap<&[String], Vec<usize>> = BTreeMap::new();
        for (i, (_, _, names)) in data.iter().enumerate() {
            sets.entry(names.as_slice()).or_default().push(i);
        }
        let mut singles: Vec<usize> = Vec::new();
        let mut last_single: Option<&[String]> = None;
        sets.retain(|set, members| {
            if members.len() == 1 {
                singles.extend(members.iter().copied());
                last_single = Some(*set);
                false
            } else {
                true
            }
        });
        let mut compared: Vec<(&[String], Vec<usize>)> = sets.into_iter().collect();
        compared.sort_by_key(|(set, _)| set.len());
        match compared.last_mut() {
            Some((_, members)) => members.extend(singles),
            None => {
                if let Some(set) = last_single {
                    compared.push((set, singles));
                }
            }
        }
        for (set, members) in compared {
            for glyph in set {
                let mut list: Vec<(u16, &Face<'_>)> = Vec::new();
                for &m in &members {
                    let Some((face, widths, _)) = data.get(m) else { continue };
                    match widths.get(glyph) {
                        Some(w) => list.push((*w, *face)),
                        None => sink.push(
                            Hit::new("missing-width", format!("Glyph '{glyph}' of family '{name}' has no width in {}: it is missing, or maxp counts fewer glyphs than the font has.", face.label))
                                .place(Location { glyph: glyph.clone(), table: Some("hmtx".into()), ..face.here() }),
                        ),
                    }
                }
                list.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label.cmp(&b.1.label)));
                let (Some((min, narrow)), Some((max, wide))) = (list.first(), list.last()) else { continue };
                if u32::from(*min) * WIDTH_RATIO < u32::from(*max) {
                    sink.push(
                        Hit::new(
                            "ratio",
                            format!(
                                "Glyph '{glyph}' is {max} units wide in {}, more than {WIDTH_RATIO} times its width of {min} in {}.",
                                wide.label, narrow.label
                            ),
                        )
                        .data("max", f64::from(*max))
                        .data("min", f64::from(*min))
                        .place(Location {
                            glyph: glyph.clone(),
                            fonts: vec![narrow.id.into(), wide.id.into()],
                            table: Some("hmtx".into()),
                            ..Location::default()
                        }),
                    );
                }
            }
        }
    }
}

/// The code points a font's character map encodes.
fn codepoints(font: &FontRef<'_>) -> BTreeSet<u32> {
    font.charmap().mappings().take(MAX_CODEPOINTS).map(|(cp, _)| cp).collect()
}

/// FontBakery `googlefonts/family/equal_codepoint_coverage` and
/// `typenetwork/family/equal_numbers_of_glyphs`.
pub(crate) fn codepoint_coverage(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (name, group) in typographic(&faces) {
        if group.len() < 2 {
            continue;
        }
        let maps: Vec<(&Face<'_>, BTreeSet<u32>)> = group.iter().map(|f| (*f, codepoints(f.font))).collect();
        let mut common: Option<BTreeSet<u32>> = None;
        for (_, cps) in &maps {
            common = Some(match common {
                None => cps.clone(),
                Some(c) => c.intersection(cps).copied().collect(),
            });
        }
        let common = common.unwrap_or_default();
        for (face, cps) in &maps {
            let extra: Vec<u32> = cps.difference(&common).copied().collect();
            if !extra.is_empty() {
                sink.push(
                    Hit::new(
                        "glyphset-diverges",
                        format!(
                            "{} encodes {} characters that other fonts of family '{name}' lack: {}.",
                            face.label,
                            extra.len(),
                            list(extra.iter().map(|c| format!("U+{c:04X}")))
                        ),
                    )
                    .data("count", extra.len() as f64)
                    .place(Location { table: Some("cmap".into()), ..face.here() }),
                );
            }
        }
        let uprights: Vec<&Face<'_>> = group.iter().filter(|f| !f.italic).copied().collect();
        let italics: Vec<&Face<'_>> = group.iter().filter(|f| f.italic).copied().collect();
        if uprights.is_empty() || italics.is_empty() {
            continue;
        }
        for (what, set) in [("upright", uprights), ("italic", italics)] {
            let counts: Vec<(&Face<'_>, u16)> = set.iter().filter_map(|f| Some((*f, f.font.maxp().ok()?.num_glyphs()))).collect();
            // The first font with the most glyphs is the reference.
            let top = counts.iter().copied().fold(None, |best: Option<(&Face<'_>, u16)>, (f, n)| match best {
                Some((_, b)) if b >= n => best,
                _ => Some((f, n)),
            });
            let Some((top, most)) = top else { continue };
            let fewer: Vec<String> = counts.iter().filter(|(_, n)| *n != most).map(|(f, n)| format!("{} {n}", f.label)).collect();
            if !fewer.is_empty() {
                sink.push(
                    Hit::new(
                        "different-glyph-count",
                        format!("The {what} fonts of family '{name}' differ in glyph count: {} has {most}, {}.", top.label, list(fewer)),
                    )
                    .place(Location { table: Some("maxp".into()), field: Some("numGlyphs".into()), ..among(&set) }),
                );
            }
        }
    }
}

/// FontBakery `typenetwork/family/tnum_horizontal_metrics` and
/// `googlefonts/family/tnum_horizontal_metrics`.
pub(crate) fn tabular_figure_widths(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    let mut any_names = false;
    for (name, group) in typographic(&faces) {
        // Width → the tabular glyphs (font, glyph) with it, in the order widths are first seen.
        let mut by_width: Vec<(u16, Vec<(&Face<'_>, String)>)> = Vec::new();
        for face in &group {
            let (names, real) = glyph_names(face.font);
            any_names |= real;
            if !real {
                continue;
            }
            let widths = widths(face.font, &names);
            for glyph in names.iter().filter(|g| TABULAR_SUFFIXES.iter().any(|s| g.contains(s))) {
                let Some(w) = widths.get(glyph).copied() else { continue };
                match by_width.iter_mut().find(|(x, _)| *x == w) {
                    Some((_, glyphs)) => glyphs.push((*face, glyph.clone())),
                    None => by_width.push((w, vec![(*face, glyph.clone())])),
                }
            }
        }
        let mut best: Option<(usize, u16)> = None;
        for (i, (w, glyphs)) in by_width.iter().enumerate() {
            if best.is_none_or(|(b, _)| by_width.get(b).is_some_and(|(_, g)| glyphs.len() > g.len())) {
                best = Some((i, *w));
            }
        }
        let Some((best, common)) = best else { continue };
        by_width.remove(best);
        let half = (f64::from(common) / 2.0).round_ties_even();
        if let Some(pos) = by_width.iter().position(|(w, _)| f64::from(*w) == half) {
            let (w, glyphs) = by_width.remove(pos);
            sink.push(
                Hit::new(
                    "half-widths",
                    format!(
                        "Family '{name}' has tabular glyphs of half the common width ({w} of {common}): {}.",
                        list(glyphs.iter().map(|(_, g)| g.clone()))
                    ),
                )
                .data("width", f64::from(w)),
            );
        }
        if !by_width.is_empty() {
            let detail = list(by_width.iter().map(|(w, glyphs)| format!("{w}: {}", list(glyphs.iter().map(|(f, g)| format!("{g} in {}", f.label))))));
            sink.push(
                Hit::new(
                    "inconsistent-widths",
                    format!("The most common tabular width in family '{name}' is {common}, but other tabular glyphs differ: {detail}."),
                )
                .data("width", f64::from(common))
                .place(Location { table: Some("hmtx".into()), ..among(&group) }),
            );
        }
    }
    if !any_names {
        sink.skip("the fonts have no glyph names to find tabular glyphs by");
    }
}

// ---------------------------------------------------------------------------------------------
// Variations and layout
// ---------------------------------------------------------------------------------------------

/// FontBakery `opentype/varfont/family_axis_ranges` and `varfont/consistent_axes`.
pub(crate) fn axis_ranges(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (name, group) in typographic(&faces) {
        type AxisRanges = Vec<(Tag, [i32; 3])>;
        let axes: Vec<(&Face<'_>, AxisRanges)> = group
            .iter()
            .filter_map(|f| {
                let fvar = f.font.fvar().ok()?;
                let axes = fvar.axes().ok()?;
                Some((
                    *f,
                    axes.iter().map(|a| (a.axis_tag(), [a.min_value().to_bits(), a.default_value().to_bits(), a.max_value().to_bits()])).collect(),
                ))
            })
            .collect();
        if axes.len() < 2 {
            continue;
        }
        let mut tags: Vec<Tag> = Vec::new();
        for (_, axis_list) in &axes {
            for (tag, _) in axis_list {
                if !tags.contains(tag) {
                    tags.push(*tag);
                }
            }
        }
        for tag in tags {
            let mut ranges: Vec<(&Face<'_>, [i32; 3])> = Vec::new();
            for (face, axis_list) in &axes {
                match axis_list.iter().find(|(t, _)| *t == tag) {
                    Some((_, r)) => ranges.push((*face, *r)),
                    None => sink.push(
                        Hit::new(
                            "missing-axis",
                            format!("The variable font {} of family '{name}' has no '{tag}' axis, which other fonts of the family have.", face.label),
                        )
                        .place(Location { axis: Some(tag.to_string()), table: Some("fvar".into()), ..face.here() }),
                    ),
                }
            }
            let distinct: BTreeSet<[i32; 3]> = ranges.iter().map(|(_, r)| *r).collect();
            if distinct.len() > 1 {
                let detail = list(ranges.iter().map(|(f, r)| format!("{} {}", f.label, range_text(r))));
                sink.push(Hit::new("axis-range-mismatch", format!("The '{tag}' axis has different ranges across family '{name}': {detail}.")).place(
                    Location {
                        axis: Some(tag.to_string()),
                        table: Some("fvar".into()),
                        fonts: ranges.iter().map(|(f, _)| f.id.to_string()).collect(),
                        ..Location::default()
                    },
                ));
            }
        }
    }
}

fn range_text(&[min, default, max]: &[i32; 3]) -> String {
    let v = |bits: i32| crate::num(f64::from(bits) / 65536.0);
    format!("{} / {} / {}", v(min), v(default), v(max))
}

/// (table, script, language, feature) → lookup indices. A language with no features has one
/// entry with an empty feature tag.
type LayoutMap = BTreeMap<(&'static str, String, String, String), Vec<u16>>;

/// The script, language and feature structure of GSUB and GPOS; `None` when it has more than
/// [`MAX_LAYOUT_ENTRIES`] entries.
fn layout_map(font: &FontRef<'_>) -> Option<LayoutMap> {
    let mut map = LayoutMap::new();
    let mut budget = MAX_LAYOUT_ENTRIES;
    if let Ok(gsub) = font.gsub()
        && let (Ok(s), Ok(f)) = (gsub.script_list(), gsub.feature_list())
    {
        add_layout("GSUB", &s, &f, &mut map, &mut budget)?;
    }
    if let Ok(gpos) = font.gpos()
        && let (Ok(s), Ok(f)) = (gpos.script_list(), gpos.feature_list())
    {
        add_layout("GPOS", &s, &f, &mut map, &mut budget)?;
    }
    Some(map)
}

fn add_layout(table: &'static str, scripts: &ScriptList<'_>, features: &FeatureList<'_>, map: &mut LayoutMap, budget: &mut usize) -> Option<()> {
    let records = features.feature_records();
    for srec in scripts.script_records() {
        let stag = srec.script_tag().to_string();
        let Ok(script) = srec.script(scripts.offset_data()) else { continue };
        let mut langs = Vec::new();
        for lrec in script.lang_sys_records() {
            *budget = budget.checked_sub(1)?;
            if let Ok(ls) = lrec.lang_sys(script.offset_data()) {
                langs.push((lrec.lang_sys_tag().to_string(), ls));
            }
        }
        if let Some(Ok(ls)) = script.default_lang_sys() {
            langs.push(("dflt".to_string(), ls));
        }
        for (ltag, ls) in langs {
            map.insert((table, stag.clone(), ltag.clone(), String::new()), Vec::new());
            let required = ls.required_feature_index();
            let indices = ls.feature_indices().iter().map(|i| i.get()).chain((required != 0xFFFF).then_some(required));
            for index in indices {
                *budget = budget.checked_sub(1)?;
                let Some(frec) = records.get(usize::from(index)) else { continue };
                let lookups: Vec<u16> =
                    frec.feature(features.offset_data()).map(|f| f.lookup_list_indices().iter().map(|i| i.get()).collect()).unwrap_or_default();
                *budget = budget.checked_sub(lookups.len())?;
                map.insert((table, stag.clone(), ltag.clone(), frec.feature_tag().to_string()), lookups);
            }
        }
    }
    Some(())
}

/// comparefamily family test 12.
pub(crate) fn layout_consistency(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (name, group) in typographic(&faces) {
        if group.len() < 2 {
            continue;
        }
        let mut kinds: Vec<(LayoutMap, Vec<&Face<'_>>)> = Vec::new();
        for face in &group {
            let Some(map) = layout_map(face.font) else {
                sink.skip(format!("{} has more than {MAX_LAYOUT_ENTRIES} layout entries", face.label));
                return;
            };
            match kinds.iter_mut().find(|(m, _)| *m == map) {
                Some((_, members)) => members.push(*face),
                None => kinds.push((map, vec![*face])),
            }
        }
        let Some(((first, first_fonts), rest)) = kinds.split_first() else { continue };
        if rest.is_empty() {
            continue;
        }
        let describe = |(t, s, l, f): &(&str, String, String, String)| if f.is_empty() { format!("{t} {s}/{l}") } else { format!("{t} {s}/{l} {f}") };
        let mut differences: Vec<String> = Vec::new();
        for (map, fonts) in rest {
            let keys: BTreeSet<&(&str, String, String, String)> = first.keys().chain(map.keys()).collect();
            let changed = keys.into_iter().filter(|k| first.get(*k) != map.get(*k)).map(describe);
            differences.push(format!("{} differ from {} in {}", labels(fonts), labels(first_fonts), list(changed)));
        }
        sink.push(
            Hit::new(
                "differs",
                format!("The fonts of family '{name}' do not have the same scripts, languages, features and lookups ({} variants; expected when their character sets differ): {}.", kinds.len(), differences.join("; ")),
            )
            .data("variants", kinds.len() as f64)
            .place(among(&group)),
        );
    }
}

/// The size feature's parameters: design size, subfamily identifier, menu name ID, range start
/// and end. Fonts without the feature read as zeros.
fn size_params(font: &FontRef<'_>) -> [u16; 5] {
    let read = || -> Option<[u16; 5]> {
        let gpos = font.gpos().ok()?;
        let list = gpos.feature_list().ok()?;
        let rec = list.feature_records().iter().find(|r| r.feature_tag() == Tag::new(b"size"))?;
        let feature = rec.feature(list.offset_data()).ok()?;
        match feature.feature_params()? {
            Ok(FeatureParams::Size(p)) => Some([p.design_size(), p.identifier(), p.name_entry(), p.range_start(), p.range_end()]),
            _ => None,
        }
    };
    read().unwrap_or_default()
}

/// comparefamily family test 4.
pub(crate) fn size_feature_groups(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    for (name, group) in typographic(&faces) {
        let sizes: Vec<(&Face<'_>, [u16; 5], String)> = group
            .iter()
            .map(|f| {
                let p = size_params(f.font);
                let [_, _, name_id, _, _] = p;
                let menu = if name_id == 0 {
                    "(no size feature)".to_string()
                } else {
                    f.names.iter().find(|n| n.id == name_id).map(|n| n.text.clone()).unwrap_or_default()
                };
                (*f, p, menu)
            })
            .collect();
        if sizes.iter().all(|(_, [_, _, name_id, _, _], _)| *name_id == 0) {
            continue;
        }
        let mut by_id: Vec<(u16, Vec<usize>)> = Vec::new();
        let mut by_menu: Vec<(&str, Vec<usize>)> = Vec::new();
        for (i, (_, [_, id, _, _, _], menu)) in sizes.iter().enumerate() {
            match by_id.iter_mut().find(|(k, _)| k == id) {
                Some((_, m)) => m.push(i),
                None => by_id.push((*id, vec![i])),
            }
            match by_menu.iter_mut().find(|(k, _)| k == menu) {
                Some((_, m)) => m.push(i),
                None => by_menu.push((menu.as_str(), vec![i])),
            }
        }
        let partition = |groups: Vec<Vec<usize>>| groups.into_iter().collect::<BTreeSet<Vec<usize>>>();
        if partition(by_id.iter().map(|(_, m)| m.clone()).collect()) != partition(by_menu.iter().map(|(_, m)| m.clone()).collect()) {
            sink.push(
                Hit::new(
                    "menu-names-differ",
                    format!("In family '{name}', fonts with the same size subfamily identifier do not all have the same size menu name."),
                )
                .place(among(&group)),
            );
        }
        let at = |i: &usize| sizes.get(*i);
        let mut previous: Option<(u16, usize)> = None;
        for (id, members) in &by_id {
            if let Some((prev_id, prev_len)) = previous
                && prev_len != members.len()
            {
                sink.push(
                    Hit::new(
                        "group-sizes-differ",
                        format!("In family '{name}', size group {id} has {} fonts but size group {prev_id} has {prev_len}.", members.len()),
                    )
                    .place(among(&members.iter().filter_map(|i| at(i).map(|s| s.0)).collect::<Vec<_>>())),
                );
            }
            previous = Some((*id, members.len()));
            if members.len() < 2 {
                continue;
            }
            let mut ranges: Vec<(u16, u16, &Face<'_>)> =
                members.iter().filter_map(at).map(|(f, [_, _, _, start, end], _)| (*start, *end, *f)).collect();
            ranges.sort_by_key(|a| (a.0, a.1));
            for pair in ranges.windows(2) {
                let [(_, end, a), (start, next_end, b)] = pair else { continue };
                if start != end {
                    sink.push(
                        Hit::new(
                            "ranges-not-contiguous",
                            format!(
                                "In family '{name}', size group {id} is not contiguous: {} covers up to {} and {} starts at {} (design ranges in tenths of a point; next ends at {}).",
                                a.label, end, b.label, start, next_end
                            ),
                        )
                        .place(Location { feature: Some("size".into()), table: Some("GPOS".into()), fonts: vec![a.id.into(), b.id.into()], ..Location::default() }),
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// PostScript hinting values
// ---------------------------------------------------------------------------------------------

/// The Private DICT values the blue checks compare, as absolute values (the DICT stores deltas).
#[derive(Clone, Debug, Default, PartialEq)]
struct PsPrivate {
    blues: Vec<f64>,
    other: Vec<f64>,
    family: Option<Vec<f64>>,
    family_other: Option<Vec<f64>>,
    blue_scale: f64,
    blue_shift: f64,
}

enum Ps {
    NotCff,
    Cid,
    Plain(PsPrivate),
}

impl Ps {
    fn is_cid(&self) -> bool {
        matches!(self, Ps::Cid)
    }

    fn plain(self) -> Option<PsPrivate> {
        match self {
            Ps::Plain(p) => Some(p),
            _ => None,
        }
    }
}

fn number(n: Number) -> f64 {
    match n {
        Number::I32(i) => f64::from(i),
        Number::Fixed(f) => f.to_f64(),
    }
}

/// Each operator of a DICT with its operands, at most [`MAX_DICT_ENTRIES`] of them.
fn dict_ops(data: &[u8]) -> Vec<(Operator, Vec<f64>)> {
    let mut out = Vec::new();
    let mut stack: Vec<f64> = Vec::new();
    for token in dict::tokens(data) {
        match token {
            Ok(Token::Operand(n, _)) => {
                if stack.len() < MAX_DICT_OPERANDS {
                    stack.push(number(n));
                }
            }
            Ok(Token::Operator(op)) => out.push((op, std::mem::take(&mut stack))),
            Ok(_) => stack.clear(),
            Err(_) => break,
        }
        if out.len() >= MAX_DICT_ENTRIES {
            break;
        }
    }
    out
}

fn absolute(deltas: &[f64]) -> Vec<f64> {
    let mut acc = 0.0;
    deltas
        .iter()
        .map(|d| {
            acc += d;
            acc
        })
        .collect()
}

/// The first font's Private DICT of a CFF table.
fn ps_info(font: &FontRef<'_>) -> Ps {
    let Ok(cff) = font.cff() else { return Ps::NotCff };
    let Some(top) = cff.top_dicts().get(0) else { return Ps::NotCff };
    let ops = dict_ops(top);
    if ops.iter().any(|(op, _)| *op == Operator::Ros) {
        return Ps::Cid;
    }
    let Some(range) = ops.iter().find(|(op, _)| *op == Operator::PrivateDictRange).and_then(|(_, args)| match args.as_slice() {
        [.., size, offset] if size.is_finite() && offset.is_finite() && *size >= 0.0 && *offset >= 0.0 => {
            let (size, offset) = (*size as usize, *offset as usize);
            Some(offset..offset.checked_add(size)?)
        }
        _ => None,
    }) else {
        return Ps::Plain(PsPrivate { blue_scale: DEFAULT_BLUE_SCALE, blue_shift: DEFAULT_BLUE_SHIFT, ..PsPrivate::default() });
    };
    let Some(data) = cff.offset_data().as_bytes().get(range) else { return Ps::NotCff };
    let mut p = PsPrivate { blue_scale: DEFAULT_BLUE_SCALE, blue_shift: DEFAULT_BLUE_SHIFT, ..PsPrivate::default() };
    for (op, args) in dict_ops(data) {
        match op {
            Operator::BlueValues => p.blues = absolute(&args),
            Operator::OtherBlues => p.other = absolute(&args),
            Operator::FamilyBlues => p.family = Some(absolute(&args)),
            Operator::FamilyOtherBlues => p.family_other = Some(absolute(&args)),
            Operator::BlueScale => p.blue_scale = args.last().copied().unwrap_or(DEFAULT_BLUE_SCALE),
            Operator::BlueShift => p.blue_shift = args.last().copied().unwrap_or(DEFAULT_BLUE_SHIFT),
            _ => {}
        }
    }
    Ps::Plain(p)
}

/// comparefamily runs its blue tests only when the first font has a CFF table that is not
/// CID-keyed; the same holds here. Skips the check otherwise.
fn blue_family(faces: &[Face<'_>], sink: &mut Sink) -> bool {
    let ok = faces.first().is_some_and(|f| matches!(ps_info(f.font), Ps::Plain(_)));
    if !ok {
        sink.skip("the first font has no CFF outlines, or they are CID-keyed");
    }
    ok
}

fn numbers(v: &[f64]) -> String {
    format!("[{}]", v.iter().map(|x| crate::num(*x)).collect::<Vec<_>>().join(" "))
}

/// comparefamily family test 18 (the array sizes; the zone tests of the same test belong to the
/// hinting checks).
pub(crate) fn blue_value_array_sizes(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    if !blue_family(&faces, sink) {
        return;
    }
    for (name, group) in style_linked(&faces) {
        let mut subgroups: Vec<(Vec<String>, Vec<&Face<'_>>)> = Vec::new();
        for face in group {
            let (names, _) = glyph_names(face.font);
            match subgroups.iter_mut().find(|(n, _)| *n == names) {
                Some((_, m)) => m.push(face),
                None => subgroups.push((names, vec![face])),
            }
        }
        for (_, members) in subgroups {
            let Some((first, rest)) = members.split_first() else { continue };
            let Some(base) = ps_info(first.font).plain() else { break };
            for face in rest {
                let p = ps_info(face.font).plain().unwrap_or_default();
                for (code, field, a, b) in [
                    ("blue-values-count-differs", "BlueValues", &base.blues, &p.blues),
                    ("other-blues-count-differs", "OtherBlues", &base.other, &p.other),
                ] {
                    if a.len() != b.len() {
                        sink.push(
                            Hit::new(
                                code,
                                format!(
                                    "In group '{name}', {} has {} {field} values {} but {} has {} {}.",
                                    face.label,
                                    b.len(),
                                    numbers(b),
                                    first.label,
                                    a.len(),
                                    numbers(a)
                                ),
                            )
                            .data("count", b.len() as f64)
                            .data("expected", a.len() as f64)
                            .place(Location { fonts: vec![first.id.into(), face.id.into()], ..face.field("CFF ", field) }),
                        );
                    }
                }
            }
        }
    }
}

/// comparefamily family test 19, with the zone limits of the specification: 7 zones for
/// FamilyBlues, 5 for FamilyOtherBlues.
pub(crate) fn family_blues(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    if !blue_family(&faces, sink) {
        return;
    }
    for (name, group) in typographic(&faces) {
        if group.len() < 2 {
            continue;
        }
        let Some(privates) = group.iter().map(|f| Some((*f, ps_info(f.font).plain()?))).collect::<Option<Vec<_>>>() else { continue };
        let any_family = privates.iter().any(|(_, p)| p.family.is_some() || p.family_other.is_some());
        if !any_family {
            if let Some((_, first)) = privates.first()
                && privates.iter().any(|(_, p)| p.blues != first.blues)
            {
                sink.push(
                    Hit::new(
                        "no-family-blues",
                        format!("The fonts of family '{name}' have different BlueValues but no FamilyBlues to align them on a shared line."),
                    )
                    .place(among(&group)),
                );
            }
            continue;
        }
        type Pick = fn(&PsPrivate) -> (Option<&Vec<f64>>, &Vec<f64>);
        let picks: [(&str, &str, usize, Pick); 2] = [
            ("FamilyBlues", "BlueValues", 14, |p| (p.family.as_ref(), &p.blues)),
            ("FamilyOtherBlues", "OtherBlues", 10, |p| (p.family_other.as_ref(), &p.other)),
        ];
        for (key, base_key, limit, pick) in picks {
            if !privates.iter().any(|(_, p)| pick(p).0.is_some()) {
                continue;
            }
            let mut values: Vec<(Vec<f64>, Vec<&Face<'_>>)> = Vec::new();
            for (face, p) in &privates {
                let (family, own) = pick(p);
                let v = family.unwrap_or(own).clone();
                match values.iter_mut().find(|(x, _)| *x == v) {
                    Some((_, m)) => m.push(*face),
                    None => values.push((v, vec![*face])),
                }
            }
            for (v, fonts) in &values {
                let has_base = privates.iter().any(|(f, p)| fonts.iter().any(|g| std::ptr::eq(*g, *f)) && pick(p).1 == v);
                let place = Location { table: Some("CFF ".into()), field: Some(key.into()), ..among(fonts) };
                if fonts.len() > 1 && !has_base {
                    sink.push(
                        Hit::new(
                            "no-base-font",
                            format!(
                                "In family '{name}', {} share the {key} {} but none of them has it as its {base_key}.",
                                labels(fonts),
                                numbers(v)
                            ),
                        )
                        .place(place.clone()),
                    );
                } else if fonts.len() == 1 && !v.is_empty() {
                    sink.push(
                        Hit::new("unique", format!("In family '{name}', only {} has the {key} {}.", labels(fonts), numbers(v))).place(place.clone()),
                    );
                }
                if v.len() > limit {
                    sink.push(
                        Hit::new(
                            "too-many-zones",
                            format!(
                                "In family '{name}', {key} {} has {} values; at most {limit} are allowed ({}).",
                                numbers(v),
                                v.len(),
                                labels(fonts)
                            ),
                        )
                        .data("count", v.len() as f64)
                        .place(place.clone()),
                    );
                }
                if v.len() % 2 != 0 {
                    sink.push(
                        Hit::new(
                            "odd-count",
                            format!("In family '{name}', {key} {} has an odd number of values; zones are pairs ({}).", numbers(v), labels(fonts)),
                        )
                        .place(place),
                    );
                }
            }
        }
    }
}

/// comparefamily family tests 20 and 21.
pub(crate) fn blue_scale_and_shift(ctx: &FamilyCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let faces = faces(ctx.fonts);
    if !blue_family(&faces, sink) {
        return;
    }
    for (name, group) in style_linked(&faces) {
        let Some((first, rest)) = group.split_first() else { continue };
        let Some(base) = ps_info(first.font).plain() else { continue };
        for face in rest {
            let Some(p) = ps_info(face.font).plain() else { break };
            let pairs = [
                ("blue-scale-differs", "BlueScale", p.blue_scale, base.blue_scale, BLUE_SCALE_EPSILON),
                ("blue-shift-differs", "BlueShift", p.blue_shift, base.blue_shift, 1e-6),
            ];
            for (code, field, value, expected, epsilon) in pairs {
                if (value - expected).abs() > epsilon {
                    sink.push(
                        Hit::new(
                            code,
                            format!(
                                "In group '{name}', {} has {field} {} but {} has {}.",
                                face.label,
                                crate::num(value),
                                first.label,
                                crate::num(expected)
                            ),
                        )
                        .data("value", value)
                        .data("expected", expected)
                        .place(Location { fonts: vec![first.id.into(), face.id.into()], ..face.field("CFF ", field) }),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CHECKS, Config, LayerReport, NoServices, audit_family};

    #[derive(Clone, Default)]
    struct Private {
        blues: Vec<i32>,
        other: Vec<i32>,
        family: Option<Vec<i32>>,
        shift: Option<i32>,
    }

    /// Tag, minimum, default and maximum of one fvar axis.
    type FvarAxis = ([u8; 4], f64, f64, f64);

    /// One font of a synthetic family, as data the builder turns into an sfnt.
    #[derive(Clone)]
    struct Spec {
        family: String,
        typo: Option<String>,
        style: String,
        bold: bool,
        italic: bool,
        weight: u16,
        width: u16,
        fs_type: u16,
        panose: [u8; 10],
        wws: bool,
        upm: u16,
        revision: i32,
        underline: (i16, i16),
        /// Typo ascender, descender, line gap; win ascent, descent; hhea ascender, descender, line gap.
        vm: [i16; 8],
        glyphs: Vec<(String, u16, Option<u32>)>,
        mac_names: bool,
        copyright: String,
        full: Option<String>,
        extra_names: Vec<(u16, u16, u16, u16, String)>,
        fvar: Option<Vec<FvarAxis>>,
        gsub: Option<Vec<([u8; 4], Vec<u16>)>>,
        size: Option<[u16; 5]>,
        cff: Option<Private>,
    }

    fn member(style: &str, bold: bool, italic: bool) -> Spec {
        let g = |n: &str, w: u16, cp: Option<char>| (n.to_string(), w, cp.map(u32::from));
        Spec {
            family: "Test".into(),
            typo: None,
            style: style.into(),
            bold,
            italic,
            weight: if bold { 700 } else { 400 },
            width: 5,
            fs_type: 0,
            panose: [2, 11, 5, 3, 2, 2, 2, 2, 2, 4],
            wws: false,
            upm: 1000,
            revision: 0x0001_0000,
            underline: (-100, 50),
            vm: [800, -200, 0, 950, 250, 800, -200, 0],
            glyphs: vec![g(".notdef", 500, None), g("a", 500, Some('a')), g("b", 520, Some('b')), g("one.tnum", 600, None), g("two.tnum", 600, None)],
            mac_names: false,
            copyright: "Copyright 2020 Test Foundry".into(),
            full: None,
            extra_names: Vec::new(),
            fvar: None,
            gsub: None,
            size: None,
            cff: None,
        }
    }

    fn clean() -> Vec<Spec> {
        vec![member("Regular", false, false), member("Italic", false, true), member("Bold", true, false), member("Bold Italic", true, true)]
    }

    fn clean_cff() -> Vec<Spec> {
        let mut specs = clean();
        for s in &mut specs {
            s.cff = Some(Private { blues: vec![-12, 0, 500, 512], ..Private::default() });
        }
        specs
    }

    fn u16be(v: &mut Vec<u8>, x: u16) {
        v.extend_from_slice(&x.to_be_bytes());
    }
    fn i16be(v: &mut Vec<u8>, x: i16) {
        v.extend_from_slice(&x.to_be_bytes());
    }
    fn u32be(v: &mut Vec<u8>, x: u32) {
        v.extend_from_slice(&x.to_be_bytes());
    }
    fn fixed(v: &mut Vec<u8>, x: f64) {
        v.extend_from_slice(&((x * 65536.0).round() as i32).to_be_bytes());
    }

    fn sfnt(mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
        tables.sort_by_key(|a| a.0);
        let cff = tables.iter().any(|(t, _)| t == b"CFF ");
        let mut out = Vec::new();
        u32be(&mut out, if cff { 0x4F54_544F } else { 0x0001_0000 });
        u16be(&mut out, tables.len() as u16);
        out.extend_from_slice(&[0; 6]);
        let mut offset = 12 + 16 * tables.len();
        let mut body = Vec::new();
        for (tag, data) in &tables {
            out.extend_from_slice(tag);
            u32be(&mut out, 0);
            u32be(&mut out, offset as u32);
            u32be(&mut out, data.len() as u32);
            body.extend_from_slice(data);
            while body.len() % 4 != 0 {
                body.push(0);
            }
            offset = 12 + 16 * tables.len() + body.len();
        }
        out.extend(body);
        out
    }

    fn head(s: &Spec) -> Vec<u8> {
        let mut v = Vec::new();
        u32be(&mut v, 0x0001_0000);
        v.extend_from_slice(&s.revision.to_be_bytes());
        u32be(&mut v, 0);
        u32be(&mut v, 0x5F0F_3CF5);
        u16be(&mut v, 0);
        u16be(&mut v, s.upm);
        v.extend_from_slice(&[0; 16]);
        for x in [0i16, -200, 1000, 800] {
            i16be(&mut v, x);
        }
        u16be(&mut v, u16::from(s.bold) | (u16::from(s.italic) << 1));
        u16be(&mut v, 8);
        i16be(&mut v, 2);
        i16be(&mut v, 0);
        i16be(&mut v, 0);
        v
    }

    fn name(s: &Spec) -> Vec<u8> {
        let full = s.full.clone().unwrap_or_else(|| format!("{} {}", s.family, s.style));
        let ps = format!("{}-{}", s.family, s.style.replace(' ', ""));
        let mut records: Vec<(u16, u16, u16, u16, String)> = Vec::new();
        let mut add = |p: u16, e: u16, l: u16| {
            records.push((p, e, l, 0, s.copyright.clone()));
            records.push((p, e, l, 1, s.family.clone()));
            records.push((p, e, l, 2, s.style.clone()));
            records.push((p, e, l, 4, full.clone()));
            records.push((p, e, l, 6, ps.clone()));
        };
        add(3, 1, 0x409);
        if s.mac_names {
            add(1, 0, 0);
        }
        if let Some(t) = &s.typo {
            records.push((3, 1, 0x409, 16, t.clone()));
        }
        records.extend(s.extra_names.iter().cloned());
        records.sort_by_key(|a| (a.0, a.1, a.2, a.3));
        let mut strings = Vec::new();
        let mut v = Vec::new();
        u16be(&mut v, 0);
        u16be(&mut v, records.len() as u16);
        u16be(&mut v, (6 + 12 * records.len()) as u16);
        for (p, e, l, id, text) in &records {
            let bytes: Vec<u8> = if *p == 3 { text.encode_utf16().flat_map(u16::to_be_bytes).collect() } else { text.bytes().collect() };
            for x in [*p, *e, *l, *id, bytes.len() as u16, strings.len() as u16] {
                u16be(&mut v, x);
            }
            strings.extend(bytes);
        }
        v.extend(strings);
        v
    }

    fn os2(s: &Spec) -> Vec<u8> {
        let mut v = Vec::new();
        u16be(&mut v, 4);
        i16be(&mut v, 500);
        u16be(&mut v, s.weight);
        u16be(&mut v, s.width);
        u16be(&mut v, s.fs_type);
        v.extend_from_slice(&[0; 22]);
        v.extend_from_slice(&s.panose);
        v.extend_from_slice(&[0; 16]);
        v.extend_from_slice(b"TEST");
        let regular = !s.bold && !s.italic;
        let sel = u16::from(s.italic) | (u16::from(s.bold) << 5) | (u16::from(regular) << 6) | (u16::from(s.wws) << 8);
        u16be(&mut v, sel);
        u16be(&mut v, 0x20);
        u16be(&mut v, 0x62);
        for x in [s.vm[0], s.vm[1], s.vm[2]] {
            i16be(&mut v, x);
        }
        u16be(&mut v, s.vm[3] as u16);
        u16be(&mut v, s.vm[4] as u16);
        v.extend_from_slice(&[0; 8]);
        i16be(&mut v, 500);
        i16be(&mut v, 700);
        v.extend_from_slice(&[0; 6]);
        assert_eq!(v.len(), 96);
        v
    }

    fn post(s: &Spec) -> Vec<u8> {
        let mut v = Vec::new();
        u32be(&mut v, 0x0002_0000);
        u32be(&mut v, 0);
        i16be(&mut v, s.underline.0);
        i16be(&mut v, s.underline.1);
        v.extend_from_slice(&[0; 20]);
        u16be(&mut v, s.glyphs.len() as u16);
        for i in 0..s.glyphs.len() {
            u16be(&mut v, 258 + i as u16);
        }
        for (n, _, _) in &s.glyphs {
            v.push(n.len() as u8);
            v.extend_from_slice(n.as_bytes());
        }
        v
    }

    fn hhea(s: &Spec) -> Vec<u8> {
        let mut v = Vec::new();
        u32be(&mut v, 0x0001_0000);
        for x in [s.vm[5], s.vm[6], s.vm[7]] {
            i16be(&mut v, x);
        }
        v.extend_from_slice(&[0; 24]);
        u16be(&mut v, s.glyphs.len() as u16);
        v
    }

    fn cmap(s: &Spec) -> Vec<u8> {
        let mut map: Vec<(u32, u32)> = s.glyphs.iter().enumerate().filter_map(|(gid, (_, _, cp))| Some(((*cp)?, gid as u32))).collect();
        map.sort();
        let mut v = Vec::new();
        u16be(&mut v, 0);
        u16be(&mut v, 1);
        u16be(&mut v, 3);
        u16be(&mut v, 10);
        u32be(&mut v, 12);
        u16be(&mut v, 12);
        u16be(&mut v, 0);
        u32be(&mut v, 16 + 12 * map.len() as u32);
        u32be(&mut v, 0);
        u32be(&mut v, map.len() as u32);
        for (cp, gid) in map {
            u32be(&mut v, cp);
            u32be(&mut v, cp);
            u32be(&mut v, gid);
        }
        v
    }

    fn fvar(axes: &[([u8; 4], f64, f64, f64)]) -> Vec<u8> {
        let mut v = Vec::new();
        for x in [1, 0, 16, 2, axes.len() as u16, 20, 0, 4 + 4 * axes.len() as u16] {
            u16be(&mut v, x);
        }
        for (i, (tag, min, def, max)) in axes.iter().enumerate() {
            v.extend_from_slice(tag);
            fixed(&mut v, *min);
            fixed(&mut v, *def);
            fixed(&mut v, *max);
            u16be(&mut v, 0);
            u16be(&mut v, 256 + i as u16);
        }
        v
    }

    type FeatureSpec = ([u8; 4], Vec<u16>, Option<[u16; 5]>);
    /// A GSUB or GPOS table with one script (latn, default language) using every feature.
    fn layout(features: &[FeatureSpec]) -> Vec<u8> {
        let n = features.len();
        let feature_list = 28 + 2 * n;
        let tables_len: usize = features.iter().map(|(_, l, p)| 4 + 2 * l.len() + if p.is_some() { 10 } else { 0 }).sum();
        let lookup_list = feature_list + 2 + 6 * n + tables_len;
        let mut v = Vec::new();
        for x in [1, 0, 10, feature_list as u16, lookup_list as u16] {
            u16be(&mut v, x);
        }
        u16be(&mut v, 1);
        v.extend_from_slice(b"latn");
        u16be(&mut v, 8);
        u16be(&mut v, 4);
        u16be(&mut v, 0);
        u16be(&mut v, 0);
        u16be(&mut v, 0xFFFF);
        u16be(&mut v, n as u16);
        for i in 0..n {
            u16be(&mut v, i as u16);
        }
        assert_eq!(v.len(), feature_list);
        u16be(&mut v, n as u16);
        let mut offset = 2 + 6 * n;
        for (tag, lookups, params) in features {
            v.extend_from_slice(tag);
            u16be(&mut v, offset as u16);
            offset += 4 + 2 * lookups.len() + if params.is_some() { 10 } else { 0 };
        }
        for (_, lookups, params) in features {
            u16be(&mut v, if params.is_some() { (4 + 2 * lookups.len()) as u16 } else { 0 });
            u16be(&mut v, lookups.len() as u16);
            for l in lookups {
                u16be(&mut v, *l);
            }
            for x in params.iter().flatten() {
                u16be(&mut v, *x);
            }
        }
        assert_eq!(v.len(), lookup_list);
        u16be(&mut v, 0);
        v
    }

    /// A bare CFF table whose one Private DICT holds the given values; integers only.
    fn cff(p: &Private) -> Vec<u8> {
        fn int(v: &mut Vec<u8>, x: i32) {
            v.push(29);
            v.extend_from_slice(&x.to_be_bytes());
        }
        let mut private = Vec::new();
        let mut array = |values: &[i32], op: &[u8]| {
            let mut last = 0;
            for x in values {
                int(&mut private, x - last);
                last = *x;
            }
            private.extend_from_slice(op);
        };
        if !p.blues.is_empty() {
            array(&p.blues, &[6]);
        }
        if !p.other.is_empty() {
            array(&p.other, &[7]);
        }
        if let Some(f) = &p.family {
            array(f, &[8]);
        }
        if let Some(shift) = p.shift {
            int(&mut private, shift);
            private.extend_from_slice(&[12, 10]);
        }
        let mut v = vec![1, 0, 4, 1];
        v.extend_from_slice(&[0, 1, 1, 1, 5]);
        v.extend_from_slice(b"Test");
        v.extend_from_slice(&[0, 1, 1, 1, 12]);
        int(&mut v, private.len() as i32);
        int(&mut v, 33);
        v.push(18);
        v.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(v.len(), 33);
        v.extend(private);
        v
    }

    fn build(s: &Spec) -> Vec<u8> {
        let n = s.glyphs.len() as u16;
        let mut maxp = Vec::new();
        u32be(&mut maxp, 0x0000_5000);
        u16be(&mut maxp, n);
        let mut hmtx = Vec::new();
        for (_, w, _) in &s.glyphs {
            u16be(&mut hmtx, *w);
            i16be(&mut hmtx, 0);
        }
        let mut tables = vec![
            (*b"head", head(s)),
            (*b"name", name(s)),
            (*b"OS/2", os2(s)),
            (*b"post", post(s)),
            (*b"hhea", hhea(s)),
            (*b"maxp", maxp),
            (*b"hmtx", hmtx),
            (*b"cmap", cmap(s)),
        ];
        if let Some(axes) = &s.fvar {
            tables.push((*b"fvar", fvar(axes)));
        }
        if let Some(features) = &s.gsub {
            let features: Vec<_> = features.iter().map(|(t, l)| (*t, l.clone(), None)).collect();
            tables.push((*b"GSUB", layout(&features)));
        }
        if let Some(size) = s.size {
            tables.push((*b"GPOS", layout(&[(*b"size", Vec::new(), Some(size))])));
        }
        if let Some(p) = &s.cff {
            tables.push((*b"CFF ", cff(p)));
        }
        sfnt(tables)
    }

    fn run(specs: &[Spec], check: &str, config: &Config) -> LayerReport {
        let bytes: Vec<Vec<u8>> = specs.iter().map(build).collect();
        let ids: Vec<String> = (0..specs.len()).map(|i| format!("f{i}")).collect();
        let services = NoServices;
        let fonts: Vec<BinaryCtx<'_>> = bytes.iter().zip(&ids).map(|(b, id)| BinaryCtx::new(id, b, 0, &services).unwrap()).collect();
        let ctx = FamilyCtx { fonts: &fonts, services: &services };
        let spec = crate::find(check).unwrap_or_else(|| panic!("{check} is not registered"));
        audit_family(&ctx, &[spec], config)
    }

    fn family_checks() -> impl Iterator<Item = &'static str> {
        CHECKS.iter().filter(|c| c.id.starts_with("check.family.")).map(|c| c.id)
    }

    #[test]
    fn a_consistent_family_passes_every_family_check() {
        assert_eq!(family_checks().count(), 28);
        for (specs, what) in [(clean(), "TrueType"), (clean_cff(), "CFF")] {
            for check in family_checks() {
                let r = run(&specs, check, &Config::default());
                assert!(r.findings.is_empty(), "{check} on the clean {what} family: {:#?}", r.findings);
            }
        }
    }

    type Mutation = fn(&mut Vec<Spec>);

    #[test]
    fn each_family_check_reports_its_fault() {
        let cases: Vec<(&str, bool, Mutation, &str, &str)> = vec![
            ("check.family.maxFourFontsPerFamilyName", false, |f| f.push(Spec { weight: 500, ..member("Medium", false, false) }), "too-many", "fail"),
            ("check.family.uniqueStyleLinking", false, |f| f[1].italic = false, "same-style-bits", "fail"),
            ("check.family.consistentFamilyName", false, |f| f[1].typo = Some("Other".into()), "inconsistent-family-name", "fail"),
            (
                "check.family.consistentFamilyName",
                true,
                |f| f[1].extra_names.push((3, 1, 0x407, 16, "Testschrift".into())),
                "differs-in-language",
                "fail",
            ),
            ("check.family.equalVersions", false, |f| f[2].revision = 0x0002_0000, "mismatch", "warn"),
            ("check.family.panoseFamilyType", false, |f| f[1].panose[0] = 3, "inconsistency", "warn"),
            ("check.family.panoseFamilyType", true, |f| f[1].panose = [0; 10], "all-zero", "fail"),
            ("check.family.panoseFamilyType", true, |f| f[1].panose = [2, 11, 5, 3, 0, 0, 0, 0, 0, 0], "first-four-only", "warn"),
            ("check.family.underlineConsistency", false, |f| f[3].underline.1 = 80, "thickness-differs", "fail"),
            ("check.family.verticalMetrics", false, |f| f[1].vm[0] += 10, "mismatch", "fail"),
            (
                "check.family.superfamilyVerticalMetrics",
                false,
                |f| {
                    f[2].typo = Some("Sib".into());
                    f[3].typo = Some("Sib".into());
                    f[3].vm[0] += 20;
                },
                "superfamily-vertical-metrics",
                "warn",
            ),
            ("check.family.unitsPerEm", false, |f| f[1].upm = 2048, "inconsistent-upem", "fail"),
            (
                "check.family.axisRanges",
                false,
                |f| {
                    f[0].fvar = Some(vec![(*b"wght", 100.0, 400.0, 900.0)]);
                    f[1].fvar = Some(vec![(*b"wght", 100.0, 400.0, 700.0)]);
                },
                "axis-range-mismatch",
                "fail",
            ),
            ("check.family.nameRecordSets", false, |f| f[1].extra_names.push((3, 1, 0x409, 13, "License".into())), "differs", "fail"),
            ("check.family.styleLinkedGroupShape", false, |f| drop(f.remove(0)), "base-not-regular", "fail"),
            ("check.family.boldLinkWeights", false, |f| f[2].weight = 500, "bold-weight-out-of-range", "fail"),
            ("check.family.boldLinkWeights", false, |f| f[2].weight = 720, "bold-weight-not-round", "warn"),
            ("check.family.widthClass", false, |f| f[1].width = 4, "differs", "warn"),
            ("check.family.copyrightAndTrademark", false, |f| f[1].copyright = "Copyright 2021 Test Foundry".into(), "copyright-years-only", "info"),
            ("check.family.copyrightAndTrademark", false, |f| f[1].copyright = "Copyright 2020 Other Foundry".into(), "copyright-differs", "warn"),
            (
                "check.family.macWindowsMenuNames",
                false,
                |f| {
                    for s in f.iter_mut() {
                        s.mac_names = true;
                    }
                },
                "regular-names-differ",
                "fail",
            ),
            ("check.family.layoutConsistency", false, |f| f[0].gsub = Some(vec![(*b"liga", vec![0])]), "differs", "fail"),
            (
                "check.family.sizeFeatureGroups",
                false,
                |f| {
                    for s in f.iter_mut().take(2) {
                        s.size = Some([100, 1, 256, 0, 100]);
                        s.extra_names.push((3, 1, 0x409, 256, "Text".into()));
                    }
                },
                "ranges-not-contiguous",
                "fail",
            ),
            ("check.family.wwsBit", false, |f| f[0].wws = true, "mixed", "fail"),
            ("check.family.embeddingPermissions", false, |f| f[1].fs_type = 8, "differs", "fail"),
            ("check.family.glyphWidthRatio", false, |f| f[1].glyphs[1].1 = 2000, "ratio", "warn"),
            ("check.family.codepointCoverage", false, |f| f[1].glyphs[2].2 = None, "glyphset-diverges", "fail"),
            ("check.family.italicsHaveUprights", false, |f| f[3].weight = 800, "missing-upright", "fail"),
            ("check.family.tabularFigureWidths", false, |f| f[1].glyphs[3].1 = 610, "inconsistent-widths", "fail"),
            ("check.family.duplicateFullNames", false, |f| f[1].full = Some("Test Regular".into()), "duplicate-full-names", "fail"),
        ];
        let cff_cases: Vec<(&str, Mutation, &str, &str)> = vec![
            (
                "check.family.blueValueArraySizes",
                |f| {
                    if let Some(p) = &mut f[1].cff {
                        p.blues = vec![-12, 0, 450, 460, 500, 512];
                    }
                },
                "blue-values-count-differs",
                "fail",
            ),
            (
                "check.family.familyBlues",
                |f| {
                    if let Some(p) = &mut f[1].cff {
                        p.family = Some(vec![-15, 0, 480, 495]);
                    }
                },
                "unique",
                "fail",
            ),
            (
                "check.family.familyBlues",
                |f| {
                    if let Some(p) = &mut f[1].cff {
                        p.blues = vec![-10, 0, 480, 492];
                    }
                },
                "no-family-blues",
                "warn",
            ),
            (
                "check.family.blueScaleAndShift",
                |f| {
                    if let Some(p) = &mut f[1].cff {
                        p.shift = Some(10);
                    }
                },
                "blue-shift-differs",
                "info",
            ),
        ];
        let all = cases
            .into_iter()
            .map(|(c, s, m, code, sev)| (c, s, m, code, sev, false))
            .chain(cff_cases.into_iter().map(|(c, m, code, sev)| (c, false, m, code, sev, true)));
        let mut covered = BTreeSet::new();
        for (check, strict, mutate, code, severity, with_cff) in all {
            let mut specs = if with_cff { clean_cff() } else { clean() };
            mutate(&mut specs);
            let mut config = Config::default();
            if strict {
                config.set(check, "strict", 1.0).unwrap();
            }
            let r = run(&specs, check, &config);
            let hit = r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{check} did not report {code}: {:#?}", r.findings));
            assert_eq!(hit.severity.as_str(), severity, "{check} {code}");
            assert!(crate::find(check).unwrap().codes.contains(&code), "{check} does not list {code}");
            covered.insert(check);
        }
        let missing: Vec<&str> = family_checks().filter(|c| !covered.contains(c)).collect();
        assert!(missing.is_empty(), "no case for {missing:?}");
    }

    #[test]
    fn strict_rules_stay_off_by_default() {
        let mut specs = clean();
        specs[1].panose = [0; 10];
        specs[1].extra_names.push((3, 1, 0x407, 16, "Testschrift".into()));
        for check in ["check.family.panoseFamilyType", "check.family.consistentFamilyName"] {
            let r = run(&specs, check, &Config::default());
            assert!(r.findings.iter().all(|f| f.code != "all-zero" && f.code != "differs-in-language"), "{check}: {:#?}", r.findings);
        }
    }

    #[test]
    fn years_are_ignored_when_comparing_legal_strings() {
        assert_eq!(without_years("Copyright 1999, 2020 Test"), without_years("Copyright 2021 Test"));
        assert_ne!(without_years("Copyright 2020 Test"), without_years("Copyright 2020 Other"));
        assert_eq!(without_years("Font  2020  ok"), "Fontok");
        assert_eq!(without_years("Room 3000 A"), "Room 3000 A", "3000 is not a year of the pattern");
    }

    #[test]
    fn unreadable_fonts_do_not_stop_family_checks() {
        const EMPTY_SFNT: [u8; 12] = [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let services = NoServices;
        let mut garbage = build(&clean()[0]);
        // Point every table past the end of the file.
        for i in 0..8 {
            let at = 12 + 16 * i + 8;
            garbage[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        }
        let a = BinaryCtx::new("f0", &EMPTY_SFNT, 0, &services).unwrap();
        let b = BinaryCtx::new("f1", &garbage, 0, &services).unwrap();
        let c = BinaryCtx::new("f2", &EMPTY_SFNT, 0, &services).unwrap();
        let fonts = [a, b, c];
        let ctx = FamilyCtx { fonts: &fonts, services: &services };
        let checks: Vec<&'static crate::CheckSpec> = family_checks().filter_map(crate::find).collect();
        let mut config = Config::default();
        config.set("check.family.panoseFamilyType", "strict", 1.0).unwrap();
        let _ = audit_family(&ctx, &checks, &config);
    }
}
