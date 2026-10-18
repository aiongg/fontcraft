//! Construction rules: the README examples of the format, rebuilt with our own glyphs, and the
//! expected values of the original library's own tests (Glyph Construction, MIT, Copyright (c)
//! 2014 Frederik Berlaen; `Lib/glyphConstruction.py`), which use a small font of triangles.

use fontcraft_model::{Anchor, Font, Glyph, Guide, Layer, Node, NodeKind, Path, Shape};

use crate::construction::*;

fn put(font: &mut Font, name: &str, width: f64, path: Option<Path>, anchors: &[(&str, f64, f64)]) {
    let mut g = Glyph::new(name);
    let mut l = Layer::new("m01", width);
    if let Some(p) = path {
        l.shapes.push(Shape::Path(p));
    }
    l.anchors = anchors.iter().map(|(n, x, y)| Anchor::new(*n, *x, *y)).collect();
    g.layers.push(l);
    font.glyphs.remove(name);
    font.glyphs.push(g);
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Option<Path> {
    Some(Path::rect(x0, y0, x1, y1))
}

/// Our own glyphs for the README's examples.
fn readme_font() -> Font {
    let mut f = Font::new("Rules");
    put(&mut f, "A", 600.0, rect(50.0, 0.0, 550.0, 700.0), &[("top", 300.0, 700.0), ("bottom", 300.0, 0.0)]);
    put(&mut f, "acute", 300.0, rect(200.0, 500.0, 300.0, 600.0), &[("_top", 250.0, 500.0)]);
    put(&mut f, "grave", 200.0, rect(100.0, 500.0, 160.0, 560.0), &[]);
    put(&mut f, "ring", 100.0, rect(0.0, 0.0, 100.0, 80.0), &[("_top", 50.0, 0.0), ("top", 50.0, 80.0)]);
    put(&mut f, "f", 300.0, rect(50.0, 0.0, 250.0, 700.0), &[]);
    put(&mut f, "i", 200.0, rect(50.0, 0.0, 150.0, 500.0), &[]);
    put(&mut f, "V", 600.0, rect(0.0, 0.0, 600.0, 700.0), &[]);
    put(&mut f, "O", 700.0, rect(50.0, -10.0, 650.0, 710.0), &[("top", 350.0, 710.0)]);
    put(&mut f, "circumflex", 300.0, rect(0.0, 0.0, 200.0, 100.0), &[("_top", 100.0, 0.0), ("top", 100.0, 100.0)]);
    put(&mut f, "dotaccent", 100.0, rect(0.0, 0.0, 60.0, 60.0), &[]);
    put(&mut f, "b", 650.0, rect(100.0, 0.0, 500.0, 700.0), &[]);
    put(&mut f, "b-1", 130.0, rect(0.0, 0.0, 100.0, 100.0), &[]);
    f
}

/// The original library's test font: triangles growing by 20 units per glyph.
fn upstream_font() -> Font {
    let mut f = Font::new("Upstream");
    for (i, name) in ["a", "grave", "f", "i", "agrave"].iter().enumerate() {
        let add = 20.0 * i as f64;
        let n = |x: f64, y: f64| Node::new(x, y, NodeKind::Line);
        let p = Path::new(true, vec![n(100.0, 100.0), n(200.0 + add, 100.0 + add), n(200.0 + add, 200.0 + add)]);
        put(&mut f, name, 60.0 + 10.0 * i as f64, Some(p), &[]);
    }
    f
}

fn no_kern(_: &str, _: &str, _: &str) -> f64 {
    0.0
}

fn one(text: &str) -> Rule {
    let file = parse_rules(text).unwrap();
    assert!(file.errors.is_empty(), "{text}: {:?}", file.errors);
    file.rules.into_iter().last().unwrap()
}

fn build_with(font: &Font, text: &str, kern: KernFn<'_>) -> Built {
    build_rule(font, None, kern, &one(text), &["m01".to_string()]).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn build(font: &Font, text: &str) -> Built {
    build_with(font, text, &no_kern)
}

/// Each component's glyph and offset.
fn at(b: &Built) -> Vec<(String, f64, f64)> {
    b.layers[0]
        .shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Component(c) => {
                let m = c.transform.affine().as_coeffs();
                Some((c.glyph.clone(), m[4], m[5]))
            }
            _ => None,
        })
        .collect()
}

fn coeffs(b: &Built, i: usize) -> [f64; 6] {
    match &b.layers[0].shapes[i] {
        Shape::Component(c) => c.transform.affine().as_coeffs(),
        other => panic!("not a component: {other:?}"),
    }
}

