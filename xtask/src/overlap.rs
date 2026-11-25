//! `cargo xtask overlap`: overlap removal (`fontcraft_pathops::union`) on every glyph layer of
//! the corpus comparison set, measured as `docs/plans/M3.md` § 5 asks.
//!
//! For each layer with closed paths (components and open paths are left out) it records the
//! outcome (result, error, panic or hang), the time of the operation, the coverage difference
//! between input and result, whether a second run changes the result, whether the result's
//! orientation is consistent, and whether it gained on-curve points. It writes the inputs to
//! `OUT/layers.jsonl`, for `crates/pathops/crosscheck/reference.py`; given that script's output
//! with `--reference` (once per operation), it also compares each result with the reference's.
//!
//! `--editor` runs the command's contour-level path (`fontcraft_pathops::remove_overlap`, results
//! on the grid) in place of the path-level union, and also reports start points kept or moved and
//! merge points left on straight lines.
//!
//! Each layer is also paired with a copy of itself moved by [`SHIFT`] and with a band at half the
//! x-height, the second operands of the difference and intersection references.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use fontcraft_engine::model::{Font, NodeKind as ModelKind, Path as ModelPath, Shape as ModelShape, metric};
use fontcraft_geom::{Affine, BezPath, PathEl, Point, Rect, Shape};
use fontcraft_pathops::{
    BoolOp, Contour, CoverageDiff, Node, NodeKind, OpenPaths, OverlapOptions, boolean, contours_from_bez, coverage_difference, remove_overlap,
    round_to_grid, union,
};
use serde_json::{Value, json};

/// The § 5 tolerance: at most this much coverage difference in one pixel…
const MAX_PIXEL: f64 = 0.5;
/// …and this share of the ink in total.
const MAX_SHARE: f64 = 0.005;
/// A layer whose operation takes longer than this is reported as hung.
const TIMEOUT: Duration = Duration::from_secs(10);
/// How far the second operand of the "shift" pair is moved, right and up.
const SHIFT: f64 = 37.0;
/// Half the height of the "band" pair's bar, and how far it reaches past the layer's sides.
const BAND_HALF_HEIGHT: f64 = 40.0;
const BAND_MARGIN: f64 = 50.0;
/// The grid `--editor` and the pairs round results to.
const GRID: f64 = 1.0;
/// A point this close, in units, to the line through its neighbours is on that line.
const ON_LINE: f64 = 0.01;
/// Layers listed per category in the summary.
const LIST: usize = 25;

enum Outcome {
    Done { out: Vec<BezPath>, again: Option<Vec<BezPath>>, micros: f64 },
    Error(String),
    Panic(String),
    Hang,
}

struct Row {
    source: String,
    glyph: String,
    layer: String,
    input: Vec<BezPath>,
    contours: Vec<Contour>,
    shift: Vec<BezPath>,
    band: BezPath,
    outcome: Outcome,
}

impl Row {
    fn name(&self) -> String {
        format!("{} › {} › {}", self.source, self.glyph, self.layer)
    }

    fn pair(&self, name: &str) -> Vec<BezPath> {
        if name == "band" { vec![self.band.clone()] } else { self.shift.clone() }
    }
}

/// A model path as the contour the editor's commands work on.
pub fn contour_of(path: &ModelPath) -> Contour {
    let kind = |k: ModelKind| match k {
        ModelKind::Line => NodeKind::Line,
        ModelKind::Curve => NodeKind::Curve,
        ModelKind::QCurve => NodeKind::QCurve,
        ModelKind::OffCurve => NodeKind::OffCurve,
    };
    let nodes = path.nodes.iter().map(|n| Node { smooth: n.smooth, ..Node::new(n.point(), kind(n.kind)) }).collect();
    Contour::new(path.closed, nodes)
}

