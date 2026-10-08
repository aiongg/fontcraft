//! The vocabulary of font information: parameters, properties and user data.
//!
//! - A [`Param`] is a custom parameter: a name, a value, and a switch. Parameters form an ordered
//!   list ([`Params`]) on the font, on each master and on each instance; a name may repeat (the
//!   reference's `Name Table Entry`, `Virtual Master`), and a disabled one is kept but not used.
//! - A [`Property`] is a named string, plain or with one value per language: the font's names
//!   (designer, copyright, licence…), which also exist per master and per instance.
//! - [`UserData`] is a free-form ordered dictionary (the reference's `userData`, a UFO's lib).
//!
//! Values are property-list values. They may carry OpenStep spellings only (see
//! `fontcraft_plist`): a reader of an XML format hands the model its values without their
//! spellings, and keeps those in its own `Extra` for writing back. In the native file a value is
//! OpenStep text, or XML text when it holds what OpenStep cannot say (a boolean, a date).

use fontcraft_plist::{Dict, Value};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::Extra;
use crate::Family;
use crate::extra::{parse_value_text, value_text};

/// A custom parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub value: Value,
    /// Kept, but not used when the font is compiled.
    pub disabled: bool,
    pub extra: Extra,
}

impl Param {
    pub fn new(name: impl Into<String>, value: Value) -> Self {
        Self { name: name.into(), value, disabled: false, extra: Extra::new() }
    }
}

/// Parameters in order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Params(pub Vec<Param>);

impl Params {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The value of the first enabled parameter called `name`.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.iter().find(|p| p.name == name && !p.disabled).map(|p| &p.value)
    }

    /// Every enabled value of `name`, in order (for parameters that may repeat).
    pub fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
        self.0.iter().filter(move |p| p.name == name && !p.disabled).map(|p| &p.value)
    }

    /// Sets the first parameter called `name` (enabling it), or appends one.
    pub fn set(&mut self, name: &str, value: Value) {
        match self.0.iter_mut().find(|p| p.name == name) {
            Some(p) => {
                p.value = value;
                p.disabled = false;
            }
            None => self.0.push(Param::new(name, value)),
        }
    }

    /// Removes every parameter called `name`; returns how many there were.
    pub fn remove(&mut self, name: &str) -> usize {
        let before = self.0.len();
        self.0.retain(|p| p.name != name);
        before - self.0.len()
    }
}

/// Whether `v` holds something only the XML syntax can say.
fn needs_xml(v: &Value) -> bool {
    let mut found = false;
    v.visit(&mut |x| found |= matches!(x, Value::Bool(_) | Value::Date(_)));
    found
}

/// A value as native-file text, and whether the text is XML.
fn native_text(v: &Value) -> (String, bool) {
    if needs_xml(v) { (value_text(Family::Ufo, v), true) } else { (value_text(Family::Glyphs, v), false) }
}

fn parse_native(text: &str, xml: bool) -> Result<Value, fontcraft_plist::Error> {
    parse_value_text(if xml { Family::Ufo } else { Family::Glyphs }, text)
}

/// In the native file a parameter is `{"name", "value", "xml"?, "disabled"?, "extra"?}`, with the
/// value as property-list text (see the module notes).
#[derive(Serialize, Deserialize)]
struct ParamRepr {
    name: String,
    value: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    xml: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    disabled: bool,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    extra: Extra,
}

impl Serialize for Param {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let (value, xml) = native_text(&self.value);
        ParamRepr { name: self.name.clone(), value, xml, disabled: self.disabled, extra: self.extra.clone() }.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Param {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let r = ParamRepr::deserialize(d)?;
        let value = parse_native(&r.value, r.xml).map_err(|e| D::Error::custom(format!("parameter `{}`: {e}", r.name)))?;
        Ok(Param { name: r.name, value, disabled: r.disabled, extra: r.extra })
    }
}

/// A named string, plain or localised.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Property {
    /// The reference's property key (`designers`, `copyrights`, `familyNames`…).
    pub key: String,
    pub value: PropertyValue,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PropertyValue {
    /// One value for every language (URLs, the version string, the vendor id…).
    Single(String),
    /// One value per language, in source order.
    Localized(Vec<Localized>),
}

/// One language's value of a property.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Localized {
    /// An OpenType language-system tag (`dflt`, `DEU`), as the reference's format writes it.
    pub language: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// The default language of a localised property.
pub const DEFAULT_LANGUAGE: &str = "dflt";

