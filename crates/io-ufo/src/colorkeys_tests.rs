use super::*;
use crate::Files;

fn plist(body: &str) -> Vec<u8> {
    format!(
        "<?xml version='1.0' encoding='UTF-8'?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n{body}\n</plist>\n"
    )
    .into_bytes()
}

fn glif(name: &str, x: i32) -> Vec<u8> {
    format!(
        "<?xml version='1.0' encoding='UTF-8'?>\n<glyph name=\"{name}\" format=\"2\">\n  <advance width=\"600\"/>\n  <outline>\n    <contour>\n      <point x=\"{x}\" y=\"0\" type=\"line\"/>\n      <point x=\"{}\" y=\"0\" type=\"line\"/>\n      <point x=\"{}\" y=\"700\" type=\"line\"/>\n      <point x=\"{x}\" y=\"700\" type=\"line\"/>\n    </contour>\n  </outline>\n</glyph>\n",
        x + 200,
        x + 200
    )
    .into_bytes()
}

/// A UFO whose `a` is drawn in two palette colours by `a.color0` (100 to 300 across, 0 to 700
/// up) and `a.color1` (300 to 500 across).
fn listed_ufo() -> Files {
    let mut files = Files::new();
    files.insert(
        "metainfo.plist".into(),
        plist("<dict>\n<key>creator</key>\n<string>org.example</string>\n<key>formatVersion</key>\n<integer>3</integer>\n</dict>"),
    );
    files.insert(
        "fontinfo.plist".into(),
        plist("<dict>\n<key>familyName</key>\n<string>Listed</string>\n<key>unitsPerEm</key>\n<integer>1000</integer>\n</dict>"),
    );
    files.insert(
        "layercontents.plist".into(),
        plist("<array>\n<array>\n<string>public.default</string>\n<string>glyphs</string>\n</array>\n</array>"),
    );
    files.insert(
        "glyphs/contents.plist".into(),
        plist("<dict>\n<key>a</key>\n<string>a.glif</string>\n<key>a.color0</key>\n<string>a.color0.glif</string>\n<key>a.color1</key>\n<string>a.color1.glif</string>\n</dict>"),
    );
    files.insert("glyphs/a.glif".into(), glif("a", 100));
    files.insert("glyphs/a.color0.glif".into(), glif("a.color0", 100));
    files.insert("glyphs/a.color1.glif".into(), glif("a.color1", 300));
    files.insert(
        "lib.plist".into(),
        plist(concat!(
            "<dict>\n<key>com.github.googlei18n.ufo2ft.colorLayers</key>\n<dict>\n<key>a</key>\n<array>\n",
            "<array>\n<string>a.color0</string>\n<integer>0</integer>\n</array>\n",
            "<array>\n<string>a.color1</string>\n<integer>1</integer>\n</array>\n</array>\n</dict>\n",
            "<key>com.github.googlei18n.ufo2ft.colorPalettes</key>\n<array>\n<array>\n",
            "<array>\n<real>1.0</real>\n<real>0.0</real>\n<real>0.0</real>\n<real>1.0</real>\n</array>\n",
            "<array>\n<real>0.0</real>\n<real>0.0</real>\n<real>1.0</real>\n<real>1.0</real>\n</array>\n</array>\n</array>\n",
            "<key>public.glyphOrder</key>\n<array>\n<string>a</string>\n<string>a.color0</string>\n<string>a.color1</string>\n</array>\n</dict>"
        )),
    );
    files
}

/// The colour layers of `glyph`: component glyph and palette entry.
fn colour_layers(font: &Font, glyph: &str) -> Vec<(String, Option<PaletteEntry>)> {
    let a = font.glyph(glyph).unwrap();
    a.layers
        .iter()
        .filter(|l| !l.is_master_layer())
        .map(|l| {
            let name = match l.shapes.as_slice() {
                [Shape::Component(c)] => c.glyph.clone(),
                other => format!("{other:?}"),
            };
            (name, l.info.attrs.color_palette)
        })
        .collect()
}

fn other_layers(font: &Font, glyph: &str) -> Vec<Layer> {
    font.glyph(glyph).unwrap().layers.iter().filter(|l| !l.is_master_layer()).cloned().collect()
}

/// [`listed_ufo`] read, without colour layers or a list.
fn plain() -> Font {
    let mut font = crate::ufo::read(&listed_ufo()).unwrap();
    font.glyph_mut("a").unwrap().layers.retain(|l| l.is_master_layer());
    font
}

fn master(font: &Font) -> String {
    font.masters[0].id.clone()
}

/// `font` with `list` as its master's colour layer list in its user data.
fn with_list(mut font: Font, list: Value) -> Font {
    Arc::make_mut(&mut font.masters)[0].user_data.0.insert(COLOR_LAYERS, list);
    font
}

