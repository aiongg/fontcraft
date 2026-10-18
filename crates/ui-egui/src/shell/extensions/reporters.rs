//! Reporter switches (`view.showReporter`): which reporter modules Edit View draws. A switch is
//! view state, kept in the interface state with the other panel settings, so it is remembered
//! across launches; the View menu has one Show entry per reporter module
//! ([`crate::shell::menus_dynamic`]) and the canvas draws the shown ones
//! ([`crate::canvas::reporters`]).

use fontcraft_engine::Session;
use fontcraft_engine::plugins::Kind;
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::shell::{Args, UiCommand};

/// Most reporter switches kept. Only installed reporters can be switched on, and at most
/// [`fontcraft_engine::plugins::error::MAX_INSTALLED`] modules are installed; switches of
/// modules removed since are dropped first when the list is full.
pub const MAX_SHOWN_REPORTERS: usize = 256;

pub const COMMAND: UiCommand = (
    "view.showReporter",
    "Show (reporter name)",
    &[],
    "",
    "{id: reporter module id, on?: bool (omitted: toggle)} show or hide an installed reporter module's drawing in Edit View; the View menu lists one Show entry per reporter module. The switch is view state and is remembered → {id, on, shown: [ids]}",
);

/// Whether the module `id` is an installed reporter.
fn is_reporter(session: &Session, id: &str) -> bool {
    session.extensions().registry.get(id).is_some_and(|m| m.plugin.kind() == Kind::Reporter)
}

pub fn run(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let id = args.string("id")?.ok_or_else(|| args.err("give `id`: a reporter module's id"))?;
    let was = app.state.panels.extensions.reporters.iter().any(|s| s == id);
    let on = args.toggle(was)?;
    if on && !is_reporter(&app.session, id) {
        return Err(match app.session.extensions().registry.get(id) {
            Some(m) => args.err(format!("module {id:?} is a {} module, not a reporter", m.plugin.kind().name())),
            None => args.err(format!("no module {id:?} is installed")),
        });
    }
    let mut list = app.state.panels.extensions.reporters.clone();
    list.retain(|s| s != id);
    if on {
        if list.len() >= MAX_SHOWN_REPORTERS {
            list.retain(|s| is_reporter(&app.session, s));
        }
        if list.len() >= MAX_SHOWN_REPORTERS {
            return Err(args.err(format!("at most {MAX_SHOWN_REPORTERS} reporters can be shown")));
        }
        list.push(id.to_string());
    }
    let reply = json!({"id": id, "on": on, "shown": list});
    app.state.panels.extensions.reporters = list;
    Ok(reply)
}
