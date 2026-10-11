//! Ligatures, ordinals and ornaments (M8.11).
//!
//! Rows: `autoFeature.liga`, `dlig`, `hlig`, `rlig`, `ornm`, `ordn`.
//!
//! A ligature is a glyph whose name, before its first dot, joins component names with
//! underscores (`f_f_i`, `lam_alef-ar.fina`), or one of `fi` and `fl`. A script suffix on the
//! last part applies to every part (`lu_lakkhangyao-thai` joins `lu-thai` and
//! `lakkhangyao-thai`). Each ligature goes to one feature, by the first rule that matches:
//!
//! 1. A last suffix `.liga`, `.dlig`, `.rlig`, or `.hlig` and `.hist` (hlig).
//! 2. The Arabic lam-alef set and its `.fina` forms: rlig.
//! 3. A `longs` component: hlig.
//! 4. The handbook's default list (`f_f_i`, `f_f_l`, `f_f`, `fi`, `fl`, and the two Thai
//!    lakkhangyao ligatures): liga.
//! 5. Everything else: dlig.
//!
//! The rules of a feature come after `lookupflag IgnoreMarks;`, longest component sequence
//! first, then in glyph order, as the reference's stored code has them.

use std::collections::HashSet;

use super::fractions::FIGURES;
use super::{AutoContext, AutoEntry, AutoGlyph, Generator, MAX_GENERATED_BYTES};
use crate::FeatureError;
use crate::source_map::EntryKind;

/// This module's generators.
pub(super) fn generators() -> Vec<&'static dyn Generator> {
    vec![&LIGATURES, &ORDINALS, &ORNAMENTS]
}

/// Most components a ligature name may join; longer names are not treated as ligatures.
const MAX_COMPONENTS: usize = 64;

/// The ligatures liga takes without a `.liga` suffix.
const DEFAULT_LIGA: [&str; 7] = ["f_f_i", "f_f_l", "f_f", "fi", "fl", "lu_lakkhangyao-thai", "ru_lakkhangyao-thai"];

/// The Arabic ligatures rlig takes, with or without `.fina`.
const LAM_ALEF: [&str; 5] = ["lam_alef-ar", "lam_alefHamzaabove-ar", "lam_alefHamzabelow-ar", "lam_alefMadda-ar", "lam_alefWasla-ar"];

/// Scripts whose letters take positional forms (`.init`, `.medi`, `.fina`).
const JOINING_SCRIPTS: [&str; 6] = ["arabic", "syriac", "nko", "mongolian", "mandaic", "adlam"];

/// The name suffixes of those scripts (`beh-ar`).
const JOINING_NAME_SUFFIXES: [&str; 6] = ["ar", "syriac", "nko", "mongolian", "mandaic", "adlam"];

/// Generated code that refuses to grow past [`MAX_GENERATED_BYTES`].
#[derive(Default)]
pub(super) struct Code(String);

impl Code {
    /// Appends `text` and a line break.
    pub(super) fn line(&mut self, text: &str) -> Result<(), FeatureError> {
        if self.0.len().saturating_add(text.len()).saturating_add(1) > MAX_GENERATED_BYTES {
            return Err(FeatureError::Limit { what: "bytes of generated code in one entry", limit: MAX_GENERATED_BYTES });
        }
        self.0.push_str(text);
        self.0.push('\n');
        Ok(())
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn into_string(self) -> String {
        self.0
    }

    /// The feature entry holding this code.
    pub(super) fn feature(self, tag: &str) -> AutoEntry {
        AutoEntry { kind: EntryKind::Feature, name: tag.to_string(), code: self.0, optional: false }
    }
}

/// The four ligature features, in the order they are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LigatureTag {
    Liga,
    Dlig,
    Hlig,
    Rlig,
}

impl LigatureTag {
    const ALL: [LigatureTag; 4] = [LigatureTag::Liga, LigatureTag::Dlig, LigatureTag::Hlig, LigatureTag::Rlig];