/// A full-colour layer as the reader makes it: item `k` of the glyph's entry.
fn full(master: &str, k: usize, c: Component) -> Layer {
    let mut l = Layer::new(master, 600.0);
    l.id = synthetic_id(master, &format!("{MARK}.{k}"));
    l.extra.insert(Family::Ufo, MARK, Value::Bool(true));
    l.info.name = Some(FULL_COLOR_NAME.to_string());
    l.info.attrs.full_color = true;
    l.shapes = vec![Shape::Component(c)];
    l
}

fn painted(glyph: &str, transform: Transform, attrs: ShapeAttrs) -> Component {
    let mut c = Component::new(glyph, 0.0, 0.0);
    c.transform = transform;
    c.info.attrs = attrs;
    c
}

fn fill(color: Color) -> ShapeAttrs {
    ShapeAttrs { fill_color: Some(color), ..Default::default() }
}

fn gradient(kind: GradientKind, extend: Extend) -> ShapeAttrs {
    let stops = vec![
        ColorStop { offset: 0.0, color: Color::Palette { index: 0, alpha: 1.0 } },
        ColorStop { offset: 1.0, color: Color::Palette { index: 1, alpha: 0.5 } },
    ];
    ShapeAttrs { gradient: Some(Gradient { kind, stops, extend }), ..Default::default() }
}

fn glyph_paint(glyph: &str, paint: Value) -> Value {
    dict(vec![("Format", int(format::GLYPH)), ("Glyph", string(glyph)), ("Paint", paint)])
}

fn solid_paint(index: i64, alpha: f64) -> Value {
    dict(vec![("Alpha", number(alpha)), ("Format", int(format::SOLID)), ("PaletteIndex", int(index))])
}

#[test]
fn a_font_wide_colour_layer_list_opens_as_palette_layers_and_is_written_back() {
    let font = crate::ufo::read(&listed_ufo()).unwrap();
    let want = vec![("a.color0".to_string(), Some(PaletteEntry::Index(0))), ("a.color1".to_string(), Some(PaletteEntry::Index(1)))];
    assert_eq!(colour_layers(&font, "a"), want);
    assert!(font.masters[0].user_data.get(COLOR_LAYERS).is_none(), "the list is not user data");

    let out = crate::ufo::write(&font).unwrap();
    let text = String::from_utf8(out["lib.plist"].clone()).unwrap();
    assert!(text.contains(COLOR_LAYERS), "{text}");
    assert!(!out.keys().any(|k| k.starts_with("glyphs.color")), "no UFO layer for the listed layers: {:?}", out.keys());
    let back = crate::ufo::read(&out).unwrap();
    assert_eq!(colour_layers(&back, "a"), want);
    assert_eq!(crate::ufo::write(&back).unwrap(), out, "a UFO we wrote comes back unchanged");
}

#[test]
fn a_list_naming_a_missing_glyph_stays_user_data() {
    let mut files = listed_ufo();
    files.remove("glyphs/a.color1.glif");
    files.insert(
        "glyphs/contents.plist".into(),
        plist("<dict>\n<key>a</key>\n<string>a.glif</string>\n<key>a.color0</key>\n<string>a.color0.glif</string>\n</dict>"),
    );
    let font = crate::ufo::read(&files).unwrap();
    assert!(colour_layers(&font, "a").is_empty());
    assert!(font.masters[0].user_data.get(COLOR_LAYERS).is_some());
}

