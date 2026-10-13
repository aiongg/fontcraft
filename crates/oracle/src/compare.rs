//! Table-by-table comparison of two compiled fonts, normalised.
//!
//! What normalisation removes or canonicalises, per table:
//!
//! | Table | Removed or canonicalised |
//! |---|---|
//! | head | `checksumAdjustment`, `created`, `modified`; `indexToLocFormat` (an offset size) |
//! | hhea, vhea | `numberOfHMetrics`, `numberOfLongVerMetrics` (how many advances are spelt out; `hmtx` and `vmtx` compare the values) |
//! | hmtx, vmtx | compared as advance and side bearing per glyph, by glyph name |
//! | name | compared as a set of records; the compiler's own stamp at the end of the version string (ID 5: `;fontc …`, `;FontCraft …`) on either side; IDs 256 and up (feature and variation names) left out |
//! | cmap | compared as code point → glyph name mappings over all Unicode subtables, plus the list of subtables and the variation sequences |
//! | glyf, CFF | outlines drawn and compared within half a unit, start points free; for `glyf` also each glyph's kind (empty, simple, composite), components, flags, bounds and instructions. With overlaps removed on both sides ([`Outlines::Coverage`]), outlines that differ are compared by rendered coverage, and a simple glyph whose outline is coverage-equal is not compared by contour count or bounds |
//! | loca | offsets: compared through `glyf` |
//! | maxp, OS/2, post | field by field |
//!
//! Glyphs are matched by name. When the two fonts have as many glyphs and a glyph's name is only in
//! one of them, it is matched with the other font's glyph of the same id if that glyph's name is
//! also only in its font: the two compilers named the glyph differently (production names), which
//! `post` reports as `post glyphName <ours>`, and the per-glyph tables still compare it. Other
//! differences in the glyph sets or orders are reported once as a source-reading difference (the
//! two compilers read the source differently), and the per-glyph tables compare the glyphs both
//! fonts have.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use skrifa::MetadataProvider;
use skrifa::raw::tables::glyf::{CompositeGlyphFlags, Glyph as RawGlyph};
use skrifa::raw::tables::loca::LocaGlyph;
use skrifa::raw::types::{GlyphId, Tag};
use skrifa::raw::{FontRef, TableProvider};

use crate::outline::{self, Match};
use crate::{Difference, Finding, FindingKind, GlyphCounts, Oracle, Status, TableResult};

/// Most differences kept per table before the rest are only counted.
pub const MAX_KEPT: usize = 100_000;
/// Most code point mappings read from one cmap subtable.
const MAX_MAPPINGS: usize = 0x11_0000;
/// Most glyph names listed in a source-reading finding.
const MAX_LISTED: usize = 10;

/// The tables FontCraft writes in M2, in report order.
pub const CORE: &[&str] = &["head", "hhea", "maxp", "OS/2", "hmtx", "cmap", "name", "post", "glyf", "loca", "CFF ", "vhea", "vmtx"];
/// Layout tables.
pub const LAYOUT: &[&str] = &["GSUB", "GPOS", "GDEF", "kern", "BASE", "JSTF", "MATH"];
/// The layout tables compared lookup by lookup ([`crate::layout`]); the others are skipped.
pub const COMPARED_LAYOUT: &[&str] = &["GSUB", "GPOS", "GDEF"];
/// Variation tables: the static comparison skips them; `variable` compares them at locations.
pub const VARIATION: &[&str] = &["fvar", "gvar", "avar", "HVAR", "VVAR", "MVAR", "STAT", "cvar", "CFF2"];

/// How outlines are compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outlines {
    /// Segment by segment within half a unit.
    Exact,
    /// Segment by segment, and outlines that differ by rendered coverage
    /// ([`outline::coverage_equal`]): two overlap removals give the same shape with different
    /// points.
    Coverage,
}

impl Outlines {
    /// The comparison for builds against `oracle`.
    pub fn of(oracle: Oracle) -> Self {
        if oracle.removes_overlaps() { Outlines::Coverage } else { Outlines::Exact }
    }
}

/// Result of comparing two fonts.
#[derive(Clone, Debug, Default)]
pub struct Comparison {
    pub tables: Vec<TableResult>,
    pub source_reading: Vec<Finding>,
    pub notes: Vec<String>,
}

/// One font, read.
struct Side<'a> {
    bytes: &'a [u8],
    font: FontRef<'a>,
    names: Vec<String>,
    by_name: HashMap<String, GlyphId>,
}

