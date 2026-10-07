//! FontCraft compiler (L3): source model → OpenType binary.
//!
//! The M0 slice builds a **static TrueType-flavoured font from the first master**: `glyf`/`loca`
//! (components flattened, cubic curves converted to quadratic), `cmap` (formats 4 and 12), `head`,
//! `hhea`, `hmtx`, `maxp`, `name`, `OS/2` and `post`. It is deterministic: the same font and
//! options give the same bytes.
//!
//! Everything handed to the table writer is validated first. `write-fonts` panics or overflows on
//! points more than 32,767 units apart, on more than 65,277 glyph names outside the standard set and
//! on oversized name strings; each has a limit and a test here.
//!
//! What each table must hold at parity, and which reference compilers define it, is specified in
//! `docs/spec/40-compile.md` and `spec/inventory/40-tables.toml`. The plan for the rest of the
//! pipeline (layout, variations, CFF, hinting) is in `docs/decisions/0002-compile-pipeline.md`.
#![forbid(unsafe_code)]

use fontcraft_geom::{BezPath, PathEl, to_quadratic};
use fontcraft_model::{Font, Glyph, ModelError};
use write_fonts::FontBuilder;
use write_fonts::tables::cmap::Cmap;
use write_fonts::tables::glyf::{GlyfLocaBuilder, Glyph as TtGlyph, SimpleGlyph};
use write_fonts::tables::head::{Flags, Head, MacStyle};
use write_fonts::tables::hhea::Hhea;
use write_fonts::tables::hmtx::{Hmtx, LongMetric};
use write_fonts::tables::loca::LocaFormat;
use write_fonts::tables::maxp::Maxp;
use write_fonts::tables::name::{Name, NameRecord};
use write_fonts::tables::os2::{Os2, SelectionFlags};
use write_fonts::tables::post::Post;
use write_fonts::types::{FWord, Fixed, GlyphId, LongDateTime, NameId, Tag, UfWord, Version16Dot16};

/// Largest coordinate, in font units, an outline point may have in a TrueType outline. `glyf`
/// stores each point as a 16-bit difference from the one before, so two points may be at most
/// 32,767 units apart. (`write-fonts` 0.54 overflows past that; found by this crate's tests.)
pub const MAX_TT_COORDINATE: f64 = 16_383.0;
/// Most points one TrueType glyph may have (`maxp.maxPoints` and contour end indices are 16-bit).
pub const MAX_TT_POINTS: usize = 65_535;
/// Above this many glyphs the `post` table is written without names (format 3): format 2 can
/// index at most 65,277 names outside the standard set.
pub const MAX_POST_NAMES: usize = 65_000;

/// Seconds between 1904-01-01 (the `head` epoch) and 1970-01-01 (the Unix epoch).
const HEAD_EPOCH_OFFSET: i64 = 2_082_844_800;

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error(transparent)]
    Model(#[from] ModelError),
    #[error("glyph `{glyph}`: {message}")]
    Glyph { glyph: String, message: String },
    #[error("U+{codepoint:04X} is mapped to both `{first}` and `{second}`")]
    DuplicateCodepoint { codepoint: u32, first: String, second: String },
    #[error("cannot build the `{table}` table: {message}")]
    Table { table: &'static str, message: String },
    #[error("the font has no glyph to export")]
    NoGlyphs,
}

/// Options of [`compile_ttf`].
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Creation and modification time, in seconds since the Unix epoch. The compiler never reads
    /// the clock; callers that want "now" pass it (the engine does, and honours `SOURCE_DATE_EPOCH`).
    pub timestamp: i64,
    /// Most font units a quadratic approximation may be from the cubic curve it replaces.
    pub quad_tolerance: f64,
    /// Style name written to the `name` table. Empty: the first master's name.
    pub style_name: String,
}

impl Default for Options {
    fn default() -> Self {
        Self { timestamp: 0, quad_tolerance: fontcraft_geom::CUBIC_TO_QUAD_TOLERANCE, style_name: String::new() }
    }
}

/// What a compile produced, besides the bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Compiled {
    pub bytes: Vec<u8>,
    /// Final glyph order (`.notdef` first), by source glyph name.
    pub glyph_order: Vec<String>,
    /// Things the user should know that did not stop the compile.
    pub warnings: Vec<String>,
}

fn table<E: std::fmt::Display>(table: &'static str) -> impl Fn(E) -> CompileError {
    move |e| CompileError::Table { table, message: e.to_string() }
}

