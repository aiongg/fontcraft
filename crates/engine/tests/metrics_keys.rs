//! The metrics-key commands: evaluating a key, reading a layer's metrics, listing the keys out of
//! sync, and Update Metrics (one key, one layer, every master, several glyphs).
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_model::plist::Value as Plist;
use fontcraft_model::{Anchor, Font, Param, Path, Shape};
use serde_json::{Value, json};

/// Masters `m01` Regular, `m02` Bold and `m03` Linked (Link Metrics With First Master).
/// n: LSB 50, RSB 50, width 500 (LSB 60, RSB 60 on Bold). o: LSB 40, RSB 50, width 500, top 450,
/// an anchor at x 245. a: LSB 10.
fn font() -> Font {
    let mut f = Font::new("Keys");
    let mut bold = f.masters[0].clone();
    bold.id = "m02".into();
    bold.name = "Bold".into();
    let mut linked = f.masters[0].clone();
    linked.id = "m03".into();
    linked.name = "Linked".into();
    linked.params.0.push(Param::new("Link Metrics With First Master", Plist::Bool(true)));
    Arc::make_mut(&mut f.masters).extend([bold, linked]);
    let names: Vec<String> = f.glyphs.as_arcs().iter().map(|g| g.name.clone()).collect();
    for name in names {
        let mut g = f.glyph_mut(&name).unwrap();
        g.layers.retain(|l| l.id == "m01");
        for id in ["m02", "m03"] {
            let mut l = g.layers[0].clone();
            l.id = id.into();
            l.master = id.into();
            g.layers.push(l);
        }
    }
    let set = |f: &mut Font, name: &str, path: Path, bold: Option<Path>| {
        let mut g = f.glyph_mut(name).unwrap();
        for l in &mut g.layers {
            l.width = 500.0;
            l.shapes = vec![Shape::Path(if l.id == "m02" { bold.clone().unwrap_or_else(|| path.clone()) } else { path.clone() })];
        }
    };
    set(&mut f, "n", Path::rect(50.0, 0.0, 450.0, 500.0), Some(Path::rect(60.0, 0.0, 440.0, 500.0)));
    set(&mut f, "o", Path::rect(40.0, 0.0, 450.0, 450.0), None);
    set(&mut f, "a", Path::rect(10.0, 0.0, 400.0, 500.0), None);
    for l in &mut f.glyph_mut("o").unwrap().layers {
        l.anchors.push(Anchor::new("top", 245.0, 450.0));
    }
    f
}

