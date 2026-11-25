//! `cargo xtask audit`: `audit.run` with the `outlines` profile (or the one `--profile` names) on
//! every source of the corpus comparison set, as the audit measure of `docs/plans/M3.md` § 5 asks.
//! It reports, per source and in total, whether the run completed, how long it took and how many
//! results it gave, and lists every source whose run failed or panicked.

use std::fmt::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::time::Instant;

use serde_json::{Value, json};

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let mut filter: Option<String> = None;
    let mut profile = "outlines".to_string();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match *a {
            "--match" => filter = Some(it.next().ok_or("--match needs text")?.to_string()),
            "--profile" => profile = it.next().ok_or("--profile needs a profile id")?.to_string(),
            other => return Err(format!("unknown argument `{other}`; usage: cargo xtask audit [--profile ID] [--match TEXT]")),
        }
    }
    let corpus = root.join("corpus");
    let sources = fontcraft_oracle::corpus::discover(&corpus)?;
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let (mut ran, mut load_failures, mut failures) = (0usize, Vec::new(), Vec::new());
    let (mut layers, mut results, mut fail, mut warn, mut info, mut truncated) = (0usize, 0u64, 0u64, 0u64, 0u64, 0u64);
    let mut times = Vec::new();
    let mut by_check: std::collections::BTreeMap<String, u64> = Default::default();
    let started = Instant::now();
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
        layers += font.glyphs.iter().map(|g| g.layers.len()).sum::<usize>();
        let mut session = fontcraft_engine::Session::new();
        if let Err(e) = session.add_document(font, None) {
            load_failures.push(format!("{rel}: {e}"));
            continue;
        }
        let t = Instant::now();
        let outcome = catch_unwind(AssertUnwindSafe(|| session.execute("audit.run", &json!({"profile": profile}))));
        times.push(t.elapsed().as_secs_f64());
        match outcome {
            Ok(Ok(report)) => {
                ran += 1;
                let summary = &report["summary"];
                let n = |key: &str| summary[key].as_u64().unwrap_or(0);
                fail += n("fail");
                warn += n("warn");
                info += n("info");
                truncated += n("truncated");
                for r in report["results"].as_array().map(Vec::as_slice).unwrap_or_default() {
                    results += 1;
                    *by_check.entry(r["check"].as_str().unwrap_or("?").to_string()).or_default() += 1;
                }
                if n("errors") > 0 {
                    failures.push(format!("{rel}: {} check errors: {}", n("errors"), errors(&report)));
                }
            }
            Ok(Err(e)) => failures.push(format!("{rel}: {e}")),
            Err(_) => failures.push(format!("{rel}: panicked")),
        }
    }
    std::panic::set_hook(default_hook);

    times.sort_by(f64::total_cmp);
    let pick = |p: f64| times.get(((times.len() as f64 - 1.0) * p).round() as usize).copied().unwrap_or(0.0);
    let mut out = format!(
        "sources {} (failed to load {}), audited {ran}, layers {layers}, failed or panicked {}\n\
         results {results} (fail {fail}, warn {warn}, info {info}; capped checks {truncated})\n\
         time per source: median {:.1} ms, p99 {:.1} ms, max {:.1} ms; total {:.1} s\n",
        sources.len(),
        load_failures.len(),
        failures.len(),
        pick(0.5) * 1e3,
        pick(0.99) * 1e3,
        times.last().copied().unwrap_or(0.0) * 1e3,
        started.elapsed().as_secs_f64(),
    );
    for (check, count) in &by_check {
        writeln!(out, "  {check}: {count}").ok();
    }
    for f in load_failures.iter().chain(&failures) {
        writeln!(out, "  {f}").ok();
    }
    print!("{out}");
    if failures.is_empty() { Ok(()) } else { Err(format!("{} sources failed the audit run", failures.len())) }
}

/// The ids and reasons of the checks that ended in an error.
fn errors(report: &Value) -> String {
    report["checks"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|c| c["status"] == "error")
        .map(|c| format!("{} ({})", c["id"].as_str().unwrap_or("?"), c["reason"].as_str().unwrap_or("")))
        .collect::<Vec<_>>()
        .join(", ")
}
