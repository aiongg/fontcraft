//! The abstract-font dump (`@font`, `@glyphs`), as AFDKO `tx -dump` prints it
//! (`docs/spec/20-cli.md` › Method: abstract-font dump).
//!
//! `@font` is `tx -dump -0` without its `## Filename` line: the Top DICT, each font DICT and
//! its Private DICT. A CFF font prints the values its DICTs store, in the order below; a
//! TrueType font prints values gathered from `head`, `name`, `post` and `OS/2`, leaving out
//! those equal to the PostScript defaults. `@glyphs` is `tx -dump -4`, `-5` or `-6`: one record
//! per glyph with its index, name and first Unicode code point, and with more detail the path in
//! absolute coordinates with its stem hints.
//!
//! How the records were settled (on corpus fonts with `tx` 1.x of AFDKO 4):
//! - A stem prints as `bottom top hstem`; a ghost stem of width -21 as `edge bottomedge`, of
//!   width -20 as `edge topedge`. Before the first point, the stems the first hint mask selects
//!   print (all stems without a mask); each later mask prints `newhints` and its stems.
//! - Numbers print as integers, or with two decimals when they have a fraction.
//! - A TrueType contour starts at its first point when that is on the curve, else at its last
//!   point when that is, else halfway between the two. A single off-curve point becomes a
//!   cubic with the usual two-thirds rule; two off-curve points in a row become one cubic whose
//!   handles lie four-thirds of the way along the quadratic handles, ending at the next implied
//!   point. TrueType coordinates are rounded to integers. A closing line back to the start does
//!   not print.
//! - Medium detail wraps the path tokens into lines of at most 78 characters, each token
//!   preceded by a space.

use std::collections::{HashMap, HashSet};

use read_fonts::TableProvider;
use read_fonts::ps::cff::CffFontRef;
use read_fonts::ps::cs::CommandSink;
use read_fonts::tables::cmap::CmapIterLimits;
use read_fonts::tables::glyf::{Anchor, Glyph};
use read_fonts::types::{Fixed, GlyphId};

use super::{GlyphDetail, Out, ViewOptions};
use crate::convert::cff_table::Seac;
use crate::features::Provider;
use crate::limits::{MAX_GLYPHS, MAX_LOOKUP_NESTING, MAX_TTX_CMAP_ENTRIES};
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// Medium detail starts a new line before a token that would take the line past this width.
const MEDIUM_WIDTH: usize = 78;

/// Most path operators one glyph prints; a charstring that draws more is refused.
pub const MAX_GLYPH_OPS: usize = 1_000_000;

const CFF: [u8; 4] = *b"CFF ";
const CFF2: [u8; 4] = *b"CFF2";

/// `@font`: the font's DICTs as text.
pub fn font_dict(font: &SfntFont) -> Result<String> {
    let mut out = Out::new();
    if let Some(data) = font.get(CFF) {
        cff_dicts(font, data, false, &mut out)?;
    } else if let Some(data) = font.get(CFF2) {
        cff_dicts(font, data, true, &mut out)?;
    } else {
        truetype_dict(font, &mut out)?;
    }
    Ok(out.text)
}

/// `@glyphs`: the records of the glyphs named in [`ViewOptions::glyphs`] (every glyph when
/// empty), in the order given, without those named in [`ViewOptions::exclude_glyphs`]
/// (`tx -gx`), kept or left out by font dict ([`ViewOptions::fd`], `tx -fd`), under the names
/// [`ViewOptions::rename`] gives them (`tx -gf`).
///
/// An accented `seac` glyph prints its width, `adx ady base accent seac` and `endchar`; with
/// [`ViewOptions::flatten_seac`] (`tx -dump -d`) it prints the base glyph's path and hints, then
/// the accent's path moved by (adx, ady) without its hints, as `tx` printed them on a font built
/// with fontTools.
pub fn glyphs(font: &SfntFont, options: &ViewOptions) -> Result<String> {
    let detail = options.glyph_detail;
    let hints = !options.no_hints;
    let mut src = Source::new(font)?;
    let font_names = src.names.clone();
    if !options.rename.is_empty() {
        let map: HashMap<&str, &str> = options.rename.iter().map(|(from, to)| (from.as_str(), to.as_str())).collect();
        for name in &mut src.names {
            if let Some(to) = map.get(name.as_str()) {
                *name = (*to).to_string();
            }
        }
    }
    let excluded = &options.exclude_glyphs;
    let left_out: HashSet<usize> = if excluded.is_empty() { HashSet::new() } else { src.select(excluded)?.into_iter().collect() };
    let ids: Vec<usize> = src.select(&options.glyphs)?.into_iter().filter(|g| !left_out.contains(g)).collect();
    let ids = by_font_dict(font, ids, &options.fd, &options.exclude_fd)?;
    // The seac parts of each glyph; only a name-keyed CFF table has them, and only the path
    // shows them. A table the reader below cannot expand is drawn as the evaluator draws it.
    let programs = match (detail, font.get(CFF)) {
        (GlyphDetail::Short, _) | (_, None) => None,
        (_, Some(data)) => crate::convert::cff_table::Programs::read(data, false).ok(),
    };
    let by_name: HashMap<&str, usize> = if programs.is_some() && options.flatten_seac {
        let mut map = HashMap::with_capacity(font_names.len());
        for (g, n) in font_names.iter().enumerate() {
            map.entry(n.as_str()).or_insert(g);
        }
        map
    } else {
        HashMap::new()
    };
    let mut out = Out::new();
    match detail {
        GlyphDetail::Short => out.line("## glyph[tag] {name,encoding}")?,
        _ => out.line("## glyph[tag] {name,encoding,path}")?,
    }
    for gid in ids {
        let name = src.name(gid);
        let enc = src.encoding(gid);
        if detail == GlyphDetail::Short {
            out.line(&format!("glyph[{gid}] {{{name},{enc}}}"))?;
            continue;
        }
        out.line(&format!("glyph[{gid}] {{{name},{enc},"))?;
        let tokens = match programs.as_ref().and_then(|p| p.seac(gid)) {
            Some(seac) => {
                let operand = programs.as_ref().and_then(|p| p.width(gid));
                src.seac_path(gid, seac, operand, options.flatten_seac.then_some(&by_name), hints)?
            }
            None => src.path(gid, hints)?,
        };
        if detail == GlyphDetail::Long {
            let count = tokens.len();
            for (i, t) in tokens.iter().enumerate() {
                out.push("  ")?;
                out.push(t)?;
                out.line(if i + 1 == count { "}" } else { "" })?;
            }
        } else {
            let words: Vec<&str> = tokens.iter().flat_map(|t| t.split(' ')).collect();
            let mut line = String::new();
            let count = words.len();
            for (i, w) in words.iter().enumerate() {
                let piece = if i + 1 == count { format!(" {w}}}") } else { format!(" {w}") };
                if !line.is_empty() && line.len() + piece.len() > MEDIUM_WIDTH {
                    out.line(&line)?;
                    line.clear();
                }
                line.push_str(&piece);
            }
            out.line(&line)?;
        }
    }
    Ok(out.text)
}

