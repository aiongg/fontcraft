//! Palette › Transformations: the origin (`transform.setOrigin`), mirror, scale with its lock
//! (`transform.setScaleLock`), rotate, slant, align, and the booleans union, subtract and intersect.
//!
//! The buttons act on the canvas selection, or on the whole layer when nothing is selected; with Font View in front, or
//! text selected in text mode, on the layers of the glyphs selected there, as one undo step (align does nothing there). With corner components
//! selected the mirror buttons flip those (`corner.mirrorViaPalette`); the booleans take the
//! selected paths, a path counting as selected when any of its nodes is. The two buttons beside a field apply its
//! value in opposite directions. Field values go through `transform.setSettings`; each button passes them, and the
//! origin, to its command.

use craft::behavior::IconId;
use craft::ui::buttons::toggle_button::ToggleButton;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::pickers::Select;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft_ext::ui::primitives::ShapeIcon;
use egui::Ui;
use serde_json::{Value, json};

use super::widgets;
use crate::app::Msg;
use crate::app::models::palette::TransformModel;
use crate::shell::palette::METRICS;

craft::ui::words! {
    /// The words of the Transformations section.
    pub struct TransformWords {
        origin: "fontcraftTransform.origin" = "Origin";
        reference: "fontcraftTransform.reference" = "Reference Point";
        metric: "fontcraftTransform.metric" = "Metric";
        baseline: "fontcraftTransform.baseline" = "Baseline";
        half_x_height: "fontcraftTransform.halfXHeight" = "Half x-height";
        x_height: "fontcraftTransform.xHeight" = "x-height";
        half_cap_height: "fontcraftTransform.halfCapHeight" = "Half cap height";
        cap_height: "fontcraftTransform.capHeight" = "Cap height";
        layer_center: "fontcraftTransform.layerCenter" = "Layer Center";
        layer_center_tip: "fontcraftTransform.layerCenterTip" = "The centre of the layer: half the width, midway between ascender and descender";
        mirror_horizontal: "fontcraftTransform.mirrorHorizontal" = "Mirror Horizontally";
        mirror_vertical: "fontcraftTransform.mirrorVertical" = "Mirror Vertically";
        scale_x: "fontcraftTransform.scaleX" = "Horizontal Scale";
        scale_y: "fontcraftTransform.scaleY" = "Vertical Scale";
        scale_lock: "fontcraftTransform.scaleLock" = "Lock Scale Proportions";
        scale: "fontcraftTransform.scale" = "Scale";
        reverse_scale: "fontcraftTransform.reverseScale" = "Reverse Scale";
        rotate: "fontcraftTransform.rotate" = "Rotate";
        rotate_ccw: "fontcraftTransform.rotateCounterclockwise" = "Rotate Counterclockwise";
        rotate_cw: "fontcraftTransform.rotateClockwise" = "Rotate Clockwise";
        slant: "fontcraftTransform.slant" = "Slant";
        slant_left: "fontcraftTransform.slantLeft" = "Slant Left";
        slant_right: "fontcraftTransform.slantRight" = "Slant Right";
        slant_down: "fontcraftTransform.slantDown" = "Slant Down";
        slant_up: "fontcraftTransform.slantUp" = "Slant Up";
        align_left: "fontcraftTransform.alignLeft" = "Align Left";
        align_center_h: "fontcraftTransform.alignCenterHorizontal" = "Align Horizontal Centers";
        align_right: "fontcraftTransform.alignRight" = "Align Right";
        align_top: "fontcraftTransform.alignTop" = "Align Top";
        align_center_v: "fontcraftTransform.alignCenterVertical" = "Align Vertical Centers";
        align_bottom: "fontcraftTransform.alignBottom" = "Align Bottom";
        union: "fontcraftTransform.union" = "Union";
        union_tip: "fontcraftTransform.unionTip" = "Merge the closed paths";
        subtract: "fontcraftTransform.subtract" = "Subtract";
        subtract_tip: "fontcraftTransform.subtractTip" = "Subtract the selected paths from the others";
        intersect: "fontcraftTransform.intersect" = "Intersect";
        intersect_tip: "fontcraftTransform.intersectTip" = "Keep where the selected paths overlap the others";
        nothing: "fontcraftTransform.nothing" = "Open a glyph to transform it";
        /// With Font View in front.
        nothing_selected: "fontcraftTransform.nothingSelected" = "Select glyphs to transform them";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<TransformModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    super::widgets::scrolling(ui, |ui| view(&mut props, ui));
}

/// An icon button named `label` that is enabled with `enabled`; whether it was pressed.
fn icon(ui: &mut Ui, name: &'static str, label: &Label, enabled: bool) -> bool {
    IconButton::new(IconId::new(name)).label(label.clone()).tooltip(label.clone()).enabled(enabled).show(ui).clicked() && enabled
}

/// [`icon`] with a shape-operation icon.
fn shape_icon(ui: &mut Ui, glyph: ShapeIcon, label: &Label, enabled: bool) -> bool {
    IconButton::new(glyph.spec()).label(label.clone()).tooltip(label.clone()).enabled(enabled).show(ui).clicked() && enabled
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<TransformWords>();
    let m: &TransformModel = props.model;
    let sent = &mut *props.sent;
    let enabled = m.targets.is_some();
    let origin: Value = serde_json::from_str(&m.origin).unwrap_or(Value::Null);
    let press = |sent: &mut Vec<Msg>, id: &str, extra: Value| {
        if let Some(t) = &m.targets {
            sent.push(t.run(id, extra));
        }
    };

    let _ = Row::new().show(ui, |ui, _| {
        let active = (m.mode == "box").then_some(m.box_point.as_str());
        if let Some(point) = widgets::box_point(ui, words.origin.clone(), active) {
            sent.push(shell("transform.setOrigin", json!({"mode": "box", "boxPoint": point, "point": null})));
        }
        let reference = ToggleButton::new(words.reference.clone(), Bind::given(m.mode == "reference"))
            .icon(IconId::new("target"))
            .icon_only()
            .tooltip(words.reference.clone())
            .enabled(m.point.is_some())
            .show(ui);
        if reference.events.committed().is_some()
            && let Some(p) = m.point
        {
            // Pressed while it is on: the reference point goes.
            let origin = if m.mode == "reference" { json!({"point": null}) } else { json!({"mode": "reference", "point": p}) };
            sent.push(shell("transform.setOrigin", origin));
        }
        // A CJK glyph has no metric heights to pick from: the layer's centre stands in for them.
        if m.cjk {
            let on = m.mode == "layer" || m.mode == "metrics";
            let layer = ToggleButton::new(words.layer_center.clone(), Bind::given(on)).tooltip(words.layer_center_tip.clone()).show(ui);
            if layer.events.committed().is_some() {
                sent.push(shell("transform.setOrigin", json!({"mode": "layer"})));
            }
            return;
        }
        let labels = [&words.baseline, &words.x_height, &words.half_x_height, &words.cap_height, &words.half_cap_height];
        let names = ["baseline", "xHeight", "halfXHeight", "capHeight", "halfCapHeight"];
        let metrics: Vec<Entry<String>> = names.iter().zip(labels).map(|(n, l)| Entry::new((*n).to_string(), l.clone())).collect();
        let shown = if m.mode == "metrics" { m.metric.clone() } else { String::new() };
        if let Some(picked) = Select::new(words.metric.clone(), Bind::given(shown), &metrics).show(ui).events.committed()
            && METRICS.contains(&picked.as_str())
        {
            sent.push(shell("transform.setOrigin", json!({"mode": "metrics", "metric": picked})));
        }
    });

    let _ = Row::new().show(ui, |ui, _| {
        // Selected corner components flip in place: no origin.
        let mirror = |sent: &mut Vec<Msg>, id: &str| match &m.corners {
            Some(corners) => sent.push(Msg::run(id, corners.with(json!({})))),
            None => press(sent, id, json!({ "origin": origin })),
        };
        if icon(ui, "swap_horizontal", &words.mirror_horizontal, enabled) {
            mirror(sent, "transform.mirrorHorizontal");
        }
        if icon(ui, "swap_vertical", &words.mirror_vertical, enabled) {
            mirror(sent, "transform.mirrorVertical");
        }
    });

    let [scale_x, scale_y] = m.scale;
    let (sx, sy) = if m.scale_locked { (scale_x, scale_x) } else { (scale_x, scale_y) };
    // The labelled fields take most of the palette's width: what does not fit goes to the next line.
    let _ = Row::new().wrap(true).show(ui, |ui, _| {
        if let Some(v) = widgets::number(ui, words.scale_x.clone(), Some(scale_x)) {
            sent.push(shell("transform.setSettings", json!({ "scaleX": v })));
        }
        let lock = ToggleButton::new(words.scale_lock.clone(), Bind::given(m.scale_locked))
            .icon(IconId::new(if m.scale_locked { "lock" } else { "lock_open" }))
            .icon_only()
            .tooltip(words.scale_lock.clone())
            .show(ui);
        if lock.events.committed().is_some() {
            sent.push(shell("transform.setScaleLock", json!({ "locked": !m.scale_locked })));
        }
        if !m.scale_locked
            && let Some(v) = widgets::number(ui, words.scale_y.clone(), Some(sy))
        {
            sent.push(shell("transform.setSettings", json!({ "scaleY": v })));
        }
        if icon(ui, "scaling", &words.scale, enabled) {
            press(sent, "transform.scale", json!({"x": sx, "y": sy, "origin": origin}));
        }
        if Button::new(words.reverse_scale.clone()).enabled(enabled).show(ui).clicked() && enabled {
            press(sent, "transform.reverseScale", json!({"x": sx, "y": sy, "origin": origin}));
        }
    });

    let _ = Row::new().show(ui, |ui, _| {
        let angle = json!({"angle": m.rotate, "origin": origin});
        if icon(ui, "rotate_ccw", &words.rotate_ccw, enabled) {
            press(sent, "transform.rotateCounterclockwise", angle.clone());
        }
        if let Some(v) = widgets::number(ui, words.rotate.clone(), Some(m.rotate)) {
            sent.push(shell("transform.setSettings", json!({ "rotate": v })));
        }
        if icon(ui, "rotate_cw", &words.rotate_cw, enabled) {
            press(sent, "transform.rotateClockwise", angle);
        }
    });

    let _ = Row::new().wrap(true).show(ui, |ui, _| {
        let angle = json!({"angle": m.slant, "origin": origin});
        if shape_icon(ui, ShapeIcon::SlantLeft, &words.slant_left, enabled) {
            press(sent, "transform.slantLeft", angle.clone());
        }
        if let Some(v) = widgets::number(ui, words.slant.clone(), Some(m.slant)) {
            sent.push(shell("transform.setSettings", json!({ "slant": v })));
        }
        for (id, glyph, label) in [
            ("transform.slantRight", ShapeIcon::SlantRight, &words.slant_right),
            ("transform.slantDown", ShapeIcon::SlantDown, &words.slant_down),
            ("transform.slantUp", ShapeIcon::SlantUp, &words.slant_up),
        ] {
            if shape_icon(ui, glyph, label, enabled) {
                press(sent, id, angle.clone());
            }
        }
    });

    let _ = Row::new().show(ui, |ui, _| {
        for (id, name, label) in [
            ("transform.alignLeft", "align_edge_left", &words.align_left),
            ("transform.alignCenterHorizontal", "align_edge_hcenter", &words.align_center_h),
            ("transform.alignRight", "align_edge_right", &words.align_right),
            ("transform.alignTop", "align_edge_top", &words.align_top),
            ("transform.alignCenterVertical", "align_edge_vcenter", &words.align_center_v),
            ("transform.alignBottom", "align_edge_bottom", &words.align_bottom),
        ] {
            if icon(ui, name, label, enabled && m.align) {
                press(sent, id, json!({}));
            }
        }
    });

    let _ = Row::new().show(ui, |ui, _| {
        for (id, glyph, label, tip) in [
            ("transform.booleanUnion", ShapeIcon::Union.spec(), &words.union, &words.union_tip),
            ("transform.booleanSubtract", IconSpec::from(IconId::new("squares_subtract")), &words.subtract, &words.subtract_tip),
            ("transform.booleanIntersect", ShapeIcon::Intersect.spec(), &words.intersect, &words.intersect_tip),
        ] {
            if IconButton::new(glyph).label(label.clone()).tooltip(tip.clone()).enabled(enabled).show(ui).clicked()
                && enabled
                && let Some(t) = &m.boolean
            {
                sent.push(t.run(id, json!({})));
            }
        }
    });

    if !enabled {
        Text::new(if m.font_view { words.nothing_selected.clone() } else { words.nothing.clone() }).shade(Shade::Muted).show(ui);
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
    use crate::app::models::palette::{Targets, TransformModel};

    /// A typed rotate angle goes to the settings; Rotate Clockwise passes it and the origin with the targets.
    #[test]
    fn rotate_uses_the_field_and_the_origin() {
        let model = Tracked::new(TransformModel {
            mode: "box".into(),
            box_point: "center".into(),
            metric: "baseline".into(),
            scale: [110.0, 110.0],
            scale_locked: true,
            rotate: 90.0,
            slant: 10.0,
            origin: json!({"mode": "box", "boxPoint": "center"}).to_string(),
            targets: Some(Targets { json: json!({"glyph": "A", "layer": "m01"}).to_string(), ..Default::default() }),
            ..Default::default()
        });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let field = fixture.by_label("Rotate").expect("the Rotate field");
        fixture.click(&field).key(Key::A, Modifiers::COMMAND).type_text("45").key(Key::Enter, Modifiers::NONE).settle();
        let button = fixture.by_label("Rotate Clockwise").expect("the Rotate Clockwise button");
        fixture.click(&button).settle();
        assert_eq!(
            *posted.borrow(),
            vec![
                Msg::Shell(craft::ui::runtime::commands::CommandRef::with_params("transform.setSettings", json!({"rotate": 45.0}))),
                Msg::run(
                    "transform.rotateClockwise",
                    json!({"glyph": "A", "layer": "m01", "angle": 90.0, "origin": {"mode": "box", "boxPoint": "center"}})
                ),
            ]
        );
    }

    /// With corner components selected, the mirror buttons flip them: the mirror command with `corners` and no
    /// origin (`corner.mirrorViaPalette`).
    #[test]
    fn mirror_buttons_flip_selected_corners() {
        let model = Tracked::new(TransformModel {
            mode: "box".into(),
            box_point: "center".into(),
            origin: json!({"mode": "box", "boxPoint": "center"}).to_string(),
            targets: Some(Targets { json: json!({"glyph": "A", "layer": "m01"}).to_string(), ..Default::default() }),
            corners: Some(Targets { json: json!({"glyph": "A", "layer": "m01", "corners": [0, 2]}).to_string(), ..Default::default() }),
            ..Default::default()
        });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let button = fixture.by_label("Mirror Horizontally").expect("the Mirror Horizontally button");
        fixture.click(&button).settle();
        let button = fixture.by_label("Mirror Vertically").expect("the Mirror Vertically button");
        fixture.click(&button).settle();
        assert_eq!(
            *posted.borrow(),
            vec![
                Msg::run("transform.mirrorHorizontal", json!({"glyph": "A", "layer": "m01", "corners": [0, 2]})),
                Msg::run("transform.mirrorVertical", json!({"glyph": "A", "layer": "m01", "corners": [0, 2]})),
            ]
        );
    }

    /// A CJK glyph shows the layer-centre control in place of the metric heights; pressing it picks the layer origin.
    #[test]
    fn cjk_glyphs_offer_the_layer_centre() {
        let model = Tracked::new(TransformModel {
            mode: "metrics".into(),
            box_point: "center".into(),
            metric: "baseline".into(),
            origin: json!({"mode": "metrics", "metric": "baseline"}).to_string(),
            targets: Some(Targets { json: json!({"glyph": "uni4E00", "layer": "m01"}).to_string(), ..Default::default() }),
            cjk: true,
            ..Default::default()
        });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        assert!(fixture.by_label("Metric").is_none(), "no metric heights for a CJK glyph");
        let button = fixture.by_label("Layer Center").expect("the Layer Center button");
        fixture.click(&button).settle();
        assert_eq!(
            *posted.borrow(),
            vec![Msg::Shell(craft::ui::runtime::commands::CommandRef::with_params("transform.setOrigin", json!({"mode": "layer"})))]
        );
    }

    /// Reference Point works about the placed point; pressed again it takes the point away, so the Rotate and Scale
    /// tools turn about the selection's centre again.
    #[test]
    fn reference_point_is_taken_and_put_aside() {
        for (mode, sent) in [("box", json!({"mode": "reference", "point": [10.0, 20.0]})), ("reference", json!({"point": null}))] {
            let model = Tracked::new(TransformModel {
                mode: mode.into(),
                box_point: "center".into(),
                metric: "baseline".into(),
                point: Some([10.0, 20.0]),
                origin: json!({"mode": "box", "boxPoint": "center"}).to_string(),
                targets: Some(Targets { json: json!({"glyph": "A", "layer": "m01"}).to_string(), ..Default::default() }),
                ..Default::default()
            });
            let posted = Rc::new(RefCell::new(Vec::new()));
            let seen = Rc::clone(&posted);
            let mut fixture = Fixture::new(move |ui| {
                let mut sent = Vec::new();
                show(ui, Props { model: &model, sent: &mut sent });
                seen.borrow_mut().extend(sent);
            });
            fixture.settle();
            let button = fixture.by_label("Reference Point").expect("the Reference Point button");
            fixture.click(&button).settle();
            assert_eq!(*posted.borrow(), vec![Msg::Shell(craft::ui::runtime::commands::CommandRef::with_params("transform.setOrigin", sent))]);
        }
    }

    /// The palette gives the panel a fixed height: a body taller than it scrolls with the wheel.
    #[test]
    fn a_tall_body_scrolls_with_the_wheel() {
        let model = Tracked::new(TransformModel {
            mode: "box".into(),
            box_point: "center".into(),
            origin: json!({"mode": "box", "boxPoint": "center"}).to_string(),
            targets: Some(Targets { json: json!({"glyph": "A", "layer": "m01"}).to_string(), ..Default::default() }),
            ..Default::default()
        });
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
        });
        fixture.room(egui::vec2(320.0, 120.0)).settle();
        let first = fixture.texts().first().cloned().expect("the panel paints text");
        let inside = egui::pos2(160.0, 60.0);
        fixture.frame(vec![craft::ui::testing::pointer::moved(inside)]);
        fixture.frame(vec![craft::ui::testing::pointer::wheel(-80.0)]);
        fixture.advance(1.0).settle();
        let moved = fixture.texts().iter().find(|t| t.text == first.text).map(|t| t.rect).expect("the first text still paints");
        assert!(moved.min.y < first.rect.min.y - 10.0, "the body scrolled up: {:?} then {moved:?}", first.rect);
    }
}
