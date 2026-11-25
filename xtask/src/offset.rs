//! `cargo xtask offset`: Offset Curve, Make Stroke and path strokes on every glyph layer of the
//! corpus comparison set, against skia-pathops (docs/plans/M3.md › M3.34).
//!
//! | Operation | Ours | The reference (`crates/pathops/crosscheck/stroke.py`) |
//! |---|---|---|
//! | `grow` | Offset Curve +20 | the glyph united with a mitred stroke 40 wide |
//! | `shrink` | Offset Curve −10 | the glyph less a mitred stroke 20 wide |
//! | `stroke` | Make Stroke 30 at 50% | a mitred stroke 60 wide |
//! | `JOIN-CAP` | a path stroke 60 wide, on the paths left open | the same stroke |
//! | `make-CAP` | Make Stroke 30 at 50%, on the paths left open | the mitred stroke with that cap |
//!
//! Caps and joins the reference has no name for (round inset, axis-aligned, round full) are left
//! out. Results are unrounded. The first run writes the inputs to `OUT/layers.jsonl`; with the
//! script's output given as `--reference`, the run compares coverage and lists every layer that
//! differs, with what may explain it: a curve tighter than the offset, a corner past the mitre limit,
//! paths that cross.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use fontcraft_engine::model::Shape as ModelShape;
use fontcraft_geom::{BezPath, PathEl, Point, Vec2};
use fontcraft_pathops::filters::offset_curve::{CapStyle, OffsetOptions, offset_curve};
use fontcraft_pathops::{Cap, Contour, CoverageDiff, Join, StrokeStyle, coverage_difference, stroke_outline, union};
use serde_json::{Value, json};

use crate::overlap::contour_of;

/// The distances stroke.py's defaults match.
const GROW: f64 = 20.0;
const SHRINK: f64 = 10.0;
const STROKE: f64 = 30.0;
/// The § 5 tolerance: at most this much coverage difference in one pixel, and this share of the ink.
const MAX_PIXEL: f64 = 0.5;
const MAX_SHARE: f64 = 0.005;
/// An operation that takes longer than this is reported as hung.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Points sampled on a curve for its tightest turn.
const SAMPLES: usize = 32;
/// Layers listed per operation in the summary.
const LIST: usize = 25;

struct Row {
    source: String,
    glyph: String,
    layer: String,
    contours: Vec<Contour>,
    /// The tightest radius of its curves, in units.
    radius: f64,
    /// The sharpest corner: the angle between its two segments, in degrees.
    corner: f64,
    crosses: bool,
}

impl Row {
    fn name(&self) -> String {
        format!("{} › {} › {}", self.source, self.glyph, self.layer)
    }

    fn open(&self) -> Vec<Contour> {
        self.contours.iter().map(opened).collect()
    }

    /// What may explain a difference at offset `d`.
    fn causes(&self, d: f64) -> String {
        let mut out = Vec::new();
        if self.radius < d {
            out.push(format!("a curve of radius {:.1}", self.radius));
        }
        // A mitre reaches 1 / sin(half the angle) times the offset; the limit is 4 times.
        if self.corner < 2.0 * (0.25f64).asin().to_degrees() {
            out.push(format!("a corner of {:.1}°", self.corner));
        }
        if self.crosses {
            out.push("paths that overlap".to_string());
        }
        if out.is_empty() { "no known cause".to_string() } else { out.join(", ") }
    }
}

