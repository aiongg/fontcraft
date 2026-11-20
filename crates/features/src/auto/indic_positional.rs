//! The positional Indic features (M8.32): pref, vatu, pres, abvs, blws and psts.
//!
//! Rows: `autoFeature.abvs`, `.blws`, `.pref`, `.pres`, `.psts`, `.vatu`.
//!
//! The handbook's appendix gives each feature's trigger. The reference's public Devanagari
//! tutorial says how pres picks an i-matra variant: each variant carries an `imatra` anchor, and
//! the variant whose anchor best meets a base's `top` anchor is chosen. The glyph database gives
//! the rest: which rakar is a spacing (pre-base) mark, the Kannada `.base` forms, and the
//! production names that spell a ligature's letter and mark. Each rule no source states is a
//! guess in its row.
//!
//! | Feature | Written for | Rule |
//! |---|---|---|
//! | pref | `rakar-S` filed as a spacing mark, with `ra-S` and the halant | `sub halant-S ra-S by rakar-S;` |
//! | vatu | the lookups the font's rkrf entry defines or calls | `lookup NAME;` for each |
//! | pres | `iMatra-S.01`… with `imatra` anchors (Devanagari, Bengali, Gurmukhi, Gujarati) | `sub iMatra-S' [bases] by iMatra-S.NN;` |
//! | abvs | a ligature whose production name is a letter and a sign with a `_top` anchor | `sub X M by L;` |
//! | blws | the same with a `_bottom` anchor; Gujarati `X-gujarati.straight` | `sub X M by L;`, `sub X-gujarati' [below forms] by X-gujarati.straight;` |
//! | psts | Kannada `X-kannada.base` and `.base.e`, with the halant | `sub X-kannada' eMatra-kannada by X-kannada.base.e;`, `sub X-kannada' [halant-kannada eMatra-kannada] by X-kannada.base;` |

use std::collections::HashMap;

use super::ccmp::Code;
use super::indic::{Name, Script, halant, is_pre_base, named, parse, parts};
use super::{AutoContext, AutoEntry, AutoGlyph, Generator};
use crate::FeatureError;
use crate::source_map::EntryKind;

/// Most numbered i-matra variants read per script; later ones are ignored.
const MAX_VARIANTS: usize = 100;

/// Most lookups vatu calls.
const MAX_LOOKUPS: usize = 256;

/// Longest lookup name the feature syntax allows.
const MAX_LOOKUP_NAME: usize = 63;

/// The scripts whose vowel sign i stands before the base, where pres picks its width.
const PRE_BASE_I: [&str; 4] = ["deva", "beng", "gurmukhi", "gujarati"];

/// One generator: a function writing the code of one feature.
struct Positional {
    names: &'static [&'static str],
    code: fn(&AutoContext) -> Result<Code, FeatureError>,
}

static PREF: Positional = Positional { names: &["pref"], code: pref };
static VATU: Positional = Positional { names: &["vatu"], code: vatu };
static PRES: Positional = Positional { names: &["pres"], code: pres };
static ABVS: Positional = Positional { names: &["abvs"], code: abvs };
static BLWS: Positional = Positional { names: &["blws"], code: blws };
static PSTS: Positional = Positional { names: &["psts"], code: psts };

/// This module's generators, in the order an Indic shaper applies their features.
pub(super) fn generators() -> Vec<&'static dyn Generator> {
    vec![&PREF, &VATU, &PRES, &ABVS, &BLWS, &PSTS]
}

impl Generator for Positional {
    fn names(&self) -> &'static [&'static str] {
        self.names
    }

    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError> {
        let Some(&tag) = self.names.first() else {
            return Ok(Vec::new());
        };
        let code = (self.code)(cx)?;
        if code.0.is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![AutoEntry { kind: EntryKind::Feature, name: tag.to_owned(), code: code.0, optional: false }])
    }
}

