//! The OS/2 bit dialog of the fsType, codePageRanges and unicodeRanges parameters: a search field over one check box
//! per bit. OK posts one `font.customParameters.set` with the checked values; Cancel posts nothing.
//!
//! fsType stores bit numbers, and its usage levels 1, 2 and 3 exclude each other: checking one clears the others.
//! codePageRanges stores code page numbers (a bit below 64 that a code page stands for reads as that page).
//! unicodeRanges stores bit numbers 0 to 122. Values the dialog has no box for are kept as they are.
//!
//! The code page and Unicode range names are the OpenType OS/2 table's names for each bit, kept as data like glyph
//! names, not as words.

use std::collections::BTreeSet;

use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::layout_components::Stack;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::TextField;
use egui::Ui;
use fontcraft_engine::fontinfo::rows::{RowKind, rows_of};
use fontcraft_engine::fontinfo::value as fv;
use fontcraft_engine::model::plist::Value as Plist;
use serde_json::{Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::shell::font_info::{InfoDialog, Target};

craft::ui::words! {
    /// The words of the OS/2 bit dialog.
    pub struct Os2BitsWords {
        fs_type: "fontcraftOs2Bits.fsType" = "Embedding (fsType)";
        code_pages: "fontcraftOs2Bits.codePages" = "Code Page Ranges";
        unicode_ranges: "fontcraftOs2Bits.unicodeRanges" = "Unicode Ranges";
        search: "fontcraftOs2Bits.search" = "Search";
        restricted: "fontcraftOs2Bits.restricted" = "Restricted License embedding";
        preview: "fontcraftOs2Bits.preview" = "Preview & Print embedding";
        editable: "fontcraftOs2Bits.editable" = "Editable embedding";
        no_subsetting: "fontcraftOs2Bits.noSubsetting" = "No subsetting";
        bitmap_only: "fontcraftOs2Bits.bitmapOnly" = "Bitmap embedding only";
    }
}

/// Most values one parameter holds.
const MAX_VALUES: usize = 256;

/// The fsType usage levels, of which one at most is set.
const USAGE_LEVELS: [i64; 3] = [1, 2, 3];

/// Windows code pages: (page, its `ulCodePageRange` bit, name).
pub const CODE_PAGES: &[(i64, i64, &str)] = &[
    (1252, 0, "Latin 1"),
    (1250, 1, "Latin 2: Eastern Europe"),
    (1251, 2, "Cyrillic"),
    (1253, 3, "Greek"),
    (1254, 4, "Turkish"),
    (1255, 5, "Hebrew"),
    (1256, 6, "Arabic"),
    (1257, 7, "Windows Baltic"),
    (1258, 8, "Vietnamese"),
    (874, 16, "Thai"),
    (932, 17, "JIS/Japan"),
    (936, 18, "Chinese Simplified"),
    (949, 19, "Korean Wansung"),
    (950, 20, "Chinese Traditional"),
    (1361, 21, "Korean Johab"),
    (869, 48, "IBM Greek"),
    (866, 49, "MS-DOS Russian"),
    (865, 50, "MS-DOS Nordic"),
    (864, 51, "Arabic"),
    (863, 52, "MS-DOS Canadian French"),
    (862, 53, "Hebrew"),
    (861, 54, "MS-DOS Icelandic"),
    (860, 55, "MS-DOS Portuguese"),
    (857, 56, "IBM Turkish"),
    (855, 57, "IBM Cyrillic"),
    (852, 58, "Latin 2"),
    (775, 59, "MS-DOS Baltic"),
    (737, 60, "Greek (437 G)"),
    (708, 61, "Arabic (ASMO 708)"),
    (850, 62, "WE/Latin 1"),
    (437, 63, "US"),
];

/// The Unicode range of each bit, by bit number (its first block).
pub const UNICODE_RANGES: [&str; 123] = [
    "Basic Latin",
    "Latin-1 Supplement",
    "Latin Extended-A",
    "Latin Extended-B",
    "IPA Extensions",
    "Spacing Modifier Letters",
    "Combining Diacritical Marks",
    "Greek and Coptic",
    "Coptic",
    "Cyrillic",
    "Armenian",
    "Hebrew",
    "Vai",
    "Arabic",
    "NKo",
    "Devanagari",
    "Bengali",
    "Gurmukhi",
    "Gujarati",
    "Oriya",
    "Tamil",
    "Telugu",
    "Kannada",
    "Malayalam",
    "Thai",
    "Lao",
    "Georgian",
    "Balinese",
    "Hangul Jamo",
    "Latin Extended Additional",
    "Greek Extended",
    "General Punctuation",
    "Superscripts And Subscripts",
    "Currency Symbols",
    "Combining Diacritical Marks For Symbols",
    "Letterlike Symbols",
    "Number Forms",
    "Arrows",
    "Mathematical Operators",
    "Miscellaneous Technical",
    "Control Pictures",
    "Optical Character Recognition",
    "Enclosed Alphanumerics",
    "Box Drawing",
    "Block Elements",
    "Geometric Shapes",
    "Miscellaneous Symbols",
    "Dingbats",
    "CJK Symbols And Punctuation",
    "Hiragana",
    "Katakana",
    "Bopomofo",
    "Hangul Compatibility Jamo",
    "Phags-pa",
    "Enclosed CJK Letters And Months",
    "CJK Compatibility",
    "Hangul Syllables",
    "Non-Plane 0",
    "Phoenician",
    "CJK Radicals Supplement",
    "Private Use Area (plane 0)",
    "CJK Strokes",
    "Alphabetic Presentation Forms",
    "Arabic Presentation Forms-A",
    "Combining Half Marks",
    "Vertical Forms",
    "Small Form Variants",
    "Arabic Presentation Forms-B",
    "Halfwidth And Fullwidth Forms",
    "Specials",
    "Tibetan",
    "Syriac",
    "Thaana",
    "Sinhala",
    "Myanmar",
    "Ethiopic",
    "Cherokee",
    "Unified Canadian Aboriginal Syllabics",
    "Ogham",
    "Runic",
    "Khmer",
    "Mongolian",
    "Braille Patterns",
    "Yi Syllables",
    "Tagalog",
    "Old Italic",
    "Gothic",
    "Deseret",
    "Byzantine Musical Symbols",
    "Mathematical Alphanumeric Symbols",
    "Private Use (plane 15)",
    "Variation Selectors",
    "Tags",
    "Limbu",
    "Tai Le",
    "New Tai Lue",
    "Buginese",
    "Glagolitic",
    "Tifinagh",
    "Yijing Hexagram Symbols",
    "Syloti Nagri",
    "Linear B Syllabary",
    "Ancient Greek Numbers",
    "Ugaritic",
    "Old Persian",
    "Shavian",
    "Osmanya",
    "Cypriot Syllabary",
    "Kharoshthi",
    "Tai Xuan Jing Symbols",
    "Cuneiform",
    "Counting Rod Numerals",
    "Sundanese",
    "Lepcha",
    "Ol Chiki",
    "Saurashtra",
    "Kayah Li",
    "Rejang",
    "Cham",
    "Ancient Symbols",
    "Phaistos Disc",
    "Lycian",
    "Mahjong Tiles",
];

/// Which bit field a dialog edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitSet {
    FsType,
    CodePages,
    UnicodeRanges,
}

