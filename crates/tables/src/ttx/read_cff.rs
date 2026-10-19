//! TTX reader: `CFF `, `CFF2` and `VORG` (M12.8).
//!
//! The `CFF` element is read into its Top DICT, Private DICTs, font dicts, subroutines and
//! charstrings, and compiled as fontTools' `cffLib` compiles it: DICT keys in the specification's
//! order with values equal to their defaults left out, the charset from the glyph order, strings
//! numbered in the order the compiler meets them (charset names, then Top DICT strings, then font
//! dict strings), and the sections laid out again until every offset is stable. A CFF (not CFF2)
//! Top DICT gets its `FontBBox` from the charstrings when bounds are recalculated. Ported from
//! fontTools' `cffLib` and `misc.psCharStrings` (MIT), and `ttLib.tables.V_O_R_G_`.
//!
//! As in fontTools, a custom encoding keeps one code per glyph (the last one), and a CFF
//! charstring with operands after its last operator is an error. Charstring bounds are taken
//! from read-fonts' charstring evaluator.

use std::collections::{BTreeMap, HashMap};

use read_fonts::ps::string::STANDARD_STRINGS;
use read_fonts::types::Fixed;

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

use super::Element;
use super::read::{Num, Out, ReadContext, attr, binary2num, float_to_fixed, int_lit, int_value, num_lit, read_hex, xml_to_tag};

#[cfg(test)]
mod tests;

/// The table a TTX table element stands for, as bytes; `None` for an element this module does
/// not handle. `font` gives the context some tables need (glyph order, `maxp`, `head`).
pub fn read_table(element: &Element, font: &SfntFont) -> Result<Option<(Tag, Vec<u8>)>> {
    read_table_with(element, &ReadContext::from_font(font))
}

/// [`read_table`] with the context [`super::import`] builds.
pub(crate) fn read_table_with(element: &Element, ctx: &ReadContext) -> Result<Option<(Tag, Vec<u8>)>> {
    let Ok(tag) = xml_to_tag(&element.name) else { return Ok(None) };
    let data = match &tag {
        b"CFF " => compile_cff(element, ctx, false)?,
        b"CFF2" => compile_cff(element, ctx, true)?,
        b"VORG" => vorg(element, ctx)?,
        _ => return Ok(None),
    };
    Ok(Some((tag, data)))
}

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(format!("the CFF data could not be compiled: {}", what.into()))
}

// ── Numbers ────────────────────────────────────────────────────────────────────────────────────

/// fontTools' `encodeIntCFF` (`t2` false) and `encodeIntT2` (`t2` true).
fn encode_int(v: i64, t2: bool, out: &mut Vec<u8>) -> Result<()> {
    if (-107..=107).contains(&v) {
        out.push((v + 139) as u8);
    } else if (108..=1131).contains(&v) {
        let w = v - 108;
        out.extend_from_slice(&[((w >> 8) + 247) as u8, (w & 0xFF) as u8]);
    } else if (-1131..=-108).contains(&v) {
        let w = -v - 108;
        out.extend_from_slice(&[((w >> 8) + 251) as u8, (w & 0xFF) as u8]);
    } else if (-32768..=32767).contains(&v) {
        out.push(28);
        out.extend_from_slice(&(v as i16).to_be_bytes());
    } else {
        let v = i32::try_from(v).map_err(|_| bad(format!("the number {v} does not fit 32 bits")))?;
        out.push(if t2 { 255 } else { 29 });
        out.extend_from_slice(&v.to_be_bytes());
    }
    Ok(())
}

/// Python's `"%.8G" % f`.
fn format_g8(f: f64) -> String {
    let sci = format!("{f:.7e}");
    let Some((mantissa, exp)) = sci.split_once('e') else { return sci };
    let exp: i32 = exp.parse().unwrap_or(0);
    let strip = |s: &str| -> String { if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s.to_string() } };
    if !(-4..8).contains(&exp) {
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{}E{sign}{:02}", strip(mantissa), exp.abs())
    } else {
        let decimals = usize::try_from(7 - exp).unwrap_or(0);
        strip(&format!("{f:.decimals$}"))
    }
}

/// fontTools' `encodeFloat`: a DICT real number.
fn encode_float(f: f64, out: &mut Vec<u8>) -> Result<()> {
    if !f.is_finite() {
        return Err(bad("a real number is not finite"));
    }
    if f == 0.0 {
        out.extend_from_slice(&[30, 0x0F]);
        return Ok(());
    }
    let mut s = format_g8(f);
    if let Some(rest) = s.strip_prefix("0.") {
        s = format!(".{rest}");
    } else if let Some(rest) = s.strip_prefix("-0.") {
        s = format!("-.{rest}");
    } else if s.ends_with("000") {
        let significant = s.trim_end_matches('0').to_string();
        s = format!("{significant}E{}", s.len() - significant.len());
    } else if let (Some(dot), Some(e)) = (s.find('.'), s.find('E')) {
        let integer = s.get(..dot).unwrap_or_default().to_string();
        let fraction = s.get(dot + 1..e).unwrap_or_default().to_string();
        let exponent: i64 = s.get(e + 1..).unwrap_or_default().parse().unwrap_or(0);
        let new_exponent = exponent - fraction.len() as i64;
        s = if new_exponent == 1 { format!("{integer}{fraction}0") } else { format!("{integer}{fraction}E{new_exponent}") };
    }
    if s.starts_with(".0") || s.starts_with("-.0") {
        let (sign, rest) = s.split_once('.').unwrap_or(("", ""));
        let digits = rest.trim_start_matches('0');
        s = format!("{sign}{digits}E-{}", rest.len());
    }
    let mut nibbles: Vec<u8> = Vec::new();
    let mut rest = s.as_str();
    while let Some(c) = rest.chars().next() {
        rest = rest.get(c.len_utf8()..).unwrap_or_default();
        let nibble = match c {
            '0'..='9' => c as u8 - b'0',
            '.' => 0xA,
            '-' => 0xE,
            'E' => {
                let n = if let Some(r) = rest.strip_prefix('-') {
                    rest = r;
                    0xC
                } else {
                    rest = rest.strip_prefix('+').unwrap_or(rest);
                    0xB
                };
                rest = rest.strip_prefix('0').unwrap_or(rest);
                n
            }
            _ => return Err(bad(format!("the real number {f} could not be encoded"))),
        };
        nibbles.push(nibble);
    }
    nibbles.push(0xF);
    if nibbles.len() % 2 == 1 {
        nibbles.push(0xF);
    }
    out.push(30);
    out.extend(nibbles.chunks(2).map(|p| (p.first().copied().unwrap_or(0xF) << 4) | p.get(1).copied().unwrap_or(0xF)));
    Ok(())
}

fn encode_number(n: Num, out: &mut Vec<u8>) -> Result<()> {
    match n {
        Num::Int(v) => encode_int(v, false, out),
        Num::Float(f) => encode_float(f, out),
    }
}

// ── Charstrings ────────────────────────────────────────────────────────────────────────────────

const T2_OPERATORS: &[(&str, &[u8])] = &[
    ("hstem", &[1]),
    ("vstem", &[3]),
    ("vmoveto", &[4]),
    ("rlineto", &[5]),
    ("hlineto", &[6]),
    ("vlineto", &[7]),
    ("rrcurveto", &[8]),
    ("callsubr", &[10]),
    ("return", &[11]),
    ("endchar", &[14]),
    ("vsindex", &[15]),
    ("blend", &[16]),
    ("hstemhm", &[18]),
    ("hintmask", &[19]),
    ("cntrmask", &[20]),
    ("rmoveto", &[21]),
    ("hmoveto", &[22]),
    ("vstemhm", &[23]),
    ("rcurveline", &[24]),
    ("rlinecurve", &[25]),
    ("vvcurveto", &[26]),
    ("hhcurveto", &[27]),
    ("callgsubr", &[29]),
    ("vhcurveto", &[30]),
    ("hvcurveto", &[31]),
    ("ignore", &[12, 0]),
    ("and", &[12, 3]),
    ("or", &[12, 4]),
    ("not", &[12, 5]),
    ("store", &[12, 8]),
    ("abs", &[12, 9]),
    ("add", &[12, 10]),
    ("sub", &[12, 11]),
    ("div", &[12, 12]),
    ("load", &[12, 13]),
    ("neg", &[12, 14]),
    ("eq", &[12, 15]),
    ("drop", &[12, 18]),
    ("put", &[12, 20]),
    ("get", &[12, 21]),
    ("ifelse", &[12, 22]),
    ("random", &[12, 23]),
    ("mul", &[12, 24]),
    ("sqrt", &[12, 26]),
    ("dup", &[12, 27]),
    ("exch", &[12, 28]),
    ("index", &[12, 29]),
    ("roll", &[12, 30]),
    ("hflex", &[12, 34]),
    ("flex", &[12, 35]),
    ("hflex1", &[12, 36]),
    ("flex1", &[12, 37]),
];

