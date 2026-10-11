//! Text tokens: `{{{token}}}` in a General field or a name parameter, replaced by a value of the
//! font, the master or the instance at export (version string, build number, a key path, a number
//! value, a glyph's property).
//!
//! One level only: an expanded value is not expanded again. At most [`crate::MAX_TOKENS`] tokens
//! per text and [`crate::MAX_EXPANDED_BYTES`] of result; past either, tokens stay as written. A
//! token that names nothing stays as written and is listed in [`Expanded::unresolved`].
//!
//! The grammar follows glyphsLib's reader (Apache-2.0, read for behaviour only): the shortest
//! text of at least one character between `{{{` and `}}}` on one line, trimmed of white space.
//!
//! What a token reads, in order:
//!
//! | Token | Context | Value |
//! |---|---|---|
//! | `versionString`, `buildNumber` | any | [`APP_VERSION`], [`BUILD_NUMBER`] |
//! | `glyph:path` | any | a property of the glyph's layer ([`GLYPH_PATHS`]) |
//! | a key of [`INSTANCE_KEYS`] | instance | the instance's value |
//! | a number value's name | master (no instance) | the master's value |
//! | a key of [`FONT_KEYS`] | font or instance | the font's value |
//!
//! Rows: `font.expandTextTokens`, `textToken.syntax`, `textToken.versionString`,
//! `textToken.buildNumber`, `textToken.keyPath`, `textToken.numberValue`,
//! `textToken.glyphProperty`, `fontInfo.general.textTokens` (M7.13). The compiler's name table
//! calls [`expand`] (M7.8).

use std::collections::BTreeMap;

use fontcraft_geom::{Rect, Shape as _};
use fontcraft_model::{Font, Instance, Master, StyleClass};

use crate::{MAX_EXPANDED_BYTES, MAX_TOKENS};

/// The opening delimiter of a token.
pub const OPEN: &str = "{{{";
/// The closing delimiter of a token.
pub const CLOSE: &str = "}}}";

/// What `{{{versionString}}}` gives: the application's version.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// What `{{{buildNumber}}}` gives: the release's build number, set when a release is built, `0`
/// in development builds.
pub const BUILD_NUMBER: &str = match option_env!("FONTCRAFT_BUILD_NUMBER") {
    Some(b) => b,
    None => "0",
};

/// The key paths a font context resolves.
pub const FONT_KEYS: &[&str] = &[
    "familyName",
    "versionMajor",
    "versionMinor",
    "unitsPerEm",
    "upm",
    "date",
    "copyright",
    "designer",
    "designerURL",
    "manufacturer",
    "manufacturerURL",
    "license",
    "licenseURL",
    "trademark",
    "description",
    "sampleText",
    "vendorID",
    "uniqueID",
];

/// The key paths an instance context resolves before falling back to [`FONT_KEYS`].
pub const INSTANCE_KEYS: &[&str] = &[
    "name",
    "familyName",
    "fullName",
    "postscriptFontName",
    "fontName",
    "postscriptFullName",
    "preferredFamilyName",
    "preferredSubfamilyName",
    "weightClass",
    "widthClass",
];

/// The layer properties a `glyph:path` token reads (`anchors.<name>.x` and `.y` too).
pub const GLYPH_PATHS: &[&str] = &["width", "LSB", "RSB", "bounds.origin.x", "bounds.origin.y", "bounds.size.width", "bounds.size.height"];

/// What a token may read.
pub struct TokenContext<'a> {
    pub font: &'a Font,
    /// The master being exported or edited; `None` at the font level.
    pub master: Option<&'a Master>,
    /// The instance being exported; `None` for a master or the font.
    pub instance: Option<&'a Instance>,
    /// The advance width of a glyph (name, master id), for glyph-property tokens. The compiler
    /// passes the resolved widths; the engine reads the font.
    pub glyph_width: &'a dyn Fn(&str, &str) -> Option<f64>,
}

/// The result of [`expand`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Expanded {
    pub text: String,
    /// The tokens kept as written because they name nothing, each once, in order (without the
    /// delimiters).
    pub unresolved: Vec<String>,
}

/// Whether `text` holds a token opening.
pub fn has_tokens(text: &str) -> bool {
    text.contains(OPEN)
}

