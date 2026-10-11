//! The metrics report (M12.16; rows `metrics`, `font.reportMetrics`). Oracles: `tx -mtx`,
//! `tx -afm`, fontTools `pens.statisticsPen`.
//!
//! What is measured, per glyph in glyph order:
//!
//! - the advance width from `hmtx` (the charstring width when the font has no `hmtx`);
//! - the exact bounding box of the outline: curve extremes count, control points do not, and
//!   the values stored in `head`, `glyf` or the CFF Top DICT are never read;
//! - the side bearings derived from that box;
//! - with `statistics`, the shape statistics of the filled outline.
//!
//! Outlines are read as the reference tools draw them. TrueType contours start at their first
//! on-curve point, implied on-curve points sit halfway between two off-curve points, and
//! composite glyphs are drawn with their components placed and transformed. CFF and CFF2
//! charstrings are drawn in charstring units (the font matrix is not applied, as neither oracle
//! applies it); a moveto that draws nothing adds nothing. Every contour is closed by a line back
//! to its start when it does not end there.
//!
//! The statistics follow fontTools' `pens.statisticsPen` and `pens.momentsPen` (MIT, Copyright
//! Just van Rossum and the fontTools authors): area and the first and second moments from
//! Green's theorem, the mean, variance, standard deviation, covariance, correlation and slant
//! derived from them with the same thresholds, the control-polygon variant, and the font-wide
//! weight, width and slant estimates. The moments are integrated here with Gauss–Legendre
//! quadrature, exact for the polynomial integrands of line, quadratic and cubic segments. Values
//! are in font units (the oracle scales to the em first); the area is negative when the outer
//! contours run clockwise, as in TrueType fonts.
//!
//! The kerning list flattens the pair adjustments of the GPOS `kern` feature: lookups apply in
//! lookup-list order and their values add up; inside one lookup the first subtable that applies
//! to a pair decides it. When GPOS has no `kern` feature, the horizontal format 0 subtables of
//! the `kern` table are read instead. Only pairs whose two glyphs are measured are listed.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

use read_fonts::ps::cff::CffFontRef;
use read_fonts::ps::cs::CommandSink;
use read_fonts::tables::glyf::{Anchor, CompositeGlyphFlags, Glyf, Glyph};
use read_fonts::tables::gpos::{PairPos, PairPosFormat1, PairPosFormat2, PositionSubtables};
use read_fonts::tables::kern::SubtableKind;
use read_fonts::tables::loca::Loca;
use read_fonts::types::{Fixed, GlyphId, GlyphId16, Tag as RTag};
use read_fonts::{FontData, TableProvider};

use crate::limits::MAX_GLYPHS;
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// Most kerning pairs one report lists.
pub const MAX_KERN_PAIRS: usize = 2_000_000;
/// Most steps spent flattening kerning (coverage entries, records, class pairs).
pub const MAX_KERN_STEPS: usize = 50_000_000;
/// Deepest nesting of composite glyphs followed.
pub const MAX_COMPONENT_DEPTH: usize = 64;
/// Most points, segments and component placements one report reads, over every glyph.
pub const MAX_OUTLINE_WORK: usize = 200_000_000;
/// Most cmap entries read.
pub const MAX_CMAP_ENTRIES: usize = 2 * crate::limits::MAX_CODE_POINTS;
/// Most items one glyph selector holds.
pub const MAX_SELECTOR_ITEMS: usize = MAX_GLYPHS;

/// What [`report`] measures.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MetricsOptions {
    /// Glyphs to measure, as glyph-list items (`cli.global.glyphSelector`: names, glyph ids,
    /// `gidN`, `/CID`, `cidN`, `U+XXXX`, ranges `A..Z`, `N-M`, `U+XXXX-YYYY`, `*`); an item may
    /// hold several, separated by commas. Empty measures every glyph.
    pub glyphs: Vec<String>,
    /// Glyphs left out, in the same grammar; `.notdef` is never left out.
    pub exclude_glyphs: Vec<String>,
    /// Font dictionaries of a CID-keyed or CFF2 font to keep; empty keeps all.
    pub fd: Vec<u16>,
    /// Include the kerning pairs.
    pub kerning: bool,
    /// Include area and moments.
    pub statistics: bool,
    /// Compute the statistics from the control polygon instead of the curves.
    pub control_box: bool,
}

/// One glyph's metrics in font units.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphMetrics {
    pub name: String,
    pub gid: u32,
    /// The lowest code point the cmap maps to the glyph.
    pub unicode: Option<u32>,
    pub advance: f64,
    /// `[x_min, y_min, x_max, y_max]`; `None` for an empty glyph.
    pub bounds: Option<[f64; 4]>,
    pub left_side_bearing: f64,
    pub right_side_bearing: f64,
    pub area: Option<f64>,
    /// Centre of mass, `[x, y]`.
    pub centroid: Option<[f64; 2]>,
    /// Every shape statistic, with `statistics`.
    pub statistics: Option<ShapeStatistics>,
}

/// The shape statistics of one glyph.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShapeStatistics {
    /// Signed: negative when the outer contours run clockwise.
    pub area: f64,
    pub mean: [f64; 2],
    pub variance: [f64; 2],
    pub stddev: [f64; 2],
    pub covariance: f64,
    /// In [-1, 1]; 0 when undefined or below 0.001 in magnitude.
    pub correlation: f64,
    /// Horizontal shift per unit of height; 0 when undefined or below 0.001 in magnitude.
    pub slant: f64,
}

/// One kerning pair, by glyph name.
#[derive(Clone, Debug, PartialEq)]
pub struct KernPair {
    pub left: String,
    pub right: String,
    pub value: f64,
}

/// The box around every measured glyph, and the glyphs that set its edges.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontBounds {
    /// `[x_min, y_min, x_max, y_max]`.
    pub bounds: [f64; 4],
    /// For each edge, the first glyph in glyph order that reaches it.
    pub gids: [u32; 4],
    pub names: [String; 4],
}

/// The font-wide estimates of the statistics, as fontTools' statistics pen prints them. All are
/// independent of the units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FontStatistics {
    /// Ink area per em of advance: the summed absolute glyph areas over (em × summed advances).
    pub weight: f64,
    /// The signed areas weighted by advance, over the summed advances, per em².
    pub weight_perceptual: f64,
    /// The mean advance in ems.
    pub width: f64,
    /// The mean glyph slant.
    pub slant: f64,
    /// `-atan(slant)` in degrees.
    pub slant_angle: f64,
    /// The glyph slants weighted by advance.
    pub slant_perceptual: f64,
    pub slant_perceptual_angle: f64,
}

/// Names and `post` values an AFM file carries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontInfo {
    pub postscript_name: String,
    pub full_name: String,
    pub family_name: String,
    pub version: String,
    pub notice: String,
    pub italic_angle: f64,
    pub fixed_pitch: bool,
    pub underline_position: f64,
    pub underline_thickness: f64,
}

/// The report.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetricsReport {
    pub units_per_em: u16,
    /// The em the values are expressed in: the units per em, or 1000 after [`MetricsReport::rescaled`].
    pub units: f64,
    pub info: FontInfo,
    pub glyphs: Vec<GlyphMetrics>,
    /// `None` when no measured glyph has an outline.
    pub font_bounds: Option<FontBounds>,
    pub kerning: Vec<KernPair>,
    /// With `statistics`; `None` when no glyph was measured or every advance is 0.
    pub font_statistics: Option<FontStatistics>,
    /// What could not be read; the rest of the report stands.
    pub warnings: Vec<String>,
}

