//! TTX writer: `CFF `, `CFF2` and `VORG` (M12.5).
//!
//! Each table is written element for element as fontTools' `ttx` writes it, read with the same
//! rules: a table fontTools fails on is an error here too, and [`super::dump`] then writes it as
//! hex. Ported from fontTools' `cffLib` (`CFFFontSet`, the DICT, INDEX, charset, encoding,
//! FDSelect and FDArray readers and their XML converters), `misc.psCharStrings` (the DICT
//! decompiler and the Type 2 charstring decompiler and writer) and `ttLib.tables.V_O_R_G_` (MIT).
//!
//! What the dump holds, as `ttx` writes it:
//! - The Top DICT in a fixed key order, with the default of every key that has one (a CFF2 Top
//!   DICT writes only its own keys); the `charset` as a comment (the glyph names are the
//!   `GlyphOrder`); a custom encoding as `map` elements.
//! - Private DICTs with their defaults, blended values as `blend` elements, and local
//!   subroutines; font dicts of an FDArray with their four keys (CFF) or only `Private` (CFF2).
//!   Keys a dict holds but does not write are named in a comment.
//! - Charstrings as their operators and operands, one operator a line, sorted by glyph name.
//!   Every glyph is decompiled; a subroutine is decompiled when a glyph calls it, and one that
//!   no glyph calls is written as hex with `raw="1"`. An operator the format does not define
//!   ends the charstring; operands after the last operator are written on the closing line.
//! - The CFF2 variation store, with its region list and VarData.
//!
//! Known differences, all on fonts the format does not allow: a charstring's operand stack is
//! limited to 513 values and subroutine nesting to [`crate::limits::MAX_SUBR_NESTING`] levels
//! (fontTools only stops at Python's recursion limit), and the work of decompiling one table
//! is bounded; the expert charsets (predefined charsets 1 and 2) and the reserved DICT operand
//! byte 255 are errors; a blended Private DICT value without a variation store is an error
//! (fontTools drops the Private DICT and the charstrings).

mod charstring;
#[cfg(test)]
mod tests;
mod varstore;

use std::collections::BTreeMap;
use std::rc::Rc;

use read_fonts::ps::string::STANDARD_STRINGS;

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

use super::core::Rd;
use super::{XmlWriter, escape_into, py_float, unique_names};
use charstring::{Budget, Env, Program, SubrSet};
use varstore::VarStore;

/// Writes table `tag` (bytes `data`, from `font`) into `w`. Returns false, writing nothing,
/// for a tag this module does not handle.
pub fn write_table(tag: Tag, data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<bool> {
    match &tag {
        b"CFF " => cff(data, font, w, false)?,
        b"CFF2" => cff(data, font, w, true)?,
        b"VORG" => vorg(data, font, w)?,
        _ => return Ok(false),
    }
    Ok(true)
}

fn bad(what: &str) -> TablesError {
    TablesError::invalid(format!("the CFF data could not be read: {what}"))
}

// ── Numbers and DICT values ────────────────────────────────────────────────────────────────────

/// A DICT or charstring operand: Python's `int` or `float`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Num {
    Int(i64),
    Real(f64),
}

impl Num {
    /// Python's `str()`.
    pub(super) fn py_str(self) -> String {
        match self {
            Num::Int(v) => v.to_string(),
            Num::Real(v) => py_float(v),
        }
    }

    fn add(self, other: Num) -> Result<Num> {
        match (self, other) {
            (Num::Int(a), Num::Int(b)) => a.checked_add(b).map(Num::Int).ok_or_else(|| bad("a number is too large")),
            (a, b) => Ok(Num::Real(a.as_f64() + b.as_f64())),
        }
    }

    fn as_f64(self) -> f64 {
        match self {
            Num::Int(v) => v as f64,
            Num::Real(v) => v,
        }
    }

    fn is_zero(self) -> bool {
        self.as_f64() == 0.0
    }
}

/// An item of the DICT operand stack: a number, or the list a `blend` operator leaves.
#[derive(Clone, Debug, PartialEq)]
enum It {
    N(Num),
    B(Vec<Num>),
}

impl It {
    /// Python's `str()`, a list as `[a, b]`.
    fn py_str(&self) -> String {
        match self {
            It::N(n) => n.py_str(),
            It::B(list) => format!("[{}]", list.iter().map(|n| n.py_str()).collect::<Vec<_>>().join(", ")),
        }
    }

    fn int(&self) -> Option<i64> {
        match self {
            It::N(Num::Int(v)) => Some(*v),
            _ => None,
        }
    }
}

/// A decoded DICT value.
#[derive(Clone, Debug, PartialEq)]
enum Value {
    Number(It),
    Array(Vec<It>),
    Sid(String),
    Ros(String, String, It),
    Pair(It, It),
}

