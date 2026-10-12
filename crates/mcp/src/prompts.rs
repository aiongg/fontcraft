//! Workflow prompts (`prompts/list`, `prompts/get`): each one walks an agent through a common task
//! with the commands that do it, so the agent starts from the right command ids.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::resources::Source;

/// Longest prompt argument value, in bytes (a pair list can be long).
pub const MAX_PROMPT_ARGUMENT: usize = 1 << 20;
/// Most arguments in one `prompts/get`.
pub const MAX_PROMPT_ARGUMENTS: usize = 16;

pub(crate) struct Arg {
    pub name: &'static str,
    pub description: &'static str,
    pub required: bool,
    /// What `completion/complete` offers for this argument.
    pub completion: Option<Source>,
}

pub(crate) struct Prompt {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub arguments: &'static [Arg],
    /// Every command the text names, in the order it names them.
    pub commands: &'static [&'static str],
    render: fn(&Args) -> String,
}

/// The arguments of one `prompts/get`.
pub(crate) struct Args(BTreeMap<String, String>);

impl Args {
    fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str).filter(|v| !v.trim().is_empty())
    }
}

pub(crate) const PROMPTS: &[Prompt] = &[
    Prompt {
        name: "start_family",
        title: "Start a type family",
        description: "Create a new font for a family: names, metrics, the first glyphs and their outlines, then save the source.",
        arguments: &[
            Arg { name: "family_name", description: "The family name, for example Fieldstone.", required: true, completion: None },
            Arg {
                name: "glyphs",
                description: "Glyphs to add beyond the starting set: names, characters or ranges separated by spaces.",
                required: false,
                completion: None,
            },
            Arg { name: "file", description: "Where to save the source (a .fontcraft, .glyphs or .ufo path).", required: false, completion: None },
        ],
        commands: &[
            "file.new",
            "font.info.set",
            "font.master.list",
            "font.master.setMetric",
            "glyph.addGlyphs",
            "path.addPath",
            "layer.inspect",
            "file.saveAs",
        ],
        render: start_family,
    },
    Prompt {
        name: "add_master",
        title: "Add a master",
        description: "Add a master at a design-space location, set its metrics, draw it compatibly with the others and check compatibility.",
        arguments: &[
            Arg { name: "name", description: "The master's name, for example Bold.", required: true, completion: None },
            Arg { name: "axes", description: "Its location as tag=value pairs, for example wght=700.", required: false, completion: None },
            Arg {
                name: "from_master",
                description: "The master whose outlines to start from (default: the first).",
                required: false,
                completion: Some(Source::MasterNames),
            },
        ],
        commands: &[
            "font.master.list",
            "font.master.duplicate",
            "font.master.set",
            "font.master.add",
            "font.master.setMetric",
            "layer.inspect",
            "path.addPath",
            "audit.run",
        ],
        render: add_master,
    },
    Prompt {
        name: "build_accented_glyphs",
        title: "Build accented glyphs",
        description: "Add accented glyphs and build them from components of their base letters and marks, positioned by anchors, on every master.",
        arguments: &[Arg {
            name: "glyphs",
            description: "The accented glyphs, by name or character, separated by spaces (for example aacute eacute Ü).",
            required: true,
            completion: Some(Source::GlyphNames),
        }],
        commands: &[
            "glyphData.lookup",
            "glyph.addGlyphs",
            "glyph.setAnchors",
            "anchor.setPosition",
            "glyph.createComposite",
            "layer.inspect",
            "audit.run",
        ],
        render: build_accented,
    },
    Prompt {
        name: "kern_pair_list",
        title: "Kern a pair list",
        description: "Set the kerning of every pair in a list on one master, as one undo step, and report the values that changed.",
        arguments: &[
            Arg {
                name: "pairs",
                description: "One pair per line: left and right glyph names (or @group), then optionally a value in units.",
                required: true,
                completion: None,
            },
            Arg { name: "master", description: "The master to kern (default: the first).", required: false, completion: Some(Source::MasterNames) },
        ],
        commands: &["font.master.list", "kerning.listGroups", "kerning.getPair", "kerning.setPair", "command.batch", "kerning.listPairs"],
        render: kern_pairs,
    },
    Prompt {
        name: "prepare_release",
        title: "Prepare a release",
        description: "Step the version, date the font, run the strict checks, fix what can be fixed, export the fonts and verify them.",
        arguments: &[
            Arg { name: "destination", description: "The folder for the exported fonts.", required: false, completion: None },
            Arg {
                name: "version",
                description: "The version to set, for example 1.002 (default: one minor step).",
                required: false,
                completion: None,
            },
        ],
        commands: &[
            "font.info.get",
            "font.info.set",
            "font.info.stepVersion",
            "font.info.updateDate",
            "audit.run",
            "audit.fix",
            "font.compilePlan",
            "file.exportStatic",
            "font.inspect",
            "file.save",
        ],
        render: prepare_release,
    },
];

