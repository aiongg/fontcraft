//! The automatic Languagesystems prefix (M8.9, M8.32).
//!
//! Rows: `autoFeature.languageSystems`.
//!
//! The code is `languagesystem DFLT dflt;`, then, for each script the `locl` features name, a
//! blank line, `languagesystem <script> dflt;` and one line per language of that script, in the
//! order `locl` names them ([`super::locl_languages`]). That is the form of the reference's stored
//! code in the corpus: a Latin font without `locl` gets the `DFLT` line alone.
//!
//! A font with glyphs of one of the nine Indic scripts the Indic generators serve also gets that
//! script's group, in glyph order after the `locl` scripts: the newer tag (`dev2`) with its `locl`
//! languages, then the older tag (`deva`) with `dflt` alone. A shaper runs its Indic engine only
//! when the font registers features under one of the script's tags; under `DFLT` alone it applies
//! none of them. Both tags and their order follow fontTools' `ot_tags_from_script`, which the
//! kern and mark writers of ufo2ft and fontc use for the same scripts.
//!
//! The prefix depends on the `locl` code Update writes, so [`super::update`] computes it from its
//! merged list with [`code_for`]; the generator projects that list from the entries before Update.

use fontcraft_model::{Features, Snippet};

use super::indic::parse;
use super::{AutoContext, AutoEntry, AutoGlyph, Generator, locl_languages};
use crate::FeatureError;
use crate::source_map::EntryKind;

/// The prefix's name.
pub(super) const NAME: &str = "Languagesystems";

struct Languagesystems;

static LANGUAGESYSTEMS: Languagesystems = Languagesystems;

/// This module's generators.
pub(super) fn generators() -> Vec<&'static dyn Generator> {
    vec![&LANGUAGESYSTEMS]
}

