#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_model::{NodeKind, Path, Shape};
use fontcraft_testkit::{oval, rect};
use serde_json::{Value, json};

use super::{FIND, REPLACE};
use crate::Session;

type Pt = (f64, f64);

/// A stem with a bottom serif that reaches 70 units left of the stem and 30 right: asymmetric,
/// so a mirrored copy differs from the original.
const I: [Pt; 8] = [(170.0, 20.0), (100.0, 20.0), (100.0, 0.0), (260.0, 0.0), (260.0, 20.0), (230.0, 20.0), (230.0, 700.0), (170.0, 700.0)];
/// The serif of [`I`] as an open run, from the stem's left side to its right side.
const SERIF: [Pt; 6] = [(170.0, 20.0), (100.0, 20.0), (100.0, 0.0), (260.0, 0.0), (260.0, 20.0), (230.0, 20.0)];
/// Another serif between the same end nodes: a wedge with one node less.
const WEDGE: [Pt; 5] = [(170.0, 20.0), (80.0, 0.0), (280.0, 0.0), (280.0, 15.0), (230.0, 20.0)];

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn nodes(points: &[Pt]) -> Value {
    Value::Array(points.iter().map(|(x, y)| json!({"x": x, "y": y})).collect())
}

fn open(points: &[Pt]) -> Value {
    json!([{"closed": false, "nodes": nodes(points)}])
}

fn add(s: &mut Session, glyph: &str, nodes: Value) {
    run(s, "path.addPath", json!({"glyph": glyph, "nodes": nodes}));
}

/// `points` moved by `f`, in reverse order when `reverse`: a mirrored copy drawn the right way
/// round.
fn moved(points: &[Pt], f: impl Fn(Pt) -> Pt, reverse: bool) -> Vec<Pt> {
    let mut out: Vec<Pt> = points.iter().copied().map(f).collect();
    if reverse {
        out.reverse();
    }
    out
}

fn paths(s: &Session, glyph: &str) -> Vec<Path> {
    let font = s.font().unwrap();
    let layer = &font.glyph(glyph).unwrap().layers[0];
    layer
        .shapes
        .iter()
        .filter_map(|sh| match sh {
            Shape::Path(p) => Some(p.clone()),
            _ => None,
        })
        .collect()
}

fn xy(p: &Path) -> Vec<Pt> {
    p.nodes.iter().map(|n| (n.x, n.y)).collect()
}

/// The serif in five glyphs: `I`, `l` (moved), `H` (two stems), `T` (turned upside down) and `J`
/// (mirrored), the last two drawn the right way round.
fn serif_font() -> Session {
    let mut s = Session::with_document();
    add(&mut s, "I", nodes(&I));
    add(&mut s, "l", nodes(&moved(&I, |(x, y)| (x + 50.0, y), false)));
    add(&mut s, "H", nodes(&I));
    add(&mut s, "H", nodes(&moved(&I, |(x, y)| (x + 300.0, y), false)));
    add(&mut s, "T", nodes(&moved(&I, |(x, y)| (x, 720.0 - y), true)));
    add(&mut s, "J", nodes(&moved(&I, |(x, y)| (460.0 - x, y), true)));
    s
}

fn find_match<'a>(r: &'a Value, glyph: &str) -> &'a Value {
    r["matches"].as_array().unwrap().iter().find(|m| m["glyph"] == glyph).unwrap_or_else(|| panic!("no match in {glyph}: {r}"))
}

#[test]
fn find_outline_finds_a_serif_in_five_glyphs_under_flips() {
    let mut s = serif_font();
    let plain = run(&mut s, FIND, json!({"pattern": open(&SERIF)}));
    assert_eq!(plain["count"], 4, "{plain}");
    assert_eq!(plain["glyphs"], json!(["H", "I", "l"]), "the turned and mirrored serifs need allowFlipped");
    assert_eq!(find_match(&plain, "l")["transform"], json!([1.0, 0.0, 0.0, 1.0, 50.0, 0.0]));

    let all = run(&mut s, FIND, json!({"pattern": open(&SERIF), "allowFlipped": true}));
    assert_eq!(all["count"], 6, "{all}");
    assert_eq!(all["glyphs"], json!(["H", "I", "J", "T", "l"]));
    let t = find_match(&all, "T");
    assert_eq!(t["flipped"], true);
    assert_eq!(t["transform"], json!([1.0, 0.0, 0.0, -1.0, 0.0, 720.0]));
    assert_eq!(t["paths"], json!([{"shape": 0, "start": 2, "nodes": 6}]));
    assert_eq!(find_match(&all, "J")["transform"], json!([-1.0, 0.0, 0.0, 1.0, 460.0, 0.0]));
    assert_eq!(all["text"].as_str().unwrap().lines().count(), 6);
    assert_eq!(all["stopped"], Value::Null);
}

