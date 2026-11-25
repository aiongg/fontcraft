//! Measure (`docs/plans/M13.md` § 5, auto-kerning against the corpus): for Inter Roman and
//! Oswald, every master with kerning of its own, `kerning.autoKern` with its defaults (target
//! from the font's own n n and H H) proposes a value for each left-to-right pair the master
//! stores with a value of at least 20 units in size, sides as stored (glyphs or groups).
//!
//! Targets: the proposal has the stored value's sign (a proposal of 0 counts as a miss) in at
//! least 80% of the pairs, and the median absolute difference between the two is at most 20 units
//! at 1,000 units per em, in every master. The correlation between proposed and stored values
//! is printed per master, with the share of agreeing signs for negative and positive stored
//! values apart. Speed target: 10,000 pairs under 1 s in a release build (scaled to the number
//! of pairs measured).
//!
//! The second measure (`docs/plans/M13.md` § "M13.21c") compares the groups `kerning.buildGroups`
//! proposes, all masters together and the sources' own groups replaced, with those groups: the
//! adjusted Rand index over the glyphs that have a group on that side (1 the same grouping, 0 what
//! chance gives; a glyph left alone counts as a group of its own). Target: at least 0.6 on each
//! side of each font, with a difference of 10 units at 1,000 units per em.
//!
//! Both measures print a sweep of the clipping depth (`depthShare`); only the defaults have
//! targets.
//!
//! `cargo test --release -p fontcraft-engine --test engine corpus_autokern:: -- --ignored --nocapture`.
//! `FONTCRAFT_CORPUS` names the corpus folder (default: the repository's `corpus/`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fontcraft_engine::Session;
use fontcraft_engine::model::glyph_props;
use fontcraft_spacing::kerning::pairs::stored_pairs;
use fontcraft_spacing::kerning::{Direction, GroupSide, KernIndex, Position, Side, facing};
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

/// The clipping depths the sweeps try, as shares of the x-height.
const DEPTH_SHARES: [f64; 6] = [0.10, 0.15, 0.20, 0.25, 0.30, 0.50];
/// The largest difference Build Kerning Groups allows, at 1,000 units per em.
const MAX_DIFFERENCE: f64 = 10.0;

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or_default()
}

/// Auto-kerning's proposals for `pairs` against the stored values: (ours, theirs) per pair.
fn proposals(s: &mut Session, master: &str, pairs: &[Value], stored: &HashMap<(String, String), f64>, share: f64) -> Vec<(f64, f64)> {
    let p = json!({"pairs": pairs, "masters": [master], "keepExisting": false, "apply": false, "depthShare": share});
    let out = s.execute("kerning.autoKern", &p).unwrap();
    let mut both = Vec::new();
    for p in out["pairs"].as_array().unwrap() {
        let key = (p["left"].as_str().unwrap().to_string(), p["right"].as_str().unwrap().to_string());
        if let Some(theirs) = stored.get(&key) {
            both.push((p["value"].as_f64().unwrap(), *theirs));
        }
    }
    both
}

fn same_sign((ours, theirs): &(f64, f64)) -> bool {
    *ours != 0.0 && ours.signum() == theirs.signum()
}

/// The adjusted Rand index of two labellings of the same items: 1 for the same partition, about 0
/// for unrelated ones.
fn adjusted_rand(a: &[String], b: &[String]) -> f64 {
    let pairs = |n: usize| (n * n.saturating_sub(1)) as f64 / 2.0;
    let mut cells: HashMap<(&str, &str), usize> = HashMap::new();
    let (mut rows, mut columns): (HashMap<&str, usize>, HashMap<&str, usize>) = Default::default();
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (x.as_str(), y.as_str());
        *cells.entry((x, y)).or_default() += 1;
        *rows.entry(x).or_default() += 1;
        *columns.entry(y).or_default() += 1;
    }
    let sum = |counts: Vec<usize>| counts.into_iter().map(pairs).sum::<f64>();
    let together = sum(cells.into_values().collect());
    let (in_a, in_b) = (sum(rows.into_values().collect()), sum(columns.into_values().collect()));
    let total = pairs(a.len().min(b.len()));
    if total == 0.0 {
        return 1.0;
    }
    let expected = in_a * in_b / total;
    let most = (in_a + in_b) / 2.0;
    if most == expected { 1.0 } else { (together - expected) / (most - expected) }
}

