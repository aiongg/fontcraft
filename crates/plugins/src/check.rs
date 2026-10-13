//! Check modules' findings (M13.5 joins them to `audit.run` under the check ids
//! `check.plugin.<module>.<name>`).
//!
//! Output: `{"findings": [{"name", "severity", "message", "position"?: [x, y]}]}`, at most
//! [`MAX_FINDINGS`] per layer; `{"error": "…"}` reports a failure. Names follow the module id
//! rule (`[A-Za-z0-9._-]`, at most `MAX_ID_LEN`), messages are 1 to [`MAX_MESSAGE_CHARS`]
//! characters, positions are within the model's `MAX_COORDINATE`.

use fontcraft_model::MAX_COORDINATE;
use serde::{Deserialize, Serialize};

use crate::capped::{capped, with_budget};
pub use crate::error::{MAX_FINDINGS, MAX_MESSAGE_CHARS};
use crate::layer_json::cut;
use crate::manifest::valid_id;
use crate::{Error, Result};

/// How serious a finding is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// One problem a check module found on a layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// The check's own name for the problem (`[A-Za-z0-9._-]`); the audit id is
    /// `check.plugin.<module>.<name>`.
    pub name: String,
    pub severity: Severity,
    pub message: String,
    /// Where on the layer, in font units; the canvas marks it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[f64; 2]>,
}

/// Parses a check module's output, refusing more than [`MAX_FINDINGS`] findings, non-finite
/// positions and over-long strings.
pub fn parse_findings(bytes: &[u8]) -> Result<Vec<Finding>> {
    #[derive(Deserialize)]
    struct Out {
        #[serde(default)]
        error: Option<String>,
        #[serde(default, deserialize_with = "capped")]
        findings: Vec<Finding>,
    }
    let (out, over) = with_budget(MAX_FINDINGS, || serde_json::from_slice::<Out>(bytes));
    if over {
        return Err(Error::Output(format!("more than {MAX_FINDINGS} findings")));
    }
    let out = out.map_err(|e| Error::Output(e.to_string()))?;
    if let Some(e) = out.error {
        return Err(Error::Failed(cut(&e, MAX_MESSAGE_CHARS)));
    }
    for f in &out.findings {
        if !valid_id(&f.name) {
            return Err(Error::Output(format!("finding name {:?}: use 1 to 64 characters from A-Z a-z 0-9 . _ -", cut(&f.name, 64))));
        }
        let chars = f.message.chars().count();
        if chars == 0 || chars > MAX_MESSAGE_CHARS {
            return Err(Error::Output(format!("finding `{}`: the message must be 1 to {MAX_MESSAGE_CHARS} characters", f.name)));
        }
        if f.position.is_some_and(|p| !p.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE)) {
            return Err(Error::Output(format!("finding `{}`: the position is beyond ±{MAX_COORDINATE}", f.name)));
        }
    }
    Ok(out.findings)
}