/// A closed contour as an open one from its start around to its last node.
fn opened(c: &Contour) -> Contour {
    let first = c.nodes.iter().position(|n| n.on_curve()).unwrap_or(0);
    let last = c.nodes.iter().rposition(|n| n.on_curve()).unwrap_or(0);
    Contour::new(false, c.nodes.get(first..=last).map(<[_]>::to_vec).unwrap_or_default())
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let mut out_dir = root.join("target/offset");
    let mut reference: Option<PathBuf> = None;
    let mut filter: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match *a {
            "--out" => out_dir = PathBuf::from(it.next().ok_or("--out needs a folder")?),
            "--reference" => reference = Some(PathBuf::from(it.next().ok_or("--reference needs a file")?)),
            "--match" => filter = Some(it.next().ok_or("--match needs text")?.to_string()),
            other => return Err(format!("unknown argument `{other}`; usage: cargo xtask offset [--out DIR] [--reference FILE] [--match TEXT]")),
        }
    }
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let corpus = root.join("corpus");
    let sources = fontcraft_oracle::corpus::discover(&corpus)?;
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for source in &sources {
        let rel = source.strip_prefix(&corpus).unwrap_or(source).display().to_string();
        if filter.as_ref().is_some_and(|f| !rel.contains(f.as_str())) {
            continue;
        }
        let font = match catch_unwind(|| fontcraft_engine::io::open(source, None)) {
            Ok(Ok((font, _))) => font,
            Ok(Err(e)) => {
                failures.push(format!("{rel}: {e}"));
                continue;
            }
            Err(_) => {
                failures.push(format!("{rel}: panicked while loading"));
                continue;
            }
        };
        for glyph in font.glyphs.iter() {
            for layer in &glyph.layers {
                let contours: Vec<Contour> = layer
                    .shapes
                    .iter()
                    .filter_map(|s| if let ModelShape::Path(p) = s { Some(p) } else { None })
                    .filter(|p| p.closed && p.nodes.len() > 1 && p.to_bez().is_ok())
                    .map(contour_of)
                    .collect();
                if contours.is_empty() {
                    continue;
                }
                let paths = bez_of(&contours);
                let crosses = union(&paths).is_ok_and(|u| (area(&u) - area(&paths)).abs() > 1.0);
                let (radius, corner) = (tightest(&paths), sharpest(&paths));
                rows.push(Row { source: rel.clone(), glyph: glyph.name.clone(), layer: layer.id.clone(), contours, radius, corner, crosses });
            }
        }
    }
    let mut lines = String::new();
    for r in &rows {
        let line =
            json!({"source": r.source, "glyph": r.glyph, "layer": r.layer, "input": svg(&bez_of(&r.contours)), "open": svg(&bez_of(&r.open()))});
        writeln!(lines, "{line}").ok();
    }
    let layers = out_dir.join("layers.jsonl");
    std::fs::write(&layers, lines).map_err(|e| format!("{}: {e}", layers.display()))?;

    let mut s = format!("sources {} (failed to load {}), layers {}\n", sources.len(), failures.len(), rows.len());
    for f in &failures {
        writeln!(s, "  load failure: {f}").ok();
    }
    let reference = match &reference {
        Some(p) => read_reference(p)?,
        None => References::new(),
    };
    for op in operations() {
        s.push_str(&report(&rows, &reference, &op));
    }
    std::panic::set_hook(default_hook);
    print!("{s}");
    std::fs::write(out_dir.join("summary.txt"), s).map_err(|e| e.to_string())?;
    println!("inputs: {}", layers.display());
    Ok(())
}

/// One operation: its name, the reference's, and how ours is run.
struct Operation {
    name: String,
    reference: String,
    distance: f64,
    run: Run,
}

#[derive(Clone, Copy)]
enum Run {
    /// Offset Curve on the closed contours, or Make Stroke on them or on the open ones.
    Filter {
        options: OffsetOptions,
        open: bool,
    },
    Stroke(StrokeStyle),
}

fn operations() -> Vec<Operation> {
    let plain =
        |d: f64| OffsetOptions { horizontal: d, vertical: d, stroke: false, auto: false, position: 50.0, compatible: false, cap: CapStyle::Flat };
    let made = |cap: CapStyle| OffsetOptions { stroke: true, cap, ..plain(STROKE) };
    let closed = |name: &str, options: OffsetOptions| Operation {
        name: name.into(),
        reference: name.into(),
        distance: options.horizontal.abs(),
        run: Run::Filter { options, open: false },
    };
    let mut out = vec![closed("grow", plain(GROW)), closed("shrink", plain(-SHRINK)), closed("stroke", made(CapStyle::Flat))];
    for join in [Join::Miter, Join::Round, Join::Bevel] {
        for cap in [Cap::Butt, Cap::Round, Cap::Square] {
            let name = format!("{}-{}", join.name(), cap.name());
            let style = StrokeStyle { join, cap_start: cap, cap_end: cap, ..StrokeStyle::new(2.0 * STROKE) };
            out.push(Operation { reference: name.clone(), name, distance: STROKE, run: Run::Stroke(style) });
        }
    }
    for (cap, word) in [(CapStyle::Flat, "butt"), (CapStyle::Round, "round"), (CapStyle::Square, "square")] {
        out.push(Operation {
            name: format!("make-{word}"),
            reference: format!("miter-{word}"),
            distance: STROKE,
            run: Run::Filter { options: made(cap), open: true },
        });
    }
    out
}

