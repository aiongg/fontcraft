//! The Primitives tool's two modes as commands (`tool.drawRectangle`, `tool.drawCircle`): the
//! items of the tool's toolbar menu.

use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;

pub const COMMANDS: &[UiCommand] = &[
    ("tool.drawRectangle", "Draw Rectangle", &[], "", "{} make the Primitives tool active in rectangle mode → {tool, previous}"),
    ("tool.drawCircle", "Draw Circle", &[], "", "{} make the Primitives tool active in ellipse mode → {tool, previous}"),
];

pub fn run(app: &mut FontcraftApp, id: &str, _args: &Args) -> Result<Value, String> {
    let tool = if id == "tool.drawCircle" { "ellipse" } else { "rectangle" };
    app.run("tool.select", &json!({ "tool": tool }))
}
