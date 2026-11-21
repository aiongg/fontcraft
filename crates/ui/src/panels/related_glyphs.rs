//! Related Glyphs & Characters: what is connected to the shown glyph, one section per list that has entries.
//!
//! | Section | Lists |
//! |---|---|
//! | Alternates | glyphs with the same base name and another suffix |
//! | Components | the glyphs this glyph's components refer to |
//! | Used as Component In | the glyphs with a component of this glyph |
//! | Decomposes Into | the characters the glyph's characters decompose into |
//! | Contained In | the characters whose decomposition contains the glyph's characters |
//!
//! An entry the font has a glyph for is a link that shows that glyph; a character without a glyph is muted text.

use craft::ui::buttons::Link;
use craft::ui::contract::{Show as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::Section;
use egui::Ui;

use crate::app::models::glyph_sections::{RelatedKind, RelatedModel};
use crate::app::{Msg, ViewMsg};

craft::ui::words! {
    /// The words of the Related Glyphs & Characters panel.
    pub struct RelatedWords {
        alternates: "fontcraftRelated.alternates" = "Alternates";
        components: "fontcraftRelated.components" = "Components";
        used_by: "fontcraftRelated.usedBy" = "Used as Component In";
        decomposes_into: "fontcraftRelated.decomposesInto" = "Decomposes Into";
        contained_in: "fontcraftRelated.containedIn" = "Contained In";
        /// Shown without a glyph.
        no_glyph: "fontcraftRelated.noGlyph" = "No glyph selected";
        /// Shown when every list is empty.
        nothing: "fontcraftRelated.nothing" = "Nothing related";
    }
}

fn title(words: &RelatedWords, kind: RelatedKind) -> Label {
    match kind {
        RelatedKind::Alternates => words.alternates.clone(),
        RelatedKind::Components => words.components.clone(),
        RelatedKind::UsedBy => words.used_by.clone(),
        RelatedKind::DecomposesInto => words.decomposes_into.clone(),
        RelatedKind::ContainedIn => words.contained_in.clone(),
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<RelatedModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<RelatedWords>();
    let model: &RelatedModel = props.model;
    if model.glyph.is_none() {
        Text::new(words.no_glyph.clone()).shade(Shade::Muted).show(ui);
        return;
    }
    if model.sections.is_empty() {
        Text::new(words.nothing.clone()).shade(Shade::Muted).show(ui);
        return;
    }
    let sent = &mut *props.sent;
    for section in &model.sections {
        Section::new(title(&words, section.kind)).show(ui, |ui, _| {
            let _ = Row::new().wrap(true).show(ui, |ui, _| {
                for entry in &section.entries {
                    match &entry.glyph {
                        Some(glyph) => {
                            if Link::new(entry.label.clone()).show(ui).clicked() {
                                sent.push(Msg::View(ViewMsg::SelectGlyph(glyph.clone())));
                            }
                        }
                        None => {
                            Text::new(entry.label.clone()).shade(Shade::Muted).show(ui);
                        }
                    }
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::contract::Tracked;
    use craft::ui::testing::Fixture;
    use fontcraft_engine::Session;
    use serde_json::json;

    use super::{Props, show};
    use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
    use crate::shell::dock::PanelId;

    /// The starter font plus `circumflexcomb`, `Acircumflex` (A + circumflexcomb) and `Acircumflex.ss01` (a component
    /// of Acircumflex), with Acircumflex shown and the panel open.
    fn app() -> FontCraftApp {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        for (name, unicode) in [("circumflexcomb", Some("0302")), ("Acircumflex", Some("00C2")), ("Acircumflex.ss01", None)] {
            app.sent.push(Msg::run("glyph.newGlyph", json!({ "name": name })));
            if let Some(u) = unicode {
                app.sent.push(Msg::run("glyph.setUnicodes", json!({ "glyphs": [name], "unicodes": [u] })));
            }
        }
        for (glyph, part) in [("Acircumflex", "A"), ("Acircumflex", "circumflexcomb"), ("Acircumflex.ss01", "Acircumflex")] {
            app.sent.push(Msg::run("glyph.addComponent", json!({ "glyph": glyph, "component": part })));
        }
        app.sent.push(Msg::View(ViewMsg::SelectGlyph("Acircumflex".into())));
        app.sent.push(Msg::View(ViewMsg::TogglePanel(PanelId::RelatedGlyphs)));
        app.drain();
        assert!(app.status.is_empty(), "{}", app.status);
        app
    }

    /// The panel lists the five sections from the engine, and a click on a glyph entry shows that glyph.
    #[test]
    fn related_glyphs_lists_the_sections_and_a_click_shows_the_glyph() {
        let app = app();
        let names = |i: usize| app.models.related_glyphs.sections[i].entries.iter().map(|e| e.label.clone()).collect::<Vec<_>>();
        assert_eq!(app.models.related_glyphs.sections.len(), 5, "{:?}", app.models.related_glyphs.sections);
        assert_eq!(names(0), ["Acircumflex.ss01"]);
        assert_eq!(names(1), ["A", "circumflexcomb"]);
        assert_eq!(names(2), ["Acircumflex.ss01"]);
        // U+1EA4 decomposes into U+00C2 U+0301; the font has no glyph for it.
        assert!(names(4).contains(&"\u{1EA4} U+1EA4".to_string()), "{:?}", names(4));

        let model = Tracked::new((*app.models.related_glyphs).clone());
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let link = fixture.by_label("circumflexcomb").expect("the component link");
        fixture.click(&link).settle();
        assert_eq!(*posted.borrow(), vec![Msg::View(ViewMsg::SelectGlyph("circumflexcomb".into()))]);
    }
}
