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
//!
//! A selected path is one selected whole or with a selected node or handle. With several, a control shows a value
//! only when every path has it. Clicking the axis-aligned cap uses the automatic axis (`capAxis: auto`); holding the
//! press on it, or a right click, opens a menu of Automatic, X Axis and Y Axis. The axis is one per path, so it serves
//! both ends. The release that ends a hold sets nothing.

use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Tracked, Trigger};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::menu::{Menu, entry_rows};
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

/// The cap whose button also opens the axis menu.
const AXIS_ALIGNED: &str = "axisAligned";

pub struct Props<'a> {
    pub model: &'a Tracked<StrokeModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    super::widgets::scrolling(ui, |ui| view(&mut props, ui));
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
            (AXIS_ALIGNED, &words.axis_aligned),
        ];
        for (key, label, current) in [("capStart", &words.cap_start, m.cap_start.as_ref()), ("capEnd", &words.cap_end, m.cap_end.as_ref())] {
            if let Some(extra) = cap_row(ui, &words, key, label, current.map(String::as_str), m.cap_axis.as_deref(), &caps) {
                set(sent, extra);
            }
        }
    }
}

/// The cap row of `key` (`capStart` or `capEnd`): one button per cap, `current` on, and on the axis-aligned button a
/// menu of axes that a held press or a right click opens. The keys to set, when a button or a menu row ran.
fn cap_row(
    ui: &mut Ui,
    words: &StrokeWords,
    key: &str,
    label: &Label,
    current: Option<&str>,
    axis: Option<&str>,
    caps: &[(&str, &Label)],
) -> Option<Value> {
    let mut chosen = None;
    let _ = Row::new().wrap(true).show(ui, |ui, _| {
        Text::new(label.clone()).show(ui);
        for (name, face) in caps {
            let on = current == Some(*name);
            let button = Button::new((*face).clone()).selected(on).id_salt(("fontcraftStroke.cap", key, *name)).show(ui);
            if *name != AXIS_ALIGNED {
                if button.clicked() {
                    chosen = Some(json!({ key: name }));
                }
                continue;
            }
            let axes = [("auto", &words.auto), ("x", &words.x_axis), ("y", &words.y_axis)];
            let entries: Vec<Entry<String>> =
                axes.iter().map(|(n, l)| Entry::new((*n).to_string(), (*l).clone()).checked(on && axis == Some(*n))).collect();
            let menu = Menu::new(words.cap_axis.clone())
                .id_salt(("fontcraftStroke.capAxis", key))
                .values()
                .opened_by(&button.response, Trigger::Hold)
                .opened_by(&button.response, Trigger::Context)
                .show(ui, |ui, _| entry_rows(ui, &entries));
            // The body runs only while the menu is open; a release on the button then ends the hold that opened it.
            let open = menu.inner.is_some();
            let picked = menu.inner.flatten().and_then(|at| entries.get(at)).and_then(Entry::value).cloned();
            match picked {
                Some(picked) => chosen = Some(json!({ key: AXIS_ALIGNED, "capAxis": picked })),
                None if button.clicked() && !open => chosen = Some(json!({ key: AXIS_ALIGNED, "capAxis": "auto" })),
                None => {}
            }
        }
    });
    chosen
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

    /// The Stroke section over one open stroked path, recording what it posts.
    fn open_path(posted: &Rc<RefCell<Vec<Msg>>>) -> Fixture {
        let model = Tracked::new(StrokeModel {
            targets: Some(Targets { json: json!({"glyph": "l", "layer": "m01", "shapes": [0]}).to_string() }),
            stroked: true,
            any_open: true,
            width: Some(40.0),
            position: Some(0.0),
            join: Some("miter".into()),
            cap_start: Some("butt".into()),
            cap_end: Some("butt".into()),
            ..Default::default()
        });
        let seen = Rc::clone(posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        fixture
    }

    /// A click on the axis-aligned cap takes the automatic axis; holding the press opens the axis menu, the release
    /// that ends the hold sets nothing, and a row of the menu forces its axis.
    #[test]
    fn the_axis_cap_takes_a_click_and_a_hold() {
        let posted = Rc::new(RefCell::new(Vec::new()));
        let mut fixture = open_path(&posted);
        let axis = fixture.by_label("Axis").expect("the Start row's axis-aligned cap");
        fixture.click(&axis).settle();
        let target = json!({"glyph": "l", "layer": "m01", "shapes": [0]});
        let with = |extra: serde_json::Value| {
            let mut params = target.clone();
            if let (Some(p), Some(e)) = (params.as_object_mut(), extra.as_object()) {
                p.extend(e.clone());
            }
            Msg::run("path.setStroke", params)
        };
        assert_eq!(*posted.borrow(), vec![with(json!({"capStart": "axisAligned", "capAxis": "auto"}))]);
        posted.borrow_mut().clear();

        fixture.press(&axis).advance(1.0).idle(3);
        fixture.release(&axis).settle();
        assert!(posted.borrow().is_empty(), "the release that ends a hold sets nothing");
        let x = fixture.by_label("X Axis").expect("the hold opens the axis menu");
        fixture.click(&x).settle();
        assert_eq!(*posted.borrow(), vec![with(json!({"capStart": "axisAligned", "capAxis": "x"}))]);
    }

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
