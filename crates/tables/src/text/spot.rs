//! The spot table listings (`docs/spec/20-cli.md` › dump, table views): the text AFDKO `spot -t
//! TAG=LEVEL` prints for the levels below. Ported from AFDKO `c/spot` (`cmap.c`, `hmtx.c`,
//! `name.c`, `tto.c`, `glyf.c`, `VORG.c`, `kern.c`; Apache-2.0, Copyright 2014 Adobe Systems
//! Incorporated).
//!
//! | Table | View | spot level |
//! |---|---|---|
//! | cmap | `hex-gid`, `dec-gid`, `hex-name`, `dec-name`, `summary` | 5, 6, 7, 8, 11 |
//! | hmtx | `advances`, `advances-lsb` (`units_1000` for 6 and 8) | 5 or 6, 7 or 8 |
//! | name | `raw`, `lines`, `described` | 2, 3, 4 |
//! | GSUB, GPOS | `features` | 5 |
//! | glyf | `points` | 5 |
//! | VORG | `names` | 4 |
//! | kern | `pairs` | 5 |
//!
//! Glyph names are those of [`crate::ttx::glyph_order`] without spot's `@index` suffix (spot
//! `-ngid`); a CID-keyed font names its glyphs `\CID`, as spot does. Where spot reads past the
//! data it was given (the sentinel pair of a `kern` format 0 subtable, `hmtx` entries past the
//! table), the listing stops at the data instead.

use read_fonts::TableProvider;
use read_fonts::tables::glyf::Glyph;
use read_fonts::types::GlyphId;

use super::spot_desc as desc;
use super::{Out, ViewOptions};
use crate::features::Provider;
use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// The listing of `tag` in `view`; `None` when the table has no such spot listing.
pub(crate) fn listing(font: &SfntFont, tag: Tag, view: &str, options: &ViewOptions) -> Result<Option<String>> {
    let level = match (&tag, view) {
        (b"cmap", "hex-gid") => 5,
        (b"cmap", "dec-gid") => 6,
        (b"cmap", "hex-name") => 7,
        (b"cmap", "dec-name") => 8,
        (b"cmap", "summary") => 11,
        (b"hmtx", "advances") => 5,
        (b"hmtx", "advances-lsb") => 7,
        (b"name", "raw") => 2,
        (b"name", "lines") => 3,
        (b"name", "described") => 4,
        (b"GSUB" | b"GPOS", "features") => 5,
        (b"glyf", "points") => 5,
        (b"VORG", "names") => 4,
        (b"kern", "pairs") => 5,
        _ => return Ok(None),
    };
    let data = font.get(tag).ok_or_else(|| TablesError::MissingTable(crate::sfnt::tag_str(tag)))?;
    let offset = options.table_offsets.iter().find(|(t, _)| *t == tag).map_or(0, |(_, o)| *o);
    let mut out = Out::new();
    match &tag {
        b"cmap" if level == 11 => cmap_summary(data, &mut out)?,
        b"cmap" => cmap_mappings(data, &Names::new(font), level, options.cmap_subtable, &mut out)?,
        b"hmtx" => hmtx(font, data, level + u8::from(options.units_1000), &Names::new(font), &mut out)?,
        b"name" => name(data, level, offset, &mut out)?,
        b"GSUB" | b"GPOS" => features(data, tag, &mut out)?,
        b"glyf" => points(font, &options.glyphs, &Names::new(font), &mut out)?,
        b"VORG" => vorg(data, offset, &Names::new(font), &mut out)?,
        _ => kern_pairs(data, &Names::new(font), &mut out)?,
    }
    Ok(Some(out.text))
}

// ── Reading ──────────────────────────────────────────────────────────────────────────────────────

fn cut(what: &str) -> TablesError {
    TablesError::invalid(format!("the '{what}' table is cut short"))
}

fn u8_at(d: &[u8], at: usize, what: &str) -> Result<u8> {
    d.get(at).copied().ok_or_else(|| cut(what))
}

fn u16_at(d: &[u8], at: usize, what: &str) -> Result<u16> {
    let b = d.get(at..at.saturating_add(2)).ok_or_else(|| cut(what))?;
    Ok(u16::from_be_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)]))
}