impl Value {
    fn int(&self) -> Option<i64> {
        match self {
            Value::Number(it) => it.int(),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Arg {
    Number,
    Sid,
    Array,
    Delta,
    Ros,
    Pair,
    BlendList,
}

/// fontTools' `topDictOperators`: escaped operators are `1200 + b1`.
const TOP_OPS: &[(u16, &str, Arg)] = &[
    (25, "maxstack", Arg::Number),
    (1230, "ROS", Arg::Ros),
    (1220, "SyntheticBase", Arg::Number),
    (0, "version", Arg::Sid),
    (1, "Notice", Arg::Sid),
    (1200, "Copyright", Arg::Sid),
    (2, "FullName", Arg::Sid),
    (1238, "FontName", Arg::Sid),
    (3, "FamilyName", Arg::Sid),
    (4, "Weight", Arg::Sid),
    (1201, "isFixedPitch", Arg::Number),
    (1202, "ItalicAngle", Arg::Number),
    (1203, "UnderlinePosition", Arg::Number),
    (1204, "UnderlineThickness", Arg::Number),
    (1205, "PaintType", Arg::Number),
    (1206, "CharstringType", Arg::Number),
    (1207, "FontMatrix", Arg::Array),
    (13, "UniqueID", Arg::Number),
    (5, "FontBBox", Arg::Array),
    (1208, "StrokeWidth", Arg::Number),
    (14, "XUID", Arg::Array),
    (1221, "PostScript", Arg::Sid),
    (1222, "BaseFontName", Arg::Sid),
    (1223, "BaseFontBlend", Arg::Delta),
    (1231, "CIDFontVersion", Arg::Number),
    (1232, "CIDFontRevision", Arg::Number),
    (1233, "CIDFontType", Arg::Number),
    (1234, "CIDCount", Arg::Number),
    (15, "charset", Arg::Number),
    (1235, "UIDBase", Arg::Number),
    (16, "Encoding", Arg::Number),
    (18, "Private", Arg::Pair),
    (1237, "FDSelect", Arg::Number),
    (1236, "FDArray", Arg::Number),
    (17, "CharStrings", Arg::Number),
    (24, "VarStore", Arg::Number),
];

const TOP_ORDER_CFF2: &[&str] = &["maxstack", "FontMatrix", "FDSelect", "FDArray", "CharStrings", "VarStore"];
const FD_ORDER_CFF: &[&str] = &["FontName", "FontMatrix", "Weight", "Private"];
const FD_ORDER_CFF2: &[&str] = &["Private"];

/// fontTools' `privateDictOperators`, used to read the Private DICTs of both versions.
const PRIVATE_OPS: &[(u16, &str, Arg)] = &[
    (22, "vsindex", Arg::Number),
    (23, "blend", Arg::BlendList),
    (6, "BlueValues", Arg::Delta),
    (7, "OtherBlues", Arg::Delta),
    (8, "FamilyBlues", Arg::Delta),
    (9, "FamilyOtherBlues", Arg::Delta),
    (1209, "BlueScale", Arg::Number),
    (1210, "BlueShift", Arg::Number),
    (1211, "BlueFuzz", Arg::Number),
    (10, "StdHW", Arg::Number),
    (11, "StdVW", Arg::Number),
    (1212, "StemSnapH", Arg::Delta),
    (1213, "StemSnapV", Arg::Delta),
    (1214, "ForceBold", Arg::Number),
    (1215, "ForceBoldThreshold", Arg::Number),
    (1216, "lenIV", Arg::Number),
    (1217, "LanguageGroup", Arg::Number),
    (1218, "ExpansionFactor", Arg::Number),
    (1219, "initialRandomSeed", Arg::Number),
    (20, "defaultWidthX", Arg::Number),
    (21, "nominalWidthX", Arg::Number),
    (19, "Subrs", Arg::Number),
];

const PRIVATE_ORDER_CFF2: &[&str] = &[
    "vsindex",
    "blend",
    "BlueValues",
    "OtherBlues",
    "FamilyBlues",
    "FamilyOtherBlues",
    "BlueScale",
    "BlueShift",
    "BlueFuzz",
    "StdHW",
    "StdVW",
    "StemSnapH",
    "StemSnapV",
    "LanguageGroup",
    "ExpansionFactor",
    "Subrs",
];

fn int(v: i64) -> Value {
    Value::Number(It::N(Num::Int(v)))
}

fn real(v: f64) -> Value {
    Value::Number(It::N(Num::Real(v)))
}

fn top_default(name: &str, cff2: bool) -> Option<Value> {
    let font_matrix = || {
        Value::Array(vec![
            It::N(Num::Real(0.001)),
            It::N(Num::Int(0)),
            It::N(Num::Int(0)),
            It::N(Num::Real(0.001)),
            It::N(Num::Int(0)),
            It::N(Num::Int(0)),
        ])
    };
    if cff2 {
        return (name == "FontMatrix").then(font_matrix);
    }
    Some(match name {
        "isFixedPitch" | "ItalicAngle" | "PaintType" | "StrokeWidth" | "CIDFontVersion" | "CIDFontRevision" | "CIDFontType" | "Encoding" => int(0),
        "UnderlinePosition" => int(-100),
        "UnderlineThickness" => int(50),
        "CharstringType" => int(2),
        "FontMatrix" => font_matrix(),
        "FontBBox" => Value::Array(vec![It::N(Num::Int(0)); 4]),
        "CIDCount" => int(8720),
        _ => return None,
    })
}

fn private_default(name: &str, cff2: bool) -> Option<Value> {
    Some(match name {
        "BlueScale" => real(0.039625),
        "BlueShift" => int(7),
        "BlueFuzz" => int(1),
        "LanguageGroup" => int(0),
        "ExpansionFactor" => real(0.06),
        "ForceBold" | "initialRandomSeed" | "defaultWidthX" | "nominalWidthX" if !cff2 => int(0),
        _ => return None,
    })
}

/// A decoded DICT: the keys in the order first seen, later values replacing earlier ones.
#[derive(Clone, Debug, Default)]
struct Dict {
    entries: Vec<(&'static str, Value)>,
}

impl Dict {
    fn get(&self, name: &str) -> Option<&Value> {
        self.entries.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }

    fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    fn set(&mut self, name: &'static str, value: Value) {
        match self.entries.iter_mut().find(|(n, _)| *n == name) {
            Some(entry) => entry.1 = value,
            None => self.entries.push((name, value)),
        }
    }

    /// The keys not in `order`, sorted, as fontTools names them in a comment.
    fn ignored(&self, order: &[&str]) -> Option<String> {
        let mut names: Vec<&str> = self.entries.iter().map(|(n, _)| *n).filter(|n| !order.contains(n)).collect();
        names.sort_unstable();
        names.dedup();
        (!names.is_empty()).then(|| format!("some keys were ignored: {}", names.join(" ")))
    }
}

/// The String INDEX: SIDs past the 391 standard strings.
struct Strings {
    custom: Vec<String>,
}

impl Strings {
    fn get(&self, sid: i64) -> Result<String> {
        let standard = STANDARD_STRINGS.len() as i64;
        if sid < standard {
            // A negative SID counts from the end of the standard strings, as a Python list
            // index does.
            let at = if sid < 0 { sid.checked_add(standard) } else { Some(sid) };
            let s = at.and_then(|i| usize::try_from(i).ok()).and_then(|i| STANDARD_STRINGS.get(i));
            return s.map(|s| (*s).to_string()).ok_or_else(|| bad("a string id is out of range"));
        }
        let at = usize::try_from(sid - standard).map_err(|_| bad("a string id is out of range"))?;
        self.custom.get(at).cloned().ok_or_else(|| bad("a string id is out of range"))
    }
}

/// fontTools' `DictDecompiler`. `regions(vsindex)` gives the region count a `blend` needs.
fn decode_dict(data: &[u8], ops: &[(u16, &'static str, Arg)], strings: Option<&Strings>, vstore: Option<&VarStore>) -> Result<Dict> {
    let mut dict = Dict::default();
    let mut stack: Vec<It> = Vec::new();
    let mut i = 0usize;
    let byte = |at: usize| data.get(at).copied().map(i64::from).ok_or_else(|| bad("a DICT is cut short"));
    while let Some(&b0) = data.get(i) {
        i += 1;
        match b0 {
            28 => {
                let v = (byte(i)? << 8 | byte(i + 1)?) as u16 as i16;
                stack.push(It::N(Num::Int(i64::from(v))));
                i += 2;
            }
            29 => {
                let b = data.get(i..i + 4).ok_or_else(|| bad("a DICT is cut short"))?;
                let v = b.iter().fold(0u32, |acc, x| acc << 8 | u32::from(*x)) as i32;
                stack.push(It::N(Num::Int(i64::from(v))));
                i += 4;
            }
            30 => {
                let (v, next) = read_real(data, i)?;
                stack.push(It::N(Num::Real(v)));
                i = next;
            }
            32..=246 => stack.push(It::N(Num::Int(i64::from(b0) - 139))),
            247..=250 => {
                stack.push(It::N(Num::Int((i64::from(b0) - 247) * 256 + byte(i)? + 108)));
                i += 1;
            }
            251..=254 => {
                stack.push(It::N(Num::Int(-(i64::from(b0) - 251) * 256 - byte(i)? - 108)));
                i += 1;
            }
            255 => return Err(bad("a DICT uses the reserved operand byte 255")),
            _ => {
                let op = if b0 == 12 {
                    let b1 = byte(i)?;
                    i += 1;
                    1200 + b1 as u16
                } else {
                    u16::from(b0)
                };
                // An operator the dict does not know leaves its operands for the next one.
                if let Some(&(_, name, arg)) = ops.iter().find(|(o, _, _)| *o == op) {
                    handle_operator(&mut dict, &mut stack, name, arg, strings, vstore)?;
                }
            }
        }
    }
    if !stack.is_empty() {
        return Err(bad("a DICT ends with operands left on its stack"));
    }
    Ok(dict)
}

/// A real operand: nibbles as text, read as Python's `float()` reads it.
fn read_real(data: &[u8], mut i: usize) -> Result<(f64, usize)> {
    const NIBBLES: [&str; 15] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", ".", "E", "E-", "", "-"];
    let mut text = String::new();
    loop {
        let b = *data.get(i).ok_or_else(|| bad("a real number is cut short"))?;
        i += 1;
        let mut done = false;
        for nibble in [b >> 4, b & 0x0F] {
            if nibble == 0x0F {
                done = true;
                break;
            }
            if nibble == 0x0D {
                return Err(bad("a real number holds a reserved nibble"));
            }
            text.push_str(NIBBLES.get(usize::from(nibble)).copied().unwrap_or(""));
        }
        if done {
            break;
        }
    }
    let v: f64 = text.parse().map_err(|_| bad("a real number cannot be read"))?;
    Ok((v, i))
}

fn handle_operator(
    dict: &mut Dict,
    stack: &mut Vec<It>,
    name: &'static str,
    arg: Arg,
    strings: Option<&Strings>,
    vstore: Option<&VarStore>,
) -> Result<()> {
    let value = match arg {
        Arg::Number => Value::Number(arg_number(stack)?),
        Arg::Sid => Value::Sid(arg_sid(stack, strings)?),
        Arg::Array => Value::Array(std::mem::take(stack)),
        Arg::Delta => Value::Array(arg_delta(stack)?),
        Arg::Ros => {
            let supplement = arg_number(stack)?;
            let order = arg_sid(stack, strings)?;
            let registry = arg_sid(stack, strings)?;
            Value::Ros(registry, order, supplement)
        }
        Arg::Pair => {
            let second = arg_number(stack)?;
            let first = arg_number(stack)?;
            Value::Pair(first, second)
        }
        Arg::BlendList => {
            let lists = arg_blend_list(dict, stack, vstore)?;
            stack.extend(lists.into_iter().map(It::B));
            return Ok(());
        }
    };
    dict.set(name, value);
    Ok(())
}

fn arg_number(stack: &mut Vec<It>) -> Result<It> {
    let empty = || bad("a DICT operator has too few operands");
    match stack.first() {
        None => Err(empty()),
        // After a blend: the last list is the value, the rest is dropped.
        Some(It::B(_)) => {
            let value = stack.pop().ok_or_else(empty)?;
            stack.clear();
            Ok(value)
        }
        Some(It::N(_)) => stack.pop().ok_or_else(empty),
    }
}

fn arg_sid(stack: &mut Vec<It>, strings: Option<&Strings>) -> Result<String> {
    let sid = stack.pop().ok_or_else(|| bad("a DICT operator has too few operands"))?;
    let sid = sid.int().ok_or_else(|| bad("a string id is not an integer"))?;
    strings.ok_or_else(|| bad("a CFF2 DICT holds a string"))?.get(sid)
}

fn arg_delta(stack: &mut Vec<It>) -> Result<Vec<It>> {
    let values = std::mem::take(stack);
    if matches!(values.first(), Some(It::B(_))) {
        return Ok(values);
    }
    let mut current = Num::Int(0);
    let mut out = Vec::with_capacity(values.len());
    for v in values {
        let It::N(n) = v else { return Err(bad("a delta array mixes numbers and blends")) };
        current = current.add(n)?;
        out.push(It::N(current));
    }
    Ok(out)
}

/// The `blend` operator of a Private DICT: per blended value, the absolute default followed by
/// its deltas.
fn arg_blend_list(dict: &Dict, stack: &mut Vec<It>, vstore: Option<&VarStore>) -> Result<Vec<Vec<Num>>> {
    let vsindex = match dict.get("vsindex") {
        None => 0,
        Some(v) => v.int().ok_or_else(|| bad("vsindex is not an integer"))?,
    };
    let vstore = vstore.ok_or_else(|| bad("a Private DICT blends values without a variation store"))?;
    let masters = i64::try_from(vstore.regions(vsindex)?).map_err(|_| bad("too many regions"))?.saturating_add(1);
    let blends = stack.pop().and_then(|v| v.int()).ok_or_else(|| bad("a blend count is not an integer"))?;
    let args = std::mem::take(stack);
    if masters.checked_mul(blends) != Some(args.len() as i64) {
        return Err(bad("a blend has the wrong number of operands"));
    }
    let mut nums = Vec::with_capacity(args.len());
    for a in args {
        let It::N(n) = a else { return Err(bad("a blend operand is a list")) };
        nums.push(n);
    }
    let blends = usize::try_from(blends).unwrap_or(0);
    let deltas = usize::try_from(masters - 1).unwrap_or(0);
    let mut out = Vec::with_capacity(blends);
    let mut prev = Num::Int(0);
    for i in 0..blends {
        let value = nums.get(i).copied().ok_or_else(|| bad("a blend is cut short"))?.add(prev)?;
        prev = value;
        let start = blends + i * deltas;
        let mut list = vec![value];
        list.extend_from_slice(nums.get(start..start + deltas).ok_or_else(|| bad("a blend is cut short"))?);
        out.push(list);
    }
    Ok(out)
}

// ── INDEX ──────────────────────────────────────────────────────────────────────────────────────

/// A CFF INDEX: its items are read when asked for, as fontTools reads them.
#[derive(Clone, Copy, Debug)]
pub(super) struct Index {
    count: usize,
    off_size: usize,
    offsets_at: usize,
    /// The position after the INDEX.
    end: usize,
}

fn read_index(data: &[u8], pos: usize, cff2: bool) -> Result<Index> {
    let mut r = Rd::at(data, pos);
    let count = if cff2 { r.u32()? as usize } else { usize::from(r.u16()?) };
    let after_count = pos + if cff2 { 4 } else { 2 };
    if count == 0 {
        return Ok(Index { count: 0, off_size: 0, offsets_at: after_count, end: after_count });
    }
    let off_size = usize::from(r.u8()?);
    if off_size > 4 {
        return Err(bad("an INDEX has an offset size above 4"));
    }
    if count > data.len() {
        return Err(bad("an INDEX counts more items than the table has bytes"));
    }
    let offsets_at = after_count + 1;
    let offsets_len = (count + 1).checked_mul(off_size).ok_or_else(|| bad("an INDEX is too large"))?;
    if offsets_at.checked_add(offsets_len).is_none_or(|end| end > data.len()) {
        return Err(bad("an INDEX is cut short"));
    }
    let mut index = Index { count, off_size, offsets_at, end: 0 };
    index.end = index.base().checked_add(index.offset(data, count)?).ok_or_else(|| bad("an INDEX is too large"))?;
    Ok(index)
}

impl Index {
    pub(super) fn count(&self) -> usize {
        self.count
    }

    /// Where offset 0 points: the last byte of the offset array.
    fn base(&self) -> usize {
        (self.offsets_at + (self.count + 1) * self.off_size).saturating_sub(1)
    }

    fn offset(&self, data: &[u8], i: usize) -> Result<usize> {
        let at = self.offsets_at + i * self.off_size;
        let bytes = data.get(at..at + self.off_size).ok_or_else(|| bad("an INDEX is cut short"))?;
        Ok(bytes.iter().fold(0usize, |acc, b| acc << 8 | usize::from(*b)))
    }

    /// Item `i`; an error when its offsets are out of order or past the end of the data.
    pub(super) fn item<'a>(&self, data: &'a [u8], i: usize) -> Result<&'a [u8]> {
        if i >= self.count {
            return Err(bad("an INDEX item is out of range"));
        }
        let (start, end) = (self.offset(data, i)?, self.offset(data, i + 1)?);
        if end < start {
            return Err(bad("an INDEX has offsets out of order"));
        }
        let base = self.base();
        data.get(base + start..base + end).ok_or_else(|| bad("an INDEX item is cut short"))
    }
}

// ── The table ──────────────────────────────────────────────────────────────────────────────────

/// A Private DICT with its local subroutines.
struct Private<'a> {
    dict: Dict,
    subrs: Option<SubrSet<'a>>,
    /// Whether its dict was read with the font's variation store (the private dicts of an
    /// FDArray are; the Top DICT's is not).
    with_vstore: bool,
}

struct FontDict<'a> {
    dict: Dict,
    private: Option<Private<'a>>,
}

/// The glyph names the charstrings are keyed by, or `None` where fontTools finds no charset
/// and leaves the charstrings out.
type Charset = Option<Vec<String>>;

struct Cff<'a> {
    data: &'a [u8],
    cff2: bool,
    major: u8,
    minor: u8,
    name: String,
    top: Dict,
    strings: Option<Strings>,
    globals: SubrSet<'a>,
    vstore: Option<VarStore>,
    private: Option<Private<'a>>,
    fd_array: Option<Vec<FontDict<'a>>>,
    fd_select: Option<(u8, Vec<Option<usize>>)>,
    charset: Charset,
    charstrings: Option<Index>,
    programs: Vec<Rc<Program>>,
}

fn cff(data: &[u8], font: &SfntFont, w: &mut XmlWriter, cff2: bool) -> Result<()> {
    let mut cff = read_cff(data, font, w, cff2)?;
    decompile_all(&mut cff)?;
    write_cff(&cff, w)
}

fn read_cff<'a>(data: &'a [u8], font: &SfntFont, w: &mut XmlWriter, cff2: bool) -> Result<Cff<'a>> {
    let mut r = Rd::new(data);
    let major = r.u8()?;
    let minor = r.u8()?;
    let header_size = usize::from(r.u8()?);
    if major != if cff2 { 2 } else { 1 } {
        return Err(bad("the major version does not match the table"));
    }
    let (name, top_data, strings, globals_at) = if cff2 {
        let top_size = usize::from(r.u16()?);
        let top = data.get(header_size..header_size + top_size).ok_or_else(|| bad("the Top DICT is cut short"))?;
        ("CFF2Font".to_string(), top, None, header_size + top_size)
    } else {
        r.u8()?;
        let names = read_index(data, header_size, false)?;
        if names.count != 1 {
            return Err(bad("a CFF table must hold exactly one font"));
        }
        let name = names.item(data, 0)?;
        if !name.is_ascii() {
            return Err(bad("the font name is not ASCII"));
        }
        let name: String = name.iter().map(|&b| char::from(b)).collect();
        let tops = read_index(data, names.end, false)?;
        let strings_index = read_index(data, tops.end, false)?;
        let mut custom = Vec::with_capacity(strings_index.count);
        for i in 0..strings_index.count {
            custom.push(strings_index.item(data, i)?.iter().map(|&b| char::from(b)).collect());
        }
        let top = tops.item(data, 0)?;
        (name, top, Some(Strings { custom }), strings_index.end)
    };
    let globals = SubrSet::new(data, read_index(data, globals_at, cff2)?);
    let top = decode_dict(top_data, TOP_OPS, strings.as_ref(), None)?;

    let offset = |name: &str| -> Result<Option<usize>> {
        match top.get(name) {
            None => Ok(None),
            Some(v) => {
                let v = v.int().ok_or_else(|| bad("an offset is not an integer"))?;
                usize::try_from(v).map(Some).map_err(|_| bad("an offset is negative"))
            }
        }
    };
    let charstrings = offset("CharStrings")?.map(|at| read_index(data, at, cff2)).transpose()?;
    let num_glyphs = charstrings.map(|c| c.count);

    let vstore = match offset("VarStore")? {
        Some(at) => Some(varstore::read(data, at)?),
        None => None,
    };
    let fd_array = match offset("FDArray")? {
        Some(at) => {
            let index = read_index(data, at, cff2)?;
            let mut fds = Vec::new();
            for i in 0..index.count {
                let dict = decode_dict(index.item(data, i)?, TOP_OPS, strings.as_ref(), None)?;
                let private = match dict.get("Private") {
                    Some(v) => Some(read_private(data, v, vstore.as_ref(), cff2)?),
                    None => None,
                };
                fds.push(FontDict { dict, private });
                w.check()?;
            }
            Some(fds)
        }
        None => None,
    };
    let fd_select = match (offset("FDSelect")?, num_glyphs) {
        (Some(at), Some(n)) => Some(read_fd_select(data, at, n)?),
        _ => None,
    };
    // A CFF2 font with an FDArray never reads a Private DICT from its Top DICT.
    let private = match top.get("Private") {
        Some(v) if !cff2 || fd_array.is_none() => Some(read_private(data, v, None, cff2)?),
        _ => None,
    };
    let charset = if cff2 { Some(w.glyph_order(font)?.as_ref().clone()) } else { read_charset(data, &top, strings.as_ref(), num_glyphs)? };
    Ok(Cff {
        data,
        cff2,
        major,
        minor,
        name,
        top,
        strings,
        globals,
        vstore,
        private,
        fd_array,
        fd_select,
        charset,
        charstrings,
        programs: Vec::new(),
    })
}

