//! The bar above the glyphs: the search field (`fontView.searchField`), which narrows the shown glyphs as it is
//! typed (`fontView.setSearch`) and takes the keyboard on `fontView.focusSearch`, and the Grid View and List View
//! buttons (`fontView.setViewMode`).

use craft::behavior::IconId;
use craft::ui::bars::{Bar, BarKind};
use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, Entry, Show as _, ShowContainer as _};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::runtime::focus::{self, FocusTarget};
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::json;

use super::FontViewWords;
use crate::app::font_view::ViewMode;
use crate::app::models::font_view::FontViewModel;
use crate::app::{Msg, ViewMsg};

pub fn show(ui: &mut Ui, model: &FontViewModel, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<FontViewWords>();
    let mut text = model.search.clone();
    let mut mode = model.mode;
    let modes = [
        Entry::new(ViewMode::Grid, words.grid_view.clone()).icon(IconId::new("layout_grid")),
        Entry::new(ViewMode::List, words.list_view.clone()).icon(IconId::new("list")),
    ];
    let _ = Bar::new(words.view.clone(), BarKind::Tool).show(ui, |ui, _| {
        let field = TextField::search(words.search.clone(), Bind::bound(&mut text)).placeholder(words.search.clone()).show(ui);
        if model.focus_search {
            let _ = focus::request(ui.ctx(), FocusTarget::Id(field.response.id));
            sent.push(Msg::View(ViewMsg::FontViewSearchFocused));
        }
        Segmented::new(words.view.clone(), Bind::bound(&mut mode), &modes).show(ui);
    });
    if text != model.search {
        sent.push(Msg::Shell(CommandRef::with_params("fontView.setSearch", json!({ "query": text }))));
    }
    if mode != model.mode {
        sent.push(Msg::Shell(CommandRef::with_params("fontView.setViewMode", json!({ "mode": mode.as_str() }))));
    }
}
