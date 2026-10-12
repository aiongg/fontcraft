//! The automatic Languagesystems prefix (M8.9).
//!
//! Rows: `autoFeature.languageSystems`.
//!
//! The code is `languagesystem DFLT dflt;`, then, for each script the `locl` features name, a
//! blank line, `languagesystem <script> dflt;` and one line per language of that script, in the
//! order `locl` names them ([`super::locl_languages`]). That is the form of the reference's stored
//! code in the corpus: a font without `locl` gets the `DFLT` line alone, whatever its scripts.
//!
//! The prefix depends on the `locl` code Update writes, so [`super::update`] computes it from its
//! merged list with [`code_for`]; the generator projects that list from the entries before Update.

use fontcraft_model::{Features, Snippet};

use super::{AutoContext, AutoEntry, Generator, locl_languages};
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

/// The prefix's code for the `locl` entries of `features`.
pub(super) fn code_for(features: &Features) -> String {
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
    for script in scripts {
        code.push_str(&format!("\nlanguagesystem {script} dflt;\n"));
        for (_, language) in pairs.iter().filter(|(s, _)| s == script) {
            code.push_str(&format!("languagesystem {script} {language};\n"));
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
        Ok(vec![AutoEntry { kind: EntryKind::Prefix, name: NAME.to_string(), code: code_for(&projected), optional: false }])
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
        assert_eq!(code_for(&Features::default()), "languagesystem DFLT dflt;\n");
        let features = locl(
            "script latn;\nlanguage AZE CRT KAZ TAT TRK;\nlookup a { sub i by idotaccent; } a;\nlanguage ROM MOL;\n\
             script cyrl;\nlanguage BGR;\nsub be-cy by be-cy.loclBGR;\nscript latn;\nlanguage CAT;\nlanguage bad*;",
        );
        assert_eq!(
            code_for(&features),
            "languagesystem DFLT dflt;\n\nlanguagesystem latn dflt;\nlanguagesystem latn AZE;\nlanguagesystem latn CRT;\n\
             languagesystem latn KAZ;\nlanguagesystem latn TAT;\nlanguagesystem latn TRK;\nlanguagesystem latn ROM;\n\
             languagesystem latn MOL;\nlanguagesystem latn CAT;\n\nlanguagesystem cyrl dflt;\nlanguagesystem cyrl BGR;\n"
        );
    }
}
