//! The Info box while the TrueType Instructor is the chosen tool (`infoBox.ttInstructor`).
//!
//! The pixel size, the rendering intent and the hinting direction are always there; the other controls follow the
//! kinds of the selected instructions. Each control runs one command:
//!
//! | Control | Shown | Command |
//! |---|---|---|
//! | PPM field | always | `hint.tt.setPixelSize` |
//! | − and + beside it | always | `hint.tt.decreasePixelSize`, `hint.tt.increasePixelSize` |
//! | rendering intent pop-up | always | `hint.tt.setRenderingIntent` |
//! | direction button | always | `hint.tt.toggleDirection` with the other direction |
//! | zone pop-up: Auto, No Zone, the TT zones | a Snap selected | `hint.tt.setSnapZone` on the selected Snaps |
//! | stem pop-up: Auto, No Stem, the TT stems of the direction | a Stem selected | `hint.tt.setStemReference` on the selected Stems |
//! | five rounding buttons | a Snap, Stem, Shift or Interpolate selected | `hint.tt.setRounding` on those |
//! | the Delta table (`dialogs::tt::deltas`) | a Delta selected | `hint.tt.setDeltas` on the selected Deltas |
//!
//! Each instruction command gets the edited glyph, its layer and the positions of the selected instructions of the
//! kinds it accepts. A pop-up or a rounding button shows the value the selected instructions share, and nothing when
//! they differ.

use craft::ui::buttons::Button;
use craft::ui::buttons::toggle_button::ToggleButton;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::Section;
use egui::Ui;
use serde_json::{Value, json};

use super::widgets;
use crate::app::Msg;
use crate::app::models::hint_canvas::{Choice, ROUNDINGS, TtBox};
use crate::canvas::hinting::{MAX_PPM, MIN_PPM, RenderingIntent};
use crate::dialogs::tt::deltas::{self, Outcome};

craft::ui::words! {
    /// The words of the Instructor's Info box.
    pub struct TtBoxWords {
        title: "fontcraftInfoBoxTt.title" = "TrueType Instructor";
        ppm: "fontcraftInfoBoxTt.ppm" = "PPM";
        smaller: "fontcraftInfoBoxTt.smaller" = "−";
        smaller_tip: "fontcraftInfoBoxTt.smallerTip" = "Smaller pixel size (,)";
        larger: "fontcraftInfoBoxTt.larger" = "+";
        larger_tip: "fontcraftInfoBoxTt.largerTip" = "Larger pixel size (.)";
        intent: "fontcraftInfoBoxTt.intent" = "Rendering intent";
        grayscale: "fontcraftInfoBoxTt.grayscale" = "Grayscale";
        cleartype: "fontcraftInfoBoxTt.cleartype" = "ClearType";
        directwrite: "fontcraftInfoBoxTt.directwrite" = "DirectWrite";
        vertical: "fontcraftInfoBoxTt.vertical" = "↕";
        vertical_tip: "fontcraftInfoBoxTt.verticalTip" = "Vertical hinting: points move up and down. Click for horizontal (X)";
        horizontal: "fontcraftInfoBoxTt.horizontal" = "↔";
        horizontal_tip: "fontcraftInfoBoxTt.horizontalTip" = "Horizontal hinting: points move left and right. Click for vertical (X)";
        zone: "fontcraftInfoBoxTt.zone" = "Zone";
        stem: "fontcraftInfoBoxTt.stem" = "Stem";
        auto: "fontcraftInfoBoxTt.auto" = "Auto";
        no_zone: "fontcraftInfoBoxTt.noZone" = "No Zone";
        no_stem: "fontcraftInfoBoxTt.noStem" = "No Stem";
        round: "fontcraftInfoBoxTt.round" = "Round";
        up: "fontcraftInfoBoxTt.up" = "Up";
        up_tip: "fontcraftInfoBoxTt.upTip" = "Round up (right in the horizontal direction)";
        down: "fontcraftInfoBoxTt.down" = "Down";
        down_tip: "fontcraftInfoBoxTt.downTip" = "Round down (left in the horizontal direction)";
        no_rounding: "fontcraftInfoBoxTt.noRounding" = "None";
        no_rounding_tip: "fontcraftInfoBoxTt.noRoundingTip" = "No rounding";
        gdi: "fontcraftInfoBoxTt.gdi" = "GDI";
        gdi_tip: "fontcraftInfoBoxTt.gdiTip" = "Round only in GDI ClearType";
        deltas: "fontcraftInfoBoxTt.deltas" = "Deltas";
    }
}

/// `params` with the target of `list`: the edited glyph, its layer and the instructions.
fn on(b: &TtBox, id: &'static str, list: &[usize], key: &str, value: Value) -> Option<Msg> {
    if list.is_empty() {
        return None;
    }
    let mut p = json!({"glyph": b.glyph, "layer": b.layer, "instructions": list});
    if let Some(m) = p.as_object_mut() {
        m.insert(key.to_string(), value);
    }
    Some(Msg::run(id, p))
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(craft::ui::runtime::commands::CommandRef::with_params(id, params))
}

/// A zone or stem pop-up: Auto, the "none" item, then `names`; the shared value shows, nothing when they differ.
fn choice(ui: &mut Ui, label: Label, current: Option<&Choice>, auto: &Label, none: &Label, names: &[String]) -> Option<String> {
    let mut entries = vec![Entry::new("auto".to_string(), auto.clone()), Entry::new("none".to_string(), none.clone())];
    entries.extend(names.iter().map(|n| Entry::new(n.clone(), Label::from(n.clone()))));
    let shown = current.map(|c| c.param().to_string()).unwrap_or_default();
    Select::new(label, Bind::given(shown.clone()), &entries).show(ui).events.committed().cloned().filter(|v| *v != shown)
}

