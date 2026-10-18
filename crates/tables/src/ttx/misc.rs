//! TTX writer: kern, DSIG, the Apple tables and the hex form of unknown tables (M12.7).
//!
//! `kern`, `DSIG`, `FFTM`, `LTSH`, `VDMX` and the Apple `prop` table, each written element for
//! element as fontTools' `ttx` writes it and read with the same checks: a table fontTools would
//! refuse to read is an error here, and [`super::dump`] then writes it as hex. Tables no module
//! knows are written as hex by [`super::dump`] itself. Ported from fontTools' `ttLib.tables`
//! (MIT): `_k_e_r_n`, `D_S_I_G_`, `F_F_T_M_`, `L_T_S_H_`, `V_D_M_X_`, the `prop` part of
//! `otData`, the `AATLookup` converter of `otConverters`, and `misc.timeTools`.
//!
//! Known differences: the Apple tables other than `prop` (`morx`, `kerx`, `trak`, `feat`, `ankr`,
//! `bsln`, `lcar`, `opbd`, `ltag`, `gcid`, `cidg`, `mort`) are left to the structured-table
//! writer ([`super::otdata`]) or written as hex.

#[cfg(test)]
mod fonts;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

use super::core::glyph_name;
use super::var::{bytes_at, i16_at, i32_at, slice, u8_at, u16_at, u32_at, u64_at};
use super::{XmlWriter, fixed_to_str, py_float};

