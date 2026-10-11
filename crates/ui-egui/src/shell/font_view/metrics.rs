//! The metric fields of the glyph inspector (`fontView.inspector.metricsFields`) and the switch
//! between their two sets (`fontView.toggleMetricsOrientation`).
//!
//! | Set | Fields | Editing runs |
//! |---|---|---|
//! | horizontal | LSB, RSB, Width | `spacing.setLeftSidebearing`, `spacing.setRightSidebearing`, `spacing.setWidth` |
//! | vertical | TSB, BSB | `spacing.setTopSidebearing`, `spacing.setBottomSidebearing` |
//! | vertical | vertical origin, vertical width | read-only: no command sets them yet |
//!
//! Each field reads the glyph's layer in Font View's master. It shows the metrics key that
//! governs the metric (the layer's own key first, written with `==`), else the measured value. A
//! typed number or key goes to every selected glyph in one command, on that master's layer.

use fontcraft_model::{Font, Glyph, Layer, metric};
use fontcraft_spacing::{self as spacing, Field, Scope};
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::panels::widgets::format_number;
use crate::shell::{Args, UiCommand};

pub const COMMANDS: &[UiCommand] = &[(
    "fontView.toggleMetricsOrientation",
    "Horizontal / Vertical Metrics",
    &[],
    "",
    "{orientation?: \"horizontal\" | \"vertical\" (omitted: switch)} show the horizontal metric fields of the glyph inspector (LSB, RSB, width) or the vertical ones (TSB, BSB, vertical origin, vertical width) → {orientation}",
)];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "fontView.toggleMetricsOrientation" => {
            let panels = &mut app.state.panels;
            let vertical = match args.get("orientation") {
                None => !panels.inspector_vertical,
                Some(Value::String(s)) if s == "horizontal" => false,
                Some(Value::String(s)) if s == "vertical" => true,
                Some(_) => return Err(args.err("`orientation` must be \"horizontal\" or \"vertical\"")),
            };
            panels.inspector_vertical = vertical;
            Ok(json!({ "orientation": orientation(vertical) }))
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

fn orientation(vertical: bool) -> &'static str {
    if vertical { "vertical" } else { "horizontal" }
}

/// One metric field of the inspector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MetricField {
    Lsb,
    Rsb,
    Width,
    Tsb,
    Bsb,
    VertOrigin,
    VertWidth,
}

/// The fields of each set, in the order the inspector lays them out.
pub const HORIZONTAL: [MetricField; 3] = [MetricField::Lsb, MetricField::Rsb, MetricField::Width];
pub const VERTICAL: [MetricField; 4] = [MetricField::Tsb, MetricField::Bsb, MetricField::VertOrigin, MetricField::VertWidth];

/// The fields the inspector shows in one orientation.
pub fn fields(vertical: bool) -> &'static [MetricField] {
    if vertical { &VERTICAL } else { &HORIZONTAL }
}

