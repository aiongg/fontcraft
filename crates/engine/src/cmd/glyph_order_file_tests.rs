#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::sync::Arc;

use fontcraft_model::{Component, Font, Path, Shape};
use serde_json::json;

use crate::Session;

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fontcraft-glyph-order-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn source() -> Font {
    let mut f = Font::new("Order Sans");
    // .notdef and the three letters only: the new font's alphabet would add Z and space.
    f.glyphs.retain(|g| g.name == ".notdef");
    for (name, cp) in [("A", 0x41), ("B", 0x42), ("C", 0x43)] {
        f.add_glyph(name, vec![cp], 600.0).unwrap();
        f.glyph_mut(name).unwrap().layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 500.0, 700.0)));
    }
    f.glyph_mut("C").unwrap().layers[0].shapes.clear();
    f.glyph_mut("C").unwrap().layers[0].shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
    Arc::make_mut(&mut f.kerning).ltr.set("m01", "A", "B", -40.0);
    f
}

fn ttf(dir: &std::path::Path) -> PathBuf {
    let f = source();
    let out = fontcraft_compile::compile_ttf(&f, &fontcraft_compile::Options::default()).unwrap();
    let path = dir.join("Order.ttf");
    std::fs::write(&path, out.bytes).unwrap();
    path
}

fn names(path: &std::path::Path) -> Vec<String> {
    let bytes = std::fs::read(path).unwrap();
    fontcraft_tables::reorder::glyph_names(&fontcraft_tables::read(&bytes, 0).unwrap()).unwrap()
}

#[test]
fn font_rename_glyphs_renames_a_compiled_font_by_goadb_and_pairs() {
    let d = dir("rename");
    let input = ttf(&d);
    let before = std::fs::read(&input).unwrap();
    std::fs::write(d.join("GlyphOrderAndAliasDB"), "# final source\n.notdef .notdef\nAlpha A uni0041\nBeta B\nZeta Z\n").unwrap();
    let mut s = Session::new();
    let r = s
        .execute(
            "font.renameGlyphs",
            &json!({"font": input, "goadb": d.join("GlyphOrderAndAliasDB"), "renames": {"Beta": "Bee"}, "output": d.join("out.ttf")}),
        )
        .unwrap();
    assert_eq!(r["missing"], json!(["Z"]), "{r}");
    let out = names(&d.join("out.ttf"));
    assert!(out.contains(&"Alpha".to_string()) && out.contains(&"Bee".to_string()) && !out.contains(&"A".to_string()), "{out:?}");
    assert_eq!(std::fs::read(&input).unwrap(), before, "the input is never changed");

    let clash = s.execute("font.renameGlyphs", &json!({"font": input, "renames": {"A": "B"}, "output": d.join("clash.ttf")}));
    assert!(clash.is_err());
    assert!(!d.join("clash.ttf").exists(), "a failed rename writes nothing");
}

#[test]
fn font_reorder_glyphs_reorders_a_compiled_font_and_a_source() {
    let d = dir("reorder");
    let input = ttf(&d);
    let mut s = Session::new();
    let r = s.execute("font.reorderGlyphs", &json!({"font": input, "order": ["C", "B"], "output": d.join("out.ttf")})).unwrap();
    let out = names(&d.join("out.ttf"));
    assert_eq!(out[..3], [".notdef", "C", "B"], "{out:?}");
    assert_eq!(r["order"][1][1], json!(1), "pairs are [old index, new index]: {r}");
    let sorted = s.execute("font.reorderGlyphs", &json!({"font": d.join("out.ttf"), "sort": "unicode", "output": d.join("sorted.ttf")})).unwrap();
    assert!(sorted["output"].is_string());
    assert_eq!(names(&d.join("sorted.ttf"))[1..4], ["A", "B", "C"]);
    assert!(s.execute("font.reorderGlyphs", &json!({"font": input, "order": ["nope"], "output": d.join("bad.ttf")})).is_err());
    assert!(s.execute("font.reorderGlyphs", &json!({"font": input, "order": ["A"], "sort": "name"})).is_err(), "one way to give the order");

    // A source: the glyphs move and the kerning keeps its names.
    let src = d.join("Order.fontcraft");
    crate::io::save(&source(), &src, crate::io::SaveFormat::Native).unwrap();
    s.execute("font.reorderGlyphs", &json!({"font": src, "order": ["C"], "output": d.join("moved.fontcraft")})).unwrap();
    let (moved, _) = crate::io::open(&d.join("moved.fontcraft"), None).unwrap();
    let order: Vec<&str> = moved.glyphs.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(order.iter().position(|n| *n == "C"), Some(order.iter().position(|n| *n == ".notdef").map_or(0, |i| i + 1)), "{order:?}");
}

#[test]
fn font_rename_glyphs_renames_a_source_with_its_references() {
    let d = dir("rename-source");
    let src = d.join("Order.fontcraft");
    crate::io::save(&source(), &src, crate::io::SaveFormat::Native).unwrap();
    let mut s = Session::new();
    s.execute("font.renameGlyphs", &json!({"font": src, "renames": {"A": "Alpha"}, "output": d.join("out.fontcraft")})).unwrap();
    let (font, _) = crate::io::open(&d.join("out.fontcraft"), None).unwrap();
    assert!(font.glyph("Alpha").is_some() && font.glyph("A").is_none());
    let c = font.glyph("C").unwrap();
    assert!(matches!(&c.layers[0].shapes[0], Shape::Component(comp) if comp.glyph == "Alpha"), "components follow");
}
