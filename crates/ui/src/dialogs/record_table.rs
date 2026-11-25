//! The row tables of the parameters whose value is a list of records:
//!
//! | Parameter | Columns | Stored as |
//! |---|---|---|
//! | gasp Table | the largest size (PPM) and the four rendering flags | a dictionary from size to flag bits |
//! | meta Table | the tag (dlng or slng) and its languages | a list of `{tag, data}` dictionaries |
//! | Name Table Entry | name ID, platform, encoding, language and text: one record, with no plus and minus | `ID platform encoding language; text` |
//!
//! OK posts one `font.customParameters.set`; it is off while a record cannot be stored or the command would refuse the
//! value. Bits and entries the table has no column for are kept, and a value of more than [`MAX_RECORDS`] records is not
//! opened.

use std::collections::BTreeSet;

use craft::behavior::IconId;
use craft::theme::variant::Tone;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::TextField;
use egui::Ui;
use fontcraft_engine::fontinfo::value as fv;
use fontcraft_engine::model::plist::Value as Plist;
use serde_json::{Map, Value, json};

use super::os2_bits::stored_value;
use crate::app::models::font_info::plain_text;
use crate::app::{FontCraftApp, Msg};
use crate::panels::font_info::param_value::refusal;
use crate::shell::font_info::{InfoDialog, Target};

craft::ui::words! {
    /// The words of the record tables.
    pub struct RecordTableWords {
        size: "fontcraftRecordTable.size" = "Up to PPM";
        gridfit: "fontcraftRecordTable.gridfit" = "Grid-fit";
        gray: "fontcraftRecordTable.gray" = "Smooth";
        symmetric_gridfit: "fontcraftRecordTable.symmetricGridfit" = "Symmetric grid-fit";
        symmetric_smoothing: "fontcraftRecordTable.symmetricSmoothing" = "Symmetric smoothing";
        tag: "fontcraftRecordTable.tag" = "Tag";
        languages: "fontcraftRecordTable.languages" = "Languages";
        name_id: "fontcraftRecordTable.nameId" = "Name ID";
        platform: "fontcraftRecordTable.platform" = "Platform";
        encoding: "fontcraftRecordTable.encoding" = "Encoding";
        language: "fontcraftRecordTable.language" = "Language";
        text: "fontcraftRecordTable.text" = "Text";
        add: "fontcraftRecordTable.add" = "Add";
        remove: "fontcraftRecordTable.remove" = "Remove this record";
        bad_size: "fontcraftRecordTable.badSize" = "A size is a whole number from 0 to 65535, each one once";
        bad_entry: "fontcraftRecordTable.badEntry" = "Fill Name ID, Platform, Encoding and Language from the left, one word each";
        empty: "fontcraftRecordTable.empty" = "No records. Add one.";
    }
}

/// Most records one table holds.
pub const MAX_RECORDS: usize = 256;
/// The `gasp` flag bits, in column order.
const GASP_BITS: [i64; 4] = [1, 2, 4, 8];
const META_TAGS: [&str; 2] = ["dlng", "slng"];

/// Which parameter's records a dialog edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordKind {
    Gasp,
    Meta,
    NameEntry,
}

impl RecordKind {
    /// The table of the parameter `name`.
    pub fn of(name: &str) -> Option<RecordKind> {
        match name {
            "gasp Table" => Some(RecordKind::Gasp),
            "meta Table" => Some(RecordKind::Meta),
            "Name Table Entry" => Some(RecordKind::NameEntry),
            _ => None,
        }
    }

    /// A new record.
    fn blank(self) -> Record {
        match self {
            RecordKind::Gasp => Record { cells: vec!["65535".into()], bits: 15, ..Record::default() },
            RecordKind::Meta => Record { cells: vec!["dlng".into(), String::new()], ..Record::default() },
            RecordKind::NameEntry => Record { cells: vec![String::new(); 5], ..Record::default() },
        }
    }

