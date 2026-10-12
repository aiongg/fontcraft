//! Hinting checks (M12.55): TrueType instruction settings, the `gasp` table, the automatic
//! TrueType hinter's traces, CFF stem hints and PostScript alignment zones in compiled fonts, and
//! the alignment zones and standard stems of a source's masters. Runners take the context of
//! their scope ([`crate::BinaryCtx`], [`crate::SourceCtx`]), the thresholds and a
//! [`crate::Sink`]; their registry entries are in `registry/hinting.rs`.
//!
//! The rules of `integerPpem`, `smartDropout`, `sizeImpact`, `gasp` and `autohinterTraces`
//! follow FontBakery 1.1.0's checks `integer_ppem_if_hinted`, `smart_dropout`, `hinting_impact`,
//! `googlefonts/gasp`, `googlefonts/has_ttfautohint_params` and `googlefonts/old_ttfautohint`
//! (Apache License 2.0). The CFF checks follow single-face tests 10, 18 and 30 to 34 and family
//! test 18 of comparefamily in AFDKO 5.0.1 (Apache License 2.0, Copyright 2015 Adobe). Only the
//! rules are taken; the code is written here. See `NOTICE`.

use std::ops::Range;

use fontcraft_model::{Font, Master, metric};
use read_fonts::TableProvider;
use read_fonts::ps::cff::CffFontRef;
use read_fonts::ps::cff::dict::{self, Entry};
use read_fonts::ps::cff::index::Index;
use read_fonts::ps::cs::CommandSink;
use read_fonts::tables::glyf::Glyph as TtGlyph;
use read_fonts::tables::loca::LocaGlyph;
use read_fonts::types::{Fixed, GlyphId, GlyphId16, NameId, Tag};

use crate::name_records::{self, MAX_NAME_TEXT_BYTES};
use crate::{BinaryCtx, Hit, Location, MAX_FINDINGS_PER_CHECK, Params, Sink, SourceCtx, Threshold, num};

/// Font dictionaries read from a CID-keyed CFF table: FDSelect names them with one byte.
const MAX_FONT_DICTS: u32 = 256;
/// Operands kept for one DICT operator (the CFF limit is 48).
const MAX_DICT_OPERANDS: usize = 513;
/// Nibbles read from one real number in a DICT.
const MAX_REAL_NIBBLES: usize = 64;
/// Stems recorded for one glyph (Type 2 allows 96).
const MAX_GLYPH_STEMS: usize = 1_024;
/// Drawing and hint callbacks over all glyphs of one font before the stem checks give up.
const MAX_CHARSTRING_EVENTS: usize = 20_000_000;
/// Alignment zones of one master that the source check compares pairwise.
const MAX_MASTER_ZONES: usize = 256;
/// Gasp ranges listed in the informational result.
const MAX_LISTED_RANGES: usize = 64;

/// The newest release of the automatic TrueType hinter, as the reference check knows it.
const LATEST_AUTOHINTER: &str = "1.8.4";
/// `PUSHW 0x01FF; SCANCTRL; PUSHB 4; SCANTYPE`: dropout control on, smart dropout rules.
const SMART_DROPOUT: [u8; 7] = [0xb8, 0x01, 0xff, 0x85, 0xb0, 0x04, 0x8d];
/// Tables that only carry TrueType hinting; dehinting drops them.
const HINTING_TABLES: [&[u8; 4]; 7] = [b"fpgm", b"prep", b"cvt ", b"hdmx", b"LTSH", b"VDMX", b"TTFA"];
/// The `gasp` behaviour a catch-all range should have: grid-fitting, grayscale, symmetric
/// grid-fitting and symmetric smoothing.
const GASP_ALL: u16 = 0x0F;
const DEFAULT_BLUE_SCALE: f64 = 0.039625;
const DEFAULT_BLUE_FUZZ: f64 = 1.0;

pub(crate) const MIN_ZONE_GAP: Threshold = Threshold {
    name: "minGap",
    default: 1.0,
    min: 0.0,
    max: 100.0,
    doc: "Alignment zones closer together than this many units are reported; 1 reports zones that overlap or touch.",
};

fn data<'a>(ctx: &BinaryCtx<'a>, tag: &[u8; 4]) -> Option<&'a [u8]> {
    ctx.font.table_data(Tag::new(tag)).map(|d| d.as_bytes())
}

fn has(ctx: &BinaryCtx<'_>, tag: &[u8; 4]) -> bool {
    data(ctx, tag).is_some()
}

/// The location of a table, or of one of its fields, with the source field behind it when the
/// font was compiled from a source in this run.
fn at(ctx: &BinaryCtx<'_>, table: &str, field: Option<&str>) -> Location {
    let source_field = field.and_then(|f| ctx.source.and_then(|s| s.field(&format!("{}.{f}", table.trim_end())))).map(str::to_string);
    Location { table: Some(table.to_string()), field: field.map(str::to_string), source_field, ..Location::default() }
}

/// The location of a glyph in a table.
fn glyph_at(name: String, table: &str) -> Location {
    Location { glyph: name, table: Some(table.to_string()), ..Location::default() }
}

// ---------------------------------------------------------------------------------------------
// TrueType
// ---------------------------------------------------------------------------------------------

/// `check.hinting.integerPpem`.
pub(crate) fn integer_ppem(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    if !has(ctx, b"fpgm") {
        sink.skip("the font has no 'fpgm' table, so it has no TrueType instructions");
        return;
    }
    let Ok(head) = ctx.font.head() else {
        sink.skip("the font's 'head' table cannot be read");
        return;
    };
    let flags = head.flags().bits();
    if flags & 0x0008 == 0 {
        sink.push(
            Hit::new(
                "bad-flags",
                "The font has TrueType instructions, but bit 3 of head.flags (force integer sizes) is clear, so scalers may use fractional sizes the instructions were not written for.",
            )
            .data("flags", f64::from(flags))
            .place(at(ctx, "head", Some("flags"))),
        );
    }
}

