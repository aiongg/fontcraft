//! Encoding files, name lists and the font comparison table (M13.46). Rows:
//! `view.filterByEncodingFile`, `font.renameGlyphsByNamelist`, `font.compareInTable`.
//!
//! `font.renameGlyphsByNamelist` is the engine command here. The other two rows are shell
//! commands (`kind = "ui"`): the shell reads files and builds the table with the functions below,
//! so the logic is tested without the UI.
//!
//! # Encoding files
//!
//! An encoding file lists the slots of a code page or of a custom character set, in order.
//! [`parse_encoding`] reads these line forms, which may be mixed in one file:
//!
//! | Line | Meaning |
//! |---|---|
//! | `Agrave 192` | glyph `Agrave` in slot 192 (decimal, or hexadecimal with `0x`) |
//! | `0xC0 0x00C0` | slot 0xC0 holds the code point U+00C0 |
//! | `Agrave`, `U+00C0` or `0x00C0` alone | the slot after the last one filled |
//!
//! `%` and `#` start a comment. A line `%%FONTLAB ENCODING: id; Title` or `%%FONTLAB CODEPAGE:
//! id; Title` gives the title; other lines starting with `%%` are skipped. Slots that no line
//! fills are empty. The built-in encodings ([`BUILTIN_ENCODINGS`]) are code pages of 256 slots.
//!
//! # Name lists
//!
//! A name list maps code points to glyph names. [`parse_namelist`] reads:
//!
//! | Line | Meaning |
//! |---|---|
//! | `0x00C0 Agrave` or `U+00C0 Agrave` | U+00C0 is named `Agrave` |
//! | `Agrave;00C0` | the same, in the Adobe Glyph List's layout; entries for a sequence of code points are skipped |
//! | `Agrave` alone | a name without a code point |
//!
//! `%` and `#` start a comment, a line `%%FONTLAB NAMETABLE: Title` gives the title and `Lang:`
//! lines are skipped. A `Based:` line (a list that extends another) is refused. When a code point
//! is named twice, the first name counts. The built-in lists are the glyph data's names (`nice`)
//! and its production names (`production`).
//!
//! Both readers take untrusted bytes: strict UTF-8, at most [`MAX_FILE_BYTES`] bytes and
//! [`MAX_LINES`] lines, names checked against the naming rules, and every failure names its line.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use fontcraft_model::glyphdata::{GlyphData, validate};
use fontcraft_model::{Font, GlyphTable, Layer, MAX_NAME_LEN, RenameReport, Renamed, Shape, rename_glyphs};
use serde::Serialize;
use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::params::Params;

/// Largest encoding or name-list file read.
pub const MAX_FILE_BYTES: usize = 8 << 20;
/// Most lines in an encoding or name-list file.
pub const MAX_LINES: usize = 300_000;
/// Most slots in an encoding (the highest slot index is one less).
pub const MAX_SLOTS: usize = 65_536;
/// Most entries in a name list (code points and bare names together).
pub const MAX_NAMES: usize = 200_000;
/// Longest title read from a file header, in bytes.
pub const MAX_TITLE_BYTES: usize = 256;
/// Most fonts in one comparison table.
pub const MAX_COMPARE_FONTS: usize = 64;
/// Most renaming problems listed in one refusal.
const MAX_LISTED_PROBLEMS: usize = 20;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "font.renameGlyphsByNamelist",
        "Rename Glyphs by Name List…",
        ["Font"],
        None,
        "{namelist: path | \"nice\" | \"production\", glyphs?: [names] (default every glyph), updateReferences?: bool (default true)} renames each glyph to the name the name list gives its first listed code point; an unencoded glyph whose name is a renamed glyph's name plus a suffix follows it (uni0041.sc becomes A.sc). A name list is a text file of lines `0x0041 A`, `U+0041 A` or `A;0041`; nice and production are the glyph data's names. updateReferences renames components, metrics keys, kerning, groups and the glyphOrder parameter too. Glyphs keep their code points and places. Refused whole when a new name breaks the naming rules or is taken → {renamed: [{from, to}], updated: [{name, place}], stillUsed: [{name, place}]}",
        has_doc,
        rename_by_namelist
    )
    .targets(&[crate::params::TargetKind::Glyphs])]
}

// ── Reading text files ──────────────────────────────────────────────────────────────────────────

