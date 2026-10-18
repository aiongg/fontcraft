//! Glyph set, name and encoding checks, and the contour-count check (M12.47). Runners take the context of their scope ([`crate::GlyphCtx`],
//! [`crate::SourceCtx`], [`crate::BinaryCtx`], [`crate::FamilyCtx`]), the thresholds and a
//! [`crate::Sink`]; their registry entries go in `registry/`.
//!
//! The binary checks follow the rules of FontBakery 1.1.0 (https://github.com/fonttools/fontbakery,
//! Apache License 2.0): `mandatory_glyphs`, `whitespace_glyphs`, `whitespace_ink`,
//! `control_chars`, `unique_glyphnames`, `valid_glyphnames`, `unreachable_glyphs`,
//! `case_mapping`, `empty_letters`, `soft_hyphen`, `rupee`, `cjk_not_enough_glyphs`,
//! `arabic_high_hamza`, `arabic_spacing_symbols`, `contour_count`,
//! `typenetwork/PUA_encoded_glyphs`, `notofonts/cmap/alien_codepoints`,
//! `typenetwork/glyph_coverage` and `googlefonts/render_own_name`; and of comparefamily's
//! single-face tests 19 and 26 in AFDKO 5.0.1 (Apache License 2.0, Copyright 2015 Adobe). The two
//! source checks are our own conventions. Deviations are noted at each runner.

use std::collections::{BTreeMap, HashMap};

use fontcraft_model::glyphdata::{GlyphData, Via, parse_code_point_name};
use read_fonts::ps::cff::dict::{Entry, entries};
use read_fonts::tables::cmap::{CmapIterLimits, CmapSubtable, PlatformId};
use read_fonts::tables::colr::Paint;
use read_fonts::tables::glyf::{Glyf, Glyph as TtGlyph};
use read_fonts::tables::gsub::{SingleSubst, SubstitutionSubtables};
use read_fonts::tables::layout::CoverageTable;
use read_fonts::tables::loca::{Loca, LocaGlyph};
use read_fonts::types::{GlyphId, Tag};
use read_fonts::{FontRef, ReadError, TableProvider};
use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlineGlyphCollection, OutlinePen};

use super::components::Names;
use super::kerning::{cmap_mappings, glyph_label};
use crate::data::{chars, code_pages, contours, glyphs};
use crate::name_records::{self, MAX_NAME_TEXT_BYTES};
use crate::services::ShapeRequest;
use crate::{BinaryCtx, Hit, Location, Params, Sink, SourceCtx};

/// Deepest nesting of composite glyphs followed; the OpenType limit for `maxComponentDepth` is
/// far below this in real fonts.
const MAX_COMPONENT_DEPTH: usize = 16;
/// Most glyph visits (outline reads, coverage entries, substitution outputs) one check spends on
/// one font. A table whose offsets all point at the same large array could otherwise ask for
/// billions.
const MAX_WORK: usize = 4_000_000;
/// Most (code point, glyph) pairs read from one `cmap` subtable.
const MAX_CMAP_MAPPINGS: usize = 8_388_608;
/// Most top DICT entries read when looking for a CID font's ROS.
const MAX_DICT_ENTRIES: usize = 256;
/// Most code points or names one message lists before it gives the count of the rest.
const MAX_IN_MESSAGE: usize = 20;
/// Segments each curve is split into when an outline's area is measured.
const CURVE_STEPS: u32 = 32;
/// The safe limit of the total glyph name size on old Mac OS X versions.
const NAME_BUFFER_LIMIT: usize = 32_767;
/// The relative area difference at which the high hamza is drawn at the wrong size.
const HIGH_HAMZA_AREA_TOLERANCE: f64 = 0.1;
/// U+0621 ARABIC LETTER HAMZA, the size the high hamza is compared with.
const ARABIC_HAMZA: u32 = 0x0621;
/// U+00AD SOFT HYPHEN.
const SOFT_HYPHEN: u32 = 0x00AD;
/// U+F8FF, a private-use code point one system vendor's fonts use for its logo.
const VENDOR_LOGO: u32 = 0xF8FF;

// ---------------------------------------------------------------------------------------------
// Shared readers
// ---------------------------------------------------------------------------------------------

fn cp(u: u32) -> String {
    format!("U+{u:04X}")
}

fn cp_list(cps: &[u32]) -> String {
    let mut out: Vec<String> = cps.iter().take(MAX_IN_MESSAGE).map(|c| cp(*c)).collect();
    if cps.len() > MAX_IN_MESSAGE {
        out.push(format!("and {} more", cps.len() - MAX_IN_MESSAGE));
    }
    out.join(", ")
}

fn glyph_place(glyph: String, table: &str) -> Location {
    Location { glyph, table: Some(table.to_string()), ..Location::default() }
}

fn table_place(table: &str) -> Location {
    Location { table: Some(table.to_string()), ..Location::default() }
}

/// Code point → glyph of the subtable a shaper uses, without unmapped entries.
fn best_cmap(font: &FontRef<'_>) -> BTreeMap<u32, u32> {
    cmap_mappings(font, true).into_iter().collect()
}

/// The number of glyphs `maxp` declares.
fn glyph_count(font: &FontRef<'_>) -> u32 {
    font.maxp().map_or(0, |m| u32::from(m.num_glyphs()))
}

/// (code point, glyph) pairs of one subtable, without unmapped entries, at most
/// [`MAX_CMAP_MAPPINGS`].
fn subtable_mappings(font: &FontRef<'_>, subtable: &CmapSubtable<'_>) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    if let CmapSubtable::Format0(t) = subtable {
        for c in 0u32..256 {
            if let Some(g) = t.map_codepoint(c).filter(|g| g.to_u32() != 0) {
                out.push((c, g.to_u32()));
            }
        }
        return out;
    }
    for (c, g) in subtable.iter_with_limits(CmapIterLimits::default_for_font(font)) {
        if out.len() >= MAX_CMAP_MAPPINGS {
            break;
        }
        if g.to_u32() != 0 {
            out.push((c, g.to_u32()));
        }
    }
    out
}

/// The mappings of the Unicode subtable comparefamily reads: platform 0 encoding 3, else
/// platform 3 encoding 1. `None` when the font has no `cmap`; empty when it has no such subtable.
fn unicode_subtable(font: &FontRef<'_>) -> Option<Vec<(u32, u32)>> {
    let cmap = font.cmap().ok()?;
    let records = cmap.encoding_records();
    let record = records
        .iter()
        .find(|r| r.platform_id() == PlatformId::Unicode && r.encoding_id() == 3)
        .or_else(|| records.iter().find(|r| r.platform_id() == PlatformId::Windows && r.encoding_id() == 1));
    let Some(subtable) = record.and_then(|r| r.subtable(cmap.offset_data()).ok()) else { return Some(Vec::new()) };
    Some(subtable_mappings(font, &subtable))
}

/// Whether the font's CFF is CID-keyed (its top DICT has an ROS or an FDArray).
fn is_cid(font: &FontRef<'_>) -> bool {
    let Ok(cff) = font.cff() else { return false };
    let Some(data) = cff.top_dicts().get(0) else { return false };
    entries(data, None).take(MAX_DICT_ENTRIES).map_while(Result::ok).any(|e| matches!(e, Entry::Ros { .. } | Entry::FdArrayOffset(_)))
}