/// Writes table `tag` (bytes `data`, from `font`) into `w`. Returns false, writing nothing,
/// for a tag this module does not handle.
pub fn write_table(tag: Tag, data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<bool> {
    match &tag {
        b"kern" => kern(data, font, w)?,
        b"DSIG" => dsig(data, w)?,
        b"FFTM" => fftm(data, w)?,
        b"LTSH" => ltsh(data, font, w)?,
        b"VDMX" => vdmx(data, w)?,
        b"prop" => prop(data, font, w)?,
        _ => return Ok(false),
    }
    Ok(true)
}

fn val(w: &mut XmlWriter, name: &str, value: impl ToString) {
    w.simple(name, &[("value", &value.to_string())]);
}

// ── kern ───────────────────────────────────────────────────────────────────────────────────────

fn kern(data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    let short_version = u16_at(data, 0)?;
    let short_count = u16_at(data, 2)?;
    let apple = data.len() >= 8 && short_version == 1;
    let (version, version_text, count, mut rest) = if apple {
        let raw = u32_at(data, 0)?;
        let v = f64::from(raw) / 65536.0;
        (v, py_float(v), u64::from(u32_at(data, 4)?), slice(data, 8, data.len()))
    } else {
        (f64::from(short_version), short_version.to_string(), u64::from(short_count), slice(data, 4, data.len()))
    };
    let order = w.glyph_order(font)?;
    val(w, "version", version_text);
    for _ in 0..count {
        let (mut length, format) = if version == 1.0 {
            (u32_at(rest, 0)? as usize, u8_at(rest, 5)?)
        } else {
            bytes_at(rest, 0, 6)?;
            let format = u8_at(rest, 4)?;
            let mut length = usize::from(u16_at(rest, 2)?);
            if count == 1 && format == 0 {
                length = usize::from(u16_at(rest, 6)?) * 6 + 14;
            }
            (length, format)
        };
        if version == 1.0 {
            bytes_at(rest, 0, 6)?;
        }
        let sub = slice(rest, 0, length);
        if format == 0 {
            kern_format0(sub, apple, &order, w)?;
        } else {
            w.open("kernsubtable", &[("format", &format.to_string())]);
            w.comment("unknown 'kern' subtable format");
            w.hex(sub);
            w.close("kernsubtable");
        }
        length = length.min(rest.len());
        rest = slice(rest, length, rest.len());
        w.check()?;
    }
    Ok(())
}

fn kern_format0(data: &[u8], apple: bool, order: &[String], w: &mut XmlWriter) -> Result<()> {
    let (coverage, format, tuple_index, header) = if apple {
        bytes_at(data, 0, 8)?;
        (u8_at(data, 4)?, u8_at(data, 5)?, Some(u16_at(data, 6)?), 8)
    } else {
        bytes_at(data, 0, 6)?;
        let version = u16_at(data, 0)?;
        if version != 0 {
            return Err(TablesError::invalid(format!("unsupported 'kern' subtable version {version}")));
        }
        (u8_at(data, 5)?, u8_at(data, 4)?, None, 6)
    };
    if format != 0 {
        return Err(TablesError::invalid("the 'kern' subtable format does not match its header"));
    }
    bytes_at(data, header, 8)?;
    let pairs = usize::from(u16_at(data, header)?);
    let start = header + 8;
    let values = slice(data, start, start + 6 * pairs);
    if values.len() < 6 * pairs {
        return Err(TablesError::invalid("the 'kern' pairs are cut short"));
    }
    let mut table: BTreeMap<(String, String), i16> = BTreeMap::new();
    for chunk in values.as_chunks::<6>().0 {
        let left = glyph_name(order, u32::from(u16_at(chunk, 0)?));
        let right = glyph_name(order, u32::from(u16_at(chunk, 2)?));
        table.insert((left, right), i16_at(chunk, 4)?);
    }
    let coverage = coverage.to_string();
    let format = format.to_string();
    let tuple = tuple_index.map(|t| t.to_string());
    let mut attributes = vec![("coverage", coverage.as_str()), ("format", format.as_str())];
    if let Some(t) = &tuple {
        attributes.push(("tupleIndex", t));
    }
    w.open("kernsubtable", &attributes);
    for ((left, right), value) in &table {
        w.simple("pair", &[("l", left), ("r", right), ("v", &value.to_string())]);
    }
    w.close("kernsubtable");
    Ok(())
}

// ── DSIG ───────────────────────────────────────────────────────────────────────────────────────

fn dsig(data: &[u8], w: &mut XmlWriter) -> Result<()> {
    bytes_at(data, 0, 8)?;
    let version = u32_at(data, 0)?;
    let count = usize::from(u16_at(data, 4)?);
    let flag = u16_at(data, 6)?;
    if version != 1 {
        return Err(TablesError::invalid("the 'DSIG' version must be 1"));
    }
    if flag & !1 != 0 {
        return Err(TablesError::invalid("the 'DSIG' flag must be 0 or 1"));
    }
    let mut records = Vec::with_capacity(count);
    for i in 0..count {
        let pos = 8 + 12 * i;
        let format = u32_at(data, pos)?;
        u32_at(data, pos + 4)?;
        let offset = u32_at(data, pos + 8)? as usize;
        if format != 1 {
            return Err(TablesError::invalid(format!("'DSIG' signature record {i} has format {format}")));
        }
        records.push((format, offset));
    }
    let mut blocks = Vec::with_capacity(count);
    for (format, offset) in records {
        bytes_at(data, offset, 8)?;
        if u16_at(data, offset)? != 0 || u16_at(data, offset.saturating_add(2))? != 0 {
            return Err(TablesError::invalid("a 'DSIG' signature block's reserved fields are not 0"));
        }
        let length = i64::from(i32_at(data, offset.saturating_add(4))?);
        let rest = slice(data, offset.saturating_add(8), data.len());
        // A Python slice `rest[:length]`: a negative length counts from the end.
        let end = if length < 0 {
            rest.len().saturating_sub(usize::try_from(length.unsigned_abs()).unwrap_or(usize::MAX))
        } else {
            usize::try_from(length).unwrap_or(usize::MAX)
        };
        blocks.push((format, slice(rest, 0, end)));
    }
    w.comment("note that the Digital Signature will be invalid after recompilation!");
    w.simple("tableHeader", &[("flag", &format!("0x{flag:X}")), ("numSigs", &count.to_string()), ("version", &version.to_string())]);
    for (format, pkcs7) in blocks {
        w.open("SignatureRecord", &[("format", &format.to_string())]);
        let encoded = base64(pkcs7);
        unindented(w, "-----BEGIN PKCS7-----");
        for line in encoded.as_bytes().chunks(76) {
            unindented(w, &String::from_utf8_lossy(line));
        }
        unindented(w, "-----END PKCS7-----");
        w.add_text(&encoded);
        w.close("SignatureRecord");
        w.check()?;
    }
    Ok(())
}

/// A line at the left margin, as fontTools' `write_noindent` writes it.
fn unindented(w: &mut XmlWriter, line: &str) {
    let depth = w.depth;
    w.depth = 0;
    w.line(line);
    w.depth = depth;
}

fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let digit = |i: u32| char::from(ALPHABET.get((i & 63) as usize).copied().unwrap_or(b'A'));
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let n = b(0) << 16 | b(1) << 8 | b(2);
        out.push(digit(n >> 18));
        out.push(digit(n >> 12));
        out.push(if chunk.len() > 1 { digit(n >> 6) } else { '=' });
        out.push(if chunk.len() > 2 { digit(n) } else { '=' });
    }
    out
}

