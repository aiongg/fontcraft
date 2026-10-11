//! Test fonts for the built milestones (M3 to M5), M6 and M7.

use std::sync::Arc;

use fontcraft_engine::model::{
    ComponentInfo, ComponentInfoBox, Font, Glyph, GlyphInfo, GlyphInfoBox, Guide, Layer, LayerAttrs, LayerInfo, LayerInfoBox, PartProperty, Shape,
    SmartPole, SmartValue, corner::CornerHint,
};

use super::draw::{Draw, Pen, Style, add, add_composite, add_letter, add_letters, base_anchors, font, instance, letter, m, open_poly, param, rect};
use super::{Built, groups, kern, keys};

pub fn fonts() -> Vec<Built> {
    vec![general(), m6(), m7_one(), m7_two(), m7_export()]
}

fn latin_basic(f: &mut Font, styles: &[Style]) {
    add_letter(f, styles, "space", &[0x20]);
    let letters: Vec<String> = ('A'..='Z').chain('a'..='z').map(String::from).collect();
    for l in &letters {
        add_letter(f, styles, l, &[l.chars().next().map_or(0, |c| c as u32)]);
    }
    for (i, d) in ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"].iter().enumerate() {
        add_letter(f, styles, d, &[0x30 + i as u32]);
    }
    add_letters(f, styles, &[("period", Some(0x2E)), ("comma", Some(0x2C)), ("hyphen", Some(0x2D)), ("slash", Some(0x2F))]);
}

fn marks(f: &mut Font, styles: &[Style], names: &[(&str, u32)]) {
    for (n, u) in names {
        add_letter(f, styles, n, &[*u]);
    }
}

