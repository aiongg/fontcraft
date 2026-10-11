//! Fractions (M8.11).
//!
//! Rows: `autoFeature.frac`, `autoFeature.afrc`.
//!
//! - afrc: one ligature rule per nut fraction of the handbook's list (`sub one slash two by
//!   oneovertwo;`), in that list's order.
//! - frac with a fraction glyph and figures that have both `.numr` and `.dnom` forms: three
//!   lookups, a slash between figures to fraction, every figure to its numerator, and numerators
//!   after fraction or a denominator to denominators.
//! - frac otherwise: ligature rules onto the precomposed fractions, the Unicode vulgar fractions
//!   in code point order (the corpus's `sub one slash four by onequarter;`) and then the nut
//!   fractions.

use std::collections::HashSet;

use super::ligatures::Code;
use super::{AutoContext, AutoEntry, Generator};
use crate::FeatureError;

/// This module's generators.
pub(super) fn generators() -> Vec<&'static dyn Generator> {
    vec![&ALTERNATIVE_FRACTIONS, &FRACTIONS]
}

/// The default figures, zero to nine.
pub(super) const FIGURES: [&str; 10] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];

/// The precomposed fractions of Unicode, in code point order: name, numerator, denominator.
const VULGAR: [(&str, &str, &str); 19] = [
    ("onequarter", "1", "4"),
    ("onehalf", "1", "2"),
    ("threequarters", "3", "4"),
    ("oneseventh", "1", "7"),
    ("oneninth", "1", "9"),
    ("onetenth", "1", "10"),
    ("onethird", "1", "3"),
    ("twothirds", "2", "3"),
    ("onefifth", "1", "5"),
    ("twofifths", "2", "5"),
    ("threefifths", "3", "5"),
    ("fourfifths", "4", "5"),
    ("onesixth", "1", "6"),
    ("fivesixths", "5", "6"),
    ("oneeighth", "1", "8"),
    ("threeeighths", "3", "8"),
    ("fiveeighths", "5", "8"),
    ("seveneighths", "7", "8"),
    ("zerothirds", "0", "3"),
];

/// The nut fractions afrc takes, in the handbook's order: numerator word, `over`, denominator
/// word.
const NUT: [&str; 47] = [
    "oneovertwo",
    "zerooverthree",
    "oneoverthree",
    "twooverthree",
    "oneoverfour",
    "threeoverfour",
    "oneoverfive",
    "twooverfive",
    "threeoverfive",
    "fouroverfive",
    "oneoversix",
    "fiveoversix",
    "oneoverseven",
    "twooverseven",
    "threeoverseven",
    "fouroverseven",
    "fiveoverseven",
    "sixoverseven",
    "oneovereight",
    "threeovereight",
    "fiveovereight",
    "sevenovereight",
    "oneovernine",
    "twoovernine",
    "fourovernine",
    "fiveovernine",
    "sevenovernine",
    "eightovernine",
    "oneoverten",
    "threeoverten",
    "sevenoverten",
    "nineoverten",
    "oneovereleven",
    "twoovereleven",
    "threeovereleven",
    "fourovereleven",
    "fiveovereleven",
    "sixovereleven",
    "sevenovereleven",
    "eightovereleven",
    "nineovereleven",
    "tenovereleven",
    "oneovertwelve",
    "fiveovertwelve",
    "sevenovertwelve",
    "elevenovertwelve",
    "oneoveronehundred",
];

/// The number words of the nut fraction names, as digits.
fn number(word: &str) -> Option<&'static str> {
    Some(match word {
        "zero" => "0",
        "one" => "1",
        "two" => "2",
        "three" => "3",
        "four" => "4",
        "five" => "5",
        "six" => "6",
        "seven" => "7",
        "eight" => "8",
        "nine" => "9",
        "ten" => "10",
        "eleven" => "11",
        "twelve" => "12",
        "onehundred" => "100",
        _ => return None,
    })
}

/// The numerator and denominator of a nut fraction name.
fn nut_parts(name: &str) -> Option<(&'static str, &'static str)> {
    let (numerator, denominator) = name.split_once("over")?;
    Some((number(numerator)?, number(denominator)?))
}

/// `one slash one zero` for 1/10, when the font has every glyph it names.
fn sequence(cx: &AutoContext, numerator: &str, denominator: &str) -> Option<String> {
    let figure = |d: char| d.to_digit(10).and_then(|d| FIGURES.get(usize::try_from(d).ok()?)).copied();
    let mut names = Vec::new();
    for d in numerator.chars() {
        names.push(figure(d)?);
    }
    names.push("slash");
    for d in denominator.chars() {
        names.push(figure(d)?);
    }
    names.iter().all(|n| cx.has(n)).then(|| names.join(" "))
}

/// One ligature rule per fraction of `fractions` the font has, skipping repeated inputs.
fn fraction_rules<'a>(cx: &AutoContext, fractions: impl Iterator<Item = (&'a str, &'a str, &'a str)>) -> Result<Code, FeatureError> {
    let mut code = Code::default();
    let mut seen = HashSet::new();
    for (name, numerator, denominator) in fractions {
        if !cx.has(name) {
            continue;
        }
        let Some(input) = sequence(cx, numerator, denominator) else { continue };
        if seen.insert(input.clone()) {
            code.line(&format!("sub {input} by {name};"))?;
        }
    }
    Ok(code)
}

