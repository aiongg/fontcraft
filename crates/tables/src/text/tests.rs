//! Text views and the normalised dump on fonts built in code. The expected text is the output of
//! AFDKO 4 (`ttxn`, `tx -dump`) and fontTools 4.66.1 on fonts built with fontTools to the same
//! structure, glyph order and values.

use super::*;
use crate::sfnt::Flavour;
use write_fonts::dump_table;
use write_fonts::tables::gpos as wpos;
use write_fonts::tables::gsub as wsub;
use write_fonts::tables::layout as wl;
use write_fonts::tables::maxp::Maxp;
use write_fonts::tables::post::Post;
use write_fonts::types::{GlyphId16, Tag as WTag};

fn g(i: u16) -> GlyphId16 {
    GlyphId16::new(i)
}

fn cov(ids: &[u16]) -> wl::CoverageTable {
    wl::CoverageTable::format_1(ids.iter().map(|i| g(*i)).collect())
}

fn feature(tag: &[u8; 4], lookups: Vec<u16>) -> wl::FeatureRecord {
    wl::FeatureRecord::new(WTag::new(tag), wl::Feature::new(None, lookups))
}

fn layout_font(names: &[&str], tables: Vec<(Tag, Vec<u8>)>) -> SfntFont {
    let mut f = SfntFont::new(Flavour::TrueType);
    f.insert(*b"maxp", dump_table(&Maxp::new(names.len() as u16)).unwrap());
    f.insert(*b"post", dump_table(&Post::new_v2(names.iter().copied())).unwrap());
    for (t, d) in tables {
        f.insert(t, d);
    }
    f
}

/// kern: A V -50 (0), [T] [o] -80 by classes (1); mark: acute and grave on o (2); A +20 with
/// IgnoreMarks, used by no feature (3). DFLT and latn use kern and mark; latn TRK only lookup 1.
fn gpos_font() -> SfntFont {
    let flag = wl::LookupFlag::empty();
    let pairs = wpos::PositionLookup::Pair(wl::Lookup::new(
        flag,
        vec![wpos::PairPos::format_1(
            cov(&[1]),
            vec![wpos::PairSet::new(vec![wpos::PairValueRecord::new(g(2), wpos::ValueRecord::new().with_x_advance(-50), wpos::ValueRecord::new())])],
        )],
    ));
    let zero = || wpos::Class2Record::new(wpos::ValueRecord::new().with_x_advance(0), wpos::ValueRecord::new());
    let classes = wpos::PositionLookup::Pair(wl::Lookup::new(
        flag,
        vec![wpos::PairPos::format_2(
            cov(&[3]),
            wl::ClassDef::format_2(vec![wl::ClassRangeRecord::new(g(3), g(3), 1)]),
            wl::ClassDef::format_2(vec![wl::ClassRangeRecord::new(g(4), g(4), 1)]),
            vec![
                wpos::Class1Record::new(vec![zero(), zero()]),
                wpos::Class1Record::new(vec![
                    zero(),
                    wpos::Class2Record::new(wpos::ValueRecord::new().with_x_advance(-80), wpos::ValueRecord::new()),
                ]),
            ],
        )],
    ));
    let mark = wpos::PositionLookup::MarkToBase(wl::Lookup::new(
        flag,
        vec![wpos::MarkBasePosFormat1::new(
            cov(&[5, 6]),
            cov(&[4]),
            wpos::MarkArray::new(vec![
                wpos::MarkRecord::new(0, wpos::AnchorTable::format_1(250, 500)),
                wpos::MarkRecord::new(0, wpos::AnchorTable::format_1(250, 500)),
            ]),
            wpos::BaseArray::new(vec![wpos::BaseRecord::new(vec![Some(wpos::AnchorTable::format_1(300, 450))])]),
        )],
    ));
    let single = wpos::PositionLookup::Single(wl::Lookup::new(
        wl::LookupFlag::IGNORE_MARKS,
        vec![wpos::SinglePos::format_1(cov(&[1]), wpos::ValueRecord::new().with_x_advance(20))],
    ));
    let features = wl::FeatureList::new(vec![feature(b"kern", vec![0, 1]), feature(b"kern", vec![1]), feature(b"mark", vec![2])]);
    let scripts = wl::ScriptList::new(vec![
        wl::ScriptRecord::new(WTag::new(b"DFLT"), wl::Script::new(Some(wl::LangSys::new(vec![0, 2])), vec![])),
        wl::ScriptRecord::new(
            WTag::new(b"latn"),
            wl::Script::new(Some(wl::LangSys::new(vec![0, 2])), vec![wl::LangSysRecord::new(WTag::new(b"TRK "), wl::LangSys::new(vec![1]))]),
        ),
    ]);
    let gpos = wpos::Gpos::new(scripts, features, wl::LookupList::new(vec![pairs, classes, mark, single]));
    layout_font(&[".notdef", "A", "V", "T", "o", "acute", "grave"], vec![(*b"GPOS", dump_table(&gpos).unwrap())])
}