/// Measures the font.
pub fn report(font: &SfntFont, options: &MetricsOptions) -> Result<MetricsReport> {
    let p = Provider(font);
    let head = p.head().map_err(|e| TablesError::invalid(format!("the 'head' table cannot be read: {e}")))?;
    let upem = head.units_per_em();
    if upem == 0 {
        return Err(TablesError::invalid("the units per em are 0"));
    }
    let mut warnings = Vec::new();
    let cff_data = font.get(*b"CFF ").or_else(|| font.get(*b"CFF2"));
    let cff = match cff_data {
        Some(data) => {
            Some(CffFontRef::new(data, 0, Some(i32::from(upem))).map_err(|e| TablesError::invalid(format!("the CFF data cannot be read: {e}")))?)
        }
        None => None,
    };
    let count = glyph_count(&p, cff.as_ref())?;
    let outlines = Outlines::new(&p, cff.as_ref(), &mut warnings);
    let codes = code_points(&p, count, &mut warnings);
    let lowest = lowest_codes(&codes, count);
    let (names, cids) = glyph_names(&p, cff.as_ref(), count, &lowest);
    let selected = select(options, &names, &cids, &codes, cff.as_ref(), count)?;
    let hmtx = p.hmtx().ok();

    let mut work = Work { left: MAX_OUTLINE_WORK };
    let mut glyphs = Vec::with_capacity(selected.len());
    let mut sums = FontSums::default();
    for &gid in &selected {
        let name = names.get(gid as usize).cloned().unwrap_or_default();
        let (contours, cff_width) = match outlines.draw(gid, &mut work) {
            Ok(drawn) => drawn,
            Err(e @ TablesError::TooLarge { .. }) => return Err(e),
            Err(e) => {
                warnings.push(format!("glyph {name}: {e}"));
                (Vec::new(), None)
            }
        };
        work.spend(contours.iter().map(|c| c.segs.len() + 1).sum())?;
        let advance = hmtx.as_ref().and_then(|h| h.advance(GlyphId::new(gid))).map(f64::from).or(cff_width).unwrap_or(0.0);
        let bounds = exact_bounds(&contours);
        let (lsb, rsb) = match bounds {
            Some([x_min, _, x_max, _]) => (x_min, advance - x_max),
            None => (0.0, advance),
        };
        let statistics = options.statistics.then(|| if options.control_box { control_statistics(&contours) } else { moment_statistics(&contours) });
        if let Some(s) = statistics {
            sums.add(&s, advance);
        }
        glyphs.push(GlyphMetrics {
            name,
            gid,
            unicode: lowest.get(gid as usize).copied().flatten(),
            advance,
            bounds,
            left_side_bearing: lsb,
            right_side_bearing: rsb,
            area: statistics.map(|s| s.area),
            centroid: statistics.map(|s| s.mean),
            statistics,
        });
    }
    let font_bounds = font_bounds(&glyphs);
    let font_statistics = if options.statistics { sums.finish(f64::from(upem)) } else { None };
    let kerning = if options.kerning { kerning(&p, count, &selected, &names, &mut warnings) } else { Vec::new() };
    let info = font_info(&p, f64::from(head.font_revision().to_bits()) / 65536.0, cff.as_ref());
    Ok(MetricsReport { units_per_em: upem, units: f64::from(upem), info, glyphs, font_bounds, kerning, font_statistics, warnings })
}

impl MetricsReport {
    /// The report with every length rescaled to an em of `units` (1000 for `--units 1000`); areas
    /// and second moments scale with its square, ratios stay as they are.
    pub fn rescaled(&self, units: f64) -> MetricsReport {
        let s = if self.units > 0.0 && units.is_finite() && units > 0.0 { units / self.units } else { 1.0 };
        let s2 = s * s;
        let mut out = self.clone();
        out.units = if s == 1.0 { self.units } else { units };
        for g in &mut out.glyphs {
            g.advance *= s;
            g.bounds = g.bounds.map(|b| b.map(|v| v * s));
            g.left_side_bearing *= s;
            g.right_side_bearing *= s;
            g.area = g.area.map(|a| a * s2);
            g.centroid = g.centroid.map(|c| c.map(|v| v * s));
            if let Some(st) = &mut g.statistics {
                st.area *= s2;
                st.mean = st.mean.map(|v| v * s);
                st.variance = st.variance.map(|v| v * s2);
                st.stddev = st.stddev.map(|v| v * s);
                st.covariance *= s2;
            }
        }
        if let Some(fb) = &mut out.font_bounds {
            fb.bounds = fb.bounds.map(|v| v * s);
        }
        for k in &mut out.kerning {
            k.value *= s;
        }
        out.info.underline_position *= s;
        out.info.underline_thickness *= s;
        out
    }
}

// ── Tables ──────────────────────────────────────────────────────────────────────────────────────

/// The font's tables as read-fonts asks for them.
struct Provider<'a>(&'a SfntFont);

impl<'a> TableProvider<'a> for Provider<'a> {
    fn data_for_tag(&self, tag: RTag) -> Option<FontData<'a>> {
        self.0.get(tag.into_bytes()).map(FontData::new)
    }
}

fn glyph_count(p: &Provider<'_>, cff: Option<&CffFontRef<'_>>) -> Result<usize> {
    let count = match p.maxp() {
        Ok(maxp) => usize::from(maxp.num_glyphs()),
        Err(_) => match cff {
            Some(cff) => cff.num_glyphs() as usize,
            None => return Err(TablesError::MissingTable("maxp".into())),
        },
    };
    if count > MAX_GLYPHS {
        return Err(TablesError::TooLarge { what: "the glyph count", limit: MAX_GLYPHS });
    }
    Ok(count)
}

/// Every (code point, glyph) pair of the preferred Unicode cmap subtable.
fn code_points(p: &Provider<'_>, count: usize, warnings: &mut Vec<String>) -> Vec<(u32, u32)> {
    let Ok(cmap) = p.cmap() else { return Vec::new() };
    let Some((_, _, subtable)) = cmap.best_subtable() else { return Vec::new() };
    let limits = read_fonts::tables::cmap::CmapIterLimits { max_char: 0x10_FFFF, glyph_count: u32::try_from(count).unwrap_or(u32::MAX) };
    let mut out = Vec::new();
    for (code, gid) in subtable.iter_with_limits(limits) {
        if out.len() >= MAX_CMAP_ENTRIES {
            warnings.push(format!("the cmap was read only in part: it maps more than {MAX_CMAP_ENTRIES} code points"));
            break;
        }
        if (gid.to_u32() as usize) < count {
            out.push((code, gid.to_u32()));
        }
    }
    out
}

fn lowest_codes(codes: &[(u32, u32)], count: usize) -> Vec<Option<u32>> {
    let mut lowest = vec![None; count];
    for &(code, gid) in codes {
        if let Some(slot) = lowest.get_mut(gid as usize) {
            if slot.is_none_or(|c: u32| code < c) {
                *slot = Some(code);
            }
        }
    }
    lowest
}

/// Glyph names from `post`, else the CFF charset, else the code points; each made unique, as
/// fontTools does, with `#1`, `#2`… Also the CID of each glyph of a CID-keyed font.
fn glyph_names(p: &Provider<'_>, cff: Option<&CffFontRef<'_>>, count: usize, lowest: &[Option<u32>]) -> (Vec<String>, Vec<Option<u32>>) {
    let post = p.post().ok();
    let charset = cff.and_then(|c| c.charset());
    let cid_keyed = cff.is_some_and(|c| c.is_cid());
    let mut names = Vec::with_capacity(count);
    let mut cids = Vec::with_capacity(count);
    let mut seen: HashMap<String, u32> = HashMap::new();
    for gid in 0..count {
        let sid = charset.as_ref().and_then(|cs| cs.string_id(GlyphId::new(gid as u32)));
        cids.push(if cid_keyed { sid.map(|s| u32::from(s.to_u16())) } else { None });
        let from_post = u16::try_from(gid).ok().and_then(|g| post.as_ref().and_then(|t| t.glyph_name(GlyphId16::new(g)))).map(str::to_string);
        let from_cff = || match (cff, sid) {
            (Some(_), Some(s)) if cid_keyed => Some(format!("cid{:05}", s.to_u16())),
            (Some(c), Some(s)) => c.string(s).map(|b| String::from_utf8_lossy(b).into_owned()),
            _ => None,
        };
        let from_code = || lowest.get(gid).copied().flatten().map(|c| if c <= 0xFFFF { format!("uni{c:04X}") } else { format!("u{c:05X}") });
        let base = if gid == 0 {
            ".notdef".to_string()
        } else {
            from_post
                .filter(|n| !n.is_empty())
                .or_else(from_cff)
                .filter(|n| !n.is_empty())
                .or_else(from_code)
                .unwrap_or_else(|| format!("glyph{gid:05}"))
        };
        let base: String = base.chars().filter(|c| !c.is_control() && !c.is_whitespace()).take(255).collect();
        let name = match seen.get_mut(&base) {
            Some(n) => {
                *n += 1;
                format!("{base}#{n}")
            }
            None => {
                seen.insert(base.clone(), 0);
                base
            }
        };
        names.push(name);
    }
    (names, cids)
}

