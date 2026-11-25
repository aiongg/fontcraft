//! The glyph set: glyph.addGlyphs, glyph.duplicateGlyph, glyph.removeGlyph with its answers,
//! glyphInfo.addToFont, whole glyphs through edit.copy and edit.paste, and edit.pasteSpecial.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Component, Glyph, Shape, Transform};
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
    assert_eq!(skipped, ["1abc", "x+acutecomb=xacute"], "the name is invalid; the font has no acutecomb for the recipe");
    // Code points come from the glyph data; a name it does not know gets none.
    assert_eq!(glyph(&s, "Ccedilla").unicodes, [0xC7]);
    assert_eq!(glyph(&s, "idotless").unicodes, [0x131]);
    assert!(glyph(&s, "myGlyph").unicodes.is_empty());
    // Letters go among those of their script in sort order, a name the data does not know to the end.
    let order = names(&s);
    let at = |name: &str| order.iter().position(|n| n == name).unwrap();
    for (before, new, after) in [("C", "Ccedilla", "D"), ("a", "adieresis", "b"), ("S", "Scommaaccent", "T"), ("t", "tcommaaccent", "u")] {
        assert!(at(before) < at(new) && at(new) < at(after), "{new}: {order:?}");
    }
    assert_eq!(order.last().map(String::as_str), Some("myGlyph"));
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

/// The component names on the glyph's first layer, in order.
fn components(s: &Session, name: &str) -> Vec<String> {
    glyph(s, name).layers[0].shapes.iter().filter_map(|sh| if let Shape::Component(c) = sh { Some(c.glyph.clone()) } else { None }).collect()
}

#[test]
fn add_glyphs_builds_recipes_and_composes_database_composites() {
    let mut s = Session::with_document();
    let text = "acutecomb eacute Eth Adieresis x+acutecomb=xacute Eth=Dtail d+e=de q+nothing=qnothing a=b";
    let r = run(&mut s, "glyph.addGlyphs", json!({"text": text}));
    assert_eq!(strings(&r["added"]), ["acutecomb", "eacute", "Eth", "Adieresis", "xacute", "Dtail", "de"]);
    assert_eq!(strings(&r["existing"]), ["b"], "a recipe for a glyph the font has is kept by default");
    let mut composed = strings(&r["composed"]);
    composed.sort();
    assert_eq!(composed, ["Dtail", "de", "eacute", "xacute"]);
    // The parts may be glyphs the same text adds; marks and bases keep the recipe's order.
    assert_eq!(components(&s, "eacute"), ["e", "acutecomb"], "built from the database's decomposition");
    assert_eq!(components(&s, "xacute"), ["x", "acutecomb"]);
    assert_eq!(components(&s, "Dtail"), ["Eth"]);
    assert_eq!(components(&s, "de"), ["d", "e"]);
    // Two bases stand side by side and the composite is as wide as both.
    let de = glyph(&s, "de");
    let d_width = glyph(&s, "d").layers[0].width;
    let Shape::Component(e) = &de.layers[0].shapes[1] else { panic!("de holds a component of e") };
    assert_eq!(e.transform.affine().as_coeffs()[4], d_width);
    assert_eq!(de.layers[0].width, d_width + glyph(&s, "e").layers[0].width);
    // A composite whose parts the font lacks is added empty and listed.
    assert!(glyph(&s, "Adieresis").layers.iter().all(|l| l.shapes.is_empty()));
    let not_composed = r["notComposed"].as_array().unwrap();
    assert_eq!(not_composed.len(), 1, "{not_composed:?}");
    assert_eq!(not_composed[0]["glyph"], "Adieresis");
    assert!(not_composed[0]["reason"].as_str().unwrap().contains("dieresiscomb"));
    // A recipe with a missing part adds nothing.
    let skipped = r["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 1, "{skipped:?}");
    assert_eq!(skipped[0]["item"], "q+nothing=qnothing");
    assert!(skipped[0]["reason"].as_str().unwrap().contains("nothing"));
    assert!(s.font().unwrap().glyph("qnothing").is_none());
    // Code points come from the glyph data; one undo step for everything.
    assert_eq!(glyph(&s, "eacute").unicodes, [0xE9]);
    run(&mut s, "edit.undo", json!({}));
    assert!(["acutecomb", "eacute", "xacute", "de"].iter().all(|n| s.font().unwrap().glyph(n).is_none()));
    run(&mut s, "edit.redo", json!({}));

    // Replace rebuilds a glyph from its recipe; keep leaves it.
    let before = glyph(&s, "xacute");
    let r = run(&mut s, "glyph.addGlyphs", json!({"text": "e+acutecomb=xacute"}));
    assert_eq!(strings(&r["existing"]), ["xacute"]);
    assert!(Arc::ptr_eq(&before, &glyph(&s, "xacute")));
    let r = run(&mut s, "glyph.addGlyphs", json!({"text": "e+acutecomb=xacute", "onExisting": "replace"}));
    assert_eq!(strings(&r["replaced"]), ["xacute"]);
    assert_eq!(components(&s, "xacute"), ["e", "acutecomb"]);
    // Bad recipes are listed with a reason and change nothing.
    let font = s.doc().unwrap().font.clone();
    let r = run(&mut s, "glyph.addGlyphs", json!({"text": "a+b=a +=x a+=y a+b=c=d"}));
    assert_eq!(r["skipped"].as_array().unwrap().len(), 4, "{r}");
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &font));
    // A helper glyph, named with a leading underscore, does not export.
    run(&mut s, "glyph.addGlyphs", json!({"text": "_part.stem x=_part.x"}));
    assert!(!glyph(&s, "_part.stem").export && !glyph(&s, "_part.x").export);
    assert!(glyph(&s, "xacute").export);
}

