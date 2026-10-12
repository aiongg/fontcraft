//! Sources written here from the format's public description; no file made by another program.

use fontcraft_model::plist::Value;
use fontcraft_model::{NodeKind, PropertyValue, Shape, Transform};

use super::*;

const MASTER: &str = "m01";

fn read(text: &str) -> Import {
    read_sfd(text.as_bytes()).unwrap_or_else(|e| panic!("{e}"))
}

fn kinds(import: &Import) -> Vec<&str> {
    import.skipped.iter().map(|s| s.what.as_str()).collect()
}

fn param(params: &fontcraft_model::Params, name: &str) -> Option<f64> {
    match params.get(name)? {
        Value::Number(n) => Some(n.value().as_f64()),
        _ => None,
    }
}

const BASIC: &str = r#"SplineFontDB: 3.0
FontName: Test-Regular
FullName: Test Regular
FamilyName: Test
Weight: Regular
Copyright: (c) Us
Version: 002.500
ItalicAngle: 0
UnderlinePosition: -100
UnderlineWidth: 50
Ascent: 800
Descent: 200
LayerCount: 2
Layer: 0 0 "Back" 1
Layer: 1 0 "Fore" 0
OS2TypoAscent: 0
OS2TypoAOffset: 1
OS2WinAscent: 900
OS2WinAOffset: 0
CreationTime: 1151539072
DisplaySize: -24
Encoding: UnicodeBmp
BeginChars: 65539 3

StartChar: A
Encoding: 65 65 0
Width: 600
Flags: W
HStem: 0 20<0 600>
LayerCount: 2
Fore
SplineSet
100 0 m 1
 100 700 l 1
 500 700 l 1
 500 0 l 1
 100 0 l 1
EndSplineSet
AnchorPoint: "top" 300 700 basechar 0
EndChar

StartChar: acute
Encoding: 180 180 1
Width: 0
Fore
SplineSet
-50 750 m 1
 50 850 l 1
EndSplineSet
AnchorPoint: "top" 0 700 mark 0
EndChar

StartChar: Aacute
Encoding: 193 193 2
Width: 600
Fore
Refer: 0 65 N 1 0 0 1 0 0 2
Refer: 1 180 S 1 0 0 1 300 100 2
EndChar
EndChars
EndSplineFont
"#;

#[test]
fn sfd_reads_outlines_references_anchors_and_metrics() {
    let import = read(BASIC);
    let font = &import.font;
    assert_eq!(font.family_name, "Test");
    assert_eq!(font.units_per_em, 1000);
    assert_eq!((font.version_major, font.version_minor), (2, 500));
    let master = font.default_master().unwrap();
    assert_eq!(master.name, "Regular");
    assert_eq!(param(&master.params, "underlinePosition"), Some(-100.0));
    assert_eq!(param(&master.params, "underlineThickness"), Some(50.0));
    assert_eq!(param(&master.params, "winAscent"), Some(900.0));
    assert_eq!(param(&master.params, "typoAscender"), None, "a value relative to a computed one is not kept");
    assert_eq!(font.info.properties.value("postscriptFontName"), Some("Test-Regular"));
    assert_eq!(font.info.properties.value("copyrights"), Some("(c) Us"));
    assert_eq!(font.info.date.as_deref(), Some("2006-06-28 23:57:52 +0000"));
    let names: Vec<&str> = font.glyphs.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["A", "acute", "Aacute"]);

    let a = font.glyph("A").unwrap();
    assert_eq!(a.unicodes, [65]);
    let layer = a.layer(MASTER).unwrap();
    assert_eq!(layer.width, 600.0);
    let Some(Shape::Path(p)) = layer.shapes.first() else { panic!("no path") };
    assert!(p.closed);
    let points: Vec<(f64, f64, NodeKind, bool)> = p.nodes.iter().map(|n| (n.x, n.y, n.kind, n.smooth)).collect();
    assert_eq!(
        points,
        [
            (100.0, 700.0, NodeKind::Line, false),
            (500.0, 700.0, NodeKind::Line, false),
            (500.0, 0.0, NodeKind::Line, false),
            (100.0, 0.0, NodeKind::Line, false)
        ],
        "a closed contour ends at its start, which is the last node"
    );
    assert_eq!(layer.anchors.first().map(|a| (a.name.as_str(), a.x, a.y)), Some(("top", 300.0, 700.0)));

    let acute = font.glyph("acute").unwrap().layer(MASTER).unwrap();
    let Some(Shape::Path(p)) = acute.shapes.first() else { panic!("no path") };
    assert!(!p.closed, "a contour that does not come back to its start is open");
    assert_eq!(p.nodes.len(), 2);
    assert_eq!(p.nodes.first().map(|n| n.kind), Some(NodeKind::Line));
    assert_eq!(acute.anchors.first().map(|a| a.name.as_str()), Some("_top"));

    let comps: Vec<(String, Transform)> = font
        .glyph("Aacute")
        .unwrap()
        .layer(MASTER)
        .unwrap()
        .shapes
        .iter()
        .filter_map(|s| if let Shape::Component(c) = s { Some((c.glyph.clone(), c.transform)) } else { None })
        .collect();
    assert_eq!(comps, [("A".to_string(), Transform::translate(0.0, 0.0)), ("acute".to_string(), Transform::translate(300.0, 100.0))]);
    assert_eq!(kinds(&import), ["PostScript stem hints"]);
}

