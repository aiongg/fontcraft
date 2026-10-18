//! Table structure and flag checks (M12.51): which tables a compiled font carries, the formats of
//! `post` and `cmap`, the integrity of `glyf` and `loca`, the OS/2 and `head` flags and fields,
//! the CFF table's strings, dictionary keys and charstrings, the BASE, meta and STAT tables, and
//! `check.outlines.pointsOutsideBBox`. Runners take a [`crate::BinaryCtx`], the thresholds and a
//! [`crate::Sink`]; their registry entries are in `registry/tables.rs`.
//!
//! The rules follow FontBakery 1.1.0 (https://github.com/fonttools/fontbakery, Apache License
//! 2.0), single-face tests 7, 9, 10, 11, 14, 18, 25 and 28 of comparefamily in AFDKO 5.0.1
//! (https://github.com/adobe-type-tools/afdko, Apache License 2.0, Copyright 2015 Adobe) and the
//! validator notes in AFDKO's `MSFontValidatorIssues.md`. Only the rules are taken; the code is
//! written here. See `NOTICE`. Each check's row in `spec/inventory/21-qa-checks.toml` says where
//! this port departs from its sources.

use std::collections::{BTreeMap, BTreeSet};

use fontcraft_geom::{Point, Rect};
use read_fonts::ps::cff::CffFontRef;
use read_fonts::ps::cff::dict::{Entry, entries};
use read_fonts::ps::cff::index::Index;
use read_fonts::ps::string::Sid;
use read_fonts::tables::cmap::CmapSubtable;
use read_fonts::tables::glyf::Glyph as TtGlyph;
use read_fonts::tables::layout::{FeatureList, ScriptList};
use read_fonts::tables::loca::LocaGlyph;
use read_fonts::tables::stat::AxisValue;
use read_fonts::types::{GlyphId, Tag};
use read_fonts::{FontRef, ReadError, TableProvider};
use skrifa::MetadataProvider;
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::{LocationRef, Size};

use crate::data::{code_pages, tags, unicode_ranges, vendors};
use crate::profile::ProjectValue;
use crate::{BinaryCtx, Hit, Location, Params, Sink, Threshold, num};

/// `sfntVersion` of a font with CFF or CFF2 outlines.
const OTTO: u32 = 0x4F54_544F;
/// `sfntVersion` of a font with TrueType outlines.
const TRUETYPE: u32 = 0x0001_0000;
/// Most cmap mappings one check reads: one per Unicode code point.
const MAX_CMAP_MAPPINGS: usize = 0x11_0000;
/// Most cmap encoding records read.
const MAX_CMAP_RECORDS: usize = 256;
/// Most script, language-system and feature records walked in one layout table.
const MAX_LAYOUT_RECORDS: usize = 1_000_000;
/// Most top DICT entries read from a CFF table.
const MAX_DICT_ENTRIES: usize = 1_024;
/// Most top DICTs read from a CFF table (a font in an sfnt has one).
const MAX_TOP_DICTS: usize = 16;
/// Deepest subroutine nesting the Type 2 charstring format allows.
const MAX_CALL_DEPTH: usize = 10;
/// Operand stack of a CFF2 charstring; a CFF charstring allows 48.
const MAX_CHARSTRING_STACK: usize = 513;
/// Charstring bytes interpreted over all glyphs of one font before the CFF scans give up.
const MAX_CHARSTRING_OPS: usize = 20_000_000;
/// Item variation data entries read for CFF2 `blend` operand counts.
const MAX_VARIATION_DATA: usize = 1_024;
/// Longest charstring old interpreters accept (comparefamily test 18).
const MAX_CHARSTRING_BYTES: usize = 32_767;
/// Points read from `glyf` over all glyphs before the bounds check gives up.
const MAX_OUTLINE_POINTS: usize = 10_000_000;
/// Largest coordinate magnitude the bounds check accepts.
const MAX_COORDINATE: f64 = 32_766.0;
/// STAT axis values read.
const MAX_AXIS_VALUES: usize = 65_535;
/// Code points one message lists.
const MAX_LISTED: usize = 20;
/// A font with more code points than this in the CJK blocks is a CJK font (FontBakery's rule).
const CJK_MIN_CODE_POINTS: usize = 150;
/// Bytes of a meta entry shown in a message.
const MAX_META_TEXT: usize = 200;

/// The CJK blocks FontBakery counts (its `CJK_UNICODE_RANGES`).
const CJK_RANGES: &[(u32, u32)] = &[
    (0x1100, 0x11FF),
    (0x3040, 0x309F),
    (0x30A0, 0x30FF),
    (0x31F0, 0x31FF),
    (0x3100, 0x312F),
    (0x31A0, 0x31BF),
    (0x3130, 0x318F),
    (0x3200, 0x32FF),
    (0x3300, 0x33FF),
    (0xAC00, 0xD7AF),
    (0x4E00, 0x9FFF),
    (0x2E80, 0x2EFF),
    (0x2F00, 0x2FDF),
    (0x2FF0, 0x2FFF),
    (0x3400, 0x4DBF),
    (0x20000, 0x2A6DF),
    (0x3190, 0x319F),
    (0x31C0, 0x31EF),
    (0xF900, 0xFAFF),
    (0x2F800, 0x2FA1F),
];

/// `check.tables.codePageRanges`: whether every bit is also compared with the coverage rules.
pub(crate) const STRICT_BITS: Threshold = Threshold {
    name: "strict",
    default: 0.0,
    min: 0.0,
    max: 1.0,
    doc: "1 also compares each code page and Unicode range bit with what the cmap covers; 0 only reports empty code page fields.",
};

/// `check.tables.lowestRecPpem`: the smallest recommended size that is not reported.
pub(crate) const MIN_PPEM: Threshold =
    Threshold { name: "minimum", default: 6.0, min: 1.0, max: 255.0, doc: "head.lowestRecPPEM below this many pixels per em is reported." };

/// `check.tables.baseTable`: how far the ideographic baseline may be from centring the cap height.
pub(crate) const BASE_TOLERANCE: Threshold = Threshold {
    name: "tolerance",
    default: 0.0,
    min: 0.0,
    max: 1_000.0,
    doc: "Units the ideographic baseline may differ from the value that centres the cap height in the em box.",
};

// ---------------------------------------------------------------------------------------------
// Shared readers
// ---------------------------------------------------------------------------------------------

fn data<'a>(ctx: &BinaryCtx<'a>, tag: &[u8; 4]) -> Option<&'a [u8]> {
    ctx.font.table_data(Tag::new(tag)).map(|d| d.as_bytes())
}

fn has(ctx: &BinaryCtx<'_>, tag: &[u8; 4]) -> bool {
    data(ctx, tag).is_some()
}

/// The tags of the font's table directory, in directory order.
fn table_tags(ctx: &BinaryCtx<'_>) -> Vec<[u8; 4]> {
    ctx.font.table_directory.table_records().iter().map(|r| r.tag().to_be_bytes()).collect()
}

fn text(tag: [u8; 4]) -> String {
    tags::tag_text(tag)
}

/// The location of a table.
fn table_at(tag: [u8; 4]) -> Location {
    Location { table: Some(text(tag)), ..Location::default() }
}

/// The location of one field of one table, with the source field behind it when the font was
/// compiled from a source in this run.
pub(crate) fn field(ctx: &BinaryCtx<'_>, table: &str, name: &str) -> Location {
    let key = format!("{}.{name}", table.trim_end());
    Location {
        table: Some(table.to_string()),
        field: Some(name.to_string()),
        source_field: ctx.source.and_then(|s| s.field(&key)).map(str::to_string),
        ..Location::default()
    }
}

/// Names glyphs for findings: the source glyph, else the font's own name for it.
struct Namer<'a> {
    ctx: &'a BinaryCtx<'a>,
    names: skrifa::GlyphNames<'a>,
}

impl<'a> Namer<'a> {
    fn new(ctx: &'a BinaryCtx<'a>) -> Self {
        Self { ctx, names: ctx.font.glyph_names() }
    }

    fn name(&self, gid: u32) -> String {
        if let Some(name) = self.ctx.source.and_then(|s| s.glyph(gid)) {
            return name.to_string();
        }
        self.names.get(GlyphId::new(gid)).map(|n| n.as_str().to_string()).unwrap_or_else(|| format!("glyph {gid}"))
    }
}

fn glyph_at(name: String, table: &str) -> Location {
    Location { glyph: name, table: Some(table.to_string()), ..Location::default() }
}

/// A name string by ID, preferring Windows English records.
fn name_string(font: &FontRef<'_>, id: u16) -> Option<String> {
    let name = font.name().ok()?;
    let strings = name.string_data();
    let mut records: Vec<_> = name.name_record().iter().filter(|r| r.name_id().to_u16() == id).collect();
    records.sort_by_key(|r| (r.platform_id() != 3, r.language_id() != 0x409));
    records.into_iter().find_map(|r| r.string(strings).ok().map(|s| s.to_string()))
}

/// The code points the font's best Unicode cmap subtable maps to a glyph other than `.notdef`.
fn code_points(font: &FontRef<'_>) -> BTreeSet<u32> {
    font.charmap().mappings().take(MAX_CMAP_MAPPINGS).filter(|(_, g)| g.to_u32() != 0).map(|(cp, _)| cp).collect()
}

