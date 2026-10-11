//! Behaviour of queries on a synthetic font: every rule and operator, groups, the predicate
//! language, search, sidebar combination, list order, folders, missing glyphs and hostile input.

use std::collections::HashSet;
use std::sync::Arc;

use fontcraft_glyphdata::GlyphData;
use fontcraft_model::{Annotation, ColorLabel, Component, Extra, Font, Layer, Path, Shape};
use serde_json::{Value, json};

use crate::*;

/// `Font::new` (`.notdef` with two paths, `space`, A–Z, a–z) plus `zero`, `A-cy` and an
/// unexported `x.ss01`, and a second master `m02`:
/// - A: two paths on m01, none on m02; tag `draft`; red label; a note.
/// - B: a component of A on m01, one path on m02.
/// - C: an annotation and a yellow layer label on m02 only.
/// - x: a stored category (`Symbol`) that differs from the glyph data's.
fn font() -> Font {
    let mut font = Font::new("Test");
    font.add_glyph("zero", vec![0x30], 600.0).unwrap();
    font.add_glyph("A-cy", vec![0x410], 600.0).unwrap();
    font.add_glyph("x.ss01", vec![], 600.0).unwrap();
    let mut m = font.masters[0].clone();
    m.id = "m02".into();
    m.name = "Bold".into();
    Arc::make_mut(&mut font.masters).push(m);
    for i in 0..font.glyphs.len() {
        let mut g = font.glyphs.get_mut_at(i).unwrap();
        g.layers.retain(|l| l.master == "m01");
        g.layers.push(Layer::new("m02", 600.0));
    }
    {
        let mut a = font.glyph_mut("A").unwrap();
        a.layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
        a.layers[0].shapes.push(Shape::Path(Path::rect(200.0, 0.0, 300.0, 100.0)));
        a.info.tags.push("draft".into());
        a.info.color = Some(ColorLabel::Index(0));
        a.info.note = "fix the apex".into();
    }
    {
        let mut b = font.glyph_mut("B").unwrap();
        b.layers[0].shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
        b.layers[1].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0)));
    }
    {
        let mut c = font.glyph_mut("C").unwrap();
        c.layers[1].info.annotations.push(Annotation {
            kind: "Text".into(),
            x: 0.0,
            y: 0.0,
            angle: 0.0,
            text: Some("check".into()),
            width: 0.0,
            extra: Extra::new(),
        });
        c.layers[1].info.color = Some(ColorLabel::Index(3));
    }
    {
        let mut x = font.glyph_mut("x").unwrap();
        x.info.category = Some("Symbol".into());
    }
    {
        let mut alt = font.glyph_mut("x.ss01").unwrap();
        alt.export = false;
    }
    font
}

fn data() -> GlyphData {
    GlyphData::new().unwrap()
}

fn select(font: &Font, data: &GlyphData, master: Option<&str>, selection: Selection) -> Vec<String> {
    Catalog::new(Env::new(font, data, master).unwrap()).select(&selection).unwrap()
}

fn with_expr(font: &Font, data: &GlyphData, master: Option<&str>, e: &Expr) -> Vec<String> {
    select(font, data, master, Selection { rules: Some(e.clone()), ..Selection::default() })
}

fn with_rules(font: &Font, data: &GlyphData, master: Option<&str>, tree: Value) -> Vec<String> {
    let tree = RuleTree::from_json(&tree).unwrap_or_else(|e| panic!("{tree}: {e}"));
    with_expr(font, data, master, &tree.compile().unwrap())
}

fn one(property: &str, op: &str, value: Value) -> Value {
    json!({"all": [{"property": property, "op": op, "value": value}]})
}

fn letters(range: std::ops::RangeInclusive<char>) -> Vec<String> {
    range.map(|c| c.to_string()).collect()
}

