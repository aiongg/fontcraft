//! TTX writer: the core TrueType and OpenType tables (M12.4).
//!
//! `head hhea maxp OS/2 name post cmap hmtx loca glyf gasp vhea vmtx fpgm prep cvt hdmx`, each
//! written element for element as fontTools' `ttx` writes it, read with the same checks: a table
//! fontTools would refuse to read is an error here too, and [`super::dump`] then writes it as hex.
//! Ported from fontTools' `ttLib.tables` (MIT): `_h_e_a_d`, `_h_h_e_a`, `_v_h_e_a`, `_m_a_x_p`,
//! `O_S_2f_2`, `_n_a_m_e`, `_p_o_s_t`, `_c_m_a_p`, `_h_m_t_x`, `_l_o_c_a`, `_g_l_y_f`, `_g_a_s_p`,
//! `_c_v_t`, `_f_p_g_m`, `_h_d_m_x`, `ttProgram`, and `misc.timeTools`.
//!
//! Known differences: the character names `ttx` writes as comments after `cmap` mappings are left
//! out (comments carry no data), and `name` records in a legacy encoding other than Mac Roman,
//! Latin-1 and ASCII are written as escaped bytes when they hold non-ASCII bytes.

pub(crate) mod cmap;
mod glyf;
pub(crate) use glyf::write_split as write_glyf_split;
pub(crate) mod name;
pub(crate) mod program;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

use super::{XmlWriter, fixed_to_str, num_glyphs, num2binary, py_float, py_hex};

/// Writes table `tag` (bytes `data`, from `font`) into `w`. Returns false, writing nothing,
/// for a tag this module does not handle.
pub fn write_table(tag: Tag, data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<bool> {
    match &tag {
        b"head" => head(data, w)?,
        b"hhea" => header(data, w, &HHEA_FIELDS)?,
        b"vhea" => header(data, w, &VHEA_FIELDS)?,
        b"maxp" => maxp(data, w)?,
        b"OS/2" => os2(data, w)?,
        b"name" => name::write(data, w)?,
        b"post" => post(data, font, w)?,
        b"cmap" => cmap::write(data, font, w)?,
        b"hmtx" => metrics(data, font, w, false)?,
        b"vmtx" => metrics(data, font, w, true)?,
        b"loca" => loca(data, font, w)?,
        b"glyf" => glyf::write(data, font, w)?,
        b"gasp" => gasp(data, w)?,
        b"fpgm" | b"prep" => program::write(data, w),
        b"cvt " => cvt(data, w)?,
        b"hdmx" => hdmx(data, font, w)?,
        _ => return Ok(false),
    }
    Ok(true)
}

/// Big-endian reading with errors instead of panics.
pub(crate) struct Rd<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Rd<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub(crate) fn at(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }

    pub(crate) fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or_else(cut)?;
        let b = self.data.get(self.pos..end).ok_or_else(cut)?;
        self.pos = end;
        Ok(b)
    }

    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?.first().copied().unwrap_or(0))
    }

    pub(crate) fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()? as i8)
    }

    pub(crate) fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)]))
    }

    pub(crate) fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }

    pub(crate) fn u24(&mut self) -> Result<u32> {
        let b = self.bytes(3)?;
        Ok(b.iter().fold(0u32, |acc, x| acc << 8 | u32::from(*x)))
    }

    pub(crate) fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(b.iter().fold(0u32, |acc, x| acc << 8 | u32::from(*x)))
    }

    pub(crate) fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }

    pub(crate) fn u64(&mut self) -> Result<u64> {
        let b = self.bytes(8)?;
        Ok(b.iter().fold(0u64, |acc, x| acc << 8 | u64::from(*x)))
    }

    pub(crate) fn rest(&self) -> &'a [u8] {
        self.data.get(self.pos..).unwrap_or_default()
    }
}

fn cut() -> TablesError {
    TablesError::invalid("the table is cut short")
}

fn val(w: &mut XmlWriter, name: &str, value: impl ToString) {
    w.simple(name, &[("value", &value.to_string())]);
}

fn fixed16(raw: i32) -> String {
    fixed_to_str(f64::from(raw) / 65536.0, 16)
}

// ── head ───────────────────────────────────────────────────────────────────────────────────────

