//! The controls the panels share: number fields that commit a value or an emptied field, the key-or-number field of
//! the metrics fields, and the nine-point grid of a selection's box. Each is a craft component with the post rules of
//! `docs/ui.md`: it reports what was committed (Enter, Tab or leaving the field), never a keystroke, and a field left
//! with its text unchanged commits nothing, so tabbing through fields adds no undo steps.
//!
//! The Info box, the Palette sections, the Font View inspector and Font Info use these instead of building their own
//! fields.

use craft::behavior::{Anchor9, IconId};
use craft::ui::buttons::IconButton;
use craft::ui::contract::{Bind, PlainData, Props as _, Show as _, ShowContainer as _};
use craft::ui::layout_components::Row;
use craft::ui::pickers::ReferencePoint;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::{NumberField, TextField};
use egui::Ui;
use serde_json::{Value, json};

craft::ui::words! {
    /// The words of the shared controls.
    pub struct WidgetWords {
        /// The button beside a metrics field whose key is out of sync.
        update_key: "fontcraftWidgets.updateKey" = "Update Metrics Key";
    }
}

/// The decimals a number shows.
const DECIMALS: i32 = 2;
/// Longest text a key-or-number field takes.
pub const MAX_KEY_CHARS: usize = 256;

/// The nine points of a box, row by row from the top, as the engine's commands name them.
pub const BOX_POINTS: [&str; 9] = ["topLeft", "top", "topRight", "left", "center", "right", "bottomLeft", "bottom", "bottomRight"];

/// `n` as a field shows it: at most two decimals, no trailing zeros, no negative zero.
pub fn format_number(n: f64) -> String {
    if !n.is_finite() {
        return String::new();
    }
    let scale = 10f64.powi(DECIMALS);
    let rounded = (n * scale).round() / scale;
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    let text = format!("{rounded:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A number typed into a text field; `None` for anything that is not a finite number.
pub fn parse_number(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|n| n.is_finite())
}

/// What the user did to a number field that may be emptied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NumberEdit {
    Set(f64),
    /// The field held a value and was emptied.
    Clear,
}

/// A number field named `label` over `value`; the number committed. An emptied field commits nothing.
pub fn number(ui: &mut Ui, label: Label, value: Option<f64>) -> Option<f64> {
    match clearable(ui, NumberField::new(label, Bind::given(value)), value) {
        Some(NumberEdit::Set(n)) => Some(n),
        _ => None,
    }
}

/// A number field named `label` over `value` that reports an emptied field as [`NumberEdit::Clear`].
pub fn clearable_number(ui: &mut Ui, label: Label, value: Option<f64>) -> Option<NumberEdit> {
    clearable(ui, NumberField::new(label, Bind::given(value)), value)
}

/// Shows `field` (a number field over `value`, configured by the caller) and reads its commit.
pub fn clearable(ui: &mut Ui, field: NumberField<'_>, value: Option<f64>) -> Option<NumberEdit> {
    let out = field.show(ui);
    match out.edit.events.committed()? {
        Some(n) if n.is_finite() && Some(*n) != value => Some(NumberEdit::Set(*n)),
        None if value.is_some() => Some(NumberEdit::Clear),
        _ => None,
    }
}

/// What a metrics field's key is doing.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyState {
    /// No key: the field holds a number.
    #[default]
    Number,
    InSync,
    /// The key's value differs from the metric.
    OutOfSync,
    /// The key names a glyph the font does not have.
    Unlinked,
    /// The key cannot be evaluated (a cycle, a division by zero, a result out of range).
    Error,
    /// Automatic alignment sets the metric; the value is the derived one and the key the offset key.
    Auto,
}

/// A metric that holds a number or a metrics key, as a key-or-number field shows it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct KeyedNumber {
    /// The metric as it is; `None` for a sidebearing of an empty layer.
    pub value: Option<f64>,
    /// The metrics key that applies, as a field shows it (`=H`, `==40`, `=+18`).
    pub key: Option<String>,
    pub state: KeyState,
    /// Why the key is out of sync, unlinked or wrong, or what it gives; the field's tooltip.
    pub message: Option<String>,
}

impl KeyedNumber {
    /// The text the field shows: the key, else the number; `auto (value)` or `key (value)` for a derived metric.
    pub fn shown(&self) -> String {
        if self.state == KeyState::Auto {
            let value = self.value.map_or_else(String::new, format_number);
            return format!("{} ({value})", self.key.as_deref().unwrap_or("auto"));
        }
        match (&self.key, self.value) {
            (Some(key), _) => key.clone(),
            (None, Some(v)) => format_number(v),
            (None, None) => String::new(),
        }
    }

