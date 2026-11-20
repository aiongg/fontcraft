# 0009: Framework gaps are staged in FontCraft, shaped for craft-libs

**Status:** accepted 2026-11-20 (owner's direction). Review when FontCraft moves into the storytold
organisation.

## Context

The new window (`crates/ui`) is built on the craft framework from craft-libs, pinned to one commit
(`4bd0107`). The port keeps finding pieces the framework does not have: a strikethrough list row, edit
commands (copy, paste) reaching a focused list, a highlight hook and caret access on text fields, an
Up/Down stepper on number fields, a press-and-hold flyout, open-URL and reveal-in-file-manager calls,
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