/// A script or language tag: one to four letters, digits or spaces, starting with a letter or
/// digit. Anything else in the `locl` code is left out rather than copied into the prefix.
fn is_tag(tag: &str) -> bool {
    (1..=4).contains(&tag.len()) && tag.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The Indic scripts: glyph-database script name, glyph-name suffix, newer and older script tag.
static INDIC_TAGS: [(&str, &str, &str, &str); 9] = [
    ("devanagari", "deva", "dev2", "deva"),
    ("bengali", "beng", "bng2", "beng"),
    ("gurmukhi", "gurmukhi", "gur2", "guru"),
    ("gujarati", "gujarati", "gjr2", "gujr"),
    ("oriya", "oriya", "ory2", "orya"),
    ("tamil", "tamil", "tml2", "taml"),
    ("telugu", "telugu", "tel2", "telu"),
    ("kannada", "kannada", "knd2", "knda"),
    ("malayalam", "malayalam", "mlm2", "mlym"),
];

/// The (newer, older) tags of the Indic script of `glyph`: its effective script, else the script
/// its name spells (`ka-deva`), as the Indic generators read it.
fn indic_tags(glyph: &AutoGlyph) -> Option<(&'static str, &'static str)> {
    let found = match glyph.script.as_deref() {
        Some(script) => INDIC_TAGS.iter().find(|row| row.0 == script),
        None => parse(&glyph.name).and_then(|name| INDIC_TAGS.iter().find(|row| row.1 == name.script.suffix)),
    };
    found.map(|row| (row.2, row.3))
}

/// The prefix's code for the `locl` entries of `features` and the Indic scripts of `glyphs`.
pub(super) fn code_for(features: &Features, glyphs: &[AutoGlyph]) -> String {
    let pairs: Vec<(String, String)> = locl_languages(features).into_iter().filter(|(s, l)| is_tag(s) && is_tag(l)).collect();
    let mut code = String::from("languagesystem DFLT dflt;\n");
    for (_, language) in pairs.iter().filter(|(s, _)| s == "DFLT") {
        code.push_str(&format!("languagesystem DFLT {language};\n"));
    }
    let mut scripts: Vec<&str> = Vec::new();
    for (script, _) in &pairs {
        if script != "DFLT" && !scripts.contains(&script.as_str()) {
            scripts.push(script);
        }
    }
    // Each Indic script once, in the order its first glyph comes; nine at most.
    let mut indic: Vec<(&str, &str)> = Vec::new();
    for tags in glyphs.iter().filter_map(indic_tags) {
        if !indic.contains(&tags) {
            indic.push(tags);
            if indic.len() == INDIC_TAGS.len() {
                break;
            }
        }
    }
    for (newer, _) in &indic {
        if !scripts.contains(newer) {
            scripts.push(*newer);
        }
    }
    for &script in &scripts {
        code.push_str(&format!("\nlanguagesystem {script} dflt;\n"));
        for (_, language) in pairs.iter().filter(|(s, _)| s == script) {
            code.push_str(&format!("languagesystem {script} {language};\n"));
        }
        // The older tag follows its newer one, unless locl gave it a group of its own.
        if let Some((_, older)) = indic.iter().find(|(newer, _)| *newer == script)
            && !scripts.contains(older)
        {
            code.push_str(&format!("languagesystem {older} dflt;\n"));
        }
    }
    code
}

impl Generator for Languagesystems {
    fn names(&self) -> &'static [&'static str] {
        &[NAME]
    }

    fn generate(&self, cx: &AutoContext) -> Result<Vec<AutoEntry>, FeatureError> {
        // The locl entries as Update leaves them: automatic ones regenerated, a new one added.
        let mut locl = Vec::new();
        for generator in super::generators().into_iter().filter(|g| g.names().contains(&"locl")) {
            locl.extend(generator.generate(cx)?.into_iter().filter(|e| e.kind == EntryKind::Feature && e.name == "locl"));
        }
        let mut projected = Features::default();
        for s in cx.existing().features.iter().filter(|s| s.name == "locl") {
            let code = if s.automatic { locl.first().map_or("", |e| e.code.as_str()) } else { s.code.as_str() };
            projected.features.push(Snippet { code: code.to_string(), disabled: s.disabled, ..Snippet::default() });
        }
        if projected.features.is_empty()
            && let Some(e) = locl.first()
        {
            projected.features.push(Snippet { code: e.code.clone(), ..Snippet::default() });
        }
        for s in &mut projected.features {
            s.name = "locl".to_string();
        }
        Ok(vec![AutoEntry { kind: EntryKind::Prefix, name: NAME.to_string(), code: code_for(&projected, cx.glyphs()), optional: false }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locl(code: &str) -> Features {
        Features { features: vec![Snippet { name: "locl".into(), code: code.into(), ..Snippet::default() }], ..Features::default() }
    }

    #[test]
    fn languagesystems_list_the_locl_languages_by_script() {
        assert_eq!(code_for(&Features::default(), &[]), "languagesystem DFLT dflt;\n");
        let features = locl(
            "script latn;\nlanguage AZE CRT KAZ TAT TRK;\nlookup a { sub i by idotaccent; } a;\nlanguage ROM MOL;\n\
             script cyrl;\nlanguage BGR;\nsub be-cy by be-cy.loclBGR;\nscript latn;\nlanguage CAT;\nlanguage bad*;",
        );
        assert_eq!(
            code_for(&features, &[]),
            "languagesystem DFLT dflt;\n\nlanguagesystem latn dflt;\nlanguagesystem latn AZE;\nlanguagesystem latn CRT;\n\
             languagesystem latn KAZ;\nlanguagesystem latn TAT;\nlanguagesystem latn TRK;\nlanguagesystem latn ROM;\n\
             languagesystem latn MOL;\nlanguagesystem latn CAT;\n\nlanguagesystem cyrl dflt;\nlanguagesystem cyrl BGR;\n"
        );
    }

    fn glyph(name: &str, script: Option<&str>) -> AutoGlyph {
        AutoGlyph { name: name.into(), script: script.map(Into::into), ..AutoGlyph::default() }
    }

    #[test]
    fn indic_glyphs_add_the_newer_then_the_older_script_tag() {
        let glyphs = [
            glyph("A", Some("latin")),
            glyph("ka-gujarati", Some("gujarati")),
            glyph("ka-deva", Some("devanagari")),
            glyph("kha-deva", None),
            glyph("xa-tamil", None),
            glyph("ka-beng", Some("latin")),
        ];
        // Latin glyphs add nothing; a name's script counts only when the glyph has none stored.
        assert_eq!(
            code_for(&Features::default(), &glyphs),
            "languagesystem DFLT dflt;\n\nlanguagesystem gjr2 dflt;\nlanguagesystem gujr dflt;\n\n\
             languagesystem dev2 dflt;\nlanguagesystem deva dflt;\n\nlanguagesystem tml2 dflt;\nlanguagesystem taml dflt;\n"
        );
        // locl's scripts come first; a newer tag locl names keeps its languages, and an older
        // tag locl names keeps its own group.
        let features = locl("script latn;\nlanguage TRK;\nscript dev2;\nlanguage MAR;\nscript tml2;\nlanguage TAM;\nscript taml;\nlanguage TAM;");
        let glyphs = [glyph("ka-tamil", Some("tamil")), glyph("ka-deva", Some("devanagari")), glyph("ka-telugu", Some("telugu"))];
        assert_eq!(
            code_for(&features, &glyphs),
            "languagesystem DFLT dflt;\n\nlanguagesystem latn dflt;\nlanguagesystem latn TRK;\n\n\
             languagesystem dev2 dflt;\nlanguagesystem dev2 MAR;\nlanguagesystem deva dflt;\n\n\
             languagesystem tml2 dflt;\nlanguagesystem tml2 TAM;\n\nlanguagesystem taml dflt;\nlanguagesystem taml TAM;\n\n\
             languagesystem tel2 dflt;\nlanguagesystem telu dflt;\n"
        );
    }
}
