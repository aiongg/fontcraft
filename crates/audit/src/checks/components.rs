//! Component checks (M12.46). Runners take the context of their scope ([`crate::GlyphCtx`],
//! [`crate::SourceCtx`], [`crate::BinaryCtx`], [`crate::FamilyCtx`]), the thresholds and a
//! [`crate::Sink`]; their registry entries go in `registry/`.
//!
//! The source checks read one layer and look its component bases up in the font, on the master
//! layer of the same master, as [`fontcraft_model::Font::outline`] draws them. Checks about what a
//! compiled font stores (`nested`, `transformed`, `overlap`, `mixedWithOutlines`,
//! `nonExportingBase`) only look at glyphs that export.
//!
//! The rules of `nested`, `transformed`, `duplicate`, `caronAlternate` and `fewComposites` follow
//! FontBakery's checks of the same purpose (Apache-2.0); `overlap` follows the component test of
//! fontTools' `ttLib.removeOverlaps` (MIT). No code is copied.

use std::collections::{HashSet, VecDeque};

use fontcraft_geom::{BezPath, PathEl, Point, Rect, Shape as _};
use fontcraft_model::{Component, Glyph, Layer, Shape};
use read_fonts::tables::glyf::Glyph as TtGlyph;
use read_fonts::tables::loca::LocaGlyph;
use read_fonts::types::GlyphId;
use read_fonts::{FontRef, TableProvider};
use skrifa::{GlyphNameSource, GlyphNames};

use crate::contour::{Bez, dist_to_segment, overlaps, proper_cross};
use crate::contours::MAX_OVERLAP_TESTS;
use crate::{BinaryCtx, Hit, LayerInput, Location, MAX_LAYER_NODES, Nth, Params, Severity, Sink, Threshold, num};

/// Most glyph layers the search for a component cycle visits from one layer.
pub(crate) const MAX_CHAIN_VISITS: usize = 10_000;
/// Longest component chain followed; a longer one is reported, as the outline code refuses it.
pub(crate) const MAX_CHAIN_DEPTH: usize = fontcraft_model::MAX_COMPONENT_DEPTH;
/// Transform coefficients this close to the identity's are the identity: half a step of the
/// 2.14 fixed-point numbers a compiled `glyf` stores them in.
pub(crate) const TRANSFORM_TOLERANCE: f64 = 0.5 / 16384.0;
/// A sample point closer than this (in units) to the other outline is on its edge and decides
/// nothing about containment.
const EDGE_MARGIN: f64 = 0.5;

pub(crate) const MAX_OUTLINE_SHARE: Threshold = Threshold {
    name: "maxOutlinePercent",
    default: 50.0,
    min: 0.0,
    max: 100.0,
    doc: "Highest share, in percent of all glyphs, of glyphs stored as outlines rather than composites. Glyphs named with one ASCII character do not count as outlines.",
};

/// The components of a layer with their indices in its shape list.
pub(crate) fn components(layer: &Layer) -> impl Iterator<Item = (usize, &Component)> {
    layer.shapes.iter().enumerate().filter_map(|(i, s)| match s {
        Shape::Component(c) => Some((i, c)),
        _ => None,
    })
}

fn has_paths(layer: &Layer) -> bool {
    layer.shapes.iter().any(|s| matches!(s, Shape::Path(p) if !p.nodes.is_empty()))
}

fn has_components(layer: &Layer) -> bool {
    layer.shapes.iter().any(|s| matches!(s, Shape::Component(_)))
}

/// A finding about the component at shape index `shape`.
fn at_component(code: &'static str, message: String, shape: usize) -> Hit {
    Hit::new(code, message).shapes(vec![shape]).place(Location { components: vec![shape], ..Location::default() })
}

/// A component's transform as `[xx, yx, xy, yy, dx, dy]`; `None` when a number is not finite.
fn matrix(c: &Component) -> Option<[f64; 6]> {
    let m = c.affine().as_coeffs();
    m.iter().all(|v| v.is_finite()).then_some(m)
}

fn is_identity_2x2(m: &[f64; 6]) -> bool {
    let [xx, yx, xy, yy, _, _] = *m;
    (xx - 1.0).abs() <= TRANSFORM_TOLERANCE
        && yx.abs() <= TRANSFORM_TOLERANCE
        && xy.abs() <= TRANSFORM_TOLERANCE
        && (yy - 1.0).abs() <= TRANSFORM_TOLERANCE
}

