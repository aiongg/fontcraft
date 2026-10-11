//! Tests of the audit reports. Fonts are assembled table by table in code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;

use super::*;

fn be16(v: u16) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

fn sfnt(tables: BTreeMap<[u8; 4], Vec<u8>>) -> Vec<u8> {
    let n = tables.len() as u16;
    let mut out = vec![0, 1, 0, 0];
    out.extend(be16(n));
    out.extend([0u8; 6]);
    let mut offset = 12 + 16 * tables.len();
    let mut data = Vec::new();
    for (tag, bytes) in &tables {
        out.extend(tag);
        out.extend([0u8; 4]);
        out.extend((offset as u32).to_be_bytes());
        out.extend((bytes.len() as u32).to_be_bytes());
        let mut padded = bytes.clone();
        while padded.len() % 4 != 0 {
            padded.push(0);
        }
        offset += padded.len();
        data.extend(padded);
    }
    out.extend(data);
    out
}

struct Face<'a> {
    names: &'a [(u16, &'a str)],
    weight: u16,
    mac_style: u16,
    panose: [u8; 10],
    gsub: Option<Vec<u8>>,
}

fn head(mac_style: u16) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend(be16(1));
    v.extend(be16(0));
    v.extend(0x0002_0000u32.to_be_bytes()); // fontRevision 2.0
    v.extend([0u8; 4]);
    v.extend(0x5F0F_3CF5u32.to_be_bytes());
    v.extend(be16(0)); // flags
    v.extend(be16(1000));
    v.extend([0u8; 16]); // created, modified
    for n in [-50i16, -200, 900, 800] {
        v.extend(n.to_be_bytes());
    }
    v.extend(be16(mac_style));
    v.extend(be16(3)); // lowestRecPPEM
    v.extend(2i16.to_be_bytes());
    v.extend(0i16.to_be_bytes());
    v.extend(0i16.to_be_bytes());
    assert_eq!(v.len(), 54);
    v
}

fn os2(weight: u16, panose: [u8; 10]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend(be16(4));
    v.extend(be16(500)); // xAvgCharWidth
    v.extend(be16(weight));
    v.extend(be16(5)); // usWidthClass
    v.extend(be16(0)); // fsType
    v.extend([0u8; 22]); // sub- and superscript, strikeout, family class
    v.extend(panose);
    v.extend([0u8; 16]); // Unicode ranges
    v.extend(*b"TEST");
    v.extend(be16(0x40)); // fsSelection
    v.extend(be16(0x20));
    v.extend(be16(0x7A));
    for n in [800i16, -200, 0] {
        v.extend(n.to_be_bytes());
    }
    v.extend(be16(900));
    v.extend(be16(200));
    v.extend([0u8; 8]); // code pages
    v.extend(500i16.to_be_bytes()); // sxHeight
    v.extend(700i16.to_be_bytes()); // sCapHeight
    v.extend([0u8; 6]);
    assert_eq!(v.len(), 96);
    v
}

fn post() -> Vec<u8> {
    let mut v = 0x0003_0000u32.to_be_bytes().to_vec();
    v.extend(0u32.to_be_bytes());
    v.extend((-100i16).to_be_bytes());
    v.extend(50i16.to_be_bytes());
    v.extend([0u8; 20]);
    v
}

fn name(records: &[(u16, &str)]) -> Vec<u8> {
    let mut strings = Vec::new();
    let mut recs = Vec::new();
    for &(id, text) in records {
        let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for v in [3u16, 1, 0x409, id, bytes.len() as u16, strings.len() as u16] {
            recs.extend(be16(v));
        }
        strings.extend(bytes);
    }
    let mut v = be16(0);
    v.extend(be16(records.len() as u16));
    v.extend(be16(6 + 12 * records.len() as u16));
    v.extend(recs);
    v.extend(strings);
    v
}

fn font(f: &Face<'_>) -> Vec<u8> {
    let mut t = BTreeMap::new();
    t.insert(*b"head", head(f.mac_style));
    t.insert(*b"maxp", [0x00, 0x00, 0x50, 0x00, 0x00, 0x05].to_vec());
    t.insert(*b"OS/2", os2(f.weight, f.panose));
    t.insert(*b"post", post());
    t.insert(*b"name", name(f.names));
    if let Some(g) = &f.gsub {
        t.insert(*b"GSUB", g.clone());
    }
    sfnt(t)
}