impl Property {
    /// The value for the default language, or the first one.
    pub fn default_value(&self) -> Option<&str> {
        match &self.value {
            PropertyValue::Single(s) => Some(s),
            PropertyValue::Localized(v) => v.iter().find(|l| l.language == DEFAULT_LANGUAGE).or_else(|| v.first()).map(|l| l.value.as_str()),
        }
    }
}

/// The properties of a font, master or instance, in order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Properties(pub Vec<Property>);

impl Properties {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&Property> {
        self.0.iter().find(|p| p.key == key)
    }

    /// The default-language value of `key`.
    pub fn value(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Property::default_value)
    }

    /// Sets `key` to a single value for all languages, or, for a localised property, its
    /// default-language value.
    pub fn set(&mut self, key: &str, value: &str, localized: bool) {
        let make = || {
            if localized {
                PropertyValue::Localized(vec![Localized { language: DEFAULT_LANGUAGE.into(), value: value.into(), extra: Extra::new() }])
            } else {
                PropertyValue::Single(value.into())
            }
        };
        match self.0.iter_mut().find(|p| p.key == key) {
            Some(p) => match &mut p.value {
                PropertyValue::Single(s) => *s = value.into(),
                PropertyValue::Localized(v) => match v.iter_mut().find(|l| l.language == DEFAULT_LANGUAGE) {
                    Some(l) => l.value = value.into(),
                    None => v.insert(0, Localized { language: DEFAULT_LANGUAGE.into(), value: value.into(), extra: Extra::new() }),
                },
            },
            None => self.0.push(Property { key: key.into(), value: make(), extra: Extra::new() }),
        }
    }

    pub fn remove(&mut self, key: &str) -> bool {
        let before = self.0.len();
        self.0.retain(|p| p.key != key);
        before != self.0.len()
    }
}

/// Free-form data: an ordered dictionary of property-list values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UserData(pub Dict);

impl UserData {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }
}

/// In the native file user data is OpenStep property-list text, or `{"xml": text}` (see the module
/// notes).
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum UserDataRepr {
    OpenStep(String),
    Xml { xml: String },
}

