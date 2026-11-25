//! The interpolation curve part of the Info box (M9.32, row `infoBox.higherOrderInterpolation`): shown for the one
//! selected node in a font with an axis and two masters, over the span from its layer to the next layer up along one
//! axis. The timing graph (`hoi_timing`) follows it. The model is `app::models::hoi::HoiBox`.
//!
//! | Control | Shown for | Runs |
//! |---|---|---|
//! | Interpolation Type: Linear, Intermediate Point, Handles, Rotation | every node | `hoi.setInterpolationType` |
//! | Point X, Point Y | an intermediate point | `hoi.setControlPoints` with the one position |
//! | Lower and Upper Handle X, Y | handles | `hoi.setControlPoints` with both handles |
//! | Center X, Center Y | a rotation | `hoi.setControlPoints` with the centre |
//! | Center Rotation in Bounds | a rotation | `hoi.centerRotation` |
//! | Rotation Direction: Automatic, Clockwise, Counterclockwise | a rotation | `hoi.setRotationDirection` |
//!
//! Every control names the node, its layer and the axis, so each edit is one undo step on that node alone.

use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::layout_components::Row;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::Section;
use egui::Ui;
use serde_json::{Value, json};

use super::widgets;
use crate::app::Msg;
use crate::app::models::hoi::HoiBox;

craft::ui::words! {
    /// The words of the interpolation curve part of the Info box.
    pub struct HoiBoxWords {
        title: "fontcraftHoiBox.title" = "Interpolation Curve";
        kind: "fontcraftHoiBox.type" = "Interpolation Type";
        linear: "fontcraftHoiBox.linear" = "Linear";
        point: "fontcraftHoiBox.intermediatePoint" = "Intermediate Point";
        handles: "fontcraftHoiBox.handles" = "Handles";
        rotation: "fontcraftHoiBox.rotation" = "Rotation";
        point_x: "fontcraftHoiBox.pointX" = "Point X";
        point_y: "fontcraftHoiBox.pointY" = "Point Y";
        lower_x: "fontcraftHoiBox.lowerX" = "Lower Handle X";
        lower_y: "fontcraftHoiBox.lowerY" = "Lower Handle Y";
        upper_x: "fontcraftHoiBox.upperX" = "Upper Handle X";
        upper_y: "fontcraftHoiBox.upperY" = "Upper Handle Y";
        center_x: "fontcraftHoiBox.centerX" = "Center X";
        center_y: "fontcraftHoiBox.centerY" = "Center Y";
        center: "fontcraftHoiBox.center" = "Center Rotation in Bounds";
        /// The tooltip of the centre button.
        center_tip: "fontcraftHoiBox.centerTip" = "Put the centre back in the middle of the path's bounds on both layers";
        direction: "fontcraftHoiBox.direction" = "Rotation Direction";
        automatic: "fontcraftHoiBox.automatic" = "Automatic";
        clockwise: "fontcraftHoiBox.clockwise" = "Clockwise";
        counterclockwise: "fontcraftHoiBox.counterclockwise" = "Counterclockwise";
    }
}