/// The box of `b`; what the person changed goes to `sent`.
pub fn view(ui: &mut Ui, b: &TtBox, sent: &mut Vec<Msg>) {
    let w = Env::of(ui).words::<TtBoxWords>();
    Section::new(w.title.clone()).show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                if Button::new(w.smaller.clone()).tooltip(w.smaller_tip.clone()).enabled(b.ppm > MIN_PPM).show(ui).clicked() {
                    sent.push(shell("hint.tt.decreasePixelSize", json!({})));
                }
                if let Some(v) = widgets::number(ui, w.ppm.clone(), Some(f64::from(b.ppm))) {
                    let ppm = v.round().clamp(f64::from(MIN_PPM), f64::from(MAX_PPM));
                    if ppm.is_finite() {
                        sent.push(shell("hint.tt.setPixelSize", json!({"ppm": ppm as u16})));
                    }
                }
                if Button::new(w.larger.clone()).tooltip(w.larger_tip.clone()).enabled(b.ppm < MAX_PPM).show(ui).clicked() {
                    sent.push(shell("hint.tt.increasePixelSize", json!({})));
                }
                let intents: Vec<Entry<String>> = RenderingIntent::ALL
                    .iter()
                    .zip([&w.grayscale, &w.cleartype, &w.directwrite])
                    .map(|(i, l)| Entry::new(i.as_str().to_string(), l.clone()))
                    .collect();
                let shown = b.intent.as_str().to_string();
                if let Some(picked) = Select::new(w.intent.clone(), Bind::given(shown.clone()), &intents).show(ui).events.committed()
                    && *picked != shown
                {
                    sent.push(shell("hint.tt.setRenderingIntent", json!({"intent": picked})));
                }
                let (arrow, tip, other) =
                    if b.vertical { (&w.vertical, &w.vertical_tip, "horizontal") } else { (&w.horizontal, &w.horizontal_tip, "vertical") };
                if Button::new(arrow.clone()).tooltip(tip.clone()).show(ui).clicked() {
                    sent.push(shell("hint.tt.toggleDirection", json!({"direction": other})));
                }
            });
            if !b.snaps.is_empty()
                && let Some(zone) = choice(ui, w.zone.clone(), b.zone.as_ref(), &w.auto, &w.no_zone, &b.zones)
            {
                sent.extend(on(b, "hint.tt.setSnapZone", &b.snaps, "zone", json!(zone)));
            }
            if !b.stems.is_empty()
                && let Some(stem) = choice(ui, w.stem.clone(), b.stem.as_ref(), &w.auto, &w.no_stem, &b.stem_names)
            {
                sent.extend(on(b, "hint.tt.setStemReference", &b.stems, "stem", json!(stem)));
            }
            if !b.rounded.is_empty() {
                Row::new().show(ui, |ui, _| {
                    let labels = [
                        (&w.round, &w.round),
                        (&w.up, &w.up_tip),
                        (&w.down, &w.down_tip),
                        (&w.no_rounding, &w.no_rounding_tip),
                        (&w.gdi, &w.gdi_tip),
                    ];
                    for ((_, param), (label, tip)) in ROUNDINGS.iter().zip(labels) {
                        let on_now = b.rounding.as_deref() == Some(*param);
                        let out = ToggleButton::new(label.clone(), Bind::given(on_now)).tooltip(tip.clone()).show(ui);
                        if out.events.committed().is_some() && !on_now {
                            sent.extend(on(b, "hint.tt.setRounding", &b.rounded, "rounding", json!(param)));
                        }
                    }
                });
            }
            if let Some(table) = &b.table {
                let table = deltas::DeltaTable { title: w.deltas.as_str().to_string(), ..table.clone() };
                if let Some(Outcome::Click(click)) = deltas::view(ui, &table)
                    && let Some(cells) = table.cells_param(&click)
                {
                    sent.extend(on(b, "hint.tt.setDeltas", &b.deltas, "cells", cells));
                }
            }
        });
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;

    use super::*;

    fn tt_box() -> TtBox {
        TtBox {
            glyph: "A".into(),
            layer: "m01".into(),
            vertical: true,
            ppm: 12,
            intent: RenderingIntent::Grayscale,
            snaps: vec![0],
            stems: Vec::new(),
            rounded: vec![0, 2],
            deltas: Vec::new(),
            zone: Some(Choice::Auto),
            stem: None,
            rounding: Some("round".into()),
            zones: vec!["xHeight".into()],
            stem_names: Vec::new(),
            table: None,
        }
    }

    #[test]
    fn the_instructor_box_runs_pixel_size_direction_and_rounding_commands() {
        let b = tt_box();
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut f = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            view(ui, &b, &mut sent);
            seen.borrow_mut().extend(sent);
        });
        f.settle();
        assert!(f.by_label("Stem").is_none(), "no Stem is selected, so no stem pop-up");
        let larger = f.by_label("+").expect("the + button");
        f.click(&larger).settle();
        let direction = f.by_label("↕").expect("the direction button");
        f.click(&direction).settle();
        let up = f.by_label("Up").expect("the Up rounding button");
        f.click(&up).settle();
        assert_eq!(
            *posted.borrow(),
            vec![
                shell("hint.tt.increasePixelSize", json!({})),
                shell("hint.tt.toggleDirection", json!({"direction": "horizontal"})),
                Msg::run("hint.tt.setRounding", json!({"glyph": "A", "layer": "m01", "instructions": [0, 2], "rounding": "up"})),
            ]
        );
    }
}
