//! The layout stage (M8.7): the font's feature code for one target compiled through
//! `fontcraft_fea` with the generated kerning and mark attachment, giving `GSUB`, `GPOS`,
//! `GDEF`, `BASE` and the table-block values (`docs/plans/M8.md` § 4, M8.7 and M8.8).
//!
//! Rows: the `GSUB`, `GPOS` and `GDEF` rows of `spec/inventory/40-tables.toml`,
//! `compile.featureWriterModes`, `compile.skipFeatures`, `compile.kernQuantisation`,
//! `implicit.markerMissing`, `implicit.markZeroWidth`, `Keep Kerning in one Lookup`,
//! `Use Extension Kerning`.
//!
//! The stage runs after the glyph order, as fontc runs it:
//!
//! 1. The entries: the automatic ones the font has regenerated for the target's glyph set
//!    (`fontcraft_features::auto::update`), unless the target's Update Features parameter is
//!    off; Update adds no entry here. Then the instance parameters (Add, Replace and Remove
//!    Classes, Prefixes and Features).
//! 2. Tokens, lowering and assembly into one feature file (`fontcraft_features::assemble`).
//! 3. The first pass (`fontcraft_fea::first_pass`): `GSUB` alone, the code's language systems
//!    and the features it declares with or without an `# Automatic Code` marker.
//! 4. The implicit features (`fontcraft_features::implicit`): the kern writer with the code's
//!    language systems and its `GSUB` closure, the mark writer registered for the language
//!    systems. A generated feature the code declares without a marker is not generated
//!    (`implicit.markerMissing`); a hand-written `kern` entry gets the marker at its end when
//!    assembled.
//! 5. The compile (`fontcraft_fea::compile`), which merges the generated lookups at the markers
//!    or after the code's lookups.
//!
//! A compile error in steps 2, 3 or 5 stops the export with the diagnostics mapped to the
//! entries ([`CompileError::Features`]), as fontc stops. A writer that reaches one of its caps is
//! left out with a warning. M8.8 applies the table-block values after the table builders and
//! moves the code's name ids above the source's.

use std::path::Path;

use fontcraft_fea::{CompileRequest, Compiled as FeaCompiled, Diagnostic, GlyphOrder, Severity};
use fontcraft_features::implicit::{ImplicitRequest, generated};
use fontcraft_features::marks::{MarkLimits, MarkRequest};
use fontcraft_features::{EntryDiagnostic, EntryKind, FEATURE_WRITERS_KEY, KernRequest, MasterValues, SourceMap, Target, append_tags};
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Features, Font, Resolved, ResolvedGlyph, is_ufo_family, param_flag};
use fontcraft_spacing::kerning::{Direction, KernIndex};

use crate::CompileError;
use crate::export_params::ExportEdits;

/// The parameter that turns the regeneration of automatic feature code off.
const UPDATE_FEATURES: &str = "Update Features";

/// How the layout stage runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutOptions {
    /// Regenerate the automatic entries for the target's glyph set before compiling (the
    /// handbook's behaviour; the Update Features parameter turns it off).
    pub regenerate_automatic: bool,
    /// Compile `GPOS`; `false` for the Edit View preview font, which shapes with substitutions
    /// only.
    pub gpos: bool,
    /// Add the generated kerning and mark attachment.
    pub implicit: bool,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self { regenerate_automatic: true, gpos: true, implicit: true }
    }
}

/// The layout stage's result.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutTables {
    /// The tables as bytes and the table-block values.
    pub compiled: FeaCompiled,
    /// The compiler's warnings, mapped to the entries.
    pub diagnostics: Vec<EntryDiagnostic>,
}

impl LayoutTables {
    /// The layout tables to add to the font, by tag.
    pub fn tables(&self) -> Vec<(fontcraft_fea::Tag, &[u8])> {
        let c = &self.compiled;
        [(b"GDEF", &c.gdef), (b"GPOS", &c.gpos), (b"GSUB", &c.gsub)]
            .into_iter()
            .filter_map(|(tag, bytes)| Some((fontcraft_fea::Tag::new(tag), bytes.as_deref()?)))
            .collect()
    }
}

