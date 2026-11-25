//! Font Info fields at the font, master and instance scopes, the row clipboard, the version
//! stepper, the creation date, Other Settings and the note (`docs/plans/M7.md` § 4 › M7.3).
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::{Session, clock::fixed_clock};
use fontcraft_model::plist::Value as PlistValue;
use fontcraft_model::{Font, Instance, Master, Param, PropertyValue};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Runs a command that must be refused, checks it changed nothing, and returns the message.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.font().unwrap().clone();
    let undo = undo_steps(s);
    let message = match s.execute(id, &p) {
        Err(e) => e.to_string(),
        Ok(r) => panic!("{id} {p} was accepted: {r}"),
    };
    assert_eq!(s.font().unwrap(), &before, "{id} {p} changed the font");
    assert_eq!(undo_steps(s), undo, "{id} {p} added an undo step");
    message
}

fn undo_steps(s: &mut Session) -> usize {
    run(s, "document.inspect", json!({}))["documents"][0]["undo"].as_array().unwrap().len()
}

/// A font with two masters (`m01` Regular, `m02` Bold) and two exports (Regular; Bold with id `i2`).
fn session() -> Session {
    let mut font = Font::new("Fields");
    let first = font.masters[0].clone();
    let bold = Master { id: "m02".into(), name: "Bold".into(), ..first.clone() };
    font.masters = Arc::new(vec![first, bold]);
    font.instances = Arc::new(vec![
        Instance { name: "Regular".into(), ..Default::default() },
        Instance { name: "Bold".into(), id: Some("i2".into()), ..Default::default() },
    ]);
    let mut s = Session::new();
    s.add_document(font, None).unwrap();
    s
}

