//! Nice names on import (M1.9): opening a UFO or a compiled font renames the glyphs the glyph
//! data knows to their nice names, unless `keepGlyphNames`, the user setting or the font's Use
//! Custom Naming option keeps the file's names; `file.exportUfo` with `productionNames` writes
//! production names and leaves the document's names alone.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Component, Family, Glyph, Layer, Path as Outline, Shape, Snippet};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-import-naming-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Adds the Cyrillic glyphs `a` (U+0410) and `b` (U+0411) with an outline, and `composite`
/// (U+04D2) made of a component of `a`. With `feature`, an ss01 feature substitutes `b`.
fn add_cyrillic(s: &mut Session, [a, b, composite]: [&str; 3], feature: bool) {
    s.edit("fixture", |f| {
        let master = f.masters[0].id.clone();
        for (name, cp) in [(a, 0x410), (b, 0x411)] {
            let mut g = Glyph::new(name);
            g.unicodes.push(cp);
            let mut l = Layer::new(&master, 600.0);
            l.shapes.push(Shape::Path(Outline::rect(50.0, 0.0, 550.0, 700.0)));
            g.layers.push(l);
            f.glyphs.push(g);
        }
        let mut g = Glyph::new(composite);
        g.unicodes.push(0x4D2);
        let mut l = Layer::new(&master, 600.0);
        l.shapes.push(Shape::Component(Component::new(a, 0.0, 0.0)));
        g.layers.push(l);
        f.glyphs.push(g);
        if feature {
            let code = format!("sub space by {b};");
            Arc::make_mut(&mut f.features).features.push(Snippet { name: "ss01".into(), code, ..Default::default() });
        }
        Ok(())
    })
    .unwrap();
}

fn has(s: &Session, name: &str) -> bool {
    s.font().unwrap().glyph(name).is_some()
}

/// The glyph the first shape of `name`'s first layer refers to.
fn component_of(s: &Session, name: &str) -> String {
    match s.font().unwrap().glyph(name).unwrap().layers[0].shapes.first() {
        Some(Shape::Component(c)) => c.glyph.clone(),
        other => panic!("{name}: {other:?}"),
    }
}