fn head(data: &[u8], w: &mut XmlWriter) -> Result<()> {
    if data.len() < 54 {
        return Err(cut());
    }
    let rest = data.get(54..).unwrap_or_default();
    if !rest.is_empty() && rest != [0, 0] {
        return Err(TablesError::invalid("extra bytes at the end of the 'head' table"));
    }
    let mut r = Rd::new(data);
    w.comment("Most of this table will be recalculated by the compiler");
    val(w, "tableVersion", fixed16(r.i32()?));
    val(w, "fontRevision", fixed16(r.i32()?));
    val(w, "checkSumAdjustment", format!("0x{:x}", r.u32()?));
    val(w, "magicNumber", format!("0x{:x}", r.u32()?));
    val(w, "flags", num2binary(u64::from(r.u16()?), 16));
    val(w, "unitsPerEm", r.u16()?);
    val(w, "created", timestamp(r.u64()?));
    val(w, "modified", timestamp(r.u64()?));
    for name in ["xMin", "yMin", "xMax", "yMax"] {
        val(w, name, r.i16()?);
    }
    val(w, "macStyle", num2binary(u64::from(r.u16()?), 16));
    val(w, "lowestRecPPEM", r.u16()?);
    for name in ["fontDirectionHint", "indexToLocFormat", "glyphDataFormat"] {
        val(w, name, r.i16()?);
    }
    Ok(())
}

