//! The feature code editor of the Features tab: a monospaced field of several lines over the shown entry's code, and
//! the last compile's diagnostics in that entry under it.
//!
//! | Control | Posts |
//! |---|---|
//! | the code | `font.features.setCode` when the field loses the keyboard and the text changed; read-only for an automatic entry |
//! | a diagnostic's Fix button | `font.features.applyFix {error, fix}` |
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
use craft::ui::runtime::env::Env;
use craft::ui::style::UiTokens;
use craft::ui::text::TextField;
use craft_ext::ui::text::{Highlight, HighlightText as _};
use egui::Color32;
use egui::Ui;
use fontcraft_engine::features::edit::{HighlightKind, highlight};
use serde_json::json;

use crate::app::Msg;
use crate::app::models::font_features::{DiagnosticModel, EntryPane};

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

/// Draws the code field and the diagnostics of `entry`; the messages to post.
pub fn view(ui: &mut Ui, entry: &EntryPane) -> Vec<Msg> {
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
    let field = if entry.automatic { field.tooltip(words.automatic_hint.clone()) } else { field };
    let out = field.show(ui);
    let env = Env::of(ui);
    out.highlight_text(ui, |text| runs(text, &entry.code, &entry.diagnostics, &env.tokens));
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
    sent
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
