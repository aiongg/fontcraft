//! The Modules panel: a section per installed filter module that declares parameters, titled with the module's
//! name, with one field per parameter drawn from the manifest's schema (modules run no interface code) and an Apply
//! button.
//!
//! | Control | Runs |
//! |---|---|
//! | a field | `palette.setModuleValue {id, name, value}` |
//! | Apply | `palette.applyModule {id}`: the filter on what its Filter menu item acts on; disabled, with the reason as tooltip, while the module cannot run |

use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Tracked};
use craft::ui::display::{Shade, Text};
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::Section;
use egui::Ui;
use serde_json::{Value, json};

use super::widgets;
use crate::app::Msg;
use crate::app::models::palette_modules::{ModuleField, ModuleSection, ModulesModel, ParamKind};

craft::ui::words! {
    /// The words of the Modules panel.
    pub struct ModulesWords {
        /// Shown while no filter module with parameters is installed.
        none: "fontcraftModules.none" = "No installed filter module has parameters";
        apply: "fontcraftModules.apply" = "Apply";
    }
}

pub struct Props<'a> {
    pub model: &'a Tracked<ModulesModel>,
    pub sent: &'a mut Vec<Msg>,
}

pub fn show(ui: &mut Ui, mut props: Props<'_>) {
    view(&mut props, ui);
}

fn view(props: &mut Props<'_>, ui: &mut Ui) {
    let words = Env::of(ui).words::<ModulesWords>();
    let model: &ModulesModel = props.model;
    let sent = &mut *props.sent;
    if model.sections.is_empty() {
        Text::new(words.none.clone()).shade(Shade::Muted).show(ui);
        return;
    }
    for section in &model.sections {
        Section::new(Label::from(section.name.clone())).show(ui, |ui, _| form(ui, &words, section, sent));
    }
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

fn form(ui: &mut Ui, words: &ModulesWords, section: &ModuleSection, sent: &mut Vec<Msg>) {
    for field in &section.fields {
        if let Some(value) = field_view(ui, field) {
            sent.push(shell("palette.setModuleValue", json!({"id": section.id, "name": field.name, "value": value})));
        }
    }
    let enabled = section.blocked.is_none();
    let tip = section.blocked.clone().unwrap_or_else(|| section.description.clone());
    let mut apply = Button::new(words.apply.clone()).enabled(enabled);
    if !tip.trim().is_empty() {
        apply = apply.tooltip(Label::from(tip));
    }
    if apply.show(ui).clicked() && enabled {
        sent.push(shell("palette.applyModule", json!({"id": section.id})));
    }
}

/// One parameter's field; the value committed.
fn field_view(ui: &mut Ui, field: &ModuleField) -> Option<Value> {
    let label = Label::from(field.name.clone());
    match field.kind {
        ParamKind::Number => widgets::number(ui, label, field.number).map(|n| json!(n)),
        ParamKind::Int => widgets::number(ui, label, field.number).map(|n| json!(n.round())),
        ParamKind::Bool => {
            let out = Checkbox::new(Bind::given(field.on)).label(label).show(ui);
            out.events.committed().map(|on| json!(*on)).filter(|v| *v != json!(field.on))
        }
        ParamKind::Choice => {
            let entries: Vec<Entry<String>> = field.options.iter().map(|o| Entry::new(o.clone(), Label::from(o.clone()))).collect();
            let out = Select::new(label, Bind::given(field.choice.clone()), &entries).show(ui);
            out.events.committed().filter(|picked| **picked != field.choice).map(|picked| json!(picked))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::contract::Tracked;
    use craft::ui::runtime::commands::CommandRef;
    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};
    use serde_json::json;

    use super::{Props, show};
    use crate::app::Msg;
    use crate::app::models::palette_modules::{ModuleField, ModuleSection, ModulesModel};

    /// A typed parameter keeps its value in the form; Apply runs the module.
    #[test]
    fn a_section_sets_its_values_and_applies() {
        let model = Tracked::new(ModulesModel {
            sections: vec![ModuleSection {
                id: "org.test.shift".into(),
                name: "Shift".into(),
                description: "Moves the outline".into(),
                blocked: None,
                fields: vec![ModuleField { name: "dx".into(), number: Some(10.0), ..Default::default() }],
            }],
        });
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            show(ui, Props { model: &model, sent: &mut sent });
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let dx = fixture.by_label("dx").expect("the dx field");
        fixture.click(&dx).key(Key::A, Modifiers::COMMAND).type_text("25").key(Key::Enter, Modifiers::NONE).settle();
        let apply = fixture.by_label("Apply").expect("the Apply button");
        fixture.click(&apply).settle();
        let shell = |id: &str, params| Msg::Shell(CommandRef::with_params(id, params));
        assert_eq!(
            *posted.borrow(),
            vec![
                shell("palette.setModuleValue", json!({"id": "org.test.shift", "name": "dx", "value": 25.0})),
                shell("palette.applyModule", json!({"id": "org.test.shift"})),
            ]
        );
    }
}
