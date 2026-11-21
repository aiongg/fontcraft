//! The Text Preview panel: running text of any length drawn in the font, for proofing passages that would slow Edit
//! View down.
//!
//! | Control | Posts |
//! |---|---|
//! | style pop-up | `textPreview.setStyle`: a master, an instance at a master's location, or the current master |
//! | size field | `textPreview.setFontSize` |
//! | Pin | `textPreview.togglePinned` |
//! | Find | `textPreview.find {open}`: the find bar, with replace and its options |
//! | text field | `textPreview.setText` on every change (texts up to [`FIELD_LIMIT`] characters) |
//! | context menu of the text | `textPreview.setLayoutOrientation`, `textPreview.setWritingDirection {scope: "text"}`, and `{scope: "selection", range}` over the text field's last selection |
//!
//! The text area reports its line length and the lines in view (`ViewMsg::TextPreviewArea`) when they leave what the
//! projection placed; the projection (`app::models::text_preview`) lays the text out, shapes it and places the
//! glyphs. The area fills the glyphs in view into one texture, drawn again only when what shows changes; a glyph with
//! a colour drawing from the compiled colour font (`Page::colors`) is laid onto that texture in place of its outline,
//! its origin on a whole pixel. The area also reports the window's pixels per point and the text colour, which the
//! colour drawings are made with.

