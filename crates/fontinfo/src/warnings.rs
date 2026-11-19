//! The cautions Font Info gives about values it accepts: a family name outside ASCII, a large
//! units per em, an unsafe or long style name, style linking across families and a weight class
//! between the standard steps. Each message comes from one function here, so the commands that
//! set a value, the queries that read it and the Font Info window use the same words.
//!
//! Rows: `fontInfo.unitsPerEm.limits`, `fontInfo.styleName.limits`,
//! `fontInfo.styleLinking.sameFamily`, `fontInfo.weightClass.range` (M7.23).

use fontcraft_model::{Font, Instance, StyleClass};

/// Family and style name together longer than this may fail in older software.
pub const MAX_SAFE_NAME_CHARS: usize = 20;

/// The standard weight classes; the handbook notes that some applications refuse the others.
pub const STANDARD_WEIGHT_CLASSES: [u16; 9] = [100, 200, 300, 400, 500, 600, 700, 800, 900];

/// One warning and the Font Info field it belongs under: `familyName` or `unitsPerEm` on the Font
/// tab; `styleName`, `styleLinking` or `weightClass` on the Exports tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    pub field: &'static str,
    pub message: String,
}

impl Warning {
    fn new(field: &'static str, message: String) -> Self {
        Self { field, message }
    }
}

/// A warning when the family name has characters other than ASCII letters, digits and spaces.
pub fn family_name(name: &str) -> Option<String> {
    (!name.trim().chars().all(|c| c.is_ascii_alphanumeric() || c == ' ')).then(|| {
        "the family name should hold only ASCII letters, digits and spaces: other characters can stop the fonts from exporting or installing (add a familyNames field for other names)"
            .to_string()
    })
}

/// The handbook's cautions about large units per em, each strictly above its threshold: 3000
/// (some applications), 4096 (glyph widths of CFF-flavoured fonts) and 5000 (some design
/// applications).
pub fn units_per_em(upm: u16) -> Vec<String> {
    let mut out = Vec::new();
    if upm > 3000 {
        out.push("some applications have trouble with more than 3000 units per em".to_string());
    }
    if upm > 4096 {
        out.push("glyph widths of fonts with CFF outlines can be unreliable above 4096 units per em".to_string());
    }
    if upm > 5000 {
        out.push("some design applications have trouble with more than 5000 units per em".to_string());
    }
    out
}

/// The warnings of the Font tab's fields, in field order.
pub fn font(font: &Font) -> Vec<Warning> {
    let mut out: Vec<Warning> = family_name(&font.family_name).map(|m| Warning::new("familyName", m)).into_iter().collect();
    out.extend(units_per_em(font.units_per_em).into_iter().map(|m| Warning::new("unitsPerEm", m)));
    out
}

/// The family name an instance exports with: its own familyNames field, else the font's.
pub fn family_of<'a>(font: &'a Font, inst: &'a Instance) -> &'a str {
    inst.properties.value("familyNames").or_else(|| inst.properties.value("familyName")).unwrap_or(font.family_name.as_str())
}

/// Whether the instance carries an abbreviated PostScript name.
fn has_postscript_name(inst: &Instance) -> bool {
    ["postscriptFontName", "postscriptFontNames"].iter().any(|k| inst.properties.get(k).is_some() || inst.params.get(k).is_some())
}

/// The handbook's advice on style names: ASCII letters, digits, spaces and hyphens only; family
/// and style name within [`MAX_SAFE_NAME_CHARS`] characters unless a PostScript name is set.
pub fn style_name(font: &Font, inst: &Instance) -> Vec<String> {
    let mut out = Vec::new();
    let name = &inst.name;
    if name.chars().any(|c| !(c.is_ascii_alphanumeric() || c == ' ' || c == '-')) {
        out.push(format!(
            "the style name `{name}` has characters other than ASCII letters, digits, spaces and hyphens, which some software cannot show; Unicode names belong in the styleNames field"
        ));
    }
    let family = family_of(font, inst);
    let total = family.chars().count().saturating_add(name.chars().count());
    if total > MAX_SAFE_NAME_CHARS && !has_postscript_name(inst) {
        out.push(format!(
            "`{family}` and `{name}` together have {total} characters; names over {MAX_SAFE_NAME_CHARS} can fail in older software: set an abbreviated postscriptFontName and postscriptFullName"
        ));
    }
    out
}

