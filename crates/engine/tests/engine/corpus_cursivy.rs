//! Measure (M3 plan § M3.25, Cursivy): on every upright corpus master with a vertical and a
//! horizontal stem, a Cursivy slant of 12° keeps the horizontal width of the H's left stem within
//! 2%, and the thickness of the O's strokes where they run diagonally within 3%.
//!
//! A master whose stem value is more than 25% off the thickness of its O's side is reported and
//! left out of the 3%: the correction is as good as the stems it reads. Masters of 1000 units per
//! em whose O is thinner than 40 units are reported and left out too, because the rounding to
//! whole units is then the larger part of the error.
//!
//! `cargo test -p fontcraft-engine --test engine corpus_cursivy:: -- --ignored --nocapture`.
//! `FONTCRAFT_CORPUS` names the corpus folder (default: the repository's `corpus/`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use fontcraft_engine::model::{Font, Shape, metric};
use fontcraft_geom::{BezPath, CubicBez, ParamCurve as _, ParamCurveNearest as _, PathSeg, Point, Shape as _, Vec2};
use serde_json::json;

const DEGREES: f64 = 12.0;
/// Thinnest stroke, in units, whose rounding leaves the 3% measurable.
const MIN_STROKE: f64 = 40.0;

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n == ".git" || n == "fontc") {
            continue;
        }
        if p.extension().is_some_and(|x| x == "glyphs" || x == "glyphspackage" || x == "ufo") {
            out.push(p);
        } else if p.is_dir() {
            sources(&p, out);
        }
    }
}

/// The outlines of the paths a glyph's master layer is made of; `None` with anything else in it.
fn outline(font: &Font, glyph: &str, master: &str) -> Option<Vec<BezPath>> {
    let layer = font.glyph(glyph)?.layer(master)?;
    layer
        .shapes
        .iter()
        .map(|s| match s {
            Shape::Path(p) if p.closed => p.to_bez().ok(),
            _ => None,
        })
        .collect()
}

/// The horizontal width of the first stem the line at `y` crosses.
fn stem_width(paths: &[BezPath], y: f64) -> Option<f64> {
    let mut xs: Vec<f64> = Vec::new();
    for seg in paths.iter().flat_map(BezPath::segments) {
        let PathSeg::Line(l) = seg else { continue };
        if (l.p0.y - y) * (l.p1.y - y) < 0.0 {
            xs.push(l.p0.x + (l.p1.x - l.p0.x) * (y - l.p0.y) / (l.p1.y - l.p0.y));
        }
    }
    xs.sort_by(f64::total_cmp);
    Some(xs.get(1)? - xs.first()?)
}

fn nearest(p: Point, path: &BezPath) -> f64 {
    path.segments().map(|s| s.nearest(p, 1e-6).distance_sq).fold(f64::INFINITY, f64::min).sqrt()
}

/// The outer and the inner outline of an O drawn as two contours of curves.
fn ring(paths: &[BezPath]) -> Option<(usize, usize)> {
    let [a, b] = paths else { return None };
    let cubic = |p: &BezPath| p.segments().any(|s| matches!(s, PathSeg::Cubic(_))) && !p.segments().any(|s| matches!(s, PathSeg::Quad(_)));
    (cubic(a) && cubic(b)).then(|| if a.area().abs() > b.area().abs() { (0, 1) } else { (1, 0) })
}

/// For each curve of the outer outline with a point where the stroke runs at 45°: the thickness
/// before and after.
fn diagonals(before: &[BezPath], after: &[BezPath], (o, i): (usize, usize)) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for (s, sa) in before[o].segments().zip(after[o].segments()) {
        let (PathSeg::Cubic(c), PathSeg::Cubic(ca)) = (s, sa) else { continue };
        let off = |t: f64| {
            let d = direction(c, t);
            if d == Vec2::ZERO { 1.0 } else { ((d.x.abs() - d.y.abs()) / d.hypot()).abs() }
        };
        let t = (2..=98).map(|k| f64::from(k) / 100.0).min_by(|a, b| off(*a).total_cmp(&off(*b))).unwrap();
        if off(t) < 0.1 {
            out.push((nearest(c.eval(t), &before[i]), nearest(ca.eval(t), &after[i])));
        }
    }
    out
}

