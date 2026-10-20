//! Audit fixes on compiled fonts (M12.61): `audit.fix` with a file target and the `tables.*`
//! repair commands. Each fault is put into a font exported in code; the check reports it before
//! the fix and not after.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_engine::Session;
use fontcraft_tables::patch::{PatchOptions, PatchValue, patch_table, set_glyph_bounds};
use fontcraft_tables::{SfntFont, TableOrder, WriteOptions};
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-audit-fix-binary-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A font with two glyphs exported to `dir/Font.ttf`.
fn exported(dir: &Path) -> (Session, PathBuf) {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(60.0, 0.0, 560.0, 700.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": rect(80.0, 0.0, 500.0, 700.0)}));
    let ttf = dir.join("Font.ttf");
    run(&mut s, "file.exportStatic", json!({"destination": ttf, "timestamp": 0}));
    (s, ttf)
}

fn read(path: &Path) -> SfntFont {
    fontcraft_tables::read(&std::fs::read(path).unwrap(), 0).unwrap()
}

fn save(path: &Path, font: &SfntFont) {
    std::fs::write(path, fontcraft_tables::write(font, &WriteOptions { order: TableOrder::Keep }).unwrap()).unwrap();
}

fn patch(font: &SfntFont, table: &[u8; 4], fields: &[(&str, f64)]) -> SfntFont {
    let fields = fields.iter().map(|(n, v)| ((*n).to_string(), PatchValue::Number(*v))).collect();
    patch_table(font, &PatchOptions { table: *table, fields }).unwrap()
}

/// A `name` table (format 0) of Windows Unicode records, and Macintosh Roman records for the
/// ASCII ones on platform 1.
fn name_table(records: &[(u16, u16, &str)]) -> Vec<u8> {
    let mut sorted: Vec<&(u16, u16, &str)> = records.iter().collect();
    sorted.sort_by_key(|(p, id, _)| (*p, *id));
    let (mut head, mut storage) = (Vec::new(), Vec::new());
    head.extend(0u16.to_be_bytes());
    head.extend((sorted.len() as u16).to_be_bytes());
    head.extend((6 + 12 * sorted.len() as u16).to_be_bytes());
    for (platform, id, text) in sorted {
        let bytes: Vec<u8> = if *platform == 1 { text.bytes().collect() } else { text.encode_utf16().flat_map(u16::to_be_bytes).collect() };
        let (encoding, language) = if *platform == 1 { (0u16, 0u16) } else { (1, 0x409) };
        for v in [*platform, encoding, language, *id, bytes.len() as u16, storage.len() as u16] {
            head.extend(v.to_be_bytes());
        }
        storage.extend(bytes);
    }
    head.extend(storage);
    head
}

/// The results of `checks` on `path` that carry a fix.
fn fixable(s: &mut Session, path: &Path, checks: &[&str]) -> Vec<(String, String)> {
    let r = run(s, "audit.run", json!({"target": path, "profile": "strict", "checks": checks, "maxResults": 10000}));
    pairs(&r)
}

fn pairs(doc: &Value) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = doc["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["fix"]["id"].is_string())
        .map(|r| (r["check"].as_str().unwrap().to_string(), r["code"].as_str().unwrap().to_string()))
        .collect();
    out.sort();
    out.dedup();
    out
}

const TABLE_CHECKS: [&str; 10] = [
    "check.metrics.maxAdvanceWidth",
    "check.metrics.caretSlope",
    "check.metrics.fontBBox",
    "check.metrics.xAvgCharWidth",
    "check.metrics.strikeoutAndScripts",
    "check.tables.os2Version",
    "check.tables.dsig",
    "check.outlines.pointsOutsideBBox",
    "check.hinting.smartDropout",
    "check.names.whitespace",
];

#[test]
fn audit_fix_on_a_file_clears_each_table_fault() {
    let dir = scratch("faults");
    let (mut s, ttf) = exported(&dir);
    let mut font = read(&ttf);
    font = patch(&font, b"hhea", &[("advanceWidthMax", 1.0), ("caretSlopeRise", 0.0)]);
    font = patch(&font, b"head", &[("yMax", 5000.0)]);
    font = patch(&font, b"OS/2", &[("xAvgCharWidth", 1.0), ("yStrikeoutSize", 0.0)]);
    let mut os2 = font.get(*b"OS/2").unwrap().to_vec();
    os2[0..2].copy_from_slice(&3u16.to_be_bytes());
    font.insert(*b"OS/2", os2);
    font.insert(*b"DSIG", vec![0, 0, 0, 1, 0, 0, 0, 0]);
    font.remove(*b"prep");
    let bytes = std::fs::read(&ttf).unwrap();
    let a = skrifa::MetadataProvider::charmap(&skrifa::FontRef::new(&bytes).unwrap()).map('A').unwrap().to_u32() as u16;
    font = set_glyph_bounds(&font, &[(a, [100, 100, 200, 200])]).unwrap();
    font.insert(*b"name", name_table(&[(3, 1, "Fix  Me "), (3, 2, "Regular"), (3, 4, "Fix Me Regular"), (3, 6, "FixMe-Regular"), (1, 1, "Fix")]));
    save(&ttf, &font);
    let original = std::fs::read(&ttf).unwrap();

    let before = fixable(&mut s, &ttf, &TABLE_CHECKS);
    for check in TABLE_CHECKS {
        assert!(before.iter().any(|(c, _)| c == check), "{check} reports the fault before the fix: {before:?}");
    }
    let mut checks: Vec<&str> = TABLE_CHECKS.to_vec();
    checks.push("check.names.noMacRecords");
    let r = run(&mut s, "audit.fix", json!({"target": ttf, "profile": "strict", "checks": checks, "safeOnly": false}));
    let output = PathBuf::from(r["output"].as_str().unwrap());
    assert_eq!(output, dir.join("Font.fixed.ttf"));
    assert_eq!(std::fs::read(&ttf).unwrap(), original, "the input is never changed");
    let commands: Vec<&str> = r["applied"].as_array().unwrap().iter().map(|a| a["command"].as_str().unwrap()).collect();
    assert_eq!(commands, ["tables.delete", "tables.cleanNameRecords", "tables.recalculate"], "the file-target order");
    assert_eq!(pairs(&r["remaining"]), Vec::<(String, String)>::new(), "every fault is cleared");
    assert_eq!(fixable(&mut s, &output, &checks), Vec::<(String, String)>::new());
    let leftovers: Vec<_> =
        std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().starts_with('.')).collect();
    assert!(leftovers.is_empty(), "no temporary file is left behind");
}

