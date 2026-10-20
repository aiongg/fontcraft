//! Subsetting `cmap`, `glyf`, `loca`, `CFF `, `CFF2` (through [`super::cff2`]), `hmtx`, `vmtx`, `post`, `name`, `OS/2`, `maxp`,
//! `head`, `hhea`, `vhea` and the font-wide hinting tables (M12.17). Ported from the table
//! methods of fontTools' subsetter (MIT).
//!
//! Tables pyftsubset copies without subsetting (`head`, `hhea`, `vhea`, `gasp`, `cvt `,
//! `fpgm`, `prep`, `VDMX`) are copied, with the counts and flags that follow the glyph set
//! updated: `numGlyphs`, the long-metric counts, `indexToLocFormat`. Bounding boxes and the
//! `maxp` maxima stay as they are, as pyftsubset leaves them by default.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use read_fonts::tables::cmap::{Cmap, CmapSubtable, PlatformId};
use read_fonts::{FontData, FontRead};

use super::closure::is_unicode;
use super::{Plan, cs};
use crate::limits::MAX_CODE_POINTS;
use crate::reorder::be::{Data, put_u16, put_u32, set_u16, to_u16, to_u32};
use crate::reorder::cmap::{write_format4, write_format12};
use crate::reorder::{cff, names};
use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// Tables this module writes or copies.
const CORE: [Tag; 18] = [
    *b"cmap", *b"glyf", *b"loca", *b"CFF ", *b"hmtx", *b"hhea", *b"vmtx", *b"vhea", *b"post", *b"name", *b"OS/2", *b"maxp", *b"head", *b"gasp",
    *b"cvt ", *b"fpgm", *b"prep", *b"VDMX",
];

/// Copied as they are.
const COPIED: [Tag; 5] = [*b"gasp", *b"cvt ", *b"fpgm", *b"prep", *b"VDMX"];

/// `tag` is one of the tables this module handles.
pub fn is_core(tag: Tag) -> bool {
    CORE.contains(&tag)
}

/// Writes the subset of the tables this module handles from `input` into `output`.
pub fn apply(plan: &Plan, input: &SfntFont, output: &mut SfntFont) -> Result<()> {
    let n_out = plan.new_to_old.len();
    for tag in COPIED {
        if let Some(bytes) = input.get(tag) {
            output.insert(tag, bytes.to_vec());
        }
    }
    let mut head = input.get(*b"head").map(<[u8]>::to_vec);
    if let Some(glyf) = Glyf::read(input, plan.num_glyphs)? {
        let (g, l, long) = glyf.subset(plan)?;
        output.insert(*b"glyf", g);
        output.insert(*b"loca", l);
        if let Some(head) = head.as_mut() {
            set_u16(head, 50, u16::from(long))?;
        }
    }
    if let Some(head) = head {
        output.insert(*b"head", head);
    }
    if let Some(bytes) = input.get(*b"CFF ") {
        output.insert(*b"CFF ", subset_cff(plan, bytes)?);
    }
    if let Some(bytes) = input.get(*b"CFF2") {
        output.insert(*b"CFF2", super::cff2::subset_cff2(plan, bytes)?);
    }
    for (mtx, hea) in [(*b"hmtx", *b"hhea"), (*b"vmtx", *b"vhea")] {
        let (Some(table), Some(header)) = (input.get(mtx), input.get(hea)) else {
            if let Some(header) = input.get(hea) {
                output.insert(hea, header.to_vec());
            }
            continue;
        };
        let long = usize::from(Data::new(hea, header).u16(34)?);
        let (bytes, count) = metrics(table, mtx, long, plan)?;
        let mut header = header.to_vec();
        set_u16(&mut header, 34, count)?;
        output.insert(mtx, bytes);
        output.insert(hea, header);
    }
    if let Some(maxp) = input.get(*b"maxp") {
        let mut maxp = maxp.to_vec();
        set_u16(&mut maxp, 4, to_u16(n_out, "the glyph count")?)?;
        if !plan.options.hinting && Data::new(*b"maxp", &maxp).u32(0)? == 0x0001_0000 && maxp.len() >= 28 {
            set_u16(&mut maxp, 14, 1)?;
            for at in (16..28).step_by(2) {
                set_u16(&mut maxp, at, 0)?;
            }
        }
        output.insert(*b"maxp", maxp);
    }
    if let Some(post) = input.get(*b"post") {
        output.insert(*b"post", subset_post(plan, post)?);
    }
    if let Some(cmap) = input.get(*b"cmap") {
        output.insert(*b"cmap", subset_cmap(plan, cmap)?);
    }
    if let Some(name) = input.get(*b"name") {
        output.insert(*b"name", subset_name(plan, name, &BTreeSet::new())?);
    }
    if let Some(os2) = input.get(*b"OS/2") {
        let code_points = output.get(*b"cmap").map(unicode_code_points).transpose()?.unwrap_or_default();
        output.insert(*b"OS/2", subset_os2(plan, os2, &code_points)?);
    }
    Ok(())
}