#[test]
fn every_paint_kind_goes_to_the_colour_layer_list_and_back() {
    let mut font = plain();
    let m = master(&font);
    let point = |x: f64, y: f64| GradientKind::Linear { start: (x, y).into(), end: (1.0, 0.5).into() };
    let components = vec![
        painted("a.color0", Transform::default(), fill(Color::Palette { index: 1, alpha: 0.5 })),
        painted("a.color1", Transform::default(), gradient(point(0.25, 0.5), Extend::Repeat)),
        painted(
            "a.color0",
            Transform::default(),
            gradient(
                GradientKind::Radial { start: (0.5, 0.5).into(), end: (0.25, 0.5).into(), start_radius: 0.0, end_radius: 0.5, angle: 0.0 },
                Extend::Reflect,
            ),
        ),
        painted(
            "a.color1",
            Transform::default(),
            gradient(GradientKind::Conic { center: (0.5, 0.5).into(), start_angle: 0.0, angle: 90.0, control_radius: 0.0 }, Extend::Pad),
        ),
        painted("a.color0", Transform::translate(50.0, -20.0), fill(Color::Palette { index: 65535, alpha: 1.0 })),
        painted("a.color1", Transform::Matrix([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]), fill(Color::Palette { index: 0, alpha: 1.0 })),
        painted(
            "a.color0",
            Transform::Matrix([2.0, 0.0, 0.0, 2.0, 10.0, 0.0]),
            gradient(GradientKind::Linear { start: (0.25, 0.0).into(), end: (0.75, 1.0).into() }, Extend::Pad),
        ),
    ];
    let layers: Vec<Layer> = components.into_iter().enumerate().map(|(k, c)| full(&m, k, c)).collect();
    font.glyph_mut("a").unwrap().layers.extend(layers.clone());
    assert!(crate::ufo::layers_left_out(&font, false) == 0);

    let out = crate::ufo::write(&font).unwrap();
    assert!(!out.keys().any(|k| k.starts_with("glyphs.")), "no UFO layer for the listed layers: {:?}", out.keys());
    let text = String::from_utf8(out["lib.plist"].clone()).unwrap();
    for kind in ["<key>x2</key>", "<key>r1</key>", "<key>endAngle</key>", "<key>dx</key>", "<key>Transform</key>", "<string>reflect</string>"] {
        assert!(text.contains(kind), "{kind} in {text}");
    }
    let back = crate::ufo::read(&out).unwrap();
    assert_eq!(other_layers(&back, "a"), layers, "model → UFO → model");
    assert!(back.masters[0].user_data.get(COLOR_LAYERS).is_none());
    assert_eq!(crate::ufo::write(&back).unwrap(), out, "written again the same");

    // A radial gradient with both radii 0 runs to the farthest corner, as the compiler paints it.
    let mut corner = plain();
    let radial = gradient(
        GradientKind::Radial { start: (0.0, 0.0).into(), end: (0.0, 0.0).into(), start_radius: 0.0, end_radius: 0.0, angle: 0.0 },
        Extend::Pad,
    );
    corner.glyph_mut("a").unwrap().layers.push(full(&m, 0, painted("a.color0", Transform::default(), radial)));
    let (v, _) = value(&corner, &corner.masters[0], 2).unwrap();
    let r1 = v.as_dict().and_then(|d| d.get("a")).and_then(Value::as_dict).and_then(|d| d.get("Layers")).and_then(Value::as_array);
    let r1 = r1.and_then(|a| a.items.first()).and_then(Value::as_dict).and_then(|d| d.get("Paint")).and_then(Value::as_dict);
    let r1 = r1.and_then(|d| d.get("r1")).and_then(Value::as_num).map(Num::as_f64).unwrap();
    assert!((r1 - 200.0f64.hypot(700.0)).abs() < 1e-9, "{r1}");
}

#[test]
fn a_list_of_paints_and_a_single_paint_read_and_write_back_as_they_were() {
    let font = plain();
    let list = array(vec![glyph_paint("a.color0", solid_paint(0, 1.0)), glyph_paint("a.color1", solid_paint(1, 1.0))]);
    let around = dict(vec![
        ("Format", int(format::SCALE_CENTER)),
        ("Paint", glyph_paint("a.color0", solid_paint(1, 0.5))),
        ("centerX", int(400)),
        ("centerY", int(0)),
        ("scaleX", int(2)),
        ("scaleY", int(1)),
    ]);
    let mut d = Dict::new();
    d.insert("a", list);
    d.insert("a.color1", around);
    let entry = Value::Dict(d);
    let mut font = with_list(font, entry.clone());
    read(&mut font);
    assert!(font.masters[0].user_data.get(COLOR_LAYERS).is_none(), "both entries typed");
    let a = other_layers(&font, "a");
    assert_eq!(a.len(), 2);
    assert!(a.iter().all(|l| l.info.attrs.full_color));
    let single = other_layers(&font, "a.color1");
    let [Shape::Component(c)] = single[0].shapes.as_slice() else { panic!("{single:?}") };
    assert_eq!(c.transform, Transform::Matrix([2.0, 0.0, 0.0, 1.0, -400.0, 0.0]));
    assert_eq!(c.info.attrs.fill_color, Some(Color::Palette { index: 1, alpha: 0.5 }));

    let (v, added) = value(&font, &font.masters[0], 2).unwrap();
    assert!(added.is_empty());
    assert!(close(&v, &entry), "{v:?}");
    let back = crate::ufo::read(&crate::ufo::write(&font).unwrap()).unwrap();
    assert_eq!(other_layers(&back, "a.color1"), single);
    assert_eq!(other_layers(&back, "a"), a);
}

