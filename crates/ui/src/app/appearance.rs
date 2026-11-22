//! Which appearance each part of the window is drawn in (`appearance.colorsPerSystemAppearance`).
//!
//! The window follows the system's light or dark mode through the theme: [`root`] is the one place that choice is
//! set. Font View and Edit View content follow it too, unless Settings › Appearance › Always use Light Mode keeps them
//! light under a dark window; then the view is shown in a light [`ThemeScope`] and the canvas takes the light colour
//! set. The window chrome, panels and sidebars stay in the system's mode.

use craft::theme::{Mode, ModeChoice};
use craft::ui::runtime::env::{RootEnv, ThemeScope};
use fontcraft_engine::settings::UserSettings;

/// Always use Light Mode for Font View.
pub const FONT_VIEW_LIGHT: &str = "appearance.alwaysLightModeFontView";
/// Always use Light Mode for Edit View.
pub const EDIT_VIEW_LIGHT: &str = "appearance.alwaysLightModeEditView";

/// The root the window's theme resolves from: the system's light or dark mode.
pub fn root() -> RootEnv {
    RootEnv::new().mode(ModeChoice::System)
}

/// Whether `root` resolves to the dark mode on `ctx`'s pass.
pub fn system_dark(root: &RootEnv, ctx: &egui::Context) -> bool {
    root.resolved(ctx).mode == Mode::Dark
}

/// The appearance of the window and of the two document views.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Appearance {
    /// The window is drawn dark.
    pub dark: bool,
    /// Always use Light Mode for Font View.
    pub font_view_light: bool,
    /// Always use Light Mode for Edit View.
    pub edit_view_light: bool,
}

impl Appearance {
    /// The appearance under `settings` while the window is dark when `dark`.
    pub fn of(settings: &UserSettings, dark: bool) -> Appearance {
        let on = |id: &str| settings.get_bool(id).unwrap_or(false);
        Appearance { dark, font_view_light: on(FONT_VIEW_LIGHT), edit_view_light: on(EDIT_VIEW_LIGHT) }
    }

    /// Whether Edit View's content, the canvas, is drawn dark: the colour set it takes.
    pub fn edit_view_dark(self) -> bool {
        self.dark && !self.edit_view_light
    }

    /// Whether Font View's content is drawn dark.
    pub fn font_view_dark(self) -> bool {
        self.dark && !self.font_view_light
    }

    /// The scope Font View's content is shown in: light while it is kept light under a dark window, else none.
    pub fn font_view_scope(self) -> Option<ThemeScope> {
        (self.dark && self.font_view_light).then(light)
    }

    /// The scope Edit View's content is shown in: light while it is kept light under a dark window, else none.
    pub fn edit_view_scope(self) -> Option<ThemeScope> {
        (self.dark && self.edit_view_light).then(light)
    }
}

fn light() -> ThemeScope {
    ThemeScope::new().mode(Mode::Light)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use fontcraft_engine::Session;
    use serde_json::json;

    use super::*;
    use crate::app::{APP, FontCraftApp, Msg};
    use crate::canvas::tokens::Tokens;

    fn app() -> FontCraftApp {
        FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()))
    }

    fn set(app: &mut FontCraftApp, id: &str, value: serde_json::Value) {
        app.run_now(Msg::run("settings.set", json!({ "id": id, "value": value })), false).unwrap();
    }

    /// The canvas takes the dark set under a dark window, and the light set again while Always use Light Mode for
    /// Edit View is on; the window itself stays dark.
    #[test]
    fn always_light_mode_for_edit_view_gives_the_light_canvas_under_a_dark_system() {
        let mut app = app();
        set(&mut app, "appearance.color.cornerNodes.dark", json!("#ff0000"));
        let light = Tokens::from_settings(app.session.settings(), false);
        let dark = Tokens::from_settings(app.session.settings(), true);
        assert_ne!(light.canvas, dark.canvas, "the two sets have different canvas defaults");
        assert_eq!(app.models.canvas_tokens.canvas, light.canvas, "a light window draws the light set");

        app.set_system_dark(true);
        app.drain();
        assert_eq!(app.models.canvas_tokens.canvas, dark.canvas);
        assert_eq!(app.models.canvas_tokens.node_corner, egui::Color32::from_rgb(0xff, 0, 0), "the stored dark colour");
        assert!(app.models.appearance.edit_view_scope().is_none() && app.models.appearance.font_view_scope().is_none());

        set(&mut app, EDIT_VIEW_LIGHT, json!(true));
        assert_eq!(app.models.canvas_tokens.canvas, light.canvas, "Edit View is kept light");
        assert_eq!(app.models.canvas_tokens.node_corner, light.node_corner);
        assert!(app.models.appearance.edit_view_scope().is_some(), "the canvas is shown in a light scope");
        assert!(app.models.appearance.font_view_scope().is_none(), "Font View still follows the window");
        assert!(app.view.system_dark, "the window stays dark");

        set(&mut app, FONT_VIEW_LIGHT, json!(true));
        assert!(app.models.appearance.font_view_scope().is_some());
        app.set_system_dark(false);
        app.drain();
        assert!(app.models.appearance.font_view_scope().is_none(), "a light window needs no scope");
        assert_eq!(app.models.canvas_tokens.canvas, light.canvas);
    }
}
