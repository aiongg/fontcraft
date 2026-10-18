//! The glyph tables of the result: `glyf`, `loca`, `hmtx`, `vmtx`, and the glyph bounds the
//! font-wide tables are recomputed from (fontTools recomputes them when it saves the merged
//! font).

use std::borrow::Cow;

use super::plan::Plan;
use crate::dehint::{DehintOptions, dehint};
use crate::reorder::be::{Data, put_u16, put_u32, set_u16, to_u16};
use crate::reorder::names::num_glyphs;
use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// One input font's glyphs and metrics.
pub(super) struct Source<'a> {
    glyphs: Vec<Cow<'a, [u8]>>,
    h_metrics: Vec<(u16, i16)>,
    v_metrics: Option<Vec<(u16, i16)>>,
}

impl<'a> Source<'a> {
    /// The glyphs of font `index`. Every font but the first loses its glyph instructions: its
    /// font programs and `cvt ` are not carried over, so the instructions would call into the
    /// first font's.
    pub fn read(font: &'a SfntFont, index: usize) -> Result<Self> {
        let n = num_glyphs(font)?;
        let mut glyphs = split_glyf(font, n)?.into_iter().map(Cow::Borrowed).collect::<Vec<_>>();
        if index > 0 {
            let mut bare = SfntFont::new(font.flavour);
            for tag in [*b"head", *b"maxp", *b"glyf", *b"loca"] {
                if let Some(bytes) = font.get(tag) {
                    bare.insert(tag, bytes.to_vec());
                }
            }
            let options = DehintOptions { keep_font_hints: true, hinting_tables: Vec::new(), ..DehintOptions::default() };
            let stripped = dehint(&bare, &options)?;
            glyphs = split_glyf(&stripped, n)?.into_iter().map(|g| Cow::Owned(g.to_vec())).collect();
        }
        let h_metrics = metrics(font, *b"hhea", *b"hmtx", n)?.ok_or_else(|| TablesError::MissingTable("hmtx".into()))?;
        let v_metrics = metrics(font, *b"vhea", *b"vmtx", n)?;
        Ok(Source { glyphs, h_metrics, v_metrics })
    }

    pub fn glyph(&self, gid: u16) -> &[u8] {
        self.glyphs.get(usize::from(gid)).map(|g| g.as_ref()).unwrap_or_default()
    }

    pub fn metric(&self, gid: u16) -> (u16, i16) {
        self.h_metrics.get(usize::from(gid)).copied().unwrap_or_default()
    }
}

fn split_glyf(font: &SfntFont, n: usize) -> Result<Vec<&[u8]>> {
    let (Some(glyf), Some(loca), Some(head)) = (font.get(*b"glyf"), font.get(*b"loca"), font.get(*b"head")) else {
        return Err(TablesError::MissingTable("glyf".into()));
    };
    let long = Data::new(*b"head", head).i16(50)? != 0;
    let l = Data::new(*b"loca", loca);
    let g = Data::new(*b"glyf", glyf);
    let at = |i: usize| -> Result<usize> { if long { l.u32_usize(4 * i) } else { Ok(2 * usize::from(l.u16(2 * i)?)) } };
    let mut out = Vec::with_capacity(n);
    let mut start = at(0)?;
    for i in 1..=n {
        let end = at(i)?;
        if end < start {
            return Err(l.bad("has decreasing offsets"));
        }
        out.push(g.slice(start, end - start)?);
        start = end;
    }
    Ok(out)
}

/// (advance, side bearing) of every glyph from `hmtx` or `vmtx`; `None` without the tables.
fn metrics(font: &SfntFont, hea: [u8; 4], mtx: [u8; 4], n: usize) -> Result<Option<Vec<(u16, i16)>>> {
    let (Some(header), Some(table)) = (font.get(hea), font.get(mtx)) else { return Ok(None) };
    let long = usize::from(Data::new(hea, header).u16(34)?);
    let d = Data::new(mtx, table);
    if long == 0 || long > n {
        return Err(d.bad(format!("has {long} long metrics for {n} glyphs")));
    }
    let mut out = Vec::with_capacity(n);
    let mut advance = 0u16;
    for g in 0..n {
        if g < long {
            advance = d.u16(4 * g)?;
            out.push((advance, d.i16(4 * g + 2)?));
        } else {
            out.push((advance, d.i16(4 * long + 2 * (g - long))?));
        }
    }
    Ok(Some(out))
}

/// The glyph tables of the result.
pub(super) struct Built {
    pub count: usize,
    pub glyf: Vec<u8>,
    pub loca: Vec<u8>,
    /// `head.indexToLocFormat`: 1 for long offsets.
    pub long_loca: bool,
    pub hmtx: Vec<u8>,
    pub h_metrics: Vec<(u16, i16)>,
    /// `vmtx` and its metrics, when every font has vertical metrics.
    pub vertical: Option<(Vec<u8>, Vec<(u16, i16)>)>,
    /// (xMin, yMin, xMax, yMax) of every glyph with outlines.
    pub bounds: Vec<Option<[i16; 4]>>,
}

