//! Tests of the comparison logic on fonts built in code, and of the tool runner with stand-in
//! tools. Nothing here needs a reference compiler.

use std::path::{Path, PathBuf};

use fontcraft_compile::{Options, compile_ttf};
use fontcraft_model::{Font, Path as Contour, Shape};

use super::*;
use crate::outline::{Match, Segment};

fn sample() -> Font {
    let mut f = Font::new("Oracle Test");
    f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Path(Contour::rect(60.0, 0.0, 560.0, 700.0)));
    f.glyph_mut("B").unwrap().layers[0].shapes.push(Shape::Path(Contour::rect(80.0, 0.0, 500.0, 700.0)));
    f
}

/// `f` compiled with its contours as drawn (overlap removal would also fix their direction).
fn build(f: &Font, timestamp: i64) -> Vec<u8> {
    compile_ttf(f, &Options { timestamp, remove_overlap: false, ..Default::default() }).unwrap().bytes
}

fn table<'a>(c: &'a compare::Comparison, tag: &str) -> &'a TableResult {
    c.tables.iter().find(|t| t.tag == tag).unwrap_or_else(|| panic!("no {tag} in {:?}", c.tables))
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-oracle-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_same_font_built_twice_is_identical_whatever_the_timestamp() {
    let c = compare::compare(&build(&sample(), 0), &build(&sample(), 1_700_000_000)).unwrap();
    assert!(c.source_reading.is_empty());
    for t in &c.tables {
        assert_eq!(t.status, Status::Identical, "{t:?}");
    }
    for tag in ["head", "hhea", "maxp", "OS/2", "hmtx", "cmap", "name", "post", "glyf", "loca"] {
        table(&c, tag);
    }
}

#[test]
fn a_changed_advance_is_a_keyed_difference() {
    let mut f = sample();
    f.glyph_mut("A").unwrap().layers[0].width = 640.0;
    let c = compare::compare(&build(&f, 0), &build(&sample(), 0)).unwrap();
    let hmtx = table(&c, "hmtx");
    assert_eq!(hmtx.status, Status::Different);
    assert_eq!(hmtx.differences, vec![Difference { key: "hmtx A advance".into(), ours: "640".into(), theirs: "600".into() }]);
    assert_eq!(table(&c, "cmap").status, Status::Identical);
}

#[test]
fn a_contour_starting_elsewhere_is_equivalent_and_a_reversed_one_is_not() {
    let mut rotated = sample();
    let mut r = Contour::rect(60.0, 0.0, 560.0, 700.0);
    r.nodes.rotate_left(2);
    rotated.glyph_mut("A").unwrap().layers[0].shapes = vec![Shape::Path(r)];
    let c = compare::compare(&build(&rotated, 0), &build(&sample(), 0)).unwrap();
    assert_eq!(table(&c, "glyf").status, Status::Equivalent, "{:?}", table(&c, "glyf"));

    let mut reversed = sample();
    let mut r = Contour::rect(60.0, 0.0, 560.0, 700.0);
    r.nodes.reverse();
    reversed.glyph_mut("A").unwrap().layers[0].shapes = vec![Shape::Path(r)];
    let c = compare::compare(&build(&reversed, 0), &build(&sample(), 0)).unwrap();
    let glyf = table(&c, "glyf");
    assert_eq!(glyf.status, Status::Different);
    assert_eq!(glyf.differences.len(), 1);
    assert_eq!(glyf.differences[0].key, "glyf A outline");
    assert!(glyf.differences[0].theirs.contains("opposite direction"), "{glyf:?}");
}

