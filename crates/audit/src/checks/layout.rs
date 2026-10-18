//! OpenType layout checks (M12.52). Runners take the context of their scope ([`crate::GlyphCtx`],
//! [`crate::SourceCtx`], [`crate::BinaryCtx`], [`crate::FamilyCtx`]), the thresholds and a
//! [`crate::Sink`]; their registry entries go in `registry/`.
//!
//! The rules follow FontBakery 1.1.0 (Apache-2.0): `opentype/layout_valid_feature_tags`,
//! `opentype/layout_valid_script_tags`, `opentype/layout_valid_language_tags`,
//! `opentype/gdef_mark_chars`, `opentype/gdef_non_mark_chars`, `opentype/gdef_spacing_marks`,
//! `legacy_accents`, `missing_small_caps_glyphs`, `smallcaps_before_ligatures`,
//! `stylisticset_description`, `gpos7`, `cjk_chws_feature`, `dotted_circle`, `soft_dotted` and
//! `ufo_features_default_languagesystem`; and the legacy family comparison of AFDKO 5.0.1
//! (Apache-2.0): single-face test 11 (BASE script tags), single-face test 24 (the `size`
//! feature) and family test 12 (DFLT/dflt, latn/dflt, TUR for TRK). The script tags that need a
//! complex shaper are ufo2ft 3.9.1's `INDIC_SCRIPTS` and `USE_SCRIPTS` with Khmer and Myanmar,
//! turned into OpenType tags by fontTools' `ot_tags_from_script` (both MIT).
//!
//! Where a check departs from its source it says so at the runner.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use fontcraft_model::glyphdata::GeneralCategory;
use read_fonts::tables::gpos::PositionSubtables;
use read_fonts::tables::gsub::{SingleSubst, SubstitutionSubtables};
use read_fonts::tables::layout::{ClassDef, FeatureList, FeatureParams, ScriptList};
use read_fonts::types::{GlyphId, Tag};
use read_fonts::{FontRef, TableProvider};
use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};

use super::components::{Names, binary_glyph_name};
use super::kerning::cmap_mappings;
use crate::data::{chars, glyphs, tags};
use crate::services::ShapeRequest;
use crate::{BinaryCtx, Hit, Location, Params, Sink, SourceCtx};

/// Most coverage and substitution glyphs one check reads over all lookups. Format 2 coverage
/// and class tables expand ranges, so a damaged font could otherwise ask for billions.
const MAX_COVERAGE_VISITS: usize = 2_000_000;
/// Most texts `check.layout.softDotted` shapes.
const MAX_SOFT_DOTTED_SHAPES: usize = 10_000;
/// Unchanged sequences outside an orthography the soft-dotted check collects (FontBakery's 20).
const MAX_SOFT_DOTTED_EXAMPLES: usize = 20;
/// Items one message lists by name; the rest are counted.
const MAX_LISTED: usize = 20;
/// Language-system statements read from the feature code.
const MAX_LANGUAGESYSTEMS: usize = 1_000;

/// OpenType script tags whose text a complex shaper (Indic, USE, Khmer, Myanmar) handles, sorted.
const COMPLEX_SCRIPT_TAGS: &[[u8; 4]] = &[
    *b"adlm", *b"ahom", *b"bali", *b"batk", *b"beng", *b"bhks", *b"bng2", *b"brah", *b"bugi", *b"buhd", *b"cakm", *b"cham", *b"chrs", *b"cpmn",
    *b"dev2", *b"deva", *b"diak", *b"dogr", *b"dupl", *b"egyp", *b"elym", *b"gjr2", *b"gong", *b"gonm", *b"gran", *b"gujr", *b"gur2", *b"guru",
    *b"hano", *b"hmng", *b"hmnp", *b"java", *b"kali", *b"kawi", *b"khar", *b"khmr", *b"khoj", *b"kits", *b"knd2", *b"knda", *b"kthi", *b"lana",
    *b"lepc", *b"limb", *b"mahj", *b"maka", *b"mand", *b"mani", *b"marc", *b"medf", *b"mlm2", *b"mlym", *b"modi", *b"mong", *b"mtei", *b"mult",
    *b"mym2", *b"mymr", *b"nagm", *b"nand", *b"newa", *b"nko ", *b"ory2", *b"orya", *b"ougr", *b"phag", *b"phlp", *b"plrd", *b"rjng", *b"rohg",
    *b"saur", *b"shrd", *b"sidd", *b"sind", *b"sinh", *b"sogd", *b"sogo", *b"soyo", *b"sund", *b"sylo", *b"tagb", *b"takr", *b"tale", *b"taml",
    *b"tavt", *b"tel2", *b"telu", *b"tfng", *b"tglg", *b"tibt", *b"tirh", *b"tml2", *b"tnsa", *b"toto", *b"vith", *b"wcho", *b"yezi", *b"zanb",
];

/// GDEF glyph class of a mark.
const MARK_CLASS: u16 = 3;

/// A count of glyph visits left; once spent, the check stops and skips.
struct Budget {
    left: usize,
    spent: bool,
}

impl Budget {
    fn new() -> Self {
        Self { left: MAX_COVERAGE_VISITS, spent: false }
    }