/// `check.hinting.smartDropout`.
pub(crate) fn smart_dropout(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    if !has(ctx, b"glyf") {
        sink.skip("the font has no TrueType outlines");
        return;
    }
    if has(ctx, b"TSI5") {
        sink.skip("the font was hinted with a visual hinting tool (it has a 'TSI5' table), which sets dropout control itself");
        return;
    }
    let found = data(ctx, b"prep").is_some_and(|p| p.windows(SMART_DROPOUT.len()).any(|w| w == SMART_DROPOUT));
    if !found {
        sink.push(
            Hit::new(
                "lacks-smart-dropout",
                "The 'prep' program does not switch on dropout control with smart dropout (SCANCTRL 0x01FF, SCANTYPE 4), so thin parts may drop out at small sizes.",
            )
            .place(at(ctx, "prep", None)),
        );
    }
}

/// `check.hinting.sizeImpact`.
pub(crate) fn size_impact(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let removed = if has(ctx, b"glyf") {
        truetype_hint_bytes(ctx)
    } else if has(ctx, b"CFF ") || has(ctx, b"CFF2") {
        cff_glyphs(ctx).map(|glyphs| glyphs.iter().map(GlyphHints::encoded_size).fold(0usize, usize::saturating_add))
    } else {
        sink.skip("the font has no TrueType or CFF outlines");
        return;
    };
    let removed = match removed {
        Ok(n) => n,
        Err(reason) => {
            sink.skip(reason);
            return;
        }
    };
    let hinted = ctx.bytes.len();
    let removed = removed.min(hinted);
    let dehinted = hinted.saturating_sub(removed);
    let change = if dehinted > 0 { (hinted as f64 / dehinted as f64 - 1.0) * 100.0 } else { 0.0 };
    sink.push(
        Hit::new(
            "size-impact",
            format!(
                "Hinting takes about {removed} bytes: the font has {hinted} bytes with hinting and about {dehinted} without, {} % more.",
                num((change * 10.0).round() / 10.0)
            ),
        )
        .data("hinted", hinted as f64)
        .data("dehinted", dehinted as f64)
        .data("increase", removed as f64)
        .data("percent", change),
    );
}

/// The bytes dehinting a TrueType font removes: the hinting tables with their padding and
/// directory entries, and every glyph's instructions.
fn truetype_hint_bytes(ctx: &BinaryCtx<'_>) -> Result<usize, String> {
    let mut total = 0usize;
    for record in ctx.font.table_directory.table_records() {
        if HINTING_TABLES.iter().any(|t| record.tag() == Tag::new(t)) {
            let padded = (record.length() as usize).saturating_add(3) & !3;
            total = total.saturating_add(padded).saturating_add(16);
        }
    }
    let maxp = ctx.font.maxp().map_err(|e| format!("the font's 'maxp' table cannot be read: {e}"))?;
    let glyf = ctx.font.glyf().map_err(|e| format!("the font's 'glyf' table cannot be read: {e}"))?;
    let loca = ctx.font.loca(None).map_err(|e| format!("the font's 'loca' table cannot be read: {e}"))?;
    for gid in 0..u32::from(maxp.num_glyphs()) {
        let bytes = match loca.get(GlyphId::new(gid), &glyf) {
            Some(LocaGlyph::Glyph(TtGlyph::Simple(g))) => usize::from(g.instruction_length()),
            Some(LocaGlyph::Glyph(TtGlyph::Composite(g))) => g.instructions().map_or(0, <[u8]>::len),
            _ => 0,
        };
        total = total.saturating_add(bytes);
    }
    Ok(total)
}

/// `check.hinting.gasp`.
pub(crate) fn gasp(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    if !has(ctx, b"glyf") {
        sink.skip("the font has no TrueType outlines");
        return;
    }
    let place = || at(ctx, "gasp", None);
    if !has(ctx, b"gasp") {
        sink.push(Hit::new("lacks-gasp", "The font has no 'gasp' table, so renderers guess when to grid-fit and smooth.").place(place()));
        return;
    }
    let Ok(table) = ctx.font.gasp() else {
        sink.push(Hit::new("empty", "The 'gasp' table cannot be read, so it gives no rendering ranges.").place(place()));
        return;
    };
    let ranges = table.gasp_ranges();
    if !ranges.iter().any(|r| r.range_max_ppem() == 0xFFFF) {
        sink.push(
            Hit::new("lacks-ffff-range", "The 'gasp' table has no range that ends at 65535 ppem, so it does not cover every size.").place(place()),
        );
        return;
    }
    let listed: Vec<String> = ranges
        .iter()
        .take(MAX_LISTED_RANGES)
        .map(|r| {
            format!("up to {} ppem: 0x{:02X} ({})", r.range_max_ppem(), r.range_gasp_behavior().bits(), gasp_meaning(r.range_gasp_behavior().bits()))
        })
        .collect();
    sink.push(
        Hit::new("ranges", format!("The 'gasp' ranges: {}.", listed.join("; "))).data("ranges", ranges.len() as f64).without_fix().place(place()),
    );
    for r in ranges {
        let (max, behavior) = (r.range_max_ppem(), r.range_gasp_behavior().bits());
        if max != 0xFFFF {
            sink.push(
                Hit::new(
                    "non-ffff-range",
                    format!("The 'gasp' table has a range up to {max} ppem, which is probably not needed: one range for all sizes is enough."),
                )
                .data("maxPpem", f64::from(max))
                .place(place()),
            );
        } else if behavior != GASP_ALL {
            sink.push(
                Hit::new("unset-flags", format!("The 'gasp' range for all sizes has flags 0x{behavior:02X}; all four (0x0F) should be set."))
                    .data("flags", f64::from(behavior))
                    .place(place()),
            );
        }
    }
}