// ── FFTM ───────────────────────────────────────────────────────────────────────────────────────

fn fftm(data: &[u8], w: &mut XmlWriter) -> Result<()> {
    bytes_at(data, 0, 28)?;
    let version = u32_at(data, 0)?;
    let stamps = [
        ("FFTimeStamp", timestamp(u64_at(data, 4)?)?),
        ("sourceCreated", timestamp(u64_at(data, 12)?)?),
        ("sourceModified", timestamp(u64_at(data, 20)?)?),
    ];
    w.comment("FontForge's timestamp, font source creation and modification dates"); // brand-ok: the comment ttx writes
    val(w, "version", version);
    for (name, text) in stamps {
        val(w, name, text);
    }
    Ok(())
}

/// fontTools' `timestampToString`: seconds since 1904 as `asctime` writes them in UTC, times
/// before 1970 written as 1970.
fn timestamp(value: u64) -> Result<String> {
    const EPOCH_DIFF: i128 = 2_082_844_800;
    let secs = (i128::from(value) - EPOCH_DIFF).max(0);
    let days = i64::try_from(secs / 86_400).map_err(|_| TablesError::invalid("a timestamp is out of range"))?;
    let rem = i64::try_from(secs % 86_400).unwrap_or(0);
    let (y, m, d) = civil_from_days(days);
    if y > i64::from(i32::MAX) {
        return Err(TablesError::invalid("a timestamp is out of range"));
    }
    const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let weekday = DAYS.get(usize::try_from((days + 3).rem_euclid(7)).unwrap_or(0)).copied().unwrap_or("Mon");
    let month = MONTHS.get(usize::try_from(m - 1).unwrap_or(0)).copied().unwrap_or("Jan");
    Ok(format!("{weekday} {month} {d:>2} {:02}:{:02}:{:02} {y}", rem / 3600, rem % 3600 / 60, rem % 60))
}

/// Year, month and day of a day count from 1970-01-01 (H. Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ── LTSH ───────────────────────────────────────────────────────────────────────────────────────

fn ltsh(data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    let version = u16_at(data, 0)?;
    let count = usize::from(u16_at(data, 2)?);
    if version != 0 {
        return Err(TablesError::invalid(format!("unknown 'LTSH' version {version}")));
    }
    let pels = slice(data, 4, data.len());
    if count == 0 || pels.len() % count >= 4 {
        return Err(TablesError::invalid("the 'LTSH' glyph count does not match its length"));
    }
    let order = w.glyph_order(font)?;
    let mut by_name = BTreeMap::new();
    for i in 0..count {
        let pel = pels.get(i).copied().ok_or_else(|| TablesError::invalid("the 'LTSH' table is cut short"))?;
        by_name.insert(glyph_name(&order, u32::try_from(i).unwrap_or(u32::MAX)), pel);
    }
    for (name, pel) in &by_name {
        w.simple("yPel", &[("name", name), ("value", &pel.to_string())]);
    }
    Ok(())
}

// ── VDMX ───────────────────────────────────────────────────────────────────────────────────────