/// `fontView.paste.componentsRelink`: components of pasted glyphs name glyphs, so they draw the
/// target font's glyphs; a name the target lacks stays as it is until the glyph is added.
#[test]
fn pasted_composites_draw_the_target_fonts_glyphs() {
    let mut source = Session::with_document();
    run(&mut source, "glyph.addGlyphs", json!({"text": "acutecomb x+acutecomb=xacute"}));
    run(&mut source, "path.addRectangle", json!({"glyph": "x", "x": 0, "y": 0, "width": 100, "height": 100}));
    let copied = run(&mut source, "edit.copy", json!({"glyphs": ["xacute"]}));
    let mut target = Session::with_document();
    run(&mut target, "path.addRectangle", json!({"glyph": "x", "x": 50, "y": 0, "width": 300, "height": 500}));
    run(&mut target, "edit.paste", json!({"json": copied["json"]}));
    assert_eq!(components(&target, "xacute"), ["x", "acutecomb"]);
    assert!(target.font().unwrap().outline("xacute", "m01").is_err(), "acutecomb is not in the target yet");
    run(&mut target, "glyph.addGlyphs", json!({"text": "acutecomb"}));
    let font = target.font().unwrap();
    assert_eq!(font.outline("xacute", "m01").unwrap(), font.outline("x", "m01").unwrap(), "the target's x");
}

#[test]
fn new_glyphs_take_the_default_layer_width() {
    use fontcraft_engine::model::Param;
    use fontcraft_engine::model::plist::{Number, Value as PValue};
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let mut m = font.default_master()?.clone();
        m.id = "m02".into();
        m.name = "Bold".into();
        m.params.0.push(Param::new("Default Layer Width", PValue::String("cyrillic: 430".into())));
        Arc::make_mut(&mut font.masters).push(m);
        let params = &mut Arc::make_mut(&mut font.info).params;
        params.0.push(Param::new("Default Layer Width", PValue::Number(Number::int(520))));
        params.0.push(Param::new("Default Layer Width", PValue::String("cyrillic: 400".into())));
        Ok(())
    })
    .unwrap();
    let widths = |s: &Session, name: &str| glyph(s, name).layers.iter().map(|l| (l.id.clone(), l.width)).collect::<Vec<_>>();
    let both = |a: f64, b: f64| vec![("m01".to_string(), a), ("m02".to_string(), b)];
    run(&mut s, "glyph.addGlyphs", json!({"text": "be-cy Ccedilla uni4E00"}));
    assert_eq!(widths(&s, "be-cy"), both(400.0, 430.0), "the script's entry, the master's first");
    assert_eq!(widths(&s, "Ccedilla"), both(520.0, 520.0), "the entry without a script");
    run(&mut s, "glyph.newGlyph", json!({"name": "ve-cy"}));
    assert_eq!(widths(&s, "ve-cy"), both(400.0, 430.0));
    run(&mut s, "glyph.newGlyph", json!({"name": "ge-cy", "width": 250}));
    assert_eq!(widths(&s, "ge-cy"), both(250.0, 250.0), "a stated width wins");
    // Without the parameter, the built-in widths.
    let mut s = Session::with_document();
    run(&mut s, "glyph.newGlyph", json!({"name": "_corner.serif"}));
    run(&mut s, "glyph.addGlyphs", json!({"text": "uni4E00 be-cy"}));
    let first = |s: &Session, name: &str| glyph(s, name).layers[0].width;
    assert_eq!([first(&s, "_corner.serif"), first(&s, "uni4E00"), first(&s, "be-cy")], [200.0, 1000.0, 600.0]);
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
fn remove_glyph_keeps_the_kerning_pairs_that_name_it() {
    let mut s = Fixture::Drawn.session();
    s.edit("kerning", |f| {
        let k = Arc::make_mut(&mut f.kerning);
        k.ltr.set("m01", "A", "V", -40.0);
        k.ltr.set("m01", "@MMK_L_T", "A", -30.0);
        Ok(())
    })
    .unwrap();
    let before = s.font().unwrap().kerning.clone();
    run(&mut s, "glyph.removeGlyph", json!({"glyphs": ["A"], "components": "remove", "metricsKeys": "keep"}));
    assert!(s.font().unwrap().glyph("A").is_none());
    assert_eq!(s.font().unwrap().kerning, before, "pairs naming A stay as obsolete pairs");
    run(&mut s, "edit.undo", json!({}));
    assert!(s.font().unwrap().glyph("A").is_some());
}