fn gasp_meaning(bits: u16) -> String {
    let names = [(0x01, "grid-fitting"), (0x02, "grayscale"), (0x04, "symmetric grid-fitting"), (0x08, "symmetric smoothing")];
    let set: Vec<&str> = names.iter().filter(|(bit, _)| bits & bit != 0).map(|(_, n)| *n).collect();
    if set.is_empty() { "nothing".to_string() } else { set.join(", ") }
}

/// `check.hinting.autohinterTraces`.
pub(crate) fn autohinter_traces(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let versions = version_strings(ctx, sink);
    let place = || Location { table: Some("name".into()), name_id: Some(5), ..Location::default() };
    if versions.is_empty() {
        sink.push(Hit::new("lacks-version-strings", "The 'name' table has no version string (name ID 5).").place(place()));
        return;
    }
    // A version string without the hinter's mark means the font was not hinted by it.
    let has_params = versions.iter().any(|v| autohinter_params(v).is_none_or(|(_, params)| !params.is_empty()));
    if !has_params {
        sink.push(
            Hit::new(
                "lacks-ttfa-params",
                "The version strings name the automatic TrueType hinter but not the options it ran with, so the hinting cannot be repeated on a subset.",
            )
            .place(place()),
        );
    }
    if !has(ctx, b"glyf") {
        return;
    }
    match versions.iter().find_map(|v| autohinter_version(v)) {
        None => sink.push(
            Hit::new(
                "version-not-detected",
                format!("No version of the automatic TrueType hinter is recorded in the version strings: {}.", versions.join(" | ")),
            )
            .place(place()),
        ),
        Some(version) if older(version, LATEST_AUTOHINTER) => sink.push(
            Hit::new(
                "old-ttfa",
                format!("The font was hinted with version {version} of the automatic TrueType hinter; the latest is {LATEST_AUTOHINTER}."),
            )
            .place(place()),
        ),
        Some(_) => {}
    }
}

/// Every version string (name ID 5) the font has, on any platform, read with the shared byte
/// budget; a skip says so when the budget ran out.
fn version_strings(ctx: &BinaryCtx<'_>, sink: &mut Sink) -> Vec<String> {
    let mut budget = MAX_NAME_TEXT_BYTES;
    let Ok(read) = name_records::read(&ctx.font, &mut budget, |_, _, _, id| id == NameId::VERSION_STRING.to_u16()) else { return Vec::new() };
    if read.over_budget {
        sink.skip(name_records::over_budget_reason());
    }
    read.records.into_iter().filter(|r| r.in_bounds).map(|r| r.text.unwrap_or_default()).collect()
}

const AUTOHINTER_MARK: &str = "ttfautohint (v";

/// The hinter version in `ttfautohint (vX)`: the text up to the last `)` on the line.
fn autohinter_version(s: &str) -> Option<&str> {
    s.split('\n').find_map(|line| {
        let start = line.find(AUTOHINTER_MARK)?.saturating_add(AUTOHINTER_MARK.len());
        let rest = line.get(start..)?;
        rest.get(..rest.rfind(')')?)
    })
}

/// The hinter version and options in `ttfautohint (vX) OPTIONS`: the version up to the last
/// `) ` on the line, the options after it up to the first `;`.
fn autohinter_params(s: &str) -> Option<(&str, &str)> {
    s.split('\n').find_map(|line| {
        let start = line.find(AUTOHINTER_MARK)?.saturating_add(AUTOHINTER_MARK.len());
        let rest = line.get(start..)?;
        let end = rest.rfind(") ")?;
        let version = rest.get(..end)?;
        let params = rest.get(end.saturating_add(2)..)?.split(';').next().unwrap_or("");
        Some((version, params))
    })
}

/// Is version `v` older than `latest`? Number by number when both are dotted numbers, as text
/// otherwise.
fn older(v: &str, latest: &str) -> bool {
    let parse = |s: &str| s.split('.').map(|p| p.trim().parse::<u32>().ok()).collect::<Option<Vec<u32>>>();
    match (parse(v), parse(latest)) {
        (Some(a), Some(b)) => a < b,
        _ => v < latest,
    }
}

// ---------------------------------------------------------------------------------------------
// CFF stem hints
// ---------------------------------------------------------------------------------------------

/// What one glyph's charstring draws and hints, in charstring units.
pub(crate) struct GlyphHints {
    gid: u32,
    /// The charstring draws something.
    marking: bool,
    /// The outline's box, curve extremes included: `[x_min, y_min, x_max, y_max]`.
    bbox: Option<[f64; 4]>,
    /// Horizontal stems as `(lower edge, upper edge)`; a ghost hint has the edges 20 or 21 units
    /// the wrong way round, as the charstring encodes it.
    h: Vec<(f64, f64)>,
    v: Vec<(f64, f64)>,
    /// Bytes of hint and counter masks, operators included.
    mask_bytes: usize,
}

impl GlyphHints {
    /// The bytes the glyph's hints take in its charstring: operands encoded as deltas, one
    /// operator per direction and the masks. Hints called from subroutines count once per glyph
    /// that calls them, so this is an estimate.
    fn encoded_size(&self) -> usize {
        fn stems(list: &[(f64, f64)]) -> usize {
            if list.is_empty() {
                return 0;
            }
            let mut prev = 0.0;
            let mut n = 1usize;
            for &(lo, hi) in list {
                n = n.saturating_add(operand_size(lo - prev)).saturating_add(operand_size(hi - lo));
                prev = hi;
            }
            n
        }
        stems(&self.h).saturating_add(stems(&self.v)).saturating_add(self.mask_bytes)
    }
}

/// The bytes a Type 2 charstring operand takes.
fn operand_size(v: f64) -> usize {
    if !v.is_finite() || v.fract() != 0.0 {
        return 5;
    }
    match v.abs() {
        a if a <= 107.0 => 1,
        a if a <= 1131.0 => 2,
        a if a <= 32_767.0 => 3,
        _ => 5,
    }
}