fn one_glyph(quadratic: bool, contour: &str) -> String {
    let q = u8::from(quadratic);
    format!(
        "SplineFontDB: 3.0\nFamilyName: Q\nAscent: 800\nDescent: 200\nLayerCount: 2\nLayer: 0 {q} \"Back\" 1\nLayer: 1 {q} \"Fore\" 0\nBeginChars: 256 1\nStartChar: o\nEncoding: 111 111 0\nWidth: 600\nFore\nSplineSet\n{contour}EndSplineSet\nEndChar\nEndChars\nEndSplineFont\n"
    )
}

/// Two quadratic segments written in cubic form; the on-curve point between them is marked as
/// implied (0x80) and lies halfway between the two control points.
const QUAD_CONTOUR: &str = "0 0 m 1\n 0 200 100 300 300 300 c 128\n 500 300 600 200 600 0 c 1\n 0 0 l 1\n";

#[test]
fn sfd_quadratic_layer_reads_quadratic_segments() {
    let font = read(&one_glyph(true, QUAD_CONTOUR)).font;
    let layer = font.glyph("o").unwrap().layer(MASTER).unwrap().clone();
    let Some(Shape::Path(p)) = layer.shapes.first() else { panic!("no path") };
    let nodes: Vec<(f64, f64, NodeKind)> = p.nodes.iter().map(|n| (n.x, n.y, n.kind)).collect();
    assert_eq!(
        nodes,
        [(0.0, 300.0, NodeKind::OffCurve), (600.0, 300.0, NodeKind::OffCurve), (600.0, 0.0, NodeKind::QCurve), (0.0, 0.0, NodeKind::Line)]
    );
    p.to_bez().unwrap();

    let font = read(&one_glyph(false, QUAD_CONTOUR)).font;
    let layer = font.glyph("o").unwrap().layer(MASTER).unwrap().clone();
    let Some(Shape::Path(p)) = layer.shapes.first() else { panic!("no path") };
    let kinds: Vec<NodeKind> = p.nodes.iter().map(|n| n.kind).collect();
    use NodeKind::{Curve, Line, OffCurve};
    assert_eq!(kinds, [OffCurve, OffCurve, Curve, OffCurve, OffCurve, Curve, Line], "a cubic layer keeps cubic segments");
}

const KERNING: &str = r#"SplineFontDB: 3.0
FamilyName: K
Ascent: 800
Descent: 200
Lookup: 258 0 0 "'kern' Horizontal Kerning in Latin lookup 0" { "kern-1" [150,0,0] "kern-2" } ['kern' ('latn' <'dflt' > ) ]
Lookup: 258 0 0 "'dist' lookup 1" { "dist-1" } ['dist' ('latn' <'dflt' > ) ]
KernClass2: 2 3 "kern-2"
 1 T
 1 A
 3 O Q
 0 0 0
 0 -50 {12-13 -1,-1} -20
BeginChars: 256 5
StartChar: A
Encoding: 65 65 0
Width: 600
Kerns2: 1 -80 "kern-1" {12-13 -1,-1} 1 -10 "dist-1"
EndChar
StartChar: V
Encoding: 86 86 1
Width: 600
EndChar
StartChar: T
Encoding: 84 84 2
Width: 600
EndChar
StartChar: O
Encoding: 79 79 3
Width: 600
EndChar
StartChar: Q
Encoding: 81 81 4
Width: 600
EndChar
EndChars
"#;

