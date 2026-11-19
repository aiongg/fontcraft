//! The `compare` layout: a font set, one row per font, at one em per glyph, so glyphs of the same
//! index line up in a column across the fonts. Fonts are sorted by glyph count, then glyph names,
//! then PostScript name, as the reference. The glyphs are cut into groups that fit a line; each
//! group gets a title line and one row per font that has glyphs in it. The first group's rows
//! start with the font's PostScript name and index, the later ones with the index only. A group
//! that fits a page is not split across pages.

use skrifa::MetadataProvider;
use skrifa::string::StringId;

use super::Display;
use crate::font::Face;
use crate::page::{self, Anchor, Document, GlyphStyle, Ink, Item, ShownGlyph};
use crate::{ProofError, ProofOptions};

/// Glyph size when none is given, in points, as the reference.
pub(crate) const DEFAULT_SIZE: f64 = 12.0;

/// One font of the set, with the glyphs it shows.
struct Row {
    name: String,
    /// Indices into [`Document::glyphs`].
    glyphs: Vec<usize>,
    names: Vec<String>,
}

pub(crate) fn sheet(faces: &[Face<'_>], doc: &mut Document, options: &ProofOptions, copies: &[Display], size: f64) -> Result<(), ProofError> {
    let default = Display::default();
    let d = copies.first().unwrap_or(&default);
    let [x0, y0, w, h] = page::content(options.page_size);
    if !size.is_finite() || size < page::MIN_SIZE || size > w.min(h) {
        return Err(ProofError::BadOptions(format!("size {size} must be from {} to {} points on this page", page::MIN_SIZE, w.min(h))));
    }
    let mut budget = crate::font::MAX_TOTAL_ELEMENTS;
    let mut rows: Vec<Row> = Vec::with_capacity(faces.len());
    for (font, face) in faces.iter().enumerate() {
        let gids: Vec<u32> = if options.glyphs.is_empty() {
            (0..face.names.len() as u32).collect()
        } else {
            // Each font shows the asked-for glyphs it has.
            let index = crate::select::name_index(&face.names);
            options.glyphs.iter().filter_map(|n| index.get(n.as_str()).copied()).collect()
        };
        let mut row = Row { name: ps_name(face), glyphs: Vec::with_capacity(gids.len()), names: Vec::with_capacity(gids.len()) };
        for gid in gids {
            let outline = face.outline(gid, &mut budget)?;
            row.glyphs.push(doc.glyphs.len());
            row.names.push(face.name(gid).to_string());
            doc.glyphs.push(ShownGlyph { font, gid, name: face.name(gid).to_string(), unicode: face.unicode(gid), outline });
        }
        rows.push(row);
    }
    rows.sort_by(|a, b| a.glyphs.len().cmp(&b.glyphs.len()).then_with(|| a.names.cmp(&b.names)).then_with(|| a.name.cmp(&b.name)));

    let label_size = d.group_title_size;
    let widest = rows.iter().enumerate().map(|(i, r)| crate::helvetica::width(&format!("{} {i}", r.name), label_size)).fold(0.0, f64::max);
    let name_field = 1 + (widest / size).floor().clamp(0.0, 1e6) as usize;
    let index_field = 1 + (crate::helvetica::width(&rows.len().to_string(), label_size) / size).floor().clamp(0.0, 1e6) as usize;
    let across = ((w / size).floor().clamp(0.0, 1e6) as usize).saturating_sub(index_field);
    if across <= name_field {
        return Err(ProofError::BadOptions(format!("glyphs of {size} points leave no room for a row on this page")));
    }
    let longest = rows.iter().map(|r| r.glyphs.len()).max().unwrap_or(0);
    if longest == 0 {
        return Err(ProofError::BadOptions("none of the fonts has a glyph to show".into()));
    }
    // The glyph ranges of the groups: the first is shorter by the name column.
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    while start < longest {
        let take = if groups.is_empty() { across - name_field } else { across };
        groups.push((start, start + take));
        start += take;
    }
    let line = size * 1.2;
    let title_h = line.max(label_size * 1.2);
    let bottom = y0 + h;
    let mut y = bottom + 1.0;
    let mut items: Vec<Item> = Vec::new();
    let mut started = false;
    let total = groups.len();
    for (g, &(first, end)) in groups.iter().enumerate() {
        let shown: Vec<(usize, &Row)> = rows.iter().enumerate().filter(|(_, r)| r.glyphs.len() > first).collect();
        let whole = title_h + shown.len() as f64 * line;
        let needed = if whole <= h { whole } else { title_h + line };
        if y + needed > bottom {
            if started {
                page::new_page(doc, options.page_size)?.items = std::mem::take(&mut items);
            }
            started = true;
            y = y0;
        }
        let text = format!("Glyph group {} of {total}", g + 1);
        items.push(Item::Label { x: x0, y: y + label_size, size: label_size, text, anchor: Anchor::Start, ink: Ink::Rgb(d.meta_colour) });
        y += title_h;
        for (i, row) in shown {
            if y + line > bottom {
                page::new_page(doc, options.page_size)?.items = std::mem::take(&mut items);
                y = y0;
            }
            let baseline = y + size;
            let (text, field) = if g == 0 { (format!("{} {i}", row.name), name_field) } else { (i.to_string(), index_field) };
            items.push(Item::Label { x: x0, y: baseline, size: label_size, text, anchor: Anchor::Start, ink: Ink::Rgb(d.meta_colour) });
            for (col, &index) in row.glyphs.iter().take(end).skip(first).enumerate() {
                let Some(glyph) = doc.glyphs.get(index) else { continue };
                let Some(face) = faces.get(glyph.font) else { continue };
                let scale = size / face.upem;
                let x = x0 + (field + col) as f64 * size;
                let style = if d.filled { GlyphStyle::Filled } else { GlyphStyle::Outlined };
                items.push(Item::Glyph { glyph: index, x, y: baseline, scale, style, primary: true });
            }
            y += line;
        }
    }
    if started {
        page::new_page(doc, options.page_size)?.items = items;
    }
    Ok(())
}

/// The PostScript name, else family and style joined by a hyphen.
fn ps_name(face: &Face<'_>) -> String {
    face.font
        .localized_strings(StringId::POSTSCRIPT_NAME)
        .english_or_first()
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("{}-{}", face.family, face.style).replace(' ', ""))
}
