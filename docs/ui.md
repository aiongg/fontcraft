# Writing the UI (`crates/ui`)

FontCraft's new UI is the crate `fontcraft-ui` (layer L6) on the craft framework. The binary
`apps/fontcraft-next` runs it. `crates/ui-egui` and `apps/fontcraft` stay until the new UI reaches
parity; then they are removed.

This page is the contract for panel writers. The worked example is the Metrics panel:
`crates/ui/src/panels/glyph_metrics.rs` and its view model in `crates/ui/src/app/models/glyph.rs`.

## Module layout

| Path | Holds |
|---|---|
| `app/mod.rs` | `FontCraftApp`: the `Session`, the `Arc<Host>`, `ViewState`, `drain()` and `project()` |
| `app/msg.rs` | `Msg` and `ViewMsg`: what views send |
| `app/catalog.rs` | the command catalog, the menu bar and each command's enabled state |
| `app/models/<area>.rs` | view models (`Tracked<T>`) and their `project` function, one file per area |
| `app/files.rs` | commands whose path comes from a file picker (`PICKED`) |
| `shell/mod.rs` | the window: title bar with menus, options bar, status bar, tool rail, body |
| `shell/dock.rs` | `PanelId`, panel titles, the default layout, `DockPanels::body` |
| `shell/commands.rs` | shell commands (view and window state): `MODULES` |
| `panels/<panel>.rs` | one panel: props, view, words, its Fixture test |
| `panels/widgets.rs` | the controls panels share: number fields, the key-or-number field, the nine-point grid |
| `canvas/` | the document area: `camera.rs`, `layers.rs` (`LAYERS`), the active-tool slot in `mod.rs` |
| `dialogs/` | dialogs on the window's `DialogHost`: `MODULES` |
| `control/` | the control channel's method table: `MODULES` |

Every list that packages extend is a slice with one line per module: `project()` arms,
`shell::commands::MODULES`, `canvas::layers::LAYERS`, `dialogs::MODULES`, `control::MODULES`.
A package adds its own file and one line; it never edits another package's file.

## The three layers

| Layer | Owns | Never touches |
|---|---|---|
| Engine (`fontcraft-engine`) | documents, commands, undo, revisions | egui, craft-ui |
| App layer (`FontCraftApp`) | the session, view state, view models, the message queue | how a panel lays out |
| Views (`fn view(&mut Props, &mut Ui)`) | nothing: they read props and push messages | `Session`, `FontCraftApp`, other panels |

`drain()` is the only code that runs `Session::execute`. It runs after the frame. A failed command
changes nothing and puts its message in the status line. `project()` may read the session and run
queries through `Session::query`, which refuses edits.

## Adding a panel

1. **PanelId.** A new panel adds, in `shell/dock.rs`, the variant to `PanelId` and `PanelId::ALL`, its id in
   `PanelId::id`, and its title in `PanelWords` and `PanelId::title`. A panel in
   `default_workspace` shows at start; the others start hidden, and
   `window.togglePanel {panel: "<id>"}` shows them.
2. **Words.** Every user-visible string goes through `craft::ui::words!`:

   ```rust
   craft::ui::words! {
       pub struct KerningWords {
           /// The pair field.
           pair: "fontcraftKerning.pair" = "Pair";
       }
   }
   let words = Env::of(ui).words::<KerningWords>();
   ```

   Keys are `fontcraft<Panel>.camelCase`: the macro wants `module.camelCase`. Text names no other
   product (`cargo xtask brands`).
3. **View model.** In `app/models/<area>.rs`, a `#[derive(PlainData, Clone, Debug, Default, PartialEq)]`
   struct and `pub fn project(model: &mut Tracked<T>, cx: &mut ProjectCx<'_>)`. Write it with
   `models::set(model, value)`, which leaves the revision alone when nothing changed. Add the field to
   `Models` and one line to `Models::project`. Projection runs only when the engine or the view
   state moves (`Stamp`).
4. **Props and view.** In `panels/<panel>.rs`:

   ```rust
   pub struct Props<'a> {
       pub model: &'a Tracked<KerningModel>,
       pub sent: &'a mut Vec<Msg>,
   }
   pub fn show(ui: &mut Ui, mut props: Props<'_>) { view(&mut props, ui); }
   fn view(props: &mut Props<'_>, ui: &mut Ui) { /* components only */ }
   ```

   Props fields are plain values, `String`, `Label`, `CommandRef`, `&T` of these, `&Tracked<T>`,
   `Vec`, `Option`, `BTreeMap` and `&mut Vec<Msg>`. No `Cell`, `RefCell`, `Mutex`, atomics, `dyn`,
   closures, `&Session` or `&FontCraftApp`.