/// The glyphs of a `glyf` table, split by `loca`.
pub(crate) struct Glyf<'a> {
    glyphs: Vec<&'a [u8]>,
}

/// One component reference inside a composite glyph: where its glyph id sits.
struct Component {
    gid_at: usize,
}

/// The components of a composite glyph and the offset just past the last one, with whether
/// any component carries the instructions flag.
fn walk_composite(glyph: &[u8]) -> Result<(Vec<Component>, usize, bool)> {
    let d = Data::new(*b"glyf", glyph);
    let mut at = 10usize;
    let mut out = Vec::new();
    let mut instructions = false;
    loop {
        let flags = d.u16(at)?;
        d.u16(at + 2)?;
        out.push(Component { gid_at: at + 2 });
        instructions |= flags & 0x0100 != 0;
        at += 4 + if flags & 0x0001 != 0 { 4 } else { 2 };
        if flags & 0x0008 != 0 {
            at += 2;
        } else if flags & 0x0040 != 0 {
            at += 4;
        } else if flags & 0x0080 != 0 {
            at += 8;
        }
        if at > glyph.len() {
            return Err(TablesError::invalid("a composite glyph is cut short"));
        }
        if flags & 0x0020 == 0 {
            return Ok((out, at, instructions));
        }
    }
}

fn is_composite(glyph: &[u8]) -> Result<bool> {
    Ok(glyph.len() >= 10 && Data::new(*b"glyf", glyph).i16(0)? < 0)
}

impl<'a> Glyf<'a> {
    /// The glyphs of `font`, or `None` without `glyf`.
    pub(crate) fn read(font: &'a SfntFont, n: usize) -> Result<Option<Self>> {
        let (Some(glyf), Some(loca)) = (font.get(*b"glyf"), font.get(*b"loca")) else { return Ok(None) };
        let head = font.get(*b"head").ok_or_else(|| TablesError::MissingTable("head".into()))?;
        let long = Data::new(*b"head", head).i16(50)? != 0;
        let l = Data::new(*b"loca", loca);
        let g = Data::new(*b"glyf", glyf);
        let at = |i: usize| -> Result<usize> { if long { l.u32_usize(4 * i) } else { Ok(2 * usize::from(l.u16(2 * i)?)) } };
        let mut glyphs = Vec::with_capacity(n);
        let mut start = at(0)?;
        for i in 1..=n {
            let end = at(i)?;
            if end < start {
                return Err(l.bad("has decreasing offsets"));
            }
            glyphs.push(g.slice(start, end - start)?);
            start = end;
        }
        Ok(Some(Glyf { glyphs }))
    }

    pub(crate) fn len(&self) -> usize {
        self.glyphs.len()
    }

    /// The bytes of glyph `g`; empty past the end.
    pub(crate) fn glyph(&self, g: usize) -> &[u8] {
        self.glyphs.get(g).copied().unwrap_or_default()
    }

    /// The component glyph ids of glyph `g` (none for a simple or empty glyph).
    pub(crate) fn components(&self, g: usize) -> Result<Vec<u32>> {
        let Some(glyph) = self.glyphs.get(g).copied() else { return Ok(Vec::new()) };
        if !is_composite(glyph)? {
            return Ok(Vec::new());
        }
        let (components, _, _) = walk_composite(glyph)?;
        components.iter().map(|c| Data::new(*b"glyf", glyph).u16(c.gid_at).map(u32::from)).collect()
    }

