//! Test fonts for M8 (features) and M9 (interpolation).

use std::sync::Arc;

use fontcraft_engine::model::{Anchor, AnchorInfo, AnchorInfoBox, AxisRule, ColorLabel, Features, Font, LayerAttrs, NumberDef, Snippet};
use fontcraft_plist::{Dict, Str, Value};

use super::draw::{
    Draw, Style, add, add_composite, add_letter, add_letters, add_parts, base_anchors, extra_layer, font, instance, letter, m, poly, push_layer, rect,
};
use super::{Built, groups, kern};

pub fn fonts() -> Vec<Built> {
    vec![
        m8_f1(),
        m8_f2(),
        m8_f3(),
        m8_f4(),
        m8_f5(),
        m8_f6(),
        m9_f1(),
        m9_f2(),
        m9_f3(),
        m9_f4(),
        m9_f5(),
        m9_f6(false),
        m9_f6(true),
        m9_f7(true),
        m9_f7(false),
    ]
}

fn one_master(family: &str) -> (Font, Vec<Style>) {
    font(family, &[], &[m("m01", "Regular", &[], 400.0)])
}

fn snippet(name: &str, code: &str, disabled: bool) -> Snippet {
    Snippet { name: name.into(), code: code.into(), disabled, ..Default::default() }
}

const DIGITS: [&str; 10] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];

