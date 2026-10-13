//! The module manifest (`fc_manifest`) and its parameter schema.
//!
//! Ported from VectorCraft's `crates/plugins/src/manifest.rs` (MIT OR Apache-2.0, the same
//! owner; see `NOTICE`), with FontCraft's three kinds, the `capabilities` list and range checks
//! on defaults.
//!
//! The manifest is UTF-8 JSON of at most [`MAX_MANIFEST_BYTES`]:
//! `{"id", "name", "version", "kind", "author", "description", "params": {name: spec, …},
//! "capabilities": []}`. `params` keeps declaration order (the form shows them in it); each spec
//! is one of the sibling apps' four types ([`ParamSpec`]). `capabilities` must be empty in ABI v1.
//! The rows served: `extensions.sandboxCapabilities`, `plugin.kind.filter`,
//! `plugin.kind.reporter`, `plugin.kind.check`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub use crate::error::{MAX_ID_LEN, MAX_MANIFEST_BYTES, MAX_PARAMS};
use crate::{Error, Result};

/// Longest module name (the menu label), in characters.
pub const MAX_NAME_CHARS: usize = 64;
/// Longest `version`, `author` and `description`, in bytes.
pub const MAX_VERSION_BYTES: usize = 32;
pub const MAX_AUTHOR_BYTES: usize = 128;
pub const MAX_DESCRIPTION_BYTES: usize = 1024;
/// Most options of a choice parameter, and the longest option, in bytes.
pub const MAX_CHOICES: usize = 64;
pub const MAX_CHOICE_BYTES: usize = 64;
/// Largest magnitude of a whole-number parameter's range: whole numbers travel as JSON numbers,
/// which are exact up to 2^53.
pub const MAX_INT_PARAM: i64 = 1 << 53;

/// What a module does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// One layer and parameters in, the layer's new contents out (Filter menu,
    /// `plugin.runFilter`, export filters, Palette forms).
    Filter,
    /// One layer, the view scale and the active flag in, drawing primitives out (View menu,
    /// `view.showReporter`).
    Reporter,
    /// One layer in, findings out (`audit.run`).
    Check,
}

impl Kind {
    /// The manifest's spelling: `filter`, `reporter` or `check`.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Filter => "filter",
            Kind::Reporter => "reporter",
            Kind::Check => "check",
        }
    }
}

/// One declared parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ParamSpec {
    /// A real number in `min..=max`.
    Number { min: f64, max: f64, default: f64 },
    /// A whole number in `min..=max` (default `-1000000..=1000000`, 0).
    Int {
        #[serde(default = "int_min")]
        min: i64,
        #[serde(default = "int_max")]
        max: i64,
        #[serde(default)]
        default: i64,
    },
    /// A checkbox.
    Bool {
        #[serde(default)]
        default: bool,
    },
    /// One of `options` (a pop-up menu); `default` is one of them, the first when omitted.
    Choice {
        options: Vec<String>,
        #[serde(default)]
        default: Option<String>,
    },
}

fn int_min() -> i64 {
    -1_000_000
}

fn int_max() -> i64 {
    1_000_000
}

impl ParamSpec {
    /// The value a parameter takes when it is not given.
    pub fn default_value(&self) -> Value {
        match self {
            ParamSpec::Number { default, .. } => json!(default),
            ParamSpec::Int { default, .. } => json!(default),
            ParamSpec::Bool { default } => json!(default),
            ParamSpec::Choice { options, default } => json!(default.clone().or_else(|| options.first().cloned()).unwrap_or_default()),
        }
    }

