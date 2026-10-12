//! Feature entries (Font Info › Features) and number values: the font.features.* entry commands,
//! Update, the stylistic-set names, the registry queries, font.numbers.* and font.master.setNumber.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::{EngineError, Session};
use fontcraft_model::{Master, Snippet};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Runs a command that must be refused with a message holding `needle`, and checks that the font
/// is the very same pointer afterwards.
fn refused(s: &mut Session, id: &str, p: Value, needle: &str) {
    let before = Arc::clone(&s.doc().unwrap().font);
    match s.execute(id, &p) {
        Err(EngineError::BadParams { message, .. }) => assert!(message.contains(needle), "{id} {p}: {message}"),
        other => panic!("{id} {p}: expected a refusal naming `{needle}`, got {other:?}"),
    }
    assert!(Arc::ptr_eq(&before, &s.doc().unwrap().font), "{id} {p}: a refused command changed the font");
}

fn names(list: &Value) -> Vec<String> {
    list["entries"].as_array().unwrap().iter().map(|e| format!("{}:{}", e["kind"].as_str().unwrap(), e["name"].as_str().unwrap())).collect()
}

/// Marks an entry automatic directly, as a file written by the reference app would hold it.
fn make_automatic(s: &mut Session, tag: &str) {
    s.edit("automatic", |font| {
        let f = Arc::make_mut(&mut font.features);
        f.features.iter_mut().filter(|e| e.name == tag).for_each(|e| e.automatic = true);
        Ok(())
    })
    .unwrap();
}

#[test]
fn feature_entries_are_added_listed_in_sidebar_groups_and_read_back() {
    let mut s = Session::with_document();
    let r = run(&mut s, "font.features.addFeature", json!({"tag": "liga", "code": "sub f i by f_i;"}));
    assert_eq!(r, json!({"kind": "feature", "name": "liga", "index": 0}));
    let r = run(&mut s, "font.features.addClass", json!({"name": "@Upper", "glyphs": ["A", "B", "C"]}));
    assert_eq!(r["name"], "Upper", "the @ is not part of the stored name");
    assert_eq!(run(&mut s, "font.features.addPrefix", json!({}))["name"], "Prefix");
    assert_eq!(run(&mut s, "font.features.addPrefix", json!({"code": "lookup L { sub a by b; } L;"}))["name"], "Prefix 2");

    // fontInfo.features.sidebarGroups: Prefix, Classes, Features, whatever the order of adding.
    let list = run(&mut s, "font.features.list", json!({}));
    assert_eq!(names(&list), ["prefix:Prefix", "prefix:Prefix 2", "class:Upper", "feature:liga"]);
    assert_eq!(names(&run(&mut s, "font.features.list", json!({"kind": "class"}))), ["class:Upper"]);

    let class = run(&mut s, "font.features.get", json!({"kind": "class", "name": "Upper"}));
    assert_eq!(class["code"], "A B C");
    assert_eq!(class["automatic"], false);
    let liga = run(&mut s, "font.features.get", json!({"kind": "feature", "name": "liga", "index": 0}));
    assert_eq!(liga["code"], "sub f i by f_i;");
    assert!(liga.get("labels").is_none(), "only stylistic sets have names");

    // fontInfo.features.automaticIndicator: the listed flag the update button follows.
    assert!(list["entries"].as_array().unwrap().iter().all(|e| e["automatic"] == false && e["errors"] == false));
    make_automatic(&mut s, "liga");
    let list = run(&mut s, "font.features.list", json!({"kind": "feature"}));
    assert_eq!(list["entries"][0]["automatic"], true);
}

