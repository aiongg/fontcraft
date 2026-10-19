//! TTX writer: COLR, CPAL, SVG, sbix, and the bitmap tables EBLC, EBDT, CBLC, CBDT, bloc,
//! bdat and EBSC (M12.7).
//!
//! Each table is written element for element as fontTools' `ttx` writes it, and read with the
//! same checks: a table fontTools would refuse to read is an error here, and [`super::dump`]
//! then writes it as hex. Ported from fontTools' `ttLib.tables` (MIT): `C_O_L_R_` (version 0),
//! `C_P_A_L_`, `S_V_G_`, `_s_b_i_x`, `sbixStrike` and `sbixGlyph`; the bitmap tables in
//! [`bitmap`]. A `COLR` table of version 1 or later goes through the structured-table writer
//! ([`super::otdata`]), whose layouts include the COLR paint graph, as in fontTools.
//!
//! Known differences:
//! - An SVG document compressed with gzip is written as hex, as the crate has no gzip reader.
//! - A bitmap glyph whose data cannot be read makes its whole data table hex with
//!   `ERROR="decompilation error"`; `ttx` reads bitmap glyphs only while writing them and stops
//!   with an error.
//! - Bitmap images are written as hex only (`rawimagedata`); the other forms `ttx` can be asked
//!   for (rows, bits, external files) are not written.

mod bitmap;
#[cfg(test)]
mod fonts;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

use super::core::glyph_name;
use super::var::{bytes_at, i16_at, latin1, slice, u16_at, u32_at};
use super::{XmlWriter, num2binary};