    /// What is wrong with this spec, if anything.
    fn problem(&self) -> Option<String> {
        match self {
            ParamSpec::Number { min, max, default } => {
                if !(min.is_finite() && max.is_finite() && default.is_finite()) || min > max {
                    Some("the range must be finite numbers with min ≤ max".into())
                } else if default < min || default > max {
                    Some(format!("the default {default} is outside {min}..{max}"))
                } else {
                    None
                }
            }
            ParamSpec::Int { min, max, default } => {
                if min > max || min.unsigned_abs() > MAX_INT_PARAM.unsigned_abs() || max.unsigned_abs() > MAX_INT_PARAM.unsigned_abs() {
                    Some(format!("the range must have min ≤ max, both within ±{MAX_INT_PARAM}"))
                } else if default < min || default > max {
                    Some(format!("the default {default} is outside {min}..{max}"))
                } else {
                    None
                }
            }
            ParamSpec::Bool { .. } => None,
            ParamSpec::Choice { options, default } => {
                if options.is_empty()
                    || options.len() > MAX_CHOICES
                    || options.iter().any(|o| o.is_empty() || o.len() > MAX_CHOICE_BYTES || o.chars().any(char::is_control))
                {
                    Some(format!("choices must be 1 to {MAX_CHOICES} non-empty, printable strings of at most {MAX_CHOICE_BYTES} bytes"))
                } else if default.as_ref().is_some_and(|d| !options.contains(d)) {
                    Some("the default is not one of the options".into())
                } else {
                    None
                }
            }
        }
    }
}

/// A module's manifest, validated: the id is `[A-Za-z0-9._-]{1,64}`, the name 1 to 64 printable
/// characters, ranges finite and ordered with the default inside, choices non-empty,
/// `capabilities` empty.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// Stable id; reverse-DNS style is recommended (`org.example.shift`).
    pub id: String,
    /// The menu label.
    pub name: String,
    #[serde(default)]
    pub version: String,
    pub kind: Kind,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    /// Parameters in declaration (form) order; a JSON object `{"name": spec, …}` in the manifest.
    #[serde(default, deserialize_with = "ordered_params", serialize_with = "params_as_object")]
    pub params: Vec<(String, ParamSpec)>,
    /// Requested host capabilities. ABI v1 grants none, so a non-empty list is refused.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl Manifest {
    /// Parses and validates manifest JSON of at most [`MAX_MANIFEST_BYTES`].
    pub fn parse(bytes: &[u8]) -> Result<Manifest> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(Error::Manifest(format!("manifest is {} bytes (limit {MAX_MANIFEST_BYTES})", bytes.len())));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| Error::Manifest("manifest is not UTF-8".into()))?;
        let man: Manifest = serde_json::from_str(text).map_err(|e| Error::Manifest(e.to_string()))?;
        man.validate()?;
        Ok(man)
    }

    /// The manifest of a module still being loaded.
    pub(crate) fn placeholder() -> Manifest {
        Manifest {
            id: String::new(),
            name: String::new(),
            version: String::new(),
            kind: Kind::Filter,
            author: String::new(),
            description: String::new(),
            params: Vec::new(),
            capabilities: Vec::new(),
        }
    }

    fn validate(&self) -> Result<()> {
        if !valid_id(&self.id) {
            return Err(Error::Manifest(format!("invalid id {:?}: use 1 to {MAX_ID_LEN} characters from A-Z a-z 0-9 . _ -", self.id)));
        }
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) {
            return Err(Error::Manifest(format!("`name` must be 1 to {MAX_NAME_CHARS} printable characters")));
        }
        if self.version.len() > MAX_VERSION_BYTES || self.author.len() > MAX_AUTHOR_BYTES || self.description.len() > MAX_DESCRIPTION_BYTES {
            return Err(Error::Manifest("`version`, `author` or `description` is too long".into()));
        }
        if let Some(c) = self.capabilities.first() {
            return Err(Error::Manifest(format!("the module asks for the capability {c:?}, but ABI version 1 grants none")));
        }
        for (k, spec) in &self.params {
            if !valid_key(k) {
                return Err(Error::Manifest(format!("invalid parameter name {k:?}")));
            }
            if let Some(problem) = spec.problem() {
                return Err(Error::Manifest(format!("parameter `{k}`: {problem}")));
            }
        }
        Ok(())
    }

    /// Checks `params` against the schema and returns the full parameter object: missing keys
    /// take their defaults, numbers are clamped into range (whole numbers rounded), unknown keys
    /// are dropped; a value of the wrong type, or `params` not an object or null, is an
    /// [`Error::Params`].
    pub fn validate_params(&self, params: &Value) -> Result<Map<String, Value>> {
        let empty = Map::new();
        let given = match params {
            Value::Null => &empty,
            Value::Object(m) => m,
            _ => return Err(Error::Params("module parameters must be a JSON object".into())),
        };
        let mut out = Map::new();
        for (k, spec) in &self.params {
            let Some(v) = given.get(k).filter(|v| !v.is_null()) else {
                out.insert(k.clone(), spec.default_value());
                continue;
            };
            let wrong = |what: &str| Error::Params(format!("`{k}` must be {what}"));
            let resolved = match spec {
                ParamSpec::Number { min, max, .. } => json!(v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| wrong("a number"))?.clamp(*min, *max)),
                ParamSpec::Int { min, max, .. } => {
                    let f = v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| wrong("a number"))?;
                    // Both bounds are within ±2^53 (checked at load), so the cast is exact.
                    json!(f.round().clamp(*min as f64, *max as f64) as i64)
                }
                ParamSpec::Bool { .. } => json!(v.as_bool().ok_or_else(|| wrong("true or false"))?),
                ParamSpec::Choice { options, .. } => {
                    let s = v.as_str().ok_or_else(|| wrong("a string"))?;
                    if !options.iter().any(|o| o == s) {
                        return Err(Error::Params(format!("`{k}` must be one of {}", options.join(", "))));
                    }
                    json!(s)
                }
            };
            out.insert(k.clone(), resolved);
        }
        Ok(out)
    }

    /// The parameters documented the way command parameters are
    /// (`{amount: 0..100 (50), mode: "soft"|"hard" (soft), mono: bool (false)}`), for the
    /// Command Console, MCP and the Actions dialog.
    pub fn params_doc(&self) -> String {
        let parts: Vec<String> = self
            .params
            .iter()
            .map(|(k, spec)| match spec {
                ParamSpec::Number { min, max, default } => format!("{k}: {min}..{max} ({default})"),
                ParamSpec::Int { min, max, default } => format!("{k}: int {min}..{max} ({default})"),
                ParamSpec::Bool { default } => format!("{k}: bool ({default})"),
                ParamSpec::Choice { options, .. } => {
                    let opts: Vec<String> = options.iter().map(|o| format!("{o:?}")).collect();
                    format!("{k}: {} ({})", opts.join("|"), spec.default_value().as_str().unwrap_or_default())
                }
            })
            .collect();
        format!("{{{}}}", parts.join(", "))
    }

    /// Every parameter at its default value.
    pub fn defaults(&self) -> Map<String, Value> {
        self.params.iter().map(|(k, spec)| (k.clone(), spec.default_value())).collect()
    }
}

