//! The body of a panel not yet ported: its title.

use craft::ui::contract::Show as _;
use craft::ui::display::Text;
use craft::ui::runtime::env::Label;
use egui::Ui;

pub struct Props {
    pub title: Label,
}

pub fn show(ui: &mut Ui, mut props: Props) {
    view(&mut props, ui);
}

fn view(props: &mut Props, ui: &mut Ui) {
    Text::new(props.title.clone()).show(ui);
}
