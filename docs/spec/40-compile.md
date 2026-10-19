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

- Static instance: interpolate masters at the instance location first
  (`compile.instanceInterpolation`, `fontcraft_interp::instance::instance_font`): the result is a
  font with one master at the instance, which resolves like an instance at a master. An instance
  at a master goes the same way when an alternate layer applies there. An instance beyond the
  masters is extrapolated and named in the export report (§ 4a). Kerning interpolates as
  fontmake's instancer does (`compile.kernGroupReconciliation`); outlines are not rounded before
  the compile, as fontmake leaves them by default. Layout values written in feature code are not
  interpolated (`compile.instanceLayoutInterpolation`).
- Variable font: keep all masters; single-point axes are removed from the variation model
  (`compile.variableOnlyTables`).
- Apply instance glyph filters here (`compile.subsetOnExport`): Keep Glyphs and Remove Glyphs
  unless the subset switch is off, as fontmake's `--no-subset` ignores them.
- Source preparation, before the master is resolved, as both references do when they load a
  source: corner components are fitted into their paths, then smart components are drawn as
  outlines (`fontcraft_compile::prepare_source`). A layer whose corners cannot be fitted, or a
  smart component that cannot be interpolated, is written as it is, with a warning.
- Glyphs-style pre-steps when the source enables them: erase open corners, propagate anchors;
  propagation precedes any flattening (`compile.glyphsOutlinePreSteps`). Propagation reads the
  source as stored, so a smart component passes on its glyph's anchors, interpolated at the
  component's values; a pole layer drawn with components gives what its components pass on, as
  glyphsLib does. A smart component that cannot be interpolated passes on its glyph's layer
  anchors instead, with a warning. A glyph
  whose anchors cannot be worked out keeps its own, with a warning; the rest of the master
  propagates.

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
   all masters converted together segment by segment (`compile.cu2quAcrossMasters`). The
   `keep-quad` mode converts nothing and refuses a glyph with a cubic curve; `mixed` and
   `keep-cubic` write cubic curves into `glyf` and wait for that format to be final
   (`compile.ttfCurveModes`). On-curve points halfway between two off-curve points are left out,
   as fontc writes them; the option to keep every on-curve node writes glyphs as fontmake does by
   default (`compile.dropImpliedOnCurves`).
4. **Direction** (TrueType only). Reverse contours unless told to keep them
   (`compile.reverseContourDirection`).
5. **Rounding.** Half-up everywhere, masters rounded before deltas (`compile.rounding`); CFF
   rounding tolerance 0.5 by default (`compile.cffRoundTolerance`).

### 1.4 Glyph order and names

- The `glyphOrder` parameter's glyphs, then the rest in stored order (`fontcraft_model::glyph_order`,
  the order Font View shows), `.notdef` forced to id 0 and generated if absent
  (`compile.glyphOrder`, `compile.notdefFirst`, `compile.notdefSynthesis`). AFDKO-style ordering is
  an alternative mode (`compile.glyphOrderMakeotf`); CID-keyed fonts use CID order
  (`compile.cidKeyedFonts`).
- Compiler-made glyphs (bracket layers, colour layers) are appended (`compile.bracketLayerGlyphs`).
- Production renaming and sanitising (`compile.productionNames`,
  `compile.productionNameSanitising`, `crates/compile/src/names.rs`): a glyph's stored production
  name, else the glyph database's; for a UFO only its `public.postscriptNames`. Names are cleaned to
  `A-Z a-z 0-9 . _` and made unique with `.1`, `.2`… as fontc does. *Don't use Production Names* and
  *Use Custom Naming* keep the source names. GOADB renaming for AFDKO projects (`compile.goadbRenaming`,
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

Built so far (M5.17, `crates/features`): step 3 for one master of a static build whose source has
no hand-written `kern` feature, ported from fontc's kern writer (`fontbe/src/features/kern.rs`). The master's
left-to-right and right-to-left pairs (those of the master it follows when its metrics are
linked) are resolved into rules in lookup order, rounded half up, split into one lookup per group
of scripts (rules with marks in lookups of their own), and assigned to `kern`, or to `dist` for
the Indic and Universal Shaping Engine scripts, Khmer and Myanmar. Glyph pairs come before class
pairs in each lookup; write-fonts splits subtables and moves lookups to extension lookups when
offsets overflow. Each feature lists each of its lookups once, in order, as fea-rs leaves them.
Vertical kerning is not written: neither reference writes `vkrn`.

Built by M8.7 (`crates/compile/src/layout.rs`), for static builds:

- The target's entries are prepared first: the automatic entries the font has are regenerated for
  the target's glyph set unless Update Features is off (Update adds no entry at export), then the
  instance parameters apply. Tokens, lowering and assembly follow (`fontcraft_features`).
- A first pass compiles `GSUB` alone and reads what the feature writers need from the code: its
  language systems, the generated features it declares and whether each holds `# Automatic Code`,
  and the classes of a `table GDEF` block.
- The kern writer adds the code's language systems to the scripts it guesses and registers its
  lookups for each declared language; unencoded glyphs the substitutions reach take the scripts
  and directions of the glyphs they come from (the `GSUB` closure). Lookups that skip marks get
  a mark filtering set of the spacing marks when the font has any.
- A generated feature the code declares without the marker is not generated (skip mode); a writer
  in append mode adds its lookups after every hand-written lookup. A hand-written `kern` entry gets
  the marker at its end when assembled.
- A compile error stops the export with the diagnostics mapped to the entries. Includes are read
  from the folder of the source (the folder that holds a UFO); a path that leaves that folder is
  refused.

The source's feature-writer setting (the `com.github.googlei18n.ufo2ft.featureWriters` key of
the font's user data or the master's UFO lib) is read as fontc reads it: without the key the
kerning is written; with it, only when the list has a valid kern writer entry, whose `features`
option can limit it to `kern` or `dist`. An entry with an unknown option, or a malformed one, is
dropped. Where fontc refuses to build (a value that is not a list, `ignoreMarks` off, a
quantization other than 1, the legacy `kernFeatureWriter2`), FontCraft builds with the standard
settings and warns; a value that is not a list turns the kerning off, as in fontmake.