/// The bytes of an encoding or name-list file, at most [`MAX_FILE_BYTES`].
#[cfg(not(target_arch = "wasm32"))]
pub fn read_text_file(path: &str) -> Result<Vec<u8>> {
    let bytes = crate::io::read_file(std::path::Path::new(path))?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(EngineError::Io(format!("{path} is {} bytes long; at most {MAX_FILE_BYTES} are read", bytes.len())));
    }
    Ok(bytes)
}

#[cfg(target_arch = "wasm32")]
pub fn read_text_file(_: &str) -> Result<Vec<u8>> {
    Err(EngineError::Unsupported("reading a file by path in the browser".into()))
}

/// `bytes` as text: strict UTF-8 without a byte-order mark, within the size limit.
fn decode<'a>(bytes: &'a [u8], what: &str) -> std::result::Result<&'a str, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(format!("the {what} is {} bytes long; at most {MAX_FILE_BYTES} are read", bytes.len()));
    }
    let text = std::str::from_utf8(bytes).map_err(|e| {
        let line = bytes.get(..e.valid_up_to()).map_or(1, |ok| ok.iter().filter(|b| **b == b'\n').count() + 1);
        format!("the {what} is not UTF-8 text (line {line})")
    })?;
    Ok(text.strip_prefix('\u{feff}').unwrap_or(text))
}

/// The lines of `text` numbered from 1, refusing more than [`MAX_LINES`].
fn numbered_lines<'a>(text: &'a str, what: &str) -> std::result::Result<Vec<(usize, &'a str)>, String> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if i >= MAX_LINES {
            return Err(format!("the {what} has more than {MAX_LINES} lines"));
        }
        out.push((i + 1, line));
    }
    Ok(out)
}

/// The title of a `%%FONTLAB <kind>: …` header line, after its `;` when there is one.
fn header_title(line: &str, kinds: &[&str]) -> std::result::Result<Option<String>, String> {
    let upper = line.to_ascii_uppercase();
    for kind in kinds {
        let prefix = format!("%%FONTLAB {kind}:");
        if upper.starts_with(&prefix) {
            let rest = line.get(prefix.len()..).unwrap_or("");
            let title = rest.split_once(';').map_or(rest, |(_, t)| t).trim();
            if title.len() > MAX_TITLE_BYTES {
                return Err(format!("the title is longer than {MAX_TITLE_BYTES} bytes"));
            }
            return Ok((!title.is_empty()).then(|| title.to_string()));
        }
    }
    Ok(None)
}

/// A line without its comment (from the first `%` or `#`).
fn without_comment(line: &str) -> &str {
    line.find(['%', '#']).and_then(|i| line.get(..i)).unwrap_or(line).trim()
}

/// A whole number in decimal, or in hexadecimal after `0x`.
fn number(token: &str) -> Option<u32> {
    match token.strip_prefix("0x").or_else(|| token.strip_prefix("0X")) {
        Some(hex) if (1..=8).contains(&hex.len()) => u32::from_str_radix(hex, 16).ok(),
        Some(_) => None,
        None if (1..=10).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_digit()) => token.parse().ok(),
        None => None,
    }
}

/// A Unicode scalar value written `U+00C0` or `0x00C0`.
fn code_point(token: &str) -> Option<u32> {
    let hex =
        token.strip_prefix("U+").or_else(|| token.strip_prefix("u+")).or_else(|| token.strip_prefix("0x")).or_else(|| token.strip_prefix("0X"))?;
    if !(1..=6).contains(&hex.len()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok().filter(|cp| char::from_u32(*cp).is_some())
}

/// Hexadecimal code point digits, as the Adobe Glyph List writes them (`00C0`).
fn bare_code_point(token: &str) -> Option<u32> {
    if !(4..=6).contains(&token.len()) {
        return None;
    }
    u32::from_str_radix(token, 16).ok().filter(|cp| char::from_u32(*cp).is_some())
}

/// A glyph name the naming rules accept, at most [`MAX_NAME_LEN`] bytes.
fn glyph_name(token: &str) -> std::result::Result<String, String> {
    if token.len() > MAX_NAME_LEN {
        return Err(format!("a glyph name is longer than {MAX_NAME_LEN} bytes"));
    }
    validate(token, false).map_err(|e| format!("`{token}` is not a glyph name: {e}"))?;
    Ok(token.to_string())
}

// ── Encodings ───────────────────────────────────────────────────────────────────────────────────

/// What one encoding slot holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Slot {
    /// A glyph by name.
    Name(String),
    /// A character by code point; the font's glyph for it fills the slot.
    Code(u32),
}

/// An encoding: a title and its slots in order, `None` for an empty slot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Encoding {
    pub title: Option<String>,
    pub slots: Vec<Option<Slot>>,
}

