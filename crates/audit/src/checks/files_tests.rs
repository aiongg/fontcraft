//! Behaviour tests of the `check.files.*` checks on fonts assembled byte by byte.

use crate::services::{AuditServices, ExternalMessage, ExternalTool, RoundTrip, Unavailable};
use crate::{BinaryCtx, Config, FamilyCtx, LayerReport, NoServices, Severity, audit_binary, audit_family, find};

const TRUETYPE: u32 = 0x0001_0000;
const OTTO: u32 = u32::from_be_bytes(*b"OTTO");

fn be16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn be32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// An sfnt with `tables` sorted by tag, each padded to four bytes, with correct search fields.
fn sfnt(version: u32, tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut sorted: Vec<(&[u8; 4], Vec<u8>)> = tables.to_vec();
    sorted.sort_by_key(|(t, _)| **t);
    let n = sorted.len() as u16;
    let selector = if n == 0 { 0 } else { 15 - n.leading_zeros() as u16 };
    let search = if n == 0 { 0 } else { 16u16 << selector };
    let mut out = Vec::new();
    be32(&mut out, version);
    for v in [n, search, selector, n * 16 - search] {
        be16(&mut out, v);
    }
    let mut offset = 12 + 16 * u32::from(n);
    let mut data = Vec::new();
    for (tag, bytes) in &sorted {
        out.extend_from_slice(*tag);
        be32(&mut out, 0);
        be32(&mut out, offset);
        be32(&mut out, bytes.len() as u32);
        data.extend_from_slice(bytes);
        while data.len() % 4 != 0 {
            data.push(0);
        }
        offset = 12 + 16 * u32::from(n) + data.len() as u32;
    }
    out.extend(data);
    out
}

fn head() -> Vec<u8> {
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    be32(&mut out, 0x0001_0000);
    be32(&mut out, 0);
    be32(&mut out, 0x5F0F_3CF5);
    be16(&mut out, 0);
    be16(&mut out, 1000);
    out.extend([0u8; 16]);
    for v in [0u16, 0, 600, 700] {
        be16(&mut out, v);
    }
    // macStyle, lowestRecPPEM, fontDirectionHint, indexToLocFormat (short), glyphDataFormat.
    for v in [0u16, 8, 2, 0, 0] {
        be16(&mut out, v);
    }
    assert_eq!(out.len(), 54);
    out
}

fn hhea(metrics: u16) -> Vec<u8> {
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    for v in [800u16, (-200i16) as u16, 0, 600, 0, 0, 600, 1, 0, 0, 0, 0, 0, 0, 0] {
        be16(&mut out, v);
    }
    be16(&mut out, metrics);
    assert_eq!(out.len(), 36);
    out
}

fn maxp(glyphs: u16) -> Vec<u8> {
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    be16(&mut out, glyphs);
    // maxPoints, maxContours, composite points and contours, zones, twilight points, storage,
    // function and instruction definitions, stack, instruction size, component elements, depth.
    for v in [16u16, 2, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0] {
        be16(&mut out, v);
    }
    assert_eq!(out.len(), 32);
    out
}

fn hmtx(glyphs: u16) -> Vec<u8> {
    let mut out = Vec::new();
    for _ in 0..glyphs {
        be16(&mut out, 600);
        be16(&mut out, 0);
    }
    out
}

/// A format 4 subtable with only the final segment.
fn cmap() -> Vec<u8> {
    let mut out = Vec::new();
    for v in [0u16, 1, 3, 1] {
        be16(&mut out, v);
    }
    be32(&mut out, 12);
    for v in [4u16, 24, 0, 2, 2, 0, 0, 0xFFFF, 0, 0xFFFF, 1, 0] {
        be16(&mut out, v);
    }
    out
}

/// Windows English records (platform 3, encoding 1, language 0x409).
fn name(records: &[(u16, &str)]) -> Vec<u8> {
    let mut storage = Vec::new();
    let mut out = Vec::new();
    be16(&mut out, 0);
    be16(&mut out, records.len() as u16);
    be16(&mut out, (6 + 12 * records.len()) as u16);
    for (id, text) in records {
        let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for v in [3, 1, 0x409, *id, bytes.len() as u16, storage.len() as u16] {
            be16(&mut out, v);
        }
        storage.extend(bytes);
    }
    out.extend(storage);
    out
}

