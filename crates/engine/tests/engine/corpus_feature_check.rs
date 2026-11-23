//! Measure (`docs/plans/M8.md` § M8.25): `font.features.check` on Inter Roman's largest feature entry, its stored code
//! as the draft, under 250 ms in a release build. Skipped when the corpus is not fetched.
//!
//! `cargo test --release -p fontcraft-engine --test engine corpus_feature_check:: -- --ignored --nocapture`.
//! `FONTCRAFT_CORPUS` names the corpus folder (default: the repository's `corpus/`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::time::Instant;

use fontcraft_engine::Session;
use serde_json::json;

fn corpus() -> PathBuf {
    std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from)
}

#[test]
#[ignore = "measure on the corpus; run with --release --ignored"]
fn check_on_inters_largest_feature() {
    let path = corpus().join("inter/src/Inter-Roman.glyphspackage");
    if !path.exists() {
        eprintln!("skipped: {} is not fetched", path.display());
        return;
    }
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    let font = s.font().unwrap().clone();
    let (index, largest) = font.features.features.iter().enumerate().max_by_key(|(_, e)| e.code.len()).expect("a feature");
    let params = json!({"kind": "feature", "name": largest.name, "index": index, "code": largest.code});
    let mut times = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let r = s.execute("font.features.check", &params).unwrap();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        assert!(r["errors"].as_u64().is_some(), "{r}");
    }
    times.sort_by(f64::total_cmp);
    let median = times[times.len() / 2];
    eprintln!(
        "check: feature {} ({} bytes of {} entries, {} glyphs): median {median:.1} ms, runs {times:?}",
        largest.name,
        largest.code.len(),
        font.features.features.len() + font.features.classes.len() + font.features.prefixes.len(),
        font.glyphs.as_arcs().len()
    );
    assert!(median < 250.0, "check took {median:.1} ms");
}
