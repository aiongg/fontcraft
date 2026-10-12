//! Automation roots (`file_access`): while they are in force, commands read only inside the read
//! root and write only inside the write root, and files a document links to outside the read root
//! are treated as missing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::file_access::{AutomationRoots, confine};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn refused(s: &mut Session, id: &str, p: Value) -> String {
    match s.execute(id, &p) {
        Ok(v) => panic!("{id} {p} ran: {v}"),
        Err(e) => e.to_string(),
    }
}

/// `in` (the read root), `out` (the write root) and `outside`, each holding a saved font.
fn folders(name: &str) -> (PathBuf, Arc<AutomationRoots>) {
    let base = std::env::temp_dir().join(format!("fontcraft-automation-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for d in ["in", "out", "outside"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    let mut s = Session::with_document();
    run(&mut s, "glyph.newGlyph", json!({"name": "zz.linked"}));
    run(&mut s, "path.addPath", json!({"glyph": "zz.linked", "nodes": [{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}]}));
    for d in ["in", "outside"] {
        run(&mut s, "file.saveAs", json!({"path": base.join(d).join("Font.glyphs")}));
    }
    let roots = AutomationRoots::new(Some(&base.join("in")), Some(&base.join("out"))).unwrap().unwrap();
    (base, roots)
}

#[test]
fn commands_read_inside_the_read_root_and_write_inside_the_write_root() {
    let (base, roots) = folders("commands");
    let inside = base.join("in/Font.glyphs");
    let outside = base.join("outside/Font.glyphs");
    let mut s = Session::new();
    confine(Some(&roots), || {
        let e = refused(&mut s, "file.open", json!({"path": outside}));
        assert!(e.contains("automation path rejected: outside the read root"), "{e}");
        run(&mut s, "file.open", json!({"path": inside}));
        // The document's own file lies in the read root, not the write root.
        let e = refused(&mut s, "file.save", json!({"force": true}));
        assert!(e.contains("writes to or beside the open document's file") && e.contains("outside the write root"), "{e}");
        let e = refused(&mut s, "file.saveAs", json!({"path": base.join("in/Copy.glyphs")}));
        assert!(e.contains("outside the write root"), "{e}");
        assert!(!base.join("in/Copy.glyphs").exists(), "a refused command writes nothing");
        run(&mut s, "file.saveAs", json!({"path": base.join("out/Copy.glyphs")}));
        assert!(base.join("out/Copy.glyphs").exists());
        // Each step of a batch is checked; the refusal names the step.
        let e = refused(&mut s, "command.batch", json!({"steps": [{"command": "file.saveAs", "params": {"path": base.join("outside/x.glyphs")}}]}));
        assert!(e.contains("step 0") && e.contains("outside the write root"), "{e}");
        let e = refused(&mut s, "script.run", json!({"macro": {"steps": []}}));
        assert!(e.contains("cannot be checked against the automation roots"), "{e}");
        // Without a path, an export returns its bytes as it always does.
        let built = run(&mut s, "file.exportStatic", json!({"timestamp": 0}));
        assert!(built["fonts"].as_array().is_some_and(|f| !f.is_empty()), "{built}");
    });
    // Without roots nothing is checked.
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": outside}));
    run(&mut s, "file.saveAs", json!({"path": base.join("in/Copy.glyphs")}));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn a_linked_source_outside_the_read_root_is_missing_at_compile() {
    let (base, roots) = folders("linked");
    let linked = base.join("outside/Font.glyphs");
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": [{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}]}));
    let warnings = |reply: &Value| reply.to_string();
    confine(Some(&roots), || {
        // A link set during the session, after every check of the open document.
        run(&mut s, "font.customParameters.add", json!({"scope": "font", "name": "Import Font", "value": linked.to_string_lossy()}));
        let built = run(&mut s, "font.compile", json!({"timestamp": 0, "destination": base.join("out")}));
        let text = warnings(&built);
        assert!(text.contains("treated as missing") && text.contains("outside the read root"), "{text}");
    });
    assert!(!s.font().unwrap().glyphs.iter().any(|g| g.name.as_str() == "zz.linked"), "the linked glyph was never read");
    // Unconfined, the same compile reads it.
    let built = run(&mut s, "font.compile", json!({"timestamp": 0, "destination": base.join("out")}));
    assert!(!warnings(&built).contains("treated as missing"), "{built}");
    let _ = std::fs::remove_dir_all(base);
}

/// A designspace naming the UFO `source` (relative to the designspace's folder).
fn designspace(source: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<designspace format="5.0">
  <axes><axis tag="wght" name="Weight" minimum="400" maximum="400" default="400"/></axes>
  <sources>
    <source filename="{source}" name="A"><location><dimension name="Weight" xvalue="400"/></location></source>
  </sources>
</designspace>
"#
    )
}

/// Saves a font with the glyph `glyph` as the UFO `path`.
fn save_ufo(path: &std::path::Path, glyph: &str) {
    let mut s = Session::with_document();
    run(&mut s, "glyph.newGlyph", json!({"name": glyph}));
    run(&mut s, "path.addPath", json!({"glyph": glyph, "nodes": [{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}]}));
    run(&mut s, "file.saveAs", json!({"path": path}));
}

#[test]
fn a_designspace_source_above_its_folder_is_not_written_under_roots() {
    let base = std::env::temp_dir().join(format!("fontcraft-automation-dsup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for d in ["src", "out", "masters"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    save_ufo(&base.join("masters/A.ufo"), "zz.one");
    std::fs::write(base.join("src/x.designspace"), designspace("../masters/A.ufo")).unwrap();
    let written = |name: &str| std::fs::read_dir(base.join("masters/A.ufo/glyphs")).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with(name));
    let roots = AutomationRoots::new(Some(&base), Some(&base.join("out"))).unwrap().unwrap();
    let mut s = Session::new();
    confine(Some(&roots), || {
        run(&mut s, "file.open", json!({"path": base.join("src/x.designspace")}));
        run(&mut s, "glyph.newGlyph", json!({"name": "zz.two"}));
        // The designspace goes into the write root; its source `../masters/A.ufo` would not.
        let e = refused(&mut s, "file.saveAs", json!({"path": base.join("out/x.designspace")}));
        assert!(e.contains("../masters/A.ufo") && e.contains("confined"), "{e}");
    });
    assert!(!written("zz.two"), "the UFO outside the write root was not written");
    assert!(!base.join("out/x.designspace").exists(), "a refused save writes nothing");
    // Unconfined, the same save writes the UFO beside the designspace's folder, as designspaces expect.
    run(&mut s, "file.saveAs", json!({"path": base.join("out/x.designspace")}));
    assert!(written("zz.two"));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn a_linked_designspace_reads_no_ufo_outside_the_read_root() {
    let (base, roots) = folders("linked-ds");
    save_ufo(&base.join("outside/Secret.ufo"), "zz.secret");
    let linked = base.join("in/x.designspace");
    std::fs::write(&linked, designspace("../outside/Secret.ufo")).unwrap();
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": [{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}]}));
    run(&mut s, "font.customParameters.add", json!({"scope": "font", "name": "Import Font", "value": linked.to_string_lossy()}));
    let has_secret = |s: &Session| s.font().unwrap().glyphs.iter().any(|g| g.name.as_str() == "zz.secret");
    confine(Some(&roots), || {
        let built = run(&mut s, "font.compile", json!({"timestamp": 0, "destination": base.join("out")}));
        let text = built.to_string();
        assert!(text.contains("Secret.ufo") && text.contains("outside the read root"), "{text}");
        // Opening the designspace itself is refused before anything is read.
        let e = refused(&mut Session::new(), "file.open", json!({"path": linked}));
        assert!(e.contains("outside the read root"), "{e}");
    });
    assert!(!has_secret(&s), "the UFO outside the read root was never read");
    let built = run(&mut s, "font.compile", json!({"timestamp": 0, "destination": base.join("out")}));
    assert!(!built.to_string().contains("outside the read root"), "{built}");
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn a_link_inside_a_ufo_does_not_lead_a_save_out_of_the_write_root() {
    let base = std::env::temp_dir().join(format!("fontcraft-automation-ufolink-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for d in ["out", "outside"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    let ufo = base.join("out/A.ufo");
    save_ufo(&ufo, "zz.one");
    let roots = AutomationRoots::new(Some(&base), Some(&base.join("out"))).unwrap().unwrap();
    let mut s = Session::new();
    confine(Some(&roots), || {
        run(&mut s, "file.open", json!({"path": ufo}));
        run(&mut s, "glyph.newGlyph", json!({"name": "zz.two"}));
        // The layer folder becomes a link out of the write root after the UFO was read.
        std::fs::rename(ufo.join("glyphs"), base.join("outside/glyphs")).unwrap();
        std::os::unix::fs::symlink(base.join("outside/glyphs"), ufo.join("glyphs")).unwrap();
        let e = refused(&mut s, "file.save", json!({"force": true}));
        assert!(e.contains("outside the write root"), "{e}");
    });
    let names: Vec<_> = std::fs::read_dir(base.join("outside/glyphs")).unwrap().flatten().map(|e| e.file_name()).collect();
    assert!(!names.iter().any(|n| n.to_string_lossy().starts_with("zz.two")), "{names:?}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn the_session_snippet_folder_is_checked_when_no_folder_is_given() {
    let (base, roots) = folders("snippets");
    let mut s = Session::with_document();
    s.set_snippets_dir(Some(base.join("outside/snippets")));
    confine(Some(&roots), || {
        let e = refused(&mut s, "font.features.listSnippets", json!({}));
        assert!(e.contains("snippet folder") && e.contains("outside the read root"), "{e}");
        let e = refused(&mut s, "font.features.showSnippetFolder", json!({}));
        assert!(e.contains("outside the write root"), "{e}");
        run(&mut s, "font.features.listSnippets", json!({"folder": base.join("in/snippets")}));
    });
    assert!(!base.join("outside/snippets").exists(), "a refused command creates no folder");
    run(&mut s, "font.features.listSnippets", json!({}));
    let _ = std::fs::remove_dir_all(base);
}