enum Token {
    Int(i64),
    Float(f64),
    Op(String),
    Mask(Vec<u8>),
}

/// A charstring: its program from the text, or its bytes when it was written raw.
enum Charstring {
    Program(Vec<Token>),
    Raw(Vec<u8>),
}

fn charstring_from(e: &Element) -> Result<Charstring> {
    if e.attribute("raw").is_some_and(|v| !v.is_empty()) {
        return Ok(Charstring::Raw(read_hex(&e.text)?));
    }
    let mut program = Vec::new();
    let mut words = e.text.split_whitespace();
    while let Some(word) = words.next() {
        if let Ok(v) = word.parse::<i64>() {
            program.push(Token::Int(v));
        } else if let Some(f) = word.parse::<f64>().ok().filter(|f| f.is_finite()) {
            program.push(Token::Float(float_to_fixed(f, 16)? as f64 / 65536.0));
        } else {
            program.push(Token::Op(word.to_string()));
            if word == "hintmask" || word == "cntrmask" {
                let mask = words.next().ok_or_else(|| bad(format!("{word} without a mask")))?;
                let bytes: Vec<u8> = mask.as_bytes().chunks(8).map(|c| binary2num(std::str::from_utf8(c).unwrap_or("")) as u8).collect();
                program.push(Token::Mask(bytes));
            }
        }
    }
    Ok(Charstring::Program(program))
}

/// fontTools' `T2CharString.compile`.
fn compile_charstring(cs: &Charstring, cff2: bool) -> Result<Vec<u8>> {
    let program = match cs {
        Charstring::Raw(bytes) => return Ok(bytes.clone()),
        Charstring::Program(p) => p,
    };
    let mut tokens: &[Token] = program;
    if cff2 {
        if let Some((Token::Op(last), rest)) = tokens.split_last()
            && (last == "return" || last == "endchar")
        {
            tokens = rest;
        }
    } else if tokens.last().is_some_and(|t| !matches!(t, Token::Op(_) | Token::Mask(_))) {
        return Err(bad("T2CharString or Subr has items on the stack after last operator."));
    }
    let mut out = Vec::new();
    for t in tokens {
        match t {
            Token::Int(v) => encode_int(*v, true, &mut out)?,
            Token::Float(f) => {
                let value = float_to_fixed(*f, 16)?;
                if value & 0xFFFF == 0 {
                    encode_int(value >> 16, true, &mut out)?;
                } else {
                    out.push(255);
                    out.extend_from_slice(&(i32::try_from(value).map_err(|_| bad("a fixed-point number is too large"))?).to_be_bytes());
                }
            }
            Token::Op(name) => {
                let code = T2_OPERATORS.iter().find(|(n, _)| n == name).map(|(_, c)| *c).ok_or_else(|| bad(format!("illegal operator: {name}")))?;
                out.extend_from_slice(code);
            }
            Token::Mask(bytes) => out.extend_from_slice(bytes),
        }
        if out.len() > crate::limits::MAX_SFNT {
            return Err(TablesError::TooLarge { what: "a charstring", limit: crate::limits::MAX_SFNT });
        }
    }
    Ok(out)
}

fn subrs_from(e: &Element) -> Result<Vec<Charstring>> {
    e.children.iter().filter(|c| c.name == "CharString").map(charstring_from).collect()
}

// ── DICTs ──────────────────────────────────────────────────────────────────────────────────────

/// A DICT operand: a number, or a blend list (default value then one delta per region).
#[derive(Clone, Debug, PartialEq)]
enum V {
    N(Num),
    B(Vec<Num>),
}

