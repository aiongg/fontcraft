#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_model::plist::{Str, Value};
use fontcraft_model::{Component, Font, Layer, Node, NodeKind, Param, Path, Shape};

use crate::*;

/// Two masters; outlines on `m01` (with `n` shifted on `m02`):
/// n: L50 R70 (w500) · o: L40 R50 (w500) · H: a triangle, L0 R100 at its bounds and L100 R200 at
/// y=350 · ka: L10 · ka-deva: L30 · koKai-thai: L20 · hyphen: L45 · n.alt: a component of n.
fn font() -> Font {
    let mut f = Font::new("Test");
    for name in ["ka", "ka-deva", "koKai-thai", "hyphen", "n.alt"] {
        f.add_glyph(name, vec![], 600.0).unwrap();
    }
    let mut bold = f.masters[0].clone();
    bold.id = "m02".into();
    bold.name = "Bold".into();
    Arc::make_mut(&mut f.masters).push(bold);
    let set = |f: &mut Font, name: &str, width: f64, shape: Shape| {
        let mut g = f.glyph_mut(name).unwrap();
        g.layers.retain(|l| l.master == "m01");
        g.layers[0].width = width;
        g.layers[0].shapes = vec![shape];
        let mut second = g.layers[0].clone();
        second.id = "m02".into();
        second.master = "m02".into();
        g.layers.push(second);
    };
    set(&mut f, "n", 500.0, Shape::Path(Path::rect(50.0, 0.0, 430.0, 500.0)));
    set(&mut f, "o", 500.0, Shape::Path(Path::rect(40.0, 0.0, 450.0, 500.0)));
    let tri =
        Path::new(true, vec![Node::new(0.0, 0.0, NodeKind::Line), Node::new(400.0, 0.0, NodeKind::Line), Node::new(200.0, 700.0, NodeKind::Line)]);
    set(&mut f, "H", 500.0, Shape::Path(tri));
    set(&mut f, "ka", 200.0, Shape::Path(Path::rect(10.0, 0.0, 110.0, 100.0)));
    set(&mut f, "ka-deva", 400.0, Shape::Path(Path::rect(30.0, 0.0, 330.0, 100.0)));
    set(&mut f, "koKai-thai", 300.0, Shape::Path(Path::rect(20.0, 0.0, 220.0, 100.0)));
    set(&mut f, "hyphen", 300.0, Shape::Path(Path::rect(45.0, 200.0, 255.0, 260.0)));
    set(&mut f, "n.alt", 500.0, Shape::Component(Component::new("n", 0.0, 0.0)));
    for g in f.glyphs.as_arcs().iter().map(|g| g.name.clone()).collect::<Vec<_>>() {
        let mut g = f.glyph_mut(&g).unwrap();
        if !g.layers.iter().any(|l| l.id == "m02") {
            let w = g.layers[0].width;
            g.layers.push(Layer::new("m02", w));
        }
    }
    f.glyph_mut("n").unwrap().layers[1].shapes = vec![Shape::Path(Path::rect(60.0, 0.0, 430.0, 500.0))];
    f
}

fn key(f: &Font, text: &str, field: Field) -> Key {
    match parse_field(text, field, |n| f.glyph(n).is_some()).unwrap() {
        Entry::Key(k) => k,
        Entry::Number(n) => panic!("{text} is the number {n}"),
    }
}

/// `text` evaluated in `field` of `o` on the first master.
fn eval(f: &Font, text: &str, field: Field) -> Result<f64, EvalError> {
    evaluate_key(f, "o", "m01", field, &key(f, text, field))
}

fn set_glyph_key(f: &mut Font, glyph: &str, field: Field, text: &str) {
    let mut g = f.glyph_mut(glyph).unwrap();
    let keys = &mut g.info.metrics_keys;
    *match field {
        Field::Left => &mut keys.left,
        Field::Right => &mut keys.right,
        Field::Width => &mut keys.width,
        Field::Top => &mut keys.top,
        Field::Bottom => &mut keys.bottom,
    } = Some(text.into());
}

