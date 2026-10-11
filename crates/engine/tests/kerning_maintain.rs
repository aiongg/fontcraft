//! Kerning maintenance: `kerning.cleanUp`, `kerning.compress` and `kerning.renameGroup`, with the
//! behaviours `kerning.obsoletePairs`, `kerning.compressRules`, `kerning.compressOneStepPerRun`
//! and `kerning.renameSidesIndependent`.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::{Font, KernGroups, Layer};
use fontcraft_spacing::kerning::{Direction, KernIndex};
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
    let out = run(s, id, p);
    let after = s.doc().unwrap().font.clone();
    assert!(!Arc::ptr_eq(&after, &before), "{id} changed nothing");
    run(s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id}: undo did not restore the font");
    run(s, "edit.redo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &after), "{id}: redo did not restore the result");
    out
}

/// Adds the glyphs with their (left, right) groups and a second master `m02`.
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

fn set(s: &mut Session, direction: Direction, master: &str, pairs: &[(&str, &str, f64)]) {
    s.edit("kern", |font| {
        let k = Arc::make_mut(&mut font.kerning);
        for (a, b, v) in pairs {
            direction.table_mut(k).set(master, a, b, *v);
        }
        Ok(())
    })
    .unwrap();
}

/// The stored pairs of one master, in stored order.
fn stored(font: &Font, direction: Direction, master: &str) -> Vec<(String, String, f64)> {
    direction
        .table(&font.kerning)
        .0
        .iter()
        .filter(|m| m.master == master)
        .flat_map(|m| &m.firsts)
        .flat_map(|f| f.seconds.iter().map(|(s, v)| (f.first.clone(), s.clone(), *v)))
        .collect()
}

fn owned(pairs: &[(&str, &str, f64)]) -> Vec<(String, String, f64)> {
    pairs.iter().map(|(a, b, v)| (a.to_string(), b.to_string(), *v)).collect()
}

fn kern(font: &Font, first: &str, second: &str) -> f64 {
    KernIndex::new(font).lookup(Direction::Ltr, "m01", first, second).unwrap().map_or(0.0, |a| a.value)
}

/// T and Tcaron share the right group T; m, n, ncaron and eng share the left group n.
fn handbook_font() -> Session {
    let mut s = font_with(&[
        ("T", None, Some("T")),
        ("Tcaron", None, Some("T")),
        ("m", Some("n"), None),
        ("n", Some("n"), None),
        ("ncaron", Some("n"), None),
        ("eng", Some("n"), None),
    ]);
    set(&mut s, Direction::Ltr, "m01", &[("T", "m", -40.0), ("T", "n", -40.0), ("T", "ncaron", -10.0)]);
    s
}

#[test]
fn compress_lifts_one_side_per_run_and_keeps_real_exceptions() {
    // kerning.compress, kerning.compressOneStepPerRun, kerning.compressRules
    let mut s = handbook_font();
    let original = s.doc().unwrap().font.clone();

    let first = undoable(&mut s, "kerning.compress", json!({}));
    assert_eq!(first["runs"], 1);
    assert_eq!(first["lifted"].as_array().unwrap().len(), 3);
    assert_eq!(first["lifted"][0]["from"], json!({"left": "T", "right": "m"}));
    assert_eq!(first["lifted"][0]["to"], json!({"left": "@T", "right": "m"}));
    let font = s.font().unwrap().clone();
    assert_eq!(stored(&font, Direction::Ltr, "m01"), owned(&[("@MMK_L_T", "m", -40.0), ("@MMK_L_T", "n", -40.0), ("@MMK_L_T", "ncaron", -10.0)]));

    let second = undoable(&mut s, "kerning.compress", json!({}));
    assert_eq!(second["lifted"][0]["to"], json!({"left": "@T", "right": "@n"}));
    assert_eq!(second["absorbed"], json!([{"run": 1, "left": "@T", "right": "n", "value": -40.0, "into": {"left": "@T", "right": "@n"}}]));
    assert_eq!(
        second["kept"],
        json!([{"left": "@T", "right": "ncaron", "value": -10.0, "reason": "differentValue", "groupPair": {"left": "@T", "right": "@n", "value": -40.0}}])
    );
    assert_eq!((second["pairsBefore"].clone(), second["pairsAfter"].clone()), (json!(3), json!(2)));
    let font = s.font().unwrap().clone();
    assert_eq!(stored(&font, Direction::Ltr, "m01"), owned(&[("@MMK_L_T", "@MMK_R_n", -40.0), ("@MMK_L_T", "ncaron", -10.0)]));

    // What applied to the kerned glyphs stays; the group pair now covers the other members.
    for (a, b) in [("T", "m"), ("T", "n"), ("T", "ncaron")] {
        assert_eq!(kern(&font, a, b), kern(&original, a, b), "{a} {b}");
    }
    assert_eq!(kern(&font, "Tcaron", "eng"), -40.0);
    assert_eq!(kern(&font, "Tcaron", "ncaron"), -10.0);

    // A run with nothing to do changes nothing and makes no undo step.
    let steps = s.doc().unwrap().history.undo.len();
    let before = s.doc().unwrap().font.clone();
    let third = run(&mut s, "kerning.compress", json!({}));
    assert_eq!(third["runs"], 0);
    assert_eq!(third["kept"].as_array().unwrap().len(), 1);
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before));
    assert_eq!(s.doc().unwrap().history.undo.len(), steps);
}