/// M3 to M5: drawing, Font View, spacing and kerning.
fn general() -> Built {
    let (mut f, styles) = font("FC General", &[("Weight", "wght")], &[m("m01", "Regular", &[400.0], 400.0), m("m02", "Bold", &[700.0], 700.0)]);
    latin_basic(&mut f, &styles);
    marks(&mut f, &styles, &[("gravecomb", 0x300), ("acutecomb", 0x301), ("dieresiscomb", 0x308), ("ringcomb", 0x30A)]);
    base_anchors(&mut f, &styles, &["A", "E", "O", "a", "e", "o", "u"]);
    for (name, u, base, mark) in [
        ("Agrave", 0xC0, "A", "gravecomb"),
        ("Aacute", 0xC1, "A", "acutecomb"),
        ("Adieresis", 0xC4, "A", "dieresiscomb"),
        ("Aring", 0xC5, "A", "ringcomb"),
        ("Eacute", 0xC9, "E", "acutecomb"),
        ("Odieresis", 0xD6, "O", "dieresiscomb"),
        ("aacute", 0xE1, "a", "acutecomb"),
        ("eacute", 0xE9, "e", "acutecomb"),
        ("odieresis", 0xF6, "o", "dieresiscomb"),
        ("udieresis", 0xFC, "u", "dieresiscomb"),
    ] {
        add_composite(&mut f, &styles, name, &[u], base, &[(mark, "top", false)]);
    }
    // Open paths: a wave and a vertical line.
    add(&mut f, &styles, "asciitilde", &[0x7E], |s| {
        let y = s.xh / 2.0;
        Draw::new(0.0)
            .path(
                Pen::at(0.0, y - 20.0)
                    .curve((60.0, y + 60.0), (120.0, y + 60.0), (180.0, y))
                    .curve((240.0, y - 60.0), (300.0, y - 60.0), (360.0, y + 20.0))
                    .open(),
            )
            .spaced(50.0, 360.0, 50.0)
    });
    add(&mut f, &styles, "bar", &[0x7C], |s| Draw::new(0.0).path(open_poly(&[(0.0, s.desc), (0.0, s.asc)])).spaced(120.0, 0.0, 120.0));

    groups(&mut f, &["A", "Aacute", "Adieresis", "Agrave", "Aring"], Some("A"), Some("A"));
    groups(&mut f, &["V", "W", "Y"], Some("V"), Some("V"));
    groups(&mut f, &["O", "Q", "C", "G", "Odieresis"], Some("O"), None);
    groups(&mut f, &["O", "Q", "D", "Odieresis"], None, Some("O"));
    groups(&mut f, &["T"], Some("T"), Some("T"));
    groups(&mut f, &["o", "e", "c", "eacute", "odieresis"], Some("o"), Some("o"));
    groups(&mut f, &["n", "m", "r"], Some("n"), None);
    for (mi, scale) in [("m01", 1.0), ("m02", 1.25)] {
        let k = |v: f64| (v * scale).round();
        kern(&mut f, mi, "@MMK_L_A", "@MMK_R_V", k(-60.0));
        kern(&mut f, mi, "@MMK_L_V", "@MMK_R_A", k(-60.0));
        kern(&mut f, mi, "@MMK_L_A", "@MMK_R_O", k(-20.0));
        kern(&mut f, mi, "@MMK_L_T", "@MMK_R_o", k(-80.0));
        kern(&mut f, mi, "@MMK_L_T", "@MMK_R_A", k(-50.0));
        kern(&mut f, mi, "@MMK_L_V", "@MMK_R_o", k(-50.0));
        kern(&mut f, mi, "@MMK_L_V", "period", k(-80.0));
        // Exceptions: a glyph against a group, and a glyph pair where both sides have groups.
        kern(&mut f, mi, "Aacute", "@MMK_R_V", k(-30.0));
        kern(&mut f, mi, "A", "Y", k(-90.0));
        kern(&mut f, mi, "V", "odieresis", k(-20.0));
        // Glyphs without groups.
        kern(&mut f, mi, "L", "@MMK_R_V", k(-70.0));
        kern(&mut f, mi, "F", "comma", k(-60.0));
    }
    // A pair in one master only.
    kern(&mut f, "m01", "L", "T", -80.0);

    for n in ["B", "D", "E", "F", "P", "R", "K", "N"] {
        keys(&mut f, n, Some("=H"), None, None);
    }
    keys(&mut f, "Q", Some("=O"), Some("=O"), None);
    keys(&mut f, "C", Some("=O"), None, None);
    keys(&mut f, "G", Some("=O"), None, None);
    keys(&mut f, "m", Some("=n"), Some("=n"), None);
    keys(&mut f, "h", Some("=n"), Some("=n"), None);
    keys(&mut f, "u", Some("=|n"), Some("=|n"), None);
    keys(&mut f, "d", None, Some("=l"), None);
    keys(&mut f, "c", Some("=o"), None, None);
    keys(&mut f, "e", None, Some("=o-10"), None);
    keys(&mut f, "b", Some("=l+5"), None, None);
    for d in ["one", "two", "three", "four", "five", "six", "seven", "eight", "nine"] {
        keys(&mut f, d, None, None, Some("=zero"));
    }
    // A key on the Bold layer of I only.
    if let Ok(mut g) = f.glyph_mut("I")
        && let Some(l) = g.layer_mut("m02")
    {
        l.info.metrics_keys.left = Some("=H+5".into());
    }
    // A guide on the Regular master, and one in H.
    if let Some(mm) = Arc::make_mut(&mut f.masters).first_mut() {
        mm.guides.push(Guide { x: 0.0, y: 350.0, name: Some("middle".into()), ..Default::default() });
    }
    if let Ok(mut g) = f.glyph_mut("H")
        && let Some(l) = g.layer_mut("m01")
    {
        l.info.guides.push(Guide { x: 100.0, y: 0.0, angle: 90.0, ..Default::default() });
    }
    f.instances = Arc::new(vec![instance("Regular", &[400.0]), instance("Medium", &[550.0]), instance("Bold", &[700.0])]);
    Built::font("m35-general.glyphs", "M3–M5 (`docs/observe.md`)", "general Latin font", f)
        .about(
            "Masters Regular (wght 400) and Bold (700); instances Regular, Medium, Bold. A–Z, a–z, zero–nine, space, period, comma, hyphen, \
             slash; gravecomb, acutecomb, dieresiscomb, ringcomb; composites Agrave Aacute Adieresis Aring Eacute Odieresis aacute eacute \
             odieresis udieresis. Overlapping contours in A (crossbar), Q (tail), W (two strokes), X (two strokes) and eight (two rings). \
             Open paths: asciitilde (curves) and bar (a line). Kerning groups A, V (V W Y), O, T, o, n; group pairs, glyph-to-group and \
             glyph-pair exceptions (Aacute/@V, A/Y, V/odieresis), pairs of ungrouped glyphs, and L/T in Regular only. Metrics keys: =H, \
             =O, =n, =|n, =l, =o, =o-10, =l+5, width =zero on the figures, and =H+5 on the Bold layer of I only. A master guide at y 350 \
             and a glyph guide in H.",
        )
        .checks("Edit View drawing, Font View, spacing, metrics keys and kerning rows of M3, M4 and M5")
}

