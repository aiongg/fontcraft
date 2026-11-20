//! Live audit and the audit markers of Edit View (`audit.toggleLiveAudit`, `audit.toggleMarkers`).
//!
//! The audit itself is the engine's (`audit.run`, `audit.results`, `audit.fix`). The shell keeps
//! the two switches and the results the canvas marks for the edited layer
//! ([`crate::canvas::audit_markers`] fills and draws them).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;

pub const COMMANDS: &[UiCommand] = &[
    (
        "audit.toggleLiveAudit",
        "Live Audit",
        &["View", "Audit"],
        "",
        "{on?: bool} re-run the outlines profile on the edited layer after each change to the document while it is on (omitted: toggle) → {on}",
    ),
    (
        "audit.toggleMarkers",
        "Show Audit Markers",
        &[],
        "",
        "{on?: bool} mark the nodes, segments and paths of the current audit results of the edited layer in Edit View (omitted: toggle) → {on}",
    ),
];

/// The audit switches, and the results the canvas marks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AuditState {
    /// Re-run the outlines profile on the edited layer after each change.
    pub live: bool,
    /// Draw markers for the current results of the edited layer.
    pub markers: bool,
    /// The results for the edited layer, and what they were read for.
    #[serde(skip)]
    pub shown: Shown,
    /// How many times live audit has run `audit.run`.
    #[serde(skip)]
    pub runs: u64,
    /// Read the kept results again at the next frame, though nothing was edited.
    #[serde(skip)]
    pub reread: bool,
}

impl Default for AuditState {
    fn default() -> Self {
        Self { live: false, markers: true, shown: Shown::default(), runs: 0, reread: false }
    }
}

/// What the results were read for: the document, its revision, the glyph and the layer, and
/// whether live audit produced them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub doc: u64,
    pub revision: u64,
    pub glyph: String,
    pub layer: String,
    pub live: bool,
}

/// The results the canvas marks, with the key they belong to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shown {
    pub key: Option<Key>,
    pub markers: Vec<Marker>,
}

/// How serious a result is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Fail,
    Warn,
    Info,
}

/// One result with a place on the edited layer.
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    /// The result id, for `audit.fix {results}`.
    pub id: String,
    pub check: String,
    pub severity: Severity,
    pub message: String,
    pub shapes: Vec<usize>,
    pub points: Vec<[usize; 2]>,
    /// `[shape, end node]` of each segment.
    pub segments: Vec<[usize; 2]>,
    /// The spot the result points at, in font units.
    pub point: Option<[f64; 2]>,
    /// `[x0, y0, x1, y1]` in font units.
    pub bounds: Option<[f64; 4]>,
}

/// Most results one layer shows markers for.
pub const MAX_MARKERS: usize = 1_000;
/// Most addresses one marker keeps of each kind.
const MAX_ADDRESSES: usize = 1_000;

impl Marker {
    /// A result of the audit result document (`docs/spec/21-qa.md` § 4). `None` for a result
    /// that does not read as one.
    pub fn from_result(r: &Value) -> Option<Self> {
        let severity = match r.get("severity")?.as_str()? {
            "fail" => Severity::Fail,
            "warn" => Severity::Warn,
            _ => Severity::Info,
        };
        let loc = r.get("location")?;
        let pairs = |key: &str| -> Vec<[usize; 2]> {
            loc.get(key)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .take(MAX_ADDRESSES)
                        .filter_map(|p| {
                            let p = p.as_array()?;
                            let s = usize::try_from(p.first()?.as_u64()?).ok()?;
                            let n = usize::try_from(p.get(1)?.as_u64()?).ok()?;
                            Some([s, n])
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let numbers = |key: &str| -> Option<Vec<f64>> {
            let list = loc.get(key)?.as_array().filter(|a| a.len() <= 4)?;
            list.iter().map(|v| v.as_f64().filter(|n| n.is_finite())).collect()
        };
        let point = numbers("point").and_then(|v| Some([*v.first()?, *v.get(1)?]));
        let bounds = numbers("bounds").and_then(|v| Some([*v.first()?, *v.get(1)?, *v.get(2)?, *v.get(3)?]));
        let shapes = loc
            .get("shapes")
            .and_then(Value::as_array)
            .map(|a| a.iter().take(MAX_ADDRESSES).filter_map(|s| usize::try_from(s.as_u64()?).ok()).collect())
            .unwrap_or_default();
        Some(Self {
            id: r.get("id")?.as_str()?.to_string(),
            check: r.get("check").and_then(Value::as_str).unwrap_or_default().to_string(),
            severity,
            message: r.get("message").and_then(Value::as_str).unwrap_or_default().to_string(),
            shapes,
            points: pairs("points"),
            segments: pairs("segments"),
            point,
            bounds,
        })
    }
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    let audit = &mut app.state.audit;
    match id {
        "audit.toggleLiveAudit" => {
            audit.live = args.toggle(audit.live)?;
            Ok(json!({"on": audit.live}))
        }
        "audit.toggleMarkers" => {
            audit.markers = args.toggle(audit.markers)?;
            // An `audit.run` from elsewhere does not change the revision; showing the markers
            // again picks it up.
            audit.reread = true;
            Ok(json!({"on": audit.markers}))
        }
        other => Err(format!("unknown command `{other}`")),
    }
}