fn m8_f1() -> Built {
    let (mut f, styles) = one_master("FC Eight Latin");
    add_letter(&mut f, &styles, "space", &[0x20]);
    for c in ('A'..='Z').chain('a'..='z') {
        add_letter(&mut f, &styles, &c.to_string(), &[c as u32]);
    }
    for (i, d) in DIGITS.iter().enumerate() {
        add_letter(&mut f, &styles, d, &[0x30 + i as u32]);
    }
    let few_caps = ["A", "B", "C", "H", "O"];
    for c in few_caps {
        add_letter(&mut f, &styles, &format!("{c}.sc"), &[]);
        add_letter(&mut f, &styles, &format!("{c}.c2sc"), &[]);
        let l = c.to_lowercase();
        add_letter(&mut f, &styles, &format!("{l}.smcp"), &[]);
        add_letter(&mut f, &styles, &format!("{l}.pc"), &[]);
    }
    for d in DIGITS {
        for sfx in ["numr", "dnom", "osf", "tosf", "tf", "lf"] {
            add_letter(&mut f, &styles, &format!("{d}.{sfx}"), &[]);
        }
    }
    for d in ["zero", "one", "two", "three"] {
        for sfx in ["sups", "subs", "sinf"] {
            add_letter(&mut f, &styles, &format!("{d}.{sfx}"), &[]);
        }
    }
    add_letters(
        &mut f,
        &styles,
        &[
            ("onesuperior", Some(0xB9)),
            ("fraction", Some(0x2044)),
            ("slash", Some(0x2F)),
            ("hyphen", Some(0x2D)),
            ("period", Some(0x2E)),
            ("periodcentered", Some(0xB7)),
            ("periodcentered.loclCAT", None),
            ("hyphen.case", None),
            ("slash.case", None),
            ("a.ss01", None),
            ("g.ss02", None),
            ("R.ss03", None),
            ("g.cv01.1", None),
            ("g.cv01.2", None),
            ("Q.swsh", None),
            ("R.swsh", None),
            ("A.titl", None),
            ("s.hist", None),
            ("A.ornm", None),
            ("B.ornm", None),
            ("bullet", Some(0x2022)),
            ("ordfeminine", Some(0xAA)),
            ("ordmasculine", Some(0xBA)),
            ("numero", Some(0x2116)),
            ("longs", Some(0x17F)),
            ("idotless", Some(0x131)),
            ("jdotless", Some(0x237)),
            ("acutecomb", Some(0x301)),
            ("dieresiscomb", Some(0x308)),
            ("dotaccentcomb", Some(0x307)),
            ("commaaccentcomb", Some(0x326)),
            ("cedillacomb", Some(0x327)),
            ("M.qartwidth", None),
            ("M.quartwidth", None),
            ("a.unused", None),
            ("Alpha", Some(0x391)),
            ("Beta", Some(0x392)),
            ("alpha", Some(0x3B1)),
            ("beta", Some(0x3B2)),
            ("A-cy", Some(0x410)),
            ("Be-cy", Some(0x411)),
            ("a-cy", Some(0x430)),
            ("be-cy", Some(0x431)),
        ],
    );
    if let Ok(mut g) = f.glyph_mut("a.unused") {
        g.export = false;
    }
    base_anchors(&mut f, &styles, &["S", "o", "idotless"]);
    add_composite(&mut f, &styles, "odieresis", &[0xF6], "o", &[("dieresiscomb", "top", false)]);
    add_composite(&mut f, &styles, "Scommaaccent", &[0x218], "S", &[("commaaccentcomb", "bottom", false)]);
    add_composite(&mut f, &styles, "Scedilla", &[0x15E], "S", &[("cedillacomb", "bottom", false)]);
    add_composite(&mut f, &styles, "idotaccent", &[], "idotless", &[("dotaccentcomb", "top", false)]);
    add_parts(&mut f, &styles, "onehalf", &[0xBD], &["one.numr", "fraction", "two.dnom"]);
    add_parts(&mut f, &styles, "oneovertwo", &[], &["one.numr", "fraction", "two.dnom"]);
    add_parts(&mut f, &styles, "f_f_i", &[], &["f", "f", "i"]);
    add_parts(&mut f, &styles, "f_i", &[], &["f", "i"]);
    add_parts(&mut f, &styles, "fi", &[0xFB01], &["f", "i"]);
    add_parts(&mut f, &styles, "f_odieresis", &[], &["f", "odieresis"]);
    add_parts(&mut f, &styles, "longs_t", &[], &["longs", "t"]);
    add_parts(&mut f, &styles, "f_f_i.dlig", &[], &["f", "f", "idotless"]);
    Built::font("m8-F1.glyphs", "M8 § 8", "F1 Latin features", f)
        .about(
            "One master. A–Z, a–z, zero–nine, space; `.sc` and `.c2sc` on A B C H O, `.smcp` and `.pc` on a b c h o; `.numr`, `.dnom`, \
             `.osf`, `.tosf`, `.tf`, `.lf` on every figure; `.sups`, `.subs`, `.sinf` on zero–three; onesuperior, fraction, slash, \
             onehalf and oneovertwo (components), hyphen.case, slash.case, a.ss01, g.ss02, R.ss03, g.cv01.1 and g.cv01.2, Q.swsh, \
             R.swsh, A.titl, s.hist, A.ornm, B.ornm, bullet, ordfeminine, ordmasculine, numero, f_f_i, f_i, fi, f_odieresis, longs_t, \
             f_f_i.dlig (components), idotless, jdotless, acutecomb, dieresiscomb, idotaccent, Scommaaccent, Scedilla, periodcentered \
             and periodcentered.loclCAT, M.qartwidth and M.quartwidth, the non-exporting a.unused; Greek Alpha Beta alpha beta and \
             Cyrillic A-cy Be-cy a-cy be-cy. No feature code: Update makes it.",
        )
        .checks("M8 checks 1, 2, 6–10, 19–21, 23–26, 29, 30")
        .gap("The `.sc`, `.c2sc`, `.smcp` and `.pc` sets cover five letters each, not the whole alphabet, to keep the file small.")
        .gap("The two `cv01` variants are named g.cv01.1 and g.cv01.2; the plan does not say how the second variant is named.")
}

