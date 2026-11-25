//! The context menus' views: the canvas menu (`editView.contextMenu`) and Font View's glyph menu
//! (`fontView.glyphContextMenu`). Their items come from `app::context_menu`; a view shows them and reports what was
//! picked, and the shell turns the pick into messages (`shell::context_menu`).
//!
//! Both menus hang from the point they were opened at. The canvas menu's sections are split by rules; its glyph
//! section starts with the colour swatches (Glyph Color, or Layer Color when Alt was held at the click) and the menu
//! ends with Copy Glyph Names. Items that remove something (Delete Guide, the hint removals) show the danger tone
//! (`craft_ext::ui::menu::danger_icon`). The glyph menu holds the swatches (Layer Color when Alt was held at the click), Exports, Tags (ending with More Tags… past
//! `MAX_MENU_TAGS`), Status (when the font defines statuses: each with its colour, then No Status) and Copy Glyph Names.
//! A tag or a status carries a check when every glyph has it and a dash when some do.

use craft::theme::Srgba;
use craft::ui::contract::overlay::Anchor;
use craft::ui::contract::{Bind, Entry, Props as _, ShowContainer as _};
use craft::ui::menu::{Menu, entry_rows};
use craft::ui::runtime::env::{Env, Label};
use craft_ext::ui::menu::{EntryMark, MarkedEntry, entry_rows as marked_entry_rows};
use egui::{Response, Ui};

use crate::app::context_menu::glyph_menu::{COPY_FORMATS, GlyphMenu, HINTING, Mark};
use crate::app::context_menu::{CANVAS, CanvasMenu, ContextItem, DESTRUCTIVE};
use crate::app::models::font_view::COLOR_LABEL_NAMES;

craft::ui::words! {
    /// The words of the context menus.
    pub struct ContextMenuWords {
        /// The canvas menu's name.
        canvas: "fontcraftContextMenu.canvas" = "Canvas Menu";
        /// The glyph menu's name.
        glyph: "fontcraftContextMenu.glyph" = "Glyph Menu";
        /// The swatches' submenu: the glyph's colour, or the layer's with Alt.
        glyph_color: "fontcraftContextMenu.glyphColor" = "Glyph Color";
        layer_color: "fontcraftContextMenu.layerColor" = "Layer Color";
        /// The cross that clears the colour.
        no_color: "fontcraftContextMenu.noColor" = "No Color";
        /// The glyph menu's export check item.
        exports: "fontcraftContextMenu.exports" = "Exports";
        /// The Tags submenu and its last item.
        tags: "fontcraftContextMenu.tags" = "Tags";
        more_tags: "fontcraftContextMenu.moreTags" = "More Tags…";
        /// The Status submenu and its item that clears the stored status.
        status: "fontcraftContextMenu.status" = "Status";
        no_status: "fontcraftContextMenu.noStatus" = "No Status";
        /// The Copy Glyph Names submenu and its formats.
        copy_names: "fontcraftContextMenu.copyNames" = "Copy Glyph Names";
        space_separated: "fontcraftContextMenu.spaceSeparated" = "Space Separated";
        one_per_line: "fontcraftContextMenu.onePerLine" = "One per Line";
        comma_separated: "fontcraftContextMenu.commaSeparated" = "Comma Separated";
        slashed: "fontcraftContextMenu.slashed" = "Slashed";
        python_list: "fontcraftContextMenu.pythonList" = "Python List";
        unicodes: "fontcraftContextMenu.unicodes" = "Unicodes";
        /// The Hinting submenu and its items.
        hinting: "fontcraftContextMenu.hinting" = "Hinting";
        autohint_tt: "fontcraftContextMenu.autohintTrueType" = "Autohint TrueType";
        remove_tt: "fontcraftContextMenu.removeTrueTypeHints" = "Remove TrueType Hints";
        autohint_ps: "fontcraftContextMenu.autohintPostScript" = "Autohint PostScript";
        remove_ps: "fontcraftContextMenu.removePostScriptHints" = "Remove PostScript Hints";
    }
}

/// The colours of the twelve colour labels, in index order. The handbook names them; the shades are ours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelTokens {
    pub colors: [[u8; 3]; 12],
}

impl LabelTokens {
    pub const fn standard() -> LabelTokens {
        LabelTokens {
            colors: [
                [0xe0, 0x4b, 0x4b],
                [0xf0, 0x92, 0x3c],
                [0xa5, 0x6f, 0x3c],
                [0xf2, 0xd3, 0x3c],
                [0x9a, 0xd4, 0x6a],
                [0x3f, 0x9a, 0x4f],
                [0x4c, 0xc4, 0xd8],
                [0x4a, 0x7f, 0xe0],
                [0x9b, 0x62, 0xd4],
                [0xe8, 0x7b, 0xb8],
                [0xc4, 0xc4, 0xc4],
                [0x7a, 0x7a, 0x7a],
            ],
        }
    }
}