    /// Why [`RecordKind::write`] refuses a record.
    fn problem(self, words: &RecordTableWords) -> &Label {
        match self {
            RecordKind::Gasp | RecordKind::Meta => &words.bad_size,
            RecordKind::NameEntry => &words.bad_entry,
        }
    }

    /// The records of a stored value; `None` when there are more than [`MAX_RECORDS`].
    pub fn read(self, value: &Plist) -> Option<Vec<Record>> {
        let count = match value {
            Plist::Dict(d) => d.entries.len(),
            Plist::Array(a) => a.items.len(),
            _ => 1,
        };
        (count <= MAX_RECORDS).then(|| self.read_all(value))
    }

    fn read_all(self, value: &Plist) -> Vec<Record> {
        match (self, value) {
            (RecordKind::Gasp, Plist::Dict(d)) => d
                .entries
                .iter()
                .map(|(size, bits)| Record {
                    cells: vec![size.as_str().to_string()],
                    bits: fv::number(bits).map_or(0, |n| n as i64),
                    ..Record::default()
                })
                .collect(),
            (RecordKind::Meta, Plist::Array(a)) => {
                let read = |item: &Plist| match item {
                    Plist::Dict(d) => {
                        let text = |key: &str| d.get(key).and_then(Plist::as_str).unwrap_or_default().to_string();
                        let kept = match fv::to_json(item) {
                            Value::Object(map) => map,
                            _ => Map::new(),
                        };
                        Record { cells: vec![text("tag"), text("data")], bits: 0, kept }
                    }
                    other => Record { cells: vec![plain_text(other).unwrap_or_default(), String::new()], ..Record::default() },
                };
                a.items.iter().map(read).collect()
            }
            (RecordKind::NameEntry, value) => {
                let text = plain_text(value).unwrap_or_default();
                let (head, tail) = text.split_once(';').unwrap_or((text.as_str(), ""));
                let mut words = head.split_whitespace();
                let mut cells: Vec<String> = (0..3).map(|_| words.next().unwrap_or_default().to_string()).collect();
                // What a fifth word would be shows in the last column, to be taken out.
                cells.push(words.collect::<Vec<_>>().join(" "));
                cells.push(tail.trim().to_string());
                vec![Record { cells, ..Record::default() }]
            }
            _ => Vec::new(),
        }
    }

    /// The value of `records`; `None` when one cannot be stored.
    pub fn write(self, records: &[Record]) -> Option<Value> {
        match self {
            RecordKind::Gasp => {
                let mut seen = BTreeSet::new();
                let mut out = Map::new();
                for r in records {
                    let size: u16 = r.cell(0).trim().parse().ok()?;
                    if !seen.insert(size) {
                        return None;
                    }
                    out.insert(size.to_string(), json!(r.bits));
                }
                Some(Value::Object(out))
            }
            RecordKind::Meta => {
                let write = |r: &Record| {
                    let mut map = r.kept.clone();
                    for (key, text) in [("tag", r.cell(0)), ("data", r.cell(1).trim())] {
                        // A value that is no text shows empty, and stays unless a text takes its place.
                        if !text.is_empty() || map.get(key).is_none_or(Value::is_string) {
                            map.insert(key.into(), json!(text));
                        }
                    }
                    Value::Object(map)
                };
                Some(records.iter().map(write).collect())
            }
            RecordKind::NameEntry => {
                let r = records.first()?;
                let cells: Vec<&str> = (0..4).map(|c| r.cell(c).trim()).collect();
                let filled = cells.iter().take_while(|t| !t.is_empty()).count();
                let (head, rest) = cells.split_at_checked(filled)?;
                let one_word = |t: &&str| !t.contains(|c: char| c.is_whitespace() || c == ';');
                if !rest.iter().all(|t| t.is_empty()) || !head.iter().all(one_word) {
                    return None;
                }
                Some(Value::String(format!("{}; {}", head.join(" "), r.cell(4).trim())))
            }
        }
    }
}

/// One record: the texts of its columns, the `gasp` flag bits, and the stored entries of a `meta` record.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    pub cells: Vec<String>,
    pub bits: i64,
    pub kept: Map<String, Value>,
}

