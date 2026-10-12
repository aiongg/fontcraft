//! Source file checks (M12.57): whether a designspace and its UFOs load, whether the masters
//! agree with the default master on glyphs and code points, and which font-info values are set
//! (`docs/spec/21-qa.md` § 8, "Source files"). The rules follow FontBakery's `designspace_*` and
//! `ufo_*_fields` checks; `ufoStructure` is our own validation, read through
//! [`crate::AuditServices::check_source_files`].
//!
//! The model holds a source in the reference's terms, so FontBakery's UFO fields map onto it:
//! metrics are the masters' metric values, the style name is the master's name, the copyright a
//! property, PANOSE and the UFO-only keys custom parameters (`io-ufo`'s `fontinfo.rs`).

use fontcraft_model::plist::{Num, Value};
use fontcraft_model::{Family, Font, Master, Params as InfoParams, Properties, SourceFormat, axes, metric};

use crate::services::{MAX_SOURCE_FILE_ISSUES, SourceFileProblem};
use crate::{Hit, Location, Params, Sink, SourceCtx, num};

/// Design coordinates closer than this are the same location.
const LOCATION_TOLERANCE: f64 = 0.001;

/// The metrics the compiler needs from every master, with their font-info names.
const REQUIRED_METRICS: [(&str, &str); 4] =
    [(metric::ASCENDER, "ascender"), (metric::DESCENDER, "descender"), (metric::X_HEIGHT, "x-height"), (metric::CAP_HEIGHT, "cap height")];

/// The parameters that hold PANOSE values: the reference's name and the UFO key.
const PANOSE: [&str; 2] = ["panose", "openTypeOS2Panose"];

/// Font-info values the compiler generates or that are obsolete, as properties.
const UNNECESSARY_PROPERTIES: [&str; 2] = ["uniqueID", "versionString"];
/// The same as parameters: UFO keys kept under their own names, and the names version 2 files use.
const UNNECESSARY_PARAMS: [&str; 6] = ["openTypeNameUniqueID", "openTypeNameVersion", "postscriptUniqueID", "year", "uniqueID", "versionString"];