pub(super) fn build(sources: &[Source<'_>], plan: &Plan, warnings: &mut Vec<String>) -> Result<Built> {
    let mut glyf: Vec<u8> = Vec::new();
    let mut offsets: Vec<usize> = Vec::with_capacity(plan.slots.len() + 1);
    let mut h_metrics = Vec::with_capacity(plan.slots.len());
    let mut bounds = Vec::with_capacity(plan.slots.len());
    for slot in &plan.slots {
        let source = sources.get(slot.font).ok_or_else(|| TablesError::invalid("a glyph comes from a font that was not given"))?;
        let mut data = source.glyph(slot.gid).to_vec();
        remap_components(&mut data, |old| {
            plan.new_id(slot.font, old).ok_or_else(|| {
                TablesError::BadOptions(format!(
                    "glyph `{}` (font {}) uses a component the result does not take: add the component to the map",
                    slot.name,
                    slot.font + 1
                ))
            })
        })?;
        bounds.push(glyph_bounds(&data)?);
        offsets.push(glyf.len());
        glyf.extend_from_slice(&data);
        while glyf.len() % 4 != 0 {
            glyf.push(0);
        }
        h_metrics.push(source.metric(slot.gid));
    }
    offsets.push(glyf.len());
    if glyf.len() > crate::limits::MAX_SFNT {
        return Err(TablesError::TooLarge { what: "the merged glyf table", limit: crate::limits::MAX_SFNT });
    }
    // fontTools' rule: short offsets whenever they reach.
    let long_loca = glyf.len() / 2 > usize::from(u16::MAX);
    let mut loca = Vec::with_capacity(offsets.len() * if long_loca { 4 } else { 2 });
    for at in offsets {
        if long_loca {
            put_u32(&mut loca, u32::try_from(at).map_err(|_| TablesError::invalid("the merged glyf table is too large"))?);
        } else {
            put_u16(&mut loca, to_u16(at / 2, "a short loca offset")?);
        }
    }
    let vertical = if sources.iter().all(|s| s.v_metrics.is_some()) {
        let v: Vec<(u16, i16)> = plan
            .slots
            .iter()
            .map(|s| sources.get(s.font).and_then(|src| src.v_metrics.as_ref()).and_then(|m| m.get(usize::from(s.gid))).copied().unwrap_or_default())
            .collect();
        Some((write_metrics(&v)?.0, v))
    } else {
        if sources.iter().any(|s| s.v_metrics.is_some()) {
            warnings.push("'vmtx' and 'vhea' are left out: not every font has vertical metrics".into());
        }
        None
    };
    let (hmtx, _) = write_metrics(&h_metrics)?;
    Ok(Built { count: plan.slots.len(), glyf, loca, long_loca, hmtx, h_metrics, vertical, bounds })
}

/// `hmtx` or `vmtx` bytes and the count of long metrics: trailing glyphs with the advance of the
/// last share it.
pub(super) fn write_metrics(all: &[(u16, i16)]) -> Result<(Vec<u8>, u16)> {
    let final_advance = all.last().map_or(0, |m| m.0);
    let mut count = all.len();
    while count >= 2 && all.get(count - 2).is_some_and(|m| m.0 == final_advance) {
        count -= 1;
    }
    let mut out = Vec::with_capacity(4 * count + 2 * (all.len() - count));
    for (i, (advance, side)) in all.iter().enumerate() {
        if i < count {
            put_u16(&mut out, *advance);
        }
        out.extend_from_slice(&side.to_be_bytes());
    }
    Ok((out, to_u16(count, "the long metrics count")?))
}

/// The long-metric count [`write_metrics`] gives.
pub(super) fn long_metric_count(all: &[(u16, i16)]) -> Result<u16> {
    let final_advance = all.last().map_or(0, |m| m.0);
    let mut count = all.len();
    while count >= 2 && all.get(count - 2).is_some_and(|m| m.0 == final_advance) {
        count -= 1;
    }
    to_u16(count, "the long metrics count")
}

/// The bounding box a glyph's header stores; `None` for an empty glyph.
fn glyph_bounds(data: &[u8]) -> Result<Option<[i16; 4]>> {
    if data.is_empty() {
        return Ok(None);
    }
    let d = Data::new(*b"glyf", data);
    if d.i16(0)? == 0 {
        return Ok(None);
    }
    Ok(Some([d.i16(2)?, d.i16(4)?, d.i16(6)?, d.i16(8)?]))
}

/// Rewrites the component glyph ids of a composite glyph in place.
fn remap_components(glyph: &mut [u8], map: impl Fn(u16) -> Result<u16>) -> Result<()> {
    if glyph.len() < 10 || Data::new(*b"glyf", glyph).i16(0)? >= 0 {
        return Ok(());
    }
    let mut at = 10usize;
    loop {
        let d = Data::new(*b"glyf", glyph);
        let flags = d.u16(at)?;
        let new = map(d.u16(at + 2)?)?;
        set_u16(glyph, at + 2, new)?;
        at += 4 + if flags & 0x0001 != 0 { 4 } else { 2 };
        if flags & 0x0008 != 0 {
            at += 2;
        } else if flags & 0x0040 != 0 {
            at += 4;
        } else if flags & 0x0080 != 0 {
            at += 8;
        }
        if flags & 0x0020 == 0 {
            return Ok(());
        }
        if at >= glyph.len() {
            return Err(TablesError::invalid("a composite glyph is cut short"));
        }
    }
}

/// `OS/2.xAvgCharWidth` as fontTools recomputes it: the rounded mean of the advances that are
/// not zero.
pub(super) fn average_advance(metrics: &[(u16, i16)]) -> u16 {
    let widths: Vec<u64> = metrics.iter().filter(|m| m.0 > 0).map(|m| u64::from(m.0)).collect();
    if widths.is_empty() {
        return 0;
    }
    let sum: u64 = widths.iter().sum();
    let n = widths.len() as u64;
    // Round half up, as fontTools' otRound does for positive values.
    u16::try_from((2 * sum + n) / (2 * n)).unwrap_or(u16::MAX)
}
