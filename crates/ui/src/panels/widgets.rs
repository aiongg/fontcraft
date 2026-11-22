//! The controls the panels share: number fields that commit a value or an emptied field, the key-or-number field of
//! the metrics fields, and the nine-point grid of a selection's box. Each is a craft component with the post rules of
//! `docs/ui.md`: it reports what was committed (Enter, Tab or leaving the field), never a keystroke, and a field left
//! with its text unchanged commits nothing, so tabbing through fields adds no undo steps.
//!
//! The Info box, the Palette sections, the Font View inspector and Font Info use these instead of building their own
//! fields. Every palette panel draws its body through [`scrolling`].

use craft::behavior::{Anchor9, IconId};
use craft::ui::buttons::IconButton;
use craft::ui::contract::{Bind, PlainData, Props as _, Scroll, Show as _, ShowContainer as _};
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::pickers::ReferencePoint;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::{NumberField, TextField};
use craft_ext::ui::text::{ArrowSteps, StepKeys as _};
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

/// A panel body that scrolls vertically: the palette gives each of its panels a fixed height, which a long
/// selection, a guide's fields or a tall form outgrows.
pub fn scrolling(ui: &mut Ui, mut body: impl FnMut(&mut Ui)) {
    Stack::new().scroll(Scroll::vertical()).fill().show(ui, |ui, _| body(ui));
}
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

/// Longest arithmetic expression a number field evaluates, in bytes.
pub const MAX_EXPRESSION_BYTES: usize = 256;
/// Deepest nesting of parentheses and signs in front of a number an expression may have.
pub const MAX_EXPRESSION_DEPTH: usize = 32;

/// A number typed into a text field: a number, or an arithmetic expression of numbers with `+ - * /` and parentheses
/// (`300+50`, `(500-20)/2`), worth its result. `None` for anything else, for an expression longer than
/// [`MAX_EXPRESSION_BYTES`] or nested deeper than [`MAX_EXPRESSION_DEPTH`], and for a result that is not finite
/// (a division by zero).
pub fn parse_number(text: &str) -> Option<f64> {
    let text = text.trim();
    if let Ok(n) = text.parse::<f64>() {
        return n.is_finite().then_some(n);
    }
    if text.is_empty() || text.len() > MAX_EXPRESSION_BYTES || expression_depth(text) > MAX_EXPRESSION_DEPTH {
        return None;
    }
    craft::behavior::Reader::new().read(text, None).ok().filter(|n| n.is_finite())
}

/// How deep `text` nests: open parentheses plus the run of signs in front of a number. The evaluator goes one level
/// deeper for each, so this bounds its recursion.
fn expression_depth(text: &str) -> usize {
    let (mut open, mut signs, mut deepest) = (0usize, 0usize, 0usize);
    let mut after_operand = false;
    for c in text.chars().filter(|c| !c.is_whitespace()) {
        match c {
            '(' => {
                open += 1;
                signs = 0;
                after_operand = false;
            }
            ')' => {
                open = open.saturating_sub(1);
                after_operand = true;
            }
            '+' | '-' if !after_operand => signs += 1,
            '+' | '-' | '*' | '/' => {
                signs = 0;
                after_operand = false;
            }
            _ => {
                signs = 0;
                after_operand = true;
            }
        }
        deepest = deepest.max(open + signs);
    }
    deepest
}

/// What the user did to a number field that may be emptied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NumberEdit {
    Set(f64),
    /// The field held a value and was emptied.
    Clear,
}

/// `body` with `label` shown beside it. A control's own label only names it to assistive technology; a panel field
/// shows its name on screen through this.
pub fn labelled<R>(ui: &mut Ui, label: &Label, mut body: impl FnMut(&mut Ui) -> R) -> Option<R> {
    Field::new(label.clone()).show(ui, |ui, _| body(ui)).inner
}

/// A number field named `label` over `value`, its label shown beside it; the number committed. An emptied field
/// commits nothing.
pub fn number(ui: &mut Ui, label: Label, value: Option<f64>) -> Option<f64> {
    labelled(ui, &label.clone(), |ui| bare_number(ui, label.clone(), value)).flatten()
}

/// [`number`] without the visible label: for a bar whose neighbours already say what the field holds.
pub fn bare_number(ui: &mut Ui, label: Label, value: Option<f64>) -> Option<f64> {
    match clearable(ui, NumberField::new(label, Bind::given(value)), value) {
        Some(NumberEdit::Set(n)) => Some(n),
        _ => None,
    }
}

