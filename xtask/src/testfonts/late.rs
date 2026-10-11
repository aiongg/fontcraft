//! Test fonts for M10 (hinting), M11 (colour), M12 (checks at export) and M13 (caps, segments,
//! brushes, URLs, instances, scripts).

use std::sync::Arc;

use fontcraft_engine::model::{
    Color, Dimension, Family, Features, Font, GlyphInfo, Image, LayerAttrs, MetricDef, MetricValue, PaletteEntry, Path, PathInfo, PathInfoBox, Shape,
    ShapeAttrs, Snippet, StemDef, Transform,
};

use super::draw::{
    Draw, Pen, Style, add, add_composite, add_letter, add_letters, base_anchors, extra_layer, font, hint, instance, letter, m, open_poly, oval,
    oval_cw, param, poly, push_layer, pv, rect, x_strokes,
};
use super::images::{picture, png, svg_with_png, tiff};
use super::{Built, kern};

pub fn fonts() -> Vec<Built> {
    vec![
        t1(),
        t2(),
        t3(),
        t4(),
        t5(),
        m11_palette(false),
        m11_full_colour(),
        m11_sbix(),
        m11_svg(),
        m11_layered(),
        m11_image(),
        m11_palette(true),
        m12_faults(),
        m12_notdef(),
        m12_mark_width(),
        m12_tall(),
        m12_incompatible(),
        m12_kerning(),
        m12_style_linking(),
        m12_features(),
        m13_cap(),
        m13_segment(),
        m13_brush(),
        m13_url(),
        m13_instances(),
        m13_scripts(),
    ]
}

// ---------------------------------------------------------------------------------------------
// M10

/// `n` with 5-unit cupped serifs at the baseline under both stems.
fn n_serif(s: &Style) -> Draw {
    let d = letter("n", s);
    // `letter` places n's 440-unit body 60 units from the origin.
    let (left, right) = (60.0, 60.0 + 440.0 - s.stem);
    let serif = |x0: f64| {
        let (a, b) = (x0 - 40.0, x0 + s.stem + 40.0);
        let mid = (a + b) / 2.0;
        let q = (b - a) / 6.0;
        Pen::at(a, 0.0)
            .curve((a + q, 0.0), (mid - q, 5.0), (mid, 5.0))
            .curve((mid + q, 5.0), (b - q, 0.0), (b, 0.0))
            .line(b, 30.0)
            .line(a, 30.0)
            .close()
    };
    d.path(serif(left)).path(serif(right))
}

fn t1_font(family: &str, masters: &[(&str, &str, f64)]) -> (Font, Vec<Style>) {
    let axes: &[(&str, &str)] = if masters.len() > 1 { &[("Weight", "wght")] } else { &[] };
    let specs: Vec<_> = masters.iter().map(|(id, name, w)| m(id, name, &if masters.len() > 1 { vec![*w] } else { vec![] }, *w)).collect();
    let (mut f, styles) = font(family, axes, &specs);
    f.stems = Arc::new(vec![
        StemDef { name: "lc".into(), horizontal: true, ..Default::default() },
        StemDef { name: "lc".into(), horizontal: false, ..Default::default() },
    ]);
    for (mm, s) in Arc::make_mut(&mut f.masters).iter_mut().zip(&styles) {
        mm.stem_values = vec![s.bar, s.stem];
    }
    add_letter(&mut f, &styles, "space", &[0x20]);
    for n in ["H", "I", "L", "O", "P", "n", "m", "o", "e", "eight"] {
        let u = match n {
            "eight" => 0x38,
            _ => n.chars().next().map_or(0, |c| c as u32),
        };
        add_letter(&mut f, &styles, n, &[u]);
    }
    add(&mut f, &styles, "n.serif", &[], n_serif);
    (f, styles)
}

const T1_ABOUT: &str = "1,000 units per em; zones baseline 0/−12, x-height 500/12, cap height 700/12, ascender 750/12, descender −250/−12; \
    stems horizontal 70 and vertical 80, both named lc; H, I, L (sans, one contour each), O, P, n, m, o, e, eight (two overlapping \
    rings whose stems sit at different positions), n.serif (5-unit cupped serifs at the baseline), space";

fn t1() -> Built {
    let (f, _) = t1_font("FC Ten T1", &[("m01", "Regular", 400.0)]);
    Built::font("m10-T1.glyphs", "M10 § 8", "T1 hint basics", f).about(&format!("One master. {T1_ABOUT}.")).checks("M10 checks 1–14, 17")
}

