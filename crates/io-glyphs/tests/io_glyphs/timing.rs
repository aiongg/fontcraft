//! Diagnostic timings on the largest corpus family (Inter, two packages of thousands of glyphs).
//! `cargo test --release -p fontcraft-io-glyphs --test io_glyphs timing:: -- --ignored --nocapture`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::time::Instant;

fn read_tree(dir: &Path, root: &Path, out: &mut fontcraft_io_glyphs::Files) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            read_tree(&p, root, out);
        } else {
            out.insert(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"), std::fs::read(&p).unwrap());
        }
    }
}

#[test]
#[ignore = "diagnostic timing"]
fn inter_timings() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/inter/src/Inter-Roman.glyphspackage");
    if !dir.is_dir() {
        return;
    }
    let t = Instant::now();
    let mut files = fontcraft_io_glyphs::Files::new();
    read_tree(&dir, &dir, &mut files);
    let bytes: usize = files.values().map(Vec::len).sum();
    let disk = t.elapsed();
    let t = Instant::now();
    let font = fontcraft_io_glyphs::read_package(&files).unwrap();
    let read = t.elapsed();
    let t = Instant::now();
    let out = fontcraft_io_glyphs::write_package(&font).unwrap();
    let write = t.elapsed();
    assert_eq!(out.len(), files.len());
    // An undo snapshot and an edit of one glyph: the glyph pointer list is copied once.
    let t = Instant::now();
    let mut next = font.clone();
    let name = font.glyphs.as_arcs().get(100).unwrap().name.clone();
    next.glyph_mut(&name).unwrap().layers[0].width += 1.0;
    let edit = t.elapsed();
    let t = Instant::now();
    font.check().unwrap();
    let check = t.elapsed();
    let t = Instant::now();
    next.check_changed(&font).unwrap();
    let check_changed = t.elapsed();
    // The out-of-sync cache: the first full scan, then the update after a width edit to `n`
    // (which many metrics keys read) and after a rename of `a`.
    let mut font = std::sync::Arc::new(font);
    let mut cache = fontcraft_spacing::SyncCache::default();
    let t = Instant::now();
    cache.update(&font);
    let sync_first = t.elapsed();
    std::sync::Arc::make_mut(&mut font).glyph_mut("n").unwrap().layers[0].width += 10.0;
    let computed = cache.computed();
    let t = Instant::now();
    cache.update(&font);
    let sync_width = t.elapsed();
    let width_glyphs = cache.computed() - computed;
    fontcraft_model::rename_glyphs(std::sync::Arc::make_mut(&mut font), &[("a".into(), "a.renamed".into())]).unwrap();
    let computed = cache.computed();
    let t = Instant::now();
    cache.update(&font);
    let sync_rename = t.elapsed();
    let rename_glyphs = cache.computed() - computed;
    println!(
        "Inter Roman: {} glyphs, {} masters, {} files, {:.1} MB\n  disk {disk:?}, read {read:?}, write {write:?}\n  snapshot + one-glyph edit {edit:?}, full check {check:?}, check after the edit {check_changed:?}\n  sync cache: first update {sync_first:?}, after a width edit to n {sync_width:?} ({width_glyphs} glyphs scanned), after a rename of a {sync_rename:?} ({rename_glyphs} glyphs)",
        font.glyphs.len(),
        font.masters.len(),
        files.len(),
        bytes as f64 / 1e6
    );
}