#[test]
fn audit_fix_on_a_file_plans_skips_and_writes_all_or_nothing() {
    let dir = scratch("plan");
    let (mut s, ttf) = exported(&dir);
    let font = patch(&read(&ttf), b"hhea", &[("advanceWidthMax", 1.0)]);
    let mut font = font;
    font.insert(*b"DSIG", vec![0, 0, 0, 1, 0, 0, 0, 0]);
    save(&ttf, &font);
    let checks = ["check.metrics.maxAdvanceWidth", "check.tables.dsig", "check.metrics.useTypoMetrics"];

    let dry = run(&mut s, "audit.fix", json!({"target": ttf, "profile": "strict", "checks": checks, "dryRun": true}));
    let planned: Vec<&str> = dry["planned"].as_array().unwrap().iter().map(|c| c["command"].as_str().unwrap()).collect();
    assert_eq!(planned, ["tables.delete", "tables.recalculate"]);
    assert_eq!(dry["planned"][1]["params"]["fields"], json!(["hhea.advanceWidthMax"]));
    assert!(!dir.join("Font.fixed.ttf").exists(), "a dry run writes nothing");

    // useTypoMetrics changes the design: skipped unless safeOnly is false.
    let r = run(&mut s, "audit.fix", json!({"target": ttf, "profile": "strict", "checks": checks, "inPlace": true}));
    assert_eq!(r["output"], json!(ttf));
    let skipped = r["skipped"].as_array().unwrap();
    if let Some(typo) = skipped.iter().find(|k| k["check"] == "check.metrics.useTypoMetrics") {
        assert!(typo["reason"].as_str().unwrap().contains("safeOnly"));
    }
    assert!(fixable(&mut s, &ttf, &checks[..2]).is_empty(), "the input itself is fixed with inPlace");

    // A refused write leaves the existing output alone.
    let font = patch(&read(&ttf), b"hhea", &[("advanceWidthMax", 1.0)]);
    save(&ttf, &font);
    let out = dir.join("Out.ttf");
    std::fs::write(&out, b"keep").unwrap();
    let refused = s.execute("audit.fix", &json!({"target": ttf, "profile": "strict", "checks": checks, "output": out, "noClobber": true}));
    assert!(refused.is_err());
    assert_eq!(std::fs::read(&out).unwrap(), b"keep");
    let leftovers = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().starts_with('.')).count();
    assert_eq!(leftovers, 0, "the temporary copy is removed");
    assert!(s.execute("audit.fix", &json!({"target": ttf, "results": ["r1"]})).is_err(), "result ids belong to the open document");
}

