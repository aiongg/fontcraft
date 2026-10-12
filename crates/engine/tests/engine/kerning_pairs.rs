//! Kerning pairs and exceptions: kerning.setPair, kerning.removePair, kerning.getPair,
//! kerning.addException, kerning.removeException and the behaviour
//! kerning.closingLockRemovesException, on test fonts built in code.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::model::plist::Value as Plist;
use fontcraft_engine::model::{Font, Param};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let e = s.execute(id, &p).err().unwrap_or_else(|| panic!("{id} {p} should fail")).to_string();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} {p}: a refused command changes nothing");
    e
}

fn group(f: &mut Font, glyph: &str, side: &str, name: &str) {
    if f.glyph(glyph).is_none() {
        f.add_glyph(glyph, vec![], 500.0).unwrap();
    }
    let mut g = f.glyph_mut(glyph).unwrap();
    let slot = match side {
        "left" => &mut g.kern_groups.left,
        "right" => &mut g.kern_groups.right,
        "top" => &mut g.kern_groups.top,
        _ => &mut g.kern_groups.bottom,
    };
    *slot = Some(name.to_string());
}

/// The oracle test font of `docs/plans/M5.md` § 2: A–@V −30, @A–V −40, @A–@V −50, T–@o −60 with
/// the exception T–oacute 0, and the right-to-left @alef–@bet −77. Master m02 follows m01.
fn oracle_font() -> Font {
    let mut f = Font::new("Kern");
    for (glyph, side, name) in [
        ("A", "right", "A"),
        ("Aacute", "right", "A"),
        ("V", "left", "V"),
        ("W", "left", "V"),
        ("T", "right", "T"),
        ("Tcaron", "right", "T"),
        ("o", "left", "o"),
        ("oacute", "left", "o"),
        ("alef", "left", "alef"),
        ("bet", "right", "bet"),
    ] {
        group(&mut f, glyph, side, name);
    }
    let k = Arc::make_mut(&mut f.kerning);
    k.ltr.set("m01", "A", "@MMK_R_V", -30.0);
    k.ltr.set("m01", "@MMK_L_A", "V", -40.0);
    k.ltr.set("m01", "@MMK_L_A", "@MMK_R_V", -50.0);
    k.ltr.set("m01", "T", "@MMK_R_o", -60.0);
    k.ltr.set("m01", "T", "oacute", 0.0);
    k.rtl.set("m01", "@MMK_R_alef", "@MMK_L_bet", -77.0);
    let mut linked = f.masters[0].clone();
    linked.id = "m02".into();
    linked.name = "Linked".into();
    linked.params.0.push(Param::new("Link Metrics With First Master", Plist::Bool(true)));
    Arc::make_mut(&mut f.masters).push(linked);
    f
}

fn session() -> Session {
    let mut s = Session::new();
    s.add_document(oracle_font(), None).unwrap();
    s
}

fn get(s: &mut Session, left: &str, right: &str, extra: Value) -> Value {
    let mut p = json!({"left": left, "right": right});
    p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    run(s, "kerning.getPair", p)
}

fn value(s: &mut Session, left: &str, right: &str) -> f64 {
    get(s, left, right, json!({}))["value"].as_f64().unwrap()
}

fn stored(s: &Session, master: &str, first: &str, second: &str) -> Option<f64> {
    s.font().unwrap().kerning.ltr.get(master, first, second)
}

#[test]
fn get_pair_gives_the_oracle_values() {
    // kerning.getPair on the § 2 test font: the values both reference compilers write.
    let mut s = session();
    let cases = [
        ("A", "V", "ltr", -30.0),
        ("Aacute", "V", "ltr", -40.0),
        ("Aacute", "W", "ltr", -50.0),
        ("T", "oacute", "ltr", 0.0),
        ("alef", "bet", "rtl", -77.0),
    ];
    for (l, r, d, v) in cases {
        let got = get(&mut s, l, r, json!({"direction": d}));
        assert_eq!(got["value"], json!(v), "{l} {r} {d}");
    }
    let av = get(&mut s, "A", "V", json!({}));
    assert_eq!(av["step"], "glyphGroup", "glyph–group comes before group–glyph");
    assert_eq!(av["pair"], json!({"left": "A", "right": "@V"}));
    assert_eq!(av["locks"], json!({"left": "open", "right": "closed"}));
    assert_eq!(av["groups"], json!({"left": "A", "right": "V"}));
    let zero = get(&mut s, "T", "oacute", json!({}));
    assert_eq!((zero["step"].as_str(), zero["pair"]["right"].as_str()), (Some("glyphGlyph"), Some("oacute")), "a zero exception is a pair");
    let none = get(&mut s, "V", "A", json!({}));
    assert_eq!((none["value"].as_f64(), &none["step"], &none["pair"]), (Some(0.0), &Value::Null, &Value::Null));
    assert_eq!(get(&mut s, "alef", "bet", json!({}))["value"], json!(0.0), "directions are stored separately");
    let rtl = get(&mut s, "alef", "bet", json!({"direction": "rtl"}));
    assert_eq!(rtl["pair"], json!({"left": "@alef", "right": "@bet"}));
    let linked = get(&mut s, "Aacute", "W", json!({"master": "m02"}));
    assert_eq!((linked["value"].as_f64(), linked["from"].as_str()), (Some(-50.0), Some("m01")), "a linked master shows its source's kerning");

    assert!(refused(&mut s, "kerning.getPair", json!({"left": "A", "right": "nope"})).contains("`right`"));
    assert!(refused(&mut s, "kerning.getPair", json!({"left": "A", "right": "V", "master": "m09"})).contains("m09"));
    assert!(refused(&mut s, "kerning.getPair", json!({"left": "A", "right": "V", "direction": "up"})).contains("`direction`"));
}