/// `text` with its tokens replaced.
pub fn expand(text: &str, ctx: &TokenContext) -> Expanded {
    let mut out = Expanded::default();
    if !has_tokens(text) {
        out.text = text.to_string();
        return out;
    }
    let mut resolver = Resolver { ctx, bounds: BTreeMap::new() };
    let mut text_out = String::with_capacity(text.len().min(MAX_EXPANDED_BYTES));
    // The length the result would have if nothing more were replaced.
    let mut projected = text.len();
    let mut tokens = 0usize;
    let mut rest = text;
    let mut from = 0usize;
    while let Some(found) = rest.get(from..).and_then(|s| s.find(OPEN)) {
        let open = from + found;
        let Some((inner, end)) = token_at(rest, open) else {
            // `{` is one byte, so `open + 1` is a character boundary.
            from = open + 1;
            continue;
        };
        tokens += 1;
        if tokens > MAX_TOKENS {
            break;
        }
        let (Some(before), Some(written), Some(after)) = (rest.get(..open), rest.get(open..end), rest.get(end..)) else {
            break;
        };
        text_out.push_str(before);
        let key = inner.trim();
        match resolver.resolve(key) {
            Some(value) => {
                let next = projected.saturating_sub(written.len()).saturating_add(value.len());
                if next > MAX_EXPANDED_BYTES && value.len() > written.len() {
                    text_out.push_str(written);
                } else {
                    projected = next;
                    text_out.push_str(&value);
                }
            }
            None => {
                text_out.push_str(written);
                if !out.unresolved.iter().any(|u| u == key) {
                    out.unresolved.push(key.to_string());
                }
            }
        }
        rest = after;
        from = 0;
    }
    text_out.push_str(rest);
    out.text = text_out;
    out
}

/// The token whose `{{{` starts at byte `open` of `text`: its inner text and the byte just past
/// its `}}}`. At least one character between the delimiters, none of them a line break.
fn token_at(text: &str, open: usize) -> Option<(&str, usize)> {
    let start = open.checked_add(OPEN.len())?;
    let body = text.get(start..)?;
    let first = body.chars().next()?;
    if first == '\n' {
        return None;
    }
    let skip = first.len_utf8();
    let close = skip.checked_add(body.get(skip..)?.find(CLOSE)?)?;
    let inner = body.get(..close)?;
    if inner.contains('\n') {
        return None;
    }
    Some((inner, start.checked_add(close)?.checked_add(CLOSE.len())?))
}

struct Resolver<'c, 'a> {
    ctx: &'c TokenContext<'a>,
    /// Outline bounds per glyph name for the context's master, each computed once per call.
    bounds: BTreeMap<String, Option<Rect>>,
}

impl Resolver<'_, '_> {
    fn resolve(&mut self, key: &str) -> Option<String> {
        match key {
            "" => return None,
            "versionString" => return Some(APP_VERSION.to_string()),
            "buildNumber" => return Some(BUILD_NUMBER.to_string()),
            _ => {}
        }
        if let Some((glyph, path)) = key.split_once(':') {
            return self.glyph_property(glyph.trim(), path.trim());
        }
        let ctx = self.ctx;
        if let Some(instance) = ctx.instance {
            return instance_key(ctx.font, instance, key).or_else(|| font_key(ctx.font, key));
        }
        if let Some(master) = ctx.master {
            return number_value(ctx.font, master, key);
        }
        font_key(ctx.font, key)
    }

    /// The master a glyph property is read on: the context's, else the font's first (an
    /// instance's interpolated layer is not built here).
    fn master_id(&self) -> Option<&str> {
        match self.ctx.master {
            Some(m) => Some(m.id.as_str()),
            None => self.ctx.font.default_master().ok().map(|m| m.id.as_str()),
        }
    }

