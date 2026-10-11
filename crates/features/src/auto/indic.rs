//! The Indic features (M8.13).
//!
//! Rows: `autoFeature.abvs`, `akhn`, `blwf`, `blws`, `cjct`, `half`, `nukt`, `pref`, `pres`, `psts`, `rkrf`, `rphf`, `vatu`;
//! the `Keep UI-Font Bounding Box` parameter.
//!
//! The rules come from the glyph names alone, in the naming of the glyph database: a script
//! suffix (`ka-deva`), half forms without the inherent `a` (`k-deva`), conjuncts joining their
//! parts with underscores (`k_ya-deva`), nukta forms ending in `Nukta` (`kaNukta-deva`) and below
//! forms suffixed `.below`. Each rule is one ligature line, written in glyph order:
//!
//! | Feature | Written for | Rule |
//! |---|---|---|
//! | nukt | `XNukta-S` with `X-S` and `nukta-S` | `sub X-S nukta-S by XNukta-S;` |
//! | akhn | `k_ssa-S`, `j_nya-S` | `sub ka-S halant-S ssa-S by k_ssa-S;` |
//! | rphf | `reph-S` with `ra-S` and the halant | `sub ra-S halant-S by reph-S;` |
//! | rkrf | `X_ra-S` or `Xa_rakar-S`, Devanagari, Gujarati and Malayalam | `sub Xa-S halant-S ra-S by X_ra-S;` |
//! | blwf | `X-S.below`; `rakar-S` | `sub halant-S X-S by X-S.below;`, `sub halant-S ra-S by rakar-S;` |
//! | half | `X-S` where `Xa-S` exists | `sub Xa-S halant-S by X-S;` |
//! | cjct | other conjuncts `X_Ya-S` | `sub X-S Ya-S by X_Ya-S;` (half forms, else consonant and halant) |
//!
//! abvs, blws, pref, pres, psts and vatu need data the handbook does not give; [`not_generated`]
//! names them with the reason for Update's report. The `Keep UI-Font Bounding Box` parameter
//! changes how the reference stacks Oriya conjuncts; the cjct written here never stacks, so the
//! parameter has nothing to change.

use super::ccmp::Code;
use super::{AutoContext, AutoEntry, AutoGlyph, Generator};
use crate::FeatureError;
use crate::source_map::EntryKind;

/// Most parts one conjunct name may join; longer names are not conjuncts.
const MAX_PARTS: usize = 16;

/// A script with Indic shaping, as the glyph names spell it.
struct Script {
    /// The name suffix after the hyphen (`deva`).
    suffix: &'static str,
    /// The virama's glyph name.
    halant: &'static str,
    /// Whether rakar ligatures go into rkrf (the handbook's Devanagari, Gujarati and Malayalam).
    rakar_forms: bool,
}

static SCRIPTS: [Script; 9] = [
    Script { suffix: "deva", halant: "halant-deva", rakar_forms: true },
    Script { suffix: "beng", halant: "halant-beng", rakar_forms: false },
    Script { suffix: "gurmukhi", halant: "halant-gurmukhi", rakar_forms: false },
    Script { suffix: "gujarati", halant: "halant-gujarati", rakar_forms: true },
    Script { suffix: "oriya", halant: "halant-oriya", rakar_forms: false },
    Script { suffix: "tamil", halant: "pulli-tamil", rakar_forms: false },
    Script { suffix: "telugu", halant: "halant-telugu", rakar_forms: false },
    Script { suffix: "kannada", halant: "halant-kannada", rakar_forms: false },
    Script { suffix: "malayalam", halant: "halant-malayalam", rakar_forms: true },
];

/// The akhand conjuncts, as (half part, last consonant).
const AKHANDS: [(&str, &str); 2] = [("k", "ssa"), ("j", "nya")];

/// The Indic features Update recognises and does not write, with the reason.
const NOT_GENERATED: [(&str, &str); 6] = [
    ("abvs", "Not generated: the base and mark pairs it joins are not documented. Write the feature by hand."),
    ("blws", "Not generated: its rules depend on the script and are not documented. Write the feature by hand."),
    ("pref", "Not generated: the letter combinations it forms are not documented. Write the feature by hand."),
    ("pres", "Not generated: how a numbered matra width is matched to each conjunct is not documented. Write the feature by hand."),
    ("psts", "Not generated: its rules depend on the script and are not documented. Write the feature by hand."),
    ("vatu", "Not generated: the rakar lookups it is derived from are not documented. Write the feature by hand."),
];

