//! Reordering the glyphs of a compiled font (M12.15; rows `glyphs reorder`,
//! `font.reorderGlyphs`). Oracle: fontTools `ttLib.reorderGlyphs`. Every glyph-indexed table is
//! rewritten.
//!
//! The tables that index glyphs and are rewritten: `glyf` and `loca` (composite glyphs follow),
//! `hmtx` and `vmtx` (with the long-metric counts of `hhea` and `vhea`), `cmap`, `post`,
//! `CFF ` (charstrings, charset, FDSelect), `GDEF`, `GSUB`, `GPOS`, `kern`, `gvar`, `HVAR`,
//! `VVAR`, `VORG`, `hdmx` and `LTSH`. A font with another table known to index glyphs (colour,
//! bitmap, `MATH`, `CFF2`, the Apple layout tables…) is refused; tables that do not index glyphs,
//! and unknown tables, pass through byte for byte, as fontTools passes them.

mod be;
pub(crate) mod cff;
mod cmap;
mod layout;
pub(crate) mod names;
pub(crate) mod tables;

use std::collections::HashMap;

use read_fonts::tables::cmap::{Cmap, PlatformId};
use read_fonts::{FontData, FontRead};

use self::be::{Data, set_u16};
use self::tables::Order;
use crate::limits::MAX_GLYPHS;
use crate::sfnt::{SfntFont, Tag, tag_str};
use crate::{Result, TablesError};

/// The glyph order [`reorder_glyphs`] gives the font.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReorderOptions {
    /// Glyph names in their new order; glyphs left out follow in their old order. `.notdef`
    /// stays first.
    pub order: Vec<String>,
}

/// Tables that index glyphs and that [`reorder_glyphs`] does not rewrite.
const REFUSED: [&[u8; 4]; 26] = [
    b"CFF2", b"COLR", b"sbix", b"CBLC", b"CBDT", b"EBLC", b"EBDT", b"bloc", b"bdat", b"MATH", b"JSTF", b"SVG ", b"morx", b"mort", b"kerx", b"prop",
    b"lcar", b"opbd", b"bsln", b"just", b"ankr", b"Zapf", b"gcid", b"VARC", b"TSI0", b"TSI1",
];

/// The font with its glyphs in the new order.
pub fn reorder_glyphs(font: SfntFont, options: &ReorderOptions) -> Result<SfntFont> {
    let names = glyph_names(&font)?;
    let new_to_old = new_order(&names, &options.order)?;
    apply_order(font, &new_to_old)
}

/// The glyph names of `font` by glyph id, as the glyph tools see them: the CFF charset, else
/// the `post` names, else `glyphNNNNN`; a repeated name gets `#1`, `#2`… appended.
pub fn glyph_names(font: &SfntFont) -> Result<Vec<String>> {
    names::glyph_names(font)
}

/// The new order as old glyph ids (`result[new] = old`): glyph 0 first, then the glyphs of
/// `order` in its order, then every glyph it leaves out in its old order. An unknown or
/// repeated name is an error.
pub fn new_order(names: &[String], order: &[String]) -> Result<Vec<u16>> {
    if order.len() > MAX_GLYPHS {
        return Err(TablesError::TooLarge { what: "the glyph order", limit: MAX_GLYPHS });
    }
    let index: HashMap<&str, usize> = names.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
    let mut placed = vec![false; names.len()];
    let mut out: Vec<u16> = Vec::with_capacity(names.len());
    if let Some(first) = placed.first_mut() {
        *first = true;
        out.push(0);
    }
    for name in order {
        let i = *index.get(name.as_str()).ok_or_else(|| TablesError::BadOptions(format!("the font has no glyph `{name}`")))?;
        if i == 0 {
            continue;
        }
        let seen = placed.get_mut(i).ok_or_else(|| TablesError::BadOptions(format!("the font has no glyph `{name}`")))?;
        if *seen {
            return Err(TablesError::BadOptions(format!("`{name}` is named twice in the new order")));
        }
        *seen = true;
        out.push(i as u16);
    }
    for (i, seen) in placed.iter().enumerate() {
        if !seen {
            out.push(i as u16);
        }
    }
    Ok(out)
}