/// A `LONGDATETIME` as fontTools writes it (`Thu Jan  1 00:00:00 1970`), with its repairs: the
/// top bytes of values past 32 bits are dropped, and values that look like Unix times are moved
/// to 1904.
fn timestamp(value: u64) -> String {
    const UNIX_1970: u64 = 0x7C25_9DC0;
    const EPOCH_DIFF: i64 = -2_082_844_800;
    let mut value = value;
    if value > 0xFFFF_FFFF {
        value &= 0xFFFF_FFFF;
    }
    if value < UNIX_1970 {
        value = value.saturating_add(UNIX_1970);
    }
    let secs = (value as i64).saturating_add(EPOCH_DIFF).max(0);
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let weekday = DAYS.get(usize::try_from((days + 3).rem_euclid(7)).unwrap_or(0)).copied().unwrap_or("Mon");
    let month = MONTHS.get(usize::try_from(m.saturating_sub(1)).unwrap_or(0)).copied().unwrap_or("Jan");
    format!("{weekday} {month} {d:>2} {:02}:{:02}:{:02} {y}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Year, month and day of a day count from 1970-01-01 (H. Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

// ── hhea, vhea ─────────────────────────────────────────────────────────────────────────────────

const HHEA_FIELDS: [&str; 17] = [
    "tableVersion",
    "ascent",
    "descent",
    "lineGap",
    "advanceWidthMax",
    "minLeftSideBearing",
    "minRightSideBearing",
    "xMaxExtent",
    "caretSlopeRise",
    "caretSlopeRun",
    "caretOffset",
    "reserved0",
    "reserved1",
    "reserved2",
    "reserved3",
    "metricDataFormat",
    "numberOfHMetrics",
];
const VHEA_FIELDS: [&str; 17] = [
    "tableVersion",
    "ascent",
    "descent",
    "lineGap",
    "advanceHeightMax",
    "minTopSideBearing",
    "minBottomSideBearing",
    "yMaxExtent",
    "caretSlopeRise",
    "caretSlopeRun",
    "caretOffset",
    "reserved1",
    "reserved2",
    "reserved3",
    "reserved4",
    "metricDataFormat",
    "numberOfVMetrics",
];

/// `hhea` and `vhea`: exactly 36 bytes; the fourth field and the last are unsigned.
fn header(data: &[u8], w: &mut XmlWriter, names: &[&str; 17]) -> Result<()> {
    if data.len() != 36 {
        return Err(TablesError::invalid(format!("the table has {} bytes; it must have 36", data.len())));
    }
    let mut r = Rd::new(data);
    for (i, name) in names.iter().enumerate() {
        let value = match i {
            0 => format!("0x{:08x}", r.u32()?),
            4 | 16 => r.u16()?.to_string(),
            _ => r.i16()?.to_string(),
        };
        val(w, name, value);
    }
    Ok(())
}

/// The number of long metrics an `hhea` or `vhea` table gives.
fn metrics_count(data: &[u8]) -> Result<usize> {
    if data.len() != 36 {
        return Err(TablesError::invalid("the metrics header could not be read"));
    }
    Ok(usize::from(Rd::at(data, 34).u16()?))
}

// ── maxp ───────────────────────────────────────────────────────────────────────────────────────

const MAXP_FIELDS: [&str; 13] = [
    "maxPoints",
    "maxContours",
    "maxCompositePoints",
    "maxCompositeContours",
    "maxZones",
    "maxTwilightPoints",
    "maxStorage",
    "maxFunctionDefs",
    "maxInstructionDefs",
    "maxStackElements",
    "maxSizeOfInstructions",
    "maxComponentElements",
    "maxComponentDepth",
];

fn maxp(data: &[u8], w: &mut XmlWriter) -> Result<()> {
    let mut r = Rd::new(data);
    let version = r.i32()?;
    let num_glyphs = r.u16()?;
    let full = version != 0x0000_5000;
    if data.len() != if full { 32 } else { 6 } {
        return Err(TablesError::invalid("the 'maxp' table has the wrong length for its version"));
    }
    if full {
        w.comment("Most of this table will be recalculated by the compiler");
    }
    val(w, "tableVersion", py_hex(i64::from(version)));
    val(w, "numGlyphs", num_glyphs);
    if full {
        for name in MAXP_FIELDS {
            val(w, name, r.u16()?);
        }
    }
    Ok(())
}

// ── OS/2 ───────────────────────────────────────────────────────────────────────────────────────

fn os2(data: &[u8], w: &mut XmlWriter) -> Result<()> {
    let version = Rd::new(data).u16()?;
    let size = match version {
        0 => 78,
        1 => 86,
        2..=4 => 96,
        5 => 100,
        v => return Err(TablesError::invalid(format!("unknown format for the 'OS/2' table: version {v}"))),
    };
    if data.len() < size {
        return Err(cut());
    }
    let mut r = Rd::new(data);
    w.comment("The fields 'usFirstCharIndex' and 'usLastCharIndex'\nwill be recalculated by the compiler");
    val(w, "version", r.u16()?);
    val(w, "xAvgCharWidth", r.i16()?);
    val(w, "usWeightClass", r.u16()?);
    val(w, "usWidthClass", r.u16()?);
    val(w, "fsType", num2binary(u64::from(r.u16()?), 16));
    for name in [
        "ySubscriptXSize",
        "ySubscriptYSize",
        "ySubscriptXOffset",
        "ySubscriptYOffset",
        "ySuperscriptXSize",
        "ySuperscriptYSize",
        "ySuperscriptXOffset",
        "ySuperscriptYOffset",
        "yStrikeoutSize",
        "yStrikeoutPosition",
        "sFamilyClass",
    ] {
        val(w, name, r.i16()?);
    }
    w.open("panose", &[]);
    for name in
        ["bFamilyType", "bSerifStyle", "bWeight", "bProportion", "bContrast", "bStrokeVariation", "bArmStyle", "bLetterForm", "bMidline", "bXHeight"]
    {
        val(w, name, r.u8()?);
    }
    w.close("panose");
    for name in ["ulUnicodeRange1", "ulUnicodeRange2", "ulUnicodeRange3", "ulUnicodeRange4"] {
        val(w, name, num2binary(u64::from(r.u32()?), 32));
    }
    val(w, "achVendID", vendor_id(r.bytes(4)?));
    val(w, "fsSelection", num2binary(u64::from(r.u16()?), 16));
    val(w, "usFirstCharIndex", r.u16()?);
    val(w, "usLastCharIndex", r.u16()?);
    val(w, "sTypoAscender", r.i16()?);
    val(w, "sTypoDescender", r.i16()?);
    val(w, "sTypoLineGap", r.i16()?);
    val(w, "usWinAscent", r.u16()?);
    val(w, "usWinDescent", r.u16()?);
    if version >= 1 {
        val(w, "ulCodePageRange1", num2binary(u64::from(r.u32()?), 32));
        val(w, "ulCodePageRange2", num2binary(u64::from(r.u32()?), 32));
    }
    if version >= 2 {
        val(w, "sxHeight", r.i16()?);
        val(w, "sCapHeight", r.i16()?);
        val(w, "usDefaultChar", r.u16()?);
        val(w, "usBreakChar", r.u16()?);
        val(w, "usMaxContext", r.u16()?);
    }
    if version >= 5 {
        val(w, "usLowerOpticalPointSize", py_float(f64::from(r.u16()?) / 20.0));
        val(w, "usUpperOpticalPointSize", py_float(f64::from(r.u16()?) / 20.0));
    }
    Ok(())
}

/// The vendor id as fontTools writes it: Python's `repr()` without its first and last
/// characters, of the text when the bytes are ASCII, of the bytes otherwise (which leaves the
/// opening quote in, as fontTools does).
fn vendor_id(bytes: &[u8]) -> String {
    let ascii = bytes.iter().all(u8::is_ascii);
    let quote = if bytes.contains(&b'\'') && !bytes.contains(&b'"') { '"' } else { '\'' };
    let mut body = String::new();
    for &b in bytes {
        match b {
            b'\\' => body.push_str("\\\\"),
            b'\t' => body.push_str("\\t"),
            b'\n' => body.push_str("\\n"),
            b'\r' => body.push_str("\\r"),
            b if b as char == quote => {
                body.push('\\');
                body.push(quote);
            }
            0x20..=0x7E => body.push(b as char),
            b => body.push_str(&format!("\\x{b:02x}")),
        }
    }
    if ascii { body } else { format!("{quote}{body}") }
}

// ── post ───────────────────────────────────────────────────────────────────────────────────────

/// What fontTools reads from a `post` table besides its header.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PostNames {
    /// The names, made unique (format 1, 2, 4); `None` for format 3.
    pub glyph_order: Option<Vec<String>>,
    /// Unique name to `post` name, where they differ (formats 2 and 4).
    pub mapping: Option<BTreeMap<String, String>>,
    /// The names past the standard Macintosh set (format 2).
    pub extra_names: Option<Vec<String>>,
}

/// Reads the glyph names of a `post` table for a font of `count` glyphs.
pub(crate) fn post_names(data: &[u8], count: usize) -> Result<PostNames> {
    use read_fonts::tables::post::DEFAULT_GLYPH_NAMES;
    if data.len() < 32 {
        return Err(cut());
    }
    let format = Rd::new(data).u32()?;
    let rest = data.get(32..).unwrap_or_default();
    match format {
        0x0001_0000 => {
            let names = DEFAULT_GLYPH_NAMES.iter().take(count).map(|s| (*s).to_string()).collect();
            Ok(PostNames { glyph_order: Some(names), ..PostNames::default() })
        }
        0x0002_0000 => {
            let mut r = Rd::new(rest);
            let n = usize::from(r.u16()?).min(count);
            let mut indices = Vec::with_capacity(n);
            for _ in 0..n {
                indices.push(usize::from(r.u16()?));
            }
            let max = indices.iter().copied().max().ok_or_else(|| TablesError::invalid("the 'post' table names no glyphs"))?;
            let extra = pascal_strings(r.rest(), max.saturating_sub(257));
            let mut order = vec![String::new(); count];
            for (slot, index) in order.iter_mut().zip(&indices) {
                *slot = if *index > 257 {
                    extra.get(index.saturating_sub(258)).cloned().unwrap_or_default()
                } else {
                    DEFAULT_GLYPH_NAMES.get(*index).map(|s| (*s).to_string()).unwrap_or_default()
                };
            }
            let (order, mapping) = ps_name_mapping(order);
            Ok(PostNames { glyph_order: Some(order), mapping: Some(mapping), extra_names: Some(extra) })
        }
        0x0003_0000 => Ok(PostNames::default()),
        0x0004_0000 => {
            if rest.len() % 2 != 0 {
                return Err(TablesError::invalid("the 'post' format 4 data has an odd length"));
            }
            let mut order = vec![String::new(); count];
            for (slot, pair) in order.iter_mut().zip(rest.chunks(2)) {
                let code = u32::from(u16::from_be_bytes([pair.first().copied().unwrap_or(0), pair.get(1).copied().unwrap_or(0)]));
                *slot = if code == 0xFFFF { String::new() } else { agl_name(code) };
            }
            let (order, mapping) = ps_name_mapping(order);
            Ok(PostNames { glyph_order: Some(order), mapping: Some(mapping), extra_names: None })
        }
        f => Err(TablesError::invalid(format!("'post' table format {} is not supported", py_float(f64::from(f as i32) / 65536.0)))),
    }
}

/// fontTools' `unpackPStrings`: `n` Pascal strings, empty ones past the end of the data.
fn pascal_strings(data: &[u8], n: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(n.min(65_536));
    let mut index = 0usize;
    for _ in 0..n {
        let length = data.get(index).map_or(0, |b| usize::from(*b));
        index = index.saturating_add(1);
        let end = index.saturating_add(length);
        let name = if data.len().saturating_add(1) <= end {
            String::new()
        } else {
            data.get(index..end).unwrap_or_default().iter().map(|&b| b as char).collect()
        };
        out.push(name);
        index = end;
    }
    out
}

/// fontTools' `build_psNameMapping`: empty names become `glyphNNNNN`, repeated ones get `.1`,
/// `.2`… (skipping names the list already has), and every renamed glyph is remembered.
fn ps_name_mapping(mut order: Vec<String>) -> (Vec<String>, BTreeMap<String, String>) {
    use std::collections::{HashMap, HashSet};
    let originals: HashSet<String> = order.iter().cloned().collect();
    let mut all: HashMap<String, usize> = HashMap::new();
    let mut mapping = BTreeMap::new();
    for (i, slot) in order.iter_mut().enumerate() {
        let ps_name = slot.clone();
        let mut name = if ps_name.is_empty() { format!("glyph{i:05}") } else { ps_name.clone() };
        if let Some(&n) = all.get(&name) {
            let mut n = n;
            while originals.contains(&format!("{name}.{n}")) {
                n = n.saturating_add(1);
            }
            all.insert(name.clone(), n.saturating_add(1));
            name = format!("{name}.{n}");
        }
        all.insert(name.clone(), 1);
        if name != ps_name {
            mapping.insert(name.clone(), ps_name);
            *slot = name;
        }
    }
    (order, mapping)
}

fn post(data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    let count = num_glyphs(font)?;
    let names = post_names(data, count)?;
    let mut r = Rd::new(data);
    val(w, "formatType", py_float(f64::from(r.i32()?) / 65536.0));
    val(w, "italicAngle", py_float(f64::from(r.i32()?) / 65536.0));
    val(w, "underlinePosition", r.i16()?);
    val(w, "underlineThickness", r.i16()?);
    for name in ["isFixedPitch", "minMemType42", "maxMemType42", "minMemType1", "maxMemType1"] {
        val(w, name, r.u32()?);
    }
    if let Some(mapping) = &names.mapping {
        w.open("psNames", &[]);
        w.comment(
            "This file uses unique glyph names based on the information\n\
             found in the 'post' table. Since these names might not be unique,\n\
             we have to invent artificial names in case of clashes. In order to\n\
             be able to retain the original information, we need a name to\n\
             ps name mapping for those cases where they differ. That's what\n\
             you see below.\n",
        );
        for (name, ps_name) in mapping {
            w.simple("psName", &[("name", name), ("psName", ps_name)]);
        }
        w.close("psNames");
    }
    if let Some(extra) = &names.extra_names {
        w.open("extraNames", &[]);
        w.comment("following are the name that are not taken from the standard Mac glyph order");
        for name in extra {
            w.simple("psName", &[("name", name)]);
        }
        w.close("extraNames");
    }
    Ok(())
}

/// The name fontTools gives a code point: its AGLFN name, else `uniXXXX` or `uXXXXX`.
pub(crate) fn agl_name(code: u32) -> String {
    match fontcraft_model::glyphdata::GlyphData::new().ok().and_then(|d| d.aglfn_name(code)) {
        Some(name) => name.to_string(),
        None if code <= 0xFFFF => format!("uni{code:04X}"),
        None => format!("u{code:X}"),
    }
}

/// fontTools' names for a font without usable `post` names: `glyphNNNNN`, `.notdef` first, and
/// for each glyph a Unicode subtable maps, the name of its lowest code point (`.alt1`, `.alt2`…
/// on repeats).
pub(crate) fn names_from_cmap(font: &SfntFont, count: usize) -> Vec<String> {
    let mut order: Vec<String> = (0..count).map(|i| format!("glyph{i:05}")).collect();
    if let Some(first) = order.first_mut() {
        *first = ".notdef".into();
    }
    let Some(data) = font.get(*b"cmap") else { return order };
    let Ok((_, subtables)) = cmap::parse(data) else { return order };
    let mut lowest: Vec<Option<u32>> = vec![None; count];
    // Records that share a subtable give the same names: each is read once.
    let mut done: std::collections::HashSet<*const cmap::Kind> = std::collections::HashSet::new();
    for st in &subtables {
        if !st.is_unicode() || !done.insert(std::rc::Rc::as_ptr(&st.kind)) {
            continue;
        }
        for &(code, gid) in st.map() {
            if let Some(slot) = usize::try_from(gid).ok().and_then(|g| lowest.get_mut(g)) {
                *slot = Some(slot.map_or(code, |c| c.min(code)));
            }
        }
    }
    let mut uses: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (slot, code) in order.iter_mut().zip(&lowest) {
        let Some(code) = code else { continue };
        let base = agl_name(*code);
        let n = uses.entry(base.clone()).or_insert(0);
        *n = n.saturating_add(1);
        *slot = if *n > 1 { format!("{base}.alt{}", n.saturating_sub(1)) } else { base };
    }
    order
}

/// The name of glyph `gid`, or `glyphNNNNN` past the end of the glyph order.
pub(crate) fn glyph_name(order: &[String], gid: u32) -> String {
    usize::try_from(gid).ok().and_then(|g| order.get(g)).cloned().unwrap_or_else(|| format!("glyph{gid:05}"))
}

// ── hmtx, vmtx ─────────────────────────────────────────────────────────────────────────────────

fn metrics(data: &[u8], font: &SfntFont, w: &mut XmlWriter, vertical: bool) -> Result<()> {
    let count = num_glyphs(font)?;
    let header_tag = if vertical { *b"vhea" } else { *b"hhea" };
    let long = match font.get(header_tag) {
        Some(h) => metrics_count(h)?,
        None => count,
    }
    .min(count);
    let side = count.saturating_sub(long);
    let needed = long.saturating_mul(4).saturating_add(side.saturating_mul(2));
    if data.len() < needed {
        return Err(TablesError::invalid(format!("not enough metrics data: expected {needed} bytes, got {}", data.len())));
    }
    if long == 0 {
        return Err(TablesError::invalid("the metrics table has no long metrics"));
    }
    let order = w.glyph_order(font)?;
    let mut r = Rd::new(data);
    let mut by_name: BTreeMap<String, (u16, i16)> = BTreeMap::new();
    let mut last_advance = 0u16;
    for i in 0..long {
        let advance = r.u16()?;
        let bearing = r.i16()?;
        last_advance = advance;
        let name = order.get(i).ok_or_else(|| TablesError::invalid("the glyph order is shorter than the metrics"))?;
        by_name.insert(name.clone(), (advance, bearing));
    }
    for i in 0..side {
        let bearing = r.i16()?;
        let name = order.get(long.saturating_add(i)).ok_or_else(|| TablesError::invalid("the glyph order is shorter than the metrics"))?;
        by_name.insert(name.clone(), (last_advance, bearing));
    }
    let (advance_name, bearing_name) = if vertical { ("height", "tsb") } else { ("width", "lsb") };
    for (name, (advance, bearing)) in &by_name {
        w.simple("mtx", &[("name", name), (advance_name, &advance.to_string()), (bearing_name, &bearing.to_string())]);
    }
    Ok(())
}

// ── loca, gasp, cvt ────────────────────────────────────────────────────────────────────────────

/// Whether `loca` uses 32-bit offsets, from `head.indexToLocFormat`.
pub(crate) fn long_loca(font: &SfntFont) -> Result<bool> {
    let head = font.get(*b"head").ok_or_else(|| TablesError::MissingTable("head".into()))?;
    if head.len() < 54 {
        return Err(TablesError::invalid("the 'head' table is cut short"));
    }
    Ok(Rd::at(head, 50).i16()? != 0)
}

/// The glyph offsets of a `loca` table.
pub(crate) fn loca_offsets(data: &[u8], font: &SfntFont) -> Result<Vec<usize>> {
    let long = long_loca(font)?;
    let size = if long { 4 } else { 2 };
    if !data.len().is_multiple_of(size) {
        return Err(TablesError::invalid("the 'loca' table length does not fit its offset size"));
    }
    let mut r = Rd::new(data);
    let mut out = Vec::with_capacity(data.len() / size);
    for _ in 0..data.len() / size {
        out.push(if long { r.u32()? as usize } else { usize::from(r.u16()?).saturating_mul(2) });
    }
    Ok(out)
}

fn loca(data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    loca_offsets(data, font)?;
    w.comment("The 'loca' table will be calculated by the compiler");
    Ok(())
}

fn gasp(data: &[u8], w: &mut XmlWriter) -> Result<()> {
    let mut r = Rd::new(data);
    let version = r.u16()?;
    let n = r.u16()?;
    if version > 1 {
        return Err(TablesError::invalid(format!("unknown 'gasp' format: {version}")));
    }
    let mut ranges = BTreeMap::new();
    for _ in 0..n {
        let ppem = r.u16()?;
        let behavior = r.u16()?;
        ranges.insert(ppem, behavior);
    }
    if !r.rest().is_empty() {
        return Err(TablesError::invalid("too much 'gasp' data"));
    }
    for (ppem, behavior) in ranges {
        w.simple("gaspRange", &[("rangeMaxPPEM", &ppem.to_string()), ("rangeGaspBehavior", &behavior.to_string())]);
    }
    Ok(())
}

fn cvt(data: &[u8], w: &mut XmlWriter) -> Result<()> {
    if !data.len().is_multiple_of(2) {
        return Err(TablesError::invalid("the 'cvt ' table has an odd length"));
    }
    for (i, pair) in data.chunks(2).enumerate() {
        let v = i16::from_be_bytes([pair.first().copied().unwrap_or(0), pair.get(1).copied().unwrap_or(0)]);
        w.simple("cv", &[("index", &i.to_string()), ("value", &v.to_string())]);
    }
    Ok(())
}

// ── hdmx ───────────────────────────────────────────────────────────────────────────────────────

fn hdmx(data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    let count = num_glyphs(font)?;
    let order = w.glyph_order(font)?;
    let mut r = Rd::new(data);
    let _version = r.u16()?;
    let records = r.u16()?;
    let record_size = usize::try_from(r.i32()?).map_err(|_| TablesError::invalid("negative 'hdmx' record size"))?;
    let mut rest = r.rest();
    let mut by_ppem: BTreeMap<u8, &[u8]> = BTreeMap::new();
    for _ in 0..records {
        let ppem = *rest.first().ok_or_else(cut)?;
        rest.get(1).ok_or_else(cut)?;
        let widths = rest.get(2..).unwrap_or_default();
        by_ppem.insert(ppem, widths.get(..count.min(widths.len())).unwrap_or_default());
        rest = rest.get(record_size..).unwrap_or_default();
    }
    if !rest.is_empty() {
        return Err(TablesError::invalid("too much 'hdmx' data"));
    }
    let mut gid_of: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (i, n) in order.iter().enumerate() {
        gid_of.insert(n.as_str(), i);
    }
    let mut names: Vec<&String> = order.iter().collect();
    names.sort();
    let width = names.iter().map(|n| n.chars().count()).max().ok_or_else(|| TablesError::invalid("the font has no glyphs"))?;
    w.open("hdmxData", &[]);
    let mut line = format!("{:>width$}:", "ppem");
    for ppem in by_ppem.keys() {
        line.push_str(&format!("{ppem:>4}"));
    }
    line.push_str(" ;");
    w.text(&line);
    w.blank();
    for name in names {
        let gid = gid_of.get(name.as_str()).copied().unwrap_or(0);
        let shown = name.replace(';', "\\x3b");
        let mut line = format!("{shown:>width$}:");
        for widths in by_ppem.values() {
            let v = widths.get(gid).ok_or_else(|| TablesError::invalid("an 'hdmx' record is cut short"))?;
            line.push_str(&format!("{v:>4}"));
        }
        line.push_str(" ;");
        w.text(&line);
    }
    w.close("hdmxData");
    Ok(())
}