/// `ids` less the glyphs outside the font dicts `keep` (all when empty) or inside `leave_out`. Only a
/// CID-keyed CFF font or a CFF2 font has font dicts to select by, as `tx -fd` requires.
fn by_font_dict(font: &SfntFont, ids: Vec<usize>, keep: &[u16], leave_out: &[u16]) -> Result<Vec<usize>> {
    if keep.is_empty() && leave_out.is_empty() {
        return Ok(ids);
    }
    let upem = Provider(font).head().ok().map(|h| i32::from(h.units_per_em()));
    let bad = |e: read_fonts::ps::error::Error| TablesError::Invalid(format!("the CFF table cannot be read: {e}"));
    let cff = if let Some(data) = font.get(CFF) {
        let cff = CffFontRef::new_cff(data, 0, upem).map_err(bad)?;
        if !cff.is_cid() {
            return Err(TablesError::BadOptions(
                "the CFF font is not CID-keyed: only a CID-keyed CFF font or a CFF2 font has font dicts to select by".into(),
            ));
        }
        cff
    } else if let Some(data) = font.get(CFF2) {
        CffFontRef::new_cff2(data, upem).map_err(bad)?
    } else {
        return Err(TablesError::BadOptions(
            "a TrueType font has no font dicts: only a CID-keyed CFF font or a CFF2 font can be selected by font dict".into(),
        ));
    };
    let count = cff.num_subfonts();
    if let Some(fd) = keep.iter().chain(leave_out).find(|fd| **fd >= count) {
        return Err(TablesError::BadOptions(format!("the font has {count} font dicts: there is no font dict {fd}")));
    }
    Ok(ids
        .into_iter()
        .filter(|g| {
            let fd = u32::try_from(*g).ok().and_then(|g| cff.subfont_index(GlyphId::new(g))).unwrap_or(0);
            (keep.is_empty() || keep.contains(&fd)) && !leave_out.contains(&fd)
        })
        .collect())
}

/// The outline entries of the normalised dump: for each glyph its name, then one line per path
/// operator indented by two spaces, sorted as text.
pub(crate) fn outline_entries(font: &SfntFont, hints: bool) -> Result<Vec<String>> {
    let src = Source::new(font)?;
    let mut entries = Vec::with_capacity(src.count);
    let mut size = 0usize;
    for gid in 0..src.count {
        let mut e = src.name(gid);
        for t in src.path(gid, hints)? {
            e.push_str("\n  ");
            e.push_str(&t);
        }
        size = size.saturating_add(e.len());
        if size > super::MAX_TEXT_OUTPUT {
            return Err(TablesError::TooLarge { what: "the outline dump", limit: super::MAX_TEXT_OUTPUT });
        }
        entries.push(e);
    }
    entries.sort();
    Ok(entries)
}

// ── numbers ─────────────────────────────────────────────────────────────────────────────────────

/// A coordinate or value as `tx` prints it: an integer, or two decimals.
fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", v as i64) } else { format!("{v:.2}") }
}

/// A DICT number: integers as integers, reals in their shortest form.
fn dict_num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", v as i64) } else { format!("{v}") }
}

fn array(values: &[f64]) -> String {
    let parts: Vec<String> = values.iter().map(|v| dict_num(*v)).collect();
    format!("{{{}}}", parts.join(","))
}

fn key(out: &mut Out, k: &str, v: &str) -> Result<()> {
    out.line(&format!("{k:<19} {v}"))
}

// ── glyph source ────────────────────────────────────────────────────────────────────────────────

struct Source<'a> {
    font: &'a SfntFont,
    count: usize,
    names: Vec<String>,
    /// First Unicode code point of each glyph.
    codes: Vec<Option<u32>>,
    widths: Vec<Option<u16>>,
}