impl Encoding {
    /// Fills slot `at`, refusing a slot past [`MAX_SLOTS`] or one already filled.
    fn put(&mut self, at: usize, slot: Slot) -> std::result::Result<(), String> {
        if at >= MAX_SLOTS {
            return Err(format!("slot {at} is past the last allowed slot, {}", MAX_SLOTS - 1));
        }
        if self.slots.len() <= at {
            self.slots.resize(at + 1, None);
        }
        match self.slots.get_mut(at) {
            Some(place @ None) => {
                *place = Some(slot);
                Ok(())
            }
            _ => Err(format!("slot {at} is filled twice")),
        }
    }

    /// How many slots are filled.
    pub fn filled(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }
}

/// Reads an encoding file (see the module docs for its line forms).
pub fn parse_encoding(bytes: &[u8]) -> std::result::Result<Encoding, String> {
    let text = decode(bytes, "encoding file")?;
    let mut enc = Encoding::default();
    let mut next = 0usize;
    for (n, raw) in numbered_lines(text, "encoding file")? {
        let at_line = |e: String| format!("line {n}: {e}");
        let trimmed = raw.trim();
        if trimmed.starts_with("%%") {
            if enc.title.is_none() {
                enc.title = header_title(trimmed, &["ENCODING", "CODEPAGE"]).map_err(at_line)?;
            }
            continue;
        }
        let line = without_comment(trimmed);
        let tokens: Vec<&str> = line.split_whitespace().take(3).collect();
        let (at, slot) = match tokens.as_slice() {
            [] => continue,
            [one] => match code_point(one) {
                Some(cp) => (next, Slot::Code(cp)),
                None => (next, Slot::Name(glyph_name(one).map_err(at_line)?)),
            },
            [first, second] => match number(first) {
                Some(index) => {
                    let cp = code_point(second).ok_or_else(|| at_line(format!("`{second}` is not a code point (write it as 0x00C0 or U+00C0)")))?;
                    (index as usize, Slot::Code(cp))
                }
                None => {
                    let index = number(second).ok_or_else(|| at_line(format!("`{second}` is not a slot number")))?;
                    (index as usize, Slot::Name(glyph_name(first).map_err(at_line)?))
                }
            },
            _ => return Err(at_line("expected a glyph name and a slot, a slot and a code point, or one name or code point".into())),
        };
        enc.put(at, slot).map_err(at_line)?;
        next = at + 1;
    }
    Ok(enc)
}

/// The built-in encodings, by id.
pub const BUILTIN_ENCODINGS: [(&str, &str); 4] =
    [("ascii", "ASCII"), ("latin1", "ISO 8859-1 (Latin 1)"), ("windows1252", "Windows 1252 (Western)"), ("macRoman", "Mac OS Roman")];

/// Slots 0x80 to 0xFF of Windows code page 1252; 0 is an empty slot.
const WINDOWS_1252_HIGH: [u32; 128] = [
    0x20AC, 0x0000, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x0000, 0x017D, 0x0000, 0x0000, 0x2018,
    0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x0000, 0x017E, 0x0178, 0x00A0, 0x00A1, 0x00A2, 0x00A3,
    0x00A4, 0x00A5, 0x00A6, 0x00A7, 0x00A8, 0x00A9, 0x00AA, 0x00AB, 0x00AC, 0x00AD, 0x00AE, 0x00AF, 0x00B0, 0x00B1, 0x00B2, 0x00B3, 0x00B4, 0x00B5,
    0x00B6, 0x00B7, 0x00B8, 0x00B9, 0x00BA, 0x00BB, 0x00BC, 0x00BD, 0x00BE, 0x00BF, 0x00C0, 0x00C1, 0x00C2, 0x00C3, 0x00C4, 0x00C5, 0x00C6, 0x00C7,
    0x00C8, 0x00C9, 0x00CA, 0x00CB, 0x00CC, 0x00CD, 0x00CE, 0x00CF, 0x00D0, 0x00D1, 0x00D2, 0x00D3, 0x00D4, 0x00D5, 0x00D6, 0x00D7, 0x00D8, 0x00D9,
    0x00DA, 0x00DB, 0x00DC, 0x00DD, 0x00DE, 0x00DF, 0x00E0, 0x00E1, 0x00E2, 0x00E3, 0x00E4, 0x00E5, 0x00E6, 0x00E7, 0x00E8, 0x00E9, 0x00EA, 0x00EB,
    0x00EC, 0x00ED, 0x00EE, 0x00EF, 0x00F0, 0x00F1, 0x00F2, 0x00F3, 0x00F4, 0x00F5, 0x00F6, 0x00F7, 0x00F8, 0x00F9, 0x00FA, 0x00FB, 0x00FC, 0x00FD,
    0x00FE, 0x00FF,
];

