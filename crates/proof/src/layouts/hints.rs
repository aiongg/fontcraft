//! The PostScript hints the `hints` layout draws: each glyph's stems with the hint-replacement
//! groups that use them, and the alignment zones of BlueValues. They are read from a `CFF ` table
//! through the decompiler's charstring reader; a TrueType or `CFF2` font has none.
//!
//! As in the reference script, a group is the set of stems one hint mask turns on; a glyph
//! without hint replacement has one group. Counter hints are not drawn.

use fontcraft_decompile::type1::{PsFont, PsKind, PsOp, StemKind};
use skrifa::Tag;

use crate::font::Face;

/// One stem hint: edges `pos` and `pos + width` in charstring order (a ghost hint has width −20
/// or −21).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stem {
    pub horizontal: bool,
    pub pos: f64,
    pub width: f64,
}

/// A glyph's stems in declaration order and its hint-replacement groups (indices into `stems`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct GlyphHints {
    pub stems: Vec<Stem>,
    pub groups: Vec<Vec<usize>>,
}

/// The hints of a font, by glyph id.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FontHints {
    pub glyphs: Vec<GlyphHints>,
    /// BlueValues as `(bottom, top)` pairs, sorted.
    pub zones: Vec<(f64, f64)>,
    /// The font is CID-keyed, which moves the default baseline of a tile to −120.
    pub cid: bool,
}

impl FontHints {
    pub(crate) fn glyph(&self, gid: u32) -> Option<&GlyphHints> {
        self.glyphs.get(gid as usize)
    }
}

/// The hints of `face`; empty when the font has no `CFF ` table or it cannot be read.
pub(crate) fn read(face: &Face<'_>) -> FontHints {
    let Some(data) = face.font.table_data(Tag::new(b"CFF ")) else { return FontHints::default() };
    let mut warnings = Vec::new();
    match fontcraft_decompile::bare_cff::read_cff(data.as_bytes(), &mut warnings) {
        Ok(font) => from_ps(&font),
        Err(_) => FontHints::default(),
    }
}

fn from_ps(font: &PsFont) -> FontHints {
    let mut zones: Vec<(f64, f64)> = font.private.blue_values.chunks_exact(2).filter_map(|p| Some((*p.first()?, *p.get(1)?))).collect();
    zones.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let glyphs = font
        .glyphs
        .iter()
        .map(|g| {
            let stems: Vec<Stem> = g.stems.iter().filter_map(stem).collect();
            let mut groups: Vec<Vec<usize>> = Vec::new();
            let mut current: Vec<usize> = Vec::new();
            for op in &g.ops {
                match op {
                    PsOp::NewHints => {
                        if !current.is_empty() {
                            groups.push(std::mem::take(&mut current));
                        }
                    }
                    PsOp::HStem { .. } | PsOp::VStem { .. } => {
                        let Some(s) = stem(op) else { continue };
                        if let Some(i) = stems.iter().position(|d| *d == s)
                            && !current.contains(&i)
                        {
                            current.push(i);
                        }
                    }
                    _ => {}
                }
            }
            if !current.is_empty() {
                groups.push(current);
            }
            GlyphHints { stems, groups }
        })
        .collect();
    FontHints { glyphs, zones, cid: font.kind == PsKind::CffCid }
}

/// A stem or ghost hint; `None` for a counter hint or another operation.
fn stem(op: &PsOp) -> Option<Stem> {
    match *op {
        PsOp::HStem { kind: StemKind::Counter, .. } | PsOp::VStem { kind: StemKind::Counter, .. } => None,
        PsOp::HStem { y, dy, .. } => Some(Stem { horizontal: true, pos: y, width: dy }),
        PsOp::VStem { x, dx, .. } => Some(Stem { horizontal: false, pos: x, width: dx }),
        _ => None,
    }
}