5. **Register.** Add `pub mod <panel>;` to `panels/mod.rs`. In `Panels::body` (`shell/dock.rs`),
   add its arm (the match has no fallback, so a panel without a body does not compile):
   `PanelId::Dimensions => panels::dimensions::show(ui, Props { model: &self.models.dimensions, sent: self.sent })`.

## Shared controls

`panels/widgets.rs` holds the controls more than one panel uses. Use them instead of building a field of your own.

| Function | Shows | Returns |
|---|---|---|
| `number(ui, label, value)` | a `NumberField` | the number committed; an emptied field commits nothing |
| `clearable_number(ui, label, value)` | a `NumberField` that may be emptied | `NumberEdit::Set(n)`, or `Clear` when a field that held a value is emptied |
| `clearable(ui, field, value)` | a `NumberField` the caller configured (range, unit) | as `clearable_number` |
| `key_or_number(ui, label, &KeyedNumber, read_only)` | a field that holds a number or a metrics key, its state in the tooltip, and an update button while the key is out of sync | `KeyedOutput { commit, update }`: `commit` is the setter's `value` parameter |
| `box_point(ui, label, point)` | craft's `ReferencePoint` over one of the nine `BOX_POINTS` names | the point picked |
| `format_number(n)` | | at most two decimals, no trailing zeros, no negative zero |

A field commits on Enter, Tab or leaving it, and a field left with its text unchanged commits nothing.
`KeyedNumber` (value, key, `KeyState`, message) is plain data for a view model; `KeyedNumber::commit_value` holds the
rules for what a typed text sets: a number, a key, or for an emptied field the current number (which removes the key).

## Posting commands

| Message | Use |
|---|---|
| `Msg::run("spacing.setWidth", json!({"glyphs": [name], "layer": layer, "value": 640}))` | an engine command; one undo step |
| `Msg::Shell(CommandRef::with_params("window.togglePanel", json!({"panel": "layers"})))` | a shell command |
| `Msg::View(ViewMsg::SelectGlyph(name))` | view state that is no command |
| `Msg::Preview(Preview::Begin(label))`, `Update(command)`, `Commit`, `Cancel` | an interaction: previews on a snapshot that end as one undo step or as nothing |
| `Msg::Tool(ToolInput::Pointer(event))` | input for the canvas tools, in the edited glyph's font units |

Commands take explicit targets in their parameters: the glyph and the layer it shows, never the first master by
default (`shell::masters::current_master` and `shown_layer` give the master and layer the window shows). Post on commit (`events.committed()`), not on
every keystroke. Menu rows, keys and the tool rail fire commands through the same catalog; a
fired command becomes one of these messages in `FontCraftApp::output`.

A new shell command goes in its module's `COMMANDS` slice (`shell/window.rs` is the example). A
new module adds one `Module` line to `shell::commands::MODULES`.

## The Fixture test

Each panel has one behaviour test with `craft::ui::testing::Fixture` (the `screen-testing` feature
is on in dev-dependencies). Build the model, show the view, act as the user, and assert the
messages:

```rust
let model = Tracked::new(GlyphMetrics { glyph: Some("A".into()), layer: Some("m02".into()), width: Some(600.0), ..Default::default() });
let posted = Rc::new(RefCell::new(Vec::new()));
let seen = Rc::clone(&posted);
let mut fixture = Fixture::new(move |ui| {
    let mut sent = Vec::new();
    show(ui, Props { model: &model, sent: &mut sent });
    seen.borrow_mut().extend(sent);
});
fixture.settle();
let width = fixture.by_label("Width").expect("the Width field");
fixture.click(&width).key(Key::A, Modifiers::COMMAND).type_text("640").key(Key::Enter, Modifiers::NONE).settle();
assert_eq!(*posted.borrow(), vec![Msg::run("spacing.setWidth", json!({"glyphs": ["A"], "layer": "m02", "value": 640.0}))]);
```

Find controls by their label or role, never by position or size. App-layer behaviour (a message
to a command, a projection, the close decision) is a plain test on `FontCraftApp::new`
(`app/tests.rs`).

