//! The feature code editor of the Features tab: a monospaced field of several lines over the shown entry's code, and
//! the last compile's diagnostics in that entry under it.
//!
//! | Control | Posts |
//! |---|---|
//! | the code | `font.features.setCode` when the field loses the keyboard and the text changed; read-only for an automatic entry |
//! | a gutter icon | opens the details of the line's diagnostics (`fontInfo.featureErrorDetails`) |
//! | a Fix button in the details | `font.features.applyFix {error, fix}` |
//!
//! The field's last selection goes back to the Features tab, whose snippets menu puts a snippet in its place; the
//! caret then goes after the snippet and the field takes the keyboard (`EntryPane::caret`).
//!
//! Typing a name character, `@`, `$` or `{` asks for the names that fit at the caret (`ViewMsg::FeatureCompletion`,
//! answered by the projection with `font.features.complete`); the list under the field shows them once the answer
//! matches the code as typed, and Tab, Return or a click puts the picked name in (`fontInfo.features.autocomplete`).
//!
//! The code as typed goes to the app while it differs from the stored code (`ViewMsg::FeatureDraft`), which checks it
//! after a pause with `font.features.check` (`crate::shell::feature_work`) without storing it.
//!
//! The code is coloured by the classes the feature parser's highlighter gives (`fontInfo.features.syntaxColouring`);
//! the coloured runs are kept while the text, the underlined ranges and the colours stay the same.
//! The diagnostics of the text shown (`fontInfo.features.inlineErrors`) are those of the last compile while the field
//! shows the stored code and the compile still describes the font (`font.features.listErrors`), else those of the
//! check of exactly the text shown; typed text hides them until its check lands. Their ranges are underlined in the
//! error colour. A gutter left of the field numbers each line (a wrapped row continues its line and gets no number)
//! and marks a line with diagnostics with an error icon, else a warning icon, and a wrench when one of them has a fix;
//! the line's first message is drawn dimmed after its text. A click on the icon opens the details of the line's
//! diagnostics with their Fix buttons.
//!
//! Alt-click on a glyph, class or lookup name opens a popover with its preview (`fontInfo.featureNamePreview`): the
//! editor asks with `ViewMsg::FeaturePreview` and the projection answers with `font.features.previewName`. The popover
//! shows a glyph's Unicode values, a class's members (with the anchors of its `markClass` statements) or a lookup's
//! first rules.

use craft::theme::variant::Tone;
use craft::theme::{Rgba, Srgba};
use craft::ui::buttons::Button;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::popover::Popover;
use craft::ui::primitives::Completion;
use craft::ui::runtime::env::Env;
use craft::ui::runtime::focus::{self, FocusTarget};
use craft::ui::style::UiTokens;
use craft::ui::text::TextField;
use craft_ext::ui::text::{
    AsyncCompleter, CompletionAnswer, CompletionQuery, FieldSelection as _, Highlight, HighlightText as _, TextSelection, field_layout, text_focused,
};
use egui::text::CCursor;
use egui::{Align2, Color32, FontId, Galley, Rect, Response, Sense, Shape, Stroke, TextStyle, Ui, pos2, vec2};
use fontcraft_engine::features::edit::{HighlightKind, highlight};
use serde_json::json;

use crate::app::models::font_features::{DiagnosticModel, EntryPane, PreviewModel};
use crate::app::{Msg, ViewMsg};
use crate::shell::font_info_tabs::{CompletionAsk, EntryKey, FeatureDraft, PreviewAsk};