#[test]
fn font_info_get_and_set_reach_every_scope() {
    let mut s = session();
    assert_eq!(run(&mut s, "font.info.get", json!({"keys": ["unitsPerEm"]})), json!({"unitsPerEm": 1000}));

    // Font scope: built-in and added fields together, warnings in the result.
    let r = run(&mut s, "font.info.set", json!({"values": {"familyName": "Café & Co", "unitsPerEm": 4500, "designers": "Ana"}}));
    let warnings: Vec<&str> = r["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert!(warnings[0].contains("ASCII letters"));
    let font = run(&mut s, "font.info.get", json!({}));
    assert_eq!((font["familyName"].as_str(), font["unitsPerEm"].as_u64()), (Some("Café & Co"), Some(4500)));
    assert_eq!(font["fields"][0]["key"], "designers");
    assert_eq!(font["fields"][0]["value"], json!([{"language": "dflt", "value": "Ana"}]));
    assert_eq!(run(&mut s, "font.info.get", json!({"keys": ["designer"]}))["designer"], json!([{"language": "dflt", "value": "Ana"}]));

    // Two masters get the same value in one undo step.
    let steps = undo_steps(&mut s);
    run(&mut s, "font.info.set", json!({"scope": "master", "targets": ["m01", {"index": 1}], "values": {"name": "Same"}}));
    assert_eq!(undo_steps(&mut s), steps + 1);
    assert!(s.font().unwrap().masters.iter().all(|m| m.name == "Same"));
    let e = refused(&mut s, "font.info.set", json!({"scope": "master", "targets": "Same", "values": {"name": "Other"}}));
    assert!(e.contains("m01") && e.contains("m02"), "{e}");
    assert_eq!(run(&mut s, "font.info.get", json!({"scope": "master", "target": "m02"}))["name"], "Same");

    // Instances: a built-in class and an added localizable field, on both.
    run(
        &mut s,
        "font.info.set",
        json!({"scope": "instance", "targets": ["Regular", "i2"], "values": {"weightClass": 300, "styleMapFamilyNames": "Fields Light"}}),
    );
    let bold = run(&mut s, "font.info.get", json!({"scope": "instance", "target": "i2"}));
    assert_eq!(bold["weightClass"].as_f64(), Some(300.0));
    assert_eq!(bold["fields"][0]["value"], json!([{"language": "dflt", "value": "Fields Light"}]));
    assert_eq!(s.font().unwrap().instances[0].properties.value("styleMapFamilyNames"), Some("Fields Light"));

    // Refusals change nothing, even when the first target was fine.
    refused(&mut s, "font.info.set", json!({"scope": "instance", "targets": ["Regular", "nope"], "values": {"weightClass": 500}}));
    refused(&mut s, "font.info.set", json!({"scope": "instance", "targets": ["Regular"], "values": {"widthClass": 10}}));
    refused(&mut s, "font.info.set", json!({"scope": "master", "values": {"name": "No target"}}));
    refused(&mut s, "font.info.set", json!({"values": {"styleNames": "Instance only"}}));
    refused(&mut s, "font.info.set", json!({"values": {"unitsPerEm": 20_000}}));
    refused(&mut s, "font.info.set", json!({"values": {"familyName": 5}}));
    refused(&mut s, "font.info.set", json!({"values": {"date": "someday"}}));
}

/// `font.info.get` gives the Font tab's warnings for the current values, by field, in the words
/// `font.info.set` returns them; the CFF caution starts above 4096 units per em.
#[test]
fn font_info_get_gives_the_warnings_of_the_current_values() {
    let mut s = session();
    assert_eq!(run(&mut s, "font.info.get", json!({"keys": ["warnings"]})), json!({"warnings": []}));
    let set = |s: &mut Session, upm: u32| -> Vec<String> {
        let r = run(s, "font.info.set", json!({"values": {"unitsPerEm": upm}}));
        r["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect()
    };
    assert_eq!(set(&mut s, 4096).len(), 1);
    let said = set(&mut s, 4097);
    assert_eq!(said.len(), 2, "{said:?}");
    let shown = run(&mut s, "font.info.get", json!({}))["warnings"].clone();
    assert_eq!(shown, json!([{"field": "unitsPerEm", "message": said[0]}, {"field": "unitsPerEm", "message": said[1]}]));

    // An instance's warnings: its name and weight class.
    let r = run(&mut s, "font.info.set", json!({"scope": "instance", "targets": ["i2"], "values": {"weightClass": 350}}));
    let said = r["warnings"].as_array().unwrap().clone();
    assert_eq!(said.len(), 1, "{said:?}");
    let shown = run(&mut s, "font.info.get", json!({"scope": "instance", "target": "i2", "keys": ["warnings"]}));
    assert_eq!(shown, json!({"warnings": [{"field": "weightClass", "message": said[0]}]}));
}

#[test]
fn localizable_fields_gain_and_lose_languages() {
    let mut s = session();
    let start = s.font().unwrap().clone();
    let listed = run(&mut s, "font.info.listFields", json!({"scope": "font", "query": "family"}));
    let keys: Vec<&str> = listed["fields"].as_array().unwrap().iter().map(|f| f["key"].as_str().unwrap()).collect();
    assert_eq!(keys, ["familyNames", "WWSFamilyName", "WWSSubfamilyName"]);
    assert_eq!(run(&mut s, "font.info.listFields", json!({"scope": "master"}))["fields"], json!([]));

    // A new familyNames starts from the Family Name, in the default language.
    run(&mut s, "font.info.addField", json!({"scope": "font", "key": "familyNames"}));
    let names = |s: &mut Session| run(s, "font.info.get", json!({"keys": ["familyNames"]}))["familyNames"].clone();
    assert_eq!(names(&mut s), json!([{"language": "dflt", "value": "Fields"}]));
    assert_eq!(run(&mut s, "font.info.listFields", json!({"scope": "font", "query": "familyNames"}))["fields"][0]["present"], true);
    assert!(refused(&mut s, "font.info.addField", json!({"key": "familyNames"})).contains("already has"));

    run(&mut s, "font.info.setLocalizedValue", json!({"key": "familyNames", "language": "DEU", "value": "Felder"}));
    assert_eq!(names(&mut s), json!([{"language": "dflt", "value": "Fields"}, {"language": "DEU", "value": "Felder"}]));
    refused(&mut s, "font.info.setLocalizedValue", json!({"key": "familyNames", "language": "XYZ", "value": "?"}));
    refused(&mut s, "font.info.setLocalizedValue", json!({"key": "vendorID", "language": "DEU", "value": "ABCD"}));

    let r = run(&mut s, "font.info.removeLocalizedValue", json!({"key": "familyNames", "language": "dflt"}));
    assert_eq!(r["fieldRemoved"], false);
    assert_eq!(names(&mut s), json!([{"language": "DEU", "value": "Felder"}]));
    let r = run(&mut s, "font.info.removeLocalizedValue", json!({"key": "familyNames", "language": "DEU"}));
    assert_eq!(r["fieldRemoved"], true);
    assert!(s.execute("font.info.get", &json!({"keys": ["familyNames"]})).is_err());

    // An export: the singular spelling names the plural field, which starts from the Style Name.
    let r = run(&mut s, "font.info.addField", json!({"scope": "instance", "targets": ["Bold"], "key": "styleName", "language": "DEU"}));
    assert_eq!(r["key"], "styleNames");
    assert_eq!(s.font().unwrap().instances[1].properties.get("styleNames").map(|p| p.default_value()), Some(Some("Bold")));
    refused(&mut s, "font.info.removeField", json!({"keys": ["unitsPerEm"]}));
    refused(&mut s, "font.info.removeField", json!({"scope": "instance", "targets": ["Regular", "Bold"], "keys": ["styleNames"]}));
    run(&mut s, "font.info.removeField", json!({"scope": "instance", "targets": ["i2"], "keys": ["styleNames"]}));
    assert!(s.font().unwrap().instances[1].properties.is_empty());

    // Each change was one undo step.
    for _ in 0..6 {
        run(&mut s, "edit.undo", json!({}));
    }
    assert_eq!(s.font().unwrap(), &start);
}

/// A localized edit of several exports is one undo step: a language change keeps each export's own text, an added
/// language leaves a text already there alone, and exports without a language are left alone when it goes.
#[test]
fn localized_edits_reach_several_exports_in_one_step() {
    let mut s = session();
    let set = |p: Value| {
        let mut all = json!({"scope": "instance", "key": "styleNames"});
        for (k, v) in p.as_object().unwrap() {
            all[k] = v.clone();
        }
        all
    };
    run(&mut s, "font.info.setLocalizedValue", set(json!({"target": 0, "language": "DEU", "value": "Normal"})));
    run(&mut s, "font.info.setLocalizedValue", set(json!({"target": 1, "language": "DEU", "value": "Fett"})));
    let texts = |s: &Session, language: &str| -> Vec<Option<String>> {
        let text = |i: &Instance| match &i.properties.get("styleNames")?.value {
            PropertyValue::Localized(l) => l.iter().find(|l| l.language == language).map(|l| l.value.clone()),
            PropertyValue::Single(_) => None,
        };
        s.font().unwrap().instances.iter().map(text).collect()
    };
    let before = undo_steps(&mut s);
    let r = run(&mut s, "font.info.setLocalizedValue", set(json!({"targets": [0, 1], "from": "DEU", "language": "FRA"})));
    assert_eq!(r["targets"], json!([0, 1]));
    assert_eq!(texts(&s, "FRA"), [Some("Normal".to_string()), Some("Fett".into())]);
    assert_eq!(texts(&s, "DEU"), [None, None]);
    assert_eq!(undo_steps(&mut s), before + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(texts(&s, "DEU"), [Some("Normal".to_string()), Some("Fett".into())]);

    run(&mut s, "font.info.removeLocalizedValue", set(json!({"target": 0, "language": "DEU"})));
    run(&mut s, "font.info.setLocalizedValue", set(json!({"targets": [0, 1], "language": "DEU", "value": "", "onlyMissing": true})));
    assert_eq!(texts(&s, "DEU"), [Some(String::new()), Some("Fett".into())]);
    refused(&mut s, "font.info.setLocalizedValue", set(json!({"targets": [0, 1], "from": "ITA", "language": "FRA"})));
    run(&mut s, "font.info.removeLocalizedValue", set(json!({"target": 0, "language": "DEU"})));
    let r = run(&mut s, "font.info.removeLocalizedValue", set(json!({"targets": [0, 1], "language": "DEU"})));
    assert_eq!((r["targets"].clone(), r["fieldRemoved"].clone()), (json!([1]), json!(true)));
    refused(&mut s, "font.info.removeLocalizedValue", set(json!({"targets": [0, 1], "language": "DEU"})));
}

/// Rows are copied and removed one by one: `index` picks among rows of one name, a target without a row keeps its
/// others, built-in fields stay, and one undo step brings everything back.
#[test]
fn single_rows_copy_and_go() {
    let mut s = session();
    for value in ["RemoveOverlap", "RoundCorner; 10"] {
        run(&mut s, "font.customParameters.add", json!({"scope": "instance", "targets": [0], "name": "Filter", "value": value}));
    }
    run(&mut s, "font.customParameters.add", json!({"scope": "instance", "targets": [0, 1], "name": "Remove Glyphs", "value": ["A"]}));
    let rows = json!([{"key": "Filter", "kind": "param", "index": 1}, {"key": "Remove Glyphs"}, {"key": "weightClass"}]);
    let copied = run(&mut s, "font.info.copyRows", json!({"scope": "instance", "target": 0, "rows": [{"key": "Filter", "index": 1}]}));
    assert_eq!(copied["rows"].as_array().unwrap().len(), 1);
    assert_eq!(copied["rows"][0]["value"], "RoundCorner; 10");
    let before = undo_steps(&mut s);
    let r = run(&mut s, "font.info.removeRows", json!({"scope": "instance", "targets": [0, 1], "rows": rows}));
    assert_eq!(r["removed"], 3);
    assert_eq!(r["skipped"].as_array().unwrap().len(), 1, "the weight class is built in");
    let counts = |s: &Session| s.font().unwrap().instances.iter().map(|i| i.params.0.len()).collect::<Vec<_>>();
    assert_eq!(counts(&s), [1, 0]);
    assert_eq!(undo_steps(&mut s), before + 1);
    refused(&mut s, "font.info.removeRows", json!({"scope": "instance", "targets": [1], "rows": rows}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(counts(&s), [3, 1]);

    // A parameter only one export holds is set on both, as one step.
    let set = |add: bool| json!({"scope": "instance", "targets": [0, 1], "name": "Filter", "index": 0, "value": "AddExtremes", "addMissing": add});
    refused(&mut s, "font.customParameters.set", set(false));
    run(&mut s, "font.customParameters.set", set(true));
    assert_eq!(counts(&s), [3, 2]);
    assert_eq!(s.font().unwrap().instances[1].params.get("Filter").and_then(PlistValue::as_str), Some("AddExtremes"));
}

#[test]
fn rows_paste_into_another_font_and_an_export() {
    let mut a = session();
    run(&mut a, "font.info.set", json!({"values": {"designers": "Ana", "vendorID": "FCFT"}}));
    a.edit("Add a parameter", |f| {
        Arc::make_mut(&mut f.info).params.0.push(Param::new("fcTestNote", PlistValue::String("kept".into())));
        Ok(())
    })
    .unwrap();
    let copied = run(&mut a, "font.info.copyRows", json!({"scope": "font"}));
    assert_eq!(copied["rows"].as_array().unwrap().len(), 3);
    let text = copied["text"].as_str().unwrap().to_string();
    assert!(text.contains("vendorID") && text.contains("fcTestNote"), "{text}");

    // Replace: a field the target has takes the pasted value.
    let mut b = session();
    run(&mut b, "font.info.set", json!({"values": {"vendorID": "XXXX"}}));
    let r = run(&mut b, "font.info.pasteRows", json!({"text": text}));
    assert_eq!((r["added"].as_u64(), r["replaced"].as_u64()), (Some(2), Some(1)));
    assert_eq!(b.font().unwrap().info.properties.value("vendorID"), Some("FCFT"));
    // Append: the parameter is added again; a field cannot repeat, so it is replaced.
    let r = run(&mut b, "font.info.pasteRows", json!({"text": text, "mode": "append"}));
    assert_eq!((r["added"].as_u64(), r["replaced"].as_u64()), (Some(1), Some(2)));
    assert_eq!(b.font().unwrap().info.params.all("fcTestNote").count(), 2);
    // Skip: everything is there already, so nothing changes.
    let steps = undo_steps(&mut b);
    let r = run(&mut b, "font.info.pasteRows", json!({"text": text, "mode": "skip"}));
    assert_eq!((r["added"].as_u64(), r["skipped"].as_array().unwrap().len()), (Some(0), 3));
    assert_eq!(undo_steps(&mut b), steps);

    // Into an export: its General section offers both fields. A master offers none.
    let r = run(&mut b, "font.info.pasteRows", json!({"scope": "instance", "targets": ["Regular"], "text": text}));
    assert_eq!(r["added"].as_u64(), Some(3));
    assert_eq!(b.font().unwrap().instances[0].properties.value("designers"), Some("Ana"));
    let r = run(&mut b, "font.info.pasteRows", json!({"scope": "master", "targets": ["m01"], "text": text}));
    assert_eq!((r["added"].as_u64(), r["skipped"].as_array().unwrap().len()), (Some(1), 2));
    assert_eq!(r["skipped"][0]["target"], 0);

    // Built-in rows by key, pasted from the JSON form and from the row list.
    run(&mut a, "font.info.set", json!({"values": {"unitsPerEm": 2048}}));
    let picked = run(&mut a, "font.info.copyRows", json!({"keys": ["unitsPerEm", "vendorID"]}));
    let mut c = session();
    run(&mut c, "font.info.pasteRows", json!({"text": picked["json"]}));
    assert_eq!((c.font().unwrap().units_per_em, c.font().unwrap().info.properties.value("vendorID")), (2048, Some("FCFT")));
    let mut d = session();
    run(&mut d, "font.info.pasteRows", json!({"rows": picked["rows"]}));
    assert_eq!(d.font().unwrap().units_per_em, 2048);

    refused(&mut d, "font.info.pasteRows", json!({"text": "not rows"}));
    refused(&mut d, "font.info.pasteRows", json!({"text": text, "mode": "merge"}));
    // A row whose value its check refuses is skipped and listed; the rest are pasted.
    let rows = json!([
        {"kind": "field", "key": "familyNames", "value": [{"language": "XYZ", "value": "?"}]},
        {"kind": "param", "key": "gasp Table", "value": {"65536": 1}},
        {"kind": "field", "key": "vendorID", "value": "ABCD"},
    ]);
    let r = run(&mut d, "font.info.pasteRows", json!({"rows": rows}));
    let skipped: Vec<&str> = r["skipped"].as_array().unwrap().iter().map(|s| s["key"].as_str().unwrap()).collect();
    assert_eq!(skipped, ["familyNames", "gasp Table"], "{r}");
    assert!(r["skipped"][1]["reason"].as_str().unwrap().contains("65536"), "{r}");
    assert_eq!(d.font().unwrap().info.properties.value("vendorID"), Some("ABCD"), "the good row is pasted");
    assert!(d.font().unwrap().info.params.get("gasp Table").is_none());
    refused(&mut d, "font.info.copyRows", json!({"keys": ["nothingHere"]}));
}

#[test]
fn version_steps_stop_at_both_ends_and_dates_are_utc() {
    let mut s = session();
    run(&mut s, "font.info.set", json!({"values": {"versionMinor": 998}}));
    assert_eq!(run(&mut s, "font.info.stepVersion", json!({}))["versionMinor"], 999);
    refused(&mut s, "font.info.stepVersion", json!({}));
    assert_eq!(run(&mut s, "font.info.stepVersion", json!({"delta": -999}))["versionMinor"], 0);
    refused(&mut s, "font.info.stepVersion", json!({"delta": -1}));
    assert_eq!(s.font().unwrap().version_major, 1);

    let r = run(&mut s, "font.info.updateDate", json!({"date": "2026-10-11T14:00:00+02:00"}));
    assert_eq!(r["date"], "2026-10-11 12:00:00 +0000");
    assert_eq!(run(&mut s, "font.info.get", json!({"keys": ["date"]}))["date"], "2026-10-11 12:00:00 +0000");
    s.set_clock(Some(fixed_clock(1_700_000_000)));
    assert_eq!(run(&mut s, "font.info.updateDate", json!({}))["date"], "2023-11-14 22:13:20 +0000");
    refused(&mut s, "font.info.updateDate", json!({"date": "soon"}));
    refused(&mut s, "font.info.updateDate", json!({"date": "1969-12-31T00:00:00Z"}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.font().unwrap().info.date.as_deref(), Some("2026-10-11 12:00:00 +0000"));
}

#[test]
fn settings_and_note_round_trip_through_glyphs() {
    let mut s = Session::with_document();
    let defaults = run(&mut s, "font.settings.get", json!({}));
    assert_eq!(
        defaults,
        json!({
            "gridSpacing": 1.0, "gridSubdivision": 1.0, "keyboardIncrementShift": 10.0, "keyboardIncrementCmd": 100.0,
            "useCustomNaming": false, "disableAutomaticAlignment": false, "keepAlternatesNextToBase": false,
            "fileFormatVersion": 3, "fontType": "default",
        })
    );
    let settings = json!({
        "gridSpacing": 5, "gridSubdivision": 2, "keyboardIncrementShift": 20, "keyboardIncrementCmd": 200,
        "useCustomNaming": true, "disableAutomaticAlignment": true, "keepAlternatesNextToBase": true, "fontType": "variable",
    });
    run(&mut s, "font.settings.set", settings);
    let note = "# Notes\n*draft*, not exported";
    run(&mut s, "font.note.set", json!({"text": note}));
    let set = run(&mut s, "font.settings.get", json!({}));
    assert_eq!((set["gridSpacing"].as_f64(), set["fontType"].as_str(), set["useCustomNaming"].as_bool()), (Some(5.0), Some("variable"), Some(true)));
    assert_eq!(run(&mut s, "font.settings.get", json!({"keys": ["gridSubdivision"]})), json!({"gridSubdivision": 2.0}));

    let dir = fontcraft_testkit::temp_dir("font-fields");
    let path = dir.join("settings.glyphs").to_string_lossy().into_owned();
    run(&mut s, "file.saveAs", json!({"path": path}));
    let mut t = Session::new();
    run(&mut t, "file.open", json!({"path": path}));
    assert_eq!(run(&mut t, "font.settings.get", json!({})), set);
    assert_eq!(run(&mut t, "font.note.get", json!({}))["text"], note);

    for bad in [
        json!({}),
        json!({"gridSubdivision": 0}),
        json!({"gridSpacing": -1}),
        json!({"keyboardIncrementShift": 0}),
        json!({"fontType": "iconSet"}),
        json!({"fileFormatVersion": 5}),
        json!({"nope": 1}),
    ] {
        refused(&mut s, "font.settings.set", bad);
    }
    run(&mut s, "font.settings.set", json!({"fileFormatVersion": 2}));
    assert_eq!(run(&mut s, "font.settings.get", json!({"keys": "fileFormatVersion"}))["fileFormatVersion"], 2);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "font.note.get", json!({}))["text"], "");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "font.settings.get", json!({})), defaults);
    std::fs::remove_dir_all(&dir).ok();
}

/// Pasted parameters match the rows the target holds under any spelling of the same parameter,
/// and a name parameter lands in the name property, as the custom-parameter commands do.
#[test]
fn pasted_parameters_match_other_spellings_and_name_parameters_become_properties() {
    let mut s = session();
    s.edit("An alias spelling", |f| {
        Arc::make_mut(&mut f.info).params.0.push(Param::new("postscriptBlueFuzz", PlistValue::Number(fontcraft_model::plist::Number::int(1))));
        Ok(())
    })
    .unwrap();
    let rows = json!([
        {"kind": "param", "key": "blueFuzz", "value": 2},
        {"kind": "param", "key": "designerURL", "value": "https://example.com"},
    ]);
    let r = run(&mut s, "font.info.pasteRows", json!({"rows": rows}));
    assert_eq!((r["added"].as_u64(), r["replaced"].as_u64()), (Some(1), Some(1)), "{r}");
    let info = &s.font().unwrap().info;
    assert_eq!(info.params.0.len(), 1, "the alias row takes the value: {:?}", info.params);
    assert!(matches!(&info.params.0[0].value, PlistValue::Number(n) if n.value().as_f64() == 2.0), "{:?}", info.params);
    assert_eq!(info.properties.value("designerURL"), Some("https://example.com"));

    let r = run(
        &mut s,
        "font.info.pasteRows",
        json!({"scope": "master", "targets": ["m01"], "rows": [{"kind": "param", "key": "designerURL", "value": "x"}]}),
    );
    assert_eq!(r["skipped"].as_array().unwrap().len(), 1, "{r}");
}

/// Views read settings every frame through `Session::query`, which leaves no command-log entry
/// and runs only queries.
#[test]
fn a_query_read_leaves_the_command_log_alone_and_refuses_an_edit() {
    let mut s = Session::with_document();
    let logged = s.extensions().log.len();
    let read = s.query("font.settings.get", &json!({})).unwrap();
    assert_eq!(read, s.execute("font.settings.get", &json!({})).unwrap());
    assert_eq!(s.extensions().log.len(), logged + 1, "only the execute is logged");
    let font = Arc::clone(&s.doc().unwrap().font);
    assert!(s.query("font.settings.set", &json!({"gridSpacing": 5})).is_err());
    assert!(Arc::ptr_eq(&font, &s.doc().unwrap().font));
}
