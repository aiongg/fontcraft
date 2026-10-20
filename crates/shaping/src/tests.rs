//! Shaping tests on fonts written byte by byte here: `cmap`, metrics and a `GSUB` whose lookups
//! the tests name. No compiled font is committed.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::*;

// Glyph ids of the test font.
const F: u16 = 1;
const I: u16 = 2;
const F_I: u16 = 3;
const G_ALT: u16 = 4;
const G_ALT_SS01: u16 = 5;
const BEH: u16 = 6;
const BEH_INIT: u16 = 7;
const BEH_MEDI: u16 = 8;
const BEH_FINA: u16 = 9;
const KA: u16 = 10;
const SIGN_I: u16 = 11;
const VIRAMA: u16 = 12;
const SSA: u16 = 13;
const K_SSA: u16 = 14;
const PAREN: u16 = 15;
const PAREN_VERT: u16 = 16;
const SPACE: u16 = 17;
const GLYPHS: u16 = 18;

/// Where the test font's unencoded glyphs live, as the preview font assigns them.
const PUA_G_ALT: u32 = 0xF0000;
const PUA_G_ALT_SS01: u32 = 0xF0001;

#[derive(Default)]
struct W(Vec<u8>);

impl W {
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn i16(&mut self, v: i16) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn tag(&mut self, t: &[u8; 4]) -> &mut Self {
        self.0.extend_from_slice(t);
        self
    }
    fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }
    fn len16(&self) -> u16 {
        u16::try_from(self.0.len()).expect("test tables stay under 64 KB")
    }
}

