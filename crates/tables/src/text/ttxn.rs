//! The normalised dump (`dump --normalize`), as AFDKO `ttxn` writes it
//! (`docs/spec/20-cli.md` › Method: normalised dump).
//!
//! Ported in behaviour from `ttxn.py` of AFDKO 5.0.1 (Apache-2.0, Copyright 2014 Adobe; see `NOTICE`).
//! The document is the TTX of [`crate::ttx`] with these changes:
//!
//! - Tables are sorted by tag; `GlyphOrder` and `DSIG` are left out.
//! - `head`: `created`, `modified`, `magicNumber` and `checkSumAdjustment` are zero, and
//!   `fontRevision` too with [`ViewOptions::no_version`]. `hhea.numberOfHMetrics` and
//!   `vhea.numberOfLongVerMetrics` are zero.
//! - `name` with `no_version`: name ID 3 reads `VERSION SUPPRESSED`; name ID 5 keeps the text
//!   before its first `;` on the Windows platform and reads `VERSION SUPPRESSED` elsewhere.
//! - `GDEF`: the glyph and mark-attachment class definitions carry `Format="0"`; an attachment
//!   list or ligature caret list with an empty coverage is left out.
//! - `cmap`: mappings to `.notdef` are left out; `length` (and `nGroups` of format 12) read 0.
//! - `CFF ` and `glyf`: the `@font` text in `FontTopDict`, then every glyph's long `@glyphs`
//!   path (without hints with [`ViewOptions::no_hints`]) in `FontOutlines`, sorted by text.
//! - `GSUB` and `GPOS`: the layout as feature-file-like text ([`super::ttxn_otl`]).

use super::{Out, ViewOptions};
use crate::Result;
use crate::sfnt::{Flavour, SfntFont, Tag};
use crate::ttx::{DumpOptions, TTX_VERSION, tag_to_xml};

const HEAD: Tag = *b"head";
const HHEA: Tag = *b"hhea";
const VHEA: Tag = *b"vhea";
const NAME: Tag = *b"name";
const GDEF: Tag = *b"GDEF";
const CMAP: Tag = *b"cmap";
const DSIG: Tag = *b"DSIG";