fn is_cjk(points: &BTreeSet<u32>) -> bool {
    points.iter().filter(|cp| CJK_RANGES.iter().any(|(a, b)| (*a..=*b).contains(*cp))).nth(CJK_MIN_CODE_POINTS).is_some()
}

fn code_point_list(points: impl IntoIterator<Item = u32>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut rest = 0usize;
    for cp in points {
        if out.len() < MAX_LISTED {
            out.push(format!("U+{cp:04X}"));
        } else {
            rest = rest.saturating_add(1);
        }
    }
    if rest > 0 {
        out.push(format!("and {rest} more"));
    }
    out.join(", ")
}

/// A project value as a binary check reads it (`audit.projectValues`). The checks that need one
/// name it in a `ProjectValue` precondition and skip with [`value_not_passed`] when it is unset
/// or the run passes no values.
fn project_value<'a>(ctx: &BinaryCtx<'a>, name: &str) -> Option<&'a ProjectValue> {
    ctx.value(name)
}

fn value_not_passed(name: &str) -> String {
    format!("the project value `{name}` is not set")
}

/// A tag as a project value spells it, padded with spaces to four bytes.
pub(crate) fn padded_tag(s: &str) -> Option<[u8; 4]> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes.len() > 4 {
        return None;
    }
    let mut tag = [b' '; 4];
    for (slot, b) in tag.iter_mut().zip(bytes) {
        *slot = *b;
    }
    Some(tag)
}

// ---------------------------------------------------------------------------------------------
// Which tables the font carries
// ---------------------------------------------------------------------------------------------

const REQUIRED: [&[u8; 4]; 8] = [b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post"];

/// The optional tables FontBakery lists as information.
const OPTIONAL: [&[u8; 4]; 20] = [
    b"cvt ", b"fpgm", b"loca", b"prep", b"VORG", b"EBDT", b"EBLC", b"EBSC", b"BASE", b"GPOS", b"GSUB", b"JSTF", b"gasp", b"hdmx", b"LTSH", b"PCLT",
    b"VDMX", b"vhea", b"vmtx", b"kern",
];

/// `check.tables.requiredTables` (FontBakery `required_tables`).
pub(crate) fn required_tables(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let optional: Vec<String> = OPTIONAL.iter().filter(|t| has(ctx, t)).map(|t| text(**t)).collect();
    if !optional.is_empty() {
        sink.push(Hit::new("optional-tables", format!("The font has these optional tables: {}.", optional.join(", "))));
    }
    let mut missing: Vec<[u8; 4]> = REQUIRED.iter().filter(|t| !has(ctx, t)).map(|t| **t).collect();
    let variable = has(ctx, b"fvar");
    if variable && !has(ctx, b"STAT") {
        missing.push(*b"STAT");
    }
    let version = ctx.font.table_directory.sfnt_version();
    if version == OTTO && !has(ctx, b"CFF ") && !has(ctx, b"CFF2") {
        missing.push(if variable { *b"CFF2" } else { *b"CFF " });
    } else if version == TRUETYPE && !has(ctx, b"glyf") {
        missing.push(*b"glyf");
    }
    for tag in missing {
        let why = match &tag {
            b"STAT" => " (variable fonts need it)",
            b"glyf" => " (the file says it has TrueType outlines)",
            b"CFF " | b"CFF2" => " (the file says it has CFF outlines)",
            _ => "",
        };
        sink.push(Hit::new("required-tables", format!("The required table '{}' is missing{why}.", text(tag))).place(table_at(tag)));
    }
}

/// `check.tables.recommendedTrueTypeTables`: tables a TrueType rasteriser uses. CFF fonts are
/// never asked for them (`MSFontValidatorIssues.md` 1.a).
pub(crate) fn recommended_truetype_tables(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    if !has(ctx, b"glyf") {
        return;
    }
    for (tag, what) in [
        (b"gasp", "the sizes at which to grid-fit and smooth"),
        (b"kern", "legacy kerning for applications that do not read GPOS"),
        (b"hdmx", "precomputed advance widths per size"),
        (b"VDMX", "precomputed vertical extents per size"),
    ] {
        if !has(ctx, tag) {
            let msg = format!("The TrueType font has no '{}' table ({what}).", text(*tag));
            sink.push(Hit::new("missing-recommended-table", msg).place(table_at(*tag)));
        }
    }
}

/// Tables editors and hinting tools keep for themselves (FontBakery `unwanted_tables` and
/// `vtt_volt_data`).
const UNWANTED: [(&[u8; 4], &str); 15] = [
    (b"FFTM", "an editor's timestamps"),
    (b"TTFA", "the automatic TrueType hinter's settings"),
    (b"TSI0", "the sources of a TrueType hinting tool"),
    (b"TSI1", "the sources of a TrueType hinting tool"),
    (b"TSI2", "the sources of a TrueType hinting tool"),
    (b"TSI3", "the sources of a TrueType hinting tool"),
    (b"TSI5", "the sources of a TrueType hinting tool"),
    (b"TSIC", "the sources of a TrueType hinting tool"),
    (b"TSIV", "the sources of a layout tool"),
    (b"TSIP", "the sources of a layout tool"),
    (b"TSIS", "the sources of a layout tool"),
    (b"TSID", "the sources of a layout tool"),
    (b"TSIJ", "the sources of a layout tool"),
    (b"TSIB", "the sources of a layout tool"),
    (b"Debg", "a compiler's debugging data"),
];

/// Feature tags and (script, language system) tags beginning with `zz` in one layout table.
fn zz_tags(scripts: Result<ScriptList<'_>, ReadError>, features: Result<FeatureList<'_>, ReadError>) -> (BTreeSet<[u8; 4]>, BTreeSet<[u8; 4]>) {
    let zz = |t: [u8; 4]| t.starts_with(b"zz");
    let mut feats = BTreeSet::new();
    let mut systems = BTreeSet::new();
    let mut seen = 0usize;
    if let Ok(features) = features {
        for rec in features.feature_records().iter().take(MAX_LAYOUT_RECORDS) {
            seen = seen.saturating_add(1);
            let tag = rec.feature_tag().to_be_bytes();
            if zz(tag) {
                feats.insert(tag);
            }
        }
    }
    if let Ok(scripts) = scripts {
        'walk: for sr in scripts.script_records() {
            let Ok(script) = sr.script(scripts.offset_data()) else { continue };
            for lr in script.lang_sys_records() {
                seen = seen.saturating_add(1);
                if seen > MAX_LAYOUT_RECORDS {
                    break 'walk;
                }
                let tag = lr.lang_sys_tag().to_be_bytes();
                if zz(tag) {
                    systems.insert(tag);
                }
            }
        }
    }
    (feats, systems)
}

/// `check.tables.unwantedTables` (FontBakery `unwanted_tables`, `vtt_volt_data`).
pub(crate) fn unwanted_tables(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    for (tag, what) in UNWANTED {
        if has(ctx, tag) {
            let msg = format!("The table '{}' holds {what} and should not ship.", text(*tag));
            sink.push(Hit::new("unwanted-table", msg).place(table_at(*tag)));
        }
    }
    let layouts = [
        ("GSUB", ctx.font.gsub().map(|t| (t.script_list(), t.feature_list()))),
        ("GPOS", ctx.font.gpos().map(|t| (t.script_list(), t.feature_list()))),
    ];
    for (table, lists) in layouts {
        let Ok((scripts, features)) = lists else { continue };
        let (feats, systems) = zz_tags(scripts, features);
        for tag in feats {
            let msg = format!("The {table} feature '{}' is a layout tool's working feature and should not ship.", text(tag));
            let place = Location { table: Some(table.to_string()), feature: Some(text(tag)), ..Location::default() };
            sink.push(Hit::new("zz-feature", msg).place(place).without_fix());
        }
        for tag in systems {
            let msg = format!("The {table} language system '{}' is a layout tool's working entry and should not ship.", text(tag));
            let place = Location { table: Some(table.to_string()), language: Some(text(tag)), ..Location::default() };
            sink.push(Hit::new("zz-language-system", msg).place(place).without_fix());
        }
    }
}

/// Apple-only tables (FontBakery `opentype/unwanted_aat_tables`).
const AAT_TABLES: [&[u8; 4]; 22] = [
    b"acnt", b"ankr", b"bdat", b"bhed", b"bloc", b"bmap", b"bsln", b"fdsc", b"feat", b"fond", b"gcid", b"just", b"kerx", b"lcar", b"ltag", b"mort",
    b"morx", b"opbd", b"prop", b"trak", b"xref", b"Zaph",
];

/// `check.tables.unwantedAatTables` (FontBakery `opentype/unwanted_aat_tables`).
pub(crate) fn unwanted_aat_tables(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    for tag in AAT_TABLES {
        if has(ctx, tag) {
            let msg = format!("The table '{}' belongs to Apple's layout model, not to OpenType.", text(*tag));
            sink.push(Hit::new("unwanted-aat-table", msg).place(table_at(*tag)));
        }
    }
}

/// `check.tables.unsupportedTables` (FontBakery `adobefonts/unsupported_tables`, with the
/// allow-list taken from the project value `allowedTables`).
pub(crate) fn unsupported_tables(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    match project_value(ctx, "allowedTables") {
        Some(ProjectValue::List(allowed)) => {
            let allowed: Vec<[u8; 4]> = allowed.iter().filter_map(|s| padded_tag(s)).collect();
            report_unsupported(&table_tags(ctx), &allowed, sink);
        }
        _ => sink.skip(value_not_passed("allowedTables")),
    }
}