fn os2() -> Vec<u8> {
    let mut out = vec![0u8; 96];
    out[1] = 4;
    out
}

fn post() -> Vec<u8> {
    let mut out = Vec::new();
    be32(&mut out, 0x0003_0000);
    out.extend([0u8; 28]);
    out
}

/// A triangle: one contour of three on-curve points.
fn triangle() -> Vec<u8> {
    let mut out = Vec::new();
    for v in [1u16, 0, 0, 600, 700, 2, 0] {
        be16(&mut out, v);
    }
    out.extend([1u8, 1, 1]);
    for v in [0i16, 600, -300, 0, 0, 700] {
        be16(&mut out, v as u16);
    }
    out
}

/// A glyph header that promises a contour, cut off before its instructions.
fn truncated_glyph() -> Vec<u8> {
    let mut out = Vec::new();
    for v in [1u16, 0, 0, 600, 700, 2] {
        be16(&mut out, v);
    }
    out
}

/// `glyf` and short `loca` for these glyphs, each padded to an even length.
fn glyf_loca(glyphs: &[Vec<u8>]) -> (Vec<u8>, Vec<u8>) {
    let (mut glyf, mut loca) = (Vec::new(), Vec::new());
    for g in glyphs {
        be16(&mut loca, (glyf.len() / 2) as u16);
        glyf.extend_from_slice(g);
        if glyf.len() % 2 != 0 {
            glyf.push(0);
        }
    }
    be16(&mut loca, (glyf.len() / 2) as u16);
    (glyf, loca)
}

/// A complete TrueType font: an empty `.notdef`, a triangle, and `extra` glyphs.
fn truetype(extra: &[Vec<u8>], names: &[(u16, &str)], more: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut glyphs = vec![Vec::new(), triangle()];
    glyphs.extend_from_slice(extra);
    let count = glyphs.len() as u16;
    let (glyf, loca) = glyf_loca(&glyphs);
    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"head", head()),
        (b"hhea", hhea(count)),
        (b"maxp", maxp(count)),
        (b"hmtx", hmtx(count)),
        (b"cmap", cmap()),
        (b"name", name(names)),
        (b"OS/2", os2()),
        (b"post", post()),
        (b"glyf", glyf),
        (b"loca", loca),
    ];
    tables.extend_from_slice(more);
    sfnt(TRUETYPE, &tables)
}

fn clean() -> Vec<u8> {
    truetype(&[], &[(1, "Test Sans"), (2, "Regular")], &[])
}

fn run(ctx: &BinaryCtx<'_>, id: &str) -> LayerReport {
    audit_binary(ctx, &[find(id).unwrap()], &Config::default())
}

fn codes(r: &LayerReport) -> Vec<&'static str> {
    r.findings.iter().map(|f| f.code).collect()
}

#[derive(Default)]
struct Fake {
    round_trip: Option<RoundTrip>,
    external: Vec<ExternalMessage>,
    tools: Vec<ExternalTool>,
}

impl AuditServices for Fake {
    fn ttx_round_trip(&self, _: &[u8]) -> Result<RoundTrip, Unavailable> {
        self.round_trip.clone().ok_or_else(|| Unavailable::new("no round trip"))
    }

    fn run_external(&self, tool: ExternalTool, _: &[u8]) -> Result<Vec<ExternalMessage>, Unavailable> {
        if self.tools.contains(&tool) { Ok(self.external.clone()) } else { Err(Unavailable::new("not installed")) }
    }
}

fn message(severity: Severity, text: &str) -> ExternalMessage {
    ExternalMessage { severity, id: None, message: text.into(), table: Some("OS/2".into()), glyph: None }
}

