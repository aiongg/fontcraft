//! Reusing shapes end to end (M6 plan § 5): with commands only, as an agent runs them over the
//! control channel, a synthetic font gets anchors, composites, a recipe, a flipped mark, an offset
//! key, a smart component and a corner component, follows an edit of its base glyph, and is
//! exported with the composites, advances and mark attachment the commands asked for.
//!
//! [`inter_speed`] (ignored) is the plan's speed measure:
//! `cargo test --release -p fontcraft-engine --test engine m6_end_to_end:: -- --ignored
//! --nocapture`; `FONTCRAFT_CORPUS` names the corpus folder (default: the repository's `corpus/`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::time::Instant;

use fontcraft_components::align::{align_layer_cached, aligned_placement};
use fontcraft_components::compose::components_for;
use fontcraft_components::index::ComponentIndex;
use fontcraft_components::metrics::realign_glyphs;
use fontcraft_components::propagate::{AnchorCache, effective_anchors_for_master};
use fontcraft_engine::Session;
use fontcraft_engine::model::glyphdata::GlyphData;
use fontcraft_engine::model::{Layer, Shape};
use serde_json::{Value, json};
use skrifa::MetadataProvider;
use skrifa::raw::TableProvider;
use skrifa::raw::tables::glyf::{Anchor as GlyfAnchor, Glyph as GlyfGlyph};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn layer<'a>(s: &'a Session, glyph: &str) -> &'a Layer {
    s.font().unwrap().glyph(glyph).unwrap_or_else(|| panic!("no glyph {glyph}")).layer("m01").unwrap()
}

fn anchor(s: &Session, glyph: &str, name: &str) -> (f64, f64) {
    let a = layer(s, glyph).anchors.iter().find(|a| a.name == name).unwrap_or_else(|| panic!("{glyph} has no anchor {name}"));
    (a.x, a.y)
}

/// The components of a glyph as (base, x, y).
fn components(s: &Session, glyph: &str) -> Vec<(String, f64, f64)> {
    let parts = layer(s, glyph).shapes.iter().filter_map(|shape| match shape {
        Shape::Component(c) => {
            let k = c.affine().as_coeffs();
            Some((c.glyph.clone(), k[4], k[5]))
        }
        _ => None,
    });
    parts.collect()
}

/// Where a mark's `_top` puts it on a base's `top`.
fn on_top(s: &Session, base: &str, mark: &str) -> (f64, f64) {
    let (b, m) = (anchor(s, base, "top"), anchor(s, mark, "_top"));
    (b.0 - m.0, b.1 - m.1)
}

fn rect(s: &mut Session, glyph: &str, layer: &str, x: f64, y: f64, width: f64, height: f64) {
    run(s, "path.addRectangle", json!({"glyph": glyph, "layer": layer, "x": x, "y": y, "width": width, "height": height}));
}