impl<'a> Side<'a> {
    fn new(bytes: &'a [u8], who: &str) -> Result<Side<'a>, String> {
        let font = FontRef::new(bytes).map_err(|e| format!("{who} cannot be read: {e}"))?;
        let count = u32::from(font.maxp().map(|m| m.num_glyphs()).unwrap_or(0));
        let glyph_names = font.glyph_names();
        let mut names = Vec::with_capacity(count as usize);
        let mut by_name = HashMap::with_capacity(count as usize);
        for gid in 0..count {
            let id = GlyphId::new(gid);
            let base = glyph_names.get(id).filter(|n| !n.is_synthesized()).map(|n| n.as_str().to_string()).unwrap_or_else(|| format!("gid{gid}"));
            // A name used twice gets the glyph id appended, so lookups stay one-to-one.
            let name = if by_name.contains_key(&base) { format!("{base}#{gid}") } else { base };
            by_name.insert(name.clone(), id);
            names.push(name);
        }
        Ok(Side { bytes, font, names, by_name })
    }

    fn name(&self, gid: GlyphId) -> String {
        self.names.get(gid.to_u32() as usize).cloned().unwrap_or_else(|| format!("gid{}", gid.to_u32()))
    }

    fn table(&self, tag: &str) -> Option<&'a [u8]> {
        self.font.table_data(tag_of(tag)).map(|d| d.as_bytes())
    }

    fn tags(&self) -> BTreeSet<String> {
        self.font.table_directory().table_records().iter().map(|r| tag_text(r.tag())).collect()
    }
}

fn tag_of(s: &str) -> Tag {
    let mut b = [b' '; 4];
    for (slot, c) in b.iter_mut().zip(s.bytes()) {
        *slot = c;
    }
    Tag::new(&b)
}

fn tag_text(t: Tag) -> String {
    String::from_utf8_lossy(&t.to_be_bytes()).into_owned()
}

/// The glyphs both fonts have: name, our id, their id, in our order.
type Common = Vec<(String, GlyphId, GlyphId)>;

/// The glyphs the two fonts name differently: glyph id, our name, their name. Only when both
/// fonts have as many glyphs, and only where neither name is in the other font and both glyphs
/// have the same code points (a glyph the reference made in place of ours is not renamed).
fn renamed(a: &Side, b: &Side) -> Vec<(u32, String, String)> {
    if a.names.len() != b.names.len() {
        return Vec::new();
    }
    let (cps_a, cps_b) = (code_points(a), code_points(b));
    a.names
        .iter()
        .zip(&b.names)
        .enumerate()
        .filter(|(_, (x, y))| x != y && !b.by_name.contains_key(*x) && !a.by_name.contains_key(*y))
        .filter_map(|(gid, (x, y))| Some((u32::try_from(gid).ok()?, x.clone(), y.clone())))
        .filter(|(gid, _, _)| cps_a.get(gid) == cps_b.get(gid))
        .collect()
}

/// Each glyph's code points, by glyph id, from the font's preferred Unicode subtable.
fn code_points(s: &Side) -> HashMap<u32, Vec<u32>> {
    let mut out: HashMap<u32, Vec<u32>> = HashMap::new();
    for (cp, gid) in s.font.charmap().mappings().take(MAX_MAPPINGS) {
        out.entry(gid.to_u32()).or_default().push(cp);
    }
    out
}

/// Compares `ours` with `theirs` (the reference compiler's font), outlines exactly.
pub fn compare(ours: &[u8], theirs: &[u8]) -> Result<Comparison, String> {
    compare_with(ours, theirs, Outlines::Exact)
}

/// Compares `ours` with `theirs`, outlines as `outlines` says.
pub fn compare_with(ours: &[u8], theirs: &[u8], outlines: Outlines) -> Result<Comparison, String> {
    let a = Side::new(ours, "FontCraft's font")?;
    let b = Side::new(theirs, "the reference font")?;
    let mut out = Comparison::default();
    if b.font.fvar().is_ok() {
        out.notes.push("the reference is a variable font: its default location is compared".into());
    }

    // Glyphs named differently are compared under our names; `post` reports the names.
    let renamed = renamed(&a, &b);
    let mut b = b;
    for (gid, ours, _) in &renamed {
        if let Some(slot) = b.names.get_mut(*gid as usize) {
            b.by_name.remove(slot.as_str());
            slot.clone_from(ours);
            b.by_name.insert(ours.clone(), GlyphId::new(*gid));
        }
    }

    // Source reading: glyph set and order.
    let only_a: Vec<&String> = a.names.iter().filter(|n| !b.by_name.contains_key(*n)).collect();
    let only_b: Vec<&String> = b.names.iter().filter(|n| !a.by_name.contains_key(*n)).collect();
    if !only_a.is_empty() || !only_b.is_empty() {
        out.source_reading.push(Finding {
            kind: FindingKind::GlyphSet,
            message: format!(
                "glyph sets differ: {} only in FontCraft's ({}), {} only in the reference's ({})",
                only_a.len(),
                list(&only_a),
                only_b.len(),
                list(&only_b)
            ),
        });
    } else if a.names != b.names {
        let at = a.names.iter().zip(&b.names).position(|(x, y)| x != y).unwrap_or(0);
        out.source_reading.push(Finding {
            kind: FindingKind::GlyphOrder,
            message: format!(
                "glyph order differs from glyph {at}: `{}` in FontCraft's, `{}` in the reference's",
                a.names.get(at).map(String::as_str).unwrap_or(""),
                b.names.get(at).map(String::as_str).unwrap_or("")
            ),
        });
    }
    let common: Common = a.names.iter().filter_map(|n| Some((n.clone(), *a.by_name.get(n)?, *b.by_name.get(n)?))).collect();

    let tags_a = a.tags();
    let tags_b = b.tags();
    let mut order: Vec<String> = CORE.iter().map(|t| (*t).to_string()).filter(|t| tags_a.contains(t) || tags_b.contains(t)).collect();
    order.extend(tags_a.union(&tags_b).filter(|t| !CORE.contains(&t.as_str())).cloned());
    for tag in order {
        out.tables.push(table(&a, &b, &common, &renamed, &tag, tags_a.contains(&tag), tags_b.contains(&tag), outlines));
    }
    Ok(out)
}

