use fontcraft_model::{ColorStop, Extend, GradientKind, Node, NodeKind, Path, Transform};
use fontcraft_plist::Num;

use super::*;
use crate::Files;
use crate::colorkeys::{COLOR_PALETTES, close};

fn plist(body: &str) -> Vec<u8> {
    format!(
        "<?xml version='1.0' encoding='UTF-8'?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n{body}\n</plist>\n"
    )
    .into_bytes()
}

fn value(body: &str) -> Value {
    fontcraft_plist::xmlplist::parse(&String::from_utf8(plist(body)).unwrap()).unwrap().root
}

/// A glyph with one rectangle from (`x0`, `y0`) to (`x1`, `y1`).
fn glif(name: &str, [x0, y0, x1, y1]: [i32; 4]) -> Vec<u8> {
    format!(
        "<?xml version='1.0' encoding='UTF-8'?>\n<glyph name=\"{name}\" format=\"2\">\n  <advance width=\"600\"/>\n  <outline>\n    <contour>\n      <point x=\"{x0}\" y=\"{y0}\" type=\"line\"/>\n      <point x=\"{x1}\" y=\"{y0}\" type=\"line\"/>\n      <point x=\"{x1}\" y=\"{y1}\" type=\"line\"/>\n      <point x=\"{x0}\" y=\"{y1}\" type=\"line\"/>\n    </contour>\n  </outline>\n</glyph>\n"
    )
    .into_bytes()
}

/// A UFO with glyphs `A`, `B` (both 100 to 500 across, 0 to 700 up) and `C` (0 to 200 square),
/// in that order, with `lib` as the rest of its lib.
fn ufo(lib: &str) -> Files {
    let mut files = Files::new();
    files.insert(
        "metainfo.plist".into(),
        plist("<dict>\n<key>creator</key>\n<string>org.example</string>\n<key>formatVersion</key>\n<integer>3</integer>\n</dict>"),
    );
    files.insert(
        "fontinfo.plist".into(),
        plist("<dict>\n<key>familyName</key>\n<string>Painted</string>\n<key>unitsPerEm</key>\n<integer>1000</integer>\n</dict>"),
    );
    files.insert(
        "layercontents.plist".into(),
        plist("<array>\n<array>\n<string>public.default</string>\n<string>glyphs</string>\n</array>\n</array>"),
    );
    files.insert(
        "glyphs/contents.plist".into(),
        plist(
            "<dict>\n<key>A</key>\n<string>A_.glif</string>\n<key>B</key>\n<string>B_.glif</string>\n<key>C</key>\n<string>C_.glif</string>\n</dict>",
        ),
    );
    files.insert("glyphs/A_.glif".into(), glif("A", [100, 0, 500, 700]));
    files.insert("glyphs/B_.glif".into(), glif("B", [100, 0, 500, 700]));
    files.insert("glyphs/C_.glif".into(), glif("C", [0, 0, 200, 200]));
    files.insert(
        "lib.plist".into(),
        plist(&format!(
            "<dict>\n{lib}<key>public.glyphOrder</key>\n<array>\n<string>A</string>\n<string>B</string>\n<string>C</string>\n</array>\n</dict>"
        )),
    );
    files
}

const PALETTES: &str = concat!(
    "<key>com.github.googlei18n.ufo2ft.colorPalettes</key><array><array>",
    "<array><real>0.0</real><real>1.0</real><real>0.0</real><real>1.0</real></array>",
    "<array><real>1.0</real><real>1.0</real><real>1.0</real><real>1.0</real></array>",
    "</array></array>\n"
);

fn rect(x0: f64, y0: f64, x1: f64, y1: f64, attrs: ShapeAttrs) -> Shape {
    let mut p = Path::rect(x0, y0, x1, y1);
    p.info.attrs = attrs;
    Shape::Path(p)
}

fn fill(color: Color) -> ShapeAttrs {
    ShapeAttrs { fill_color: Some(color), ..Default::default() }
}

fn gradient(kind: GradientKind, from: Color, to: Color) -> ShapeAttrs {
    let stops = vec![ColorStop { offset: 0.0, color: from }, ColorStop { offset: 1.0, color: to }];
    ShapeAttrs { gradient: Some(Gradient { kind, stops, extend: Extend::Pad }), ..Default::default() }
}

fn full_color(master: &str, id: &str, shapes: Vec<Shape>) -> Layer {
    let mut l = Layer::new(master, 600.0);
    l.id = id.to_string();
    l.info.name = Some(FULL_COLOR_NAME.to_string());
    l.info.attrs.full_color = true;
    l.shapes = shapes;
    l
}