/// The bar of the "band" pair: across the whole layer, at half the master's x-height.
fn band(font: &Font, master: &str, input: &[BezPath]) -> BezPath {
    let x_height = font.master(master).map_or(500.0, |m| font.metric_pos(m, metric::X_HEIGHT, 500.0));
    let b = input.iter().map(BezPath::bounding_box).reduce(|a, b| a.union(b)).unwrap_or_default();
    let y = x_height / 2.0;
    Rect::new(b.x0 - BAND_MARGIN, y - BAND_HALF_HEIGHT, b.x1 + BAND_MARGIN, y + BAND_HALF_HEIGHT).to_path(0.1)
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let mut out_dir = root.join("target/overlap");
    let mut references: Vec<PathBuf> = Vec::new();
    let mut editor = false;
    let mut filter: Option<String> = None;
    let mut quadratic = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match *a {
            "--out" => out_dir = PathBuf::from(it.next().ok_or("--out needs a folder")?),
            "--reference" => references.push(PathBuf::from(it.next().ok_or("--reference needs a file")?)),
            "--editor" => editor = true,
            "--quadratic" => quadratic = true,
            "--match" => filter = Some(it.next().ok_or("--match needs text")?.to_string()),
            other => return Err(format!("overlap: unknown argument `{other}`")),
        }
    }
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let corpus = root.join("corpus");
    let sources = fontcraft_oracle::corpus::discover(&corpus)?;
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let mut rows = Vec::new();
    let mut load_failures = Vec::new();
    let (mut layers_total, mut skipped_empty, mut open_paths, mut bad_paths) = (0usize, 0usize, 0usize, 0usize);
    for source in &sources {
        let rel = source.strip_prefix(&corpus).unwrap_or(source).display().to_string();
        if filter.as_ref().is_some_and(|f| !rel.contains(f.as_str())) {
            continue;
        }
        let font = match catch_unwind(|| fontcraft_engine::io::open(source, None)) {
            Ok(Ok((font, _))) => font,
            Ok(Err(e)) => {
                load_failures.push(format!("{rel}: {e}"));
                continue;
            }
            Err(_) => {
                load_failures.push(format!("{rel}: panicked while loading"));
                continue;
            }
        };
        for glyph in font.glyphs.iter() {
            for layer in &glyph.layers {
                layers_total += 1;
                let mut input = Vec::new();
                let mut contours = Vec::new();
                for shape in &layer.shapes {
                    let ModelShape::Path(p) = shape else { continue };
                    if !p.closed {
                        open_paths += 1;
                        continue;
                    }
                    match p.to_bez() {
                        Ok(b) if b.elements().len() > 1 && quadratic => {
                            let q = fontcraft_geom::to_quadratic(&b, fontcraft_geom::CUBIC_TO_QUAD_TOLERANCE);
                            contours.extend(contours_from_bez(&q));
                            input.push(q);
                        }
                        Ok(b) if b.elements().len() > 1 => {
                            contours.push(contour_of(p));
                            input.push(b);
                        }
                        Ok(_) => {}
                        Err(_) => bad_paths += 1,
                    }
                }
                if input.is_empty() {
                    skipped_empty += 1;
                    continue;
                }
                let outcome = measure(input.clone(), editor.then(|| contours.clone()));
                let shift = input.iter().map(|p| Affine::translate((SHIFT, SHIFT)) * p.clone()).collect();
                let band = band(&font, &layer.master, &input);
                rows.push(Row { source: rel.clone(), glyph: glyph.name.clone(), layer: layer.id.clone(), input, contours, shift, band, outcome });
            }
        }
    }
    std::panic::set_hook(default_hook);

    let mut layers_file = String::new();
    for (i, r) in rows.iter().enumerate() {
        let line = json!({"i": i, "source": r.source, "glyph": r.glyph, "layer": r.layer, "input": svg(&r.input), "shift": svg(&r.shift), "band": r.band.to_svg()});
        writeln!(layers_file, "{line}").ok();
    }
    let layers_path = out_dir.join("layers.jsonl");
    std::fs::write(&layers_path, layers_file).map_err(|e| format!("{}: {e}", layers_path.display()))?;

    let mut reference = References::new();
    for p in &references {
        read_reference(p, &mut reference)?;
    }
    let mut report = summarise(&rows, (!reference.is_empty()).then_some(&reference));
    if editor {
        report.push_str(&editor_report(&rows));
    }
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    for op in [BoolOp::Difference, BoolOp::Intersection] {
        report.push_str(&pair_report(&rows, &reference, op));
    }
    std::panic::set_hook(hook);
    let mut head = format!(
        "sources {} (failed to load {}), layers {layers_total}, measured {}, without closed paths {skipped_empty}, open paths left out {open_paths}, unreadable paths {bad_paths}\n",
        sources.len(),
        load_failures.len(),
        rows.len()
    );
    for f in &load_failures {
        writeln!(head, "  load failure: {f}").ok();
    }
    print!("{head}{report}");
    std::fs::write(out_dir.join("summary.txt"), format!("{head}{report}")).map_err(|e| e.to_string())?;
    println!("inputs: {}", layers_path.display());
    Ok(())
}