fn read_private<'a>(data: &'a [u8], value: &Value, vstore: Option<&VarStore>, cff2: bool) -> Result<Private<'a>> {
    let Value::Pair(size, offset) = value else { return Err(bad("the Private DICT entry is not a pair")) };
    let size = size.int().and_then(|v| usize::try_from(v).ok()).ok_or_else(|| bad("the Private DICT size is not valid"))?;
    let offset = offset.int().and_then(|v| usize::try_from(v).ok()).ok_or_else(|| bad("the Private DICT offset is not valid"))?;
    let bytes = data.get(offset..offset.saturating_add(size)).ok_or_else(|| bad("the Private DICT is cut short"))?;
    let dict = decode_dict(bytes, PRIVATE_OPS, None, vstore)?;
    let subrs = match dict.get("Subrs") {
        Some(v) => {
            let rel = v.int().ok_or_else(|| bad("the Subrs offset is not an integer"))?;
            let at = i64::try_from(offset).ok().and_then(|o| o.checked_add(rel)).and_then(|a| usize::try_from(a).ok());
            let at = at.ok_or_else(|| bad("the Subrs offset is out of range"))?;
            Some(SubrSet::new(data, read_index(data, at, cff2)?))
        }
        None => None,
    };
    Ok(Private { dict, subrs, with_vstore: vstore.is_some() })
}