#[test]
fn every_rule_and_operator() {
    let (font, data) = (font(), data());
    let total = font.glyphs.len();
    let mut upper = letters('A'..='Z');
    upper.push("A-cy".into());
    let exact: [(Value, Option<&str>, Vec<String>); 28] = [
        (one("glyphName", "is", json!("A")), None, vec!["A".into()]),
        (one("glyphName", "contains", json!("cy")), None, vec!["A-cy".into()]),
        (one("glyphName", "beginsWith", json!("x.")), None, vec!["x.ss01".into()]),
        (one("glyphName", "endsWith", json!("-cy")), None, vec!["A-cy".into()]),
        (one("glyphName", "matches", json!("[A-C]")), None, letters('A'..='C')),
        (one("countOfPaths", "==", json!(2)), None, vec![".notdef".into(), "A".into()]),
        (one("countOfComponents", ">=", json!(1)), None, vec!["B".into()]),
        (one("tags", "is", json!("draft")), None, vec!["A".into()]),
        (one("tags", "contains", json!("raf")), None, vec!["A".into()]),
        (one("tags", "beginsWith", json!("dr")), None, vec!["A".into()]),
        (one("tags", "endsWith", json!("ft")), None, vec!["A".into()]),
        (one("tags", "matches", json!("d.*")), None, vec!["A".into()]),
        (one("script", "is", json!("cyrillic")), None, vec!["A-cy".into()]),
        (one("script", "endsWith", json!("lic")), None, vec!["A-cy".into()]),
        (one("category", "is", json!("Number")), None, vec!["zero".into()]),
        (one("category", "is", json!("Number/Decimal Digit")), None, vec!["zero".into()]),
        (one("category", "is", json!("Separator")), None, vec![".notdef".into(), "space".into()]),
        (one("category", "is", json!("Letter/Uppercase")), None, upper.clone()),
        (one("case", "is", json!("upper")), None, upper.clone()),
        (one("exportGlyph", "is", json!(false)), None, vec!["x.ss01".into()]),
        (one("hasUnicode", "is", json!(false)), None, vec![".notdef".into(), "x.ss01".into()]),
        (one("hasAnnotations", "is", json!(true)), Some("m02"), vec!["C".into()]),
        (one("hasAnnotations", "is", json!(true)), Some("m01"), vec![]),
        (one("colorLabel", "is", json!(0)), None, vec!["A".into()]),
        (one("colorLabel", "is", json!("Red")), None, vec!["A".into()]),
        (one("layerColorLabel", "is", json!(3)), Some("m02"), vec!["C".into()]),
        (one("layerColorLabel", "is", json!("yellow")), None, vec![]),
        (one("custom", "is", json!("name == \"A\" OR \"draft\" in tags")), None, vec!["A".into()]),
    ];
    for (tree, master, want) in exact {
        assert_eq!(with_rules(&font, &data, master, tree.clone()), want, "{tree} on {master:?}");
    }
    // The negative operators leave out exactly what their positive form keeps.
    for (tree, left_out) in [
        (one("glyphName", "isNot", json!("A")), "A"),
        (one("tags", "isNot", json!("draft")), "A"),
        (one("tags", "doesNotContain", json!("raf")), "A"),
        (one("exportGlyph", "isNot", json!(false)), "x.ss01"),
        (one("colorLabel", "is", json!("notSet")), "A"),
        (one("colorLabel", "isNot", json!(0)), "A"),
        (one("custom", "isNot", json!("name == 'A'")), "A"),
        (one("countOfComponents", "<", json!(1)), "B"),
        (one("countOfComponents", "!=", json!(1)), "B"),
    ] {
        let got = with_rules(&font, &data, None, tree.clone());
        assert_eq!(got.len(), total - 1, "{tree}");
        assert!(!got.iter().any(|n| n == left_out), "{tree}");
    }
    let no_a = with_rules(&font, &data, None, one("glyphName", "doesNotContain", json!("a")));
    assert!(!no_a.contains(&"space".to_string()) && !no_a.contains(&"a".to_string()) && no_a.contains(&"A".to_string()));
    let lower = with_rules(&font, &data, None, one("category", "is", json!("Letter/Lowercase")));
    assert!(lower.contains(&"a".to_string()) && !lower.contains(&"x".to_string()), "x stores another category");
    let not_letters = with_rules(&font, &data, None, one("category", "isNot", json!("Letter")));
    assert!(not_letters.contains(&"zero".to_string()) && not_letters.contains(&"x".to_string()) && !not_letters.contains(&"a".to_string()));
    let caseless = with_rules(&font, &data, None, one("case", "is", json!("noCase")));
    assert!(caseless.contains(&"zero".to_string()) && !caseless.contains(&"A".to_string()));
    let custom = with_rules(&font, &data, None, one("hasCustomGlyphInfo", "is", json!(true)));
    assert!(custom.contains(&"x".to_string()) && !custom.contains(&"A".to_string()));
    let fewer = with_rules(&font, &data, None, one("countOfPaths", "<=", json!(1)));
    assert!(fewer.contains(&"B".to_string()) && !fewer.contains(&"A".to_string()));
    let more = with_rules(&font, &data, None, one("countOfPaths", ">", json!(1)));
    assert_eq!(more, vec![".notdef".to_string(), "A".to_string()]);
}