/// M6 § 5's end-to-end font, with flipped marks and `#exit`/`#entry` parts.
fn m6() -> Built {
    let (mut f, styles) = font("FC Components", &[], &[m("m01", "Regular", &[], 400.0)]);
    add_letter(&mut f, &styles, "space", &[0x20]);
    for (n, u) in [("A", 0x41), ("E", 0x45), ("n", 0x6E), ("o", 0x6F), ("idotless", 0x131)] {
        add_letter(&mut f, &styles, n, &[u]);
    }
    marks(&mut f, &styles, &[("gravecomb", 0x300), ("acutecomb", 0x301), ("dieresiscomb", 0x308), ("ringcomb", 0x30A)]);
    // E and o are left without anchors, for Set Anchors.
    base_anchors(&mut f, &styles, &["A", "n"]);
    if let Ok(mut g) = f.glyph_mut("idotless") {
        for (l, s) in g.layers.iter_mut().zip(&styles) {
            l.anchors.push(fontcraft_engine::model::Anchor::new("top", (l.width / 2.0).round(), s.xh));
        }
    }
    add_composite(&mut f, &styles, "Aacute", &[0xC1], "A", &[("acutecomb", "top", false)]);
    add_composite(&mut f, &styles, "Adieresis", &[0xC4], "A", &[("dieresiscomb", "top", false)]);
    add_composite(&mut f, &styles, "ntilde.stack", &[], "n", &[("dieresiscomb", "top", false), ("acutecomb", "top", false)]);
    add_composite(&mut f, &styles, "odieresis", &[0xF6], "o", &[("dieresiscomb", "top", false)]);
    // Flipped marks: a ring and an acute mirrored below the base.
    add_composite(&mut f, &styles, "Aringbelow", &[0x1E00], "A", &[("ringcomb", "bottom", true)]);
    add_composite(&mut f, &styles, "n.acutebelow", &[], "n", &[("acutecomb", "bottom", true)]);

    // A smart glyph with a Width property, and a glyph using it.
    let mut part = Glyph::new("_part.bar");
    part.info = GlyphInfoBox::new(GlyphInfo {
        parts: vec![PartProperty {
            name: "Width".into(),
            bottom_value: 100.0,
            top_value: 400.0,
            bottom_name: None,
            top_name: None,
            extra: Default::default(),
        }],
        ..Default::default()
    });
    for (id, top, w) in [("m01", false, 100.0), ("bar-wide", true, 400.0)] {
        let mut l = Layer::new("m01", w);
        l.id = id.into();
        l.shapes.push(Shape::Path(rect(0.0, 200.0, w, 280.0)));
        l.info = LayerInfoBox::new(LayerInfo {
            name: if top { Some("Wide".into()) } else { None },
            attrs: LayerAttrs { smart_poles: vec![SmartPole { property: "Width".into(), top }], ..Default::default() },
            ..Default::default()
        });
        part.layers.push(l);
    }
    f.glyphs.push(part);
    let mut user = Glyph::new("emdash");
    user.unicodes = vec![0x2014];
    let mut c = fontcraft_engine::model::Component::new("_part.bar", 50.0, 0.0);
    c.info = ComponentInfoBox::new(ComponentInfo { smart_values: vec![SmartValue { property: "Width".into(), value: 300.0 }], ..Default::default() });
    let mut l = Layer::new("m01", 400.0);
    l.shapes.push(Shape::Component(c));
    user.layers.push(l);
    f.glyphs.push(user);

    // A corner component: an open serif drawn around the origin, on the bottom-left node of a stem.
    add(&mut f, &styles, "_corner.serif", &[], |_| Draw::new(0.0).path(open_poly(&[(0.0, 90.0), (-40.0, 30.0), (-40.0, 0.0), (10.0, 0.0)])));
    add(&mut f, &styles, "I.serif", &[], |s| letter("I", s));
    if let Ok(mut g) = f.glyph_mut("I.serif")
        && let Some(l) = g.layer_mut("m01")
    {
        // The stem rectangle's nodes: bottom right 0, top right 1, top left 2, bottom left 3.
        l.info.hints.push(CornerHint::new("_corner.serif", 0, 3).to_hint());
    }

    // Parts joined by `#exit` and `#entry` anchors.
    add(&mut f, &styles, "_part.stem", &[], |s| Draw::new(s.stem).path(rect(0.0, 0.0, s.stem, s.xh)).anchor("#exit", s.stem, s.xh));
    add(&mut f, &styles, "_part.arch", &[], |s| {
        let w = 320.0;
        Draw::new(w)
            .path(super::draw::poly(&[(0.0, s.xh - s.bar), (w - s.stem, s.xh - s.bar), (w - s.stem, 0.0), (w, 0.0), (w, s.xh), (0.0, s.xh)]))
            .anchor("#entry", 0.0, s.xh)
            .anchor("#exit", w - s.stem, s.xh)
    });
    add(&mut f, &styles, "n.parts", &[], |s| {
        Draw::new(0.0).comp("_part.stem", 0.0, 0.0).comp("_part.arch", s.stem, 0.0).spaced(60.0, s.stem + 320.0, 50.0)
    });
    add(&mut f, &styles, "m.parts", &[], |s| {
        Draw::new(0.0).comp("_part.stem", 0.0, 0.0).comp("_part.arch", s.stem, 0.0).comp("_part.arch", 320.0, 0.0).spaced(60.0, 320.0 + 320.0, 50.0)
    });
    Built::font("m6-components.glyphs", "M6 § 5 (end to end) and `docs/observe.md`", "end-to-end font", f)
        .about(
            "One master. A, E, n, o, idotless, space; gravecomb, acutecomb, dieresiscomb, ringcomb with `_top` and `top` anchors; A and n \
             with `top` and `bottom` anchors, idotless with `top`; E and o without anchors (for Set Anchors). Composites Aacute, \
             Adieresis, odieresis, ntilde.stack (two stacked marks); flipped marks in Aringbelow (ring mirrored onto `bottom`) and \
             n.acutebelow. Smart glyph `_part.bar` (property Width 100–400, pole layers at each end) used by emdash with Width 300. \
             Corner component `_corner.serif` (an open path around the origin) on the bottom-left node of I.serif. Parts `_part.stem` \
             (`#exit`) and `_part.arch` (`#entry`, `#exit`) joined in n.parts and m.parts. Agrave, Eacute, idieresis, Aring and n_n are \
             left for the check to add.",
        )
        .checks("M6 automatic alignment, anchors, composition, smart and corner components rows")
}

