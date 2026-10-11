//! Localised forms (M8.12).
//!
//! Rows: `autoFeature.locl`, and the languages `editView.setLocalizedForms` offers
//! ([`locl_languages`]).
//!
//! Two kinds of trigger write rules:
//! - The built-in sets of the handbook, each one lookup shared by its languages: the Turkish
//!   dotted i (AZE CRT KAZ TAT TRK), the Romanian comma accents (ROM MOL), the Catalan punt volat
//!   (CAT), the Dutch accented IJ (NLD) and the Urdu figures (URD, Arabic script).
//! - Any other glyph whose name has a `.loclXXX` suffix: `sub base by base.loclXXX;` under its
//!   script and language XXX, when the base glyph exists. The glyphs the built-in sets name are
//!   left to those sets, whether or not their conditions hold.
//!
//! The code has the form of the reference's version 4 output in the corpus: one `script` section
//! per script, and in it one `language` statement per lookup, listing every language that shares
//! it (`language AZE CRT KAZ TAT TRK;`, the multiple-languages extension fea-rs accepts).

use super::{AutoContext, AutoEntry, Generator, MAX_GENERATED_BYTES};
use crate::FeatureError;
use crate::source_map::EntryKind;
use fontcraft_model::Features;

/// Most (script, language) pairs [`locl_languages`] returns.
pub const MAX_LOCL_LANGUAGES: usize = 4_096;

/// Glyph names the built-in sets own; the `.loclXXX` rule never writes them.
const BUILT_IN_NAMES: &[&str] = &[
    "idotaccent",
    "i.TRK",
    "i.loclTRK",
    "periodcentered.loclCAT",
    "periodcentered.loclCAT.case",
    "L_periodcentered_L.loclCAT",
    "l_periodcentered_l.loclCAT",
    "Iacute_J.loclNLD",
    "iacute_j.loclNLD",
];

struct Locl;

static LOCL: Locl = Locl;

/// This module's generators.
pub(super) fn generators() -> Vec<&'static dyn Generator> {
    vec![&LOCL]
}

impl Generator for Locl {
    fn names(&self) -> &'static [&'static str] {
        &["locl"]
    }

    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError> {
        let mut sets = built_in_sets(cx);
        sets.extend(suffix_sets(cx));
        if sets.is_empty() {
            return Ok(Vec::new());
        }
        let code = write(&sets)?;
        Ok(vec![AutoEntry { kind: EntryKind::Feature, name: "locl".into(), code, optional: false }])
    }
}

/// One lookup and the language systems it is registered for.
#[derive(Debug)]
struct RuleSet {
    lookup: String,
    /// OpenType script tags, without padding.
    scripts: Vec<String>,
    languages: Vec<String>,
    rules: Vec<String>,
}

fn set(lookup: &str, script: &str, languages: &[&str], rules: Vec<String>) -> Option<RuleSet> {
    (!rules.is_empty()).then(|| RuleSet {
        lookup: lookup.into(),
        scripts: vec![script.into()],
        languages: languages.iter().map(|l| (*l).to_string()).collect(),
        rules,
    })
}

/// The first of `names` the context has.
fn first<'a>(cx: &AutoContext, names: &[&'a str]) -> Option<&'a str> {
    names.iter().copied().find(|n| cx.has(n))
}