#[test]
fn groups_nest() {
    let (font, data) = (font(), data());
    let tree = json!({"all": [
        {"property": "case", "op": "is", "value": "upper"},
        {"none": [
            {"property": "glyphName", "op": "is", "value": "A"},
            {"any": [
                {"property": "glyphName", "op": "is", "value": "B"},
                {"property": "glyphName", "op": "is", "value": "C"},
            ]},
        ]},
    ]});
    let mut want = letters('D'..='Z');
    want.push("A-cy".into());
    assert_eq!(with_rules(&font, &data, None, tree), want);
    // Empty groups: All lets everything through, Any nothing, None everything.
    assert_eq!(with_rules(&font, &data, None, json!({"all": []})).len(), font.glyphs.len());
    assert!(with_rules(&font, &data, None, json!({"any": []})).is_empty());
    assert_eq!(with_rules(&font, &data, None, json!({"none": []})).len(), font.glyphs.len());
}

#[test]
fn count_rules_read_the_first_master_and_layer_rules_the_shown_one() {
    let (font, data) = (font(), data());
    // On m02, A has no path and B has one; the count rule still reads m01.
    let counted = with_rules(&font, &data, Some("m02"), one("countOfPaths", ">", json!(0)));
    assert_eq!(counted, vec![".notdef".to_string(), "A".to_string()]);
    let on_shown = parse_predicate("layer.countOfPaths > 0").unwrap();
    assert_eq!(with_expr(&font, &data, Some("m02"), &on_shown), vec!["B".to_string()]);
    assert_eq!(with_expr(&font, &data, Some("m01"), &on_shown), vec![".notdef".to_string(), "A".to_string()]);
    let annotated = one("hasAnnotations", "is", json!(true));
    assert_eq!(with_rules(&font, &data, Some("m02"), annotated.clone()), vec!["C".to_string()]);
    assert!(with_rules(&font, &data, Some("m01"), annotated).is_empty());
    assert!(Env::new(&font, &data, Some("m09")).is_err());
}

#[test]
fn a_custom_predicate_equals_the_rule_tree_it_mirrors() {
    let (font, data) = (font(), data());
    let tree = json!({"any": [
        {"all": [
            {"property": "case", "op": "is", "value": "upper"},
            {"property": "glyphName", "op": "beginsWith", "value": "A"},
        ]},
        {"property": "tags", "op": "is", "value": "draft"},
        {"property": "category", "op": "is", "value": "Number"},
        {"none": [{"property": "hasUnicode", "op": "is", "value": true}]},
    ]});
    let text = "(case == upper AND name beginswith \"A\") OR \"draft\" in tags OR category == 'Number' OR NOT hasUnicode";
    let from_tree = with_rules(&font, &data, None, tree.clone());
    let from_text = with_expr(&font, &data, None, &parse_predicate(text).unwrap());
    assert_eq!(from_tree, from_text);
    assert_eq!(from_tree, vec![".notdef", "A", "zero", "A-cy", "x.ss01"]);
    // A compiled tree prints as predicate text that reads back to the same result.
    for t in [
        tree,
        one("colorLabel", "is", json!("notSet")),
        one("glyphName", "matches", json!("[a-c]")),
        one("tags", "doesNotContain", json!("raf")),
        one("category", "isNot", json!("Letter/Uppercase")),
        one("layerColorLabel", "is", json!(3)),
    ] {
        let compiled = RuleTree::from_json(&t).unwrap().compile().unwrap();
        let text = compiled.to_string();
        let reread = parse_predicate(&text).unwrap_or_else(|e| panic!("{text}: {e}"));
        for master in [None, Some("m02")] {
            assert_eq!(with_expr(&font, &data, master, &compiled), with_expr(&font, &data, master, &reread), "{text}");
        }
    }
}

