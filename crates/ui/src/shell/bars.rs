//! The options bar and the status bar.

use craft::behavior::IconId;
use craft::ui::bars::{Bar, BarKind};
use craft::ui::buttons::IconButton;
use craft::ui::contract::tooltip::with_chord;
use craft::ui::contract::{Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::runtime::commands::{self, CommandRef};
use craft::ui::runtime::env::Env;
use egui::Ui;

use super::ChromeWords;
use crate::app::catalog::TOOL_PREFIX;
use crate::panels::edit_bar::{self, EditBar};

/// What the options bar reads: the current tool, whose options it will show, and whether the Palette shows, for the
/// button at the bar's end that shows or hides it.
pub struct OptionsProps<'a> {
    pub tool: &'static str,
    pub palette: bool,
    pub sent: &'a mut Vec<crate::app::Msg>,
}

pub fn options(ui: &mut Ui, mut props: OptionsProps<'_>) {
    options_view(&mut props, ui);
}

fn options_view(props: &mut OptionsProps<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<ChromeWords>();
    let tool = commands::resolve(ui.ctx(), CommandRef::new(format!("{TOOL_PREFIX}{}", props.tool))).label;
    let palette = props.palette;
    // The end group runs in its own closure beside the body's, so it posts into its own list.
    let mut end_sent = Vec::new();
    let end = &mut end_sent;
    let _ = Bar::new(words.options.clone(), BarKind::Options).trailing(move |ui, _| palette_button(ui, palette, &mut *end)).show(ui, |ui, _| {
        Text::new(tool.to_string()).show(ui);
    });
    props.sent.extend(end_sent);
}

/// The sidebar button at the window's top right: shows or hides the Palette (`window.palette`), lit while it shows.
fn palette_button(ui: &mut Ui, shows: bool, sent: &mut Vec<crate::app::Msg>) {
    let words = Env::of(ui).words::<ChromeWords>();
    let toggle = CommandRef::new("window.palette");
    let os = craft::ui::runtime::windows::os_of(ui.ctx());
    let tip = match commands::resolve(ui.ctx(), toggle.clone()).chord {
        Some(chord) => with_chord(words.palette.as_str(), &chord, os),
        None => words.palette.as_str().to_string(),
    };
    let out = IconButton::new(IconId::new("panel_right"))
        .label(words.palette.clone())
        .tooltip(tip)
        .selected(shows)
        .id_salt("fontcraftShell.palette")
        .show(ui);
    if out.clicked() {
        sent.push(crate::app::Msg::Shell(toggle));
    }
}

/// What the status bar reads.
pub struct StatusProps<'a> {
    /// The last error or notice; empty when there is none.
    pub status: &'a str,
    /// Why the edited or selected glyph cannot interpolate, shown while there is no error or notice; empty when it can.
    pub reason: &'a str,
    /// The Features menu, at the bar's start.
    pub features: &'a craft::ui::contract::Tracked<crate::app::models::text::FeaturesModel>,
    /// The Edit View's controls; `None` while Font View shows.
    pub edit: Option<EditBar>,
    /// The Higher-Order Interpolation tool's axis, while it is the current tool.
    pub hoi: Option<crate::canvas::hoi::AxisChoice>,
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
    let hoi = props.hoi.take();
    // The end group runs in its own closure beside the body's, so it posts into its own list.
    let mut end_sent = Vec::new();
    let mut bar = Bar::new(words.status.clone(), BarKind::Status);
    if let Some(edit) = edit {
        let end_sent = &mut end_sent;
        bar = bar.trailing(move |ui, _| {
            edit_bar::end_controls(ui, edit_bar::Props { bar: edit, sent: &mut *end_sent });
            if let Some(choice) = &hoi {
                edit_bar::hoi_axis(ui, choice, &mut *end_sent);
            }
        });
    }
    let _ = bar.show(ui, |ui, _| {
        crate::panels::features_menu::show(ui, crate::panels::features_menu::Props { model: props.features, sent: props.sent });
        if let Some(edit) = edit {
            edit_bar::preview_button(ui, edit_bar::Props { bar: edit, sent: props.sent });
        }
        if props.status.is_empty() && !props.reason.is_empty() {
            Text::new(props.reason.to_string()).show(ui);
        } else if props.status.is_empty() {
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
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::runtime::commands::CommandRef;
    use craft::ui::testing::Fixture;
    use serde_json::json;

    use super::edit_state;
    use crate::app::Msg;
    use crate::testing::TestApp;

    #[test]
    fn the_edit_controls_follow_the_showing_tab() {
        let mut t = TestApp::new();
        let bar = edit_state(&t.app).expect("the test starts on an Edit View tab");
        assert_eq!((bar.zoom, bar.preview_area), (t.app.view.canvas.zoom * 1000.0, false));
        t.app.fire(CommandRef::with_params("view.togglePreviewArea", json!({"on": true})));
        t.frame();
        assert_eq!(edit_state(&t.app).map(|b| b.preview_area), Some(true), "the controls read the live tab");
        t.app.fire(CommandRef::new("fontView.show"));
        t.frame();
        assert_eq!(edit_state(&t.app), None, "Font View shows no Edit View controls");
    }

    /// The sidebar button at the options bar's end shows or hides the Palette.
    #[test]
    fn the_palette_button_posts_window_palette() {
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            super::palette_button(ui, true, &mut sent);
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let button = fixture.by_label("Palette").expect("the Palette button");
        fixture.click(&button).settle();
        assert_eq!(*posted.borrow(), vec![Msg::Shell(CommandRef::new("window.palette"))]);
    }
}