fn lib_value(files: &Files, key: &str) -> Value {
    let lib = fontcraft_plist::xmlplist::parse(std::str::from_utf8(&files["lib.plist"]).unwrap()).unwrap().root;
    lib.as_dict().and_then(|d| d.get(key)).cloned().unwrap_or(Value::Bool(false))
}

/// The colour layers of `glyph` without their ids, which the reader numbers itself.
fn painted(font: &Font, glyph: &str) -> Vec<Layer> {
    let g = font.glyph(glyph).unwrap();
    g.layers
        .iter()
        .filter(|l| l.info.attrs.full_color)
        .map(|l| {
            let mut l = l.clone();
            l.id = String::new();
            l.extra = Default::default();
            l
        })
        .collect()
}

/// glyphsLib's `to_ufos(minimal=True)` of a Glyphs 3 source with the same three glyphs: `A`'s
/// full-colour layer has a red rectangle and a red-to-blue linear gradient one, `B`'s master layer
/// is full colour with a green-to-red radial gradient, and `C`'s full-colour layer has a red
/// square and a component of `B` moved 300 across. Generated once with glyphsLib 6.
const GLYPHSLIB_LAYERS: &str = concat!(
    "<dict><key>A</key><dict><key>Format</key><integer>1</integer><key>Layers</key><array>",
    "<dict><key>Format</key><integer>10</integer><key>Glyph</key><string>A.color0</string><key>Paint</key><dict><key>Alpha</key><real>1.0</real><key>Format</key><integer>2</integer><key>PaletteIndex</key><integer>2</integer></dict></dict>",
    "<dict><key>Format</key><integer>10</integer><key>Glyph</key><string>A.color1</string><key>Paint</key><dict><key>ColorLine</key><dict><key>ColorStop</key><array>",
    "<dict><key>Alpha</key><real>1.0</real><key>PaletteIndex</key><integer>2</integer><key>StopOffset</key><integer>0</integer></dict>",
    "<dict><key>Alpha</key><real>1.0</real><key>PaletteIndex</key><integer>3</integer><key>StopOffset</key><integer>1</integer></dict>",
    "</array><key>Extend</key><string>pad</string></dict><key>Format</key><integer>4</integer><key>x0</key><integer>300</integer><key>x1</key><integer>500</integer><key>x2</key><integer>1000</integer><key>y0</key><integer>0</integer><key>y1</key><integer>700</integer><key>y2</key><integer>-200</integer></dict></dict>",
    "</array></dict>",
    "<key>B</key><dict><key>Format</key><integer>1</integer><key>Layers</key><array>",
    "<dict><key>Format</key><integer>10</integer><key>Glyph</key><string>B</string><key>Paint</key><dict><key>ColorLine</key><dict><key>ColorStop</key><array>",
    "<dict><key>Alpha</key><real>1.0</real><key>PaletteIndex</key><integer>4</integer><key>StopOffset</key><integer>0</integer></dict>",
    "<dict><key>Alpha</key><real>1.0</real><key>PaletteIndex</key><integer>2</integer><key>StopOffset</key><integer>1</integer></dict>",
    "</array><key>Extend</key><string>pad</string></dict><key>Format</key><integer>6</integer><key>r0</key><integer>0</integer><key>r1</key><real>531.5072906367325</real><key>x0</key><integer>100</integer><key>x1</key><integer>100</integer><key>y0</key><real>350.0</real><key>y1</key><real>350.0</real></dict></dict>",
    "</array></dict>",
    "<key>C</key><dict><key>Format</key><integer>1</integer><key>Layers</key><array>",
    "<dict><key>Format</key><integer>10</integer><key>Glyph</key><string>C.color0</string><key>Paint</key><dict><key>Alpha</key><real>1.0</real><key>Format</key><integer>2</integer><key>PaletteIndex</key><integer>2</integer></dict></dict>",
    "<dict><key>Format</key><integer>14</integer><key>Paint</key><dict><key>Format</key><integer>11</integer><key>Glyph</key><string>B</string></dict><key>dx</key><integer>300</integer><key>dy</key><integer>0</integer></dict>",
    "</array></dict></dict>"
);