/// The FDSelect format and the font dict of each glyph (`None` where no range covers it).
fn read_fd_select(data: &[u8], at: usize, num_glyphs: usize) -> Result<(u8, Vec<Option<usize>>)> {
    let mut r = Rd::at(data, at);
    let format = r.u8()?;
    let mut fds: Vec<Option<usize>> = Vec::new();
    match format {
        0 => {
            // fontTools reads what is there; a glyph past the end has no font dict.
            let bytes = r.rest();
            fds.extend(bytes.iter().take(num_glyphs).map(|&b| Some(usize::from(b))));
        }
        3 | 4 => {
            fds = vec![None; num_glyphs];
            let ranges = if format == 3 { usize::from(r.u16()?) } else { r.u32()? as usize };
            let mut prev: Option<(usize, usize)> = None;
            for _ in 0..ranges {
                let first = glyph_id(&mut r, format)?;
                if let Some((start, fd)) = prev {
                    fill(&mut fds, start, first, fd)?;
                }
                let fd = if format == 3 { usize::from(r.u8()?) } else { usize::from(r.u16()?) };
                prev = Some((first, fd));
            }
            if let Some((start, fd)) = prev {
                let sentinel = glyph_id(&mut r, format)?;
                fill(&mut fds, start, sentinel, fd)?;
            }
        }
        _ => return Err(bad("the FDSelect format is not known")),
    }
    Ok((format, fds))
}