impl Serialize for UserData {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match native_text(&Value::Dict(self.0.clone())) {
            (text, false) => UserDataRepr::OpenStep(text),
            (xml, true) => UserDataRepr::Xml { xml },
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for UserData {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (text, xml) = match UserDataRepr::deserialize(d)? {
            UserDataRepr::OpenStep(t) => (t, false),
            UserDataRepr::Xml { xml } => (xml, true),
        };
        match parse_native(&text, xml).map_err(|e| D::Error::custom(format!("user data: {e}")))? {
            Value::Dict(dict) => Ok(UserData(dict)),
            _ => Err(D::Error::custom("user data is not a dictionary")),
        }
    }
}

/// OpenType language-system tags and Windows language ids (glyphsLib's `LANGUAGE_MAPPING`,
/// Apache-2.0; fontc's reader has the same table). A property's localised value in one of these
/// languages becomes a name record in it.
pub const LANGUAGES: &[(&str, u16)] = &[
    ("AFK", 0x0436),
    ("ARA", 0x0C01),
    ("ASM", 0x044D),
    ("AZE", 0x042C),
    ("BEL", 0x0423),
    ("BEN", 0x0845),
    ("BGR", 0x0402),
    ("BRE", 0x047E),
    ("CAT", 0x0403),
    ("CSY", 0x0405),
    ("DAN", 0x0406),
    ("DEU", 0x0407),
    ("ELL", 0x0408),
    ("ENG", 0x0409),
    ("ESP", 0x0C0A),
    ("ETI", 0x0425),
    ("EUQ", 0x042D),
    ("FIN", 0x040B),
    ("FLE", 0x0813),
    ("FOS", 0x0438),
    ("FRA", 0x040C),
    ("FRI", 0x0462),
    ("GRN", 0x046F),
    ("GUJ", 0x0447),
    ("HAU", 0x0468),
    ("HIN", 0x0439),
    ("HRV", 0x041A),
    ("HUN", 0x040E),
    ("HVE", 0x042B),
    ("IRI", 0x083C),
    ("ISL", 0x040F),
    ("ITA", 0x0410),
    ("IWR", 0x040D),
    ("JPN", 0x0411),
    ("KAN", 0x044B),
    ("KAT", 0x0437),
    ("KAZ", 0x043F),
    ("KHM", 0x0453),
    ("KOK", 0x0457),
    ("KOR", 0x0412),
    ("LAO", 0x0454),
    ("LSB", 0x082E),
    ("LTH", 0x0427),
    ("LVI", 0x0426),
    ("MAR", 0x044E),
    ("MKD", 0x042F),
    ("MLR", 0x044C),
    ("MLY", 0x043E),
    ("MOL", 0x0818),
    ("MNG", 0x0352),
    ("MTS", 0x043A),
    ("NEP", 0x0461),
    ("NLD", 0x0413),
    ("NOB", 0x0414),
    ("ORI", 0x0448),
    ("PAN", 0x0446),
    ("PAS", 0x0463),
    ("PLK", 0x0415),
    ("PTG", 0x0816),
    ("PTG-BR", 0x0416),
    ("RMS", 0x0417),
    ("ROM", 0x0418),
    ("RUS", 0x0419),
    ("SAN", 0x044F),
    ("SKY", 0x041B),
    ("SLV", 0x0424),
    ("SQI", 0x041C),
    ("SRB", 0x081A),
    ("SVE", 0x041D),
    ("TAM", 0x0449),
    ("TAT", 0x0444),
    ("TEL", 0x044A),
    ("THA", 0x041E),
    ("TIB", 0x0451),
    ("TRK", 0x041F),
    ("UKR", 0x0422),
    ("URD", 0x0420),
    ("USB", 0x042E),
    ("UYG", 0x0480),
    ("UZB", 0x0443),
    ("VIT", 0x042A),
    ("WEL", 0x0452),
    ("ZHH", 0x0C04),
    ("ZHS", 0x0804),
    ("ZHT", 0x0404),
];

/// The Windows language id of a language-system tag ([`LANGUAGES`]).
pub fn language_id(tag: &str) -> Option<u16> {
    LANGUAGES.iter().find(|(t, _)| *t == tag).map(|(_, id)| *id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fontcraft_plist::{Number, Str};

    #[test]
    fn parameters_keep_order_repeats_and_switches() {
        let mut p = Params::default();
        p.0.push(Param::new("Name Table Entry", Value::String("1; a".into())));
        p.0.push(Param { disabled: true, ..Param::new("typoAscender", Value::Number(Number::int(900))) });
        p.0.push(Param::new("Name Table Entry", Value::String("2; b".into())));
        assert_eq!(p.all("Name Table Entry").count(), 2);
        assert!(p.get("typoAscender").is_none(), "a disabled parameter is not used");
        p.set("typoAscender", Value::Number(Number::int(950)));
        assert_eq!(p.get("typoAscender").and_then(Value::as_num).map(|n| n.as_f64()), Some(950.0));
        assert_eq!(p.remove("Name Table Entry"), 2);
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Params>(&json).unwrap(), p);
    }

    #[test]
    fn properties_are_plain_or_localised() {
        let mut p = Properties::default();
        p.set("designers", "A", true);
        p.set("designerURL", "https://example.org", false);
        p.set("designers", "B", true);
        assert_eq!(p.value("designers"), Some("B"));
        assert!(matches!(p.get("designerURL").unwrap().value, PropertyValue::Single(_)));
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Properties>(&json).unwrap(), p);
    }

    #[test]
    fn user_data_keeps_order_and_spelling_in_the_native_file() {
        let doc = fontcraft_plist::openstep::parse("{\nzeta = 1.0;\nalpha = (a,b);\n}").unwrap();
        let Value::Dict(d) = doc.root else { panic!() };
        let u = UserData(d);
        let json = serde_json::to_string(&u).unwrap();
        let back: UserData = serde_json::from_str(&json).unwrap();
        assert_eq!(back.0.entries.first().map(|(k, _)| k.as_str()), Some("zeta"));
        assert_eq!(value_text(Family::Glyphs, &Value::Dict(back.0)), "{\nzeta = 1.0;\nalpha = (a,b);\n}");
        let _ = Str::new("");
    }

    #[test]
    fn booleans_and_dates_survive_the_native_file() {
        let mut d = Dict::new();
        d.insert("flag", Value::Bool(true));
        d.insert("when", Value::Date("2026-10-08T12:00:00Z".into()));
        let u = UserData(d.clone());
        let back: UserData = serde_json::from_str(&serde_json::to_string(&u).unwrap()).unwrap();
        assert_eq!(back, u);
        let p = Param::new("postscriptIsFixedPitch", Value::Bool(false));
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"xml\":true"), "{json}");
        assert_eq!(serde_json::from_str::<Param>(&json).unwrap().value, Value::Bool(false));
    }
}