/// The part for `b`, then its timing graph, whose dots show at `clock` (0 to 1); what the person changed goes to
/// `sent`.
pub fn view(ui: &mut Ui, b: &HoiBox, clock: Option<f64>, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<HoiBoxWords>();
    let run = |sent: &mut Vec<Msg>, id: &str, extra: Value| sent.push(Msg::run(id, with(&target(b), extra)));
    Section::new(words.title.clone()).id_salt("fontcraftHoiBox").show(ui, |ui, _| {
        let kinds = [("linear", &words.linear), ("intermediatePoint", &words.point), ("handles", &words.handles), ("rotation", &words.rotation)];
        if let Some(kind) = choice(ui, words.kind.clone(), &b.kind, &kinds) {
            run(sent, "hoi.setInterpolationType", json!({ "type": kind }));
        }
        let labels: Vec<[&Label; 2]> = match b.kind.as_str() {
            "intermediatePoint" => vec![[&words.point_x, &words.point_y]],
            "handles" => vec![[&words.lower_x, &words.lower_y], [&words.upper_x, &words.upper_y]],
            "rotation" => vec![[&words.center_x, &words.center_y]],
            _ => Vec::new(),
        };
        if let Some(positions) = knob_fields(ui, &labels, &b.knobs) {
            run(sent, "hoi.setControlPoints", json!({ "positions": positions }));
        }
        if b.kind == "rotation" {
            let _ = Row::new().show(ui, |ui, _| {
                if Button::new(words.center.clone()).tooltip(words.center_tip.clone()).show(ui).clicked() {
                    run(sent, "hoi.centerRotation", json!({}));
                }
                let directions = [("automatic", &words.automatic), ("clockwise", &words.clockwise), ("counterclockwise", &words.counterclockwise)];
                if let Some(direction) = choice(ui, words.direction.clone(), &b.direction, &directions) {
                    run(sent, "hoi.setRotationDirection", json!({ "direction": direction }));
                }
            });
        }
    });
    super::hoi_timing::view(ui, &b.graph, clock);
}

/// The node, its layer and the axis: the targets every `hoi.*` command of the part takes.
fn target(b: &HoiBox) -> Value {
    json!({"glyph": b.glyph, "layer": b.layer, "points": [[b.shape, b.node]], "axis": b.axis})
}

fn with(base: &Value, extra: Value) -> Value {
    let mut out = base.clone();
    if let (Some(o), Value::Object(e)) = (out.as_object_mut(), extra) {
        o.extend(e);
    }
    out
}

/// A labelled pop-up over `entries` (parameter value, label) showing `shown`; the value picked when it differs.
fn choice(ui: &mut Ui, label: Label, shown: &str, entries: &[(&str, &Label)]) -> Option<String> {
    let entries: Vec<Entry<String>> = entries.iter().map(|(v, l)| Entry::new((*v).to_string(), (*l).clone())).collect();
    widgets::labelled(ui, &label.clone(), |ui| {
        Select::new(label.clone(), Bind::given(shown.to_string()), &entries).show(ui).events.committed().cloned().filter(|v| v != shown)
    })
    .flatten()
}

