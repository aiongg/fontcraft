//! norad, an independent UFO and designspace reader, reads what we write: the same glyphs, code
//! points, advances, point counts and components, and the same sources and locations.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fontcraft_io_ufo::{Files, designspace, ufo};
use fontcraft_model::{Axis, Component, Font, Node, NodeKind, Path as Contour, Shape};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-norad-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_tree(dir: &Path, files: &Files) {
    for (rel, bytes) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
}

fn sample() -> Font {
    let mut font = Font::new("Second Reader");
    {
        let mut a = font.glyph_mut("A").unwrap();
        let l = a.layer_mut("m01").unwrap();
        l.shapes.push(Shape::Path(Contour::new(
            true,
            vec![
                Node::new(0.0, 0.0, NodeKind::Line),
                Node::new(100.0, 0.0, NodeKind::Line),
                Node::new(150.0, 50.0, NodeKind::OffCurve),
                Node::new(150.0, 150.0, NodeKind::OffCurve),
                Node::new(100.0, 200.0, NodeKind::Curve),
            ],
        )));
        l.anchors.push(fontcraft_model::Anchor::new("top", 50.0, 700.0));
    }
    font.glyph_mut("B").unwrap().layer_mut("m01").unwrap().shapes.push(Shape::Component(Component::new("A", 20.5, 0.0)));
    font
}

/// Checks a norad font against the model's master `master`.
fn same(font: &Font, master: &str, n: &norad::Font) {
    assert_eq!(n.default_layer().len(), font.glyphs.iter().filter(|g| g.layer(master).is_some()).count());
    for g in font.glyphs.iter() {
        let Some(layer) = g.layer(master) else { continue };
        let ng = n.default_layer().get_glyph(g.name.as_str()).unwrap_or_else(|| panic!("norad has no {}", g.name));
        assert_eq!(ng.codepoints.iter().map(|c| c as u32).collect::<Vec<_>>(), g.unicodes, "{}", g.name);
        assert_eq!(ng.width, layer.width, "{}", g.name);
        let points: usize = layer.shapes.iter().map(|s| if let Shape::Path(p) = s { p.nodes.len() } else { 0 }).sum();
        assert_eq!(ng.contours.iter().map(|c| c.points.len()).sum::<usize>(), points, "{}", g.name);
        assert_eq!(ng.components.len(), layer.shapes.iter().filter(|s| matches!(s, Shape::Component(_))).count(), "{}", g.name);
        assert_eq!(ng.anchors.len(), layer.anchors.len(), "{}", g.name);
    }
    assert_eq!(n.font_info.family_name.as_deref(), Some(font.family_name.as_str()));
}