#[test]
fn hostile_predicates_rules_and_patterns_fail_with_a_message() {
    let deep = format!("{}name == 'A'{}", "(".repeat(1000), ")".repeat(1000));
    assert!(parse_predicate(&deep).unwrap_err().0.contains("nested"));
    let nots = format!("{}hasUnicode", "NOT ".repeat(1000));
    assert!(parse_predicate(&nots).unwrap_err().0.contains("nested"));
    assert!(parse_predicate(&"name == 'A' OR ".repeat(2000)).is_err());
    for (text, needle) in [
        ("", "empty"),
        ("name ==", "value"),
        ("name == 'A", "quote"),
        ("weight > 3", "unknown attribute"),
        ("hasHints == true", "not supported"),
        ("countOfPaths contains 'x'", "number"),
        ("case == huge", "one of"),
        ("name == 'A')", "after the end"),
        ("3 in name", "list attribute"),
    ] {
        let e = parse_predicate(text).unwrap_err();
        assert!(e.0.contains(needle), "{text}: {e}");
    }
    let huge = "(?:a{1000}){1000}";
    assert!(Pattern::new(huge, false).is_err());
    assert!(Pattern::new(&"a".repeat(MAX_PATTERN_BYTES + 1), false).is_err());
    assert!(Matcher::new(&Search { query: huge.into(), regex: true, ..Search::default() }).is_err());
    assert!(RuleTree::from_json(&one("glyphName", "matches", json!(huge))).is_err());
    assert!(RuleTree::from_json(&one("custom", "is", json!(deep))).is_err());
    let mut nested = json!({"property": "hasUnicode", "op": "is", "value": true});
    for _ in 0..100 {
        nested = json!({"all": [nested]});
    }
    assert!(RuleTree::from_json(&nested).unwrap_err().0.contains("nested"));
    let wide = json!({"any": vec![json!({"property": "hasUnicode"}); MAX_RULES + 1]});
    assert!(RuleTree::from_json(&wide).is_err());
    for (tree, needle) in [
        (json!([]), "one key"),
        (json!({"all": [], "any": []}), "one key"),
        (json!({"all": [{"property": "weight"}]}), "unknown rule property"),
        (json!({"all": [{"property": "glyphName", "op": ">", "value": "A"}]}), "operators"),
        (json!({"all": [{"property": "countOfPaths", "op": "==", "value": "two"}]}), "number"),
        (json!({"all": [{"property": "category", "value": "Letters"}]}), "category"),
        (json!({"all": [{"property": "colorLabel", "value": 12}]}), "0 to 11"),
        (json!({"all": [{"property": "glyphName", "op": "is", "value": "A", "extra": 1}]}), "keys"),
    ] {
        let e = RuleTree::from_json(&tree).unwrap_err();
        assert!(e.0.contains(needle), "{tree}: {e}");
    }
}

#[test]
fn search_matches_names_code_points_characters_and_notes() {
    let (font, data) = (font(), data());
    let find = |query: &str, fields: &[SearchField], match_case: bool, regex: bool| {
        let m = Matcher::new(&Search { query: query.into(), fields: fields.to_vec(), match_case, regex }).unwrap();
        select(&font, &data, None, Selection { search: Some(m), ..Selection::default() })
    };
    use SearchField::{Name, Note, Unicode};
    assert_eq!(find("a-c", &[Name], false, false), vec!["A-cy"]);
    assert!(find("a-c", &[Name], true, false).is_empty());
    assert_eq!(find("0041", &[Unicode], false, false), vec!["A"]);
    assert_eq!(find("U+0410", &[], false, false), vec!["A-cy"]);
    assert_eq!(find("A", &[Unicode], false, false), vec!["A"]);
    assert_eq!(find("APEX", &[Note], false, false), vec!["A"]);
    assert!(find("apex", &[Name], false, false).is_empty());
    let one_char = find(".", &[Name], true, true);
    assert_eq!(one_char.len(), 52);
    assert_eq!(find("[Aa].*-CY", &[Name], false, true), vec!["A-cy"]);
    assert!(find("[Aa].*-CY", &[Name], true, true).is_empty());
    assert_eq!(find("04.0", &[Unicode], false, true), vec!["A-cy"]);
    assert_eq!(find("", &[], false, true).len(), font.glyphs.len());
}