/// Runs the group measure on one source; returns the failures.
fn measure_groups(path: &Path) -> Vec<String> {
    let mut failures = Vec::new();
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    let font = s.font().unwrap().clone();
    let data = s.glyph_data().unwrap();
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let max_difference = MAX_DIFFERENCE * f64::from(font.units_per_em) / 1000.0;
    for share in std::iter::once(None).chain(DEPTH_SHARES.map(Some)) {
        let mut p = json!({"maxDifference": max_difference, "replaceExisting": true});
        if let Some(share) = share {
            p["depthShare"] = json!(share);
        }
        let t = Instant::now();
        let out = s.execute("kerning.buildGroups", &p).unwrap();
        let took = t.elapsed();
        let skipped: HashSet<&str> = out["skipped"].as_array().unwrap().iter().filter_map(|v| v["glyph"].as_str()).collect();
        for side in [GroupSide::Left, GroupSide::Right] {
            let mut proposed: BTreeMap<&str, &str> = BTreeMap::new();
            for g in out["groups"][side.name()].as_array().unwrap() {
                for glyph in g["glyphs"].as_array().unwrap() {
                    proposed.insert(glyph.as_str().unwrap(), g["name"].as_str().unwrap());
                }
            }
            let (mut ours, mut theirs) = (Vec::new(), Vec::new());
            for g in font.glyphs.iter().filter(|g| g.export && !skipped.contains(g.name.as_str())) {
                if !matches!(glyph_props(g, &data, false).category.as_deref(), Some("Letter" | "Number")) {
                    continue;
                }
                let Some(group) = side.of(&g.kern_groups) else { continue };
                theirs.push(group.to_string());
                // A glyph left alone is a group of its own; the prefix keeps it apart from groups.
                ours.push(proposed.get(g.name.as_str()).map_or_else(|| format!("alone {}", g.name), |n| format!("group {n}")));
            }
            let index = adjusted_rand(&ours, &theirs);
            let groups = |v: &[String]| v.iter().collect::<HashSet<_>>().len();
            let shown = share.map_or_else(|| "default".to_string(), |s| format!("{s:.2}"));
            println!(
                "{name}: {} groups, depth share {shown}: adjusted Rand index {index:.3} over {} glyphs ({} groups here, {} in the source), {took:?}",
                side.name(),
                ours.len(),
                groups(&ours),
                groups(&theirs),
            );
            if share.is_none() && index < 0.6 {
                failures.push(format!("{name}: {} groups: adjusted Rand index {index:.3} < 0.6", side.name()));
            }
        }
    }
    failures
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
        let out = s.execute("kerning.autoKern", &json!({"pairs": &pairs, "masters": [m.id], "keepExisting": false, "apply": false})).unwrap();
        let took = t.elapsed();
        let mut both = Vec::new();
        let (mut agree, mut neg, mut neg_agree, mut pos, mut pos_agree) = (0usize, 0usize, 0usize, 0usize, 0usize);
        for p in out["pairs"].as_array().unwrap() {
            let key = (p["left"].as_str().unwrap().to_string(), p["right"].as_str().unwrap().to_string());
            let Some(theirs) = stored.get(&key).copied() else { continue };
            let ours = p["value"].as_f64().unwrap();
            let same = same_sign(&(ours, theirs));
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
        let per_em = 1000.0 / f64::from(font.units_per_em);
        let distance = |both: &[(f64, f64)]| median(both.iter().map(|(a, b)| (a - b).abs() * per_em).collect());
        let off = distance(&both);
        let pct = |a: usize, n: usize| if n == 0 { 0.0 } else { a as f64 * 100.0 / n as f64 };
        println!(
            "{name} / {}: {measured} of {count} pairs measured ({} skipped), {took:?}, targets {}",
            m.name,
            out["skipped"].as_array().map_or(0, Vec::len),
            out["targets"][&m.id],
        );
        println!(
            "  sign agrees {:5.1}%  (stored negative {neg}: {:5.1}%, positive {pos}: {:5.1}%)  median {off:.1} units per 1,000 off  correlation {:.3}",
            share * 100.0,
            pct(neg_agree, neg),
            pct(pos_agree, pos),
            correlation(&both),
        );
        if share < 0.8 {
            failures.push(format!("{name} / {}: sign agrees in {:.1}% < 80%", m.name, share * 100.0));
        }
        if off > 20.0 {
            failures.push(format!("{name} / {}: median difference {off:.1} > 20 units per 1,000", m.name));
        }
        for depth in DEPTH_SHARES {
            let both = proposals(&mut s, &m.id, &pairs, &stored, depth);
            println!(
                "    depth share {depth:.2}: sign agrees {:5.1}%  median {:.1} units per 1,000 off",
                pct(both.iter().filter(|p| same_sign(p)).count(), both.len()),
                distance(&both),
            );
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

#[test]
#[ignore = "corpus measure"]
fn build_groups_against_inter_and_oswald() {
    let root = corpus();
    let inter = root.join("inter/src/Inter-Roman.glyphspackage");
    let oswald = root.join("oswald/sources/Oswald.glyphs");
    if !inter.is_dir() || !oswald.is_file() {
        eprintln!("corpus not fetched: skipped");
        return;
    }
    let mut failures = measure_groups(&inter);
    failures.extend(measure_groups(&oswald));
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn the_adjusted_rand_index_of_known_partitions() {
    let labels = |text: &str| text.chars().map(String::from).collect::<Vec<_>>();
    assert_eq!(adjusted_rand(&labels("aabbcc"), &labels("xxyyzz")), 1.0);
    assert_eq!(adjusted_rand(&labels("abc"), &labels("xyz")), 1.0);
    // Pairs kept together: 2 on the left, 1 on the right, 1 in both, of 6: (1 − 1/3) ÷ (3/2 − 1/3).
    let index = adjusted_rand(&labels("aabb"), &labels("xxyz"));
    assert!((index - 4.0 / 7.0).abs() < 1e-12, "{index}");
}