fn bez_of(contours: &[Contour]) -> Vec<BezPath> {
    contours.iter().filter_map(|c| c.to_bez().ok()).collect()
}

/// Runs the union, and the union of its result, on a thread of its own so that a hang is
/// reported instead of stopping the run. A hung thread is left behind. With `contours` it is the
/// editor's contour-level operation.
fn measure(input: Vec<BezPath>, contours: Option<Vec<Contour>>) -> Outcome {
    let options = OverlapOptions { grid: GRID, open_paths: OpenPaths::Remove };
    timed(move || {
        let start = Instant::now();
        if let Some(contours) = contours {
            let first = remove_overlap(&contours, options);
            let micros = start.elapsed().as_secs_f64() * 1e6;
            return match first {
                Ok(out) => {
                    let again = remove_overlap(&out, options).ok().map(|a| bez_of(&a));
                    Outcome::Done { out: bez_of(&out), again, micros }
                }
                Err(e) => Outcome::Error(e.to_string()),
            };
        }
        let first = union(&input);
        let micros = start.elapsed().as_secs_f64() * 1e6;
        match first {
            Ok(out) => {
                let again = union(&out).ok();
                Outcome::Done { out, again, micros }
            }
            Err(e) => Outcome::Error(e.to_string()),
        }
    })
}

fn timed(job: impl FnOnce() -> Outcome + Send + 'static) -> Outcome {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let outcome = catch_unwind(AssertUnwindSafe(job));
        let outcome = outcome.unwrap_or_else(|p| Outcome::Panic(fontcraft_engine::guard::panic_message(p.as_ref())));
        let _ = tx.send(outcome);
    });
    rx.recv_timeout(TIMEOUT).unwrap_or(Outcome::Hang)
}

fn svg(paths: &[BezPath]) -> String {
    paths.iter().map(BezPath::to_svg).collect::<Vec<_>>().join(" ")
}

/// Reference results by source, glyph, layer and operation (with the second operand's name): the
/// result or the reference's error, and the time.
type References = HashMap<(String, String, String, String), (Result<Vec<BezPath>, String>, f64)>;

/// The operation part of a [`References`] key.
fn operation(op: BoolOp, pair: &str) -> String {
    match op {
        BoolOp::Union => "union ".to_string(),
        BoolOp::Difference => format!("difference {pair}"),
        BoolOp::Intersection => format!("intersection {pair}"),
    }
}

fn read_reference(path: &Path, out: &mut References) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    for line in text.lines() {
        let v: Value = serde_json::from_str(line).map_err(|e| format!("{}: {e}", path.display()))?;
        let key = |k: &str| v[k].as_str().map(str::to_owned).ok_or(format!("a reference line has no {k}"));
        let op = format!("{} {}", v["op"].as_str().unwrap_or("union"), v["pair"].as_str().unwrap_or(""));
        let key = (key("source")?, key("glyph")?, key("layer")?, op);
        let ms = v["ms"].as_f64().unwrap_or(0.0);
        let result = match (v["d"].as_str(), v["error"].as_str()) {
            (Some(d), _) => BezPath::from_svg(d).map(|p| vec![p]).map_err(|e| format!("unreadable reference path: {e}")),
            (None, Some(e)) => Err(e.to_string()),
            (None, None) => Err("no result".into()),
        };
        out.insert(key, (result, ms));
    }
    Ok(())
}

/// The points of on-curve nodes between two straight segments that run on along one line.
fn flat_points(c: &Contour) -> Vec<Point> {
    let n = c.nodes.len();
    if !c.closed || n < 3 {
        return Vec::new();
    }
    let line = |i: usize| c.nodes[i % n].kind == NodeKind::Line;
    (0..n)
        .filter(|&i| line(i) && line(i + 1))
        .filter_map(|i| {
            let (a, b, d) = (c.nodes[(i + n - 1) % n].point, c.nodes[i].point, c.nodes[(i + 1) % n].point);
            let chord = d - a;
            let off = if chord.hypot() > 0.0 { (b - a).cross(chord).abs() / chord.hypot() } else { f64::INFINITY };
            ((b - a).dot(d - b) > 0.0 && off <= ON_LINE).then_some(b)
        })
        .collect()
}

