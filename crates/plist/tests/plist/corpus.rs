//! Corpus round trips through the property-list layer alone. Skipped when `corpus/` has not been
//! fetched (`cargo xtask corpus --fetch`). `cargo xtask roundtrip --mode plist` gives the full
//! report; this test guards the part that must stay exact: every `.glyphs` file and package the
//! reference app saved, outside fontc's test data (which has hand edits), comes back byte for byte.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use fontcraft_plist::openstep::{self, Dialect};
use fontcraft_plist::{Num, Value};

fn corpus() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    dir.is_dir().then_some(dir)
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == ".git" {
            continue;
        }
        if name.ends_with(".glyphs") && p.is_file() || name.ends_with(".glyphspackage") && p.is_dir() {
            out.push(p);
        } else if p.is_dir() {
            sources(&p, out);
        }
    }
}

fn dialect(root: &Value) -> Dialect {
    let v = root.as_dict().and_then(|d| d.get(".formatVersion")).and_then(Value::as_num).map(Num::as_f64);
    if v == Some(4.0) { Dialect::Glyphs4 } else { Dialect::Glyphs3 }
}

fn files_of(source: &Path) -> Vec<PathBuf> {
    if source.is_file() {
        return vec![source.to_path_buf()];
    }
    let mut out = Vec::new();
    let mut stack = vec![source.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "plist" || x == "glyph") {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn reference_written_glyphs_sources_round_trip_exactly() {
    let Some(corpus) = corpus() else {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    };
    let mut all = Vec::new();
    sources(&corpus, &mut all);
    let mut checked = 0;
    let mut failures = Vec::new();
    for source in all.iter().filter(|p| !p.starts_with(corpus.join("fontc"))) {
        let head_file = if source.is_dir() { source.join("fontinfo.plist") } else { source.clone() };
        let Ok(head) = std::fs::read_to_string(&head_file) else { continue };
        if !head.lines().take(3).any(|l| l.starts_with(".appVersion = ")) {
            continue;
        }
        let d = dialect(&openstep::parse(&head).unwrap().root);
        for file in files_of(source) {
            let text = std::fs::read_to_string(&file).unwrap();
            let doc = openstep::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            if openstep::write(&doc, d) != text {
                failures.push(file.display().to_string());
            }
        }
        checked += 1;
    }
    assert!(failures.is_empty(), "{} files did not round-trip:\n{}", failures.len(), failures.join("\n"));
    assert!(checked >= 30, "only {checked} reference-written sources found");
    eprintln!("{checked} reference-written sources round-trip exactly");
}