fn u32_at(d: &[u8], at: usize, what: &str) -> Result<u32> {
    let hi = u16_at(d, at, what)?;
    let lo = u16_at(d, at.saturating_add(2), what)?;
    Ok((u32::from(hi) << 16) | u32::from(lo))
}

fn tag_at(d: &[u8], at: usize, what: &str) -> Result<String> {
    let b = d.get(at..at.saturating_add(4)).ok_or_else(|| cut(what))?;
    Ok(b.iter().map(|c| char::from(*c)).collect())
}

/// Glyph names as spot prints them.
struct Names(Vec<String>);

impl Names {
    fn new(font: &SfntFont) -> Self {
        let mut names = crate::ttx::glyph_order(font).unwrap_or_default();
        if font.get(*b"CFF ").is_some_and(super::cff_literal::is_cid) {
            for n in &mut names {
                if let Some(cid) = n.strip_prefix("cid").and_then(|c| c.parse::<u32>().ok()) {
                    *n = format!("\\{cid}");
                }
            }
        }
        Self(names)
    }

    /// spot's name of a glyph id: `undefined` for 0xFFFF, the name of glyph 0 for an id past
    /// the glyphs, the index when the font names no glyphs.
    fn get(&self, gid: u32) -> String {
        if gid == 0xFFFF {
            return "undefined".into();
        }
        let gid = if (gid as usize) < self.0.len() { gid } else { 0 };
        self.0.get(gid as usize).cloned().unwrap_or_else(|| gid.to_string())
    }
}

fn header(tag: &str, offset: u32) -> String {
    format!("### [{tag}] ({offset:08x})")
}

// ── cmap ─────────────────────────────────────────────────────────────────────────────────────────

/// The encoding records: platform, script and the subtable's offset in the table.
fn encodings(d: &[u8]) -> Result<Vec<(u16, u16, usize)>> {
    let n = u16_at(d, 2, "cmap")?;
    (0..usize::from(n))
        .map(|i| {
            let at = 4 + 8 * i;
            let offset = u32_at(d, at + 4, "cmap")? as usize;
            if offset >= d.len() {
                return Err(cut("cmap"));
            }
            Ok((u16_at(d, at, "cmap")?, u16_at(d, at + 2, "cmap")?, offset))
        })
        .collect()
}

/// The language of a subtable as spot reads it (format 14 has none).
fn cmap_language(d: &[u8], at: usize) -> Result<(u16, u16)> {
    let format = u16_at(d, at, "cmap")?;
    let language = match format {
        14 => 0,
        8 | 10 | 12 | 13 => u32_at(d, at + 8, "cmap")? as u16,
        _ => u16_at(d, at + 4, "cmap")?,
    };
    Ok((format, language))
}

/// `cmap=11`: one line per encoding record.
fn cmap_summary(d: &[u8], out: &mut Out) -> Result<()> {
    out.line("--- encoding[index]={platform,script,language}")?;
    for (i, (platform, script, at)) in encodings(d)?.into_iter().enumerate() {
        let (_, language) = cmap_language(d, at)?;
        out.line(&format!(
            "[{i:2}]={{{},{},{}}}",
            desc::platform(platform),
            desc::script(platform, script),
            desc::language(true, platform, language)
        ))?;
    }
    Ok(())
}

/// Most code points the mapping listings visit over all subtables, printed or not.
const MAX_CMAP_STEPS: usize = 1 << 24;

fn spend(budget: &mut usize) -> Result<()> {
    *budget = budget.checked_sub(1).ok_or(TablesError::TooLarge { what: "the code points of the 'cmap' listing", limit: MAX_CMAP_STEPS })?;
    Ok(())
}

/// One mapping line of levels 5 to 8.
fn mapping(out: &mut Out, names: &Names, level: u8, code: u32, gid: u32, precision: usize) -> Result<()> {
    match level {
        5 => out.line(&format!("[{code:0precision$X}]={} ", gid as u16)),
        6 => out.line(&format!("[{code}]={} ", gid as u16)),
        7 => out.line(&format!("[{code:0precision$X}]=<{}> ", names.get(gid))),
        _ => out.line(&format!("[{code}]= <{}> ", names.get(gid))),
    }
}

