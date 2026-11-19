//! Palette › Anchor Context: the context of the selected contextual anchor, shown while exactly one anchor whose name
//! starts with `*` is selected in the edited layer. The field holds feature code, one context per line, and commits
//! through `anchor.setContext` when it loses the focus.

use craft::ui::contract::{Bind, Show as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::runtime::env::Env;
use craft::ui::text::TextField;
use egui::Ui;
use fontcraft_engine::features::marks::contextual::MAX_CONTEXT_BYTES;
use serde_json::json;

use crate::app::Msg;
use crate::app::models::palette::AnchorContextModel;

craft::ui::words! {
    /// The words of the Anchor Context section.
    pub struct AnchorContextWords {
        context: "fontcraftAnchorContext.context" = "Context";
        none: "fontcraftAnchorContext.none" = "Select one anchor whose name starts with *";
    }
}

/// Rows the field shows.
const ROWS: usize = 3;

pub struct Props<'a> {
    pub model: &'a Tracked<AnchorContextModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<AnchorContextWords>();
    let m: &AnchorContextModel = props.model;
    let (Some(anchor), Some(targets)) = (&m.anchor, &m.targets) else {
        Text::new(words.none.clone()).shade(Shade::Muted).show(ui);
        return;
    };
    Text::new(anchor.clone()).shade(Shade::Muted).show(ui);
    let out = TextField::new(words.context.clone(), Bind::given(m.context.clone())).lines(ROWS).mono().limit(MAX_CONTEXT_BYTES).show(ui);
    if let Some(text) = out.events.committed()
        && *text != m.context
    {
        props.sent.push(Msg::run("anchor.setContext", targets.with(json!({"anchor": anchor, "context": text}))));
    }
}
