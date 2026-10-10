//! Edit View's toolbar and the drawn icons, in headless frames: every tool has an icon and a
//! button, a click selects the tool, holding a shared button opens its menu, and the Palette's
//! Transformations buttons draw icons instead of characters the interface fonts lack.

use egui::{Color32, Event, FontId, Modifiers, PointerButton, Pos2, Rect, Shape, pos2, vec2};
use fontcraft_tools::TOOLS;
use serde_json::json;

use crate::FontcraftApp;
use crate::icons;
use crate::panels::widgets;
use crate::shell::toolbar;

/// A headless window with its own clock, so a held button can be timed.
struct Window {
    ctx: egui::Context,
    time: f64,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default(), time: 0.0 }
    }

    /// One frame `elapsed` seconds after the last; the shapes it drew, nested lists flattened.
    fn frame_after(&mut self, app: &mut FontcraftApp, elapsed: f64, events: Vec<Event>) -> Vec<Shape> {
        self.time += elapsed;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 1000.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
        let mut out: Vec<Shape> = output.shapes.into_iter().map(|c| c.shape).collect();
        let mut flat = Vec::new();
        while let Some(shape) = out.pop() {
            match shape {
                Shape::Vec(list) => out.extend(list),
                other => flat.push(other),
            }
        }
        flat
    }

    fn frame(&mut self, app: &mut FontcraftApp, events: Vec<Event>) -> Vec<Shape> {
        self.frame_after(app, 1.0 / 60.0, events)
    }

    fn settle(&mut self, app: &mut FontcraftApp) {
        for _ in 0..3 {
            self.frame(app, Vec::new());
        }
    }

    /// Where the widget `id` was drawn in the last frame.
    fn rect(&self, id: egui::Id) -> Option<Rect> {
        self.ctx.read_response(id).map(|r| r.rect)
    }

    fn button(&mut self, app: &mut FontcraftApp, at: Pos2, pressed: bool) {
        self.frame(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
    }

    /// Clicks the middle of the widget `id`.
    fn click(&mut self, app: &mut FontcraftApp, id: egui::Id) {
        let at = self.rect(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).center();
        self.frame(app, vec![Event::PointerMoved(at)]);
        self.button(app, at, true);
        self.button(app, at, false);
    }
}

/// Glyph A with a square, open in Edit View.
fn edit_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app
}

fn active(app: &FontcraftApp) -> &'static str {
    app.tools.toolbox.base_id()
}

#[test]
fn every_tool_has_an_icon_and_a_toolbar_button() {
    let mut app = edit_app();
    let mut w = Window::new();
    w.settle(&mut app);
    let buttons = toolbar::buttons();
    for tool in TOOLS {
        assert!(icons::for_tool(tool.id).is_some(), "tool `{}` has no icon", tool.id);
        let b = buttons.iter().find(|b| b.tools.iter().any(|t| t.id == tool.id)).unwrap_or_else(|| panic!("tool `{}` has no button", tool.id));
        assert!(w.rect(toolbar::button_id(b.key)).is_some(), "the button of `{}` is not drawn", tool.id);
    }
    // Tools of one group share one button.
    for b in &buttons {
        let groups: Vec<&str> = b.tools.iter().map(|t| t.group).collect();
        assert!(groups.iter().all(|g| *g == groups[0]), "button {} mixes groups {groups:?}", b.key);
        assert_eq!(buttons.iter().filter(|o| o.key == b.key).count(), 1);
    }
    assert_eq!(buttons.iter().find(|b| b.key == "primitives").map(|b| b.tools.len()), Some(2));
}

#[test]
fn icons_draw_inside_their_rect() {
    let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(40.0, 40.0));
    for &icon in icons::ALL {
        let shapes = icons::shapes(icon, rect, Color32::WHITE);
        assert!(!shapes.is_empty(), "{icon:?} draws nothing");
        for shape in &shapes {
            let bounds = shape.visual_bounding_rect();
            assert!(rect.expand(4.0).contains_rect(bounds), "{icon:?} draws outside its rect: {bounds:?}");
        }
    }
    assert!(icons::shapes(icons::Icon::Select, Rect::NOTHING, Color32::WHITE).is_empty());
}

#[test]
fn clicking_a_button_selects_its_tool() {
    let mut app = edit_app();
    let mut w = Window::new();
    w.settle(&mut app);
    assert_eq!(active(&app), "select");
    w.click(&mut app, toolbar::button_id("zoom"));
    assert_eq!(active(&app), "zoom");
    // The active tool's button is highlighted.
    let zoom = w.rect(toolbar::button_id("zoom")).unwrap();
    let shapes = w.frame(&mut app, Vec::new());
    let highlighted = shapes.iter().any(|s| matches!(s, Shape::Rect(r) if r.rect == zoom && r.fill == app.tokens.row_selected));
    assert!(highlighted, "the Zoom button is not highlighted");
    w.click(&mut app, toolbar::button_id("draw"));
    assert_eq!(active(&app), "draw");

    // A shared button chooses the tool of its group used last, also when a key chose it.
    app.run("tool.select", &json!({"tool": "lasso"})).unwrap();
    w.frame(&mut app, Vec::new());
    w.click(&mut app, toolbar::button_id("hand"));
    assert_eq!(active(&app), "hand");
    w.click(&mut app, toolbar::button_id("select"));
    assert_eq!(active(&app), "lasso");
}