/// Reports every table of `present` that `allowed` does not list.
fn report_unsupported(present: &[[u8; 4]], allowed: &[[u8; 4]], sink: &mut Sink) {
    for tag in present {
        if !allowed.contains(tag) {
            let msg = format!("The table '{}' is not on the project's list of allowed tables.", text(*tag));
            sink.push(Hit::new("unsupported-table", msg).place(table_at(*tag)));
        }
    }
}

/// `check.tables.dsig` (FontBakery `opentype/dsig`).
pub(crate) fn dsig(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    if has(ctx, b"DSIG") {
        let msg = "The font has a digital signature table (DSIG). Only old office software needed one; current advice is to remove it.";
        sink.push(Hit::new("found-dsig", msg).place(table_at(*b"DSIG")));
    }
}

// ---------------------------------------------------------------------------------------------
// post, glyf and loca
// ---------------------------------------------------------------------------------------------

/// `check.tables.postVersion` (FontBakery `opentype/post_table_version`). As there, every font
/// without a `CFF ` table counts as one that loses its glyph names with format 3, CFF2 fonts
/// included.
pub(crate) fn post_version(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(version) = data(ctx, b"post").and_then(|d| d.get(..4)).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(u32::from_be_bytes) else {
        sink.skip("the post table is too short to hold a version");
        return;
    };
    let cff = has(ctx, b"CFF ");
    let place = || field(ctx, "post", "version");
    let (shown, value) = match version {
        0x0001_0000 => ("1", 1.0),
        0x0002_0000 => ("2", 2.0),
        0x0002_5000 => ("2.5", 2.5),
        0x0003_0000 => ("3", 3.0),
        0x0004_0000 => ("4", 4.0),
        _ => ("unknown", f64::from(version) / 65_536.0),
    };
    let hit = if cff && version != 0x0003_0000 {
        Hit::new("cff-not-format-3", format!("The post table has format {shown}; a font with CFF outlines must use format 3."))
    } else if !cff && version == 0x0003_0000 {
        Hit::new("format-3", "The post table has format 3, which stores no glyph names: text in documents made with the font may not be searchable.")
    } else if version == 0x0002_5000 {
        Hit::new("format-2-5", "The post table has format 2.5, which OpenType 1.3 deprecated.")
    } else if version == 0x0004_0000 {
        Hit::new("format-4", "The post table has format 4, which is not needed and should not be used.")
    } else {
        return;
    };
    sink.push(hit.data("version", value).place(place()));
}

/// `check.tables.glyfIntegrity` (FontBakery `opentype/glyf_unused_data`,
/// `opentype/loca/maxp_num_glyphs`). Empty glyphs at the end never count as missing data
/// (`MSFontValidatorIssues.md` 5.a): only a glyph with a non-empty range can reach past the table.
pub(crate) fn glyf_integrity(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(glyf) = data(ctx, b"glyf") else { return };
    let Some(loca) = data(ctx, b"loca") else {
        sink.push(Hit::new("loca-too-short", "The font has a glyf table but no loca table to find its glyphs.").place(table_at(*b"loca")));
        return;
    };
    let (Ok(head), Ok(maxp)) = (ctx.font.head(), ctx.font.maxp()) else {
        sink.skip("the head or maxp table cannot be read");
        return;
    };
    let long = head.index_to_loc_format() == 1;
    let width = if long { 4 } else { 2 };
    let num_glyphs = usize::from(maxp.num_glyphs());
    let entries = loca.len() / width;
    let wanted = num_glyphs.saturating_add(1);
    if entries < wanted {
        let msg = format!("The loca table has {entries} entries; maxp.numGlyphs of {num_glyphs} needs {wanted}.");
        sink.push(Hit::new("loca-too-short", msg).data("entries", entries as f64).data("expected", wanted as f64).place(table_at(*b"loca")));
    }
    let offset = |i: usize| -> Option<usize> {
        let start = i.checked_mul(width)?;
        let b = loca.get(start..start.checked_add(width)?)?;
        Some(match b {
            [a, b] => usize::from(u16::from_be_bytes([*a, *b])).saturating_mul(2),
            [a, b, c, d] => usize::try_from(u32::from_be_bytes([*a, *b, *c, *d])).unwrap_or(usize::MAX),
            _ => return None,
        })
    };
    let count = entries.min(wanted);
    let namer = Namer::new(ctx);
    let mut decreasing = None;
    let mut beyond: Option<(usize, usize)> = None;
    let mut missing = 0usize;
    for i in 0..count.saturating_sub(1) {
        let (Some(start), Some(end)) = (offset(i), offset(i + 1)) else { break };
        if end < start && decreasing.is_none() {
            decreasing = Some(i);
        }
        if start < end && end > glyf.len() {
            missing = missing.saturating_add(1);
            beyond.get_or_insert((i, end));
        }
    }
    if let Some(i) = decreasing {
        let msg = format!("The loca offsets go backwards at glyph {}: its end lies before its start.", namer.name(i as u32));
        sink.push(Hit::new("offsets-decrease", msg).data("glyphId", i as f64).place(glyph_at(namer.name(i as u32), "loca")));
    }
    if let Some((i, end)) = beyond {
        let msg = format!(
            "The loca table points past the end of the glyf table ({} bytes) for {missing} glyph(s), first {}, which ends at byte {end}.",
            glyf.len(),
            namer.name(i as u32)
        );
        sink.push(Hit::new("missing-data", msg).data("glyphs", missing as f64).place(glyph_at(namer.name(i as u32), "glyf")));
    }
    if count == wanted
        && let Some(last) = offset(num_glyphs)
        && glyf.len() > last.saturating_add(3)
    {
        let unused = glyf.len().saturating_sub(last);
        let msg = format!("The glyf table has {unused} bytes after the last glyph that no loca entry reaches.");
        sink.push(Hit::new("unreachable-data", msg).data("bytes", unused as f64).place(table_at(*b"glyf")));
    }
}

// ---------------------------------------------------------------------------------------------
// cmap
// ---------------------------------------------------------------------------------------------

/// One cmap encoding record: (platform, encoding, subtable).
type CmapRecord<'a> = (u16, u16, Result<CmapSubtable<'a>, ReadError>);

/// The readable encoding records of the cmap table.
fn cmap_subtables<'a>(ctx: &BinaryCtx<'a>) -> Result<Vec<CmapRecord<'a>>, String> {
    let cmap = ctx.font.cmap().map_err(|e| format!("the cmap table cannot be read: {e}"))?;
    Ok(cmap
        .encoding_records()
        .iter()
        .take(MAX_CMAP_RECORDS)
        .map(|r| (r.platform_id() as u16, r.encoding_id(), r.subtable(cmap.offset_data())))
        .collect())
}

fn mapped(sub: &CmapSubtable<'_>) -> BTreeSet<u32> {
    sub.iter().take(MAX_CMAP_MAPPINGS).filter(|(_, g)| g.to_u32() != 0).map(|(cp, _)| cp).collect()
}

/// `check.tables.cmapFormat12` (FontBakery `cmap/format_12`). A format 12 subtable earns its
/// place by mapping a code point beyond the Basic Multilingual Plane; FontBakery's test accepts
/// any code point above U+0FFF.
pub(crate) fn cmap_format_12(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let subtables = match cmap_subtables(ctx) {
        Ok(s) => s,
        Err(e) => {
            sink.skip(e);
            return;
        }
    };
    let twelves: Vec<_> =
        subtables.iter().filter_map(|(p, e, s)| s.as_ref().ok().filter(|s| matches!(s, CmapSubtable::Format12(_))).map(|s| (*p, *e, s))).collect();
    if twelves.is_empty() {
        return;
    }
    let place = || table_at(*b"cmap");
    let Some(four) = subtables.iter().find_map(|(_, _, s)| s.as_ref().ok().filter(|s| matches!(s, CmapSubtable::Format4(_)))) else {
        sink.push(
            Hit::new("no-cmap-4", "The cmap table has a format 12 subtable but no format 4 one; older software reads only format 4.").place(place()),
        );
        return;
    };
    let bmp = mapped(four);
    for (platform, encoding, sub) in twelves {
        let full = mapped(sub);
        let at = || Location { platform: Some(platform), ..place() };
        if !full.iter().any(|cp| *cp > 0xFFFF) {
            let msg = format!(
                "The format 12 subtable ({platform}, {encoding}) maps no code point beyond the Basic Multilingual Plane, so it is not needed."
            );
            sink.push(Hit::new("pointless-format-12", msg).place(at()));
        }
        let left_out: Vec<u32> = bmp.difference(&full).copied().collect();
        if !left_out.is_empty() {
            let msg = format!(
                "The format 12 subtable ({platform}, {encoding}) leaves out {} code point(s) that the format 4 subtable maps: {}.",
                left_out.len(),
                code_point_list(left_out.iter().copied())
            );
            sink.push(Hit::new("unmapped-from-4", msg).data("count", left_out.len() as f64).place(at()));
        }
    }
}

