//! One feature file per target, from the entries (M8.3).
//!
//! Rows: `featureEditor.outputOrder`, `featureEditor.featureBody`, `featureEditor.classContent`,
//! `featureEditor.afdkoSyntax`, `featureCode.mm.singleSource`, `fontInfo.features.stylisticSetNames`
//! (the `featureNames` output), `kerning.groupClassNames`, `kerning.explicitKernFeature`,
//! `fea.featureNames`, `font.features.expandCode`'s data path.
//!
//! The pipeline for one entry, each pass returning its [`OffsetMap`]:
//! [`crate::tokens::expand`] → [`crate::lower::lower`] (`#ifdef VARIABLE` and the syntax
//! extensions) → [`crate::variable::lower`] (variable target only). Each entry becomes one
//! virtual file holding its full statement (`@Name = [ … ];`, `feature tag { … } tag;`,
//! a prefix's code as it is), and the root file includes them in the oracles' order: classes,
//! prefixes, features, each group in sidebar order. Disabled entries are left out. Kerning
//! groups the code refers to are defined as `@MMK_L_`/`@MMK_R_` classes before the first entry.
//!
//! The text around each entry's code is glyphsLib's, so [`Assembly::expanded_text`] equals the
//! `features.fea` glyphsLib writes for the same entries (with disabled entries left out), with
//! these differences:
//!
//! - feature notes are not written (fontc leaves them out too);
//! - a label in a language without a Windows language id is left out (as fontc does; glyphsLib
//!   stops with an error);
//! - the kerning-group classes the code uses are defined first; the reference compilers leave
//!   them undefined.
//!
//! The text around each entry follows glyphsLib's feature builder (`builder/features.py`,
//! Apache-2.0).

use std::collections::{BTreeMap, BTreeSet, HashSet};

use fontcraft_fea::{Diagnostic, Limits, MAX_DIAGNOSTICS, MAX_INCLUDE_FILES, Severity, VirtualFiles};
use fontcraft_model::{DEFAULT_LANGUAGE, Features, Font, Localized, Snippet, language_id};

use crate::source_map::{EntryDiagnostic, EntryKind, EntryRange, EntryRef, OffsetMap, SourceMap};
use crate::{FeatureError, MAX_ENTRIES, MAX_ENTRY_BYTES, Target, TargetValues};

/// The path of the root file of every assembly.
pub const ROOT_FILE: &str = "features.fea";
/// The virtual file that defines the kerning-group classes the code uses.
pub const KERNING_GROUPS_FILE: &str = "entries/kerning-groups.fea";
/// The comment that marks where the generated kerning goes in a hand-written `kern` feature.
pub const AUTOMATIC_CODE_MARKER: &str = "# Automatic Code";
/// glyphsLib's name for a prefix without a name; no `# Prefix:` comment is written for it.
const ANONYMOUS_PREFIX: &str = "<anonymous>";

/// The assembled feature file of one target.
#[derive(Clone, Debug, PartialEq)]
pub struct Assembly {
    pub files: VirtualFiles,
    pub map: SourceMap,
}

impl Assembly {
    /// Every file's text joined in include order, includes replaced by the files they name:
    /// what `font.features.expandCode` shows. Includes of files outside the assembly stay.
    pub fn expanded_text(&self) -> String {
        let lines: Vec<&str> = self
            .files
            .root_text()
            .split('\n')
            .map(|line| line.strip_prefix("include(").and_then(|l| l.strip_suffix(");")).and_then(|path| self.files.get(path)).unwrap_or(line))
            .collect();
        lines.join("\n")
    }
}

/// The virtual file that holds `entry` in every assembly.
pub fn entry_path(entry: EntryRef) -> String {
    let kind = match entry.kind {
        EntryKind::Prefix => "prefix",
        EntryKind::Class => "class",
        EntryKind::Feature => "feature",
    };
    format!("entries/{kind}-{}.fea", entry.index)
}