/// Rounds to the 16-bit signed range the tables store.
fn i16_of(v: f64) -> i16 {
    if v.is_finite() { v.round().clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16 } else { 0 }
}
fn u16_of(v: f64) -> u16 {
    if v.is_finite() { v.round().clamp(0.0, f64::from(u16::MAX)) as u16 } else { 0 }
}

/// Every point of `path`, on-curve and off-curve.
fn points(path: &BezPath) -> impl Iterator<Item = fontcraft_geom::Point> + '_ {
    path.elements().iter().flat_map(|el| {
        let (a, b, c) = match *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => (Some(p), None, None),
            PathEl::QuadTo(a, p) => (Some(a), Some(p), None),
            PathEl::CurveTo(a, b, p) => (Some(a), Some(b), Some(p)),
            PathEl::ClosePath => (None, None, None),
        };
        [a, b, c].into_iter().flatten()
    })
}

/// `path` with every coordinate rounded to whole font units.
fn rounded(path: &BezPath) -> BezPath {
    let r = |p: fontcraft_geom::Point| fontcraft_geom::Point::new(p.x.round(), p.y.round());
    path.elements()
        .iter()
        .map(|el| match *el {
            PathEl::MoveTo(p) => PathEl::MoveTo(r(p)),
            PathEl::LineTo(p) => PathEl::LineTo(r(p)),
            PathEl::QuadTo(c, p) => PathEl::QuadTo(r(c), r(p)),
            PathEl::CurveTo(a, b, p) => PathEl::CurveTo(r(a), r(b), r(p)),
            PathEl::ClosePath => PathEl::ClosePath,
        })
        .collect()
}

/// Does `path` have a contour that is not closed? (TrueType contours are always closed.)
fn has_open_contour(path: &BezPath) -> bool {
    let mut open = false;
    for el in path.elements() {
        match el {
            PathEl::MoveTo(_) if open => return true,
            PathEl::MoveTo(_) => open = true,
            PathEl::ClosePath => open = false,
            _ => {}
        }
    }
    open
}

/// One glyph of the output: its source name and, unless it was synthesised, its source glyph.
struct Slot<'a> {
    name: &'a str,
    glyph: Option<&'a Glyph>,
}

/// The glyphs to write, `.notdef` first (an empty one is added when the font has none), then the
/// font's own order.
fn glyph_order(font: &Font) -> Vec<Slot<'_>> {
    let exported = || font.glyphs.iter().map(|g| g.as_ref()).filter(|g| g.export);
    let mut order: Vec<Slot> = exported().filter(|g| g.name == ".notdef").map(|g| Slot { name: &g.name, glyph: Some(g) }).collect();
    if order.is_empty() {
        order.push(Slot { name: ".notdef", glyph: None });
    }
    order.extend(exported().filter(|g| g.name != ".notdef").map(|g| Slot { name: &g.name, glyph: Some(g) }));
    order
}

