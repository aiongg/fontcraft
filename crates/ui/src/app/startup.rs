//! What the desktop window sets up when it opens and keeps up while it runs, beside the UI.
//!
//! - [`open_extensions`]: the user's glyph data and groups files, the snippets folder and the extension folders (Modules and Macros) inside the settings
//!   folder, the module switches, and when the session started.
//! - [`launch`]: what the window opens with. A file named on the command line opens with its display-string tabs; a
//!   `fontcraft://show/` URL runs `url.show`. With neither, the Start Window shows while `start.showAtLaunch` is on,
//!   else a new font opens. When nothing opened, the Start Window shows.
//! - [`poll_folders`], every frame: reads the extension folders again on the first frame, when the window gains focus
//!   and every [`FOLDER_POLL_MS`] while it has it. Alt and Shift held on the first frame start safe mode, as
//!   `--safe-mode` does.
//!
//! The desktop app hands these facts over in `fontcraft_ui::Options`; tests and the browser build have none of them.

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::extensions::FOLDER_POLL_MS;
use serde_json::{Value, json};

use super::{FontCraftApp, Msg};

/// What the window opens with in place of the Start Window or a new font.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Launch {
    /// A font file to open (`fontcraft FILE`).
    File(String),
    /// A `fontcraft://show/?…` URL: open its files and show its glyphs in a new tab.
    Url(String),
}

/// The status line after Alt and Shift held as the window opened turned safe mode on.
pub const SAFE_MODE: &str = "Safe mode: extension modules are off. Help › Load Extensions turns them on.";

/// When the extension folders were last read, and whether the window had focus at the last frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FolderPoll {
    /// The input time of the last poll, in seconds; `None` before the first frame.
    pub last: Option<f64>,
    /// The window had focus at the last frame.
    pub focused: bool,
}

/// Where the snippets are kept inside the settings folder.
pub const SNIPPETS_FOLDER: &str = "snippets";

/// Reads the user's glyph data and groups from the `Info` folder of the settings folder `dir`, sets the snippets folder and the extension folders inside the settings folder `dir`, notes when the session
/// started, and reads the module switches; `safe_mode` lists the modules but runs none. The folders are read by the
/// first poll. Every problem goes to the log, the first to the status line.
#[cfg(not(target_arch = "wasm32"))]
pub fn open_extensions(app: &mut FontCraftApp, dir: Option<&std::path::Path>, safe_mode: bool) {
    use std::time::{SystemTime, UNIX_EPOCH};

    app.session.set_snippets_dir(dir.map(|d| d.join(SNIPPETS_FOLDER)));
    let started = SystemTime::now().duration_since(UNIX_EPOCH).ok().and_then(|d| i64::try_from(d.as_secs()).ok());
    app.session.extensions_mut().started_at = started;
    let mut problems = dir.map(|d| app.session.load_user_info(d)).unwrap_or_default();
    problems.extend(fontcraft_engine::extensions::open_folders(app.session.extensions_mut(), dir, safe_mode));
    report(app, &problems);
}

/// Opens what the window starts with, after the settings were read: the file or URL of `launch`; without one, the
/// Start Window while `start.showAtLaunch` is on, else a new font. When no document is open after that, the Start
/// Window shows. A failure goes to the log and the status line.
pub fn launch(app: &mut FontCraftApp, launch: Option<&Launch>) {
    let result = match launch {
        Some(Launch::File(path)) => app.run_now(Msg::run("file.open", json!({ "path": path })), false),
        Some(Launch::Url(url)) => app.run_now(Msg::Shell(CommandRef::with_params("url.show", json!({ "url": url }))), false),
        None if app.session.settings().get_bool(crate::shell::start::SHOW_AT_LAUNCH).unwrap_or(true) => Ok(Value::Null),
        None => app.run_now(Msg::run("file.new", json!({})), false),
    };
    if let Err(e) = result {
        log::error!("{e}");
    }
    if app.session.documents().is_empty() {
        app.sent.push(Msg::Shell(CommandRef::with_params("window.startWindow", json!({}))));
    }
}

/// Logs every problem and puts the first in the status line.
fn report(app: &mut FontCraftApp, problems: &[String]) {
    for problem in problems {
        log::warn!("{problem}");
    }
    if let Some(first) = problems.first() {
        app.status = first.clone();
    }
}