#[test]
fn malformed_entries_stay_user_data() {
    let nested = (0..=MAX_WRAPPERS).fold(glyph_paint("a.color0", solid_paint(0, 1.0)), |p, _| {
        dict(vec![("Format", int(format::TRANSLATE)), ("Paint", p), ("dx", int(1)), ("dy", int(0))])
    });
    let layers = |n: usize| dict(vec![("Format", int(format::LAYERS)), ("Layers", array(vec![glyph_paint("a.color0", solid_paint(0, 1.0)); n]))]);
    let rotated_gradient = dict(vec![
        ("Format", int(format::ROTATE)),
        ("angle", int(30)),
        (
            "Paint",
            glyph_paint(
                "a.color0",
                dict(vec![
                    ("ColorLine", dict(vec![("ColorStop", array(Vec::new())), ("Extend", string("pad"))])),
                    ("Format", int(format::LINEAR)),
                    ("x0", int(0)),
                    ("x1", int(1)),
                    ("x2", int(0)),
                    ("y0", int(0)),
                    ("y1", int(0)),
                    ("y2", int(-1)),
                ]),
            ),
        ),
    ]);
    let zero_radii = glyph_paint(
        "a.color0",
        dict(vec![
            ("ColorLine", dict(vec![("ColorStop", array(Vec::new())), ("Extend", string("pad"))])),
            ("Format", int(format::RADIAL)),
            ("r0", int(0)),
            ("r1", int(0)),
            ("x0", int(200)),
            ("x1", int(200)),
            ("y0", int(350)),
            ("y1", int(350)),
        ]),
    );
    let bad = vec![
        ("a string", string("a.color0")),
        ("a number", int(3)),
        ("a format as text", dict(vec![("Format", string("PaintColrLayers")), ("Layers", array(Vec::new()))])),
        ("a missing glyph", glyph_paint("missing", solid_paint(0, 1.0))),
        ("the base glyph itself", glyph_paint("a", solid_paint(0, 1.0))),
        ("a composite", dict(vec![("Format", int(32))])),
        (
            "a variable solid",
            glyph_paint("a.color0", dict(vec![("Alpha", int(1)), ("Format", int(3)), ("PaletteIndex", int(0)), ("VarIndexBase", int(0))])),
        ),
        ("an index beyond 65535", glyph_paint("a.color0", solid_paint(70_000, 1.0))),
        ("an alpha beyond 1", glyph_paint("a.color0", solid_paint(0, 2.0))),
        ("too many transforms", nested),
        ("too many layers", layers(MAX_LIST_LAYERS + 1)),
        ("no layers", layers(0)),
        ("a gradient that is turned", rotated_gradient),
        ("a radial gradient the writer gives other radii", zero_radii),
        ("an empty pair list", array(Vec::new())),
    ];
    for (what, entry) in bad {
        let mut d = Dict::new();
        d.insert("a", entry.clone());
        d.insert("a.color1", glyph_paint("a.color0", solid_paint(1, 1.0)));
        let mut font = with_list(plain(), Value::Dict(d));
        read(&mut font);
        assert!(other_layers(&font, "a").is_empty(), "{what}");
        assert_eq!(other_layers(&font, "a.color1").len(), 1, "{what}: the other entry is typed");
        let kept = font.masters[0].user_data.get(COLOR_LAYERS).and_then(Value::as_dict).unwrap();
        assert_eq!(kept.entries.len(), 1, "{what}");
        assert_eq!(kept.get("a"), Some(&entry), "{what}");
        // The writer merges the kept entry with the typed one.
        let (v, _) = value(&font, &font.masters[0], 2).unwrap();
        let v = v.as_dict().unwrap();
        assert_eq!((v.get("a"), v.get("a.color1").is_some()), (Some(&entry), true), "{what}");
    }
}

#[test]
fn colours_outside_the_palettes_are_added_to_them() {
    let mut font = plain();
    let m = master(&font);
    let orange = painted("a.color0", Transform::default(), fill(Color::Rgba([1.0, 0.5, 0.0, 1.0])));
    let grey = painted("a.color1", Transform::default(), ShapeAttrs::default());
    font.glyph_mut("a").unwrap().layers.extend([full(&m, 0, orange), full(&m, 1, grey)]);
    let losses = crate::ufo::info_losses(&font);
    assert!(losses.iter().any(|l| l.starts_with("the colours of full-colour layers in the colour layer list") && l.ends_with("(2)")), "{losses:?}");

    let back = crate::ufo::read(&crate::ufo::write(&font).unwrap()).unwrap();
    let fills: Vec<Option<Color>> = other_layers(&back, "a")
        .iter()
        .map(|l| match l.shapes.as_slice() {
            [Shape::Component(c)] => c.info.attrs.fill_color,
            _ => None,
        })
        .collect();
    assert_eq!(fills, [Some(Color::Palette { index: 2, alpha: 1.0 }), Some(Color::Palette { index: 3, alpha: 1.0 })]);
    let palettes = back.info.params.get("Color Palettes").or_else(|| back.masters[0].params.get("Color Palettes")).and_then(Value::as_array).unwrap();
    assert_eq!(palettes.items.first().and_then(Value::as_array).map(|p| p.items.len()), Some(4));
}