fn face(path: &str, f: &Face<'_>, layout: bool) -> FaceInfo {
    FaceInfo::read(path, &font(f), 0, layout).unwrap()
}

const LATIN_SANS: [u8; 10] = [2, 11, 5, 3, 0, 0, 0, 0, 0, 0];

fn family() -> Vec<FaceInfo> {
    let light = Face {
        names: &[(1, "Fam Light"), (2, "Regular"), (4, "Fam Light"), (6, "Fam-Light"), (16, "Fam"), (17, "Light")],
        weight: 300,
        mac_style: 0,
        panose: LATIN_SANS,
        gsub: None,
    };
    let bold = Face {
        names: &[(1, "Fam"), (2, "Bold"), (4, "Fam Bold"), (6, "Fam-Bold")],
        weight: 700,
        mac_style: 1,
        panose: [2, 11, 8, 3, 0, 0, 0, 0, 0, 0],
        gsub: None,
    };
    let regular = Face {
        names: &[(1, "Fam"), (2, "Regular"), (4, "Fam Regular"), (6, "Fam-Regular")],
        weight: 400,
        mac_style: 0,
        panose: LATIN_SANS,
        gsub: None,
    };
    vec![face("Light.otf", &light, false), face("Bold.otf", &bold, false), face("Regular.otf", &regular, false)]
}

#[test]
fn faces_group_by_typographic_family_in_the_comparison_tools_order() {
    let faces = family();
    let menus = menu_name_report(&faces);
    assert_eq!(menus.len(), 1, "the Light face joins Fam through its typographic family");
    assert_eq!(menus[0].family, "Fam");
    let rows: Vec<(&str, &str, &str)> =
        menus[0].fonts.iter().map(|r| (r.post_script_name.as_str(), r.preferred_menu.as_str(), r.windows_compatible_menu.as_str())).collect();
    // Windows family name first, then italic and bold: Fam Regular, Fam Bold, Fam Light.
    assert_eq!(
        rows,
        [("Fam-Regular", "Fam/Regular", "Fam/Regular"), ("Fam-Bold", "Fam/Bold", "Fam/Bold"), ("Fam-Light", "Fam/Light", "Fam Light/Regular")]
    );
    assert_eq!(menus[0].fonts[2].mac_compatible_menu, "Fam Light", "without ID 18 the full name stands in");
}

#[test]
fn metrics_collapse_when_every_face_agrees_and_sort_when_they_differ() {
    let faces = family();
    let report = metrics_report(&faces);
    let fam = &report[0];
    let field = |name: &str| fam.values.iter().find(|v| v.field == name).unwrap();

    let glyphs = field("numGlyphs");
    assert!(glyphs.same);
    assert_eq!(glyphs.value, Some(ReportValue::Int(5)));
    assert!(glyphs.fonts.is_empty());
    assert_eq!(field("typoMetrics").value, Some(ReportValue::Ints(vec![800, -200, 0, 700, 500])));
    assert_eq!(field("fontRevision").value, Some(ReportValue::Number(2.0)));
    assert_eq!(field("fsType").label.as_deref(), Some("Installable embedding"));

    let weights = field("usWeightClass");
    assert!(!weights.same);
    let listed: Vec<(&str, Option<&str>)> = weights.fonts.iter().map(|c| (c.post_script_name.as_str(), c.label.as_deref())).collect();
    assert_eq!(listed, [("Fam-Bold", Some("Bold")), ("Fam-Regular", Some("Normal (Regular)")), ("Fam-Light", Some("Light"))]);

    let bold: Vec<&str> = field("bold").fonts.iter().map(|c| c.post_script_name.as_str()).collect();
    assert_eq!(bold, ["Fam-Bold", "Fam-Regular", "Fam-Light"], "bold faces first, the rest in family order");

    let bbox = field("fontBBox");
    assert!(bbox.same && bbox.fonts.len() == 3, "the bounding boxes are always listed");
    assert_eq!(fam.first_face.post_script_name, "Fam-Regular");
}

