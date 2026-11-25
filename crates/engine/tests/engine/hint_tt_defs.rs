//! TT zone and stem definitions (M10.5): Get PS zones and stems, names kept unique and followed
//! through every master and every instruction, deltas and filters on the main master only, and
//! the stored values round-tripping through `.glyphs`.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::plist::{Value as PValue, openstep};
use fontcraft_engine::model::{Font, Path, Shape, StemDef};
use fontcraft_hinting::{Direction, HintSettings, NodeRef, StemRef, TtInstruction, TtKind, TtPoint, ZoneRef, set_tt_instructions, tt_instructions};
use serde_json::{Value, json};

fn snap(zone: usize, node: usize) -> TtInstruction {
    TtInstruction::new(TtKind::Snap { zone: ZoneRef::Index(zone) }, Direction::Vertical, TtPoint::Node(NodeRef::new(0, node)))
}

fn stem(stem: usize, direction: Direction) -> TtInstruction {
    let mut t = TtInstruction::new(TtKind::Stem { stem: StemRef::Index(stem), triple: None }, direction, TtPoint::Node(NodeRef::new(1, 1)));
    t.origin = Some(TtPoint::Node(NodeRef::new(1, 0)));
    t
}

/// Two masters (`m01`, the main one, and its copy), the starting font's five alignment zones,
/// four stem definitions, and glyph `I` with the instructions `tt` on `m01`. Returns the session
/// and the second master's id.
fn session(tt: &[TtInstruction]) -> (Session, String) {
    let mut s = Session::with_document();
    let out = s.execute("font.master.duplicate", &json!({"masters": ["m01"]})).unwrap();
    let second = out["masters"][0]["id"].as_str().unwrap().to_string();
    s.edit("setup", |font| {
        let def = |name: &str, horizontal: bool| StemDef { name: name.into(), horizontal, ..StemDef::default() };
        font.stems = Arc::new(vec![def("lc bar", true), def("uc bar", true), def("lc stem", false), def("uc stem", false)]);
        for (i, m) in Arc::make_mut(&mut font.masters).iter_mut().enumerate() {
            let k = i as f64 * 10.0;
            m.stem_values = vec![60.0 + k, 70.0 + k, 80.0 + k, 90.0 + k];
        }
        let mut g = font.glyph_mut("I")?;
        let layer = g.layer_mut("m01").unwrap();
        layer.shapes = vec![Shape::Path(Path::rect(100.0, 0.0, 200.0, 700.0)), Shape::Path(Path::rect(300.0, 0.0, 400.0, 700.0))];
        set_tt_instructions(layer, tt).unwrap();
        Ok(())
    })
    .unwrap();
    (s, second)
}

fn font(s: &Session) -> &Font {
    s.font().unwrap()
}

fn tt(s: &Session) -> Vec<TtKind> {
    let layer = font(s).glyph("I").unwrap().layer("m01").unwrap().clone();
    tt_instructions(&layer).unwrap().into_iter().map(|(_, t)| t.kind).collect()
}

fn names(out: &Value, key: &str) -> Vec<String> {
    out[key].as_array().unwrap().iter().map(|z| z["name"].as_str().unwrap().to_string()).collect()
}

fn settings(s: &Session, master: &str) -> HintSettings {
    HintSettings::for_master(font(s), master).unwrap()
}

fn param(s: &Session, master: &str, name: &str) -> Option<PValue> {
    font(s).master(master).unwrap().params.get(name).cloned()
}

fn plist(text: &str) -> PValue {
    openstep::parse_value(text).unwrap()
}

/// Runs `id`, checks that it changes the font and that undo gives back the very font, runs it
/// again and returns its result.
fn run(s: &mut Session, id: &str, p: Value) -> Value {
    let before = s.doc().unwrap().font.clone();
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    assert!(!Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} changed nothing");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id}: undo does not give back the very font");
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Runs `id` expecting a refusal that leaves the font pointer-equal; returns the message.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let e = s.execute(id, &p).expect_err(id).to_string();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id}: a refusal changed the font");
    e
}