### 1.7 Variation tables

Only when at least one axis has a range (`compile.variableOnlyTables`).

- fvar and its names (`compile.fvarInstances`, `compile.nameIdAllocation`).
- avar, version 1 or 2 (`compile.avarMaps`).
- gvar: per-glyph deltas with phantom points, IUP at 0.5 units, shared tuples
  (`compile.gvarEveryGlyph`, `compile.gvarPhantomPoints`, `compile.gvarIupOptimisation`,
  `compile.gvarSharedTuples`).
- HVAR and VVAR, smaller of direct and mapped stores (`compile.hvarStoreChoice`).
- MVAR for tagged global metrics.
- Variable kerning and mark positions in GPOS, their deltas in the GDEF variation store
  (`compile.instanceLayoutInterpolation`). Kerning: the rules of every master; a master's value
  of a rule is what its own pairs and groups give it, else 0; a master with no kerning at all
  has no value there, so the others' values interpolate. Marks: the mark writer runs for each
  master and each anchor position varies; when a master's rules differ from the origin's, the
  positions stay the origin's with a warning.
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
| CFF2 | C | read-fonts | own (M9.13) | subroutiniser, specialiser and FDSelect not written |
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

State (M2.3, M10.14): items 1 to 5 are done for name-keyed fonts (`crates/cff`, byte for byte
as fontTools writes them for glyphs without hints; hint operators, masks and flex as
`crates/cff/src/hints.rs` describes), and the compiler builds static CFF fonts with them
(`Flavour::Cff`, `crates/compile/src/cff.rs`). What the compiler supplies follows ufo2ft's
`OutlineOTFCompiler`:

| Part | Value |
|---|---|
| Outlines | the decomposed contours in their drawn direction (no reversal); quadratic segments made cubic by fontTools' two-thirds rule; a closed contour's closing line left out unless the point before it sits on the start; the compiler's own `.notdef` keeps its closing lines, as ufo2ft draws it |
| Glyph names | the charset holds the names `post` would hold; `post` is 3.0 |
| Top DICT | font name = name ID 6; `version` = `major.minor`; `Notice` and `Copyright` from the trademark and copyright with `©` spelt out and reduced to printable ASCII; `FullName` and `FamilyName` from the PostScript full name or the typographic names of the `name` table; `Weight` only from `postscriptWeightName`; `ItalicAngle` always real; `FontMatrix` `1/upem` |
| Private DICT | blue values from the master's alignment zones (rounded), family blues from Family Alignment Zones, and only with them `BlueFuzz` (1), `BlueShift` (7), `BlueScale` (0.037) and `ForceBold` (§ 4a); `StemSnapH/V` sorted with `StdHW/VW` the first stem of each direction, only when both directions have stems; widths optimised unless the source sets `postscriptDefaultWidthX` or `postscriptNominalWidthX` |
| Other tables | `maxp` 0.5; `hmtx` side bearings and `vmtx` top side bearings from the charstring bounds rounded half up; `hhea`/`vhea` extents from the bounds rounded outwards; `head` box = `FontBBox` |

Where FontCraft's CFF differs from ufo2ft's on purpose:

