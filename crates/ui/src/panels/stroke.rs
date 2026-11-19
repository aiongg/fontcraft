//! Palette › Stroke: the stroke of the selected paths, shown while a path is selected. Every control runs
//! `path.setStroke` on those paths with only the key it changes.
//!
//! | Control | Key | Shown |
//! |---|---|---|
//! | width; emptied, it removes the stroke | `width` | always |
//! | height; empty means the width | `height` | with a stroke |
//! | position in percent, and the presets −100 (left), 0 (centre), 100 (right) | `position` | with a stroke |
//! | Fill | `fill` | with a stroke on a closed path |
//! | Join: miter, bevel, round, round full | `join` | with a stroke |
//! | Start and End caps: butt, square, round, round inset, axis-aligned | `capStart`, `capEnd` | with a stroke on an open path |
//! | Cap axis: automatic, X, Y | `capAxis` | while a cap is axis-aligned |
//!
//! A selected path is one selected whole or with a selected node or handle. With several, a control shows a value
//! only when every path has it.

use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label};
use egui::Ui;
use serde_json::{Value, json};

use super::widgets::{self, NumberEdit};
use crate::app::Msg;
use crate::app::models::palette::StrokeModel;

craft::ui::words! {
    /// The words of the Stroke section.
    pub struct StrokeWords {
        width: "fontcraftStroke.width" = "Stroke Width";
        height: "fontcraftStroke.height" = "Stroke Height";
        position: "fontcraftStroke.position" = "Position";
        left: "fontcraftStroke.left" = "Left of the path";
        centre: "fontcraftStroke.centre" = "Centred on the path";
        right: "fontcraftStroke.right" = "Right of the path";
        left_face: "fontcraftStroke.leftFace" = "−100";
        centre_face: "fontcraftStroke.centreFace" = "0";
        right_face: "fontcraftStroke.rightFace" = "100";
        fill: "fontcraftStroke.fill" = "Fill";
        join: "fontcraftStroke.join" = "Join";
        miter: "fontcraftStroke.miter" = "Miter";
        bevel: "fontcraftStroke.bevel" = "Bevel";
        round: "fontcraftStroke.round" = "Round";
        round_full: "fontcraftStroke.roundFull" = "Round Full";
        cap_start: "fontcraftStroke.capStart" = "Start";
        cap_end: "fontcraftStroke.capEnd" = "End";
        butt: "fontcraftStroke.butt" = "Butt";
        square: "fontcraftStroke.square" = "Square";
        round_inset: "fontcraftStroke.roundInset" = "Round Inset";
        axis_aligned: "fontcraftStroke.axisAligned" = "Axis";
        cap_axis: "fontcraftStroke.capAxis" = "Cap Axis";
        auto: "fontcraftStroke.auto" = "Automatic";
        x_axis: "fontcraftStroke.xAxis" = "X Axis";
        y_axis: "fontcraftStroke.yAxis" = "Y Axis";
        none: "fontcraftStroke.none" = "Select a path";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<StrokeModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

/// A segmented row named `label` over `current` (`None`: mixed, no segment on); the value picked.
fn choice(ui: &mut Ui, label: &Label, current: Option<&String>, options: &[(&str, &Label)]) -> Option<String> {
    let entries: Vec<Entry<String>> = options.iter().map(|(name, l)| Entry::new((*name).to_string(), (*l).clone())).collect();
    let was = current.cloned().unwrap_or_default();
    let mut value = was.clone();
    Segmented::new(label.clone(), Bind::bound(&mut value), &entries).show(ui);
    (value != was).then_some(value)
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<StrokeWords>();
    let m: &StrokeModel = props.model;
    let sent = &mut *props.sent;
    let Some(targets) = &m.targets else {
        Text::new(words.none.clone()).shade(Shade::Muted).show(ui);
        return;
    };
    let set = |sent: &mut Vec<Msg>, extra: Value| sent.push(Msg::run("path.setStroke", targets.with(extra)));

    let _ = Row::new().wrap(true).show(ui, |ui, _| {
        match widgets::clearable_number(ui, words.width.clone(), m.width) {
            Some(NumberEdit::Set(v)) => set(sent, json!({ "width": v })),
            Some(NumberEdit::Clear) => set(sent, json!({ "width": null })),
            None => {}
        }
        if m.stroked {
            match widgets::clearable_number(ui, words.height.clone(), m.height) {
                Some(NumberEdit::Set(v)) => set(sent, json!({ "height": v })),
                Some(NumberEdit::Clear) => set(sent, json!({ "height": null })),
                None => {}
            }
        }
    });
    if !m.stroked {
        return;
    }

    let _ = Row::new().wrap(true).show(ui, |ui, _| {
        if let Some(v) = widgets::number(ui, words.position.clone(), m.position) {
            set(sent, json!({ "position": v.clamp(-100.0, 100.0) }));
        }
        for (value, face, tip) in
            [(-100.0, &words.left_face, &words.left), (0.0, &words.centre_face, &words.centre), (100.0, &words.right_face, &words.right)]
        {
            if Button::new(face.clone()).tooltip(tip.clone()).selected(m.position == Some(value)).show(ui).clicked() {
                set(sent, json!({ "position": value }));
            }
        }
    });

    if m.any_closed {
        let fill = m.fill.unwrap_or(false);
        if Checkbox::new(Bind::given(fill)).label(words.fill.clone()).show(ui).events.committed().is_some() {
            set(sent, json!({ "fill": !fill }));
        }
    }

    let joins = [("miter", &words.miter), ("bevel", &words.bevel), ("round", &words.round), ("roundFull", &words.round_full)];
    if let Some(join) = choice(ui, &words.join, m.join.as_ref(), &joins) {
        set(sent, json!({ "join": join }));
    }

    if m.any_open {
        let caps = [
            ("butt", &words.butt),
            ("square", &words.square),
            ("round", &words.round),
            ("roundInset", &words.round_inset),
            ("axisAligned", &words.axis_aligned),
        ];
        for (key, label, current) in [("capStart", &words.cap_start, m.cap_start.as_ref()), ("capEnd", &words.cap_end, m.cap_end.as_ref())] {
            if let Some(cap) = choice(ui, label, current, &caps) {
                set(sent, json!({ key: cap }));
            }
        }
        let aligned = [&m.cap_start, &m.cap_end].iter().any(|c| c.as_deref() == Some("axisAligned"));
        if aligned {
            let axes = [("auto", &words.auto), ("x", &words.x_axis), ("y", &words.y_axis)];
            let entries: Vec<Entry<String>> = axes.iter().map(|(n, l)| Entry::new((*n).to_string(), (*l).clone())).collect();
            let shown = m.cap_axis.clone().unwrap_or_default();
            if let Some(axis) = Select::new(words.cap_axis.clone(), Bind::given(shown.clone()), &entries).show(ui).events.committed()
                && *axis != shown
            {
                set(sent, json!({ "capAxis": axis }));
            }
        }
    }
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
    use crate::app::models::palette::{StrokeModel, Targets};

    /// Emptying the width removes the stroke of the selected paths.
    #[test]
    fn an_emptied_width_removes_the_stroke() {
        let model = Tracked::new(StrokeModel {
            targets: Some(Targets { json: json!({"glyph": "l", "layer": "m01", "shapes": [0]}).to_string() }),
            stroked: true,
            any_open: true,
            width: Some(40.0),
            position: Some(0.0),
            join: Some("miter".into()),
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
        let width = fixture.by_label("Stroke Width").expect("the width field");
        fixture.click(&width).key(Key::A, Modifiers::COMMAND).key(Key::Backspace, Modifiers::NONE).key(Key::Enter, Modifiers::NONE).settle();
        assert_eq!(*posted.borrow(), vec![Msg::run("path.setStroke", json!({"glyph": "l", "layer": "m01", "shapes": [0], "width": null}))]);
    }
}
