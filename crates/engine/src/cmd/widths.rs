//! The width a new glyph or layer gets: the *Default Layer Width* parameter
//! (`docs/spec/18-custom-parameters.md`), else the built-in widths. New Glyph, Add Glyphs,
//! Generate, pasted and imported glyphs and the master layers that drawing commands add all
//! take their width from here.

use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::plist::Value as PValue;
use fontcraft_model::{Font, Layer, data_props, glyph_props};
use fontcraft_spacing::WIDTH_RANGE;

/// The custom parameter that sets the width of new glyphs and layers.
pub(crate) const DEFAULT_LAYER_WIDTH: &str = "Default Layer Width";

/// The width of a new layer when no *Default Layer Width* parameter applies.
pub(crate) const BUILT_IN_LAYER_WIDTH: f64 = 600.0;

/// The built-in width of a new layer of a CJK glyph.
const CJK_LAYER_WIDTH: f64 = 1000.0;

/// The scripts whose glyphs get [`CJK_LAYER_WIDTH`].
const CJK_SCRIPTS: [&str; 4] = ["han", "hangul", "kana", "bopomofo"];

/// The built-in width of a new layer of a corner component, and the prefix that names one.
const CORNER_LAYER_WIDTH: f64 = 200.0;
const CORNER_PREFIX: &str = "_corner.";

/// The width a new layer of `glyph` gets on `master`. The glyph's script is the one it stores,
/// else the one `data` gives its name and code points; a glyph not in the font yet is looked up
/// by name alone.
pub(crate) fn new_layer_width(font: &Font, data: Option<&GlyphData>, glyph: &str, master: &str) -> f64 {
    let script = match (font.glyph(glyph), data) {
        (Some(g), _) if g.info.script.is_some() => g.info.script.clone(),
        (Some(g), Some(data)) => glyph_props(g, data, false).script.value,
        (None, Some(data)) => data_props(glyph, &[], data, false).script.value,
        (_, None) => None,
    };
    width_for(font, script.map(|s| s.to_lowercase()).as_deref(), glyph, master)
}

/// An empty layer for every master of `font`, for a new glyph `name` with `unicodes`, each as
/// wide as [`new_layer_width`] says for that master.
pub(crate) fn new_glyph_layers(font: &Font, data: Option<&GlyphData>, name: &str, unicodes: &[u32]) -> Vec<Layer> {
    let script = data.and_then(|data| data_props(name, unicodes, data, false).script.value).map(|s| s.to_lowercase());
    font.masters.iter().map(|m| Layer::new(m.id.as_str(), width_for(font, script.as_deref(), name, &m.id))).collect()
}

/// The *Default Layer Width* entry for `script`, else the entry without a script, else the
/// built-in width. Entries come from the master's parameters, then the font's; an entry that is
/// not a width is ignored.
fn width_for(font: &Font, script: Option<&str>, glyph: &str, master: &str) -> f64 {
    let master_params = font.masters.iter().find(|m| m.id == master).map(|m| &m.params);
    let entries: Vec<(Option<String>, f64)> = master_params
        .into_iter()
        .flat_map(|params| params.all(DEFAULT_LAYER_WIDTH))
        .chain(font.info.params.all(DEFAULT_LAYER_WIDTH))
        .filter_map(width_entry)
        .collect();
    let for_script = script.and_then(|sc| entries.iter().find(|(s, _)| s.as_deref() == Some(sc)));
    match for_script.or_else(|| entries.iter().find(|(s, _)| s.is_none())) {
        Some((_, width)) => *width,
        None if glyph.starts_with(CORNER_PREFIX) => CORNER_LAYER_WIDTH,
        None if script.is_some_and(|s| CJK_SCRIPTS.contains(&s)) => CJK_LAYER_WIDTH,
        None => BUILT_IN_LAYER_WIDTH,
    }
}

/// One *Default Layer Width* value: a number, or text holding a number with an optional
/// `script:` prefix.
fn width_entry(v: &PValue) -> Option<(Option<String>, f64)> {
    let (script, number) = match v {
        PValue::Number(n) => (None, n.value().as_f64()),
        PValue::String(s) => match s.as_str().split_once(':') {
            Some((script, rest)) => (Some(script.trim().to_lowercase()), rest.trim().parse::<f64>().ok()?),
            None => (None, s.as_str().trim().parse::<f64>().ok()?),
        },
        _ => return None,
    };
    (number.is_finite() && WIDTH_RANGE.contains(&number)).then_some((script, number))
}
