//! Axis commands (M9.15): list, add, edit, remove and reorder axes, the predefined axis names,
//! and the axis locations of masters and instances, on a two-axis, three-master font built in
//! code with an intermediate layer, an alternate layer and every axis parameter.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::sync::Arc;

use fontcraft_engine::{EngineError, Session};
use fontcraft_interp::params::{
    AXIS_MAPPINGS, AxisLocationEntry, VIRTUAL_MASTER, VirtualMasterEntry, axis_location, axis_mappings, set_axis_location, set_axis_mappings,
    set_virtual_masters, virtual_masters,
};
use fontcraft_interp::{Designspace, GlyphSources};
use fontcraft_model::{Axis, AxisRule, Font, Instance, Layer, Master};
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

fn tags(s: &Session) -> Vec<String> {
    font(s).axes.iter().map(|a| a.tag.clone()).collect()
}

fn locations(s: &Session) -> Vec<Vec<f64>> {
    font(s).masters.iter().map(|m| m.location.clone()).collect()
}

fn layer<'f>(font: &'f Font, glyph: &str, id: &str) -> &'f Layer {
    font.glyph(glyph).unwrap().layers.iter().find(|l| l.id == id).unwrap()
}

fn warnings(out: &Value) -> Vec<String> {
    out["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect()
}

/// Axes Weight (`wght`) and Width (`wdth`). Masters Regular (`m01`, 400/100), Bold (`m02`,
/// 700/100) and Narrow (`m03`, 400/75); instance Medium at 550/100. Glyph `a` has an
/// intermediate layer `brace` under Bold at 550/90 and an alternate layer `alt` under Regular for
/// wght ≥ 600 and wdth ≤ 90. Regular has an Axis Location for both axes; the font has Axis
/// Mappings for both axes and a Virtual Master at 700/75.
fn two_axes() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        *Arc::make_mut(&mut font.axes) = vec![
            Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() },
            Axis { name: "Width".into(), tag: "wdth".into(), ..Default::default() },
        ];
        let masters = Arc::make_mut(&mut font.masters);
        let regular = masters.get_mut(0).unwrap();
        regular.location = vec![400.0, 100.0];
        let entries = [AxisLocationEntry { axis: "Weight".into(), location: 400.0 }, AxisLocationEntry { axis: "Width".into(), location: 100.0 }];
        regular.params = set_axis_location(&regular.params, &entries).unwrap();
        let metric_values = regular.metric_values.clone();
        for (id, name, location) in [("m02", "Bold", [700.0, 100.0]), ("m03", "Narrow", [400.0, 75.0])] {
            masters.push(Master {
                id: id.into(),
                name: name.into(),
                location: location.to_vec(),
                metric_values: metric_values.clone(),
                ..Default::default()
            });
        }
        Arc::make_mut(&mut font.instances).push(Instance { name: "Medium".into(), location: vec![550.0, 100.0], ..Default::default() });
        let info = Arc::make_mut(&mut font.info);
        info.params = set_axis_mappings(&info.params, "wght", &[(300.0, 400.0), (700.0, 700.0)]).unwrap();
        info.params = set_axis_mappings(&info.params, "wdth", &[(75.0, 75.0), (100.0, 100.0)]).unwrap();
        let virtual_master = VirtualMasterEntry { coordinates: vec![("Weight".into(), 700.0), ("Width".into(), 75.0)] };
        info.params = set_virtual_masters(&info.params, &[virtual_master]).unwrap();
        for i in 0..font.glyphs.len() {
            let mut g = font.glyphs.get_mut_at(i).unwrap();
            let width = g.layers[0].width;
            g.layers.push(Layer::new("m02", width));
            g.layers.push(Layer::new("m03", width));
            if g.name == "a" {
                let mut brace = Layer::new("m02", width);
                brace.id = "brace".into();
                brace.info.attrs.coordinates = vec![Some(550.0), Some(90.0)];
                g.layers.push(brace);
                let mut alt = Layer::new("m01", width);
                alt.id = "alt".into();
                alt.info.attrs.axis_rules = vec![AxisRule { min: Some(600.0), max: None }, AxisRule { min: None, max: Some(90.0) }];
                g.layers.push(alt);
            }
        }
        Ok(())
    })
    .unwrap();
    s
}

/// Where every source of glyph `a` sits, by layer id, each location by axis tag, as the
/// interpolation reads it.
fn source_locations(s: &Session) -> BTreeMap<String, BTreeMap<String, f64>> {
    let font = font(s);
    let ds = Designspace::new(font).unwrap();
    let sources = GlyphSources::of(font, &ds, "a").unwrap();
    let tags: Vec<String> = ds.axes().iter().map(|a| a.tag.trim_end().to_string()).collect();
    sources
        .main
        .iter()
        .chain(sources.alternates.iter().flat_map(|a| a.sources.iter()))
        .map(|src| (src.layer_id.clone(), tags.iter().cloned().zip(src.location.0.iter().copied()).collect()))
        .collect()
}

