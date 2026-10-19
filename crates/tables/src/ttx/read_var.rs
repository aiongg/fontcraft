//! TTX reader: variation tables (M12.9).
//!
//! HVAR, VVAR and MVAR go through the generic layout compile of [`super::read_otdata`], as in
//! fontTools, where they are `otData` tables too. `gvar` and `cvar` compile their tuple
//! variation stores as fontTools does: shared peak tuples, the most useful point set shared
//! within a glyph, and deltas packed in runs with its size optimisation. Ported from
//! fontTools' `ttLib.tables` (MIT): `otData`, `_g_v_a_r`, `_c_v_a_r` and `TupleVariation`.

use std::collections::{BTreeMap, HashMap};

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

use super::Element;
use super::read::{Out, ReadContext, float_lit, int_lit, ot_round};

/// The table a TTX table element stands for, as bytes; `None` for an element this module does
/// not handle. `font` gives the context some tables need (glyph order, `maxp`, `head`).
pub fn read_table(element: &Element, font: &SfntFont) -> Result<Option<(Tag, Vec<u8>)>> {
    read_table_with(element, &ReadContext::from_font(font), font)
}

/// [`read_table`] with the glyph order of the text being read; `font` holds the tables read so
/// far (`fvar` among the first).
pub(crate) fn read_table_with(element: &Element, ctx: &ReadContext, font: &SfntFont) -> Result<Option<(Tag, Vec<u8>)>> {
    let tag = match element.name.as_str() {
        "gvar" => return Ok(Some((*b"gvar", gvar(element, ctx, font, None)?))),
        "cvar" => return Ok(Some((*b"cvar", cvar(element, font)?))),
        "HVAR" => *b"HVAR",
        "VVAR" => *b"VVAR",
        "MVAR" => *b"MVAR",
        _ => return Ok(None),
    };
    let data = super::read_otdata::build::compile_layout(&element.name, element, ctx, font)?;
    Ok(Some((tag, data)))
}

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what.into())
}

