//! Page layout: the sheets of [`crate::ProofOptions`] as pages of drawing items, in points with
//! the origin at the top left and y growing downwards. The SVG and PDF writers draw these items;
//! nothing here knows a file format.

use crate::font::{Container, El, Face, Outline};
use crate::{MAX_PAGES, ProofError, ProofOptions, Sheet};

/// Page margin on every side, in points.
pub(crate) const MARGIN: f64 = 36.0;
/// Height of the title line at the top of each page.
pub(crate) const HEADER: f64 = 24.0;
/// Title text size.
pub(crate) const TITLE_SIZE: f64 = 9.0;
/// Most point sizes in one proof.
pub(crate) const MAX_SIZES: usize = 64;
/// Smallest and largest page side, in points (one inch to 200 inches, the PDF limit).
const PAGE_SIDE: (f64, f64) = (72.0, 14_400.0);
/// Smallest glyph or text size, in points.
pub(crate) const MIN_SIZE: f64 = 1.0;
/// Width of the size column of a waterfall.
const WATERFALL_GUTTER: f64 = 40.0;
/// Most characters of waterfall text.
const MAX_TEXT_CHARS: usize = 100_000;
/// Most lines of a waterfall.
const MAX_WATERFALL_LINES: usize = 200_000;
/// An outline page labels the coordinates of its on-curve points only up to this many.
const MAX_POINT_LABELS: usize = 400;

/// How an item is coloured; the writers choose the colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ink {
    /// Glyphs and titles.
    Text,
    /// Labels and secondary text.
    Faint,
    /// Cell borders and metric lines.
    Guide,
    /// Handle lines and off-curve points.
    Handle,
    /// On-curve points.
    Point,
    /// A colour of the drawing settings (`crate::layouts::Display`).
    Rgb([u8; 3]),
}

impl Ink {
    /// The colour every writer uses for this ink.
    pub(crate) fn rgb(self) -> [u8; 3] {
        match self {
            Ink::Text => [0, 0, 0],
            Ink::Faint => [0x6b, 0x6b, 0x6b],
            Ink::Guide => [0xc4, 0xc4, 0xc4],
            Ink::Handle => [0x3b, 0x7d, 0xd8],
            Ink::Point => [0xd0, 0x34, 0x2c],
            Ink::Rgb(c) => c,
        }
    }

    /// The ink of a print in black: labels and marks black, guides grey; a colour of the drawing
    /// settings is kept, since `black` sets those itself.
    pub(crate) fn black(self) -> Ink {
        match self {
            Ink::Text | Ink::Faint | Ink::Handle | Ink::Point => Ink::Text,
            Ink::Guide => Ink::Rgb([0x80, 0x80, 0x80]),
            Ink::Rgb(c) => Ink::Rgb(c),
        }
    }

    pub(crate) fn hex(self) -> String {
        let [r, g, b] = self.rgb();
        format!("#{r:02x}{g:02x}{b:02x}")
    }
}

/// Fill of an outlined glyph.
pub(crate) const OUTLINED_FILL: [u8; 3] = [0xe6, 0xe6, 0xe6];
/// Width of drawn lines, in points.
pub(crate) const LINE_WIDTH: f64 = 0.5;
/// Half the side of a point marker, in points.
pub(crate) const POINT_RADIUS: f64 = 1.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Anchor {
    Start,
    /// The text ends at `x`.
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GlyphStyle {
    /// Filled black.
    Filled,
    /// A light fill with a thin dark contour, for outline pages.
    Outlined,
}

/// One thing drawn on a page.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Item {
    /// Glyph `glyph` of [`Document::glyphs`] with its origin at (`x`, `y`), `scale` points per
    /// font unit. `primary` marks the glyph a cell or page is about (not a small preview).
    Glyph {
        glyph: usize,
        x: f64,
        y: f64,
        scale: f64,
        style: GlyphStyle,
        primary: bool,
    },
    Line {
        from: [f64; 2],
        to: [f64; 2],
        ink: Ink,
    },
    Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        ink: Ink,
    },
    /// A point marker: a filled square on the curve, an open circle off it.
    Point {
        x: f64,
        y: f64,
        on_curve: bool,
        ink: Ink,
    },
    /// A clickable area that opens page `page` (zero-based) of the same document.
    Link {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        page: usize,
    },
    Label {
        x: f64,
        y: f64,
        size: f64,
        text: String,
        anchor: Anchor,
        ink: Ink,
    },
    /// Text set in the embedded font at `size` points, baseline at `y`.
    Sample {
        x: f64,
        y: f64,
        size: f64,
        text: String,
    },
}