#[test]
fn an_agent_builds_composites_and_exports_them() {
    let dir = std::env::temp_dir().join(format!("fontcraft-m6-end-to-end-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_document();
    run(&mut s, "glyph.addGlyphs", json!({"text": "A E n o idotless acutecomb gravecomb dieresiscomb ringcomb _part.bar _corner.one"}));
    let drawn = [
        ("A", 50.0, 0.0, 500.0, 700.0, 600.0),
        ("E", 60.0, 0.0, 400.0, 700.0, 520.0),
        ("n", 50.0, 0.0, 400.0, 500.0, 500.0),
        ("o", 40.0, 0.0, 420.0, 500.0, 500.0),
        ("idotless", 60.0, 0.0, 80.0, 500.0, 200.0),
        ("acutecomb", 20.0, 560.0, 160.0, 120.0, 200.0),
        ("gravecomb", 20.0, 560.0, 160.0, 120.0, 200.0),
        ("dieresiscomb", 10.0, 560.0, 240.0, 100.0, 260.0),
        ("ringcomb", 30.0, 560.0, 140.0, 140.0, 200.0),
    ];
    for (glyph, x, y, w, h, width) in drawn {
        rect(&mut s, glyph, "m01", x, y, w, h);
        run(&mut s, "spacing.setWidth", json!({"glyphs": [glyph], "value": width}));
    }

    // Set Anchors, then the composites.
    let glyphs: Vec<&str> = drawn.iter().map(|d| d.0).collect();
    let set = run(&mut s, "glyph.setAnchors", json!({"glyphs": glyphs}));
    assert!(set["added"].as_u64().unwrap() >= 18, "{set}");
    for (glyph, names) in [("A", ["bottom", "top"]), ("idotless", ["bottom", "top"]), ("acutecomb", ["_top", "top"])] {
        names.map(|n| anchor(&s, glyph, n));
    }
    let added = run(&mut s, "glyph.addGlyphs", json!({"text": "Agrave Eacute idieresis n+n=n_n"}));
    assert_eq!(added["skipped"], json!([]), "{added}");
    run(&mut s, "glyph.newGlyph", json!({"name": "Aring", "unicodes": [0xC5]}));
    let made = run(&mut s, "glyph.createComposite", json!({"glyph": "Aring"}));
    assert_eq!(made["glyphs"], json!([{"glyph": "Aring", "components": ["A", "ringcomb"]}]));
    for (glyph, base, mark, width) in [
        ("Agrave", "A", "gravecomb", 600.0),
        ("Eacute", "E", "acutecomb", 520.0),
        ("idieresis", "idotless", "dieresiscomb", 200.0),
        ("Aring", "A", "ringcomb", 600.0),
    ] {
        let (x, y) = on_top(&s, base, mark);
        assert_eq!(components(&s, glyph), [(base.to_string(), 0.0, 0.0), (mark.to_string(), x, y)], "{glyph}");
        assert_eq!(layer(&s, glyph).width, width, "{glyph}");
    }
    assert_eq!(components(&s, "n_n"), [("n".to_string(), 0.0, 0.0), ("n".to_string(), 500.0, 0.0)]);
    assert_eq!(layer(&s, "n_n").width, 1000.0);

    // A's left sidebearing moves its outline and anchors; every A composite follows.
    let ring = components(&s, "Aring")[1].1;
    run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["A"], "value": 80}));
    assert_eq!(components(&s, "Aring")[1].1, ring + 30.0);
    for glyph in ["A", "Aring", "Agrave"] {
        assert_eq!(layer(&s, glyph).width, 630.0, "{glyph}");
    }

    // A vertically flipped mark hangs from the bottom anchor.
    run(&mut s, "component.setTransform", json!({"glyph": "Agrave", "component": 1, "scaleY": -100}));
    let (bottom, mark) = (anchor(&s, "A", "bottom"), anchor(&s, "gravecomb", "_top"));
    assert_eq!(components(&s, "Agrave")[1], ("gravecomb".to_string(), bottom.0 - mark.0, bottom.1 + mark.1));

    // An offset key shifts the composite and widens it.
    run(&mut s, "spacing.setLeftSidebearing", json!({"glyphs": ["idieresis"], "value": "=+20"}));
    let (x, y) = on_top(&s, "idotless", "dieresiscomb");
    assert_eq!(components(&s, "idieresis"), [("idotless".to_string(), 20.0, 0.0), ("dieresiscomb".to_string(), x + 20.0, y)]);
    assert_eq!(layer(&s, "idieresis").width, 220.0);

    // A smart component halfway along its Width, and a corner component.
    let wide = run(&mut s, "layers.addBackupLayer", json!({"glyphs": ["_part.bar"], "name": "Wide"}));
    let wide = wide["layer"].as_str().unwrap().to_string();
    rect(&mut s, "_part.bar", "m01", 0.0, 0.0, 200.0, 60.0);
    rect(&mut s, "_part.bar", &wide, 0.0, 0.0, 600.0, 60.0);
    run(&mut s, "smartGlyph.addProperty", json!({"glyph": "_part.bar", "name": "Width", "min": 200, "max": 600}));
    run(&mut s, "smartGlyph.setLayerPole", json!({"glyph": "_part.bar", "layer": "m01", "property": "Width", "pole": "min"}));
    run(&mut s, "smartGlyph.setLayerPole", json!({"glyph": "_part.bar", "layer": wide, "property": "Width", "pole": "max"}));
    let part = run(&mut s, "glyph.addComponent", json!({"glyph": "E", "component": "_part.bar"}));
    run(&mut s, "smartComponent.setValue", json!({"glyph": "E", "component": part["shape"], "property": "Width", "value": 400}));
    let corner = json!([{"x": 0, "y": 50}, {"x": -50, "y": 50}, {"x": -50, "y": 0}, {"x": 25, "y": 0}]);
    run(&mut s, "path.addPath", json!({"glyph": "_corner.one", "closed": false, "nodes": corner}));
    run(&mut s, "cornerComponent.add", json!({"glyph": "o", "points": [[0, 0]], "corner": "_corner.one"}));
    run(&mut s, "glyph.decomposeGlyph", json!({"glyph": "Aring"}));
    assert!(layer(&s, "Aring").shapes.iter().all(|shape| matches!(shape, Shape::Path(_))));

    // The exported font.
    let ttf = dir.join("out.ttf");
    run(&mut s, "font.compile", json!({"destination": ttf, "timestamp": 0}));
    let bytes = std::fs::read(&ttf).unwrap();
    let font = skrifa::FontRef::new(&bytes).unwrap();
    let (loca, glyf) = (font.loca(None).unwrap(), font.glyf().unwrap());
    let gid = |c: char| font.charmap().map(c).unwrap();
    let composite = |c: char| -> Option<Vec<(u32, i16, i16)>> {
        let Some(GlyfGlyph::Composite(g)) = loca.get_glyf(gid(c), &glyf).unwrap() else { return None };
        let parts = g.components().map(|part| match part.anchor {
            GlyfAnchor::Offset { x, y } => (part.glyph.to_u32(), x, y),
            GlyfAnchor::Point { .. } => panic!("a point-matched component"),
        });
        Some(parts.collect())
    };
    let offset = |glyph: &str, i: usize| {
        let (_, x, y) = components(&s, glyph)[i].clone();
        (x.round() as i16, y.round() as i16)
    };
    for (c, glyph, base) in [('\u{C9}', "Eacute", 'E'), ('\u{EF}', "idieresis", '\u{131}'), ('\u{C0}', "Agrave", 'A')] {
        let parts = composite(c).unwrap_or_else(|| panic!("{glyph} is a composite"));
        assert_eq!(parts.len(), 2, "{glyph}");
        assert_eq!(parts[0].0, gid(base).to_u32(), "{glyph}");
        for (i, part) in parts.iter().enumerate() {
            assert_eq!((part.1, part.2), offset(glyph, i), "{glyph} component {i}");
        }
    }
    assert!(composite('\u{C5}').is_none(), "Aring was decomposed");
    let hmtx = font.hmtx().unwrap();
    for (c, width) in [('A', 630), ('\u{C0}', 630), ('\u{C5}', 630), ('\u{C9}', 520), ('\u{EF}', 220), ('n', 500)] {
        assert_eq!(hmtx.advance(gid(c)), Some(width), "{c}");
    }
    // E holds a path beside the smart component, which decomposes: from 60 to 460, and 0 to 400.
    let x_range = |c: char| {
        let g = loca.get_glyf(gid(c), &glyf).unwrap().unwrap();
        (g.x_min(), g.x_max())
    };
    assert_eq!(x_range('E'), (0, 460), "the smart part is 400 wide");
    assert!(matches!(loca.get_glyf(gid('o'), &glyf).unwrap(), Some(GlyfGlyph::Simple(g)) if g.num_points() > 4), "the corner is fitted");

    let code = run(&mut s, "layout.decompileFeatures", json!({"font": ttf, "tables": ["GPOS"], "classes": "inline"}));
    let code = code["text"].as_str().unwrap().split_whitespace().collect::<Vec<_>>().join(" ");
    for (glyph, name) in [("A", "top"), ("idotless", "top"), ("acutecomb", "_top"), ("ringcomb", "_top")] {
        let (x, y) = anchor(&s, glyph, name);
        let statement = format!("<anchor {} {}>", x.round(), y.round());
        assert!(code.contains(&statement), "{glyph} {name} {statement}: {code}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn timed<T>(what: &str, limit: f64, f: impl Fn() -> T) -> bool {
    let mut times: Vec<f64> = (0..5)
        .map(|_| {
            let start = Instant::now();
            std::hint::black_box(f());
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(f64::total_cmp);
    eprintln!("{what}: median {:.2} ms (target {limit} ms), runs {times:.2?}", times[2]);
    times[2] < limit
}

#[test]
#[ignore = "speed measure on Inter Roman; run with --release --ignored"]
fn inter_speed() {
    let corpus = std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from);
    let path = corpus.join("inter/src/Inter-Roman.glyphspackage");
    if !path.exists() {
        eprintln!("skipped: {} is not fetched", path.display());
        return;
    }
    let (font, _) = fontcraft_engine::io::open(&path, None).unwrap();
    let data = GlyphData::new().unwrap();
    let master = font.masters[0].id.clone();
    let mut ok = vec![timed("reverse index", 10.0, || ComponentIndex::new(&font))];
    ok.push(timed("propagation, all glyphs on all masters", 100.0, || {
        font.masters.iter().map(|m| effective_anchors_for_master(&font, &m.id).unwrap().anchors.len()).sum::<usize>()
    }));
    ok.push(timed("aligning every eligible master layer", 200.0, || {
        let mut cache = AnchorCache::new();
        let mut changed = 0;
        for g in font.glyphs.iter() {
            for l in g.layers.iter().filter(|l| l.is_master_layer() && l.shapes.iter().any(|s| matches!(s, Shape::Component(_)))) {
                changed += usize::from(align_layer_cached(&font, g, l, &mut cache).is_ok_and(|l| l.is_some()));
            }
        }
        changed
    }));
    // An edit of A and o on one master, then the step that re-aligns what uses them.
    let mut edited = font.clone();
    for name in ["A", "o"] {
        let mut g = edited.glyph_mut(name).unwrap();
        for a in &mut g.layer_mut(&master).unwrap().anchors {
            a.x += 10.0;
        }
    }
    let changed = ["A".to_string(), "o".to_string()];
    ok.push(timed("re-aligning the dependents of A and o", 5.0, || realign_glyphs(edited.clone(), &changed).unwrap()));
    ok.push(timed("naming the composites of every glyph", 500.0, || {
        font.glyphs.iter().filter(|g| components_for(&font, &data, &g.name).is_ok_and(|c| c.is_some())).count()
    }));
    let aligned = font.glyphs.iter().filter(|g| aligned_placement(&font, &g.name, &master).is_ok_and(|p| !p.is_empty())).count();
    eprintln!("{} glyphs, {} masters, {aligned} aligned on the first master", font.glyphs.as_arcs().len(), font.masters.len());
    // Create Composite itself, through the command.
    let mut s = Session::new();
    s.add_document(font.clone(), None).unwrap();
    let names: Vec<&str> =
        font.glyphs.iter().filter(|g| components_for(&font, &data, &g.name).is_ok_and(|c| c.is_some())).map(|g| g.name.as_str()).collect();
    let start = Instant::now();
    let made = s.execute("glyph.createComposite", &json!({"glyphs": names})).unwrap();
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    eprintln!("Create Composite on {} glyphs: {ms:.1} ms (target 500 ms)", made["glyphs"].as_array().map_or(0, Vec::len));
    ok.push(ms < 500.0);
    assert!(ok.iter().all(|t| *t), "a speed target is missed: {ok:?}");
}
