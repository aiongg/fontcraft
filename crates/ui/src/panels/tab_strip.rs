//! The document area's tab strip: Font View first, then one tab per Edit View tab (`shell::tabs`). A click shows a
//! tab, a tab's close button closes it, and with Alt held closes every other Edit View tab.

use craft::ui::contract::Tracked;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::tabs::{DocumentTab, DocumentTabs, DocumentTabsEvent};
use egui::Ui;
use serde_json::json;

use crate::app::Msg;
use crate::app::models::tabs::{TabEntry, TabsModel};
use crate::shell::dock::PanelWords;

/// The strip's key for Font View; Edit View tabs have ids from 1.
const FONT_VIEW: u64 = 0;

craft::ui::words! {
    /// The tab strip's words; Font View's tab takes its panel title.
    pub struct TabStripWords {
        /// The strip's accessible name.
        strip: "fontcraftTabs.strip" = "Tabs";
        /// The title of a tab whose text shows no glyph.
        empty: "fontcraftTabs.empty" = "Empty";
        /// After the glyph names of a tab that shows more glyphs than its title lists.
        more: "fontcraftTabs.more" = "…";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<TabsModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<TabStripWords>();
    let panels = Env::of(ui).words::<PanelWords>();
    let model: &TabsModel = props.model;
    let mut tabs = Vec::with_capacity(model.tabs.len() + 1);
    tabs.push(DocumentTab::new(FONT_VIEW, panels.font_view.clone()).closable(false));
    tabs.extend(model.tabs.iter().map(|tab| DocumentTab::new(tab.id, title(tab, &words))));
    let shown = model.active.unwrap_or(FONT_VIEW);
    let out = DocumentTabs::new(words.strip.clone(), &tabs).selected(shown).show(ui);
    // Alt held at the close button's click closes the other tabs.
    let alt = craft_ext::ui::contract::held_mods(ui.ctx()).alt;
    for event in out.events {
        let msg = match event {
            DocumentTabsEvent::Selected(id) | DocumentTabsEvent::Picked(id) if id != shown => {
                shell("view.showTab", json!({ "index": position(model, id) }))
            }
            DocumentTabsEvent::CloseRequested(id) if id != FONT_VIEW => {
                let at = position(model, id);
                if alt { shell("view.closeOtherTabs", json!({ "keep": at })) } else { shell("view.closeTab", json!({ "index": at })) }
            }
            _ => continue,
        };
        props.sent.push(msg);
    }
}

/// A tab's title: the glyphs it shows.
fn title(tab: &TabEntry, words: &TabStripWords) -> Label {
    if tab.glyphs.is_empty() {
        return words.empty.clone();
    }
    let mut text = tab.glyphs.join(" ");
    if tab.more {
        text.push(' ');
        text.push_str(words.more.as_str());
    }
    Label::from(text)
}

/// The strip position of the tab `id`: 1 for Font View.
fn position(model: &TabsModel, id: u64) -> usize {
    model.tabs.iter().position(|t| t.id == id).map_or(1, |i| i + 2)
}

fn shell(id: &str, params: serde_json::Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::contract::Tracked;
    use craft::ui::testing::Fixture;
    use serde_json::json;

    use super::{Props, show};
    use crate::app::Msg;
    use crate::app::models::tabs::{TabEntry, TabsModel};

    fn shell(id: &str, params: serde_json::Value) -> Msg {
        Msg::Shell(craft::ui::runtime::commands::CommandRef::with_params(id, params))
    }

    /// Font View is the first tab; a click on a tab title shows it by its position, and the showing tab's close
    /// button closes it.
    #[test]
    fn the_strip_shows_and_closes_tabs_by_position() {
        let model = Tracked::new(TabsModel {
            tabs: vec![
                TabEntry { id: 4, glyphs: vec!["A".into()], more: false },
                TabEntry { id: 7, glyphs: vec!["B".into(), "C".into(), "D".into(), "E".into()], more: true },
            ],
            active: Some(4),
        });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let font_view = fixture.by_label("Font View").expect("Font View's tab");
        fixture.click(&font_view).settle();
        let long = fixture.by_label("B C D E …").expect("the second Edit View tab");
        fixture.click(&long).settle();
        let close = fixture.by_label("Close A").expect("the showing tab's close button");
        fixture.click(&close).settle();
        assert_eq!(
            *posted.borrow(),
            vec![shell("view.showTab", json!({"index": 1})), shell("view.showTab", json!({"index": 3})), shell("view.closeTab", json!({"index": 2}))]
        );
    }

    /// Tabs that do not fit the window are in the strip's overflow menu, and a pick there shows the tab.
    #[test]
    fn a_tab_that_does_not_fit_is_shown_from_the_overflow_menu() {
        let tabs = (1..=8).map(|n| TabEntry { id: n, glyphs: vec![format!("tabnumber{n}")], more: false }).collect();
        let model = Tracked::new(TabsModel { tabs, active: None });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.room(egui::vec2(260.0, 600.0)).settle();
        assert!(fixture.by_label("tabnumber8").is_none(), "the last tab does not fit");
        fixture.key(egui::Key::Tab, egui::Modifiers::NONE).key(egui::Key::ArrowDown, egui::Modifiers::ALT).settle();
        let row = fixture.by_label("tabnumber8").expect("the last tab's row in the overflow menu");
        fixture.click(&row).settle();
        assert_eq!(*posted.borrow(), vec![shell("view.showTab", json!({"index": 9}))]);
    }
}
