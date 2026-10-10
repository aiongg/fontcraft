//! The Annotation tool's shell commands: Select All while it is active, and the palette's six
//! sub-tool buttons (`annotation.selectAll`, `annotation.setTool`). The annotations themselves
//! are engine commands (`annotation.*`).

use fontcraft_tools::Selection;
use fontcraft_tools::annotate::{SUB_TOOLS, sub_tool_id};
use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::tools_glue;

pub const COMMANDS: &[UiCommand] = &[
    (
        "annotation.selectAll",
        "Select All Annotations",
        &[],
        "",
        "{glyph?: name (the edited glyph), layer?: id} select every annotation of the edited layer and nothing else; the Annotation tool runs it for Cmd+A → {selection}",
    ),
    (
        "annotation.setTool",
        "Choose Annotation Sub-tool",
        &[],
        "",
        "{tool: \"select\" | \"text\" | \"arrow\" | \"circle\" | \"plus\" | \"minus\"} make that annotation sub-tool the active tool; a gesture in progress is cancelled → {tool, previous}",
    ),
];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "annotation.setTool" => {
            let names: Vec<&str> = SUB_TOOLS.iter().map(|(n, _, _)| *n).collect();
            let name = args.string("tool")?.ok_or_else(|| args.err(format!("`tool` is required: one of {}", names.join(", "))))?;
            let tool = sub_tool_id(name).ok_or_else(|| args.err(format!("unknown sub-tool `{name}`; the sub-tools are {}", names.join(", "))))?;
            let switch = app.tools.toolbox.select(tool).map_err(|e| args.err(e))?;
            tools_glue::apply(app, switch.output);
            tools_glue::sync(app);
            Ok(json!({"tool": switch.tool, "previous": switch.previous}))
        }
        "annotation.selectAll" => {
            tools_glue::sync(app);
            let (selection, reply) = {
                let font = app.session.font().map_err(|e| e.to_string())?;
                let tab = app.state.tab().ok_or_else(|| args.err("no Edit View tab is showing (open a glyph first)"))?;
                let scene = tools_glue::scene(font, tab).ok_or_else(|| args.err("the edited glyph has no layer in the active master"))?;
                if let Some(glyph) = args.string("glyph")?
                    && glyph != scene.glyph
                {
                    return Err(args.err(format!("`{glyph}` is not the glyph being edited (`{}`); open it first", scene.glyph)));
                }
                if let Some(layer) = args.string("layer")?
                    && layer != scene.layer_id
                {
                    return Err(args.err(format!("layer `{layer}` is not the layer being edited (`{}`)", scene.layer_id)));
                }
                let mut s = Selection::new();
                s.annotations.extend(0..scene.layer.info.annotations.len());
                let reply = json!({"selection": Value::Object(s.targets(&scene))});
                (s, reply)
            };
            if let Some(tab) = app.state.tab_mut() {
                tab.selection = selection;
            }
            Ok(reply)
        }
        other => Err(format!("unknown command `{other}`")),
    }
}
