//! Automatic feature code (M8.9; `docs/spec/12-feature-code.md` § 8): the prefixes, classes and
//! features Update writes from the glyph names, and the merge rules that decide what Update
//! keeps.
//!
//! Rows: `autoFeature.generalRule`, `autoFeature.languageSystems`, `autoClass.general`,
//! `fontInfo.features.updateDetectsNew`, `.optionalAutomatic`, `.manualNotUpdated`,
//! `glyphName.chainedSuffixes` (ordering), `Update Features`; each generator module serves the
//! `autoFeature.<tag>` rows of its tags.
//!
//! Contract:
//! - A [`Generator`] reads an [`AutoContext`] (the exporting glyphs in order, with their
//!   categories, scripts and code points, and the existing entries) and returns the entries it
//!   would write ([`AutoEntry`]), each with code of at most [`MAX_GENERATED_BYTES`]. It never
//!   reads the font directly, so the same generator serves Update and each export target.
//! - Each generator module has a `generators()` function listing its generators;
//!   [`generators`] joins them. Package agents fill their own module and leave this file alone.
//! - [`update`] applies the merge rules: manual entries kept; automatic entries replaced; new
//!   entries added in the reference's tag order; optional entries written only when present.

mod aalt;
mod ccmp;
mod classes;
mod cpsp;
mod figures;
mod fractions;
mod indic;
mod indic_positional;
mod langsys;
mod ligatures;
mod locl;
mod suffix;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Features, Font, Glyph, Snippet, glyph_props};

use crate::FeatureError;
use crate::source_map::{EntryKind, EntryRef};

pub use locl::locl_languages;

/// Most bytes of code one generated entry may hold.
pub const MAX_GENERATED_BYTES: usize = 4 << 20;

/// Most anchors read from one glyph into an [`AutoShape`].
pub const MAX_AUTO_ANCHORS: usize = 256;

/// One glyph as the generators see it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AutoGlyph {
    pub name: String,
    pub codepoints: Vec<u32>,
    /// The effective category and subcategory (`Letter`, `Lowercase`), from the glyph or the
    /// glyph database.
    pub category: Option<String>,
    pub sub_category: Option<String>,
    /// The effective script (`latin`, `devanagari`).
    pub script: Option<String>,
}

/// An anchor of an [`AutoShape`]: its name and horizontal position.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AutoAnchor {
    pub name: String,
    pub x: f64,
}

/// What the positional Indic generators read beyond a glyph's name: its production name and,
/// from the first master, its advance width and anchors. Widths and positions are finite.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AutoShape {
    /// The production name (`uni0C950CBF`), from the glyph or the glyph database.
    pub production: Option<String>,
    pub width: f64,
    /// At most [`MAX_AUTO_ANCHORS`].
    pub anchors: Vec<AutoAnchor>,
}

impl AutoShape {
    /// The horizontal position of the anchor `name`.
    pub fn anchor(&self, name: &str) -> Option<f64> {
        self.anchors.iter().find(|a| a.name == name).map(|a| a.x)
    }

    /// The shape with non-finite numbers replaced by 0 and at most [`MAX_AUTO_ANCHORS`] anchors.
    fn checked(mut self) -> Self {
        if !self.width.is_finite() {
            self.width = 0.0;
        }
        self.anchors.truncate(MAX_AUTO_ANCHORS);
        for a in &mut self.anchors {
            if !a.x.is_finite() {
                a.x = 0.0;
            }
        }
        self
    }
}

/// What the generators read: the target's exporting glyphs in the font's order and the entries
/// the font has.
#[derive(Clone, Debug, Default)]
pub struct AutoContext {
    glyphs: Vec<AutoGlyph>,
    index: HashMap<String, usize>,
    existing: Arc<Features>,
    shapes: HashMap<String, AutoShape>,
}

impl AutoContext {
    /// Refused: more than `fontcraft_model::MAX_GLYPHS` glyphs.
    pub fn new(glyphs: Vec<AutoGlyph>, existing: Arc<Features>) -> Result<Self, FeatureError> {
        if glyphs.len() > fontcraft_model::MAX_GLYPHS {
            return Err(FeatureError::Limit { what: "glyphs", limit: fontcraft_model::MAX_GLYPHS });
        }
        let index = glyphs.iter().enumerate().map(|(i, g)| (g.name.clone(), i)).collect();
        Ok(Self { glyphs, index, existing, shapes: HashMap::new() })
    }