pub(crate) fn find(name: &str) -> Option<&'static Prompt> {
    PROMPTS.iter().find(|p| p.name == name)
}

/// The `prompts/list` result.
pub(crate) fn list() -> Value {
    let prompts: Vec<Value> = PROMPTS
        .iter()
        .map(|p| {
            let arguments: Vec<Value> =
                p.arguments.iter().map(|a| json!({"name": a.name, "description": a.description, "required": a.required})).collect();
            json!({"name": p.name, "title": p.title, "description": p.description, "arguments": arguments})
        })
        .collect();
    json!({"prompts": prompts})
}

/// The `prompts/get` result. `refused` lists the prompt's commands this server will not run.
pub(crate) fn get(prompt: &Prompt, arguments: &Value, refused: &[&str]) -> Result<Value, String> {
    let mut args = BTreeMap::new();
    match arguments {
        Value::Null => {}
        Value::Object(map) => {
            if map.len() > MAX_PROMPT_ARGUMENTS {
                return Err(format!("at most {MAX_PROMPT_ARGUMENTS} arguments"));
            }
            for (name, value) in map {
                if !prompt.arguments.iter().any(|a| a.name == name) {
                    return Err(format!("prompt `{}` has no argument `{name}`", prompt.name));
                }
                let Some(text) = value.as_str() else {
                    return Err(format!("argument `{name}` must be a string"));
                };
                if text.len() > MAX_PROMPT_ARGUMENT {
                    return Err(format!("argument `{name}` may be at most {MAX_PROMPT_ARGUMENT} bytes"));
                }
                args.insert(name.clone(), text.to_string());
            }
        }
        _ => return Err("`arguments` must be an object of strings".into()),
    }
    let args = Args(args);
    if let Some(missing) = prompt.arguments.iter().find(|a| a.required && args.get(a.name).is_none()) {
        return Err(format!("prompt `{}` needs the argument `{}`", prompt.name, missing.name));
    }
    let mut text = (prompt.render)(&args);
    text.push_str(&format!("\n\nCommands for this task: {}. Each documents its parameters in list_commands.", prompt.commands.join(", ")));
    if !refused.is_empty() {
        text.push_str(&format!(
            "\n\nThis server refuses {}: do the parts that need them by other means, or ask the user to run them.",
            refused.join(", ")
        ));
    }
    Ok(json!({
        "description": prompt.description,
        "messages": [{"role": "user", "content": {"type": "text", "text": text}}],
    }))
}

fn start_family(a: &Args) -> String {
    let family = a.get("family_name").unwrap_or("Untitled");
    let glyphs = a.get("glyphs").map(|g| format!(" Add these glyphs with glyph.addGlyphs {{text}}: {g}.")).unwrap_or_default();
    let file = a
        .get("file")
        .map(|f| format!("Save it with file.saveAs {{path: {f}}}."))
        .unwrap_or_else(|| "Save it with file.saveAs, choosing a path ending in .fontcraft, .glyphs or .ufo.".to_string());
    format!(
        "Start a new type family named {family} in FontCraft.\n\n\
         1. Run file.new {{familyName: {family}}}. It opens a font with one Regular master and the starting glyph set.\n\
         2. Set the family's names and version with font.info.set {{values: {{familyName, versionMajor, versionMinor}}}}.\n\
         3. Read the master with font.master.list, then set ascender, cap height, x-height and descender with font.master.setMetric.\n\
         4.{glyphs} Draw outlines with path.addPath {{glyph, nodes}} (coordinates in font units, counter-clockwise outer contours), and read a layer back with layer.inspect to check it.\n\
         5. {file}"
    )
}

