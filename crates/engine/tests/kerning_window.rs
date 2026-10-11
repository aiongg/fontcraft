//! The Kerning window's commands: the pair list with its search options and sort, copy and paste
//! of kerning-text between masters and documents with each conflict choice, deletion, and
//! hostile kerning-text.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use fontcraft_engine::Session;
use fontcraft_engine::model::plist::Value as Plist;
use fontcraft_engine::model::{Font, Param};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Runs a command that must fail, checks that the font is the same object as before, and
/// returns the message.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let e = s.execute(id, &p).expect_err(id).to_string();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id}: a refused command changed the font");
    e
}

fn set(f: &mut Font, master: &str, first: &str, second: &str, value: f64) {
    Arc::make_mut(&mut f.kerning).ltr.set(master, first, second, value);
}

/// Groups: right group T (T, Tcaron), left group o (o, oacute), right group A, left group V.
/// Pairs of `m01` in stored order: @T–@o −60, T–oacute −20, A–V −80, @A–@V −50, V–A −80.
/// A second master `m02` (Bold) has no pairs.
fn font() -> Font {
    let mut f = Font::new("Kern");
    f.add_glyph("Tcaron", vec![], 600.0).unwrap();
    f.add_glyph("oacute", vec![], 500.0).unwrap();
    for (glyph, right, left) in [
        ("T", Some("T"), None),
        ("Tcaron", Some("T"), None),
        ("o", None, Some("o")),
        ("oacute", None, Some("o")),
        ("A", Some("A"), None),
        ("V", None, Some("V")),
    ] {
        let mut g = f.glyph_mut(glyph).unwrap();
        g.kern_groups.right = right.map(str::to_string);
        g.kern_groups.left = left.map(str::to_string);
    }
    set(&mut f, "m01", "@MMK_L_T", "@MMK_R_o", -60.0);
    set(&mut f, "m01", "T", "oacute", -20.0);
    set(&mut f, "m01", "A", "V", -80.0);
    set(&mut f, "m01", "@MMK_L_A", "@MMK_R_V", -50.0);
    set(&mut f, "m01", "V", "A", -80.0);
    let mut bold = f.masters[0].clone();
    bold.id = "m02".into();
    bold.name = "Bold".into();
    Arc::make_mut(&mut f.masters).push(bold);
    f
}

fn session() -> Session {
    let mut s = Session::new();
    s.add_document(font(), None).unwrap();
    s
}

/// The stored indices `kerning.listPairs` returns for `params` on `m01`.
fn listed(s: &mut Session, params: Value) -> Vec<u64> {
    let out = run(s, "kerning.listPairs", params);
    out["pairs"].as_array().unwrap().iter().map(|r| r["index"].as_u64().unwrap()).collect()
}