/// A warning when a bold or italic instance links to a static instance of this font that exports
/// under another family name: style linking works only within one family. A base not in this
/// font is not checked.
pub fn style_link(font: &Font, inst: &Instance) -> Option<String> {
    let base = inst.link_style.as_deref()?;
    if !inst.is_bold && !inst.is_italic {
        return None;
    }
    let own = family_of(font, inst);
    let other = font.instances.iter().find(|b| b.name == base && b.kind.is_none())?;
    let theirs = family_of(font, other);
    (own != theirs).then(|| {
        format!("`{}` (family `{own}`) links to `{base}` (family `{theirs}`): style linking works only between fonts of the same family", inst.name)
    })
}

/// A warning when a weight class is not one of [`STANDARD_WEIGHT_CLASSES`].
pub fn weight_class(class: f64) -> Option<String> {
    let standard = STANDARD_WEIGHT_CLASSES.iter().any(|&w| f64::from(w) == class);
    (!standard).then(|| {
        format!("the weight class {class} is not one of the standard steps 100, 200, … 900; some applications refuse fonts with other weight classes")
    })
}

/// The weight class warning of an instance; none when the source states no class (400) or names
/// one (`Bold`).
pub fn instance_weight_class(inst: &Instance) -> Option<String> {
    match inst.weight_class {
        Some(StyleClass::Number(n)) => weight_class(n),
        _ => None,
    }
}

/// The warnings of one export entry's Exports tab fields, in field order. A variable font
/// setting has no style linking or weight class to warn about.
pub fn instance(font: &Font, inst: &Instance) -> Vec<Warning> {
    let mut out: Vec<Warning> = style_name(font, inst).into_iter().map(|m| Warning::new("styleName", m)).collect();
    if inst.kind.is_none() {
        out.extend(style_link(font, inst).map(|m| Warning::new("styleLinking", m)));
        out.extend(instance_weight_class(inst).map(|m| Warning::new("weightClass", m)));
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn named(name: &str) -> Instance {
        Instance { name: name.into(), ..Default::default() }
    }

    #[test]
    fn units_per_em_warns_strictly_above_each_threshold() {
        assert!(units_per_em(3000).is_empty());
        assert_eq!(units_per_em(3001).len(), 1);
        assert_eq!(units_per_em(4096).len(), 1, "the CFF caution is for more than 4096");
        assert_eq!(units_per_em(4097).len(), 2);
        assert_eq!(units_per_em(5001).len(), 3);
        let mut f = Font::new("Test Sans");
        f.units_per_em = 4097;
        let fields: Vec<&str> = font(&f).iter().map(|w| w.field).collect();
        assert_eq!(fields, ["unitsPerEm", "unitsPerEm"]);
    }

    #[test]
    fn instance_warnings_name_their_fields() {
        let mut f = Font::new("Test Sans");
        let mut regular = named("Regular");
        regular.properties.set("familyNames", "Test Sans Display", true);
        // 31 characters with the family: over the safe length.
        let mut italic = named("Extra Condensed Italic");
        italic.is_italic = true;
        italic.link_style = Some("Regular".into());
        italic.weight_class = Some(StyleClass::Number(350.0));
        f.instances = Arc::new(vec![regular, italic.clone()]);
        let fields: Vec<&str> = instance(&f, &italic).iter().map(|w| w.field).collect();
        assert_eq!(fields, ["styleName", "styleLinking", "weightClass"]);

        let mut setting = italic;
        setting.kind = Some("variable".into());
        let fields: Vec<&str> = instance(&f, &setting).iter().map(|w| w.field).collect();
        assert_eq!(fields, ["styleName"], "a variable font setting has no linking or class");
    }

    #[test]
    fn weight_class_warns_between_and_beyond_the_standard_steps() {
        assert!(weight_class(400.0).is_none());
        assert!(weight_class(900.0).is_none());
        assert!(weight_class(350.0).is_some());
        assert!(weight_class(1000.0).is_some());
        assert!(weight_class(1.0).is_some());
        assert!(instance_weight_class(&named("Regular")).is_none(), "no stated class is 400");
    }
}