fn m8_f2() -> Built {
    let (mut f, styles) = one_master("FC Eight Arabic");
    add_letters(
        &mut f,
        &styles,
        &[
            ("space", Some(0x20)),
            ("beh-ar", Some(0x628)),
            ("beh-ar.init", None),
            ("beh-ar.medi", None),
            ("beh-ar.fina", None),
            ("alef-ar", Some(0x627)),
            ("alef-ar.fina", None),
            ("lam-ar", Some(0x644)),
            ("lam-ar.init", None),
            ("lam_alef-ar", None),
            ("lam_alef-ar.fina", None),
            ("one", Some(0x31)),
            ("two", Some(0x32)),
            ("three", Some(0x33)),
            ("four", Some(0x34)),
        ],
    );
    for (name, u, above) in [("fatha-ar", 0x64E, true), ("kasra-ar", 0x650, false)] {
        add(&mut f, &styles, name, &[u], |s| {
            let (y, a) = if above { (s.xh * 0.5 + 120.0, "top") } else { (-180.0, "bottom") };
            Draw::new(0.0)
                .path(poly(&[(60.0, y), (60.0 + s.stem, y), (160.0, y + 60.0), (160.0 - s.stem, y + 60.0)]))
                .anchor(&format!("_{a}"), 110.0, if above { s.xh * 0.5 + 60.0 } else { -100.0 })
                .anchor(a, 110.0, if above { y + 160.0 } else { y - 80.0 })
        });
    }
    for n in ["beh-ar", "beh-ar.init", "beh-ar.medi", "beh-ar.fina", "lam_alef-ar"] {
        if let Ok(mut g) = f.glyph_mut(n) {
            for (l, s) in g.layers.iter_mut().zip(&styles) {
                let cx = (l.width / 2.0).round();
                l.anchors.push(Anchor::new("top", cx, s.xh * 0.5 + 60.0));
                l.anchors.push(Anchor::new("bottom", cx, -100.0));
            }
        }
    }
    Built::font("m8-F2.glyphs", "M8 § 8", "F2 Arabic", f)
        .about(
            "One master. beh-ar with `.init`, `.medi`, `.fina`; alef-ar and alef-ar.fina, lam-ar and lam-ar.init (so the ligature has \
             its parts); lam_alef-ar and lam_alef-ar.fina; marks fatha-ar and kasra-ar with `_top`/`_bottom` anchors, and `top`/`bottom` \
             anchors on the beh forms and lam_alef-ar; space and one–four for right-to-left digit order.",
        )
        .checks("M8 checks 3, 22, 27")
}

fn m8_f3() -> Built {
    let (mut f, styles) = one_master("FC Eight Devanagari");
    add_letters(
        &mut f,
        &styles,
        &[
            ("space", Some(0x20)),
            ("ka-deva", Some(0x915)),
            ("ssa-deva", Some(0x937)),
            ("ra-deva", Some(0x930)),
            ("halant-deva", Some(0x94D)),
            ("nukta-deva", Some(0x93C)),
            ("reph-deva", None),
            ("rakar-deva", None),
            ("kaHalfform-deva", None),
            ("k-deva", None),
            ("kaNukta-deva", None),
            ("qa-deva", Some(0x958)),
            ("k_ssa-deva", None),
            ("k_ra-deva", None),
            ("iMatra-deva", Some(0x93F)),
            ("iMatra-deva.01", None),
            ("iMatra-deva.02", None),
        ],
    );
    Built::font("m8-F3.glyphs", "M8 § 8", "F3 Devanagari", f)
        .about(
            "One master. ka-deva, ssa-deva, ra-deva, halant-deva, nukta-deva, reph-deva, rakar-deva, kaHalfform-deva, kaNukta-deva, \
             k_ssa-deva, the rakar ligature k_ra-deva, iMatra-deva with `.01` and `.02`; also k-deva and qa-deva, the glyph-data names \
             of the half form and the nukta form, next to the plan's names.",
        )
        .checks("M8 check 4")
        .gap("The plan does not name the rakar ligature; it is k_ra-deva. k-deva and qa-deva are added beside kaHalfform-deva and kaNukta-deva because the glyph database knows only the former names.")
}

fn m8_f4() -> Built {
    let (mut f, styles) = one_master("FC Eight Widths");
    add_letter(&mut f, &styles, "space", &[0x20]);
    for (n, u) in [("a-hira", 0x3042), ("ka-hira", 0x304B), ("a-kata", 0x30A2), ("ka-kata", 0x30AB)] {
        add_letter(&mut f, &styles, n, &[u]);
        for sfx in ["hori", "vert", "proportional"] {
            add_letter(&mut f, &styles, &format!("{n}.{sfx}"), &[]);
        }
    }
    for (n, u) in [("A", 0x41), ("B", 0x42), ("a", 0x61), ("b", 0x62)] {
        add_letter(&mut f, &styles, n, &[u]);
        for sfx in ["full", "half", "thirdwidth"] {
            add_letter(&mut f, &styles, &format!("{n}.{sfx}"), &[]);
        }
    }
    Built::font("m8-F4.glyphs", "M8 § 8", "F4 widths and kana", f)
        .about("One master. a-hira, ka-hira, a-kata, ka-kata, each with `.hori`, `.vert`, `.proportional`; A, B, a, b, each with `.full`, `.half`, `.thirdwidth`; space.")
        .checks("M8 check 5")
}