/// A glyph name taken apart: `k_ya-deva.below` is stem `k_ya`, script Devanagari, suffix `.below`.
struct Name<'a> {
    stem: &'a str,
    script: &'static Script,
    suffix: &'a str,
}

fn parse(name: &str) -> Option<Name<'_>> {
    let (base, suffix) = match name.find('.') {
        Some(dot) => (name.get(..dot)?, name.get(dot..)?),
        None => (name, ""),
    };
    let (stem, tail) = base.rsplit_once('-')?;
    let script = SCRIPTS.iter().find(|s| s.suffix == tail)?;
    (!stem.is_empty()).then_some(Name { stem, script, suffix })
}

/// The glyph `stem-S`, when the font has it.
fn named(cx: &AutoContext, stem: &str, script: &Script) -> Option<String> {
    let name = format!("{stem}-{}", script.suffix);
    cx.has(&name).then_some(name)
}

/// The full consonant of a conjunct part: `ka-S` for `k` or `ka`.
fn consonant(cx: &AutoContext, part: &str, script: &Script) -> Option<String> {
    if part.ends_with('a')
        && let Some(name) = named(cx, part, script)
    {
        return Some(name);
    }
    named(cx, &format!("{part}a"), script)
}

fn halant(cx: &AutoContext, script: &Script) -> Option<&'static str> {
    cx.has(script.halant).then_some(script.halant)
}

/// The parts of a conjunct name, two or more.
fn parts(stem: &str) -> Option<Vec<&str>> {
    if !stem.contains('_') {
        return None;
    }
    let parts: Vec<&str> = stem.split('_').take(MAX_PARTS + 1).collect();
    (parts.len() <= MAX_PARTS && parts.iter().all(|p| !p.is_empty())).then_some(parts)
}

fn is_akhand(stem: &str) -> bool {
    AKHANDS.iter().any(|(h, c)| stem.strip_prefix(h).and_then(|r| r.strip_prefix('_')) == Some(*c))
}

/// The consonants before the last part of a rakar ligature (`k` of `k_ra`, `ka` of `ka_rakar`).
fn rakar_lead<'a>(stem: &'a str, script: &Script) -> Option<Vec<&'a str>> {
    if !script.rakar_forms {
        return None;
    }
    let mut parts = parts(stem)?;
    let last = parts.pop()?;
    (last == "ra" || last == "rakar").then_some(parts)
}

/// One generator: a rule asked of every glyph in order, each answer one line of code.
struct Indic {
    names: &'static [&'static str],
    rule: fn(&AutoContext, &AutoGlyph, &Name<'_>) -> Option<String>,
}

static NUKT: Indic = Indic { names: &["nukt"], rule: nukt };
static AKHN: Indic = Indic { names: &["akhn"], rule: akhn };
static RPHF: Indic = Indic { names: &["rphf"], rule: rphf };
static RKRF: Indic = Indic { names: &["rkrf"], rule: rkrf };
static BLWF: Indic = Indic { names: &["blwf"], rule: blwf };
static HALF: Indic = Indic { names: &["half"], rule: half };
static CJCT: Indic = Indic { names: &["cjct"], rule: cjct };

/// This module's generators, in the order an Indic shaper applies their features.
pub(super) fn generators() -> Vec<&'static dyn Generator> {
    vec![&NUKT, &AKHN, &RPHF, &RKRF, &BLWF, &HALF, &CJCT]
}

/// The Indic features Update does not write, with the reason, when `cx` has glyphs of a script
/// with Indic shaping; none otherwise. [`super::update`] adds them to its report.
#[allow(dead_code, reason = "called by Update's report, written with the automatic feature framework")]
pub(super) fn not_generated(cx: &AutoContext) -> Vec<(String, String)> {
    if !cx.glyphs().iter().any(|g| parse(&g.name).is_some()) {
        return Vec::new();
    }
    NOT_GENERATED.iter().map(|(tag, why)| ((*tag).to_owned(), (*why).to_owned())).collect()
}

impl Generator for Indic {
    fn names(&self) -> &'static [&'static str] {
        self.names
    }

    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError> {
        let Some(&tag) = self.names.first() else {
            return Ok(Vec::new());
        };
        let mut code = Code::default();
        for glyph in cx.glyphs() {
            let Some(name) = parse(&glyph.name) else {
                continue;
            };
            if let Some(line) = (self.rule)(cx, glyph, &name) {
                code.push(&line)?;
            }
        }
        if code.0.is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![AutoEntry { kind: EntryKind::Feature, name: tag.to_owned(), code: code.0, optional: false }])
    }
}

