//! Master commands: list, get (several masters at once), set, add, duplicate, remove, reorder and
//! the master icon, on a two-master font built in code with a brace layer and kerning.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::cmd::masters::MASTER_ICONS;
use fontcraft_engine::{EngineError, Session};
use fontcraft_model::fc_keys::LINKED_SOURCE;
use fontcraft_model::plist::{Str, Value as Plist};
use fontcraft_model::{Axis, Font, Layer, Master, MetricValue, Param};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Runs a command that must be refused, with a message holding `needle`, and leave the font as it
/// was: the very same pointer.
fn refused(s: &mut Session, id: &str, p: Value, needle: &str) {
    let before = Arc::clone(&s.doc().unwrap().font);
    match s.execute(id, &p) {
        Err(EngineError::BadParams { message, .. }) => assert!(message.contains(needle), "{id} {p}: {message}"),
        other => panic!("{id} {p}: expected a refusal naming `{needle}`, got {other:?}"),
    }
    assert!(Arc::ptr_eq(&before, &s.doc().unwrap().font), "{id} {p}: a refused command changed the font");
}

fn font(s: &Session) -> &Font {
    s.font().unwrap()
}

fn master_ids(s: &Session) -> Vec<String> {
    font(s).masters.iter().map(|m| m.id.clone()).collect()
}

/// Regular (`m01`, wght 400) and Bold (`m02`, wght 700). Every glyph has a layer for both;
/// `a` also has a brace layer tied to Bold at wght 550. Kerning: T–o −40 in Regular, −60 in Bold.
/// Regular's `Get Hints From Master` parameter names Bold.
fn two_masters() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        Arc::make_mut(&mut font.axes).push(Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() });
        let masters = Arc::make_mut(&mut font.masters);
        let regular = masters.get_mut(0).unwrap();
        regular.location = vec![400.0];
        regular.params.0.push(Param::new("Get Hints From Master", Plist::String(Str::new("Bold"))));
        let mut bold = Master { id: "m02".into(), name: "Bold".into(), location: vec![700.0], ..Default::default() };
        bold.metric_values = regular.metric_values.clone();
        if let Some(v) = bold.metric_values.first_mut() {
            *v = MetricValue { pos: 820.0, over: 16.0 };
        }
        masters.push(bold);
        for i in 0..font.glyphs.len() {
            let mut g = font.glyphs.get_mut_at(i).unwrap();
            let width = g.layers[0].width + 40.0;
            g.layers.push(Layer::new("m02", width));
            if g.name == "a" {
                let mut brace = Layer::new("m02", width);
                brace.id = "brace-a".into();
                brace.info.name = Some("{550}".into());
                brace.info.attrs.coordinates = vec![Some(550.0)];
                g.layers.push(brace);
            }
        }
        let k = Arc::make_mut(&mut font.kerning);
        k.ltr.set("m01", "T", "o", -40.0);
        k.ltr.set("m02", "T", "o", -60.0);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn a_new_font_has_one_master_called_regular() {
    let mut s = Session::with_document();
    let r = run(&mut s, "font.master.list", json!({}));
    let masters = r["masters"].as_array().unwrap();
    assert_eq!(masters.len(), 1);
    assert_eq!(masters[0]["name"], "Regular");
}

#[test]
fn list_and_get_report_each_master_and_null_where_several_differ() {
    let mut s = two_masters();
    let r = run(&mut s, "font.master.list", json!({}));
    assert_eq!(r["masters"][1], json!({"index": 1, "id": "m02", "name": "Bold", "icon": null, "axes": {"wght": 700.0}}));

    let bold = run(&mut s, "font.master.get", json!({"master": "Bold"}));
    assert_eq!(bold["id"], "m02");
    assert_eq!(bold["metrics"]["ascender"], json!({"position": 820.0, "zone": 16.0}));

    let both = run(&mut s, "font.master.get", json!({"masters": ["m01", "m02"]}));
    assert_eq!(both["ids"], json!(["m01", "m02"]));
    assert_eq!(both["name"], Value::Null, "the names differ");
    assert_eq!(both["axes"]["wght"], Value::Null, "the coordinates differ");
    assert_eq!(both["metrics"]["ascender"]["position"], Value::Null, "the ascenders differ");
    assert_eq!(both["metrics"]["ascender"]["zone"], json!(16.0), "the zones agree");
    assert_eq!(both["metrics"]["x-height"], json!({"position": 500.0, "zone": 16.0}));
}

