//! The options bar and the status bar.

use craft::ui::bars::{Bar, BarKind};
use craft::ui::contract::{Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::runtime::commands::{self, CommandRef};
use craft::ui::runtime::env::Env;
use egui::Ui;

use super::ChromeWords;
use crate::app::catalog::TOOL_PREFIX;
use crate::panels::edit_bar::{self, EditBar};

/// What the options bar reads: the current tool, whose options it will show.
pub struct OptionsProps {
    pub tool: &'static str,
}

pub fn options(ui: &mut Ui, mut props: OptionsProps) {
    options_view(&mut props, ui);
}

fn options_view(props: &mut OptionsProps, ui: &mut Ui) {
    let words = Env::of(ui).words::<ChromeWords>();
    let tool = commands::resolve(ui.ctx(), CommandRef::new(format!("{TOOL_PREFIX}{}", props.tool))).label;
    let _ = Bar::new(words.options.clone(), BarKind::Options).show(ui, |ui, _| {
        Text::new(tool.to_string()).show(ui);
    });
}

/// What the status bar reads.
pub struct StatusProps<'a> {
    /// The last error or notice; empty when there is none.
    pub status: &'a str,
    /// The Features menu, at the bar's start.
    pub features: &'a craft::ui::contract::Tracked<crate::app::models::text::FeaturesModel>,
    /// The Edit View's controls; `None` while Font View shows.
    pub edit: Option<EditBar>,
    pub sent: &'a mut Vec<crate::app::Msg>,
}

/// What the Edit View's controls show, read from the live tab; `None` while Font View is the showing tab.
pub fn edit_state(app: &crate::app::FontCraftApp) -> Option<EditBar> {
    if app.font_view_shows() {
        return None;
    }
    let view = &app.view;
    Some(EditBar {
        direction: view.text.direction,
        kerning: view.text.kerning,
        zoom: view.canvas.zoom * 1000.0,
        preview_area: view.preview.area_open,
    })
}

pub fn status(ui: &mut Ui, mut props: StatusProps<'_>) {
    status_view(&mut props, ui);
}

fn status_view(props: &mut StatusProps<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<ChromeWords>();
    let edit = props.edit;
    // The end group runs in its own closure beside the body's, so it posts into its own list.
    let mut end_sent = Vec::new();
    let mut bar = Bar::new(words.status.clone(), BarKind::Status);
    if let Some(edit) = edit {
        let end_sent = &mut end_sent;
        bar = bar.trailing(move |ui, _| edit_bar::end_controls(ui, edit_bar::Props { bar: edit, sent: &mut *end_sent }));
    }
    let _ = bar.show(ui, |ui, _| {
        crate::panels::features_menu::show(ui, crate::panels::features_menu::Props { model: props.features, sent: props.sent });
        if let Some(edit) = edit {
            edit_bar::preview_button(ui, edit_bar::Props { bar: edit, sent: props.sent });
        }
        if props.status.is_empty() {
            Text::new(words.ready.clone()).show(ui);
        } else {
            Text::new(props.status.to_string()).show(ui);
        }
    });
    props.sent.extend(end_sent);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use craft::ui::runtime::commands::CommandRef;
    use serde_json::json;

    use super::edit_state;
    use crate::testing::TestApp;

    #[test]
    fn the_edit_controls_follow_the_showing_tab() {
        let mut t = TestApp::new();
        let bar = edit_state(&t.app).expect("a window opens on an Edit View tab");
        assert_eq!((bar.zoom, bar.preview_area), (t.app.view.canvas.zoom * 1000.0, false));
        t.app.fire(CommandRef::with_params("view.togglePreviewArea", json!({"on": true})));
        t.frame();
        assert_eq!(edit_state(&t.app).map(|b| b.preview_area), Some(true), "the controls read the live tab");
        t.app.fire(CommandRef::new("fontView.show"));
        t.frame();
        assert_eq!(edit_state(&t.app), None, "Font View shows no Edit View controls");
    }
}
