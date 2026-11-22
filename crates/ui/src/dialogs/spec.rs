//! The spec API: what a package writes to get a dialog.
//!
//! A dialog is a [`DialogSpec`]: its row id in `spec/inventory`, its title, the command OK runs and where its fields
//! come from ([`Fields`]). A package lists its specs in a `pub const SPECS: &[DialogSpec]` and adds one line to
//! [`super::SPECS`]. The framework does the rest: `dialog.open`, the menu item of the command, the live preview, OK and
//! Cancel, the window's dialog and the control channel's `ui.dialog.*`.

use fontcraft_engine::Session;
use serde_json::{Map, Value};

use super::fields::Field;

/// One dialog: the command it runs and where its fields come from.
#[derive(Clone, Copy, Debug)]
pub struct DialogSpec {
    /// The dialog's row in `spec/inventory` (`dialog.filterRoundCorners`); also its id for `dialog.open {dialog}`.
    pub id: &'static str,
    /// The window title, also the label of the undo step OK makes.
    pub title: &'static str,
    /// The command OK runs.
    pub command: &'static str,
    pub fields: Fields,
    pub mode: Mode,
    /// Asked only after another command ([`After`]): never the dialog of its own command's menu item or of
    /// `dialog.open {command}`.
    pub follow_up: bool,
    /// The command whose result asks this dialog, and the parameters to open it with.
    pub after: Option<After>,
    /// The captions of the confirm and cancel buttons; an empty caption is the common OK or Cancel.
    pub buttons: [&'static str; 2],
    /// A third button that answers the dialog's question another way.
    pub third: Option<Third>,
    /// A line above the fields; empty for none.
    pub message: &'static str,
    /// The items of the actions menu at the bottom left.
    pub actions: &'static [Action],
    /// Parameters the window supplies when `dialog.open` or the menu item leaves them out (what a tab shows).
    pub window: Option<FromWindow>,
    /// Values the dialog shows but does not send, read after every change ([`Annotate`]).
    pub annotate: Option<Annotate>,
}

impl DialogSpec {
    /// A previewing dialog with OK and Cancel, no message and no actions.
    pub const fn new(id: &'static str, title: &'static str, command: &'static str, fields: Fields) -> Self {
        DialogSpec {
            id,
            title,
            command,
            fields,
            mode: Mode::Preview,
            follow_up: false,
            after: None,
            buttons: ["", ""],
            third: None,
            message: "",
            actions: &[],
            window: None,
            annotate: None,
        }
    }

    /// Nothing runs while the dialog is open; OK runs the command once ([`Mode::RunOnOk`]).
    pub const fn run_on_ok(self) -> Self {
        DialogSpec { mode: Mode::RunOnOk, ..self }
    }

    /// The captions of the confirm and cancel buttons (`Next…`, `Close`); an empty one keeps OK or Cancel.
    pub const fn buttons(self, ok: &'static str, cancel: &'static str) -> Self {
        DialogSpec { buttons: [ok, cancel], ..self }
    }

    /// A line above the fields.
    pub const fn message(self, message: &'static str) -> Self {
        DialogSpec { message, ..self }
    }

    /// A third button beside OK and Cancel.
    pub const fn third(self, third: Third) -> Self {
        DialogSpec { third: Some(third), ..self }
    }

    /// The items of the actions menu.
    pub const fn actions(self, actions: &'static [Action]) -> Self {
        DialogSpec { actions, ..self }
    }

    /// Parameters the window supplies for those the dialog is opened without.
    pub const fn from_window(self, window: FromWindow) -> Self {
        DialogSpec { window: Some(window), ..self }
    }

    /// Values read from the session after every change and shown in the dialog's own fields ([`Annotate`]).
    pub const fn annotate(self, annotate: Annotate) -> Self {
        DialogSpec { annotate: Some(annotate), ..self }
    }

    /// A question asked after `after.command` when its result calls for it; never opened from its own command.
    pub const fn after(self, after: After) -> Self {
        DialogSpec { after: Some(after), follow_up: true, ..self }
    }
}

/// How OK applies the dialog's command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Every change previews the command on the font as it was when the dialog opened; OK keeps the last preview as
    /// one undo step, Cancel puts back the very font the dialog opened on.
    Preview,
    /// Nothing runs until OK, which runs the command once (journaled, as a menu item runs it). A failure keeps the
    /// dialog open with its message. For commands that act outside the font (exporting writes files).
    RunOnOk,
}