#[test]
fn feature_entry_commands_refuse_bad_requests_and_change_nothing() {
    let mut s = Session::with_document();
    run(&mut s, "font.features.addClass", json!({"name": "Upper", "code": "A B"}));
    run(&mut s, "font.features.addFeature", json!({"tag": "calt", "code": "sub a' b by c;"}));
    run(&mut s, "font.features.addFeature", json!({"tag": "liga", "code": ""}));

    // A taken name, a tag that is not four characters, a bad class name.
    refused(&mut s, "font.features.addClass", json!({"name": "Upper"}), "already has a class `Upper`");
    refused(&mut s, "font.features.addFeature", json!({"tag": "liga"}), "already has a feature `liga`");
    refused(&mut s, "font.features.addFeature", json!({"tag": "lig"}), "four letters");
    refused(&mut s, "font.features.addFeature", json!({"tag": "ligatures"}), "four letters");
    refused(&mut s, "font.features.addClass", json!({"name": "1st"}), "class name");
    refused(&mut s, "font.features.addClass", json!({"name": "X", "code": "A", "glyphs": ["A"]}), "not both");
    refused(&mut s, "font.features.addClass", json!({"name": "X", "glyphs": ["nosuchglyph"]}), "nosuchglyph");
    refused(&mut s, "font.features.rename", json!({"kind": "feature", "name": "calt", "newName": "liga"}), "already has a feature `liga`");

    // An unknown entry, and an index that holds another entry.
    refused(&mut s, "font.features.get", json!({"kind": "feature", "name": "smcp"}), "no feature `smcp`");
    refused(&mut s, "font.features.get", json!({"kind": "feature", "name": "liga", "index": 0}), "is `calt`, not `liga`");
    refused(&mut s, "font.features.setNotes", json!({"kind": "prefix", "name": "Prefix", "notes": "x"}), "no prefix `Prefix`");
    refused(&mut s, "font.features.remove", json!({"entries": [{"kind": "feature", "name": "smcp"}]}), "no feature `smcp`");
    refused(&mut s, "font.features.get", json!({"kind": "lookup", "name": "x"}), "\"prefix\", \"class\" or \"feature\"");

    // Code on an automatic entry; automatic code where none exists; Update Entry on manual code.
    make_automatic(&mut s, "liga");
    refused(&mut s, "font.features.setCode", json!({"kind": "feature", "name": "liga", "code": "sub a by b;"}), "Generate Feature Automatically");
    refused(&mut s, "font.features.setAutomatic", json!({"kind": "feature", "name": "calt", "automatic": true}), "written by hand");
    refused(&mut s, "font.features.addFeature", json!({"tag": "rand", "automatic": true}), "no automatic code");
    refused(&mut s, "font.features.updateEntry", json!({"kind": "feature", "name": "calt"}), "written by hand");

    // Reordering must list each entry once.
    refused(&mut s, "font.features.reorder", json!({"kind": "feature", "order": ["calt"]}), "each must be listed once");
    refused(&mut s, "font.features.reorder", json!({"kind": "feature", "order": ["calt", "calt"]}), "no feature `calt` left");
}

#[test]
fn feature_entries_are_renamed_reordered_and_removed() {
    let mut s = Session::with_document();
    for tag in ["liga", "smcp", "kern"] {
        run(&mut s, "font.features.addFeature", json!({"tag": tag}));
    }
    run(&mut s, "font.features.addClass", json!({"name": "Lower", "code": "a b"}));

    let r = run(&mut s, "font.features.rename", json!({"kind": "feature", "name": "smcp", "newName": "c2sc"}));
    assert_eq!(r, json!({"kind": "feature", "name": "c2sc", "index": 1}));
    run(&mut s, "font.features.rename", json!({"kind": "class", "name": "@Lower", "newName": "@lc"}));
    assert_eq!(s.font().unwrap().features.classes[0].name, "lc");

    let r = run(&mut s, "font.features.reorder", json!({"kind": "feature", "order": ["kern", "liga", "c2sc"]}));
    assert_eq!(r["order"], json!(["kern", "liga", "c2sc"]));
    let tags: Vec<&str> = s.font().unwrap().features.features.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(tags, ["kern", "liga", "c2sc"]);

    // A tag stored twice (a file may hold that) is placed by index, and named by index too.
    s.edit("duplicate", |font| {
        Arc::make_mut(&mut font.features).features.push(Snippet { name: "kern".into(), code: "pos a b -5;".into(), ..Snippet::default() });
        Ok(())
    })
    .unwrap();
    run(&mut s, "font.features.reorder", json!({"kind": "feature", "order": [3, "liga", "c2sc", 0]}));
    let f = &s.font().unwrap().features.features;
    assert_eq!((f[0].name.as_str(), f[0].code.as_str()), ("kern", "pos a b -5;"));
    assert_eq!(f[3].name, "kern");

    let r = run(
        &mut s,
        "font.features.remove",
        json!({"entries": [{"kind": "feature", "name": "kern", "index": 3}, {"kind": "class", "name": "lc"}, {"kind": "feature", "name": "liga"}]}),
    );
    assert_eq!(r["removed"].as_array().unwrap().len(), 3);
    let f = &s.font().unwrap().features;
    assert!(f.classes.is_empty());
    let tags: Vec<&str> = f.features.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(tags, ["kern", "c2sc"]);
    assert_eq!(f.features[0].code, "pos a b -5;", "the stored kern that was named by index stays");
}