impl<'a> Source<'a> {
    fn new(font: &'a SfntFont) -> Result<Self> {
        let names = crate::ttx::glyph_order(font)?;
        let count = names.len();
        if count > MAX_GLYPHS {
            return Err(TablesError::TooLarge { what: "the glyph count", limit: MAX_GLYPHS });
        }
        let p = Provider(font);
        let mut codes: Vec<Option<u32>> = vec![None; count];
        if let Ok(cmap) = p.cmap() {
            let data = cmap.offset_data();
            // The best Unicode subtable: full repertoire first, then the BMP ones.
            let rank = |platform: u16, encoding: u16| match (platform, encoding) {
                (3, 10) => Some(0),
                (0, 6) | (0, 4) => Some(1),
                (3, 1) => Some(2),
                (0, _) => Some(3),
                _ => None,
            };
            let mut best: Option<(u8, read_fonts::tables::cmap::CmapSubtable)> = None;
            for rec in cmap.encoding_records() {
                let Some(r) = rank(rec.platform_id() as u16, rec.encoding_id()) else { continue };
                let Ok(sub) = rec.subtable(data) else { continue };
                if matches!(sub, read_fonts::tables::cmap::CmapSubtable::Format14(_)) {
                    continue;
                }
                if best.as_ref().is_none_or(|(b, _)| r < *b) {
                    best = Some((r, sub));
                }
            }
            if let Some((_, sub)) = best {
                let limits = CmapIterLimits { max_char: 0x10_FFFF, glyph_count: u32::try_from(count).unwrap_or(u32::MAX) };
                for (seen, (code, gid)) in sub.iter_with_limits(limits).enumerate() {
                    if seen >= MAX_TTX_CMAP_ENTRIES {
                        return Err(TablesError::TooLarge { what: "the cmap mappings", limit: MAX_TTX_CMAP_ENTRIES });
                    }
                    // tx prints a glyph mapped only to .notdef's code points as unencoded.
                    if gid.to_u32() == 0 {
                        continue;
                    }
                    if let Some(slot) = codes.get_mut(gid.to_u32() as usize)
                        && slot.is_none_or(|c| code < c)
                    {
                        *slot = Some(code);
                    }
                }
            }
        }
        let mut widths = vec![None; count];
        if let Ok(hmtx) = p.hmtx() {
            for (gid, w) in widths.iter_mut().enumerate() {
                *w = hmtx.advance(GlyphId::new(gid as u32));
            }
        }
        Ok(Self { font, count, names, codes, widths })
    }

    fn name(&self, gid: usize) -> String {
        self.names.get(gid).cloned().unwrap_or_else(|| format!("glyph{gid:05}"))
    }

    fn encoding(&self, gid: usize) -> String {
        match self.codes.get(gid).copied().flatten() {
            Some(c) => format!("U+{c:04X}"),
            None => "-".into(),
        }
    }

    fn select(&self, selection: &[String]) -> Result<Vec<usize>> {
        if selection.is_empty() {
            return Ok((0..self.count).collect());
        }
        // The first glyph of each name, as a search from the start finds it.
        let mut by_name: HashMap<&str, usize> = HashMap::with_capacity(self.names.len());
        for (g, n) in self.names.iter().enumerate() {
            by_name.entry(n.as_str()).or_insert(g);
        }
        let mut out = Vec::with_capacity(selection.len());
        for s in selection {
            match by_name.get(s.as_str()).copied() {
                Some(g) => out.push(g),
                None => return Err(TablesError::BadOptions(format!("the font has no glyph named `{s}`"))),
            }
        }
        Ok(out)
    }

    /// The path tokens of one glyph: the width, the hints, one item per operator, `endchar`.
    fn path(&self, gid: usize, hints: bool) -> Result<Vec<String>> {
        let id = GlyphId::new(u32::try_from(gid).map_err(|_| TablesError::invalid("glyph index out of range"))?);
        if let Some(data) = self.font.get(CFF).map(|d| (d, false)).or_else(|| self.font.get(CFF2).map(|d| (d, true))) {
            return self.cff_path(data.0, data.1, id, gid, hints);
        }
        self.tt_path(id, gid)
    }

    fn cff_path(&self, data: &[u8], cff2: bool, id: GlyphId, gid: usize, hints: bool) -> Result<Vec<String>> {
        let (ops, width) = self.cff_ops(data, cff2, id, gid)?;
        Ok(render(&ops, width, hints))
    }

    /// The path of an accented glyph: its `seac` operator, or with `parts` (glyph indices by
    /// name) the base glyph and the moved accent drawn out. `operand` is the glyph's width
    /// operand (16.16, against nominalWidthX).
    fn seac_path(&self, gid: usize, seac: Seac, operand: Option<i64>, parts: Option<&HashMap<&str, usize>>, hints: bool) -> Result<Vec<String>> {
        let data = self.font.get(CFF).ok_or_else(|| TablesError::MissingTable("CFF ".into()))?;
        let glyph = |g: usize| u32::try_from(g).map(GlyphId::new).map_err(|_| TablesError::invalid("glyph index out of range"));
        // The width alone: drawing the glyph would follow the seac through the charset's
        // standard strings, which a charset of custom strings lacks.
        let width = self.cff_width(data, glyph(gid)?, gid, operand)?;
        let Some(parts) = parts else {
            return Ok(vec![
                format!("{} width", num(width)),
                format!("{} {} {} {} seac", num(seac.adx), num(seac.ady), seac.base, seac.accent),
                "endchar".into(),
            ]);
        };
        let part = |code: u8| -> Result<usize> {
            let name = read_fonts::ps::encoding::PredefinedEncoding::Standard.name(code);
            parts.get(name).copied().ok_or_else(|| TablesError::Invalid(format!("glyph {gid}: the font has no glyph `{name}` for seac code {code}")))
        };
        let (base, accent) = (part(seac.base)?, part(seac.accent)?);
        let (mut ops, _) = self.cff_ops(data, false, glyph(base)?, base)?;
        let (accent_ops, _) = self.cff_ops(data, false, glyph(accent)?, accent)?;
        let (dx, dy) = (seac.adx, seac.ady);
        for op in accent_ops {
            match op {
                Op::Move(x, y) => ops.push(Op::Move(x + dx, y + dy)),
                Op::Line(x, y) => ops.push(Op::Line(x + dx, y + dy)),
                Op::Curve([a, b, c, d, e, f]) => ops.push(Op::Curve([a + dx, b + dy, c + dx, d + dy, e + dx, f + dy])),
                Op::HStem(..) | Op::VStem(..) | Op::Mask(_) => {}
            }
        }
        Ok(render(&ops, width, hints))
    }

