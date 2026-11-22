//! Reference Font: fonts to draw against. The file field adds a font (Enter), the list selects one or removes it, Show
//! draws its glyph behind the edited outline, and two fields set a custom character and a language code. Every control
//! runs `view.setReferenceFont` (`shell::reference_font`).

use craft::ui::buttons::{Button, ToggleButton};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::{Value, json};

use crate::app::Msg;
use crate::app::models::glyph_sections::ReferenceFontModel;

craft::ui::words! {
    /// The words of the Reference Font panel.
    pub struct ReferenceFontWords {
        /// The field a font's path is typed into.
        font_file: "fontcraftReferenceFont.fontFile" = "Font file";
        empty: "fontcraftReferenceFont.empty" = "Add a font file to draw against";
        unread: "fontcraftReferenceFont.unread" = "Click the font to read it again";
        remove: "fontcraftReferenceFont.remove" = "Remove";
        remove_tip: "fontcraftReferenceFont.removeTip" = "Take this font off the list";
        show: "fontcraftReferenceFont.show" = "Show";
        show_tip: "fontcraftReferenceFont.showTip" = "Draw the reference glyph behind the outline";
        character: "fontcraftReferenceFont.character" = "Character";
        language: "fontcraftReferenceFont.language" = "Language";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<ReferenceFontModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    super::widgets::scrolling(ui, |ui| view(&mut props, ui));
}

fn set(params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params("view.setReferenceFont", params))
}

/// A text field over `value` that gives the text committed.
fn committed(ui: &mut Ui, field: TextField<'_>) -> Option<String> {
    field.show(ui).events.committed().cloned()
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<ReferenceFontWords>();
    let model: &ReferenceFontModel = props.model;
    let sent = &mut *props.sent;
    if let Some(path) = committed(ui, TextField::new(words.font_file.clone(), Bind::given(String::new())).placeholder(words.font_file.clone()))
        && !path.trim().is_empty()
    {
        sent.push(set(json!({ "path": path.trim() })));
    }
    if model.fonts.is_empty() {
        Text::new(words.empty.clone()).shade(Shade::Muted).show(ui);
    }
    for font in &model.fonts {
        let _ = Row::new().show(ui, |ui, _| {
            let mut on = font.selected;
            ToggleButton::new(font.name.clone(), Bind::bound(&mut on)).tooltip(font.path.clone()).show(ui);
            if on != font.selected && on {
                sent.push(set(json!({ "path": font.path })));
            }
            if Button::new(words.remove.clone()).tooltip(words.remove_tip.clone()).show(ui).clicked() {
                sent.push(set(json!({ "remove": font.path })));
            }
        });
    }
    if model.unread {
        Text::new(words.unread.clone()).shade(Shade::Muted).show(ui);
    }
    let mut show = model.show;
    ToggleButton::new(words.show.clone(), Bind::bound(&mut show)).tooltip(words.show_tip.clone()).show(ui);
    if show != model.show {
        sent.push(set(json!({ "show": show })));
    }
    let _ = Row::new().show(ui, |ui, _| {
        if let Some(c) = committed(ui, TextField::new(words.character.clone(), Bind::given(model.character.clone())))
            && c != model.character
        {
            sent.push(set(json!({ "character": c })));
        }
        if let Some(l) = committed(ui, TextField::new(words.language.clone(), Bind::given(model.language.clone())))
            && l != model.language
        {
            sent.push(set(json!({ "language": l })));
        }
    });
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::contract::Tracked;
    use craft::ui::runtime::commands::CommandRef;
    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};
    use fontcraft_engine::Session;
    use serde_json::json;

    use super::{Props, show};
    use crate::app::models::glyph_sections::ReferenceFontModel;
    use crate::app::{APP, FontCraftApp, Msg};

    fn shell(params: serde_json::Value) -> Msg {
        Msg::Shell(CommandRef::with_params("view.setReferenceFont", params))
    }

    /// Typing a path and pressing Enter posts the font to add; Show turns the display off.
    #[test]
    fn a_typed_path_adds_the_font_and_show_toggles() {
        let model = Tracked::new(ReferenceFontModel { show: true, ..ReferenceFontModel::default() });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let field = fixture.by_label("Font file").expect("the font file field");
        fixture.click(&field).type_text("/fonts/Ref.ttf").key(Key::Enter, Modifiers::NONE).settle();
        let toggle = fixture.by_label("Show").expect("the Show toggle");
        fixture.click(&toggle).settle();
        assert_eq!(*posted.borrow(), vec![shell(json!({ "path": "/fonts/Ref.ttf" })), shell(json!({ "show": false }))]);
    }

    /// A compiled font is read and selected, its localised form is found by the language code, and a file that is not
    /// a font is refused without changing anything.
    #[test]
    fn view_set_reference_font_reads_selects_and_refuses() {
        let dir = std::env::temp_dir().join(format!("fontcraft-ui-reference-font-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ttf = dir.join("Reference.ttf");
        let mut src = Session::with_document();
        src.execute("path.addRectangle", &json!({"glyph": "A", "x": 0, "y": 0, "width": 500, "height": 500})).unwrap();
        src.execute("glyph.newGlyph", &json!({"name": "A.loclTRK"})).unwrap();
        src.execute("path.addRectangle", &json!({"glyph": "A.loclTRK", "x": 0, "y": 0, "width": 200, "height": 200})).unwrap();
        src.execute("file.exportStatic", &json!({"destination": ttf})).unwrap();

        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        let path = ttf.to_string_lossy().to_string();
        app.sent.push(shell(json!({ "path": path })));
        app.sent.push(shell(json!({ "language": "trk" })));
        app.drain();
        assert!(app.status.is_empty(), "{}", app.status);
        let state = app.view.reference_font.clone();
        assert_eq!((state.paths.clone(), state.show), (vec![path.clone()], true));
        assert_eq!(app.models.reference_font.fonts.first().map(|f| (f.name.as_str(), f.selected)), Some(("Reference.ttf", true)));
        let reference = state.loaded.0.clone().expect("the font is read");
        let edited = app.session.font().unwrap().glyph("A").unwrap().clone();
        assert_eq!(reference.glyph_for(&edited, "", &state.language).map(|g| g.name.as_str()), Some("A.loclTRK"));
        assert_eq!(reference.glyph_for(&edited, "", "").map(|g| g.name.as_str()), Some("A"));

        let junk = dir.join("junk.ttf");
        std::fs::write(&junk, b"not a font").unwrap();
        app.sent.push(shell(json!({ "path": junk.to_string_lossy() })));
        app.drain();
        assert!(app.status.starts_with("view.setReferenceFont"), "{}", app.status);
        assert_eq!(app.view.reference_font, state);
        app.sent.push(shell(json!({ "remove": path })));
        app.drain();
        assert!(app.view.reference_font.paths.is_empty() && app.view.reference_font.loaded.0.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