/// The glyphs of an Indic script in order, with their parsed names.
fn indic_glyphs(cx: &AutoContext) -> impl Iterator<Item = (&AutoGlyph, Name<'_>)> {
    cx.glyphs().iter().filter_map(|g| parse(&g.name).map(|n| (g, n)))
}

/// `[a b c]`, or the one name alone.
fn class(names: &[&str]) -> String {
    match names {
        [one] => (*one).to_owned(),
        _ => format!("[{}]", names.join(" ")),
    }
}

fn is_mark(glyph: &AutoGlyph) -> bool {
    glyph.category.as_deref() == Some("Mark")
}

/// A consonant, nukta form or conjunct that a vowel sign can follow: not a mark, vowel sign or
/// half form, no suffix, and its last part ends in the inherent `a`.
fn is_base(glyph: &AutoGlyph, n: &Name<'_>) -> bool {
    let sub = glyph.sub_category.as_deref();
    n.suffix.is_empty()
        && !is_mark(glyph)
        && sub != Some("Matra")
        && sub != Some("Halfform")
        && !n.stem.contains("Matra")
        && n.stem.rsplit('_').next().is_some_and(|last| last.ends_with('a'))
}

fn pref(cx: &AutoContext) -> Result<Code, FeatureError> {
    let mut code = Code::default();
    for (glyph, n) in indic_glyphs(cx) {
        if n.stem != "rakar" || !n.suffix.is_empty() || !is_pre_base(glyph) {
            continue;
        }
        let (Some(halant), Some(ra)) = (halant(cx, n.script), named(cx, "ra", n.script)) else {
            continue;
        };
        code.push(&format!("sub {halant} {ra} by {};\n", glyph.name))?;
    }
    Ok(code)
}

fn vatu(cx: &AutoContext) -> Result<Code, FeatureError> {
    let mut lookups: Vec<String> = Vec::new();
    for entry in cx.existing().features.iter().filter(|e| e.name == "rkrf" && !e.disabled) {
        for name in lookup_names(&entry.code) {
            if lookups.len() >= MAX_LOOKUPS {
                break;
            }
            if !lookups.contains(&name) {
                lookups.push(name);
            }
        }
    }
    let mut code = Code::default();
    for name in lookups {
        code.push(&format!("lookup {name};\n"))?;
    }
    Ok(code)
}

/// The names after each `lookup` keyword in `code` (definitions and calls alike), comments
/// skipped.
fn lookup_names(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in code.lines() {
        let line = line.split('#').next().unwrap_or_default();
        let spaced = line.replace('{', " { ").replace('}', " } ").replace(';', " ; ");
        let mut words = spaced.split_whitespace();
        while let Some(word) = words.next() {
            if word != "lookup" {
                continue;
            }
            let Some(name) = words.next() else {
                break;
            };
            let valid = name.len() <= MAX_LOOKUP_NAME
                && name.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_' || b == b'.')
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.');
            if valid {
                out.push(name.to_owned());
            }
            if out.len() >= MAX_LOOKUPS {
                return out;
            }
        }
    }
    out
}

/// An i-matra the pres rules can choose: its name and how far its hook reaches past its own
/// advance (its `imatra` anchor less its width).
struct Variant<'a> {
    name: &'a str,
    reach: f64,
}

/// The numbered variants `iMatra-S.1` to `iMatra-S.999` of `matra`, in glyph order.
fn is_numbered_variant(name: &str, matra: &str) -> bool {
    name.strip_prefix(matra)
        .and_then(|rest| rest.strip_prefix('.'))
        .is_some_and(|digits| (1..=3).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit()))
}