/// `cmap=5` to `=8`: every mapping of every subtable, or of the one selected (spot `-C`).
fn cmap_mappings(d: &[u8], names: &Names, level: u8, selected: Option<usize>, out: &mut Out) -> Result<()> {
    let parsed = crate::ttx::core::cmap::parse(d).ok().map(|(_, s)| s);
    let mut budget = MAX_CMAP_STEPS;
    for (i, (platform, script, at)) in encodings(d)?.into_iter().enumerate() {
        if selected.is_some_and(|s| s != i) {
            continue;
        }
        out.line(&format!("--- encoding[{i}]"))?;
        out.line(&format!("platformId={platform}"))?;
        out.line(&format!("scriptId  ={script}"))?;
        let (format, language) = cmap_language(d, at)?;
        let title = if level >= 7 { "<name/CID>" } else { "glyphId" };
        if format == 14 {
            out.line(&format!("--- [UVS]= {title}"))?;
            uvs(d, at, names, level, out)?;
            out.line("")?;
            continue;
        }
        out.line(&format!("languageId={language}"))?;
        out.line(&format!("--- [code]={title}"))?;
        match format {
            0 => {
                for code in 0..256u32 {
                    let gid = u8_at(d, at + 6 + code as usize, "cmap")?;
                    mapping(out, names, level, code, u32::from(gid), 2)?;
                }
            }
            4 => {
                let segments = usize::from(u16_at(d, at + 6, "cmap")? / 2);
                let ends = at + 14;
                let starts = ends + 2 * segments + 2;
                let deltas = starts + 2 * segments;
                let ranges = deltas + 2 * segments;
                for s in 0..segments {
                    let end = u32::from(u16_at(d, ends + 2 * s, "cmap")?);
                    let start = u32::from(u16_at(d, starts + 2 * s, "cmap")?);
                    let delta = u32::from(u16_at(d, deltas + 2 * s, "cmap")?);
                    let mut range = usize::from(u16_at(d, ranges + 2 * s, "cmap")?);
                    if range == 0xFFFF {
                        range = 0;
                    }
                    for code in start..=end {
                        spend(&mut budget)?;
                        let gid = if range == 0 {
                            code.wrapping_add(delta) & 0xFFFF
                        } else {
                            let at = (ranges + 2 * s).saturating_add(range).saturating_add(2 * (code.saturating_sub(start)) as usize);
                            u32::from(u16_at(d, at, "cmap")?)
                        };
                        // As spot does: the stored glyph is tested before the delta is added.
                        if code != 0xFFFF && gid != 0 {
                            let gid = if range == 0 { gid } else { gid.wrapping_add(delta) & 0xFFFF };
                            mapping(out, names, level, code, gid, 4)?;
                        }
                    }
                }
            }
            6 => {
                let first = u32::from(u16_at(d, at + 6, "cmap")?);
                let count = usize::from(u16_at(d, at + 8, "cmap")?);
                let precision = if count < 256 { 2 } else { 4 };
                for k in 0..count {
                    let gid = u32::from(u16_at(d, at + 10 + 2 * k, "cmap")?);
                    mapping(out, names, level, first + k as u32, gid, precision)?;
                }
            }
            12 => {
                let groups = u32_at(d, at + 12, "cmap")? as usize;
                for k in 0..groups {
                    let g = at.saturating_add(16).saturating_add(k.saturating_mul(12));
                    let (start, end, mut gid) = (u32_at(d, g, "cmap")?, u32_at(d, g + 4, "cmap")?, u32_at(d, g + 8, "cmap")? & 0xFFFF);
                    for code in start..=end {
                        spend(&mut budget)?;
                        if code != 0xFFFF && gid != 0 {
                            mapping(out, names, level, code, gid, 8)?;
                        }
                        gid = (gid + 1) & 0xFFFF;
                    }
                }
            }
            _ => {
                // Formats 2 and 13 as fontTools reads them: glyph 0 left out, codes in order.
                if let Some(sub) = parsed.as_ref().and_then(|p| p.get(i)) {
                    for (code, gid) in sub.map() {
                        let precision = match format {
                            2 if *code < 256 => 2,
                            2 => 4,
                            _ => 8,
                        };
                        mapping(out, names, level, *code, *gid, precision)?;
                    }
                }
            }
        }
        out.line("")?;
    }
    Ok(())
}