    /// The advance of one glyph of a `CFF ` table from its width operand, without drawing it.
    fn cff_width(&self, data: &[u8], id: GlyphId, gid: usize, operand: Option<i64>) -> Result<f64> {
        let upem = Provider(self.font).head().ok().map(|h| i32::from(h.units_per_em()));
        let bad = |e: read_fonts::ps::error::Error| TablesError::Invalid(format!("glyph {gid}: the charstring cannot be read: {e}"));
        let cff = CffFontRef::new_cff(data, 0, upem).map_err(bad)?;
        let sub = cff.subfont(cff.subfont_index(id).unwrap_or(0), &[]).map_err(bad)?;
        Ok(match (operand, sub.default_width()) {
            (Some(w), _) => w as f64 / 65536.0 + sub.nominal_width().to_f64(),
            (None, Some(default)) => default.to_f64(),
            (None, None) => f64::from(self.widths.get(gid).copied().flatten().unwrap_or(0)),
        })
    }

    /// The operators of one CFF glyph as the evaluator draws it, and its advance.
    fn cff_ops(&self, data: &[u8], cff2: bool, id: GlyphId, gid: usize) -> Result<(Vec<Op>, f64)> {
        let upem = Provider(self.font).head().ok().map(|h| i32::from(h.units_per_em()));
        let bad = |e: read_fonts::ps::error::Error| TablesError::Invalid(format!("glyph {gid}: the charstring cannot be read: {e}"));
        let cff = if cff2 { CffFontRef::new_cff2(data, upem) } else { CffFontRef::new_cff(data, 0, upem) }.map_err(bad)?;
        let fd = cff.subfont_index(id).unwrap_or(0);
        let sub = cff.subfont(fd, &[]).map_err(bad)?;
        let mut sink = Recorder::default();
        let width = cff.evaluate_charstring(&sub, id, &[], &mut sink).map_err(bad)?;
        if sink.overflow {
            return Err(TablesError::TooLarge { what: "the path operators of one glyph", limit: MAX_GLYPH_OPS });
        }
        let width = match width {
            Some(w) => w.to_f64(),
            None => f64::from(self.widths.get(gid).copied().flatten().unwrap_or(0)),
        };
        Ok((sink.ops, width))
    }

    fn tt_path(&self, id: GlyphId, gid: usize) -> Result<Vec<String>> {
        let width = f64::from(self.widths.get(gid).copied().flatten().unwrap_or(0));
        let mut ops = Vec::new();
        let p = Provider(self.font);
        if let (Ok(loca), Ok(glyf)) = (p.loca(None), p.glyf()) {
            let mut points = 0usize;
            tt_glyph(&loca, &glyf, id, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], 0, &mut ops, &mut points)?;
        }
        let mut tokens = vec![format!("{} width", num(width))];
        for op in ops {
            tokens.push(op);
        }
        tokens.push("endchar".into());
        Ok(tokens)
    }
}

// ── CFF paths ───────────────────────────────────────────────────────────────────────────────────

pub(super) enum Op {
    HStem(f64, f64),
    VStem(f64, f64),
    Mask(Vec<u8>),
    Move(f64, f64),
    Line(f64, f64),
    Curve([f64; 6]),
}

#[derive(Default)]
struct Recorder {
    ops: Vec<Op>,
    overflow: bool,
}

impl Recorder {
    fn add(&mut self, op: Op) {
        if self.ops.len() >= MAX_GLYPH_OPS {
            self.overflow = true;
        } else {
            self.ops.push(op);
        }
    }
}

impl CommandSink for Recorder {
    fn move_to(&mut self, x: Fixed, y: Fixed) {
        self.add(Op::Move(x.to_f64(), y.to_f64()));
    }
    fn line_to(&mut self, x: Fixed, y: Fixed) {
        self.add(Op::Line(x.to_f64(), y.to_f64()));
    }
    fn curve_to(&mut self, cx0: Fixed, cy0: Fixed, cx1: Fixed, cy1: Fixed, x: Fixed, y: Fixed) {
        self.add(Op::Curve([cx0.to_f64(), cy0.to_f64(), cx1.to_f64(), cy1.to_f64(), x.to_f64(), y.to_f64()]));
    }
    fn close(&mut self) {}
    fn hstem(&mut self, y: Fixed, dy: Fixed) {
        self.add(Op::HStem(y.to_f64(), dy.to_f64()));
    }
    fn vstem(&mut self, x: Fixed, dx: Fixed) {
        self.add(Op::VStem(x.to_f64(), dx.to_f64()));
    }
    fn hint_mask(&mut self, mask: &[u8]) {
        self.add(Op::Mask(mask.to_vec()));
    }
}

/// One stem as `tx` prints it. `a` and `b` are the two edges the evaluator reports (the second
/// is the first plus the stored width).
fn stem(vertical: bool, a: f64, b: f64) -> String {
    let w = b - a;
    match (vertical, w) {
        (false, -21.0) => format!("{} bottomedge", num(b)),
        (false, -20.0) => format!("{} topedge", num(a)),
        (true, -21.0) => format!("{} leftedge", num(b)),
        (true, -20.0) => format!("{} rightedge", num(a)),
        (false, _) => format!("{} {} hstem", num(a), num(b)),
        (true, _) => format!("{} {} vstem", num(a), num(b)),
    }
}

