//! `cargo xtask pathops-noop`: Correct Path Direction and Add Extremes on every master layer of
//! the corpus's `.glyphs` and `.glyphspackage` sources (docs/plans/M3.md › M3.32).
//!
//! The reference app saved most of these sources, usually after its own two commands ran, so a
//! layer ours change shows a rule that may differ. For each layer the report says whether Correct
//! Path Direction left it alone or changed directions, start points or the shape order; whether
//! Add Extremes added or moved nodes, how short the new segments are, and whether the on-curve
//! bounds equal the exact bounds before and after. Both commands run as the editor runs them,
//! through a session. Totals cover the sources the reference app wrote; the per-source table covers
//! all of them.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Font, Layer, NodeKind, Path as ModelPath, Shape as ModelShape};
use fontcraft_geom::{BezPath, PathEl, Point, Rect, Shape};
use serde_json::json;

use crate::roundtrip::{self, Kind};

/// On-curve and exact bounds that differ by at most this many units are equal.
const BOUNDS_TOLERANCE: f64 = 0.5;
/// Upper ends of the buckets new segments are counted in, by length in units.
const BUCKETS: [f64; 6] = [3.0, 5.0, 10.0, 20.0, 50.0, f64::INFINITY];
/// Lines listed per category in the summary.
const LIST: usize = 25;

/// What the two commands did, over the layers of one source or of all.
#[derive(Default, Clone)]
struct Counts {
    /// Master layers with a closed path.
    layers: usize,
    /// Layers none of whose paths changed direction.
    wound: usize,
    /// Of those, the ones Correct Path Direction left exactly as they were.
    unchanged: usize,
    start: usize,
    order: usize,
    /// Master layers with a curve.
    curves: usize,
    /// Of those, the ones whose on-curve bounds are exact before the command…
    bounds_ok: usize,
    /// …to which Add Extremes added nothing, and whose nodes it left in place.
    none_added: usize,
    none_moved: usize,
    /// Layers whose on-curve bounds are exact after the command.
    bounds_after: usize,
    skipped: usize,
}

impl Counts {
    fn add(&mut self, o: &Counts) {
        for (a, b) in [
            (&mut self.layers, o.layers),
            (&mut self.wound, o.wound),
            (&mut self.unchanged, o.unchanged),
            (&mut self.start, o.start),
            (&mut self.order, o.order),
            (&mut self.curves, o.curves),
            (&mut self.bounds_ok, o.bounds_ok),
            (&mut self.none_added, o.none_added),
            (&mut self.none_moved, o.none_moved),
            (&mut self.bounds_after, o.bounds_after),
            (&mut self.skipped, o.skipped),
        ] {
            *a += b;
        }
    }

    fn line(&self) -> String {
        format!(
            "direction: layers {}, wound {}, unchanged {} ({}), start moved {}, order changed {}; extremes: layers {}, bounds exact {}, nothing added {} ({}), nothing moved {}, bounds exact after {}; skipped {}",
            self.layers,
            self.wound,
            self.unchanged,
            share(self.unchanged, self.wound),
            self.start,
            self.order,
            self.curves,
            self.bounds_ok,
            self.none_added,
            share(self.none_added, self.bounds_ok),
            self.none_moved,
            self.bounds_after,
            self.skipped
        )
    }
}

fn share(n: usize, of: usize) -> String {
    format!("{:.2}%", 100.0 * n as f64 / of.max(1) as f64)
}