/// A number field named `label` over `value`, its label shown beside it, that reports an emptied field as
/// [`NumberEdit::Clear`].
pub fn clearable_number(ui: &mut Ui, label: Label, value: Option<f64>) -> Option<NumberEdit> {
    labelled(ui, &label.clone(), |ui| clearable(ui, NumberField::new(label.clone(), Bind::given(value)), value)).flatten()
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
/// key gives or why it is out of sync; an out-of-sync key gets an update button beside it. While it holds a number, Up
/// and Down commit the number stepped. A read-only field (a linked
/// master's metrics) shows the text and commits nothing.
pub fn key_or_number(ui: &mut Ui, label: Label, metric: &KeyedNumber, read_only: bool) -> KeyedOutput {
    let words = Env::of(ui).words::<WidgetWords>();
    let shown = metric.shown();
    let mut out = KeyedOutput::default();
    let _ = labelled(ui, &label, |ui| {
        Row::new().show(ui, |ui, _| {
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
            // Up and Down step a metric that holds a number: 1, 10 with Shift, 100 with Cmd+Shift.
            if !read_only
                && metric.state == KeyState::Number
                && let Some(delta) = edit.arrow_steps(ui, ArrowSteps::UNITS)
                && let Some(value) = metric.value
            {
                out.commit = Some(json!(value + delta));
            }
            if metric.state == KeyState::OutOfSync && !read_only {
                let update_label = format!("{} {}", words.update_key.as_str(), label.as_str());
                out.update = IconButton::new(IconId::new("refresh")).label(update_label).tooltip(words.update_key.clone()).show(ui).clicked();
            }
        })
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

    use super::{KeyState, KeyedNumber, Label, MAX_EXPRESSION_DEPTH, format_number, key_or_number, parse_number};

    #[test]
    fn number_fields_take_arithmetic_within_their_limits() {
        assert_eq!(parse_number("300+50"), Some(350.0));
        assert_eq!(parse_number(" 300 * 1.2 "), Some(360.0));
        assert_eq!(parse_number("(500-20)/2"), Some(240.0));
        assert_eq!(parse_number("-(-4)"), Some(4.0));
        assert_eq!(parse_number("1e3"), Some(1_000.0), "a plain number still reads as before");
        for bad in ["", "1/0", "0/0", "2*", "(1", "H", "=H+10", "1e400"] {
            assert_eq!(parse_number(bad), None, "{bad}");
        }
        let deep = format!("{}1{}", "(".repeat(MAX_EXPRESSION_DEPTH + 1), ")".repeat(MAX_EXPRESSION_DEPTH + 1));
        assert_eq!(parse_number(&deep), None, "too deep");
        let shallow = format!("{}1{}", "(".repeat(MAX_EXPRESSION_DEPTH), ")".repeat(MAX_EXPRESSION_DEPTH));
        assert_eq!(parse_number(&shallow), Some(1.0));
        assert_eq!(parse_number(&"-".repeat(10_000)), None, "a long run of signs is refused before it is read");
        assert_eq!(parse_number(&format!("1{}", "+1".repeat(200))), None, "longer than the limit");
    }

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

    /// Up and Down step a metric that holds a number and commit it; a keyed metric does not step.
    #[test]
    fn arrows_step_a_numeric_metric() {
        use std::cell::RefCell;
        use std::rc::Rc;

        use craft::ui::testing::Fixture;
        use egui::{Key, Modifiers};

        let commits = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&commits);
        let mut fixture = Fixture::new(move |ui| {
            let plain = KeyedNumber { value: Some(40.0), ..Default::default() };
            let keyed = KeyedNumber { value: Some(50.0), key: Some("=H".into()), state: KeyState::InSync, message: None };
            for (label, metric) in [("LSB", &plain), ("RSB", &keyed)] {
                if let Some(commit) = key_or_number(ui, Label::from(label), metric, false).commit {
                    seen.borrow_mut().push((label, commit));
                }
            }
        });
        fixture.settle();
        let lsb = fixture.by_label("LSB").expect("the LSB field");
        fixture.click(&lsb).key(Key::ArrowUp, Modifiers::SHIFT).settle();
        let rsb = fixture.by_label("RSB").expect("the RSB field");
        fixture.click(&rsb).key(Key::ArrowUp, Modifiers::NONE).settle();
        assert_eq!(*commits.borrow(), vec![("LSB", json!(50.0))]);
    }

    /// Craft's number fields read every draft through `NumberCodec`, whose expression reader recurses once per
    /// parenthesis and sign with no depth limit, and the field sets no length limit on its draft, so a long enough
    /// paste of `(` overflows the stack. Craft offers no hook before the read; this stays ignored until craft bounds
    /// it, and aborts the test binary when run.
    #[test]
    #[ignore = "craft bug: aborts with a stack overflow until craft bounds its expression reader"]
    fn craft_number_fields_survive_a_paste_of_deep_parentheses() {
        use craft::behavior::Codec as _;
        let text = "(".repeat(100_000);
        let read = std::thread::Builder::new()
            .stack_size(8 << 20)
            .spawn(move || craft::behavior::NumberCodec::default().parse(&text, &None).is_ok())
            .unwrap()
            .join()
            .unwrap();
        assert!(!read);
    }
}
