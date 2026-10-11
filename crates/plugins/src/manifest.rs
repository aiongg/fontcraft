//! The module manifest (`fc_manifest`) and its parameter schema (M13.2 fills the bodies, ported
//! from VectorCraft's `crates/plugins/src/manifest.rs` with credit in the header and `NOTICE`).
//!
//! The manifest is UTF-8 JSON of at most [`MAX_MANIFEST_BYTES`]:
//! `{"id", "name", "version", "kind", "author", "description", "params": {name: spec, …},
//! "capabilities": []}`. `params` keeps declaration order (the form shows them in it); each spec
//! is one of the sibling apps' four types ([`ParamSpec`]). `capabilities` must be empty in ABI v1.
//! The rows served: `extensions.sandboxCapabilities`, `plugin.kind.filter`,
//! `plugin.kind.reporter`, `plugin.kind.check`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub use crate::error::{MAX_ID_LEN, MAX_MANIFEST_BYTES, MAX_PARAMS};
use crate::{Error, Result};

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
    /// A whole number in `min..=max`.
    Int { min: i64, max: i64, default: i64 },
    /// A checkbox.
    Bool { default: bool },
    /// One of `options` (a pop-up menu); `default` is one of them, the first when omitted.
    Choice { options: Vec<String>, default: Option<String> },
}

impl ParamSpec {
    /// The value a parameter takes when it is not given.
    pub fn default_value(&self) -> Value {
        // M13.2: as VectorCraft's.
        Value::Null
    }
}

/// A module's manifest, validated: the id is `[A-Za-z0-9._-]{1,64}`, the name 1 to 64 printable
/// characters, ranges finite and ordered, choices non-empty, `capabilities` empty.
///
/// M13.2 adds `Serialize`/`Deserialize` with the order-keeping `params` form.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    /// Stable id; reverse-DNS style is recommended (`org.example.shift`).
    pub id: String,
    /// The menu label.
    pub name: String,
    pub version: String,
    pub kind: Kind,
    pub author: String,
    pub description: String,
    /// Parameters in declaration (form) order.
    pub params: Vec<(String, ParamSpec)>,
    /// Requested host capabilities. ABI v1 grants none, so a non-empty list is refused.
    pub capabilities: Vec<String>,
}

impl Manifest {
    /// Parses and validates manifest JSON of at most [`MAX_MANIFEST_BYTES`].
    pub fn parse(bytes: &[u8]) -> Result<Manifest> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(Error::Manifest(format!("manifest is {} bytes (limit {MAX_MANIFEST_BYTES})", bytes.len())));
        }
        // M13.2: parse and validate.
        Err(Error::Unsupported("module manifests".into()))
    }

    /// Checks `params` against the schema and returns the full parameter object: missing keys
    /// take their defaults, numbers are clamped into range (whole numbers rounded), unknown keys
    /// are dropped; a value of the wrong type, or `params` not an object or null, is an
    /// [`Error::Params`].
    pub fn validate_params(&self, params: &Value) -> Result<Map<String, Value>> {
        let _ = params;
        // M13.2: as VectorCraft's `resolve_params`.
        Err(Error::Unsupported("module parameters".into()))
    }

    /// The parameters documented the way command parameters are
    /// (`{amount: 0..100 (50), mode: "soft"|"hard" (soft), mono: bool (false)}`), for the
    /// Command Console, MCP and the Actions dialog.
    pub fn params_doc(&self) -> String {
        // M13.2.
        "{}".into()
    }

    /// Every parameter at its default value.
    pub fn defaults(&self) -> Map<String, Value> {
        // M13.2.
        Map::new()
    }
}

/// Whether `id` is a valid module id: 1 to [`MAX_ID_LEN`] characters from `A-Z a-z 0-9 . _ -`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_LEN && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}