/// smcp (0), liga (1), salt (2), ccmp (3) and calt, a chaining context b a' c (4).
fn gsub_font() -> SfntFont {
    let flag = wl::LookupFlag::empty();
    let single = wsub::SubstitutionLookup::Single(wl::Lookup::new(flag, vec![wsub::SingleSubst::format_2(cov(&[1, 9]), vec![g(8), g(1)])]));
    let liga = wsub::SubstitutionLookup::Ligature(wl::Lookup::new(
        flag,
        vec![wsub::LigatureSubstFormat1::new(cov(&[4]), vec![wsub::LigatureSet::new(vec![wsub::Ligature::new(g(6), vec![g(5)])])])],
    ));
    let alt = wsub::SubstitutionLookup::Alternate(wl::Lookup::new(
        flag,
        vec![wsub::AlternateSubstFormat1::new(cov(&[1]), vec![wsub::AlternateSet::new(vec![g(7), g(8)])])],
    ));
    let multi = wsub::SubstitutionLookup::Multiple(wl::Lookup::new(
        flag,
        vec![wsub::MultipleSubstFormat1::new(cov(&[6]), vec![wsub::Sequence::new(vec![g(4), g(5)])])],
    ));
    let chain = wsub::SubstitutionLookup::ChainContextual(wl::Lookup::new(
        flag,
        vec![
            wl::ChainedSequenceContext::format_3(vec![cov(&[2])], vec![cov(&[1])], vec![cov(&[3])], vec![wl::SequenceLookupRecord::new(0, 0)]).into(),
        ],
    ));
    let features = wl::FeatureList::new(vec![
        feature(b"calt", vec![4]),
        feature(b"ccmp", vec![3]),
        feature(b"liga", vec![1]),
        feature(b"salt", vec![2]),
        feature(b"smcp", vec![0]),
    ]);
    let scripts =
        wl::ScriptList::new(vec![wl::ScriptRecord::new(WTag::new(b"DFLT"), wl::Script::new(Some(wl::LangSys::new(vec![0, 1, 2, 3, 4])), vec![]))]);
    let gsub = wsub::Gsub::new(scripts, features, wl::LookupList::new(vec![single, liga, alt, multi, chain]));
    layout_font(&[".notdef", "a", "b", "c", "f", "i", "f_i", "a.alt", "a.sc", "sub"], vec![(*b"GSUB", dump_table(&gsub).unwrap())])
}

fn otl(font: &SfntFont, tag: Tag) -> String {
    let mut out = Out::new();
    ttxn_otl::write(font, tag, false, &mut out).unwrap();
    out.text
}

#[test]
fn normalised_positioning_matches_the_oracle() {
    let expected = "    languagesystem latn dflt;
    languagesystem DFLT dflt;
    languagesystem latn TRK ;


    # Class definitions *********************************
    @empty = [  ];


    # Mark Class definitions *********************************
    mark [acute grave] <anchor 250 500> @acute_grave_mark;

    feature kern {
      lookup kern_DFLT_dflt_0 {
        pos A V -50;
      } kern_DFLT_dflt_0;

      lookup kern_DFLT_dflt_1 {
        pos [T] [o] -80;
      } kern_DFLT_dflt_1;

    } kern;

    feature mark {
      lookup mark_DFLT_dflt_0 {
        pos base [o] <anchor 300 450> mark @acute_grave_mark;
      } mark_DFLT_dflt_0;

    } mark;

";
    assert_eq!(otl(&gpos_font(), *b"GPOS"), expected);
}

