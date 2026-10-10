//! Outline preparation (`docs/spec/40-compile.md` § 1.3), in the reference compilers' filter
//! order (`compile.filterOrder`): decompose, remove overlaps, convert curves, fix the direction.
//!
//! 1. **Decompose** (`compile.mixedGlyphsBecomeSimple`,
//!    `compile.overflowingComponentTransformDecompose`, `compile.otfDecomposesAllComponents`): a
//!    glyph becomes contours when it has both contours and components, when a component's 2×2
//!    transform has a value outside −2 to 2 (the F2Dot14 range of `glyf`), or, for CFF, whenever
//!    it has components. Every other composite stays a composite. A decomposed glyph keeps its own
//!    contours first, then the components' contours in breadth-first order, each transformed and,
//!    when the transform mirrors it, reversed; glyphs that are decomposed themselves are
//!    decomposed before the glyphs that use them. This is fontc's `convert_components_to_contours`
//!    and `resolve_inconsistencies` (`fontir/src/glyph.rs`, Apache-2.0).
//! 2. **Overlaps** (`compile.removeOverlapsStatic`, `export.removeOverlapOption`), unless the
//!    options keep them: every glyph that has contours after step 1 becomes the nonzero union of
//!    its closed contours, as Path › Remove Overlap makes it ([`fontcraft_pathops::remove_overlap`];
//!    result points stay unrounded until the glyph is written, as in fontmake). Open paths take
//!    no part and are left out, with a warning. A composite keeps its components, so components
//!    that overlap each other stay as they are (fontmake's `RemoveOverlapsFilter` skips glyphs
//!    without contours the same way). Each glyph runs on a worker thread under
//!    [`Settings::overlap_time_limit`] (docs/plans/M3.md › M3.6, rule 8): a glyph that runs out
//!    of time or fails keeps its overlaps, with a warning naming it. In the browser the union
//!    runs inline.
//! 3. **Curves** (TrueType only, `compile.cu2quTolerance`): every cubic segment becomes a
//!    quadratic spline within the tolerance, by kurbo's port of fontTools' cu2qu
//!    (`cubics_to_quadratic_splines`), as fontc converts them. A segment it cannot convert within
//!    100 pieces (not seen in practice) is approximated by kurbo's subdivision instead.
//! 4. **Direction** (TrueType only, `compile.reverseContourDirection`): contours are reversed,
//!    from the PostScript convention sources use to the TrueType one, unless the options keep
//!    the direction. Composites are not touched.
//!
//! Rounding (`compile.rounding`) happens when the glyph is written: `write-fonts` rounds every
//! point half up, and the table builders round component offsets and advances the same way
//! ([`crate::ot_round`]).

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use fontcraft_geom::{Affine, BezPath, CubicBez, PathEl, Point, cubics_to_quadratic_splines};
use fontcraft_model::{MAX_COMPONENT_DEPTH, MAX_OUTLINE_ELEMENTS, ModelError, ResolvedComponent, ResolvedGlyph};
use fontcraft_pathops::{OpenPaths, OverlapOptions, PathOpError, contours_from_bez, remove_overlap, run_with_time_limit};

use crate::{CompileError, Expansion, MAX_COMPONENTS, MAX_TT_COORDINATE, preflight::transformed};

/// The outline format of the font.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavour {
    /// Quadratic outlines in `glyf`, with composites.
    TrueType,
    /// Cubic outlines in `CFF `, no composites (M2.3).
    Cff,
}

/// How outlines are prepared.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub flavour: Flavour,
    /// The most a quadratic curve may be from the cubic it replaces, in font units.
    pub tolerance: f64,
    pub keep_direction: bool,
    /// Most path elements decomposition may create over all glyphs
    /// ([`crate::MAX_EXPANDED_ELEMENTS`] in a compile).
    pub max_expanded: usize,
    pub remove_overlap: bool,
    /// How long one glyph's overlap removal may run.
    pub overlap_time_limit: Duration,
}

/// One glyph ready for its table.
#[derive(Clone, Debug, PartialEq)]
pub enum Prepared {
    /// No outline.
    Empty,
    /// Contours: quadratic and in TrueType direction for TrueType, as drawn for CFF. Coordinates
    /// are not rounded yet.
    Simple(BezPath),
    /// Components, by glyph name, each with its transform.
    Composite(Vec<ResolvedComponent>),
}

/// Does a component's 2×2 transform hold a value that `glyf` cannot store?
fn overflows(c: &ResolvedComponent) -> bool {
    c.transform.as_coeffs().iter().take(4).any(|v| !(-2.0..=2.0).contains(v))
}