fn m8_f5() -> Built {
    let (mut f, styles) = font("FC Eight Tokens", &[("Weight", "wght")], &[m("m01", "Regular", &[400.0], 400.0), m("m02", "Bold", &[700.0], 700.0)]);
    add_letters(
        &mut f,
        &styles,
        &[
            ("space", None),
            ("A", None),
            ("H", None),
            ("a", None),
            ("b", None),
            ("n", None),
            ("o", None),
            ("period", Some(0x2E)),
            ("comma", Some(0x2C)),
            ("o.unused", None),
        ],
    );
    if let Ok(mut g) = f.glyph_mut("space") {
        g.unicodes = vec![0x20];
    }
    f.numbers = Arc::new(vec![NumberDef { name: "tracking".into(), ..Default::default() }]);
    for (mm, v) in Arc::make_mut(&mut f.masters).iter_mut().zip([10.0, 30.0]) {
        mm.number_values = vec![v];
    }
    if let Ok(mut g) = f.glyph_mut("b") {
        for (l, s) in g.layers.iter_mut().zip(&styles) {
            l.anchors.push(Anchor::new("top_special", (l.width / 2.0).round(), s.asc));
        }
    }
    let mut info = |name: &str, f2: &dyn Fn(&mut fontcraft_engine::model::GlyphInfo)| {
        if let Ok(mut g) = f.glyph_mut(name) {
            f2(&mut g.info);
        }
    };
    info("A", &|i| i.note = "x marks the spot".into());
    info("a", &|i| i.tags = vec!["ipa".into(), "test".into()]);
    info("H", &|i| {
        i.tags = vec!["ipa".into()];
        i.color = Some(ColorLabel::Index(4));
    });
    info("n", &|i| {
        i.note = "plain note".into();
        i.color = Some(ColorLabel::Index(0));
    });
    info("o", &|i| i.color = Some(ColorLabel::Rgba([0.2, 0.6, 1.0, 1.0])));
    if let Ok(mut g) = f.glyph_mut("o.unused") {
        g.export = false;
    }
    Built::font("m8-F5.glyphs", "M8 § 8", "F5 tokens", f)
        .about(
            "Masters Regular and Bold; number value `tracking` (10 in Regular, 30 in Bold); space, A, H, a, b, n, o, period, comma, the \
             non-exporting o.unused; b with a `top_special` anchor; A noted \"x marks the spot\", n noted \"plain note\"; tags ipa and \
             test on a, ipa on H; colour labels on H (4), n (0) and o (an RGB colour).",
        )
        .checks("M8 checks 12–16")
}

fn m8_f6() -> Built {
    let (mut f, styles) = one_master("FC Eight Context");
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("n", Some(0x6E)), ("o", Some(0x6F)), ("acutecomb", Some(0x301))]);
    for n in ["n", "o"] {
        if let Ok(mut g) = f.glyph_mut(n) {
            for (l, s) in g.layers.iter_mut().zip(&styles) {
                let cx = (l.width / 2.0).round();
                l.anchors.push(Anchor::new("top", cx, s.xh));
                let mut star = Anchor::new("*top", cx + 40.0, s.xh + 40.0);
                if n == "n" {
                    let mut ud = Dict::new();
                    ud.insert("GPOS_Context", Value::String(Str::new("* & o")));
                    star.info = AnchorInfoBox::new(AnchorInfo { user_data: fontcraft_engine::model::UserData(ud), ..Default::default() });
                }
                l.anchors.push(star);
            }
        }
    }
    Built::font("m8-F6.glyphs", "M8 § 8", "F6 contextual anchors", f)
        .about(
            "One master. n, o, acutecomb (`_top`, `top`), space; n and o with `top` and `*top` anchors; n's `*top` has the context \
             `* & o` in its user data under GPOS_Context (the base, the mark, then o).",
        )
        .checks("M8 checks 18, 28")
        .gap("The context text follows the convention of the open-source mark writer (`*` the base, `&` the mark); the reference app's own syntax is what check 28 looks at.")
}