fn rows(s: &mut Session, master: &str) -> Vec<(String, String, f64)> {
    let out = run(s, "kerning.listPairs", json!({"master": master}));
    out["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["left"].as_str().unwrap().into(), r["right"].as_str().unwrap().into(), r["value"].as_f64().unwrap()))
        .collect()
}

fn ltr(s: &Session, master: &str, first: &str, second: &str) -> Option<f64> {
    s.font().unwrap().kerning.ltr.get(master, first, second)
}

#[test]
fn kerning_list_pairs_filters_with_the_search_options_and_sorts() {
    let mut s = session();
    let all = run(&mut s, "kerning.listPairs", json!({}));
    assert_eq!(all["total"], 5);
    assert_eq!(
        all["pairs"][0],
        json!({"left": "@T", "right": "@o", "value": -60.0, "index": 0}),
        "groups are written with @, in the spelling of the group facing the pair"
    );
    assert_eq!(listed(&mut s, json!({})), [0, 1, 2, 3, 4], "stored order by default");

    // Match modes; names are compared without @ unless the term has it, case-sensitively.
    assert_eq!(listed(&mut s, json!({"term": "T"})), [0, 1]);
    assert_eq!(listed(&mut s, json!({"term": "o", "match": "is"})), [0]);
    assert_eq!(listed(&mut s, json!({"term": "@o", "match": "is"})), [0]);
    assert_eq!(listed(&mut s, json!({"term": "@"})), [0, 3]);
    assert_eq!(listed(&mut s, json!({"term": "oa", "match": "beginsWith"})), [1]);
    assert_eq!(listed(&mut s, json!({"term": "cute", "match": "endsWith"})), [1]);
    assert_eq!(listed(&mut s, json!({"term": "A", "match": "doesNotContain"})), [0, 1], "neither side contains the term");
    assert_eq!(listed(&mut s, json!({"term": "A", "match": "doesNotContain", "side": "left"})), [0, 1, 4]);

    // Entry kind, side, and the glyph selection (a group counts when a selected glyph is in it).
    assert_eq!(listed(&mut s, json!({"kind": "group"})), [0, 3]);
    assert_eq!(listed(&mut s, json!({"kind": "notGroup"})), [1, 2, 4]);
    assert_eq!(listed(&mut s, json!({"term": "o", "kind": "group", "side": "right"})), [0]);
    assert_eq!(listed(&mut s, json!({"term": "V", "side": "right"})), [2, 3]);
    assert_eq!(listed(&mut s, json!({"glyphs": ["Tcaron"]})), [0]);
    assert_eq!(listed(&mut s, json!({"glyphs": ["oacute"]})), [0, 1]);

    // Sorting; ties keep the stored order in both directions.
    assert_eq!(listed(&mut s, json!({"sort": "value"})), [2, 4, 0, 3, 1]);
    assert_eq!(listed(&mut s, json!({"sort": "value", "descending": true})), [1, 3, 0, 2, 4]);
    assert_eq!(listed(&mut s, json!({"sort": "left"})), [3, 0, 2, 1, 4]);
    assert_eq!(listed(&mut s, json!({"term": "T", "sort": "right", "descending": true})), [1, 0]);

    for bad in [
        json!({"match": "like"}),
        json!({"kind": "both"}),
        json!({"side": "top"}),
        json!({"sort": "name"}),
        json!({"glyphs": ["nope"]}),
        json!({"master": "m09"}),
    ] {
        s.execute("kerning.listPairs", &bad).expect_err(&bad.to_string());
    }
    let long = "x".repeat(10_000);
    s.execute("kerning.listPairs", &json!({"term": long})).expect_err("a term longer than any name");
}

#[test]
fn kerning_copy_pairs_and_paste_pairs_into_another_master() {
    let mut s = session();
    let copied = run(&mut s, "kerning.copyPairs", json!({}));
    assert_eq!(copied["count"], 5);
    assert_eq!(copied["text"], "kerning-text\t1\nmaster\tRegular\ndirection\tltr\n@T\t@o\t-60\nT\toacute\t-20\nA\tV\t-80\n@A\t@V\t-50\nV\tA\t-80\n");
    let some = run(&mut s, "kerning.copyPairs", json!({"pairs": [{"left": "@A", "right": "@V"}, {"left": "T", "right": "oacute"}]}));
    assert_eq!(some["text"], "kerning-text\t1\nmaster\tRegular\ndirection\tltr\n@A\t@V\t-50\nT\toacute\t-20\n", "selected pairs in the order given");
    s.execute("kerning.copyPairs", &json!({"pairs": [{"left": "T", "right": "o"}]})).expect_err("a pair the master lacks");

    let before = s.doc().unwrap().font.clone();
    let out = run(&mut s, "kerning.pastePairs", json!({"text": copied["text"], "master": "m02"}));
    assert_eq!((out["added"].clone(), out["asked"].clone(), out["conflictCount"].clone()), (json!(5), json!(false), json!(0)));
    assert_eq!(rows(&mut s, "m02"), rows(&mut s, "m01"), "the other master has the same pairs in the same order");
    assert_eq!(ltr(&s, "m02", "@MMK_L_T", "@MMK_R_o"), Some(-60.0), "group sides are stored in the file's spelling");
    run(&mut s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "undo restores the font");

    // The JSON form pastes the same way.
    let out = run(&mut s, "kerning.pastePairs", json!({"pairs": [{"left": "@T", "right": "@o", "value": -12.5}], "master": "m02"}));
    assert_eq!(out["added"], 1);
    assert_eq!(ltr(&s, "m02", "@MMK_L_T", "@MMK_R_o"), Some(-12.0), "rounded half up, as a typed value");
}

#[test]
fn kerning_paste_pairs_into_another_document_reports_missing_glyphs_and_groups() {
    let mut s = session();
    let text = run(&mut s, "kerning.copyPairs", json!({}))["text"].clone();
    s.add_document(Font::new("Other"), None).unwrap();
    let out = run(&mut s, "kerning.pastePairs", json!({"text": text}));
    assert_eq!(out["added"], 5);
    assert_eq!(out["unknownGlyphs"], json!(["oacute"]));
    assert_eq!(out["unknownGroups"], json!(["@T", "@o", "@A", "@V"]), "a group the font lacks is pasted as it is and reported");
    assert_eq!(s.font().unwrap().family_name, "Other");
    assert_eq!(ltr(&s, "m01", "T", "oacute"), Some(-20.0));
    assert_eq!(ltr(&s, "m01", "@MMK_L_A", "@MMK_R_V"), Some(-50.0));
}

#[test]
fn kerning_paste_conflict_choice_ask_overwrite_keep_existing() {
    let text = "@T\t@o\t-70\nT\toacute\t-20\no\tT\t-15\n";
    let mut s = session();
    let before = s.doc().unwrap().font.clone();
    let asked = run(&mut s, "kerning.pastePairs", json!({"text": text}));
    assert_eq!(asked["asked"], true);
    assert_eq!(asked["conflictCount"], 1);
    assert_eq!(asked["conflicts"], json!([{"left": "@T", "right": "@o", "value": -70.0, "existing": -60.0}]));
    assert_eq!(asked["unchanged"], 1, "a pair with the same value is not a conflict");
    assert_eq!(asked["added"], 0);
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "ask changes nothing while there are conflicts");

    let out = run(&mut s, "kerning.pastePairs", json!({"text": text, "onConflict": "overwrite"}));
    assert_eq!((out["added"].clone(), out["overwritten"].clone(), out["kept"].clone()), (json!(1), json!(1), json!(0)));
    assert_eq!(ltr(&s, "m01", "@MMK_L_T", "@MMK_R_o"), Some(-70.0));
    assert_eq!(ltr(&s, "m01", "o", "T"), Some(-15.0));
    run(&mut s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "the paste is one undo step");

    let out = run(&mut s, "kerning.pastePairs", json!({"text": text, "onConflict": "keepExisting"}));
    assert_eq!((out["added"].clone(), out["overwritten"].clone(), out["kept"].clone()), (json!(1), json!(0), json!(1)));
    assert_eq!(ltr(&s, "m01", "@MMK_L_T", "@MMK_R_o"), Some(-60.0));
    assert_eq!(ltr(&s, "m01", "o", "T"), Some(-15.0));

    // Without conflicts, ask pastes at once.
    let out = run(&mut s, "kerning.pastePairs", json!({"text": "o\tV\t-5"}));
    assert_eq!((out["asked"].clone(), out["added"].clone()), (json!(false), json!(1)));
    // A pair named twice: the later value counts.
    run(&mut s, "kerning.pastePairs", json!({"text": "o\tA\t-1\no\tA\t-2", "master": "m02"}));
    assert_eq!(ltr(&s, "m02", "o", "A"), Some(-2.0));
}

