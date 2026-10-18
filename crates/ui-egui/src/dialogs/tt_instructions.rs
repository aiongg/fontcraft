//! The TrueType Instructions dialog (`dialog.ttInstructions`, `hint.tt.editInstructions`): the
//! raw program of a glyph, or the font program (`fpgm`) or control value program (`prep`), as a
//! listing of its instructions, and as assembly text to edit.
//!
//! | Control | Does |
//! |---|---|
//! | listing | one row per instruction: its byte offset and the instruction as the disassembler writes it; hovering a row explains the instruction |
//! | Edit | puts the stored text in a text area |
//! | Parse | runs `hint.tt.setAssembly` with the text: stored, the dialog shows the listing again; refused, the text stays with the message under it and nothing changes |
//! | Cancel | drops the text and shows the listing |
//!
//! The dialog reads the font every frame, so it follows undo and edits made elsewhere. It blocks
//! nothing. Without a master, a glyph's program is its main master layer's, and the `fpgm` or
//! `prep` text Parse stores goes to every master, as `hint.tt.setAssembly` does without one.

use fontcraft_engine::cmd::tt_assembly::{ListedInstruction, ProgramTarget, StoredProgram, assemble, listing, master_for, stored_program};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::panels::{Pending, current_glyph, widgets};
use crate::shell::{Args, UiCommand};

pub const ID: &str = "hint.tt.editInstructions";

/// Most listing rows the command's result carries; the dialog shows every row.
pub const MAX_REPLY_ROWS: usize = 10_000;
/// Rows of the text area.
const EDIT_ROWS: usize = 18;
/// Listing rows visible before it scrolls.
const VISIBLE_ROWS: f32 = 18.0;

pub const COMMANDS: &[UiCommand] = &[(
    ID,
    "Edit TrueType Instructions…",
    &[],
    "",
    "{target?: {glyph: name} | \"fpgm\" | \"prep\" (default the edited glyph, else the first glyph selected in Font View), master?: id (a glyph: the layer of that master, default the main master; fpgm and prep: shown from that master and stored to it only, default the main master shown and every master stored), open?: bool (false closes the dialog)} opens the TrueType Instructions dialog on the program: one row per instruction with its byte offset and an explanation on hover; Edit turns it into assembly text, and Parse stores the text with `hint.tt.setAssembly` and shows the listing again, or keeps the text with the message when it is refused → {open, target, master, assembly: text | null, stale: bool (a glyph program whose outline changed since; export leaves it out), listing: [{offset, text, gloss}] (at most 10000 rows), truncated: bool}",
)];

/// The dialog's state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TtInstructionsState {
    pub open: bool,
    /// The program shown, as `hint.tt.setAssembly` takes its `target`.
    pub target: Value,
    /// The master given when the dialog opened; `None` for the main master (see the module notes).
    pub master: Option<String>,
    /// The text area is showing.
    #[serde(skip)]
    pub editing: bool,
    #[serde(skip)]
    pub draft: String,
    /// Why the last Parse was refused.
    #[serde(skip)]
    pub error: Option<String>,
}

pub fn edit_button_id() -> egui::Id {
    widgets::id((ID, "edit"))
}

pub fn parse_button_id() -> egui::Id {
    widgets::id((ID, "parse"))
}

pub fn cancel_button_id() -> egui::Id {
    widgets::id((ID, "cancel"))
}

pub fn text_id() -> egui::Id {
    widgets::id((ID, "text"))
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != ID {
        return Err(format!("unknown command `{id}`"));
    }
    if args.flag("open")? == Some(false) {
        app.state.panels.tt_instructions.open = false;
        return Ok(json!({"open": false}));
    }
    let target = match args.get("target") {
        Some(v) => ProgramTarget::from_json(v).map_err(|e| args.err(e))?,
        None => ProgramTarget::Glyph(current_glyph(app).ok_or_else(|| args.err("no glyph is edited or selected: give `target`"))?),
    };
    let master = args.string("master")?.map(str::to_string);
    let font = app.session.font().map_err(|e| args.err(e))?;
    let shown_master = master_for(font, master.as_deref()).map_err(|e| args.err(e))?;
    let stored = stored_program(font, &target, Some(&shown_master)).map_err(|e| args.err(e))?;
    let reply = reply(&target, &shown_master, stored.as_ref());
    app.state.panels.tt_instructions =
        TtInstructionsState { open: true, target: target.to_json(), master, editing: false, draft: String::new(), error: None };
    Ok(reply)
}