/// M7 font A.
fn m7_base(family: &str) -> (Font, Vec<Style>) {
    let (mut f, styles) = font(family, &[], &[m("m01", "Regular", &[], 400.0)]);
    add_letter(&mut f, &styles, "space", &[0x20]);
    for (n, u) in [("A", 0x41), ("B", 0x42), ("V", 0x56), ("a", 0x61), ("n", 0x6E), ("o", 0x6F)] {
        add_letter(&mut f, &styles, n, &[u]);
    }
    add_letter(&mut f, &styles, "acutecomb", &[0x301]);
    base_anchors(&mut f, &styles, &["A"]);
    add_composite(&mut f, &styles, "Aacute", &[0xC1], "A", &[("acutecomb", "top", false)]);
    groups(&mut f, &["A", "Aacute"], Some("A"), Some("A"));
    groups(&mut f, &["V"], Some("V"), Some("V"));
    kern(&mut f, "m01", "@MMK_L_A", "@MMK_R_V", -60.0);
    kern(&mut f, "m01", "@MMK_L_V", "@MMK_R_A", -60.0);
    {
        let ms = Arc::make_mut(&mut f.masters);
        if let Some(mm) = ms.first_mut() {
            mm.guides.push(Guide { x: 0.0, y: 600.0, ..Default::default() });
            mm.params.0.push(param("typoAscender", "780"));
        }
    }
    let info = Arc::make_mut(&mut f.info);
    info.params.0.push(param("versionString", "\"Version %d.%03d\""));
    info.params.0.push(param("Name Table Entry", "\"10; Synthetic test font\""));
    f.instances = Arc::new(vec![instance("Regular", &[])]);
    (f, styles)
}