    /// `glyf` and `loca` of the subset, and whether `loca` uses long offsets.
    fn subset(&self, plan: &Plan) -> Result<(Vec<u8>, Vec<u8>, bool)> {
        let mut glyf = Vec::new();
        let mut offsets = Vec::with_capacity(plan.new_to_old.len() + 1);
        for old in &plan.new_to_old {
            offsets.push(glyf.len());
            let empty = plan.emptied.contains(old) || (*old == 0 && plan.options.notdef_glyph && !plan.options.notdef_outline);
            if empty {
                continue;
            }
            let data = self.glyphs.get(*old as usize).copied().unwrap_or_default();
            let data = data.get(..glyph_len(data)?).unwrap_or(data);
            glyf.extend(glyph_out(data, plan)?);
            while !glyf.len().is_multiple_of(4) {
                glyf.push(0);
            }
        }
        offsets.push(glyf.len());
        let long = glyf.len() >= 0x2_0000;
        let mut loca = Vec::with_capacity(offsets.len() * if long { 4 } else { 2 });
        for at in offsets {
            if long {
                put_u32(&mut loca, to_u32(at, "the glyf table")?);
            } else {
                put_u16(&mut loca, to_u16(at / 2, "the glyf table")?);
            }
        }
        Ok((glyf, loca, long))
    }
}

/// The length of a glyph's data without the padding after it, as fontTools trims it.
fn glyph_len(data: &[u8]) -> Result<usize> {
    if data.is_empty() {
        return Ok(0);
    }
    let d = Data::new(*b"glyf", data);
    let contours = d.i16(0)?;
    if contours < 0 {
        let (_, end, instructions) = walk_composite(data)?;
        return Ok(if instructions { end + 2 + usize::from(d.u16(end)?) } else { end });
    }
    let contours = usize::try_from(contours).unwrap_or(0);
    if contours == 0 {
        return Ok(data.len());
    }
    let points = usize::from(d.u16(10 + 2 * (contours - 1))?) + 1;
    let mut at = 12 + 2 * contours;
    at += usize::from(d.u16(at - 2)?);
    let (mut x, mut y, mut seen) = (0usize, 0usize, 0usize);
    while seen < points {
        let flags = d.u8(at)?;
        at += 1;
        let mut repeat = 1usize;
        if flags & 0x08 != 0 {
            repeat += usize::from(d.u8(at)?);
            at += 1;
        }
        let xs = if flags & 0x02 != 0 {
            1
        } else if flags & 0x10 != 0 {
            0
        } else {
            2
        };
        let ys = if flags & 0x04 != 0 {
            1
        } else if flags & 0x20 != 0 {
            0
        } else {
            2
        };
        x += xs * repeat;
        y += ys * repeat;
        seen += repeat;
    }
    Ok((at + x + y).min(data.len()))
}

/// One glyph of the subset: components renumbered, instructions dropped without hinting.
fn glyph_out(data: &[u8], plan: &Plan) -> Result<Vec<u8>> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let d = Data::new(*b"glyf", data);
    let contours = d.i16(0)?;
    if contours >= 0 {
        if plan.options.hinting {
            return Ok(data.to_vec());
        }
        let ends = 10 + 2 * usize::try_from(contours).unwrap_or(0);
        let len = usize::from(d.u16(ends)?);
        let rest = d.slice(ends + 2 + len, data.len().saturating_sub(ends + 2 + len))?;
        let mut out = d.slice(0, ends)?.to_vec();
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(rest);
        return Ok(out);
    }
    let (components, end, instructions) = walk_composite(data)?;
    let mut out = data.to_vec();
    for c in &components {
        let old = u32::from(Data::new(*b"glyf", &out).u16(c.gid_at)?);
        let new = plan
            .glyph_map
            .get(&old)
            .copied()
            .ok_or_else(|| TablesError::invalid(format!("a composite glyph uses glyph {old}, which is past the glyph count")))?;
        set_u16(&mut out, c.gid_at, to_u16(new as usize, "a glyph id")?)?;
        if !plan.options.hinting && instructions {
            let flags_at = c.gid_at - 2;
            let flags = Data::new(*b"glyf", &out).u16(flags_at)?;
            set_u16(&mut out, flags_at, flags & !0x0100)?;
        }
    }
    if !plan.options.hinting && instructions {
        out.truncate(end);
    }
    Ok(out)
}