impl Mode {
    /// The word `ui.dialog.inspect` shows.
    pub fn word(self) -> &'static str {
        match self {
            Mode::Preview => "preview",
            Mode::RunOnOk => "runOnOk",
        }
    }
}

/// A third button beside OK and Cancel, for a question with two answers that both go ahead (Keep Existing or
/// Replace). It shows while the dialog has the field `key`, which the buttons answer instead of a control of its own:
/// OK gives it `ok`, the third button gives it `value`, and either then confirms. `ui.dialog.click {id}` presses it.
/// While it shows, `ok_caption`, when given, names the OK button.
#[derive(Clone, Copy, Debug)]
pub struct Third {
    pub id: &'static str,
    pub caption: &'static str,
    pub key: &'static str,
    pub ok: &'static str,
    pub value: &'static str,
    pub ok_caption: Option<&'static str>,
}

/// An item of the actions menu: a query run with the dialog's parameters whose `text` goes to the system clipboard.
/// `ui.dialog.click {id}` runs it too.
#[derive(Clone, Copy, Debug)]
pub struct Action {
    pub id: &'static str,
    pub label: &'static str,
    pub command: &'static str,
}

/// One tab of a dialog with tabs: its id (`file.export {tab}`), its label, and its dialog.
#[derive(Clone, Copy, Debug)]
pub struct Tab {
    pub id: &'static str,
    pub label: &'static str,
    pub dialog: DialogSpec,
}

/// Parameters read from the window's view state (the edited glyph, the current master); see
/// [`DialogSpec::from_window`].
pub type FromWindow = fn(&crate::app::FontCraftApp) -> Map<String, Value>;

/// Values for fields that are no parameter (a preview of what OK will do), from the session and the command's
/// parameters; run when the dialog opens and after every change. Only a [`Fields::Shaped`] dialog has such fields: its
/// `params` leaves them out. A previewing dialog reads the font with its last preview applied.
pub type Annotate = fn(&Session, &Value) -> Map<String, Value>;

/// Builds the fields of [`Fields::Targeted`] from the session and the dialog's targets.
pub type TargetedFields = fn(&mut Session, &Map<String, Value>) -> Result<Vec<Field>, String>;

/// Field values from the parameters `dialog.open` was given, read with the session (see [`Shaped::given`]).
pub type Given = fn(&Session, &Map<String, Value>) -> Result<Map<String, Value>, String>;

/// Where a dialog's fields come from.
#[derive(Clone, Copy, Debug)]
pub enum Fields {
    /// The parameters of the filter `command`, as `filter.list` describes them.
    Filter,
    /// A list for a command that is not a filter. The session gives values that depend on the font.
    Fixed(fn(&Session) -> Vec<Field>),
    /// A list that depends on what the command acts on: the targets the dialog opens with (the given ones, else the
    /// selection). The session answers queries such as `layer.inspect`.
    Targeted(TargetedFields),
    /// Fields the dialog shapes into the command's parameters.
    Shaped(Shaped),
    /// One dialog per tab; the dialog opens on the first.
    Tabs(&'static [Tab]),
}

/// Fields that are not the command's parameters one to one: two number fields for one `{x, y}` parameter, a lock
/// that keeps two fields equal.
#[derive(Clone, Copy, Debug)]
pub struct Shaped {
    pub fields: fn(&Session) -> Vec<Field>,
    /// Field values from parameters in the command's shape (`dialog.open`'s `params`); keys that are no field's pass
    /// through (the targets). An error keeps the dialog closed.
    pub given: Given,
    /// The command's parameters from the field values.
    pub params: fn(&Map<String, Value>) -> Map<String, Value>,
    /// Brings linked fields in step after the field named by the `&str` took a value (an empty name when the dialog
    /// opens).
    pub link: fn(&mut Map<String, Value>, &str),
}

/// The parameters of a follow-up dialog from the request and the result of the command that asks it; `None` when the
/// result asks nothing.
pub type FollowUp = fn(&Value, &Value) -> Option<Map<String, Value>>;

/// The command that asks a follow-up dialog.
#[derive(Clone, Copy, Debug)]
pub struct After {
    pub command: &'static str,
    pub params: FollowUp,
}