/// Our result and the time it took, on a thread of its own so that a hang is reported. A hung
/// thread is left behind.
fn ours(row: &Row, run: Run) -> Result<(Vec<BezPath>, f64), String> {
    let contours = match run {
        Run::Filter { open: false, .. } => row.contours.clone(),
        _ => row.open(),
    };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let start = Instant::now();
            let out = match run {
                Run::Filter { options, .. } => offset_curve(&contours, &options, 0.0).map(|c| bez_of(&c)),
                Run::Stroke(style) => {
                    let mut path = BezPath::new();
                    for b in bez_of(&contours) {
                        path.extend(b);
                    }
                    stroke_outline(&path, &style)
                }
            };
            out.map(|o| (o, start.elapsed().as_secs_f64() * 1e3)).map_err(|e| e.to_string())
        }));
        let result = result.unwrap_or_else(|p| Err(format!("panicked: {}", fontcraft_engine::guard::panic_message(p.as_ref()))));
        let _ = tx.send(result);
    });
    rx.recv_timeout(TIMEOUT).unwrap_or_else(|_| Err("hung".to_string()))
}

fn report(rows: &[Row], reference: &References, op: &Operation) -> String {
    let (mut done, mut equal, mut times) = (0usize, 0usize, Vec::new());
    let (mut bad, mut failed, mut worst) = (Vec::new(), Vec::new(), (0.0f64, 0.0f64));
    let mut causes: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows {
        let (out, ms) = match ours(r, op.run) {
            Ok(x) => x,
            Err(e) => {
                failed.push(format!("{}: {e}", r.name()));
                continue;
            }
        };
        times.push(ms);
        let key = (r.source.clone(), r.glyph.clone(), r.layer.clone(), op.reference.clone());
        let theirs = match reference.get(&key) {
            Some(Ok(p)) => p,
            Some(Err(e)) => {
                failed.push(format!("{}: reference: {e}", r.name()));
                continue;
            }
            None => continue,
        };
        done += 1;
        match coverage_difference(theirs, &out) {
            Ok(d) => {
                let share = d.total / d.ink.max(1.0);
                worst = (worst.0.max(d.max_pixel), worst.1.max(share));
                if d.within(MAX_PIXEL, MAX_SHARE) {
                    equal += 1;
                } else {
                    let cause = r.causes(op.distance);
                    let kind = cause.split(" of ").next().unwrap_or(&cause).to_string();
                    *causes.entry(kind).or_default() += 1;
                    bad.push((share, format!("{}: {} ({cause})", r.name(), describe(&d))));
                }
            }
            Err(e) => bad.push((f64::INFINITY, format!("{}: {e}", r.name()))),
        }
    }
    times.sort_by(f64::total_cmp);
    let pick = |p: f64| times.get(((times.len() as f64 - 1.0) * p).round().max(0.0) as usize).copied().unwrap_or(0.0);
    let mut s = String::new();
    writeln!(
        s,
        "\n{}: ran {}, failed {}; time per layer median {:.2} ms, p99 {:.2} ms, max {:.2} ms",
        op.name,
        times.len(),
        failed.len(),
        pick(0.5),
        pick(0.99),
        pick(1.0)
    )
    .ok();
    if done > 0 {
        writeln!(
            s,
            "  {equal} of {done} equal to the reference ({:.2}%); worst {:.3} per pixel, {:.3}% of the ink; causes {causes:?}",
            100.0 * equal as f64 / done as f64,
            worst.0,
            100.0 * worst.1
        )
        .ok();
    }
    bad.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (what, list) in [("failed", failed), ("differs", bad.into_iter().map(|b| b.1).collect())] {
        if !list.is_empty() {
            writeln!(s, "  {what} ({}):", list.len()).ok();
        }
        for line in list.iter().take(LIST) {
            writeln!(s, "    {line}").ok();
        }
    }
    s
}

