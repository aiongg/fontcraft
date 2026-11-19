//! The TrueType Instructions dialog (`dialog.ttInstructions`, `hint.tt.editInstructions`): the raw program of a
//! glyph, or the font program (`fpgm`) or control value program (`prep`), as a listing of its instructions, and as
//! assembly text to edit.
//!
//! | Control | Does |
//! |---|---|
//! | listing | one row per instruction: its byte offset and the instruction as the disassembler writes it; its tooltip explains the instruction |
//! | Edit | puts the stored text in a text area |
//! | Parse | assembles the text: refused, the text stays with the message under it and nothing changes; assembled, it runs `hint.tt.setAssembly` with the text and shows the listing again |
//! | Cancel | drops the text and shows the listing |
//!
//! The dialog module reads the program again whenever the document moves, so the dialog follows undo and edits made
//! elsewhere. It blocks nothing. Without a master, a glyph's program is its main master layer's, and the `fpgm` or
//! `prep` text Parse stores goes to every master, as `hint.tt.setAssembly` does without one.

use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::runtime::env::Env;
use craft::ui::text::TextField;
use egui::Ui;
use fontcraft_engine::cmd::tt_assembly::{ProgramTarget, assemble, listing, stored_program};
use fontcraft_engine::model::Font;
use serde_json::{Value, json};

use crate::app::Msg;

/// Rows of the text area.
const EDIT_ROWS: usize = 18;

craft::ui::words! {
    /// The words of the TrueType Instructions dialog.
    pub struct InstructionsWords {
        /// The title over a glyph program.
        glyph_title: template "fontcraftTtInstructions.glyphTitle" = "TrueType Instructions: {glyph}";
        /// The title over the font program.
        fpgm_title: "fontcraftTtInstructions.fpgmTitle" = "TrueType Instructions: font program (fpgm)";
        /// The title over the control value program.
        prep_title: "fontcraftTtInstructions.prepTitle" = "TrueType Instructions: control value program (prep)";
        /// A program with no instructions.
        none: "fontcraftTtInstructions.none" = "No instructions.";
        /// A glyph program whose outline changed since it was stored.
        stale: "fontcraftTtInstructions.stale" = "The outline changed since these instructions were stored, so export leaves them out.";
        /// The stored text does not assemble.
        broken: template "fontcraftTtInstructions.broken" = "The stored text cannot be assembled: {error}";
        /// The Edit button.
        edit: "fontcraftTtInstructions.edit" = "Edit";
        /// The tooltip of the Edit button.
        edit_tip: "fontcraftTtInstructions.editTip" = "Change the instructions as assembly text";
        /// The text area.
        text: "fontcraftTtInstructions.text" = "Assembly";
        /// The Parse button.
        parse: "fontcraftTtInstructions.parse" = "Parse";
        /// The tooltip of the Parse button.
        parse_tip: "fontcraftTtInstructions.parseTip" = "Check the text and store it";
        /// The Cancel button.
        cancel: "fontcraftTtInstructions.cancel" = "Cancel";
        /// The tooltip of the Cancel button.
        cancel_tip: "fontcraftTtInstructions.cancelTip" = "Drop the text and show the listing";
    }
}

/// The program the dialog shows: the target as `hint.tt.setAssembly` takes it, and the master given when it opened
/// (`None` for the main master).
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub target: Value,
    pub master: Option<String>,
}

/// One listing row.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub offset: usize,
    pub text: String,
    pub gloss: String,
}

/// The stored program as the dialog shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Program {
    /// The stored text; `None` without instructions.
    pub text: Option<String>,
    pub stale: bool,
    pub listing: Vec<Line>,
    /// Why the program cannot be read or listed.
    pub error: Option<String>,
    /// The stored text does not assemble.
    pub broken: Option<String>,
}

