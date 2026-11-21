//! The reference font's glyph behind the outline (`view.setReferenceFont`, layer `referenceFont`).
//!
//! | Item | Drawn as | Shown when |
//! |---|---|---|
//! | the selected reference font's glyph for the edited character | thin outline at the edited glyph's origin, `reference_outline` | the Reference Font panel's Show is on |
//!
//! The glyph is the one [`Reference::glyph_for`] finds (the custom character, else the edited glyph's first code point,
//! else its name; then its localised form), in the reference font's first master ([`Reference::layer`]), scaled from
//! the reference font's units per em to the edited font's. [`refresh`] (after `drain`) builds the outline only when
//! what it depends on changed, and puts it in `OverlayData::reference`; painting reads it alone.

use std::sync::Arc;

use egui::{Shape, Stroke};
use fontcraft_engine::model::{Glyph, Shape as ModelShape};
use fontcraft_geom::{Affine, BezPath};

use super::layers::PaintCx;
use crate::app::FontCraftApp;
use crate::shell::reference_font::{Loaded, Reference, ReferenceFontState};

/// Most outline elements kept for the reference glyph; components past this are left out.
const MAX_ELEMENTS: usize = 200_000;

/// What the reference outline was last built from, and the outline.
#[derive(Clone, Debug, Default)]
pub struct ReferenceCache {
    key: Option<Key>,
    outline: Option<Arc<BezPath>>,
}

/// What the reference outline depends on.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    loaded: Loaded,
    character: String,
    language: String,
    /// The edited glyph's name and code points.
    glyph: String,
    unicodes: Vec<u32>,
    /// The edited font's units per em.
    upm: f64,
}

/// Brings `OverlayData::reference` up to date. Runs after `overlays::refresh`, which may have cleared it.
pub fn refresh(app: &mut FontCraftApp) {
    let key = key(app);
    if key != app.overlays.reference.key {
        let outline = key.as_ref().and_then(|_| build(app)).map(Arc::new);
        app.overlays.reference = ReferenceCache { key, outline };
    }
    let same = match (&app.overlays.data.reference, &app.overlays.reference.outline) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    };
    if !same {
        app.overlays.data.reference = app.overlays.reference.outline.clone();
    }
}

/// The key of the outline to draw: `None` while nothing is to be drawn.
fn key(app: &FontCraftApp) -> Option<Key> {
    let state = &app.view.reference_font;
    if !state.show || state.loaded.0.is_none() {
        return None;
    }
    let font = app.session.font().ok()?;
    let glyph = font.glyph(super::tools::edited_glyph(font, &app.view)?)?;
    Some(Key {
        loaded: state.loaded.clone(),
        character: state.character.clone(),
        language: state.language.clone(),
        glyph: glyph.name.clone(),
        unicodes: glyph.unicodes.clone(),
        upm: f64::from(font.units_per_em.max(1)),
    })
}

fn build(app: &FontCraftApp) -> Option<BezPath> {
    let state = &app.view.reference_font;
    let reference = state.loaded.0.as_deref()?;
    let font = app.session.font().ok()?;
    let edited = font.glyph(super::tools::edited_glyph(font, &app.view)?)?;
    outline(reference, edited, state, f64::from(font.units_per_em.max(1)))
}

/// The reference glyph for `edited` in the edited font's units (`upm` units per em), components resolved.
pub fn outline(reference: &Reference, edited: &Glyph, state: &ReferenceFontState, upm: f64) -> Option<BezPath> {
    let glyph = reference.glyph_for(edited, &state.character, &state.language)?;
    let layer = reference.layer(glyph)?;
    let mut out = BezPath::new();
    for shape in &layer.shapes {
        let part = match shape {
            ModelShape::Path(p) => p.to_bez().ok(),
            ModelShape::Component(c) => reference.font.component_outline(c, &layer.master).ok(),
            _ => None,
        };
        let Some(part) = part else { continue };
        if out.elements().len().saturating_add(part.elements().len()) > MAX_ELEMENTS {
            break;
        }
        out.extend(part);
    }
    let k = upm / f64::from(reference.font.units_per_em.max(1));
    if out.elements().is_empty() || !(k.is_finite() && k > 0.0) {
        return None;
    }
    Some(Affine::scale(k) * out)
}

pub fn paint(cx: &PaintCx<'_>) {
    cx.painter.extend(shapes(cx));
}

/// The reference glyph's outline, while the canvas edits a glyph.
pub fn shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let Some(outline) = cx.overlays.data.reference.as_deref() else { return Vec::new() };
    if cx.glyph().is_none() {
        return Vec::new();
    }
    let stroke = Stroke::new(cx.tokens.reference_outline_width, cx.tokens.reference_outline);
    super::paint::outline_shapes(cx, outline, stroke)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use fontcraft_engine::Session;
    use fontcraft_engine::io::SaveFormat;
    use fontcraft_engine::model::{Font, Path, Shape};
    use serde_json::json;

    use crate::app::{APP, FontCraftApp, Msg, ViewMsg};

    /// A reference font file: glyph A (U+0041) holds a 500-unit square, glyph B (U+0042) a 100-unit one, at 2000 units
    /// per em.
    fn reference_file(dir: &std::path::Path) -> String {
        let mut font = Font::new("Reference");
        font.units_per_em = 2000;
        for (name, size) in [("A", 500.0), ("B", 100.0)] {
            let mut glyph = font.glyph_mut(name).unwrap();
            glyph.layers[0].shapes = vec![Shape::Path(Path::rect(0.0, 0.0, size, size))];
        }
        let path = dir.join("reference.glyphs");
        fontcraft_engine::io::save(&font, &path, SaveFormat::Glyphs).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn shell(app: &mut FontCraftApp, params: serde_json::Value) {
        app.sent.push(Msg::Shell(CommandRef::with_params("view.setReferenceFont", params)));
        app.drain();
        assert!(app.status.is_empty(), "{}", app.status);
    }

    /// The x extent of the drawn reference outline, in the edited font's units.
    fn width(app: &FontCraftApp) -> Option<f64> {
        use fontcraft_geom::Shape as _;
        app.overlays.data.reference.as_deref().map(|o| o.bounding_box().width())
    }

    /// Adding a reference font draws its glyph for the edited character, scaled to the edited font's units per em; the
    /// custom character picks another glyph, and Show off hides it.
    #[test]
    fn the_reference_glyph_is_drawn_behind_the_outline() {
        let dir = std::env::temp_dir().join(format!("fontcraft-ui-reference-canvas-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = reference_file(&dir);
        let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
        app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
        app.drain();
        assert_eq!(width(&app), None, "no reference font yet");

        shell(&mut app, json!({"path": path}));
        let upm = f64::from(app.session.font().unwrap().units_per_em);
        let w = width(&app).expect("adding a font shows it");
        assert!((w - 500.0 * upm / 2000.0).abs() < 1e-6, "scaled to the edited font: {w}");

        shell(&mut app, json!({"character": "B"}));
        let w = width(&app).unwrap();
        assert!((w - 100.0 * upm / 2000.0).abs() < 1e-6, "the custom character's glyph: {w}");

        shell(&mut app, json!({"show": false}));
        assert_eq!(width(&app), None, "Show off hides it");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