fn vdmx(data: &[u8], w: &mut XmlWriter) -> Result<()> {
    let version = u16_at(data, 0)?;
    let groups_count = usize::from(u16_at(data, 2)?);
    let ratios_count = usize::from(u16_at(data, 4)?);
    let mut pos = 6usize;
    let mut ratios = Vec::with_capacity(ratios_count);
    for _ in 0..ratios_count {
        let r = bytes_at(data, pos, 4)?;
        ratios.push([byte(r, 0), byte(r, 1), byte(r, 2), byte(r, 3)]);
        pos += 4;
    }
    let mut offsets = Vec::with_capacity(ratios_count);
    for _ in 0..ratios_count {
        offsets.push(usize::from(u16_at(data, pos)?));
        pos += 2;
    }
    let mut group_of: Vec<Option<usize>> = vec![None; ratios_count];
    let mut groups: Vec<BTreeMap<u16, (i16, i16)>> = Vec::with_capacity(groups_count.min(data.len()));
    for index in 0..groups_count {
        let start = pos;
        bytes_at(data, pos, 4)?;
        let records = usize::from(u16_at(data, pos)?);
        let first = u8_at(data, pos + 2)?;
        let last = u8_at(data, pos + 3)?;
        pos += 4;
        let mut group = BTreeMap::new();
        for _ in 0..records {
            bytes_at(data, pos, 6)?;
            group.insert(u16_at(data, pos)?, (i16_at(data, pos + 2)?, i16_at(data, pos + 4)?));
            pos += 6;
        }
        let (Some((&min, _)), Some((&max, _))) = (group.first_key_value(), group.last_key_value()) else {
            return Err(TablesError::invalid(format!("'VDMX' group {index} has no records")));
        };
        if u16::from(first) != min || u16::from(last) != max {
            return Err(TablesError::invalid(format!("'VDMX' group {index}'s sizes do not match its records")));
        }
        groups.push(group);
        for (ratio, offset) in offsets.iter().enumerate() {
            if *offset == start
                && let Some(slot) = group_of.get_mut(ratio)
            {
                *slot = Some(index);
            }
        }
        w.check()?;
    }
    val(w, "version", version);
    w.open("ratRanges", &[]);
    for (i, r) in ratios.iter().enumerate() {
        let group = group_of.get(i).copied().flatten().ok_or_else(|| TablesError::invalid(format!("no 'VDMX' group for ratio range {i}")))?;
        let [charset, x, y_start, y_end] = *r;
        w.simple(
            "ratRange",
            &[
                ("bCharSet", &charset.to_string()),
                ("groupIndex", &group.to_string()),
                ("xRatio", &x.to_string()),
                ("yEndRatio", &y_end.to_string()),
                ("yStartRatio", &y_start.to_string()),
            ],
        );
    }
    w.close("ratRanges");
    w.open("groups", &[]);
    for (index, group) in groups.iter().enumerate() {
        w.open("group", &[("index", &index.to_string())]);
        let (min, max) = (group.keys().next().copied().unwrap_or(0), group.keys().last().copied().unwrap_or(0));
        w.comment(&format!("recs={}, startsz={min}, endsz={max}", group.len()));
        for (height, (y_max, y_min)) in group {
            w.simple("record", &[("yPelHeight", &height.to_string()), ("yMax", &y_max.to_string()), ("yMin", &y_min.to_string())]);
        }
        w.close("group");
        w.check()?;
    }
    w.close("groups");
    Ok(())
}

fn byte(bytes: &[u8], i: usize) -> u8 {
    bytes.get(i).copied().unwrap_or(0)
}

// ── prop and AAT lookups ───────────────────────────────────────────────────────────────────────

fn prop(data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    let version = i32_at(data, 0)?;
    let format = u16_at(data, 4)?;
    let default = u16_at(data, 6)?;
    if format > 1 {
        return Err(TablesError::invalid(format!("unknown 'prop' format {format}")));
    }
    let order = w.glyph_order(font)?;
    let lookup = if format == 1 { Some(aat_lookup(data, 8, &order)?) } else { None };
    val(w, "Version", fixed_to_str(f64::from(version) / 65536.0, 16));
    w.open("GlyphProperties", &[("Format", &format.to_string())]);
    val(w, "DefaultProperties", default);
    if let Some(lookup) = lookup {
        write_lookup(w, "Properties", &lookup);
    }
    w.close("GlyphProperties");
    Ok(())
}

fn write_lookup(w: &mut XmlWriter, name: &str, lookup: &[(String, u16)]) {
    w.open(name, &[]);
    for (glyph, value) in lookup {
        w.simple("Lookup", &[("glyph", glyph), ("value", &value.to_string())]);
    }
    w.close(name);
}