const GLYPHSLIB_PALETTES: &str = concat!(
    "<array><array>",
    "<array><real>0.0</real><real>1.0</real><real>0.0</real><real>1.0</real></array>",
    "<array><real>1.0</real><real>1.0</real><real>1.0</real><real>1.0</real></array>",
    "<array><real>1.0</real><real>0.0</real><real>0.0</real><real>1.0</real></array>",
    "<array><real>0.0</real><real>0.0</real><real>1.0</real><real>1.0</real></array>",
    "<array><real>0.0</real><real>1.0</real><real>0.0</real><real>1.0</real></array>",
    "</array></array>"
);

#[test]
fn full_colour_layers_of_paths_are_written_as_glyphslib_writes_them() {
    let mut font = crate::ufo::read(&ufo(PALETTES)).unwrap();
    let m = font.masters[0].id.clone();
    let (red, green, blue) = (Color::Rgba([1.0, 0.0, 0.0, 1.0]), Color::Rgba([0.0, 1.0, 0.0, 1.0]), Color::Rgba([0.0, 0.0, 1.0, 1.0]));
    let linear = GradientKind::Linear { start: (0.0, 0.0).into(), end: (1.0, 1.0).into() };
    let a = full_color(&m, "L1", vec![rect(100.0, 0.0, 300.0, 700.0, fill(red)), rect(300.0, 0.0, 500.0, 700.0, gradient(linear, red, blue))]);
    font.glyph_mut("A").unwrap().layers.push(a);
    {
        let mut b = font.glyph_mut("B").unwrap();
        let master = b.layer_mut(&m).unwrap();
        master.info.attrs.full_color = true;
        let radial = GradientKind::Radial { start: (0.0, 0.5).into(), end: (1.0, 0.5).into(), start_radius: 0.0, end_radius: 0.0, angle: 0.0 };
        master.shapes = vec![rect(100.0, 0.0, 500.0, 700.0, gradient(radial, green, red))];
    }
    let c = full_color(&m, "L2", vec![rect(0.0, 0.0, 200.0, 200.0, fill(red)), Shape::Component(Component::new("B", 300.0, 0.0))]);
    font.glyph_mut("C").unwrap().layers.push(c);
    assert_eq!(crate::ufo::layers_left_out(&font, false), 0);

    let out = crate::ufo::write(&font).unwrap();
    let layers = lib_value(&out, COLOR_LAYERS);
    assert!(close(&layers, &value(GLYPHSLIB_LAYERS)), "{}", fontcraft_plist::xmlplist::write_fragment(&layers));
    assert!(close(&lib_value(&out, COLOR_PALETTES), &value(GLYPHSLIB_PALETTES)));
    for (name, x) in [("A.color0", "300"), ("A.color1", "500"), ("C.color0", "200")] {
        let file = out.iter().find(|(k, v)| k.starts_with("glyphs/") && String::from_utf8_lossy(v).contains(&format!("name=\"{name}\""))).unwrap();
        let text = String::from_utf8_lossy(file.1);
        assert!(text.contains(&format!("x=\"{x}\"")) && !text.contains("<unicode"), "{name}: {text}");
    }
    let order = lib_value(&out, "public.glyphOrder");
    assert!(!fontcraft_plist::xmlplist::write_fragment(&order).contains(".color"), "the layer glyphs stay out of the glyph order");
    assert!(!out.keys().any(|k| k.starts_with("glyphs.")), "no UFO layer for the colour layers: {:?}", out.keys());
    assert!(!crate::ufo::info_losses(&font).iter().any(|l| l.contains("paint attributes")), "{:?}", crate::ufo::info_losses(&font));
}