/// A glyph id of an FDSelect range: 16-bit in format 3, 32-bit in format 4.
fn glyph_id(r: &mut Rd, format: u8) -> Result<usize> {
    if format == 3 { Ok(usize::from(r.u16()?)) } else { Ok(r.u32()? as usize) }
}

fn fill(fds: &mut [Option<usize>], start: usize, end: usize, fd: usize) -> Result<()> {
    for gid in start..end {
        *fds.get_mut(gid).ok_or_else(|| bad("an FDSelect range runs past the last glyph"))? = Some(fd);
    }
    Ok(())
}

/// The charset of a CFF (version 1) Top DICT, as fontTools reads it: `None` when there is none
/// to read.
fn read_charset(data: &[u8], top: &Dict, strings: Option<&Strings>, num_glyphs: Option<usize>) -> Result<Charset> {
    let Some(value) = top.get("charset") else { return Ok(None) };
    let Some(num_glyphs) = num_glyphs else { return Ok(None) };
    let is_cid = top.has("ROS");
    let Value::Number(It::N(value)) = value else { return Err(bad("the charset offset is not a number")) };
    let offset = match value {
        Num::Int(v) if *v > 2 => usize::try_from(*v).map_err(|_| bad("the charset offset is out of range"))?,
        v if is_cid => {
            return Err(if v.is_zero() { bad("a CID-keyed font has no charset") } else { bad("a CID-keyed font names a predefined charset") });
        }
        v if v.is_zero() => {
            let names = STANDARD_STRINGS.iter().take(229.min(num_glyphs)).map(|s| (*s).to_string()).collect();
            return Ok(Some(names));
        }
        Num::Int(1 | 2) => return Err(TablesError::Unsupported("the predefined expert charsets of CFF".into())),
        _ => return Err(bad("the charset offset is not valid")),
    };
    let strings = strings.ok_or_else(|| bad("no String INDEX"))?;
    let name_of = |id: usize| -> Result<String> { if is_cid { Ok(format!("cid{id:05}")) } else { strings.get(id as i64) } };
    let mut r = Rd::at(data, offset);
    let format = r.u8()?;
    let mut names = vec![".notdef".to_string()];
    match format {
        0 => {
            for _ in 1..num_glyphs {
                names.push(name_of(usize::from(r.u16()?))?);
            }
        }
        1 | 2 => {
            while names.len() < num_glyphs {
                let first = usize::from(r.u16()?);
                let left = if format == 1 { usize::from(r.u8()?) } else { usize::from(r.u16()?) };
                if names.len() + left >= num_glyphs {
                    return Err(bad("the charset names more glyphs than the font has"));
                }
                for id in first..=first + left {
                    names.push(name_of(id)?);
                }
            }
        }
        _ => return Err(bad("the charset format is not known")),
    }
    if names.len() != num_glyphs {
        return Err(bad("the charset does not name every glyph"));
    }
    Ok(Some(unique_names(names, ".", true)))
}