| What | ufo2ft | FontCraft | Why |
|---|---|---|---|
| A full or family name that Latin-1 cannot hold | the build fails (fontTools encodes CFF strings as Latin-1) | written with `?` for those characters, and a warning | a compile does not fail over a name |
| A font whose `name` table has no PostScript name | n/a (glyphsLib names the family `Unnamed font`) | the typographic family and subfamily joined by a hyphen | follows the M2.2 name rules (fontc's) |

ufo2ft writes no `LanguageGroup` or `ExpansionFactor`, whatever the source says, and neither does
FontCraft.

### 3.2 CFF2 writing (blocks variable OTF, M9)

`fontcraft_cff::cff2` writes the whole table (M9.13): header, Top DICT, an empty global
subroutine INDEX, one Font DICT in the FDArray (no FDSelect), a Private DICT with `blend`, the
charstrings with `blend` and `vsindex`, and the item variation store with no items. Not written:
charstring specialisation, subroutines, hints and FDSelect for CID-keyed fonts.

| What | fontmake `-o variable-cff2` | FontCraft |
|---|---|---|
| Charstrings | specialised (`hlineto`, `hvcurveto`…) and subroutinised by `tx` | one `rmoveto`, `rlineto` or `rrcurveto` per segment, no subroutines; same outlines, larger table |
| Blend runs | as many varying operands per `blend` as the 513-operand stack allows | the same; a run past the stack is split |
| Hints | the hints of the masters' CFF tables, merged into blended stems and masks | none, with a warning when the source has manual PostScript hints |
| Private DICT | blended where the masters differ; a list some master lacks keeps the default's | the same, with a warning; BlueScale, BlueShift and BlueFuzz blended too |
| Composites | decomposed in each master; a composite follows its components' intermediate layers | the same: a decomposed composite gains its components' intermediate locations as sources |
| `post` | 2.0 with names (varLib switches CFF2 fonts to it) | 2.0 with names |

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
| name platforms | Windows only | Windows only | Windows + Mac unless `-omitMacNames` | Windows only; Mac records with Export Mac Name Table Entries or Compatible name Table |
| Version string | `Version M.mmm` | same plus `;fontc <ver>` | `Version x.xxx;addfeatures <ver>`, "Development" unless `-r` | `Version M.mmm;FontCraft <ver>` from exports (the `versionStamp` option); a stated version string as written, `%d` forms filled |
| Unique id | `ver;vendor;psname` | same | `x.xxx;vendor;FontName[;license]` | fontc style |
| Static instance names and classes | glyphsLib's style-linked names (the instance's bold and italic flags and base style), its familyNames and style-map parameters; `usWeightClass` and `usWidthClass` from the location on a weight or width axis | no static instances | fontmake's, plus the keys only the handbook describes (§ 4a); without a weight or width axis, the instance's Weight Class and Width Class |
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
| post version | 2.0 for TTF and for CFF2 (varLib) | 2.0 always | 3.0 for CFF; CFF2 3.0 unless `-k` | 2.0 TTF and CFF2, 3.0 CFF |
| Timestamps | now, or SOURCE_DATE_EPOCH (fontTools) | source created date, now for modified, SOURCE_DATE_EPOCH | now | fontc rule |
| CFF subroutinisation | on by default (level 2) | n/a | off unless `-S` or `-r` | static CFF: on unless Disable Subroutines or `subroutinize: false`; a table past the subroutiniser's limits is written without, with a warning. CFF2: off. The draft preset (M12.40) is to turn it off |
| Feature compiler | feaLib; MTI optional | fea-rs | addfeatures (own parser) | fea-rs |
| Generated kern/mark | ufo2ft writers, modes skip/append, quantisation option | same modes, no quantisation | none; kerning is written in the feature file | fontc rule, quantisation optional |
| Contextual anchors (`*top` with a context in `GPOS_Context`) | ufo2ft's mark writer adds chained lookups that call the contextual anchors' attachments | ignored | n/a | as fontmake (M8.21) |
| Kern class trick | n/a | n/a | left class 0 optimisation unless `-skco` | not used |
| GDEF classes | generated block unless `--no-generate-GDEF` | FEA block wins, else source categories | from feature file | fontc rule |
| Feature variations tag | rvrn/rclt from designspace | same; Glyphs bracket layers default rvrn | designspace grammar noted in CFF2 doc | fontc rule |
| DSIG | none | none | stub in release mode | none |
| `meta` header's reserved third field | the offset of the data | 0 | n/a | 0, as fontc and the OpenType spec |
| TrueType hinting | external ttfautohint after build | none | n/a | own, after build |
| Variable CFF2 route | interpolatable masters merged | n/a | buildmasterotfs + buildcff2vf, or bare CFF2 + addfeatures | direct from source model |
| Option precedence | CLI over UFO lib keys | CLI over source defaults (tri-state flags) | fontinfo < project file < CLI, left to right | export preset < per-export options |

Cells for fontmake marked "per fontc" restate what fontc's comments say ufo2ft does; they were not
read in ufo2ft itself.

**Variable fonts: fontc against fontmake.** The variable comparison (`fontcraft-cli oracle
--variable`, M9.10) was run with fontc 1.0.0's variable font on one side and fontmake 3.12.1's
on the other, at each font's default, axis ends and named instances. Of the 133 corpus sources
both build, 112 agree in every check. The 21 others differ in these ways, which M9.5 to M9.9
decide when FontCraft's variable font is compared with each:

