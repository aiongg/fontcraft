//! Measure (M3 plan § M3.50): in every corpus `.glyphs` and `.ufo` source, random moves of
//! on-curve nodes leave every smooth node next to a line within 1 unit of that line.
//!
//! `cargo test -p fontcraft-engine --test engine corpus_tangents:: -- --ignored --nocapture`.
//! `FONTCRAFT_CORPUS` names the corpus folder (default: the repository's `corpus/`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path as FilePath, PathBuf};

use fontcraft_engine::Session;
use fontcraft_engine::model::{Path, Shape};
use serde_json::json;

const MOVES_PER_SOURCE: usize = 100;
/// Largest distance of a handle from the line of its smooth node, in units.
const TOLERANCE: f64 = 1.0;

fn sources(dir: &FilePath, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n == ".git" || n == "fontc") {
            continue;
        }
        if p.extension().is_some_and(|x| x == "glyphs" || x == "ufo") {
            out.push(p);
        } else if p.is_dir() {
            sources(&p, out);
        }
    }
}

/// The neighbours of on-curve node `k` of a closed path.
fn around(path: &Path, k: usize) -> (usize, usize) {
    let n = path.nodes.len();
    ((k + n - 1) % n, (k + 1) % n)
}

/// How far the handle of the smooth node `k` is from its line; `None` when `k` is no such node.
/// A handle that is the only one of its segment, between two smooth nodes, is left out (M3.54).
fn off_line(path: &Path, k: usize) -> Option<f64> {
    let node = &path.nodes[k];
    if !node.on_curve() || !node.smooth || path.nodes.len() < 3 {
        return None;
    }
    let (a, b) = around(path, k);
    let (handle, far, beyond) = match (path.nodes[a].on_curve(), path.nodes[b].on_curve()) {
        (false, true) => (a, b, around(path, a).0),
        (true, false) => (b, a, around(path, b).1),
        _ => return None,
    };
    if path.nodes[beyond].on_curve() && path.nodes[beyond].smooth {
        return None;
    }
    let (p, h, f) = (node.point(), path.nodes[handle].point(), path.nodes[far].point());
    let (line, arm) = (p - f, h - p);
    (line.hypot() > 1e-9 && arm.hypot() > 1e-9).then(|| (line.cross(arm) / line.hypot()).abs())
}

#[test]
#[ignore = "reads the corpus"]
fn random_moves_keep_tangents_on_their_lines() {
    let root =
        std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus")), PathBuf::from);
    let mut list = Vec::new();
    sources(&root, &mut list);
    list.sort();
    let (mut moves, mut checked, mut off) = (0usize, 0usize, Vec::new());
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut random = |below: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % below.max(1) as u64) as usize
    };
    for file in &list {
        let mut s = Session::new();
        if s.execute("file.open", &json!({"path": file})).is_err() {
            continue;
        }
        // Closed paths with a tangent: glyph, layer, shape.
        let found: Vec<(String, String, usize)> = {
            let font = s.font().unwrap();
            let mut found = Vec::new();
            for g in font.glyphs.iter() {
                for l in g.layers.iter().filter(|l| l.is_master_layer()) {
                    for (i, shape) in l.shapes.iter().enumerate() {
                        if let Shape::Path(p) = shape
                            && p.closed
                            && (0..p.nodes.len()).any(|k| off_line(p, k).is_some())
                        {
                            found.push((g.name.to_string(), l.id.to_string(), i));
                        }
                    }
                }
            }
            found
        };
        if found.is_empty() {
            continue;
        }
        for _ in 0..MOVES_PER_SOURCE {
            let (glyph, layer, shape) = found[random(found.len())].clone();
            let path = |s: &Session| match &s.font().unwrap().glyph(&glyph).unwrap().layers.iter().find(|l| l.id == layer).unwrap().shapes[shape] {
                Shape::Path(p) => p.clone(),
                _ => unreachable!("the shape was a path"),
            };
            let before = path(&s);
            let tangents: Vec<usize> = (0..before.nodes.len()).filter(|&k| off_line(&before, k).is_some()).collect();
            // A tangent, or the far end of its line.
            let k = tangents[random(tangents.len())];
            let (a, b) = around(&before, k);
            let far = if before.nodes[a].on_curve() { a } else { b };
            let node = if random(2) == 0 { k } else { far };
            let (dx, dy) = (random(81) as f64 - 40.0, random(81) as f64 - 40.0);
            let p = json!({"glyph": glyph, "layer": layer, "points": [[shape, node]], "dx": dx, "dy": dy});
            if (dx, dy) == (0.0, 0.0) || s.execute("selection.move", &p).is_err() {
                continue;
            }
            moves += 1;
            let after = path(&s);
            if let Some(d) = off_line(&after, k) {
                checked += 1;
                if d > TOLERANCE {
                    off.push(format!("{}: {glyph} {layer} path {shape} node {k} is {d:.2} off after [{node}] by ({dx}, {dy})", file.display()));
                }
            }
        }
    }
    println!("{} sources, {moves} moves, {checked} tangents checked, {} off their line", list.len(), off.len());
    for line in off.iter().take(40) {
        println!("  {line}");
    }
    assert!(off.is_empty(), "{} tangents left their line", off.len());
}