#[test]
fn layer_glyphs_fold_back_into_their_colour_layers() {
    let mut font = crate::ufo::read(&ufo(PALETTES)).unwrap();
    let m = font.masters[0].id.clone();
    let p = |index: u16| Color::Palette { index, alpha: 1.0 };
    let linear = GradientKind::Linear { start: (0.0, 0.25).into(), end: (1.0, 0.75).into() };
    let one = full_color(&m, "L1", vec![rect(100.0, 0.0, 300.0, 700.0, fill(p(0)))]);
    // Two shapes with one gradient make one layer glyph.
    let two = full_color(
        &m,
        "L2",
        vec![rect(300.0, 0.0, 500.0, 700.0, gradient(linear, p(0), p(1))), rect(320.0, 20.0, 340.0, 40.0, gradient(linear, p(0), p(1)))],
    );
    font.glyph_mut("A").unwrap().layers.extend([one, two]);
    {
        let mut b = font.glyph_mut("B").unwrap();
        let master = b.layer_mut(&m).unwrap();
        master.info.attrs.full_color = true;
        let path = Path::new(
            true,
            vec![Node::new(100.0, 0.0, NodeKind::Line), Node::new(500.0, 0.0, NodeKind::Line), Node::new(300.0, 700.0, NodeKind::Line)],
        );
        let mut path = path;
        path.info.attrs = fill(Color::Palette { index: 1, alpha: 0.5 });
        master.shapes = vec![Shape::Path(path)];
    }
    // A component of a colour glyph paints that glyph's colour layers.
    let colr = full_color(&m, "L3", vec![Shape::Component(Component::new("B", 300.0, 0.0))]);
    font.glyph_mut("C").unwrap().layers.push(colr);

    let out = crate::ufo::write(&font).unwrap();
    assert!(out.values().any(|v| String::from_utf8_lossy(v).contains("name=\"A.color1\"")));
    let back = crate::ufo::read(&out).unwrap();
    for name in ["A.color0", "A.color1"] {
        assert!(back.glyph(name).is_none(), "{name} is folded into its layer");
    }
    for glyph in ["A", "B", "C"] {
        assert_eq!(painted(&back, glyph), painted(&font, glyph), "{glyph}: model → UFO → model");
    }
    assert!(back.masters[0].user_data.get(COLOR_LAYERS).is_none());
    assert_eq!(crate::ufo::write(&back).unwrap(), out, "written again the same");

    // A layer glyph that carries more than the writer gives it stays a glyph of its own.
    let mut files = out.clone();
    let (key, glif) = files.iter().find(|(k, v)| k.starts_with("glyphs/") && String::from_utf8_lossy(v).contains("name=\"A.color0\"")).unwrap();
    let (key, glif) = (key.clone(), String::from_utf8_lossy(glif).replace("  <outline>", "  <unicode hex=\"E000\"/>\n  <outline>"));
    files.insert(key, glif.into_bytes());
    let kept = crate::ufo::read(&files).unwrap();
    assert!(kept.glyph("A.color0").is_some());
    assert!(kept.glyph("A.color1").is_none());
    assert_eq!(crate::ufo::write(&kept).unwrap(), files, "written back as read");
}

