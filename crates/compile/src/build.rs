//! Per-table builders and assembly (`docs/spec/40-compile.md` §§ 1.5, 1.8).
//!
//! The builders read only the resolved font, the final glyph order and the prepared outlines.
//! `glyf`/`loca` come first: composite bounds, side bearings, `hhea` extents, `maxp` limits and
//! the `head` box all follow from the written glyphs. The other tables keep the M0 compiler's
//! values until M2.2 brings each to the reference compiler's.
//!
//! Composite bounds and `maxp`'s composite limits follow fontc (`fontbe/src/glyphs.rs`,
//! `fontbe/src/metrics_and_limits.rs`, Apache-2.0): a composite's box is the box of its simple
//! glyphs' written points under the written component transforms, and its limits add up the
//! points and contours it reaches.

use std::collections::{BTreeMap, HashMap};

use fontcraft_geom::{Affine, Point, Rect};
use fontcraft_model::{MAX_OUTLINE_ELEMENTS, ModelError, Resolved, ResolvedGlyph};
use write_fonts::FontBuilder;
use write_fonts::tables::cmap::Cmap;
use write_fonts::tables::glyf::{Anchor, Bbox, Component, ComponentFlags, CompositeGlyph, GlyfLocaBuilder, Glyph as TtGlyph, SimpleGlyph, Transform};
use write_fonts::tables::head::{Flags, Head, MacStyle};
use write_fonts::tables::hhea::Hhea;
use write_fonts::tables::hmtx::{Hmtx, LongMetric};
use write_fonts::tables::loca::LocaFormat;
use write_fonts::tables::maxp::Maxp;
use write_fonts::tables::name::{Name, NameRecord};
use write_fonts::tables::os2::{Os2, SelectionFlags};
use write_fonts::tables::post::Post;
use write_fonts::types::{F2Dot14, FWord, Fixed, GlyphId, GlyphId16, LongDateTime, NameId, Tag, UfWord, Version16Dot16};

use crate::outline::Prepared;
use crate::{CompileError, MAX_POST_NAMES, MAX_TT_POINTS, Options, i16_of, u16_of};

/// Seconds between 1904-01-01 (the `head` epoch) and 1970-01-01 (the Unix epoch).
const HEAD_EPOCH_OFFSET: i64 = 2_082_844_800;

