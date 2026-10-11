//! Behaviour of glyph recipes (M6.3): `docs/spec/09-reusing-shapes.md` § 2.5, rows
//! `components.recipeSingleComponent`, `components.recipeBasePlusMarks`,
//! `components.recipeMultipleBases` and `addGlyphs.recipe`.

use fontcraft_glyphdata::GlyphData;
use fontcraft_model::{Font, Shape};

use crate::ComponentsError;
use crate::compose::composite_glyph;
use crate::recipe::{PartRole, Recipe, classify, is_recipe, parse_recipe};

fn data() -> GlyphData {
    GlyphData::new().unwrap()
}

fn recipe(parts: &[&str], target: &str) -> Recipe {
    Recipe { parts: parts.iter().map(|s| s.to_string()).collect(), target: target.into() }
}

#[test]
fn the_three_recipe_forms_parse() {
    assert_eq!(parse_recipe("Eth=Dtail"), Ok(recipe(&["Eth"], "Dtail")));
    assert_eq!(parse_recipe("x+acutecomb=xacute"), Ok(recipe(&["x", "acutecomb"], "xacute")));
    assert_eq!(parse_recipe("d+ezh.connect=dezh"), Ok(recipe(&["d", "ezh.connect"], "dezh")));
    assert_eq!(parse_recipe("  q+tildecomb=qtilde\n"), Ok(recipe(&["q", "tildecomb"], "qtilde")));
    assert!(is_recipe("n+n=n_n") && !is_recipe("Agrave"));
}

#[test]
fn marks_attach_and_everything_else_is_a_base() {
    let mut font = Font::new("Recipes");
    for name in ["acutecomb", "ezh.connect", "dotaccentcomb", "myhat"] {
        font.add_glyph(name, Vec::new(), 0.0).unwrap();
    }
    let d = data();
    assert_eq!(classify(&font, &d, &recipe(&["x", "acutecomb"], "xacute")), vec![PartRole::Base, PartRole::Mark]);
    assert_eq!(classify(&font, &d, &recipe(&["d", "ezh.connect"], "dezh")), vec![PartRole::Base, PartRole::Base]);
    // The first part has nothing to attach to.
    assert_eq!(classify(&font, &d, &recipe(&["dotaccentcomb", "dotaccentcomb"], "dieresiscomb")), vec![PartRole::Base, PartRole::Mark]);
    // A category stored in the font wins over the database.
    assert_eq!(classify(&font, &d, &recipe(&["o", "myhat"], "omyhat")), vec![PartRole::Base, PartRole::Base]);
    font.glyph_mut("myhat").unwrap().info.category = Some("Mark".into());
    assert_eq!(classify(&font, &d, &recipe(&["o", "myhat"], "omyhat")), vec![PartRole::Base, PartRole::Mark]);
}

#[test]
fn a_recipe_builds_its_glyph_from_the_written_parts_in_order() {
    let mut font = Font::new("Recipes");
    font.add_glyph("tildecomb", Vec::new(), 0.0).unwrap();
    let r = parse_recipe("q+tildecomb=qtilde").unwrap();
    let g = composite_glyph(&font, &data(), &r.target, &r.parts).unwrap();
    let parts: Vec<&str> =
        g.layer("m01").unwrap().shapes.iter().filter_map(|s| if let Shape::Component(c) = s { Some(c.glyph.as_str()) } else { None }).collect();
    assert_eq!(parts, ["q", "tildecomb"]);
    let r = parse_recipe("Delta=increment").unwrap();
    assert_eq!(
        composite_glyph(&font, &data(), &r.target, &r.parts),
        Err(ComponentsError::MissingParts { glyph: "increment".into(), missing: vec!["Delta".into()] })
    );
}

#[test]
fn hostile_recipes_are_refused_with_a_message() {
    let bad = |text: &str| matches!(parse_recipe(text), Err(ComponentsError::BadRecipe(_)));
    let limit = |text: &str| matches!(parse_recipe(text), Err(ComponentsError::LimitExceeded { .. }));
    for text in
        ["", "=", "Agrave", "a=", "=a", "a+=b", "+a=b", "a++b=c", "a=b=c", "a+b=c+d", "a + b=c", "a\tb=c", "a=a", "a+b=a", "a/b=c", "a=é", "\u{0}=a"]
    {
        assert!(bad(text), "{text:?} should be refused");
    }
    assert!(limit(&format!("{}=x", "a".repeat(1 << 20))));
    assert!(limit(&format!("{}=x", vec!["a"; crate::MAX_RECIPE_PARTS + 1].join("+"))));
    assert!(parse_recipe(&format!("{}=x", vec!["a"; crate::MAX_RECIPE_PARTS].join("+"))).is_ok());
    assert!(limit(&format!("a={}", "b".repeat(fontcraft_model::MAX_NAME_LEN + 1))));
    // No prefix of a mixed string panics.
    let text = "ä+ö=ü+=+é=a+b.sc=c_d";
    for (i, _) in text.char_indices() {
        let _ = parse_recipe(&text[..i]);
    }
}