fn count(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// The delta sets one tuple variation store can hold: its count field has 12 bits.
const MAX_STORE_TUPLES: usize = 0x0FFF;

/// One delta set: a region and the deltas of the points that have one; the other points are
/// left to inference.
struct Tuple {
    /// Axis tag to (start, peak, end), in 2.14 units.
    axes: Vec<(String, (i64, i64, i64))>,
    /// The points (or control values) the set covers.
    points: usize,
    /// Point index to its delta. Sparse, so that the memory follows the text rather than the
    /// point count the font claims.
    deltas: BTreeMap<usize, (i64, i64)>,
}

impl Tuple {
    /// `TupleVariation.fromXML` over the children of a `tuple` element.
    fn read(e: &Element, points: usize, numeric_axes: bool) -> Result<Tuple> {
        let mut t = Tuple { axes: Vec::new(), points, deltas: BTreeMap::new() };
        for c in &e.children {
            match c.name.as_str() {
                "coord" => {
                    let mut axis = c.attribute("axis").ok_or_else(|| bad("a coordinate without an axis"))?.to_string();
                    if numeric_axes {
                        axis = int_lit(&axis)?.to_string();
                    }
                    let f2 = |text: &str| -> Result<i64> { ot_round(float_lit(text)? * 16384.0) };
                    let peak = f2(c.attribute("value").ok_or_else(|| bad("a coordinate without a value"))?)?;
                    let start = match c.attribute("min") {
                        Some(v) => f2(v)?,
                        None => peak.min(0),
                    };
                    let end = match c.attribute("max") {
                        Some(v) => f2(v)?,
                        None => peak.max(0),
                    };
                    t.axes.retain(|(a, _)| *a != axis);
                    if t.axes.len() >= crate::limits::MAX_AXES {
                        return Err(TablesError::TooLarge { what: "the axes of a delta set", limit: crate::limits::MAX_AXES });
                    }
                    t.axes.push((axis, (start, peak, end)));
                }
                "delta" => {
                    let (index, value) = if let Some(pt) = c.attribute("pt") {
                        let x = int_lit(c.attribute("x").ok_or_else(|| bad("a delta without 'x'"))?)?;
                        let y = int_lit(c.attribute("y").ok_or_else(|| bad("a delta without 'y'"))?)?;
                        (int_lit(pt)?, (x, y))
                    } else if let Some(cvt) = c.attribute("cvt") {
                        (int_lit(cvt)?, (int_lit(c.attribute("value").ok_or_else(|| bad("a delta without a value"))?)?, 0))
                    } else {
                        continue;
                    };
                    let slot = usize::try_from(index)
                        .ok()
                        .filter(|i| *i < points)
                        .ok_or_else(|| bad(format!("a delta for point {index}, past the glyph's points")))?;
                    t.deltas.insert(slot, value);
                }
                _ => {}
            }
        }
        Ok(t)
    }

    /// The points with deltas: `Some(empty)` for all of them, `None` for none.
    fn used(&self) -> Option<Vec<usize>> {
        if self.deltas.len() == self.points {
            return Some(Vec::new());
        }
        let used: Vec<usize> = self.deltas.keys().copied().collect();
        (!used.is_empty()).then_some(used)
    }

    /// Whether [`store`] keeps the set ([`Tuple::used`] is not `None`).
    fn kept(&self) -> bool {
        !self.deltas.is_empty() || self.points == 0
    }

    fn coord(&self, tags: &[String]) -> Result<Vec<u8>> {
        let mut o = Out::default();
        for tag in tags {
            let peak = self.axes.iter().find(|(a, _)| a == tag).map_or(0, |(_, t)| t.1);
            o.i16(peak, "a peak coordinate")?;
        }
        Ok(o.0)
    }

    fn intermediate(&self, tags: &[String]) -> Result<Option<Vec<u8>>> {
        let triple = |tag: &String| self.axes.iter().find(|(a, _)| a == tag).map_or((0, 0, 0), |(_, t)| *t);
        if !tags.iter().map(triple).any(|(lo, peak, hi)| lo != peak.min(0) || hi != peak.max(0)) {
            return Ok(None);
        }
        let mut o = Out::default();
        for tag in tags {
            o.i16(triple(tag).0, "a start coordinate")?;
        }
        for tag in tags {
            o.i16(triple(tag).2, "an end coordinate")?;
        }
        Ok(Some(o.0))
    }

    /// `TupleVariation.compile`: the tuple header and its serialised data.
    fn compile(&self, tags: &[String], shared: &HashMap<Vec<u8>, usize>, points: &[u8], two: bool) -> Result<(Vec<u8>, Vec<u8>)> {
        if let Some((axis, _)) = self.axes.iter().find(|(a, _)| !tags.contains(a)) {
            return Err(bad(format!("a delta set on the axis '{axis}', which the font does not have")));
        }
        let coord = self.coord(tags)?;
        let mut tuple = Vec::new();
        let mut flags = match shared.get(&coord) {
            Some(i) => i64::try_from(*i).unwrap_or(0),
            None => {
                tuple.extend_from_slice(&coord);
                0x8000
            }
        };
        if let Some(inter) = self.intermediate(tags)? {
            flags |= 0x4000;
            tuple.extend_from_slice(&inter);
        }
        let mut aux = Vec::new();
        if !points.is_empty() {
            flags |= 0x2000;
            aux.extend_from_slice(points);
        }
        let xs: Vec<i64> = self.deltas.values().map(|d| d.0).collect();
        deltas(&xs, &mut aux)?;
        if two {
            let ys: Vec<i64> = self.deltas.values().map(|d| d.1).collect();
            deltas(&ys, &mut aux)?;
        }
        let mut head = Out::default();
        head.u16(count(aux.len()), "a variation data size")?;
        head.u16(flags, "tuple flags")?;
        head.bytes(&tuple);
        Ok((head.0, aux))
    }
}

/// `TupleVariation.compilePoints`.
fn compile_points(points: &[usize]) -> Result<Vec<u8>> {
    if points.is_empty() {
        return Ok(vec![0]);
    }
    let mut points = points.to_vec();
    points.sort_unstable();
    let n = points.len();
    let mut out = Vec::new();
    if n < 0x80 {
        out.push(n as u8);
    } else if n <= 0x7FFF {
        out.push(((n >> 8) | 0x80) as u8);
        out.push((n & 0xFF) as u8);
    } else {
        return Err(bad("more than 32,767 points with deltas"));
    }
    let (mut pos, mut last) = (0usize, 0usize);
    while pos < n {
        let header = out.len();
        out.push(0);
        let mut run = 0usize;
        let mut bytes: Option<bool> = None;
        while pos < n && run <= 127 {
            let current = points.get(pos).copied().unwrap_or(0);
            let delta = current.wrapping_sub(last);
            let small = current >= last && delta <= 0xFF;
            let use_bytes = *bytes.get_or_insert(small);
            if use_bytes && !small {
                break;
            }
            if use_bytes {
                out.push(delta as u8);
            } else {
                let d = u16::try_from(delta).map_err(|_| bad("a point number past 65535"))?;
                out.extend_from_slice(&d.to_be_bytes());
            }
            last = current;
            pos += 1;
            run += 1;
        }
        let flag = if bytes == Some(true) { 0 } else { 0x80 };
        if let Some(h) = out.get_mut(header) {
            *h = (run.saturating_sub(1) as u8) | flag;
        }
    }
    Ok(out)
}

/// `TupleVariation.compileDeltaValues_` with size optimisation.
fn deltas(values: &[i64], out: &mut Vec<u8>) -> Result<()> {
    let n = values.len();
    let at = |i: usize| values.get(i).copied().unwrap_or(0);
    let byte = |v: i64| (-128..=127).contains(&v);
    let word = |v: i64| (-32768..=32767).contains(&v);
    let mut pos = 0usize;
    while pos < n {
        let value = at(pos);
        let start = pos;
        if value == 0 {
            while pos < n && at(pos) == 0 {
                pos += 1;
            }
            let mut run = pos - start;
            while run >= 64 {
                out.push(0x80 | 63);
                run -= 64;
            }
            if run > 0 {
                out.push(0x80 | (run - 1) as u8);
            }
            continue;
        }
        let (flag, size): (u8, usize) = if byte(value) {
            while pos < n && byte(at(pos)) && !(at(pos) == 0 && pos + 1 < n && at(pos + 1) == 0) {
                pos += 1;
            }
            (0, 1)
        } else if word(value) {
            while pos < n && at(pos) != 0 && !(byte(at(pos)) && pos + 1 < n && byte(at(pos + 1))) && word(at(pos)) {
                pos += 1;
            }
            (0x40, 2)
        } else {
            while pos < n && !word(at(pos)) {
                pos += 1;
            }
            (0xC0, 4)
        };
        let mut offset = start;
        while offset < pos {
            let run = (pos - offset).min(64);
            out.push(flag | (run - 1) as u8);
            for v in values.get(offset..offset + run).unwrap_or_default() {
                let mut o = Out::default();
                match size {
                    1 => o.i8(*v, "a delta")?,
                    2 => o.i16(*v, "a delta")?,
                    _ => o.i32(*v, "a delta")?,
                }
                out.extend_from_slice(&o.0);
            }
            offset += run;
        }
    }
    Ok(())
}

/// A point set of one glyph's tuples: its points, how many tuples use it, its packed bytes.
type PointSet = (Vec<usize>, usize, Vec<u8>);

/// `compileTupleVariationStore`: the tuple count (with the shared-points flag), the tuple
/// headers and the serialised data.
fn store(tuples: &[&Tuple], tags: &[String], shared: &HashMap<Vec<u8>, usize>, share_points: bool, two: bool) -> Result<(i64, Vec<u8>, Vec<u8>)> {
    let mut kept: Vec<(&Tuple, Vec<usize>)> = Vec::new();
    // Point sets in the order first met, with how often each occurs.
    let mut sets: Vec<PointSet> = Vec::new();
    let mut set_index: HashMap<Vec<usize>, usize> = HashMap::new();
    for t in tuples {
        let Some(points) = t.used() else { continue };
        match set_index.get(&points).and_then(|&i| sets.get_mut(i)) {
            Some(s) => s.1 += 1,
            None => {
                let compiled = compile_points(&points)?;
                set_index.insert(points.clone(), sets.len());
                sets.push((points.clone(), 1, compiled));
            }
        }
        kept.push((t, points));
    }
    let Some(first) = kept.first() else { return Ok((0, Vec::new(), Vec::new())) };
    let n = first.0.points;
    if kept.iter().any(|(t, _)| t.points != n) {
        return Err(bad("delta sets of different sizes"));
    }
    if kept.len() > MAX_STORE_TUPLES {
        return Err(TablesError::TooLarge { what: "the delta sets of one glyph", limit: MAX_STORE_TUPLES });
    }
    let mut total = count(kept.len());
    let mut data = Vec::new();
    let mut shared_points: Option<&Vec<usize>> = None;
    if share_points {
        // `max` keeps the first of equal keys.
        let mut best: Option<(&PointSet, usize)> = None;
        for s in &sets {
            let key = s.2.len() * s.1.saturating_sub(1);
            if best.is_none_or(|(_, k)| key > k) {
                best = Some((s, key));
            }
        }
        if let Some((s, _)) = best {
            data.extend_from_slice(&s.2);
            shared_points = Some(&s.0);
        }
        total |= 0x8000;
    }
    let mut headers = Vec::new();
    for (t, points) in &kept {
        let compiled: &[u8] =
            if Some(points) == shared_points { &[] } else { set_index.get(points).and_then(|&i| sets.get(i)).map_or(&[], |s| s.2.as_slice()) };
        let (h, d) = t.compile(tags, shared, compiled, two)?;
        headers.extend_from_slice(&h);
        data.extend_from_slice(&d);
    }
    Ok((total, headers, data))
}

/// The points of each glyph of a `glyf` table plus the four phantom points, as fontTools'
/// `gvar` counts them.
fn point_counts(glyf: &[u8], locations: &[u32]) -> Result<Vec<usize>> {
    let cut = || bad("the 'glyf' table is cut short");
    let u16_at = |d: &[u8], at: usize| {
        d.get(at..at + 2).map(|b| usize::from(u16::from_be_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)])))
    };
    let mut out = Vec::with_capacity(locations.len().saturating_sub(1));
    for w in locations.windows(2) {
        let [start, end] = w else { continue };
        let g = glyf.get(*start as usize..*end as usize).ok_or_else(cut)?;
        if g.is_empty() {
            out.push(4);
            continue;
        }
        let contours = u16_at(g, 0).ok_or_else(cut)? as u16 as i16;
        if contours >= 0 {
            let n = usize::try_from(contours).unwrap_or(0);
            let points = if n == 0 { 0 } else { u16_at(g, 10 + 2 * (n - 1)).ok_or_else(cut)? + 1 };
            if !has_flags(g, n, points) {
                return Err(cut());
            }
            out.push(points + 4);
            continue;
        }
        let (mut at, mut components) = (10usize, 0usize);
        loop {
            let flags = u16_at(g, at).ok_or_else(cut)?;
            at += 4 + if flags & 1 != 0 { 4 } else { 2 };
            at += if flags & 0x08 != 0 {
                2
            } else if flags & 0x40 != 0 {
                4
            } else if flags & 0x80 != 0 {
                8
            } else {
                0
            };
            components += 1;
            if flags & 0x20 == 0 || at >= g.len() {
                break;
            }
        }
        out.push(components + 4);
    }
    Ok(out)
}