fn m7_one() -> Built {
    let (mut f, _) = m7_base("FC Seven One");
    // An unknown master parameter, for the "kept on save" check.
    if let Some(mm) = Arc::make_mut(&mut f.masters).first_mut() {
        mm.params.0.push(param("Master Name", "Regular"));
    }
    Built::font("m7-one.glyphs", "M7 § 8", "A `m7-one`", f)
        .about(
            "One master Regular with the five Latin metrics and no scoped metric; space, A, B, V, a, n, o, acutecomb, Aacute (composite \
             on A's `top` anchor); one master guide; kerning groups A and V with @A/@V and @V/@A pairs; instance Regular; typoAscender \
             780 on the master, versionString and a Name Table Entry (ID 10) on the font; an unknown master parameter `Master Name`.",
        )
        .checks("M7 checks 1–18, 24–28, 34, 53")
        .gap("V is added so the A/V group pair has both sides; the plan's glyph list does not name it.")
}

fn m7_two() -> Built {
    let (mut f, styles) = font("FC Seven Two", &[("Weight", "wght")], &[m("m01", "Thin", &[100.0], 100.0), m("m02", "Bold", &[700.0], 700.0)]);
    add_letter(&mut f, &styles, "space", &[0x20]);
    for (n, u) in [("A", 0x41), ("V", 0x56), ("a", 0x61), ("n", 0x6E), ("o", 0x6F)] {
        add_letter(&mut f, &styles, n, &[u]);
    }
    // A brace layer tied to Bold, at weight 400.
    if let Ok(mut g) = f.glyph_mut("a") {
        let s = Style::weight("brace-a", 400.0);
        let mut l = letter("a", &s).into_layer("m02");
        l.id = "brace-a-400".into();
        l.info = LayerInfoBox::new(LayerInfo {
            name: Some("{400}".into()),
            attrs: LayerAttrs { coordinates: vec![Some(400.0)], ..Default::default() },
            ..Default::default()
        });
        g.layers.push(l);
    }
    groups(&mut f, &["A"], Some("A"), Some("A"));
    groups(&mut f, &["V"], Some("V"), Some("V"));
    kern(&mut f, "m01", "@MMK_L_A", "@MMK_R_V", -40.0);
    kern(&mut f, "m02", "@MMK_L_A", "@MMK_R_V", -80.0);
    kern(&mut f, "m01", "V", "a", -20.0);
    kern(&mut f, "m02", "V", "a", -45.0);
    if let Some(mm) = Arc::make_mut(&mut f.masters).first_mut() {
        mm.params.0.push(param("Link Metrics With Master", "Bold"));
    }
    let mut bold = instance("Bold", &[700.0]);
    bold.exports = false;
    let mut bi = instance("Bold Italic", &[400.0]);
    bi.is_bold = true;
    bi.is_italic = true;
    f.instances = Arc::new(vec![instance("Thin", &[100.0]), instance("Regular", &[400.0]), bold, bi]);
    Built::font("m7-two.glyphs", "M7 § 8", "B `m7-two`", f)
        .about(
            "Masters Thin (wght 100) and Bold (700); space, A, V, a, n, o; a brace layer {400} on `a` tied to Bold; kerning in both \
             masters (group pair and a glyph pair); instances Thin, Regular (400), Bold (inactive), Bold Italic (bold and italic \
             linking, empty base field, at 400); Link Metrics With Master naming Bold on Thin.",
        )
        .checks("M7 checks 19–23, 29–33, 50–52")
}

