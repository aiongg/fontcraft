//! Name lists (`font.renameGlyphsByNamelist`) and the font comparison table.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::cmd::encodings::{NameSource, PlannedRename, compare_documents, preview_renames, resolve_documents};
use fontcraft_engine::model::{Alignment, Component, ComponentInfo, ComponentInfoBox, Font, Glyph, Layer, Shape};
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// A name-list file in the system's temporary folder, unique to the test.
fn namelist(test: &str, text: &str) -> String {
    let path: PathBuf = std::env::temp_dir().join(format!("fontcraft-namelist-{}-{test}.nam", std::process::id()));
    std::fs::write(&path, text).unwrap();
    path.to_string_lossy().into_owned()
}

/// The default font with `A` drawn, used as a component in `B`, kerned against `V`, and an
/// unencoded `A.sc`.
fn built() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(50.0, 0.0, 300.0, 200.0)}));
    s.edit("references", |f| {
        let mut sc = Glyph::new("A.sc");
        sc.layers.push(Layer::new("m01", 500.0));
        f.glyphs.push(sc);
        {
            let mut b = f.glyph_mut("B")?;
            let mut c = Component::new("A", 100.0, 0.0);
            c.info = ComponentInfoBox::new(ComponentInfo { alignment: Alignment::Disabled, ..Default::default() });
            b.layers[0].shapes.push(Shape::Component(c));
        }
        Arc::make_mut(&mut f.kerning).ltr.set("m01", "A", "V", -40.0);
        Ok(())
    })
    .unwrap();
    s
}

fn component_of_b(s: &Session) -> String {
    match s.font().unwrap().glyph("B").unwrap().layers[0].shapes.last() {
        Some(Shape::Component(c)) => c.glyph.clone(),
        other => panic!("B has no component: {other:?}"),
    }
}

#[test]
fn renaming_by_name_list_updates_components_and_kerning() {
    let file = namelist("refs", "%%FONTLAB NAMETABLE: Test\n0x0041 Alpha\n# a comment\nU+0042 B\n");
    let mut s = built();
    let r = run(&mut s, "font.renameGlyphsByNamelist", json!({"namelist": file}));
    assert_eq!(r["renamed"], json!([{"from": "A", "to": "Alpha"}, {"from": "A.sc", "to": "Alpha.sc"}]), "{r}");
    let font = s.font().unwrap();
    assert!(font.glyph("A").is_none());
    assert_eq!(font.glyph("Alpha").unwrap().unicodes, vec![0x41], "code points stay");
    assert!(font.glyph("Alpha.sc").is_some(), "a suffixed variant follows its base");
    assert_eq!(component_of_b(&s), "Alpha");
    let k = &s.font().unwrap().kerning.ltr;
    assert_eq!(k.get("m01", "Alpha", "V"), Some(-40.0));
    assert_eq!(k.get("m01", "A", "V"), None);

    // One undo step puts every name back.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(*s.font().unwrap(), *built().font().unwrap());

    // Without updateReferences only the glyphs are renamed.
    let mut plain = built();
    run(&mut plain, "font.renameGlyphsByNamelist", json!({"namelist": file, "glyphs": ["A"], "updateReferences": false}));
    assert!(plain.font().unwrap().glyph("Alpha").is_some());
    assert!(plain.font().unwrap().glyph("A.sc").is_some(), "only the glyphs asked for are renamed");
    assert_eq!(component_of_b(&plain), "A");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn renaming_by_name_list_refuses_a_taken_name_and_changes_nothing() {
    let file = namelist("taken", "0x0041 B\n");
    let mut s = built();
    let before = s.doc().unwrap().font.clone();
    let e = s.execute("font.renameGlyphsByNamelist", &json!({"namelist": file})).unwrap_err().to_string();
    assert!(e.contains("taken"), "{e}");
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before));
    let e = s.execute("font.renameGlyphsByNamelist", &json!({"namelist": namelist("bad", "0x0041 A B\n")})).unwrap_err().to_string();
    assert!(e.contains("line 1"), "{e}");
    let _ = std::fs::remove_file(&file);
}

