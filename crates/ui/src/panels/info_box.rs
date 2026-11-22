//! The Info box: the edited glyph's metrics and kerning groups, and what the canvas selection measures.
//!
//! | Part | Fields | Editing runs |
//! |---|---|---|
//! | glyph, left to right | LSB, RSB, width: numbers or metrics keys | `spacing.setLeftSidebearing`, `…RightSidebearing`, `spacing.setWidth` |
//! | glyph, top to bottom | T, B, O (vertical origin), H (vertical width) | `spacing.setTopSidebearing`, `…BottomSidebearing`, `spacing.setVerticalOrigin`, `spacing.setVerticalWidth` |
//! | an out-of-sync key | the update button beside its field | `spacing.updateMetricsKey` |
//! | kerning groups | one field per side | `kerning.setLeftGroup`, `…RightGroup`, `…TopGroup`, `…BottomGroup` |
//! | kerning, while the glyph shows in the edit view's text | kern fields and locks towards its neighbours (`info_box_kerning`) | `kerning.setPair`, `…removePair`, `…addException`, `…removeException` |
//! | one node | X, Y | `selection.setPosition` |
//! | one handle | X, Y, dX, dY and its length, measured from its node | `selection.setPosition` on the handle |
//! | anything else | X, Y of the box at the reference point, width, height, lock | `selection.setPosition`; `transform.scale` about the reference point |
//!
//! A selected component, corner component or cap component adds its part below (`info_box_component`). A selected
//! placed image shows its own part in place of the selection part (`info_box_image`). One selected node in a font with
//! an axis and two masters adds its interpolation curve and timing graph (`info_box_hoi`); one selected guide adds its
//! properties (`info_box_guide`).
//!
//! The nine-point grid chooses the reference point (`infoBox.setReferencePoint`); the lock makes a width or height
//! edit scale both (`infoBox.setSizeLock`). A master that takes its metrics from another shows that master's values,
//! read only.