#[test]
fn set_pair_stores_rounds_and_keeps_the_spelling() {
    // kerning.setPair: new pairs by position, existing pairs in their spelling, typed values rounded.
    let mut s = session();
    let r = run(&mut s, "kerning.setPair", json!({"left": "@A", "right": "@V", "value": -55}));
    assert_eq!((r["previous"].as_f64(), r["value"].as_f64()), (Some(-50.0), Some(-55.0)));
    assert_eq!(stored(&s, "m01", "@MMK_L_A", "@MMK_R_V"), Some(-55.0));
    assert_eq!(value(&mut s, "Aacute", "W"), -55.0);

    // A UFO-spelled pair is overwritten in place, not doubled.
    {
        let mut f = oracle_font();
        Arc::make_mut(&mut f.kerning).ltr.set("m01", "public.kern1.T", "x", 12.5);
        s.add_document(f, None).unwrap();
    }
    run(&mut s, "kerning.setPair", json!({"left": "@T", "right": "x", "value": 20}));
    assert_eq!(stored(&s, "m01", "public.kern1.T", "x"), Some(20.0));
    assert_eq!(stored(&s, "m01", "@MMK_L_T", "x"), None);
    assert_eq!(value(&mut s, "Tcaron", "x"), 20.0);

    // Halves round up; zero is stored as a pair.
    run(&mut s, "kerning.setPair", json!({"left": "x", "right": "@V", "value": -10.5}));
    assert_eq!(stored(&s, "m01", "x", "@MMK_R_V"), Some(-10.0));
    run(&mut s, "kerning.setPair", json!({"left": "x", "right": "@V", "value": 10.5}));
    assert_eq!(stored(&s, "m01", "x", "@MMK_R_V"), Some(11.0));
    run(&mut s, "kerning.setPair", json!({"left": "Aacute", "right": "W", "value": 0}));
    assert_eq!(get(&mut s, "Aacute", "W", json!({}))["step"], "glyphGlyph");
    assert_eq!(value(&mut s, "Aacute", "W"), 0.0);

    // Right-to-left and vertical sides take their own prefixes.
    run(&mut s, "kerning.setPair", json!({"left": "bet", "right": "@bet", "value": -5, "direction": "rtl"}));
    assert_eq!(s.font().unwrap().kerning.rtl.get("m01", "bet", "@MMK_L_bet"), Some(-5.0));
    {
        let mut f = oracle_font();
        group(&mut f, "x", "bottom", "low");
        group(&mut f, "o", "top", "high");
        s.add_document(f, None).unwrap();
    }
    run(&mut s, "kerning.setPair", json!({"left": "@low", "right": "@high", "value": 40, "direction": "vertical"}));
    assert_eq!(s.font().unwrap().kerning.vertical.get("m01", "@MMK_T_low", "@MMK_B_high"), Some(40.0));
    assert_eq!(get(&mut s, "x", "o", json!({"direction": "vertical"}))["value"], json!(40.0));
    assert!(s.font().unwrap().kerning.ltr.get("m01", "@MMK_T_low", "@MMK_B_high").is_none());

    // One undo step gives back the very font.
    let before = s.doc().unwrap().font.clone();
    run(&mut s, "kerning.setPair", json!({"left": "o", "right": "o", "value": 3}));
    run(&mut s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before));

    let e = refused(&mut s, "kerning.setPair", json!({"left": "@a b", "right": "V", "value": 1}));
    assert!(e.contains("`left`") && e.contains("character 2"), "{e}");
    assert!(refused(&mut s, "kerning.setPair", json!({"left": "@V", "right": "A", "value": 1})).contains("right kerning group `V`"));
    assert!(refused(&mut s, "kerning.setPair", json!({"left": "nope", "right": "A", "value": 1})).contains("no glyph `nope`"));
    assert!(refused(&mut s, "kerning.setPair", json!({"left": "A", "right": "V", "value": 40000})).contains("`value`"));
    assert!(refused(&mut s, "kerning.setPair", json!({"left": "A", "right": "V"})).contains("`value`"));
    let e = refused(&mut s, "kerning.setPair", json!({"left": "A", "right": "V", "value": 1, "master": "m02"}));
    assert!(e.contains("m01"), "a linked master names the master it follows: {e}");
}

