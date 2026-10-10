//! The glyph set: glyph.addGlyphs, glyph.duplicateGlyph, glyph.removeGlyph with its answers,
//! glyphInfo.addToFont, whole glyphs through edit.copy and edit.paste, and edit.pasteSpecial.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Component, Glyph, Shape};
use fontcraft_testkit::{Fixture, rect};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn names(s: &Session) -> Vec<String> {
    s.font().unwrap().glyphs.iter().map(|g| g.name.clone()).collect()
}

fn glyph(s: &Session, name: &str) -> Arc<Glyph> {
    s.font().unwrap().glyph(name).unwrap_or_else(|| panic!("no glyph {name}")).clone()
}

/// The x of every node of the glyph's first layer, and its width.
fn xs(s: &Session, name: &str) -> (Vec<f64>, f64) {
    let g = glyph(s, name);
    let layer = &g.layers[0];
    let xs =
        layer.shapes.iter().filter_map(|sh| if let Shape::Path(p) = sh { Some(p) } else { None }).flat_map(|p| p.nodes.iter().map(|n| n.x)).collect();
    (xs, layer.width)
}

fn min_max(v: &[f64]) -> (f64, f64) {
    (v.iter().copied().fold(f64::INFINITY, f64::min), v.iter().copied().fold(f64::NEG_INFINITY, f64::max))
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect()
}