/// Slots 0x80 to 0xFF of Mac OS Roman.
const MAC_ROMAN_HIGH: [u32; 128] = [
    0x00C4, 0x00C5, 0x00C7, 0x00C9, 0x00D1, 0x00D6, 0x00DC, 0x00E1, 0x00E0, 0x00E2, 0x00E4, 0x00E3, 0x00E5, 0x00E7, 0x00E9, 0x00E8, 0x00EA, 0x00EB,
    0x00ED, 0x00EC, 0x00EE, 0x00EF, 0x00F1, 0x00F3, 0x00F2, 0x00F4, 0x00F6, 0x00F5, 0x00FA, 0x00F9, 0x00FB, 0x00FC, 0x2020, 0x00B0, 0x00A2, 0x00A3,
    0x00A7, 0x2022, 0x00B6, 0x00DF, 0x00AE, 0x00A9, 0x2122, 0x00B4, 0x00A8, 0x2260, 0x00C6, 0x00D8, 0x221E, 0x00B1, 0x2264, 0x2265, 0x00A5, 0x00B5,
    0x2202, 0x2211, 0x220F, 0x03C0, 0x222B, 0x00AA, 0x00BA, 0x03A9, 0x00E6, 0x00F8, 0x00BF, 0x00A1, 0x00AC, 0x221A, 0x0192, 0x2248, 0x2206, 0x00AB,
    0x00BB, 0x2026, 0x00A0, 0x00C0, 0x00C3, 0x00D5, 0x0152, 0x0153, 0x2013, 0x2014, 0x201C, 0x201D, 0x2018, 0x2019, 0x00F7, 0x25CA, 0x00FF, 0x0178,
    0x2044, 0x20AC, 0x2039, 0x203A, 0xFB01, 0xFB02, 0x2021, 0x00B7, 0x201A, 0x201E, 0x2030, 0x00C2, 0x00CA, 0x00C1, 0x00CB, 0x00C8, 0x00CD, 0x00CE,
    0x00CF, 0x00CC, 0x00D3, 0x00D4, 0xF8FF, 0x00D2, 0x00DA, 0x00DB, 0x00D9, 0x0131, 0x02C6, 0x02DC, 0x00AF, 0x02D8, 0x02D9, 0x02DA, 0x00B8, 0x02DD,
    0x02DB, 0x02C7,
];

/// A built-in encoding by id or title (case ignored): 256 slots, the control characters empty.
pub fn builtin_encoding(name: &str) -> Option<Encoding> {
    let (id, title) = BUILTIN_ENCODINGS.iter().find(|(id, title)| id.eq_ignore_ascii_case(name) || title.eq_ignore_ascii_case(name))?;
    let high: Option<&[u32; 128]> = match *id {
        "windows1252" => Some(&WINDOWS_1252_HIGH),
        "macRoman" => Some(&MAC_ROMAN_HIGH),
        _ => None,
    };
    let latin1 = *id == "latin1";
    let slots = (0u32..256)
        .map(|i| {
            let cp = match i {
                0x20..=0x7E => i,
                0x80..=0xFF => match high {
                    Some(table) => table.get((i - 0x80) as usize).copied().unwrap_or(0),
                    None if latin1 && i >= 0xA0 => i,
                    None => 0,
                },
                _ => 0,
            };
            (cp != 0).then_some(Slot::Code(cp))
        })
        .collect();
    Some(Encoding { title: Some((*title).to_string()), slots })
}

/// The glyph name each slot stands for in `font`: the name a slot gives, or for a code point the
/// font's glyph with it (the first in the font's glyph list), else the glyph data's name for it.
pub fn slot_names(font: &Font, data: &GlyphData, enc: &Encoding) -> Vec<Option<String>> {
    let mut by_code: HashMap<u32, &str> = HashMap::new();
    for g in font.glyphs.iter() {
        for cp in &g.unicodes {
            by_code.entry(*cp).or_insert(g.name.as_str());
        }
    }
    enc.slots
        .iter()
        .map(|slot| match slot {
            None => None,
            Some(Slot::Name(n)) => Some(n.clone()),
            Some(Slot::Code(cp)) => Some(by_code.get(cp).map_or_else(|| data.nice_name(*cp), |n| (*n).to_string())),
        })
        .collect()
}

