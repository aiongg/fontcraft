//! TTX reader: the core tables (M12.8).
//!
//! `head hhea maxp OS/2 name post cmap hmtx loca glyf gasp vhea vmtx fpgm prep cvt hdmx`, each
//! compiled from its element as fontTools compiles it, so that a dump `ttx` wrote gives the bytes
//! `ttx` gives. Fields fontTools fills from other tables at compile time are filled by
//! [`super::import`], which knows the whole font. Ported from fontTools' `ttLib.tables` (MIT):
//! `_h_e_a_d`, `_h_h_e_a`, `_v_h_e_a`, `_m_a_x_p`, `O_S_2f_2`, `_n_a_m_e`, `_p_o_s_t`,
//! `_c_m_a_p`, `_h_m_t_x`, `_l_o_c_a`, `_g_l_y_f`, `_g_a_s_p`, `_c_v_t`, `_f_p_g_m`, `_h_d_m_x`
//! and `ttProgram`.
//!
//! Known differences: a `name` record marked `unicode="False"` in a Unicode encoding is read
//! back as the bytes its escaped text spells (fontTools encodes those characters as UTF-16, so
//! its round trip changes the record); a missing `<instructions>` element of a simple glyph and
//! missing `psNames` or `extraNames` of a `post` table read as empty where fontTools stops; and
//! `cmap` format 2 is not read yet.

mod cmap;
mod glyf;
mod program;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

use super::Element;
use super::core::name::{Encoding, MAC_ROMAN_HIGH, encoding};
use super::read::{
    Num, Out, ReadContext, attr, binary2num, float_lit, float_to_fixed, int_lit, int_value, num_lit, py_strip, read_hex, timestamp_from_string,
    xml_to_tag,
};

pub(crate) use glyf::{Glyf, apply_maxp_recalc, compile_glyf, compile_loca, maxp_recalc, recalc_compiled_maxp};

/// The table a TTX table element stands for, as bytes; `None` for an element this module does
/// not handle. `font` gives the context some tables need (glyph order, `maxp`, `head`).
pub fn read_table(element: &Element, font: &SfntFont) -> Result<Option<(Tag, Vec<u8>)>> {
    read_table_with(element, &ReadContext::from_font(font))
}

/// [`read_table`] with the context [`super::import`] builds.
pub(crate) fn read_table_with(element: &Element, ctx: &ReadContext) -> Result<Option<(Tag, Vec<u8>)>> {
    let Ok(tag) = xml_to_tag(&element.name) else { return Ok(None) };
    let data = match &tag {
        b"head" => head(element)?,
        b"hhea" => header(element, &HHEA)?,
        b"vhea" => header(element, &VHEA)?,
        b"maxp" => maxp(element)?,
        b"OS/2" => os2(element)?,
        b"name" => name(element)?,
        b"post" => post(element, ctx)?,
        b"cmap" => cmap::compile(element, ctx)?,
        b"hmtx" | b"vmtx" => {
            let vertical = &tag == b"vmtx";
            let header = if vertical { *b"vhea" } else { *b"hhea" };
            pack_metrics(&compile_metrics(element, ctx, vertical)?, ctx.has(header))?.0
        }
        b"loca" => Vec::new(),
        b"glyf" => compile_glyf(element, ctx)?.data,
        b"gasp" => gasp(element)?,
        b"fpgm" | b"prep" => program_table(element)?,
        b"cvt " => cvt(element)?,
        b"hdmx" => hdmx(element, ctx)?,
        _ => return Ok(None),
    };
    Ok(Some((tag, data)))
}

fn missing(table: &str, field: &str) -> TablesError {
    TablesError::invalid(format!("the '{table}' table has no '{field}'"))
}

/// The children of a table element by name; a later child replaces an earlier one, as each
/// `setattr` in fontTools does.
fn fields(element: &Element) -> HashMap<&str, &Element> {
    element.children.iter().map(|c| (c.name.as_str(), c)).collect()
}

