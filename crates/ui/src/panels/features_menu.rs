//! The Features button and its menu, at the left of the status bar (rows `editView.featuresMenu`,
//! `features.menuNeedsCompile`, `features.buttonShowsActiveTags`).
//!
//! The menu's entries, top to bottom: the dash, which turns every feature off; one checkable entry per compiled
//! feature; Localized Forms; the script and language pairs the compiled `locl` feature is registered for, which work
//! while Localized Forms is on. The button shows the active tags and stays pressed (the accent fill) while any feature
//! or Localized Forms is on. A typed letter runs the entry it names, as in every menu.

use craft::ui::buttons::ToggleButton;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, Tracked, Trigger};
use craft::ui::menu::Menu;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use egui::Ui;
use serde_json::json;

use crate::app::Msg;
use crate::app::models::text::FeaturesModel;

craft::ui::words! {
    /// The words of the Features menu.
    pub struct FeaturesWords {
        /// The button with no feature on.
        features: "fontcraftFeatures.features" = "Features";
        /// The menu's name.
        menu: "fontcraftFeatures.menu" = "Preview Features";
        /// The button's tooltip.
        tip: "fontcraftFeatures.tip" = "Features to preview in the edit view's text";
        /// The entry that turns every feature off.
        clear: "fontcraftFeatures.clear" = "-";
        localized: "fontcraftFeatures.localized" = "Localized Forms";
        /// Shown at the top while the document has no feature compile.
        compile_first: "fontcraftFeatures.compileFirst" = "Compile the features in Font Info to list them here.";
        /// Over the features of a compile the font has moved on from.
        compile_again: "fontcraftFeatures.compileAgain" = "Changed since the last compile";
        /// Why a script and language entry is off.
        localized_off: "fontcraftFeatures.localizedOff" = "Turn Localized Forms on first";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<FeaturesModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

/// What an entry of the menu does.
#[derive(Clone, Debug, PartialEq)]
enum Pick {
    Clear,
    Feature(String),
    Localized,
    Pair(String, String),
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<FeaturesWords>();
    let model = &**props.model;
    let on = !model.active.is_empty() || model.localized;
    let label: Label = if model.active.is_empty() { words.features.clone() } else { model.active.join(" ").into() };
    let button = ToggleButton::new(label, Bind::given(on)).tooltip(words.tip.clone()).show(ui);
    let entries = entries(model, &words);
    let menu = Menu::new(words.menu.clone()).opened_by(&button.response, Trigger::Click).pick(ui, &entries);
    if let Some(pick) = menu.picked {
        props.sent.push(command(pick));
    }
}

fn entries(model: &FeaturesModel, words: &FeaturesWords) -> Vec<Entry<Pick>> {
    let mut out = Vec::new();
    if !model.compiled {
        out.push(Entry::heading(words.compile_first.clone()));
    } else if !model.current {
        out.push(Entry::heading(words.compile_again.clone()));
    }
    out.push(Entry::new(Pick::Clear, words.clear.clone()));
    for (tag, _) in &model.features {
        out.push(Entry::new(Pick::Feature(tag.clone()), tag.clone()).checked(model.active.contains(tag)));
    }
    out.push(Entry::separator());
    out.push(Entry::new(Pick::Localized, words.localized.clone()).checked(model.localized));
    for (script, language) in &model.language_systems {
        let current = model.script.as_deref() == Some(script.as_str()) && model.language.as_deref() == Some(language.as_str());
        let mut entry = Entry::new(Pick::Pair(script.clone(), language.clone()), format!("{script} / {language}")).checked(current);
        if !model.localized {
            entry = entry.enabled(false).disabled_reason(words.localized_off.clone());
        }
        out.push(entry);
    }
    out
}

/// The shell command an entry runs.
fn command(pick: Pick) -> Msg {
    let (id, params) = match pick {
        Pick::Clear => ("editView.clearFeatures", json!({})),
        Pick::Feature(tag) => ("editView.toggleFeature", json!({ "tag": tag })),
        Pick::Localized => ("editView.setLocalizedForms", json!({})),
        Pick::Pair(script, language) => ("editView.setLocalizedForms", json!({"on": true, "script": script, "language": language})),
    };
    Msg::Shell(CommandRef::with_params(id, params))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;

    use super::*;

    #[test]
    fn the_menu_lists_the_compiled_features_and_posts_the_picked_one() {
        let model = Tracked::new(FeaturesModel {
            compiled: true,
            current: true,
            features: vec![("liga".into(), "GSUB".into()), ("smcp".into(), "GSUB".into())],
            active: vec!["liga".into()],
            ..Default::default()
        });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        // The button shows the active tags.
        let button = fixture.by_label("liga").expect("the Features button shows the active tag");
        fixture.click(&button).settle();
        let smcp = fixture.by_label("smcp").expect("the menu lists smcp");
        fixture.click(&smcp).settle();
        assert_eq!(*posted.borrow(), vec![Msg::Shell(CommandRef::with_params("editView.toggleFeature", json!({"tag": "smcp"})))]);
    }
}
