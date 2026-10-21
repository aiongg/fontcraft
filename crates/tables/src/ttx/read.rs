//! TTX reading (M12.8): the XML parser, the context the reader modules share, and the import
//! that reads each table element and then recalculates what fontTools' compiler recalculates.
//!
//! The text is untrusted. [`parse`] works without recursion and stops past
//! [`MAX_TTX_BYTES`], [`MAX_TTX_DEPTH`] levels and [`MAX_TTX_ELEMENTS`] elements. It reads what
//! expat reads for the documents `ttx` writes: the XML declaration, comments, processing
//! instructions, CDATA sections, the five named entities and character references. Line ends
//! become `\n`; literal tabs and line ends inside attribute values become spaces, as XML
//! requires, so a value that held a line break reads back with a space.
//!
//! [`import`] follows fontTools' `TTFont.importXML` and `TTFont.save` with `recalcBBoxes` on
//! unless [`super::ImportOptions::keep_bounds`] is set, and `recalcTimestamp` off: each table
//! element is compiled on its own (a table element with `raw` or `ERROR`, or one no module
//! knows, from its `hexdata`); then the fields fontTools fills at compile time are filled
//! (`loca` from `glyf`, `head.indexToLocFormat`, `maxp.numGlyphs`, the metric counts of `hhea`
//! and `vhea`, `OS/2.usFirstCharIndex` and `usLastCharIndex`, and with bounds recalculation the
//! glyph and font bounding boxes, the `maxp` maxima, `head.flags` bit 1 and the `hhea` and
//! `vhea` extremes). A font with `maxp`, `post` and `CFF ` gets its `CFF ` table compiled again
//! when the text holds none, as fontTools compiles the table it loaded for the glyph order.
//! Ported from fontTools' `ttLib.ttFont`, `misc.xmlReader`, `misc.textTools`, `misc.timeTools`
//! and `misc.fixedTools` (MIT).

use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use crate::limits::{MAX_TTX_BYTES, MAX_TTX_DEPTH, MAX_TTX_ELEMENTS, MAX_TTX_TREE_BYTES};
use crate::sfnt::{Flavour, SfntFont, Tag};
use crate::{Result, TablesError};

use super::{Element, ImportOptions, glyph_order, read_bitmap, read_cff, read_color, read_core, read_misc, read_otdata, read_var};

// ── Context ────────────────────────────────────────────────────────────────────────────────────

/// What a reader module needs besides its own element: the glyph order the text gives (or the
/// font's), the tables the font will hold, and whether bounds are recalculated.
pub(crate) struct ReadContext {
    order: std::result::Result<Rc<Vec<String>>, TablesError>,
    ids: HashMap<String, u32>,
    present: BTreeSet<Tag>,
    /// fontTools' `recalcBBoxes`.
    pub recalc: bool,
}

impl ReadContext {
    pub(crate) fn new(order: Result<Vec<String>>, present: BTreeSet<Tag>, recalc: bool) -> Self {
        let order = order.map(Rc::new);
        let mut ids = HashMap::new();
        if let Ok(names) = &order {
            for (i, name) in names.iter().enumerate() {
                // A repeated name maps to its last glyph, as in fontTools' reverse glyph map.
                ids.insert(name.clone(), u32::try_from(i).unwrap_or(u32::MAX));
            }
        }
        Self { order, ids, present, recalc }
    }

    /// The context for one table element read against `font` alone.
    pub(crate) fn from_font(font: &SfntFont) -> Self {
        Self::new(glyph_order(font), font.tables.keys().copied().collect(), true)
    }

    /// The glyph order.
    pub(crate) fn order(&self) -> Result<&[String]> {
        match &self.order {
            Ok(names) => Ok(names.as_slice()),
            Err(e) => Err(e.clone()),
        }
    }

    /// fontTools' `getGlyphID`: the glyph's index, or `N` for a name `glyphN` that is not in the
    /// glyph order.
    pub(crate) fn glyph_id(&self, name: &str) -> Result<u32> {
        if let Some(id) = self.ids.get(name) {
            return Ok(*id);
        }
        if let Some(digits) = name.strip_prefix("glyph")
            && !digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
            && let Ok(id) = digits.parse::<u32>()
        {
            return Ok(id);
        }
        Err(TablesError::invalid(format!("the glyph '{name}' is not in the glyph order")))
    }

    /// The index of a name in the glyph order (fontTools' reverse glyph map, without the
    /// `glyphN` names [`Self::glyph_id`] also takes).
    pub(crate) fn known_glyph(&self, name: &str) -> Option<u32> {
        self.ids.get(name).copied()
    }

    /// Whether the font will hold table `tag` (from the text or from the font itself).
    pub(crate) fn has(&self, tag: Tag) -> bool {
        self.present.contains(&tag)
    }
}

// ── Numbers and text as fontTools reads them ───────────────────────────────────────────────────

/// A number as Python's `literal_eval` gives it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Num {
    Int(i64),
    Float(f64),
}

impl Num {
    pub(crate) fn as_f64(self) -> f64 {
        match self {
            Num::Int(v) => v as f64,
            Num::Float(v) => v,
        }
    }

    /// The value rounded as fontTools' `otRound` rounds it.
    pub(crate) fn round(self) -> Result<i64> {
        match self {
            Num::Int(v) => Ok(v),
            Num::Float(v) => ot_round(v),
        }
    }
}

fn bad_number(text: &str) -> TablesError {
    TablesError::invalid(format!("'{text}' is not a number"))
}