/// `hmtx` or `vmtx` of the subset and its count of long metrics. Emptied glyphs get (0, 0);
/// trailing glyphs with the advance of the last one share it.
fn metrics(table: &[u8], tag: Tag, long: usize, plan: &Plan) -> Result<(Vec<u8>, u16)> {
    let d = Data::new(tag, table);
    if long == 0 {
        return Err(d.bad("has no long metrics"));
    }
    let entry = |g: usize| -> Result<(u16, i16)> {
        if g < long {
            Ok((d.u16(4 * g)?, d.i16(4 * g + 2)?))
        } else {
            let advance = d.u16(4 * (long - 1))?;
            let side = d.i16(4 * long + 2 * (g - long)).unwrap_or(0);
            Ok((advance, side))
        }
    };
    let mut rows = Vec::with_capacity(plan.new_to_old.len());
    for old in &plan.new_to_old {
        rows.push(if plan.emptied.contains(old) { (0, 0) } else { entry(*old as usize)? });
    }
    let last = rows.last().map_or(0, |r| r.0);
    let mut count = rows.len();
    while count > 1 && rows.get(count - 2).is_some_and(|r| r.0 == last) {
        count -= 1;
    }
    let mut out = Vec::with_capacity(4 * count + 2 * (rows.len() - count));
    for (i, (advance, side)) in rows.iter().enumerate() {
        if i < count {
            put_u16(&mut out, *advance);
        }
        put_u16(&mut out, *side as u16);
    }
    Ok((out, to_u16(count, "the long metrics count")?))
}

/// `post` of the subset: version 3 unless glyph names are kept.
fn subset_post(plan: &Plan, post: &[u8]) -> Result<Vec<u8>> {
    let d = Data::new(*b"post", post);
    let header = d.slice(4, 28)?;
    if plan.options.glyph_names
        && let Some(all) = names::post_names(post, plan.num_glyphs)?
    {
        let kept: Vec<String> = plan.new_to_old.iter().map(|o| all.get(*o as usize).cloned().unwrap_or_default()).collect();
        return names::write_post_v2(post, &kept);
    }
    let mut out = Vec::with_capacity(32);
    put_u32(&mut out, 0x0003_0000);
    out.extend_from_slice(header);
    Ok(out)
}

/// The default code points and the non-default (code point, glyph) pairs of one variation
/// selector.
type Sequences = (Vec<u32>, Vec<(u32, u32)>);

/// One variation selector of a format 14 subtable with the new glyph ids.
type SelectorRecord = (u32, Vec<u32>, Vec<(u32, u16)>);

/// One `cmap` subtable of the subset before it is encoded.
struct SubCmap {
    platform: u16,
    encoding: u16,
    language: u32,
    format: u16,
    map: BTreeMap<u32, u32>,
    /// Format 14: selector → (default code points, non-default (code point, glyph)).
    uvs: BTreeMap<u32, Sequences>,
}