#[test]
fn compress_until_stable_is_one_undo_step_on_one_master() {
    let mut s = handbook_font();
    set(&mut s, Direction::Ltr, "m02", &[("T", "m", -30.0)]);
    let out = undoable(&mut s, "kerning.compress", json!({"untilStable": true}));
    assert_eq!(out["runs"], 2);
    let font = s.font().unwrap().clone();
    assert_eq!(stored(&font, Direction::Ltr, "m01"), owned(&[("@MMK_L_T", "@MMK_R_n", -40.0), ("@MMK_L_T", "ncaron", -10.0)]));
    assert_eq!(stored(&font, Direction::Ltr, "m02"), owned(&[("T", "m", -30.0)]));
    let e = refused(&mut s, "kerning.compress", json!({"direction": "sideways"}));
    assert!(e.contains("direction"), "{e}");
}

#[test]
fn compress_collisions_first_pair_lifts_and_the_rest_meet_it() {
    // A has no right group, so its pairs lift their right side: A x lifts to A @x first, A y
    // then meets it with another value and stays, A z has the same value and is absorbed.
    let mut s = font_with(&[("A", None, None), ("x", Some("x"), None), ("y", Some("x"), None), ("z", Some("x"), None)]);
    set(&mut s, Direction::Ltr, "m01", &[("A", "x", -10.0), ("A", "y", -20.0), ("A", "z", -10.0)]);
    let original = s.doc().unwrap().font.clone();
    let out = run(&mut s, "kerning.compress", json!({}));
    assert_eq!(out["lifted"].as_array().unwrap().len(), 1);
    assert_eq!(out["kept"][0]["left"], "A");
    assert_eq!(out["kept"][0]["right"], "y");
    assert_eq!(out["absorbed"][0]["right"], "z");
    let font = s.font().unwrap().clone();
    assert_eq!(stored(&font, Direction::Ltr, "m01"), owned(&[("A", "@MMK_R_x", -10.0), ("A", "y", -20.0)]));
    for b in ["x", "y", "z"] {
        assert_eq!(kern(&font, "A", b), kern(&original, "A", b), "A {b}");
    }

    // A glyph–group pair is not lifted over group–glyph pairs of its group's members: T @n
    // would lose to @T m for T m once lifted.
    let mut s = handbook_font();
    s.edit("kern", |font| {
        Arc::make_mut(&mut font.kerning).ltr = Default::default();
        Ok(())
    })
    .unwrap();
    set(&mut s, Direction::Ltr, "m01", &[("T", "@MMK_R_n", -30.0), ("@MMK_L_T", "m", -50.0)]);
    let original = s.doc().unwrap().font.clone();
    let out = run(&mut s, "kerning.compress", json!({"untilStable": true}));
    let font = s.font().unwrap().clone();
    assert_eq!(stored(&font, Direction::Ltr, "m01"), owned(&[("T", "@MMK_R_n", -30.0), ("@MMK_L_T", "@MMK_R_n", -50.0)]));
    assert_eq!(out["kept"][0]["reason"], "differentValue");
    for (a, b) in [("T", "m"), ("T", "n"), ("Tcaron", "m")] {
        assert_eq!(kern(&font, a, b), kern(&original, a, b), "{a} {b}");
    }
}