// ---------------------------------------------------------------------------------------------
// M9

fn wght2(family: &str) -> (Font, Vec<Style>) {
    font(family, &[("Weight", "wght")], &[m("m01", "Thin", &[100.0], 100.0), m("m02", "Black", &[900.0], 900.0)])
}

fn f1_glyphs(f: &mut Font, styles: &[Style]) {
    add_letter(f, styles, "space", &[0x20]);
    add_letter(f, styles, "a", &[0x61]);
    add(f, styles, "b", &[0x62], |s| Draw::new(0.0).path(rect(0.0, 0.0, 300.0 + s.stem, s.asc)).spaced(50.0, 300.0 + s.stem, 50.0));
    for (n, u) in [("o", 0x6F), ("H", 0x48), ("e", 0x65)] {
        add_letter(f, styles, n, &[u]);
    }
}

fn m9_f1_font(family: &str) -> (Font, Vec<Style>) {
    let (mut f, styles) = wght2(family);
    f1_glyphs(&mut f, &styles);
    f.instances =
        Arc::new(vec![instance("Thin", &[100.0]), instance("Regular", &[400.0]), instance("Black", &[900.0]), instance("ExtraBlack", &[1000.0])]);
    (f, styles)
}

fn m9_f1() -> Built {
    let (mut f, _) = m9_f1_font("FC Nine One");
    f.features = Arc::new(Features {
        features: vec![
            snippet("ss01", "#ifndef VARIABLE\n#ifdef VARIABLE\nsub a by b;\n#endif\nsub o by e;\n#endif\n", true),
            snippet("ss02", "#ifdef VARIABLE\nsub a by o;\n#else\nsub a by e;\n#endif\n", true),
            snippet("calt", "condition 400 < wght;\ncondition 600 < wght < 800;\nsub a by b;\ncondition;\nsub o by e;\n", true),
        ],
        ..Default::default()
    });
    Built::font("m9-F1.glyphs", "M9 § 8", "F1 two masters", f)
        .about(
            "Masters Thin (wght 100) and Black (900); space, a, b (a rectangle), o (two contours), H, e; instances Thin, Regular (400), \
             Black and ExtraBlack (1000, extrapolated). Three disabled features for checks 33 and 34: ss01 (`#ifdef VARIABLE` inside \
             `#ifndef VARIABLE`), ss02 (`#else`), calt (two `condition` lines and a bare `condition;`).",
        )
        .checks("M9 checks 1, 6, 8–19, 22, 30, 33, 34")
}

fn m9_f2() -> Built {
    let (mut f, styles) = m9_f1_font("FC Nine Two");
    push_layer(
        &mut f,
        "e",
        extra_layer(
            letter("e", &Style::weight("x", 400.0)),
            "m01",
            "e-400",
            "{400}",
            LayerAttrs { coordinates: vec![Some(400.0)], ..Default::default() },
        ),
    );
    add(&mut f, &styles, "dollar", &[0x24], |s| letter("dollar", s));
    for s in &styles {
        let mut alt = Style::weight("x", if s.id == "m01" { 100.0 } else { 900.0 });
        alt.wide = 1.2;
        let d = Draw::new(0.0)
            .paths(vec![rect(0.0, 0.0, 480.0, alt.cap), super::draw::rect_cw(alt.stem, alt.bar, 480.0 - alt.stem, alt.cap - alt.bar)])
            .spaced(50.0, 480.0, 50.0);
        let attrs = LayerAttrs { axis_rules: vec![AxisRule { min: Some(600.0), max: None }], ..Default::default() };
        push_layer(&mut f, "dollar", extra_layer(d, &s.id, &format!("dollar-alt-{}", s.id), "[600‹wg]", attrs));
    }
    add(&mut f, &styles, "Hcomp", &[], |_| Draw::new(0.0).comp("e", 0.0, 0.0));
    let widths = super::draw::drawings(&f, "e");
    if let Ok(mut g) = f.glyph_mut("Hcomp") {
        for (l, d) in g.layers.iter_mut().zip(widths) {
            l.width = d.width;
        }
    }
    Built::font("m9-F2.glyphs", "M9 § 8", "F2 special layers", f)
        .about(
            "F1 plus an intermediate layer {400} on `e` under Thin; `dollar` with an alternate layer `[600‹wg]` (axis rule: weight 600 \
             and up) under each master, whose drawing has no bar; Hcomp, a composite of `e`.",
        )
        .checks("M9 checks 2, 3, 21, 23–25, 35")
}