#[test]
fn feature_code_notes_and_stylistic_set_names_are_set() {
    let mut s = Session::with_document();
    run(&mut s, "font.features.addFeature", json!({"tag": "ss01"}));
    run(&mut s, "font.features.addFeature", json!({"tag": "liga"}));

    run(&mut s, "font.features.setCode", json!({"kind": "feature", "name": "ss01", "code": "sub a by a.ss01;"}));
    let r = run(&mut s, "font.features.setNotes", json!({"kind": "feature", "name": "ss01", "notes": "single-storey a"}));
    assert_eq!(r["notes"], "single-storey a");

    let r = run(&mut s, "font.features.setLabel", json!({"tag": "ss01", "name": "Alternate a"}));
    assert_eq!(r["labels"], json!([{"language": "dflt", "name": "Alternate a"}]));
    run(&mut s, "font.features.setLabel", json!({"tag": "ss01", "language": "DEU", "name": "Alternatives a"}));
    let r = run(&mut s, "font.features.setLabel", json!({"tag": "ss01", "name": "Single-storey a"}));
    assert_eq!(r["labels"], json!([{"language": "dflt", "name": "Single-storey a"}, {"language": "DEU", "name": "Alternatives a"}]));

    let got = run(&mut s, "font.features.get", json!({"kind": "feature", "name": "ss01"}));
    assert_eq!(got["code"], "sub a by a.ss01;");
    assert_eq!(got["notes"], "single-storey a");
    assert_eq!(got["labels"].as_array().unwrap().len(), 2);

    let r = run(&mut s, "font.features.removeLabel", json!({"tag": "ss01", "language": "DEU"}));
    assert_eq!(r["labels"], json!([{"language": "dflt", "name": "Single-storey a"}]));

    refused(&mut s, "font.features.setLabel", json!({"tag": "liga", "name": "Ligatures"}), "only stylistic sets");
    refused(&mut s, "font.features.setLabel", json!({"tag": "ss02", "name": "Two"}), "no feature `ss02`");
    refused(&mut s, "font.features.setLabel", json!({"tag": "ss01", "language": "Klingon", "name": "x"}), "language tag");
    refused(&mut s, "font.features.setLabel", json!({"tag": "ss01", "name": "  "}), "empty");
    refused(&mut s, "font.features.removeLabel", json!({"tag": "ss01", "language": "DEU"}), "no name in \"DEU\"");
}

#[test]
fn feature_registry_queries_find_tags_and_their_specification() {
    let mut s = Session::with_document();
    let found = run(&mut s, "font.features.listAvailable", json!({"query": "small cap"}));
    let tags: Vec<&str> = found["features"].as_array().unwrap().iter().map(|f| f["tag"].as_str().unwrap()).collect();
    assert_eq!(tags, ["c2sc", "smcp"]);
    assert_eq!(found["features"][1]["inFont"], false);
    run(&mut s, "font.features.addFeature", json!({"tag": "smcp"}));
    let found = run(&mut s, "font.features.listAvailable", json!({"query": "SMCP"}));
    assert_eq!(found["features"][0]["inFont"], true);
    assert_eq!(found["features"][0]["name"], "Small Capitals");
    assert_eq!(run(&mut s, "font.features.listAvailable", json!({}))["features"].as_array().unwrap().len(), 243);

    let spec = run(&mut s, "font.features.openSpec", json!({"tag": "smcp"}));
    assert_eq!(spec["url"], "https://learn.microsoft.com/typography/opentype/spec/features_pt#tag-smcp");
    refused(&mut s, "font.features.openSpec", json!({"tag": "abcd"}), "not a registered feature tag");
}