#[test]
fn set_writes_every_named_master_in_one_undo_step() {
    let mut s = two_masters();
    run(&mut s, "font.master.set", json!({"masters": ["Regular", "Bold"], "metrics": {"x-height": {"position": 520}}, "axes": {"wght": 450}}));
    for m in font(&s).masters.iter() {
        assert_eq!(font(&s).metric_pos(m, "x-height", 0.0), 520.0, "{}", m.name);
        assert_eq!(m.location, vec![450.0], "{}", m.name);
    }
    run(&mut s, "edit.undo", json!({}));
    let f = font(&s);
    assert_eq!(f.masters.iter().map(|m| m.location[0]).collect::<Vec<_>>(), vec![400.0, 700.0], "one undo restores both masters");

    run(&mut s, "font.master.set", json!({"masters": ["m02"], "name": "Black"}));
    assert_eq!(font(&s).masters[1].name, "Black");
    refused(&mut s, "font.master.set", json!({"masters": ["m02"], "name": "  "}), "empty");
    refused(&mut s, "font.master.set", json!({"masters": ["m02"], "metrics": {"overshoot": {"position": 1}}}), "no metric `overshoot`");
    refused(&mut s, "font.master.set", json!({"masters": ["m02"], "axes": {"wdth": 100}}), "no axis `wdth`");
    refused(&mut s, "font.master.set", json!({"masters": ["m02"], "numbers": {"stroke": 10}}), "no number `stroke`");
    refused(&mut s, "font.master.set", json!({"masters": ["m02"]}), "at least one");
}

