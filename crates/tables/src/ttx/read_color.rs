//! TTX reader: colour and bitmap tables (M12.9).
//!
//! CPAL, COLR version 0 and SVG, compiled as fontTools compiles them. Ported from fontTools'
//! `ttLib.tables.C_P_A_L_`, `C_O_L_R_`, `S_V_G_` and `colorLib.builder.populateCOLRv0` (MIT).
//!
//! Known differences:
//! - an SVG document marked `compressed` is stored as it is in the text; fontTools gzips it
//!   when that makes it smaller, and its gzip output cannot be reproduced byte for byte here;
//! - COLR version 1 and the bitmap tables (sbix, CBDT, CBLC, EBDT, EBLC) are not read yet:
//!   they fall back to their `hexdata` when the element has one.

use std::collections::HashMap;

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

use super::Element;
use super::read::{Out, ReadContext, int_lit, py_strip};

/// The table a TTX table element stands for, as bytes; `None` for an element this module does
/// not handle. `font` gives the context some tables need (glyph order, `maxp`, `head`).
pub fn read_table(element: &Element, font: &SfntFont) -> Result<Option<(Tag, Vec<u8>)>> {
    read_table_with(element, &ReadContext::from_font(font), font)
}

/// [`read_table`] with the glyph order of the text being read.
pub(crate) fn read_table_with(element: &Element, ctx: &ReadContext, _font: &SfntFont) -> Result<Option<(Tag, Vec<u8>)>> {
    Ok(Some(match element.name.as_str() {
        "CPAL" => (*b"CPAL", cpal(element)?),
        "COLR" => match colr_v0(element, ctx)? {
            Some(data) => (*b"COLR", data),
            None => return Ok(None),
        },
        "SVG" => (*b"SVG ", svg(element)?),
        _ => return Ok(None),
    }))
}

fn bad(what: impl Into<String>) -> TablesError {
    TablesError::invalid(what.into())
}