/// The (format, platform, encoding) triples FontBakery's `notofonts/cmap/unexpected_subtables`
/// accepts: Mac Roman formats 0 and 6, Windows formats 4 and 12, Unicode formats 4, 12 and 14.
const EXPECTED_SUBTABLES: &[(u16, u16, u16)] = &[(0, 1, 0), (6, 1, 0), (4, 3, 1), (12, 3, 10), (14, 0, 5), (4, 0, 3), (12, 0, 4)];
/// The Mac placeholder subtables CJK fonts may also carry (Japanese, both Chinese, Korean).
const EXPECTED_CJK_SUBTABLES: &[(u16, u16, u16)] = &[(6, 1, 1), (6, 1, 2), (6, 1, 3), (6, 1, 25)];

/// `check.tables.cmapSubtables` (FontBakery `notofonts/cmap/unexpected_subtables`).
pub(crate) fn cmap_subtables_check(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let subtables = match cmap_subtables(ctx) {
        Ok(s) => s,
        Err(e) => {
            sink.skip(e);
            return;
        }
    };
    let cjk = is_cjk(&code_points(&ctx.font));
    for (platform, encoding, sub) in subtables {
        let place = Location { platform: Some(platform), ..table_at(*b"cmap") };
        let Ok(sub) = sub else {
            let msg = format!("The cmap subtable for platform {platform}, encoding {encoding} cannot be read.");
            sink.push(Hit::new("unreadable-subtable", msg).place(place));
            continue;
        };
        let key = (sub.format(), platform, encoding);
        if EXPECTED_SUBTABLES.contains(&key) || (cjk && EXPECTED_CJK_SUBTABLES.contains(&key)) {
            continue;
        }
        let msg = format!("The cmap table has a format {} subtable for platform {platform}, encoding {encoding}, which a font does not need.", key.0);
        sink.push(Hit::new("unexpected-subtable", msg).data("format", f64::from(key.0)).data("encoding", f64::from(encoding)).place(place));
    }
}

// ---------------------------------------------------------------------------------------------
// OS/2 and head
// ---------------------------------------------------------------------------------------------

/// The style of a compiled font for checks that judge its style bits: italic when name ID 2 or 17
/// contains "Italic"; bold when name ID 2 begins with "Bold", or, in a variable font, when the
/// default weight is 700. `None` without a name ID 2.
struct Style {
    name: String,
    bold: bool,
    italic: bool,
}

fn style(font: &FontRef<'_>) -> Option<Style> {
    let subfamily = name_string(font, 2)?;
    let typographic = name_string(font, 17);
    let italic = subfamily.contains("Italic") || typographic.as_deref().is_some_and(|t| t.contains("Italic"));
    let bold = subfamily.starts_with("Bold") || default_weight(font).is_some_and(|w| w == 700.0);
    Some(Style { name: typographic.unwrap_or(subfamily), bold, italic })
}

/// The default of the `wght` axis of a variable font.
fn default_weight(font: &FontRef<'_>) -> Option<f64> {
    let fvar = font.fvar().ok()?;
    let axes = fvar.axes().ok()?;
    axes.iter().find(|a| a.axis_tag() == Tag::new(b"wght")).map(|a| a.default_value().to_f64())
}

/// `check.tables.fsSelection` (FontBakery `opentype/fsselection`, `opentype/mac_style`;
/// comparefamily test 14). The style comes from the names, not the file name
/// (`docs/spec/21-qa.md` § 10, question 9).
pub(crate) fn fs_selection(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (Ok(os2), Ok(head)) = (ctx.font.os2(), ctx.font.head()) else {
        sink.skip("the OS/2 or head table cannot be read");
        return;
    };
    let Some(style) = style(&ctx.font) else {
        sink.skip("the font has no subfamily name (name ID 2) to take its style from");
        return;
    };
    let fs = os2.fs_selection().bits();
    let mac = head.mac_style().bits();
    let (bold, italic, regular) = (fs & 0x20 != 0, fs & 0x01 != 0, fs & 0x40 != 0);
    let expected_regular = !style.bold && !style.italic;
    let on = |b: bool| if b { "set" } else { "clear" };
    for (code, seen, expected, label) in [
        ("bad-bold", bold, style.bold, "bold (bit 5)"),
        ("bad-italic", italic, style.italic, "italic (bit 0)"),
        ("bad-regular", regular, expected_regular, "regular (bit 6)"),
    ] {
        if seen != expected {
            let msg = format!("The fsSelection {label} flag is {} but the style '{}' needs it {}.", on(seen), style.name, on(expected));
            sink.push(Hit::new(code, msg).place(field(ctx, "OS/2", "fsSelection")));
        }
    }
    for (code, fs_bit, mac_bit, label) in [("macstyle-bold", bold, mac & 0x01 != 0, "bold"), ("macstyle-italic", italic, mac & 0x02 != 0, "italic")] {
        if fs_bit != mac_bit {
            let msg = format!("The {label} bit is {} in OS/2.fsSelection but {} in head.macStyle.", on(fs_bit), on(mac_bit));
            sink.push(Hit::new(code, msg).place(field(ctx, "head", "macStyle")));
        }
    }
}

/// `check.tables.vendorId` (FontBakery `opentype/vendor_id`, `googlefonts/vendor_id`).
pub(crate) fn vendor_id(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Ok(os2) = ctx.font.os2() else {
        sink.skip("the OS/2 table cannot be read");
        return;
    };
    let configured = match project_value(ctx, "vendorId") {
        Some(ProjectValue::Text(s)) => padded_tag(s),
        _ => None,
    };
    report_vendor_id(ctx, os2.ach_vend_id().to_be_bytes(), configured, sink);
}

/// Compares the stored vendor ID with the configured one, or, without one, with the registry.
fn report_vendor_id(ctx: &BinaryCtx<'_>, found: [u8; 4], configured: Option<[u8; 4]>, sink: &mut Sink) {
    let place = || field(ctx, "OS/2", "achVendID");
    let shown = text(found);
    let normal = found.map(|b| if b == 0 { b' ' } else { b });
    if let Some(wanted) = configured {
        if normal != wanted {
            let msg = format!("OS/2.achVendID is '{shown}' but the project's vendor ID is '{}'.", text(wanted));
            sink.push(Hit::new("bad-vendor-id", msg).place(place()));
        }
        return;
    }
    let hit = if normal == *b"    " {
        Hit::new("not-set", "OS/2.achVendID is empty. Set it to the foundry's registered four-character vendor ID.")
    } else if vendors::PLACEHOLDERS.contains(&found) {
        Hit::new("placeholder", format!("OS/2.achVendID is '{shown}', a value tools write when nobody chose one."))
    } else if !vendors::is_registered(found) {
        Hit::new("unregistered", format!("OS/2.achVendID '{shown}' is not in the vendor registry. If it was registered recently, ignore this."))
    } else {
        return;
    };
    sink.push(hit.place(place()));
}

/// `check.tables.codePageRanges` (FontBakery `opentype/code_pages`; with `strict`, also
/// `notofonts/unicode_range_bits` and comparefamily test 28). The OEM and Symbol code page bits
/// are never earned by coverage, so they are not compared.
pub(crate) fn code_page_ranges(ctx: &BinaryCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    let Ok(os2) = ctx.font.os2() else {
        let msg = if has(ctx, b"OS/2") {
            "The OS/2 table cannot be read, so the font declares no code pages."
        } else {
            "The font has no OS/2 table, so it declares no code pages."
        };
        sink.push(Hit::new("no-os2-table", msg).place(table_at(*b"OS/2")));
        return;
    };
    let (r1, r2) = (os2.ul_code_page_range_1().unwrap_or(0), os2.ul_code_page_range_2().unwrap_or(0));
    if r1 == 0 && r2 == 0 {
        let msg = "OS/2.ulCodePageRange1 and 2 are both zero: some applications then treat the font as covering no character set.";
        sink.push(Hit::new("no-code-pages", msg).place(field(ctx, "OS/2", "ulCodePageRange1")));
    }
    if params.get(&STRICT_BITS) < 0.5 {
        return;
    }
    let points = code_points(&ctx.font);
    let stored_pages = code_pages::from_fields(r1, r2);
    let earned_pages = code_pages::bits_for(|cp| points.contains(&cp));
    for (bit, number, name) in code_pages::CODE_PAGES {
        if matches!(bit, 30 | 31) {
            continue;
        }
        let (stored, earned) = ((stored_pages >> bit) & 1 == 1, (earned_pages >> bit) & 1 == 1);
        if stored != earned {
            let page = if *number == 0 { (*name).to_string() } else { format!("{number}, {name}") };
            let msg = if stored {
                format!("Code page bit {bit} ({page}) is set, but the cmap does not cover that code page.")
            } else {
                format!("Code page bit {bit} ({page}) is clear, but the cmap covers that code page.")
            };
            sink.push(Hit::new("bad-code-page-bit", msg).data("bit", f64::from(*bit)).place(field(ctx, "OS/2", "ulCodePageRange")));
        }
    }
    let stored_ranges =
        unicode_ranges::from_fields(os2.ul_unicode_range_1(), os2.ul_unicode_range_2(), os2.ul_unicode_range_3(), os2.ul_unicode_range_4());
    let earned_ranges = unicode_ranges::bits_for(points.iter().copied());
    for bit in 0..unicode_ranges::BIT_COUNT {
        let (stored, earned) = ((stored_ranges >> bit) & 1 == 1, (earned_ranges >> bit) & 1 == 1);
        if stored != earned {
            let name = unicode_ranges::bit_name(bit).unwrap_or("reserved");
            let msg = if stored {
                format!("Unicode range bit {bit} ({name}) is set, but the cmap maps no character of it.")
            } else {
                format!("Unicode range bit {bit} ({name}) is clear, but the cmap maps characters of it.")
            };
            sink.push(Hit::new("bad-range-bit", msg).data("bit", f64::from(bit)).place(field(ctx, "OS/2", "ulUnicodeRange")));
        }
    }
    let full_cmap = cmap_subtables(ctx).is_ok_and(|s| s.iter().any(|(_, _, s)| matches!(s, Ok(CmapSubtable::Format12(_)))));
    if (stored_ranges >> unicode_ranges::NON_PLANE_0) & 1 == 1 && !full_cmap {
        let msg = "Unicode range bit 57 (beyond the Basic Multilingual Plane) is set, but the cmap has no format 12 subtable to map such characters.";
        sink.push(Hit::new("non-plane-0-without-format-12", msg).place(field(ctx, "OS/2", "ulUnicodeRange")));
    }
}