impl Cff<'_> {
    /// Whether fontTools finds the charstrings: it needs their offset, a charset, and a
    /// Private DICT or an FDArray.
    fn has_charstrings(&self) -> bool {
        self.charstrings.is_some() && self.charset.is_some() && (self.fd_array.is_some() || self.private.is_some())
    }

    /// The font dict of glyph `gid` and whether the charstrings carry it.
    fn fd_of(&self, gid: usize) -> Result<Option<usize>> {
        if self.fd_array.is_none() {
            return Ok(None);
        }
        match &self.fd_select {
            None => Ok(Some(0)),
            Some((_, fds)) => fds.get(gid).copied().flatten().map(Some).ok_or_else(|| bad("a glyph has no font dict")),
        }
    }
}

/// Decompiles every charstring in glyph order, and the subroutines they call.
fn decompile_all(cff: &mut Cff) -> Result<()> {
    if !cff.has_charstrings() {
        return Ok(());
    }
    let Some(index) = cff.charstrings else { return Ok(()) };
    let mut budget = Budget::new();
    let mut programs = Vec::with_capacity(index.count);
    for gid in 0..index.count {
        let code = index.item(cff.data, gid)?;
        let fd = cff.fd_of(gid)?;
        let private = match (fd, cff.fd_array.as_mut()) {
            (Some(fd), Some(fds)) => {
                let font_dict = fds.get_mut(fd).ok_or_else(|| bad("a glyph's font dict is out of range"))?;
                font_dict.private.as_mut().ok_or_else(|| bad("a font dict has no Private DICT"))?
            }
            _ => cff.private.as_mut().ok_or_else(|| bad("the font has no Private DICT"))?,
        };
        let vstore = if private.with_vstore { cff.vstore.as_ref() } else { None };
        let env = Env { vstore, private_vsindex: private.dict.get("vsindex").map(|v| v.int().ok_or(())) };
        let mut empty = SubrSet::empty(cff.data);
        let locals = private.subrs.as_mut().unwrap_or(&mut empty);
        let program = charstring::decompile(code, locals, &mut cff.globals, &env, &mut budget)?;
        programs.push(Rc::new(program));
    }
    cff.programs = programs;
    Ok(())
}

// ── Writing ────────────────────────────────────────────────────────────────────────────────────

fn write_cff(cff: &Cff, w: &mut XmlWriter) -> Result<()> {
    w.simple("major", &[("value", &cff.major.to_string())]);
    w.simple("minor", &[("value", &cff.minor.to_string())]);
    w.open("CFFFont", &[("name", &cff.name)]);
    write_top(cff, w)?;
    w.close("CFFFont");
    w.blank();
    w.open("GlobalSubrs", &[]);
    write_subrs(&cff.globals, w)?;
    w.close("GlobalSubrs");
    Ok(())
}