fn filters() -> Vec<Filter> {
    let mut items = Vec::new();
    add_filter(&mut items, &[], Filter::List { name: "List".into(), glyphs: list_names(["C B", "A\nzz"]).unwrap() }).unwrap();
    add_filter(&mut items, &[], Filter::Smart { name: "Upper".into(), rules: RuleTree::from_json(&one("case", "is", json!("upper"))).unwrap() })
        .unwrap();
    add_filter(&mut items, &[], Filter::Folder { name: "Folder".into(), children: Vec::new() }).unwrap();
    let folder = vec!["Folder".to_string()];
    add_filter(&mut items, &folder, Filter::List { name: "Small a".into(), glyphs: vec!["a".into()] }).unwrap();
    add_filter(&mut items, &folder, Filter::Smart { name: "B".into(), rules: RuleTree::from_json(&one("glyphName", "is", json!("B"))).unwrap() })
        .unwrap();
    items
}

fn path(text: &str) -> Vec<String> {
    parse_path(&json!(text)).unwrap()
}

#[test]
fn sidebar_entries_combine_by_section() {
    let (font, data) = (font(), data());
    let items = filters();
    let catalog = Catalog::new(Env::new(&font, &data, None).unwrap()).with_filters(&items);
    let sel = |entries: &[&str]| catalog.select(&Selection { entries: entries.iter().copied().map(path).collect(), ..Selection::default() }).unwrap();
    // Union within Categories, intersection across sections.
    assert_eq!(sel(&["categories/Number", "categories/Separator"]), vec![".notdef", "space", "zero"]);
    assert_eq!(sel(&["categories/Letter", "languages/cyrillic"]), vec!["A-cy"]);
    assert_eq!(sel(&["languages/latin", "languages/cyrillic", "categories/Letter/Uppercase"]).len(), 27);
    // A list filter keeps its order and leaves out names the font lacks; filters intersect.
    assert_eq!(sel(&["filters/List"]), vec!["C", "B", "A"]);
    assert_eq!(sel(&["filters/Upper", "filters/List"]), vec!["C", "B", "A"]);
    assert_eq!(sel(&["filters/List", "filters/Folder/Small a"]), Vec::<String>::new());
    // A folder is the union of its contents, in glyph order.
    assert_eq!(sel(&["filters/Folder"]), vec!["B", "a"]);
    assert_eq!(sel(&["all"]).len(), font.glyphs.len());
    // Rules and search narrow the sidebar's result.
    let m = Matcher::new(&Search { query: "A".into(), fields: vec![SearchField::Name], match_case: true, regex: false }).unwrap();
    let got = catalog.select(&Selection { entries: vec![path("categories/Letter")], search: Some(m), ..Selection::default() }).unwrap();
    assert_eq!(got, vec!["A", "A-cy"]);
    for bad in ["nowhere", "categories/Letter/Tall", "filters/None", "languages/LATIN"] {
        assert!(catalog.resolve(&path(bad)).is_err(), "{bad}");
    }
}

