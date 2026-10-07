# 0001: The reference apps

**Status:** accepted 2026-10-07. The owner left the choice to the planning pass, on one condition: any app
may be a reference, proprietary ones included, as long as no code is copied and the clean-room
rules hold.

## Context

A reimplementation needs one reference whose menus, tools and shortcuts it follows, because
command ids mirror the reference's menu structure and users arrive with one app's habits. The
candidates were the leading commercial Mac editor (Glyphs), the cross-platform commercial editor
(FontLab), the UFO-native Mac editor (RoboFont) and the open-source editor the owner first named
(FontForge). For the command line the candidates were AFDKO and fonttools.

## Decision

- **Glyphs 4** is the primary reference for the editor.
- **AFDKO** is the primary reference for the command-line tools and the feature-file language;
  fonttools, fontmake and fontc are oracles and further tool references.
- **FontLab 8** is the secondary editor reference for what the primary lacks or leaves to
  plug-ins, and for Windows conventions. RoboFont is a minor one.
- **FontForge and Fontra** are consulted through public user documentation only (GPL).

## Why

- The primary's **whole handbook is public** and covers the app in 18 chapters, which fits the
  rule that behaviour comes from public documentation.
- Its **source format is documented** (three versions, with schemas, under Apache-2.0) and is what
  much of the open font toolchain builds from, so interchange and oracles come with it.
- Its single window with tabs and a palette sidebar is close to the sibling apps' layout.
- FontForge's interface is dated and its code is copyleft, so it cannot be the model for either.
- AFDKO is permissively licensed, so its code can be read, and it is the reference implementation
  of the feature-file language.

## Costs

- The primary runs only on macOS. Windows and Linux conventions are ours to define.
- Observation needs a Mac with a licence. **Nothing has been observed yet**: every row from the
  primary has `status = "handbook"`. Where the handbook is silent, rows say so and
  `docs/spec/*.md` list the open questions (about 440 across the notes).
- The product name is also the generic word for what a font is made of, so the vendor-names check
  matches phrases, not the bare word (`xtask/src/brands.rs`).

## What would change it

If the owner has no way to observe the primary and does have FontLab, the *observable* app for
interaction details becomes FontLab, while ids and layout stay with the primary.