/// Reads the extension folders again on the first frame, when the window gains focus and every [`FOLDER_POLL_MS`]
/// while it has it; nothing without [`FontCraftApp::folder_poll`]. Alt and Shift held on the first frame start safe
/// mode. Every problem goes to the log, the first to the status line.
pub fn poll_folders(app: &mut FontCraftApp, ctx: &egui::Context) {
    let Some(poll) = app.folder_poll.as_mut() else { return };
    let every = std::time::Duration::from_millis(FOLDER_POLL_MS);
    let (now, focused, launch_keys) = ctx.input(|i| (i.time, i.focused, i.modifiers.alt && i.modifiers.shift));
    let first = poll.last.is_none();
    let gained = focused && !poll.focused;
    poll.focused = focused;
    let due = poll.last.is_none_or(|t| now - t >= every.as_secs_f64());
    let read = first || gained || (focused && due);
    if read {
        poll.last = Some(now);
    }
    if first && launch_keys && !app.session.extensions().safe_mode {
        app.session.extensions_mut().safe_mode = true;
        app.status = SAFE_MODE.into();
    }
    if read {
        let problems = fontcraft_engine::extensions::poll_folders(app.session.extensions_mut());
        report(app, &problems);
    }
    if focused {
        ctx.request_repaint_after(every);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use fontcraft_engine::Session;

    use craft::ui::runtime::commands::CommandRef;
    use serde_json::json;

    use super::{FolderPoll, Launch, SAFE_MODE, launch, poll_folders};
    use crate::app::{APP, FontCraftApp, Msg};

    fn polled_app() -> FontCraftApp {
        let mut app = FontCraftApp::new(Session::with_document(), Arc::new(Host::builder(APP).build()));
        app.folder_poll = Some(FolderPoll::default());
        app
    }

    /// A frame at input time `time`, with the window focused or not and `modifiers` held.
    fn frame(ctx: &egui::Context, app: &mut FontCraftApp, time: f64, focused: bool, modifiers: egui::Modifiers) {
        let input = egui::RawInput { time: Some(time), focused, events: vec![egui::Event::ModifiersChanged(modifiers)], ..Default::default() };
        ctx.run_ui(input, |ui| poll_folders(app, ui.ctx())).textures_delta.clear();
    }

    fn last(app: &FontCraftApp) -> Option<f64> {
        app.folder_poll.and_then(|p| p.last)
    }

    /// The folders are read on the first frame, when the window gains focus and once the poll interval has passed
    /// while it has it; not while it is in the background.
    #[test]
    fn the_extension_folders_are_read_on_the_first_frame_on_focus_and_while_focused() {
        let none = egui::Modifiers::NONE;
        let ctx = egui::Context::default();
        let mut app = polled_app();
        frame(&ctx, &mut app, 0.0, false, none);
        assert_eq!(last(&app), Some(0.0), "the first frame reads them");
        frame(&ctx, &mut app, 30.0, false, none);
        assert_eq!(last(&app), Some(0.0), "not in the background");
        frame(&ctx, &mut app, 31.0, true, none);
        assert_eq!(last(&app), Some(31.0), "the window gained focus");
        frame(&ctx, &mut app, 32.0, true, none);
        assert_eq!(last(&app), Some(31.0), "not due yet");
        frame(&ctx, &mut app, 36.5, true, none);
        assert_eq!(last(&app), Some(36.5), "due while focused");
        assert!(!app.session.extensions().safe_mode);

        // Without the desktop's poll nothing is read.
        let mut app = polled_app();
        app.folder_poll = None;
        frame(&ctx, &mut app, 40.0, true, none);
        assert_eq!(last(&app), None);
    }

    /// Alt and Shift held on the first frame start safe mode and say so; held later they do nothing.
    #[test]
    fn alt_and_shift_held_as_the_window_opens_start_safe_mode() {
        let keys = egui::Modifiers { alt: true, shift: true, ..Default::default() };
        let mut app = polled_app();
        frame(&egui::Context::default(), &mut app, 0.0, true, keys);
        assert!(app.session.extensions().safe_mode);
        assert_eq!(app.status, SAFE_MODE);

        let ctx = egui::Context::default();
        let mut app = polled_app();
        frame(&ctx, &mut app, 0.0, true, egui::Modifiers::NONE);
        frame(&ctx, &mut app, 1.0, true, keys);
        assert!(!app.session.extensions().safe_mode, "only the first frame counts");
    }

    fn start_window() -> Msg {
        Msg::Shell(CommandRef::with_params("window.startWindow", json!({})))
    }

    fn empty_app() -> FontCraftApp {
        FontCraftApp::new(Session::new(), Arc::new(Host::builder(APP).build()))
    }

    /// A file opens in place of the Start Window; one that cannot be opened says why and the Start Window shows.
    /// Without a file the Start Window shows, or a new font opens while Show at Launch is off.
    #[test]
    #[allow(clippy::unwrap_used)]
    fn the_window_opens_the_file_given_else_the_start_window_or_a_new_font() {
        let dir = std::env::temp_dir().join(format!("fontcraft-ui-launch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("launch.glyphs").to_string_lossy().into_owned();
        Session::with_document().execute("file.saveAs", &json!({ "path": path })).unwrap();

        let mut app = empty_app();
        launch(&mut app, Some(&Launch::File(path.clone())));
        assert_eq!(app.session.documents().len(), 1);
        assert!(app.sent.is_empty(), "no Start Window: {:?}", app.sent);

        let mut app = empty_app();
        launch(&mut app, Some(&Launch::File(dir.join("missing.glyphs").to_string_lossy().into_owned())));
        assert!(app.session.documents().is_empty());
        assert!(app.status.starts_with("file.open: "), "{}", app.status);
        assert_eq!(app.sent, vec![start_window()]);

        let mut app = empty_app();
        launch(&mut app, None);
        assert!(app.session.documents().is_empty());
        assert_eq!(app.sent, vec![start_window()]);

        let mut app = empty_app();
        app.session.execute("settings.set", &json!({ "id": crate::shell::start::SHOW_AT_LAUNCH, "value": false })).unwrap();
        launch(&mut app, None);
        assert_eq!(app.session.documents().len(), 1, "a new font");
        assert!(app.sent.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing)]
mod window_tests {
    use craft::behavior::kurbo::Point as KPoint;
    use craft::ui::runtime::commands::CommandRef;
    use craft::ui::testing::input::{key_press, text};
    use egui::{Key, Modifiers};
    use serde_json::json;

    use crate::shell::dock::PanelId;
    use crate::testing::TestApp;

    /// The desktop window starts with no document; once a font opens, the document area draws its tab strip at the
    /// top of the group and Font View under it, with Font View's status line at the group's bottom.
    #[test]
    fn the_document_area_draws_font_view_once_a_font_opens() {
        let mut t = TestApp::window(crate::Options::default());
        assert!(t.app.session.documents().is_empty());
        t.frames(2);
        t.ok("engine.execute", json!({"command": "file.new", "params": {}}));
        t.frames(2);
        let tab = t.frames.texts().iter().find(|x| x.text == "Font View").map(|x| x.rect).expect("the Font View tab");
        let status = t.frames.texts().iter().find(|x| x.text.contains("54 shown")).map(|x| x.rect).expect("Font View's status line");
        assert!(status.min.y > tab.max.y + 400.0, "the status line under the tab strip, at the bottom: {tab:?} {status:?}");
    }

    /// With a font open, nothing in the window sits between pixels: debug builds paint "Unaligned" over such a
    /// region, as they did over the Layers group when the palette's groups split the column's height evenly.
    #[test]
    fn the_window_draws_every_region_on_whole_points() {
        let mut t = TestApp::window(crate::Options::default());
        t.ok("engine.execute", json!({"command": "file.new", "params": {}}));
        t.frames(2);
        assert!(t.frames.texts().iter().any(|x| x.text == "Layers"), "the Layers group shows");
        let unaligned: Vec<_> = t.frames.texts().iter().filter(|x| x.text == "Unaligned").map(|x| x.rect).collect();
        assert!(unaligned.is_empty(), "regions between pixels at {unaligned:?}");
    }

    /// Font Info, the Kerning window and the other panels the reference shows as windows or dialogs open as windows
    /// of their own over the document area, as tall as the window allows, not as a tab of the palette's first group.
    #[test]
    fn font_info_and_the_kerning_window_open_as_windows() {
        let mut t = TestApp::window(crate::Options::default());
        t.ok("engine.execute", json!({"command": "file.new", "params": {}}));
        t.frames(2);
        let windows = [
            ("font.info", json!({}), PanelId::FontInfo),
            ("window.kerning", json!({}), PanelId::KerningWindow),
            ("window.spaceCenter", json!({}), PanelId::SpaceCenter),
            ("window.textPreview", json!({}), PanelId::TextPreview),
            ("edit.compareFonts", json!({}), PanelId::CompareFonts),
            ("edit.compareFamily", json!({}), PanelId::CompareFamily),
            ("window.kerningPairsPreview", json!({}), PanelId::KerningPairsPreview),
            ("dialog.kerningGroupsEditor", json!({}), PanelId::KerningGroupsEditor),
            ("window.shapeOrder", json!({"glyph": "A"}), PanelId::ShapeOrder),
            ("audit.showPanel", json!({"visible": true}), PanelId::FontAudit),
            ("bitmap.setStrikes", json!({"add": [{"ppem": 12}]}), PanelId::BitmapEditor),
        ];
        for (command, params, panel) in windows {
            t.ok("engine.execute", json!({"command": command, "params": params}));
            if panel == PanelId::BitmapEditor {
                t.ok("engine.execute", json!({"command": "window.bitmapEditor", "params": {"glyph": "A"}}));
            }
            t.frames(2);
            let layout = t.app.view.workspace.layout();
            let window = layout.floating.iter().find(|w| w.contains(&panel)).expect("the panel floats");
            assert!(window.rect.height() >= 400.0, "{command}: {:?}", window.rect);
            assert!(!window.panels().any(|p| *p == PanelId::GlyphMetrics), "{command} floats alone");
        }
    }

    /// A tab opened on several glyphs fits their whole line in the canvas the first time it draws.
    #[test]
    fn a_tab_on_several_glyphs_fits_the_line() {
        let mut t = TestApp::window(crate::Options::default());
        t.ok("engine.execute", json!({"command": "file.new", "params": {}}));
        t.frames(2);
        t.ok("engine.execute", json!({"command": "view.editSelectedGlyphs", "params": {"glyphs": ["H", "O", "H", "O", "H", "O"]}}));
        t.frames(2);
        let layout = t.app.models.text.layout.clone().expect("the text's layout");
        let ends = layout.placed.iter().map(|p| (p.x, p.x + p.advance));
        let (x0, x1) = ends.fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), (l, r)| (a.min(l), b.max(r)));
        let view = t.app.view.canvas;
        let o = t.app.models.text.origin[0];
        let (left, right) = (view.to_screen(KPoint::new(x0 - o, 0.0)).x, view.to_screen(KPoint::new(x1 - o, 0.0)).x);
        assert!(layout.placed.len() == 6 && x1 - x0 > 1000.0, "{:?}", layout.placed);
        assert!(view.rect.left() <= left && right <= view.rect.right(), "the line {left}..{right} inside {:?}", view.rect);
    }

    /// A row of checkboxes in a dialog names each box beside it: the Export dialog's File Format row shows its three
    /// containers.
    #[test]
    fn a_dialog_names_each_box_of_a_checkbox_row() {
        let mut t = TestApp::window(crate::Options::default());
        t.ok("engine.execute", json!({"command": "file.new", "params": {}}));
        t.frames(2);
        t.ok("ui.menu.invoke", json!({"command": "file.export"}));
        t.frames(2);
        for name in ["OTF/TTF", "WOFF", "WOFF2"] {
            assert!(t.frames.texts().iter().any(|x| x.text == name), "{name} shows beside its box");
        }
    }

    fn selected(t: &mut TestApp) -> serde_json::Value {
        t.ok("ui.inspect", json!({}))["fontView"]["selected"].clone()
    }

    /// While Font View shows, a letter runs no shortcut of the edit view: G, the TrueType Instructor's Interpolate,
    /// does nothing from the keyboard or the control channel. With a cell focused, a letter selects by name.
    #[test]
    fn a_letter_in_font_view_runs_no_edit_view_shortcut() {
        let mut t = TestApp::new();
        t.app.fire(CommandRef::new("fontView.show"));
        t.frames(2);
        assert!(t.app.font_view_shows());
        t.frame_with(vec![key_press(Key::G, Modifiers::NONE), text("g")]).frame();
        assert_eq!(t.app.status, "", "no shortcut ran");
        let reply = t.ok("ui.key", json!({"key": "g"}));
        assert_eq!(reply.get("command"), None, "{reply}");
        assert_eq!(t.app.status, "");
        // The Instructor's keys work in the edit view while it is the tool.
        let works = crate::app::catalog::shortcut_works;
        assert!(works("hint.tt.interpolate", false, "trueTypeInstructor"));
        assert!(!works("hint.tt.interpolate", true, "trueTypeInstructor"));
        assert!(!works("hint.tt.interpolate", false, "select"));

        // The cell's name is the last `.notdef` drawn: the tab strip's Edit View tab shows it first.
        let cell = t.frames.texts().iter().rfind(|x| x.text == ".notdef").map(|x| x.rect.center()).expect(".notdef's cell");
        t.ok("ui.click", json!({"x": cell.x, "y": cell.y - 30.0}));
        assert_eq!(selected(&mut t), json!([".notdef"]));
        let reply = t.ok("ui.key", json!({"key": "k"}));
        assert_eq!(reply["fontView"], true, "{reply}");
        assert_eq!(selected(&mut t), json!(["K"]), "type-ahead selects the first name starting with k");
        assert_eq!(t.app.status, "");
    }
}