/// The variation sequences of a format 14 subtable, default ones first in each record.
fn uvs(d: &[u8], at: usize, names: &Names, level: u8, out: &mut Out) -> Result<()> {
    let u24 = |p: usize| -> Result<u32> { Ok((u32::from(u8_at(d, p, "cmap")?) << 16) | u32::from(u16_at(d, p + 1, "cmap")?)) };
    let line = |out: &mut Out, uv: u32, vs: u32, gid: Option<u32>| -> Result<()> {
        let name = gid.map_or_else(|| "-".to_string(), |g| names.get(g));
        let id = gid.map_or_else(|| "-".to_string(), |g| (g as u16).to_string());
        match level {
            5 => out.line(&format!("[{uv:04X} {vs:04X}]= {id}")),
            6 => out.line(&format!("[{uv} {vs}]= {id}")),
            7 => out.line(&format!("[{uv:04X} {vs:04X}]= <{name}>")),
            _ => out.line(&format!("[{uv} {vs}]= <{name}>")),
        }
    };
    let records = u32_at(d, at + 6, "cmap")? as usize;
    for r in 0..records {
        let rec = at.saturating_add(10).saturating_add(r.saturating_mul(11));
        let vs = u24(rec)?;
        let default = u32_at(d, rec + 3, "cmap")? as usize;
        let other = u32_at(d, rec + 7, "cmap")? as usize;
        if default != 0 {
            let p = at.saturating_add(default);
            for k in 0..u32_at(d, p, "cmap")? as usize {
                let e = p.saturating_add(4).saturating_add(k.saturating_mul(4));
                let first = u24(e)?;
                for extra in 0..=u32::from(u8_at(d, e + 3, "cmap")?) {
                    line(out, first.saturating_add(extra), vs, None)?;
                }
            }
        }
        if other != 0 {
            let p = at.saturating_add(other);
            for k in 0..u32_at(d, p, "cmap")? as usize {
                let e = p.saturating_add(4).saturating_add(k.saturating_mul(5));
                line(out, u24(e)?, vs, Some(u32::from(u16_at(d, e + 3, "cmap")?)))?;
            }
        }
    }
    Ok(())
}

// ── hmtx ─────────────────────────────────────────────────────────────────────────────────────────

/// `hmtx=5` to `=8`: the advance (and side bearing) of every glyph on one line.
fn hmtx(font: &SfntFont, d: &[u8], level: u8, names: &Names, out: &mut Out) -> Result<()> {
    let glyphs = crate::ttx::num_glyphs(font)?;
    let long = usize::from(u16_at(font.get(*b"hhea").ok_or_else(|| TablesError::MissingTable("hhea".into()))?, 34, "hhea")?);
    let upem = u16_at(font.get(*b"head").ok_or_else(|| TablesError::MissingTable("head".into()))?, 18, "head")?;
    if upem == 0 || long == 0 {
        return Err(TablesError::invalid("the font has no units per em or no long horizontal metrics"));
    }
    let scaled = level == 6 || level == 8;
    let units = if scaled { 1000 } else { upem };
    // spot's `RND`: add a half, then truncate toward zero.
    let scale = |v: f64| -> i64 { (v * 1000.0 / f64::from(upem) + 0.5) as i64 };
    if level <= 6 {
        out.line(&format!("--- [name]=width ({units} units/em)"))?;
    } else {
        out.line(&format!("--- [name]=advance width,left side bearing ({units} units/em)"))?;
    }
    let mut advance = 0u16;
    for gid in 0..glyphs {
        let lsb = if gid < long {
            advance = u16_at(d, 4 * gid, "hmtx")?;
            u16_at(d, 4 * gid + 2, "hmtx")? as i16
        } else {
            match u16_at(d, 4 * long + 2 * (gid - long), "hmtx") {
                Ok(v) => v as i16,
                Err(_) => break,
            }
        };
        let name = names.get(gid as u32);
        let text = match level {
            5 => format!("[{name}]={} ", advance as i16),
            6 => format!("[{name}]={} ", scale(f64::from(advance)) as i16),
            7 => format!("[{name}]={},{lsb} ", advance),
            _ => format!("[{name}]={},{} ", scale(f64::from(advance)) as u16, scale(f64::from(lsb)) as i16),
        };
        out.push(&text)?;
    }
    out.line("")
}