    /// The context with these production names, widths and anchors; shapes of glyphs it lacks
    /// are dropped.
    pub fn with_shapes(mut self, shapes: impl IntoIterator<Item = (String, AutoShape)>) -> Self {
        for (name, shape) in shapes {
            if self.index.contains_key(&name) {
                self.shapes.insert(name, shape.checked());
            }
        }
        self
    }

    /// The context of `font`'s exporting glyphs (or of `glyphs`, an export target's glyph set,
    /// when given), with properties from `data`.
    /// A glyph without a subcategory takes one from its case (`upper` gives `Uppercase`), as the
    /// glyph database of the reference's later versions gives letters a case and no subcategory.
    pub fn from_font(font: &Font, data: &GlyphData, glyphs: Option<&[String]>) -> Result<Self, FeatureError> {
        let chosen: Vec<&Glyph> = match glyphs {
            Some(names) => {
                if names.len() > fontcraft_model::MAX_GLYPHS {
                    return Err(FeatureError::Limit { what: "glyphs", limit: fontcraft_model::MAX_GLYPHS });
                }
                let mut seen = HashSet::new();
                names.iter().filter(|n| seen.insert(n.as_str())).filter_map(|n| font.glyphs.get(n)).map(|g| &**g).collect()
            }
            None => font.glyphs.iter().filter(|g| g.export).map(|g| &**g).collect(),
        };
        let master = font.masters.first().map(|m| m.id.as_str());
        let mut out = Vec::with_capacity(chosen.len());
        let mut shapes = Vec::with_capacity(chosen.len());
        for glyph in chosen {
            let props = glyph_props(glyph, data, false);
            let layer = master.and_then(|m| glyph.layer(m));
            shapes.push((
                glyph.name.clone(),
                AutoShape {
                    production: props.production_name.value.clone(),
                    width: layer.map_or(0.0, |l| l.width),
                    anchors: layer
                        .map(|l| l.anchors.iter().take(MAX_AUTO_ANCHORS).map(|a| AutoAnchor { name: a.name.clone(), x: a.x }).collect())
                        .unwrap_or_default(),
                },
            ));
            let sub_category = props.sub_category.value.or_else(|| {
                let sub = match props.case.as_deref()? {
                    "upper" => "Uppercase",
                    "lower" => "Lowercase",
                    "smallCaps" => "Smallcaps",
                    _ => return None,
                };
                Some(sub.to_string())
            });
            out.push(AutoGlyph {
                name: glyph.name.clone(),
                codepoints: glyph.unicodes.clone(),
                category: props.category.value,
                sub_category,
                script: props.script.value,
            });
        }
        Ok(Self::new(out, font.features.clone())?.with_shapes(shapes))
    }

    pub fn glyphs(&self) -> &[AutoGlyph] {
        &self.glyphs
    }

    pub fn glyph(&self, name: &str) -> Option<&AutoGlyph> {
        self.index.get(name).and_then(|&i| self.glyphs.get(i))
    }

    pub fn has(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    pub fn existing(&self) -> &Features {
        &self.existing
    }

    /// The production name, width and anchors of `name`, when known.
    pub fn shape(&self, name: &str) -> Option<&AutoShape> {
        self.shapes.get(name)
    }
}

/// An entry a generator writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoEntry {
    pub kind: EntryKind,
    /// The feature tag, or the class or prefix name.
    pub name: String,
    pub code: String,
    /// Written by Update only when the font already has the entry (`fontInfo.features.optionalAutomatic`).
    pub optional: bool,
}

/// One generator of automatic code.
pub trait Generator: Sync {
    /// The entry names (feature tags, class or prefix names) it may write, in the reference's
    /// order.
    fn names(&self) -> &'static [&'static str];

    /// The entries it writes for `cx`; none when the glyphs do not call for them.
    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError>;
}

