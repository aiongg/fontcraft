//! Child nodes for what a widget draws itself: the points of a canvas, the glyphs of a drawn page.
//!
//! Meant for craft-ui-contract, `src/runtime/access`: a component's parts become sub-nodes of its AccessKit node, and a
//! tool's `ToolAccess` makes one for each anchor of its drawing, but a canvas body, and a widget that is no craft
//! component, can add none, and `ToolAccess` is built on every pass, read or not. This stand-in adds nodes under a shown
//! widget's own node, and only on a pass whose tree somebody reads ([`listening`]).
//!
//! A child has a name, a value and a place, and no action: it cannot take the focus, and a click on its place is the
//! widget's. The full version in craft would let a child declare the actions its widget answers.

use egui::accesskit::{self, Role};
use egui::{Rect, Response};

/// One thing a widget draws.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawn {
    /// What tells it from the widget's other children, from pass to pass.
    pub key: (&'static str, usize, usize),
    pub name: String,
    /// What it holds, such as its coordinates.
    pub value: Option<String>,
    /// Where it is drawn, on the screen.
    pub rect: Rect,
    /// Whether it is selected; `None` when it cannot be.
    pub selected: Option<bool>,
}

/// Whether this pass builds an AccessKit tree with a node for the shown widget. Costs one lookup, so a caller asks it
/// before it lists what it drew.
pub fn listening(response: &Response) -> bool {
    response.ctx.has_accesskit_node(response.id)
}

/// Adds the first `most` of `items` that show inside `clip` under the widget's node, in their order. Call it right after
/// the widget showed, while [`listening`]; nothing happens on a pass without a tree.
pub fn report(response: &Response, clip: Rect, most: usize, items: impl IntoIterator<Item = Drawn>) {
    if !listening(response) {
        return;
    }
    for item in items.into_iter().filter(|item| item.rect.is_finite() && item.rect.intersects(clip)).take(most) {
        let id = response.id.with(("craft_ext.access.child", item.key));
        response.ctx.register_accesskit_parent(id, response.id);
        response.ctx.accesskit_node_builder(id, |node| {
            node.set_role(Role::GraphicsObject);
            node.set_label(item.name);
            if let Some(value) = item.value {
                node.set_value(value);
            }
            let r = item.rect;
            node.set_bounds(accesskit::Rect { x0: r.min.x.into(), y0: r.min.y.into(), x1: r.max.x.into(), y1: r.max.y.into() });
            if let Some(selected) = item.selected {
                node.set_selected(selected);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use craft::ui::testing::Fixture;
    use craft::ui::testing::access::name;
    use egui::{Sense, pos2, vec2};

    use super::*;

    /// The things a widget drew are named children of its node, up to the cap and inside the clip.
    #[test]
    fn drawn_things_are_named_children_of_their_widget() {
        let mut f = Fixture::new(|ui| {
            let (rect, response) = ui.allocate_exact_size(vec2(200.0, 100.0), Sense::hover());
            response.widget_info(|| egui::WidgetInfo::labeled(Role::Image, true, "Page"));
            let items = (0..5).map(|i| Drawn {
                key: ("point", i, 0),
                name: format!("Point {i}"),
                value: Some(format!("{i}, 0")),
                // The first one lies outside the widget.
                rect: Rect::from_center_size(rect.min + vec2(i as f32 * 20.0 - 10.0, 50.0), vec2(4.0, 4.0)),
                selected: Some(i == 2),
            });
            report(&response, rect, 3, items);
        });
        f.settle();
        let tree = f.tree();
        let page = tree.keys().into_iter().find(|key| name(tree, *key) == Some("Page")).expect("the widget's node");
        let mut children: Vec<_> = tree.keys().into_iter().filter(|key| tree.parent_of_key(*key) == Some(page)).collect();
        children.sort_by_key(|key| name(tree, *key).map(str::to_string));
        let names: Vec<_> = children.iter().map(|key| name(tree, *key).unwrap_or_default()).collect();
        assert_eq!(names, ["Point 1", "Point 2", "Point 3"]);
        let second = children[1];
        assert_eq!(tree.value_of(second), Some("2, 0"));
        let target = f.by_label("Point 2").expect("a child is found by its name");
        assert!(target.rect.contains(pos2(target.rect.center().x, target.rect.center().y)) && target.rect.width() < 5.0);
    }
}