/// `check.tables.os2Version` (comparefamily family test 17).
pub(crate) fn os2_version(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Ok(os2) = ctx.font.os2() else {
        sink.skip("the OS/2 table cannot be read");
        return;
    };
    let version = os2.version();
    if version < 4 {
        let msg = format!(
            "The OS/2 table has version {version}; version 4 or later is needed for the USE_TYPO_METRICS, WWS and OBLIQUE bits of fsSelection."
        );
        sink.push(Hit::new("old-version", msg).data("version", f64::from(version)).place(field(ctx, "OS/2", "version")));
    }
}

/// The weight a style name implies, as FontBakery's `typenetwork/weightclass` reads it: the
/// style is name ID 17 when the font has a name ID 16, else name ID 2; "Semi", "Ultra" and
/// "Extra" join the following word; "Italic" and "Oblique" are dropped. The first known weight
/// word decides. Thin and ExtraLight accept two values.
fn expected_weight(font: &FontRef<'_>) -> Option<(String, Option<&'static [u16]>)> {
    let name = (if name_string(font, 16).is_some() { name_string(font, 17) } else { name_string(font, 2) })?;
    let mut s = name;
    for prefix in ["Semi ", "Ultra ", "Extra "] {
        s = s.replace(prefix, prefix.trim_end());
    }
    let weight = if s == "Italic" {
        "Regular".to_string()
    } else if let Some(rest) = s.strip_suffix("Italic").or_else(|| s.strip_suffix("Oblique")) {
        rest.trim_end().to_string()
    } else {
        s
    };
    const WEIGHTS: [(&str, &[u16]); 9] = [
        ("thin", &[100, 250]),
        ("extralight", &[200, 275]),
        ("light", &[300]),
        ("regular", &[400]),
        ("medium", &[500]),
        ("semibold", &[600]),
        ("bold", &[700]),
        ("extrabold", &[800]),
        ("black", &[900]),
    ];
    let lower = weight.to_lowercase();
    let words: Vec<&str> = lower.split(' ').collect();
    let values = WEIGHTS.iter().find(|(w, _)| words.contains(w)).map(|(_, v)| *v);
    Some((weight, values))
}

/// `check.tables.weightClassRange` (comparefamily tests 9 and 10; FontBakery
/// `typenetwork/weightclass`).
pub(crate) fn weight_class_range(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Ok(os2) = ctx.font.os2() else {
        sink.skip("the OS/2 table cannot be read");
        return;
    };
    let value = os2.us_weight_class();
    let variable = has(ctx, b"fvar");
    let place = || field(ctx, "OS/2", "usWeightClass");
    if !variable && has(ctx, b"CFF ") && value < 250 {
        let msg = format!("OS/2.usWeightClass is {value}; below 250, old Windows renderers draw CFF glyphs bolder than they are.");
        sink.push(Hit::new("cff-below-250", msg).data("actual", f64::from(value)).place(place()));
    }
    if os2.fs_selection().bits() & 0x20 != 0 && value < 500 {
        let msg = format!("The bold bit of fsSelection is set but OS/2.usWeightClass is {value}, below 500.");
        sink.push(Hit::new("bold-below-500", msg).data("actual", f64::from(value)).place(place()));
    }
    if variable {
        let expected = default_weight(&ctx.font).map_or(400, |w| w.trunc().clamp(0.0, 65_535.0) as u16);
        if value != expected {
            let msg = format!("OS/2.usWeightClass is {value}; a variable font's must equal its default weight, {expected}.");
            sink.push(Hit::new("bad-value", msg).data("actual", f64::from(value)).data("expected", f64::from(expected)).place(place()));
        }
        return;
    }
    let Some((weight, values)) = expected_weight(&ctx.font) else { return };
    match values {
        None => {
            let msg = format!("OS/2.usWeightClass is {value} and the weight name '{weight}' implies no known value.");
            sink.push(Hit::new("no-value", msg).data("actual", f64::from(value)).place(place()));
        }
        Some(list) if !list.contains(&value) => {
            let wanted = list.iter().map(u16::to_string).collect::<Vec<_>>().join(" or ");
            let msg = format!("OS/2.usWeightClass is {value}; the weight name '{weight}' calls for {wanted}.");
            let expected = list.first().copied().map_or(0.0, f64::from);
            sink.push(Hit::new("bad-value", msg).data("actual", f64::from(value)).data("expected", expected).place(place()));
        }
        Some(_) => {}
    }
}

/// What the embedding bits of fsType allow.
fn fs_type_meaning(v: u16) -> String {
    let mut parts = vec![match v & 0x000E {
        0 => "installable",
        2 => "restricted license",
        4 => "preview and print",
        8 => "editable",
        _ => "conflicting embedding bits",
    }];
    if v & 0x0100 != 0 {
        parts.push("no subsetting");
    }
    if v & 0x0200 != 0 {
        parts.push("bitmap embedding only");
    }
    if v & 0xFCF1 != 0 {
        parts.push("reserved bits set");
    }
    parts.join(", ")
}

/// `check.tables.fsType` (FontBakery `googlefonts/fstype`, `microsoft/fstype`; comparefamily
/// family test 14), against the project value `fsType`.
pub(crate) fn fs_type(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Ok(os2) = ctx.font.os2() else {
        sink.skip("the OS/2 table cannot be read");
        return;
    };
    match project_value(ctx, "fsType") {
        Some(ProjectValue::Number(n)) if n.is_finite() && (0.0..=65_535.0).contains(n) => report_fs_type(ctx, os2.fs_type(), *n as u16, sink),
        _ => sink.skip(value_not_passed("fsType")),
    }
}

fn report_fs_type(ctx: &BinaryCtx<'_>, found: u16, wanted: u16, sink: &mut Sink) {
    if found != wanted {
        let msg = format!(
            "OS/2.fsType is {found} ({}); the project's embedding setting is {wanted} ({}).",
            fs_type_meaning(found),
            fs_type_meaning(wanted)
        );
        sink.push(
            Hit::new("unexpected-value", msg)
                .data("actual", f64::from(found))
                .data("expected", f64::from(wanted))
                .place(field(ctx, "OS/2", "fsType")),
        );
    }
}

/// `check.tables.lowestRecPpem` (`MSFontValidatorIssues.md` 4.c): informational, since CFF fonts
/// set a low value on purpose.
pub(crate) fn lowest_rec_ppem(ctx: &BinaryCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    let Ok(head) = ctx.font.head() else {
        sink.skip("the head table cannot be read");
        return;
    };
    let ppem = head.lowest_rec_ppem();
    let minimum = params.get(&MIN_PPEM);
    if f64::from(ppem) < minimum {
        let note =
            if has(ctx, b"CFF ") || has(ctx, b"CFF2") { " For a CFF font a low value is usual: its hints do not decide legibility." } else { "" };
        let msg = format!("head.lowestRecPPEM is {ppem}, below {}.{note}", num(minimum));
        sink.push(Hit::new("small-lowest-ppem", msg).data("actual", f64::from(ppem)).place(field(ctx, "head", "lowestRecPPEM")));
    }
}

// ---------------------------------------------------------------------------------------------
// CFF
// ---------------------------------------------------------------------------------------------

/// The CFF or CFF2 table of the font.
fn cff_font<'a>(ctx: &BinaryCtx<'a>) -> Result<CffFontRef<'a>, String> {
    let upem = ctx.font.head().ok().map(|h| i32::from(h.units_per_em()));
    let bytes = data(ctx, b"CFF ").or_else(|| data(ctx, b"CFF2")).ok_or("the font has no CFF table")?;
    CffFontRef::new(bytes, 0, upem).map_err(|e| format!("the CFF table cannot be read: {e}"))
}

/// What walking one glyph's charstring found.
#[derive(Clone, Copy, Default)]
struct CharstringScan {
    /// Deepest subroutine nesting reached (the glyph's own charstring is depth 0).
    max_depth: usize,
    /// A subroutine calls itself, directly or through others.
    recursion: bool,
    /// `endchar` with four or more operands: an accented glyph built the Type 1 way.
    seac: bool,
    /// The deprecated `dotsection` operator.
    dotsection: bool,
}