/// Every generator, in the order Update runs them.
pub fn generators() -> Vec<&'static dyn Generator> {
    let mut all = Vec::new();
    all.extend(langsys::generators());
    all.extend(classes::generators());
    all.extend(aalt::generators());
    all.extend(ccmp::generators());
    all.extend(locl::generators());
    all.extend(suffix::generators());
    all.extend(figures::generators());
    all.extend(fractions::generators());
    all.extend(ligatures::generators());
    all.extend(indic::generators());
    all.extend(indic_positional::generators());
    all.extend(cpsp::generators());
    all
}

/// What [`update`] did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpdateReport {
    /// The entries after the update.
    pub features: Features,
    /// Entries added, as (kind, name).
    pub added: Vec<(EntryKind, String)>,
    /// Automatic entries whose code changed.
    pub updated: Vec<(EntryKind, String)>,
    /// Automatic entries removed because their generator no longer writes them (the glyphs that
    /// called for them are gone).
    pub removed: Vec<(EntryKind, String)>,
    /// Entries Update recognised but does not generate, with the reason.
    pub not_generated: Vec<(String, String)>,
}

/// The entries computed from Update's merged list rather than from the glyphs alone.
const DERIVED: [&str; 2] = ["aalt", langsys::NAME];

/// The order in which Update places the features it adds. `ssXX` and `cvXX` stand for the
/// numbered sets, in number order.
const FEATURE_ORDER: &[&str] = &[
    "aalt", "ccmp", "locl", "nukt", "akhn", "rphf", "rkrf", "pref", "blwf", "half", "vatu", "cjct", "pres", "abvs", "blws", "psts", "subs", "sinf",
    "sups", "numr", "dnom", "frac", "afrc", "ordn", "lnum", "pnum", "tnum", "onum", "case", "init", "medi", "med2", "fina", "fin2", "fin3", "dlig",
    "liga", "hlig", "rlig", "c2sc", "smcp", "c2pc", "pcap", "hist", "titl", "swsh", "ornm", "salt", "ssXX", "cvXX", "ital", "fwid", "hwid", "pwid",
    "qwid", "twid", "hkna", "vkna", "pkna", "hojo", "ruby", "vrt2", "cpsp",
];

/// Where `name` sorts among the entries Update adds; names outside the order sort last, in the
/// order generated.
fn rank(kind: EntryKind, name: &str) -> usize {
    if kind != EntryKind::Feature {
        return 0;
    }
    let numbered = |prefix: &str| {
        let n: usize = name.strip_prefix(prefix).filter(|d| d.len() == 2 && d.bytes().all(|b| b.is_ascii_digit()))?.parse().ok()?;
        let base = FEATURE_ORDER.iter().position(|t| t.get(..2) == Some(prefix) && t.ends_with("XX"))?;
        Some(base * 100 + n)
    };
    numbered("ss").or_else(|| numbered("cv")).or_else(|| FEATURE_ORDER.iter().position(|t| *t == name).map(|i| i * 100)).unwrap_or(usize::MAX)
}

fn list_mut(features: &mut Features, kind: EntryKind) -> &mut Vec<Snippet> {
    match kind {
        EntryKind::Prefix => &mut features.prefixes,
        EntryKind::Class => &mut features.classes,
        EntryKind::Feature => &mut features.features,
    }
}

fn list(features: &Features, kind: EntryKind) -> &[Snippet] {
    match kind {
        EntryKind::Prefix => &features.prefixes,
        EntryKind::Class => &features.classes,
        EntryKind::Feature => &features.features,
    }
}

/// Every name some generator may write.
fn known_names() -> HashSet<&'static str> {
    generators().iter().flat_map(|g| g.names().iter().copied()).collect()
}

