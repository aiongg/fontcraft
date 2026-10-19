//! Subsetting `MATH`, ported from fontTools' subsetter (MIT, Copyright 2013 Google, Inc.).
//!
//! The closure adds, in one pass, the size variants and assembly parts of every glyph the
//! vertical or horizontal variant coverage lists. Italic corrections, top accent attachments,
//! kerning and the extended shape coverage keep the glyphs after that closure; the variant
//! lists keep the glyphs before it. Subtables left empty stay, as pyftsubset leaves them.
//!
//! Before the table is read into owned types, [`check`] counts what the conversion would build
//! (shared subtables count once per reference) and refuses a table past
//! [`MAX_LAYOUT_ITEMS`].

use std::collections::{BTreeSet, HashMap};
use std::mem::take;

use read_fonts::{FontData, FontRead};
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::layout::CoverageTable;
use write_fonts::tables::math::{Math, MathGlyphConstruction};
use write_fonts::types::GlyphId16;

use super::super::Plan;
use super::super::layout::coverage;
use super::close_over;
use crate::limits::MAX_LAYOUT_ITEMS;
use crate::reorder::be::Data;
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// Adds the variants and assembly parts of the glyphs in `glyphs` (one pass, as pyftsubset).
pub(crate) fn close_glyphs(font: &SfntFont, glyphs: &mut BTreeSet<u32>, n: usize) -> Result<()> {
    let Some(bytes) = font.get(*b"MATH") else { return Ok(()) };
    let d = Data::new(*b"MATH", bytes);
    let variants = usize::from(d.u16(8)?);
    if variants == 0 {
        return Ok(());
    }
    let vert = usize::from(d.u16(variants + 6)?);
    let mut lists: Vec<(HashMap<u16, usize>, Vec<usize>)> = Vec::new();
    // Vertical then horizontal: coverage offset, count, first construction offset.
    for (cov, count, first) in [(2, 6, 10), (4, 8, 10 + 2 * vert)] {
        let cov_at = usize::from(d.u16(variants + cov)?);
        if cov_at == 0 {
            continue;
        }
        let mut index = HashMap::new();
        for (i, g) in coverage_glyphs(d, variants + cov_at)?.into_iter().enumerate() {
            index.entry(g).or_insert(i);
        }
        let n_here = usize::from(d.u16(variants + count)?);
        let mut constructions = Vec::with_capacity(n_here);
        for i in 0..n_here {
            constructions.push(variants + usize::from(d.u16(variants + first + 2 * i)?));
        }
        lists.push((index, constructions));
    }
    close_over(glyphs, n, false, |g, out| {
        for (index, constructions) in &lists {
            let Some(c) = u16::try_from(g).ok().and_then(|g| index.get(&g)).and_then(|i| constructions.get(*i)) else { continue };
            construction_glyphs(d, *c, out)?;
        }
        Ok(())
    })
}

/// The glyphs a coverage table lists, in its order.
fn coverage_glyphs(d: Data<'_>, at: usize) -> Result<Vec<u16>> {
    let format = d.u16(at)?;
    let count = usize::from(d.u16(at + 2)?);
    let mut out = Vec::new();
    match format {
        1 => {
            for i in 0..count {
                out.push(d.u16(at + 4 + 2 * i)?);
            }
        }
        2 => {
            for i in 0..count {
                let (start, end) = (d.u16(at + 4 + 6 * i)?, d.u16(at + 6 + 6 * i)?);
                out.extend(start..=end);
                if out.len() > 1 << 16 {
                    return Err(d.bad("has a coverage table with overlapping ranges"));
                }
            }
        }
        _ => return Err(d.bad(format!("has a coverage table of format {format}"))),
    }
    Ok(out)
}