    fn take(&mut self) -> bool {
        if self.left == 0 {
            self.spent = true;
            return false;
        }
        self.left -= 1;
        true
    }

    fn reason() -> String {
        format!("the layout tables list more than {MAX_COVERAGE_VISITS} glyphs")
    }
}

/// GSUB or GPOS with the lists the checks read, when they can be read.
struct Layout<'a> {
    tag: &'static str,
    scripts: Option<ScriptList<'a>>,
    features: Option<FeatureList<'a>>,
}

fn layouts<'a>(font: &FontRef<'a>) -> Vec<Layout<'a>> {
    let mut out = Vec::new();
    if let Ok(t) = font.gsub() {
        out.push(Layout { tag: "GSUB", scripts: t.script_list().ok(), features: t.feature_list().ok() });
    }
    if let Ok(t) = font.gpos() {
        out.push(Layout { tag: "GPOS", scripts: t.script_list().ok(), features: t.feature_list().ok() });
    }
    out
}

/// The language systems of a script list: (script, language), `None` for the script's default
/// language system.
fn language_systems(scripts: &ScriptList<'_>) -> Vec<([u8; 4], Option<[u8; 4]>)> {
    let mut out = Vec::new();
    for rec in scripts.script_records() {
        let script = rec.script_tag().to_be_bytes();
        let Ok(s) = rec.script(scripts.offset_data()) else { continue };
        if s.default_lang_sys().is_some_and(|l| l.is_ok()) {
            out.push((script, None));
        }
        out.extend(s.lang_sys_records().iter().map(|l| (script, Some(l.lang_sys_tag().to_be_bytes()))));
    }
    out
}

fn feature_tags(features: &FeatureList<'_>) -> BTreeSet<[u8; 4]> {
    features.feature_records().iter().map(|r| r.feature_tag().to_be_bytes()).collect()
}

/// The lookup-list indices of every feature record tagged `tag`.
fn feature_lookups(features: &FeatureList<'_>, tag: &[u8; 4]) -> BTreeSet<u16> {
    let mut out = BTreeSet::new();
    for rec in features.feature_records().iter().filter(|r| r.feature_tag() == Tag::new(tag)) {
        if let Ok(feature) = rec.feature(features.offset_data()) {
            out.extend(feature.lookup_list_indices().iter().map(|i| i.get()));
        }
    }
    out
}

fn quoted(tag: [u8; 4]) -> String {
    format!("'{}'", tags::tag_text(tag))
}

fn table_place(table: &str) -> Location {
    Location { table: Some(table.to_string()), ..Location::default() }
}

fn feature_place(table: &str, tag: [u8; 4]) -> Location {
    Location { feature: Some(tags::tag_text(tag)), ..table_place(table) }
}

fn num_glyphs(font: &FontRef<'_>) -> u32 {
    font.maxp().map_or(0, |m| u32::from(m.num_glyphs()))
}

fn glyph_classes<'a>(font: &FontRef<'a>) -> Option<ClassDef<'a>> {
    font.gdef().ok().and_then(|g| g.glyph_class_def()).and_then(Result::ok)
}

/// A list for a message: the first [`MAX_LISTED`] items, then how many more.
fn listing(items: &[String]) -> String {
    let shown: Vec<&str> = items.iter().take(MAX_LISTED).map(String::as_str).collect();
    let more = items.len().saturating_sub(MAX_LISTED);
    if more > 0 { format!("{} and {more} more", shown.join(", ")) } else { shown.join(", ") }
}

/// `check.layout.validFeatureTags`: a GSUB or GPOS feature tag that is neither registered nor
/// private. FontBakery takes any tag Python calls upper case as private (digits allowed beside
/// capitals); here a private tag is capital letters only, padded with spaces, as its rationale
/// says.
pub(crate) fn valid_feature_tags(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    for layout in layouts(&ctx.font) {
        let Some(features) = &layout.features else { continue };
        for tag in feature_tags(features) {
            if tags::is_registered_feature(tag) || tags::is_private_feature(tag) {
                continue;
            }
            let message = format!(
                "{} uses the feature tag {}, which is not registered and is not a private tag (one to four capital letters).",
                layout.tag,
                quoted(tag)
            );
            sink.push(Hit::new("bad-feature-tag", message).place(feature_place(layout.tag, tag)));
        }
    }
}