#[test]
fn every_documented_form_evaluates_and_keeps_its_text() {
    let f = font();
    use Field::*;
    let cases: &[(&str, Field, f64, &str)] = &[
        ("n", Left, 50.0, "n"),
        ("n", Width, 500.0, "n"),
        ("=n", Left, 50.0, "=n"),
        ("= n + 8", Left, 58.0, "=n+8"),
        ("=n-8", Left, 42.0, "=n-8"),
        ("=n*1.5", Left, 75.0, "=n*1.5"),
        ("=hyphen/2", Left, 23.0, "=hyphen/2"), // 22.5 rounds up
        ("=n/2", Width, 250.0, "=n/2"),
        ("=64", Left, 64.0, "=64"),
        ("=|n", Right, 50.0, "=|n"),
        ("=|n", Left, 70.0, "=|n"),
        ("=|", Left, 50.0, "=|"),
        ("==n", Left, 50.0, "==n"),
        ("=H@350", Left, 100.0, "=H@350"),
        ("=H@350", Right, 200.0, "=H@350"),
        ("=H@800", Left, 0.0, "=H@800"), // no crossing: the bounds
        ("=H+8@350", Left, 108.0, "=H+8@350"),
        ("=+18", Left, 18.0, "=+18"),
        ("=-12", Left, -12.0, "=-12"),
        ("=|n*0.5", Right, 25.0, "=|n*0.5"),
        ("=n", Top, 300.0, "=n"),
        ("=n", Bottom, 200.0, "=n"),
    ];
    for &(text, field, want, display) in cases {
        let k = key(&f, text, field);
        assert_eq!(evaluate_key(&f, "o", "m01", field, &k), Ok(want), "{text} in {field:?}");
        assert_eq!(k.display(), display, "{text}");
        let again = parse_stored(&k.stored(), k.scope, field, |n| f.glyph(n).is_some()).unwrap();
        assert_eq!(again, k, "{text} reads back from its stored form");
    }
    assert_eq!(key(&f, "==n", Left).scope, Scope::Local);
    assert_eq!(key(&f, "==n", Left).stored(), "=n");
    assert_eq!(parse_field("-20", Left, |_| true), Ok(Entry::Number(-20.0)));
    // Halves round up, also below zero.
    assert_eq!(parse_field("=hyphen/-2", Left, |n| f.glyph(n).is_some()).map_err(|e| (e.kind, e.pos)), Err((KeyErrorKind::ExpectedNumber, 8)));
    assert_eq!(eval(&f, "=hyphen-55.5", Left), Ok(-10.0));
}

#[test]
fn fields_refuse_forms_they_do_not_take_and_results_out_of_range() {
    let f = font();
    let exists = |n: &str| f.glyph(n).is_some();
    for (text, field) in [("=|n", Field::Width), ("=|", Field::Width), ("=n@10", Field::Width), ("=n@10", Field::Top)] {
        let e = parse_field(text, field, exists).unwrap_err();
        assert!(matches!(e.kind, KeyErrorKind::NotInField { .. }), "{text}: {e}");
    }
    for (text, kind) in [
        ("=n+1+2", KeyErrorKind::TwoOperations),
        ("=(n+1)", KeyErrorKind::Unexpected('(')),
        ("=n@5+1", KeyErrorKind::Unexpected('+')),
        ("=", KeyErrorKind::ExpectedName),
        ("=n+", KeyErrorKind::ExpectedNumber),
        ("=n*99999999", KeyErrorKind::NumberTooLarge),
    ] {
        assert_eq!(parse_field(text, Field::Left, exists).map_err(|e| e.kind), Err(kind), "{text}");
    }
    assert!(matches!(eval(&f, "=n*200", Field::Width), Err(EvalError::OutOfRange { .. })));
    assert!(matches!(eval(&f, "=n-40000", Field::Left), Err(EvalError::OutOfRange { .. })));
    assert_eq!(eval(&f, "=n/0", Field::Left), Err(EvalError::DivisionByZero));
}

#[test]
fn hyphenated_names_take_the_longest_glyph_name() {
    let f = font();
    // `ka` exists too; the longer name wins.
    assert_eq!(eval(&f, "=ka-deva/2", Field::Left), Ok(15.0));
    assert_eq!(eval(&f, "ka-deva", Field::Left), Ok(30.0));
    // No shorter glyph `koKai`.
    assert_eq!(eval(&f, "=koKai-thai-5", Field::Left), Ok(15.0));
    assert_eq!(eval(&f, "=ka-5", Field::Left), Ok(5.0));
    // Without `=` the whole text is one name.
    assert_eq!(eval(&f, "ka-5", Field::Left), Err(EvalError::Unlinked("ka-5".into())));
    assert_eq!(eval(&f, "=xa-deva-5", Field::Left), Err(EvalError::Unlinked("xa-deva".into())));
    assert_eq!(eval(&f, "=zz+5", Field::Left), Err(EvalError::Unlinked("zz".into())));
}