fn m7_export() -> Built {
    let (mut f, styles) = m7_base("FC Seven Export");
    for n in ["beh-ar", "beh-ar.init", "beh-ar.medi", "beh-ar.fina", "lam_alef-ar"] {
        let u: &[u32] = match n {
            "beh-ar" => &[0x628],
            _ => &[],
        };
        add_letter(&mut f, &styles, n, u);
    }
    add_letter(&mut f, &styles, "a.alt", &[]);
    if let Ok(mut g) = f.glyph_mut("a.alt") {
        g.export = false;
    }
    add_letter(&mut f, &styles, "g", &[0x67]);
    add_letter(&mut f, &styles, "g.single", &[]);
    let p = |name: &str, value: &str| param(name, value);
    let checks: Vec<(&str, Vec<fontcraft_engine::model::Param>)> = vec![
        ("Plain", vec![]),
        ("x35 Remove A", vec![p("Remove Glyphs", "(A)")]),
        ("x36 Remove export0", vec![p("Remove Glyphs", "(\"export=0\")")]),
        ("x36 Remove 1E", vec![p("Remove Glyphs", "(\"unicode=1E*\")")]),
        ("x36 Remove marks", vec![p("Remove Glyphs", "(\"category=Mark\")")]),
        ("x37 Keep", vec![p("Keep Glyphs", "(A, a)")]),
        ("x37 Keep and Remove", vec![p("Keep Glyphs", "(A, a)"), p("Remove Glyphs", "(a)")]),
        ("x38 Rename single", vec![p("Rename Glyphs", "(\"g.single=g\")")]),
        ("x38 Rename missing", vec![p("Rename Glyphs", "(\"a=nonexistent\")")]),
        ("x39 Reencode", vec![p("Reencode Glyphs", "(\"a=0061,0251\", \"space=\")")]),
        ("x40 Arabic forms", vec![p("Use Arabic Presentation Form Unicodes", "1")]),
        ("x41 Slant filter", vec![p("Filter", "\"Transformations;Slant:10\"")]),
        ("x43 UPM 2048", vec![p("Scale to UPM", "2048")]),
        ("x44 kern table", vec![p("Export kern Table", "1")]),
        ("x45 No ID 16", vec![p("Prevent Name ID", "16")]),
        ("x45 No ID 1", vec![p("Prevent Name ID", "1")]),
        ("x46 No Mac names", vec![p("Export Mac Name Table Entries", "0")]),
        ("x47 Compatible names", vec![p("Compatible name Table", "1")]),
        ("x48 Version beta", vec![p("versionString", "\"Version %d.%03d;beta\"")]),
        ("x49 Folder up", vec![p("Export Folder", "\"../trial\"")]),
        ("x49 Folder tmp", vec![p("Export Folder", "\"/tmp/x\"")]),
    ];
    let mut instances: Vec<_> = checks
        .into_iter()
        .map(|(name, params)| {
            let mut i = instance(name, &[]);
            i.params.0 = params;
            i
        })
        .collect();
    let mut var = instance("x42 Variable PreFilter", &[]);
    var.kind = Some("variable".into());
    var.params.0.push(p("PreFilter", "\"RemoveOverlap\""));
    instances.push(var);
    f.instances = Arc::new(instances);
    // The version-string check compares with and without the font's own versionString.
    Built::font("m7-export.glyphs", "M7 § 8", "C `m7-export`", f)
        .about(
            "Font A plus beh-ar, beh-ar.init, beh-ar.medi, beh-ar.fina, lam_alef-ar, the non-exporting `a.alt`, g and g.single. One \
             instance per export check, named `x<check> …`: Remove Glyphs (A; export=0; unicode=1E*; category=Mark), Keep Glyphs, Keep \
             plus Remove, Rename Glyphs (g.single=g; a=nonexistent), Reencode Glyphs, Use Arabic Presentation Form Unicodes, Filter \
             Transformations;Slant:10, Scale to UPM 2048, Export kern Table, Prevent Name ID 16 and 1, Export Mac Name Table Entries off, \
             Compatible name Table, versionString with ;beta, Export Folder ../trial and /tmp/x, a variable font setting with PreFilter \
             RemoveOverlap, and a Plain instance with no parameter.",
        )
        .checks("M7 checks 35–49")
        .gap("Check 48's \"no versionString\" case needs the font's own versionString parameter removed by hand.")
}