fn list(names: &[&String]) -> String {
    let mut s: Vec<&str> = names.iter().take(MAX_LISTED).map(|n| n.as_str()).collect();
    if names.len() > MAX_LISTED {
        s.push("…");
    }
    s.join(", ")
}

fn skipped(tag: &str, note: &str) -> TableResult {
    TableResult {
        tag: tag.to_string(),
        status: Status::Skipped,
        differences: Vec::new(),
        count: 0,
        note: Some(note.to_string()),
        allowed: Vec::new(),
        glyphs: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn table(
    a: &Side,
    b: &Side,
    common: &Common,
    renamed: &[(u32, String, String)],
    tag: &str,
    in_a: bool,
    in_b: bool,
    outlines: Outlines,
) -> TableResult {
    if COMPARED_LAYOUT.contains(&tag) {
        match (in_a, in_b) {
            (true, false) => return result(tag, false, vec![diff("table", "present", "absent")], None),
            (false, true) => return result(tag, false, vec![diff("table", "absent", "present")], None),
            _ => {}
        }
        let same = a.table(tag) == b.table(tag);
        let rename: BTreeMap<String, String> = renamed.iter().map(|(_, ours, theirs)| (theirs.clone(), ours.clone())).collect();
        return match crate::layout::compare(tag, a.bytes, b.bytes, &rename) {
            Ok(differences) => result(tag, same, differences, Some("compared lookup by lookup as feature code".to_string())),
            Err(e) => skipped(tag, &format!("cannot be decompiled: {e}")),
        };
    }
    if LAYOUT.contains(&tag) {
        return skipped(tag, "layout table: not compared");
    }
    if VARIATION.contains(&tag) {
        return skipped(tag, "variation table: compared by the variable run (--variable)");
    }
    if tag == "DSIG" {
        return skipped(tag, "signature: not compared");
    }
    let has_outlines = |t: &BTreeSet<String>| t.contains("glyf") || t.contains("CFF ");
    if !(in_a && in_b) && (tag == "glyf" || tag == "loca" || tag == "CFF ") && has_outlines(&a.tags()) && has_outlines(&b.tags()) {
        return skipped(tag, "the two fonts have different outline flavours");
    }
    match (in_a, in_b) {
        (true, false) => return result(tag, false, vec![diff("table", "present", "absent")], None),
        (false, true) if CORE.contains(&tag) => return result(tag, false, vec![diff("table", "absent", "present")], None),
        (false, true) => return skipped(tag, "not written by FontCraft yet (outside M2's core tables)"),
        _ => {}
    }
    let (Some(raw_a), Some(raw_b)) = (a.table(tag), b.table(tag)) else { return skipped(tag, "cannot be read") };
    let same_bytes = raw_a == raw_b;
    let mut glyphs = None;
    let (diffs, note) = match tag {
        "head" => (head(a, b), None),
        "hhea" => (hhea(a, b), None),
        "vhea" => (vhea(a, b), None),
        "vmtx" => (vmtx(a, b, common), None),
        "maxp" => (maxp(a, b), None),
        "OS/2" => (os2(a, b), None),
        "post" => (post(a, b, renamed), None),
        "name" => name(a, b),
        "cmap" => (cmap(a, b), None),
        "hmtx" => (hmtx(a, b, common), None),
        "glyf" => {
            let (d, g) = glyf(a, b, common, outlines);
            glyphs = Some(g);
            (d, variable_reference(b).then(|| "the reference is variable: USE_MY_METRICS is not compared".to_string()))
        }
        "CFF " => {
            let (d, g) = cff(a, b, common, outlines);
            glyphs = Some(g);
            (d, Some("decoded outlines compared; size and encoding are not".to_string()))
        }
        "loca" => (Vec::new(), Some("offsets: compared through glyf".to_string())),
        _ => {
            let d = if same_bytes { Vec::new() } else { vec![diff("bytes", format!("{} bytes", raw_a.len()), format!("{} bytes", raw_b.len()))] };
            (d, Some("compared as bytes".to_string()))
        }
    };
    let identical = same_bytes || (tag == "head" && head_bytes_equal(raw_a, raw_b));
    let mut out = result(tag, identical, diffs, note);
    if out.status == Status::Equivalent && glyphs.is_some_and(|g| g.coverage_equal > 0) {
        out.status = Status::CoverageEqual;
    }
    out.glyphs = glyphs;
    out
}

fn result(tag: &str, identical: bool, mut differences: Vec<Difference>, note: Option<String>) -> TableResult {
    let count = differences.len();
    differences.truncate(MAX_KEPT);
    let status = if !differences.is_empty() {
        Status::Different
    } else if identical {
        Status::Identical
    } else {
        Status::Equivalent
    };
    TableResult { tag: tag.to_string(), status, differences, count, note, allowed: Vec::new(), glyphs: None }
}

fn diff(key: impl Into<String>, ours: impl Into<String>, theirs: impl Into<String>) -> Difference {
    Difference { key: key.into(), ours: ours.into(), theirs: theirs.into() }
}

/// head bytes with the checksum adjustment and both dates set to zero.
fn head_bytes_equal(a: &[u8], b: &[u8]) -> bool {
    let blank = |t: &[u8]| {
        let mut t = t.to_vec();
        for i in (8..12).chain(20..36) {
            if let Some(byte) = t.get_mut(i) {
                *byte = 0;
            }
        }
        t
    };
    blank(a) == blank(b)
}

type Fields = Vec<(&'static str, String)>;

fn fields(prefix: &str, a: Option<Fields>, b: Option<Fields>) -> Vec<Difference> {
    match (a, b) {
        (Some(a), Some(b)) => a
            .iter()
            .zip(&b)
            .filter(|((_, x), (_, y))| x != y)
            .map(|((name, x), (_, y))| diff(format!("{prefix}.{name}"), x.clone(), y.clone()))
            .collect(),
        (a, b) => vec![diff(prefix, if a.is_some() { "readable" } else { "unreadable" }, if b.is_some() { "readable" } else { "unreadable" })],
    }
}

fn opt<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map(|v| v.to_string()).unwrap_or_else(|| "absent".into())
}

fn head(a: &Side, b: &Side) -> Vec<Difference> {
    let f = |s: &Side| -> Option<Fields> {
        let h = s.font.head().ok()?;
        Some(vec![
            ("version", format!("{}.{}", h.version().major, h.version().minor)),
            ("fontRevision", h.font_revision().to_f64().to_string()),
            ("magicNumber", format!("{:#010x}", h.magic_number())),
            ("flags", format!("{:#06x}", h.flags().bits())),
            ("unitsPerEm", h.units_per_em().to_string()),
            ("xMin", h.x_min().to_string()),
            ("yMin", h.y_min().to_string()),
            ("xMax", h.x_max().to_string()),
            ("yMax", h.y_max().to_string()),
            ("macStyle", format!("{:#06x}", h.mac_style().bits())),
            ("lowestRecPPEM", h.lowest_rec_ppem().to_string()),
            ("fontDirectionHint", h.font_direction_hint().to_string()),
            ("glyphDataFormat", h.glyph_data_format().to_string()),
        ])
    };
    fields("head", f(a), f(b))
}

fn hhea(a: &Side, b: &Side) -> Vec<Difference> {
    let f = |s: &Side| -> Option<Fields> {
        let h = s.font.hhea().ok()?;
        Some(vec![
            ("version", format!("{}.{}", h.version().major, h.version().minor)),
            ("ascender", h.ascender().to_i16().to_string()),
            ("descender", h.descender().to_i16().to_string()),
            ("lineGap", h.line_gap().to_i16().to_string()),
            ("advanceWidthMax", h.advance_width_max().to_u16().to_string()),
            ("minLeftSideBearing", h.min_left_side_bearing().to_i16().to_string()),
            ("minRightSideBearing", h.min_right_side_bearing().to_i16().to_string()),
            ("xMaxExtent", h.x_max_extent().to_i16().to_string()),
            ("caretSlopeRise", h.caret_slope_rise().to_string()),
            ("caretSlopeRun", h.caret_slope_run().to_string()),
            ("caretOffset", h.caret_offset().to_string()),
            ("metricDataFormat", h.metric_data_format().to_string()),
        ])
    };
    fields("hhea", f(a), f(b))
}

fn vhea(a: &Side, b: &Side) -> Vec<Difference> {
    let f = |s: &Side| -> Option<Fields> {
        let h = s.font.vhea().ok()?;
        Some(vec![
            ("version", format!("{:?}", h.version())),
            ("ascender", h.ascender().to_i16().to_string()),
            ("descender", h.descender().to_i16().to_string()),
            ("lineGap", h.line_gap().to_i16().to_string()),
            ("advanceHeightMax", h.advance_height_max().to_u16().to_string()),
            ("minTopSideBearing", h.min_top_side_bearing().to_i16().to_string()),
            ("minBottomSideBearing", h.min_bottom_side_bearing().to_i16().to_string()),
            ("yMaxExtent", h.y_max_extent().to_i16().to_string()),
            ("caretSlopeRise", h.caret_slope_rise().to_string()),
            ("caretSlopeRun", h.caret_slope_run().to_string()),
            ("caretOffset", h.caret_offset().to_string()),
            ("metricDataFormat", h.metric_data_format().to_string()),
        ])
    };
    fields("vhea", f(a), f(b))
}

fn vmtx(a: &Side, b: &Side, common: &Common) -> Vec<Difference> {
    let (Ok(va), Ok(vb)) = (a.font.vmtx(), b.font.vmtx()) else { return vec![diff("vmtx", "readable", "unreadable")] };
    let mut out = Vec::new();
    for (name, ga, gb) in common {
        let (adv_a, adv_b) = (va.advance(*ga), vb.advance(*gb));
        if adv_a != adv_b {
            out.push(diff(format!("vmtx {name} advance"), opt(adv_a), opt(adv_b)));
        }
        let (tsb_a, tsb_b) = (va.side_bearing(*ga), vb.side_bearing(*gb));
        if tsb_a != tsb_b {
            out.push(diff(format!("vmtx {name} tsb"), opt(tsb_a), opt(tsb_b)));
        }
    }
    out
}

fn maxp(a: &Side, b: &Side) -> Vec<Difference> {
    let f = |s: &Side| -> Option<Fields> {
        let m = s.font.maxp().ok()?;
        Some(vec![
            ("version", format!("{:?}", m.version())),
            ("numGlyphs", m.num_glyphs().to_string()),
            ("maxPoints", opt(m.max_points())),
            ("maxContours", opt(m.max_contours())),
            ("maxCompositePoints", opt(m.max_composite_points())),
            ("maxCompositeContours", opt(m.max_composite_contours())),
            ("maxZones", opt(m.max_zones())),
            ("maxTwilightPoints", opt(m.max_twilight_points())),
            ("maxStorage", opt(m.max_storage())),
            ("maxFunctionDefs", opt(m.max_function_defs())),
            ("maxInstructionDefs", opt(m.max_instruction_defs())),
            ("maxStackElements", opt(m.max_stack_elements())),
            ("maxSizeOfInstructions", opt(m.max_size_of_instructions())),
            ("maxComponentElements", opt(m.max_component_elements())),
            ("maxComponentDepth", opt(m.max_component_depth())),
        ])
    };
    fields("maxp", f(a), f(b))
}

fn os2(a: &Side, b: &Side) -> Vec<Difference> {
    let f = |s: &Side| -> Option<Fields> {
        let o = s.font.os2().ok()?;
        let hex = |v: u32| format!("{v:#010x}");
        Some(vec![
            ("version", o.version().to_string()),
            ("xAvgCharWidth", o.x_avg_char_width().to_string()),
            ("usWeightClass", o.us_weight_class().to_string()),
            ("usWidthClass", o.us_width_class().to_string()),
            ("fsType", format!("{:#06x}", o.fs_type())),
            ("ySubscriptXSize", o.y_subscript_x_size().to_string()),
            ("ySubscriptYSize", o.y_subscript_y_size().to_string()),
            ("ySubscriptXOffset", o.y_subscript_x_offset().to_string()),
            ("ySubscriptYOffset", o.y_subscript_y_offset().to_string()),
            ("ySuperscriptXSize", o.y_superscript_x_size().to_string()),
            ("ySuperscriptYSize", o.y_superscript_y_size().to_string()),
            ("ySuperscriptXOffset", o.y_superscript_x_offset().to_string()),
            ("ySuperscriptYOffset", o.y_superscript_y_offset().to_string()),
            ("yStrikeoutSize", o.y_strikeout_size().to_string()),
            ("yStrikeoutPosition", o.y_strikeout_position().to_string()),
            ("sFamilyClass", o.s_family_class().to_string()),
            ("panose", format!("{:?}", o.panose_10())),
            ("ulUnicodeRange1", hex(o.ul_unicode_range_1())),
            ("ulUnicodeRange2", hex(o.ul_unicode_range_2())),
            ("ulUnicodeRange3", hex(o.ul_unicode_range_3())),
            ("ulUnicodeRange4", hex(o.ul_unicode_range_4())),
            ("achVendID", format!("{:?}", tag_text(o.ach_vend_id()))),
            ("fsSelection", format!("{:#06x}", o.fs_selection().bits())),
            ("usFirstCharIndex", o.us_first_char_index().to_string()),
            ("usLastCharIndex", o.us_last_char_index().to_string()),
            ("sTypoAscender", o.s_typo_ascender().to_string()),
            ("sTypoDescender", o.s_typo_descender().to_string()),
            ("sTypoLineGap", o.s_typo_line_gap().to_string()),
            ("usWinAscent", o.us_win_ascent().to_string()),
            ("usWinDescent", o.us_win_descent().to_string()),
            ("ulCodePageRange1", opt(o.ul_code_page_range_1().map(hex))),
            ("ulCodePageRange2", opt(o.ul_code_page_range_2().map(hex))),
            ("sxHeight", opt(o.sx_height())),
            ("sCapHeight", opt(o.s_cap_height())),
            ("usDefaultChar", opt(o.us_default_char())),
            ("usBreakChar", opt(o.us_break_char())),
            ("usMaxContext", opt(o.us_max_context())),
            ("usLowerOpticalPointSize", opt(o.us_lower_optical_point_size())),
            ("usUpperOpticalPointSize", opt(o.us_upper_optical_point_size())),
        ])
    };
    fields("OS/2", f(a), f(b))
}

/// The fields, then each glyph the two fonts name differently (`post glyphName <ours>`).
fn post(a: &Side, b: &Side, renamed: &[(u32, String, String)]) -> Vec<Difference> {
    let f = |s: &Side| -> Option<Fields> {
        let p = s.font.post().ok()?;
        Some(vec![
            ("version", format!("{:?}", p.version())),
            ("italicAngle", p.italic_angle().to_f64().to_string()),
            ("underlinePosition", p.underline_position().to_i16().to_string()),
            ("underlineThickness", p.underline_thickness().to_i16().to_string()),
            ("isFixedPitch", p.is_fixed_pitch().to_string()),
            ("minMemType42", p.min_mem_type42().to_string()),
            ("maxMemType42", p.max_mem_type42().to_string()),
            ("minMemType1", p.min_mem_type1().to_string()),
            ("maxMemType1", p.max_mem_type1().to_string()),
            ("numGlyphs", opt(p.num_glyphs())),
        ])
    };
    let mut out = fields("post", f(a), f(b));
    out.extend(renamed.iter().map(|(_, x, y)| diff(format!("post glyphName {x}"), x.clone(), y.clone())));
    out
}

/// A name record: platform, encoding, language, name id.
type NameKey = (u16, u16, u16, u16);

/// A compiler's own stamp at the end of a version string (`Version 1.000;fontc 1.0.0`,
/// `Version 1.000;FontCraft 0.1.0`).
pub(crate) fn strip_tool_stamp(s: &str) -> (&str, bool) {
    match s.rsplit_once(';') {
        Some((head, tail)) if tail.trim_start().starts_with("fontc ") || tail.trim_start().starts_with("FontCraft ") => (head, true),
        _ => (s, false),
    }
}

fn name_records(s: &Side) -> Option<(BTreeMap<NameKey, String>, usize)> {
    let name = s.font.name().ok()?;
    let data = name.string_data();
    let mut out = BTreeMap::new();
    let mut high = 0;
    for r in name.name_record() {
        let id = r.name_id().to_u16();
        if id >= 256 {
            high += 1;
            continue;
        }
        let text = r.string(data).map(|t| t.chars().collect::<String>()).unwrap_or_else(|_| "<unreadable>".into());
        out.insert((r.platform_id(), r.encoding_id(), r.language_id(), id), text);
    }
    Some((out, high))
}

fn name(a: &Side, b: &Side) -> (Vec<Difference>, Option<String>) {
    let (Some((ra, high_a)), Some((rb, high_b))) = (name_records(a), name_records(b)) else {
        return (vec![diff("name", "readable", "unreadable or absent")], None);
    };
    let mut notes = Vec::new();
    let mut out = Vec::new();
    let keys: BTreeSet<&NameKey> = ra.keys().chain(rb.keys()).collect();
    let mut stamped = false;
    for k in keys {
        let key = format!("name {}/{}/{:#06x}/{}", k.0, k.1, k.2, k.3);
        match (ra.get(k), rb.get(k)) {
            (Some(x), Some(y)) => {
                let (x, cut_x) = if k.3 == 5 { strip_tool_stamp(x) } else { (x.as_str(), false) };
                let (y, cut) = if k.3 == 5 { strip_tool_stamp(y) } else { (y.as_str(), false) };
                stamped |= cut || cut_x;
                if x != y {
                    out.push(diff(key, x, y));
                }
            }
            (x, y) => out.push(diff(key, x.cloned().unwrap_or_else(|| "absent".into()), y.cloned().unwrap_or_else(|| "absent".into()))),
        }
    }
    if stamped {
        notes.push("the compilers' stamps in the version string are not compared".to_string());
    }
    if high_a + high_b > 0 {
        notes.push(format!("records with name id 256 or more left out ({high_a} in FontCraft's, {high_b} in the reference's)"));
    }
    (out, (!notes.is_empty()).then(|| notes.join("; ")))
}

/// cmap as data: the subtables, the Unicode mappings (to glyph names), the variation sequences.
struct CmapData {
    subtables: Vec<String>,
    map: BTreeMap<u32, String>,
    uvs: BTreeSet<(u32, u32, String)>,
    conflicts: Vec<u32>,
}

fn cmap_data(s: &Side) -> Option<CmapData> {
    use skrifa::raw::tables::cmap::{CmapSubtable, MapVariant};
    let cmap = s.font.cmap().ok()?;
    let data = cmap.offset_data();
    let mut out = CmapData { subtables: Vec::new(), map: BTreeMap::new(), uvs: BTreeSet::new(), conflicts: Vec::new() };
    for rec in cmap.encoding_records() {
        let (p, e) = (rec.platform_id() as u16, rec.encoding_id());
        let Ok(sub) = rec.subtable(data) else {
            out.subtables.push(format!("{p}/{e} unreadable"));
            continue;
        };
        out.subtables.push(format!("{p}/{e} format {}", sub.format()));
        let unicode = p == 0 || (p == 3 && (e == 1 || e == 10));
        match sub {
            CmapSubtable::Format14(uvs) => {
                for (cp, selector, variant) in uvs.iter().take(MAX_MAPPINGS) {
                    let target = match variant {
                        MapVariant::UseDefault => "default".to_string(),
                        MapVariant::Variant(gid) => s.name(gid),
                    };
                    out.uvs.insert((cp, selector, target));
                }
            }
            sub if unicode => {
                for (cp, gid) in sub.iter().take(MAX_MAPPINGS) {
                    let name = s.name(gid);
                    match out.map.get(&cp) {
                        Some(prev) if *prev != name => out.conflicts.push(cp),
                        Some(_) => {}
                        None => {
                            out.map.insert(cp, name);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out.subtables.sort();
    Some(out)
}

fn cmap(a: &Side, b: &Side) -> Vec<Difference> {
    let (Some(x), Some(y)) = (cmap_data(a), cmap_data(b)) else { return vec![diff("cmap", "readable", "unreadable")] };
    let mut out = Vec::new();
    if x.subtables != y.subtables {
        out.push(diff("cmap subtables", x.subtables.join(", "), y.subtables.join(", ")));
    }
    if !x.conflicts.is_empty() || !y.conflicts.is_empty() {
        out.push(diff("cmap subtables disagree", format!("{} code points", x.conflicts.len()), format!("{} code points", y.conflicts.len())));
    }
    let cps: BTreeSet<&u32> = x.map.keys().chain(y.map.keys()).collect();
    for cp in cps {
        let (gx, gy) = (x.map.get(cp), y.map.get(cp));
        if gx != gy {
            out.push(diff(
                format!("cmap U+{cp:04X}"),
                gx.cloned().unwrap_or_else(|| "unmapped".into()),
                gy.cloned().unwrap_or_else(|| "unmapped".into()),
            ));
        }
    }
    for v in x.uvs.symmetric_difference(&y.uvs) {
        let mine = x.uvs.contains(v);
        let text = format!("{} for U+{:04X} U+{:04X}", v.2, v.0, v.1);
        out.push(diff(
            format!("cmap variation U+{:04X} U+{:04X}", v.0, v.1),
            if mine { text.clone() } else { "absent".into() },
            if mine { "absent".into() } else { text },
        ));
    }
    out
}

fn hmtx(a: &Side, b: &Side, common: &Common) -> Vec<Difference> {
    let (Ok(ha), Ok(hb)) = (a.font.hmtx(), b.font.hmtx()) else { return vec![diff("hmtx", "readable", "unreadable")] };
    let mut out = Vec::new();
    for (name, ga, gb) in common {
        let (adv_a, adv_b) = (ha.advance(*ga), hb.advance(*gb));
        if adv_a != adv_b {
            out.push(diff(format!("hmtx {name} advance"), opt(adv_a), opt(adv_b)));
        }
        let (lsb_a, lsb_b) = (ha.side_bearing(*ga), hb.side_bearing(*gb));
        if lsb_a != lsb_b {
            out.push(diff(format!("hmtx {name} lsb"), opt(lsb_a), opt(lsb_b)));
        }
    }
    out
}

/// Whether `s` is a variable font. fontc never sets `USE_MY_METRICS` in one (the flag is safe
/// only when every master's advances agree), and the static builds it is compared with do.
fn variable_reference(s: &Side) -> bool {
    s.font.fvar().is_ok()
}

/// A glyf entry's structure, as text that compares equal when the structures are equal.
/// `USE_MY_METRICS` is left out with `use_my_metrics` false.
fn glyf_structure(s: &Side, gid: GlyphId, use_my_metrics: bool) -> Result<(String, String, String), String> {
    let glyf = s.font.glyf().map_err(|e| e.to_string())?;
    let loca = s.font.loca(None).map_err(|e| e.to_string())?;
    match loca.get(gid, &glyf) {
        None => Err("unreadable".into()),
        Some(LocaGlyph::Empty) => Ok(("empty".into(), String::new(), String::new())),
        Some(LocaGlyph::Glyph(g)) => {
            let bounds = format!("{} {} {} {}", g.x_min(), g.y_min(), g.x_max(), g.y_max());
            match g {
                RawGlyph::Simple(simple) => Ok((
                    format!("simple, {} contours", simple.number_of_contours()),
                    bounds,
                    format!("{} instruction bytes", simple.instructions().len()),
                )),
                RawGlyph::Composite(c) => {
                    let metrics = if use_my_metrics { CompositeGlyphFlags::USE_MY_METRICS } else { CompositeGlyphFlags::empty() };
                    let keep = metrics
                        | CompositeGlyphFlags::ROUND_XY_TO_GRID
                        | CompositeGlyphFlags::OVERLAP_COMPOUND
                        | CompositeGlyphFlags::SCALED_COMPONENT_OFFSET
                        | CompositeGlyphFlags::UNSCALED_COMPONENT_OFFSET;
                    let parts: Vec<String> = c
                        .components()
                        .map(|comp| {
                            let t = comp.transform;
                            let anchor = match comp.anchor {
                                skrifa::raw::tables::glyf::Anchor::Offset { x, y } => format!("at {x} {y}"),
                                skrifa::raw::tables::glyf::Anchor::Point { base, component } => format!("points {base} {component}"),
                            };
                            let matrix = if (t.xx.to_f32(), t.yx.to_f32(), t.xy.to_f32(), t.yy.to_f32()) == (1.0, 0.0, 0.0, 1.0) {
                                String::new()
                            } else {
                                format!(" matrix {} {} {} {}", t.xx.to_f32(), t.yx.to_f32(), t.xy.to_f32(), t.yy.to_f32())
                            };
                            let flags = (comp.flags & keep).bits();
                            let flags = if flags == 0 { String::new() } else { format!(" flags {flags:#06x}") };
                            format!("{} {anchor}{matrix}{flags}", s.name(comp.glyph.into()))
                        })
                        .collect();
                    let instructions = c.instructions().map(<[u8]>::len).unwrap_or(0);
                    Ok((format!("composite of {}", parts.join(", ")), bounds, format!("{instructions} instruction bytes")))
                }
            }
        }
    }
}

fn glyf(a: &Side, b: &Side, common: &Common, outlines: Outlines) -> (Vec<Difference>, GlyphCounts) {
    let mut out = Vec::new();
    let mut counts = GlyphCounts::default();
    let use_my_metrics = !variable_reference(b);
    for (name, ga, gb) in common {
        let outline = outline_match("glyf", name, a, *ga, b, *gb, outlines);
        counts.count(&outline);
        // Two overlap removals may split a shape into contours differently and place its
        // extreme points a little differently; the coverage says whether the shape is the same.
        let same_shape = outline == OutlineMatch::CoverageEqual;
        match (glyf_structure(a, *ga, use_my_metrics), glyf_structure(b, *gb, use_my_metrics)) {
            (Ok(x), Ok(y)) => {
                let both_simple = x.0.starts_with("simple") && y.0.starts_with("simple");
                if x.0 != y.0 && !(same_shape && both_simple) {
                    // Bounds and instructions of glyphs of different kinds say nothing more.
                    out.push(diff(format!("glyf {name} structure"), x.0, y.0));
                } else {
                    if x.1 != y.1 && !same_shape {
                        out.push(diff(format!("glyf {name} bounds"), x.1, y.1));
                    }
                    if x.2 != y.2 {
                        out.push(diff(format!("glyf {name} instructions"), x.2, y.2));
                    }
                }
            }
            (x, y) => out.push(diff(format!("glyf {name} structure"), x.map(|v| v.0).unwrap_or_else(|e| e), y.map(|v| v.0).unwrap_or_else(|e| e))),
        }
        if let OutlineMatch::Different(d) = outline {
            out.push(d);
        }
    }
    (out, counts)
}

fn cff(a: &Side, b: &Side, common: &Common, outlines: Outlines) -> (Vec<Difference>, GlyphCounts) {
    let mut out = Vec::new();
    let mut counts = GlyphCounts::default();
    for (name, ga, gb) in common {
        let outline = outline_match("CFF", name, a, *ga, b, *gb, outlines);
        counts.count(&outline);
        if let OutlineMatch::Different(d) = outline {
            out.push(d);
        }
    }
    (out, counts)
}

/// How one glyph's outlines compared.
#[derive(Clone, Debug, PartialEq)]
enum OutlineMatch {
    Same,
    CoverageEqual,
    Different(Difference),
}

impl GlyphCounts {
    fn count(&mut self, m: &OutlineMatch) {
        self.compared += 1;
        match m {
            OutlineMatch::Same => self.same += 1,
            OutlineMatch::CoverageEqual => self.coverage_equal += 1,
            OutlineMatch::Different(_) => self.different += 1,
        }
    }
}

fn outline_match(table: &str, name: &str, a: &Side, ga: GlyphId, b: &Side, gb: GlyphId, outlines: Outlines) -> OutlineMatch {
    let key = format!("{table} {name} outline");
    match (outline::draw(&a.font, ga), outline::draw(&b.font, gb)) {
        (Ok(x), Ok(y)) => {
            let (x, y) = (x.unwrap_or_default(), y.unwrap_or_default());
            match outline::compare(&x, &y) {
                Match::Same | Match::Rotated => OutlineMatch::Same,
                Match::Different { ours, theirs } => match outlines {
                    Outlines::Coverage => match outline::coverage_equal(&x, &y) {
                        Ok(c) if c.equal => OutlineMatch::CoverageEqual,
                        Ok(c) => OutlineMatch::Different(diff(
                            key,
                            format!("{ours}; coverage differs by up to {:.2} per pixel, {:.2}% of the ink", c.max_pixel, c.share * 100.0),
                            theirs,
                        )),
                        Err(e) => OutlineMatch::Different(diff(key, format!("{ours}; coverage not compared: {e}"), theirs)),
                    },
                    Outlines::Exact => OutlineMatch::Different(diff(key, ours, theirs)),
                },
            }
        }
        (x, y) => OutlineMatch::Different(diff(key, x.err().unwrap_or_else(|| "drawn".into()), y.err().unwrap_or_else(|| "drawn".into()))),
    }
}