## The other seams

- **Canvas.** `canvas::show` runs every pass; it is never memoized, so its props may hold the active
  tool. Its document is the edited glyph's advance box from the ascender down to the descender, and
  `Camera` turns font units (y up) into screen points through the canvas view. It paints the
  projected `models.canvas` (the edited glyph in font units, resolved once per engine revision) with
  `models.canvas_tokens` (colours and sizes; the appearance settings feed them). A paint layer is one
  line in `canvas::layers::LAYERS` and a `fn(&PaintCx)` in `canvas/paint/<layer>.rs`; drawing uses
  `PaintCx::painter`. Colour glyphs are `canvas/color/`: content comes from `fontcraft_render::color::artwork`
  (and iColor and svg layer files, read once per document through `ColorFiles`) for the edited glyph, the text's
  other glyphs and Font View cells, and `color::paint::draw` draws it on any painter. The active-tool slot is `FontCraftApp::active_tool`, a `canvas::ActiveTool`
  (a craft `CanvasTool` that also hands over the commands its gestures made): craft's `ToolHost`
  runs it on the canvas's pass, after the layers. View commands and display toggles are
  `shell/view.rs`. Guides, grids, measurement labels, annotations, display aids, the auto-meter and audit markers
  are `canvas/overlays/`: `overlays::refresh` (after `drain`) reads what they need from the engine into
  `OverlayData`, which layers read as `PaintCx::overlays`; their switches are `shell/overlays.rs`. Reporter module
  drawings are read the same way (`canvas/reporters.rs`) and paint as `reportersBack` and `reportersFront`. A text note is
  typed in a field over its box (`canvas/annotation_editor.rs`).
  Placed images are `canvas/images/`: `images::refresh` (after `drain`) decodes their files through `ImageFiles`
  (large ones on the host's jobs) into `ToolView::images`, which the `images` and `imageSelection` layers paint; the
  image's press, drags and keys come before the toolbox's, and whether it is selected is `Tools::image`.
- **Edit View tabs.** The document area (`PanelId::Document`, a canvas group the host draws in
  `Panels::group_view`) shows craft's `DocumentTabs` strip over its body (`panels/tab_strip.rs`, model
  `models.tabs`): Font View first and not closable, then one tab per Edit View tab, titled by its first glyphs. The
  body is Font View while its tab shows, else the canvas. Font View is a tab of this strip and no dock panel, as in
  the reference, where Cmd+Alt+1 shows it as the first tab; tabs in the dock would have needed a `PanelId` per tab
  and would let a tab dock beside another, which the reference does not do. `ViewState::tabs`
  (`shell/tabs.rs`, `EditTabs`) holds the tabs, the showing one (`None`: Font View) and the closed ones (at most
  `MAX_CLOSED_TABS`; at most `MAX_TABS` open). The edit-view fields of `ViewState` (`glyph`, `canvas`, `text`'s text,
  layout options and features, `layers`' master, picked layers and row selection, `edit_background`,
  `preview.area_open`) and the selection in `FontCraftApp::tools` are the *live* tab's: the showing tab, or while
  Font View shows, the tab shown last. The other tabs keep their copy in `EditTab::state`, and `tabs::show` swaps
  the two, so projections, the canvas and the tools read the live fields and never look at tabs. Code that opens
  glyphs for editing calls `tabs::edit_glyphs` (the tab holding exactly those glyphs, else a new one),
  `tabs::open_glyphs` (always a new tab) or `tabs::show_glyph` (the live tab); `app.font_view_shows()` says whether
  Font View is the showing tab. A window opens with one Edit View tab. The tabs belong to the window, not to the
  active document. Cmd+Alt+2 to Cmd+Alt+9 show tabs 2 to 9 (numbered shortcuts, below).
- **Numbered shortcuts.** A shortcut that stands for a numbered series runs one command with the number as a
  parameter: Cmd+1 to Cmd+9 run `view.selectMaster {master}`, Cmd+Alt+2 to Cmd+Alt+9 run `view.showTab {index}`.
  `shell/numbered_keys.rs` lists them; `app::catalog` gives each its own catalog definition through
  `craft_ext::ui::runtime::commands::param_def`, and `FontCraftApp::output` takes fired commands through
  `fired_command`. craft's keymap routes them like every other shortcut, so a focused text field keeps the chords it
  types with, and a menu row or tooltip that names the command with the same parameters shows the chord. `ui.key`
  reads the same list. A handler of its own for unbound chords would have needed its own focus rules and could not
  show the chord on any surface.