impl Record {
    fn cell(&self, at: usize) -> &str {
        self.cells.get(at).map_or("", String::as_str)
    }

    /// A text field over the cell at `at`.
    fn field(&mut self, ui: &mut Ui, label: &Label, at: usize, row: usize) {
        if let Some(cell) = self.cells.get_mut(at) {
            let _ = TextField::new(label.clone(), Bind::bound(cell)).id_salt(("cell", row, at)).show(ui);
        }
    }
}

/// The dialog over one parameter row of a target.
pub struct RecordTableDialog {
    id: DialogId,
    pub target: Target,
    pub name: String,
    pub index: usize,
    pub kind: RecordKind,
    pub records: Vec<Record>,
    open: bool,
}

impl RecordTableDialog {
    pub fn new(target: Target, name: String, index: usize, kind: RecordKind, records: Vec<Record>) -> RecordTableDialog {
        RecordTableDialog { id: dialog_id("fontInfo.recordTable"), target, name, index, kind, records, open: true }
    }
}

/// The columns of one record; true when its minus was clicked.
fn record_row(ui: &mut Ui, kind: RecordKind, record: &mut Record, at: usize, words: &RecordTableWords) -> bool {
    let mut removed = false;
    Row::new().id_salt(("record", at)).show(ui, |ui, _| {
        match kind {
            RecordKind::Gasp => {
                record.field(ui, &words.size, 0, at);
                let labels = [&words.gridfit, &words.gray, &words.symmetric_gridfit, &words.symmetric_smoothing];
                for (bit, label) in GASP_BITS.into_iter().zip(labels) {
                    let out = Checkbox::new(Bind::given(record.bits & bit != 0)).label(label.clone()).id_salt(("bit", at, bit)).show(ui);
                    if let Some(on) = out.events.committed() {
                        record.bits = if *on { record.bits | bit } else { record.bits & !bit };
                    }
                }
            }
            RecordKind::Meta => {
                let entries: Vec<Entry<String>> = META_TAGS.iter().map(|t| Entry::new((*t).to_string(), *t)).collect();
                let out = Select::new(words.tag.clone(), Bind::given(record.cell(0).to_string()), &entries).id_salt(("tag", at)).show(ui);
                if let Some(tag) = out.events.committed()
                    && let Some(cell) = record.cells.get_mut(0)
                {
                    cell.clone_from(tag);
                }
                record.field(ui, &words.languages, 1, at);
            }
            RecordKind::NameEntry => {
                for (column, label) in [&words.name_id, &words.platform, &words.encoding, &words.language, &words.text].into_iter().enumerate() {
                    record.field(ui, label, column, at);
                }
            }
        }
        if kind != RecordKind::NameEntry {
            let minus = IconButton::new(IconId::new("minus")).label(words.remove.clone()).tooltip(words.remove.clone());
            removed = minus.id_salt(("minus", at)).show(ui).clicked();
        }
    });
    removed
}

