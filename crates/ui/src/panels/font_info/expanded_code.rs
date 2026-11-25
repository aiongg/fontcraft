//! The expanded code of one build in place of an entry (`font.features.expandCode`): the code the feature compiler reads
//! for a master, an instance or the variable font, read-only.
//!
//! | Control | Does |
//! |---|---|
//! | the build menu | shows the code of another master, instance or the variable font |
//! | With Generated Features | the code an export compiles: the automatic code regenerated, the generated contextual mark code after it |
//! | the find field, Previous, Next (also Return in the field) | underlines the matches and scrolls to one |
//! | Copy | puts the whole code on the clipboard |
//! | Close | shows the entry again |
//!
//! The code comes from a worker (`crate::shell::feature_work`). The field shows its first megabyte.

use craft::theme::Srgba;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Sizing};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Env;
use craft::ui::style::TypeRole;
use craft::ui::text::TextField;
use craft_ext::ui::contract::reveal_rect;
use craft_ext::ui::text::{Highlight, HighlightText as _, field_layout};
use egui::text::CCursor;
use egui::{Color32, TextStyle, Ui};

use super::code_editor::{kept_runs, lines_in, match_chars};
use super::features_tab::scroll_box;
use crate::app::models::font_features::{ExpandedModel, MAX_FOUND};
use crate::app::{Msg, ViewMsg};
use crate::shell::font_info_tabs::{ExpandTarget, ExpandedView};

craft::ui::words! {
    /// The words of the expanded-code view.
    pub struct ExpandedWords {
        title: "fontcraftExpandedCode.title" = "Expanded Code";
        build: "fontcraftExpandedCode.build" = "Build";
        master: "fontcraftExpandedCode.master" = "Master";
        instance: "fontcraftExpandedCode.instance" = "Instance";
        variable: "fontcraftExpandedCode.variable" = "Variable Font";
        generated: "fontcraftExpandedCode.generated" = "With Generated Features";
        generated_tip: "fontcraftExpandedCode.generatedTip" =
            "Show the code an export compiles: automatic code regenerated, contextual mark attachment after it. Kerning and other mark attachment are generated without code.";
        find: "fontcraftExpandedCode.find" = "Find";
        previous: "fontcraftExpandedCode.previous" = "Previous";
        next: "fontcraftExpandedCode.next" = "Next";
        matches: "fontcraftExpandedCode.matches" = "matches";
        no_matches: "fontcraftExpandedCode.noMatches" = "No matches";
        copy: "fontcraftExpandedCode.copy" = "Copy";
        close: "fontcraftExpandedCode.close" = "Close";
        busy: "fontcraftExpandedCode.busy" = "Expanding the code…";
        cut: "fontcraftExpandedCode.cut" = "The first megabyte shows; Copy takes all of it.";
        no_code: "fontcraftExpandedCode.noCode" = "No feature code";
        refused: "fontcraftExpandedCode.refused" = "Parameter not applied:";
    }
}

/// Most characters of the find field.
const MAX_QUERY_CHARS: usize = 200;

/// Which match the view is at, and whether it is still to be scrolled to.
#[derive(Clone, Default)]
struct At {
    query: String,
    index: usize,
    reveal: bool,
}

