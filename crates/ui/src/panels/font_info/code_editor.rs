//! The feature code editor of the Features tab: a monospaced field of several lines over the shown entry's code, and
//! the last compile's diagnostics in that entry under it.
//!
//! | Control | Posts |
//! |---|---|
//! | the code | `font.features.setCode` when the field loses the keyboard and the text changed; read-only for an automatic entry |
//! | a diagnostic's Fix button | `font.features.applyFix {error, fix}` |
//!
//! The field's last selection goes back to the Features tab, whose snippets menu puts a snippet in its place.
//!
//! Typing a name character, `@`, `$` or `{` asks for the names that fit at the caret (`ViewMsg::FeatureCompletion`,
//! answered by the projection with `font.features.complete`); the list under the field shows them once the answer
//! matches the code as typed, and Tab, Return or a click puts the picked name in (`fontInfo.features.autocomplete`).
//!
//! The diagnostics show only while the font is unchanged since the compile (`font.features.listErrors`).
//!
//! The code is coloured by the classes the feature parser's highlighter gives (`fontInfo.features.syntaxColouring`),
//! and the ranges of the diagnostics are underlined in the error colour while the field shows the stored code
//! (`fontInfo.features.inlineErrors`): typed text moves them.

use craft::theme::variant::Tone;
use craft::theme::{Rgba, Srgba};
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::primitives::Completion;
use craft::ui::runtime::env::Env;
use craft::ui::style::UiTokens;
use craft::ui::text::TextField;
use craft_ext::ui::text::{
    AsyncCompleter, CompletionAnswer, CompletionQuery, FieldSelection as _, Highlight, HighlightText as _, TextSelection, text_focused,
};
use egui::Color32;
use egui::Ui;
use fontcraft_engine::features::edit::{HighlightKind, highlight};
use serde_json::json;

use crate::app::models::font_features::{DiagnosticModel, EntryPane};
use crate::app::{Msg, ViewMsg};
use crate::shell::font_info_tabs::CompletionAsk;

craft::ui::words! {
    /// The words of the feature code editor.
    pub struct CodeEditorWords {
        code: "fontcraftFeatureCode.code" = "Feature Code";
        automatic_hint: "fontcraftFeatureCode.automaticHint" = "Generated automatically: turn off Generate Feature Automatically to edit the code";
        line: "fontcraftFeatureCode.line" = "Line";
        fix: "fontcraftFeatureCode.fix" = "Fix";
        fix_tip: "fontcraftFeatureCode.fixTip" = "Make the suggested correction";
    }
}

/// Lines the code field shows.
const CODE_LINES: usize = 18;

/// What the code editor reports.
pub struct CodeOut {
    /// The messages to post.
    pub sent: Vec<Msg>,
    /// The code field's last selection, in characters of the stored code.
    pub selection: Option<TextSelection>,
}