#[derive(Default)]
struct HintSink {
    cur: (f64, f64),
    bbox: Option<[f64; 4]>,
    marking: bool,
    h: Vec<(f64, f64)>,
    v: Vec<(f64, f64)>,
    mask_bytes: usize,
    events: usize,
}

impl HintSink {
    fn extend(&mut self, (x, y): (f64, f64)) {
        if !(x.is_finite() && y.is_finite()) {
            return;
        }
        self.bbox = Some(match self.bbox {
            None => [x, y, x, y],
            Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        });
    }

    fn stem(&mut self, horizontal: bool, a: Fixed, b: Fixed) {
        self.events = self.events.saturating_add(1);
        if self.h.len().saturating_add(self.v.len()) >= MAX_GLYPH_STEMS {
            return;
        }
        let list = if horizontal { &mut self.h } else { &mut self.v };
        list.push((a.to_f64(), b.to_f64()));
    }
}

impl CommandSink for HintSink {
    fn move_to(&mut self, x: Fixed, y: Fixed) {
        self.events = self.events.saturating_add(1);
        self.marking = true;
        self.cur = (x.to_f64(), y.to_f64());
    }

    fn line_to(&mut self, x: Fixed, y: Fixed) {
        self.events = self.events.saturating_add(1);
        let p = (x.to_f64(), y.to_f64());
        self.extend(self.cur);
        self.extend(p);
        self.cur = p;
    }

    fn curve_to(&mut self, cx0: Fixed, cy0: Fixed, cx1: Fixed, cy1: Fixed, x: Fixed, y: Fixed) {
        self.events = self.events.saturating_add(1);
        let p0 = self.cur;
        let (p1, p2, p3) = ((cx0.to_f64(), cy0.to_f64()), (cx1.to_f64(), cy1.to_f64()), (x.to_f64(), y.to_f64()));
        self.extend(p0);
        self.extend(p3);
        for t in cubic_extrema(p0.0, p1.0, p2.0, p3.0).into_iter().chain(cubic_extrema(p0.1, p1.1, p2.1, p3.1)).flatten() {
            self.extend((cubic_at(p0.0, p1.0, p2.0, p3.0, t), cubic_at(p0.1, p1.1, p2.1, p3.1, t)));
        }
        self.cur = p3;
    }

    fn close(&mut self) {}

    fn hstem(&mut self, y: Fixed, dy: Fixed) {
        self.stem(true, y, dy);
    }

    fn vstem(&mut self, x: Fixed, dx: Fixed) {
        self.stem(false, x, dx);
    }

    fn hint_mask(&mut self, mask: &[u8]) {
        self.mask_bytes = self.mask_bytes.saturating_add(mask.len()).saturating_add(1);
    }

    fn counter_mask(&mut self, mask: &[u8]) {
        self.mask_bytes = self.mask_bytes.saturating_add(mask.len()).saturating_add(1);
    }
}

fn cubic_at(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    let mt = 1.0 - t;
    mt * mt * mt * p0 + 3.0 * mt * mt * t * p1 + 3.0 * mt * t * t * p2 + t * t * t * p3
}

/// The parameters in (0, 1) where one coordinate of a cubic has a turning point.
fn cubic_extrema(p0: f64, p1: f64, p2: f64, p3: f64) -> [Option<f64>; 2] {
    let (d0, d1, d2) = (p1 - p0, p2 - p1, p3 - p2);
    let (a, b, c) = (d0 - 2.0 * d1 + d2, 2.0 * (d1 - d0), d0);
    let inside = |t: f64| (t.is_finite() && t > 0.0 && t < 1.0).then_some(t);
    if a.abs() < 1e-12 {
        if b.abs() < 1e-12 {
            return [None, None];
        }
        return [inside(-c / b), None];
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return [None, None];
    }
    let root = disc.sqrt();
    [inside((-b + root) / (2.0 * a)), inside((-b - root) / (2.0 * a))]
}

/// The bytes of the font's `CFF ` table, or its `CFF2` table.
fn cff_table<'a>(ctx: &BinaryCtx<'a>) -> Option<&'a [u8]> {
    data(ctx, b"CFF ").or_else(|| data(ctx, b"CFF2"))
}

/// Every glyph's drawing and hints, at the default location of a variable font. Glyphs whose
/// charstring cannot be run are left out.
/// The hints of every CFF glyph, read once per font ([`BinaryCtx`] keeps them) and shared by the
/// hinting checks.
fn cff_glyphs<'c>(ctx: &'c BinaryCtx<'_>) -> Result<&'c [GlyphHints], String> {
    ctx.cff_hints().as_deref().map_err(Clone::clone)
}

pub(crate) fn read_cff_glyphs(ctx: &BinaryCtx<'_>) -> Result<Vec<GlyphHints>, String> {
    let bytes = cff_table(ctx).ok_or("the font has no CFF table")?;
    let upem = ctx.font.head().ok().map(|h| i32::from(h.units_per_em()));
    let cff = CffFontRef::new(bytes, 0, upem).map_err(|e| format!("the CFF table cannot be read: {e}"))?;
    let count = cff.num_glyphs().min(u32::from(u16::MAX));
    let mut out = Vec::with_capacity(count as usize);
    let mut events = 0usize;
    for gid in 0..count {
        let g = GlyphId::new(gid);
        let Some(fd) = cff.subfont_index(g) else { continue };
        let Ok(subfont) = cff.subfont(fd, &[]) else { continue };
        let mut sink = HintSink::default();
        let result = cff.evaluate_charstring(&subfont, g, &[], &mut sink);
        events = events.saturating_add(sink.events);
        if events > MAX_CHARSTRING_EVENTS {
            return Err(format!("the charstrings issue more than {MAX_CHARSTRING_EVENTS} drawing and hint operations"));
        }
        if result.is_err() {
            continue;
        }
        out.push(GlyphHints { gid, marking: sink.marking, bbox: sink.bbox, h: sink.h, v: sink.v, mask_bytes: sink.mask_bytes });
    }
    Ok(out)
}