    fn glyph_property(&mut self, glyph: &str, path: &str) -> Option<String> {
        let font = self.ctx.font;
        let master = self.master_id()?.to_string();
        let g = font.glyph(glyph)?;
        let width = || (self.ctx.glyph_width)(glyph, &master).or_else(|| g.layer(&master).map(|l| l.width));
        if path == "width" {
            return number(width()?);
        }
        if let Some(anchor) = path.strip_prefix("anchors.") {
            let (name, axis) = anchor.rsplit_once('.')?;
            let a = g.layer(&master)?.anchors.iter().find(|a| a.name == name)?;
            return match axis {
                "x" => number(a.x),
                "y" => number(a.y),
                _ => None,
            };
        }
        if !GLYPH_PATHS.contains(&path) {
            return None;
        }
        let advance = width();
        let r = self.bounds(glyph, &master)?;
        let v = match path {
            "LSB" => r.x0,
            "RSB" => advance? - r.x1,
            "bounds.origin.x" => r.x0,
            "bounds.origin.y" => r.y0,
            "bounds.size.width" => r.x1 - r.x0,
            "bounds.size.height" => r.y1 - r.y0,
            _ => return None,
        };
        number(v)
    }

    /// The bounds of the glyph's outline on `master`, components resolved; `None` for an empty
    /// layer or an outline that cannot be drawn.
    fn bounds(&mut self, glyph: &str, master: &str) -> Option<Rect> {
        if let Some(b) = self.bounds.get(glyph) {
            return *b;
        }
        let b = self
            .ctx
            .font
            .outline(glyph, master)
            .ok()
            .filter(|o| !o.elements().is_empty())
            .map(|o| o.bounding_box())
            .filter(|r| [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()));
        self.bounds.insert(glyph.to_string(), b);
        b
    }
}

/// A number as a token writes it: whole numbers without a fraction, others to at most three
/// decimals. `None` for a value that is not finite.
fn number(v: f64) -> Option<String> {
    if !v.is_finite() {
        return None;
    }
    let r = (v * 1000.0).round() / 1000.0;
    if !r.is_finite() {
        return None;
    }
    if r == r.trunc() && r.abs() < 1e15 {
        return Some(format!("{}", r as i64));
    }
    Some(format!("{r}"))
}

fn style_class(c: Option<&StyleClass>, default: f64) -> Option<String> {
    match c {
        Some(StyleClass::Number(n)) => number(*n),
        Some(StyleClass::Name(s)) => Some(s.clone()),
        None => number(default),
    }
}

/// A property by its plural (format 3) or singular key, or the singular key as a custom
/// parameter (format 2).
fn named(props: &fontcraft_model::Properties, params: &fontcraft_model::Params, plural: &str, singular: &str) -> Option<String> {
    props.value(plural).or_else(|| props.value(singular)).or_else(|| params.get(singular).and_then(|v| v.as_str())).map(str::to_string)
}

fn font_key(font: &Font, key: &str) -> Option<String> {
    let info = &font.info;
    let prop = |plural: &str, singular: &str| named(&info.properties, &info.params, plural, singular);
    match key {
        "familyName" => Some(font.family_name.clone()),
        "versionMajor" => Some(font.version_major.to_string()),
        "versionMinor" => Some(font.version_minor.to_string()),
        "unitsPerEm" | "upm" => Some(font.units_per_em.to_string()),
        "date" => info.date.clone(),
        "copyright" => prop("copyrights", "copyright"),
        "designer" => prop("designers", "designer"),
        "designerURL" => prop("designerURL", "designerURL"),
        "manufacturer" => prop("manufacturers", "manufacturer"),
        "manufacturerURL" => prop("manufacturerURL", "manufacturerURL"),
        "license" => prop("licenses", "license"),
        "licenseURL" => prop("licenseURL", "licenseURL"),
        "trademark" => prop("trademarks", "trademark"),
        "description" => prop("descriptions", "description"),
        "sampleText" => prop("sampleTexts", "sampleText"),
        "vendorID" => prop("vendorID", "vendorID"),
        "uniqueID" => prop("uniqueID", "uniqueID"),
        _ => None,
    }
}