fn subset_cmap(plan: &Plan, bytes: &[u8]) -> Result<Vec<u8>> {
    let cmap = Cmap::read(FontData::new(bytes)).map_err(|e| TablesError::invalid(format!("the 'cmap' table cannot be read: {e}")))?;
    let d = Data::new(*b"cmap", bytes);
    let keep = |u: u32, g: u32| plan.requested.contains(&g) || plan.unicodes.contains(&u);
    let mut subs: Vec<SubCmap> = Vec::new();
    let mut budget = MAX_CODE_POINTS.saturating_mul(4);
    for record in cmap.encoding_records() {
        let (platform, encoding) = (record.platform_id(), record.encoding_id());
        let unicode = is_unicode(platform, encoding);
        let symbol = platform == PlatformId::Windows && encoding == 0;
        if (!unicode && !symbol && !plan.options.legacy_cmap) || (symbol && !plan.options.symbol_cmap) {
            continue;
        }
        let at = record.subtable_offset().to_u32() as usize;
        let format = d.u16(at)?;
        let language = match format {
            8 | 10 | 12 | 13 => d.u32(at + 8)?,
            14 => 0,
            _ => u32::from(d.u16(at + 4)?),
        };
        let sub = record.subtable(cmap.offset_data()).map_err(|e| TablesError::invalid(format!("a 'cmap' subtable cannot be read: {e}")))?;
        let mut out = SubCmap { platform: platform as u16, encoding, language, format, map: BTreeMap::new(), uvs: BTreeMap::new() };
        match &sub {
            CmapSubtable::Format0(_) => continue,
            CmapSubtable::Format14(t) => {
                for selector in t.var_selector() {
                    let vs = selector.var_selector().to_u32();
                    if !plan.unicodes.contains(&vs) {
                        continue;
                    }
                    let mut defaults = Vec::new();
                    let mut mapped = Vec::new();
                    if let Some(Ok(def)) = selector.default_uvs(t.offset_data()) {
                        for range in def.ranges() {
                            let start = range.start_unicode_value().to_u32();
                            for u in start..=start + u32::from(range.additional_count()) {
                                budget = budget.checked_sub(1).ok_or(TablesError::TooLarge { what: "the cmap mappings", limit: MAX_CODE_POINTS })?;
                                if plan.unicodes.contains(&u) {
                                    defaults.push(u);
                                }
                            }
                        }
                    }
                    if let Some(Ok(non_default)) = selector.non_default_uvs(t.offset_data()) {
                        for m in non_default.uvs_mapping() {
                            budget = budget.checked_sub(1).ok_or(TablesError::TooLarge { what: "the cmap mappings", limit: MAX_CODE_POINTS })?;
                            let (u, g) = (m.unicode_value().to_u32(), u32::from(m.glyph_id()));
                            if keep(u, g) && plan.in_output(g) {
                                mapped.push((u, g));
                            }
                        }
                    }
                    if !defaults.is_empty() || !mapped.is_empty() {
                        out.uvs.insert(vs, (defaults, mapped));
                    }
                }
            }
            _ => {
                for (u, gid) in sub.iter() {
                    budget = budget.checked_sub(1).ok_or(TablesError::TooLarge { what: "the cmap mappings", limit: MAX_CODE_POINTS })?;
                    let g = gid.to_u32();
                    if g == 0 {
                        continue;
                    }
                    let kept = if unicode { keep(u, g) } else { plan.requested.contains(&g) };
                    if kept && plan.in_output(g) {
                        out.map.insert(u, g);
                    }
                }
            }
        }
        subs.push(out);
    }
    // A format 12 subtable with only BMP code points repeats its BMP sibling.
    let siblings: Vec<(u16, u16, u32, BTreeMap<u32, u32>)> = subs
        .iter()
        .filter(|s| (s.platform == 0 && s.encoding == 3) || (s.platform == 3 && s.encoding == 1))
        .map(|s| (s.platform, s.encoding, s.language, s.map.clone()))
        .collect();
    for s in subs.iter_mut().filter(|s| s.format == 12 && s.map.last_key_value().is_some_and(|(u, _)| *u < 0x1_0000)) {
        let bmp_encoding = match (s.platform, s.encoding) {
            (0, 4) => Some(3),
            (3, 10) => Some(1),
            _ => None,
        };
        if let Some(e) = bmp_encoding
            && siblings.iter().any(|(p, enc, lang, map)| *p == s.platform && *enc == e && *lang == s.language && *map == s.map)
        {
            s.map.clear();
        }
    }
    subs.retain(|s| if s.format == 14 { !s.uvs.is_empty() } else { !s.map.is_empty() });
    subs.sort_by_key(|s| (s.platform, s.encoding, s.language));

    let mut encoded: Vec<(u16, u16, Vec<u8>)> = Vec::with_capacity(subs.len());
    for s in &subs {
        let new = |g: u32| -> Result<u16> { to_u16(plan.glyph_map.get(&g).copied().unwrap_or(0) as usize, "a glyph id") };
        let bytes = if s.format == 14 {
            let mut uvs = Vec::with_capacity(s.uvs.len());
            for (vs, (defaults, mapped)) in &s.uvs {
                let mapped: Vec<(u32, u16)> = mapped.iter().map(|(u, g)| Ok((*u, new(*g)?))).collect::<Result<_>>()?;
                uvs.push((*vs, defaults.clone(), mapped));
            }
            write_format14(&uvs)?
        } else {
            let map: Vec<(u32, u16)> = s.map.iter().map(|(u, g)| Ok((*u, new(*g)?))).collect::<Result<_>>()?;
            let bmp = s.map.last_key_value().is_none_or(|(u, _)| *u <= 0xFFFF);
            match s.format {
                4 | 6 | 2 if bmp => match s.format {
                    6 => write_format6(s.language as u16, &map)?,
                    _ => write_format4(s.language as u16, &map)?,
                },
                13 => write_format13(s.language, &map)?,
                _ => write_format12(s.language, &map)?,
            }
        };
        encoded.push((s.platform, s.encoding, bytes));
    }
    let header_len = 4 + 8 * encoded.len();
    let mut out = Vec::new();
    put_u16(&mut out, 0);
    put_u16(&mut out, to_u16(encoded.len(), "the cmap subtable count")?);
    let mut body: Vec<u8> = Vec::new();
    let mut placed: HashMap<&[u8], usize> = HashMap::new();
    for (platform, encoding, bytes) in &encoded {
        let at = match placed.get(bytes.as_slice()) {
            Some(at) => *at,
            None => {
                let at = header_len + body.len();
                body.extend_from_slice(bytes);
                while !body.len().is_multiple_of(4) {
                    body.push(0);
                }
                placed.insert(bytes.as_slice(), at);
                at
            }
        };
        put_u16(&mut out, *platform);
        put_u16(&mut out, *encoding);
        put_u32(&mut out, to_u32(at, "the cmap table")?);
    }
    out.extend(body);
    Ok(out)
}