/// Assembles `entries` (the font's own, or the result of Update and the instance parameters)
/// for `target`. `font` supplies the kerning groups and the stylistic set names' context;
/// `values` the token values. Diagnostics of the passes come back as [`FeatureError::Code`],
/// mapped to the entries.
pub fn assemble(font: &Font, entries: &Features, target: &Target, values: &dyn TargetValues) -> Result<Assembly, FeatureError> {
    let count = entries.prefixes.len().saturating_add(entries.classes.len()).saturating_add(entries.features.len());
    if count > MAX_ENTRIES {
        return Err(FeatureError::Limit { what: "prefixes, classes and features", limit: MAX_ENTRIES });
    }
    let groups = [(EntryKind::Class, &entries.classes), (EntryKind::Prefix, &entries.prefixes), (EntryKind::Feature, &entries.features)];
    let mut pieces: Vec<(EntryRef, String, OffsetMap)> = Vec::new();
    let mut diagnostics: Vec<EntryDiagnostic> = Vec::new();
    for (kind, list) in groups {
        for (index, entry) in list.iter().enumerate() {
            if entry.disabled {
                continue;
            }
            if entry.code.len() > MAX_ENTRY_BYTES {
                return Err(FeatureError::Limit { what: "bytes of code in one entry", limit: MAX_ENTRY_BYTES });
            }
            let entry_ref = EntryRef { kind, index };
            match run_passes(kind, entry, target, values) {
                Ok((text, map)) => pieces.push((entry_ref, text, map)),
                Err((found, map)) => {
                    for diagnostic in found {
                        if diagnostics.len() >= MAX_DIAGNOSTICS {
                            break;
                        }
                        let range = map.source_range(diagnostic.span.clone());
                        diagnostics.push(EntryDiagnostic { place: Some(EntryRange { entry: entry_ref, range }), diagnostic });
                    }
                }
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(FeatureError::Code(diagnostics));
    }

    let groups_text = kerning_groups(font, entries, &pieces, values);
    // The root file and the kerning groups' file take two of the files a compile may read.
    if pieces.len() > MAX_INCLUDE_FILES.saturating_sub(2) {
        return Err(FeatureError::Limit { what: "prefixes, classes and features to compile", limit: MAX_INCLUDE_FILES.saturating_sub(2) });
    }
    let mut includes: Vec<String> = Vec::with_capacity(pieces.len().saturating_add(1));
    if !groups_text.is_empty() {
        includes.push(format!("include({KERNING_GROUPS_FILE});"));
    }
    includes.extend(pieces.iter().map(|(entry, _, _)| format!("include({});", entry_path(*entry))));
    let root = if includes.is_empty() { String::new() } else { includes.join("\n\n") + "\n" };

    let limit = Limits::default().text_bytes;
    let total = pieces.iter().fold(root.len().saturating_add(groups_text.len()), |n, (_, text, _)| n.saturating_add(text.len()));
    if total > limit {
        return Err(FeatureError::Limit { what: "bytes of assembled code", limit });
    }

    let unusable = |d: Diagnostic| FeatureError::Unsupported(d.message);
    let mut files = VirtualFiles::new(ROOT_FILE, &root).map_err(unusable)?;
    if !groups_text.is_empty() {
        files.insert(KERNING_GROUPS_FILE, &groups_text).map_err(unusable)?;
    }
    let mut map = SourceMap::new();
    for (entry, text, offsets) in pieces {
        let path = entry_path(entry);
        files.insert(&path, &text).map_err(unusable)?;
        map.add(path, entry, offsets);
    }
    Ok(Assembly { files, map })
}

/// Token expansion, the entry's wrapper, the syntax extensions and (variable target) the
/// variable syntax over one entry: the text and the map back to the stored code, or the
/// diagnostics of the pass that failed with the map from that pass's input to the stored code.
/// The syntax extensions see the wrapper: a `standalone` lookup closes and reopens the
/// `feature tag { … } tag;` block, so it needs the tag.
fn run_passes(
    kind: EntryKind,
    entry: &Snippet,
    target: &Target,
    values: &dyn TargetValues,
) -> Result<(String, OffsetMap), (Vec<Diagnostic>, OffsetMap)> {
    let (expanded, tokens_map) = crate::tokens::expand(&entry.code, values, kind).map_err(|d| (non_empty(d), OffsetMap::identity()))?;
    let (wrapped, wrapped_map) = wrap(kind, entry, &expanded, &tokens_map);
    let (lowered, lower_map) = crate::lower::lower(&wrapped, target.is_variable()).map_err(|d| (non_empty(d), wrapped_map.clone()))?;
    let map = OffsetMap::compose(&wrapped_map, &lower_map);
    if !target.is_variable() {
        return Ok((lowered, map));
    }
    // M9.11 supplies the axes.
    let (text, variable_map) = crate::variable::lower(&lowered, &[]).map_err(|d| (non_empty(d), map.clone()))?;
    Ok((text, OffsetMap::compose(&map, &variable_map)))
}

/// A failed pass always says why.
fn non_empty(mut diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    if diagnostics.is_empty() {
        diagnostics.push(Diagnostic::request(Severity::Error, "the code could not be prepared for compiling"));
    }
    diagnostics
}

/// The entry's statement around its prepared `code`, as glyphsLib writes it, and the map from
/// that text to the stored code. Text outside `code` maps to nothing.
fn wrap(kind: EntryKind, entry: &Snippet, code: &str, map: &OffsetMap) -> (String, OffsetMap) {
    let automatic = if entry.automatic { "# automatic\n" } else { "" };
    let (head, tail) = match kind {
        EntryKind::Class => {
            let at = if entry.name.starts_with('@') { "" } else { "@" };
            (format!("{automatic}{at}{} = [ ", entry.name), "\n];".to_string())
        }
        EntryKind::Prefix => {
            let name = entry.name.replace(['\n', '\r'], " ");
            let comment = if name.is_empty() || name == ANONYMOUS_PREFIX { String::new() } else { format!("# Prefix: {name}\n") };
            (format!("{comment}{automatic}"), String::new())
        }
        EntryKind::Feature => {
            let tag = &entry.name;
            let mut head = format!("feature {tag} {{\n");
            for line in feature_names(&entry.labels) {
                head.push_str(&line);
                head.push('\n');
            }
            head.push_str(automatic);
            // A hand-written kern feature gets the generated kerning after its code.
            let marker = tag == "kern" && !entry.automatic && !code.contains(AUTOMATIC_CODE_MARKER);
            let marker = if marker { format!("\n{AUTOMATIC_CODE_MARKER}\n") } else { String::new() };
            (head, format!("{marker}\n}} {tag};"))
        }
    };
    let mut wrapper = OffsetMap::default();
    wrapper.push(head.len()..head.len().saturating_add(code.len()), 0..code.len());
    let text = format!("{head}{code}{tail}");
    (text, OffsetMap::compose(map, &wrapper))
}

/// The `featureNames` block of a feature's labels, one line each: the default language without
/// a platform, the others as Windows names in their language. Empty labels and languages
/// without a Windows id are left out; no labels, no block.
fn feature_names(labels: &[Localized]) -> Vec<String> {
    let mut lines = Vec::new();
    for label in labels.iter().filter(|l| !l.value.is_empty()) {
        let value = label.value.replace('\\', "\\005c").replace('"', "\\0022");
        if label.language == DEFAULT_LANGUAGE {
            lines.push(format!("  name \"{value}\";"));
        } else if let Some(id) = language_id(&label.language) {
            lines.push(format!("  name 3 1 0x{id:X} \"{value}\";"));
        }
    }
    if !lines.is_empty() {
        lines.insert(0, "featureNames {".to_string());
        lines.push("};".to_string());
    }
    lines
}

/// Definitions of the `@MMK_L_`/`@MMK_R_` classes the prepared code refers to and no class entry
/// defines, with the target's glyphs of each kerning group in font order. A group without
/// glyphs is not defined, so the compiler reports the reference to it.
fn kerning_groups(font: &Font, entries: &Features, pieces: &[(EntryRef, String, OffsetMap)], values: &dyn TargetValues) -> String {
    let defined: HashSet<&str> = entries.classes.iter().filter(|c| !c.disabled).map(|c| c.name.strip_prefix('@').unwrap_or(&c.name)).collect();
    // (side letter, group name)
    let mut wanted: BTreeSet<(char, String)> = BTreeSet::new();
    for (_, text, _) in pieces {
        for (at, _) in text.match_indices("@MMK_") {
            let rest = text.get(at.saturating_add(1)..).unwrap_or_default();
            let class: &str = rest.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))).next().unwrap_or_default();
            let Some(group) = class.strip_prefix("MMK_L_").map(|g| ('L', g)).or_else(|| class.strip_prefix("MMK_R_").map(|g| ('R', g))) else {
                continue;
            };
            if !group.1.is_empty() && !defined.contains(class) {
                wanted.insert((group.0, group.1.to_string()));
            }
        }
    }
    if wanted.is_empty() {
        return String::new();
    }
    let exported: HashSet<&str> = values.glyphs().iter().map(String::as_str).collect();
    let mut members: BTreeMap<(char, String), Vec<&str>> = BTreeMap::new();
    for glyph in font.glyphs.iter().filter(|g| exported.contains(g.name.as_str())) {
        // A glyph's right group holds it when it is the left member of a pair: `@MMK_L_`.
        for (side, group) in [('L', &glyph.kern_groups.right), ('R', &glyph.kern_groups.left)] {
            if let Some(group) = group {
                let key = (side, group.clone());
                if wanted.contains(&key) {
                    members.entry(key).or_default().push(glyph.name.as_str());
                }
            }
        }
    }
    let lines: Vec<String> = members.iter().map(|((side, group), glyphs)| format!("@MMK_{side}_{group} = [{}];", glyphs.join(" "))).collect();
    lines.join("\n")
}

#[cfg(test)]
#[path = "assemble_tests.rs"]
mod tests;
