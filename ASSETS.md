# Assets

Every icon, image, font source, test fixture and other non-code file that ships in this repository
has a row here. `cargo xtask assets` (part of `cargo xtask ci`) fails on a file under `assets/` or
under any `tests/data/` folder that has no row, and on any compiled font anywhere in the tree.

An asset must be one of: original work made for FontCraft by a contributor (licensed MIT OR
Apache-2.0), OSI open source, public domain or CC0, or Creative Commons with redistribution
allowed. Never anything taken from a reference app: no icons, glyph artwork, sample text, presets,
example fonts or screenshots. If where a file came from is unclear, it does not go in.

Rules for fonts (the full text is in [docs/clean-room.md](docs/clean-room.md)):

- **Compiled fonts are never committed.** Tests build the fonts they need in code. Real fonts come
  from the pinned corpus (`cargo xtask corpus --fetch`, git-ignored).
- **Font sources** (`.glyphs`, `.ufo`, `.designspace`…) are committed only as small fixtures
  written for FontCraft, under a `tests/data/` folder, with a row below.
- Fonts the app itself draws its interface with, when it needs more than the toolkit's defaults,
  come from the shared craft-fonts repository as an optional build input, as in the sibling apps.

| Path | What | Author | Source | Licence |
|---|---|---|---|---|
| *(none yet)* | | | | |
