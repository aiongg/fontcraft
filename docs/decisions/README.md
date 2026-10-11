# Decisions

One page per decision that had real alternatives: what was decided, why, what was tried, and what
would change it. Add a page when you make such a choice; never rewrite an old one, supersede it.

| # | Decision | Status |
|---|---|---|
| [0001](0001-references.md) | Glyphs 4 is the primary editor reference; AFDKO the command-line one | accepted 2026-10-07 |
| [0002](0002-compile-pipeline.md) | Our own table builders on write-fonts; fontc as oracle and as a bootstrap backend | accepted 2026-10-07, review at M9 |
| [0003](0003-source-model.md) | One document per family, shaped like the primary's format; our own `.glyphs` reader and writer | accepted 2026-10-07 |
| [0004](0004-commands.md) | Commands take explicit targets, fail atomically, and exist once per action | accepted 2026-10-07 |
| [0005](0005-extensions.md) | No embedded Python: the command registry, macros and WebAssembly plug-ins of three kinds | accepted 2026-10-07, amended 2026-10-11 |
| [0006](0006-fonts-in-the-repository.md) | No compiled fonts in the repository; test fonts in code, real fonts from a pinned corpus | accepted 2026-10-07 |
| [0007](0007-feature-compiler-and-shaper.md) | fea-rs 1.0.0 behind the `fea` crate, with pre-checks and the guard; harfrust and unicode-bidi in `shaping` | accepted 2026-10-11 |
