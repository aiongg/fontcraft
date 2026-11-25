//! Glyph Sources (`palette.glyphSources`): the shown glyph's master, intermediate and alternate layers, with each
//! one's location, whether it takes part in interpolation, and how much it contributes at the location the previews
//! show; then the masters the glyph has no layer for.
//!
//! | Control | Runs |
//! |---|---|
//! | an axis' slider and number field | `view.setLocation {location}` in external coordinates |
//! | an axis' Reset button | `view.setLocation {reset}` |
//! | an axis' Animate button | `view.animateAxis {axis, on}` |
//! | Show Outline | `view.showOutlineAtLocation {on}` |
//! | Add Source | `layers.addIntermediate {glyph, location}` at the previewed location |
//! | a source's Delete button, on layers that are no master layers | `layers.delete {glyphs, layers}` |
//! | a source's checkbox | `glyph.setSourceActive {glyph, source, active}`; the origin master's layer has it ticked and disabled |

use craft::ui::buttons::Button;
use craft::ui::buttons::toggle_button::ToggleButton;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, Tracked};
use craft::ui::display::{Progress, Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::Section;
use craft::ui::value_drag::Slider;
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
        /// An axis' slider, for screen readers.
        slider: template "fontcraftGlyphSources.slider" = "{axis} slider";
        reset: template "fontcraftGlyphSources.reset" = "Reset {axis}";
        /// An axis' internal coordinate, beside the external one.
        internal: template "fontcraftGlyphSources.internal" = "internal {value}";
        outline: "fontcraftGlyphSources.outline" = "Show Outline";
        outline_tip: "fontcraftGlyphSources.outlineTip" = "Draw the glyph as it interpolates at this location behind the edited layer";
        add: "fontcraftGlyphSources.add" = "Add Source";
        add_tip: "fontcraftGlyphSources.addTip" = "Add an intermediate layer at this location, drawn as the glyph interpolates here";
        there_tip: "fontcraftGlyphSources.thereTip" = "The glyph has a source at this location";
        delete: template "fontcraftGlyphSources.delete" = "Delete {name}";
        /// Before what makes a source's layer incompatible.
        incompatible: "fontcraftGlyphSources.incompatible" = "Cannot interpolate:";
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
            location_row(ui, &words, glyph, model, sent);
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

/// Show Outline and Add Source.
fn location_row(ui: &mut Ui, words: &GlyphSourcesWords, glyph: &str, model: &GlyphSourcesModel, sent: &mut Vec<Msg>) {
    let _ = Row::new().show(ui, |ui, _| {
        let out = ToggleButton::new(words.outline.clone(), Bind::given(model.outline_shows))
            .tooltip(words.outline_tip.clone())
            .id_salt("fontcraftGlyphSourcesOutline")
            .show(ui);
        if let Some(on) = out.events.committed() {
            sent.push(Msg::Shell(CommandRef::with_params("view.showOutlineAtLocation", json!({"on": *on}))));
        }
        let tip = if model.source_there { words.there_tip.clone() } else { words.add_tip.clone() };
        let add = Button::new(words.add.clone()).tooltip(tip).enabled(!model.source_there).id_salt("fontcraftGlyphSourcesAdd").show(ui);
        if add.clicked() && !model.source_there {
            let location: serde_json::Map<String, serde_json::Value> = model.location.iter().map(|(tag, v)| (tag.clone(), json!(v))).collect();
            sent.push(Msg::run("layers.addIntermediate", json!({"glyph": glyph, "location": location})));
        }
    });
}

fn axis_row(ui: &mut Ui, words: &GlyphSourcesWords, axis: &AxisRow, sent: &mut Vec<Msg>) {
    let mut moved = None;
    let _ = Row::new().show(ui, |ui, _| {
        Text::new(axis.name.clone()).show(ui);
        if let Some(value) = widgets::bare_number(ui, Label::from(axis.name.clone()), Some(axis.external)) {
            moved = Some(value);
        }
        Text::new(words.internal.fill(&[("value", format_number(axis.value).as_str())])).shade(Shade::Muted).show(ui);
    });
    if axis.max > axis.min {
        let mut value = axis.external.clamp(axis.min, axis.max);
        let before = value;
        Slider::new(Bind::bound(&mut value), axis.min..=axis.max)
            .label(words.slider.fill(&[("axis", axis.name.as_str())]))
            .id_salt(("fontcraftGlyphSourcesSlider", axis.tag.as_str()))
            .show(ui);
        // The preview follows while the slider is dragged.
        if value != before {
            moved = Some(value);
        }
    }
    if let Some(value) = moved.filter(|v| v.is_finite()) {
        sent.push(Msg::Shell(CommandRef::with_params("view.setLocation", json!({"location": {axis.tag.as_str(): value}}))));
    }
    let _ = Row::new().show(ui, |ui, _| {
        let reset = Button::new(words.reset.fill(&[("axis", axis.name.as_str())]))
            .enabled(!axis.at_origin)
            .id_salt(("fontcraftGlyphSourcesReset", axis.tag.as_str()))
            .show(ui);
        if reset.clicked() && !axis.at_origin {
            sent.push(Msg::Shell(CommandRef::with_params("view.setLocation", json!({"reset": axis.tag}))));
        }
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
    if source.deletable {
        let delete = Button::new(words.delete.fill(&[("name", source.name.as_str())]))
            .id_salt(("fontcraftGlyphSourcesDelete", source.layer.as_str()))
            .show(ui);
        if delete.clicked() {
            sent.push(Msg::run("layers.delete", json!({"glyphs": [glyph], "layers": [source.layer]})));
        }
    }
    if !source.mismatch.is_empty() {
        Text::new(format!("{} {}", words.incompatible.as_str(), source.mismatch)).show(ui);
    }
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
            deletable: layer == "medium",
            mismatch: if layer == "black" { "thin and black differ".into() } else { String::new() },
        };
        let model = Tracked::new(GlyphSourcesModel {
            glyph: Some("e".into()),
            problem: None,
            axes: vec![AxisRow {
                tag: "wght".into(),
                name: "Weight".into(),
                value: 30.0,
                external: 300.0,
                min: 100.0,
                max: 900.0,
                at_origin: false,
                animated: false,
            }],
            location: vec![("wght".into(), 30.0)],
            source_there: false,
            outline_shows: false,
            sources: vec![source("thin", true, 0.75), source("black", false, 0.25), source("medium", false, 0.0)],
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
        for text in ["internal 30", "75 %", "25 %", "Wide", "Influence of Master black", "Weight slider", "Cannot interpolate: thin and black differ"]
        {
            assert!(f.by_label(text).is_some(), "{text}");
        }
        let origin = f.by_label("Master thin").expect("the origin's checkbox");
        f.click(&origin).settle();
        assert!(posted.borrow().is_empty(), "the origin always takes part");
        let black = f.by_label("Master black").expect("the other checkbox");
        f.click(&black).settle();
        let animate = f.by_label("Animate Weight").expect("the axis button");
        f.click(&animate).settle();
        let reset = f.by_label("Reset Weight").expect("the reset button");
        f.click(&reset).settle();
        let add = f.by_label("Add Source").expect("the add button");
        f.click(&add).settle();
        assert!(f.by_label("Delete Master black").is_none(), "a master layer is not deleted here");
        let delete = f.by_label("Delete Master medium").expect("the delete button");
        f.click(&delete).settle();
        assert_eq!(
            *posted.borrow(),
            vec![
                Msg::run("glyph.setSourceActive", json!({"glyph": "e", "source": "black", "active": false})),
                Msg::Shell(CommandRef::with_params("view.animateAxis", json!({"axis": "wght", "on": true}))),
                Msg::Shell(CommandRef::with_params("view.setLocation", json!({"reset": "wght"}))),
                Msg::run("layers.addIntermediate", json!({"glyph": "e", "location": {"wght": 30.0}})),
                Msg::run("layers.delete", json!({"glyphs": ["e"], "layers": ["medium"]})),
            ]
        );
    }
}