#[test]
fn panose_names_each_digit_and_marks_what_differs_from_the_regular_face() {
    let faces = family();
    let report = panose_report(&faces);
    let fam = &report[0];
    assert_eq!(fam.base, "Fam-Regular");
    assert_eq!(fam.fonts[0].post_script_name, "Fam-Regular", "the base face comes first");
    assert_eq!(fam.fonts[0].kind, "Latin Text");
    let weight = &fam.fonts[0].fields[2];
    assert_eq!((weight.name, weight.value, weight.label, weight.differs), ("Weight", 5, Some("Book"), false));
    let bold = fam.fonts.iter().find(|f| f.post_script_name == "Fam-Bold").unwrap();
    let differing: Vec<&str> = bold.fields.iter().filter(|f| f.differs).map(|f| f.name).collect();
    assert_eq!(differing, ["Weight"]);
    assert_eq!(bold.fields[2].label, Some("Bold"));
    assert_eq!(weight_class_label(450), "Normal (Regular) to Medium");
    assert_eq!(width_class_label(0), "value out of range");
}

/// GSUB with `liga` (lookup 0) on latn/dflt and, when `turkish`, latn/TRK with `liga` and
/// `locl` (lookup 1).
fn gsub(turkish: bool) -> Vec<u8> {
    let langsys = |indices: &[u16]| {
        let mut v = be16(0);
        v.extend(be16(0xFFFF));
        v.extend(be16(indices.len() as u16));
        for &i in indices {
            v.extend(be16(i));
        }
        v
    };
    let dflt = langsys(&[0]);
    let trk = langsys(&[0, 1]);
    let n: u16 = u16::from(turkish);
    let header = 4 + 6 * n;
    let mut script = be16(header);
    script.extend(be16(n));
    if turkish {
        script.extend(*b"TRK ");
        script.extend(be16(header + dflt.len() as u16));
    }
    script.extend(&dflt);
    if turkish {
        script.extend(&trk);
    }
    let mut script_list = be16(1);
    script_list.extend(*b"latn");
    script_list.extend(be16(8));
    script_list.extend(script);
    let mut feature_list = be16(2);
    feature_list.extend(*b"liga");
    feature_list.extend(be16(14));
    feature_list.extend(*b"locl");
    feature_list.extend(be16(20));
    for lookup in [0u16, 1] {
        feature_list.extend(be16(0));
        feature_list.extend(be16(1));
        feature_list.extend(be16(lookup));
    }
    let mut v = be16(1);
    v.extend(be16(0));
    v.extend(be16(10));
    v.extend(be16(10 + script_list.len() as u16));
    v.extend(be16(10 + (script_list.len() + feature_list.len()) as u16));
    v.extend(script_list);
    v.extend(feature_list);
    v.extend(be16(0));
    v
}

#[test]
fn feature_report_groups_faces_whose_layout_agrees() {
    let tr = |ps: &'static str, file: &str| {
        let names = [(1, "Fam"), (2, "Regular"), (6, ps)];
        face(file, &Face { names: &names, weight: 400, mac_style: 0, panose: LATIN_SANS, gsub: Some(gsub(true)) }, true)
    };
    let plain = face(
        "C.otf",
        &Face { names: &[(1, "Fam"), (2, "Bold"), (6, "Fam-C")], weight: 700, mac_style: 1, panose: LATIN_SANS, gsub: Some(gsub(false)) },
        true,
    );
    let faces = vec![tr("Fam-B", "B.otf"), plain, tr("Fam-A", "A.otf")];
    let report = feature_report(&faces);
    assert!(!report.same);
    assert_eq!(report.groups.len(), 2);
    assert_eq!(report.groups[0].fonts, ["Fam-B", "Fam-A"], "groups sort by their first face; faces keep the family order");
    assert_eq!(report.groups[1].fonts, ["Fam-C"]);

    let gsub = report.groups[0].tables.iter().find(|t| t.table == "GSUB").unwrap();
    assert_eq!(gsub.scripts, [ScriptLanguages { script: "latn".into(), languages: vec!["TRK".into(), "dflt".into()] }]);
    assert_eq!(gsub.lookup_sets, [LookupSet { id: 1, lookups: vec![0] }, LookupSet { id: 2, lookups: vec![1] }]);
    let rows: Vec<(&str, bool, &[Option<usize>])> = gsub.features.iter().map(|f| (f.feature.as_str(), f.same, f.sets.as_slice())).collect();
    assert_eq!(rows, [("liga", true, &[Some(1), Some(1)][..]), ("locl", false, &[Some(2), None][..])]);
    let gpos = report.groups[0].tables.iter().find(|t| t.table == "GPOS").unwrap();
    assert!(gpos.scripts.is_empty() && gpos.error.is_none(), "a missing GPOS is empty, not an error");
}