    fn tag(self) -> &'static str {
        match self {
            LigatureTag::Liga => "liga",
            LigatureTag::Dlig => "dlig",
            LigatureTag::Hlig => "hlig",
            LigatureTag::Rlig => "rlig",
        }
    }

    fn from_suffix(suffix: &str) -> Option<Self> {
        match suffix {
            "liga" => Some(LigatureTag::Liga),
            "dlig" => Some(LigatureTag::Dlig),
            "hlig" | "hist" => Some(LigatureTag::Hlig),
            "rlig" => Some(LigatureTag::Rlig),
            _ => None,
        }
    }
}

/// Where a ligature of a joining script stands in a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Position {
    Isolated,
    Initial,
    Medial,
    Final,
}

impl Position {
    /// The positional suffix of component `i` of `n`.
    fn component_suffix(self, i: usize, n: usize) -> &'static str {
        let first = i == 0;
        let last = i.saturating_add(1) == n;
        match self {
            Position::Isolated if first => "init",
            Position::Isolated | Position::Final if last => "fina",
            Position::Initial if first => "init",
            _ => "medi",
        }
    }
}

/// One ligature rule: the feature it goes to and the glyphs it joins.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Ligature {
    tag: LigatureTag,
    components: Vec<String>,
}

/// The ligature rule `glyph` calls for, if any: its components must all be in the font.
fn ligature(cx: &AutoContext, glyph: &AutoGlyph) -> Option<Ligature> {
    if glyph.category.as_deref() == Some("Mark") {
        return None;
    }
    let (base, rest) = match glyph.name.split_once('.') {
        Some((base, rest)) => (base, Some(rest)),
        None => (glyph.name.as_str(), None),
    };
    let mut suffixes: Vec<&str> = rest.map(|r| r.split('.').collect()).unwrap_or_default();
    let routed = suffixes.last().and_then(|s| LigatureTag::from_suffix(s));
    if routed.is_some() {
        suffixes.pop();
    }
    // Localised ligatures belong to locl; an empty suffix is a malformed name.
    if suffixes.iter().any(|s| s.is_empty() || s.starts_with("locl")) {
        return None;
    }
    let parts = name_parts(base)?;
    let tag = routed.unwrap_or_else(|| {
        let plain_or_final = matches!(suffixes.as_slice(), [] | ["fina"]);
        if plain_or_final && LAM_ALEF.contains(&base) {
            LigatureTag::Rlig
        } else if parts.iter().any(|p| p == "longs") {
            LigatureTag::Hlig
        } else if DEFAULT_LIGA.contains(&base) {
            LigatureTag::Liga
        } else {
            LigatureTag::Dlig
        }
    });

    let position = if is_joining(glyph, base) {
        match suffixes.as_slice() {
            [] => Some(Position::Isolated),
            ["init"] => Some(Position::Initial),
            ["medi"] => Some(Position::Medial),
            ["fina"] => Some(Position::Final),
            _ => None,
        }
    } else {
        None
    };
    let n = parts.len();
    let mut components = Vec::with_capacity(n);
    for (i, part) in parts.into_iter().enumerate() {
        let component = match position {
            Some(position) => {
                let positional = format!("{part}.{}", position.component_suffix(i, n));
                if cx.has(&positional) { positional } else { part }
            }
            None if suffixes.is_empty() => part,
            None => format!("{part}.{}", suffixes.join(".")),
        };
        if !cx.has(&component) {
            return None;
        }
        components.push(component);
    }
    let all_marks = components.iter().all(|c| cx.glyph(c).and_then(|g| g.category.as_deref()) == Some("Mark"));
    if all_marks {
        return None;
    }
    Some(Ligature { tag, components })
}