/// An sfnt with these tables (checksums are not computed; nothing here reads them).
fn sfnt(mut tables: Vec<(&[u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by_key(|(t, _)| **t);
    let mut w = W::default();
    w.u32(0x0001_0000).u16(tables.len() as u16).u16(0).u16(0).u16(0);
    let mut offset = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, data) in &tables {
        w.tag(tag).u32(0).u32(offset as u32).u32(data.len() as u32);
        let mut padded = data.clone();
        while padded.len() % 4 != 0 {
            padded.push(0);
        }
        offset += padded.len();
        body.extend(padded);
    }
    w.bytes(&body);
    w.0
}

fn head() -> Vec<u8> {
    let mut w = W::default();
    w.u32(0x0001_0000).u32(0x0001_0000).u32(0).u32(0x5F0F_3CF5).u16(0).u16(1000);
    w.u32(0).u32(0).u32(0).u32(0); // created, modified
    w.i16(0).i16(-200).i16(1000).i16(800); // bounding box
    w.u16(0).u16(8).i16(2).i16(0).i16(0); // macStyle, lowestRecPPEM, direction hint, loca, glyf
    w.0
}

/// `hhea` (ascender 800, descender -200, no line gap) and `hmtx` with these advance widths.
fn metrics(widths: &[u16]) -> (Vec<u8>, Vec<u8>) {
    let mut h = W::default();
    h.u32(0x0001_0000).i16(800).i16(-200).i16(0).u16(widths.iter().copied().max().unwrap_or(0));
    h.i16(0).i16(0).i16(0); // minLeftSideBearing, minRightSideBearing, xMaxExtent
    h.i16(1).i16(0).i16(0); // caretSlopeRise, caretSlopeRun, caretOffset: an upright caret
    h.i16(0).i16(0).i16(0).i16(0).i16(0); // reserved, metricDataFormat
    h.u16(widths.len() as u16);
    let mut m = W::default();
    for width in widths {
        m.u16(*width).i16(0);
    }
    (h.0, m.0)
}

fn maxp(glyphs: u16) -> Vec<u8> {
    let mut w = W::default();
    w.u32(0x0000_5000).u16(glyphs);
    w.0
}

/// `cmap` with one format 12 subtable (platform 3, encoding 10).
fn cmap(map: &[(u32, u16)]) -> Vec<u8> {
    let mut sorted = map.to_vec();
    sorted.sort_by_key(|(c, _)| *c);
    let mut w = W::default();
    w.u16(0).u16(1).u16(3).u16(10).u32(12);
    w.u16(12).u16(0).u32(16 + 12 * sorted.len() as u32).u32(0).u32(sorted.len() as u32);
    for (c, g) in &sorted {
        w.u32(*c).u32(*c).u32(u32::from(*g));
    }
    w.0
}

fn coverage(glyphs: &[u16]) -> Vec<u8> {
    let mut w = W::default();
    w.u16(1).u16(glyphs.len() as u16);
    for g in glyphs {
        w.u16(*g);
    }
    w.0
}

/// A lookup table of `kind` with one subtable.
fn lookup(kind: u16, subtable: &[u8]) -> Vec<u8> {
    let mut w = W::default();
    w.u16(kind).u16(0).u16(1).u16(8).bytes(subtable);
    w.0
}

/// Single substitution, format 2.
fn single(map: &[(u16, u16)]) -> Vec<u8> {
    let mut sorted = map.to_vec();
    sorted.sort_by_key(|(from, _)| *from);
    let from: Vec<u16> = sorted.iter().map(|(f, _)| *f).collect();
    let mut w = W::default();
    w.u16(2).u16(6 + 2 * sorted.len() as u16).u16(sorted.len() as u16);
    for (_, to) in &sorted {
        w.u16(*to);
    }
    w.bytes(&coverage(&from));
    lookup(1, &w.0)
}

/// Ligature substitution, format 1: (components, ligature glyph).
fn ligature(ligatures: &[(&[u16], u16)]) -> Vec<u8> {
    let mut by_first: BTreeMap<u16, Vec<(&[u16], u16)>> = BTreeMap::new();
    for (components, glyph) in ligatures {
        by_first.entry(components[0]).or_default().push((components, *glyph));
    }
    let sets: Vec<Vec<u8>> = by_first
        .values()
        .map(|ligs| {
            let tables: Vec<Vec<u8>> = ligs
                .iter()
                .map(|(components, glyph)| {
                    let mut l = W::default();
                    l.u16(*glyph).u16(components.len() as u16);
                    for c in &components[1..] {
                        l.u16(*c);
                    }
                    l.0
                })
                .collect();
            let mut set = W::default();
            set.u16(tables.len() as u16);
            let mut offset = 2 + 2 * tables.len() as u16;
            for t in &tables {
                set.u16(offset);
                offset += t.len() as u16;
            }
            for t in &tables {
                set.bytes(t);
            }
            set.0
        })
        .collect();
    let firsts: Vec<u16> = by_first.keys().copied().collect();
    let header = 6 + 2 * sets.len() as u16;
    let sets_len: usize = sets.iter().map(Vec::len).sum();
    let mut w = W::default();
    w.u16(1).u16(header + sets_len as u16).u16(sets.len() as u16);
    let mut offset = header;
    for s in &sets {
        w.u16(offset);
        offset += s.len() as u16;
    }
    for s in &sets {
        w.bytes(s);
    }
    w.bytes(&coverage(&firsts));
    lookup(4, &w.0)
}

/// Contextual substitution, format 3, on one glyph: each record applies a lookup at it.
fn context_on(glyph: u16, records: &[u16]) -> Vec<u8> {
    let mut w = W::default();
    let header = 8 + 4 * records.len() as u16;
    w.u16(3).u16(1).u16(records.len() as u16).u16(header);
    for lookup_index in records {
        w.u16(0).u16(*lookup_index);
    }
    w.bytes(&coverage(&[glyph]));
    lookup(5, &w.0)
}

/// A `GSUB` whose scripts (`DFLT`, `arab`, `dev2`, `latn`) all list every feature;
/// `features` must be sorted by tag.
fn gsub(features: &[(&[u8; 4], &[u16])], lookups: &[Vec<u8>]) -> Vec<u8> {
    let scripts: [&[u8; 4]; 4] = [b"DFLT", b"arab", b"dev2", b"latn"];
    let mut script_list = W::default();
    script_list.u16(scripts.len() as u16);
    let script_offset = 2 + 6 * scripts.len() as u16;
    for tag in scripts {
        script_list.tag(tag).u16(script_offset);
    }
    // Script table: the default language system right after its 4-byte header.
    script_list.u16(4).u16(0);
    script_list.u16(0).u16(0xFFFF).u16(features.len() as u16);
    for index in 0..features.len() {
        script_list.u16(index as u16);
    }

    let mut feature_list = W::default();
    feature_list.u16(features.len() as u16);
    let mut offset = 2 + 6 * features.len() as u16;
    for (tag, indices) in features {
        feature_list.tag(tag).u16(offset);
        offset += 4 + 2 * indices.len() as u16;
    }
    for (_, indices) in features {
        feature_list.u16(0).u16(indices.len() as u16);
        for i in *indices {
            feature_list.u16(*i);
        }
    }

    let mut lookup_list = W::default();
    lookup_list.u16(lookups.len() as u16);
    let mut offset = 2 + 2 * lookups.len() as u16;
    for l in lookups {
        lookup_list.u16(offset);
        offset += l.len() as u16;
    }
    for l in lookups {
        lookup_list.bytes(l);
    }

    let mut w = W::default();
    let scripts_at = 10;
    let features_at = scripts_at + script_list.len16();
    let lookups_at = features_at + feature_list.len16();
    w.u16(1).u16(0).u16(scripts_at).u16(features_at).u16(lookups_at);
    w.bytes(&script_list.0).bytes(&feature_list.0).bytes(&lookup_list.0);
    w.0
}

fn font_bytes(glyphs: u16, widths: &[u16], cmap_entries: &[(u32, u16)], gsub_table: Vec<u8>) -> Arc<[u8]> {
    let (hhea, hmtx) = metrics(widths);
    let bytes = sfnt(vec![
        (b"head", head()),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"maxp", maxp(glyphs)),
        (b"cmap", cmap(cmap_entries)),
        (b"GSUB", gsub_table),
    ]);
    Arc::from(bytes)
}

