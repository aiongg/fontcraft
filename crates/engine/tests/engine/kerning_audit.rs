//! Kerning audit and cross-master matching (M13.21b): `kerning.auditExceptions` finds class pairs
//! that make two glyphs collide and adds the exceptions that clear them; `kerning.matchAcrossMasters`
//! gives every master the same pairs without changing any glyph pair's value. One undo step each;
//! refusals change nothing.
//!
//! The corpus measure runs with
//! `cargo test --release -p fontcraft-engine --test engine kerning_audit:: -- --ignored --nocapture`.
//! `FONTCRAFT_CORPUS` names the corpus folder (default: the repository's `corpus/`).
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Font, KernGroups, Layer};
use fontcraft_spacing::kerning::pairs::stored_pairs;
use fontcraft_spacing::kerning::{Direction, KernIndex, Position, Side, facing};
use fontcraft_spacing::metrics_source;
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Runs a command that must fail and checks that the font is the same object as before.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let e = s.execute(id, &p).expect_err(id).to_string();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} {p}: a refused command changed the font");
    e
}

/// Runs a command and checks that one `edit.undo` gives back the very font it started from.
fn undoable(s: &mut Session, id: &str, p: Value) -> Value {
    let before = s.doc().unwrap().font.clone();
    let steps = s.doc().unwrap().history.undo.len();
    let out = run(s, id, p);
    let after = s.doc().unwrap().font.clone();
    assert!(!Arc::ptr_eq(&after, &before), "{id} changed nothing");
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "{id} is one undo step");
    run(s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id}: undo did not restore the font");
    run(s, "edit.redo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &after), "{id}: redo did not restore the result");
    out
}

fn draw(s: &mut Session, glyph: &str, paths: &[Value], width: f64) {
    for nodes in paths {
        run(s, "path.addPath", json!({"glyph": glyph, "nodes": nodes}));
    }
    run(s, "spacing.setWidth", json!({"glyphs": [glyph], "value": width}));
}

/// The left-to-right pair stored in `master` under these spellings.
fn kern(s: &Session, master: &str, left: &str, right: &str) -> Option<f64> {
    s.font().unwrap().kerning.ltr.get(master, left, right)
}

/// Adds a second master `m02`, then the glyphs with their (left, right) groups.
fn font_with(glyphs: &[(&str, Option<&str>, Option<&str>)]) -> Session {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let mut m = font.default_master()?.clone();
        m.id = "m02".into();
        m.name = "Bold".into();
        Arc::make_mut(&mut font.masters).push(m);
        for (name, left, right) in glyphs {
            if font.glyph(name).is_none() {
                font.add_glyph(name, Vec::new(), 500.0)?;
            }
            let mut g = font.glyph_mut(name)?;
            if g.layer("m02").is_none() {
                g.layers.push(Layer::new("m02", 500.0));
            }
            g.kern_groups = KernGroups { left: left.map(str::to_string), right: right.map(str::to_string), ..KernGroups::default() };
        }
        Ok(())
    })
    .unwrap();
    s
}

fn set(s: &mut Session, master: &str, pairs: &[(&str, &str, f64)]) {
    s.edit("kern", |font| {
        let k = Arc::make_mut(&mut font.kerning);
        for (a, b, v) in pairs {
            k.ltr.set(master, a, b, *v);
        }
        Ok(())
    })
    .unwrap();
}

/// The left-to-right value between every two of `glyphs` in each master, through groups.
fn effective(font: &Font, glyphs: &[&str]) -> Vec<f64> {
    let index = KernIndex::new(font);
    let mut out = Vec::new();
    for m in font.masters.iter() {
        for a in glyphs {
            for b in glyphs {
                out.push(index.lookup(Direction::Ltr, &m.id, a, b).unwrap().map_or(0.0, |x| x.value));
            }
        }
    }
    out
}

/// The sides of the left-to-right pairs `master` stores, in command spelling.
fn sides(font: &Font, master: &str) -> BTreeSet<(String, String)> {
    stored_pairs(font, Direction::Ltr, master).iter().map(|lp| (lp.left().command(), lp.right().command())).collect()
}

