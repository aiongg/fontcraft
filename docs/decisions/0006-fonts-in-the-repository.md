# 0006: Fonts in the repository

**Status:** accepted 2026-10-07.

## Context

The sibling apps never commit font files: fonts live in a separate repository as an optional build
input, and tests build what they need in code. A font editor needs fonts for testing far more than
an illustration app does: sources in three formats, compiled fonts to import, families to compile
and compare.

## Decision

- **No compiled font is committed, anywhere.** `cargo xtask assets` fails on `.ttf`, `.otf`,
  `.woff`, `.woff2`, `.ttc`, `.pfb` and the rest.
- **Tests build fonts in code** (the model's constructors, the testkit's fixtures, later
  `write-fonts` for binaries with specific table contents).
- **Small source fixtures written for FontCraft** (a ten-line `.glyphs` exercising one rule) may be
  committed under a crate's `tests/data/`, each with a row in `ASSETS.md`.
- **Real fonts come from a pinned corpus**: openly licensed families and the reference compilers'
  own test data, named by repository and commit in `xtask/corpus.toml`, fetched into the
  git-ignored `corpus/` by `cargo xtask corpus --fetch`. Tests that need a corpus skip with a
  message when it is absent; a CI service, once there is one, will fetch it.
- **Oracle outputs are not committed either.** The harness runs the reference tools and caches
  their output by (tool version, source commit, options).

## Why

Licensing stays simple (nothing third-party in the tree), the repository stays small, and a pin
change is a reviewable one-line diff.