fn t2() -> Built {
    let (mut f, styles) = t1_font("FC Ten T2", &[("m01", "Regular", 400.0), ("m02", "Bold", 700.0), ("m03", "Black", 900.0)]);
    // e: outer oval (path 0: top 5, bottom 11), upper counter (path 1) and lower counter (path 2),
    // both clockwise (top 5, bottom 11).
    if let Ok(mut g) = f.glyph_mut("e") {
        for id in ["m02", "m03"] {
            if let Some(l) = g.layer_mut(id) {
                for h in [
                    "{horizontal = 1; origin = (0,11); target = (2,11); type = Stem;}",
                    "{horizontal = 1; origin = (2,5); target = (1,11); type = Stem;}",
                    "{horizontal = 1; origin = (1,5); target = (0,5); type = Stem;}",
                ] {
                    l.info.hints.push(hint(h));
                }
            }
        }
    }
    // I: one vertical stem hint not linked to nodes.
    if let Ok(mut g) = f.glyph_mut("I") {
        for (l, s) in g.layers.iter_mut().zip(&styles) {
            l.info.hints.push(hint(&format!("{{place = (70,{}); type = Stem;}}", s.stem)));
        }
    }
    f.instances =
        Arc::new(vec![instance("Regular", &[400.0]), instance("Medium", &[550.0]), instance("Bold", &[700.0]), instance("Black", &[900.0])]);
    Built::font("m10-T2.glyphs", "M10 § 8", "T2 masters", f)
        .about(&format!(
            "T1 with masters Regular (400), Bold (700) and Black (900) on wght, compatible. {T1_ABOUT}. `e` has manual horizontal stem \
             hints (bottom, bar, top) in Bold and Black only; `I` has one vertical stem hint placed by value, not linked to nodes; \
             instances Regular, Medium (550, halfway between masters 1 and 2), Bold, Black."
        ))
        .checks("M10 checks 15, 16, 18")
}

fn tt_params(f: &mut Font, styles: &[Style]) {
    for (mm, s) in Arc::make_mut(&mut f.masters).iter_mut().zip(styles) {
        mm.params.0.push(param(
            "TTFZones",
            "({name = xHeight; position = 500; size = 12;}, {name = capHeight; position = 700; size = 12;}, \
             {name = ascender; position = 750; size = 12;}, {name = baseline; position = 0; size = -12;}, \
             {name = descender; position = -250; size = -12;})",
        ));
        mm.params.0.push(param(
            "TTFStems",
            &format!(
                "({{horizontal = 1; name = Y1; width = {};}}, {{horizontal = 1; name = Y2; width = {};}}, {{name = X1; width = {};}}, {{name = X2; width = {};}})",
                s.bar,
                (s.bar * 0.6).round(),
                s.stem,
                (s.stem * 0.6).round()
            ),
        ));
    }
}

fn t3() -> Built {
    let (mut f, styles) = t1_font("FC Ten T3", &[("m01", "Regular", 400.0), ("m02", "Bold", 700.0)]);
    tt_params(&mut f, &styles);
    add_letters(&mut f, &styles, &[("A", None), ("acutecomb", Some(0x301))]);
    if let Ok(mut g) = f.glyph_mut("A") {
        g.unicodes = vec![0x41];
    }
    base_anchors(&mut f, &styles, &["A"]);
    add_composite(&mut f, &styles, "Aacute", &[0xC1], "A", &[("acutecomb", "top", false)]);
    f.instances = Arc::new(vec![instance("Regular", &[400.0]), instance("Bold", &[700.0])]);
    Built::font("m10-T3.glyphs", "M10 § 8", "T3 TrueType", f)
        .about(&format!(
            "T1 with masters Regular and Bold on wght. {T1_ABOUT}. Each master has TTFZones (xHeight, capHeight, ascender, baseline, \
             descender) and TTFStems (two Y stems, two X stems); A, acutecomb and the composite Aacute."
        ))
        .checks("M10 checks 19–38, 40–46")
        .gap("The TTFZones and TTFStems values are written as lists of dictionaries (`name`, `position`, `size`; `horizontal`, `name`, `width`). The stem form follows a public sample file; the zone keys are a guess. If the reference app drops them on opening, enter them in its editors.")
        .gap("The plan says `acute`; the combining `acutecomb` is used so the composite attaches by anchors.")
}

fn t4() -> Built {
    let (mut f, styles) = font("FC Ten T4", &[], &[m("m01", "Regular", &[], 400.0)]);
    add_letter(&mut f, &styles, "space", &[0x20]);
    add(&mut f, &styles, "X", &[0x58], |s| Draw::new(0.0).paths(x_strokes(s, 600.0, s.cap, true)).spaced(20.0, 600.0, 20.0));
    Built::font("m10-T4.glyphs", "M10 § 8", "T4 overlap", f)
        .about("One master. X drawn as two overlapping strokes, each with nodes where its edges cross the other stroke; space.")
        .checks("M10 check 39")
}

fn t5() -> Built {
    let (mut f, _) = t1_font("FC Ten T5", &[("m01", "Regular", 400.0)]);
    let stems = Arc::make_mut(&mut f.stems);
    for i in 2..=13 {
        stems.push(StemDef { name: format!("v{i}"), ..Default::default() });
    }
    let metrics = Arc::make_mut(&mut f.metrics);
    let extra = [300.0, 380.0, 560.0, 620.0, 660.0];
    for p in extra {
        metrics.push(MetricDef { kind: String::new(), name: Some(format!("zone {p}")), filter: None, extra: Default::default() });
    }
    for mm in Arc::make_mut(&mut f.masters).iter_mut() {
        for i in 2..=13 {
            mm.stem_values.push(60.0 + 4.0 * f64::from(i));
        }
        for p in extra {
            mm.metric_values.push(MetricValue { pos: p, over: 10.0 });
        }
    }
    Built::font("m10-T5.glyphs", "M10 § 8", "T5 limits", f)
        .about(&format!(
            "T1 ({T1_ABOUT}) with 13 vertical stems (lc and v2–v13) and 8 top zones (x-height, cap height, ascender and five named \
             metrics at 300, 380, 560, 620, 660 with 10-unit overshoots); no blueScale, blueFuzz or blueShift parameter."
        ))
        .checks("M10 checks 9, 10")
}