/// A format 6 subtable covering the lowest to the highest code point of `map`.
fn write_format6(language: u16, map: &[(u32, u16)]) -> Result<Vec<u8>> {
    let first = map.first().map_or(0, |m| m.0);
    let last = map.last().map_or(0, |m| m.0);
    let count = (last - first + 1) as usize;
    let mut glyphs = vec![0u16; count];
    for (u, g) in map {
        if let Some(slot) = glyphs.get_mut((u - first) as usize) {
            *slot = *g;
        }
    }
    let mut out = Vec::with_capacity(10 + 2 * count);
    put_u16(&mut out, 6);
    put_u16(&mut out, to_u16(10 + 2 * count, "a format 6 cmap subtable")?);
    put_u16(&mut out, language);
    put_u16(&mut out, to_u16(first as usize, "a code point")?);
    put_u16(&mut out, to_u16(count, "a format 6 cmap subtable")?);
    for g in glyphs {
        put_u16(&mut out, g);
    }
    Ok(out)
}

/// A format 13 subtable: one group per run of consecutive code points on the same glyph.
fn write_format13(language: u32, map: &[(u32, u16)]) -> Result<Vec<u8>> {
    let mut groups: Vec<(u32, u32, u16)> = Vec::new();
    for (u, g) in map {
        match groups.last_mut() {
            Some((_, end, glyph)) if *end + 1 == *u && *glyph == *g => *end = *u,
            _ => groups.push((*u, *u, *g)),
        }
    }
    let mut out = Vec::with_capacity(16 + 12 * groups.len());
    put_u16(&mut out, 13);
    put_u16(&mut out, 0);
    put_u32(&mut out, to_u32(16 + 12 * groups.len(), "a format 13 cmap subtable")?);
    put_u32(&mut out, language);
    put_u32(&mut out, to_u32(groups.len(), "a format 13 cmap subtable")?);
    for (start, end, g) in groups {
        put_u32(&mut out, start);
        put_u32(&mut out, end);
        put_u32(&mut out, u32::from(g));
    }
    Ok(out)
}

/// A format 14 subtable from (selector, default code points, non-default mappings).
fn write_format14(records: &[SelectorRecord]) -> Result<Vec<u8>> {
    let header = 10 + 11 * records.len();
    let mut body: Vec<u8> = Vec::new();
    let mut offsets = Vec::with_capacity(records.len());
    for (_, defaults, mapped) in records {
        let mut def_at = 0;
        if !defaults.is_empty() {
            let mut ranges: Vec<(u32, u8)> = Vec::new();
            for u in defaults {
                match ranges.last_mut() {
                    Some((start, extra)) if *extra < u8::MAX && *start + u32::from(*extra) + 1 == *u => *extra += 1,
                    _ => ranges.push((*u, 0)),
                }
            }
            def_at = header + body.len();
            put_u32(&mut body, to_u32(ranges.len(), "a format 14 cmap subtable")?);
            for (start, extra) in ranges {
                body.extend_from_slice(start.to_be_bytes().get(1..).unwrap_or_default());
                body.push(extra);
            }
        }
        let mut non_at = 0;
        if !mapped.is_empty() {
            non_at = header + body.len();
            put_u32(&mut body, to_u32(mapped.len(), "a format 14 cmap subtable")?);
            for (u, g) in mapped {
                body.extend_from_slice(u.to_be_bytes().get(1..).unwrap_or_default());
                put_u16(&mut body, *g);
            }
        }
        offsets.push((def_at, non_at));
    }
    let mut out = Vec::with_capacity(header + body.len());
    put_u16(&mut out, 14);
    put_u32(&mut out, to_u32(header + body.len(), "a format 14 cmap subtable")?);
    put_u32(&mut out, to_u32(records.len(), "a format 14 cmap subtable")?);
    for ((vs, _, _), (def_at, non_at)) in records.iter().zip(offsets) {
        out.extend_from_slice(vs.to_be_bytes().get(1..).unwrap_or_default());
        put_u32(&mut out, to_u32(def_at, "a format 14 cmap subtable")?);
        put_u32(&mut out, to_u32(non_at, "a format 14 cmap subtable")?);
    }
    out.extend(body);
    Ok(out)
}