impl StackDialog for RecordTableDialog {
    /// The target, the parameter name and its row index, and the value to store.
    type Result = (Target, String, usize, Value);

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Self::Result>> {
        let words = Env::of(ui).words::<RecordTableWords>();
        let kind = self.kind;
        let RecordTableDialog { name, records, open, .. } = self;
        let shown = FormDialog::new(name.clone(), Bind::bound(records)).opened(Bind::bound(open)).show(ui, |ui, _, records| {
            let mut removed = None;
            Stack::new().show(ui, |ui, _| {
                for (at, record) in records.iter_mut().enumerate() {
                    if record_row(ui, kind, record, at, &words) {
                        removed = Some(at);
                    }
                }
            });
            if let Some(at) = removed.filter(|at| *at < records.len()) {
                records.remove(at);
            }
            if records.is_empty() {
                Text::new(words.empty.clone()).shade(Shade::Muted).show(ui);
            }
            if kind != RecordKind::NameEntry
                && Button::new(words.add.clone()).enabled(records.len() < MAX_RECORDS).id_salt("record add").show(ui).clicked()
            {
                records.push(kind.blank());
            }
            let refused = match kind.write(records) {
                Some(value) => refusal(name, &value),
                None => Some(kind.problem(&words).as_str().to_string()),
            };
            if let Some(message) = &refused {
                Text::new(message.clone()).tone(Tone::Bad).show(ui);
            }
            refused.is_none()
        });
        match (shown.accepted, shown.closed) {
            (Some(records), _) => {
                let value = kind.write(&records)?;
                Some(DialogResult::Ok((self.target.clone(), self.name.clone(), self.index, value)))
            }
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

/// Opens the table a row asked for, and posts what OK stored.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    if let Some(InfoDialog::Records { target, name, index }) = app.view.font_info.request.clone() {
        app.view.font_info.request = None;
        if let Some(kind) = RecordKind::of(&name) {
            let stored = stored_value(app, &target, &name, index);
            match stored.map_or_else(|| Some(Vec::new()), |v| kind.read(&v)) {
                Some(mut records) => {
                    if kind == RecordKind::NameEntry && records.is_empty() {
                        records.push(kind.blank());
                    }
                    host.open(RecordTableDialog::new(target, name, index, kind, records));
                }
                None => app.status = format!("{name} has more than {MAX_RECORDS} records; the table does not open"),
            }
        }
    }
    if let Some(DialogResult::Ok((target, name, index, value))) = host.take::<RecordTableDialog>() {
        let mut p = target.params();
        p.insert("name".into(), json!(name));
        p.insert("index".into(), json!(index));
        p.insert("value".into(), value);
        app.sent.push(Msg::run("font.customParameters.set", Value::Object(p)));
    }
}

#[cfg(test)]
mod tests {
    use fontcraft_engine::fontinfo::value::from_json;

    use super::*;

    /// Each kind reads what it writes, and a value the dialog leaves alone is written as it was.
    #[test]
    fn records_read_back_as_they_were_written() {
        for (name, value) in [
            ("gasp Table", json!({"8": 10, "20": 7, "65535": 31})),
            ("meta Table", json!([{"tag": "dlng", "data": "Latn"}, {"tag": "slng", "data": "Latn, sr-Cyrl", "note": "kept"}])),
            ("meta Table", json!([{"tag": "slng", "data": ["Latn"]}])),
            ("Name Table Entry", json!("25; Prefix")),
            ("Name Table Entry", json!("19 3 1 0x0407; Muster; mit Strichpunkt")),
        ] {
            let kind = RecordKind::of(name).unwrap();
            let records = kind.read(&from_json(name, &value).unwrap()).unwrap();
            assert_eq!(kind.write(&records), Some(value), "{name}");
        }
        let same = vec![RecordKind::Gasp.blank(), RecordKind::Gasp.blank()];
        assert_eq!(RecordKind::Gasp.write(&same), None, "one size twice");
        assert_eq!(RecordKind::Gasp.write(&[Record { cells: vec!["big".into()], bits: 1, ..Record::default() }]), None);
        let many: Map<String, Value> = (0..=MAX_RECORDS).map(|size| (size.to_string(), json!(1))).collect();
        let many = from_json("gasp Table", &Value::Object(many)).unwrap();
        assert_eq!(RecordKind::Gasp.read(&many), None, "no record is cut off");
    }

    /// A Name Table Entry with a gap before a filled column is refused, not stored without it.
    #[test]
    fn a_name_entry_with_a_gap_is_refused() {
        let entry = |cells: [&str; 5]| {
            RecordKind::NameEntry.write(&[Record { cells: cells.iter().map(|c| (*c).to_string()).collect(), ..Record::default() }])
        };
        assert_eq!(entry(["19", "", "1", "0x0407", "Muster"]), None);
        assert_eq!(entry(["19", "3", "1", "0x0407 x", "Muster"]), None);
        assert_eq!(entry(["19", "3", "", "", "Muster"]), Some(json!("19 3; Muster")));
    }
}