/// The master layer of the glyph a component names, for a layer of `master`.
fn base_layer<'a>(input: &LayerInput<'a>, c: &Component) -> Option<(&'a Glyph, &'a Layer)> {
    let base = input.ctx.font.glyph(&c.glyph)?;
    let layer = base.layer(&input.layer.master)?;
    Some((base.as_ref(), layer))
}

/// A component whose base glyph is made of components only: the compiled glyph refers to a
/// composite. A base that also has paths compiles as outlines (`mixedWithOutlines`), and a base
/// that does not export is decomposed into the glyph (`nonExportingBase`).
pub(crate) fn nested(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    if !input.glyph.export {
        return;
    }
    for (i, c) in components(input.layer) {
        let Some((base, layer)) = base_layer(input, c) else { continue };
        if base.export && has_components(layer) && !has_paths(layer) {
            sink.push(at_component("nested", format!("Shape {} uses {}, which is itself made of components.", Nth(i), c.glyph), i));
        }
    }
}

/// A component that is mirrored (always a problem: it reverses the outline direction) or scaled,
/// rotated or slanted (a problem for hinted TrueType fonts).
pub(crate) fn transformed(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    if !input.glyph.export {
        return;
    }
    for (i, c) in components(input.layer) {
        let Some(m) = matrix(c) else { continue };
        let [xx, yx, xy, yy, _, _] = m;
        let det = xx * yy - xy * yx;
        let hit = if det < 0.0 {
            at_component("mirrored", format!("Shape {} ({}) is mirrored, which reverses its outline direction.", Nth(i), c.glyph), i)
        } else if !is_identity_2x2(&m) {
            at_component("scaled-or-rotated", format!("Shape {} ({}) is scaled, rotated or slanted.", Nth(i), c.glyph), i)
        } else {
            continue;
        };
        sink.push(hit.data("xx", xx).data("yx", yx).data("xy", xy).data("yy", yy));
    }
}

/// The same component twice, at the same rounded offset with the same 2×2 transform. The finding
/// names the later copy, which the fix deletes.
pub(crate) fn duplicate(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    let placed: Vec<(usize, &Component, [f64; 6])> = components(input.layer).filter_map(|(i, c)| Some((i, c, matrix(c)?))).collect();
    for (k, (i, c, m)) in placed.iter().enumerate() {
        let same = |n: &[f64; 6]| {
            m.iter().zip(n.iter()).take(4).all(|(a, b)| (a - b).abs() <= TRANSFORM_TOLERANCE)
                && m.get(4).map(|v| v.round()) == n.get(4).map(|v| v.round())
                && m.get(5).map(|v| v.round()) == n.get(5).map(|v| v.round())
        };
        let Some((first, _, _)) = placed.iter().take(k).find(|(_, d, n)| d.glyph == c.glyph && same(n)) else { continue };
        sink.push(
            at_component("duplicate", format!("Shape {} repeats shape {}: {} at the same place.", Nth(*i), Nth(*first), c.glyph), *i)
                .data("first", *first as f64),
        );
    }
}

/// A component outline flattened into closed polylines.
struct Part {
    shape: usize,
    outline: BezPath,
    contours: Vec<Vec<Point>>,
    bbox: Rect,
}

/// Closed polylines of an outline; each ends where it started.
fn polylines(outline: &BezPath) -> Vec<Vec<Point>> {
    let mut out: Vec<Vec<Point>> = Vec::new();
    let mut current: Vec<Point> = Vec::new();
    let (mut start, mut last) = (None::<Point>, None::<Point>);
    let mut pts = Vec::new();
    let flush = |current: &mut Vec<Point>, out: &mut Vec<Vec<Point>>| {
        if current.len() >= 3 {
            out.push(std::mem::take(current));
        } else {
            current.clear();
        }
    };
    for el in outline.elements() {
        pts.clear();
        match *el {
            PathEl::MoveTo(p) => {
                flush(&mut current, &mut out);
                start = Some(p);
                last = Some(p);
                current.push(p);
            }
            PathEl::LineTo(p) => {
                if let Some(l) = last {
                    Bez::Line(l, p).polyline(&mut pts);
                }
                last = Some(p);
            }
            PathEl::QuadTo(c, p) => {
                if let Some(l) = last {
                    Bez::Quad(l, c, p).polyline(&mut pts);
                }
                last = Some(p);
            }
            PathEl::CurveTo(c1, c2, p) => {
                if let Some(l) = last {
                    Bez::Cubic(l, c1, c2, p).polyline(&mut pts);
                }
                last = Some(p);
            }
            PathEl::ClosePath => {
                if let (Some(l), Some(s)) = (last, start) {
                    Bez::Line(l, s).polyline(&mut pts);
                }
                last = start;
            }
        }
        for p in pts.iter().filter(|p| p.is_finite()) {
            if current.last() != Some(p) {
                current.push(*p);
            }
        }
    }
    flush(&mut current, &mut out);
    for c in &mut out {
        if let (Some(first), Some(last)) = (c.first().copied(), c.last().copied())
            && first != last
        {
            c.push(first);
        }
    }
    out
}

