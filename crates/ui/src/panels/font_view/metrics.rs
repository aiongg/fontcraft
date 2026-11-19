//! The inspector's metric fields (`fontView.inspector.metricsFields`) and the switch between their two sets
//! (`fontView.toggleMetricsOrientation`).
//!
//! | Set | Fields | Editing runs |
//! |---|---|---|
//! | horizontal | LSB, RSB, Width | `spacing.setLeftSidebearing`, `spacing.setRightSidebearing`, `spacing.setWidth` |
//! | vertical | TSB, BSB | `spacing.setTopSidebearing`, `spacing.setBottomSidebearing` |
//! | vertical | vertical origin (O), vertical width (H) | `spacing.setVerticalOrigin`, `spacing.setVerticalWidth` |
//!
//! Each field shows the metrics key that governs the metric, else the measured value, in Font View's master. A typed
//! number or key goes to every selected glyph in one command, on that master's layer: `{glyphs, layer, value}`.

use craft::behavior::IconId;
use craft::ui::buttons::IconButton;
use craft::ui::contract::{Props as _, Show as _, ShowContainer as _};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use egui::Ui;
use serde_json::json;

use super::inspector::text_field;
use crate::app::Msg;
use crate::app::models::glyph_props::{Edit, GlyphPropsModel, MetricField, Shown, metric_fields};

craft::ui::words! {
    /// The words of the metric fields.
    pub struct MetricWords {
        metrics: "fontcraftInspectorMetrics.metrics" = "Metrics";
        lsb: "fontcraftInspectorMetrics.lsb" = "LSB";
        rsb: "fontcraftInspectorMetrics.rsb" = "RSB";
        width: "fontcraftInspectorMetrics.width" = "Width";
        tsb: "fontcraftInspectorMetrics.tsb" = "TSB";
        bsb: "fontcraftInspectorMetrics.bsb" = "BSB";
        vert_origin: "fontcraftInspectorMetrics.vertOrigin" = "Vertical Origin";
        vert_width: "fontcraftInspectorMetrics.vertWidth" = "Vertical Width";
        /// The orientation switch, by the set it shows next.
        show_vertical: "fontcraftInspectorMetrics.showVertical" = "Show Vertical Metrics";
        show_horizontal: "fontcraftInspectorMetrics.showHorizontal" = "Show Horizontal Metrics";
    }
}

/// The field's label.
pub fn label(words: &MetricWords, field: MetricField) -> Label {
    match field {
        MetricField::Lsb => words.lsb.clone(),
        MetricField::Rsb => words.rsb.clone(),
        MetricField::Width => words.width.clone(),
        MetricField::Tsb => words.tsb.clone(),
        MetricField::Bsb => words.bsb.clone(),
        MetricField::VertOrigin => words.vert_origin.clone(),
        MetricField::VertWidth => words.vert_width.clone(),
    }
}

/// The Metrics rows: the fields of one orientation, with the switch after the first field. Horizontal: one row of
/// LSB, RSB and width. Vertical: TSB and BSB, then vertical origin and vertical width on a second row.
pub fn show(ui: &mut Ui, model: &GlyphPropsModel, enabled: bool, multiple: &Label, edits: &mut Vec<Edit>, shell: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<MetricWords>();
    let fields = metric_fields(model.vertical);
    let split = if model.vertical { 2 } else { fields.len() };
    let (first, second) = fields.split_at(split.min(fields.len()));
    let value = |at: usize| model.metrics.get(at).cloned().unwrap_or_default();
    let field = |ui: &mut Ui, f: MetricField, at: usize, edits: &mut Vec<Edit>| {
        if let Some(text) = text_field(ui, label(&words, f), &value(at), enabled, multiple) {
            edits.push(Edit::Metric(f, text));
        }
    };
    Field::new(words.metrics.clone()).show(ui, |ui, _| {
        Row::new().id_salt("first").show(ui, |ui, _| {
            for (at, &f) in first.iter().enumerate() {
                field(ui, f, at, edits);
                if at == 0 {
                    let (icon, tip, next) = if model.vertical {
                        ("arrow-up-down", words.show_horizontal.clone(), "horizontal")
                    } else {
                        ("arrow-left-right", words.show_vertical.clone(), "vertical")
                    };
                    if IconButton::new(IconId::new(icon)).label(tip.clone()).tooltip(tip).show(ui).clicked() {
                        shell.push(Msg::Shell(CommandRef::with_params("fontView.toggleMetricsOrientation", json!({ "orientation": next }))));
                    }
                }
            }
        });
        if !second.is_empty() {
            Row::new().id_salt("second").show(ui, |ui, _| {
                for (at, &f) in second.iter().enumerate() {
                    field(ui, f, first.len() + at, edits);
                }
            });
        }
    });
}

/// What `field` shows in `model`; empty for a field of the other orientation.
pub fn shown(model: &GlyphPropsModel, field: MetricField) -> Shown {
    let at = metric_fields(model.vertical).iter().position(|f| *f == field);
    at.and_then(|at| model.metrics.get(at)).cloned().unwrap_or_default()
}