#[test]
fn variable_paints_turned_gradients_and_colour_glyph_paints_read_and_write_back() {
    // `C` is painted by a variable linear gradient (its stops variable too) inside a variable
    // translation, a radial gradient turned 30 degrees about a point, and `A`'s colour layers.
    let stop = |offset: i64, index: i64, var: Option<i64>| {
        let var = var.map(|v| format!("<key>VarIndexBase</key><integer>{v}</integer>")).unwrap_or_default();
        format!(
            "<dict><key>Alpha</key><real>1.0</real><key>PaletteIndex</key><integer>{index}</integer><key>StopOffset</key><integer>{offset}</integer>{var}</dict>"
        )
    };
    let line = |var: bool| {
        let (a, b) = if var { (Some(3), Some(4)) } else { (None, None) };
        format!(
            "<key>ColorLine</key><dict><key>ColorStop</key><array>{}{}</array><key>Extend</key><string>pad</string></dict>",
            stop(0, 0, a),
            stop(1, 1, b)
        )
    };
    let lib = format!(
        concat!(
            "{palettes}<key>com.github.googlei18n.ufo2ft.colorLayers</key><dict>",
            "<key>A</key><array><dict><key>Format</key><integer>10</integer><key>Glyph</key><string>A</string><key>Paint</key>",
            "<dict><key>Alpha</key><real>1.0</real><key>Format</key><integer>2</integer><key>PaletteIndex</key><integer>0</integer></dict></dict></array>",
            "<key>C</key><dict><key>Format</key><integer>1</integer><key>Layers</key><array>",
            "<dict><key>Format</key><integer>15</integer><key>Paint</key><dict><key>Format</key><integer>10</integer><key>Glyph</key><string>B</string>",
            "<key>Paint</key><dict>{var_line}<key>Format</key><integer>5</integer><key>VarIndexBase</key><integer>0</integer>",
            "<key>x0</key><integer>100</integer><key>x1</key><integer>500</integer><key>x2</key><integer>100</integer>",
            "<key>y0</key><integer>0</integer><key>y1</key><integer>0</integer><key>y2</key><integer>-400</integer></dict></dict>",
            "<key>VarIndexBase</key><integer>5</integer><key>dx</key><integer>10</integer><key>dy</key><integer>0</integer></dict>",
            "<dict><key>Format</key><integer>26</integer><key>Paint</key><dict><key>Format</key><integer>10</integer><key>Glyph</key><string>B</string>",
            "<key>Paint</key><dict>{line}<key>Format</key><integer>6</integer><key>r0</key><integer>0</integer><key>r1</key><integer>200</integer>",
            "<key>x0</key><integer>300</integer><key>x1</key><integer>300</integer><key>y0</key><integer>350</integer><key>y1</key><integer>350</integer></dict></dict>",
            "<key>angle</key><integer>30</integer><key>centerX</key><integer>300</integer><key>centerY</key><integer>350</integer></dict>",
            "<dict><key>Format</key><integer>11</integer><key>Glyph</key><string>A</string></dict>",
            "</array></dict></dict>\n"
        ),
        palettes = PALETTES,
        var_line = line(true),
        line = line(false),
    );
    let files = ufo(&lib);
    let font = crate::ufo::read(&files).unwrap();
    assert!(font.masters[0].user_data.get(COLOR_LAYERS).is_none(), "every entry is typed: {:?}", font.masters[0].user_data.get(COLOR_LAYERS));
    let a = font.glyph("A").unwrap().layer(&font.masters[0].id).unwrap();
    assert!(a.info.attrs.full_color, "`A` paints its own master layer");
    let c = painted(&font, "C");
    let components: Vec<&Component> = c
        .iter()
        .filter_map(|l| match l.shapes.as_slice() {
            [Shape::Component(c)] => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(components.len(), 3);
    let [var, turned, colr] = components.as_slice() else { panic!() };
    let record = var.extra.get(Family::Ufo, "paintVar").and_then(Value::as_dict).unwrap();
    assert_eq!(record.get("VarIndexBase").and_then(Value::as_num), Some(Num::Int(0)));
    assert!(matches!(turned.transform, Transform::Matrix(_)), "{:?}", turned.transform);
    assert!(matches!(turned.info.attrs.gradient.as_ref().map(|g| &g.kind), Some(GradientKind::Radial { .. })));
    assert_eq!((colr.glyph.as_str(), colr.info.attrs.is_empty()), ("A", true));

    let out = crate::ufo::write(&font).unwrap();
    let written = lib_value(&out, COLOR_LAYERS);
    let read = lib_value(&files, COLOR_LAYERS);
    assert!(close(&written, &read), "{}", fontcraft_plist::xmlplist::write_fragment(&written));
    assert_eq!(crate::ufo::read(&out).map(|f| painted(&f, "C")).unwrap(), c);

    // A radial gradient under a skew is an ellipse: it stays user data.
    let skewed = lib
        .replace("<integer>26</integer>", "<integer>30</integer>")
        .replace("<key>angle</key>", "<key>xSkewAngle</key><integer>20</integer><key>ySkewAngle</key>");
    let font = crate::ufo::read(&ufo(&skewed)).unwrap();
    let kept = font.masters[0].user_data.get(COLOR_LAYERS).and_then(Value::as_dict).unwrap();
    assert_eq!(kept.entries.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["C"]);
}

#[test]
fn folding_many_layer_glyphs_looks_at_each_glyph_a_bounded_number_of_times() {
    const BASES: usize = 1000;
    let mut font = crate::ufo::read(&ufo(PALETTES)).unwrap();
    let m = font.masters[0].id.clone();
    let p = |index: u16| Color::Palette { index, alpha: 1.0 };
    for i in 0..BASES {
        let name = format!("g{i}");
        font.add_glyph(&name, vec![], 600.0).unwrap();
        let layers = [
            full_color(&m, &format!("{name}.a"), vec![rect(100.0, 0.0, 300.0, 700.0, fill(p(0)))]),
            full_color(&m, &format!("{name}.b"), vec![rect(300.0, 0.0, 500.0, 700.0, fill(p(1)))]),
        ];
        font.glyph_mut(&name).unwrap().layers.extend(layers);
    }
    let out = crate::ufo::write(&font).unwrap();
    let written = 3 + 3 * BASES;
    COUNTED.with(|c| c.set(0));
    let back = crate::ufo::read(&out).unwrap();
    let counted = COUNTED.with(|c| c.get());
    assert!(back.glyph("g0.color0").is_none() && back.glyph(&format!("g{}.color1", BASES - 1)).is_none());
    assert_eq!(back.glyphs.len(), font.glyphs.len());
    assert_eq!(painted(&back, "g7"), painted(&font, "g7"));
    // Each glyph once when the fold starts, then each base twice and each layer glyph once as it
    // folds: linear in the font, however many layer glyphs there are.
    assert!(counted <= 3 * written, "{counted} glyphs looked at for {written} written");
}