#[test]
fn clean_up_removes_pairs_of_removed_glyphs_and_empty_groups() {
    // kerning.cleanUp, kerning.obsoletePairs
    let mut s = font_with(&[("A", None, Some("A")), ("V", Some("V"), None), ("O", Some("O"), None)]);
    let pairs = [
        ("A", "V", -50.0),
        ("@MMK_L_A", "@MMK_R_O", -20.0),
        ("@MMK_L_Gone", "V", -10.0),
        // O has a left group O but no right group O, so this first side names no glyph.
        ("@MMK_L_O", "V", -5.0),
        ("V", "@MMK_R_O", -15.0),
    ];
    set(&mut s, Direction::Ltr, "m01", &pairs);
    set(&mut s, Direction::Ltr, "m02", &[("A", "V", -60.0)]);

    // Removing a glyph leaves its pairs; the right group A loses its only member.
    run(&mut s, "glyph.removeGlyph", json!({"glyphs": ["A"]}));
    assert_eq!(stored(s.font().unwrap(), Direction::Ltr, "m01"), owned(&pairs));

    let out = undoable(&mut s, "kerning.cleanUp", json!({}));
    let removed: Vec<(String, String, String)> = out["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["left"].as_str().unwrap().into(), r["reason"].as_str().unwrap().into(), r["name"].as_str().unwrap().into()))
        .collect();
    let want = [("A", "missingGlyph", "A"), ("@A", "emptyGroup", "A"), ("@Gone", "emptyGroup", "Gone"), ("@O", "emptyGroup", "O")];
    assert_eq!(removed, want.iter().map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string())).collect::<Vec<_>>());
    let font = s.font().unwrap().clone();
    assert_eq!(stored(&font, Direction::Ltr, "m01"), owned(&[("V", "@MMK_R_O", -15.0)]));
    assert_eq!(stored(&font, Direction::Ltr, "m02"), owned(&[("A", "V", -60.0)]), "Clean Up acts on one master");

    // Nothing left to clean: the font stays the same object.
    let before = s.doc().unwrap().font.clone();
    assert_eq!(run(&mut s, "kerning.cleanUp", json!({}))["removed"], json!([]));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before));
    refused(&mut s, "kerning.cleanUp", json!({"master": "nope"}));
}

#[test]
fn rename_group_for_the_whole_font_or_one_pair_keeps_sides_apart() {
    // kerning.renameGroup, kerning.renameSidesIndependent
    let mut s = font_with(&[("T", Some("T"), Some("T")), ("n", Some("n"), Some("n"))]);
    set(&mut s, Direction::Ltr, "m01", &[("@MMK_L_T", "@MMK_R_n", -40.0), ("n", "@MMK_R_T", -12.0)]);
    set(&mut s, Direction::Ltr, "m02", &[("@MMK_L_T", "@MMK_R_n", -50.0)]);
    // Right to left: the second glyph faces the pair with its right group.
    set(&mut s, Direction::Rtl, "m01", &[("n", "@MMK_L_T", -7.0)]);

    let out = undoable(&mut s, "kerning.renameGroup", json!({"side": "left", "from": "T", "to": "Tee", "scope": "font"}));
    assert_eq!(out["glyphs"], json!(["T"]));
    assert_eq!(out["pairs"].as_array().unwrap().len(), 3);
    let font = s.font().unwrap().clone();
    let t = font.glyph("T").unwrap();
    assert_eq!((t.kern_groups.left.as_deref(), t.kern_groups.right.as_deref()), (Some("T"), Some("Tee")));
    assert_eq!(stored(&font, Direction::Ltr, "m01"), owned(&[("@MMK_L_Tee", "@MMK_R_n", -40.0), ("n", "@MMK_R_T", -12.0)]));
    assert_eq!(stored(&font, Direction::Ltr, "m02"), owned(&[("@MMK_L_Tee", "@MMK_R_n", -50.0)]));
    assert_eq!(stored(&font, Direction::Rtl, "m01"), owned(&[("n", "@MMK_L_Tee", -7.0)]));

    // One pair only: the pair points at the new name, glyph n keeps its group.
    let pair = json!({"left": "@Tee", "right": "@n"});
    let out = undoable(&mut s, "kerning.renameGroup", json!({"side": "right", "from": "n", "to": "m", "scope": "pair", "pair": pair}));
    assert_eq!(out["pairs"], json!([{"direction": "ltr", "master": "m01", "left": "@Tee", "right": "@m"}]));
    let font = s.font().unwrap().clone();
    assert_eq!(font.glyph("n").unwrap().kern_groups.left.as_deref(), Some("n"));
    assert_eq!(stored(&font, Direction::Ltr, "m01"), owned(&[("@MMK_L_Tee", "@MMK_R_m", -40.0), ("n", "@MMK_R_T", -12.0)]));
    assert_eq!(stored(&font, Direction::Ltr, "m02"), owned(&[("@MMK_L_Tee", "@MMK_R_n", -50.0)]));

    // Refusals change nothing.
    let e = refused(&mut s, "kerning.renameGroup", json!({"side": "left", "from": "Tee", "to": "n", "scope": "font"}));
    assert!(e.contains("exists"), "{e}");
    let e = refused(
        &mut s,
        "kerning.renameGroup",
        json!({"side": "right", "from": "Tee", "to": "X", "scope": "pair", "pair": {"left": "@Tee", "right": "@m"}}),
    );
    assert!(e.contains("@m"), "{e}");
    let e = refused(&mut s, "kerning.renameGroup", json!({"side": "left", "from": "Tee", "to": "a b", "scope": "font"}));
    assert!(e.contains("character 2"), "{e}");
    let e = refused(&mut s, "kerning.renameGroup", json!({"side": "left", "from": "Nope", "to": "Other", "scope": "font"}));
    assert!(e.contains("Nope"), "{e}");
    refused(&mut s, "kerning.renameGroup", json!({"side": "left", "from": "Tee", "to": "U", "scope": "pair"}));
}