#[test]
fn glyph_set_and_order_differences_are_source_reading_findings() {
    let mut fewer = sample();
    fewer.remove_glyph("z").unwrap();
    let c = compare::compare(&build(&fewer, 0), &build(&sample(), 0)).unwrap();
    assert_eq!(c.source_reading.len(), 1);
    assert_eq!(c.source_reading[0].kind, FindingKind::GlyphSet);
    assert!(c.source_reading[0].message.contains("1 only in the reference's (z)"), "{:?}", c.source_reading);
    // Per-glyph tables compare the glyphs both fonts have: the outlines still agree.
    assert_ne!(table(&c, "glyf").status, Status::Different);

    let mut moved = sample();
    let b = moved.glyphs.remove("B").unwrap();
    moved.glyphs.push((*b).clone());
    let c = compare::compare(&build(&moved, 0), &build(&sample(), 0)).unwrap();
    assert_eq!(c.source_reading.len(), 1);
    assert_eq!(c.source_reading[0].kind, FindingKind::GlyphOrder);
    // Glyphs are matched by name, so the moved glyph's metrics and code point still agree.
    assert_ne!(table(&c, "hmtx").status, Status::Different);
    assert_ne!(table(&c, "cmap").status, Status::Different);
}

#[test]
fn glyphs_named_differently_are_compared_by_id_and_reported_by_post() {
    let mut ours = sample();
    ours.glyph_mut("B").unwrap().info.production = Some("Bprod".into());
    ours.glyph_mut("B").unwrap().layers[0].width = 640.0;
    let c = compare::compare(&build(&ours, 0), &build(&sample(), 0)).unwrap();
    assert!(c.source_reading.is_empty(), "{:?}", c.source_reading);
    let post = table(&c, "post");
    assert_eq!(post.differences, vec![Difference { key: "post glyphName Bprod".into(), ours: "Bprod".into(), theirs: "B".into() }]);
    // The glyph is still compared, under our name, and its code point still maps to it.
    assert_eq!(table(&c, "hmtx").differences, vec![Difference { key: "hmtx Bprod advance".into(), ours: "640".into(), theirs: "600".into() }]);
    assert_eq!(table(&c, "cmap").status, Status::Identical);
}

#[test]
fn a_changed_code_point_shows_in_cmap_as_a_mapping() {
    let mut f = sample();
    f.glyph_mut("B").unwrap().unicodes.push(0x00DF);
    let c = compare::compare(&build(&f, 0), &build(&sample(), 0)).unwrap();
    let cmap = table(&c, "cmap");
    assert_eq!(cmap.status, Status::Different);
    assert_eq!(cmap.differences, vec![Difference { key: "cmap U+00DF".into(), ours: "B".into(), theirs: "unmapped".into() }]);
}

#[test]
fn name_and_os2_differences_are_named_by_record_and_field() {
    let mut f = sample();
    f.family_name = "Other Family".into();
    let c = compare::compare(&build(&f, 0), &build(&sample(), 0)).unwrap();
    let name = table(&c, "name");
    assert_eq!(name.status, Status::Different);
    assert!(name.differences.iter().any(|d| d.key == "name 3/1/0x0409/1" && d.ours == "Other Family" && d.theirs == "Oracle Test"), "{name:?}");
}

#[test]
fn the_reference_tool_stamp_is_not_part_of_the_version_string() {
    assert_eq!(compare::strip_tool_stamp("Version 1.000;fontc 1.0.0"), ("Version 1.000", true));
    assert_eq!(compare::strip_tool_stamp("Version 1.000;Development"), ("Version 1.000;Development", false));
    assert_eq!(compare::strip_tool_stamp("Version 1.000"), ("Version 1.000", false));
}

#[test]
fn unreadable_and_truncated_fonts_are_errors_not_panics() {
    let good = build(&sample(), 0);
    assert!(compare::compare(&[], &good).is_err());
    assert!(compare::compare(&good, b"not a font").is_err());
    for cut in (0..good.len()).step_by(37) {
        let _ = compare::compare(good.get(..cut).unwrap(), &good);
    }
    // Flipped bytes in table data: whatever the outcome, no panic.
    for at in (12..good.len()).step_by(53) {
        let mut bad = good.clone();
        bad[at] ^= 0xA5;
        let _ = compare::compare(&bad, &good);
    }
}

fn line(x: f32, y: f32) -> Segment {
    Segment::Line([x, y])
}

fn outline(contours: Vec<Vec<Segment>>) -> outline::Outline {
    outline::Outline { contours }
}