#[test]
fn kerning_audit_finds_and_fixes_a_class_pair_collision() {
    // A and V stand 50 + 50 apart without kerning, W stands 50 + 250 from A.
    let mut s = Session::with_document();
    draw(&mut s, "A", &[rect(50.0, 0.0, 450.0, 700.0)], 500.0);
    draw(&mut s, "V", &[rect(50.0, 0.0, 450.0, 700.0)], 500.0);
    draw(&mut s, "W", &[rect(250.0, 0.0, 450.0, 700.0)], 500.0);
    draw(&mut s, "Y", &[rect(50.0, 0.0, 450.0, 700.0)], 500.0);
    run(&mut s, "kerning.setRightGroup", json!({"glyphs": ["A"], "group": "A"}));
    run(&mut s, "kerning.setLeftGroup", json!({"glyphs": ["V", "W", "Y"], "group": "V"}));
    run(&mut s, "kerning.setPair", json!({"left": "@A", "right": "@V", "value": -200}));
    run(&mut s, "kerning.setPair", json!({"left": "A", "right": "Y", "value": -50}));

    // −200 overlaps A V by 100. A W ends 100 apart, clear of 20. A Y has its own pair and is not
    // measured.
    let out = run(&mut s, "kerning.auditExceptions", json!({"threshold": 20}));
    assert_eq!(out["measured"], json!(2), "{out}");
    let pairs = out["pairs"].as_array().unwrap();
    assert_eq!(pairs.len(), 1, "{out}");
    let hit = &pairs[0];
    assert_eq!((hit["left"].as_str(), hit["right"].as_str()), (Some("A"), Some("V")));
    assert_eq!((hit["classLeft"].as_str(), hit["classRight"].as_str()), (Some("@A"), Some("@V")));
    assert_eq!(hit["value"], json!(-200.0));
    assert!((hit["distance"].as_f64().unwrap() + 100.0).abs() < 0.01, "{out}");
    assert_eq!(hit["fix"], json!(-80.0), "100 apart without kerning, so −80 leaves 20");
    assert_eq!(kern(&s, "m01", "A", "V"), None, "without fix nothing is stored");

    undoable(&mut s, "kerning.auditExceptions", json!({"threshold": 20, "fix": true}));
    assert_eq!(kern(&s, "m01", "A", "V"), Some(-80.0));
    assert_eq!(run(&mut s, "kerning.getPair", json!({"left": "A", "right": "W"}))["value"], json!(-200.0));
    // The exception takes A V out of class kerning: nothing is left to report.
    let out = run(&mut s, "kerning.auditExceptions", json!({"threshold": 20}));
    assert_eq!((out["pairs"].clone(), out["measured"].clone()), (json!([]), json!(1)), "{out}");
    // A given group side stands for its members; only A W is under class kerning.
    let out = run(&mut s, "kerning.auditExceptions", json!({"pairs": [["A", "@V"]], "threshold": 200}));
    assert_eq!(out["measured"], json!(1), "{out}");
    assert_eq!(out["pairs"][0]["fix"], json!(-100.0), "{out}");

    refused(&mut s, "kerning.auditExceptions", json!({"threshold": 1e9}));
    refused(&mut s, "kerning.auditExceptions", json!({"direction": "vertical"}));
    refused(&mut s, "kerning.auditExceptions", json!({"pairs": []}));
    refused(&mut s, "kerning.auditExceptions", json!({"masters": ["nosuch"]}));
}

