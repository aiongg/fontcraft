//! Measure (M6 plan § 5, alignment against the reference's stored output): for every corpus
//! `.glyphs` and `.glyphspackage` source, every master layer FontCraft aligns is re-aligned from
//! its base glyphs as stored, and each component's offset and the layer's width are compared
//! with the stored ones (within 0.5 unit). Differing layers are counted by setup and listed.
//! `cargo test -p fontcraft-engine --test corpus_alignment -- --ignored --nocapture`; set
//! `ALIGN_LIST=<n>` to list more than 40 differing layers.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fontcraft_components::align::aligned_placement;
use fontcraft_components::metrics::derived_metrics;
use fontcraft_model::{Font, Layer, Shape};

const TOLERANCE: f64 = 0.5;

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().and_then(|n| n.to_str()) == Some(".git") {
            continue;
        }
        let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
        if ext == "glyphs" && p.is_file() || ext == "glyphspackage" && p.is_dir() {
            out.push(p);
        } else if p.is_dir() {
            sources(&p, out);
        }
    }
}

/// A rough description of a layer's setup, for grouping differences.
fn setup(font: &Font, layer: &Layer) -> String {
    let comps: Vec<_> = layer
        .shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Component(c) => Some(c),
            _ => None,
        })
        .collect();
    let mut tags = Vec::new();
    tags.push(match comps.len() {
        1 => "single",
        _ if comps
            .iter()
            .skip(1)
            .all(|c| font.glyph(&c.glyph).is_some_and(|g| g.layers.iter().any(|l| l.anchors.iter().any(|a| a.name.starts_with('_'))))) =>
        {
            "base+marks"
        }
        _ => "several bases",
    });
    if comps.iter().any(|c| {
        let k = c.affine().as_coeffs();
        k[0] != 1.0 || k[1] != 0.0 || k[2] != 0.0 || k[3] != 1.0
    }) {
        tags.push("transformed");
    }
    if comps.iter().any(|c| c.info.anchor.is_some()) {
        tags.push("anchor attribute");
    }
    if layer.shapes.iter().any(|s| matches!(s, Shape::Path(_))) {
        tags.push("with paths");
    }
    tags.join(", ")
}

#[test]
#[ignore = "measure"]
fn corpus_alignment() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    let mut all = Vec::new();
    sources(&corpus, &mut all);
    all.sort();
    let list: usize = std::env::var("ALIGN_LIST").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
    let (mut read, mut aligned, mut same, mut errors) = (0usize, 0usize, 0usize, 0usize);
    let mut by_setup: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut listed = Vec::new();
    for p in &all {
        let Ok((font, _)) = fontcraft_engine::io::open(p, None) else { continue };
        read += 1;
        let source = p.strip_prefix(&corpus).unwrap_or(p).display().to_string();
        for g in font.glyphs.iter() {
            for l in g.layers.iter().filter(|l| l.is_master_layer() && l.shapes.iter().any(|s| matches!(s, Shape::Component(_)))) {
                let placements = match aligned_placement(&font, &g.name, &l.id) {
                    Ok(p) if p.is_empty() => continue,
                    Ok(p) => p,
                    Err(e) => {
                        errors += 1;
                        if listed.len() < list {
                            listed.push(format!("{source} {} {}: error {e}", g.name, l.id));
                        }
                        continue;
                    }
                };
                aligned += 1;
                let mut diffs = Vec::new();
                for pl in &placements {
                    let Some(Shape::Component(c)) = l.shapes.get(pl.shape) else { continue };
                    let (stored, now) = (c.affine().as_coeffs(), pl.transform.affine().as_coeffs());
                    let (dx, dy) = (now[4] - stored[4], now[5] - stored[5]);
                    if dx.abs() > TOLERANCE || dy.abs() > TOLERANCE {
                        diffs.push(format!("{} by ({dx:.1}, {dy:.1})", c.glyph));
                    }
                }
                if let Ok(Some(m)) = derived_metrics(&font, &g.name, &l.id)
                    && (m.width - l.width).abs() > TOLERANCE
                {
                    diffs.push(format!("width {} → {}", l.width, m.width));
                }
                let row = by_setup.entry(setup(&font, l)).or_default();
                row.0 += 1;
                if diffs.is_empty() {
                    same += 1;
                } else {
                    row.1 += 1;
                    if listed.len() < list {
                        listed.push(format!("{source} {} {}: {}", g.name, l.id, diffs.join("; ")));
                    }
                }
            }
        }
    }
    eprintln!("{read} of {} sources read; {aligned} aligned master layers, {same} equal the stored output, {errors} errors", all.len());
    eprintln!("by setup (layers, differing):");
    for (k, (n, d)) in &by_setup {
        eprintln!("  {k}: {n}, {d}");
    }
    eprintln!("differing layers (first {list}):");
    for row in &listed {
        eprintln!("  {row}");
    }
}
