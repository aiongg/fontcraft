//! What a reader found and the model does not hold as a field, kept for the writer.
//!
//! Every struct that maps to an object in a source file has an [`Extra`]: the keys of that object
//! the model has no field for, each tagged with the [`Family`] of formats it came from and kept as
//! a property-list value that remembers its spelling and layout. A writer puts back the entries of
//! its own family where they were, so a key we do not understand yet survives a load and a save
//! byte for byte (`roundtrip.unknownKeysPreserved`, `roundtrip.extraIsPerFormat`).
//!
//! An empty `Extra` is one pointer wide; almost every object has one.

use fontcraft_plist::openstep::{self, Dialect};
use fontcraft_plist::{Str, Value};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The family of formats an entry came from. A writer writes only its own family's entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    /// `.glyphs` and `.glyphspackage`, every version.
    Glyphs,
    /// UFO, every version.
    Ufo,
    Designspace,
    /// Compiled fonts.
    Binary,
    /// Our own format: keys a newer FontCraft wrote, or values a command stored.
    Native,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExtraEntry {
    pub family: Family,
    pub key: Str,
    pub value: Value,
}

/// Keys the model has no field for, in the order they were read.
// The box keeps an empty `Extra` one pointer wide instead of three: every node has one.
#[allow(clippy::box_collection)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extra(Option<Box<Vec<ExtraEntry>>>);

impl Extra {
    pub fn new() -> Self {
        Self(None)
    }

    pub fn is_empty(&self) -> bool {
        self.0.as_ref().is_none_or(|v| v.is_empty())
    }

    pub fn len(&self) -> usize {
        self.0.as_ref().map_or(0, |v| v.len())
    }

    pub fn iter(&self) -> impl Iterator<Item = &ExtraEntry> {
        self.0.iter().flat_map(|v| v.iter())
    }

    /// The entries of one family, in order.
    pub fn of(&self, family: Family) -> impl Iterator<Item = &ExtraEntry> {
        self.iter().filter(move |e| e.family == family)
    }

    pub fn get(&self, family: Family, key: &str) -> Option<&Value> {
        self.of(family).find(|e| e.key.as_str() == key).map(|e| &e.value)
    }

    /// Appends an entry, even if the key is already there (malformed files repeat keys, and the
    /// writer puts every one back).
    pub fn push(&mut self, family: Family, key: impl Into<Str>, value: Value) {
        self.0.get_or_insert_with(Default::default).push(ExtraEntry { family, key: key.into(), value });
    }

    /// Replaces the first entry with this family and key, or appends one.
    pub fn insert(&mut self, family: Family, key: impl Into<Str>, value: Value) {
        let key = key.into();
        let entries = self.0.get_or_insert_with(Default::default);
        match entries.iter_mut().find(|e| e.family == family && e.key == key) {
            Some(e) => e.value = value,
            None => entries.push(ExtraEntry { family, key, value }),
        }
    }

    /// Removes and returns the first entry with this family and key.
    pub fn remove(&mut self, family: Family, key: &str) -> Option<Value> {
        let entries = self.0.as_mut()?;
        let i = entries.iter().position(|e| e.family == family && e.key.as_str() == key)?;
        let value = entries.remove(i).value;
        if entries.is_empty() {
            self.0 = None;
        }
        Some(value)
    }
}

/// In the native file an entry is `{"family", "key", "value"}`, with the value as property-list
/// text so its spelling and layout survive, and `keySpelling` when the key was not written
/// canonically.
#[derive(Serialize, Deserialize)]
struct EntryRepr {
    family: Family,
    key: String,
    #[serde(rename = "keySpelling", default, skip_serializing_if = "Option::is_none")]
    key_spelling: Option<String>,
    value: String,
}

impl Serialize for Extra {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let reprs: Vec<EntryRepr> = self
            .iter()
            .map(|e| {
                let mut value = String::new();
                openstep::write_value(&mut value, &e.value, Dialect::Glyphs3);
                EntryRepr { family: e.family, key: e.key.as_str().to_string(), key_spelling: e.key.spelling().map(str::to_string), value }
            })
            .collect();
        reprs.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Extra {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let reprs = Vec::<EntryRepr>::deserialize(d)?;
        let mut out = Extra::new();
        for r in reprs {
            let key = match r.key_spelling {
                Some(sp) => {
                    let k = openstep::parse_str_token(&sp).map_err(|e| D::Error::custom(format!("extra key `{}`: {e}", r.key)))?;
                    if k.as_str() != r.key {
                        return Err(D::Error::custom(format!("extra key `{}`: its spelling reads as `{}`", r.key, k.as_str())));
                    }
                    k
                }
                None => Str::new(r.key),
            };
            let value = openstep::parse_value(&r.value).map_err(|e| D::Error::custom(format!("extra value of `{}`: {e}", key.as_str())))?;
            out.push(r.family, key, value);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_keep_order_and_family() {
        let mut x = Extra::new();
        assert!(x.is_empty());
        x.push(Family::Glyphs, "b", Value::String("1".into()));
        x.push(Family::Ufo, "a", Value::String("2".into()));
        x.push(Family::Glyphs, "a", Value::String("3".into()));
        let keys: Vec<&str> = x.of(Family::Glyphs).map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["b", "a"]);
        assert_eq!(x.get(Family::Ufo, "a").and_then(Value::as_str), Some("2"));
        x.insert(Family::Glyphs, "b", Value::String("4".into()));
        assert_eq!(x.get(Family::Glyphs, "b").and_then(Value::as_str), Some("4"));
        assert_eq!(x.remove(Family::Glyphs, "b").and_then(|v| v.as_str().map(str::to_string)).as_deref(), Some("4"));
        assert_eq!(x.len(), 2);
    }

    #[test]
    fn the_native_form_keeps_spelling_and_layout() {
        let doc = openstep::parse("{\n\"m01\" = (\n1.0,\n(1,2)\n);\n}").unwrap();
        let Value::Dict(d) = doc.root else { panic!() };
        let mut x = Extra::new();
        for (k, v) in d.entries {
            x.push(Family::Glyphs, k, v);
        }
        let json = serde_json::to_string(&x).unwrap();
        assert!(json.contains("keySpelling"), "{json}");
        let back: Extra = serde_json::from_str(&json).unwrap();
        let e = back.iter().next().unwrap();
        assert_eq!(e.key.spelling(), Some("\"m01\""));
        let mut text = String::new();
        openstep::write_value(&mut text, &e.value, Dialect::Glyphs3);
        assert_eq!(text, "(\n1.0,\n(1,2)\n)");
    }

    #[test]
    fn a_bad_native_entry_is_an_error() {
        assert!(serde_json::from_str::<Extra>(r#"[{"family":"glyphs","key":"a","value":"(1"}]"#).is_err());
        assert!(serde_json::from_str::<Extra>(r#"[{"family":"glyphs","key":"a","keySpelling":"b","value":"1"}]"#).is_err());
        assert!(serde_json::from_str::<Extra>(r#"[{"family":"nope","key":"a","value":"1"}]"#).is_err());
    }
}