fn write_top(cff: &Cff, w: &mut XmlWriter) -> Result<()> {
    let top_order: Vec<&str> = if cff.cff2 { TOP_ORDER_CFF2.to_vec() } else { TOP_OPS.iter().map(|(_, n, _)| *n).collect() };
    let is_cid = cff.top.has("ROS");
    let has_charstrings = cff.has_charstrings();
    let mut skip: &[&str] = &[];
    if is_cid {
        skip = &["Encoding"];
    }
    if !is_cid || !has_charstrings {
        skip = &["CIDFontVersion", "CIDFontRevision", "CIDFontType", "CIDCount"];
    }
    for &name in &top_order {
        if skip.contains(&name) {
            continue;
        }
        match name {
            "charset" => w.comment("charset is dumped separately as the 'GlyphOrder' element"),
            "Encoding" => write_encoding(cff, w)?,
            "Private" => {
                if let Some(p) = &cff.private {
                    write_private(p, cff.cff2, w)?;
                }
            }
            "FDSelect" => {
                if let Some((format, _)) = &cff.fd_select {
                    w.simple("FDSelect", &[("format", &format.to_string())]);
                }
            }
            "FDArray" => {
                if let Some(fds) = &cff.fd_array {
                    write_fd_array(fds, cff.cff2, w)?;
                }
            }
            "CharStrings" => {
                if has_charstrings {
                    write_charstrings(cff, w)?;
                }
            }
            "VarStore" => {
                if let Some(vstore) = &cff.vstore {
                    vstore.write(w);
                }
            }
            _ => {
                if let Some(value) = cff.top.get(name).cloned().or_else(|| top_default(name, cff.cff2)) {
                    write_value(w, name, &value)?;
                }
            }
        }
        w.check()?;
    }
    if let Some(comment) = cff.top.ignored(&top_order) {
        w.comment(&comment);
    }
    Ok(())
}

fn write_value(w: &mut XmlWriter, name: &str, value: &Value) -> Result<()> {
    match value {
        Value::Number(It::B(list)) => write_blends(w, name, std::slice::from_ref(list)),
        Value::Number(It::N(n)) => w.simple(name, &[("value", &n.py_str())]),
        Value::Array(items) if matches!(items.first(), Some(It::B(_))) => {
            let mut lists = Vec::with_capacity(items.len());
            for item in items {
                let It::B(list) = item else { return Err(bad("a blended array mixes numbers and blends")) };
                lists.push(list.clone());
            }
            write_blends(w, name, &lists);
        }
        Value::Array(items) => {
            let text = items.iter().map(It::py_str).collect::<Vec<_>>().join(" ");
            w.simple(name, &[("value", &text)]);
        }
        Value::Sid(s) => {
            let s = if name == "Notice" || name == "Copyright" { collapse_breaks(s) } else { s.clone() };
            w.simple(name, &[("value", &s)]);
        }
        Value::Ros(registry, order, supplement) => {
            w.simple(name, &[("Registry", registry), ("Order", order), ("Supplement", &supplement.py_str())]);
        }
        Value::Pair(a, b) => w.simple(name, &[("value", &format!("({}, {})", a.py_str(), b.py_str()))]),
    }
    Ok(())
}

/// A blended value: one `blend` element per value, two levels deeper, as fontTools indents it.
fn write_blends(w: &mut XmlWriter, name: &str, lists: &[Vec<Num>]) {
    w.open(name, &[]);
    w.depth += 1;
    for list in lists {
        let text = list.iter().map(|n| n.py_str()).collect::<Vec<_>>().join(" ");
        w.simple("blend", &[("value", &text)]);
    }
    w.depth = w.depth.saturating_sub(1);
    w.close(name);
}