/// How a walk of one program ended.
enum Flow {
    /// The program returned (or ran off its end): the caller continues.
    Return,
    /// The glyph ended (`endchar`, an error, recursion): nothing more is read.
    End,
    /// The font's operation budget ran out.
    Budget,
}

/// A Type 2 charstring reader that follows subroutine calls and records what [`CharstringScan`]
/// holds. It tracks the operand stack only as far as subroutine numbers, stem counts (for the
/// length of hint masks) and the operand count of `endchar` need.
struct Walker<'a> {
    cff2: bool,
    global: Index<'a>,
    local: Option<Index<'a>>,
    /// Regions per item variation data, for the operand count of CFF2's `blend`.
    regions: Vec<usize>,
    vs_index: usize,
    stack: Vec<f64>,
    stems: usize,
    chain: Vec<(bool, usize)>,
    ops: usize,
    scan: CharstringScan,
}

impl<'a> Walker<'a> {
    fn new(cff: &CffFontRef<'a>) -> Self {
        let regions = cff
            .var_store()
            .map(|vs| {
                vs.item_variation_data()
                    .iter()
                    .take(MAX_VARIATION_DATA)
                    .map(|d| d.and_then(Result::ok).map_or(0, |d| usize::from(d.region_index_count())))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            cff2: cff.version() == 2,
            global: cff.global_subrs().clone(),
            local: None,
            regions,
            vs_index: 0,
            stack: Vec::new(),
            stems: 0,
            chain: Vec::new(),
            ops: 0,
            scan: CharstringScan::default(),
        }
    }

    /// Walks one glyph. `None` when the font's budget ran out.
    fn glyph(&mut self, program: &'a [u8], local: Option<Index<'a>>, vs_index: usize) -> Option<CharstringScan> {
        self.local = local;
        self.vs_index = vs_index;
        self.stack.clear();
        self.stems = 0;
        self.chain.clear();
        self.scan = CharstringScan::default();
        match self.walk(program, 0) {
            Flow::Budget => None,
            Flow::Return | Flow::End => Some(self.scan),
        }
    }

    fn push(&mut self, v: f64) -> bool {
        if self.stack.len() >= MAX_CHARSTRING_STACK {
            return false;
        }
        self.stack.push(v);
        true
    }

    fn walk(&mut self, program: &'a [u8], depth: usize) -> Flow {
        self.scan.max_depth = self.scan.max_depth.max(depth);
        if depth > MAX_CALL_DEPTH {
            return Flow::Return;
        }
        let byte = |i: usize| program.get(i).copied();
        let mut i = 0usize;
        while let Some(b) = byte(i) {
            self.ops = self.ops.saturating_add(1);
            if self.ops > MAX_CHARSTRING_OPS {
                return Flow::Budget;
            }
            i += 1;
            let number = match b {
                28 => {
                    let (Some(hi), Some(lo)) = (byte(i), byte(i + 1)) else { return Flow::End };
                    i += 2;
                    Some(f64::from(i16::from_be_bytes([hi, lo])))
                }
                32..=246 => Some(f64::from(b) - 139.0),
                247..=250 => {
                    let Some(b1) = byte(i) else { return Flow::End };
                    i += 1;
                    Some(f64::from(b - 247) * 256.0 + f64::from(b1) + 108.0)
                }
                251..=254 => {
                    let Some(b1) = byte(i) else { return Flow::End };
                    i += 1;
                    Some(-f64::from(b - 251) * 256.0 - f64::from(b1) - 108.0)
                }
                255 => {
                    let Some(v) = program.get(i..i + 4).and_then(|s| <[u8; 4]>::try_from(s).ok()) else { return Flow::End };
                    i += 4;
                    Some(f64::from(i32::from_be_bytes(v)) / 65_536.0)
                }
                _ => None,
            };
            if let Some(v) = number {
                if !self.push(v) {
                    return Flow::End;
                }
                continue;
            }
            match b {
                // hstem, vstem, hstemhm, vstemhm
                1 | 3 | 18 | 23 => {
                    self.stems = self.stems.saturating_add(self.stack.len() / 2);
                    self.stack.clear();
                }
                // hintmask, cntrmask: operands left on the stack are an implied vstem.
                19 | 20 => {
                    self.stems = self.stems.saturating_add(self.stack.len() / 2);
                    self.stack.clear();
                    i = i.saturating_add(self.stems.div_ceil(8));
                }
                // callsubr, callgsubr
                10 | 29 => {
                    let Some(n) = self.stack.pop() else { return Flow::End };
                    match self.call(b == 29, n, depth) {
                        Flow::Return => {}
                        other => return other,
                    }
                }
                // return
                11 => return Flow::Return,
                // endchar
                14 if !self.cff2 => {
                    if self.stack.len() >= 4 {
                        self.scan.seac = true;
                    }
                    return Flow::End;
                }
                // vsindex
                15 if self.cff2 => {
                    self.vs_index = self.stack.pop().filter(|v| v.is_finite() && *v >= 0.0).map_or(0, |v| v as usize);
                    self.stack.clear();
                }
                // blend: n results from n × (regions + 1) operands and the count.
                16 if self.cff2 => {
                    let n = self.stack.pop().filter(|v| v.is_finite() && *v >= 0.0).map_or(0, |v| v as usize);
                    let k = self.regions.get(self.vs_index).copied().unwrap_or(0);
                    let keep = self.stack.len().saturating_sub(n.saturating_mul(k));
                    self.stack.truncate(keep);
                }
                12 => {
                    let Some(b1) = byte(i) else { return Flow::End };
                    i += 1;
                    self.escape(b1);
                }
                _ => self.stack.clear(),
            }
        }
        Flow::Return
    }

    fn call(&mut self, global: bool, n: f64, depth: usize) -> Flow {
        let index = if global { Some(self.global.clone()) } else { self.local.clone() };
        let Some(index) = index else { return Flow::Return };
        if !n.is_finite() {
            return Flow::Return;
        }
        let target = n as i64 + i64::from(index.subr_bias());
        let Ok(target) = usize::try_from(target) else { return Flow::Return };
        let Some(sub) = index.get(target) else { return Flow::Return };
        if self.chain.contains(&(global, target)) {
            self.scan.recursion = true;
            return Flow::End;
        }
        self.chain.push((global, target));
        let flow = self.walk(sub, depth.saturating_add(1));
        self.chain.pop();
        flow
    }

    /// Two-byte operators. The arithmetic operators of Type 2 are tracked by their stack effect
    /// only (results read as 0); every other one clears the stack.
    fn escape(&mut self, op: u8) {
        let len = self.stack.len();
        match op {
            0 => {
                self.scan.dotsection = true;
                self.stack.clear();
            }
            // and, or, add, sub, div, eq, mul: two operands, one result.
            3 | 4 | 10 | 11 | 12 | 15 | 24 => {
                self.stack.truncate(len.saturating_sub(1));
                if let Some(top) = self.stack.last_mut() {
                    *top = 0.0;
                }
            }
            // not, abs, neg, sqrt, get, index: one operand, one result.
            5 | 9 | 14 | 26 | 21 | 29 => {
                if let Some(top) = self.stack.last_mut() {
                    *top = 0.0;
                }
            }
            // drop
            18 => {
                self.stack.pop();
            }
            // put, roll
            20 | 30 => self.stack.truncate(len.saturating_sub(2)),
            // ifelse: four operands, one result.
            22 => {
                self.stack.truncate(len.saturating_sub(3));
                if let Some(top) = self.stack.last_mut() {
                    *top = 0.0;
                }
            }
            // random
            23 => {
                self.push(0.0);
            }
            // dup
            27 => {
                if let Some(top) = self.stack.last().copied() {
                    self.push(top);
                }
            }
            // exch
            28 => {
                if len >= 2 {
                    self.stack.swap(len - 2, len - 1);
                }
            }
            _ => self.stack.clear(),
        }
    }
}

/// Walks every glyph's charstring. Returns the scans of the glyphs that have something to
/// report, by glyph id, and whether the budget ran out before the end.
fn scan_charstrings<'a>(cff: &CffFontRef<'a>) -> (Vec<(u32, CharstringScan)>, bool) {
    let mut walker = Walker::new(cff);
    let mut locals: BTreeMap<u16, (Option<Index<'a>>, usize)> = BTreeMap::new();
    let charstrings = cff.charstrings().clone();
    let count = cff.num_glyphs().min(u32::from(u16::MAX));
    let mut out = Vec::new();
    for gid in 0..count {
        let g = GlyphId::new(gid);
        let Some(program) = charstrings.get(gid as usize) else { continue };
        let fd = cff.subfont_index(g).unwrap_or(0);
        let (local, vs_index) = locals
            .entry(fd)
            .or_insert_with(|| match cff.subfont(fd, &[]) {
                Ok(sub) => {
                    let local = (sub.subrs_offset() != 0)
                        .then(|| cff.data().get(sub.subrs_offset() as usize..).and_then(|d| Index::new(d, cff.version() == 2).ok()))
                        .flatten();
                    (local, usize::from(sub.vs_index()))
                }
                Err(_) => (None, 0),
            })
            .clone();
        let Some(scan) = walker.glyph(program, local, vs_index) else { return (out, true) };
        if scan.max_depth > MAX_CALL_DEPTH || scan.recursion || scan.seac || scan.dotsection {
            out.push((gid, scan));
        }
    }
    (out, false)
}

fn budget_reason() -> String {
    format!("the charstrings take more than {MAX_CHARSTRING_OPS} operations to read")
}

/// `check.tables.cffCallDepth` (FontBakery `opentype/cff_call_depth`,
/// `opentype/cff2_call_depth`).
pub(crate) fn cff_call_depth(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let cff = match cff_font(ctx) {
        Ok(c) => c,
        Err(e) => {
            sink.skip(e);
            return;
        }
    };
    let table = if cff.version() == 2 { "CFF2" } else { "CFF " };
    let namer = Namer::new(ctx);
    let (scans, over) = scan_charstrings(&cff);
    for (gid, scan) in scans {
        let name = namer.name(gid);
        if scan.max_depth > MAX_CALL_DEPTH {
            let msg = format!("The charstring of {name} nests subroutine calls more than {MAX_CALL_DEPTH} deep.");
            sink.push(Hit::new("max-depth", msg).place(glyph_at(name.clone(), table)));
        }
        if scan.recursion {
            let msg = format!("The charstring of {name} calls a subroutine that calls itself.");
            sink.push(Hit::new("recursion", msg).place(glyph_at(name, table)));
        }
    }
    if over {
        sink.skip(budget_reason());
    }
}

/// `check.tables.cffDeprecatedOperators` (FontBakery `opentype/cff_deprecated_operators`;
/// comparefamily test 7).
pub(crate) fn cff_deprecated_operators(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let cff = match cff_font(ctx) {
        Ok(c) if c.version() == 1 => c,
        Ok(_) => return,
        Err(e) => {
            sink.skip(e);
            return;
        }
    };
    let namer = Namer::new(ctx);
    let (scans, over) = scan_charstrings(&cff);
    for (gid, scan) in scans {
        let name = namer.name(gid);
        if scan.seac {
            let msg = format!("{name} is built with endchar's accent form (seac), which some renderers no longer support; decompose it.");
            sink.push(Hit::new("endchar-seac", msg).place(glyph_at(name.clone(), "CFF ")));
        }
        if scan.dotsection {
            let msg = format!("The charstring of {name} uses the deprecated dotsection operator.");
            sink.push(Hit::new("dotsection", msg).place(glyph_at(name, "CFF ")));
        }
    }
    if over {
        sink.skip(budget_reason());
    }
}

/// `check.tables.cffCharstringSize` (comparefamily test 18).
pub(crate) fn cff_charstring_size(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let cff = match cff_font(ctx) {
        Ok(c) => c,
        Err(e) => {
            sink.skip(e);
            return;
        }
    };
    let table = if cff.version() == 2 { "CFF2" } else { "CFF " };
    let namer = Namer::new(ctx);
    let charstrings = cff.charstrings();
    for gid in 0..cff.num_glyphs().min(u32::from(u16::MAX)) {
        let Some(program) = charstrings.get(gid as usize) else { continue };
        if program.len() > MAX_CHARSTRING_BYTES {
            let name = namer.name(gid);
            let msg = format!("The charstring of {name} is {} bytes long; old interpreters stop at {MAX_CHARSTRING_BYTES}.", program.len());
            sink.push(Hit::new("charstring-too-long", msg).data("bytes", program.len() as f64).place(glyph_at(name, table)));
        }
    }
}

/// The top DICTs of the `CFF ` table with the table itself.
fn cff_top_dicts<'a>(ctx: &BinaryCtx<'a>) -> Result<(read_fonts::tables::cff::Cff<'a>, Vec<&'a [u8]>), String> {
    let cff = ctx.font.cff().map_err(|e| format!("the CFF table cannot be read: {e}"))?;
    let index = cff.top_dicts();
    let dicts = (0..index.count() as usize).take(MAX_TOP_DICTS).filter_map(|i| index.get(i)).collect();
    Ok((cff, dicts))
}

