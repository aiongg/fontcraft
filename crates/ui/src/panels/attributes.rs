//! Attributes: stroke, fill, gradient and shadows of the selected paths of a full-colour layer, each control running an
//! `attributes.*` command on those paths (rows `inspector.attributes`, `inspector.attributes.gradientBar`).
//!
//! | Control | Command |
//! |---|---|
//! | Add: Gradient, Shadow, Inner Shadow, Pattern Image | `attributes.addAttribute` |
//! | Stroke width; emptied, no stroke width | `attributes.setStrokeWidth` |
//! | Stroke alignment: centre, inside, outside | `attributes.setStrokeAlignment` |
//! | Stroke colour | `attributes.setStrokeColor` |
//! | Remove Stroke | `attributes.removeStroke` |
//! | Fill colour (hidden while a gradient is present) | `attributes.setFillColor` |
//! | Remove Fill | `attributes.removeFill` |
//! | Gradient type: linear, radial | `attributes.setGradientType` |
//! | Remove Gradient | `attributes.removeGradient` |
//! | the gradient bar: a stop added, removed, dragged or recoloured | `attributes.addGradientStop`, `removeGradientStop`, `setGradientStopColor` |
//! | Add Stop: a stop in the middle of the widest gap | `attributes.addGradientStop` |
//! | a stop's offset or colour | `attributes.setGradientStopColor` with `offset` or `color` |
//! | a stop's Remove (never below two stops) | `attributes.removeGradientStop` |
//!
//! A colour shows as a well beside its `#RRGGBBAA` text; a click on the well opens a colour picker with an opacity
//! row. A fully transparent colour is a removed fill or stroke. A colour field commits on Enter or leaving it, and the
//! picker and the gradient bar commit when a gesture ends, so one edit is one undo step. With several paths, a control shows
//! a value only when every path has it.

use craft::behavior::Mixed;
use craft::color_ui::ui::gradient::{ColorStop, GradientEditor, GradientStops, StopRows, StopRules};
use craft::color_ui::ui::{ColorPopover, PickedColor};
use craft::theme::Srgba;
use craft::ui::buttons::Button;
use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::{Value, json};

use super::widgets::{self, NumberEdit};
use crate::app::Msg;
use crate::app::models::color::{AttributesModel, Rgba8, ShadowLine, hex, parse_hex};

/// Longest colour text a colour field takes.
const MAX_COLOR_TEXT: usize = 16;

craft::ui::words! {
    /// The words of the Attributes panel.
    pub struct AttributesWords {
        add: "fontcraftAttributes.add" = "Add";
        gradient: "fontcraftAttributes.gradient" = "Gradient";
        shadow: "fontcraftAttributes.shadow" = "Shadow";
        inner_shadow: "fontcraftAttributes.innerShadow" = "Inner Shadow";
        pattern_image: "fontcraftAttributes.patternImage" = "Pattern Image";
        stroke_width: "fontcraftAttributes.strokeWidth" = "Stroke Width";
        alignment: "fontcraftAttributes.alignment" = "Stroke Alignment";
        centre: "fontcraftAttributes.centre" = "Centre";
        inside: "fontcraftAttributes.inside" = "Inside";
        outside: "fontcraftAttributes.outside" = "Outside";
        stroke_color: "fontcraftAttributes.strokeColor" = "Stroke Colour";
        remove_stroke: "fontcraftAttributes.removeStroke" = "Remove Stroke";
        fill_color: "fontcraftAttributes.fillColor" = "Fill Colour";
        remove_fill: "fontcraftAttributes.removeFill" = "Remove Fill";
        gradient_type: "fontcraftAttributes.gradientType" = "Gradient Type";
        linear: "fontcraftAttributes.linear" = "Linear";
        radial: "fontcraftAttributes.radial" = "Radial";
        remove_gradient: "fontcraftAttributes.removeGradient" = "Remove Gradient";
        gradient_bar: "fontcraftAttributes.gradientBar" = "Gradient Stops";
        add_stop: "fontcraftAttributes.addStop" = "Add Stop";
        stop_offset: "fontcraftAttributes.stopOffset" = "Offset";
        stop_color: "fontcraftAttributes.stopColor" = "Stop Colour";
        remove_stop: "fontcraftAttributes.removeStop" = "Remove Stop";
        shadow_line: "fontcraftAttributes.shadowLine" = "Shadow: blur {blur}, offset {x}, {y}";
        inner_shadow_line: "fontcraftAttributes.innerShadowLine" = "Inner Shadow: blur {blur}, offset {x}, {y}";
        none: "fontcraftAttributes.none" = "Select a path on a full-colour layer";
    }
}