#[test]
fn font_axes_list_and_list_predefined() {
    let mut s = two_axes();
    let out = run(&mut s, "font.axes.list", json!({}));
    assert_eq!(
        out["axes"],
        json!([{"index": 0, "name": "Weight", "tag": "wght", "hidden": false}, {"index": 1, "name": "Width", "tag": "wdth", "hidden": false}])
    );
    let out = run(&mut s, "font.axes.listPredefined", json!({}));
    let axes = out["axes"].as_array().unwrap();
    for (name, tag) in [("Weight", "wght"), ("Width", "wdth"), ("Italic", "ital"), ("Slant", "slnt"), ("Optical Size", "opsz")] {
        assert!(axes.contains(&json!({"name": name, "tag": tag, "registered": true})), "{name} is predefined as a registered axis");
    }
    assert!(axes.iter().any(|a| a["registered"] == json!(false)), "the proposed axes follow the registered ones");
}

#[test]
fn font_axes_add_names_tags_and_new_coordinates() {
    let mut s = Session::with_document();
    s.edit("instance", |font| {
        Arc::make_mut(&mut font.instances).push(Instance { name: "Regular".into(), ..Default::default() });
        Ok(())
    })
    .unwrap();
    let out = run(&mut s, "font.axes.add", json!({}));
    assert_eq!(out["axis"], json!({"index": 0, "name": "Weight", "tag": "wght", "hidden": false}), "the first axis is Weight");
    assert_eq!(locations(&s), [vec![0.0]], "the master gets 0 on the new axis");
    assert_eq!(font(&s).instances[0].location, [0.0]);

    let out = run(&mut s, "font.axes.add", json!({}));
    assert_eq!(out["axis"]["name"], "Axis 2");
    assert_eq!(out["axis"]["tag"], "AXSA", "a private tag of four capitals");
    assert!(warnings(&out).is_empty(), "{out}");

    let out = run(&mut s, "font.axes.add", json!({"name": "Width", "hidden": true}));
    assert_eq!(out["axis"], json!({"index": 2, "name": "Width", "tag": "wdth", "hidden": true}), "a predefined name sets its tag");
    let out = run(&mut s, "font.axes.add", json!({"tag": "opsz"}));
    assert_eq!(out["axis"]["name"], "Optical Size", "a predefined tag names the axis");
    let out = run(&mut s, "font.axes.add", json!({"name": "Swash", "tag": "swl"}));
    assert_eq!(warnings(&out).len(), 2, "a short, lowercase private tag: {out}");
    assert_eq!(locations(&s), [vec![0.0; 5]]);

    refused(&mut s, "font.axes.add", json!({"tag": "wght"}), "tag `wght`");
    refused(&mut s, "font.axes.add", json!({"name": "Weight", "tag": "WGHT"}), "named `Weight`");
    refused(&mut s, "font.axes.add", json!({"tag": "swl "}), "tag `swl `");
    refused(&mut s, "font.axes.add", json!({"tag": "toolong"}), "four printable");
    refused(&mut s, "font.axes.add", json!({"name": ""}), "empty");
}

#[test]
fn font_axes_add_gives_virtual_masters_a_coordinate_and_keeps_special_layers() {
    let mut s = two_axes();
    let before = (layer(font(&s), "a", "brace").clone(), layer(font(&s), "a", "alt").clone());
    run(&mut s, "font.axes.add", json!({"name": "Grade"}));
    assert_eq!(tags(&s), ["wght", "wdth", "GRAD"]);
    assert_eq!(locations(&s), [vec![400.0, 100.0, 0.0], vec![700.0, 100.0, 0.0], vec![400.0, 75.0, 0.0]]);
    assert_eq!(font(&s).instances[0].location, [550.0, 100.0, 0.0]);
    let vm = virtual_masters(font(&s)).unwrap();
    assert_eq!(vm[0].coordinates, [("Weight".to_string(), 700.0), ("Width".to_string(), 75.0), ("Grade".to_string(), 0.0)]);
    assert_eq!((layer(font(&s), "a", "brace"), layer(font(&s), "a", "alt")), (&before.0, &before.1), "special layers inherit or stay open");
}