/// An integer literal: decimal, `0x`, `0o` or `0b`, with a sign; `True` and `False` as 1 and 0.
pub(crate) fn int_lit(text: &str) -> Result<i64> {
    let t = text.trim();
    match t {
        "True" => return Ok(1),
        "False" => return Ok(0),
        _ => {}
    }
    let (negative, body) = match t.strip_prefix('-') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let body = body.replace('_', "");
    let lower = body.to_ascii_lowercase();
    let value = if let Some(hex) = lower.strip_prefix("0x") {
        i64::from_str_radix(hex, 16)
    } else if let Some(oct) = lower.strip_prefix("0o") {
        i64::from_str_radix(oct, 8)
    } else if let Some(bin) = lower.strip_prefix("0b") {
        i64::from_str_radix(bin, 2)
    } else if !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit()) {
        body.parse::<i64>()
    } else {
        return Err(bad_number(text));
    }
    .map_err(|_| bad_number(text))?;
    Ok(if negative { value.checked_neg().ok_or_else(|| bad_number(text))? } else { value })
}

/// A number literal: an integer, else a finite float.
pub(crate) fn num_lit(text: &str) -> Result<Num> {
    if let Ok(v) = int_lit(text) {
        return Ok(Num::Int(v));
    }
    let t = text.trim();
    let looks_numeric = !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'+' | b'e' | b'E' | b'_'));
    match t.replace('_', "").parse::<f64>() {
        Ok(v) if v.is_finite() && looks_numeric => Ok(Num::Float(v)),
        _ => Err(bad_number(text)),
    }
}

/// A float as Python's `float()` reads it.
pub(crate) fn float_lit(text: &str) -> Result<f64> {
    num_lit(text).map(Num::as_f64)
}

/// fontTools' `otRound`: halves round up.
pub(crate) fn ot_round(v: f64) -> Result<i64> {
    if !v.is_finite() || v.abs() > 9.0e15 {
        return Err(TablesError::invalid(format!("the value {v} is out of range")));
    }
    Ok((v + 0.5).floor() as i64)
}

/// fontTools' `floatToFixed`.
pub(crate) fn float_to_fixed(v: f64, bits: u32) -> Result<i64> {
    ot_round(v * f64::from(1u32 << bits.min(30)))
}

/// fontTools' `binary2num`: every digit other than `0` is a one.
pub(crate) fn binary2num(text: &str) -> u64 {
    text.chars().filter(|c| !c.is_whitespace()).fold(0u64, |acc, c| (acc << 1) | u64::from(c != '0'))
}

/// fontTools' `readHex`: the hex digits of `text`, whitespace ignored, a lone last digit
/// completed with `0`.
pub(crate) fn read_hex(text: &str) -> Result<Vec<u8>> {
    let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(digits.len().div_ceil(2));
    for pair in digits.chunks(2) {
        let hi = pair.first().and_then(|b| (*b as char).to_digit(16));
        let lo = match pair.get(1) {
            Some(b) => (*b as char).to_digit(16),
            None => Some(0),
        };
        match (hi, lo) {
            (Some(h), Some(l)) => out.push((h * 16 + l) as u8),
            _ => return Err(TablesError::invalid("the hex data holds a character that is not a hex digit")),
        }
    }
    Ok(out)
}

/// The text of an element's `hexdata` child (or of the element itself) as bytes.
pub(crate) fn hexdata(element: &Element) -> Result<Vec<u8>> {
    match element.child("hexdata") {
        Some(h) => read_hex(&h.text),
        None => read_hex(&element.text),
    }
}

/// An attribute that must be there.
pub(crate) fn attr<'e>(element: &'e Element, name: &str) -> Result<&'e str> {
    element.attribute(name).ok_or_else(|| TablesError::invalid(format!("the element '{}' has no '{name}' attribute", element.name)))
}

/// The `value` attribute as an integer.
pub(crate) fn int_value(element: &Element) -> Result<i64> {
    int_lit(attr(element, "value")?)
}