#[test]
fn sfnt_version_matches_the_outlines() {
    let services = NoServices;
    let tables = [(b"glyf", vec![0u8; 4]), (b"loca", vec![0u8; 4])];
    let right = sfnt(TRUETYPE, &tables);
    let ctx = BinaryCtx::new("f0", &right, 0, &services).unwrap();
    assert!(run(&ctx, "check.files.sfntVersion").findings.is_empty());

    let wrong = sfnt(OTTO, &tables);
    let ctx = BinaryCtx::new("f0", &wrong, 0, &services).unwrap();
    let r = run(&ctx, "check.files.sfntVersion");
    assert_eq!(codes(&r), ["wrong-sfnt-version-ttf"]);
    assert_eq!(r.findings[0].severity, Severity::Fail);
    assert_eq!(r.findings[0].location.field.as_deref(), Some("sfntVersion"));
    assert_eq!(r.findings[0].fix.map(|f| f.id), Some("patchTable"));

    let cff = sfnt(TRUETYPE, &[(b"CFF ", vec![1, 0, 4, 4])]);
    let ctx = BinaryCtx::new("f0", &cff, 0, &services).unwrap();
    assert_eq!(codes(&run(&ctx, "check.files.sfntVersion")), ["wrong-sfnt-version-cff"]);
    let cff = sfnt(OTTO, &[(b"CFF ", vec![1, 0, 4, 4])]);
    let ctx = BinaryCtx::new("f0", &cff, 0, &services).unwrap();
    assert!(run(&ctx, "check.files.sfntVersion").findings.is_empty());
}

#[test]
fn file_size_reads_the_file_on_disk_and_its_thresholds() {
    let services = NoServices;
    let bytes = clean();
    let small = BinaryCtx::new("f0", &bytes, 0, &services).unwrap();
    assert!(run(&small, "check.files.fileSize").findings.is_empty(), "the font's own bytes are small");

    let large = BinaryCtx::new("f0", &bytes, 0, &services).unwrap().with_file("fonts/TestSans-Regular.ttf", 2_000_000);
    let r = run(&large, "check.files.fileSize");
    assert_eq!(codes(&r), ["large-font"]);
    assert_eq!(r.findings[0].severity, Severity::Warn);
    assert_eq!(r.findings[0].data.get("size"), Some(&2_000_000.0));

    let massive = BinaryCtx::new("f0", &bytes, 0, &services).unwrap().with_file("TestSans-Regular.ttf", 10_000_000);
    let r = run(&massive, "check.files.fileSize");
    assert_eq!(codes(&r), ["massive-font"]);
    assert_eq!(r.findings[0].severity, Severity::Fail);

    let mut config = Config::default();
    config.set("check.files.fileSize", "warnSize", 100.0).unwrap();
    let r = audit_binary(&small, &[find("check.files.fileSize").unwrap()], &config);
    assert_eq!(codes(&r), ["large-font"], "a lower threshold applies");
}

#[test]
fn single_directory_compares_the_directories_of_the_files() {
    let services = NoServices;
    let bytes = clean();
    let check = [find("check.files.singleDirectory").unwrap()];
    let font = |path: &str| BinaryCtx::new("f", &bytes, 0, &services).unwrap().with_file(path, 100);

    let together = [font("fonts/A-Regular.ttf"), font("fonts/A-Bold.ttf")];
    let r = audit_family(&FamilyCtx { fonts: &together, services: &services }, &check, &Config::default());
    assert!(r.findings.is_empty());

    let apart = [font("fonts/A-Regular.ttf"), font("other/A-Bold.ttf"), font("A-Italic.ttf")];
    let r = audit_family(&FamilyCtx { fonts: &apart, services: &services }, &check, &Config::default());
    assert_eq!(codes(&r), ["single-directory"]);
    assert_eq!(r.findings[0].data.get("directories"), Some(&3.0));
    assert!(r.findings[0].message.contains("`fonts`") && r.findings[0].message.contains("`other`"), "{}", r.findings[0].message);

    let in_memory = [BinaryCtx::new("f0", &bytes, 0, &services).unwrap(), BinaryCtx::new("f1", &bytes, 0, &services).unwrap()];
    let r = audit_family(&FamilyCtx { fonts: &in_memory, services: &services }, &check, &Config::default());
    assert!(r.findings.is_empty());
    assert_eq!(r.skipped.len(), 1, "fonts compiled in memory have no directory");
}

#[test]
fn sanitiser_accepts_a_clean_font() {
    let services = NoServices;
    let bytes = clean();
    let ctx = BinaryCtx::new("f0", &bytes, 0, &services).unwrap();
    let r = run(&ctx, "check.files.sanitiser");
    assert!(r.findings.is_empty(), "{:?}", r.findings);
}