/// What the lists and histograms hold.
#[derive(Default)]
struct Detail {
    /// Where the saved start of a path whose start moved lies, by kind.
    starts: BTreeMap<&'static str, usize>,
    /// What the saved order of a reordered layer is, by kind.
    orders: BTreeMap<&'static str, usize>,
    /// New segments by length: added by Add Extremes on layers whose bounds were exact.
    segments: [usize; BUCKETS.len()],
    turned: Vec<String>,
    start: Vec<String>,
    order: Vec<String>,
    added: Vec<String>,
    moved: Vec<String>,
    after: Vec<String>,
    failed: Vec<String>,
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let mut filter: Option<String> = None;
    let mut out_dir = root.join("target/pathops-noop");
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match *a {
            "--match" => filter = Some(it.next().ok_or("--match needs text")?.to_string()),
            "--out" => out_dir = it.next().ok_or("--out needs a folder")?.into(),
            other => return Err(format!("unknown argument `{other}`; usage: cargo xtask pathops-noop [--out DIR] [--match TEXT]")),
        }
    }
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let corpus = root.join("corpus");
    let sources: Vec<roundtrip::Sample> = roundtrip::discover(&corpus)?
        .into_iter()
        .filter(|s| matches!(s.kind, Kind::Glyphs | Kind::GlyphsPackage))
        .filter(|s| filter.as_ref().is_none_or(|f| s.path.contains(f.as_str())))
        .collect();
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut total = Counts::default();
    let mut table = Vec::new();
    let mut detail = Detail::default();
    for sample in &sources {
        let path = corpus.join(&sample.path);
        let opened = catch_unwind(|| fontcraft_engine::io::open(&path, None));
        let font = match opened {
            Ok(Ok((font, _))) => font,
            Ok(Err(e)) => {
                detail.failed.push(format!("{}: {e}", sample.path));
                continue;
            }
            Err(_) => {
                detail.failed.push(format!("{}: panicked while loading", sample.path));
                continue;
            }
        };
        let mut other = Detail::default();
        let sink = if sample.reference_written { &mut detail } else { &mut other };
        let measured = catch_unwind(AssertUnwindSafe(|| measure(&font, &sample.path, sink)));
        match measured {
            Ok(Ok(counts)) => {
                if sample.reference_written {
                    total.add(&counts);
                }
                if counts.layers + counts.curves > 0 {
                    table.push((
                        counts.layers,
                        format!("{}{}: {}", sample.path, if sample.reference_written { "" } else { " (not saved by the reference)" }, counts.line()),
                    ));
                }
            }
            Ok(Err(e)) => detail.failed.push(format!("{}: {e}", sample.path)),
            Err(_) => detail.failed.push(format!("{}: panicked", sample.path)),
        }
        let failed = std::mem::take(&mut other.failed);
        detail.failed.extend(failed);
    }
    std::panic::set_hook(default_hook);

    table.sort_by(|a, b| b.0.cmp(&a.0));
    let mut s = format!("sources {}, failed {}\nsaved by the reference: {}\n", sources.len(), detail.failed.len(), total.line());
    writeln!(s, "saved start of the paths whose start moved: {:?}", detail.starts).ok();
    writeln!(s, "saved order of the layers that were reordered: {:?}", detail.orders).ok();
    let buckets: Vec<String> = BUCKETS.iter().zip(detail.segments).map(|(to, n)| format!("< {to}: {n}")).collect();
    writeln!(s, "shortest segment next to each added extreme, on layers with exact bounds: {}", buckets.join(", ")).ok();
    writeln!(s, "\nper source:").ok();
    for (_, line) in &table {
        writeln!(s, "  {line}").ok();
    }
    let full = [
        ("failed", &detail.failed),
        ("direction changed", &detail.turned),
        ("start moved", &detail.start),
        ("order changed", &detail.order),
        ("extremes added although the bounds were exact", &detail.added),
        ("nodes moved although the bounds were exact", &detail.moved),
        ("bounds not exact after Add Extremes", &detail.after),
    ];
    let mut file = s.clone();
    for (title, list) in full {
        writeln!(s, "\n{title} ({}):", list.len()).ok();
        writeln!(file, "\n{title} ({}):", list.len()).ok();
        for (i, line) in list.iter().enumerate() {
            writeln!(file, "  {line}").ok();
            if i < LIST {
                writeln!(s, "  {line}").ok();
            }
        }
    }
    print!("{s}");
    let summary = out_dir.join("summary.txt");
    std::fs::write(&summary, file).map_err(|e| format!("{}: {e}", summary.display()))?;
    println!("full lists: {}", summary.display());
    Ok(())
}