fn font_info(p: &Provider<'_>, revision: f64, cff: Option<&CffFontRef<'_>>) -> FontInfo {
    let mut info = FontInfo::default();
    if let Ok(name) = p.name() {
        let data = name.string_data();
        let mut best: BTreeMap<u16, (u8, String)> = BTreeMap::new();
        for rec in name.name_record() {
            let id = rec.name_id().to_u16();
            if !matches!(id, 0 | 1 | 4 | 5 | 6) {
                continue;
            }
            let rank = match (rec.platform_id(), rec.encoding_id(), rec.language_id()) {
                (3, 1, 0x409) => 0,
                (3, _, _) => 1,
                (0, _, _) => 2,
                (1, 0, 0) => 3,
                _ => continue,
            };
            if best.get(&id).is_some_and(|(r, _)| *r <= rank) {
                continue;
            }
            if let Ok(s) = rec.string(data) {
                let text: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(1024).collect();
                best.insert(id, (rank, text.trim().to_string()));
            }
        }
        let get = |id: u16| best.get(&id).map(|(_, s)| s.clone()).unwrap_or_default();
        info.notice = get(0);
        info.family_name = get(1);
        info.full_name = get(4);
        info.version = get(5);
        info.postscript_name = get(6);
    }
    if info.postscript_name.is_empty() {
        if let Some(name) = cff.and_then(|c| c.metadata()).and_then(|m| m.name()) {
            info.postscript_name = name.chars().filter(|c| !c.is_control()).take(127).collect();
        }
    }
    let version = info.version.trim_start_matches("Version").trim_start_matches("version").trim();
    let version = version.split(';').next().unwrap_or_default().trim().to_string();
    info.version = if version.is_empty() { format!("{revision:.3}") } else { version };
    if let Ok(post) = p.post() {
        info.italic_angle = f64::from(post.italic_angle().to_bits()) / 65536.0;
        info.fixed_pitch = post.is_fixed_pitch() != 0;
        info.underline_position = f64::from(post.underline_position().to_i16());
        info.underline_thickness = f64::from(post.underline_thickness().to_i16());
    }
    info
}

// ── Glyph selection ─────────────────────────────────────────────────────────────────────────────

fn select(
    options: &MetricsOptions,
    names: &[String],
    cids: &[Option<u32>],
    codes: &[(u32, u32)],
    cff: Option<&CffFontRef<'_>>,
    count: usize,
) -> Result<Vec<u32>> {
    let by_name: HashMap<&str, u32> = names.iter().enumerate().map(|(i, n)| (n.as_str(), i as u32)).collect();
    let ctx = SelectContext { names, by_name, cids, codes, count };
    let mut chosen = if options.glyphs.is_empty() { (0..count as u32).collect() } else { ctx.resolve(&options.glyphs)? };
    if !options.exclude_glyphs.is_empty() {
        for gid in ctx.resolve(&options.exclude_glyphs)? {
            if gid != 0 {
                chosen.remove(&gid);
            }
        }
    }
    if !options.fd.is_empty() {
        let Some(cff) = cff else { return Err(TablesError::BadOptions("font dictionaries can only be chosen in a CFF or CFF2 font".into())) };
        chosen.retain(|&gid| cff.subfont_index(GlyphId::new(gid)).is_some_and(|fd| options.fd.contains(&fd)));
    }
    Ok(chosen.into_iter().collect())
}

struct SelectContext<'a> {
    names: &'a [String],
    by_name: HashMap<&'a str, u32>,
    cids: &'a [Option<u32>],
    codes: &'a [(u32, u32)],
    count: usize,
}

impl SelectContext<'_> {
    fn resolve(&self, items: &[String]) -> Result<BTreeSet<u32>> {
        let mut out = BTreeSet::new();
        let mut seen = 0usize;
        for item in items.iter().flat_map(|i| i.split(',')).map(str::trim).filter(|i| !i.is_empty()) {
            seen += 1;
            if seen > MAX_SELECTOR_ITEMS {
                return Err(TablesError::TooLarge { what: "the glyph list", limit: MAX_SELECTOR_ITEMS });
            }
            self.item(item, &mut out)?;
        }
        Ok(out)
    }

    fn item(&self, item: &str, out: &mut BTreeSet<u32>) -> Result<()> {
        let bad = |why: String| Err(TablesError::BadOptions(why));
        if item == "*" {
            out.extend(0..self.count as u32);
            return Ok(());
        }
        if item.starts_with("re:") {
            return Err(TablesError::Unsupported("regular expressions in glyph lists".into()));
        }
        if let Some(hex) = item.strip_prefix("U+").or_else(|| item.strip_prefix("u+")) {
            let (lo, hi) = match hex.split_once('-') {
                Some((a, b)) => (parse_hex(a), parse_hex(b.trim_start_matches("U+").trim_start_matches("u+"))),
                None => (parse_hex(hex), parse_hex(hex)),
            };
            let (Some(lo), Some(hi)) = (lo, hi) else { return bad(format!("`{item}` is not a code point or code point range")) };
            out.extend(self.codes.iter().filter(|(c, _)| (lo..=hi).contains(c)).map(|&(_, g)| g));
            return Ok(());
        }
        let cid = item.strip_prefix('/').or_else(|| item.strip_prefix("cid").filter(|r| r.bytes().all(|b| b.is_ascii_digit()) && !r.is_empty()));
        if let Some(rest) = cid {
            let Some((lo, hi)) = int_range(rest) else { return bad(format!("`{item}` is not a CID or CID range")) };
            out.extend(self.cids.iter().enumerate().filter(|(_, c)| c.is_some_and(|c| (lo..=hi).contains(&c))).map(|(g, _)| g as u32));
            return Ok(());
        }
        let gid = item.strip_prefix("gid").filter(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit())).or_else(|| {
            let digits = item.bytes().all(|b| b.is_ascii_digit() || b == b'-') && item.bytes().next().is_some_and(|b| b.is_ascii_digit());
            digits.then_some(item)
        });
        if let Some(rest) = gid {
            let Some((lo, hi)) = int_range(rest) else { return bad(format!("`{item}` is not a glyph id or glyph id range")) };
            if hi as usize >= self.count {
                return bad(format!("glyph id {hi} is past the last glyph ({})", self.count.saturating_sub(1)));
            }
            out.extend(lo..=hi);
            return Ok(());
        }
        if let Some((a, b)) = item.split_once("..") {
            let (Some(&lo), Some(&hi)) = (self.by_name.get(a), self.by_name.get(b)) else {
                return bad(format!("`{item}`: both ends of a range must be glyph names of the font"));
            };
            out.extend(lo.min(hi)..=lo.max(hi));
            return Ok(());
        }
        match self.by_name.get(item) {
            Some(&g) => {
                out.insert(g);
                Ok(())
            }
            None => bad(format!("the font has no glyph named `{item}`{}", if self.names.is_empty() { " (it has no glyphs)" } else { "" })),
        }
    }
}

