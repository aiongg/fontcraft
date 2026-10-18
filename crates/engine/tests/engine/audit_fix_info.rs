//! The Font Info fixes of the audit (M12.63): each command clears, in the source, the check that
//! calls for it, as one undo step. Fixtures are built in code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_model::plist::{Array, Number, Value as Plist};
use fontcraft_model::{Axis, Instance};
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn int(n: i64) -> Plist {
    Plist::Number(Number::int(n))
}

fn ints(v: &[i64]) -> Plist {
    let mut a = Array::default();
    a.items = v.iter().map(|n| int(*n)).collect();
    Plist::Array(a)
}

/// How many results `check` gives when `audit.run` runs it alone with `extra` parameters.
fn found(s: &mut Session, check: &str, extra: Value) -> usize {
    let mut p = json!({"checks": [check]});
    if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    let r = run(s, "audit.run", p);
    r["results"].as_array().unwrap().iter().filter(|x| x["check"] == check).count()
}

/// A document with one glyph reaching from -120 to 900.
fn document() -> Session {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(60.0, -120.0, 560.0, 900.0)}));
    s
}

fn instance(name: &str) -> Instance {
    Instance { name: name.into(), ..Instance::default() }
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-audit-fix-info-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn font_info_sync_vertical_metrics_clears_the_three_metric_checks() {
    let mut s = document();
    s.edit("setup", |font| {
        let m = &mut Arc::make_mut(&mut font.masters)[0];
        m.params.set("winAscent", int(100));
        m.params.set("hheaLineGap", int(90));
        Arc::make_mut(&mut font.instances).push(instance("Regular"));
        Arc::make_mut(&mut font.instances)[0].params.set("typoAscender", int(950));
        Ok(())
    })
    .unwrap();
    let checks = ["check.metrics.hheaMatchesTypo", "check.metrics.winAscentDescent", "check.metrics.lineGaps"];
    for c in checks {
        assert!(found(&mut s, c, json!({})) > 0, "{c} fires before the fix");
    }
    let before = s.font().unwrap().clone();
    let dry = run(&mut s, "font.info.syncVerticalMetrics", json!({"dryRun": true}));
    assert!(!dry["changed"].as_array().unwrap().is_empty());
    assert!(s.font().unwrap().same_as(&before), "a dry run changes nothing");

    let r = run(&mut s, "font.info.syncVerticalMetrics", json!({}));
    assert_eq!(r["changed"], dry["changed"], "the dry run lists the same edits");
    let font = s.font().unwrap();
    let master = &font.masters[0].params;
    let get = |k: &str| master.get(k).and_then(|v| v.as_num()).map(|n| n.as_f64());
    // The default master compiles to hhea 1000 / -200 with a line gap of 90 on top.
    assert_eq!(get("typoAscender"), Some(1090.0));
    assert_eq!(get("hheaAscender"), Some(1090.0));
    assert_eq!((get("typoDescender"), get("hheaDescender")), (Some(-200.0), Some(-200.0)));
    assert_eq!((get("typoLineGap"), get("hheaLineGap")), (Some(0.0), Some(0.0)));
    assert_eq!((get("winAscent"), get("winDescent")), (Some(900.0), Some(120.0)));
    assert!(font.instances[0].params.get("typoAscender").is_none(), "the instance no longer overrides the master");
    for c in checks {
        assert_eq!(found(&mut s, c, json!({})), 0, "{c} passes after the fix");
    }
    run(&mut s, "edit.undo", json!({}));
    assert!(s.font().unwrap().same_as(&before), "one undo step takes the whole fix back");
    assert!(s.execute("font.info.syncVerticalMetrics", &json!({"masters": ["nope"]})).is_err());
}

#[test]
fn font_info_sync_version_writes_the_revision_into_the_version_string() {
    let mut s = document();
    s.edit("setup", |font| {
        (font.version_major, font.version_minor) = (1, 2);
        Arc::make_mut(&mut font.info).properties.set("versionString", "Version 0.5; build 7", false);
        Ok(())
    })
    .unwrap();
    assert!(found(&mut s, "check.names.versionString", json!({})) > 0);
    let r = run(&mut s, "font.info.syncVersion", json!({}));
    assert_eq!(r["changed"][0]["new"], "Version 1.002; build 7");
    assert_eq!(s.font().unwrap().info.properties.value("versionString"), Some("Version 1.002; build 7"));
    assert_eq!(found(&mut s, "check.names.versionString", json!({})), 0);
    let again = run(&mut s, "font.info.syncVersion", json!({}));
    assert!(again["changed"].as_array().unwrap().is_empty(), "a second run has nothing to do");
}

#[test]
fn font_instance_sync_style_linking_follows_the_recommended_scheme() {
    let mut s = document();
    s.edit("setup", |font| {
        let list = Arc::make_mut(&mut font.instances);
        for name in ["Regular", "Bold", "Italic", "Bold Italic", "Medium", "Medium Italic"] {
            list.push(instance(name));
        }
        list[0].is_bold = true;
        Ok(())
    })
    .unwrap();
    run(&mut s, "font.instance.syncStyleLinking", json!({}));
    let font = s.font().unwrap();
    let got: Vec<(&str, bool, bool, Option<&str>)> =
        font.instances.iter().map(|i| (i.name.as_str(), i.is_bold, i.is_italic, i.link_style.as_deref())).collect();
    assert_eq!(
        got,
        [
            ("Regular", false, false, None),
            ("Bold", true, false, Some("Regular")),
            ("Italic", false, true, Some("Regular")),
            ("Bold Italic", true, true, Some("Regular")),
            ("Medium", false, false, None),
            ("Medium Italic", false, true, Some("Medium")),
        ]
    );
    for c in ["check.names.subfamilyMatchesStyleBits", "check.tables.fsSelection"] {
        assert_eq!(found(&mut s, c, json!({})), 0, "{c}");
    }
}

#[test]
fn font_info_sync_family_values_makes_every_font_agree() {
    let mut s = document();
    s.edit("setup", |font| {
        let list = Arc::make_mut(&mut font.instances);
        list.push(instance("Regular"));
        let mut bold = instance("Bold");
        bold.params.set("underlinePosition", int(-150));
        bold.params.set("typoAscender", int(900));
        list.push(bold);
        Ok(())
    })
    .unwrap();
    let checks = ["check.family.underlineConsistency", "check.family.verticalMetrics"];
    for c in checks {
        assert!(found(&mut s, c, json!({})) > 0, "{c} fires before the fix");
    }
    assert!(s.execute("font.info.syncFamilyValues", &json!({"values": ["kerning"]})).is_err());
    run(&mut s, "font.info.syncFamilyValues", json!({}));
    let font = s.font().unwrap();
    assert!(font.instances.iter().all(|i| i.params.is_empty()), "the instances' own values are gone");
    assert_eq!(font.info.params.get("underlinePosition").and_then(|v| v.as_num()).map(|n| n.as_f64()), Some(-100.0));
    for c in checks {
        assert_eq!(found(&mut s, c, json!({})), 0, "{c} passes after the fix");
    }
}

#[test]
fn font_info_clean_names_removes_stray_white_space() {
    let mut s = document();
    s.edit("setup", |font| {
        font.family_name = "My  Family ".into();
        let info = Arc::make_mut(&mut font.info);
        info.properties.set("designers", "  Jane   Doe \n", true);
        info.properties.set("trademarks", "   ", true);
        Ok(())
    })
    .unwrap();
    assert!(found(&mut s, "check.names.whitespace", json!({})) > 0);
    run(&mut s, "font.info.cleanNames", json!({}));
    let font = s.font().unwrap();
    assert_eq!(font.family_name, "My Family");
    assert_eq!(font.info.properties.value("designers"), Some("Jane   Doe"), "only menu names lose inner spaces");
    assert!(font.info.properties.get("trademarks").is_none(), "an empty name is deleted");
    for c in ["check.names.whitespace", "check.names.emptyRecords", "check.names.lineBreaks"] {
        assert_eq!(found(&mut s, c, json!({"profile": "strict"})), 0, "{c}");
    }
}

#[test]
fn font_instance_clean_up_sorts_by_weight_and_drops_repeats() {
    let mut s = document();
    s.edit("setup", |font| {
        Arc::make_mut(&mut font.axes).push(Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() });
        Arc::make_mut(&mut font.masters)[0].location = vec![400.0];
        let list = Arc::make_mut(&mut font.instances);
        for (name, w) in [("Bold", 700.0), ("Regular", 400.0), ("Light", 300.0), ("Regular", 400.0)] {
            list.push(Instance { location: vec![w], ..instance(name) });
        }
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "font.instance.cleanUp", json!({}));
    let names: Vec<&str> = s.font().unwrap().instances.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["Light", "Regular", "Bold"]);
    let fields: Vec<&str> = r["changed"].as_array().unwrap().iter().filter_map(|c| c["field"].as_str()).collect();
    assert_eq!(fields, ["removed", "order"]);
}

