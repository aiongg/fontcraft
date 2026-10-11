use fontcraft_model::{Anchor, Font, PartProperty};

use crate::checks::components::tests::{Tt, W, cmap, glyph, outlines, post, sfnt};
use crate::{AuditContext, BinaryCtx, Config, LayerReport, NoServices, SourceCtx, SourceMap, audit_binary, audit_layer, audit_source, find};

fn anchored(name: &str, anchors: &[&str]) -> fontcraft_model::Glyph {
    let mut g = glyph(name, vec![]);
    g.layers[0].anchors = anchors.iter().enumerate().map(|(i, a)| Anchor::new(*a, 100.0 * i as f64, 500.0)).collect();
    g
}

#[test]
fn duplicate_anchor_names_are_reported_after_the_first() {
    let mut font = Font::new("Test");
    font.glyphs.push(anchored("a", &["top", "top", "bottom"]));
    font.glyphs.push(anchored("b", &["top", "bottom"]));
    let ctx = AuditContext::new(&font);
    let spec = find("check.anchors.duplicateName").unwrap();
    let run = |name: &str| {
        let g = font.glyph(name).unwrap();
        audit_layer(&ctx, g, &g.layers[0], &[spec], &Config::default())
    };
    let r = run("a");
    assert_eq!(r.findings.len(), 1);
    assert_eq!(r.findings[0].location.anchors, vec!["top".to_string()]);
    assert_eq!(r.findings[0].location.point, Some([100.0, 500.0]), "the second one is reported");
    assert!(run("b").findings.is_empty());
}

#[test]
fn anchors_without_a_partner_are_reported() {
    let mut font = Font::new("Test");
    let mut smart = anchored("_part.shoulder", &["Width"]);
    smart.info.parts.push(PartProperty {
        name: "Width".into(),
        bottom_value: 0.0,
        top_value: 1.0,
        bottom_name: None,
        top_name: None,
        extra: Default::default(),
    });
    for g in [
        anchored("a", &["top", "bottom", "top_viet", "ogonek", "*top"]),
        anchored("acutecomb", &["_top", "top"]),
        anchored("dotbelowcomb", &["_bottom"]),
        anchored("cedillacomb", &["_cedilla"]),
        anchored("f_i", &["caret_1"]),
        anchored("beh-ar.init", &["exit", "#entry", "origin"]),
        anchored("_corner.serif", &["right"]),
        smart,
    ] {
        font.glyphs.push(g);
    }
    let ctx = AuditContext::new(&font);
    let source = SourceCtx { audit: &ctx, services: &NoServices };
    let r = audit_source(&source, &[find("check.anchors.unknownName").unwrap()], &Config::default());
    let got: Vec<(&str, &str)> = r.findings.iter().map(|f| (f.location.glyph.as_str(), f.location.anchors[0].as_str())).collect();
    assert_eq!(got, vec![("a", "ogonek"), ("cedillacomb", "_cedilla")]);
    assert!(r.findings.iter().all(|f| f.location.layer == "m01" && f.code == "unknown-name"));
}

/// `GSUB` with one `liga` lookup making `lig` from `(first, second)` for each triple.
fn gsub(ligatures: &[(u16, u16, u16)]) -> ([u8; 4], Vec<u8>) {
    let n = ligatures.len() as u16;
    let mut w = W::default();
    // Header (10), empty ScriptList at 10, FeatureList at 12 (14 bytes), LookupList at 26.
    w.u16(1).u16(0).u16(10).u16(12).u16(26);
    w.u16(0);
    w.u16(1).tag(b"liga").u16(8).u16(0).u16(1).u16(0);
    // LookupList: one lookup at +4; the lookup has one subtable at +8.
    w.u16(1).u16(4);
    w.u16(4).u16(0).u16(1).u16(8);
    // LigatureSubstFormat1: coverage after the set offsets, then one set per first glyph.
    let coverage = 6 + 2 * n;
    let sets = coverage + 4 + 2 * n;
    w.u16(1).u16(coverage).u16(n);
    for k in 0..n {
        w.u16(sets + 10 * k);
    }
    w.u16(1).u16(n);
    for (first, _, _) in ligatures {
        w.u16(*first);
    }
    for (_, second, lig) in ligatures {
        w.u16(1).u16(4).u16(*lig).u16(2).u16(*second);
    }
    (*b"GSUB", w.0)
}

