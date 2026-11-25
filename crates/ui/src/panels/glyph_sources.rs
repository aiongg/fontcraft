//! Glyph Sources (`palette.glyphSources`): the shown glyph's master, intermediate and alternate layers, with each
//! one's location, whether it takes part in interpolation, and how much it contributes at the location the previews
//! show; then the masters the glyph has no layer for.
//!
//! | Control | Runs |
//! |---|---|
//! | an axis' Animate button | `view.animateAxis {axis, on}` |
//! | a source's checkbox | `glyph.setSourceActive {glyph, source, active}`; the origin master's layer has it ticked and disabled |

use craft::ui::buttons::toggle_button::ToggleButton;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, Tracked};
use craft::ui::display::{Progress, Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::Section;
use egui::Ui;
use serde_json::json;

use super::widgets::{self, format_number};
use crate::app::Msg;
use crate::app::models::glyph_sources::{AxisRow, GlyphSourcesModel, SourceEntry};

craft::ui::words! {
    /// The words of the Glyph Sources panel.
    pub struct GlyphSourcesWords {
        /// Shown without a glyph.
        no_glyph: "fontcraftGlyphSources.noGlyph" = "Open a glyph to see its sources";
        /// The axes' section.
        location: "fontcraftGlyphSources.location" = "Location";
        sources: "fontcraftGlyphSources.sources" = "Sources";
        /// The masters without a layer.
        missing: "fontcraftGlyphSources.missing" = "Masters Without a Layer";
        animate: template "fontcraftGlyphSources.animate" = "Animate {axis}";
        animate_tip: "fontcraftGlyphSources.animateTip" = "Sweep the preview back and forth along this axis";
        active_tip: "fontcraftGlyphSources.activeTip" = "Use this source in interpolation and export";
        origin_tip: "fontcraftGlyphSources.originTip" = "The origin master's layer always takes part";
        /// The influence bar's name, for screen readers.
        influence: template "fontcraftGlyphSources.influence" = "Influence of {name}";
        alternate: "fontcraftGlyphSources.alternate" = "alternate";
        /// After the count of sources left out.
        more: "fontcraftGlyphSources.more" = "more sources not shown";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<GlyphSourcesModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    widgets::scrolling(ui, |ui| view(&mut props, ui));
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<GlyphSourcesWords>();
    let model: &GlyphSourcesModel = props.model;
    let sent = &mut *props.sent;
    let Some(glyph) = model.glyph.as_deref() else {
        Text::new(words.no_glyph.clone()).shade(Shade::Muted).show(ui);
        return;
    };
    Text::new(Label::from(glyph.to_string())).show(ui);
    if let Some(problem) = &model.problem {
        Text::new(Label::from(problem.clone())).shade(Shade::Muted).show(ui);
    }
    if !model.axes.is_empty() {
        Section::new(words.location.clone()).id_salt("fontcraftGlyphSourcesLocation").show(ui, |ui, _| {
            for axis in &model.axes {
                axis_row(ui, &words, axis, sent);
            }
        });
    }
    if !model.sources.is_empty() {
        Section::new(words.sources.clone()).id_salt("fontcraftGlyphSourcesSources").show(ui, |ui, _| {
            for source in &model.sources {
                source_row(ui, &words, glyph, source, sent);
            }
            if model.more > 0 {
                Text::new(format!("{} {}", model.more, words.more.as_str())).shade(Shade::Muted).show(ui);
            }
        });
    }
    if !model.missing.is_empty() {
        Section::new(words.missing.clone()).id_salt("fontcraftGlyphSourcesMissing").show(ui, |ui, _| {
            for name in &model.missing {
                Text::new(Label::from(name.clone())).shade(Shade::Muted).show(ui);
            }
        });
    }
}

fn axis_row(ui: &mut Ui, words: &GlyphSourcesWords, axis: &AxisRow, sent: &mut Vec<Msg>) {
    let _ = Row::new().show(ui, |ui, _| {
        Text::new(format!("{} {}", axis.name, format_number(axis.value))).show(ui);
        let out = ToggleButton::new(words.animate.fill(&[("axis", axis.name.as_str())]), Bind::given(axis.animated))
            .tooltip(words.animate_tip.clone())
            .id_salt(("fontcraftGlyphSourcesAnimate", axis.tag.as_str()))
            .show(ui);
        if let Some(on) = out.events.committed() {
            sent.push(Msg::Shell(CommandRef::with_params("view.animateAxis", json!({"axis": axis.tag, "on": *on}))));
        }
    });
}

fn source_row(ui: &mut Ui, words: &GlyphSourcesWords, glyph: &str, source: &SourceEntry, sent: &mut Vec<Msg>) {
    let tip = if source.origin { words.origin_tip.clone() } else { words.active_tip.clone() };
    let out = Checkbox::new(Bind::given(source.active))
        .label(Label::from(source.name.clone()))
        .tooltip(tip)
        .enabled(!source.origin)
        .id_salt(("fontcraftGlyphSource", source.layer.as_str()))
        .show(ui);
    if let Some(active) = out.events.committed().copied().filter(|on| *on != source.active && !source.origin) {
        sent.push(Msg::run("glyph.setSourceActive", json!({"glyph": glyph, "source": source.layer, "active": active})));
    }
    let kind = if source.alternate { format!(", {}", words.alternate.as_str()) } else { String::new() };
    Text::new(format!("{}{kind}", source.location)).shade(Shade::Muted).show(ui);
    let percent = format!("{} %", format_number((source.weight * 1000.0).round() / 10.0));
    let _ = Row::new().show(ui, |ui, _| {
        // Beyond the masters a weight can be below 0 or above 1; the bar shows what fits, the number the rest.
        Progress::new(source.weight as f32).label(words.influence.fill(&[("name", source.name.as_str())])).show(ui);
        Text::new(percent.clone()).show(ui);
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::contract::Tracked;
    use craft::ui::runtime::commands::CommandRef;
    use craft::ui::testing::Fixture;
    use serde_json::json;

    use super::{Props, show};
    use crate::app::Msg;
    use crate::app::models::glyph_sources::{AxisRow, GlyphSourcesModel, SourceEntry};

    /// The section lists the axes, sources and missing masters; its checkbox and Animate button run their commands.
    #[test]
    fn the_sources_show_with_their_influence_and_switch_off() {
        let source = |layer: &str, origin: bool, weight: f64| SourceEntry {
            layer: layer.into(),
            name: format!("Master {layer}"),
            location: "wght 100".into(),
            active: true,
            origin,
            alternate: false,
            weight,
        };
        let model = Tracked::new(GlyphSourcesModel {
            glyph: Some("e".into()),
            problem: None,
            axes: vec![AxisRow { tag: "wght".into(), name: "Weight".into(), value: 300.0, animated: false }],
            sources: vec![source("thin", true, 0.75), source("black", false, 0.25)],
            more: 0,
            missing: vec!["Wide".into()],
        });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut f = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        // The body scrolls: a room tall enough keeps every row on screen.
        f.room(egui::vec2(400.0, 1200.0)).settle();
        for text in ["Weight 300", "75 %", "25 %", "Wide", "Influence of Master black"] {
            assert!(f.by_label(text).is_some(), "{text}");
        }
        let origin = f.by_label("Master thin").expect("the origin's checkbox");
        f.click(&origin).settle();
        assert!(posted.borrow().is_empty(), "the origin always takes part");
        let black = f.by_label("Master black").expect("the other checkbox");
        f.click(&black).settle();
        let animate = f.by_label("Animate Weight").expect("the axis button");
        f.click(&animate).settle();
        assert_eq!(
            *posted.borrow(),
            vec![
                Msg::run("glyph.setSourceActive", json!({"glyph": "e", "source": "black", "active": false})),
                Msg::Shell(CommandRef::with_params("view.animateAxis", json!({"axis": "wght", "on": true}))),
            ]
        );
    }
}