fn m9_f3() -> Built {
    let (mut f, _) = font(
        "FC Nine Three",
        &[("Weight", "wght"), ("Width", "wdth")],
        &[
            m("m01", "Regular", &[400.0, 100.0], 400.0),
            m("m02", "Bold", &[900.0, 100.0], 900.0),
            m("m03", "Condensed", &[400.0, 50.0], 400.0),
            m("m04", "SemiBold SemiCondensed", &[650.0, 75.0], 650.0),
        ],
    );
    let mut styles: Vec<Style> = f.masters.iter().zip([400.0, 900.0, 400.0, 650.0]).map(|(mm, w)| Style::weight(&mm.id, w)).collect();
    for (s, wide) in styles.iter_mut().zip([1.0, 1.0, 0.6, 0.8]) {
        s.wide = wide;
    }
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("H", None), ("n", None), ("o", None)]);
    f.instances = Arc::new(vec![instance("Test 650 60", &[650.0, 60.0])]);
    Built::font("m9-F3.glyphs", "M9 § 8", "F3 irregular two axes", f)
        .about("Axes wght and wdth; masters at (400, 100), (900, 100), (400, 50) and (650, 75); space, H, n, o drawn narrower at lower widths; an instance at (650, 60).")
        .checks("M9 check 5")
}

fn m9_f4() -> Built {
    let (mut f, styles) = font(
        "FC Nine Four",
        &[("Weight", "wght")],
        &[m("m01", "Light", &[300.0], 300.0), m("m02", "Regular", &[400.0], 400.0), m("m03", "Bold", &[700.0], 700.0)],
    );
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("H", None), ("n", None), ("o", None)]);
    f.instances = Arc::new(vec![instance("Light", &[300.0]), instance("Regular", &[400.0]), instance("Bold", &[700.0])]);
    Built::font("m9-F4.glyphs", "M9 § 8", "F4 three named masters", f)
        .about("Masters Light (300), Regular (400) and Bold (700), Light first; no Variable Font Origin parameter; space, H, n, o; instances at the masters.")
        .checks("M9 check 36")
}

fn m9_f5() -> Built {
    let (mut f, styles) = wght2("FC Nine Five");
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("H", None), ("O", None)]);
    // O: an extra contour on Black.
    if let Ok(mut g) = f.glyph_mut("O")
        && let Some(l) = g.layer_mut("m02")
    {
        l.shapes.push(fontcraft_engine::model::Shape::Path(rect(300.0, 300.0, 380.0, 380.0)));
    }
    add(&mut f, &styles, "nine.shapes", &[], |s| {
        let mut d = Draw::new(0.0);
        for i in 0..9 {
            let (x, y) = (f64::from(i % 3) * 180.0, f64::from(i / 3) * 220.0);
            d = d.path(rect(x, y, x + s.stem + 40.0, y + s.stem + 40.0));
        }
        d.spaced(50.0, 360.0 + s.stem + 40.0, 50.0)
    });
    for (name, deg) in [("turn19", 19.0f64), ("turn21", 21.0f64)] {
        add(&mut f, &styles, name, &[], |s| {
            let w = 400.0;
            let rise = if s.id == "m02" { w * deg.to_radians().tan() } else { 0.0 };
            Draw::new(0.0).path(poly(&[(0.0, 0.0), (w, 0.0), (w, 500.0 + rise), (0.0, 500.0)])).spaced(50.0, w, 50.0)
        });
    }
    Built::font("m9-F5.glyphs", "M9 § 8", "F5 incompatible", f)
        .about(
            "Masters Thin and Black; O with an extra contour on Black; nine.shapes with nine squares; turn19 and turn21, whose top \
             segment is level in Thin and turns by 19° and 21° in Black; space, H.",
        )
        .checks("M9 checks 26–29")
}

