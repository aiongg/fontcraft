//! Built-in language and glyph-set data: which characters each language needs (Font View's
//! Languages section and its missing-glyph badges) and the glyph sets a new document can start
//! from (the Start Window's chooser).
//!
//! The languages are the main exemplar characters of the Unicode CLDR locales, with the
//! single-character uppercase form of each lowercase letter added, grouped under the glyph
//! database's script ids. The glyph sets are the Google Fonts glyph sets. `cargo xtask glyphsets
//! --generate` compiles both into `data/sets.bin` (sources and licences: `xtask/glyphsets.toml`,
//! `ASSETS.md`, `NOTICE`); it is decoded once, on first use.
//!
//! The blob uses the container of [`crate::blob`] with these sections (integers little-endian; a
//! string is a length byte and UTF-8 bytes):
//!
//! | Tag | Content |
//! |---|---|
//! | `SETV` | u16: the layout version of the other sections, [`SETS_VERSION`] |
//! | `META` | UTF-8 text, `key=value` lines: the sources |
//! | `LANG` | u32 count, then per language: id, English name, script id (strings), u32 character count, the code points ascending as LEB128 deltas (the first from 0) |
//! | `GLYF` | u32 count, then per glyph: name (string), code point (u24, `FFFFFF` none) |
//! | `GSET` | u32 count, then per set: id, name, script id (strings), flags (u8, bit 0: picked by default), u32 glyph count, that many u16 indexes into `GLYF` |
//!
//! Every read is bounds-checked and every count capped: a damaged blob gives an error, never a
//! panic.

use std::sync::OnceLock;

use crate::Error;
use crate::blob::{Payload, u8_at, u16_at, u24_at, u32_at};

/// The layout version of the sections; the reader refuses others.
pub const SETS_VERSION: u16 = 1;
/// Most languages the reader accepts.
pub const MAX_LANGUAGES: usize = 4096;
/// Most characters one language may list.
pub const MAX_LANGUAGE_CHARACTERS: usize = 65_536;
/// Most distinct glyphs all glyph sets together may hold (they are indexed by u16).
pub const MAX_SET_GLYPHS: usize = 65_535;
/// Most glyph sets the reader accepts.
pub const MAX_GLYPH_SETS: usize = 1024;
/// Most glyphs one glyph set may list.
pub const MAX_GLYPHS_PER_SET: usize = 65_535;
/// Glyph set flag: picked when a script is chosen without naming its sets.
pub const FLAG_DEFAULT: u8 = 1;

const NONE_U24: u32 = 0xFF_FFFF;
const MAX_CODE_POINT: u32 = 0x10_FFFF;

/// A language (a CLDR locale) and the characters it needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Language {
    /// The locale id with `-` between subtags (`de`, `sr-Latn`, `zh-Hant`).
    pub id: String,
    /// The English name (`German`, `Serbian (Latin)`).
    pub name: String,
    /// The glyph database's script id it is listed under (`latin`).
    pub script: String,
    /// The code points, ascending, each once.
    pub characters: Vec<u32>,
}

/// A glyph of a glyph set: its name in the source and its code point, if it has one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SetGlyph {
    pub name: String,
    pub code_point: Option<u32>,
}

/// A glyph set the Start Window offers under a script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphSet {
    /// A short id (`latin-core`).
    pub id: String,
    /// The name shown (`Latin Core`).
    pub name: String,
    /// The glyph database's script id it is listed under (`latin`).
    pub script: String,
    /// Picked when its script is chosen without naming sets.
    pub default: bool,
    /// The glyphs, in the source's order.
    pub glyphs: Vec<SetGlyph>,
}

/// The decoded data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sets {
    /// Where the data came from, `key=value` lines.
    pub meta: String,
    /// Sorted by script, then name, then id.
    pub languages: Vec<Language>,
    /// In the order the chooser lists them.
    pub glyph_sets: Vec<GlyphSet>,
}

static BLOB: &[u8] = include_bytes!("../data/sets.bin");
static SETS: OnceLock<Result<Sets, Error>> = OnceLock::new();

/// The built-in data, decoded on first use.
pub fn builtin() -> Result<&'static Sets, Error> {
    match SETS.get_or_init(|| Sets::open(BLOB)) {
        Ok(s) => Ok(s),
        Err(e) => Err(e.clone()),
    }
}

impl Sets {
    /// The glyph set with this id.
    pub fn glyph_set(&self, id: &str) -> Option<&GlyphSet> {
        self.glyph_sets.iter().find(|s| s.id == id)
    }

