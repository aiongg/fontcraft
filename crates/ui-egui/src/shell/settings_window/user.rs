//! Settings › User Settings: import behaviour, the interface language, versioned saving and the
//! crash-report choice with its contact fields (`settings.userSettings`). Every control writes
//! its `user.*` setting with `settings.set`. Only Disable Automatic Alignment for Imported Files
//! changes what FontCraft does today; the others are stored for the features that will read them.

use super::{checkbox, note, text_field};
use crate::app::FontcraftApp;
use crate::panels::Pending;

/// The section's body.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let gap = app.tokens.dialog_spacing;
    checkbox(app, ui, pending, "user.keepGlyphNamesFromImportedFiles", "Keep Glyph Names from Imported Files");
    note(app, ui, "Not used yet: opened files keep their glyph names either way.");
    checkbox(app, ui, pending, "user.disableAutomaticAlignmentForImportedFiles", "Disable Automatic Alignment for Imported Files");
    note(app, ui, "Components of an opened font stay where the file places them.");
    ui.add_space(gap);
    checkbox(app, ui, pending, "user.disableLocalization", "Disable Localization");
    note(app, ui, "Keeps the interface in English. Takes effect after FontCraft restarts.");
    checkbox(app, ui, pending, "user.useVersions", "Use Versions");
    note(app, ui, "Not used yet: FontCraft writes a file only when you save it and keeps no earlier versions.");
    ui.add_space(gap);
    checkbox(app, ui, pending, "user.sendCrashReports", "Send Crash Reports");
    text_field(app, ui, pending, "user.crashReportName", "Name");
    text_field(app, ui, pending, "user.crashReportEmail", "Email");
    note(app, ui, "Not used yet: FontCraft sends no crash reports. The choice and the contact details are kept.");
}