#[test]
fn sfd_kerning_pairs_and_classes() {
    let import = read(KERNING);
    let font = &import.font;
    let k = &font.kerning.ltr;
    assert_eq!(k.get(MASTER, "A", "V"), Some(-80.0));
    assert_eq!(k.get(MASTER, "@MMK_L_T", "@MMK_R_A"), Some(-50.0));
    assert_eq!(k.get(MASTER, "@MMK_L_T", "@MMK_R_O"), Some(-20.0));
    assert_eq!(k.pair_count(), 3);
    assert_eq!(font.glyph("T").unwrap().kern_groups.right.as_deref(), Some("T"));
    assert_eq!(font.glyph("A").unwrap().kern_groups.left.as_deref(), Some("A"));
    assert_eq!(font.glyph("Q").unwrap().kern_groups.left.as_deref(), Some("O"));
    assert!(kinds(&import).contains(&"pair positioning outside the kerning feature"));
}

#[test]
fn sfd_names_are_localised() {
    let text = BASIC.replace(
        "Encoding: UnicodeBmp\n",
        "Encoding: UnicodeBmp\nLangName: 1033 \"\" \"\" \"Bold\" \"\" \"\" \"\" \"\" \"TM\"\nLangName: 1036 \"\" \"Famille\" \"\" \"\" \"\" \"\" \"\" \"\" \"\" \"Montr+AOk-al\"\nUComments: \"Line +AOk-\"\n",
    );
    let font = read(&text).font;
    assert_eq!(font.default_master().unwrap().name, "Bold");
    assert_eq!(font.info.properties.value("trademarks"), Some("TM"));
    let localized = |key: &str| match &font.info.properties.get(key).unwrap().value {
        PropertyValue::Localized(v) => v.iter().map(|l| (l.language.clone(), l.value.clone())).collect::<Vec<_>>(),
        PropertyValue::Single(s) => vec![("single".into(), s.clone())],
    };
    assert_eq!(localized("familyNames"), [("dflt".to_string(), "Test".to_string()), ("FRA".to_string(), "Famille".to_string())]);
    assert_eq!(localized("designers"), [("FRA".to_string(), "Montréal".to_string())]);
    assert_eq!(font.note, "Line é");
}

#[test]
fn sfd_utf7_and_dates() {
    assert_eq!(lines::utf7("Montr+AOk-al"), "Montréal");
    assert_eq!(lines::utf7("1+-1"), "1+1");
    assert_eq!(lines::unix_date(0).as_deref(), Some("1970-01-01 00:00:00 +0000"));
    assert_eq!(lines::unix_date(951_782_400).as_deref(), Some("2000-02-29 00:00:00 +0000"));
}

#[test]
fn sfd_skipped_data_is_reported() {
    let text = r#"SplineFontDB: 3.0
FamilyName: S
Ascent: 800
Descent: 200
BeginPrivate: 2
BlueValues 23 [-19 0 502 517 750 768]
BlueScale 5 0.039
EndPrivate
TtTable: prep
PUSHW_1
 640
NPUSHB
 2
 251
 254
EndTTInstrs
ShortTable: cvt  2
  309 "Big stem"
  184
EndShort
FooBar: 12
PickledData: "I3
."
BeginChars: 256 1
StartChar: a
Encoding: 97 97 0
Width: 500
TtInstrs:
NPUSHB
 1
 2
MDAP[rnd]
EndTTInstrs
TtfInstrs: 8
z!!!!!
EndTtf
Position2: "Inferiors" dx=0 dy=-900 dh=0 dv=0
Fore
SplineSet
0 0 m 1
 0 100 l 1
 100 100 l 1
 0 0 l 1
  Spiro
    0 0 v
    0 100 v
    100 100 v
    0 0 z
  EndSpiro
EndSplineSet
Image: 1 1 0 1 2 1 0 0 1 1
z
EndImage
EndChar
EndChars
BitmapFont: 12 285 10 2 1
BDFChar: 0 32 3 0 0 0 0
z
EndBitmapFont
EndSplineFont
"#;
    let import = read(text);
    let k = kinds(&import);
    for want in [
        "the PostScript private dictionary",
        "TrueType programs",
        "TrueType tables",
        "unknown keyword `FooBar`",
        "script data",
        "TrueType instructions",
        "glyph data of substitution and positioning lookups",
        "Spiro control points (the Bézier contours are kept)",
        "images",
        "bitmap strikes",
    ] {
        assert!(k.contains(&want), "{want} not in {k:?}");
    }
    let instructions = import.skipped.iter().find(|s| s.what == "TrueType instructions").unwrap();
    assert_eq!((instructions.count, instructions.first_at.as_str()), (2, "line 28"));
    let a = import.font.glyph("a").unwrap();
    let Some(Shape::Path(p)) = a.layer(MASTER).unwrap().shapes.first() else { panic!("the contour was lost") };
    assert_eq!(p.nodes.len(), 3);
}