type References = HashMap<(String, String, String, String), Result<Vec<BezPath>, String>>;

fn read_reference(path: &Path) -> Result<References, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = References::new();
    for line in text.lines() {
        let v: Value = serde_json::from_str(line).map_err(|e| format!("{}: {e}", path.display()))?;
        let key = |k: &str| v[k].as_str().map(str::to_owned).ok_or(format!("a reference line has no {k}"));
        let result = match (v["d"].as_str(), v["error"].as_str()) {
            (Some(d), _) => BezPath::from_svg(d).map(|p| vec![p]).map_err(|e| format!("unreadable path: {e}")),
            (None, e) => Err(e.unwrap_or("no result").to_string()),
        };
        out.insert((key("source")?, key("glyph")?, key("layer")?, key("op")?), result);
    }
    Ok(out)
}

fn bez_of(contours: &[Contour]) -> Vec<BezPath> {
    contours.iter().filter_map(|c| c.to_bez().ok()).filter(|b| b.elements().len() > 1).collect()
}

fn svg(paths: &[BezPath]) -> String {
    paths.iter().map(BezPath::to_svg).collect::<Vec<_>>().join(" ")
}

fn area(paths: &[BezPath]) -> f64 {
    use fontcraft_geom::Shape as _;
    paths.iter().map(|p| p.area()).sum()
}

fn describe(d: &CoverageDiff) -> String {
    format!("max px {:.3}, total {:.1} u² of {:.0} u² ink ({:.3}%)", d.max_pixel, d.total, d.ink, 100.0 * d.total / d.ink.max(1.0))
}

/// The segments of `paths` as cubics' points, lines as two points.
fn segments(path: &BezPath) -> Vec<Vec<Point>> {
    let mut out: Vec<Vec<Point>> = Vec::new();
    let (mut start, mut last) = (Point::ZERO, Point::ZERO);
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => (start, last) = (p, p),
            PathEl::LineTo(p) => {
                out.push(vec![last, p]);
                last = p;
            }
            PathEl::QuadTo(a, p) => {
                out.push(vec![last, last + (a - last) * (2.0 / 3.0), p + (a - p) * (2.0 / 3.0), p]);
                last = p;
            }
            PathEl::CurveTo(a, b, p) => {
                out.push(vec![last, a, b, p]);
                last = p;
            }
            PathEl::ClosePath => {
                out.push(vec![last, start]);
                last = start;
            }
        }
    }
    out.retain(|s| s.iter().any(|p| p.distance(s[0]) > 1e-6));
    out
}

fn tightest(paths: &[BezPath]) -> f64 {
    let mut least = f64::INFINITY;
    for s in paths.iter().flat_map(segments).filter(|s| s.len() == 4) {
        let (d0, d1, d2) = (s[1] - s[0], s[2] - s[1], s[3] - s[2]);
        for i in 0..=SAMPLES {
            let t = i as f64 / SAMPLES as f64;
            let m = 1.0 - t;
            let v = (d0 * (m * m) + d1 * (2.0 * m * t) + d2 * (t * t)) * 3.0;
            let a = ((d1 - d0) * m + (d2 - d1) * t) * 6.0;
            let cross = v.cross(a).abs();
            if cross > 1e-9 {
                least = least.min(v.hypot().powi(3) / cross);
            }
        }
    }
    least
}

fn sharpest(paths: &[BezPath]) -> f64 {
    let unit = |v: Vec2| (v.hypot() > 1e-9).then(|| v.normalize());
    let mut least: f64 = 180.0;
    for path in paths {
        let segs = segments(path);
        for (i, s) in segs.iter().enumerate() {
            let next = &segs[(i + 1) % segs.len()];
            let end = s[s.len() - 1];
            let Some(a) = s.iter().rev().find_map(|p| unit(end - *p)) else { continue };
            let Some(b) = next.iter().find_map(|p| unit(*p - next[0])) else { continue };
            least = least.min(180.0 - a.cross(b).atan2(a.dot(b)).abs().to_degrees());
        }
    }
    least
}