/// Another master of a variable font: its id and its glyphs after the glyph stages, by name.
pub struct OtherMaster<'a> {
    pub id: &'a str,
    pub glyphs: std::collections::HashMap<&'a str, &'a ResolvedGlyph>,
}

/// What a variable font's layout stage reads beyond the origin: the masters and their model,
/// the other masters' glyphs for their anchors, and the alternate layers' feature variations.
pub struct VariableLayout<'a> {
    pub layout: &'a fontcraft_features::var_layout::VarLayout,
    pub masters: Vec<OtherMaster<'a>>,
    pub alternates: Option<&'a crate::var::featvar::AlternateRules>,
}

/// What the layout stage reads.
pub struct LayoutInput<'a> {
    /// The source the target was resolved from (its entries, kerning, anchors and number
    /// values); `None` for a resolved font compiled on its own, which has no feature code.
    pub font: Option<&'a Font>,
    /// The glyph database for token values, Update and the mark writer.
    pub data: Option<&'a GlyphData>,
    pub resolved: &'a Resolved,
    /// The final glyph order.
    pub order: &'a [ResolvedGlyph],
    /// The export parameters' renames, so the kerning finds renamed glyphs.
    pub edits: &'a ExportEdits,
    /// The folder `include` statements are read from (the source's folder).
    pub include_dir: Option<&'a Path>,
    /// Which included files on disk may be read; `None` reads any.
    pub include_check: Option<&'a fontcraft_fea::IncludeCheck>,
    /// The variable font's masters (`resolved` is the origin's): the generated kerning and mark
    /// positions vary with them. `None` for a static font.
    pub variable: Option<&'a VariableLayout<'a>>,
    /// The source's axes and the target's internal location for the variable feature syntax,
    /// when `font` is an interpolated instance font that no longer holds them.
    pub feature_axes: Option<&'a (fontcraft_features::fea_variable::FeatureAxes, Vec<f64>)>,
}

/// The entries compiled for `input`'s target: the automatic ones regenerated (unless off), then
/// the instance parameters applied.
fn entries(input: &LayoutInput, font: &Font, opts: &LayoutOptions, warnings: &mut Vec<String>) -> Result<Features, CompileError> {
    let resolved = input.resolved;
    let mut entries: Features = (*font.features).clone();
    let update_off = resolved.params.get(UPDATE_FEATURES).and_then(param_flag) == Some(false);
    let has_automatic = entries.prefixes.iter().chain(&entries.classes).chain(&entries.features).any(|e| e.automatic);
    if opts.regenerate_automatic
        && !update_off
        && has_automatic
        && !is_ufo_family(resolved.format)
        && let Some(data) = input.data
    {
        let names: Vec<String> = input.order.iter().map(|g| g.name.clone()).collect();
        match fontcraft_features::auto::AutoContext::from_font(font, data, Some(&names))
            .and_then(|cx| fontcraft_features::auto::update(&entries, &cx))
        {
            Ok(report) => entries = refreshed(&entries, report.features),
            Err(e) => warnings.push(format!("the automatic feature code was compiled as stored: it could not be regenerated: {e}")),
        }
    }
    fontcraft_features::params::apply_instance_params(&entries, &resolved.params).map_err(|e| request_error(e.to_string()))
}

/// `updated` without the entries Update added: at export only the entries the font has are
/// regenerated.
fn refreshed(stored: &Features, mut updated: Features) -> Features {
    let keep = |list: &mut Vec<fontcraft_model::Snippet>, stored: &[fontcraft_model::Snippet]| {
        list.retain(|e| stored.iter().any(|s| s.name == e.name));
    };
    keep(&mut updated.prefixes, &stored.prefixes);
    keep(&mut updated.classes, &stored.classes);
    keep(&mut updated.features, &stored.features);
    updated
}

/// A tag's four bytes, padded with spaces.
fn tag_bytes(tag: &str) -> [u8; 4] {
    let mut out = [b' '; 4];
    for (slot, b) in out.iter_mut().zip(tag.bytes()) {
        *slot = b;
    }
    out
}

