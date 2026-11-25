//! Outline preparation (`docs/spec/40-compile.md` § 1.3), in the reference compilers' filter
//! order (`compile.filterOrder`): decompose, remove overlaps, convert curves, fix the direction.
//! Invisible open corners are erased before all of these ([`erase_open_corners`],
//! `compile.glyphsOutlinePreSteps`), so a component draws its base without them.
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
//! 3. **Curves** (TrueType only, `compile.cu2quTolerance`, `compile.ttfCurveModes`): in the
//!    default mode ([`CurveMode::Cu2Qu`]) every cubic segment becomes a quadratic spline within
//!    the tolerance, by kurbo's port of fontTools' cu2qu (`cubics_to_quadratic_splines`), as fontc
//!    converts them. A segment it cannot convert within 100 pieces (not seen in practice) is
//!    approximated by kurbo's subdivision instead. [`CurveMode::KeepQuad`] converts nothing: the
//!    source's curves must already be quadratic, and a glyph with a cubic segment stops the
//!    compile, as fontmake's `keep-quad` mode refuses it.
//! 4. **Direction** (TrueType only, `compile.reverseContourDirection`): contours are reversed,
//!    from the PostScript convention sources use to the TrueType one, unless the options keep
//!    the direction. Composites are not touched.
//! 5. **Implied on-curve points** (TrueType only, `compile.dropImpliedOnCurves`): an on-curve
//!    point exactly halfway between two off-curve points is left out by default, as fontc writes
//!    glyphs. With [`Settings::drop_implied`] off, only the points that the conversion or a
//!    quadratic spline of the source implies are left out, and every on-curve point the outline
//!    has as a node is written, as fontmake writes glyphs by default.
//!
//! Rounding (`compile.rounding`) happens when the glyph is written: `write-fonts` rounds every
//! point half up, and the table builders round component offsets and advances the same way
//! ([`crate::ot_round`]).

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use fontcraft_geom::{Affine, BezPath, CubicBez, PathEl, Point, cubics_to_quadratic_splines};
use fontcraft_model::{MAX_COMPONENT_DEPTH, MAX_COORDINATE, MAX_OUTLINE_ELEMENTS, ModelError, ResolvedComponent, ResolvedGlyph};
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

/// How the curves of a TrueType outline are made (`compile.ttfCurveModes`; the reference
/// compiler's `--ttf-curves`). Its two modes that write cubic curves into `glyf` wait for that
/// format to be final.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CurveMode {
    /// Every cubic segment becomes a quadratic spline (`cu2qu`).
    #[default]
    Cu2Qu,
    /// No conversion: the outlines must be quadratic already (`keep-quad`).
    KeepQuad,
}

impl CurveMode {
    /// The mode's name as the reference compiler spells it.
    pub fn name(self) -> &'static str {
        match self {
            CurveMode::Cu2Qu => "cu2qu",
            CurveMode::KeepQuad => "keep-quad",
        }
    }

    /// The mode called `name`; `None` for a name that is not one of [`Self::name`]'s.
    pub fn parse(name: &str) -> Option<Self> {
        [CurveMode::Cu2Qu, CurveMode::KeepQuad].into_iter().find(|m| m.name() == name)
    }
}

/// How outlines are prepared.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub flavour: Flavour,
    /// The most a quadratic curve may be from the cubic it replaces, in font units.
    pub tolerance: f64,
    pub keep_direction: bool,
    /// How TrueType curves are made.
    pub curves: CurveMode,
    /// Leave out every on-curve point that two off-curve points imply, not only those the
    /// conversion and the source's quadratic splines imply.
    pub drop_implied: bool,
    /// Most path elements decomposition may create over all glyphs
    /// ([`crate::MAX_EXPANDED_ELEMENTS`] in a compile).
    pub max_expanded: usize,
    pub remove_overlap: bool,
    /// How long one glyph's overlap removal may run.
    pub overlap_time_limit: Duration,
    /// Components are drawn in with single-precision arithmetic, as the reference's UFO reader
    /// places them (a project compile writing CFF, [`decomposed_single`]).
    pub single_precision: bool,
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
    /// Rounded quadratic points made with the other sources of a variable font, so every source
    /// keeps the same points; written as they are (TrueType only).
    Points(write_fonts::tables::glyf::SimpleGlyph),
}