#[test]
fn replace_outline_swaps_every_serif_for_another_in_one_step() {
    let mut s = serif_font();
    let r = run(&mut s, REPLACE, json!({"pattern": open(&SERIF), "replacement": open(&WEDGE), "allowFlipped": true}));
    assert_eq!(r["replaced"], 6, "{r}");
    assert_eq!(r["glyphs"], json!(["H", "I", "J", "T", "l"]));

    let mut wedged: Vec<Pt> = WEDGE.to_vec();
    wedged.extend([(230.0, 700.0), (170.0, 700.0)]);
    assert_eq!(xy(&paths(&s, "I")[0]), wedged);
    assert_eq!(xy(&paths(&s, "H")[1]), moved(&wedged, |(x, y)| (x + 300.0, y), false));
    assert_eq!(xy(&paths(&s, "T")[0]), moved(&wedged, |(x, y)| (x, 720.0 - y), true), "the turned serif is replaced turned");
    assert_eq!(xy(&paths(&s, "J")[0]), moved(&wedged, |(x, y)| (460.0 - x, y), true), "the mirrored serif is replaced mirrored");

    let again = run(&mut s, FIND, json!({"pattern": open(&SERIF), "allowFlipped": true}));
    assert_eq!(again["count"], 0, "{again}");
    let wedges = run(&mut s, FIND, json!({"pattern": open(&WEDGE), "allowFlipped": true}));
    assert_eq!(wedges["count"], 6, "{wedges}");

    run(&mut s, "edit.undo", json!({}));
    assert_eq!(xy(&paths(&s, "I")[0]), I.to_vec(), "Replace All is one undo step");
    assert_eq!(xy(&paths(&s, "J")[0]), moved(&I, |(x, y)| (460.0 - x, y), true));
}

#[test]
fn open_pattern_end_nodes_match_by_position_and_on_curve_status_only() {
    let mut s = Session::with_document();
    // The stem's left side arrives at the serif's first node as a curve.
    let mut curved: Vec<Value> = I.iter().map(|(x, y)| json!({"x": x, "y": y})).collect();
    curved[0]["type"] = json!("curve");
    curved.extend([json!({"x": 170, "y": 500, "type": "offcurve"}), json!({"x": 170, "y": 200, "type": "offcurve"})]);
    add(&mut s, "I", Value::Array(curved));
    // Here the serif's last point is a handle of the curve up the stem's right side.
    let handle = json!([
        {"x": 170, "y": 20}, {"x": 100, "y": 20}, {"x": 100, "y": 0}, {"x": 260, "y": 0}, {"x": 260, "y": 20},
        {"x": 230, "y": 20, "type": "offcurve"}, {"x": 230, "y": 300, "type": "offcurve"}, {"x": 230, "y": 700, "type": "curve"},
        {"x": 170, "y": 700}
    ]);
    add(&mut s, "O", handle);

    let r = run(&mut s, FIND, json!({"pattern": open(&SERIF)}));
    assert_eq!(r["glyphs"], json!(["I"]), "a curve node matches the pattern's line end node; a handle does not: {r}");

    run(&mut s, REPLACE, json!({"pattern": open(&SERIF), "replacement": open(&WEDGE)}));
    let p = &paths(&s, "I")[0];
    assert_eq!(p.nodes[0].kind, NodeKind::Curve, "the first node keeps the kind of the segment that arrives there");
    assert_eq!(xy(p)[..5], WEDGE);
    assert_eq!(xy(&paths(&s, "O")[0]).len(), 9, "the glyph without a match is unchanged");
}

#[test]
fn loose_end_points_give_only_direction_and_least_length() {
    let mut s = Session::with_document();
    add(&mut s, "I", nodes(&I));
    // A stem 30 units tall: shorter than the pattern's 40.
    add(&mut s, "i", nodes(&[(170.0, 20.0), (100.0, 20.0), (100.0, 0.0), (260.0, 0.0), (260.0, 20.0), (230.0, 20.0), (230.0, 50.0), (170.0, 50.0)]));
    // A stem whose right side leans.
    add(
        &mut s,
        "k",
        nodes(&[(170.0, 20.0), (100.0, 20.0), (100.0, 0.0), (260.0, 0.0), (260.0, 20.0), (230.0, 20.0), (330.0, 700.0), (170.0, 700.0)]),
    );
    let pattern: [Pt; 8] = [(170.0, 60.0), (170.0, 20.0), (100.0, 20.0), (100.0, 0.0), (260.0, 0.0), (260.0, 20.0), (230.0, 20.0), (230.0, 60.0)];

    let strict = run(&mut s, FIND, json!({"pattern": open(&pattern)}));
    assert_eq!(strict["count"], 0, "{strict}");
    let loose = run(&mut s, FIND, json!({"pattern": open(&pattern), "endpointsLoose": true}));
    assert_eq!(loose["glyphs"], json!(["I"]), "{loose}");
    assert_eq!(find_match(&loose, "I")["paths"], json!([{"shape": 0, "start": 7, "nodes": 8}]));

    let replacement: [Pt; 7] = [(170.0, 60.0), (170.0, 20.0), (80.0, 0.0), (280.0, 0.0), (280.0, 15.0), (230.0, 20.0), (230.0, 60.0)];
    run(&mut s, REPLACE, json!({"pattern": open(&pattern), "replacement": open(&replacement), "endpointsLoose": true}));
    assert_eq!(
        xy(&paths(&s, "I")[0]),
        vec![(170.0, 700.0), (170.0, 20.0), (80.0, 0.0), (280.0, 0.0), (280.0, 15.0), (230.0, 20.0), (230.0, 700.0)],
        "the stem keeps its own end nodes"
    );
}