use craft::ui::buttons::toggle_button::ToggleButton;
use craft::ui::contract::{Bind, Props as _, Scroll, Show as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::Section;
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::{Map, Value, json};

use super::widgets::{self, KeyedNumber};
use crate::app::Msg;
use crate::app::models::palette::{GlyphBox, InfoBoxModel, SelectionBox, SelectionKind};

craft::ui::words! {
    /// The words of the Info box.
    pub struct InfoBoxWords {
        /// The glyph part's title.
        glyph: "fontcraftInfoBox.glyph" = "Glyph";
        /// The selection part's title.
        selection: "fontcraftInfoBox.selection" = "Selection";
        /// Nothing to show.
        nothing: "fontcraftInfoBox.nothing" = "No glyph is being edited";
        left: "fontcraftInfoBox.left" = "LSB";
        right: "fontcraftInfoBox.right" = "RSB";
        width: "fontcraftInfoBox.width" = "Width";
        top: "fontcraftInfoBox.top" = "T";
        bottom: "fontcraftInfoBox.bottom" = "B";
        /// The vertical origin field.
        origin: "fontcraftInfoBox.verticalOrigin" = "O";
        /// The vertical width (height) field.
        height_metric: "fontcraftInfoBox.verticalWidth" = "H";
        left_group: "fontcraftInfoBox.leftGroup" = "Left Group";
        right_group: "fontcraftInfoBox.rightGroup" = "Right Group";
        top_group: "fontcraftInfoBox.topGroup" = "Top Group";
        bottom_group: "fontcraftInfoBox.bottomGroup" = "Bottom Group";
        /// Before the name of the master whose metrics show.
        linked: "fontcraftInfoBox.linked" = "Metrics of master";
        reference: "fontcraftInfoBox.reference" = "Reference Point";
        x: "fontcraftInfoBox.x" = "X";
        y: "fontcraftInfoBox.y" = "Y";
        dx: "fontcraftInfoBox.dx" = "dX";
        dy: "fontcraftInfoBox.dy" = "dY";
        length: "fontcraftInfoBox.length" = "L";
        box_width: "fontcraftInfoBox.boxWidth" = "W";
        box_height: "fontcraftInfoBox.boxHeight" = "H";
        lock: "fontcraftInfoBox.lock" = "Lock Info Box Proportions";
        /// Before the distance between two selected nodes.
        distance: "fontcraftInfoBox.distance" = "Distance";
        /// Before the selected point count.
        selected: "fontcraftInfoBox.selected" = "Selected points";
        /// Before the layer's point count.
        total: "fontcraftInfoBox.total" = "Points in the layer";
        /// After the count of glyphs selected in text mode.
        several: "fontcraftInfoBox.several" = "glyphs selected";
        /// The placeholder of a field whose glyphs differ.
        multiple: "fontcraftInfoBox.multiple" = "Multiple Values";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<InfoBoxModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    // The palette gives the panel a fixed height, which a guide or interpolation curve part outgrows.
    Stack::new().scroll(Scroll::vertical()).fill().show(ui, |ui, _| view(&mut props, ui));
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<InfoBoxWords>();
    let model: &InfoBoxModel = props.model;
    let sent = &mut *props.sent;
    if let Some(tt) = &model.tt {
        super::info_box_tt::view(ui, tt, sent);
        return;
    }
    let Some(glyph) = &model.glyph else {
        Text::new(words.nothing.clone()).shade(Shade::Muted).show(ui);
        return;
    };
    Section::new(words.glyph.clone()).show(ui, |ui, _| glyph_part(ui, &words, glyph, sent));
    if let Some(k) = &model.kerning {
        super::info_box_kerning::view(ui, k, sent);
    }
    if let Some(image) = &model.image {
        super::info_box_image::view(ui, image, sent);
    } else if let Some(hint) = &model.ps_hint {
        super::info_box_ps_hint::view(ui, hint, sent);
    } else if let Some(selection) = &model.selection {
        Section::new(words.selection.clone()).show(ui, |ui, _| selection_part(ui, &words, selection, sent));
    }
    if let Some(b) = &model.component {
        super::info_box_component::component_part(ui, b, sent);
    }
    if let Some(b) = &model.corner {
        super::info_box_component::corner_part(ui, b, sent);
    }
    if let Some(b) = &model.cap {
        super::info_box_component::cap_part(ui, b, sent);
    }
    if let Some(guide) = &model.guide {
        super::info_box_guide::view(ui, guide, sent);
    }
    if let Some(hoi) = &model.hoi {
        super::info_box_hoi::view(ui, hoi, sent);
    }
}

fn glyph_part(ui: &mut Ui, words: &InfoBoxWords, g: &GlyphBox, sent: &mut Vec<Msg>) {
    let title = match g.unicode {
        _ if !g.glyphs.is_empty() => format!("{} {}", g.glyphs.len(), words.several.as_str()),
        Some(u) => format!("{}  {u:04X}", g.glyph),
        None => g.glyph.clone(),
    };
    let targets = g.targets();
    Text::new(title).show(ui);
    let fields: Vec<(&Label, &KeyedNumber, &str, &str)> = if g.vertical {
        vec![
            (&words.top, &g.top, "top", "spacing.setTopSidebearing"),
            (&words.bottom, &g.bottom, "bottom", "spacing.setBottomSidebearing"),
            (&words.origin, &g.vert_origin, "vertOrigin", "spacing.setVerticalOrigin"),
            (&words.height_metric, &g.vert_width, "vertWidth", "spacing.setVerticalWidth"),
        ]
    } else {
        vec![
            (&words.left, &g.left, "left", "spacing.setLeftSidebearing"),
            (&words.right, &g.right, "right", "spacing.setRightSidebearing"),
            (&words.width, &g.width, "width", "spacing.setWidth"),
        ]
    };
    let read_only = g.linked.is_some();
    for (label, metric, field, command) in fields {
        let out = if g.is_mixed(field) {
            mixed_field(ui, label.clone(), words.multiple.clone())
        } else {
            widgets::key_or_number(ui, label.clone(), metric, read_only)
        };
        if let Some(value) = out.commit {
            sent.push(Msg::run(command, json!({"glyphs": targets, "layer": g.layer, "value": value})));
        }
        if out.update {
            sent.push(Msg::run("spacing.updateMetricsKey", json!({"glyph": g.glyph, "layer": g.layer, "field": field})));
        }
    }
    let [left, right, top, bottom] = &g.groups;
    let [left_mixed, right_mixed, top_mixed, bottom_mixed] = g.mixed_groups;
    let groups = if g.vertical {
        [(&words.top_group, top, top_mixed, "kerning.setTopGroup"), (&words.bottom_group, bottom, bottom_mixed, "kerning.setBottomGroup")]
    } else {
        [(&words.left_group, left, left_mixed, "kerning.setLeftGroup"), (&words.right_group, right, right_mixed, "kerning.setRightGroup")]
    };
    let _ = Row::new().show(ui, |ui, _| {
        for (label, group, mixed, command) in groups {
            let committed = widgets::labelled(ui, label, |ui| {
                let mut field = TextField::new(label.clone(), Bind::given(group.clone())).select_on_focus();
                if mixed {
                    field = field.placeholder(words.multiple.clone());
                }
                field.show(ui).events.committed().cloned()
            });
            // A mixed field starts empty: committing it empty leaves the groups as they are.
            if let Some(Some(text)) = committed
                && text != *group
                && !(mixed && text.trim().is_empty())
            {
                sent.push(Msg::run(command, json!({"glyphs": targets, "group": text.trim()})));
            }
        }
    });
    if let Some(master) = &g.linked {
        Text::new(format!("{} {master}", words.linked.as_str())).shade(Shade::Muted).show(ui);
    }
}

/// A metric field whose glyphs differ: empty, with the *Multiple Values* placeholder; any value typed goes to every
/// glyph, and an emptied field sets nothing.
fn mixed_field(ui: &mut Ui, label: Label, placeholder: Label) -> widgets::KeyedOutput {
    let commit = widgets::labelled(ui, &label.clone(), |ui| {
        let field = TextField::new(label.clone(), Bind::given(String::new())).select_on_focus().limit(widgets::MAX_KEY_CHARS);
        field.placeholder(placeholder.clone()).show(ui).events.committed().and_then(|text| KeyedNumber::default().commit_value(text))
    })
    .flatten();
    widgets::KeyedOutput { commit, update: false }
}

fn selection_part(ui: &mut Ui, words: &InfoBoxWords, s: &SelectionBox, sent: &mut Vec<Msg>) {
    let place = |sent: &mut Vec<Msg>, x: Option<f64>, y: Option<f64>| {
        let mut extra = Map::new();
        if let Some(x) = x {
            extra.insert("x".into(), json!(x));
        }
        if let Some(y) = y {
            extra.insert("y".into(), json!(y));
        }
        extra.insert("referencePoint".into(), json!(s.reference));
        sent.push(Msg::run("selection.setPosition", s.place.with(Value::Object(extra))));
    };
    if let Some(point) = widgets::box_point(ui, words.reference.clone(), Some(&s.reference)) {
        sent.push(shell("infoBox.setReferencePoint", json!({ "point": point })));
    }
    let _ = Row::new().show(ui, |ui, _| {
        if let Some(x) = widgets::number(ui, words.x.clone(), s.x) {
            place(sent, Some(x), None);
        }
        if let Some(y) = widgets::number(ui, words.y.clone(), s.y) {
            place(sent, None, Some(y));
        }
    });
    match s.kind {
        SelectionKind::Node => {}
        SelectionKind::Handle => handle_fields(ui, words, s, sent, &place),
        SelectionKind::Box => box_fields(ui, words, s, sent),
    }
    if s.selected > 1 {
        Text::new(format!("{} {}  {} {}", words.selected.as_str(), s.selected, words.total.as_str(), s.total)).shade(Shade::Muted).show(ui);
    }
}

/// Sends the messages that move a handle to (x, y), each `None` left as it is.
type Place<'a> = dyn Fn(&mut Vec<Msg>, Option<f64>, Option<f64>) + 'a;

/// dX, dY and the length of one handle: each moves the handle, measured from its node.
fn handle_fields(ui: &mut Ui, words: &InfoBoxWords, s: &SelectionBox, sent: &mut Vec<Msg>, place: &Place<'_>) {
    let node = s.x.zip(s.dx).map(|(h, d)| h - d).zip(s.y.zip(s.dy).map(|(h, d)| h - d));
    let _ = Row::new().show(ui, |ui, _| {
        if let Some(v) = widgets::number(ui, words.dx.clone(), s.dx)
            && let Some((nx, _)) = node
        {
            place(sent, Some(nx + v), None);
        }
        if let Some(v) = widgets::number(ui, words.dy.clone(), s.dy)
            && let Some((_, ny)) = node
        {
            place(sent, None, Some(ny + v));
        }
        if let Some(v) = widgets::number(ui, words.length.clone(), s.length)
            && let (Some((nx, ny)), Some(dx), Some(dy), Some(l)) = (node, s.dx, s.dy, s.length)
            && l > 0.0
        {
            let k = v / l;
            place(sent, Some(nx + dx * k), Some(ny + dy * k));
        }
    });
}

/// The box's width and height, with the lock between them: each scales the selection about the reference point.
fn box_fields(ui: &mut Ui, words: &InfoBoxWords, s: &SelectionBox, sent: &mut Vec<Msg>) {
    let scale = |sent: &mut Vec<Msg>, [fx, fy]: [f64; 2]| {
        let extra = json!({"x": fx * 100.0, "y": fy * 100.0, "origin": {"mode": "box", "boxPoint": s.reference}});
        sent.push(Msg::run("transform.scale", s.scale.with(extra)));
    };
    let _ = Row::new().show(ui, |ui, _| {
        if let Some(v) = widgets::number(ui, words.box_width.clone(), s.width)
            && let Some(old) = s.width.filter(|w| *w > 0.0)
        {
            let k = v / old;
            scale(sent, [k, if s.locked { k } else { 1.0 }]);
        }
        let lock = ToggleButton::new(words.lock.clone(), Bind::given(s.locked)).tooltip(words.lock.clone()).show(ui);
        if lock.events.committed().is_some() {
            sent.push(shell("infoBox.setSizeLock", json!({ "locked": !s.locked })));
        }
        if let Some(v) = widgets::number(ui, words.box_height.clone(), s.height)
            && let Some(old) = s.height.filter(|h| *h > 0.0)
        {
            let k = v / old;
            scale(sent, [if s.locked { k } else { 1.0 }, k]);
        }
    });
    if let Some(d) = s.distance {
        Text::new(format!("{} {}", words.distance.as_str(), widgets::format_number(d))).shade(Shade::Muted).show(ui);
    }
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::contract::Tracked;
    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};
    use serde_json::json;

    use super::{Props, show};
    use crate::app::Msg;
    use crate::app::models::palette::{GlyphBox, InfoBoxModel, SelectionBox, SelectionKind, Targets};
    use crate::panels::widgets::{KeyState, KeyedNumber};

    fn posted_after(model: InfoBoxModel, label: &str, text: &str) -> Vec<Msg> {
        let model = Tracked::new(model);
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let field = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} field"));
        fixture.click(&field).key(Key::A, Modifiers::COMMAND).type_text(text).key(Key::Enter, Modifiers::NONE).settle();
        posted.borrow().clone()
    }

    fn glyph(vertical: bool) -> GlyphBox {
        let number = |v: f64| KeyedNumber { value: Some(v), ..Default::default() };
        GlyphBox {
            glyph: "A".into(),
            layer: "m01".into(),
            vertical,
            left: number(20.0),
            right: KeyedNumber { value: Some(30.0), key: Some("=H".into()), state: KeyState::InSync, message: None },
            width: number(600.0),
            vert_origin: number(0.0),
            vert_width: number(1000.0),
            ..Default::default()
        }
    }

    /// A key typed into a sidebearing field goes to its setter as text; a number as a number.
    #[test]
    fn metrics_fields_take_keys_and_numbers() {
        let model = InfoBoxModel { glyph: Some(glyph(false)), ..Default::default() };
        assert_eq!(
            posted_after(model.clone(), "LSB", "=O"),
            vec![Msg::run("spacing.setLeftSidebearing", json!({"glyphs": ["A"], "layer": "m01", "value": "=O"}))]
        );
        assert_eq!(posted_after(model, "Width", "640"), vec![Msg::run("spacing.setWidth", json!({"glyphs": ["A"], "layer": "m01", "value": 640.0}))]);
    }

    /// With several glyphs selected, a typed metric and group go to every glyph, and a mixed field shows empty (M5.22).
    #[test]
    fn several_glyphs_edit_together() {
        let shared =
            GlyphBox { glyphs: vec!["A".into(), "B".into()], mixed: vec!["width".into()], mixed_groups: [true, false, false, false], ..glyph(false) };
        let model = InfoBoxModel { glyph: Some(shared), ..Default::default() };
        assert_eq!(
            posted_after(model.clone(), "Width", "640"),
            vec![Msg::run("spacing.setWidth", json!({"glyphs": ["A", "B"], "layer": "m01", "value": 640.0}))]
        );
        assert_eq!(
            posted_after(model.clone(), "LSB", "=O"),
            vec![Msg::run("spacing.setLeftSidebearing", json!({"glyphs": ["A", "B"], "layer": "m01", "value": "=O"}))]
        );
        assert_eq!(
            posted_after(model, "Left Group", "round"),
            vec![Msg::run("kerning.setLeftGroup", json!({"glyphs": ["A", "B"], "group": "round"}))]
        );
    }

    /// The vertical layout edits the vertical origin and width (M5.20).
    #[test]
    fn the_vertical_layout_sets_origin_and_height() {
        let model = InfoBoxModel { glyph: Some(glyph(true)), ..Default::default() };
        assert_eq!(
            posted_after(model.clone(), "O", "120"),
            vec![Msg::run("spacing.setVerticalOrigin", json!({"glyphs": ["A"], "layer": "m01", "value": 120.0}))]
        );
        assert_eq!(
            posted_after(model, "H", "=1000"),
            vec![Msg::run("spacing.setVerticalWidth", json!({"glyphs": ["A"], "layer": "m01", "value": "=1000"}))]
        );
    }

    /// X moves the selection so its reference point lands on the typed value.
    #[test]
    fn x_places_the_selection_at_the_reference_point() {
        let selection = SelectionBox {
            kind: SelectionKind::Box,
            reference: "center".into(),
            x: Some(100.0),
            y: Some(50.0),
            width: Some(200.0),
            height: Some(100.0),
            place: Targets { json: json!({"glyph": "A", "layer": "m01", "points": [[0, 0]]}).to_string() },
            ..Default::default()
        };
        let model = InfoBoxModel { glyph: Some(glyph(false)), selection: Some(selection), ..Default::default() };
        assert_eq!(
            posted_after(model, "X", "140"),
            vec![Msg::run(
                "selection.setPosition",
                json!({"glyph": "A", "layer": "m01", "points": [[0, 0]], "x": 140.0, "referencePoint": "center"})
            )]
        );
    }
}