#[derive(Clone, Debug, PartialEq)]
enum Value {
    Num(V),
    List(Vec<V>),
    Sid(String),
    Ros(String, String, Num),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Arg {
    Number,
    Array,
    Delta,
    Sid,
    Ros,
    /// Keys whose values are sections of their own (Private, CharStrings…).
    Special,
}

/// fontTools' `topDictOperators`, in its order: operator bytes, name, kind.
const TOP_OPS: &[(&[u8], &str, Arg)] = &[
    (&[25], "maxstack", Arg::Number),
    (&[12, 30], "ROS", Arg::Ros),
    (&[12, 20], "SyntheticBase", Arg::Number),
    (&[0], "version", Arg::Sid),
    (&[1], "Notice", Arg::Sid),
    (&[12, 0], "Copyright", Arg::Sid),
    (&[2], "FullName", Arg::Sid),
    (&[12, 38], "FontName", Arg::Sid),
    (&[3], "FamilyName", Arg::Sid),
    (&[4], "Weight", Arg::Sid),
    (&[12, 1], "isFixedPitch", Arg::Number),
    (&[12, 2], "ItalicAngle", Arg::Number),
    (&[12, 3], "UnderlinePosition", Arg::Number),
    (&[12, 4], "UnderlineThickness", Arg::Number),
    (&[12, 5], "PaintType", Arg::Number),
    (&[12, 6], "CharstringType", Arg::Number),
    (&[12, 7], "FontMatrix", Arg::Array),
    (&[13], "UniqueID", Arg::Number),
    (&[5], "FontBBox", Arg::Array),
    (&[12, 8], "StrokeWidth", Arg::Number),
    (&[14], "XUID", Arg::Array),
    (&[12, 21], "PostScript", Arg::Sid),
    (&[12, 22], "BaseFontName", Arg::Sid),
    (&[12, 23], "BaseFontBlend", Arg::Delta),
    (&[12, 31], "CIDFontVersion", Arg::Number),
    (&[12, 32], "CIDFontRevision", Arg::Number),
    (&[12, 33], "CIDFontType", Arg::Number),
    (&[12, 34], "CIDCount", Arg::Number),
    (&[15], "charset", Arg::Special),
    (&[12, 35], "UIDBase", Arg::Number),
    (&[16], "Encoding", Arg::Special),
    (&[18], "Private", Arg::Special),
    (&[12, 37], "FDSelect", Arg::Special),
    (&[12, 36], "FDArray", Arg::Special),
    (&[17], "CharStrings", Arg::Special),
    (&[24], "VarStore", Arg::Special),
];

const TOP_ORDER_CFF2: &[&str] = &["maxstack", "FontMatrix", "FDSelect", "FDArray", "CharStrings", "VarStore"];
const FD_ORDER: &[&str] = &["FontName", "FontMatrix", "Weight", "Private"];

/// fontTools' `privateDictOperators` (`blend` is only an XML form, never written).
const PRIVATE_OPS: &[(&[u8], &str, Arg)] = &[
    (&[22], "vsindex", Arg::Number),
    (&[6], "BlueValues", Arg::Delta),
    (&[7], "OtherBlues", Arg::Delta),
    (&[8], "FamilyBlues", Arg::Delta),
    (&[9], "FamilyOtherBlues", Arg::Delta),
    (&[12, 9], "BlueScale", Arg::Number),
    (&[12, 10], "BlueShift", Arg::Number),
    (&[12, 11], "BlueFuzz", Arg::Number),
    (&[10], "StdHW", Arg::Number),
    (&[11], "StdVW", Arg::Number),
    (&[12, 12], "StemSnapH", Arg::Delta),
    (&[12, 13], "StemSnapV", Arg::Delta),
    (&[12, 14], "ForceBold", Arg::Number),
    (&[12, 15], "ForceBoldThreshold", Arg::Number),
    (&[12, 16], "lenIV", Arg::Number),
    (&[12, 17], "LanguageGroup", Arg::Number),
    (&[12, 18], "ExpansionFactor", Arg::Number),
    (&[12, 19], "initialRandomSeed", Arg::Number),
    (&[20], "defaultWidthX", Arg::Number),
    (&[21], "nominalWidthX", Arg::Number),
    (&[19], "Subrs", Arg::Special),
];

fn int_v(v: i64) -> Value {
    Value::Num(V::N(Num::Int(v)))
}

fn real_v(v: f64) -> Value {
    Value::Num(V::N(Num::Float(v)))
}

/// The defaults fontTools leaves out when a value equals them (Python's `==`: 0 equals 0.0).
fn top_default(name: &str) -> Option<Value> {
    Some(match name {
        "isFixedPitch" | "ItalicAngle" | "PaintType" | "StrokeWidth" | "CIDFontVersion" | "CIDFontRevision" | "CIDFontType" => int_v(0),
        "UnderlinePosition" => int_v(-100),
        "UnderlineThickness" => int_v(50),
        "CharstringType" => int_v(2),
        "CIDCount" => int_v(8720),
        "FontMatrix" => Value::List([0.001, 0.0, 0.0, 0.001, 0.0, 0.0].iter().map(|v| V::N(Num::Float(*v))).collect()),
        "FontBBox" => Value::List(vec![V::N(Num::Int(0)); 4]),
        _ => return None,
    })
}

fn private_default(name: &str) -> Option<Value> {
    Some(match name {
        "BlueScale" => real_v(0.039625),
        "BlueShift" => int_v(7),
        "BlueFuzz" => int_v(1),
        "ForceBold" | "LanguageGroup" | "initialRandomSeed" | "defaultWidthX" | "nominalWidthX" => int_v(0),
        "ExpansionFactor" => real_v(0.06),
        _ => return None,
    })
}

fn num_eq(a: &Num, b: &Num) -> bool {
    a.as_f64() == b.as_f64()
}

fn v_eq(a: &V, b: &V) -> bool {
    match (a, b) {
        (V::N(x), V::N(y)) => num_eq(x, y),
        (V::B(x), V::B(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| num_eq(p, q)),
        _ => false,
    }
}

fn value_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => v_eq(x, y),
        (Value::List(x), Value::List(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| v_eq(p, q)),
        _ => false,
    }
}

/// fontTools' `parseBlendList`: one list per `blend` child.
fn blend_lists(e: &Element) -> Result<Vec<Vec<Num>>> {
    e.children.iter().filter(|c| c.name == "blend").map(|c| attr(c, "value")?.split_whitespace().map(num_lit).collect::<Result<Vec<_>>>()).collect()
}

fn read_value(e: &Element, arg: Arg) -> Result<Value> {
    Ok(match arg {
        Arg::Number => match e.attribute("value") {
            Some(v) => Value::Num(V::N(num_lit(v)?)),
            None => {
                let mut lists = blend_lists(e)?;
                if lists.len() > 1 {
                    return Err(bad(format!("'{}' holds more than one blend", e.name)));
                }
                Value::Num(V::B(lists.pop().unwrap_or_default()))
            }
        },
        Arg::Array | Arg::Delta => match e.attribute("value") {
            Some(v) => Value::List(v.split_whitespace().map(|n| num_lit(n).map(V::N)).collect::<Result<_>>()?),
            None => {
                let mut lists = blend_lists(e)?;
                if lists.len() == 1 {
                    // fontTools flattens a single blend list into plain numbers.
                    Value::List(lists.pop().unwrap_or_default().into_iter().map(V::N).collect())
                } else {
                    Value::List(lists.into_iter().map(V::B).collect())
                }
            }
        },
        Arg::Sid => Value::Sid(attr(e, "value")?.to_string()),
        Arg::Ros => Value::Ros(attr(e, "Registry")?.to_string(), attr(e, "Order")?.to_string(), num_lit(attr(e, "Supplement")?)?),
        Arg::Special => return Err(bad(format!("'{}' is not a plain value", e.name))),
    })
}

#[derive(Default)]
struct Private {
    values: BTreeMap<&'static str, Value>,
    subrs: Option<Vec<Charstring>>,
}

fn private_from(e: &Element) -> Result<Private> {
    let mut p = Private::default();
    for child in &e.children {
        if child.name == "Subrs" {
            p.subrs = Some(subrs_from(child)?);
            continue;
        }
        if child.name == "blend" {
            continue;
        }
        let &(_, name, arg) =
            PRIVATE_OPS.iter().find(|(_, n, _)| *n == child.name).ok_or_else(|| bad(format!("unknown Private DICT key '{}'", child.name)))?;
        p.values.insert(name, read_value(child, arg)?);
    }
    Ok(p)
}

#[derive(Default)]
struct FontDict {
    values: BTreeMap<&'static str, Value>,
    private: Option<Private>,
}

enum EncodingValue {
    Standard,
    Expert,
    Custom(Vec<String>),
}

#[derive(Default)]
struct TopDict {
    name: String,
    values: BTreeMap<&'static str, Value>,
    encoding: Option<EncodingValue>,
    private: Option<Private>,
    fd_array: Option<Vec<FontDict>>,
    fd_select_format: Option<i64>,
    charstrings: HashMap<String, (Charstring, i64)>,
    has_charstrings: bool,
    var_store: Option<Vec<u8>>,
}

fn top_key(name: &str) -> Option<(&'static [u8], &'static str, Arg)> {
    TOP_OPS.iter().find(|(_, n, _)| *n == name).copied()
}

fn font_dict_from(e: &Element) -> Result<FontDict> {
    let mut fd = FontDict::default();
    for child in &e.children {
        if child.name == "Private" {
            fd.private = Some(private_from(child)?);
            continue;
        }
        let (_, name, arg) = top_key(&child.name).ok_or_else(|| bad(format!("unknown font dict key '{}'", child.name)))?;
        if arg != Arg::Special {
            fd.values.insert(name, read_value(child, arg)?);
        }
    }
    Ok(fd)
}

fn top_from(e: &Element) -> Result<TopDict> {
    let mut top = TopDict { name: attr(e, "name")?.to_string(), ..TopDict::default() };
    for child in &e.children {
        match child.name.as_str() {
            "charset" => {}
            "Encoding" => {
                top.encoding = Some(match child.attribute("name") {
                    Some("StandardEncoding") => EncodingValue::Standard,
                    Some("ExpertEncoding") => EncodingValue::Expert,
                    Some(other) => return Err(bad(format!("unknown encoding '{other}'"))),
                    None => {
                        let mut codes = vec![".notdef".to_string(); 256];
                        for m in child.children.iter().filter(|m| m.name == "map") {
                            let code = usize::try_from(int_lit(attr(m, "code")?)?)
                                .ok()
                                .filter(|c| *c < 256)
                                .ok_or_else(|| bad("an encoding code is past 255"))?;
                            if let Some(slot) = codes.get_mut(code) {
                                *slot = attr(m, "name")?.to_string();
                            }
                        }
                        EncodingValue::Custom(codes)
                    }
                });
            }
            "Private" => top.private = Some(private_from(child)?),
            "FDArray" => top.fd_array = Some(child.children.iter().filter(|c| c.name == "FontDict").map(font_dict_from).collect::<Result<_>>()?),
            "FDSelect" => top.fd_select_format = Some(int_lit(attr(child, "format")?)?),
            "CharStrings" => {
                top.has_charstrings = true;
                for cs in child.children.iter().filter(|c| c.name == "CharString") {
                    let fd = match cs.attribute("fdSelectIndex") {
                        Some(v) => int_lit(v)?,
                        None => 0,
                    };
                    top.charstrings.insert(attr(cs, "name")?.to_string(), (charstring_from(cs)?, fd));
                }
            }
            "VarStore" => top.var_store = Some(compile_var_store(child)?),
            other => {
                let (_, name, arg) = top_key(other).ok_or_else(|| bad(format!("unknown Top DICT key '{other}'")))?;
                if arg != Arg::Special {
                    top.values.insert(name, read_value(child, arg)?);
                }
            }
        }
    }
    Ok(top)
}

/// fontTools' `IndexedStrings`: standard strings by their SID, others numbered from 391 in the
/// order they are asked for.
#[derive(Default)]
struct Strings {
    custom: Vec<String>,
    ids: HashMap<String, usize>,
}

impl Strings {
    fn sid(&mut self, s: &str) -> usize {
        if let Some(i) = STANDARD_STRINGS.iter().position(|t| *t == s) {
            return i;
        }
        if let Some(i) = self.ids.get(s) {
            return *i;
        }
        let id = STANDARD_STRINGS.len() + self.custom.len();
        self.custom.push(s.to_string());
        self.ids.insert(s.to_string(), id);
        id
    }
}

/// Encodes one DICT operand list as fontTools' `arg_*` handlers do.
fn encode_arg(value: &Value, arg: Arg, strings: &mut Option<&mut Strings>, max_blend: usize, out: &mut Vec<u8>) -> Result<()> {
    match (value, arg) {
        (Value::Num(v), _) => encode_v(v, out),
        (Value::List(list), Arg::Array) => list.iter().try_for_each(|v| encode_v(v, out)),
        (Value::List(list), _) => match list.first() {
            None => Ok(()),
            Some(V::B(_)) => {
                let lists: Vec<Vec<Num>> = list
                    .iter()
                    .map(|v| if let V::B(b) = v { Ok(b.clone()) } else { Err(bad("a delta list mixes blends and numbers")) })
                    .collect::<Result<_>>()?;
                delta_blend(&lists, max_blend, out)
            }
            Some(V::N(_)) => {
                let mut last = Num::Int(0);
                for v in list {
                    let V::N(n) = v else { return Err(bad("a delta list mixes blends and numbers")) };
                    let d = match (n, last) {
                        (Num::Int(a), Num::Int(b)) => Num::Int(a.checked_sub(b).ok_or_else(|| bad("a delta is too large"))?),
                        (a, b) => Num::Float(a.as_f64() - b.as_f64()),
                    };
                    encode_number(d, out)?;
                    last = *n;
                }
                Ok(())
            }
        },
        (Value::Sid(s), _) => {
            let strings = strings.as_mut().ok_or_else(|| bad("a CFF2 DICT cannot hold strings"))?;
            encode_int(strings.sid(s) as i64, false, out)
        }
        (Value::Ros(r, o, s), _) => {
            let strings = strings.as_mut().ok_or_else(|| bad("a CFF2 DICT cannot hold strings"))?;
            encode_int(strings.sid(r) as i64, false, out)?;
            encode_int(strings.sid(o) as i64, false, out)?;
            encode_number(*s, out)
        }
    }
}

fn encode_v(v: &V, out: &mut Vec<u8>) -> Result<()> {
    match v {
        V::N(n) => encode_number(*n, out),
        V::B(list) => {
            for n in list {
                encode_number(*n, out)?;
            }
            encode_int(1, false, out)?;
            out.push(23);
            Ok(())
        }
    }
}

/// fontTools' `arg_delta_blend`, split into runs that fit `max_blend` stack slots.
fn delta_blend(lists: &[Vec<Num>], max_blend: usize, out: &mut Vec<u8>) -> Result<()> {
    let masters = lists.first().map_or(1, Vec::len).max(1);
    let n = lists.len();
    if n * masters + 1 > max_blend {
        let per_run = max_blend.saturating_sub(1) / masters;
        if per_run == 0 {
            return Ok(());
        }
        for chunk in lists.chunks(per_run) {
            delta_blend(chunk, max_blend, out)?;
        }
        return Ok(());
    }
    let mut firsts = Vec::with_capacity(n);
    let mut prev = Num::Int(0);
    for l in lists {
        let v = l.first().copied().unwrap_or(Num::Int(0));
        firsts.push(match (v, prev) {
            (Num::Int(a), Num::Int(b)) => Num::Int(a.saturating_sub(b)),
            (a, b) => Num::Float(a.as_f64() - b.as_f64()),
        });
        prev = v;
    }
    for f in firsts {
        encode_number(f, out)?;
    }
    for l in lists {
        for d in l.iter().skip(1) {
            encode_number(*d, out)?;
        }
    }
    encode_int(n as i64, false, out)?;
    out.push(23);
    Ok(())
}

/// The bytes of a DICT: `values` in `order`, defaults left out, `specials` (offsets) given.
fn compile_dict(
    values: &BTreeMap<&'static str, Value>,
    ops: &[(&[u8], &str, Arg)],
    order: &[&str],
    default: fn(&str) -> Option<Value>,
    specials: &[(&str, Vec<i64>)],
    strings: &mut Option<&mut Strings>,
    max_blend: usize,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for name in order {
        let Some((op, _, arg)) = ops.iter().find(|(_, n, _)| n == name) else { continue };
        if *arg == Arg::Special {
            if let Some((_, args)) = specials.iter().find(|(n, _)| n == name) {
                for a in args {
                    encode_int(*a, false, &mut out)?;
                }
                out.extend_from_slice(op);
            }
            continue;
        }
        let Some(value) = values.get(name) else { continue };
        if default(name).is_some_and(|d| value_eq(value, &d)) {
            continue;
        }
        encode_arg(value, *arg, strings, max_blend, &mut out)?;
        out.extend_from_slice(op);
    }
    Ok(out)
}

fn no_default(_: &str) -> Option<Value> {
    None
}

// ── INDEX ──────────────────────────────────────────────────────────────────────────────────────

fn off_size(largest: usize) -> usize {
    match largest {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        0x1_0000..=0xFF_FFFF => 3,
        _ => 4,
    }
}

fn index_bytes(items: &[Vec<u8>], cff2: bool) -> Result<Vec<u8>> {
    let mut out = Out::default();
    if cff2 {
        out.u32(items.len() as i64, "an INDEX count")?;
    } else {
        out.u16(items.len() as i64, "an INDEX count")?;
    }
    if items.is_empty() {
        return Ok(out.0);
    }
    let total: usize = items.iter().map(Vec::len).sum();
    let size = off_size(total + 1);
    out.0.push(size as u8);
    let mut offset = 1usize;
    let write = |o: &mut Out, v: usize| {
        let bytes = (v as u32).to_be_bytes();
        o.bytes(bytes.get(4 - size..).unwrap_or_default());
    };
    write(&mut out, offset);
    for item in items {
        offset += item.len();
        write(&mut out, offset);
    }
    for item in items {
        out.bytes(item);
    }
    Ok(out.0)
}

// ── Charset, encoding, FDSelect ────────────────────────────────────────────────────────────────

/// The predefined charset (0 ISOAdobe, 1 Expert, 2 ExpertSubset) the glyph order matches.
fn standard_charset(charset: &[String]) -> Option<i64> {
    // ISOAdobe is SIDs 0 to 228 in order; the expert charsets are never matched by fontTools
    // for fonts with a glyph past their lists, and are left to the custom charset here.
    if charset.len() <= 229 && charset.iter().enumerate().all(|(i, n)| STANDARD_STRINGS.get(i).is_some_and(|s| *s == n.as_str())) {
        return Some(0);
    }
    None
}

fn charset_bytes(charset: &[String], cid: bool, strings: &mut Strings) -> Result<Vec<u8>> {
    let id = |name: &str, strings: &mut Strings| -> Result<i64> {
        if cid {
            name.get(3..).and_then(|d| d.parse::<i64>().ok()).ok_or_else(|| bad(format!("the CID glyph name '{name}' has no number")))
        } else {
            Ok(strings.sid(name) as i64)
        }
    };
    let ids: Vec<i64> = charset.iter().skip(1).map(|n| id(n, strings)).collect::<Result<_>>()?;
    let mut f0 = Out::default();
    f0.u8(0, "format")?;
    for v in &ids {
        f0.u16(*v, "a charset id")?;
    }
    let mut ranges: Vec<(i64, i64)> = Vec::new();
    let mut format = 1;
    let mut first: Option<i64> = None;
    let mut end = 0i64;
    for &sid in &ids {
        match first {
            None => first = Some(sid),
            Some(f) if end + 1 != sid => {
                if end - f > 255 {
                    format = 2;
                }
                ranges.push((f, end - f));
                first = Some(sid);
            }
            Some(_) => {}
        }
        end = sid;
    }
    if end != 0 {
        let f = first.unwrap_or(0);
        if end - f > 255 {
            format = 2;
        }
        ranges.push((f, end - f));
    }
    let mut f1 = Out::default();
    f1.u8(format, "format")?;
    for (f, left) in ranges {
        f1.u16(f, "a charset range start")?;
        if format == 1 { f1.u8(left, "a charset range count")? } else { f1.u16(left, "a charset range count")? }
    }
    Ok(if f1.0.len() < f0.0.len() { f1.0 } else { f0.0 })
}

fn encoding_bytes(charset: &[String], codes: &[String]) -> Result<Vec<u8>> {
    let mut m: HashMap<&str, i64> = HashMap::new();
    for (code, name) in codes.iter().enumerate() {
        if name != ".notdef" {
            m.insert(name.as_str(), code as i64);
        }
    }
    let mut list: Vec<Option<i64>> = charset.iter().skip(1).map(|n| m.get(n.as_str()).copied()).collect();
    while list.last() == Some(&None) {
        list.pop();
    }
    let mut f0 = Out::default();
    f0.u8(0, "format")?;
    f0.u8(list.len() as i64, "the encoding code count")?;
    for c in &list {
        f0.u8(c.unwrap_or(0), "an encoding code")?;
    }
    let mut ranges: Vec<(i64, i64)> = Vec::new();
    let mut first: Option<i64> = None;
    let mut end = 0i64;
    for name in charset.iter().skip(1) {
        let code = m.get(name.as_str()).copied().unwrap_or(-1);
        match first {
            None => first = Some(code),
            Some(f) if end + 1 != code => {
                ranges.push((f, end - f));
                first = Some(code);
            }
            Some(_) => {}
        }
        end = code;
    }
    // With only `.notdef`, fontTools stops here with an error; an empty range list is written.
    if let Some(f) = first {
        ranges.push((f, end - f));
    }
    while ranges.last().is_some_and(|r| r.0 == -1) {
        ranges.pop();
    }
    let mut f1 = Out::default();
    f1.u8(1, "format")?;
    f1.u8(ranges.len() as i64, "the encoding range count")?;
    for (f, left) in ranges {
        f1.u8(if f == -1 { 0 } else { f }, "an encoding range start")?;
        f1.u8(left, "an encoding range count")?;
    }
    Ok(if f0.0.len() < f1.0.len() { f0.0 } else { f1.0 })
}

fn fd_select_bytes(fds: &[i64], format: Option<i64>) -> Result<Vec<u8>> {
    let pack = |format: i64| -> Result<Vec<u8>> {
        let mut out = Out::default();
        out.u8(format, "the FDSelect format")?;
        if format == 0 {
            for fd in fds {
                out.u8(*fd, "an FD index")?;
            }
            return Ok(out.0);
        }
        let mut ranges: Vec<(usize, i64)> = Vec::new();
        let mut last = -1i64;
        for (i, fd) in fds.iter().enumerate() {
            if *fd != last {
                ranges.push((i, *fd));
                last = *fd;
            }
        }
        if format == 3 {
            out.u16(ranges.len() as i64, "the FDSelect range count")?;
            for (first, fd) in &ranges {
                out.u16(*first as i64, "an FDSelect range start")?;
                out.u8(*fd, "an FD index")?;
            }
            out.u16(fds.len() as i64, "the FDSelect sentinel")?;
        } else {
            out.u32(ranges.len() as i64, "the FDSelect range count")?;
            for (first, fd) in &ranges {
                out.u32(*first as i64, "an FDSelect range start")?;
                out.u16(*fd, "an FD index")?;
            }
            out.u32(fds.len() as i64, "the FDSelect sentinel")?;
        }
        Ok(out.0)
    };
    match format {
        Some(f @ (0 | 3 | 4)) => pack(f),
        _ => {
            let (f0, f3) = (pack(0)?, pack(3)?);
            Ok(if f0.len() < f3.len() { f0 } else { f3 })
        }
    }
}

// ── VarStore ───────────────────────────────────────────────────────────────────────────────────

fn f2dot14_of(e: Option<&Element>) -> Result<i64> {
    let e = e.ok_or_else(|| bad("a region axis lacks a coordinate"))?;
    float_to_fixed(num_lit(attr(e, "value")?)?.as_f64(), 14)
}

/// An `ItemVariationStore` from its TTX element, laid out as fontTools' table writer lays it out
/// for the CFF2 store: header, region list, then each `VarData` (identical ones stored once).
fn compile_var_store(e: &Element) -> Result<Vec<u8>> {
    let format = match e.child("Format") {
        Some(f) => int_value(f)?,
        None => 1,
    };
    let mut regions = Out::default();
    let region_elements: Vec<&Element> =
        e.child("VarRegionList").map(|l| l.children.iter().filter(|c| c.name == "Region").collect()).unwrap_or_default();
    let mut axis_count = 0usize;
    let mut region_body = Out::default();
    for r in &region_elements {
        let axes: Vec<&Element> = r.children.iter().filter(|c| c.name == "VarRegionAxis").collect();
        axis_count = axes.len();
        for a in axes {
            for coord in ["StartCoord", "PeakCoord", "EndCoord"] {
                region_body.i16(f2dot14_of(a.child(coord))?, coord)?;
            }
        }
    }
    regions.u16(axis_count as i64, "RegionAxisCount")?;
    regions.u16(region_elements.len() as i64, "RegionCount")?;
    regions.bytes(&region_body.0);
    let mut datas: Vec<Vec<u8>> = Vec::new();
    for vd in e.children.iter().filter(|c| c.name == "VarData") {
        let num_shorts = match vd.child("NumShorts") {
            Some(n) => int_value(n)?,
            None => 0,
        };
        let indices: Vec<i64> = vd.children.iter().filter(|c| c.name == "VarRegionIndex").map(int_value).collect::<Result<_>>()?;
        let items: Vec<Vec<i64>> = vd
            .children
            .iter()
            .filter(|c| c.name == "Item")
            .map(|c| {
                attr(c, "value")?
                    .trim()
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(int_lit)
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<_>>()?;
        let mut out = Out::default();
        out.u16(items.len() as i64, "ItemCount")?;
        out.u16(num_shorts, "NumShorts")?;
        out.u16(indices.len() as i64, "VarRegionCount")?;
        for i in &indices {
            out.u16(*i, "VarRegionIndex")?;
        }
        let long = num_shorts & 0x8000 != 0;
        let words = usize::try_from(num_shorts & 0x7FFF).unwrap_or(0);
        let regions_n = indices.len();
        for item in &items {
            let (n1, n2) = (regions_n.min(words), regions_n.max(words));
            for k in 0..n2 {
                let v = if k < regions_n { item.get(k).copied().unwrap_or(0) } else { 0 };
                match (k < n1, long) {
                    (true, true) => out.i32(v, "a delta")?,
                    (true, false) | (false, true) => out.i16(v, "a delta")?,
                    (false, false) => out.i8(v, "a delta")?,
                }
            }
        }
        datas.push(out.0);
    }
    let mut out = Out::default();
    let header_len = 8 + 4 * datas.len();
    out.u16(format, "VarStore.Format")?;
    out.u32(header_len as i64, "the region list offset")?;
    out.u16(datas.len() as i64, "VarDataCount")?;
    let mut body: Vec<u8> = regions.0.clone();
    let mut placed: HashMap<&Vec<u8>, usize> = HashMap::new();
    for d in &datas {
        let at = match placed.get(d) {
            Some(at) => *at,
            None => {
                let at = header_len + body.len();
                body.extend_from_slice(d);
                placed.insert(d, at);
                at
            }
        };
        out.u32(at as i64, "a VarData offset")?;
    }
    out.bytes(&body);
    Ok(out.0)
}

// ── The table ──────────────────────────────────────────────────────────────────────────────────

/// One section of the table, in the order fontTools' writer lays them out.
enum Section {
    Header,
    /// The offset size (CFF) or the Top DICT size (CFF2) that follows the header.
    HeaderTail,
    Fixed(Vec<u8>),
    TopDict,
    FdArray,
    Private(usize),
    Subrs(usize),
    Charset,
    Encoding,
    FdSelect,
    CharStrings,
    VarStore,
}

/// Everything compiled once, and the offsets that change from one layout pass to the next.
struct Layout<'a> {
    cff2: bool,
    top: &'a TopDict,
    /// The Private DICT of the Top DICT (one entry) or of each font dict, by font dict index.
    privates: Vec<Option<&'a Private>>,
    subrs: Vec<Option<Vec<u8>>>,
    charset: Option<Vec<u8>>,
    charset_code: Option<i64>,
    encoding: Option<Vec<u8>>,
    encoding_code: Option<i64>,
    fd_select: Option<Vec<u8>>,
    charstrings: Vec<u8>,
    var_store: Option<Vec<u8>>,
    // Offsets, as fontTools' compilers store them in the DICTs.
    at_charset: i64,
    at_encoding: Option<i64>,
    at_charstrings: i64,
    at_fd_array: i64,
    at_fd_select: i64,
    at_var_store: i64,
    private_at: Vec<(i64, i64)>,
    subrs_at: Vec<i64>,
}

impl Layout<'_> {
    fn private_bytes(&self, i: usize, strings: &mut Option<&mut Strings>) -> Result<Vec<u8>> {
        let p = self.privates.get(i).copied().flatten().ok_or_else(|| bad("a Private DICT is missing"))?;
        let order: Vec<&str> = PRIVATE_OPS.iter().map(|(_, n, _)| *n).collect();
        let specials: Vec<(&str, Vec<i64>)> =
            if p.subrs.is_some() { vec![("Subrs", vec![self.subrs_at.get(i).copied().unwrap_or(0)])] } else { Vec::new() };
        compile_dict(&p.values, PRIVATE_OPS, &order, private_default, &specials, strings, crate::limits::MAX_CFF2_STACK)
    }

    fn top_bytes(&self, strings: &mut Option<&mut Strings>) -> Result<Vec<u8>> {
        let mut specials: Vec<(&str, Vec<i64>)> = Vec::new();
        if let Some(code) = self.charset_code {
            specials.push(("charset", vec![code]));
        } else if self.charset.is_some() {
            specials.push(("charset", vec![self.at_charset]));
        }
        if let Some(code) = self.encoding_code {
            specials.push(("Encoding", vec![code]));
        } else if let Some(at) = self.at_encoding {
            specials.push(("Encoding", vec![at]));
        }
        if self.top.private.is_some() {
            let (size, at) = self.private_at.first().copied().unwrap_or((0, 0));
            specials.push(("Private", vec![size, at]));
        }
        if self.fd_select.is_some() {
            specials.push(("FDSelect", vec![self.at_fd_select]));
        }
        if self.top.fd_array.is_some() {
            specials.push(("FDArray", vec![self.at_fd_array]));
        }
        if self.top.has_charstrings {
            specials.push(("CharStrings", vec![self.at_charstrings]));
        }
        if self.var_store.is_some() {
            specials.push(("VarStore", vec![self.at_var_store]));
        }
        if self.cff2 {
            compile_dict(&self.top.values, TOP_OPS, TOP_ORDER_CFF2, cff2_top_default, &specials, strings, 0)
        } else {
            let order: Vec<&str> = TOP_OPS.iter().map(|(_, n, _)| *n).collect();
            compile_dict(&self.top.values, TOP_OPS, &order, top_default, &specials, strings, 0)
        }
    }

    fn font_dict_bytes(&self, strings: &mut Option<&mut Strings>) -> Result<Vec<Vec<u8>>> {
        let fds = self.top.fd_array.as_deref().unwrap_or_default();
        let mut out = Vec::with_capacity(fds.len());
        for (i, fd) in fds.iter().enumerate() {
            let specials: Vec<(&str, Vec<i64>)> = if fd.private.is_some() {
                let (size, at) = self.private_at.get(i).copied().unwrap_or((0, 0));
                vec![("Private", vec![size, at])]
            } else {
                Vec::new()
            };
            out.push(compile_dict(&fd.values, TOP_OPS, FD_ORDER, no_default, &specials, strings, 0)?);
        }
        Ok(out)
    }

    /// The length of a section in this pass; offsets the section owns are stored on the way, as
    /// fontTools' `setPos` stores them.
    fn length(&mut self, section: &Section, pos: usize, strings: &mut Option<&mut Strings>) -> Result<usize> {
        Ok(match section {
            Section::Header => {
                if self.cff2 {
                    5
                } else {
                    4
                }
            }
            Section::HeaderTail => {
                if self.cff2 {
                    2
                } else {
                    1
                }
            }
            Section::Fixed(bytes) => bytes.len(),
            Section::TopDict => {
                let t = self.top_bytes(strings)?;
                if self.cff2 { t.len() } else { index_bytes(&[t], false)?.len() }
            }
            Section::FdArray => {
                let len = index_bytes(&self.font_dict_bytes(strings)?, self.cff2)?.len();
                self.at_fd_array = pos as i64;
                len
            }
            Section::Private(i) => {
                let len = self.private_bytes(*i, strings)?.len();
                if let Some(slot) = self.private_at.get_mut(*i) {
                    *slot = (len as i64, pos as i64);
                }
                len
            }
            Section::Subrs(i) => {
                let private_pos = self.private_at.get(*i).map_or(0, |p| p.1);
                if let Some(slot) = self.subrs_at.get_mut(*i) {
                    *slot = pos as i64 - private_pos;
                }
                self.subrs.get(*i).and_then(Option::as_ref).map_or(0, Vec::len)
            }
            Section::Charset => {
                self.at_charset = pos as i64;
                self.charset.as_ref().map_or(0, Vec::len)
            }
            Section::Encoding => {
                self.at_encoding = Some(pos as i64);
                self.encoding.as_ref().map_or(0, Vec::len)
            }
            Section::FdSelect => {
                self.at_fd_select = pos as i64;
                self.fd_select.as_ref().map_or(0, Vec::len)
            }
            Section::CharStrings => {
                self.at_charstrings = pos as i64;
                self.charstrings.len()
            }
            Section::VarStore => {
                self.at_var_store = pos as i64;
                self.var_store.as_ref().map_or(0, |v| v.len() + 2)
            }
        })
    }
}

fn cff2_top_default(name: &str) -> Option<Value> {
    if name == "FontMatrix" { top_default(name) } else { None }
}

fn compile_cff(element: &Element, ctx: &ReadContext, cff2: bool) -> Result<Vec<u8>> {
    let major = match element.child("major") {
        Some(m) => int_value(m)?,
        None => 1,
    };
    let minor = match element.child("minor") {
        Some(m) => int_value(m)?,
        None => 0,
    };
    let expected = if cff2 { 2 } else { 1 };
    if major != expected {
        return Err(bad(format!("Invalid CFF 'major' version: expected {expected}, found {major}")));
    }
    let mut tops: Vec<TopDict> = Vec::new();
    let mut gsubrs: Vec<Charstring> = Vec::new();
    for child in &element.children {
        match child.name.as_str() {
            "CFFFont" => tops.push(top_from(child)?),
            "GlobalSubrs" => gsubrs.extend(subrs_from(child)?),
            _ => {}
        }
    }
    if tops.len() > 1 {
        return Err(TablesError::Unsupported("CFF tables with more than one font".into()));
    }
    let mut top = tops.pop().ok_or_else(|| bad("the table holds no CFFFont"))?;
    let gsubrs: Vec<Vec<u8>> = gsubrs.iter().map(|c| compile_charstring(c, cff2)).collect::<Result<_>>()?;
    let data = layout(&top, &gsubrs, ctx, cff2, major, minor)?;
    if cff2 || !ctx.recalc {
        return Ok(data);
    }
    // fontTools' `recalcFontBBox`: the union of the charstring bounds, rounded outwards.
    let bbox = glyph_bounds(&data)?.into_iter().flatten().reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]);
    let new = match bbox {
        Some(b) => Value::List([b[0].floor(), b[1].floor(), b[2].ceil(), b[3].ceil()].iter().map(|v| V::N(Num::Int(*v as i64))).collect()),
        None => Value::List(vec![V::N(Num::Int(0)); 4]),
    };
    top.values.insert("FontBBox", new);
    layout(&top, &gsubrs, ctx, cff2, major, minor)
}