fn field_int(f: &HashMap<&str, &Element>, table: &str, name: &str) -> Result<i64> {
    int_value(f.get(name).ok_or_else(|| missing(table, name))?)
}

fn field_text<'e>(f: &HashMap<&str, &'e Element>, table: &str, name: &str) -> Result<&'e str> {
    attr(f.get(name).ok_or_else(|| missing(table, name))?, "value")
}

/// A 16.16 fixed-point field from its decimal text.
fn fixed_field(f: &HashMap<&str, &Element>, table: &str, name: &str) -> Result<i64> {
    float_to_fixed(float_lit(field_text(f, table, name)?)?, 16)
}

#[derive(Clone, Copy)]
enum Kind {
    U16,
    I16,
    U32,
}

fn put(out: &mut Out, kind: Kind, v: i64, what: &str) -> Result<()> {
    match kind {
        Kind::U16 => out.u16(v, what),
        Kind::I16 => out.i16(v, what),
        Kind::U32 => out.u32(v, what),
    }
}

// ── head ───────────────────────────────────────────────────────────────────────────────────────

fn head(element: &Element) -> Result<Vec<u8>> {
    let f = fields(element);
    let mut out = Out::default();
    out.i32(fixed_field(&f, "head", "tableVersion")?, "head.tableVersion")?;
    out.i32(fixed_field(&f, "head", "fontRevision")?, "head.fontRevision")?;
    out.u32(field_int(&f, "head", "checkSumAdjustment")?, "head.checkSumAdjustment")?;
    out.u32(field_int(&f, "head", "magicNumber")?, "head.magicNumber")?;
    out.u16(binary2num(field_text(&f, "head", "flags")?) as i64, "head.flags")?;
    out.u16(field_int(&f, "head", "unitsPerEm")?, "head.unitsPerEm")?;
    out.u64(timestamp_from_string(field_text(&f, "head", "created")?)?);
    out.u64(timestamp_from_string(field_text(&f, "head", "modified")?)?);
    for name in ["xMin", "yMin", "xMax", "yMax"] {
        out.i16(field_int(&f, "head", name)?, name)?;
    }
    out.u16(binary2num(field_text(&f, "head", "macStyle")?) as i64, "head.macStyle")?;
    out.u16(field_int(&f, "head", "lowestRecPPEM")?, "head.lowestRecPPEM")?;
    for name in ["fontDirectionHint", "indexToLocFormat", "glyphDataFormat"] {
        out.i16(field_int(&f, "head", name)?, name)?;
    }
    Ok(out.0)
}

// ── hhea, vhea ─────────────────────────────────────────────────────────────────────────────────

