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

use craft::keymap::{Chord, Key, Mods};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::popover::Popover;
use craft::ui::primitives::Completion;
use craft::ui::runtime::env::Env;
use craft::ui::runtime::focus::{self, FocusTarget};
use craft::ui::runtime::keys;
use craft::ui::style::UiTokens;
use craft::ui::text::TextField;
use craft_ext::ui::text::{
    AsyncCompleter, CompletionAnswer, CompletionQuery, FieldSelection as _, Highlight, HighlightText as _, TextSelection, field_layout, text_focused,
};
use egui::text::CCursor;
use egui::{Align2, Color32, FontId, Galley, Rect, Response, Sense, Shape, Stroke, TextStyle, Ui, pos2, vec2};
use fontcraft_engine::features::edit::{HighlightKind, highlight};
use serde_json::{Value, json};

use super::code_find::{self, FindAction};
use crate::app::models::font_features::{DiagnosticModel, EntryPane, FindModel, PreviewModel};
use crate::app::{Msg, ViewMsg};
use crate::shell::font_info_tabs::{CompletionAsk, EntryKey, EntryKind, FeatureDraft, FindState, PreviewAsk, select_entries};

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
    let action = code_find::bar(ui, entry, &mut sent);
    let edit_id = egui::Id::unique(("fontcraftFeatureCode.edit", key.kind.name(), key.index, key.name.as_str()));
    let pending: Option<PendingEdit> = ui.ctx().data_mut(|d| {
        let pending = d.get_temp::<PendingEdit>(edit_id);
        d.remove::<PendingEdit>(edit_id);
        pending
    });
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
    let field = match &pending {
        Some(p) if !entry.automatic => field.replace_selection(p.insert.clone()),
        _ => field,
    };
    let number_font = TextStyle::Monospace.resolve(ui.style());
    let gutter_width = gutter_width(ui, entry.draft.as_deref().unwrap_or(&entry.code), &number_font);
    let out = ui
        .horizontal_top(|ui| {
            ui.add_space(gutter_width);
            field.show(ui)
        })
        .inner;
    if let Some(p) = &pending {
        out.select(ui.ctx(), p.select);
    }
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
    let focused = text_focused(ui.ctx(), out.response.id);
    let caret = out.selection(ui.ctx()).filter(|s| focused && s.is_empty()).map(|s| s.caret);
    let found: &[(usize, usize)] = entry.find.as_ref().map(|f| f.here.as_slice()).unwrap_or_default();
    let accent = Color32::from(Srgba::from(env.tokens.color.accent.solid));
    out.highlight_text(ui, |text| {
        if text != entry.code {
            typed = Some(text.to_string());
        }
        let mut runs = kept_runs(ui.ctx(), runs_id, text, shown_diagnostics(entry, text), found, &env.tokens);
        if let Some(c) = caret {
            runs.extend(bracket_pair(text, c).into_iter().map(|at| Highlight::underline(at..at.saturating_add(1), accent)));
        }
        runs
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
    let mut action = action;
    if focused {
        action = action.or(editing_keys(ui, &out.response, entry, shown, edit_id, &mut sent));
    }
    if let Some(found) = &entry.find {
        find_action(ui, &out.response, entry, found, shown, action, &mut sent);
    }
    let selection = out.last_selection(ui.ctx()).and_then(|s| s.within(entry.code.chars().count()));
    if let Some(text) = out.events.committed()
        && !entry.automatic
        && *text != entry.code
    {
        sent.push(Msg::run("font.features.setCode", key.with(&[("code", json!(text))])));
    }
    // Without a layout of the field the open details stay as they are: closing them there would lose them in a pass
    // egui discards.
    if let Some(opener) = gutter(ui, &out.response, gutter_width, &number_font, listed, &words) {
        details(ui, &out.response, opener, listed, &words, &mut sent);
    }
    preview(ui, &out.response, entry, &words, &mut sent);
    CodeOut { sent, selection }
}

/// A change of the code field's text the editor makes in the next pass: the field selects `replace` in this pass,
/// `insert` takes its place in the next (`TextField::replace_selection`), and `select` is selected after it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PendingEdit {
    pub replace: TextSelection,
    pub insert: String,
    pub select: TextSelection,
}

/// The chord of `key` with Cmd (Ctrl off macOS), and Shift when `shift`.
fn cmd_chord(key: Key, shift: bool) -> Chord {
    Chord::new(Mods { cmd: true, shift, ..Mods::NONE }, key)
}