/// Puts `entry` into `entries` by Update's rules for one entry: an automatic entry of that name
/// takes its code (all of them, when the name is repeated); a new entry goes before the first
/// entry that sorts after it, or last; aalt and the Languagesystems prefix go first.
fn merge_one(entries: &mut Features, entry: AutoEntry, report: &mut UpdateReport) {
    let kind = entry.kind;
    let snippets = list_mut(entries, kind);
    if snippets.iter().any(|s| s.name == entry.name) {
        for s in snippets.iter_mut().filter(|s| s.name == entry.name && s.automatic) {
            if s.code != entry.code {
                s.code.clone_from(&entry.code);
                if !report.updated.iter().any(|(k, n)| *k == kind && *n == entry.name) {
                    report.updated.push((kind, entry.name.clone()));
                }
            }
        }
        return;
    }
    if entry.optional {
        return;
    }
    let r = rank(kind, &entry.name);
    let at = if kind == EntryKind::Feature && entry.name != "aalt" {
        snippets.iter().position(|s| rank(kind, &s.name) != usize::MAX && rank(kind, &s.name) > r).unwrap_or(snippets.len())
    } else if entry.name == "aalt" || (kind == EntryKind::Prefix && entry.name == langsys::NAME) {
        0
    } else {
        snippets.len()
    };
    report.added.push((kind, entry.name.clone()));
    snippets.insert(at, Snippet { name: entry.name, code: entry.code, automatic: true, ..Snippet::default() });
}

/// Update: every generator run on `cx`, merged into `entries`:
/// - manual entries are kept as they are, and so are automatic entries no generator knows;
/// - an automatic entry takes the code its generator writes now, and is removed when the
///   generator writes nothing (the glyphs that called for it are gone);
/// - an entry the font lacks is added in [`FEATURE_ORDER`], unless it is optional (the classes,
///   cpsp), which Update only refreshes;
/// - aalt and the Languagesystems prefix are computed last, from the merged features.
pub fn update(entries: &Features, cx: &AutoContext) -> Result<UpdateReport, FeatureError> {
    let mut generated = Vec::new();
    for generator in generators() {
        if generator.names().iter().any(|n| DERIVED.contains(n)) {
            continue;
        }
        generated.extend(generator.generate(cx)?);
    }
    let known = known_names();
    let mut report = UpdateReport::default();
    let mut merged = entries.clone();
    for kind in [EntryKind::Prefix, EntryKind::Class, EntryKind::Feature] {
        list_mut(&mut merged, kind).retain(|s| {
            let stays = !s.automatic
                || !known.contains(s.name.as_str())
                || DERIVED.contains(&s.name.as_str())
                || generated.iter().any(|e| e.kind == kind && e.name == s.name);
            if !stays && !report.removed.iter().any(|(k, n)| *k == kind && *n == s.name) {
                report.removed.push((kind, s.name.clone()));
            }
            stays
        });
    }
    for entry in generated {
        merge_one(&mut merged, entry, &mut report);
    }

    let features = merged.features.iter().map(|s| (s.name.as_str(), s.code.as_str(), s.disabled));
    let aalt = aalt::code_for(features)?;
    let aalt_manual = merged.features.iter().any(|s| s.name == "aalt" && !s.automatic);
    match aalt {
        Some(code) => merge_one(&mut merged, AutoEntry { kind: EntryKind::Feature, name: "aalt".into(), code, optional: false }, &mut report),
        None if !aalt_manual && merged.features.iter().any(|s| s.name == "aalt") => {
            merged.features.retain(|s| s.name != "aalt");
            report.removed.push((EntryKind::Feature, "aalt".into()));
        }
        None => {}
    }
    let code = langsys::code_for(&merged, cx.glyphs());
    merge_one(&mut merged, AutoEntry { kind: EntryKind::Prefix, name: langsys::NAME.into(), code, optional: false }, &mut report);

    report.features = merged;
    Ok(report)
}