fn measure(font: &Font, source: &str, detail: &mut Detail) -> Result<Counts, String> {
    let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
    let masters: Vec<String> = font.masters.iter().map(|m| m.id.clone()).collect();
    if names.is_empty() || masters.is_empty() {
        return Ok(Counts::default());
    }
    let mut counts = Counts::default();
    for (id, direction) in [("path.correctPathDirection", true), ("path.addExtremes", false)] {
        let mut session = Session::new();
        session.add_document(font.clone(), None).map_err(|e| e.to_string())?;
        let report = session.execute(id, &json!({"glyphs": names, "layers": masters})).map_err(|e| format!("{id}: {e}"))?;
        counts.skipped += report["skipped"].as_array().map_or(0, Vec::len);
        let after = session.font().map_err(|e| e.to_string())?;
        for glyph in font.glyphs.iter() {
            let Some(g) = after.glyph(&glyph.name) else { continue };
            for old in glyph.layers.iter().filter(|l| l.is_master_layer()) {
                let Some(new) = g.layers.iter().find(|l| l.id == old.id) else { continue };
                let name = format!("{source} › {} › {}", glyph.name, old.id);
                if direction {
                    compare_direction(old, new, &name, &mut counts, detail);
                } else {
                    compare_extremes(old, new, &name, &mut counts, detail);
                }
            }
        }
    }
    Ok(counts)
}

fn paths(layer: &Layer) -> Vec<&ModelPath> {
    layer.shapes.iter().filter_map(|s| if let ModelShape::Path(p) = s { Some(p) } else { None }).collect()
}

/// A path's nodes without their order, so a path is found again after it was turned or restarted.
fn key(p: &ModelPath) -> Vec<(u64, u64)> {
    let mut k: Vec<(u64, u64)> = p.nodes.iter().map(|n| ((n.x + 0.0).to_bits(), (n.y + 0.0).to_bits())).collect();
    k.sort_unstable();
    k
}

fn area(p: &ModelPath) -> f64 {
    p.to_bez().map_or(0.0, |b| b.area())
}

fn bounds(p: &ModelPath) -> Option<Rect> {
    p.to_bez().ok().filter(|b| !b.elements().is_empty()).map(|b| b.bounding_box())
}

fn start(p: &ModelPath) -> Option<Point> {
    p.nodes.iter().find(|n| n.on_curve()).map(|n| n.point())
}

fn compare_direction(old: &Layer, new: &Layer, name: &str, counts: &mut Counts, detail: &mut Detail) {
    let before = paths(old);
    if !before.iter().any(|p| p.closed && p.nodes.len() > 1) {
        return;
    }
    counts.layers += 1;
    let mut found: HashMap<Vec<(u64, u64)>, Vec<&ModelPath>> = HashMap::new();
    for p in paths(new).into_iter().rev() {
        found.entry(key(p)).or_default().push(p);
    }
    let (mut turned, mut moved) = (false, Vec::new());
    for p in &before {
        let Some(q) = found.get_mut(&key(p)).and_then(Vec::pop) else { continue };
        turned |= area(p) * area(q) < 0.0;
        if p.closed && start(p) != start(q) {
            moved.push(*p);
        }
    }
    // Shapes in order: each path by its nodes, each component by itself.
    let order = |l: &Layer| -> Vec<String> {
        l.shapes
            .iter()
            .map(|s| match s {
                ModelShape::Path(p) => format!("{:?}", key(p)),
                other => serde_json::to_string(other).unwrap_or_default(),
            })
            .collect()
    };
    let reordered = order(old) != order(new);
    if turned {
        detail.turned.push(name.to_string());
        return;
    }
    counts.wound += 1;
    counts.unchanged += usize::from(moved.is_empty() && !reordered);
    if !moved.is_empty() {
        counts.start += 1;
        detail.start.push(format!("{name}: {} of {} paths", moved.len(), before.len()));
        for p in moved {
            *detail.starts.entry(start_kind(p)).or_default() += 1;
        }
    }
    if reordered {
        counts.order += 1;
        let kind = order_kind(&before);
        *detail.orders.entry(kind).or_default() += 1;
        detail.order.push(format!("{name}: {} shapes, saved {kind}", old.shapes.len()));
    }
}

