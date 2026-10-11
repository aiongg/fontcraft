//! The designspace of a font or of one variable font setting (M9.2).
//!
//! Rows: `axes.definition`, `axes.tagRules`, `axes.coordinatesAreFreeNumbers`,
//! `axes.linearSpread`, `axes.internalVersusExternal`, `axes.unusedAxesDropped`,
//! `masters.mustCoverDesignspace`, `masters.minimalSetup`, `masters.cornerAndIntermediateSetups`,
//! `variable.originMaster`, `variable.originIsFallback`, `variable.originNeedsMinimalSetup`,
//! `axisLocation.*`, `axisMappings.*`, `virtualMaster.*`, `variableFontSetting.disableMasters`,
//! `variableFontSetting.defaultWhenNone`, `instances.extrapolatedExcludedFromVariable`,
//! `font.axes`, `master.axesValues`, `instance.axesValues`, `instance.type`.
//!
//! Contract (`docs/plans/M9.md` § 3 › Design decisions):
//! - Coordinates are **internal** (design) everywhere in the model and in commands. External
//!   (user) coordinates appear only at the table boundary (`fvar`, `avar`, `STAT`, feature
//!   variations) and when a command asks for them. Normalized coordinates (−1…1, 0 at the
//!   origin) are what the variation model works in.
//! - The map from external to internal coordinates is the font's Axis Mappings parameter, else
//!   the masters' Axis Location parameters, else identity (`fontcraft_model::axes::mapping`
//!   derives it as the oracles do); with both present the oracles' precedence applies.
//! - The origin is the first master unless the Variable Font Origin parameter (or its older name)
//!   names another (`docs/plans/M9.md` § 7 question 3, decided: the handbook's rule). The
//!   difference from the oracles is recorded in `docs/spec/40-compile.md` § 4a.
//! - [`Designspace::for_setting`] applies a variable font setting: its Disable Masters list
//!   removes masters, and an axis on which every remaining master sits at the same value is
//!   dropped.
//! - Axis ranges cover the masters and the virtual masters; instances outside them are
//!   [`Placement::Outside`] (extrapolated: allowed for static instances, left out of variable
//!   fonts).
//! - Building a designspace checks every count against [`crate::error`]'s limits and refuses
//!   coordinates that are not finite. Pure; no I/O.

use fontcraft_model::axes::Mapping;
use fontcraft_model::{AxisRange, Font};

use crate::InterpError;

/// A location in internal (design) coordinates: one value per axis of the designspace, in its
/// axis order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Location(pub Vec<f64>);

/// A location in external (user) coordinates, one value per axis.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UserLocation(pub Vec<f64>);

/// A location in normalized coordinates: −1 at an axis minimum, 0 at the origin, 1 at the
/// maximum; beyond ±1 for an extrapolated location.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NormLocation(pub Vec<f64>);

/// The coordinate space a value is given in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    Internal,
    External,
}

/// One axis of a designspace.
#[derive(Clone, Debug, PartialEq)]
pub struct AxisInfo {
    /// The four-character OpenType tag.
    pub tag: String,
    pub name: String,
    pub hidden: bool,
    /// Minimum, origin and maximum in internal coordinates.
    pub internal: AxisRange,
    /// Minimum, origin and maximum in external coordinates.
    pub external: AxisRange,
    /// External to internal map of the axis (identity when the font sets none).
    pub map: Mapping,
    /// The axis's index in `Font::axes` (differs from its index here when a setting dropped an
    /// axis).
    pub font_axis: usize,
}

/// A master of the designspace.
#[derive(Clone, Debug, PartialEq)]
pub struct MasterInfo {
    pub id: String,
    pub name: String,
    /// Its index in `Font::masters`.
    pub index: usize,
    pub location: Location,
}

/// A virtual master: a location the interpolation treats as a master that has no layers of its
/// own, so intermediate layers on an axis with a single master have something to interpolate
/// from (`virtualMaster.*`).
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualMasterInfo {
    pub name: String,
    pub location: Location,
}

/// Where a location lies relative to the axis ranges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// On or within every axis range.
    Inside,
    /// Beyond the range of at least one axis: an extrapolated location.
    Outside,
}