#[test]
fn stored_keys_local_wins_and_layers_read_their_own_master() {
    let mut f = font();
    set_glyph_key(&mut f, "o", Field::Left, "=n");
    assert_eq!(evaluate(&f, "o", "m01", Field::Left), Ok(Some(50.0)));
    assert_eq!(evaluate(&f, "o", "m02", Field::Left), Ok(Some(60.0)));
    assert_eq!(evaluate(&f, "o", "m01", Field::Right), Ok(None));
    f.glyph_mut("o").unwrap().layers[1].info.metrics_keys.left = Some("n".into());
    {
        let g = f.glyph("o").unwrap();
        assert_eq!(effective_key(g, &g.layers[1], Field::Left), Some((Scope::Local, "n")));
    }
    f.glyph_mut("o").unwrap().layers[1].info.metrics_keys.left = Some("=n+5".into());
    assert_eq!(evaluate(&f, "o", "m02", Field::Left), Ok(Some(65.0)));
    assert_eq!(evaluate(&f, "o", "m01", Field::Left), Ok(Some(50.0)));
    // An intermediate layer reads the layer with its id, else its master's layer.
    let mut mid = f.glyph("o").unwrap().layers[0].clone();
    mid.id = "mid".into();
    mid.info.attrs.coordinates = vec![Some(50.0)];
    f.glyph_mut("o").unwrap().layers.push(mid.clone());
    assert_eq!(evaluate(&f, "o", "mid", Field::Left), Ok(Some(50.0)));
    mid.shapes = vec![Shape::Path(Path::rect(55.0, 0.0, 400.0, 500.0))];
    f.glyph_mut("n").unwrap().layers.push(mid);
    assert_eq!(evaluate(&f, "o", "mid", Field::Left), Ok(Some(55.0)));
}

#[test]
fn cycles_long_chains_and_huge_keys_are_errors() {
    let mut f = font();
    set_glyph_key(&mut f, "o", Field::Left, "=o");
    assert_eq!(evaluate(&f, "o", "m01", Field::Left), Err(EvalError::Cycle));
    set_glyph_key(&mut f, "o", Field::Left, "=|");
    assert_eq!(evaluate(&f, "o", "m01", Field::Left), Ok(Some(50.0)));
    set_glyph_key(&mut f, "o", Field::Right, "=|");
    assert_eq!(evaluate(&f, "o", "m01", Field::Left), Err(EvalError::Cycle));
    // A loop the key leads into but is not part of: the key evaluates; the loop's keys report it.
    set_glyph_key(&mut f, "A", Field::Left, "=o");
    assert_eq!(evaluate(&f, "A", "m01", Field::Left), Ok(Some(40.0)));
    set_glyph_key(&mut f, "n", Field::Left, "=ka");
    set_glyph_key(&mut f, "ka", Field::Left, "=n");
    assert_eq!(evaluate(&f, "n", "m01", Field::Left), Err(EvalError::Cycle));

    // c0 → c1 → … → c33: 33 steps from c0, 32 from c1.
    for i in 0..=33 {
        f.add_glyph(&format!("c{i}"), vec![], 500.0).unwrap();
        f.glyph_mut(&format!("c{i}")).unwrap().layers[0].shapes = vec![Shape::Path(Path::rect(7.0, 0.0, 100.0, 100.0))];
        if i < 33 {
            set_glyph_key(&mut f, &format!("c{i}"), Field::Left, &format!("=c{}", i + 1));
        }
    }
    assert_eq!(evaluate(&f, "c0", "m01", Field::Left), Err(EvalError::TooDeep));
    assert_eq!(evaluate(&f, "c1", "m01", Field::Left), Ok(Some(7.0)));

    let huge = format!("={}", "n".repeat(1 << 20));
    assert_eq!(parse_field(&huge, Field::Left, |_| true).map_err(|e| e.kind), Err(KeyErrorKind::TooLong));
    set_glyph_key(&mut f, "B", Field::Left, &huge);
    set_glyph_key(&mut f, "C", Field::Left, "=n/0");
    set_glyph_key(&mut f, "D", Field::Left, "=missing");
    let found = scan(&f);
    let problem = |g: &str| found.iter().find(|x| x.glyph == g && x.layer == "m01").map(|x| x.problem.clone());
    assert!(matches!(problem("B"), Some(Problem::Error(EvalError::Key(KeyError { kind: KeyErrorKind::TooLong, .. })))));
    assert_eq!(problem("C"), Some(Problem::Error(EvalError::DivisionByZero)));
    assert_eq!(problem("D"), Some(Problem::Unlinked { glyph: "missing".into() }));
    assert_eq!(problem("c0"), Some(Problem::Error(EvalError::TooDeep)));
}