fn session() -> Session {
    let mut s = Session::new();
    s.add_document(font(), None).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Stores a glyph-level key (`layer: None`) or a layer's own key.
fn set_key(s: &mut Session, glyph: &str, layer: Option<&str>, field: &str, text: &str) {
    s.edit("key", |f| {
        let mut g = f.glyph_mut(glyph)?;
        let keys = match layer {
            None => &mut g.info.metrics_keys,
            Some(id) => &mut g.layers.iter_mut().find(|l| l.id == id).unwrap().info.metrics_keys,
        };
        let slot = match field {
            "left" => &mut keys.left,
            "right" => &mut keys.right,
            "width" => &mut keys.width,
            "top" => &mut keys.top,
            _ => &mut keys.bottom,
        };
        *slot = Some(text.into());
        Ok(())
    })
    .unwrap();
}

fn get(s: &mut Session, glyph: &str, layer: &str) -> Value {
    run(s, "spacing.getMetrics", json!({"glyph": glyph, "layer": layer}))
}

fn font_arc(s: &Session) -> Arc<Font> {
    s.doc().unwrap().font.clone()
}

#[test]
fn spacing_evaluate_metrics_key_gives_the_value_and_names_the_byte_of_a_bad_key() {
    let mut s = session();
    let eval = |s: &mut Session, key: &str, field: &str| s.execute("spacing.evaluateMetricsKey", &json!({"key": key, "glyph": "o", "field": field}));
    let r = eval(&mut s, "=n+10", "left").unwrap();
    assert_eq!((r["value"].as_f64(), r["key"].as_str(), r["scope"].as_str()), (Some(60.0), Some("=n+10"), Some("glyph")));
    let r = eval(&mut s, "==|n", "right").unwrap();
    assert_eq!((r["value"].as_f64(), r["key"].as_str(), r["scope"].as_str()), (Some(50.0), Some("==|n"), Some("local")));
    let r = eval(&mut s, "n", "width").unwrap();
    assert_eq!((r["value"].as_f64(), r["key"].as_str()), (Some(500.0), Some("n")));
    let r = eval(&mut s, "64", "left").unwrap();
    assert_eq!((r["value"].as_f64(), r["key"].is_null()), (Some(64.0), true));
    let e = eval(&mut s, "=n**2", "left").unwrap_err().to_string();
    assert!(e.contains("`key`") && e.contains("byte"), "{e}");
    let e = eval(&mut s, "=n/0", "left").unwrap_err().to_string();
    assert!(e.contains("divides by zero"), "{e}");
    assert!(eval(&mut s, &format!("={}", "n".repeat(1 << 20)), "left").is_err());
    assert!(eval(&mut s, "=n", "sideways").is_err());
}

#[test]
fn spacing_get_metrics_reports_metrics_keys_and_whether_they_are_in_sync() {
    let mut s = session();
    set_key(&mut s, "o", None, "left", "=n");
    set_key(&mut s, "o", Some("m02"), "right", "=n+5");
    let r = get(&mut s, "o", "m01");
    assert_eq!((r["left"].as_f64(), r["right"].as_f64(), r["width"].as_f64()), (Some(40.0), Some(50.0), Some(500.0)));
    assert_eq!(r["keys"]["left"], json!({"key": "=n", "scope": "glyph", "value": 50.0, "inSync": false, "error": null}));
    assert!(r["keys"]["width"].is_null());
    assert!(r["linkedTo"].is_null());
    let r = get(&mut s, "o", "m02");
    assert_eq!(r["keys"]["right"]["key"], "==n+5");
    assert_eq!(r["keys"]["right"]["scope"], "local");
    assert_eq!(r["keys"]["right"]["value"].as_f64(), Some(65.0));
    assert_eq!(get(&mut s, "o", "m03")["linkedTo"], "m01");
}

#[test]
fn spacing_list_out_of_sync_metrics_follows_edits_to_the_referenced_glyph() {
    let mut s = session();
    set_key(&mut s, "o", None, "left", "=n");
    set_key(&mut s, "a", None, "right", "=_gone");
    let list = |s: &mut Session, p: Value| run(s, "spacing.listOutOfSyncMetrics", p);
    let all = list(&mut s, json!({}));
    // o's left on Regular and Bold, and a's unlinked key on both; the linked master is left out.
    assert_eq!(all["count"], 4, "{all}");
    assert!(all["findings"].as_array().unwrap().iter().all(|f| f["master"] != "m03"));
    let regular = list(&mut s, json!({"glyphs": ["o"], "masters": ["m01"]}));
    assert_eq!(
        regular["findings"],
        json!([{"glyph": "o", "layer": "m01", "master": "m01", "field": "left", "key": "=n", "scope": "glyph", "problem": "outOfSync", "value": 50.0, "actual": 40.0}])
    );
    let unlinked = list(&mut s, json!({"glyphs": ["a"], "masters": ["m02"]}));
    assert_eq!((unlinked["findings"][0]["problem"].as_str(), unlinked["findings"][0]["missing"].as_str()), (Some("unlinked"), Some("_gone")));
    // n's outline moves to LSB 40 on Regular: o is in sync there, still out of sync on Bold.
    s.edit("move n", |f| {
        let mut g = f.glyph_mut("n")?;
        g.layers[0].shapes = vec![Shape::Path(Path::rect(40.0, 0.0, 440.0, 500.0))];
        Ok(())
    })
    .unwrap();
    let after = list(&mut s, json!({"glyphs": ["o"]}));
    assert_eq!(after["count"], 1, "{after}");
    assert_eq!(after["findings"][0]["layer"], "m02");
    assert!(s.execute("spacing.listOutOfSyncMetrics", &json!({"masters": ["nope"]})).is_err());
}

#[test]
fn spacing_update_metrics_key_moves_the_layer_and_undo_gives_back_the_font() {
    let mut s = session();
    set_key(&mut s, "o", None, "left", "=n");
    set_key(&mut s, "o", None, "top", "=n");
    let before = font_arc(&s);
    let r = run(&mut s, "spacing.updateMetricsKey", json!({"glyph": "o", "field": "left"}));
    assert_eq!((r["changed"].as_bool(), r["previous"].as_f64(), r["value"].as_f64()), (Some(true), Some(40.0), Some(50.0)));
    let m = get(&mut s, "o", "m01");
    assert_eq!((m["left"].as_f64(), m["right"].as_f64(), m["width"].as_f64()), (Some(50.0), Some(50.0), Some(510.0)));
    assert_eq!(s.font().unwrap().glyph("o").unwrap().layers[0].anchors[0].x, 255.0);
    assert_eq!(get(&mut s, "o", "m02")["left"].as_f64(), Some(40.0), "only the addressed layer changes");
    run(&mut s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&font_arc(&s), &before));

    // The top sidebearing moves the vertical origin and keeps the bottom one.
    let bottom = get(&mut s, "o", "m01")["bottom"].as_f64();
    run(&mut s, "spacing.updateMetricsKey", json!({"glyph": "o", "field": "top"}));
    let m = get(&mut s, "o", "m01");
    assert_eq!((m["top"].as_f64(), m["bottom"].as_f64()), (Some(300.0), bottom));

    // In sync already: nothing changes and no undo step is made.
    let now = font_arc(&s);
    let r = run(&mut s, "spacing.updateMetricsKey", json!({"glyph": "o", "field": "top"}));
    assert_eq!(r["changed"], false);
    assert!(Arc::ptr_eq(&font_arc(&s), &now));

    // Refused, with nothing changed: no key on the field, a linked master.
    assert!(s.execute("spacing.updateMetricsKey", &json!({"glyph": "o", "field": "width"})).is_err());
    let e = s.execute("spacing.updateMetricsKey", &json!({"glyph": "o", "layer": "m03", "field": "left"})).unwrap_err().to_string();
    assert!(e.contains("Regular"), "{e}");
    assert!(Arc::ptr_eq(&font_arc(&s), &now));
}