use craft::theme::{Rgba, Srgba};
use craft::ui::buttons::{Button, ToggleButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Scroll, Show as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::menu::Menu;
use craft::ui::pickers::Select;
use craft::ui::primitives::TextureCache;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::TextField;
use craft_ext::ui::text::{FieldSelection as _, TextSelection};
use egui::{Align2, Color32, ColorImage, FontId, Pos2, Rect, Sense, TextureOptions, Ui, pos2, vec2};
use fontcraft_geom::{Affine, BezPath};
use fontcraft_render::color::composite;
use fontcraft_render::{Style, render_path};
use serde_json::{Value, json};

use super::widgets;
use crate::app::models::text_preview::{Page, PageDirection, PageGlyph, TextPreviewModel};
use crate::app::{Msg, ViewMsg};
use crate::shell::text_preview::{Area, BaseDirection, FIELD_LIMIT, MAX_QUERY_CHARS, Matching, Orientation, SIZE_RANGE, grouped};

/// Rows of the text field.
const FIELD_LINES: usize = 3;
/// The line length moves in whole steps of this many points, so resizing lays the text out again only now and then.
const SPAN_STEP: f32 = 8.0;

craft::ui::words! {
    /// The words of the Text Preview.
    pub struct TextPreviewWords {
        style: "fontcraftTextPreview.style" = "Style";
        current_master: "fontcraftTextPreview.currentMaster" = "Current master";
        size: "fontcraftTextPreview.size" = "Size";
        pin: "fontcraftTextPreview.pin" = "Pin";
        pin_tip: "fontcraftTextPreview.pinTip" = "Keep this window above the others";
        find: "fontcraftTextPreview.find" = "Find";
        replace: "fontcraftTextPreview.replace" = "Replace";
        replace_one: "fontcraftTextPreview.replaceOne" = "Replace";
        replace_all: "fontcraftTextPreview.replaceAll" = "All";
        done: "fontcraftTextPreview.done" = "Done";
        previous: "fontcraftTextPreview.previous" = "Previous Match";
        next: "fontcraftTextPreview.next" = "Next Match";
        ignore_case: "fontcraftTextPreview.ignoreCase" = "Ignore Case";
        matching: "fontcraftTextPreview.matching" = "Match";
        contains: "fontcraftTextPreview.contains" = "Contains";
        starts_with: "fontcraftTextPreview.startsWith" = "Starts With";
        full_word: "fontcraftTextPreview.fullWord" = "Full Word";
        one_match: "fontcraftTextPreview.oneMatch" = "1 match";
        matches: template "fontcraftTextPreview.matches" = "{count} matches";
        text: "fontcraftTextPreview.text" = "Preview text";
        too_long: template "fontcraftTextPreview.tooLong" = "{length} characters: the field edits texts of up to {limit}; Find and Replace change any text.";
        no_font: "fontcraftTextPreview.noFont" = "No font is open";
        area: "fontcraftTextPreview.area" = "Preview";
        menu: "fontcraftTextPreview.menu" = "Text Preview";
        orientation: "fontcraftTextPreview.orientation" = "Layout Orientation";
        horizontal: "fontcraftTextPreview.horizontal" = "Horizontal";
        vertical: "fontcraftTextPreview.vertical" = "Vertical";
        writing_direction: "fontcraftTextPreview.writingDirection" = "Writing Direction";
        default_direction: "fontcraftTextPreview.defaultDirection" = "Default";
        ltr: "fontcraftTextPreview.ltr" = "Left to Right";
        rtl: "fontcraftTextPreview.rtl" = "Right to Left";
        selection_direction: "fontcraftTextPreview.selectionDirection" = "Selection Direction";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<TextPreviewModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<TextPreviewWords>();
    let model: &TextPreviewModel = props.model;
    let sent = &mut *props.sent;
    Stack::new().fill().show(ui, |ui, _| {
        top_row(ui, model, &words, sent);
        if model.find.open {
            find_bar(ui, model, &words, sent);
        }
        let selection = text_field(ui, model, &words, sent);
        if !model.font {
            Text::new(words.no_font.clone()).shade(Shade::Muted).show(ui);
            return;
        }
        if let Some(message) = &model.message {
            Text::new(message.clone()).shade(Shade::Muted).show(ui);
        }
        area(ui, model, &words, selection, sent);
    });
}

fn top_row(ui: &mut Ui, model: &TextPreviewModel, words: &TextPreviewWords, sent: &mut Vec<Msg>) {
    Row::new().show(ui, |ui, _| {
        let mut entries = vec![Entry::new(String::new(), words.current_master.clone())];
        entries.extend(model.styles.iter().map(|s| Entry::new(s.clone(), Label::from(s.clone()))));
        let shown = model.style.clone().unwrap_or_default();
        if let Some(picked) = Select::new(words.style.clone(), Bind::given(shown.clone()), &entries).show(ui).events.committed()
            && *picked != shown
        {
            let params = if picked.is_empty() { json!({}) } else { json!({ "instance": picked }) };
            sent.push(shell("textPreview.setStyle", params));
        }
        if let Some(size) = widgets::number(ui, words.size.clone(), Some(model.size))
            && SIZE_RANGE.contains(&size)
        {
            sent.push(shell("textPreview.setFontSize", json!({ "size": size })));
        }
        if let Some(on) = ToggleButton::new(words.pin.clone(), Bind::given(model.pinned)).tooltip(words.pin_tip.clone()).show(ui).events.committed() {
            sent.push(shell("textPreview.togglePinned", json!({ "on": on })));
        }
        if let Some(open) = ToggleButton::new(words.find.clone(), Bind::given(model.find.open)).show(ui).events.committed() {
            sent.push(shell("textPreview.find", json!({ "open": open })));
        }
    });
}

fn find_bar(ui: &mut Ui, model: &TextPreviewModel, words: &TextPreviewWords, sent: &mut Vec<Msg>) {
    let count = model.matches;
    Row::new().show(ui, |ui, _| {
        let query = TextField::search(words.find.clone(), Bind::given(model.find.query.clone()))
            .placeholder(words.find.clone())
            .limit(MAX_QUERY_CHARS)
            .show(ui);
        if let Some(q) = query.events.value()
            && *q != model.find.query
        {
            sent.push(shell("textPreview.find", json!({ "query": q })));
        }
        if let Some(ignore) = Checkbox::new(Bind::given(model.find.ignore_case)).label(words.ignore_case.clone()).show(ui).events.committed() {
            sent.push(shell("textPreview.find", json!({ "options": { "ignoreCase": ignore } })));
        }
        let modes = [(Matching::Contains, &words.contains), (Matching::StartsWith, &words.starts_with), (Matching::FullWord, &words.full_word)];
        let entries: Vec<Entry<String>> = modes.iter().map(|(m, label)| Entry::new(m.name().to_string(), (*label).clone())).collect();
        let shown = model.find.matching.name().to_string();
        if let Some(m) = Select::new(words.matching.clone(), Bind::given(shown.clone()), &entries).show(ui).events.committed()
            && *m != shown
        {
            sent.push(shell("textPreview.find", json!({ "options": { "match": m } })));
        }
        let current = model.find.current.unwrap_or(0);
        if Button::new(words.previous.clone()).enabled(count > 0).show(ui).clicked() && count > 0 {
            let index = current.checked_sub(1).unwrap_or(count - 1);
            sent.push(shell("textPreview.find", json!({ "options": { "index": index } })));
        }
        if Button::new(words.next.clone()).enabled(count > 0).show(ui).clicked() && count > 0 {
            let index = (current + 1) % count;
            sent.push(shell("textPreview.find", json!({ "options": { "index": index } })));
        }
        let label: Label = match (count, model.find.query.is_empty()) {
            (_, true) => Label::from(String::new()),
            (1, false) => words.one_match.clone(),
            (n, false) => words.matches.fill(&[("count", grouped(n).as_str())]).into(),
        };
        Text::new(label).shade(Shade::Muted).show(ui);
    });
    Row::new().show(ui, |ui, _| {
        let replace = TextField::new(words.replace.clone(), Bind::kept()).placeholder(words.replace.clone()).limit(MAX_QUERY_CHARS).show(ui);
        let with = match &replace.shown {
            Some(craft::behavior::Mixed::Same(text)) => text.to_string(),
            _ => String::new(),
        };
        if Button::new(words.replace_one.clone()).enabled(model.find.current.is_some()).show(ui).clicked()
            && let Some(index) = model.find.current
        {
            sent.push(shell("textPreview.find", json!({ "replace": with, "options": { "index": index } })));
        }
        if Button::new(words.replace_all.clone()).enabled(count > 0).show(ui).clicked() && count > 0 {
            sent.push(shell("textPreview.find", json!({ "replace": with })));
        }
        if Button::new(words.done.clone()).show(ui).clicked() {
            sent.push(shell("textPreview.find", json!({ "open": false })));
        }
    });
}

/// Shows the text field; returns its last selection when it selects characters of the text.
fn text_field(ui: &mut Ui, model: &TextPreviewModel, words: &TextPreviewWords, sent: &mut Vec<Msg>) -> Option<TextSelection> {
    if model.length > FIELD_LIMIT {
        let note = words.too_long.fill(&[("length", grouped(model.length).as_str()), ("limit", grouped(FIELD_LIMIT).as_str())]);
        Text::new(note).shade(Shade::Muted).show(ui);
        return None;
    }
    let out = TextField::new(words.text.clone(), Bind::given(model.text.to_string())).lines(FIELD_LINES).limit(FIELD_LIMIT).show(ui);
    if let Some(text) = out.events.value()
        && **text != *model.text
    {
        sent.push(shell("textPreview.setText", json!({ "text": text })));
    }
    out.last_selection(ui.ctx()).and_then(|s| s.within(model.length)).filter(|s| !s.is_empty())
}

/// Where the text sits on the screen: a layout point (x, y) is at (`ax` + x·`scale`, `ay` − y·`scale`).
#[derive(Clone, Copy)]
struct Placement {
    ax: f32,
    ay: f32,
    scale: f32,
}

impl Placement {
    fn screen(self, x: f64, y: f64) -> Pos2 {
        pos2(self.ax + x as f32 * self.scale, self.ay - y as f32 * self.scale)
    }

    fn affine(self) -> Affine {
        let s = f64::from(self.scale);
        Affine::new([s, 0.0, 0.0, -s, f64::from(self.ax), f64::from(self.ay)])
    }
}

fn color(c: Rgba) -> Color32 {
    Color32::from(Srgba::from(c))
}

fn area(ui: &mut Ui, model: &TextPreviewModel, words: &TextPreviewWords, selection: Option<TextSelection>, sent: &mut Vec<Msg>) {
    let env = Env::of(ui);
    let pad = env.tokens.space.pad.regular;
    let colors = &env.tokens.color;
    let (ink, canvas, mark, current, fallback) = (
        colors.text.normal,
        color(colors.surface.canvas),
        color(colors.state.text_selection),
        color(colors.state.match_current),
        color(colors.text.muted),
    );
    let avail = ui.available_size();
    let size = vec2(avail.x.max(pad * 8.0), avail.y.max(pad * 8.0));
    let vertical = model.orientation == Orientation::Vertical;
    let span = if vertical { size.y } else { size.x } - 3.0 * pad;
    let span = ((span / SPAN_STEP).floor() * SPAN_STEP).max(SPAN_STEP) as u32;
    let ppp = ui.ctx().pixels_per_point().to_bits();
    let ink_rgba = color(ink).to_srgba_unmultiplied();
    let Some(page) = model.page.as_deref() else {
        report(model, Area { span, first: 0, last: 0, ppp, ink: ink_rgba }, sent);
        return;
    };
    let scale = page.scale as f32;
    let lh = (page.line_height as f32 * scale).max(f32::EPSILON);
    let extent = pad * 2.0 + lh * page.lines.max(1) as f32;
    let content = if vertical { vec2(extent.max(size.x), size.y) } else { vec2(size.x, extent.max(size.y)) };
    let mut seen = None;
    Stack::new().scroll(Scroll::both()).fill().show(ui, |ui, _| {
        let (rect, response) = ui.allocate_exact_size(content, Sense::click());
        let visible = rect.intersect(ui.clip_rect());
        let origin = rect.min;
        let asc = page.ascender as f32;
        let place = match page.direction {
            PageDirection::Ltr => Placement { ax: origin.x + pad, ay: origin.y + pad + asc * scale, scale },
            PageDirection::Rtl => Placement { ax: origin.x + content.x - pad, ay: origin.y + pad + asc * scale, scale },
            PageDirection::Vertical => Placement { ax: origin.x + content.x - pad - 0.5 * lh, ay: origin.y + pad, scale },
        };
        let viewport = visible.translate(-origin.to_vec2());
        let (lo, hi) = if vertical {
            (content.x - pad - viewport.max.x, content.x - pad - viewport.min.x)
        } else {
            (viewport.min.y - pad, viewport.max.y - pad)
        };
        // The lines in view, with one more on each side for ascenders and descenders.
        let first = ((lo / lh).floor() - 1.0).max(0.0) as usize;
        let last = (((hi / lh).ceil() + 1.0).max(0.0) as usize).min(page.lines);
        seen = Some((first, last));
        if !visible.is_positive() {
            return;
        }
        let painter = ui.painter_at(visible);
        painter.rect_filled(visible, 0.0, canvas);
        let lines = first..last;
        for b in page.highlights.iter().filter(|b| lines.contains(&b.line)) {
            let [x0, y0, x1, y1] = b.rect;
            painter.rect_filled(Rect::from_two_pos(place.screen(x0, y0), place.screen(x1, y1)), 0.0, if b.current { current } else { mark });
        }
        fill(ui, page, place, visible, &lines, ink);
        for c in page.fallback.iter().filter(|c| lines.contains(&c.line)) {
            painter.text(place.screen(c.x, c.y), Align2::CENTER_CENTER, c.c, FontId::proportional(model.size as f32), fallback);
        }
        context_menu(ui, model, words, selection, &response, sent);
    });
    if let Some((first, last)) = seen {
        let wanted = Area { span, first, last, ppp, ink: ink_rgba };
        let covered = page.placed.start <= first && last <= page.placed.end;
        if span != model.area.span || ppp != model.area.ppp || ink_rgba != model.area.ink || !covered {
            report(model, wanted, sent);
        }
    }
}

/// Posts the area when it differs from what the projection read, so an area the projection cannot cover is not
/// posted on every frame.
fn report(model: &TextPreviewModel, area: Area, sent: &mut Vec<Msg>) {
    let moved = area.span != model.area.span || area.ppp != model.area.ppp || area.ink != model.area.ink;
    if area != model.area && (moved || area.last > 0) {
        sent.push(Msg::View(ViewMsg::TextPreviewArea(area)));
    }
}

/// Fills the outlines of the glyphs on `lines` into one texture of the area in view, with the colour glyphs laid over
/// it in place of their outlines.
fn fill(ui: &Ui, page: &Page, place: Placement, clip: Rect, lines: &std::ops::Range<usize>, ink: Rgba) {
    let ppp = ui.ctx().pixels_per_point();
    let (w, h) = ((clip.width() * ppp).round(), (clip.height() * ppp).round());
    if !(w >= 1.0 && h >= 1.0) {
        return;
    }
    // Drawings made at another pixel size wait for the projection to make them again.
    let sized = (page.colors.pixel_scale - f64::from(place.scale) * f64::from(ppp)).abs() <= 1e-9 * page.colors.pixel_scale.abs().max(1.0);
    let in_color = |g: &PageGlyph| sized && matches!(page.colors.rasters.get(&g.name), Some(Some(_)));
    let bits = [place.ax, place.ay, place.scale, clip.min.x, clip.min.y, w, h, ppp].map(f32::to_bits);
    let key = ("fontcraftTextPreview", page.key, page.outlines.id, page.colors.id, lines.start, lines.end, bits, ink.0);
    let image = || {
        let mut outline = BezPath::new();
        for g in page.glyphs.iter().filter(|g| lines.contains(&g.line) && !in_color(g)) {
            if let Some(path) = page.outlines.paths.get(&g.name) {
                let mut glyph = path.clone();
                glyph.apply_affine(Affine::translate((g.x, g.y)));
                outline.extend(glyph.elements().iter().copied());
            }
        }
        let to_pixels = Affine::scale(f64::from(ppp)) * Affine::translate((-f64::from(clip.min.x), -f64::from(clip.min.y))) * place.affine();
        let style = Style { ink: ink.0, background: [0, 0, 0, 0], anti_alias: true };
        match render_path(&outline, to_pixels, w as u32, h as u32, &style) {
            Ok(mut image) => {
                for g in page.glyphs.iter().filter(|g| lines.contains(&g.line) && in_color(g)) {
                    if let Some(Some(raster)) = page.colors.rasters.get(&g.name) {
                        let at = place.screen(g.x, g.y);
                        let x = (f64::from(at.x - clip.min.x) * f64::from(ppp)).round() as i64;
                        let y = (f64::from(at.y - clip.min.y) * f64::from(ppp)).round() as i64;
                        composite(&mut image, raster, x, y);
                    }
                }
                ColorImage::from_rgba_premultiplied([w as usize, h as usize], &image.pixels)
            }
            Err(_) => ColorImage::filled([1, 1], Color32::TRANSPARENT),
        }
    };
    let texture = TextureCache::texture_with(ui.ctx(), key, TextureOptions::LINEAR, image);
    // White is the identity tint: the texture's own colours show.
    let painter = ui.painter_at(clip);
    painter.image(texture, clip, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
}

/// The text's context menu: its orientation, the direction of the whole text, and the direction of the run the text
/// field selects (`selection`, in characters), offered while the field selects characters.
fn context_menu(
    ui: &mut Ui,
    model: &TextPreviewModel,
    words: &TextPreviewWords,
    selection: Option<TextSelection>,
    response: &egui::Response,
    sent: &mut Vec<Msg>,
) {
    let orientations = [(Orientation::Horizontal, &words.horizontal), (Orientation::Vertical, &words.vertical)];
    let directions = [(BaseDirection::Default, &words.default_direction), (BaseDirection::Ltr, &words.ltr), (BaseDirection::Rtl, &words.rtl)];
    let mut entries = vec![Entry::heading(words.orientation.clone())];
    entries.extend(orientations.iter().map(|(o, label)| Entry::new(format!("o:{}", o.name()), (*label).clone()).checked(model.orientation == *o)));
    entries.push(Entry::separator());
    entries.push(Entry::heading(words.writing_direction.clone()));
    entries.extend(directions.iter().map(|(d, label)| Entry::new(format!("d:{}", d.name()), (*label).clone()).checked(model.direction == *d)));
    entries.push(Entry::separator());
    entries.push(Entry::heading(words.selection_direction.clone()));
    entries.extend(directions.iter().map(|(d, label)| Entry::new(format!("s:{}", d.name()), (*label).clone()).enabled(selection.is_some())));
    let menu = Menu::context(words.menu.clone(), response).pick(ui, &entries);
    let Some(picked) = menu.picked else { return };
    if let Some(orientation) = picked.strip_prefix("o:") {
        sent.push(shell("textPreview.setLayoutOrientation", json!({ "orientation": orientation })));
    } else if let Some(direction) = picked.strip_prefix("d:") {
        sent.push(shell("textPreview.setWritingDirection", json!({ "scope": "text", "direction": direction })));
    } else if let (Some(direction), Some(s)) = (picked.strip_prefix("s:"), selection) {
        let range = s.range();
        sent.push(shell(
            "textPreview.setWritingDirection",
            json!({ "scope": "selection", "direction": direction, "range": [range.start, range.end] }),
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::contract::Tracked;
    use craft::ui::testing::Fixture;
    use serde_json::json;

    use super::*;
    use crate::shell::text_preview::FindState;

    /// The text field reports its selection to the context menu, and keeps it once a menu or another control takes the
    /// keyboard.
    #[test]
    fn the_text_fields_selection_outlives_its_focus() {
        let model = TextPreviewModel { font: true, text: "AVAV".into(), length: 4, ..TextPreviewModel::default() };
        let seen = Rc::new(RefCell::new(None));
        let record = Rc::clone(&seen);
        let mut f = Fixture::new(move |ui| {
            let words = Env::of(ui).words::<TextPreviewWords>();
            let mut sent = Vec::new();
            *record.borrow_mut() = text_field(ui, &model, &words, &mut sent);
            let _ = TextField::new("Other", Bind::given(String::new())).show(ui);
        });
        f.room(egui::Vec2::new(800.0, 600.0)).settle();
        assert_eq!(*seen.borrow(), None, "nothing is selected before the field is used");
        let field = f.by_label("Preview text").expect("the text field");
        f.click(&field).key(egui::Key::A, egui::Modifiers::COMMAND).settle();
        assert_eq!(seen.borrow().map(TextSelection::range), Some(0..4));
        let other = f.by_label("Other").expect("the other field");
        f.click(&other).settle();
        assert_eq!(seen.borrow().map(TextSelection::range), Some(0..4), "the selection is kept for the menu");
    }

    /// The Find toggle opens the find bar, the size field posts the size, and the area reports its line length.
    #[test]
    fn controls_post_text_preview_commands_and_the_area_reports_itself() {
        let model = Tracked::new(TextPreviewModel {
            font: true,
            text: "AV".into(),
            length: 2,
            size: 36.0,
            styles: vec!["Regular".into()],
            find: FindState::default(),
            page: Some(std::sync::Arc::new(Page {
                scale: 0.036,
                units_per_em: 1000.0,
                line_height: 1200.0,
                ascender: 800.0,
                lines: 1,
                placed: 0..1,
                ..Page::default()
            })),
            ..TextPreviewModel::default()
        });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut f = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        f.room(egui::Vec2::new(800.0, 600.0)).settle();
        assert!(
            posted.borrow().iter().any(|m| matches!(m, Msg::View(ViewMsg::TextPreviewArea(a)) if a.span > 0 && a.last > 0)),
            "the area reports its span and lines: {:?}",
            posted.borrow()
        );
        posted.borrow_mut().clear();
        let find = f.by_label("Find").expect("the Find toggle");
        f.click(&find).settle();
        assert!(posted.borrow().contains(&shell("textPreview.find", json!({ "open": true }))), "{:?}", posted.borrow());
        posted.borrow_mut().clear();
        let size = f.by_label("Size").expect("the size field");
        f.click(&size).key(egui::Key::A, egui::Modifiers::COMMAND).type_text("72").key(egui::Key::Enter, egui::Modifiers::NONE).settle();
        assert!(posted.borrow().contains(&shell("textPreview.setFontSize", json!({ "size": 72.0 }))), "{:?}", posted.borrow());
    }
}
