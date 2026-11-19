//! The master icons of the toolbar: one per master while the font has two or more, the current master's pressed. A
//! click runs `view.selectMaster` for that master. Each icon is the master's icon from Font Info › Masters
//! (`master_icons`): a built-in n, or a glyph of the font drawn in that master.

use craft::ui::buttons::toggle_button::ToggleButton;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, Tracked};
use craft::ui::layout_components::Row;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::commands::CommandRef;
use egui::Ui;
use serde_json::json;

use super::master_icons::MasterGlyph;
use crate::app::Msg;
use crate::app::models::layers::MastersModel;

pub struct Props<'a> {
    pub model: &'a Tracked<MastersModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let masters = &props.model.masters;
    if masters.len() < 2 {
        return;
    }
    let current = props.model.current.as_deref();
    let sent = &mut *props.sent;
    Row::new().show(ui, |ui, _| {
        for (index, master) in masters.iter().enumerate() {
            let glyph = MasterGlyph(&master.icon);
            let on = current == Some(master.id.as_str());
            let out = ToggleButton::new(master.name.clone(), Bind::given(on))
                .icon(IconSpec::glyph(&glyph))
                .icon_only()
                .tooltip(master.name.clone())
                .id_salt(("fontcraftMasterSwitcher", index))
                .show(ui);
            if out.events.committed().is_some() {
                sent.push(Msg::Shell(CommandRef::with_params("view.selectMaster", json!({ "master": index }))));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::contract::Tracked;
    use craft::ui::runtime::commands::CommandRef;
    use craft::ui::testing::Fixture;
    use serde_json::json;

    use super::{Props, show};
    use crate::app::Msg;
    use crate::app::models::layers::{MasterEntry, MasterIcon, MastersModel};

    /// A click on the second master's icon selects that master; a font with one master shows no icons.
    #[test]
    fn a_click_on_a_master_icon_selects_that_master() {
        let entry = |id: &str, name: &str| MasterEntry { id: id.into(), name: name.into(), icon: MasterIcon::Builtin { weight: 1, width: 2 } };
        let model = Tracked::new(MastersModel { masters: vec![entry("m01", "Regular"), entry("m02", "Bold")], current: Some("m01".into()) });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let bold = fixture.by_label("Bold").expect("the Bold master's icon");
        fixture.click(&bold).settle();
        assert_eq!(*posted.borrow(), vec![Msg::Shell(CommandRef::with_params("view.selectMaster", json!({ "master": 1 })))]);

        let one = Tracked::new(MastersModel { masters: vec![entry("m01", "Regular")], current: Some("m01".into()) });
        let mut fixture = Fixture::new(move |ui| show(ui, Props { model: &one, sent: &mut Vec::new() }));
        fixture.settle();
        assert!(fixture.by_label("Regular").is_none(), "one master: no icons");
    }
}
