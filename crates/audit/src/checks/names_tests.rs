//! Behaviour tests of the `check.names.*` checks on fonts assembled byte by byte.

use crate::{BinaryCtx, CheckGroup, Config, NoServices, Severity, audit_binary, find, select};

type Rec = (u16, u16, u16, u16, String);

fn be16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn be32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn name_table(recs: &[Rec]) -> Vec<u8> {
    let mut storage = Vec::new();
    let mut out = Vec::new();
    be16(&mut out, 0);
    be16(&mut out, recs.len() as u16);
    be16(&mut out, (6 + 12 * recs.len()) as u16);
    for (p, e, l, id, text) in recs {
        let bytes: Vec<u8> = if *p == 1 { text.bytes().collect() } else { text.encode_utf16().flat_map(u16::to_be_bytes).collect() };
        for v in [*p, *e, *l, *id, bytes.len() as u16, storage.len() as u16] {
            be16(&mut out, v);
        }
        storage.extend(bytes);
    }
    out.extend(storage);
    out
}

fn head(revision: f64, mac_style: u16) -> Vec<u8> {
    let mut out = Vec::new();
    be16(&mut out, 1);
    be16(&mut out, 0);
    be32(&mut out, (revision * 65536.0).round() as u32);
    be32(&mut out, 0);
    be32(&mut out, 0x5F0F_3CF5);
    be16(&mut out, 0);
    be16(&mut out, 1000);
    out.extend([0u8; 16]);
    out.extend([0u8; 8]);
    be16(&mut out, mac_style);
    out.extend([0u8; 8]);
    assert_eq!(out.len(), 54);
    out
}

fn os2(fs_selection: u16) -> Vec<u8> {
    let mut out = vec![0u8; 96];
    out[1] = 4;
    out[58..62].copy_from_slice(b"TEST");
    out[62..64].copy_from_slice(&fs_selection.to_be_bytes());
    out
}

fn fixed(v: i32) -> u32 {
    (v << 16) as u32
}

/// One `wght` axis and one instance whose subfamily name is ID 256.
fn fvar() -> Vec<u8> {
    let mut out = Vec::new();
    for v in [1, 0, 16, 2, 1, 20, 1, 8] {
        be16(&mut out, v);
    }
    out.extend(b"wght");
    for v in [100, 400, 900] {
        be32(&mut out, fixed(v));
    }
    be16(&mut out, 0);
    be16(&mut out, 257);
    be16(&mut out, 256);
    be16(&mut out, 0);
    be32(&mut out, fixed(600));
    out
}

/// One `wdth` axis with the values ID 258 (non-elidable) and ID 259 (elidable).
fn stat() -> Vec<u8> {
    let mut out = Vec::new();
    for v in [1, 1, 8, 1] {
        be16(&mut out, v);
    }
    be32(&mut out, 20);
    be16(&mut out, 2);
    be32(&mut out, 28);
    be16(&mut out, 2);
    out.extend(b"wdth");
    be16(&mut out, 257);
    be16(&mut out, 0);
    be16(&mut out, 4);
    be16(&mut out, 16);
    for (flags, name, value) in [(0, 258, 75), (2, 259, 100)] {
        for v in [1, 0, flags, name] {
            be16(&mut out, v);
        }
        be32(&mut out, fixed(value));
    }
    out
}

/// A CFF table with one font name and a Top DICT whose FullName is the first custom string.
fn cff(font_name: &str, full_name: &str) -> Vec<u8> {
    let index = |out: &mut Vec<u8>, data: &[u8]| {
        be16(out, 1);
        out.push(1);
        out.push(1);
        out.push(data.len() as u8 + 1);
        out.extend_from_slice(data);
    };
    let mut out = vec![1, 0, 4, 1];
    index(&mut out, font_name.as_bytes());
    index(&mut out, &[248, 27, 2]);
    index(&mut out, full_name.as_bytes());
    be16(&mut out, 0);
    out
}