pub(super) fn render(ops: &[Op], width: f64, hints: bool) -> Vec<String> {
    let stems: Vec<(bool, f64, f64)> = ops
        .iter()
        .filter_map(|op| match op {
            Op::HStem(a, b) => Some((false, *a, *b)),
            Op::VStem(a, b) => Some((true, *a, *b)),
            _ => None,
        })
        .collect();
    let selected = |mask: &[u8]| -> Vec<String> {
        stems
            .iter()
            .enumerate()
            .filter(|(i, _)| mask.get(i / 8).is_some_and(|b| b & (0x80 >> (i % 8)) != 0))
            .map(|(_, (v, a, b))| stem(*v, *a, *b))
            .collect()
    };
    let mut tokens = vec![format!("{} width", num(width))];
    let first_path = ops.iter().position(|op| matches!(op, Op::Move(..) | Op::Line(..) | Op::Curve(..))).unwrap_or(ops.len());
    let first_mask = ops.iter().take(first_path).position(|op| matches!(op, Op::Mask(_)));
    if hints {
        match first_mask.and_then(|i| ops.get(i)) {
            Some(Op::Mask(m)) => tokens.extend(selected(m)),
            _ => tokens.extend(stems.iter().map(|(v, a, b)| stem(*v, *a, *b))),
        }
    }
    for (i, op) in ops.iter().enumerate() {
        match op {
            Op::Mask(m) if hints && i >= first_path => {
                tokens.push("newhints".into());
                tokens.extend(selected(m));
            }
            Op::Move(x, y) => tokens.push(format!("{} {} move", num(*x), num(*y))),
            Op::Line(x, y) => tokens.push(format!("{} {} line", num(*x), num(*y))),
            Op::Curve(c) => tokens.push(format!("{} {} {} {} {} {} curve", num(c[0]), num(c[1]), num(c[2]), num(c[3]), num(c[4]), num(c[5]))),
            _ => {}
        }
    }
    tokens.push("endchar".into());
    tokens
}

// ── TrueType paths ──────────────────────────────────────────────────────────────────────────────

type Affine = [f64; 6];