/// A spending limit for work that a hostile table can multiply.
struct Budget(usize);

impl Budget {
    fn new() -> Self {
        Self(MAX_WORK)
    }

    /// Takes one unit; false once the budget is spent.
    fn take(&mut self) -> bool {
        match self.0.checked_sub(1) {
            Some(rest) => {
                self.0 = rest;
                true
            }
            None => false,
        }
    }
}

/// Counts what a glyph's drawing contains.
#[derive(Default)]
struct CountPen {
    moves: usize,
    segments: usize,
}

impl OutlinePen for CountPen {
    fn move_to(&mut self, _: f32, _: f32) {
        self.moves += 1;
    }
    fn line_to(&mut self, _: f32, _: f32) {
        self.segments += 1;
    }
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
        self.segments += 1;
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.segments += 1;
    }
    fn close(&mut self) {}
}

/// Whether glyphs have ink, read from `glyf` directly (so composites are judged by their
/// components) or, for CFF and CFF2 outlines, by drawing them.
enum Ink<'a> {
    Glyf(Glyf<'a>, Loca<'a>),
    Drawn(OutlineGlyphCollection<'a>),
}

impl<'a> Ink<'a> {
    fn new(font: &FontRef<'a>) -> Self {
        match (font.glyf(), font.loca(None)) {
            (Ok(glyf), Ok(loca)) => Ink::Glyf(glyf, loca),
            _ => Ink::Drawn(font.outline_glyphs()),
        }
    }

    /// The glyph draws something: a simple TrueType glyph with at least three points, a
    /// composite with a component that has ink, or a CFF glyph that draws any point.
    fn has_ink(&self, gid: u32, budget: &mut Budget) -> bool {
        match self {
            Ink::Glyf(glyf, loca) => glyf_ink(glyf, loca, gid, 0, budget),
            Ink::Drawn(outlines) => drawn(outlines, gid).is_some_and(|p| p.moves > 0 || p.segments > 0),
        }
    }

    /// The glyph is certainly empty: a simple TrueType glyph without contours, or a CFF glyph
    /// that draws no segment. Composites are never empty here.
    fn is_empty(&self, gid: u32) -> bool {
        match self {
            Ink::Glyf(glyf, loca) => match loca.get(GlyphId::new(gid), glyf) {
                Some(LocaGlyph::Glyph(TtGlyph::Simple(s))) => s.number_of_contours() == 0,
                Some(LocaGlyph::Glyph(TtGlyph::Composite(_))) => false,
                Some(LocaGlyph::Empty) => true,
                None => false,
            },
            Ink::Drawn(outlines) => drawn(outlines, gid).is_some_and(|p| p.segments == 0),
        }
    }
}

