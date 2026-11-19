//! The Kerning Groups Editor (`dialog.kerningGroupsEditor`): the kerning groups of one side, the members of the current
//! group as cells, as a list or stacked, and every glyph of the font below for dragging into the group.
//!
//! | Input | Does |
//! |---|---|
//! | Left Groups, Right Groups | the side whose groups are listed (`{side}`) |
//! | Cells, List, Stacked | how the members are drawn (`{view}`) |
//! | click a group | makes it the current group (`{group}`) |
//! | New Group field, Enter | starts a new, empty group of that name |
//! | Clear Group | clears the current group from every member (`kerning.setLeftGroup` or `kerning.setRightGroup` with `null`) |
//! | drag glyphs from the font onto the members | adds them to the current group |
//! | Delete, Remove from Group (List) | takes the selected members out of the group |
//! | Propose Groups, Apply, Discard | `kerning.buildGroups` without applying, its proposal shown for review; Apply stores it |
//!
//! The stacked view draws every member over the others, aligned where the group's side meets the pair: a left group
//! (the glyph comes second in left-to-right text) on the glyphs' origin, a right group on their advance width, so a
//! member whose shape or sidebearing does not fit stands out.

use std::borrow::Cow;
use std::sync::Arc;

use craft::behavior::{Selection, keyed};
use craft::ui::buttons::Button;
use craft::ui::choice::Segmented;
use craft::ui::contract::{
    Bind, CollectionEvent, DropKind, DropPlace, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Tracked,
};
use craft::ui::display::{Picture, PictureSize, Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, ThumbAspect, ThumbSize};
use craft::ui::primitives::StrokeWidth;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::text::{NumberField, TextField};
use craft::ui::tile_grid::{CaptionPlace, Tile, TileGrid};
use egui::{Stroke, Ui, pos2};
use serde_json::{Value, json};

use super::{baseline, scale_for, stroke_glyph};
use crate::app::Msg;
use crate::app::models::kerning_tools::GroupsEditorModel;
use crate::panels::font_view::cells;
use crate::shell::kerning_tools::{EditorSide, GroupsView, MAX_DIFFERENCE};

craft::ui::words! {
    /// The words of the Kerning Groups Editor.
    pub struct GroupsEditorWords {
        side: "fontcraftGroupsEditor.side" = "Side";
        left_groups: "fontcraftGroupsEditor.leftGroups" = "Left Groups";
        right_groups: "fontcraftGroupsEditor.rightGroups" = "Right Groups";
        view: "fontcraftGroupsEditor.view" = "View";
        cells: "fontcraftGroupsEditor.cells" = "Cells";
        list: "fontcraftGroupsEditor.list" = "List";
        stacked: "fontcraftGroupsEditor.stacked" = "Stacked";
        groups: "fontcraftGroupsEditor.groups" = "Kerning Groups";
        no_groups: "fontcraftGroupsEditor.noGroups" = "No groups on this side";
        new_tag: "fontcraftGroupsEditor.newTag" = "new";
        new_group: "fontcraftGroupsEditor.newGroup" = "New Group";
        clear: "fontcraftGroupsEditor.clear" = "Clear Group";
        clear_tip: "fontcraftGroupsEditor.clearTip" = "Take every member out of this group";
        members: "fontcraftGroupsEditor.members" = "Members";
        no_members: "fontcraftGroupsEditor.noMembers" = "Drag glyphs here to add them";
        choose: "fontcraftGroupsEditor.choose" = "Choose or start a group";
        remove: "fontcraftGroupsEditor.remove" = "Remove from Group";
        stacked_members: "fontcraftGroupsEditor.stackedMembers" = "Members drawn over each other";
        max_difference: "fontcraftGroupsEditor.maxDifference" = "Largest Profile Difference";
        propose: "fontcraftGroupsEditor.propose" = "Propose Groups";
        propose_tip: "fontcraftGroupsEditor.proposeTip" = "Group glyphs whose side profiles match, for review";
        proposed: "fontcraftGroupsEditor.proposed" = "Proposed Groups";
        skipped: "fontcraftGroupsEditor.skipped" = "skipped";
        apply: "fontcraftGroupsEditor.apply" = "Apply";
        discard: "fontcraftGroupsEditor.discard" = "Discard";
        glyphs: "fontcraftGroupsEditor.glyphs" = "Glyphs (drag onto the members)";
        no_font: "fontcraftGroupsEditor.noFont" = "No font is open";
    }
}

/// What a glyph dragged from the font's glyphs carries: the dragged glyph names.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphDrag(pub Vec<String>);

pub struct Props<'a> {
    pub model: &'a Tracked<GroupsEditorModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn editor(params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params("dialog.kerningGroupsEditor", params))
}