/// The variant glyphs and assembly part glyphs of the construction at `at`.
fn construction_glyphs(d: Data<'_>, at: usize, out: &mut Vec<u32>) -> Result<()> {
    let assembly = usize::from(d.u16(at)?);
    let count = usize::from(d.u16(at + 2)?);
    for i in 0..count {
        out.push(u32::from(d.u16(at + 4 + 4 * i)?));
    }
    if assembly != 0 {
        let a = at + assembly;
        let parts = usize::from(d.u16(a + 4)?);
        for i in 0..parts {
            out.push(u32::from(d.u16(a + 6 + 10 * i)?));
        }
    }
    Ok(())
}

/// Writes `MATH` subset to the plan's glyphs, with glyph ids in the output numbering.
pub(crate) fn subset(plan: &Plan, bytes: &[u8]) -> Result<Vec<u8>> {
    check(bytes)?;
    let table = read_fonts::tables::math::Math::read(FontData::new(bytes))
        .map_err(|e| TablesError::invalid(format!("the 'MATH' table cannot be read: {e}")))?;
    let mut m: Math = table.to_owned_table();
    let new = |g: u16| plan.glyph_map.get(&u32::from(g)).and_then(|n| u16::try_from(*n).ok());
    let in_math = |g: u16| plan.math_glyphs.contains(&u32::from(g));
    let in_pre = |g: u16| plan.pre_math_glyphs.contains(&u32::from(g));

    let info = &mut *m.math_glyph_info;
    if let Some(t) = info.math_italics_correction_info.as_mut() {
        let (cov, list) = keep(&t.coverage, take(&mut t.italics_correction), in_math, new);
        t.coverage = cov.into();
        t.italics_correction = list;
    }
    if let Some(t) = info.math_top_accent_attachment.as_mut() {
        let (cov, list) = keep(&t.top_accent_coverage, take(&mut t.top_accent_attachment), in_math, new);
        t.top_accent_coverage = cov.into();
        t.top_accent_attachment = list;
    }
    if let Some(t) = info.math_kern_info.as_mut() {
        let (cov, list) = keep(&t.math_kern_coverage, take(&mut t.math_kern_info_records), in_math, new);
        t.math_kern_coverage = cov.into();
        t.math_kern_info_records = list;
    }
    if let Some(c) = info.extended_shape_coverage.as_mut() {
        let glyphs: Vec<u16> = c.iter().map(|g| g.to_u16()).filter(|g| in_math(*g)).filter_map(new).collect();
        *c = coverage(&glyphs);
    }

    let v = &mut *m.math_variants;
    for (cov, list) in
        [(&mut v.vert_glyph_coverage, &mut v.vert_glyph_constructions), (&mut v.horiz_glyph_coverage, &mut v.horiz_glyph_constructions)]
    {
        let Some(c) = cov.as_mut() else { continue };
        let (new_cov, mut kept) = keep(c, take(list), in_pre, new);
        *c = new_cov;
        for construction in &mut kept {
            remap_construction(construction, new)?;
        }
        *list = kept;
    }
    write_fonts::dump_table(&m).map_err(|e| TablesError::invalid(format!("the 'MATH' table cannot be written: {e}")))
}

/// The coverage entries of `c` (and the parallel `list`) that `keep_glyph` keeps, the coverage
/// in the output numbering.
fn keep<T>(c: &CoverageTable, list: Vec<T>, keep_glyph: impl Fn(u16) -> bool, new: impl Fn(u16) -> Option<u16>) -> (CoverageTable, Vec<T>) {
    let mut glyphs = Vec::new();
    let mut out = Vec::new();
    for (g, item) in c.iter().zip(list) {
        let g = g.to_u16();
        if keep_glyph(g)
            && let Some(n) = new(g)
        {
            glyphs.push(n);
            out.push(item);
        }
    }
    (coverage(&glyphs), out)
}

fn remap_construction(c: &mut MathGlyphConstruction, new: impl Fn(u16) -> Option<u16>) -> Result<()> {
    let missing = |g: GlyphId16| TablesError::invalid(format!("a MATH variant uses glyph {g}, which the subset lacks"));
    for r in &mut c.math_glyph_variant_records {
        r.variant_glyph = GlyphId16::new(new(r.variant_glyph.to_u16()).ok_or_else(|| missing(r.variant_glyph))?);
    }
    if let Some(a) = c.glyph_assembly.as_mut() {
        for p in &mut a.part_records {
            p.glyph_id = GlyphId16::new(new(p.glyph_id.to_u16()).ok_or_else(|| missing(p.glyph_id))?);
        }
    }
    Ok(())
}