fn table<E: std::fmt::Display>(table: &'static str) -> impl Fn(E) -> CompileError {
    move |e| CompileError::Table { table, message: e.to_string() }
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

/// A glyph as written to `glyf`, with what the other tables need from it.
struct Built {
    glyph: TtGlyph,
    /// Points and contours (simple glyphs), for `maxp`.
    points: u16,
    contours: u16,
}

/// The written component transform as an affine map: F2Dot14 values and whole-unit offsets, as
/// a reader of the font sees it.
fn written_affine(c: &Component) -> Affine {
    let Anchor::Offset { x, y } = c.anchor else { return Affine::IDENTITY };
    let t = &c.transform;
    Affine::new([f64::from(t.xx.to_f32()), f64::from(t.yx.to_f32()), f64::from(t.xy.to_f32()), f64::from(t.yy.to_f32()), f64::from(x), f64::from(y)])
}

/// `glyf` entries for every glyph of the order.
fn build_glyphs(order: &[ResolvedGlyph], outlines: &[Prepared]) -> Result<Vec<Built>, CompileError> {
    let gids: HashMap<&str, u16> = order.iter().enumerate().filter_map(|(i, g)| Some((g.name.as_str(), u16::try_from(i).ok()?))).collect();
    let mut built = Vec::with_capacity(order.len());
    for (g, prepared) in order.iter().zip(outlines) {
        let glyph_error = |message: String| CompileError::Glyph { glyph: g.name.clone(), message };
        let entry = match prepared {
            Prepared::Empty => Built { glyph: TtGlyph::Empty, points: 0, contours: 0 },
            Prepared::Simple(path) => {
                let simple = SimpleGlyph::from_bezpath(path).map_err(|e| glyph_error(format!("{e:?}")))?;
                let point_count: usize = simple.contours.iter().map(|c| c.len()).sum();
                if point_count > MAX_TT_POINTS || simple.contours.len() > MAX_TT_POINTS {
                    return Err(glyph_error(format!("the outline has {point_count} points; a TrueType glyph holds at most {MAX_TT_POINTS}")));
                }
                let (points, contours) = (u16::try_from(point_count).unwrap_or(u16::MAX), u16::try_from(simple.contours.len()).unwrap_or(u16::MAX));
                Built { glyph: if contours == 0 { TtGlyph::Empty } else { TtGlyph::Simple(simple) }, points, contours }
            }
            Prepared::Composite(components) => {
                let mut parts = Vec::with_capacity(components.len());
                for c in components {
                    let gid = gids.get(c.base.as_str()).ok_or_else(|| glyph_error(format!("the component `{}` is not in the font", c.base)))?;
                    let [a, b, cc, d, e, f] = c.transform.as_coeffs();
                    let component = Component::new(
                        GlyphId16::new(*gid),
                        Anchor::Offset { x: i16_of(e), y: i16_of(f) },
                        Transform { xx: F2Dot14::from_f64(a), yx: F2Dot14::from_f64(b), xy: F2Dot14::from_f64(cc), yy: F2Dot14::from_f64(d) },
                        // Always on, as fontc and ufo2ft write it (`compile.componentFlags`).
                        ComponentFlags { round_xy_to_grid: true, ..Default::default() },
                    );
                    parts.push((component, Bbox::default()));
                }
                let composite = CompositeGlyph::try_from_iter(parts).map_err(|e| glyph_error(e.to_string()))?;
                Built { glyph: TtGlyph::Composite(composite), points: 0, contours: 0 }
            }
        };
        built.push(entry);
    }
    composite_boxes(&mut built, order)?;
    Ok(built)
}

/// Sets every composite's box (`compile.compositeBounds`): the written points of the simple
/// glyphs it reaches, under the written transforms, rounded half up.
fn composite_boxes(built: &mut [Built], order: &[ResolvedGlyph]) -> Result<(), CompileError> {
    let mut boxes: Vec<(usize, Bbox)> = Vec::new();
    for (i, b) in built.iter().enumerate() {
        let TtGlyph::Composite(composite) = &b.glyph else { continue };
        let mut budget = MAX_OUTLINE_ELEMENTS;
        let mut rect: Option<Rect> = None;
        reach(built, composite, Affine::IDENTITY, 0, &mut budget, &mut rect)
            .ok_or_else(|| ModelError::OutlineTooLarge(order.get(i).map_or_else(String::new, |g| g.name.clone())))?;
        boxes.push((i, rect.unwrap_or_default().into()));
    }
    for (i, bbox) in boxes {
        if let Some(Built { glyph: TtGlyph::Composite(c), .. }) = built.get_mut(i) {
            c.bbox = bbox;
        }
    }
    Ok(())
}

/// Adds the points `composite` draws under `affine` to `rect`. `None` when the work exceeds the
/// budget or the nesting the depth limit.
fn reach(built: &[Built], composite: &CompositeGlyph, affine: Affine, depth: usize, budget: &mut usize, rect: &mut Option<Rect>) -> Option<()> {
    if depth > fontcraft_model::MAX_COMPONENT_DEPTH {
        return None;
    }
    for c in composite.components() {
        let affine = affine * written_affine(c);
        match &built.get(usize::from(c.glyph.to_u16()))?.glyph {
            TtGlyph::Empty => {}
            TtGlyph::Simple(s) => {
                for p in s.contours.iter().flat_map(|c| c.iter()) {
                    *budget = budget.checked_sub(1)?;
                    let p = affine * Point::new(f64::from(p.x), f64::from(p.y));
                    *rect = Some(rect.map_or_else(|| Rect::from_points(p, p), |r| r.union_pt(p)));
                }
            }
            TtGlyph::Composite(inner) => reach(built, inner, affine, depth + 1, budget, rect)?,
        }
    }
    Some(())
}

/// `maxp`'s composite limits: (points, contours, depth) of the largest composite, as fontTools
/// computes them.
fn composite_limits(built: &[Built]) -> (u16, u16, u16) {
    fn limits(built: &[Built], i: usize, depth: usize, memo: &mut BTreeMap<usize, (u16, u16, u16)>) -> (u16, u16, u16) {
        if let Some(l) = memo.get(&i) {
            return *l;
        }
        let Some(b) = built.get(i) else { return (0, 0, 0) };
        let l = match &b.glyph {
            TtGlyph::Composite(c) if depth <= fontcraft_model::MAX_COMPONENT_DEPTH => c.components().iter().fold((0u16, 0u16, 0u16), |acc, comp| {
                let child = limits(built, usize::from(comp.glyph.to_u16()), depth + 1, memo);
                (acc.0.saturating_add(child.0), acc.1.saturating_add(child.1), acc.2.max(child.2.saturating_add(1)))
            }),
            _ => (b.points, b.contours, 0),
        };
        memo.insert(i, l);
        l
    }
    let mut memo = BTreeMap::new();
    let mut max = (0u16, 0u16, 0u16);
    for (i, b) in built.iter().enumerate() {
        if matches!(b.glyph, TtGlyph::Composite(_)) {
            let l = limits(built, i, 0, &mut memo);
            max = (max.0.max(l.0), max.1.max(l.1), max.2.max(l.2));
        }
    }
    max
}

/// Builds every table and the font file.
pub fn assemble(
    font: &Resolved,
    order: &[ResolvedGlyph],
    outlines: &[Prepared],
    options: &Options,
    warnings: &mut Vec<String>,
) -> Result<Vec<u8>, CompileError> {
    let built = build_glyphs(order, outlines)?;

    // Horizontal metrics and the font box, from the glyphs as written.
    let mut metrics = Vec::with_capacity(order.len());
    let mut bounds: Option<Bbox> = None;
    let (mut max_points, mut max_contours, mut max_elements) = (0u16, 0u16, 0u16);
    let (mut min_lsb, mut min_rsb, mut max_extent, mut max_advance) = (None::<i16>, None::<i16>, None::<i16>, 0u16);
    for (g, b) in order.iter().zip(&built) {
        let advance = u16_of(g.width);
        if crate::ot_round(g.width) != f64::from(advance) {
            warnings.push(format!("glyph `{}`: the advance width {} does not fit the font and was written as {advance}", g.name, g.width));
        }
        max_advance = max_advance.max(advance);
        max_points = max_points.max(b.points);
        max_contours = max_contours.max(b.contours);
        if let TtGlyph::Composite(c) = &b.glyph {
            max_elements = max_elements.max(u16::try_from(c.components().len()).unwrap_or(u16::MAX));
        }
        let Some(bbox) = b.glyph.bbox() else {
            metrics.push(LongMetric::new(advance, 0));
            continue;
        };
        bounds = Some(bounds.map_or(bbox, |f| f.union(bbox)));
        let rsb = i32::from(advance) - i32::from(bbox.x_max);
        min_lsb = Some(min_lsb.map_or(bbox.x_min, |v| v.min(bbox.x_min)));
        min_rsb = Some(min_rsb.map_or(clamp16(rsb), |v| v.min(clamp16(rsb))));
        max_extent = Some(max_extent.map_or(bbox.x_max, |v| v.max(bbox.x_max)));
        metrics.push(LongMetric::new(advance, bbox.x_min));
    }
    let (composite_points, composite_contours, depth) = composite_limits(&built);
    let mut glyf = GlyfLocaBuilder::new();
    for b in &built {
        glyf.add_glyph(&b.glyph).map_err(table("glyf"))?;
    }
    let (glyf, loca, loca_format) = glyf.build();
    let bounds = bounds.unwrap_or_default();
    let glyph_count = u16::try_from(order.len()).map_err(|_| CompileError::TooManyGlyphs(order.len()))?;

    // cmap: every code point to its glyph id (conflicts were refused in preflight).
    let mut mappings: Vec<(char, GlyphId)> = Vec::new();
    let mut owners: BTreeMap<u32, &str> = BTreeMap::new();
    for (gid, g) in order.iter().enumerate() {
        for &cp in &g.unicodes {
            let Some(ch) = char::from_u32(cp) else { continue };
            if owners.insert(cp, &g.name).is_none() {
                mappings.push((ch, GlyphId::new(u32::try_from(gid).unwrap_or(0))));
            }
        }
    }
    let (first_char, last_char) = (owners.keys().next().copied().unwrap_or(0), owners.keys().next_back().copied().unwrap_or(0));
    let has = |range: std::ops::RangeInclusive<u32>| owners.range(range).next().is_some();
    let cmap = Cmap::from_mappings(mappings).map_err(|e| CompileError::Table { table: "cmap", message: e.to_string() })?;

    let style = if options.style_name.is_empty() { font.style_name.as_str() } else { options.style_name.as_str() };
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

    // A font may lack a metric definition; fall back to the usual proportions of the em.
    let upm = f64::from(font.units_per_em);
    let m_ascender = font.metrics.ascender.unwrap_or(upm * 0.8);
    let m_descender = font.metrics.descender.unwrap_or(-upm * 0.2);
    let m_x_height = font.metrics.x_height.unwrap_or(upm * 0.5);
    let m_cap_height = font.metrics.cap_height.unwrap_or(upm * 0.7);
    let (ascender, descender) = (i16_of(m_ascender), i16_of(m_descender));
    let line_gap = i16_of((upm * 1.2 - (m_ascender - m_descender)).max(0.0));
    let time = LongDateTime::new(options.timestamp.saturating_add(HEAD_EPOCH_OFFSET));
    let head = Head {
        font_revision: Fixed::from_f64(f64::from(font.version_major) + f64::from(font.version_minor) / 1000.0),
        // Baseline at y = 0 and left sidebearing at x = 0 (lsb equals the glyph's xMin above).
        flags: Flags::BASELINE_AT_Y_0 | Flags::LSB_AT_X_0,
        units_per_em: font.units_per_em,
        created: time,
        modified: time,
        x_min: bounds.x_min,
        y_min: bounds.y_min,
        x_max: bounds.x_max,
        y_max: bounds.y_max,
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
        min_left_side_bearing: FWord::new(min_lsb.unwrap_or(0)),
        min_right_side_bearing: FWord::new(min_rsb.unwrap_or(0)),
        x_max_extent: FWord::new(max_extent.unwrap_or(0)),
        caret_slope_rise: 1,
        caret_slope_run: 0,
        caret_offset: 0,
        number_of_h_metrics: glyph_count,
    };
    let maxp = Maxp {
        num_glyphs: glyph_count,
        max_points: Some(max_points),
        max_contours: Some(max_contours),
        max_composite_points: Some(composite_points),
        max_composite_contours: Some(composite_contours),
        max_zones: Some(1),
        max_twilight_points: Some(0),
        max_storage: Some(0),
        max_function_defs: Some(0),
        max_instruction_defs: Some(0),
        max_stack_elements: Some(0),
        max_size_of_instructions: Some(0),
        max_component_elements: Some(max_elements),
        max_component_depth: Some(depth),
    };

    let nonzero: Vec<f64> = metrics.iter().map(|m| f64::from(m.advance)).filter(|a| *a > 0.0).collect();
    let average = if nonzero.is_empty() { 0.0 } else { nonzero.iter().sum::<f64>() / nonzero.len() as f64 };
    // Only the two ranges this compiler can vouch for so far; the full range and code-page tables
    // are M2.2.
    let basic_latin = u32::from(has(0x20..=0x7E));
    let latin_1 = u32::from(has(0xA0..=0xFF)) << 1;
    let os2 = Os2 {
        x_avg_char_width: i16_of(average),
        us_weight_class: if bold { 700 } else { 400 },
        us_width_class: 5,
        y_subscript_x_size: i16_of(upm * 0.65),
        y_subscript_y_size: i16_of(upm * 0.6),
        y_subscript_y_offset: i16_of(upm * 0.075),
        y_superscript_x_size: i16_of(upm * 0.65),
        y_superscript_y_size: i16_of(upm * 0.6),
        y_superscript_y_offset: i16_of(upm * 0.35),
        y_strikeout_size: i16_of(upm * 0.05),
        y_strikeout_position: i16_of(m_x_height * 0.6),
        ul_unicode_range_1: basic_latin | latin_1,
        ach_vend_id: Tag::new(b"NONE"),
        fs_selection: selection,
        us_first_char_index: u16::try_from(first_char).unwrap_or(u16::MAX),
        us_last_char_index: u16::try_from(last_char).unwrap_or(u16::MAX),
        s_typo_ascender: ascender,
        s_typo_descender: descender,
        s_typo_line_gap: line_gap,
        us_win_ascent: u16_of(m_ascender.max(f64::from(bounds.y_max))),
        us_win_descent: u16_of((-m_descender).max(-f64::from(bounds.y_min))),
        ul_code_page_range_1: Some(basic_latin), // bit 0: Latin 1
        ul_code_page_range_2: Some(0),
        sx_height: Some(i16_of(m_x_height)),
        s_cap_height: Some(i16_of(m_cap_height)),
        us_default_char: Some(0),
        us_break_char: Some(if owners.contains_key(&0x20) { 0x20 } else { 0 }),
        us_max_context: Some(0),
        ..Default::default()
    };

    // Glyph names: written when every name is a conventional one and there are few enough.
    let post_names: Vec<String> = order
        .iter()
        .enumerate()
        .map(|(gid, g)| {
            if is_post_name(&g.name) {
                g.name.clone()
            } else {
                warnings.push(format!(
                    "glyph `{}` is written to the font as `glyph{gid:05}`: its name is not a valid glyph name there",
                    g.name.escape_debug()
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
    post.underline_position = FWord::new(i16_of(-upm * 0.1));
    post.underline_thickness = FWord::new(i16_of(upm * 0.05));

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
    Ok(builder.build())
}

fn clamp16(v: i32) -> i16 {
    i16::try_from(v.clamp(i32::from(i16::MIN), i32::from(i16::MAX))).unwrap_or(0)
}
