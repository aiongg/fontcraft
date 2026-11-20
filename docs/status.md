# Status: the hand-over note

The short note the next session reads first. Keep it current; sessions end without warning.

_Updated: 2026-11-20, after the seventeenth and eighteenth batches: the second window's ports UI.30a to UI.36d (UI.30a, UI.30b, UI.24b, UI.27b, UI.31 to UI.34, UI.20b, UI.35a to UI.35d, UI.36a, UI.36b, UI.36d) and `crates/craft-ext` (decision 0009), merged on `master` as of `f99ee43`; the gate is green (5,382 tests). The sixteenth batch's entries below stay as written._

## State of the tree

- **Where the seventeenth and eighteenth batches stand.** Every branch is merged on `master` and pushed; the gate (`cargo xtask ci`) is green
  on `f99ee43` with 5,382 tests. The writers ran no builds; one integrator built several branches at once (X.integ81
  to X.integ97). The new window (`crates/ui`) gained:
  - Font Info field warnings, the Number Values editor and the Features sidebar menu (UI.30a); multi-glyph Info box
    fields, `.ufoz` saving with format warnings, reply notices in the status line, the Glyph as Image and Overlay
    Fonts dialogs (UI.30b).
  - The Shape Order panel and drag reordering in order fields (UI.24b).
  - Colour glyphs drawn through the render artwork, iColor and svg layers, colour in text and Font View cells, a
    colour picker and a gradient stop bar (UI.27b); Text Preview colour from the compiled colour font of the shown
    master or instance (UI.35a).
  - The shown layer and master used by Font View, menus, paste and panels, a person's copy reaching the system
    clipboard only, and the instruction badge menu (UI.31); the Metrics panel, the hinting waterfall, the metrics
    beam and the picker fallbacks read the current master and shown layer (UI.36d).
  - Info box kern fields and locks, the Modules panel and View › Audit › Live Audit (UI.32).
  - Space Center drags and arrow nudges, Table Viewer file pickers, the TrueType preview address dialog, user
    settings kept between runs, custom glyph data read from the settings folder (UI.33).
  - The Extension Manager window, Project panel copy and paste, Add Custom Parameter and reporter drawings (UI.20b);
    the Compare Fonts in List panel (UI.35b); reporter switches kept in `view.shownReporters` (UI.35c); the
    Instructor section at the end of the canvas menu while that tool is active (UI.35d).
  - Alt in the Font View glyph menu colours the layer (UI.36b); menu rows that remove something show a danger tone
    (UI.36a).

  `crates/craft-ext` (decision 0009) stages the framework pieces craft lacks, in craft-libs' layout. It has: a struck
  list row, edit commands (copy, paste) reaching a focused list, a mixed mark on menu entries, a danger tone on menu
  rows (icon only; the label ink needs a change in craft), and modifiers on context-menu requests. The new window's
  ports have headless tests only; none has been tried in the running window.