fn sfnt(mut tables: Vec<(&[u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by_key(|(tag, _)| **tag);
    let version: u32 = if tables.iter().any(|(t, _)| *t == b"CFF ") { 0x4F54_544F } else { 0x0001_0000 };
    let mut out = Vec::new();
    be32(&mut out, version);
    be16(&mut out, tables.len() as u16);
    out.extend([0u8; 6]);
    let mut offset = 12 + 16 * tables.len();
    let mut data = Vec::new();
    for (tag, bytes) in &tables {
        out.extend_from_slice(*tag);
        be32(&mut out, 0);
        be32(&mut out, offset as u32);
        be32(&mut out, bytes.len() as u32);
        data.extend_from_slice(bytes);
        while data.len() % 4 != 0 {
            data.push(0);
        }
        offset = 12 + 16 * tables.len() + data.len();
    }
    out.extend(data);
    out
}

fn win(id: u16, text: &str) -> Rec {
    (3, 1, 0x409, id, text.to_string())
}
fn mac(id: u16, text: &str) -> Rec {
    (1, 0, 0, id, text.to_string())
}

/// The Windows English records of a clean regular font.
fn clean() -> Vec<Rec> {
    vec![
        win(0, "Copyright 2026 The Example Project Authors"),
        win(1, "Example Sans"),
        win(2, "Regular"),
        win(3, "1.001;TEST;ExampleSans-Regular"),
        win(4, "Example Sans Regular"),
        win(5, "Version 1.001"),
        win(6, "ExampleSans-Regular"),
        win(7, "Example is a trademark of the Example Project."),
        win(8, "Example Foundry"),
        win(9, "Drawn for the test suite."),
        win(11, "https://example.com"),
        win(14, "https://example.com/licence"),
    ]
}

/// The clean records with `changes` replacing records of the same key or added, minus the
/// Windows records of `removed`.
fn records(changes: Vec<Rec>, removed: &[u16]) -> Vec<Rec> {
    let mut recs: Vec<Rec> = clean().into_iter().filter(|r| !removed.contains(&r.3)).collect();
    for c in changes {
        match recs.iter_mut().find(|r| (r.0, r.1, r.2, r.3) == (c.0, c.1, c.2, c.3)) {
            Some(r) => *r = c,
            None => recs.push(c),
        }
    }
    recs
}

fn font(recs: &[Rec], revision: f64, fs_selection: u16) -> Vec<u8> {
    sfnt(vec![(b"name", name_table(recs)), (b"head", head(revision, 0)), (b"OS/2", os2(fs_selection))])
}

/// The codes and severities `check` reports on `bytes`.
fn run(check: &str, bytes: &[u8]) -> Vec<(&'static str, Severity)> {
    let services = NoServices;
    let ctx = BinaryCtx::new("f0", bytes, 0, &services).unwrap();
    let spec = find(check).unwrap_or_else(|| panic!("{check} is not registered"));
    let report = audit_binary(&ctx, &[spec], &Config::default());
    for f in &report.findings {
        assert!(spec.codes.contains(&f.code), "{check} emits unlisted `{}`", f.code);
        assert_eq!(f.location.font.as_deref(), Some("f0"));
    }
    report.findings.iter().map(|f| (f.code, f.severity)).collect()
}

fn codes(check: &str, bytes: &[u8]) -> Vec<&'static str> {
    run(check, bytes).into_iter().map(|(c, _)| c).collect()
}

fn with(changes: Vec<Rec>) -> Vec<u8> {
    font(&records(changes, &[]), 1.001, 0x0040)
}

#[test]
fn a_clean_font_passes_every_name_check() {
    let bytes = font(&clean(), 1.001, 0x0040);
    let checks = select(None, &["check.names.*"], &["check.names.macRecordsComplete"]);
    assert_eq!(checks.len(), 17);
    for spec in checks {
        assert_eq!(spec.group, CheckGroup::Names);
        assert!(codes(spec.id, &bytes).is_empty(), "{} fires on the clean font: {:?}", spec.id, run(spec.id, &bytes));
    }
}

#[test]
fn each_name_check_fires_on_its_fault() {
    let long = "A".repeat(64);
    let cases: Vec<(&str, Vec<u8>, Vec<&str>)> = vec![
        ("check.names.requiredRecords", font(&records(vec![], &[4]), 1.001, 0x40), vec!["missing"]),
        ("check.names.requiredRecords", font(&records(vec![win(6, " ")], &[7]), 1.001, 0x40), vec!["empty", "missing-recommended"]),
        ("check.names.emptyRecords", with(vec![win(3, "  ")]), vec!["empty-record"]),
        (
            "check.names.whitespace",
            with(vec![win(1, "Example  Sans"), win(3, "Unique "), win(8, " Example")]),
            vec!["double-space", "trailing-space", "leading-space"],
        ),
        ("check.names.lineBreaks", with(vec![win(10, "One line\nand another")]), vec!["line-break"]),
        ("check.names.postScriptNameCharacters", with(vec![win(6, "Example Sans"), win(20, "Ex(ample)")]), vec!["bad-psname", "bad-string"]),
        ("check.names.postScriptNameConsistency", with(vec![mac(6, "Other-Name")]), vec!["inconsistency"]),
        ("check.names.fullNameStartsWithFamily", with(vec![win(4, "Sample Sans Regular")]), vec!["mismatch-font-names"]),
        ("check.names.fullNameStartsWithFamily", font(&records(vec![], &[1]), 1.001, 0x40), vec!["missing-font-names"]),
        ("check.names.recommendedLengths", with(vec![win(4, &long)]), vec!["too-long"]),
        ("check.names.familyAndStyleLength", with(vec![win(1, "Example Sans Extra Condensed Display Bold")]), vec!["nameid1-too-long"]),
        ("check.names.familyAndStyleLength", with(vec![win(2, "Extra Condensed Semibold Oblique")]), vec!["long-name"]),
        ("check.names.menuNameLimits", with(vec![win(17, &long)]), vec!["too-long"]),
        ("check.names.versionString", font(&clean(), 2.0, 0x40), vec!["mismatch"]),
        ("check.names.versionString", with(vec![win(5, "Version 1.0011")]), vec!["near-mismatch", "version-decimals"]),
        ("check.names.versionString", with(vec![win(5, "Version one")]), vec!["parse", "version-format"]),
        (
            "check.names.versionString",
            font(&records(vec![win(5, "Version 1.000 Development")], &[]), 1.0, 0x40),
            vec!["development", "version-not-above-one"],
        ),
        ("check.names.versionString", font(&records(vec![], &[5]), 1.001, 0x40), vec!["missing"]),
        (
            "check.names.italicNames",
            font(&records(vec![win(1, "Example Sans Italic")], &[]), 1.001, 0x01),
            vec!["bad-familyname", "bad-subfamilyname"],
        ),
        ("check.names.italicNames", with(vec![win(2, "Italic"), win(17, "Light")]), vec!["bad-typographicsubfamilyname"]),
        ("check.names.subfamilyMatchesStyleBits", font(&clean(), 1.001, 0x20), vec!["mismatch"]),
        ("check.names.noMacRecords", with(vec![mac(1, "Example Sans")]), vec!["mac-names"]),
        ("check.names.copyrightInDescription", with(vec![win(10, "Copyright 2020 Someone")]), vec!["copyright-on-description"]),
        ("check.names.copyrightInDescription", with(vec![win(10, &"x".repeat(201))]), vec!["too-long"]),
        ("check.names.legalAndCredits", font(&records(vec![], &[9]), 1.001, 0x40), vec!["missing-designer-note"]),
        ("check.names.legalAndCredits", with(vec![win(0, "Copyright 2026, 123,456 Example")]), vec!["patent-number"]),
        ("check.names.familyNameStartsWithDigit", with(vec![win(1, "3D Sans"), win(4, "3D Sans Regular")]), vec!["begins-with-digit"]),
    ];
    for (check, bytes, expected) in cases {
        assert_eq!(codes(check, &bytes), expected, "{check}");
    }
}

#[test]
fn style_bits_and_severities_follow_the_rows() {
    let bold_italic = font(&records(vec![win(2, "Bold Italic"), win(4, "Example Sans Bold Italic")], &[]), 1.001, 0x21);
    assert!(codes("check.names.subfamilyMatchesStyleBits", &bold_italic).is_empty());
    assert!(codes("check.names.italicNames", &bold_italic).is_empty());
    assert_eq!(run("check.names.recommendedLengths", &with(vec![win(4, &"A".repeat(64))])), vec![("too-long", Severity::Info)]);
    assert_eq!(
        run("check.names.versionString", &with(vec![win(5, "Version 1.0011")])),
        vec![("near-mismatch", Severity::Warn), ("version-decimals", Severity::Info)]
    );
    let mac_codes = run("check.names.macRecordsComplete", &font(&clean(), 1.001, 0x40));
    assert_eq!(mac_codes.iter().filter(|(c, s)| *c == "missing-mac-record" && *s == Severity::Fail).count(), 4);
    assert_eq!(mac_codes.iter().filter(|(c, s)| *c == "missing-optional-mac-record" && *s == Severity::Warn).count(), 4);
    let mut complete = clean();
    for id in [0, 1, 2, 4, 5, 6, 7, 14] {
        complete.push(mac(id, "x"));
    }
    assert!(codes("check.names.macRecordsComplete", &font(&complete, 1.001, 0x40)).is_empty());
}

#[test]
fn cff_names_are_compared_with_the_name_table() {
    let recs = records(vec![win(6, "ExampleSans-Regular")], &[]);
    let tables = |cff_table: Vec<u8>| sfnt(vec![(b"name", name_table(&recs)), (b"head", head(1.001, 0)), (b"OS/2", os2(0x40)), (b"CFF ", cff_table)]);
    let same = tables(cff("ExampleSans-Regular", "Example Sans Regular"));
    assert!(codes("check.names.postScriptNameConsistency", &same).is_empty());
    assert!(codes("check.names.fullNameStartsWithFamily", &same).is_empty());
    let other = tables(cff("ExampleSans-Bold", "Example Sans Bold"));
    assert_eq!(codes("check.names.postScriptNameConsistency", &other), vec!["ps-cff-name-mismatch"]);
    assert_eq!(run("check.names.fullNameStartsWithFamily", &other), vec![("cff-full-name", Severity::Info)]);
}

#[test]
fn variable_font_names_are_measured_per_instance_and_stat_combination() {
    let family = "Example Sans Condensed Text";
    let recs = records(vec![win(1, family), win(4, "Example Sans Condensed Text Regular"), win(256, "Semibold"), win(257, "Weight")], &[]);
    let base = || vec![(b"name", name_table(&recs)), (b"head", head(1.001, 0)), (b"OS/2", os2(0x40)), (b"fvar", fvar())];
    assert_eq!(codes("check.names.familyAndStyleLength", &sfnt(base())), vec!["fvar-instance-too-long"]);

    let recs = records(
        vec![
            win(1, "Example Sans Display"),
            win(4, "Example Sans Display Regular"),
            win(257, "Width"),
            win(258, "Extra Condensed"),
            win(259, "Regular"),
        ],
        &[],
    );
    let mut tables = vec![(b"name", name_table(&recs)), (b"head", head(1.001, 0)), (b"OS/2", os2(0x40)), (b"fvar", fvar()), (b"STAT", stat())];
    assert_eq!(codes("check.names.familyAndStyleLength", &sfnt(tables.clone())), vec!["familyname-plus-stat-entries-too-long"]);
    let short = records(vec![win(1, "Example"), win(4, "Example Regular"), win(258, "Extra Condensed"), win(259, "Regular")], &[]);
    tables[0] = (b"name", name_table(&short));
    assert!(codes("check.names.familyAndStyleLength", &sfnt(tables)).is_empty());
}

#[test]
fn hostile_name_tables_are_bounded() {
    let services = NoServices;
    let names: Vec<_> = select(None, &["check.names.*"], &[]);

    // 65,535 records that all point at one 65,534-byte string: 4 GB of text if decoded naively.
    let mut table = Vec::new();
    be16(&mut table, 0);
    be16(&mut table, u16::MAX);
    be16(&mut table, (6 + 12 * usize::from(u16::MAX)) as u16);
    for i in 0..u16::MAX {
        for v in [3, 1, 0x409, i % 32, 65_534, 0] {
            be16(&mut table, v);
        }
    }
    table.extend(" A".repeat(32_767).bytes());
    let bytes = sfnt(vec![(b"name", table)]);
    let ctx = BinaryCtx::new("f0", &bytes, 0, &services).unwrap();
    let report = audit_binary(&ctx, &names, &Config::default());
    assert!(report.skipped.iter().any(|s| s.check == "check.names.emptyRecords"), "the text budget is reported");

    // A record count with no records behind it, and a font with no name table.
    for bytes in [sfnt(vec![(b"name", vec![0, 0, 0, 10, 0, 126])]), sfnt(vec![(b"head", head(1.0, 0))])] {
        let ctx = BinaryCtx::new("f0", &bytes, 0, &services).unwrap();
        let report = audit_binary(&ctx, &names, &Config::default());
        assert!(report.findings.is_empty(), "{:?}", report.findings);
        assert_eq!(report.skipped.len(), names.len());
    }
}
