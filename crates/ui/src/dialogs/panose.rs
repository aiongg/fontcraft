//! The PANOSE dialog: ten menus, one per PANOSE digit. OK posts one `font.customParameters.set` with the ten digits;
//! Cancel posts nothing.
//!
//! The menus name the values of family kind 2 (Latin Text), as the PANOSE classification names them; for another
//! family kind the digits after the first are plain numbers. The value names are kept as data, not as words.

use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::form::Field;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label};
use egui::Ui;
use serde_json::{Value, json};

use super::os2_bits::{numbers, stored_value};
use crate::app::{FontCraftApp, Msg};
use crate::shell::font_info::{InfoDialog, Target};

craft::ui::words! {
    /// The words of the PANOSE dialog.
    pub struct PanoseWords {
        title: "fontcraftPanose.title" = "PANOSE";
        family_kind: "fontcraftPanose.familyKind" = "Family Kind";
        serif_style: "fontcraftPanose.serifStyle" = "Serif Style";
        weight: "fontcraftPanose.weight" = "Weight";
        proportion: "fontcraftPanose.proportion" = "Proportion";
        contrast: "fontcraftPanose.contrast" = "Contrast";
        stroke_variation: "fontcraftPanose.strokeVariation" = "Stroke Variation";
        arm_style: "fontcraftPanose.armStyle" = "Arm Style";
        letterform: "fontcraftPanose.letterform" = "Letterform";
        midline: "fontcraftPanose.midline" = "Midline";
        x_height: "fontcraftPanose.xHeight" = "X-height";
        /// The label of a digit whose category the family kind does not name: "Digit" and its number.
        digit: "fontcraftPanose.digit" = "Digit";
    }
}

/// Digits in a PANOSE value.
pub const DIGITS: usize = 10;
/// Values a digit takes: 0 to 15.
const VALUES: i64 = 16;
/// The family kind whose digits the menus name.
const LATIN_TEXT: i64 = 2;

const ANY: [&str; 2] = ["Any", "No Fit"];

/// The value names of each digit of family kind 2 (Latin Text), from 2 on; 0 and 1 are Any and No Fit.
const LATIN_TEXT_NAMES: [&[&str]; DIGITS] = [
    &["Latin Text", "Latin Hand Written", "Latin Decorative", "Latin Symbol"],
    &[
        "Cove",
        "Obtuse Cove",
        "Square Cove",
        "Obtuse Square Cove",
        "Square",
        "Thin",
        "Oval",
        "Exaggerated",
        "Triangle",
        "Normal Sans",
        "Obtuse Sans",
        "Perpendicular Sans",
        "Flared",
        "Rounded",
    ],
    &["Very Light", "Light", "Thin", "Book", "Medium", "Demi", "Bold", "Heavy", "Black", "Extra Black"],
    &["Old Style", "Modern", "Even Width", "Extended", "Condensed", "Very Extended", "Very Condensed", "Monospaced"],
    &["None", "Very Low", "Low", "Medium Low", "Medium", "Medium High", "High", "Very High"],
    &[
        "No Variation",
        "Gradual/Diagonal",
        "Gradual/Transitional",
        "Gradual/Vertical",
        "Gradual/Horizontal",
        "Rapid/Vertical",
        "Rapid/Horizontal",
        "Instant/Vertical",
        "Instant/Horizontal",
    ],
    &[
        "Straight Arms/Horizontal",
        "Straight Arms/Wedge",
        "Straight Arms/Vertical",
        "Straight Arms/Single Serif",
        "Straight Arms/Double Serif",
        "Non-Straight/Horizontal",
        "Non-Straight/Wedge",
        "Non-Straight/Vertical",
        "Non-Straight/Single Serif",
        "Non-Straight/Double Serif",
    ],
    &[
        "Normal/Contact",
        "Normal/Weighted",
        "Normal/Boxed",
        "Normal/Flattened",
        "Normal/Rounded",
        "Normal/Off Center",
        "Normal/Square",
        "Oblique/Contact",
        "Oblique/Weighted",
        "Oblique/Boxed",
        "Oblique/Flattened",
        "Oblique/Rounded",
        "Oblique/Off Center",
        "Oblique/Square",
    ],
    &[
        "Standard/Trimmed",
        "Standard/Pointed",
        "Standard/Serifed",
        "High/Trimmed",
        "High/Pointed",
        "High/Serifed",
        "Constant/Trimmed",
        "Constant/Pointed",
        "Constant/Serifed",
        "Low/Trimmed",
        "Low/Pointed",
        "Low/Serifed",
    ],
    &["Constant/Small", "Constant/Standard", "Constant/Large", "Ducking/Small", "Ducking/Standard", "Ducking/Large"],
];