/// Python's `str.strip()`.
pub(crate) fn py_strip(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

/// Seconds since 1904 from fontTools' timestamp text (`Thu Jan  1 00:00:00 1970`).
pub(crate) fn timestamp_from_string(text: &str) -> Result<u64> {
    const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    const EPOCH_DIFF: i64 = 2_082_844_800;
    let bad = || TablesError::invalid(format!("'{text}' is not a timestamp"));
    let parts: Vec<&str> = text.split_whitespace().collect();
    let [weekday, month, day, time, year] = parts.as_slice() else { return Err(bad()) };
    let weekday = DAYS.iter().position(|d| d == weekday).ok_or_else(bad)?;
    let month = MONTHS.iter().position(|m| m == month).ok_or_else(bad)? as i64 + 1;
    let day: i64 = day.parse().map_err(|_| bad())?;
    let year: i64 = year.parse().map_err(|_| bad())?;
    let hms: Vec<i64> = time.split(':').map(|p| p.parse::<i64>()).collect::<std::result::Result<_, _>>().map_err(|_| bad())?;
    let [h, m, s] = hms.as_slice() else { return Err(bad()) };
    if !(1..=31).contains(&day) || !(0..24).contains(h) || !(0..60).contains(m) || !(0..62).contains(s) || !(1..=9999).contains(&year) {
        return Err(bad());
    }
    let days = days_from_civil(year, month, day);
    if usize::try_from((days + 3).rem_euclid(7)).ok() != Some(weekday) {
        return Err(TablesError::invalid(format!("'{text}' has a weekday that does not match its date")));
    }
    let secs = days * 86_400 + h * 3600 + m * 60 + s + EPOCH_DIFF;
    u64::try_from(secs).map_err(|_| bad())
}

/// Days since 1970-01-01 (H. Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Big-endian output with range checks: a value that does not fit is an error, as Python's
/// `struct.pack` makes it.
#[derive(Default)]
pub(crate) struct Out(pub Vec<u8>);

impl Out {
    fn range(v: i64, lo: i64, hi: i64, what: &str) -> Result<i64> {
        if (lo..=hi).contains(&v) { Ok(v) } else { Err(TablesError::invalid(format!("{what} = {v} does not fit its field"))) }
    }
    pub(crate) fn u8(&mut self, v: i64, what: &str) -> Result<()> {
        self.0.push(Self::range(v, 0, 0xFF, what)? as u8);
        Ok(())
    }
    pub(crate) fn i8(&mut self, v: i64, what: &str) -> Result<()> {
        self.0.push(Self::range(v, -128, 127, what)? as i8 as u8);
        Ok(())
    }
    pub(crate) fn u16(&mut self, v: i64, what: &str) -> Result<()> {
        self.0.extend_from_slice(&(Self::range(v, 0, 0xFFFF, what)? as u16).to_be_bytes());
        Ok(())
    }
    pub(crate) fn i16(&mut self, v: i64, what: &str) -> Result<()> {
        self.0.extend_from_slice(&(Self::range(v, -32768, 32767, what)? as i16).to_be_bytes());
        Ok(())
    }
    pub(crate) fn u24(&mut self, v: i64, what: &str) -> Result<()> {
        let v = Self::range(v, 0, 0xFF_FFFF, what)? as u32;
        self.0.extend_from_slice(v.to_be_bytes().get(1..).unwrap_or_default());
        Ok(())
    }
    pub(crate) fn u32(&mut self, v: i64, what: &str) -> Result<()> {
        self.0.extend_from_slice(&(Self::range(v, 0, 0xFFFF_FFFF, what)? as u32).to_be_bytes());
        Ok(())
    }
    pub(crate) fn i32(&mut self, v: i64, what: &str) -> Result<()> {
        self.0.extend_from_slice(&(Self::range(v, i64::from(i32::MIN), i64::from(i32::MAX), what)? as i32).to_be_bytes());
        Ok(())
    }
    pub(crate) fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    pub(crate) fn bytes(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
    }
}

// ── XML ────────────────────────────────────────────────────────────────────────────────────────

fn xml_error(what: &str, pos: usize) -> TablesError {
    TablesError::invalid(format!("the XML could not be read: {what} (at byte {pos})"))
}

/// What the TTX texts parsed against one budget may still use: text bytes, elements and an
/// estimate of the element tree's memory.
#[derive(Clone, Debug)]
pub struct ParseBudget {
    text: usize,
    elements: usize,
    tree: usize,
}

impl Default for ParseBudget {
    fn default() -> Self {
        Self { text: MAX_TTX_BYTES, elements: MAX_TTX_ELEMENTS, tree: MAX_TTX_TREE_BYTES }
    }
}

impl ParseBudget {
    fn tree(&mut self, bytes: usize) -> Result<()> {
        self.tree =
            self.tree.checked_sub(bytes).ok_or(TablesError::TooLarge { what: "the memory of the TTX element tree", limit: MAX_TTX_TREE_BYTES })?;
        Ok(())
    }
}

/// The memory a string's heap allocation takes, with a share for the allocator.
fn heap(s: &str) -> usize {
    if s.is_empty() { 0 } else { s.len().saturating_add(16) }
}

/// The memory an element just read takes, without its text and children.
fn element_cost(e: &Element) -> usize {
    let attributes = e
        .attributes
        .iter()
        .fold(0usize, |n, (k, v)| n.saturating_add(std::mem::size_of::<(String, String)>()).saturating_add(heap(k)).saturating_add(heap(v)));
    std::mem::size_of::<Element>().saturating_add(heap(&e.name)).saturating_add(attributes)
}

/// Parses TTX text into its root element within the limits.
pub(crate) fn parse(text: &str) -> Result<Element> {
    parse_within(text, &mut ParseBudget::default())
}

/// Parses TTX text into its root element, charging `budget`; several texts can share one.
pub(crate) fn parse_within(text: &str, budget: &mut ParseBudget) -> Result<Element> {
    budget.text = budget.text.checked_sub(text.len()).ok_or(TablesError::TooLarge { what: "the TTX text", limit: MAX_TTX_BYTES })?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let b = text.as_bytes();
    let mut pos = 0usize;
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;
    while pos < b.len() {
        let rest = text.get(pos..).unwrap_or_default();
        if !rest.starts_with('<') {
            let end = rest.find('<').map_or(b.len(), |i| pos.saturating_add(i));
            let raw = text.get(pos..end).unwrap_or_default();
            let decoded = decode_text(raw, false, pos)?;
            match stack.last_mut() {
                Some(top) => {
                    budget.tree(decoded.len())?;
                    top.text.push_str(&decoded);
                }
                None if decoded.trim().is_empty() => {}
                None => return Err(xml_error("text outside the root element", pos)),
            }
            pos = end;
            continue;
        }
        if rest.starts_with("<?") {
            pos = skip_past(text, pos, "?>")?;
        } else if rest.starts_with("<!--") {
            pos = skip_past(text, pos, "-->")?;
        } else if rest.starts_with("<![CDATA[") {
            let start = pos.saturating_add(9);
            let end = text
                .get(start..)
                .and_then(|r| r.find("]]>"))
                .map(|i| start.saturating_add(i))
                .ok_or_else(|| xml_error("an unclosed CDATA section", pos))?;
            let body = normalize_newlines(text.get(start..end).unwrap_or_default());
            match stack.last_mut() {
                Some(top) => {
                    budget.tree(body.len())?;
                    top.text.push_str(&body);
                }
                None => return Err(xml_error("a CDATA section outside the root element", pos)),
            }
            pos = end.saturating_add(3);
        } else if rest.starts_with("<!") {
            let close = rest.find('>').ok_or_else(|| xml_error("an unclosed declaration", pos))?;
            if rest.get(..close).is_some_and(|d| d.contains('[')) {
                return Err(TablesError::Unsupported("document type declarations with an internal subset".into()));
            }
            pos = pos.saturating_add(close).saturating_add(1);
        } else if rest.starts_with("</") {
            let close = rest.find('>').ok_or_else(|| xml_error("an unclosed end tag", pos))?;
            let name = rest.get(2..close).unwrap_or_default().trim();
            let element = stack.pop().ok_or_else(|| xml_error("an end tag without a start tag", pos))?;
            if element.name != name {
                return Err(xml_error(&format!("the end tag '{name}' does not match '{}'", element.name), pos));
            }
            attach(&mut stack, &mut root, element, pos)?;
            pos = pos.saturating_add(close).saturating_add(1);
        } else {
            let (element, empty, next) = start_tag(text, pos)?;
            budget.elements =
                budget.elements.checked_sub(1).ok_or(TablesError::TooLarge { what: "the number of TTX elements", limit: MAX_TTX_ELEMENTS })?;
            budget.tree(element_cost(&element))?;
            if stack.len() >= MAX_TTX_DEPTH {
                return Err(TablesError::TooLarge { what: "the TTX element nesting", limit: MAX_TTX_DEPTH });
            }
            if root.is_some() {
                return Err(xml_error("an element after the root element", pos));
            }
            if empty {
                attach(&mut stack, &mut root, element, pos)?;
            } else {
                stack.push(element);
            }
            pos = next;
        }
    }
    if let Some(open) = stack.last() {
        return Err(xml_error(&format!("the element '{}' is not closed", open.name), pos));
    }
    root.ok_or_else(|| xml_error("no root element", pos))
}

fn attach(stack: &mut [Element], root: &mut Option<Element>, element: Element, pos: usize) -> Result<()> {
    match stack.last_mut() {
        Some(parent) => parent.children.push(element),
        None if root.is_none() => *root = Some(element),
        None => return Err(xml_error("a second root element", pos)),
    }
    Ok(())
}

fn skip_past(text: &str, pos: usize, end: &str) -> Result<usize> {
    let rest = text.get(pos..).unwrap_or_default();
    let i = rest.find(end).ok_or_else(|| xml_error(&format!("no closing '{end}'"), pos))?;
    Ok(pos.saturating_add(i).saturating_add(end.len()))
}

fn is_name_end(b: u8) -> bool {
    b.is_ascii_whitespace() || matches!(b, b'/' | b'>' | b'=')
}

/// A start tag at `pos`: the element, whether it closes itself, and the position after it.
fn start_tag(text: &str, pos: usize) -> Result<(Element, bool, usize)> {
    let b = text.as_bytes();
    let mut i = pos.saturating_add(1);
    let name_start = i;
    while b.get(i).is_some_and(|c| !is_name_end(*c)) {
        i = i.saturating_add(1);
    }
    let name = text.get(name_start..i).unwrap_or_default();
    if name.is_empty() {
        return Err(xml_error("a tag without a name", pos));
    }
    let mut element = Element { name: name.to_string(), ..Element::default() };
    loop {
        while b.get(i).is_some_and(u8::is_ascii_whitespace) {
            i = i.saturating_add(1);
        }
        match b.get(i) {
            None => return Err(xml_error("an unclosed start tag", pos)),
            Some(b'>') => return Ok((element, false, i.saturating_add(1))),
            Some(b'/') => {
                if b.get(i.saturating_add(1)) != Some(&b'>') {
                    return Err(xml_error("a '/' inside a start tag", i));
                }
                return Ok((element, true, i.saturating_add(2)));
            }
            Some(_) => {}
        }
        let attr_start = i;
        while b.get(i).is_some_and(|c| !is_name_end(*c)) {
            i = i.saturating_add(1);
        }
        let attr_name = text.get(attr_start..i).unwrap_or_default().to_string();
        while b.get(i).is_some_and(u8::is_ascii_whitespace) {
            i = i.saturating_add(1);
        }
        if attr_name.is_empty() || b.get(i) != Some(&b'=') {
            return Err(xml_error("an attribute without a value", i));
        }
        i = i.saturating_add(1);
        while b.get(i).is_some_and(u8::is_ascii_whitespace) {
            i = i.saturating_add(1);
        }
        let quote = match b.get(i) {
            Some(q @ (b'"' | b'\'')) => *q,
            _ => return Err(xml_error("an attribute value without quotes", i)),
        };
        let value_start = i.saturating_add(1);
        let rel = text.get(value_start..).and_then(|r| r.find(quote as char)).ok_or_else(|| xml_error("an unclosed attribute value", i))?;
        let value_end = value_start.saturating_add(rel);
        let raw = text.get(value_start..value_end).unwrap_or_default();
        if raw.contains('<') {
            return Err(xml_error("a '<' inside an attribute value", value_start));
        }
        if element.attributes.iter().any(|(n, _)| *n == attr_name) {
            return Err(xml_error(&format!("the attribute '{attr_name}' is given twice"), attr_start));
        }
        element.attributes.push((attr_name, decode_text(raw, true, value_start)?));
        i = value_end.saturating_add(1);
    }
}

fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Character data with its entities decoded; in an attribute value, literal tabs and line ends
/// become spaces.
fn decode_text(raw: &str, attribute: bool, pos: usize) -> Result<String> {
    let mut text = normalize_newlines(raw);
    if attribute {
        text = text.replace(['\n', '\t'], " ");
    }
    if !text.contains('&') {
        return Ok(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(i) = rest.find('&') {
        out.push_str(rest.get(..i).unwrap_or_default());
        let after = rest.get(i.saturating_add(1)..).unwrap_or_default();
        let end = after.find(';').ok_or_else(|| xml_error("an entity without ';'", pos))?;
        let entity = after.get(..end).unwrap_or_default();
        let c = match entity {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
                    u32::from_str_radix(hex, 16).ok()
                } else if let Some(dec) = entity.strip_prefix('#') {
                    dec.parse::<u32>().ok()
                } else {
                    return Err(xml_error(&format!("the unknown entity '&{entity};'"), pos));
                };
                code.and_then(char::from_u32).ok_or_else(|| xml_error(&format!("the character reference '&{entity};'"), pos))?
            }
        };
        out.push(c);
        rest = after.get(end.saturating_add(1)..).unwrap_or_default();
    }
    out.push_str(rest);
    Ok(out)
}

// ── Import ─────────────────────────────────────────────────────────────────────────────────────

/// fontTools' `xmlToTag`.
pub(crate) fn xml_to_tag(name: &str) -> Result<Tag> {
    let bad = || TablesError::invalid(format!("'{name}' does not name a table"));
    if name == "OS_2" {
        return Ok(*b"OS/2");
    }
    let mut tag = [b' '; 4];
    if name.len() == 8 && name.is_ascii() {
        let ident = name.as_bytes();
        let mut out = Vec::new();
        for pair in ident.chunks(2) {
            match pair {
                [b'_', c] => out.push(*c),
                [c, b'_'] => out.push(*c),
                [h, l] => {
                    let hex = std::str::from_utf8(&[*h, *l]).ok().and_then(|s| u8::from_str_radix(s, 16).ok()).ok_or_else(bad)?;
                    out.push(hex);
                }
                _ => return Err(bad()),
            }
        }
        if out.len() > 4 {
            return Err(bad());
        }
        for (slot, c) in tag.iter_mut().zip(out) {
            *slot = c;
        }
        return Ok(tag);
    }
    if name.is_empty() || name.len() > 4 || !name.is_ascii() {
        return Err(bad());
    }
    for (slot, c) in tag.iter_mut().zip(name.bytes()) {
        *slot = c;
    }
    Ok(tag)
}

fn flavour_from_attribute(value: &str) -> Option<Flavour> {
    match value {
        "\\x00\\x01\\x00\\x00" | "\u{0}\u{1}\u{0}\u{0}" => Some(Flavour::TrueType),
        "OTTO" => Some(Flavour::Cff),
        "true" => Some(Flavour::AppleTrue),
        _ => None,
    }
}

/// Whether an element's attribute holds a true Python literal.
fn truthy(element: &Element, name: &str) -> bool {
    element.attribute(name).is_some_and(|v| int_lit(v).map(|n| n != 0).unwrap_or(v.trim() == "True"))
}

/// See [`super::import`].
pub(crate) fn import(font: SfntFont, text: &str, options: &ImportOptions) -> Result<SfntFont> {
    import_tree(font, &parse(text)?, options)
}

/// See [`super::import_tree`].
pub(crate) fn import_tree(mut font: SfntFont, root: &Element, options: &ImportOptions) -> Result<SfntFont> {
    if root.name != "ttFont" {
        return Err(TablesError::invalid(format!("the root element is '{}', not 'ttFont'", root.name)));
    }
    if font.tables.is_empty()
        && let Some(f) = root.attribute("sfntVersion").and_then(flavour_from_attribute)
    {
        font.flavour = f;
    }
    let wanted = |tag: &Tag| options.tables.is_empty() || options.tables.contains(tag);
    let mut explicit_order: Option<Vec<String>> = None;
    let mut elements: Vec<(Tag, &Element)> = Vec::new();
    for child in &root.children {
        if child.attribute("src").is_some() {
            return Err(TablesError::Unsupported(format!("the element '{}' names another file, which this call cannot read", child.name)));
        }
        if child.name == "GlyphOrder" {
            let names: Vec<String> =
                child.children.iter().filter(|g| g.name == "GlyphID").map(|g| attr(g, "name").map(str::to_string)).collect::<Result<_>>()?;
            if names.len() > crate::limits::MAX_GLYPHS {
                return Err(TablesError::TooLarge { what: "the glyph order", limit: crate::limits::MAX_GLYPHS });
            }
            explicit_order = Some(names);
            continue;
        }
        let tag = xml_to_tag(&child.name)?;
        if wanted(&tag) {
            elements.push((tag, child));
        }
    }
    let order = match explicit_order {
        Some(names) => Ok(names),
        None => glyph_order(&font),
    };
    // fontTools compiles a table's dependencies first: `fvar` before the tables that take their
    // axes from it, `glyf` and `cvt ` before the variations of their points and values.
    elements.sort_by_key(|(tag, _)| match tag {
        b"fvar" => 0,
        b"glyf" | b"cvt " => 1,
        _ => 2,
    });
    let mut present: BTreeSet<Tag> = font.tables.keys().copied().collect();
    present.extend(elements.iter().map(|(t, _)| *t));
    let ctx = ReadContext::new(order, present, !options.keep_bounds);

    // A bitmap location table and its data table compile together (fontTools compiles the data
    // table first and gives the location table where each image went).
    let mut bitmaps: HashMap<Tag, Vec<u8>> = HashMap::new();
    for (tag, element) in &elements {
        let raw = element.attribute("ERROR").is_some() || truthy(element, "raw");
        if raw || read_bitmap::is_location(*tag) || bitmaps.contains_key(tag) {
            continue;
        }
        let Some(location_tag) = read_bitmap::partner(*tag) else { continue };
        let location = elements.iter().rev().find(|(t, e)| *t == location_tag && e.attribute("ERROR").is_none() && !truthy(e, "raw"));
        let Some((_, location)) = location else {
            return Err(TablesError::Unsupported(format!(
                "reading the table '{}' from TTX without its '{}' table in the same text",
                crate::sfnt::tag_str(*tag),
                crate::sfnt::tag_str(location_tag)
            )));
        };
        let (data, locations) = read_bitmap::bitmap_pair(*tag, element, location, &ctx)?;
        bitmaps.insert(*tag, data);
        bitmaps.insert(location_tag, locations);
    }

    let mut loaded: BTreeSet<Tag> = BTreeSet::new();
    let mut glyf: Option<read_core::Glyf> = None;
    let mut metrics: [Option<Vec<(i64, i64)>>; 2] = [None, None];
    // fontTools' `importXML` reads the glyph order of a font with `maxp` and `post` before the
    // text, which loads a CFF font's `CFF ` table; the save then compiles that table again, its
    // FontBBox recalculated, although the text holds no `CFF ` element. A call that picks its
    // tables (the internal copies of layout tables) changes only those.
    if options.tables.is_empty()
        && !elements.iter().any(|(tag, _)| tag == b"CFF ")
        && let Some(data) = reload_cff(&font, ctx.recalc)
    {
        font.insert(*b"CFF ", data);
        loaded.insert(*b"CFF ");
    }
    for (tag, element) in &elements {
        let tag = *tag;
        let raw = element.attribute("ERROR").is_some() || truthy(element, "raw");
        let bytes = if raw {
            hexdata(element)?
        } else if let Some(data) = bitmaps.get(&tag) {
            data.clone()
        } else if read_bitmap::partner(tag).is_some() {
            return Err(TablesError::Unsupported(format!(
                "reading the table '{}' from TTX without its '{}' table in the same text",
                crate::sfnt::tag_str(tag),
                read_bitmap::partner(tag).map(crate::sfnt::tag_str).unwrap_or_default()
            )));
        } else if &tag == b"glyf" {
            let compiled = read_core::compile_glyf(element, &ctx)?;
            let data = compiled.data.clone();
            glyf = Some(compiled);
            data
        } else if &tag == b"loca" {
            // fontTools keeps the font's own `loca`; `glyf` replaces it when it is compiled.
            font.get(tag).map(<[u8]>::to_vec).unwrap_or_default()
        } else if &tag == b"gvar" {
            read_var::gvar(element, &ctx, &font, glyf.as_ref().map(|g| (g.data.as_slice(), g.locations.as_slice())))?
        } else if &tag == b"hmtx" || &tag == b"vmtx" {
            let vertical = &tag == b"vmtx";
            let m = read_core::compile_metrics(element, &ctx, vertical)?;
            let header = if vertical { *b"vhea" } else { *b"hhea" };
            let (data, _) = read_core::pack_metrics(&m, ctx.has(header))?;
            if let Some(slot) = metrics.get_mut(usize::from(vertical)) {
                *slot = Some(m);
            }
            data
        } else {
            match read_known(element, &ctx, &font)? {
                Some((_, data)) => data,
                None if element.child("hexdata").is_some() => hexdata(element)?,
                None => return Err(TablesError::Unsupported(format!("reading the table '{}' from TTX", crate::sfnt::tag_str(tag)))),
            }
        };
        font.insert(tag, bytes);
        loaded.insert(tag);
    }
    recalculate(&mut font, &ctx, &mut loaded, glyf.as_ref(), &metrics)?;
    Ok(font)
}

/// The font's own `CFF ` table compiled again as fontTools compiles a table it has decompiled:
/// written as TTX and read back. `None` when the font lacks `maxp`, `post` or `CFF `, or when the
/// table cannot be written or read; its bytes then stay as they are.
fn reload_cff(font: &SfntFont, recalc: bool) -> Option<Vec<u8>> {
    if font.get(*b"maxp").is_none() || font.get(*b"post").is_none() || font.get(*b"CFF ").is_none() {
        return None;
    }
    let options = super::DumpOptions { tables: vec![*b"CFF "], strict: true, ..Default::default() };
    let text = super::dump(font, &options).ok()?;
    let root = parse(&attribute_breaks_as_references(&text)).ok()?;
    let element = root.children.iter().find(|c| xml_to_tag(&c.name).ok() == Some(*b"CFF "))?;
    let ctx = ReadContext::new(glyph_order(font), font.tables.keys().copied().collect(), recalc);
    read_cff::read_table_with(element, &ctx).ok().flatten().map(|(_, data)| data)
}

/// `text` with the line ends and tabs inside attribute values written as character references,
/// which the parser keeps (a literal one reads as a space). The writer escapes `"`, `<` and `>`
/// in values, so a quote inside a tag always opens or closes a value.
fn attribute_breaks_as_references(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let (mut in_tag, mut in_value) = (false, false);
    let mut skip_to = 0usize;
    for (i, c) in text.char_indices() {
        if i < skip_to {
            continue;
        }
        if !in_tag && c == '<' && text.get(i..).is_some_and(|rest| rest.starts_with("<!--")) {
            let end = text.get(i..).and_then(|rest| rest.find("-->")).map_or(text.len(), |e| i + e + 3);
            out.push_str(text.get(i..end).unwrap_or_default());
            skip_to = end;
            continue;
        }
        match c {
            '\n' if in_value => out.push_str("&#10;"),
            '\t' if in_value => out.push_str("&#9;"),
            _ => {
                match c {
                    '<' if !in_tag => in_tag = true,
                    '>' if in_tag && !in_value => in_tag = false,
                    '"' if in_tag => in_value = !in_value,
                    _ => {}
                }
                out.push(c);
            }
        }
    }
    out
}

fn read_known(element: &Element, ctx: &ReadContext, font: &SfntFont) -> Result<Option<(Tag, Vec<u8>)>> {
    if let Some(t) = read_core::read_table_with(element, ctx)? {
        return Ok(Some(t));
    }
    if let Some(t) = read_cff::read_table_with(element, ctx)? {
        return Ok(Some(t));
    }
    if let Some(t) = read_misc::read_table_with(element, ctx)? {
        return Ok(Some(t));
    }
    match element.name.as_str() {
        "sbix" => return Ok(Some((*b"sbix", read_bitmap::sbix(element, ctx)?))),
        "EBSC" => return Ok(Some((*b"EBSC", read_bitmap::ebsc(element)?))),
        _ => {}
    }
    type Reader = fn(&Element, &ReadContext, &SfntFont) -> Result<Option<(Tag, Vec<u8>)>>;
    let others: [Reader; 3] = [read_otdata::read_table_with, read_var::read_table_with, read_color::read_table_with];
    for read in others {
        if let Some(t) = read(element, ctx, font)? {
            return Ok(Some(t));
        }
    }
    Ok(None)
}

// ── What the compiler recalculates ─────────────────────────────────────────────────────────────

pub(crate) fn put_u16(font: &mut SfntFont, tag: Tag, at: usize, v: i64, what: &str) -> Result<()> {
    put(font, tag, at, &{
        let mut o = Out::default();
        o.u16(v, what)?;
        o.0
    })
}

pub(crate) fn put_i16(font: &mut SfntFont, tag: Tag, at: usize, v: i64, what: &str) -> Result<()> {
    put(font, tag, at, &{
        let mut o = Out::default();
        o.i16(v, what)?;
        o.0
    })
}

fn put(font: &mut SfntFont, tag: Tag, at: usize, bytes: &[u8]) -> Result<()> {
    let name = crate::sfnt::tag_str(tag);
    let mut data = font.get(tag).map(<[u8]>::to_vec).ok_or_else(|| TablesError::MissingTable(name.clone()))?;
    let end = at.saturating_add(bytes.len());
    let slot = data.get_mut(at..end).ok_or_else(|| TablesError::invalid(format!("the '{name}' table is cut short")))?;
    slot.copy_from_slice(bytes);
    font.insert(tag, data);
    Ok(())
}

pub(crate) fn get_u16(font: &SfntFont, tag: Tag, at: usize) -> Option<u16> {
    let d = font.get(tag)?;
    Some(u16::from_be_bytes([*d.get(at)?, *d.get(at.saturating_add(1))?]))
}

/// The metrics of every glyph from the font's own `hmtx` or `vmtx`.
pub(crate) fn font_metrics(font: &SfntFont, vertical: bool, count: usize) -> Option<Vec<(i64, i64)>> {
    let (table, header) = if vertical { (*b"vmtx", *b"vhea") } else { (*b"hmtx", *b"hhea") };
    let data = font.get(table)?;
    let long = get_u16(font, header, 34).map_or(count, usize::from).min(count);
    let mut out = Vec::with_capacity(count);
    let mut last = 0i64;
    for i in 0..count {
        if i < long {
            let at = i.checked_mul(4)?;
            let adv = i64::from(u16::from_be_bytes([*data.get(at)?, *data.get(at.saturating_add(1))?]));
            let sb = i64::from(i16::from_be_bytes([*data.get(at.saturating_add(2))?, *data.get(at.saturating_add(3))?]));
            last = adv;
            out.push((adv, sb));
        } else {
            let at = long.checked_mul(4)?.checked_add(i.saturating_sub(long).checked_mul(2)?)?;
            let sb = i64::from(i16::from_be_bytes([*data.get(at)?, *data.get(at.saturating_add(1))?]));
            out.push((last, sb));
        }
    }
    Some(out)
}

/// Fills the fields fontTools' compiler fills, in the order its table dependencies give.
fn recalculate(
    font: &mut SfntFont,
    ctx: &ReadContext,
    loaded: &mut BTreeSet<Tag>,
    glyf: Option<&read_core::Glyf>,
    metrics: &[Option<Vec<(i64, i64)>>; 2],
) -> Result<()> {
    let count = ctx.order().map(<[String]>::len).ok();
    // glyf → loca, head.indexToLocFormat, maxp.numGlyphs
    if let Some(g) = glyf {
        if font.get(*b"loca").is_some() || loaded.contains(b"loca") {
            let (loca, long) = read_core::compile_loca(&g.locations);
            font.insert(*b"loca", loca);
            loaded.insert(*b"loca");
            if font.get(*b"head").is_some() {
                put_i16(font, *b"head", 50, i64::from(long), "head.indexToLocFormat")?;
                loaded.insert(*b"head");
            }
        }
        if font.get(*b"maxp").is_some() {
            loaded.insert(*b"maxp");
        }
    }
    // hmtx, vmtx → the metric counts of hhea and vhea
    for (vertical, header) in [(false, *b"hhea"), (true, *b"vhea")] {
        if let Some(Some(m)) = metrics.get(usize::from(vertical))
            && font.get(header).is_some()
        {
            let (_, n) = read_core::pack_metrics(m, true)?;
            put_u16(font, header, 34, i64::try_from(n).unwrap_or(i64::MAX), "the number of long metrics")?;
            loaded.insert(header);
        }
    }
    let metrics_of = |font: &SfntFont, vertical: bool| -> Option<Vec<(i64, i64)>> {
        match metrics.get(usize::from(vertical)) {
            Some(Some(m)) => Some(m.clone()),
            _ => font_metrics(font, vertical, count?),
        }
    };
    // maxp
    if loaded.contains(b"maxp") && font.get(*b"maxp").is_some() {
        if let Some(g) = glyf.filter(|_| ctx.recalc) {
            let hmtx = metrics_of(font, false);
            let r = read_core::maxp_recalc(&g.glyphs, hmtx.as_deref())?;
            if read_core::apply_maxp_recalc(font, &r)? {
                loaded.insert(*b"head");
            }
        }
        if let Some(n) = count {
            put_u16(font, *b"maxp", 4, i64::try_from(n).unwrap_or(i64::MAX), "maxp.numGlyphs")?;
        }
        if get_u16(font, *b"maxp", 0) != Some(0) || get_u16(font, *b"maxp", 2) != Some(0x5000) {
            put(font, *b"maxp", 0, &0x0001_0000u32.to_be_bytes())?;
        }
    }
    // head is compiled before hhea and vhea. Compiling OS/2 reads head, which loads it; with
    // bounds recalculation, compiling head loads `CFF ` (else `CFF2`), so hhea and vhea then
    // recalculate their extremes from it.
    if loaded.contains(b"OS/2") && font.get(*b"head").is_some() {
        loaded.insert(*b"head");
    }
    if loaded.contains(b"head") && ctx.recalc && font.get(*b"head").is_some() {
        if font.get(*b"CFF ").is_some() {
            loaded.insert(*b"CFF ");
        } else if font.get(*b"CFF2").is_some() {
            loaded.insert(*b"CFF2");
        }
    }
    // Glyph bounds for the hhea and vhea extremes.
    let outlines_loaded = loaded.contains(b"glyf") || loaded.contains(b"CFF ") || loaded.contains(b"CFF2");
    let bounds: Option<Vec<Option<[f64; 4]>>> = if !ctx.recalc || !outlines_loaded {
        None
    } else if let Some(g) = glyf {
        Some(g.glyphs.iter().map(|s| (s.contours != 0).then(|| s.bounds.map(|v| v as f64))).collect())
    } else if let Some(data) = font.get(*b"CFF ") {
        Some(read_cff::glyph_bounds(data)?)
    } else if let Some(data) = font.get(*b"CFF2") {
        Some(read_cff::glyph_bounds(data)?)
    } else {
        None
    };
    let from_cff = glyf.is_none();
    for (vertical, header) in [(false, *b"hhea"), (true, *b"vhea")] {
        if !loaded.contains(&header) || font.get(header).is_none() {
            continue;
        }
        let (Some(bounds), Some(m)) = (bounds.as_ref(), metrics_of(font, vertical)) else { continue };
        let advance_max = m.iter().map(|(a, _)| *a).max().unwrap_or(0);
        put_u16(font, header, 10, advance_max, "the largest advance")?;
        let mut min_first = i64::MAX;
        let mut min_second = i64::MAX;
        let mut max_extent = i64::MIN;
        let mut any = false;
        for (b, (advance, bearing)) in bounds.iter().zip(&m) {
            let Some(b) = b else { continue };
            let extent = if vertical {
                if from_cff { (b[3].ceil() - b[1].floor()) as i64 } else { (b[3] - b[1]) as i64 }
            } else if from_cff {
                (b[2].ceil() - b[0].floor()) as i64
            } else {
                (b[2] - b[0]) as i64
            };
            any = true;
            min_first = min_first.min(*bearing);
            min_second = min_second.min(advance.saturating_sub(*bearing).saturating_sub(extent));
            max_extent = max_extent.max(bearing.saturating_add(extent));
        }
        if !any {
            (min_first, min_second, max_extent) = (0, 0, 0);
        }
        put_i16(font, header, 12, min_first, "the smallest side bearing")?;
        put_i16(font, header, 14, min_second, "the smallest side bearing")?;
        put_i16(font, header, 16, max_extent, "the largest extent")?;
    }
    // head: the font bounding box of CFF fonts; a Top DICT without FontBBox has the default, zeros.
    if loaded.contains(b"head") && ctx.recalc && font.get(*b"head").is_some() {
        let bbox = if let Some(data) = font.get(*b"CFF ") {
            Some(read_cff::font_bbox(data)?.unwrap_or_default())
        } else if let Some(data) = font.get(*b"CFF2") {
            read_cff::glyph_bounds(data)?.into_iter().flatten().reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
        } else {
            None
        };
        if let Some(b) = bbox {
            for (at, v) in [(36, b[0].floor()), (38, b[1].floor()), (40, b[2].ceil()), (42, b[3].ceil())] {
                put_i16(font, *b"head", at, v as i64, "the font bounding box")?;
            }
        }
    }
    // OS/2: the first and last character indices from the Unicode cmap subtables.
    if loaded.contains(b"OS/2")
        && let Some(cmap) = font.get(*b"cmap")
        && let Ok((_, subtables)) = super::core::cmap::parse(cmap)
    {
        let codes = subtables.iter().filter(|s| s.is_unicode()).flat_map(|s| s.map().iter().map(|(c, _)| *c));
        let (lo, hi) = codes.fold((u32::MAX, 0u32), |(lo, hi), c| (lo.min(c), hi.max(c)));
        if lo <= hi && font.get(*b"OS/2").is_some_and(|d| d.len() >= 68) {
            put_u16(font, *b"OS/2", 64, i64::from(lo.min(0xFFFF)), "OS/2.usFirstCharIndex")?;
            put_u16(font, *b"OS/2", 66, i64::from(hi.min(0xFFFF)), "OS/2.usLastCharIndex")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