/// How [`sort_order`] sorts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    /// Encoded glyphs by their lowest code point, then the others in their old order.
    Unicode,
    /// By name, comparing bytes.
    Name,
    /// By CID in a CID-keyed font; other fonts keep their order.
    Cid,
    /// By the size of the glyph's outline data, smallest first.
    Size,
}

impl SortKey {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "unicode" => Some(SortKey::Unicode),
            "name" => Some(SortKey::Name),
            "cid" => Some(SortKey::Cid),
            "size" => Some(SortKey::Size),
            _ => None,
        }
    }
}

/// The glyph names of `font` sorted by `key` (a stable sort; `.notdef` stays first).
pub fn sort_order(font: &SfntFont, key: SortKey) -> Result<Vec<String>> {
    let names = glyph_names(font)?;
    let n = names.len();
    let mut ids: Vec<usize> = (1..n).collect();
    match key {
        SortKey::Name => ids.sort_by(|a, b| names.get(*a).cmp(&names.get(*b))),
        SortKey::Cid => {
            // The charset order is the CID order the font was built with.
            if let Some(bytes) = font.get(*b"CFF ") {
                let cff = cff::parse(bytes, n)?;
                if cff.is_cid() {
                    let cids = cff.charset();
                    ids.sort_by_key(|g| cids.get(*g).copied().unwrap_or(u16::MAX));
                }
            }
        }
        SortKey::Unicode => {
            let code_points = glyph_code_points(font)?;
            ids.sort_by_key(|g| code_points.get(*g).and_then(|c| c.first()).copied().unwrap_or(u32::MAX));
        }
        SortKey::Size => {
            let sizes = outline_sizes(font, n)?;
            ids.sort_by_key(|g| sizes.get(*g).copied().unwrap_or(0));
        }
    }
    Ok(std::iter::once(0).chain(ids).filter_map(|g| names.get(g).cloned()).collect())
}

/// The Unicode code points that map to each glyph, from every Unicode subtable of `cmap`,
/// sorted; empty for every glyph of a font without `cmap`.
pub fn glyph_code_points(font: &SfntFont) -> Result<Vec<Vec<u32>>> {
    let n = names::num_glyphs(font)?;
    let mut out = vec![Vec::new(); n];
    let Some(bytes) = font.get(*b"cmap") else { return Ok(out) };
    let cmap = Cmap::read(FontData::new(bytes)).map_err(|e| TablesError::invalid(format!("the 'cmap' table cannot be read: {e}")))?;
    for record in cmap.encoding_records() {
        let unicode =
            record.platform_id() == PlatformId::Unicode || (record.platform_id() == PlatformId::Windows && matches!(record.encoding_id(), 1 | 10));
        if !unicode {
            continue;
        }
        let Ok(sub) = record.subtable(cmap.offset_data()) else { continue };
        for (cp, gid) in sub.iter().take(crate::limits::MAX_CODE_POINTS) {
            if let Some(list) = out.get_mut(gid.to_u32() as usize)
                && gid.to_u32() != 0
            {
                list.push(cp);
            }
        }
    }
    for list in &mut out {
        list.sort_unstable();
        list.dedup();
    }
    Ok(out)
}

fn outline_sizes(font: &SfntFont, n: usize) -> Result<Vec<usize>> {
    if let Some(bytes) = font.get(*b"CFF ") {
        return Ok(cff::parse(bytes, n)?.charstring_lengths());
    }
    let (Some(loca), Some(head)) = (font.get(*b"loca"), font.get(*b"head")) else { return Ok(vec![0; n]) };
    let long = Data::new(*b"head", head).i16(50)? != 0;
    let l = Data::new(*b"loca", loca);
    let at = |i: usize| -> Result<usize> { if long { l.u32_usize(4 * i) } else { Ok(2 * usize::from(l.u16(2 * i)?)) } };
    (0..n).map(|g| Ok(at(g + 1)?.saturating_sub(at(g)?))).collect()
}