// ---------------------------------------------------------------------------------------------
// M11

const PALETTES: &str = "(((255,0,0,255),(0,255,0,255),(0,0,255,255)),((0,255,255,255),(255,0,255,255),(255,255,0,255)))";

fn attrs_path(p: Path, attrs: ShapeAttrs) -> Path {
    let mut p = p;
    p.info = PathInfoBox::new(PathInfo { attrs, ..Default::default() });
    p
}

fn palette_attrs(entry: PaletteEntry) -> LayerAttrs {
    LayerAttrs { color_palette: Some(entry), ..Default::default() }
}

fn m11_palette(two_masters: bool) -> Built {
    let (mut f, styles) = if two_masters {
        font("FC Eleven Palette Two", &[("Weight", "wght")], &[m("m01", "Regular", &[400.0], 400.0), m("m02", "Bold", &[700.0], 700.0)])
    } else {
        font("FC Eleven Palette", &[], &[m("m01", "Regular", &[], 400.0)])
    };
    Arc::make_mut(&mut f.info).params.0.push(param("Color Palettes", PALETTES));
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("A", None), ("B", None)]);
    for (g, u) in [("A", 0x41), ("B", 0x42)] {
        if let Ok(mut gl) = f.glyph_mut(g) {
            gl.unicodes = vec![u];
        }
    }
    let shared = |dx: f64| rect(60.0 + dx, 0.0, 260.0 + dx, 700.0);
    for (i, s) in styles.iter().enumerate() {
        // The second master's palette layers move right.
        let dx = if i == 1 { 40.0 } else { 0.0 };
        let mk = |paths: Vec<Path>| Draw::new(700.0).paths(paths);
        push_layer(
            &mut f,
            "A",
            extra_layer(mk(vec![shared(dx)]), &s.id, &format!("A-c0-{}", s.id), "Color 0", palette_attrs(PaletteEntry::Index(0))),
        );
        push_layer(
            &mut f,
            "A",
            extra_layer(
                mk(vec![rect(40.0 + dx, 300.0, 660.0 + dx, 380.0)]),
                &s.id,
                &format!("A-cfg-{}", s.id),
                "Color *",
                palette_attrs(PaletteEntry::Foreground),
            ),
        );
        push_layer(
            &mut f,
            "A",
            extra_layer(
                mk(vec![oval(460.0 + dx, 550.0, 150.0, 150.0)]),
                &s.id,
                &format!("A-c1-{}", s.id),
                "Color 1",
                palette_attrs(PaletteEntry::Index(1)),
            ),
        );
        push_layer(
            &mut f,
            "B",
            extra_layer(mk(vec![shared(dx)]), &s.id, &format!("B-c0-{}", s.id), "Color 0", palette_attrs(PaletteEntry::Index(0))),
        );
    }
    let (file, label, about, checks) = if two_masters {
        (
            "m11-palette-two.glyphs",
            "two-master palette font",
            "The palette font with masters Regular and Bold on wght; Bold's palette layers are moved 40 units right.",
            "M11 export check: variable COLR",
        )
    } else {
        (
            "m11-palette.glyphs",
            "palette font",
            "1,000 units per em, one master; Color Palettes with two palettes of three colours (red, green, blue; cyan, magenta, \
             yellow); A with palette layers Color 0, Color * and Color 1 whose outlines differ (a bar, a crossbar, a disc); B with one \
             Color 0 layer whose outline equals A's Color 0 outline; A and B keep their own outline on the master layer as the fallback.",
            "M11 Color Palettes editor, Layers palette, Font View and COLR/CPAL export checks",
        )
    };
    Built::font(file, "M11 § 8", label, f).about(about).checks(checks)
}