fn place(p: Point) -> (i64, i64) {
    let p = round_to_grid(p, GRID);
    ((p.x * 1024.0).round() as i64, (p.y * 1024.0).round() as i64)
}

/// Start points and merge points of the editor's contour-level operation.
fn editor_report(rows: &[Row]) -> String {
    let (mut kept, mut moved, mut fresh) = (0usize, Vec::new(), 0usize);
    let (mut left, mut left_points, mut starts_on_lines) = (Vec::new(), 0usize, 0usize);
    let mut s = String::new();
    for r in rows {
        let Ok(out) = remove_overlap(&r.contours, OverlapOptions { grid: GRID, open_paths: OpenPaths::Remove }) else { continue };
        let start_of = |c: &Contour| c.nodes.iter().find(|n| n.on_curve()).map(|n| place(n.point));
        let starts: HashSet<(i64, i64)> = r.contours.iter().filter_map(start_of).collect();
        let drawn: HashSet<(i64, i64)> = r.contours.iter().flat_map(flat_points).map(place).collect();
        for c in &out {
            let passes = c.nodes.iter().any(|n| n.on_curve() && starts.contains(&place(n.point)));
            let starts_there = start_of(c).is_some_and(|p| starts.contains(&p));
            match (passes, starts_there) {
                (true, true) => kept += 1,
                (true, false) => moved.push(r.name()),
                (false, _) => fresh += 1,
            }
            let extra: Vec<Point> = flat_points(c).into_iter().filter(|p| !drawn.contains(&place(*p))).collect();
            let at_start = extra.iter().filter(|p| start_of(c) == Some(place(**p))).count();
            starts_on_lines += at_start;
            if extra.len() > at_start {
                left_points += extra.len() - at_start;
                left.push(format!("{}: {:?}", r.name(), extra.first()));
            }
        }
    }
    writeln!(s, "\neditor: result contours that start where an input contour did {kept}, moved {}, through no input start {fresh}", moved.len()).ok();
    writeln!(
        s,
        "editor: merge points left on straight lines {left_points} (on {} contours), kept starts on straight lines {starts_on_lines}",
        left.len()
    )
    .ok();
    for (title, list) in [("start moved", moved), ("merge points left", left)] {
        if !list.is_empty() {
            writeln!(s, "\n{title} ({}):", list.len()).ok();
        }
        for line in list.iter().take(LIST) {
            writeln!(s, "  {line}").ok();
        }
    }
    s
}