/// The test font: Latin `f i` with an `f_i` ligature, an unencoded `g.alt` with an `ss01`
/// alternate, Arabic beh with its positional forms, Devanagari ka, sign i, virama and ssa with
/// a `k_ssa` conjunct, and a parenthesis with a vertical form.
fn test_font() -> ShapingFont {
    let widths: Vec<u16> = (0..GLYPHS).map(|g| 100 + 10 * g).collect();
    let cmap_entries =
        [(0x20, SPACE), (0x28, PAREN), (0x66, F), (0x69, I), (0x0628, BEH), (0x0915, KA), (0x0937, SSA), (0x093F, SIGN_I), (0x094D, VIRAMA)];
    let lookups = vec![
        ligature(&[(&[F, I][..], F_I)]),              // 0 liga
        single(&[(BEH, BEH_INIT)]),                   // 1 init
        single(&[(BEH, BEH_MEDI)]),                   // 2 medi
        single(&[(BEH, BEH_FINA)]),                   // 3 fina
        ligature(&[(&[KA, VIRAMA, SSA][..], K_SSA)]), // 4 akhn
        single(&[(G_ALT, G_ALT_SS01)]),               // 5 ss01
        single(&[(PAREN, PAREN_VERT)]),               // 6 vert
    ];
    let features: [(&[u8; 4], &[u16]); 7] =
        [(b"akhn", &[4]), (b"fina", &[3]), (b"init", &[1]), (b"liga", &[0]), (b"medi", &[2]), (b"ss01", &[5]), (b"vert", &[6])];
    let bytes = font_bytes(GLYPHS, &widths, &cmap_entries, gsub(&features, &lookups));
    ShapingFont::new(bytes, &[(PUA_G_ALT, G_ALT), (PUA_G_ALT_SS01, G_ALT_SS01)]).expect("the test font reads")
}

fn width(glyph: u16) -> i32 {
    100 + 10 * i32::from(glyph)
}

fn glyphs(out: &[ShapedGlyph]) -> Vec<u16> {
    out.iter().map(|g| g.glyph).collect()
}

#[test]
fn bytes_that_are_not_a_font_are_refused() {
    assert!(matches!(ShapingFont::new(Arc::from(&b"not a font"[..]), &[]), Err(ShapeError::Font(_))));
}

#[test]
fn shaping_font_can_be_shared_between_threads() {
    fn shared<T: Send + Sync>() {}
    shared::<ShapingFont>();
}