/// Draws the code field and the diagnostics of `entry`.
pub fn view(ui: &mut Ui, entry: &EntryPane) -> CodeOut {
    let words = Env::of(ui).words::<CodeEditorWords>();
    let key = &entry.key;
    let mut sent = Vec::new();
    let field = TextField::new(words.code.clone(), Bind::given(entry.code.clone()))
        .mono()
        .lines(CODE_LINES)
        .tab_inserts(true)
        .limit(None)
        .enabled(!entry.automatic)
        .id_salt(("fontcraftFeatureCode", key.kind.name(), key.index, key.name.as_str()));
    let completer = AsyncCompleter::kept(ui.ctx(), egui::Id::new(("fontcraftFeatureCode.complete", key.kind.name(), key.index, key.name.as_str())));
    if let Some(c) = &entry.completion {
        let items = c.items.iter().map(|i| Completion::new(i.label.clone(), i.label.clone())).collect();
        let range = c.items.first().map_or(c.position..c.position, |i| i.start..i.end);
        completer.answer(CompletionQuery { text: c.code.clone(), caret: c.position }, CompletionAnswer { range, items });
    }
    let field = if entry.automatic { field.tooltip(words.automatic_hint.clone()) } else { field.complete(completer.clone()).complete_on_enter() };
    let out = field.show(ui);
    if let Some(q) = completer.take_query() {
        let ask = wants_completion(&q.text, q.caret).then(|| CompletionAsk { key: key.clone(), code: q.text, position: q.caret });
        if ask.is_some() || entry.completion.is_some() {
            sent.push(Msg::View(ViewMsg::FeatureCompletion(ask)));
        }
    } else if entry.completion.is_some() && !text_focused(ui.ctx(), out.response.id) {
        sent.push(Msg::View(ViewMsg::FeatureCompletion(None)));
    }
    let env = Env::of(ui);
    out.highlight_text(ui, |text| runs(text, &entry.code, &entry.diagnostics, &env.tokens));
    let selection = out.last_selection(ui.ctx()).and_then(|s| s.within(entry.code.chars().count()));
    if let Some(text) = out.events.committed()
        && !entry.automatic
        && *text != entry.code
    {
        sent.push(Msg::run("font.features.setCode", key.with(&[("code", json!(text))])));
    }
    for d in &entry.diagnostics {
        Row::new().id_salt(("fontcraftFeatureCode.diagnostic", d.id)).show(ui, |ui, _| {
            let text = Text::new(format!("{} {}: {}", words.line.as_str(), d.line, d.message));
            if d.error { text.tone(Tone::Bad) } else { text.shade(Shade::Muted) }.show(ui);
            for (at, label) in d.fixes.iter().enumerate() {
                let button = Button::new(format!("{}: {label}", words.fix.as_str())).tooltip(words.fix_tip.clone()).id_salt(("fix", d.id, at));
                if button.show(ui).clicked() {
                    sent.push(Msg::run("font.features.applyFix", json!({ "error": d.id, "fix": at })));
                }
            }
        });
    }
    CodeOut { sent, selection }
}

/// Whether the character before byte `at` of `text` may start or continue a name worth completing: a name character,
/// `@`, `$` or `{`.
fn wants_completion(text: &str, at: usize) -> bool {
    text.get(..at).and_then(|s| s.chars().next_back()).is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-' | '@' | '$' | '{'))
}

/// Most diagnostics underlined at once.
const MAX_UNDERLINES: usize = 1_000;

/// The colour of one class of code. The theme has no syntax colours of its own: keywords and tags take the accent,
/// glyph names the text colour, comments and punctuation the muted text, the rest the data series.
fn ink(kind: HighlightKind, tokens: &UiTokens) -> Rgba {
    let series = |i: usize| tokens.color.data.series.get(i).copied().unwrap_or(tokens.color.text.normal);
    match kind {
        HighlightKind::Keyword | HighlightKind::Tag => tokens.color.accent.solid,
        HighlightKind::GlyphName => tokens.color.text.normal,
        HighlightKind::ClassName => series(5),
        HighlightKind::LookupName => series(1),
        HighlightKind::Number => series(7),
        HighlightKind::String => series(2),
        HighlightKind::Comment | HighlightKind::Punctuation => tokens.color.text.muted,
        HighlightKind::Token => series(3),
    }
}

/// The runs of the field's `text`: a colour per class, and the diagnostics' ranges underlined while `text` is the
/// stored `code` they belong to.
fn runs(text: &str, code: &str, diagnostics: &[DiagnosticModel], tokens: &UiTokens) -> Vec<Highlight> {
    let color = |c: Rgba| Color32::from(Srgba::from(c));
    let mut out: Vec<Highlight> = highlight(text).into_iter().map(|h| Highlight::ink(h.range, color(ink(h.kind, tokens)))).collect();
    if text == code {
        let bad = color(tokens.color.status.bad.fg);
        out.extend(diagnostics.iter().filter_map(|d| d.range).take(MAX_UNDERLINES).map(|(start, end)| Highlight::underline(start..end, bad)));
    }
    out
}