/// One row of X and Y fields per knob; every knob's position, the edited coordinate replaced, when one is committed.
/// Nothing shows while the knobs do not match the labels (a knob that is not finite).
fn knob_fields(ui: &mut Ui, labels: &[[&Label; 2]], knobs: &[[f64; 2]]) -> Option<Vec<[f64; 2]>> {
    if labels.len() != knobs.len() {
        return None;
    }
    let mut out = None;
    for (k, ([lx, ly], knob)) in labels.iter().zip(knobs).enumerate() {
        let _ = Row::new().show(ui, |ui, _| {
            for (axis, label) in [lx, ly].into_iter().enumerate() {
                let value = knob.get(axis).copied();
                if let Some(v) = widgets::number(ui, (*label).clone(), value).filter(|v| v.is_finite()) {
                    let mut positions = knobs.to_vec();
                    if let Some(slot) = positions.get_mut(k).and_then(|p| p.get_mut(axis)) {
                        *slot = v;
                        out = Some(positions);
                    }
                }
            }
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};
    use serde_json::{Value, json};

    use super::view;
    use crate::app::Msg;
    use crate::app::models::hoi::{HoiBox, TimingGraph};

    fn hoi_box(kind: &str, knobs: Vec<[f64; 2]>) -> HoiBox {
        HoiBox {
            glyph: "A".into(),
            layer: "m01".into(),
            shape: 0,
            node: 2,
            axis: "wght".into(),
            kind: kind.into(),
            knobs,
            direction: "automatic".into(),
            graph: TimingGraph { axis: "wght".into(), points: vec![[0.0, 0.0], [1.0, 1.0]] },
        }
    }

    fn fixture(b: HoiBox) -> (Fixture, Rc<RefCell<Vec<Msg>>>) {
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut f = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            view(ui, &b, None, &mut sent);
            seen.borrow_mut().extend(sent);
        });
        f.settle();
        (f, posted)
    }

    /// The command the part posts, with the node's targets and `extra`.
    fn expected(id: &str, extra: Value) -> Msg {
        Msg::run(id, super::with(&json!({"glyph": "A", "layer": "m01", "points": [[0, 2]], "axis": "wght"}), extra))
    }

    /// Opens the pop-up `label` and picks `entry`.
    fn pick(f: &mut Fixture, label: &str, entry: &str) {
        let menu = f.by_label(label).unwrap_or_else(|| panic!("the {label} pop-up"));
        f.click(&menu).settle();
        let item = f.by_label(entry).unwrap_or_else(|| panic!("{entry} in the {label} pop-up"));
        f.click(&item).settle();
    }

    fn type_into(f: &mut Fixture, label: &str, text: &str) {
        let field = f.by_label(label).unwrap_or_else(|| panic!("the {label} field"));
        f.click(&field).key(Key::A, Modifiers::COMMAND).type_text(text).key(Key::Enter, Modifiers::NONE).settle();
    }

    /// The type pop-up runs the type command; a linear node shows no knob fields and no rotation controls.
    #[test]
    fn the_type_pop_up_sets_the_interpolation_type() {
        let (mut f, posted) = fixture(hoi_box("linear", Vec::new()));
        assert!(f.by_label("Point X").is_none() && f.by_label("Rotation Direction").is_none(), "nothing but the type for a linear node");
        pick(&mut f, "Interpolation Type", "Handles");
        assert_eq!(*posted.borrow(), vec![expected("hoi.setInterpolationType", json!({"type": "handles"}))]);
    }

    /// A handle coordinate sends both handles, the edited one changed.
    #[test]
    fn a_handle_field_sets_both_control_points() {
        let (mut f, posted) = fixture(hoi_box("handles", vec![[100.0, 50.0], [300.0, 80.0]]));
        type_into(&mut f, "Upper Handle Y", "120");
        assert_eq!(*posted.borrow(), vec![expected("hoi.setControlPoints", json!({"positions": [[100.0, 50.0], [300.0, 120.0]]}))]);
    }

    /// An intermediate point's X moves the point.
    #[test]
    fn the_point_fields_move_the_intermediate_point() {
        let (mut f, posted) = fixture(hoi_box("intermediatePoint", vec![[200.0, 300.0]]));
        type_into(&mut f, "Point X", "240");
        assert_eq!(*posted.borrow(), vec![expected("hoi.setControlPoints", json!({"positions": [[240.0, 300.0]]}))]);
    }

    /// A rotation's centre fields, the centre button and the direction pop-up each run their command.
    #[test]
    fn the_rotation_controls_run_their_commands() {
        let rotation = || hoi_box("rotation", vec![[250.0, 250.0]]);
        let (mut f, posted) = fixture(rotation());
        type_into(&mut f, "Center Y", "260");
        assert_eq!(*posted.borrow(), vec![expected("hoi.setControlPoints", json!({"positions": [[250.0, 260.0]]}))]);

        let (mut f, posted) = fixture(rotation());
        let center = f.by_label("Center Rotation in Bounds").expect("the centre button");
        f.click(&center).settle();
        assert_eq!(*posted.borrow(), vec![expected("hoi.centerRotation", json!({}))]);

        let (mut f, posted) = fixture(rotation());
        pick(&mut f, "Rotation Direction", "Counterclockwise");
        assert_eq!(*posted.borrow(), vec![expected("hoi.setRotationDirection", json!({"direction": "counterclockwise"}))]);
    }
}