fn m11_full_colour() -> Built {
    let (mut f, styles) = font("FC Eleven Full Colour", &[], &[m("m01", "Regular", &[], 400.0)]);
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("C", Some(0x43)), ("D", Some(0x44))]);
    let full = || LayerAttrs { full_color: true, ..Default::default() };
    let solid = attrs_path(rect(40.0, 0.0, 300.0, 700.0), ShapeAttrs { fill_color: Some(Color::Rgba([1.0, 0.0, 0.0, 1.0])), ..Default::default() });
    let mut lin = ShapeAttrs::default();
    lin.extra.insert(Family::Glyphs, "gradient", pv("{colors = (((255,0,0,255),0),((0,0,255,255),1)); start = (0,0); end = (1,1);}"));
    let linear = attrs_path(rect(320.0, 0.0, 600.0, 700.0), lin);
    let stroke = attrs_path(
        open_poly(&[(60.0, 760.0), (320.0, 900.0), (580.0, 760.0)]),
        ShapeAttrs { stroke_width: Some(Dimension::Number(20.0)), stroke_color: Some(Color::Rgba([0.0, 0.5, 0.0, 1.0])), ..Default::default() },
    );
    push_layer(&mut f, "C", extra_layer(Draw::new(640.0).path(solid).path(linear).path(stroke), "m01", "C-color", "Color", full()));
    let mut rad = ShapeAttrs::default();
    rad.extra.insert(
        Family::Glyphs,
        "gradient",
        pv("{colors = (((255,255,0,255),0),((255,0,255,255),1)); start = (0.5,0.5); end = (1,0.5); type = circle;}"),
    );
    push_layer(&mut f, "D", extra_layer(Draw::new(700.0).path(attrs_path(oval(350.0, 350.0, 300.0, 350.0), rad)), "m01", "D-color", "Color", full()));
    let _ = &styles;
    Built::font("m11-full-colour.glyphs", "M11 § 8", "full-colour font", f)
        .about(
            "One master. C with a Color layer holding three paths: a solid red fill, a linear gradient (red to blue, corner to corner) \
             and an open path with a 20-unit green stroke; D with a Color layer holding a radial gradient (yellow to magenta); master \
             layers keep plain outlines.",
        )
        .checks("M11 Attributes inspector and full-colour export checks")
}

fn sbix_layer(master: &str, id: &str, size: u16, image: Option<&str>) -> fontcraft_engine::model::Layer {
    let mut l = extra_layer(Draw::new(600.0), master, id, &format!("iColor {size}"), LayerAttrs { sbix_size: Some(size), ..Default::default() });
    if let Some(p) = image {
        l.info.image = Some(Image { path: Some(p.into()), ..Default::default() });
    }
    l
}

fn m11_sbix() -> Built {
    let (mut f, styles) = font("FC Eleven Sbix", &[], &[m("m01", "Regular", &[], 400.0)]);
    add_letter(&mut f, &styles, "space", &[0x20]);
    let corners = |_: &Style| {
        Draw::new(600.0)
            .path(Path::new(false, vec![fontcraft_engine::model::Node::new(50.0, -100.0, fontcraft_engine::model::NodeKind::Line)]))
            .path(Path::new(false, vec![fontcraft_engine::model::Node::new(550.0, 700.0, fontcraft_engine::model::NodeKind::Line)]))
    };
    for (n, u) in [("E", Some(0x45)), ("E.tif", None), ("E.wide", None), ("E.missing", None)] {
        add(&mut f, &styles, n, &u.map(|u| vec![u]).unwrap_or_default(), corners);
    }
    let dir = "m11-sbix-images";
    push_layer(&mut f, "E", sbix_layer("m01", "E-64", 64, None));
    push_layer(&mut f, "E", sbix_layer("m01", "E-128", 128, Some(&format!("{dir}/E 128.png"))));
    push_layer(&mut f, "E.tif", sbix_layer("m01", "Etif-64", 64, Some(&format!("{dir}/E 64.tif"))));
    push_layer(&mut f, "E.wide", sbix_layer("m01", "Ewide-64", 64, Some(&format!("{dir}/E wide.png"))));
    push_layer(&mut f, "E.missing", sbix_layer("m01", "Emissing-64", 64, Some(&format!("{dir}/missing.png"))));
    let red = [220, 30, 30];
    Built::font("m11-sbix.glyphs", "M11 § 8", "sbix font", f)
        .about(
            "One master. E whose master layer has two zero-size corner paths (one node each) at (50, −100) and (550, 700), an empty \
             iColor 64 layer (drop `E 64.png` on it) and an iColor 128 layer placing `E 128.png`; E.tif, E.wide and E.missing with \
             iColor 64 layers placing the TIFF, the non-square PNG and a file that does not exist. Images in `m11-sbix-images/`: \
             `E 64.png` (64 × 64), `E 128.png` (128 × 128), `E 64.tif` (the same picture), `E wide.png` (96 × 64).",
        )
        .checks("M11 iColor layer and sbix export checks")
        .gap("No JPEG: the generator has no JPEG encoder. Convert `E 64.png` with any image tool for the graphic-type check.")
        .gap("The iColor layers place their image as the layer's image (`backgroundImage`); whether the reference app stores sbix pictures that way is not confirmed.")
        .file(&format!("{dir}/E 64.png"), png(64, 64, &picture(64, 64, red)))
        .file(&format!("{dir}/E 128.png"), png(128, 128, &picture(128, 128, red)))
        .file(&format!("{dir}/E 64.tif"), tiff(64, 64, &picture(64, 64, red)))
        .file(&format!("{dir}/E wide.png"), png(96, 64, &picture(96, 64, red)))
}