fn layout(top: &TopDict, gsubrs: &[Vec<u8>], ctx: &ReadContext, cff2: bool, major: i64, minor: i64) -> Result<Vec<u8>> {
    let charset_names = ctx.order()?;
    let cid = top.values.contains_key("ROS");
    let mut strings = Strings::default();
    // The charset compiler numbers the glyph names before any DICT is compiled.
    let (charset, charset_code) = if cff2 || charset_names.is_empty() {
        (None, None)
    } else {
        if charset_names.first().map(String::as_str) != Some(".notdef") {
            return Err(bad("the first glyph is not '.notdef'"));
        }
        match if cid { None } else { standard_charset(charset_names) } {
            Some(code) => (None, Some(code)),
            None => (Some(charset_bytes(charset_names, cid, &mut strings)?), None),
        }
    };
    let (encoding, encoding_code) = match (&top.encoding, cff2) {
        (Some(EncodingValue::Custom(codes)), false) => (Some(encoding_bytes(charset_names, codes)?), None),
        (Some(EncodingValue::Expert), false) => (None, Some(1)),
        _ => (None, None),
    };
    let charstring_of = |n: &String| top.charstrings.get(n).ok_or_else(|| bad(format!("no charstring for the glyph '{n}'")));
    let fd_select = match top.fd_select_format {
        Some(_) => {
            let fds: Vec<i64> = charset_names.iter().map(|n| charstring_of(n).map(|c| c.1)).collect::<Result<_>>()?;
            Some(fd_select_bytes(&fds, top.fd_select_format)?)
        }
        None => None,
    };
    let charstrings = if top.has_charstrings {
        let items: Vec<Vec<u8>> =
            charset_names.iter().map(|n| charstring_of(n).and_then(|c| compile_charstring(&c.0, cff2))).collect::<Result<_>>()?;
        index_bytes(&items, cff2)?
    } else {
        Vec::new()
    };
    let privates: Vec<Option<&Private>> = match &top.fd_array {
        Some(fds) => fds.iter().map(|fd| fd.private.as_ref()).collect(),
        None => vec![top.private.as_ref()],
    };
    let subrs: Vec<Option<Vec<u8>>> = privates
        .iter()
        .map(|p| match p.and_then(|p| p.subrs.as_ref()) {
            Some(list) => {
                list.iter().map(|c| compile_charstring(c, cff2)).collect::<Result<Vec<_>>>().and_then(|items| index_bytes(&items, cff2)).map(Some)
            }
            None => Ok(None),
        })
        .collect::<Result<_>>()?;
    let n_private = privates.len();
    let mut l = Layout {
        cff2,
        top,
        privates,
        subrs,
        charset,
        charset_code,
        encoding,
        encoding_code,
        fd_select,
        charstrings,
        var_store: if cff2 { top.var_store.clone() } else { None },
        at_charset: 0,
        at_encoding: None,
        at_charstrings: 0,
        at_fd_array: 0,
        at_fd_select: 0,
        at_var_store: 0,
        private_at: vec![(0, 0); n_private],
        subrs_at: vec![0; n_private],
    };
    // Strings the DICTs name, in the order fontTools' first layout pass asks for them.
    l.top_bytes(&mut (!cff2).then_some(&mut strings))?;
    l.font_dict_bytes(&mut (!cff2).then_some(&mut strings))?;
    let mut sections: Vec<Section> = vec![Section::Header, Section::HeaderTail];
    if !cff2 {
        sections.push(Section::Fixed(index_bytes(&[top.name.chars().map(|c| c as u32 as u8).collect()], false)?));
    }
    sections.push(Section::TopDict);
    if !cff2 {
        let items: Vec<Vec<u8>> = strings.custom.iter().map(|s| s.chars().map(|c| c as u32 as u8).collect()).collect();
        sections.push(Section::Fixed(index_bytes(&items, false)?));
    }
    sections.push(Section::Fixed(index_bytes(gsubrs, cff2)?));
    if l.var_store.is_some() {
        sections.push(Section::VarStore);
    }
    if l.charset.is_some() {
        sections.push(Section::Charset);
    }
    if l.encoding.is_some() {
        sections.push(Section::Encoding);
    }
    if l.fd_select.is_some() {
        sections.push(Section::FdSelect);
    }
    if top.has_charstrings {
        sections.push(Section::CharStrings);
    }
    if top.fd_array.is_some() {
        sections.push(Section::FdArray);
    }
    for i in 0..n_private {
        if l.privates.get(i).copied().flatten().is_some() {
            sections.push(Section::Private(i));
            if l.subrs.get(i).is_some_and(Option::is_some) {
                sections.push(Section::Subrs(i));
            }
        }
    }
    let mut last: Option<Vec<usize>> = None;
    let mut settled = false;
    for _pass in 0..64 {
        let mut pos = 0usize;
        let mut positions = vec![0usize];
        for section in &sections {
            pos += l.length(section, pos, &mut (!cff2).then_some(&mut strings))?;
            positions.push(pos);
            if pos > crate::limits::MAX_SFNT {
                return Err(TablesError::TooLarge { what: "the CFF table", limit: crate::limits::MAX_SFNT });
            }
        }
        if last.as_ref() == Some(&positions) {
            settled = true;
            break;
        }
        last = Some(positions);
    }
    let total = last.as_ref().and_then(|p| p.last()).copied().unwrap_or(0);
    if !settled {
        return Err(bad("the layout did not settle"));
    }
    let top_bytes = l.top_bytes(&mut (!cff2).then_some(&mut strings))?;
    let mut out = Out::default();
    for section in &sections {
        match section {
            Section::Header => {
                out.u8(major, "the CFF major version")?;
                out.u8(minor, "the CFF minor version")?;
                out.u8(if cff2 { 5 } else { 4 }, "the CFF header size")?;
            }
            Section::HeaderTail => {
                if cff2 {
                    out.u16(top_bytes.len() as i64, "the Top DICT size")?;
                } else {
                    out.u8(off_size(total) as i64, "the CFF offset size")?;
                }
            }
            Section::Fixed(bytes) => out.bytes(bytes),
            Section::TopDict => {
                if cff2 {
                    out.bytes(&top_bytes);
                } else {
                    out.bytes(&index_bytes(std::slice::from_ref(&top_bytes), false)?);
                }
            }
            Section::FdArray => out.bytes(&index_bytes(&l.font_dict_bytes(&mut (!cff2).then_some(&mut strings))?, cff2)?),
            Section::Private(i) => out.bytes(&l.private_bytes(*i, &mut (!cff2).then_some(&mut strings))?),
            Section::Subrs(i) => out.bytes(l.subrs.get(*i).and_then(Option::as_deref).unwrap_or_default()),
            Section::Charset => out.bytes(l.charset.as_deref().unwrap_or_default()),
            Section::Encoding => out.bytes(l.encoding.as_deref().unwrap_or_default()),
            Section::FdSelect => out.bytes(l.fd_select.as_deref().unwrap_or_default()),
            Section::CharStrings => out.bytes(&l.charstrings),
            Section::VarStore => {
                let v = l.var_store.as_deref().unwrap_or_default();
                out.u16(v.len().min(0xFFFF) as i64, "the VarStore length")?;
                out.bytes(v);
            }
        }
    }
    if out.0.len() != total {
        return Err(bad("the layout did not settle"));
    }
    Ok(out.0)
}