/// One page.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Page {
    pub width: f64,
    pub height: f64,
    pub items: Vec<Item>,
}

/// A glyph the pages show.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ShownGlyph {
    /// The font of a font set the glyph comes from (0 unless the layout compares fonts).
    pub font: usize,
    pub gid: u32,
    pub name: String,
    pub unicode: Option<u32>,
    pub outline: Outline,
}

/// The font a page embeds for text set in it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Embedded {
    pub bytes: Vec<u8>,
    pub container: Container,
    /// `font-variation-settings` of the proof's location.
    pub variation: Vec<(String, f64)>,
}

/// A laid-out proof.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Document {
    pub title: String,
    pub pages: Vec<Page>,
    pub glyphs: Vec<ShownGlyph>,
    /// Set when a page sets text in the font.
    pub embedded: Option<Embedded>,
}

/// The cell of a glyph grid at `size`: width, height, label size.
pub(crate) fn grid_cell(size: f64) -> (f64, f64, f64) {
    let label = (size * 0.1).min(9.0);
    (size, size + label * 2.6, label)
}

/// The usable area of a page below the title: `[x, y, w, h]`.
pub(crate) fn content(page: [f64; 2]) -> [f64; 4] {
    [MARGIN, MARGIN + HEADER, (page[0] - 2.0 * MARGIN).max(0.0), (page[1] - 2.0 * MARGIN - HEADER).max(0.0)]
}

/// Columns and rows of a glyph grid at `size` on a page of `page`.
pub(crate) fn grid_shape(page: [f64; 2], size: f64) -> (usize, usize) {
    let [_, _, w, h] = content(page);
    let (cw, ch, _) = grid_cell(size);
    if cw <= 0.0 || ch <= 0.0 || !cw.is_finite() || !ch.is_finite() {
        return (0, 0);
    }
    ((w / cw).floor().clamp(0.0, 1e6) as usize, (h / ch).floor().clamp(0.0, 1e6) as usize)
}

/// The largest glyph size at which `glyphs_per_page` glyphs fit one page of a glyph grid
/// (`--glyphs-per-page`).
pub fn grid_size_for(page_size: [f64; 2], glyphs_per_page: usize) -> Result<f64, ProofError> {
    check_page(page_size)?;
    let fits = |s: f64| {
        let (c, r) = grid_shape(page_size, s);
        c.saturating_mul(r) >= glyphs_per_page
    };
    if glyphs_per_page == 0 || glyphs_per_page > crate::MAX_SHEET_GLYPHS || !fits(MIN_SIZE) {
        return Err(ProofError::BadOptions(format!("{glyphs_per_page} glyphs do not fit on one page of this size")));
    }
    let (mut lo, mut hi) = (MIN_SIZE, page_size[0].max(page_size[1]));
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    // Two decimals, rounded down so the count still fits.
    let rounded = (lo * 100.0).floor() / 100.0;
    Ok(if fits(rounded) { rounded } else { lo })
}

pub(crate) fn check_page(page: [f64; 2]) -> Result<(), ProofError> {
    if page.iter().all(|s| s.is_finite() && (PAGE_SIDE.0..=PAGE_SIDE.1).contains(s)) {
        Ok(())
    } else {
        Err(ProofError::BadOptions(format!("page sides must be from {} to {} points", PAGE_SIDE.0, PAGE_SIDE.1)))
    }
}