/// The component names a ligature's base name joins, with its script suffix spread to each
/// part; `None` when the name is not a ligature.
fn name_parts(base: &str) -> Option<Vec<String>> {
    match base {
        "fi" => return Some(vec!["f".to_string(), "i".to_string()]),
        "fl" => return Some(vec!["f".to_string(), "l".to_string()]),
        _ => {}
    }
    if !base.contains('_') {
        return None;
    }
    let parts: Vec<&str> = base.split('_').collect();
    if parts.len() > MAX_COMPONENTS || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let script = parts.last().and_then(|p| p.rsplit_once('-')).map(|(_, s)| s).filter(|s| !s.is_empty());
    Some(
        parts
            .into_iter()
            .map(|p| match script {
                Some(script) if !p.contains('-') => format!("{p}-{script}"),
                _ => p.to_string(),
            })
            .collect(),
    )
}

fn is_joining(glyph: &AutoGlyph, base: &str) -> bool {
    match glyph.script.as_deref() {
        Some(script) => JOINING_SCRIPTS.contains(&script),
        None => base.rsplit_once('-').is_some_and(|(_, s)| JOINING_NAME_SUFFIXES.contains(&s)),
    }
}

/// liga, dlig, hlig and rlig.
struct Ligatures;

static LIGATURES: Ligatures = Ligatures;

impl Generator for Ligatures {
    fn names(&self) -> &'static [&'static str] {
        &["liga", "dlig", "hlig", "rlig"]
    }

    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError> {
        let found: Vec<(&AutoGlyph, Ligature)> = cx.glyphs().iter().filter_map(|g| ligature(cx, g).map(|l| (g, l))).collect();
        let mut entries = Vec::new();
        for tag in LigatureTag::ALL {
            let mut rules: Vec<&(&AutoGlyph, Ligature)> = found.iter().filter(|(_, l)| l.tag == tag).collect();
            // Stable: equal lengths keep the glyph order.
            rules.sort_by_key(|(_, l)| std::cmp::Reverse(l.components.len()));
            let mut seen: HashSet<&[String]> = HashSet::new();
            let mut code = Code::default();
            for (glyph, l) in rules {
                // A second rule with the same input could never apply.
                if !seen.insert(l.components.as_slice()) {
                    continue;
                }
                if code.is_empty() {
                    code.line("lookupflag IgnoreMarks;")?;
                }
                code.line(&format!("sub {} by {};", l.components.join(" "), glyph.name))?;
            }
            if !code.is_empty() {
                entries.push(code.feature(tag.tag()));
            }
        }
        Ok(entries)
    }
}

/// ordn: a and o after figures become the ordinal indicators, and `N o period` becomes numero.
struct Ordinals;

static ORDINALS: Ordinals = Ordinals;

impl Generator for Ordinals {
    fn names(&self) -> &'static [&'static str] {
        &["ordn"]
    }

    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError> {
        let required = ["numero", "ordfeminine", "ordmasculine"];
        if !FIGURES.iter().chain(required.iter()).all(|n| cx.has(n)) {
            return Ok(Vec::new());
        }
        let figures = FIGURES.join(" ");
        let mut code = Code::default();
        for (letters, indicator) in [(["A", "a"], "ordfeminine"), (["O", "o"], "ordmasculine")] {
            let present: Vec<&str> = letters.into_iter().filter(|l| cx.has(l)).collect();
            if !present.is_empty() {
                code.line(&format!("sub [{figures}] [{}]' by {indicator};", present.join(" ")))?;
            }
        }
        if ["N", "o", "period"].iter().all(|n| cx.has(n)) {
            code.line("sub N o period by numero;")?;
        }
        Ok(if code.is_empty() { Vec::new() } else { vec![code.feature("ordn")] })
    }
}

/// ornm: bullet offers every ornament, and each letter turns into its own.
struct Ornaments;

static ORNAMENTS: Ornaments = Ornaments;

