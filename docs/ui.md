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

1. **PanelId.** Every panel of `crates/ui-egui` already has a `PanelId` and a title, and shows a
   stub. A new panel adds, in `shell/dock.rs`, the variant to `PanelId` and `PanelId::ALL`, its id in
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
   replace the stub arm with
   `PanelId::Kerning => panels::kerning::show(ui, Props { model: &self.models.kerning, sent: self.sent })`.

## Posting commands

| Message | Use |
|---|---|
| `Msg::run("spacing.setWidth", json!({"glyphs": [name], "value": 640}))` | an engine command; one undo step |
| `Msg::Shell(CommandRef::with_params("window.togglePanel", json!({"panel": "layers"})))` | a shell command |
| `Msg::View(ViewMsg::SelectGlyph(name))` | view state that is no command |

Commands take explicit targets in their parameters. Post on commit (`events.committed()`), not on
every keystroke. Menu rows, keys and the tool rail fire commands through the same catalog; a
fired command becomes one of these messages in `FontCraftApp::output`.

A new shell command goes in its module's `COMMANDS` slice (`shell/window.rs` is the example). A
new module adds one `Module` line to `shell::commands::MODULES`.

## The Fixture test

Each panel has one behaviour test with `craft::ui::testing::Fixture` (the `screen-testing` feature
is on in dev-dependencies). Build the model, show the view, act as the user, and assert the
messages:

```rust
let model = Tracked::new(GlyphMetrics { glyph: Some("A".into()), width: Some(600.0), ..Default::default() });
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
assert_eq!(*posted.borrow(), vec![Msg::run("spacing.setWidth", json!({"glyphs": ["A"], "value": 640.0}))]);
```

Find controls by their label or role, never by position or size. App-layer behaviour (a message
to a command, a projection, the close decision) is a plain test on `FontCraftApp::new`
(`app/tests.rs`).

## The other seams

- **Canvas.** `canvas::show` runs every pass. `Camera` turns font units (y up) into screen points.
  A paint layer is one line in `canvas::layers::LAYERS` and a `fn(&PaintCx)` in its own file. The
  active tool runs in `canvas::active_tool`.
- **Dialogs.** `dialogs::run` gets the window's `DialogHost` every pass. A module opens its dialog
  when app state asks for it and takes the answer on a later pass (`dialogs/unsaved.rs` is the
  example: the close question).
- **Control channel.** `control::dispatch(app, method, params)` looks the method up in
  `control::MODULES`. A method that changes the engine runs it through `FontCraftApp::run_now`,
  the path `drain` takes, and returns the command's result. A package whose methods are not built
  yet is in `control::LATER`, which names it in the error; the package removes its line when it adds
  its module. `ui.inspect` keys come from `control::INSPECT`, `ui.set` switches from
  `control::SWITCHES`, one line per package each. `ui.click {id}` runs `ui.dialog.click`.
  `apps/fontcraft-next --control PORT` listens on the loopback port (same protocol and flags as
  `apps/fontcraft`); window input is replied to once the frames have carried it.
- **Control-driven tests.** `crate::testing::TestApp` runs the whole app headless at 1280 × 800:
  `t.ok(method, params)` returns a request's result, `t.error(…)` its error, and window input
  (`ui.click`, `ui.drag`, `ui.move`, `ui.wheel`) has run when they return.
- **File pickers.** A command listed in `app::files::PICKED` that arrives without its path opens
  the host's picker; the answer posts the command again with the path.

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
