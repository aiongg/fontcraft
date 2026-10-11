//! The text-mode measurement line's shell commands: View › Show Measurement Line and the line's
//! height in each tab. The canvas draws it (`canvas::measurement_line`).

use std::collections::BTreeMap;

use fontcraft_model::{Font, MAX_COORDINATE, metric};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::state::Tab;

pub const COMMANDS: &[UiCommand] = &[
    (
        "view.showMeasurementLine",
        "Show Measurement Line",
        &["View"],
        "",
        "{on?: bool} show the measurement line in text mode (omitted: toggle): a line at an adjustable height with each glyph's sidebearings measured on it, and thin lines at the edges of every advance. It shows only while the Text tool is active",
    ),
    (
        "view.setMeasurementLineHeight",
        "Set Measurement Line Height",
        &[],
        "",
        "{tab?: id, y: number (from -32768 to 32767)} put the tab's measurement line at this height above the baseline, in font units → {tab, y}",
    ),
];

/// Most tabs whose line height is kept; past it, the heights of closed tabs are dropped.
const KEPT_HEIGHTS: usize = 256;

/// Whether the measurement line shows, and its height in each tab that has moved it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MeasurementLine {
    pub on: bool,
    /// Tab id → height above the baseline. A tab without an entry uses [`default_height`].
    pub heights: BTreeMap<u64, f64>,
}

impl MeasurementLine {
    /// The height of `tab`'s line in `font`.
    pub fn height(&self, font: &Font, tab: &Tab) -> f64 {
        self.heights.get(&tab.id).copied().filter(|y| y.is_finite()).unwrap_or_else(|| default_height(font, tab))
    }
}

/// Half the x-height of the tab's master.
pub fn default_height(font: &Font, tab: &Tab) -> f64 {
    let upm = f64::from(font.units_per_em);
    let x_height = crate::canvas::view::tab_master(font, tab).map_or(upm * 0.5, |m| font.metric_pos(m, metric::X_HEIGHT, upm * 0.5));
    if x_height.is_finite() { x_height / 2.0 } else { 0.0 }
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "view.showMeasurementLine" => {
            let line = &mut app.state.panels.measurement;
            line.on = args.toggle(line.on)?;
            Ok(json!({"on": line.on}))
        }
        "view.setMeasurementLineHeight" => {
            let index = super::view::target_tab(app, args)?;
            let y = args.number("y")?.ok_or_else(|| args.err("`y` (height above the baseline) is required"))?;
            if y.abs() > MAX_COORDINATE {
                return Err(args.err(format!("`y` must lie within ±{MAX_COORDINATE}")));
            }
            let tab = app.state.tabs.get(index).ok_or_else(|| args.err("no such tab"))?.id;
            let open: Vec<u64> = app.state.tabs.iter().map(|t| t.id).collect();
            let heights = &mut app.state.panels.measurement.heights;
            if heights.len() >= KEPT_HEIGHTS && !heights.contains_key(&tab) {
                heights.retain(|id, _| open.contains(id));
            }
            heights.insert(tab, y);
            Ok(json!({"tab": tab, "y": y}))
        }
        _ => Err(args.err("unknown command")),
    }
}