#[test]
fn sfd_hostile_input_fails_with_a_message() {
    let cases: Vec<(String, &str)> = vec![
        ("FontName: x\n".into(), "not an SFD"),
        (BASIC.replace("EndChar\nEndChars", "EndChars"), "EndChar"),
        (BASIC.split("AnchorPoint: \"top\" 300").next().unwrap().to_string(), "the file ends inside glyph `A`"),
        (one_glyph(false, " 0 100 l 1\n"), "does not start with a move"),
        (one_glyph(false, "0 1e300 m 1\n"), "not a number within"),
        (one_glyph(false, "0 0 m 1\n 1 2 3 l 1\n"), "contour line"),
        (BASIC.replace("BeginChars", "KernClass2: 70000 70000 \"k\"\nBeginChars"), "beyond the limits"),
        ("SplineFontDB: 3.0\nKernClass2: 3 3 \"k\"\n 1 A\n".into(), "the file ends inside a class kerning table"),
        (BASIC.replace("Ascent: 800", "Ascent: 100000"), "em of"),
        (BASIC.replace("LayerCount: 2\nLayer: 0", "MMCounts: 4 2 0 0\nLayer: 0"), "multiple master"),
        (BASIC.replace("Refer: 1 180 S 1 0 0 1 300 100 2", "Refer: 1 180 S 1 0"), "reference"),
        (BASIC.replace("StartChar: acute", "StartChar: A"), "second glyph"),
        (BASIC.replace("BeginChars", "Layer: 9999 0 \"x\" 0\nBeginChars"), "beyond the limit"),
        (format!("SplineFontDB: 3.0\nComments: {}\n", "x".repeat(MAX_LINE_BYTES + 1)), "longer than"),
        (BASIC.replace("BeginChars", "BeginPrivate: 1\nBlueValues 1 []\nBeginChars"), "EndPrivate"),
    ];
    for (text, want) in cases {
        let err = read_sfd(text.as_bytes()).err().unwrap_or_else(|| panic!("accepted input that should fail with {want:?}")).to_string();
        assert!(err.contains(want), "{err:?} does not mention {want:?}");
    }
    assert!(matches!(read_sfd(&[0xFF, 0xFE]), Err(SfdError::NotSfd(_))));
}

#[test]
fn sfdir_reads_glyph_files_in_glyph_index_order() {
    let mut files = Files::new();
    files.insert("font.props".into(), b"SplineFontDB: 3.0\nFamilyName: Dir\nAscent: 750\nDescent: 250\n".to_vec());
    files.insert("a.glyph".into(), b"StartChar: a\nEncoding: 97 97 1\nWidth: 500\nEndChar\n".to_vec());
    files.insert("b.glyph".into(), b"StartChar: b\nEncoding: 98 98 0\nWidth: 520\nFore\nRefer: 1 97 N 1 0 0 1 10 0 2\nEndChar\n".to_vec());
    files.insert("12.strike/strike.props".into(), b"BitmapFont: 12 2 9 3 1\n".to_vec());
    let import = read_sfdir(&files).unwrap();
    let names: Vec<&str> = import.font.glyphs.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["b", "a"]);
    let b = import.font.glyph("b").unwrap().layer(MASTER).unwrap();
    assert!(matches!(b.shapes.first(), Some(Shape::Component(c)) if c.glyph == "a"));
    assert!(kinds(&import).contains(&"bitmap strikes"));
    assert!(is_sfd(b"SplineFontDB: 3.0\n"));
    assert!(read_sfdir(&Files::new()).is_err());
}
