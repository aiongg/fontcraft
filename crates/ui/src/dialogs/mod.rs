//! The dialogs the app opens on the window's `DialogHost`.
//!
//! The shell's body calls [`run`] every pass with the host. Each module opens its dialog when the app state asks
//! for it, takes the answer on a later pass and turns it into messages or app state. A module adds one line to
//! [`MODULES`].

pub mod custom_parameter_picker;
pub mod field_picker;
pub mod kerning_audit;
pub mod kerning_questions;
pub mod os2_bits;
pub mod panose;
mod unsaved;

use craft::ui::dialog::DialogHost;

use crate::app::FontCraftApp;

/// Opens and answers one kind of dialog.
pub type Module = fn(&mut DialogHost, &mut FontCraftApp);

/// Every dialog module.
pub const MODULES: &[Module] =
    &[unsaved::run, field_picker::run, custom_parameter_picker::run, os2_bits::run, panose::run, kerning_questions::run, kerning_audit::run];

/// Runs every dialog module for this pass.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    for module in MODULES {
        module(host, app);
    }
}