/// Reads the `params` object keeping its key order (serde_json's `Map` would sort it).
fn ordered_params<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<(String, ParamSpec)>, D::Error> {
    struct V;
    impl<'de> serde::de::Visitor<'de> for V {
        type Value = Vec<(String, ParamSpec)>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an object of parameter specs")
        }
        fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(Vec::new())
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> std::result::Result<Self::Value, A::Error> {
            let mut out: Vec<(String, ParamSpec)> = Vec::new();
            while let Some((k, v)) = map.next_entry::<String, ParamSpec>()? {
                if out.len() >= MAX_PARAMS {
                    return Err(serde::de::Error::custom(format!("too many parameters (limit {MAX_PARAMS})")));
                }
                if out.iter().any(|(o, _)| *o == k) {
                    return Err(serde::de::Error::custom(format!("parameter `{k}` is declared twice")));
                }
                out.push((k, v));
            }
            Ok(out)
        }
    }
    d.deserialize_any(V)
}

fn params_as_object<S: serde::Serializer>(p: &[(String, ParamSpec)], s: S) -> std::result::Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut m = s.serialize_map(Some(p.len()))?;
    for (k, v) in p {
        m.serialize_entry(k, v)?;
    }
    m.end()
}

/// Whether `id` is a valid module id: 1 to [`MAX_ID_LEN`] characters from `A-Z a-z 0-9 . _ -`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_LEN && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `[A-Za-z0-9_]{1,64}`, not starting with `_` and not a key the commands and forms use
/// themselves.
fn valid_key(k: &str) -> bool {
    (1..=64).contains(&k.len())
        && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !k.starts_with('_')
        && !matches!(k, "id" | "ids" | "params" | "preview" | "glyphs" | "layers")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"{"id":"org.example.tone","name":"Tone","version":"1.2","kind":"reporter",
        "params":{"amount":{"type":"number","min":0,"max":100,"default":50},
                  "mode":{"type":"choice","options":["soft","hard"],"default":"hard"},
                  "mono":{"type":"bool"},"seed":{"type":"int","min":0,"max":9}}}"#;

    #[test]
    fn parses_keeps_param_order_and_resolves_parameters() {
        let m = Manifest::parse(FULL.as_bytes()).unwrap();
        assert_eq!((m.id.as_str(), m.kind), ("org.example.tone", Kind::Reporter));
        assert_eq!(m.params.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), ["amount", "mode", "mono", "seed"]);
        assert_eq!(m.params_doc(), r#"{amount: 0..100 (50), mode: "soft"|"hard" (hard), mono: bool (false), seed: int 0..9 (0)}"#);
        assert_eq!(Value::Object(m.defaults()), json!({"amount": 50.0, "mode": "hard", "mono": false, "seed": 0}));
        let p = m.validate_params(&json!({"amount": 500, "seed": 3.6, "junk": 1})).unwrap();
        assert_eq!(Value::Object(p), json!({"amount": 100.0, "mode": "hard", "mono": false, "seed": 4}));
        for bad in [json!({"amount": "lots"}), json!({"mode": "medium"}), json!({"mono": 1}), json!([1, 2]), json!(3)] {
            assert!(matches!(m.validate_params(&bad), Err(Error::Params(_))), "{bad}");
        }
        assert!(m.validate_params(&Value::Null).is_ok());
        // The manifest round-trips through its JSON form with the order kept.
        let again = Manifest::parse(&serde_json::to_vec(&m).unwrap()).unwrap();
        assert_eq!(again, m);
    }

    #[test]
    fn rejects_bad_manifests() {
        let k = r#""kind":"filter""#;
        for bad in [
            String::new(),
            "[]".into(),
            "not json".into(),
            format!(r#"{{"id":"","name":"x",{k}}}"#),
            format!(r#"{{"id":"a b","name":"x",{k}}}"#),
            format!(r#"{{"id":"a","name":"",{k}}}"#),
            r#"{"id":"a","name":"x","kind":"panel"}"#.into(),
            r#"{"id":"a","name":"x"}"#.into(),
            format!(r#"{{"id":"a","name":"x",{k},"capabilities":["files"]}}"#),
            format!(r#"{{"id":"a","name":"x",{k},"params":{{"r":{{"type":"number","min":5,"max":1,"default":1}}}}}}"#),
            format!(r#"{{"id":"a","name":"x",{k},"params":{{"r":{{"type":"number","min":0,"max":1,"default":7}}}}}}"#),
            format!(r#"{{"id":"a","name":"x",{k},"params":{{"r":{{"type":"int","min":0,"max":9,"default":-1}}}}}}"#),
            format!(r#"{{"id":"a","name":"x",{k},"params":{{"r":{{"type":"int","min":0,"max":9007199254740993}}}}}}"#),
            format!(r#"{{"id":"a","name":"x",{k},"params":{{"r":{{"type":"number","min":0,"max":1e999,"default":0}}}}}}"#),
            format!(r#"{{"id":"a","name":"x",{k},"params":{{"c":{{"type":"choice","options":[]}}}}}}"#),
            format!(r#"{{"id":"a","name":"x",{k},"params":{{"__x":{{"type":"bool"}}}}}}"#),
            format!(r#"{{"id":"a","name":"x",{k},"params":{{"glyphs":{{"type":"bool"}}}}}}"#),
            format!(r#"{{"id":"a","name":"x",{k},"params":{{"a":{{"type":"bool"}},"a":{{"type":"bool"}}}}}}"#),
            format!(r#"{{"id":"a","name":"x",{k},"params":[1]}}"#),
        ] {
            assert!(matches!(Manifest::parse(bad.as_bytes()), Err(Error::Manifest(_))), "{bad}");
        }
        assert!(Manifest::parse(&[0xff, 0xfe]).is_err());
        assert!(Manifest::parse(&vec![b' '; MAX_MANIFEST_BYTES + 1]).is_err());
    }
}