#[test]
fn sanitiser_reports_directory_faults() {
    let services = NoServices;

    let mut cut = clean();
    cut.truncate(cut.len() - 8);
    let ctx = BinaryCtx::new("f0", &cut, 0, &services).unwrap();
    let r = run(&ctx, "check.files.sanitiser");
    assert!(codes(&r).contains(&"table-out-of-bounds"), "{:?}", codes(&r));

    // Swap the first two table records: the directory is no longer sorted.
    let mut unsorted = clean();
    let first: Vec<u8> = unsorted[12..28].to_vec();
    let second: Vec<u8> = unsorted[28..44].to_vec();
    unsorted[12..28].copy_from_slice(&second);
    unsorted[28..44].copy_from_slice(&first);
    let ctx = BinaryCtx::new("f0", &unsorted, 0, &services).unwrap();
    assert!(codes(&run(&ctx, "check.files.sanitiser")).contains(&"unsorted-tables"));

    // A wrong search range is a warning.
    let mut search = clean();
    search[6..8].copy_from_slice(&[0, 0]);
    let ctx = BinaryCtx::new("f0", &search, 0, &services).unwrap();
    let r = run(&ctx, "check.files.sanitiser");
    let f = r.findings.iter().find(|f| f.code == "bad-search-fields").unwrap();
    assert_eq!(f.severity, Severity::Warn);

    let bare = sfnt(TRUETYPE, &[(b"glyf", vec![0u8; 4])]);
    let ctx = BinaryCtx::new("f0", &bare, 0, &services).unwrap();
    let r = run(&ctx, "check.files.sanitiser");
    let missing: Vec<Option<&str>> = r.findings.iter().filter(|f| f.code == "missing-table").map(|f| f.location.table.as_deref()).collect();
    assert_eq!(missing.len(), 9, "eight required tables and 'loca': {missing:?}");
    assert!(missing.contains(&Some("OS/2")) && missing.contains(&Some("loca")));
}

#[test]
fn sanitiser_and_rasteriser_report_a_glyph_that_does_not_parse() {
    let services = NoServices;
    let bytes = truetype(&[truncated_glyph()], &[(1, "Test Sans")], &[]);
    let ctx = BinaryCtx::new("f0", &bytes, 0, &services).unwrap();
    let r = run(&ctx, "check.files.sanitiser");
    let bad: Vec<_> = r.findings.iter().filter(|f| f.code == "bad-glyph").collect();
    assert_eq!(bad.len(), 1, "{:?}", r.findings);
    assert_eq!(bad[0].data.get("glyphId"), Some(&2.0));
    assert_eq!(bad[0].location.table.as_deref(), Some("glyf"));

    let r = run(&ctx, "check.files.rasterises");
    assert_eq!(r.findings.len(), 1, "{:?}", r.findings);
    assert!(["no-outline", "draw-failed"].contains(&r.findings[0].code));
    assert_eq!(r.findings[0].data.get("glyphId"), Some(&2.0));
}

#[test]
fn rasteriser_draws_every_glyph_of_a_clean_font() {
    let services = NoServices;
    let bytes = clean();
    let ctx = BinaryCtx::new("f0", &bytes, 0, &services).unwrap();
    let r = run(&ctx, "check.files.rasterises");
    assert!(r.findings.is_empty() && r.skipped.is_empty(), "{r:?}");
}

#[test]
fn sanitiser_passes_on_an_installed_sanitisers_messages() {
    let services = Fake {
        external: vec![message(Severity::Fail, "OS/2: bad range"), message(Severity::Warn, "OS/2: odd value"), message(Severity::Info, "note")],
        tools: vec![ExternalTool::Sanitiser],
        ..Fake::default()
    };
    let bytes = clean();
    let ctx = BinaryCtx::new("f0", &bytes, 0, &services).unwrap();
    let r = run(&ctx, "check.files.sanitiser");
    assert_eq!(codes(&r), ["sanitiser-error", "sanitiser-warning"]);
    assert_eq!(r.findings[1].severity, Severity::Warn);
    assert_eq!(r.findings[0].location.table.as_deref(), Some("OS/2"));
}