fn sample_doc() -> ReportDoc {
    ReportDoc {
        tool_version: "0.12.0".into(),
        profile: "universal".into(),
        target_kind: "source".into(),
        target_path: Some("Test.fontcraft".into()),
        fonts: vec![ReportFont { id: "f0".into(), name: "Test".into() }],
        summary: ReportSummary { fail: 1, warn: 0, info: 0, passed: 3, skipped: 1, ..ReportSummary::default() },
        checks: vec![
            CheckRow { id: "check.outlines.openPath".into(), status: "fail".into(), results: 1, reason: None },
            CheckRow { id: "check.hinting.smartDropout".into(), status: "skipped".into(), results: 0, reason: Some("no instructions".into()) },
        ],
        results: vec![ResultRow {
            id: "r1".into(),
            check: "check.outlines.openPath".into(),
            code: "open".into(),
            severity: Severity::Fail,
            message: "Path 1 is open <b>".into(),
            font: None,
            glyph: "A".into(),
            master: None,
            layer: "m01".into(),
            table: None,
            field: None,
            fonts: Vec::new(),
            stale: false,
        }],
    }
}

#[test]
fn reports_render_one_line_per_result_and_group_by_check() {
    let doc = sample_doc();
    let text = render(&doc, ExportFormat::Text);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines, ["FAIL check.outlines.openPath [Test] A/m01: Path 1 is open <b>", "1 failed, 0 warnings, 0 notes; 3 checks passed, 1 skipped"]);

    let md = render(&doc, ExportFormat::Markdown);
    let open = crate::find("check.outlines.openPath").unwrap();
    assert!(md.contains(&format!("## {} (check.outlines.openPath)", open.label)), "{md}");
    assert!(md.contains("- **FAIL** A/m01: Path 1 is open \\<b\\>"), "{md}");
    assert!(md.contains("## Checks that did not run") && md.contains("no instructions"), "{md}");

    let page = render(&doc, ExportFormat::Html);
    assert!(page.starts_with("<!doctype html>"));
    assert!(page.contains("Path 1 is open &lt;b&gt;") && !page.contains("open <b>"), "messages are escaped");
    assert!(page.contains(&html(open.description)));
}

#[test]
fn validator_messages_known_to_be_wrong_are_recognised_only_where_they_are_wrong() {
    let cff = ValidatorFacts { cff: true, os2_version: Some(4), single_style: false, trailing_empty_glyphs: 3 };
    let ttf = ValidatorFacts { cff: false, ..cff };
    let id = |code: &str, text: &str, facts: &ValidatorFacts| known_validator_issue(code, text, facts).map(|k| k.id);
    assert_eq!(id("W0022", "Recommended table is missing gasp", &cff), Some("trueTypeTablesInCff"));
    assert_eq!(id("W0022", "Recommended table is missing gasp", &ttf), None);
    assert_eq!(
        id("W5300", "The FeatureRecord tag is valid, but unregistered FeatureList, FeatureRecord[3], tag = 'ss01'", &ttf),
        Some("registeredFeatureTag")
    );
    assert_eq!(id("W5300", "The FeatureRecord tag is valid, but unregistered FeatureList, FeatureRecord[3], tag = 'zz01'", &ttf), None);
    assert_eq!(id("E2132", "The version number is invalid 4", &ttf), Some("os2Version4"));
    assert_eq!(id("E1703", "Loca entry points outside the glyf range | Number of glyphs with the error = 3", &ttf), Some("trailingEmptyGlyphsLoca"));
    assert_eq!(id("E1703", "Loca entry points outside the glyf range | Number of glyphs with the error = 2", &ttf), None);
    assert_eq!(id("E2308", "The italicAngle is nonzero, but the head.macStyle italic bit is not set", &ttf), None);
    assert_eq!(id("E2308", "", &ValidatorFacts { single_style: true, ..ttf }), Some("italicAngleWithoutItalicBit"));
    assert_eq!(id("W0307", "Characters are mapped in the Unicode Private Use Area", &cff), None);
}