fn m11_svg() -> Built {
    let (mut f, styles) = font("FC Eleven SVG", &[], &[m("m01", "Regular", &[], 400.0)]);
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("F", Some(0x46))]);
    let mut l = extra_layer(Draw::new(600.0), "m01", "F-svg", "svg", LayerAttrs { svg: true, ..Default::default() });
    l.info.image = Some(Image { path: Some("Images/F.svg".into()), ..Default::default() });
    push_layer(&mut f, "F", l);
    Built::font("m11-svg.glyphs", "M11 § 8", "SVG font", f)
        .about("One master. F with an svg layer placing `Images/F.svg` (a 100 × 100 viewBox with a linear gradient and an embedded PNG); space.")
        .checks("M11 svg layer and SVG table checks")
        .gap("The svg layer places the file as the layer's image (`backgroundImage`); how the reference app links an svg layer to its file is not confirmed.")
        .file("Images/F.svg", svg_with_png())
}

fn m11_layered() -> Built {
    let (mut f, styles) = font(
        "FC Eleven Layered",
        &[("Color", "COLR")],
        &[m("m01", "Red", &[1.0], 400.0), m("m02", "Green", &[2.0], 400.0), m("m03", "Blue", &[3.0], 400.0)],
    );
    for (mm, c) in Arc::make_mut(&mut f.masters).iter_mut().zip(["(255,0,0,255)", "(0,255,0,128)", "(0,0,255,255)"]) {
        mm.params.0.push(param("Master Color", c));
    }
    Arc::make_mut(&mut f.info).params.0.push(param("Enforce Compatibility Check", "0"));
    add_letter(&mut f, &styles, "space", &[0x20]);
    add(&mut f, &styles, "G", &[0x47], |s| match s.id.as_str() {
        "m01" => Draw::new(700.0).path(rect(50.0, 0.0, 650.0, 700.0)),
        "m02" => Draw::new(700.0).path(oval(350.0, 350.0, 250.0, 250.0)),
        _ => Draw::new(700.0).path(poly(&[(100.0, 100.0), (600.0, 100.0), (350.0, 600.0)])),
    });
    Built::font("m11-layered.glyphs", "M11 § 8", "layered font", f)
        .about(
            "A Color axis (tag COLR); masters Red, Green and Blue at 1, 2, 3 with Master Color red, green at 50 % opacity, and blue; \
             Enforce Compatibility Check off; G is a square, a disc and a triangle in the three masters (incompatible on purpose); space.",
        )
        .checks("M11 layered-font checks (stacking, Master Color, Color Layers to COLR and SVG)")
        .gap("The axis is named Color with tag COLR and no font type is set; the plan does not say which axis tag or font type the reference app expects for a layered font.")
}

fn m11_image() -> Built {
    let (mut f, styles) = font("FC Eleven Image", &[], &[m("m01", "Regular", &[], 400.0)]);
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("A", Some(0x41)), ("H", Some(0x48)), ("I", Some(0x49)), ("a", Some(0x61))]);
    let dir = "m11-image-images";
    let place = |p: &str| Image {
        path: Some(format!("{dir}/{p}")),
        transform: Transform::Decomposed { pos: (50.0, 0.0), scale: (1.0, 1.0), angle: 0.0, slant: (0.0, 0.0) },
        ..Default::default()
    };
    if let Ok(mut g) = f.glyph_mut("H")
        && let Some(l) = g.layer_mut("m01")
    {
        l.info.image = Some(place("H.png"));
    }
    if let Ok(mut g) = f.glyph_mut("I")
        && let Some(l) = g.layer_mut("m01")
    {
        l.shapes.clear();
        l.info.image = Some(place("I.png"));
    }
    let img = |rgb| png(100, 150, &picture(100, 150, rgb));
    Built::font("m11-image.glyphs", "M11 § 8", "image font", f)
        .about(
            "One master. H with its outline and a placed `H.png`; I empty with a placed `I.png`; A and a (for the case check); space. \
             `m11-image-images/` holds H.png, I.png, J.png (no glyph J) and a.png, each 100 × 150.",
        )
        .checks("M11 image checks in Edit View and Font View")
        .gap("No PDF or PSD file for the supported-formats check.")
        .file(&format!("{dir}/H.png"), img([30, 120, 220]))
        .file(&format!("{dir}/I.png"), img([30, 180, 60]))
        .file(&format!("{dir}/J.png"), img([200, 120, 30]))
        .file(&format!("{dir}/a.png"), img([150, 30, 150]))
}

// ---------------------------------------------------------------------------------------------
// M12

fn one(family: &str) -> (Font, Vec<Style>) {
    font(family, &[], &[m("m01", "Regular", &[], 400.0)])
}