fn parse_hex(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 6 {
        return None;
    }
    u32::from_str_radix(s, 16).ok().filter(|&c| c <= 0x10_FFFF)
}

fn int_range(s: &str) -> Option<(u32, u32)> {
    let (a, b) = s.split_once('-').unwrap_or((s, s));
    let (a, b) = (a.parse::<u32>().ok()?, b.parse::<u32>().ok()?);
    Some((a.min(b), a.max(b)))
}

// ── Outlines ────────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Pt {
    pub x: f64,
    pub y: f64,
}

fn pt(x: f64, y: f64) -> Pt {
    Pt { x, y }
}

fn mid(a: Pt, b: Pt) -> Pt {
    pt((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Seg {
    Line(Pt, Pt),
    Quad(Pt, Pt, Pt),
    Cubic(Pt, Pt, Pt, Pt),
}

/// One closed contour: its start and its segments, the last one ending at the start.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Contour {
    pub start: Pt,
    pub segs: Vec<Seg>,
}

/// A TrueType point, after component placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TtPoint {
    pub x: f64,
    pub y: f64,
    pub on: bool,
}

struct Work {
    left: usize,
}

impl Work {
    fn spend(&mut self, n: usize) -> Result<()> {
        if n > self.left {
            self.left = 0;
            return Err(TablesError::TooLarge { what: "the outline data measured", limit: MAX_OUTLINE_WORK });
        }
        self.left -= n;
        Ok(())
    }
}

enum Outlines<'a> {
    Glyf(Loca<'a>, Glyf<'a>),
    Cff(&'a CffFontRef<'a>),
    None,
}

impl<'a> Outlines<'a> {
    fn new(p: &Provider<'a>, cff: Option<&'a CffFontRef<'a>>, warnings: &mut Vec<String>) -> Outlines<'a> {
        if p.0.get(*b"glyf").is_some() {
            match (p.loca(None), p.glyf()) {
                (Ok(loca), Ok(glyf)) => return Outlines::Glyf(loca, glyf),
                _ => warnings.push("the 'glyf' or 'loca' table cannot be read; outlines are left out".into()),
            }
        }
        match cff {
            Some(cff) => Outlines::Cff(cff),
            None => Outlines::None,
        }
    }

    /// The glyph's contours, and the width its charstring states.
    fn draw(&self, gid: u32, work: &mut Work) -> Result<(Vec<Contour>, Option<f64>)> {
        match self {
            Outlines::Glyf(loca, glyf) => {
                let contours = tt_points(loca, glyf, gid, 0, work)?;
                Ok((contours.iter().filter_map(|c| tt_contour(c)).collect(), None))
            }
            Outlines::Cff(cff) => {
                let glyph = GlyphId::new(gid);
                let fd = cff.subfont_index(glyph).unwrap_or(0);
                let sub = cff.subfont(fd, &[]).map_err(|e| TablesError::invalid(format!("its font dictionary cannot be read: {e}")))?;
                let mut sink = CffSink::default();
                let width = cff
                    .evaluate_charstring(&sub, glyph, &[], &mut sink)
                    .map_err(|e| TablesError::invalid(format!("its charstring cannot be read: {e}")))?;
                sink.close();
                Ok((sink.contours, width.map(fixed)))
            }
            Outlines::None => Ok((Vec::new(), None)),
        }
    }
}

fn fixed(v: Fixed) -> f64 {
    f64::from(v.to_bits()) / 65536.0
}

/// The glyph's points, contour by contour, with every component placed.
fn tt_points(loca: &Loca<'_>, glyf: &Glyf<'_>, gid: u32, depth: usize, work: &mut Work) -> Result<Vec<Vec<TtPoint>>> {
    if depth > MAX_COMPONENT_DEPTH {
        return Err(TablesError::invalid(format!("its components nest deeper than {MAX_COMPONENT_DEPTH} levels")));
    }
    let glyph = loca.get_glyf(GlyphId::new(gid), glyf).map_err(|e| TablesError::invalid(format!("glyph {gid} cannot be read: {e}")))?;
    match glyph {
        None => Ok(Vec::new()),
        Some(Glyph::Simple(simple)) => {
            work.spend(simple.num_points() + 1)?;
            let points: Vec<TtPoint> = simple.points().map(|p| TtPoint { x: f64::from(p.x), y: f64::from(p.y), on: p.on_curve }).collect();
            let mut contours = Vec::new();
            let mut first = 0usize;
            for end in simple.end_pts_of_contours() {
                let end = usize::from(end.get());
                let slice = (end >= first)
                    .then(|| points.get(first..=end))
                    .flatten()
                    .ok_or_else(|| TablesError::invalid(format!("glyph {gid} has contour ends out of order")))?;
                contours.push(slice.to_vec());
                first = end + 1;
            }
            Ok(contours)
        }
        Some(Glyph::Composite(composite)) => {
            let mut out: Vec<Vec<TtPoint>> = Vec::new();
            for c in composite.components() {
                work.spend(1)?;
                let child = tt_points(loca, glyf, u32::from(c.glyph.to_u16()), depth + 1, work)?;
                let t = &c.transform;
                let [xx, yx, xy, yy] = [t.xx, t.yx, t.xy, t.yy].map(|v| f64::from(v.to_bits()) / 16384.0);
                let apply = |x: f64, y: f64| (xx * x + xy * y, yx * x + yy * y);
                let mut placed: Vec<Vec<TtPoint>> = child
                    .into_iter()
                    .map(|contour| {
                        contour
                            .into_iter()
                            .map(|p| {
                                let (x, y) = apply(p.x, p.y);
                                TtPoint { x, y, on: p.on }
                            })
                            .collect()
                    })
                    .collect();
                let (dx, dy) = match c.anchor {
                    Anchor::Offset { x, y } => {
                        let (x, y) = (f64::from(x), f64::from(y));
                        let scaled = CompositeGlyphFlags::SCALED_COMPONENT_OFFSET;
                        if c.flags & (scaled | CompositeGlyphFlags::UNSCALED_COMPONENT_OFFSET) == scaled { apply(x, y) } else { (x, y) }
                    }
                    Anchor::Point { base, component } => {
                        let parent = out.iter().flatten().nth(usize::from(base));
                        let own = placed.iter().flatten().nth(usize::from(component));
                        match (parent, own) {
                            (Some(a), Some(b)) => (a.x - b.x, a.y - b.y),
                            _ => return Err(TablesError::invalid(format!("glyph {gid} attaches a component to a point it does not have"))),
                        }
                    }
                };
                for contour in &mut placed {
                    for p in contour.iter_mut() {
                        p.x += dx;
                        p.y += dy;
                    }
                }
                work.spend(placed.iter().map(Vec::len).sum())?;
                out.extend(placed);
            }
            Ok(out)
        }
    }
}

/// A TrueType contour as segments, drawn as fontTools draws it.
pub(crate) fn tt_contour(points: &[TtPoint]) -> Option<Contour> {
    let n = points.len();
    let first = points.first()?;
    let p = |q: &TtPoint| pt(q.x, q.y);
    // Iteration starts after `begin` and ends on it.
    let (start, begin) = match points.iter().position(|q| q.on) {
        Some(s) => (p(points.get(s)?), s),
        None => (mid(p(points.last()?), p(first)), n - 1),
    };
    let mut contour = Contour { start, segs: Vec::new() };
    let mut cur = start;
    let mut pending: Option<Pt> = None;
    for i in 1..=n {
        let q = points.get((begin + i) % n)?;
        let here = p(q);
        let last = i == n;
        match (q.on, pending.take()) {
            (true, Some(off)) => {
                contour.segs.push(Seg::Quad(cur, off, here));
                cur = here;
            }
            // The closing line is implied by the close, which draws it only when needed.
            (true, None) if last && here == cur => {}
            (true, None) => {
                contour.segs.push(Seg::Line(cur, here));
                cur = here;
            }
            (false, Some(off)) => {
                let m = mid(off, here);
                contour.segs.push(Seg::Quad(cur, off, m));
                cur = m;
                pending = Some(here);
            }
            (false, None) => pending = Some(here),
        }
    }
    if let Some(off) = pending {
        contour.segs.push(Seg::Quad(cur, off, start));
        cur = start;
    }
    if cur != start {
        contour.segs.push(Seg::Line(cur, start));
    }
    Some(contour)
}

/// Collects charstring drawing commands as contours.
#[derive(Default)]
struct CffSink {
    contours: Vec<Contour>,
    open: Option<Contour>,
    pos: Pt,
}

impl CffSink {
    fn begin(&mut self) -> &mut Contour {
        let start = self.pos;
        self.open.get_or_insert_with(|| Contour { start, segs: Vec::new() })
    }
}

impl CommandSink for CffSink {
    fn move_to(&mut self, x: Fixed, y: Fixed) {
        self.close();
        self.pos = pt(fixed(x), fixed(y));
    }