impl BitSet {
    /// The bit field of the parameter `name`.
    pub fn of(name: &str) -> Option<BitSet> {
        match name {
            "fsType" => Some(BitSet::FsType),
            "codePageRanges" => Some(BitSet::CodePages),
            "unicodeRanges" => Some(BitSet::UnicodeRanges),
            _ => None,
        }
    }

    fn title(self, words: &Os2BitsWords) -> Label {
        match self {
            BitSet::FsType => words.fs_type.clone(),
            BitSet::CodePages => words.code_pages.clone(),
            BitSet::UnicodeRanges => words.unicode_ranges.clone(),
        }
    }

    /// The boxes: the value each stores and its label.
    fn entries(self, words: &Os2BitsWords) -> Vec<(i64, Label)> {
        match self {
            BitSet::FsType => vec![
                (1, words.restricted.clone()),
                (2, words.preview.clone()),
                (3, words.editable.clone()),
                (8, words.no_subsetting.clone()),
                (9, words.bitmap_only.clone()),
            ],
            BitSet::CodePages => CODE_PAGES.iter().map(|(page, _, name)| (*page, format!("{page} {name}").into())).collect(),
            BitSet::UnicodeRanges => (0i64..).zip(UNICODE_RANGES).map(|(bit, name)| (bit, format!("{bit} {name}").into())).collect(),
        }
    }

    /// A stored value as the dialog's boxes hold it: a code page's bit reads as the page.
    fn read(self, value: i64) -> i64 {
        match self {
            BitSet::CodePages => CODE_PAGES.iter().find(|(_, bit, _)| *bit == value).map_or(value, |(page, _, _)| *page),
            _ => value,
        }
    }
}

