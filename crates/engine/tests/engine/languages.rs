//! Font View's Languages section with the built-in language data and the user's groups files,
//! and new documents from the built-in glyph sets (file.newFromGlyphSets).
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::settings::Reserved;
use fontcraft_engine::{EngineError, Session};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn refused(s: &mut Session, id: &str, p: Value, needle: &str) {
    match s.execute(id, &p) {
        Err(EngineError::BadParams { message, .. }) => assert!(message.contains(needle), "{id} {p}: {message}"),
        other => panic!("{id} {p}: expected a refusal naming `{needle}`, got {other:?}"),
    }
}

#[test]
fn a_language_lists_the_characters_the_font_lacks() {
    // New gives .notdef, space, A–Z and a–z; German also needs Ä Ö Ü ä ö ü ß.
    let mut s = Session::with_document();
    let r = run(&mut s, "fontView.listMissingGlyphs", json!({"entry": "languages/latin/de"}));
    let mut missing: Vec<&str> = r["missing"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    missing.sort_unstable();
    assert_eq!(missing, vec!["Adieresis", "Odieresis", "Udieresis", "adieresis", "germandbls", "odieresis", "udieresis"]);
    assert_eq!((r["present"].clone(), r["expected"].clone()), (json!(52), json!(59)));

    // The script entry knows the union of its languages.
    let latin = run(&mut s, "fontView.listMissingGlyphs", json!({"entry": "languages/latin"}));
    assert!(latin["expected"].as_u64().unwrap() > 59);
    let sidebar = run(&mut s, "fontView.queryGlyphs", json!({"sidebar": true}));
    let scripts = sidebar["sidebar"][2]["children"].as_array().unwrap();
    let latin_item = scripts.iter().find(|e| e["label"] == "Latin").unwrap();
    assert!(latin_item["expected"].as_u64().unwrap() > 0, "the script shows a badge: {latin_item}");

    run(&mut s, "fontView.generateMissingGlyphs", json!({"entry": "languages/latin/de"}));
    let done = run(&mut s, "fontView.listMissingGlyphs", json!({"entry": "languages/latin/de"}));
    assert_eq!(done["missing"], json!([]));
    assert_eq!(s.font().unwrap().glyph("germandbls").unwrap().unicodes, vec![0xDF]);
    refused(&mut s, "fontView.listMissingGlyphs", json!({"entry": "languages/latin/xx-none"}), "no language");
}

#[test]
fn groups_files_add_sidebar_entries() {
    let mut s = Session::with_document();
    let good = "{languages = ({name = \"My Set\"; list = (a, b, \"\u{e4}\");}); categories = ({name = Odd; list = (A, zz);});}";
    let problems = s.set_user_groups(&[("Groups.plist", good)]);
    assert!(problems.is_empty(), "{problems:?}");
    let r = run(&mut s, "fontView.queryGlyphs", json!({"entries": ["languages/My Set"]}));
    assert_eq!(r["glyphs"], json!(["a", "b"]));
    let r = run(&mut s, "fontView.listMissingGlyphs", json!({"entry": "languages/My Set"}));
    assert_eq!(r["missing"], json!(["adieresis"]));
    let r = run(&mut s, "fontView.listMissingGlyphs", json!({"entry": "categories/Odd"}));
    assert_eq!(r["missing"], json!(["zz"]));
}

#[test]
fn hostile_groups_files_fail_with_a_message() {
    let mut s = Session::with_document();
    let huge = format!("{{languages = ({{name = Big; list = ({});}});}}", "a, ".repeat(2 << 20));
    let deep = format!("{{languages = {}{};}}", "(".repeat(100_000), ")".repeat(100_000));
    let mut nested = String::from("{name = Leaf; list = (a);}");
    for i in 0..20 {
        nested = format!("{{name = L{i}; subGroup = ({nested});}}");
    }
    let nested = format!("{{languages = ({nested});}}");
    let problems = s.set_user_groups(&[("Groups-huge.plist", huge.as_str()), ("Groups-deep.plist", deep.as_str())]);
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems[0].contains("Groups-huge.plist") && problems[0].contains("larger"), "{problems:?}");
    assert!(problems[1].contains("Groups-deep.plist"), "{problems:?}");
    assert!(s.user_groups().is_none());

    let problems = s.set_user_groups(&[("Groups-nested.plist", nested.as_str())]);
    assert!(problems.iter().any(|p| p.contains("nested")), "{problems:?}");
    // The levels within the limit are kept.
    assert_eq!(s.user_groups().unwrap().languages.len(), 1);
    run(&mut s, "fontView.queryGlyphs", json!({"sidebar": true}));
}