    /// The scripts that have glyph sets, in the order of their first set.
    pub fn glyph_set_scripts(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for s in &self.glyph_sets {
            if !out.contains(&s.script.as_str()) {
                out.push(&s.script);
            }
        }
        out
    }

    /// Decodes a blob written by [`Sets::write`].
    pub fn open(blob: &[u8]) -> Result<Sets, Error> {
        let payload = Payload::open(blob)?;
        let version = payload.section(b"SETV").and_then(|s| u16_at(s, 0)).ok_or_else(|| bad("missing section SETV"))?;
        if version != SETS_VERSION {
            return Err(Error::Blob(format!("language and glyph-set data version {version}, expected {SETS_VERSION}")));
        }
        let meta = match payload.section(b"META") {
            Some(m) => String::from_utf8(m.to_vec()).map_err(|_| bad("META is not UTF-8"))?,
            None => String::new(),
        };
        let languages = read_languages(section(&payload, b"LANG")?)?;
        let glyphs = read_glyphs(section(&payload, b"GLYF")?)?;
        let glyph_sets = read_glyph_sets(section(&payload, b"GSET")?, &glyphs)?;
        Ok(Sets { meta, languages, glyph_sets })
    }
}

fn bad(m: &str) -> Error {
    Error::Blob(m.to_string())
}

fn section<'p>(payload: &'p Payload, tag: &[u8; 4]) -> Result<&'p [u8], Error> {
    payload.section(tag).ok_or_else(|| Error::Blob(format!("missing section {}", String::from_utf8_lossy(tag))))
}

/// A reading position in one section.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    tag: &'static str,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], tag: &'static str) -> Self {
        Reader { bytes, at: 0, tag }
    }

    fn short(&self) -> Error {
        Error::Blob(format!("section {} is short", self.tag))
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.at)
    }

    fn u8(&mut self) -> Result<u8, Error> {
        let v = u8_at(self.bytes, self.at).ok_or_else(|| self.short())?;
        self.at += 1;
        Ok(v)
    }

    fn u16(&mut self) -> Result<u16, Error> {
        let v = u16_at(self.bytes, self.at).ok_or_else(|| self.short())?;
        self.at += 2;
        Ok(v)
    }

    fn u24(&mut self) -> Result<u32, Error> {
        let v = u24_at(self.bytes, self.at).ok_or_else(|| self.short())?;
        self.at += 3;
        Ok(v)
    }

    fn u32(&mut self) -> Result<u32, Error> {
        let v = u32_at(self.bytes, self.at).ok_or_else(|| self.short())?;
        self.at += 4;
        Ok(v)
    }

    /// A count that must not exceed `limit`, nor the bytes left at `min_size` bytes per item.
    fn count(&mut self, limit: usize, min_size: usize, what: &str) -> Result<usize, Error> {
        let n = self.u32()? as usize;
        if n > limit {
            return Err(Error::Blob(format!("{n} {what} is over the limit of {limit}")));
        }
        if n.saturating_mul(min_size) > self.remaining() {
            return Err(self.short());
        }
        Ok(n)
    }

    fn string(&mut self) -> Result<String, Error> {
        let len = usize::from(self.u8()?);
        let end = self.at.checked_add(len).ok_or_else(|| self.short())?;
        let bytes = self.bytes.get(self.at..end).ok_or_else(|| self.short())?;
        self.at = end;
        String::from_utf8(bytes.to_vec()).map_err(|_| Error::Blob(format!("section {} holds text that is not UTF-8", self.tag)))
    }

    /// A LEB128 number of at most three bytes (21 bits, enough for a code point).
    fn varint(&mut self) -> Result<u32, Error> {
        let mut v = 0u32;
        for shift in [0u32, 7, 14] {
            let b = self.u8()?;
            v |= u32::from(b & 0x7F) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(Error::Blob(format!("section {} holds a number longer than three bytes", self.tag)))
    }

    fn done(&self) -> Result<(), Error> {
        if self.at == self.bytes.len() { Ok(()) } else { Err(Error::Blob(format!("section {} has bytes after its data", self.tag))) }
    }
}