fn edges(contours: &[Vec<Point>]) -> impl Iterator<Item = (Point, Point)> + '_ {
    contours.iter().flat_map(|c| c.windows(2).filter_map(|w| if let [a, b] = w { Some((*a, *b)) } else { None }))
}

enum Contact {
    None,
    At(Point),
    OverBudget,
}

/// Where the filled areas of two component outlines share some area: a proper crossing of their
/// edges, or a contour of one lying inside the other.
fn contact(a: &Part, b: &Part, tests: &mut usize) -> Contact {
    for (p, q) in edges(&a.contours) {
        let ra = Rect::from_points(p, q);
        if !overlaps(ra, b.bbox) {
            continue;
        }
        for (r, s) in edges(&b.contours) {
            *tests += 1;
            if *tests > MAX_OVERLAP_TESTS {
                return Contact::OverBudget;
            }
            if overlaps(ra, Rect::from_points(r, s))
                && let Some(x) = proper_cross(p, q, r, s)
            {
                return Contact::At(x);
            }
        }
    }
    for (inner, outer) in [(a, b), (b, a)] {
        for contour in &inner.contours {
            match inside(contour, outer, tests) {
                Contact::None => {}
                found => return found,
            }
        }
    }
    Contact::None
}

/// Is a contour that crosses no edge of `outer` inside its filled area? The first sample point
/// clearly off `outer`'s edges decides.
fn inside(contour: &[Point], outer: &Part, tests: &mut usize) -> Contact {
    let mids = contour.windows(2).filter_map(|w| if let [a, b] = w { Some(a.midpoint(*b)) } else { None });
    for p in contour.iter().copied().chain(mids) {
        let b = outer.bbox;
        if p.x < b.x0 - EDGE_MARGIN || p.x > b.x1 + EDGE_MARGIN || p.y < b.y0 - EDGE_MARGIN || p.y > b.y1 + EDGE_MARGIN {
            return Contact::None;
        }
        let mut near = false;
        for (a, b) in edges(&outer.contours) {
            *tests += 1;
            if *tests > MAX_OVERLAP_TESTS {
                return Contact::OverBudget;
            }
            if dist_to_segment(p, a, b) <= EDGE_MARGIN {
                near = true;
                break;
            }
        }
        if !near {
            return if outer.outline.winding(p) != 0 { Contact::At(p) } else { Contact::None };
        }
    }
    Contact::None
}

/// Two components whose filled areas overlap.
pub(crate) fn overlap(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    if !input.glyph.export {
        return;
    }
    let mut parts: Vec<Part> = Vec::new();
    let mut budget = MAX_LAYER_NODES;
    for (i, c) in components(input.layer) {
        let Ok(outline) = input.ctx.font.component_outline(c, &input.layer.master) else { continue };
        let count = outline.elements().len();
        if count > budget {
            sink.skip(format!("the components' outlines have more than {MAX_LAYER_NODES} segments"));
            return;
        }
        budget -= count;
        let contours = polylines(&outline);
        let bbox = outline.bounding_box();
        if contours.is_empty() || ![bbox.x0, bbox.y0, bbox.x1, bbox.y1].iter().all(|v| v.is_finite()) {
            continue;
        }
        parts.push(Part { shape: i, outline, contours, bbox });
    }
    let mut tests = 0usize;
    for (k, a) in parts.iter().enumerate() {
        for b in parts.iter().skip(k + 1) {
            if !overlaps(a.bbox, b.bbox) {
                continue;
            }
            match contact(a, b, &mut tests) {
                Contact::None => {}
                Contact::At(x) => {
                    let (i, j) = (a.shape, b.shape);
                    sink.push(
                        Hit::new("components-overlap", format!("Shapes {} and {} (components) overlap.", Nth(i), Nth(j)))
                            .shapes(vec![i, j])
                            .place(Location { components: vec![i, j], ..Location::default() })
                            .at(x),
                    );
                }
                Contact::OverBudget => {
                    sink.skip(format!("the components' outlines need more than {MAX_OVERLAP_TESTS} comparisons"));
                    return;
                }
            }
        }
    }
}