/// Should `g` be decomposed for `flavour`?
pub fn must_decompose(g: &ResolvedGlyph, flavour: Flavour) -> bool {
    !g.components.is_empty() && (flavour == Flavour::Cff || !g.contours.is_empty() || g.components.iter().any(overflows))
}

/// Prepares every glyph of the final order.
pub fn prepare(glyphs: &[ResolvedGlyph], s: &Settings, warnings: &mut Vec<String>) -> Result<Vec<Prepared>, CompileError> {
    let index: HashMap<&str, usize> = glyphs.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
    let decompose: Vec<bool> = glyphs.iter().map(|g| must_decompose(g, s.flavour)).collect();
    let mut memo: HashMap<usize, Vec<BezPath>> = HashMap::new();
    let mut expansion = Expansion::new(s.max_expanded);
    let mut out = Vec::with_capacity(glyphs.len());
    for (i, g) in glyphs.iter().enumerate() {
        let contours = if decompose.get(i) == Some(&true) {
            decomposed(glyphs, &index, &decompose, i, 0, &mut memo, &mut expansion)?
        } else if !g.components.is_empty() {
            out.push(composite(g)?);
            continue;
        } else {
            g.contours.clone()
        };
        let contours = if s.remove_overlap { without_overlaps(g, contours, s.overlap_time_limit, warnings) } else { contours };
        out.push(simple(g, contours, s, warnings)?);
    }
    Ok(out)
}

/// A composite as it is written, after the checks `glyf` needs.
fn composite(g: &ResolvedGlyph) -> Result<Prepared, CompileError> {
    let bad = |message: String| CompileError::Glyph { glyph: g.name.clone(), message };
    if g.components.len() > MAX_COMPONENTS {
        return Err(bad(format!("{} components; a glyph holds at most {MAX_COMPONENTS}", g.components.len())));
    }
    for c in &g.components {
        let [.., dx, dy] = c.transform.as_coeffs();
        if !(dx.abs() <= MAX_TT_COORDINATE && dy.abs() <= MAX_TT_COORDINATE) {
            return Err(bad(format!("the component `{}` is placed more than {MAX_TT_COORDINATE} units from the origin", c.base)));
        }
    }
    Ok(Prepared::Composite(g.components.clone()))
}

/// The contours of glyph `i` with all its components drawn in.
fn decomposed(
    glyphs: &[ResolvedGlyph],
    index: &HashMap<&str, usize>,
    decompose: &[bool],
    i: usize,
    depth: usize,
    memo: &mut HashMap<usize, Vec<BezPath>>,
    expansion: &mut Expansion,
) -> Result<Vec<BezPath>, CompileError> {
    if let Some(done) = memo.get(&i) {
        return Ok(done.clone());
    }
    let Some(g) = glyphs.get(i) else { return Ok(Vec::new()) };
    if depth > MAX_COMPONENT_DEPTH {
        return Err(ModelError::ComponentCycle(g.name.clone()).into());
    }
    let too_large = || CompileError::from(ModelError::OutlineTooLarge(g.name.clone()));
    let mut out = g.contours.clone();
    let mut budget = MAX_OUTLINE_ELEMENTS.checked_sub(out.iter().map(|c| c.elements().len()).sum()).ok_or_else(too_large)?;
    let mut frontier: VecDeque<(&str, Affine)> = g.components.iter().map(|c| (c.base.as_str(), c.transform)).collect();
    while let Some((base, affine)) = frontier.pop_front() {
        budget = budget.checked_sub(1).ok_or_else(too_large)?;
        let Some(&j) = index.get(base) else { continue };
        let Some(r) = glyphs.get(j) else { continue };
        if decompose.get(j) == Some(&true) {
            for c in decomposed(glyphs, index, decompose, j, depth + 1, memo, expansion)? {
                budget = budget.checked_sub(c.elements().len()).ok_or_else(too_large)?;
                expansion.take(c.elements().len(), &g.name)?;
                out.push(transformed(&c, affine));
            }
        } else {
            for c in &r.contours {
                budget = budget.checked_sub(c.elements().len()).ok_or_else(too_large)?;
                expansion.take(c.elements().len(), &g.name)?;
                out.push(transformed(c, affine));
            }
            frontier.extend(r.components.iter().map(|c| (c.base.as_str(), affine * c.transform)));
        }
    }
    memo.insert(i, out.clone());
    Ok(out)
}

