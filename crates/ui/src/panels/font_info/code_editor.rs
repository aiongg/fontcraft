//! The feature code editor of the Features tab: a monospaced field of several lines over the shown entry's code, and
//! the last compile's diagnostics in that entry under it.
//!
//! | Control | Posts |
//! |---|---|
//! | the code | `font.features.setCode` when the field loses the keyboard and the text changed; read-only for an automatic entry |
//! | a diagnostic's Fix button | `font.features.applyFix {error, fix}` |
//!
//! The diagnostics show only while the font is unchanged since the compile (`font.features.listErrors`).

use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::runtime::env::Env;
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::json;

use crate::app::Msg;
use crate::app::models::font_features::EntryPane;

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
    if let Some(text) = field.show(ui).events.committed()
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
