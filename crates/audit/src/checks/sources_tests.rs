//! Behaviour tests of the `check.sources.*` checks on sources built in code.

use std::sync::Arc;

use fontcraft_model::plist::{Array, Number, Value};
use fontcraft_model::{Axis, AxisRange, Family, Font, Glyph, Layer, Master, SourceFormat};

use crate::services::{AuditServices, SourceFileIssue, SourceFileProblem, Unavailable};
use crate::{AuditContext, Config, LayerReport, NoServices, Severity, SourceCtx, audit_source, find};

fn run_with(font: &Font, services: &dyn AuditServices, id: &str) -> LayerReport {
    let audit = AuditContext::new(font);
    audit_source(&SourceCtx { audit: &audit, services }, &[find(id).unwrap()], &Config::default())
}

fn run(font: &Font, id: &str) -> LayerReport {
    run_with(font, &NoServices, id)
}

fn codes(r: &LayerReport) -> Vec<&'static str> {
    r.findings.iter().map(|f| f.code).collect()
}

fn codepoint_list(u: &[i64]) -> Value {
    Value::Array(Array { items: u.iter().map(|c| Value::Number(Number::int(*c))).collect(), ..Default::default() })
}

/// Two masters on a weight axis from 400 to 700, Regular (the designspace's default source) at
/// 400 and Bold at 700.
fn two_masters() -> Font {
    let mut font = Font::new("Test");
    font.axes = Arc::new(vec![Axis {
        name: "Weight".into(),
        tag: "wght".into(),
        range: Some(AxisRange { minimum: 400.0, default: 400.0, maximum: 700.0 }),
        ..Default::default()
    }]);
    let masters = Arc::make_mut(&mut font.masters);
    if let Some(regular) = masters.first_mut() {
        regular.location = vec![400.0];
    }
    let bold = Master { id: "m02".into(), name: "Bold".into(), location: vec![700.0], ..masters[0].clone() };
    masters.push(bold);
    Arc::make_mut(&mut font.extra).push(Family::Designspace, "defaultSource", Value::String("m01".into()));
    font
}

#[test]
fn required_info_fields_reads_the_masters_metrics() {
    let mut font = Font::new("Test");
    assert!(run(&font, "check.sources.requiredInfoFields").findings.is_empty());

    Arc::make_mut(&mut font.masters)[0].metric_values.truncate(2);
    font.units_per_em = 0;
    font.family_name = " ".into();
    let r = run(&font, "check.sources.requiredInfoFields");
    assert_eq!(codes(&r), ["missing-required-fields", "missing-required-fields"]);
    assert_eq!(r.findings[0].message, "The font does not set units per em and a family name.");
    assert_eq!(r.findings[1].message, "Master Regular does not set its descender and x-height.");
    assert_eq!(r.findings[1].location.master.as_deref(), Some("m01"));
    assert_eq!(r.findings[1].severity, Severity::Fail);
}

#[test]
fn recommended_and_unnecessary_info_fields() {
    let mut font = Font::new("Test");
    let r = run(&font, "check.sources.recommendedInfoFields");
    assert_eq!(codes(&r), ["missing-recommended-fields", "missing-recommended-fields"]);
    assert_eq!(r.findings[0].message, "The font sets no copyright.");
    assert_eq!(r.findings[1].message, "Master Regular sets no PANOSE values.");
    assert_eq!(r.findings[1].severity, Severity::Warn);

    let info = Arc::make_mut(&mut font.info);
    info.properties.set("copyrights", "Copyright 2026 Test", true);
    info.params.set("panose", Value::String("2 0 5 3 0 0 0 0 0 0".into()));
    assert!(run(&font, "check.sources.recommendedInfoFields").findings.is_empty());

    assert!(run(&font, "check.sources.unnecessaryInfoFields").findings.is_empty());
    let info = Arc::make_mut(&mut font.info);
    info.params.set("year", Value::Number(Number::int(2026)));
    info.properties.set("versionString", "Version 1.0", false);
    Arc::make_mut(&mut font.masters)[0].params.set("postscriptUniqueID", Value::Number(Number::int(4_000_000)));
    let r = run(&font, "check.sources.unnecessaryInfoFields");
    assert_eq!(codes(&r), ["unnecessary-fields", "unnecessary-fields"]);
    assert!(r.findings[0].message.starts_with("The font info sets year and versionString,"), "{}", r.findings[0].message);
    assert_eq!(r.findings[1].location.master.as_deref(), Some("m01"));
    assert_eq!(r.findings[0].fix.map(|f| f.id), Some("clearInfoFields"));
}