/// A glyph that exports holds both drawn paths and components.
pub(crate) fn mixed_with_outlines(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    if !input.glyph.export || !has_paths(input.layer) {
        return;
    }
    let shapes: Vec<usize> = components(input.layer).map(|(i, _)| i).collect();
    if shapes.is_empty() {
        return;
    }
    let n = shapes.len();
    let message = format!("The layer has paths and {n} component{}; the export decomposes it.", if n == 1 { "" } else { "s" });
    sink.push(
        Hit::new("mixed", message).shapes(shapes.clone()).place(Location { components: shapes, ..Location::default() }).data("components", n as f64),
    );
}

/// A component names a glyph the font does not have.
pub(crate) fn orphaned(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    for (i, c) in components(input.layer) {
        if input.ctx.font.glyph(&c.glyph).is_none() {
            sink.push(at_component("missing-base", format!("Shape {} uses {}, which is not in the font.", Nth(i), c.glyph), i));
        }
    }
}

/// What following a component chain found.
enum Chain {
    Clear,
    Cycle,
    TooDeep,
    OverBudget,
}

/// Follows the components of `start` breadth first on `master`, looking for `target`.
fn chain(input: &LayerInput<'_>, start: &str, target: &str, visits: &mut usize) -> Chain {
    let font = input.ctx.font;
    let master = input.layer.master.as_str();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut queue: VecDeque<(&str, usize)> = VecDeque::from([(start, 1)]);
    while let Some((name, depth)) = queue.pop_front() {
        if name == target {
            return Chain::Cycle;
        }
        if !seen.insert(name) {
            continue;
        }
        if depth > MAX_CHAIN_DEPTH {
            return Chain::TooDeep;
        }
        *visits += 1;
        if *visits > MAX_CHAIN_VISITS {
            return Chain::OverBudget;
        }
        let Some(layer) = font.glyph(name).and_then(|g| g.layer(master)) else { continue };
        for (_, c) in components(layer) {
            queue.push_back((c.glyph.as_str(), depth + 1));
        }
    }
    Chain::Clear
}

/// A component chain that leads back to this glyph, or nests deeper than the outline code
/// follows.
pub(crate) fn circular(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    let mut visits = 0usize;
    for (i, c) in components(input.layer) {
        match chain(input, &c.glyph, &input.glyph.name, &mut visits) {
            Chain::Clear => {}
            Chain::Cycle => {
                sink.push(at_component("cycle", format!("Shape {} uses {}, whose components lead back to {}.", Nth(i), c.glyph, input.glyph.name), i))
            }
            Chain::TooDeep => sink.push(at_component(
                "too-deep",
                format!("Shape {} uses {}, whose components nest more than {MAX_CHAIN_DEPTH} levels deep.", Nth(i), c.glyph),
                i,
            )),
            Chain::OverBudget => {
                sink.skip(format!("the component chains reach more than {MAX_CHAIN_VISITS} glyphs"));
                return;
            }
        }
    }
}

/// A glyph that exports uses a component of a glyph that does not.
pub(crate) fn non_exporting_base(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    if !input.glyph.export {
        return;
    }
    for (i, c) in components(input.layer) {
        if input.ctx.font.glyph(&c.glyph).is_some_and(|g| !g.export) {
            sink.push(at_component("non-exporting-base", format!("Shape {} uses {}, which does not export.", Nth(i), c.glyph), i));
        }
    }
}