#[test]
fn latin_ligature_is_formed_with_the_font_advance() {
    let font = test_font();
    let out = shape(&font, &ShapeRequest::new("fi")).expect("shapes");
    assert_eq!(glyphs(&out), vec![F_I]);
    assert_eq!(out.first().map(|g| (g.cluster, g.x_advance)), Some((0, width(F_I))));
    // liga turned off keeps the letters.
    let off = [FeatureSetting::global(Tag::new(b"liga"), false)];
    let out = shape(&font, &ShapeRequest { features: &off, ..ShapeRequest::new("fi") }).expect("shapes");
    assert_eq!(glyphs(&out), vec![F, I]);
}

#[test]
fn live_advances_replace_the_font_widths() {
    let font = test_font();
    let live = |glyph: u16| (glyph == F_I).then_some(777_i32);
    let req = ShapeRequest { advances: Some(&live), ..ShapeRequest::new("fi f") };
    let out = shape(&font, &req).expect("shapes");
    let advances: Vec<(u16, i32)> = out.iter().map(|g| (g.glyph, g.x_advance)).collect();
    assert_eq!(advances, vec![(F_I, 777), (SPACE, width(SPACE)), (F, width(F))]);
}

#[test]
fn arabic_takes_its_positional_forms_in_visual_order() {
    let font = test_font();
    let req = ShapeRequest { direction: Direction::RightToLeft, script: Some(Tag::new(b"Arab")), ..ShapeRequest::new("\u{0628}\u{0628}\u{0628}") };
    let out = shape(&font, &req).expect("shapes");
    // Logical init, medi, fina; drawn right to left, so the final form comes first.
    assert_eq!(glyphs(&out), vec![BEH_FINA, BEH_MEDI, BEH_INIT]);
    assert_eq!(out.iter().map(|g| g.cluster).collect::<Vec<_>>(), vec![4, 2, 0]);
}

#[test]
fn devanagari_reorders_the_sign_i_and_forms_the_conjunct() {
    let font = test_font();
    fn deva(text: &str) -> ShapeRequest<'_> {
        ShapeRequest { script: Some(Tag::new(b"Deva")), ..ShapeRequest::new(text) }
    }
    // The vowel sign i is written after ka and drawn before it.
    assert_eq!(glyphs(&shape(&font, &deva("\u{0915}\u{093F}")).expect("shapes")), vec![SIGN_I, KA]);
    assert_eq!(glyphs(&shape(&font, &deva("\u{0915}\u{094D}\u{0937}")).expect("shapes")), vec![K_SSA]);
}

#[test]
fn vertical_text_takes_vertical_forms_and_advances_down() {
    let font = test_font();
    let req = ShapeRequest { direction: Direction::TopToBottom, ..ShapeRequest::new("((") };
    let out = shape(&font, &req).expect("shapes");
    assert_eq!(glyphs(&out), vec![PAREN_VERT, PAREN_VERT]);
    for glyph in &out {
        assert_eq!(glyph.x_advance, 0);
        assert!(glyph.y_advance < 0, "{glyph:?}");
    }
    // Without vhea and vmtx every glyph advances by the line: ascender minus descender.
    assert_eq!(out.first().map(|g| g.y_advance), Some(-1000));
}

#[test]
fn live_vertical_metrics_replace_the_font_defaults() {
    let font = test_font();
    let live = |gid: u16| (gid == PAREN_VERT).then_some(VerticalMetrics { advance: 800, origin_y: 700 });
    let req = ShapeRequest { direction: Direction::TopToBottom, vertical: Some(&live), ..ShapeRequest::new("((") };
    let out = shape(&font, &req).expect("shapes");
    for glyph in &out {
        assert_eq!(glyph.y_advance, -800, "{glyph:?}");
        // The glyph's origin lies below and left of the vertical origin the pen stands on.
        assert_eq!((glyph.x_offset, glyph.y_offset), (-width(PAREN_VERT) / 2, -700), "{glyph:?}");
    }
}

#[test]
fn a_glyph_typed_by_name_is_substituted_by_a_feature() {
    let font = test_font();
    let text = "\u{F0000}";
    assert_eq!(glyphs(&shape(&font, &ShapeRequest::new(text)).expect("shapes")), vec![G_ALT]);
    let on = [FeatureSetting::global(Tag::new(b"ss01"), true)];
    let out = shape(&font, &ShapeRequest { features: &on, ..ShapeRequest::new(text) }).expect("shapes");
    assert_eq!(glyphs(&out), vec![G_ALT_SS01]);
    assert_eq!(font.private_use_glyph(PUA_G_ALT_SS01), Some(G_ALT_SS01));
    // Without the private-use map the code point has no glyph.
    let bare = ShapingFont::new(Arc::from(font.bytes()), &[]).expect("reads");
    assert_eq!(glyphs(&shape(&bare, &ShapeRequest::new(text)).expect("shapes")), vec![0]);
}