#[test]
fn normalised_substitution_matches_the_oracle() {
    let expected = "    languagesystem DFLT dflt;



    feature calt {
      lookup calt_DFLT_dflt_0 {
        sub [b] [a]' [c];
      } calt_DFLT_dflt_0;

    } calt;

    feature ccmp {
      lookup ccmp_DFLT_dflt_0 {
        sub f_i by [f i];
      } ccmp_DFLT_dflt_0;

    } ccmp;

    feature liga {
      lookup liga_DFLT_dflt_0 {
        sub f i by f_i;
      } liga_DFLT_dflt_0;

    } liga;

    feature salt {
      lookup salt_DFLT_dflt_0 {
        sub a from [a.alt a.sc];
      } salt_DFLT_dflt_0;

    } salt;

    feature smcp {
      lookup smcp_DFLT_dflt_0 {
        sub a by a.sc;
        sub sub by a;
      } smcp_DFLT_dflt_0;

    } smcp;

";
    assert_eq!(otl(&gsub_font(), *b"GSUB"), expected);
}

#[test]
fn long_statements_wrap_as_python_textwrap_does() {
    let statement = "@c_28_q.ss01 = [ c cacute ccaron ccedilla cdotaccent d dcaron dcroat e eacute ecaron ecircumflex edieresis \
                     edotaccent egrave emacron eogonek o oacute ocircumflex odieresis ograve ohungarumlaut omacron oslash otilde q q.ss01 ];";
    assert_eq!(
        ttxn_otl::wrap(statement),
        [
            "@c_28_q.ss01 = [ c cacute ccaron ccedilla cdotaccent d dcaron dcroat e eacute",
            "    ecaron ecircumflex edieresis edotaccent egrave emacron eogonek o oacute",
            "    ocircumflex odieresis ograve ohungarumlaut omacron oslash otilde q q.ss01 ];",
        ]
    );
    assert_eq!(ttxn_otl::wrap("@empty = [  ];"), ["@empty = [  ];"]);
    let long = format!("{} y", "x".repeat(90));
    assert_eq!(ttxn_otl::wrap(&long), ["x".repeat(80), format!("    {} y", "x".repeat(10))]);
}

// ── a TrueType font for the abstract-font dump ─────────────────────────────────────────────────

#[derive(Default)]
struct Be(Vec<u8>);

impl Be {
    fn u16s(&mut self, v: &[u16]) -> &mut Self {
        for x in v {
            self.0.extend_from_slice(&x.to_be_bytes());
        }
        self
    }
    fn i16s(&mut self, v: &[i16]) -> &mut Self {
        for x in v {
            self.0.extend_from_slice(&x.to_be_bytes());
        }
        self
    }
    fn u32s(&mut self, v: &[u32]) -> &mut Self {
        for x in v {
            self.0.extend_from_slice(&x.to_be_bytes());
        }
        self
    }
}

/// A simple glyph from (x, y, on-curve) contours, every coordinate as a two-byte delta.
fn simple_glyph(contours: &[&[(i16, i16, bool)]]) -> Vec<u8> {
    let all: Vec<(i16, i16, bool)> = contours.iter().flat_map(|c| c.iter().copied()).collect();
    let (xs, ys): (Vec<i16>, Vec<i16>) = (all.iter().map(|p| p.0).collect(), all.iter().map(|p| p.1).collect());
    let mut g = Be::default();
    g.i16s(&[contours.len() as i16, *xs.iter().min().unwrap(), *ys.iter().min().unwrap(), *xs.iter().max().unwrap(), *ys.iter().max().unwrap()]);
    let mut end = 0u16;
    for c in contours {
        end += c.len() as u16;
        g.u16s(&[end - 1]);
    }
    g.u16s(&[0]);
    g.0.extend(all.iter().map(|p| u8::from(p.2)));
    let (mut px, mut py) = (0i16, 0i16);
    let mut dx = Vec::new();
    let mut dy = Vec::new();
    for p in &all {
        dx.push(p.0 - px);
        dy.push(p.1 - py);
        (px, py) = (p.0, p.1);
    }
    g.i16s(&dx);
    g.i16s(&dy);
    g.0
}

