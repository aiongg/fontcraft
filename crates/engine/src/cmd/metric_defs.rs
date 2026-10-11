//! Metric and stem definitions: `font.metrics.list/add/set/remove`, `font.master.setMetric`, `font.stems.list/add/set/remove`, `font.master.setStem`. Adding a definition adds a value to every master; removing one removes them.
//!
//! Package M7.6 (`docs/plans/M7.md` § 4) fills [`specs`].

use super::*;

pub fn specs() -> Vec<CommandSpec> {
    // M7.6: the commands.
    Vec::new()
}