fn reply(target: &ProgramTarget, master: &str, stored: Option<&StoredProgram>) -> Value {
    let rows = stored.and_then(|p| assemble(&p.text).ok()).and_then(|a| listing(&a.bytes).ok()).unwrap_or_default();
    let shown: Vec<Value> =
        rows.iter().take(MAX_REPLY_ROWS).map(|r| json!({"offset": r.offset, "text": r.text.trim_start(), "gloss": r.gloss})).collect();
    json!({
        "open": true,
        "target": target.to_json(),
        "master": master,
        "assembly": stored.map(|p| p.text.as_str()),
        "stale": stored.is_some_and(|p| p.stale),
        "listing": shown,
        "truncated": rows.len() > MAX_REPLY_ROWS,
    })
}

/// Puts the stored text in the text area.
pub fn begin_edit(app: &mut FontcraftApp) {
    let st = &app.state.panels.tt_instructions;
    let text = ProgramTarget::from_json(&st.target)
        .ok()
        .and_then(|t| app.session.font().ok().and_then(|font| stored_program(font, &t, st.master.as_deref()).ok().flatten()))
        .map(|p| p.text)
        .unwrap_or_default();
    let st = &mut app.state.panels.tt_instructions;
    st.draft = text;
    st.editing = true;
    st.error = None;
}

/// Parse: stores the text with `hint.tt.setAssembly`. Stored, the listing shows again; refused,
/// the text stays and the message shows under it.
pub fn parse(app: &mut FontcraftApp) -> Result<Value, String> {
    let st = &app.state.panels.tt_instructions;
    let mut params = json!({"target": st.target, "assembly": st.draft});
    if let (Some(m), Some(obj)) = (&st.master, params.as_object_mut()) {
        obj.insert("master".into(), json!(m));
    }
    let result = app.run("hint.tt.setAssembly", &params);
    let st = &mut app.state.panels.tt_instructions;
    match &result {
        Ok(_) => {
            st.editing = false;
            st.draft.clear();
            st.error = None;
        }
        Err(e) => st.error = Some(e.clone()),
    }
    result
}

/// What the person pressed this frame.
enum Press {
    Edit,
    Parse,
    Cancel,
}

fn title(target: &ProgramTarget) -> String {
    match target {
        ProgramTarget::Glyph(name) => format!("TrueType Instructions: {name}"),
        ProgramTarget::Fpgm => "TrueType Instructions: font program (fpgm)".into(),
        ProgramTarget::Prep => "TrueType Instructions: control value program (prep)".into(),
    }
}

/// Draws the dialog while it is open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, _pending: &mut Pending) {
    if !app.state.panels.tt_instructions.open {
        return;
    }
    let mut st = app.state.panels.tt_instructions.clone();
    let Ok(target) = ProgramTarget::from_json(&st.target) else {
        app.state.panels.tt_instructions.open = false;
        return;
    };
    let mut open = true;
    let mut press = None;
    {
        let shown: &FontcraftApp = app;
        let tokens = &shown.tokens;
        egui::Window::new(title(&target))
            .id(egui::Id::new(ID))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_pos(egui::pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
            .show(ui.ctx(), |ui| {
                let Ok(font) = shown.session.font() else { return };
                if st.editing {
                    ui.add(
                        egui::TextEdit::multiline(&mut st.draft)
                            .id(text_id())
                            .code_editor()
                            .desired_rows(EDIT_ROWS)
                            .desired_width(tokens.dialog_field_width * 2.0),
                    );
                    if let Some(e) = &st.error {
                        ui.label(egui::RichText::new(e).color(tokens.error_text));
                    }
                    ui.horizontal(|ui| {
                        if widgets::button(ui, parse_button_id(), "Parse", "Check the text and store it", true, false) {
                            press = Some(Press::Parse);
                        }
                        if widgets::button(ui, cancel_button_id(), "Cancel", "Drop the text and show the listing", true, false) {
                            press = Some(Press::Cancel);
                        }
                    });
                    return;
                }
                match stored_program(font, &target, st.master.as_deref()) {
                    Err(e) => {
                        ui.label(egui::RichText::new(e).color(tokens.text_dim));
                        return;
                    }
                    Ok(None) => {
                        ui.label(egui::RichText::new("No instructions.").color(tokens.text_dim));
                    }
                    Ok(Some(p)) => {
                        if p.stale {
                            ui.label(
                                egui::RichText::new("The outline changed since these instructions were stored, so export leaves them out.")
                                    .color(tokens.error_text),
                            );
                        }
                        match assemble(&p.text).and_then(|a| listing(&a.bytes)) {
                            Ok(rows) => listing_rows(ui, tokens, &rows),
                            Err(e) => {
                                ui.label(egui::RichText::new(format!("The stored text cannot be assembled: {e}")).color(tokens.error_text));
                            }
                        }
                    }
                }
                if widgets::button(ui, edit_button_id(), "Edit", "Change the instructions as assembly text", true, false) {
                    press = Some(Press::Edit);
                }
            });
    }
    let draft = std::mem::take(&mut st.draft);
    let state = &mut app.state.panels.tt_instructions;
    state.open = open;
    if state.editing {
        state.draft = draft;
    }
    match press {
        Some(Press::Edit) => begin_edit(app),
        // A refusal shows in the dialog.
        Some(Press::Parse) => _ = parse(app),
        Some(Press::Cancel) => {
            let state = &mut app.state.panels.tt_instructions;
            state.editing = false;
            state.draft.clear();
            state.error = None;
        }
        None => {}
    }
}

