# FontCraft app icon

**Creature:** an engraved ibis in profile, FontCraft's mascot.

**Style:** an engraving-style portrait in the Crafting Apps owl template (the FilmCraft owl): a
head-and-shoulders portrait on a full-bleed field, no frame or roundel, with the body running off the tile.

**Palette** (exactly three colours):

| Colour | Hex | Used for |
|---|---|---|
| Ink | `#0b0b0c` | line work and contour |
| Paper | `#efe9dc` | the figure |
| FontCraft colour | `#6e2541` | the full-bleed field (sampled from the drawing) |

**Tile:** `viewBox="0 0 512 512"`, a rounded square with `rx=112`, clipped: the same tile as every
Crafting App. The macOS files add Apple's transparent margin (tile = 824/1024 of the canvas); Windows and
Linux use the tile edge to edge.

**Provenance:** the owner's original ArtCraft drawing (2048 px, keyed to the palette), vectorised with
craftrules `assets/logo-options/_tools/vectorize_tile.py` (field key lowered to saturation > 37 and
luminance > 30 for the dark field). `fontcraft.svg` is the canonical artwork. Licence: see `LICENSE.txt`.

## Files

| File | What |
|---|---|
| `fontcraft.svg` | master vector (traced at 2048 px) and the source for everything below |
| `fontcraft-small.svg` | lighter vector (traced at 1024 px); also `hicolor/scalable` |
| `fontcraft-1024.png` | full tile, 1024 px (docs, store listings) |
| `fontcraft-macos-512.png` | tile with Apple's margin, 512 px (runtime Dock icon on macOS) |
| `fontcraft.icns` | macOS icon set, 16 to 1024 px (`CFBundleIconFile`) |
| `fontcraft.ico` | Windows icon, 16 to 256 px (embed in the `.exe` from `build.rs`) |
| `hicolor/<size>/apps/ai.storyteller.fontcraft.png` | Linux theme icons, 16 to 512 px; use the 256 one as the runtime icon on Windows and Linux and the 128 one as the brand mark in the app bar |
| `hicolor/scalable/apps/ai.storyteller.fontcraft.svg` | Linux scalable icon |

## Regenerate

Edit or replace `fontcraft.svg` (and `fontcraft-small.svg`), then run `packaging/icons.sh` (needs `resvg`;
`iconutil` on macOS for the `.icns`). It writes every PNG, the `.ico` (via `cargo xtask ico`, the packer the
sibling apps carry in `xtask/src/ico.rs`) and the `.icns`.
