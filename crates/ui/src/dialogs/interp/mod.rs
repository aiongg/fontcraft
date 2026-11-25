//! The masters and interpolation dialogs: the Masters tab's dialogs ([`masters`]), Axis Location and Virtual Master
//! ([`location`]), and the interpolation commands of the Path, Glyph and Font menus ([`variation`]). All are specs.

pub mod fonts;
pub mod location;
pub mod masters;
#[cfg(test)]
mod tests;
pub mod variation;

use fontcraft_engine::{DocState, Session};
use serde_json::{Map, Value};

use super::fields::MAX_CHOICES;
use super::spec::DialogSpec;

pub const SPECS: &[DialogSpec] = &[
    location::AXIS_LOCATION_DIALOG,
    location::VIRTUAL_MASTER_DIALOG,
    masters::ADD_OTHER_FONT,
    masters::ICON_PICKER,
    masters::ADD_METRIC,
    masters::METRIC_EDITOR,
    masters::STEM_SCOPE,
    variation::INTERPOLATE_WITH_BACKGROUND,
    variation::ADJUST_BY_VARIATION,
    variation::SET_MASTER_LOCATIONS,
    variation::ADD_VARIATION,
    variation::MERGE_FONTS,
    fonts::INTERPOLATE_FONTS,
    fonts::GENERATE_INSTANCES,
];

/// The open documents `wanted` takes, as the commands name them.
#[derive(Debug, Default)]
pub struct OpenFonts {
    /// References: the path a document was opened from, else its title. Each names its document.
    pub choices: Vec<String>,
    pub labels: Vec<String>,
    /// A document was left out: the commands find the first one of a title or path, and that is another.
    pub left_out: bool,
}

impl OpenFonts {
    pub fn of(session: &Session, wanted: impl Fn(&DocState) -> bool) -> OpenFonts {
        let named = |r: &str| {
            session.documents().iter().find(|d| d.title == r || d.path.as_deref().is_some_and(|p| p == std::path::Path::new(r))).map(|d| d.id)
        };
        let mut out = OpenFonts::default();
        for d in session.documents().iter().filter(|d| wanted(d)) {
            let reference = d.path.as_ref().map_or_else(|| d.title.clone(), |p| p.display().to_string());
            if named(&reference) != Some(d.id) {
                out.left_out = true;
            } else if out.choices.len() < MAX_CHOICES {
                out.choices.push(reference);
                out.labels.push(d.title.clone());
            }
        }
        // Documents of one title are told apart by their path.
        let repeated: Vec<bool> = out.labels.iter().map(|l| out.labels.iter().filter(|other| *other == l).count() > 1).collect();
        for ((label, reference), repeated) in out.labels.iter_mut().zip(&out.choices).zip(repeated) {
            if repeated {
                label.clone_from(reference);
            }
        }
        out
    }

    /// What a dialog says when a document was left out, after a sentence; empty otherwise.
    pub fn note(&self) -> &'static str {
        if self.left_out { ". A font with the title of another that was never saved is not listed: save it first" } else { "" }
    }
}

/// A dialog whose fields keep no step with each other.
pub fn no_link(_: &mut Map<String, Value>, _: &str) {}