#[test]
fn tables_recalculate_recomputes_the_named_fields_only() {
    let dir = scratch("recalc");
    let (mut s, ttf) = exported(&dir);
    let font = patch(&read(&ttf), b"hhea", &[("advanceWidthMax", 1.0), ("caretSlopeRise", 0.0)]);
    let font = patch(&font, b"OS/2", &[("fsType", 8.0)]);
    save(&ttf, &font);
    let r = run(&mut s, "tables.recalculate", json!({"font": ttf, "fields": ["hhea.caretSlopeRun"], "values": {"OS/2.fsType": 0}}));
    let fields: Vec<&str> = r["changed"].as_array().unwrap().iter().map(|c| c["field"].as_str().unwrap()).collect();
    assert_eq!(fields, ["hhea.caretSlopeRise", "OS/2.fsType"], "one field of a group stands for the group; values come last");
    let out = read(Path::new(r["output"].as_str().unwrap()));
    assert_eq!(fontcraft_tables::patch::read_field(&out, *b"hhea", "advanceWidthMax").unwrap(), PatchValue::Number(1.0), "fields not named stay");
    assert_eq!(fontcraft_tables::patch::read_field(&out, *b"hhea", "caretSlopeRise").unwrap(), PatchValue::Number(1.0));

    let r = run(&mut s, "tables.recalculate", json!({"font": ttf, "inPlace": true}));
    assert!(r["changed"].as_array().unwrap().iter().any(|c| c["field"] == "hhea.advanceWidthMax"), "the defaults include advanceWidthMax");
    assert!(s.execute("tables.recalculate", &json!({"font": ttf, "fields": ["OS/2.achVendID"]})).is_err(), "not a derived field");
    assert!(s.execute("tables.recalculate", &json!({"font": ttf, "fields": ["OS/2.usWeightClass"]})).is_err(), "a static font has no wght axis");
}