#[test]
fn round_trip_reports_a_failed_dump_and_skips_without_the_service() {
    let bytes = clean();
    let none = NoServices;
    let ctx = BinaryCtx::new("f0", &bytes, 0, &none).unwrap();
    let r = run(&ctx, "check.files.roundTrip");
    assert!(r.findings.is_empty());
    assert_eq!(r.skipped.len(), 1);

    let ok = Fake { round_trip: Some(RoundTrip::default()), ..Fake::default() };
    let ctx = BinaryCtx::new("f0", &bytes, 0, &ok).unwrap();
    assert!(run(&ctx, "check.files.roundTrip").findings.is_empty());

    let failed = Fake { round_trip: Some(RoundTrip { tables: Vec::new(), error: Some("bad element".into()) }), ..Fake::default() };
    let ctx = BinaryCtx::new("f0", &bytes, 0, &failed).unwrap();
    let r = run(&ctx, "check.files.roundTrip");
    assert_eq!(codes(&r), ["round-trip-failed"]);
    assert!(r.findings[0].message.contains("bad element"));
}

#[test]
fn external_validator_maps_severities_and_drops_known_issues() {
    let bytes = clean();
    let none = NoServices;
    let ctx = BinaryCtx::new("f0", &bytes, 0, &none).unwrap();
    assert_eq!(run(&ctx, "check.files.externalValidator").skipped.len(), 1);

    let services = Fake {
        external: vec![
            message(Severity::Fail, "The glyph has an invalid contour"),
            message(Severity::Info, "Table checked"),
            ExternalMessage { id: Some("W1404".into()), ..message(Severity::Warn, "The line gap differs from the recommended value") },
        ],
        tools: vec![ExternalTool::Validator],
        ..Fake::default()
    };
    let ctx = BinaryCtx::new("f0", &bytes, 0, &services).unwrap();
    let r = run(&ctx, "check.files.externalValidator");
    assert_eq!(codes(&r), ["validator-error", "validator-info"], "the line-metrics advice is a known false positive");
    assert_eq!(r.findings[0].severity, Severity::Warn, "the validator's errors are warnings here");
    assert_eq!(r.findings[1].severity, Severity::Info);
}

/// One axis record of `fvar`: tag, minimum, default and maximum.
fn fvar(axes: &[(&[u8; 4], i32, i32, i32)]) -> Vec<u8> {
    let mut out = Vec::new();
    for v in [1u16, 0, 16, 2, axes.len() as u16, 20, 0, 4 + 4 * axes.len() as u16] {
        be16(&mut out, v);
    }
    for (tag, min, default, max) in axes {
        out.extend_from_slice(*tag);
        for v in [min, default, max] {
            be32(&mut out, (*v << 16) as u32);
        }
        be16(&mut out, 0);
        be16(&mut out, 256);
    }
    out
}

#[test]
fn canonical_file_name_is_built_from_the_names() {
    let services = NoServices;
    let bytes = truetype(&[], &[(1, "Test Sans"), (2, "Bold")], &[]);
    let at = |path: &str| BinaryCtx::new("f0", &bytes, 0, &services).unwrap().with_file(path, 100);
    assert!(run(&at("out/TestSans-Bold.ttf"), "check.files.canonicalFileName").findings.is_empty());
    let r = run(&at("out/test.ttf"), "check.files.canonicalFileName");
    assert_eq!(codes(&r), ["bad-filename"]);
    assert!(r.findings[0].message.contains("`TestSans-Bold.ttf`"), "{}", r.findings[0].message);
    assert!(run(&at("TestSans-Bold.woff2"), "check.files.canonicalFileName").findings.is_empty(), "a web font keeps its extension");

    let in_memory = BinaryCtx::new("f0", &bytes, 0, &services).unwrap();
    assert_eq!(run(&in_memory, "check.files.canonicalFileName").skipped.len(), 1);

    // Typographic names win over the legacy ones; axis tags are sorted.
    let names = [(1, "Test Sans Light"), (2, "Italic"), (16, "Test Sans"), (17, "Light Italic")];
    let variable = truetype(&[], &names, &[(b"fvar", fvar(&[(b"wght", 100, 400, 900), (b"wdth", 75, 100, 100)]))]);
    let ctx = BinaryCtx::new("f0", &variable, 0, &services).unwrap().with_file("TestSans-Italic[wdth,wght].ttf", 100);
    let r = run(&ctx, "check.files.canonicalFileName");
    assert!(r.findings.is_empty(), "{:?}", r.findings);
}