craft::ui::words! {
    /// The words of the feature code editor.
    pub struct CodeEditorWords {
        code: "fontcraftFeatureCode.code" = "Feature Code";
        automatic_hint: "fontcraftFeatureCode.automaticHint" = "Generated automatically: turn off Generate Feature Automatically to edit the code";
        line: "fontcraftFeatureCode.line" = "Line";
        fix: "fontcraftFeatureCode.fix" = "Fix";
        fix_tip: "fontcraftFeatureCode.fixTip" = "Make the suggested correction";
        problems: "fontcraftFeatureCode.problems" = "Problems on line";
        fixable: "fontcraftFeatureCode.fixable" = "a fix is available";
        details: "fontcraftFeatureCode.details" = "Problem details";
        preview: "fontcraftFeatureCode.preview" = "Name preview";
        glyph: "fontcraftFeatureCode.glyph" = "Glyph";
        unicode: "fontcraftFeatureCode.unicode" = "Unicode";
        no_unicode: "fontcraftFeatureCode.noUnicode" = "No Unicode value";
        not_exported: "fontcraftFeatureCode.notExported" = "Does not export";
        class: "fontcraftFeatureCode.class" = "Class";
        members: "fontcraftFeatureCode.members" = "glyphs";
        not_in_font: "fontcraftFeatureCode.notInFont" = "Not in the font";
        anchor: "fontcraftFeatureCode.anchor" = "Anchor";
        lookup: "fontcraftFeatureCode.lookup" = "Lookup";
        more_rules: "fontcraftFeatureCode.moreRules" = "More rules follow";
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
        .limit(None)
        .enabled(!entry.automatic)
        .id_salt(("fontcraftFeatureCode", key.kind.name(), key.index, key.name.as_str()));
    let completer =
        AsyncCompleter::kept(ui.ctx(), egui::Id::unique(("fontcraftFeatureCode.complete", key.kind.name(), key.index, key.name.as_str())));
    if let Some(c) = &entry.completion {
        let items = c.items.iter().map(|i| Completion::new(i.label.clone(), i.label.clone())).collect();
        let range = c.items.first().map_or(c.position..c.position, |i| i.start..i.end);
        completer.answer(CompletionQuery { text: c.code.clone(), caret: c.position }, CompletionAnswer { range, items });
    }
    let field = if entry.automatic { field.tooltip(words.automatic_hint.clone()) } else { field.complete(completer.clone()).complete_on_enter() };
    let number_font = TextStyle::Monospace.resolve(ui.style());
    let gutter_width = gutter_width(ui, entry.draft.as_deref().unwrap_or(&entry.code), &number_font);
    let out = ui
        .horizontal_top(|ui| {
            ui.add_space(gutter_width);
            field.show(ui)
        })
        .inner;
    if let Some(q) = completer.take_query() {
        let ask = wants_completion(&q.text, q.caret).then(|| CompletionAsk { key: key.clone(), code: q.text, position: q.caret });
        if ask.is_some() || entry.completion.is_some() {
            sent.push(Msg::View(ViewMsg::FeatureCompletion(ask)));
        }
    } else if entry.completion.is_some() && !text_focused(ui.ctx(), out.response.id) {
        sent.push(Msg::View(ViewMsg::FeatureCompletion(None)));
    }
    if let Some(caret) = entry.caret.filter(|_| !entry.automatic) {
        out.select(ui.ctx(), TextSelection::caret_at(caret.min(entry.code.chars().count())));
        let _ = focus::request(ui.ctx(), FocusTarget::Id(out.response.id));
        sent.push(Msg::View(ViewMsg::FeatureCaretPlaced));
    }
    let env = Env::of(ui);
    let mut typed: Option<String> = None;
    let runs_id = out.response.id.with("fontcraftFeatureCode.runs");
    out.highlight_text(ui, |text| {
        if text != entry.code {
            typed = Some(text.to_string());
        }
        kept_runs(ui.ctx(), runs_id, text, shown_diagnostics(entry, text), &env.tokens)
    });
    let typed = typed.filter(|_| !entry.automatic);
    if typed != entry.draft {
        sent.push(Msg::View(ViewMsg::FeatureDraft(typed.clone().map(|code| FeatureDraft { key: key.clone(), code }))));
    }
    let shown = typed.as_deref().unwrap_or(&entry.code);
    if let Some(ask) = alt_clicked(ui, &out.response, shown, key) {
        sent.push(Msg::View(ViewMsg::FeaturePreview(Some(ask))));
    }
    let listed = shown_diagnostics(entry, shown);
    let selection = out.last_selection(ui.ctx()).and_then(|s| s.within(entry.code.chars().count()));
    if let Some(text) = out.events.committed()
        && !entry.automatic
        && *text != entry.code
    {
        sent.push(Msg::run("font.features.setCode", key.with(&[("code", json!(text))])));
    }
    let opener = gutter(ui, &out.response, gutter_width, &number_font, listed, &words);
    details(ui, &out.response, opener, listed, &words, &mut sent);
    preview(ui, &out.response, entry, &words, &mut sent);
    CodeOut { sent, selection }
}

/// Most characters a name Alt-click reads on each side of the click.
const MAX_NAME_CHARS: usize = 200;
/// Most glyph names a preview lists in its text.
const MAX_PREVIEW_NAMES: usize = 300;

/// Whether `c` may be part of a glyph, class or lookup name; `@` and a backslash only lead one.
fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '-')
}