/// Names glyphs for findings: the source glyph, the `post` name, the CFF charset name, or the
/// glyph id. Past `MAX_FINDINGS_PER_CHECK` names the sink drops the findings anyway, so the cheap
/// fallback is used.
struct Namer<'a> {
    ctx: &'a BinaryCtx<'a>,
    cff: Option<CffFontRef<'a>>,
    used: usize,
}

impl<'a> Namer<'a> {
    fn new(ctx: &'a BinaryCtx<'a>) -> Self {
        let upem = ctx.font.head().ok().map(|h| i32::from(h.units_per_em()));
        let cff = data(ctx, b"CFF ").and_then(|b| CffFontRef::new(b, 0, upem).ok());
        Self { ctx, cff, used: 0 }
    }

    fn name(&mut self, gid: u32) -> String {
        self.used = self.used.saturating_add(1);
        if self.used > MAX_FINDINGS_PER_CHECK {
            return format!("gid{gid}");
        }
        if let Some(name) = self.ctx.source.and_then(|s| s.glyph(gid)) {
            return name.to_string();
        }
        if let (Ok(post), Ok(id)) = (self.ctx.font.post(), u16::try_from(gid))
            && let Some(name) = post.glyph_name(GlyphId16::new(id))
        {
            return name.to_string();
        }
        if let Some(cff) = &self.cff
            && let Some(sid) = cff.charset().and_then(|c| c.string_id(GlyphId::new(gid)))
        {
            if cff.is_cid() {
                return format!("cid{}", sid.to_u16());
            }
            if let Some(bytes) = cff.string(sid) {
                return String::from_utf8_lossy(bytes).into_owned();
            }
        }
        format!("gid{gid}")
    }
}

/// `check.hinting.missingStemHints`.
pub(crate) fn missing_stem_hints(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    if !has(ctx, b"CFF ") {
        sink.skip("the font has no CFF table");
        return;
    }
    let glyphs = match cff_glyphs(ctx) {
        Ok(g) => g,
        Err(reason) => {
            sink.skip(reason);
            return;
        }
    };
    let mut namer = Namer::new(ctx);
    for g in glyphs.iter().filter(|g| g.marking && g.h.is_empty() && g.v.is_empty()) {
        sink.push(Hit::new("missing-hints", "The glyph draws an outline but has no stem hints.").place(glyph_at(namer.name(g.gid), "CFF ")));
    }
}