/// `contours` with their overlaps removed, or as they are, with a warning, when that fails or
/// takes longer than `limit`.
fn without_overlaps(g: &ResolvedGlyph, contours: Vec<BezPath>, limit: Duration, warnings: &mut Vec<String>) -> Vec<BezPath> {
    if contours.is_empty() {
        return contours;
    }
    let input = contours.clone();
    let job = move || -> Result<(Vec<BezPath>, usize), PathOpError> {
        let nodes: Vec<_> = input.iter().flat_map(contours_from_bez).collect();
        let open = nodes.iter().filter(|c| !c.closed && c.nodes.len() > 1).count();
        let merged = remove_overlap(&nodes, OverlapOptions { grid: 0.0, open_paths: OpenPaths::Remove })?;
        let paths = merged.iter().map(|c| c.to_bez().map(with_closing_line)).collect::<Result<_, _>>()?;
        Ok((paths, open))
    };
    let kept = |why: String| format!("glyph `{}` keeps its overlapping contours: {why}", g.name);
    match run_with_time_limit(limit, job) {
        Ok(Ok((paths, open))) => {
            if open > 0 {
                warnings.push(format!("glyph `{}`: {open} open paths were left out; overlap removal takes closed paths only", g.name));
            }
            paths
        }
        Ok(Err(e)) => {
            warnings.push(kept(e.to_string()));
            contours
        }
        Err(e) => {
            warnings.push(kept(e.to_string()));
            contours
        }
    }
}

/// `path` with an explicit line back to the start of each closed subpath that does not end
/// there, the form [`fontcraft_model::Path::to_contour`] gives compilers.
fn with_closing_line(path: BezPath) -> BezPath {
    let mut out = BezPath::new();
    let (mut start, mut current) = (Point::ZERO, Point::ZERO);
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => (start, current) = (p, p),
            PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => current = p,
            PathEl::ClosePath => {
                if current != start {
                    out.line_to(start);
                }
                current = start;
            }
        }
        out.push(*el);
    }
    out
}

/// Every point of `path`, on-curve and off-curve.
pub(crate) fn points(path: &BezPath) -> impl Iterator<Item = Point> + '_ {
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

/// A glyph of contours only, prepared for `s.flavour`.
fn simple(g: &ResolvedGlyph, contours: Vec<BezPath>, s: &Settings, warnings: &mut Vec<String>) -> Result<Prepared, CompileError> {
    let mut path = BezPath::new();
    for c in contours {
        path.extend(c);
    }
    if path.elements().is_empty() {
        return Ok(Prepared::Empty);
    }
    if s.flavour == Flavour::Cff {
        return Ok(Prepared::Simple(path));
    }
    // Checked on every point, control points included, and before any curve work: the
    // conversion does work in proportion to the size of what it is given.
    if points(&path).any(|p| !(p.x.abs() <= MAX_TT_COORDINATE && p.y.abs() <= MAX_TT_COORDINATE)) {
        return Err(CompileError::Glyph {
            glyph: g.name.clone(),
            message: format!("a point lies more than {MAX_TT_COORDINATE} units from the origin, further than a TrueType outline can store"),
        });
    }
    if has_open_contour(&path) {
        warnings.push(format!("glyph `{}` has an open path; TrueType outlines are closed, so it was closed", g.name));
    }
    let quadratic = to_quadratic(&path, s.tolerance);
    Ok(Prepared::Simple(if s.keep_direction { quadratic } else { quadratic.reverse_subpaths() }))
}

/// `path` with every cubic segment replaced by a quadratic spline at most `tolerance` font units
/// from it (cu2qu, as kurbo ports it). Lines, quadratic segments, moves and closes are kept.
pub fn to_quadratic(path: &BezPath, tolerance: f64) -> BezPath {
    let tolerance = if tolerance.is_finite() && tolerance > 0.0 { tolerance } else { 1.0 };
    let mut out = BezPath::new();
    let (mut last, mut start) = (Point::ZERO, Point::ZERO);
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                out.move_to(p);
                (last, start) = (p, p);
            }
            PathEl::LineTo(p) => {
                out.line_to(p);
                last = p;
            }
            PathEl::QuadTo(c, p) => {
                out.quad_to(c, p);
                last = p;
            }
            PathEl::CurveTo(c1, c2, p) => {
                let cubic = CubicBez::new(last, c1, c2, p);
                match cubics_to_quadratic_splines(&[cubic], tolerance).and_then(|mut v| v.pop()) {
                    Some(spline) => spline.to_quads().for_each(|q| out.quad_to(q.p1, q.p2)),
                    None => cubic.to_quads(tolerance).for_each(|(_, _, q)| out.quad_to(q.p1, q.p2)),
                }
                last = p;
            }
            PathEl::ClosePath => {
                out.close_path();
                last = start;
            }
        }
    }
    out
}