    fn line_to(&mut self, x: Fixed, y: Fixed) {
        let (from, to) = (self.pos, pt(fixed(x), fixed(y)));
        self.begin().segs.push(Seg::Line(from, to));
        self.pos = to;
    }

    fn curve_to(&mut self, cx0: Fixed, cy0: Fixed, cx1: Fixed, cy1: Fixed, x: Fixed, y: Fixed) {
        let from = self.pos;
        let (c0, c1, to) = (pt(fixed(cx0), fixed(cy0)), pt(fixed(cx1), fixed(cy1)), pt(fixed(x), fixed(y)));
        self.begin().segs.push(Seg::Cubic(from, c0, c1, to));
        self.pos = to;
    }

    fn close(&mut self) {
        if let Some(mut c) = self.open.take() {
            if self.pos != c.start {
                c.segs.push(Seg::Line(self.pos, c.start));
            }
            self.pos = c.start;
            self.contours.push(c);
        }
    }
}

// ── Geometry ────────────────────────────────────────────────────────────────────────────────────

fn eval_quad(a: f64, b: f64, c: f64, t: f64) -> f64 {
    let u = 1.0 - t;
    u * u * a + 2.0 * u * t * b + t * t * c
}

fn eval_cubic(a: f64, b: f64, c: f64, d: f64, t: f64) -> f64 {
    let u = 1.0 - t;
    u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
}

/// The parameters in (0, 1) where one coordinate of a cubic has a zero derivative.
fn cubic_extremes(a: f64, b: f64, c: f64, d: f64) -> [Option<f64>; 2] {
    let qa = -a + 3.0 * b - 3.0 * c + d;
    let qb = 2.0 * (a - 2.0 * b + c);
    let qc = b - a;
    let inside = |t: f64| (t.is_finite() && t > 0.0 && t < 1.0).then_some(t);
    if qa.abs() < 1e-12 {
        if qb.abs() < 1e-12 {
            return [None, None];
        }
        return [inside(-qc / qb), None];
    }
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 {
        return [None, None];
    }
    let r = disc.sqrt();
    [inside((-qb + r) / (2.0 * qa)), inside((-qb - r) / (2.0 * qa))]
}

/// The exact bounding box of the contours: curve extremes, not control points. A contour that
/// is only a start point counts that point.
pub(crate) fn exact_bounds(contours: &[Contour]) -> Option<[f64; 4]> {
    let mut b: Option<[f64; 4]> = None;
    let mut add = |p: Pt| {
        b = Some(match b {
            None => [p.x, p.y, p.x, p.y],
            Some([x0, y0, x1, y1]) => [x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)],
        });
    };
    for c in contours {
        add(c.start);
        for s in &c.segs {
            match *s {
                Seg::Line(_, p1) => add(p1),
                Seg::Quad(p0, p1, p2) => {
                    add(p2);
                    let tx = (p0.x - p1.x) / (p0.x - 2.0 * p1.x + p2.x);
                    let ty = (p0.y - p1.y) / (p0.y - 2.0 * p1.y + p2.y);
                    for t in [tx, ty] {
                        if t.is_finite() && t > 0.0 && t < 1.0 {
                            add(pt(eval_quad(p0.x, p1.x, p2.x, t), eval_quad(p0.y, p1.y, p2.y, t)));
                        }
                    }
                }
                Seg::Cubic(p0, p1, p2, p3) => {
                    add(p3);
                    let xs = cubic_extremes(p0.x, p1.x, p2.x, p3.x);
                    let ys = cubic_extremes(p0.y, p1.y, p2.y, p3.y);
                    for t in xs.into_iter().chain(ys).flatten() {
                        add(pt(eval_cubic(p0.x, p1.x, p2.x, p3.x, t), eval_cubic(p0.y, p1.y, p2.y, p3.y, t)));
                    }
                }
            }
        }
    }
    b
}

/// Six-point Gauss–Legendre nodes and weights on [0, 1]: exact for polynomials up to degree 11,
/// the degree of the second moments' integrand on a cubic segment.
const GAUSS: [(f64, f64); 6] = [
    (0.033_765_242_898_423_99, 0.085_662_246_189_585_17),
    (0.169_395_306_766_867_74, 0.180_380_786_524_069_3),
    (0.380_690_406_958_401_55, 0.233_956_967_286_345_5),
    (0.619_309_593_041_598_5, 0.233_956_967_286_345_5),
    (0.830_604_693_233_132_3, 0.180_380_786_524_069_3),
    (0.966_234_757_101_576, 0.085_662_246_189_585_17),
];

/// Area and moments, as `momentsPen` defines them: each is −∮ f(x, y) dx.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Moments {
    pub area: f64,
    pub x: f64,
    pub y: f64,
    pub xx: f64,
    pub xy: f64,
    pub yy: f64,
}

pub(crate) fn moments(contours: &[Contour]) -> Moments {
    let mut m = Moments::default();
    for c in contours {
        for s in &c.segs {
            for (t, w) in GAUSS {
                // The point and dx/dt at t.
                let (x, y, dx) = match *s {
                    Seg::Line(a, b) => (a.x + t * (b.x - a.x), a.y + t * (b.y - a.y), b.x - a.x),
                    Seg::Quad(a, b, c) => {
                        let u = 1.0 - t;
                        (eval_quad(a.x, b.x, c.x, t), eval_quad(a.y, b.y, c.y, t), 2.0 * (u * (b.x - a.x) + t * (c.x - b.x)))
                    }
                    Seg::Cubic(a, b, c, d) => {
                        let u = 1.0 - t;
                        let dx = 3.0 * (u * u * (b.x - a.x) + 2.0 * u * t * (c.x - b.x) + t * t * (d.x - c.x));
                        (eval_cubic(a.x, b.x, c.x, d.x, t), eval_cubic(a.y, b.y, c.y, d.y, t), dx)
                    }
                };
                let wd = w * dx;
                m.area -= wd * y;
                m.x -= wd * x * y;
                m.y -= wd * y * y / 2.0;
                m.xx -= wd * x * x * y;
                m.xy -= wd * x * y * y / 2.0;
                m.yy -= wd * y * y * y / 3.0;
            }
        }
    }
    m
}

/// The derived values, with `statisticsPen`'s guards: absolute variances, a correlation clamped to
/// [-1, 1], and a correlation or slant that is undefined or below 0.001 in magnitude reported as 0.
fn finish_statistics(area: f64, mean: [f64; 2], variance: [f64; 2], covariance: f64) -> ShapeStatistics {
    let variance = variance.map(f64::abs);
    let stddev = variance.map(f64::sqrt);
    let [sx, sy] = stddev;
    let correlation = if sx * sy == 0.0 { 0.0 } else { (covariance / (sx * sy)).clamp(-1.0, 1.0) };
    let slant = if variance[1] == 0.0 { 0.0 } else { covariance / variance[1] };
    let small = |v: f64| if v.is_finite() && v.abs() > 1e-3 { v } else { 0.0 };
    ShapeStatistics { area, mean, variance, stddev, covariance, correlation: small(correlation), slant: small(slant) }
}