/// An AAT lookup table of 16-bit values at `at` (formats 0, 2, 4, 6 and 8), as glyph names and
/// values sorted by name. Later segments win where segments overlap, as in fontTools.
fn aat_lookup(data: &[u8], at: usize, order: &[String]) -> Result<Vec<(String, u16)>> {
    let format = u16_at(data, at)?;
    let mut map: BTreeMap<u32, u16> = BTreeMap::new();
    match format {
        0 => {
            for gid in 0..order.len() {
                map.insert(u32::try_from(gid).unwrap_or(u32::MAX), u16_at(data, at + 2 + 2 * gid)?);
            }
        }
        2 | 4 | 6 => {
            let unit = usize::from(u16_at(data, at + 2)?);
            let units = usize::from(u16_at(data, at + 4)?);
            let least = match format {
                2 | 4 => 6,
                _ => 4,
            };
            if unit < least {
                return Err(TablesError::invalid(format!("an AAT lookup's unit size {unit} is too small")));
            }
            // Segments are applied last to first, each glyph taking the first value it meets, so
            // overlapping segments cost no more than the glyphs they cover.
            let mut filled = Unfilled::new();
            let mut segments = Vec::with_capacity(units);
            for i in 0..units {
                let pos = at.saturating_add(i * unit + 12);
                segments.push(pos);
                u16_at(data, pos)?;
                u16_at(data, pos + 2)?;
                if format != 6 {
                    u16_at(data, pos + 4)?;
                }
            }
            for pos in segments.into_iter().rev() {
                match format {
                    6 => {
                        let gid = u16_at(data, pos)?;
                        if gid != 0xFFFF {
                            map.entry(u32::from(gid)).or_insert(u16_at(data, pos + 2)?);
                        }
                    }
                    _ => {
                        let last = u32::from(u16_at(data, pos)?);
                        let first = u32::from(u16_at(data, pos + 2)?);
                        let value = u16_at(data, pos + 4)?;
                        if last == 0xFFFF || last < first {
                            continue;
                        }
                        let array = at.saturating_add(usize::from(value));
                        if format == 4 {
                            bytes_at(data, array, 2 * (last - first + 1) as usize)?;
                        }
                        let mut gid = filled.next(first);
                        while gid <= last {
                            let v = if format == 2 { value } else { u16_at(data, array + 2 * (gid - first) as usize)? };
                            map.insert(gid, v);
                            filled.fill(gid);
                            gid = filled.next(gid);
                        }
                    }
                }
            }
        }
        8 => {
            let first = u32::from(u16_at(data, at + 2)?);
            let count = usize::from(u16_at(data, at + 4)?);
            for k in 0..count {
                map.insert(first + u32::try_from(k).unwrap_or(0), u16_at(data, at + 6 + 2 * k)?);
            }
        }
        _ => return Err(TablesError::invalid(format!("unsupported AAT lookup format {format}"))),
    }
    let mut out: Vec<(String, u16)> = map.into_iter().map(|(gid, v)| (glyph_name(order, gid), v)).collect();
    out.sort();
    out.dedup_by(|b, a| a.0 == b.0);
    Ok(out)
}

/// The glyph ids not yet given a value, with a skip pointer per id (path-halving union-find),
/// so filling a range touches each id once.
struct Unfilled {
    next: Vec<u32>,
}

impl Unfilled {
    fn new() -> Self {
        Self { next: (0..=0x1_0000).collect() }
    }

    /// The first unfilled id at or after `gid`.
    fn next(&mut self, gid: u32) -> u32 {
        let mut g = gid;
        while let Some(&n) = self.next.get(g as usize) {
            if n == g {
                return g;
            }
            let skip = self.next.get(n as usize).copied().unwrap_or(n);
            if let Some(slot) = self.next.get_mut(g as usize) {
                *slot = skip;
            }
            g = skip;
        }
        g
    }

    fn fill(&mut self, gid: u32) {
        if let Some(slot) = self.next.get_mut(gid as usize) {
            *slot = gid + 1;
        }
    }
}