/// The characters of the name at character `at` of `text`, start and end: name characters on both sides of `at`, with a
/// leading `@` or backslash kept, at most [`MAX_NAME_CHARS`] on each side. `None` where no name is.
pub(super) fn name_at(text: &str, at: usize) -> Option<(usize, usize)> {
    let byte = text.char_indices().nth(at).map_or(text.len(), |(b, _)| b);
    let mut back = 0;
    for c in text.get(..byte).unwrap_or_default().chars().rev().take(MAX_NAME_CHARS) {
        if is_name_char(c) {
            back += 1;
        } else {
            if matches!(c, '@' | '\\') {
                back += 1;
            }
            break;
        }
    }
    let ahead = text.get(byte..).unwrap_or_default().chars().take(MAX_NAME_CHARS).take_while(|c| is_name_char(*c)).count();
    let (start, end) = (at.saturating_sub(back), at.saturating_add(ahead));
    let named = text.chars().skip(start).take(end.saturating_sub(start)).any(is_name_char);
    named.then_some((start, end))
}

/// The name an Alt-click on the code field `field` landed on this pass, in `text` (the text shown).
fn alt_clicked(ui: &Ui, field: &Response, text: &str, key: &EntryKey) -> Option<PreviewAsk> {
    let (clicked, alt, at) = ui.input(|i| (i.pointer.primary_clicked(), i.modifiers.alt, i.pointer.interact_pos()));
    let at = at.filter(|at| clicked && alt && field.rect.intersect(ui.clip_rect()).contains(*at))?;
    let (pos, galley) = field_layout(ui, field)?;
    let index = galley.cursor_from_pos(at - pos).index.0;
    let (start, end) = name_at(text, index)?;
    let name: String = text.chars().skip(start).take(end.saturating_sub(start)).collect();
    Some(PreviewAsk { key: key.clone(), name, start, end })
}

/// The popover of the Alt-clicked name's preview, hanging under the name; closing it drops the question.
fn preview(ui: &mut Ui, field: &Response, entry: &EntryPane, words: &CodeEditorWords, sent: &mut Vec<Msg>) {
    let Some(p) = &entry.preview else { return };
    let Some((pos, galley)) = field_layout(ui, field) else { return };
    let under = galley.pos_from_cursor(CCursor::new(p.start)).union(galley.pos_from_cursor(CCursor::new(p.end))).translate(pos.to_vec2());
    let mut open = true;
    Popover::new(words.preview.clone(), Bind::bound(&mut open)).at_point(under.left_bottom()).show(ui, |ui, _| {
        for line in preview_lines(p, words) {
            Text::new(line).show(ui);
        }
        if let Some(problem) = &p.problem {
            Text::new(problem.clone()).shade(Shade::Muted).show(ui);
        }
    });
    if !open {
        sent.push(Msg::View(ViewMsg::FeaturePreview(None)));
    }
}

/// The lines of text a preview shows.
pub(super) fn preview_lines(p: &PreviewModel, words: &CodeEditorWords) -> Vec<String> {
    let names = |list: &[String]| {
        let mut text = list.iter().take(MAX_PREVIEW_NAMES).cloned().collect::<Vec<_>>().join(" ");
        if list.len() > MAX_PREVIEW_NAMES {
            text.push_str(" …");
        }
        text
    };
    let mut out = Vec::new();
    match p.kind.as_str() {
        "glyph" => {
            out.push(format!("{} {}", words.glyph.as_str(), p.name.trim_start_matches('\\')));
            if p.unicodes.is_empty() {
                out.push(words.no_unicode.as_str().to_string());
            } else {
                out.push(format!("{}: {}", words.unicode.as_str(), p.unicodes.join(" ")));
            }
            if p.hidden {
                out.push(words.not_exported.as_str().to_string());
            }
        }
        "class" => {
            out.push(format!("{} @{}: {} {}", words.class.as_str(), p.name.trim_start_matches('@'), p.glyphs.len(), words.members.as_str()));
            if p.anchors.is_empty() {
                out.push(names(&p.glyphs));
            }
            for (anchor, glyphs) in &p.anchors {
                out.push(format!("{} {anchor}: {glyphs}", words.anchor.as_str()));
            }
            if !p.unknown.is_empty() {
                out.push(format!("{}: {}", words.not_in_font.as_str(), names(&p.unknown)));
            }
        }
        "lookup" => {
            out.push(format!("{} {}", words.lookup.as_str(), p.name));
            out.extend(p.rules.iter().cloned());
            if p.more {
                out.push(words.more_rules.as_str().to_string());
            }
        }
        _ => {}
    }
    out
}