/// Update Entry: one automatic entry regenerated, the rest left as they are. An entry whose
/// generator no longer writes it keeps its place with empty code. Refused: a manual entry, or
/// one no generator writes.
pub fn update_entry(entries: &Features, entry: EntryRef, cx: &AutoContext) -> Result<Features, FeatureError> {
    let snippet = list(entries, entry.kind).get(entry.index).ok_or_else(|| FeatureError::Unsupported("there is no such entry".to_string()))?;
    if !snippet.automatic {
        return Err(FeatureError::Unsupported(format!("`{}` is written by hand; Update leaves it alone", snippet.name)));
    }
    if !known_names().contains(snippet.name.as_str()) {
        return Err(FeatureError::Unsupported(format!("no automatic code is written for `{}`", snippet.name)));
    }
    let report = update(entries, cx)?;
    let code = list(&report.features, entry.kind).iter().find(|s| s.name == snippet.name).map(|s| s.code.clone()).unwrap_or_default();
    let mut out = entries.clone();
    if let Some(s) = list_mut(&mut out, entry.kind).get_mut(entry.index) {
        s.code = code;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(name: &str, sub: Option<&str>) -> AutoGlyph {
        let category = sub.map(|_| "Letter".to_string());
        AutoGlyph { name: name.into(), category, sub_category: sub.map(Into::into), ..AutoGlyph::default() }
    }

    fn entry(name: &str, code: &str, automatic: bool) -> Snippet {
        Snippet { name: name.into(), code: code.into(), automatic, ..Snippet::default() }
    }

    fn names(list: &[Snippet]) -> Vec<&str> {
        list.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn update_keeps_manual_entries_refreshes_automatic_ones_and_adds_new_ones_in_order() {
        let glyphs = ["A", "a", "b", "f", "a.sc", "b.sc", "f_f"]
            .iter()
            .map(|n| glyph(n, n.chars().next().filter(|c| c.is_ascii_uppercase()).map(|_| "Uppercase").or(Some("Lowercase"))))
            .collect();
        let entries = Features {
            prefixes: Vec::new(),
            classes: vec![entry("Uppercase", "B\n", true)],
            features: vec![
                entry("calt", "sub a' b by b;", false),
                entry("smcp", "sub a by a.sc;", true),
                entry("ss05", "sub a by a.ss05;", true),
                entry("case", "# by hand", false),
            ],
        };
        let cx = AutoContext::new(glyphs, Arc::new(entries.clone())).unwrap();
        let report = update(&entries, &cx).unwrap();
        let f = &report.features;
        assert_eq!(names(&f.features), ["aalt", "calt", "liga", "c2sc", "smcp", "case"]);
        assert_eq!(f.features.get(4).map(|s| s.code.as_str()), Some("sub a by a.sc;\nsub b by b.sc;\n"));
        assert_eq!(f.features.get(5).map(|s| s.code.as_str()), Some("# by hand"), "manual code is left alone");
        assert_eq!(f.features.first().map(|s| s.code.as_str()), Some("feature calt;\nfeature c2sc;\nfeature smcp;\n"));
        assert_eq!(names(&f.classes), ["Uppercase"], "optional classes are refreshed, never added");
        assert_eq!(f.classes.first().map(|s| s.code.as_str()), Some("A\n"));
        assert_eq!(names(&f.prefixes), ["Languagesystems"]);
        assert_eq!(f.prefixes.first().map(|s| s.code.as_str()), Some("languagesystem DFLT dflt;\n"));
        assert_eq!(report.removed, [(EntryKind::Feature, "ss05".to_string())]);
        assert!(report.added.contains(&(EntryKind::Feature, "liga".to_string())), "{:?}", report.added);
        assert!(report.updated.contains(&(EntryKind::Feature, "smcp".to_string())));
        assert!(report.not_generated.is_empty());

        // Update Entry: one entry, nothing else.
        let one = update_entry(&entries, EntryRef { kind: EntryKind::Feature, index: 1 }, &cx).unwrap();
        assert_eq!(names(&one.features), names(&entries.features));
        assert_eq!(one.features.get(1).map(|s| s.code.as_str()), Some("sub a by a.sc;\nsub b by b.sc;\n"));
        assert!(update_entry(&entries, EntryRef { kind: EntryKind::Feature, index: 0 }, &cx).is_err(), "manual");
        assert!(update_entry(&entries, EntryRef { kind: EntryKind::Feature, index: 9 }, &cx).is_err(), "no such entry");
        // Again: nothing changes.
        let again = update(&report.features, &cx).unwrap();
        assert_eq!(again.features, report.features);
        assert!(again.added.is_empty() && again.updated.is_empty() && again.removed.is_empty());
    }
}