/// Whether a simple glyph of `contours` contours holds a flag for each of the `points` its
/// last end point claims, so a short glyph cannot claim more points than its data.
fn has_flags(g: &[u8], contours: usize, points: usize) -> bool {
    let at = 10 + 2 * contours;
    let Some(instructions) =
        g.get(at..at + 2).map(|b| usize::from(u16::from_be_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)])))
    else {
        return false;
    };
    let mut flags = g.get(at + 2 + instructions..).unwrap_or_default().iter();
    let mut seen = 0usize;
    while seen < points {
        let Some(flag) = flags.next() else { return false };
        seen += 1;
        if flag & 0x08 != 0 {
            let Some(repeat) = flags.next() else { return false };
            seen += usize::from(*repeat);
        }
    }
    true
}

/// The axis tags variation data refers to: the `fvar` axes, else `count` numbered axes.
fn axis_tags(font: &SfntFont, count: Option<i64>) -> Result<(Vec<String>, bool)> {
    match (font.get(*b"fvar"), count) {
        (Some(fvar), _) => Ok((super::read_otdata::fvar_axis_tags(fvar)?, false)),
        (None, Some(n)) if (0..=i64::try_from(crate::limits::MAX_AXES).unwrap_or(64)).contains(&n) => {
            Ok(((0..n).map(|i| i.to_string()).collect(), true))
        }
        _ => Err(bad("variation data needs an 'fvar' table")),
    }
}

