//! `audit.results` and `audit.fix` on files (M12.84): the kept run of a path, and a source fixed
//! on disk without opening it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use fontcraft_testkit::rect;
use serde_json::{Value, json};

const OVERLAP: &str = "check.outlines.overlap";

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn refused(s: &mut Session, id: &str, p: Value) -> String {
    match s.execute(id, &p) {
        Ok(r) => panic!("{id} {p} should be refused: {r}"),
        Err(e) => e.to_string(),
    }
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-audit-files-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every file under `dir` with its bytes.
fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        match e.path().is_dir() {
            true => out.extend(tree(&e.path())),
            false => out.push((e.path(), std::fs::read(e.path()).unwrap())),
        }
    }
    out.sort();
    out
}

/// A source with two overlapping squares in A, saved to `dir/name`.
fn source(dir: &Path, name: &str) -> PathBuf {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(0.0, 0.0, 100.0, 100.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(50.0, 50.0, 150.0, 150.0)}));
    let path = dir.join(name);
    run(&mut s, "file.saveAs", json!({"path": path}));
    path
}

fn count(doc: &Value, check: &str) -> usize {
    doc["results"].as_array().unwrap().iter().filter(|r| r["check"] == check).count()
}

#[test]
fn audit_fix_on_a_source_writes_a_fixed_copy_or_the_source_itself() {
    for name in ["Font.glyphs", "Font.ufo", "Font.fontcraft"] {
        let dir = scratch(&name.replace('.', "-"));
        let path = source(&dir, name);
        let before = tree(&dir);
        // Another document is open: it and its undo steps stay as they are.
        let mut s = Session::with_document();
        let open = s.font().unwrap().clone();
        let found = run(&mut s, "audit.run", json!({"target": path, "checks": [OVERLAP]}));
        assert_eq!(count(&found, OVERLAP), 1, "{name}: {found}");
        let fix = found["results"][0]["fix"].clone();
        assert_eq!((&fix["params"]["target"], &fix["params"]["checks"]), (&json!(path), &json!([OVERLAP])), "{name}");

        let mut dry = fix["params"].clone();
        dry["dryRun"] = json!(true);
        let planned = run(&mut s, "audit.fix", dry);
        assert_eq!(planned["planned"][0]["command"], json!("path.removeOverlap"), "{name}: {planned}");
        assert_eq!(tree(&dir), before, "{name}: a dry run writes nothing");

        let copy = run(&mut s, "audit.fix", fix["params"].clone());
        let (stem, ext) = name.split_once('.').unwrap();
        let output = dir.join(format!("{stem}.fixed.{ext}"));
        assert_eq!(copy["output"], json!(output), "{name}: {copy}");
        assert_eq!(copy["applied"][0]["fix"], json!("removeOverlap"), "{name}");
        assert_eq!(count(&copy["remaining"], OVERLAP), 0, "{name}");
        let unchanged: Vec<_> = tree(&dir).into_iter().filter(|(p, _)| !p.starts_with(&output)).collect();
        assert_eq!(unchanged, before, "{name}: the source itself is not changed");
        assert_eq!(count(&run(&mut s, "audit.run", json!({"target": output, "checks": [OVERLAP]})), OVERLAP), 0, "{name}");

        let mut taken = fix["params"].clone();
        taken["noClobber"] = json!(true);
        let written = tree(&dir);
        assert!(refused(&mut s, "audit.fix", taken).contains("noClobber"), "{name}");
        assert_eq!(tree(&dir), written, "{name}: a refused write changes nothing");

        let in_place = run(&mut s, "audit.fix", json!({"target": path, "checks": [OVERLAP], "inPlace": true, "keepOriginal": "none"}));
        assert_eq!(in_place["output"], json!(path), "{name}: {in_place}");
        assert_eq!(count(&run(&mut s, "audit.run", json!({"target": path, "checks": [OVERLAP]})), OVERLAP), 0, "{name}");
        // Nothing left to fix: nothing is written.
        let written = tree(&dir);
        let again = run(&mut s, "audit.fix", json!({"target": path, "checks": [OVERLAP], "inPlace": true}));
        assert_eq!((again["output"].clone(), again["applied"].clone()), (Value::Null, json!([])), "{name}");
        assert_eq!(tree(&dir), written, "{name}");
        assert!(*s.font().unwrap() == open, "{name}: the open document is left alone");
        assert!(s.doc().unwrap().history.undo.is_empty(), "{name}");
    }
}