#[test]
fn a_ufo_opens_with_nice_names_unless_the_names_are_kept() {
    let dir = temp("ufo");
    let mut source = Session::with_document();
    add_cyrillic(&mut source, ["uni0410", "uni0411", "uni04D2"], true);
    let ufo = dir.join("Imported.ufo");
    run(&mut source, "file.saveAs", json!({ "path": ufo }));

    let mut s = Session::new();
    let r = run(&mut s, "file.open", json!({ "path": ufo }));
    assert_eq!(r["renamed"], json!(2), "{r}");
    assert!(has(&s, "A-cy") && has(&s, "Adieresis-cy") && !has(&s, "uni0410"), "uni0410 becomes A-cy");
    assert_eq!(component_of(&s, "Adieresis-cy"), "A-cy", "components follow the rename");
    assert_eq!(s.font().unwrap().glyph("A-cy").unwrap().unicodes, [0x410]);
    // The feature code names uni0411, so it keeps its name.
    assert!(has(&s, "uni0411"));
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("uni0411")), "{r}");
    assert!(!s.doc().unwrap().modified(), "renaming on opening is part of reading the file");

    let mut kept = Session::new();
    let r = run(&mut kept, "file.open", json!({ "path": ufo, "keepGlyphNames": true }));
    assert_eq!(r["renamed"], json!(0), "{r}");
    assert!(has(&kept, "uni0410") && has(&kept, "uni04D2"));

    let mut setting = Session::new();
    run(&mut setting, "settings.set", json!({ "id": "user.keepGlyphNamesFromImportedFiles", "value": true }));
    let r = run(&mut setting, "file.open", json!({ "path": ufo }));
    assert_eq!(r["renamed"], json!(0), "{r}");
    assert!(has(&setting, "uni0410"));

    // The font's own Use Custom Naming option keeps its names whatever the user setting says.
    run(&mut source, "font.settings.set", json!({ "useCustomNaming": true }));
    let custom = dir.join("Custom.ufo");
    run(&mut source, "file.saveAs", json!({ "path": custom }));
    let mut own = Session::new();
    let r = run(&mut own, "file.open", json!({ "path": custom }));
    assert_eq!(r["renamed"], json!(0), "{r}");
    assert!(has(&own, "uni0410"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_compiled_font_opens_with_nice_names() {
    let dir = temp("compiled");
    let mut source = Session::with_document();
    add_cyrillic(&mut source, ["uni0410", "uni0411", "uni04D2"], false);
    // Use Custom Naming writes the source names into the font.
    run(&mut source, "font.settings.set", json!({ "useCustomNaming": true }));
    let ttf = dir.join("Compiled.ttf");
    run(&mut source, "file.exportStatic", json!({ "destination": ttf }));

    let mut s = Session::new();
    let r = run(&mut s, "file.open", json!({ "path": ttf }));
    assert_eq!((&r["viewOnly"], &r["renamed"]), (&json!(true), &json!(3)), "{r}");
    assert!(has(&s, "A-cy") && has(&s, "Be-cy") && has(&s, "Adieresis-cy"));
    assert_eq!(component_of(&s, "Adieresis-cy"), "A-cy");
    // The glyph order recorded on opening lists the new names.
    let recorded = format!("{:?}", s.font().unwrap().extra.get(Family::Glyphs, "customParameters"));
    assert!(recorded.contains("A-cy") && !recorded.contains("uni0410"), "{recorded}");

    let mut kept = Session::new();
    run(&mut kept, "file.open", json!({ "path": ttf, "keepGlyphNames": true }));
    assert!(has(&kept, "uni0410") && has(&kept, "uni0411"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The text of the glif `name` in the UFO at `ufo`, found through its `contents.plist`.
fn glif(ufo: &Path, name: &str) -> Option<String> {
    let contents = std::fs::read_to_string(ufo.join("glyphs/contents.plist")).unwrap();
    let after = contents.split(&format!("<key>{name}</key>")).nth(1)?;
    let file = after.split("<string>").nth(1)?.split("</string>").next()?;
    std::fs::read_to_string(ufo.join("glyphs").join(file)).ok()
}

#[test]
fn export_ufo_writes_production_names_and_the_document_keeps_its_own() {
    let dir = temp("export");
    let mut s = Session::with_document();
    add_cyrillic(&mut s, ["A-cy", "Be-cy", "Adieresis-cy"], false);

    let out = run(&mut s, "file.exportUfo", json!({ "destination": dir.join("production"), "productionNames": true }));
    let ufo = PathBuf::from(out["files"][0]["path"].as_str().unwrap());
    assert!(glif(&ufo, "uni0410").is_some() && glif(&ufo, "A-cy").is_none(), "{out}");
    assert!(glif(&ufo, "uni04D2").unwrap().contains("base=\"uni0410\""), "the component follows");
    assert!(glif(&ufo, "A").is_some(), "a glyph whose production name is its name stays");
    assert!(out["losses"].as_array().unwrap().iter().all(|l| !l["what"].as_str().unwrap().contains("own names")), "{out}");
    assert!(has(&s, "A-cy") && !has(&s, "uni0410"), "the document keeps its names");

    let out = run(&mut s, "file.exportUfo", json!({ "destination": dir.join("nice") }));
    let ufo = PathBuf::from(out["files"][0]["path"].as_str().unwrap());
    assert!(glif(&ufo, "A-cy").is_some() && glif(&ufo, "uni0410").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

fn corpus() -> PathBuf {
    std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from)
}

/// Compiled fonts under `dir`, at most `depth` folders down.
fn binaries(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth > 0 {
                binaries(&path, depth - 1, out);
            }
        } else if path
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| matches!(x.to_ascii_lowercase().as_str(), "ttf" | "otf" | "woff" | "woff2"))
        {
            out.push(path);
        }
    }
}

/// Every corpus binary that opens with its own names also opens with nice names, with as many
/// glyphs and a font that passes the model's checks.
/// `cargo test -p fontcraft-engine --test engine import_naming:: -- --ignored`.
#[test]
#[ignore = "needs the fetched corpus"]
fn every_corpus_binary_still_opens_with_nice_names() {
    let mut fonts = Vec::new();
    binaries(&corpus(), 8, &mut fonts);
    fonts.sort();
    let (mut opened, mut renamed) = (0, 0);
    for path in &fonts {
        let mut kept = Session::new();
        let Ok(as_read) = kept.execute("file.open", &json!({ "path": path, "keepGlyphNames": true })) else { continue };
        let mut s = Session::new();
        let r = s.execute("file.open", &json!({ "path": path })).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(r["glyphs"], as_read["glyphs"], "{}", path.display());
        s.font().unwrap().check().unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        opened += 1;
        renamed += r["renamed"].as_u64().unwrap();
    }
    println!("{opened} of {} corpus binaries opened; {renamed} glyphs renamed", fonts.len());
}
