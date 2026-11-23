//! The find bar of the feature code editor (`fontInfo.features.findBar`), above the code while it is open.
//!
//! | Control | Posts |
//! |---|---|
//! | the query, the options and the scope | `ViewMsg::FeatureFind` with the new state; the projection lists the matches with `font.features.find` |
//! | Previous, Next, Return in the query | a [`FindAction`] the code editor carries out on its field |
//! | Replace, Replace All | a [`FindAction`]; the code editor posts `font.features.replace` |
//! | Close | `ViewMsg::FeatureFind(None)` |

use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Env;
use craft::ui::runtime::focus::{self, FocusTarget};
use craft::ui::text::TextField;
use egui::Ui;

use crate::app::models::font_features::{EntryPane, FindModel};
use crate::app::{Msg, ViewMsg};

craft::ui::words! {
    /// The words of the feature code editor's find bar.
    pub struct CodeFindWords {
        find: "fontcraftFeatureFind.find" = "Find";
        replace_with: "fontcraftFeatureFind.replaceWith" = "Replace with";
        replace: "fontcraftFeatureFind.replace" = "Replace";
        replace_all: "fontcraftFeatureFind.replaceAll" = "Replace All";
        previous: "fontcraftFeatureFind.previous" = "Previous";
        next: "fontcraftFeatureFind.next" = "Next";
        match_case: "fontcraftFeatureFind.matchCase" = "Match Case";
        whole_word: "fontcraftFeatureFind.wholeWord" = "Whole Word";
        regex: "fontcraftFeatureFind.regex" = "Regular Expression";
        scope: "fontcraftFeatureFind.scope" = "Search in";
        this_entry: "fontcraftFeatureFind.thisEntry" = "This Entry";
        all_entries: "fontcraftFeatureFind.allEntries" = "All Entries";
        close: "fontcraftFeatureFind.close" = "Close Find";
        no_matches: "fontcraftFeatureFind.noMatches" = "No matches";
        matches: "fontcraftFeatureFind.matches" = "matches";
        here: "fontcraftFeatureFind.here" = "here";
    }
}

/// Most characters of the query.
pub const MAX_QUERY_CHARS: usize = 1_000;
/// Most characters of the replacement.
pub const MAX_REPLACEMENT_CHARS: usize = 16_000;

/// What the find bar asks the code editor to do with its field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindAction {
    Next,
    Previous,
    Replace,
    ReplaceAll,
}

/// Draws the find bar while `entry.find` holds one; returns the action a button or Return in the query asked for.
pub fn bar(ui: &mut Ui, entry: &EntryPane, sent: &mut Vec<Msg>) -> Option<FindAction> {
    let found = entry.find.as_ref()?;
    let state = &found.state;
    let words = Env::of(ui).words::<CodeFindWords>();
    let mut action = None;
    let mut next = state.clone();
    let mut closed = false;
    let any = found.total > 0;
    // Three rows, each narrower than the Features tab's code pane at the panel's default size: a wrapping row of
    // craft squeezes a checkbox it ends a line with to the width left, its label then a column of letters.
    Row::new().wrap(true).show(ui, |ui, _| {
        let query = TextField::search(words.find.clone(), Bind::given(state.query.clone()))
            .placeholder(words.find.clone())
            .limit(MAX_QUERY_CHARS)
            .submits()
            .id_salt("fontcraftFeatureFind.query")
            .show(ui);
        if state.focus {
            let _ = focus::request(ui.ctx(), FocusTarget::Id(query.response.id));
            next.focus = false;
        }
        if let Some(q) = query.events.value() {
            next.query = q.clone();
        }
        if query.submitted && any {
            action = Some(FindAction::Next);
        }
        Text::new(count_text(found, &words)).shade(Shade::Muted).show(ui);
        if Button::new(words.previous.clone()).enabled(any).id_salt("fontcraftFeatureFind.previous").show(ui).clicked() && any {
            action = Some(FindAction::Previous);
        }
        if Button::new(words.next.clone()).enabled(any).id_salt("fontcraftFeatureFind.next").show(ui).clicked() && any {
            action = Some(FindAction::Next);
        }
        if Button::new(words.close.clone()).id_salt("fontcraftFeatureFind.close").show(ui).clicked() {
            closed = true;
        }
    });
    Row::new().wrap(true).show(ui, |ui, _| {
        let with = TextField::new(words.replace_with.clone(), Bind::given(state.replacement.clone()))
            .placeholder(words.replace_with.clone())
            .limit(MAX_REPLACEMENT_CHARS)
            .id_salt("fontcraftFeatureFind.replacement")
            .show(ui);
        if let Some(r) = with.events.value() {
            next.replacement = r.clone();
        }
        let here = !entry.automatic && !found.here.is_empty();
        if Button::new(words.replace.clone()).enabled(here).id_salt("fontcraftFeatureFind.replaceOne").show(ui).clicked() && here {
            action = Some(FindAction::Replace);
        }
        let all = any && (state.all_entries || !entry.automatic);
        if Button::new(words.replace_all.clone()).enabled(all).id_salt("fontcraftFeatureFind.replaceAll").show(ui).clicked() && all {
            action = Some(FindAction::ReplaceAll);
        }
    });
    Row::new().wrap(true).show(ui, |ui, _| {
        let scopes = vec![Entry::new("entry".to_string(), words.this_entry.clone()), Entry::new("all".to_string(), words.all_entries.clone())];
        let shown = if state.all_entries { "all" } else { "entry" }.to_string();
        if let Some(picked) = Select::new(words.scope.clone(), Bind::given(shown), &scopes).show(ui).events.committed() {
            next.all_entries = picked.as_str() == "all";
        }
        if let Some(on) = Checkbox::new(Bind::given(state.match_case)).label(words.match_case.clone()).show(ui).events.committed() {
            next.match_case = *on;
        }
        if let Some(on) = Checkbox::new(Bind::given(state.whole_word)).label(words.whole_word.clone()).show(ui).events.committed() {
            next.whole_word = *on;
        }
        if let Some(on) = Checkbox::new(Bind::given(state.regex)).label(words.regex.clone()).show(ui).events.committed() {
            next.regex = *on;
        }
    });
    if closed {
        sent.push(Msg::View(ViewMsg::FeatureFind(None)));
    } else if next != *state {
        sent.push(Msg::View(ViewMsg::FeatureFind(Some(next))));
    }
    action
}

/// The count beside the query: why it cannot run, nothing while it is empty, else the matches (in the shown entry
/// too, in the all-entries scope).
fn count_text(found: &FindModel, words: &CodeFindWords) -> String {
    if let Some(problem) = &found.problem {
        return problem.clone();
    }
    if found.state.query.is_empty() {
        return String::new();
    }
    if found.total == 0 {
        return words.no_matches.as_str().to_string();
    }
    let more = if found.truncated { "+" } else { "" };
    let mut text = format!("{}{more} {}", found.total, words.matches.as_str());
    if found.state.all_entries {
        text = format!("{text}, {} {}", found.here.len(), words.here.as_str());
    }
    text
}