/// A pasted name list (`text`) renames as the same lines in a file do, and the preview lists the same renames,
/// marking the one that clashes, without changing the font.
#[test]
fn a_pasted_name_list_renames_and_previews_as_a_file_does() {
    let mut s = built();
    let data = s.glyph_data().unwrap();
    let before = s.doc().unwrap().font.clone();
    let clash = preview_renames(s.font().unwrap(), &data, NameSource::Text("0x0041 B\n"), None).unwrap().unwrap();
    assert_eq!(
        clash,
        vec![
            PlannedRename { from: "A".into(), to: "B".into(), problem: Some("the name is taken".into()) },
            PlannedRename { from: "A.sc".into(), to: "B.sc".into(), problem: None },
        ]
    );
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "a preview changes nothing");
    let bad = preview_renames(s.font().unwrap(), &data, NameSource::Text("0x0041 A B\n"), None).unwrap();
    assert!(bad.is_err_and(|e| e.contains("line 1")));

    let r = run(&mut s, "font.renameGlyphsByNamelist", json!({"text": "0x0041 Alpha\n"}));
    assert_eq!(r["renamed"], json!([{"from": "A", "to": "Alpha"}, {"from": "A.sc", "to": "Alpha.sc"}]), "{r}");
    let both = s.execute("font.renameGlyphsByNamelist", &json!({"text": "0x0041 A\n", "namelist": "nice"}));
    assert!(both.is_err(), "namelist and text together are refused");
    assert!(s.execute("font.renameGlyphsByNamelist", &json!({})).is_err(), "one of them is required");
}

#[test]
fn renaming_by_the_glyph_data_turns_code_point_names_into_nice_names() {
    let mut s = Session::with_document();
    s.edit("add", |f| {
        let mut g = Glyph::new("uni00C4");
        g.unicodes.push(0xC4);
        g.layers.push(Layer::new("m01", 600.0));
        f.glyphs.push(g);
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "font.renameGlyphsByNamelist", json!({"namelist": "nice"}));
    assert_eq!(r["renamed"], json!([{"from": "uni00C4", "to": "Adieresis"}]), "{r}");
}

#[test]
fn the_comparison_table_has_a_width_column_per_font() {
    let mut s = Session::with_document();
    let mut other = Font::new("Other");
    {
        let mut a = other.glyph_mut("A").unwrap();
        a.layers[0].width = 650.0;
    }
    other.glyphs.push({
        let mut g = Glyph::new("extra");
        g.layers.push(Layer::new("m01", 100.0));
        g
    });
    s.add_document(other, None).unwrap();
    let ids = resolve_documents(&s, None).unwrap();
    assert_eq!(ids.len(), 2);
    let table = compare_documents(&s, &ids, "width").unwrap();
    assert_eq!(table["fonts"].as_array().unwrap().len(), 2);
    let row = |name: &str| table["rows"].as_array().unwrap().iter().find(|r| r["glyph"] == name).cloned().unwrap();
    assert_eq!(row("A")["values"], json!([600.0, 650.0]));
    assert_eq!(row("A")["differ"], true);
    assert_eq!(row("B")["differ"], false);
    assert_eq!(row("extra")["values"], json!([null, 100.0]), "a glyph one font lacks is empty there");

    assert_eq!(resolve_documents(&s, Some(&json!([1, 0]))).unwrap(), vec![ids[1], ids[0]]);
    assert!(resolve_documents(&s, Some(&json!([0, 0]))).is_err());
    assert!(resolve_documents(&s, Some(&json!([5]))).is_err());
    assert!(compare_documents(&s, &ids, "colour").is_err());
}

#[test]
fn renaming_by_name_list_follows_the_first_code_point_only() {
    let file = namelist("first", "0xE002 Beta\n0xE003 Gamma\n");
    let mut s = Session::with_document();
    s.edit("add", |f| {
        for (name, unicodes) in [("twin", vec![0xE000, 0xE002]), ("second", vec![0xE003, 0xE001])] {
            let mut g = Glyph::new(name);
            g.unicodes = unicodes;
            g.layers.push(Layer::new("m01", 500.0));
            f.glyphs.push(g);
        }
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "font.renameGlyphsByNamelist", json!({"namelist": file, "glyphs": ["twin", "second"]}));
    assert_eq!(r["renamed"], json!([{"from": "second", "to": "Gamma"}]), "{r}");
    let _ = std::fs::remove_file(&file);
}