/// Needs the automatic generators (the suffix features): `a.sc` calls for `smcp`.
#[test]
fn feature_update_writes_automatic_entries_and_leaves_manual_ones() {
    let mut s = Session::with_document();
    s.edit("small caps", |font| {
        font.add_glyph("a.sc", vec![], 600.0)?;
        font.add_glyph("b.sc", vec![], 600.0)?;
        Ok(())
    })
    .unwrap();
    run(&mut s, "font.features.addFeature", json!({"tag": "calt", "code": "sub a' b by b;"}));
    let r = run(&mut s, "font.features.update", json!({}));
    assert!(r["added"].as_array().unwrap().contains(&json!({"kind": "feature", "name": "smcp"})), "{r}");
    let smcp = run(&mut s, "font.features.get", json!({"kind": "feature", "name": "smcp"}));
    assert_eq!(smcp["automatic"], true);
    assert!(smcp["code"].as_str().unwrap().contains("a.sc"), "{smcp}");
    let calt = run(&mut s, "font.features.get", json!({"kind": "feature", "name": "calt"}));
    assert_eq!(calt["code"], "sub a' b by b;", "manual code is left alone");

    // Update Entry regenerates the stored code.
    run(&mut s, "font.features.setAutomatic", json!({"kind": "feature", "name": "smcp", "automatic": false}));
    run(&mut s, "font.features.setCode", json!({"kind": "feature", "name": "smcp", "code": "# by hand"}));
    let r = run(&mut s, "font.features.setAutomatic", json!({"kind": "feature", "name": "smcp", "automatic": true}));
    assert!(r["code"].as_str().unwrap().contains("a.sc"), "turning automatic on regenerates: {r}");
    let r = run(&mut s, "font.features.updateEntry", json!({"kind": "feature", "name": "smcp"}));
    assert!(r["code"].as_str().unwrap().contains("b.sc"), "{r}");
    let listed = run(&mut s, "font.features.listAvailable", json!({"query": "smcp"}));
    assert_eq!(listed["features"][0]["canGenerate"], true);
}

/// Needs the assembly (one file per entry, classes before prefixes before features).
#[test]
fn feature_expand_code_assembles_one_master() {
    let mut s = Session::with_document();
    run(&mut s, "font.features.addFeature", json!({"tag": "liga", "code": "sub f i by f_i;"}));
    run(&mut s, "font.features.addClass", json!({"name": "Upper", "code": "A B"}));
    let r = run(&mut s, "font.features.expandCode", json!({"target": {"master": "Regular"}}));
    assert_eq!(r["target"], json!({"master": "m01"}));
    let code = r["code"].as_str().unwrap();
    let class = code.find("@Upper = [").expect("the class");
    let feature = code.find("feature liga {").expect("the feature");
    assert!(class < feature, "classes come before features:\n{code}");
    assert!(code.contains("sub f i by f_i;"));
    match s.execute("font.features.expandCode", &json!({"target": {"instance": 0}})) {
        Err(EngineError::Unsupported(_)) => {}
        other => panic!("an instance target is not available yet, got {other:?}"),
    }
    refused(&mut s, "font.features.expandCode", json!({"target": {"master": "Bold"}}), "no master `Bold`");
}

