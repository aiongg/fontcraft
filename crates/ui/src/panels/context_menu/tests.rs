//! The canvas menu's view: its items show with their state, and a click reports the item.

use std::cell::RefCell;
use std::rc::Rc;

use craft::ui::testing::Fixture;

use super::{Answer, Pick, canvas_menu};
use crate::app::context_menu::{CANVAS, CanvasMenu, ContextItem, NODE, Section};

fn item(command: &'static str, label: &'static str, enabled: bool) -> ContextItem {
    ContextItem {
        command,
        label,
        enabled,
        disabled_reason: (!enabled).then(|| "select two open ends of paths".into()),
        checked: None,
        alternate: false,
    }
}

#[test]
fn a_click_on_an_item_reports_it_and_a_disabled_item_reports_nothing() {
    let menu = CanvasMenu {
        glyph: "A".into(),
        screen: [40.0, 40.0],
        at: [300.0, 300.0],
        hit: None,
        sections: vec![
            Section { context: NODE, items: vec![item("path.openCorner", "Open Corner", true), item("path.connectNodes", "Connect Nodes", false)] },
            Section { context: CANVAS, items: vec![ContextItem { checked: Some(true), ..item("glyph.toggleLocked", "Locked", true) }] },
        ],
        alt: false,
    };
    let answers = Rc::new(RefCell::new(Vec::<Answer>::new()));
    let seen = Rc::clone(&answers);
    let mut fixture = Fixture::new(move |ui| {
        let opener = ui.response();
        seen.borrow_mut().push(canvas_menu(ui, &opener, &menu, false));
    });
    fixture.settle();
    assert!(fixture.by_label("Locked").is_some(), "the glyph section shows");
    assert!(fixture.by_label("Glyph Color").is_some(), "the swatches' submenu opens the glyph section");
    let disabled = fixture.by_label("Connect Nodes").expect("the disabled item shows");
    fixture.click(&disabled).settle();
    assert!(answers.borrow().iter().all(|a| a.picked.is_none()), "a disabled item runs nothing");
    let open_corner = fixture.by_label("Open Corner").expect("the Open Corner item");
    fixture.click(&open_corner).settle();
    assert!(answers.borrow().iter().any(|a| a.picked == Some(Pick::Item("path.openCorner")) && !a.open), "{:?}", answers.borrow().last());
}