// ── Size check ──

struct Walk<'a> {
    d: Data<'a>,
    left: usize,
    kerns: HashMap<usize, usize>,
    constructions: HashMap<usize, usize>,
}

impl Walk<'_> {
    fn charge(&mut self, n: usize) -> Result<()> {
        self.left = self.left.checked_sub(n).ok_or_else(|| {
            TablesError::invalid(format!("the 'MATH' table is too large to rewrite: it expands to more than {MAX_LAYOUT_ITEMS} records"))
        })?;
        Ok(())
    }

    /// What one value record at `at` builds, its device resolved from `parent`.
    fn value(&self, parent: usize, at: usize) -> usize {
        let off = self.d.u16(at + 2).map(usize::from).unwrap_or(0);
        if off == 0 {
            return 1;
        }
        let dev = parent + off;
        match (self.d.u16(dev), self.d.u16(dev + 2), self.d.u16(dev + 4)) {
            (Ok(start), Ok(end), Ok(1..=3)) => 2 + usize::from(end.saturating_sub(start)),
            _ => 2,
        }
    }

    fn values(&self, parent: usize, at: usize, count: usize) -> usize {
        (0..count).map(|i| self.value(parent, at + 4 * i)).sum()
    }

    fn kern(&mut self, at: usize) -> usize {
        if let Some(c) = self.kerns.get(&at) {
            return *c;
        }
        let h = self.d.u16(at).map(usize::from).unwrap_or(0);
        let c = 1 + self.values(at, at + 2, 2 * h + 1);
        self.kerns.insert(at, c);
        c
    }

    fn construction(&mut self, at: usize) -> usize {
        if let Some(c) = self.constructions.get(&at) {
            return *c;
        }
        let assembly = self.d.u16(at).map(usize::from).unwrap_or(0);
        let mut c = 1 + self.d.u16(at + 2).map(usize::from).unwrap_or(0);
        if assembly != 0 {
            let a = at + assembly;
            c += self.value(a, a) + self.d.u16(a + 4).map(usize::from).unwrap_or(0);
        }
        self.constructions.insert(at, c);
        c
    }
}

/// Refuses a `MATH` table whose owned form would hold more than [`MAX_LAYOUT_ITEMS`] records.
fn check(bytes: &[u8]) -> Result<()> {
    let d = Data::new(*b"MATH", bytes);
    let mut w = Walk { d, left: MAX_LAYOUT_ITEMS, kerns: HashMap::new(), constructions: HashMap::new() };
    let constants = usize::from(d.u16(4)?);
    if constants != 0 {
        let c = w.values(constants, constants + 8, 51);
        w.charge(c)?;
    }
    let info = usize::from(d.u16(6)?);
    if info != 0 {
        for field in [0, 2] {
            let off = usize::from(d.u16(info + field)?);
            if off != 0 {
                let t = info + off;
                let c = w.values(t, t + 4, usize::from(d.u16(t + 2)?));
                w.charge(c)?;
            }
        }
        let off = usize::from(d.u16(info + 6)?);
        if off != 0 {
            let t = info + off;
            for i in 0..usize::from(d.u16(t + 2)?) {
                for k in 0..4 {
                    let kern = usize::from(d.u16(t + 4 + 8 * i + 2 * k)?);
                    if kern != 0 {
                        let c = w.kern(t + kern);
                        w.charge(c)?;
                    }
                }
            }
        }
    }
    let variants = usize::from(d.u16(8)?);
    if variants != 0 {
        let count = usize::from(d.u16(variants + 6)?) + usize::from(d.u16(variants + 8)?);
        for i in 0..count {
            let c = w.construction(variants + usize::from(d.u16(variants + 10 + 2 * i)?));
            w.charge(c)?;
        }
    }
    Ok(())
}