// ── Name lists ──────────────────────────────────────────────────────────────────────────────────

/// A name list: names by code point, and names given without one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NameList {
    pub title: Option<String>,
    pub names: BTreeMap<u32, String>,
    pub bare: Vec<String>,
}

/// Reads a name list (see the module docs for its line forms).
pub fn parse_namelist(bytes: &[u8]) -> std::result::Result<NameList, String> {
    let text = decode(bytes, "name list")?;
    let mut list = NameList::default();
    let mut entries = 0usize;
    for (n, raw) in numbered_lines(text, "name list")? {
        let at_line = |e: String| format!("line {n}: {e}");
        let trimmed = raw.trim();
        if trimmed.starts_with("%%") {
            if list.title.is_none() {
                list.title = header_title(trimmed, &["NAMETABLE"]).map_err(at_line)?;
            }
            continue;
        }
        let line = without_comment(trimmed);
        if line.is_empty() || line.get(..5).is_some_and(|k| k.eq_ignore_ascii_case("lang:")) {
            continue;
        }
        if line.get(..6).is_some_and(|k| k.eq_ignore_ascii_case("based:")) {
            return Err(at_line("this list extends another one (`Based:`), which is not supported; give the full list".into()));
        }
        entries += 1;
        if entries > MAX_NAMES {
            return Err(format!("the name list has more than {MAX_NAMES} entries"));
        }
        if let Some((name, codes)) = line.split_once(';') {
            let name = glyph_name(name.trim()).map_err(at_line)?;
            let codes: Vec<&str> = codes.split_whitespace().take(2).collect();
            match codes.as_slice() {
                [one] => {
                    let cp = bare_code_point(one).ok_or_else(|| at_line(format!("`{one}` is not a code point")))?;
                    list.names.entry(cp).or_insert(name);
                }
                // A name for a sequence of characters names no single code point.
                [_, _] => {}
                _ => return Err(at_line("a code point must follow the `;`".into())),
            }
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().take(3).collect();
        match tokens.as_slice() {
            [one] => list.bare.push(glyph_name(one).map_err(at_line)?),
            [code, name] => {
                let cp = code_point(code).ok_or_else(|| at_line(format!("`{code}` is not a code point (write it as 0x00C0 or U+00C0)")))?;
                let name = glyph_name(name).map_err(at_line)?;
                list.names.entry(cp).or_insert(name);
            }
            _ => return Err(at_line("expected a code point and a name".into())),
        }
    }
    Ok(list)
}

/// The built-in name lists.
pub const BUILTIN_NAMELISTS: [&str; 2] = ["nice", "production"];

/// Where renamed glyphs take their names from.
enum Names {
    File(NameList),
    Nice,
    Production,
}

impl Names {
    fn name_for(&self, data: &GlyphData, cp: u32) -> Option<String> {
        match self {
            Names::File(list) => list.names.get(&cp).cloned(),
            Names::Nice => Some(data.nice_name(cp)),
            Names::Production => Some(data.production_name(&data.nice_name(cp), &[cp])),
        }
    }
}

fn read_namelist(p: &Params, source: &str) -> Result<Names> {
    match source {
        "nice" => Ok(Names::Nice),
        "production" => Ok(Names::Production),
        path => {
            let bytes = read_text_file(path)?;
            let list = parse_namelist(&bytes).map_err(|e| p.bad(format!("{path}: {e}")))?;
            if list.names.is_empty() {
                return Err(p.bad(format!("{path} names no code points")));
            }
            Ok(Names::File(list))
        }
    }
}

/// The renames a name list asks for among `targets`: encoded glyphs by their first listed code
/// point, then unencoded glyphs named after a renamed one plus a suffix.
fn planned_renames(font: &Font, data: &GlyphData, names: &Names, targets: &[String]) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut bases: HashMap<&str, String> = HashMap::new();
    for name in targets {
        let Some(g) = font.glyph(name) else { continue };
        let Some(new) = g.unicodes.iter().find_map(|cp| names.name_for(data, *cp)) else { continue };
        if new != *name {
            bases.insert(name.as_str(), new.clone());
            pairs.push((name.clone(), new));
        }
    }
    for name in targets {
        let Some(g) = font.glyph(name) else { continue };
        if !g.unicodes.is_empty() {
            continue;
        }
        let Some((base, suffix)) = name.split_once('.') else { continue };
        if let Some(new_base) = bases.get(base) {
            pairs.push((name.clone(), format!("{new_base}.{suffix}")));
        }
    }
    pairs
}