#[test]
fn font_axes_set_predefined_name_sets_tag_and_parameters_follow() {
    let mut s = two_axes();
    let out = run(&mut s, "font.axes.set", json!({"axis": "wdth", "name": "Optical Size"}));
    assert_eq!(out["axis"], json!({"index": 1, "name": "Optical Size", "tag": "opsz", "hidden": false}));
    assert_eq!(tags(&s), ["wght", "opsz"]);
    let mappings: Vec<String> = axis_mappings(font(&s)).unwrap().into_iter().map(|(t, _)| t).collect();
    assert_eq!(mappings, ["wght", "opsz"], "the mapping follows the tag");
    let entries = axis_location(&font(&s).masters[0].params).unwrap().unwrap();
    assert_eq!(entries[1], AxisLocationEntry { axis: "Optical Size".into(), location: 100.0 }, "the Axis Location follows the name");
    assert_eq!(virtual_masters(font(&s)).unwrap()[0].coordinates[1].0, "Optical Size");

    run(&mut s, "font.axes.set", json!({"axis": 0, "hidden": true}));
    assert!(font(&s).axes[0].hidden);
    let out = run(&mut s, "font.axes.set", json!({"axis": "Optical Size", "name": "Serif"}));
    assert_eq!(out["axis"]["tag"], "opsz", "a free-form name leaves the tag");
    let out = run(&mut s, "font.axes.set", json!({"axis": 1, "tag": "SERF"}));
    assert!(warnings(&out).is_empty(), "{out}");

    refused(&mut s, "font.axes.set", json!({"axis": "wght", "tag": "SERF"}), "tag `SERF`");
    refused(&mut s, "font.axes.set", json!({"axis": "wght", "name": "Serif"}), "named `Serif`");
    refused(&mut s, "font.axes.set", json!({"axis": 2, "hidden": true}), "axis");
    refused(&mut s, "font.axes.set", json!({"axis": "slnt", "hidden": true}), "no axis `slnt`");
    refused(&mut s, "font.axes.set", json!({"axis": "wght"}), "at least one");
}

#[test]
fn font_axes_remove_drops_the_coordinate_everywhere() {
    let mut s = two_axes();
    let out = run(&mut s, "font.axes.remove", json!({"axis": "wdth"}));
    assert_eq!(out["removed"]["tag"], "wdth");
    assert_eq!(tags(&s), ["wght"]);
    assert_eq!(locations(&s), [vec![400.0], vec![700.0], vec![400.0]]);
    assert_eq!(font(&s).instances[0].location, [550.0]);
    assert_eq!(layer(font(&s), "a", "brace").info.attrs.coordinates, [Some(550.0)]);
    assert_eq!(layer(font(&s), "a", "alt").info.attrs.axis_rules, [AxisRule { min: Some(600.0), max: None }]);
    let mappings: Vec<String> = axis_mappings(font(&s)).unwrap().into_iter().map(|(t, _)| t).collect();
    assert_eq!(mappings, ["wght"]);
    assert_eq!(axis_location(&font(&s).masters[0].params).unwrap().unwrap(), [AxisLocationEntry { axis: "Weight".into(), location: 400.0 }]);
    assert_eq!(virtual_masters(font(&s)).unwrap()[0].coordinates, [("Weight".to_string(), 700.0)]);
    assert!(warnings(&out).iter().any(|w| w.contains("`Regular`, `Narrow`")), "Regular and Narrow now coincide: {out}");

    let out = run(&mut s, "font.axes.remove", json!({"axis": 0}));
    assert!(font(&s).axes.is_empty());
    assert_eq!(layer(font(&s), "a", "alt").info.attrs.axis_rules, [AxisRule::default()], "the alternate layer stays `[]`");
    assert_eq!(layer(font(&s), "a", "brace").info.attrs.coordinates, [None], "the intermediate layer stays one");
    assert!(warnings(&out).iter().any(|w| w.contains("`a`")), "the brace layer sits at its master: {out}");
    let params = &font(&s).info.params;
    assert!(params.get(AXIS_MAPPINGS).is_none() && params.get(VIRTUAL_MASTER).is_none(), "emptied parameters are removed");
    assert_eq!(axis_location(&font(&s).masters[0].params).unwrap(), None);

    refused(&mut s, "font.axes.remove", json!({"axis": 0}), "no axes");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(tags(&s), ["wght"], "one undo step brings the axis back");
    assert_eq!(layer(font(&s), "a", "brace").info.attrs.coordinates, [Some(550.0)]);
}