fn name_table(records: &[(u16, &str)]) -> Vec<u8> {
    let mut strings = Vec::new();
    let mut t = Be::default();
    t.u16s(&[0, records.len() as u16, 6 + 12 * records.len() as u16]);
    for (id, s) in records {
        let bytes: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        t.u16s(&[3, 1, 0x409, *id, bytes.len() as u16, strings.len() as u16]);
        strings.extend(bytes);
    }
    t.0.extend(strings);
    t.0
}

const O: &[(i16, i16, bool)] = &[
    (213, -8, true),
    (149, -8, false),
    (75, 44, false),
    (44, 140, false),
    (44, 205, true),
    (44, 373, true),
    (44, 438, false),
    (75, 534, false),
    (149, 586, false),
    (213, 586, true),
    (277, 586, false),
    (350, 534, false),
    (381, 438, false),
    (381, 373, true),
    (381, 205, true),
    (381, 140, false),
    (350, 44, false),
    (277, -8, false),
];
const BOX: &[(i16, i16, bool)] = &[(0, 0, false), (100, 0, true), (100, 100, true), (0, 100, true)];

/// `.notdef` (empty), `o`, `box` (a contour starting off the curve) and `space`; `o` is U+006F,
/// `space` U+0020 and U+00A0, and U+0041 maps to `.notdef`.
fn tt_font() -> SfntFont {
    let names = [".notdef", "o", "box", "space"];
    let mut f = SfntFont::new(Flavour::TrueType);
    let mut head = Be::default();
    head.u32s(&[0x0001_0000, 0x0001_4000, 0x1234_5678, 0x5F0F_3CF5]).u16s(&[3, 1000]).u32s(&[0, 0xABCD, 0, 0xABCD]);
    head.i16s(&[0, -8, 381, 586]).u16s(&[0, 3]).i16s(&[2, 1, 0]);
    f.insert(*b"head", head.0);
    let mut hhea = Be::default();
    hhea.u32s(&[0x0001_0000]).i16s(&[800, -200, 0]).u16s(&[500]).i16s(&[0, 0, 381, 1, 0, 0, 0, 0, 0, 0, 0]).u16s(&[4]);
    f.insert(*b"hhea", hhea.0);
    f.insert(*b"maxp", dump_table(&Maxp::new(4)).unwrap());
    let mut hmtx = Be::default();
    hmtx.u16s(&[500, 0, 425, 44, 100, 0, 200, 0]);
    f.insert(*b"hmtx", hmtx.0);
    let mut post = dump_table(&Post::new_v2(names.iter().copied())).unwrap();
    post[8..12].copy_from_slice(&[0xFF, 0x6A, 0x00, 0x3C]);
    f.insert(*b"post", post);
    f.insert(
        *b"name",
        name_table(&[(0, "Copyright Test"), (1, "Tx Test"), (2, "Regular"), (4, "Tx Test Regular"), (6, "TxTest-Regular"), (7, "Mark Test")]),
    );
    let mut os2 = vec![0u8; 78];
    os2[8..10].copy_from_slice(&4u16.to_be_bytes());
    f.insert(*b"OS/2", os2);
    // cmap: one format 4 subtable, segments 0x20, 0x41, 0x6F, 0xA0 and the final 0xFFFF.
    let ends = [0x20u16, 0x41, 0x6F, 0xA0, 0xFFFF];
    let gids = [3u16, 0, 1, 3, 0];
    let mut sub = Be::default();
    sub.u16s(&[4, 16 + 8 * 5, 0, 10, 8, 2, 2]).u16s(&ends).u16s(&[0]).u16s(&ends);
    sub.u16s(&ends.iter().zip(gids).map(|(c, gid)| if *c == 0xFFFF { 1 } else { gid.wrapping_sub(*c) }).collect::<Vec<_>>());
    sub.u16s(&[0; 5]);
    let mut cmap = Be::default();
    cmap.u16s(&[0, 1, 3, 1]).u32s(&[12]);
    cmap.0.extend(sub.0);
    f.insert(*b"cmap", cmap.0);
    let glyphs = [Vec::new(), simple_glyph(&[O]), simple_glyph(&[BOX]), Vec::new()];
    let (mut glyf, mut loca) = (Vec::new(), Be::default());
    for gl in &glyphs {
        loca.u32s(&[glyf.len() as u32]);
        glyf.extend_from_slice(gl);
    }
    loca.u32s(&[glyf.len() as u32]);
    f.insert(*b"loca", loca.0);
    f.insert(*b"glyf", glyf);
    f
}

