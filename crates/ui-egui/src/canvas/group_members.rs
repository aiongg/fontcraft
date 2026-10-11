//! View › Show Group Members: the other members of the kerning groups at the edited pair, drawn
//! as thin outlines behind the text so collisions across a group show while one pair is kerned.
//!
//! Two glyphs take part (`kerning.groupMembersDisplayScope`): the edited glyph, with the group
//! of its side that faces the glyph before it, and that preceding glyph on the same line, with
//! the group of its side that faces the edited glyph. Which side faces the pair follows the
//! kerning lookup ([`facing`]): in left-to-right text the edited glyph shows its left group and
//! the preceding one its right group. Each member is placed so its facing side lines up with
//! the glyph it stands in for.

use egui::Stroke;
use fontcraft_geom::Affine;
use fontcraft_model::Font;
use fontcraft_spacing::kerning::{self, Direction, GroupSide, Position, facing};
use fontcraft_textlayout::Layout;
use serde_json::{Value, json};

use super::fill::FillCache;
use super::{Painting, flatten};
use crate::app::FontcraftApp;
use crate::shell::{Args, UiCommand};
use crate::state::{Tab, WritingDirection};

/// Most members drawn for one glyph; a larger group shows its first ones in glyph order.
pub const MAX_MEMBERS: usize = 200;

pub const COMMANDS: &[UiCommand] = &[(
    "view.showGroupMembers",
    "Show Group Members",
    &["View"],
    "",
    "{on?: bool} draw, behind the edited pair, the other members of the edited glyph's facing kerning group and of the preceding glyph's (omitted: toggle) → {on}",
)];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "view.showGroupMembers" => {
            let on = args.toggle(app.state.panels.group_members)?;
            app.state.panels.group_members = on;
            Ok(json!({"on": on}))
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

/// One member drawn: the glyph, the item it stands in for, the side its group is on, and where
/// its origin goes in the layout's coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub glyph: String,
    pub item: usize,
    pub side: GroupSide,
    pub group: String,
    pub x: f64,
    pub y: f64,
}

/// The kerning direction of the tab's writing direction.
fn direction(tab: &Tab) -> Direction {
    match tab.direction {
        WritingDirection::Ltr => Direction::Ltr,
        WritingDirection::Rtl => Direction::Rtl,
        WritingDirection::Ttb => Direction::Vertical,
    }
}

/// The members to draw for the tab's edited glyph and the glyph before it on its line.
pub fn shown(font: &Font, tab: &Tab, layout: &Layout) -> Vec<Shown> {
    let Some(edited) = tab.edited_index() else { return Vec::new() };
    let direction = direction(tab);
    let line = layout.placed.get(edited).map(|p| p.line);
    let before = edited.checked_sub(1).filter(|i| layout.placed.get(*i).map(|p| p.line) == line);
    let mut out = Vec::new();
    for (item, position) in [(before, Position::First), (Some(edited), Position::Second)] {
        if let Some(item) = item {
            members_of(font, tab, layout, item, facing(direction, position), &mut out);
        }
    }
    out
}

fn members_of(font: &Font, tab: &Tab, layout: &Layout, item: usize, side: GroupSide, out: &mut Vec<Shown>) {
    let Some(placed) = layout.placed.get(item) else { return };
    let Some(name) = placed.glyph.as_deref() else { return };
    let Some(group) = font.glyph(name).and_then(|g| side.of(&g.kern_groups)) else { return };
    for member in kerning::members(font, side, group).into_iter().filter(|m| *m != name).take(MAX_MEMBERS) {
        let Some(layer) = super::view::shown_layer(font, tab, member) else { continue };
        // The facing side lines up: the left side at the item's origin, the right side at the
        // end of its advance. Vertical members share the item's origin.
        let x = match side {
            GroupSide::Right => placed.x + placed.advance - layer.width,
            GroupSide::Left | GroupSide::Top | GroupSide::Bottom => placed.x,
        };
        if !x.is_finite() {
            continue;
        }
        out.push(Shown { glyph: member.to_string(), item, side, group: group.to_string(), x, y: placed.y });
    }
}

/// Draws the members [`shown`] gives as thin outlines; `origin` is where the edited glyph sits.
pub fn draw(c: &Painting, cache: &mut FillCache, font: &Font, tab: &Tab, layout: Option<&Layout>, origin: [f64; 2], doc: (u64, u64)) {
    let Some(layout) = layout else { return };
    let to_screen = Affine::translate(c.view.origin.to_vec2()) * Affine::scale_non_uniform(c.view.scale(), -c.view.scale());
    let stroke = Stroke::new(c.tokens.group_members_width, c.tokens.group_members);
    for member in shown(font, tab, layout) {
        let Some(layer) = super::view::shown_layer(font, tab, &member.glyph) else { continue };
        let Some(composite) = cache.outlines.get(doc, font, &member.glyph, layer) else { continue };
        let mut outline = composite.outline.clone();
        outline.apply_affine(Affine::translate((member.x - origin[0], member.y - origin[1])));
        for contour in flatten(&(to_screen * outline)) {
            c.painter.add(egui::Shape::line(contour, stroke));
        }
    }
}