#[test]
fn kerning_paste_takes_values_as_set_pair_does_and_refuses_control_characters() {
    let mut s = session();
    run(&mut s, "kerning.pastePairs", json!({"text": "A\tW\t-7.5\nW\tA\t2.5\nT\tW\t32767", "master": "m02"}));
    assert_eq!((ltr(&s, "m02", "A", "W"), ltr(&s, "m02", "W", "A"), ltr(&s, "m02", "T", "W")), (Some(-7.0), Some(3.0), Some(32767.0)));
    for (pairs, text) in [(json!(null), "A\tW\t-32768"), (json!([{"left": "A", "right": "W", "value": -32768}]), "")] {
        let p = if text.is_empty() { json!({"pairs": pairs}) } else { json!({"text": text}) };
        let e = refused(&mut s, "kerning.pastePairs", p);
        assert!(e.contains("-32767 to 32767"), "{e}");
    }
    let e = refused(&mut s, "kerning.pastePairs", json!({"pairs": [{"left": "a\nb", "right": "V", "value": -10}]}));
    assert!(e.contains("control character (byte 1)"), "{e}");
}

#[test]
fn kerning_paste_refuses_a_linked_master() {
    let mut f = font();
    Arc::make_mut(&mut f.masters)[1].params.0.push(Param::new("Link Metrics With First Master", Plist::Bool(true)));
    let mut s = Session::new();
    s.add_document(f, None).unwrap();
    let e = refused(&mut s, "kerning.pastePairs", json!({"text": "o\tT\t-15", "master": "m02"}));
    assert!(e.contains("m01"), "{e}");
    refused(&mut s, "kerning.deletePairs", json!({"pairs": [{"left": "A", "right": "V"}], "master": "m02"}));
    let out = run(&mut s, "kerning.listPairs", json!({"master": "m02"}));
    assert_eq!((out["source"].clone(), out["count"].clone()), (json!("m01"), json!(5)), "a linked master lists the pairs it shows");
}