fn direction(c: CubicBez, t: f64) -> Vec2 {
    let u = 1.0 - t;
    u * u * (c.p1 - c.p0) + 2.0 * u * t * (c.p2 - c.p1) + t * t * (c.p3 - c.p2)
}

#[test]
#[ignore = "reads every corpus source: the M3.25 measure"]
fn cursivy_keeps_stem_widths_and_diagonal_strokes() {
    let corpus = std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from);
    let mut all = Vec::new();
    sources(&corpus, &mut all);
    all.sort();
    let (mut masters, mut strokes, mut stems, mut failures) = (0usize, 0usize, 0usize, Vec::new());
    let (mut worst_stem, mut worst_stroke) = (0.0f64, 0.0f64);
    for path in &all {
        let name = path.strip_prefix(&corpus).unwrap_or(path).display().to_string();
        let mut s = Session::new();
        if s.execute("file.open", &json!({"path": path})).is_err() {
            continue;
        }
        let ids: Vec<(String, String)> = s.font().unwrap().masters.iter().map(|m| (m.id.clone(), m.name.clone())).collect();
        for (id, master_name) in ids {
            let font = s.font().unwrap().clone();
            let master = font.master(&id).unwrap();
            let first = |horizontal: bool| font.stem_widths(master, horizontal).into_iter().find(|v| *v > 0.0);
            let (Some(v), Some(_)) = (first(false), first(true)) else { continue };
            if font.metric(master, metric::ITALIC_ANGLE).is_some_and(|m| m.pos != 0.0) {
                continue;
            }
            let glyphs: Vec<&str> = ["H", "O", "o"].into_iter().filter(|g| outline(&font, g, &id).is_some_and(|p| !p.is_empty())).collect();
            if glyphs.is_empty() {
                continue;
            }
            masters += 1;
            let label = format!("{name} [{master_name}]");
            s.execute("path.transformations", &json!({"glyphs": glyphs, "layer": id, "slant": DEGREES, "cursivy": true}))
                .unwrap_or_else(|e| panic!("{label}: {e}"));
            let slanted = s.font().unwrap().clone();
            for g in glyphs {
                let (before, after) = (outline(&font, g, &id).unwrap(), outline(&slanted, g, &id).unwrap());
                if g == "H" {
                    let top = before.iter().map(|p| p.bounding_box().y1).fold(0.0, f64::max);
                    if let (Some(a), Some(b)) = (stem_width(&before, top * 0.25), stem_width(&after, top * 0.25)) {
                        stems += 1;
                        worst_stem = worst_stem.max((b / a - 1.0).abs());
                        if (b / a - 1.0).abs() > 0.02 {
                            failures.push(format!("{label} H: stem {a:.1} became {b:.1}"));
                        }
                    }
                    continue;
                }
                let Some(pair) = ring(&before) else { continue };
                let side = before[pair.1].bounding_box().x0 - before[pair.0].bounding_box().x0;
                if (v / side - 1.0).abs() > 0.25 || side < MIN_STROKE {
                    eprintln!("left out: {label} {g}: stem {v}, side {side:.1}");
                    continue;
                }
                for (a, b) in diagonals(&before, &after, pair) {
                    strokes += 1;
                    worst_stroke = worst_stroke.max((b / a - 1.0).abs());
                    if (b / a - 1.0).abs() > 0.03 {
                        failures.push(format!("{label} {g}: stroke {a:.1} became {b:.1}"));
                    }
                }
            }
        }
    }
    eprintln!(
        "{masters} masters with stems: {stems} stems of H, worst {:.2}%; {strokes} diagonal strokes, worst {:.2}%",
        100.0 * worst_stem,
        100.0 * worst_stroke
    );
    assert!(strokes > 0, "no master was measured: is the corpus there?");
    assert!(failures.is_empty(), "{failures:#?}");
}