#[test]
fn add_appends_an_empty_master_with_a_layer_in_every_glyph() {
    let mut s = two_masters();
    let r = run(&mut s, "font.master.add", json!({}));
    let id = r["master"]["id"].as_str().unwrap().to_string();
    assert_eq!(r["master"]["index"], 2);
    let f = font(&s);
    let m = &f.masters[2];
    assert_eq!(m.name, "Regular");
    assert_eq!(m.location, vec![400.0], "the first master's coordinates");
    assert_eq!(f.metric(m, "ascender"), Some(MetricValue { pos: 800.0, over: 16.0 }));
    assert_eq!(f.metric(m, "descender"), Some(MetricValue { pos: -200.0, over: -16.0 }));
    for g in f.glyphs.iter() {
        let layer = g.layers.iter().find(|l| l.id == id).unwrap_or_else(|| panic!("{} has no layer for the new master", g.name));
        assert!(layer.shapes.is_empty() && layer.anchors.is_empty());
    }

    run(&mut s, "font.master.add", json!({"name": "Thin", "axes": {"wght": 100}}));
    assert_eq!(font(&s).masters[3].location, vec![100.0]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(font(&s).masters.len(), 3);
}

#[test]
fn duplicate_copies_layers_brace_layers_and_kerning_right_after_the_original() {
    let mut s = two_masters();
    run(&mut s, "font.master.set", json!({"masters": ["m01"], "icon": "Light"}));
    let r = run(&mut s, "font.master.duplicate", json!({"masters": ["m01", "Bold"]}));
    let made = r["masters"].as_array().unwrap();
    let regular_copy = made[0]["id"].as_str().unwrap().to_string();
    let bold_copy = made[1]["id"].as_str().unwrap().to_string();
    assert_eq!(master_ids(&s), vec!["m01".to_string(), regular_copy.clone(), "m02".into(), bold_copy.clone()]);
    assert_eq!(made[0]["index"], 1);
    assert_eq!(made[1]["index"], 3);

    let f = font(&s);
    assert_eq!(f.masters[3].name, "Bold", "a copy keeps the name");
    assert_eq!(f.masters[1].icon.as_deref(), Some("Light"));
    let a = f.glyph("a").unwrap();
    let copy_layer = a.layers.iter().find(|l| l.id == bold_copy).unwrap();
    assert_eq!(copy_layer.width, a.layers.iter().find(|l| l.id == "m02").unwrap().width);
    let braces: Vec<&Layer> = a.layers.iter().filter(|l| l.info.attrs.coordinates == vec![Some(550.0)]).collect();
    assert_eq!(braces.len(), 2, "the brace layer is copied");
    assert!(braces.iter().any(|l| l.master == bold_copy && l.id != "brace-a"), "the copy is tied to the new master with an id of its own");
    assert_eq!(f.kerning.ltr.get(&bold_copy, "T", "o"), Some(-60.0));
    assert_eq!(f.kerning.ltr.get(&regular_copy, "T", "o"), Some(-40.0));

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(master_ids(&s), vec!["m01".to_string(), "m02".into()], "one undo step");
}

#[test]
fn a_copy_of_a_linked_master_belongs_to_the_document() {
    let mut s = two_masters();
    s.edit("link Bold", |font| {
        let bold = Arc::make_mut(&mut font.masters).get_mut(1).unwrap();
        bold.user_data.0.insert(LINKED_SOURCE, Plist::String(Str::new("Other.glyphs")));
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "font.master.duplicate", json!({"masters": ["m02"]}));
    let copy = r["masters"][0]["id"].as_str().unwrap().to_string();
    let f = font(&s);
    let m = f.masters.iter().find(|m| m.id == copy).unwrap();
    assert!(m.user_data.get(LINKED_SOURCE).is_none(), "the copy is saved with the document, so it carries no linked mark");
    assert!(f.masters[1].user_data.get(LINKED_SOURCE).is_some(), "the linked original keeps its mark");
}

#[test]
fn a_name_two_masters_share_is_refused_as_a_target() {
    let mut s = two_masters();
    run(&mut s, "font.master.duplicate", json!({"masters": ["Bold"]}));
    refused(&mut s, "font.master.get", json!({"master": "Bold"}), "more than one master");
    refused(&mut s, "font.master.remove", json!({"masters": ["Bold"]}), "more than one master");
}

#[test]
fn remove_takes_layers_and_kerning_and_refuses_the_last_master() {
    let mut s = two_masters();
    let r = run(&mut s, "font.master.remove", json!({"masters": ["Bold"]}));
    assert_eq!(r["removed"], json!(["m02"]));
    let warnings = r["warnings"].as_array().unwrap();
    assert!(warnings.iter().any(|w| w.as_str().unwrap().contains("Get Hints From Master")), "{warnings:?}");
    let f = font(&s);
    assert_eq!(master_ids(&s), vec!["m01".to_string()]);
    assert!(f.glyphs.iter().all(|g| g.layers.iter().all(|l| l.master == "m01")), "no layer is left tied to Bold");
    assert_eq!(f.kerning.ltr.get("m02", "T", "o"), None);
    assert_eq!(f.kerning.ltr.get("m01", "T", "o"), Some(-40.0));
    assert_eq!(f.masters[0].params.get("Get Hints From Master").and_then(Plist::as_str), Some("Bold"), "the parameter stays");

    refused(&mut s, "font.master.remove", json!({"masters": ["Regular"]}), "at least one master");
    run(&mut s, "edit.undo", json!({}));
    refused(&mut s, "font.master.remove", json!({"masters": ["m01", "m02"]}), "at least one master");
}

#[test]
fn reorder_moves_a_master_and_undoes() {
    let mut s = two_masters();
    let r = run(&mut s, "font.master.reorder", json!({"master": "Bold", "toIndex": 0}));
    assert_eq!(r, json!({"master": "m02", "masters": ["m02"], "index": 0, "indices": [0]}));
    assert_eq!(master_ids(&s), vec!["m02".to_string(), "m01".into()]);
    refused(&mut s, "font.master.reorder", json!({"master": "Bold", "toIndex": 2}), "toIndex");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(master_ids(&s), vec!["m01".to_string(), "m02".into()]);
}

#[test]
fn set_icon_takes_a_built_in_icon_or_a_glyph() {
    let mut s = two_masters();
    assert_eq!(MASTER_ICONS.len(), 25);
    run(&mut s, "font.master.setIcon", json!({"masters": ["m01", "m02"], "icon": "Bold_Condensed"}));
    assert!(font(&s).masters.iter().all(|m| m.icon.as_deref() == Some("Bold_Condensed")));
    run(&mut s, "font.master.setIcon", json!({"masters": ["m02"], "glyph": "n"}));
    assert_eq!(font(&s).masters[1].icon.as_deref(), Some("n"));
    refused(&mut s, "font.master.setIcon", json!({"masters": ["m02"], "icon": "Heavy"}), "not a built-in master icon");
    refused(&mut s, "font.master.setIcon", json!({"masters": ["m02"], "glyph": "nosuch"}), "no glyph `nosuch`");
    refused(&mut s, "font.master.setIcon", json!({"masters": ["m02"], "icon": "Bold", "glyph": "n"}), "not both");
}