// ── name ─────────────────────────────────────────────────────────────────────────────────────────

/// A name string as spot prints it: printable ASCII as is, anything else as `\` and hex.
fn name_string(d: &[u8], strings: usize, platform: u16, offset: u16, length: u16) -> String {
    let two = platform == 0 || platform == 3;
    let start = strings.saturating_add(usize::from(offset));
    let bytes = d.get(start..).unwrap_or_default();
    let bytes = bytes.get(..usize::from(length)).unwrap_or(bytes);
    let mut s = String::new();
    let mut it = bytes.iter();
    while let Some(b) = it.next() {
        let mut code = u32::from(*b);
        if two {
            code = (code << 8) | u32::from(it.next().copied().unwrap_or(0));
        }
        if (0x20..=0x7e).contains(&code) {
            s.push(char::from(code as u8));
        } else if two {
            s.push_str(&format!("\\{code:04x}"));
        } else {
            s.push_str(&format!("\\{code:02x}"));
        }
    }
    s
}

/// `name=2`, `=3` and `=4`.
fn name(d: &[u8], level: u8, offset: u32, out: &mut Out) -> Result<()> {
    let format = u16_at(d, 0, "name")?;
    let count = usize::from(u16_at(d, 2, "name")?);
    let strings = usize::from(u16_at(d, 4, "name")?);
    let mut records = Vec::with_capacity(count);
    for i in 0..count {
        let at = 6 + 12 * i;
        let field = |k: usize| u16_at(d, at + 2 * k, "name");
        records.push([field(0)?, field(1)?, field(2)?, field(3)?, field(4)?, field(5)?]);
    }
    let mut tags = Vec::new();
    if format == 1 {
        let at = 6 + 12 * count;
        let n = usize::from(u16_at(d, at, "name")?);
        for k in 0..n {
            tags.push((u16_at(d, at + 2 + 4 * k, "name")?, u16_at(d, at + 4 + 4 * k, "name")?));
        }
    }
    let record = |r: &[u16; 6], i: usize| -> String {
        let [p, s, l, n, len, off] = *r;
        format!("[{i:2}]={{{p:2x},{s:2x},{l:4x},{n:4},{len:4},{off:04x}}} ")
    };
    let tag_string = |len: u16, off: u16| name_string(d, strings, 0, off, len);
    if level == 3 {
        out.line("### [name] ")?;
        out.line("--- record[index]={platformId,scriptId,languageId,nameId,length,offset} = <name value>")?;
        for (i, r) in records.iter().enumerate() {
            let [platform, _, language, _, length, off] = *r;
            out.push(&record(r, i))?;
            out.line(&format!("[{off:04x}]=<{}>", name_string(d, strings, platform, off, length)))?;
            if language & 0x8000 != 0 {
                let index = usize::from(language - 0x8000);
                if let Some((len, toff)) = tags.get(index) {
                    out.line(&format!("\tLanguage Tag Index[{index}]={{{toff:04x}}} [{toff:04x}]=<{}>", tag_string(*len, *toff)))?;
                }
            }
        }
        return Ok(());
    }
    out.line(&header("name", offset))?;
    out.line(&format!("format      ={format}"))?;
    out.line(&format!("count       ={count}"))?;
    if format == 1 {
        out.line(&format!("langTagCount       ={}", tags.len()))?;
    }
    out.line(&format!("stringOffset={strings:04x}"))?;
    out.line("--- record[index]={platformId,scriptId,languageId,nameId,length,offset}")?;
    for (i, r) in records.iter().enumerate() {
        let [platform, script, language, name_id, _, _] = *r;
        out.push(&record(r, i))?;
        if level == 4 {
            out.push(&format!(
                "[{},{},{},{}]",
                desc::platform(platform),
                desc::script(platform, script),
                desc::language(false, platform, language),
                desc::name(name_id)
            ))?;
        }
        out.line("")?;
    }
    out.line("--- LangTag[index]={length,offset}")?;
    for (i, (len, off)) in tags.iter().enumerate() {
        out.line(&format!("[{i:2}]={{{len:4}, {off:04x}}} "))?;
    }
    out.line("--- string[name string offset]=<string>")?;
    if level == 4 {
        let mut last = None;
        for r in &records {
            let [platform, _, _, _, length, off] = *r;
            if last != Some((off, length)) {
                out.line(&format!("[{off:04x}]=<{}>", name_string(d, strings, platform, off, length)))?;
                last = Some((off, length));
            }
        }
    }
    out.line("--- string[lang tag string offset]=<string>")?;
    if level == 4 {
        for (len, off) in &tags {
            out.line(&format!("[{off:04x}]=<{}>", tag_string(*len, *off)))?;
        }
    }
    Ok(())
}