fn instance_key(font: &Font, instance: &Instance, key: &str) -> Option<String> {
    let prop = |plural: &str, singular: &str| named(&instance.properties, &instance.params, plural, singular);
    let style = || {
        if instance.name.is_empty() { prop("styleNames", "styleName").unwrap_or_default() } else { instance.name.clone() }
    };
    let family = || prop("familyNames", "familyName").unwrap_or_else(|| font.family_name.clone());
    match key {
        "name" => Some(style()),
        "familyName" => Some(family()),
        "fullName" | "postscriptFullName" => {
            Some(prop("postscriptFullNames", "postscriptFullName").unwrap_or_else(|| format!("{} {}", family(), style())))
        }
        "postscriptFontName" | "fontName" => Some(
            prop("postscriptFontName", "postscriptFontName").unwrap_or_else(|| format!("{}-{}", family().split(' ').collect::<String>(), style())),
        ),
        "preferredFamilyName" => Some(prop("preferredFamilyNames", "preferredFamilyName").unwrap_or_else(family)),
        "preferredSubfamilyName" => Some(prop("preferredSubfamilyNames", "preferredSubfamilyName").unwrap_or_else(style)),
        "weightClass" => style_class(instance.weight_class.as_ref(), 400.0),
        "widthClass" => style_class(instance.width_class.as_ref(), 5.0),
        _ => None,
    }
}