/// `check.hinting.stemHintSanity`.
pub(crate) fn stem_hint_sanity(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    if !has(ctx, b"CFF ") {
        sink.skip("the font has no CFF table");
        return;
    }
    let glyphs = match cff_glyphs(ctx) {
        Ok(g) => g,
        Err(reason) => {
            sink.skip(reason);
            return;
        }
    };
    let mut namer = Namer::new(ctx);
    for g in glyphs.iter().filter(|g| g.marking) {
        let Some([x0, y0, x1, y1]) = g.bbox else { continue };
        // The reference reads integer boxes; rounding outwards keeps hints on a fractional
        // extreme from counting as outside.
        let (x0, y0, x1, y1) = (x0.floor(), y0.floor(), x1.ceil(), y1.ceil());
        let mut problems: Vec<Hit> = Vec::new();
        for (horizontal, stems) in [(true, &g.h), (false, &g.v)] {
            let (min, max) = if horizontal { (y0, y1) } else { (x0, x1) };
            let kind = if horizontal { "horizontal" } else { "vertical" };
            for &(lo, hi) in stems.iter() {
                let width = hi - lo;
                let hit = |code: &'static str, message: String| Hit::new(code, message).data("low", lo).data("high", hi);
                // Ghost hints exist for horizontal stems only: -20 marks a top edge at the lower
                // value, -21 a bottom edge at the upper one.
                let edge = match (horizontal, width) {
                    (true, -20.0) => Some(lo),
                    (true, -21.0) => Some(hi),
                    _ => None,
                };
                if let Some(e) = edge {
                    if e > max || e < min {
                        problems.push(
                            Hit::new(
                                "edge-outside-glyph",
                                format!("The edge hint at {} lies outside the glyph's box ({} to {}).", num(e), num(min), num(max)),
                            )
                            .data("edge", e),
                        );
                    }
                    continue;
                }
                let span = format!("{} to {}", num(lo), num(hi));
                if width < 0.0 {
                    problems.push(hit("negative-width", format!("The {kind} stem hint {span} has a negative width.")));
                }
                if width.abs() > max - min {
                    problems
                        .push(hit("wider-than-glyph", format!("The {kind} stem hint {span} is wider than the glyph's box ({}).", num(max - min))));
                }
                if hi > max {
                    let side = if horizontal { "above the top" } else { "right of the right edge" };
                    problems.push(hit("above-glyph", format!("The {kind} stem hint {span} reaches {side} of the glyph's box ({}).", num(max))));
                }
                if lo < min {
                    let side = if horizontal { "below the bottom" } else { "left of the left edge" };
                    problems.push(hit("below-glyph", format!("The {kind} stem hint {span} reaches {side} of the glyph's box ({}).", num(min))));
                }
            }
        }
        if problems.is_empty() {
            continue;
        }
        let name = namer.name(g.gid);
        for hit in problems {
            sink.push(hit.place(glyph_at(name.clone(), "CFF ")));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// CFF private dictionaries
// ---------------------------------------------------------------------------------------------

/// The hinting values of one Private DICT, as stored (arrays delta-decoded).
#[derive(Debug, Default, PartialEq)]
struct PrivateDict {
    /// The index in the FDArray of a CID-keyed font.
    fd: Option<u32>,
    blue_values: Option<Vec<f64>>,
    other_blues: Option<Vec<f64>>,
    family_blues: Option<Vec<f64>>,
    family_other_blues: Option<Vec<f64>>,
    blue_scale: Option<f64>,
    blue_fuzz: Option<f64>,
    force_bold: bool,
}

impl PrivateDict {
    fn field(&self, key: &str) -> String {
        match self.fd {
            Some(i) => format!("FDArray[{i}].Private.{key}"),
            None => format!("Private.{key}"),
        }
    }

    fn blue_scale(&self) -> f64 {
        self.blue_scale.filter(|v| v.is_finite()).unwrap_or(DEFAULT_BLUE_SCALE)
    }

    fn blue_fuzz(&self) -> f64 {
        self.blue_fuzz.filter(|v| v.is_finite()).unwrap_or(DEFAULT_BLUE_FUZZ)
    }

    /// The four zone arrays with their names and the most values each may hold.
    fn arrays(&self) -> [(&'static str, Option<&[f64]>, usize); 4] {
        [
            ("BlueValues", self.blue_values.as_deref(), 14),
            ("OtherBlues", self.other_blues.as_deref(), 10),
            ("FamilyBlues", self.family_blues.as_deref(), 14),
            ("FamilyOtherBlues", self.family_other_blues.as_deref(), 10),
        ]
    }
}

/// The Private DICTs of the font's `CFF ` table: one, or one per font dictionary of a CID-keyed
/// font.
fn private_dicts(ctx: &BinaryCtx<'_>) -> Result<Vec<PrivateDict>, String> {
    let bytes = data(ctx, b"CFF ").ok_or("the font has no CFF table")?;
    let cff = ctx.font.cff().map_err(|e| format!("the CFF table cannot be read: {e}"))?;
    let top = cff.top_dicts().get(0).ok_or("the CFF table has no Top DICT")?;
    let (mut private, mut fd_array) = (None, None);
    for entry in dict::entries(top, None).flatten() {
        match entry {
            Entry::PrivateDictRange(range) => private = Some(range),
            Entry::FdArrayOffset(offset) => fd_array = Some(offset),
            _ => {}
        }
    }
    let read = |range: Range<usize>, fd: Option<u32>| -> Result<PrivateDict, String> {
        let dict = bytes.get(range).ok_or("a Private DICT lies outside the CFF table")?;
        Ok(parse_private(dict, fd))
    };
    if let Some(offset) = fd_array {
        let index = bytes.get(offset..).and_then(|d| Index::new(d, false).ok()).ok_or("the CFF table's font dictionary array cannot be read")?;
        let mut out = Vec::new();
        for i in 0..index.count().min(MAX_FONT_DICTS) {
            let font_dict = index.get(i as usize).ok_or("a font dictionary cannot be read")?;
            let range = dict::entries(font_dict, None).flatten().find_map(|e| match e {
                Entry::PrivateDictRange(r) => Some(r),
                _ => None,
            });
            match range {
                Some(r) => out.push(read(r, Some(i))?),
                None => out.push(PrivateDict { fd: Some(i), ..PrivateDict::default() }),
            }
        }
        return Ok(out);
    }
    match private {
        Some(r) => Ok(vec![read(r, None)?]),
        None => Ok(vec![PrivateDict::default()]),
    }
}

/// Reads the hinting values of a Private DICT. Malformed data ends the reading; what was read
/// before stays.
fn parse_private(bytes: &[u8], fd: Option<u32>) -> PrivateDict {
    let mut d = PrivateDict { fd, ..PrivateDict::default() };
    let mut stack: Vec<f64> = Vec::new();
    let mut it = bytes.iter().copied();
    let deltas = |stack: &[f64]| -> Vec<f64> {
        let mut sum = 0.0;
        stack
            .iter()
            .map(|v| {
                sum += v;
                sum
            })
            .collect()
    };
    while let Some(b0) = it.next() {
        let operand = match b0 {
            32..=246 => Some(f64::from(b0) - 139.0),
            247..=250 => it.next().map(|b1| (f64::from(b0) - 247.0) * 256.0 + f64::from(b1) + 108.0),
            251..=254 => it.next().map(|b1| -(f64::from(b0) - 251.0) * 256.0 - f64::from(b1) - 108.0),
            28 => match (it.next(), it.next()) {
                (Some(a), Some(b)) => Some(f64::from(i16::from_be_bytes([a, b]))),
                _ => return d,
            },
            29 => match (it.next(), it.next(), it.next(), it.next()) {
                (Some(a), Some(b), Some(c), Some(e)) => Some(f64::from(i32::from_be_bytes([a, b, c, e]))),
                _ => return d,
            },
            30 => Some(read_real(&mut it)),
            _ => None,
        };
        if let Some(v) = operand {
            if stack.len() < MAX_DICT_OPERANDS {
                stack.push(v);
            }
            continue;
        }
        let op = if b0 == 12 {
            match it.next() {
                Some(b1) => 1200 + u16::from(b1),
                None => return d,
            }
        } else {
            u16::from(b0)
        };
        let last = stack.last().copied().filter(|v| v.is_finite());
        match op {
            6 => d.blue_values = Some(deltas(&stack)),
            7 => d.other_blues = Some(deltas(&stack)),
            8 => d.family_blues = Some(deltas(&stack)),
            9 => d.family_other_blues = Some(deltas(&stack)),
            1209 => d.blue_scale = last,
            1211 => d.blue_fuzz = last,
            1214 => d.force_bold = last.is_some_and(|v| v != 0.0),
            _ => {}
        }
        stack.clear();
    }
    d
}

/// A DICT real number: nibbles of digits, point, exponent and sign, ended by 0xF.
fn read_real(it: &mut impl Iterator<Item = u8>) -> f64 {
    let mut text = String::new();
    let mut nibbles = 0usize;
    'bytes: while let Some(b) = it.next() {
        for n in [b >> 4, b & 0x0F] {
            nibbles += 1;
            match n {
                0..=9 => text.push(char::from(b'0' + n)),
                0x0A => text.push('.'),
                0x0B => text.push('e'),
                0x0C => text.push_str("e-"),
                0x0E => text.push('-'),
                0x0F => break 'bytes,
                _ => {}
            }
        }
        if nibbles >= MAX_REAL_NIBBLES {
            // Skip the rest of an over-long number up to its end nibble.
            for b in it.by_ref() {
                if b >> 4 == 0x0F || b & 0x0F == 0x0F {
                    break;
                }
            }
            break;
        }
    }
    text.parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(f64::NAN)
}

/// Runs `each` on every Private DICT, or records why the check cannot run.
fn with_private_dicts(ctx: &BinaryCtx<'_>, sink: &mut Sink, mut each: impl FnMut(&PrivateDict, &mut Sink)) {
    if !has(ctx, b"CFF ") {
        sink.skip("the font has no CFF table");
        return;
    }
    match private_dicts(ctx) {
        Ok(dicts) => {
            for d in &dicts {
                each(d, sink);
            }
        }
        Err(reason) => sink.skip(reason),
    }
}

fn list(values: &[f64]) -> String {
    values.iter().map(|v| num(*v)).collect::<Vec<_>>().join(" ")
}

/// The value pairs of a zone array, each as `(first, second)`; an odd last value is left out.
fn pairs(values: &[f64]) -> impl Iterator<Item = (f64, f64)> + '_ {
    values.chunks_exact(2).filter_map(|c| match c {
        [a, b] => Some((*a, *b)),
        _ => None,
    })
}

/// `check.hinting.forceBold`.
pub(crate) fn force_bold(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    with_private_dicts(ctx, sink, |d, sink| {
        if d.force_bold {
            sink.push(Hit::new("force-bold", "The private dictionary sets ForceBold. It is harmless but obsolete.").place(at(
                ctx,
                "CFF ",
                Some(&d.field("ForceBold")),
            )));
        }
    });
}

/// `check.hinting.blueValuesCount`.
pub(crate) fn blue_values_count(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    with_private_dicts(ctx, sink, |d, sink| {
        let blues = d.blue_values.as_deref().unwrap_or(&[]);
        if blues.len() < 2 || blues == [0.0, 0.0] {
            sink.push(Hit::new("no-blue-values", "The private dictionary has no alignment zones (BlueValues).").place(at(
                ctx,
                "CFF ",
                Some(&d.field("BlueValues")),
            )));
        }
        for (key, values, max) in d.arrays() {
            let Some(values) = values else { continue };
            let place = || at(ctx, "CFF ", Some(&d.field(key)));
            if values.len() > max {
                sink.push(
                    Hit::new("too-many-zones", format!("{key} holds {} zones; at most {} are allowed.", values.len().div_ceil(2), max / 2))
                        .data("values", values.len() as f64)
                        .place(place()),
                );
            }
            if values.len() % 2 != 0 {
                sink.push(
                    Hit::new("odd-count", format!("{key} holds {} values ({}); zones need them in pairs.", values.len(), list(values)))
                        .data("values", values.len() as f64)
                        .place(place()),
                );
            }
        }
    });
}

/// `check.hinting.blueValuesOrder`.
pub(crate) fn blue_values_order(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    with_private_dicts(ctx, sink, |d, sink| {
        for (key, values, _) in d.arrays() {
            for (a, b) in pairs(values.unwrap_or(&[])) {
                if a > b {
                    sink.push(
                        Hit::new("unordered-pair", format!("The {key} pair {} {} has its larger value first.", num(a), num(b)))
                            .data("first", a)
                            .data("second", b)
                            .place(at(ctx, "CFF ", Some(&d.field(key)))),
                    );
                }
            }
        }
    });
}

/// `check.hinting.blueZoneSeparation`.
pub(crate) fn blue_zone_separation(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    with_private_dicts(ctx, sink, |d, sink| {
        let blues = d.blue_values.as_deref().unwrap_or(&[]);
        let span = |(a, b): (f64, f64)| (a.min(b), a.max(b));
        // The first BlueValues pair is the baseline zone, a bottom zone like every OtherBlues pair.
        let bottom: Vec<(f64, f64)> = pairs(blues).take(1).chain(pairs(d.other_blues.as_deref().unwrap_or(&[]))).map(span).collect();
        let top: Vec<(f64, f64)> = pairs(blues).skip(1).map(span).collect();
        let need = 2.0 * d.blue_fuzz() + 1.0;
        for (side, zones) in [("bottom", &bottom), ("top", &top)] {
            for (i, a) in zones.iter().enumerate() {
                for b in zones.iter().skip(i + 1) {
                    let gap = (b.0 - a.1).max(a.0 - b.1);
                    if gap < need {
                        sink.push(
                            Hit::new(
                                "zones-too-close",
                                format!(
                                    "The {side} zones {} {} and {} {} are {} units apart; they need at least {} (2 × BlueFuzz + 1).",
                                    num(a.0),
                                    num(a.1),
                                    num(b.0),
                                    num(b.1),
                                    num(gap),
                                    num(need)
                                ),
                            )
                            .data("gap", gap)
                            .data("needed", need)
                            .place(at(
                                ctx,
                                "CFF ",
                                Some(&d.field(if side == "top" { "BlueValues" } else { "OtherBlues" })),
                            )),
                        );
                    }
                }
            }
        }
    });
}

/// `check.hinting.blueZoneHeight`.
pub(crate) fn blue_zone_height(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    with_private_dicts(ctx, sink, |d, sink| {
        let scale = d.blue_scale();
        for (key, values, _) in d.arrays() {
            for (a, b) in pairs(values.unwrap_or(&[])) {
                let height = (a - b).abs();
                if scale > 0.0 && height * scale >= 1.0 {
                    sink.push(
                        Hit::new(
                            "zone-too-tall",
                            format!(
                                "The {key} zone {} {} is {} units tall; with BlueScale {} a zone must be lower than {} units.",
                                num(a),
                                num(b),
                                num(height),
                                scale,
                                num(1.0 / scale)
                            ),
                        )
                        .data("height", height)
                        .data("maximum", 1.0 / scale)
                        .place(at(ctx, "CFF ", Some(&d.field(key)))),
                    );
                }
            }
        }
    });
}

/// `check.hinting.blueZonesOutsideBBox`.
pub(crate) fn blue_zones_outside_bbox(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Ok(head) = ctx.font.head() else {
        sink.skip("the font's 'head' table cannot be read");
        return;
    };
    let (y_min, y_max) = (f64::from(head.y_min()), f64::from(head.y_max()));
    with_private_dicts(ctx, sink, |d, sink| {
        for key in ["BlueValues", "OtherBlues"] {
            let values = if key == "BlueValues" { d.blue_values.as_deref() } else { d.other_blues.as_deref() };
            let mut sorted: Vec<f64> = values.unwrap_or(&[]).iter().copied().filter(|v| v.is_finite()).collect();
            if sorted.len() < 2 {
                continue;
            }
            sorted.sort_by(f64::total_cmp);
            let place = || at(ctx, "CFF ", Some(&d.field(key)));
            if let (Some(&low), Some(&low_top)) = (sorted.first(), sorted.get(1))
                && low_top < y_min
            {
                sink.push(
                    Hit::new(
                        "below-bbox",
                        format!(
                            "The lowest {key} zone {} {} lies below the font's bounding box (lowest point {}).",
                            num(low),
                            num(low_top),
                            num(y_min)
                        ),
                    )
                    .data("zone", low_top)
                    .data("yMin", y_min)
                    .place(place()),
                );
            }
            let n = sorted.len();
            if let (Some(&high_bottom), Some(&high)) = (sorted.get(n.saturating_sub(2)), sorted.get(n.saturating_sub(1)))
                && high_bottom > y_max
            {
                sink.push(
                    Hit::new(
                        "above-bbox",
                        format!(
                            "The highest {key} zone {} {} lies above the font's bounding box (highest point {}).",
                            num(high_bottom),
                            num(high),
                            num(y_max)
                        ),
                    )
                    .data("zone", high_bottom)
                    .data("yMax", y_max)
                    .place(place()),
                );
            }
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------------------------

fn master_at(master: &Master) -> Location {
    Location { master: Some(master.id.clone()), ..Location::default() }
}

/// The heights of a master's metric lines, the baseline included; the italic angle is not one.
fn metric_lines(font: &Font, master: &Master) -> Vec<f64> {
    font.metrics
        .iter()
        .zip(master.metric_values.iter())
        .filter(|(d, _)| d.kind != metric::ITALIC_ANGLE)
        .map(|(_, v)| v.pos)
        .chain([0.0])
        .filter(|v| v.is_finite())
        .collect()
}

/// `check.hinting.zonesOverlapInSource`.
pub(crate) fn zones_overlap_in_source(ctx: &SourceCtx<'_>, params: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    let min_gap = params.get(&MIN_ZONE_GAP);
    for master in font.masters.iter() {
        let zones: Vec<(f64, f64)> = font
            .alignment_zones(master)
            .iter()
            .filter(|z| z.pos.is_finite() && z.size.is_finite())
            .take(MAX_MASTER_ZONES)
            .map(|z| (z.pos.min(z.pos + z.size), z.pos.max(z.pos + z.size)))
            .collect();
        let lines = metric_lines(font, master);
        for (i, a) in zones.iter().enumerate() {
            for b in zones.iter().skip(i + 1) {
                let gap = (b.0 - a.1).max(a.0 - b.1);
                if gap < min_gap {
                    let how = if gap < 0.0 { "overlap" } else { "touch or lie closer than the minimum gap" };
                    sink.push(
                        Hit::new(
                            "zones-overlap",
                            format!(
                                "In master {}, the alignment zones {} to {} and {} to {} {how}.",
                                master.name,
                                num(a.0),
                                num(a.1),
                                num(b.0),
                                num(b.1)
                            ),
                        )
                        .data("gap", gap)
                        .place(master_at(master)),
                    );
                }
            }
            if !lines.iter().any(|p| *p >= a.0 - 1e-9 && *p <= a.1 + 1e-9) {
                sink.push(
                    Hit::new(
                        "zone-without-metric",
                        format!("In master {}, the alignment zone {} to {} contains no metric line.", master.name, num(a.0), num(a.1)),
                    )
                    .data("bottom", a.0)
                    .data("top", a.1)
                    .place(master_at(master)),
                );
            }
        }
    }
}

/// `check.hinting.stemsUndefined`.
pub(crate) fn stems_undefined(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    let defined = |widths: Vec<f64>| widths.iter().any(|w| w.is_finite() && *w > 0.0);
    for master in font.masters.iter() {
        if !defined(font.stem_widths(master, true)) {
            sink.push(
                Hit::new(
                    "no-horizontal-stems",
                    format!("Master {} defines no horizontal standard stem (crossbars, serifs), so the autohinter has no width to normalise horizontal stems to.", master.name),
                )
                .place(master_at(master)),
            );
        }
        if !defined(font.stem_widths(master, false)) {
            sink.push(
                Hit::new(
                    "no-vertical-stems",
                    format!(
                        "Master {} defines no vertical standard stem, so the autohinter has no width to normalise vertical stems to.",
                        master.name
                    ),
                )
                .place(master_at(master)),
            );
        }
    }
}

#[cfg(test)]
mod tests;