#[test]
fn new_from_glyph_sets_makes_a_sorted_font_with_database_names() {
    let mut s = Session::new();
    let r = run(&mut s, "file.newFromGlyphSets", json!({"scripts": [{"script": "latin", "glyphSets": ["latin-kernel"]}], "familyName": "Sets"}));
    assert_eq!(r["sets"], json!(["latin-kernel"]));
    let font = s.font().unwrap();
    assert_eq!(font.family_name, "Sets");
    assert_eq!(font.glyphs.iter().next().unwrap().name, ".notdef", "the drawn .notdef stays, first");
    let a = font.glyph("A").unwrap();
    assert_eq!(a.unicodes, vec![0x41]);
    assert_eq!(a.layers.len(), font.masters.len());
    assert!(a.layers.iter().all(|l| l.shapes.is_empty()), "new glyphs are empty");
    assert!(font.glyph("dollar").is_some());
    assert_eq!(r["glyphs"].as_u64().unwrap() as usize, font.glyphs.len());
    // Placed by the default sort key: letters before the figures and symbols.
    let order: Vec<&str> = font.glyphs.iter().map(|g| g.name.as_str()).collect();
    let at = |n: &str| order.iter().position(|o| *o == n).unwrap();
    assert!(at("A") < at("a") && at("a") < at("zero") && at("zero") < at("dollar"), "{order:?}");

    // The picks are kept; a call without scripts repeats them.
    let stored = s.settings().reserved::<Value>(Reserved::ChooserPicks).unwrap();
    assert_eq!(stored, Some(json!([{"script": "latin", "glyphSets": ["latin-kernel"]}])));
    let again = run(&mut s, "file.newFromGlyphSets", json!({}));
    assert_eq!(again["sets"], json!(["latin-kernel"]));
    assert_eq!(s.documents().len(), 2);

    // A script alone takes its default sets, which hold more than the kernel.
    let core = run(&mut s, "file.newFromGlyphSets", json!({"scripts": [{"script": "latin"}, {"script": "cyrillic"}]}));
    assert!(core["sets"].as_array().unwrap().len() >= 2);
    assert!(s.font().unwrap().glyph("A-cy").is_some());

    // Refusals change nothing.
    let docs = s.documents().len();
    let font = s.doc().unwrap().font.clone();
    refused(&mut s, "file.newFromGlyphSets", json!({"scripts": [{"script": "klingon"}]}), "latin");
    refused(&mut s, "file.newFromGlyphSets", json!({"scripts": [{"script": "latin", "glyphSets": ["nope"]}]}), "latin-core");
    refused(&mut s, "file.newFromGlyphSets", json!({"scripts": [{"script": "latin", "glyphSets": []}]}), "no glyph set");
    refused(&mut s, "file.newFromGlyphSets", json!({"scripts": "latin"}), "scripts");
    assert_eq!(s.documents().len(), docs);
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &font));

    let mut fresh = Session::new();
    refused(&mut fresh, "file.newFromGlyphSets", json!({}), "scripts");
}

#[test]
fn glyph_sets_lists_what_new_from_glyph_sets_takes() {
    let mut s = Session::new();
    let all = run(&mut s, "glyphData.glyphSets", json!({}));
    let scripts = all["scripts"].as_array().unwrap();
    let ids: Vec<&str> = scripts.iter().map(|x| x["script"].as_str().unwrap()).collect();
    assert!(ids.contains(&"latin") && ids.contains(&"cyrillic"), "{ids:?}");
    let cyrillic = run(&mut s, "glyphData.glyphSets", json!({"script": "cyrillic"}));
    assert_eq!(cyrillic["scripts"].as_array().unwrap().len(), 1);
    let entry = &cyrillic["scripts"][0];
    assert_eq!(entry["name"], "Cyrillic");
    // Every listed set builds a font of its size plus .notdef, and a script without sets named
    // takes the ones marked default.
    let sets = entry["sets"].as_array().unwrap();
    let defaults: Vec<&str> = sets.iter().filter(|x| x["default"] == true).map(|x| x["id"].as_str().unwrap()).collect();
    assert!(!defaults.is_empty());
    let made = run(&mut s, "file.newFromGlyphSets", json!({"scripts": [{"script": "cyrillic"}]}));
    assert_eq!(made["sets"], json!(defaults));
    let first = &sets[0];
    let made = run(&mut s, "file.newFromGlyphSets", json!({"scripts": [{"script": "cyrillic", "glyphSets": [first["id"]]}]}));
    let skipped = made["skipped"].as_array().unwrap().len() as u64;
    assert_eq!(made["glyphs"].as_u64().unwrap() + skipped, first["glyphs"].as_u64().unwrap() + 1);
    refused(&mut s, "glyphData.glyphSets", json!({"script": "klingon"}), "latin");
}