// ── GSUB and GPOS ────────────────────────────────────────────────────────────────────────────────

/// `GSUB=5`, `GPOS=5`: each script and language system with its features and their lookups.
/// The required feature is not listed, as spot leaves it out.
fn features(d: &[u8], tag: Tag, out: &mut Out) -> Result<()> {
    let what = crate::sfnt::tag_str(tag);
    let what = what.as_str();
    let scripts = usize::from(u16_at(d, 4, what)?);
    let list = usize::from(u16_at(d, 6, what)?);
    let feature_count = usize::from(u16_at(d, list, what)?);
    out.line(&format!("{} Features:", what.trim_end()))?;
    let lang_sys = |out: &mut Out, at: usize| -> Result<()> {
        let n = usize::from(u16_at(d, at + 4, what)?);
        for m in 0..n {
            let index = usize::from(u16_at(d, at + 6 + 2 * m, what)?);
            if index >= feature_count {
                break;
            }
            let rec = list + 2 + 6 * index;
            let feature = list + usize::from(u16_at(d, rec + 4, what)?);
            let mut line = format!("\t\t[{m:2}]='{}' LookupListIndex: ", tag_at(d, rec, what)?);
            for k in 0..usize::from(u16_at(d, feature + 2, what)?) {
                line.push_str(&format!("{} ", u16_at(d, feature + 4 + 2 * k, what)?));
            }
            out.line(&line)?;
        }
        Ok(())
    };
    for j in 0..usize::from(u16_at(d, scripts, what)?) {
        let rec = scripts + 2 + 6 * j;
        out.line(&format!("Script '{}'", tag_at(d, rec, what)?))?;
        let script = scripts + usize::from(u16_at(d, rec + 4, what)?);
        let default = usize::from(u16_at(d, script, what)?);
        if default != 0 {
            out.line("\tDefault language system")?;
            lang_sys(out, script + default)?;
        }
        for k in 0..usize::from(u16_at(d, script + 2, what)?) {
            let lrec = script + 4 + 6 * k;
            out.line(&format!("\tLanguage '{}'", tag_at(d, lrec, what)?))?;
            lang_sys(out, script + usize::from(u16_at(d, lrec + 4, what)?))?;
        }
    }
    Ok(())
}

// ── glyf ─────────────────────────────────────────────────────────────────────────────────────────