fn built_in_sets(cx: &AutoContext) -> Vec<RuleSet> {
    let mut sets = Vec::new();

    // Turkish and related languages: the dotted i keeps its dot under capitalisation rules, so
    // the lowercase i becomes the explicitly dotted glyph.
    if cx.has("i")
        && let Some(target) = first(cx, &["idotaccent", "i.TRK", "i.loclTRK"])
    {
        sets.extend(set("idotaccent", "latn", &["AZE", "CRT", "KAZ", "TAT", "TRK"], vec![format!("sub i by {target};")]));
    }

    let pairs = [("Scedilla", "Scommaaccent"), ("scedilla", "scommaaccent"), ("Tcedilla", "Tcommaaccent"), ("tcedilla", "tcommaaccent")];
    let rules = pairs.iter().filter(|(from, to)| cx.has(from) && cx.has(to)).map(|(from, to)| format!("sub {from} by {to};")).collect();
    sets.extend(set("commaaccent", "latn", &["ROM", "MOL"], rules));

    if cx.has("L") && cx.has("l") && cx.has("periodcentered") {
        let mut rules = Vec::new();
        if cx.has("periodcentered.loclCAT") {
            rules.push("sub l periodcentered' l by periodcentered.loclCAT;".to_string());
        } else if cx.has("l_periodcentered_l.loclCAT") {
            rules.push("sub l periodcentered l by l_periodcentered_l.loclCAT;".to_string());
        } else if cx.has("ldot") {
            rules.push("sub l' periodcentered' l by ldot;".to_string());
        }
        if let Some(dot) = first(cx, &["periodcentered.loclCAT.case", "periodcentered.loclCAT"]) {
            rules.push(format!("sub L periodcentered' L by {dot};"));
        } else if cx.has("L_periodcentered_L.loclCAT") {
            rules.push("sub L periodcentered L by L_periodcentered_L.loclCAT;".to_string());
        } else if cx.has("Ldot") {
            rules.push("sub L' periodcentered' L by Ldot;".to_string());
        }
        sets.extend(set("puntvolat", "latn", &["CAT"], rules));
    }

    if cx.has("Iacute") && cx.has("iacute") && cx.has("J") && cx.has("j") {
        let mut rules = Vec::new();
        if cx.has("iacute_j.loclNLD") {
            rules.push("sub iacute j by iacute_j.loclNLD;".to_string());
        } else if cx.has("jacute") {
            rules.push("sub iacute j' by jacute;".to_string());
        }
        if cx.has("Iacute_J.loclNLD") {
            rules.push("sub Iacute J by Iacute_J.loclNLD;".to_string());
        } else if cx.has("Jacute") {
            rules.push("sub Iacute J' by Jacute;".to_string());
        }
        sets.extend(set("ijacute", "latn", &["NLD"], rules));
    }

    let urdu = cx
        .glyphs()
        .iter()
        .filter_map(|g| g.name.strip_suffix(".urdu").filter(|base| !base.is_empty() && cx.has(base)).map(|base| format!("sub {base} by {};", g.name)))
        .collect();
    sets.extend(set("urdu", "arab", &["URD"], urdu));

    sets
}

/// The language tag of a `locl` name segment (`loclSRB` → `SRB`).
fn locl_tag(segment: &str) -> Option<&str> {
    let tag = segment.strip_prefix("locl")?;
    let valid = (3..=4).contains(&tag.len())
        && tag.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && tag.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    valid.then_some(tag)
}

/// The base name and language of a `.loclXXX` glyph name: the name without that segment.
fn split_locl(name: &str) -> Option<(String, &str)> {
    let mut parts = name.split('.');
    let head = parts.next().filter(|h| !h.is_empty())?;
    let mut base = head.to_string();
    let mut tag = None;
    for part in parts {
        match locl_tag(part) {
            Some(t) if tag.is_none() => tag = Some(t),
            _ => {
                base.push('.');
                base.push_str(part);
            }
        }
    }
    tag.map(|t| (base, t))
}

/// Glyph-data script names and their OpenType script tags (the newer tag for Indic scripts).
const SCRIPT_TAGS: &[(&str, &str)] = &[
    ("arabic", "arab"),
    ("armenian", "armn"),
    ("bengali", "bng2"),
    ("cyrillic", "cyrl"),
    ("devanagari", "dev2"),
    ("ethiopic", "ethi"),
    ("georgian", "geor"),
    ("greek", "grek"),
    ("gujarati", "gjr2"),
    ("gurmukhi", "gur2"),
    ("han", "hani"),
    ("hangul", "hang"),
    ("hebrew", "hebr"),
    ("kana", "kana"),
    ("kannada", "knd2"),
    ("khmer", "khmr"),
    ("lao", "lao"),
    ("latin", "latn"),
    ("malayalam", "mlm2"),
    ("mongolian", "mong"),
    ("myanmar", "mym2"),
    ("oriya", "ory2"),
    ("sinhala", "sinh"),
    ("syriac", "syrc"),
    ("tamil", "tml2"),
    ("telugu", "tel2"),
    ("thaana", "thaa"),
    ("thai", "thai"),
    ("tibet", "tibt"),
];