/// The code points of the Unicode subtables of a `cmap`.
fn unicode_code_points(bytes: &[u8]) -> Result<BTreeSet<u32>> {
    let cmap = Cmap::read(FontData::new(bytes)).map_err(|e| TablesError::invalid(format!("the 'cmap' table cannot be read: {e}")))?;
    let mut out = BTreeSet::new();
    for record in cmap.encoding_records() {
        if !is_unicode(record.platform_id(), record.encoding_id()) {
            continue;
        }
        if let Ok(sub) = record.subtable(cmap.offset_data()) {
            out.extend(sub.iter().take(MAX_CODE_POINTS).filter(|(_, g)| g.to_u32() != 0).map(|(u, _)| u));
        }
    }
    Ok(out)
}

/// `name` with the records the options keep (the requested IDs and the IDs in `referenced`,
/// which other tables use), sorted, strings stored once each.
pub(crate) fn subset_name(plan: &Plan, bytes: &[u8], referenced: &BTreeSet<u16>) -> Result<Vec<u8>> {
    let d = Data::new(*b"name", bytes);
    let format = d.u16(0)?;
    let count = usize::from(d.u16(2)?);
    let storage = usize::from(d.u16(4)?);
    let o = &plan.options;
    let ids: BTreeSet<u16> = o.name_ids.clone().unwrap_or_else(|| super::DEFAULT_NAME_IDS.to_vec()).into_iter().collect();
    let languages: Option<BTreeSet<u16>> = if o.all_name_languages {
        None
    } else {
        Some(o.name_languages.clone().unwrap_or_else(|| super::DEFAULT_NAME_LANGUAGES.to_vec()).into_iter().collect())
    };
    let mut records: Vec<([u16; 4], &[u8])> = Vec::new();
    for i in 0..count {
        let at = 6 + 12 * i;
        let r = [d.u16(at)?, d.u16(at + 2)?, d.u16(at + 4)?, d.u16(at + 6)?];
        let [platform, encoding, language, id] = r;
        let text = d.slice(storage + usize::from(d.u16(at + 10)?), usize::from(d.u16(at + 8)?))?;
        let id_kept = ids.contains(&id) || referenced.contains(&id) || (o.all_name_ids && id < 256);
        let unicode = platform == 0 || (platform == 3 && matches!(encoding, 0 | 1 | 10));
        let language_kept = languages.as_ref().is_none_or(|l| l.contains(&language));
        if id_kept && (unicode || o.name_legacy) && language_kept {
            records.push((r, text));
        }
    }
    records.sort_by_key(|(r, _)| *r);
    let lang_tags: Vec<&[u8]> = if format == 1 {
        let tags_at = 6 + 12 * count;
        let n = usize::from(d.u16(tags_at)?);
        let mut tags = Vec::with_capacity(n);
        for i in 0..n {
            let at = tags_at + 2 + 4 * i;
            tags.push(d.slice(storage + usize::from(d.u16(at + 2)?), usize::from(d.u16(at)?))?);
        }
        tags
    } else {
        Vec::new()
    };
    let mut strings: Vec<u8> = Vec::new();
    let mut placed: HashMap<&[u8], usize> = HashMap::new();
    let place = |text: &[u8], strings: &mut Vec<u8>| -> usize {
        let at = strings.len();
        strings.extend_from_slice(text);
        at
    };
    let header = 6 + 12 * records.len() + if format == 1 { 2 + 4 * lang_tags.len() } else { 0 };
    let mut out = Vec::new();
    put_u16(&mut out, format);
    put_u16(&mut out, to_u16(records.len(), "the name record count")?);
    put_u16(&mut out, to_u16(header, "the name table")?);
    for (r, text) in &records {
        for v in r {
            put_u16(&mut out, *v);
        }
        let at = match placed.get(text) {
            Some(at) => *at,
            None => {
                let at = place(text, &mut strings);
                placed.insert(text, at);
                at
            }
        };
        put_u16(&mut out, to_u16(text.len(), "a name string")?);
        put_u16(&mut out, to_u16(at, "the name strings")?);
    }
    if format == 1 {
        put_u16(&mut out, to_u16(lang_tags.len(), "the language tag count")?);
        for tag in &lang_tags {
            let at = place(tag, &mut strings);
            put_u16(&mut out, to_u16(tag.len(), "a language tag")?);
            put_u16(&mut out, to_u16(at, "the name strings")?);
        }
    }
    out.extend(strings);
    Ok(out)
}