/// `GDEF` with caret positions for `carets`, or without a caret list.
fn gdef(carets: Option<&[u16]>) -> ([u8; 4], Vec<u8>) {
    let mut w = W::default();
    let Some(gids) = carets else {
        w.u16(1).u16(0).u16(0).u16(0).u16(0).u16(0);
        return (*b"GDEF", w.0);
    };
    let n = gids.len() as u16;
    w.u16(1).u16(0).u16(0).u16(0).u16(12).u16(0);
    // LigCaretList: coverage after the offsets, then one LigGlyph (4) and CaretValue (4) each.
    let coverage = 4 + 2 * n;
    let glyphs = coverage + 4 + 2 * n;
    w.u16(coverage).u16(n);
    for k in 0..n {
        w.u16(glyphs + 8 * k);
    }
    w.u16(1).u16(n);
    for g in gids {
        w.u16(*g);
    }
    for _ in gids {
        w.u16(1).u16(4).u16(1).i16(300);
    }
    (*b"GDEF", w.0)
}

const NAMES: [&str; 6] = [".notdef", "f", "i", "l", "f_i", "f_l"];

fn liga_font(extra: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    let mut tables = outlines(&[Tt::Empty, Tt::Simple, Tt::Simple, Tt::Simple, Tt::Simple, Tt::Simple]);
    tables.push(post(&NAMES));
    tables.push(cmap(vec![(0x66, 1), (0x69, 2), (0x6C, 3)]));
    tables.extend(extra);
    sfnt(tables)
}

fn carets(bytes: &[u8], source: Option<&SourceMap>) -> LayerReport {
    let services = NoServices;
    let mut ctx = BinaryCtx::new("f0", bytes, 0, &services).unwrap();
    ctx.source = source;
    audit_binary(&ctx, &[find("check.anchors.ligatureCaretsMissing").unwrap()], &Config::default())
}

#[test]
fn ligatures_need_caret_positions() {
    // f i → f_i, and l l → f_l (the test font's coverage needs distinct first glyphs).
    let ligs = [(1, 2, 4), (3, 3, 5)];
    let codes = |r: &LayerReport| r.findings.iter().map(|f| (f.code, f.location.glyph.clone())).collect::<Vec<_>>();

    let none = carets(&liga_font(vec![gsub(&ligs)]), None);
    assert_eq!(codes(&none), vec![("no-gdef", String::new())]);
    assert!(none.findings[0].fix.is_none(), "a font given as a binary has no command to fix it");

    let empty = carets(&liga_font(vec![gsub(&ligs), gdef(None)]), None);
    assert_eq!(codes(&empty), vec![("no-carets", String::new())]);

    let partial = carets(&liga_font(vec![gsub(&ligs), gdef(Some(&[4]))]), None);
    assert_eq!(codes(&partial), vec![("incomplete-carets", "f_l".to_string())]);

    assert!(carets(&liga_font(vec![gsub(&ligs), gdef(Some(&[4, 5]))]), None).findings.is_empty());

    let skipped = carets(&liga_font(vec![]), None);
    assert!(skipped.findings.is_empty());
    assert_eq!(skipped.skipped.len(), 1, "no ligatures: nothing to check");

    // Compiled from a source: findings name source glyphs and carry the fix.
    let map = SourceMap { glyphs: [".notdef", "f", "i", "l", "f_i", "f_l.src"].map(String::from).to_vec(), ..SourceMap::default() };
    let from_source = carets(&liga_font(vec![gsub(&ligs), gdef(Some(&[4]))]), Some(&map));
    assert_eq!(codes(&from_source), vec![("incomplete-carets", "f_l.src".to_string())]);
    assert_eq!(from_source.findings[0].fix.and_then(|f| f.command), Some("glyph.setAnchors"));
}
