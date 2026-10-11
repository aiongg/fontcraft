//! Behaviour tests for the reverse component index.

use std::sync::Arc;

use fontcraft_model::{Component, Font, Glyph, GlyphTable, Layer, Shape};

use crate::ComponentIndex;

fn glyph(name: &str, bases: &[&str]) -> Glyph {
    let mut g = Glyph::new(name);
    let mut layer = Layer::new("m01", 600.0);
    layer.shapes = bases.iter().map(|b| Shape::Component(Component::new(*b, 0.0, 0.0))).collect();
    g.layers.push(layer);
    g
}

fn font(glyphs: Vec<Glyph>) -> Font {
    let mut f = Font::new("Index");
    f.glyphs = GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect());
    f
}

fn names(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

/// `edieresis` nests: dieresiscomb is made of two dotaccentcomb.
fn accents() -> Font {
    font(vec![
        glyph("edieresis", &["e", "dieresiscomb"]),
        glyph("e", &[]),
        glyph("dieresiscomb", &["dotaccentcomb", "dotaccentcomb"]),
        glyph("dotaccentcomb", &[]),
        glyph("eacute", &["e", "acutecomb"]),
        glyph("acutecomb", &[]),
    ])
}

#[test]
fn users_follow_nesting_and_the_order_puts_bases_first() {
    let index = ComponentIndex::new(&accents());
    assert_eq!(index.direct_users("dotaccentcomb"), ["dieresiscomb"]);
    assert_eq!(index.users("dotaccentcomb"), ["dieresiscomb", "edieresis"]);
    assert_eq!(index.users("e"), ["edieresis", "eacute"]);
    assert_eq!(index.dependents(["acutecomb", "dotaccentcomb"]), ["dieresiscomb", "edieresis", "eacute"]);
    // Glyphs without components keep their font order; each composite follows its bases.
    assert_eq!(names(index.dependency_order()), ["e", "dotaccentcomb", "dieresiscomb", "edieresis", "acutecomb", "eacute"]);
    assert!(index.cycles().is_empty());
    assert!(!index.is_empty());
}

#[test]
fn a_glyph_that_uses_another_of_the_set_is_its_dependent() {
    let index = ComponentIndex::new(&accents());
    assert_eq!(index.dependents(["dotaccentcomb", "dieresiscomb"]), ["dieresiscomb", "edieresis"]);
}

#[test]
fn the_index_after_a_rename_and_a_removal() {
    let mut f = accents();
    let index = ComponentIndex::new(&f);
    assert!(index.is_current(&f));

    // Renaming e to e.alt in the glyph and in its users (what glyph.rename does).
    if let Some(mut g) = f.glyphs.get_mut("e") {
        g.name = "e.alt".into();
    }
    for user in ["edieresis", "eacute"] {
        if let Some(mut g) = f.glyphs.get_mut(user) {
            for l in &mut g.layers {
                for s in &mut l.shapes {
                    if let Shape::Component(c) = s
                        && c.glyph == "e"
                    {
                        c.glyph = "e.alt".into();
                    }
                }
            }
        }
    }
    assert!(!index.is_current(&f));
    let index = ComponentIndex::new(&f);
    assert_eq!(index.users("e.alt"), ["edieresis", "eacute"]);
    assert!(index.users("e").is_empty());

    // Removing acutecomb leaves eacute with a missing base and no edge.
    f.glyphs.remove("acutecomb");
    let index = ComponentIndex::new(&f);
    assert_eq!(index.missing_bases(), [("eacute".to_string(), "acutecomb".to_string())]);
    assert!(index.users("acutecomb").is_empty());
    assert!(index.dependency_order().iter().any(|g| g == "eacute"));
}

#[test]
fn cycles_are_listed_and_left_out_of_the_order() {
    let f =
        font(vec![glyph("x", &[]), glyph("b", &["c"]), glyph("a", &["b"]), glyph("c", &["a", "x"]), glyph("user", &["a"]), glyph("self", &["self"])]);
    let index = ComponentIndex::new(&f);
    assert_eq!(index.cycles(), [vec!["b".to_string(), "c".to_string(), "a".to_string()], vec!["self".to_string()]]);
    assert_eq!(names(index.dependency_order()), ["x"]);
    assert!(index.in_cycle("a") && index.in_cycle("user") && index.in_cycle("self"));
    assert!(!index.in_cycle("x") && !index.in_cycle("nothere"));
    // Users of a glyph in a cycle still come out, those outside the order last.
    assert_eq!(index.users("x"), ["b", "a", "c", "user"]);
}

#[test]
fn every_layer_counts_and_a_long_chain_does_not_recurse() {
    let mut g = glyph("composite", &["base"]);
    let mut other = Layer::new("m01", 600.0);
    other.id = "brace".into();
    other.shapes.push(Shape::Component(Component::new("other", 0.0, 0.0)));
    g.layers.push(other);
    let mut glyphs = vec![g, glyph("base", &[]), glyph("other", &[])];
    for i in 0..10_000 {
        let base = if i == 0 { "base".to_string() } else { format!("chain{}", i - 1) };
        glyphs.push(glyph(&format!("chain{i}"), &[&base]));
    }
    let index = ComponentIndex::new(&font(glyphs));
    assert_eq!(index.direct_users("other"), ["composite"]);
    assert_eq!(index.users("base").len(), 10_001);
    assert_eq!(index.dependency_order().last().map(String::as_str), Some("chain9999"));
}