/// `OS/2` with the range bits pruned to `code_points` and the first and last character
/// indices recomputed, as pyftsubset leaves it.
fn subset_os2(plan: &Plan, bytes: &[u8], code_points: &BTreeSet<u32>) -> Result<Vec<u8>> {
    let mut out = bytes.to_vec();
    let d = Data::new(*b"OS/2", bytes);
    let version = d.u16(0)?;
    if plan.options.prune_unicode_ranges && bytes.len() >= 58 {
        let used = fontcraft_compile::ranges::unicode_range_bits(code_points);
        for word in 0..4usize {
            let old = d.u32(42 + 4 * word)?;
            let mut new = 0u32;
            for bit in 0..32u32 {
                let index = word as u32 * 32 + bit;
                if old & (1 << bit) != 0 && (index >= 123 || used.contains(&index)) {
                    new |= 1 << bit;
                }
            }
            put_at(&mut out, 42 + 4 * word, new)?;
        }
    }
    if plan.options.prune_codepage_ranges && version >= 1 && bytes.len() >= 86 {
        let used = fontcraft_compile::ranges::code_page_bits(code_points);
        let old = [d.u32(78)?, d.u32(82)?];
        let mut new = [0u32; 2];
        for (word, slot) in new.iter_mut().enumerate() {
            for bit in 0..32u32 {
                if old.get(word).is_some_and(|o| o & (1 << bit) != 0) && used.contains(&(word as u32 * 32 + bit)) {
                    *slot |= 1 << bit;
                }
            }
        }
        if new == [0, 0] {
            new = [1, 0];
        }
        put_at(&mut out, 78, new[0])?;
        put_at(&mut out, 82, new[1])?;
    }
    if let (Some(first), Some(last)) = (code_points.first(), code_points.last()) {
        set_u16(&mut out, 64, (*first).min(0xFFFF) as u16)?;
        set_u16(&mut out, 66, (*last).min(0xFFFF) as u16)?;
    }
    Ok(out)
}

fn put_at(out: &mut [u8], at: usize, v: u32) -> Result<()> {
    let slot = out.get_mut(at..at + 4).ok_or_else(|| TablesError::invalid("the 'OS/2' table is cut short"))?;
    slot.copy_from_slice(&v.to_be_bytes());
    Ok(())
}

/// `CFF ` of the subset: charstrings, charset and FDSelect follow the glyph set, `.notdef`
/// keeps only its width unless its outline is kept, emptied glyphs become `endchar`, a
/// custom Encoding gives way to the Standard Encoding; then [`super::cff2::finish_cff`] drops
/// the unused Font DICTs and subroutines, or the hints without hinting.
fn subset_cff(plan: &Plan, bytes: &[u8]) -> Result<Vec<u8>> {
    let table = cff::parse(bytes, plan.num_glyphs)?;
    let new_to_old: Vec<u16> = plan.new_to_old.iter().map(|o| to_u16(*o as usize, "a glyph id")).collect::<Result<_>>()?;
    let mut replace: HashMap<usize, Vec<u8>> = HashMap::new();
    for (new, old) in plan.new_to_old.iter().enumerate() {
        if plan.emptied.contains(old) {
            replace.insert(new, vec![14]);
        }
    }
    if plan.options.notdef_glyph && !plan.options.notdef_outline {
        let program = table.charstring(0).unwrap_or_default();
        let (local, global) = table.subrs(0);
        let width = cs::scan(program, local, global, &mut plan.charstring_ops_left.clone())?.width;
        let (default, nominal) = table.widths(0);
        let mut empty = Vec::new();
        if let Some(w) = width {
            let same = matches!((default, nominal), (Some(d), Some(n)) if n as f64 + w == d as f64);
            if !same {
                empty.extend(cs::encode_number(w));
            }
        }
        empty.push(14);
        replace.insert(0, empty);
    }
    let rebuilt = table.rebuild_with(&new_to_old, None, &replace, true)?;
    super::cff2::finish_cff(plan, rebuilt)
}