craft::ui::words! {
    /// The words of a colour field.
    pub struct ColorFieldWords {
        /// Follows the field's label in the name of its swatch.
        swatch: "fontcraftColorField.swatch" = "swatch";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<AttributesModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    super::widgets::scrolling(ui, |ui| view(&mut props, ui));
}

/// `c` as craft's colour.
pub fn srgba([r, g, b, a]: Rgba8) -> Srgba {
    Srgba::new(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, f32::from(a) / 255.0)
}

/// A colour well of `color` beside its `#RRGGBBAA` text (`None`: mixed, an empty field). A click on the well opens a
/// colour picker with an opacity row. Returns the colour, once, when the picker ends an edit or the field commits a
/// readable colour other than `color`.
pub fn color_field(ui: &mut Ui, label: &Label, color: Option<Rgba8>, salt: impl std::hash::Hash + std::fmt::Debug) -> Option<Rgba8> {
    color_cell(ui, label, color, salt).0
}

/// [`color_field`], with the response of its row.
pub fn color_cell(ui: &mut Ui, label: &Label, color: Option<Rgba8>, salt: impl std::hash::Hash + std::fmt::Debug) -> (Option<Rgba8>, egui::Response) {
    let shown = color.map(hex).unwrap_or_default();
    let mut picked = None;
    let row = Row::new().show(ui, |ui, _| {
        let well = color.map_or(Mixed::Mixed, |c| Mixed::Same(PickedColor::rgb(srgba(c))));
        let swatch = format!("{} {}", label.as_str(), Env::of(ui).words::<ColorFieldWords>().swatch.as_str());
        let popover = ColorPopover::new(Label::from(swatch), Bind::given(well)).picker(|p| p.alpha(true)).show(ui);
        if let Some(c) = popover.events.committed().map(|p| rgba8(p.color))
            && Some(c) != color
        {
            picked = Some(c);
        }
        let out = TextField::new(label.clone(), Bind::given(shown.clone())).mono().limit(MAX_COLOR_TEXT).id_salt(&salt).show(ui);
        if let Some(text) = out.events.committed()
            && let Some(c) = parse_hex(text)
            && Some(c) != color
        {
            picked = Some(c);
        }
    });
    (picked, row.output)
}

/// craft's colour as 8-bit components; a component that is not a number is 0.
pub fn rgba8(c: Srgba) -> Rgba8 {
    let byte = |v: f32| if v.is_finite() { (v.clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 };
    [byte(c.r), byte(c.g), byte(c.b), byte(c.a)]
}

/// Most stops a gradient holds (the attributes commands' limit).
const MAX_STOPS: usize = 256;
/// Offsets closer than this are the same.
const OFFSET_EPSILON: f64 = 1e-4;

/// `stops` as the gradient bar's stops, each tagged with its index plus one so an edit can be traced to the stop it
/// touched.
pub fn gradient_stops(stops: &[(f64, Rgba8)]) -> GradientStops {
    GradientStops::new(
        stops
            .iter()
            .enumerate()
            .map(|(i, (offset, colour))| {
                let mut stop = ColorStop::new(offset.clamp(0.0, 1.0) as f32, srgba(*colour));
                stop.tag = i as u64 + 1;
                stop
            })
            .collect(),
    )
}

/// The command one edit of the gradient bar makes of `old`, the stops the paths hold: a stop the bar added (tag 0),
/// a stop it removed, or a stop it moved or recoloured. `None` when nothing changed.
pub fn stop_edit(old: &[(f64, Rgba8)], new: &GradientStops) -> Option<(&'static str, Value)> {
    if let Some(added) = new.color.iter().find(|s| s.tag == 0) {
        return Some((
            "attributes.addGradientStop",
            json!({ "offset": f64::from(added.location).clamp(0.0, 1.0), "color": hex(rgba8(added.color)) }),
        ));
    }
    let kept = |i: usize| new.color.iter().any(|s| s.tag == i as u64 + 1);
    if let Some(gone) = (0..old.len()).find(|&i| !kept(i)) {
        return Some(("attributes.removeGradientStop", json!({ "stop": gone })));
    }
    new.color.iter().find_map(|s| {
        let i = usize::try_from(s.tag.checked_sub(1)?).ok()?;
        let (offset, colour) = old.get(i)?;
        let mut params = serde_json::Map::new();
        params.insert("stop".into(), json!(i));
        let at = f64::from(s.location).clamp(0.0, 1.0);
        if (at - offset).abs() > OFFSET_EPSILON {
            params.insert("offset".into(), json!(at));
        }
        let c = rgba8(s.color);
        if c != *colour {
            params.insert("color".into(), json!(hex(c)));
        }
        (params.len() > 1).then_some(("attributes.setGradientStopColor", Value::Object(params)))
    })
}

/// The offset in the middle of the widest gap between `stops` (0 and 1 count as ends).
pub fn new_stop_offset(stops: &[(f64, Rgba8)]) -> f64 {
    let mut at: Vec<f64> = stops.iter().map(|(o, _)| *o).filter(|o| o.is_finite()).collect();
    at.push(0.0);
    at.push(1.0);
    at.sort_by(f64::total_cmp);
    at.windows(2)
        .filter_map(|w| Some((*w.first()?, *w.get(1)?)))
        .max_by(|a, b| (a.1 - a.0).total_cmp(&(b.1 - b.0)))
        .map_or(0.5, |(a, b)| (a + b) / 2.0)
}

fn shadow_text(template: &Label, s: &ShadowLine) -> String {
    template
        .as_str()
        .replace("{blur}", &widgets::format_number(s.blur))
        .replace("{x}", &widgets::format_number(s.x))
        .replace("{y}", &widgets::format_number(s.y))
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<AttributesWords>();
    let m: &AttributesModel = props.model;
    let sent = &mut *props.sent;
    let Some(targets) = &m.targets else {
        Text::new(words.none.clone()).shade(Shade::Muted).show(ui);
        return;
    };
    let run = |sent: &mut Vec<Msg>, id: &str, extra: Value| sent.push(Msg::run(id, targets.with(extra)));

    let addable =
        [("gradient", &words.gradient), ("shadow", &words.shadow), ("innerShadow", &words.inner_shadow), ("patternImage", &words.pattern_image)];
    let entries: Vec<Entry<String>> = addable.iter().map(|(n, l)| Entry::new((*n).to_string(), (*l).clone())).collect();
    if let Some(attribute) = Select::new(words.add.clone(), Bind::given(String::new()), &entries).show(ui).events.committed()
        && !attribute.is_empty()
    {
        run(sent, "attributes.addAttribute", json!({ "attribute": attribute }));
    }

    // Stroke: width, alignment, colour, remove.
    match widgets::clearable_number(ui, words.stroke_width.clone(), m.stroke_width) {
        Some(NumberEdit::Set(v)) => run(sent, "attributes.setStrokeWidth", json!({ "width": v.max(0.0) })),
        Some(NumberEdit::Clear) => run(sent, "attributes.setStrokeWidth", json!({ "width": null })),
        None => {}
    }
    let alignments = [("center", &words.centre), ("inside", &words.inside), ("outside", &words.outside)];
    if let Some(word) = segmented(ui, &words.alignment, m.alignment.as_ref(), &alignments) {
        run(sent, "attributes.setStrokeAlignment", json!({ "alignment": word }));
    }
    let _ = Row::new().wrap(true).show(ui, |ui, _| {
        if let Some(c) = color_field(ui, &words.stroke_color, m.stroke_color, "fontcraftAttributes.strokeColor") {
            run(sent, "attributes.setStrokeColor", json!({ "color": hex(c) }));
        }
        if Button::new(words.remove_stroke.clone()).show(ui).clicked() {
            run(sent, "attributes.removeStroke", json!({}));
        }
    });

    // Fill: hidden while a gradient replaces it.
    if !m.any_gradient {
        let _ = Row::new().wrap(true).show(ui, |ui, _| {
            if let Some(c) = color_field(ui, &words.fill_color, m.fill_color, "fontcraftAttributes.fillColor") {
                run(sent, "attributes.setFillColor", json!({ "color": hex(c) }));
            }
            if Button::new(words.remove_fill.clone()).show(ui).clicked() {
                run(sent, "attributes.removeFill", json!({}));
            }
        });
    }

    if m.any_gradient {
        let kinds = [("linear", &words.linear), ("radial", &words.radial)];
        if let Some(kind) = segmented(ui, &words.gradient_type, m.gradient_type.as_ref(), &kinds) {
            run(sent, "attributes.setGradientType", json!({ "type": kind }));
        }
        let bar = GradientEditor::new(words.gradient_bar.clone(), Bind::given(gradient_stops(&m.stops)))
            .rules(StopRules::new(StopRows::Combined).stops(2, MAX_STOPS))
            .midpoints(false)
            .fields(false)
            .color_popover(true)
            .picker(|p| p.alpha(true))
            .show(ui);
        if let Some((id, params)) = bar.edit.events.committed().and_then(|stops| stop_edit(&m.stops, stops)) {
            run(sent, id, params);
        }
        let _ = Row::new().wrap(true).show(ui, |ui, _| {
            if Button::new(words.add_stop.clone()).show(ui).clicked() {
                run(sent, "attributes.addGradientStop", json!({ "offset": new_stop_offset(&m.stops) }));
            }
            if Button::new(words.remove_gradient.clone()).show(ui).clicked() {
                run(sent, "attributes.removeGradient", json!({}));
            }
        });
        for (i, (offset, colour)) in m.stops.iter().enumerate() {
            let _ = Row::new().wrap(true).id_salt(("fontcraftAttributes.stop", i)).show(ui, |ui, _| {
                if let Some(v) = widgets::number(ui, words.stop_offset.clone(), Some(*offset)) {
                    run(sent, "attributes.setGradientStopColor", json!({ "stop": i, "offset": v.clamp(0.0, 1.0) }));
                }
                if let Some(c) = color_field(ui, &words.stop_color, Some(*colour), ("fontcraftAttributes.stopColor", i)) {
                    run(sent, "attributes.setGradientStopColor", json!({ "stop": i, "color": hex(c) }));
                }
                if m.stops.len() > 2 && Button::new(words.remove_stop.clone()).show(ui).clicked() {
                    run(sent, "attributes.removeGradientStop", json!({ "stop": i }));
                }
            });
        }
    }

    // Shadows: stored and drawn by image export only; listed so the person knows they are there.
    for (template, shadow) in [(&words.shadow_line, &m.shadow), (&words.inner_shadow_line, &m.inner_shadow)] {
        if let Some(s) = shadow {
            Text::new(Label::from(shadow_text(template, s))).shade(Shade::Muted).show(ui);
        }
    }
}

/// A segmented row named `label` over `current` (`None`: mixed, no segment on); the value picked.
fn segmented(ui: &mut Ui, label: &Label, current: Option<&String>, options: &[(&str, &Label)]) -> Option<String> {
    let entries: Vec<Entry<String>> = options.iter().map(|(name, l)| Entry::new((*name).to_string(), (*l).clone())).collect();
    let was = current.cloned().unwrap_or_default();
    let mut value = was.clone();
    Segmented::new(label.clone(), Bind::bound(&mut value), &entries).show(ui);
    (value != was).then_some(value)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::contract::Tracked;
    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};
    use serde_json::json;

    use super::{Props, gradient_stops, new_stop_offset, rgba8, show, srgba, stop_edit};
    use crate::app::Msg;
    use crate::app::models::color::AttributesModel;
    use crate::app::models::palette::Targets;

    fn fixture(model: AttributesModel) -> (Fixture, Rc<RefCell<Vec<Msg>>>) {
        let model = Tracked::new(model);
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        (fixture, posted)
    }

    fn targets() -> Option<Targets> {
        Some(Targets { json: json!({"glyph": "A", "layer": "color", "shapes": [0]}).to_string() })
    }

    /// A typed fill colour and Remove Stroke run their commands on the selected paths.
    #[test]
    fn the_fill_colour_and_remove_stroke_run_the_attributes_commands() {
        let (mut fixture, posted) = fixture(AttributesModel { targets: targets(), fill_color: Some([128, 128, 128, 255]), ..Default::default() });
        fixture.settle();
        let fill = fixture.by_label("Fill Colour").expect("the fill colour field");
        fixture.click(&fill).key(Key::A, Modifiers::COMMAND).type_text("#FF000080").key(Key::Enter, Modifiers::NONE).settle();
        let remove = fixture.by_label("Remove Stroke").expect("the Remove Stroke button");
        fixture.click(&remove).settle();
        assert_eq!(
            *posted.borrow(),
            vec![
                Msg::run("attributes.setFillColor", json!({"glyph": "A", "layer": "color", "shapes": [0], "color": "#FF000080"})),
                Msg::run("attributes.removeStroke", json!({"glyph": "A", "layer": "color", "shapes": [0]})),
            ]
        );
    }

    /// With a gradient the fill row hides, and Add Stop adds one in the widest gap.
    #[test]
    fn a_gradient_hides_the_fill_and_adds_stops_in_the_widest_gap() {
        let stops = vec![(0.0, [255, 0, 0, 255]), (0.25, [0, 255, 0, 255]), (1.0, [0, 0, 255, 255])];
        assert!((new_stop_offset(&stops) - 0.625).abs() < 1e-9);
        let (mut fixture, posted) =
            fixture(AttributesModel { targets: targets(), any_gradient: true, gradient_type: Some("linear".into()), stops, ..Default::default() });
        fixture.settle();
        assert!(fixture.by_label("Fill Colour").is_none(), "the gradient replaces the fill row");
        let add = fixture.by_label("Add Stop").expect("the Add Stop button");
        fixture.click(&add).settle();
        assert_eq!(
            *posted.borrow(),
            vec![Msg::run("attributes.addGradientStop", json!({"glyph": "A", "layer": "color", "shapes": [0], "offset": 0.625}))]
        );
    }

    /// Each edit of the gradient bar becomes one stop command: an added stop with its offset and colour, a removed
    /// stop, a dragged or recoloured stop by its index.
    #[test]
    fn gradient_bar_edits_become_stop_commands() {
        let old = vec![(0.0, [255, 0, 0, 255]), (1.0, [0, 0, 255, 128])];
        let bar = gradient_stops(&old);
        assert_eq!(bar.color.iter().map(|s| s.tag).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(rgba8(bar.color[1].color), [0, 0, 255, 128], "the opacity rides in the colour");
        assert_eq!(stop_edit(&old, &bar), None, "nothing changed");

        let mut added = bar.clone();
        added.color.insert(1, craft::color_ui::ui::gradient::ColorStop::new(0.5, srgba([0, 255, 0, 255])));
        assert_eq!(stop_edit(&old, &added), Some(("attributes.addGradientStop", json!({"offset": 0.5, "color": "#00FF00FF"}))));

        let three = vec![(0.0, [255, 0, 0, 255]), (0.5, [0, 255, 0, 255]), (1.0, [0, 0, 255, 255])];
        let mut removed = gradient_stops(&three);
        removed.color.remove(1);
        assert_eq!(stop_edit(&three, &removed), Some(("attributes.removeGradientStop", json!({"stop": 1}))));

        let mut moved = bar.clone();
        moved.color[0].location = 0.25;
        moved.color[0].color = srgba([255, 255, 0, 255]);
        assert_eq!(stop_edit(&old, &moved), Some(("attributes.setGradientStopColor", json!({"stop": 0, "offset": 0.25, "color": "#FFFF00FF"}))));
    }
}