/// Renames glyphs and nothing else: references keep the old names.
fn rename_names_only(font: &mut Font, pairs: &[(String, String)]) -> Result<RenameReport> {
    let map: HashMap<&str, &str> = pairs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let glyphs = font
        .glyphs
        .as_arcs()
        .iter()
        .map(|g| {
            let mut arc = Arc::clone(g);
            if let Some(new) = map.get(g.name.as_str()) {
                Arc::make_mut(&mut arc).name = (*new).to_string();
            }
            arc
        })
        .collect();
    font.glyphs = GlyphTable::from_glyphs(glyphs);
    Ok(RenameReport { renamed: pairs.iter().map(|(a, b)| Renamed { from: a.clone(), to: b.clone() }).collect(), ..RenameReport::default() })
}

fn rename_by_namelist(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.renameGlyphsByNamelist", v)?;
    let source = p.str("namelist")?;
    let update = p.bool_or("updateReferences", true)?;
    let names = read_namelist(&p, source)?;
    let data = s.glyph_data()?;
    let font = s.font()?;
    let targets = match p.opt_glyphs("glyphs", font)? {
        Some(list) => list,
        None => font.glyphs.iter().map(|g| g.name.clone()).collect(),
    };
    let pairs = planned_renames(font, &data, &names, &targets);
    if pairs.is_empty() {
        return Ok(json!({"renamed": [], "updated": [], "stillUsed": []}));
    }
    // Every new name is checked before anything is renamed: a list with a problem is refused whole.
    let renamed: HashSet<&str> = pairs.iter().map(|(old, _)| old.as_str()).collect();
    let mut taken: HashSet<&str> = font.glyphs.iter().map(|g| g.name.as_str()).filter(|n| !renamed.contains(n)).collect();
    let mut problems = Vec::new();
    for (old, new) in &pairs {
        let exports = font.glyph(old).is_none_or(|g| g.export);
        if new.len() > MAX_NAME_LEN {
            problems.push(format!("`{old}` to a name longer than {MAX_NAME_LEN} bytes"));
        } else if let Err(e) = validate(new, exports) {
            problems.push(format!("`{old}` to `{new}`: {e}"));
        } else if !taken.insert(new.as_str()) {
            problems.push(format!("`{old}` to `{new}`: the name is taken"));
        }
    }
    if !problems.is_empty() {
        let count = problems.len();
        let listed: Vec<String> = problems.into_iter().take(MAX_LISTED_PROBLEMS).collect();
        let more = if count > listed.len() { format!(" and {} more", count - listed.len()) } else { String::new() };
        return Err(p.bad(format!("nothing was renamed: {}{more}", listed.join("; "))));
    }
    let label = "Rename Glyphs by Name List";
    let report =
        if update { s.edit(label, |font| Ok(rename_glyphs(font, &pairs)?))? } else { s.edit(label, |font| rename_names_only(font, &pairs))? };
    let refs = |list: &[fontcraft_model::GlyphReference]| Value::Array(list.iter().map(|x| json!({"name": x.name, "place": x.place})).collect());
    let renamed: Vec<Value> = report.renamed.iter().map(|r| json!({"from": r.from, "to": r.to})).collect();
    Ok(json!({"renamed": renamed, "updated": refs(&report.updated), "stillUsed": refs(&report.still_used)}))
}

// ── The comparison table ────────────────────────────────────────────────────────────────────────

/// The properties a comparison table can show: id and column caption.
pub const COMPARE_PROPERTIES: [(&str, &str); 8] = [
    ("width", "Width"),
    ("lsb", "Left Sidebearing"),
    ("rsb", "Right Sidebearing"),
    ("nodes", "Nodes"),
    ("paths", "Paths"),
    ("components", "Components"),
    ("anchors", "Anchors"),
    ("unicodes", "Unicode"),
];

/// One glyph's row: the property in each font (`null` where the font lacks the glyph or the
/// value), and whether the fonts disagree.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CompareRow {
    pub glyph: String,
    pub values: Vec<Value>,
    pub differ: bool,
}