/// The scripts a localised glyph's rule goes under: those of the base glyph's code point, else
/// the stored script of either glyph, else the default script.
fn scripts_of(cx: &AutoContext, name: &str, base: &str) -> Vec<String> {
    let base_glyph = cx.glyph(base);
    if let Some(&cp) = base_glyph.and_then(|g| g.codepoints.first()) {
        let tags: Vec<String> = crate::evaluate::script_tags(cp).iter().map(|t| t.to_string().trim_end().to_string()).collect();
        if !tags.is_empty() {
            return tags;
        }
    }
    let stored = cx.glyph(name).and_then(|g| g.script.as_deref()).or_else(|| base_glyph.and_then(|g| g.script.as_deref()));
    if let Some(tag) = stored.and_then(|s| SCRIPT_TAGS.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)).map(|(_, t)| *t)) {
        return vec![tag.to_string()];
    }
    vec!["DFLT".to_string()]
}

/// One set per (scripts, language) of the `.loclXXX` glyphs, in the order of their first glyph.
fn suffix_sets(cx: &AutoContext) -> Vec<RuleSet> {
    let mut sets: Vec<RuleSet> = Vec::new();
    for glyph in cx.glyphs() {
        if BUILT_IN_NAMES.contains(&glyph.name.as_str()) {
            continue;
        }
        let Some((base, tag)) = split_locl(&glyph.name) else { continue };
        if !cx.has(&base) {
            continue;
        }
        let scripts = scripts_of(cx, &glyph.name, &base);
        let rule = format!("sub {base} by {};", glyph.name);
        match sets.iter_mut().find(|s| s.scripts == scripts && s.languages.first().map(String::as_str) == Some(tag)) {
            Some(existing) => existing.rules.push(rule),
            None => {
                let lookup = format!("locl_{}_{tag}", scripts.first().map(String::as_str).unwrap_or("DFLT"));
                sets.push(RuleSet { lookup, scripts, languages: vec![tag.to_string()], rules: vec![rule] });
            }
        }
    }
    sets
}

/// Appends `text`, refusing code beyond [`MAX_GENERATED_BYTES`].
fn push(out: &mut String, text: &str) -> Result<(), FeatureError> {
    if out.len().saturating_add(text.len()) > MAX_GENERATED_BYTES {
        return Err(FeatureError::Limit { what: "bytes of generated locl code", limit: MAX_GENERATED_BYTES });
    }
    out.push_str(text);
    Ok(())
}

/// The feature code: a `script` section per script in order of first use, each lookup defined
/// where it first appears and referenced in later scripts.
fn write(sets: &[RuleSet]) -> Result<String, FeatureError> {
    let mut scripts: Vec<&str> = Vec::new();
    for script in sets.iter().flat_map(|s| s.scripts.iter()) {
        if !scripts.contains(&script.as_str()) {
            scripts.push(script.as_str());
        }
    }
    let mut defined = vec![false; sets.len()];
    let mut out = String::new();
    for (i, script) in scripts.iter().enumerate() {
        if i > 0 {
            push(&mut out, "\n")?;
        }
        push(&mut out, &format!("script {script};\n"))?;
        for (set, done) in sets.iter().zip(defined.iter_mut()) {
            if !set.scripts.iter().any(|s| s == script) {
                continue;
            }
            push(&mut out, &format!("\nlanguage {};\n", set.languages.join(" ")))?;
            if *done {
                push(&mut out, &format!("lookup {};\n", set.lookup))?;
                continue;
            }
            push(&mut out, &format!("lookup {} {{\n", set.lookup))?;
            for rule in &set.rules {
                push(&mut out, &format!("\t{rule}\n"))?;
            }
            push(&mut out, &format!("}} {};\n", set.lookup))?;
            *done = true;
        }
    }
    Ok(out)
}