fn m12_faults() -> Built {
    let (mut f, styles) = one("FC Twelve Faults");
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("A", Some(0x41)), ("A.dup", Some(0x41)), ("B", Some(0x42))]);
    add(&mut f, &styles, "l.open", &[], |s| Draw::new(0.0).path(open_poly(&[(0.0, 0.0), (0.0, s.asc), (s.stem, s.asc)])).spaced(80.0, s.stem, 80.0));
    add(&mut f, &styles, "bad name", &[], |s| letter("H", s));
    add(&mut f, &styles, "broken", &[], |_| Draw::new(600.0).comp("missingglyph", 0.0, 0.0));
    add(&mut f, &styles, "cycle.one", &[], |_| Draw::new(600.0).comp("cycle.two", 0.0, 0.0));
    add(&mut f, &styles, "cycle.two", &[], |_| Draw::new(600.0).comp("cycle.one", 0.0, 0.0));
    let long = format!("long{}", "x".repeat(66));
    add(&mut f, &styles, &long, &[], |s| letter("B", s));
    Built::font("m12-faults.glyphs", "M12 § 8", "check 1: one fault per glyph", f)
        .about(
            "One master. l.open (an open path), `bad name` (a space in the name), A and A.dup (both U+0041), broken (a component of the \
             missing glyph missingglyph), cycle.one and cycle.two (components of each other), a glyph whose name is 70 characters \
             (`long` and 66 x), plus space and B.",
        )
        .checks("M12 check 1")
}

fn m12_notdef() -> Built {
    let (mut f, styles) = one("FC Twelve Notdef");
    add_letters(&mut f, &styles, &[("A", Some(0x41)), ("B", Some(0x42)), ("space", Some(0x20))]);
    Built::font("m12-notdef.glyphs", "M12 § 8", "check 2: no .notdef", f)
        .about("One master. A, B and space, in that order; no .notdef.")
        .checks("M12 check 2")
}

fn m12_mark_width() -> Built {
    let (mut f, styles) = one("FC Twelve Mark Width");
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("a", Some(0x61)), ("acutecomb", Some(0x301))]);
    base_anchors(&mut f, &styles, &["a"]);
    add_composite(&mut f, &styles, "aacute", &[0xE1], "a", &[("acutecomb", "top", false)]);
    Built::font("m12-mark-width.glyphs", "M12 § 8", "check 3: mark width", f)
        .about("One master. acutecomb (U+0301) with width 200, a, aacute (composite), space.")
        .checks("M12 check 3")
}

fn m12_tall() -> Built {
    let (mut f, styles) = one("FC Twelve Tall");
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("A", Some(0x41))]);
    add(&mut f, &styles, "Aringacute", &[0x1FA], |s| {
        let a = letter("A", s);
        let cx = a.width / 2.0;
        a.path(oval(cx, 830.0, 60.0, 60.0)).path(oval_cw(cx, 830.0, 30.0, 30.0)).path(poly(&[
            (cx - 30.0, 920.0),
            (cx + 10.0, 920.0),
            (cx + 90.0, 1042.0),
            (cx + 50.0, 1042.0),
        ]))
    });
    Built::font("m12-tall.glyphs", "M12 § 8", "check 4: tall glyph", f)
        .about("One master; ascender 750, descender −250. A, space, and Aringacute (outlines) whose acute reaches 1,042.")
        .checks("M12 check 4")
}

fn m12_incompatible() -> Built {
    let (mut f, styles) =
        font("FC Twelve Incompatible", &[("Weight", "wght")], &[m("m01", "Regular", &[400.0], 400.0), m("m02", "Bold", &[700.0], 700.0)]);
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("O", Some(0x4F)), ("n", Some(0x6E))]);
    if let Ok(mut g) = f.glyph_mut("O")
        && let Some(l) = g.layer_mut("m02")
    {
        l.shapes.truncate(1);
    }
    if let Ok(mut g) = f.glyph_mut("n")
        && let Some(l) = g.layer_mut("m02")
        && let Some(Shape::Path(p)) = l.shapes.first_mut()
    {
        // An extra node halfway along the bottom of the left stem.
        let (a, b) = (p.nodes[0].clone(), p.nodes[p.nodes.len() - 1].clone());
        let mut mid = fontcraft_engine::model::Node::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0, fontcraft_engine::model::NodeKind::Line);
        mid.smooth = false;
        p.nodes.insert(0, mid);
    }
    Built::font("m12-incompatible.glyphs", "M12 § 8", "check 5: incompatible masters", f)
        .about("Masters Regular and Bold on wght. O has two paths in Regular and one in Bold; n has one node more in Bold; space.")
        .checks("M12 check 5")
}

fn m12_kerning() -> Built {
    let (mut f, styles) = one("FC Twelve Kerning");
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("A", Some(0x41)), ("V", Some(0x56))]);
    kern(&mut f, "m01", "A", "V", -80.0);
    kern(&mut f, "m01", "V", "A", -80.0);
    let _ = f.remove_glyph("V");
    Built::font("m12-kerning.glyphs", "M12 § 8", "check 6: kerning on a deleted glyph", f)
        .about("One master. A and space; pairs A/V and V/A kerned −80, with the glyph V removed afterwards.")
        .checks("M12 check 6")
}