#[test]
fn tables_recalculate_recomputes_maxp_counts_and_vhea_extents() {
    let dir = scratch("recalc-maxp-vhea");
    let (mut s, ttf) = exported(&dir);
    let field = |f: &SfntFont, table: &[u8; 4], name: &str| fontcraft_tables::patch::read_field(f, *table, name).unwrap();
    let original = read(&ttf);
    let points = field(&original, b"maxp", "maxPoints");
    let contours = field(&original, b"maxp", "maxContours");
    assert!(s.execute("tables.recalculate", &json!({"font": ttf, "fields": ["vhea.extents"]})).is_err(), "the font has no vhea");

    // Wrong maxp maxima, and vertical metrics with every advance 1000 and every top side
    // bearing 100 under a vhea whose derived fields are all 0.
    let mut font = patch(&original, b"maxp", &[("maxPoints", 1.0), ("maxContours", 0.0)]);
    let maxp = font.get(*b"maxp").unwrap();
    let count = u16::from_be_bytes([maxp[4], maxp[5]]);
    let mut vhea = Vec::new();
    for v in [0x0001u16, 0x1000, 880, (-120i16) as u16, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, count] {
        vhea.extend(v.to_be_bytes());
    }
    let vmtx: Vec<u8> = (0..count).flat_map(|_| [1000u16.to_be_bytes(), 100i16.to_be_bytes()].concat()).collect();
    font.insert(*b"vhea", vhea);
    font.insert(*b"vmtx", vmtx);
    save(&ttf, &font);

    let r = run(&mut s, "tables.recalculate", json!({"font": ttf, "fields": ["maxp.maxPoints", "vhea.yMaxExtent"], "inPlace": true}));
    let fields: Vec<&str> = r["changed"].as_array().unwrap().iter().map(|c| c["field"].as_str().unwrap()).collect();
    assert!(fields.contains(&"maxp.maxPoints") && fields.contains(&"maxp.maxContours"), "{r}");
    assert!(fields.contains(&"vhea.advanceHeightMax") && fields.contains(&"vhea.yMaxExtent"), "{r}");
    let out = read(&ttf);
    assert_eq!(field(&out, b"maxp", "maxPoints"), points, "the compiler's value again");
    assert_eq!(field(&out, b"maxp", "maxContours"), contours);
    assert_eq!(field(&out, b"vhea", "advanceHeightMax"), PatchValue::Number(1000.0));
    assert_eq!(field(&out, b"vhea", "minTopSideBearing"), PatchValue::Number(100.0));
    // A and B are 700 units tall: 1000 - 100 - 700 below, 100 + 700 above at least.
    let PatchValue::Number(below) = field(&out, b"vhea", "minBottomSideBearing") else { panic!() };
    let PatchValue::Number(extent) = field(&out, b"vhea", "yMaxExtent") else { panic!() };
    assert!(below <= 200.0 && extent >= 800.0, "{below} {extent}");
    assert_eq!(field(&out, b"vhea", "vertTypoAscender"), PatchValue::Number(880.0), "fields not derived stay");
}

#[test]
fn tables_clean_name_records_and_rebuild_cmap_work_on_files() {
    let dir = scratch("names");
    let (mut s, ttf) = exported(&dir);
    let mut font = read(&ttf);
    font.insert(*b"name", name_table(&[(3, 1, "Fix\nMe"), (3, 3, "  "), (3, 6, "FixMe-Regular"), (1, 1, "Fix")]));
    save(&ttf, &font);
    let r = run(&mut s, "tables.cleanNameRecords", json!({"font": ttf}));
    assert_eq!((r["removed"].as_u64(), r["changed"].as_u64()), (Some(2), Some(1)));
    let out = PathBuf::from(r["output"].as_str().unwrap());
    assert!(fixable(&mut s, &out, &["check.names.lineBreaks", "check.names.emptyRecords", "check.names.noMacRecords"]).is_empty());

    let r = run(&mut s, "tables.rebuildCmap", json!({"font": ttf, "output": dir.join("Cmap.ttf")}));
    assert!(r["mappings"].as_u64().unwrap() >= 2);
    let before = run(&mut s, "audit.run", json!({"target": ttf, "profile": "strict", "checks": ["check.tables.cmapFormat12"]}));
    let after = run(&mut s, "audit.run", json!({"target": dir.join("Cmap.ttf"), "profile": "strict", "checks": ["check.tables.cmapFormat12"]}));
    assert_eq!(pairs(&after), pairs(&before), "a font without code points beyond the BMP keeps passing");
    assert!(s.execute("tables.dropAxis", &json!({"font": ttf, "axis": "wght"})).is_err(), "a static font has no axis to drop");
}