/// `op` on every layer and its two second operands, against the reference's result.
fn pair_report(rows: &[Row], reference: &References, op: BoolOp) -> String {
    let mut s = String::new();
    let (mut done, mut equal, mut same, mut more, mut fewer) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let (mut bad, mut counts, mut failed, mut missing) = (Vec::new(), Vec::new(), Vec::new(), 0usize);
    let mut worst = (0.0f64, 0.0f64);
    for r in rows {
        for pair in ["shift", "band"] {
            let key = (r.source.clone(), r.glyph.clone(), r.layer.clone(), operation(op, pair));
            let Some((result, _)) = reference.get(&key) else {
                missing += 1;
                continue;
            };
            let name = format!("{} ({pair})", r.name());
            let (a, b) = (r.contours.clone(), r.pair(pair).iter().flat_map(contours_from_bez).collect::<Vec<_>>());
            let outcome = timed(move || match boolean(&a, &b, op, GRID) {
                Ok(out) => Outcome::Done { out: bez_of(&out), again: None, micros: 0.0 },
                Err(e) => Outcome::Error(e.to_string()),
            });
            let out = match outcome {
                Outcome::Done { out, .. } => out,
                Outcome::Error(e) => {
                    failed.push(format!("{name}: {e}"));
                    continue;
                }
                Outcome::Panic(e) => {
                    failed.push(format!("{name}: panicked: {e}"));
                    continue;
                }
                Outcome::Hang => {
                    failed.push(format!("{name}: hung"));
                    continue;
                }
            };
            let theirs = match result {
                Ok(p) => p,
                Err(e) => {
                    failed.push(format!("{name}: reference: {e}"));
                    continue;
                }
            };
            done += 1;
            match coverage_difference(theirs, &out) {
                Ok(d) => {
                    worst = (worst.0.max(d.max_pixel), worst.1.max(d.total / d.ink.max(1.0)));
                    if d.within(MAX_PIXEL, MAX_SHARE) {
                        equal += 1;
                    } else {
                        bad.push((d.total / d.ink.max(1.0), format!("{name}: {}", describe(&d))));
                    }
                }
                Err(e) => bad.push((f64::INFINITY, format!("{name}: {e}"))),
            }
            let (ours, refs) = (on_curve(&out), on_curve(theirs));
            match ours.cmp(&refs) {
                std::cmp::Ordering::Equal => same += 1,
                std::cmp::Ordering::Greater => {
                    more += 1;
                    counts.push(format!("{name}: {ours}, reference {refs}"));
                }
                std::cmp::Ordering::Less => {
                    fewer += 1;
                    counts.push(format!("{name}: {ours}, reference {refs}"));
                }
            }
        }
    }
    let title = if op == BoolOp::Difference { "difference" } else { "intersection" };
    if done + failed.len() == 0 {
        return s;
    }
    bad.sort_by(|a, b| b.0.total_cmp(&a.0));
    writeln!(s, "\n{title}: {equal} of {done} equal to the reference, {} failed, {missing} without a reference result", failed.len()).ok();
    writeln!(s, "  worst pair: {:.4} per pixel, {:.4}% of its ink", worst.0, 100.0 * worst.1).ok();
    writeln!(s, "  on-curve points: {same} the same ({:.2}%), {more} more, {fewer} fewer", 100.0 * same as f64 / done.max(1) as f64).ok();
    let lists = [("failed", failed), ("differs from the reference", bad.into_iter().map(|x| x.1).collect()), ("different point counts", counts)];
    for (what, list) in lists {
        if !list.is_empty() {
            writeln!(s, "\n{title}, {what} ({}):", list.len()).ok();
        }
        for line in list.iter().take(LIST) {
            writeln!(s, "  {line}").ok();
        }
    }
    s
}

/// On-curve points of closed contours: the segments of each loop, counting an implied closing
/// line once and an explicit one (which ends on the start) not again.
fn on_curve(paths: &[BezPath]) -> usize {
    let mut n = 0;
    for p in paths {
        let (mut start, mut last) = (None, None);
        for el in p.elements() {
            match el {
                PathEl::MoveTo(q) => {
                    start = Some(*q);
                    last = Some(*q);
                }
                PathEl::LineTo(q) | PathEl::QuadTo(_, q) | PathEl::CurveTo(_, _, q) => {
                    n += 1;
                    last = Some(*q);
                }
                PathEl::ClosePath => {
                    if last != start {
                        n += 1;
                    }
                    last = start;
                }
            }
        }
    }
    n
}