fn m12_style_linking() -> Built {
    let (mut f, styles) = font("FC Twelve Linking", &[("Weight", "wght")], &[m("m01", "Light", &[300.0], 300.0), m("m02", "Bold", &[700.0], 700.0)]);
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("H", Some(0x48)), ("n", Some(0x6E)), ("o", Some(0x6F))]);
    let mut out = Vec::new();
    for (n, w) in [("Light", 300.0), ("Regular", 400.0), ("Semibold", 600.0), ("Bold", 700.0)] {
        let mut up = instance(n, &[w]);
        if n == "Bold" {
            up.is_bold = true;
        }
        out.push(up);
        let mut it = instance(&if n == "Regular" { "Italic".to_string() } else { format!("{n} Italic") }, &[w]);
        it.is_italic = true;
        if n == "Bold" {
            it.is_bold = true;
        }
        if n != "Regular" && n != "Bold" {
            it.link_style = Some(n.into());
        }
        out.push(it);
    }
    f.instances = Arc::new(out);
    Built::font("m12-style-linking.glyphs", "M12 § 8", "check 7: style linking", f)
        .about("Masters Light (300) and Bold (700); instances Light, Regular, Semibold, Bold, each with an Italic (italic flag; Bold and Bold Italic also bold); space, H, n, o.")
        .checks("M12 check 7")
}

fn m12_features() -> Built {
    let (mut f, styles) = one("FC Twelve Features");
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("f", Some(0x66)), ("i", Some(0x69))]);
    f.features = Arc::new(Features {
        features: vec![Snippet { name: "liga".into(), code: "sub f i by f_i;\n".into(), ..Default::default() }],
        ..Default::default()
    });
    Built::font("m12-features.glyphs", "M12 § 8", "check 8: undefined glyph in features", f)
        .about("One master. f, i, space; a liga feature `sub f i by f_i;` with no f_i glyph. Any font serves check 9.")
        .checks("M12 checks 8, 9")
}

// ---------------------------------------------------------------------------------------------
// M13

fn m13_cap() -> Built {
    let (mut f, styles) = font("FC Thirteen Cap", &[("Weight", "wght")], &[m("m01", "Regular", &[400.0], 400.0), m("m02", "Bold", &[700.0], 700.0)]);
    add_letter(&mut f, &styles, "space", &[0x20]);
    add(&mut f, &styles, "_cap.flag", &[], |_| {
        Draw::new(0.0).path(open_poly(&[(-40.0, 0.0), (-40.0, 60.0), (40.0, 60.0), (40.0, 0.0)])).anchor("origin", 0.0, 0.0)
    });
    add(&mut f, &styles, "_cap.round", &[], |_| {
        Draw::new(0.0).path(Pen::at(-25.0, 0.0).arc_vh((0.0, 25.0)).arc_hv((25.0, 0.0)).open()).anchor("node1", -25.0, 0.0).anchor("node2", 25.0, 0.0)
    });
    add(&mut f, &styles, "l", &[0x6C], |s| {
        let st = if s.id == "m02" { 120.0 } else { 80.0 };
        Draw::new(0.0).path(rect(0.0, 0.0, st, s.asc)).spaced(80.0, st, 80.0)
    });
    add(&mut f, &styles, "i", &[0x69], |s| {
        Draw::new(0.0).path(rect(0.0, 0.0, 120.0, s.xh)).path(rect(0.0, s.xh + 80.0, 120.0, s.xh + 200.0)).spaced(80.0, 120.0, 80.0)
    });
    add(&mut f, &styles, "slash", &[0x2F], |s| {
        Draw::new(0.0).path(poly(&[(0.0, -100.0), (s.stem, -100.0), (320.0, s.cap), (320.0 - s.stem, s.cap)])).spaced(30.0, 320.0, 30.0)
    });
    Built::font("m13-cap.glyphs", "M13 § 8", "Cap test font", f)
        .about(
            "Masters Regular and Bold. `_cap.flag` (an `origin` anchor and a 4-node open path), `_cap.round` (`node1` and `node2` \
             anchors 50 units apart, an open half-round path); l with an 80-unit stem (120 in Bold), i with a 120-unit stem, slash with \
             a slanted stem; space.",
        )
        .checks("M13 cap checks")
}

fn m13_segment() -> Built {
    let (mut f, styles) = one("FC Thirteen Segment");
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("o", Some(0x6F))]);
    add(&mut f, &styles, "_segment.wave", &[], |_| {
        Draw::new(300.0)
            .path(Pen::at(0.0, 0.0).curve((50.0, 60.0), (100.0, 60.0), (150.0, 0.0)).curve((200.0, -60.0), (250.0, -60.0), (300.0, 0.0)).open())
            .anchor("start", 0.0, 0.0)
            .anchor("end", 300.0, 0.0)
    });
    add(&mut f, &styles, "square", &[0x25A0], |_| Draw::new(0.0).path(rect(0.0, 0.0, 500.0, 500.0)).spaced(50.0, 500.0, 50.0));
    Built::font("m13-segment.glyphs", "M13 § 8", "Segment test font", f)
        .about("One master. `_segment.wave` (a 3-node open path on the baseline, `start` and `end` anchors on its end nodes), square, o, space.")
        .checks("M13 segment checks")
}

