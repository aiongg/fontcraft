//! The dialogs of the Font › CID menu. Each runs its command once, on OK; Cancel changes nothing.
//!
//! | Dialog | Command |
//! |---|---|
//! | `dialog.convertToCid` | `cid.convertToCid`: the character collection, and a CMap file to take the CIDs from |
//! | `dialog.flattenCid` | `cid.flatten`: a CMap file to take code points from |
//! | `dialog.editCidSubfonts` | `cid.editSubfonts`: adds a blank sub-font or an open font, removes one, or sets the supplement |
//!
//! Showing a sub-font's glyphs and its hinting values have no field: `show` and `private` stay with the command.

use fontcraft_engine::Session;
use fontcraft_engine::model::MAX_NAME_LEN;
use fontcraft_engine::model::cid::{CidFont, is_cid_keyed};
use serde_json::{Map, Value, json};

use super::fields::{Field, Kind, MAX_CHOICES};
use super::interp::no_link;
use super::spec::{DialogSpec, Fields, Shaped};

pub const SPECS: &[DialogSpec] = &[CONVERT, FLATTEN, EDIT_SUBFONTS];

/// The supplement a character collection may have.
const MAX_SUPPLEMENT: i64 = 65_535;
const BLANK: &str = "blank";

const CONVERT: DialogSpec = DialogSpec::new(
    "dialog.convertToCid",
    "Convert to CID-Keyed",
    "cid.convertToCid",
    Fields::Shaped(Shaped { fields: convert_fields, given: convert_given, params: convert_params, link: no_link }),
)
.run_on_ok()
.buttons("Convert", "");

const FLATTEN: DialogSpec = DialogSpec::new(
    "dialog.flattenCid",
    "Flatten CID Font",
    "cid.flatten",
    Fields::Shaped(Shaped { fields: flatten_fields, given: keyed_given, params: flatten_params, link: no_link }),
)
.run_on_ok()
.buttons("Flatten", "");

const EDIT_SUBFONTS: DialogSpec = DialogSpec::new(
    "dialog.editCidSubfonts",
    "Edit CID Sub-fonts",
    "cid.editSubfonts",
    Fields::Shaped(Shaped { fields: edit_fields, given: keyed_given, params: edit_params, link: no_link }),
)
.run_on_ok();

fn text<'a>(values: &'a Map<String, Value>, key: &str) -> &'a str {
    values.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default()
}

fn whole(values: &Map<String, Value>, key: &str) -> i64 {
    values.get(key).and_then(Value::as_f64).filter(|n| n.is_finite()).map_or(0, |n| n as i64)
}

/// A CMap file has no extension of its own.
fn cmap_field(doc: &str) -> Field {
    Field::open_file("cmap", "CMap File", &[], doc)
}

/// The CMap path of `values` as the command's `cmap`; left out when empty.
fn with_cmap(values: &Map<String, Value>, mut m: Map<String, Value>) -> Map<String, Value> {
    let path = text(values, "cmap");
    if !path.is_empty() {
        m.insert("cmap".into(), json!(path));
    }
    m
}

fn convert_fields(_: &Session) -> Vec<Field> {
    let word = |key: &str, label: &str, default: &str, doc: &str| Field { default: json!(default), ..Field::text(key, label, MAX_NAME_LEN, doc) };
    vec![
        word("registry", "Registry", "Adobe", "The character collection's registry: letters, digits and _"),
        word("ordering", "Ordering", "Identity", "The collection's ordering: letters, digits and _; Identity without a CMap file"),
        Field::integer("supplement", "Supplement", 0, MAX_SUPPLEMENT, 0, "The collection's supplement number"),
        cmap_field("A Unicode CMap file of the same registry and ordering to take the CIDs from; empty numbers the glyphs in glyph order"),
    ]
}

fn convert_given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let font = session.font().map_err(|e| e.to_string())?;
    if is_cid_keyed(font) {
        return Err("Convert to CID-Keyed: the font is CID-keyed already".into());
    }
    Ok(given.clone())
}

fn convert_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("registry".into(), json!(text(values, "registry")));
    m.insert("ordering".into(), json!(text(values, "ordering")));
    m.insert("supplement".into(), json!(whole(values, "supplement")));
    with_cmap(values, m)
}

fn flatten_fields(_: &Session) -> Vec<Field> {
    vec![cmap_field("A Unicode CMap file that gives each glyph its code points by CID; empty keeps the glyphs' own")]
}

fn keyed_given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let font = session.font().map_err(|e| e.to_string())?;
    if !is_cid_keyed(font) {
        return Err("the font is not CID-keyed".into());
    }
    Ok(given.clone())
}