impl Generator for Ornaments {
    fn names(&self) -> &'static [&'static str] {
        &["ornm"]
    }

    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError> {
        if !cx.has("bullet") {
            return Ok(Vec::new());
        }
        let ornaments: Vec<(&str, &str)> = cx
            .glyphs()
            .iter()
            .filter_map(|g| {
                let base = g.name.strip_suffix(".ornm")?;
                let mut chars = base.chars();
                let letter = chars.next()?;
                (letter.is_ascii_alphabetic() && chars.next().is_none()).then_some((base, g.name.as_str()))
            })
            .collect();
        if ornaments.is_empty() {
            return Ok(Vec::new());
        }
        let mut code = Code::default();
        let all: Vec<&str> = ornaments.iter().map(|(_, o)| *o).collect();
        code.line(&format!("sub bullet from [{}];", all.join(" ")))?;
        for (base, ornament) in &ornaments {
            if cx.has(base) {
                code.line(&format!("sub {base} by {ornament};"))?;
            }
        }
        Ok(vec![code.feature("ornm")])
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::Features;

    use super::*;

    fn glyph(name: &str) -> AutoGlyph {
        AutoGlyph { name: name.to_string(), ..AutoGlyph::default() }
    }

    fn context(glyphs: Vec<AutoGlyph>) -> AutoContext {
        AutoContext::new(glyphs, Arc::new(Features::default())).unwrap()
    }

    fn names(names: &[&str]) -> AutoContext {
        context(names.iter().map(|n| glyph(n)).collect())
    }

    fn code_of<'a>(entries: &'a [AutoEntry], tag: &str) -> Option<&'a str> {
        entries.iter().find(|e| e.name == tag && e.kind == EntryKind::Feature).map(|e| e.code.as_str())
    }

    /// Oswald's glyph order and its stored liga and dlig code; the mark ligatures it also has
    /// get no rule.
    #[test]
    fn oswald_liga_and_dlig_match_the_stored_code() {
        let mut glyphs: Vec<AutoGlyph> =
            ["f", "i", "j", "l", "ij", "longs", "f_f", "f_f_i", "f_f_ij", "f_f_l", "f_ij", "fi", "fl"].iter().map(|n| glyph(n)).collect();
        for mark in ["brevecomb", "acutecomb", "brevecomb_acutecomb"] {
            glyphs.push(AutoGlyph { category: Some("Mark".to_string()), ..glyph(mark) });
        }
        let entries = LIGATURES.generate(&context(glyphs)).unwrap();
        assert_eq!(
            code_of(&entries, "liga"),
            Some("lookupflag IgnoreMarks;\nsub f f i by f_f_i;\nsub f f l by f_f_l;\nsub f f by f_f;\nsub f i by fi;\nsub f l by fl;\n")
        );
        assert_eq!(code_of(&entries, "dlig"), Some("lookupflag IgnoreMarks;\nsub f f ij by f_f_ij;\nsub f ij by f_ij;\n"));
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn ligatures_are_routed_by_suffix_longs_and_the_lam_alef_set() {
        let cx = names(&[
            "f",
            "b",
            "i",
            "s",
            "t",
            "longs",
            "odieresis",
            "f_b.liga",
            "f_f_i.dlig",
            "longs_t",
            "s_t.hist",
            "f_odieresis",
            "f_i.rlig",
            "f_x",
        ]);
        let entries = LIGATURES.generate(&cx).unwrap();
        assert_eq!(code_of(&entries, "liga"), Some("lookupflag IgnoreMarks;\nsub f b by f_b.liga;\n"));
        assert_eq!(code_of(&entries, "dlig"), Some("lookupflag IgnoreMarks;\nsub f f i by f_f_i.dlig;\nsub f odieresis by f_odieresis;\n"));
        assert_eq!(code_of(&entries, "hlig"), Some("lookupflag IgnoreMarks;\nsub longs t by longs_t;\nsub s t by s_t.hist;\n"));
        assert_eq!(code_of(&entries, "rlig"), Some("lookupflag IgnoreMarks;\nsub f i by f_i.rlig;\n"));
    }

    #[test]
    fn script_suffixes_and_arabic_positional_forms_reach_the_components() {
        let mut glyphs: Vec<AutoGlyph> = ["lu-thai", "lakkhangyao-thai", "lu_lakkhangyao-thai"].iter().map(|n| glyph(n)).collect();
        for name in ["lam-ar", "lam-ar.init", "lam-ar.medi", "alef-ar", "alef-ar.fina", "lam_alef-ar", "lam_alef-ar.fina"] {
            glyphs.push(AutoGlyph { script: Some("arabic".to_string()), ..glyph(name) });
        }
        let entries = LIGATURES.generate(&context(glyphs)).unwrap();
        assert_eq!(code_of(&entries, "liga"), Some("lookupflag IgnoreMarks;\nsub lu-thai lakkhangyao-thai by lu_lakkhangyao-thai;\n"));
        assert_eq!(
            code_of(&entries, "rlig"),
            Some("lookupflag IgnoreMarks;\nsub lam-ar.init alef-ar.fina by lam_alef-ar;\nsub lam-ar.medi alef-ar.fina by lam_alef-ar.fina;\n")
        );
    }

    /// Without positional forms both lam-alef glyphs would need the same input; the first wins.
    #[test]
    fn a_repeated_input_sequence_keeps_the_first_rule() {
        let cx = names(&["lam-ar", "alef-ar", "lam_alef-ar", "lam_alef-ar.fina"]);
        let entries = LIGATURES.generate(&cx).unwrap();
        assert_eq!(code_of(&entries, "rlig"), Some("lookupflag IgnoreMarks;\nsub lam-ar alef-ar by lam_alef-ar;\n"));
    }

    #[test]
    fn suffixed_ligatures_take_suffixed_components_and_localised_ones_are_left_out() {
        let cx = names(&[
            "f",
            "i",
            "f.sc",
            "i.sc",
            "f_i.sc",
            "f_i.ss01",
            "f.ss01",
            "L",
            "periodcentered",
            "L_periodcentered_L.loclCAT",
            "_part.stem",
            "a__b",
        ]);
        let entries = LIGATURES.generate(&cx).unwrap();
        assert_eq!(code_of(&entries, "dlig"), Some("lookupflag IgnoreMarks;\nsub f.sc i.sc by f_i.sc;\n"));
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn no_ligature_glyphs_give_no_entries() {
        assert!(LIGATURES.generate(&names(&["a", "b", "f", "i"])).unwrap().is_empty());
    }

    /// The three lines every corpus source with ordn stores.
    #[test]
    fn ordn_matches_the_stored_code() {
        let mut list: Vec<&str> = FIGURES.to_vec();
        list.extend(["A", "N", "O", "a", "o", "period", "numero", "ordfeminine", "ordmasculine"]);
        let entries = ORDINALS.generate(&names(&list)).unwrap();
        assert_eq!(
            code_of(&entries, "ordn"),
            Some(
                "sub [zero one two three four five six seven eight nine] [A a]' by ordfeminine;\n\
                 sub [zero one two three four five six seven eight nine] [O o]' by ordmasculine;\n\
                 sub N o period by numero;\n"
            )
        );
        list.retain(|n| *n != "numero");
        assert!(ORDINALS.generate(&names(&list)).unwrap().is_empty());
    }

    #[test]
    fn ornm_offers_every_ornament_from_bullet() {
        let cx = names(&["A", "B", "bullet", "A.ornm", "B.ornm", "c.ornm", "AB.ornm"]);
        let entries = ORNAMENTS.generate(&cx).unwrap();
        assert_eq!(code_of(&entries, "ornm"), Some("sub bullet from [A.ornm B.ornm c.ornm];\nsub A by A.ornm;\nsub B by B.ornm;\n"));
        assert!(ORNAMENTS.generate(&names(&["A", "A.ornm"])).unwrap().is_empty());
    }
}