#[test]
fn the_cache_recomputes_only_what_an_edit_reaches() {
    let mut f = font();
    // o's LSB follows n directly; A's follows n.alt, whose outline is a component of n.
    set_glyph_key(&mut f, "o", Field::Left, "=n");
    set_glyph_key(&mut f, "A", Field::Left, "=n.alt");
    for g in ["o", "A"] {
        let mut g = f.glyph_mut(g).unwrap();
        g.layers[0].shapes = vec![Shape::Path(Path::rect(50.0, 0.0, 400.0, 500.0))];
        g.layers[1].shapes = vec![Shape::Path(Path::rect(60.0, 0.0, 400.0, 500.0))];
    }
    let mut font = Arc::new(f);
    let mut cache = SyncCache::default();
    cache.update(&font);
    let n = font.glyphs.len() as u64;
    assert_eq!(cache.computed(), n);
    assert_eq!(cache.all(&font).count(), 0, "{:?}", cache.all(&font).collect::<Vec<_>>());
    assert_eq!(scan(&font), Vec::new());

    // An unrelated edit: one glyph scanned again, nothing found.
    Arc::make_mut(&mut font).glyph_mut("x").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 10.0, 10.0)));
    cache.update(&font);
    assert_eq!(cache.computed(), n + 1);
    assert_eq!(cache.all(&font).count(), 0);

    // n moves: n, o (reads n) and A (reads n through n.alt's component) are scanned again.
    Arc::make_mut(&mut font).glyph_mut("n").unwrap().layers[0].shapes = vec![Shape::Path(Path::rect(70.0, 0.0, 430.0, 500.0))];
    cache.update(&font);
    assert_eq!(cache.computed(), n + 4, "n, o and A");
    let out_of_sync = |g: &str| cache.glyph(g).to_vec();
    let want = |g: &str| {
        vec![Finding {
            glyph: g.into(),
            layer: "m01".into(),
            field: Field::Left,
            scope: Scope::Glyph,
            key: format!("={}", if g == "o" { "n" } else { "n.alt" }),
            problem: Problem::OutOfSync { value: 70.0, actual: 50.0 },
        }]
    };
    assert_eq!(out_of_sync("o"), want("o"));
    assert_eq!(out_of_sync("A"), want("A"));
    assert_eq!(cache.all(&font).cloned().collect::<Vec<_>>(), scan(&font));
}

#[test]
fn update_order_puts_what_a_key_reads_first() {
    let mut f = font();
    set_glyph_key(&mut f, "A", Field::Left, "=B");
    set_glyph_key(&mut f, "B", Field::Left, "=C");
    set_glyph_key(&mut f, "o", Field::Right, "=|");
    set_glyph_key(&mut f, "o", Field::Left, "=n");
    set_glyph_key(&mut f, "D", Field::Left, "=E");
    set_glyph_key(&mut f, "E", Field::Left, "=D");
    let slot = |g: &str, field| Slot { glyph: g.into(), layer: "m01".into(), field };
    let given = [
        slot("D", Field::Left),
        slot("A", Field::Left),
        slot("o", Field::Right),
        slot("B", Field::Left),
        slot("o", Field::Left),
        slot("C", Field::Left),
        slot("A", Field::Left),
        slot("E", Field::Left),
    ];
    let got = update_order(&f, &given);
    let names: Vec<String> = got.iter().map(|s| format!("{}.{}", s.glyph, s.field.name())).collect();
    assert_eq!(names, ["o.left", "o.right", "C.left", "B.left", "A.left", "D.left", "E.left"]);
}