/// `check.tables.cffAsciiStrings` (FontBakery `opentype/cff_ascii_strings`): the font name and
/// the top DICT's string entries.
pub(crate) fn cff_ascii_strings(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (cff, dicts) = match cff_top_dicts(ctx) {
        Ok(v) => v,
        Err(e) => {
            sink.skip(e);
            return;
        }
    };
    let mut check = |key: &str, bytes: Option<&[u8]>| {
        let place = Location { table: Some("CFF ".into()), field: Some(key.to_string()), ..Location::default() };
        match bytes {
            None => sink.push(Hit::new("cannot-decode", format!("The CFF {key} string cannot be found in the string table.")).place(place)),
            Some(b) if !b.is_ascii() => {
                let shown = String::from_utf8_lossy(b);
                sink.push(Hit::new("not-ascii", format!("The CFF {key} '{shown}' has characters outside ASCII.")).place(place));
            }
            Some(_) => {}
        }
    };
    for (i, dict) in dicts.iter().enumerate() {
        check("FontName", cff.name(i));
        let mut strings: Vec<(&str, Sid)> = Vec::new();
        for entry in entries(dict, None).take(MAX_DICT_ENTRIES) {
            match entry {
                Ok(Entry::Version(s)) => strings.push(("version", s)),
                Ok(Entry::Notice(s)) => strings.push(("Notice", s)),
                Ok(Entry::Copyright(s)) => strings.push(("Copyright", s)),
                Ok(Entry::FullName(s)) => strings.push(("FullName", s)),
                Ok(Entry::FamilyName(s)) => strings.push(("FamilyName", s)),
                Ok(Entry::Weight(s)) => strings.push(("Weight", s)),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        for (key, sid) in strings {
            check(key, cff.string(sid));
        }
    }
}

/// `check.tables.cffUniqueIds` (comparefamily test 25): name-keyed fonts only.
pub(crate) fn cff_unique_ids(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (_, dicts) = match cff_top_dicts(ctx) {
        Ok(v) => v,
        Err(e) => {
            sink.skip(e);
            return;
        }
    };
    for dict in dicts {
        let mut found: Vec<(&'static str, &'static str)> = Vec::new();
        let mut cid = false;
        for entry in entries(dict, None).take(MAX_DICT_ENTRIES) {
            match entry {
                Ok(Entry::UniqueId(_)) => found.push(("unique-id", "UniqueID")),
                Ok(Entry::Xuid) => found.push(("xuid", "XUID")),
                Ok(Entry::UidBase(_)) => found.push(("uid-base", "UIDBase")),
                Ok(Entry::Ros { .. }) => cid = true,
                Ok(_) => {}
                Err(_) => break,
            }
        }
        if cid {
            continue;
        }
        for (code, key) in found {
            let msg = format!(
                "The CFF top dictionary has a {key} entry; these identifiers are obsolete and can collide with another font's in printer caches."
            );
            sink.push(Hit::new(code, msg).place(field(ctx, "CFF ", key)));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// BASE, meta, STAT
// ---------------------------------------------------------------------------------------------

/// `check.tables.baseTable` (comparefamily test 11).
pub(crate) fn base_table(ctx: &BinaryCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    let place = || table_at(*b"BASE");
    if !has(ctx, b"BASE") {
        let msg = "The font has no BASE table, which applications use to align this font with text in other scripts.";
        sink.push(Hit::new("missing-base", msg).place(place()));
        return;
    }
    let base = match ctx.font.base() {
        Ok(b) => b,
        Err(e) => {
            sink.push(Hit::new("unreadable", format!("The BASE table cannot be read: {e}.")).place(place()));
            return;
        }
    };
    let tolerance = params.get(&BASE_TOLERANCE);
    let upem = ctx.font.head().ok().map(|h| f64::from(h.units_per_em()));
    let cap_height = ctx.font.os2().ok().and_then(|o| o.s_cap_height()).map(f64::from);
    let ideo_offset = upem.zip(cap_height).map(|(u, c)| -(u - c) / 2.0);
    for (axis_name, axis) in [("HorizAxis", base.horiz_axis()), ("VertAxis", base.vert_axis())] {
        let Some(axis) = axis else { continue };
        let unreadable =
            |what: &str, e: ReadError| Hit::new("unreadable", format!("The BASE {axis_name} {what} cannot be read: {e}.")).place(place());
        let axis = match axis {
            Ok(a) => a,
            Err(e) => {
                sink.push(unreadable("axis", e));
                continue;
            }
        };
        let tag_list = match axis.base_tag_list() {
            None => continue,
            Some(Ok(t)) => t,
            Some(Err(e)) => {
                sink.push(unreadable("tag list", e));
                continue;
            }
        };
        let baselines: Vec<[u8; 4]> = tag_list.baseline_tags().iter().map(|t| t.get().to_be_bytes()).collect();
        for tag in &baselines {
            if !tags::is_registered_baseline(*tag) {
                let msg = format!("The BASE {axis_name} uses the baseline tag '{}', which is not registered.", text(*tag));
                sink.push(Hit::new("unregistered-baseline-tag", msg).place(place()));
            }
        }
        let scripts = match axis.base_script_list() {
            Ok(s) => s,
            Err(e) => {
                sink.push(unreadable("script list", e));
                continue;
            }
        };
        for rec in scripts.base_script_records() {
            let stag = rec.base_script_tag().to_be_bytes();
            let script_place = || Location { script: Some(text(stag)), ..place() };
            if !tags::is_registered_script(stag) {
                let msg = format!("The BASE {axis_name} uses the script tag '{}', which is not registered.", text(stag));
                sink.push(Hit::new("unregistered-script-tag", msg).place(script_place()));
            }
            let script = match rec.base_script(scripts.offset_data()) {
                Ok(s) => s,
                Err(e) => {
                    sink.push(unreadable("script", e));
                    continue;
                }
            };
            let values = match script.base_values() {
                None => continue,
                Some(Ok(v)) => v,
                Some(Err(e)) => {
                    sink.push(unreadable("base values", e));
                    continue;
                }
            };
            if values.base_coord_count() != tag_list.base_tag_count() {
                let msg = format!(
                    "The BASE {axis_name} values of script '{}' have {} coordinates for {} baseline tags.",
                    text(stag),
                    values.base_coord_count(),
                    tag_list.base_tag_count()
                );
                sink.push(Hit::new("coord-count-mismatch", msg).place(script_place()));
                continue;
            }
            let default = usize::from(values.default_baseline_index());
            for (i, coord) in values.base_coords().iter().enumerate() {
                let Ok(coord) = coord else {
                    sink.push(
                        Hit::new("unreadable", format!("A BASE {axis_name} coordinate of script '{}' cannot be read.", text(stag)))
                            .place(script_place()),
                    );
                    continue;
                };
                let c = f64::from(coord.coordinate());
                let Some(tag) = baselines.get(i).copied() else { continue };
                if i == default {
                    if c != 0.0 {
                        let msg = format!(
                            "The default baseline '{}' of script '{}' in the BASE {axis_name} is offset by {}; the default baseline should be at 0.",
                            text(tag),
                            text(stag),
                            num(c)
                        );
                        sink.push(Hit::new("default-baseline-offset", msg).data("actual", c).place(script_place()));
                    }
                } else if &tag == b"ideo"
                    && let Some(wanted) = ideo_offset
                    && (c - wanted).abs() > tolerance
                {
                    let msg = format!(
                        "The ideographic baseline of script '{}' in the BASE {axis_name} is {}; {} centres the cap height in the em box.",
                        text(stag),
                        num(c),
                        num(wanted)
                    );
                    sink.push(Hit::new("ideo-not-centred", msg).data("actual", c).data("expected", wanted).place(script_place()));
                }
            }
        }
    }
}

/// `check.tables.metaScriptLangTags` (FontBakery `googlefonts/meta/script_lang_tags`).
pub(crate) fn meta_script_lang_tags(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let place = || table_at(*b"meta");
    let Some(bytes) = data(ctx, b"meta") else {
        sink.push(
            Hit::new("lacks-meta-table", "The font has no meta table to say which languages and scripts it is designed for and supports.")
                .place(place()),
        );
        return;
    };
    let meta = match ctx.font.meta() {
        Ok(m) => m,
        Err(e) => {
            sink.skip(format!("the meta table cannot be read: {e}"));
            return;
        }
    };
    let entry = |tag: &[u8; 4]| {
        meta.data_maps().iter().find(|r| r.tag() == Tag::new(tag)).map(|r| {
            let start = r.data_offset().to_u32() as usize;
            let end = start.saturating_add(r.data_length() as usize);
            let raw = bytes.get(start..end.min(bytes.len())).unwrap_or_default();
            let shown = String::from_utf8_lossy(raw.get(..raw.len().min(MAX_META_TEXT)).unwrap_or_default()).into_owned();
            if raw.len() > MAX_META_TEXT { format!("{shown}…") } else { shown }
        })
    };
    for (tag, missing, present, what) in [
        (b"dlng", "missing-dlng-tag", "dlng-tag", "the languages and scripts the font is designed for"),
        (b"slng", "missing-slng-tag", "slng-tag", "the languages and scripts the font supports"),
    ] {
        match entry(tag) {
            None => {
                let msg = format!("The meta table has no '{}' entry listing {what}.", text(*tag));
                sink.push(Hit::new(missing, msg).place(place()));
            }
            Some(value) => {
                sink.push(Hit::new(present, format!("meta '{}': {value}", text(*tag))).place(place()).without_fix());
            }
        }
    }
}

/// `check.tables.statInStatics` (FontBakery `STAT_in_statics`).
pub(crate) fn stat_in_statics(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    if has(ctx, b"fvar") {
        return;
    }
    let stat = match ctx.font.stat() {
        Ok(s) => s,
        Err(e) => {
            sink.skip(format!("the STAT table cannot be read: {e}"));
            return;
        }
    };
    let Ok(axes) = stat.design_axes() else {
        sink.skip("the STAT design axes cannot be read");
        return;
    };
    let Some(Ok(values)) = stat.offset_to_axis_values() else { return };
    let mut counts: BTreeMap<u16, usize> = BTreeMap::new();
    let mut count = |i: u16| {
        let n = counts.entry(i).or_default();
        *n = n.saturating_add(1);
    };
    for value in values.axis_values().iter().take(MAX_AXIS_VALUES) {
        match value {
            Ok(AxisValue::Format1(v)) => count(v.axis_index()),
            Ok(AxisValue::Format2(v)) => count(v.axis_index()),
            Ok(AxisValue::Format3(v)) => count(v.axis_index()),
            Ok(AxisValue::Format4(v)) => v.axis_values().iter().for_each(|r| count(r.axis_index())),
            Err(_) => {}
        }
    }
    for (index, n) in counts {
        if n < 2 {
            continue;
        }
        let tag = axes.get(usize::from(index)).map_or_else(|| format!("axis {index}"), |a| text(a.axis_tag().to_be_bytes()));
        let msg = format!(
            "The STAT table of this static font has {n} values for the '{tag}' axis; a static font describes one style, so one value per axis is expected."
        );
        let place = Location { axis: Some(tag), ..table_at(*b"STAT") };
        sink.push(Hit::new("multiple-stat-entries", msg).data("count", n as f64).place(place));
    }
}

// ---------------------------------------------------------------------------------------------
// check.outlines.pointsOutsideBBox
// ---------------------------------------------------------------------------------------------

/// Every point a composite glyph draws, off-curve points included.
#[derive(Default)]
struct PointPen(Vec<(f64, f64)>);

impl OutlinePen for PointPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push((f64::from(x), f64::from(y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push((f64::from(x), f64::from(y)));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.extend([(f64::from(cx0), f64::from(cy0)), (f64::from(x), f64::from(y))]);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.extend([(f64::from(cx0), f64::from(cy0)), (f64::from(cx1), f64::from(cy1)), (f64::from(x), f64::from(y))]);
    }
    fn close(&mut self) {}
}

/// `check.outlines.pointsOutsideBBox` (FontBakery `opentype/points_out_of_bounds`): coordinates
/// are rounded half to even before the comparison, as there.
pub(crate) fn points_outside_bbox(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let (Ok(loca), Ok(glyf), Ok(maxp)) = (ctx.font.loca(None), ctx.font.glyf(), ctx.font.maxp()) else {
        sink.skip("the glyf, loca or maxp table cannot be read");
        return;
    };
    let outlines = ctx.font.outline_glyphs();
    let namer = Namer::new(ctx);
    let mut read = 0usize;
    for gid in 0..u32::from(maxp.num_glyphs()) {
        let g = GlyphId::new(gid);
        let (bbox, points) = match loca.get(g, &glyf) {
            Some(LocaGlyph::Glyph(TtGlyph::Simple(s))) => {
                let bbox = [s.x_min(), s.y_min(), s.x_max(), s.y_max()].map(f64::from);
                (bbox, s.points().map(|p| (f64::from(p.x), f64::from(p.y))).collect::<Vec<_>>())
            }
            Some(LocaGlyph::Glyph(TtGlyph::Composite(c))) => {
                let bbox = [c.x_min(), c.y_min(), c.x_max(), c.y_max()].map(f64::from);
                let mut pen = PointPen::default();
                let Some(glyph) = outlines.get(g) else { continue };
                if glyph.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).is_err() {
                    continue;
                }
                (bbox, pen.0)
            }
            _ => continue,
        };
        read = read.saturating_add(points.len());
        if read > MAX_OUTLINE_POINTS {
            sink.skip(format!("the glyphs have more than {MAX_OUTLINE_POINTS} points"));
            return;
        }
        let [x0, y0, x1, y1] = bbox;
        let outside = |(x, y): &(f64, f64)| {
            let (rx, ry) = (x.round_ties_even(), y.round_ties_even());
            rx < x0 || rx > x1 || ry < y0 || ry > y1 || x.abs() > MAX_COORDINATE || y.abs() > MAX_COORDINATE
        };
        let bad: Vec<&(f64, f64)> = points.iter().filter(|p| outside(p)).collect();
        let Some(&&(x, y)) = bad.first() else { continue };
        let name = namer.name(gid);
        let msg = format!(
            "{name} has {} point(s) outside its stored bounding box ({}, {}, {}, {}), first at ({}, {}). Usually the outline lacks points at its extremes.",
            bad.len(),
            num(x0),
            num(y0),
            num(x1),
            num(y1),
            num(x),
            num(y)
        );
        sink.push(
            Hit::new("points-out-of-bounds", msg)
                .at(Point::new(x, y))
                .bounds(Rect::new(x0, y0, x1, y1))
                .data("count", bad.len() as f64)
                .place(glyph_at(name, "glyf")),
        );
    }
}

#[cfg(test)]
#[path = "tables_tests.rs"]
mod tests;
