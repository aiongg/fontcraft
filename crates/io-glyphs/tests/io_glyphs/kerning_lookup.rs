//! The kerning-lookup speed measure of `docs/plans/M5.md` § 5: the pair that applies to every
//! pair of glyphs a stored pair of Inter Roman's Regular master covers (groups expanded).
//! `cargo test --release -p fontcraft-io-glyphs --test kerning_lookup -- --ignored --nocapture`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use fontcraft_spacing::kerning::{Direction, KernIndex, Position, Side, facing};

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
#[ignore = "speed measure"]
fn inter_regular_kerning_lookup() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/inter/src/Inter-Roman.glyphspackage");
    if !dir.is_dir() {
        return;
    }
    let mut files = fontcraft_io_glyphs::Files::new();
    read_tree(&dir, &dir, &mut files);
    let font = fontcraft_io_glyphs::read_package(&files).unwrap();
    let master = font.masters.iter().find(|m| m.name == "Regular").unwrap().id.clone();

    let t = Instant::now();
    let index = KernIndex::new(&font);
    let build = t.elapsed();

    let direction = Direction::Ltr;
    let expand = |stored: &str, position: Position| -> Vec<usize> {
        match Side::from_stored(stored) {
            Side::Glyph(g) => index.glyph_id(&g).into_iter().collect(),
            Side::Group(g) => index.members(facing(direction, position), &g).filter_map(|n| index.glyph_id(n)).collect(),
        }
    };
    let mut covered = HashSet::new();
    let mut stored = 0;
    for m in direction.table(&font.kerning).0.iter().filter(|m| m.master == master) {
        for f in &m.firsts {
            let firsts = expand(&f.first, Position::First);
            for (s, _) in &f.seconds {
                stored += 1;
                for b in expand(s, Position::Second) {
                    covered.extend(firsts.iter().map(|a| (*a, b)));
                }
            }
        }
    }
    let covered: Vec<(usize, usize)> = covered.into_iter().collect();

    let t = Instant::now();
    let table = index.table(direction, &master).unwrap();
    let mut found = 0usize;
    let mut sum = 0.0;
    for (a, b) in &covered {
        if let Some(p) = table.lookup(*a, *b) {
            found += 1;
            sum += p.value;
        }
    }
    let lookup = t.elapsed();
    assert_eq!(found, covered.len(), "every covered pair has a pair that applies");
    println!(
        "Inter Roman Regular: {stored} stored pairs, {} covered glyph pairs; index built in {build:?}, all lookups {lookup:?} (sum {sum})",
        covered.len()
    );
}