// ── Bounds ─────────────────────────────────────────────────────────────────────────────────────

/// fontTools' `BoundsPen`: on-curve points, and curve extremes where a control point lies
/// outside the box so far.
#[derive(Default)]
struct BoundsSink {
    bounds: Option<[f64; 4]>,
    current: (f64, f64),
}

impl BoundsSink {
    fn add(&mut self, x: f64, y: f64) {
        self.bounds = Some(match self.bounds {
            None => [x, y, x, y],
            Some(b) => [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)],
        });
    }

    fn inside(&self, x: f64, y: f64) -> bool {
        self.bounds.is_some_and(|b| b[0] <= x && x <= b[2] && b[1] <= y && y <= b[3])
    }
}

fn f(v: Fixed) -> f64 {
    f64::from(v.to_bits()) / 65536.0
}

/// The extremes of one coordinate of a cubic segment.
fn cubic_extremes(p0: f64, p1: f64, p2: f64, p3: f64) -> Vec<f64> {
    let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
    let b = 2.0 * (p0 - 2.0 * p1 + p2);
    let c = p1 - p0;
    let mut ts = Vec::new();
    if a.abs() < 1e-12 {
        if b.abs() > 1e-12 {
            ts.push(-c / b);
        }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc >= 0.0 {
            let r = disc.sqrt();
            ts.push((-b + r) / (2.0 * a));
            ts.push((-b - r) / (2.0 * a));
        }
    }
    ts.into_iter()
        .filter(|t| *t > 0.0 && *t < 1.0)
        .map(|t| {
            let mt = 1.0 - t;
            mt * mt * mt * p0 + 3.0 * mt * mt * t * p1 + 3.0 * mt * t * t * p2 + t * t * t * p3
        })
        .collect()
}

