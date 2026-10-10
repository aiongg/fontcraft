# Compile specification

What `fontcraft` must produce when it turns a source into a binary font, stage by stage and table by
table. Rows live in `spec/inventory/40-tables.toml` (`[[table]]` by tag, `[[behavior]]` ids
`compile.*`); this file explains how they fit together.

References and the versions read (2026-10-07, local clones):

| Reference | Version | Licence | Role |
|---|---|---|---|
| fontations | read-fonts 0.45.0, write-fonts 0.54.0, font-types 0.12.6, skrifa 0.48.0, skera 0.8.0 (commit d6a9007) | MIT/Apache-2.0 | The crates we read and write tables with |
| fontc | fontc, fontbe, fontir, fea-rs 1.0.0 (commit f7d2bd9); depends on write-fonts 0.54.0, kurbo 0.13.0 | Apache-2.0 | Rust compiler; TrueType-flavoured output only |
| fontmake | commit 92f6ea7; pins fonttools 4.63.0, ufo2ft 3.8.0, glyphsLib 6.11.6, cffsubr 0.3.0, compreffor 0.6.0 | Apache-2.0 | Python pipeline; options and documented defaults |
| fonttools | 4.66.2.dev0 (commit 9d3adb4) | MIT | Assembly, WOFF/WOFF2, collections, cu2qu, CFF helpers |
| AFDKO | commit c53bdc6 | Apache-2.0 | makeotf / addfeatures rules for the CFF path |

**How far each statement was checked.** "Verified" below means read in the source code or
documentation named. ufo2ft and glyphsLib are not in the reference folder: anything said about
them comes either from fontmake's `USAGE.md` or from fontc source comments that cite the Python
line they imitate, and is marked "per fontc" or "per fontmake docs". Nothing was compiled or run;
no binary output of any reference was inspected.

---

## 1. Pipeline