| What | fontc | fontmake | Sources |
|---|---|---|---|
| `STAT` of a source with italic instances | no `ital` axis | an `ital` axis with a Roman value (format 3, linked to Italic) or an Italic value | 7 (`StatLabels`, `IntermediateLayer`, …); `StatLabels` also has an instance Black in fontc's `fvar` only |
| An Axis Location or Axis Mapping beyond the masters | `wght` 400–700 | `wght` 400–900 and 300–900; `avar` and the advances at the axis end differ with it | 2 (`AxisLocationBeyondMasters`, `AxisMappingsBeyondMasters`) |
| The `fvar` default of `UnnamedMaster` (both files) | 100 | 400 | 2 |
| Instances without an Axis Location (`AxisLocationInstanceWithout`) | at `opsz` 1 and 2, with an `avar` | at `opsz` 16 and 36, no `avar` | 1 |
| A generated `.notdef`, and `MVAR` where masters' metrics differ | `.notdef` the same at every location; `MVAR` deltas 0 | `.notdef` and the `cpht`, `hasc`, `hcla` deltas vary with the masters (up to 800 units) | 3 (`MasterNames`, `MasterNames-Italic`, `TheBestV2NamesInAV3File`) |
| A generated `.notdef` at a midpoint | rounded one way | one unit off | 2 (`wght_var`, `wght_var_wide_axis`) |
| Pairs with alternate (bracket) glyphs (`BracketTestFontKerning`) | `x x.BRACKET.varAlt01` −50 | no pair | 1 |
| Colour layer glyphs | marks attach to `A.color0`; `B.color0` advance 500, the colour layer's; the layer glyph of a glyph with a production name is named after it (`uni0311.color0`) | no attachment; advance 600, the base glyph's (0 for a non-spacing mark); the layer glyph keeps its own name (`mymark.color0`) | 3 (`COLRv0-component-anchors`, `color-and-bracket-glyph-order`, `COLRv0-marks`); FontCraft writes them as fontc does |
| Kerning of a linked master at the midpoint (`KerningLinkedMaster`) | `A B` −3.333 | −40 | 1 |

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
| *Use Custom Naming* (`disablesNiceNames`) | ignored: production names are written | the source names are written | the handbook gives it the effect of *Don't use Production Names*; glyphsLib also ignores it (allow-listed for `CornerComponents.glyphs`) |
| A UFO that glyphsLib wrote, with a non-spacing mark's width as 0 in the GLIF and the drawn width in `com.schriftgestaltung.Glyphs.originalWidth` | advance 0 | the drawn width | the model reads the drawn width so the UFO converts back to the reference's format unchanged, and the compiler takes a UFO's widths as they are (allow-listed for the two `designspace_from_glyphs` sources) |
| A pair in both the left-to-right and the right-to-left kerning of a master, and groups of glyphs with right-to-left kerning | the two tables are joined: the right-to-left value replaces the left-to-right one, and a glyph with right-to-left kerning in any master is given by its other group on each side of every pair | the same, with a warning that counts the replaced pairs and the glyphs whose groups swap sides in left-to-right pairs | fontc's join, kept so the values agree; glyphsLib shares the group names between the directions the same way. The warning tells the user that left-to-right kerning changed |
| Variable `pos` values (`(wght=400:10 wght=700:20)`) in a static build of one master | compiled into the variable font | the static export stops: fea-rs refuses variable metrics outside a variable font; the variable export compiles them | a static build has no axes to resolve the locations against. `varpos*` and `variable_fea/VarFeaOneShot*` in the corpus |
| Contextual anchors (`*top`, `*bottom.alt`, `*top_2` with a context) | not built: the anchors and their contexts are ignored | chained contextual mark lookups as fontmake builds them; when one is written, the generated `mark`, `mkmk`, `abvm` and `blwm` lookups move into the same generated code | the reference app builds them; fontmake is the oracle. No corpus source has contextual anchors, so the allow list has no entry |
| Anchor propagation in a `.glyphs` source with a ufo2ft filter list (`com.github.googlei18n.ufo2ft.filters`) | propagates only when the list has a `propagateAnchors` entry | follows the Propagate Anchors setting, as fontmake does | the reference app knows no ufo2ft filters; its setting decides. `UfoFiltersDontPropagateAnchors.glyphs` and `UfoFiltersWithoutPropagateAnchors.glyphs` differ; the marks measure lists them (`KNOWN` in `crates/oracle/tests/marks.rs`) |
| An `hhea` or `vhea` table block that leaves out some of its metrics | the metrics left out are written as 0 (the defaults of fea-rs's table) | the metrics left out keep their computed values | feaLib, and so fontmake, changes only the metrics the block writes, which is what the feature file specification describes; no corpus source leaves one out (`compile.feaTableOverrides`) |
| A variable font whose feature code declares names (stylistic set labels, a `name` block with IDs of 256 and above) | the code's IDs start above the axis and instance names | the code's IDs start above the source's static names; a code name whose ID meets an axis or instance name is left out with a warning | the variable names are allocated after the layout stage (`compile.nameFeaOverrides`; M9) |
| Marks of a UFO source in the kern writer's split | from `public.openTypeCategories` | from the glyph's stored category and subcategory | the compiler does not fill a UFO's categories from the glyph database (`compile.nonSpacingMarkWidth`); the rules with marks land in other lookups, with the same values |
| The scripts of the generated kerning of a source whose masters kern in different directions | taken from every master's pairs: the variable font gets a script for pairs only another master has, holding only those pairs | taken from the built master's pairs | FontCraft builds one master statically until M9. In `KerningRtlOnlyMaster.glyphs` only the second master has right-to-left pairs, so a Latin pair read in `hebr` is 0 in fontc's font and the `DFLT` value in FontCraft's; `latn` and `DFLT` agree. The kern measure lists this difference (`KNOWN` in `crates/oracle/tests/kern.rs`), not the allow-list |
| Yes-or-no custom parameters of a `.glyphs` source (`Use Typo Metrics`, `Has WWS Names`, `isFixedPitch`, `postscriptForceBold`) | on only when the value is exactly 1 | on for any number, or text that reads as a number, other than 0 (`fontcraft_model::param_flag`) | glyphsLib applies `bool(value)`, so fontmake builds a hand-written 2 as on; the reference app writes only 0 and 1. No corpus source has another value, so this row has no allow-list entry |
| The *Decompose Components* and *Decompose Glyphs* parameters | not read (nor by fontmake) | every composite, or the listed composites, of a TrueType build become contours | the handbook describes them (`docs/plans/M6.md` › M6.16). No corpus source has them, so this row has no allow-list entry |
| A `decomposeComponents` entry in the ufo2ft filter list | not read: only `--decompose-components` decomposes every composite | the composites the entry names become contours, as ufo2ft (fontmake) reads it | the entry is ufo2ft's filter, and fontmake applies it. No corpus source has one |
| The `include` and `exclude` lists of a `flattenComponents` or `decomposeTransformedComponents` entry | ignored: the filter reaches every glyph | the filter reaches only the glyphs the list allows, as in ufo2ft | the lists are part of ufo2ft's filter definition. No corpus source has one |
| Where the ufo2ft filter list is read for a `.glyphs` master that is not the default one | the default master's user data | the built master's user data, else the font's | FontCraft builds one master statically until M9, and fontmake builds each master from its own lib, where glyphsLib puts that master's user data |
| A designspace axis whose map has points outside the axis range (range 300–700, points at 100 and 900 as well) | the points outside are written to `avar` with from-coordinates beyond −1 and 1 (−2 and nearly 2), which the format does not allow, and the masters' normalized locations come from the map's ends | the points outside are left out; the masters' locations come from the axis range. A map that is linear inside the range writes no `avar` | fontmake 3.12.1 writes the same font. Pinned by `map_points_outside_the_axis_range_are_left_out` in `crates/compile/src/var/avar.rs`. No corpus source has such a map |
| A designspace axis whose range is wider than its map (range 100–900, map 300–700) | refused: the map must reach the axis minimum, default and maximum (fontmake 3.12.1 refuses it too) | compiled: the map is read past its ends as fontTools' `piecewiseLinearMap` reads it (the same offset as the nearest point), and the `avar` points are those fontTools' normalization gives | refusing would leave such a source with no variable font; the result is a valid table and matches what fontTools computes before its check. Pinned by `an_axis_range_wider_than_the_map_extends_the_map` |
| Rounding of `gvar` deltas | each delta rounded on its own, so a source's coordinates can come back one unit off | region by region, each later region taken from the rounded earlier ones, as fontmake's varLib rounds them | every master and intermediate layer comes back exactly as `glyf` would store it; fontmake gives the same font (`compile.gvarIupOptimisation`) |
| Overlap flags in a variable font | never set | `OVERLAP_SIMPLE` on every simple glyph | a variable font keeps its overlaps, and some rasterizers need the flag to draw them (`compile.overlapFlags`) |
| Kerning of alternate (bracket) glyphs in a variable font, for a pair between two glyphs that both have alternate layers | the pair is copied to every combination of the two glyphs and their alternate glyphs (`x x.BRACKET.varAlt01` −50 in `BracketTestFontKerning`) | copied as glyphsLib copies it, one alternate glyph at a time: (parent, x) gives (alternate, x), (x, parent) gives (x, alternate), (parent, parent) gives only (alternate, alternate) | fontmake builds the same pairs (`compile.bracketLayerGlyphs`, a guess until the reference app is checked). The variable measure counts the source as different in `GPOS` kerning against fontc |
| Feature variations of a decreasing axis (`decreasing_axis/axis.glyphs`, whose `opsz` masters fall from 144 to 9) | `fvar` gives the axis as 144 to 9 and `avar` from-coordinates of −2, so no alternate glyph applies at any location | the axis ascends from 9 to 144, and `bar.BRACKET.varAlt01` applies inside its range | fontc's tables are outside what the format allows; fontmake does not build the source. The variable measure counts it as different against fontc |
| `CPAL` of a source with `Color Palettes` and no colour layers | written on its own | not written, as fontmake writes it | no table refers to the palettes, so they change nothing in the font (`CPAL`) |
| Palette layers of a `.glyphs` version 2 file, named `Color 0`, `Color 1`… instead of a `colorPalette` attribute | not read | read as palette layers, as fontmake reads them | they are how the reference app's version 2 format stores palette layers (`color.export.colrFromPaletteLayers`) |
| Anchors of colour layer glyphs (`COLRv0-2layers`, `COLRv0-component-anchors`) | copied from the colour layer, so marks attach to `A.color0` and `GDEF` classes it as a base | none, as fontmake writes them | renderers draw the layers of the base glyph where the base glyph is placed, so marks attach to the base glyph |
| Full-colour paints in a separate Color layer, the reference app's usual form | ignored; only colour attributes on the master layer itself are read | read, as fontmake reads them; a master layer's own colour attributes are read too | the handbook makes full-colour glyphs with Color layers (`color.export.fullColorToColrV1`) |
| A full-colour glyph whose shapes form one run of equal paints (`COLRv0-reordered`, `COLRv1-gradient`) | the base glyph itself is the glyph its paint draws | a layer glyph `<base>.color0`, as for every colour glyph | the base glyph stays the master layer's outline, the fallback for software without COLR (`color.fallback.masterLayerColr`) |
| Stroked paths on full-colour layers | refused | the stroke expanded with the path's width, position, caps and joins into its own layer glyph above the fill | COLR has no stroke paint (`color.export.strokeInColr`); fontmake outlines centred strokes only |
| Paths on full-colour layers without a fill colour or gradient | refused | painted 50 % gray, the fill a new shape gets | the canvas draws them that way (`color.export.fullColorToColrV1`) |
| Gradient bounds | the layer glyph's `glyf` box (rounded control points) | the curve bounds of the layer glyph's outline | the bounds the editor shows, and the ones a compiled font is read back against; equal for outlines without curves (`compile.colrGradientGeometry`) |
| A variable font from full-colour layers whose gradients differ between the masters (`COLRv1-var`) | the default master's paints, static, with a clip list | `PaintVarLinearGradient` with a `DeltaSetIndexMap` and an `ItemVariationStore`, no clip list, as fontmake writes it | the gradient follows the outline at every location; a static clip box would cut the larger masters (`color.export.variableColr`) |
| Clip boxes that reach below 0 | the minimum rounded toward zero, which can cut the glyph | rounded down, as fontmake (fontTools) rounds it | a clip box must contain its glyph (`compile.colrClipBoxes`) |
| Gradient kinds and options of `.glyphs` version 4: radii, conic gradients, extend modes, a palette reference's alpha | linear and `circle` gradients only, others refused; radii, extend and alpha not read, every paint opaque | PaintRadialGradient with both radii, PaintSweepGradient, the stored extend, the alpha in the paint | the version 4 file format stores them |

**Where the handbook decides against an oracle.** The M7, M8 and M9 plans follow the handbook on
these points (§ 7 of each plan, decided by the orchestrator on the owner's behalf). The oracle
comparison allow-lists each difference for the corpus sources it shows in.

| What | fontc | fontmake | FontCraft | Plan |
|---|---|---|---|---|
| `postscriptFullName` or `postscriptFullNames` on an instance | not read | the singular form only, written to the CFF FullName | either form sets name ID 4 and the CFF FullName (4 corpus sources differ) | M7, question 2 |
| `styleNames`, `styleMapFamilyNames`, `styleMapStyleNames` and `compatibleFullNames` properties | not read | not read | written to the name IDs the handbook gives them (14 corpus sources hold `styleNames`) | M7, question 2 |
| An instance's Width Class without a width axis | no static instances | OS/2 `usWidthClass` stays 5 | the instance's value | M7, question 2 |
| Automatic feature code whose stored text is older than the glyph set | compiled as stored | compiled as stored | regenerated for each export target unless Update Features is off; a source last updated in the reference app gives the same code | M8, question 4 |
| Kerning of a master with Link Metrics With Master in a variable font (`KerningLinkedMaster`) | its own reading of the links: `A B` −3.333 at the linked Bold, 0 at Heavy | the kerning of the master the link names (−40); a master whose link leads to no kerning takes the others' interpolated value, as fontmake does | the handbook's link rule; fontmake agrees |
| Mark attachment of colour layer glyphs in a variable font (`COLRv0-component-anchors`) | marks attach to `A.color0` | no attachment, as fontmake | the layer glyphs carry no anchors of their own |
| The origin of a variable font without Variable Font Origin | the master named by the words all master names share (Regular when none), else the first | the same | the first master; designs at every location agree, `fvar` defaults and `glyf` differ (22 corpus sources name their origin and agree; the three Oswald sources, whose first master is ExtraLight, are allow-listed for `fvar`, `avar` and the values at instances, which move by a unit where the quantized coordinates differ) | M9, question 3 |
| A layer whose axis rules have every bound empty (`BlankAlternateLayers`, `ReverseBracketLayers`, `ReverseBracketLayersMirror`) | an alternate set covering the whole designspace: its glyph is substituted everywhere | the same | the `[]` partner of a master layer with a range, which stands in as that master's main source; no layer with a range, no source at all (`alternateLayer.onMasterLayer`), so `GSUB` variations and the marks of those alternate glyphs differ in the variable measure | M9.3 |
| A static instance outside the masters | no static instances | clamped to the axis range | extrapolated, named in the export report, left out of variable fonts (25 instances in 11 corpus sources); compared with fontTools' model with extrapolation on | M9, question 6 |

Macintosh name records stay off unless Export Mac Name Table Entries is on, as both oracles
write them and against the handbook's default (the M2 decision, kept by M7 question 2).

**Static instances against fontmake.** fontc builds no static instance, so fontmake is the oracle
for their names. Over the corpus, the 135 instances that sit at a master and both compilers build (64 sources) give 1,678
equal values and 120 different ones (name IDs 0–25 without FontCraft's version stamp, `usWeightClass`,
`usWidthClass`, `fsSelection`, `head.macStyle`; M7 plan § 5). Each difference has one of these
causes; none comes from the handbook's keys, which no corpus instance at a master uses:

| What | fontmake | FontCraft | Values | Sources |
|---|---|---|---|---|
| A source without a family name | `Unnamed font`, glyphsLib's default | no family name, as fontc (and as FontCraft's masters) | 108 (IDs 1, 3, 4, 6, 16) | 11 |
| A source without a version | 1.000 | 0.000, as fontc | 8 (IDs 3, 5) | 2 |
| A weight axis whose masters state no user location (no Axis Location, no weight class) | every location at 400, so `usWeightClass` 400 | the location through the axis mapping FontCraft derives for the font | 4 (`usWeightClass`) | 3 |


**Static instances of designspace sources.** These follow fontmake's instancer (ufo2ft's
`Instantiator`, then glyphsLib's `apply_instance_data_to_ufo`):

| What | Where it comes from |
|---|---|
| Family, style, PostScript and style-map names | the `<instance>` element; style-map names it leaves out are ufo2ft's fallbacks from the style name |
| Names of one style in the masters' UFOs (PostScript, full, typographic, WWS, compatible, unique id, style-map) | left out, unless the source has one master and the instance sits at it |
| Other font information | the default source's; numbers interpolated over the masters |
| `usWeightClass`, `usWidthClass` | the user location on a `wght` or `wdth` axis (weight truncated, width the nearest class), over the masters' classes; the instance's own parameters win |
| `<rules>` the reader does not make alternate layers | glyphs swapped where a rule holds at the instance's design location; in a variable font, feature variations under the lib key's first tag, `rclt` for `processing="last"`, else `rvrn` |

One case is known to differ: a single-source designspace whose instance states no style-map
names takes ufo2ft's fallbacks, where fontmake keeps the UFO's own style-map names. A
`featureVarsFeatureTag` lib key that lists several tags gives the first only; fontTools registers
the rules under each.

**Against the handbook.** TrueType composites with overlapping or transformed components stay
composites by default, as fontc and fontmake keep them. The texts of the Keep Overlapping
Components and Keep Transformed Components parameters imply that the reference app decomposes
such composites unless the parameter is set; FontCraft does not, so the two parameters change
nothing (the owner's answer to `docs/plans/M6.md` § 7, question 5). No allow-list entry is
needed: both oracles agree with FontCraft. A compile test checks that a build with both
parameters set is byte for byte the build without them.

In variable fonts, composites whose base glyph has intermediate layers stay composites, as both
oracles keep them; the handbook's Decompose Brace Layers default decomposes them. The parameter
set to on decomposes them (`docs/plans/M9.md` § 7, question 4 (a)). Alternate layers are switched
in `rvrn`, as the appendix and both oracles say, against the Switching Shapes page's `rlig`
(question 2 (a)).

The readers follow fontc where a source leaves a value out (`docs/spec/30-source-formats.md` ›
Values a source leaves out): a `.glyphs` layer without a `width` is 600 wide; a designspace
master without a trailing metric gets ufo2ft's default; a UFO glyph without a height in a
vertical UFO has height 0; version 2's top-level names in a version 3 file are names. One case
stays different: a designspace master that lacks a metric but has a later one reads the missing
one as 0, where fontc uses ufo2ft's default (no corpus source has it).

Against fontmake (and so for CFF fonts, which fontc does not write), every field the two
references derive differently keeps fontc's value; `crates/oracle/allow.toml` lists those
differences for fontmake with their reasons (`docs/plans/M2.md` › M2.3b).

**Private DICT defaults, against fontmake.** When a master has alignment zones and the source
sets no blueScale or blueFuzz parameter, FontCraft writes BlueScale 0.037 and BlueFuzz 1, the
defaults the handbook's parameter descriptions give (the answer to `docs/plans/M10.md` § 7,
question 6). fontmake writes BlueScale as 3 / (4 × the tallest zone) and BlueFuzz 0, so every
CFF build from such a source differs in these two Private DICT entries. Where a source sets
blueFuzz or Family Alignment Zones, FontCraft writes them (Family Alignment Zones as FamilyBlues
and FamilyOtherBlues), and fontmake ignores both. The oracle compares the decoded outlines of a
`CFF ` table, not its Private DICT, so these differences need no allow-list entry. The
reference app's own export is not yet observed (`hinting.ps.blueScale`, `hinting.ps.blueFuzz`).

UFO sources get the same defaults: a UFO without `postscriptBlueScale` or `postscriptBlueFuzz`
is written with BlueScale 0.037 and BlueFuzz 1, where ufo2ft computes BlueScale from the zones
and writes BlueFuzz 0.

The handbook limits BlueScale by the tallest zone: (largest point size at 300 dpi − 0.49) ×
tallest zone < 240, that is BlueScale × tallest zone < 1. The default 0.037 breaks this for a
zone of 28 units or more. FontCraft keeps the BlueScale it writes (the default or the source's
value) and adds an export warning naming the zone size and the largest BlueScale it allows.

**Manual PostScript hints.** No reference compiler writes a `.glyphs` source's manual hints:
glyphsLib passes them through as lib keys and ufo2ft drops them. FontCraft writes them into the
charstrings (`crates/compile/src/cff_hints.rs`), so a hinted glyph's charstring differs from
fontmake's in its stem operators and masks while its outline stays the same; the oracle's
outline comparison does not see the difference.

**CID-keyed CFF against makeotf.** A CID-keyed source builds a CID-keyed table in any CFF
export, not only in a project compile (`compile.cidKeyedFonts`). Against makeotf 5.0.1 on the
same synthetic CID-keyed UFO and CMaps the CID data, sub-fonts, Private values, glyph order and
`cmap` subtables agree. The differences:

| What | makeotf | FontCraft |
|---|---|---|
| charstring coordinates | fractions kept | rounded, as every FontCraft CFF (`compile.cffRoundTolerance`) |
| `defaultWidthX`, `nominalWidthX` | its own choice per sub-font | fontTools' method per sub-font |
| no Unicode CMap given | stops | encodes the source's own code points |
| two-byte Macintosh CMap | a Macintosh subtable (not yet observed) | no Macintosh subtable, with a warning |

## 4b. FontDame conversion against mtiLib

`features convert` writes FontDame layout text as feature code (`crates/tables/src/fontdame.rs`).
fontTools' mtiLib is the oracle. The measure takes the 28 table tests of mtiLib's test data
(14 GSUB, 9 GPOS, 5 GDEF), compiles our feature code with feaLib into a font with the tests'
glyph order, and builds the same table from the file with mtiLib. It compares the two tables as
rules: lookup types folded, extension lookups unwrapped, class rules expanded over the glyph
order, called lookups replaced by their own rules, and features as script, language, tag and
lookups. 24 of the 28 tables are equal. The other 4 hold what feature code cannot say:

| Test file | mtiLib | FontCraft | Why |
|---|---|---|---|
| `chained-glyph` (read as GSUB and as GPOS) | the lookup flag holds mark attachment class 2; there is no GDEF table | the flag without the class, a warning, and a comment in the lookup naming the class | feature code names a mark attachment class by its glyphs, and the compiler numbers the classes itself, so a class number alone has no form. Converting the GDEF file in the same run keeps the class |
| `gdefmarkattach` | a GDEF table with a mark attachment class definition | the classes as named glyph classes (`@GDEF_MarkAttachClass1`, …); no GDEF table | the `GDEF` table block of feature code has no mark attachment classes: a compiler builds them only from the lookup flags that use them |
| `gdefmarkfilter` | a GDEF table with mark glyph sets | the sets as named glyph classes (`@GDEF_MarkFilterSet1`, …); no GDEF table | the same holds for mark filtering sets |

A lookup flag's mark filtering set without its GDEF file is left out in the same way as a mark
attachment class (no test file has one).

Two choices differ from mtiLib on input the tests do not hold:

- `# N` names glyph N of the glyph order when a glyph order is given (`--font`); mtiLib always
  writes `glyphNNNNN`.
- A reverse chaining subtable is written as one rule whose classes are sorted by name, or as one
  rule per glyph when the replacements do not sort in the same order. feaLib pairs the two
  classes after sorting each by name, and fea-rs pairs them in written order; both forms give
  the file's pairs in both compilers.

## 4c. Format conversion against the reference converters

`font.convert` (`crates/tables/src/convert.rs`) is measured over the corpus, glyph by glyph in
glyph order, with the outlines as fontTools draws them:

| Direction | Oracle | Compared |
|---|---|---|
| CFF → TrueType | AFDKO otf2ttf | `glyf` points, on-curve flags and contour ends |
| TrueType → CFF | `tx -cff` | drawn area within 0.5 % and drawn bounds within 1 unit |
| CFF → CFF2 | fontTools `CFFToCFF2` | the drawn path |
| CFF2 → CFF | fontTools `CFF2ToCFF` | the drawn path |

The rules FontCraft takes from the oracles:

- **Drawing.** Charstrings are drawn as fontTools' `T2OutlineExtractor` draws them: operands
  added up in 16.16 units, so fractions stay; the FontMatrix is not applied; a path without a
  `moveto` starts at the current point.
- **Error bound.** Without `maxError` the curve conversion stays within one font unit, the
  default of otf2ttf (`-e`, in font units) and of fontTools' qu2cu. `maxError` is a fraction of
  the em when given.
- **seac to TrueType.** A `seac` glyph becomes a composite: the base at the origin and the
  accent at `(adx, ady)` rounded, both with `ROUND_XY_TO_GRID`, as otf2ttf's `TTGlyphPen`
  writes the two components fontTools draws. `maxp` holds the composite fields.
- **Left side bearing.** TrueType to CFF moves each glyph so that the `xMin` of its `glyf`
  header sits at its `hmtx` left side bearing. tx and fontTools' glyph set both draw a
  TrueType glyph there; the two values differ only in fonts edited by hand.

Differences kept:

| What | Reference | FontCraft | Why |
|---|---|---|---|
| A `seac` glyph, CFF → CFF2 | `CFFToCFF2` leaves the four seac operands at the end of the charstring, so the glyph draws nothing | the base and the moved accent written into the charstring; the base keeps its hints when the glyph has none of its own, the accent's are dropped | CFF2 has no seac, and the reference's result loses the glyph |
| A `seac` glyph with contours of its own, or whose base or accent is accented, CFF → TrueType | otf2ttf's pen decomposes the components without converting them, so cubic points reach `glyf` | the parts drawn out and converted with the rest of the glyph | a `glyf` table of format 0 holds quadratic curves only |
| Merging quadratic runs, TrueType → CFF | tx merges by its own rule, which is not documented; on some glyphs of the old Oswald fonts its curves stray up to 3 % in area from the source | fontTools' qu2cu within the error bound, every curve cubic | both are valid cubic outlines; FontCraft's stay within the stated bound |
| Glyph names of a TrueType font without `post` names, TrueType → CFF | tx names glyphs from the cmap with Adobe Glyph List names (`Gcommaaccent`, `ff`) and `g<gid>` otherwise | `glyphNNNNN` | tx's name table is not documented, and fontTools names the same glyphs differently again (`uni0122`, `glyph00001`); the measure compares outlines by glyph index |
| A path operator with operands `T2OutlineExtractor` cannot draw (an `rrcurveto` with five), CFF ↔ CFF2 | `CFFToCFF2` and `CFF2ToCFF` do not check its operands | the conversion is refused with a message | every conversion reads each charstring once into operators under one memory limit, and an operator that cannot be drawn is refused as it is read |
| CFF2 → CFF oracle | fontTools 4.66 `CFF2ToCFF` fails on a table with local subroutines, and on recalculating `hhea` after it renames the glyphs | — | the measure expands the subroutines first and keeps the stated bounds |

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
   Answer (M12.25): the subroutiniser (`crates/cff/src/subr`) follows compreffor's method; the
   measure compares outlines after expansion and the bytes of the CharStrings and subroutine
   INDEXes, with a budget of 2% above cffsubr (`crates/cff/crosscheck/subr_check.py`).
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
    **Checked in the M9 planning pass:** fontmake clamps rather than refuses: ufo2ft's
    instantiator normalises with fontTools' `normalizeLocation`, which clamps to the axis range.
    The handbook says the reference extrapolates static instances. 25 static instances in 11
    corpus sources lie outside their masters. **Decided** (`docs/plans/M9.md` § 7, question 6 (a),
    decided by the orchestrator on the owner's behalf): the instancer extrapolates, the export
    report names each extrapolated instance, and variable fonts leave such instances out (§ 4a).
16. **VORG, hdmx in write-fonts.** Both have generated code that is not compiled into 0.54.0.
    Check newer write-fonts releases before writing our own.
17. **STAT format 4.** write-fonts can write it and fontc does not emit it; decide whether our
    Font Info exposes multi-axis labels.
    **For M9:** not written, as fontc; STAT comes from the derivation fontc and glyphsLib share
    (`docs/plans/M9.md` § 3).
18. **GDEF class inference.** fontc infers base and ligature classes from anchors for Glyphs
    sources only. Our native sources carry Glyphs-style categories, so we need to state which rule
    applies to them and to UFO imports.
    **Checked in the M6 planning pass (oracle):** for Glyphs sources both oracles make a glyph a
    mark when its category is Mark with subcategory Nonspacing or Spacing Combining, a ligature
    when its subcategory is Ligature and it has an attaching anchor, and a base when it has any
    anchor without an underscore after propagation (fontc `recompute_gdef_categories`); UFO
    sources keep the categories they state. The mark writer that uses them is M6.17 (the owner's
    answer to `docs/plans/M6.md` § 7, question 2).