    /// Whether the field takes typing: a sidebearing of an empty layer without a key does not.
    pub fn editable(&self) -> bool {
        self.value.is_some() || self.key.is_some() || self.state == KeyState::Auto
    }

    /// The `value` parameter a committed `text` sets: a number, a key, the metric's current number for an emptied
    /// field that held a key (which removes the key), or `=+0` for an emptied derived offset. `None`: nothing to set.
    pub fn commit_value(&self, text: &str) -> Option<Value> {
        let t = text.trim();
        if t.is_empty() && self.state == KeyState::Auto {
            return self.key.as_ref().map(|_| json!("=+0"));
        }
        if t.is_empty() {
            return self.key.as_ref().and(self.value).map(|v| json!(v));
        }
        match parse_number(t) {
            Some(n) => Some(json!(n)),
            None => Some(json!(t)),
        }
    }
}

/// What a key-or-number field reports.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyedOutput {
    /// The `value` parameter for the field's setter.
    pub commit: Option<Value>,
    /// The update button beside an out-of-sync key was pressed.
    pub update: bool,
}

/// A field named `label` that holds a number or a metrics key (`docs/spec/03b` › Info box). Its tooltip says what the
/// key gives or why it is out of sync; an out-of-sync key gets an update button beside it. A read-only field (a linked
/// master's metrics) shows the text and commits nothing.
pub fn key_or_number(ui: &mut Ui, label: Label, metric: &KeyedNumber, read_only: bool) -> KeyedOutput {
    let words = Env::of(ui).words::<WidgetWords>();
    let shown = metric.shown();
    let mut out = KeyedOutput::default();
    let _ = Row::new().show(ui, |ui, _| {
        let mut field = TextField::new(label.clone(), Bind::given(shown.clone()))
            .select_on_focus()
            .limit(MAX_KEY_CHARS)
            .read_only(read_only || !metric.editable());
        if let Some(message) = &metric.message {
            field = field.tooltip(message.clone());
        }
        let edit = field.show(ui);
        if let Some(text) = edit.events.committed()
            && *text != shown
        {
            out.commit = metric.commit_value(text);
        }
        if metric.state == KeyState::OutOfSync && !read_only {
            let update_label = format!("{} {}", words.update_key.as_str(), label.as_str());
            out.update = IconButton::new(IconId::new("refresh")).label(update_label).tooltip(words.update_key.clone()).show(ui).clicked();
        }
    });
    out
}

/// The nine-point grid named `label` over the box point `point` (a [`BOX_POINTS`] name; `None` picks no point). Returns
/// the point picked.
pub fn box_point(ui: &mut Ui, label: Label, point: Option<&str>) -> Option<&'static str> {
    let index = point.and_then(|p| BOX_POINTS.iter().position(|n| *n == p));
    let current = index.and_then(|i| Anchor9::ALL.get(i).copied());
    let shown = current.unwrap_or(Anchor9::Centre);
    let edit = ReferencePoint::new(label, Bind::given(shown)).show(ui);
    let picked = *edit.events.committed()?;
    if Some(picked) == current {
        return None;
    }
    Anchor9::ALL.iter().position(|a| *a == picked).and_then(|i| BOX_POINTS.get(i).copied())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{KeyState, KeyedNumber, format_number};

    #[test]
    fn numbers_show_at_most_two_decimals() {
        assert_eq!(format_number(12.0), "12");
        assert_eq!(format_number(-0.001), "0");
        assert_eq!(format_number(1.256), "1.26");
    }

    /// Emptying a field that holds a key stores the current number; a derived offset goes back to `=+0`.
    #[test]
    fn an_emptied_key_field_removes_the_key() {
        let keyed = KeyedNumber { value: Some(50.0), key: Some("=H".into()), state: KeyState::InSync, message: None };
        assert_eq!(keyed.commit_value(" "), Some(json!(50.0)));
        assert_eq!(keyed.commit_value("=O"), Some(json!("=O")));
        assert_eq!(keyed.commit_value("60"), Some(json!(60.0)));
        let derived = KeyedNumber { value: Some(68.0), key: Some("=+18".into()), state: KeyState::Auto, message: None };
        assert_eq!(derived.shown(), "=+18 (68)");
        assert_eq!(derived.commit_value(""), Some(json!("=+0")));
        let plain = KeyedNumber { value: Some(10.0), ..Default::default() };
        assert_eq!(plain.commit_value(""), None);
    }
}
