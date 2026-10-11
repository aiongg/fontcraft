//! FontCraft interpolation (L2): the designspace (axes, internal and external coordinates,
//! normalization, the origin, virtual masters, variable font settings), the sources each glyph
//! interpolates from, the compatibility check, interpolated layers and static instance fonts, and
//! the ranges and overlay of alternate layers. Pure functions over a [`fontcraft_model::Font`];
//! no I/O. Plan: `docs/plans/M9.md` § 3 and § 4. Behaviour notes: `docs/spec/10-interpolation.md`;
//! rows in `spec/inventory/10-interpolation.toml`.
//!
//! | Module | Package | What it gives |
//! |---|---|---|
//! | [`designspace`] | M9.2 | axes, masters, virtual masters, the origin, coordinate conversion, settings |
//! | [`mapping`] | M9.2 | axis mapping edits and checks, converting a value between spaces |
//! | [`params`] | M9.2 | reading and writing Axis Location, Axis Mappings, Virtual Master, Disable Masters |
//! | [`sources`] | M9.3 | a glyph's master, intermediate and alternate layers with their locations |
//! | [`compat`] | M9.3 | the compatibility check and its scope |
//! | [`rules`] | M9.3, M9.4 | alternate range sets, inclusive bounds |
//! | [`instance`] | M9.4 | interpolated layers, static instance fonts, master weights, a model cache |
//! | [`overlay`] | M9.8 | non-overlapping feature-variation records from alternate boxes |
//! | [`labels`] | M9.16 | the brace and bracket labels of special layers |
//!
//! Who calls what: the engine keeps one [`Designspace`] and one compatibility table per font
//! revision (`fontcraft_engine::interp_cache`); commands use every module; the compiler reads
//! [`instance::instance_font`] for static instances and [`designspace`], [`sources`] and
//! [`overlay`] for variable fonts (`fontcraft_compile::var`). The variation model itself is
//! `fontcraft_geom::variation`.
//!
//! Limits are named in [`error`]; every function returns [`InterpError`] instead of panicking.
#![forbid(unsafe_code)]

pub mod compat;
pub mod designspace;
pub mod error;
pub mod instance;
pub mod labels;
pub mod mapping;
pub mod overlay;
pub mod params;
pub mod rules;
pub mod sources;

pub use compat::{AngleWarning, Compat, MasterPairs, Mismatch, MismatchKind};
pub use designspace::{AxisInfo, Designspace, Location, MasterInfo, NormLocation, Placement, Space, UserLocation, VirtualMasterInfo};
pub use error::{
    ANGLE_WARNING_DEGREES, InterpError, MAX_AXES, MAX_COORDINATE, MAX_GENERATED_INSTANCES, MAX_INSTANCES, MAX_MAPPING_POINTS, MAX_OVERLAY_RECORDS,
    MAX_OVERLAY_WORK, MAX_RANGE_SETS, MAX_SOURCES, MAX_VIRTUAL_MASTERS,
};
pub use instance::{InterpOptions, ModelCache, ModelKind};
pub use overlay::{AxisSpan, OverlayRecord, OverlayRule, RegionBox};
pub use rules::RangeSet;
pub use sources::{AlternateSet, GlyphSources, Source};

#[cfg(test)]
mod tests {
    use fontcraft_model::{Font, Instance};

    use crate::designspace::variable_settings;

    #[test]
    fn variable_settings_are_the_instances_of_kind_variable() {
        let mut font = Font::new("Settings");
        let setting = Instance { kind: Some("variable".to_string()), ..Instance::default() };
        font.instances = std::sync::Arc::new(vec![Instance::default(), setting]);
        assert_eq!(variable_settings(&font), vec![1]);
    }
}