fn m13_brush() -> Built {
    let (mut f, styles) = one("FC Thirteen Brush");
    add_letter(&mut f, &styles, "space", &[0x20]);
    add(&mut f, &styles, "_brush.arrow", &[], |_| {
        Draw::new(0.0)
            .path(poly(&[(0.0, -20.0), (200.0, -20.0), (200.0, -60.0), (300.0, 0.0), (200.0, 60.0), (200.0, 20.0), (0.0, 20.0)]))
            .anchor("start", 0.0, 0.0)
            .anchor("end", 200.0, 0.0)
    });
    add(&mut f, &styles, "_brush.bar", &[], |_| {
        Draw::new(0.0).path(rect(0.0, -30.0, 200.0, 30.0)).anchor("start", 0.0, 0.0).anchor("end", 200.0, 0.0)
    });
    add(&mut f, &styles, "line.straight", &[], |_| Draw::new(600.0).path(open_poly(&[(50.0, 100.0), (550.0, 600.0)])));
    add(&mut f, &styles, "line.curved", &[], |_| {
        Draw::new(600.0).path(Pen::at(50.0, 100.0).curve((50.0, 500.0), (300.0, 650.0), (550.0, 600.0)).open())
    });
    add(&mut f, &styles, "line.three", &[], |_| Draw::new(600.0).path(open_poly(&[(50.0, 100.0), (250.0, 500.0), (400.0, 150.0), (550.0, 600.0)])));
    Built::font("m13-brush.glyphs", "M13 § 8", "Brush test font", f)
        .about(
            "One master. `_brush.arrow` (one closed arrow outline, `start` at the tail, `end` where the head begins), `_brush.bar` (line \
             segments only); line.straight, line.curved (open paths) and line.three (a 3-segment open path); space.",
        )
        .checks("M13 brush checks")
}

fn m13_url() -> Built {
    let (mut f, styles) = font("FC Thirteen URL", &[("Weight", "wght")], &[m("m01", "Regular", &[400.0], 400.0), m("m02", "Bold", &[700.0], 700.0)]);
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("A", Some(0x41)), ("B", Some(0x42)), ("G", Some(0x47)), ("brevecomb", Some(0x306))]);
    base_anchors(&mut f, &styles, &["G"]);
    add_composite(&mut f, &styles, "Gbreve", &[0x11E], "G", &[("brevecomb", "top", false)]);
    if let Ok(mut g) = f.glyph_mut("Gbreve") {
        g.info = fontcraft_engine::model::GlyphInfoBox::new(GlyphInfo { production: Some("uni011E".into()), ..Default::default() });
    }
    Built::font("m13 url test/m13-url.glyphs", "M13 § 8", "URL test font", f)
        .about("Masters Regular and Bold. A, B, G, brevecomb and Gbreve (production name uni011E), space; saved in the folder `m13 url test`, whose name has a space.")
        .checks("M13 URL checks")
}

fn m13_instances() -> Built {
    let (mut f, styles) =
        font("FC Thirteen Instances", &[("Weight", "wght")], &[m("m01", "Light", &[300.0], 300.0), m("m02", "Bold", &[700.0], 700.0)]);
    add_letters(&mut f, &styles, &[("space", Some(0x20)), ("H", Some(0x48)), ("n", Some(0x6E)), ("o", Some(0x6F))]);
    let mut reg = instance("Regular", &[400.0]);
    reg.params.0.push(param("Name Table Entry", "\"10; Regular instance\""));
    f.instances = Arc::new(vec![instance("Light", &[300.0]), reg, instance("Bold", &[700.0])]);
    Built::font("m13-instances.glyphs", "M13 § 8", "Instances test font", f)
        .about("Masters Light and Bold; instances Light, Regular (with a Name Table Entry parameter) and Bold; space, H, n, o.")
        .checks("M13 instance checks")
}

fn m13_scripts() -> Built {
    let script = |title: Option<&str>, doc: &str, say: &str| {
        let mut s = String::new();
        if let Some(t) = title {
            s.push_str(&format!("# MenuTitle: {t}\n"));
        }
        s.push_str(&format!("# -*- coding: utf-8 -*-\n__doc__ = \"\"\"{doc}\"\"\"\n\nprint(\"{say}\")\n"));
        s.into_bytes()
    };
    Built::material("m13-scripts/", "M13 § 8", "Scripts")
        .about(
            "Four short scripts of our own for the Scripts folder: b.py titled Alpha with a two-line `__doc__`, a.py titled Zulu, c.py \
             without a MenuTitle, and Tools/d.py titled Delta. Each prints one line.",
        )
        .checks("M13 Script menu checks 1–6")
        .gap("Plug-ins for the Plugin Manager checks come from the reference app's own manager; nothing is generated for them.")
        .file("m13-scripts/b.py", script(Some("Alpha"), "Prints a greeting.\nThe second line of the description.", "Alpha ran"))
        .file("m13-scripts/a.py", script(Some("Zulu"), "Prints Zulu.", "Zulu ran"))
        .file("m13-scripts/c.py", script(None, "A script without a menu title.", "c ran"))
        .file("m13-scripts/Tools/d.py", script(Some("Delta"), "A script in a subfolder.", "Delta ran"))
}