#[test]
fn remove_pair_deletes_one_stored_pair() {
    // kerning.removePair: the pair goes in this master only; the next step applies.
    let mut s = session();
    let r = run(&mut s, "kerning.removePair", json!({"left": "A", "right": "@V"}));
    assert_eq!(r["value"], json!(-30.0));
    assert_eq!(get(&mut s, "A", "V", json!({}))["value"], json!(-40.0), "the group–glyph pair applies next");
    run(&mut s, "kerning.removePair", json!({"left": "@alef", "right": "@bet", "direction": "rtl"}));
    assert!(s.font().unwrap().kerning.rtl.is_empty());
    // A pair naming a glyph the font no longer has can still be removed.
    {
        let mut f = oracle_font();
        Arc::make_mut(&mut f.kerning).ltr.set("m01", "gone", "V", 7.0);
        s.add_document(f, None).unwrap();
    }
    run(&mut s, "kerning.removePair", json!({"left": "gone", "right": "V"}));
    assert_eq!(stored(&s, "m01", "gone", "V"), None);
    assert!(refused(&mut s, "kerning.removePair", json!({"left": "V", "right": "A"})).contains("no ltr pair V A"));
    assert!(refused(&mut s, "kerning.removePair", json!({"left": "T", "right": "oacute", "master": "m02"})).contains("m01"));
}

#[test]
fn add_exception_opens_locks_without_moving_anything() {
    // kerning.addException: the pair the opened locks name, at the value that applies.
    let mut s = session();
    assert_eq!(get(&mut s, "Tcaron", "oacute", json!({}))["locks"], json!({"left": "closed", "right": "closed"}));
    assert_eq!(value(&mut s, "Tcaron", "oacute"), 0.0);
    {
        let mut f = oracle_font();
        Arc::make_mut(&mut f.kerning).ltr.set("m01", "@MMK_L_T", "@MMK_R_o", -60.0);
        s.add_document(f, None).unwrap();
    }
    let r = run(&mut s, "kerning.addException", json!({"left": "Tcaron", "right": "oacute", "side": "right"}));
    assert_eq!(r, json!({"left": "@T", "right": "oacute", "value": -60.0, "master": "m01", "direction": "ltr"}));
    assert_eq!(stored(&s, "m01", "@MMK_L_T", "oacute"), Some(-60.0));
    let now = get(&mut s, "Tcaron", "oacute", json!({}));
    assert_eq!((now["value"].as_f64(), &now["locks"]), (Some(-60.0), &json!({"left": "closed", "right": "open"})));
    assert!(refused(&mut s, "kerning.addException", json!({"left": "Tcaron", "right": "oacute", "side": "right"})).contains("no closed lock"));

    // Both locks with a value: the glyph–glyph pair at that value.
    run(&mut s, "kerning.addException", json!({"left": "Tcaron", "right": "o", "side": "both", "value": -70.4}));
    assert_eq!(stored(&s, "m01", "Tcaron", "o"), Some(-70.0));
    assert_eq!(value(&mut s, "Tcaron", "o"), -70.0);
    assert_eq!(value(&mut s, "T", "o"), -60.0, "the rest of the group keeps its kerning");

    // A glyph without a group has no lock to open.
    assert!(refused(&mut s, "kerning.addException", json!({"left": "x", "right": "A", "side": "left"})).contains("no closed lock"));
    assert!(refused(&mut s, "kerning.addException", json!({"left": "T", "right": "o", "side": "up"})).contains("`side`"));
    assert!(refused(&mut s, "kerning.addException", json!({"left": "Tcaron", "right": "o", "side": "left", "master": "m02"})).contains("m01"));
    // Right to left: the first glyph's lock faces the pair with its left group.
    run(&mut s, "kerning.addException", json!({"left": "alef", "right": "bet", "side": "left", "direction": "rtl"}));
    assert_eq!(s.font().unwrap().kerning.rtl.get("m01", "alef", "@MMK_L_bet"), Some(-77.0));
}

