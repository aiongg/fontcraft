//! The engine's side of the audit services (M12.44; `fontcraft_audit::AuditServices`): in-memory
//! compiles, the TTX round trip, shaping, rasterising and the master-compatibility report. The
//! report is the audit's structural comparison (`fontcraft_audit::structural_report`: missing
//! glyph, path count, node count, node type, component and anchor mismatch).
//!
//! The module registers no command of its own; `audit.run` (in `audit.rs`) hands
//! [`EngineServices`] to the audit's runners.

use fontcraft_audit::{AuditServices, CompatibilityReport, Unavailable};
use fontcraft_model::Font;

use super::CommandSpec;

/// The services the engine gives the audit. A service not filled yet answers "not available",
/// which makes the checks that need it skip with the reason.
#[derive(Clone, Copy, Debug, Default)]
pub struct EngineServices;

// M12.44: compile_instances, ttx_round_trip, shape, rasterise.
impl AuditServices for EngineServices {
    fn compatibility(&self, font: &Font) -> Result<CompatibilityReport, Unavailable> {
        Ok(fontcraft_audit::structural_report(font))
    }
}

/// No commands: the services are reached through `audit.run`.
pub fn specs() -> Vec<CommandSpec> {
    Vec::new()
}
