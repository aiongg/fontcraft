# 0009: Framework gaps are staged in FontCraft, shaped for craft-libs

**Status:** accepted 2026-11-20 (owner's direction). Review when FontCraft moves into the storytold
organisation.

## Context

The window (`crates/ui`) is built on the craft framework from craft-libs, pinned to one commit
(`4bd0107`). The port keeps finding pieces the framework does not have: a strikethrough list row, edit
commands (copy, paste) reaching a focused list, a highlight hook and caret access on text fields, an
Up/Down stepper on number fields, open-URL and reveal-in-file-manager calls,
alternate menu rows, an app-level paste event, modifiers on context-menu requests, a mixed mark on menu
entries, a danger tone, and some icons.

craft-libs is not taking outside changes at this stage, and FontCraft cannot wait for them.

## Decision

1. **One staging crate, `crates/craft-ext`** (package `fontcraft-craft-ext`, library `craft_ext`),
   holds every framework piece FontCraft needs and craft lacks. It is L6 (it depends on egui).
2. **Nothing FontCraft-specific goes in it.** It depends only on craft, egui and general-purpose
   crates craft already uses; never on a `fontcraft-*` crate. Words, tokens and tests follow craft's
   conventions, so the code reads as if it were part of craft.
3. **Its modules follow craft's layout.** `craft_ext::ui::widgets::list`, `craft_ext::platform::open`
   and so on, mirroring where the piece belongs in craft-libs. Each module's doc names the craft crate
   and file it is meant for.
4. **Extend from outside where craft allows it:** extension traits on craft types, wrappers that
   compose craft components, functions over craft's public API. Where craft's internals block that, the
   piece is a small component of its own, built the way craft builds its components (Props, Bind,
   events, tokens), not a copy of a craft component with one change.
5. **`crates/ui` uses the staged pieces through `craft_ext`**, never by reaching into craft internals.
   When a piece moves into craft-libs, the call sites change their import path and nothing else.
6. **No repin for these pieces.** The craft pin moves only when FontCraft needs a change craft itself
   has made.

## Consequences

- Moving into craft-libs later is mostly moving files: each module already sits at its target path
  and depends on nothing of FontCraft's.
- Some pieces exist twice for a while (a staged list row beside craft's). The module doc says which
  craft piece it extends or stands in for.
- Pieces that need changes deep inside craft (for example, the shell delivering a paste event) may need
  a narrower stand-in here; the module doc records what the full version in craft would do.

## The app runner

`craft_ext::ui::shell::app::run` stands in for craft's `app::run`. craft opens the root window at its saved
place before any monitor is known, its `SystemWindows` lists no monitors, and its runner keeps eframe's window to itself;
so a window saved on an unplugged monitor opened out of reach. The staged runner is built on craft's public `embed`
hooks. It lists the monitors of eframe's window for the window host (which places floating dock windows by them too) and
moves the root window back onto one. In craft, `SystemWindows` would list them and `app::run` would place the window
again.

## Craft bugs FontCraft cannot work around

Some defects sit inside craft components with no public hook before them, so a staged piece cannot
fix them. They wait for craft-libs.

| Bug | Where in craft | Effect in FontCraft | Test |
|---|---|---|---|
| A floating dock window opens as a palette OS window (`WindowLevel::Palette`, always on top while the app is active), and an app-modal dialog is a layer of the main window: the palette hides the dialog, and craft lowers no palette while the modal is open | `ui-contract/src/runtime/windows/spec.rs` (`WindowLevel::for_egui`), `ui-panels/src/docking/native.rs` | A form dialog opened under Font Info or another floating panel is hidden behind it. `craft_ext::ui::runtime::windows::keep_modal_on_top` lowers the other windows to the normal level and focuses the main window while a modal is open, and restores them after; without a window manager (Xvfb) the stacking does not change. | `craft_ext::ui::runtime::windows::tests`; `fontcraft_ui::testing::native::tests` (a floated Font Info under Add Glyphs) |
| eframe paints an immediate viewport without its screenshot requests: `render_immediate_viewport` passes no capture to the painter, and only the passes of the root and of deferred viewports hand `ViewportCommand::Screenshot` callbacks over, so a capture of a floating dock window (an immediate viewport) is never answered | the craft fork of egui: `eframe/src/native/wgpu_integration.rs` (`render_immediate_viewport`) | `ui.screenshot` captures the main window only; floating windows are missing from it. A capture queued for a floating window's viewport through `WindowHost::screenshot` times out. | none: needs a fix in the fork |
| The app runner gives the app neither eframe's `CreationContext` nor its `Frame`, and eframe only logs the wgpu adapter it picked | `ui-shell/src/shell/app.rs` (`create`, `Driver`) | About FontCraft names wgpu but says the adapter in use is not reported (`dialogs::about::backend_text`); `FontCraftApp::graphics` stays `None` | `dialogs::about::tests::the_graphics_line_falls_back_when_no_adapter_is_reported` |
| A text part that inserts Tab keeps only Tab from the focus service: `keys_of` chooses `KeepKeys::Tab` in place of `BlockArrows` (multi-line) or the base keys, so the arrows, with Shift or without, move focus instead of the caret | `ui-contract/src/primitives/text_adapter.rs` (`keys_of`) | The macro editor of the Command Console does not insert Tab, so its arrows move the caret and Shift with them selects; Tab leaves the field | `panels::command_console::tests::shift_arrow_over_the_control_channel_selects_in_the_macro_editor` fails with `tab_inserts(true)` |
| A multi-line text part centres its text vertically: the egui `TextEdit` under it gets `min_size` of the part's rect and `Align2::LEFT_CENTER` (`RIGHT_CENTER` right to left) whether it is multi-line or not, so egui places a galley shorter than the rect in its middle. `TextField::lines` has no option that changes the alignment, and the edit is internal to craft | `ui-contract/src/runtime/egui/text_edit.rs` (`show`, the `.align(…)` of the edit) | The macro editor of the Command Console (ten lines) draws a short macro in the middle of its box, not at the top; a longer text fills it from the top | none: the field's own alignment is not reachable from FontCraft |
| A wrapping `Row` keeps a checkbox on the line it starts when only its least width fits there: the label then wraps in the room left, one or two letters a line, and the line grows as tall as that column | `ui-contract/src/layout/body.rs` (the wrapped row's placement; `Checkbox` has no option that keeps its label on one line) | The find bar of the feature code editor ended its one wrapping row with Regular Expression at the panel's default width and pushed the code out of the window. The bar is three rows, each narrower than the pane at that width; a narrower window still squeezes the last checkbox | `panels::font_info::tabs_tests::the_find_bar_and_the_code_field_fit_the_panels_default_size` (the layout that avoids it) |
| The expression reader recurses once per `(` and per sign with no depth limit, and `NumberField` sets no length limit on its draft (a `TextField` holds 1,024 characters) | `behavior/src/quantity.rs` (`Parse::factor`), `ui-widgets/src/text/number.rs` | Pasting about 100,000 `(` into any panel number field overflows the stack and aborts the app. Dialog number fields read through `widgets::parse_number`, which caps length and depth first. | `panels::widgets::tests::craft_number_fields_survive_a_paste_of_deep_parentheses` (ignored) |