/// Updating a base glyph's left sidebearing moves its outline in every composite that shows it,
/// so composites wait for their bases, through components of components too.
#[test]
fn update_order_puts_a_composites_bases_first() {
    let mut f = font();
    f.add_glyph("n.alt2", vec![], 500.0).unwrap();
    for l in &mut f.glyph_mut("n.alt2").unwrap().layers {
        l.shapes = vec![Shape::Component(Component::new("n.alt", 0.0, 0.0))];
    }
    set_glyph_key(&mut f, "n", Field::Left, "=o");
    let slot = |g: &str, field| Slot { glyph: g.into(), layer: "m01".into(), field };
    let names = |slots: &[Slot]| -> Vec<String> { update_order(&f, slots).iter().map(|s| format!("{}.{}", s.glyph, s.field.name())).collect() };
    let given =
        [slot("n.alt2", Field::Left), slot("n.alt", Field::Right), slot("n.alt", Field::Left), slot("n", Field::Left), slot("o", Field::Left)];
    assert_eq!(names(&given), ["o.left", "n.left", "n.alt.right", "n.alt.left", "n.alt2.left"]);
    let given = [slot("n.alt2", Field::Left), slot("n.alt", Field::Right), slot("n", Field::Left), slot("o", Field::Left)];
    assert_eq!(names(&given), ["o.left", "n.left", "n.alt2.left", "n.alt.right"]);
    // The base's right sidebearing and width leave its outline where it is.
    let given = [slot("n.alt", Field::Left), slot("n", Field::Right), slot("n", Field::Width)];
    assert_eq!(names(&given), ["n.alt.left", "n.right", "n.width"]);
}

#[test]
fn linked_masters_name_their_source() {
    let mut f = font();
    let mut third = f.masters[1].clone();
    third.id = "m03".into();
    third.name = "Black".into();
    Arc::make_mut(&mut f.masters).push(third);
    let link = |f: &mut Font, m: usize, name: &str, v: Value| Arc::make_mut(&mut f.masters)[m].params.0.push(Param::new(name, v));
    assert_eq!(metrics_source(&f, "m02").map(|m| m.id.as_str()), Some("m02"));
    assert_eq!(metrics_source(&f, "nope"), None);
    link(&mut f, 2, LINK_WITH_MASTER, Value::String(Str::from("Bold")));
    assert_eq!(metrics_source(&f, "m03").map(|m| m.id.as_str()), Some("m02"));
    link(&mut f, 1, LINK_WITH_MASTER, Value::String(Str::from("unknown")));
    assert_eq!(metrics_source(&f, "m02").map(|m| m.id.as_str()), Some("m02"));
    link(&mut f, 2, LINK_WITH_FIRST_MASTER, Value::Bool(true));
    assert_eq!(metrics_source(&f, "m03").map(|m| m.id.as_str()), Some("m01"));
    // A linked master's layers are not scanned.
    f.glyph_mut("o").unwrap().layers.push(Layer::new("m03", 500.0));
    set_glyph_key(&mut f, "o", Field::Left, "=99");
    let layers: Vec<String> = scan(&f).into_iter().filter(|x| x.glyph == "o").map(|x| x.layer).collect();
    assert_eq!(layers, ["m01", "m02"]);
}

#[test]
fn italic_masters_measure_along_the_angle_through_half_the_x_height() {
    let mut f = font();
    f.set_metric_pos("m01", fontcraft_model::metric::ITALIC_ANGLE, 10.0).unwrap();
    let o = f.glyph("o").unwrap();
    // o is the box 40..450 × 0..500 in a width of 500; half the x-height is 250.
    let lean = 10f64.to_radians().tan() * 250.0;
    let near = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() < 1e-9);
    assert!(near(metric(&f, &o.layers[0], Field::Left).unwrap(), 40.0 - lean));
    assert!(near(metric(&f, &o.layers[0], Field::Right).unwrap(), 50.0 - lean));
    assert!(near(metric(&f, &o.layers[0], Field::Top).unwrap(), 300.0));
    // The second master is upright.
    assert_eq!(metric(&f, &o.layers[1], Field::Left), Ok(Some(40.0)));
    // At a height, the crossing moves along the angle to half the x-height.
    let (l, r) = sidebearings_at_height(&f, &o.layers[0], 500.0).unwrap().unwrap();
    assert!((l - (40.0 - lean)).abs() < 1e-9 && (r - (50.0 + lean)).abs() < 1e-9, "{l} {r}");
}