Each stage consumes only earlier stages' results. Stages 3 and 5 are per-glyph and run in parallel
(fontc's work graph does the same); stage 8 is the only one that sees every table.

### 1.1 Preflight

Input: the source model plus export options. Output: a list of errors and warnings, or a go.

- Drop component references to glyphs that do not exist, with a warning (`compile.pruneMissingComponents`).
- Remove non-exported glyphs from the build set and inline them where exported glyphs use them
  (`compile.nonExportGlyphs`).
- Check master compatibility for variable output: same component sequence and same path structure
  in every master (`compile.interpolationCompatibility`).
- Refuse more than 65535 glyphs (`compile.glyphCountLimit`), a codepoint mapped to two glyphs
  (`compile.cmapConflict`), and report advisory items: name lengths (`compile.nameLengthAdvice`),
  weight classes below 250 (`compile.os2WeightAdvice`).
- Resolve feature `include()` paths against the source location (`compile.expandFeatureIncludes`).

### 1.2 Instance or master preparation

- Static instance: interpolate masters at the instance location first; no extrapolation unless the
  user asks for it (`compile.instanceInterpolation`). Layout values for instances come from the
  variable layout data, not from re-running generators (`compile.instanceLayoutInterpolation`).
- Variable font: keep all masters; single-point axes are removed from the variation model
  (`compile.variableOnlyTables`).
- Apply instance glyph filters here (`compile.subsetOnExport`).
- Glyphs-style pre-steps when the source enables them: erase open corners, propagate anchors;
  propagation precedes any flattening (`compile.glyphsOutlinePreSteps`).

### 1.3 Outline preparation

Order follows fontmake's filter order (`compile.filterOrder`): decompose, flatten, remove overlaps,
convert curves.

1. **Components.**
   - CFF and CFF2 output: decompose everything (`compile.otfDecomposesAllComponents`).
   - TrueType output: a glyph mixing contours and components becomes contours
     (`compile.mixedGlyphsBecomeSimple`); so does a composite whose transforms vary between
     masters (`compile.inconsistentComponentsDecompose`) or exceed ±2.0
     (`compile.overflowingComponentTransformDecompose`).
   - Optional: decompose all or listed glyphs (`compile.decomposeAllComponents`), decompose
     transformed components (`compile.decomposeTransformedComponents`), flatten nesting
     (`compile.flattenNestedComponents`). When both of the last two are on, transformed components
     go first.
2. **Overlaps.** Removed for static output by default, kept for variable output
   (`compile.removeOverlapsStatic`); variable TrueType glyphs get the overlap flags
   (`compile.overlapFlags`, our choice). Before this step, a layer with strokes or masks is
   replaced by the composite of its shapes (`compositing.shapesCombineInOrder`): strokes
   expanded, masks applied in shape order. Details in § 3.8.
3. **Curves** (TrueType only). Cubic to quadratic at 0.001 em tolerance (`compile.cu2quTolerance`),
   all masters converted together segment by segment (`compile.cu2quAcrossMasters`). Other curve
   modes are deferred (`compile.ttfCurveModes`). Optional implied-point removal
   (`compile.dropImpliedOnCurves`).
4. **Direction** (TrueType only). Reverse contours unless told to keep them
   (`compile.reverseContourDirection`).
5. **Rounding.** Half-up everywhere, masters rounded before deltas (`compile.rounding`); CFF
   rounding tolerance 0.5 by default (`compile.cffRoundTolerance`).

### 1.4 Glyph order and names

- Source order, then leftovers sorted, `.notdef` forced to id 0 and generated if absent
  (`compile.glyphOrder`, `compile.notdefFirst`, `compile.notdefSynthesis`). AFDKO-style ordering is
  an alternative mode (`compile.glyphOrderMakeotf`); CID-keyed fonts use CID order
  (`compile.cidKeyedFonts`).
- Compiler-made glyphs (bracket layers, colour layers) are appended (`compile.bracketLayerGlyphs`).
- Production renaming and sanitising (`compile.productionNames`,
  `compile.productionNameSanitising`); GOADB renaming for AFDKO projects (`compile.goadbRenaming`,
  `compile.unicodeFromGlyphNames`).

The glyph order is final after this stage. Everything later indexes by glyph id.

### 1.5 Per-table builders (static core)

Dependencies between builders, as fontc schedules them (verified in each module's `read_access`):

| Builder | Needs first |
|---|---|
| glyf fragments (per glyph) | prepared outlines, component glyphs |
| glyf + loca | all fragments, final glyph order; yields loca format and composite bounds (`compile.compositeBounds`) |
| CFF / CFF2 | prepared outlines, glyph order, names, hint data |
| hmtx, hhea, maxp | built glyphs with bounds (`compile.hmtxConstruction`, `compile.hheaExtents`, `compile.maxpCompositeLimits`) |
| vmtx, vhea | same, only with vertical data (`compile.verticalMetrics`) |
| head | loca format, font box, timestamps (`compile.headFlagsAndPpem`, `compile.headFontRevision`, `compile.timestamps`, `compile.headMacStyle`) |
| cmap | glyph order, codepoints (`compile.cmapSubtables`, `compile.cmapVariationSequences`) |
| post | glyph order, production names, metrics (`compile.postVersion`, `compile.postFields`) |
| name | font info, then axis/instance/STAT names, then FEA records (`compile.namePlatforms`, `compile.nameFallbacks`, `compile.nameFullAndPostScript`, `compile.nameVersionString`, `compile.nameUniqueId`, `compile.nameTypographicDropped`, `compile.styleLinking`, `compile.nameIdAllocation`, `compile.nameFeaOverrides`) |
| OS/2 | hmtx, hhea, cmap codepoints, GSUB and GPOS (for usMaxContext), FEA overrides (`compile.os2*`, `compile.typoMetricsDefaults`, `compile.winMetricsDefaults`, `compile.subSuperStrikeoutDefaults`) |
| gasp, meta | font info only |

Default metric derivations are in `compile.ascenderDescenderDefaults`,
`compile.typoMetricsDefaults`, `compile.winMetricsDefaults`, `compile.hheaMetricsDefaults`,
`compile.underlineDefaults`, `compile.caretSlopeDefaults`. Feature-code table blocks are applied
last (`compile.feaTableOverrides`).

### 1.6 Layout compile

1. Parse and validate feature code once against the final glyph order (`compile.featureCompilation`).
2. Decide which generated features run and where their lookups land
   (`compile.featureWriterModes`).
3. Generate kerning: resolve pair values per master (`compile.kernValueLookup`), reconcile
   differing groups (`compile.kernGroupReconciliation`), split by script and direction into
   kern/dist (`compile.kernFeatureWriter`), optional quantisation (`compile.kernQuantisation`).
4. Generate mark, mkmk, abvm, blwm, curs from anchors (`compile.markFeatureWriter`).
5. GDEF: classes (`compile.gdefClasses`), ligature carets (`compile.gdefLigatureCarets`), and in a
   variable font the variation store for all varying values.
6. Feature variations from rules or bracket layers (`compile.featureVariations`).
7. Pack subtables; failure to pack is an error (`compile.layoutPacking`).

`--skip-features` style preview builds stop before this stage (`compile.skipFeatures`).

### 1.7 Variation tables

Only when at least one axis has a range (`compile.variableOnlyTables`).

- fvar and its names (`compile.fvarInstances`, `compile.nameIdAllocation`).
- avar, version 1 or 2 (`compile.avarMaps`).
- gvar: per-glyph deltas with phantom points, IUP at 0.5 units, shared tuples
  (`compile.gvarEveryGlyph`, `compile.gvarPhantomPoints`, `compile.gvarIupOptimisation`,
  `compile.gvarSharedTuples`).
- HVAR and VVAR, smaller of direct and mapped stores (`compile.hvarStoreChoice`).
- MVAR for tagged global metrics.
- STAT (`compile.statDefaults`), also allowed in static fonts.
- CFF2 instead of gvar for the cubic flavour, with its companion tables
  (`compile.cff2VariableBuild`, `compile.cff2CompanionTables`).
- cvar when hinted control values vary (M10).

### 1.8 Assemble and checksum

- sfnt version from outline flavour (`compile.sfntVersion`).
- Recommended physical table order, tag-sorted directory, 4-byte padding (`compile.tableOrder`).
- Per-table checksums and `head.checkSumAdjustment` (`compile.checksums`).
- Deterministic output under a fixed `SOURCE_DATE_EPOCH` (`compile.reproducibleOutput`,
  `compile.timestamps`).

write-fonts's `FontBuilder` does all of this for single fonts (verified in
`src/font_builder.rs`).

### 1.9 Container

- Plain `.ttf` / `.otf`: the assembled bytes.
- WOFF: per-table zlib (`compile.woffEncoding`).
- WOFF2: glyf/loca transform, DSIG dropped, Brotli (`compile.woff2Encoding`).
- Collections: shared identical tables (`compile.collectionSharing`).
- TrueType autohinting, when requested, runs on the finished font before wrapping
  (`compile.ttfAutohintPostStep`).

Output names and format lists: `compile.outputNaming`, `compile.outputFormats`.

### 1.10 Stage-independent rules

- The stage split itself is our design (`compile.pipelineStages`), and every stage is checked
  against the reference compilers by the oracle harness (`compile.oracleComparison`).
- Composite details: `compile.componentFlags`, `compile.useMyMetrics`.
- The individual OS/2 rules behind the `compile.os2*` shorthand used above:
  `compile.os2Version`, `compile.os2WeightWidthClass`, `compile.os2FsSelection`,
  `compile.os2VendorId`, `compile.os2FsType`, `compile.os2AvgCharWidth`,
  `compile.os2UnicodeRanges`, `compile.os2CodePageRanges`, `compile.os2CharIndexes`,
  `compile.os2MaxContext`.
- CFF naming data must agree with name and post (`compile.cffNamesAndStrings`).
- Colour: clip boxes and gradient geometry (`compile.colrClipBoxes`,
  `compile.colrGradientGeometry`).
- Hinting: instructions travel as bytes (`compile.instructionBytes`).

### 1.11 AFDKO-compatible mode

A project that carries AFDKO side files is compiled with makeotf's rules instead of the defaults
above. The rules are recorded so the mode can be implemented without re-reading the guide:

- Draft versus release (`compile.makeotfReleaseMode`) and the precedence of fontinfo, project file
  and command line (`compile.makeotfOptionPrecedence`).
- Menu names from `FontMenuNameDB` (`compile.fontMenuNameDb`, `compile.styleLinking`).
- Order, aliases and Unicode values from `GlyphOrderAndAliasDB` (`compile.glyphOrderMakeotf`,
  `compile.goadbRenaming`, `compile.unicodeFromGlyphNames`).
- CID-keyed sources and their CMap resources (`compile.cidKeyedFonts`,
  `compile.cidCmapResources`).
- Legacy switches we expose only here: `compile.cmapLegacyOptions`,
  `compile.kernClassOptimisation`. Synthetic glyphs are not reproduced
  (`compile.makeotfSyntheticGlyphs`).

### 1.12 Export

`file.exportStatic`, `file.exportAll` and `font.compile` share one way of choosing, naming and
writing fonts; `font.compilePlan` describes the same choices without building.

| Question | Rule | Row |
|---|---|---|
| Which fonts | The exportable static instances; the first master when the font has no static instance. Named instances are built whether they export or not. `font.compile` can add each master | `export.staticUsesInstancesOrDefault` |
| Instances between masters | Skipped with a reason in the result until interpolation (M9). An export that builds nothing fails | `export.instancesBetweenMasters` |
| File name | The instance's `fileName` parameter, else the PostScript name (name ID 6, as the compiler derives it), plus `.ttf`, `.otf`, `.woff` or `.woff2`; characters a file name cannot hold are dropped. A font with no family name takes the source file's name without its extension as the family for the file name only (`Draft.glyphs` gives `Draft-Bold.ttf`, where fontc writes `-Bold.ttf`); the `name` table keeps the empty family | `export.fileNaming`, `compile.outputNaming` |
| Both flavours at once | Each into a `ttf` or `otf` subfolder; a repeated file name keeps the later font, with a warning | `export.flavourSubfolders` |
| One file as destination | Its extension sets the flavour and container; it receives the first font that can be built | `export.singleFileDestination` |
| Failure | Files are staged under temporary names and renamed only when every font is built; a failure leaves the destination as it was | `export.failingExportWritesNothing` |
| Overlaps | Removed by default (`removeOverlap`, `outlines.removeOverlaps`, `--keep-overlaps`); `font.compilePlan` reports the setting per output. A glyph whose removal fails or runs out of time keeps its overlaps, with a warning naming it | `export.removeOverlapOption` |

WOFF2 uses the container's default options: `glyf` and `loca` transformed, `hmtx` not, as
fontTools does.

---

## 2. Table matrix

"Needed for": S = static TTF, O = static OTF (CFF), V = variable TTF, C = variable OTF (CFF2);
lower-case = optional or conditional. "Gap" says what is missing in the crates, not in the
reference compilers.

| Tag | Needed for | Read crate | Write crate | Gap |
|---|---|---|---|---|
| head | S O V C | read-fonts | write-fonts | none |
| hhea | S O V C | read-fonts | write-fonts | none |
| hmtx | S O V C | read-fonts | write-fonts | none |
| maxp | S O V C | read-fonts | write-fonts | instruction limits are ours to compute when hinted |
| OS/2 | S O V C | read-fonts | write-fonts | none; range-bit tables and code page heuristics are compiler logic (fontc has them) |
| name | S O V C | read-fonts | write-fonts | Mac-platform encodings if we emit Mac records |
| cmap | S O V C | read-fonts | write-fonts | builders only for formats 4, 12, 14; legacy Mac subtable (format 0/6) has structs but no builder |
| post | S O V C | read-fonts | write-fonts | helper only for 2.0; 3.0 by setting fields |
| glyf | S V | read-fonts | write-fonts | no instruction assembler; no cubic outlines |
| loca | S V | read-fonts | write-fonts | none |
| CFF  | O | read-fonts | none-yet | whole writer: DICTs, INDEXes, charsets, charstrings, specialiser, subroutiniser, CID structures |
| CFF2 | C | read-fonts | none-yet | whole writer beyond header and INDEX: DICTs, blend charstrings, variation store, FDSelect, subroutiniser |
| vhea, vmtx | s o v c | read-fonts | write-fonts | none |
| VORG | o c | read-fonts | none-yet | generated code exists in the repository but is not compiled into the crate |
| GSUB | s o v c | read-fonts | write-fonts | none (contextual lookups assembled by fea-rs) |
| GPOS | s o v c | read-fonts | write-fonts | none |
| GDEF | s o v c | read-fonts | write-fonts | none |
| BASE | s o v c | read-fonts | write-fonts | no builder from a source model; fontc only passes through FEA output |
| fvar | V C | read-fonts | write-fonts | none |
| avar | v c | read-fonts | write-fonts | none (v1 and v2) |
| gvar | V | read-fonts | write-fonts | none (IUP included) |
| HVAR | V C | read-fonts | write-fonts | none |
| VVAR | v c | read-fonts | write-fonts | none |
| MVAR | v c | read-fonts | write-fonts | none |
| STAT | s o V C | read-fonts | write-fonts | none |
| cvar | v | read-fonts | write-fonts | source model for varying control values |
| gasp | s v | read-fonts | write-fonts | none |
| cvt  | s v | read-fonts (raw i16 slice) | own | trivial serialiser |
| fpgm, prep | s v | own (raw bytes) | own | assembler and disassembler |
| COLR | s o v c | read-fonts | write-fonts | none in the crate; v1 variable paints need our source model |
| CPAL | s o v c | read-fonts | write-fonts | none |
| sbix | s v | read-fonts | write-fonts | builder from placed images |
| SVG  | s o | read-fonts | none-yet | document index writer, SVG generation |
| CBDT (+CBLC) | s | read-fonts | none-yet | both writers |
| EBDT (+EBLC, EBSC) | s | read-fonts | none-yet | both writers |
| kern | s | read-fonts | none-yet | format 0 writer |
| meta | s o v c | read-fonts | write-fonts | none |
| DSIG | s o v c | read-fonts | write-fonts | stub only; no signing |
| hdmx | s | read-fonts | none-yet | generated code not compiled in; needs interpreter runs |
| LTSH, VDMX | s | own | none-yet | both directions |
| MATH | s o | read-fonts | write-fonts | source model |
| JSTF | s o | own | none-yet | both directions; pass-through only |
| morx (+feat, ltag), kerx (+ankr), trak | s | read-fonts | none-yet | pass-through only |
| VARC | v | read-fonts | write-fonts | experimental |

Containers: read-fonts reads collections (`CollectionRef`); nothing in fontations reads or writes
WOFF or WOFF2, and `TTCHeader` writing panics as unsupported.

Counts from the inventory: 50 table rows, of which 32 are written with write-fonts, 4 are trivial
"own" byte arrays and 14 have no writer yet.

---

## 3. Gaps we must fill ourselves

Ordered by how much they block.

### 3.1 CFF writing (blocks static OTF, M2)

Verified state of write-fonts 0.54.0:

- `src/ps/cff.rs` declares one submodule, `v2`. There is no CFF version 1 module.
- `generated/generated_cff.rs` (941 lines: header, INDEX, FDSelect formats 0/3/4, charset formats
  0/1/2) exists in the repository but no `include!` references it, so it is not part of the crate.
- No DICT operand/operator encoder, no charstring encoder, no specialiser, no subroutiniser
  anywhere under `write-fonts/src` (searched for "dict", "charstring", "subr").
- The `skera` subsetter imports the CFF table types but has no CFF module in `src/`.

fontc cannot help either: its backend merges only TrueType-flavoured tables (the table list in
`fontbe/src/font.rs` has glyf/loca/gvar and no CFF).

What we build:

1. Type 2 number encoding and DICT serialisation (Top, Private, Font DICT), with two-pass offset
   resolution.
2. INDEX, String INDEX with standard-string lookup, charset formats, FDSelect, FDArray.
3. Charstring encoder from our path model: hint operators (hstem/vstem, hintmask, cntrmask),
   moves, lines, curves, width prefix.
4. Specialiser equivalent to fontmake's `--optimize-cff 1` (`compile.cffOptimisationLevels`).
5. Width optimisation (`compile.cffWidthOptimisation`) and FontBBox (`compile.cffFontBBox`).
6. Subroutiniser (`compile.cffSubroutinisation`): a separate, optional pass with a cap on
   subroutine count and nesting depth 10 (the limit read-fonts enforces when reading).
7. CID-keyed variant (`compile.cidKeyedFonts`), M12.

Verification plan: read our output back with read-fonts's charstring evaluator and compare
outlines; compare against fontmake and makeotf builds through the oracle harness.

State (M2.3): items 1 to 5 are done for name-keyed fonts without hints (`crates/cff`, byte for
byte as fontTools writes them), and the compiler builds static CFF fonts with them
(`Flavour::Cff`, `crates/compile/src/cff.rs`). What the compiler supplies follows ufo2ft's
`OutlineOTFCompiler`:

| Part | Value |
|---|---|
| Outlines | the decomposed contours in their drawn direction (no reversal); quadratic segments made cubic by fontTools' two-thirds rule; a closed contour's closing line left out unless the point before it sits on the start; the compiler's own `.notdef` keeps its closing lines, as ufo2ft draws it |
| Glyph names | the charset holds the names `post` would hold; `post` is 3.0 |
| Top DICT | font name = name ID 6; `version` = `major.minor`; `Notice` and `Copyright` from the trademark and copyright with `©` spelt out and reduced to printable ASCII; `FullName` and `FamilyName` from the PostScript full name or the typographic names of the `name` table; `Weight` only from `postscriptWeightName`; `ItalicAngle` always real; `FontMatrix` `1/upem` |
| Private DICT | blue values from the master's alignment zones (rounded), and only with them `BlueFuzz` (0), `BlueShift` (7), `BlueScale` (`3/(4 × tallest zone)`) and `ForceBold`; `StemSnapH/V` sorted with `StdHW/VW` the first stem of each direction, only when both directions have stems; widths optimised unless the source sets `postscriptDefaultWidthX` or `postscriptNominalWidthX` |
| Other tables | `maxp` 0.5; `hmtx` side bearings and `vmtx` top side bearings from the charstring bounds rounded half up; `hhea`/`vhea` extents from the bounds rounded outwards; `head` box = `FontBBox` |

Where FontCraft's CFF differs from ufo2ft's on purpose:

| What | ufo2ft | FontCraft | Why |
|---|---|---|---|
| A full or family name that Latin-1 cannot hold | the build fails (fontTools encodes CFF strings as Latin-1) | written with `?` for those characters, and a warning | a compile does not fail over a name |
| A font whose `name` table has no PostScript name | n/a (glyphsLib names the family `Unnamed font`) | the typographic family and subfamily joined by a hyphen | follows the M2.2 name rules (fontc's) |

ufo2ft writes no `LanguageGroup` or `ExpansionFactor`, whatever the source says, and neither does
FontCraft.

### 3.2 CFF2 writing (blocks variable OTF, M9)

write-fonts gives us `Cff2Header` (raw Top DICT bytes plus a raw tail) and `Index::from_items`.
Everything inside is ours: Top DICT, FDArray, Private DICTs with `blend`, the charstrings with
`blend` and `vsindex`, the embedded ItemVariationStore (the write-fonts store builder can produce
the store bytes), FDSelect, and CFF2 subroutinisation. Shares items 1 to 4 and 6 of the CFF list.

### 3.3 TrueType instruction assembly (M10)

write-fonts carries instruction bytes verbatim on glyphs; fpgm, prep and cvt have no table types
at all. We need an assembler from our hinting model to bytecode, a disassembler for import, and
the maxp limits (`maxFunctionDefs`, `maxStorage`, `maxStackElements`, `maxSizeOfInstructions`,
`maxTwilightPoints`, `maxZones`) that fontc hard-codes to zero. skrifa's interpreter can serve to
validate and to measure stack depth, but it is a consumer, not a producer.

### 3.4 Containers

- WOFF2 encoder: the glyf/loca transform, the optional hmtx transform, the table directory with
  known-tag indices, and Brotli. Decoder for import as well.
- WOFF 1 encoder and decoder (zlib, optional metadata block).
- Collection writer with table sharing.

State (M2.4b, `crates/container`): WOFF 1 both ways, and WOFF2 both ways with the `glyf`/`loca`
and `hmtx` transforms (on as fontTools has them by default: `glyf`/`loca` yes, `hmtx` on request);
collections are M12. Differences from fontTools, all checked with `crates/container/crosscheck`:

| Difference | Reason |
|---|---|
| WOFF 1 zlib streams differ (0 of 340 files byte-identical; in 57 a table one implementation shrinks is left raw by the other) | `miniz_oxide` is not zlib. A C zlib cannot build for wasm; `zlib-rs` matched zlib on 104 of 720 sampled tables, not enough to be worth a second deflate crate. Headers, directories and inflated tables are identical in all 340 |
| WOFF2 Brotli streams differ in 154 of 340 fonts untransformed, 319 with the `glyf` transform, 316 with `hmtx` too | the `brotli` crate and the C library (1.0.9 to 1.2.0 give the same result) choose other encodings at quality 11. Headers, directories and decompressed (transformed) streams are identical in all 340 |
| fontTools' WOFF2 output has other `head` timestamps when the source's are below 0x7C259DC0 (208 corpus fonts, mostly test fonts with zero dates) | its `head` reader reads such times as Unix time and shifts them, and its WOFF2 writer recompiles `head` to set bit 11. A container should not change dates, so FontCraft keeps them |
| A damaged glyph, or a `loca` that does not fit `glyf`, leaves `glyf` and `loca` untransformed; an `hmtx` whose size does not match `hhea`, `maxp` and `loca` stays untransformed | fontTools fails or drops data there. Wrapping must not lose data |
| A point with the cubic flag in any combination keeps `glyf` untransformed | fontTools only checks for a flag byte equal to the cubic bit alone, so a cubic point that also carries another kept flag would lose its cubic bit |
| The rebuilt `hmtx` keeps `hhea.numberOfHMetrics` (1 of 340 fonts differs: `Oswald-Stencil.ttf`) | fontTools' reader recompiles `hmtx`, which shortens it to the fewest metrics, and leaves `hhea` as it was, so the two disagree |

The rebuilt `glyf` follows fontTools' reader: glyphs written with greedy flag and coordinate
packing and recomputed component flags, unpadded except that odd-sized glyphs get one byte when
that keeps `loca` short. On fontTools' own WOFF2 files our reader gives the tables fontTools
reads for 340 of 340 with the default transforms and 339 of 340 with `hmtx` (the row above), and
for the 49 WOFF2 files in the corpus.

### 3.5 Bitmap and SVG colour tables (M11)

SVG document index; CBDT/CBLC index subtables and glyph bitmap formats. sbix has types but no
builder from images.

### 3.6 Legacy and rare tables (M12)

- kern format 0 from flattened pair kerning.
- VORG (small; wire up or re-implement), hdmx, LTSH, VDMX.
- EBDT/EBLC/EBSC, JSTF, AAT tables: pass-through of existing bytes with glyph-id remapping where
  feasible; generation is not planned.

### 3.7 Compiler logic we own even where a crate has it

fontc 1.0.0 can be called as a library: `fontc::generate_font` takes a source and returns the
font's bytes, and a source can be built from `.glyphs` text held in memory (checked on 2026-10-07;
see `docs/decisions/0002-compile-pipeline.md`). It compiles a whole font in one batch from its own
reading of the source, and its intermediate representation is internal. So it serves as an oracle
and as a bootstrap backend through the file format, and the decisions are still ours to implement
for the editor's own pipeline: metric defaults, name derivation, OS/2 bit tables, kern and mark
generation, the variation model, feature-variation region splitting, COLR geometry. fea-rs is the
crate we do build on: it exposes its compilation result (GSUB, GPOS, GDEF, BASE, STAT, name, OS/2,
head, hhea, vhea) for reuse.

### 3.8 Overlap removal

fontc does none; fontmake calls Python libraries. Ours comes from the path-operations engine
(`fontcraft_pathops::remove_overlap`, the operation behind Path › Remove Overlap) and is on by
default for static export.

| Question | Rule |
|---|---|
| Which glyphs | Every glyph that has contours after decomposition. A composite keeps its components, so components that overlap each other stay (ufo2ft's `RemoveOverlapsFilter` skips glyphs without contours the same way). In CFF every glyph is decomposed first |
| Where in the order | After decomposition, before cubic-to-quadratic conversion and the TrueType direction change (`compile.filterOrder`) |
| Strokes and masks | Applied before, per layer, in shape order (`fontcraft_compile::drawing`): such a glyph is contours only |
| Result | The nonzero union of the closed contours, outer contours counter-clockwise; result points are not rounded until the glyph is written (rounding them first put intersection points a unit away from fontmake's in 22 more Oswald glyphs); points of the input that survive keep their exact place |
| Open paths | Left out, with a warning naming the glyph: the union takes closed paths only |
| Failure and hangs | Each glyph runs on a worker thread with a 5 s limit (`fontcraft_pathops::run_with_time_limit`; docs/plans/M3.md › M3.6, rule 8). A glyph that fails or runs out of time keeps its overlaps, and a warning names it. In the browser the union runs inline, without a limit |
| Oracle | Against fontc, FontCraft builds with overlaps kept. Against fontmake both sides remove them, and outlines that differ in their points are compared by rendered coverage ("coverage-equal", `docs/testing.md`) |

Against fontmake (booleanOperations, its default backend) over the corpus comparison set, M3.7:

| Flavour | Glyphs compared | Same points | Coverage-equal | Different from overlap removal |
|---|---|---|---|---|
| TrueType | 2,360 | 2,191 | 125 | 10, all in Oswald |
| CFF | 2,361 | 1,818 | 506 | 3, all in Oswald |

No glyph timed out or failed. The remaining glyph differences of the table are older,
allow-listed causes unrelated to overlaps.

The differences that overlap removal causes are on `crates/oracle/allow.toml`, for fontmake only:

| Difference | Why |
|---|---|
| A `glyf` or `CFF` outline whose coverage differs by less than a whole pixel anywhere and under 0.1% of the ink in total | An intersection point that falls near a half unit is rounded to the other side (327.5 written as 328 against fontmake's 327), or one engine leaves a point on a straight line where the other merges two segments |
| `maxp.maxPoints` | The two engines leave different numbers of points where contours merge (FontCraft keeps a point on a straight line where two contours met, docs/plans/M3.md › M3.6, rule 7) |

fontmake's overlap removal fails on five corpus sources that build with overlaps kept:
booleanOperations takes no quadratic segments (`glyphs2/QCurve.glyphs`, `glyphs3/QCurve.glyphs`)
and stops with an internal error on the three `DoubleComponentError` sources. FontCraft builds
all five.

---

## 4. Where the references differ

| Decision | fontmake (Python) | fontc | makeotf / AFDKO | FontCraft default |
|---|---|---|---|---|
| Output flavours | TTF, CFF, CFF2, variable TTF and CFF2 | TrueType-flavoured static or variable only | CFF from one source; CFF2 by merging masters | all four |
| Overlap removal | on for static, off for variable; two backends | never | not part of makeotf | on for static, off for variable |
| Overlap flag bits | not stated in the docs read | never set | n/a | set on variable TTF glyphs |
| cu2qu tolerance | 0.001 em, `-e` to change | upm/1000, fixed | n/a | 0.001 em, option |
| Curve modes | cu2qu, mixed, keep-quad, keep-cubic | cu2qu only | n/a | cu2qu, keep-quad |
| Contour direction | reversed for TTF | reversed for TTF | source direction | reversed for TTF |
| Implied on-curve points (an on-curve point exactly between two off-curve points) | kept unless `--drop-implied-oncurves` | left out | n/a | left out, as fontc; the oracle harness passes the flag to fontmake |
| Mixed contour+component glyph | decomposed | decomposed, or contours split off when preference is off | all decomposed (CFF) | decomposed |
| Flatten nested components | off, `-f` | source setting, flag overrides | n/a | off |
| USE_MY_METRICS | static only, switchable | static only, no switch | n/a | static only |
| IUP on gvar | on, `--no-optimize-gvar` | on, no switch, tolerance 0.5 | n/a | on, option |
| Glyph order | source order (via ufo2ft) | source order, rest sorted, .notdef first | CFF standard names, then GOADB, then source; CID order | fontc rule; AFDKO mode optional |
| Missing .notdef | generated (per fontc comments on ufo2ft) | generated box, variable if metrics vary | kept as is; `-addn` substitutes a designed one | generated box |
| Production names | map + uniXXXX + ligature naming | the source's map only (front ends fill it) | GOADB aliases, only with `-r`/`-ga` | map + uniXXXX |
| Unicode assignment | from source glyphs | from source glyphs | GOADB, then AGLFN, then uniXXXX names | from source glyphs |
| cmap subtables | (0,3)+(3,1) format 4; (0,4)+(3,10) format 12 if needed | same (uses write-fonts) | adds a Mac subtable; UVS from a file | fontc rule |
| name platforms | Windows only | Windows only | Windows + Mac unless `-omitMacNames` | Windows only, Mac optional |
| Version string | `Version M.mmm` | same plus `;fontc <ver>` | `Version x.xxx;addfeatures <ver>`, "Development" unless `-r` | fontc style, stamp optional |
| Unique id | `ver;vendor;psname` | same | `x.xxx;vendor;FontName[;license]` | fontc style |
| Vendor id default | four spaces (fontTools) | `NONE`, also for four spaces | from notice string, else `UKWN` | `NONE`, as fontc |
| fsType default | not checked | bit 2 for UFO, bit 3 for Glyphs | from source font | explicit in source |
| OS/2 version | 4 (per a write-fonts comment) | 4 | 3, or 4 when bits 7 to 9 used | 4 |
| Typo metrics default | ascender / descender / max(0, 1.2 em + desc − asc) | same | from d-height or cap height; desc = asc − em; gap = 1.2 em − asc + desc | fontc rule |
| Win metrics default | asc + line gap / abs(desc) | same | font box top / −bottom | fontc rule, box-based optional |
| hhea metrics default | asc + typo gap / desc / 0 | same | copies typo values | fontc rule |
| x-height, cap height default | 0.5 em, 0.7 em | same | measured from o and O | measured when glyphs exist, else fontc values (open question) |
| Underline position default | −0.075 em | −100 for `.glyphs`, −0.075 em for UFO | from source font | as fontc (the UFO reader stores ufo2ft's value) |
| lowestRecPPEM | 6 | 6 | 3 | 6 |
| xAvgCharWidth | mean of non-zero advances, rounded | same | same mean, truncated | rounded |
| post version | 2.0 for TTF | 2.0 always | 3.0 for CFF; CFF2 3.0 unless `-k` | 2.0 TTF and CFF2, 3.0 CFF |
| Timestamps | now, or SOURCE_DATE_EPOCH (fontTools) | source created date, now for modified, SOURCE_DATE_EPOCH | now | fontc rule |
| CFF subroutinisation | on by default (level 2) | n/a | off unless `-S` or `-r` | on for release, off for draft |
| Feature compiler | feaLib; MTI optional | fea-rs | addfeatures (own parser) | fea-rs |
| Generated kern/mark | ufo2ft writers, modes skip/append, quantisation option | same modes, no quantisation | none; kerning is written in the feature file | fontc rule, quantisation optional |
| Kern class trick | n/a | n/a | left class 0 optimisation unless `-skco` | not used |
| GDEF classes | generated block unless `--no-generate-GDEF` | FEA block wins, else source categories | from feature file | fontc rule |
| Feature variations tag | rvrn/rclt from designspace | same; Glyphs bracket layers default rvrn | designspace grammar noted in CFF2 doc | fontc rule |
| DSIG | none | none | stub in release mode | none |
| TrueType hinting | external ttfautohint after build | none | n/a | own, after build |
| Variable CFF2 route | interpolatable masters merged | n/a | buildmasterotfs + buildcff2vf, or bare CFF2 + addfeatures | direct from source model |
| Option precedence | CLI over UFO lib keys | CLI over source defaults (tri-state flags) | fontinfo < project file < CLI, left to right | export preset < per-export options |

Cells for fontmake marked "per fontc" restate what fontc's comments say ufo2ft does; they were not
read in ufo2ft itself.

---

## 4a. Where FontCraft differs from fontc

The core tables match fontc 1.0.0 over the corpus (`docs/plans/M2.md` › M2.2) except where this
table says otherwise. Each row is also on `crates/oracle/allow.toml` with the sources it shows in.

| What | fontc | FontCraft | Why |
|---|---|---|---|
| A `.glyphs` master's custom parameters (`unicodeRanges`, `vendorID`, `license`, `trademark` and the other name parameters) | read from the font only; only the metric parameters come from the master | the master's in place of the font's, also in place of a font property of the same name | a static build of a master is that master's font, as glyphsLib and the reference app build it; fontmake writes the same values. `Use Typo Metrics` and `Has WWS Names` are the exception: glyphsLib adds their bit for the font's and the master's and never clears it, so a master's that is off leaves the font's on, which is also fontc's result |
| `USE_MY_METRICS` in a multi-master source | never set: the reference is a variable font | set, as in fontc's own static builds | FontCraft builds one master statically until M9; the harness does not compare the flag against a variable reference |
| Composites whose components differ between masters; curves | decomposed; converted across all masters | kept; converted per master | static build of one master until M9 |
| Names and panose of the variable font's export settings | merged into the font's | not used | they belong to the variable font (M9) and instances (M2.5) |
| `openTypeNameRecords` of a UFO | written as English records whatever their language | each in its own language | the records state their language |
| Non-spacing marks the glyph database knows | advance 0 | advance 0 only when the glyph states its category | the glyph database is M4 |
| Yes-or-no custom parameters of a `.glyphs` source (`Use Typo Metrics`, `Has WWS Names`, `isFixedPitch`, `postscriptForceBold`) | on only when the value is exactly 1 | on for any number, or text that reads as a number, other than 0 (`fontcraft_model::param_flag`) | glyphsLib applies `bool(value)`, so fontmake builds a hand-written 2 as on; the reference app writes only 0 and 1. No corpus source has another value, so this row has no allow-list entry |

The readers follow fontc where a source leaves a value out (`docs/spec/30-source-formats.md` ›
Values a source leaves out): a `.glyphs` layer without a `width` is 600 wide; a designspace
master without a trailing metric gets ufo2ft's default; a UFO glyph without a height in a
vertical UFO has height 0; version 2's top-level names in a version 3 file are names. One case
stays different: a designspace master that lacks a metric but has a later one reads the missing
one as 0, where fontc uses ufo2ft's default (no corpus source has it).

Against fontmake (and so for CFF fonts, which fontc does not write), every field the two
references derive differently keeps fontc's value; `crates/oracle/allow.toml` lists those
differences for fontmake with their reasons (`docs/plans/M2.md` › M2.3b).

---

## 5. Commands and data touched

This chapter defines no commands of its own. The stages are driven by the export commands in the
import/export inventory (`file.export` and its headless equivalents) and by `fontcraft-cli`
subcommands defined elsewhere; each stage should also be reachable as an engine query for agents
(for example "give me the final glyph order" or "build only table X"), which the command inventory
for the CLI area should add.

Data read: glyph outlines, components and anchors per master; kerning and groups per master;
feature code; font info (names, metrics, OS/2 and head fields, vendor, fsType, gasp ranges, meta
values); axes, mappings, instances, STAT labels; glyph categories and export flags; production
name map; Unicode variation sequences; colour palettes and layers; hinting data. Data written:
the binary only; compilation never changes the source.

---

## Open questions

1. **Overlap flags in fontmake.** fontc never sets OVERLAP_SIMPLE / OVERLAP_COMPOUND. Whether the
   fontmake/ufo2ft pipeline sets them on variable fonts was not determinable from the files read.
   Check a fontmake-built variable font before fixing our default.
2. **Mac names and Mac cmap.** makeotf writes them by default; modern pipelines do not. Decide
   whether the AFDKO-compatible preset needs them at all, and which Mac encodings to support.
3. **Measured versus fixed x-height and cap height.** The source model normally stores both, so the
   defaults matter only for sparse imports. makeotf measures glyphs; fontc uses fractions of the em.
   Pick one after checking what Glyphs exports for a font with no metrics set.
4. **Underline position default.** fontc chose −0.1 em to match Glyphs; confirm against Glyphs
   output, and decide what a UFO import without the value should get (ufo2ft's −0.075 em would
   keep parity with fontmake for UFO sources).
5. **fsType default per source format.** Bit 2 for UFO and bit 3 for Glyphs in fontc. Our native
   format should store an explicit value; decide the value written on first import.
6. **Vendor id default.** Answered for M2: `NONE`, as fontc 1.0.0 writes it, also for a source
   that sets four spaces (`docs/plans/M2.md` § 3). makeotf writes `UKWN`, fontmake four spaces.
7. **CFF2 table order.** write-fonts and fontTools both pick the CFF physical order only when a
   `CFF ` table is present, so CFF2 fonts get the TrueType list. Harmless, but confirm that this is
   what fontmake-built CFF2 fonts look like before our oracle diff treats order as significant.
8. **post 3.0 for release TrueType fonts.** fontc lists dropping glyph names as a to-do. Decide
   whether release presets drop names and how production renaming interacts with it.
9. **Subroutiniser parity.** compreffor, cffsubr and makeotf's subroutiniser produce different
   byte streams for the same outlines. The oracle can only compare decoded outlines and total size,
   not bytes; define the size budget we accept (for example within a few percent of cffsubr).
10. **makeotf's "Development" wording.** The user guide says release mode removes the word from
    the version string; the exact non-release string was not located in the addfeatures source read
    (`name.cpp` builds `Version x.xxx;addfeatures <client>`). Observe makeotf output if we want the
    AFDKO preset to match byte for byte.
11. **Default and nominal width in fontmake.** fontTools ships an optimiser, but whether the
    ufo2ft pipeline calls it was not checkable here. Compare a fontmake OTF's Private DICT.
12. **Kerning quantisation and rounding order.** fontc rounds master values before computing
    deltas; with quantisation on, decide whether to quantise masters or the default plus deltas.
13. **USE_MY_METRICS with our own hinting.** fontc argues the flag belongs to the hinting stage.
    Once we emit instructions, decide whether the hinting stage owns the flag for variable fonts.
14. **Glyph order for sources without an explicit order.** fontc sorts by name with `.notdef`
    first. Our native format always has an order (Font View order), so this only matters for
    imports; confirm we persist the imported order instead of re-sorting at each compile.
15. **Instance extrapolation.** fontmake's default interpolator refuses it. Decide whether our
    instancer allows extrapolated static instances and how the compile preflight reports them.
16. **VORG, hdmx in write-fonts.** Both have generated code that is not compiled into 0.54.0.
    Check newer write-fonts releases before writing our own.
17. **STAT format 4.** write-fonts can write it and fontc does not emit it; decide whether our
    Font Info exposes multi-axis labels.
18. **GDEF class inference.** fontc infers base and ligature classes from anchors for Glyphs
    sources only. Our native sources carry Glyphs-style categories, so we need to state which rule
    applies to them and to UFO imports.
