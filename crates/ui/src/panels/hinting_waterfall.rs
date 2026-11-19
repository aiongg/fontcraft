//! The Hinting Preview: a hinted waterfall of the shown glyph from 8 to 80 PPM at three scales.
//!
//! Each row is one pixel size: its size at the left (with the reason as a tooltip when the glyph was drawn unhinted)
//! and its device pixels, `scale` screen pixels per device pixel, so 1x puts one device pixel on one screen pixel. The
//! scale control posts `hint.tt.setPreviewScale`. The view model (`app::models::hinting`) renders the rows.

use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, Entry, Props as _, Scroll, Show as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::primitives::TextureCache;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use egui::{Color32, ColorImage, Pos2, Rect, Sense, TextureOptions, Ui, Vec2};
use serde_json::json;

use crate::app::Msg;
use crate::app::models::hinting::{SCALE_COMMAND, SCALES, Waterfall, WaterfallRow};

craft::ui::words! {
    /// The words of the Hinting Preview.
    pub struct WaterfallWords {
        /// The scale control.
        scale: "fontcraftHintingWaterfall.scale" = "Scale";
        /// The 1x scale.
        one: "fontcraftHintingWaterfall.one" = "1×";
        /// The 2x scale.
        two: "fontcraftHintingWaterfall.two" = "2×";
        /// The 4x scale.
        four: "fontcraftHintingWaterfall.four" = "4×";
        /// Shown without a glyph.
        no_glyph: "fontcraftHintingWaterfall.noGlyph" = "No glyph to preview.";
        /// The rows cannot be drawn.
        failed: template "fontcraftHintingWaterfall.failed" = "The Hinting Preview cannot be drawn: {error}";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<Waterfall>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<WaterfallWords>();
    let model = props.model;
    let scale = model.scale();
    let entries: Vec<Entry<u8>> = SCALES.iter().zip([&words.one, &words.two, &words.four]).map(|(s, label)| Entry::new(*s, label.clone())).collect();
    let sent = &mut *props.sent;
    Stack::new().fill().show(ui, |ui, _| {
        let picked = Segmented::new(words.scale.clone(), Bind::given(scale), &entries).show(ui);
        if let Some(s) = picked.events.committed().filter(|s| **s != scale) {
            sent.push(Msg::Shell(CommandRef::with_params(SCALE_COMMAND, json!({ "scale": s }))));
        }
        if model.glyph.is_none() {
            Text::new(words.no_glyph.clone()).shade(Shade::Muted).show(ui);
            return;
        }
        if let Some(error) = &model.message {
            Text::new(words.failed.fill(&[("error", error.as_str())])).show(ui);
        }
        Stack::new().scroll(Scroll::both()).fill().show(ui, |ui, _| {
            for row in &model.rows {
                Row::new().show(ui, |ui, _| {
                    let size = Text::new(row.ppem.to_string()).shade(if row.warning.is_some() { Shade::Normal } else { Shade::Muted });
                    match &row.warning {
                        Some(why) => size.tooltip(why.clone()).show(ui),
                        None => size.show(ui),
                    };
                    pixels(ui, model.key, row, scale);
                });
            }
        });
    });
}

/// Draws `row`'s pixels at `scale` device pixels per screen pixel, sampled without smoothing.
fn pixels(ui: &mut Ui, key: u64, row: &WaterfallRow, scale: u8) {
    let ppp = ui.ctx().pixels_per_point().max(0.01);
    let unit = f32::from(scale) / ppp;
    let size = Vec2::new(row.width as f32 * unit, row.height as f32 * unit);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let image = || ColorImage::from_rgba_premultiplied([row.width as usize, row.height as usize], &row.pixels);
    let texture = TextureCache::texture_with(ui.ctx(), ("fontcraftHintingWaterfall", key, row.ppem), TextureOptions::NEAREST, image);
    // White is the identity tint: the texture's own colours show.
    ui.painter().image(texture, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use craft::ui::contract::Tracked;
    use craft::ui::testing::Fixture;
    use serde_json::json;

    use super::*;

    /// Every row shows its size, and a click on 4× posts `hint.tt.setPreviewScale {scale: 4}`.
    #[test]
    fn rows_show_their_sizes_and_the_scale_posts_set_preview_scale() {
        let rows = (8..=80).map(|ppem| WaterfallRow { ppem, width: 2, height: 2, pixels: Arc::new(vec![255; 16]), warning: None }).collect();
        let model = Tracked::new(Waterfall { glyph: Some("A".into()), scale: 1, rows, message: None, key: 1 });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut f = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        f.room(Vec2::new(400.0, 4000.0)).settle();
        for ppem in [8, 80] {
            assert!(f.by_label(&ppem.to_string()).is_some(), "the row at {ppem} PPM");
        }
        let four = f.by_label("4×").expect("the 4× segment");
        f.click(&four).settle();
        assert_eq!(*posted.borrow(), vec![Msg::Shell(CommandRef::with_params(SCALE_COMMAND, json!({"scale": 4})))]);
    }
}