fn required(font: &SfntFont, tag: Tag) -> Result<&[u8]> {
    font.get(tag).ok_or_else(|| TablesError::MissingTable(tag_str(tag)))
}

/// `font` with glyph `new_to_old[i]` at index `i` in every glyph-indexed table. Nothing else
/// changes; an order equal to the current one returns the font as it is.
pub(crate) fn apply_order(font: SfntFont, new_to_old: &[u16]) -> Result<SfntFont> {
    let n = names::num_glyphs(&font)?;
    if new_to_old.len() != n {
        return Err(TablesError::BadOptions(format!("the new order has {} glyphs, the font {n}", new_to_old.len())));
    }
    let mut seen = vec![false; n];
    for old in new_to_old {
        match seen.get_mut(usize::from(*old)) {
            Some(s) if !*s => *s = true,
            _ => return Err(TablesError::BadOptions(format!("glyph {old} appears twice or is past the glyph count"))),
        }
    }
    if new_to_old.iter().enumerate().all(|(i, o)| usize::from(*o) == i) {
        return Ok(font);
    }
    if let Some(tag) = font.tags().into_iter().find(|t| REFUSED.contains(&t)) {
        return Err(TablesError::Unsupported(format!("reordering the glyphs of a font with a '{}' table", tag_str(tag))));
    }
    let order = Order::new(new_to_old.to_vec());
    let mut out = font.clone();

    if let (Some(glyf), Some(loca)) = (font.get(*b"glyf"), font.get(*b"loca")) {
        let long = Data::new(*b"head", required(&font, *b"head")?).i16(50)? != 0;
        let (g, l) = tables::glyf_loca(glyf, loca, long, &order)?;
        out.insert(*b"glyf", g);
        out.insert(*b"loca", l);
    }
    for (mtx, hea) in [(*b"hmtx", *b"hhea"), (*b"vmtx", *b"vhea")] {
        if let Some(table) = font.get(mtx) {
            let header = required(&font, hea)?;
            let count = usize::from(Data::new(hea, header).u16(34)?);
            let (bytes, count) = tables::metrics(table, mtx, count, &order)?;
            let mut header = header.to_vec();
            set_u16(&mut header, 34, count)?;
            out.insert(mtx, bytes);
            out.insert(hea, header);
        }
    }
    if let Some(post) = font.get(*b"post")
        && let Some(bytes) = names::reorder_post(post, new_to_old)?
    {
        out.insert(*b"post", bytes);
    }
    if let Some(bytes) = font.get(*b"CFF ") {
        out.insert(*b"CFF ", cff::parse(bytes, n)?.rebuild(new_to_old, None)?);
    }
    type Rewrite = fn(&[u8], &Order) -> Result<Vec<u8>>;
    let rewrites: [(Tag, Rewrite); 10] = [
        (*b"cmap", cmap::cmap),
        (*b"GDEF", layout::gdef),
        (*b"GSUB", layout::gsub),
        (*b"GPOS", layout::gpos),
        (*b"kern", tables::kern),
        (*b"gvar", tables::gvar),
        (*b"VORG", tables::vorg),
        (*b"hdmx", tables::hdmx),
        (*b"LTSH", tables::ltsh),
        (*b"HVAR", |t, o| tables::metrics_variations(t, *b"HVAR", o)),
    ];
    for (tag, rewrite) in rewrites {
        if let Some(bytes) = font.get(tag) {
            out.insert(tag, rewrite(bytes, &order)?);
        }
    }
    if let Some(bytes) = font.get(*b"VVAR") {
        out.insert(*b"VVAR", tables::metrics_variations(bytes, *b"VVAR", &order)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