#[test]
fn closing_lock_removes_exception() {
    // kerning.removeException and kerning.closingLockRemovesException: closing deletes the
    // exception in this master, and the group value applies again.
    let mut s = session();
    let r = run(&mut s, "kerning.removeException", json!({"left": "T", "right": "oacute", "side": "right"}));
    assert_eq!(r["removed"], json!({"left": "T", "right": "oacute", "value": 0.0}));
    assert_eq!(r["value"], json!(-60.0), "the group pair applies again");
    assert_eq!(stored(&s, "m01", "T", "oacute"), None);
    assert!(refused(&mut s, "kerning.removeException", json!({"left": "T", "right": "oacute", "side": "right"})).contains("no open lock"));
    // T has a group but the pair names T itself: its left lock is open.
    let r = run(&mut s, "kerning.removeException", json!({"left": "T", "right": "o", "side": "left"}));
    assert_eq!((r["removed"]["right"].as_str(), r["value"].as_f64()), (Some("@o"), Some(0.0)));

    // Only this master: another master's exception stays.
    {
        let mut f = oracle_font();
        let mut bold = f.masters[0].clone();
        bold.id = "m03".into();
        bold.name = "Bold".into();
        Arc::make_mut(&mut f.masters).push(bold);
        Arc::make_mut(&mut f.kerning).ltr.set("m03", "T", "oacute", 5.0);
        s.add_document(f, None).unwrap();
    }
    let before = s.doc().unwrap().font.clone();
    run(&mut s, "kerning.removeException", json!({"left": "T", "right": "oacute", "side": "both"}));
    assert_eq!((stored(&s, "m01", "T", "oacute"), stored(&s, "m03", "T", "oacute")), (None, Some(5.0)));
    run(&mut s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "undo gives back the very font");
    assert!(refused(&mut s, "kerning.removeException", json!({"left": "T", "right": "oacute", "side": "both", "master": "m02"})).contains("m01"));
}

#[test]
fn closing_one_lock_of_a_glyph_pair_keeps_the_other_open() {
    let locks = |s: &mut Session, l: &str, r: &str| get(s, l, r, json!({}))["locks"].clone();
    let with_pair = |l: &str, r: &str, v: f64| {
        let mut f = oracle_font();
        Arc::make_mut(&mut f.kerning).ltr.set("m01", l, r, v);
        let mut s = Session::new();
        s.add_document(f, None).unwrap();
        s
    };
    // Right lock: the left glyph against the right glyph's group, at the value that applied.
    let mut s = with_pair("Aacute", "W", -80.0);
    let r = run(&mut s, "kerning.removeException", json!({"left": "Aacute", "right": "W", "side": "right"}));
    assert_eq!(r["stored"], json!({"left": "Aacute", "right": "@V", "value": -80.0}));
    assert_eq!((stored(&s, "m01", "Aacute", "W"), stored(&s, "m01", "Aacute", "@MMK_R_V")), (None, Some(-80.0)));
    assert_eq!(r["value"], json!(-80.0));
    assert_eq!(locks(&mut s, "Aacute", "W"), json!({"left": "open", "right": "closed"}));
    // Left lock: the left glyph's group against the right glyph.
    let mut s = with_pair("Aacute", "W", -80.0);
    run(&mut s, "kerning.removeException", json!({"left": "Aacute", "right": "W", "side": "left"}));
    assert_eq!(stored(&s, "m01", "@MMK_L_A", "W"), Some(-80.0));
    assert_eq!(locks(&mut s, "Aacute", "W"), json!({"left": "closed", "right": "open"}));
    assert_eq!(value(&mut s, "Aacute", "V"), -40.0, "the rest of the group keeps its kerning");

    // The pair for the side that stays open is already stored: only the exception goes.
    let mut s = with_pair("A", "V", -90.0);
    let r = run(&mut s, "kerning.removeException", json!({"left": "A", "right": "V", "side": "right"}));
    assert_eq!((r["stored"].clone(), r["value"].clone()), (Value::Null, json!(-30.0)));
    assert_eq!(locks(&mut s, "A", "V"), json!({"left": "open", "right": "closed"}));
    // A–@V comes before @A–V, so the left lock alone cannot close.
    let mut s = with_pair("A", "V", -90.0);
    let e = refused(&mut s, "kerning.removeException", json!({"left": "A", "right": "V", "side": "left"}));
    assert!(e.contains("A @V") && e.contains("close both locks"), "{e}");
    let r = run(&mut s, "kerning.removeException", json!({"left": "A", "right": "V", "side": "both"}));
    assert_eq!((r["stored"].clone(), r["value"].clone()), (Value::Null, json!(-30.0)));
}