- **Where the sixteenth batch stands.** Every branch is merged on `master` (over 250 commits on top of `9f8f3e5`,
  the last pushed commit, X.docs19). Nothing of this batch is pushed. The tests honour `TMPDIR`; point it at a disk
  with room when the system's temporary directory is full.

  | Step | State |
  |---|---|
  | Workspace check and clippy with the no-panic lints | clean (X.integ73 to X.integ80) |
  | Command ids blessed, generated lists (`cli-options`, guesses, observe, parity), fmt | done (X.integ78 to X.integ80) |
  | First full test run (cargo nextest) | 5,304 run, 5,271 passed, 33 failed |
  | The 24 failures outside the window crates (tables, compile, io-ufo, proof, engine, cli) | fixed by X.fix86; a filtered run of them passes |
  | The 9 failures in `crates/ui`, `apps/fontcraft` and `apps/fontcraft-next` | fixed by X.fix87 (three were code bugs: the Instructor's Delete, dialogs targeting `.notdef`, a zero-height Preview Area strip) |
  | Review pass | 7 findings, fixed by X.fix88 (CFF2 region cap, revisions never reused after a rollback, CLI `@` values, palette round trip) and X.fix89 (`ui.menu.invoke` plays as a click, dialogs target the view in front, Preview Area height) |
  | The gate, `cargo xtask ci` | green: 5,310 tests pass; wasm after X.fix90 (craft's OS clipboard is native-only in `crates/ui`). `pathops` `random_paths_through_every_operation_never_panic` can miss its one-second limit on a loaded machine |

  The writers ran no builds and most branches were merged without one, so most of this code has been run only by its
  tests, not in the app. Built and run on Linux x86-64 and on macOS 15 (Apple Silicon:
  both windows build with no changes and open; a font opens). Windows was not compiled. Still to check on the Mac:
  the quit prompt with unsaved work, the Open chooser and drop, X.pdf3's pasteboard PDF.
- **Two windows.** `apps/fontcraft` on `crates/ui-egui` is still the app. The new window, `apps/fontcraft-next` on
  `crates/ui`, is built on the Craft suite's craft-next framework (pinned at `4bd0107`; egui 0.37 from the suite's
  fork beside egui 0.36 for the old window; a vendored egui-winit 0.36 under `vendor/` keeps the old window building
  against the shared winit fork). It sits beside the egui window until it reaches parity; then the egui window goes (the ports left are in "Where to start").
  Its ports have headless tests only, and none has been tried in the running window. Conventions for writing it are
  in `docs/ui.md`. Leave `crates/ui-egui` alone except for compile fixes.
- 960 of the 1,039 inventory commands exist (`docs/parity.md`). Per milestone: M1 (11), M2 (5), M3 (185), M5 (100),
  M6 (47), M10 (60) and M11 (42) are all live; M4 62 of 68, M7 66 of 69, M8 44 of 45, M9 64 of 66, M12 99 of 103,
  M13 167 of 197, M14 3 of 32. The inventory has 3,786 rows. Feature parity is about 38% (ROADMAP.md › Parity
  estimate; 38.4, was 37.2), graded on the egui window and on engine and CLI behaviour; the new window counts once it
  replaces the old one. 3,081 guesses on 1,606 rows are listed in `docs/guesses.md`; `docs/observe.md` lists 1,500 rows. CLI option coverage: 672 of
  770 reference options present (`docs/cli-options.md`).
- Packages built (done of total, counted from commit subjects; the totals include the depth packages that carry a
  new number): M6 21 of 26, M7 21 of 24, M8 27 of 36, M9 30 of 40, M10 35 of 42, M11 20 of 22, M12 77 of 87, M13 63
  of 83; 294 of 360 in all. Depth packages (`docs/plans/depth.md`): 55 of 167 (wave 1 and 45 writers of waves 2 to
  11, listed in ROADMAP.md › Where we are); no builder of waves 2 to 13. M3.22, M3.23, M4.18, M4.19, M5.29, M7.22 and
  M9.32 have editor parts in the new window only and are not counted. The lettered follow-ups (M10.37b, M10.37c,
  M13.65b and those of earlier batches) are not counted.
- **Baselines** (from the fifteenth batch; the sixteenth ran none of them, and a later change that moves one of these numbers needs a reason):

  | Measure | Result |
  |---|---|
  | Static oracle against fontmake, TrueType | 2,376 compared, 2,218 the same, 139 coverage-equal, 19 different; 39 of 354 sources differ (unchanged). `GSUB` 25 of 26, `GPOS` 82 of 88, `GDEF` 47 of 55 equal. M8.8 removed 5 allow-list entries |
  | Static oracle against fontmake, CFF | 2,377 compared, 1,854 the same, 511 coverage-equal, 12 different; 38 of 354 sources differ (unchanged). Exported CFF fonts are 0.957 of the size of fontmake's unsubroutinised builds |
  | `COLR` against fontc | 10 identical, 7 allowed, 0 different |
  | `COLR` against fontmake | 11 identical, 7 allowed, 0 different |
  | Variable `COLR` against fontmake (M11.19) | COLRv1-var, COLRv0-var and COLRv0-2masters-brace equal (compared with ttx; one allow-listed layer-glyph naming difference); fontc fails on COLRv0-2masters-brace itself |
  | Contextual anchors against fontmake (M8.21) | identical on the probe source; `GPOS` lookup order matches (`abvm` and `blwm` mark-to-mark lookups last); fontc ignores contextual anchors |
  | Static instances against `fontmake -i` (X.ds1) | 180 the same, 30 different, 0 not built, 24 failed in fontmake itself (unchanged) |
  | Designspace `<rules>` against fontmake | 3 sources: 2 the same, 1 different (CustomFeatures: a rule that feeds several feature tags) |
  | Instances from sources against fontmake's UFO instances (M12.22, M12.22b, M12.22c) | 39 instances: 0 differ in a glyph (was 6, `iacute`), 0 in font info (was 8, then 1 before X.inst4) |
  | Layout-only instances against fontTools `interpolate_layout` (M12.22c) | 6 the same, 2 refused by both |
  | Round trips (`cargo xtask roundtrip`) | `.glyphs` native 122/1/118/4 (the 118 differences are fontc's hand-written test data; all 18 files the reference app wrote pass); designspace native 59/0/0/53; UFO model 125/0/2/13; `.glyphspackage` 22/0/7/0 |
  | `corpus_import` (hinted import) | 27 CFF and 48 TrueType fonts pass |
  | TTX against ttx | CFF 253 equal, 0 different; layout 700/0; other tables 143/0 |
  | qu2cu | 3,192 equal, 0 different |
  | CFF subroutiniser (M12.25) | 233 corpus fonts, 0 outline differences, 0 fontTools warnings, total size 0.785 of cffsubr's; 3 small test fonts over by more than 2% (Lobster.subset 1.020, ttLib I.otf 1.167, TestOTF 1.096). Measure: `python -I crates/cff/crosscheck/subr_check.py OUT` (it needs tx). Run again after the full-CJK subroutiniser (M12.25b) and after the thirteenth batch (ratio 0.7848): unchanged |
  | CFF subroutiniser on a full CJK font (M12.25b) | Source Han Serif TC Bold subroutinises in 136 s with a 692 MB peak (budget 4 GiB, 512 MiB on wasm), to 0.883 of the unsubroutinised size; before, charstrings above 8.4 million tokens were refused |
  | Convert of a full CJK font (X.fix42) | Source Han Serif TC Bold to TrueType in 7.6 s, 354 MB peak; charstring memory peak 141.5 MiB (limit 1 GiB, 512 MiB on wasm), 10.56 million points (limit 2^26, about 67.1 million) |
  | Static instances against fontTools `varLib.instancer` (M12.20) | 26 cases on 8 corpus variable TrueType fonts: all tables identical except `GPOS` in 18 (subtable layout kept from the input) and `maxp` in 2 (not recalculated) |
  | Transform against fontTools `scaleUpem` (M12.24) | `glyf` identical in 12 of 12; `cvt ` differs in 4 (expected); `maxp` and `head` flags differ (no `maxp` recalculation); `avar2` item variation store left unscaled; `.null` kept in `post` |
  | Units per em on CFF and CFF2 against `scaleUpem` (M12.24b, X.fix52) | 21 fonts: the only differences are the `OS/2` first and last character index (fontTools recomputes them at save), CFF2 blended Private values (written down), and LinLibertine_RBI's empty-coverage `GPOS` pair subtable fitted to its used classes (fontTools keeps the stored counts; before this batch the font could not be written back) |
  | Merge against fontTools merge (M12.23) | 8 pairs: `OS/2`, `cmap`, `name`, `hhea`, `loca` identical; `glyf` and `head` differ in 5 (fontTools recalculates bounds); duplicate names `#n` against `.n` in 2 |
  | TTX reader against fontTools `importXML` and save (M12.8) | 16 fonts: `maxp`, `OS/2`, `name`, `cmap`, `post`, `hmtx`, `CFF `, `glyf`, `loca`, `cvt `, `gasp`, `prep`, `fpgm` identical; `head` and `hhea` differ on CFF fonts (fontTools recompiles the font bounding box); X.ttx4 recalculates them: see the row below |
  | CFF subroutines against cffsubr (M12.26) | 14 fonts: 0.974 of cffsubr's size |
  | Partial instances against fontTools `varLib.instancer` (M12.21) | 38 cases on corpus variable TrueType fonts: 9 equal, 5 refused (`avar` 2), 3 different (each equals fontTools' own full instance), 21 failed inside fontTools |
  | Partial instances of `avar` version 2 and `VARC` fonts (M12.21b) | Amstelvar: 5 of 5 equal; `VARC` has no oracle (fontTools 4.66.1 cannot read the corpus `VARC` fonts) |
  | `variations` tools against fontTools (M12.34) | `model` agrees in 104 of 112 cases; `build-hvar` 8 of 8; `plan-avar` on NotoSans-VF-cubic differs by 0.004; `plot` refuses varc-6868 and Amstelvar-avar2 |
  | TTX reader, layout and variation tables (M12.9) | 39 corpus fonts: all tags identical after reading the TTX back; 239 of 240 CFF fonts identical |
  | TTX writers, Apple, bitmap and `COLR` version 1 tables (M12.7b) | 31 of 34 cases equal to ttx; the other 3 are the `prop` dump (written raw) and `cmap` format 2 |
  | `maxp`, `head` and `post` after instance, transform and merge (X.tables1) | equal to fontTools' on the 12 comparison cases; `maxSizeOfInstructions` keeps its stored value unless glyph programs were removed |
  | FontDame conversion (M12.13b) | 24 of 28 equal; VOLT equal |
  | Autohinter against ttfautohint (M10.21b), on 7 of 325 sources (3 distinct fonts, no Inter) | x-height rows 92.9% (target 95%), cap-height rows 100%, zone agreement 98.29% (target 99%), size 78.8% of ttfautohint's |
  | Detect Composites on Inter (M13.15b, M13.15c, X.fix66) | 78.1% of 2,017 composites rebuilt from the source's bases (63.0% before M13.15c; target 95%), 241 from other bases, 200 missed; 0.4 to 0.7 s (target 2 s); the per-edit alignment pass 0.24 ms (target 2 ms) |
  | Colour drawing in Edit View (M11.14), measure profile | steady frame 0.53 ms, scrolling frame 0.53 ms; first frame 27.2 ms before the paint budget |
  | Text Preview (M8.20, M8.20b), release | preview font build 169 ms; layout of 100,000 characters with bidi 30.9 ms, the bidi analysis within its 30 ms target; a shaped 30-line page 0.38 ms (target 8 ms); layout on shaped widths with an empty cache 29.0 ms; a million characters 286 ms, 319 ms on shaped widths at a load of 17 (limit 500 ms). Re-measured in this batch at a load of 19 to 20, with vertical text: a million characters 290 to 325 ms (323 to 404 ms on shaped widths), a page 0.377 ms, a colour frame 0.419 ms, the preview font build 276 to 383 ms, and 170 to 173 ms at a load of 8 |
  | Feature compile shared with the previews (X.edit1) | an edit to compile state and preview fonts on Inter: 200 ms (was 374 ms) |
  | Auto-kerning sign agreement (X.tune1) | 85.7% to 99.2% in every master (target 80%) |
  | Auto-spacing median error (X.tune1) | Oswald 5% to 12.5%, Inter 30% to 75% (target 10%) |
  | Space Center, 200-glyph frame (M13.23b) | 1.30 ms plain, 1.44 ms with every display option, in release |
  | Subset against pyftsubset, layout (M12.18) | 909 runs: `GSUB`, `GPOS` and `GDEF` equal wherever both tools run |
  | Subset against pyftsubset, variation and colour tables (M12.19) | 33 of 33 equal |
  | Subset against pyftsubset, CFF and CFF2 (M12.19b) | 1,608 runs: 536 of 536 equal in each of prune, desubroutinize and no-hinting (unchanged) |
  | Subset of `VARC` against pyftsubset (M12.19c) | 14 of 14 runs equal; a 15th hangs inside fontTools (its `VARC` closure loops on a self-referencing component) |
  | Subset of compressed `SVG ` and composite bitmaps against pyftsubset (M12.19d) | compressed `SVG ` 6 of 6 equal; `CBDT` 2 equal and 4 different, as expected: composite bitmaps keep their components in the output numbering (a guess) |
  | Subset options against pyftsubset (M12.18b) | every option compared gives the same output; `--canonical-order` matches with the flag; pyftsubset fails `--recalc-max-context` on `OS/2` tables older than version 2 |
  | Convert against otf2ttf, tx and fontTools (X.conv2, M12.30) | CFF to TrueType 43,967 identical, 0 off by one, 26 different (was 12,449, 446, 31,098); TrueType to CFF 36,466 the same, 203 different (glyph names of `post` format 3 fonts, written down); CFF to CFF2 43,963 equal, 30 different (seac, written down); CFF2 to CFF 43,993 equal, 0 different; WOFF2 331 equal, 0 different (unchanged) |
  | TTX `mort` and `morx` against ttx (M12.7d) | 100 of 100 dumps and reads equal, on fonts built with fontTools (`crates/tables/crosscheck/morph.py --build 50`; the corpus has none) |
  | UFO colour layers against glyphsLib (M11.5c) | 10 of 10 round trips equal (`crates/io-ufo/crosscheck/color_layers.py`) |
  | `optimize` against fontTools (X.opt1, X.opt2) | `GPOS` compaction at level 5: 117 equal, 1 fontTools crash (LinLibertine_RBI); `gvar` IUP 9 equal, 0 different; implied on-curve points 92 and 0; variation store 3 and 0; `COLR` layer reuse 5 and 0 (`crates/tables/crosscheck/optimize.py`) |
  | Project compile against makeotf (M12.40, M12.40b to M12.40d) | 7 of 10 corpus UFOs give a dump equal to makeotf's (makeotf fails on 114 others); the other 3 differ only in written-down makeotf faults: two negative advance widths that makeotf wraps (to 65503 and 65458, Oswald-Bold and OswaldMonoline) and a move makeotf loses in `threequarters` (Oswald-Regular-Round) |
  | TTX read bounds against fontTools (X.ttx4) | 253 of 253 corpus CFF fonts equal, with the default tags and with `head,hhea,hmtx` |
  | CID-keyed CFF against makeotf (M12.41) | 13 of 13 keys equal, with the `dev` and `noMac` options; unchanged after M12.40d |
  | Type 1, bare CFF and TTX input against tx (M12.30b) | TTX input: 3,172 glyphs the same, 0 different, 3 fonts failed |
  | rotatefont matrices (M12.24c, X.fix55) | 5 corpus CFF fonts, 6 matrices, 21,774 glyphs: 0 outline, 0 stem, 0 width and 0 Private DICT differences (before X.fix55: LinLibertine_RBI 615 outline and 490 stem differences, Markow 164 stem differences when scaled). The `-rt` half-angle quirk is kept as a written-down difference |
  | Variable merge against fontTools' instancer on the merged halves (M12.23c) | the same at 4 locations for the Oswald halves, Oswald with itself, I.ttf and Amstelvar; NotoSans needs a name table on the fontTools side; InterVariable's `GPOS` differs only by 2 Extension lookups that fontTools' serializer adds when it saves the instanced half (without uharfbuzz); `VARC` is dropped; fonts with different axes are refused |
  | Variable CFF2 against fontmake's `variable-cff2` (M9.13, M12.25c, M12.25d) | 107 equal, 31 different, 20 failed of 158 after the thirteenth batch (106, 32, 20 before it; 104, 34, 20 before the eleventh batch). On Oswald the subroutinised CFF2 table is 1.009 of fontmake's size; the charstrings have the same operators and differ in their numbers only because the default master differs (the allow-listed Oswald origin). Measure: `python -I crates/cff/crosscheck/cff2_export_check.py` (it needs tx for fontmake's default build) |
  | TTX bitmap forms against ttx (M12.7f) | row form: 18 written and 36 read equal; bitwise: 15 and 30 (3 skipped, ttx fails on `CBDT`); external files: 18 dumps and 17 image folders written equal, 36 read equal; 0 different |
  | `--interpolate-binary-layout` against fontmake (M12.22d) | `GPOS` identical on 3 of 3 sources (Oswald-AE-comb, 18 tables; WdthVar, 9; kerning_ltr_and_rtl, 3) |
  | `EBDT` and `EBLC` writer (M13.55c) | bytes equal to fontTools' on the test strikes |
  | Compile extras against fontmake (M12.42) | implied on-curve modes equal in 484 of 484 glyphs each way; pass-through tables 31 of 31 after `tables add` and `optimize` |
  | Merge against fontTools merge (M12.23b) | 10 pairs merged by both, 2 refused by both; 143 tables the same, 2 different (`hhea` on fontTools' CFF test data; `BASE` tag lists kept once, a guess); unchanged in this batch. Replaces the M12.23 row above |
  | CFF2 instances against `varLib.instancer` (M12.20b) | every full instance and downgrade equal; partial range cases differ by fontTools' own partial-then-full rounding; `--update-name-table` 19 equal, 0 different |
  | TTX readers and split dumps (M12.7c) | every tag identical in the measure; `ttx -g` split dumps: 30 of 30 read back, 2,750 split files equal to ttx's |
  | Layout rebuilt on open (M8.23) | 302 corpus binaries export again (13 failed before the fixes); `GDEF` 282 equal and 20 different; `GSUB` 269, 3 and 30; `GPOS` 202, 51 and 49 (equal, different, not comparable); InterVariable opens in 135 ms |
  | Font View with 691 image cells (M11.16) | steady frame 1.08 ms |
  | Colour Text Preview (M11.17) | steady frame 0.39 ms |
  | Static oracle against fontc, TrueType | 63 of 354 sources differ (unchanged) |
  | Variable fonts against fontc and fontmake | fontc 103 equal, 35 different, 20 failed (was 101, 37, 20); fontmake 106, 36, 16 (was 104, 38, 16) |
  | Feature automation | unchanged from the sections below |
  | Measurement over a heavy line (M3.21, X.fix73) | 200 large glyphs measured up to the element limit, the rest left out, not refused; an edit to one glyph measures that glyph again only |
  | Custom parameter value checks (M7.21) | 64 parameters checked (10 before); every corpus value passes but three in fontc's test sources (`docs/spec/18-custom-parameters.md` › What FontCraft checks) |

  The fifteenth batch (depth wave 1) changed no compile output, so no oracle or AFDKO measure ran again; the
  `.glyphs` native round trip stays 122/1/118/4. The fourteenth batch ran every AFDKO measure again at `601ebf0` (tx, makeotf, CID keys, rotatefont, cffsubr, the
  CFF2 compile and its size on Oswald): none moved, and the 3 makeotf differences are now written down as makeotf
  faults. Times of the twelfth batch were
  taken at a load of 19 to 20 on 16 cores (the preview font build also at 8); earlier rows at 7 to 18. All are
  upper bounds. The oracle counts above are
  the first columns of the summary line; the TrueType and CFF totals include the 10 glyphs of colour layers. The "0 different" for CFF in older text below is stale: 12 CFF cases are allowed. The static oracle summaries did not move in this batch; CFF version 1 output is unchanged by the CFF2 work.
- **Measures that missed their targets** (a tuning package each, ROADMAP.md › What to build next):
  - TrueType autohinter (M10.21, M10.21b): size agreement (78.8% of ttfautohint's output) and cap-height rows
    (100%) meet their targets. X-height rows agree with ttfautohint in 92.9% (target 95%) and zone extremes
    land on their rounded row in 98.29% (target 99%). The measure covers only 7 of 325 sources (3 distinct
    fonts, no Inter); all 11 x-height misses come from one fault on a COLRv1 gradient source. ttfautohint
    itself reaches 91.6% on Oswald's zone row, so the 99% target is ours (M10.21c).
  - Auto-spacing (M13.20): after X.tune1 the median error is 5% to 12.5% on Oswald and 30% to 75% on Inter
    (target 10%). This method's best fit to the sources' own spacing is about 60% on Inter, so a different
    model is needed. Speed is met. Measure: `cargo test --release -p fontcraft-engine --test engine
    corpus_autospace:: -- --ignored --nocapture`.
  - FontDame conversion (M12.13, M12.13b): 24 of 28 equal. The other 4 (chained-glyph `GSUB` and `GPOS`,
    `gdefmarkattach`, `gdefmarkfilter`) cannot be expressed alike in both syntaxes and are written down in
    `docs/spec/40-compile.md` § 4b. VOLT conversion is equal.
  - Detect Composites (M13.15b, M13.15c): 78.1% of Inter's composites rebuilt from the source's bases (target
    95%). The 200 misses are merged attachments (ogonek, cedilla, stroke letters); 241 take other bases whose
    outlines equal the source's and that no name or glyph data tells apart. The target is out of reach without
    anchor or source hints.
  - Auto-kerning (M13.21) meets its target after X.tune1: the sign agrees with the sources' in 85.7% to 99.2% of
    pairs in every master. Measure: `corpus_autokern::` in the same test binary.
- **Other measures:** construction rules build 196 of 200 measured glyphs and place 2,406
  of 2,442 components within 1 unit. The Instructor draws a frame in 0.52 ms and an edit in 0.53 ms; the
  Hinting Preview waterfall takes 0.024 ms. FontBakery's verdicts agree on the 44 pairs compared
  (the `fsSelection` and `mac_style` checks are merged by design). Not run: ttxn and tx for the
  normalised dump, sfntdiff for `diff`, cu2qu and ttfcomponentizer for the curve tools, otfautohint for
  the `autohint` shell and over the corpus (target 99.5%), FontBakery over the corpus, and the § 5 measures of
  M6 (but mark attachment), M7 (but names) and M8. The TrueType debugger's end points are tested against the hinted preview's.
- **Measures that need AFDKO's tools** (tx, makeotf, rotatefont, cffsubr) write temporary files the agent sessions
  cannot write, so the owner runs them outside the sessions' sandbox. They last ran at `601ebf0`; the results are in
  the baselines table above.
- **Measures of the sixteenth batch, not yet run** (run them once the gate is green, with the oracles):

  | Package | Measure |
  |---|---|
  | M13.65 | `CPAL` version 1 and `COLR` over the 5 corpus COLRv0 sources, compared with ttx |
  | M10.37 | fontTools decodes every charstring of autohinted compiled CFF fonts |
  | M2.12 | `GPOS` and `GDEF` against `fontmake --feature-writer KernFeatureWriter --no-generate-GDEF` on 3 sources |
  | M12.74 | subset of 10 corpus sources, compiled, against the compiled font's subset (`--no-layout-closure`) |
  | M12.78, M12.36 | page counts against AFDKO's proof tool on Oswald; 20 corpus PDF proofs read back |
  | M12.69 | tx dumps of the merged font against mergefonts' (needs AFDKO's tools) |
  | M9.34 | Match Masters on the corpus, target 95%: `cargo test --release -p fontcraft-engine --test engine match_masters:: -- --ignored --nocapture` |
  | M13.21b | the kerning audit on the corpus, and `GPOS` of variable Inter before and after Match Kerning: `kerning_audit::` in the same binary |
  | M1.11, X.fix82 | a node drag timed on a large colour UFO with the format warnings on |
  | M12.75 | SVG fonts against `tx -svg -gn2` |
  | M12.66 | outline repairs against fontTools' removeOverlaps and checkoutlinesufo |
- **Decisions:**
  - A trusted ttfautohint binary runs only when it comes from the `export.ttfAutohintBinary` setting or
    `--ttfautohint`. The document cannot name one. This departs from the reference, for safety (the
    pre-push review's finding H1).
  - CFF import keeps every declared stem (`PsGlyph.stems`), because the handbook says hints are
    preserved and the measure counts declared stems; re-export keeps the charstring's stems.
  - The hinting web preview defaults to the local network, as the M10 plan's question 5 chose, with an
    "Only this computer" option. It serves a capped `std::net` server under a random path until the
    dialog closes.
  - `CPAL` colours are numbered in source glyph order, as both oracles number them.
  - `plugin` modules run only through `ExtensionState::runnable`; auto-updates of extension indexes are
    on by default, as the handbook has it.
  - The Delete Hints command has no shortcut of its own: the Select tool routes Delete.
  - The tracer smooths the pixel staircase before fitting curves.
  - The TrueType debugger (M13.52) runs on a ported copy of skrifa's interpreter in
    `crates/engine/src/ttvm`, credited in `NOTICE`. This departs from M13 plan § 3 (one interpreter, wait for
    skrifa). The copy keeps the upstream behaviour and a test compares its end points with the preview's. If
    skrifa gains a stepping API, drop the copy.
  - A glyph's own program (`hint.tt.setAssembly`) wins over its Instructor hints at export. It is kept when the
    Instructor tables cannot be built, and left out with a warning when it needs font programs that are not
    exported.
  - `path.setShapeOrder` is removed as a duplicate of `glyph.setShapeOrder`, which renumbers hints.
  - Auto-spacing clips at 10% of the x-height and weights the mean clipped depth with a fitted factor
    (least squares on n, o and v; H, O and V for capitals). Auto-kerning clips at 30% of the x-height and takes the
    power mean of order -2 of the row distances as the gap. Build Kerning Groups keeps 15%.
  - Instancing is capped at 1,024 instances per run with staged outputs; split TTX parts stay inside the dump's
    folder and are read-checked; the TTX tree has a memory budget; an anchor context that names an unknown glyph
    or class is left out with a warning (X.fix29).
  - Instance file names from a designspace may contain `..` and resolve from the designspace's folder, as
    fontmake does. An output on or inside a source file is refused, and names built from family and style
    names may not contain `/` (X.fix33, X.fix34).
  - A designspace instance's own `weightClass` and `widthClass` lib keys are ignored, as fontmake ignores them;
    the classes come from the axis location (X.ds1). Instances from sources and designspace instances share one
    naming rule.
  - Instance, transform and merge keep the stored `maxp.maxSizeOfInstructions` unless glyph programs were
    removed, as fontTools does.
  - `font.optimize` keeps the input's container (.ttc collection, WOFF, WOFF2) and its extension.
  - A colour glyph that needs more paint than the budget allows draws as one plain fill, the silhouette of
    the colour drawing, with a larger per-pixel allowance (X.fix33, X.fix34).
  - `audit.fix` with `inPlace` through a symlink writes the linked file and keeps the link (a guess on the row).
    `audit.fix` declares that it writes files.
  - Review fixes (X.fix31 to X.fix34): sparse `gvar` and `cvar` delta sets with an early tuple cap, glyph point
    counts checked against flags, one subtable budget per bitmap location table, `gvar` decoding charged and
    cached per glyph, name strings capped and deduplicated; a linear `cmap` conflict report, hashed
    feature-variation and `HVAR` region deduplication under a cap on store regions, one draw budget and hashed
    measurement tables per `avar` plan, a byte cap on each rebased delta item, one budget for designspace
    rules with composed glyph swaps (a cut is reported), a linear `prep` search, glyph grouping by font
    dictionary, a width stack check, a cell cap on control corrections; `plan-avar` writes the font and the
    plot together, resolved paths for output collisions, a write check before staging, the face chosen by
    PostScript name on single-font files.
  - Convert follows the oracles: the curve error is one font unit by default (as otf2ttf), fractions are kept,
    FontMatrix is ignored (as fontTools ignores it), and seac becomes composites in TrueType and is drawn out in
    CFF2 (X.conv2). The differences left are written down in `docs/spec/40-compile.md` § 4c.
  - A TrueType compile drops implied on-curve points by default, as fontc does; `--keep-implied-oncurves` gives
    fontmake's output (M12.42).
  - Project compile follows makeotf 5.0.1: names 1 to 6 and the menu names only, `OS/2` and `hhea` computed, the
    version from the feature code, overlaps kept, `kerning.plist` ignored, composites as the file places them
    (M12.40b).
  - Merged bounds follow fontTools 4.66, which skips components that use point bounds; a test pins the values.
  - Save choosers show one filter, and the extension picks the format. The reference has a format pop-up, so this is
    a guess on the rows `file.save` and `file.saveAs`. A chosen name without an extension gets the offered one.
  - Untrusted fonts are held to a charstring memory limit of 1 GiB (512 MiB on wasm), a TrueType-to-CFF point limit
    of 16 million (8 million on wasm) and a CFF-to-TrueType point limit of 67,108,864 (2^26); a full CJK font fits
    (X.fix42). The subroutiniser charges its arrays to a 4 GiB budget (512 MiB on wasm) and takes up to 2^26
    tokens (2^24 on wasm) (M12.25b).
  - A CFF2 subset prunes subroutines before it dehints. Where pruning cannot be done safely the subroutines stay and
    a warning says so; `--desubroutinize` is held to the charstring memory limit.
  - Review fixes (X.fix35 to X.fix40): empty-lookup calls and 16-bit index overflows are refused before fea-rs sees
    them; backwards `SVG ` and `COLR` ranges are skipped; `SVG ` tables are parsed once; bitmap strikes and entries
    are capped; convert, instance, subset and dehint share charstring memory and point budgets; class pair tables
    whose two value formats are 0 keep their records when `GPOS` is read again (`restore_empty_pair_records`);
    seac parts are parsed and drawn once; TTX bitmap image sizes are refused before the padding is allocated;
    project-compile paths are checked against the read root; convert outputs and collections are staged; split-dump
    parts cannot lead outside the dump folder through links; stored selections are never written back cut; staged
    audit findings stay until the report is written.
  - The feature-code lookup guard counts lookups as fea-rs groups them and is an upper bound (X.fix41, X.fix47): it
    may refuse valid code only very near 65,535 lookups written in rare forms.
  - `optimize` with no step named keeps implied on-curve points, as fontTools does. When the step is named it tests
    every tuple-peak instance and keeps points that composites, anchors or carets number. `GPOS` compaction is off
    unless a level is given (X.opt1, X.opt2, X.fix50).
  - Project compile writes OS/2 version 3 by default, as makeotf does (M12.40d).
  - Variable `COLR` is instanced in full and partial instances; fontTools leaves it untouched, so this is a guess
    (M12.21b).
  - Of two same-named instances the later wins with a warning, as fontmake does (M12.22b).
  - Audit folder entries outside the read root are skipped with a warning (`docs/mcp.md`; X.fix45).
  - Subset `--drop-tables` is a whole list (`=`, `+=`, `-=` as pyftsubset); the output container follows the input
    (M12.18b).
  - Every command that writes `GPOS` back fits class-pair record counts to the class definitions
    (`fit_pair_classes`, X.fix52); fontTools keeps the stored counts.
  - The preview font and `features.compile` share one build per revision and master; a compile always builds `GPOS`
    with the generated kerning and marks (X.edit1).
  - Review fixes (X.fix47 to X.fix54): the lookup guard follows flags through nested lookup blocks; a colour
    reference to a glyph the compile leaves out draws as an outline, and reference checks take linear work per
    master; Unicode Variation Sequence values and ranges are checked, and a stub `cmap` keeps the Mac and format 14
    subtables; `mort` state cells are charged for their entries' actions; `VARC` records are parsed once and an
    empty unused store is kept; the UFO colour-layer fold looks up each layer glyph's users; removing a glyph
    removes its strike bitmaps; unkeyed includes never reuse a kept preview build; layout-only masters are checked
    against the read root first; shared variation-store subtables are charged per use and the merge heap is
    capped; `interpolate_layout` charges every master's `GPOS` against one budget; `avar` version 2 regions merge
    by hash; a `gvar` zero point count means all points; CFF bounds count move points; dehint copies a glyph's data
    only up to its end.
  - CJK exports subroutinise by default, as the handbook documents; Disable Subroutines turns it off. The
    subroutiniser handles full CJK fonts (136 s on Source Han Serif TC Bold). The owner asked to follow the reference.
  - Variable fonts merge as variable fonts only with identical axes; every rule is ours, as fontTools has no variable
    merge. A merge that drops `fvar` gives a static font with the default values and no variation stores (fontTools
    refuses) (M12.23c, X.fix58).
  - CFF2 exports are specialised as fontTools specialises merged charstrings and subroutinised when that makes them
    smaller (M12.25c). Hints blend as varLib merges hinted masters; a glyph whose masters' hint structures differ
    loses its hints with a warning, where fontTools fails the font (a guess; M12.25d).
  - CFF2 exports write `maxstack 513` in the Top DICT only when a charstring needs more than 193 operands (the
    default stack). fontTools omits the entry, so such fonts differ from its output by that entry; ours follows the
    specification (X.fix58, X.gate16).
  - The ROS custom parameter makes CFF exports CID-keyed: CIDs come from `cid<n>` names, then from a user CMap, and
    the other glyphs are appended. No character collection data is bundled (M12.41b).
  - Bitmap-only fonts (BDF, PCF, `.otb`) open as fonts at 1,000 units per em (a guess), and the embedded strikes of
    compiled fonts are read into the model on open (M13.55d).
  - `.otb` export writes a full-length `loca` and keeps every glyph (guesses; FontForge's documentation differs)
    (M13.55c).
  - Vertical Text Preview shapes through harfrust with `vkrn` on and `vrt2` when the font has it; `vpal` and `vhal`
    stay off (a guess; M8.20c).
  - Every menu switch shows a check mark (40 switches). The handbook documents check marks only for Show Info and
    Show Metrics (X.ui2).
  - Subset `--show-defaults` is ours (pyftsubset has none); `--OPTION?` prints pyftsubset's line for the option
    (M12.18c).
  - The CFF matrix of `transform` follows rotatefont's reader: single-precision sums, end points rounded to
    hundredths, stems of negative width swapped, hint groups regrouped as its writer does. Its `-rt` half-angle
    quirk is kept as a written-down difference (X.fix55).
  - A merged name table with more records than its 16-bit storage offsets can address is refused before writing
    (`MAX_NAME_RECORDS`; the table writer panics past it) (X.gate16).
  - Review fixes (X.fix56 to X.fix58): `--interpolate-binary-layout` compiles into a scratch folder and writes
    nothing on failure; `bitmap.setStrikes` measures every strike against one budget per call; the parser decides
    whether an optional option value is given; `SVG ` export bounds the fan-out of colour references with a paint
    visit limit and a running table size, and caches nesting per referenced glyph; CFF2 glyphs whose stem blends
    overflow the stack are reported; strikes share a font-wide decode budget and an image cache; static merges drop
    `GDEF` stores and variation devices; joined variation stores refuse mixed axis counts and deduplicate regions by
    hash.
  - A `.glyphs` source without a family name compiles to `Unnamedfont-*` files, as fontmake names them; other
    sources take the source file's name. The file exports share this naming (M12.22e).
  - `.glyphs` node flags are read in any order and unknown letters are kept: `lXR` reads as locked and Right, and
    an edit writes the known fields in the writer's order followed by the unknown letters (X.fix61).
  - One-line layer guides, hints and annotations in `.glyphs` files reach the model and keep their layout
    (X.fix66). Stale kept spellings and layouts are left out on save (X.glyphs3).
  - Detect Composites orders bases with equal outlines: the Latin letter beats a look-alike copy (in either font
    order), then the glyph database's recipe, then name relations (`sub` read as `below`), then suffix case
    (`.uc`, `.case`, `.tonos`, `.lc`, `.sc`, `.c2sc`, `.smcp`), fewer extra suffixes, a combining mark (`…comb` or
    `…cmb`), the shorter shift (M13.15c, X.fix66). The 95% target needs anchor or source hints; the measure stays
    at 78.1%.
  - `cvar` comes from each master's control values as fontTools merges hinting, rounded half to even. Masters
    whose control values do not line up give no `cvar` and a warning, where fontTools fails the font; a
    duplicate master location warns (M10.19b, X.fix62). An alternate layer's own instructions export for its
    alternate glyph when the layer belongs to the main master; an intermediate layer's never do (a guess).
  - CFF2 hints of intermediate and alternate layers come from the main master's manual hints through node links,
    or from the autohinter run on all sources together; autohinted CFF2 glyphs have no flex (a guess; M12.25e).
  - Convert: a CID-keyed result draws `seac` glyphs out; a glyph whose CFF overlaps are removed loses its stem
    hints; a variable font converted to its own flavour keeps its variations without a location; `fdSelect` takes
    a list of Font DICTs and accepts CFF2 (guesses; M12.30c, M12.30d, X.fix63).
  - Bitmap formats: FNT glyph names, PK and GF pixel sizes and names, the FON writer's layout and the Mac reader's
    choice of the first family's plain style are guesses. PK and GF codes are in TeX encodings, so their bitmaps
    match glyphs by name only (M13.55e, M13.55f, X.fix65).
  - A pasted PDF keeps one font unit per point with the page's bottom-left corner on the glyph's origin; JSON is
    read first, then SVG, then PDF (guesses; X.pdf1).
  - `SVG ` filters draw eight primitives on 8-bit premultiplied pixels in linear RGB; an element whose filter
    cannot be drawn is drawn unfiltered with a warning (M11.17d, X.fix60).
  - The audit's layer cache covers the layer checks only and keys on the resolved thresholds (X.audit3, X.fix61).
  - Each document window keeps its own Font View master (a guess; X.ui3).
  - Review fixes (X.fix60 to X.fix66): filters honour enclosing clips and the sprite is sized by their reach; a
    placed PDF keeps its bottom edge; the CFF2 instancer charges expansions against the charstring memory budget
    and accepts 513-value blend runs, real `vsindex` and `dotsection`; GOADB files and CID selectors are capped;
    replaced UFOs are restored when a batch fails; Mac strike listings are bounded and deduplicated, TeX codes
    stay out of Unicode and an import takes one strike per size; Detect Composites' Latin-copy rule does not
    depend on glyph order, and its suffix comparison is charged to the work limit.
  - Instance UFOs keep a style-map style outside the four RIBBI names in the lib
    (`com.fontcraft.styleMapStyleName`), not in `fontinfo.plist`, because ufoLib with validation and fontc refuse
    it there. fontmake writes it to `fontinfo.plist`; we differ on purpose (X.fix72).
  - A designspace without axes: masters build; instances come from one source only; a stated location is
    refused. The writer leaves out `<location>` and always writes `<axis-subsets>` for variable-font instances,
    which fontTools needs (X.inst4, X.fix72).
  - UFO style-map styles follow ufo2ft: exact comparison, name ID 2 title-cased, no REGULAR fallback (X.fix70).
  - The `checkOutlines` clean-up after a fix keeps smooth flags, node names and component offsets, which the
    reference tool loses or rounds (X.audit4).
  - TTX import keeps a CFF table's bytes, with a warning, when its strings hold characters XML text cannot carry
    (X.fix69).
  - Clipboard PDF paste reads off the UI thread through the platform's clipboard tools (wl-paste, xclip,
    osascript, PowerShell), with a 64 MiB cap and bounded waits. Native clipboard crates replace them on Windows
    and macOS in X.pdf3 (X.pdf2, X.fix68).
  - `SVG ` filters in Edit View draw clipped to the canvas, and the canvas says when it drops filters. The
    turbulence code follows the W3C specification's reference code, credited in `LICENSE-W3C.txt` and `NOTICE`
    (M11.17e, X.fix71).
  - Project-compile feature code has one lookup per rule type, as makeotf does; its messages keep the file and
    position of included code, mapped through the include table of the files (M12.40c, X.fix67).
  - Closing and quitting ask about one document at a time, fonts first, then projects. A failed or cancelled save
    stops the quit; a document edited while the question is open is asked about again. `window.closePrompt` with
    `save` from automation on a document without a file returns an error instead of opening a chooser. While the
    question is open only Return and Escape reach it (M14.3, X.fix76).
  - Off the Mac the Measurement tool is held with Ctrl+Alt+Shift, because Ctrl+Alt is AltGr there and the Select
    tool's mirror chord; on the Mac with Cmd+Alt+Ctrl, as the handbook says (M3.21, X.fix73).
  - A master-row click in the Layers section with text selected switches every selected glyph and the edited glyph
    (M3.40, X.fix75; a guess).
  - Axes are addressed by index where two share a tag; a shared tag in a command is refused as ambiguous (M9.27,
    X.fix74).
  - Settings… is in the Edit menu on every platform; the User Settings switches are off by default (M14.1).
  - Control-channel input is applied one event per drawn frame, inside the confinement of input frames; the
    person's own press or the save question drops waiting synthetic input; `ui.resize` clamps to the monitor and
    the GPU's texture size (M13.78, X.fix77). The `ui.set` key names are our own, listed in
    `docs/control-protocol.md` › View switches.
- **Decisions of the sixteenth batch:**
  - A source with several masters and an axis compiles to a variable TrueType font unless a static format is asked
    for, as fontc does; fontmake builds statics (M2.8). The owner confirmed this default.
  - The CLI's `open` keeps the source's glyph names (`keepGlyphNames`), for parity with fontmake; the window renames
    to nice names on import unless the setting says otherwise (M1.9).
  - Overlapping multi-master glyphs are autohinted on their overlap-free outline by default, as otfautohint does
    (M10.36); every CFF the converter writes takes tx's defaults, which may move the convert measures (M12.70).
  - Source includes apply only when no feature folder is given (M2.12). Color Palettes of versions 2 and 3 become
    the version 4 dictionary when `color.generateDarkPalette` runs; the writers write lists back with a loss report
    (M13.65, M13.65b).
  - Commands that rebuild shapes renumber hints themselves and the general hint upkeep skips them; mixed PostScript,
    TrueType and corner hints on a split stay and are unlinked (X.fix78, X.fix79, a guess).
  - The new window: one audit step per frame (UI.17); Return in a dialog field confirms the dialog (UI.7); fired
    dialogs open on the Font View selection and runs over the control channel ask no follow-up questions (X.fix84);
    canvas tools edit the layer the Layers panel shows (X.fix83). Its control server keeps FontCraft's own transport
    without a token, so MCP attach is unchanged; the suite's server needs a token (see "Waiting on the owner").
- **New dependencies between crates:** `engine` and `textlayout` depend on `shaping`, `ui-egui` on `io-pdf` (all internal); `hinting` re-exports `ttasm`; `engine` re-exports `fea`. This batch: `engine` depends on `io-pdf` (PDF page sizes), `io-bitmap` (strikes); `render` on `container`, which holds the gzip reader; `textlayout` on `icu_properties` (line breaking); `tables` takes `miniz_oxide` as a dev-dependency. `io-bitmap` depends on `container` for gzip (`.pcf.gz`, `.bdf.gz`). The fourteenth batch: `sha2` is a workspace dependency (the PDF cache key; it was in the lock through lopdf); the desktop app starts the platform's clipboard tools.
- **New crates and dependencies:** `craft-quit` from the suite's craft-libs, pinned by git commit, with its
  `egui` feature, and its `menu` feature on macOS for the app menu's Quit (M14.3). `crates/shaping` (harfrust), `crates/io-bitmap` (BDF and PCF),
  `crates/io-pdf` (lopdf, with a hand-written reader and rasteriser under limits). `ureq` with rustls serves
  HTTPS downloads in the desktop app only. `craft-print` is a workspace dependency and is not in the
  lock file until M12.36 uses it (that needs network).
- **Known failures:** none in the gate. `pathops` `random_paths_through_every_operation_never_panic` can miss its one-second limit per case when the machine is loaded; it passes on a quiet machine.
- **New crates and dependencies of the sixteenth batch:** `crates/engine-core` (errors, guard, clock, parameters, settings) and `crates/ui` (the new window), with `apps/fontcraft-next`; the craft-next framework from the suite's craft-libs, pinned by git commit; icu 2.3.0 across the workspace, with the glyph-data comparison against glyphs-reader moved into an excluded crate under `xtask`; `tools` depends on `components` and `pathops`, `proof` on `decompile` (and lopdf), `engine` on `textlayout`, `ui` on `spacing` and `textlayout`; `apps/fontcraft` takes clipboard-win on Windows and objc2-app-kit and objc2-foundation on macOS (X.pdf3); the CLI takes `log`.
- Test layout: the gate runs the tests with cargo nextest (all test binaries in parallel,
  doctests apart). The command sweep is 16 tests (4 states times 4 chunks, chunked by a stable
  hash of the command id), and one randomised pathops test reserves 8 threads
  (`.config/nextest.toml`). The engine's integration tests are modules of one binary
  (`crates/engine/tests/engine/`; `cargo test -p fontcraft-engine --test engine <filter>`), and
  the decompile, io-glyphs, io-ufo and plist tests are one binary each. Speed measures and oracle
  runs use `--profile measure` (release without link-time optimisation); see
  `docs/development.md`.
- Exports carry compiled feature code. Against fontmake, `GSUB` is equal in 25 of 26 sources,
  `GPOS` in 82 of 88 and `GDEF` in 47 of 55; against fontc, 29 of 46, 80 of 91 and 42 of 68. The
  rest are bracket glyphs and colour glyphs (M11). The name table and OS/2
  of static instances equal fontmake's in 1,678 compared values and differ in 120 (three
  allow-listed causes). Automatic entries regenerated over the corpus: 69 equal, 23 different
  (stale sources, older reference forms, samples), 3 removed
  (`cargo test --release -p fontcraft-features --test corpus_auto -- --ignored --nocapture`).
- Variable fonts (`fontcraft-cli oracle --corpus --variable`; `--instances` for static instances)
  are compared with fontc's or fontmake's at masters, virtual masters, instances and midpoints
  between masters. Over 158 sources: against fontc 103 equal, 35 different, 20 failed; against
  fontmake 106, 36, 16 (before the eleventh batch: 101/37/20 and 104/38/16; before M9.12: 100/37/21 and 103/38/17; before the variable compile all
  158 were not built). Per check, against
  fontc / fontmake (equal and different): outlines 127 and 8 / 128 and 11; `GSUB` variations 129
  and 8 / 135 and 6; `GPOS` kerning 135 and 1 / 141 and 0; marks 134 and 2 / 139 and 2. fontc and
  fontmake agree with each other on 112 of 133 sources (`docs/spec/40-compile.md` § 4). Static
  instances against `fontmake -i`: 180 equal, 30 different, 0 not built, 24 failed (all 24 are
  fontmake's own failures); before M9.4: 111, 25, 83, 15. Speed: Oswald compiles in 0.19 s (fontc
  0.16 s, target 0.5 s); Inter Roman in about 1.3 to 1.6 s (fontc 1.66 s, target 3 s).
- Variable feature syntax (M9.11, M9.12): `condition` statements become `conditionset` and
  `variation` blocks, with internal limits mapped to user coordinates and overlapping regions
  overlaid; `(tag:coord) value` positions become variable scalars, also in value records and
  anchors. The lowering is ported in part from glyphsLib (credit in the file header and
  `NOTICE`) and equals its converter on 32 synthetic texts. Variable exports compile the feature
  code on fea-rs with a real variation info (axes with their maps, the masters, number values):
  `condition` and `#ifdef VARIABLE` become `GSUB` feature variations merged with the alternate
  layers' records, and variable `pos` values and per-master number tokens resolve through the
  same model as the generated kerning, into one `GDEF` store. Inverted conditions are export
  warnings. Equal to fontmake on variable `pos` plus alternate layers and to fontc on condition
  sets plus number tokens. Static exports resolve values and conditional rules at their
  location, instances between masters included. A number token varies only in metric statements
  and takes the origin value elsewhere; two masters at one location with different token values
  are an error. Variable values in table blocks (`fea.var.tableValues`) are not built.
- CFF2 variable fonts (M9.13; `fontcraft-cli compile --variable --format otf`, or `--format
  variable-cff2`, and `file.exportVariable {outlines: "cff2"}`; the Export dialog writes
  TrueType): blended charstrings, specialised and subroutinised when that makes them smaller
  (M12.25c), with blended hints (M12.25d), for intermediate and alternate layers too (M12.25e), one
  Font DICT with a blended Private DICT holding each master's family blues (X.fix59), the item
  variation store, `hmtx` with `HVAR`, `post` 2.0 and `maxp` 0.5. `oracle --variable --format otf`
  against fontmake's `-o variable-cff2`: 107 equal, 31 different, 20 failed of 158. Every outline and advance difference also shows in
  the TrueType build against fontmake.
- TrueType hinting (M10.17, M10.6, M10.7): `hinting::tt_font` builds `cvt ` (zone positions and
  fitted rows, stem widths and fitted widths, family zones), a seven-function `fpgm` library
  (zone rounding and alignment, rounding, cvt delta, zone snap with overshoot suppression, stem
  with five rounding modes and the minimum distance, point delta) and a per-size `prep`; each
  handbook behaviour is tested in skrifa's interpreter. M10.19 writes them into exports.
  The handbook's rounding-threshold example contradicts its stated default (0.09375 lifts
  6.45 px to 7, the example says 6); FontCraft follows the default, and a trial check is listed.
  14 commands edit instructions against `TouchedState`, which replays the touched points per
  direction: Snap Point, Set Snap Zone, Stem, Reverse, Set Stem Reference, Make Triple Hint, Set
  Instruction Rounding, Shift Points, Interpolate, Delta, Set Delta Cells, Delete Instructions,
  Remove Hints and List TrueType Instructions. Delta is vertical and main-master only, with
  bounded PPM cells; Delete Instructions reports the later instructions it leaves unmet and
  dissolves broken triple hints. Each command is one undo step, a refusal leaves the font
  pointer-equal, and the command sweep passes.
- Oswald's variable origin: the oracles choose the master named "Regular" (glyphsLib's rule);
  the handbook says the first master unless Variable Font Origin is set. FontCraft follows the
  handbook, and Oswald is allow-listed.
- An Edit View text-mode frame with 200 composites takes 5.9 ms (target 16 ms).
- Hint upkeep (M10.3) costs 3.1 µs per edit on Inter for a glyph without hints and 83 µs with
  hints. It does not renumber background layers or corner and cap hints.
- CFF exports carry stem hints, masks, flex and the source's manual PostScript hints (M10.14).
  fontTools decodes 43 exported fonts with 0 warnings; skrifa's PostScript hinter draws every
  glyph; the CFF oracle against fontmake has 0 different glyphs (1,838 same).
- TrueType export carries instructions (M10.18, M10.19). `fontcraft_hinting::glyph_program`
  compiles a layer's instructions plus the exported point map to `glyf` bytecode: Snap (zones,
  named-zone distance, five rounding modes), Stem (Auto, named, No Stem), Shift and Interpolate
  with rounding, Delta before IUP, triple hints keeping a pixel between stems, diagonals
  preserved unless `TTFDontPreserveDiagonals`, Ignore Vertical Hints; `fpgm` has 10 functions,
  and 14 behaviour tests run the programs in skrifa's interpreter in both directions. The point
  map follows cu2qu, overlap removal and decomposition in one linear pass under one work budget
  per glyph; an off-curve node next to a removed node is left unmapped with a warning. The font
  gets the generated `cvt `, `fpgm` and `prep`, or the imported `cvt Table` and the `prep` and
  `fpgm Table Assembly` parameters, `maxp` covering the analysed needs, and the `head` and
  `gasp` flags. Static instances interpolate zones and stems by name and take the main master's
  hints (TrueType and PostScript) at matching nodes. A glyph that fails to compile exports
  unhinted with a warning. Variable fonts take the default master's programs, without deltas
  or `cvar`; an instructed source built without glyph programs writes plain points.
  `hint.tt.getPointIndexes` is live. skrifa runs every exported glyph of the instructed corpus
  samples at 8 to 72 PPM in three intents with 0 errors; fontTools reads them without
  warnings. Oracle baselines are unchanged (static against fontmake: 2,360 compared, 2,202 the
  same; variable against fontc: 101/37/20). Left: `cvar` and special layers' own instructions.
- Atomic file writes (X.suite1) go through the Craft suite's `craft-atomic` library, taken from
  craft-libs and pinned by git commit; `fontcraft_format::write_atomic` keeps its signature.
  Native builds only: the browser build refuses to write. Saving over a read-only file is
  refused and the original kept; the folder is flushed after the rename.
  `docs/decisions/0008-shared-suite-libraries.md` lists which suite libraries FontCraft adopts
  and when (`craft-quit`, `craft-settings`, `craft-print`, `craft-ui`; not `craft-cms`).
- Paint attributes are typed in `.glyphs` versions 2 to 4 (M11.5 part 1). Writing version 3 and
  saving a UFO report what they cannot hold. All 23 corpus colour sources keep their colour keys.
- The audit runner (M12.44) ran 3,676 times over 4 profiles and 919 corpus files with no crash;
  the readers refuse 70 sources as unreadable. The outline checks give many warnings on decoded
  compiled fonts; the cause is not examined.
- Subset (M12.17) against pyftsubset over 340 corpus fonts and 4 selections: the glyph map is
  equal in 338 or 339 per selection and the core tables in 337 or 338. The rest are one CFF2
  font (refused) and two `cmap` test fonts.
- The module runtime (M13.2) on 1,000 synthetic layers: a filter takes 292 ms (target under 1 s);
  a reporter takes 0.35 ms per layer uncached (target 5 ms) and 0.4 µs cached (target 1 ms). No
  module commands, export filters and check modules call it (M13.3 to M13.5).
- Round trips: `.glyphs` native 122/1/118/4, `.glyphspackage` 22/0/7/0, designspace native 59/0/0/53, UFO model
  125/0/2/13.
- Measures: console completion takes 2.28 ms at worst (target 5 ms, met). The alignment pass of the
  follow-up hook after a one-glyph edit on Inter takes 0.24 ms (target 2 ms, met). Detect Composites on
  Inter finishes in 0.4 to 0.7 s (target 2 s, met) and rebuilds 78.1% of the composites from the source's
  bases (target 95%; 63.0% before M13.15c). Among bases with equal outlines it prefers the Latin letter over a
  look-alike copy, then the glyph database's recipe, name relations, suffix case, fewer extra suffixes, a
  combining mark and the shortest shift.
- Detect Composites keeps automatic alignment on where it reproduces the detected placement and
  turns it off only where alignment would move a component (a guess on the row).
- The desktop app was run on a virtual display and driven over the control channel; Font View and
  Edit View were looked at, and the screenshots of each M3, M4 and M5 UI package were reviewed.
  The Font Info window, the M13 dialogs and M6.10's component display have frame tests and no screenshot review yet. macOS
  and Windows have **not** been built.
- The repository is private at `github.com/aiongg/fontcraft`. There is no hosted CI (the gate
  runs locally) and no release.

### Automation file roots

`fontcraft-cli mcp` takes `--automation-read-root DIR` and `--automation-write-root DIR`. The
desktop app's control channel takes the same two flags, or the variables
`FONTCRAFT_AUTOMATION_READ_ROOT` and `FONTCRAFT_AUTOMATION_WRITE_ROOT`. Without them, file access
is unrestricted. Read and write roots are separate. While a root is in force, every declared file
parameter, every implicit place (export folders, packages, snippet folders, screenshots) and every
linked file is checked against it, and a linked file outside the read root reads as missing.
`--read-only`, `--allow` and `--deny` are unchanged. Details: `docs/mcp.md`.

## Where to start

1. **Finish the sixteenth batch.** The gate is green and the batch is reviewed and pushed. Next, the measures in "State of the tree" › Measures of the sixteenth batch.
   Checks on the Mac, which builds and runs: wave 1's Cmd+Q, app-menu Quit, File › Open… chooser and drop with unsaved
   work, and X.pdf3's pasteboard PDF; on Windows (not built yet), X.pdf3's clipboard PDF.
2. **Then, in this order** (ROADMAP.md › What to build next, items 2 to 5):
   - The new window's ports left (item 2): the Extension Manager's Indexes tab and downloads; and the framework gaps
     in `crates/craft-ext` (a text-field hook for syntax colour and error underlines, caret and selection access, an
     async completer, Up/Down stepping, a press-and-hold flyout, union, intersect and slant icons, open URL and
     reveal in the file manager, alternate menu rows with their own label and shortcut, an app-level paste event);
     then the small follow-ups listed there.
   - Retire the egui window: remove `crates/ui-egui` and `apps/fontcraft`. `cargo xtask parity` must then read the
     commands of `crates/ui`.
   - The engine split from ES.4.
   - The oracle measures already listed and the depth writers left in waves 2 to 11, with the builder queue from
     M8.25.

   Each depth package is written out in its milestone plan's section "Depth packages" (`docs/plans/M<n>.md`). Check
   the disk: build output is large (`cargo clean` or delete the target directory when the disk passes about 75%;
   incremental builds are off, `CARGO_INCREMENTAL=0`). Parallel agents that share one check directory across
   worktrees see false errors: give each its own `CARGO_TARGET_DIR`. Run one agent at a time on the new window's
   crates. The measure-only builders (M12.71, M12.72, M12.83, M8.34, M2.11, M10.39) can run in a second lane with
   their own clone and target directory.
3. **Open decisions** (`docs/plans/depth.md` › Open decisions): eight questions for the owner. The packages that
   wait on them sit at the end of the waves (M3.27, M3.28, M3.38, M4.22, M4.23, M5.31, M5.32, M13.72 to M13.77,
   M14.7, M14.8); the auto-spacing and auto-kerning targets do not hold up M13.20b or M13.21c.
4. **The measures that missed** have depth packages: M10.21c (the TrueType autohinter's misses and a measure over
   more than 7 sources) and M13.20b (auto-spacing by shape classes). Detect Composites on Inter stays at 78.1%
   against 95% (M13.15c); the rest needs anchor or source hints, and no package is queued. Measure: `cargo test
   --release -p fontcraft-engine --test engine detect:: -- --ignored --nocapture`.
5. **The close builders, last:** M6.19 (after M6.24), M7.20, M8.24, M9.26, M10.32, M11.20, M12.65, M13.61. The
   other packages the first plans left are in the waves: M12.36 (built), M13.40 to M13.42 (waves 4 to 6), and
   M13.59 in a spare slot of wave 11 or 12. M12.1a to M12.1c are built.
6. **Follow-up packages the writers named:**
   - TTX: the TrueType `post` table compiled again when a dump lacks it; glyph-order renames in a TTX import into a
     CFF font (X.ttx4).
   - Instances: the file names of masters exported as instances, against fontmake's (`FontInfo-Thin.ttf`).
   - PDF: `draw_placed_pdf` hashes again when glyphs alternate between two PDFs.
   - Merge: CFF2, CID-keyed fonts and `JSTF`, which fontTools refuses too.
   - Convert (M12.30): locations beyond the axis ranges, the other output targets and options for UFO output.
   - Project compile (M12.40 to M12.42): mirrored components in single precision and makeotf's lookup grouping
     across lookup flags, which are not followed (M12.40c); a guess entry for the compile's fallback warning;
     bundled character collection data and `GSUB` resources for the ROS parameter.
   - Colour: `feImage` in `SVG ` filters; filtered sprites keyed by glyph, not by occurrence (a large glyph clipped
     at both canvas edges runs its filters again on every frame). Colour drawing: screenshots.
   - Table Viewer (M12.38): the source of the last export, a control-channel measure and screenshots.
   - Small UI leftovers: the Exports tab's aliases (not yet defined).
   - Wave 1: the project asked about in the save question is not brought forward; `ui.text` can type into a
     dialog behind the question, and Tab does not move focus inside it; check that tool letters stay blocked while
     it is open (M14.3). `ui.contextMenu.invoke` does not notice a master switch since the menu opened, and a
     multi-step item is not one transaction (M13.78). The Text tool moves the measurement line with Cmd+Ctrl+Alt,
     which is AltGr off the Mac (`canvas/text.rs`); off the Mac a Ctrl-drag also turns on the lever, as Ctrl reports
     as Cmd (M3.19, M3.21). Cmd+Delete on an axis row; command warnings do not reach the panels (M9.27). The
     extrapolation audit check (`instanceOutsideAxisRange`) keeps its own tolerance rule: decide whether it shares
     the placement helper (X.fix74). A corpus listing of the values the new parameter checks refuse (M7.21).
7. **The suite libraries:** `craft-quit` is adopted (M14.3); X.suite3 (`craft-settings`, settings in `config.toml`
   with backup and recovery and a one-time import of the JSON settings file) is the wave 12 builder; `craft-ui`
   is M14.9; `craft-keymap` (M14.7, M14.8) waits on two upstream pull requests. Watch item: the suite's proposed
   shared control-port authentication (`craft-control`); adopt it for the control channel once it exists. Pin
   each crate to a git commit; bumping a pin is its own commit with the full gate.
8. **Smaller follow-ups:** an intermediate layer's own TrueType instructions and the variation of composites with
   intermediate layers (M10.19, M10.19b); variable values in table blocks (`fea.var.tableValues`: `BASE`, `GDEF`
   carets, `hhea`, `OS/2`, `vmtx`; fea-rs rejects them); the layout of InterpolateLayout3 (two `.notdef`
   glyphs), fontc's combinatorial bracket kerning and decreasing-axis tables, and brace sources for composites of
   non-exported brace glyphs (with M9.39); subsetting sources (M12.74).
9. **The measures not yet run:** ttxn and tx for the normalised dump, sfntdiff, cu2qu, ttfcomponentizer,
   otfautohint for the `autohint` shell and over the corpus (M10.33), FontBakery over the corpus (M12.81), and the
   § 5 measures of M6 (M6.22; mark attachment is run), M7 (names are run) and M8 (M8.34). M6's commands:
   `cargo test -p fontcraft-oracle --features oracle --test marks -- --ignored`;
   `crates/engine/tests/engine/corpus_propagation.rs` (it reads a JSON file of glyphsLib's results made outside
   the repository), `corpus_composition.rs`, `corpus_alignment.rs`.

Low findings from the last review (the automation roots), for a later fix pass:

- A timing bound in the variable-font decompile test (`few*2+20` ms) may fail under load. If it
  does, count comparisons through a hook instead.
- An image path taken from the document and resolved through links can be stored with `../..`
  segments that climb out of the folder. Cosmetic.

What the second-wave and M13 packages left for later packages (the hooks and gaps the integrators
recorded):

- Font Info: the Features tab stays read-only until M8.17; the Exports tab shows a message for
  instances between masters until M9.20 draws interpolated instances from `interp::instance_font`
  (then the `fontInfo.exports.instancePreview` row changes); Add Other Font on the Masters tab waits for
  M9.23.
- M8.15: prefix-defined classes cannot be previewed, and `markClass` shows only through
  definitions; M8.19 compiles the preview font apart from `features.compile`; one compile for both is open.
- M7.14: the Save and Close buttons should run `project.save` and `project.close`, and
  `file.open` should route `.fontcraftproject` to `project.open`; `Session::inspect` does not
  list projects; project edits are not undo steps; `.glyphsproject` waits for a reference sample.
- M6.14: each automatic composite copies the glyph list, so composing very many glyphs at once is
  quadratic; compose without the per-glyph copy.
- M8.7: variable-font layout tables come from M9.9 and bracket glyphs from M9.8b; colour glyphs
  wait for M11 (the layout oracle lists them as known differences). Feature includes may climb out of the
  source folder, as the reference compilers read them; the check-then-open window of the include
  check is documented as accepted.
- M8.9: `salt` sits beside `ss01` and Uppercase, Lowercase and AllLetters are optional (guesses on
  the rows); the ligature, fraction and aalt generators are built in M8.11.
- M7.8: `plan::postscript_name` and `plan::file_stem` do not expand text tokens; the `.glyphs` 2
  `InstancePostscript` metric 2^63 fails the instance build. Mac name records drop unsupported
  encodings with a warning; Prevent Name ID adds up across levels (a guess).
- M6.10: corner components cannot be selected or deleted in Edit View (needs a hint selection
  kind); clicking a missing-base placeholder does nothing.
- M13: `Session::commit` does not record interaction commits in the macro recorder. Command Console lines from filters wait for
  M13.3. A URL tab shows one layer per glyph. SFD sources are read by `crates/io-sfd` but not
  yet opened by File › Open or the CLI. Contour names persist only in the native format. The
  Thickness tool has no toolbar button (needs an icon). Metrics-file import, Status Definitions
  and the name-list rename have no dialog yet.

The first waves of M9 to M12, as built (ROADMAP.md › What is real: the first waves of M6 to M12):

| Milestone | Built | Not yet |
|---|---|---|
| M9 | M9.2, M9.3, M9.4, M9.5, M9.6, M9.7, M9.8, M9.8b, M9.9, M9.10, M9.11, M9.12, M9.13, M9.14, M9.19: axes and mappings, glyph sources and compatibility, the variable tables and `STAT`, feature variations, opening variable fonts, the compare commands, static instances between and beyond masters, the variable TrueType compile, variable kerning and marks, alternate layers in variable fonts, the oracle harness for variable fonts, variable feature syntax lowered and compiled (M9.11, M9.12), variable fonts with CFF2 outlines (M9.13) | hints, subroutines and specialisation in CFF2, variable values in table blocks, designspace instance info and `<rules>`, the axis, instance and master commands and the UI |
| M10 | M10.2, M10.3, M10.6, M10.7, M10.8 to M10.14, M10.16, M10.17, M10.20, M10.23: typed hints and settings, the otfautohint port (agrees with the AFDKO 5.0.1 oracle scripts; each master gets an overlap-free outline), the TrueType assembler, `gasp`, hinted previews, node-reference upkeep, CFF hints at export, the TrueType font programs (`cvt `, `fpgm`, `prep`), the touched-state model and 14 instruction-editing commands; M10.18 and M10.19: glyph programs compiled from manual instructions and written into static and variable TrueType exports | PostScript autohinting at export and flex for hand-hinted glyphs (M10.15), the TrueType autohinter (M10.21), instruction deltas in variable fonts; upkeep on background layers and for corner and cap hints |
| M11 | M11.2 to M11.5 (part 1), M11.9, M11.12: palettes, colour layers, path attributes, typed paint attributes in `.glyphs` files, the `SVG ` table at export, images | `COLR`, `CPAL`, `sbix`, `CBDT` |
| M12 | M12.2, 3, 4, 11, 12, 15, 16, 17, 27, 28, 31, 33, 35, 37, 43, 44, 45, 46, 48, 50, 53 to 60: the table tools, 157 checks, profiles and report export, the audit runner over sources, compiled fonts and families (`fontcraft-cli check`), subset by pyftsubset's options | the audit runner's follow-ups above; the other corpus measures |

Every command that takes an `outputDir` creates it when missing. Type 1 PFA files whose
CharStrings open with `dup /CharStrings` after the subroutines read.

Requests the integrations left for the builder packages:

| Package | Request |
|---|---|
| M9.3 | a `checkCompatibility` command and its measure (9 incompatible corpus glyphs; Inter under 50 ms) |
| M9.8 | M11 sorts alternate and colour glyphs together; brace sources for composites of non-exported brace glyphs |
| M9.20 | the Exports tab preview draws interpolated instances; update the `fontInfo.exports.instancePreview` row |
| M9.14 | kerning and mark variations are not read from variable fonts |
| M9.4, M9.5 | vertical phantom points and `VVAR`; `OVERLAP_COMPOUND`; the avar version 2 build (M12.33) |
| M10.2 | the Alignment Zones master parameter is not read |
| M10.19 | the hinted-preview measures of the M10 plan (§ 5) can run against instructed exports; they have not been run. Left in M10.19: `cvar` and deltas for variable fonts, `showPointIndexes` drawing, special layers' own instructions |
| X.suite2, X.suite3 | `craft-quit` is adopted with M14.3 (native macOS quit and window-close handling; the save prompts are ours); `craft-settings` (`config.toml` with backup and recovery, one-time import of the JSON settings file; pairs with the settings window), after the builder packages above (decision 0008) |
| craft-control | watch the suite's open proposal for shared control-port authentication; adopt it for the control channel once it exists (decision 0008) |
| M12.36 | the PDF proof prints through the suite's `craft-print` (decision 0008) |
| M14 | adopt the suite's `craft-ui` (docking, tabs, icon buttons, widget-state theme; decision 0008) |
| M10.10, M10.16 | the counter-hint summary against LanguageGroup; `FunctionInfo` consumes and uncertain fields for M10.17 and M10.22 |
| M11.5b | palette conversion between the version 3 and 4 forms on version change; the UFO palette reader cannot read the version 4 dictionary; full-colour layers with paths as `.colorN` layer glyphs; `PaintColrGlyph`, variable paints and turned gradients |
| M11.8, M11.10 | embedded image and SVG files must sniff as the format their consumer expects (a hostile document's paths can point at any file) |
| M12.44 | `checkOutlines` decimal output and keepContourOrder, `compareFamily -l`; source-field names beyond the common fields; the outline checks' many warnings on decoded compiled fonts |
| M12.17 | `VARC`, compressed `SVG ` documents and composite bitmaps (M12.19c); several pyftsubset options; subsetting sources |
| M13.2 | module commands, export filters and check modules call the runtime; installing a module by dropping a file on the window or File › Open, and web storage of modules, are open |
| M12 checks | the engine's external-validator and source-file services (both answer Unavailable); M12.46's three unregistered checks (misaligned components, missing anchors, marks without attachment) need the components or glyph-data dependency; a font-wide work budget for rasterising in the file checks and for static instancing in the variable-font checks |
| M12 tools | `tables.list` splitting a WOFF2 collection by face; glyph renames in a source's feature code (`font.renameGlyphs` lists the glyphs still named there); commands to delete tabular and redundant kerning pairs and to clear info fields; `fontcraft-cli proof` defaults to PDF and exits with status 4 until M12.36; `metrics --location` waits for M12.20; dehint refuses sources until M10's model is used; outline repairs refuse CFF and `gvar` fonts except rounding |

Open from M10: otfautohint keeps every input point through its overlap union, so a glyph such
as a plus sign is hinted as if its overlaps were still there; what the port should do there is
not settled, and the multi-master overlap test checks only agreement with the oracle.

M6 wave A, as built: `crates/components` (the component index; anchor propagation through nested,
pinned and smart components; automatic alignment, re-run by the engine after each command inside
its undo step; composition recipes from Unicode decomposition and our own tables); smart and
corner components drawn by the model's outline resolution and exported; the `mark`, `mkmk`,
`abvm` and `blwm` writers and `GDEF` (`crates/features/src/marks.rs`). Before the review fixes,
propagation equalled glyphsLib's on 42,734 of 42,736 corpus layers. Two code reviews found
unbounded work and allocation on hostile files, alignment that missed setting changes and pinned
components, and outlines that failed on unfittable corners; all are fixed with tests. Known gaps:
`Font::outline` draws a component pinned to another master from its own master; smart
components where the oracles disagree (`w` in NestedSmartComponent follows glyphsLib, `m` in
SmartComponents follows fontc) wait for the reference app.

Clean-room record (`docs/clean-room.md` › Recording): the M6 planning pass read fontc's
`fontir/src/propagate_anchors.rs` and glyphsLib's `propagate_anchors.py`, whose notes say they
derive from the reference app's privately shared source. Propagation was then written from the
handbook and the oracles' outputs by agents that read neither file; M6.1's agent saw one line of
glyphsLib's propagation code in a crash traceback and used only the fact that it crashes.

Checks for the reference app: `cargo xtask observe` writes `docs/observe.md`, every row the
running app could settle (1,017 rows), built milestones first, grouped by place in the app. Rows
take optional `observe = […]` steps; `observe --check` is a gate step. `cargo xtask testfonts`
writes 45 synthetic `.glyphs` fonts for those checks into the git-ignored `plan/local/testfonts/`.

Open items from M5 (the owner accepted M5 with the speed misses):

- The out-of-sync update after an edit to one glyph takes 7.4 ms on Inter against a 2 ms
  target, and the full scan 53.5 ms against 50 ms.
- 12 metrics-key values in Inter Italic differ from the stored values by up to 1.4 units; the
  cause is not settled.
- Preview Descender defaults to `winDescent`, which departs from the handbook's `winAscent`
  (read as a misprint); Preview Ascender stays at the handbook's 1000 units.
- Text Preview orders mixed-direction runs by simple rules of ours (`textlayout::bidi`); Edit
  View keeps one direction per line until M8. Text Preview's million-character layout measured
  296 to 382 ms under heavy load (target 500; 202 to 213 ms at M5's close).
- Vertical origin and vertical width have no commands and no rows.
- Import Metrics' file choosers.

M5, spacing, kerning and text mode (`docs/plans/M5.md`), as built:

| Done | What it gave |
|---|---|
| M5.0 | spec pass: the rows, the crates registered in the layers, the owner's answers |
| M5.1, M5.2 | `crates/spacing`: metrics keys, sidebearings, the out-of-sync cache, linked masters; the kerning index, groups, exceptions and locks |
| M5.3, M5.4 | the spacing setters, shortcuts, Update Metrics and Transform Metrics; the kerning pair, group, exception and shortcut commands |
| M5.5 | Clean Up, Compress, group rename, the Kerning window's pair list, kerning-text |
| M5.6, M5.7 | `crates/textlayout`; text mode in Edit View, tabs from display strings |
| M5.8 to M5.13 | the Info box's spacing and kerning fields, the Kerning window, Font View spacing, the Preview Area and Panel, the measurement line, Text Preview, sample strings and Find Glyphs |
| M5.14, M5.15 | metrics import and export (kerning-text, AFM, another font); pair kerning recovered from compiled fonts |
| M5.16, M5.17 | linked metrics and Default Layer Width at export; `crates/features`, the generated pair kerning of exported fonts, ported from fontc |
| M5.18 | the § 5 measures (all met but the two out-of-sync speed targets); the kerning lookup measured against both oracles; the spacing and kerning undo sweep; the end-to-end test to the second master |

Left for later milestones: automatically aligned glyphs (M6), shaping (M8), vertical kerning at
export.


M4, Font View and glyph management (`docs/plans/M4.md`), as built:

| Done | What it gave |
|---|---|
| M4.0 | spec pass: the owner's answers in the rows; licence history of GlyphsInfo (MIT until 2024-11-22, BSD-3-Clause after) |
| M4.1 | `crates/glyphdata`: the glyph database and `cargo xtask glyphdata`, cross-checked against fontc and glyphsLib |
| M4.2 | effective properties, glyph order, sorted placement, Last Changed; production names and database mark widths at export; `post` names compared with both oracles |
| M4.3 | 22 glyph property commands, renames that follow references, Find and Replace |
| M4.4 | Add Glyphs, Duplicate, Remove with its answers, Add to Font, whole-glyph copy and paste, Paste Special |
| M4.5 | `crates/glyphquery` and the search and filter commands |
| M4.6 | the settings store and the app's settings file; CLDR language data and glyph sets (`cargo xtask glyphsets`, sources in git-ignored `data-sources/`); `file.newFromGlyphSets` |
| M4.7 | Font View grid, List View, status area, batch targets |
| M4.8 | inspector, Selection Info, glyph and canvas context menus, Info box colours |
| M4.9 | sidebar, search field, filter dialogs, global guides drawn and hit only where their filter allows |
| M4.10 | Add Glyphs, Remove Glyphs, Paste Special and Find and Replace dialogs; Font View clipboard |
| M4.11 | Glyph Info window, Start Window, recent documents in the engine, `glyphData.glyphSets` |
| M4.12 | the § 5 measures, all met; undo-step sweep; the glyph-set end-to-end test |

Left for later milestones: the inspector's metric and kerning fields (M5), Add Glyphs recipes
(M6), images in cells (M11), renaming and dragging filters in the sidebar.

M3, Edit View drawing (`docs/plans/M3.md`), as built:

| Done | What it gave |
|---|---|
| M3.0, M3.0b | spec pass: targets in every M3 command's params, `when` and `context` fields, D13 answered; 5 editor rows confirmed by tutorial videos |
| M3.1 | engine foundations: `params::Targets`, `Session::begin`/`preview`/`commit`/`cancel`, `layer.inspect`, the glyph lock, `geom` segments, grid rounding |
| M3.2 | the canvas split (`shell/`, `canvas/`), display conventions, zoom and scroll, View toggles, tabs, keymap, cached menus; `crates/render`, `ui.screenshot`, `ui.render`; the toolbar with drawn icons |
| Wave 1 | crates without commands: `tools` (M3.3 core), `audit` (M3.17, 34 of 36 outline checks), `io-svg` (M3.8), `pathops` (M3.6) |
| M3.3 | pointer editing in the window: Select, Lasso, Hand, Zoom, snapping, keymap with `when`, selection commands, `ui.pointer`, `ui.key`, `ui.text`, `ui.tool.*`; context menus, menu items greyed out by the selection, `ui.menu.list`, `ui.menu.invoke` |
| Wave 2 | engine commands: M3.4 nodes and Fit Curve, M3.5 transformations, M3.8 clipboard and SVG import, M3.9 guides, measuring and Dimensions, M3.10 background and layers, M3.16 annotations |
| Wave 3 | M3.6 path-operation commands (each layer under a time limit) and the Knife; M3.11 strokes, masks and shape order in the engine; M3.12 the filter framework (`filter.apply`, `filter.list`, Repeat Last Filter) and the six filters |
| Wave 4 | M3.13 Draw, Pencil, Rectangle, Ellipse and shared tool letters; M3.5 Rotate and Scale tools and the bounding box; M3.9, M3.10, M3.16 on the canvas: guides, grid, measurement, the Measurement tool, background, annotations and the Annotation tools; M3.15 Info box, Palette (Dimensions, Fit Curve, Layers, Transformations), bottom bar |
| M3.14 | the dialog framework: a form over a command's parameters with a live preview, OK one undo step, Cancel restores the font; run-on-OK mode, follow-up questions, shaped fields, "…" actions, reorderable lists, multi-choice and folder fields, tabs. Dialogs: each filter, Transformations, Shape Order, Interpolate with Background, Primitive Measurements, Guide Properties, the missing-glyph and pasted-outline questions, Export (OTF and UFO tabs) |
| M3.7 | overlap removal at export: the compiler's outline stage, each glyph under a time limit, the `removeOverlap` option, coverage-equal comparison in the oracle |
| M3.8 (window) | the system clipboard in Edit View; system choosers for the export folder and File › Import › Outlines |
| M3.9 (window) | guides dragged and edited on the canvas, guide context menu items |
| M3.10 (window) | the active layer (`layers.select`), the tab's master (`view.selectMaster`, Cmd+1 to Cmd+9), Edit Background (Cmd+B) |
| M3.11 (rest) | strokes and masks drawn on the canvas and applied at export; the Palette's Stroke section |
| M3.17 | audit commands (`audit.run`, `list`, `describe`, `results`, `fix`, `profile.list`), eight fixes that each clear their check, `path.addInflections`, Tidy up Paths' `straighten`; live audit and canvas markers; `cargo xtask audit` |
| M3.18 | the § 5 measures, all met (184 of 184 commands live); the end-to-end test (an agent draws H, O and n over the control channel and exports them) |

Moved to `fidelity`: `path.sharpenCorner` (no source). Left for later milestones: text mode
(M5), the toolbar's master icons (M9), the Export dialog's
Variable (M9) and Metrics (M5) tabs.

Decisions that hold:

- linesweeper 0.5.0 stays, from crates.io, and its rare hang and failed assertion are not
  reported upstream for now (owner's decision, plan § M3.6 "Spike result"). Callers that run
  without a person waiting (export, batch commands) run each glyph under a time limit.
- Behaviour not yet seen in the reference app or a video follows the handbook and is the
  default; the row's `notes` say it is documented but not observed. Example: Remove Overlap
  deletes open paths and stray nodes.
- Points a command creates or moves round to the font's grid (`path.addPath` included).

M1 and M2 are done (`docs/plans/M1.md`, `docs/plans/M2.md`). M2 left on purpose: CFF subroutines (done at export in M12.26), the compile plan's `features` section, the CLI's `--name-by`, `--output-layout` and pattern
instance selection, and one reader case (a designspace master missing a metric before one it
states reads it as 0). `fontcraft-cli oracle --corpus` (fontc, or fontmake with `--format otf`)
measures the compiler; `crates/oracle/allow.toml` holds every accepted difference with its reason.

Before you start: `cargo xtask corpus --fetch` (about 300 MB) if `corpus/` is missing, and the
oracles in `docs/testing.md` (fontc via cargo, fontmake via a Python tool installer).
`cargo xtask roundtrip` shows where the round trips stand; `cargo xtask overlap` measures
overlap removal on the corpus; `cargo xtask audit` runs the outline audit on it.

## What the planning session decided

See `docs/decisions/`. In one line each: Glyphs 4 and AFDKO are the primary references; our own
compile pipeline with fontc as oracle and bootstrap backend; a one-document model shaped like the
primary's format with our own `.glyphs` reader and writer; commands with explicit targets and
atomic failure; WebAssembly plug-ins and no Python; no compiled fonts in the repository.

## Known weak spots

- **The inventory is unreviewed line by line.** It passed mechanical checks and one sample audit
  (209 handbook rows re-read by a second reader: 1 wrong, 14 imprecise, all corrected; patterns in
  `docs/spec/README.md`). Duplicated meaning across files is likely (open decision D2 lists known
  pairs).
- **6 rows are from memory** (`docs/parity.md` › By source): 3 in `00-…`, 2 in `90-…` and 1 in
  `07-…`. Verify or drop them before implementing.
- **The reference app has not been run.** 62 rows are `observed`: 57 compiled-output rows
  checked against fontc, fontmake and makeotf, and 5 editor rows confirmed by public tutorial videos.
  About 450 open questions are listed at the ends of the notes.
- **No M3 behaviour has been compared with the reference app.** Tools, panels and dialogs
  follow the handbook; 5 editor rows are confirmed by tutorial videos.
- **M3 gaps** (ROADMAP.md › What is not): `path.sharpenCorner` waits in `fidelity`; Font View draws
  the first master only (M4); the Import Outlines
  missing-glyph question is asked once for all unmatched files, not once per file; hint upkeep
  (M10.3) and Shape Order renumber hints that refer to a path by index, but upkeep does not cover
  background layers or corner and cap hints; a UFO keeps no stroke
  attribute or gradient (a save reports them), and cap code 4 and the forced cap axis are unconfirmed against the reference's
  files.
- **linesweeper can hang** on rare inputs (plan § M3.6 "Spike result"). The path-operation
  commands and export run each glyph on a worker thread under a time limit; a glyph that runs
  out of time stays as it was (at export, it keeps its overlaps, with a warning), and its worker
  cannot be stopped and runs on detached. The 10,000-case random-path test in `crates/pathops`
  is ignored in the gate.
- **The compiler writes static fonts of both flavours and variable fonts with TrueType or CFF2 outlines.** TrueType and
  CFF, plain, WOFF and WOFF2; the core tables match fontc and fontmake over the corpus (with
  allow-listed causes: bracket and colour glyphs, the glyph database, smart components); overlaps
  are removed, strokes expanded and masks applied at export; `gasp`, `GDEF`, the mark features,
  compiled feature code and the `SVG ` table are written. Variable TrueType fonts match fontc's in 103 of
  158 sources and fontmake's in 106; variable CFF2 fonts match fontmake's in 107 of 158. TrueType exports carry the glyph programs of manual instructions or the autohinter's, and variable ones carry `cvar` (the handbook says the reference ignores instruction deltas in variable fonts). Colour fonts carry `COLR` (versions 0 and 1, also variable), `CPAL`, `sbix`, `CBDT` and `CBLC` (through `Options::cbdt`, the export commands, the Export dialog and the CLI) and `SVG `. CFF2 exports are specialised, carry blended hints (special layers included) and are subroutinised when smaller, as static CFF exports are
  (`docs/plans/M2.md` § 5, `docs/plans/M3.md` § 5, `docs/spec/40-compile.md` § 4).
- **Most M10 to M12 corpus measures have not been run.** The audit runner (M12.44), subset
  (M12.17), the TTX reader and writers, instance, partial instances, merge, transform, the `variations` tools, the CFF subroutiniser, convert, project compile, CID-keyed CFF, the compile extras, `optimize`, TTX `mort` and `morx` and the UFO colour layers have; the other packages were tested against small outputs of the oracles, and each
  plan's § 5 lists the corpus runs still owed.
- **`docs/spec/20-cli.md` is very long** (about 26,000 words). Read the section for the subcommand
  you are building, not the whole file.

## Known limits in the code

An adversarial review of the skeleton reported 31 findings. Most were fixed in the same session
(coordinate and point-count bounds before the glyph writer, glyph-name sanitising for `post`,
style linking, bounded reads on every channel, protocol id checks, the indexing lints, a
virtualised Font View). The first two rows below were left on purpose; the reviews of the M9 to
M12 first waves left the last two for the packages named. None can lose data.

| Limit | Where | Fix it in |
|---|---|---|
| Each command's rollback snapshot copies every document's history list (pointers and labels, no glyph data) | `crates/engine` | when it matters |
| The vendor-names scanner does not see through `concat!` or string escapes | `xtask/src/brands.rs` | when it matters |
| No font-wide work budget for rasterising in the file checks, for drawing every glyph in the static-instancing check, or for the CFF glyph reading each hinting check repeats | `crates/audit` | a check package |
| An embedded image or SVG file is not checked to be the format its table expects | `crates/engine` (assets) | M11.8, M11.10 |

## Waiting on the owner

The open decisions of the second planning pass: `docs/plans/depth.md` › Open decisions (eight questions; the
packages that wait on them are listed there).

`docs/spec/00-open-decisions.md` › For the owner: the name (O2), access to the sibling apps' shared
standards (O3), the application id (rest of O4), FontLab (O5), platforms (O6). O1 is answered (no
licence; tutorial videos watched by the owner) and so is the repository's home.

Wave 1 needs a build on the Mac: Cmd+Q and the app menu's Quit with unsaved work, the File › Open… chooser and
dropping a file on the window (see "Where to start", item 1). X.pdf3 needs the same build on the Mac and one on
Windows for the clipboard's PDF.

One question from the sixteenth batch:

- **The suite's token-authenticated control protocol.** The craft-next control server requires a token; FontCraft's
  control channel has none, and the new window keeps FontCraft's transport so MCP attach works unchanged. Adopting
  the suite's protocol changes the protocol and the MCP backend. Adopt it, and when?

MCP and the control channel are unrestricted unless `--automation-read-root` and
`--automation-write-root` are given, as in the sibling apps (owner's decision).

## Dev log

Append one terse entry per session: what landed, numbers, what is still open.

- **2026-10-07, planning session.** Read VectorCraft and PhotoCraft conventions. Chose
  references. Spiked dependencies: 14 font crates check for native and wasm; fontc compiles an
  in-memory `.glyphs` source to a variable font in 21 ms as a library. Inventory compiled by 17
  parallel readers (3,618 rows, 22 notes), deduplicated, checked for verbatim overlap with the
  handbook, then audited in a sample and fact-checked against the code. Skeleton built: 8 crates,
  2 apps, xtask; 27 commands; 78 tests; gate green. Found and fenced a panic in the glyph writer
  for points more than 32,767 units apart; fixed most of a 31-finding code review. Still open:
  everything in "Where to start" and the table of known limits.
- **2026-10-07, set-up on the build box.** History rewritten to the owner's identity; repository
  pushed (private). Gate green locally (4 min 17 s). O1 (no licence; tutorial videos) and O4
  recorded. Corpus fetched, oracles installed. M1 plan drafted: 7 packages. No version 4
  `.glyphs` file in the corpus. The owner accepted the plan.
- **2026-10-07, M1.0 and M1.1 (OpenStep).** Version 4 corpora pinned (16 files); sparse corpus
  fetch; `cargo xtask roundtrip` (527 sources). Spikes: no existing crate round-trips any file
  (plan § 8); own plist crate, own UFO I/O recommended (owner asked). `crates/plist`
  OpenStep reader and writer: 215 of 274 `.glyphs` sources identical, every reference-written
  one outside fontc's test data. Next: the model (M1.2); the XML plist after question 11.
- **2026-10-08, M1.2, M1.3, M1.5, M1.6 (part).** Model restructured (tagged `Extra`, `Arc` parts,
  glyph index, metric definitions, layer ids, kept transforms, kerning, axes, instances, features,
  file store); native format version 2. `crates/io-glyphs`: versions 2 to 4 and packages, every
  reference-written corpus source byte-identical; package side files are model data, so a package
  and a single file of the same font read the same. `crates/decompile`: all 342 corpus binaries
  open. `file.open`/`file.save` on every source and compiled fonts. Oswald shown in the app.
  Next: question 11, then UFO; meanwhile the native format's layout and the CLI's
  inspection commands (both done since). M2 plan drafted (`docs/plans/M2.md`), waiting for review.
- **2026-10-08, M1.4.** Owner chose our own UFO I/O (decision 0003 amended) and accepted the M2
  plan. `crates/plist`: lossless XML tree and XML property lists. `crates/io-ufo`: GLIF, UFO 2
  and 3, designspace, UFOZ read; kept spellings used only while they still mean what the model
  holds. Corpus: 19,453 GLIF, 125 UFOs, 59 designspace documents with their UFOs, all identical
  through the model and through `.fontcraft`. Engine: open, save, save as, `file.exportUfo`;
  CLI `convert` to `.ufo`/`.designspace`. Oswald converted to designspace and compiled by fontc
  matches its `.glyphs` build in 822 of 850 glyphs at ExtraLight (the rest explained). Open:
  glyphsLib lib keys, non-master layers to UFO, UFO 2 kerning groups (plan § M1.4).
- **2026-10-08, M1.7, the typed model.** Steps 1 to 8c: font information, axes, masters,
  instances, glyphs, layers, shapes, kerning context, groups and feature markers are model
  fields in `.glyphs` 2 to 4, UFO, designspace and the native file; glyphsLib's lib keys carry
  Glyphs-only data; intermediate layers are sparse sources and alternate layers bracket glyphs
  with rules. Census: `.glyphs` keys in `Extra` from tens of thousands of entries to stored
  values, undocumented keys and hand-made spellings. Measures: round trips unchanged (`.glyphs`
  108/1/132/4, packages 22/0/7/0, UFO 125/15, designspace 59/53); 35 of 35 reference-written
  sources through a designspace and back (new test); fontmake from our Oswald designspace:
  4,614 of 4,626 static glyphs and 882 of 883 variable-font glyphs equal, names and OS/2 equal
  but `xAvgCharWidth` ±1 (non-spacing mark widths need the glyph database, M4). Fixes from the
  measures: slanted component matrices decompose; version 2 name parameters, instance names and
  palettes; virtual masters widen axis ranges; loss reports list the data UFO file records hold,
  not the records. Next: M2.0 and M2.1.
- **2026-10-08, M2.0, the oracle harness.** New crate `crates/oracle` (L3) and `fontcraft-cli
  oracle` (`--corpus`, `--against fontc|fontmake`, `--format ttf|otf`, `--json`): both builds
  static at the same master, fixed timestamp, read back and compared per table, normalised;
  source-reading differences apart; allow-list with reasons. Corpus set 354 sources: fontc built
  334, fontmake 330 (TrueType) and 331 (CFF), FontCraft 313; 300 compared with fontc (per-table
  numbers in the plan). glyphsLib does not read `.glyphs` version 4 (8 of 16 files, none with
  outlines); fontc writes vendor `NONE`. 24 tests (20 in the gate, 4 opt-in). Next: M2.1.
- **2026-10-08, M2.1, pipeline and resolved projection.** `Font::resolve(&Target)` in the model
  (a master, or an instance at a master; between masters an error until M9); `crates/compile`
  split into preflight, order, outline and build stages. Missing components dropped with a
  warning, non-exported glyphs inlined, cycles and depth refused, `.notdef` drawn as fontc draws
  it, composites kept (mixed glyphs and transforms beyond ±2 decomposed), cu2qu at 0.001 em,
  contours reversed, half-up rounding, composite boxes and `maxp` limits. Oracle against fontc:
  FontCraft builds 347 of 354 (was 313), 332 compared (was 300); glyf identical 218 and
  equivalent 18 (was 2 and 0), head 277 (22), maxp 275 (58), hmtx 171 + 132 (1 + 14). 406 tests.
  Next: M2.2.
- **2026-10-08, M2.2.** Core tables derived as fontc 1.0.0 derives them (`crates/compile/src/info.rs`,
  `ranges.rs`, `build.rs`): metric defaults per format at resolve time, names and fallbacks,
  style flags, vendor `NONE` (plan § 3 question 6 answered so), fsType, weight and width class,
  Unicode and code page ranges, `USE_MY_METRICS`, hmtx run compression, vhea and vmtx. Readers:
  version 2 vertical metrics no master states stay undefined, unusual code point and property
  spellings read, `*origin` anchors applied; the binary reader takes the typographic ascender
  and descender as the master's. Harness: vhea and vmtx by field, allow-list with
  several keys, sources and `*`. Against fontc over 332 sources every core table identical,
  equivalent or allowed; round trips: glyphs 122 identical (was 108). Open: four reader
  questions (see Where to start).
- **2026-10-08, M2.3a.** New standalone crate `crates/cff`: CFF tables from plain data, porting
  fontTools' number encodings, INDEX, standard strings and charsets, DICT order and defaults,
  charstring pen rounding, specialiser, width optimiser, BoundsPen FontBBox and the offset
  passes. Cross-check against ufo2ft at `--optimize-cff 1`: 115 of 115 fonts byte-identical
  (17,896 glyphs; tolerances 0, 0.3, 0.5). 45 tests, read back with read-fonts. Not yet used by
  the compiler (M2.3b).
- **2026-10-08, M2.3b.** `Flavour::Cff` builds static OTF: `crates/compile/src/cff.rs` fills
  the `cff` crate as ufo2ft does (names, blue zones and stems from the new `Resolved::zones`,
  `stems_h`, `stems_v`, widths); `maxp` 0.5, `post` 3.0, metrics from charstring bounds.
  Against fontmake over 331 sources: CFF 195 identical / 125 equivalent / 10 allowed / 1
  different; 347 OTF and 347 TTF read back by fontTools without error. Allow-list entries can
  name values (`ours`, `theirs`); fontmake/fontc disagreements allow-listed. fontc and round
  trips unchanged. 8 new tests. Export commands still TrueType only (M2.5).
- **2026-10-08, M2.4a.** New standalone crate `crates/container`: WOFF 1 and untransformed WOFF2,
  both directions, porting fontTools' sfnt.py and woff2.py choices (table order, checksums,
  zlib 6, Brotli 11/22 font mode, DSIG dropped, head bit 11). Corpus round trip 342 of 342 for
  both; fontTools reads all 340 TTF/OTF wrappings back to its own tables; byte identity WOFF 0
  (miniz_oxide is not zlib), WOFF2 19, the rest equal apart from the compressed streams.
  Transformed WOFF2 and collections refused with `Unsupported`. decompile's `woff.rs` moved
  into the crate. 12 new tests. Round trips unchanged.
- **2026-10-09, M2.4b.** WOFF2 `glyf`/`loca` (default) and `hmtx` (on request) transforms, encode
  and decode, ported from fontTools; `file.open`, `font.inspect` and `tables.list` read WOFF2.
  Corpus round trip 342 of 342 for WOFF, WOFF2 untransformed, default and with `hmtx`; 49 of 49
  corpus WOFF2 files read; byte identity with fontTools 21 of 340 (default) and 24 (with `hmtx`),
  the rest differ only in the Brotli stream; fontTools reads all 340 of ours; ours reads
  fontTools' 340 and 339 of 340 (fontTools shortens `hmtx` in Oswald-Stencil).
- **2026-10-09, M2.6.** Follow-ups of M2.2 and M2.3b: no core table differs from fontc or from
  fontmake (TTF and CFF) over the corpus; the rest allow-listed with reasons. fontmake CFF
  byte-identical 195 to 206. The four reader questions answered (`docs/spec/30-source-formats.md`
  › Values a source leaves out). Open: a designspace master missing a metric before one it
  states; alternate and colour glyphs (M9, M11).
- **2026-10-09, M2.5.** `file.exportStatic` (instances at masters, TrueType and CFF, plain, WOFF,
  WOFF2), `file.exportAll`, `font.compile`, `font.compilePlan`, CLI `compile` (`--plan`),
  `convert` to otf, woff, woff2. A failing export writes nothing. Instances between masters
  skipped (M9), overlaps kept with a warning (M3). Not built: the plan's `productionNames` and
  `features` sections, `--name-by`, `--output-layout`, `familyName`, `testInstall`, pattern
  instance selection.
- **2026-10-09, M2 closed.** § 5 measures: core tables and CFF outlines as in M2.6; the 347
  buildable comparison sources give 551 TTF and 551 OTF (masters and instances), all read back by
  skrifa and fontTools without error, all 1,102 round trip through WOFF2 (default and `hmtx`) to
  the same tables, and the 1,102 WOFF2 files `compile` wrote decode to the plain fonts' tables.
  New `crates/engine/tests/engine/determinism.rs`: two sessions, 12 files (TTF and OTF, plain, WOFF,
  WOFF2), byte-identical. Next: M3's planning pass.
- **2026-10-09, M2.7.** Owner decision: a font with no family name (42 corpus sources) is
  exported as `<source file name>-<style>.ttf` instead of fontc's `-<style>.ttf`; only the file
  name, the `name` table keeps the empty family. Smart components can be written to a UFO or
  designspace as outlines (`file.saveAs` `smartComponents`, `convert --smart-components`,
  `file.exportUfo` by default), ported from glyphsLib with fontTools' `VariationModel`
  (`crates/geom/src/variation.rs`, `crates/model/src/smart.rs`): the 10 glyph layers with smart
  components in 7 corpus sources match glyphsLib's points exactly. Follow-up: the compiler can
  draw smart components with the same function and drop their allow-list entries.
- **2026-10-09, M3.0.** The owner accepted the M3 plan. Spec pass: the 42 secondary-reference
  rows moved to M13; super-smooth nodes, pen points, Erase and Select All Layers (12 rows) to the
  new milestone `fidelity` (the interaction-fidelity pass), each with its reason in `notes`; the
  image formats to M11; the export dialogs and `export.removeOverlapOption` from M2 to M3; the
  four `memory` M3 rows settled. New rows `layer.inspect`, `path.addInflections`, `tool.select`,
  `tool.list`. Every M3 command's `params` name their targets (decision 0004 › How commands name
  their targets). The schema gained `when`, `context` and `notes`; `cargo xtask spec` requires a
  `when` on every shortcut without `Cmd` or `Ctrl`. D13: a row a tutorial video confirms is
  `observed`. The § 3 answers are in the notes' open questions. M3 now holds 447 rows (176
  commands, 7 live). Next: the M3.6 spike, then M3.1.
- **2026-10-10, M3 waves 1 and 2, M3.1, M3.2.** M3.6 spike: linesweeper 0.5.0 unions all 2,368
  corpus layers with closed paths to skia-pathops' coverage; kept from crates.io with input
  rules and a per-glyph time limit, no upstream report (owner). Wave 1 crates: `tools`,
  `audit` (34 of 36 outline checks), `io-svg` (deep nesting fenced before the XML parser),
  `pathops`. M3.1: targets, interactions, `layer.inspect`, glyph lock, segment geometry.
  M3.2: `render`, `ui.screenshot`, `ui.render`, canvas and shell split, tabs, zoom, cached
  menus. Wave 2: node, transform, clipboard, guide, measure, Dimensions, background, layer and
  annotation commands. Remove Overlap deletes open paths and stray nodes, as the handbook says.
  679 tests; 142 of 929 commands live (M3: 114 of 181). Next: the M3.3 wiring.
- **2026-10-10, M3 waves 3 and 4, the dialog framework.** Path-operation commands on `pathops`,
  each layer under a time limit, and the Knife; strokes, masks and shape order in the engine;
  the filter framework and all six filters; dialogs with live preview and Cancel, one per filter
  from its schema; Draw, Pencil, Rectangle, Ellipse, Rotate, Scale, Measurement and the six
  Annotation tools (18 tools in the window); the bounding box; guides, grid, measurement,
  background and annotations on the canvas; Info box, four Palette sections, bottom bar. 854
  tests; 200 of 932 commands live (M3: 172 of 184); about 102,600 lines of Rust. Open: audit
  commands and the other dialogs (on branches), M3.7, strokes and masks on the canvas and at
  export, live audit, context menus, M3.18.
- **2026-10-10, M3 close.** Since waves 3 and 4: M3.7 overlap removal at export, each glyph
  under a time limit; strokes and masks drawn and exported, the Stroke section; the rest of the
  dialog framework and the Transformations, Shape Order, Guide Properties and Export (OTF, UFO)
  dialogs; the toolbar, context menus and menu greying; the active layer, master switching and
  Edit Background; live audit, markers and the audit commands; the system clipboard. § 5
  measures: all met but "commands live", 184 of 185 (`path.sharpenCorner`: no source; moving it
  to `fidelity` awaits the owner). Overlap removal: 2,368 of 2,368 corpus layers within the
  coverage rule, median 38 µs; against fontmake 99.58% (TrueType) and 99.87% (CFF) of glyphs
  without an overlap-removal difference. Node-drag preview step on Oswald 142 µs. Audit over 348
  corpus sources: 0 failures. Round trips and oracle equal the baselines. 931 tests; 213 of
  932 commands live; about 113,000 lines of Rust. Next: the owner's review of M3, then M4's
  planning pass.
- **2026-10-10, owner's review of M3.** The owner accepted the agents' choices where the
  handbook is silent, except Extrude's export string: Extrude has none until a source shows
  one (`filter.apply` refuses it). `path.sharpenCorner` moved to `fidelity`, so M3 is closed
  with 184 of 184 commands live. Every choice of ours where no source says what the reference
  does is a row's `guesses` entry; `cargo xtask guesses` writes `docs/guesses.md` (203 on 160
  rows) and fails when a live `inferred` or `memory` command has none; `own = true` marks rows
  with no reference counterpart. 934 tests. Next: M4's planning pass.
- **2026-10-10, M4 planning pass.** `docs/plans/M4.md` proposed: 244 rows (61 commands, 6
  live; 155 behaviours, 24 panels); 19 rows moved in (production names from M2, `glyphOrder`
  from M7, settings and recent documents from M14); no `memory` rows left; 16 of the Font View
  note's 20 open questions answered. Waits for the owner.
- **2026-10-10, M4 wave 1.** Plan accepted. M4.1: `crates/glyphdata` (GlyphsInfo 1f126bd, AGL,
  UCD 17.0; 538 KB blob; `cargo xtask glyphdata --fetch|--generate|--check`): equal to fontc's
  glyphs-reader on category, subcategory and production name for 200,504 names; glyphsLib
  differences allow-listed by cause. M4.2: effective properties, `glyph_order`, sorted placement
  (`insert_by_sort_key`), Last Changed; production names at export (fontc's cleaning), database
  marks at advance 0, File › Export refuses invalid names; `post` names equal fontc's and
  fontmake's except CornerComponents (Use Custom Naming, allow-listed); the M4 marks allow-list
  is gone. M4.3: 22 glyph property commands, renames that follow references, Find and Replace
  with regex. M4.4: Add Glyphs, Duplicate, Remove with its answers, Add to Font, whole-glyph
  copy and paste, Paste Special. M4.5: `crates/glyphquery` (search, smart and list filters,
  predicates, sidebar entries, missing glyphs) and 11 commands. M4.6: settings store and the
  app's settings file. 1,021 tests; 252 of 932 commands live (M4 45 of 61).
- **2026-10-10, M4 close.** Wave 2 (M4.8 to M4.11) built in worktrees and integrated; the dialog
  framework gained multi-line text, a third button, scrolling lists and a rule-tree control;
  engine follow-ups: Match Case, several tags in one step, recent documents in the engine, an
  empty glyph data search, `glyphData.glyphSets`, readable script names. M4.12: every § 5 measure
  met (62 of 62 commands; glyph database equal to fontc's; `post` names equal both oracles'
  except one allow-listed source; dialogs equal commands; the glyph-set end-to-end test; on Inter
  a Font View frame under 1 ms). 1,076 tests; 269 of 933 commands live; about 138,000 lines of
  Rust. Next: the owner's review of M4, then M5's planning pass.
- **2026-10-10, M4 review and M5 planning.** The owner's answers to M4's guesses: hidden selected
  glyphs are deselected; a rename takes the database's Unicode value; generated glyphs are 600
  units wide; removing a glyph keeps the kerning pairs that name it, as the handbook says
  (M4.16). M4.14: Remove Glyph's dialog defaults
  to Decompose. M5.0: the planning pass (222 rows, 97 commands; 27 rows
  moved; 19 packages; 463 guesses on 328 rows). Next: the owner's acceptance of the M5 plan.
- **2026-10-11, M5 started.** The owner accepted the M5 plan: the pair-kerning writer comes into
  M5 (M5.17), tabs open from the document's display strings, Text Preview uses our own layout.
  M5.0: the answers and the handbook's kerning details in the rows (5 kern-export rows moved from
  M8; 463 guesses on 328 rows).
  M5.1: `crates/spacing` parses metrics keys, measures sidebearings (italic masters along the
  angle, as the handbook says), evaluates keys with limits, scans for out-of-sync keys with a
  cache, orders Update Metrics and resolves linked masters. Corpus: 4,413 of 4,413 keys parse;
  17,217 master values equal the stored metric; 12 Inter Italic differences up to 1.4 units are
  not explained. Open: the cache update after editing a much-used glyph takes 7.7 ms (target 2).
  M5.2: the kerning core (sides, lookup with its step, locks, groups); lookup over Inter Regular's
  490,911 covered pairs takes 52.6 ms. M5.3–M5.5: 48 spacing and kerning commands (setters with
  keys, shortcuts, Update Metrics, Transform Metrics, Default Layer Width, pairs, exceptions,
  groups, Clean Up, Compress, rename, the pair list, kerning-text copy and paste). Update Metrics
  orders composites after their bases and settles Inter in one run (about 130 ms). Compress
  follows stored order so the handbook's example settles. M5.6: `crates/textlayout`; 45 of 45
  corpus display strings round trip; 10,000 items lay out in 2.15 ms. M5.15: kerning recovered
  from compiled fonts, 945,310 covered pairs in 97 fontmake builds with 0 differences.
  317 of 933 commands live.
  M5.fix1: seven review fixes (a work budget on the GPOS script walk, closing one kerning lock,
  control characters in kerning sides, duplicate master entries, Compress with one group in two
  spellings, GPOS classes past the count, paste values like Set Pair). M5.kernmerge: loaders keep
  one kerning entry per master. M5.16: exported masters that link their metrics take the linked
  master's widths (17 of 17 master builds equal fontmake's hmtx and vmtx); new glyphs take
  Default Layer Width (1000 for CJK, 200 for `_corner.` glyphs without the parameter).
  M5.7: text mode in Edit View. Tabs hold typed text (cursor, selection, direction, kerning
  mode); the Text tool (T), typing, cursor keys, Escape between text entry and editing,
  Cmd+Space panning, copy and paste as text; the canvas draws the caret, the selection, kerned
  gaps (negative light blue, positive yellow) and metrics labels; clicks place the cursor; the
  bottom bar's direction and kerning-mode buttons; spacing and kerning shortcuts act at the
  cursor; tabs open from the document's display strings and are written on save. The § 5
  end-to-end test runs up to the Kerning window step. A frame with 10,000 items takes about
  8–15 ms (target 16). 329 of 933 commands live.
  M5.fix2: tabs belong to their document (each saves only its own display strings); the text
  layout follows a picked layer. M5.8–M5.14: the Info box's spacing and kerning fields (numbers
  and keys, out-of-sync red with an update button, unlinked turquoise, kern fields and locks),
  the Kerning window (search, sort, in-place editing, copy and paste between masters), the rename
  and paste-conflict dialogs, Show Group Members, Font View's editable metric and group fields
  and out-of-sync badge, three smart-filter rules, the Transform Metrics dialog, the Preview Area
  and Panel, the measurement line, Text Preview (1,000,000 characters in about 360 ms), sample
  strings, Find Glyphs and `glyph.search`, metrics import and export (kerning-text; AFM read back
  by afmLib both ways on Inter). M5.syncfast: after a rename the out-of-sync cache rescans only
  the glyphs that name it; Font View frames after an edit on Inter take 6–10 ms. Open: the cache
  update after editing `n` takes 7.2 ms (target 2). 365 of 933 commands live; M5's 97 all live.
  M5.fix3: nine review fixes (Replace All and AFM expansion refused before allocating; AFM import
  leaves italic masters unchanged; nested composites import after their components; menu
  shortcuts work while a list has the keyboard; Selected Glyphs works in Edit View tabs; the Find
  Glyphs cache follows the document revision; dialogs open during a drag).
  M5.17: exported fonts get generated pair kerning (GPOS `kern` and `dist`) for sources without
  feature code, ported from fontc 4504578 into `crates/features`, following the ufo2ft
  feature-writer setting. Against the oracles on the 37 corpus sources without feature code:
  129 pairs each, 0 differences with fontmake, 1 known difference with fontc (written in
  `docs/spec/40-compile.md` § 4a). Sources with feature code (21 kerned ones) get no generated
  kerning until M8.