#[test]
fn default_master_sits_at_every_axis_default() {
    let mut font = two_masters();
    assert!(run(&font, "check.sources.defaultMaster").findings.is_empty());

    Arc::make_mut(&mut font.axes)[0].range = Some(AxisRange { minimum: 400.0, default: 500.0, maximum: 700.0 });
    let r = run(&font, "check.sources.defaultMaster");
    assert_eq!(codes(&r), ["not-found"]);
    assert!(r.findings[0].message.contains("wght 500"), "{}", r.findings[0].message);
}

#[test]
fn masters_agree_with_the_default_master_on_glyphs_and_codepoints() {
    let mut font = two_masters();
    assert!(run(&font, "check.sources.consistentGlyphSet").findings.is_empty(), "glyphs the other master lacks are allowed");

    let mut extra = Glyph::new("extra");
    extra.layers.push(Layer::new("m02", 500.0));
    font.glyphs.push(extra);
    let r = run(&font, "check.sources.consistentGlyphSet");
    assert_eq!(codes(&r), ["inconsistent-glyphset"]);
    assert_eq!(r.findings[0].location.glyph, "extra");
    assert_eq!(r.findings[0].message, "Glyph extra is in master Bold but not in the default master Regular.");

    assert!(run(&font, "check.sources.consistentCodepoints").findings.is_empty());
    let with_own = |name: &str, unicode: u32, own: i64, read: i64| {
        let mut g = Glyph::new(name);
        g.unicodes = vec![unicode];
        g.layers.push(Layer::new("m01", 600.0));
        let mut bold = Layer::new("m02", 600.0);
        bold.extra.push(Family::Ufo, "unicodes", codepoint_list(&[own]));
        bold.extra.push(Family::Ufo, "unicodesRead", codepoint_list(&[read]));
        g.layers.push(bold);
        g
    };
    font.glyphs.remove("A");
    font.glyphs.push(with_own("A", 0x41, 0x61, 0x41));
    // The glyph's code points changed after reading: the master's own list no longer counts.
    font.glyphs.remove("B");
    font.glyphs.push(with_own("B", 0x42, 0x62, 0x43));
    let r = run(&font, "check.sources.consistentCodepoints");
    assert_eq!(codes(&r), ["inconsistent-codepoints"]);
    assert_eq!(r.findings[0].location.glyph, "A");
    assert_eq!(r.findings[0].location.master.as_deref(), Some("m02"));
    assert_eq!(r.findings[0].message, "Glyph A has U+0061 in master Bold but U+0041 in the default master Regular.");
    assert_eq!(r.findings[0].fix.map(|f| f.command), Some(Some("glyph.updateGlyphInfo")));
}

struct Files(Vec<SourceFileIssue>);

impl AuditServices for Files {
    fn check_source_files(&self, _: &Font) -> Result<Vec<SourceFileIssue>, Unavailable> {
        Ok(self.0.clone())
    }
}

fn issue(problem: SourceFileProblem, file: &str) -> SourceFileIssue {
    SourceFileIssue { problem, file: file.into(), message: "cannot be read".into() }
}

#[test]
fn designspace_and_ufo_checks_read_the_source_files() {
    let services = Files(vec![
        issue(SourceFileProblem::DesignspaceUnreadable, "Test.designspace"),
        issue(SourceFileProblem::SourceUnreadable, "Test-Bold.ufo"),
        issue(SourceFileProblem::UfoStructure, "Test-Regular.ufo/glyphs/a.glif"),
    ]);
    let mut font = two_masters();
    assert_eq!(run_with(&font, &services, "check.sources.designspaceLoads").skipped.len(), 1, "not a designspace");
    assert_eq!(run_with(&font, &services, "check.sources.ufoStructure").skipped.len(), 1, "not a UFO");

    font.source.format = SourceFormat::Designspace;
    let r = run_with(&font, &services, "check.sources.designspaceLoads");
    assert_eq!(codes(&r), ["unreadable", "source-unreadable"]);
    assert_eq!(r.findings[1].message, "`Test-Bold.ufo`: cannot be read");
    let r = run_with(&font, &services, "check.sources.ufoStructure");
    assert_eq!(codes(&r), ["invalid-ufo"]);

    let r = run(&font, "check.sources.designspaceLoads");
    assert!(r.findings.is_empty());
    assert_eq!(r.skipped.len(), 1, "the files cannot be read again without the service");

    Arc::make_mut(&mut font.masters).clear();
    let r = run(&font, "check.sources.designspaceLoads");
    assert_eq!(codes(&r), ["no-sources"]);
}