#[test]
fn get_ps_zones_and_stems_replace_the_list_and_instructions_follow_by_name() {
    let (mut s, second) = session(&[snap(2, 0), snap(5, 1)]);
    run(&mut s, "hint.tt.addZone", json!({"master": "m01", "name": "smallcap", "position": 560, "size": 12}));

    let out = run(&mut s, "hint.tt.getPsZones", json!({"master": "m01"}));
    assert_eq!(names(&out, "zones"), ["ascender", "capHeight", "xHeight", "baseline", "descender"], "replaced, named after the metrics");
    assert_eq!((out["zones"][2]["position"].as_f64(), out["zones"][2]["size"].as_f64()), (Some(500.0), Some(16.0)));
    assert_eq!((out["zones"][4]["position"].as_f64(), out["zones"][4]["size"].as_f64()), (Some(-200.0), Some(-16.0)));
    assert_eq!(out["instructions"], json!(1));
    assert_eq!(tt(&s), [TtKind::Snap { zone: ZoneRef::Index(2) }, TtKind::Snap { zone: ZoneRef::Auto }], "xHeight kept, smallcap gone");
    assert_eq!(settings(&s, "m01").tt_zones.len(), 5);

    let out = run(&mut s, "hint.tt.getPsStems", json!({"master": "m01"}));
    assert_eq!(names(&out, "stems"), ["lc bar", "uc bar", "lc stem", "uc stem"]);
    let orientations: Vec<&str> = out["stems"].as_array().unwrap().iter().map(|t| t["orientation"].as_str().unwrap()).collect();
    assert_eq!(orientations, ["y", "y", "x", "x"]);
    assert_eq!(out["stems"][1]["size"].as_f64(), Some(70.0));
    assert!(param(&s, "m01", "TTFStems").is_some());

    // Another master takes its own values; the main master's list is untouched.
    let out = run(&mut s, "hint.tt.getPsStems", json!({"master": second}));
    assert_eq!(out["stems"][1]["size"].as_f64(), Some(80.0));
    assert_eq!(settings(&s, "m01").tt_stems[1].size, 70.0);
    refused(&mut s, "hint.tt.getPsZones", json!({"master": "nope"}));
}

/// `family: true` edits the font's TTFZones: a list of its own, without deltas and filters, which leaves the masters'
/// zones and the instructions alone.
#[test]
fn family_zones_are_edited_on_the_font() {
    let (mut s, _) = session(&[snap(1, 0)]);
    let masters = settings(&s, "m01").tt_zones;
    let out = run(&mut s, "hint.tt.getPsZones", json!({"family": true}));
    assert_eq!(names(&out, "zones"), masters.iter().map(|z| z.name.clone()).collect::<Vec<_>>());
    assert_eq!((&out["family"], out.get("master")), (&json!(true), None));
    run(&mut s, "hint.tt.setZone", json!({"family": true, "zone": "xHeight", "name": "x", "position": 510, "size": 14}));
    run(&mut s, "hint.tt.setZone", json!({"family": true, "zone": "capHeight", "align": "x"}));
    let out = run(&mut s, "hint.tt.addZone", json!({"family": true, "position": 600, "size": 12}));
    assert_eq!(names(&out, "zones").last().map(String::as_str), Some("zone 6"));
    run(&mut s, "hint.tt.removeZone", json!({"family": true, "zone": "x"}));
    let family = fontcraft_hinting::settings::family_tt_zones(font(&s)).unwrap();
    assert_eq!(family.len(), masters.len());
    assert!(family.iter().all(|z| z.align.is_none() && z.name != "x"), "{family:?}");
    assert_eq!(settings(&s, "m01").tt_zones, masters, "the masters keep their zones");
    assert_eq!(tt(&s)[0], TtKind::Snap { zone: ZoneRef::Index(1) });

    refused(&mut s, "hint.tt.setZone", json!({"family": true, "zone": "capHeight", "filter": "category == \"Letter\""}));
    refused(&mut s, "hint.tt.setZone", json!({"family": true, "master": "m01", "zone": "capHeight", "size": 3}));
    refused(&mut s, "hint.tt.removeZone", json!({"family": true, "zone": "x"}));
}

