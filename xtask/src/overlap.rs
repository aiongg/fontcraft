//! `cargo xtask overlap`: overlap removal (`fontcraft_pathops::union`) on every glyph layer of
//! the corpus comparison set, measured as `docs/plans/M3.md` § 5 asks.
//!
//! For each layer with closed paths (components and open paths are left out) it records the
//! outcome (result, error, panic or hang), the time of the operation, the coverage difference
//! between input and result, whether a second run changes the result, whether the result's
//! orientation is consistent, and whether it gained on-curve points. It writes the inputs to
//! `OUT/layers.jsonl`, for `crates/pathops/crosscheck/reference.py`; given that script's output
//! with `--reference`, it also compares each result with the reference's.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use fontcraft_engine::model::Shape as ModelShape;
use fontcraft_geom::{BezPath, PathEl, Shape};
use fontcraft_pathops::{CoverageDiff, coverage_difference, union};
use serde_json::{Value, json};

/// The § 5 tolerance: at most this much coverage difference in one pixel…
const MAX_PIXEL: f64 = 0.5;
/// …and this share of the ink in total.
const MAX_SHARE: f64 = 0.005;
/// A layer whose operation takes longer than this is reported as hung.
const TIMEOUT: Duration = Duration::from_secs(10);
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
    outcome: Outcome,
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let mut out_dir = root.join("target/overlap");
    let mut reference: Option<PathBuf> = None;
    let mut filter: Option<String> = None;
    let mut quadratic = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match *a {
            "--out" => out_dir = PathBuf::from(it.next().ok_or("--out needs a folder")?),
            "--reference" => reference = Some(PathBuf::from(it.next().ok_or("--reference needs a file")?)),
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
                for shape in &layer.shapes {
                    let ModelShape::Path(p) = shape else { continue };
                    if !p.closed {
                        open_paths += 1;
                        continue;
                    }
                    match p.to_bez() {
                        Ok(b) if b.elements().len() > 1 && quadratic => {
                            input.push(fontcraft_geom::to_quadratic(&b, fontcraft_geom::CUBIC_TO_QUAD_TOLERANCE))
                        }
                        Ok(b) if b.elements().len() > 1 => input.push(b),
                        Ok(_) => {}
                        Err(_) => bad_paths += 1,
                    }
                }
                if input.is_empty() {
                    skipped_empty += 1;
                    continue;
                }
                let outcome = measure(input.clone());
                rows.push(Row { source: rel.clone(), glyph: glyph.name.clone(), layer: layer.id.clone(), input, outcome });
            }
        }
    }
    std::panic::set_hook(default_hook);

    let mut layers_file = String::new();
    for (i, r) in rows.iter().enumerate() {
        let line = json!({"i": i, "source": r.source, "glyph": r.glyph, "layer": r.layer, "input": svg(&r.input)});
        writeln!(layers_file, "{line}").ok();
    }
    let layers_path = out_dir.join("layers.jsonl");
    std::fs::write(&layers_path, layers_file).map_err(|e| format!("{}: {e}", layers_path.display()))?;

    let reference = match &reference {
        Some(p) => Some(read_reference(p)?),
        None => None,
    };
    let report = summarise(&rows, reference.as_ref());
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

/// Runs the union, and the union of its result, on a thread of its own so that a hang is
/// reported instead of stopping the run. A hung thread is left behind.
fn measure(input: Vec<BezPath>) -> Outcome {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let start = Instant::now();
            let first = union(&input);
            let micros = start.elapsed().as_secs_f64() * 1e6;
            match first {
                Ok(out) => {
                    let again = union(&out).ok();
                    Outcome::Done { out, again, micros }
                }
                Err(e) => Outcome::Error(e.to_string()),
            }
        }));
        let outcome = outcome.unwrap_or_else(|p| Outcome::Panic(fontcraft_engine::guard::panic_message(p.as_ref())));
        let _ = tx.send(outcome);
    });
    rx.recv_timeout(TIMEOUT).unwrap_or(Outcome::Hang)
}

fn svg(paths: &[BezPath]) -> String {
    paths.iter().map(BezPath::to_svg).collect::<Vec<_>>().join(" ")
}

/// Reference results by source, glyph and layer: the result or the reference's error, and the time.
type References = HashMap<(String, String, String), (Result<Vec<BezPath>, String>, f64)>;

fn read_reference(path: &Path) -> Result<References, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = HashMap::new();
    for line in text.lines() {
        let v: Value = serde_json::from_str(line).map_err(|e| format!("{}: {e}", path.display()))?;
        let key = |k: &str| v[k].as_str().map(str::to_owned).ok_or(format!("a reference line has no {k}"));
        let key = (key("source")?, key("glyph")?, key("layer")?);
        let ms = v["ms"].as_f64().unwrap_or(0.0);
        let result = match (v["d"].as_str(), v["error"].as_str()) {
            (Some(d), _) => BezPath::from_svg(d).map(|p| vec![p]).map_err(|e| format!("unreadable reference path: {e}")),
            (None, Some(e)) => Err(e.to_string()),
            (None, None) => Err("no result".into()),
        };
        out.insert(key, (result, ms));
    }
    Ok(out)
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
    let (mut ref_ok, mut ref_bad, mut ref_err, mut ref_times) = (0usize, Vec::new(), Vec::new(), Vec::new());
    let mut ref_vs_input_bad = 0usize;
    let mut slowest: Vec<(f64, String)> = Vec::new();
    // Largest per-pixel difference and largest share of the ink, over all layers.
    let (mut worst_alone, mut worst_ref) = ((0.0f64, 0.0f64), (0.0f64, 0.0f64));
    let (mut quad_in, mut quad_out, mut cubic_added) = (0usize, 0usize, 0usize);
    let has_cubic = |paths: &[BezPath]| paths.iter().any(|p| p.elements().iter().any(|e| matches!(e, PathEl::CurveTo(..))));
    let has_quad = |paths: &[BezPath]| paths.iter().any(|p| p.elements().iter().any(|e| matches!(e, PathEl::QuadTo(..))));
    for r in rows {
        let key = (r.source.clone(), r.glyph.clone(), r.layer.clone());
        let name = format!("{} › {} › {}", r.source, r.glyph, r.layer);
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
            Some(_) => points_vs_reference.2 += 1,
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
        writeln!(s, "on-curve points against the reference: {same} layers the same, {fewer} with more than the reference, {more} with fewer").ok();
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
    let lists: [(&str, Vec<String>); 11] = [
        ("errors", errors),
        ("panics", panics),
        ("hangs", hangs),
        ("slowest", slowest.into_iter().map(|(t, n)| format!("{t:.0} µs {n}")).collect()),
        ("result differs from input", alone_bad.into_iter().map(|x| x.1).collect()),
        ("second run changes the result", idem_bad),
        ("inconsistent orientation", orient_bad),
        ("gained on-curve points", gained_list),
        ("more on-curve points than the reference", more_than_reference),
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
