//! The implicit features in the compiler's neutral form (M8.7): the generated kerning
//! ([`crate::kern_feature`]'s lookups) and mark attachment ([`crate::marks::generate_marks`]'s
//! [`GeneratedMarks`]) turned into [`fontcraft_fea::GeneratedLookups`] for fea-rs to place at
//! `# Automatic Code` or after the code.
//!
//! Rows: `compile.featureWriterModes`, `compile.skipFeatures`, `fontInfo.features.implicit`,
//! `fontInfo.features.automaticCodePlaceholder`, `implicit.markerMissing`.
//!
//! M8.7 also joins the kern writer's scripts with the code's language systems and the scripts
//! its substitutions reach, ported from fontc's `properties.rs`.

use fontcraft_fea::{GeneratedLookups, Tag};

use crate::marks::{GeneratedMarks, MarkRequest};
use crate::{FeatureError, KernRequest};

/// What the implicit features are generated from. `None` leaves that writer out (its tags are
/// off, or the source has no data for it).
pub struct ImplicitRequest<'a> {
    pub kern: Option<&'a KernRequest<'a>>,
    pub marks: Option<&'a MarkRequest<'a>>,
    /// The language systems the feature code declares, as (script, language).
    pub language_systems: &'a [(Tag, Tag)],
}

/// The implicit features of one master, with the warnings of the writers.
#[derive(Debug, Default)]
pub struct Implicit {
    pub lookups: GeneratedLookups,
    pub warnings: Vec<String>,
}

/// Runs the kern and mark writers and returns their lookups in neutral form, kerning lookups
/// after the cursive ones and before the other mark lookups, as the reference compilers order
/// them.
pub fn generated(req: &ImplicitRequest) -> Result<Implicit, FeatureError> {
    // M8.7
    let _ = req;
    Ok(Implicit::default())
}

/// The mark writer's output in neutral form: one lookup per mark lookup, registered for each of
/// its language systems under its feature; positions as fixed [`fontcraft_fea::Metric`]s; the
/// `GDEF` classes and carets carried over.
pub fn from_marks(marks: &GeneratedMarks) -> GeneratedLookups {
    // M8.7
    let _ = marks;
    GeneratedLookups::default()
}