fn glyf_ink(glyf: &Glyf<'_>, loca: &Loca<'_>, gid: u32, depth: usize, budget: &mut Budget) -> bool {
    if depth > MAX_COMPONENT_DEPTH || !budget.take() {
        return false;
    }
    match loca.get(GlyphId::new(gid), glyf) {
        Some(LocaGlyph::Glyph(TtGlyph::Simple(s))) => s.num_points() > 2,
        Some(LocaGlyph::Glyph(TtGlyph::Composite(c))) => {
            for (g, _) in c.component_glyphs_and_flags() {
                if glyf_ink(glyf, loca, g.to_u32(), depth + 1, budget) {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

fn drawn(outlines: &OutlineGlyphCollection<'_>, gid: u32) -> Option<CountPen> {
    let glyph = outlines.get(GlyphId::new(gid))?;
    let mut pen = CountPen::default();
    glyph.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).ok()?;
    Some(pen)
}

/// The number of contours of a TrueType glyph with its composites decomposed. Components named
/// `.ttfautohint` are not counted.
fn glyf_contours(glyf: &Glyf<'_>, loca: &Loca<'_>, names: &Names, gid: u32, depth: usize, budget: &mut Budget) -> usize {
    if depth > MAX_COMPONENT_DEPTH || !budget.take() {
        return 0;
    }
    match loca.get(GlyphId::new(gid), glyf) {
        Some(LocaGlyph::Glyph(TtGlyph::Simple(s))) => usize::try_from(s.number_of_contours()).unwrap_or(0),
        Some(LocaGlyph::Glyph(TtGlyph::Composite(c))) => c
            .component_glyphs_and_flags()
            .filter(|(g, _)| names.stored(g.to_u32()) != Some(".ttfautohint"))
            .map(|(g, _)| glyf_contours(glyf, loca, names, g.to_u32(), depth + 1, budget))
            .fold(0usize, usize::saturating_add),
        _ => 0,
    }
}

/// The signed area of a drawn glyph, curves split into [`CURVE_STEPS`] lines.
#[derive(Default)]
struct AreaPen {
    area: f64,
    start: (f64, f64),
    at: (f64, f64),
}

impl AreaPen {
    fn segment(&mut self, to: (f64, f64)) {
        self.area += (self.at.0 * to.1 - to.0 * self.at.1) / 2.0;
        self.at = to;
    }

    fn steps(&mut self, point: impl Fn(f64) -> (f64, f64)) {
        for i in 1..=CURVE_STEPS {
            self.segment(point(f64::from(i) / f64::from(CURVE_STEPS)));
        }
    }
}

impl OutlinePen for AreaPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close();
        self.start = (f64::from(x), f64::from(y));
        self.at = self.start;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.segment((f64::from(x), f64::from(y)));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (p0, p1, p2) = (self.at, (f64::from(cx0), f64::from(cy0)), (f64::from(x), f64::from(y)));
        self.steps(|t| {
            let u = 1.0 - t;
            (u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0, u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1)
        });
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (p0, p1, p2, p3) = (self.at, (f64::from(cx0), f64::from(cy0)), (f64::from(cx1), f64::from(cy1)), (f64::from(x), f64::from(y)));
        self.steps(|t| {
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            (a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0, a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1)
        });
    }

    fn close(&mut self) {
        if self.at != self.start {
            self.segment(self.start);
        }
    }
}

fn area(outlines: &OutlineGlyphCollection<'_>, gid: u32) -> Option<f64> {
    let glyph = outlines.get(GlyphId::new(gid))?;
    let mut pen = AreaPen::default();
    glyph.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).ok()?;
    pen.close();
    pen.area.is_finite().then_some(pen.area)
}

/// The GDEF glyph class of `gid` (3 is a mark), or `None` without a class definition.
fn gdef_class(font: &FontRef<'_>, gid: u32) -> Option<u16> {
    let class_def = font.gdef().ok()?.glyph_class_def()?.ok()?;
    Some(class_def.get(GlyphId::new(gid)))
}

// ---------------------------------------------------------------------------------------------
// Mandatory glyphs and whitespace
// ---------------------------------------------------------------------------------------------

/// `check.glyphset.notdef`: .notdef is missing, not first, encoded (warnings) or blank (a
/// failure). A font that stores no glyph names has .notdef as glyph 0, as the reference reads
/// such fonts. A mapping to glyph 0 means "no glyph" in `cmap`, so only a .notdef elsewhere can
/// be reported as encoded.
pub(crate) fn notdef(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    if glyph_count(font) == 0 {
        sink.push(Hit::new("notdef-not-found", "The font has no glyphs, so no .notdef glyph either.").place(table_place("maxp")));
        return;
    }
    let names = Names::new(font);
    let found = (0..glyph_count(font)).find(|g| names.stored(*g) == Some(".notdef"));
    let gid = match found {
        Some(g) => g,
        None if names.stored(0).is_none() => 0,
        None => {
            sink.push(Hit::new("notdef-not-found", "The font has no .notdef glyph.").place(table_place("post")));
            return;
        }
    };
    let place = || glyph_place(".notdef".to_string(), "post");
    if gid != 0 {
        sink.push(
            Hit::new("notdef-not-first", format!("The .notdef glyph is glyph {gid}; it should be the first glyph."))
                .data("glyphId", f64::from(gid))
                .place(place()),
        );
    }
    if let Some(code) = best_cmap(font).iter().find(|(_, g)| **g == gid).map(|(c, _)| *c) {
        sink.push(
            Hit::new("notdef-has-codepoint", format!("The .notdef glyph is mapped from {}; it should have no code point.", cp(code)))
                .data("codepoint", f64::from(code))
                .place(glyph_place(".notdef".to_string(), "cmap")),
        );
    }
    if !Ink::new(font).has_ink(gid, &mut Budget::new()) {
        sink.push(
            Hit::new("notdef-is-blank", "The .notdef glyph is blank; it should show a drawing so missing characters are visible.").place(place()),
        );
    }
}

/// `check.glyphset.whitespaceGlyphs`: no glyph for U+0020 or U+00A0.
pub(crate) fn whitespace_glyphs(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let cmap = best_cmap(&ctx.font);
    for (code, name) in [(0x0020u32, "SPACE"), (0x00A0, "NO-BREAK SPACE")] {
        if !cmap.contains_key(&code) {
            sink.push(
                Hit::new("missing-whitespace-glyph", format!("There is no glyph for {} {name}.", cp(code)))
                    .data("codepoint", f64::from(code))
                    .place(table_place("cmap")),
            );
        }
    }
}

/// `check.glyphset.whitespaceInk`: a glyph mapped from a whitespace or invisible character has
/// an outline (U+1680 OGHAM SPACE MARK is meant to draw).
pub(crate) fn whitespace_ink(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let ink = Ink::new(font);
    let names = Names::new(font);
    let mut budget = Budget::new();
    for (code, gid) in best_cmap(font) {
        if chars::must_be_empty(code) && ink.has_ink(gid, &mut budget) {
            let glyph = glyph_label(ctx, &names, gid);
            sink.push(
                Hit::new("has-ink", format!("`{glyph}`, mapped from {}, has an outline; it must be an empty glyph.", cp(code)))
                    .data("codepoint", f64::from(code))
                    .place(glyph_place(glyph, "cmap")),
            );
        }
    }
}

/// `check.glyphset.controlCharacters`: a TrueType font maps control characters U+0001 to U+001F
/// other than U+000D, or has a glyph named after one (`uni0001`), as the reference finds them.
pub(crate) fn control_characters(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let names = Names::new(font);
    let unacceptable = |c: u32| (0x01..=0x1F).contains(&c) && c != 0x0D;
    let mut reported: Vec<u32> = Vec::new();
    for (code, gid) in best_cmap(font) {
        if unacceptable(code) {
            let glyph = glyph_label(ctx, &names, gid);
            sink.push(
                Hit::new(
                    "unacceptable",
                    format!("`{glyph}` is mapped from the control character {}; only U+0000 and U+000D may be mapped.", cp(code)),
                )
                .data("codepoint", f64::from(code))
                .place(glyph_place(glyph, "cmap")),
            );
            reported.push(gid);
        }
    }
    for gid in 0..glyph_count(font) {
        let Some(name) = names.stored(gid) else { continue };
        let named = name.strip_prefix("uni").filter(|h| h.len() == 4).and_then(|h| u32::from_str_radix(h, 16).ok());
        if named.is_some_and(unacceptable) && !reported.contains(&gid) {
            sink.push(
                Hit::new("unacceptable", format!("The glyph `{name}` stands for a control character other than U+0000 and U+000D."))
                    .without_fix()
                    .place(glyph_place(name.to_string(), "post")),
            );
        }
    }
}

/// `check.glyphset.truetypeLeadingGlyphs`: comparefamily's glyph-order test. A TrueType font's
/// first four glyphs are .notdef, NULL, CR and space, and the last three are mapped from U+0000,
/// U+000D and U+0020 (the lowest code point of each glyph in the Unicode subtable counts). Also
/// reports an OS/2 default character other than 0, for every font. A font that stores no glyph
/// names is checked only for the mappings of glyphs 1 to 3.
pub(crate) fn truetype_leading_glyphs(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    if let Some(c) = font.os2().ok().and_then(|t| t.us_default_char()).filter(|c| *c != 0) {
        sink.push(
            Hit::new("default-char-not-zero", format!("The OS/2 default character is {c}; 0 (.notdef) is the useful value."))
                .without_fix()
                .data("usDefaultChar", f64::from(c))
                .place(Location { field: Some("usDefaultChar".into()), ..table_place("OS/2") }),
        );
    }
    if font.table_data(Tag::new(b"glyf")).is_none() {
        return;
    }
    let names = Names::new(font);
    let count = glyph_count(font);
    let stored = !names.synthesized;
    let name_of = |gid: u32| if gid < count { names.stored(gid) } else { None };
    let place = |gid: u32| glyph_place(glyph_label(ctx, &names, gid), "post");
    if stored && name_of(0) != Some(".notdef") {
        sink.push(
            Hit::new("wrong-glyph-order", format!("Glyph 0 is `{}`; in a TrueType font it should be .notdef.", name_of(0).unwrap_or("missing")))
                .place(place(0)),
        );
    }
    // (glyph id, expected name, tolerated legacy name, code point)
    let slots = [(1u32, "NULL", Some(".null"), 0x0000u32), (2, "CR", Some("nonmarkingreturn"), 0x000D), (3, "space", None, 0x0020)];
    let mut encoded: Vec<(u32, u32)> = Vec::new();
    for (gid, expected, legacy, code) in slots {
        if stored {
            let name = name_of(gid);
            if name == legacy && legacy.is_some() {
                sink.push(
                    Hit::new("legacy-name", format!("Glyph {gid} is named `{}`; the conventional name is `{expected}`.", name.unwrap_or("")))
                        .place(place(gid)),
                );
            } else if name != Some(expected) {
                sink.push(
                    Hit::new(
                        "wrong-glyph-order",
                        format!("Glyph {gid} is `{}`; in a TrueType font it should be `{expected}`.", name.unwrap_or("missing")),
                    )
                    .place(place(gid)),
                );
                continue;
            }
        } else if gid >= count {
            continue;
        }
        encoded.push((gid, code));
    }
    let Some(unicode) = unicode_subtable(font).filter(|m| !m.is_empty()) else { return };
    let mut lowest: HashMap<u32, u32> = HashMap::new();
    for (c, g) in unicode {
        let slot = lowest.entry(g).or_insert(c);
        *slot = (*slot).min(c);
    }
    for (gid, code) in encoded {
        let glyph = glyph_label(ctx, &names, gid);
        match lowest.get(&gid) {
            Some(c) if *c == code => {}
            Some(c) => sink.push(
                Hit::new("wrong-encoding", format!("`{glyph}` is mapped from {} rather than {}.", cp(*c), cp(code)))
                    .data("codepoint", f64::from(*c))
                    .place(glyph_place(glyph, "cmap")),
            ),
            None => sink.push(Hit::new("not-encoded", format!("`{glyph}` should be mapped from {}.", cp(code))).place(glyph_place(glyph, "cmap"))),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Glyph names
// ---------------------------------------------------------------------------------------------

/// The glyph names the font stores, by glyph id; `None` when it stores none (a TrueType or CFF2
/// font with a version 3 `post` table).
fn stored_names(font: &FontRef<'_>, names: &Names) -> Option<Vec<(u32, String)>> {
    if names.synthesized {
        return None;
    }
    Some((0..glyph_count(font)).filter_map(|g| names.stored(g).map(|n| (g, n.to_string()))).collect())
}

const NO_NAMES: &str = "the font stores no glyph names";

/// `check.glyphset.uniqueGlyphNames`: two glyphs share one name.
pub(crate) fn unique_glyph_names(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let names = Names::new(&ctx.font);
    let Some(stored) = stored_names(&ctx.font, &names) else {
        sink.skip(NO_NAMES);
        return;
    };
    let mut seen: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
    for (gid, name) in &stored {
        seen.entry(name.as_str()).or_default().push(*gid);
    }
    for (name, gids) in seen.into_iter().filter(|(_, g)| g.len() > 1) {
        let ids: Vec<String> = gids.iter().take(MAX_IN_MESSAGE).map(u32::to_string).collect();
        sink.push(
            Hit::new("duplicated-glyph-names", format!("{} glyphs are named `{name}` (glyphs {}).", gids.len(), ids.join(", ")))
                .data("count", gids.len() as f64)
                .place(glyph_place(name.to_string(), "post")),
        );
    }
}

/// A glyph name the naming rules accept: 1 to 63 characters from A-Z, a-z, 0-9, period and
/// underscore, not starting with a digit or a period.
fn valid_glyph_name(name: &str) -> bool {
    let ok_chars = name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_');
    let first_ok = name.bytes().next().is_some_and(|b| b != b'.' && !b.is_ascii_digit());
    ok_chars && first_ok && name.len() <= 63
}

/// `check.glyphset.validGlyphNames`: names with characters outside A-Z, a-z, 0-9, period and
/// underscore, starting with a digit or period, or longer than 63 characters (failures); when
/// every name is valid, names of 32 to 63 characters (warnings).
pub(crate) fn valid_glyph_names(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let names = Names::new(&ctx.font);
    let Some(stored) = stored_names(&ctx.font, &names) else {
        sink.skip(NO_NAMES);
        return;
    };
    let checked: Vec<&String> =
        stored.iter().map(|(_, n)| n).filter(|n| !n.starts_with(".null") && !n.starts_with(".notdef") && !n.starts_with(".ttfautohint")).collect();
    let bad: Vec<&String> = checked.iter().copied().filter(|n| !valid_glyph_name(n)).collect();
    if !bad.is_empty() {
        for name in bad {
            sink.push(
                Hit::new("found-invalid-names", format!("The glyph name `{name}` uses characters other than A-Z, a-z, 0-9, period and underscore, starts with a digit or period, or is longer than 63 characters."))
                    .place(glyph_place(name.to_string(), "post")),
            );
        }
        return;
    }
    for name in checked.into_iter().filter(|n| n.len() > 31) {
        sink.push(
            Hit::new(
                "legacy-long-names",
                format!("The glyph name `{name}` is {} characters long; some older systems expect at most 31.", name.len()),
            )
            .data("length", name.len() as f64)
            .place(glyph_place(name.to_string(), "post")),
        );
    }
}

/// The glyph name buffer comparefamily measures: every name followed by two bytes.
fn name_buffer_size(names: &[(u32, String)]) -> usize {
    names.iter().map(|(_, n)| n.chars().count().saturating_add(2)).fold(0usize, usize::saturating_add)
}

/// `check.glyphset.glyphNameBufferSize`: the names of a CFF font together exceed 32,767 bytes
/// (a failure), or come within two bytes a glyph of it (a warning). CID-keyed fonts are skipped.
pub(crate) fn glyph_name_buffer_size(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    if is_cid(&ctx.font) {
        sink.skip("the font is CID-keyed");
        return;
    }
    let names = Names::new(&ctx.font);
    let Some(stored) = stored_names(&ctx.font, &names) else {
        sink.skip(NO_NAMES);
        return;
    };
    let size = name_buffer_size(&stored);
    let margin = stored.len().saturating_mul(2);
    if size > NAME_BUFFER_LIMIT {
        sink.push(
            Hit::new(
                "buffer-too-large",
                format!("The glyph names take {size} bytes, {} more than the 32,767 that old Mac OS X versions can hold.", size - NAME_BUFFER_LIMIT),
            )
            .data("size", size as f64)
            .place(table_place("CFF ")),
        );
    } else if size.saturating_add(margin) > NAME_BUFFER_LIMIT {
        sink.push(
            Hit::new(
                "buffer-near-limit",
                format!("The glyph names take {size} bytes, {} short of the 32,767 that old Mac OS X versions can hold.", NAME_BUFFER_LIMIT - size),
            )
            .data("size", size as f64)
            .place(table_place("CFF ")),
        );
    }
}

/// A `uniXXXX…` name (groups of four upper-case hex digits) or a `uXXXX` to `uXXXXXX` name.
fn is_uni_name(name: &str) -> bool {
    let hex = |s: &str| s.bytes().all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b));
    if let Some(rest) = name.strip_prefix("uni") {
        return !rest.is_empty() && rest.len() % 4 == 0 && hex(rest);
    }
    name.strip_prefix('u').is_some_and(|rest| (4..=6).contains(&rest.len()) && hex(rest))
}

/// What comparefamily says about one part of a glyph name: `Some((code, final name))` when it
/// is a working name, `Some(("unmappable-name", ""))` when nothing maps it to Unicode.
fn name_part_problem(data: &GlyphData, part: &str, ligature: bool, all: &[(u32, String)]) -> Option<(&'static str, String)> {
    if let Some(record) = data.record(part) {
        let fin = record.production?;
        if fin == part || (!ligature && all.iter().any(|(_, n)| *n == fin)) {
            return None;
        }
        return Some(("working-name", fin));
    }
    if data.canonical_name(part).is_some() || is_uni_name(part) {
        return None;
    }
    Some(("unmappable-name", String::new()))
}

/// `check.glyphset.productionNames`: a glyph name, or a part of a ligature name, is a working
/// name where the glyph database gives another final name (not reported for a single glyph when
/// a glyph with the final name exists too), or is neither known to the database nor a `uni` or
/// `u` name. Names starting with a period and CID-keyed fonts are skipped.
pub(crate) fn production_names(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(data) = glyphs::glyph_data() else {
        sink.skip("the glyph database cannot be read");
        return;
    };
    if is_cid(&ctx.font) {
        sink.skip("the font is CID-keyed");
        return;
    }
    let names = Names::new(&ctx.font);
    let Some(stored) = stored_names(&ctx.font, &names) else {
        sink.skip(NO_NAMES);
        return;
    };
    for (_, name) in &stored {
        if name.starts_with('.') {
            continue;
        }
        let base = name.split('.').next().unwrap_or(name);
        let parts: Vec<&str> = base.split('_').collect();
        let ligature = parts.len() > 1;
        for part in parts.into_iter().filter(|p| !p.is_empty()) {
            let Some((code, fin)) = name_part_problem(data, part, ligature, &stored) else { continue };
            let message = match (code, ligature) {
                ("working-name", true) => format!("The ligature `{name}` uses the working name `{part}` where the final name is `{fin}`."),
                ("working-name", false) => format!("`{name}` is a working name; the final name is `{fin}`."),
                (_, true) => {
                    format!("`{part}` in the ligature `{name}` is neither a known glyph name nor a `uni` name, so it cannot be mapped to Unicode.")
                }
                (_, false) => format!("`{name}` is neither a known glyph name nor a `uni` name, so it cannot be mapped to Unicode."),
            };
            let mut hit = Hit::new(code, message);
            if code == "unmappable-name" {
                hit = hit.without_fix();
            }
            sink.push(hit.place(glyph_place(name.clone(), "post")));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Reachability
// ---------------------------------------------------------------------------------------------

/// Glyph ids marked as reached.
struct Reached(Vec<bool>);

impl Reached {
    /// Marks `gid`; true when it was not marked before.
    fn mark(&mut self, gid: u32) -> bool {
        match usize::try_from(gid).ok().and_then(|i| self.0.get_mut(i)) {
            Some(slot) if !*slot => {
                *slot = true;
                true
            }
            _ => false,
        }
    }
}

/// Marks every glyph of `coverage`; false when the budget ran out.
fn mark_coverage(coverage: Result<CoverageTable<'_>, ReadError>, reached: &mut Reached, budget: &mut Budget) -> bool {
    let Ok(coverage) = coverage else { return true };
    for g in coverage.iter() {
        if !budget.take() {
            return false;
        }
        reached.mark(u32::from(g.to_u16()));
    }
    true
}

/// Marks the glyphs any GSUB lookup can output, whatever its input; false when the budget ran
/// out. Contextual lookups output nothing themselves.
fn mark_gsub_outputs(font: &FontRef<'_>, reached: &mut Reached, budget: &mut Budget) -> bool {
    let Ok(list) = font.gsub().and_then(|g| g.lookup_list()) else { return true };
    let mut out = |g: u32, budget: &mut Budget| {
        reached.mark(g);
        budget.take()
    };
    for lookup in list.lookups().iter().flatten() {
        let Ok(subtables) = lookup.subtables() else { continue };
        match subtables {
            SubstitutionSubtables::Single(s) => {
                for sub in s.iter().flatten() {
                    match sub {
                        SingleSubst::Format1(f) => {
                            let delta = i32::from(f.delta_glyph_id());
                            let Ok(coverage) = f.coverage() else { continue };
                            for g in coverage.iter() {
                                let target = (i32::from(g.to_u16()) + delta).rem_euclid(65_536);
                                if !out(u32::try_from(target).unwrap_or(0), budget) {
                                    return false;
                                }
                            }
                        }
                        SingleSubst::Format2(f) => {
                            for g in f.substitute_glyph_ids() {
                                if !out(u32::from(g.get().to_u16()), budget) {
                                    return false;
                                }
                            }
                        }
                    }
                }
            }
            SubstitutionSubtables::Multiple(s) => {
                for sub in s.iter().flatten() {
                    for seq in sub.sequences().iter().flatten() {
                        for g in seq.substitute_glyph_ids() {
                            if !out(u32::from(g.get().to_u16()), budget) {
                                return false;
                            }
                        }
                    }
                }
            }
            SubstitutionSubtables::Alternate(s) => {
                for sub in s.iter().flatten() {
                    for set in sub.alternate_sets().iter().flatten() {
                        for g in set.alternate_glyph_ids() {
                            if !out(u32::from(g.get().to_u16()), budget) {
                                return false;
                            }
                        }
                    }
                }
            }
            SubstitutionSubtables::Ligature(s) => {
                for sub in s.iter().flatten() {
                    for set in sub.ligature_sets().iter().flatten() {
                        for lig in set.ligatures().iter().flatten() {
                            if !out(u32::from(lig.ligature_glyph().to_u16()), budget) {
                                return false;
                            }
                        }
                    }
                }
            }
            SubstitutionSubtables::Reverse(s) => {
                for sub in s.iter().flatten() {
                    for g in sub.substitute_glyph_ids() {
                        if !out(u32::from(g.get().to_u16()), budget) {
                            return false;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    true
}

/// Marks the glyphs of non-default variation sequences in `cmap` format 14 subtables.
fn mark_variation_sequences(font: &FontRef<'_>, reached: &mut Reached, budget: &mut Budget) -> bool {
    let Ok(cmap) = font.cmap() else { return true };
    for record in cmap.encoding_records() {
        let Ok(CmapSubtable::Format14(t)) = record.subtable(cmap.offset_data()) else { continue };
        for selector in t.var_selector() {
            let Some(Ok(uvs)) = selector.non_default_uvs(t.offset_data()) else { continue };
            for m in uvs.uvs_mapping() {
                if !budget.take() {
                    return false;
                }
                reached.mark(u32::from(m.glyph_id()));
            }
        }
    }
    true
}

fn be16(bytes: &[u8], at: usize) -> Option<usize> {
    let b = bytes.get(at..at.checked_add(2)?)?;
    Some(usize::from(u16::from_be_bytes([*b.first()?, *b.get(1)?])))
}

/// Marks the extender glyphs (kashidas) of each script in `JSTF`, read directly: the table has
/// no reader in the font library.
fn mark_jstf_extenders(font: &FontRef<'_>, reached: &mut Reached, budget: &mut Budget) -> bool {
    let Some(data) = font.table_data(Tag::new(b"JSTF")) else { return true };
    let bytes = data.as_bytes();
    let Some(scripts) = be16(bytes, 4) else { return true };
    for i in 0..scripts {
        let Some(script) = be16(bytes, 6 + 6 * i + 4) else { break };
        let Some(offset) = be16(bytes, script).filter(|o| *o != 0) else { continue };
        let base = script + offset;
        let Some(n) = be16(bytes, base) else { continue };
        for j in 0..n {
            let Some(g) = be16(bytes, base + 2 + 2 * j) else { break };
            if !budget.take() {
                return false;
            }
            reached.mark(u32::try_from(g).unwrap_or(0));
        }
    }
    true
}

/// Marks the glyphs `MATH` refers to: accent and extended-shape coverage, the variant coverage,
/// and every variant and assembly part.
fn mark_math(font: &FontRef<'_>, reached: &mut Reached, budget: &mut Budget) -> bool {
    let Ok(math) = font.math() else { return true };
    if let Ok(info) = math.math_glyph_info() {
        if let Some(Ok(accents)) = info.math_top_accent_attachment()
            && !mark_coverage(accents.top_accent_coverage(), reached, budget)
        {
            return false;
        }
        if let Some(coverage) = info.extended_shape_coverage()
            && !mark_coverage(coverage, reached, budget)
        {
            return false;
        }
    }
    let Ok(variants) = math.math_variants() else { return true };
    for coverage in [variants.vert_glyph_coverage(), variants.horiz_glyph_coverage()].into_iter().flatten() {
        if !mark_coverage(coverage, reached, budget) {
            return false;
        }
    }
    for construction in variants.vert_glyph_constructions().iter().chain(variants.horiz_glyph_constructions().iter()).flatten() {
        if let Some(Ok(assembly)) = construction.glyph_assembly() {
            for part in assembly.part_records() {
                if !budget.take() {
                    return false;
                }
                reached.mark(u32::from(part.glyph_id().to_u16()));
            }
        }
        for variant in construction.math_glyph_variant_records() {
            if !budget.take() {
                return false;
            }
            reached.mark(u32::from(variant.variant_glyph().to_u16()));
        }
    }
    true
}

/// Marks the glyphs `COLR` refers to as the reference reads it: version 0 layers; in version 1
/// also the base glyphs of the version 0 records and the glyphs of top-level glyph paints.
fn mark_colr(font: &FontRef<'_>, reached: &mut Reached, budget: &mut Budget) -> bool {
    let Ok(colr) = font.colr() else { return true };
    let mut glyphs: Vec<u32> = Vec::new();
    if let Some(Ok(layers)) = colr.layer_records() {
        glyphs.extend(layers.iter().map(|l| u32::from(l.glyph_id().to_u16())));
    }
    if colr.version() >= 1 {
        if let Some(Ok(bases)) = colr.base_glyph_records() {
            glyphs.extend(bases.iter().map(|b| u32::from(b.glyph_id().to_u16())));
        }
        if let Some(Ok(list)) = colr.base_glyph_list() {
            let data = list.offset_data();
            for record in list.base_glyph_paint_records() {
                if let Ok(Paint::Glyph(p)) = record.paint(data) {
                    glyphs.push(u32::from(p.glyph_id().to_u16()));
                }
            }
        }
        if let Some(Ok(list)) = colr.layer_list() {
            for paint in list.paints().iter() {
                if let Ok(Paint::Glyph(p)) = paint {
                    glyphs.push(u32::from(p.glyph_id().to_u16()));
                }
            }
        }
    }
    for g in glyphs {
        if !budget.take() {
            return false;
        }
        reached.mark(g);
    }
    true
}

/// `check.glyphset.unreachableGlyphs`: a glyph no code point, variation sequence, substitution
/// output, `JSTF` extender, `MATH` entry, `COLR` entry or reachable composite refers to.
/// .notdef and .null are exempt. Components of components count too, at any depth; the
/// reference looks one level down in glyph order only.
pub(crate) fn unreachable_glyphs(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let count = glyph_count(font);
    let names = Names::new(font);
    let mut reached = Reached(vec![false; usize::try_from(count).unwrap_or(0)]);
    reached.mark(0);
    for gid in 0..count {
        if matches!(names.stored(gid), Some(".notdef" | ".null")) {
            reached.mark(gid);
        }
    }
    for gid in best_cmap(font).into_values() {
        reached.mark(gid);
    }
    let mut budget = Budget::new();
    let complete = mark_variation_sequences(font, &mut reached, &mut budget)
        && mark_jstf_extenders(font, &mut reached, &mut budget)
        && mark_math(font, &mut reached, &mut budget)
        && mark_colr(font, &mut reached, &mut budget)
        && mark_gsub_outputs(font, &mut reached, &mut budget);
    if !complete {
        sink.skip("the font's tables refer to more glyphs than the check reads");
        return;
    }
    if let (Ok(glyf), Ok(loca)) = (font.glyf(), font.loca(None)) {
        let mut queue: Vec<u32> = (0..count).filter(|g| reached.0.get(*g as usize).copied().unwrap_or(false)).collect();
        while let Some(gid) = queue.pop() {
            let Some(LocaGlyph::Glyph(TtGlyph::Composite(c))) = loca.get(GlyphId::new(gid), &glyf) else { continue };
            for (g, _) in c.component_glyphs_and_flags() {
                if !budget.take() {
                    sink.skip("the font's composites refer to more glyphs than the check reads");
                    return;
                }
                if reached.mark(u32::from(g.to_u16())) {
                    queue.push(u32::from(g.to_u16()));
                }
            }
        }
    }
    for (gid, _) in reached.0.iter().enumerate().filter(|(_, r)| !**r) {
        let gid = u32::try_from(gid).unwrap_or(u32::MAX);
        let glyph = glyph_label(ctx, &names, gid);
        sink.push(
            Hit::new("unreachable-glyphs", format!("`{glyph}` cannot be reached: no code point, substitution or composite refers to it."))
                .data("glyphId", f64::from(gid))
                .place(Location { glyph, ..Location::default() }),
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Characters and coverage
// ---------------------------------------------------------------------------------------------

/// `check.glyphset.caseMapping`: a letter whose single-character case partner is not mapped,
/// skipping the reference's list of partners that are often missing on purpose.
pub(crate) fn case_mapping(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    if glyphs::glyph_data().is_none() {
        sink.skip("the glyph database cannot be read");
        return;
    }
    let names = Names::new(&ctx.font);
    let cmap = best_cmap(&ctx.font);
    for (&code, &gid) in &cmap {
        if chars::CASE_MAPPING_EXCEPTIONS.contains(&code) {
            continue;
        }
        let Some(partner) = glyphs::case_partner(code) else { continue };
        if cmap.contains_key(&partner) {
            continue;
        }
        let glyph = glyph_label(ctx, &names, gid);
        sink.push(
            Hit::new("missing-case-counterparts", format!("{} (`{glyph}`) is present but its case partner {} is not.", cp(code), cp(partner)))
                .data("codepoint", f64::from(code))
                .data("partner", f64::from(partner))
                .place(glyph_place(glyph, "cmap")),
        );
    }
}

/// `check.glyphset.emptyLetters`: an encoded letter whose glyph is certainly empty (a TrueType
/// glyph without contours, a CFF glyph that draws no segment; the reference looks at the
/// charstring's length instead). The Hangul fillers are exempt; empty Hangul syllables outside
/// KS X 1001 give one warning with their count.
pub(crate) fn empty_letters(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    if glyphs::glyph_data().is_none() {
        sink.skip("the glyph database cannot be read");
        return;
    }
    let font = &ctx.font;
    let ink = Ink::new(font);
    let names = Names::new(font);
    let mut blank_hangul = 0usize;
    for (code, gid) in best_cmap(font) {
        if !ink.is_empty(gid) {
            continue;
        }
        let rare_hangul = (code_pages::HANGUL_SYLLABLES.0..=code_pages::HANGUL_SYLLABLES.1).contains(&code) && !code_pages::is_ks_x_1001_hangul(code);
        if rare_hangul {
            blank_hangul += 1;
        } else if glyphs::is_letter(code) && !chars::HANGUL_FILLERS.contains(&code) {
            let glyph = glyph_label(ctx, &names, gid);
            sink.push(
                Hit::new("empty-letter", format!("{} should be visible, but its glyph `{glyph}` is empty.", cp(code)))
                    .data("codepoint", f64::from(code))
                    .place(glyph_place(glyph, "glyf")),
            );
        }
    }
    if blank_hangul > 0 {
        sink.push(Hit::new("empty-hangul-letter", format!("{blank_hangul} Hangul syllable glyphs are empty.")).data("count", blank_hangul as f64));
    }
}

/// `check.glyphset.softHyphen`: the font maps U+00AD.
pub(crate) fn soft_hyphen(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(&gid) = best_cmap(&ctx.font).get(&SOFT_HYPHEN) else { return };
    let glyph = glyph_label(ctx, &Names::new(&ctx.font), gid);
    sink.push(
        Hit::new("softhyphen", format!("The font maps U+00AD SOFT HYPHEN to `{glyph}`; text engines draw a hyphen there themselves."))
            .data("codepoint", f64::from(SOFT_HYPHEN))
            .place(glyph_place(glyph, "cmap")),
    );
}

/// `check.glyphset.rupee`: a font that maps a letter of an Indic script lacks U+20B9.
pub(crate) fn rupee(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let cmap = best_cmap(&ctx.font);
    if !chars::INDIC_LETTERS.iter().any(|c| cmap.contains_key(c)) {
        sink.skip("the font supports no Indic script");
        return;
    }
    if !cmap.contains_key(&chars::RUPEE) {
        sink.push(
            Hit::new("missing-rupee", "The font supports an Indic script but has no glyph for U+20B9 INDIAN RUPEE SIGN.").place(table_place("cmap")),
        );
    }
}

/// Whether OS/2 claims CJK support: a CJK code page bit, or a CJK Unicode range bit below 96.
fn claims_cjk(font: &FontRef<'_>) -> bool {
    let Ok(os2) = font.os2() else { return false };
    let pages = os2.ul_code_page_range_1().unwrap_or(0);
    let ranges = [os2.ul_unicode_range_1(), os2.ul_unicode_range_2(), os2.ul_unicode_range_3()];
    let range_bit = |bit: u8| ranges.get(usize::from(bit / 32)).is_some_and(|r| r & (1u32 << (bit % 32)) != 0);
    code_pages::CJK_BITS.iter().any(|b| *b < 32 && pages & (1u32 << b) != 0) || chars::CJK_UNICODE_RANGE_BITS.iter().any(|b| *b < 96 && range_bit(*b))
}

/// `check.glyphset.cjkGlyphCount`: OS/2 claims CJK support but fewer than 150 code points (and
/// more than none) fall in the CJK blocks.
pub(crate) fn cjk_glyph_count(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    if !claims_cjk(&ctx.font) {
        sink.skip("the font does not claim CJK support");
        return;
    }
    let cjk: Vec<u32> = best_cmap(&ctx.font).into_keys().filter(|c| chars::is_cjk(*c)).collect();
    if !cjk.is_empty() && cjk.len() < chars::MIN_CJK_GLYPHS {
        sink.push(
            Hit::new(
                "cjk-not-enough-glyphs",
                format!(
                    "The font claims CJK support but maps only {} CJK characters ({}); the smallest CJK set has 150. Check their code points.",
                    cjk.len(),
                    cp_list(&cjk)
                ),
            )
            .data("count", cjk.len() as f64)
            .place(table_place("OS/2")),
        );
    }
}

/// `check.glyphset.arabicHighHamza`: U+0674 is a mark in GDEF (a failure) or its outline area
/// differs from U+0621's by more than a tenth (a warning).
pub(crate) fn arabic_high_hamza(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let cmap = best_cmap(font);
    let Some(&high) = cmap.get(&chars::ARABIC_HIGH_HAMZA) else {
        sink.skip("the font has no glyph for U+0674");
        return;
    };
    let names = Names::new(font);
    let glyph = glyph_label(ctx, &names, high);
    if gdef_class(font, high) == Some(3) {
        sink.push(
            Hit::new("mark-in-gdef", format!("`{glyph}` (U+0674 ARABIC LETTER HIGH HAMZA) is a mark in GDEF; it is a letter."))
                .place(glyph_place(glyph.clone(), "GDEF")),
        );
    }
    let Some(&hamza) = cmap.get(&ARABIC_HAMZA) else { return };
    let outlines = font.outline_glyphs();
    let (Some(a_hamza), Some(a_high)) = (area(&outlines, hamza), area(&outlines, high)) else { return };
    if a_hamza == 0.0 {
        return;
    }
    let ratio = ((a_high - a_hamza) / a_hamza).abs();
    if ratio.is_finite() && ratio > HIGH_HAMZA_AREA_TOLERANCE {
        sink.push(
            Hit::new(
                "glyph-area",
                format!(
                    "`{glyph}` (U+0674) should be about the size of the hamza U+0621, raised; its outline area differs by {:.0}%.",
                    ratio * 100.0
                ),
            )
            .data("ratio", ratio)
            .place(glyph_place(glyph, "glyf")),
        );
    }
}

/// `check.glyphset.arabicSpacingSymbols`: a glyph mapped (in any subtable) from U+FBB2 to
/// U+FBC2 is a mark in GDEF.
pub(crate) fn arabic_spacing_symbols(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let mut gids: Vec<(u32, u32)> =
        cmap_mappings(font, false).into_iter().filter(|(c, _)| chars::is_arabic_spacing_symbol(*c)).map(|(c, g)| (g, c)).collect();
    gids.sort_unstable();
    gids.dedup_by_key(|(g, _)| *g);
    let names = Names::new(font);
    for (gid, code) in gids {
        if gdef_class(font, gid) == Some(3) {
            let glyph = glyph_label(ctx, &names, gid);
            sink.push(
                Hit::new("mark-in-gdef", format!("`{glyph}` ({}) is a mark in GDEF; the Arabic spacing symbols are base glyphs.", cp(code)))
                    .data("codepoint", f64::from(code))
                    .place(glyph_place(glyph, "GDEF")),
            );
        }
    }
}

/// `check.glyphset.unicodeCmapMissing`: no `cmap`, or no non-empty Unicode subtable (platform 0
/// encoding 3, or platform 3 encoding 1).
pub(crate) fn unicode_cmap_missing(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    match unicode_subtable(&ctx.font) {
        None => sink.push(Hit::new("no-cmap", "The font has no cmap table.").place(table_place("cmap"))),
        Some(m) if m.is_empty() => sink.push(
            Hit::new("no-unicode-cmap", "The font has no Unicode cmap subtable (platform 0 encoding 3, or platform 3 encoding 1).")
                .place(table_place("cmap")),
        ),
        Some(_) => {}
    }
}

/// `check.glyphset.doubleEncodedGlyphs`: a glyph the Unicode subtable maps from more than one
/// code point. CID-keyed fonts are skipped.
pub(crate) fn double_encoded_glyphs(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    if is_cid(&ctx.font) {
        sink.skip("the font is CID-keyed");
        return;
    }
    let Some(mappings) = unicode_subtable(&ctx.font) else { return };
    let mut by_glyph: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (c, g) in mappings {
        by_glyph.entry(g).or_default().push(c);
    }
    let names = Names::new(&ctx.font);
    for (gid, mut codes) in by_glyph.into_iter().filter(|(_, c)| c.len() > 1) {
        codes.sort_unstable();
        let glyph = glyph_label(ctx, &names, gid);
        sink.push(
            Hit::new("double-mapped", format!("`{glyph}` is mapped from {}; text extracted from documents cannot tell them apart.", cp_list(&codes)))
                .data("count", codes.len() as f64)
                .place(glyph_place(glyph, "cmap")),
        );
    }
}

/// `check.glyphset.privateUseCodepoints`: code points in the private use areas (warnings;
/// U+F8FF is exempt, as one reference leaves it) or surrogates (failures).
pub(crate) fn private_use_codepoints(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let names = Names::new(&ctx.font);
    for (code, gid) in best_cmap(&ctx.font) {
        let hit = if contours::is_private_use(code) && code != VENDOR_LOGO {
            Hit::new("pua-encoded", format!("`{}` is mapped from the private use code point {}.", glyph_label(ctx, &names, gid), cp(code)))
        } else if (0xD800..=0xDFFF).contains(&code) {
            Hit::new(
                "surrogate-encoded",
                format!("`{}` is mapped from the surrogate code point {}, which is not a character.", glyph_label(ctx, &names, gid), cp(code)),
            )
        } else {
            continue;
        };
        sink.push(hit.data("codepoint", f64::from(code)).place(glyph_place(glyph_label(ctx, &names, gid), "cmap")));
    }
}

/// `check.glyphset.minimumCoverage`: the font lacks code points of the project's `glyphSet`.
/// The binary context does not carry project values, so the check skips even when the value is
/// set.
pub(crate) fn minimum_coverage(_: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    sink.skip("the run does not pass the project's glyph set to compiled-font checks");
}

/// `check.glyphset.rendersOwnName`: the Windows English family name (name ID 1) shapes to
/// .notdef somewhere. Without a shaper, every character of the name must be mapped.
pub(crate) fn renders_own_name(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let mut budget = MAX_NAME_TEXT_BYTES;
    let records = match name_records::read(font, &mut budget, |p, e, l, id| p == 3 && e == 1 && l == 0x409 && id == 1) {
        Ok(r) => r,
        Err(reason) => {
            sink.skip(reason);
            return;
        }
    };
    let Some(family) = records.records.into_iter().find_map(|r| r.text).filter(|t| !t.is_empty()) else {
        sink.skip("the font has no Windows English family name");
        return;
    };
    let place = || Location { name_id: Some(1), platform: Some(3), ..table_place("name") };
    let request = ShapeRequest { text: family.clone(), ..ShapeRequest::default() };
    match ctx.services.shape(ctx.bytes, &request) {
        Ok(shaped) => {
            if shaped.iter().any(|g| g.glyph_id == 0) {
                sink.push(
                    Hit::new("render-own-name", format!("Shaping the family name \u{201C}{family}\u{201D} gives .notdef glyphs.")).place(place()),
                );
            }
        }
        Err(_) => {
            let cmap = best_cmap(font);
            let mut missing: Vec<u32> = family.chars().map(u32::from).filter(|c| !cmap.contains_key(c)).collect();
            missing.sort_unstable();
            missing.dedup();
            if !missing.is_empty() {
                sink.push(
                    Hit::new(
                        "render-own-name",
                        format!("The family name \u{201C}{family}\u{201D} uses characters the font does not map: {}.", cp_list(&missing)),
                    )
                    .data("count", missing.len() as f64)
                    .place(place()),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Contour counts
// ---------------------------------------------------------------------------------------------

/// `check.outlines.contourCount`: an encoded glyph of a static TrueType font whose decomposed
/// contour count is not one the expected-count table lists for its code point; no contours where
/// some are expected is a failure.
pub(crate) fn contour_count(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    if font.cmap().is_err() {
        sink.push(
            Hit::new("lacks-cmap", "The font has no cmap table, so its glyphs' expected contour counts are unknown.").place(table_place("cmap")),
        );
        return;
    }
    let (Ok(glyf), Ok(loca)) = (font.glyf(), font.loca(None)) else { return };
    let names = Names::new(font);
    let mut budget = Budget::new();
    for (code, gid) in best_cmap(font) {
        let Some(expected) = contours::expected_contours(code) else { continue };
        let count = glyf_contours(&glyf, &loca, &names, gid, 0, &mut budget);
        if u8::try_from(count).is_ok_and(|c| expected.contains(&c)) {
            continue;
        }
        let glyph = glyph_label(ctx, &names, gid);
        let wanted: Vec<String> = expected.iter().map(u8::to_string).collect();
        let wanted = wanted.join(" or ");
        let hit = if count == 0 {
            Hit::new("no-contour", format!("`{glyph}` ({}) has no contours; {wanted} are expected.", cp(code)))
        } else {
            Hit::new(
                "contour-count",
                format!("`{glyph}` ({}) has {count} contours; {wanted} are usual. Check the drawing and the code point.", cp(code)),
            )
        };
        sink.push(hit.data("contours", count as f64).data("codepoint", f64::from(code)).place(glyph_place(glyph, "glyf")));
    }
}

// ---------------------------------------------------------------------------------------------
// Source checks
// ---------------------------------------------------------------------------------------------

/// `check.glyphset.duplicateCodepoint`: two exporting glyphs of the source claim one code
/// point. The finding sits on the later glyph and names the others.
pub(crate) fn duplicate_codepoint(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let mut owners: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
    for g in ctx.audit.font.glyphs.as_arcs().iter().filter(|g| g.export) {
        for u in &g.unicodes {
            let names = owners.entry(*u).or_default();
            if !names.contains(&g.name.as_str()) {
                names.push(&g.name);
            }
        }
    }
    for (code, names) in owners.into_iter().filter(|(_, n)| n.len() > 1) {
        let Some((last, others)) = names.split_last() else { continue };
        let others: Vec<String> = others.iter().take(MAX_IN_MESSAGE).map(|n| format!("`{n}`")).collect();
        sink.push(
            Hit::new(
                "duplicate-codepoint",
                format!("`{last}` has {}, which {} also has; only one of them can be reached.", cp(code), others.join(", ")),
            )
            .data("codepoint", f64::from(code))
            .place(Location { glyph: (*last).to_string(), ..Location::default() }),
        );
    }
}

/// The code point a glyph name stands for: the glyph database's record for that name (by its
/// own or its production name), else a `uniXXXX` or `uXXXXX` name.
fn implied_codepoint(data: &GlyphData, name: &str) -> Option<u32> {
    let info = data.lookup(name, &[]);
    if let Some(found) = info.found.filter(|f| matches!(f.via, Via::Name | Via::ProductionName)) {
        return found.record.unicode;
    }
    parse_code_point_name(name)
}

/// `check.glyphset.nameCodepointMismatch`: an exporting glyph's code points do not include the
/// one its name implies (including a glyph with no code point at all).
pub(crate) fn name_codepoint_mismatch(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(data) = glyphs::glyph_data() else {
        sink.skip("the glyph database cannot be read");
        return;
    };
    for g in ctx.audit.font.glyphs.as_arcs().iter().filter(|g| g.export) {
        let Some(implied) = implied_codepoint(data, &g.name) else { continue };
        if g.unicodes.contains(&implied) {
            continue;
        }
        let has = if g.unicodes.is_empty() { "no code point".to_string() } else { cp_list(&g.unicodes) };
        sink.push(
            Hit::new("codepoint-mismatch", format!("`{}` has {has}, but its name stands for {}.", g.name, cp(implied)))
                .data("codepoint", f64::from(implied))
                .place(Location { glyph: g.name.clone(), ..Location::default() }),
        );
    }
}

#[cfg(test)]
#[path = "glyphset_tests.rs"]
mod tests;