/// What was picked in a context menu.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// An item of the canvas menu, by its command id.
    Item(&'static str),
    /// A value of a submenu item of the canvas menu: its command id and the value.
    Choice(&'static str, String),
    /// A swatch (`Some(index)`) or No Color (`None`).
    Color(Option<u8>),
    /// The glyph menu's Exports.
    Export,
    /// A tag of the Tags submenu.
    Tag(String),
    MoreTags,
    /// A status of the Status submenu by its code, or No Status (`None`).
    Status(Option<i64>),
    /// A command of the Hinting submenu.
    Hinting(&'static str),
    /// A format of Copy Glyph Names, by its `glyph.copyGlyphNames` name.
    CopyNames(&'static str),
}

/// What a menu view reports: the pick, and whether the menu stays open.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub picked: Option<Pick>,
    pub open: bool,
}

/// The canvas menu, hanging from `opener` at the point it opened at.
pub fn canvas_menu(ui: &mut Ui, opener: &Response, menu: &CanvasMenu, alt: bool) -> Answer {
    let words = Env::of(ui).words::<ContextMenuWords>();
    let mut open = true;
    let at = egui::pos2(menu.screen[0], menu.screen[1]);
    let shown = Menu::context(words.canvas.clone(), opener).anchor(Anchor::Point(at)).open(Bind::bound(&mut open)).show(ui, |ui, _| {
        let mut picked = None;
        for (index, section) in menu.sections.iter().enumerate() {
            if index > 0 {
                let _ = entry_rows(ui, &[Entry::<()>::separator()]);
            }
            let mut entries: Vec<MarkedEntry<&'static str>> = Vec::new();
            if section.context == CANVAS {
                let caption = if alt { words.layer_color.clone() } else { words.glyph_color.clone() };
                picked = picked.or(swatches(ui, caption, &words));
            }
            for (at, item) in section.items.iter().enumerate() {
                let replaced = section.items.get(at + 1).is_some_and(|next| next.alternate);
                if (item.alternate && !alt) || (replaced && alt) {
                    continue;
                }
                let mut entry = Entry::new(item.command, item.label);
                if let Some(on) = item.checked {
                    entry = entry.checked(on);
                }
                entry = match &item.disabled_reason {
                    Some(why) => entry.disabled_reason(Label::from(why.clone())),
                    None => entry.enabled(item.enabled),
                };
                if item.enabled && !item.choices.is_empty() {
                    // The rows so far go first, so the submenu keeps its place among them.
                    if let Some(i) = marked_entry_rows(ui, &entries) {
                        picked = picked.or(entries.get(i).and_then(MarkedEntry::value).map(|id| Pick::Item(id)));
                    }
                    entries.clear();
                    picked = picked.or(choices(ui, item));
                    continue;
                }
                entries.push(MarkedEntry::new(entry).danger(DESTRUCTIVE.contains(&item.command)));
            }
            if let Some(i) = marked_entry_rows(ui, &entries) {
                picked = picked.or(entries.get(i).and_then(MarkedEntry::value).map(|id| Pick::Item(id)));
            }
        }
        picked.or(copy_names(ui, &words))
    });
    let picked = shown.inner.flatten();
    Answer { open: open && picked.is_none() && shown.output.closed.is_none(), picked }
}

/// A submenu item of the canvas menu: its values, the one its targets share checked.
fn choices(ui: &mut Ui, item: &ContextItem) -> Option<Pick> {
    let mut picked = None;
    let _ = Menu::submenu_row(ui, Label::from(item.label), |ui, _| {
        let entries: Vec<MarkedEntry<usize>> =
            item.choices.iter().enumerate().map(|(i, c)| MarkedEntry::new(Entry::new(i, Label::from(c.label.clone())).checked(c.checked))).collect();
        if let Some(i) = marked_entry_rows(ui, &entries) {
            picked = item.choices.get(i).map(|c| Pick::Choice(item.command, c.value.clone()));
        }
    });
    picked
}

/// The glyph menu, hanging from `opener` at the point it opened at.
pub fn glyph_menu(ui: &mut Ui, opener: &Response, menu: &GlyphMenu) -> Answer {
    let words = Env::of(ui).words::<ContextMenuWords>();
    let mut open = true;
    let at = egui::pos2(menu.screen[0], menu.screen[1]);
    let shown = Menu::context(words.glyph.clone(), opener)
        .id_salt("fontcraftContextMenu.glyphMenu")
        .anchor(Anchor::Point(at))
        .open(Bind::bound(&mut open))
        .show(ui, |ui, _| {
            let caption = if menu.alt { words.layer_color.clone() } else { words.glyph_color.clone() };
            let mut picked = swatches(ui, caption, &words);
            let exports = [MarkedEntry::new(Entry::new((), words.exports.clone())).mark(entry_mark(menu.exports))];
            if marked_entry_rows(ui, &exports).is_some() {
                picked = picked.or(Some(Pick::Export));
            }
            if !menu.tags.is_empty() {
                let _ = Menu::submenu_row(ui, words.tags.clone(), |ui, _| {
                    let mut entries: Vec<MarkedEntry<Option<usize>>> = menu
                        .tags
                        .iter()
                        .enumerate()
                        .map(|(i, (tag, mark))| MarkedEntry::new(Entry::new(Some(i), Label::from(tag.clone()))).mark(entry_mark(*mark)))
                        .collect();
                    if menu.more_tags {
                        entries.push(Entry::separator().into());
                        entries.push(Entry::new(None, words.more_tags.clone()).into());
                    }
                    if let Some(i) = marked_entry_rows(ui, &entries) {
                        picked = picked.clone().or(match entries.get(i).and_then(MarkedEntry::value) {
                            Some(Some(tag)) => menu.tags.get(*tag).map(|(t, _)| Pick::Tag(t.clone())),
                            Some(None) => Some(Pick::MoreTags),
                            None => None,
                        });
                    }
                });
            }
            if !menu.statuses.is_empty() {
                picked = picked.or(statuses(ui, menu, &words));
            }
            picked = picked.or(hinting(ui, &words));
            picked.or(copy_names(ui, &words))
        });
    let picked = shown.inner.flatten();
    Answer { open: open && picked.is_none() && shown.output.closed.is_none(), picked }
}

/// The Status submenu: each status with its colour and mark, then No Status.
fn statuses(ui: &mut Ui, menu: &GlyphMenu, words: &ContextMenuWords) -> Option<Pick> {
    let mut picked = None;
    let _ = Menu::submenu_row(ui, words.status.clone(), |ui, _| {
        let unit = |c: u8| f32::from(c) / 255.0;
        let mut entries: Vec<MarkedEntry<Option<i64>>> = menu
            .statuses
            .iter()
            .map(|s| {
                let [r, g, b, a] = s.color;
                let entry = Entry::new(Some(s.code), Label::from(s.name.clone())).swatch(Srgba::new(unit(r), unit(g), unit(b), unit(a)));
                MarkedEntry::new(entry).mark(entry_mark(s.mark))
            })
            .collect();
        entries.push(Entry::separator().into());
        entries.push(Entry::new(None, words.no_status.clone()).into());
        if let Some(i) = marked_entry_rows(ui, &entries) {
            picked = entries.get(i).and_then(MarkedEntry::value).map(|code| Pick::Status(*code));
        }
    });
    picked
}

/// The Hinting submenu.
fn hinting(ui: &mut Ui, words: &ContextMenuWords) -> Option<Pick> {
    let labels = [&words.autohint_tt, &words.remove_tt, &words.autohint_ps, &words.remove_ps];
    let mut picked = None;
    let _ = Menu::submenu_row(ui, words.hinting.clone(), |ui, _| {
        let entries: Vec<MarkedEntry<&'static str>> =
            HINTING.iter().zip(labels).map(|(id, label)| MarkedEntry::new(Entry::new(*id, label.clone())).danger(DESTRUCTIVE.contains(id))).collect();
        if let Some(i) = marked_entry_rows(ui, &entries) {
            picked = entries.get(i).and_then(MarkedEntry::value).map(|id| Pick::Hinting(id));
        }
    });
    picked
}

/// The menu mark of a switch that is on for all, some or none of the menu's glyphs.
fn entry_mark(mark: Mark) -> EntryMark {
    match mark {
        Mark::All => EntryMark::Checked,
        Mark::Some => EntryMark::Mixed,
        Mark::None => EntryMark::Unchecked,
    }
}

/// The swatches' submenu: the twelve label colours and No Color.
fn swatches(ui: &mut Ui, caption: Label, words: &ContextMenuWords) -> Option<Pick> {
    let tokens = LabelTokens::standard();
    let mut picked = None;
    let _ = Menu::submenu_row(ui, caption, |ui, _| {
        let mut entries: Vec<Entry<Option<u8>>> = tokens
            .colors
            .iter()
            .zip(COLOR_LABEL_NAMES)
            .zip(0u8..)
            .map(|(([r, g, b], name), i)| {
                Entry::new(Some(i), name).swatch(Srgba::new(f32::from(*r) / 255.0, f32::from(*g) / 255.0, f32::from(*b) / 255.0, 1.0))
            })
            .collect();
        entries.push(Entry::new(None, words.no_color.clone()).empty_swatch());
        if let Some(i) = entry_rows(ui, &entries) {
            picked = entries.get(i).and_then(Entry::value).map(|c| Pick::Color(*c));
        }
    });
    picked
}

/// The Copy Glyph Names submenu.
fn copy_names(ui: &mut Ui, words: &ContextMenuWords) -> Option<Pick> {
    let labels = [&words.space_separated, &words.one_per_line, &words.comma_separated, &words.slashed, &words.python_list, &words.unicodes];
    let mut picked = None;
    let _ = Menu::submenu_row(ui, words.copy_names.clone(), |ui, _| {
        let entries: Vec<Entry<&'static str>> =
            COPY_FORMATS.iter().zip(labels).map(|((format, _), label)| Entry::new(*format, label.clone())).collect();
        if let Some(i) = entry_rows(ui, &entries) {
            picked = entries.get(i).and_then(Entry::value).map(|f| Pick::CopyNames(f));
        }
    });
    picked
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests;
