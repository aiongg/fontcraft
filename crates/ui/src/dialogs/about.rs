//! About FontCraft (`app.about`): the version and the licences.

use craft::ui::dialog::DialogHost;
use craft::ui::patterns::form_dialogs::{AboutInfo, Contributor, Credits, HostedAbout, ModelCredit};
use craft::ui::runtime::env::{Label, Word};

use crate::app::console::About;
use crate::app::{APP, FontCraftApp};

/// One sentence about the app.
const BLURB: Word = Word::keyed("fontcraftAbout.blurb", "A font editor and font tools, open source.");
/// The licence line; the notices of the code and assets FontCraft ships are in `NOTICE` and `ASSETS.md`.
const LICENCES: Word =
    Word::keyed("fontcraftAbout.licences", "Licensed MIT OR Apache-2.0. Notices for the code and assets it ships: NOTICE and ASSETS.md.");

pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    match app.view.commands.about {
        About::Open => {
            let info = AboutInfo::new(Label::Static(APP.name), BLURB, Label::Static(APP.version)).built_with(LICENCES);
            host.open(HostedAbout::new(info, Credits::new(Vec::<Contributor>::new(), Vec::<ModelCredit>::new(), 0)));
            app.view.commands.about = About::Shown;
        }
        About::Shown => {
            if host.take::<HostedAbout>().is_some() || !host.is_open::<HostedAbout>() {
                app.view.commands.about = About::Closed;
            }
        }
        About::Closed => {}
    }
}