impl Program {
    /// The program `request` names in `font`.
    pub fn read(font: &Font, request: &Request) -> Program {
        let stored = ProgramTarget::from_json(&request.target).and_then(|target| stored_program(font, &target, request.master.as_deref()));
        match stored {
            Err(error) => Program { error: Some(error), ..Program::default() },
            Ok(None) => Program::default(),
            Ok(Some(p)) => {
                let (listing, broken) = match assemble(&p.text).and_then(|a| listing(&a.bytes)) {
                    Ok(rows) => {
                        let lines = rows.into_iter().map(|r| Line { offset: r.offset, text: r.text, gloss: r.gloss.to_string() }).collect();
                        (lines, None)
                    }
                    Err(e) => (Vec::new(), Some(e.to_string())),
                };
                Program { text: Some(p.text), stale: p.stale, listing, error: None, broken }
            }
        }
    }
}

/// The open dialog on the window's dialog host.
pub struct InstructionsDialog {
    pub request: Request,
    /// The stored program, refreshed by the dialog module when the document moves.
    pub program: Program,
    /// The text area's text while it shows.
    pub draft: Option<String>,
    /// Why the last Parse was refused.
    pub error: Option<String>,
    /// The commands the dialog posts.
    pub outbox: Vec<Msg>,
    /// Set from outside (`hint.tt.editInstructions {open: false}`): the dialog closes in its next pass.
    pub closing: bool,
    open: bool,
}

impl InstructionsDialog {
    pub fn new(request: Request) -> Self {
        InstructionsDialog { request, program: Program::default(), draft: None, error: None, outbox: Vec::new(), closing: false, open: true }
    }

    /// Parse: assembles the draft; assembled, posts `hint.tt.setAssembly` and shows the listing again.
    fn parse(&mut self) {
        let Some(draft) = self.draft.clone() else { return };
        if let Err(e) = assemble(&draft) {
            self.error = Some(e.to_string());
            return;
        }
        let mut params = json!({"target": self.request.target, "assembly": draft});
        if let (Some(master), Some(object)) = (&self.request.master, params.as_object_mut()) {
            object.insert("master".into(), json!(master));
        }
        self.outbox.push(Msg::run("hint.tt.setAssembly", params));
        self.draft = None;
        self.error = None;
    }
}

/// What the person pressed in a pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Edit,
    Parse,
    Cancel,
}

/// The id of a button: `edit`, `parse` or `cancel`; or of the text area, `text`.
pub fn control_id(part: &str) -> egui::Id {
    egui::Id::unique(("dialog.ttInstructions", part))
}

/// Shows the listing of `program`, or the text area over `draft` while it is `Some`.
pub fn view(ui: &mut Ui, program: &Program, draft: &mut Option<String>, error: Option<&str>) -> Option<Press> {
    let words = Env::of(ui).words::<InstructionsWords>();
    let mut press = None;
    Stack::new().show(ui, |ui, _| {
        if let Some(text) = draft.as_mut() {
            TextField::new(words.text.clone(), Bind::bound(text)).id(control_id("text")).mono().lines(EDIT_ROWS).tab_inserts(true).show(ui);
            if let Some(error) = error {
                Text::new(error.to_string()).shade(Shade::Muted).show(ui);
            }
            Row::new().show(ui, |ui, _| {
                if Button::new(words.parse.clone()).id(control_id("parse")).tooltip(words.parse_tip.clone()).show(ui).clicked() {
                    press = Some(Press::Parse);
                }
                if Button::new(words.cancel.clone()).id(control_id("cancel")).tooltip(words.cancel_tip.clone()).show(ui).clicked() {
                    press = Some(Press::Cancel);
                }
            });
            return;
        }
        if let Some(error) = &program.error {
            Text::new(error.clone()).shade(Shade::Muted).show(ui);
            return;
        }
        if program.text.is_none() {
            Text::new(words.none.clone()).shade(Shade::Muted).show(ui);
        }
        if program.stale {
            Text::new(words.stale.clone()).show(ui);
        }
        if let Some(broken) = &program.broken {
            Text::new(words.broken.fill(&[("error", broken.as_str())])).show(ui);
        }
        Stack::new().scroll(craft::ui::contract::Scroll::vertical()).show(ui, |ui, _| {
            for line in &program.listing {
                Row::new().show(ui, |ui, _| {
                    Text::new(format!("{:>5}", line.offset)).shade(Shade::Muted).show(ui);
                    let text = Text::new(line.text.clone());
                    if line.gloss.is_empty() {
                        text.show(ui)
                    } else {
                        text.tooltip(line.gloss.clone()).show(ui)
                    };
                });
            }
        });
        if Button::new(words.edit.clone()).id(control_id("edit")).tooltip(words.edit_tip.clone()).show(ui).clicked() {
            press = Some(Press::Edit);
        }
    });
    press
}