/// Most lines the gutter marks.
const MAX_MARKS: usize = 1_000;
/// Most diagnostics the details of one line list.
const MAX_DETAILS: usize = 50;
/// Fewest digits the gutter has room for.
const MIN_DIGITS: usize = 2;

/// What the gutter shows at one line with diagnostics: an error icon if one of them is an error (else a warning icon),
/// a wrench if one of them has a fix, and the first one's message after the line's text.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct LineMark<'a> {
    pub line: u64,
    pub error: bool,
    pub fixable: bool,
    pub message: &'a str,
}

/// The marks of `diagnostics`, one per line, in line order, at most [`MAX_MARKS`]; a diagnostic without a line
/// (line 0) gets none.
pub(super) fn marks(diagnostics: &[DiagnosticModel]) -> Vec<LineMark<'_>> {
    let mut sorted: Vec<&DiagnosticModel> = diagnostics.iter().filter(|d| d.line > 0).collect();
    sorted.sort_by_key(|d| d.line);
    let mut out: Vec<LineMark<'_>> = Vec::new();
    for d in sorted {
        match out.last_mut() {
            Some(mark) if mark.line == d.line => {
                mark.error |= d.error;
                mark.fixable |= !d.fixes.is_empty();
            }
            _ if out.len() >= MAX_MARKS => break,
            _ => out.push(LineMark { line: d.line, error: d.error, fixable: !d.fixes.is_empty(), message: &d.message }),
        }
    }
    out
}

/// One line of the laid-out code: its number from 1, the top and bottom of its first row and the right end of its
/// last row, in the layout's points.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LineSpan {
    line: u64,
    top: f32,
    bottom: f32,
    end: f32,
}

/// The lines of `galley`: the first row and each row after a newline start one; a wrapped row continues its line.
fn line_spans(galley: &Galley) -> Vec<LineSpan> {
    let mut out: Vec<LineSpan> = Vec::new();
    let mut starts = true;
    for row in &galley.rows {
        let rect = row.rect();
        if starts {
            let line = out.last().map_or(1, |last| last.line + 1);
            out.push(LineSpan { line, top: rect.min.y, bottom: rect.max.y, end: rect.max.x });
        } else if let Some(last) = out.last_mut() {
            last.end = rect.max.x;
        }
        starts = row.ends_with_newline;
    }
    out
}

/// The gutter's width for `text`: an icon, a wrench and the digits of its last line number, with gaps.
fn gutter_width(ui: &Ui, text: &str, font: &FontId) -> f32 {
    let lines = text.bytes().filter(|b| *b == b'\n').count().saturating_add(1);
    let digits = lines.to_string().len().max(MIN_DIGITS);
    let (digit, row) = ui.ctx().fonts_mut(|fonts| (fonts.glyph_width(font, '0'), fonts.row_height(font)));
    let gap = Env::of(ui).tokens.space.gap.inline;
    2.0 * row + digit * digits as f32 + 3.0 * gap
}

/// The id under which the code field `field` keeps the line whose details are open.
fn open_line_id(field: &Response) -> egui::Id {
    field.id.with("fontcraftFeatureCode.details")
}