fn count(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

// ── CPAL ───────────────────────────────────────────────────────────────────────────────────────

const NO_NAME_ID: i64 = 0xFFFF;

/// `Color.fromHex`: `#RRGGBB` or `#RRGGBBAA`, as blue, green, red, alpha.
fn color(text: &str) -> Result<[u8; 4]> {
    let hex = text.strip_prefix('#').unwrap_or(text);
    let byte = |at: usize| hex.get(at..at + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).ok_or_else(|| bad(format!("'{text}' is not a colour")));
    let alpha = if hex.len() >= 8 { byte(6)? } else { 0xFF };
    Ok([byte(4)?, byte(2)?, byte(0)?, alpha])
}

fn cpal(element: &Element) -> Result<Vec<u8>> {
    let (mut version, mut entries) = (0i64, 0i64);
    let mut palettes: Vec<Vec<[u8; 4]>> = Vec::new();
    let (mut types, mut labels): (Vec<i64>, Vec<i64>) = (Vec::new(), Vec::new());
    let mut entry_labels: Vec<i64> = Vec::new();
    for child in &element.children {
        match child.name.as_str() {
            "palette" => {
                if palettes.len() >= 0xFFFF {
                    return Err(TablesError::TooLarge { what: "the palettes of a 'CPAL' table", limit: 0xFFFF });
                }
                labels.push(match child.attribute("label") {
                    Some(v) => int_lit(v)?,
                    None => NO_NAME_ID,
                });
                types.push(match child.attribute("type") {
                    Some(v) => int_lit(v)?,
                    None => 0,
                });
                let colors = child
                    .children
                    .iter()
                    .map(|c| color(c.attribute("value").ok_or_else(|| bad("a colour without a value"))?))
                    .collect::<Result<_>>()?;
                palettes.push(colors);
            }
            "paletteEntryLabels" => {
                let mut by_index: HashMap<i64, i64> = HashMap::new();
                for l in child.children.iter().filter(|l| l.name == "label") {
                    let index = int_lit(l.attribute("index").ok_or_else(|| bad("a label without an index"))?)?;
                    by_index.insert(index, int_lit(l.attribute("value").ok_or_else(|| bad("a label without a value"))?)?);
                }
                entry_labels = (0..entries).map(|i| by_index.get(&i).copied().unwrap_or(NO_NAME_ID)).collect();
            }
            _ => {
                if let Some(v) = child.attribute("value") {
                    let v = int_lit(v)?;
                    match child.name.as_str() {
                        "version" => version = v,
                        "numPaletteEntries" => {
                            if !(0..=0xFFFF).contains(&v) {
                                return Err(bad("the number of palette entries does not fit its field"));
                            }
                            entries = v;
                            entry_labels = vec![NO_NAME_ID; usize::try_from(v).unwrap_or(0)];
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    // Equal palettes share their colour records.
    let mut records: Vec<u8> = Vec::new();
    let mut indices = Out::default();
    let mut pool: HashMap<Vec<u8>, i64> = HashMap::new();
    let mut n_records = 0i64;
    for palette in &palettes {
        if count(palette.len()) != entries {
            return Err(bad("a palette whose length differs from the number of palette entries"));
        }
        let packed: Vec<u8> = palette.iter().flatten().copied().collect();
        let index = match pool.get(&packed) {
            Some(i) => *i,
            None => {
                let i = n_records;
                n_records += 1;
                records.extend_from_slice(&packed);
                pool.insert(packed, i);
                i
            }
        };
        indices.u16(index.saturating_mul(entries), "a colour record index")?;
    }
    let mut types_out = Out::default();
    if version >= 1 && types.iter().any(|t| *t != 0) {
        for t in &types {
            types_out.u32(*t, "a palette type")?;
        }
    }
    let mut labels_out = Out::default();
    if version >= 1 && !labels.iter().all(|l| *l == NO_NAME_ID) {
        for l in &labels {
            labels_out.u16(*l, "a palette label")?;
        }
    }
    let mut entry_out = Out::default();
    if version >= 1 && !entry_labels.iter().all(|l| *l == NO_NAME_ID) {
        if count(entry_labels.len()) != entries {
            return Err(bad("palette entry labels that differ in number from the palette entries"));
        }
        for l in &entry_labels {
            entry_out.u16(*l, "a palette entry label")?;
        }
    }
    let records_count = count(records.len() / 4);
    let mut first = 12 + count(indices.0.len());
    if version >= 1 {
        first += 12;
    }
    let mut o = Out::default();
    o.u16(version, "CPAL.version")?;
    o.u16(entries, "numPaletteEntries")?;
    o.u16(count(palettes.len()), "numPalettes")?;
    o.u16(records_count, "numColorRecords")?;
    o.u32(first, "offsetToFirstColorRecord")?;
    o.bytes(&indices.0);
    if version == 0 {
        o.bytes(&records);
        return Ok(o.0);
    }
    let mut pos = first + count(records.len());
    let mut offset = |len: usize, advance: usize| {
        if len == 0 {
            0
        } else {
            let at = pos;
            pos += count(advance);
            at
        }
    };
    let types_at = offset(types_out.0.len(), types_out.0.len());
    let labels_at = offset(labels_out.0.len(), labels_out.0.len());
    // fontTools advances by the palette labels' length here; nothing follows, so it is unseen.
    let entry_at = offset(entry_out.0.len(), labels_out.0.len());
    o.u32(types_at, "offsetToPaletteTypeArray")?;
    o.u32(labels_at, "offsetToPaletteLabelArray")?;
    o.u32(entry_at, "offsetToPaletteEntryLabelArray")?;
    o.bytes(&records);
    o.bytes(&types_out.0);
    o.bytes(&labels_out.0);
    o.bytes(&entry_out.0);
    Ok(o.0)
}

// ── COLR ───────────────────────────────────────────────────────────────────────────────────────

/// COLR version 0 from its `ColorGlyph` form; `None` for the version 1 form.
fn colr_v0(element: &Element, ctx: &ReadContext) -> Result<Option<Vec<u8>>> {
    let mut version = 0i64;
    let mut glyphs: Vec<(String, Vec<(String, i64)>)> = Vec::new();
    for child in &element.children {
        match child.name.as_str() {
            "version" => version = int_lit(child.attribute("value").ok_or_else(|| bad("the COLR version has no value"))?)?,
            "ColorGlyph" => {
                let name = child.attribute("name").ok_or_else(|| bad("a colour glyph without a name"))?;
                let mut layers = Vec::with_capacity(child.children.len());
                for l in &child.children {
                    let glyph = l.attribute("name").ok_or_else(|| bad("a colour layer without a glyph"))?.to_string();
                    let id = int_lit(l.attribute("colorID").ok_or_else(|| bad("a colour layer without a colour"))?)?;
                    layers.push((glyph, id));
                }
                glyphs.retain(|(g, _)| g != name);
                glyphs.push((name.to_string(), layers));
            }
            _ => return Ok(None),
        }
    }
    let gid = |name: &str| -> Result<i64> { Ok(i64::from(ctx.glyph_id(name)?)) };
    let mut keyed: Vec<(i64, &(String, Vec<(String, i64)>))> = Vec::with_capacity(glyphs.len());
    for g in &glyphs {
        keyed.push((gid(&g.0)?, g));
    }
    keyed.sort_by_key(|(id, _)| *id);
    let mut base = Out::default();
    let mut layers = Out::default();
    let mut n_layers = 0i64;
    for (id, (_, ls)) in &keyed {
        base.u16(*id, "a base glyph")?;
        base.u16(n_layers, "a first layer index")?;
        base.u16(count(ls.len()), "a layer count")?;
        for (glyph, color) in ls {
            layers.u16(gid(glyph)?, "a layer glyph")?;
            layers.u16(*color, "a palette index")?;
            n_layers += 1;
        }
    }
    let base_at = if keyed.is_empty() { 0 } else { 14 };
    let layers_at = if n_layers == 0 { 0 } else { 14 + count(base.0.len()) };
    let mut o = Out::default();
    o.u16(version, "COLR.version")?;
    o.u16(count(keyed.len()), "the number of base glyphs")?;
    o.u32(base_at, "the offset to the base glyphs")?;
    o.u32(layers_at, "the offset to the layers")?;
    o.u16(n_layers, "the number of layers")?;
    o.bytes(&base.0);
    o.bytes(&layers.0);
    Ok(Some(o.0))
}

// ── SVG ────────────────────────────────────────────────────────────────────────────────────────

fn svg(element: &Element) -> Result<Vec<u8>> {
    let mut docs: Vec<(String, i64, i64)> = Vec::new();
    for child in element.children.iter().filter(|c| c.name == "svgDoc") {
        let start = int_lit(child.attribute("startGlyphID").ok_or_else(|| bad("an SVG document without a first glyph"))?)?;
        let end = int_lit(child.attribute("endGlyphID").ok_or_else(|| bad("an SVG document without a last glyph"))?)?;
        docs.push((py_strip(&child.text).to_string(), start, end));
    }
    let mut entries = Out::default();
    entries.u16(count(docs.len()), "the number of SVG documents")?;
    let mut offset = 2 + 12 * count(docs.len());
    let mut seen: HashMap<&[u8], i64> = HashMap::new();
    let mut bodies: Vec<&[u8]> = Vec::new();
    for (doc, start, end) in &docs {
        let bytes = doc.as_bytes();
        let at = match seen.get(bytes) {
            Some(at) => *at,
            None => {
                let at = offset;
                offset += count(bytes.len());
                seen.insert(bytes, at);
                bodies.push(bytes);
                at
            }
        };
        entries.u16(*start, "startGlyphID")?;
        entries.u16(*end, "endGlyphID")?;
        entries.u32(at, "an SVG document offset")?;
        entries.u32(count(bytes.len()), "an SVG document length")?;
    }
    let mut o = Out::default();
    o.u16(0, "SVG.version")?;
    o.u32(10, "offsetToSVGDocIndex")?;
    o.u32(0, "reserved")?;
    o.bytes(&entries.0);
    for b in bodies {
        o.bytes(b);
    }
    Ok(o.0)
}