- **Window menu and Palette.** Window › Panels has a check row per dock panel (`shell/window.rs`, `refresh_menu`,
  rebuilt when a panel opens or closes), so a closed panel can come back; craft's `WindowMenuRows` draws into a menu
  body of its own, which the data-driven menu bar has no place for. Window › Palette (`window.palette`, Cmd+Alt+P)
  hides or shows every panel docked at the end edge. A new panel needs no menu line: `PanelId::ALL` lists it.
- **Menu row state.** `app/menu_states.rs` reads, after each drain, what the live state says about menu bar rows
  beyond the engine's `enabled`: every on/off switch in `SWITCHES` gets a check mark from its state (the display
  flags, the overlay switches, the panel it shows). `Catalog::set_checks` rebuilds the bar only when a mark changed.
  A new switch command needs one line in `SWITCHES` and, if its state is not a `flag` or `switch` field, one arm in
  `checked`.
  The same module greys out an engine row whose command would be refused for its targets (`greyed`): with no glyph
  chosen (`targets_chosen`) every targeted command, with the refusal firing it gives; in Font View the commands on
  selected objects; in the edit view what `item_state::supplies` refuses. The reason is the row's tooltip and its
  `disabled_reason` in `ui.menu.list`. It is worked out again only when the documents, view state, choice or canvas
  selection moved (`Catalog::greyed_for`), never while a tool drags.
- **Edit View controls.** While an Edit View tab shows, the status bar holds the Preview Area button after the
  Features menu and, at its end, the writing direction, kerning mode and zoom controls (`panels/edit_bar.rs`), read
  from the live tab each frame (`shell::bars::edit_state`).
- **Tools.** The slot holds `canvas::tools::bridge::ToolBridge`. It posts the canvas's press, drags,
  hover and keys as `Msg::Tool`; `drain` runs `fontcraft_tools::Toolbox` on them
  (`canvas/tools/apply.rs`) and applies its answer before the next message, so each event sees what
  the one before it did. The selection lives in `FontCraftApp::tools` (the engine never stores it);
  the selection and the tool's overlays paint as layers (`canvas/tools/overlays.rs`). A menu or key
  command run without parameters gets its targets from the selection (`canvas/tools/targets.rs`).
- **Space Center preview.** `panels/space_center/preview.rs` is a second craft `Canvas` whose document is the laid-out
  text in points. Its view and its tool (`shell::space_center::drag::SpacePreview`) live in
  `FontCraftApp::space_preview` and reach the panel through the dock, as the edit view's tool does. Painting hands
  the tool each glyph's box (`PreviewFrame`); the tool's edits become messages: a selection, one command or one
  `command.batch` per drag, a nudge per arrow.
- **Dialogs.** `dialogs::run` gets the window's `DialogHost` every pass. A module opens its dialog
  when app state asks for it and takes the answer on a later pass (`dialogs/unsaved.rs` is the
  example: the close question). A form over one command's parameters is a spec instead
  (below).
- **Control channel.** `control::dispatch(app, method, params)` looks the method up in
  `control::MODULES`. A method that changes the engine runs it through `FontCraftApp::run_now`,
  the path `drain` takes, and returns the command's result; `engine.execute` uses
  `FontCraftApp::run_request`, a run of `Origin::Control`, after which no follow-up question opens. A package whose methods are not built
  yet is in `control::LATER`, which names it in the error; the package removes its line when it adds
  its module. `ui.inspect` keys come from `control::INSPECT`, `ui.set` switches from
  `control::SWITCHES`, one line per package each. `ui.click {id}` runs `ui.dialog.click`.
  `apps/fontcraft-next --control PORT` listens on the loopback port (same protocol and flags as
  `apps/fontcraft`); window input is replied to once the frames have carried it.
- **Control-driven tests.** `crate::testing::TestApp` runs the whole app headless at 1280 × 800:
  `t.ok(method, params)` returns a request's result, `t.error(…)` its error, and window input
  (`ui.click`, `ui.drag`, `ui.move`, `ui.wheel`) has run when they return.
- **File pickers.** A command listed in `app::files::PICKED` (engine or shell) that arrives without its path opens
  the host's picker; the answer posts the command again with the path.