fn read_languages(bytes: &[u8]) -> Result<Vec<Language>, Error> {
    let mut r = Reader::new(bytes, "LANG");
    // An id, a name and a script of at least one byte each, and a count.
    let n = r.count(MAX_LANGUAGES, 10, "languages")?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let id = r.string()?;
        let name = r.string()?;
        let script = r.string()?;
        let count = r.count(MAX_LANGUAGE_CHARACTERS, 1, "characters of one language")?;
        let mut characters = Vec::with_capacity(count);
        let mut prev: Option<u32> = None;
        for _ in 0..count {
            let delta = r.varint()?;
            let cp = match prev {
                None => delta,
                Some(p) if delta > 0 => p.checked_add(delta).ok_or_else(|| bad("a code point overflows"))?,
                Some(_) => return Err(Error::Blob(format!("language {id}: code points are not ascending"))),
            };
            if cp > MAX_CODE_POINT {
                return Err(Error::Blob(format!("language {id}: {cp:X} is not a code point")));
            }
            characters.push(cp);
            prev = Some(cp);
        }
        out.push(Language { id, name, script, characters });
    }
    r.done()?;
    Ok(out)
}

fn read_glyphs(bytes: &[u8]) -> Result<Vec<SetGlyph>, Error> {
    let mut r = Reader::new(bytes, "GLYF");
    let n = r.count(MAX_SET_GLYPHS, 5, "glyphs")?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let name = r.string()?;
        if name.is_empty() {
            return Err(bad("a glyph of a glyph set has no name"));
        }
        let cp = r.u24()?;
        let code_point = match cp {
            NONE_U24 => None,
            cp if cp <= MAX_CODE_POINT => Some(cp),
            cp => return Err(Error::Blob(format!("glyph {name}: {cp:X} is not a code point"))),
        };
        out.push(SetGlyph { name, code_point });
    }
    r.done()?;
    Ok(out)
}

fn read_glyph_sets(bytes: &[u8], glyphs: &[SetGlyph]) -> Result<Vec<GlyphSet>, Error> {
    let mut r = Reader::new(bytes, "GSET");
    let n = r.count(MAX_GLYPH_SETS, 8, "glyph sets")?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let id = r.string()?;
        let name = r.string()?;
        let script = r.string()?;
        let flags = r.u8()?;
        let count = r.count(MAX_GLYPHS_PER_SET, 2, "glyphs of one glyph set")?;
        let mut list = Vec::with_capacity(count);
        for _ in 0..count {
            let i = usize::from(r.u16()?);
            let g = glyphs.get(i).ok_or_else(|| Error::Blob(format!("glyph set {id} names glyph {i}, which does not exist")))?;
            list.push(g.clone());
        }
        out.push(GlyphSet { id, name, script, default: flags & FLAG_DEFAULT != 0, glyphs: list });
    }
    r.done()?;
    Ok(out)
}

// ── Writing (the generator and tests) ───────────────────────────────────────────────────────────

