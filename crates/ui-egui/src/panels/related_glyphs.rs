//! Palette › Related Glyphs & Characters (`palette.relatedGlyphs`): what is connected to the
//! current glyph, from [`fontcraft_engine::cmd::related::related_glyphs`].
//!
//! | Section | Lists |
//! |---|---|
//! | Alternates | glyphs with the same base name and another suffix |
//! | Components | the glyphs this glyph's components refer to |
//! | Used as Component In | the glyphs with a component of this glyph |
//! | Decomposes Into | the characters the glyph's characters decompose into |
//! | Contained In | the characters whose decomposition contains the glyph's characters |
//!
//! A section shows only when it has entries, and each collapses on its own. Clicking an entry
//! the font has a glyph for opens that glyph in a new tab (`view.newTab`); characters without a
//! glyph are dimmed. The query result is kept until the font or the current glyph changes.

use std::cell::RefCell;
use std::sync::Arc;

use fontcraft_engine::cmd::related::{RelatedChar, RelatedGlyphs, related_glyphs};
use fontcraft_model::Font;
use serde_json::json;

use super::{Pending, current_glyph, palette};
use crate::app::FontcraftApp;

/// One clickable entry of a section.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub label: String,
    /// The glyph a click opens; `None` for a character the font has no glyph for.
    pub glyph: Option<String>,
}

/// One section: (id, title, entries).
pub type Section = (&'static str, &'static str, Vec<Entry>);

struct Cached {
    font: Arc<Font>,
    glyph: String,
    related: Arc<RelatedGlyphs>,
}

thread_local! {
    static CACHE: RefCell<Option<Cached>> = const { RefCell::new(None) };
}

fn glyph_entry(name: &str) -> Entry {
    Entry { label: name.to_string(), glyph: Some(name.to_string()) }
}

fn char_entry(c: &RelatedChar) -> Entry {
    let shown = char::from_u32(c.code_point).filter(|ch| !ch.is_control()).map(String::from).unwrap_or_default();
    let label = match &c.glyph {
        Some(name) => format!("{shown} {name}"),
        None => format!("{shown} U+{:04X}", c.code_point),
    };
    Entry { label: label.trim_start().to_string(), glyph: c.glyph.clone() }
}

/// The five sections in display order, empty ones included.
pub fn sections(r: &RelatedGlyphs) -> [Section; 5] {
    [
        ("alternates", "Alternates", r.alternates.iter().map(|n| glyph_entry(n)).collect()),
        ("components", "Components", r.components.iter().map(|n| glyph_entry(n)).collect()),
        ("usedBy", "Used as Component In", r.used_by.iter().map(|n| glyph_entry(n)).collect()),
        ("decomposesInto", "Decomposes Into", r.decomposes_into.iter().map(char_entry).collect()),
        ("decomposedFrom", "Contained In", r.decomposed_from.iter().map(char_entry).collect()),
    ]
}

/// The related glyphs of `glyph` in the active font, from the cache when the font is the same.
pub fn related(app: &FontcraftApp, glyph: &str) -> Option<Arc<RelatedGlyphs>> {
    let font = app.session.doc().ok()?.font.clone();
    let hit = CACHE.with(|c| c.borrow().as_ref().filter(|c| Arc::ptr_eq(&c.font, &font) && c.glyph == glyph).map(|c| c.related.clone()));
    if hit.is_some() {
        return hit;
    }
    let data = app.session.glyph_data().ok()?;
    let related = Arc::new(related_glyphs(&font, &data, glyph)?);
    CACHE.with(|c| *c.borrow_mut() = Some(Cached { font, glyph: glyph.to_string(), related: related.clone() }));
    Some(related)
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let Some(glyph) = current_glyph(app) else {
        palette::note(app, ui, "No glyph selected");
        return;
    };
    let Some(related) = related(app, &glyph) else {
        palette::note(app, ui, "No glyph selected");
        return;
    };
    let mut any = false;
    for (id, title, entries) in sections(&related) {
        if entries.is_empty() {
            continue;
        }
        any = true;
        egui::CollapsingHeader::new(title).id_salt(("relatedGlyphs", id)).default_open(true).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for e in &entries {
                    match &e.glyph {
                        Some(g) => {
                            if ui.link(&e.label).clicked() {
                                pending.push(("view.newTab", json!({ "glyphs": [g] })));
                            }
                        }
                        None => {
                            ui.label(egui::RichText::new(&e.label).color(app.tokens.text_dim));
                        }
                    }
                }
            });
        });
    }
    if !any {
        palette::note(app, ui, "Nothing related");
    }
}

#[cfg(test)]
mod tests {
    use egui::Pos2;
    use fontcraft_model::{Component, Glyph, Layer, Shape};

    use super::*;

    fn glyph(name: &str, cp: Option<u32>, parts: &[&str]) -> Glyph {
        let mut g = Glyph::new(name);
        g.unicodes.extend(cp);
        let mut l = Layer::new("m01", 600.0);
        for part in parts {
            l.shapes.push(Shape::Component(Component::new(*part, 0.0, 0.0)));
        }
        g.layers.push(l);
        g
    }

    /// The starter font plus `circumflexcomb`, `Acircumflex` (A + circumflexcomb) and
    /// `Acircumflex.ss01` (a component of Acircumflex), with Acircumflex selected.
    fn app() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        app.session
            .edit("fixture", |f| {
                f.glyphs.push(glyph("circumflexcomb", Some(0x302), &[]));
                f.glyphs.push(glyph("Acircumflex", Some(0xC2), &["A", "circumflexcomb"]));
                f.glyphs.push(glyph("Acircumflex.ss01", None, &["Acircumflex"]));
                Ok(())
            })
            .unwrap();
        app.state.selected_glyphs = vec!["Acircumflex".into()];
        app
    }

    #[test]
    fn related_glyphs_section_lists_all_five_sections() {
        let mut app = app();
        let r = related(&app, "Acircumflex").expect("the glyph exists");
        let s = sections(&r);
        let names = |i: usize| s[i].2.iter().map(|e| e.label.clone()).collect::<Vec<_>>();
        assert_eq!(names(0), ["Acircumflex.ss01"]);
        assert_eq!(names(1), ["A", "circumflexcomb"]);
        assert_eq!(names(2), ["Acircumflex.ss01"]);
        assert_eq!(s[3].2.iter().map(|e| e.glyph.clone()).collect::<Vec<_>>(), [Some("A".to_string()), Some("circumflexcomb".to_string())]);
        // U+1EA4 decomposes into U+00C2 U+0301; the font has no glyph for it.
        assert!(s[4].2.contains(&Entry { label: "\u{1EA4} U+1EA4".into(), glyph: None }), "{:?}", s[4].2);

        // A headless frame draws the section without queueing anything.
        let ctx = egui::Context::default();
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 800.0))), ..Default::default() };
        let mut pending = Pending::new();
        ctx.run_ui(input, |ui| show(&mut app, ui, &mut pending)).textures_delta.clear();
        assert!(pending.is_empty());

        // An edit invalidates the cached result.
        app.session
            .edit("alt", |f| {
                f.glyphs.push(glyph("Acircumflex.sc", None, &[]));
                Ok(())
            })
            .unwrap();
        assert_eq!(related(&app, "Acircumflex").unwrap().alternates, ["Acircumflex.ss01", "Acircumflex.sc"]);
    }
}