#[test]
fn audit_fix_on_a_source_refuses_what_it_cannot_write() {
    let dir = scratch("refused");
    let path = source(&dir, "Font.designspace");
    let before = tree(&dir);
    let mut s = Session::new();
    for (p, why) in [
        (json!({"target": path}), "UFOs"),
        (json!({"target": path, "output": dir.join("Out.otf")}), "format"),
        (json!({"target": path, "results": ["r1"], "inPlace": true}), "results"),
        (json!({"target": dir.join("None.glyphs"), "inPlace": true}), "None.glyphs"),
        (json!({"target": [path], "inPlace": true}), "not a compiled font"),
    ] {
        let e = refused(&mut s, "audit.fix", p.clone());
        assert!(e.contains(why), "{p}: {e}");
    }
    assert_eq!(tree(&dir), before, "nothing was written");
    let fixed = run(&mut s, "audit.fix", json!({"target": path, "checks": [OVERLAP], "inPlace": true, "keepOriginal": "none"}));
    assert_eq!(count(&fixed["remaining"], OVERLAP), 0, "{fixed}");
}

/// The run of a path is kept: `audit.results` narrows it without running anything, marks it
/// stale once the file changes, and a `runId` is checked against it.
#[test]
fn audit_results_reads_the_kept_run_of_a_file() {
    let dir = scratch("kept");
    let path = source(&dir, "Font.glyphs");
    let mut s = Session::new();
    assert!(refused(&mut s, "audit.results", json!({"target": path})).contains("audit.run"));
    let ran = run(&mut s, "audit.run", json!({"target": path, "profile": "outlines"}));
    let kept = run(&mut s, "audit.results", json!({"target": path, "runId": ran["runId"]}));
    assert_eq!((&kept["results"], &kept["summary"]["stale"]), (&ran["results"], &json!(0)));
    assert_eq!(count(&run(&mut s, "audit.results", json!({"target": path, "exclude": [OVERLAP]})), OVERLAP), 0);
    assert!(refused(&mut s, "audit.fix", json!({"target": path, "runId": "run0"})).contains("no longer kept"));
    assert!(refused(&mut s, "audit.results", json!({"target": [path, path]})).contains("one path"));

    // The fix runs its own checks, which do not replace the kept run; writing makes it stale.
    let fix = json!({"target": path, "runId": ran["runId"], "checks": [OVERLAP], "inPlace": true, "keepOriginal": "none"});
    run(&mut s, "audit.fix", fix);
    let stale = run(&mut s, "audit.results", json!({"target": path}));
    assert_eq!(stale["runId"], ran["runId"]);
    assert!(stale["results"].as_array().unwrap().iter().all(|r| r["stale"] == true), "{stale}");
    assert_eq!(stale["summary"]["stale"], json!(stale["results"].as_array().unwrap().len()));

    assert_eq!(run(&mut s, "audit.clearCache", json!({"target": path})), json!({"cleared": true}));
    assert!(refused(&mut s, "audit.results", json!({"target": path})).contains("audit.run"));
    // The runs of the eight files audited last are kept.
    run(&mut s, "audit.run", json!({"target": path, "checks": [OVERLAP]}));
    for n in 0..8 {
        let other = source(&dir, &format!("F{n}.glyphs"));
        run(&mut s, "audit.run", json!({"target": other, "checks": [OVERLAP]}));
    }
    run(&mut s, "audit.results", json!({"target": dir.join("F0.glyphs")}));
    assert!(refused(&mut s, "audit.results", json!({"target": path})).contains("audit.run"));
}
