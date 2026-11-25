//! Per-table builders and assembly (`docs/spec/40-compile.md` §§ 1.5, 1.8).
//!
//! The builders read only the resolved font, what [`crate::info`] derives from it, the final
//! glyph order and the prepared outlines. `glyf`/`loca` come first: composite bounds, side
//! bearings, `hhea` and `vhea` extents, `maxp` limits and the `head` box all follow from the
//! written glyphs.
//!
//! Every field is derived as fontc 1.0.0 derives it (`fontbe/src`, Apache-2.0, see `NOTICE`):
//! composite bounds, `USE_MY_METRICS` and component flags from `glyphs.rs`; `hmtx`, `hhea`,
//! `maxp` and the `head` box from `metrics_and_limits.rs`; `vhea` and `vmtx` from
//! `vertical_metrics.rs`; `head` from `head.rs`; `OS/2` from `os2.rs`; `post` from `post.rs`.
//! Where FontCraft writes something else, `docs/spec/40-compile.md` › Where FontCraft differs
//! from fontc says what and why.
//!
//! A font with CFF outlines (which fontc does not write) follows ufo2ft's `OutlineOTFCompiler`
//! (MIT, see `NOTICE`) and what fontTools recomputes when it saves one: `CFF ` in place of
//! `glyf`/`loca`, `maxp` 0.5, `post` 3.0, glyph boxes from the charstrings rounded half up for
//! `hmtx` and `vmtx`, rounded outwards for the `hhea` and `vhea` extents, and the `head` box from
//! the CFF `FontBBox`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

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
use write_fonts::tables::vhea::Vhea;
use write_fonts::tables::vmtx::Vmtx;
use write_fonts::types::{F2Dot14, FWord, Fixed, GlyphId, GlyphId16, LongDateTime, NameId, Tag, UfWord, Version16Dot16};

use crate::cff;
use crate::info::{self, Info};
use crate::outline::{Flavour, Prepared};
use crate::{CompileError, MAX_POST_NAMES, MAX_TT_POINTS, Options, i16_of, ot_round, ranges, u16_of};

/// Seconds between 1904-01-01 (the `head` epoch) and 1970-01-01 (the Unix epoch).
const HEAD_EPOCH_OFFSET: i64 = 2_082_844_800;