/// The editing keys (`fontInfo.features.editingKeys`) while the field `field` has the keyboard: they are claimed from
/// the focus service and the shortcuts each pass, so they reach the field from the next one. Cmd+F opens the find bar,
/// Cmd+G and Shift+Cmd+G return Next and Previous; the others edit the code as typed through a [`PendingEdit`].
fn editing_keys(ui: &Ui, field: &Response, entry: &EntryPane, shown: &str, edit_id: egui::Id, sent: &mut Vec<Msg>) -> Option<FindAction> {
    let ctx = ui.ctx();
    let owner = field.id;
    let selection = field.selection(ctx);
    let find = [cmd_chord(Key::F, false), cmd_chord(Key::G, false), cmd_chord(Key::G, true)];
    let tab = [Chord::plain(Key::Tab), Chord::new(Mods::SHIFT, Key::Tab)];
    let edits = [cmd_chord(Key::Slash, false), cmd_chord(Key::CloseBracket, false), cmd_chord(Key::OpenBracket, false), Chord::plain(Key::Enter)];
    for chord in find {
        keys::claim(ctx, owner, chord);
    }
    if !entry.automatic {
        for chord in edits {
            keys::claim(ctx, owner, chord);
        }
        if selection.is_some_and(|s| spans_lines(shown, s)) {
            for chord in tab {
                keys::claim(ctx, owner, chord);
            }
        }
    }
    let [open_find, find_next, find_previous] = find.map(|chord| keys::take_claimed(ctx, owner, chord));
    let [indent_tab, outdent_tab] = tab.map(|chord| keys::take_claimed(ctx, owner, chord));
    let [comment, indent_key, outdent_key, enter] = edits.map(|chord| keys::take_claimed(ctx, owner, chord));
    if open_find {
        let mut state = entry.find.as_ref().map(|f| f.state.clone()).unwrap_or_default();
        if let Some(picked) = selection.filter(|s| !s.is_empty() && !spans_lines(shown, *s)) {
            let range = picked.range();
            state.query = shown.chars().skip(range.start).take(range.len()).take(code_find::MAX_QUERY_CHARS).collect();
        }
        state.focus = true;
        sent.push(Msg::View(ViewMsg::FeatureFind(Some(state))));
    }
    if let Some(s) = selection.filter(|_| !entry.automatic) {
        let edit = if comment {
            toggle_comments(shown, s)
        } else if indent_key || indent_tab {
            indent(shown, s, true)
        } else if outdent_key || outdent_tab {
            indent(shown, s, false)
        } else if enter {
            Some(new_line(shown, s))
        } else {
            None
        };
        if let Some(edit) = edit {
            field.select(ctx, edit.replace);
            ctx.data_mut(|d| d.insert_temp(edit_id, edit));
            ctx.request_repaint();
        }
    }
    if find_next {
        Some(FindAction::Next)
    } else if find_previous {
        Some(FindAction::Previous)
    } else {
        None
    }
}

/// Most bytes of code one editing key rewrites.
const MAX_EDIT_BYTES: usize = 1 << 20;
/// The indentation one indent adds.
const INDENT: &str = "\t";
/// Most spaces one outdent removes where a line has no tab.
const OUTDENT_SPACES: usize = 4;

/// The byte of character `chars` of `text`, or its end.
fn byte_of(text: &str, chars: usize) -> usize {
    text.char_indices().nth(chars).map_or(text.len(), |(b, _)| b)
}

/// The characters of `text` before byte `byte`.
fn chars_before(text: &str, byte: usize) -> usize {
    text.get(..byte).map_or(0, |s| s.chars().count())
}

/// Whether the selection `s` of `text` holds a line break.
pub(super) fn spans_lines(text: &str, s: TextSelection) -> bool {
    let range = s.range();
    !s.is_empty() && text.get(byte_of(text, range.start)..byte_of(text, range.end)).is_some_and(|t| t.contains('\n'))
}

/// The whole lines the selection `s` touches, as bytes of `text`; a selection that ends at the start of a line leaves
/// that line out.
fn line_block(text: &str, s: TextSelection) -> (usize, usize) {
    let range = s.range();
    let (lo, mut hi) = (byte_of(text, range.start), byte_of(text, range.end));
    if hi > lo && text.get(..hi).is_some_and(|t| t.ends_with('\n')) {
        hi -= 1;
    }
    let start = text.get(..lo).and_then(|t| t.rfind('\n')).map_or(0, |i| i + 1);
    let end = text.get(hi..).and_then(|t| t.find('\n')).map_or(text.len(), |i| hi + i);
    (start, end.max(start))
}