#[test]
fn font_dict_of_a_truetype_font_matches_the_oracle() {
    let expected = r#"## Top Dict
version             "1.250"
Notice              "Mark Test"
Copyright           "Copyright Test"
FullName            "Tx Test Regular"
FamilyName          "Tx Test"
Weight              "Regular"
UnderlinePosition   -180
UnderlineThickness  60
FontBBox            {0,-8,381,586}
FSType              4
sup.srcFontType     TrueType
sup.nGlyphs         4
## FontDict[0]
FontName            "TxTest-Regular"
## Private
"#;
    let options = ViewOptions { view: FONT_VIEW.into(), ..Default::default() };
    assert_eq!(view(&tt_font(), &options).unwrap(), expected);
}

/// The oracle names the unencoded glyph `g2` where its `post` table says `box`; the rest is its
/// output as printed.
#[test]
fn glyph_records_of_truetype_outlines_match_the_oracle() {
    let font = tt_font();
    let at = |detail| view(&font, &ViewOptions { view: GLYPHS_VIEW.into(), glyph_detail: detail, ..Default::default() }).unwrap();
    assert_eq!(
        at(GlyphDetail::Short),
        "## glyph[tag] {name,encoding}\nglyph[0] {.notdef,-}\nglyph[1] {o,U+006F}\nglyph[2] {box,-}\nglyph[3] {space,U+0020}\n"
    );
    let medium = "## glyph[tag] {name,encoding,path}
glyph[0] {.notdef,-,
 500 width endchar}
glyph[1] {o,U+006F,
 425 width 213 -8 move 128 -8 80 28 60 92 curve 49 124 44 162 44 205 curve 44
 373 line 44 460 63 525 112 560 curve 137 577 170 586 213 586 curve 298 586
 345 550 366 486 curve 376 454 381 416 381 373 curve 381 205 line 381 118 362
 53 314 18 curve 289 1 256 -8 213 -8 curve endchar}
glyph[2] {box,-,
 100 width 0 100 move 0 33 33 0 100 0 curve 100 100 line endchar}
glyph[3] {space,U+0020,
 200 width endchar}
";
    assert_eq!(at(GlyphDetail::Medium), medium);
    let long = at(GlyphDetail::Long);
    assert!(
        long.contains(
            "glyph[1] {o,U+006F,
  425 width
  213 -8 move
  128 -8 80 28 60 92 curve
  49 124 44 162 44 205 curve
  44 373 line
  44 460 63 525 112 560 curve
  137 577 170 586 213 586 curve
  298 586 345 550 366 486 curve
  376 454 381 416 381 373 curve
  381 205 line
  381 118 362 53 314 18 curve
  289 1 256 -8 213 -8 curve
  endchar}
glyph[2] {box,-,
  100 width
  0 100 move
  0 33 33 0 100 0 curve
  100 100 line
  endchar}
"
        ),
        "{long}"
    );
    let chosen = view(&font, &ViewOptions { view: GLYPHS_VIEW.into(), glyphs: vec!["space".into(), "o".into()], ..Default::default() }).unwrap();
    assert_eq!(chosen, "## glyph[tag] {name,encoding}\nglyph[3] {space,U+0020}\nglyph[1] {o,U+006F}\n");
    let missing = view(&font, &ViewOptions { view: GLYPHS_VIEW.into(), glyphs: vec!["nope".into()], ..Default::default() });
    assert!(matches!(missing, Err(TablesError::BadOptions(_))));
}

