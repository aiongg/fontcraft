//! Every XML file of the UFO family in the corpus (`.glif`, the property lists inside UFOs,
//! `.designspace`) parses and writes back byte for byte through the XML tree. Skipped when
//! `corpus/` has not been fetched (`cargo xtask corpus --fetch`). Files that are not well-formed
//! (deliberately broken test fixtures) must be refused with an error, never a panic; they are
//! counted and listed.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use fontcraft_plist::xml;

fn corpus() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    dir.is_dir().then_some(dir)
}

fn xml_files(dir: &Path, in_ufo: bool, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == ".git" {
            continue;
        }
        if p.is_dir() {
            xml_files(&p, in_ufo || name.ends_with(".ufo"), out);
        } else if name.ends_with(".glif") || name.ends_with(".designspace") || in_ufo && name.ends_with(".plist") {
            out.push(p);
        }
    }
}

#[test]
fn ufo_family_xml_writes_back_byte_for_byte() {
    let Some(corpus) = corpus() else {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    };
    let mut files = Vec::new();
    xml_files(&corpus, false, &mut files);
    files.sort();
    let (mut same, mut refused, mut different) = (0, Vec::new(), Vec::new());
    for f in &files {
        let Ok(bytes) = std::fs::read(f) else { continue };
        let Ok(text) = String::from_utf8(bytes) else {
            refused.push(format!("{}: not UTF-8", f.display()));
            continue;
        };
        match xml::parse(&text) {
            Ok(doc) if doc.write() == text => same += 1,
            Ok(_) => different.push(f.display().to_string()),
            Err(e) => refused.push(format!("{}: {e}", f.display())),
        }
    }
    eprintln!("{} XML files: {same} identical, {} refused, {} different", files.len(), refused.len(), different.len());
    for r in &refused {
        eprintln!("  refused {r}");
    }
    assert!(files.len() > 1000, "expected the corpus's UFOs and designspace files, found {} files", files.len());
    assert!(different.is_empty(), "written back differently: {different:#?}");
    assert!(refused.len() < 20, "too many refused files");
}

/// Whether the UFO holding `file` says fontTools wrote it.
fn written_by_ufolib(file: &Path) -> bool {
    file.ancestors().find(|a| a.extension().is_some_and(|x| x == "ufo")).is_some_and(|ufo| {
        std::fs::read_to_string(ufo.join("metainfo.plist")).is_ok_and(|m| m.contains("<string>com.github.fonttools.ufoLib</string>"))
    })
}

#[test]
fn ufo_property_lists_write_back_byte_for_byte() {
    use fontcraft_plist::xmlplist;
    let Some(corpus) = corpus() else {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    };
    let mut files = Vec::new();
    xml_files(&corpus, false, &mut files);
    files.retain(|f| f.extension().is_some_and(|x| x == "plist"));
    files.sort();
    let (mut same, mut spellings, mut refused, mut different) = (0, 0, Vec::new(), Vec::new());
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        match xmlplist::parse(&text) {
            Ok(doc) => {
                spellings += doc.root.spelling_count();
                if std::env::var_os("FONTCRAFT_SHOW_SPELLINGS").is_some() {
                    for s in doc.root.spellings(5) {
                        eprintln!("  spelling {s:?} in {}", f.display());
                    }
                }
                if xmlplist::write(&doc) == text {
                    same += 1;
                } else {
                    different.push((written_by_ufolib(f), f.display().to_string(), doc.irregular));
                }
            }
            Err(e) => refused.push(format!("{}: {e}", f.display())),
        }
    }
    eprintln!(
        "{} property lists: {same} identical, {} different, {} refused; {spellings} spellings kept",
        files.len(),
        different.len(),
        refused.len()
    );
    for (by_ufolib, d, irregular) in &different {
        eprintln!("  different{}{}: {d}", if *by_ufolib { " (ufoLib)" } else { "" }, if *irregular > 0 { " (irregular)" } else { "" });
    }
    for r in &refused {
        eprintln!("  refused {r}");
    }
    let ufolib: Vec<_> = different.iter().filter(|(u, _, _)| *u).collect();
    assert!(ufolib.is_empty(), "property lists fontTools wrote come back different: {ufolib:#?}");
}
