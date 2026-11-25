//! Measure (`docs/plans/M13.md` § 5, auto-spacing against the corpus): for Inter Roman and
//! Oswald, every master, `spacing.autoSpace` with its defaults (every exported letter and figure,
//! calibrated on the font's own n, o, v, H, O and V) against the sidebearings the source stores.
//! Glyphs with a metrics key on that layer and layers under automatic alignment are left out, as
//! the measure asks.
//!
//! For each side the relative difference is |ours − stored| ÷ |stored| (sides whose stored value
//! is below 5 units in size are counted apart, as a relative difference means nothing there).
//! Target: the median relative difference is at most 10% for lowercase and for capitals, in
//! every master, with the default model. The distribution per group (lowercase, capitals,
//! figures, other letters) is printed for the default and for `model: "area"`, with the median
//! without the control glyphs, and the 20 sides of lowercase and capitals furthest off with the
//! control side each took. Speed target: one master of Inter under 500 ms in a release build.
//!
//! Inter keeps few letters without keys (about 55 sides per group and master), mostly alternates
//! and phonetic letters, so the figures jump by tens of percent with a handful of glyphs. A second
//! table, without a target, therefore takes every letter and figure that is not aligned, its keys
//! removed first: the sidebearings the source's keys give are the designer's spacing too.
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
    /// Those of glyphs other than the controls.
    uncontrolled: Vec<f64>,
    small: Vec<f64>,
}

/// The glyphs the models calibrate on.
const CONTROLS: [&str; 6] = ["n", "o", "v", "H", "O", "V"];

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

/// Compares one model's values with `stored` on one master and prints the table; returns the
/// medians of lowercase and capitals, with the time taken.
fn compare(
    s: &mut Session,
    font: &Font,
    master: (&str, &str),
    stored: &BTreeMap<String, (f64, f64)>,
    model: &str,
    worst: usize,
) -> (BTreeMap<&'static str, f64>, Duration) {
    let (id, name) = master;
    let t = Instant::now();
    let out = s.execute("spacing.autoSpace", &json!({"masters": [id], "model": model})).unwrap();
    let took = t.elapsed();
    s.execute("edit.undo", &json!({})).unwrap();
    let mut tallies: BTreeMap<&'static str, Tally> = BTreeMap::new();
    let mut off: Vec<(f64, String)> = Vec::new();
    for (glyph, (left, right)) in stored {
        let Some(v) = out["values"].get(glyph).and_then(|v| v.get(id)) else { continue };
        let group = group(font.glyph(glyph).unwrap(), s);
        let tally = tallies.entry(group).or_default();
        for (side, theirs) in [("left", *left), ("right", *right)] {
            let ours = v[side].as_f64().unwrap();
            let diff = (ours - theirs).abs();
            if theirs.abs() < 5.0 {
                tally.small.push(diff);
                continue;
            }
            let relative = diff / theirs.abs();
            tally.relative.push(relative);
            if !CONTROLS.contains(&glyph.as_str()) {
                tally.uncontrolled.push(relative);
            }
            if matches!(group, "lowercase" | "capitals") {
                let class = out["classes"][glyph][id][side].as_str().unwrap_or("-");
                off.push((relative, format!("{glyph} {side} {theirs:.0} got {ours:.0} ({class})")));
            }
        }
    }
    println!("{name}, model {model}: {} glyphs compared, {took:?}, areas {}", stored.len(), out["areas"][id]);
    let mut medians = BTreeMap::new();
    for (group, t) in &mut tallies {
        let med = median(&mut t.relative);
        medians.insert(*group, med);
        println!(
            "  {group:14} sides {:5}  median {:5.1}%  without controls {:5.1}%  p75 {:5.1}%  p90 {:5.1}%  | stored under 5 units: {} sides, median {:.0} units off",
            t.relative.len(),
            med * 100.0,
            median(&mut t.uncontrolled) * 100.0,
            percentile(&t.relative, 0.75) * 100.0,
            percentile(&t.relative, 0.9) * 100.0,
            t.small.len(),
            median(&mut t.small),
        );
    }
    off.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (r, text) in off.iter().take(worst) {
        println!("    {:6.0}%  {text}", r * 100.0);
    }
    (medians, took)
}

/// Removes every metrics key of the font.
fn remove_keys(s: &mut Session, font: &Font) {
    let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
    s.edit("remove keys", |f| {
        for name in &names {
            let mut g = f.glyph_mut(name)?;
            g.info.metrics_keys = Default::default();
            for l in g.layers.iter_mut() {
                l.info.metrics_keys = Default::default();
            }
        }
        Ok(())
    })
    .unwrap();
}

/// Runs the measure on one source; returns the failures.
fn measure(path: &Path, speed_limit: Option<Duration>) -> Vec<String> {
    let mut failures = Vec::new();
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    for keyed in [false, true] {
        if keyed {
            let font = s.font().unwrap().clone();
            remove_keys(&mut s, &font);
            println!("{name}: every letter and figure, metrics keys removed");
        }
        let font = s.font().unwrap().clone();
        for m in font.masters.iter() {
            let stored: BTreeMap<String, (f64, f64)> =
                font.glyphs.iter().filter_map(|g| compared(&font, g, &m.id).map(|v| (g.name.clone(), v))).collect();
            let title = format!("{name} / {}", m.name);
            for model in ["area", "classes"] {
                let worst = if model == "classes" && !keyed { 20 } else { 0 };
                let (medians, took) = compare(&mut s, &font, (m.id.as_str(), title.as_str()), &stored, model, worst);
                if model == "area" || keyed {
                    continue;
                }
                for (group, med) in medians {
                    if matches!(group, "lowercase" | "capitals") && med > 0.10 {
                        failures.push(format!("{title}: {group} median {:.1}% > 10%", med * 100.0));
                    }
                }
                if let Some(limit) = speed_limit
                    && took > limit
                {
                    failures.push(format!("{title}: {took:?} > {limit:?}"));
                }
            }
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