/// The axes, masters and origin a font interpolates with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Designspace {
    axes: Vec<AxisInfo>,
    masters: Vec<MasterInfo>,
    virtual_masters: Vec<VirtualMasterInfo>,
    /// Index into `masters`.
    origin: Option<usize>,
    setting: Option<usize>,
}

impl Designspace {
    /// The designspace of the whole font: every axis, every master, the virtual masters and the
    /// origin. Refuses bad axis tags, duplicate tags, unusable mappings, coordinates that are not
    /// finite and counts beyond the limits.
    pub fn new(font: &Font) -> Result<Designspace, InterpError> {
        // M9.2
        let _ = font;
        Err(InterpError::NotYetImplemented("the designspace"))
    }

    /// The designspace of the variable font setting at `setting` (an index into
    /// `Font::instances` whose kind is `variable`): the masters its Disable Masters parameter
    /// names are removed, and axes on which the remaining masters all sit at one value are
    /// dropped.
    pub fn for_setting(font: &Font, setting: usize) -> Result<Designspace, InterpError> {
        // M9.2
        let _ = (font, setting);
        Err(InterpError::NotYetImplemented("the designspace of a variable font setting"))
    }

    /// The axes, in the font's axis order (less any a setting dropped).
    pub fn axes(&self) -> &[AxisInfo] {
        &self.axes
    }

    /// The axis with tag `tag` and its index here.
    pub fn axis(&self, tag: &str) -> Option<(usize, &AxisInfo)> {
        self.axes.iter().enumerate().find(|(_, a)| a.tag == tag)
    }

    /// The masters, in the font's order (less any a setting disabled).
    pub fn masters(&self) -> &[MasterInfo] {
        &self.masters
    }

    /// The master with id `id`.
    pub fn master(&self, id: &str) -> Option<&MasterInfo> {
        self.masters.iter().find(|m| m.id == id)
    }

    pub fn virtual_masters(&self) -> &[VirtualMasterInfo] {
        &self.virtual_masters
    }

    /// The origin master: the default of every axis, whose outlines a variable font stores.
    /// `None` only for a font without masters.
    pub fn origin(&self) -> Option<&MasterInfo> {
        self.origin.and_then(|i| self.masters.get(i))
    }

    /// The variable font setting this designspace was built for; `None` for the whole font.
    pub fn setting(&self) -> Option<usize> {
        self.setting
    }

    /// The internal location of the instance at `instance` in `Font::instances`, its
    /// coordinates matched to this designspace's axes.
    pub fn instance_location(&self, font: &Font, instance: usize) -> Result<Location, InterpError> {
        // M9.2
        let _ = (font, instance);
        Err(InterpError::NotYetImplemented("instance locations"))
    }

    /// `location` normalized for the variation model. Values beyond an axis range extrapolate.
    /// Refuses a location of the wrong length or with a value that is not finite.
    pub fn to_normalized(&self, location: &Location) -> Result<NormLocation, InterpError> {
        // M9.2
        let _ = location;
        Err(InterpError::NotYetImplemented("normalized coordinates"))
    }

    /// `location` in external coordinates, through each axis's map read backwards.
    pub fn to_external(&self, location: &Location) -> Result<UserLocation, InterpError> {
        // M9.2
        let _ = location;
        Err(InterpError::NotYetImplemented("external coordinates"))
    }

    /// `location` in internal coordinates, through each axis's map.
    pub fn to_internal(&self, location: &UserLocation) -> Result<Location, InterpError> {
        // M9.2
        let _ = location;
        Err(InterpError::NotYetImplemented("internal coordinates"))
    }

    /// Whether `location` is within every axis range (both ends included).
    pub fn classify(&self, location: &Location) -> Placement {
        // M9.2
        let _ = location;
        Placement::Inside
    }
}

/// The indices in `Font::instances` of the variable font settings, in order. A font without any
/// exports one variable font from the whole designspace (`variableFontSetting.defaultWhenNone`).
pub fn variable_settings(font: &Font) -> Vec<usize> {
    font.instances.iter().enumerate().filter(|(_, i)| i.kind.as_deref() == Some("variable")).map(|(n, _)| n).collect()
}