fn table<E: std::fmt::Display>(table: &'static str) -> impl Fn(E) -> CompileError {
    move |e| CompileError::Table { table, message: e.to_string() }
}

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
fn build_glyphs(order: &[ResolvedGlyph], outlines: &[Prepared], use_my_metrics: bool) -> Result<Vec<Built>, CompileError> {
    let gids: HashMap<&str, u16> = order.iter().enumerate().filter_map(|(i, g)| Some((g.name.as_str(), u16::try_from(i).ok()?))).collect();
    let widths: HashMap<&str, f64> = order.iter().map(|g| (g.name.as_str(), g.width)).collect();
    let mut built = Vec::with_capacity(order.len());
    for (g, prepared) in order.iter().zip(outlines) {
        let glyph_error = |message: String| CompileError::Glyph { glyph: g.name.clone(), message };
        let entry = match prepared {
            Prepared::Empty => Built { glyph: TtGlyph::Empty, points: 0, contours: 0 },
            Prepared::Simple(_) | Prepared::Points(_) => {
                let simple = match prepared {
                    Prepared::Points(simple) => simple.clone(),
                    Prepared::Simple(path) => SimpleGlyph::from_bezpath(path).map_err(|e| glyph_error(format!("{e:?}")))?,
                    _ => SimpleGlyph::default(),
                };
                let point_count: usize = simple.contours.iter().map(|c| c.len()).sum();
                if point_count > MAX_TT_POINTS || simple.contours.len() > MAX_TT_POINTS {
                    return Err(glyph_error(format!("the outline has {point_count} points; a TrueType glyph holds at most {MAX_TT_POINTS}")));
                }
                let (points, contours) = (u16::try_from(point_count).unwrap_or(u16::MAX), u16::try_from(simple.contours.len()).unwrap_or(u16::MAX));
                Built { glyph: if contours == 0 { TtGlyph::Empty } else { TtGlyph::Simple(simple) }, points, contours }
            }
            Prepared::Composite(components) => {
                let mut parts = Vec::with_capacity(components.len());
                let mut metrics_from_component = false;
                for c in components {
                    let gid = gids.get(c.base.as_str()).ok_or_else(|| glyph_error(format!("the component `{}` is not in the font", c.base)))?;
                    let [a, b, cc, d, e, f] = c.transform.as_coeffs();
                    // `USE_MY_METRICS` on the first component that has the glyph's advance and is
                    // not moved horizontally, scaled or turned (`compile.useMyMetrics`).
                    let use_my_metrics = use_my_metrics
                        && !metrics_from_component
                        && widths.get(c.base.as_str()).is_some_and(|w| reuses_metrics(g.width, *w, &c.transform));
                    metrics_from_component |= use_my_metrics;
                    let component = Component::new(
                        GlyphId16::new(*gid),
                        Anchor::Offset { x: i16_of(e), y: i16_of(f) },
                        Transform { xx: F2Dot14::from_f64(a), yx: F2Dot14::from_f64(b), xy: F2Dot14::from_f64(cc), yy: F2Dot14::from_f64(d) },
                        // ROUND_XY_TO_GRID always, as fontc and ufo2ft write it (`compile.componentFlags`).
                        ComponentFlags { round_xy_to_grid: true, use_my_metrics, ..Default::default() },
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

/// Whether a composite of advance `width` can take its metrics from a component of advance
/// `component_width` placed by `transform`: the same advance once rounded, and a transform that
/// is the identity but for a vertical offset and a horizontal one that rounds to zero (fontc's
/// `can_reuse_metrics`, after ufo2ft's instruction compiler).
fn reuses_metrics(width: f64, component_width: f64, transform: &Affine) -> bool {
    if u16_of(width) != u16_of(component_width) {
        return false;
    }
    let [a, b, c, d, e, _] = transform.as_coeffs();
    [a, b, c, d, ot_round(e)] == [1.0, 0.0, 0.0, 1.0, 0.0]
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

/// Advances and side bearings with their summaries, for `hhea`/`hmtx` or `vhea`/`vmtx`
/// (fontc's `MetricsBuilder`).
#[derive(Default)]
struct MetricsBuilder {
    metrics: Vec<LongMetric>,
    advance_max: u16,
    min_first: Option<i16>,
    min_second: Option<i16>,
    max_extent: Option<i16>,
}

impl MetricsBuilder {
    /// One glyph: its advance, its first side bearing and, when it has an outline, the extent
    /// of the outline along the advance.
    fn update(&mut self, advance: u16, side_bearing: i16, bounds: Option<i32>) {
        self.metrics.push(LongMetric::new(advance, side_bearing));
        self.advance_max = self.advance_max.max(advance);
        if let Some(bounds) = bounds {
            self.min_first = Some(self.min_first.map_or(side_bearing, |v| v.min(side_bearing)));
            let second = clamp16(i32::from(advance) - i32::from(side_bearing) - bounds);
            self.min_second = Some(self.min_second.map_or(second, |v| v.min(second)));
            let extent = clamp16(i32::from(side_bearing) + bounds);
            self.max_extent = Some(self.max_extent.map_or(extent, |v| v.max(extent)));
        }
    }

    /// The long metrics and the side bearings of the run of equal advances at the end, which
    /// need no advance of their own.
    fn split(&self) -> (Vec<LongMetric>, Vec<i16>) {
        let mut long = self.metrics.clone();
        let last = long.last().map(|m| m.advance);
        let run = long.iter().rev().take_while(|m| Some(m.advance) == last).count();
        let short = long.split_off(long.len() - run.saturating_sub(1));
        (long, short.into_iter().map(|m| m.side_bearing).collect())
    }
}

/// What the outline tables tell the metrics tables.
struct Boxes {
    /// Each glyph's integer box, as `hmtx`, `vmtx` and `head` read it; `None` when it draws
    /// nothing.
    glyphs: Vec<Option<Bbox>>,
    /// Each glyph's width and height, for the `hhea` and `vhea` extents.
    extents: Vec<Option<(i32, i32)>>,
    /// The `head` box.
    font: Bbox,
    /// `head.indexToLocFormat`: 1 for long `loca` offsets, else 0.
    loc_format: i16,
}

/// `glyf` and `loca`, with the boxes and `maxp` that follow from them.
/// `composite_programs`: the programs of composite glyphs, by index in the glyph order.
fn truetype(
    order: &[ResolvedGlyph],
    outlines: &[Prepared],
    composite_programs: &[(usize, Vec<u8>)],
    use_my_metrics: bool,
    builder: &mut FontBuilder,
) -> Result<(Boxes, Maxp), CompileError> {
    let mut built = build_glyphs(order, outlines, use_my_metrics)?;
    for (gid, program) in composite_programs {
        if let Some(Built { glyph: TtGlyph::Composite(c), .. }) = built.get_mut(*gid) {
            c.set_instructions(program);
        }
    }
    let (mut max_points, mut max_contours, mut max_elements) = (0u16, 0u16, 0u16);
    let mut boxes = Boxes { glyphs: Vec::with_capacity(built.len()), extents: Vec::with_capacity(built.len()), font: Bbox::default(), loc_format: 0 };
    let mut font_box: Option<Bbox> = None;
    for b in &built {
        max_points = max_points.max(b.points);
        max_contours = max_contours.max(b.contours);
        if let TtGlyph::Composite(c) = &b.glyph {
            max_elements = max_elements.max(u16::try_from(c.components().len()).unwrap_or(u16::MAX));
        }
        let bbox = b.glyph.bbox();
        if let Some(bbox) = bbox {
            font_box = Some(font_box.map_or(bbox, |f| f.union(bbox)));
        }
        boxes.glyphs.push(bbox);
        boxes.extents.push(bbox.map(|b| (i32::from(b.x_max) - i32::from(b.x_min), i32::from(b.y_max) - i32::from(b.y_min))));
    }
    boxes.font = font_box.unwrap_or_default();
    let (composite_points, composite_contours, depth) = composite_limits(&built);
    let mut glyf = GlyfLocaBuilder::new();
    for b in &built {
        glyf.add_glyph(&b.glyph).map_err(table("glyf"))?;
    }
    let (glyf, loca, loca_format) = glyf.build();
    let maxp = Maxp {
        num_glyphs: u16::try_from(order.len()).map_err(|_| CompileError::TooManyGlyphs(order.len()))?,
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
    builder.add_table(&loca).map_err(table("loca"))?;
    builder.add_table(&glyf).map_err(table("glyf"))?;
    boxes.loc_format = match loca_format {
        LocaFormat::Short => 0,
        LocaFormat::Long => 1,
    };
    Ok((boxes, maxp))
}

/// The `CFF ` table, with the boxes and `maxp` (version 0.5) that follow from it. Glyph boxes
/// are the drawn charstrings' bounds rounded half up, the extents rounded outwards and the font
/// box is the table's `FontBBox`, as fontTools recomputes `hmtx`, `hhea` and `head` from a CFF
/// table when it writes a font.
fn postscript(input: &CffInput, builder: &mut FontBuilder, warnings: &mut Vec<String>) -> Result<(Boxes, Maxp), CompileError> {
    let built = match input.cid {
        Some(plan) => crate::cid::build(
            plan,
            input.source,
            input.font,
            input.info,
            input.order,
            input.outlines,
            input.synthesised_notdef,
            input.hinting,
            warnings,
        )?,
        None => cff::build(
            input.source,
            input.font,
            input.info,
            input.order,
            input.outlines,
            input.names,
            input.synthesised_notdef,
            input.hinting,
            input.two_decimals,
            input.cff,
            warnings,
        )?,
    };
    let clamp = |v: i64| i16::try_from(v.clamp(i64::from(i16::MIN), i64::from(i16::MAX))).unwrap_or(0);
    let bbox = |[x_min, y_min, x_max, y_max]: [i64; 4]| Bbox { x_min: clamp(x_min), y_min: clamp(y_min), x_max: clamp(x_max), y_max: clamp(y_max) };
    let glyphs: Vec<Option<Bbox>> = if input.two_decimals {
        built.glyph_bounds.iter().map(|b| b.map(|b| bbox(crate::makeotf::metrics::int_box(&b)))).collect()
    } else {
        built.glyph_bounds.iter().map(|b| b.map(|b| bbox(b.round_with_tolerance(cff::ROUND_TOLERANCE)))).collect()
    };
    let font = if input.two_decimals { crate::makeotf::metrics::font_box(&glyphs) } else { bbox(built.font_bbox) };
    let boxes = Boxes {
        glyphs,
        extents: built
            .glyph_bounds
            .iter()
            .map(|b| {
                b.map(|b| {
                    let [x_min, y_min, x_max, y_max] = b.round_out();
                    let span = |a: i64, b: i64| i32::try_from(b.saturating_sub(a)).unwrap_or(i32::MAX);
                    (span(x_min, x_max), span(y_min, y_max))
                })
            })
            .collect(),
        font,
        loc_format: 0,
    };
    let maxp =
        Maxp { num_glyphs: u16::try_from(input.order.len()).map_err(|_| CompileError::TooManyGlyphs(input.order.len()))?, ..Default::default() };
    let table = if input.cff.optimize_widths { widths_optimized(built.table, warnings) } else { built.table };
    let table = if input.subroutinize { subroutinized(table, false, input.cff.max_subrs, warnings) } else { table };
    builder.add_raw(Tag::new(b"CFF "), table);
    Ok((boxes, maxp))
}

/// The source's Disable Subroutines parameter is on.
fn subroutines_disabled(font: &Resolved) -> bool {
    font.params.get(DISABLE_SUBROUTINES).and_then(fontcraft_model::param_flag).unwrap_or(false)
}

/// The custom parameter that writes `CFF ` charstrings without subroutines.
const DISABLE_SUBROUTINES: &str = "Disable Subroutines";

/// `table` (a `CFF ` table, or `CFF2` when `cff2`) with its charstrings subroutinised
/// ([`fontcraft_cff::subr::subroutinize_table`]), or `table` itself when that is not smaller. A
/// table the subroutiniser refuses (too many tokens, a self-check that failed) is kept as it is,
/// with a warning: the export never fails for want of subroutines.
fn subroutinized(table: Vec<u8>, cff2: bool, max_subrs: usize, warnings: &mut Vec<String>) -> Vec<u8> {
    use fontcraft_cff::subr::{SubrOptions, subroutinize_table};
    match subroutinize_table(&table, cff2, &SubrOptions { max_subrs, ..SubrOptions::default() }) {
        Ok(out) if out.len() < table.len() => out,
        Ok(_) => table,
        Err(e) => {
            let name = if cff2 { "CFF2" } else { "CFF" };
            warnings.push(format!("the {name} table was written without subroutines: {e}"));
            table
        }
    }
}

/// `table` (a `CFF ` table) with the `defaultWidthX` and `nominalWidthX` that make its widths
/// smallest ([`fontcraft_cff::subr::optimize_widths_table`]), or `table` itself, with a warning,
/// when that fails.
fn widths_optimized(table: Vec<u8>, warnings: &mut Vec<String>) -> Vec<u8> {
    match fontcraft_cff::subr::optimize_widths_table(&table) {
        Ok((out, _)) => out,
        Err(e) => {
            warnings.push(format!("the CFF widths were written as they are: {e}"));
            table
        }
    }
}

/// The `CFF2` table of a variable font, subroutinised as [`subroutinized`] does when
/// `subroutinize`, with the boxes and `maxp` (version 0.5) of its outlines at the origin, rounded
/// as [`postscript`] rounds them.
fn cff2_table(
    built: &fontcraft_cff::cff2::Cff2Built,
    glyphs: usize,
    subroutinize: bool,
    builder: &mut FontBuilder,
    warnings: &mut Vec<String>,
) -> Result<(Boxes, Maxp), CompileError> {
    let clamp = |v: i64| i16::try_from(v.clamp(i64::from(i16::MIN), i64::from(i16::MAX))).unwrap_or(0);
    let bbox = |[x_min, y_min, x_max, y_max]: [i64; 4]| Bbox { x_min: clamp(x_min), y_min: clamp(y_min), x_max: clamp(x_max), y_max: clamp(y_max) };
    let union = built.glyph_bounds.iter().flatten().fold(None, |acc: Option<fontcraft_cff::Bounds>, b| {
        Some(match acc {
            None => *b,
            Some(a) => fontcraft_cff::Bounds {
                x_min: a.x_min.min(b.x_min),
                y_min: a.y_min.min(b.y_min),
                x_max: a.x_max.max(b.x_max),
                y_max: a.y_max.max(b.y_max),
            },
        })
    });
    let boxes = Boxes {
        glyphs: built.glyph_bounds.iter().map(|b| b.map(|b| bbox(b.round_with_tolerance(cff::ROUND_TOLERANCE)))).collect(),
        extents: built
            .glyph_bounds
            .iter()
            .map(|b| {
                b.map(|b| {
                    let [x_min, y_min, x_max, y_max] = b.round_out();
                    let span = |a: i64, b: i64| i32::try_from(b.saturating_sub(a)).unwrap_or(i32::MAX);
                    (span(x_min, x_max), span(y_min, y_max))
                })
            })
            .collect(),
        font: union.map_or(Bbox::default(), |u| bbox(u.round_out())),
        loc_format: 0,
    };
    let maxp = Maxp { num_glyphs: u16::try_from(glyphs).map_err(|_| CompileError::TooManyGlyphs(glyphs))?, ..Default::default() };
    let table =
        if subroutinize { subroutinized(built.table.clone(), true, fontcraft_cff::subr::DEFAULT_MAX_SUBRS, warnings) } else { built.table.clone() };
    builder.add_raw(Tag::new(b"CFF2"), table);
    Ok((boxes, maxp))
}

/// What [`postscript`] reads.
struct CffInput<'a> {
    source: Option<&'a fontcraft_model::Font>,
    font: &'a Resolved,
    info: &'a Info,
    order: &'a [ResolvedGlyph],
    outlines: &'a [Prepared],
    names: &'a [String],
    synthesised_notdef: bool,
    hinting: &'a crate::HintingOptions,
    /// Subroutinise the table ([`crate::Options::subroutinize`], resolved).
    subroutinize: bool,
    /// Write a CID-keyed table ([`crate::cid`]).
    cid: Option<&'a crate::cid::CidPlan>,
    /// Coordinates to two decimals and the reference's glyph boxes
    /// ([`crate::makeotf::Finish::two_decimals`]).
    two_decimals: bool,
    cff: &'a crate::CffOptions,
}

/// The `CPAL` table, its labels given name IDs above every font-specific ID in `names`, to
/// which their records are added.
fn cpal_with_labels(
    pending: &crate::color::PendingCpal,
    names: &mut Vec<info::NameRecord>,
    warnings: &mut Vec<String>,
) -> Result<Vec<u8>, CompileError> {
    let first_font_id = crate::var::names::FIRST_FONT_NAME_ID;
    let first = match names.iter().map(|r| r.id).filter(|id| *id >= first_font_id).max() {
        Some(highest) => highest.saturating_add(1),
        None => first_font_id,
    };
    let built = crate::color::cpal::build(&pending.plan, &pending.added, first, warnings)?;
    if !built.names.is_empty() {
        names.extend(built.names.into_iter().map(|(id, text)| info::NameRecord { platform: 3, encoding: 1, language: 0x409, id, text }));
        names.sort_by_key(|r| (r.platform, r.encoding, r.language, r.id));
    }
    Ok(built.table)
}

/// Builds every table and the font file. `synthesised_notdef`: glyph 0 is the compiler's own
/// `.notdef`.
#[allow(clippy::too_many_arguments)]
pub fn assemble(
    source: Option<&fontcraft_model::Font>,
    font: &Resolved,
    order: &[ResolvedGlyph],
    outlines: &[Prepared],
    flavour: Flavour,
    synthesised_notdef: bool,
    layout: &crate::layout::LayoutTables,
    kern: Option<&[u8]>,
    color: &[(Tag, Vec<u8>)],
    extra: &crate::Extra,
    options: &Options,
    warnings: &mut Vec<String>,
) -> Result<Vec<u8>, CompileError> {
    let mut info = info::derive(font, &options.style_name, warnings);
    info::add_names(&mut info, &extra.names);
    let blocks = &layout.compiled;
    let generated_version = info.name(5);
    let generated_unique = info.name(3);
    if let Some(code_names) = &blocks.name {
        // A variable font's names are allocated after the layout stage, so the code's IDs may
        // meet them; those names keep their IDs.
        let protected: Vec<u16> = extra.names.iter().map(|(id, _)| *id).filter(|id| *id >= crate::var::names::FIRST_FONT_NAME_ID).collect();
        crate::layout_tables::merge_names(&mut info.names, code_names, &protected, warnings);
    }
    // A project compile's version string follows the feature code's FontRevision, unless the
    // code names the version itself.
    if let (true, Some(revision)) = (options.project.is_some(), blocks.head.font_revision)
        && info.name(5) == generated_version
    {
        let text = crate::makeotf::metrics::version_string(revision);
        info.names.iter_mut().filter(|r| r.id == 5).for_each(|r| r.text.clone_from(&text));
    }
    // A project compile's licence code ends a unique identifier the feature code does not set.
    if let Some(code) = options.project.as_ref().and_then(|p| p.licence.as_deref())
        && info.name(3) == generated_unique
    {
        info.names.iter_mut().filter(|r| r.id == 3).for_each(|r| r.text = format!("{};{code}", r.text));
    }
    if options.version_stamp {
        info::stamp_version(&mut info, font);
    }
    let cpal = match &extra.cpal {
        Some(pending) => Some(cpal_with_labels(pending, &mut info.names, warnings)?),
        None => None,
    };
    let m = info.metrics;
    let mut builder = FontBuilder::new();
    // Glyph names: in `post` for TrueType, unless there are too many; in the charset for CFF.
    let names = if flavour == Flavour::Cff || order.len() <= MAX_POST_NAMES { crate::names::final_names(font, order, warnings) } else { Vec::new() };
    let variable = extra.tables.iter().any(|(tag, _)| *tag == Tag::new(b"gvar"));
    let tt_hinting = match (flavour, source) {
        (Flavour::TrueType, Some(source)) => {
            crate::tt_hints::tt_hinting(source, font, order, outlines, options, (m.win_ascent, m.win_descent), variable, warnings)?
        }
        _ => None,
    };
    let cvar = match (&tt_hinting, source, &extra.cvar) {
        (Some(h), Some(source), Some(masters)) => {
            crate::var::cvar::font_cvar(source, masters, &h.cvt, options, (m.win_ascent, m.win_descent), warnings)
        }
        _ => None,
    };
    if let Some(suffix) = tt_hinting.as_ref().and_then(|h| h.version_suffix.as_deref()) {
        for r in info.names.iter_mut().filter(|r| r.id == 5) {
            if r.text.chars().count() + suffix.chars().count() <= info::MAX_NAME_CHARS {
                r.text.push_str(suffix);
            }
        }
    }
    // Glyphs with a program are written from the points their program was compiled against.
    let instructed: Option<Vec<Prepared>> = tt_hinting.as_ref().filter(|h| !h.glyphs.is_empty()).map(|h| {
        let mut all = outlines.to_vec();
        for (gid, glyph) in &h.glyphs {
            if let Some(slot) = all.get_mut(*gid) {
                *slot = Prepared::Points(glyph.clone());
            }
        }
        all
    });
    let outlines = instructed.as_deref().unwrap_or(outlines);
    let (boxes, mut maxp) = match flavour {
        Flavour::TrueType => {
            truetype(order, outlines, tt_hinting.as_ref().map_or(&[][..], |h| h.composites.as_slice()), options.use_my_metrics, &mut builder)?
        }
        Flavour::Cff => match &extra.cff2 {
            Some(built) => {
                let subroutinize = options.subroutinize.unwrap_or_else(|| !subroutines_disabled(font));
                cff2_table(built, order.len(), subroutinize, &mut builder, warnings)?
            }
            None => {
                let subroutinize = options.subroutinize.unwrap_or_else(|| !subroutines_disabled(font));
                let input = CffInput {
                    source,
                    font,
                    info: &info,
                    order,
                    outlines,
                    names: &names,
                    synthesised_notdef,
                    hinting: &options.hinting,
                    subroutinize,
                    cid: extra.cid.as_ref(),
                    two_decimals: options.project.as_ref().is_some_and(|p| p.two_decimals),
                    cff: &options.cff,
                };
                postscript(&input, &mut builder, warnings)?
            }
        },
    };

    if let Some(source) = options.project.as_ref().and_then(|p| p.metrics.as_ref()) {
        crate::makeotf::metrics::apply(&mut info, source, order, &boxes.glyphs, boxes.font, font.units_per_em, extra.cid.is_some());
    }
    let m = info.metrics;

    // Horizontal metrics, from the glyphs as written.
    let mut horizontal = MetricsBuilder::default();
    for (i, g) in order.iter().enumerate() {
        let advance = u16_of(g.width);
        if ot_round(g.width) != f64::from(advance) {
            warnings.push(format!("glyph `{}`: the advance width {} does not fit the font and was written as {advance}", g.name, g.width));
        }
        let bbox = boxes.glyphs.get(i).copied().flatten();
        let extent = boxes.extents.get(i).copied().flatten();
        horizontal.update(advance, bbox.map_or(0, |b| b.x_min), extent.map(|e| e.0));
    }
    let bounds = boxes.font;

    // cmap: every code point to its glyph id (conflicts were refused in preflight).
    let mut mappings: Vec<(char, GlyphId)> = Vec::new();
    let mut code_points: BTreeSet<u32> = BTreeSet::new();
    for (gid, g) in order.iter().enumerate() {
        for &cp in &g.unicodes {
            let Some(ch) = char::from_u32(cp) else { continue };
            if code_points.insert(cp) {
                mappings.push((ch, GlyphId::new(u32::try_from(gid).unwrap_or(0))));
            }
        }
    }
    let cmap = Cmap::from_mappings(mappings).map_err(|e| CompileError::Table { table: "cmap", message: e.to_string() })?;

    let mut mac_style = MacStyle::empty();
    if info.selection.contains(SelectionFlags::BOLD) {
        mac_style |= MacStyle::BOLD;
    }
    if info.selection.contains(SelectionFlags::ITALIC) {
        mac_style |= MacStyle::ITALIC;
    }
    let modified = options.timestamp;
    let created = info.created.unwrap_or(modified);
    let date = |t: i64| LongDateTime::new(t.saturating_add(HEAD_EPOCH_OFFSET));
    // The revision is the major version, a point and the minor version written with at least
    // three digits, read as a number (minor 7 is .007, minor 7000 is .7).
    let revision = format!("{}.{:03}", info.version_major, info.version_minor).parse::<f64>().unwrap_or(0.0);
    let mut head = Head {
        font_revision: Fixed::from_f64(revision),
        // Left side bearings are always the glyphs' xMin here, so bit 1 is always true.
        // A hinted font's programs depend on the size in whole pixels (guess on
        // `hinting.tt.export.generatedPrograms`).
        flags: info.head_flags
            | Flags::LSB_AT_X_0
            | if tt_hinting.is_some() { Flags::INSTRUCTIONS_MAY_DEPEND_ON_POINT_SIZE | Flags::FORCE_INTEGER_PPEM } else { Flags::empty() },
        units_per_em: font.units_per_em,
        created: date(created),
        modified: date(modified),
        x_min: bounds.x_min,
        y_min: bounds.y_min,
        x_max: bounds.x_max,
        y_max: bounds.y_max,
        mac_style,
        lowest_rec_ppem: info.lowest_rec_ppem,
        index_to_loc_format: boxes.loc_format,
        ..Default::default()
    };
    crate::layout_tables::apply_head(&blocks.head, &mut head, warnings);
    let (h_metrics, h_side_bearings) = horizontal.split();
    let mut hhea = Hhea {
        ascender: FWord::new(i16_of(m.hhea_ascender)),
        descender: FWord::new(i16_of(m.hhea_descender)),
        line_gap: FWord::new(i16_of(m.hhea_line_gap)),
        advance_width_max: UfWord::new(horizontal.advance_max),
        min_left_side_bearing: FWord::new(horizontal.min_first.unwrap_or(0)),
        min_right_side_bearing: FWord::new(horizontal.min_second.unwrap_or(0)),
        x_max_extent: FWord::new(horizontal.max_extent.unwrap_or(0)),
        caret_slope_rise: i16_of(m.caret_slope_rise),
        caret_slope_run: i16_of(m.caret_slope_run),
        caret_offset: i16_of(m.caret_offset),
        number_of_h_metrics: u16::try_from(h_metrics.len()).unwrap_or(u16::MAX),
    };
    crate::layout_tables::apply_hhea(&blocks.hhea, &mut hhea);
    let max_context = crate::layout_tables::os2_max_context(blocks)?;
    let mut os2 = os2(&info, &horizontal.metrics, &code_points, max_context);
    crate::layout_tables::apply_os2(&blocks.os2, &mut os2);
    if let Some(source) = options.project.as_ref().and_then(|p| p.metrics.as_ref()) {
        crate::makeotf::metrics::after_overrides(&mut os2, &mut hhea, source, &blocks.os2, &blocks.hhea);
    }

    // Glyph names: in `post` 2.0 for TrueType when there are few enough; CFF keeps them in its
    // charset and has `post` 3.0 (`compile.postFormat`).
    // A CFF2 table stores no names, so a CFF2 font keeps them in `post` 2.0 as fontmake does.
    let mut post = if flavour == Flavour::Cff && extra.cff2.is_none() {
        Post { version: Version16Dot16::VERSION_3_0, ..Default::default() }
    } else if order.len() > MAX_POST_NAMES {
        warnings.push(format!("the font has more than {MAX_POST_NAMES} glyphs, so glyph names were left out of it"));
        Post { version: Version16Dot16::VERSION_3_0, ..Default::default() }
    } else {
        Post::new_v2(names.iter().map(String::as_str))
    };
    post.italic_angle = Fixed::from_f64(info.italic_angle);
    post.underline_position = FWord::new(i16_of(m.underline_position));
    post.underline_thickness = FWord::new(i16_of(m.underline_thickness));
    post.is_fixed_pitch = u32::from(info.fixed_pitch);

    let name =
        Name::new(info.names.iter().map(|r| NameRecord::new(r.platform, r.encoding, r.language, NameId::new(r.id), r.text.clone().into())).collect());

    if let Some(h) = &tt_hinting {
        set_program_limits(&mut maxp, h, outlines);
        if !h.cvt.is_empty() {
            builder.add_raw(Tag::new(b"cvt "), h.cvt.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>());
            if let Some(cvar) = &cvar {
                builder.add_raw(Tag::new(b"cvar"), cvar.as_slice());
            }
        }
        if !h.fpgm.is_empty() {
            builder.add_raw(Tag::new(b"fpgm"), h.fpgm.clone());
        }
        if !h.prep.is_empty() {
            builder.add_raw(Tag::new(b"prep"), h.prep.clone());
        }
    }
    builder.add_table(&head).map_err(table("head"))?;
    builder.add_table(&hhea).map_err(table("hhea"))?;
    builder.add_table(&maxp).map_err(table("maxp"))?;
    builder.add_table(&os2).map_err(table("OS/2"))?;
    builder.add_table(&Hmtx::new(h_metrics, h_side_bearings)).map_err(table("hmtx"))?;
    builder.add_table(&cmap).map_err(table("cmap"))?;
    builder.add_table(&name).map_err(table("name"))?;
    builder.add_table(&post).map_err(table("post"))?;
    for (tag, bytes) in layout.tables() {
        builder.add_raw(tag, bytes);
    }
    if let Some(kern) = kern {
        builder.add_raw(Tag::new(b"kern"), kern);
    }
    let hinted = tt_hinting.is_some();
    if let Some(gasp) = crate::gasp::gasp_table(font, flavour, hinted, warnings)? {
        builder.add_table(&gasp).map_err(table("gasp"))?;
    }
    if info.vertical {
        let (mut vhea, vmtx) = vertical(order, &boxes, &info);
        crate::layout_tables::apply_vhea(&blocks.vhea, &mut vhea);
        builder.add_table(&vhea).map_err(table("vhea"))?;
        builder.add_table(&vmtx).map_err(table("vmtx"))?;
        if flavour == Flavour::Cff
            && let Some(vorg) = crate::extras::vorg_table(&order.iter().map(|g| vertical_origin(g, &info)).collect::<Vec<_>>())
        {
            builder.add_raw(Tag::new(b"VORG"), vorg);
        }
    } else if crate::layout_tables::sets_vhea(&blocks.vhea) {
        warnings.push("feature code: the vhea table block was left out: the font has no vertical metrics".into());
    }
    if let Some(meta) = crate::extras::meta_table(&font.params, warnings)? {
        builder.add_raw(Tag::new(b"meta"), meta);
    }
    if let Some(plan) = &extra.math {
        builder.add_raw(Tag::new(b"MATH"), crate::math::build(plan, order, &boxes.glyphs, warnings)?);
    }
    for (tag, bytes) in color.iter().chain(&extra.tables) {
        // The code's `STAT` replaces a variable font's own.
        if blocks.stat.is_some() && *tag == Tag::new(b"STAT") {
            continue;
        }
        builder.add_raw(*tag, bytes.as_slice());
    }
    if let Some(cpal) = &cpal {
        builder.add_raw(Tag::new(b"CPAL"), cpal.as_slice());
    }
    if let Some(ttfa) = tt_hinting.as_ref().and_then(|h| h.ttfa.as_ref()) {
        builder.add_raw(Tag::new(b"TTFA"), ttfa.as_slice());
    }
    Ok(builder.build())
}

/// `maxp`'s program values: the merged needs of the TrueType programs, and the longest glyph
/// program.
fn set_program_limits(maxp: &mut Maxp, h: &crate::tt_hints::TtHinting, outlines: &[Prepared]) {
    let l = &h.limits;
    let longest = outlines
        .iter()
        .filter_map(|o| match o {
            Prepared::Points(g) => u16::try_from(g.instructions.len()).ok(),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    maxp.max_zones = Some(l.max_zones.max(1));
    maxp.max_twilight_points = Some(l.max_twilight_points);
    maxp.max_storage = Some(l.max_storage);
    maxp.max_function_defs = Some(l.max_function_defs);
    maxp.max_instruction_defs = Some(l.max_instruction_defs);
    maxp.max_stack_elements = Some(l.max_stack_elements);
    maxp.max_size_of_instructions = Some(longest.max(l.max_size_of_instructions));
}

/// `OS/2` version 4 (`compile.os2*`); version 5 when the feature code sets an optical size.
fn os2(info: &Info, metrics: &[LongMetric], code_points: &BTreeSet<u32>, max_context: u16) -> Os2 {
    let m = &info.metrics;
    // The mean of the advances that are not zero, in single precision as fontc computes it.
    let (count, total) = metrics.iter().filter(|l| l.advance > 0).fold((0u64, 0u64), |(c, t), l| (c + 1, t + u64::from(l.advance)));
    #[allow(clippy::cast_precision_loss)]
    let average = if count == 0 { 0.0 } else { (total as f32 / count as f32 + 0.5).floor() };
    let unicode = info.unicode_ranges.clone().unwrap_or_else(|| ranges::unicode_range_bits(code_points));
    let pages = info.code_pages.clone().unwrap_or_else(|| ranges::code_page_bits(code_points));
    let [u1, u2, u3, u4] = ranges::words::<4>(&unicode);
    let [c1, c2] = ranges::words::<2>(&pages);
    let first = code_points.first().copied().unwrap_or(0xFFFF).min(0xFFFF);
    let last = code_points.last().copied().unwrap_or(0).min(0xFFFF);
    Os2 {
        x_avg_char_width: f64::from(average).clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16,
        us_weight_class: info.weight_class,
        us_width_class: info.width_class,
        fs_type: info.fs_type,
        y_subscript_x_size: i16_of(m.subscript_x_size),
        y_subscript_y_size: i16_of(m.subscript_y_size),
        y_subscript_x_offset: i16_of(m.subscript_x_offset),
        y_subscript_y_offset: i16_of(m.subscript_y_offset),
        y_superscript_x_size: i16_of(m.superscript_x_size),
        y_superscript_y_size: i16_of(m.superscript_y_size),
        y_superscript_x_offset: i16_of(m.superscript_x_offset),
        y_superscript_y_offset: i16_of(m.superscript_y_offset),
        y_strikeout_size: i16_of(m.strikeout_size),
        y_strikeout_position: i16_of(m.strikeout_position),
        s_family_class: info.family_class,
        panose_10: info.panose.unwrap_or_default(),
        ul_unicode_range_1: u1,
        ul_unicode_range_2: u2,
        ul_unicode_range_3: u3,
        ul_unicode_range_4: u4,
        ach_vend_id: info.vendor,
        fs_selection: info.selection,
        us_first_char_index: u16::try_from(first).unwrap_or(u16::MAX),
        us_last_char_index: u16::try_from(last).unwrap_or(u16::MAX),
        s_typo_ascender: i16_of(m.typo_ascender),
        s_typo_descender: i16_of(m.typo_descender),
        s_typo_line_gap: i16_of(m.typo_line_gap),
        us_win_ascent: u16_of(m.win_ascent),
        us_win_descent: u16_of(m.win_descent),
        ul_code_page_range_1: Some(c1),
        ul_code_page_range_2: Some(c2),
        sx_height: Some(i16_of(m.x_height)),
        s_cap_height: Some(i16_of(m.cap_height)),
        us_default_char: Some(0),
        // The space, whether the font has one or not, as ufo2ft and fontc write it.
        us_break_char: Some(32),
        // The longest context of the compiled lookups (`compile.os2MaxContext`).
        us_max_context: Some(max_context),
        ..Default::default()
    }
}

/// `vhea` and `vmtx` (`compile.verticalMetrics`): each glyph's vertical advance, its own or the
/// typographic ascender to descender, and its top side bearing from its vertical origin, its
/// own or the typographic ascender. The model measures a vertical origin down from the
/// typographic ascender, for every source format (glyphsLib's rule, which the UFO reader
/// follows).
fn vertical(order: &[ResolvedGlyph], boxes: &Boxes, info: &Info) -> (Vhea, Vmtx) {
    let m = &info.metrics;
    let mut v = MetricsBuilder::default();
    for (i, g) in order.iter().enumerate() {
        let advance = u16_of(g.vert_width.unwrap_or(m.typo_ascender - m.typo_descender));
        let origin = vertical_origin(g, info);
        let bbox = boxes.glyphs.get(i).copied().flatten();
        let top = clamp16(i32::from(origin) - i32::from(bbox.map_or(0, |b| b.y_max)));
        v.update(advance, top, boxes.extents.get(i).copied().flatten().map(|e| e.1));
    }
    let (long, short) = v.split();
    let vhea = Vhea {
        version: Version16Dot16::VERSION_1_1,
        ascender: FWord::new(i16_of(m.vhea_ascender)),
        descender: FWord::new(i16_of(m.vhea_descender)),
        line_gap: FWord::new(i16_of(m.vhea_line_gap)),
        advance_height_max: UfWord::new(v.advance_max),
        min_top_side_bearing: FWord::new(v.min_first.unwrap_or(0)),
        min_bottom_side_bearing: FWord::new(v.min_second.unwrap_or(0)),
        y_max_extent: FWord::new(v.max_extent.unwrap_or(0)),
        caret_slope_rise: i16_of(m.vhea_caret_slope_rise),
        caret_slope_run: i16_of(m.vhea_caret_slope_run),
        caret_offset: i16_of(m.vhea_caret_offset),
        number_of_long_ver_metrics: u16::try_from(long.len()).unwrap_or(u16::MAX),
    };
    (vhea, Vmtx::new(long, short))
}

/// The y of `g`'s vertical origin: its own, measured down from the typographic ascender, or the
/// ascender.
fn vertical_origin(g: &ResolvedGlyph, info: &Info) -> i16 {
    let m = &info.metrics;
    i16_of(g.vert_origin.map_or(m.typo_ascender, |o| m.typo_ascender - o))
}

fn clamp16(v: i32) -> i16 {
    i16::try_from(v.clamp(i32::from(i16::MIN), i32::from(i16::MAX))).unwrap_or(0)
}