/// Does `layer` of `master` draw anything, its components followed to `depth`? A chain too deep
/// or cyclic counts as drawing, so `circular` alone reports it.
fn draws(input: &LayerInput<'_>, layer: &Layer, depth: usize, visits: &mut usize) -> bool {
    if has_paths(layer) {
        return true;
    }
    *visits += 1;
    if depth > MAX_CHAIN_DEPTH || *visits > MAX_CHAIN_VISITS {
        return true;
    }
    components(layer).any(|(_, c)| match input.ctx.font.glyph(&c.glyph).and_then(|g| g.layer(&input.layer.master)) {
        Some(l) => draws(input, l, depth + 1, visits),
        // A missing base is `orphaned`; a missing layer draws nothing.
        None => input.ctx.font.glyph(&c.glyph).is_none(),
    })
}

/// A component whose base glyph draws nothing on this master.
pub(crate) fn empty_base(input: &LayerInput<'_>, _: &Params<'_>, sink: &mut Sink) {
    let mut visits = 0usize;
    for (i, c) in components(input.layer) {
        let Some(base) = input.ctx.font.glyph(&c.glyph) else { continue };
        let empty = match base.layer(&input.layer.master) {
            Some(layer) => !draws(input, layer, 1, &mut visits),
            None => true,
        };
        if empty {
            sink.push(at_component("empty-base", format!("Shape {} uses {}, which has no outline on this master.", Nth(i), c.glyph), i));
        }
    }
}

/// The glyph names of a compiled font by glyph id, read once (reading one name of a `post`
/// table can scan all of them).
pub(crate) struct Names {
    /// Name and whether the font stores it (rather than a made-up `gidN`).
    names: Vec<(String, bool)>,
    /// The font stores no names at all.
    pub(crate) synthesized: bool,
}

impl Names {
    pub(crate) fn new(font: &FontRef<'_>) -> Self {
        let source = GlyphNames::new(font);
        // Glyph ids are 16-bit, so this also bounds a damaged count.
        let count = usize::try_from(source.num_glyphs()).unwrap_or(0).min(usize::from(u16::MAX) + 1);
        let mut names: Vec<(String, bool)> = (0..count).map(|gid| (format!("gid{gid}"), false)).collect();
        for (gid, name) in source.iter().take(count) {
            if let Some(slot) = usize::try_from(gid.to_u32()).ok().and_then(|i| names.get_mut(i)) {
                *slot = (name.as_str().to_string(), !name.is_synthesized());
            }
        }
        Self { names, synthesized: source.source() == GlyphNameSource::Synthesized }
    }

    /// The font's name for `gid`, stored or made up.
    pub(crate) fn get(&self, gid: u32) -> Option<&str> {
        self.names.get(usize::try_from(gid).ok()?).map(|(n, _)| n.as_str())
    }

    /// The name the font stores for `gid`, if any.
    pub(crate) fn stored(&self, gid: u32) -> Option<&str> {
        self.names.get(usize::try_from(gid).ok()?).filter(|(_, stored)| *stored).map(|(n, _)| n.as_str())
    }
}

/// The name findings give glyph `gid` of a compiled font: the source glyph when the font was
/// compiled in this run, otherwise the font's own name for it.
pub(crate) fn binary_glyph_name(ctx: &BinaryCtx<'_>, names: &Names, gid: u32) -> String {
    ctx.source.and_then(|s| s.glyph(gid)).or_else(|| names.get(gid)).map(str::to_string).unwrap_or_else(|| format!("gid{gid}"))
}

/// The glyphs `font`'s character map gives `codepoints`, without repeats.
fn mapped(font: &FontRef<'_>, codepoints: &[u32]) -> Vec<u32> {
    let Ok(cmap) = font.cmap() else { return Vec::new() };
    let mut out: Vec<u32> = Vec::new();
    for gid in codepoints.iter().filter_map(|&u| cmap.map_codepoint(u)).map(|g| g.to_u32()) {
        if !out.contains(&gid) {
            out.push(gid);
        }
    }
    out.sort_unstable();
    out
}

/// Lcaron, dcaron, lcaron, tcaron.
const CARON_LETTERS: [u32; 4] = [0x013D, 0x010F, 0x013E, 0x0165];
/// The caron and the combining caron: the wrong accent for the four letters.
const CARONS: [u32; 2] = [0x02C7, 0x030C];
/// Comma, right single quotation mark, single low-9 quotation mark, apostrophe: look-alikes of
/// the vertical caron that should have a glyph of their own.
const COMMA_LIKE: [u32; 4] = [0x002C, 0x2019, 0x201A, 0x0027];