/// Paints the gutter left of the code field `field` (`width` wide): the line numbers, and for each line with
/// diagnostics its icon, its wrench and its first message, dimmed, after the line's text. A click on an icon opens or
/// closes the line's details. Only rows inside the field's visible rect are painted. Returns the icon of the line whose
/// details are open, while it shows.
fn gutter(ui: &mut Ui, field: &Response, width: f32, font: &FontId, diagnostics: &[DiagnosticModel], words: &CodeEditorWords) -> Option<Response> {
    let (pos, galley) = field_layout(ui, field)?;
    let env = Env::of(ui);
    let color = |c: Rgba| Color32::from(Srgba::from(c));
    let (muted, bad, warn) = (color(env.tokens.color.text.muted), color(env.tokens.color.status.bad.fg), color(env.tokens.color.status.warn.fg));
    let gap = env.tokens.space.gap.inline;
    let stroke = env.tokens.stroke.thin;
    let area = field.rect;
    let gutter = Rect::from_min_max(pos2(area.min.x - width, area.min.y), pos2(area.min.x, area.max.y));
    let clip = gutter.intersect(ui.clip_rect());
    let painter = ui.painter().with_clip_rect(clip);
    let message_painter = ui.painter().with_clip_rect(area.intersect(ui.clip_rect()));
    let open_id = open_line_id(field);
    let open: Option<u64> = ui.ctx().data(|d| d.get_temp(open_id));
    let marks = marks(diagnostics);
    let mut pending = marks.iter().peekable();
    let mut opener = None;
    for span in line_spans(&galley) {
        while pending.peek().is_some_and(|m| m.line < span.line) {
            pending.next();
        }
        let mark = pending.peek().copied().filter(|m| m.line == span.line);
        let (top, bottom) = (pos.y + span.top, pos.y + span.bottom);
        if bottom < clip.min.y {
            continue;
        }
        if top > clip.max.y {
            break;
        }
        painter.text(pos2(gutter.max.x - gap, top), Align2::RIGHT_TOP, span.line, font.clone(), muted);
        let Some(mark) = mark else { continue };
        let side = (bottom - top).max(0.0);
        let icon = Rect::from_min_size(pos2(gutter.min.x + gap, top), vec2(side, side));
        paint_mark(&painter, icon, if mark.error { bad } else { warn }, mark.error);
        if mark.fixable {
            paint_wrench(&painter, icon.translate(vec2(side, 0.0)), muted, stroke);
        }
        message_painter.text(pos2(pos.x + span.end + 2.0 * gap, top), Align2::LEFT_TOP, mark.message, font.clone(), muted);
        let mut label = format!("{} {}", words.problems.as_str(), mark.line);
        if mark.fixable {
            label = format!("{label} ({})", words.fixable.as_str());
        }
        let response = ui.interact(icon.intersect(clip), field.id.with(("fontcraftFeatureCode.mark", mark.line)), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label.clone()));
        if response.clicked() {
            let opened = open != Some(mark.line);
            ui.ctx().data_mut(|d| if opened { d.insert_temp(open_id, mark.line) } else { d.remove::<u64>(open_id) });
            if opened {
                opener = Some(response);
            }
        } else if open == Some(mark.line) {
            opener = Some(response);
        }
    }
    opener
}

/// An error icon (a disc) or a warning icon (a triangle) in `rect`.
fn paint_mark(painter: &egui::Painter, rect: Rect, color: Color32, error: bool) {
    let r = 0.4 * rect.width().min(rect.height());
    let c = rect.center();
    if error {
        painter.circle_filled(c, r, color);
    } else {
        painter.add(Shape::convex_polygon(vec![c + vec2(0.0, -r), c + vec2(r, r), c + vec2(-r, r)], color, Stroke::NONE));
    }
}

/// A wrench in `rect`: a handle and an open ring at its head.
fn paint_wrench(painter: &egui::Painter, rect: Rect, color: Color32, stroke: f32) {
    let r = 0.4 * rect.width().min(rect.height());
    let c = rect.center();
    let head = c + vec2(0.35 * r, -0.35 * r);
    painter.line_segment([c + vec2(-0.8 * r, 0.8 * r), head], Stroke::new(2.0 * stroke, color));
    painter.circle_stroke(head, 0.4 * r, Stroke::new(1.5 * stroke, color));
}