#[test]
fn kerning_delete_pairs_from_one_or_all_masters() {
    let mut s = session();
    run(&mut s, "kerning.pastePairs", json!({"text": "A\tV\t-80\n@A\t@V\t-50", "master": "m02"}));
    let before = s.doc().unwrap().font.clone();
    let out = run(&mut s, "kerning.deletePairs", json!({"pairs": [{"left": "A", "right": "V"}]}));
    assert_eq!(out, json!({"deleted": 1, "masters": ["m01"]}));
    assert_eq!(ltr(&s, "m01", "A", "V"), None);
    assert_eq!(ltr(&s, "m02", "A", "V"), Some(-80.0), "the other master keeps it");
    assert_eq!(ltr(&s, "m01", "V", "A"), Some(-80.0));
    run(&mut s, "edit.undo", json!({}));
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before));

    let out = run(&mut s, "kerning.deletePairs", json!({"pairs": [{"left": "A", "right": "V"}, {"left": "@A", "right": "@V"}], "allMasters": true}));
    assert_eq!(out, json!({"deleted": 4, "masters": ["m01", "m02"]}));
    assert!(s.font().unwrap().kerning.ltr.0.iter().all(|m| m.master != "m02"), "an emptied master leaves the table");

    let e = refused(&mut s, "kerning.deletePairs", json!({"pairs": [{"left": "V", "right": "A"}, {"left": "T", "right": "o"}]}));
    assert!(e.contains("pairs[1]"), "{e}");
    refused(&mut s, "kerning.deletePairs", json!({"pairs": []}));
    refused(&mut s, "kerning.deletePairs", json!({"pairs": [{"left": "@bad name", "right": "A"}]}));
}

