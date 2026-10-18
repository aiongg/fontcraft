//! Measure (`docs/plans/M13.md` § 5, auto-spacing against the corpus): for Inter Roman and
//! Oswald, every master, `spacing.autoSpace` with its defaults (every exported letter and figure,
//! calibrated on the font's own n, o, H and O) against the sidebearings the source stores.
//! Glyphs with a metrics key on that layer and layers under automatic alignment are left out, as
//! the measure asks.
//!
//! For each side the relative difference is |ours − stored| ÷ |stored| (sides whose stored value
//! is below 5 units in size are counted apart, as a relative difference means nothing there).
//! Target: the median relative difference is at most 10% for lowercase and for capitals, in
//! every master. The distribution per group (lowercase, capitals, figures, other letters) is
//! printed. Speed target: one master of Inter under 500 ms in a release build.
//!
//! `cargo test --release -p fontcraft-engine --test engine corpus_autospace:: -- --ignored --nocapture`.
//! `FONTCRAFT_CORPUS` names the corpus folder (default: the repository's `corpus/`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fontcraft_engine::Session;
use fontcraft_engine::model::{Font, Glyph, glyph_props};
use fontcraft_spacing::{Field, derives_metrics, effective_key, metric};
use serde_json::json;

fn corpus() -> PathBuf {
    std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from)
}

/// The report group of a glyph.
fn group(g: &Glyph, s: &Session) -> &'static str {
    let data = s.glyph_data().unwrap();
    let props = glyph_props(g, &data, false);
    match (props.category.as_deref(), props.case.as_deref()) {
        (Some("Number"), _) => "figures",
        (_, Some("upper")) => "capitals",
        (_, Some("lower")) => "lowercase",
        _ => "other letters",
    }
}

/// Glyph names whose layer `id` the measure compares: no metrics key on any horizontal field,
/// not aligned automatically, not empty.
fn compared(font: &Font, g: &Glyph, id: &str) -> Option<(f64, f64)> {
    let l = g.layers.iter().find(|l| l.id == id)?;
    if derives_metrics(font, l) || [Field::Left, Field::Right, Field::Width].iter().any(|f| effective_key(g, l, *f).is_some()) {
        return None;
    }
    Some((metric(font, l, Field::Left).ok()??, metric(font, l, Field::Right).ok()??))
}

#[derive(Default)]
struct Tally {
    relative: Vec<f64>,
    small: Vec<f64>,
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn percentile(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v[((v.len() - 1) as f64 * p).round() as usize]
}

/// Runs the measure on one source; returns the failures.
fn measure(path: &Path, speed_limit: Option<Duration>) -> Vec<String> {
    let mut failures = Vec::new();
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    let font = s.font().unwrap().clone();
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    for m in font.masters.iter() {
        let stored: BTreeMap<String, (f64, f64)> =
            font.glyphs.iter().filter_map(|g| compared(&font, g, &m.id).map(|v| (g.name.clone(), v))).collect();
        let t = Instant::now();
        let out = s.execute("spacing.autoSpace", &json!({"masters": [m.id]})).unwrap();
        let took = t.elapsed();
        s.execute("edit.undo", &json!({})).unwrap();
        let mut tallies: BTreeMap<&str, Tally> = BTreeMap::new();
        for (glyph, (left, right)) in &stored {
            let Some(v) = out["values"].get(glyph).and_then(|v| v.get(&m.id)) else { continue };
            let g = font.glyph(glyph).unwrap();
            let tally = tallies.entry(group(g, &s)).or_default();
            for (ours, theirs) in [(v["left"].as_f64().unwrap(), *left), (v["right"].as_f64().unwrap(), *right)] {
                let diff = (ours - theirs).abs();
                if theirs.abs() < 5.0 {
                    tally.small.push(diff);
                } else {
                    tally.relative.push(diff / theirs.abs());
                }
            }
        }
        println!("{name} / {}: {} glyphs compared, {:?}, areas {}", m.name, stored.len(), took, out["areas"][&m.id]);
        for (group, t) in &mut tallies {
            let med = median(&mut t.relative);
            println!(
                "  {group:14} sides {:5}  median {:5.1}%  p75 {:5.1}%  p90 {:5.1}%  | stored under 5 units: {} sides, median {:.0} units off",
                t.relative.len(),
                med * 100.0,
                percentile(&t.relative, 0.75) * 100.0,
                percentile(&t.relative, 0.9) * 100.0,
                t.small.len(),
                median(&mut t.small),
            );
            if matches!(*group, "lowercase" | "capitals") && med > 0.10 {
                failures.push(format!("{name} / {}: {group} median {:.1}% > 10%", m.name, med * 100.0));
            }
        }
        if let Some(limit) = speed_limit
            && took > limit
        {
            failures.push(format!("{name} / {}: {took:?} > {limit:?}", m.name));
        }
    }
    failures
}

#[test]
#[ignore = "corpus measure"]
fn auto_spacing_against_inter_and_oswald() {
    let root = corpus();
    let inter = root.join("inter/src/Inter-Roman.glyphspackage");
    let oswald = root.join("oswald/sources/Oswald.glyphs");
    if !inter.is_dir() || !oswald.is_file() {
        eprintln!("corpus not fetched: skipped");
        return;
    }
    // The speed target holds for optimised builds only.
    let limit = (!cfg!(debug_assertions)).then(|| Duration::from_millis(500));
    let mut failures = measure(&inter, limit);
    failures.extend(measure(&oswald, None));
    assert!(failures.is_empty(), "{failures:#?}");
}