impl read_fonts::ps::cs::CommandSink for BoundsSink {
    fn move_to(&mut self, x: Fixed, y: Fixed) {
        self.current = (f(x), f(y));
        self.add(f(x), f(y));
    }
    fn line_to(&mut self, x: Fixed, y: Fixed) {
        self.current = (f(x), f(y));
        self.add(f(x), f(y));
    }
    fn curve_to(&mut self, cx0: Fixed, cy0: Fixed, cx1: Fixed, cy1: Fixed, x: Fixed, y: Fixed) {
        let (x0, y0) = self.current;
        let (c0, c1, p) = ((f(cx0), f(cy0)), (f(cx1), f(cy1)), (f(x), f(y)));
        self.add(p.0, p.1);
        if !self.inside(c0.0, c0.1) || !self.inside(c1.0, c1.1) {
            // fontTools' `calcCubicBounds`: the end points and the extremes in between.
            self.add(x0, y0);
            for ex in cubic_extremes(x0, c0.0, c1.0, p.0) {
                self.add(ex, p.1);
            }
            for ey in cubic_extremes(y0, c0.1, c1.1, p.1) {
                self.add(p.0, ey);
            }
        }
        self.current = p;
    }
    fn close(&mut self) {}
}

/// The bounds of every glyph of a `CFF ` or `CFF2` table, in glyph order; `None` for glyphs
/// without outlines.
pub(crate) fn glyph_bounds(data: &[u8]) -> Result<Vec<Option<[f64; 4]>>> {
    use read_fonts::ps::cff::CffFontRef;
    use read_fonts::types::GlyphId;
    let cant = |e: read_fonts::ps::error::Error| TablesError::invalid(format!("the CFF charstrings could not be evaluated: {e}"));
    let font = CffFontRef::new(data, 0, None).map_err(cant)?;
    let mut out = Vec::with_capacity(font.num_glyphs() as usize);
    for gid in 0..font.num_glyphs() {
        let gid = GlyphId::new(gid);
        let index = font.subfont_index(gid).unwrap_or(0);
        let subfont = font.subfont(index, &[]).map_err(cant)?;
        let mut sink = BoundsSink::default();
        font.evaluate_charstring(&subfont, gid, &[], &mut sink).map_err(cant)?;
        out.push(sink.bounds);
    }
    Ok(out)
}