pub(crate) fn moment_statistics(contours: &[Contour]) -> ShapeStatistics {
    let m = moments(contours);
    if m.area == 0.0 || !m.area.is_finite() {
        return ShapeStatistics::default();
    }
    let mean = [m.x / m.area, m.y / m.area];
    let [mx, my] = mean;
    finish_statistics(m.area, mean, [m.xx / m.area - mx * mx, m.yy / m.area - my * my], m.xy / m.area - mx * my)
}

/// The control-polygon statistics of `StatisticsControlPen`: every on- and off-curve point once
/// per contour, plus the start again when a closing line is drawn, as one polygon.
pub(crate) fn control_statistics(contours: &[Contour]) -> ShapeStatistics {
    let mut nodes = Vec::new();
    for c in contours {
        nodes.push(c.start);
        for s in &c.segs {
            match *s {
                Seg::Line(_, b) => nodes.push(b),
                Seg::Quad(_, b, c) => nodes.extend([b, c]),
                Seg::Cubic(_, b, c, d) => nodes.extend([b, c, d]),
            }
        }
    }
    let n = nodes.len();
    if n == 0 {
        return ShapeStatistics::default();
    }
    let area = nodes.iter().zip(nodes.iter().cycle().skip(1)).map(|(a, b)| a.x * b.y - b.x * a.y).sum::<f64>() / 2.0;
    let nf = n as f64;
    let (sx, sy) = nodes.iter().fold((0.0, 0.0), |(x, y), p| (x + p.x, y + p.y));
    let mean = [sx / nf, sy / nf];
    let (variance, covariance) = if n > 1 {
        let sxx: f64 = nodes.iter().map(|p| p.x * p.x).sum();
        let syy: f64 = nodes.iter().map(|p| p.y * p.y).sum();
        let sxy: f64 = nodes.iter().map(|p| p.x * p.y).sum();
        let d = nf - 1.0;
        ([(sxx - sx * sx / nf) / d, (syy - sy * sy / nf) / d], (sxy - sx * sy / nf) / d)
    } else {
        ([0.0, 0.0], 0.0)
    };
    finish_statistics(area, mean, variance, covariance)
}

#[derive(Default)]
struct FontSums {
    count: usize,
    area: f64,
    area_by_width: f64,
    width: f64,
    slant: f64,
    slant_by_width: f64,
}

impl FontSums {
    fn add(&mut self, s: &ShapeStatistics, advance: f64) {
        self.count += 1;
        self.area += s.area.abs();
        self.area_by_width += s.area * advance;
        self.width += advance;
        self.slant += s.slant;
        self.slant_by_width += s.slant * advance;
    }

    fn finish(&self, upem: f64) -> Option<FontStatistics> {
        if self.count == 0 || self.width == 0.0 || upem == 0.0 {
            return None;
        }
        let em2 = upem * upem;
        let n = self.count as f64;
        let slant = self.slant / n;
        let slant_perceptual = self.slant_by_width / self.width;
        let angle = |s: f64| -s.atan().to_degrees();
        Some(FontStatistics {
            weight: self.area / em2 * upem / self.width,
            weight_perceptual: self.area_by_width / em2 / self.width,
            width: self.width / upem / n,
            slant,
            slant_angle: angle(slant),
            slant_perceptual,
            slant_perceptual_angle: angle(slant_perceptual),
        })
    }
}

fn font_bounds(glyphs: &[GlyphMetrics]) -> Option<FontBounds> {
    let mut out: Option<FontBounds> = None;
    for g in glyphs {
        let Some(b) = g.bounds else { continue };
        let fb = out.get_or_insert_with(|| FontBounds { bounds: b, gids: [g.gid; 4], names: std::array::from_fn(|_| g.name.clone()) });
        for edge in 0..4 {
            let (Some(&v), Some(cur)) = (b.get(edge), fb.bounds.get(edge).copied()) else { continue };
            let beyond = if edge < 2 { v < cur } else { v > cur };
            if beyond {
                if let (Some(slot), Some(gid), Some(name)) = (fb.bounds.get_mut(edge), fb.gids.get_mut(edge), fb.names.get_mut(edge)) {
                    *slot = v;
                    *gid = g.gid;
                    name.clone_from(&g.name);
                }
            }
        }
    }
    out
}

// ── Kerning ─────────────────────────────────────────────────────────────────────────────────────

struct KernBudget {
    steps: usize,
    exhausted: bool,
}

impl KernBudget {
    fn step(&mut self, n: usize) -> bool {
        if self.exhausted || self.steps < n {
            self.exhausted = true;
            return false;
        }
        self.steps -= n;
        true
    }
}

fn kerning(p: &Provider<'_>, count: usize, selected: &[u32], names: &[String], warnings: &mut Vec<String>) -> Vec<KernPair> {
    let mut budget = KernBudget { steps: MAX_KERN_STEPS, exhausted: false };
    let pairs = match gpos_pairs(p, count, &mut budget, warnings) {
        Some(pairs) => pairs,
        None => kern_table_pairs(p, count, &mut budget),
    };
    if budget.exhausted {
        warnings.push(format!("the kerning was read only in part: it holds more than {MAX_KERN_PAIRS} pairs or needs too much work"));
    }
    let mut wanted = vec![false; count];
    for &g in selected {
        if let Some(w) = wanted.get_mut(g as usize) {
            *w = true;
        }
    }
    let is = |g: u16| wanted.get(usize::from(g)).copied().unwrap_or(false);
    let name = |g: u16| names.get(usize::from(g)).cloned().unwrap_or_default();
    pairs
        .into_iter()
        .filter(|&((l, r), v)| v != 0 && is(l) && is(r))
        .map(|((l, r), v)| KernPair { left: name(l), right: name(r), value: f64::from(v) })
        .collect()
}

/// The pairs of the GPOS `kern` feature; `None` when the font has no such feature.
fn gpos_pairs(p: &Provider<'_>, count: usize, budget: &mut KernBudget, warnings: &mut Vec<String>) -> Option<BTreeMap<(u16, u16), i32>> {
    let gpos = p.gpos().ok()?;
    let features = gpos.feature_list().ok()?;
    let mut indices = Vec::new();
    for rec in features.feature_records() {
        if rec.feature_tag() != RTag::new(b"kern") {
            continue;
        }
        let Ok(feature) = rec.feature(features.offset_data()) else { continue };
        if !budget.step(feature.lookup_list_indices().len()) {
            break;
        }
        indices.extend(feature.lookup_list_indices().iter().map(|i| i.get()));
    }
    if indices.is_empty() {
        return None;
    }
    indices.sort_unstable();
    indices.dedup();
    let Ok(list) = gpos.lookup_list() else {
        warnings.push("the kerning lookups cannot be read".into());
        return Some(BTreeMap::new());
    };
    let lookups = list.lookups();
    let mut total: BTreeMap<(u16, u16), i32> = BTreeMap::new();
    let mut flattener = Flattener { count, claimed: vec![0; count], listed: vec![0; count], stamp: 0, sub_stamp: 0, skipped: 0 };
    for index in indices {
        if budget.exhausted {
            break;
        }
        let Ok(lookup) = lookups.get(usize::from(index)) else {
            flattener.skipped += 1;
            continue;
        };
        let Ok(PositionSubtables::Pair(subtables)) = lookup.subtables() else { continue };
        flattener.stamp = flattener.stamp.wrapping_add(1);
        let mut decided: HashMap<(u16, u16), i32> = HashMap::new();
        for sub in subtables.iter() {
            if !budget.step(1) {
                break;
            }
            match sub {
                Ok(PairPos::Format1(t)) => flattener.format1(&t, &mut decided, budget),
                Ok(PairPos::Format2(t)) => flattener.format2(&t, &mut decided, budget),
                Err(_) => flattener.skipped += 1,
            }
        }
        for (pair, value) in decided {
            if value == 0 {
                continue;
            }
            if total.len() >= MAX_KERN_PAIRS && !total.contains_key(&pair) {
                budget.exhausted = true;
                break;
            }
            *total.entry(pair).or_insert(0) += value;
        }
    }
    if flattener.skipped > 0 {
        warnings.push(format!("{} damaged kerning lookups or subtables were skipped", flattener.skipped));
    }
    Some(total)
}