#[test]
fn replace_outline_swaps_closed_shapes_from_glyphs_and_skips_locked_glyphs() {
    let mut s = Session::with_document();
    add(&mut s, "a", rect(0.0, 0.0, 40.0, 40.0));
    add(&mut s, "b", oval(20.0, 20.0, 20.0, 20.0));
    add(&mut s, "i", rect(0.0, 100.0, 40.0, 500.0));
    add(&mut s, "i", rect(300.0, 600.0, 340.0, 640.0));
    add(&mut s, "j", rect(0.0, 600.0, 40.0, 640.0));
    add(&mut s, "j", rect(100.0, 600.0, 140.0, 640.0));
    add(&mut s, "k", rect(0.0, 600.0, 40.0, 640.0));
    s.edit("lock", |f| {
        f.glyph_mut("k")?.info.locked = true;
        Ok(())
    })
    .unwrap();

    let found = run(&mut s, FIND, json!({"patternGlyph": "a"}));
    assert_eq!(found["glyphs"], json!(["i", "j", "k"]), "the pattern glyph is not searched: {found}");

    let r = run(&mut s, REPLACE, json!({"patternGlyph": "a", "replacementGlyph": "b"}));
    assert_eq!(r["replaced"], 3, "{r}");
    assert_eq!(r["skipped"], json!([{"glyph": "k", "reason": "the glyph is locked"}]));
    let dot = xy(&paths(&s, "b")[0]);
    let i = paths(&s, "i");
    assert_eq!(i.len(), 2);
    assert_eq!(xy(&i[0]).len(), 4, "the stem, a square of another size, stays");
    assert_eq!(xy(&i[1]), moved(&dot, |(x, y)| (x + 300.0, y + 600.0), false), "the oval takes the square's place");
    assert_eq!(xy(&paths(&s, "j")[1]), moved(&dot, |(x, y)| (x + 100.0, y + 600.0), false));
    assert_eq!(paths(&s, "a").len(), 1, "the pattern glyph is left alone");
    assert_eq!(xy(&paths(&s, "k")[0]).len(), 4, "the locked glyph is unchanged");
}

#[test]
fn bad_requests_change_nothing() {
    let mut s = serif_font();
    let before = s.font().unwrap().clone();
    let refused = |s: &mut Session, id: &str, p: Value| s.execute(id, &p).err().unwrap_or_else(|| panic!("{id} should refuse {p}")).to_string();

    let e = refused(&mut s, FIND, json!({"pattern": open(&SERIF), "allowRotated": true}));
    assert!(e.contains("not supported"), "{e}");
    let e = refused(&mut s, FIND, json!({"pattern": open(&SERIF), "patternGlyph": "I"}));
    assert!(e.contains("not both"), "{e}");
    let e = refused(&mut s, REPLACE, json!({"pattern": open(&SERIF), "replacement": [{"nodes": nodes(&WEDGE)}]}));
    assert!(e.contains("one open path"), "{e}");
    let e = refused(&mut s, REPLACE, json!({"pattern": open(&SERIF)}));
    assert!(e.contains("replacement"), "{e}");
    let e = refused(&mut s, FIND, json!({"pattern": open(&SERIF), "tolerance": f64::MAX}));
    assert!(e.contains("tolerance"), "{e}");
    let huge: Vec<Pt> = (0..1_100).map(|i| (f64::from(i), 0.0)).collect();
    let e = refused(&mut s, FIND, json!({"pattern": open(&huge)}));
    assert!(e.contains("1024"), "{e}");
    let e = refused(&mut s, FIND, json!({"pattern": [{"nodes": [{"x": 1e300, "y": 0}]}]}));
    assert!(e.contains("within"), "{e}");
    assert!(s.font().unwrap().same_as(&before));
}
