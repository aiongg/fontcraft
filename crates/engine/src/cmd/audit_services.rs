//! The engine's side of the audit services (M12.44; `fontcraft_audit::AuditServices`): in-memory
//! compiles, the TTX round trip, shaping, rasterising and the master-compatibility report. Until
//! M9's compatibility routine lands, the report comes from a plain structural comparison written
//! here (missing glyph, path count, node count, node type, component and anchor mismatch).
//!
//! The module registers no command of its own; `audit.run` (in `audit.rs`) hands
//! [`EngineServices`] to the audit's runners.

use fontcraft_audit::AuditServices;

use super::CommandSpec;

/// The services the engine gives the audit. Every service answers "not available" until M12.44
/// fills it, which makes the checks that need it skip with the reason.
#[derive(Clone, Copy, Debug, Default)]
pub struct EngineServices;

// M12.44: compile_instances, ttx_round_trip, shape, compatibility, rasterise.
impl AuditServices for EngineServices {}

/// No commands: the services are reached through `audit.run`.
pub fn specs() -> Vec<CommandSpec> {
    Vec::new()
}