struct Flattener {
    count: usize,
    /// Per glyph: the stamp of the lookup in which a class subtable decided it as a first glyph.
    claimed: Vec<u32>,
    /// Per glyph: the stamp of the class subtable whose second class definition lists it.
    listed: Vec<u32>,
    stamp: u32,
    sub_stamp: u32,
    skipped: usize,
}

impl Flattener {
    fn glyph(&self, g: GlyphId16) -> Option<u16> {
        let g = g.to_u16();
        (usize::from(g) < self.count).then_some(g)
    }

    fn free(&self, g: u16) -> bool {
        self.claimed.get(usize::from(g)) != Some(&self.stamp)
    }

    fn format1(&mut self, t: &PairPosFormat1<'_>, decided: &mut HashMap<(u16, u16), i32>, budget: &mut KernBudget) {
        let Ok(coverage) = t.coverage() else {
            self.skipped += 1;
            return;
        };
        let sets = t.pair_sets();
        for g1 in coverage.iter() {
            if !budget.step(1) {
                return;
            }
            let Some(first) = self.glyph(g1).filter(|&g| self.free(g)) else { continue };
            let Some(Ok(set)) = coverage.get(g1).map(|i| sets.get(usize::from(i))) else { continue };
            for record in set.pair_value_records().iter() {
                if !budget.step(1) {
                    return;
                }
                let Ok(record) = record else { break };
                let Some(second) = self.glyph(record.second_glyph()) else { continue };
                if decided.len() >= MAX_KERN_PAIRS {
                    budget.exhausted = true;
                    return;
                }
                decided.entry((first, second)).or_insert_with(|| record.value_record1().x_advance().map_or(0, i32::from));
            }
        }
    }

    fn format2(&mut self, t: &PairPosFormat2<'_>, decided: &mut HashMap<(u16, u16), i32>, budget: &mut KernBudget) {
        let (Ok(coverage), Ok(class1), Ok(class2)) = (t.coverage(), t.class_def1(), t.class_def2()) else {
            self.skipped += 1;
            return;
        };
        let (count1, count2) = (usize::from(t.class1_count()), usize::from(t.class2_count()));
        if !budget.step(count1 + count2) {
            return;
        }
        // The covered first glyphs this subtable decides, by class.
        let mut firsts = vec![Vec::new(); count1];
        for g in coverage.iter() {
            if !budget.step(1) {
                return;
            }
            let Some(first) = self.glyph(g).filter(|&g| self.free(g)) else { continue };
            // A class past the class count makes a shaper go on to the next subtable.
            let Some(members) = firsts.get_mut(usize::from(class1.get(g))) else { continue };
            members.push(first);
            if let Some(slot) = self.claimed.get_mut(usize::from(first)) {
                *slot = self.stamp;
            }
        }
        self.sub_stamp = self.sub_stamp.wrapping_add(1);
        let mut seconds = vec![Vec::new(); count2];
        for (g, class) in class2.iter() {
            if !budget.step(1) {
                return;
            }
            let Some(second) = self.glyph(g) else { continue };
            if class == 0 || class2.get(g) != class || self.listed.get(usize::from(second)) == Some(&self.sub_stamp) {
                continue;
            }
            if let Some(members) = seconds.get_mut(usize::from(class)) {
                members.push(second);
            }
            if let Some(slot) = self.listed.get_mut(usize::from(second)) {
                *slot = self.sub_stamp;
            }
        }
        let records = t.class_value_records();
        let mut unlisted: Option<Vec<u16>> = None;
        for (c1, members1) in firsts.iter().enumerate() {
            let Ok(c1) = u16::try_from(c1) else { break };
            if members1.is_empty() {
                continue;
            }
            for c2 in 0..t.class2_count() {
                if !budget.step(1) {
                    return;
                }
                let value = records.get(c1, c2).map_or(0, |[v, _]| v.x_advance().map_or(0, i32::from));
                // A zero pair still decides the pair, but the first glyph is already claimed for
                // this lookup, so nothing later can give it another value.
                if value == 0 {
                    continue;
                }
                let members2: &[u16] = if c2 == 0 {
                    if unlisted.is_none() {
                        if !budget.step(self.count) {
                            return;
                        }
                        let stamp = self.sub_stamp;
                        unlisted =
                            Some((0..self.count).filter(|&g| self.listed.get(g) != Some(&stamp)).filter_map(|g| u16::try_from(g).ok()).collect());
                    }
                    unlisted.as_deref().unwrap_or_default()
                } else {
                    seconds.get(usize::from(c2)).map_or(&[], Vec::as_slice)
                };
                for &first in members1 {
                    for &second in members2 {
                        if !budget.step(1) {
                            return;
                        }
                        if decided.len() >= MAX_KERN_PAIRS {
                            budget.exhausted = true;
                            return;
                        }
                        decided.entry((first, second)).or_insert(value);
                    }
                }
            }
        }
    }
}

/// The pairs of the horizontal format 0 subtables of the `kern` table, added up.
fn kern_table_pairs(p: &Provider<'_>, count: usize, budget: &mut KernBudget) -> BTreeMap<(u16, u16), i32> {
    let mut total = BTreeMap::new();
    let Ok(kern) = p.kern() else { return total };
    for sub in kern.subtables() {
        let Ok(sub) = sub else { continue };
        if !sub.is_horizontal() || sub.is_cross_stream() || sub.is_variable() {
            continue;
        }
        let Ok(SubtableKind::Format0(t)) = sub.kind() else { continue };
        for pair in t.pairs() {
            if !budget.step(1) {
                return total;
            }
            let (l, r) = (pair.left().to_u16(), pair.right().to_u16());
            if usize::from(l) >= count || usize::from(r) >= count {
                continue;
            }
            if total.len() >= MAX_KERN_PAIRS && !total.contains_key(&(l, r)) {
                budget.exhausted = true;
                return total;
            }
            *total.entry((l, r)).or_insert(0) += i32::from(pair.value());
        }
    }
    total
}

// ── Text, CSV and AFM ───────────────────────────────────────────────────────────────────────────

/// What the text forms show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextOptions {
    /// Print exact values instead of integers.
    pub real: bool,
    /// Add the side bearings.
    pub sidebearings: bool,
    /// Add the font bounding box and the glyphs that set its edges.
    pub aggregate_bbox: bool,
    /// With statistics, only the font-wide values.
    pub font_only: bool,
}

/// A length: rounded to an integer, or exact with up to four decimals.
fn num(v: f64, real: bool) -> String {
    trimmed(if real { format!("{v:.4}") } else { format!("{:.0}", v.round()) })
}

/// A statistic: up to six decimals.
fn stat(v: f64) -> String {
    trimmed(format!("{v:.6}"))
}

fn trimmed(s: String) -> String {
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    if s == "-0" { "0".into() } else { s }
}

fn enc(code: Option<u32>) -> String {
    code.map_or_else(|| "-".to_string(), |c| format!("0x{c:04X}"))
}