/// The `FontBBox` of the Top DICT of a `CFF ` table; `None` when the DICT has none.
pub(crate) fn font_bbox(data: &[u8]) -> Result<Option<[f64; 4]>> {
    let cut = || TablesError::invalid("the 'CFF ' table is cut short");
    let header = usize::from(*data.get(2).ok_or_else(cut)?);
    let (_, after_names) = skip_index(data, header)?;
    let count = usize::from(u16::from_be_bytes([*data.get(after_names).ok_or_else(cut)?, *data.get(after_names + 1).ok_or_else(cut)?]));
    if count == 0 {
        return Ok(None);
    }
    let size = usize::from(*data.get(after_names + 2).ok_or_else(cut)?);
    let read_off = |at: usize| -> Result<usize> {
        data.get(at..at.saturating_add(size))
            .map(|b| b.iter().fold(0usize, |a, x| a.saturating_mul(256).saturating_add(usize::from(*x))))
            .ok_or_else(cut)
    };
    let base = after_names + 2 + (count + 1) * size;
    let start = base.saturating_add(read_off(after_names + 3)?);
    let end = base.saturating_add(read_off(after_names + 3 + size)?);
    let dict = data.get(start..end).ok_or_else(cut)?;
    let mut stack: Vec<f64> = Vec::new();
    let mut i = 0usize;
    while let Some(&b0) = dict.get(i) {
        i += 1;
        match b0 {
            0..=21 => {
                let op = if b0 == 12 {
                    let b1 = dict.get(i).copied().unwrap_or(0);
                    i += 1;
                    1200 + u16::from(b1)
                } else {
                    u16::from(b0)
                };
                if op == 5 {
                    if let [a, b, c, d] = stack.as_slice() {
                        return Ok(Some([*a, *b, *c, *d]));
                    }
                    return Ok(None);
                }
                stack.clear();
            }
            28 => {
                let v = i16::from_be_bytes([*dict.get(i).ok_or_else(cut)?, *dict.get(i + 1).ok_or_else(cut)?]);
                stack.push(f64::from(v));
                i += 2;
            }
            29 => {
                let b: [u8; 4] = dict.get(i..i + 4).and_then(|b| b.try_into().ok()).ok_or_else(cut)?;
                stack.push(f64::from(i32::from_be_bytes(b)));
                i += 4;
            }
            30 => {
                let mut s = String::new();
                'real: while let Some(&byte) = dict.get(i) {
                    i += 1;
                    for nibble in [byte >> 4, byte & 0xF] {
                        match nibble {
                            0..=9 => s.push((b'0' + nibble) as char),
                            0xA => s.push('.'),
                            0xB => s.push('E'),
                            0xC => s.push_str("E-"),
                            0xE => s.push('-'),
                            0xF => break 'real,
                            _ => {}
                        }
                    }
                }
                stack.push(s.parse::<f64>().unwrap_or(0.0));
            }
            32..=246 => stack.push(f64::from(i16::from(b0) - 139)),
            247..=250 => {
                let b1 = *dict.get(i).ok_or_else(cut)?;
                i += 1;
                stack.push(f64::from((i16::from(b0) - 247) * 256 + i16::from(b1) + 108));
            }
            251..=254 => {
                let b1 = *dict.get(i).ok_or_else(cut)?;
                i += 1;
                stack.push(f64::from(-(i16::from(b0) - 251) * 256 - i16::from(b1) - 108));
            }
            _ => return Ok(None),
        }
    }
    Ok(None)
}