fn nukt(cx: &AutoContext, glyph: &AutoGlyph, n: &Name<'_>) -> Option<String> {
    let base = n.stem.strip_suffix("Nukta").filter(|b| !b.is_empty() && n.suffix.is_empty())?;
    let plain = named(cx, base, n.script)?;
    let nukta = named(cx, "nukta", n.script)?;
    Some(format!("sub {plain} {nukta} by {};\n", glyph.name))
}

fn akhn(cx: &AutoContext, glyph: &AutoGlyph, n: &Name<'_>) -> Option<String> {
    if !n.suffix.is_empty() || !is_akhand(n.stem) {
        return None;
    }
    let [first, last] = parts(n.stem)?.try_into().ok()?;
    let first = consonant(cx, first, n.script)?;
    let last = named(cx, last, n.script)?;
    let halant = halant(cx, n.script)?;
    Some(format!("sub {first} {halant} {last} by {};\n", glyph.name))
}

fn rphf(cx: &AutoContext, glyph: &AutoGlyph, n: &Name<'_>) -> Option<String> {
    if n.stem != "reph" || !n.suffix.is_empty() {
        return None;
    }
    let ra = named(cx, "ra", n.script)?;
    let halant = halant(cx, n.script)?;
    Some(format!("sub {ra} {halant} by {};\n", glyph.name))
}

fn rkrf(cx: &AutoContext, glyph: &AutoGlyph, n: &Name<'_>) -> Option<String> {
    if !n.suffix.is_empty() {
        return None;
    }
    let lead = rakar_lead(n.stem, n.script)?;
    let halant = halant(cx, n.script)?;
    let ra = named(cx, "ra", n.script)?;
    let mut input = Vec::with_capacity(lead.len().saturating_mul(2).saturating_add(1));
    for part in lead {
        input.push(consonant(cx, part, n.script)?);
        input.push(halant.to_owned());
    }
    input.push(ra);
    Some(format!("sub {} by {};\n", input.join(" "), glyph.name))
}

fn blwf(cx: &AutoContext, glyph: &AutoGlyph, n: &Name<'_>) -> Option<String> {
    let halant = halant(cx, n.script)?;
    if n.suffix == ".below" {
        let plain = named(cx, n.stem, n.script)?;
        return Some(format!("sub {halant} {plain} by {};\n", glyph.name));
    }
    if n.stem == "rakar" && n.suffix.is_empty() {
        let ra = named(cx, "ra", n.script)?;
        // A `ra-S.below` already takes halant and ra.
        if cx.has(&format!("{ra}.below")) {
            return None;
        }
        return Some(format!("sub {halant} {ra} by {};\n", glyph.name));
    }
    None
}

fn half(cx: &AutoContext, glyph: &AutoGlyph, n: &Name<'_>) -> Option<String> {
    if !n.suffix.is_empty() || n.stem.ends_with('a') || glyph.category.as_deref() == Some("Mark") {
        return None;
    }
    let full = named(cx, &format!("{}a", n.stem), n.script)?;
    let halant = halant(cx, n.script)?;
    Some(format!("sub {full} {halant} by {};\n", glyph.name))
}