#[test]
fn font_info_sort_zones_orders_each_pair_and_clears_the_cff_check() {
    let dir = scratch("zones");
    let mut s = document();
    s.edit("setup", |font| {
        Arc::make_mut(&mut font.masters)[0].params.set("postscriptBlueValues", ints(&[-15, 0, 715, 700]));
        Ok(())
    })
    .unwrap();
    let export = |s: &mut Session, name: &str| {
        let path = dir.join(name);
        run(s, "file.exportStatic", json!({"destination": path, "outlines": "cff", "timestamp": 0}));
        path
    };
    let before = export(&mut s, "before.otf");
    assert!(found(&mut s, "check.hinting.blueValuesOrder", json!({"target": before})) > 0);
    run(&mut s, "font.info.sortZones", json!({}));
    assert_eq!(s.font().unwrap().masters[0].params.get("postscriptBlueValues"), Some(&ints(&[-15, 0, 700, 715])));
    let after = export(&mut s, "after.otf");
    assert_eq!(found(&mut s, "check.hinting.blueValuesOrder", json!({"target": after})), 0);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn font_info_clear_generated_fields_clears_the_source_check() {
    let mut s = document();
    s.edit("setup", |font| {
        let info = Arc::make_mut(&mut font.info);
        info.params.set("year", int(2026));
        info.properties.set("versionString", "Version 1.0", false);
        Arc::make_mut(&mut font.masters)[0].params.set("postscriptUniqueID", int(4_000_000));
        Ok(())
    })
    .unwrap();
    assert_eq!(found(&mut s, "check.sources.unnecessaryInfoFields", json!({})), 2);
    let r = run(&mut s, "font.info.clearGeneratedFields", json!({}));
    assert_eq!(r["changed"].as_array().unwrap().len(), 3);
    assert_eq!(found(&mut s, "check.sources.unnecessaryInfoFields", json!({})), 0);
}