/// The documents `refs` names: positions in the open-document list, titles or file paths. `None`
/// is every open document. Refuses an unknown, repeated or excess reference.
pub fn resolve_documents(s: &Session, refs: Option<&Value>) -> std::result::Result<Vec<u64>, String> {
    let docs = s.documents();
    let Some(refs) = refs.filter(|v| !v.is_null()) else {
        return Ok(docs.iter().take(MAX_COMPARE_FONTS).map(|d| d.id).collect());
    };
    let list = refs.as_array().ok_or("`fonts` must be a list of open documents (positions, titles or paths)")?;
    if list.len() > MAX_COMPARE_FONTS {
        return Err(format!("`fonts` holds {} documents; at most {MAX_COMPARE_FONTS} are compared", list.len()));
    }
    let mut out = Vec::with_capacity(list.len());
    for r in list {
        let doc = match r {
            Value::Number(n) => n.as_u64().and_then(|i| usize::try_from(i).ok()).and_then(|i| docs.get(i)),
            Value::String(t) => docs
                .iter()
                .find(|d| d.title == *t)
                .or_else(|| docs.iter().find(|d| d.path.as_deref().is_some_and(|p| p.to_string_lossy() == t.as_str()))),
            _ => None,
        };
        let doc = doc.ok_or_else(|| format!("{r} is not an open document"))?;
        if out.contains(&doc.id) {
            return Err(format!("{r} is listed twice"));
        }
        out.push(doc.id);
    }
    Ok(out)
}

/// A measured value rounded to hundredths of a unit, so tiny float differences do not count.
fn measured(v: f64) -> Value {
    if v.is_finite() { json!((v * 100.0).round() / 100.0) } else { Value::Null }
}

fn layer_value(font: &Font, layer: &Layer, property: &str) -> Value {
    let count = |f: fn(&Shape) -> usize| json!(layer.shapes.iter().map(f).sum::<usize>());
    match property {
        "width" => measured(layer.width),
        "lsb" | "rsb" => match fontcraft_spacing::measure::bounds(font, layer) {
            Ok(Some(r)) if property == "lsb" => measured(r.x0),
            Ok(Some(r)) => measured(layer.width - r.x1),
            _ => Value::Null,
        },
        "nodes" => count(|s| if let Shape::Path(p) = s { p.nodes.len() } else { 0 }),
        "paths" => count(|s| usize::from(matches!(s, Shape::Path(_)))),
        "components" => count(|s| usize::from(matches!(s, Shape::Component(_)))),
        "anchors" => json!(layer.anchors.len()),
        _ => Value::Null,
    }
}

/// One value of the table: `property` of glyph `name` in the first master of `font`.
fn cell(font: &Font, name: &str, property: &str) -> Value {
    let Some(g) = font.glyph(name) else { return Value::Null };
    if property == "unicodes" {
        return json!(g.unicodes.iter().map(|c| format!("{c:04X}")).collect::<Vec<_>>().join(" "));
    }
    let Some(layer) = font.masters.first().and_then(|m| g.layer(&m.id)) else { return Value::Null };
    layer_value(font, layer, property)
}

/// The comparison table of `property` across `fonts`: one row per glyph name any font has, in the
/// first font's glyph list order, then each further font's extra glyphs in its order.
pub fn compare_table(fonts: &[&Font], property: &str) -> std::result::Result<Vec<CompareRow>, String> {
    if !COMPARE_PROPERTIES.iter().any(|(id, _)| *id == property) {
        let ids: Vec<&str> = COMPARE_PROPERTIES.iter().map(|(id, _)| *id).collect();
        return Err(format!("`{property}` is not a property to compare; use one of {}", ids.join(", ")));
    }
    if fonts.len() > MAX_COMPARE_FONTS {
        return Err(format!("at most {MAX_COMPARE_FONTS} fonts are compared"));
    }
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for font in fonts {
        for g in font.glyphs.iter() {
            if seen.insert(g.name.as_str()) {
                names.push(g.name.as_str());
            }
        }
    }
    Ok(names
        .into_iter()
        .map(|name| {
            let values: Vec<Value> = fonts.iter().map(|f| cell(f, name, property)).collect();
            let differ = values.windows(2).any(|w| w.first() != w.get(1));
            CompareRow { glyph: name.to_string(), values, differ }
        })
        .collect())
}

