//! Every PostScript hint and TrueType instruction stored in the corpus's Glyphs sources parses
//! into its typed form and writes back unchanged, byte for byte, and its typed form written
//! afresh reads back the same (`docs/plans/M10.md` § 5, hint data round trip). Skipped when
//! `corpus/` has not been fetched (`cargo xtask corpus --fetch`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use fontcraft_hinting::{NodeOrder, PsHint, TtInstruction};
use fontcraft_model::plist::Value;
use fontcraft_model::plist::openstep::{self, Dialect};
use fontcraft_model::{Family, Hint};

fn corpus() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    dir.is_dir().then_some(dir)
}

/// `.glyphs` files and the `.glyph` and `.plist` files of packages.
fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == ".git" {
            continue;
        }
        if p.is_dir() {
            files(&p, out);
        } else if name.ends_with(".glyphs") || name.ends_with(".glyph") {
            out.push(p);
        }
    }
}

/// Every element of every `hints` list in a document.
fn stored_hints<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
    match v {
        Value::Dict(d) => {
            for (k, x) in &d.entries {
                match (k.as_str(), x) {
                    ("hints", Value::Array(a)) => out.extend(a.items.iter()),
                    _ => stored_hints(x, out),
                }
            }
        }
        Value::Array(a) => a.items.iter().for_each(|x| stored_hints(x, out)),
        _ => {}
    }
}

fn text(v: &Value) -> String {
    let mut out = String::new();
    openstep::write_value(&mut out, v, Dialect::Glyphs3);
    out
}

#[test]
fn corpus_hints_parse_and_write_back_unchanged() {
    let Some(corpus) = corpus() else {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    };
    let mut paths = Vec::new();
    files(&corpus, &mut paths);
    let (mut ps, mut tt, mut failures) = (0usize, 0usize, Vec::new());
    for path in &paths {
        let Ok(source) = std::fs::read_to_string(path) else { continue };
        // Only files that hold hints are parsed; the rest of the corpus is the plist tests' job.
        if !source.contains("hints = (") {
            continue;
        }
        let Ok(doc) = openstep::parse(&source) else { continue };
        let mut found = Vec::new();
        stored_hints(&doc.root, &mut found);
        // Values alone, without their layers: the read and write convert node indexes alike.
        let order = NodeOrder::default();
        for value in found {
            let hint = Hint { family: Family::Glyphs, value: value.clone() };
            let at = format!("{}: {}", path.display(), text(value).replace('\n', " "));
            if let Some(r) = PsHint::from_hint(0, &hint, &order) {
                ps += 1;
                match r {
                    Ok(h) => {
                        if text(&h.to_hint(Family::Glyphs, &order).value) != text(value) {
                            failures.push(format!("{at}: written back differently"));
                        }
                        let fresh = PsHint { source: None, ..h.clone() }.to_hint(Family::Glyphs, &order);
                        if !PsHint::from_hint(0, &fresh, &order).and_then(Result::ok).is_some_and(|again| again.same_as(&h)) {
                            failures.push(format!("{at}: written afresh, it reads back differently"));
                        }
                    }
                    Err(e) => failures.push(format!("{at}: {e}")),
                }
            } else if let Some(r) = TtInstruction::from_hint(0, &hint, &order) {
                tt += 1;
                match r {
                    Ok(t) => {
                        if text(&t.to_hint(Family::Glyphs, &order).value) != text(value) {
                            failures.push(format!("{at}: written back differently"));
                        }
                        let fresh = TtInstruction { source: None, ..t.clone() }.to_hint(Family::Glyphs, &order);
                        if !TtInstruction::from_hint(0, &fresh, &order).and_then(Result::ok).is_some_and(|again| again.same_as(&t)) {
                            failures.push(format!("{at}: written afresh, it reads back differently"));
                        }
                    }
                    Err(e) => failures.push(format!("{at}: {e}")),
                }
            }
        }
    }
    eprintln!("{ps} PostScript hints and {tt} TrueType instructions checked");
    assert!(failures.is_empty(), "{} of {} hints differ:\n{}", failures.len(), ps + tt, failures.join("\n"));
    assert!(ps > 0 && tt > 0, "the format samples hold both kinds");
}