#[test]
fn entries_with_a_known_set_list_their_missing_glyphs() {
    let (font, data) = (font(), data());
    let items = filters();
    let languages = vec![LanguageSet { id: "xx".into(), name: "Test".into(), script: "latin".into(), characters: vec![0x41, 0x42, 0xC4] }];
    let catalog = Catalog::new(Env::new(&font, &data, None).unwrap()).with_filters(&items).with_languages(&languages).unwrap();
    let missing = |p: &str| catalog.missing(&catalog.resolve(&path(p)).unwrap()).map(|v| v.into_iter().map(|e| e.name).collect::<Vec<_>>());
    assert_eq!(missing("filters/List"), Some(vec!["zz".to_string()]));
    assert_eq!(missing("languages/latin/xx"), Some(vec!["Adieresis".to_string()]));
    assert_eq!(missing("languages/latin"), Some(vec!["Adieresis".to_string()]));
    assert_eq!(missing("categories/Letter/Uppercase"), Some(vec!["Adieresis".to_string()]));
    assert_eq!(missing("languages/cyrillic"), None);
    assert_eq!(missing("filters/Upper"), None);
    let entry = catalog.resolve(&path("languages/latin/xx")).unwrap();
    assert_eq!(catalog.members(&entry), vec!["A", "B"]);

    let sidebar = catalog.sidebar();
    let find = |items: &[SidebarItem], p: &str| -> Option<SidebarItem> {
        fn walk(items: &[SidebarItem], want: &[String]) -> Option<SidebarItem> {
            for i in items {
                if i.path == want {
                    return Some(i.clone());
                }
                if let Some(found) = walk(&i.children, want) {
                    return Some(found);
                }
            }
            None
        }
        walk(items, &path(p))
    };
    let upper = find(&sidebar, "categories/Letter/Uppercase").unwrap();
    assert_eq!((upper.count, upper.expected, upper.present), (27, Some(3), Some(2)));
    let list = find(&sidebar, "filters/List").unwrap();
    assert_eq!((list.count, list.expected, list.present), (3, Some(4), Some(3)));
    assert_eq!(find(&sidebar, "filters/Folder").unwrap().count, 2);
    assert_eq!(find(&sidebar, "languages/latin/xx").unwrap().label, "Test");
    assert_eq!(find(&sidebar, "categories/Number/Decimal Digit").unwrap().count, 1);
    assert!(find(&sidebar, "languages/greek").is_some(), "a default script is listed without glyphs");

    // A shared index gives the same entries; one built from other glyph data lends only its sets.
    let shared = Arc::new(languages.clone());
    let json = |items: Vec<SidebarItem>| serde_json::to_value(items).unwrap();
    let names = |c: &Catalog, p: &str| c.missing(&c.resolve(&path(p)).unwrap()).map(|v| v.into_iter().map(|e| e.name).collect::<Vec<_>>());
    let own_data = Arc::new(super::tests::data());
    let own = LanguageIndex::new(shared.clone(), own_data.clone()).unwrap();
    assert!(own.is_for(&shared, &own_data));
    let with_own = Catalog::new(Env::new(&font, &own_data, None).unwrap()).with_filters(&items).with_language_index(&own);
    assert_eq!(json(with_own.sidebar()), json(sidebar.clone()));
    let renamed = GlyphData::with_custom_files(&[br#"<glyphData><glyph name="Aumlaut" unicode="00C4" category="Letter"/></glyphData>"#]).unwrap();
    let other = LanguageIndex::new(shared.clone(), Arc::new(renamed)).unwrap();
    assert!(!other.is_for(&shared, &data));
    let with_other = Catalog::new(Env::new(&font, &data, None).unwrap()).with_filters(&items).with_language_index(&other);
    assert_eq!(names(&with_other, "languages/latin/xx"), Some(vec!["Adieresis".to_string()]));
    assert_eq!(json(with_other.sidebar()), json(sidebar));
}

#[test]
fn groups_files_add_entries() {
    let (font, data) = (font(), data());
    let text = r#"{
categories = (
{ name = Mine; list = (B, A, "Ä"); },
{ name = Pred; predicate = "name beginswith 'x'"; subGroup = ( { name = Inner; list = (a); } ); },
{ name = Bad; predicate = "weight == 1"; },
{ list = (A); }
);
languages = ( { name = Cyr; script = cyrillic; } );
}"#;
    let (groups, problems) = parse_groups(text, &data).unwrap();
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert_eq!(groups.categories.len(), 2);
    let catalog = Catalog::new(Env::new(&font, &data, None).unwrap()).with_groups(&groups);
    let entry = catalog.resolve(&path("categories/Mine")).unwrap();
    assert_eq!(catalog.members(&entry), vec!["A", "B"]);
    assert_eq!(catalog.missing(&entry).unwrap().into_iter().map(|e| e.name).collect::<Vec<_>>(), vec!["Adieresis"]);
    assert_eq!(catalog.members(&catalog.resolve(&path("categories/Pred")).unwrap()), vec!["x", "x.ss01"]);
    assert_eq!(catalog.members(&catalog.resolve(&path("categories/Pred/Inner")).unwrap()), vec!["a"]);
    assert_eq!(catalog.members(&catalog.resolve(&path("languages/Cyr")).unwrap()), vec!["A-cy"]);
    assert!(parse_groups("{ categories = (", &data).is_err());
    assert!(parse_groups(&" ".repeat(MAX_GROUPS_BYTES + 1), &data).is_err());
}

#[test]
fn the_filter_tree_is_edited_by_path() {
    let mut items = filters();
    assert!(add_filter(&mut items, &[], Filter::Folder { name: "List".into(), children: vec![] }).is_err(), "names are unique");
    assert!(add_filter(&mut items, &path("List"), Filter::Folder { name: "X".into(), children: vec![] }).is_err(), "a list is no folder");
    assert_eq!(rename_filter(&mut items, &path("Folder/B"), " Bee ").unwrap(), path("Folder/Bee"));
    assert!(rename_filter(&mut items, &path("Folder/Bee"), "Small a").is_err());
    assert_eq!(move_filter(&mut items, &path("List"), &path("Folder"), 0).unwrap(), path("Folder/List"));
    assert_eq!(filter_at(&items, &path("Folder/List")).unwrap().kind(), "list");
    assert!(move_filter(&mut items, &path("Folder"), &path("Folder"), 0).is_err(), "not into itself");
    assert!(move_filter(&mut items, &path("Upper"), &[], 5).is_err(), "index out of range");
    let before = items.clone();
    assert!(move_filter(&mut items, &path("Upper"), &path("Nowhere"), 0).is_err());
    assert_eq!(items, before, "a refused move changes nothing");
    assert_eq!(move_filter(&mut items, &path("Upper"), &[], 1).unwrap(), path("Upper"));
    assert_eq!(items.iter().map(Filter::name).collect::<Vec<_>>(), vec!["Folder", "Upper"]);
    let removed = remove_filter(&mut items, &path("Folder")).unwrap();
    assert_eq!(removed.kind(), "folder");
    assert_eq!(items.len(), 1, "a folder takes its contents with it");
    // Folders nest at most MAX_FOLDER_DEPTH deep.
    let mut deep = Vec::new();
    let mut parent: Vec<String> = Vec::new();
    for i in 0..MAX_FOLDER_DEPTH {
        parent = add_filter(&mut deep, &parent, Filter::Folder { name: format!("f{i}"), children: vec![] }).unwrap();
    }
    assert!(add_filter(&mut deep, &parent, Filter::List { name: "too deep".into(), glyphs: vec![] }).is_err());
    assert!(list_names(["a\u{1}"]).is_err());
    assert_eq!(list_names(["a b", "b c"]).unwrap(), vec!["a", "b", "c"]);
    // The tree survives the JSON round trip the settings store makes.
    let items = filters();
    let back: Vec<Filter> = serde_json::from_value(serde_json::to_value(&items).unwrap()).unwrap();
    assert_eq!(back, items);
}

#[test]
fn spacing_rules() {
    let (mut font, data) = (font(), data());
    font.glyph_mut("A").unwrap().info.metrics_keys.left = Some("=H".into());
    font.glyph_mut("B").unwrap().layers[1].info.metrics_keys.width = Some("=600".into());
    font.glyph_mut("C").unwrap().kern_groups.right = Some("C".into());
    font.glyph_mut("D").unwrap().kern_groups.top = Some("D".into());
    let keys = with_rules(&font, &data, None, one("hasMetricsKeys", "is", json!(true)));
    assert_eq!(keys, vec!["A".to_string(), "B".to_string()], "a key on any layer counts");
    let groups = with_rules(&font, &data, None, one("hasKerningGroups", "is", json!(true)));
    assert_eq!(groups, vec!["C".to_string(), "D".to_string()], "vertical groups count");
    let ungrouped = with_rules(&font, &data, None, one("hasKerningGroups", "isNot", json!(true)));
    assert_eq!(ungrouped.len(), font.glyphs.len() - 2);

    let tree = RuleTree::from_json(&one("metricsKeysOutOfSync", "is", json!(true))).unwrap().compile().unwrap();
    let selection = Selection { rules: Some(tree), ..Selection::default() };
    let without_set = Catalog::new(Env::new(&font, &data, None).unwrap()).select(&selection).unwrap();
    assert!(without_set.is_empty(), "no set given: nothing is out of sync");
    let stale: HashSet<String> = ["B".to_string(), "missing".to_string()].into_iter().collect();
    let env = Env::new(&font, &data, None).unwrap().with_out_of_sync(&stale);
    assert_eq!(Catalog::new(env).select(&selection).unwrap(), vec!["B".to_string()]);

    let custom = with_expr(&font, &data, None, &parse_predicate("hasMetricsKeys AND NOT hasKerningGroups").unwrap());
    assert_eq!(custom, vec!["A".to_string(), "B".to_string()]);
}
