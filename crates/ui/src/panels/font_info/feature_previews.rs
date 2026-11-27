//! The preview of a name or a `$` token Alt-clicked in feature code (`fontInfo.featureNamePreview`,
//! `fontInfo.featureTokenPreview`).
//!
//! The code editor asks with `ViewMsg::FeaturePreview` and the projection answers (`PreviewModel`) with
//! `font.features.previewName` or `font.features.evaluateToken`. The popover under the name shows
//!
//! - for a glyph its drawing and Unicode values;
//! - for a class its members drawn in a grid, with the anchors of its `markClass` statements;
//! - for a lookup its first rules;
//! - for a number or property token its value for the current master, and in every master and exported instance;
//! - for a predicate token the glyphs it matches, drawn in a grid.
//!
//! A double click on a drawn glyph opens it in a tab (`view.editSelectedGlyphs`) and closes the popover.

use std::borrow::Cow;

use craft::behavior::keyed;
use craft::ui::contract::{Bind, CollectionEvent, Props as _, Scroll, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Stack;
use craft::ui::list::ThumbAspect;
use craft::ui::popover::Popover;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::style::TypeRole;
use craft::ui::tile_grid::{CaptionPlace, GridZoom, Tile, TileGrid};
use craft_ext::ui::text::field_layout;
use egui::text::CCursor;
use egui::{Rect, Response, Ui, vec2};
use fontcraft_engine::features::tokens::token_ranges;
use serde_json::json;

use crate::app::models::font_features::{EntryPane, PreviewModel};
use crate::app::{Msg, ViewMsg};
use crate::panels::kerning_tools::paint_run;
use crate::shell::font_info_tabs::{EntryKey, PreviewAsk};

craft::ui::words! {
    /// The words of the previews of names and tokens in feature code.
    pub struct PreviewWords {
        preview: "fontcraftFeaturePreview.preview" = "Name preview";
        glyph: "fontcraftFeaturePreview.glyph" = "Glyph";
        unicode: "fontcraftFeaturePreview.unicode" = "Unicode";
        no_unicode: "fontcraftFeaturePreview.noUnicode" = "No Unicode value";
        not_exported: "fontcraftFeaturePreview.notExported" = "Does not export";
        class: "fontcraftFeaturePreview.class" = "Class";
        members: "fontcraftFeaturePreview.members" = "glyphs";
        not_in_font: "fontcraftFeaturePreview.notInFont" = "Not in the font";
        anchor: "fontcraftFeaturePreview.anchor" = "Anchor";
        lookup: "fontcraftFeaturePreview.lookup" = "Lookup";
        more_rules: "fontcraftFeaturePreview.moreRules" = "More rules follow";
        /// After the count of glyphs of a class or token with more than are drawn.
        first_shown: "fontcraftFeaturePreview.firstShown" = "first drawn:";
        drawn: "fontcraftFeaturePreview.drawn" = "Glyphs";
        no_glyphs: "fontcraftFeaturePreview.noGlyphs" = "No glyphs";
    }
}

/// Most characters a name Alt-click reads on each side of the click.
const MAX_NAME_CHARS: usize = 200;
/// Most names a list in the text shows.
const MAX_PREVIEW_NAMES: usize = 300;
/// The popover's content width, the most height of its text, and the drawings' sizes, in points.
const WIDTH: f32 = 360.0;
const MAX_TEXT_HEIGHT: f32 = 200.0;
const TILE: f32 = 56.0;
const ONE_TILE: f32 = 120.0;
const GRID_HEIGHT: f32 = 220.0;
/// How much taller a line of tiles is than its pictures: the caption.
const CAPTION: f32 = 28.0;

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

/// The characters of the `$` token at character `at` of `text`, start and end.
pub(super) fn token_at(text: &str, at: usize) -> Option<(usize, usize)> {
    let byte = text.char_indices().nth(at).map_or(text.len(), |(b, _)| b);
    let range = token_ranges(text).into_iter().find(|r| r.start <= byte && byte <= r.end)?;
    let chars = |end: usize| text.get(..end).map_or(0, |t| t.chars().count());
    Some((chars(range.start), chars(range.end)))
}

/// The token or name an Alt-click on the code field `field` landed on this pass, in `text` (the text shown).
pub(super) fn alt_clicked(ui: &Ui, field: &Response, text: &str, key: &EntryKey) -> Option<PreviewAsk> {
    let (clicked, alt, at) = ui.input(|i| (i.pointer.primary_clicked(), i.modifiers.alt, i.pointer.interact_pos()));
    let at = at.filter(|at| clicked && alt && field.rect.intersect(ui.clip_rect()).contains(*at))?;
    let (pos, galley) = field_layout(ui, field)?;
    let index = galley.cursor_from_pos(at - pos).index.0;
    let token = token_at(text, index);
    let (start, end) = token.or_else(|| name_at(text, index))?;
    let name: String = text.chars().skip(start).take(end.saturating_sub(start)).collect();
    Some(PreviewAsk { key: key.clone(), name, start, end, token: token.is_some() })
}

/// The popover of the preview the editor asked for, hanging under the name; closing it drops the question.
pub(super) fn popover(ui: &mut Ui, field: &Response, entry: &EntryPane, sent: &mut Vec<Msg>) {
    let Some(p) = &entry.preview else { return };
    let Some((pos, galley)) = field_layout(ui, field) else { return };
    let words = Env::of(ui).words::<PreviewWords>();
    let under = galley.pos_from_cursor(CCursor::new(p.start)).union(galley.pos_from_cursor(CCursor::new(p.end))).translate(pos.to_vec2());
    let mut open = true;
    let mut picked = None;
    Popover::new(words.preview.clone(), Bind::bound(&mut open)).at_point(under.left_bottom()).show(ui, |ui, _| {
        picked = body(ui, p, &words).or(picked.take());
    });
    if let Some(glyph) = picked {
        sent.push(Msg::Shell(CommandRef::with_params("view.editSelectedGlyphs", json!({ "glyphs": [glyph] }))));
        open = false;
    }
    if !open {
        sent.push(Msg::View(ViewMsg::FeaturePreview(None)));
    }
}

/// What the popover holds; the glyph double-clicked in it.
fn body(ui: &mut Ui, p: &PreviewModel, words: &PreviewWords) -> Option<String> {
    let picked = match p.kind.as_str() {
        "glyph" => drawings(ui, p, words),
        _ => None,
    };
    let mut lines = preview_lines(p, words).into_iter();
    if let Some(title) = lines.next() {
        Text::new(title).role(TypeRole::Heading).show(ui);
    }
    let lines: Vec<String> = lines.collect();
    if !lines.is_empty() {
        // Long lists scroll; a line wraps in about half the body's height per 60 characters.
        let line = ui.text_style_height(&egui::TextStyle::Body).max(1.0);
        let rows: usize = lines.iter().map(|l| 1 + l.chars().count() / 60).sum();
        let height = (rows as f32 * 1.6 * line).min(MAX_TEXT_HEIGHT);
        boxed(ui, "fontcraftFeaturePreview.text", height, true, |ui| {
            for line in &lines {
                Text::new(line.clone()).show(ui);
            }
        });
    }
    if let Some(problem) = &p.problem {
        Text::new(problem.clone()).shade(Shade::Muted).show(ui);
    }
    match p.kind.as_str() {
        "class" | "glyphs" => drawings(ui, p, words),
        _ => picked,
    }
}

/// A box `height` points tall and as wide as the popover's content, which scrolls when `scrolls`.
fn boxed(ui: &mut Ui, salt: &'static str, height: f32, scrolls: bool, mut body: impl FnMut(&mut Ui)) {
    let rect = Rect::from_min_size(ui.cursor().min, vec2(WIDTH, height));
    let stack = Stack::new().id_salt(salt).fill();
    let stack = if scrolls { stack.scroll(Scroll::vertical()) } else { stack };
    stack.show_at_taking_room(ui, rect, |ui, _| body(ui));
}

/// The glyphs of the preview drawn in a wrapping grid, names under them; the glyph double-clicked.
fn drawings(ui: &mut Ui, p: &PreviewModel, words: &PreviewWords) -> Option<String> {
    if p.glyphs.is_empty() {
        return None;
    }
    let single = p.kind == "glyph";
    let side = if single { ONE_TILE } else { TILE };
    let card = Env::of(ui).tokens.size.thumb.card;
    let zoom = if card > 0.0 { (side / card).max(f32::EPSILON) } else { 1.0 };
    let positions: Vec<usize> = (0..p.glyphs.len()).collect();
    let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(p.glyphs.get(*at).cloned().unwrap_or_default()));
    let mut picked = None;
    let per_line = (WIDTH / (side + CAPTION)).floor().max(1.0);
    let lines = (p.glyphs.len() as f32 / per_line).ceil().max(1.0);
    let height = (lines * (side + CAPTION) + CAPTION).min(GRID_HEIGHT);
    boxed(ui, "fontcraftFeaturePreview.glyphs", height, false, |ui| {
        let out = TileGrid::<usize>::new(words.drawn.clone())
            .empty(words.no_glyphs.clone())
            .zoom(GridZoom::Fixed(f64::from(zoom)))
            .caption(CaptionPlace::Below)
            .type_ahead(true)
            .select(SelectMode::Single)
            .show_items(ui, &source, &mut |at, _| {
                let Some(name) = p.glyphs.get(*at) else { return Tile::new(String::new()) };
                let tile = Tile::new(name.clone());
                match p.art.get(*at) {
                    Some(art) => tile.thumb(ThumbAspect::SQUARE, move |cx, rect| paint_run(cx, rect, p.units_per_em, &[(art.as_ref(), 0.0)])),
                    None => tile,
                }
            });
        for event in &out.response.events {
            if let CollectionEvent::Activate(at) = event {
                picked = p.glyphs.get(*at).cloned();
            }
        }
    });
    picked
}

/// The lines of text a preview shows, the first its title.
pub(super) fn preview_lines(p: &PreviewModel, words: &PreviewWords) -> Vec<String> {
    let names = |list: &[String]| {
        let mut text = list.iter().take(MAX_PREVIEW_NAMES).cloned().collect::<Vec<_>>().join(" ");
        if list.len() > MAX_PREVIEW_NAMES {
            text.push_str(" …");
        }
        text
    };
    let count = || {
        let count = format!("{} {}", p.total, words.members.as_str());
        if p.total > p.glyphs.len() { format!("{count}, {} {}", words.first_shown.as_str(), p.glyphs.len()) } else { count }
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
            out.push(format!("{} @{}: {}", words.class.as_str(), p.name.trim_start_matches('@'), count()));
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
        "number" => {
            out.push(format!("{} = {}", p.name, p.value));
            for (name, value, problem) in &p.values {
                out.push(match value {
                    Some(value) => format!("{name}: {value}"),
                    None => format!("{name}: {problem}"),
                });
            }
        }
        "glyphs" => out.push(format!("{}: {}", p.name, count())),
        _ => {}
    }
    out
}