/// The details of the open line (`fontInfo.featureErrorDetails`), hanging from its icon `opener`: each diagnostic's
/// message and its Fix buttons. A Fix, a close of the popover, or the icon no longer showing closes them.
fn details(ui: &mut Ui, field: &Response, opener: Option<Response>, diagnostics: &[DiagnosticModel], words: &CodeEditorWords, sent: &mut Vec<Msg>) {
    let open_id = open_line_id(field);
    let Some(line) = ui.ctx().data(|d| d.get_temp::<u64>(open_id)) else { return };
    let here: Vec<&DiagnosticModel> = diagnostics.iter().filter(|d| d.line == line).take(MAX_DETAILS).collect();
    let Some(opener) = opener.filter(|_| !here.is_empty()) else {
        ui.ctx().data_mut(|d| d.remove::<u64>(open_id));
        return;
    };
    let mut open = true;
    let out = Popover::new(words.details.clone(), Bind::bound(&mut open)).opened_by(&opener).show(ui, |ui, _| {
        let mut fixed = false;
        for d in &here {
            let text = Text::new(format!("{} {}: {}", words.line.as_str(), d.line, d.message));
            if d.error { text.tone(Tone::Bad) } else { text.shade(Shade::Muted) }.show(ui);
            for (at, label) in d.fixes.iter().enumerate() {
                let button = Button::new(format!("{}: {label}", words.fix.as_str())).tooltip(words.fix_tip.clone()).id_salt(("fix", d.id, at));
                if button.show(ui).clicked() {
                    sent.push(Msg::run("font.features.applyFix", json!({ "error": d.id, "fix": at })));
                    fixed = true;
                }
            }
        }
        fixed
    });
    if !open || out.inner == Some(true) {
        ui.ctx().data_mut(|d| d.remove::<u64>(open_id));
    }
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

/// The diagnostics that belong to `text`: the last compile's while it is the stored code and the compile still
/// describes the font, else those of the check of `text`, else none.
fn shown_diagnostics<'a>(entry: &'a EntryPane, text: &str) -> &'a [DiagnosticModel] {
    match &entry.check {
        _ if entry.compiled && text == entry.code => &entry.diagnostics,
        Some((checked, found)) if checked == text => found,
        _ => &[],
    }
}

/// The runs of `text` ([`runs`]), kept in the context's memory under `id` while the text, the underlined ranges and the
/// colours stay the same, so a long entry is classed once rather than every frame.
fn kept_runs(ctx: &egui::Context, id: egui::Id, text: &str, diagnostics: &[DiagnosticModel], tokens: &UiTokens) -> Vec<Highlight> {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    for d in diagnostics.iter().take(MAX_UNDERLINES) {
        d.range.hash(&mut hasher);
    }
    palette(tokens).hash(&mut hasher);
    let key = hasher.finish();
    if let Some(kept) = ctx.data(|d| d.get_temp::<Arc<(u64, Vec<Highlight>)>>(id)).filter(|kept| kept.0 == key) {
        return kept.1.clone();
    }
    let fresh = runs(text, diagnostics, tokens);
    ctx.data_mut(|d| d.insert_temp(id, Arc::new((key, fresh.clone()))));
    fresh
}

/// Every colour [`runs`] uses, so a theme change makes new runs.
fn palette(tokens: &UiTokens) -> [Color32; 11] {
    let color = |c: Rgba| Color32::from(Srgba::from(c));
    [
        color(ink(HighlightKind::Keyword, tokens)),
        color(ink(HighlightKind::Tag, tokens)),
        color(ink(HighlightKind::GlyphName, tokens)),
        color(ink(HighlightKind::ClassName, tokens)),
        color(ink(HighlightKind::LookupName, tokens)),
        color(ink(HighlightKind::Number, tokens)),
        color(ink(HighlightKind::String, tokens)),
        color(ink(HighlightKind::Comment, tokens)),
        color(ink(HighlightKind::Punctuation, tokens)),
        color(ink(HighlightKind::Token, tokens)),
        color(tokens.color.status.bad.fg),
    ]
}

/// The runs of the field's `text`: a colour per class, and the ranges of `diagnostics`, which belong to `text`,
/// underlined.
fn runs(text: &str, diagnostics: &[DiagnosticModel], tokens: &UiTokens) -> Vec<Highlight> {
    let color = |c: Rgba| Color32::from(Srgba::from(c));
    let mut out: Vec<Highlight> = highlight(text).into_iter().map(|h| Highlight::ink(h.range, color(ink(h.kind, tokens)))).collect();
    let bad = color(tokens.color.status.bad.fg);
    out.extend(diagnostics.iter().filter_map(|d| d.range).take(MAX_UNDERLINES).map(|(start, end)| Highlight::underline(start..end, bad)));
    out
}