fn flatten_params(values: &Map<String, Value>) -> Map<String, Value> {
    with_cmap(values, Map::new())
}

fn edit_fields(session: &Session) -> Vec<Field> {
    let cid = session.font().ok().and_then(|f| CidFont::read(f).ok().flatten());
    let subfonts: Vec<String> = cid.iter().flat_map(|c| c.subfonts.iter()).take(MAX_CHOICES).map(|s| s.name.clone()).collect();
    let supplement = cid.as_ref().map_or(0, |c| i64::from(c.ros.supplement));
    let active = session.active().map(|d| d.id);
    let others: Vec<(String, String)> = session
        .documents()
        .iter()
        .filter(|d| Some(d.id) != active)
        .take(MAX_CHOICES.saturating_sub(1))
        .map(|d| (d.path.as_ref().map_or_else(|| d.title.clone(), |p| p.display().to_string()), d.title.clone()))
        .collect();
    let (choices, labels): (Vec<String>, Vec<String>) = std::iter::once((BLANK.to_string(), "Blank".to_string())).chain(others).unzip();
    let pick = |key: &str, label: &str, choices: Vec<String>, labels: Vec<String>, doc: &str| Field {
        key: key.into(),
        label: label.into(),
        default: choices.first().map_or(Value::Null, |c| json!(c)),
        kind: Kind::Choice { choices, labels },
        doc: doc.into(),
        group: None,
        same_as: None,
    };
    vec![
        Field::choice(
            "action",
            "Action",
            &[("insert", "Add Sub-font"), ("remove", "Remove Sub-font"), ("supplement", "Set Supplement")],
            "insert",
            "What OK does",
        ),
        pick("insert", "Add", choices, labels, "The sub-font to add: an empty one, or the glyphs of another open font").in_group("Add Sub-font"),
        Field::text("name", "Name", MAX_NAME_LEN, "The new sub-font's name; empty takes the added font's family name").in_group("Add Sub-font"),
        pick("remove", "Remove", subfonts.clone(), subfonts, "The sub-font to remove with its glyphs").in_group("Remove Sub-font"),
        Field::integer("supplement", "Supplement", 0, MAX_SUPPLEMENT, supplement, "The character collection's supplement number")
            .in_group("Set Supplement"),
    ]
}

/// The one action the command takes per run.
fn edit_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut m = Map::new();
    match text(values, "action") {
        "remove" => {
            m.insert("remove".into(), json!(text(values, "remove")));
        }
        "supplement" => {
            m.insert("supplement".into(), json!(whole(values, "supplement")));
        }
        _ => {
            let source = text(values, "insert");
            m.insert("insert".into(), json!(if source.is_empty() { BLANK } else { source }));
            let name = text(values, "name");
            if !name.is_empty() {
                m.insert("name".into(), json!(name));
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each dialog's values are the parameters its command takes.
    #[test]
    fn the_values_are_the_commands_parameters() {
        let values = |v: Value| v.as_object().cloned().unwrap_or_default();
        let mut s = Session::with_document();
        let fields = convert_fields(&s);
        let defaults: Map<String, Value> = fields.iter().map(|f| (f.key.clone(), f.default.clone())).collect();
        let params = Value::Object(convert_params(&defaults));
        assert_eq!(params, json!({"registry": "Adobe", "ordering": "Identity", "supplement": 0}));
        assert!(keyed_given(&s, &Map::new()).is_err(), "an ordinary font has no sub-fonts");
        s.execute("cid.convertToCid", &params).unwrap();
        assert!(convert_given(&s, &Map::new()).is_err());

        let defaults: Map<String, Value> = edit_fields(&s).iter().map(|f| (f.key.clone(), f.default.clone())).collect();
        let add = Value::Object(edit_params(&defaults));
        assert_eq!(add, json!({"insert": "blank"}));
        s.execute("cid.editSubfonts", &add).unwrap();
        let names: Vec<String> = CidFont::read(s.font().unwrap()).unwrap().unwrap().subfonts.iter().map(|f| f.name.clone()).collect();
        assert_eq!(names.len(), 2);
        let remove = Value::Object(edit_params(&values(json!({"action": "remove", "remove": names[1], "supplement": 3}))));
        s.execute("cid.editSubfonts", &remove).unwrap();
        let set = Value::Object(edit_params(&values(json!({"action": "supplement", "supplement": 3.0, "name": "x"}))));
        assert_eq!(set, json!({"supplement": 3}));
        s.execute("cid.editSubfonts", &set).unwrap();
        s.execute("cid.flatten", &Value::Object(flatten_params(&values(json!({"cmap": ""}))))).unwrap();
        assert!(!is_cid_keyed(s.font().unwrap()));
    }
}