/// The menu entries of digit `at` when the family kind is `kind`.
fn entries(at: usize, kind: i64) -> Vec<Entry<i64>> {
    let named = at == 0 || kind == LATIN_TEXT;
    let names = LATIN_TEXT_NAMES.get(at).copied().unwrap_or_default();
    (0..VALUES)
        .map(|v| {
            let name = match v {
                0 | 1 if named => usize::try_from(v).ok().and_then(|i| ANY.get(i)).map(|s| (*s).to_string()),
                _ if named => usize::try_from(v - 2).ok().and_then(|i| names.get(i)).map(|s| (*s).to_string()),
                _ => None,
            };
            Entry::new(v, name.map_or_else(|| v.to_string(), |n| format!("{v} {n}")))
        })
        .collect()
}

/// The dialog over one `panose` row of a target.
pub struct PanoseDialog {
    id: DialogId,
    pub target: Target,
    pub index: usize,
    /// The ten digits.
    pub digits: Vec<i64>,
    open: bool,
}

impl PanoseDialog {
    /// The dialog over `stored`; missing digits read as 0 (Any).
    pub fn new(target: Target, index: usize, stored: &[i64]) -> PanoseDialog {
        let digits = (0..DIGITS).map(|at| stored.get(at).copied().filter(|d| (0..VALUES).contains(d)).unwrap_or(0)).collect();
        PanoseDialog { id: dialog_id("fontInfo.panose"), target, index, digits, open: true }
    }
}

impl StackDialog for PanoseDialog {
    /// The target, the row index and the ten digits.
    type Result = (Target, usize, Vec<i64>);

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Self::Result>> {
        let w = Env::of(ui).words::<PanoseWords>();
        let categories: [&Label; DIGITS] = [
            &w.family_kind,
            &w.serif_style,
            &w.weight,
            &w.proportion,
            &w.contrast,
            &w.stroke_variation,
            &w.arm_style,
            &w.letterform,
            &w.midline,
            &w.x_height,
        ];
        let PanoseDialog { digits, open, .. } = self;
        let shown = FormDialog::new(w.title.clone(), Bind::bound(digits)).opened(Bind::bound(open)).show(ui, |ui, _, digits| {
            let kind = digits.first().copied().unwrap_or(0);
            for at in 0..DIGITS {
                let label: Label = match categories.get(at) {
                    Some(name) if at == 0 || kind == LATIN_TEXT => (*name).clone(),
                    _ => format!("{} {}", w.digit.as_str(), at.saturating_add(1)).into(),
                };
                let list = entries(at, kind);
                let current = digits.get(at).copied().unwrap_or(0);
                Field::new(label.clone()).id_salt(("panose", at)).show(ui, |ui, _| {
                    if let Some(picked) = Select::new(label.clone(), Bind::given(current), &list).show(ui).events.committed()
                        && let Some(slot) = digits.get_mut(at)
                    {
                        *slot = *picked;
                    }
                });
            }
            true
        });
        match (shown.accepted, shown.closed) {
            (Some(digits), _) => Some(DialogResult::Ok((self.target.clone(), self.index, digits))),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

/// Opens the dialog a row asked for, and posts what OK stored.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    if let Some(InfoDialog::Panose { target, index }) = app.view.font_info.request.clone() {
        app.view.font_info.request = None;
        let stored = stored_value(app, &target, "panose", index).map(|v| numbers(&v)).unwrap_or_default();
        host.open(PanoseDialog::new(target, index, &stored));
    }
    if let Some(DialogResult::Ok((target, index, digits))) = host.take::<PanoseDialog>() {
        let mut p = target.params();
        p.insert("name".into(), json!("panose"));
        p.insert("index".into(), json!(index));
        p.insert("value".into(), json!(digits));
        app.sent.push(Msg::run("font.customParameters.set", Value::Object(p)));
    }
}