/// The plain listing, in the layout of the reference tool's metrics dump.
pub fn to_text(report: &MetricsReport, o: &TextOptions) -> String {
    let mut s = String::new();
    let r = o.real;
    if !o.font_only {
        let extra = if o.sidebearings { ",lsb,rsb" } else { "" };
        let _ = writeln!(s, "### glyph[tag] {{gname,enc,width,{{left,bottom,right,top}}{extra}}}");
        for g in &report.glyphs {
            let [a, b, c, d] = g.bounds.unwrap_or_default().map(|v| num(v, r));
            let _ = write!(s, "glyph[{}] {{{},{},{},{{{a},{b},{c},{d}}}", g.gid, g.name, enc(g.unicode), num(g.advance, r));
            if o.sidebearings {
                let _ = write!(s, ",{},{}", num(g.left_side_bearing, r), num(g.right_side_bearing, r));
            }
            s.push_str("}\n");
        }
    }
    if o.aggregate_bbox {
        if let Some(fb) = &report.font_bounds {
            let [a, b, c, d] = fb.bounds.map(|v| num(v, r));
            let [ga, gb, gc, gd] = fb.gids;
            let [na, nb, nc, nd] = &fb.names;
            let _ = writeln!(s, "### aggregate\nbbox  {{{a},{b},{c},{d}}}\ntag   {{{ga},{gb},{gc},{gd}}}\ngname {{{na},{nb},{nc},{nd}}}");
        }
    }
    if !o.font_only && report.glyphs.iter().any(|g| g.statistics.is_some()) {
        s.push_str("### statistics[tag] {gname,area,{meanX,meanY},{varianceX,varianceY},covariance,correlation,slant}\n");
        for g in &report.glyphs {
            let Some(st) = g.statistics else { continue };
            let _ = writeln!(
                s,
                "statistics[{}] {{{},{},{{{},{}}},{{{},{}}},{},{},{}}}",
                g.gid,
                g.name,
                stat(st.area),
                stat(st.mean[0]),
                stat(st.mean[1]),
                stat(st.variance[0]),
                stat(st.variance[1]),
                stat(st.covariance),
                stat(st.correlation),
                stat(st.slant)
            );
        }
    }
    if let Some(f) = &report.font_statistics {
        let _ = writeln!(
            s,
            "### font statistics (estimates)\nweight {}\nweight (perceptual) {}\nwidth {}\nslant {}\nslant angle {}\nslant (perceptual) {}\nslant (perceptual) angle {}",
            stat(f.weight),
            stat(f.weight_perceptual),
            stat(f.width),
            stat(f.slant),
            stat(f.slant_angle),
            stat(f.slant_perceptual),
            stat(f.slant_perceptual_angle)
        );
    }
    if !report.kerning.is_empty() {
        s.push_str("### kerning {left,right,value}\n");
        for k in &report.kerning {
            let _ = writeln!(s, "kern {{{},{},{}}}", k.left, k.right, num(k.value, r));
        }
    }
    s
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

/// Comma-separated values: the glyph table, then the kerning table and the font-wide values,
/// each after a blank line and its own header.
pub fn to_csv(report: &MetricsReport, o: &TextOptions) -> String {
    let r = o.real;
    let stats = report.glyphs.iter().any(|g| g.statistics.is_some());
    let mut s = String::new();
    if !o.font_only {
        s.push_str("gid,name,unicode,advance,xMin,yMin,xMax,yMax");
        if o.sidebearings {
            s.push_str(",lsb,rsb");
        }
        if stats {
            s.push_str(",area,meanX,meanY,varianceX,varianceY,covariance,correlation,slant");
        }
        s.push('\n');
        for g in &report.glyphs {
            let code = g.unicode.map(|c| format!("U+{c:04X}")).unwrap_or_default();
            let bounds = g.bounds.map(|b| b.map(|v| num(v, r)).join(",")).unwrap_or_else(|| ",,,".into());
            let _ = write!(s, "{},{},{code},{},{bounds}", g.gid, csv_field(&g.name), num(g.advance, r));
            if o.sidebearings {
                let _ = write!(s, ",{},{}", num(g.left_side_bearing, r), num(g.right_side_bearing, r));
            }
            if stats {
                match g.statistics {
                    Some(st) => {
                        let v = [st.area, st.mean[0], st.mean[1], st.variance[0], st.variance[1], st.covariance, st.correlation, st.slant];
                        let _ = write!(s, ",{}", v.map(stat).join(","));
                    }
                    None => s.push_str(",,,,,,,,"),
                }
            }
            s.push('\n');
        }
    }
    if !report.kerning.is_empty() {
        s.push_str("\nleft,right,value\n");
        for k in &report.kerning {
            let _ = writeln!(s, "{},{},{}", csv_field(&k.left), csv_field(&k.right), num(k.value, r));
        }
    }
    let mut props: Vec<(String, String)> = Vec::new();
    if o.aggregate_bbox {
        if let Some(fb) = &report.font_bounds {
            for (i, edge) in ["xMin", "yMin", "xMax", "yMax"].iter().enumerate() {
                let (Some(v), Some(n)) = (fb.bounds.get(i), fb.names.get(i)) else { continue };
                props.push(((*edge).into(), num(*v, r)));
                props.push((format!("{edge}Glyph"), csv_field(n)));
            }
        }
    }
    if let Some(f) = &report.font_statistics {
        for (k, v) in [
            ("weight", f.weight),
            ("weightPerceptual", f.weight_perceptual),
            ("width", f.width),
            ("slant", f.slant),
            ("slantAngle", f.slant_angle),
            ("slantPerceptual", f.slant_perceptual),
            ("slantPerceptualAngle", f.slant_perceptual_angle),
        ] {
            props.push((k.into(), stat(v)));
        }
    }
    if !props.is_empty() {
        s.push_str("\nproperty,value\n");
        for (k, v) in props {
            let _ = writeln!(s, "{k},{v}");
        }
    }
    s
}

/// An Adobe Font Metrics file. As the reference tool writes it, the numbers are in the font's
/// own units and `C` holds the glyph's Unicode code point (-1 for none); unlike it, the
/// character count is the number of glyphs written.
pub fn to_afm(report: &MetricsReport, o: &TextOptions) -> String {
    let r = o.real;
    let i = &report.info;
    let clean = |t: &str| t.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>();
    let mut s = String::from("StartFontMetrics 2.0\n");
    if !i.notice.is_empty() {
        let _ = writeln!(s, "Comment {}", clean(&i.notice));
    }
    if report.units != 1000.0 {
        let _ = writeln!(s, "Comment Values are in units of {} per em", num(report.units, true));
    }
    let _ = writeln!(s, "FontName {}", clean(&i.postscript_name));
    if !i.full_name.is_empty() {
        let _ = writeln!(s, "FullName {}", clean(&i.full_name));
    }
    if !i.family_name.is_empty() {
        let _ = writeln!(s, "FamilyName {}", clean(&i.family_name));
    }
    let _ = writeln!(s, "ItalicAngle {}", num(i.italic_angle, true));
    let _ = writeln!(s, "IsFixedPitch {}", i.fixed_pitch);
    let [a, b, c, d] = report.font_bounds.as_ref().map(|f| f.bounds).unwrap_or_default().map(|v| num(v, r));
    let _ = writeln!(s, "FontBBox {a} {b} {c} {d}");
    let _ = writeln!(s, "UnderlinePosition {}", num(i.underline_position, r));
    let _ = writeln!(s, "UnderlineThickness {}", num(i.underline_thickness, r));
    let _ = writeln!(s, "Version {}", clean(&i.version));
    let _ = writeln!(s, "StartCharMetrics {}", report.glyphs.len());
    for g in &report.glyphs {
        let code = g.unicode.map_or_else(|| "-1".to_string(), |c| c.to_string());
        let [a, b, c, d] = g.bounds.unwrap_or_default().map(|v| num(v, r));
        let _ = writeln!(s, "C {code} ; WX {} ; N {} ; B {a} {b} {c} {d} ;", num(g.advance, r), g.name);
    }
    s.push_str("EndCharMetrics\n");
    if !report.kerning.is_empty() {
        let _ = writeln!(s, "StartKernData\nStartKernPairs {}", report.kerning.len());
        for k in &report.kerning {
            let _ = writeln!(s, "KPX {} {} {}", k.left, k.right, num(k.value, r));
        }
        s.push_str("EndKernPairs\nEndKernData\n");
    }
    s.push_str("EndFontMetrics\n");
    s
}

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;