fn request_error(message: String) -> CompileError {
    CompileError::Features(vec![EntryDiagnostic { place: None, diagnostic: Diagnostic::request(Severity::Error, message) }])
}

/// A warning of the compiler as one line, naming the entry it is in.
fn describe(entries: &Features, d: &EntryDiagnostic) -> String {
    let place = d.place.as_ref().and_then(|p| {
        let (list, kind) = match p.entry.kind {
            EntryKind::Prefix => (&entries.prefixes, "prefix"),
            EntryKind::Class => (&entries.classes, "class"),
            EntryKind::Feature => (&entries.features, "feature"),
        };
        list.get(p.entry.index).map(|e| if e.name.is_empty() { format!(" (in a {kind})") } else { format!(" (in {kind} {})", e.name) })
    });
    format!("feature code: {}{}", d.message(), place.unwrap_or_default())
}

/// Compiles the feature code of `input`'s target with the generated kerning and mark
/// attachment. A source without feature code, kerning or anchors gives no tables.
pub fn build(input: &LayoutInput, opts: &LayoutOptions, warnings: &mut Vec<String>) -> Result<LayoutTables, CompileError> {
    let resolved = input.resolved;
    let names: Vec<String> = input.order.iter().map(|g| g.name.clone()).collect();
    let glyph_order = GlyphOrder::new(names.clone()).map_err(|d| CompileError::Features(vec![EntryDiagnostic { place: None, diagnostic: d }]))?;

    // Steps 1 and 2: the assembled code, when the source has any.
    let mut entries = Features::default();
    let mut assembly = None;
    if let Some(font) = input.font.filter(|f| !f.features.is_empty()) {
        match input.data {
            Some(data) => {
                entries = self::entries(input, font, opts, warnings)?;
                let mut values = MasterValues::new(font, data, &resolved.master).map_err(|e| request_error(e.to_string()))?;
                if let Some((axes, location)) = input.feature_axes {
                    values = values.with_feature_axes(axes.clone(), location.clone());
                }
                let assembled = match fontcraft_features::assemble(font, &entries, &Target::Master(resolved.master.clone()), &values) {
                    Ok(a) => a,
                    Err(fontcraft_features::FeatureError::Code(found)) => return Err(CompileError::Features(found)),
                    Err(e) => return Err(request_error(e.to_string())),
                };
                if !assembled.files.root_text().trim().is_empty() {
                    assembly = Some(assembled);
                }
            }
            None => warnings.push("the feature code was not compiled: the glyph database could not be read".into()),
        }
    }
    let empty_map = SourceMap::new();
    let map = assembly.as_ref().map_or(&empty_map, |a| &a.map);
    // Errors first, so the summary names one.
    let mapped = |mut found: Vec<Diagnostic>| {
        found.sort_by_key(|d| !d.is_error());
        CompileError::Features(found.into_iter().map(|d| map.map(d)).collect())
    };
    let empty_files = fontcraft_fea::VirtualFiles::new(fontcraft_features::assemble::ROOT_FILE, "").map_err(|d| mapped(vec![d]))?;
    let mut files = assembly.as_ref().map_or_else(|| empty_files.clone(), |a| a.files.clone());
    if let Some(dir) = input.include_dir {
        files = files.with_include_dir(dir.to_path_buf());
    }
    if let Some(check) = input.include_check {
        files = files.with_include_check(check.clone());
    }

    // Step 3: what the writers read from the code.
    let mut request = CompileRequest::new(&files, &glyph_order);
    request.gpos = opts.gpos;
    let first = match &assembly {
        Some(_) => fontcraft_fea::first_pass(&request).map_err(mapped)?,
        None => fontcraft_fea::FirstPass::default(),
    };
    // A writer in append mode adds its lookups whatever the code declares.
    let setting = input.font.and_then(|font| {
        font.info.user_data.get(FEATURE_WRITERS_KEY).or_else(|| font.master(&resolved.master).ok()?.user_data.get(FEATURE_WRITERS_KEY))
    });
    let append = append_tags(setting);
    let declared_without_marker: Vec<String> = first.features.iter().filter(|(_, marker)| !marker).map(|(t, _)| t.to_string()).collect();
    let declared: Vec<&str> = declared_without_marker.iter().map(|t| t.trim_end()).filter(|t| !append.contains(t)).collect();

    // Step 4: the implicit features.
    let mut generated_lookups = fontcraft_fea::GeneratedLookups::default();
    if opts.implicit {
        let index = input.font.map(KernIndex::new);
        let sources = input.edits.source_names();
        let kern_glyphs = crate::kern::glyphs(input.order, &sources, first.gdef_classes.as_ref());
        let kern_request = match (input.font, &index) {
            (Some(font), Some(index)) => {
                let has_kerning =
                    [Direction::Ltr, Direction::Rtl].into_iter().any(|d| index.table(d, &resolved.master).is_ok_and(|t| t.pairs().next().is_some()));
                has_kerning.then(|| KernRequest {
                    font,
                    index,
                    master: &resolved.master,
                    glyphs: &kern_glyphs,
                    tags: crate::kern::tags(font, &resolved.master, &declared, warnings),
                    limits: crate::kern::limits(),
                    language_systems: &first.language_systems,
                    gsub: first.gsub.as_deref(),
                    variable: input.variable.map(|v| v.layout),
                })
            }
            _ => None,
        };
        let systems: Vec<(String, String)> = first.language_systems.iter().map(|(s, l)| (s.to_string(), l.to_string())).collect();
        let others = input.variable.map_or(&[][..], |v| v.masters.as_slice());
        let mark_input = crate::marks::input(input.font, input.data, resolved, input.order, systems, others, warnings);
        let any_anchor = mark_input.glyphs.iter().any(|g| g.anchors.iter().any(|m| !m.anchors.is_empty()));
        let mark_tags = match input.font {
            Some(font) => crate::marks::tags(font, &resolved.master, any_anchor, &declared, warnings),
            None => fontcraft_features::marks::MarkTags::default(),
        };
        let mark_request =
            MarkRequest { input: &mark_input, master: &resolved.master, tags: mark_tags, limits: MarkLimits::default(), gsub: first.gsub.as_deref() };
        // A variable font's mark positions: the writer runs for each other master too.
        let other_marks: Vec<MarkRequest> = input
            .variable
            .map(|v| v.layout.masters().iter().skip(1).map(|m| MarkRequest { master: &m.id, ..mark_request }).collect())
            .unwrap_or_default();
        let master_marks = input.variable.map(|v| (v.layout, other_marks.as_slice()));
        let implicit = generated(&ImplicitRequest { kern: kern_request.as_ref(), marks: Some(&mark_request), master_marks });
        warnings.extend(implicit.warnings);
        generated_lookups = implicit.lookups;
        generated_lookups.axes = input.variable.map(|v| v.layout.axes().to_vec()).unwrap_or_default();
        generated_lookups.append = append.iter().map(|t| fontcraft_fea::Tag::new(&tag_bytes(t))).collect();
    }
    // The alternate layers' substitutions, merged into the code's `GSUB` by fea-rs.
    if let Some(variable) = input.variable {
        if let Some(alternates) = variable.alternates {
            generated_lookups.feature_variations = alternates
                .variations(&names)?
                .into_iter()
                .map(|v| fontcraft_fea::FeatureVariation {
                    tag: v.tag,
                    conditions: v.conditions.into_iter().map(|(axis, min, max)| fontcraft_fea::Condition { axis, min, max }).collect(),
                    substitutions: v.substitutions,
                })
                .collect();
        }
        if generated_lookups.axes.is_empty() {
            generated_lookups.axes = variable.layout.axes().to_vec();
        }
    }
    if assembly.is_none() && generated_lookups.is_empty() {
        return Ok(LayoutTables::default());
    }

    // Step 5: the compile.
    request.generated = (!generated_lookups.is_empty()).then_some(&generated_lookups);
    let compiled = fontcraft_fea::compile(&request).map_err(mapped)?;
    let diagnostics: Vec<EntryDiagnostic> = compiled.warnings.iter().map(|d| map.map(d.clone())).collect();
    warnings.extend(diagnostics.iter().map(|d| describe(&entries, d)));
    Ok(LayoutTables { compiled, diagnostics })
}