/// Sets the group of `glyphs` on the editor's side; `None` takes them out of their group.
pub fn set_group(model: &GroupsEditorModel, glyphs: &[String], group: Option<&str>) -> Msg {
    Msg::run(model.side.set_command(), json!({ "glyphs": glyphs, "group": group }))
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let model: &GroupsEditorModel = props.model;
    let sent = &mut *props.sent;
    let words = Env::of(ui).words::<GroupsEditorWords>();
    let _ = Stack::new().fill_width().show(ui, |ui, _| {
        if !model.has_font {
            Text::new(words.no_font.clone()).shade(Shade::Muted).show(ui);
            return;
        }
        switches(ui, &words, model, sent);
        group_list(ui, &words, model, sent);
        members(ui, &words, model, sent);
        proposal(ui, &words, model, sent);
        glyphs(ui, &words, model);
    });
}

fn switches(ui: &mut Ui, words: &GroupsEditorWords, model: &GroupsEditorModel, sent: &mut Vec<Msg>) {
    let sides = [Entry::new(EditorSide::Left, words.left_groups.clone()), Entry::new(EditorSide::Right, words.right_groups.clone())];
    let labels = [&words.cells, &words.list, &words.stacked];
    let views: Vec<Entry<GroupsView>> = GroupsView::ALL.into_iter().zip(labels).map(|(v, l)| Entry::new(v, l.clone())).collect();
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        let mut side = model.side;
        Segmented::new(words.side.clone(), Bind::bound(&mut side), &sides).show(ui);
        if side != model.side {
            sent.push(editor(json!({ "side": side.name() })));
        }
        let mut shown = model.view;
        Segmented::new(words.view.clone(), Bind::bound(&mut shown), &views).show(ui);
        if shown != model.view {
            sent.push(editor(json!({ "view": shown.name() })));
        }
    });
}

/// A group row's text: its name and member count.
pub fn group_text(name: &str, count: usize) -> String {
    format!("{name} ({count})")
}

fn group_list(ui: &mut Ui, words: &GroupsEditorWords, model: &GroupsEditorModel, sent: &mut Vec<Msg>) {
    // The current group comes last when it has no member yet.
    let mut rows: Vec<(String, String)> = model.groups.iter().map(|g| (g.name.clone(), group_text(&g.name, g.count))).collect();
    if let Some(g) = model.group.as_ref().filter(|_| model.is_new) {
        rows.push((g.clone(), format!("{g} ({})", words.new_tag.as_str())));
    }
    let before = model.group.clone().map_or_else(Selection::default, Selection::single);
    let mut after = before.clone();
    let source = keyed(&rows, |(name, _)| name.clone()).with_text(|(_, text)| Cow::Borrowed(text.as_str()));
    let _ = List::<String>::new(words.groups.clone())
        .empty(words.no_groups.clone())
        .type_ahead(true)
        .select(SelectMode::Single)
        .selection(Bind::bound(&mut after))
        .show_items(ui, &source, &mut |name, _| {
            let text = rows.iter().find(|(n, _)| n == name).map_or_else(String::new, |(_, t)| t.clone());
            ListRow::new(text)
        });
    if after.items != before.items
        && let Some(name) = after.items.last()
    {
        sent.push(editor(json!({ "group": name })));
    }
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        let field = TextField::new(words.new_group.clone(), Bind::given(String::new())).placeholder(words.new_group.clone()).show(ui);
        if field.submitted
            && let Some(name) = field.events.committed().map(|t| t.trim().to_string())
            && !name.is_empty()
        {
            sent.push(editor(json!({ "group": name })));
        }
        let clear = Button::new(words.clear.clone()).tooltip(words.clear_tip.clone()).enabled(!model.members.is_empty()).show(ui);
        if clear.clicked() {
            sent.push(set_group(model, &model.members, None));
        }
    });
}

/// The glyphs a drop carries that are not members yet.
fn dropped<K, E>(model: &GroupsEditorModel, events: &[CollectionEvent<K, E>]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for event in events {
        if let CollectionEvent::Dropped { payload, .. } = event
            && let Some(GlyphDrag(names)) = payload.get::<GlyphDrag>()
        {
            for name in names {
                if !model.members.contains(name) && !out.contains(name) {
                    out.push(name.clone());
                }
            }
        }
    }
    out
}

fn members(ui: &mut Ui, words: &GroupsEditorWords, model: &GroupsEditorModel, sent: &mut Vec<Msg>) {
    let Some(group) = model.group.as_deref() else {
        Text::new(words.choose.clone()).shade(Shade::Muted).show(ui);
        return;
    };
    let source = keyed(&model.members, |name| name.clone()).with_text(|name| Cow::Borrowed(name.as_str()));
    let added = match model.view {
        GroupsView::Cells | GroupsView::Stacked => {
            if model.view == GroupsView::Stacked {
                stacked(ui, words, model);
            }
            let out = TileGrid::<String>::new(words.members.clone())
                .empty(words.no_members.clone())
                .caption(CaptionPlace::Below)
                .takes_drop(DropKind::new::<GlyphDrag>(DropPlace::Between, |_, _| true))
                .show_items(ui, &source, &mut |name, _| glyph_tile(model, name));
            dropped(model, &out.response.events)
        }
        GroupsView::List => {
            let mut picked = Selection::default();
            let out = List::<String>::new(words.members.clone())
                .empty(words.no_members.clone())
                .select(SelectMode::Multi)
                .deletable(true)
                .selection(Bind::bound(&mut picked))
                .takes_drop(DropKind::new::<GlyphDrag>(DropPlace::Between, |_, _| true))
                .show_items(ui, &source, &mut |name, _| ListRow::new(name.clone()));
            for event in &out.events {
                if let CollectionEvent::Delete(names) = event {
                    sent.push(set_group(model, names, None));
                }
            }
            let remove = Button::new(words.remove.clone()).enabled(!picked.items.is_empty()).show(ui);
            if remove.clicked() {
                sent.push(set_group(model, &picked.items, None));
            }
            dropped(model, &out.events)
        }
    };
    if !added.is_empty() {
        sent.push(set_group(model, &added, Some(group)));
    }
}