#[test]
fn holding_a_shared_button_opens_its_menu() {
    let mut app = edit_app();
    let mut w = Window::new();
    w.settle(&mut app);
    let at = w.rect(toolbar::button_id("primitives")).unwrap().center();
    w.frame(&mut app, vec![Event::PointerMoved(at)]);

    // A quick click selects the tool the button shows and opens nothing.
    w.button(&mut app, at, true);
    w.button(&mut app, at, false);
    assert_eq!(active(&app), "rectangle");
    w.settle(&mut app);
    assert!(w.rect(toolbar::menu_item_id("ellipse")).is_none(), "a click opened the menu");

    // Held: the menu lists the group's tools, and the release selects nothing.
    app.run("tool.select", &json!({"tool": "select"})).unwrap();
    w.button(&mut app, at, true);
    w.frame_after(&mut app, toolbar::HOLD_SECONDS + 0.1, Vec::new());
    w.frame(&mut app, Vec::new());
    assert!(w.rect(toolbar::menu_item_id("rectangle")).is_some(), "holding opened no menu");
    assert!(w.rect(toolbar::menu_item_id("ellipse")).is_some());
    w.button(&mut app, at, false);
    assert_eq!(active(&app), "select", "releasing the hold selected a tool");

    w.click(&mut app, toolbar::menu_item_id("ellipse"));
    assert_eq!(active(&app), "ellipse");
    w.settle(&mut app);
    assert!(w.rect(toolbar::menu_item_id("ellipse")).is_none(), "choosing a tool leaves the menu open");

    // A button without a group has no menu.
    let hand = w.rect(toolbar::button_id("hand")).unwrap().center();
    w.frame(&mut app, vec![Event::PointerMoved(hand)]);
    w.button(&mut app, hand, true);
    w.frame_after(&mut app, toolbar::HOLD_SECONDS + 0.1, Vec::new());
    w.button(&mut app, hand, false);
    assert!(w.rect(toolbar::menu_item_id("hand")).is_none());
}

/// The Transformations buttons, by their control ids.
const TRANSFORM_BUTTONS: &[&str] = &[
    "transform.reference",
    "transform.mirrorHorizontal",
    "transform.mirrorVertical",
    "transform.reverseScale",
    "transform.scaleLock",
    "transform.scale",
    "transform.rotateCounterclockwise",
    "transform.rotateClockwise",
    "transform.slantLeft",
    "transform.slantRight",
    "transform.slantDown",
    "transform.slantUp",
    "transform.alignLeft",
    "transform.alignCenterHorizontal",
    "transform.alignRight",
    "transform.alignTop",
    "transform.alignCenterVertical",
    "transform.alignBottom",
    "transform.booleanUnion",
    "transform.booleanSubtract",
    "transform.booleanIntersect",
];

#[test]
fn transformations_buttons_draw_icons_the_fonts_can_show() {
    let mut app = edit_app();
    for section in ["dimensions", "fitCurve", "layers"] {
        app.run("palette.toggleSection", &json!({"section": section, "expanded": false})).unwrap();
    }
    let mut w = Window::new();
    w.settle(&mut app);
    let shapes = w.frame(&mut app, Vec::new());
    let texts: Vec<(Rect, Vec<(FontId, String)>)> = shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Text(t) => {
                let job = &t.galley.job;
                let runs = job
                    .sections
                    .iter()
                    .map(|sec| {
                        (sec.format.font_id.clone(), job.text.get(sec.byte_range.start.0..sec.byte_range.end.0).unwrap_or_default().to_string())
                    })
                    .collect();
                Some((s.visual_bounding_rect(), runs))
            }
            _ => None,
        })
        .collect();
    let mut area = Rect::NOTHING;
    for id in TRANSFORM_BUTTONS {
        let rect = w.rect(widgets::id(*id)).unwrap_or_else(|| panic!("{id} was not drawn"));
        area = area.union(rect);
        assert!(texts.iter().all(|(r, _)| !r.intersects(rect)), "{id} shows text instead of an icon");
        let marks = shapes.iter().filter(|s| !matches!(s, Shape::Text(_)) && rect.expand(2.0).contains_rect(s.visual_bounding_rect())).count();
        assert!(marks >= 2, "{id} draws no icon");
    }
    // Any text in the section is text the interface fonts have glyphs for.
    for (rect, runs) in &texts {
        if !rect.intersects(area) {
            continue;
        }
        for (font, text) in runs {
            for c in text.chars().filter(|c| !c.is_whitespace()) {
                assert!(w.ctx.fonts_mut(|f| f.has_glyph(font, c)), "{c:?} in {text:?} has no glyph in {font:?}");
            }
        }
    }
}