/// `check.layout.validScriptTags`: an unregistered script tag in GSUB, GPOS or the BASE
/// table's script lists.
pub(crate) fn valid_script_tags(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let mut found: BTreeSet<(&'static str, [u8; 4])> = BTreeSet::new();
    for layout in layouts(&ctx.font) {
        let Some(scripts) = &layout.scripts else { continue };
        found.extend(scripts.script_records().iter().map(|r| (layout.tag, r.script_tag().to_be_bytes())));
    }
    if let Ok(base) = ctx.font.base() {
        for axis in [base.horiz_axis(), base.vert_axis()].into_iter().flatten().filter_map(Result::ok) {
            let Ok(list) = axis.base_script_list() else { continue };
            found.extend(list.base_script_records().iter().map(|r| ("BASE", r.base_script_tag().to_be_bytes())));
        }
    }
    for (table, tag) in found {
        if tags::is_registered_script(tag) {
            continue;
        }
        let place = Location { script: Some(tags::tag_text(tag)), ..table_place(table) };
        sink.push(Hit::new("bad-script-tag", format!("{table} uses the script tag {}, which is not registered.", quoted(tag))).place(place));
    }
}

/// `check.layout.validLanguageTags`: an unregistered language-system tag in GSUB or GPOS. A tag
/// often written by mistake for another (TUR for Turkish's TRK) gets its own message and the fix.
pub(crate) fn valid_language_tags(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    for layout in layouts(&ctx.font) {
        let Some(scripts) = &layout.scripts else { continue };
        let mut seen: BTreeSet<[u8; 4]> = BTreeSet::new();
        for (script, language) in language_systems(scripts) {
            let Some(tag) = language else { continue };
            if tags::is_registered_language(tag) || !seen.insert(tag) {
                continue;
            }
            let place = Location { script: Some(tags::tag_text(script)), language: Some(tags::tag_text(tag)), ..table_place(layout.tag) };
            if let Some(meant) = tags::language_slip(tag) {
                let name = tags::language_name(meant).unwrap_or("another language");
                let message = format!("{} uses the language tag {}, which is not registered; {name} is {}.", layout.tag, quoted(tag), quoted(meant));
                sink.push(Hit::new("language-slip", message).place(place));
            } else {
                let message = format!("{} uses the language tag {}, which is not registered.", layout.tag, quoted(tag));
                sink.push(Hit::new("bad-language-tag", message).place(place).without_fix());
            }
        }
    }
}

/// `check.layout.defaultLanguageSystems`: a GSUB or GPOS with features lacks the default
/// language system of script DFLT, or, in a font that maps the basic Latin letters, that of
/// script latn. The legacy family comparison asks for latn/dflt in every font; limiting it to
/// fonts with Latin letters is our choice.
pub(crate) fn default_language_systems(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let latin = cmap_mappings(&ctx.font, true).iter().any(|(c, _)| char::from_u32(*c).is_some_and(|ch| ch.is_ascii_alphabetic()));
    for layout in layouts(&ctx.font) {
        if layout.features.as_ref().is_none_or(|f| f.feature_records().is_empty()) {
            continue;
        }
        let systems = layout.scripts.as_ref().map(language_systems).unwrap_or_default();
        let has = |script: &[u8; 4]| systems.iter().any(|(s, l)| s == script && l.is_none());
        if !has(b"DFLT") {
            let message = format!("{} has no default language system for script 'DFLT'.", layout.tag);
            let place = Location { script: Some("DFLT".into()), language: Some("dflt".into()), ..table_place(layout.tag) };
            sink.push(Hit::new("missing-dflt-dflt", message).place(place));
        }
        if latin && !has(b"latn") {
            let message = format!("{} has no default language system for script 'latn', and the font maps Latin letters.", layout.tag);
            let place = Location { script: Some("latn".into()), language: Some("dflt".into()), ..table_place(layout.tag) };
            sink.push(Hit::new("missing-latn-dflt", message).place(place));
        }
    }
}

/// A tag in a `languagesystem` statement: one to four characters a feature-file tag may hold.
fn is_fea_tag(token: &str) -> bool {
    (1..=4).contains(&token.len()) && token.chars().all(|c| c.is_ascii_alphanumeric() || "._!$%&*+:?^'|~".contains(c))
}

/// The (script, language) pairs of the `languagesystem` statements in feature code, in order,
/// with comments left out. At most `limit`.
fn languagesystem_statements(code: &str, limit: usize) -> Vec<(String, String)> {
    let mut tokens = Vec::new();
    for line in code.lines() {
        let text = line.split('#').next().unwrap_or_default();
        tokens.extend(text.split(|c: char| c.is_whitespace() || c == ';' || c == '{' || c == '}').filter(|t| !t.is_empty()));
    }
    let mut out = Vec::new();
    let mut rest = tokens.iter();
    while let Some(token) = rest.next() {
        if *token != "languagesystem" {
            continue;
        }
        let mut ahead = rest.clone();
        if let (Some(script), Some(language)) = (ahead.next(), ahead.next())
            && is_fea_tag(script)
            && is_fea_tag(language)
        {
            out.push((script.to_string(), language.to_string()));
            rest = ahead;
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

/// `check.layout.defaultLanguagesystemStatement`: the feature code declares language systems
/// and the first is not `DFLT dflt`. Prefixes are read first, then classes, then features, the
/// order the compiler joins them in; switched-off snippets and comments are left out.
pub(crate) fn default_languagesystem_statement(ctx: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let features = &ctx.audit.font.features;
    let mut statements: Vec<(String, String, &str)> = Vec::new();
    for snippet in features.prefixes.iter().chain(&features.classes).chain(&features.features).filter(|s| !s.disabled) {
        let left = MAX_LANGUAGESYSTEMS.saturating_sub(statements.len());
        if left == 0 {
            break;
        }
        statements.extend(languagesystem_statements(&snippet.code, left).into_iter().map(|(s, l)| (s, l, snippet.name.as_str())));
    }
    let Some((script, language, snippet)) = statements.first() else { return };
    if script == "DFLT" && language == "dflt" {
        return;
    }
    let declared: Vec<String> = statements.iter().map(|(s, l, _)| format!("{s} {l}")).collect();
    let message = format!(
        "The feature code declares the language systems {} (the first in '{snippet}'), but not 'languagesystem DFLT dflt' as the first of them.",
        listing(&declared)
    );
    sink.push(Hit::new("default-languagesystem", message).place(Location { feature: Some(snippet.to_string()), ..Location::default() }));
}

/// Neither a mark nor a non-mark for the GDEF checks: control, format, surrogate, private-use
/// and unassigned characters.
fn is_other(category: GeneralCategory) -> bool {
    matches!(category, GeneralCategory::Cc | GeneralCategory::Cf | GeneralCategory::Cs | GeneralCategory::Co | GeneralCategory::Cn)
}

fn glyph_place(name: String) -> Location {
    Location { glyph: name, ..table_place("GDEF") }
}

/// `check.layout.gdefMarkClass`: the GDEF glyph classes against Unicode. A non-spacing mark
/// character (Mn, Me) whose glyph is not in the mark class; a character of another category
/// (control and other C characters aside) whose glyph is; a glyph in the mark class with an
/// advance width.
pub(crate) fn gdef_mark_class(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let Some(classes) = glyph_classes(font) else {
        sink.skip("the font has no GDEF glyph classes");
        return;
    };
    if glyphs::glyph_data().is_none() {
        sink.skip("the Unicode data cannot be read");
        return;
    }
    let names = Names::new(font);
    let is_mark = |g: u32| classes.get(GlyphId::new(g)) == MARK_CLASS;
    for (cp, g) in cmap_mappings(font, true) {
        let Some(category) = glyphs::general_category(cp) else { continue };
        if is_other(category) {
            continue;
        }
        let nonspacing = matches!(category, GeneralCategory::Mn | GeneralCategory::Me);
        let name = binary_glyph_name(ctx, &names, g);
        if nonspacing && !is_mark(g) {
            let message = format!("{name} (U+{cp:04X}) is a non-spacing mark but is not in the GDEF mark class.");
            sink.push(Hit::new("mark-chars", message).place(glyph_place(name)));
        } else if !nonspacing && is_mark(g) {
            let message = format!("{name} (U+{cp:04X}) is not a mark character but is in the GDEF mark class.");
            sink.push(Hit::new("non-mark-chars", message).place(glyph_place(name)));
        }
    }
    let Ok(hmtx) = font.hmtx() else { return };
    for g in 0..num_glyphs(font) {
        if !is_mark(g) {
            continue;
        }
        let Some(width) = hmtx.advance(GlyphId::new(g)).filter(|w| *w > 0) else { continue };
        let name = binary_glyph_name(ctx, &names, g);
        let message = format!(
            "{name} is in the GDEF mark class but advances by {width} units: a spacing glyph classed as a mark, or a mark that should have no width."
        );
        sink.push(Hit::new("spacing-mark-glyphs", message).place(glyph_place(name)).data("width", f64::from(width)));
    }
}

/// `check.layout.legacyAccents`: a spacing legacy accent with no advance width, or classed as a
/// mark in GDEF. Every `cmap` subtable is read, as FontBakery's reversed map does.
pub(crate) fn legacy_accents(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let classes = glyph_classes(font);
    let hmtx = font.hmtx().ok();
    let names = Names::new(font);
    let mut accents: BTreeMap<u32, u32> = BTreeMap::new();
    for (cp, g) in cmap_mappings(font, false).into_iter().filter(|(cp, _)| chars::LEGACY_ACCENTS.contains(cp)) {
        accents.entry(g).or_insert(cp);
    }
    for (g, cp) in accents {
        let name = binary_glyph_name(ctx, &names, g);
        if hmtx.as_ref().and_then(|h| h.advance(GlyphId::new(g))) == Some(0) {
            let message = format!("The legacy accent {name} (U+{cp:04X}) has no advance width; it is typed on its own and needs one.");
            let place = Location { glyph: name.clone(), ..table_place("hmtx") };
            sink.push(Hit::new("legacy-accents-width", message).place(place).without_fix());
        }
        if classes.as_ref().is_some_and(|c| c.get(GlyphId::new(g)) == MARK_CLASS) {
            let message = format!("The legacy accent {name} (U+{cp:04X}) is classed as a mark in GDEF; it should be a base glyph.");
            sink.push(Hit::new("legacy-accents-gdef", message).place(glyph_place(name)));
        }
    }
}

/// What a set of GSUB lookups substitutes: the glyphs single, multiple and alternate
/// substitutions take in and give out (ligature and contextual lookups are not read, as in
/// FontBakery).
#[derive(Default)]
struct Substitutions {
    inputs: HashSet<u32>,
    outputs: BTreeSet<u32>,
}

fn substitutions(font: &FontRef<'_>, lookups: &BTreeSet<u16>, budget: &mut Budget) -> Substitutions {
    let mut out = Substitutions::default();
    let Some(list) = font.gsub().ok().and_then(|g| g.lookup_list().ok()) else { return out };
    for i in lookups {
        let Ok(lookup) = list.lookups().get(usize::from(*i)) else { continue };
        match lookup.subtables() {
            Ok(SubstitutionSubtables::Single(subs)) => {
                for sub in subs.iter().filter_map(Result::ok) {
                    match sub {
                        SingleSubst::Format1(t) => {
                            let Ok(coverage) = t.coverage() else { continue };
                            let delta = i32::from(t.delta_glyph_id());
                            for g in coverage.iter() {
                                if !budget.take() {
                                    return out;
                                }
                                out.inputs.insert(g.to_u32());
                                out.outputs.insert((i32::from(g.to_u16()) + delta).rem_euclid(65_536).unsigned_abs());
                            }
                        }
                        SingleSubst::Format2(t) => {
                            let Ok(coverage) = t.coverage() else { continue };
                            for g in coverage.iter() {
                                if !budget.take() {
                                    return out;
                                }
                                out.inputs.insert(g.to_u32());
                            }
                            for s in t.substitute_glyph_ids() {
                                if !budget.take() {
                                    return out;
                                }
                                out.outputs.insert(s.get().to_u32());
                            }
                        }
                    }
                }
            }
            Ok(SubstitutionSubtables::Multiple(subs)) => {
                for t in subs.iter().filter_map(Result::ok) {
                    let Ok(coverage) = t.coverage() else { continue };
                    for g in coverage.iter() {
                        if !budget.take() {
                            return out;
                        }
                        out.inputs.insert(g.to_u32());
                    }
                    for sequence in t.sequences().iter().filter_map(Result::ok) {
                        for s in sequence.substitute_glyph_ids() {
                            if !budget.take() {
                                return out;
                            }
                            out.outputs.insert(s.get().to_u32());
                        }
                    }
                }
            }
            Ok(SubstitutionSubtables::Alternate(subs)) => {
                for t in subs.iter().filter_map(Result::ok) {
                    let Ok(coverage) = t.coverage() else { continue };
                    for g in coverage.iter() {
                        if !budget.take() {
                            return out;
                        }
                        out.inputs.insert(g.to_u32());
                    }
                    for set in t.alternate_sets().iter().filter_map(Result::ok) {
                        for s in set.alternate_glyph_ids() {
                            if !budget.take() {
                                return out;
                            }
                            out.outputs.insert(s.get().to_u32());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// `check.layout.smallCapsGlyphs`: `smcp` or `c2sc` leaves letters without a small cap, or
/// substitutes a glyph the font does not have. FontBakery checks only the second; the letters
/// a feature should cover are our choice: the cased letters of the feature's case (lowercase for
/// `smcp`, uppercase for `c2sc`) that the font maps, in the scripts of the letters the feature
/// does cover (every script when it covers none).
pub(crate) fn small_caps_glyphs(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let Some(features) = font.gsub().ok().and_then(|g| g.feature_list().ok()) else { return };
    let present = feature_tags(&features);
    if !present.contains(b"smcp") && !present.contains(b"c2sc") {
        return;
    }
    let Some(data) = glyphs::glyph_data() else {
        sink.skip("the Unicode data cannot be read");
        return;
    };
    let count = num_glyphs(font);
    let mapped = cmap_mappings(font, true);
    let names = Names::new(font);
    let mut budget = Budget::new();
    for (tag, case, wanted) in [(*b"smcp", "lowercase", GeneralCategory::Ll), (*b"c2sc", "uppercase", GeneralCategory::Lu)] {
        if !present.contains(&tag) {
            continue;
        }
        let subs = substitutions(font, &feature_lookups(&features, &tag), &mut budget);
        if budget.spent {
            sink.skip(Budget::reason());
            return;
        }
        let missing: Vec<String> = subs.outputs.iter().filter(|g| **g >= count).map(|g| format!("glyph {g}")).collect();
        if !missing.is_empty() {
            let message = format!("{} substitutes glyphs the font does not have: {}.", quoted(tag), listing(&missing));
            sink.push(Hit::new("missing-glyphs", message).place(feature_place("GSUB", tag)).data("glyphs", missing.len() as f64));
        }
        let letters: Vec<(u32, u32)> =
            mapped.iter().copied().filter(|(cp, _)| glyphs::general_category(*cp) == Some(wanted) && glyphs::case_partner(*cp).is_some()).collect();
        let scripts: BTreeSet<&str> =
            letters.iter().filter(|(_, g)| subs.inputs.contains(g)).filter_map(|(cp, _)| data.unicode_script(*cp)).collect();
        let left: Vec<String> = letters
            .iter()
            .filter(|(cp, g)| !subs.inputs.contains(g) && (scripts.is_empty() || data.unicode_script(*cp).is_some_and(|s| scripts.contains(s))))
            .map(|(cp, g)| format!("U+{cp:04X} {}", binary_glyph_name(ctx, &names, *g)))
            .collect();
        if !left.is_empty() {
            let message = format!("{} gives no small cap for {} {case} letters: {}.", quoted(tag), left.len(), listing(&left));
            sink.push(Hit::new("missing-small-caps", message).place(feature_place("GSUB", tag)).data("letters", left.len() as f64));
        }
    }
}

/// `check.layout.smallCapsBeforeLigatures`: the first `liga` lookup comes before the first
/// `smcp` lookup in the lookup list. Skipped when either feature is missing.
pub(crate) fn small_caps_before_ligatures(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(features) = ctx.font.gsub().ok().and_then(|g| g.feature_list().ok()) else {
        sink.skip("the font has no readable GSUB feature list");
        return;
    };
    let (Some(smcp), Some(liga)) = (feature_lookups(&features, b"smcp").first().copied(), feature_lookups(&features, b"liga").first().copied())
    else {
        sink.skip("the font has no 'smcp' or no 'liga' lookups");
        return;
    };
    if smcp < liga {
        return;
    }
    let message = format!(
        "The first 'liga' lookup ({}) does not come after the first 'smcp' lookup ({}), so ligatures form before small caps apply.",
        liga, smcp
    );
    let place = Location { feature: Some("liga".into()), lookup: Some(usize::from(liga)), ..table_place("GSUB") };
    sink.push(Hit::new("feature-ordering", message).place(place).data("ligaLookup", f64::from(liga)).data("smcpLookup", f64::from(smcp)));
}

/// `ss01` to `ss20`.
fn is_stylistic_set(tag: [u8; 4]) -> bool {
    match tag {
        [b's', b's', a, b] if a.is_ascii_digit() && b.is_ascii_digit() => (1..=20).contains(&(u32::from(a - b'0') * 10 + u32::from(b - b'0'))),
        _ => false,
    }
}

/// `check.layout.stylisticSetNames`: a stylistic set feature without feature parameters (no
/// name), or whose name ID is not in the `name` table. FontBakery leaves the second as a to-do;
/// it is reported here as well.
pub(crate) fn stylistic_set_names(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(features) = ctx.font.gsub().ok().and_then(|g| g.feature_list().ok()) else { return };
    let ids: HashSet<u16> = ctx.font.name().map(|n| n.name_record().iter().map(|r| r.name_id().to_u16()).collect()).unwrap_or_default();
    let mut reported: BTreeSet<[u8; 4]> = BTreeSet::new();
    for rec in features.feature_records() {
        let tag = rec.feature_tag().to_be_bytes();
        if !is_stylistic_set(tag) || reported.contains(&tag) {
            continue;
        }
        let params = rec.feature(features.offset_data()).ok().and_then(|f| f.feature_params()).and_then(Result::ok);
        let hit = match params {
            Some(FeatureParams::StylisticSet(p)) => {
                let id = p.ui_name_id().to_u16();
                if ids.contains(&id) {
                    continue;
                }
                let message = format!("The stylistic set {} names its description by name ID {id}, which is not in the name table.", quoted(tag));
                Hit::new("name-id-missing", message).data("nameId", f64::from(id))
            }
            _ => Hit::new("missing-description", format!("The stylistic set {} has no description string in the name table.", quoted(tag))),
        };
        reported.insert(tag);
        sink.push(hit.place(feature_place("GSUB", tag)));
    }
}

/// `check.layout.gpos7`: a GPOS contextual (type 7) lookup, directly or through an extension.
pub(crate) fn gpos7(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(list) = ctx.font.gpos().ok().and_then(|g| g.lookup_list().ok()) else { return };
    for (i, lookup) in list.lookups().iter().enumerate() {
        let Ok(lookup) = lookup else { continue };
        if matches!(lookup.subtables(), Ok(PositionSubtables::Contextual(_))) {
            let message = format!(
                "GPOS lookup {i} is a contextual positioning lookup (type 7), which some shapers do not apply; chaining contextual lookups (type 8) work everywhere."
            );
            sink.push(Hit::new("has-gpos7", message).place(Location { lookup: Some(i), ..table_place("GPOS") }));
        }
    }
}

/// `check.layout.cjkSpacingFeatures`: a CJK font (more than [`chars::MIN_CJK_GLYPHS`] mapped
/// characters in CJK blocks) without `chws` or `vchw`.
pub(crate) fn cjk_spacing_features(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let cjk = cmap_mappings(&ctx.font, true).iter().filter(|(cp, _)| chars::is_cjk(*cp)).count();
    if cjk <= chars::MIN_CJK_GLYPHS {
        sink.skip(format!("the font is not a CJK font: it maps {cjk} CJK characters"));
        return;
    }
    let present: BTreeSet<[u8; 4]> = layouts(&ctx.font).iter().filter_map(|l| l.features.as_ref()).flat_map(feature_tags).collect();
    for (tag, code) in [(*b"chws", "missing-chws-feature"), (*b"vchw", "missing-vchw-feature")] {
        if !present.contains(&tag) {
            let message = format!("The CJK font has no {} feature for contextual half-width spacing.", quoted(tag));
            sink.push(Hit::new(code, message).place(feature_place("GPOS", tag)));
        }
    }
}

/// A script of GSUB or GPOS needs a complex shaper.
fn uses_complex_shaper(font: &FontRef<'_>) -> bool {
    layouts(font)
        .iter()
        .filter_map(|l| l.scripts.as_ref())
        .any(|s| s.script_records().iter().any(|r| COMPLEX_SCRIPT_TAGS.binary_search(&r.script_tag().to_be_bytes()).is_ok()))
}

/// `check.layout.dottedCircle`: the font has encoded non-spacing marks (mark class, no advance)
/// but no U+25CC, or marks that attach to some base through a mark-to-base lookup do not attach
/// to the dotted circle. FontBakery runs it on TrueType fonts only; here it runs on every font.
pub(crate) fn dotted_circle(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let mapped = cmap_mappings(font, true);
    let classes = glyph_classes(font);
    let hmtx = font.hmtx().ok();
    let marks: BTreeSet<u32> = mapped
        .iter()
        .map(|(_, g)| *g)
        .filter(|g| classes.as_ref().is_some_and(|c| c.get(GlyphId::new(*g)) == MARK_CLASS))
        .filter(|g| hmtx.as_ref().and_then(|h| h.advance(GlyphId::new(*g))) == Some(0))
        .collect();
    if marks.is_empty() {
        sink.skip("the font has no non-spacing mark glyphs");
        return;
    }
    let Some(circle) = mapped.iter().find(|(cp, _)| *cp == chars::DOTTED_CIRCLE).map(|(_, g)| *g) else {
        if uses_complex_shaper(font) {
            sink.push(Hit::new(
                "missing-dotted-circle-complex",
                "The font has non-spacing marks and a script that needs a complex shaper, but no dotted circle (U+25CC).",
            ));
        } else {
            sink.push(Hit::new("missing-dotted-circle", "The font has non-spacing marks but no dotted circle (U+25CC)."));
        }
        return;
    };
    let mut attaching: HashSet<u32> = HashSet::new();
    let mut on_circle: HashSet<u32> = HashSet::new();
    let mut budget = Budget::new();
    if let Some(list) = font.gpos().ok().and_then(|g| g.lookup_list().ok()) {
        for lookup in list.lookups().iter().filter_map(Result::ok) {
            let Ok(PositionSubtables::MarkToBase(subs)) = lookup.subtables() else { continue };
            for sub in subs.iter().filter_map(Result::ok) {
                let Ok(coverage) = sub.mark_coverage() else { continue };
                let circle_base = sub.base_coverage().is_ok_and(|b| b.get(GlyphId::new(circle)).is_some());
                for g in coverage.iter() {
                    if !budget.take() {
                        sink.skip(Budget::reason());
                        return;
                    }
                    attaching.insert(g.to_u32());
                    if circle_base {
                        on_circle.insert(g.to_u32());
                    }
                }
            }
        }
    }
    let names = Names::new(font);
    for g in marks.into_iter().filter(|g| attaching.contains(g) && !on_circle.contains(g)) {
        let name = binary_glyph_name(ctx, &names, g);
        let message = format!("The mark {name} attaches to other bases but not to the dotted circle.");
        sink.push(Hit::new("unattached-dotted-circle-marks", message).place(Location { glyph: name, ..table_place("GPOS") }));
    }
}

/// Counts the contours a glyph draws.
#[derive(Default)]
struct ContourCount(usize);

impl OutlinePen for ContourCount {
    fn move_to(&mut self, _: f32, _: f32) {
        self.0 = self.0.saturating_add(1);
    }
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
    fn close(&mut self) {}
}

fn contours(font: &FontRef<'_>, glyph: Option<u32>) -> Option<usize> {
    let outline = font.outline_glyphs().get(GlyphId::new(glyph?))?;
    let mut pen = ContourCount::default();
    outline.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).ok()?;
    Some(pen.0)
}

/// The Unicode blocks whose names start with "Combining Diacritical Marks".
fn in_diacritical_blocks(cp: u32) -> bool {
    matches!(cp, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF)
}

/// The Unicode blocks whose names start with "Cyrillic".
fn in_cyrillic_blocks(cp: u32) -> bool {
    matches!(cp, 0x0400..=0x04FF | 0x0500..=0x052F | 0x1C80..=0x1C8F | 0x2DE0..=0x2DFF | 0xA640..=0xA69F | 0x1E030..=0x1E08F)
}

/// `check.layout.softDotted`: shaping a soft-dotted letter, an optional mark below or
/// overlaid, and a mark above leaves the letter's glyph unchanged, so the dot stays. Texts in
/// [`chars::SOFT_DOTTED_SEQUENCES`] are reported apart from the rest; the rest stops at
/// [`MAX_SOFT_DOTTED_EXAMPLES`]. Skipped when `i` draws as many contours as `I` or `ı`, which
/// leaves unclear whether its glyph has a dot.
pub(crate) fn soft_dotted(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let font = &ctx.font;
    let map: BTreeMap<u32, u32> = cmap_mappings(font, true).into_iter().collect();
    let soft: Vec<u32> = map.keys().copied().filter(|cp| chars::is_soft_dotted(*cp)).collect();
    let above: Vec<u32> =
        map.keys().copied().filter(|cp| glyphs::combining_class(*cp) == 230 && (in_diacritical_blocks(*cp) || in_cyrillic_blocks(*cp))).collect();
    let mut below: Vec<u32> = vec![0];
    below.extend(map.keys().copied().filter(|cp| glyphs::combining_class(*cp) < 230 && in_diacritical_blocks(*cp)));
    if soft.is_empty() || above.is_empty() {
        sink.skip("the font has no soft-dotted letters or no marks above");
        return;
    }
    let count = |cp: u32| contours(font, map.get(&cp).copied());
    let i = count(0x69);
    if i.is_some() && (i == count(0x49) || i == count(0x131)) {
        sink.skip("the glyph of i draws as many contours as I or dotless i, so whether it has a dot is unclear");
        return;
    }
    let mut orthography: Vec<String> = Vec::new();
    let mut others: Vec<String> = Vec::new();
    let mut shaped = 0usize;
    for s in &soft {
        for b in &below {
            for a in &above {
                let sequence: Vec<u32> = [*s, *b, *a].into_iter().filter(|cp| *cp != 0).collect();
                let known = chars::SOFT_DOTTED_SEQUENCES.contains(&sequence.as_slice());
                if !known && others.len() >= MAX_SOFT_DOTTED_EXAMPLES {
                    continue;
                }
                if shaped >= MAX_SOFT_DOTTED_SHAPES {
                    sink.skip(format!("the font needs more than {MAX_SOFT_DOTTED_SHAPES} texts shaped"));
                    return;
                }
                shaped += 1;
                let text: String = sequence.iter().filter_map(|cp| char::from_u32(*cp)).collect();
                let request = ShapeRequest { text: text.clone(), ..ShapeRequest::default() };
                let output = match ctx.services.shape(ctx.bytes, &request) {
                    Ok(glyphs) => glyphs,
                    Err(u) => {
                        sink.skip(u.reason);
                        return;
                    }
                };
                let unchanged: Vec<u32> = sequence.iter().filter_map(|cp| map.get(cp).copied()).collect();
                if output.iter().map(|g| g.glyph_id).eq(unchanged.iter().copied()) {
                    if known { orthography.push(text) } else { others.push(text) }
                }
            }
        }
    }
    if !orthography.is_empty() {
        let message = format!("The dot of a soft-dotted letter stays under a mark above in texts that orthographies use: {}.", listing(&orthography));
        sink.push(Hit::new("soft-dotted-orthography", message).data("texts", orthography.len() as f64));
    }
    if !others.is_empty() {
        let message = format!("The dot of a soft-dotted letter stays under a mark above, for example in: {}.", listing(&others));
        sink.push(Hit::new("soft-dotted", message).data("texts", others.len() as f64));
    }
}

/// `check.layout.sizeFeature`: the GPOS `size` feature's parameters are missing, the design
/// size lies outside the range they give (checked, as the legacy comparison does, only when
/// they name a menu name), or that menu name ID is not in the `name` table.
pub(crate) fn size_feature(ctx: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    let Some(features) = ctx.font.gpos().ok().and_then(|g| g.feature_list().ok()) else { return };
    let ids: HashSet<u16> = ctx.font.name().map(|n| n.name_record().iter().map(|r| r.name_id().to_u16()).collect()).unwrap_or_default();
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    let place = || feature_place("GPOS", *b"size");
    for rec in features.feature_records().iter().filter(|r| r.feature_tag() == Tag::new(b"size")) {
        if !seen.insert(rec.feature_offset().to_u32()) {
            continue;
        }
        let params = rec.feature(features.offset_data()).ok().and_then(|f| f.feature_params()).and_then(Result::ok);
        let Some(FeatureParams::Size(p)) = params else {
            sink.push(Hit::new("size-params-missing", "The 'size' feature has no readable parameters.").place(place()));
            continue;
        };
        let (design, name_id, start, end) = (p.design_size(), p.name_entry(), p.range_start(), p.range_end());
        if name_id == 0 {
            continue;
        }
        if design < start || design > end {
            let message = format!(
                "The 'size' feature's design size {} pt lies outside its range {} to {} pt.",
                crate::num(f64::from(design) / 10.0),
                crate::num(f64::from(start) / 10.0),
                crate::num(f64::from(end) / 10.0)
            );
            sink.push(Hit::new("size-outside-range", message).place(place()).data("designSize", f64::from(design)));
        }
        if !ids.contains(&name_id) {
            let message = format!("The 'size' feature names its menu name by name ID {name_id}, which is not in the name table.");
            sink.push(Hit::new("size-name-missing", message).place(Location { name_id: Some(name_id), ..place() }));
        }
    }
}

/// `check.layout.shapingRegression`: the project's shaping test files. No test-file reader
/// exists yet, so the check always skips.
pub(crate) fn shaping_regression(_: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    sink.skip("shaping test files cannot be read yet");
}

/// `check.layout.languageShaping`: needs a database of shaping expectations per language, which
/// FontCraft does not have.
pub(crate) fn language_shaping(_: &BinaryCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    sink.skip("no database of shaping expectations per language is available");
}

/// `check.layout.undefinedGlyphInFeatures`: needs the feature-code parser to tell glyph names
/// from keywords, tags and lookup names.
pub(crate) fn undefined_glyph_in_features(_: &SourceCtx<'_>, _: &Params<'_>, sink: &mut Sink) {
    sink.skip("the feature code cannot be parsed for glyph names yet");
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