fn cjct(cx: &AutoContext, glyph: &AutoGlyph, n: &Name<'_>) -> Option<String> {
    if !n.suffix.is_empty() || is_akhand(n.stem) || rakar_lead(n.stem, n.script).is_some() {
        return None;
    }
    let mut parts = parts(n.stem)?;
    let last = parts.pop().filter(|p| p.ends_with('a'))?;
    let last = named(cx, last, n.script)?;
    let mut input = Vec::with_capacity(parts.len().saturating_mul(2).saturating_add(1));
    for part in parts {
        if let Some(half) = named(cx, part, n.script).filter(|_| !part.ends_with('a')) {
            input.push(half);
        } else {
            input.push(consonant(cx, part, n.script)?);
            input.push(halant(cx, n.script)?.to_owned());
        }
    }
    input.push(last);
    Some(format!("sub {} by {};\n", input.join(" "), glyph.name))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn glyph(name: &str, category: &str) -> AutoGlyph {
        AutoGlyph {
            name: name.to_owned(),
            codepoints: Vec::new(),
            category: Some(category.to_owned()),
            sub_category: None,
            script: Some("devanagari".to_owned()),
        }
    }

    /// A Devanagari font built in code, in the glyph database's naming.
    fn devanagari() -> AutoContext {
        let glyphs = [
            ("ka-deva", "Letter"),
            ("ja-deva", "Letter"),
            ("nya-deva", "Letter"),
            ("ssa-deva", "Letter"),
            ("ya-deva", "Letter"),
            ("ra-deva", "Letter"),
            ("halant-deva", "Mark"),
            ("nukta-deva", "Mark"),
            ("k-deva", "Letter"),
            ("kaNukta-deva", "Letter"),
            ("k_ssa-deva", "Letter"),
            ("j_nya-deva", "Letter"),
            ("k_ya-deva", "Letter"),
            ("j_ya-deva", "Letter"),
            ("k_ra-deva", "Letter"),
            ("reph-deva", "Mark"),
            ("rakar-deva", "Mark"),
            ("ya-deva.below", "Letter"),
        ];
        AutoContext::new(glyphs.iter().map(|(n, c)| glyph(n, c)).collect(), Arc::default()).unwrap()
    }

    fn code(generator: &Indic, cx: &AutoContext) -> Option<String> {
        let mut entries = generator.generate(cx).unwrap();
        assert!(entries.len() <= 1);
        entries.pop().map(|e| {
            assert_eq!(e.kind, EntryKind::Feature);
            assert_eq!(Some(&e.name.as_str()), generator.names().first());
            e.code
        })
    }

    #[test]
    fn indic_features_from_devanagari_glyph_names() {
        let cx = devanagari();
        let expected = [
            (&NUKT, "sub ka-deva nukta-deva by kaNukta-deva;\n"),
            (&AKHN, "sub ka-deva halant-deva ssa-deva by k_ssa-deva;\nsub ja-deva halant-deva nya-deva by j_nya-deva;\n"),
            (&RPHF, "sub ra-deva halant-deva by reph-deva;\n"),
            (&RKRF, "sub ka-deva halant-deva ra-deva by k_ra-deva;\n"),
            (&BLWF, "sub halant-deva ra-deva by rakar-deva;\nsub halant-deva ya-deva by ya-deva.below;\n"),
            (&HALF, "sub ka-deva halant-deva by k-deva;\n"),
            (&CJCT, "sub k-deva ya-deva by k_ya-deva;\nsub ja-deva halant-deva ya-deva by j_ya-deva;\n"),
        ];
        for (generator, code_expected) in expected {
            assert_eq!(code(generator, &cx).as_deref(), Some(code_expected), "{:?}", generator.names());
        }
    }

    /// Without the halant only nukt is written, and a Devanagari rakar ligature is no conjunct;
    /// rakar ligatures outside Devanagari, Gujarati and Malayalam are conjuncts.
    #[test]
    fn indic_features_need_their_trigger_glyphs() {
        let no_halant = AutoContext::new(
            ["ka-deva", "ra-deva", "nukta-deva", "kaNukta-deva", "k-deva", "reph-deva", "k_ra-deva"].iter().map(|n| glyph(n, "Letter")).collect(),
            Arc::default(),
        )
        .unwrap();
        for generator in [&AKHN, &RPHF, &RKRF, &HALF] {
            assert_eq!(code(generator, &no_halant), None, "{:?}", generator.names());
        }
        assert!(code(&NUKT, &no_halant).is_some());
        assert_eq!(code(&CJCT, &no_halant), None);

        let bengali =
            AutoContext::new(["ka-beng", "ra-beng", "halant-beng", "k_ra-beng"].iter().map(|n| glyph(n, "Letter")).collect(), Arc::default())
                .unwrap();
        assert_eq!(code(&RKRF, &bengali), None);
        assert_eq!(code(&CJCT, &bengali).as_deref(), Some("sub ka-beng halant-beng ra-beng by k_ra-beng;\n"));
    }

    #[test]
    fn indic_features_not_generated_are_reported_for_indic_fonts_only() {
        let reported: Vec<String> = not_generated(&devanagari()).into_iter().map(|(tag, _)| tag).collect();
        assert_eq!(reported, ["abvs", "blws", "pref", "pres", "psts", "vatu"]);

        let latin = AutoContext::new(vec![glyph("a", "Letter"), glyph("k-dot", "Letter")], Arc::default()).unwrap();
        assert!(not_generated(&latin).is_empty());
        for generator in generators() {
            assert!(generator.generate(&latin).unwrap().is_empty());
        }
    }
}