fn add_master(a: &Args) -> String {
    let name = a.get("name").unwrap_or("New Master");
    let axes = a.get("axes").map(|x| format!(" at {x}")).unwrap_or_default();
    let from = a.get("from_master").map(|m| format!("the master {m}")).unwrap_or_else(|| "the first master".to_string());
    format!(
        "Add a master named {name}{axes} to the open font.\n\n\
         1. List the masters and their axis locations with font.master.list.\n\
         2. Duplicate {from} with font.master.duplicate {{masters}}, so every glyph starts with compatible outlines, then name and place the copy with font.master.set {{masters, name, axes}}. (font.master.add {{name, axes}} adds an empty master instead.)\n\
         3. Set its metrics with font.master.setMetric.\n\
         4. Change each glyph's outlines on the new master's layer: read it with layer.inspect, redraw with path.addPath or the outline commands, keeping the node structure of the other masters.\n\
         5. Run audit.run and fix the compatibility findings, so the masters interpolate."
    )
}

fn build_accented(a: &Args) -> String {
    let glyphs = a.get("glyphs").unwrap_or("");
    format!(
        "Build these accented glyphs from components: {glyphs}\n\n\
         1. For each, glyphData.lookup {{name}} (or {{char}}) gives its name and what it decomposes to.\n\
         2. Add the missing ones with glyph.addGlyphs {{text}}.\n\
         3. Make sure the base letters and marks have anchors (top, bottom, _top, _bottom): glyph.setAnchors adds the default ones; adjust them with anchor.setPosition.\n\
         4. Build them with glyph.createComposite {{glyphs, allMasters: true}}. It names any part the font lacks.\n\
         5. Check the result with layer.inspect and audit.run {{glyphs}}."
    )
}

fn kern_pairs(a: &Args) -> String {
    let pairs = a.get("pairs").unwrap_or("");
    let master = a.get("master").map(|m| format!("the master {m}")).unwrap_or_else(|| "the first master".to_string());
    format!(
        "Kern these pairs on {master}:\n\n{pairs}\n\n\
         1. Find the master's id with font.master.list, and the kerning groups with kerning.listGroups (a side written @name is a group).\n\
         2. For a pair without a value, read the current one with kerning.getPair and propose a value from the outlines' spacing.\n\
         3. Set all pairs in one command.batch {{label: \"Kern pair list\", steps: [{{command: \"kerning.setPair\", params: {{left, right, value, master}}}}, …]}}, so they are one undo step and none applies if one fails.\n\
         4. Report the changes: each kerning.setPair result has the previous value; kerning.listPairs shows the master's pairs."
    )
}

fn prepare_release(a: &Args) -> String {
    let version = a
        .get("version")
        .map(|v| format!("Set the version to {v} with font.info.set {{values: {{versionMajor, versionMinor}}}}."))
        .unwrap_or_else(|| "Step the minor version with font.info.stepVersion.".to_string());
    let destination = a
        .get("destination")
        .map(|d| format!("{{destination: {d}, containers: [\"plain\", \"woff2\"]}}"))
        .unwrap_or_else(|| "{destination: a folder, containers: [\"plain\", \"woff2\"]}".to_string());
    format!(
        "Prepare the open font for release.\n\n\
         1. Read the current names and version with font.info.get. {version}\n\
         2. Date it with font.info.updateDate.\n\
         3. Run audit.run {{profile: \"strict\"}}. Fix what is fixable with audit.fix (it keeps backup layers), and report the rest to the user.\n\
         4. See what will be built with font.compilePlan, then export with file.exportStatic {destination}.\n\
         5. Verify each exported file with font.inspect {{input}} (names, version, glyph count).\n\
         6. Save the source with file.save."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_checks_arguments() {
        let prompt = find("kern_pair_list").unwrap();
        assert!(get(prompt, &json!({}), &[]).unwrap_err().contains("pairs"));
        assert!(get(prompt, &json!({"pairs": 3}), &[]).is_err());
        assert!(get(prompt, &json!({"pairs": "A V", "extra": "x"}), &[]).is_err());
        let r = get(prompt, &json!({"pairs": "A V -40"}), &["kerning.setPair"]).unwrap();
        let text = r["messages"][0]["content"]["text"].as_str().unwrap();
        assert!(text.contains("A V -40") && text.contains("refuses kerning.setPair"), "{text}");
    }
}