/// [`compare_table`] over open documents by id, as the JSON `font.compareInTable` returns:
/// `{fonts: [titles], property, rows: [{glyph, values, differ}]}`.
pub fn compare_documents(s: &Session, ids: &[u64], property: &str) -> std::result::Result<Value, String> {
    let docs: Vec<&crate::DocState> = ids.iter().filter_map(|id| s.documents().iter().find(|d| d.id == *id)).collect();
    let fonts: Vec<&Font> = docs.iter().map(|d| d.font.as_ref()).collect();
    let rows = compare_table(&fonts, property)?;
    let titles: Vec<&str> = docs.iter().map(|d| d.title.as_str()).collect();
    Ok(json!({"fonts": titles, "property": property, "rows": rows}))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn an_encoding_file_keeps_empty_slots_in_place() {
        let text = "%%FONTLAB ENCODING: 1; Test Set\n% a comment\nspace 32\nA 65\nB\n0x44 0x0044 % D by code point\nU+0045\nzero 0x48\n";
        let enc = parse_encoding(text.as_bytes()).unwrap();
        assert_eq!(enc.title.as_deref(), Some("Test Set"));
        assert_eq!(enc.slots.len(), 0x49);
        assert_eq!(enc.slots[32], Some(Slot::Name("space".into())));
        assert_eq!(enc.slots[65], Some(Slot::Name("A".into())));
        assert_eq!(enc.slots[66], Some(Slot::Name("B".into())), "a lone name takes the next slot");
        assert_eq!(enc.slots[0x44], Some(Slot::Code(0x44)));
        assert_eq!(enc.slots[0x45], Some(Slot::Code(0x45)));
        assert_eq!(enc.slots[0x48], Some(Slot::Name("zero".into())));
        assert_eq!(enc.slots[0], None);
        assert_eq!(enc.slots[0x46], None, "a gap stays an empty slot");
        assert_eq!(enc.filled(), 6);

        let font = Font::new("Test");
        let data = GlyphData::new().unwrap();
        let names = slot_names(&font, &data, &enc);
        assert_eq!(names[0x44].as_deref(), Some("D"), "a code point names the font's glyph for it");
        assert_eq!(names[0x47], None);
    }

    #[test]
    fn untrusted_encoding_and_name_list_files_are_refused_with_the_line() {
        assert!(parse_encoding(b"A 65\nB 65\n").unwrap_err().contains("line 2"), "a slot filled twice");
        assert!(parse_encoding(b"A 70000\n").unwrap_err().contains("past the last allowed slot"));
        assert!(parse_encoding(b"A B C\n").unwrap_err().starts_with("line 1"));
        assert!(parse_encoding(b"A\xff 65\n").unwrap_err().contains("not UTF-8"));
        assert!(parse_encoding(b"0x41 0xD800\n").unwrap_err().contains("not a code point"), "a surrogate is no code point");
        assert!(parse_encoding(b"A/B 65\n").unwrap_err().contains("not a glyph name"));
        let many = "A\n".repeat(MAX_LINES + 1);
        assert!(parse_encoding(many.as_bytes()).unwrap_err().contains("lines"));
        assert!(parse_namelist(b"Based: other\n").unwrap_err().contains("Based"));
        assert!(parse_namelist(b"0x41 A B\n").is_err());
    }

    #[test]
    fn name_lists_read_every_layout_and_keep_the_first_name() {
        let text = "%%FONTLAB NAMETABLE: Mine\n# comment\nLang: en\n0x0041 Alpha\nU+0042 Beta\nGamma;0043\nOther;0041\nlig;0066 0069\nbare\n";
        let list = parse_namelist(text.as_bytes()).unwrap();
        assert_eq!(list.title.as_deref(), Some("Mine"));
        assert_eq!(list.names.get(&0x41).map(String::as_str), Some("Alpha"), "the first name for a code point counts");
        assert_eq!(list.names.get(&0x42).map(String::as_str), Some("Beta"));
        assert_eq!(list.names.get(&0x43).map(String::as_str), Some("Gamma"));
        assert_eq!(list.names.len(), 3, "a sequence names no code point");
        assert_eq!(list.bare, vec!["bare".to_string()]);
    }

    #[test]
    fn builtin_encodings_follow_the_code_pages() {
        let w = builtin_encoding("windows1252").unwrap();
        assert_eq!(w.slots.len(), 256);
        assert_eq!(w.slots[0x80], Some(Slot::Code(0x20AC)));
        assert_eq!(w.slots[0x81], None);
        assert_eq!(w.slots[0x41], Some(Slot::Code(0x41)));
        assert_eq!(w.slots[0x0A], None);
        let m = builtin_encoding("Mac OS Roman").unwrap();
        assert_eq!(m.slots[0xDE], Some(Slot::Code(0xFB01)));
        let l = builtin_encoding("latin1").unwrap();
        assert_eq!(l.slots[0x85], None);
        assert_eq!(l.slots[0xE9], Some(Slot::Code(0xE9)));
        assert!(builtin_encoding("nothing").is_none());
    }
}