- **2026-10-11, M5.18, M5's close.** The kerning lookup against both oracles
  (`index_lookup_equals_both_references`): 54 sources without `kern` feature code,
  506,285 pairs per oracle, 1 difference (`FeatureWritersEmpty.glyphs` turns its kern writer
  off); 5 sources with one, 5 pairs, 1 difference (the `kern` feature in `FeaInc-Bold.ufo`'s `features.fea`). Round
  trips equal the M4 baselines (glyphs 122/1/118/4, packages 22/0/7/0, UFO 125/15, designspace
  59/53). New sweep: 45 undo runs over 43 spacing and kerning commands, 11 refusals pointer-equal.
  The end-to-end test reaches the Kerning window, Compress and the import into a second master.
  Speed on Inter (load 14 to 19): out-of-sync full scan 53.5 ms (50), after editing `n` 7.4 ms
  (2), both missed; every other target met. Inter's `Tcommaaccent` (U+0162) and `uni021A`:
  the references write `uni021A` and `uni021A.1` because they ignore Inter's `disablesNiceNames`;
  FontCraft keeps the source names as documented. M5 is done but those two speed targets.
- **2026-10-11, M5.fix4 and M5.fix5, review fixes on M5's close.** M5.fix4: generated kerning
  unless the source has a hand-written `kern` feature; a dialog that does not open leaves the
  drag running. M5.fix5: a `kern` feature is found with `useExtension`, and a kern block with only
  comments does not count; a top-level `include` in enabled feature code leaves the generated
  kerning out with a warning naming the file (a guess on `kerning.implicitKernFeature`, a row in
  `docs/spec/40-compile.md` § 4a); class-table cells are counted per subtable from the builder;
  the writer's caps are 500,000 glyph pairs and 1,000,000 class cells per lookup, 2,000,000 in
  all (Inter, the largest corpus font: 11,442 and 25,317); Targeted dialogs build their fields
  with the drag left running; both measures fail when a known difference's count changes.
  Measures rerun in release: export half, 54 sources (5 with `kern` feature code not compared,
  the same as before), 506,285 pairs per oracle, 27 known differences (fontc 1 `hebr` in
  `KerningRtlOnlyMaster.glyphs` and 13 `cyrl` in `Oswald.glyphs`, fontmake the same 13), none
  other. Lookup half, 59 sources: 54 without `kern` feature code, 506,285 pairs per oracle, 1
  differs per oracle (`FeatureWritersEmpty.glyphs`); 5 with it, 5 pairs, 1 differs per oracle
  (`FeaInc-Bold.ufo`).
- **2026-10-11, M5 accepted, M6 planned.** M5.fix6: Preview Descender defaults to `winDescent`;
  Text Preview keeps left-to-right runs in order in right-to-left text (`textlayout::bidi`,
  Writing Direction ranges as embeddings). M6.plan: `docs/plans/M6.md`, 220 M6 rows (59
  commands), no recalled rows left, 11 moves, 48 new guesses (675 on 434 rows). Open: the
  owner's answers to the plan's § 7.
- **2026-10-11, review fixes.** M5.fix7: Text Preview's trailing blanks take the paragraph level
  before their kerns, so a line no longer shifts by a kern; embeddings are taken in order of their
  start. The M6 plan asks the owner how to build anchor propagation (fontc's code derives from the
  reference app's private source); `docs/clean-room.md` gains the rule for such code. 677 guesses
  on 435 rows.
- **2026-10-12, the first waves of M9 to M12 integrated.** M9: axes and mappings, glyph sources
  and compatibility, `fvar`, `avar`, `HVAR`, `VVAR`, `MVAR`, names and `STAT` byte-equal to
  fontc's on test sources, feature variations, variable fonts opened, the compare commands. M10:
  the otfautohint port with its reports, typed hints and settings, the TrueType assembler,
  `gasp`, hinted previews. M11: palettes, colour layers, path attributes, images, the `SVG `
  table. M8.16: snippets. M12: the table tools and CLI subcommands, 157 checks, profiles and
  report export. Review fixes on M7 (linked sources read once per export, decomposition and
  pattern-test budgets, exports kept inside their folder, Paste Rows spellings) and on M9 to
  M12: named limits for variable-font layers and avar points, mapping sizes, autohinter element
  walks and segment counts, alignment zones per master, name records per font, interpolation
  checks per glyph, layout tables before expansion, font-wide charstring and dehint work, TTX
  output, collection table sizes, unique-name suffixes and component depth; image paths across
  Save As and undo, network and device paths refused. `outputDir` is created when missing.
  2,463 tests, 29 ignored; 571 of 949 commands live; 1,406 guesses on 984 rows; 1,017 observe
  rows; round trips unchanged (`.glyphs` 122/1/118/4, UFO 125/15). Colour-layer update after a
  one-glyph edit on Inter: 35.5 µs. Open: the builder packages and their requests (Where to
  start), the corpus measures, the branch with M6 to M8's second waves and M13's first wave.
- **2026-10-12, batch 4 and the automation roots.** Integrated: M6.14, M7.14, M7.15 to M7.18
  (the Font Info window), M8.15, and all 22 first-wave packages of M13 (tension, helpers, stem
  filters, the Thickness tool, expressions, Detect Composites, stored selections, status
  definitions, export profiles, external-change review, encoding tools, PFM and AFM import, UFO 1,
  EPS and SFD reading, the Command Console, macros, the URL scheme, MCP permissions, prompts and
  resources). Fix passes bounded hostile-input work in the EPS, SFD, UFO 1 and PFM readers,
  features previews, macros, outline matching, intersections and stem filters, made multi-profile
  exports all-or-nothing, bounded name-table reads, and made `avar` segment maps for axis maps
  outside the axis range match fontmake (`docs/spec/40-compile.md`). MCP and the control channel
  take `--automation-read-root` and `--automation-write-root`; files are unrestricted without them.
  2,781 or more tests pass, none fail; all eleven gate steps pass; 658 of 960 commands live; 1,658
  guesses on 1,079 rows; 1,097 observe rows. Open: the Lows listed under Where to start, the
  Detect Composites base choice (45.4% against 95%), the per-edit alignment pass (2.4 ms against
  2 ms), the skeleton pass for waves 2 to 5, and the builder packages.
- **2026-10-13, batch 5: M8.9, M8.7 and M8.7b, M7.8, X.build1, M6.10 and the fixes.** Integrated:
  automatic feature code (framework, Update, regeneration at export; 69 equal, 23 different, 3
  removed over the corpus); layout tables at export with the oracle comparing `GSUB`, `GPOS` and
  `GDEF` lookup by lookup (fontmake 25 of 26, 82 of 88, 47 of 55; fontc 29 of 46, 80 of 91, 42 of
  68), the mark writer, UFO `GDEF` classes, feature includes that may climb out of the source
  folder; the name table and OS/2 of static instances as fontmake builds them (1,678 equal, 120
  different), the `;FontCraft <version>` stamp; one engine test binary, one binary each for
  decompile, io-glyphs, io-ufo and plist, and the `measure` profile; Edit View's component and
  anchor display and gestures (200-composite frame 5.9 ms). Fixes: the web build compiles; every
  export path and the code editor read includes from the source's include folder; `.glyphs` 3
  files keep deleted kerning deleted and keep a version 2 kerning key the reader did not use; a
  refused include candidate does not hide an allowed one. All eleven gate steps pass; 2,835
  tests pass, none fail, 34 ignored; 660 of 960 commands live; 1,683 guesses on 1,089 rows; 1,107
  observe rows; round trips unchanged (`.glyphs` 122/1/118/4, `.glyphspackage` 22/0/7/0, UFO 125
  identical, 2 different, 13 failed; designspace 59 identical, 53 failed). Packages built: 125 of
  277. Open: the builder packages M9.4, M9.5, M9.9, M9.10, M10.3, M10.14, M10.19, M11.5, M12.44,
  M12.17, M13.2; the follow-ups M13.15, token expansion in `plan::postscript_name` and
  `plan::file_stem`, corner-component selection; the skeleton pass for waves 2 to 5.
- **2026-10-13, batch 6: M9.10, M9.4, M9.5, M9.9, M9.8b and X.fix19.** Integrated: the oracle
  harness for variable fonts (`fontcraft-cli oracle --corpus --variable`, and `--instances`
  against `fontmake -i`); static instances between and beyond masters (`interp::instance_font`;
  extrapolated instances named in the export report and allow-listed against fontmake, which
  clamps); the variable TrueType compile (`compile_variable`, `file.exportVariable`,
  `fontcraft-cli compile --variable`; Oswald 0.19 s, Inter Roman about 1.3 to 1.6 s); variable
  kerning and mark positions (new dependency `fontdrasil` in `crates/fea`); alternate layers in
  variable fonts (`.BRACKET.varAltNN` glyphs, feature variations merged into the feature code's
  `GSUB`). Review fixes (X.fix19): oversized `gvar` tuples and deltas past 16 bits are refused
  with a message, kerning copied to alternate glyphs is indexed, and an instance at a master's
  location is interpolated when it sets manual weights, InterpolationWeightY or Local
  Interpolation. Variable fonts against fontc: 100 equal, 37 different, 21 failed; against
  fontmake 103, 38, 17 (before: all 158 not built). Instances against `fontmake -i`: 169, 41, 0,
  24 (before: 111, 25, 83, 15). Static comparison summaries and round trips unchanged. 2,865
  tests pass, none fail, 34 ignored; 661 of 960 commands live; 1,695 guesses on 1,097 rows; 1,115
  observe rows. Packages built: 129 of 277. Open: the builder packages M10.3, M10.14, M10.19,
  M11.5, M12.44, M12.17, M13.2, M9.11, M9.12, M9.13; the follow-ups designspace instance info
  from `<instance>`, designspace `<rules>`, M13.15, token expansion in
  `plan::postscript_name` and `plan::file_stem`, corner-component selection; the skeleton pass for
  waves 2 to 5.
- **2026-10-13, batch 7: M10.3, M10.14, M11.5 (part 1), X.fix20 and X.fix21.** Integrated:
  node-reference upkeep (after every font-changing command, PostScript hint ends and TrueType
  instruction points stay on their nodes through node inserts and deletes, contour reversal,
  start-point changes, Remove Overlap and Expand Outline, in the command's own undo step; a
  deleted node's TrueType instruction is removed and reported under `hintUpkeep`; 3.1 µs per edit
  on Inter without hints, 83 µs with); CFF hints at export (stem hints, masks, the four flex
  forms, manual hints per master through node links, family zones, `BlueScale` and `BlueFuzz`
  defaults with a warning when the tallest zone breaks the limit; fontTools decodes 43 exported
  fonts with 0 warnings, skrifa draws every glyph, the CFF oracle has 0 different glyphs, 1,838
  same); typed `.glyphs` paint attributes in versions 2 to 4 (colours with palette references,
  linear, radial and conic gradients, shadows; `strokePos` stored in the version 4 convention and
  converted exactly; version 3 and UFO writes report what they cannot hold; all 23 corpus colour
  sources keep their colour keys). X.fix20: hint references in `.glyphs` files count nodes in
  file order, converted by `NodeOrder` in the hinting crate. X.fix21 (review fixes): hint upkeep
  matches by place only for permuted paths, out-of-range manual CFF stems are dropped with a
  warning, masks come from rounded stems, `strokePos` conversion is exact, the UFO image-loss
  check looks where each writer puts a master's files, and the native-format flag for version 2
  kerning keys is kept. Round trips, the static oracle summary, the variable measure (fontc
  100/37/21, fontmake 103/38/17) and the instances (169/41/0/24) are unchanged. 2,903 tests
  pass, none fail, 35 ignored; 661 of 960 commands live; 1,706 guesses on 1,103 rows; 1,119
  observe rows. Packages built: 132 of 277. Open: the builder packages M12.44, M12.17, M13.2,
  M9.11, M9.12, M9.13, M10.17, M10.18, M10.19, M10.15, M11.5b; the follow-ups hint upkeep on
  background layers and for corner and cap hints, designspace instance info from `<instance>`,
  designspace `<rules>`, M13.15, token expansion in `plan::postscript_name` and
  `plan::file_stem`, corner-component selection; the skeleton pass for waves 2 to 5.
- **2026-10-13, batch 8: M12.44, M12.17, M13.2 and X.fix22.** Integrated: the audit runner
  (`audit.run` over the open document, sources compiled in memory, compiled fonts, collections,
  WOFF and WOFF2 files, and several files or a folder as a family; binary findings name the
  source field; preconditions, glyph selections, profile resolution and the ignore list apply,
  with ignored results counted apart and left out of the exit status; `audit.checkOutlines`,
  `audit.checkInterpolation`, `audit.compareFamily` and `fontcraft-cli check` with `outlines`,
  `family` and `interpolation` take the reference tools' option names and exit with 0, 1, 2, 3 or
  4); 3,676 corpus runs over 4 profiles and 919 files, no crash, 70 sources refused as unreadable.
  Subset (`font.subset`, `fontcraft-cli subset`, pyftsubset's option names; selections by text,
  code points, glyph names, glyph ids and `*`; closure through `cmap`, `glyf` components and CFF
  `seac`; core tables only, layout, variation and colour tables dropped with a stated reason;
  no new dependency): over 340 corpus fonts and 4 selections the glyph map equals pyftsubset's
  in 338 or 339 per selection and the core tables in 337 or 338. The plug-in module runtime
  (`crates/plugins`: wasmi, no imports, caps on fuel, wall-clock time, memory, call depth,
  output, manifest size and ABI version; filter output checked against the model's limits;
  reporter and check outputs parsed under item budgets; reporter drawings cached; 18 hostile
  modules each fail with their own error and change nothing; ported with credit, see `NOTICE`):
  filter 292 ms and reporter 0.35 ms (cached 0.4 µs) on 1,000 synthetic layers, all inside the
  targets; builds for wasm32. X.fix22 (review fixes): subset parses CFF subroutine INDEXes once
  and keeps one charstring operation budget per font, a component past the glyph count is an
  error, a format 4 `cmap` mapping U+FFFF stays format 4; the audit shares the bytes of a file's
  faces and caps the bytes, glyphs and folder entries of a run; folder entries and the outputs of
  whole-file commands pass the automation read and write root checks; plug-in output values are
  counted before anything is built, long logs are skipped and the reporter cache is capped by
  size; native-only limits and counters stay out of the web build. Round trips and the static,
  variable and instance oracle summaries are unchanged (`.glyphs` 122/1/118/4, `.glyphspackage`
  22/0/7/0, UFO 125/2/13, designspace 59/53; variable fontc 100/37/21, fontmake 103/38/17;
  instances 169/41/0/24). All eleven gate steps pass; 2,941 tests pass, none fail, 36 ignored; 665 of 960
  commands live; 1,714 guesses on 1,109 rows; 1,124 observe rows. Packages built: 135 of 277
  (M6 16/20, M7 18/21, M8 16/25, M9 12/27, M10 13/33, M11 7/21, M12 28/68, M13 25/62). Open: the
  builder packages M9.11, M9.12, M9.13, M10.17, M10.18, M10.19, M10.15, M11.5b, M12.18, M12.19
  and M13.3 to M13.6; the follow-ups for the audit runner (hash cache, options, width checks,
  `summary.errors`, source-field names, the outline warnings on decoded fonts) and subset (CFF2,
  subroutine pruning, `--desubroutinize`, sources), hint upkeep on background layers and for
  corner and cap hints, designspace instance info from `<instance>`, designspace `<rules>`,
  M13.15, token expansion in `plan::postscript_name` and `plan::file_stem`,
  corner-component selection; the skeleton pass for waves 2 to 5.
- **2026-10-13, batch 9: M9.11, M9.12, X.build2 and X.fix23.** Integrated: the reference's
  variable feature syntax lowered as glyphsLib does (`condition` statements to `conditionset` and
  `variation` blocks with limits mapped to user coordinates and overlapping regions overlaid;
  `(tag:coord) value` positions to variable scalars, also in value records and anchors; equal to
  glyphsLib's converter on 32 synthetic texts; ported in part, credited); variable exports
  compile their feature code on fea-rs with a real variation info (axes with maps, masters,
  number values), with conditions and `#ifdef VARIABLE` as `GSUB` feature variations merged with
  the alternate layers' records and variable `pos` values and number tokens resolved through the
  model of the generated kerning, in one `GDEF` store (inverted conditions are export warnings;
  equal to fontmake on variable `pos` plus alternate layers and to fontc on condition sets plus
  number tokens); static exports resolve values and conditional rules at their location,
  instances between masters included. Variable measure: fontc 101 equal, 37 different, 20 failed
  (was 100/37/21), fontmake 104/38/16 (was 103/38/17); VarFeaOneShot builds and is equal.
  X.build2: the gate runs its tests on cargo nextest (all test binaries in parallel, doctests
  apart), the command sweep is 16 tests (4 states times 4 chunks by a stable hash of the command
  id), and one randomised pathops test reserves 8 threads (`.config/nextest.toml`); a full gate
  on a warm build takes about 5 minutes. X.fix23 (review fixes): the variable-syntax scanner is
  linear with a value-record size limit; conditions on axes wider than ±1 keep their ranges; a
  glyph named `condition` compiles; number-token expressions in metric statements become
  per-master variable values (tokens elsewhere take the origin value); two masters at one
  location with different token values are an error; one-line `.glyphs` dictionaries and lists
  (for example `numbers = ({name = gap;})`, `axisRules = ({min = 600;})`) reach the model and
  keep their layout on write, matched by position in linear time. Round trips (`.glyphs`
  122/1/118/4, `.glyphspackage` 22/0/7/0, UFO 125/2/13, designspace 59/53), the static oracle
  summary and the instances (169/41/0/24) are unchanged. All eleven gate steps pass; 2,983
  tests pass, none fail, 36 ignored; 665 of 960 commands live; 1,720 guesses on 1,111 rows; 1,126
  observe rows. Packages built: 137 of 277 (M6 16/20, M7 18/21, M8 16/25, M9 14/27, M10 13/33,
  M11 7/21, M12 28/68, M13 25/62). Open: the builder packages M9.13, M10.17, M10.18, M10.19,
  M10.15, M11.5b, M12.18, M12.19 and M13.3 to M13.6; the follow-ups variable values in table
  blocks, stale kept-layout extras, the audit runner (hash cache, options, width checks,
  `summary.errors`, source-field names, the outline warnings on decoded fonts) and subset (CFF2,
  subroutine pruning, `--desubroutinize`, sources), hint upkeep on background layers and for
  corner and cap hints, designspace instance info from `<instance>`, designspace `<rules>`,
  M13.15, token expansion in `plan::postscript_name` and `plan::file_stem`,
  corner-component selection; the skeleton pass for waves 2 to 5.
- **2026-10-13, batch 10: M9.13, M10.17, M10.6, M10.7 and X.fix24.** Integrated: variable fonts
  with CFF2 outlines from `fontcraft-cli compile --variable --format otf` (or `--format
  variable-cff2`) and `file.exportVariable {outlines: "cff2"}`, with blended charstrings
  (`vsindex`, blends split at the 513-value stack limit), one Font DICT with a blended Private
  DICT, the item variation store, `hmtx` with `HVAR`, `post` 2.0 and `maxp` 0.5; no hints (a
  warning when the source has manual PostScript hints), no subroutines, no specialisation.
  `oracle --variable --format otf` against fontmake's `variable-cff2`: 104 equal, 34 different,
  20 failed of 158; every outline and advance difference also shows in the TrueType build
  against fontmake; fontTools reads all 153 outputs without warnings. The TrueType variable
  measure is unchanged (fontc 101/37/20). M10.17: `hinting::tt_font` builds `cvt `, a
  seven-function `fpgm` library and a per-size `prep`, each handbook behaviour tested in
  skrifa's interpreter, written from the TrueType specification and the handbook; export wiring
  is M10.19; the handbook's rounding-threshold example contradicts its default and FontCraft
  follows the default (a trial check is listed). M10.6 and M10.7: the `TouchedState` model and 14
  commands (Snap Point, Set Snap Zone, Stem, Reverse, Set Stem Reference, Make Triple Hint, Set
  Instruction Rounding, Shift Points, Interpolate, Delta, Set Delta Cells, Delete Instructions,
  Remove Hints, List TrueType Instructions) that check their selection against it; each is one
  undo step and a refusal leaves the font pointer-equal. X.fix24 (review fixes): the component
  closure of decomposed CFF2 composites is linear; PPM parameters past 32767 are clamped; `prep`
  sizes are capped to signed words; CFF2 Private DICT deltas are checked against the region list
  and region lists past one blend are refused up front; a glyph whose outline and location
  counts differ is an error; the dropped-hints note counts PostScript hints only; braced delta
  column keys are normalised. Round trips (`.glyphs` 122/1/118/4, `.glyphspackage` 22/0/7/0, UFO
  125/2/13, designspace 59/53), the static oracle summary and the instances (169/41/0/24) are
  unchanged. All eleven gate steps pass; 3,015 tests pass, none fail, 36 ignored; 679 of 960
  commands live; 1,741 guesses on 1,125 rows; 1,139 observe rows. Packages built: 141 of 277 (M6
  16/20, M7 18/21, M8 16/25, M9 15/27, M10 16/33, M11 7/21, M12 28/68, M13 25/62). Open: the
  builder packages M10.18, M10.19, M10.15, M11.5b, M12.18, M12.19 and M13.3 to M13.6; the
  follow-ups hints, subroutines and specialisation in CFF2, the variation of TrueType composites
  with intermediate layers, variable values in table blocks, stale kept-layout extras, the audit
  runner (hash cache, options, width checks, `summary.errors`, source-field names, the outline
  warnings on decoded fonts) and subset (CFF2, subroutine pruning, `--desubroutinize`,
  sources), hint upkeep on background layers and for corner and cap hints, designspace instance
  info from `<instance>`, designspace `<rules>`, M13.15, token expansion in
  `plan::postscript_name` and `plan::file_stem`, corner-component selection; the skeleton pass
  for waves 2 to 5.
- **2026-10-13, batch 11: M10.18, M10.19, X.suite1 and X.fix25.** Integrated: glyph programs
  (`fontcraft_hinting::glyph_program`) that compile a layer's TrueType instructions plus the
  exported point map to `glyf` bytecode (Snap through zones, named-zone distance and five
  rounding modes; Stem with Auto, named and No Stem; Shift and Interpolate with rounding; Delta
  before IUP; triple hints keeping a pixel between stems; diagonals preserved unless
  `TTFDontPreserveDiagonals`; Ignore Vertical Hints; a 10-function `fpgm`; 14 behaviour tests in
  skrifa's interpreter in both directions). TrueType export (M10.19): manual instructions compile
  against a point map that follows cu2qu, overlap removal and decomposition, the font gets the
  generated `cvt `, `fpgm` and `prep` or the imported `cvt Table` and `prep` and `fpgm Table
  Assembly` parameters, `maxp` covers the analysed needs, `head` and `gasp` flags are set,
  static instances interpolate zones and stems by name and take the main master's hints, a
  glyph that fails to compile exports unhinted with a warning, and variable fonts take the
  default master's programs without deltas or `cvar`. `hint.tt.getPointIndexes` is live. skrifa
  runs every exported glyph of the instructed corpus samples at 8 to 72 PPM in three intents
  with 0 errors; fontTools reads them without warnings. X.suite1: `write_atomic` goes through
  the Craft suite's `craft-atomic` (craft-libs, pinned by git commit; native builds only); a
  read-only file is refused and kept; the folder is flushed after the rename; decision 0008
  records which suite libraries FontCraft adopts and when. X.fix25 (review fixes): a linear point
  map, unhinted variable exports of instructed sources write plain points, off-curve nodes next
  to removed nodes are left unmapped with a warning, the PostScript instance-hint rule is
  documented and tested with a CFF instance test. Round trips (`.glyphs` 122/1/118/4,
  `.glyphspackage` 22/0/7/0, UFO 125/2/13, designspace 59/53), the static oracle summary
  against fontmake (2,360 compared, 2,202 the same), the variable summary against fontc
  (101/37/20) and the instances (169/41/0/24) are unchanged. All eleven gate steps pass;
  3,043 tests pass, none fail, 36 ignored; 680 of 960 commands live; 1,755 guesses on 1,132
  rows; 1,146 observe rows. Packages built: 143 of 277 (M6 16/20, M7 18/21, M8 16/25, M9 15/27,
  M10 18/33, M11 7/21, M12 28/68, M13 25/62). Open: the builder packages M10.15, M10.21,
  M11.5b, M12.18, M12.19 and M13.3 to M13.6, then X.suite2 (`craft-quit`) and X.suite3
  (`craft-settings`), with `craft-print` at M12.36, `craft-ui` at M14 and a watch on the suite's
  proposed `craft-control`; the follow-ups `cvar` and instruction deltas in variable fonts,
  drawing of point indexes, special layers' own instructions, hints, subroutines and
  specialisation in CFF2, the variation of TrueType composites with intermediate layers,
  variable values in table blocks, stale kept-layout extras, the audit runner (hash cache,
  options, width checks, `summary.errors`, source-field names, the outline warnings on decoded
  fonts) and subset (CFF2, subroutine pruning, `--desubroutinize`, sources), hint upkeep on
  background layers and for corner and cap hints, designspace instance info from `<instance>`,
  designspace `<rules>`, M13.15, token expansion in `plan::postscript_name` and
  `plan::file_stem`, corner-component selection; the skeleton pass for waves 2 to 5.

- **2026-10-18, batch 12: 75 packages of M6 to M13, X.deps1, X.fix26, X.fix27, X.audit1.** Written in
  parallel in separate worktrees, then integrated in groups X.integ1 (engine commands), X.integ2
  (hinting, compile, colour), X.integ3a and X.integ3b (tables, TTX, CLI; audit), X.integ4 (modules),
  X.integ5 (canvas aids, windows, Instructor), each with its fixes; a read-only review of the whole
  range found two high findings (the document could name the ttfautohint binary; the control
  channel's parser could run out of memory), fixed in X.fix27 with the rest. Landed: the TrueType
  autohinter and the Instructor; PostScript autohinting at export; `COLR` v0 and v1, `CPAL`, `sbix`,
  `SVG `; the axis, special-layer, instance and master commands and windows; TTX writers; feature
  convert, compile and lint; more checks and audit fixes; plug-in modules, the Extension Manager, MCP
  over HTTP; display aids, drag aids, power guides, Rapid and Spiro; auto-spacing and auto-kerning;
  the shaping crate; `io-bitmap` and `io-pdf`. Numbers: 852 of 985 commands live (was 680 of 960);
  218 of 277 packages (was 143); oracle, `COLR`, round-trip, TTX and `corpus_import` baselines under
  "State of the tree". Missed: TrueType autohinter x-height (61.7%) and cap-height (93.2%) rows,
  auto-spacing median error, auto-kerning sign on Inter Thin, FontDame 19 of 28. Gate: 3,729 passed, 0 failed, 42 skipped tests
  pass. Open: everything under "Where to start".

- **2026-10-19, batch 13: 17 packages of M8 to M13, X.tune1, X.integ6, X.fix29, X.fix30.** Written in
  parallel in separate worktrees, integrated in X.integ6 (with the removal of the duplicate
  `path.setShapeOrder`; the ported code is credited in `NOTICE`), reviewed
  and fixed in X.fix29 and X.fix30. Landed: table blocks and `usMaxContext` (M8.8); shaping and the Features
  menu in Edit View (M8.19); contextual anchors (M8.21); the preview's instance pop-up (M9.20); `CBDT` and `CBLC`
  (M11.11); images in Edit View (M11.13); variable `COLR` (M11.19); the TTX reader (M12.8); static instances
  (M12.20), merge (M12.23), transform (M12.24) and the CFF subroutiniser (M12.25); Erase Open Corners, Flatten
  Components and more `audit.fix` steps (M12.62); pair lists, the Kerning Pairs Preview and Groups Editor
  (M13.22); the Space Center (M13.23); TrueType assembly and the debugger (M13.51, M13.52); the tuning of
  auto-spacing and auto-kerning (X.tune1). Numbers: 885 of 993 commands live (was 852 of 985); 235 of 277
  packages (was 218); the oracle, `COLR`, subroutiniser and tool-against-fontTools baselines are under "State of
  the tree". Missed: TrueType autohinter x-height (61.7%) and cap-height (93.2%) rows, auto-spacing on Inter
  (30% to 75% against 10%), FontDame 19 of 28; auto-kerning meets its target (85.7% to 99.2%). Gate: 3,897 tests passed, 0 failed, 41 skipped.
  Open: everything under "Where to start"; the push waits for the owner.

- **2026-10-19, batch 14: 14 packages and three tasks of M8 to M13, X.tables1, X.ds1, X.small1, X.integ7 to
  X.integ9, X.fix31 to X.fix34.** Written in parallel in separate worktrees, integrated in X.integ7 (four
  groups) and X.integ8, reviewed and fixed in X.fix31 to X.fix34, then merged and gated in X.integ9. Landed:
  shaping and bidi in Text Preview (M8.20); the Space Center's display options (M13.23b); colour drawing in Edit
  View and the previews (M11.14); the autohinter's x-height scaling (M10.21b); the Detect Composites base choice
  (M13.15b); instances from sources (M12.22) and partial instances (M12.21); designspace instance data and rules
  (X.ds1); CFF subroutines at export and `optimize` (M12.26); the `variations` tools (M12.34); the TTX reader for
  the layout, variation and colour tables (M12.9) and writers for `COLR` version 1, the bitmap and the Apple
  tables (M12.7b); FontDame conversion (M12.13b); `maxp` recalculation (X.tables1); `audit.fix` on compiled fonts
  (M12.61); the Font Audit panel (M12.64); the `CBDT` option, the assembly warning and hint nudging (X.small1).
  Numbers: 905 of 997 commands live (was 885 of 993); 244 of 277 packages (was 235); instances against
  `fontmake -i` 180 equal (was 169); the other baselines are under "State of the tree". Missed: autohinter
  x-height rows (92.9% against 95%) and zone agreement (98.29% against 99%), Detect Composites (63.0% against
  95%), auto-spacing on Inter (30% to 75% against 10%), FontDame 24 of 28. Gate: 4,042 tests passed, 0 failed,
  43 skipped. Open: everything under "Where to start"; the push waits for the owner.

- **2026-10-19, batch 15: 17 packages of M8 to M12, X.conv2, X.small2 to X.small6, X.integ10 to X.integ12,
  X.fix35 to X.fix40.** Written in parallel in separate worktrees, integrated in X.integ10 (three groups) and
  X.integ11 (three groups), reviewed and fixed in X.fix35 to X.fix40, then gated in X.integ12. Landed: feature
  code rebuilt from compiled fonts on open (M8.23); subsetting of layout, variation, colour and CFF2 tables with
  subroutine pruning (M12.18, M12.19, M12.19b); conversion between TrueType, CFF and CFF2 outlines (M12.30, X.conv2);
  AFDKO project compile, CID-keyed CFF and the compile extras (M12.40, M12.40b, M12.41, M12.42); CFF2 instances
  (M12.20b); merge of CFF fonts, `MATH` and `BASE` (M12.23b); TTX readers for `COLR` version 1, the bitmap and Apple
  tables, and split dumps (M12.7c); the Table Viewer (M12.38); staged audit runs and save choosers (M12.64b, X.small5,
  X.small6); images and colour in Font View cells (M11.16); the colour Text Preview (M11.17); full-colour layers in UFO
  (M11.5b); `CBDT` in export profiles, truncation warnings and autohinting from export profiles (X.small3, X.small4).
  Integration found a bug that dropped `GDEF` classes from subsets (the class remap clamped ranges to the output glyph
  count) and a fea-rs panic on calls to empty lookups. Numbers: 914 of 1,004 commands live (was 905 of 997); 254 of 277
  packages (was 244); CFF to TrueType conversion 43,967 identical (was 12,449); project compile equal to makeotf on 7
  of 10 corpus UFOs; the other baselines are under "State of the tree". The measures that missed their targets did not
  move. Gate: 4,237 tests passed, 0 failed, 46 skipped. Open: everything under "Where to start", with M12.19c, X.fix41
  and X.fix42 written and waiting to be integrated; the push waits for the owner.

- **2026-10-20, the eleventh batch: M9.25 and 20 follow-up packages of M8 to M13, X.fix41 to X.fix45, X.tables2,
  X.ui1, X.audit2, X.audit3, X.opt1, X.opt2, X.edit1, X.integ13 to X.integ16, X.fix47 to X.fix54, X.gate15.**
  Written in parallel in separate worktrees, merged in two waves, tested in X.integ13 to X.integ16, reviewed in four
  parts and fixed in X.fix47 to X.fix54, then gated once in X.gate15. Landed: higher-order interpolation data on
  nodes (M9.25); subsetting of `VARC` and compressed `SVG ` and pyftsubset's list options (M12.19c, M12.19d,
  M12.18b); TTX `mort`, `morx` and bitmap image forms (M12.7d, M12.7e); convert from Type 1, bare CFF and TTX
  (M12.30b); transform of CFF, CFF2 and variable fonts (M12.24b, M12.24c); partial instances of `avar` version 2,
  `VARC` and variable `COLR` fonts (M12.21b); the rest of instances from sources and `--layout-only` (M12.22b,
  M12.22c); `optimize` (X.opt1, X.opt2); the subroutiniser and convert on full CJK fonts (M12.25b, X.fix42);
  makeotf's project keys (M12.40d); `PaintColrGlyph` in UFO and compiled `COLR` (M11.5c, M11.7b); the rest of the
  colour Text Preview (M11.17b); image editing in Edit View (M11.13b); line breaking and vertical text in Text
  Preview (M8.20b); one feature compile for the previews (X.edit1); bitmap strikes in the model (M13.55b); one
  feature-code writer (M8.23b); the rest of `audit.fix` (X.audit2, X.audit3); `post` and `maxp` after instance,
  transform and merge (X.tables2); small UI leftovers (X.ui1). The review found a lookup-guard undercount that could
  let fea-rs abort and an out-of-memory path in the `mort` reader. Numbers: 917 of 1,004 commands live (was 914);
  255 of 277 packages (was 254); feature parity about 35% (35.0, was 34.5); variable fonts against fontc 103/35/20
  and fontmake 106/36/16 (was 101/37/20 and 104/38/16); instances from sources 0 glyph differences (was 6); the
  other baselines are under "State of the tree". The AFDKO-tool measures are pending. Gate: 4,431 tests passed, 0
  failed, 49 skipped. Open: everything under "Where to start", with six next-batch branches written and not merged.

- **2026-10-20, the twelfth batch: 12 follow-up packages of M8 to M13, X.ui2, X.fix55, X.integ17, X.integ18, X.fix56 to
  X.fix58, X.gate16.** Written in parallel in separate worktrees, merged in two waves, tested in X.integ17 and
  X.integ18, reviewed in three parts and fixed in X.fix56 to X.fix58, then gated once in X.gate16. Landed: variable
  TrueType fonts merged as variable fonts (M12.23c); specialised and subroutinised CFF2 exports with blended hints
  (M12.25c, M12.25d); the ROS parameter as CID-keyed CFF (M12.41b); the TTX bitmap writer forms and external image
  files (M12.7f); an `EBDT` and `EBLC` writer, `.otb` export and strike rasterising (M13.55c); BDF, PCF and `.otb`
  fonts opened and compiled strikes read into the model (M13.55d); `PaintColrGlyph` references in Edit View and the
  `SVG ` table (M11.14b); `SVG ` clip paths, masks, patterns and sliced images in the colour Text Preview (M11.17c);
  vertical positioning in Text Preview (M8.20c); the last four pyftsubset options (M12.18c); layout-only instances of
  `.glyphs` sources and `--interpolate-binary-layout` (M12.22d); rotatefont's reading precision (X.fix55); check
  marks on every menu switch (X.ui2). The review found unbounded memory in strike decoding, rasterising and `SVG `
  reference fan-out, and a binary-layout interpolation that left files behind on failure; the gate found a name-table
  writer panic past about 5,460 records. Numbers: 917 of 1,004 commands live (unchanged); 255 of 277 packages
  (unchanged); feature parity about 36% (35.7, was 35.0); variable CFF2 against fontmake 106/32/20; rotatefont 0
  differences over 21,774 glyphs; the eleventh batch's AFDKO measures run (tx 3,172/0/3, makeotf 7 of 10, CID 13 of
  13, cffsubr 0.785). Gate: 4,501 tests passed, 0 failed, 49 skipped. Open: everything under "Where to start", with
  `w-M12.22e` and `w-X.fix59` written and not merged, and M13.15c and M12.40c next in the builder queue.

- **2026-10-21, the thirteenth batch: 10 follow-up packages of M10 to M13, X.fix59, X.pdf1, X.var3, X.cff3, X.audit3,
  X.ui3, X.glyphs3, X.integ19, X.integ20, X.fix60 to X.fix66, X.gate17.** Written in parallel in separate
  worktrees, M13.15c built in the main checkout, merged and tested in X.integ19 and X.integ20, reviewed in seven
  parts and fixed in X.fix60 to X.fix66, gated once in X.gate17. Landed: `cvar` (M10.19b); CFF2 hints of
  intermediate and alternate layers (M12.25e) and each master's family blues (X.fix59); `compile
  --interpolate-binary-layout` and `Unnamedfont-*` file names (M12.22e); convert to UFO with glyph subsets, GOADB,
  CID keying, Font DICT and `/CID` selection, `--location` and overlap removal (M12.30c, M12.30d); spot listings and
  the CFF literal view (M12.10b); the instancer on `crates/cff` (X.cff3); `recompileCff`, the layer cache and audit
  options (X.audit3); feature variations under several tags and instance font info (X.var3); `SVG ` filters
  (M11.17d); PDF in paste and Import Outlines (X.pdf1); gzipped PCF, FNT and FON (M13.55e); PK, GF and Mac bitmap
  fonts (M13.55f); Detect Composites among equal outlines (M13.15c); the small UI leftovers (X.ui3); stale kept
  `.glyphs` layouts (X.glyphs3). M8.11b had nothing to build: M8.11 landed in `47a7f58`. The review found an audit
  cache that ignored profile thresholds, a CID-keyed convert that kept `seac` glyphs, filters that ignored enclosing
  clips, unbounded Mac strike listings, unread one-line layer guides and a glyph-order dependence in Detect
  Composites. Numbers: 920 of 1,006 commands live (was 917 of 1,004); 255 of 277 packages (unchanged); feature
  parity about 36% (35.9, was 35.7); Detect Composites on Inter 78.1% (was 63.0%); variable CFF2 against fontmake
  107/31/20 (was 106/32/20); every other AFDKO measure, the static oracles and the round trips unchanged. Gate:
  4,625 tests passed, 0 failed, 49 skipped. Open: everything under "Where to start"; batch 18 writes the remaining
  packages, then the builder queue runs M12.40c.
- **2026-10-21, batch 18 (the fourteenth batch) and the second planning pass.** Landed: project compiles that
  place components in single precision and give each rule type its own lookup (M12.40c); the remaining `SVG `
  filter primitives but `feImage`, filters in Edit View and raster warnings in Text Preview (M11.17e);
  `checkOutlines` clean-up, `check family -l` and repeatable list options (X.audit4); TTX bounds of CFF fonts
  (X.ttx4); designspace documents without axes, Thin style-map names and name tokens in file names (X.inst4); the
  clipboard's PDF in Edit View and placed PDFs in bitmap strikes (X.pdf2); `.glyphs` node attribute layouts and
  version 2 settings in version 3 files (X.glyphs4). The review found a font saved as a designspace that would not
  compile and off-screen filtered glyphs rasterised on every frame (X.fix67 to X.fix72). Numbers: 920 of 1,006
  commands live (unchanged); 255 of 277 packages (unchanged); feature parity about 36% (36.0, was 35.9); makeotf
  7 of 10 with the other 3 written down as makeotf faults; TTX read bounds 253 of 253; instance font info 0
  differences; every other AFDKO measure, the oracles and the round trips unchanged. Gate: 4,682 tests passed, 0
  failed, 49 skipped. The second planning pass graded the 22 areas and wrote 167 packages in 13 waves plus a builder
  tail (`docs/plans/depth.md`), from 36% to about 51% parity. Open: everything under "Where to start", wave 1
  first.
- **2026-10-21, depth wave 1 (the fifteenth batch).** Landed: save prompts on close, quit and File › Close,
  `app.quit`, `window.closePrompt` and the window title with the suite's `craft-quit` (M14.3); the Open chooser,
  Open Folder, drop to open and project routing (M1.8); the settings window with User Settings and Extensions
  (M14.1); axes and coordinates in Font Info (M9.27); value lists and checks for 64 custom parameters (M7.21);
  filter rename, drag and Alt groups in the sidebar (M4.17); Layers drag reorder, backup to master and a master per
  glyph in text (M3.40); the Select tool's Alt and Ctrl+Alt drags and `path.reshapeSegment` (M3.19); the
  Measurement tool over the whole line with held tools (M3.21); pointer, wheel, `ui.set`, resize, focus and context
  menus over the control channel (M13.78). The reviews found a measurement that failed on long lines and recomputed
  every frame, keys that reached the canvas behind the save question, and input applied before the frame that draws
  it (X.fix73 to X.fix77). Numbers: 926 of 1,009 commands live; feature parity about 37% (37.2, was 36.0; the plan
  expected 38.1); depth packages 10 of 167; round trips and compile output unchanged. Gate: 4,767 tests passed, 0
  failed, 49 skipped. Open: the owner's Mac check, then wave 2.
- **2026-11-19 and 2026-11-20, the sixteenth batch.** Writers in worktrees without builds, integrators merging in
  rounds, one check at the end. Landed: 47 writer packages of depth waves 2 to 11 (compile options, CLI common
  options and listings, subsets, mergefonts, convert and dump extras, PDF proofs, autohint of compiled CFF and CFF2,
  higher-order interpolation, Match Masters, designspace helpers, Indic positional features, `CPAL` version 1,
  colour artwork, bitmap commands, External Preview, MCP window tools, nice names, UFOZ save), with M10.37b, M10.37c
  and M13.65b; ES.1 to ES.3 (`engine-core`, the command registry, extension slots); the craft-next window, UI.1 to
  UI.29 with five lettered parts; fixes X.fix78 to X.fix85 (hint renumbering, CLI prefixes, Indic languagesystems,
  format-warning cost, active layer, dialog targets). Integration X.integ26 to X.integ80 and X.inte1; check and
  clippy clean. First full test run: 5,304 run, 33 failed; X.fix86 fixed 24, X.fix87 takes the 9 UI failures.
  Numbers: 960 of 1,039 commands live; feature parity about 38% (38.4, was 37.2); depth packages 55 of 167; CLI
  option coverage 672 of 770. Open: the second test run, the gate, review, push; the measures listed under "State
  of the tree"; the owner's two questions.
- **2026-11-20, the seventeenth and eighteenth batches.** Writers in worktrees without builds; one integrator built several branches at
  once (X.integ81 to X.integ97). Landed: UI.30a, UI.30b, UI.24b, UI.27b, UI.31, UI.32, UI.34, UI.33 and UI.20b
  (the seventeenth batch); UI.35a to UI.35d, UI.36a, UI.36b and UI.36d (the eighteenth batch); `crates/craft-ext` with decision 0009 (UI.34,
  UI.36); the window's contents are listed in "State of the tree" › Where the seventeenth and eighteenth batches stand. Decisions:
  - Writers don't build; one integrator builds several branches at once. One agent works at a time on the window.
  - The export dialog gets no feature-compiler section; the reference's has none, and the options stay on
    `font.compile`.
  - Font View cells draw the current master and shown layer (UI.31, UI.36d).
  - The Space Center matrix shows at most eight rows before it scrolls (a guess, recorded on the panel's row).
  - Compare Fonts in List is a dock panel.
  - Reporter switches persist through the reserved setting `view.shownReporters`.
  - Alt in the Font View glyph menu is read when the menu opens (a guess).
  - The danger tone on menu rows is our own choice (a guess); it colours the icon only, because the label ink needs
    a change in craft.
  Gate: green on `f99ee43`, 5,382 tests. Open: the ports left in "Where to start" item 2, then retiring the egui
  window, the engine split from ES.4 and the oracle measures.