#[test]
fn add_rename_and_remove_keep_names_unique_and_follow_every_master_and_instruction() {
    let (mut s, second) =
        session(&[snap(1, 0), snap(5, 1), snap(2, 2), stem(0, Direction::Vertical), stem(2, Direction::Horizontal), stem(3, Direction::Horizontal)]);

    // Added through the second master, the zone reaches both.
    let out = run(&mut s, "hint.tt.addZone", json!({"master": second, "name": "smallcap", "position": 550, "size": 12}));
    assert_eq!(names(&out, "zones").last().map(String::as_str), Some("smallcap"));
    for m in ["m01", second.as_str()] {
        assert_eq!(settings(&s, m).tt_zones.last().unwrap().name, "smallcap", "{m}");
        assert!(param(&s, m, "TTFZones").is_some(), "{m} has its own list");
    }
    let e = refused(&mut s, "hint.tt.addZone", json!({"master": "m01", "name": "xHeight", "position": 0, "size": 1}));
    assert!(e.contains("unique"), "{e}");
    refused(&mut s, "hint.tt.addZone", json!({"master": "m01", "name": "auto", "position": 0, "size": 1}));
    refused(&mut s, "hint.tt.addZone", json!({"master": "m01", "position": 0, "size": 1, "align": "nope"}));
    let out = run(&mut s, "hint.tt.addZone", json!({"master": "m01", "position": 0, "size": -10}));
    assert_eq!(names(&out, "zones").last().map(String::as_str), Some("zone 7"), "named after its place in the list");
    run(&mut s, "hint.tt.removeZone", json!({"master": "m01", "zone": "zone 7"}));

    // Alignments follow a rename; position and size belong to one master.
    run(&mut s, "hint.tt.setZone", json!({"master": "m01", "zone": "capHeight", "align": "smallcap"}));
    run(&mut s, "hint.tt.setZone", json!({"master": "m01", "zone": "xHeight", "align": "capHeight"}));
    let e = refused(&mut s, "hint.tt.setZone", json!({"master": "m01", "zone": "smallcap", "align": "xHeight"}));
    assert!(e.contains("itself"), "{e}");
    refused(&mut s, "hint.tt.setZone", json!({"master": "m01", "zone": "smallcap", "name": "ascender"}));
    run(&mut s, "hint.tt.setZone", json!({"master": "m01", "zone": "smallcap", "name": "sc"}));
    run(&mut s, "hint.tt.setZone", json!({"master": second, "zone": "sc", "position": 560}));
    let (main, other) = (settings(&s, "m01"), settings(&s, &second));
    assert_eq!(main.tt_zones[1].align.as_deref(), Some("sc"));
    assert_eq!((main.tt_zones[5].name.as_str(), main.tt_zones[5].position), ("sc", 550.0));
    assert_eq!((other.tt_zones[5].name.as_str(), other.tt_zones[5].position), ("sc", 560.0));
    assert_eq!(tt(&s)[1], TtKind::Snap { zone: ZoneRef::Index(5) }, "a rename leaves the index");
    refused(&mut s, "hint.tt.setZone", json!({"master": "m01", "zone": "sc"}));

    // Removing capHeight: its Snap turns to Auto, later indexes move down, xHeight loses its alignment.
    let out = run(&mut s, "hint.tt.removeZone", json!({"master": second, "zone": "capHeight"}));
    assert_eq!(out["instructions"], json!(3));
    assert_eq!(
        &tt(&s)[..3],
        [TtKind::Snap { zone: ZoneRef::Auto }, TtKind::Snap { zone: ZoneRef::Index(4) }, TtKind::Snap { zone: ZoneRef::Index(1) }]
    );
    for m in ["m01", second.as_str()] {
        let zones = settings(&s, m).tt_zones;
        assert!(zones.iter().all(|z| z.name != "capHeight"), "{m}");
        assert_eq!(zones[1].align, None, "{m}");
    }
    refused(&mut s, "hint.tt.removeZone", json!({"master": "m01", "zone": "capHeight"}));

    // Stems: a rename reaches both masters; a removal and a turned orientation re-point instructions.
    run(&mut s, "hint.tt.addStemDefinition", json!({"master": "m01", "name": "hair", "orientation": "y", "size": 20}));
    refused(&mut s, "hint.tt.addStemDefinition", json!({"master": "m01", "name": "hair", "orientation": "y", "size": 20}));
    refused(&mut s, "hint.tt.addStemDefinition", json!({"master": "m01", "name": "thin", "orientation": "z", "size": 20}));
    refused(&mut s, "hint.tt.addStemDefinition", json!({"master": "m01", "name": "thin", "orientation": "x", "size": -1}));
    run(&mut s, "hint.tt.setStemDefinition", json!({"master": second, "stem": "lc bar", "name": "bar"}));
    assert_eq!(settings(&s, "m01").tt_stems[0].name, "bar");
    assert_eq!(settings(&s, &second).tt_stems[0].name, "bar");
    let out = run(&mut s, "hint.tt.removeStemDefinition", json!({"master": "m01", "stem": "uc bar"}));
    assert_eq!(out["instructions"], json!(2));
    let stems: Vec<TtKind> = tt(&s)[3..].to_vec();
    let at = |i| TtKind::Stem { stem: StemRef::Index(i), triple: None };
    assert_eq!(stems, [at(0), at(1), at(2)]);
    // `lc stem` turned to a Y stem no longer suits its horizontal instruction.
    let out = run(&mut s, "hint.tt.setStemDefinition", json!({"master": "m01", "stem": "lc stem", "orientation": "y"}));
    assert_eq!(out["instructions"], json!(1));
    assert_eq!(tt(&s)[4], TtKind::Stem { stem: StemRef::Auto, triple: None });
    assert!(settings(&s, &second).tt_stems[1].horizontal);
}