fn apply(m: &Affine, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

fn tt_glyph(
    loca: &read_fonts::tables::loca::Loca,
    glyf: &read_fonts::tables::glyf::Glyf,
    id: GlyphId,
    m: Affine,
    depth: usize,
    ops: &mut Vec<String>,
    points: &mut usize,
) -> Result<()> {
    if depth > MAX_LOOKUP_NESTING {
        return Err(TablesError::invalid(format!("glyph {}: components nest deeper than {MAX_LOOKUP_NESTING}", id.to_u32())));
    }
    // Every glyph visited counts, so components that fan out without points stay bounded too.
    *points = points.saturating_add(1);
    if *points > MAX_GLYPH_OPS {
        return Err(TablesError::TooLarge { what: "the path operators of one glyph", limit: MAX_GLYPH_OPS });
    }
    let glyph = loca.get_glyf(id, glyf).map_err(|e| TablesError::Invalid(format!("glyph {}: {e}", id.to_u32())))?;
    match glyph {
        None => Ok(()),
        Some(Glyph::Simple(s)) => {
            let pts: Vec<(f64, f64, bool)> = s
                .points()
                .map(|p| {
                    let (x, y) = apply(&m, f64::from(p.x), f64::from(p.y));
                    (x, y, p.on_curve)
                })
                .collect();
            *points = points.saturating_add(pts.len());
            if *points > MAX_GLYPH_OPS {
                return Err(TablesError::TooLarge { what: "the path operators of one glyph", limit: MAX_GLYPH_OPS });
            }
            let mut start = 0usize;
            for end in s.end_pts_of_contours() {
                let end = usize::from(end.get());
                if end < start {
                    return Err(TablesError::invalid(format!("glyph {}: contour end points decrease", id.to_u32())));
                }
                if let Some(contour) = pts.get(start..=end) {
                    tt_contour(contour, ops);
                }
                start = end + 1;
            }
            Ok(())
        }
        Some(Glyph::Composite(c)) => {
            for comp in c.components() {
                let t = comp.transform;
                let (dx, dy) = match comp.anchor {
                    Anchor::Offset { x, y } => (f64::from(x), f64::from(y)),
                    Anchor::Point { .. } => (0.0, 0.0),
                };
                let local: Affine = [t.xx.to_f64(), t.yx.to_f64(), t.xy.to_f64(), t.yy.to_f64(), dx, dy];
                let combined: Affine = [
                    m[0] * local[0] + m[2] * local[1],
                    m[1] * local[0] + m[3] * local[1],
                    m[0] * local[2] + m[2] * local[3],
                    m[1] * local[2] + m[3] * local[3],
                    m[0] * local[4] + m[2] * local[5] + m[4],
                    m[1] * local[4] + m[3] * local[5] + m[5],
                ];
                tt_glyph(loca, glyf, GlyphId::from(comp.glyph), combined, depth + 1, ops, points)?;
            }
            Ok(())
        }
    }
}

fn rnum(v: f64) -> String {
    num(v.round())
}

fn mid(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

fn curve(ops: &mut Vec<String>, c1: (f64, f64), c2: (f64, f64), e: (f64, f64)) {
    ops.push(format!("{} {} {} {} {} {} curve", rnum(c1.0), rnum(c1.1), rnum(c2.0), rnum(c2.1), rnum(e.0), rnum(e.1)));
}

fn tt_contour(pts: &[(f64, f64, bool)], ops: &mut Vec<String>) {
    let n = pts.len();
    if n == 0 {
        return;
    }
    // The points after the start point, ending back at the start.
    let first = pts.first().copied().unwrap_or_default();
    let last = pts.last().copied().unwrap_or_default();
    #[allow(clippy::type_complexity)]
    let (start, rest): ((f64, f64), Vec<(f64, f64, bool)>) = if first.2 {
        let mut rest: Vec<(f64, f64, bool)> = pts.iter().skip(1).copied().collect();
        rest.push(first);
        ((first.0, first.1), rest)
    } else if last.2 {
        let mut rest: Vec<(f64, f64, bool)> = pts.iter().take(n - 1).copied().collect();
        rest.push(last);
        ((last.0, last.1), rest)
    } else {
        let m = mid((last.0, last.1), (first.0, first.1));
        let mut rest: Vec<(f64, f64, bool)> = pts.to_vec();
        rest.push((m.0, m.1, true));
        (m, rest)
    };
    ops.push(format!("{} {} move", rnum(start.0), rnum(start.1)));
    let mut cur = start;
    let mut run: Vec<(f64, f64)> = Vec::new();
    let count = rest.len();
    for (i, p) in rest.iter().enumerate() {
        if !p.2 {
            run.push((p.0, p.1));
            continue;
        }
        let end = (p.0, p.1);
        if run.is_empty() {
            if i + 1 < count {
                ops.push(format!("{} {} line", rnum(end.0), rnum(end.1)));
            }
            cur = end;
            continue;
        }
        let k = run.len();
        let mut j = 0;
        while j < k {
            let q1 = run.get(j).copied().unwrap_or(cur);
            match run.get(j + 1).copied() {
                Some(q2) => {
                    let e = match run.get(j + 2).copied() {
                        Some(q3) => mid(q2, q3),
                        None => end,
                    };
                    let c1 = (cur.0 + (q1.0 - cur.0) * 4.0 / 3.0, cur.1 + (q1.1 - cur.1) * 4.0 / 3.0);
                    let c2 = (e.0 + (q2.0 - e.0) * 4.0 / 3.0, e.1 + (q2.1 - e.1) * 4.0 / 3.0);
                    curve(ops, c1, c2, e);
                    cur = e;
                    j += 2;
                }
                None => {
                    let c1 = (cur.0 + (q1.0 - cur.0) * 2.0 / 3.0, cur.1 + (q1.1 - cur.1) * 2.0 / 3.0);
                    let c2 = (end.0 + (q1.0 - end.0) * 2.0 / 3.0, end.1 + (q1.1 - end.1) * 2.0 / 3.0);
                    curve(ops, c1, c2, end);
                    cur = end;
                    j += 1;
                }
            }
        }
        run.clear();
    }
}

// ── DICTs ───────────────────────────────────────────────────────────────────────────────────────

/// One DICT operand.
#[derive(Clone, Copy)]
struct Operand(f64);

/// The operator (12-escaped ones as `1200 + b1`) and operands of each DICT entry.
fn parse_dict(data: &[u8]) -> Vec<(u16, Vec<Operand>)> {
    let mut out = Vec::new();
    let mut stack: Vec<Operand> = Vec::new();
    let mut i = 0usize;
    let byte = |k: usize| data.get(k).copied();
    while let Some(b0) = byte(i) {
        match b0 {
            0..=21 => {
                let op = if b0 == 12 {
                    i += 1;
                    1200 + u16::from(byte(i).unwrap_or(0))
                } else {
                    u16::from(b0)
                };
                out.push((op, std::mem::take(&mut stack)));
                i += 1;
            }
            28 => {
                let v = i16::from_be_bytes([byte(i + 1).unwrap_or(0), byte(i + 2).unwrap_or(0)]);
                stack.push(Operand(f64::from(v)));
                i += 3;
            }
            29 => {
                let v = i32::from_be_bytes([byte(i + 1).unwrap_or(0), byte(i + 2).unwrap_or(0), byte(i + 3).unwrap_or(0), byte(i + 4).unwrap_or(0)]);
                stack.push(Operand(f64::from(v)));
                i += 5;
            }
            30 => {
                let mut s = String::new();
                i += 1;
                'real: while let Some(b) = byte(i) {
                    i += 1;
                    for nib in [b >> 4, b & 0x0F] {
                        match nib {
                            0..=9 => s.push(char::from(b'0' + nib)),
                            0xA => s.push('.'),
                            0xB => s.push('E'),
                            0xC => s.push_str("E-"),
                            0xE => s.push('-'),
                            0xF => break 'real,
                            _ => {}
                        }
                    }
                }
                stack.push(Operand(s.parse::<f64>().unwrap_or(0.0)));
            }
            32..=246 => {
                stack.push(Operand(f64::from(i16::from(b0) - 139)));
                i += 1;
            }
            247..=250 => {
                let b1 = i16::from(byte(i + 1).unwrap_or(0));
                stack.push(Operand(f64::from((i16::from(b0) - 247) * 256 + b1 + 108)));
                i += 2;
            }
            251..=254 => {
                let b1 = i16::from(byte(i + 1).unwrap_or(0));
                stack.push(Operand(f64::from(-(i16::from(b0) - 251) * 256 - b1 - 108)));
                i += 2;
            }
            _ => i += 1,
        }
        if stack.len() > crate::limits::MAX_CFF2_STACK {
            break;
        }
    }
    out
}

/// The items of a CFF INDEX at `pos` and the offset after it.
fn cff_index(data: &[u8], pos: usize, cff2: bool) -> Option<(Vec<&[u8]>, usize)> {
    let rd = |k: usize, n: usize| -> Option<usize> {
        let mut v = 0usize;
        for b in data.get(k..k.checked_add(n)?)? {
            v = (v << 8) | usize::from(*b);
        }
        Some(v)
    };
    let (count, head) = if cff2 { (rd(pos, 4)?, 4) } else { (rd(pos, 2)?, 2) };
    if count == 0 {
        return Some((Vec::new(), pos + head));
    }
    let off_size = rd(pos + head, 1)?;
    if !(1..=4).contains(&off_size) || count > crate::limits::MAX_SFNT / off_size {
        return None;
    }
    let offsets_at = pos + head + 1;
    let data_at = offsets_at.checked_add((count + 1).checked_mul(off_size)?)?.checked_sub(1)?;
    let mut items = Vec::with_capacity(count.min(65_536));
    let mut prev = rd(offsets_at, off_size)?;
    for k in 1..=count {
        let next = rd(offsets_at + k * off_size, off_size)?;
        items.push(data.get(data_at.checked_add(prev)?..data_at.checked_add(next)?)?);
        prev = next;
    }
    Some((items, data_at.checked_add(prev)?))
}

struct CffStrings<'a> {
    strings: Vec<&'a [u8]>,
}