/// The dialog over one parameter row of a target.
pub struct Os2BitsDialog {
    id: DialogId,
    pub target: Target,
    pub name: String,
    pub index: usize,
    pub set: BitSet,
    /// The checked values, and the stored values no box shows.
    pub values: BTreeSet<i64>,
    pub search: String,
    open: bool,
}

impl Os2BitsDialog {
    pub fn new(target: Target, name: String, index: usize, set: BitSet, stored: &[i64]) -> Os2BitsDialog {
        let values = stored.iter().take(MAX_VALUES).map(|v| set.read(*v)).collect();
        Os2BitsDialog { id: dialog_id("fontInfo.os2Bits"), target, name, index, set, values, search: String::new(), open: true }
    }
}

/// `values` with `value` checked or not; checking an fsType usage level clears the other levels.
pub fn toggle(set: BitSet, values: &mut BTreeSet<i64>, value: i64, on: bool) {
    if !on {
        values.remove(&value);
        return;
    }
    if set == BitSet::FsType && USAGE_LEVELS.contains(&value) {
        values.retain(|v| !USAGE_LEVELS.contains(v));
    }
    values.insert(value);
}

impl StackDialog for Os2BitsDialog {
    /// The target, the parameter name and its row index, and the values to store.
    type Result = (Target, String, usize, Vec<i64>);

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Self::Result>> {
        let words = Env::of(ui).words::<Os2BitsWords>();
        let set = self.set;
        let entries = set.entries(&words);
        let Os2BitsDialog { values, search, open, .. } = self;
        let shown = FormDialog::new(set.title(&words), Bind::bound(values)).opened(Bind::bound(open)).show(ui, |ui, _, values| {
            let _ = TextField::search(words.search.clone(), Bind::bound(&mut *search)).show(ui);
            let wanted = search.trim().to_lowercase();
            Stack::new().show(ui, |ui, _| {
                for (value, label) in entries.iter().filter(|(_, l)| wanted.is_empty() || l.as_str().to_lowercase().contains(&wanted)) {
                    let out = Checkbox::new(Bind::given(values.contains(value))).label(label.clone()).id_salt(("bit", *value)).show(ui);
                    if let Some(on) = out.events.committed() {
                        toggle(set, values, *value, *on);
                    }
                }
            });
            true
        });
        match (shown.accepted, shown.closed) {
            (Some(values), _) => Some(DialogResult::Ok((self.target.clone(), self.name.clone(), self.index, values.into_iter().collect()))),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

/// The value of the `index`th row of the parameter `name` of `target`'s first font, master or export.
pub fn stored_value(app: &FontCraftApp, target: &Target, name: &str, index: usize) -> Option<Plist> {
    let font = app.session.font().ok()?;
    rows_of(font, target.scope, target.first()).into_iter().filter(|r| r.kind == RowKind::Param && r.key == name).nth(index).map(|r| r.value)
}

/// The whole numbers of a list value; `bit N` reads as N.
pub fn numbers(value: &Plist) -> Vec<i64> {
    let items = match value {
        Plist::Array(a) => a.items.as_slice(),
        other => std::slice::from_ref(other),
    };
    let read = |item: &Plist| {
        let n = fv::number(item).or_else(|| item.as_str()?.trim().strip_prefix("bit ")?.trim().parse::<f64>().ok())?;
        (n.fract() == 0.0 && n.abs() < 1.0e9).then_some(n as i64)
    };
    items.iter().take(MAX_VALUES).filter_map(read).collect()
}

/// Opens the dialog a row asked for, and posts what OK stored.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    if let Some(InfoDialog::Bits { target, name, index }) = app.view.font_info.request.clone() {
        app.view.font_info.request = None;
        if let Some(set) = BitSet::of(&name) {
            let stored = stored_value(app, &target, &name, index).map(|v| numbers(&v)).unwrap_or_default();
            host.open(Os2BitsDialog::new(target, name, index, set, &stored));
        }
    }
    if let Some(DialogResult::Ok((target, name, index, values))) = host.take::<Os2BitsDialog>() {
        let mut p = target.params();
        p.insert("name".into(), json!(name));
        p.insert("index".into(), json!(index));
        p.insert("value".into(), json!(values));
        app.sent.push(Msg::run("font.customParameters.set", Value::Object(p)));
    }
}