fn signed_area(paths: &[BezPath]) -> f64 {
    paths.iter().map(|p| p.area()).sum()
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

fn describe(d: &CoverageDiff) -> String {
    format!("max px {:.3}, total {:.2} u² of {:.0} u² ink ({:.3}%)", d.max_pixel, d.total, d.ink, 100.0 * d.total / d.ink.max(1.0))
}

fn summarise(rows: &[Row], reference: Option<&References>) -> String {
    let mut s = String::new();
    let (mut errors, mut panics, mut hangs) = (Vec::new(), Vec::new(), Vec::new());
    let mut times = Vec::new();
    let (mut alone_ok, mut alone_bad) = (0usize, Vec::new());
    let (mut idem_exact, mut idem_cov, mut idem_bad) = (0usize, 0usize, Vec::new());
    let (mut orient_ok, mut orient_bad) = (0usize, Vec::new());
    let (mut gained, mut gained_points, mut gained_list) = (0usize, 0usize, Vec::new());
    let mut points_vs_reference = (0usize, 0usize, 0usize);
    let mut more_than_reference = Vec::new();
    let mut fewer_than_reference = Vec::new();
    let (mut ref_ok, mut ref_bad, mut ref_err, mut ref_times) = (0usize, Vec::new(), Vec::new(), Vec::new());
    let mut ref_vs_input_bad = 0usize;
    let mut slowest: Vec<(f64, String)> = Vec::new();
    // Largest per-pixel difference and largest share of the ink, over all layers.
    let (mut worst_alone, mut worst_ref) = ((0.0f64, 0.0f64), (0.0f64, 0.0f64));
    let (mut quad_in, mut quad_out, mut cubic_added) = (0usize, 0usize, 0usize);
    let has_cubic = |paths: &[BezPath]| paths.iter().any(|p| p.elements().iter().any(|e| matches!(e, PathEl::CurveTo(..))));
    let has_quad = |paths: &[BezPath]| paths.iter().any(|p| p.elements().iter().any(|e| matches!(e, PathEl::QuadTo(..))));
    for r in rows {
        let key = (r.source.clone(), r.glyph.clone(), r.layer.clone(), operation(BoolOp::Union, ""));
        let name = r.name();
        let Outcome::Done { out, again, micros } = &r.outcome else {
            match &r.outcome {
                Outcome::Error(e) => errors.push(format!("{name}: {e}")),
                Outcome::Panic(e) => panics.push(format!("{name}: {e}")),
                _ => hangs.push(name),
            }
            continue;
        };
        times.push(*micros);
        slowest.push((*micros, format!("{name} ({} input points)", on_curve(&r.input))));
        quad_in += usize::from(has_quad(&r.input));
        quad_out += usize::from(has_quad(out));
        cubic_added += usize::from(!has_cubic(&r.input) && has_cubic(out));
        if let Ok(d) = coverage_difference(&r.input, out) {
            worst_alone = (worst_alone.0.max(d.max_pixel), worst_alone.1.max(d.total / d.ink.max(1.0)));
        }
        match coverage_difference(&r.input, out) {
            Ok(d) if d.within(MAX_PIXEL, MAX_SHARE) => alone_ok += 1,
            Ok(d) => alone_bad.push((d.total / d.ink.max(1.0), format!("{name}: {}", describe(&d)))),
            Err(e) => alone_bad.push((f64::INFINITY, format!("{name}: {e}"))),
        }
        match again {
            Some(a) if a == out => {
                idem_exact += 1;
                idem_cov += 1;
            }
            Some(a) => match coverage_difference(out, a) {
                Ok(d) if d.max_pixel < 1e-6 && on_curve(a) == on_curve(out) => idem_cov += 1,
                Ok(d) => idem_bad.push(format!("{name}: {} → {} points, {}", on_curve(out), on_curve(a), describe(&d))),
                Err(e) => idem_bad.push(format!("{name}: {e}")),
            },
            None => idem_bad.push(format!("{name}: the second run failed")),
        }
        if let Ok(d) = coverage_difference(out, &[]) {
            let area = signed_area(out);
            if (area - d.ink).abs() <= 1.0f64.max(0.001 * d.ink) {
                orient_ok += 1;
            } else {
                orient_bad.push(format!("{name}: signed area {area:.1}, covered {:.1}", d.ink));
            }
        }
        let (pin, pout) = (on_curve(&r.input), on_curve(out));
        let pref = reference.and_then(|m| m.get(&key)).and_then(|(r, _)| r.as_ref().ok()).map(|r| on_curve(r));
        if pout > pin {
            gained += 1;
            gained_points += pout - pin;
            let pref = pref.map(|n| format!(", reference {n}")).unwrap_or_default();
            gained_list.push(format!("{name}: {pin} → {pout}{pref}"));
        }
        match pref {
            Some(n) if n < pout => {
                points_vs_reference.0 += 1;
                more_than_reference.push(format!("{name}: input {pin}, result {pout}, reference {n}"));
            }
            Some(n) if n == pout => points_vs_reference.1 += 1,
            Some(n) => {
                points_vs_reference.2 += 1;
                fewer_than_reference.push(format!("{name}: input {pin}, result {pout}, reference {n}"));
            }
            None => {}
        }
        if let Some(reference) = reference {
            match reference.get(&key) {
                Some((Ok(rp), ms)) => {
                    ref_times.push(ms * 1000.0);
                    if let Ok(d) = coverage_difference(rp, out) {
                        worst_ref = (worst_ref.0.max(d.max_pixel), worst_ref.1.max(d.total / d.ink.max(1.0)));
                    }
                    match coverage_difference(rp, out) {
                        Ok(d) if d.within(MAX_PIXEL, MAX_SHARE) => ref_ok += 1,
                        Ok(d) => {
                            let vs_input = coverage_difference(&r.input, rp).map(|x| x.within(MAX_PIXEL, MAX_SHARE)).unwrap_or(false);
                            if !vs_input {
                                ref_vs_input_bad += 1;
                            }
                            ref_bad.push((
                                d.total / d.ink.max(1.0),
                                format!("{name}: {} (reference vs input {})", describe(&d), if vs_input { "equal" } else { "differs" }),
                            ));
                        }
                        Err(e) => ref_bad.push((f64::INFINITY, format!("{name}: {e}"))),
                    }
                }
                Some((Err(e), _)) => ref_err.push(format!("{name}: {e}")),
                None => ref_err.push(format!("{name}: missing")),
            }
        }
    }
    let done = times.len();
    times.sort_by(f64::total_cmp);
    let mean = times.iter().sum::<f64>() / done.max(1) as f64;
    writeln!(s, "\noutcomes: {done} done, {} errors, {} panics, {} hangs (> {} s)", errors.len(), panics.len(), hangs.len(), TIMEOUT.as_secs()).ok();
    writeln!(
        s,
        "time per layer (µs): median {:.1}, mean {mean:.1}, p99 {:.1}, max {:.1}",
        percentile(&times, 0.5),
        percentile(&times, 0.99),
        times.last().copied().unwrap_or(0.0)
    )
    .ok();
    writeln!(s, "alone (result vs input, ≤ {MAX_PIXEL}/px and ≤ {}% of ink): {alone_ok} of {done}", MAX_SHARE * 100.0).ok();
    writeln!(s, "  worst layer: {:.4} per pixel, {:.4}% of its ink", worst_alone.0, 100.0 * worst_alone.1).ok();
    writeln!(s, "layers with quadratic input: {quad_in}; with quadratic segments in the result: {quad_out}; with cubic segments only in the result: {cubic_added}").ok();
    writeln!(s, "idempotent: {idem_exact} identical, {idem_cov} same coverage and points, {} not", idem_bad.len()).ok();
    writeln!(s, "orientation consistent (signed area = covered area): {orient_ok} of {done}").ok();
    writeln!(s, "layers that gained on-curve points: {gained} ({gained_points} points)").ok();
    if reference.is_some() {
        ref_times.sort_by(f64::total_cmp);
        writeln!(s, "against the reference: {ref_ok} equal of {}, {} differ ({ref_vs_input_bad} of them where the reference also differs from the input), {} reference errors", done, ref_bad.len(), ref_err.len()).ok();
        writeln!(s, "  worst layer: {:.4} per pixel, {:.4}% of its ink", worst_ref.0, 100.0 * worst_ref.1).ok();
        let (fewer, same, more) = points_vs_reference;
        let share = 100.0 * same as f64 / (same + fewer + more).max(1) as f64;
        writeln!(
            s,
            "on-curve points against the reference: {same} layers the same ({share:.2}%), {fewer} with more than the reference, {more} with fewer"
        )
        .ok();
        writeln!(
            s,
            "reference time per layer (µs): median {:.1}, p99 {:.1}, max {:.1}",
            percentile(&ref_times, 0.5),
            percentile(&ref_times, 0.99),
            ref_times.last().copied().unwrap_or(0.0)
        )
        .ok();
    }
    slowest.sort_by(|a, b| b.0.total_cmp(&a.0));
    alone_bad.sort_by(|a, b| b.0.total_cmp(&a.0));
    ref_bad.sort_by(|a, b| b.0.total_cmp(&a.0));
    let lists: [(&str, Vec<String>); 12] = [
        ("errors", errors),
        ("panics", panics),
        ("hangs", hangs),
        ("slowest", slowest.into_iter().map(|(t, n)| format!("{t:.0} µs {n}")).collect()),
        ("result differs from input", alone_bad.into_iter().map(|x| x.1).collect()),
        ("second run changes the result", idem_bad),
        ("inconsistent orientation", orient_bad),
        ("gained on-curve points", gained_list),
        ("more on-curve points than the reference", more_than_reference),
        ("fewer on-curve points than the reference", fewer_than_reference),
        ("differs from the reference", ref_bad.into_iter().map(|x| x.1).collect()),
        ("reference errors", ref_err),
    ];
    for (title, list) in lists {
        if list.is_empty() {
            continue;
        }
        writeln!(s, "\n{title} ({}):", list.len()).ok();
        for line in list.iter().take(LIST) {
            writeln!(s, "  {line}").ok();
        }
    }
    s
}