/// `fontView.batch.updateMetrics`: Update Metrics on several selected glyphs is one undo step,
/// and a key that reads a glyph updated in the same run gets its new value.
#[test]
fn glyph_update_metrics_and_font_view_batch_update_metrics_settle_a_chain_in_one_step() {
    let mut s = session();
    set_key(&mut s, "a", None, "left", "=o");
    set_key(&mut s, "o", None, "left", "=n");
    set_key(&mut s, "o", None, "width", "=n");
    let before = font_arc(&s);
    let r = run(&mut s, "glyph.updateMetrics", json!({"glyphs": ["a", "o"], "layer": "m01"}));
    assert_eq!(r["glyphs"], json!(["a", "o"]), "{r}");
    assert_eq!(get(&mut s, "o", "m01")["left"].as_f64(), Some(50.0));
    assert_eq!(get(&mut s, "o", "m01")["width"].as_f64(), Some(500.0), "the width key runs after the left sidebearing it follows");
    assert_eq!(get(&mut s, "a", "m01")["left"].as_f64(), Some(50.0), "a reads o after o was updated");
    assert_eq!(get(&mut s, "o", "m02")["left"].as_f64(), Some(40.0));
    run(&mut s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&font_arc(&s), &before));

    // A key that cannot be evaluated is listed; the others are still updated.
    set_key(&mut s, "a", None, "right", "=_gone");
    let r = run(&mut s, "glyph.updateMetrics", json!({"glyphs": ["a", "o"]}));
    assert!(r["skipped"].as_array().unwrap().iter().any(|k| k["glyph"] == "a" && k["field"] == "right"), "{r}");
    assert_eq!(get(&mut s, "a", "m01")["left"].as_f64(), Some(50.0));

    // A linked master is refused and nothing changes.
    let now = font_arc(&s);
    assert!(s.execute("glyph.updateMetrics", &json!({"glyphs": ["o"], "layer": "m03"})).is_err());
    assert!(Arc::ptr_eq(&font_arc(&s), &now));
}

#[test]
fn glyph_update_metrics_for_all_masters_clears_every_out_of_sync_key() {
    let mut s = session();
    set_key(&mut s, "o", None, "left", "=n");
    set_key(&mut s, "o", Some("m02"), "right", "=n+5");
    let before = font_arc(&s);
    let r = run(&mut s, "glyph.updateMetricsForAllMasters", json!({"glyphs": ["o"]}));
    assert_eq!(r["glyphs"], json!(["o"]));
    assert_eq!(get(&mut s, "o", "m01")["left"].as_f64(), Some(50.0));
    let bold = get(&mut s, "o", "m02");
    assert_eq!((bold["left"].as_f64(), bold["right"].as_f64()), (Some(60.0), Some(65.0)), "the layer's own key wins on Bold");
    assert_eq!(get(&mut s, "o", "m03")["left"].as_f64(), Some(40.0), "the linked master is left as it is");
    assert_eq!(run(&mut s, "spacing.listOutOfSyncMetrics", json!({}))["count"], 0);
    run(&mut s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&font_arc(&s), &before));
}

fn read_tree(dir: &std::path::Path, root: &std::path::Path, out: &mut fontcraft_io_glyphs::Files) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            read_tree(&p, root, out);
        } else {
            out.insert(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"), std::fs::read(&p).unwrap());
        }
    }
}

/// `docs/plans/M5.md` § 5: Update Metrics on every glyph and master of Inter Roman.
/// `cargo test --release -p fontcraft-engine --test metrics_keys -- --ignored --nocapture`.
#[test]
#[ignore = "speed measure"]
fn update_metrics_on_inter() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/inter/src/Inter-Roman.glyphspackage");
    if !dir.is_dir() {
        return;
    }
    let mut files = fontcraft_io_glyphs::Files::new();
    read_tree(&dir, &dir, &mut files);
    let font = fontcraft_io_glyphs::read_package(&files).unwrap();
    let (glyphs, masters) = (font.glyphs.len(), font.masters.len());
    let mut s = Session::new();
    s.add_document(font, None).unwrap();
    for pass in ["first", "second"] {
        let t = std::time::Instant::now();
        let out = s.execute("glyph.updateMetricsForAllMasters", &json!({})).unwrap();
        let took = t.elapsed();
        let updated = out["updated"].as_array().map_or(0, Vec::len);
        let skipped = out["skipped"].as_array().map_or(0, Vec::len);
        println!("{pass} run: {glyphs} glyphs, {masters} masters, {updated} updated, {skipped} skipped, {took:?}");
    }
}