fn number_value(font: &Font, master: &Master, key: &str) -> Option<String> {
    let i = font.numbers.iter().position(|n| n.name == key)?;
    number(master.number_values.get(i).copied().unwrap_or(0.0))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::{Anchor, Glyph, Layer, NumberDef, Path, Shape};
    use fontcraft_plist::{Str, Value};

    use super::*;

    fn no_width(_: &str, _: &str) -> Option<f64> {
        None
    }

    fn font() -> Font {
        let mut f = Font::new("Test Sans");
        f.version_major = 2;
        f.version_minor = 5;
        let info = Arc::make_mut(&mut f.info);
        info.properties.set("designers", "Ada", true);
        info.properties.set("copyrights", "© {{{familyName}}}", true);
        info.params.set("vendorID", Value::String(Str::from("TEST")));
        info.date = Some("2026-10-08 12:00:00 +0000".into());
        Arc::make_mut(&mut f.numbers).push(NumberDef { name: "tracking".into(), ..Default::default() });
        if let Some(m) = Arc::make_mut(&mut f.masters).first_mut() {
            m.number_values = vec![12.5];
        }
        let mut zero = Glyph::new("zero");
        let mut layer = Layer::new("m01", 600.0);
        layer.shapes.push(Shape::Path(Path::rect(50.0, -10.0, 530.0, 710.0)));
        layer.anchors.push(Anchor::new("top", 290.0, 720.0));
        zero.layers.push(layer);
        f.glyphs.push(zero);
        f
    }

    fn bold() -> Instance {
        let mut i = Instance { name: "Bold".into(), weight_class: Some(StyleClass::Number(700.0)), ..Default::default() };
        i.properties.set("familyNames", "Test Sans Display", true);
        i
    }

    fn run(f: &Font, master: Option<&Master>, instance: Option<&Instance>, text: &str) -> Expanded {
        expand(text, &TokenContext { font: f, master, instance, glyph_width: &no_width })
    }

    #[test]
    fn font_context_reads_key_paths_and_keeps_unknown_tokens() {
        let f = font();
        let e = run(
            &f,
            None,
            None,
            "{{{familyName}}} v{{{versionMajor}}}.{{{ versionMinor }}} by {{{designer}}} ({{{vendorID}}}) {{{nonsense}}} {{{nonsense}}}",
        );
        assert_eq!(e.text, "Test Sans v2.5 by Ada (TEST) {{{nonsense}}} {{{nonsense}}}");
        assert_eq!(e.unresolved, vec!["nonsense".to_string()]);
        // One level: the copyright's own token is not expanded when it is the value of another.
        assert_eq!(run(&f, None, None, "{{{copyright}}}").text, "© {{{familyName}}}");
        assert_eq!(run(&f, None, None, "{{{date}}} {{{unitsPerEm}}}").text, "2026-10-08 12:00:00 +0000 1000");
    }

    #[test]
    fn app_tokens_work_in_every_context() {
        let f = font();
        let i = bold();
        let m = f.masters.first().cloned();
        for (master, instance) in [(None, None), (m.as_ref(), None), (None, Some(&i))] {
            let e = run(&f, master, instance, "{{{versionString}}}/{{{buildNumber}}}");
            assert_eq!(e.text, format!("{APP_VERSION}/{BUILD_NUMBER}"));
            assert!(e.unresolved.is_empty());
        }
    }

    #[test]
    fn instance_context_reads_the_instance_then_the_font() {
        let f = font();
        let i = bold();
        let e = run(&f, None, Some(&i), "{{{fullName}}}|{{{postscriptFontName}}}|{{{weightClass}}}|{{{widthClass}}}|{{{designer}}}");
        assert_eq!(e.text, "Test Sans Display Bold|TestSansDisplay-Bold|700|5|Ada");
        let mut j = Instance { name: "Light".into(), ..Default::default() };
        j.params.set("postscriptFullName", Value::String(Str::from("Test Sans Lite")));
        assert_eq!(run(&f, None, Some(&j), "{{{familyName}}}: {{{fullName}}}").text, "Test Sans: Test Sans Lite");
    }

    #[test]
    fn master_context_reads_number_values_only() {
        let f = font();
        let m = f.masters.first().cloned();
        let e = run(&f, m.as_ref(), None, "{{{tracking}}} {{{familyName}}}");
        assert_eq!(e.text, "12.5 {{{familyName}}}");
        assert_eq!(e.unresolved, vec!["familyName".to_string()]);
        // At the font level a number value names nothing.
        assert_eq!(run(&f, None, None, "{{{tracking}}}").unresolved, vec!["tracking".to_string()]);
    }

    #[test]
    fn glyph_properties_read_the_layer() {
        let f = font();
        let e = run(&f, None, None, "{{{zero:width}}} {{{zero:LSB}}} {{{zero:RSB}}} {{{zero:anchors.top.y}}} {{{zero:bounds.size.height}}}");
        assert_eq!(e.text, "600 50 70 720 720");
        let wide = |g: &str, _: &str| (g == "zero").then_some(640.0);
        let e = expand("{{{zero:width}}} {{{zero:RSB}}}", &TokenContext { font: &f, master: None, instance: None, glyph_width: &wide });
        assert_eq!(e.text, "640 110");
        let e = run(&f, None, None, "{{{nothing:width}}} {{{zero:TSB}}} {{{space:LSB}}}");
        assert_eq!(e.unresolved, vec!["nothing:width", "zero:TSB", "space:LSB"]);
    }

    #[test]
    fn the_grammar_takes_the_shortest_token_on_one_line() {
        let f = font();
        assert_eq!(run(&f, None, None, "{{{{familyName}}}}").text, "{{{{familyName}}}}");
        assert_eq!(run(&f, None, None, "{{{\nfamilyName}}} {{{familyName}}}").text, "{{{\nfamilyName}}} Test Sans");
        let e = run(&f, None, None, "{{{ }}} {{{familyName");
        assert_eq!((e.text.as_str(), e.unresolved), ("{{{ }}} {{{familyName", vec!["".to_string()]));
        // At least one character inside: `{{{}}}}` reads `}`.
        assert_eq!(run(&f, None, None, "{{{}}}}").unresolved, vec!["}".to_string()]);
        assert_eq!(run(&f, None, None, "ü{{{familyName}}}é").text, "üTest Sansé");
        assert!(!has_tokens("{{familyName}}"));
    }

    #[test]
    fn the_limits_keep_tokens_as_written() {
        let f = font();
        let many = "{{{upm}}}".repeat(MAX_TOKENS + 5);
        let e = run(&f, None, None, &many);
        assert_eq!(e.text, format!("{}{}", "1000".repeat(MAX_TOKENS), "{{{upm}}}".repeat(5)));
        // A value that would make the text too long stays a token.
        let mut g = font();
        let long = "x".repeat(MAX_EXPANDED_BYTES);
        Arc::make_mut(&mut g.info).properties.set("descriptions", &long, true);
        let e = run(&g, None, None, "a {{{description}}} {{{upm}}}");
        assert_eq!(e.text, "a {{{description}}} 1000");
        assert!(e.unresolved.is_empty());
        assert!(e.text.len() <= MAX_EXPANDED_BYTES);
    }

    #[test]
    fn numbers_are_written_plainly() {
        assert_eq!(number(600.0).as_deref(), Some("600"));
        assert_eq!(number(-0.0).as_deref(), Some("0"));
        assert_eq!(number(12.3456).as_deref(), Some("12.346"));
        assert_eq!(number(f64::NAN), None);
    }
}