/// The edit that puts `lines(block)` in place of the lines the selection `s` touches: the new lines are selected after
/// it, or for a caret the caret moves with its line.
fn rewrite_lines(text: &str, s: TextSelection, lines: impl Fn(&str) -> String) -> Option<PendingEdit> {
    let (start, end) = line_block(text, s);
    if end - start > MAX_EDIT_BYTES {
        return None;
    }
    let block = text.get(start..end)?;
    let insert = block.split('\n').map(&lines).collect::<Vec<_>>().join("\n");
    if insert == block {
        return None;
    }
    let first = chars_before(text, start);
    let old_len = block.chars().count();
    let new_len = insert.chars().count();
    let select = if s.is_empty() {
        let moved = s.caret.saturating_add(new_len).saturating_sub(old_len).max(first);
        TextSelection::caret_at(moved)
    } else {
        TextSelection::new(first, first.saturating_add(new_len))
    };
    Some(PendingEdit { replace: TextSelection::new(first, first.saturating_add(old_len)), insert, select })
}

/// A line's leading tabs and spaces.
fn indentation(line: &str) -> &str {
    let body = line.trim_start_matches([' ', '\t']);
    line.get(..line.len() - body.len()).unwrap_or_default()
}

/// Indents (`more`) or outdents the lines the selection `s` touches: one tab more, or one tab (else up to four spaces)
/// fewer. Empty lines stay empty.
pub(super) fn indent(text: &str, s: TextSelection, more: bool) -> Option<PendingEdit> {
    rewrite_lines(text, s, |line| {
        if line.is_empty() {
            String::new()
        } else if more {
            format!("{INDENT}{line}")
        } else if let Some(rest) = line.strip_prefix('\t') {
            rest.to_string()
        } else {
            let spaces = line.len() - line.trim_start_matches(' ').len();
            line.get(spaces.min(OUTDENT_SPACES)..).unwrap_or_default().to_string()
        }
    })
}

/// Comments the lines the selection `s` touches with `# ` after their indentation, or, when every line with text
/// already starts with `#`, takes that `#` (and one space after it) away.
pub(super) fn toggle_comments(text: &str, s: TextSelection) -> Option<PendingEdit> {
    let (start, end) = line_block(text, s);
    let block = text.get(start..end)?;
    let commented = block.split('\n').filter(|l| !l.trim().is_empty()).all(|l| l.trim_start().starts_with('#'));
    rewrite_lines(text, s, |line| {
        if line.trim().is_empty() {
            return line.to_string();
        }
        let lead = indentation(line);
        let body = line.get(lead.len()..).unwrap_or_default();
        if commented {
            let rest = body.strip_prefix('#').unwrap_or(body);
            format!("{lead}{}", rest.strip_prefix(' ').unwrap_or(rest))
        } else {
            format!("{lead}# {body}")
        }
    })
}

/// Return: a line break in place of the selection `s`, followed by the indentation of the line the selection starts
/// in.
pub(super) fn new_line(text: &str, s: TextSelection) -> PendingEdit {
    let range = s.range();
    let lo = byte_of(text, range.start);
    let start = text.get(..lo).and_then(|t| t.rfind('\n')).map_or(0, |i| i + 1);
    let lead = indentation(text.get(start..lo).unwrap_or_default());
    let insert = format!("\n{lead}");
    let caret = range.start.saturating_add(insert.chars().count());
    PendingEdit { replace: s, insert, select: TextSelection::caret_at(caret) }
}

/// Most characters bracket matching looks through.
const MAX_BRACKET_SCAN: usize = 100_000;

/// The bytes of the bracket beside the caret (character `caret` of `text`; the one before it first) and of its partner,
/// when both are there within [`MAX_BRACKET_SCAN`] characters.
pub(super) fn bracket_pair(text: &str, caret: usize) -> Vec<usize> {
    let at = byte_of(text, caret);
    let before = text.get(..at).and_then(|t| t.char_indices().next_back());
    let after = text.get(at..).and_then(|t| t.chars().next()).map(|c| (at, c));
    for (byte, c) in [before, after].into_iter().flatten() {
        let partner = match c {
            '(' | '[' | '{' => scan(text, byte, c, true),
            ')' | ']' | '}' => scan(text, byte, c, false),
            _ => continue,
        };
        if let Some(p) = partner {
            return vec![byte, p];
        }
    }
    Vec::new()
}

