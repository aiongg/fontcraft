//! Every designspace in the corpus whose UFOs are all present, through the model and back: the
//! document and every UFO's files. Skipped when `corpus/` has not been fetched (`cargo xtask
//! corpus --fetch`). Guards the documents whose UFOs fontTools wrote.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fontcraft_io_ufo::{Files, designspace};

fn documents(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == ".git" {
            continue;
        }
        if p.is_dir() {
            documents(&p, out);
        } else if name.ends_with(".designspace") {
            out.push(p);
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
                out.insert(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"), std::fs::read(&p).unwrap());
            }
        }
    }
    out
}

fn first_difference(name: &str, a: &Files, b: &Files) -> Option<String> {
    for (k, v) in a {
        match b.get(k) {
            None => return Some(format!("{name}/{k}: not written")),
            Some(w) if w != v => {
                let (x, y) = (String::from_utf8_lossy(v), String::from_utf8_lossy(w));
                let line = x.lines().zip(y.lines()).position(|(p, q)| p != q).unwrap_or(0);
                return Some(format!("{name}/{k}:{}: {:?} → {:?}", line + 1, x.lines().nth(line).unwrap_or(""), y.lines().nth(line).unwrap_or("")));
            }
            _ => {}
        }
    }
    b.keys().find(|k| !a.contains_key(*k)).map(|k| format!("{name}/{k}: written, not in the original"))
}

#[test]
fn designspaces_round_trip_through_the_model() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    if !corpus.is_dir() {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    }
    let mut all = Vec::new();
    documents(&corpus, &mut all);
    all.sort();
    let (mut checked, mut same, mut different, mut failed) = (0, 0, Vec::new(), Vec::new());
    for path in &all {
        let text = std::fs::read_to_string(path).unwrap();
        let Ok(paths) = designspace::ufo_paths(&text) else { continue };
        let dir = path.parent().unwrap();
        if paths.is_empty() || !paths.iter().all(|p| dir.join(p).join("metainfo.plist").is_file()) {
            continue;
        }
        checked += 1;
        let ufos: BTreeMap<String, Files> = paths.iter().map(|p| (p.clone(), read_tree(&dir.join(p)))).collect();
        let ufolib =
            ufos.values().all(|u| u.get("metainfo.plist").is_some_and(|m| String::from_utf8_lossy(m).contains("com.github.fonttools.ufoLib")));
        let rel = path.strip_prefix(&corpus).unwrap().display().to_string();
        match designspace::read(&text, &ufos).and_then(|f| designspace::write(&f)) {
            Ok(out) => {
                let mut diff = (out.document != text).then(|| {
                    let line = text.lines().zip(out.document.lines()).position(|(p, q)| p != q).unwrap_or(0);
                    format!("document:{}: {:?} → {:?}", line + 1, text.lines().nth(line).unwrap_or(""), out.document.lines().nth(line).unwrap_or(""))
                });
                for (p, files) in &ufos {
                    if diff.is_none() {
                        diff = match out.ufos.get(p) {
                            Some(w) => first_difference(p, files, w),
                            None => Some(format!("{p}: not written")),
                        };
                    }
                }
                match diff {
                    None => same += 1,
                    Some(d) => different.push((ufolib, format!("{rel}: {d}"))),
                }
            }
            Err(e) => failed.push((ufolib, format!("{rel}: {e}"))),
        }
    }
    eprintln!("{checked} designspaces with their UFOs: {same} identical, {} different, {} failed", different.len(), failed.len());
    for (u, d) in different.iter().chain(&failed) {
        eprintln!("  {}{d}", if *u { "(ufoLib) " } else { "" });
    }
    let bad: Vec<_> = different.iter().chain(&failed).filter(|(u, _)| *u).collect();
    assert!(bad.is_empty(), "{} designspaces whose UFOs fontTools wrote do not come back unchanged", bad.len());
}