#[test]
fn normalised_dump_sorts_tables_zeroes_what_always_differs_and_writes_outlines() {
    let font = tt_font();
    let text = view(&font, &ViewOptions { normalize: true, ..Default::default() }).unwrap();
    assert!(
        text.starts_with(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ttFont sfntVersion=\"\\x00\\x01\\x00\\x00\" ttLibVersion=\"4.66\">\n\n  <OS_2>"
        ),
        "{text}"
    );
    assert!(!text.contains("<GlyphOrder>"), "{text}");
    let order: Vec<usize> = ["<OS_2>", "<cmap>", "<glyf>", "<head>", "<hhea>", "<hmtx>", "<loca>", "<maxp>", "<name>", "<post>"]
        .iter()
        .map(|t| text.find(t).unwrap_or_else(|| panic!("{t} missing: {text}")))
        .collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{text}");
    assert!(text.ends_with("</ttFont>\n"));
    let outlines = "    <FontOutlines>
      .notdef
        500 width
        endchar
      box
        100 width
        0 100 move
        0 33 33 0 100 0 curve
        100 100 line
        endchar
      o
        425 width
        213 -8 move
        128 -8 80 28 60 92 curve
        49 124 44 162 44 205 curve
        44 373 line
        44 460 63 525 112 560 curve
        137 577 170 586 213 586 curve
        298 586 345 550 366 486 curve
        376 454 381 416 381 373 curve
        381 205 line
        381 118 362 53 314 18 curve
        289 1 256 -8 213 -8 curve
        endchar
      space
        200 width
        endchar
    </FontOutlines>
  </glyf>
";
    assert!(text.contains(outlines), "{text}");
    assert!(text.contains("  <glyf>\n    <FontTopDict>\n      ## Top Dict\n      version             \"1.250\"\n"), "{text}");
    for line in
        ["<checkSumAdjustment value=\"0x0\"/>", "<magicNumber value=\"0x0\"/>", "<numberOfHMetrics value=\"0\"/>", "<fontRevision value=\"1.25\"/>"]
    {
        assert!(text.contains(line), "{line}: {text}");
    }
    assert!(!text.lines().any(|l| l.contains("<map code=") && l.contains(r#"name=".notdef""#)), "mappings to .notdef are left out: {text}");
    assert!(text.contains(r#"<map code="0x6f" name="o"/>"#), "{text}");
    let hmtx = text.find("<hmtx>").unwrap();
    assert!(text[hmtx..].contains(r#"<mtx name="o" width="425" lsb="44"/>"#), "hmtx is read before hhea is zeroed: {text}");

    let quiet = view(&font, &ViewOptions { normalize: true, no_version: true, ..Default::default() }).unwrap();
    assert!(quiet.contains("<fontRevision value=\"0.0\"/>"), "{quiet}");

    let one = view(&font, &ViewOptions { normalize: true, tables: vec![*b"hhea"], ..Default::default() }).unwrap();
    assert!(one.contains("  <hhea>") && !one.contains("<head>") && !one.contains("<glyf>"), "{one}");
}

#[test]
fn ttxn_rewrites_of_name_gdef_and_cmap_lines() {
    let lines = |s: &str| s.lines().map(str::to_string).collect::<Vec<_>>();
    let name = lines(
        r#"    <namerecord nameID="3" platformID="3" platEncID="1" langID="0x409">
      1.250;TEST;TxTest-Regular
    </namerecord>
    <namerecord nameID="5" platformID="3" platEncID="1" langID="0x409">
      Version 1.250 &amp; more; test
    </namerecord>
    <namerecord nameID="5" platformID="1" platEncID="0" langID="0x0">
      Version 1.250; test
    </namerecord>
    <namerecord nameID="6" platformID="3" platEncID="1" langID="0x409">
      TxTest-Regular
    </namerecord>"#,
    );
    assert_eq!(
        ttxn::name_lines(name).join("\n"),
        r#"    <namerecord nameID="3" platformID="3" platEncID="1" langID="0x409">
      VERSION SUPPRESSED
    </namerecord>
    <namerecord nameID="5" platformID="3" platEncID="1" langID="0x409">
      Version 1.250 &amp; more
    </namerecord>
    <namerecord nameID="5" platformID="1" platEncID="0" langID="0x0">
      VERSION SUPPRESSED
    </namerecord>
    <namerecord nameID="6" platformID="3" platEncID="1" langID="0x409">
      TxTest-Regular
    </namerecord>"#
    );

    let gdef = lines(
        r#"  <GDEF>
    <Version value="0x00010000"/>
    <GlyphClassDef>
      <ClassDef glyph="A" class="1"/>
    </GlyphClassDef>
    <AttachList>
      <Coverage>
      </Coverage>
    </AttachList>
    <LigCaretList>
      <Coverage>
        <Glyph value="f_i"/>
      </Coverage>
    </LigCaretList>
    <MarkAttachClassDef/>
  </GDEF>"#,
    );
    let out = ttxn::gdef_lines(gdef).join("\n");
    assert!(out.contains("    <GlyphClassDef Format=\"0\">\n") && out.contains("    <MarkAttachClassDef Format=\"0\"/>"), "{out}");
    assert!(!out.contains("AttachList>") && out.contains("<LigCaretList>"), "{out}");

    let cmap = lines(
        r#"    <cmap_format_12 platformID="3" platEncID="10" format="12" reserved="0" length="40" language="0" nGroups="2">
      <map code="0x0" name=".notdef"/>
      <map code="0x41" name="A"/>
    </cmap_format_12>
    <cmap_format_13 platformID="0" platEncID="6" format="13" reserved="0" length="28" language="0" nGroups="1">"#,
    );
    assert_eq!(
        ttxn::cmap_lines(cmap).join("\n"),
        r#"    <cmap_format_12 platformID="3" platEncID="10" format="12" reserved="0" length="0" language="0" nGroups="0">
      <map code="0x41" name="A"/>
    </cmap_format_12>
    <cmap_format_13 platformID="0" platEncID="6" format="13" reserved="0" length="0" language="0" nGroups="1">"#
    );
}

/// The stems and masks of the oracle's glyph `B` of a corpus font (charstring desubroutinised
/// with fontTools), and its ghost stems; the expected tokens are `tx -dump -6`'s.
#[test]
fn cff_hints_print_as_the_oracle_prints_them() {
    use super::txdump::{Op, render};
    let ops = vec![
        Op::HStem(0.0, 70.0),
        Op::HStem(332.0, 402.0),
        Op::HStem(630.0, 700.0),
        Op::VStem(90.0, 180.0),
        Op::VStem(453.0, 540.0),
        Op::VStem(493.0, 581.0),
        Op::Mask(vec![0xF4]),
        Op::Move(90.0, 0.0),
        Op::Line(335.0, 0.0),
        Op::Curve([499.0, 0.0, 581.0, 69.0, 581.0, 208.0]),
        Op::Line(438.0, 377.0),
        Op::Mask(vec![0xF8]),
        Op::Line(438.0, 380.5),
    ];
    assert_eq!(
        render(&ops, 621.0, true),
        [
            "621 width",
            "0 70 hstem",
            "332 402 hstem",
            "630 700 hstem",
            "90 180 vstem",
            "493 581 vstem",
            "90 0 move",
            "335 0 line",
            "499 0 581 69 581 208 curve",
            "438 377 line",
            "newhints",
            "0 70 hstem",
            "332 402 hstem",
            "630 700 hstem",
            "90 180 vstem",
            "453 540 vstem",
            "438 380.50 line",
            "endchar",
        ]
    );
    let ghosts = vec![Op::HStem(21.0, 0.0), Op::HStem(599.0, 579.0), Op::Move(0.0, 0.0)];
    assert_eq!(render(&ghosts, 650.0, true), ["650 width", "0 bottomedge", "599 topedge", "0 0 move", "endchar"]);
    assert_eq!(render(&ghosts, 650.0, false), ["650 width", "0 0 move", "endchar"]);
}

#[test]
fn views_route_to_their_writers_and_refuse_what_is_not_built() {
    let gsub = gsub_font();
    let fea = view(&gsub, &ViewOptions { view: "fea".into(), tables: vec![*b"GSUB"], ..Default::default() }).unwrap();
    assert!(fea.contains("lookup") && fea.contains("sub"), "{fea}");
    let wrong = view(&gsub, &ViewOptions { view: "normalized".into(), tables: vec![*b"GSUB"], ..Default::default() });
    assert!(matches!(wrong, Err(TablesError::BadOptions(_))), "{wrong:?}");
    let spot = view(&gsub, &ViewOptions { view: "raw".into(), tables: vec![*b"GSUB"], ..Default::default() });
    assert!(matches!(spot, Err(TablesError::NotBuilt(_))), "{spot:?}");
    let both = view(&gsub, &ViewOptions { view: "fea".into(), normalize: true, ..Default::default() });
    assert!(matches!(both, Err(TablesError::BadOptions(_))), "{both:?}");
    let kern = view(&gpos_font(), &ViewOptions { view: "normalized".into(), tables: vec![*b"GPOS"], ..Default::default() }).unwrap();
    assert!(kern.contains("A -50 V"), "{kern}");
    let normalized = view(&gsub, &ViewOptions { normalize: true, ..Default::default() }).unwrap();
    assert!(normalized.contains("  <GSUB>\n    languagesystem DFLT dflt;\n"), "{normalized}");
}

#[test]
fn damaged_tables_fail_with_a_message_or_print_what_they_can() {
    let gpos = gpos_font();
    let data = gpos.get(*b"GPOS").unwrap().to_vec();
    for len in 0..data.len() {
        let mut f = gpos.clone();
        f.insert(*b"GPOS", data[..len].to_vec());
        let _ = view(&f, &ViewOptions { normalize: true, ..Default::default() });
    }
    let tt = tt_font();
    for tag in [*b"glyf", *b"loca", *b"head", *b"name", *b"post", *b"cmap", *b"hmtx"] {
        let data = tt.get(tag).unwrap().to_vec();
        for len in 0..data.len() {
            let mut f = tt.clone();
            f.insert(tag, data[..len].to_vec());
            let _ = view(&f, &ViewOptions { normalize: true, ..Default::default() });
            let _ = view(&f, &ViewOptions { view: GLYPHS_VIEW.into(), glyph_detail: GlyphDetail::Long, ..Default::default() });
            let _ = view(&f, &ViewOptions { view: FONT_VIEW.into(), ..Default::default() });
        }
    }
    let mut cff = SfntFont::new(Flavour::Cff);
    cff.insert(*b"maxp", dump_table(&Maxp::new(2)).unwrap());
    for junk in [vec![], vec![1, 0, 4, 1], vec![1, 0, 4, 1, 0, 1, 1, 1, 0xFF, 0, 1, 2, 3, 4, 5, 6, 7, 8]] {
        cff.insert(*b"CFF ", junk);
        assert!(view(&cff, &ViewOptions { view: FONT_VIEW.into(), ..Default::default() }).is_err());
        let _ = view(&cff, &ViewOptions { view: GLYPHS_VIEW.into(), glyph_detail: GlyphDetail::Long, ..Default::default() });
    }
}

#[test]
fn text_past_its_limit_stops_with_an_error() {
    let mut out = Out::with_limit(8);
    assert!(out.line("1234567").is_ok());
    assert!(matches!(out.line("x"), Err(TablesError::TooLarge { .. })));
}
