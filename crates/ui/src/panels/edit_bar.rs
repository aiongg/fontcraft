//! The Edit View's controls in the status bar, shown while an Edit View tab shows (row `editView.bottomBar`): the
//! Preview Area button after the Features menu, and at the bar's end the writing direction buttons, the kerning mode
//! button, and zoom out, the zoom field and zoom in.
//!
//! | Control | Command |
//! |---|---|
//! | Preview Area | `view.togglePreviewArea` |
//! | Left to Right, Right to Left, Top to Bottom | `editView.setWritingDirection {direction}` |
//! | Kerning mode: no kerning, kerning, locked kerning | `kerning.setMode`, which goes to the next of the three |
//! | Zoom Out, Zoom In | `view.zoomOut`, `view.zoomIn` |
//! | Zoom field: screen points per 1000 units, as the font size the text shows at | `view.setZoom {size}` |

use craft::behavior::IconId;
use craft::ui::buttons::IconButton;
use craft::ui::contract::{Bind, Props as _, Show as _};
use craft::ui::display::Divider;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::text::NumberField;
use egui::Ui;
use fontcraft_textlayout::KerningMode;
use serde_json::json;

use super::widgets;
use crate::app::Msg;
use crate::canvas::text::WritingDirection;

craft::ui::words! {
    /// The words of the Edit View's status bar controls.
    pub struct EditBarWords {
        left_to_right: "fontcraftEditBar.leftToRight" = "Left to Right";
        right_to_left: "fontcraftEditBar.rightToLeft" = "Right to Left";
        top_to_bottom: "fontcraftEditBar.topToBottom" = "Top to Bottom";
        /// The kerning mode button's name.
        kerning: "fontcraftEditBar.kerning" = "Kerning Mode";
        kerning_none: "fontcraftEditBar.kerningNone" = "No kerning (click for kerning)";
        kerning_on: "fontcraftEditBar.kerningOn" = "Kerning (click for locked kerning)";
        kerning_locked: "fontcraftEditBar.kerningLocked" = "Locked kerning (click for no kerning)";
        zoom_out: "fontcraftEditBar.zoomOut" = "Zoom Out";
        zoom_in: "fontcraftEditBar.zoomIn" = "Zoom In";
        /// The zoom field's name.
        zoom: "fontcraftEditBar.zoom" = "Zoom";
        /// The unit after the zoom value: display points per 1000 units.
        zoom_unit: "fontcraftEditBar.zoomUnit" = "pt";
        preview: "fontcraftEditBar.preview" = "Preview Area";
        preview_tip: "fontcraftEditBar.previewTip" = "Show or hide the Preview Area";
    }
}

/// What the controls show: the live Edit View tab's state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditBar {
    pub direction: WritingDirection,
    pub kerning: KerningMode,
    /// Screen points per 1000 font units.
    pub zoom: f64,
    pub preview_area: bool,
}

pub struct Props<'a> {
    pub bar: EditBar,
    pub sent: &'a mut Vec<Msg>,
}

fn shell(id: &str, params: serde_json::Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

/// The Preview Area button, beside the Features menu.
pub fn preview_button(ui: &mut Ui, props: Props<'_>) {
    let words = Env::of(ui).words::<EditBarWords>();
    let out = IconButton::new(IconId::new("eye"))
        .label(words.preview.clone())
        .tooltip(words.preview_tip.clone())
        .selected(props.bar.preview_area)
        .id_salt("fontcraftEditBar.preview")
        .show(ui);
    if out.clicked() {
        props.sent.push(shell("view.togglePreviewArea", json!({})));
    }
}

/// The controls at the bar's end: writing direction, kerning mode, zoom.
pub fn end_controls(ui: &mut Ui, props: Props<'_>) {
    let words = Env::of(ui).words::<EditBarWords>();
    let bar = props.bar;
    let sent = props.sent;
    let directions: [(WritingDirection, &'static str, &Label); 3] = [
        (WritingDirection::Ltr, "pilcrow_right", &words.left_to_right),
        (WritingDirection::Rtl, "pilcrow_left", &words.right_to_left),
        (WritingDirection::Ttb, "type_mask_vertical", &words.top_to_bottom),
    ];
    for (direction, icon, label) in directions {
        let out = IconButton::new(IconId::new(icon))
            .label(label.clone())
            .tooltip(label.clone())
            .selected(bar.direction == direction)
            .id_salt(("fontcraftEditBar.direction", direction.name()))
            .show(ui);
        if out.clicked() && bar.direction != direction {
            sent.push(shell("editView.setWritingDirection", json!({ "direction": direction.name() })));
        }
    }
    Divider::vertical().show(ui);
    let (icon, tip) = match bar.kerning {
        KerningMode::None => ("unlink", &words.kerning_none),
        KerningMode::Kerning => ("link", &words.kerning_on),
        KerningMode::Locked => ("lock", &words.kerning_locked),
    };
    let out = IconButton::new(IconId::new(icon)).label(words.kerning.clone()).tooltip(tip.clone()).id_salt("fontcraftEditBar.kerning").show(ui);
    if out.clicked() {
        sent.push(shell("kerning.setMode", json!({})));
    }
    Divider::vertical().show(ui);
    if IconButton::new(IconId::new("zoom_out")).label(words.zoom_out.clone()).tooltip(words.zoom_out.clone()).show(ui).clicked() {
        sent.push(shell("view.zoomOut", json!({})));
    }
    let zoom = Some(bar.zoom.round());
    if let Some(widgets::NumberEdit::Set(size)) =
        widgets::clearable(ui, NumberField::new(words.zoom.clone(), Bind::given(zoom)).unit(words.zoom_unit.clone()), zoom)
        && size > 0.0
    {
        sent.push(shell("view.setZoom", json!({ "size": size })));
    }
    if IconButton::new(IconId::new("zoom_in")).label(words.zoom_in.clone()).tooltip(words.zoom_in.clone()).show(ui).clicked() {
        sent.push(shell("view.zoomIn", json!({})));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};
    use fontcraft_textlayout::KerningMode;
    use serde_json::json;

    use super::{EditBar, Props, end_controls, preview_button, shell};
    use crate::canvas::text::WritingDirection;

    #[test]
    fn the_controls_post_the_edit_view_commands() {
        let bar = EditBar { direction: WritingDirection::Ltr, kerning: KerningMode::Kerning, zoom: 100.0, preview_area: false };
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            preview_button(ui, Props { bar, sent: &mut sent });
            end_controls(ui, Props { bar, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        assert!(fixture.texts().iter().any(|t| t.text == "pt"), "the zoom field shows its unit");
        for label in ["Preview Area", "Right to Left", "Kerning Mode", "Zoom In"] {
            let target = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} button"));
            fixture.click(&target).settle();
        }
        let zoom = fixture.by_label("Zoom").expect("the zoom field");
        fixture.click(&zoom).key(Key::A, Modifiers::COMMAND).type_text("250").key(Key::Enter, Modifiers::NONE).settle();
        assert_eq!(
            *posted.borrow(),
            vec![
                shell("view.togglePreviewArea", json!({})),
                shell("editView.setWritingDirection", json!({"direction": "rtl"})),
                shell("kerning.setMode", json!({})),
                shell("view.zoomIn", json!({})),
                shell("view.setZoom", json!({"size": 250.0})),
            ]
        );
    }
}