fn nut_fractions() -> impl Iterator<Item = (&'static str, &'static str, &'static str)> {
    NUT.iter().filter_map(|name| nut_parts(name).map(|(n, d)| (*name, n, d)))
}

/// afrc.
struct AlternativeFractions;

static ALTERNATIVE_FRACTIONS: AlternativeFractions = AlternativeFractions;

impl Generator for AlternativeFractions {
    fn names(&self) -> &'static [&'static str] {
        &["afrc"]
    }

    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError> {
        let code = fraction_rules(cx, nut_fractions())?;
        Ok(if code.is_empty() { Vec::new() } else { vec![code.feature("afrc")] })
    }
}

/// frac.
struct Fractions;

static FRACTIONS: Fractions = Fractions;

impl Generator for Fractions {
    fn names(&self) -> &'static [&'static str] {
        &["frac"]
    }

    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError> {
        let paired: Vec<(&str, String, String)> =
            FIGURES.iter().map(|f| (*f, format!("{f}.numr"), format!("{f}.dnom"))).filter(|(f, n, d)| cx.has(f) && cx.has(n) && cx.has(d)).collect();
        let code = if cx.has("fraction") && !paired.is_empty() {
            let figures: Vec<&str> = paired.iter().map(|(f, _, _)| *f).collect();
            let numerators: Vec<&str> = paired.iter().map(|(_, n, _)| n.as_str()).collect();
            let denominators: Vec<&str> = paired.iter().map(|(_, _, d)| d.as_str()).collect();
            let (figures, numerators, denominators) = (figures.join(" "), numerators.join(" "), denominators.join(" "));
            let mut code = Code::default();
            if cx.has("slash") {
                code.line("lookup frac_bar {")?;
                code.line(&format!("\tsub [{figures}] slash' [{figures}] by fraction;"))?;
                code.line("} frac_bar;")?;
                code.line("")?;
            }
            code.line("lookup frac_numr {")?;
            code.line(&format!("\tsub [{figures}] by [{numerators}];"))?;
            code.line("} frac_numr;")?;
            code.line("")?;
            code.line("lookup frac_dnom {")?;
            code.line(&format!("\tsub [fraction {denominators}] [{numerators}]' by [{denominators}];"))?;
            code.line("} frac_dnom;")?;
            code
        } else {
            fraction_rules(cx, VULGAR.iter().copied().chain(nut_fractions()))?
        };
        Ok(if code.is_empty() { Vec::new() } else { vec![code.feature("frac")] })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::Features;

    use super::super::AutoGlyph;
    use super::*;

    fn names(names: &[&str]) -> AutoContext {
        let glyphs = names.iter().map(|n| AutoGlyph { name: n.to_string(), ..AutoGlyph::default() }).collect();
        AutoContext::new(glyphs, Arc::new(Features::default())).unwrap()
    }

    fn with_figures(extra: &[&str]) -> AutoContext {
        let mut list: Vec<&str> = FIGURES.to_vec();
        list.extend_from_slice(extra);
        names(&list)
    }

    fn code(entries: &[AutoEntry]) -> Option<&str> {
        entries.first().map(|e| e.code.as_str())
    }

    #[test]
    fn every_nut_fraction_name_parses() {
        assert!(NUT.iter().all(|n| nut_parts(n).is_some()));
        assert_eq!(nut_parts("oneoveronehundred"), Some(("1", "100")));
    }

    /// Oswald: a fraction glyph but no numerators, so the precomposed fractions, in code point
    /// order although the font lists onehalf first.
    #[test]
    fn frac_falls_back_to_precomposed_fractions_as_oswald_stores_it() {
        let cx = with_figures(&["slash", "fraction", "onehalf", "onequarter", "threequarters"]);
        let entries = FRACTIONS.generate(&cx).unwrap();
        assert_eq!(
            code(&entries),
            Some("sub one slash four by onequarter;\nsub one slash two by onehalf;\nsub three slash four by threequarters;\n")
        );
        assert!(FRACTIONS.generate(&with_figures(&["slash", "fraction"])).unwrap().is_empty());
    }

    #[test]
    fn frac_with_numerators_and_denominators_uses_three_lookups() {
        let cx = names(&["one", "two", "one.numr", "two.numr", "one.dnom", "two.dnom", "slash", "fraction", "onehalf"]);
        let entries = FRACTIONS.generate(&cx).unwrap();
        assert_eq!(
            code(&entries),
            Some(
                "lookup frac_bar {\n\tsub [one two] slash' [one two] by fraction;\n} frac_bar;\n\n\
                 lookup frac_numr {\n\tsub [one two] by [one.numr two.numr];\n} frac_numr;\n\n\
                 lookup frac_dnom {\n\tsub [fraction one.dnom two.dnom] [one.numr two.numr]' by [one.dnom two.dnom];\n} frac_dnom;\n"
            )
        );
    }

    #[test]
    fn afrc_writes_one_rule_per_nut_fraction_in_the_handbook_order() {
        let cx = with_figures(&["slash", "oneoverten", "oneovertwo", "oneoveronehundred", "twooverthirteen"]);
        let entries = ALTERNATIVE_FRACTIONS.generate(&cx).unwrap();
        assert_eq!(
            code(&entries),
            Some("sub one slash two by oneovertwo;\nsub one slash one zero by oneoverten;\nsub one slash one zero zero by oneoveronehundred;\n")
        );
        assert!(ALTERNATIVE_FRACTIONS.generate(&with_figures(&["oneovertwo"])).unwrap().is_empty());
    }
}