impl MetricField {
    /// The salt part of the field's control id. The horizontal ones equal the List View column
    /// ids, so a field keeps its id across both.
    pub fn id(self) -> &'static str {
        match self {
            MetricField::Lsb => "lsb",
            MetricField::Rsb => "rsb",
            MetricField::Width => "width",
            MetricField::Tsb => "tsb",
            MetricField::Bsb => "bsb",
            MetricField::VertOrigin => "vertOrigin",
            MetricField::VertWidth => "vertWidth",
        }
    }

    /// The field's tooltip.
    pub fn label(self) -> &'static str {
        match self {
            MetricField::Lsb => "LSB",
            MetricField::Rsb => "RSB",
            MetricField::Width => "Width",
            MetricField::Tsb => "TSB",
            MetricField::Bsb => "BSB",
            MetricField::VertOrigin => "Vertical Origin",
            MetricField::VertWidth => "Vertical Width",
        }
    }

    /// The metric a spacing command sets; `None` for the two fields no command sets.
    pub fn field(self) -> Option<Field> {
        match self {
            MetricField::Lsb => Some(Field::Left),
            MetricField::Rsb => Some(Field::Right),
            MetricField::Width => Some(Field::Width),
            MetricField::Tsb => Some(Field::Top),
            MetricField::Bsb => Some(Field::Bottom),
            MetricField::VertOrigin | MetricField::VertWidth => None,
        }
    }

    /// The command that sets this field.
    pub fn command(self) -> Option<&'static str> {
        Some(match self.field()? {
            Field::Left => "spacing.setLeftSidebearing",
            Field::Right => "spacing.setRightSidebearing",
            Field::Width => "spacing.setWidth",
            Field::Top => "spacing.setTopSidebearing",
            Field::Bottom => "spacing.setBottomSidebearing",
        })
    }

    /// Whether the field can be edited.
    pub fn editable(self) -> bool {
        self.field().is_some()
    }

    /// What the field shows for `glyph` in `master`: its key, else its value (empty for a
    /// sidebearing of an empty layer and for a glyph without that master's layer).
    pub fn text(self, font: &Font, glyph: &Glyph, master: &str) -> String {
        let Some(layer) = glyph.layer(master) else { return String::new() };
        match self.field() {
            Some(field) => match spacing::effective_key(glyph, layer, field) {
                Some((scope, stored)) => key_text(font, field, scope, stored),
                None => spacing::metric(font, layer, field).ok().flatten().filter(|n| n.is_finite()).map(format_number).unwrap_or_default(),
            },
            None => self.vertical_text(font, glyph, layer),
        }
    }

    fn vertical_text(self, font: &Font, glyph: &Glyph, layer: &Layer) -> String {
        let key = |keys: &fontcraft_model::MetricsKeys| {
            let k = if self == MetricField::VertOrigin { keys.vert_origin.as_deref() } else { keys.vert_width.as_deref() };
            k.map(str::trim).filter(|t| !t.is_empty()).map(str::to_string)
        };
        if let Some(k) = key(&layer.info.metrics_keys) {
            return format!("=={}", k.trim_start_matches('='));
        }
        if let Some(k) = key(&glyph.info.metrics_keys) {
            return k;
        }
        let value = match self {
            MetricField::VertOrigin => Some(layer.info.vert_origin.unwrap_or(0.0)),
            _ => layer.info.vert_width.or_else(|| default_vertical_width(font, &layer.master)),
        };
        value.filter(|n| n.is_finite()).map(format_number).unwrap_or_default()
    }
}

/// A stored key as the field shows it: `==` for a key of the layer only.
fn key_text(font: &Font, field: Field, scope: Scope, stored: &str) -> String {
    match spacing::parse_stored(stored, scope, field, |n| font.glyph(n).is_some()) {
        Ok(key) => key.display(),
        Err(_) => match scope {
            Scope::Local => format!("=={}", stored.trim().trim_start_matches('=')),
            Scope::Glyph => stored.trim().to_string(),
        },
    }
}

/// The vertical width of a layer that stores none: the master's ascender to its descender.
fn default_vertical_width(font: &Font, master: &str) -> Option<f64> {
    let upm = f64::from(font.units_per_em);
    let m = font.masters.iter().find(|m| m.id == master)?;
    Some(font.metric_pos(m, metric::ASCENDER, upm * 0.8) - font.metric_pos(m, metric::DESCENDER, -upm * 0.2))
}

/// The command a typed `text` runs on `glyphs` in `master`'s layer; `None` for an empty text or
/// a read-only field.
pub fn edit_command(field: MetricField, glyphs: &[String], master: &str, text: &str) -> Option<(&'static str, Value)> {
    let text = text.trim();
    if text.is_empty() || glyphs.is_empty() {
        return None;
    }
    let id = field.command()?;
    Some((id, json!({ "glyphs": glyphs, "layer": master, "value": text })))
}
