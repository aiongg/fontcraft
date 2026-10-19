//! The autohinter's read-only reports (M10.13): `autohint.ps.reportStems`,
//! `autohint.ps.reportZones` and `autohint.ps.listFdDicts`.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::plist::{Str, Value as Plist};
use fontcraft_engine::model::{Glyph, Layer, Node, NodeKind, Path, Shape};
use serde_json::{Value, json};

/// `l`: a stem 80 units wide from 0 to 700; `a.sc` and `b.sc` (added): the same, for override
/// sets; `open` (added): an open path only (nothing to analyse).
fn session() -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let n = |x, y| Node::new(x, y, NodeKind::Line);
        let stem = |closed: bool| Shape::Path(Path::new(closed, vec![n(100.0, 0.0), n(180.0, 0.0), n(180.0, 700.0), n(100.0, 700.0)]));
        font.glyph_mut("l")?.layer_mut("m01").unwrap().shapes.push(stem(true));
        for (name, closed) in [("a.sc", true), ("b.sc", true), ("open", false)] {
            let mut g = Glyph::new(name);
            let mut layer = Layer::new("m01", 300.0);
            layer.shapes.push(stem(closed));
            g.layers.push(layer);
            font.glyphs.push(g);
        }
        Ok(())
    })
    .unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn row<'a>(rows: &'a Value, key: &str, value: f64) -> Option<&'a Value> {
    rows.as_array()?.iter().find(|r| r[key].as_f64() == Some(value))
}

#[test]
fn autohint_ps_report_stems_finds_the_stem_width_without_hinting() {
    let mut s = session();
    let before = s.font().unwrap().clone();
    let r = run(&mut s, "autohint.ps.reportStems", json!({"glyphs": ["l", "open"]}));
    assert_eq!(r["master"], "m01");
    let v = row(&r["vertical"], "width", 80.0).unwrap_or_else(|| panic!("{r}"));
    assert_eq!((&v["count"], &v["glyphs"]), (&json!(1), &json!(["l"])));
    assert!(r["text"].as_str().unwrap().contains("Vertical Stem List"));
    assert_eq!(*s.font().unwrap(), before, "a report changes nothing");

    let r = run(&mut s, "autohint.ps.reportStems", json!({"glyphs": ["l"], "excludeGlyphs": ["l"]}));
    assert_eq!((&r["horizontal"], &r["vertical"]), (&json!([]), &json!([])));
    for bad in [json!({"master": "nope"}), json!({"glyphs": ["nope"]}), json!({"includeCurved": 3})] {
        assert!(s.execute("autohint.ps.reportStems", &bad).is_err(), "{bad}");
    }
}

#[test]
fn autohint_ps_report_zones_lists_the_glyph_extent() {
    let mut s = session();
    let r = run(&mut s, "autohint.ps.reportZones", json!({"glyphs": ["l"]}));
    let top = row(&r["top"], "height", 700.0).unwrap_or_else(|| panic!("{r}"));
    assert!(top["glyphs"].as_array().unwrap().contains(&json!("l")));
    assert!(row(&r["bottom"], "height", 0.0).is_some(), "{r}");
}

#[test]
fn a_gasp_parameter_that_cannot_be_read_does_not_stop_the_reports() {
    let mut s = session();
    s.edit("gasp", |font| {
        Arc::make_mut(&mut font.info).params.set("gasp Table", Plist::String(Str::from("not a table".to_string())));
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "autohint.ps.reportZones", json!({"glyphs": ["l"]}));
    assert!(row(&r["top"], "height", 700.0).is_some(), "{r}");
    run(&mut s, "autohint.ps.reportStems", json!({"glyphs": ["l"]}));
    run(&mut s, "autohint.ps.listFdDicts", json!({}));
}

#[test]
fn autohint_ps_list_fd_dicts_assigns_glyphs_to_override_sets() {
    let mut s = session();
    let r = run(&mut s, "autohint.ps.listFdDicts", json!({}));
    let dicts = r["dicts"].as_array().unwrap();
    assert_eq!(dicts.len(), 1, "without override sets there is the font's set only");
    assert!(dicts[0]["glyphs"].as_array().unwrap().contains(&json!("a.sc")));

    let text = "begin FDDict SC\n BaselineYCoord 0\n BaselineOvershoot -10\n CapHeight 600\n CapOvershoot 10\n DominantH [60]\n DominantV [70]\nend FDDict SC\nbegin GlyphSet SC\n \\.sc$\nend GlyphSet SC\n";
    let r = run(&mut s, "autohint.ps.listFdDicts", json!({"fontinfo": text}));
    let dicts = r["dicts"].as_array().unwrap();
    assert_eq!(dicts.len(), 2);
    assert_eq!(dicts[0]["name"], "No Alignment Zones");
    assert!(!dicts[0]["glyphs"].as_array().unwrap().contains(&json!("a.sc")));
    assert_eq!(dicts[1]["name"], "SC");
    assert_eq!(dicts[1]["glyphs"], json!(["a.sc", "b.sc"]));
    assert_eq!(dicts[1]["glyphCount"], 2);
    assert_eq!(dicts[1]["blueValues"], json!([[-10.0, 0.0], [600.0, 610.0]]));
    assert_eq!((&dicts[1]["dominantH"], &dicts[1]["dominantV"]), (&json!([60.0]), &json!([70.0])));
    assert!(r.get("final").is_none());

    let err = s.execute("autohint.ps.listFdDicts", &json!({"fontinfo": "begin FDDict SC\n Colour 5\nend FDDict SC\n"})).unwrap_err();
    assert!(err.to_string().contains("line 2"), "{err}");
}

/// More standard stems than a CFF font keeps are cut to the first 12, and the reports say so.
#[test]
fn autohint_ps_list_fd_dicts_warns_when_the_stems_are_cut() {
    use fontcraft_engine::model::StemDef;
    let mut s = session();
    let r = run(&mut s, "autohint.ps.listFdDicts", json!({}));
    assert_eq!(r["warnings"], json!([]), "{r}");
    s.edit("stems", |font| {
        font.stems = Arc::new((0..13).map(|i| StemDef { name: format!("v{i}"), horizontal: false, ..StemDef::default() }).collect());
        Arc::make_mut(&mut font.masters).first_mut().unwrap().stem_values = (0..13).map(|i| 50.0 + f64::from(i)).collect();
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "autohint.ps.listFdDicts", json!({}));
    let warnings = r["warnings"].as_array().unwrap();
    assert!(warnings.len() == 1 && warnings.first().and_then(Value::as_str).unwrap().contains("13 vertical stems"), "{r}");
    let r = run(&mut s, "autohint.ps.reportStems", json!({"glyphs": ["l"]}));
    assert_eq!(r["warnings"].as_array().map(Vec::len), Some(1), "{r}");
}