/// `gvar` (`glyf` gives `(data, locations)` when the text being read holds a `glyf` table).
pub(crate) fn gvar(element: &Element, ctx: &ReadContext, font: &SfntFont, glyf: Option<(&[u8], &[u32])>) -> Result<Vec<u8>> {
    let order = ctx.order()?;
    let points = match glyf {
        Some((data, locations)) => point_counts(data, locations)?,
        None => {
            let data = font.get(*b"glyf").ok_or_else(|| bad("the 'gvar' table needs a 'glyf' table"))?;
            let loca = font.get(*b"loca").ok_or_else(|| bad("the 'gvar' table needs a 'loca' table"))?;
            let long = font.get(*b"head").and_then(|h| h.get(51)).is_some_and(|b| *b != 0);
            let locations: Vec<u32> = if long {
                loca.as_chunks::<4>().0.iter().map(|c| u32::from_be_bytes(*c)).collect()
            } else {
                loca.as_chunks::<2>().0.iter().map(|c| u32::from(u16::from_be_bytes(*c)) * 2).collect()
            };
            point_counts(data, &locations)?
        }
    };
    let (mut version, mut reserved, mut axis_count) = (1i64, 0i64, None);
    for c in &element.children {
        let v = || c.attribute("value").map(int_lit).transpose();
        match c.name.as_str() {
            "version" => version = v()?.unwrap_or(1),
            "reserved" => reserved = v()?.unwrap_or(0),
            "axisCount" => axis_count = v()?,
            _ => {}
        }
    }
    let (tags, numeric) = axis_tags(font, axis_count)?;
    // Glyph name to its delta sets, in the order the glyphs first appear.
    let mut by_glyph: Vec<(String, Vec<Tuple>)> = Vec::new();
    let mut glyph_index: HashMap<String, usize> = HashMap::new();
    for c in element.children.iter().filter(|c| c.name == "glyphVariations") {
        let glyph = c.attribute("glyph").ok_or_else(|| bad("glyph variations without a glyph"))?;
        let gid = usize::try_from(ctx.glyph_id(glyph)?).unwrap_or(usize::MAX);
        let n = *points.get(gid).ok_or_else(|| bad(format!("the glyph '{glyph}' is not in the 'glyf' table")))?;
        let mut tuples = Vec::new();
        let mut kept = 0usize;
        for t in c.children.iter().filter(|t| t.name == "tuple") {
            let tuple = Tuple::read(t, n, numeric)?;
            kept += usize::from(tuple.kept());
            if kept > MAX_STORE_TUPLES {
                return Err(TablesError::TooLarge { what: "the delta sets of one glyph", limit: MAX_STORE_TUPLES });
            }
            tuples.push(tuple);
        }
        match glyph_index.get(glyph).and_then(|&i| by_glyph.get_mut(i)) {
            Some(slot) => slot.1 = tuples,
            None => {
                glyph_index.insert(glyph.to_string(), by_glyph.len());
                by_glyph.push((glyph.to_string(), tuples));
            }
        }
    }
    // `compileSharedTuples`: peaks used more than once, most used first, then by their bytes.
    let mut uses: HashMap<Vec<u8>, usize> = HashMap::new();
    for t in by_glyph.iter().flat_map(|(_, ts)| ts) {
        *uses.entry(t.coord(&tags)?).or_insert(0) += 1;
    }
    let mut uses: Vec<(Vec<u8>, usize)> = uses.into_iter().collect();
    uses.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let shared: Vec<Vec<u8>> = uses.into_iter().take(0x1000).filter(|(_, n)| *n > 1).map(|(c, _)| c).collect();
    let indices: HashMap<Vec<u8>, usize> = shared.iter().enumerate().map(|(i, c)| (c.clone(), i)).collect();
    let mut glyphs: Vec<Vec<u8>> = Vec::with_capacity(order.len());
    for name in order {
        let tuples: Vec<&Tuple> = glyph_index.get(name).and_then(|&i| by_glyph.get(i)).map(|(_, ts)| ts.iter().collect()).unwrap_or_default();
        if tuples.is_empty() {
            glyphs.push(Vec::new());
            continue;
        }
        let (n, headers, data) = store(&tuples, &tags, &indices, true, true)?;
        if n == 0 {
            glyphs.push(Vec::new());
            continue;
        }
        let mut o = Out::default();
        o.u16(n, "the tuple variation count")?;
        let to_data = 4 + count(headers.len());
        o.u16(to_data, "the offset to the variation data")?;
        o.bytes(&headers);
        o.bytes(&data);
        if (to_data + count(data.len())) % 2 != 0 {
            o.bytes(&[0]);
        }
        glyphs.push(o.0);
    }
    let mut offsets = vec![0usize];
    for g in &glyphs {
        offsets.push(offsets.last().copied().unwrap_or(0).saturating_add(g.len()));
    }
    let long = offsets.last().copied().unwrap_or(0) > 0xFFFF * 2;
    let mut packed = Out::default();
    for off in &offsets {
        if long { packed.u32(count(*off), "a glyph offset")? } else { packed.u16(count(*off >> 1), "a glyph offset")? }
    }
    let shared_size: usize = shared.iter().map(Vec::len).sum();
    let to_shared = 20 + count(packed.0.len());
    let mut o = Out::default();
    o.u16(version, "gvar.version")?;
    o.u16(reserved, "gvar.reserved")?;
    o.u16(count(tags.len()), "gvar.axisCount")?;
    o.u16(count(shared.len()), "gvar.sharedTupleCount")?;
    o.u32(to_shared, "gvar.offsetToSharedTuples")?;
    o.u16(count(glyphs.len()), "gvar.glyphCount")?;
    o.u16(i64::from(long), "gvar.flags")?;
    o.u32(to_shared + count(shared_size), "gvar.offsetToGlyphVariationData")?;
    o.bytes(&packed.0);
    for c in &shared {
        o.bytes(c);
    }
    for g in &glyphs {
        o.bytes(g);
    }
    Ok(o.0)
}