fn m9_f6(ital: bool) -> Built {
    let names = ["Thin", "Light", "Regular", "Bold", "Black"];
    let weights = [100.0, 300.0, 400.0, 700.0, 900.0];
    let (mut f, styles) = if ital {
        font(
            "FC Nine Six Ital",
            &[("Weight", "wght"), ("Italic", "ital")],
            &[
                m("m01", "Thin", &[100.0, 0.0], 100.0),
                m("m02", "Black", &[900.0, 0.0], 900.0),
                m("m03", "Thin Italic", &[100.0, 1.0], 100.0),
                m("m04", "Black Italic", &[900.0, 1.0], 900.0),
            ],
        )
    } else {
        wght2("FC Nine Six")
    };
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("H", None), ("n", None), ("o", None)]);
    let mut instances = Vec::new();
    for (n, w) in names.iter().zip(weights) {
        instances.push(instance(n, &if ital { vec![w, 0.0] } else { vec![w] }));
    }
    for (n, w) in names.iter().zip(weights) {
        let style = if *n == "Regular" { "Italic".to_string() } else { format!("{n} Italic") };
        let mut i = instance(&style, &if ital { vec![w, 1.0] } else { vec![w] });
        i.is_italic = true;
        if *n != "Regular" {
            i.link_style = Some((*n).into());
        }
        instances.push(i);
    }
    f.instances = Arc::new(instances);
    let (file, label, about) = if ital {
        (
            "m9-F6-ital.glyphs",
            "F6 style names (with an ital axis)",
            "Axes wght and ital; masters Thin, Black, Thin Italic, Black Italic; instances Thin, Light, Regular, Bold, Black and their Italics (ital 1, linked to the upright); space, H, n, o.",
        )
    } else {
        (
            "m9-F6.glyphs",
            "F6 style names (no ital axis)",
            "Masters Thin and Black on wght; instances Thin, Light, Regular, Bold, Black and their Italics (italic flag, linked to the upright); space, H, n, o.",
        )
    };
    Built::font(file, "M9 § 8", label, f).about(about).checks("M9 check 37")
}

fn m9_f7(first: bool) -> Built {
    let (mut f, styles) = if first {
        font("FC Nine Seven A", &[("Weight", "wght")], &[m("m01", "Regular", &[400.0], 400.0), m("m02", "Bold", &[700.0], 700.0)])
    } else {
        font("FC Nine Seven B", &[("Weight", "wght")], &[m("m01", "Light", &[300.0], 300.0), m("m02", "Black", &[900.0], 900.0)])
    };
    let set: &[&str] = if first { &["A", "B", "C", "V", "a", "b", "c"] } else { &["A", "B", "D", "V", "a", "b", "d", "o"] };
    add_letter(&mut f, &styles, "space", &[0x20]);
    for n in set {
        add_letter(&mut f, &styles, n, &[n.chars().next().map_or(0, |c| c as u32)]);
    }
    if first {
        groups(&mut f, &["A"], Some("A"), Some("A"));
        groups(&mut f, &["V"], Some("V"), Some("V"));
        kern(&mut f, "m01", "@MMK_L_A", "@MMK_R_V", -50.0);
        kern(&mut f, "m02", "@MMK_L_A", "@MMK_R_V", -70.0);
    } else {
        groups(&mut f, &["A", "V"], Some("diagonal"), Some("diagonal"));
        groups(&mut f, &["b", "o"], Some("round"), Some("round"));
        kern(&mut f, "m01", "@MMK_L_diagonal", "@MMK_R_diagonal", -30.0);
        kern(&mut f, "m02", "@MMK_L_diagonal", "@MMK_R_round", -40.0);
    }
    let (file, label, about) = if first {
        (
            "m9-F7-a.glyphs",
            "F7 two documents (first)",
            "Masters Regular (400) and Bold (700); space, A, B, C, V, a, b, c; groups A and V with an @A/@V pair in each master.",
        )
    } else {
        (
            "m9-F7-b.glyphs",
            "F7 two documents (second)",
            "Masters Light (300) and Black (900); space, A, B, D, V, a, b, d, o; groups `diagonal` (A V) and `round` (b o) with pairs.",
        )
    };
    Built::font(file, "M9 § 8", label, f).about(about).checks("M9 checks 7, 31, 32")
}