#[test]
fn audit_fix_on_a_file_builds_a_missing_hvar_table() {
    let dir = scratch("hvar");
    let source = dir.join("TestSans.glyphs");
    std::fs::write(&source, crate::export::TWO_MASTERS).unwrap();
    let mut s = Session::new();
    run(&mut s, "file.open", json!({"path": source}));
    run(&mut s, "file.exportVariable", json!({"destination": dir, "timestamp": 0}));
    let ttf = dir.join("TestSansVF.ttf");
    let mut font = read(&ttf);
    font.remove(*b"HVAR");
    save(&ttf, &font);
    let checks = ["check.variations.hvarMissing"];
    let found = run(&mut s, "audit.run", json!({"target": ttf, "profile": "strict", "checks": checks}));
    let fix = &found["results"][0]["fix"];
    assert_eq!(fix["available"], true, "{fix}");
    assert_eq!(fix["command"], "audit.fix");
    assert_eq!(fix["params"], json!({"target": ttf, "checks": checks}));
    assert_eq!(fix["via"], "variations.buildHvar");

    let r = run(&mut s, "audit.fix", json!({"target": ttf, "profile": "strict", "checks": checks}));
    assert_eq!(r["applied"][0]["command"], "variations.buildHvar", "{r}");
    assert_eq!(r["applied"][0]["result"]["hvar"], "built", "{r}");
    let fixed = dir.join("TestSansVF.fixed.ttf");
    assert!(read(&fixed).get(*b"HVAR").is_some());
    assert!(fixable(&mut s, &fixed, &checks).is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn audit_fix_in_place_through_a_symlink_writes_the_linked_file_and_keeps_the_link() {
    let dir = scratch("symlink");
    let (mut s, real) = exported(&dir);
    save(&real, &patch(&read(&real), b"hhea", &[("advanceWidthMax", 1.0)]));
    let link = dir.join("Link.ttf");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let checks = ["check.metrics.maxAdvanceWidth"];
    assert!(!fixable(&mut s, &link, &checks).is_empty());

    let r = run(&mut s, "audit.fix", json!({"target": link, "profile": "strict", "checks": checks, "inPlace": true}));
    assert_eq!(r["applied"].as_array().unwrap().len(), 1, "{r}");
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link stays a link");
    assert!(fixable(&mut s, &real, &checks).is_empty(), "the linked file is fixed");
}

#[test]
fn tables_recalculate_rewrites_the_cff_font_bbox_and_is_fixed_pitch() {
    use fontcraft_tables::patch::{CFF_FONT_BBOX, CFF_IS_FIXED_PITCH, cff_top_ints, set_cff_top_ints};
    let dir = scratch("recalc-cff-top");
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(60.0, 0.0, 560.0, 700.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": rect(80.0, -10.0, 500.0, 720.0)}));
    let otf = dir.join("Font.otf");
    run(&mut s, "file.exportStatic", json!({"outlines": "cff", "destination": otf, "timestamp": 0}));
    // post.isFixedPitch as the advances give it, so that only the CFF entry disagrees.
    run(&mut s, "tables.recalculate", json!({"font": otf, "fields": ["post.isFixedPitch"], "inPlace": true}));
    let original = read(&otf);
    let PatchValue::Number(post_fixed) = fontcraft_tables::patch::read_field(&original, *b"post", "isFixedPitch").unwrap() else { panic!() };
    let fixed = i64::from(post_fixed != 0.0);
    let compiled_box = cff_top_ints(&original, CFF_FONT_BBOX).unwrap();
    assert!(compiled_box.is_some());

    let broken = set_cff_top_ints(&original, CFF_FONT_BBOX, &[0, 0, 1, 1]).unwrap();
    let broken = set_cff_top_ints(&broken, CFF_IS_FIXED_PITCH, &[1 - fixed]).unwrap();
    save(&otf, &broken);
    let checks = json!(["check.metrics.fontBBox", "check.metrics.monospaceFlags"]);
    let codes = |doc: &Value| -> Vec<String> {
        let mut list: Vec<String> = doc["results"].as_array().unwrap().iter().map(|r| r["code"].as_str().unwrap().to_string()).collect();
        list.retain(|c| c.starts_with("head-cff") || c.starts_with("cff-"));
        list.sort();
        list
    };
    let before = run(&mut s, "audit.run", json!({"target": otf, "checks": checks}));
    assert_eq!(codes(&before), ["cff-is-fixed-pitch", "head-cff-differ"], "{before}");
    let plan = run(&mut s, "audit.fix", json!({"target": otf, "checks": checks, "safeOnly": false, "dryRun": true}));
    let fields = plan["planned"].as_array().unwrap().iter().find(|c| c["command"] == "tables.recalculate").map(|c| c["params"]["fields"].clone());
    let fields = fields.unwrap_or_else(|| panic!("{plan}"));
    assert!(fields.as_array().unwrap().contains(&json!("CFF.FontBBox")) && fields.as_array().unwrap().contains(&json!("CFF.isFixedPitch")), "{plan}");

    let r = run(&mut s, "tables.recalculate", json!({"font": otf, "fields": ["CFF.FontBBox", "CFF.isFixedPitch"], "inPlace": true}));
    let changed: Vec<&str> = r["changed"].as_array().unwrap().iter().map(|c| c["field"].as_str().unwrap()).collect();
    assert_eq!(changed, ["CFF.FontBBox", "CFF.isFixedPitch"], "{r}");
    let out = read(&otf);
    assert_eq!(cff_top_ints(&out, CFF_FONT_BBOX).unwrap(), compiled_box, "the compiler's box again");
    assert_eq!(cff_top_ints(&out, CFF_IS_FIXED_PITCH).unwrap().map(|v| v[0]).unwrap_or(0), fixed);
    for (tag, bytes) in &original.tables {
        // head differs in its checksum adjustment only.
        if tag != b"CFF " && tag != b"head" {
            assert_eq!(out.get(*tag), Some(bytes.as_slice()), "{} is left as it was", String::from_utf8_lossy(tag));
        }
    }
    let after = run(&mut s, "audit.run", json!({"target": otf, "checks": checks}));
    assert!(codes(&after).is_empty(), "{after}");
}

#[test]
fn audit_fix_on_the_files_of_a_family_makes_their_family_values_equal() {
    use fontcraft_tables::patch::read_field;
    let dir = scratch("family-sync");
    let (mut s, regular_path) = exported(&dir);
    let number = |f: &SfntFont, table: &[u8; 4], name: &str| match read_field(f, *table, name).unwrap() {
        PatchValue::Number(n) => n,
        other => panic!("{name}: {other:?}"),
    };
    let regular = read(&regular_path);
    let ascender = number(&regular, b"hhea", "ascender");
    let underline = number(&regular, b"post", "underlinePosition");
    // A bold with a higher ascender and a lower underline.
    let bold = patch(&regular, b"hhea", &[("ascender", ascender + 50.0)]);
    let bold = patch(&bold, b"post", &[("underlinePosition", underline - 40.0)]);
    let bold_path = dir.join("Bold.ttf");
    save(&bold_path, &bold);
    let checks = json!(["check.family.verticalMetrics", "check.family.underlineConsistency"]);
    let target = json!([regular_path, bold_path]);
    let before = run(&mut s, "audit.run", json!({"target": target, "checks": checks}));
    assert_eq!(before["results"].as_array().unwrap().len(), 2, "{before}");
    assert!(s.execute("audit.fix", &json!({"target": target, "checks": checks, "safeOnly": false, "output": dir.join("x.ttf")})).is_err());

    let r = run(&mut s, "audit.fix", json!({"target": target, "checks": checks, "safeOnly": false, "inPlace": true}));
    assert_eq!(r["outputs"].as_array().unwrap().len(), 2, "both files change: {r}");
    assert!(r["applied"].as_array().unwrap().iter().all(|a| a["fix"] == "syncFamilyValues" && a["command"] == "tables.recalculate"), "{r}");
    assert!(r["remaining"]["results"].as_array().unwrap().is_empty(), "{r}");
    let (a, b) = (read(&regular_path), read(&bold_path));
    assert_eq!(number(&a, b"hhea", "ascender"), ascender + 50.0, "the higher ascender wins");
    assert_eq!(number(&b, b"hhea", "ascender"), ascender + 50.0);
    assert_eq!(number(&a, b"post", "underlinePosition"), number(&b, b"post", "underlinePosition"));
    for (font, was) in [(&a, &regular), (&b, &bold)] {
        for (tag, bytes) in &was.tables {
            if !matches!(tag, b"hhea" | b"post" | b"head") {
                assert_eq!(font.get(*tag), Some(bytes.as_slice()), "{} is left as it was", String::from_utf8_lossy(tag));
            }
        }
    }
}
