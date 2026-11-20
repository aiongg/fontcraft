//! The context menus' views: their items show with their state, and a click reports the item.

use std::cell::RefCell;
use std::rc::Rc;

use craft::ui::testing::Fixture;

use super::{Answer, Pick, canvas_menu, glyph_menu};
use crate::app::context_menu::glyph_menu::{GlyphMenu, Mark};
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

/// In the glyph menu's Tags submenu a tag some of the glyphs carry shows a dash and one all carry a check (no dash);
/// picking the mixed tag reports it.
#[test]
fn a_tag_some_glyphs_carry_shows_the_mixed_mark() {
    let menu = GlyphMenu {
        glyphs: vec!["a".into(), "b".into()],
        screen: [40.0, 40.0],
        tags: vec![("alternate".into(), Mark::Some), ("smallcap".into(), Mark::All)],
        ..GlyphMenu::default()
    };
    let answers = Rc::new(RefCell::new(Vec::<Answer>::new()));
    let seen = Rc::clone(&answers);
    let mut fixture = Fixture::new(move |ui| {
        let opener = ui.response();
        seen.borrow_mut().push(glyph_menu(ui, &opener, &menu));
    });
    fixture.settle();
    let tags = fixture.by_label("Tags").expect("the Tags submenu row");
    fixture.click(&tags).settle();
    let mixed = fixture.by_label("alternate").expect("the mixed tag").rect;
    let all = fixture.by_label("smallcap").expect("the tag every glyph carries").rect;
    let dashes_in = |shapes: &[egui::Shape], row: egui::Rect| {
        shapes
            .iter()
            .filter(|shape| {
                matches!(shape, egui::Shape::LineSegment { points: [a, b], .. }
                    if row.contains(*a) && row.contains(*b) && (a.y - b.y).abs() < f32::EPSILON && (a.y - row.center().y).abs() < 1.0)
            })
            .count()
    };
    let shapes = fixture.frame_shapes();
    assert_eq!(dashes_in(&shapes, mixed), 1);
    assert_eq!(dashes_in(&shapes, all), 0);
    let alternate = fixture.by_label("alternate").expect("the mixed tag");
    fixture.click(&alternate).settle();
    assert!(answers.borrow().iter().any(|a| a.picked == Some(Pick::Tag("alternate".into()))), "{:?}", answers.borrow().last());
}