/// `cvar`: every delta set with private point numbers and its own peak.
fn cvar(element: &Element, font: &SfntFont) -> Result<Vec<u8>> {
    let values = font.get(*b"cvt ").ok_or_else(|| bad("the 'cvar' table needs a 'cvt ' table"))?.len() / 2;
    let (tags, _) = axis_tags(font, None)?;
    let (mut major, mut minor) = (1i64, 0i64);
    let mut tuples = Vec::new();
    for c in &element.children {
        match c.name.as_str() {
            "version" => {
                major = int_lit(c.attribute("major").unwrap_or("1"))?;
                minor = int_lit(c.attribute("minor").unwrap_or("0"))?;
            }
            "tuple" => {
                let tuple = Tuple::read(c, values, false)?;
                if !tuple.deltas.is_empty() {
                    if tuples.len() >= MAX_STORE_TUPLES {
                        return Err(TablesError::TooLarge { what: "the delta sets of one glyph", limit: MAX_STORE_TUPLES });
                    }
                    tuples.push(tuple);
                }
            }
            _ => {}
        }
    }
    let with_impact: Vec<&Tuple> = tuples.iter().collect();
    let (n, headers, data) = store(&with_impact, &tags, &HashMap::new(), false, false)?;
    let mut o = Out::default();
    o.u16(major, "cvar.majorVersion")?;
    o.u16(minor, "cvar.minorVersion")?;
    o.u16(n, "cvar.tupleVariationCount")?;
    o.u16(8 + count(headers.len()), "cvar.offsetToData")?;
    o.bytes(&headers);
    o.bytes(&data);
    Ok(o.0)
}

