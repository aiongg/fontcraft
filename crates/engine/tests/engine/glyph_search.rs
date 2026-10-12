//! `glyph.search`, the query behind the Find Glyphs pop-up: every term must match, the Unicode
//! mode, a typed character, the order of the results and the limits.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn found(s: &mut Session, p: Value) -> Vec<String> {
    run(s, "glyph.search", p)["results"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap().to_string()).collect()
}

/// The new font's A to Z and a to z, Cyrillic Ha and Sha with a localised form, a hacek and a
/// breve that each hold only one of the handbook's two terms, and a glyph for U+228E whose name
/// holds no digits.
fn font() -> Session {
    let mut s = Session::with_document();
    for (name, unicodes) in [
        ("Ha-cy", json!(["0425"])),
        ("Sha-cy", json!(["0428"])),
        ("sha-cy", json!(["0448"])),
        ("sha-cy.loclBGR", json!([])),
        ("hacek", json!(["02C7"])),
        ("brevecomb-cy", json!([])),
        ("multisetunion", json!(["228E"])),
    ] {
        run(&mut s, "glyph.newGlyph", json!({"name": name, "unicodes": unicodes}));
    }
    s
}

#[test]
fn glyph_search_finds_the_handbook_examples() {
    let mut s = font();

    // `ha cy`: every glyph whose name holds both terms, ignoring case, suffixed ones too.
    let mut hits = found(&mut s, json!({"query": "ha cy"}));
    hits.sort();
    assert_eq!(hits, ["Ha-cy", "Sha-cy", "sha-cy", "sha-cy.loclBGR"]);
    assert_eq!(found(&mut s, json!({"query": "  sha   loclbgr "})), ["sha-cy.loclBGR"], "terms split at any run of spaces");

    // A hexadecimal code point in Unicode mode; part of one, and `U+`, match as well.
    assert_eq!(found(&mut s, json!({"query": "228E", "mode": "unicode"})), ["multisetunion"]);
    assert_eq!(found(&mut s, json!({"query": "u+228e", "mode": "unicode"})), ["multisetunion"]);
    assert_eq!(found(&mut s, json!({"query": "228", "mode": "all"})), ["multisetunion"]);
    assert!(found(&mut s, json!({"query": "228E"})).is_empty(), "name mode does not read code points");
    assert!(found(&mut s, json!({"query": "union", "mode": "unicode"})).is_empty(), "Unicode mode does not read names");
    assert_eq!(found(&mut s, json!({"query": "union", "mode": "all"})), ["multisetunion"]);

    // A character finds the glyph encoded with it, in every mode.
    for mode in ["name", "unicode", "all"] {
        assert_eq!(found(&mut s, json!({"query": "Х", "mode": mode})), ["Ha-cy"], "{mode}");
        assert_eq!(found(&mut s, json!({"query": "⊎", "mode": mode})), ["multisetunion"], "{mode}");
    }

    // The exact match comes first, then the name that differs only in case, then the others in
    // glyph order.
    let a = found(&mut s, json!({"query": "a"}));
    assert_eq!(a[..2], ["a", "A"]);
    assert!(a.contains(&"Ha-cy".to_string()) && a.contains(&"hacek".to_string()));
    assert_eq!(found(&mut s, json!({"query": "H"}))[..2], ["H", "h"]);
    assert_eq!(found(&mut s, json!({"query": "sha-CY"}))[..2], ["Sha-cy", "sha-cy"], "both differ in case: glyph order");

    // An empty query finds nothing; the limit cuts the list but not the total.
    let r = run(&mut s, "glyph.search", json!({"query": "   "}));
    assert_eq!((&r["count"], &r["total"]), (&json!(0), &json!(0)));
    let r = run(&mut s, "glyph.search", json!({"query": "cy", "limit": 2}));
    assert_eq!((&r["count"], &r["total"]), (&json!(2), &json!(5)));
    let r = run(&mut s, "glyph.search", json!({"query": "ha-cy"}));
    assert_eq!((&r["results"][0]["name"], &r["results"][0]["unicodes"]), (&json!("Ha-cy"), &json!(["0425"])));
}

#[test]
fn glyph_search_refuses_bad_parameters() {
    let mut s = font();
    for bad in [
        json!({}),
        json!({"query": 5}),
        json!({"query": "a", "mode": "regex"}),
        json!({"query": "a", "limit": 0}),
        json!({"query": "a", "limit": 1001}),
        json!({"query": "x".repeat(257)}),
    ] {
        assert!(s.execute("glyph.search", &bad).is_err(), "{bad}");
    }
    assert!(Session::new().execute("glyph.search", &json!({"query": "a"})).is_err(), "no document");
}