/// fontTools' `re.sub(r"[\r\n]\s+", " ", value)` on `Notice` and `Copyright`.
fn collapse_breaks(s: &str) -> String {
    // Python's `\s` on text: the characters `str.isspace()` accepts (Latin-1 range shown).
    let space = |c: char| {
        matches!(c, '\t' | '\n' | '\x0B' | '\x0C' | '\r' | '\x1C'..='\x1F' | ' ' | '\u{85}' | '\u{A0}') || (c > '\u{FF}' && c.is_whitespace())
    };
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if (c == '\r' || c == '\n') && chars.get(i + 1).is_some_and(|&n| space(n)) {
            out.push(' ');
            i += 1;
            while chars.get(i).is_some_and(|&n| space(n)) {
                i += 1;
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn write_encoding(cff: &Cff, w: &mut XmlWriter) -> Result<()> {
    let value = cff.top.get("Encoding").cloned().unwrap_or_else(|| int(0));
    let Value::Number(It::N(v)) = value else { return Err(bad("the Encoding offset is not a number")) };
    if v.is_zero() {
        w.simple("Encoding", &[("name", "StandardEncoding")]);
        return Ok(());
    }
    if v.as_f64() == 1.0 {
        w.simple("Encoding", &[("name", "ExpertEncoding")]);
        return Ok(());
    }
    let Num::Int(offset) = v else { return Err(bad("the Encoding offset is not valid")) };
    let offset = usize::try_from(offset).ok().filter(|&o| o > 1).ok_or_else(|| bad("the Encoding offset is not valid"))?;
    // fontTools reads a custom encoding through the charset; without one it leaves the
    // encoding out.
    let Some(charset) = &cff.charset else { return Ok(()) };
    let mut r = Rd::at(cff.data, offset);
    let format = r.u8()?;
    let mut codes: Vec<Option<String>> = vec![None; 256];
    let glyph = |gid: usize| charset.get(gid).cloned().ok_or_else(|| bad("the encoding names a glyph past the charset"));
    match format & 0x7F {
        0 => {
            let n = usize::from(r.u8()?);
            for gid in 1..=n {
                let code = usize::from(r.u8()?);
                if code != 0
                    && let Some(slot) = codes.get_mut(code)
                {
                    *slot = Some(glyph(gid)?);
                }
            }
        }
        1 => {
            let ranges = r.u8()?;
            let mut gid = 1usize;
            for _ in 0..ranges {
                let first = usize::from(r.u8()?);
                let left = usize::from(r.u8()?);
                for code in first..=first + left {
                    let name = glyph(gid)?;
                    *codes.get_mut(code).ok_or_else(|| bad("an encoding range runs past code 255"))? = Some(name);
                    gid += 1;
                }
            }
        }
        _ => return Err(bad("the Encoding format is not known")),
    }
    if format & 0x80 != 0 {
        let strings = cff.strings.as_ref().ok_or_else(|| bad("no String INDEX"))?;
        for _ in 0..r.u8()? {
            let code = usize::from(r.u8()?);
            let sid = i64::from(r.u16()?);
            if let Some(slot) = codes.get_mut(code) {
                *slot = Some(strings.get(sid)?);
            }
        }
    }
    w.open("Encoding", &[]);
    for (code, name) in codes.iter().enumerate() {
        if let Some(name) = name
            && name != ".notdef"
        {
            w.simple("map", &[("code", &format!("0x{code:x}")), ("name", name)]);
        }
    }
    w.close("Encoding");
    Ok(())
}

fn write_private(p: &Private, cff2: bool, w: &mut XmlWriter) -> Result<()> {
    let order: Vec<&str> = if cff2 { PRIVATE_ORDER_CFF2.to_vec() } else { PRIVATE_OPS.iter().map(|(_, n, _)| *n).collect() };
    w.open("Private", &[]);
    for &name in &order {
        if name == "Subrs" {
            if let Some(subrs) = &p.subrs {
                w.open("Subrs", &[]);
                write_subrs(subrs, w)?;
                w.close("Subrs");
            }
            continue;
        }
        if let Some(value) = p.dict.get(name).cloned().or_else(|| private_default(name, cff2)) {
            write_value(w, name, &value)?;
        }
    }
    if let Some(comment) = p.dict.ignored(&order) {
        w.comment(&comment);
    }
    w.close("Private");
    Ok(())
}

fn write_fd_array(fds: &[FontDict], cff2: bool, w: &mut XmlWriter) -> Result<()> {
    let order = if cff2 { FD_ORDER_CFF2 } else { FD_ORDER_CFF };
    w.open("FDArray", &[]);
    for (i, fd) in fds.iter().enumerate() {
        w.open("FontDict", &[("index", &i.to_string())]);
        for &name in order {
            if name == "Private" {
                if let Some(p) = &fd.private {
                    write_private(p, cff2, w)?;
                }
            } else if let Some(value) = fd.dict.get(name) {
                write_value(w, name, value)?;
            }
        }
        if let Some(comment) = fd.dict.ignored(order) {
            w.comment(&comment);
        }
        w.close("FontDict");
        w.check()?;
    }
    w.close("FDArray");
    Ok(())
}

fn write_subrs(subrs: &SubrSet, w: &mut XmlWriter) -> Result<()> {
    w.comment("The 'index' attribute is only for humans; it is ignored when parsed.");
    for i in 0..subrs.count() {
        let index = i.to_string();
        match subrs.program(i) {
            Some(program) => {
                w.open("CharString", &[("index", &index)]);
                close_program(w, program);
            }
            None => {
                let bytes = subrs.item(i)?;
                w.open("CharString", &[("index", &index), ("raw", "1")]);
                w.hex(bytes);
                w.close("CharString");
            }
        }
        w.check()?;
    }
    Ok(())
}

fn write_charstrings(cff: &Cff, w: &mut XmlWriter) -> Result<()> {
    let Some(names) = &cff.charset else { return Ok(()) };
    let with_fd = cff.fd_array.is_some() && cff.fd_select.is_some();
    let mut sorted: Vec<(&str, usize)> = names.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
    sorted.sort_unstable();
    w.open("CharStrings", &[]);
    for (name, gid) in sorted {
        let program = cff.programs.get(gid).ok_or_else(|| bad("the glyph order names more glyphs than the charstrings hold"))?;
        if with_fd {
            let fd = cff.fd_of(gid)?.unwrap_or(0);
            w.open("CharString", &[("name", name), ("fdSelectIndex", &fd.to_string())]);
        } else {
            w.open("CharString", &[("name", name)]);
        }
        close_program(w, program);
        w.check()?;
    }
    w.close("CharStrings");
    Ok(())
}

/// Writes a decompiled charstring into the open `CharString` element and closes it.
fn close_program(w: &mut XmlWriter, program: &Program) {
    match charstring::write_program(w, program) {
        Some(rest) => text_then_close(w, &rest, "CharString"),
        None => w.close("CharString"),
    }
}

/// `text</name>` on one line: fontTools writes operands left after the last operator without a
/// line break, so the end tag follows them.
fn text_then_close(w: &mut XmlWriter, text: &str, name: &str) {
    if w.spent() {
        w.depth = w.depth.saturating_sub(1);
        w.open_elements.pop();
        return;
    }
    let before = w.out.len();
    w.indent();
    escape_into(&mut w.out, text, false);
    w.out.push_str("</");
    w.out.push_str(name);
    w.out.push_str(">\n");
    w.charge_text(before);
    w.add_text(text);
    w.depth = w.depth.saturating_sub(1);
    if let Some(e) = w.open_elements.pop() {
        w.attach(e);
    }
}

// ── VORG ───────────────────────────────────────────────────────────────────────────────────────

fn vorg(data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    let mut r = Rd::new(data);
    let major = r.u16()?;
    let minor = r.u16()?;
    let default = r.i16()?;
    let count = r.u16()?;
    if major > 1 {
        return Err(TablesError::invalid("the 'VORG' table has a major version above 1"));
    }
    let order = w.glyph_order(font)?;
    // Later records for a glyph replace earlier ones.
    let mut records: BTreeMap<String, i16> = BTreeMap::new();
    for _ in 0..count {
        let gid = usize::from(r.u16()?);
        let origin = r.i16()?;
        let name = order.get(gid).cloned().unwrap_or_else(|| format!("glyph{gid:05}"));
        records.insert(name, origin);
    }
    let ids: std::collections::HashMap<&str, usize> = order.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
    let mut sorted = Vec::with_capacity(records.len());
    for (name, origin) in &records {
        let gid = match ids.get(name.as_str()) {
            Some(&gid) => gid,
            None => name
                .strip_prefix("glyph")
                .and_then(|n| n.parse::<usize>().ok())
                .ok_or_else(|| TablesError::invalid("the 'VORG' table names a glyph the font does not have"))?,
        };
        sorted.push((gid, name.as_str(), *origin));
    }
    sorted.sort_unstable();
    w.simple("majorVersion", &[("value", &major.to_string())]);
    w.simple("minorVersion", &[("value", &minor.to_string())]);
    w.simple("defaultVertOriginY", &[("value", &default.to_string())]);
    w.simple("numVertOriginYMetrics", &[("value", &count.to_string())]);
    for (_, name, origin) in sorted {
        w.open("VOriginRecord", &[]);
        w.simple("glyphName", &[("value", name)]);
        w.simple("vOrigin", &[("value", &origin.to_string())]);
        w.close("VOriginRecord");
        w.check()?;
    }
    Ok(())
}
