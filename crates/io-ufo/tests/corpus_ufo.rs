//! Every UFO in the corpus through the model and back, file by file. Skipped when `corpus/` has
//! not been fetched (`cargo xtask corpus --fetch`). Guards the UFOs fontTools wrote: every file
//! must come back byte for byte, and nothing may be added or lost.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use fontcraft_io_ufo::{Files, ufo};

fn ufos(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == ".git" || !p.is_dir() {
            continue;
        }
        if name.ends_with(".ufo") {
            out.push(p);
        } else {
            ufos(&p, out);
        }
    }
}

fn read_tree(root: &Path) -> Files {
    let mut out = Files::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                out.insert(rel, std::fs::read(&p).unwrap());
            }
        }
    }
    out
}

fn first_difference(a: &Files, b: &Files) -> Option<String> {
    for (k, v) in a {
        match b.get(k) {
            None => return Some(format!("{k}: not written")),
            Some(w) if w != v => {
                let (x, y) = (String::from_utf8_lossy(v), String::from_utf8_lossy(w));
                let line = x.lines().zip(y.lines()).position(|(p, q)| p != q).unwrap_or_else(|| x.lines().count().min(y.lines().count()));
                return Some(format!(
                    "{k}:{}: {:?} → {:?}",
                    line + 1,
                    x.lines().nth(line).unwrap_or("<end>"),
                    y.lines().nth(line).unwrap_or("<end>")
                ));
            }
            _ => {}
        }
    }
    b.keys().find(|k| !a.contains_key(*k)).map(|k| format!("{k}: written, not in the original"))
}

#[test]
fn ufos_round_trip_through_the_model() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    if !corpus.is_dir() {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    }
    let mut all = Vec::new();
    ufos(&corpus, &mut all);
    all.sort();
    let (mut same, mut different, mut failed) = (0, Vec::new(), Vec::new());
    for path in &all {
        let files = read_tree(path);
        let ufolib = files.get("metainfo.plist").is_some_and(|m| String::from_utf8_lossy(m).contains("<string>com.github.fonttools.ufoLib</string>"));
        let rel = path.strip_prefix(&corpus).unwrap().display().to_string();
        match ufo::read(&files).and_then(|f| ufo::write(&f)) {
            Ok(out) => match first_difference(&files, &out) {
                None => same += 1,
                Some(d) => different.push((ufolib, format!("{rel}: {d}"))),
            },
            Err(e) => failed.push((ufolib, format!("{rel}: {e}"))),
        }
    }
    let all_lines = std::env::var_os("FONTCRAFT_ALL").is_some();
    eprintln!("{} UFOs: {same} identical, {} different, {} failed", all.len(), different.len(), failed.len());
    for (u, d) in different.iter().filter(|(u, _)| all_lines || *u) {
        eprintln!("  different{}: {d}", if *u { " (ufoLib)" } else { "" });
    }
    for (u, d) in failed.iter().filter(|(u, _)| all_lines || *u) {
        eprintln!("  failed{}: {d}", if *u { " (ufoLib)" } else { "" });
    }
    let bad: Vec<_> = different.iter().chain(&failed).filter(|(u, _)| *u).collect();
    assert!(bad.is_empty(), "{} UFOs fontTools wrote do not come back unchanged", bad.len());
}