/// Writes table `tag` (bytes `data`, from `font`) into `w`. Returns false, writing nothing,
/// for a tag this module does not handle.
pub fn write_table(tag: Tag, data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<bool> {
    match &tag {
        b"COLR" => {
            if u16_at(data, 0)? == 0 {
                colr_v0(data, font, w)?;
            } else {
                super::otdata::write_layout_table(tag, data, font, w)?;
            }
        }
        b"EBLC" | b"CBLC" | b"bloc" => bitmap::write_location(data, font, w)?,
        b"EBSC" => bitmap::write_scale(data, w)?,
        b"EBDT" | b"CBDT" | b"bdat" => {
            let locator = bitmap::locator(tag).and_then(|t| font.get(t).map(|d| (t, d)));
            let Some((_, locator_data)) = locator else {
                return Err(TablesError::invalid("a bitmap data table without its location table"));
            };
            bitmap::write_data(tag, data, locator_data, font, w)?;
        }
        b"CPAL" => cpal(data, w)?,
        b"SVG " => svg(data, w)?,
        b"sbix" => sbix(data, font, w)?,
        _ => return Ok(false),
    }
    Ok(true)
}

fn val(w: &mut XmlWriter, name: &str, value: impl ToString) {
    w.simple(name, &[("value", &value.to_string())]);
}

// ── COLR ───────────────────────────────────────────────────────────────────────────────────────

fn colr_v0(data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    let base_count = usize::from(u16_at(data, 2)?);
    let base_offset = u32_at(data, 4)? as usize;
    let layer_offset = u32_at(data, 8)? as usize;
    let layer_count = usize::from(u16_at(data, 12)?);
    let mut layers = Vec::with_capacity(layer_count.min(data.len()));
    if layer_offset != 0 {
        for i in 0..layer_count {
            let pos = layer_offset.saturating_add(4 * i);
            layers.push((u16_at(data, pos)?, u16_at(data, pos.saturating_add(2))?));
        }
    }
    let mut glyphs: BTreeMap<u16, Vec<(u16, u16)>> = BTreeMap::new();
    if layer_offset != 0 {
        if base_offset == 0 {
            return Err(TablesError::invalid("'COLR' has layers and no base glyph records"));
        }
        for i in 0..base_count {
            let pos = base_offset.saturating_add(6 * i);
            let base = u16_at(data, pos)?;
            let first = usize::from(u16_at(data, pos.saturating_add(2))?);
            let count = usize::from(u16_at(data, pos.saturating_add(4))?);
            let run = layers
                .get(first..first.saturating_add(count))
                .ok_or_else(|| TablesError::invalid("a 'COLR' base glyph's layers run past the layer records"))?;
            glyphs.insert(base, run.to_vec());
        }
    } else if base_offset != 0 {
        for i in 0..base_count {
            bytes_at(data, base_offset.saturating_add(6 * i), 6)?;
        }
    }
    let order = w.glyph_order(font)?;
    val(w, "version", 0);
    for (base, layers) in &glyphs {
        w.open("ColorGlyph", &[("name", &glyph_name(&order, u32::from(*base)))]);
        for (glyph, color) in layers {
            w.simple("layer", &[("colorID", &color.to_string()), ("name", &glyph_name(&order, u32::from(*glyph)))]);
        }
        w.close("ColorGlyph");
        w.check()?;
    }
    Ok(())
}

// ── CPAL ───────────────────────────────────────────────────────────────────────────────────────

const NO_NAME_ID: u16 = 0xFFFF;

fn cpal(data: &[u8], w: &mut XmlWriter) -> Result<()> {
    let version = u16_at(data, 0)?;
    let entries = usize::from(u16_at(data, 2)?);
    let palettes = usize::from(u16_at(data, 4)?);
    let records = usize::from(u16_at(data, 6)?);
    let first_record = u32_at(data, 8)? as usize;
    if version > 1 {
        return Err(TablesError::invalid(format!("unknown 'CPAL' version {version}")));
    }
    let mut starts = Vec::with_capacity(palettes);
    for i in 0..palettes {
        let start = usize::from(u16_at(data, 12 + 2 * i)?);
        if start + entries > records {
            return Err(TablesError::invalid(format!("'CPAL' palette {i} runs past the color records")));
        }
        starts.push(start);
    }
    let (types, labels, entry_labels) = if version == 0 {
        (0, 0, 0)
    } else {
        let pos = 12 + 2 * palettes;
        (u32_at(data, pos)? as usize, u32_at(data, pos + 4)? as usize, u32_at(data, pos + 8)? as usize)
    };
    let types = u32_array(data, types, palettes, 0)?;
    let labels = u16_array(data, labels, palettes, NO_NAME_ID)?;
    let entry_labels = u16_array(data, entry_labels, entries, NO_NAME_ID)?;

    val(w, "version", version);
    val(w, "numPaletteEntries", entries);
    for (index, start) in starts.iter().enumerate() {
        let index_text = index.to_string();
        let label = labels.get(index).copied().unwrap_or(NO_NAME_ID);
        let kind = types.get(index).copied().unwrap_or(0);
        let label_text = label.to_string();
        let kind_text = kind.to_string();
        let mut attributes = vec![("index", index_text.as_str())];
        if version > 0 && label != NO_NAME_ID {
            attributes.push(("label", &label_text));
        }
        if version > 0 && kind != 0 {
            attributes.push(("type", &kind_text));
        }
        w.open("palette", &attributes);
        for j in 0..entries {
            let pos = first_record.saturating_add(4 * (start + j));
            let bgra = bytes_at(data, pos, 4)?;
            let (b, g, r, a) = (byte(bgra, 0), byte(bgra, 1), byte(bgra, 2), byte(bgra, 3));
            w.simple("color", &[("index", &j.to_string()), ("value", &format!("#{r:02X}{g:02X}{b:02X}{a:02X}"))]);
        }
        w.close("palette");
        w.check()?;
    }
    if version > 0 && entry_labels.iter().any(|l| *l != NO_NAME_ID) {
        w.open("paletteEntryLabels", &[]);
        for (index, label) in entry_labels.iter().enumerate() {
            if *label != NO_NAME_ID {
                w.simple("label", &[("index", &index.to_string()), ("value", &label.to_string())]);
            }
        }
        w.close("paletteEntryLabels");
    }
    Ok(())
}

fn byte(bytes: &[u8], i: usize) -> u8 {
    bytes.get(i).copied().unwrap_or(0)
}

fn u16_array(data: &[u8], offset: usize, count: usize, default: u16) -> Result<Vec<u16>> {
    if offset == 0 {
        return Ok(vec![default; count]);
    }
    (0..count).map(|i| u16_at(data, offset.saturating_add(2 * i))).collect()
}

fn u32_array(data: &[u8], offset: usize, count: usize, default: u32) -> Result<Vec<u32>> {
    if offset == 0 {
        return Ok(vec![default; count]);
    }
    (0..count).map(|i| u32_at(data, offset.saturating_add(4 * i))).collect()
}

// ── SVG ────────────────────────────────────────────────────────────────────────────────────────

fn svg(data: &[u8], w: &mut XmlWriter) -> Result<()> {
    bytes_at(data, 0, 10)?;
    let index = u32_at(data, 2)? as usize;
    let count = usize::from(u16_at(data, index)?);
    let records = index.saturating_add(2);
    let mut docs = Vec::with_capacity(count);
    for i in 0..count {
        let pos = records.saturating_add(12 * i);
        let start_glyph = u16_at(data, pos)?;
        let end_glyph = u16_at(data, pos.saturating_add(2))?;
        let offset = u32_at(data, pos.saturating_add(4))? as usize;
        let length = u32_at(data, pos.saturating_add(8))? as usize;
        docs.push((start_glyph, end_glyph, offset, length));
    }
    for (start_glyph, end_glyph, offset, length) in docs {
        let start = offset.saturating_add(index);
        let doc = slice(data, start, start.saturating_add(length));
        if doc.starts_with(&[0x1F, 0x8B]) {
            return Err(TablesError::invalid("gzip-compressed SVG documents cannot be read"));
        }
        let text = std::str::from_utf8(doc).map_err(|_| TablesError::invalid("an SVG document is not UTF-8"))?;
        w.open("svgDoc", &[("endGlyphID", &end_glyph.to_string()), ("startGlyphID", &start_glyph.to_string())]);
        // A CDATA section cannot hold "]]>": it is closed and reopened around the ">".
        let body = text.replace("]]>", "]]]]><![CDATA[>");
        w.text_raw(&format!("<![CDATA[{body}]]>"), text);
        w.close("svgDoc");
        w.check()?;
    }
    Ok(())
}

// ── sbix ───────────────────────────────────────────────────────────────────────────────────────

struct SbixGlyph {
    gid: u32,
    /// Origin offsets, graphic type and either the image or the glyph it refers to (`dupe`).
    #[allow(clippy::type_complexity)]
    image: Option<(i16, i16, String, Result<u16, Vec<u8>>)>,
}

fn sbix(data: &[u8], font: &SfntFont, w: &mut XmlWriter) -> Result<()> {
    bytes_at(data, 0, 8)?;
    let version = u16_at(data, 0)?;
    let flags = u16_at(data, 2)?;
    let count = u32_at(data, 4)? as usize;
    let mut offsets = Vec::new();
    for i in 0..count {
        offsets.push(u32_at(data, 8usize.saturating_add(4 * i))? as usize);
    }
    let mut strikes: BTreeMap<u16, (u16, Vec<SbixGlyph>)> = BTreeMap::new();
    let mut end = data.len();
    for offset in offsets.iter().rev() {
        let strike = slice(data, *offset, end);
        end = end.min(*offset);
        let (ppem, resolution, glyphs) = sbix_strike(strike)?;
        if strikes.insert(ppem, (resolution, glyphs)).is_some() {
            return Err(TablesError::invalid("'sbix' has two strikes for one ppem"));
        }
    }
    let order = w.glyph_order(font)?;
    val(w, "version", version);
    val(w, "flags", num2binary(u64::from(flags), 16));
    for (ppem, (resolution, glyphs)) in &strikes {
        w.open("strike", &[]);
        val(w, "ppem", ppem);
        val(w, "resolution", resolution);
        // Written in glyph order; glyph ids past the font's glyphs have no name there.
        for glyph in glyphs.iter().filter(|g| (g.gid as usize) < order.len()) {
            let name = glyph_name(&order, glyph.gid);
            let Some((x, y, graphic_type, content)) = &glyph.image else {
                w.simple("glyph", &[("name", &name)]);
                continue;
            };
            w.open("glyph", &[("graphicType", graphic_type), ("name", &name), ("originOffsetX", &x.to_string()), ("originOffsetY", &y.to_string())]);
            match content {
                Ok(gid) => w.simple("ref", &[("glyphname", &glyph_name(&order, u32::from(*gid)))]),
                Err(image) => {
                    w.open("hexdata", &[]);
                    w.hex(image);
                    w.close("hexdata");
                }
            }
            w.close("glyph");
            w.check()?;
        }
        w.close("strike");
    }
    Ok(())
}

fn sbix_strike(strike: &[u8]) -> Result<(u16, u16, Vec<SbixGlyph>)> {
    let ppem = u16_at(strike, 0)?;
    let resolution = u16_at(strike, 2)?;
    let first = i64::from(u32_at(strike, 4)?);
    let count = (first - 4).div_euclid(4) - 1;
    let count = usize::try_from(count).unwrap_or(0);
    let mut offsets = Vec::new();
    for i in 0..=count {
        offsets.push(u32_at(strike, 4usize.saturating_add(4 * i))? as usize);
    }
    let mut glyphs = Vec::with_capacity(count);
    for (gid, pair) in offsets.windows(2).enumerate() {
        let (Some(&start), Some(&stop)) = (pair.first(), pair.get(1)) else { continue };
        let raw = slice(strike, start, stop);
        let gid = u32::try_from(gid).unwrap_or(u32::MAX);
        if raw.is_empty() {
            glyphs.push(SbixGlyph { gid, image: None });
            continue;
        }
        if raw.len() < 8 {
            return Err(TablesError::invalid("an 'sbix' glyph header is cut short"));
        }
        let x = i16_at(raw, 0)?;
        let y = i16_at(raw, 2)?;
        let graphic_type = latin1(bytes_at(raw, 4, 4)?);
        let rest = slice(raw, 8, raw.len());
        let content = if graphic_type == "dupe" {
            if rest.len() != 2 {
                return Err(TablesError::invalid("an 'sbix' dupe glyph does not hold one glyph id"));
            }
            Ok(u16_at(rest, 0)?)
        } else {
            Err(rest.to_vec())
        };
        glyphs.push(SbixGlyph { gid, image: Some((x, y, graphic_type, content)) });
    }
    Ok((ppem, resolution, glyphs))
}
