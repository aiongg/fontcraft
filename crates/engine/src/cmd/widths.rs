//! The width a new glyph or layer gets: the *Default Layer Width* parameter
//! (`docs/spec/18-custom-parameters.md`), else the built-in widths. New Glyph, Add Glyphs,
//! Generate, pasted and imported glyphs and the master layers that drawing commands add all
//! take their width from here.
//!
//! Colour layers carry their master layer's width (`color.layers.widthFollowsMaster`):
//! [`color_layers_follow_master`] runs inside every edit.

use std::sync::Arc;

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
    width_for(font, script_of(font, data, glyph).as_deref(), glyph, master)
}

/// The script of `glyph` in lower case: the one it stores, else the one `data` gives its name
/// and code points; a glyph not in the font yet is looked up by name alone.
fn script_of(font: &Font, data: Option<&GlyphData>, glyph: &str) -> Option<String> {
    let script = match (font.glyph(glyph), data) {
        (Some(g), _) if g.info.script.is_some() => g.info.script.clone(),
        (Some(g), Some(data)) => glyph_props(g, data, false).script.value,
        (None, Some(data)) => data_props(glyph, &[], data, false).script.value,
        (_, None) => None,
    };
    script.map(|s| s.to_lowercase())
}

/// Whether `glyph` belongs to one of the [`CJK_SCRIPTS`], by [`script_of`].
pub(crate) fn is_cjk(font: &Font, data: Option<&GlyphData>, glyph: &str) -> bool {
    script_of(font, data, glyph).is_some_and(|s| CJK_SCRIPTS.contains(&s.as_str()))
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

/// Gives the colour layers of every glyph `next` changed the width of their master layer, where
/// that master layer's width differs from `before`'s. Called by [`crate::Session::edit`] on the
/// edited copy, so the colour layers move in the same undo step as the master layer. Glyphs the
/// edit did not touch are not looked into, and only glyphs with a colour layer to change are
/// copied.
pub(crate) fn color_layers_follow_master(next: &mut Font, before: &Font) {
    let (new, old) = (next.glyphs.as_arcs(), before.glyphs.as_arcs());
    if std::ptr::eq(new, old) {
        return;
    }
    let changed: Vec<usize> = new
        .iter()
        .enumerate()
        .filter(|(i, g)| {
            let was = match old.get(*i) {
                Some(b) if b.name == g.name => Some(b),
                _ => before.glyph(&g.name),
            };
            let Some(was) = was.filter(|b| !Arc::ptr_eq(b, g)) else { return false };
            g.layers.iter().any(|l| {
                !l.is_master_layer()
                    && fontcraft_color::layers::kind(l).is_some()
                    && g.layer(&l.master).is_some_and(|m| m.width != l.width && was.layer(&m.id).is_some_and(|b| b.width != m.width))
            })
        })
        .map(|(i, _)| i)
        .collect();
    for i in changed {
        let Some(mut g) = next.glyphs.get_mut_at(i) else { continue };
        let widths: Vec<(String, f64)> = g.layers.iter().filter(|l| l.is_master_layer()).map(|l| (l.id.clone(), l.width)).collect();
        for layer in g.layers.iter_mut().filter(|l| !l.is_master_layer() && fontcraft_color::layers::kind(l).is_some()) {
            if let Some((_, width)) = widths.iter().find(|(id, _)| *id == layer.master) {
                layer.width = *width;
            }
        }
    }
}

#[cfg(test)]
mod measure {
    use std::time::Instant;

    use fontcraft_model::Font;

    use super::color_layers_follow_master;

    #[test]
    #[ignore = "measure: the cost of the colour-layer hook on a one-glyph edit of a large source"]
    fn color_layer_hook_cost_on_one_glyph_edit() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/inter/src/Inter-Roman.glyphspackage");
        let Ok((font, _)) = crate::io::open(&path, None) else { return };
        let name = font.glyphs.iter().find(|g| g.name == "A").map(|g| g.name.clone()).unwrap_or_default();
        const RUNS: u32 = 200;
        let mut total = std::time::Duration::ZERO;
        for _ in 0..RUNS {
            let mut next = Font::clone(&font);
            if let Some(mut g) = next.glyphs.get_mut(&name)
                && let Some(l) = g.layers.first_mut()
            {
                l.width += 1.0;
            }
            let t = Instant::now();
            color_layers_follow_master(&mut next, &font);
            total += t.elapsed();
        }
        eprintln!("{} glyphs: {:?} per edit", font.glyphs.len(), total / RUNS);
    }
}
