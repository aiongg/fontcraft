//! Master compatibility checks: the structural report from [`crate::AuditServices::compatibility`] and the heuristic tests ported from fontTools `varLib.interpolatable` (MIT) (M12.53). Runners take the context of their scope ([`crate::GlyphCtx`],
//! [`crate::SourceCtx`], [`crate::BinaryCtx`], [`crate::FamilyCtx`]), the thresholds and a
//! [`crate::Sink`]; their registry entries go in `registry/`.

// M12.53