#[cfg(any(test, feature = "build"))]
fn push_string(out: &mut Vec<u8>, s: &str, what: &str) -> Result<(), String> {
    let len = u8::try_from(s.len()).map_err(|_| format!("{what} `{s}` is longer than 255 bytes"))?;
    out.push(len);
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

#[cfg(any(test, feature = "build"))]
fn push_varint(out: &mut Vec<u8>, mut v: u32) {
    while v >= 0x80 {
        out.push((v & 0x7F) as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

#[cfg(any(test, feature = "build"))]
fn push_count(out: &mut Vec<u8>, n: usize, limit: usize, what: &str) -> Result<(), String> {
    if n > limit {
        return Err(format!("{n} {what} is over the limit of {limit}"));
    }
    crate::blob::push_u32(out, n as u32);
    Ok(())
}

#[cfg(any(test, feature = "build"))]
impl Sets {
    /// The blob [`Sets::open`] reads. Glyphs shared by several sets are stored once. Fails when
    /// the data breaks the reader's limits.
    pub fn write(&self) -> Result<Vec<u8>, String> {
        use crate::blob::{Writer, push_u16, push_u24};
        use std::collections::HashMap;

        let mut lang = Vec::new();
        push_count(&mut lang, self.languages.len(), MAX_LANGUAGES, "languages")?;
        for l in &self.languages {
            for (s, what) in [(&l.id, "language id"), (&l.name, "language name"), (&l.script, "script id")] {
                if s.is_empty() {
                    return Err(format!("language `{}`: an empty {what}", l.id));
                }
                push_string(&mut lang, s, what)?;
            }
            push_count(&mut lang, l.characters.len(), MAX_LANGUAGE_CHARACTERS, "characters of one language")?;
            let mut prev: Option<u32> = None;
            for &cp in &l.characters {
                if cp > MAX_CODE_POINT || prev.is_some_and(|p| cp <= p) {
                    return Err(format!("language `{}`: the characters must be code points, ascending and distinct", l.id));
                }
                push_varint(&mut lang, cp - prev.unwrap_or(0));
                prev = Some(cp);
            }
        }

        let mut index: HashMap<&SetGlyph, u16> = HashMap::new();
        let mut table: Vec<&SetGlyph> = Vec::new();
        let mut gset = Vec::new();
        push_count(&mut gset, self.glyph_sets.len(), MAX_GLYPH_SETS, "glyph sets")?;
        for s in &self.glyph_sets {
            for (v, what) in [(&s.id, "glyph set id"), (&s.name, "glyph set name"), (&s.script, "script id")] {
                if v.is_empty() {
                    return Err(format!("glyph set `{}`: an empty {what}", s.id));
                }
                push_string(&mut gset, v, what)?;
            }
            gset.push(if s.default { FLAG_DEFAULT } else { 0 });
            push_count(&mut gset, s.glyphs.len(), MAX_GLYPHS_PER_SET, "glyphs of one glyph set")?;
            for g in &s.glyphs {
                let i = match index.get(g) {
                    Some(i) => *i,
                    None => {
                        let i = u16::try_from(table.len()).map_err(|_| format!("more than {MAX_SET_GLYPHS} distinct glyphs"))?;
                        index.insert(g, i);
                        table.push(g);
                        i
                    }
                };
                push_u16(&mut gset, i);
            }
        }
        let mut glyf = Vec::new();
        push_count(&mut glyf, table.len(), MAX_SET_GLYPHS, "glyphs")?;
        for g in table {
            if g.name.is_empty() {
                return Err("a glyph of a glyph set has no name".into());
            }
            push_string(&mut glyf, &g.name, "glyph name")?;
            match g.code_point {
                Some(cp) if cp <= MAX_CODE_POINT => push_u24(&mut glyf, cp)?,
                Some(cp) => return Err(format!("glyph `{}`: {cp:X} is not a code point", g.name)),
                None => push_u24(&mut glyf, NONE_U24)?,
            }
        }

        let mut version = Vec::new();
        push_u16(&mut version, SETS_VERSION);
        let mut w = Writer::default();
        w.add(b"SETV", version);
        w.add(b"META", self.meta.as_bytes().to_vec());
        w.add(b"LANG", lang);
        w.add(b"GLYF", glyf);
        w.add(b"GSET", gset);
        w.finish()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;

    fn synthetic() -> Sets {
        let a = SetGlyph { name: "A".into(), code_point: Some(0x41) };
        let alt = SetGlyph { name: "a.ss01".into(), code_point: None };
        Sets {
            meta: "cldr=test\n".into(),
            languages: vec![
                Language { id: "de".into(), name: "German".into(), script: "latin".into(), characters: vec![0x41, 0x61, 0xC4, 0xE4, 0x1E9E] },
                Language { id: "sr-Latn".into(), name: "Serbian (Latin)".into(), script: "latin".into(), characters: vec![0x10FFFF] },
            ],
            glyph_sets: vec![
                GlyphSet { id: "latin-kernel".into(), name: "Latin Kernel".into(), script: "latin".into(), default: false, glyphs: vec![a.clone()] },
                GlyphSet { id: "latin-core".into(), name: "Latin Core".into(), script: "latin".into(), default: true, glyphs: vec![a, alt] },
            ],
        }
    }

    #[test]
    fn a_synthetic_set_round_trips_and_damage_fails_cleanly() {
        let sets = synthetic();
        let blob = sets.write().unwrap();
        assert_eq!(Sets::open(&blob).unwrap(), sets);
        assert_eq!(sets.glyph_set_scripts(), vec!["latin"]);
        for cut in 0..blob.len() {
            let _ = Sets::open(&blob[..cut]);
        }
        for i in 0..blob.len() {
            let mut b = blob.clone();
            b[i] ^= 0xFF;
            let _ = Sets::open(&b);
        }
    }

    #[test]
    fn the_writer_refuses_what_the_reader_would() {
        let mut s = synthetic();
        s.languages[0].characters = vec![0x61, 0x41];
        assert!(s.write().is_err(), "descending characters");
        let mut s = synthetic();
        s.glyph_sets[0].id = "x".repeat(300);
        assert!(s.write().is_err(), "an id over 255 bytes");
    }

    #[test]
    fn the_builtin_data_loads_with_languages_and_glyph_sets() {
        let sets = builtin().unwrap();
        assert!(sets.languages.iter().any(|l| l.id == "de" && l.script == "latin" && l.characters.contains(&0xC4)), "German needs Ä");
        assert!(sets.glyph_sets.iter().any(|s| s.script == "latin" && s.default));
    }
}