#[test]
fn a_feature_range_covers_only_its_bytes() {
    let font = test_font();
    // "fifi": liga off for the second pair only (bytes 2..4).
    let ranged = [FeatureSetting { tag: Tag::new(b"liga"), value: 0, range: Some(2..4) }];
    let out = shape(&font, &ShapeRequest { features: &ranged, ..ShapeRequest::new("fifi") }).expect("shapes");
    assert_eq!(glyphs(&out), vec![F_I, F, I]);
}

#[test]
fn plans_are_cached_per_settings_and_capped() {
    let font = test_font();
    shape(&font, &ShapeRequest::new("fi")).expect("shapes");
    shape(&font, &ShapeRequest::new("if")).expect("shapes");
    assert_eq!(font.cached_plans(), 1);
    let off = [FeatureSetting::global(Tag::new(b"liga"), false)];
    shape(&font, &ShapeRequest { features: &off, ..ShapeRequest::new("fi") }).expect("shapes");
    assert_eq!(font.cached_plans(), 2);
    for value in 0..(MAX_PLANS as u32 + 8) {
        let settings = [FeatureSetting { tag: Tag::new(b"ss01"), value, range: None }];
        shape(&font, &ShapeRequest { features: &settings, ..ShapeRequest::new("f") }).expect("shapes");
    }
    assert_eq!(font.cached_plans(), MAX_PLANS);
}

#[test]
fn requests_over_the_limits_are_refused_before_shaping() {
    let font = test_font();
    let long = "f".repeat(MAX_TEXT_BYTES + 1);
    assert_eq!(shape(&font, &ShapeRequest::new(&long)), Err(ShapeError::TextTooLong { limit: MAX_TEXT_BYTES }));
    let many: Vec<FeatureSetting> = (0..=MAX_FEATURES).map(|_| FeatureSetting::global(Tag::new(b"liga"), true)).collect();
    assert_eq!(shape(&font, &ShapeRequest { features: &many, ..ShapeRequest::new("fi") }), Err(ShapeError::TooManyFeatures { limit: MAX_FEATURES }));
    let language = "x".repeat(MAX_LANGUAGE_BYTES + 1);
    assert!(matches!(shape(&font, &ShapeRequest { language: Some(&language), ..ShapeRequest::new("fi") }), Err(ShapeError::Failed(_))));
    assert_eq!(shape(&font, &ShapeRequest::new("")), Ok(Vec::new()));
}

#[test]
fn a_pathological_contextual_lookup_stops_at_max_ops_with_a_message() {
    // Lookup k (k < DEPTH) applies lookup k + 1 twice at the same glyph, so one `x` asks for
    // 2^DEPTH nested lookups, far past harfrust's operation budget for a short text (65,536),
    // while the nesting stays under its depth limit of 64.
    const DEPTH: u16 = 24;
    const X: u16 = 1;
    let mut lookups: Vec<Vec<u8>> = (0..DEPTH).map(|k| context_on(X, &[k + 1, k + 1])).collect();
    lookups.push(single(&[(X, X)]));
    let features: [(&[u8; 4], &[u16]); 1] = [(b"ccmp", &[0])];
    let bytes = font_bytes(2, &[500, 500], &[(0x78, X)], gsub(&features, &lookups));
    let font = ShapingFont::new(bytes, &[]).expect("reads");
    match shape(&font, &ShapeRequest::new("x")) {
        Err(ShapeError::Failed(message)) => assert!(!message.is_empty()),
        other => panic!("expected the shaper to stop, got {other:?}"),
    }
}

#[test]
fn a_location_on_a_static_font_changes_nothing() {
    let font = test_font();
    let moved = font.at_location(&[(Tag::new(b"wght"), 700.0), (Tag::new(b"wdth"), f32::NAN)]);
    assert_eq!(shape(&moved, &ShapeRequest::new("fi")), shape(&font, &ShapeRequest::new("fi")));
}