- **User settings.** The settings live in the session's settings store. `apps/fontcraft-next` supplies an
  `app::user_store::UserStore` that reads them from the settings folder when the window opens and writes them back
  when they change (once per frame); `crates/ui` reads and writes no files itself.
- **Network and the system.** `apps/fontcraft-next` hands over `Options::fetch` (a blocking HTTPS GET for the
  Extension Manager's indexes, entry files and screenshots, run on the host's jobs by `shell/extension_downloads.rs`)
  and `Options::opener` (`FontCraftApp::opener`: links in the browser, files in the file manager). Tests set fakes; the
  browser build has neither, and what needs them says so.
- **Closing.** `app/close.rs` keeps the question asked before unsaved work closes (the close box,
  `app.quit`, File › Close): one document at a time, then the window or the document closes.
  `dialogs/unsaved.rs` shows it; `window.closePrompt` answers it from the control channel.
- **Frame hook.** `FontCraftApp::frame_hook` (`app/preview_host.rs`) runs once per frame in
  `logic`. `apps/fontcraft-next` puts External Preview's server there; `preview_host` holds the
  web server for the TrueType preview page.

## Dialog specs

A dialog over one command's parameters is a `DialogSpec` (`dialogs/spec.rs`). The framework does
the rest: `dialog.open {dialog | command, params?}`, the command's menu item, the live preview,
OK and Cancel, the window's dialog and the control channel's `ui.dialog.*`.

```rust
// dialogs/kerning_questions.rs
use super::fields::Field;
use super::spec::{DialogSpec, Fields};

pub const SPECS: &[DialogSpec] = &[
    DialogSpec::new("dialog.renameKerningGroup", "Rename Kerning Group", "kerning.renameGroup", Fields::Fixed(fields)),
];

fn fields(_: &fontcraft_engine::Session) -> Vec<Field> {
    vec![Field::text("name", "Name", 64, "the new group name")]
}
```

Then add `mod kerning_questions;` and one line to `dialogs::SPECS`. The spec's `id` is its row in
`spec/inventory`.