/// Where the saved start of `p` lies.
fn start_kind(p: &ModelPath) -> &'static str {
    let on: Vec<Point> = p.nodes.iter().filter(|n| n.on_curve()).map(|n| n.point()).collect();
    let Some(s) = start(p) else { return "none" };
    let least = |f: &dyn Fn(&Point) -> f64| on.iter().map(f).fold(f64::INFINITY, f64::min);
    if s.y == least(&|q| q.y) {
        "lowest, not leftmost"
    } else if s.x + s.y == least(&|q| q.x + q.y) {
        "least x + y"
    } else if s.x == least(&|q| q.x) {
        "leftmost"
    } else if s.y == -least(&|q| -q.y) {
        "highest"
    } else {
        "elsewhere"
    }
}

/// The first of some simple orders the paths already have.
fn order_kind(paths: &[&ModelPath]) -> &'static str {
    let boxes: Vec<Rect> = paths.iter().filter_map(|p| bounds(p)).collect();
    let rising = |f: &dyn Fn(&Rect) -> f64| boxes.windows(2).all(|w| f(&w[0]) <= f(&w[1]));
    if rising(&|r| r.x0) {
        "left to right"
    } else if rising(&|r| r.y0) {
        "bottom to top"
    } else if rising(&|r| -r.width() * r.height()) {
        "largest first"
    } else if rising(&|r| -r.x0) {
        "right to left"
    } else if rising(&|r| -r.y0) {
        "top to bottom"
    } else {
        "another order"
    }
}

/// The bounds of the segments' end points, and of the curves themselves.
fn two_bounds(layer: &Layer) -> Option<(Rect, Rect)> {
    let outline: Vec<BezPath> = paths(layer).into_iter().filter(|p| p.nodes.len() > 1).filter_map(|p| p.to_bez().ok()).collect();
    let mut ends = None::<Rect>;
    for el in outline.iter().flat_map(BezPath::elements) {
        if let PathEl::MoveTo(p) | PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) = el {
            ends = Some(ends.map_or(Rect::from_points(*p, *p), |r| r.union_pt(*p)));
        }
    }
    let exact = outline.iter().map(|p| p.bounding_box()).reduce(|a, b| a.union(b))?;
    Some((ends?, exact))
}

fn equal(a: Rect, b: Rect) -> bool {
    [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|d| d.abs() <= BOUNDS_TOLERANCE)
}

fn on_points(p: &ModelPath) -> Vec<Point> {
    p.nodes.iter().filter(|n| n.on_curve()).map(|n| n.point()).collect()
}

fn compare_extremes(old: &Layer, new: &Layer, name: &str, counts: &mut Counts, detail: &mut Detail) {
    let before = paths(old);
    if !before.iter().any(|p| p.nodes.iter().any(|n| n.kind == NodeKind::Curve || n.kind == NodeKind::QCurve)) {
        return;
    }
    let (Some((ends, exact)), Some((ends_after, exact_after))) = (two_bounds(old), two_bounds(new)) else { return };
    counts.curves += 1;
    if equal(ends_after, exact_after) {
        counts.bounds_after += 1;
    } else {
        detail.after.push(format!("{name}: {ends_after:?} against {exact_after:?}"));
    }
    if !equal(ends, exact) {
        return;
    }
    counts.bounds_ok += 1;
    let (mut added, mut moved, mut shortest) = (0usize, 0usize, f64::INFINITY);
    for (p, q) in before.iter().zip(paths(new)) {
        let (a, b) = (on_points(p), on_points(q));
        added += b.len().saturating_sub(a.len());
        let gone = a.iter().filter(|x| !b.contains(x)).count();
        moved += gone;
        // With nodes added and none moved, every point that was not there is a new extreme.
        for (i, x) in b.iter().enumerate().filter(|(_, x)| b.len() > a.len() && !a.contains(x)) {
            let near = [b.get((i + b.len() - 1) % b.len()), b.get((i + 1) % b.len())];
            let length = near.into_iter().flatten().map(|y| y.distance(*x)).fold(f64::INFINITY, f64::min);
            shortest = shortest.min(length);
            if let Some(n) = BUCKETS.iter().position(|to| length < *to).and_then(|k| detail.segments.get_mut(k)) {
                *n += 1;
            }
        }
    }
    if added == 0 {
        counts.none_added += 1;
    } else {
        detail.added.push(format!("{name}: {added} added, shortest segment {shortest:.1}"));
    }
    if moved == 0 {
        counts.none_moved += 1;
    } else {
        detail.moved.push(format!("{name}: {moved} moved"));
    }
}