fn skip_index(data: &[u8], at: usize) -> Result<(usize, usize)> {
    let cut = || TablesError::invalid("the 'CFF ' table is cut short");
    let count = usize::from(u16::from_be_bytes([*data.get(at).ok_or_else(cut)?, *data.get(at + 1).ok_or_else(cut)?]));
    if count == 0 {
        return Ok((0, at + 2));
    }
    let size = usize::from(*data.get(at + 2).ok_or_else(cut)?);
    let last_at = at + 3 + count * size;
    let last = data.get(last_at..last_at + size).ok_or_else(cut)?.iter().fold(0usize, |a, x| a.saturating_mul(256).saturating_add(usize::from(*x)));
    Ok((count, (last_at + size - 1).saturating_add(last)))
}

// ── VORG ───────────────────────────────────────────────────────────────────────────────────────

fn vorg(element: &Element, ctx: &ReadContext) -> Result<Vec<u8>> {
    let mut major = None;
    let mut minor = None;
    let mut default_y = None;
    let mut records: HashMap<String, i64> = HashMap::new();
    let mut names: Vec<String> = Vec::new();
    for child in &element.children {
        match child.name.as_str() {
            "VOriginRecord" => {
                let glyph = child.child("glyphName").map(|g| attr(g, "value")).transpose()?.ok_or_else(|| bad("a VOriginRecord has no glyphName"))?;
                let origin = child.child("vOrigin").map(int_value).transpose()?.ok_or_else(|| bad("a VOriginRecord has no vOrigin"))?;
                if records.insert(glyph.to_string(), origin).is_none() {
                    names.push(glyph.to_string());
                }
            }
            "majorVersion" => major = Some(int_value(child)?),
            "minorVersion" => minor = Some(int_value(child)?),
            "defaultVertOriginY" => default_y = Some(int_value(child)?),
            _ => {}
        }
    }
    let mut table: Vec<(u32, i64)> = names.iter().map(|n| Ok((ctx.glyph_id(n)?, records.get(n).copied().unwrap_or(0)))).collect::<Result<_>>()?;
    table.sort_unstable();
    let mut out = Out::default();
    out.u16(major.ok_or_else(|| bad("VORG has no majorVersion"))?, "VORG.majorVersion")?;
    out.u16(minor.ok_or_else(|| bad("VORG has no minorVersion"))?, "VORG.minorVersion")?;
    out.i16(default_y.ok_or_else(|| bad("VORG has no defaultVertOriginY"))?, "VORG.defaultVertOriginY")?;
    out.u16(table.len() as i64, "VORG.numVertOriginYMetrics")?;
    for (gid, y) in table {
        out.u16(i64::from(gid), "a glyph id")?;
        out.i16(y, "a vertical origin")?;
    }
    Ok(out.0)
}
