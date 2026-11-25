# 0008: The Craft suite's shared libraries

**Status:** accepted 2026-10-13 (pilot approved by the owner). craft-ui is adopted (M14.9).

## Context

FontCraft follows the conventions of the Craft app suite. The suite keeps code that every app
needs in one repository, craft-libs (https://github.com/storytold/craft-libs, MIT OR
Apache-2.0, Rust edition 2024, egui 0.36, the same as FontCraft). Its crates are craft-atomic,
craft-cms, craft-print, craft-quit, craft-settings and craft-ui. The crates are not on
crates.io; apps take them from git.

FontCraft already had its own versions of some of this code. Two copies of the same code drift
apart, and fixes made in one app do not reach the other.

## Decision

1. **FontCraft uses the suite's shared libraries where one covers a need.** Each crate is a
   workspace dependency taken from git and pinned to one commit:
   `craft-<name> = { git = "https://github.com/storytold/craft-libs", rev = "<full commit hash>" }`.
   A branch or tag is never used, so a build never changes under us.
2. **Adopted now: craft-atomic.** It does the work of `fontcraft_format::write_atomic`, which
   keeps its signature, so callers did not change.
   - Replaced: the temporary file beside the target, the flush, the rename, symlink following
     and keeping the replaced file's permissions. craft-atomic does each of these too. It follows
     symlinks one link at a time with a limit of 40, so a loop is an error, not a hang. It
     creates a replacement's temporary file readable only by the user until its mode is
     restored. It also flushes the folder after the rename, refuses to replace a file the user
     cannot write, retries a rename that a virus scanner blocks on Windows, and writes error
     messages that say whether the original file was kept.
   - Kept in FontCraft: the `FormatError` result type (craft-atomic's message becomes its
     text), and the browser build's refusal. craft-atomic builds for wasm, but on wasm the
     standard library panics when asked for the process id, which craft-atomic uses to name its
     temporary file. `fontcraft-format` therefore depends on it on native targets only, and on
     wasm `write_atomic` returns an error without touching the file system.
3. **Adopted with the craft framework: craft-settings** (X.suite3), as `craft::settings` at the
   framework's pin, so it added no pin. It stores `config.toml` in FontCraft's own settings folder
   (`Location::Dir`), writes it atomically, keeps the backup and moves a damaged file aside.
   - Kept in FontCraft (`apps/fontcraft/src/settings_file.rs`): the engine's settings store and
     its checks; a size limit on what is read; putting the backup in a damaged file's place, which
     craft-settings leaves to the app; reporting a `settings.json` that cannot be imported.
   - Saving goes through the store by setting id, so two windows' changes to different settings
     both stay.
   - craft-window's placement store keeps the window's place (`window.json`). The dock's layout is
     FontCraft's own file (`ui.json`), written through `fontcraft_format::write_atomic`.
4. **Adopted with the craft framework: craft-ui** (docking, tabs, icon buttons, the widget theme)
   **and craft-quit**, through the `craft` facade at one pin (M14.9, M14.3). **craft-print** is
   pinned on its own for the PDF proof (M12.36).
5. **Not planned: craft-cms.** It manages colour profiles. FontCraft's colour fonts use sRGB, as
   the font formats define, so there is nothing for it to convert.

## Rules

- Only the UI crates (`ui`, `craft-ext`) may depend on craft-ui. egui stays above L6 (`cargo xtask layers`).
- Every pinned crate must keep the wasm build (`cargo xtask wasm`) and the never-crash rules.
  Before adopting a crate, read its source for panic paths on our inputs, as for any dependency.
- Bumping a pin is its own commit, with the full gate. Read what changed upstream first.
- A change FontCraft needs in a shared crate goes upstream to craft-libs. FontCraft does not
  keep a patched copy.

## Alternatives

| Option | Why not |
|---|---|
| Keep FontCraft's own copies | the code drifts from the suite's, and fixes are made twice |
| Copy the crates into this repository | the same drift, with more code to maintain |
| Follow a branch of craft-libs | a build can change without a commit here |

## What would change it

- craft-libs publishing to crates.io: the git pins become version requirements.
- A shared crate that stops building for wasm, takes a licence other than MIT or Apache-2.0, or
  can panic on input we pass it: it is pinned at the last good commit, or replaced by our own code.