/// Lays out every sheet of `options`. `gids` are the glyphs to show of the first face; the other
/// faces of a font set are shown only by the `Compare` sheet, which picks their glyphs itself.
pub(crate) fn layout(faces: &[Face<'_>], gids: &[u32], options: &ProofOptions, embedded: Option<Embedded>) -> Result<Document, ProofError> {
    check_page(options.page_size)?;
    let sizes = check_sizes(&options.sizes, options.page_size)?;
    let Some(face) = faces.first() else {
        return Err(ProofError::BadOptions("a proof needs a font".into()));
    };
    if options.sheets.is_empty() {
        return Err(ProofError::BadOptions("a proof needs at least one sheet".into()));
    }
    if gids.is_empty() {
        return Err(ProofError::BadOptions("there are no glyphs to show".into()));
    }
    let needs_outlines = options.sheets.iter().any(|s| !matches!(s, Sheet::Waterfall { .. } | Sheet::Compare));
    let mut glyphs = Vec::with_capacity(gids.len());
    if needs_outlines {
        let mut budget = crate::font::MAX_TOTAL_ELEMENTS;
        for &gid in gids {
            let outline = face.outline(gid, &mut budget)?;
            glyphs.push(ShownGlyph { font: 0, gid, name: face.name(gid).to_string(), unicode: face.unicode(gid), outline });
        }
    }
    let title = if faces.len() > 1 && options.sheets.iter().all(|s| *s == Sheet::Compare) {
        format!("font set of {}", faces.len())
    } else {
        format!("{} {}", face.family, face.style)
    };
    let mut doc = Document { title, pages: Vec::new(), glyphs, embedded: None };
    let mut sheet_names: Vec<&'static str> = Vec::new();
    // A size the caller gave; the tile layouts otherwise fit a number of glyphs per page.
    let given_size = options.sizes.first().copied();
    for sheet in &options.sheets {
        let before = doc.pages.len();
        let name = match sheet {
            Sheet::GlyphGrid => {
                grid(face, &mut doc, options.page_size, sizes.first().copied().unwrap_or(72.0))?;
                "glyphs"
            }
            Sheet::GlyphPerPage => {
                outline_pages(face, &mut doc, options.page_size)?;
                "outlines"
            }
            Sheet::Waterfall { text } => {
                waterfall(face, &mut doc, options.page_size, &sizes, text, gids)?;
                doc.embedded = embedded.clone();
                "waterfall"
            }
            Sheet::GridWithOutlines => {
                crate::layouts::linked::sheet(face, &mut doc, options.page_size, sizes.first().copied().unwrap_or(24.0))?;
                "glyphs and outlines"
            }
            Sheet::Boxes | Sheet::Hints | Sheet::Digi => {
                let displays = crate::layouts::Display::for_sheet(sheet, options)?;
                let fit = match (given_size, options.glyphs_per_page) {
                    (Some(size), _) => crate::layouts::tiles::Fit::Size(size),
                    (None, Some(n)) => crate::layouts::tiles::Fit::PerPage(n),
                    (None, None) => crate::layouts::tiles::Fit::PerPage(crate::layouts::default_per_page(sheet)),
                };
                crate::layouts::tiles::sheet(face, &mut doc, options, &displays, fit)?;
                match sheet {
                    Sheet::Boxes => "boxes",
                    Sheet::Hints => "hints",
                    _ => "digi",
                }
            }
            Sheet::Compare => {
                let displays = crate::layouts::Display::for_sheet(sheet, options)?;
                let size = given_size.unwrap_or(crate::layouts::compare::DEFAULT_SIZE);
                crate::layouts::compare::sheet(faces, &mut doc, options, &displays, size)?;
                "font set"
            }
        };
        sheet_names.extend(std::iter::repeat_n(name, doc.pages.len() - before));
    }
    let header = crate::layouts::Display::for_title(options)?;
    if header.page_title {
        let total = doc.pages.len();
        let size = header.page_title_size;
        for (i, (page, sheet)) in doc.pages.iter_mut().zip(sheet_names).enumerate() {
            let text = format!("{} \u{2014} {sheet} \u{2014} page {} of {total}", doc.title, i + 1);
            page.items.push(Item::Label { x: MARGIN, y: MARGIN + size, size, text, anchor: Anchor::Start, ink: Ink::Text });
        }
    }
    if options.black {
        for page in &mut doc.pages {
            for item in &mut page.items {
                match item {
                    Item::Line { ink, .. } | Item::Rect { ink, .. } | Item::Label { ink, .. } | Item::Point { ink, .. } => *ink = ink.black(),
                    Item::Glyph { .. } | Item::Sample { .. } | Item::Link { .. } => {}
                }
            }
        }
    }
    Ok(doc)
}

pub(crate) fn check_sizes(sizes: &[f64], page: [f64; 2]) -> Result<Vec<f64>, ProofError> {
    if sizes.len() > MAX_SIZES {
        return Err(ProofError::BadOptions(format!("a proof takes at most {MAX_SIZES} sizes")));
    }
    let [_, _, w, h] = content(page);
    let largest = w.min(h);
    for s in sizes {
        if !s.is_finite() || *s < MIN_SIZE || *s > largest {
            return Err(ProofError::BadOptions(format!("size {s} must be from {MIN_SIZE} to {largest} points on this page")));
        }
    }
    Ok(if sizes.is_empty() { vec![72.0_f64.min(largest)] } else { sizes.to_vec() })
}

pub(crate) fn new_page(doc: &mut Document, page: [f64; 2]) -> Result<&mut Page, ProofError> {
    if doc.pages.len() >= MAX_PAGES {
        return Err(ProofError::BadOptions(format!("the proof would have more than {MAX_PAGES} pages: select fewer glyphs or a smaller size")));
    }
    doc.pages.push(Page { width: page[0], height: page[1], items: Vec::new() });
    doc.pages.last_mut().ok_or_else(|| ProofError::BadOptions("no page".into()))
}

/// The scale and baseline offset that fit the font's ascender-to-descender height into `size`.
pub(crate) fn em_fit(face: &Face<'_>, size: f64) -> (f64, f64) {
    let height = (face.ascender - face.descender).max(face.upem);
    let scale = size / height;
    let baseline = (size - (face.ascender - face.descender) * scale) / 2.0 + face.ascender * scale;
    (scale, baseline)
}

/// Text cut to `max` characters, with an ellipsis when cut.
pub(crate) fn fit_text(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

pub(crate) fn grid(face: &Face<'_>, doc: &mut Document, page: [f64; 2], size: f64) -> Result<(), ProofError> {
    let (cols, rows) = grid_shape(page, size);
    let per_page = cols.saturating_mul(rows);
    if per_page == 0 {
        return Err(ProofError::BadOptions(format!("a glyph of {size} points does not fit on the page")));
    }
    let pages = doc.glyphs.len().div_ceil(per_page);
    if doc.pages.len().saturating_add(pages) > MAX_PAGES {
        return Err(ProofError::BadOptions(format!("the proof would have more than {MAX_PAGES} pages: select fewer glyphs or a smaller size")));
    }
    let [x0, y0, _, _] = content(page);
    let (cw, ch, label) = grid_cell(size);
    let (scale, baseline) = em_fit(face, size);
    let max_chars = ((size / (label * 0.55)).floor() as usize).max(1);
    let count = doc.glyphs.len();
    for start in (0..count).step_by(per_page) {
        let mut items = Vec::new();
        for (slot, index) in (start..count.min(start + per_page)).enumerate() {
            let Some(g) = doc.glyphs.get(index) else { continue };
            let (col, row) = (slot % cols, slot / cols);
            let (cx, cy) = (x0 + col as f64 * cw, y0 + row as f64 * ch);
            items.push(Item::Rect { x: cx, y: cy, w: cw, h: ch, ink: Ink::Guide });
            let x = cx + (cw - g.outline.advance * scale) / 2.0;
            items.push(Item::Glyph { glyph: index, x, y: cy + baseline, scale, style: GlyphStyle::Filled, primary: true });
            let name = fit_text(&g.name, max_chars);
            items.push(Item::Label {
                x: cx + label * 0.3,
                y: cy + size + label * 1.1,
                size: label,
                text: name,
                anchor: Anchor::Start,
                ink: Ink::Text,
            });
            let second = match g.unicode {
                Some(cp) => format!("{} {}", crate::text::code_point(cp), crate::text::num(g.outline.advance)),
                None => crate::text::num(g.outline.advance),
            };
            items.push(Item::Label {
                x: cx + label * 0.3,
                y: cy + size + label * 2.3,
                size: label,
                text: fit_text(&second, max_chars),
                anchor: Anchor::Start,
                ink: Ink::Faint,
            });
        }
        new_page(doc, page)?.items = items;
    }
    Ok(())
}

pub(crate) fn outline_pages(face: &Face<'_>, doc: &mut Document, page: [f64; 2]) -> Result<(), ProofError> {
    if doc.pages.len().saturating_add(doc.glyphs.len()) > MAX_PAGES {
        return Err(ProofError::BadOptions(format!("the proof would have more than {MAX_PAGES} pages: select fewer glyphs")));
    }
    let [x0, y0, w, h] = content(page);
    let info = 44.0;
    let draw_h = (h - info).max(1.0);
    let mut pages = Vec::with_capacity(doc.glyphs.len());
    for (index, g) in doc.glyphs.iter().enumerate() {
        let adv = g.outline.advance;
        let [bx0, by0, bx1, by1] = g.outline.bounds.unwrap_or([0.0, 0.0, adv, 0.0]);
        let (xmin, xmax) = (bx0.min(0.0), bx1.max(adv));
        let (ymin, ymax) = (by0.min(face.descender), by1.max(face.ascender));
        let pad = 0.05 * (xmax - xmin).max(ymax - ymin).max(1.0);
        let (xmin, xmax, ymin, ymax) = (xmin - pad, xmax + pad, ymin - pad, ymax + pad);
        let scale = (w / (xmax - xmin)).min(draw_h / (ymax - ymin));
        if !scale.is_finite() || scale <= 0.0 {
            return Err(ProofError::BadOptions(format!("glyph `{}` cannot be drawn on this page", g.name)));
        }
        let ox = x0 + (w - (xmax - xmin) * scale) / 2.0 - xmin * scale;
        let oy = y0 + (draw_h - (ymax - ymin) * scale) / 2.0 + ymax * scale;
        let at = |fx: f64, fy: f64| [ox + fx * scale, oy - fy * scale];
        let mut items = Vec::new();
        let metric = |name: &str, value: f64, items: &mut Vec<Item>| {
            let (from, to) = (at(xmin, value), at(xmax, value));
            items.push(Item::Line { from, to, ink: Ink::Guide });
            let text = format!("{name} {}", crate::text::num(value));
            items.push(Item::Label { x: from[0] + 2.0, y: from[1] - 2.0, size: 6.0, text, anchor: Anchor::Start, ink: Ink::Faint });
        };
        metric("ascender", face.ascender, &mut items);
        if let Some(v) = face.cap_height.filter(|v| *v != 0.0) {
            metric("cap height", v, &mut items);
        }
        if let Some(v) = face.x_height.filter(|v| *v != 0.0) {
            metric("x-height", v, &mut items);
        }
        metric("baseline", 0.0, &mut items);
        metric("descender", face.descender, &mut items);
        for x in [0.0, adv] {
            items.push(Item::Line { from: at(x, ymin), to: at(x, ymax), ink: Ink::Guide });
        }
        let [gx, gy] = at(0.0, 0.0);
        items.push(Item::Glyph { glyph: index, x: gx, y: gy, scale, style: GlyphStyle::Outlined, primary: true });
        points(&g.outline.els, &at, &mut items);
        // The information block: name, code point, metrics, and a small filled preview.
        let base = y0 + draw_h + 14.0;
        let mut head = format!("{}   glyph {}", g.name, g.gid);
        if let Some(cp) = g.unicode {
            head.push_str(&format!("   {}", crate::text::code_point(cp)));
        }
        items.push(Item::Label { x: x0, y: base, size: 11.0, text: head, anchor: Anchor::Start, ink: Ink::Text });
        let n = crate::text::num;
        let metrics = match g.outline.bounds {
            Some([a, b, c, d]) => {
                format!(
                    "advance {}   bounds {} {} {} {}   left side bearing {}   right side bearing {}",
                    n(adv),
                    n(a),
                    n(b),
                    n(c),
                    n(d),
                    n(a),
                    n(adv - c)
                )
            }
            None => format!("advance {}   no contours", n(adv)),
        };
        items.push(Item::Label { x: x0, y: base + 14.0, size: 8.0, text: metrics, anchor: Anchor::Start, ink: Ink::Faint });
        let preview = 28.0;
        let (pscale, pbase) = em_fit(face, preview);
        items.push(Item::Glyph {
            glyph: index,
            x: x0 + w - preview,
            y: base - 12.0 + pbase,
            scale: pscale,
            style: GlyphStyle::Filled,
            primary: false,
        });
        pages.push(items);
    }
    for items in pages {
        new_page(doc, page)?.items = items;
    }
    Ok(())
}

/// Point markers, handle lines and, for small glyphs, the coordinates of on-curve points.
fn points(els: &[El], at: &impl Fn(f64, f64) -> [f64; 2], items: &mut Vec<Item>) {
    let on_curve = els.iter().filter(|e| !matches!(e, El::Close)).count();
    let labels = on_curve <= MAX_POINT_LABELS;
    let mut cur: Option<(f64, f64)> = None;
    let on = |x: f32, y: f32, items: &mut Vec<Item>| {
        let (fx, fy) = (f64::from(x), f64::from(y));
        let [px, py] = at(fx, fy);
        items.push(Item::Point { x: px, y: py, on_curve: true, ink: Ink::Point });
        if labels {
            let text = format!("{},{}", crate::text::num(fx), crate::text::num(fy));
            items.push(Item::Label { x: px + 3.0, y: py - 3.0, size: 5.0, text, anchor: Anchor::Start, ink: Ink::Faint });
        }
        (fx, fy)
    };
    let off = |a: (f64, f64), c: (f32, f32), items: &mut Vec<Item>| {
        let [px, py] = at(f64::from(c.0), f64::from(c.1));
        items.push(Item::Line { from: at(a.0, a.1), to: [px, py], ink: Ink::Handle });
        items.push(Item::Point { x: px, y: py, on_curve: false, ink: Ink::Handle });
    };
    for el in els {
        match *el {
            El::Move(x, y) | El::Line(x, y) => cur = Some(on(x, y, items)),
            El::Quad(cx, cy, x, y) => {
                let end = (f64::from(x), f64::from(y));
                if let Some(c) = cur {
                    off(c, (cx, cy), items);
                }
                off(end, (cx, cy), items);
                cur = Some(on(x, y, items));
            }
            El::Cubic(c0x, c0y, c1x, c1y, x, y) => {
                if let Some(c) = cur {
                    off(c, (c0x, c0y), items);
                }
                off((f64::from(x), f64::from(y)), (c1x, c1y), items);
                cur = Some(on(x, y, items));
            }
            El::Close => {}
        }
    }
}

fn waterfall(face: &Face<'_>, doc: &mut Document, page: [f64; 2], sizes: &[f64], text: &str, gids: &[u32]) -> Result<(), ProofError> {
    let paragraphs: Vec<Vec<char>> = if text.trim().is_empty() {
        let chars: Vec<char> = gids.iter().filter_map(|g| face.unicode(*g)).filter_map(char::from_u32).filter(|c| !c.is_control()).collect();
        if chars.is_empty() {
            return Err(ProofError::BadOptions("none of the glyphs to show has a code point: give a text for the waterfall".into()));
        }
        vec![chars]
    } else {
        if text.chars().count() > MAX_TEXT_CHARS {
            return Err(ProofError::BadOptions(format!("the waterfall text is longer than {MAX_TEXT_CHARS} characters")));
        }
        text.lines().map(|l| l.chars().filter(|c| !c.is_control()).collect::<Vec<char>>()).filter(|l| !l.is_empty()).collect()
    };
    let [x0, y0, w, h] = content(page);
    let line_w = (w - WATERFALL_GUTTER).max(1.0);
    let largest = sizes.iter().copied().fold(MIN_SIZE, f64::max);
    let per_unit = largest / face.upem;
    // Groups of characters that fit one line at the largest size.
    let mut groups: Vec<String> = Vec::new();
    for para in paragraphs {
        let mut group = String::new();
        let mut width = 0.0;
        for c in para {
            let a = face.char_advance(c) * per_unit;
            if !group.is_empty() && width + a > line_w {
                groups.push(std::mem::take(&mut group));
                width = 0.0;
            }
            group.push(c);
            width += a;
        }
        if !group.is_empty() {
            groups.push(group);
        }
    }
    if groups.len().saturating_mul(sizes.len()) > MAX_WATERFALL_LINES {
        return Err(ProofError::BadOptions(format!(
            "the waterfall would have more than {MAX_WATERFALL_LINES} lines: give a shorter text or fewer sizes"
        )));
    }
    let bottom = y0 + h;
    let mut y = bottom + 1.0; // forces a first page
    let mut items: Vec<Item> = Vec::new();
    let mut started = false;
    for group in &groups {
        for &size in sizes {
            let lh = size * 1.25;
            if y + lh > bottom {
                if started {
                    new_page(doc, page)?.items = std::mem::take(&mut items);
                }
                started = true;
                y = y0;
            }
            let baseline = y + size;
            items.push(Item::Label { x: x0, y: baseline, size: 7.0, text: crate::text::num(size), anchor: Anchor::Start, ink: Ink::Faint });
            items.push(Item::Sample { x: x0 + WATERFALL_GUTTER, y: baseline, size, text: group.clone() });
            y += lh;
        }
        y += 8.0;
    }
    if started {
        new_page(doc, page)?.items = items;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_per_page_gives_a_size_that_fits_exactly_that_many_or_more() {
        let letter = [612.0, 792.0];
        let size = grid_size_for(letter, 156).unwrap();
        let (c, r) = grid_shape(letter, size);
        assert!(c * r >= 156, "{c} x {r} at {size}");
        let (c2, r2) = grid_shape(letter, size + 0.5);
        assert!(c2 * r2 < 156, "a larger size would not fit 156");
        assert!(grid_size_for(letter, 1).unwrap() > 500.0);
        assert!(grid_size_for(letter, 10_000_000).is_err());
        assert!(grid_size_for([10.0, 10.0], 1).is_err());
    }
}