| Part | Choices |
|---|---|
| `Fields` | `Filter` (the filter's `filter.list` schema), `Fixed(fn(&Session) -> Vec<Field>)`, `Targeted(fn(&mut Session, &targets) -> Result<Vec<Field>, String>)` (fields that depend on what the command acts on), `Shaped(Shaped)` (fields turned into the parameters: `given`, `params`, `link`), `Tabs(&[Tab])` (one dialog per tab; values come back when a tab shows again) |
| `Mode` | `Preview` (default): every change previews on the font as it was at open; OK is one undo step; Cancel puts the font back. `.run_on_ok()`: nothing runs while open; OK runs the command once; a failure keeps the dialog open |
| Builders | `.buttons(ok, cancel)` captions (empty: the common words), `.message(line)`, `.third(Third)` (a third answer to the field `key`), `.actions(&[Action])` (queries whose `text` goes to the clipboard), `.after(After)` (a follow-up question asked when `After::command`'s result calls for it; never opened from its own menu item), `.from_window(fn(&FontCraftApp) -> Map)` (parameters the window supplies when the dialog opens without them: what the canvas shows) |

Fields (`dialogs/fields.rs`): `Field::number`, `integer`, `bool`, `lock`, `text`, `text_area`,
`choice`, `multi`, `folder`, `open_file`, `save_file`, `order`, `rows`, `select_list`, and `custom` for a kind the package
defines (`Custom { name, check, row }`: the check and the control cell; rule trees, palettes,
colours). `.in_group(heading)` puts a heading over a run of fields. A field's `key` is the
parameter's key and its id for `ui.dialog.set` and `ui.dialog.click`. Each field shows as a craft
`Param` row of a `FormDialog`; checking stays in the app layer, so a refused value changes nothing.

A field keyed `master` opens on the current master unless it was given. In a run-on-OK dialog, OK in the window with
an empty folder or save-file field asks the chooser for it first; over the control channel that is an error.

Without a `glyph` or `glyphs` in `dialog.open`'s params, the targets come from
`form::selection_targets`: the glyph the glyph panels show. The canvas and Font View selections
join there when they exist.

How it runs:

| Piece | Does |
|---|---|
| `form.rs` | `FontCraftApp::dialogs.open`: the open dialog. `open`, `set`, `confirm`, `cancel`, `click`, `key`, `type_text`, `switch_tab`, `run_action`, `inspect`. With `drain`, the only code that drives the engine: the interaction (`begin`, `preview`, `commit`, `cancel`) and OK's run, whose result decides whether the dialog stays open |
| `host.rs` | `SpecDialog` on the window's `DialogHost`: shows the app's dialog, refreshes it when its revision moves, and turns what the person does into the same `form.rs` calls |
| `after.rs` | a command the person fires with no parameters opens its dialog on the targets `FontCraftApp::with_targets` gives it (`FontCraftApp::fire`); a follow-up opens after the person's run of its command (`Origin::Person`, in `drain`) or from a run-on-OK OK |
| `control/dialogs.rs` | `ui.dialog.inspect`, `set`, `confirm`, `cancel`, `click {id}`, `key {key, shift?}`, `text {text}`. `ui.click {id}`, `ui.key` and `ui.text` hand dialog input to `click`, `key` and `text` |

In the window the dialog has the keys of craft's `FormDialog`: Enter is OK unless the focused
control takes it, Escape is Cancel, Tab commits a field and moves on. `ui.dialog.key` keeps the
field rules of the earlier UI: `Return` in a field that has the keyboard takes the typed value and
leaves the dialog open.

A dialog that is not a form over parameters (a picker, a table, a question with its own layout)
stays a `dialogs::MODULES` module with its own `StackDialog`.

## Framework pieces staged in craft-ext

Pieces craft does not have yet live in `crates/craft-ext` (library `craft_ext`), in the module craft-libs would
hold them in ([decision 0009](decisions/0009-framework-gaps-staged-here.md)). Views use them through `craft_ext`
only; each module's doc names the craft file the piece is meant for.

| Piece | Stands in for | Used in |
|---|---|---|
| `craft_ext::ui::list::StrikeRows` (`strike_rows`, `Strike`) | a struck `RowInk` on `ListRow` | Font Info › Features: a disabled entry is struck through |
| `craft_ext::ui::list::CollectionEdits` (`edit_commands`), `deliver` | a collection taking `Intent::Edit` (`CollectionEvent::Edit`) | the Features sidebar's lists take Copy, Paste and Select All; `app::clipboard::to_focused_list` hands the fired Edit command to them |
| `craft_ext::ui::menu::{MarkedEntry, EntryMark, entry_rows}` | a mixed state on `Entry` and a dash mark on `MenuItem` | the glyph menu's Tags: a tag some of the glyphs carry |
| `craft_ext::ui::menu::{MarkedEntry::danger, danger_icon}` | a danger flag on `Entry` and a danger tone on `MenuItem` | items that remove something: the canvas menu's Delete Guide and hint removals, Remove Filter, the Features sidebar's Delete |
| `craft_ext::ui::menu::PickMarked` (`pick_marked`) | `Menu::pick` over entries with those states | the Features sidebar and Font View sidebar context menus |
| `craft_ext::ui::contract::RequestMods` (`mods`, `held_mods`) | a `mods` field on `ContextMenuRequest` | Font View's glyph menu: Alt held at the click makes the swatches colour the layer |
| `craft_ext::platform::open` (`Opener`, `open_url`, `reveal`) | checked, wasm-safe forms of `craft::platform::open::{open_external, reveal}` | `FontCraftApp::opener`: Show in Finder, the Extension Manager's Website and Donate and its folder buttons; `apps/fontcraft-next` hands over `Opener::system()`, tests a fake |
| `craft_ext::ui::text::{HighlightText, Highlight}` (`highlight_text`) | a `highlight` option on `TextOptions` that the text widget's layouter turns into sections | Font Info › Features: the code editor's syntax colours and the underlined ranges of the compile's diagnostics |
| `craft_ext::ui::text::{FieldSelection, TextSelection}` (`selection`, `last_selection`, `select`) | a `selection` on `EditResponse` and a `TextField::selection` prop | Text Preview: the Selection Direction items of the text's context menu; Font Info › Features: a snippet replaces the code field's selection |
| `craft_ext::ui::text::{AsyncCompleter, CompletionQuery, CompletionAnswer}` (`kept`, `take_query`, `answer`) | a `Completer` that may answer in a later pass | Font Info › Features: the code editor's name completion, asked through `ViewMsg::FeatureCompletion` and answered by the projection with `font.features.complete` |
| `craft_ext::menu::{alternate_labels, held_alternate_labels}` | a `label` on `craft::menu::Alternate` that the bar's row shows while the alternate is selected | the menu bar: with Alt held, Tidy Up Paths reads Tidy Up Paths for All Masters (`app::menu_alternates::alternate_label`) |
| `craft_ext::ui::primitives::ShapeIcon` (`spec`) | named icons `union`, `intersect`, `slant_left`, `slant_right`, `slant_down`, `slant_up` in craft's icon set | Palette › Transformations: the slant buttons and the Union and Intersect buttons (Subtract takes craft's `squares_subtract`) |
| `craft_ext::ui::runtime::commands::{param_def, fired_command}` | an `AppCatalog` indexed by the binding key (`CommandRef::key`: id and parameters), so one id takes a definition and a default binding per parameter set | the numbered shortcuts: Cmd+1 to Cmd+9 select a master, Cmd+Alt+2 to Cmd+Alt+9 show a tab |
| `craft_ext::ui::text::{StepKeys, ArrowSteps}` (`arrow_steps`, `stepped`) | a `steps` prop on `TextField` that keeps the arrows for a stepped value, as `NumberField` does | the metric fields that take a number or a key (`panels::widgets::key_or_number`), Font Info's minor version field |

The stand-ins paint over what craft drew, so they cover the plain forms only (a row without icon, toggles or
indent; a menu row's mark column), and a screen reader hears a mixed menu row as unchecked. A danger row shows the
tone on its icon only (the label keeps its ink). Dialog buttons take craft's own danger look:
`DialogRole::Destructive` on a `FooterButton`, `Variant::DANGER` on a `Button`. An Edit command reaches
the focused list one pass after it fired.

Paste outside a text field takes no craft-ext piece: craft gives the platform's paste event only to a focused
component that lists Paste and turns it into the Cmd+V chord elsewhere, dropping its text. `app::clipboard` therefore
reads the system clipboard through craft's own `craft::platform::clipboard` (`FontCraftApp::clipboard_reader`, which
`apps/fontcraft-next` sets to `clipboard::system_text`); tests and the browser build have no reader and ask the window
for a paste event.

## Forbidden in UI code

| Do not use | Use instead |
|---|---|
| `Lazy::settles`, `Lazy::content`, `DockPanels::content`, `DockPanels::spec_version`, `AppShell::content`, `Slot::content`, `input_through`, `unsettled_through`, `lazy::unsettle` | the plain view; memoization comes later as `MemoView` |
| egui widgets and containers in views (`ui.label`, `Button`, `ScrollArea`, `Window`, `Area`, panels), key and pointer queries, `insert_temp`, `push_id` | craft components (`docs/components-index.md` in craft-libs) and `Intents` |
| `static`, `thread_local!`, `OnceLock`, `lazy_static!`, `std::fs` in `crates/ui` | state in the app layer or in the component (`Bind::kept()`) |
| raw colours and sizes (`Color32::RED`, `vec2(4.0, …)`, `style_mut`) | `Tokens` |
| `Instant::now`, `SystemTime::now` in views | time from `ShellApp::tick` |
| `on_hover_text` | the component's `tooltip` prop |
| a string literal the user reads | `words!` |

## Later: one-line memoization

When the craft `memo-views` change lands, each panel changes in one place:

```rust
#[derive(MemoProps)]
pub struct Props<'a> { pub model: &'a Tracked<GlyphMetrics>, pub sent: Outbox<'a, Msg> }
pub fn show(ui: &mut Ui, props: Props<'_>) { let _ = MemoView::new("glyphMetrics", props, view).show(ui); }
```

The view function stays as it is. That is why views read only their props.

## Dependencies and pins

- `craft` is pinned by rev in the root `Cargo.toml`. The egui and winit `[patch.crates-io]`
  entries must equal `craft::egui_fork::REV` at that rev.
- egui 0.37 (the fork) and egui 0.36 (crates.io, for `crates/ui-egui`) build side by side.
  `fontcraft-ui` names egui 0.37 directly, not through the workspace.
- The winit fork replaces winit for both. The 0.36 app builds its window events through
  `vendor/egui-winit-0.36`, egui-winit 0.36.2 with the fork's extra event.
- The `craft::egui_fork::check_lockfile` step in `cargo xtask ci` waits until `crates/ui-egui` is
  removed: until then the lockfile holds egui 0.36 from crates.io on purpose.