impl StackDialog for InstructionsDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id("fontcraftTtInstructions")
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        if self.closing {
            return Some(DialogResult::Cancel);
        }
        let words = Env::of(ui).words::<InstructionsWords>();
        let title = match ProgramTarget::from_json(&self.request.target) {
            Ok(ProgramTarget::Glyph(name)) => words.glyph_title.fill(&[("glyph", name.as_str())]).to_string(),
            Ok(ProgramTarget::Fpgm) => words.fpgm_title.as_str().to_string(),
            Ok(ProgramTarget::Prep) => words.prep_title.as_str().to_string(),
            Err(_) => return Some(DialogResult::Cancel),
        };
        let InstructionsDialog { program, draft, error, open, .. } = self;
        let mut press = None;
        let shown = Dialog::new(title).opened(Bind::bound(open)).modeless().resizable(true).show(ui, |ui, _| {
            press = view(ui, program, draft, error.as_deref());
        });
        match press {
            Some(Press::Edit) => {
                self.draft = Some(self.program.text.clone().unwrap_or_default());
                self.error = None;
            }
            Some(Press::Parse) => self.parse(),
            Some(Press::Cancel) => {
                self.draft = None;
                self.error = None;
            }
            None => {}
        }
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;

    use super::*;

    /// Edit fills the text area with the stored text; Parse of text that does not assemble keeps it with the message
    /// and posts nothing; Parse of good text posts `hint.tt.setAssembly` and shows the listing again.
    #[test]
    fn edit_and_parse_store_only_text_that_assembles() {
        let dialog = Rc::new(RefCell::new(InstructionsDialog::new(Request { target: json!({"glyph": "H"}), master: None })));
        dialog.borrow_mut().program = Program { text: Some("SVTCA[0]".into()), ..Program::default() };
        let shown = Rc::clone(&dialog);
        let mut f = Fixture::new(move |ui| {
            let mut d = shown.borrow_mut();
            let InstructionsDialog { program, draft, error, .. } = &mut *d;
            let press = view(ui, program, draft, error.as_deref());
            match press {
                Some(Press::Edit) => d.draft = d.program.text.clone(),
                Some(Press::Parse) => d.parse(),
                Some(Press::Cancel) => d.draft = None,
                None => {}
            }
        });
        f.settle();
        let edit = f.node_of(control_id("edit")).expect("Edit");
        f.click(&edit).settle();
        assert_eq!(dialog.borrow().draft.as_deref(), Some("SVTCA[0]"));
        dialog.borrow_mut().draft = Some("NOT AN INSTRUCTION".into());
        f.settle();
        let parse = f.node_of(control_id("parse")).expect("Parse");
        f.click(&parse).settle();
        assert!(dialog.borrow().error.is_some(), "a refused text shows why");
        assert!(dialog.borrow().outbox.is_empty());
        dialog.borrow_mut().draft = Some("SVTCA[1]".into());
        f.settle();
        let parse = f.node_of(control_id("parse")).expect("Parse");
        f.click(&parse).settle();
        let d = dialog.borrow();
        assert_eq!(d.outbox, vec![Msg::run("hint.tt.setAssembly", json!({"target": {"glyph": "H"}, "assembly": "SVTCA[1]"}))]);
        assert_eq!(d.draft, None);
    }
}