#[test]
fn norad_reads_a_ufo_we_wrote() {
    let font = sample();
    let dir = temp("ufo");
    let path = dir.join("Sample.ufo");
    write_tree(&path, &ufo::write(&font).unwrap());
    let n = norad::Font::load(&path).unwrap();
    same(&font, "m01", &n);
    let b = n.default_layer().get_glyph("B").unwrap();
    assert_eq!(b.components[0].transform.x_offset, 20.5);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn norad_reads_a_designspace_we_wrote() {
    let mut font = sample();
    Arc::make_mut(&mut font.axes).push(Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() });
    let mut bold = font.masters[0].clone();
    bold.id = "bold".into();
    bold.name = "Bold".into();
    bold.location = vec![700.0];
    Arc::make_mut(&mut font.masters)[0].location = vec![400.0];
    Arc::make_mut(&mut font.masters).push(bold);
    let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
    for name in names {
        let mut g = font.glyph_mut(&name).unwrap();
        let mut l = g.layers[0].clone();
        l.id = "bold".into();
        l.master = "bold".into();
        l.width += 50.0;
        g.layers.push(l);
    }
    let out = designspace::write(&font).unwrap();
    let dir = temp("designspace");
    std::fs::write(dir.join("Sample.designspace"), &out.document).unwrap();
    for (rel, files) in &out.ufos {
        write_tree(&dir.join(rel), files);
    }
    let doc = norad::designspace::DesignSpaceDocument::load(dir.join("Sample.designspace")).unwrap();
    assert_eq!(doc.sources.len(), 2);
    assert_eq!(doc.axes[0].tag, "wght");
    for (source, master) in doc.sources.iter().zip(font.masters.iter()) {
        assert_eq!(source.location[0].xvalue, Some(master.location[0]));
        let n = norad::Font::load(dir.join(&source.filename)).unwrap();
        same(&font, &master.id, &n);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Every file under `dir`, by path relative to it.
fn read_tree(dir: &Path) -> Files {
    fn walk(root: &Path, dir: &Path, out: &mut Files) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = Files::new();
    walk(dir, dir, &mut out);
    out
}

fn plist(body: &str) -> Vec<u8> {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n{body}\n</plist>\n").into_bytes()
}

/// A UFO 1 with renamed font-info keys, feature code and hint data in the lib, and a UFO 2-style
/// kerning group.
fn ufo1() -> Files {
    let mut files = Files::new();
    files.insert(
        "metainfo.plist".into(),
        plist("<dict><key>creator</key><string>org.robofab.ufoLib</string><key>formatVersion</key><integer>1</integer></dict>"),
    );
    files.insert(
        "fontinfo.plist".into(),
        plist(
            "<dict><key>familyName</key><string>Sample</string><key>styleName</key><string>Bold</string>\
             <key>unitsPerEm</key><integer>1000</integer><key>ascender</key><integer>750</integer>\
             <key>descender</key><integer>-250</integer><key>capHeight</key><integer>700</integer>\
             <key>xHeight</key><integer>500</integer><key>versionMajor</key><integer>1</integer>\
             <key>versionMinor</key><integer>2</integer><key>menuName</key><string>Sample</string>\
             <key>fontStyle</key><integer>32</integer><key>fontName</key><string>Sample-Bold</string>\
             <key>designer</key><string>A Designer</string><key>notice</key><string>A description</string>\
             <key>widthName</key><string>Condensed</string><key>weightValue</key><integer>700</integer>\
             <key>ttVendor</key><string>NONE</string><key>msCharSet</key><integer>0</integer></dict>",
        ),
    );
    files.insert(
        "lib.plist".into(),
        plist(
            "<dict><key>com.example.keep</key><string>yes</string>\
             <key>org.robofab.opentype.classes</key><string>@UC = [A B];\n</string>\
             <key>org.robofab.opentype.features</key><dict><key>kern</key><string>feature kern {\n    pos A B -20;\n} kern;\n</string></dict>\
             <key>org.robofab.opentype.featureorder</key><array><string>kern</string></array>\
             <key>org.robofab.postScriptHintData</key><dict><key>blueValues</key><array><array><integer>-10</integer><integer>0</integer></array>\
             <array><integer>700</integer><integer>710</integer></array></array><key>hStems</key><array><integer>80</integer></array></dict></dict>",
        ),
    );
    files.insert("groups.plist".into(), plist("<dict><key>@MMK_L_A</key><array><string>A</string></array></dict>"));
    files.insert("kerning.plist".into(), plist("<dict><key>@MMK_L_A</key><dict><key>B</key><integer>-20</integer></dict></dict>"));
    files.insert("glyphs/contents.plist".into(), plist("<dict><key>A</key><string>A_.glif</string><key>B</key><string>B_.glif</string></dict>"));
    files.insert(
        "glyphs/A_.glif".into(),
        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<glyph name=\"A\" format=\"1\"><advance width=\"600\"/><unicode hex=\"0041\"/><outline>\
          <contour><point x=\"0\" y=\"0\" type=\"line\"/><point x=\"600\" y=\"0\" type=\"line\"/><point x=\"300\" y=\"700\" type=\"line\"/></contour>\
          <contour><point x=\"300\" y=\"700\" type=\"move\" name=\"top\"/></contour></outline></glyph>\n"
            .to_vec(),
    );
    files.insert(
        "glyphs/B_.glif".into(),
        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<glyph name=\"B\" format=\"1\"><advance width=\"600\"/><unicode hex=\"0042\"/><outline>\
          <component base=\"A\" xOffset=\"10\"/></outline></glyph>\n"
            .to_vec(),
    );
    files
}

#[test]
fn a_ufo1_opens_as_the_ufo3_norad_converts_it_to() {
    let files = ufo1();
    let dir = temp("ufo1");
    write_tree(&dir.join("One.ufo"), &files);
    // norad up-converts on loading and saves UFO 3.
    norad::Font::load(dir.join("One.ufo")).unwrap().save(dir.join("Three.ufo")).unwrap();
    let three = read_tree(&dir.join("Three.ufo"));
    std::fs::remove_dir_all(&dir).unwrap();

    let ours = ufo::read(&files).unwrap();
    let theirs = ufo::read(&three).unwrap();
    assert_eq!(ours.source.ufo_version, Some(3));
    assert_eq!(
        (&ours.family_name, ours.units_per_em, ours.version_major, ours.version_minor),
        (&theirs.family_name, theirs.units_per_em, theirs.version_major, theirs.version_minor)
    );
    assert_eq!(ours.info, theirs.info, "font info");
    let (a, b) = (&ours.masters[0], &theirs.masters[0]);
    assert_eq!((&a.name, &a.metric_values, &a.zones, &a.stem_values), (&b.name, &b.metric_values, &b.zones, &b.stem_values));
    assert_eq!(a.user_data, b.user_data, "the lib without the keys moved out of it");
    assert_eq!(ours.kerning, theirs.kerning);
    assert_eq!(ours.features, theirs.features);
    // norad keeps the old kerning group as well; it is a plain group in its UFO 3.
    assert!(ours.groups.is_empty());
    assert_eq!(theirs.groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["@MMK_L_A"]);
    for (g, h) in ours.glyphs.iter().zip(theirs.glyphs.iter()) {
        assert_eq!((&g.name, &g.unicodes, &g.kern_groups), (&h.name, &h.unicodes, &h.kern_groups));
        let (l, m) = (g.layer(&a.id).unwrap(), h.layer(&b.id).unwrap());
        assert_eq!(l.width, m.width);
        assert_eq!(
            l.anchors.iter().map(|a| (&a.name, a.x, a.y)).collect::<Vec<_>>(),
            m.anchors.iter().map(|a| (&a.name, a.x, a.y)).collect::<Vec<_>>()
        );
        let shapes = |l: &fontcraft_model::Layer| {
            l.shapes
                .iter()
                .map(|s| match s {
                    Shape::Path(p) => format!("{:?}", p.nodes.iter().map(|n| (n.x, n.y, n.kind)).collect::<Vec<_>>()),
                    Shape::Component(c) => format!("{} {:?}", c.glyph, c.transform),
                    other => format!("{other:?}"),
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(shapes(l), shapes(m), "{}", g.name);
    }
}
