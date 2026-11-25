//! Palette › Fit Curve: the two range fields (`fitCurve.setRange`), minus, the eight step buttons
//! (`fitCurve.applyStep`), plus, and balance, all on the curve segments of the canvas selection. A line under the
//! steps shows the selected handles' mean length (`fitCurve.getHandleLength`). With Font View in front they act on the
//! glyphs selected there, each with the nodes last selected in it in Edit View, as one undo step.

use craft::ui::buttons::Button;
use craft::ui::contract::{Props as _, Show as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use egui::Ui;
use serde_json::{Value, json};

use super::widgets::{self, format_number};
use crate::app::Msg;
use crate::app::models::palette::FitCurveModel;

craft::ui::words! {
    /// The words of the Fit Curve section.
    pub struct FitCurveWords {
        min: "fontcraftFitCurve.min" = "Minimum";
        max: "fontcraftFitCurve.max" = "Maximum";
        balance: "fontcraftFitCurve.balance" = "Balance Handles";
        decrease: "fontcraftFitCurve.decrease" = "Decrease Handle Length";
        increase: "fontcraftFitCurve.increase" = "Increase Handle Length";
        /// Before a step's percentage in its tooltip.
        step: "fontcraftFitCurve.step" = "Fit Curve";
        /// Before the selected handles' mean length.
        current: "fontcraftFitCurve.current" = "Handles";
        hint: "fontcraftFitCurve.hint" = "Select curve segments";
        /// With Font View in front.
        font_view_hint: "fontcraftFitCurve.fontViewHint" = "Select glyphs whose nodes you selected in Edit View";
        /// Before how many selected glyphs a step leaves out.
        skipped: "fontcraftFitCurve.skipped" = "Glyphs skipped, no nodes selected";
        /// The balance button's face.
        balance_face: "fontcraftFitCurve.balanceFace" = "=";
        /// The decrease button's face.
        decrease_face: "fontcraftFitCurve.decreaseFace" = "−";
        /// The increase button's face.
        increase_face: "fontcraftFitCurve.increaseFace" = "+";
    }
}

/// The step buttons.
const STEPS: u8 = 8;

pub struct Props<'a> {
    pub model: &'a Tracked<FitCurveModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    super::widgets::scrolling(ui, |ui| view(&mut props, ui));
}

/// The percentage step `step` (1 to 8) applies: the minimum, the maximum and six even steps between.
fn percent(step: u8, min: f64, max: f64) -> f64 {
    min + f64::from(step.saturating_sub(1)) * (max - min) / f64::from(STEPS - 1)
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<FitCurveWords>();
    let m: &FitCurveModel = props.model;
    let sent = &mut *props.sent;
    let enabled = m.targets.is_some();
    let run = |sent: &mut Vec<Msg>, id: &str, extra: Value| {
        if let Some(t) = &m.targets {
            sent.push(t.run(id, extra));
        }
    };
    let range = json!({"min": m.min, "max": m.max});
    let _ = Row::new().show(ui, |ui, _| {
        if let Some(v) = widgets::number(ui, words.min.clone(), Some(m.min)) {
            sent.push(shell("fitCurve.setRange", json!({ "min": v })));
        }
        if Button::new(words.balance_face.clone()).tooltip(words.balance.clone()).enabled(enabled).show(ui).clicked() && enabled {
            run(sent, "fitCurve.balanceHandles", json!({}));
        }
        if let Some(v) = widgets::number(ui, words.max.clone(), Some(m.max)) {
            sent.push(shell("fitCurve.setRange", json!({ "max": v })));
        }
    });
    let _ = Row::new().wrap(true).show(ui, |ui, _| {
        if Button::new(words.decrease_face.clone()).tooltip(words.decrease.clone()).enabled(enabled).show(ui).clicked() && enabled {
            run(sent, "fitCurve.decrease", range.clone());
        }
        for step in 1..=STEPS {
            let tip = format!("{}: {}%", words.step.as_str(), format_number(percent(step, m.min, m.max)));
            if Button::new(step.to_string()).tooltip(tip).enabled(enabled).show(ui).clicked() && enabled {
                run(sent, "fitCurve.applyStep", json!({"step": step, "min": m.min, "max": m.max}));
            }
        }
        if Button::new(words.increase_face.clone()).tooltip(words.increase.clone()).enabled(enabled).show(ui).clicked() && enabled {
            run(sent, "fitCurve.increase", range.clone());
        }
    });
    match m.percent {
        Some(p) => {
            Text::new(format!("{} {}%", words.current.as_str(), format_number(p))).shade(Shade::Muted).show(ui);
        }
        None if !enabled => {
            Text::new(if m.font_view { words.font_view_hint.clone() } else { words.hint.clone() }).shade(Shade::Muted).show(ui);
        }
        None => {}
    }
    if enabled && m.skipped > 0 {
        Text::new(format!("{}: {}", words.skipped.as_str(), m.skipped)).shade(Shade::Muted).show(ui);
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
    use serde_json::json;

    use super::{Props, percent, show};
    use crate::app::Msg;
    use crate::app::models::palette::{FitCurveModel, Targets};

    /// Step 8 applies the maximum on the selected segments.
    #[test]
    fn a_step_applies_its_percentage() {
        assert_eq!(percent(1, 48.0, 76.0), 48.0);
        assert_eq!(percent(8, 48.0, 76.0), 76.0);
        let model = Tracked::new(FitCurveModel {
            min: 48.0,
            max: 76.0,
            targets: Some(Targets { json: json!({"glyph": "o", "layer": "m01", "points": [[0, 1]]}).to_string(), ..Default::default() }),
            percent: Some(55.0),
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
        let eight = fixture.by_label("8").expect("step 8");
        fixture.click(&eight).settle();
        assert_eq!(
            *posted.borrow(),
            vec![Msg::run("fitCurve.applyStep", json!({"glyph": "o", "layer": "m01", "points": [[0, 1]], "step": 8, "min": 48.0, "max": 76.0}))]
        );
    }
}
