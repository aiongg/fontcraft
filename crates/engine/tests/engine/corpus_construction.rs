//! Measure (M13 plan § 5, construction rules): a rules file written for up to 200 of Inter's
//! accented glyphs rebuilds them with component positions within 1 unit of the stored ones.
//!
//! The rules are written from the source itself: for each composite whose first component sits
//! at the origin and whose other components are plain moves, each mark is attached by the anchor
//! its `_name` anchor pairs with on the nearest component before it (`@top`, or `@G:top,G:top`
//! when that component is not the one just before). Glyphs where a mark names an alternative
//! anchor, a part repeats, or no anchor pairs are left out. The rules then rebuild those glyphs in
//! every master, and each component's position is compared with the stored one.
//!
//! `cargo test -p fontcraft-engine --test engine corpus_construction:: -- --ignored --nocapture`.
//! `FONTCRAFT_CORPUS` names the corpus folder (default: the repository's `corpus/`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use fontcraft_components::propagate::effective_anchors_with;
use fontcraft_engine::Session;
use fontcraft_engine::model::{Font, Shape, Transform};
use serde_json::json;

const MAX_GLYPHS: usize = 200;

fn corpus() -> PathBuf {
    std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from)
}

/// The offset of a plain move, `None` for any other transform.
fn offset(t: &Transform) -> Option<(f64, f64)> {
    match *t {
        Transform::Decomposed { pos, scale, angle, slant } if scale == (1.0, 1.0) && angle == 0.0 && slant == (0.0, 0.0) => Some(pos),
        Transform::Matrix([a, b, c, d, e, f]) if [a, b, c, d] == [1.0, 0.0, 0.0, 1.0] => Some((e, f)),
        _ => None,
    }
}

/// A rule rebuilding `glyph` from its first master's components, or `None` when the rule format
/// cannot say how its marks attach.
fn rule_for(font: &Font, data: &fontcraft_engine::model::glyphdata::GlyphData, glyph: &str) -> Option<String> {
    let master = font.masters.first()?.id.clone();
    let g = font.glyph(glyph)?;
    let layer = g.layer(&master)?;
    let comps: Vec<_> = layer
        .shapes
        .iter()
        .map(|s| match s {
            Shape::Component(c) => Some(c),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    if comps.len() < 2 || offset(&comps[0].transform)? != (0.0, 0.0) {
        return None;
    }
    let names: Vec<&str> = comps.iter().map(|c| c.glyph.as_str()).collect();
    if names.iter().collect::<HashSet<_>>().len() != names.len() || names.iter().any(|n| font.glyph(n).is_none()) {
        return None;
    }
    let anchors = |name: &str| -> Vec<String> {
        effective_anchors_with(font, Some(data), name, &master).map(|v| v.into_iter().map(|a| a.name).collect()).unwrap_or_default()
    };
    let mut parts = vec![names[0].to_string()];
    for (k, c) in comps.iter().enumerate().skip(1) {
        offset(&c.transform)?;
        if c.info.anchor.is_some() {
            return None;
        }
        let own = anchors(&c.glyph);
        let found = own
            .iter()
            .filter_map(|a| a.strip_prefix('_'))
            .find_map(|name| (0..k).rev().find(|j| anchors(names[*j]).iter().any(|a| a == name)).map(|j| (name.to_string(), j)));
        let (name, j) = found?;
        parts.push(if j + 1 == k { format!("{}@{name}", c.glyph) } else { format!("{}@{}:{name},{}:{name}", c.glyph, names[j], names[j]) });
    }
    Some(format!("{glyph} = {}", parts.join(" + ")))
}

#[test]
#[ignore = "reads the corpus: the M13 construction-rules measure"]
fn corpus_construction_rebuilds_inter_accents() {
    let path = corpus().join("inter/src/Inter-Roman.glyphspackage");
    if !path.exists() {
        eprintln!("corpus_construction: {} is missing; run `cargo xtask corpus --fetch`", path.display());
        return;
    }
    let (font, _) = fontcraft_engine::io::open(&path, None).unwrap();
    let mut s = Session::new();
    s.add_document(font.clone(), None).unwrap();
    let data = s.glyph_data().unwrap();
    let mut rules = Vec::new();
    let mut chosen = Vec::new();
    for g in font.glyphs.iter() {
        if chosen.len() == MAX_GLYPHS {
            break;
        }
        if let Some(rule) = rule_for(&font, &data, &g.name) {
            rules.push(rule);
            chosen.push(g.name.clone());
        }
    }
    let text = rules.join("\n");
    let r = s.execute("font.buildFromConstructions", &json!({"rules": text, "autoUnicodes": false})).unwrap();
    assert_eq!(r["errors"], json!([]), "every rule builds");
    let after = s.font().unwrap();

    let (mut checked, mut within, mut glyphs_ok) = (0usize, 0usize, 0usize);
    let mut misses = Vec::new();
    for name in &chosen {
        let (old, new) = (font.glyph(name).unwrap(), after.glyph(name).unwrap());
        let mut all = true;
        for m in font.masters.iter() {
            let (Some(ol), Some(nl)) = (old.layer(&m.id), new.layer(&m.id)) else { continue };
            for (o, n) in ol.shapes.iter().zip(&nl.shapes) {
                let (Shape::Component(o), Shape::Component(n)) = (o, n) else { continue };
                let (Some(a), Some(b)) = (offset(&o.transform), offset(&n.transform)) else { continue };
                checked += 1;
                if (a.0 - b.0).abs() <= 1.0 && (a.1 - b.1).abs() <= 1.0 {
                    within += 1;
                } else {
                    all = false;
                    if misses.len() < 30 {
                        misses.push(format!("{name} [{}] {}: stored {a:?}, built {b:?}", m.name, o.glyph));
                    }
                }
            }
        }
        if all {
            glyphs_ok += 1;
        }
    }
    eprintln!(
        "construction measure: {} rules; {glyphs_ok} glyphs fully within 1 unit; {within} of {checked} component positions within 1 unit",
        chosen.len()
    );
    for m in &misses {
        eprintln!("  {m}");
    }
    assert!(chosen.len() >= MAX_GLYPHS, "found only {} glyphs the rules can describe", chosen.len());
    assert!(glyphs_ok * 100 >= chosen.len() * 95, "{glyphs_ok} of {} glyphs within 1 unit", chosen.len());
}