#[test]
fn kerning_paste_refuses_hostile_kerning_text() {
    let mut s = session();
    let big = "A\tV\t-1\n".repeat(16 * 1024 * 1024 / 7 + 1);
    let mut cases: Vec<(String, &str)> = vec![
        (big, "16777216"),
        (format!("A\t{}\t5", "x".repeat(1_000_000)), "line 1 is"),
        ("A\tV\t-1\n".repeat(500_001), "more than 500000 pairs"),
        ("A\tV\t-1\nA\tV\tx".into(), "line 2, byte 11"),
        ("A\tV".into(), "line kind"),
        ("A\tV\t-1\textra".into(), "fields"),
        ("A\tV\tNaN".into(), "not a number"),
        ("A\tV\tinf".into(), "not a number"),
        ("A\tV\t1e400".into(), "too large"),
        ("A\tV\t40000".into(), "outside"),
        ("@bad name\tV\t1".into(), "group name"),
        ("\tV\t1".into(), "empty"),
        ("kerning-text\t2\nA\tV\t1".into(), "version"),
        ("A\tV\t1\nkerning-text\t1".into(), "first line"),
        ("direction\tsideways\nA\tV\t1".into(), "direction"),
        ("group\tmiddle\tV\tV".into(), "group side"),
        ("metrics\tA\t1\t2".into(), "metrics line"),
        ("\n\n".into(), "no pairs"),
    ];
    cases.push(("A\tV\t-1\r\n\r\nV\tA\t-2".into(), ""));
    for (text, message) in cases {
        if message.is_empty() {
            let out = run(&mut s, "kerning.pastePairs", json!({"text": text, "master": "m02"}));
            assert_eq!(out["added"], 2, "CRLF line ends and blank lines are fine");
            continue;
        }
        let e = refused(&mut s, "kerning.pastePairs", json!({"text": text, "master": "m02"}));
        assert!(e.contains(message), "{}: {e}", text.chars().take(30).collect::<String>());
    }
    for pairs in [
        json!([{"left": "A", "right": "V", "value": 1e308}]),
        json!([{"left": "A", "right": "V", "value": "1"}]),
        json!([{"left": "A"}]),
        json!("A V 1"),
    ] {
        refused(&mut s, "kerning.pastePairs", json!({"pairs": pairs}));
    }
    refused(&mut s, "kerning.pastePairs", json!({"text": "A\tV\t1", "pairs": [{"left": "A", "right": "V", "value": 1}]}));
    refused(&mut s, "kerning.pastePairs", json!({"text": "A\tV\t1", "onConflict": "merge"}));
}

/// Kerning-text written from Inter Roman's Regular master reads back equal, and copying all its
/// pairs and pasting them into the master emptied again rebuilds its table exactly. Also times
/// `kerning.listPairs` with a search and a sort (target: under 10 ms in a release build).
/// `cargo test -p fontcraft-engine --test kerning_window -- --ignored --nocapture`
#[test]
#[ignore = "needs corpus/ (cargo xtask corpus --fetch)"]
fn kerning_text_round_trip_on_inter_regular() {
    use fontcraft_spacing::kerning::Direction;
    use fontcraft_spacing::kerning_text;

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/inter/src/Inter-Roman.glyphspackage");
    let (font, _) = fontcraft_engine::io::open(&path, None).unwrap();
    let master = font.masters.iter().find(|m| m.name == "Regular").expect("a Regular master").id.clone();
    let doc = kerning_text::from_master(&font, Direction::Ltr, &master);
    assert!(doc.pairs.len() > 1000, "Inter's Regular master has {} pairs", doc.pairs.len());
    let text = kerning_text::write(&doc).unwrap();
    assert_eq!(kerning_text::read(&text).unwrap(), doc, "kerning-text written and read back is equal");

    let original = font.kerning.ltr.0.iter().find(|m| m.master == master).unwrap().clone();
    let mut emptied = font.clone();
    Arc::make_mut(&mut emptied.kerning).ltr.0.retain(|m| m.master != master);
    let mut s = Session::new();
    s.add_document(font, None).unwrap();
    let copied = run(&mut s, "kerning.copyPairs", json!({"master": master}))["text"].clone();
    s.add_document(emptied, None).unwrap();
    let out = run(&mut s, "kerning.pastePairs", json!({"text": copied, "master": master}));
    assert_eq!(out["added"].as_u64().unwrap() as usize, doc.pairs.len());
    let pasted = s.font().unwrap().kerning.ltr.0.iter().find(|m| m.master == master).unwrap().clone();
    assert_eq!(pasted, original, "the pasted table equals the stored one, order and spelling included");

    let t = Instant::now();
    let runs = 20;
    for _ in 0..runs {
        run(&mut s, "kerning.listPairs", json!({"master": master, "term": "o", "sort": "value"}));
    }
    let each = t.elapsed() / runs;
    eprintln!("kerning.listPairs with a search and a sort: {each:?} over {} pairs", doc.pairs.len());
    if !cfg!(debug_assertions) {
        assert!(each.as_millis() < 10, "kerning.listPairs took {each:?}");
    }
}
