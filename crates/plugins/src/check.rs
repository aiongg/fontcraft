//! Check modules' findings (M13.2 fills the parser; M13.5 joins them to `audit.run` under the
//! check ids `check.plugin.<module>.<name>`).
//!
//! Output: `{"findings": [{"name", "severity", "message", "position"?: [x, y]}]}`, at most
//! [`MAX_FINDINGS`] per layer.

use serde::{Deserialize, Serialize};

pub use crate::error::MAX_FINDINGS;
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
    let _ = bytes;
    // M13.2.
    Err(Error::Unsupported("check output".into()))
}