fn pres(cx: &AutoContext) -> Result<Code, FeatureError> {
    let mut code = Code::default();
    for suffix in PRE_BASE_I {
        let matra = format!("iMatra-{suffix}");
        if !cx.has(&matra) {
            continue;
        }
        let variant = |name: &str| {
            let shape = cx.shape(name)?;
            Some(shape.anchor("imatra")? - shape.width)
        };
        let mut candidates: Vec<Variant<'_>> = Vec::new();
        let default = cx.glyph(&matra).map(|g| g.name.as_str());
        if let Some(name) = default
            && let Some(reach) = variant(name)
        {
            candidates.push(Variant { name, reach });
        }
        let numbered = cx.glyphs().iter().filter(|g| is_numbered_variant(&g.name, &matra)).take(MAX_VARIANTS);
        let mut any_numbered = false;
        for glyph in numbered {
            if let Some(reach) = variant(&glyph.name) {
                candidates.push(Variant { name: &glyph.name, reach });
                any_numbered = true;
            }
        }
        if !any_numbered {
            continue;
        }
        // The bases each numbered variant fits best, in glyph order.
        let mut chosen: HashMap<&str, Vec<&str>> = HashMap::new();
        for (glyph, n) in indic_glyphs(cx) {
            if n.script.suffix != suffix || !is_base(glyph, &n) {
                continue;
            }
            let Some(shape) = cx.shape(&glyph.name) else {
                continue;
            };
            // Where the stem the hook meets stands: the `top` anchor, else the right edge.
            let target = shape.anchor("top").unwrap_or(shape.width);
            let mut best: Option<(&str, f64)> = None;
            for c in &candidates {
                let miss = (c.reach - target).abs();
                if best.is_none_or(|(_, m)| miss < m) {
                    best = Some((c.name, miss));
                }
            }
            if let Some((name, _)) = best
                && Some(name) != default
            {
                chosen.entry(name).or_default().push(&glyph.name);
            }
        }
        for c in &candidates {
            if let Some(bases) = chosen.get(c.name) {
                code.push(&format!("sub {matra}' {} by {};\n", class(bases), c.name))?;
            }
        }
    }
    Ok(code)
}

/// The two code points of a production name `uniXXXXYYYY`.
fn production_pair(production: &str) -> Option<(u32, u32)> {
    let hex = production.strip_prefix("uni").filter(|h| h.len() == 8 && h.bytes().all(|b| b.is_ascii_hexdigit()))?;
    let first = u32::from_str_radix(hex.get(..4)?, 16).ok()?;
    let second = u32::from_str_radix(hex.get(4..)?, 16).ok()?;
    Some((first, second))
}

/// Ligatures of a letter and one sign, from their production names: `sub X M by L;` for each
/// unencoded `L` whose sign `M` attaches through the anchor `attach` (`_top` for abvs, `_bottom`
/// for blws). The halant and nukta have their own features and are left out.
fn sign_ligatures(cx: &AutoContext, attach: &str, code: &mut Code) -> Result<(), FeatureError> {
    let mut by_code_point: HashMap<u32, &AutoGlyph> = HashMap::new();
    for glyph in cx.glyphs() {
        for &cp in &glyph.codepoints {
            by_code_point.entry(cp).or_insert(glyph);
        }
    }
    for (glyph, n) in indic_glyphs(cx) {
        if !glyph.codepoints.is_empty() || !n.suffix.is_empty() {
            continue;
        }
        let Some((first, second)) = cx.shape(&glyph.name).and_then(|s| s.production.as_deref()).and_then(production_pair) else {
            continue;
        };
        let (Some(letter), Some(sign)) = (by_code_point.get(&first), by_code_point.get(&second)) else {
            continue;
        };
        let sign_is_virama_or_nukta = sign.name == n.script.halant || parse(&sign.name).is_some_and(|s| s.stem == "nukta");
        if is_mark(letter) || letter.name.contains("Matra") || sign_is_virama_or_nukta || letter.name == sign.name {
            continue;
        }
        if cx.shape(&sign.name).and_then(|s| s.anchor(attach)).is_none() {
            continue;
        }
        code.push(&format!("sub {} {} by {};\n", letter.name, sign.name, glyph.name))?;
    }
    Ok(())
}

fn abvs(cx: &AutoContext) -> Result<Code, FeatureError> {
    let mut code = Code::default();
    sign_ligatures(cx, "_top", &mut code)?;
    Ok(code)
}