#[test]
fn add_glyphs_takes_every_item_kind() {
    let mut s = Session::with_document();
    let r = run(&mut s, "glyph.addGlyphs", json!({"text": "Ccedilla ä\tuni00E4\ndotlessi uni0218:uni021B myGlyph x+acutecomb=xacute 1abc"}));
    let added = ["Ccedilla", "adieresis", "idotless", "Scommaaccent", "scommaaccent", "Tcommaaccent", "tcommaaccent", "myGlyph"];
    assert_eq!(strings(&r["added"]), added);
    let skipped: Vec<&str> = r["skipped"].as_array().unwrap().iter().map(|k| k["item"].as_str().unwrap()).collect();
    assert_eq!(skipped, ["x+acutecomb=xacute", "1abc"], "the recipe waits for components; the name is invalid");
    // Code points come from the glyph data; a name it does not know gets none.
    assert_eq!(glyph(&s, "Ccedilla").unicodes, [0xC7]);
    assert_eq!(glyph(&s, "idotless").unicodes, [0x131]);
    assert!(glyph(&s, "myGlyph").unicodes.is_empty());
    // New glyphs follow the letters in sort order; the glyphs already there keep theirs.
    let order = names(&s);
    let z = order.iter().position(|n| n == "z").unwrap();
    assert_eq!(order[z + 1..], added.map(String::from));
    // One undo step for the lot.
    run(&mut s, "edit.undo", json!({}));
    assert!(s.font().unwrap().glyph("Ccedilla").is_none() && s.font().unwrap().glyph("myGlyph").is_none());

    // Use Custom Naming keeps typed names.
    s.edit("custom naming", |f| {
        Arc::make_mut(&mut f.info).settings.disables_nice_names = Some(true);
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "glyph.addGlyphs", json!({"text": "uni00E4 dotlessi"}));
    assert_eq!(strings(&r["added"]), ["uni00E4", "dotlessi"]);
    assert_eq!(glyph(&s, "uni00E4").unicodes, [0xE4]);
}

#[test]
fn add_glyphs_existing_names_ask_keep_replace_cancel() {
    let mut s = Fixture::Drawn.session();
    let before = s.doc().unwrap().font.clone();
    for answer in ["ask", "cancel"] {
        let r = run(&mut s, "glyph.addGlyphs", json!({"text": "A Ccedilla", "onExisting": answer}));
        assert_eq!((strings(&r["added"]), strings(&r["existing"])), (vec![], vec!["A".to_string()]), "{answer}");
        assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{answer} changes nothing");
    }
    // Without a clash, ask adds as usual.
    let r = run(&mut s, "glyph.addGlyphs", json!({"text": "Eth", "onExisting": "ask"}));
    assert_eq!(strings(&r["added"]), ["Eth"]);
    run(&mut s, "edit.undo", json!({}));

    let a_before = glyph(&s, "A");
    let r = run(&mut s, "glyph.addGlyphs", json!({"text": "A Ccedilla"}));
    assert_eq!((strings(&r["added"]), strings(&r["existing"])), (vec!["Ccedilla".to_string()], vec!["A".to_string()]), "keep is the default");
    assert!(Arc::ptr_eq(&a_before, &glyph(&s, "A")));
    run(&mut s, "edit.undo", json!({}));

    let at = names(&s).iter().position(|n| n == "A").unwrap();
    let r = run(&mut s, "glyph.addGlyphs", json!({"text": "A Ccedilla", "onExisting": "replace"}));
    assert_eq!((strings(&r["added"]), strings(&r["replaced"])), (vec!["Ccedilla".to_string()], vec!["A".to_string()]));
    let a = glyph(&s, "A");
    assert!(a.layers.iter().all(|l| l.shapes.is_empty()), "a fresh empty glyph");
    assert_eq!((a.unicodes.as_slice(), a.layers[0].width), (&[0x41][..], 600.0));
    assert_eq!(names(&s).iter().position(|n| n == "A"), Some(at), "in the old glyph's place");
}

#[test]
fn add_glyphs_ranges_at_the_cap_and_hostile_text() {
    let mut s = Session::with_document();
    let before = s.font().unwrap().glyphs.len();
    // Plane 14 spans exactly 65,536 code points; only the assigned ones become glyphs.
    let r = run(&mut s, "glyph.addGlyphs", json!({"text": "uE0000:uEFFFF"}));
    let added = r["added"].as_array().unwrap().len();
    assert!(added > 300 && added < 400, "{added}");
    assert_eq!(s.font().unwrap().glyphs.len(), before + added);

    let font = s.doc().unwrap().font.clone();
    for text in ["uE0000:uF0000".to_string(), "uni0000:u10FFFF".to_string(), "a ".repeat(500_000), "a ".repeat(70_000)] {
        let e = s.execute("glyph.addGlyphs", &json!({"text": text})).unwrap_err().to_string();
        assert!(e.contains("limit"), "{e}");
        assert!(Arc::ptr_eq(&s.doc().unwrap().font, &font));
    }
    assert!(s.execute("glyph.addGlyphs", &json!({"text": " \n\t"})).is_err());
    assert!(s.execute("glyph.addGlyphs", &json!({"text": "A", "onExisting": "merge"})).is_err());
}

#[test]
fn duplicate_glyph_names_copies_with_counter_suffixes() {
    let mut s = Fixture::Drawn.session();
    let r = run(&mut s, "glyph.duplicateGlyph", json!({"glyphs": ["A", "O"]}));
    assert_eq!(strings(&r["glyphs"]), ["A.001", "O.001"]);
    let r = run(&mut s, "glyph.duplicateGlyph", json!({"glyphs": ["A"]}));
    assert_eq!(strings(&r["glyphs"]), ["A.002"]);
    let order = names(&s);
    let a = order.iter().position(|n| n == "A").unwrap();
    assert_eq!(order[a + 1..a + 3], ["A.002".to_string(), "A.001".to_string()], "each copy right after its original");
    let copy = glyph(&s, "A.001");
    assert!(copy.unicodes.is_empty(), "the copy has no code points");
    assert_eq!(copy.layers, glyph(&s, "A").layers, "and the same drawing");
    let r = run(&mut s, "glyph.duplicateGlyph", json!({"glyphs": ["A"], "suffix": "ss01"}));
    assert_eq!(strings(&r["glyphs"]), ["A.ss01"]);
    let r = run(&mut s, "glyph.duplicateGlyph", json!({"glyphs": ["A"], "suffix": ".ss01"}));
    assert_eq!(strings(&r["glyphs"]), ["A.ss01.001"]);
    assert!(s.execute("glyph.duplicateGlyph", &json!({"glyphs": ["A"], "suffix": "a b"})).is_err());
}

/// A font where B uses A as a component (moved 10 units right) and C's left metrics key is `=A`.
fn font_with_references() -> Session {
    let mut s = Fixture::Drawn.session();
    s.edit("references", |f| {
        f.glyph_mut("B")?.layers[0].shapes.push(Shape::Component(Component::new("A", 10.0, 0.0)));
        f.glyph_mut("C")?.info.metrics_keys.left = Some("=A".into());
        f.glyph_mut("D")?.info.metrics_keys.right = Some("=O-4".into());
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn remove_glyph_asks_about_components_and_metrics_keys() {
    let mut s = font_with_references();
    let before = s.doc().unwrap().font.clone();
    let e = s.execute("glyph.removeGlyph", &json!({"glyphs": ["A"]})).unwrap_err().to_string();
    assert!(e.contains("`components`") && e.contains("`metricsKeys`") && e.contains("`B`") && e.contains("`C`"), "{e}");
    let e = s.execute("glyph.removeGlyph", &json!({"glyphs": ["A"], "components": "remove"})).unwrap_err().to_string();
    assert!(!e.contains("`components`") && e.contains("`metricsKeys`"), "{e}");
    let r = run(&mut s, "glyph.removeGlyph", json!({"glyphs": ["A"], "confirm": false}));
    assert_eq!(r["needs"], json!(["components", "metricsKeys"]));
    assert_eq!(r["usedAsComponent"], json!([{"glyph": "B", "layer": "m01", "component": "A"}]));
    assert_eq!(r["metricsKeys"], json!([{"glyph": "C", "key": "left", "value": "=A"}]));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before));

    // Remove and keep.
    run(&mut s, "glyph.removeGlyph", json!({"glyphs": ["A"], "components": "remove", "metricsKeys": "keep"}));
    assert!(s.font().unwrap().glyph("A").is_none());
    assert!(glyph(&s, "B").layers[0].shapes.is_empty());
    assert_eq!(glyph(&s, "C").info.metrics_keys.left.as_deref(), Some("=A"));
    run(&mut s, "edit.undo", json!({}));

    // Decompose and dissolve.
    run(&mut s, "glyph.removeGlyph", json!({"glyphs": ["A"], "components": "decompose", "metricsKeys": "dissolve"}));
    let (b, _) = xs(&s, "B");
    assert_eq!(min_max(&b), (70.0, 570.0), "A's outline, moved with the component");
    assert!(!glyph(&s, "B").layers[0].shapes.iter().any(|sh| matches!(sh, Shape::Component(_))));
    assert_eq!(glyph(&s, "C").info.metrics_keys.left, None);

    // A key with arithmetic names its glyph too.
    let r = run(&mut s, "glyph.removeGlyph", json!({"glyphs": ["O"], "confirm": false}));
    assert_eq!(r["metricsKeys"], json!([{"glyph": "D", "key": "right", "value": "=O-4"}]));
    // Nothing to ask: no answer needed.
    run(&mut s, "glyph.removeGlyph", json!({"glyphs": ["Z"]}));
    assert!(s.font().unwrap().glyph("Z").is_none());
}

#[test]
fn add_to_font_adds_database_names() {
    let mut s = Session::with_document();
    let r = run(&mut s, "glyphInfo.addToFont", json!({"names": ["Ccedilla", "A", "Ccedilla"]}));
    assert_eq!((strings(&r["added"]), strings(&r["existing"])), (vec!["Ccedilla".to_string()], vec!["A".to_string()]));
    assert_eq!(glyph(&s, "Ccedilla").unicodes, [0xC7]);
    assert!(s.execute("glyphInfo.addToFont", &json!({"names": ["bad name"]})).is_err());
}

#[test]
fn copy_and_paste_whole_glyphs_into_a_font_that_has_the_names() {
    let mut source = Fixture::Drawn.session();
    let copied = run(&mut source, "edit.copy", json!({"glyphs": ["A", "extra"]}));
    let text = copied["json"].as_str().unwrap().to_string();
    assert_eq!(strings(&copied["glyphs"]), ["A", "extra"]);

    let mut s = Session::with_document();
    let r = run(&mut s, "edit.paste", json!({"json": text}));
    assert_eq!(r["flavour"], "glyphs");
    assert_eq!(strings(&r["glyphs"]), ["A.001", "extra"]);
    assert_eq!(r["renamed"], json!([{"from": "A", "to": "A.001"}]));
    assert!(glyph(&s, "A").layers[0].shapes.is_empty(), "the glyph already there is untouched");
    let pasted = glyph(&s, "A.001");
    assert!(pasted.unicodes.is_empty(), "a renamed glyph drops its code points");
    assert_eq!(xs(&s, "A.001"), xs(&source, "A"));
    assert_eq!(glyph(&s, "extra").unicodes, [0xE000]);
    // One undo step.
    run(&mut s, "edit.undo", json!({}));
    assert!(s.font().unwrap().glyph("A.001").is_none() && s.font().unwrap().glyph("extra").is_none());
    // The same text through `text`, pasted twice: A.001, then A.002.
    run(&mut s, "edit.paste", json!({"text": text}));
    let r = run(&mut s, "edit.paste", json!({"text": text}));
    assert_eq!(strings(&r["glyphs"]), ["A.002", "extra.001"]);
    // Damaged text is refused.
    let broken = text.replace("\"glyphs\":[", "\"glyphs\":7,\"x\":[");
    assert!(s.execute("edit.paste", &json!({"json": broken})).is_err());
}

#[test]
fn paste_special_modes_and_parts() {
    let mut source = Fixture::Drawn.session();
    source
        .edit("groups and keys", |f| {
            let mut a = f.glyph_mut("A")?;
            a.kern_groups.left = Some("A".into());
            a.info.metrics_keys.right = Some("=H".into());
            Ok(())
        })
        .unwrap();
    let text = run(&mut source, "edit.copy", json!({"glyphs": ["A", "O", "extra"]}))["json"].as_str().unwrap().to_string();
    let special = |s: &mut Session, p: Value| {
        let mut p = p;
        p["json"] = json!(text);
        run(s, "edit.pasteSpecial", p)
    };

    // Glyphs With Same Name: A and O overwritten whole (names and code points kept), extra added.
    let mut s = Session::with_document();
    let r = special(&mut s, json!({"mode": "sameName"}));
    assert_eq!((strings(&r["glyphs"]), strings(&r["added"])), (vec!["A".to_string(), "O".to_string()], vec!["extra".to_string()]));
    assert_eq!(xs(&s, "A"), xs(&source, "A"));
    assert_eq!(glyph(&s, "A").unicodes, [0x41]);
    assert_eq!(glyph(&s, "A").kern_groups.left.as_deref(), Some("A"));

    // Selected Glyphs: targets in glyph order (B before Q) take A then O; extra is left over.
    let mut s = Session::with_document();
    let r = special(&mut s, json!({"mode": "selected", "targets": ["Q", "B"]}));
    assert_eq!(strings(&r["glyphs"]), ["B", "Q"]);
    assert_eq!(r["skipped"].as_array().unwrap().len(), 1);
    assert_eq!(xs(&s, "B"), xs(&source, "A"));
    assert_eq!(xs(&s, "Q"), xs(&source, "O"));
    assert!(s.font().unwrap().glyph("A").unwrap().layers[0].shapes.is_empty());

    // Parts: width alone; kerning groups; metrics keys; the active layer.
    let mut s = Session::with_document();
    special(&mut s, json!({"mode": "selected", "targets": ["H"], "parts": ["width"]}));
    assert_eq!(xs(&s, "H"), (vec![], 620.0));
    special(&mut s, json!({"mode": "selected", "targets": ["H"], "parts": ["kerningGroups", "metricsKeys"]}));
    let h = glyph(&s, "H");
    assert_eq!((h.kern_groups.left.as_deref(), h.info.metrics_keys.right.as_deref()), (Some("A"), Some("=H")));
    assert!(h.layers[0].shapes.is_empty());
    special(&mut s, json!({"mode": "selected", "targets": ["H"], "parts": ["activeLayer"]}));
    assert_eq!(xs(&s, "H"), xs(&source, "A"));

    // Side bearings: A has 60 on both sides; H is drawn from 100 to 500 in a 600 advance.
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "H", "nodes": rect(100.0, 0.0, 500.0, 700.0)}));
    special(&mut s, json!({"mode": "selected", "targets": ["H"], "parts": ["lsb"]}));
    let (h, width) = xs(&s, "H");
    assert_eq!((min_max(&h), width), ((60.0, 460.0), 560.0), "moved left by 40, right side bearing kept");
    special(&mut s, json!({"mode": "selected", "targets": ["H"], "parts": ["rsb", "width"]}));
    assert_eq!(xs(&s, "H").1, 520.0, "the right side bearing wins over the width");

    // As Components: each copied glyph becomes a component of each target.
    let mut s = Session::with_document();
    let r = special(&mut s, json!({"mode": "asComponents", "targets": ["B", "A"]}));
    assert_eq!(strings(&r["glyphs"]), ["B", "A"]);
    let comps = |s: &Session, g: &str| -> Vec<String> {
        glyph(s, g).layers[0].shapes.iter().filter_map(|sh| if let Shape::Component(c) = sh { Some(c.glyph.clone()) } else { None }).collect()
    };
    assert_eq!(comps(&s, "B"), ["A", "O", "extra"]);
    assert_eq!(comps(&s, "A"), ["O", "extra"], "A does not get a component of itself");

    // Refusals change nothing.
    let mut s = Session::with_document();
    let before = s.doc().unwrap().font.clone();
    for p in [
        json!({"mode": "selected"}),
        json!({"mode": "other"}),
        json!({"mode": "sameName", "parts": ["everything"]}),
        json!({"mode": "sameName", "parts": []}),
        json!({"mode": "asComponents"}),
    ] {
        let mut p = p;
        p["json"] = json!(text);
        assert!(s.execute("edit.pasteSpecial", &p).is_err(), "{p}");
    }
    assert!(s.execute("edit.pasteSpecial", &json!({"mode": "sameName", "json": "{}"})).is_err());
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before));
}