/// Erases the invisible open corners ([`fontcraft_pathops::erase_invisible_corners`]) of the
/// contours of the glyphs `erased` names.
pub(crate) fn erase_open_corners(glyphs: &mut [ResolvedGlyph], erased: impl Fn(&str) -> bool) -> Result<(), crate::CompileError> {
    for g in glyphs {
        if g.contours.is_empty() || !erased(&g.name) {
            continue;
        }
        crate::check_stop()?;
        let mut work = fontcraft_pathops::MAX_CORNER_WORK;
        for contour in &mut g.contours {
            if let Some(closed) = fontcraft_pathops::erase_invisible_corners_within(contour, &mut work) {
                *contour = closed;
            }
        }
    }
    Ok(())
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
        crate::check_stop()?;
        let contours = if decompose.get(i) == Some(&true) && s.single_precision {
            let mut out = Vec::new();
            let mut budget = MAX_OUTLINE_ELEMENTS;
            decomposed_single(
                glyphs,
                &index,
                i,
                None,
                0,
                &mut Walk { out: &mut out, budget: &mut budget, expansion: &mut expansion, name: &g.name },
            )?;
            out
        } else if decompose.get(i) == Some(&true) {
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
pub(crate) fn composite(g: &ResolvedGlyph) -> Result<Prepared, CompileError> {
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
pub(crate) fn decomposed(
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
        // Charged to the font as well: a shared tree of composites without contours costs
        // every glyph that uses it a visit per path through it.
        expansion.take(1, &g.name)?;
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

/// A component transform in single precision, as AFDKO's UFO reader keeps it (`Transform`,
/// `matMult` and `setPointKeyValue` in `c/shared/uforead.cpp`, Apache-2.0): coefficients
/// `[xx, xy, yx, yy, dx, dy]` as `f32`, and whether it only moves.
#[derive(Clone, Copy)]
struct SingleTransform {
    m: [f32; 6],
    offset_only: bool,
}

impl SingleTransform {
    /// `None` for the identity, which leaves points as they are read.
    fn of(transform: Affine) -> Option<Self> {
        let m = transform.as_coeffs().map(|v| v as f32);
        let [xx, xy, yx, yy, dx, dy] = m;
        let offset_only = xx == 1.0 && yy == 1.0 && xy == 0.0 && yx == 0.0;
        (!offset_only || dx != 0.0 || dy != 0.0).then_some(Self { m, offset_only })
    }

    /// Whether every coefficient is finite: a value past the `f32` range, or a product of two
    /// large ones, is not.
    fn is_finite(&self) -> bool {
        self.m.iter().all(|v| v.is_finite())
    }

    /// The transform of a component with `local` inside a glyph placed by `outer`. An outer
    /// move adds to the inner offsets; anything else multiplies the matrices.
    fn within(local: Option<Self>, outer: Option<Self>) -> Option<Self> {
        match (local, outer) {
            (None, outer) => outer,
            (local, None) => local,
            (Some(mut a), Some(b)) if b.offset_only => {
                a.m[4] += b.m[4];
                a.m[5] += b.m[5];
                Some(a)
            }
            (Some(Self { m: a, .. }), Some(Self { m: b, .. })) => {
                let [a0, a1, a2, a3, a4, a5] = a;
                let [b0, b1, b2, b3, b4, b5] = b;
                Some(Self {
                    m: [a0 * b0 + a1 * b2, a0 * b1 + a1 * b3, a2 * b0 + a3 * b2, a2 * b1 + a3 * b3, a4 * b0 + a5 * b2 + b4, a4 * b1 + a5 * b3 + b5],
                    offset_only: false,
                })
            }
        }
    }

    fn apply(&self, p: Point) -> Point {
        let (x, y) = (p.x as f32, p.y as f32);
        let [xx, xy, yx, yy, dx, dy] = self.m;
        Point::new(f64::from(xx * x + yx * y + dx), f64::from(xy * x + yy * y + dy))
    }

    fn mirrors(&self) -> bool {
        let [xx, xy, yx, yy, ..] = self.m.map(f64::from);
        xx * yy - xy * yx < 0.0
    }
}

/// `contour` with every point read in single precision and placed by `t`; `None` when a point
/// lands past ±[`MAX_COORDINATE`] or is not a number.
fn single_contour(contour: &BezPath, t: Option<SingleTransform>) -> Option<BezPath> {
    let place = |p: Point| {
        let q = match &t {
            Some(t) => t.apply(p),
            None => Point::new(f64::from(p.x as f32), f64::from(p.y as f32)),
        };
        (q.is_finite() && q.x.abs() <= MAX_COORDINATE && q.y.abs() <= MAX_COORDINATE).then_some(q)
    };
    let path: BezPath = contour
        .elements()
        .iter()
        .map(|el| {
            Some(match *el {
                PathEl::MoveTo(p) => PathEl::MoveTo(place(p)?),
                PathEl::LineTo(p) => PathEl::LineTo(place(p)?),
                PathEl::QuadTo(a, p) => PathEl::QuadTo(place(a)?, place(p)?),
                PathEl::CurveTo(a, b, p) => PathEl::CurveTo(place(a)?, place(b)?, place(p)?),
                PathEl::ClosePath => PathEl::ClosePath,
            })
        })
        .collect::<Option<_>>()?;
    Some(if t.is_some_and(|t| t.mirrors()) { path.reverse_subpaths() } else { path })
}

/// Where [`decomposed_single`] puts contours, and what it may still add.
struct Walk<'a> {
    out: &'a mut Vec<BezPath>,
    budget: &'a mut usize,
    expansion: &'a mut Expansion,
    name: &'a str,
}

/// The contours of glyph `i` placed by `t`, then those of its components depth first, each
/// placed by its transform within `t`, all in single precision: the reference's UFO reader
/// reads coordinates and transforms as `f32` and combines them before placing a point, so a
/// component's point can land a hundredth away from where double precision puts it once the
/// charstring keeps two decimals.
fn decomposed_single(
    glyphs: &[ResolvedGlyph],
    index: &HashMap<&str, usize>,
    i: usize,
    t: Option<SingleTransform>,
    depth: usize,
    walk: &mut Walk,
) -> Result<(), CompileError> {
    let Some(g) = glyphs.get(i) else { return Ok(()) };
    let too_large = || CompileError::from(ModelError::OutlineTooLarge(walk.name.to_string()));
    let out_of_range = || CompileError::Glyph {
        glyph: walk.name.to_string(),
        message: format!("a component places points beyond ±{MAX_COORDINATE} in single precision"),
    };
    if depth > MAX_COMPONENT_DEPTH {
        return Err(ModelError::ComponentCycle(walk.name.to_string()).into());
    }
    for c in &g.contours {
        *walk.budget = walk.budget.checked_sub(c.elements().len()).ok_or_else(too_large)?;
        if depth > 0 {
            walk.expansion.take(c.elements().len(), walk.name)?;
        }
        walk.out.push(single_contour(c, t).ok_or_else(out_of_range)?);
    }
    for c in &g.components {
        *walk.budget = walk.budget.checked_sub(1).ok_or_else(too_large)?;
        // Charged to the font as well: a shared tree of composites without contours costs
        // every glyph that uses it a visit per path through it.
        walk.expansion.take(1, walk.name)?;
        let Some(&j) = index.get(c.base.as_str()) else { continue };
        let placed = SingleTransform::within(SingleTransform::of(c.transform), t);
        if placed.is_some_and(|p| !p.is_finite()) {
            return Err(out_of_range());
        }
        decomposed_single(glyphs, index, j, placed, depth + 1, walk)?;
    }
    Ok(())
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
    let quadratic = match s.curves {
        CurveMode::Cu2Qu => to_quadratic(&path, s.tolerance),
        CurveMode::KeepQuad => {
            refuse_cubic(&g.name, [&path])?;
            path.clone()
        }
    };
    let quadratic = if s.keep_direction { quadratic } else { quadratic.reverse_subpaths() };
    if s.drop_implied {
        return Ok(Prepared::Simple(quadratic));
    }
    // Positions are not changed by the conversion's end points or by reversing, so the nodes of
    // `path` find their points in `quadratic`.
    let nodes = on_curve_nodes(&path);
    Ok(match crate::points::interpolatable_glyphs(std::slice::from_ref(&quadratic), &nodes).and_then(|mut v| v.pop()) {
        Some(glyph) => Prepared::Points(glyph),
        None => Prepared::Simple(quadratic),
    })
}

/// An error naming `glyph` when one of `paths` has a cubic segment: [`CurveMode::KeepQuad`]
/// writes the curves as they are, and `glyf` stores only quadratic ones.
pub(crate) fn refuse_cubic<'a>(glyph: &str, paths: impl IntoIterator<Item = &'a BezPath>) -> Result<(), CompileError> {
    if paths.into_iter().any(|p| p.elements().iter().any(|el| matches!(el, PathEl::CurveTo(..)))) {
        return Err(CompileError::Glyph {
            glyph: glyph.to_string(),
            message: "it has cubic curves, which the keep-quad curve mode writes as they are and a TrueType outline cannot hold; convert them to quadratic curves or use the cu2qu mode".into(),
        });
    }
    Ok(())
}

/// The on-curve points of `path` that are nodes of the outline: every point a move, a line or a
/// cubic segment ends at, and every end of a quadratic segment except the points a quadratic
/// spline of the source implies (exactly halfway between its two off-curve neighbours, which is
/// how [`fontcraft_model::Path::to_contour`] writes them). The points cu2qu adds inside a spline
/// are not in `path` at all.
pub(crate) fn on_curve_nodes(path: &BezPath) -> Vec<Point> {
    let els = path.elements();
    let mut out = Vec::new();
    let mut start = None::<Point>;
    for (i, el) in els.iter().enumerate() {
        match *el {
            PathEl::MoveTo(p) => {
                start = Some(p);
                // A closed subpath made only of off-curve nodes starts at an implied point: the
                // one between its last and its first off-curve point.
                let first = match els.get(i + 1) {
                    Some(PathEl::QuadTo(c, _)) => Some(*c),
                    _ => None,
                };
                let rest = els.get(i + 1..).unwrap_or_default();
                let end = rest.iter().position(|e| matches!(e, PathEl::MoveTo(_))).unwrap_or(rest.len());
                let closing = rest.get(..end).unwrap_or_default().iter().rev().find(|e| !matches!(e, PathEl::ClosePath));
                let implied = match (first, closing) {
                    (Some(a), Some(PathEl::QuadTo(b, q))) => *q == p && b.midpoint(a) == p,
                    _ => false,
                };
                if !implied {
                    out.push(p);
                }
            }
            PathEl::LineTo(p) | PathEl::CurveTo(_, _, p) => out.push(p),
            PathEl::QuadTo(c, p) => {
                let implied = match els.get(i + 1) {
                    Some(PathEl::QuadTo(next, _)) => c.midpoint(*next) == p,
                    // The segment that closes the subpath ends at its start, which the move decided.
                    Some(PathEl::ClosePath) => start == Some(p),
                    _ => false,
                };
                if !implied {
                    out.push(p);
                }
            }
            PathEl::ClosePath => {}
        }
    }
    out
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