impl CffStrings<'_> {
    fn get(&self, sid: f64) -> String {
        let sid = sid as usize;
        if sid < 391 {
            let s = read_fonts::ps::string::Sid::new(sid as u16);
            return s.resolve_standard().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
        }
        self.strings.get(sid - 391).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default()
    }
}

fn first(ops: &[(u16, Vec<Operand>)], op: u16) -> Option<&Vec<Operand>> {
    ops.iter().find(|(o, _)| *o == op).map(|(_, v)| v)
}

fn os2_fstype(font: &SfntFont) -> Option<u16> {
    Provider(font).os2().ok().map(|o| o.fs_type())
}

fn cff_dicts(font: &SfntFont, data: &[u8], cff2: bool, out: &mut Out) -> Result<()> {
    let bad = || TablesError::invalid(if cff2 { "the CFF2 table cannot be read" } else { "the CFF table cannot be read" });
    let hdr = usize::from(*data.get(2).ok_or_else(bad)?);
    let (names, strings, top_dict) = if cff2 {
        let len = usize::from(u16::from_be_bytes([*data.get(3).ok_or_else(bad)?, *data.get(4).ok_or_else(bad)?]));
        let top = data.get(hdr..hdr.checked_add(len).ok_or_else(bad)?).ok_or_else(bad)?;
        (Vec::new(), Vec::new(), top)
    } else {
        let (names, after) = cff_index(data, hdr, false).ok_or_else(bad)?;
        let (tops, after) = cff_index(data, after, false).ok_or_else(bad)?;
        let (strings, _) = cff_index(data, after, false).ok_or_else(bad)?;
        let top = tops.first().copied().ok_or_else(bad)?;
        (names, strings, top)
    };
    let s = CffStrings { strings };
    let ops = parse_dict(top_dict);
    let num_glyphs = crate::ttx::num_glyphs(font).unwrap_or(0);
    out.line("## Top Dict")?;
    let sid_keys: [(u16, &str); 6] = [(0, "version"), (1, "Notice"), (1200, "Copyright"), (2, "FullName"), (3, "FamilyName"), (4, "Weight")];
    for (op, k) in sid_keys {
        if let Some(v) = first(&ops, op).and_then(|v| v.first()) {
            key(out, k, &format!("\"{}\"", s.get(v.0)))?;
        }
    }
    let scalar_keys: [(u16, &str); 6] = [
        (1201, "isFixedPitch"),
        (1202, "ItalicAngle"),
        (1203, "UnderlinePosition"),
        (1204, "UnderlineThickness"),
        (1205, "PaintType"),
        (1206, "CharstringType"),
    ];
    for (op, k) in scalar_keys {
        if let Some(v) = first(&ops, op).and_then(|v| v.first()) {
            key(out, k, &dict_num(v.0))?;
        }
    }
    let is_cid = first(&ops, 1230).is_some();
    if let Some(v) = first(&ops, 13).and_then(|v| v.first()) {
        key(out, "UniqueID", &dict_num(v.0))?;
    }
    if let Some(v) = first(&ops, 5) {
        key(out, "FontBBox", &array(&v.iter().map(|o| o.0).collect::<Vec<_>>()))?;
    }
    if let Some(v) = first(&ops, 1208).and_then(|v| v.first()) {
        key(out, "StrokeWidth", &dict_num(v.0))?;
    }
    if let Some(v) = first(&ops, 14) {
        key(out, "XUID", &array(&v.iter().map(|o| o.0).collect::<Vec<_>>()))?;
    }
    if let Some(v) = first(&ops, 1221).and_then(|v| v.first()) {
        key(out, "PostScript", &format!("\"{}\"", s.get(v.0)))?;
    }
    if let Some(v) = first(&ops, 1222).and_then(|v| v.first()) {
        key(out, "BaseFontName", &format!("\"{}\"", s.get(v.0)))?;
    }
    if let Some(fs) = os2_fstype(font) {
        key(out, "FSType", &fs.to_string())?;
    }
    if is_cid {
        if let Some(name) = names.first() {
            key(out, "cid.CIDFontName", &format!("\"{}\"", String::from_utf8_lossy(name)))?;
        }
        if let Some(v) = first(&ops, 1230) {
            let get = |k: usize| v.get(k).map(|o| o.0).unwrap_or(0.0);
            key(out, "cid.Registry", &format!("\"{}\"", s.get(get(0))))?;
            key(out, "cid.Ordering", &format!("\"{}\"", s.get(get(1))))?;
            key(out, "cid.Supplement", &dict_num(get(2)))?;
        }
        if let Some(v) = first(&ops, 1231).and_then(|v| v.first()) {
            key(out, "cid.CIDFontVersion", &dict_num(v.0))?;
        }
    }
    let src = match (cff2, is_cid) {
        (true, _) => "CFF2 (CID-keyed)",
        (false, true) => "CFF (CID-keyed)",
        (false, false) => "CFF (name-keyed)",
    };
    key(out, "sup.srcFontType", src)?;
    if let Ok(head) = Provider(font).head()
        && head.units_per_em() != 1000
        && cff2
    {
        key(out, "sup.UnitsPerEm", &head.units_per_em().to_string())?;
    }
    key(out, "sup.nGlyphs", &num_glyphs.to_string())?;

    // Font DICTs: the FDArray of a CID-keyed or CFF2 font, else the Top DICT itself.
    let fd_dicts: Vec<&[u8]> = match first(&ops, 1236).and_then(|v| v.first()) {
        Some(off) if off.0 >= 0.0 => cff_index(data, off.0 as usize, cff2).map(|(v, _)| v).unwrap_or_default(),
        _ => vec![top_dict],
    };
    for (i, fd) in fd_dicts.iter().enumerate().take(256) {
        out.line(&format!("## FontDict[{i}]"))?;
        let fops = if fd.as_ptr() == top_dict.as_ptr() { ops.clone() } else { parse_dict(fd) };
        if let Some(v) = first(&fops, 1238).and_then(|v| v.first()) {
            key(out, "FontName", &format!("\"{}\"", s.get(v.0)))?;
        } else if !is_cid && let Some(name) = names.first() {
            key(out, "FontName", &format!("\"{}\"", String::from_utf8_lossy(name)))?;
        }
        if let Some(v) = first(&fops, 1207) {
            let m: Vec<f64> = v.iter().map(|o| o.0).collect();
            if m != [0.001, 0.0, 0.0, 0.001, 0.0, 0.0] {
                key(out, "FontMatrix", &array(&m))?;
            }
        }
        out.line("## Private")?;
        let Some(range) = first(&fops, 18) else { continue };
        let (size, off) = (range.first().map(|o| o.0).unwrap_or(0.0), range.get(1).map(|o| o.0).unwrap_or(0.0));
        if size < 0.0 || off < 0.0 {
            continue;
        }
        let (size, off) = (size as usize, off as usize);
        let Some(private) = data.get(off..off.saturating_add(size)) else { continue };
        let pops = parse_dict(private);
        let delta = |v: &Vec<Operand>| -> Vec<f64> {
            let mut acc = 0.0;
            v.iter()
                .map(|o| {
                    acc += o.0;
                    acc
                })
                .collect()
        };
        let private_keys: [(u16, &str, bool); 15] = [
            (6, "BlueValues", true),
            (7, "OtherBlues", true),
            (8, "FamilyBlues", true),
            (9, "FamilyOtherBlues", true),
            (1209, "BlueScale", false),
            (1210, "BlueShift", false),
            (1211, "BlueFuzz", false),
            (10, "StdHW", false),
            (11, "StdVW", false),
            (1212, "StemSnapH", true),
            (1213, "StemSnapV", true),
            (1214, "ForceBold", false),
            (1217, "LanguageGroup", false),
            (1218, "ExpansionFactor", false),
            (1219, "initialRandomSeed", false),
        ];
        for (op, k, is_array) in private_keys {
            let Some(v) = first(&pops, op) else { continue };
            if is_array {
                key(out, k, &array(&delta(v)))?;
            } else if let Some(x) = v.first() {
                key(out, k, &dict_num(x.0))?;
            }
        }
    }
    Ok(())
}