#[test]
fn feature_and_number_commands_undo_to_the_very_font() {
    let mut s = Session::with_document();
    let steps: Vec<(&str, Value)> = vec![
        ("font.features.addPrefix", json!({"name": "Shared"})),
        ("font.features.addClass", json!({"name": "Upper", "glyphs": ["A"]})),
        ("font.features.addFeature", json!({"tag": "ss01", "code": "sub a by b;"})),
        ("font.features.addFeature", json!({"tag": "liga"})),
        ("font.features.rename", json!({"kind": "prefix", "name": "Shared", "newName": "Lookups"})),
        ("font.features.reorder", json!({"kind": "feature", "order": ["liga", "ss01"]})),
        ("font.features.setCode", json!({"kind": "feature", "name": "liga", "code": "sub f l by f_l;"})),
        ("font.features.setNotes", json!({"kind": "class", "name": "Upper", "notes": "capitals"})),
        ("font.features.setLabel", json!({"tag": "ss01", "name": "Alternates"})),
        ("font.features.removeLabel", json!({"tag": "ss01", "language": "dflt"})),
        ("font.features.remove", json!({"entries": [{"kind": "feature", "name": "liga"}]})),
        ("font.numbers.add", json!({"name": "padding", "values": {"m01": 20}})),
        ("font.master.setNumber", json!({"masters": ["m01"], "number": "padding", "value": 30})),
        ("font.numbers.rename", json!({"number": 0, "name": "gap"})),
        ("font.numbers.remove", json!({"number": "gap"})),
    ];
    for (id, p) in steps {
        let before = Arc::clone(&s.doc().unwrap().font);
        run(&mut s, id, p.clone());
        assert!(!Arc::ptr_eq(&before, &s.doc().unwrap().font), "{id} {p} changed nothing");
        run(&mut s, "edit.undo", json!({}));
        assert!(Arc::ptr_eq(&before, &s.doc().unwrap().font), "{id}: undo did not give back the very font");
        run(&mut s, "edit.redo", json!({}));
    }
}

#[test]
fn number_values_are_added_set_per_master_renamed_and_removed() {
    let mut s = Session::with_document();
    s.edit("second master", |font| {
        Arc::make_mut(&mut font.masters).push(Master { id: "m02".into(), name: "Bold".into(), ..Master::default() });
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "font.numbers.add", json!({"name": "tracking", "values": {"Bold": 12}}));
    assert_eq!(r, json!({"index": 0, "name": "tracking", "values": {"m01": 0.0, "m02": 12.0}}));
    run(&mut s, "font.numbers.add", json!({"name": ".monospace"}));

    let r = run(&mut s, "font.master.setNumber", json!({"masters": [0, "m02"], "number": "tracking", "value": 25}));
    assert_eq!(r["masters"], json!(["m01", "m02"]));
    run(&mut s, "font.master.setNumber", json!({"masters": "Bold", "number": 1, "value": -4.5}));

    let list = run(&mut s, "font.numbers.list", json!({"master": "m02"}));
    assert_eq!(list["numbers"][0], json!({"index": 0, "name": "tracking", "values": {"m01": 25.0, "m02": 25.0}, "value": 25.0}));
    assert_eq!(list["numbers"][1]["value"], -4.5);
    assert_eq!(s.font().unwrap().masters[0].number_values, [25.0, 0.0]);

    run(&mut s, "font.numbers.rename", json!({"number": "tracking", "name": "space"}));
    let r = run(&mut s, "font.numbers.remove", json!({"number": "space"}));
    assert_eq!(r, json!({"index": 0, "name": "space"}));
    let font = s.font().unwrap();
    assert_eq!(font.numbers.len(), 1);
    assert_eq!(font.masters[1].number_values, [-4.5], "each master's value goes with its number");

    refused(&mut s, "font.numbers.add", json!({"name": ".monospace"}), "already has a number value");
    refused(&mut s, "font.numbers.add", json!({"name": "2x"}), "number value name");
    // A `$name` token stops at a dot, so it could not read this name.
    refused(&mut s, "font.numbers.add", json!({"name": "pad.left"}), "number value name");
    refused(&mut s, "font.numbers.add", json!({"name": "..x"}), "number value name");
    refused(&mut s, "font.numbers.add", json!({"name": "pad", "values": {"Black": 1}}), "no master `Black`");
    refused(&mut s, "font.numbers.rename", json!({"number": "nothing", "name": "x"}), "no number value `nothing`");
    refused(&mut s, "font.master.setNumber", json!({"masters": ["m01"], "number": 0, "value": 2e6}), "within");
    refused(&mut s, "font.master.setNumber", json!({"masters": [7], "number": 0, "value": 1}), "no master 7");
    refused(&mut s, "font.numbers.remove", json!({"number": 5}), "no number value 5");
}