/// The partner of the bracket `c` at byte `at`, forward from an opening one or back from a closing one.
fn scan(text: &str, at: usize, c: char, forward: bool) -> Option<usize> {
    let (open, close) = match c {
        '(' | ')' => ('(', ')'),
        '[' | ']' => ('[', ']'),
        _ => ('{', '}'),
    };
    let mut depth = 0usize;
    let step = |ch: char, depth: &mut usize| -> bool {
        let (deeper, shallower) = if forward { (open, close) } else { (close, open) };
        if ch == deeper {
            *depth += 1;
        } else if ch == shallower {
            if *depth == 0 {
                return true;
            }
            *depth -= 1;
        }
        false
    };
    if forward {
        let from = at.saturating_add(c.len_utf8());
        text.get(from..)?.char_indices().take(MAX_BRACKET_SCAN).find(|(_, ch)| step(*ch, &mut depth)).map(|(i, _)| from + i)
    } else {
        text.get(..at)?.char_indices().rev().take(MAX_BRACKET_SCAN).find(|(_, ch)| step(*ch, &mut depth)).map(|(i, _)| i)
    }
}

/// Carries out the find bar's `action` on the field `field`, and selects the first or last match of an entry Next or
/// Previous moved to. Next and Previous select the following or preceding match and wrap at the end of the entry, or
/// in the all-entries scope go on to the next or previous entry with matches. Replace replaces the selected match
/// (else it acts as Next); Replace All replaces in the scope.
fn find_action(ui: &Ui, field: &Response, entry: &EntryPane, found: &FindModel, shown: &str, action: Option<FindAction>, sent: &mut Vec<Msg>) {
    let ctx = ui.ctx();
    let here = match_chars(shown, &found.here);
    let end = |first: bool| if first { here.first() } else { here.last() };
    if let Some(m) = found.state.reveal.and_then(end) {
        field.select(ctx, TextSelection::new(m.0, m.1));
        sent.push(Msg::View(ViewMsg::FeatureFind(Some(FindState { reveal: None, ..found.state.clone() }))));
    }
    let Some(action) = action else { return };
    let selection = field.last_selection(ctx).unwrap_or_default();
    let range = selection.range();
    let mut options = found.state.params();
    options.insert("replacement".into(), json!(found.state.replacement));
    match action {
        FindAction::Next | FindAction::Previous | FindAction::Replace => {
            let current = here.iter().position(|m| m.0 == range.start && m.1 == range.end);
            if action == FindAction::Replace
                && let Some(i) = current
                && let Some((start, _)) = found.here.get(i)
            {
                let mut p = entry.key.params();
                p.extend(options);
                p.insert("code".into(), json!(shown));
                p.insert("start".into(), json!(start));
                sent.push(Msg::run("font.features.replace", Value::Object(p)));
                return;
            }
            let forward = action != FindAction::Previous;
            let pick = if forward { here.iter().find(|m| m.0 >= range.end) } else { here.iter().rev().find(|m| m.1 <= range.start) };
            if let Some(m) = pick {
                field.select(ctx, TextSelection::new(m.0, m.1));
                return;
            }
            let others: Vec<&EntryKey> = found.entries.iter().filter(|k| **k != entry.key).collect();
            if found.state.all_entries && !others.is_empty() {
                let at = found.entries.iter().position(|k| *k == entry.key);
                let target = match (at, forward) {
                    (Some(i), true) => found.entries.iter().skip(i + 1).chain(found.entries.iter()).find(|k| **k != entry.key),
                    (Some(i), false) => found.entries.iter().take(i).rev().chain(found.entries.iter().rev()).find(|k| **k != entry.key),
                    (None, true) => found.entries.iter().find(|k| next_after(k, &entry.key)).or(others.first().copied()),
                    (None, false) => found.entries.iter().rev().find(|k| !next_after(k, &entry.key)).or(others.last().copied()),
                };
                if let Some(k) = target {
                    sent.push(Msg::Shell(select_entries(std::slice::from_ref(k))));
                    sent.push(Msg::View(ViewMsg::FeatureFind(Some(FindState { reveal: Some(forward), ..found.state.clone() }))));
                    return;
                }
            }
            if let Some(m) = end(forward) {
                field.select(ctx, TextSelection::new(m.0, m.1));
            }
        }
        FindAction::ReplaceAll => {
            let mut p = options;
            p.insert("all".into(), json!(true));
            if !found.state.all_entries {
                p.extend(entry.key.params());
                p.insert("code".into(), json!(shown));
            }
            sent.push(Msg::run("font.features.replace", Value::Object(p)));
        }
    }
}