fn parts(list: &[(&str, f64, f64)]) -> Vec<(String, f64, f64)> {
    list.iter().map(|(n, x, y)| (n.to_string(), *x, *y)).collect()
}

fn width(b: &Built) -> f64 {
    b.layers[0].width
}

// ── The README's examples ───────────────────────────────────────────────────────────────────────

#[test]
fn parts_join_with_plus_and_a_missing_suffix_falls_back() {
    let f = readme_font();
    let b = build(&f, "Aacute = A + acute");
    assert_eq!(at(&b), parts(&[("A", 0.0, 0.0), ("acute", 0.0, 0.0)]));
    assert_eq!(width(&b), 600.0);
    let b = build(&f, "Aacute = A + acute.cap");
    assert_eq!(at(&b), parts(&[("A", 0.0, 0.0), ("acute", 0.0, 0.0)]), "acute.cap is missing, so acute");
    assert!(b.missing.is_empty());
    for layer in &b.layers {
        for s in &layer.shapes {
            let Shape::Component(c) = s else { panic!("only components") };
            assert_eq!(c.info.alignment, fontcraft_model::Alignment::Disabled, "rule positions are kept");
        }
    }
}

#[test]
fn comments_notes_code_points_and_colours() {
    let file = parse_rules("# a comment\nAacute = A + acute\nAgrave = A + grave # this is a note\n").unwrap();
    assert_eq!(file.rules.len(), 2);
    assert_eq!(file.rules[0].note, None);
    assert_eq!(file.rules[1].note.as_deref(), Some("this is a note"));
    assert_eq!(file.rules[1].line, 3);

    assert_eq!(one("Aacute = A + acute | 00C1").unicodes, vec![0xC1]);
    let r = one("Tcommaaccent = T + commaaccent @ bottom | 021A, 0162");
    assert_eq!(r.unicodes, vec![0x21A, 0x162]);
    let b = build(&readme_font(), "Tcommaaccent = T + commaaccent @ bottom | 021A, 0162");
    assert_eq!(b.missing, vec!["commaaccent".to_string()], "a missing part stays a component and is listed");
    assert_eq!(at(&b), parts(&[("T", 0.0, 0.0), ("commaaccent", 0.0, 0.0)]));

    assert_eq!(one("Aacute = A + acute ! 1, 0, 0, 1").color, Some([1.0, 0.0, 0.0, 1.0]));
}