struct HeaderFormat {
    table: &'static str,
    fields: [(&'static str, Kind); 16],
    /// A legacy name fontTools accepts for a field.
    alias: Option<(&'static str, &'static str)>,
}

const HHEA: HeaderFormat = HeaderFormat {
    table: "hhea",
    fields: [
        ("ascent", Kind::I16),
        ("descent", Kind::I16),
        ("lineGap", Kind::I16),
        ("advanceWidthMax", Kind::U16),
        ("minLeftSideBearing", Kind::I16),
        ("minRightSideBearing", Kind::I16),
        ("xMaxExtent", Kind::I16),
        ("caretSlopeRise", Kind::I16),
        ("caretSlopeRun", Kind::I16),
        ("caretOffset", Kind::I16),
        ("reserved0", Kind::I16),
        ("reserved1", Kind::I16),
        ("reserved2", Kind::I16),
        ("reserved3", Kind::I16),
        ("metricDataFormat", Kind::I16),
        ("numberOfHMetrics", Kind::U16),
    ],
    alias: None,
};

const VHEA: HeaderFormat = HeaderFormat {
    table: "vhea",
    fields: [
        ("ascent", Kind::I16),
        ("descent", Kind::I16),
        ("lineGap", Kind::I16),
        ("advanceHeightMax", Kind::U16),
        ("minTopSideBearing", Kind::I16),
        ("minBottomSideBearing", Kind::I16),
        ("yMaxExtent", Kind::I16),
        ("caretSlopeRise", Kind::I16),
        ("caretSlopeRun", Kind::I16),
        ("caretOffset", Kind::I16),
        ("reserved1", Kind::I16),
        ("reserved2", Kind::I16),
        ("reserved3", Kind::I16),
        ("reserved4", Kind::I16),
        ("metricDataFormat", Kind::I16),
        ("numberOfVMetrics", Kind::U16),
    ],
    alias: Some(("reserved0", "caretOffset")),
};

/// fontTools' `versionToFixed`: text starting with `0` is an integer literal, other text a
/// decimal; values below 0x10000 are taken as decimals and made 16.16.
fn version_to_fixed(text: &str) -> Result<i64> {
    let t = text.trim();
    let value = if t.starts_with('0') { Num::Int(int_lit(t)?) } else { Num::Float(float_lit(t)?) };
    if value.as_f64() < 65536.0 { float_to_fixed(value.as_f64(), 16) } else { value.round() }
}

fn header(element: &Element, format: &HeaderFormat) -> Result<Vec<u8>> {
    let mut f = fields(element);
    if let Some((old, new)) = format.alias
        && let Some(e) = f.get(old).copied()
    {
        f.insert(new, e);
    }
    let mut out = Out::default();
    out.u32(version_to_fixed(field_text(&f, format.table, "tableVersion")?)?, "the table version")?;
    for (name, kind) in format.fields {
        put(&mut out, kind, field_int(&f, format.table, name)?, name)?;
    }
    Ok(out.0)
}

// ── maxp ───────────────────────────────────────────────────────────────────────────────────────

const MAXP_V1: [&str; 13] = [
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

fn maxp(element: &Element) -> Result<Vec<u8>> {
    let f = fields(element);
    let version = field_int(&f, "maxp", "tableVersion")?;
    let version = if version == 0x5000 { version } else { 0x0001_0000 };
    let mut out = Out::default();
    out.u32(version, "maxp.tableVersion")?;
    out.u16(field_int(&f, "maxp", "numGlyphs")?, "maxp.numGlyphs")?;
    if version == 0x0001_0000 {
        for name in MAXP_V1 {
            out.u16(field_int(&f, "maxp", name)?, name)?;
        }
    }
    Ok(out.0)
}

// ── OS/2 ───────────────────────────────────────────────────────────────────────────────────────

const PANOSE: [&str; 10] =
    ["bFamilyType", "bSerifStyle", "bWeight", "bProportion", "bContrast", "bStrokeVariation", "bArmStyle", "bLetterForm", "bMidline", "bXHeight"];

/// One field of the `OS/2` record: its name and how it is packed.
enum Os2Field {
    Int(&'static str, Kind),
    Bits(&'static str, Kind),
    Panose,
    Vendor,
}

const OS2_V0: &[Os2Field] = &[
    Os2Field::Int("version", Kind::U16),
    Os2Field::Int("xAvgCharWidth", Kind::I16),
    Os2Field::Int("usWeightClass", Kind::U16),
    Os2Field::Int("usWidthClass", Kind::U16),
    Os2Field::Bits("fsType", Kind::U16),
    Os2Field::Int("ySubscriptXSize", Kind::I16),
    Os2Field::Int("ySubscriptYSize", Kind::I16),
    Os2Field::Int("ySubscriptXOffset", Kind::I16),
    Os2Field::Int("ySubscriptYOffset", Kind::I16),
    Os2Field::Int("ySuperscriptXSize", Kind::I16),
    Os2Field::Int("ySuperscriptYSize", Kind::I16),
    Os2Field::Int("ySuperscriptXOffset", Kind::I16),
    Os2Field::Int("ySuperscriptYOffset", Kind::I16),
    Os2Field::Int("yStrikeoutSize", Kind::I16),
    Os2Field::Int("yStrikeoutPosition", Kind::I16),
    Os2Field::Int("sFamilyClass", Kind::I16),
    Os2Field::Panose,
    Os2Field::Bits("ulUnicodeRange1", Kind::U32),
    Os2Field::Bits("ulUnicodeRange2", Kind::U32),
    Os2Field::Bits("ulUnicodeRange3", Kind::U32),
    Os2Field::Bits("ulUnicodeRange4", Kind::U32),
    Os2Field::Vendor,
    Os2Field::Bits("fsSelection", Kind::U16),
    Os2Field::Int("usFirstCharIndex", Kind::U16),
    Os2Field::Int("usLastCharIndex", Kind::U16),
    Os2Field::Int("sTypoAscender", Kind::I16),
    Os2Field::Int("sTypoDescender", Kind::I16),
    Os2Field::Int("sTypoLineGap", Kind::I16),
    Os2Field::Int("usWinAscent", Kind::U16),
    Os2Field::Int("usWinDescent", Kind::U16),
];

const OS2_V1: &[Os2Field] = &[Os2Field::Bits("ulCodePageRange1", Kind::U32), Os2Field::Bits("ulCodePageRange2", Kind::U32)];

const OS2_V2: &[Os2Field] = &[
    Os2Field::Int("sxHeight", Kind::I16),
    Os2Field::Int("sCapHeight", Kind::I16),
    Os2Field::Int("usDefaultChar", Kind::U16),
    Os2Field::Int("usBreakChar", Kind::U16),
    Os2Field::Int("usMaxContext", Kind::U16),
];

fn os2(element: &Element) -> Result<Vec<u8>> {
    let mut f = fields(element);
    if let Some(e) = f.get("usMaxContex").copied() {
        f.insert("usMaxContext", e);
    }
    let version = field_int(&f, "OS/2", "version")?;
    let mut list: Vec<&Os2Field> = OS2_V0.iter().collect();
    match version {
        0 => {}
        1 => list.extend(OS2_V1),
        2..=5 => {
            list.extend(OS2_V1);
            list.extend(OS2_V2);
        }
        v => return Err(TablesError::invalid(format!("unknown format for OS/2 table: version {v}"))),
    }
    let mut out = Out::default();
    for field in list {
        match field {
            Os2Field::Int(name, kind) => put(&mut out, *kind, field_int(&f, "OS/2", name)?, name)?,
            Os2Field::Bits(name, kind) => put(&mut out, *kind, binary2num(field_text(&f, "OS/2", name)?) as i64, name)?,
            Os2Field::Panose => {
                let panose = f.get("panose").ok_or_else(|| missing("OS/2", "panose"))?;
                let p = fields(panose);
                for name in PANOSE {
                    out.u8(field_int(&p, "OS/2", name)?, name)?;
                }
            }
            Os2Field::Vendor => out.bytes(&vendor_id(field_text(&f, "OS/2", "achVendID")?)?),
        }
    }
    if version == 5 {
        for name in ["usLowerOpticalPointSize", "usUpperOpticalPointSize"] {
            let v = float_lit(field_text(&f, "OS/2", name)?)?;
            out.u16((v * 20.0).round_ties_even() as i64, name)?;
        }
    }
    Ok(out.0)
}

/// `achVendID`: the text as a Python string literal (escapes processed), as four bytes.
fn vendor_id(text: &str) -> Result<[u8; 4]> {
    let chars = py_unescape(text)?;
    let mut out = [0u8; 4];
    for (slot, c) in out.iter_mut().zip(chars.chars()) {
        *slot = u8::try_from(u32::from(c)).map_err(|_| TablesError::invalid(format!("the vendor id '{text}' does not fit in bytes")))?;
    }
    Ok(out)
}

/// The escapes of a Python string literal: `\\ \' \" \n \r \t \a \b \f \v \xHH \ooo \uXXXX`;
/// any other backslash stays as it is.
pub(crate) fn py_unescape(text: &str) -> Result<String> {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(e) = chars.next() else {
            out.push('\\');
            break;
        };
        let simple = match e {
            '\\' => Some('\\'),
            '\'' => Some('\''),
            '"' => Some('"'),
            'n' => Some('\n'),
            'r' => Some('\r'),
            't' => Some('\t'),
            'a' => Some('\u{7}'),
            'b' => Some('\u{8}'),
            'f' => Some('\u{c}'),
            'v' => Some('\u{b}'),
            '\n' => None,
            _ => None,
        };
        if let Some(s) = simple {
            out.push(s);
            continue;
        }
        let hex_digits = match e {
            'x' => Some(2),
            'u' => Some(4),
            'U' => Some(8),
            _ => None,
        };
        if let Some(n) = hex_digits {
            let digits: String = (0..n).filter_map(|_| chars.next()).collect();
            let code = u32::from_str_radix(&digits, 16).ok().filter(|_| digits.len() == n).and_then(char::from_u32);
            out.push(code.ok_or_else(|| TablesError::invalid(format!("the escape '\\{e}{digits}' is not valid")))?);
            continue;
        }
        if e.is_digit(8) {
            let mut code = e.to_digit(8).unwrap_or(0);
            for _ in 0..2 {
                match chars.peek().and_then(|d| d.to_digit(8)) {
                    Some(d) => {
                        code = code * 8 + d;
                        chars.next();
                    }
                    None => break,
                }
            }
            out.push(char::from_u32(code).unwrap_or('\u{0}'));
            continue;
        }
        if e != '\n' {
            out.push('\\');
            out.push(e);
        }
    }
    Ok(out)
}

// ── name ───────────────────────────────────────────────────────────────────────────────────────

fn encode_with(text: &str, enc: Option<Encoding>) -> Result<Vec<u8>> {
    let refuse = |c: char| TablesError::invalid(format!("the name text holds '{c}', which its encoding cannot hold"));
    match enc {
        Some(Encoding::Utf16) => Ok(text.encode_utf16().flat_map(u16::to_be_bytes).collect()),
        Some(Encoding::Latin1) => latin1(text),
        Some(Encoding::MacRoman) => text
            .chars()
            .map(|c| {
                if (c as u32) < 0x80 {
                    Ok(c as u8)
                } else {
                    MAC_ROMAN_HIGH.chars().position(|m| m == c).map(|i| (i + 0x80) as u8).ok_or_else(|| refuse(c))
                }
            })
            .collect(),
        Some(Encoding::Ascii | Encoding::AsciiCompatible) | None => {
            text.chars().map(|c| if c.is_ascii() { Ok(c as u8) } else { Err(refuse(c)) }).collect()
        }
    }
}

fn latin1(text: &str) -> Result<Vec<u8>> {
    text.chars().map(|c| u8::try_from(u32::from(c)).map_err(|_| TablesError::invalid(format!("the text holds '{c}', which is not a byte")))).collect()
}

fn name(element: &Element) -> Result<Vec<u8>> {
    let mut records: Vec<(u16, u16, u16, u16, Vec<u8>)> = Vec::new();
    for r in element.children.iter().filter(|c| c.name == "namerecord") {
        let id = |n: &str| -> Result<u16> {
            u16::try_from(int_lit(attr(r, n)?)?).map_err(|_| TablesError::invalid(format!("the name record's {n} does not fit 16 bits")))
        };
        let (name_id, platform, enc_id, lang) = (id("nameID")?, id("platformID")?, id("platEncID")?, id("langID")?);
        let text = py_strip(&r.text);
        let enc = encoding(platform, enc_id, lang);
        let compatible = matches!(enc, Some(Encoding::Utf16 | Encoding::Ascii | Encoding::Latin1));
        let unicode = r.attribute("unicode").map(|v| int_lit(v).map(|n| n != 0).unwrap_or(false));
        let bytes = match (compatible, unicode) {
            (true, Some(false)) => latin1(text)?,
            (true, _) | (false, Some(true)) => encode_with(text, enc)?,
            (false, _) => latin1(text)?,
        };
        if records.len() >= usize::from(u16::MAX) {
            return Err(TablesError::TooLarge { what: "the name records", limit: usize::from(u16::MAX) });
        }
        records.push((platform, enc_id, lang, name_id, bytes));
    }
    records.sort_by_key(|r| (r.0, r.1, r.2, r.3));
    let mut out = Out::default();
    let n = records.len() as i64;
    out.u16(0, "name.format")?;
    out.u16(n, "the name record count")?;
    out.u16(6 + n * 12, "the name string offset")?;
    let mut strings: Vec<u8> = Vec::new();
    let mut done: HashMap<Vec<u8>, (usize, usize)> = HashMap::new();
    for (platform, enc_id, lang, name_id, bytes) in &records {
        let (offset, length) = match done.get(bytes) {
            Some(v) => *v,
            None => {
                let v = (strings.len(), bytes.len());
                strings.extend_from_slice(bytes);
                done.insert(bytes.clone(), v);
                v
            }
        };
        for v in [*platform, *enc_id, *lang, *name_id] {
            out.u16(i64::from(v), "a name record field")?;
        }
        out.u16(length as i64, "a name string length")?;
        out.u16(offset as i64, "a name string offset")?;
    }
    out.bytes(&strings);
    Ok(out.0)
}

// ── post ───────────────────────────────────────────────────────────────────────────────────────

fn post(element: &Element, ctx: &ReadContext) -> Result<Vec<u8>> {
    use read_fonts::tables::post::DEFAULT_GLYPH_NAMES;
    let f = fields(element);
    let format = float_to_fixed(float_lit(field_text(&f, "post", "formatType")?)?, 16)?;
    let mut out = Out::default();
    out.i32(format, "post.formatType")?;
    out.i32(fixed_field(&f, "post", "italicAngle")?, "post.italicAngle")?;
    out.i16(field_int(&f, "post", "underlinePosition")?, "post.underlinePosition")?;
    out.i16(field_int(&f, "post", "underlineThickness")?, "post.underlineThickness")?;
    for name in ["isFixedPitch", "minMemType42", "maxMemType42", "minMemType1", "maxMemType1"] {
        out.u32(field_int(&f, "post", name)?, name)?;
    }
    match format {
        0x0001_0000 | 0x0003_0000 => {}
        0x0002_0000 => {
            let order = ctx.order()?;
            let mapping: HashMap<&str, &str> = f
                .get("psNames")
                .map(|p| p.children.iter().filter(|c| c.name == "psName").map(|c| Ok((attr(c, "name")?, attr(c, "psName")?))).collect::<Result<_>>())
                .transpose()?
                .unwrap_or_default();
            let mut extra: Vec<String> = f
                .get("extraNames")
                .map(|p| p.children.iter().filter(|c| c.name == "psName").map(|c| attr(c, "name").map(str::to_string)).collect::<Result<_>>())
                .transpose()?
                .unwrap_or_default();
            let standard: HashMap<&str, usize> = DEFAULT_GLYPH_NAMES.iter().enumerate().map(|(i, n)| (*n, i)).rev().collect();
            extra.retain(|n| !standard.contains_key(n.as_str()));
            let mut extra_ids: HashMap<String, usize> = HashMap::new();
            for (i, n) in extra.iter().enumerate() {
                extra_ids.insert(n.clone(), i);
            }
            out.u16(order.len() as i64, "post.numGlyphs")?;
            for glyph in order {
                let ps = mapping.get(glyph.as_str()).copied().unwrap_or(glyph.as_str());
                let index = if let Some(i) = extra_ids.get(ps) {
                    258 + i
                } else if let Some(i) = standard.get(ps) {
                    *i
                } else {
                    extra_ids.insert(ps.to_string(), extra.len());
                    extra.push(ps.to_string());
                    258 + extra.len() - 1
                };
                out.u16(index as i64, "a post name index")?;
            }
            for n in &extra {
                let bytes = latin1(n)?;
                out.u8(bytes.len() as i64, "a post name length")?;
                out.bytes(&bytes);
            }
        }
        0x0004_0000 => {
            let data = fontcraft_model::glyphdata::GlyphData::new().ok();
            for glyph in ctx.order()? {
                let base = glyph.split('#').next().unwrap_or_default();
                out.u16(i64::from(post4_code(base, data.as_ref())), "a post format 4 code")?;
            }
        }
        _ => return Err(TablesError::Unsupported(format!("'post' table format {}", super::py_float(format as f64 / 65536.0)))),
    }
    Ok(out.0)
}

/// The code `post` format 4 stores for a glyph name: its Adobe Glyph List code point, the
/// value of a `uniXXXX` name, else 0xFFFF.
fn post4_code(name: &str, data: Option<&fontcraft_model::glyphdata::GlyphData>) -> u32 {
    if name.len() == 7
        && let Some(hex) = name.strip_prefix("uni")
        && let Ok(c) = u32::from_str_radix(hex, 16)
    {
        return c;
    }
    let algorithmic = name.strip_prefix('u').is_some_and(|h| (4..=6).contains(&h.len()) && h.bytes().all(|b| b.is_ascii_hexdigit()));
    if algorithmic || name.contains(['_', '.']) {
        return 0xFFFF;
    }
    let text = data.map(|d| d.name_to_text(name)).unwrap_or_default();
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if (c as u32) <= 0xFFFF => c as u32,
        _ => 0xFFFF,
    }
}

// ── hmtx, vmtx ─────────────────────────────────────────────────────────────────────────────────

/// The advance and side bearing of every glyph, in glyph order.
pub(crate) fn compile_metrics(element: &Element, ctx: &ReadContext, vertical: bool) -> Result<Vec<(i64, i64)>> {
    let (advance, bearing) = if vertical { ("height", "tsb") } else { ("width", "lsb") };
    let mut by_name: HashMap<&str, (Num, Num)> = HashMap::new();
    for m in element.children.iter().filter(|c| c.name == "mtx") {
        by_name.insert(attr(m, "name")?, (num_lit(attr(m, advance)?)?, num_lit(attr(m, bearing)?)?));
    }
    ctx.order()?
        .iter()
        .map(|glyph| {
            let (a, b) =
                by_name.get(glyph.as_str()).ok_or_else(|| TablesError::invalid(format!("the metrics table has no entry for the glyph '{glyph}'")))?;
            Ok((a.round()?, b.round()?))
        })
        .collect()
}

/// The table bytes and the number of long metrics: trailing glyphs with the advance of the last
/// one keep only their side bearing when the font has the header table.
pub(crate) fn pack_metrics(metrics: &[(i64, i64)], has_header: bool) -> Result<(Vec<u8>, usize)> {
    let n = metrics.len();
    let mut long = n;
    if has_header {
        let last = metrics.last().map(|m| m.0).ok_or_else(|| TablesError::invalid("the metrics table has no glyphs"))?;
        loop {
            let prev = if long >= 2 { metrics.get(long - 2) } else { metrics.last() };
            if prev.map(|m| m.0) != Some(last) {
                break;
            }
            long -= 1;
            if long <= 1 {
                long = 1;
                break;
            }
        }
    }
    let mut out = Out::default();
    for (i, (advance, bearing)) in metrics.iter().enumerate() {
        if i < long {
            out.u16(*advance, "an advance")?;
        }
        out.i16(*bearing, "a side bearing")?;
    }
    Ok((out.0, long))
}

// ── gasp, cvt, fpgm, prep, hdmx ────────────────────────────────────────────────────────────────

fn gasp(element: &Element) -> Result<Vec<u8>> {
    let mut ranges: BTreeMap<i64, i64> = BTreeMap::new();
    for r in element.children.iter().filter(|c| c.name == "gaspRange") {
        ranges.insert(int_lit(attr(r, "rangeMaxPPEM")?)?, int_lit(attr(r, "rangeGaspBehavior")?)?);
    }
    let version = i64::from(ranges.values().any(|b| b & !3 != 0));
    let mut out = Out::default();
    out.u16(version, "gasp.version")?;
    out.u16(ranges.len() as i64, "the gasp range count")?;
    for (ppem, behavior) in &ranges {
        out.u16(*ppem, "gasp.rangeMaxPPEM")?;
        out.u16(*behavior, "gasp.rangeGaspBehavior")?;
    }
    Ok(out.0)
}

/// The program of an element holding `assembly` or `bytecode`.
pub(crate) fn program_of(element: &Element) -> Result<Option<Vec<u8>>> {
    let mut code = None;
    for child in &element.children {
        code = Some(match child.name.as_str() {
            "assembly" => program::assemble(&child.text)?,
            "bytecode" => read_hex(&child.text)?,
            other => return Err(TablesError::invalid(format!("a TrueType program has the unknown element '{other}'"))),
        });
    }
    Ok(code)
}

fn program_table(element: &Element) -> Result<Vec<u8>> {
    Ok(program_of(element)?.unwrap_or_default())
}

fn cvt(element: &Element) -> Result<Vec<u8>> {
    const MAX_CVT: usize = crate::limits::MAX_SFNT / 2;
    let mut values: Vec<i64> = Vec::new();
    for cv in element.children.iter().filter(|c| c.name == "cv") {
        let index = usize::try_from(int_lit(attr(cv, "index")?)?).map_err(|_| TablesError::invalid("a cvt index is negative"))?;
        if index >= MAX_CVT {
            return Err(TablesError::TooLarge { what: "the cvt index", limit: MAX_CVT });
        }
        if values.len() <= index {
            values.resize(index + 1, 0);
        }
        if let Some(slot) = values.get_mut(index) {
            *slot = int_value(cv)?;
        }
    }
    let mut out = Out::default();
    for v in values {
        out.i16(v, "a cvt value")?;
    }
    Ok(out.0)
}

fn hdmx(element: &Element, ctx: &ReadContext) -> Result<Vec<u8>> {
    let data = element.child("hdmxData").ok_or_else(|| missing("hdmx", "hdmxData"))?;
    let mut lines = data.text.split(';');
    let top: Vec<&str> = lines.next().unwrap_or_default().split_whitespace().collect();
    if top.first() != Some(&"ppem:") {
        return Err(TablesError::invalid("illegal hdmx format"));
    }
    let ppems: Vec<i64> = top.iter().skip(1).map(|p| int_lit(p)).collect::<Result<_>>()?;
    let mut widths: BTreeMap<i64, HashMap<String, i64>> = ppems.iter().map(|p| (*p, HashMap::new())).collect();
    for line in lines {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let Some((first, values)) = parts.split_first() else { continue };
        let glyph = first.strip_suffix(':').ok_or_else(|| TablesError::invalid("illegal hdmx format"))?;
        let glyph = if glyph.contains('\\') { py_unescape(glyph)? } else { glyph.to_string() };
        if values.len() != ppems.len() {
            return Err(TablesError::invalid("illegal hdmx format"));
        }
        for (ppem, v) in ppems.iter().zip(values) {
            if let Some(m) = widths.get_mut(ppem) {
                m.insert(glyph.clone(), int_lit(v)?);
            }
        }
    }
    let order = ctx.order()?;
    let n = order.len();
    let record_size = 4 * (2 + n).div_ceil(4);
    let mut out = Out::default();
    out.u16(0, "hdmx.version")?;
    out.i16(widths.len() as i64, "the hdmx record count")?;
    out.i32(record_size as i64, "the hdmx record size")?;
    for (ppem, w) in &widths {
        out.u8(*ppem, "an hdmx ppem")?;
        let row: Vec<i64> = order
            .iter()
            .map(|g| w.get(g).copied().ok_or_else(|| TablesError::invalid(format!("the hdmx table has no width for the glyph '{g}'"))))
            .collect::<Result<_>>()?;
        out.u8(row.iter().copied().max().unwrap_or(0), "an hdmx width")?;
        for v in row {
            out.u8(v, "an hdmx width")?;
        }
        out.bytes(&vec![0u8; record_size.saturating_sub(2 + n)]);
    }
    Ok(out.0)
}