/// `glyf=5`: the points of each simple glyph, contour by contour. Composite and empty glyphs
/// print nothing (spot warns about composites).
fn points(font: &SfntFont, selection: &[String], names: &Names, out: &mut Out) -> Result<()> {
    let p = Provider(font);
    let loca = p.loca(None).map_err(|e| TablesError::invalid(format!("the 'loca' table cannot be read: {e}")))?;
    let glyf = p.glyf().map_err(|e| TablesError::invalid(format!("the 'glyf' table cannot be read: {e}")))?;
    let count = crate::ttx::num_glyphs(font)?;
    let ids: Vec<usize> = if selection.is_empty() {
        (0..count).collect()
    } else {
        selection
            .iter()
            .map(|s| names.0.iter().position(|n| n == s).ok_or_else(|| TablesError::BadOptions(format!("the font has no glyph '{s}'"))))
            .collect::<Result<_>>()?
    };
    for gid in ids {
        let glyph = loca.get_glyf(GlyphId::new(gid as u32), &glyf).map_err(|e| TablesError::invalid(format!("glyph {gid}: {e}")))?;
        let Some(Glyph::Simple(s)) = glyph else { continue };
        if s.number_of_contours() <= 0 {
            continue;
        }
        out.line(&format!("--- glyph[{gid}]"))?;
        let pts: Vec<_> = s.points().collect();
        let mut j = 0usize;
        for (i, end) in s.end_pts_of_contours().iter().enumerate() {
            out.line(&format!("--- contour[{i}]"))?;
            out.line("--- point[index]={off/on,x,y}")?;
            let start = j;
            let mut line = String::new();
            while j <= usize::from(end.get()) {
                let Some(pt) = pts.get(j) else { break };
                line.push_str(&format!("[{}]={{{},{},{}}} ", j - start, if pt.on_curve { "on" } else { "off" }, pt.x, pt.y));
                j += 1;
            }
            out.line(&line)?;
        }
    }
    Ok(())
}

// ── VORG ─────────────────────────────────────────────────────────────────────────────────────────

/// `VORG=4`: the header and each vertical origin by glyph name and index.
fn vorg(d: &[u8], offset: u32, names: &Names, out: &mut Out) -> Result<()> {
    out.line(&header("VORG", offset))?;
    out.line(&format!("majorVersion            = {}", u16_at(d, 0, "VORG")?))?;
    out.line(&format!("minorVersion            = {}", u16_at(d, 2, "VORG")?))?;
    out.line(&format!("defaultVertOriginY     = {}", u16_at(d, 4, "VORG")? as i16))?;
    let n = usize::from(u16_at(d, 6, "VORG")?);
    out.line(&format!("numVertOriginYMetrics  = {n}"))?;
    out.line("--- glyphname @glyphID = vertOriginY")?;
    for i in 0..n {
        let gid = u16_at(d, 8 + 4 * i, "VORG")?;
        let y = u16_at(d, 10 + 4 * i, "VORG")? as i16;
        out.line(&format!("{} @{gid} = {y} ", names.get(u32::from(gid))))?;
    }
    out.line("")
}

// ── kern ─────────────────────────────────────────────────────────────────────────────────────────

/// `kern=5`: the pairs of each format 0 subtable, Microsoft or Apple. Subtables of other
/// formats print nothing.
fn kern_pairs(d: &[u8], names: &Names, out: &mut Out) -> Result<()> {
    let microsoft = u16_at(d, 0, "kern")? == 0;
    let (count, mut at) = if microsoft { (u32::from(u16_at(d, 2, "kern")?), 4usize) } else { (u32_at(d, 4, "kern")?, 8usize) };
    for _ in 0..count {
        if at >= d.len() {
            return Err(cut("kern"));
        }
        let (length, format, body) = if microsoft {
            (usize::from(u16_at(d, at + 2, "kern")?), u16_at(d, at + 4, "kern")? >> 8, at + 6)
        } else {
            (u32_at(d, at, "kern")? as usize, u16_at(d, at + 4, "kern")? & 0xFF, at + 8)
        };
        if format == 0 {
            out.line("--- format 0 [left right value]")?;
            let pairs = usize::from(u16_at(d, body, "kern")?);
            for k in 0..pairs {
                let p = body + 8 + 6 * k;
                let left = u32::from(u16_at(d, p, "kern")?);
                let right = u32::from(u16_at(d, p + 2, "kern")?);
                let value = u16_at(d, p + 4, "kern")? as i16;
                out.line(&format!("{} {} {value}", names.get(left), names.get(right)))?;
            }
        }
        if length == 0 {
            break;
        }
        at = at.saturating_add(length);
    }
    Ok(())
}