fn listing_rows(ui: &mut egui::Ui, tokens: &crate::theme::Tokens, rows: &[ListedInstruction]) {
    let height = tokens.list_row_height;
    egui::ScrollArea::vertical().id_salt((ID, "listing")).max_height(height * VISIBLE_ROWS).show_rows(ui, height, rows.len(), |ui, range| {
        for row in rows.get(range).unwrap_or_default() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{:>5}", row.offset)).monospace().color(tokens.text_dim));
                let label = ui.label(egui::RichText::new(&row.text).monospace());
                if !row.gloss.is_empty() {
                    label.on_hover_text(row.gloss);
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::sync::Arc;

    use fontcraft_engine::model::{Font, Path, Shape};

    use super::*;

    fn app() -> FontcraftApp {
        let mut font = Font::new("Instructions");
        font.glyph_mut("H").unwrap().layers[0].shapes = vec![Shape::Path(Path::rect(50.0, 0.0, 150.0, 700.0))];
        let mut session = fontcraft_engine::Session::new();
        session.add_document(font, None).unwrap();
        FontcraftApp::new(session)
    }

    fn frame(app: &mut FontcraftApp, ctx: &egui::Context) {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        ctx.run_ui(input, |ui| app.ui(ui)).textures_delta.clear();
    }

    #[test]
    fn dialog_tt_instructions_edits_parses_and_lists_a_glyph_program() {
        let mut app = app();
        let ctx = egui::Context::default();
        let r = app.run(ID, &json!({"target": {"glyph": "H"}})).unwrap();
        assert_eq!((r["open"].as_bool(), r["assembly"].is_null()), (Some(true), true));
        frame(&mut app, &ctx);

        begin_edit(&mut app);
        app.state.panels.tt_instructions.draft = "SVTCA[0]\nMDAP[1] 0".into();
        frame(&mut app, &ctx);
        parse(&mut app).unwrap();
        assert!(!app.state.panels.tt_instructions.editing, "back to the listing");
        frame(&mut app, &ctx);

        let r = app.run(ID, &json!({"target": {"glyph": "H"}})).unwrap();
        let rows: Vec<&str> = r["listing"].as_array().unwrap().iter().map(|row| row["text"].as_str().unwrap()).collect();
        assert_eq!(rows, ["SVTCA[0]", "PUSHB[ ] 0", "MDAP[1]"]);
        assert_eq!(r["listing"][0]["offset"], 0);
        assert!(!r["listing"][0]["gloss"].as_str().unwrap().is_empty());
    }

    #[test]
    fn dialog_tt_instructions_keeps_the_text_when_parse_is_refused() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.run(ID, &json!({"target": "prep"})).unwrap();
        begin_edit(&mut app);
        app.state.panels.tt_instructions.draft = "PUSHB[ ] 1\nNOSUCH[ ]".into();
        let before = app.session.doc().unwrap().font.clone();
        assert!(parse(&mut app).is_err());
        assert!(Arc::ptr_eq(&app.session.doc().unwrap().font, &before), "a parse error changes nothing");
        let st = &app.state.panels.tt_instructions;
        assert!(st.editing && st.draft == "PUSHB[ ] 1\nNOSUCH[ ]");
        assert!(st.error.as_deref().is_some_and(|e| e.contains("line 2")));
        frame(&mut app, &ctx);
        app.run(ID, &json!({"open": false})).unwrap();
        frame(&mut app, &ctx);
        assert!(!app.state.panels.tt_instructions.open);
    }

    #[test]
    fn dialog_tt_instructions_needs_a_glyph_or_a_font_program() {
        let mut app = app();
        assert!(app.run(ID, &json!({})).is_err(), "nothing edited or selected");
        assert!(app.run(ID, &json!({"target": {"glyph": "Missing"}})).is_err());
        assert!(app.run(ID, &json!({"target": "cvt"})).is_err());
        assert!(!app.state.panels.tt_instructions.open);
    }
}