fn blws(cx: &AutoContext) -> Result<Code, FeatureError> {
    let mut code = Code::default();
    sign_ligatures(cx, "_bottom", &mut code)?;
    // Gujarati letters take their `.straight` form before a below-base form.
    let below: Vec<&str> = indic_glyphs(cx)
        .filter(|(g, n)| n.script.suffix == "gujarati" && (n.suffix == ".below" || (n.stem == "rakar" && n.suffix.is_empty() && !is_pre_base(g))))
        .map(|(g, _)| g.name.as_str())
        .collect();
    if below.is_empty() {
        return Ok(code);
    }
    for (glyph, n) in indic_glyphs(cx) {
        if n.script.suffix != "gujarati" || n.suffix != ".straight" {
            continue;
        }
        if let Some(plain) = named(cx, n.stem, n.script) {
            code.push(&format!("sub {plain}' {} by {};\n", class(&below), glyph.name))?;
        }
    }
    Ok(code)
}

/// The script psts writes `.base` forms for.
fn kannada(n: &Name<'_>) -> Option<&'static Script> {
    (n.script.suffix == "kannada").then_some(n.script)
}

fn psts(cx: &AutoContext) -> Result<Code, FeatureError> {
    let mut code = Code::default();
    for (glyph, n) in indic_glyphs(cx) {
        let Some(script) = kannada(&n) else {
            continue;
        };
        if n.suffix != ".base" && n.suffix != ".base.e" {
            continue;
        }
        let Some(halant) = halant(cx, script) else {
            continue;
        };
        let Some(plain) = named(cx, n.stem, script) else {
            continue;
        };
        if cx.glyph(&plain).is_some_and(is_mark) || n.stem.contains("Matra") || parts(n.stem).is_some() {
            continue;
        }
        let e_matra = named(cx, "eMatra", script);
        if n.suffix == ".base.e" {
            if let Some(e) = e_matra {
                code.push(&format!("sub {plain}' {e} by {};\n", glyph.name))?;
            }
            continue;
        }
        // A `.base.e` form takes the e sign; `.base` keeps the rest.
        let mut follow = vec![halant];
        if let Some(e) = e_matra.as_deref()
            && !cx.has(&format!("{}.e", glyph.name))
        {
            follow.push(e);
        }
        code.push(&format!("sub {plain}' {} by {};\n", class(&follow), glyph.name))?;
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::{Features, Snippet};

    use super::super::{AutoAnchor, AutoShape};
    use super::*;

    fn glyph(name: &str, category: &str, sub: Option<&str>) -> AutoGlyph {
        AutoGlyph { name: name.to_owned(), category: Some(category.to_owned()), sub_category: sub.map(str::to_owned), ..AutoGlyph::default() }
    }

    fn shape(width: f64, anchors: &[(&str, f64)]) -> AutoShape {
        AutoShape { production: None, width, anchors: anchors.iter().map(|(n, x)| AutoAnchor { name: (*n).to_owned(), x: *x }).collect() }
    }

    fn code(generator: &Positional, cx: &AutoContext) -> Option<String> {
        let mut entries = generator.generate(cx).unwrap();
        assert!(entries.len() <= 1);
        entries.pop().map(|e| {
            assert_eq!(Some(&e.name.as_str()), generator.names().first());
            e.code
        })
    }

    /// A Devanagari font built in code: five i-matra widths and three conjuncts of different
    /// widths, each with a `top` anchor on its stem.
    fn matra_variants() -> AutoContext {
        let mut glyphs = vec![glyph("ka-deva", "Letter", None), glyph("halant-deva", "Mark", None), glyph("iMatra-deva", "Letter", Some("Matra"))];
        let mut shapes =
            vec![("ka-deva".to_owned(), shape(500.0, &[("top", 420.0)])), ("iMatra-deva".to_owned(), shape(200.0, &[("imatra", 620.0)]))];
        // Variant n reaches 200 + 200n units past its own advance.
        for n in 1..=5u32 {
            let name = format!("iMatra-deva.{n:02}");
            glyphs.push(glyph(&name, "Letter", Some("Matra")));
            shapes.push((name, shape(200.0, &[("imatra", 400.0 + 200.0 * f64::from(n))])));
        }
        for (name, stem) in [("k_ssa-deva", 610.0), ("k_ya-deva", 1010.0), ("t_ta-deva", 1390.0)] {
            glyphs.push(glyph(name, "Letter", None));
            shapes.push((name.to_owned(), shape(stem + 80.0, &[("top", stem)])));
        }
        // A half form and a mark are no bases.
        glyphs.push(glyph("k-deva", "Letter", Some("Halfform")));
        shapes.push(("k-deva".to_owned(), shape(300.0, &[("top", 600.0)])));
        AutoContext::new(glyphs, Arc::default()).unwrap().with_shapes(shapes)
    }

    #[test]
    fn pres_picks_the_i_matra_variant_whose_anchor_meets_each_conjunct_stem() {
        let cx = matra_variants();
        // ka (stem 420) keeps the plain matra (reach 420); the conjuncts take .02, .04 and .05.
        assert_eq!(
            code(&PRES, &cx).as_deref(),
            Some(
                "sub iMatra-deva' k_ssa-deva by iMatra-deva.02;\nsub iMatra-deva' k_ya-deva by iMatra-deva.04;\nsub iMatra-deva' t_ta-deva by iMatra-deva.05;\n"
            )
        );

        // Without imatra anchors there is nothing to match.
        let bare = AutoContext::new(cx.glyphs().to_vec(), Arc::default()).unwrap();
        assert_eq!(code(&PRES, &bare), None);
    }

    #[test]
    fn pref_takes_the_spacing_rakar_and_blwf_leaves_it() {
        let glyphs = vec![
            glyph("ra-malayalam", "Letter", None),
            glyph("halant-malayalam", "Mark", None),
            glyph("rakar-malayalam", "Mark", Some("Spacing Combining")),
            glyph("ra-deva", "Letter", None),
            glyph("halant-deva", "Mark", None),
            glyph("rakar-deva", "Mark", Some("Nonspacing")),
        ];
        let cx = AutoContext::new(glyphs, Arc::default()).unwrap();
        assert_eq!(code(&PREF, &cx).as_deref(), Some("sub halant-malayalam ra-malayalam by rakar-malayalam;\n"));
        let blwf = super::super::indic::generators().into_iter().find(|g| g.names() == ["blwf"]).unwrap();
        let blwf = blwf.generate(&cx).unwrap().pop().map(|e| e.code);
        assert_eq!(blwf.as_deref(), Some("sub halant-deva ra-deva by rakar-deva;\n"));
    }

    #[test]
    fn vatu_calls_the_lookups_of_the_rkrf_entry() {
        let rkrf = "lookup rakar_forms {\n  sub ka-deva halant-deva ra-deva by k_ra-deva;\n} rakar_forms;\nlookup shared; # lookup commented\n";
        let features = Features { features: vec![Snippet { name: "rkrf".into(), code: rkrf.into(), ..Snippet::default() }], ..Features::default() };
        let cx = AutoContext::new(vec![glyph("ka-deva", "Letter", None)], Arc::new(features)).unwrap();
        assert_eq!(code(&VATU, &cx).as_deref(), Some("lookup rakar_forms;\nlookup shared;\n"));

        let plain = Features {
            features: vec![Snippet { name: "rkrf".into(), code: "sub ka-deva halant-deva ra-deva by k_ra-deva;".into(), ..Snippet::default() }],
            ..Features::default()
        };
        let cx = AutoContext::new(vec![glyph("ka-deva", "Letter", None)], Arc::new(plain)).unwrap();
        assert_eq!(code(&VATU, &cx), None, "an rkrf without lookups gives no vatu");
    }

    #[test]
    fn abvs_and_blws_join_letters_and_signs_by_production_name() {
        let mut ka = glyph("ka-kannada", "Letter", None);
        ka.codepoints = vec![0x0C95];
        let mut i = glyph("iMatra-kannada", "Mark", Some("Nonspacing"));
        i.codepoints = vec![0x0CBF];
        let mut u = glyph("uMatra-deva", "Mark", Some("Nonspacing"));
        u.codepoints = vec![0x0941];
        let mut ra = glyph("ra-deva", "Letter", None);
        ra.codepoints = vec![0x0930];
        let mut halant = glyph("halant-deva", "Mark", None);
        halant.codepoints = vec![0x094D];
        let glyphs =
            vec![ka, i, u, ra, halant, glyph("ki-kannada", "Letter", None), glyph("ru-deva", "Letter", None), glyph("reph-deva", "Mark", None)];
        let production = |p: &str| AutoShape { production: Some(p.to_owned()), ..AutoShape::default() };
        let shapes = [
            ("iMatra-kannada".to_owned(), shape(0.0, &[("_top", 0.0)])),
            ("uMatra-deva".to_owned(), shape(0.0, &[("_bottom", 0.0)])),
            ("halant-deva".to_owned(), shape(0.0, &[("_bottom", 0.0)])),
            ("ki-kannada".to_owned(), production("uni0C950CBF")),
            ("ru-deva".to_owned(), production("uni09300941")),
            ("reph-deva".to_owned(), production("uni0930094D")),
        ];
        let cx = AutoContext::new(glyphs, Arc::default()).unwrap().with_shapes(shapes);
        assert_eq!(code(&ABVS, &cx).as_deref(), Some("sub ka-kannada iMatra-kannada by ki-kannada;\n"));
        assert_eq!(code(&BLWS, &cx).as_deref(), Some("sub ra-deva uMatra-deva by ru-deva;\n"), "the halant is left to rphf and half");
    }

    #[test]
    fn blws_takes_straight_gujarati_letters_before_below_forms() {
        let glyphs = vec![
            glyph("da-gujarati", "Letter", None),
            glyph("da-gujarati.straight", "Letter", None),
            glyph("rakar-gujarati", "Mark", Some("Nonspacing")),
            glyph("va-gujarati.below", "Mark", None),
        ];
        let cx = AutoContext::new(glyphs, Arc::default()).unwrap();
        assert_eq!(code(&BLWS, &cx).as_deref(), Some("sub da-gujarati' [rakar-gujarati va-gujarati.below] by da-gujarati.straight;\n"));
    }

    #[test]
    fn psts_takes_kannada_base_forms_before_the_halant_and_the_e_sign() {
        let glyphs = vec![
            glyph("ka-kannada", "Letter", None),
            glyph("ma-kannada", "Letter", None),
            glyph("halant-kannada", "Mark", None),
            glyph("eMatra-kannada", "Mark", Some("Nonspacing")),
            glyph("ka-kannada.base", "Letter", None),
            glyph("ma-kannada.base", "Letter", None),
            glyph("ma-kannada.base.e", "Letter", None),
        ];
        let cx = AutoContext::new(glyphs.clone(), Arc::default()).unwrap();
        assert_eq!(
            code(&PSTS, &cx).as_deref(),
            Some(
                "sub ka-kannada' [halant-kannada eMatra-kannada] by ka-kannada.base;\nsub ma-kannada' halant-kannada by ma-kannada.base;\nsub ma-kannada' eMatra-kannada by ma-kannada.base.e;\n"
            )
        );
        let no_halant = AutoContext::new(glyphs.into_iter().filter(|g| g.name != "halant-kannada").collect(), Arc::default()).unwrap();
        assert_eq!(code(&PSTS, &no_halant), None, "the halant is the trigger");
    }

    #[test]
    fn positional_features_are_not_written_for_latin_fonts() {
        let latin = AutoContext::new(vec![glyph("a", "Letter", None), glyph("iMatra.01", "Letter", None)], Arc::default()).unwrap();
        for generator in generators() {
            assert!(generator.generate(&latin).unwrap().is_empty(), "{:?}", generator.names());
        }
    }
}