#[cfg(test)]
mod tests {
    use crate::TablesError;
    use crate::sfnt::{Flavour, SfntFont};
    use crate::ttx::{ImportOptions, import};

    /// A font with binary `head`, `loca` and `glyf`: `.notdef` empty and `A` as given.
    fn font_with(glyph: &[u8]) -> SfntFont {
        let mut font = SfntFont::new(Flavour::TrueType);
        let mut head = vec![0u8; 54];
        head[51] = 1;
        font.insert(*b"head", head);
        let mut loca = Vec::new();
        for offset in [0u32, 0, glyph.len() as u32] {
            loca.extend_from_slice(&offset.to_be_bytes());
        }
        font.insert(*b"loca", loca);
        font.insert(*b"glyf", glyph.to_vec());
        font
    }

    /// One contour of `points` points, all at the origin: repeated flags, no coordinate bytes.
    fn simple_glyph(points: usize) -> Vec<u8> {
        let mut g = vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 0];
        g.extend_from_slice(&u16::try_from(points - 1).unwrap().to_be_bytes());
        g.extend_from_slice(&[0, 0]);
        let mut left = points;
        while left > 0 {
            let run = left.min(256);
            g.extend_from_slice(&[0x39, u8::try_from(run - 1).unwrap()]);
            left -= run;
        }
        g
    }

    fn gvar_text(tuples: &str) -> String {
        format!(
            "<ttFont><GlyphOrder><GlyphID name=\".notdef\"/><GlyphID name=\"A\"/></GlyphOrder>\
             <gvar><version value=\"1\"/><reserved value=\"0\"/><axisCount value=\"1\"/>\
             <glyphVariations glyph=\"A\">{tuples}</glyphVariations></gvar></ttFont>"
        )
    }

    #[test]
    fn many_delta_sets_on_a_large_glyph_are_read_sparsely_and_capped() {
        // 60,000 points x 200,000 delta sets once took 24 bytes per point per set (about 300 GB).
        let font = font_with(&simple_glyph(60_000));
        let one = "<tuple><coord axis=\"0\" value=\"1.0\"/><delta pt=\"5\" x=\"10\" y=\"0\"/></tuple>";
        let text = gvar_text(&format!("{}{one}", "<tuple/>".repeat(200_000)));
        let out = import(font.clone(), &text, &ImportOptions::default()).unwrap();
        assert!(out.get(*b"gvar").is_some_and(|g| g.len() > 20));

        let many = gvar_text(&one.repeat(super::MAX_STORE_TUPLES + 1));
        assert!(matches!(import(font, &many, &ImportOptions::default()), Err(TablesError::TooLarge { .. })));
    }

    #[test]
    fn a_glyph_cannot_claim_more_points_than_its_flags() {
        // A 14-byte glyph whose last end point says 65,535 points.
        let short = [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFE, 0, 0];
        let text = gvar_text("<tuple><coord axis=\"0\" value=\"1.0\"/><delta pt=\"0\" x=\"1\" y=\"0\"/></tuple>");
        assert!(import(font_with(&short), &text, &ImportOptions::default()).is_err());
        assert!(import(font_with(&simple_glyph(3)), &text, &ImportOptions::default()).is_ok());
    }
}