#[test]
fn outlines_match_within_half_a_unit_from_any_start_point() {
    let square = vec![line(100.0, 0.0), line(100.0, 100.0), line(0.0, 100.0), line(0.0, 0.0)];
    let a = outline(vec![square.clone()]);
    assert_eq!(outline::compare(&a, &a), Match::Same);
    let mut shifted = square.clone();
    shifted.rotate_left(1);
    assert_eq!(outline::compare(&a, &outline(vec![shifted])), Match::Rotated);
    let near = vec![line(100.4, 0.0), line(100.0, 100.0), line(0.0, 100.0), line(0.0, 0.0)];
    assert_eq!(outline::compare(&a, &outline(vec![near])), Match::Same);
    let far = vec![line(100.6, 0.0), line(100.0, 100.0), line(0.0, 100.0), line(0.0, 0.0)];
    assert!(matches!(outline::compare(&a, &outline(vec![far])), Match::Different { .. }));
    assert!(matches!(outline::compare(&a, &outline(vec![])), Match::Different { .. }));
}

#[test]
fn a_reversed_curved_contour_is_reported_as_a_direction_difference() {
    let curvy = vec![
        Segment::Quad([50.0, -20.0], [100.0, 0.0]),
        Segment::Cubic([120.0, 30.0], [120.0, 70.0], [100.0, 100.0]),
        line(0.0, 100.0),
        line(0.0, 0.0),
    ];
    // The same contour drawn backwards from the origin.
    let back = vec![
        line(0.0, 100.0),
        line(100.0, 100.0),
        Segment::Cubic([120.0, 70.0], [120.0, 30.0], [100.0, 0.0]),
        Segment::Quad([50.0, -20.0], [0.0, 0.0]),
    ];
    match outline::compare(&outline(vec![curvy]), &outline(vec![back])) {
        Match::Different { theirs, .. } => assert!(theirs.contains("opposite direction"), "{theirs}"),
        other => panic!("expected a direction difference, got {other:?}"),
    }
}

#[test]
fn the_allow_list_accepts_a_table_only_when_every_difference_is_listed() {
    let allow = AllowList::parse(
        r#"
        [[allow]]
        table = "OS/2"
        key = "OS/2.usMaxContext"
        reason = "layout"

        [[allow]]
        table = "name"
        key = "name 3/1/0x0409/*"
        oracle = "fontmake"
        source = "only/here"
        reason = "names"
        "#,
    )
    .unwrap();
    let d = |key: &str| Difference { key: key.into(), ours: "a".into(), theirs: "b".into() };
    let t = |tag: &str, diffs: Vec<Difference>| TableResult {
        tag: tag.into(),
        status: Status::Different,
        count: diffs.len(),
        differences: diffs,
        note: None,
        allowed: Vec::new(),
        glyphs: None,
    };
    let mut tables = vec![t("OS/2", vec![d("OS/2.usMaxContext")]), t("name", vec![d("name 3/1/0x0409/5")]), t("hhea", vec![d("hhea.ascender")])];
    apply_allow(&mut tables, &allow, "fontmake", "x/only/here/a.glyphs");
    assert_eq!((tables[0].status, tables[0].allowed.clone()), (Status::Allowed, vec!["layout".to_string()]));
    assert_eq!(tables[1].status, Status::Allowed);
    assert_eq!(tables[2].status, Status::Different);

    // The oracle and the source restrict an entry.
    let mut tables = vec![t("name", vec![d("name 3/1/0x0409/5")]), t("OS/2", vec![d("OS/2.usMaxContext"), d("OS/2.fsType")])];
    apply_allow(&mut tables, &allow, "fontc", "x/only/here/a.glyphs");
    assert_eq!(tables[0].status, Status::Different);
    assert_eq!(tables[1].status, Status::Different);
    assert_eq!(tables[1].differences, vec![d("OS/2.fsType")]);
}

#[test]
fn the_built_in_allow_list_reads_and_every_entry_has_a_reason() {
    let list = AllowList::builtin().unwrap();
    assert!(list.entries.iter().all(|e| !e.reason.trim().is_empty()));
    assert!(AllowList::parse("[[allow]]\ntable = \"head\"\nkey = \"head.flags\"\nreason = \" \"\n").is_err());
    assert!(AllowList::parse("[[allow]]\ntable = \"head\"\nkey = \"head.flags\"\n").is_err());
    assert!(AllowList::parse("[[allow]]\ntable = \"head\"\nkey = \"x\"\nreason = \"r\"\nextra = 1\n").is_err());
}

