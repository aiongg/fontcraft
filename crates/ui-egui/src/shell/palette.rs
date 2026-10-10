//! The Palette sidebar's window command; the sidebar itself is drawn by [`crate::panels`].

use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;

pub const COMMANDS: &[UiCommand] =
    &[("window.palette", "Palette", &["Window"], "Cmd+Alt+P", "{on?: bool} show or hide the Palette sidebar (omitted: toggle)")];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "window.palette" => {
            app.state.show_palette = args.toggle(app.state.show_palette)?;
            Ok(json!({"on": app.state.show_palette}))
        }
        other => Err(format!("unknown command `{other}`")),
    }
}