/// The normalised dump of the tables `options.tables` names (every table when empty).
pub fn dump(font: &SfntFont, options: &ViewOptions) -> Result<String> {
    let mut tags: Vec<Tag> = if options.tables.is_empty() {
        font.tables.keys().copied().collect()
    } else {
        options.tables.iter().copied().filter(|t| font.tables.contains_key(t)).collect()
    };
    tags.sort_unstable();
    tags.dedup();
    tags.retain(|t| *t != DSIG);

    let zeroed = zeroed_font(font, options.no_version);
    let mut out = Out::new();
    out.line(r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
    let version = match font.flavour {
        Flavour::TrueType => r"\x00\x01\x00\x00",
        Flavour::Cff => "OTTO",
        Flavour::AppleTrue => "true",
    };
    out.line(&format!(r#"<ttFont sfntVersion="{version}" ttLibVersion="{TTX_VERSION}">"#))?;
    out.line("")?;
    for tag in tags {
        match &tag {
            b"CFF " | b"glyf" => outlines(font, tag, options, &mut out)?,
            b"GSUB" | b"GPOS" => {
                let element = tag_to_xml(tag);
                out.line(&format!("  <{element}>"))?;
                super::ttxn_otl::write(font, tag, options.show_extensions, &mut out)?;
                out.line(&format!("  </{element}>"))?;
                out.line("")?;
            }
            _ => {
                let source = if [HEAD, HHEA, VHEA].contains(&tag) { &zeroed } else { font };
                let lines = ttx_lines(source, tag)?;
                let lines = match tag {
                    NAME if options.no_version => name_lines(lines),
                    GDEF => gdef_lines(lines),
                    CMAP => cmap_lines(lines),
                    _ => lines,
                };
                for l in lines {
                    out.line(&l)?;
                }
            }
        }
    }
    out.line("</ttFont>")?;
    Ok(out.text)
}

/// A copy of the font with the `head`, `hhea` and `vhea` fields that always differ set to zero.
fn zeroed_font(font: &SfntFont, no_version: bool) -> SfntFont {
    let mut copy = SfntFont::new(font.flavour);
    let zero = |data: &[u8], ranges: &[std::ops::Range<usize>]| -> Vec<u8> {
        let mut v = data.to_vec();
        for r in ranges {
            if let Some(bytes) = v.get_mut(r.clone()) {
                bytes.fill(0);
            }
        }
        v
    };
    if let Some(head) = font.get(HEAD) {
        let mut ranges = vec![8..12, 12..16, 20..36];
        if no_version {
            ranges.push(4..8);
        }
        copy.insert(HEAD, zero(head, &ranges));
    }
    for tag in [HHEA, VHEA] {
        if let Some(data) = font.get(tag) {
            copy.insert(tag, zero(data, &[34..36]));
        }
    }
    // The writers of these tables read the glyph count and names from the rest of the font.
    for (tag, data) in &font.tables {
        if !copy.tables.contains_key(tag) {
            copy.insert(*tag, data.clone());
        }
    }
    copy
}

/// The lines of one table's TTX element and the blank line after it.
fn ttx_lines(font: &SfntFont, tag: Tag) -> Result<Vec<String>> {
    let text = crate::ttx::dump(font, &DumpOptions { tables: vec![tag], ..Default::default() })?;
    let mut lines: Vec<String> = text.lines().skip(3).map(str::to_string).collect();
    while lines.last().is_some_and(|l| l.is_empty() || l == "</ttFont>") {
        lines.pop();
    }
    lines.push(String::new());
    Ok(lines)
}

/// The `CFF ` or `glyf` element: the `@font` text, then the sorted outline entries.
fn outlines(font: &SfntFont, tag: Tag, options: &ViewOptions, out: &mut Out) -> Result<()> {
    let element = tag_to_xml(tag);
    out.line(&format!("  <{element}>"))?;
    out.line("    <FontTopDict>")?;
    let dict = super::txdump::font_dict(font)?;
    for l in dict.trim().lines() {
        out.line(&format!("      {l}"))?;
    }
    out.line("    </FontTopDict>")?;
    out.line("    <FontOutlines>")?;
    for entry in super::txdump::outline_entries(font, !options.no_hints)? {
        for l in entry.lines() {
            out.line(&format!("      {l}"))?;
        }
    }
    out.line("    </FontOutlines>")?;
    out.line(&format!("  </{element}>"))?;
    out.line("")
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn attr<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let start = line.find(&key)? + key.len();
    let rest = line.get(start..)?;
    rest.find('"').and_then(|end| rest.get(..end))
}

/// ttxn `-nv` on the `name` element.
pub(super) fn name_lines(lines: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len());
    let mut it = lines.into_iter();
    while let Some(line) = it.next() {
        let t = line.trim_start();
        let id = if t.starts_with("<namerecord ") { attr(t, "nameID") } else { None };
        if !matches!(id, Some("3") | Some("5")) {
            out.push(line);
            continue;
        }
        let windows = attr(t, "platformID") == Some("3");
        let id5 = id == Some("5");
        out.push(line.clone());
        let mut body: Vec<String> = Vec::new();
        let mut close = None;
        for l in it.by_ref() {
            if l.trim_start().starts_with("</namerecord>") {
                close = Some(l);
                break;
            }
            body.push(l);
        }
        let indent: String = body.first().map(|l| l.chars().take_while(|c| *c == ' ').collect()).unwrap_or_default();
        let text = unescape(&body.iter().map(|l| l.trim()).collect::<Vec<_>>().join("\n"));
        let new = if id5 && windows { text.split(';').next().unwrap_or_default().to_string() } else { "VERSION SUPPRESSED".to_string() };
        out.push(format!("{indent}{}", escape(&new)));
        if let Some(c) = close {
            out.push(c);
        }
    }
    out
}

/// ttxn's changes to the `GDEF` element.
pub(super) fn gdef_lines(lines: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0usize;
    while let Some(line) = lines.get(i) {
        let t = line.trim_start();
        let indent = line.get(..line.len() - t.len()).unwrap_or_default();
        if let Some(class) = ["GlyphClassDef", "MarkAttachClassDef"].iter().find(|c| t == format!("<{c}>") || t == format!("<{c}/>")) {
            let slash = if t.ends_with("/>") { "/" } else { "" };
            out.push(format!("{indent}<{class} Format=\"0\"{slash}>"));
            i += 1;
            continue;
        }
        if let Some(list) = ["AttachList", "LigCaretList"].iter().find(|l| t == format!("<{l}>")) {
            let close = format!("</{list}>");
            if let Some(end) = lines.iter().skip(i).position(|l| l.trim() == close).map(|k| k + i)
                && !lines.get(i..=end).unwrap_or_default().iter().any(|l| l.contains("<Glyph value="))
            {
                i = end + 1;
                continue;
            }
        }
        out.push(line.clone());
        i += 1;
    }
    out
}

/// ttxn's changes to the `cmap` element.
pub(super) fn cmap_lines(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("<map code=") && t.contains(r#"name=".notdef""#))
        })
        .map(|l| {
            let t = l.trim_start();
            let format12 = t.starts_with("<cmap_format_12 ");
            if format12 || t.starts_with("<cmap_format_13 ") {
                let mut l = set_attr(&l, "length", "0");
                if format12 {
                    l = set_attr(&l, "nGroups", "0");
                }
                l
            } else {
                l
            }
        })
        .collect()
}

fn set_attr(line: &str, name: &str, value: &str) -> String {
    let key = format!(" {name}=\"");
    let Some(start) = line.find(&key).map(|s| s + key.len()) else { return line.to_string() };
    let Some(len) = line.get(start..).and_then(|r| r.find('"')) else { return line.to_string() };
    format!("{}{value}{}", line.get(..start).unwrap_or_default(), line.get(start + len..).unwrap_or_default())
}