/// May `name` be written to the `post` table as it is? (At most 63 bytes of letters, digits,
/// period, underscore and hyphen, as glyph names are conventionally restricted.)
fn is_post_name(name: &str) -> bool {
    (1..=63).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Which of the four style-linked styles `style` names, as `(bold, italic)`; `None` for any
/// other style name ("Light", "Condensed Bold"…), which gets its own family in name IDs 1 and 2.
fn linked_style(style: &str) -> Option<(bool, bool)> {
    match style.to_ascii_lowercase().as_str() {
        "regular" => Some((false, false)),
        "bold" => Some((true, false)),
        "italic" => Some((false, true)),
        "bold italic" => Some((true, true)),
        _ => None,
    }
}

/// Compiles the first master of `font` to a static TrueType font.
pub fn compile_ttf(font: &Font, options: &Options) -> Result<Compiled, CompileError> {
    font.check()?;
    let master = font.default_master()?;
    let order = glyph_order(font);
    if order.iter().all(|s| s.glyph.is_none()) {
        return Err(CompileError::NoGlyphs);
    }
    let mut warnings = Vec::new();
    if order.first().is_some_and(|s| s.glyph.is_none()) {
        warnings.push("the font has no `.notdef` glyph; an empty one was added as glyph 0".to_string());
    }

    // Outlines and horizontal metrics.
    let mut glyf = GlyfLocaBuilder::new();
    let mut metrics = Vec::with_capacity(order.len());
    // Font bounding box as (x_min, y_min, x_max, y_max), from the glyphs' own boxes.
    let mut bounds: Option<(i16, i16, i16, i16)> = None;
    let (mut max_points, mut max_contours) = (0u16, 0u16);
    let (mut min_lsb, mut min_rsb, mut max_extent, mut max_advance) = (i16::MAX, i16::MAX, i16::MIN, 0u16);
    for slot in &order {
        let Some(g) = slot.glyph else {
            glyf.add_glyph(&TtGlyph::Empty).map_err(table("glyf"))?;
            metrics.push(LongMetric::new(u16_of(f64::from(font.units_per_em) / 2.0), 0));
            continue;
        };
        let glyph_error = |message: String| CompileError::Glyph { glyph: g.name.clone(), message };
        let source = font.outline_of(g, &master.id)?;
        // Checked on every point, control points included, and before any curve work: the
        // conversion below does work in proportion to the size of what it is given.
        if points(&source).any(|p| !(p.x.abs() <= MAX_TT_COORDINATE && p.y.abs() <= MAX_TT_COORDINATE)) {
            return Err(glyph_error(format!(
                "a point lies more than {MAX_TT_COORDINATE} units from the origin, further than a TrueType outline can store"
            )));
        }
        if has_open_contour(&source) {
            warnings.push(format!("glyph `{}` has an open path; TrueType outlines are closed, so it was closed", g.name));
        }
        let outline = rounded(&to_quadratic(&source, options.quad_tolerance));
        let width = g.layer(&master.id).map_or(0.0, |l| l.width);
        let advance = u16_of(width);
        if width.round() != f64::from(advance) {
            warnings.push(format!("glyph `{}`: the advance width {width} does not fit the font and was written as {advance}", g.name));
        }
        max_advance = max_advance.max(advance);
        if outline.elements().is_empty() {
            glyf.add_glyph(&TtGlyph::Empty).map_err(table("glyf"))?;
            metrics.push(LongMetric::new(advance, 0));
            continue;
        }
        let simple = SimpleGlyph::from_bezpath(&outline).map_err(|e| glyph_error(format!("{e:?}")))?;
        let point_count: usize = simple.contours.iter().map(|c| c.len()).sum();
        if point_count > MAX_TT_POINTS || simple.contours.len() > MAX_TT_POINTS {
            return Err(glyph_error(format!("the outline has {point_count} points; a TrueType glyph holds at most {MAX_TT_POINTS}")));
        }
        max_points = max_points.max(u16::try_from(point_count).unwrap_or(u16::MAX));
        max_contours = max_contours.max(u16::try_from(simple.contours.len()).unwrap_or(u16::MAX));
        // The glyph's own box (every point, as the glyph header stores it), so `hmtx`, `hhea` and
        // `head` agree with `glyf`.
        let b = simple.bbox;
        bounds =
            Some(bounds.map_or((b.x_min, b.y_min, b.x_max, b.y_max), |f| (f.0.min(b.x_min), f.1.min(b.y_min), f.2.max(b.x_max), f.3.max(b.y_max))));
        min_lsb = min_lsb.min(b.x_min);
        min_rsb = min_rsb.min(i16_of(f64::from(advance) - f64::from(b.x_max)));
        max_extent = max_extent.max(b.x_max);
        metrics.push(LongMetric::new(advance, b.x_min));
        glyf.add_glyph(&simple).map_err(table("glyf"))?;
    }
    let (glyf, loca, loca_format) = glyf.build();
    let bounds = bounds.unwrap_or((0, 0, 0, 0));
    if min_lsb == i16::MAX {
        (min_lsb, min_rsb, max_extent) = (0, 0, 0);
    }
    let glyph_count = u16::try_from(order.len()).map_err(|_| ModelError::TooManyGlyphs)?;

    // cmap: every code point to its glyph id; one code point may not name two glyphs.
    let mut mappings: Vec<(char, GlyphId)> = Vec::new();
    let mut owners: std::collections::BTreeMap<u32, &str> = std::collections::BTreeMap::new();
    for (gid, slot) in order.iter().enumerate() {
        for &cp in slot.glyph.map(|g| g.unicodes.as_slice()).unwrap_or_default() {
            let Some(ch) = char::from_u32(cp) else { continue };
            if let Some(first) = owners.insert(cp, slot.name) {
                if first != slot.name {
                    return Err(CompileError::DuplicateCodepoint { codepoint: cp, first: first.to_string(), second: slot.name.to_string() });
                }
                continue;
            }
            mappings.push((ch, GlyphId::new(u32::try_from(gid).unwrap_or(0))));
        }
    }
    let (first_char, last_char) = (owners.keys().next().copied().unwrap_or(0), owners.keys().next_back().copied().unwrap_or(0));
    let has = |range: std::ops::RangeInclusive<u32>| owners.range(range).next().is_some();
    let cmap = Cmap::from_mappings(mappings).map_err(|e| CompileError::Table { table: "cmap", message: format!("{e:?}") })?;

    let style = if options.style_name.is_empty() { master.name.as_str() } else { options.style_name.as_str() };
    let (bold, italic) = linked_style(style).unwrap_or((false, false));
    let mut selection = SelectionFlags::USE_TYPO_METRICS;
    let mut mac_style = MacStyle::empty();
    if bold {
        selection |= SelectionFlags::BOLD;
        mac_style |= MacStyle::BOLD;
    }
    if italic {
        selection |= SelectionFlags::ITALIC;
        mac_style |= MacStyle::ITALIC;
    }
    if !bold && !italic {
        selection |= SelectionFlags::REGULAR;
    }

    let (ascender, descender) = (i16_of(master.ascender), i16_of(master.descender));
    let line_gap = i16_of((f64::from(font.units_per_em) * 1.2 - (master.ascender - master.descender)).max(0.0));
    let time = LongDateTime::new(options.timestamp.saturating_add(HEAD_EPOCH_OFFSET));
    let head = Head {
        font_revision: Fixed::from_f64(f64::from(font.version_major) + f64::from(font.version_minor) / 1000.0),
        // Baseline at y = 0 and left sidebearing at x = 0 (lsb equals the glyph's xMin above).
        flags: Flags::BASELINE_AT_Y_0 | Flags::LSB_AT_X_0,
        units_per_em: font.units_per_em,
        created: time,
        modified: time,
        x_min: bounds.0,
        y_min: bounds.1,
        x_max: bounds.2,
        y_max: bounds.3,
        mac_style,
        lowest_rec_ppem: 6,
        index_to_loc_format: match loca_format {
            LocaFormat::Short => 0,
            LocaFormat::Long => 1,
        },
        ..Default::default()
    };
    let hhea = Hhea {
        ascender: FWord::new(ascender),
        descender: FWord::new(descender),
        line_gap: FWord::new(line_gap),
        advance_width_max: UfWord::new(max_advance),
        min_left_side_bearing: FWord::new(min_lsb),
        min_right_side_bearing: FWord::new(min_rsb),
        x_max_extent: FWord::new(max_extent),
        caret_slope_rise: 1,
        caret_slope_run: 0,
        caret_offset: 0,
        number_of_h_metrics: glyph_count,
    };
    let maxp = Maxp {
        num_glyphs: glyph_count,
        max_points: Some(max_points),
        max_contours: Some(max_contours),
        max_composite_points: Some(0),
        max_composite_contours: Some(0),
        max_zones: Some(1),
        max_twilight_points: Some(0),
        max_storage: Some(0),
        max_function_defs: Some(0),
        max_instruction_defs: Some(0),
        max_stack_elements: Some(0),
        max_size_of_instructions: Some(0),
        max_component_elements: Some(0),
        max_component_depth: Some(0),
    };

    let nonzero: Vec<f64> = metrics.iter().map(|m| f64::from(m.advance)).filter(|a| *a > 0.0).collect();
    let average = if nonzero.is_empty() { 0.0 } else { nonzero.iter().sum::<f64>() / nonzero.len() as f64 };
    // Only the two ranges this slice can vouch for; the full range and code-page tables are M2.
    let basic_latin = u32::from(has(0x20..=0x7E));
    let latin_1 = u32::from(has(0xA0..=0xFF)) << 1;
    let os2 = Os2 {
        x_avg_char_width: i16_of(average),
        us_weight_class: if bold { 700 } else { 400 },
        us_width_class: 5,
        y_subscript_x_size: i16_of(f64::from(font.units_per_em) * 0.65),
        y_subscript_y_size: i16_of(f64::from(font.units_per_em) * 0.6),
        y_subscript_y_offset: i16_of(f64::from(font.units_per_em) * 0.075),
        y_superscript_x_size: i16_of(f64::from(font.units_per_em) * 0.65),
        y_superscript_y_size: i16_of(f64::from(font.units_per_em) * 0.6),
        y_superscript_y_offset: i16_of(f64::from(font.units_per_em) * 0.35),
        y_strikeout_size: i16_of(f64::from(font.units_per_em) * 0.05),
        y_strikeout_position: i16_of(master.x_height * 0.6),
        ul_unicode_range_1: basic_latin | latin_1,
        ach_vend_id: Tag::new(b"NONE"),
        fs_selection: selection,
        us_first_char_index: u16::try_from(first_char).unwrap_or(u16::MAX),
        us_last_char_index: u16::try_from(last_char).unwrap_or(u16::MAX),
        s_typo_ascender: ascender,
        s_typo_descender: descender,
        s_typo_line_gap: line_gap,
        us_win_ascent: u16_of(master.ascender.max(f64::from(bounds.3))),
        us_win_descent: u16_of((-master.descender).max(-f64::from(bounds.1))),
        ul_code_page_range_1: Some(basic_latin), // bit 0: Latin 1
        ul_code_page_range_2: Some(0),
        sx_height: Some(i16_of(master.x_height)),
        s_cap_height: Some(i16_of(master.cap_height)),
        us_default_char: Some(0),
        us_break_char: Some(if owners.contains_key(&0x20) { 0x20 } else { 0 }),
        us_max_context: Some(0),
        ..Default::default()
    };

    // Glyph names: written when every name is a conventional one and there are few enough.
    let post_names: Vec<String> = order
        .iter()
        .enumerate()
        .map(|(gid, slot)| {
            if is_post_name(slot.name) {
                slot.name.to_string()
            } else {
                warnings.push(format!(
                    "glyph `{}` is written to the font as `glyph{gid:05}`: its name is not a valid glyph name there",
                    slot.name.escape_debug()
                ));
                format!("glyph{gid:05}")
            }
        })
        .collect();
    let mut post = if order.len() > MAX_POST_NAMES {
        warnings.push(format!("the font has more than {MAX_POST_NAMES} glyphs, so glyph names were left out of it"));
        Post { version: Version16Dot16::VERSION_3_0, ..Default::default() }
    } else {
        Post::new_v2(post_names.iter().map(String::as_str))
    };
    post.underline_position = FWord::new(i16_of(-f64::from(font.units_per_em) * 0.1));
    post.underline_thickness = FWord::new(i16_of(f64::from(font.units_per_em) * 0.05));

    // Names. A style outside Regular, Bold, Italic and Bold Italic becomes its own family in the
    // legacy IDs 1 and 2, with the real family and style in IDs 16 and 17.
    let version = format!("Version {}.{:03}", font.version_major, font.version_minor);
    let full = format!("{} {style}", font.family_name);
    let postscript: String =
        format!("{}-{style}", font.family_name).chars().filter(|c| c.is_ascii_graphic() && !"[](){}<>/%".contains(*c)).take(63).collect();
    let unique = format!("{version};NONE;{postscript}");
    let record = |id: u16, s: &str| NameRecord::new(3, 1, 0x409, NameId::new(id), s.to_string().into());
    let linked = linked_style(style).is_some();
    let (legacy_family, legacy_style) = if linked { (font.family_name.clone(), style.to_string()) } else { (full.clone(), "Regular".to_string()) };
    let mut records =
        vec![record(1, &legacy_family), record(2, &legacy_style), record(3, &unique), record(4, &full), record(5, &version), record(6, &postscript)];
    if !linked {
        records.push(record(16, &font.family_name));
        records.push(record(17, style));
    }
    let name = Name::new(records);

    let mut builder = FontBuilder::new();
    builder.add_table(&head).map_err(table("head"))?;
    builder.add_table(&hhea).map_err(table("hhea"))?;
    builder.add_table(&maxp).map_err(table("maxp"))?;
    builder.add_table(&os2).map_err(table("OS/2"))?;
    builder.add_table(&Hmtx::new(metrics, Vec::new())).map_err(table("hmtx"))?;
    builder.add_table(&cmap).map_err(table("cmap"))?;
    builder.add_table(&loca).map_err(table("loca"))?;
    builder.add_table(&glyf).map_err(table("glyf"))?;
    builder.add_table(&name).map_err(table("name"))?;
    builder.add_table(&post).map_err(table("post"))?;
    Ok(Compiled { bytes: builder.build(), glyph_order: order.iter().map(|s| s.name.to_string()).collect(), warnings })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use fontcraft_model::{Component, Extra, Layer, Node, NodeKind, Path, Shape};
    use skrifa::instance::{LocationRef, Size};
    use skrifa::raw::TableProvider;
    use skrifa::raw::types::GlyphId16;
    use skrifa::string::StringId;
    use skrifa::{FontRef, MetadataProvider};

    fn path(closed: bool, nodes: Vec<Node>) -> Shape {
        Shape::Path(Path { closed, nodes, extra: Extra::new() })
    }

    fn sample() -> Font {
        let mut f = Font::new("Slice Sans");
        let n = Node::new;
        let a = f.glyph_mut("A").unwrap();
        a.layers[0].width = 620.0;
        a.layers[0].shapes.push(Shape::Path(Path::rect(60.0, 0.0, 560.0, 700.0)));
        let o = f.glyph_mut("O").unwrap();
        // A four-curve oval: cubic input must come out as quadratic TrueType curves.
        let k = 0.5523;
        let (cx, cy, rx, ry) = (350.0, 350.0, 300.0, 360.0);
        o.layers[0].width = 700.0;
        o.layers[0].shapes.push(path(
            true,
            vec![
                n(cx + rx, cy, NodeKind::Curve),
                n(cx + rx, cy + ry * k, NodeKind::OffCurve),
                n(cx + rx * k, cy + ry, NodeKind::OffCurve),
                n(cx, cy + ry, NodeKind::Curve),
                n(cx - rx * k, cy + ry, NodeKind::OffCurve),
                n(cx - rx, cy + ry * k, NodeKind::OffCurve),
                n(cx - rx, cy, NodeKind::Curve),
                n(cx - rx, cy - ry * k, NodeKind::OffCurve),
                n(cx - rx * k, cy - ry, NodeKind::OffCurve),
                n(cx, cy - ry, NodeKind::Curve),
                n(cx + rx * k, cy - ry, NodeKind::OffCurve),
                n(cx + rx, cy - ry * k, NodeKind::OffCurve),
            ],
        ));
        f.glyph_mut("B").unwrap().layers[0].shapes.push(Shape::Component(Component::new("A", 10.0, 0.0)));
        f
    }

    fn names(font: &FontRef, id: StringId) -> Vec<String> {
        font.localized_strings(id).map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_compiled_font_reads_back() {
        let out = compile_ttf(&sample(), &Options::default()).unwrap();
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(out.glyph_order.first().map(String::as_str), Some(".notdef"));
        let font = FontRef::new(&out.bytes).unwrap();
        assert_eq!(font.head().unwrap().units_per_em(), 1000);
        assert_eq!(font.head().unwrap().flags().bits(), 0x0003);
        assert_eq!(usize::from(font.maxp().unwrap().num_glyphs()), out.glyph_order.len());
        let gid = font.charmap().map('A').unwrap();
        assert_eq!(out.glyph_order[gid.to_u32() as usize], "A");
        let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
        assert_eq!(metrics.advance_width(gid), Some(620.0));
        assert_eq!(metrics.left_side_bearing(gid), Some(60.0));
        let bounds = metrics.bounds(gid).unwrap();
        assert_eq!((bounds.x_min, bounds.y_min, bounds.x_max, bounds.y_max), (60.0, 0.0, 560.0, 700.0));
        // The component was flattened and shifted.
        let b = metrics.bounds(font.charmap().map('B').unwrap()).unwrap();
        assert_eq!((b.x_min, b.x_max), (70.0, 570.0));
        // The oval kept its extremes through the cubic → quadratic conversion.
        let o = metrics.bounds(font.charmap().map('O').unwrap()).unwrap();
        assert_eq!((o.x_min, o.y_min, o.x_max, o.y_max), (50.0, -10.0, 650.0, 710.0));
        assert_eq!(names(&font, StringId::FAMILY_NAME), ["Slice Sans"]);
        assert_eq!(names(&font, StringId::SUBFAMILY_NAME), ["Regular"]);
        let os2 = font.os2().unwrap();
        assert_eq!(os2.s_cap_height(), Some(700));
        assert_eq!(os2.us_break_char(), Some(0x20));
        assert_eq!(os2.ul_unicode_range_1() & 1, 1, "Basic Latin is covered");
        assert_eq!(font.post().unwrap().glyph_name(GlyphId16::new(gid.to_u32() as u16)), Some("A"));
    }

    #[test]
    fn compiling_is_deterministic_and_the_timestamp_is_an_input() {
        let font = sample();
        let a = compile_ttf(&font, &Options::default()).unwrap();
        let b = compile_ttf(&font, &Options::default()).unwrap();
        assert_eq!(a.bytes, b.bytes);
        let later = compile_ttf(&font, &Options { timestamp: 1_800_000_000, ..Default::default() }).unwrap();
        assert_ne!(a.bytes, later.bytes);
        let head = FontRef::new(&later.bytes).unwrap().head().unwrap().created().as_secs();
        assert_eq!(head, 1_800_000_000 + HEAD_EPOCH_OFFSET);
    }

    #[test]
    fn problems_are_errors_with_the_glyph_named() {
        let mut f = sample();
        f.glyph_mut("C").unwrap().unicodes = vec![0x41];
        assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::DuplicateCodepoint { codepoint: 0x41, .. })));

        let mut f = sample();
        f.glyph_mut("C").unwrap().layers[0].shapes.push(Shape::Component(Component::new("missing", 0.0, 0.0)));
        assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::Model(ModelError::NoSuchGlyph(_)))));

        let mut f = sample();
        f.glyph_mut("C").unwrap().layers[0].shapes.push(path(true, vec![Node::new(0.0, 0.0, NodeKind::Curve)]));
        assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::Model(ModelError::BadOutline { .. }))));

        let mut f = sample();
        for g in &mut f.glyphs {
            Arc::make_mut(g).export = false;
        }
        assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::NoGlyphs)));
    }

    #[test]
    fn far_points_are_errors_before_any_curve_work() {
        let n = Node::new;
        // On-curve points at any distance, including ones that once exhausted memory in the
        // cubic conversion and overflowed the glyph writer.
        for far in [16_384.0, 40_000.0, 1e5, 1e6] {
            let mut f = sample();
            f.glyph_mut("C").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(-far, -far, far, far)));
            assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::Glyph { glyph, .. }) if glyph == "C"), "{far}");
        }
        // A control point out of range while the curve itself stays inside.
        let mut f = sample();
        f.glyph_mut("C").unwrap().layers[0]
            .shapes
            .push(path(true, vec![n(-16_000.0, 0.0, NodeKind::Line), n(30_000.0, 0.0, NodeKind::OffCurve), n(-16_000.0, 10.0, NodeKind::QCurve)]));
        assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::Glyph { glyph, .. }) if glyph == "C"));
        // A huge cubic: an error at once, not a long conversion.
        let mut f = sample();
        let big = MAX_TT_COORDINATE * 50.0;
        f.glyph_mut("C").unwrap().layers[0].shapes.push(path(
            true,
            vec![n(0.0, 0.0, NodeKind::Line), n(big, 0.0, NodeKind::OffCurve), n(0.0, big, NodeKind::OffCurve), n(big, big, NodeKind::Curve)],
        ));
        let start = std::time::Instant::now();
        assert!(compile_ttf(&f, &Options::default()).is_err());
        assert!(start.elapsed().as_millis() < 2000);
        // The largest allowed outline compiles.
        let mut f = sample();
        f.glyph_mut("C").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(
            -MAX_TT_COORDINATE,
            -MAX_TT_COORDINATE,
            MAX_TT_COORDINATE,
            MAX_TT_COORDINATE,
        )));
        compile_ttf(&f, &Options::default()).unwrap();
    }

    #[test]
    fn odd_values_become_warnings() {
        let mut f = sample();
        f.glyph_mut("A").unwrap().layers[0].width = 70_000.0;
        f.glyph_mut("C").unwrap().layers[0].shapes.push(path(
            false,
            vec![Node::new(0.0, 0.0, NodeKind::Line), Node::new(100.0, 0.0, NodeKind::Line), Node::new(100.0, 100.0, NodeKind::Line)],
        ));
        let out = compile_ttf(&f, &Options::default()).unwrap();
        assert!(out.warnings.iter().any(|w| w.contains("advance width")));
        assert!(out.warnings.iter().any(|w| w.contains("open path")));
        assert!(FontRef::new(&out.bytes).is_ok());
    }

    #[test]
    fn too_many_points_is_an_error_not_a_corrupt_glyph() {
        let mut f = sample();
        let nodes: Vec<Node> = (0..70_000).map(|i| Node::new(f64::from(i % 1000), f64::from(i / 1000), NodeKind::Line)).collect();
        f.glyph_mut("C").unwrap().layers[0].shapes.push(path(true, nodes));
        assert!(matches!(compile_ttf(&f, &Options::default()), Err(CompileError::Glyph { message, .. }) if message.contains("70000 points")));
    }

    #[test]
    fn sidebearings_and_boxes_agree_with_the_glyph_header() {
        // A curve whose control point lies left of the curve itself: the glyph header stores the
        // box of all points, and lsb, hhea and head must say the same.
        let n = Node::new;
        let mut f = sample();
        f.glyph_mut("C").unwrap().layers[0].shapes.push(path(
            true,
            vec![
                n(100.0, 0.0, NodeKind::Line),
                n(-100.0, 350.0, NodeKind::OffCurve),
                n(100.0, 700.0, NodeKind::QCurve),
                n(300.0, 350.0, NodeKind::Line),
            ],
        ));
        let out = compile_ttf(&f, &Options::default()).unwrap();
        let font = FontRef::new(&out.bytes).unwrap();
        let gid = font.charmap().map('C').unwrap();
        let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
        assert_eq!(metrics.left_side_bearing(gid), Some(-100.0));
        assert_eq!(font.head().unwrap().x_min(), -100);
        assert_eq!(font.hhea().unwrap().min_left_side_bearing().to_i16(), -100);
    }

    #[test]
    fn a_font_without_notdef_gets_an_empty_one() {
        let mut f = sample();
        f.remove_glyph(".notdef").unwrap();
        let out = compile_ttf(&f, &Options::default()).unwrap();
        assert!(out.warnings.iter().any(|w| w.contains(".notdef")));
        assert_eq!(out.glyph_order.first().map(String::as_str), Some(".notdef"));
        let font = FontRef::new(&out.bytes).unwrap();
        assert_ne!(font.charmap().map('A').unwrap().to_u32(), 0, "glyph 0 is .notdef, not a letter");
    }

    #[test]
    fn unusual_glyph_names_are_replaced_in_the_font_not_written_raw() {
        let mut f = sample();
        f.add_glyph(&"n".repeat(200), vec![0xE000], 500.0).unwrap();
        f.add_glyph("with space", vec![0xE001], 500.0).unwrap();
        f.add_glyph("ça", vec![0xE002], 500.0).unwrap();
        let out = compile_ttf(&f, &Options::default()).unwrap();
        assert_eq!(out.warnings.iter().filter(|w| w.contains("not a valid glyph name")).count(), 3);
        let font = FontRef::new(&out.bytes).unwrap();
        let post = font.post().unwrap();
        for (ch, _) in [('\u{E000}', ()), ('\u{E001}', ()), ('\u{E002}', ())] {
            let gid = font.charmap().map(ch).unwrap().to_u32();
            assert_eq!(post.glyph_name(GlyphId16::new(gid as u16)), Some(format!("glyph{gid:05}").as_str()));
        }
        // The glyph after them still has its own name: the table is not shifted.
        assert_eq!(post.glyph_name(GlyphId16::new(font.charmap().map('z').unwrap().to_u32() as u16)), Some("z"));
    }

    #[test]
    fn more_glyphs_than_post_can_name_fall_back_to_a_nameless_table() {
        let mut f = Font::new("Many");
        let master = f.masters[0].id.clone();
        for i in 0..(MAX_POST_NAMES as u32 + 100) {
            let mut g = Glyph::new(format!("glyph{i:05}x"));
            g.layers.push(Layer::new(&master, 500.0));
            f.glyphs.push(Arc::new(g));
        }
        let out = compile_ttf(&f, &Options::default()).unwrap();
        assert!(out.warnings.iter().any(|w| w.contains("glyph names were left out")));
        let font = FontRef::new(&out.bytes).unwrap();
        assert_eq!(font.post().unwrap().version(), Version16Dot16::VERSION_3_0);
        assert_eq!(usize::from(font.maxp().unwrap().num_glyphs()), f.glyphs.len());
    }

    #[test]
    fn style_names_drive_style_linking() {
        let compile = |style: &str| {
            let mut f = sample();
            f.masters[0].name = style.to_string();
            compile_ttf(&f, &Options::default()).unwrap().bytes
        };
        let bytes = compile("Bold Italic");
        let font = FontRef::new(&bytes).unwrap();
        assert_eq!(font.os2().unwrap().fs_selection().bits() & 0x61, 0x21, "ITALIC and BOLD set, REGULAR clear");
        assert_eq!(font.head().unwrap().mac_style().bits(), 0x3);
        assert_eq!(names(&font, StringId::SUBFAMILY_NAME), ["Bold Italic"]);
        assert!(names(&font, StringId::TYPOGRAPHIC_FAMILY_NAME).is_empty());

        // A style outside the four linked ones is its own legacy family.
        let bytes = compile("Light");
        let font = FontRef::new(&bytes).unwrap();
        assert_eq!(font.os2().unwrap().fs_selection().bits() & 0x61, 0x40, "REGULAR only");
        assert_eq!(names(&font, StringId::FAMILY_NAME), ["Slice Sans Light"]);
        assert_eq!(names(&font, StringId::SUBFAMILY_NAME), ["Regular"]);
        assert_eq!(names(&font, StringId::TYPOGRAPHIC_FAMILY_NAME), ["Slice Sans"]);
        assert_eq!(names(&font, StringId::TYPOGRAPHIC_SUBFAMILY_NAME), ["Light"]);
    }

    #[test]
    fn a_font_without_a_space_has_no_break_character() {
        let mut f = sample();
        f.remove_glyph("space").unwrap();
        let out = compile_ttf(&f, &Options::default()).unwrap();
        assert_eq!(FontRef::new(&out.bytes).unwrap().os2().unwrap().us_break_char(), Some(0));
    }
}