/// `a`, `a and b`, `a, b and c`.
fn list(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn master_place(m: &Master) -> Location {
    Location { master: Some(m.id.clone()), ..Location::default() }
}

/// The default master: the one a designspace names, else the origin master.
fn find_default_master(font: &Font) -> Option<&Master> {
    font.extra
        .get(Family::Designspace, "defaultSource")
        .and_then(Value::as_str)
        .and_then(|id| font.masters.iter().find(|m| m.id == id))
        .or_else(|| axes::origin(font))
}

/// `check.sources.designspaceLoads`: the document parses, lists sources, and each opens.
pub(crate) fn designspace_loads(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    if font.source.format != SourceFormat::Designspace {
        sink.skip("the source is not a designspace document");
        return;
    }
    let empty = font.masters.is_empty();
    if empty {
        sink.push(Hit::new("no-sources", "The designspace document lists no sources."));
    }
    match ctx.services.check_source_files(font) {
        Ok(issues) => {
            for issue in issues.iter().take(MAX_SOURCE_FILE_ISSUES) {
                let code = match issue.problem {
                    SourceFileProblem::DesignspaceUnreadable => "unreadable",
                    SourceFileProblem::NoSources if empty => continue,
                    SourceFileProblem::NoSources => "no-sources",
                    SourceFileProblem::SourceUnreadable => "source-unreadable",
                    SourceFileProblem::UfoStructure => continue,
                };
                sink.push(Hit::new(code, format!("`{}`: {}", issue.file, issue.message)));
            }
        }
        // What the model shows was checked; the files on disk could not be.
        Err(u) if !empty => sink.skip(u.reason),
        Err(_) => {}
    }
}

/// `check.sources.ufoStructure`: the UFO files pass a strict structural reading.
pub(crate) fn ufo_structure(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    if !matches!(font.source.format, SourceFormat::Ufo | SourceFormat::Ufoz | SourceFormat::Designspace) {
        sink.skip("the source is not a UFO");
        return;
    }
    match ctx.services.check_source_files(font) {
        Err(u) => sink.skip(u.reason),
        Ok(issues) => {
            for issue in issues.iter().filter(|i| i.problem == SourceFileProblem::UfoStructure).take(MAX_SOURCE_FILE_ISSUES) {
                sink.push(Hit::new("invalid-ufo", format!("`{}`: {}", issue.file, issue.message)));
            }
        }
    }
}

/// `check.sources.defaultMaster`: some master sits at every axis's default.
pub(crate) fn default_master(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    if font.axes.is_empty() || font.masters.is_empty() {
        return;
    }
    // Per axis: the default in user coordinates, and in design coordinates through the axis map.
    let defaults: Vec<(f64, f64)> = (0..font.axes.len())
        .map(|i| {
            let user = axes::range(font, i).default;
            (user, axes::piecewise_linear_map(user, &axes::mapping(font, i).pairs))
        })
        .collect();
    let at_default =
        |m: &Master| defaults.iter().enumerate().all(|(i, (_, design))| (axes::design_value(&m.location, i) - design).abs() <= LOCATION_TOLERANCE);
    if font.masters.iter().any(at_default) {
        return;
    }
    let location: Vec<String> = font.axes.iter().zip(&defaults).map(|(a, (user, _))| format!("{} {}", a.tag, num(*user))).collect();
    let message = format!("No master sits at the default location ({}), so the font has no default master.", location.join(", "));
    sink.push(Hit::new("not-found", message));
}

/// The masters that have a master layer of `glyph`.
fn has_master_layer(glyph: &fontcraft_model::Glyph, master: &str) -> bool {
    glyph.layers.iter().any(|l| l.is_master_layer() && l.master == master)
}

/// `check.sources.consistentGlyphSet`: no other master has a glyph the default master lacks.
pub(crate) fn consistent_glyph_set(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    if font.masters.len() < 2 {
        return;
    }
    let Some(default) = find_default_master(font) else { return };
    for g in font.glyphs.as_arcs() {
        if has_master_layer(g, &default.id) {
            continue;
        }
        let others: Vec<&str> = font.masters.iter().filter(|m| m.id != default.id && has_master_layer(g, &m.id)).map(|m| m.name.as_str()).collect();
        if others.is_empty() {
            continue;
        }
        let noun = if others.len() == 1 { "master" } else { "masters" };
        let message = format!("Glyph {} is in {noun} {} but not in the default master {}.", g.name, list(&others), default.name);
        let place = Location { glyph: g.name.clone(), master: Some(default.id.clone()), ..Location::default() };
        sink.push(Hit::new("inconsistent-glyphset", message).place(place));
    }
}

/// A code point list kept in a layer's data; `None` when a value is not a code point.
fn codepoints(v: &Value) -> Option<Vec<u32>> {
    v.as_array()?
        .items
        .iter()
        .map(|x| {
            let f = x.as_num().map(Num::as_f64)?;
            (f.fract() == 0.0 && (0.0..=1_114_111.0).contains(&f)).then_some(f as u32)
        })
        .collect()
}

fn codepoints_text(u: &[u32]) -> String {
    if u.is_empty() {
        return "no code point".into();
    }
    u.iter().map(|c| format!("U+{c:04X}")).collect::<Vec<_>>().join(", ")
}

/// `check.sources.consistentCodepoints`: every master gives a glyph the default master's code
/// points. The designspace reader keeps a master's own list on its master layer where it differs
/// from the glyph's; the list counts only while the glyph's code points are the ones read.
pub(crate) fn consistent_codepoints(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    let Some(default) = find_default_master(font) else { return };
    for g in font.glyphs.as_arcs() {
        // A glyph the default master lacks is consistentGlyphSet's finding.
        if !has_master_layer(g, &default.id) {
            continue;
        }
        for l in g.layers.iter().filter(|l| l.is_master_layer() && l.master != default.id) {
            let Some(own) = l.extra.get(Family::Ufo, "unicodes").and_then(codepoints) else { continue };
            let read = l.extra.get(Family::Ufo, "unicodesRead").and_then(codepoints);
            if read.is_some_and(|r| r != g.unicodes) || own == g.unicodes {
                continue;
            }
            let name = font.masters.iter().find(|m| m.id == l.master).map_or(l.master.as_str(), |m| m.name.as_str());
            let message = format!(
                "Glyph {} has {} in master {name} but {} in the default master {}.",
                g.name,
                codepoints_text(&own),
                codepoints_text(&g.unicodes),
                default.name
            );
            let place = Location { glyph: g.name.clone(), master: Some(l.master.clone()), ..Location::default() };
            sink.push(Hit::new("inconsistent-codepoints", message).place(place));
        }
    }
}

/// Does `master` state a value for the plain metric `kind`? A metric no master states has no
/// definition; one a master leaves out at the end of its list is absent from that master.
fn has_metric(font: &Font, master: &Master, kind: &str) -> bool {
    font.metrics.iter().position(|d| d.kind == kind && d.name.is_none() && d.filter.is_none()).is_some_and(|i| i < master.metric_values.len())
}

/// `check.sources.requiredInfoFields`: units per em, the family name and each master's vertical
/// metrics are set.
pub(crate) fn required_info_fields(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    let mut missing = Vec::new();
    if font.units_per_em == 0 {
        missing.push("units per em");
    }
    if font.family_name.trim().is_empty() {
        missing.push("a family name");
    }
    if !missing.is_empty() {
        sink.push(Hit::new("missing-required-fields", format!("The font does not set {}.", list(&missing))));
    }
    for m in font.masters.iter() {
        let missing: Vec<&str> = REQUIRED_METRICS.iter().filter(|(kind, _)| !has_metric(font, m, kind)).map(|(_, label)| *label).collect();
        if !missing.is_empty() {
            let message = format!("Master {} does not set its {}.", m.name, list(&missing));
            sink.push(Hit::new("missing-required-fields", message).place(master_place(m)));
        }
    }
}

fn has_param(params: &InfoParams, names: &[&str]) -> bool {
    names.iter().any(|n| params.get(n).is_some())
}

fn has_text(properties: &Properties, keys: &[&str]) -> bool {
    keys.iter().any(|k| properties.value(k).is_some_and(|v| !v.trim().is_empty()))
}

/// `check.sources.recommendedInfoFields`: the copyright and each master's style name and PANOSE
/// values are set. The underline values and the version always have a value in the model (the
/// reference's defaults), so they are never missing here.
pub(crate) fn recommended_info_fields(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    if !has_text(&font.info.properties, &["copyrights", "copyright"]) {
        sink.push(Hit::new("missing-recommended-fields", "The font sets no copyright."));
    }
    let font_panose = has_param(&font.info.params, &PANOSE);
    for m in font.masters.iter() {
        let mut missing = Vec::new();
        if m.name.trim().is_empty() {
            missing.push("a style name");
        }
        if !font_panose && !has_param(&m.params, &PANOSE) {
            missing.push("PANOSE values");
        }
        if !missing.is_empty() {
            let shown = if m.name.trim().is_empty() { m.id.as_str() } else { m.name.as_str() };
            let message = format!("Master {shown} sets no {}.", list(&missing));
            sink.push(Hit::new("missing-recommended-fields", message).place(master_place(m)));
        }
    }
}

fn unnecessary(properties: &Properties, params: &InfoParams) -> Vec<&'static str> {
    let mut out = Vec::new();
    for name in UNNECESSARY_PARAMS {
        if (UNNECESSARY_PROPERTIES.contains(&name) && properties.get(name).is_some()) || params.get(name).is_some() {
            out.push(name);
        }
    }
    out
}

/// `check.sources.unnecessaryInfoFields`: the font and its masters leave out the values the
/// compiler generates (unique ID, version string, PostScript unique ID) and the obsolete year.
pub(crate) fn unnecessary_info_fields(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = ctx.audit.font;
    let found = unnecessary(&font.info.properties, &font.info.params);
    if !found.is_empty() {
        let message = format!("The font info sets {}, which the compiler generates or which is obsolete.", list(&found));
        sink.push(Hit::new("unnecessary-fields", message));
    }
    for m in font.masters.iter() {
        let found = unnecessary(&m.properties, &m.params);
        if !found.is_empty() {
            let message = format!("Master {} sets {}, which the compiler generates or which is obsolete.", m.name, list(&found));
            sink.push(Hit::new("unnecessary-fields", message).place(master_place(m)));
        }
    }
}

#[cfg(test)]
#[path = "sources_tests.rs"]
mod tests;