/// A glyph's tile: its outline over its name.
fn glyph_tile<'a>(model: &'a GroupsEditorModel, name: &str) -> Tile<'a> {
    let tile = Tile::new(name.to_string());
    let Some(art) = model.art.get(name).map(Arc::clone) else { return tile };
    let (ascender, descender) = (model.ascender, model.descender);
    tile.thumb(ThumbAspect::SQUARE, move |cx, rect| cells::paint(cx, rect, &art, ascender, descender))
}

/// Every member stroked over the others, aligned on the group's side, with the alignment line.
fn stacked(ui: &mut Ui, words: &GroupsEditorWords, model: &GroupsEditorModel) {
    let upm = (model.ascender - model.descender).max(1.0);
    let left = model.side == EditorSide::Left;
    Picture::painter(words.stacked_members.clone(), ThumbAspect::new(3, 2), move |cx, rect| {
        let scale = scale_for(rect, upm);
        let line_x = if left { rect.left() + rect.width() * 0.25 } else { rect.right() - rect.width() * 0.25 };
        let base = baseline(rect, upm) + (model.ascender + model.descender - upm * 0.6) / 2.0 * scale;
        cx.faded(0.5, |cx| {
            let stroke = Stroke::new(cx.stroke_width(StrokeWidth::Hairline), cx.ink());
            cx.painter.line_segment([pos2(line_x, rect.top()), pos2(line_x, rect.bottom())], stroke);
        });
        for member in &model.stacked {
            if let Some(art) = model.art.get(&member.name) {
                stroke_glyph(cx, art, pos2(line_x + member.offset as f32 * scale, base), scale);
            }
        }
    })
    .extent(PictureSize::Thumb(ThumbSize::Large))
    .show(ui);
}

fn proposal(ui: &mut Ui, words: &GroupsEditorWords, model: &GroupsEditorModel, sent: &mut Vec<Msg>) {
    let Some(groups) = &model.proposal else {
        let _ = Row::new().fill_width().show(ui, |ui, _| {
            let field = NumberField::new(words.max_difference.clone(), Bind::given(Some(model.max_difference))).range(0.0..=MAX_DIFFERENCE).show(ui);
            if let Some(Some(v)) = field.edit.events.committed() {
                sent.push(editor(json!({ "maxDifference": v })));
            }
            if Button::new(words.propose.clone()).tooltip(words.propose_tip.clone()).show(ui).clicked() {
                sent.push(editor(json!({ "propose": { "maxDifference": model.max_difference, "side": model.side.name() } })));
            }
        });
        return;
    };
    Text::new(words.proposed.clone()).show(ui);
    let lines: Vec<String> = groups.iter().map(|g| format!("{} {}: {}", g.side, g.name, g.glyphs.join(" "))).collect();
    let source = keyed(&lines, |line| line.clone()).with_text(|line| Cow::Borrowed(line.as_str()));
    let _ = List::<String>::new(words.proposed.clone()).show_items(ui, &source, &mut |line, _| ListRow::new(line.clone()));
    if model.skipped > 0 {
        Text::new(format!("{} {}", model.skipped, words.skipped.as_str())).shade(Shade::Muted).show(ui);
    }
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        if Button::new(words.apply.clone()).show(ui).clicked() {
            sent.push(editor(json!({ "apply": true })));
        }
        if Button::new(words.discard.clone()).show(ui).clicked() {
            sent.push(editor(json!({ "propose": null })));
        }
    });
}

/// The font's glyphs as tiles that drag onto the members.
fn glyphs(ui: &mut Ui, words: &GroupsEditorWords, model: &GroupsEditorModel) {
    Text::new(words.glyphs.clone()).shade(Shade::Muted).show(ui);
    let source = keyed(&model.glyphs, |name| name.clone()).with_text(|name| Cow::Borrowed(name.as_str()));
    let mut picked = Selection::default();
    let _ = TileGrid::<String>::new(words.glyphs.clone())
        .caption(CaptionPlace::Below)
        .type_ahead(true)
        .select(SelectMode::Multi)
        .selection(Bind::bound(&mut picked))
        .drag_payload(|drag| Some(GlyphDrag(drag.keys.clone())))
        .show_items(ui, &source, &mut |name, _| glyph_tile(model, name));
}