/// Whether the entry `k` comes after `shown` in sidebar order.
fn next_after(k: &EntryKey, shown: &EntryKey) -> bool {
    let group = |kind: EntryKind| EntryKind::ALL.iter().position(|g| *g == kind).unwrap_or(0);
    (group(k.kind), k.index) > (group(shown.kind), shown.index)
}

/// The byte ranges `ranges` of `text`, in order, as character ranges, in one pass over the text.
fn match_chars(text: &str, ranges: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut out = Vec::with_capacity(ranges.len());
    let (mut byte, mut chars) = (0usize, 0usize);
    let to = |target: usize, byte: &mut usize, chars: &mut usize| {
        if target > *byte {
            *chars += text.get(*byte..target).map_or(0, |s| s.chars().count());
            *byte = target;
        }
        *chars
    };
    for (start, end) in ranges {
        if *start < byte || end < start {
            continue;
        }
        let s = to(*start, &mut byte, &mut chars);
        let e = to(*end, &mut byte, &mut chars);
        out.push((s, e));
    }
    out
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
        if let Some(mark) = out.last_mut().filter(|mark| mark.line == d.line) {
            mark.error |= d.error;
            mark.fixable |= !d.fixes.is_empty();
            continue;
        }
        if out.len() >= MAX_MARKS {
            break;
        }
        out.push(LineMark { line: d.line, error: d.error, fixable: !d.fixes.is_empty(), message: &d.message });
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
/// details are open, while it shows; `None` when the field's text has no layout in this pass yet, as in a pass egui
/// discards before painting.
fn gutter(ui: &mut Ui, field: &Response, width: f32, font: &FontId, diagnostics: &[DiagnosticModel], words: &CodeEditorWords) -> Option<Option<Response>> {
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
            ui.ctx().data_mut(|d| {
                if opened {
                    d.insert_temp(open_id, mark.line);
                } else {
                    d.remove::<u64>(open_id);
                }
            });
            if opened {
                opener = Some(response);
            }
        } else if open == Some(mark.line) {
            opener = Some(response);
        }
    }
    Some(opener)
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
fn kept_runs(
    ctx: &egui::Context,
    id: egui::Id,
    text: &str,
    diagnostics: &[DiagnosticModel],
    found: &[(usize, usize)],
    tokens: &UiTokens,
) -> Vec<Highlight> {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    for d in diagnostics.iter().take(MAX_UNDERLINES) {
        d.range.hash(&mut hasher);
    }
    found.iter().take(MAX_UNDERLINES).for_each(|m| m.hash(&mut hasher));
    palette(tokens).hash(&mut hasher);
    let key = hasher.finish();
    if let Some(kept) = ctx.data(|d| d.get_temp::<Arc<(u64, Vec<Highlight>)>>(id)).filter(|kept| kept.0 == key) {
        return kept.1.clone();
    }
    let fresh = runs(text, diagnostics, found, tokens);
    ctx.data_mut(|d| d.insert_temp(id, Arc::new((key, fresh.clone()))));
    fresh
}

/// The colours of feature code with no diagnostics or matches, for the code fields of the feature parameters.
pub(super) fn code_runs(text: &str, tokens: &UiTokens) -> Vec<Highlight> {
    runs(text, &[], &[], tokens)
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

/// The runs of the field's `text`: a colour per class, the ranges of `diagnostics`, which belong to `text`, underlined in
/// the error colour, and the find bar's matches `found` in the accent colour.
fn runs(text: &str, diagnostics: &[DiagnosticModel], found: &[(usize, usize)], tokens: &UiTokens) -> Vec<Highlight> {
    let color = |c: Rgba| Color32::from(Srgba::from(c));
    let mut out: Vec<Highlight> = highlight(text).into_iter().map(|h| Highlight::ink(h.range, color(ink(h.kind, tokens)))).collect();
    let bad = color(tokens.color.status.bad.fg);
    out.extend(diagnostics.iter().filter_map(|d| d.range).take(MAX_UNDERLINES).map(|(start, end)| Highlight::underline(start..end, bad)));
    let accent = color(tokens.color.accent.solid);
    out.extend(found.iter().take(MAX_UNDERLINES).map(|(start, end)| Highlight::underline(*start..*end, accent)));
    out
}