/// The (script, language) pairs the font's `locl` features register rules for, in order of
/// first appearance: what Localized Forms offers in Edit View. Disabled entries are left out;
/// `dflt` is not a language here. A `language` statement before any `script` statement counts
/// under `DFLT`. At most [`MAX_LOCL_LANGUAGES`] pairs.
pub fn locl_languages(features: &Features) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for entry in features.features.iter().filter(|e| e.name == "locl" && !e.disabled) {
        let mut script = "DFLT".to_string();
        for statement in statements(&entry.code) {
            let mut words = statement.split_whitespace();
            match words.next() {
                Some("script") => {
                    if let Some(tag) = words.next() {
                        script = tag.to_string();
                    }
                }
                Some("language") => {
                    for language in words.take_while(|w| !matches!(*w, "exclude_dflt" | "include_dflt" | "excludeDFLT" | "includeDFLT" | "required"))
                    {
                        if language == "dflt" || out.iter().any(|(s, l)| *s == script && l == language) {
                            continue;
                        }
                        if out.len() >= MAX_LOCL_LANGUAGES {
                            return out;
                        }
                        out.push((script.clone(), language.to_string()));
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// The statements of feature code: comments removed, split at `;`, `{` and `}`.
fn statements(code: &str) -> impl Iterator<Item = &str> {
    code.lines()
        .map(|line| line.split('#').next().unwrap_or(""))
        .flat_map(|line| line.split([';', '{', '}']))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::Snippet;

    use super::*;
    use crate::auto::AutoGlyph;

    fn glyph(name: &str) -> AutoGlyph {
        let codepoints = match name.chars().collect::<Vec<_>>().as_slice() {
            [c] => vec![*c as u32],
            _ => Vec::new(),
        };
        AutoGlyph { name: name.into(), codepoints, ..AutoGlyph::default() }
    }

    fn cx(names: &[&str]) -> AutoContext {
        let mut glyphs: Vec<AutoGlyph> = names.iter().map(|n| glyph(n)).collect();
        for g in &mut glyphs {
            match g.name.as_str() {
                "periodcentered" => g.codepoints = vec![0xB7],
                "Scedilla" => g.codepoints = vec![0x15E],
                "be-cy" => g.codepoints = vec![0x431],
                "four-persian" => g.codepoints = vec![0x6F4],
                _ => {}
            }
        }
        AutoContext::new(glyphs, Arc::new(Features::default())).unwrap()
    }

    fn locl(names: &[&str]) -> Option<String> {
        let entries = LOCL.generate(&cx(names)).unwrap();
        assert!(entries.len() <= 1);
        entries.into_iter().next().map(|e| {
            assert_eq!((e.kind, e.name.as_str(), e.optional), (EntryKind::Feature, "locl", false));
            e.code
        })
    }

    /// The glyph set of the corpus's version 4 source with a stored automatic locl entry.
    #[test]
    fn locl_matches_the_version_4_output_for_turkish_romanian_and_catalan() {
        let code = locl(&[
            "L",
            "l",
            "i",
            "Scedilla",
            "Scommaaccent",
            "Tcedilla",
            "Tcommaaccent",
            "idotaccent",
            "scedilla",
            "scommaaccent",
            "tcedilla",
            "tcommaaccent",
            "periodcentered",
            "periodcentered.loclCAT",
            "periodcentered.loclCAT.case",
        ])
        .unwrap();
        let expected = "script latn;\n\nlanguage AZE CRT KAZ TAT TRK;\nlookup idotaccent {\n\tsub i by idotaccent;\n} idotaccent;\n\n\
             language ROM MOL;\nlookup commaaccent {\n\tsub Scedilla by Scommaaccent;\n\tsub scedilla by scommaaccent;\n\t\
             sub Tcedilla by Tcommaaccent;\n\tsub tcedilla by tcommaaccent;\n} commaaccent;\n\n\
             language CAT;\nlookup puntvolat {\n\tsub l periodcentered' l by periodcentered.loclCAT;\n\t\
             sub L periodcentered' L by periodcentered.loclCAT.case;\n} puntvolat;\n";
        assert_eq!(code, expected);
    }

    /// The italic source's glyph set: no dotted i, the legacy Ldot and ldot; and a missing
    /// Tcedilla leaves its rule out, as in the stored code of the sources without it.
    #[test]
    fn locl_uses_the_legacy_ldot_and_skips_missing_pairs() {
        let code =
            locl(&["L", "l", "Ldot", "ldot", "i", "periodcentered", "Scedilla", "Scommaaccent", "Tcommaaccent", "tcedilla", "tcommaaccent"]).unwrap();
        assert!(!code.contains("idotaccent"), "{code}");
        assert!(code.contains("\tsub Scedilla by Scommaaccent;\n\tsub tcedilla by tcommaaccent;\n}"), "{code}");
        assert!(!code.contains("Tcedilla"), "{code}");
        assert!(code.contains("\tsub l' periodcentered' l by ldot;\n\tsub L' periodcentered' L by Ldot;\n"), "{code}");
        // periodcentered.loclCAT wins over the legacy glyphs, and serves the capitals without a `.case` form.
        let code = locl(&["L", "l", "Ldot", "ldot", "periodcentered", "periodcentered.loclCAT"]).unwrap();
        assert!(code.contains("sub L periodcentered' L by periodcentered.loclCAT;"), "{code}");
        assert!(!code.contains("ldot"), "{code}");
    }

    #[test]
    fn locl_turkish_i_needs_i_and_takes_any_of_the_three_names() {
        assert_eq!(locl(&["idotaccent"]), None);
        assert!(locl(&["i", "i.loclTRK"]).unwrap().contains("\tsub i by i.loclTRK;\n"));
        assert!(locl(&["i", "i.TRK"]).unwrap().contains("\tsub i by i.TRK;\n"));
        // i.loclTRK belongs to the Turkish set: no separate TRK lookup.
        assert!(!locl(&["i", "i.loclTRK"]).unwrap().contains("language TRK;"));
    }

    #[test]
    fn locl_catalan_and_dutch_ligatures() {
        let code = locl(&[
            "L",
            "l",
            "periodcentered",
            "L_periodcentered_L.loclCAT",
            "l_periodcentered_l.loclCAT",
            "J",
            "j",
            "Iacute",
            "iacute",
            "Iacute_J.loclNLD",
            "iacute_j.loclNLD",
        ])
        .unwrap();
        assert!(
            code.contains("\tsub l periodcentered l by l_periodcentered_l.loclCAT;\n\tsub L periodcentered L by L_periodcentered_L.loclCAT;\n"),
            "{code}"
        );
        assert!(
            code.contains(
                "\nlanguage NLD;\nlookup ijacute {\n\tsub iacute j by iacute_j.loclNLD;\n\tsub Iacute J by Iacute_J.loclNLD;\n} ijacute;\n"
            ),
            "{code}"
        );
        // The accented j glyphs, used in context.
        let code = locl(&["J", "j", "Iacute", "iacute", "Jacute", "jacute"]).unwrap();
        assert!(code.contains("\tsub iacute j' by jacute;\n\tsub Iacute J' by Jacute;\n"), "{code}");
        // Without J the Dutch set is not written, and its glyphs never fall to the suffix rule.
        assert_eq!(locl(&["Iacute", "iacute", "j", "Iacute_J", "Iacute_J.loclNLD"]), None);
    }

    #[test]
    fn locl_urdu_figures_go_under_arabic() {
        let code = locl(&["four-persian", "four-persian.urdu", "seven-persian.urdu"]).unwrap();
        assert_eq!(code, "script arab;\n\nlanguage URD;\nlookup urdu {\n\tsub four-persian by four-persian.urdu;\n} urdu;\n");
    }

    #[test]
    fn locl_suffix_rule_per_script_and_language() {
        let names = ["be-cy", "be-cy.loclSRB", "i", "idotaccent", "a", "a.loclDEU", "a.ss01", "a.ss01.loclDEU", "b.loclXX", "c.locl"];
        // The unencoded a.ss01 takes its stored script, so it shares the DEU lookup with a.
        let glyphs = cx(&names)
            .glyphs()
            .iter()
            .cloned()
            .map(|mut g| {
                if g.name.starts_with("a.ss01") {
                    g.script = Some("latin".into());
                }
                g
            })
            .collect();
        let entries = LOCL.generate(&AutoContext::new(glyphs, Arc::new(Features::default())).unwrap()).unwrap();
        let expected = "script latn;\n\nlanguage AZE CRT KAZ TAT TRK;\nlookup idotaccent {\n\tsub i by idotaccent;\n} idotaccent;\n\n\
             language DEU;\nlookup locl_latn_DEU {\n\tsub a by a.loclDEU;\n\tsub a.ss01 by a.ss01.loclDEU;\n} locl_latn_DEU;\n\n\
             script cyrl;\n\nlanguage SRB;\nlookup locl_cyrl_SRB {\n\tsub be-cy by be-cy.loclSRB;\n} locl_cyrl_SRB;\n";
        assert_eq!(entries[0].code, expected);
        // Without a stored script, the unencoded glyph goes under the default script.
        let code = locl(&names).unwrap();
        assert!(
            code.ends_with("\nscript DFLT;\n\nlanguage DEU;\nlookup locl_DFLT_DEU {\n\tsub a.ss01 by a.ss01.loclDEU;\n} locl_DFLT_DEU;\n"),
            "{code}"
        );
    }

    #[test]
    fn locl_nothing_without_triggers() {
        assert_eq!(locl(&["a", "b", "Scedilla", "periodcentered"]), None);
    }

    #[test]
    fn locl_code_compiles_its_lookup_names_once_per_set() {
        // A set under two scripts is defined in the first and referenced in the second.
        let sets = vec![RuleSet {
            lookup: "x".into(),
            scripts: vec!["latn".into(), "cyrl".into()],
            languages: vec!["SRB".into()],
            rules: vec!["sub a by a.loclSRB;".into()],
        }];
        assert_eq!(
            write(&sets).unwrap(),
            "script latn;\n\nlanguage SRB;\nlookup x {\n\tsub a by a.loclSRB;\n} x;\n\nscript cyrl;\n\nlanguage SRB;\nlookup x;\n"
        );
    }

    #[test]
    fn locl_languages_lists_the_language_systems_of_locl() {
        let entry = |code: &str, disabled: bool| Snippet { name: "locl".into(), code: code.into(), disabled, ..Snippet::default() };
        let mut features = Features::default();
        features.features.push(entry(
            "script latn;\n\nlanguage AZE CRT TRK; # Turkic\nlookup idotaccent {\n\tsub i by idotaccent;\n} idotaccent;\nlanguage ROM exclude_dflt;\nlanguage dflt;\n\
             lookup x { script cyrl; language SRB; sub be-cy by be-cy.loclSRB; } x;\nlanguage TRK;",
            false,
        ));
        features.features.push(entry("language DEU;", false));
        features.features.push(entry("script grek; language ELL;", true));
        features.features.push(Snippet { name: "liga".into(), code: "script latn; language NLD;".into(), ..Snippet::default() });
        let pairs = locl_languages(&features);
        let pairs: Vec<(&str, &str)> = pairs.iter().map(|(s, l)| (s.as_str(), l.as_str())).collect();
        assert_eq!(pairs, [("latn", "AZE"), ("latn", "CRT"), ("latn", "TRK"), ("latn", "ROM"), ("cyrl", "SRB"), ("cyrl", "TRK"), ("DFLT", "DEU")]);
    }

    #[test]
    fn locl_names_split_at_the_locl_segment() {
        assert_eq!(split_locl("a.ss01.loclDEU"), Some(("a.ss01".to_string(), "DEU")));
        assert_eq!(split_locl("periodcentered.loclCAT.case"), Some(("periodcentered.case".to_string(), "CAT")));
        assert_eq!(split_locl("a.locldflt"), None);
        assert_eq!(split_locl(".loclDEU"), None);
        assert_eq!(split_locl("a.loclIPPH"), Some(("a".to_string(), "IPPH")));
    }
}