/// Twice the signed area of each path of `name`'s first layer (positive: counter-clockwise).
fn path_areas(s: &Session, name: &str) -> Vec<f64> {
    let g = glyph(s, name);
    g.layers[0]
        .shapes
        .iter()
        .filter_map(|sh| if let Shape::Path(p) = sh { Some(p) } else { None })
        .map(|p| {
            let n = &p.nodes;
            (0..n.len()).map(|i| n[i].x * n[(i + 1) % n.len()].y - n[(i + 1) % n.len()].x * n[i].y).sum()
        })
        .collect()
}

#[test]
fn decomposing_a_mirrored_component_keeps_the_outline_direction() {
    let mut s = Fixture::Drawn.session();
    s.edit("mirrored", |f| {
        let mut mirrored = Component::new("A", 0.0, 0.0);
        mirrored.transform = Transform::Matrix([-1.0, 0.0, 0.0, 1.0, 600.0, 0.0]);
        f.glyph_mut("E")?.layers[0].shapes = vec![Shape::Component(Component::new("A", 0.0, 0.0)), Shape::Component(mirrored)];
        Ok(())
    })
    .unwrap();
    let a = path_areas(&s, "A");
    assert!(!a.is_empty());
    run(&mut s, "glyph.removeGlyph", json!({"glyphs": ["A"], "components": "decompose"}));
    let e = path_areas(&s, "E");
    assert_eq!(e.len(), 2 * a.len());
    for (area, original) in e.iter().zip(a.iter().chain(a.iter())) {
        assert!(area.signum() == original.signum() && (area.abs() - original.abs()).abs() < 1e-6, "{e:?} from {a:?}");
    }
}

#[test]
fn add_to_font_adds_database_names() {
    let mut s = Session::with_document();
    let r = run(&mut s, "glyphInfo.addToFont", json!({"names": ["Ccedilla", "A", "Ccedilla"]}));
    assert_eq!((strings(&r["added"]), strings(&r["existing"])), (vec!["Ccedilla".to_string()], vec!["A".to_string()]));
    assert_eq!(glyph(&s, "Ccedilla").unicodes, [0xC7]);
    assert!(s.execute("glyphInfo.addToFont", &json!({"names": ["bad name"]})).is_err());
    // A composite whose parts the font has is built from them.
    run(&mut s, "glyphInfo.addToFont", json!({"names": ["acutecomb"]}));
    let r = run(&mut s, "glyphInfo.addToFont", json!({"names": ["aacute"]}));
    assert_eq!(strings(&r["composed"]), ["aacute"]);
    assert_eq!(components(&s, "aacute"), ["a", "acutecomb"]);
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

#[test]
fn removing_nested_glyphs_decomposes_them_with_their_corner_components() {
    use fontcraft_engine::model::{Node, NodeKind, Path};
    let mut s = Fixture::Drawn.session();
    s.edit("corner and nesting", |f| {
        f.add_glyph("_corner.x", Vec::new(), 0.0)?;
        let corner = [(0.0, 50.0), (-50.0, 50.0), (-50.0, 0.0), (25.0, 0.0)];
        f.glyph_mut("_corner.x")?.layers[0].shapes =
            vec![Shape::Path(Path::new(false, corner.iter().map(|&(x, y)| Node::new(x, y, NodeKind::Line)).collect()))];
        f.glyph_mut("A")?.layers[0].shapes = vec![Shape::Path(Path::rect(100.0, 0.0, 400.0, 500.0))];
        f.glyph_mut("B")?.layers[0].shapes = vec![Shape::Component(Component::new("A", 10.0, 0.0))];
        f.glyph_mut("E")?.layers[0].shapes = vec![Shape::Component(Component::new("B", 0.0, 0.0)), Shape::Component(Component::new("O", 0.0, 0.0))];
        Ok(())
    })
    .unwrap();
    run(&mut s, "cornerComponent.add", json!({"glyph": "A", "points": [[0, 0]], "corner": "_corner.x"}));
    run(&mut s, "glyph.removeGlyph", json!({"glyphs": ["A", "B"], "components": "decompose", "metricsKeys": "keep"}));
    let shapes = &glyph(&s, "E").layers[0].shapes;
    // B and, inside it, A are drawn as paths; O stays a component.
    assert!(matches!(shapes.last(), Some(Shape::Component(c)) if c.glyph == "O"), "{shapes:?}");
    let paths: Vec<&Path> = shapes.iter().filter_map(|sh| if let Shape::Path(p) = sh { Some(p) } else { None }).collect();
    assert_eq!(paths.len(), 1);
    assert!(paths[0].nodes.len() > 4, "the corner component is part of the outline: {:?}", paths[0].nodes);
    assert!(paths[0].nodes.iter().all(|n| n.x >= 50.0), "moved with B's component");
}