/// The DICT values a TrueType font implies, as `tx` gathers them.
fn truetype_dict(font: &SfntFont, out: &mut Out) -> Result<()> {
    let p = Provider(font);
    let head = p.head().map_err(|e| TablesError::Invalid(format!("the head table cannot be read: {e}")))?;
    let name = |id: u16| -> Option<String> { name_string(font, id) };
    out.line("## Top Dict")?;
    key(out, "version", &format!("\"{:.3}\"", head.font_revision().to_f64()))?;
    for (id, k) in [(7u16, "Notice"), (0, "Copyright"), (4, "FullName"), (1, "FamilyName"), (2, "Weight")] {
        if let Some(v) = name(id) {
            key(out, k, &format!("\"{v}\""))?;
        }
    }
    if let Ok(post) = p.post() {
        if post.is_fixed_pitch() != 0 {
            key(out, "isFixedPitch", "1")?;
        }
        let angle = post.italic_angle().to_f64();
        if angle != 0.0 {
            key(out, "ItalicAngle", &dict_num(angle))?;
        }
        let thickness = f64::from(post.underline_thickness().to_i16());
        let position = f64::from(post.underline_position().to_i16()) - (thickness / 2.0).trunc();
        if position != -100.0 {
            key(out, "UnderlinePosition", &dict_num(position))?;
        }
        if thickness != 50.0 {
            key(out, "UnderlineThickness", &dict_num(thickness))?;
        }
    }
    key(out, "FontBBox", &format!("{{{},{},{},{}}}", head.x_min(), head.y_min(), head.x_max(), head.y_max()))?;
    if let Some(fs) = os2_fstype(font) {
        key(out, "FSType", &fs.to_string())?;
    }
    key(out, "sup.srcFontType", "TrueType")?;
    let upem = head.units_per_em();
    if upem != 1000 {
        key(out, "sup.UnitsPerEm", &upem.to_string())?;
    }
    key(out, "sup.nGlyphs", &crate::ttx::num_glyphs(font).unwrap_or(0).to_string())?;
    out.line("## FontDict[0]")?;
    if let Some(v) = name(6) {
        key(out, "FontName", &format!("\"{v}\""))?;
    }
    if upem != 1000 && upem != 0 {
        let s = 1.0 / f64::from(upem);
        key(out, "FontMatrix", &array(&[s, 0.0, 0.0, s, 0.0, 0.0]))?;
    }
    out.line("## Private")?;
    Ok(())
}

/// A name record as text: Windows English first, then Macintosh Roman.
fn name_string(font: &SfntFont, id: u16) -> Option<String> {
    let name = Provider(font).name().ok()?;
    let data = name.string_data();
    let mut mac = None;
    for rec in name.name_record() {
        if rec.name_id().to_u16() != id {
            continue;
        }
        let (platform, encoding, lang) = (rec.platform_id(), rec.encoding_id(), rec.language_id());
        if platform == 3 && (encoding == 1 || encoding == 10) && lang == 0x409 {
            return rec.string(data).ok().map(|s| s.chars().collect());
        }
        if platform == 1 && encoding == 0 && lang == 0 && mac.is_none() {
            mac = rec.string(data).ok().map(|s| s.chars().collect::<String>());
        }
    }
    mac
}