#[test]
fn width_and_margins_from_numbers_and_other_glyphs() {
    let f = readme_font();
    assert_eq!(width(&build(&f, "Aacute = A + acute ^ 400")), 400.0);

    let b = build(&f, "Aacute = A + acute ^ 30, 30");
    assert_eq!(at(&b), parts(&[("A", -20.0, 0.0), ("acute", -20.0, 0.0)]));
    assert_eq!(width(&b), 560.0);

    assert_eq!(width(&build(&f, "Aacute = A + acute ^ A * 2")), 1200.0);

    // Left: half of A's left margin (25); right: twice A's right margin (100).
    let b = build(&f, "Aacute = A + acute ^ A / 2, A * 2");
    assert_eq!(at(&b), parts(&[("A", -25.0, 0.0), ("acute", -25.0, 0.0)]));
    assert_eq!(width(&b), 625.0);

    // `'` takes the opposite side: b's right margin (150) on the left, its left (100) on the right.
    let b = build(&f, "b.alt = b ^ b', b'");
    assert_eq!(at(&b), parts(&[("b", 50.0, 0.0)]));
    assert_eq!(width(&b), 650.0);

    // Quoted names may hold operators.
    let b = build(&f, r#"X = b ^ "b", "b-1" * 2"#);
    assert_eq!(at(&b), parts(&[("b", 0.0, 0.0)]));
    assert_eq!(width(&b), 560.0);
}

#[test]
fn flags_decompose_add_source_and_skip_existing() {
    let r = one("*agrave = a + grave");
    assert!(r.decompose && !r.add_source && !r.skip_existing);
    let r = one("?Aacute = A + acute");
    assert!(r.skip_existing && r.glyph == "Aacute");
    assert!(one("*>a = a.ss01").add_source);

    // `>` starts from the glyph's own drawing; margins then move it.
    let b = build(&readme_font(), ">b = ^ 40, 50");
    let Shape::Path(p) = &b.layers[0].shapes[0] else { panic!("the source path") };
    let xs: Vec<f64> = p.nodes.iter().map(|n| n.x).collect();
    assert_eq!(xs, vec![40.0, 440.0, 440.0, 40.0]);
    assert_eq!(width(&b), 490.0);
}

#[test]
fn positions_by_numbers_percentages_references_and_matrices() {
    let f = readme_font();
    assert_eq!(at(&build(&f, "Aacute = A + acute@100")), parts(&[("A", 0.0, 0.0), ("acute", -100.0, -400.0)]));
    assert_eq!(at(&build(&f, "Aacute = A + acute@100,100")), parts(&[("A", 0.0, 0.0), ("acute", -100.0, -400.0)]));
    assert_eq!(at(&build(&f, "Aacute = A + acute@50%,50%")), parts(&[("A", 0.0, 0.0), ("acute", 50.0, -200.0)]));
    // `top` finds A's anchor and acute's `_top`; `center` the middle of the bounds.
    assert_eq!(at(&build(&f, "Aacute = A + acute@center,top")), parts(&[("A", 0.0, 0.0), ("acute", 50.0, 200.0)]));
    // Without anchors `top` puts the mark's bottom on the base's top anchor.
    assert_eq!(at(&build(&f, "Agrave = A + grave@center,top")), parts(&[("A", 0.0, 0.0), ("grave", 170.0, 200.0)]));
    assert_eq!(at(&build(&f, "Aacute = A + acute@1, 0, 0, 1, 100, 100")), parts(&[("A", 0.0, 0.0), ("acute", -100.0, -400.0)]));
    assert_eq!(coeffs(&build(&f, "Aacute = A + acute@2,0,0,2,100,100"), 1), [2.0, 0.0, 0.0, 2.0, -100.0, -400.0]);
}

#[test]
fn a_named_base_changes_the_glyph_measured_from() {
    let f = readme_font();
    let expected = parts(&[("O", 0.0, 0.0), ("circumflex", 250.0, 710.0), ("dotaccent", 320.0, 650.0)]);
    assert_eq!(at(&build(&f, "Ocircumflexdotaccent = O + circumflex@center,top + dotaccent@O:center,bottom")), expected);
    assert_eq!(at(&build(&f, r#"Ocircumflexdotaccent = O + circumflex@center,top + dotaccent@"O":center,bottom"#)), expected);
}

#[test]
fn flipping_mirrors_a_part_in_place() {
    let f = readme_font();
    assert_eq!(coeffs(&build(&f, "Aacute = A + acute@~center,top"), 1), [-1.0, 0.0, 0.0, 1.0, 550.0, 200.0]);
    assert_eq!(coeffs(&build(&f, "Aacute = A + acute@center,~top"), 1), [1.0, 0.0, 0.0, -1.0, 50.0, 1300.0]);
    assert_eq!(coeffs(&build(&f, "Aacute = A + acute@~center,~top"), 1), [-1.0, 0.0, 0.0, -1.0, 550.0, 1300.0]);
}

#[test]
fn stacking_and_formulas() {
    let f = readme_font();
    let b = build(&f, "Aringacute = A + ring@center,top + acute@center,top");
    assert_eq!(at(&b), parts(&[("A", 0.0, 0.0), ("ring", 250.0, 700.0), ("acute", 50.0, 280.0)]));
    // On the base `top+10` is 10 above the anchor; on the mark the number changes nothing.
    let b = build(&f, "Aringacute = A + ring@center,`top+10` + acute@center,`top-10`");
    assert_eq!(at(&b), parts(&[("A", 0.0, 0.0), ("ring", 250.0, 710.0), ("acute", 50.0, 280.0)]));
}

#[test]
fn bases_stack_horizontally_and_kern_on_request() {
    let f = readme_font();
    let b = build(&f, "ffi = f & f & i");
    assert_eq!(at(&b), parts(&[("f", 0.0, 0.0), ("f", 300.0, 0.0), ("i", 600.0, 0.0)]));
    assert_eq!(width(&b), 800.0);

    let kern = |_: &str, a: &str, b: &str| if (a, b) == ("A", "V") { -50.0 } else { 0.0 };
    let b = build_with(&f, "A_V = A &\\ V", &kern);
    assert_eq!(at(&b), parts(&[("A", 0.0, 0.0), ("V", 550.0, 0.0)]));
    assert_eq!(width(&b), 1150.0);
    let b = build_with(&f, "A_V = A & V", &kern);
    assert_eq!(at(&b), parts(&[("A", 0.0, 0.0), ("V", 600.0, 0.0)]), "no kerning without `\\`");
}

#[test]
fn variables_and_file_options() {
    let text = "# OverwriteExistingGlyphs: False\n# autounicodes : TRUE\n# MarkGlyphs: 0.5, 0, 0, 1\n\
                $myColorMark = 1, 0, 0, 1 # declaration\n$where = center,top\n$inner = {where}\n\
                Agrave = A + grave@{inner} ! {myColorMark} # usage\n";
    let file = parse_rules(text).unwrap();
    assert_eq!(file.options.overwrite, Some(false));
    assert_eq!(file.options.auto_unicodes, Some(true));
    assert_eq!(file.options.mark_color, Some(Some([0.5, 0.0, 0.0, 1.0])));
    let r = &file.rules[0];
    assert_eq!(r.color, Some([1.0, 0.0, 0.0, 1.0]));
    assert_eq!(r.note.as_deref(), Some("usage"));
    let b = build_rule(&readme_font(), None, &no_kern, r, &["m01".to_string()]).unwrap();
    assert_eq!(at(&b), parts(&[("A", 0.0, 0.0), ("grave", 170.0, 200.0)]));

    assert_eq!(parse_rules("# MarkGlyphs: False").unwrap().options.mark_color, Some(None));
}

#[test]
fn construct_glyph_keeps_what_the_rule_does_not_set() {
    let mut f = readme_font();
    let mut existing = Glyph::new("Aacute");
    existing.unicodes = vec![0xC1];
    existing.info.note = "keep me".into();
    existing.layers.push(Layer::new("m01", 10.0));
    f.glyphs.push(existing);
    let rule = one("Aacute = A + acute@center,top");
    let built = build_rule(&f, None, &no_kern, &rule, &["m01".to_string()]).unwrap();
    let opts = ApplyOptions { auto_unicodes: true, mark_color: Some([0.0, 0.0, 1.0, 1.0]) };
    let g = construct_glyph(&f, None, &rule, &built, &opts);
    assert_eq!(g.unicodes, vec![0xC1]);
    assert_eq!(g.info.note, "keep me");
    assert_eq!(g.info.color, Some(fontcraft_model::ColorLabel::Rgba([0.0, 0.0, 1.0, 1.0])));
    assert_eq!(g.layers.len(), 1);
    assert_eq!(g.layers[0].width, 600.0);
    assert_eq!(g.layers[0].shapes.len(), 2);

    let rule = one("Aacute = A + acute | 00E1 ! 1, 0, 0, 1 # new");
    let built = build_rule(&f, None, &no_kern, &rule, &["m01".to_string()]).unwrap();
    let g = construct_glyph(&f, None, &rule, &built, &opts);
    assert_eq!((g.unicodes.clone(), g.info.note.as_str()), (vec![0xE1], "new"));
    assert_eq!(g.info.color, Some(fontcraft_model::ColorLabel::Rgba([1.0, 0.0, 0.0, 1.0])));
}

// ── The original library's own expected values ─────────────────────────────────────────────────

#[test]
fn upstream_positioning() {
    let f = upstream_font();
    let cases: [(&str, (f64, f64)); 5] = [
        ("agrave = a + grave @ 0, 0", (-100.0, -100.0)),
        ("agrave = a + grave @ center, top", (-10.0, 100.0)),
        ("agrave = a + grave @i: center, top", (20.0, 100.0)),
        ("agrave = a + grave @i: center, i: top", (20.0, 160.0)),
        ("agrave = a + grave @100%", (100.0, 100.0)),
    ];
    for (text, (x, y)) in cases {
        assert_eq!(at(&build(&f, text)), parts(&[("a", 0.0, 0.0), ("grave", x, y)]), "{text}");
    }
    let b = build(&f, "agrave = a + grave + anotherGlyph");
    assert_eq!(at(&b), parts(&[("a", 0.0, 0.0), ("grave", 0.0, 0.0), ("anotherGlyph", 0.0, 0.0)]));
    assert_eq!(width(&b), 60.0);
    let b = build(&f, "f_f_i = f & f & i");
    assert_eq!(at(&b), parts(&[("f", 0.0, 0.0), ("f", 80.0, 0.0), ("i", 160.0, 0.0)]));
    assert_eq!(width(&b), 250.0);
    let b = build(&f, "agrave = a + grave ^ 300, 200");
    assert_eq!(at(&b), parts(&[("a", 200.0, 0.0), ("grave", 200.0, 0.0)]));
    assert_eq!(width(&b), 620.0);
}

#[test]
fn upstream_master_guides() {
    let mut f = upstream_font();
    std::sync::Arc::make_mut(&mut f.masters)[0].guides.push(Guide { x: 100.0, y: 200.0, name: Some("guide".into()), ..Default::default() });
    for (text, x) in [("agrave = a + grave@guide", 50.0), ("agrave = a + grave@guide,guide", 50.0), ("agrave = a + grave@0,guide", -100.0)] {
        assert_eq!(at(&build(&f, text)), parts(&[("a", 0.0, 0.0), ("grave", x, 0.0)]), "{text}");
    }
}

#[test]
fn upstream_kerning_and_metrics_only_rules() {
    let f = upstream_font();
    let kern = |_: &str, a: &str, b: &str| if (a, b) == ("a", "i") { -100.0 } else { 0.0 };
    let b = build_with(&f, "r = a & i", &kern);
    assert_eq!((at(&b), width(&b)), (parts(&[("a", 0.0, 0.0), ("i", 60.0, 0.0)]), 150.0));
    let b = build_with(&f, "dest = a & \\i", &kern);
    assert_eq!((at(&b), width(&b)), (parts(&[("a", 0.0, 0.0), ("i", -40.0, 0.0)]), 50.0));
    let b = build_with(&f, "dest = a & \\foo", &kern);
    assert_eq!((at(&b), width(&b)), (parts(&[("a", 0.0, 0.0), ("foo", 60.0, 0.0)]), 60.0));

    let b = build(&f, "space = ^ 100");
    assert!(b.layers[0].shapes.is_empty());
    assert_eq!(width(&b), 100.0);
    let b = build(&f, ">a = ^ 11, 22");
    let Shape::Path(p) = &b.layers[0].shapes[0] else { panic!("the source path") };
    assert_eq!(p.nodes.iter().map(|n| n.x).collect::<Vec<_>>(), vec![11.0, 111.0, 111.0]);
    assert_eq!(width(&b), 133.0);
    let b = build(&f, ">doesNotExist = ^ 11, 22");
    assert!(b.layers[0].shapes.is_empty());
    assert_eq!(width(&b), 0.0);
}

// ── Hostile input ───────────────────────────────────────────────────────────────────────────────

#[test]
fn hostile_rule_files_fail_with_a_message() {
    let big = "a = b\n".repeat(MAX_RULES_BYTES / 6 + 1);
    assert!(matches!(parse_rules(&big), Err(ConstructionError::TooLarge(_))));
    let many = "a=b\n".repeat(MAX_RULES + 1);
    assert!(matches!(parse_rules(&many), Err(ConstructionError::LimitExceeded { .. })));

    let circle = parse_rules("$a = {b}\n$b = {a}\nx = {a}\n").unwrap_err().to_string();
    assert!(circle.contains("deep"), "{circle}");
    let unknown = parse_rules("x = {nope}\n").unwrap_err().to_string();
    assert!(unknown.contains("nope"), "{unknown}");
    // Each level repeats the one below a hundred times: far past the expansion limit.
    let mut bomb = format!("$v0 = {}\n", "a".repeat(1000));
    for k in 1..=4 {
        bomb.push_str(&format!("$v{k} = {}\n", format!("{{v{}}}", k - 1).repeat(100)));
    }
    bomb.push_str("x = {v4}\n");
    assert!(matches!(parse_rules(&bomb), Err(ConstructionError::LimitExceeded { .. })));

    let lines = [
        format!("x = {}a", "a+".repeat(MAX_RULE_PARTS)),
        format!("x = a + b@{}1{}", "(".repeat(100), ")".repeat(100)),
        format!("x = a + b@{}", "1+".repeat(1 << 19)),
        "x = a + b@`top+10".to_string(),
        "x = a + b@1@2".to_string(),
        "x = a + b@1,2,3".to_string(),
        "x = a | zzzz".to_string(),
        "x = a ! 2, 0, 0, 1".to_string(),
        "not a rule".to_string(),
        "x = a = b".to_string(),
        "x = a + b@center,top%".to_string(),
    ];
    for line in &lines {
        let file = parse_rules(&format!("ok = a\n{line}\nalso = b\n")).unwrap();
        assert_eq!(file.rules.len(), 2, "{line:.80}: the other rules still read");
        assert_eq!(file.errors.len(), 1, "{line:.80}");
        assert_eq!(file.errors[0].line, 2);
    }
}

#[test]
fn a_rule_that_would_contain_itself_fails() {
    let mut f = readme_font();
    let err = build_rule(&f, None, &no_kern, &one("A = A + acute"), &["m01".to_string()]).unwrap_err();
    assert!(err.to_string().contains("itself"), "{err}");
    let mut x = Glyph::new("wrapsA");
    let mut l = Layer::new("m01", 100.0);
    l.shapes.push(Shape::Component(fontcraft_model::Component::new("A", 0.0, 0.0)));
    x.layers.push(l);
    f.glyphs.push(x);
    let err = build_rule(&f, None, &no_kern, &one("A = wrapsA"), &["m01".to_string()]).unwrap_err();
    assert!(err.to_string().contains("itself"), "{err}");
}
