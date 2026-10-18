//! Measure (`docs/plans/M13.md` § 5, auto-kerning against the corpus): for Inter Roman and
//! Oswald, every master with kerning of its own, `kerning.autoKern` with its defaults (target
//! from the font's own n n and H H) proposes a value for each left-to-right pair the master
//! stores with a value of at least 20 units in size, sides as stored (glyphs or groups).
//!
//! Target: the proposal has the stored value's sign (a proposal of 0 counts as a miss) in at
//! least 80% of the pairs, in every master. The correlation between proposed and stored values
//! is printed per master, with the share of agreeing signs for negative and positive stored
//! values apart. Speed target: 10,000 pairs under 1 s in a release build (scaled to the number
//! of pairs measured).
//!
//! `cargo test --release -p fontcraft-engine --test engine corpus_autokern:: -- --ignored --nocapture`.
//! `FONTCRAFT_CORPUS` names the corpus folder (default: the repository's `corpus/`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fontcraft_engine::Session;
use fontcraft_spacing::kerning::pairs::stored_pairs;
use fontcraft_spacing::kerning::{Direction, KernIndex, Position, Side, facing};
use fontcraft_spacing::metrics_source;
use serde_json::{Value, json};

fn corpus() -> PathBuf {
    std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from)
}

/// Whether the command accepts the side: a glyph of the font, or a group with members.
fn usable(index: &KernIndex, side: &Side, position: Position) -> bool {
    match side {
        Side::Glyph(n) => index.glyph_id(n).is_some(),
        Side::Group(g) => index.members(facing(Direction::Ltr, position), g).next().is_some(),
    }
}

fn correlation(pairs: &[(f64, f64)]) -> f64 {
    let n = pairs.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let (mx, my) = (pairs.iter().map(|p| p.0).sum::<f64>() / n, pairs.iter().map(|p| p.1).sum::<f64>() / n);
    let cov: f64 = pairs.iter().map(|(x, y)| (x - mx) * (y - my)).sum();
    let vx: f64 = pairs.iter().map(|(x, _)| (x - mx).powi(2)).sum();
    let vy: f64 = pairs.iter().map(|(_, y)| (y - my).powi(2)).sum();
    if vx == 0.0 || vy == 0.0 { 0.0 } else { cov / (vx * vy).sqrt() }
}

/// Runs the measure on one source; returns the failures.
fn measure(path: &Path, speed: bool) -> Vec<String> {
    let mut failures = Vec::new();
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    let font = s.font().unwrap().clone();
    let index = KernIndex::new(&font);
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    for m in font.masters.iter() {
        if metrics_source(&font, &m.id).is_some_and(|src| src.id != m.id) {
            continue;
        }
        let mut stored: HashMap<(String, String), f64> = HashMap::new();
        let mut pairs: Vec<Value> = Vec::new();
        for lp in stored_pairs(&font, Direction::Ltr, &m.id) {
            let (l, r) = (lp.left(), lp.right());
            if lp.value.abs() < 20.0 || !usable(&index, &l, Position::First) || !usable(&index, &r, Position::Second) {
                continue;
            }
            if stored.insert((l.command(), r.command()), lp.value).is_none() {
                pairs.push(json!([l.command(), r.command()]));
            }
        }
        if pairs.is_empty() {
            println!("{name} / {}: no stored pairs of 20 units or more", m.name);
            continue;
        }
        let count = pairs.len();
        let t = Instant::now();
        let out = s.execute("kerning.autoKern", &json!({"pairs": pairs, "masters": [m.id], "keepExisting": false, "apply": false})).unwrap();
        let took = t.elapsed();
        let mut both = Vec::new();
        let (mut agree, mut neg, mut neg_agree, mut pos, mut pos_agree) = (0usize, 0usize, 0usize, 0usize, 0usize);
        for p in out["pairs"].as_array().unwrap() {
            let key = (p["left"].as_str().unwrap().to_string(), p["right"].as_str().unwrap().to_string());
            let Some(theirs) = stored.get(&key).copied() else { continue };
            let ours = p["value"].as_f64().unwrap();
            let same = ours != 0.0 && ours.signum() == theirs.signum();
            agree += usize::from(same);
            if theirs < 0.0 {
                neg += 1;
                neg_agree += usize::from(same);
            } else {
                pos += 1;
                pos_agree += usize::from(same);
            }
            both.push((ours, theirs));
        }
        let measured = both.len();
        let share = if measured == 0 { 0.0 } else { agree as f64 / measured as f64 };
        let pct = |a: usize, n: usize| if n == 0 { 0.0 } else { a as f64 * 100.0 / n as f64 };
        println!(
            "{name} / {}: {measured} of {count} pairs measured ({} skipped), {took:?}, targets {}",
            m.name,
            out["skipped"].as_array().map_or(0, Vec::len),
            out["targets"][&m.id],
        );
        println!(
            "  sign agrees {:5.1}%  (stored negative {neg}: {:5.1}%, positive {pos}: {:5.1}%)  correlation {:.3}",
            share * 100.0,
            pct(neg_agree, neg),
            pct(pos_agree, pos),
            correlation(&both),
        );
        if share < 0.8 {
            failures.push(format!("{name} / {}: sign agrees in {:.1}% < 80%", m.name, share * 100.0));
        }
        let limit = Duration::from_secs_f64(count as f64 / 10_000.0);
        if speed && took > limit.max(Duration::from_millis(50)) {
            failures.push(format!("{name} / {}: {count} pairs in {took:?} > {limit:?}", m.name));
        }
    }
    failures
}

#[test]
#[ignore = "corpus measure"]
fn auto_kerning_against_inter_and_oswald() {
    let root = corpus();
    let inter = root.join("inter/src/Inter-Roman.glyphspackage");
    let oswald = root.join("oswald/sources/Oswald.glyphs");
    if !inter.is_dir() || !oswald.is_file() {
        eprintln!("corpus not fetched: skipped");
        return;
    }
    // The speed target holds for optimised builds only.
    let speed = !cfg!(debug_assertions);
    let mut failures = measure(&inter, speed);
    failures.extend(measure(&oswald, speed));
    assert!(failures.is_empty(), "{failures:#?}");
}