#[test]
fn the_first_designspace_source_skips_layer_sources() {
    let xml = r#"<?xml version="1.0"?>
<designspace format="5.0">
  <axes><axis tag="wght" name="Weight" minimum="400" maximum="700" default="400"/></axes>
  <sources>
    <source filename="Light.ufo" layer="support" name="layer"/>
    <source name='main' filename='A &amp; B.ufo'>
      <location/>
    </source>
    <source filename="Bold.ufo"/>
  </sources>
</designspace>"#;
    assert_eq!(tools::first_source_filename(xml).as_deref(), Some("A & B.ufo"));
    assert_eq!(tools::first_source_filename("<designspace><sources></sources></designspace>"), None);
    assert_eq!(tools::first_source_filename("<sources><source/></sources>"), None);
    assert_eq!(tools::first_source_filename(""), None);
}

#[test]
fn the_comparison_set_is_found_by_extension_without_entering_sources() {
    let root = temp("corpus");
    let data = root.join("fontc/resources/testdata");
    std::fs::create_dir_all(data.join("glyphs3")).unwrap();
    std::fs::create_dir_all(data.join("A.ufo/glyphs")).unwrap();
    std::fs::write(data.join("A.ufo/glyphs/inner.glyphs"), "").unwrap();
    std::fs::create_dir_all(data.join("P.glyphspackage")).unwrap();
    std::fs::write(data.join("glyphs3/b.glyphs"), "").unwrap();
    std::fs::write(data.join("c.designspace"), "").unwrap();
    std::fs::write(data.join("notes.txt"), "").unwrap();
    let found = corpus::discover(&root).unwrap();
    let rel: Vec<String> = found.iter().map(|p| p.strip_prefix(&data).unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(rel, vec!["A.ufo", "P.glyphspackage", "c.designspace", "glyphs3/b.glyphs"]);
    assert!(corpus::discover(&root.join("nothing")).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_tool_is_found_only_where_it_exists() {
    let dir = temp("locate");
    assert_eq!(tools::locate(Oracle::Fontc, Some(&dir.join("missing"))), None);
    let fake = dir.join("fontc");
    std::fs::write(&fake, "").unwrap();
    assert_eq!(tools::locate(Oracle::Fontc, Some(&fake)), Some(fake));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn colour_sequences_are_removed_from_tool_logs() {
    assert_eq!(tools::strip_ansi("\u{1b}[1;31mERROR\u{1b}[0m bad"), "ERROR bad");
    assert_eq!(tools::strip_ansi("plain"), "plain");
    assert_eq!(tools::strip_ansi("cut \u{1b}["), "cut ");
}

#[test]
fn a_summary_counts_tables_over_compared_sources() {
    let t =
        |tag: &str, status| TableResult { tag: tag.into(), status, differences: Vec::new(), count: 0, note: None, allowed: Vec::new(), glyphs: None };
    let report = |outcome, tables| SourceReport {
        source: "s".into(),
        oracle: Oracle::Fontc,
        flavour: Flavour::TrueType,
        outcome,
        messages: Vec::new(),
        reference: None,
        notes: Vec::new(),
        source_reading: Vec::new(),
        tables,
    };
    let reports = vec![
        report(Outcome::Compared, vec![t("head", Status::Identical), t("GSUB", Status::Skipped)]),
        report(Outcome::Compared, vec![t("head", Status::Different)]),
        report(Outcome::FontcraftFailed, vec![]),
    ];
    let s = summarise(&reports);
    assert_eq!((s.sources, s.compared, s.fontcraft_built), (3, 2, 2));
    let head = &s.tables["head"];
    assert_eq!((head.identical, head.different), (1, 1));
    assert_eq!(s.tables["GSUB"].skipped, 1);
    assert!(render_summary(&s).contains("head"));
}

/// A stand-in reference compiler: a shell script that copies `font` to the path after `-o`.
#[cfg(unix)]
fn fake_tool(dir: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-fontc");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
fn settings(tool: PathBuf, timeout: std::time::Duration) -> Settings {
    Settings {
        oracle: Oracle::Fontc,
        flavour: Flavour::TrueType,
        tool,
        timestamp: DEFAULT_TIMESTAMP,
        timeout,
        keep: None,
        shown: DEFAULT_SHOWN,
        allow: AllowList::default(),
    }
}

#[cfg(unix)]
#[test]
fn a_run_compares_our_build_with_the_tools_output() {
    let dir = temp("run");
    let reference = dir.join("reference.ttf");
    let mut other = sample();
    other.glyph_mut("A").unwrap().layers[0].width = 650.0;
    std::fs::write(&reference, build(&other, 0)).unwrap();
    let source = dir.join("Test.ufo");
    std::fs::create_dir_all(&source).unwrap();
    // Copy the prepared font to the argument after -o.
    let body = format!("while [ $# -gt 0 ]; do if [ \"$1\" = -o ]; then cp '{}' \"$2\"; exit 0; fi; shift; done; exit 3", reference.display());
    let s = settings(fake_tool(&dir, &body), DEFAULT_TIMEOUT);
    let ours = || Ok(Ours { bytes: build(&sample(), 0), warnings: vec!["a warning".into()], hint: Hint::default() });
    let report = check(&source, "Test.ufo", &s, ours);
    assert_eq!(report.outcome, Outcome::Compared, "{report:?}");
    assert!(report.notes.iter().any(|n| n == "FontCraft: a warning"));
    let hmtx = report.tables.iter().find(|t| t.tag == "hmtx").unwrap();
    assert_eq!(hmtx.status, Status::Different);
    assert!(report.has_differences());
    assert!(render(&report).contains("hmtx A advance: 600 | reference: 650"));

    // FontCraft failing still tries the reference; FontCraft not supporting a flavour is not a failure.
    let report = check(&source, "Test.ufo", &s, || Err(OursError::Failed("broken".into())));
    assert_eq!(report.outcome, Outcome::FontcraftFailed);
    assert!(report.reference.is_some());
    let report = check(&source, "Test.ufo", &s, || Err(OursError::Unsupported { message: "later".into(), hint: Hint::default() }));
    assert_eq!(report.outcome, Outcome::NotCompared);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn masters_that_share_a_name_are_compared_with_the_closest() {
    let dir = temp("same-name");
    let mut other = sample();
    other.glyph_mut("A").unwrap().layers[0].width = 650.0;
    let (ours_ttf, other_ttf) = (dir.join("ours.ttf"), dir.join("other.ttf"));
    std::fs::write(&ours_ttf, build(&sample(), 0)).unwrap();
    std::fs::write(&other_ttf, build(&other, 0)).unwrap();
    let source = dir.join("Test.glyphs");
    std::fs::write(&source, "{}").unwrap();
    // fontmake's files for three masters, two of them named Regular: the first master of the
    // name is not the one FontCraft compiled.
    let body = format!(
        "while [ $# -gt 0 ]; do if [ \"$1\" = --output-dir ]; then mkdir -p \"$2\"; cp '{o}' \"$2/Fam-Regular.ttf\"; cp '{u}' \"$2/Fam-Regular#1.ttf\"; cp '{o}' \"$2/Fam-Bold.ttf\"; exit 0; fi; shift; done; exit 3",
        o = other_ttf.display(),
        u = ours_ttf.display()
    );
    let mut s = settings(fake_tool(&dir, &body), DEFAULT_TIMEOUT);
    s.oracle = Oracle::Fontmake;
    let hint = Hint { family: "Fam".into(), style: "Regular".into() };
    let report = check(&source, "Test.glyphs", &s, || Ok(Ours { bytes: build(&sample(), 0), warnings: Vec::new(), hint: hint.clone() }));
    assert_eq!(report.outcome, Outcome::Compared, "{report:?}");
    assert!(!report.has_differences(), "{:?}", report.tables);
    assert!(report.reference.as_ref().unwrap().path.ends_with("Fam-Regular#1.ttf"));
    assert!(report.notes.iter().any(|n| n.contains("2 masters share the name")), "{:?}", report.notes);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_failing_or_hanging_tool_is_reported_with_its_log() {
    let dir = temp("fail");
    let source = dir.join("Test.glyphs");
    std::fs::write(&source, "{}").unwrap();
    let ours = || Ok(Ours { bytes: build(&sample(), 0), ..Default::default() });

    let s = settings(fake_tool(&dir, "echo 'something went wrong' >&2; exit 2"), DEFAULT_TIMEOUT);
    let report = check(&source, "Test.glyphs", &s, ours);
    assert_eq!(report.outcome, Outcome::ReferenceFailed);
    assert!(report.messages.iter().any(|m| m.contains("something went wrong")), "{:?}", report.messages);

    let s = settings(fake_tool(&dir, "sleep 5"), std::time::Duration::from_millis(200));
    let report = check(&source, "Test.glyphs", &s, ours);
    assert_eq!(report.outcome, Outcome::ReferenceFailed);
    assert!(report.messages.iter().any(|m| m.contains("was stopped")), "{:?}", report.messages);

    // A tool that succeeds without writing the font.
    let s = settings(fake_tool(&dir, "exit 0"), DEFAULT_TIMEOUT);
    let report = check(&source, "Test.glyphs", &s, ours);
    assert_eq!(report.outcome, Outcome::ReferenceFailed);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_source_the_tools_do_not_read_is_refused() {
    let dir = temp("kind");
    let s = Settings {
        oracle: Oracle::Fontc,
        flavour: Flavour::TrueType,
        tool: dir.join("none"),
        timestamp: 0,
        timeout: DEFAULT_TIMEOUT,
        keep: None,
        shown: 1,
        allow: AllowList::default(),
    };
    let report = check(&dir.join("font.ttf"), "font.ttf", &s, || Err(OursError::Failed("x".into())));
    assert_eq!(report.outcome, Outcome::BothFailed);
    assert!(report.messages.iter().any(|m| m.contains("not a source")), "{:?}", report.messages);
    assert!(!Oracle::Fontc.builds(Flavour::Cff));
    assert!(Oracle::Fontmake.builds(Flavour::Cff));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn outlines_with_different_points_but_the_same_coverage_are_coverage_equal() {
    use fontcraft_model::{Node, NodeKind};
    let mut ours = sample();
    ours.glyph_mut("A").unwrap().layers[0].shapes =
        vec![Shape::Path(Contour::rect(0.0, 0.0, 400.0, 400.0)), Shape::Path(Contour::rect(200.0, 200.0, 600.0, 600.0))];
    // The same union drawn by hand, with one more point on its bottom edge.
    let corners =
        [(0.0, 0.0), (200.0, 0.0), (400.0, 0.0), (400.0, 200.0), (600.0, 200.0), (600.0, 600.0), (200.0, 600.0), (200.0, 400.0), (0.0, 400.0)];
    let theirs_font = |dx: f64| {
        let mut f = sample();
        let nodes = corners.iter().map(|&(x, y)| Node::new(x + dx, y, NodeKind::Line)).collect();
        f.glyph_mut("A").unwrap().layers[0].shapes = vec![Shape::Path(Contour::new(true, nodes))];
        compile_ttf(&f, &Options { remove_overlap: false, ..Default::default() }).unwrap().bytes
    };
    let ours = compile_ttf(&ours, &Options::default()).unwrap().bytes;

    let exact = compare::compare(&ours, &theirs_font(0.0)).unwrap();
    assert_eq!(table(&exact, "glyf").status, Status::Different);
    let c = compare::compare_with(&ours, &theirs_font(0.0), compare::Outlines::Coverage).unwrap();
    let glyf = table(&c, "glyf");
    assert_eq!(glyf.status, Status::CoverageEqual, "{glyf:?}");
    assert_eq!(glyf.glyphs.map(|g| (g.coverage_equal, g.different)), Some((1, 0)));
    assert_eq!(summarise(&[]).glyphs.len(), 0);

    // Two units to the side is a different shape.
    let c = compare::compare_with(&ours, &theirs_font(2.0), compare::Outlines::Coverage).unwrap();
    let glyf = table(&c, "glyf");
    assert_eq!(glyf.status, Status::Different);
    assert!(glyf.differences.iter().any(|d| d.key == "glyf A outline" && d.ours.contains("coverage differs")), "{glyf:?}");
}
