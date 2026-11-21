//! Selections: the shown glyph's stored selections. Typing a name and pressing Enter stores the edit view's selection
//! under it (`edit.storeSelection`); a click on a name selects it again (`edit.recallSelection`); Update stores the
//! current selection under that entry's name; Remove deletes the entry (`edit.removeStoredSelection`). An entry that
//! does not fully resolve on the layer shows how much is missing.

use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::json;

use crate::app::Msg;
use crate::app::models::glyph_sections::SelectionsModel;

craft::ui::words! {
    /// The words of the Selections panel.
    pub struct SelectionsWords {
        /// The field a new entry's name is typed into.
        name: "fontcraftSelections.name" = "Name";
        no_glyph: "fontcraftSelections.noGlyph" = "Open a glyph to store selections";
        empty: "fontcraftSelections.empty" = "No stored selections";
        recall_tip: "fontcraftSelections.recallTip" = "Select these nodes again";
        /// After a count of items that do not resolve.
        missing: "fontcraftSelections.missing" = "missing";
        update: "fontcraftSelections.update" = "Update";
        update_tip: "fontcraftSelections.updateTip" = "Store the current selection under this name";
        remove: "fontcraftSelections.remove" = "Remove";
        remove_tip: "fontcraftSelections.removeTip" = "Delete this stored selection";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<SelectionsModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<SelectionsWords>();
    let model: &SelectionsModel = props.model;
    let (Some(glyph), Some(layer)) = (&model.glyph, &model.layer) else {
        Text::new(words.no_glyph.clone()).shade(Shade::Muted).show(ui);
        return;
    };
    let sent = &mut *props.sent;
    let can_store = !model.selection.is_empty();
    let store = |name: &str| {
        let s = &model.selection;
        Msg::run(
            "edit.storeSelection",
            json!({ "glyph": glyph, "layer": layer, "name": name, "points": s.points, "shapes": s.shapes, "anchors": s.anchors }),
        )
    };
    let out = TextField::new(words.name.clone(), Bind::given(String::new())).placeholder(words.name.clone()).enabled(can_store).show(ui);
    if out.submitted
        && let Some(name) = out.events.committed().map(|n| n.trim().to_string())
        && !name.is_empty()
    {
        sent.push(store(&name));
    }
    if model.entries.is_empty() {
        Text::new(words.empty.clone()).shade(Shade::Muted).show(ui);
    }
    for entry in &model.entries {
        let _ = Row::new().show(ui, |ui, _| {
            if Button::new(entry.name.clone()).tooltip(words.recall_tip.clone()).show(ui).clicked() {
                let params = json!({ "name": entry.name, "glyph": glyph, "layer": layer });
                sent.push(Msg::Shell(CommandRef::with_params("edit.recallSelection", params)));
            }
            if entry.missing > 0 {
                Text::new(format!("{} {}", entry.missing, words.missing.as_str())).shade(Shade::Muted).show(ui);
            }
            if Button::new(words.update.clone()).tooltip(words.update_tip.clone()).enabled(can_store).show(ui).clicked() && can_store {
                sent.push(store(&entry.name));
            }
            if Button::new(words.remove.clone()).tooltip(words.remove_tip.clone()).show(ui).clicked() {
                sent.push(Msg::run("edit.removeStoredSelection", json!({ "glyph": glyph, "name": entry.name })));
            }
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

    /// A stored selection is listed with what it misses, and Remove posts its removal.
    #[test]
    fn stored_selections_are_listed_and_remove_posts() {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        app.sent.push(Msg::run("path.addRectangle", json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})));
        app.sent.push(Msg::run("edit.storeSelection", json!({"glyph": "A", "name": "corners", "points": [[0, 0], [0, 2]]})));
        app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
        app.sent.push(Msg::View(ViewMsg::TogglePanel(PanelId::Selections)));
        app.drain();
        assert!(app.status.is_empty(), "{}", app.status);
        let entries: Vec<(String, u64)> = app.models.selections.entries.iter().map(|e| (e.name.clone(), e.missing)).collect();
        assert_eq!(entries, [("corners".to_string(), 0)]);

        let model = Tracked::new((*app.models.selections).clone());
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let remove = fixture.by_label("Remove").expect("the Remove button");
        fixture.click(&remove).settle();
        assert_eq!(*posted.borrow(), vec![Msg::run("edit.removeStoredSelection", json!({"glyph": "A", "name": "corners"}))]);
    }
}