/// The four letters with a vertical caron, composed with the ordinary caron or a comma-like glyph.
/// Glyph names compare after dropping `.case`, `.uc` and `.sc`.
pub(crate) fn caron_alternate(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let (Ok(glyf), Ok(loca)) = (font.glyf(), font.loca(None)) else { return };
    let names = Names::new(font);
    let name = |gid: u32| names.get(gid).unwrap_or_default().to_string();
    let named = |gids: Vec<u32>| -> Vec<String> { gids.into_iter().map(name).filter(|n| !n.is_empty()).collect() };
    let carons = named(mapped(font, &CARONS));
    let commas = named(mapped(font, &COMMA_LIKE));
    for gid in mapped(font, &CARON_LETTERS) {
        let glyph = binary_glyph_name(ctx, &names, gid);
        let place = || Location { glyph: glyph.clone(), table: Some("glyf".into()), ..Location::default() };
        let parts: Vec<u32> = match loca.get(GlyphId::new(gid), &glyf) {
            Some(LocaGlyph::Glyph(TtGlyph::Composite(c))) => c.component_glyphs_and_flags().map(|(g, _)| g.to_u32()).collect(),
            Some(_) => {
                sink.push(
                    Hit::new("decomposed-outline", format!("{glyph} is stored as an outline, so its caron cannot be checked."))
                        .severity(Severity::Warn)
                        .place(place()),
                );
                continue;
            }
            None => continue,
        };
        if parts.len() == 1 {
            sink.push(
                Hit::new("single-component", format!("{glyph} has a single component, so its caron cannot be checked."))
                    .severity(Severity::Warn)
                    .place(place()),
            );
            continue;
        }
        for part in parts {
            let own = name(part);
            let bare = own.replace(".case", "").replace(".uc", "").replace(".sc", "");
            let shown = binary_glyph_name(ctx, &names, part);
            if carons.contains(&bare) {
                sink.push(Hit::new("wrong-mark", format!("{glyph} uses {shown}, the ordinary caron.")).place(place()));
            } else if commas.contains(&bare) {
                sink.push(
                    Hit::new("bad-mark", format!("{glyph} uses {shown}, a comma or quote, instead of a vertical caron of its own."))
                        .severity(Severity::Warn)
                        .place(place()),
                );
            }
        }
    }
}

/// Is `name` one printable ASCII character, a name that does not count against composites?
fn single_character(name: &str) -> bool {
    let mut chars = name.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if c.is_ascii_graphic())
}

/// More than `maxOutlinePercent` of a TrueType font's glyphs are outlines rather than
/// composites. As in the reference check, empty glyphs count as outlines, the share is of every
/// glyph, and glyphs named with one ASCII character are left out of the count of outlines.
pub(crate) fn few_composites(ctx: &BinaryCtx<'_>, p: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let (Ok(maxp), Ok(glyf), Ok(loca)) = (font.maxp(), font.glyf(), font.loca(None)) else { return };
    let total = u32::from(maxp.num_glyphs());
    if total == 0 {
        return;
    }
    let names = Names::new(font);
    // Without stored names, a font's tools name the ASCII letters after themselves.
    let letters: Vec<u32> = if names.synthesized {
        let ascii: Vec<u32> = (0x41..=0x5A).chain(0x61..=0x7A).collect();
        mapped(font, &ascii)
    } else {
        Vec::new()
    };
    let mut outlines = 0u32;
    for gid in 0..total {
        if matches!(loca.get(GlyphId::new(gid), &glyf), Some(LocaGlyph::Glyph(TtGlyph::Composite(_)))) {
            continue;
        }
        let named = names.stored(gid).is_some_and(single_character);
        if !named && letters.binary_search(&gid).is_err() {
            outlines += 1;
        }
    }
    let percent = (f64::from(outlines) * 100.0 / f64::from(total)).round_ties_even();
    if percent > p.get(&MAX_OUTLINE_SHARE) {
        sink.push(
            Hit::new("few-composites", format!("{}% of the glyphs are stored as outlines rather than composites.", num(percent)))
                .data("percent", percent)
                .data("outlines", f64::from(outlines))
                .data("glyphs", f64::from(total))
                .place(Location { table: Some("glyf".into()), ..Location::default() }),
        );
    }
}

#[cfg(test)]
pub(crate) mod tests;