#[test]
fn font_axes_reorder_permutes_every_list() {
    let mut s = two_axes();
    let out = run(&mut s, "font.axes.reorder", json!({"order": ["wdth", "wght"]}));
    assert_eq!(out["axes"], json!(["wdth", "wght"]));
    assert_eq!(tags(&s), ["wdth", "wght"]);
    assert_eq!(locations(&s), [vec![100.0, 400.0], vec![100.0, 700.0], vec![75.0, 400.0]]);
    assert_eq!(font(&s).instances[0].location, [100.0, 550.0]);
    assert_eq!(layer(font(&s), "a", "brace").info.attrs.coordinates, [Some(90.0), Some(550.0)]);
    assert_eq!(
        layer(font(&s), "a", "alt").info.attrs.axis_rules,
        [AxisRule { min: None, max: Some(90.0) }, AxisRule { min: Some(600.0), max: None }]
    );
    let mappings: Vec<String> = axis_mappings(font(&s)).unwrap().into_iter().map(|(t, _)| t).collect();
    assert_eq!(mappings, ["wdth", "wght"]);
    let entries: Vec<String> = axis_location(&font(&s).masters[0].params).unwrap().unwrap().into_iter().map(|e| e.axis).collect();
    assert_eq!(entries, ["Width", "Weight"]);

    refused(&mut s, "font.axes.reorder", json!({"order": ["wght"]}), "each of");
    refused(&mut s, "font.axes.reorder", json!({"order": ["wght", "wght"]}), "twice");
    refused(&mut s, "font.axes.reorder", json!({"order": ["wght", "slnt"]}), "no axis `slnt`");
    let before = Arc::clone(&s.doc().unwrap().font);
    run(&mut s, "font.axes.reorder", json!({"order": ["wdth", "wght"]}));
    assert!(Arc::ptr_eq(&before, &s.doc().unwrap().font), "the current order changes nothing");
}

#[test]
fn axis_edits_keep_every_source_location() {
    let mut s = two_axes();
    let before = source_locations(&s);
    assert!(before.contains_key("brace") && before.contains_key("alt"), "{before:?}");

    run(&mut s, "font.axes.reorder", json!({"order": ["wdth", "wght"]}));
    assert_eq!(source_locations(&s), before, "reordering moves no source");

    let out = run(&mut s, "font.axes.add", json!({}));
    assert_eq!(out["axis"]["tag"], "AXSA");
    let mut widened = before.clone();
    for location in widened.values_mut() {
        location.insert("AXSA".into(), 0.0);
    }
    assert_eq!(source_locations(&s), widened, "every source, the intermediate layer included, sits at 0 on the new axis");

    run(&mut s, "font.axes.remove", json!({"axis": "AXSA"}));
    assert_eq!(source_locations(&s), before, "removing the new axis restores every location");
}

#[test]
fn font_master_set_axis_location_keeps_the_other_axes() {
    let mut s = two_axes();
    let out = run(&mut s, "font.master.setAxisLocation", json!({"masters": ["Bold"], "location": {"wdth": 80}}));
    assert_eq!(out["masters"], json!(["m02"]));
    assert_eq!(font(&s).masters[1].location, [700.0, 80.0]);
    let out = run(&mut s, "font.master.setAxisLocation", json!({"masters": ["m03"], "location": {"wdth": 100}}));
    assert!(warnings(&out).iter().any(|w| w.contains("`Regular`, `Narrow`")), "{out}");

    s.edit("short location", |font| {
        Arc::make_mut(&mut font.masters)[0].location.clear();
        Ok(())
    })
    .unwrap();
    run(&mut s, "font.master.setAxisLocation", json!({"masters": [0], "location": {"wdth": 90}}));
    assert_eq!(font(&s).masters[0].location, [100.0, 90.0], "a short location is read with the defaults");

    refused(&mut s, "font.master.setAxisLocation", json!({"masters": ["Bold"], "location": {"slnt": 1}}), "no axis `slnt`");
    refused(&mut s, "font.master.setAxisLocation", json!({"masters": ["Bold"], "location": {}}), "names no axis");
    refused(&mut s, "font.master.setAxisLocation", json!({"masters": ["Bold"], "location": {"wght": "heavy"}}), "finite");
    refused(&mut s, "font.master.setAxisLocation", json!({"masters": ["Bold"], "location": {"wght": 1e9}}), "finite");
    refused(&mut s, "font.master.setAxisLocation", json!({"masters": ["Black"], "location": {"wght": 900}}), "Black");
}

#[test]
fn font_instance_set_axis_location_names_extrapolated_instances() {
    let mut s = two_axes();
    let out = run(&mut s, "font.instance.setAxisLocation", json!({"instances": ["Medium"], "location": {"wght": 500}}));
    assert_eq!(out, json!({"instances": [0], "warnings": []}));
    assert_eq!(font(&s).instances[0].location, [500.0, 100.0]);
    let out = run(&mut s, "font.instance.setAxisLocation", json!({"instances": [0], "location": {"wght": 1000, "wdth": 100}}));
    assert_eq!(font(&s).instances[0].location, [1000.0, 100.0]);
    let w = warnings(&out);
    assert!(w.len() == 1 && w[0].contains("wght") && w[0].contains("extrapolated"), "{out}");

    refused(&mut s, "font.instance.setAxisLocation", json!({"instances": ["Nope"], "location": {"wght": 400}}), "Nope");
    refused(&mut s, "font.instance.setAxisLocation", json!({"instances": [0]}), "`location` is required");
}
