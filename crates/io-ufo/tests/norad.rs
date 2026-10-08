//! norad, an independent UFO and designspace reader, reads what we write: the same glyphs, code
//! points, advances, point counts and components, and the same sources and locations.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use fontcraft_io_ufo::{Files, designspace, ufo};
use fontcraft_model::{Axis, Component, Extra, Font, Node, NodeKind, Path as Contour, Shape};

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
    Arc::make_mut(&mut font.axes).push(Axis { name: "Weight".into(), tag: "wght".into(), hidden: false, extra: Extra::new() });
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