#[test]
fn kerning_match_gives_every_master_the_same_pairs_and_keeps_the_values() {
    let mut s = font_with(&[("A", None, Some("A")), ("Aacute", None, Some("A")), ("V", Some("V"), None), ("W", Some("V"), None)]);
    set(&mut s, "m01", &[("@MMK_L_A", "@MMK_R_V", -80.0), ("@MMK_L_A", "V", -50.0)]);
    set(&mut s, "m02", &[("@MMK_L_A", "@MMK_R_V", -100.0), ("A", "@MMK_R_V", -90.0), ("T", "o", -30.0)]);
    let glyphs = ["A", "Aacute", "T", "V", "W", "o"];
    let before = effective(s.font().unwrap(), &glyphs);

    let out = undoable(&mut s, "kerning.matchAcrossMasters", json!({}));
    let font = s.font().unwrap();
    assert_eq!(effective(font, &glyphs), before, "no glyph pair changes in any master: {out}");
    assert_eq!(sides(font, "m01"), sides(font, "m02"));
    assert_eq!(sides(font, "m01").len(), 5, "{out}");
    // m01 lacked A @V: under it A V had −50 and A W −80 there, so A @V takes A V's −50 and A W
    // becomes an exception in both masters.
    assert_eq!(kern(&s, "m01", "A", "@MMK_R_V"), Some(-50.0));
    assert_eq!((kern(&s, "m01", "A", "W"), kern(&s, "m02", "A", "W")), (Some(-80.0), Some(-90.0)));
    // m02 lacked @A V: Aacute V had −100 there through the group pair.
    assert_eq!(kern(&s, "m02", "@MMK_L_A", "V"), Some(-100.0));
    // m01 had no kerning for T o at all.
    assert_eq!(kern(&s, "m01", "T", "o"), Some(0.0));
    assert_eq!(out["masters"], json!({"m01": {"before": 2, "after": 5}, "m02": {"before": 3, "after": 5}}), "{out}");
    let kinds: Vec<&str> = out["pairs"].as_array().unwrap().iter().map(|p| p["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds.iter().filter(|k| **k == "exception").count(), 2, "{out}");

    // Matched kerning has nothing left to add, and the call is no undo step.
    let steps = s.doc().unwrap().history.undo.len();
    let out = run(&mut s, "kerning.matchAcrossMasters", json!({"mainMaster": "m02"}));
    assert_eq!(out["pairs"], json!([]), "{out}");
    assert_eq!(s.doc().unwrap().history.undo.len(), steps);

    assert!(refused(&mut s, "kerning.matchAcrossMasters", json!({"masters": ["m01"]})).contains("two masters"));
    refused(&mut s, "kerning.matchAcrossMasters", json!({"mainMaster": "nosuch"}));
    refused(&mut s, "kerning.matchAcrossMasters", json!({"masters": ["m01", "m02"], "mainMaster": 3}));
}

#[test]
fn kerning_audit_and_match_refuse_hostile_pair_counts() {
    // One group pair between groups of 2,001 and 2,000 glyphs stands for 4,002,000 glyph pairs.
    let mut s = font_with(&[]);
    s.edit("groups", |font| {
        for i in 0..2_001 {
            let name = format!("l{i}");
            font.add_glyph(&name, Vec::new(), 500.0)?;
            let mut g = font.glyph_mut(&name)?;
            g.kern_groups.right = Some("L".into());
        }
        for i in 0..2_000 {
            let name = format!("r{i}");
            font.add_glyph(&name, Vec::new(), 500.0)?;
            let mut g = font.glyph_mut(&name)?;
            g.kern_groups.left = Some("R".into());
        }
        Ok(())
    })
    .unwrap();
    set(&mut s, "m01", &[("@MMK_L_L", "@MMK_R_R", -10.0)]);
    set(&mut s, "m02", &[("@MMK_L_L", "@MMK_R_R", -20.0)]);
    assert!(refused(&mut s, "kerning.auditExceptions", json!({"masters": ["m01"]})).contains("more than"));
    assert!(refused(&mut s, "kerning.auditExceptions", json!({"pairs": [["@L", "@R"]], "fix": true})).contains("more than"));
    assert!(refused(&mut s, "kerning.matchAcrossMasters", json!({})).contains("more than"));
}

fn corpus() -> PathBuf {
    std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from)
}

/// Glyph ids a side stands for at `position`.
fn expand(index: &KernIndex, side: &Side, direction: Direction, position: Position) -> Vec<usize> {
    match side {
        Side::Glyph(n) => index.glyph_id(n).into_iter().collect(),
        Side::Group(g) => index.members(facing(direction, position), g).filter_map(|m| index.glyph_id(m)).collect(),
    }
}

#[test]
#[ignore = "corpus measure"]
fn kerning_match_keeps_every_value_of_inter() {
    let path = corpus().join("inter/src/Inter-Roman.glyphspackage");
    if !path.is_dir() {
        eprintln!("corpus not fetched: skipped");
        return;
    }
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    let before = s.font().unwrap().clone();
    let t = Instant::now();
    let out = s.execute("kerning.matchAcrossMasters", &json!({})).unwrap();
    let took = t.elapsed();
    let after = s.font().unwrap().clone();
    let kinds = |kind: &str| out["pairs"].as_array().unwrap().iter().filter(|p| p["kind"] == kind).count();
    println!("Inter Roman: {} pairs filled, {} exceptions, {took:?}; {}", kinds("filled"), kinds("exception"), out["masters"]);
    let own: Vec<&str> =
        before.masters.iter().filter(|m| metrics_source(&before, &m.id).is_none_or(|src| src.id == m.id)).map(|m| m.id.as_str()).collect();
    let (ib, ia) = (KernIndex::new(&before), KernIndex::new(&after));
    let mut changed = Vec::new();
    for direction in Direction::ALL {
        // Every glyph pair a stored pair of any master stands for.
        let mut covered: HashSet<(usize, usize)> = HashSet::new();
        for id in &own {
            for lp in stored_pairs(&before, direction, id) {
                let seconds = expand(&ib, &lp.right(), direction, Position::Second);
                for f in expand(&ib, &lp.left(), direction, Position::First) {
                    covered.extend(seconds.iter().map(|s| (f, *s)));
                }
            }
        }
        for id in &own {
            let (tb, ta) = (ib.table(direction, id).unwrap(), ia.table(direction, id).unwrap());
            for (f, s) in &covered {
                let (b, a) = (tb.lookup(*f, *s).map_or(0.0, |x| x.value), ta.lookup(*f, *s).map_or(0.0, |x| x.value));
                if a != b && changed.len() < 20 {
                    changed.push(format!("{} {id} glyphs {f} {s}: {b} → {a}", direction.name()));
                }
            }
        }
        let usable = |font: &Font, index: &KernIndex, id: &str| -> BTreeSet<(String, String)> {
            stored_pairs(font, direction, id)
                .iter()
                .filter(|lp| {
                    !expand(index, &lp.left(), direction, Position::First).is_empty()
                        && !expand(index, &lp.right(), direction, Position::Second).is_empty()
                })
                .map(|lp| (lp.left().command(), lp.right().command()))
                .collect()
        };
        let first = own.first().map(|id| usable(&after, &ia, *id)).unwrap_or_default();
        for id in &own {
            assert_eq!(usable(&after, &ia, *id), first, "{} pairs of master {id} differ from the first master's", direction.name());
        }
    }
    assert!(changed.is_empty(), "values changed: {changed:#?}");
}