#[test]
fn deltas_and_filters_are_refused_outside_the_main_master() {
    let (mut s, second) = session(&[]);
    let e = refused(&mut s, "hint.tt.setZone", json!({"master": second, "zone": "xHeight", "deltas": {"m01": {"12": "up"}}}));
    assert!(e.contains("main master"), "{e}");
    refused(&mut s, "hint.tt.setZone", json!({"master": second, "zone": "xHeight", "filter": "name == \"a\""}));
    refused(&mut s, "hint.tt.setStemDefinition", json!({"master": second, "stem": "lc bar", "deltas": {"m01": {"12": "down"}}}));
    refused(&mut s, "hint.tt.setStemDefinition", json!({"master": second, "stem": "lc bar", "filter": null}));

    let out = run(&mut s, "hint.tt.setZone", json!({"master": "m01", "zone": "xHeight", "deltas": {second.clone(): {"12": "up", "13": "down"}}}));
    assert_eq!(out["zones"][2]["deltas"], json!({second.clone(): {"12": "up", "13": "down"}}));
    run(
        &mut s,
        "hint.tt.setZone",
        json!({"master": "m01", "zone": "xHeight", "deltas": {second.clone(): {"13": "none"}}, "filter": "name beginswith \"a\""}),
    );
    let stored = param(&s, "m01", "TTFZones").unwrap();
    let x = &stored.as_array().unwrap().items[2];
    let deltas = x.as_dict().unwrap().get("deltas").unwrap().as_dict().unwrap();
    assert_eq!(deltas.len(), 1);
    let cells = deltas.get(&second).unwrap().as_dict().unwrap();
    assert_eq!(cells.len(), 1);
    assert_eq!(cells.get("12").and_then(PValue::as_num).map(|n| n.as_f64()), Some(1.0), "one pixel up");
    assert_eq!(settings(&s, "m01").tt_zones[2].filter.as_deref(), Some("name beginswith \"a\""));
    // The other master's own list carries neither.
    let other = param(&s, &second, "TTFZones");
    assert!(other.is_none() || other.unwrap().as_array().unwrap().items.iter().all(|z| z.as_dict().unwrap().get("deltas").is_none()));

    refused(&mut s, "hint.tt.setZone", json!({"master": "m01", "zone": "xHeight", "deltas": {"nobody": {"12": "up"}}}));
    refused(&mut s, "hint.tt.setZone", json!({"master": "m01", "zone": "xHeight", "deltas": {"m01": {"0": "up"}}}));
    refused(&mut s, "hint.tt.setZone", json!({"master": "m01", "zone": "xHeight", "deltas": {"m01": {"12": "sideways"}}}));
    refused(&mut s, "hint.tt.setZone", json!({"master": "m01", "zone": "xHeight", "filter": "(("}));
    run(&mut s, "hint.tt.setZone", json!({"master": "m01", "zone": "xHeight", "filter": null}));
    assert_eq!(settings(&s, "m01").tt_zones[2].filter, None);
}

#[test]
fn written_definitions_round_trip_through_glyphs_and_keep_unknown_keys() {
    let (mut s, second) = session(&[]);
    s.edit("stored stems", |font| {
        let m = Arc::make_mut(&mut font.masters).iter_mut().find(|m| m.id == "m01").unwrap();
        m.params.set("TTFStems", plist("({horizontal = 1; name = Thin; size = 16; note = kept;})"));
        Ok(())
    })
    .unwrap();
    run(&mut s, "hint.tt.setStemDefinition", json!({"master": "m01", "stem": "Thin", "size": 18, "deltas": {"m01": {"9": "up"}}}));
    run(&mut s, "hint.tt.addStemDefinition", json!({"master": "m01", "name": "Stem", "orientation": "x", "size": 80}));
    run(&mut s, "hint.tt.addZone", json!({"master": "m01", "name": "smallcap", "position": 550, "size": 12, "align": "xHeight"}));
    run(&mut s, "hint.tt.setZone", json!({"master": second, "zone": "smallcap", "position": 555}));
    assert_eq!(
        param(&s, "m01", "TTFStems"),
        Some(plist("({horizontal = 1; name = Thin; size = 18; note = kept; deltas = {m01 = {9 = 1;};};}, {name = Stem; width = 80;})"))
    );

    let text = fontcraft_io_glyphs::write(font(&s)).unwrap();
    let back = fontcraft_io_glyphs::read(&text).unwrap();
    for m in ["m01", second.as_str()] {
        for name in ["TTFZones", "TTFStems"] {
            assert_eq!(back.master(m).unwrap().params.get(name), param(&s, m, name).as_ref(), "{m} {name}");
        }
        let (a, b) = (HintSettings::for_master(&back, m).unwrap(), settings(&s, m));
        assert_eq!((a.tt_zones, a.tt_stems), (b.tt_zones, b.tt_stems), "{m}");
    }
}