/// The view, in a pane `height` points tall.
pub fn view(ui: &mut Ui, model: &ExpandedModel, height: f32, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<ExpandedWords>();
    let top = ui.cursor().min.y;
    let memory = egui::Id::unique("fontcraftExpandedCode.at");
    let mut at = ui.ctx().data(|d| d.get_temp::<At>(memory)).unwrap_or_default();
    if at.query != model.state.query {
        at = At { query: model.state.query.clone(), index: 0, reveal: true };
    }
    let mut next = model.state.clone();
    let mut closed = false;
    Row::new().id_salt("fontcraftExpandedCode.title").wrap(true).show(ui, |ui, _| {
        Text::new(words.title.clone()).role(TypeRole::Heading).show(ui);
        if Button::new(words.copy.clone()).enabled(!model.code.is_empty()).id_salt("fontcraftExpandedCode.copy").show(ui).clicked() {
            ui.ctx().copy_text(model.code.as_str().to_string());
        }
        if Button::new(words.close.clone()).id_salt("fontcraftExpandedCode.close").show(ui).clicked() {
            closed = true;
        }
    });
    Row::new().id_salt("fontcraftExpandedCode.target").wrap(true).show(ui, |ui, _| {
        let entries: Vec<Entry<ExpandTarget>> = model.targets.iter().map(|(t, name)| Entry::new(t.clone(), label(t, name, &words))).collect();
        if let Some(picked) = Select::new(words.build.clone(), Bind::given(model.target.clone()), &entries).show(ui).events.committed() {
            next.target = Some(picked.clone());
        }
        let check = Checkbox::new(Bind::given(model.state.generated)).label(words.generated.clone()).tooltip(words.generated_tip.clone());
        if let Some(on) = check.show(ui).events.committed() {
            next.generated = *on;
        }
    });
    find_row(ui, model, &words, &mut at, &mut next);
    if model.busy {
        Text::new(words.busy.clone()).shade(Shade::Muted).show(ui);
    } else if model.code.is_empty() && model.problem.is_none() {
        Text::new(words.no_code.clone()).shade(Shade::Muted).show(ui);
    }
    if model.shown.len() < model.code.len() {
        Text::new(words.cut.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    }
    for refused in &model.refused {
        Text::new(format!("{} {refused}", words.refused.as_str())).shade(Shade::Muted).show(ui);
    }
    if let Some(problem) = &model.problem {
        Text::new(problem.clone()).show(ui);
    }
    if !model.shown.is_empty() {
        let gap = Env::of(ui).tokens.space.gap.row;
        let room = height - (ui.cursor().min.y - top) - gap;
        let row = ui.ctx().fonts_mut(|fonts| fonts.row_height(&TextStyle::Monospace.resolve(ui.style()))).max(1.0);
        let lines = lines_in(room, 1.5 * row);
        scroll_box(ui, ("fontcraftExpandedCode.box", model.target.clone()), lines as f32 * 1.5 * row, |ui| code(ui, model, lines, &words, &mut at));
    }
    ui.ctx().data_mut(|d| d.insert_temp(memory, at));
    if closed {
        sent.push(Msg::View(ViewMsg::FeatureExpanded(None)));
    } else if next != model.state {
        sent.push(Msg::View(ViewMsg::FeatureExpanded(Some(next))));
    }
}

/// How a build is named in the menu.
fn label(target: &ExpandTarget, name: &str, words: &ExpandedWords) -> String {
    match target {
        ExpandTarget::Master(_) => format!("{} {name}", words.master.as_str()),
        ExpandTarget::Instance(_) => format!("{} {name}", words.instance.as_str()),
        ExpandTarget::Variable => words.variable.as_str().to_string(),
    }
}

/// The find field with its count and buttons.
fn find_row(ui: &mut Ui, model: &ExpandedModel, words: &ExpandedWords, at: &mut At, next: &mut ExpandedView) {
    let count = model.found.len();
    Row::new().id_salt("fontcraftExpandedCode.findRow").wrap(true).show(ui, |ui, _| {
        let field = TextField::search(words.find.clone(), Bind::given(model.state.query.clone()))
            .placeholder(words.find.clone())
            .limit(MAX_QUERY_CHARS)
            .submits()
            .id_salt("fontcraftExpandedCode.find")
            .show(ui);
        if let Some(text) = field.events.value() {
            next.query = text.clone();
        }
        let mut step = field.submitted.then_some(1);
        let text = match count {
            _ if model.state.query.is_empty() => String::new(),
            0 => words.no_matches.as_str().to_string(),
            _ => format!("{}/{count}{} {}", at.index.min(count - 1) + 1, if count >= MAX_FOUND { "+" } else { "" }, words.matches.as_str()),
        };
        Text::new(text).shade(Shade::Muted).show(ui);
        if Button::new(words.previous.clone()).enabled(count > 0).id_salt("fontcraftExpandedCode.previous").show(ui).clicked() {
            step = Some(count.saturating_sub(1));
        }
        if Button::new(words.next.clone()).enabled(count > 0).id_salt("fontcraftExpandedCode.next").show(ui).clicked() {
            step = Some(1);
        }
        if let Some(step) = step.filter(|_| count > 0) {
            at.index = (at.index.min(count - 1) + step) % count;
            at.reveal = true;
        }
    });
}

/// The code field, the matches underlined and the one the view is at filled, scrolled to when it is new.
fn code(ui: &mut Ui, model: &ExpandedModel, lines: usize, words: &ExpandedWords, at: &mut At) {
    let env = Env::of(ui);
    let out = TextField::new(words.title.clone(), Bind::given(model.shown.as_str().to_string()))
        .mono()
        .lines(lines)
        .sizing(Sizing::FillWidth)
        .limit(None)
        .enabled(false)
        .id_salt("fontcraftExpandedCode.code")
        .show(ui);
    let current = model.found.get(at.index.min(model.found.len().saturating_sub(1))).copied();
    let fill = Color32::from(Srgba::from(env.tokens.color.accent.soft));
    let runs_id = out.response.id.with("fontcraftExpandedCode.runs");
    out.highlight_text(ui, |text| {
        let mut runs = kept_runs(ui.ctx(), runs_id, text, &[], &model.found, &env.tokens);
        runs.extend(current.map(|(start, end)| Highlight::background(start..end, fill)));
        runs
    });
    if !at.reveal {
        return;
    }
    let Some((pos, galley)) = field_layout(ui, &out.response) else {
        ui.ctx().request_repaint();
        return;
    };
    at.reveal = false;
    if let Some(range) = current
        && let Some((start, _)) = match_chars(&model.shown, &[range]).first().copied()
    {
        reveal_rect(ui, galley.pos_from_cursor(CCursor::new(start)).translate(pos.to_vec2()));
    }
}
