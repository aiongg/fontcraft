# The command-line tool suite: `fontcraft-cli`

Design of the subcommand set, its shared conventions, the mapping from every option of the reference
command-line tools (AFDKO, fontTools, fontmake, fontc) to ours, what is left out, and how each subcommand
is tested against a reference tool. Rows: `spec/inventory/20-cli.toml` (`[[cli]]`, `[[command]]`,
`[[behavior]] cli.global.*`).

Shape of the command line:

```
fontcraft-cli <subcommand> [<action>] <inputs>... [options]
```

Thirty top-level subcommands replace roughly seventy reference programs and scripts. Subcommands that bundle
several operations on one kind of object (`tables`, `glyphs`, `outlines`, `check`, `features`,
`collection`, `woff`, `variations`) take an action word; the others do one thing.

## Subcommands at a glance

| Subcommand | Engine command | Area | Milestone | Replaces |
|---|---|---|---|---|
| `compile` | `font.compile` | compile | M2 | makeotf, addfeatures, buildmasterotfs, buildcff2vf, varLib, fontmake, fontc, ttx |
| `oracle` | (shell) | compile | M2 | ttx_diff, fontc_crater |
| `dump` | `font.dumpTables` | cli-tools | M12 | ttx, ttxn, spot, tx, detype1, otl-normalizer, varLib.avar.unbuild, ttLib, pyftsubset |
| `info` | `font.inspect` | cli-tools | M1 | spot, ttx, tx, otc2otf, makeinstancesufo |
| `tables` | (shell) | cli-tools | M12 | sfntedit |
| `tables list` | `tables.list` | cli-tools | M1 | sfntedit, spot, ttx |
| `tables extract` | `tables.extract` | cli-tools | M12 | sfntedit |
| `tables add` | `tables.add` | cli-tools | M12 | sfntedit |
| `tables delete` | `tables.delete` | cli-tools | M12 | sfntedit |
| `tables edit` | `tables.edit` | cli-tools | M12 | sfntedit |
| `tables copy` | `tables.add` | cli-tools | M12 | sfntedit |
| `tables checksum` | `tables.checksum` | cli-tools | M12 | sfntedit |
| `tables import` | `tables.importTtx` | cli-tools | M12 | ttx, pyftmerge |
| `tables reorder` | `tables.reorder` | cli-tools | M12 | pyftsubset, sfntedit |
| `tables show` | `tables.show` | cli-tools | M12 | — (our own; the reference vendor's table-viewer tool) |
| `glyphs` | (shell) | cli-tools | M12 | spot |
| `glyphs list` | `font.listGlyphs` | cli-tools | M1 | spot, tx |
| `glyphs rename` | `font.renameGlyphs` | cli-tools | M12 | tx, makeotf |
| `glyphs reorder` | `font.reorderGlyphs` | cli-tools | M12 | ttLib.reorderGlyphs, tx |
| `metrics` | `font.reportMetrics` | cli-tools | M12 | tx, spot, pens.statisticsPen |
| `convert` | `font.convert` | cli-tools | M12 | tx, type1, detype1, otf2ttf, cffLib.CFFToCFF2, cffLib.CFF2ToCFF, ttx, ttLib, designspaceLib, fontmake, makeotf, mergefonts |
| `subset` | `font.subset` | cli-tools | M12 | pyftsubset, tx, mergefonts, makeotf, fontmake, fontc |
| `instance` | `font.instantiate` | cli-tools | M12 | varLib.instancer, varLib.mutator, varLib.interpolate_layout, makeinstancesufo, fontmake, tx |
| `merge` | `font.merge` | cli-tools | M12 | mergefonts, pyftmerge |
| `transform` | `font.transform` | cli-tools | M12 | rotatefont, ttLib.scaleUpem |
| `optimize` | `font.optimize` | cli-tools | M12 | otlLib.optimize, cffLib.width, cffLib.specializer, varLib.varStore, tx, fontmake, varLib, pyftsubset, ttx, ttLib |
| `collection` | (shell) | cli-tools | M12 | otc2otf |
| `collection list` | `collection.list` | cli-tools | M12 | otc2otf, tx |
| `collection pack` | `collection.pack` | cli-tools | M12 | otf2otc, ttLib |
| `collection unpack` | `collection.unpack` | cli-tools | M12 | otc2otf, ttLib, tx |
| `woff` | (shell) | cli-tools | M12 | ttLib.woff2 |
| `woff compress` | `woff.compress` | cli-tools | M12 | ttLib.woff2, ttx, pyftsubset |
| `woff decompress` | `woff.decompress` | cli-tools | M12 | ttLib.woff2, pyftsubset |
| `diff` | `font.diff` | cli-tools | M12 | sfntdiff, fonttools diff, ttxn, ttx_diff |
| `sources` | (shell) | cli-tools | M12 | — |
| `sources diff` | `source.diff` | cli-tools | M12 | — (our own; the reference vendor's source-comparison tool) |
| `sources merge` | `source.merge` | cli-tools | M12 | — (our own) |
| `outlines` | (shell) | cli-tools | M12 | checkoutlinesufo |
| `outlines remove-overlaps` | `outlines.removeOverlaps` | cli-tools | M12 | ttLib.removeOverlaps, tx, checkoutlinesufo, fontmake |
| `outlines direction` | `outlines.correctDirection` | cli-tools | M12 | fontmake, cu2qu |
| `outlines extremes` | `outlines.addExtremes` | cli-tools | M12 | — |
| `outlines round` | `outlines.roundCoordinates` | cli-tools | M12 | fontmake |
| `outlines to-quadratic` | `outlines.convertCurves` | cli-tools | M12 | cu2qu, fontmake |
| `outlines to-cubic` | `outlines.convertCurves` | cli-tools | M12 | qu2cu, tx |
| `outlines componentize` | `outlines.componentize` | cli-tools | M12 | ttfcomponentizer |
| `outlines decompose` | `outlines.decompose` | cli-tools | M12 | ttfdecomponentizer, fontmake, fontc |
| `autohint` | `autohint.ps.run` | hinting-ps | M12 | otfautohint, fontmake |
| `stems` | `autohint.ps.reportStems` | hinting-ps | M12 | otfstemhist |
| `dehint` | `hints.remove` | hinting-ps | M12 | tx, ttxn, pyftsubset |
| `check` | `audit.run` | qa | M12 | comparefamily, tx, makeotf |
| `check outlines` | `audit.checkOutlines` | qa | M12 | checkoutlinesufo |
| `check family` | `audit.compareFamily` | qa | M12 | comparefamily |
| `check interpolation` | `audit.checkInterpolation` | qa | M12 | varLib.interpolatable, buildcff2vf, fontmake |
| `proof` | `font.proof` | cli-tools | M12 | proofpdf, tx, spot, pens.svgPathPen |
| `features` | (shell) | features | M12 | spot |
| `features list` | `layout.listFeatures` | features | M12 | spot, comparefamily |
| `features compile` | `layout.compileFeatures` | features | M12 | feaLib, mtiLib, voltLib, addfeatures, fontmake, fontc |
| `features decompile` | `layout.decompileFeatures` | features | M12 | spot, ttxn |
| `features lint` | `layout.lintFeatures` | features | M12 | addfeatures, makeotf |
| `features convert` | `layout.convertSyntax` | features | M12 | voltLib.voltToFea, mtiLib |
| `features normalize` | `layout.normalize` | features | M12 | otl-normalizer |
| `variations` | (shell) | cli-tools | M12 | varLib.avar.unbuild |
| `variations build-avar` | `variations.buildAvar` | cli-tools | M12 | varLib.avar |
| `variations plan-avar` | `variations.planAvar` | cli-tools | M12 | varLib.avar.plan |
| `variations map` | `variations.mapCoordinates` | cli-tools | M12 | varLib.avar.map |
| `variations build-hvar` | `variations.buildHvar` | cli-tools | M12 | varLib.hvar |
| `variations model` | `variations.describeModel` | cli-tools | M12 | varLib.models |
| `variations plot` | `variations.plot` | cli-tools | M12 | varLib.plot |
| `run` | (shell) | cli-tools | M0 | tx, sfntedit, spot, sfntdiff |
| `commands` | `commands.list` | cli-tools | M0 | fonttools help, afdko |
| `mcp` | (shell) | cli-tools | M13 | — |
| `bench` | (shell) | cli-tools | M12 | cu2qu, qu2cu |
| `completion` | (shell) | cli-tools | M12 | afdko |
| `help` | (shell) | cli-tools | M0 | afdko, fonttools help |

## Conventions

These hold for every subcommand. Each paragraph names the `[[behavior]]` row that records it.

### Inputs are recognised by content (`cli.global.inputDetection`)

The first bytes decide what a file is: sfnt version (`0x00010000`, `true`, `OTTO`), `ttcf`, `wOFF`,
`wOF2`, a bare CFF or CFF2 header, `%!` (PFA or decrypted Type 1 text), the PFB segment marker, an XML
root of `ttFont` or `designspace`, a property list that is a `.glyphs` file, a directory or zip with
`metainfo.plist` (UFO), or the native project signature. Extensions are only used to choose default
*output* names. A wrong extension produces a warning and processing continues. fontmake's `-g`, `-u`,
`-m` switches and its guessing from the extension are therefore unnecessary.

### Outputs (`cli.global.output`, `.outputDir`, `.inPlace`, `.overwrite`)

- `-o PATH` names the output of a run that produces one file. `-o -` is standard output. If `PATH` is an
  existing directory the file is written into it under its default name.
- `--output-dir DIR` is for runs that produce several files; it is created if missing.
- Without either, text results go to standard output and file results go next to the input under a
  derived name: a new extension for a format change (`Font.otf` → `Font.ttf`), otherwise a
  dot-separated suffix naming the operation (`Font.subset.ttf`, `Font.instance.ttf`).
- The input is never overwritten unless `--in-place` is given. In-place writes are atomic: the result is
  written to a temporary file in the same directory and renamed.
- Existing outputs are replaced. `--no-clobber` makes that an error. No numbered alternative names.
- `--name-by postscript-name` names compiled or extracted fonts `<PostScript name>.<ext>` (the makeotf
  and `tx -A` convention) instead of after the input file.

### Standard input and output (`cli.global.stdinStdout`)

`-` as an input reads one font, XML dump or feature file from standard input. Directory formats cannot
be piped. A binary result is never written to a terminal: if standard output is a terminal and `-o -`
was asked for, the run stops with a usage error. This allows pipelines such as

```
fontcraft-cli subset Font.ttf --text-file page.txt -o - | fontcraft-cli woff compress - -o Font.woff2
```

### Several inputs (`cli.global.batch`, `.responseFiles`)

Subcommands that transform one font accept many and treat each independently, in parallel when
`--jobs` allows. Shell globs do the selection; on Windows `*` and `?` are expanded by the tool.
`@FILE` splices arguments from a file. A failure on one input is reported and the others continue; the
exit status is then 5.

### Machine-readable output (`cli.global.json`)

`--json` prints one JSON object per input, one per line:

```
{"input": "Font.ttf", "ok": true, "outputs": ["Font.subset.ttf"], "warnings": [], "errors": [], "result": { ... }}
```

`result` is the return value of the engine command and is documented with that command. Warnings and
errors are objects with `code`, `message` and, where known, `table`, `glyph`, `file`, `line`, `column`.
Nothing else is written to standard output in this mode.

### Exit status (`cli.global.exitCodes`)

| Status | Meaning |
|---|---|
| 0 | done, nothing to report |
| 1 | done, and the answer is "yes, there is something": differences (`diff`), failed checks (`check`), lint errors, a regression (`bench --baseline`) |
| 2 | the command line is wrong |
| 3 | an input is missing, unreadable or not a supported format |
| 4 | the operation itself failed |
| 5 | a batch finished with some inputs failed |
| 130 | interrupted |

### Diagnostics (`cli.global.logging`, `.color`, `.backtrace`, `.timing`)

Everything that is not the result goes to standard error: errors, warnings, progress, timing. Levels:
`-q` errors only; default warnings and notes (what was written or skipped); `-v` info; `-vv` debug.
`-q`, `-v` and `--log` act in the order given. `--log` sets a level or per-module levels
(`info,engine=debug`; modules filter the libraries' messages); `--log-file` duplicates the stream into
a file. Each line carries the level, the input it concerns and, for compiled fonts, the PostScript
name, so interleaved output from parallel jobs stays attributable:

```
fontcraft-cli: warning: Font.ttf: no 'GPOS' table
```

`--color auto|always|never` colours the level names. `--timing` prints per-stage times at the end;
`--trace FILE` writes them as trace events. These options, with `-j`/`--jobs`, `--backtrace` and
`-V`/`--version`, belong to every subcommand and may also come before its name; an option of the
subcommand's own with the same long name takes precedence (`dump --version`).

### Reproducible output (`cli.global.determinism`, `.timestamp`)

A given set of inputs and options yields the same bytes on every machine and for every `--jobs` value.
Consequences for the implementation: no iteration over unordered maps when writing; stable sorts with
total orderings; parallel stages collect results by index, not by completion order; subroutine numbering,
lookup order and name-record order are defined by rules, not by discovery order; no absolute paths, user
names, host names or tool build dates in any table.

Time stamps are the only clock-dependent data. `head.created` and `head.modified` come from the source
when compiling. When an existing font is changed, `created` is kept and `modified` is taken from
`SOURCE_DATE_EPOCH` if that environment variable is set, otherwise from the clock; `--timestamp keep`
leaves both alone and `--timestamp EPOCH` sets `modified` explicitly. Operations that only repackage
(`woff`, `collection`, `tables` actions) never touch time stamps.

### Shared option syntax

- **Collections** (`cli.global.faceSelection`): `--face N`, `--face PostScriptName`, `--face all`.
- **Glyph lists** (`cli.global.glyphSelector`): names, `gid` numbers, `/CID`, `U+XXXX`, ranges with
  `..` (or `-` between two numbers), `re:` patterns, `*`; file variants read the same items.
- **Locations** (`cli.global.locationSyntax`): `wght=600 wdth=85`; ranges `wght=400:700`; `opsz=drop`.
- **Table tags** (`cli.global.tableTags`): padded to four characters; file-name-safe form for extraction.
- **Lists** (`cli.global.listValues`): `--opt=a,b`; `--opt=+c,-a` edits the default; `*`; empty.
- **Switches** (`cli.global.booleanFlags`): `--x` and `--no-x` always both exist.
- **Precedence** (`cli.global.optionOrder`, `.optionsFile`): source settings < `--options` file <
  command line, left to right, last one wins.
- **Layers of a UFO** (`cli.global.afdkoProcessedLayer`): `--layer` to read, `--write-layer` to write.

### Sources and binaries (`cli.global.sourceOrBinary`)

Where an operation makes sense on both (outline repairs, hinting, subsetting, renaming, transforming), the
same subcommand takes either. On a binary font the tables not concerned are copied byte for byte, so a
font does not change in ways the user did not ask for. This is different from opening the font in the
editor and exporting it again, which rebuilds everything.

### Relation to the command registry (`cli.global.commandDispatch`)

A subcommand is a thin translation from options to the parameter object of one engine command. The table
at the top lists the command behind each subcommand. Rows marked "(shell)" either only dispatch to their
actions, or orchestrate other commands and external programs (`oracle`, `bench`, `run`, `mcp`).
Commands defined in other inventory files and reused here: `autohint.ps.run`, `autohint.ps.reportStems`,
`autohint.ps.reportZones`, `autohint.ps.listFdDicts`, `autohint.tt.run` (hinting inventories) and
`audit.run`, `audit.fix`, `audit.export`, `audit.list`, `audit.describe`, `audit.profile.list`,
`audit.checkOutlines`, `audit.checkInterpolation`, `audit.compareFamily`, `audit.report.*`,
`audit.clearCache` (QA inventory).

## Subcommand reference

Each section gives the usage line, behaviour the options alone do not convey, and the mapping from reference options to ours. The full option list with one-line meanings is the `options` field of the matching `[[cli]]` row in `spec/inventory/20-cli.toml`. Every subcommand also takes the global options described under *Conventions*.

### `compile`

Compiles sources into binary fonts: static TTF or OTF per instance or master, each plain or wrapped as WOFF or WOFF2; later, variable TTF (gvar) or CFF2 and interpolatable masters (M9). Sources are a native project, a .glyphs file or package, a designspace or a UFO; later also a bare outline font (CFF, CFF2, TrueType, Type 1) that receives the remaining OpenType tables from a feature file and name data. Without --format every source yields static TTFs.

```
fontcraft-cli compile SOURCE... [--format FMT]... [--container plain|woff|woff2]... [-o PATH | --output-dir DIR] [options]
```

**What it does.** `compile` is the one entry point for every route from source to binary. The route is
chosen by the kind of source and by `--format`:

| Source | Formats | Route |
|---|---|---|
| native project, `.glyphs`, designspace, UFO(s) | all | full compile: preprocess glyphs, build every table |
| designspace + `--master-finder` | `variable`, `variable-cff2` | merge already compiled master fonts into a variable font (fontTools `varLib` style) |
| bare CFF / OTF / Type 1 / TrueType | `otf`, `ttf` | keep the outlines, build cmap, name, OS/2, hhea, post, layout from feature code, menu-name file and GOADB (AFDKO `makeotf` / `addfeatures` style) |
| bare CFF2 variable font | `variable-cff2` | as above, with variable feature code; `fvar`, `avar`, `HVAR` are carried over, `MVAR` merged when `--keep-mvar` |

**Built (M2).** `--format ttf|otf`, `--container plain|woff|woff2`, `-o`/`--output-dir`,
`--instances[=NAME,...]`, `--masters-as-instances`, `--keep-overlaps`, `--keep-direction`,
`--conversion-error`, `--timestamp`, `--cbdt` and `--plan`. Each SOURCE is opened as a document and built by
`font.compile` (or described by `font.compilePlan`); the command prints one JSON line per SOURCE, and
with several sources one that fails does not stop the others. The other options in the table below
are later work.

**Defaults.** Without `--format`: `ttf`, whatever the number of masters (`variable` comes with M9).
`--afdko-project`: `otf`. Without `--container`: `plain`. Every exportable instance is built; a font
without instances builds its first master. `--instances=NAME,...` builds only the instances with those
exact names (a comma-separated list, not a pattern); `--masters-as-instances` adds one font per master.
Until interpolation (M9) an instance is built only where it sits at a master; the others are listed as
skipped, and a source that builds nothing fails. Overlapping contours are merged unless
`--keep-overlaps` is given; a glyph whose overlaps cannot be removed keeps them, with a warning
naming it. Contours are reversed for TrueType unless
`--keep-direction`; curves are converted to quadratics within `--conversion-error` em (default 0.001).
`--timestamp` sets `head.created` and `head.modified` (default `SOURCE_DATE_EPOCH`, else now).
`--cbdt` also writes CBDT and CBLC bitmap tables from the iColor layers; variable fonts never get
them. Static CFF
output is subroutinised unless `--no-subroutinize` is given or the source sets Disable Subroutines; a table
beyond the subroutiniser's limits is written without subroutines, with a warning. Variable CFF2 output
follows the same rule. Glyphs are written under their production names, made valid as fontc makes
them, with a warning listing the names changed. Later: `--no-production-names`; glyphs flagged as not
exported left out unless `--no-subset` (until then they are always left out); with `--output-layout
by-format`, outputs and intermediate sources in per-format folders as fontmake arranges them
(`master_ufo`, `instance_ufo`, `master_ttf`, `variable_ttf` and so on); CFF specialised and
subroutinised (`--optimize-cff 2`); variable `gvar` IUP-optimised.

**Option sources and precedence.** Lowest to highest: values stored in the source (custom parameters,
instance settings), `--afdko-fontinfo`, `--options` file, command line left to right. `--release` is a
bundle; a later `--no-subroutinize` undoes that part of it, an earlier one does not.

**AFDKO name data.** `--menu-names` reads both FontMenuNameDB syntaxes (the `f=`, `s=`, `l=`, `m=` form and
the older `c=` form) and builds name IDs 1, 2, 16, 17 and Mac 18 by the rules of the MakeOTF user guide.
`--goadb` alone only takes Unicode values from the third column; renaming and ordering need
`--apply-goadb` or `--release`. A TrueType source cannot be reordered: the GOADB order must match.
Unicode assignment for a non-CID font follows GOADB column 3, then AGLFN, then `uniXXXX` / `uXXXXX` names;
no Private Use code points are invented.

**CID-keyed sources.** The CMap resources default to the files matching the font's registry and ordering
(for example `UniJIS-UTF32-H`); only glyphs named by the CMaps are encoded. A UVS file has three fields
per line for CID fonts and two for name-keyed fonts.

**fsSelection.** Turning any of bits 7, 8, 9 on or off requires all three to be stated, because the table
version then becomes 4 where both states carry meaning.

**Output naming.** `-o` names one file and takes one SOURCE. The build must give exactly one file,
and the file's extension must match its format and container (`.woff` or `.woff2` chooses that
container when `--container` is not given). Otherwise each
file is named by the instance's `fileName` parameter, else by its PostScript name, inside
`--output-dir` (default: the current folder); with both formats, each goes into a `ttf` or `otf`
subfolder. A source without a family name: a `.glyphs` or `.glyphspackage` source takes
glyphsLib's default family, `Unnamed font`, as fontmake does (`Unnamedfont-Regular.ttf`); any
other source takes its file name, as fontmake names a master after its UFO's file name.

**Binary layout.** `--interpolate-binary-layout [DIR]` is fontmake's `-i
--interpolate-binary-layout`. Each instance is compiled as usual; its `GPOS` is then replaced by the
one interpolated from the compiled masters at the instance's location, and its `GDEF` and `GSUB` by
the default master's when that master has them (the same step as `instance
--interpolate-binary-layout`). The masters are `DIR/<master UFO stem>.ttf` (`.otf` for `--format
otf`); `DIR` defaults to `master_ttf` or `master_otf` beside the source. Fonts built from masters
(`--masters-as-instances`, or a font without instances) keep their own layout. The source is a
designspace or a `.glyphs` file or package. The option is refused with `--variable` (as fontmake
refuses it with variable output), `--collection` and a project compile. Every layout is
interpolated before the first font is compiled, so a missing master stops the build and nothing
is written. The argument after the option is its folder only when it is not an option and does
not end in a source extension.

**Looking inside a build.** `--plan` answers questions such as "what will the glyph order be" or "what
feature code is actually compiled" without building (`font.compilePlan`). `--tables GSUB,GPOS` builds
just those tables for inspection. `--emit-debug` keeps every intermediate file.

Engine commands: `font.compile`, `font.compilePlan`. Autohinting delegates to `autohint.ps.run` or
`autohint.tt.run`. The pipeline stages are specified in `docs/spec/40-compile.md`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| makeotf | `-f` | SOURCE (or --afdko-project) |  |
| makeotf | `-o` | -o (a directory keeps the default <PostScript name>.otf) |  |
| makeotf | `-b / -nb` | --bold / --no-bold |  |
| makeotf | `-i / -ni` | --italic / --no-italic |  |
| makeotf | `-ff` | --features |  |
| makeotf | `-fs / -nfs` | --stub-gsub / --no-stub-gsub |  |
| makeotf | `-mf` | --menu-names |  |
| makeotf | `-gf` | --goadb |  |
| makeotf | `-ga / -nga` | --apply-goadb / --no-apply-goadb |  |
| makeotf | `-gs / -ngs` | --subset-to-goadb |  |
| makeotf | `-ds` | --designspace |  |
| makeotf | `-r / -nr` | --release / --no-release |  |
| makeotf | `-S / -nS` | --subroutinize / --no-subroutinize |  |
| makeotf | `-maxs` | --max-subrs |  |
| makeotf | `-dr` | --keep-outline-table |  |
| makeotf | `-amnd` | --notdef allow-missing |  |
| makeotf | `-addn / -naddn` | --notdef replace / --notdef auto | we draw a plain box; no weight-matched glyph |
| makeotf | `-rev` | --bump-version |  |
| makeotf | `-osbOn / -osbOff` | --fs-selection-on / --fs-selection-off |  |
| makeotf | `-osv` | --os2-version |  |
| makeotf | `-cs / -cl` | --mac-cmap-script / --mac-cmap-language |  |
| makeotf | `-cm / -ch / -cv` | --cmap-mac / --cmap-h / --cmap-v |  |
| makeotf | `-ci` | --uvs |  |
| makeotf | `-dbl` | --legacy-double-mapping |  |
| makeotf | `-fi` | --afdko-fontinfo |  |
| makeotf | `-fp` | --options (reads .fpr too) |  |
| makeotf | `-sp` | --save-options |  |
| makeotf | `-cn / -ncn` | --cid-keyed / (default) |  |
| makeotf | `-shw / -nshw` | --no-warn-unhinted / --warn-unhinted |  |
| makeotf | `-swo / -nswo` | --no-optimize-widths / --optimize-widths |  |
| makeotf | `-stubCmap4` | --stub-cmap4 |  |
| makeotf | `-omitMacNames / -useMacNames` | --no-mac-names / --mac-names |  |
| makeotf | `-overrideMenuNames / -donotOverrideMenuNames` | --feature-names-override / (default) |  |
| makeotf | `-skco / -nskco / -nsko` | --no-kern-class0-optimization / --kern-class0-optimization |  |
| makeotf | `-addDSIG / -omitDSIG` | --stub-dsig / --no-stub-dsig |  |
| makeotf | `-mvar` | --keep-mvar |  |
| makeotf | `-V / -nV` | --warnings all / --warnings default |  |
| makeotf | `-v` | --version |  |
| makeotf | `-showFinal` | --show-final-names |  |
| makeotf | `-lic` | --license-code |  |
| makeotf | default file names (font.pfa, features, FontMenuNameDB …) | --afdko-project |  |
| addfeatures | `-f -o -b -i -ff -fs -mf -gf -ds -osbOn -osbOff -osv -cs -cl -cm -ch -cv -ci -dbl -lic -stubCmap4 -omitMacNames -overrideMenuNames -skco -mvar -V -showFinal -addDSIG -omitDSIG` | same options as for makeotf | addfeatures is the back end makeotf calls; compile on a bare font does its job |
| addfeatures | `-lookupFinal` | --lookup-final-names |  |
| addfeatures | `-h / -u` | --help / -h |  |
| buildmasterotfs | `-d / --designspace` | SOURCE with --format otf-interpolatable |  |
| buildmasterotfs | `--mkot` | the compile options themselves |  |
| buildmasterotfs | `-v / --version` | -v / --version |  |
| buildcff2vf | `-d / --designspace` | SOURCE with --format variable-cff2 |  |
| buildcff2vf | `-o / --output` | -o |  |
| buildcff2vf | `-k / --keep-glyph-names` | --post-format 2 |  |
| buildcff2vf | `--omit-mac-names` | --no-mac-names |  |
| buildcff2vf | `-c / --check-compat` | --check-compatibility=fix |  |
| buildcff2vf | `-i / --include-glyphs` | --sparse-glyphs |  |
| buildcff2vf | `-v / --version` | -v / --version |  |
| varLib | designspace | SOURCE with --format variable and --master-finder |  |
| varLib | `-o / -d / --output-dir` | -o / --output-dir |  |
| varLib | `-x` | --exclude-table |  |
| varLib | `--disable-iup` | --no-optimize-gvar |  |
| varLib | `--no-colr-layer-reuse` | --no-colr-layer-reuse |  |
| varLib | `--drop-implied-oncurves` | --drop-implied-oncurves |  |
| varLib | `--master-finder` | --master-finder |  |
| varLib | `--variable-fonts` | --variable-fonts |  |
| varLib | `-v / --verbose / -q / --quiet` | -v / -q |  |
| fontmake | `-g / --glyphs-path, -u / --ufo-paths, -m / --mm-designspace, positional` | SOURCE (kind detected from content) |  |
| fontmake | `--glyph-data` | --glyph-data |  |
| fontmake | `-o / --output otf ttf otf-cff2 variable variable-cff2 ttf-interpolatable otf-interpolatable` | --format |  |
| fontmake | `-o ufo` | convert --to ufo, or instance --to ufo for interpolated instances |  |
| fontmake | `--output-path` | -o |  |
| fontmake | `--output-dir` | --output-dir (--output-layout by-format for fontmake's folders) |  |
| fontmake | `-i / --interpolate` | --instances[=NAME,...] | exact instance names, not a regular expression |
| fontmake | `--variable-fonts` | --variable-fonts |  |
| fontmake | `-M / --masters-as-instances` | --masters-as-instances |  |
| fontmake | `--family-name` | --family-name |  |
| fontmake | `--round-instances` | --round-instances |  |
| fontmake | `--designspace-path` | --emit-designspace |  |
| fontmake | `--master-dir / --instance-dir` | --master-dir / --instance-dir |  |
| fontmake | `--validate-ufo` | --validate-sources |  |
| fontmake | `--check-compatibility / --no-check-compatibility` | --check-compatibility / --no-check-compatibility |  |
| fontmake | `--expand-features-to-instances` | --expand-features-to-instances |  |
| fontmake | `--fea-include-dir` | --include-dir |  |
| fontmake | `--no-generate-GDEF` | --no-generate-gdef |  |
| fontmake | `--save-ufo-as-zip` | --ufo-structure zip |  |
| fontmake | `--ufo-structure / --indent-json` | --ufo-structure / --indent-json |  |
| fontmake | `--keep-overlaps` | --keep-overlaps |  |
| fontmake | `--keep-direction` | --keep-direction |  |
| fontmake | `--ttf-curves` | --ttf-curves |  |
| fontmake | `-e / --conversion-error` | --conversion-error |  |
| fontmake | `-f / --flatten-components` | --flatten-components |  |
| fontmake | `-a / --autohint, -A / --no-autohint` | --autohint[=OPTIONS] / --no-autohint |  |
| fontmake | `--cff-round-tolerance` | --cff-round-tolerance |  |
| fontmake | `--optimize-cff` | --optimize-cff |  |
| fontmake | `-s / --subroutinize, -S / --no-subroutinize` | --subroutinize / --no-subroutinize |  |
| fontmake | `--no-optimize-gvar` | --no-optimize-gvar |  |
| fontmake | `--filter` | --filter (names a registered filter command, not a Python class) |  |
| fontmake | `--auto-use-my-metrics / --no-auto-use-my-metrics` | same |  |
| fontmake | `--drop-implied-oncurves` | --drop-implied-oncurves |  |
| fontmake | `--interpolate-binary-layout` | --interpolate-binary-layout [DIR] | instances only; DIR relative to the source |
| fontmake | `--feature-writer` | --feature-writer (names a built-in generator, not a Python class) |  |
| fontmake | `--debug-feature-file` | --debug-feature-file |  |
| fontmake | `--no-variable-features` | --no-variable-features |  |
| fontmake | `--mti-source` | --mti-source |  |
| fontmake | `--production-names / --no-production-names` | same |  |
| fontmake | `--subset / --no-subset` | same |  |
| fontmake | `--timing` | --timing |  |
| fontmake | `--verbose LEVEL` | --log LEVEL |  |
| fontmake | `--version` | --version |  |
| fontmake | `--no-write-skipexportglyphs` | convert --skip-export-key glyphs |  |
| fontc | input_source, -s / --source | SOURCE |  |
| fontc | `-o / --output-file` | -o |  |
| fontc | `--emit-debug` | --emit-debug |  |
| fontc | `--prefer-simple-glyphs` | --prefer-simple-glyphs[=BOOL] |  |
| fontc | `--flatten-components` | --flatten-components[=BOOL] |  |
| fontc | `--erase-open-corners` | --erase-open-corners[=BOOL] |  |
| fontc | `--propagate-anchors` | --propagate-anchors[=BOOL] |  |
| fontc | `--decompose-transformed-components` | same |  |
| fontc | `--decompose-components` | same |  |
| fontc | `--emit-timing` | --trace FILE (plus --timing for a summary) |  |
| fontc | `-b / --build-dir` | --build-dir |  |
| fontc | `-g / --glyph-name-filter` | --glyphs re:PATTERN |  |
| fontc | `--skip-features` | --skip-features |  |
| fontc | `--emit-lookup-debug-info` | --debug-lookups |  |
| fontc | `--keep-direction` | --keep-direction |  |
| fontc | `--no-production-names` | --no-production-names |  |
| fontc | `--vv` | --version -v |  |
| fontc | `--log` | --log |  |
| ttx | FILE.ttx (compile direction) | convert FILE.ttx --to ttf\|otf | an XML dump is a binary font in another notation, so it belongs to convert |

### `oracle`

Development aid for differential testing: compiles each source with FontCraft and with a reference compiler found on PATH (or takes an already built reference font), normalises both results and reports per table how similar they are. With --corpus it runs over a list of source repositories and writes a JSON and HTML summary. It is a shell around font.compile and font.diff and is not needed to produce fonts.

```
fontcraft-cli oracle SOURCE... --against fontmake|fontc|makeotf|FONT [--output-dir DIR] [options]
```

`oracle` exists so that the comparison method in the *Oracles* section can be run with one command by a
person, by CI and by an agent. For each source it:

1. compiles with `font.compile` and with the reference tool, giving both the same fixed
   `SOURCE_DATE_EPOCH` and equivalent options;
2. runs `font.diff` with `normalize: "compiler"`;
3. prints, per table, `identical`, a similarity ratio, or which side alone has the table; with `--json`
   the same as `{"success": {tag: ratio | side}}` or `{"error": {side: {command, stderr}}}`.

Exit status 0 when everything is identical after normalisation, 1 when something differs, 4 when a
compiler failed. A failed reference build is cached with its command line and error text so that a later
run with `--rebuild ours` reports it again instead of rebuilding.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| ttx_diff | source | SOURCE |  |
| ttx_diff | `--fontc_path` | --reference-bin |  |
| ttx_diff | `--compare default\|gftools` | --mode |  |
| ttx_diff | `--config` | --config |  |
| ttx_diff | `--rebuild both\|fontc\|fontmake\|none` | --rebuild |  |
| ttx_diff | `--off_by_one_budget` | --off-by-one-budget |  |
| ttx_diff | `--expected_fontc_ttf_hash` | --expect-hash |  |
| ttx_diff | `--json` | --json |  |
| ttx_diff | `--outdir` | --output-dir |  |
| ttx_diff | `--production_names` | --production-names / --no-production-names |  |
| ttx_diff | `--keep_overlaps` | --keep-overlaps / --remove-overlaps |  |
| ttx_diff | `--keep_direction` | --keep-direction |  |
| ttx_diff | `--timings` | --timing |  |
| ttx_diff | `--unwrap_extensions` | --unwrap-extensions |  |
| ttx_diff | `--fontc_font / --fontmake_font` | diff A B --normalize compiler (or --against FONT) |  |
| ttx_diff | `--version / -V` | --version |  |
| fontc_crater | ci TO_RUN | --corpus FILE |  |
| fontc_crater | `-c / --cache` | --cache |  |
| fontc_crater | `-o / --out` | --output-dir |  |
| fontc_crater | `--gftools` | --mode gftools |  |
| fontc_crater | `--html-only` | --report html --html-only |  |

### `dump`

Writes the contents of a font as text: every table or chosen tables, as TTX-compatible XML, JSON, a readable report or hex. Tables can be shown in named views (raw fields, a friendlier listing, feature-file syntax, kerning pairs and so on), and the font program can be listed per glyph. A Type 1 font is dumped as its decrypted, readable text.

```
fontcraft-cli dump FONT... [-t TAG[=VIEW]]... [-x TAG]... [--format ttx|json|text|hex] [--split tables|glyphs] [-o PATH]
```

**Formats.** `ttx` writes the XML dialect of fontTools so that dumps from either tool can be compared and
`convert` can read both back. `json` carries the same tree. `text` is the report style of AFDKO `spot` and
`tx -dump`. `hex` prints each table as offset, bytes and ASCII.

**Table views** (`-t TAG=VIEW`, text format). A table without a view uses `raw` in text format.

| Table | View | Shows | AFDKO spot |
|---|---|---|---|
| any | `raw` | every field as stored | default |
| BASE | `list`, `friendly` | baseline tags and coordinates; the second with script names and axis headings | `=4`, `=5` |
| CFF, CFF2 | `literal` | the table's own structures, selected by `--cff-parts` | `tx -dcf` |
| GDEF | `fea` | glyph classes, attachment points and carets as a feature-file `table GDEF` block | `=7` |
| GPOS | `features` | script, language and feature list with lookup indices | `=5` |
| GPOS | `pairs` | kerning as AFM-style `KPX left right value` lines | `=6` |
| GPOS, GSUB | `fea` | lookups decompiled to feature-file syntax | `=7` |
| GPOS, GDEF | `normalized` | the rule listing of `features normalize` | otl-normalizer |
| GSUB | `names` | raw structure with glyph names or CIDs instead of indices | `=4` |
| GSUB | `features` | script, language and feature list | `=5` |
| VORG | `names` | vertical origins by glyph name and index | `=4` |
| avar | `designspace` | the mapping written as designspace `<map>` elements | varLib.avar.unbuild |
| cmap | `hex-gid`, `dec-gid`, `hex-name`, `dec-name` | code to glyph, code in hex or decimal, glyph as index or name | `=5`–`=8` |
| cmap | `summary` | one line per subtable: platform, encoding, language, format, count | `=11` |
| glyf | `points` | contours as point lists with on/off-curve flags | `=5` |
| hmtx | `advances`, `advances-lsb` | by glyph name; `--units 1000` rescales | `=5`/`=6`, `=7`/`=8` |
| kern | `headers`, `indices`, `names`, `pairs` | subtable headers; pairs by index; pairs by name; flat pair list | `=2`–`=5` |
| name | `raw`, `lines`, `described` | fields; one line per record; fields with platform, encoding and language spelled out | `=2`, `=3`, `=4` |

The graphical spot views (`CFF_=6/7/8`, `glyf=6/7/8`, `cmap=9/10`, `GPOS=8`, `GSUB=8`, `VORG=8`) are
`proof` layouts.

**Pseudo-tables.** `@font` lists what a renderer needs besides the outlines: names, FontBBox, units per
em, the hint dictionaries (zones, stems, blue scale), and for CID fonts the registry-ordering-supplement
and font-dict array. `@glyphs` lists each selected glyph: index, name or CID, encoding, advance, bounding
box, and with `medium`/`long` detail the path in absolute coordinates with hints. Both work for CFF, CFF2,
TrueType and Type 1 alike, as `tx -dump` does through its abstract font.

**Normalised dump.** `--normalize` is the `ttxn` idea: remove everything that can differ between two
functionally equal fonts. Charstrings are written without subroutines; GSUB/GPOS are written as feature
code grouped by feature with script/language sets, with class names derived from their members so they do
not depend on build order; lookups appear without Extension wrappers unless `--show-extensions`; glyph
references use names; checksums are omitted.

**Fallbacks.** A table that cannot be parsed is written as hex with a warning, and the exit status is 0,
unless `--strict`. Unknown tables are always hex.

Engine command: `font.dumpTables`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| ttx | FONT (dump direction) | dump FONT |  |
| ttx | `-o / -d` | -o / --output-dir |  |
| ttx | `-f` | default (see --no-clobber) |  |
| ttx | `-v / -q` | -v / -q |  |
| ttx | `-a` | --allow-virtual-gids |  |
| ttx | `-l` | tables list |  |
| ttx | `-t / -x` | -t / -x |  |
| ttx | `-s` | --split tables |  |
| ttx | `-g` | --split glyphs |  |
| ttx | `-i` | --no-disassemble |  |
| ttx | `-z` | --bitmap-format |  |
| ttx | `-e` | --strict |  |
| ttx | `-y` | --face |  |
| ttx | `--unicodedata` | --unicode-data |  |
| ttx | `--newline` | --newline |  |
| ttx | `-h / --version` | --help / --version |  |
| ttxn | (default normalised dump, one file per table) | --normalize --split tables |  |
| ttxn | `-nh` | --no-hints |  |
| ttxn | `-nv` | --no-version |  |
| ttxn | `-se` | --show-extensions |  |
| ttxn | `-t / -x / -o / -d and the other ttx dump options` | same as for ttx |  |
| spot | `-t TAG[=N]` | -t TAG[=VIEW] (view names below) |  |
| spot | `-T` | tables list |  |
| spot | `-n / -nc` | glyphs list / glyphs list --one-per-line |  |
| spot | `-ngid` | default: no index is appended to glyph names |  |
| spot | `-m / -ag` | --names source --goadb FILE |  |
| spot | `-F` | features list |  |
| spot | `-o` | --face |  |
| spot | `-C` | --cmap-subtable |  |
| spot | `-of` | -o |  |
| spot | `-g` | --glyphs |  |
| spot | `-u / -h / -ht` | -h / --help (the view names are part of the help) |  |
| spot | `-X` | @FILE or run --script |  |
| tx | `-dump -0` | -t @font --format text |  |
| tx | `-dump -1 / -2 / -3` | -t @font -t @glyphs --glyph-detail short\|medium\|long |  |
| tx | `-dump -4 / -5 / -6` | -t @glyphs --glyph-detail short\|medium\|long |  |
| tx | `-dump -d` | --flatten-seac |  |
| tx | `-dump -n` | --no-hints |  |
| tx | `-dcf` | -t CFF=literal |  |
| tx | `-dcf -T` | --cff-parts |  |
| tx | `-dcf -0 / -1 / -5` | --cff-level offsets\|bytes\|values |  |
| tx | `-dcf -b` | --no-wrap |  |
| tx | `-g / -gx` | --glyphs / --exclude-glyphs |  |
| tx | `-fd / -fdx` | --fd / --exclude-fd |  |
| tx | `-gf / -gr` | --goadb with --names final\|source |  |
| tx | `-i / -y` | --face N / --face all |  |
| detype1 | font [text] | dump FONT --format text (a Type 1 font is written as its decrypted text) |  |
| detype1 | `-h` | --help |  |
| otl-normalizer | font_path, -o, -t, -i | features normalize (also dump -t GPOS=normalized) |  |
| varLib.avar.unbuild | font | dump -t avar=designspace |  |
| ttLib | `-t` | dump -t (decompiling named tables is what dump does) |  |
| pyftsubset | `--xml` | subset --xml (writes the subset and prints it as TTX) |  |

### `info`

Prints a one-screen description of a font or source: format and flavour, names, version, units per em, glyph count, vertical metrics, style flags, tables with sizes, cmap subtables, layout scripts and features, variation axes and named instances, colour and hinting presence, and the members of a collection. For a designspace or other source it lists masters and instances with their indices.

```
fontcraft-cli info INPUT... [--section NAME,...] [--json]
```

`info` answers "what is this file" without the reader needing to know table names. It never fails on a
font that merely has problems: unreadable tables are reported in place and the rest is still printed.

Sections for a binary font: `format` (sfnt flavour, container, outline type, variable or static),
`names` (family, style, full, PostScript, version, vendor, typographic names when present), `metrics`
(units per em, ascender, descender, line gap in hhea / OS/2 typo / OS/2 win, cap height, x-height, italic
angle, underline), `style` (weight class, width class, fsSelection bits, macStyle, fixed pitch, embedding
permission), `tables` (tag, length, checksum), `cmap` (subtables and number of mappings), `layout`
(scripts, languages and features in GSUB and GPOS with lookup counts), `axes` and `instances` (fvar, STAT
axis values), `color` (COLR version, palettes, SVG, bitmap strikes), `hinting` (TrueType program sizes,
gasp ranges, or CFF zones and stems per font dict), `faces` (collection members and which tables they
share).

For a designspace or `.glyphs` source: `axes`, `masters` (name, location, file) and `instances` (index,
name, location, output name). The instance indices are the ones `instance --index` accepts.

Engine command: `font.inspect`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| spot | `-T` | info --section tables (or tables list) |  |
| spot | `-F` | info --section layout (or features list) |  |
| spot | `-t cmap=11` | info --section cmap |  |
| ttx | `-l` | info --section tables (or tables list) |  |
| tx | `-dump -0` | info --section names,metrics,hinting |  |
| tx | (collection contents list when no -i is given) | info --section faces |  |
| otc2otf | `-r / --report` | info --section faces,tables |  |
| makeinstancesufo | `--instance_info` | info DESIGNSPACE --section instances |  |
| tx | `-N` | info --section names (and -v on any subcommand) |  |

### `tables`

Group of actions that treat a font as a set of opaque tables: list the directory, pull tables out as raw bytes, put raw tables in, remove tables, verify and repair checksums, merge tables from an XML dump, and rewrite the file in the recommended table order. Without an action it lists.

```
fontcraft-cli tables list|extract|add|delete|edit|copy|checksum|import|reorder FONT [args]
```

All editing actions rewrite the whole file: tables are copied in the order the OpenType specification
recommends, every table checksum, the directory search fields and `head.checkSumAdjustment` are
recomputed, and padding is normalised. They never parse the tables they move, so they work on tables the
tool does not understand and on damaged fonts. For WOFF and WOFF2 input the container is unpacked,
edited and packed again with the same settings. `--face` selects the member of a collection; tables shared
between members stay shared unless the edited member changes them.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| sfntedit | (no option) | tables list |  |

#### `tables list`

Lists the table directory: sfnt version, number of tables, and for each table its tag, checksum, offset and length, in file order. For a collection it lists every member and marks shared tables; for WOFF/WOFF2 it adds the compressed length.

```
fontcraft-cli tables list FONT... [--json]
```

Engine command: `tables.list`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| sfntedit | `-l` | tables list |  |
| spot | `-T` | tables list |  |
| ttx | `-l` | tables list |  |

#### `tables extract`

Copies the raw bytes of the named tables to files. The default file name is the font's name without its extension, a period and the tag; TAG=FILE names the file explicitly. The font is not changed.

```
fontcraft-cli tables extract FONT TAG[=FILE]... [--output-dir DIR]
```

Engine command: `tables.extract`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| sfntedit | `-x` | tables extract TAG[=FILE]... |  |

#### `tables add`

Adds each named table from a file of raw bytes, replacing a table of the same tag if the font has one, and rewrites the font with fresh checksums.

```
fontcraft-cli tables add FONT TAG=FILE... [-o OUT | --in-place]
```

Engine command: `tables.add`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| sfntedit | `-a` | tables add TAG=FILE... |  |

#### `tables delete`

Removes the named tables and rewrites the font with fresh checksums. Naming a table the font does not have is a warning, not an error.

```
fontcraft-cli tables delete FONT TAG... [-o OUT | --in-place]
```

Engine command: `tables.delete`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| sfntedit | `-d` | tables delete TAG... |  |

#### `tables edit`

Performs extraction, deletion and addition in one pass over the file, always in that order regardless of how the options are written, so a table can be saved, removed and replaced in a single call.

```
fontcraft-cli tables edit FONT [--extract TAG[=FILE],...] [--delete TAG,...] [--add TAG=FILE,...] [-o OUT | --in-place]
```

Engine command: `tables.edit`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| sfntedit | `-x … -d … -a … in one call` | tables edit --extract … --delete … --add … |  |
| sfntedit | srcfile [dstfile] | FONT with -o OUT, or --in-place |  |
| sfntedit | `-u / -h` | -h / --help |  |
| sfntedit | `-X` | @FILE or run --script |  |

#### `tables copy`

Copies the named tables from one font into another without an intermediate file; it is tables add with the bytes read from the first font. The written font is TO (or OUT).

```
fontcraft-cli tables copy FROM TO TAG... [-o OUT | --in-place]
```

Engine command: `tables.add`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| sfntedit | `-x TAG src; -a TAG=file dst` | tables copy src dst TAG |  |

#### `tables checksum`

Verifies every table checksum and the whole-font checksum adjustment in head and reports mismatches; exit status 1 when any is wrong. With --fix it rewrites the font with correct values, which also reorders tables and recomputes the directory search fields.

```
fontcraft-cli tables checksum FONT... [--fix] [-o OUT | --in-place]
```

Engine command: `tables.checksum`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| sfntedit | `-c` | tables checksum |  |
| sfntedit | `-f` | tables checksum --fix |  |

#### `tables import`

Compiles the tables found in one or more XML dumps and places them in an existing font, replacing tables of the same tag and leaving all others untouched. Useful for patching metadata tables with a hand-edited dump.

```
fontcraft-cli tables import FONT DUMP.ttx... [-o OUT | --in-place]
```

Engine command: `tables.importTtx`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| ttx | `-m` | tables import FONT DUMP.ttx |  |
| ttx | `-b` | --no-recalc-bounds |  |
| pyftmerge | `--import-file` | merge --import-ttx, or tables import afterwards |  |

#### `tables reorder`

Rewrites the font with its tables in the order the OpenType specification recommends for its flavour, or in an explicit order; tables not named keep their relative order after the named ones.

```
fontcraft-cli tables reorder FONT [--order recommended|TAG,...] [-o OUT | --in-place]
```

Engine command: `tables.reorder`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| pyftsubset | `--canonical-order / --no-canonical-order` | tables reorder (subset --canonical-order does the same in one step) |  |
| sfntedit | -f (copies tables in recommended order) | tables reorder |  |

#### `tables show`

Shows the decoded fields of tables as a tree, following each table's structure in the OpenType
specification: record and field names, values, offsets followed to their records, glyph indices
with their names. It is our own subcommand, after the reference vendor's free table viewer; the
Table Viewer window shows the same tree and compares two fonts through `diff`.

```
fontcraft-cli tables show FONT [TAG...] [--path PATH] [--depth N] [--offsets] [--format json|text]
```

- `--path GPOS.lookupList.lookups[3]` narrows the output to one record or field.
- `--depth N` (default 3) limits how many levels of nested records are expanded; deeper records
  are counted, not shown.
- A table the reader cannot decode is listed with its error; its bytes stay reachable through
  `tables extract`.

Engine command: `tables.show`. Unlike `dump`, which reproduces the reference tools' formats,
`tables show` has no oracle: its tree is read-fonts' traversal of the table, checked by tests on
fonts built in code.

### `glyphs`

Group of actions on the glyph set as a list: show index, name, CID and code points; rename glyphs; change the glyph order. Subsetting is the separate subcommand subset. Without an action it lists.

```
fontcraft-cli glyphs list|rename|reorder FONT [args]
```

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| spot | `-n` | glyphs list |  |

#### `glyphs list`

Lists the glyphs in glyph order with index, name (or CID), code points and optionally advance, bounding box, font dict and component flag. The default is a compact multi-column listing of index and name.

```
fontcraft-cli glyphs list FONT [--columns COLS] [--one-per-line] [--glyphs SELECTOR]
```

Engine command: `font.listGlyphs`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| spot | `-n` | glyphs list |  |
| spot | `-nc` | glyphs list --one-per-line |  |
| spot | `-m` | --names source --goadb FILE |  |
| tx | `-dump -4` | glyphs list --columns gid,name,unicodes,advance,bbox |  |

#### `glyphs rename`

Renames glyphs everywhere they are referenced: outline table, post or CFF names, and for sources also components, kerning, groups and feature code. Names come from a GOADB (source name to final name), a two-column map, the production-name rules, or pairs on the command line. Renaming two glyphs to the same name is an error.

```
fontcraft-cli glyphs rename FONT (--goadb FILE | --map FILE | --production-names | OLD=NEW...) [-o OUT | --in-place]
```

Engine command: `font.renameGlyphs`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| tx | `-gf` | glyphs rename --goadb |  |
| tx | `-gr` | omit glyphs rename (convert --goadb F --no-rename) |  |
| tx | `-go` | glyphs rename --goadb F --order, or glyphs reorder --goadb F |  |
| makeotf | `-ga with -gf` | compile --apply-goadb, or glyphs rename --goadb F --order --update-unicodes on an existing font |  |

#### `glyphs reorder`

Changes the glyph order of a font and rewrites everything indexed by glyph: outlines, metrics, cmap, coverage and class tables in GDEF/GSUB/GPOS, variation data, colour tables. The new order must contain every glyph exactly once, except that glyphs left unnamed keep their relative order at the end; .notdef stays at index 0.

```
fontcraft-cli glyphs reorder FONT (--order-file FILE | --goadb FILE | --sort KEY | NAME...) [-o OUT | --in-place]
```

Engine command: `font.reorderGlyphs`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| ttLib.reorderGlyphs | reorderGlyphs(font, new_glyph_order) | glyphs reorder |  |
| tx | `-go` | glyphs reorder --goadb |  |
| tx | `+b / -b` | convert --keep-glyph-order / (default lets the writer sort) |  |

### `metrics`

Reports glyph metrics computed from the outlines: per glyph the identity, advance width and exact bounding box, optionally side bearings, the font-wide bounding box with the glyphs that set each edge, the kerning pair list, and shape statistics (area, centre of mass, slant). It can also write an AFM file.

```
fontcraft-cli metrics FONT... [--format text|json|csv|afm] [--precision integer|real] [--aggregate-bbox] [--kerning] [--statistics] [--glyphs SELECTOR]
```

Bounding boxes are computed from the curves themselves, not from control points, so they are exact; the
values stored in `head`, `hhea` or the CFF Top DICT are not consulted (comparing the two is a `check`).

The AFM output is a convenience for tools that still read the format. As in `tx -afm` the numbers are in
the font's own units, which is noted in a comment when that is not 1000 per em, and the character count is
the number of glyphs actually written when a subset was selected.

`--statistics` weights the font-wide averages by glyph area in the same way fontTools' statistics pen does
and prints `weight` and `width` estimates derived from them; these are heuristics, labelled as such.

Engine command: `font.reportMetrics`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| tx | `-mtx -0 / -1` | --precision integer / real |  |
| tx | `-mtx -2 / -3` | --aggregate-bbox with --precision integer / real |  |
| tx | `-afm` | --format afm |  |
| tx | `-g / -gx / -fd / -fdx` | --glyphs / --exclude-glyphs / --fd |  |
| tx | `-U` | --location |  |
| spot | `-t hmtx=5 / =6` | metrics [--units 1000] (or dump -t hmtx=advances) |  |
| spot | `-t hmtx=7 / =8` | metrics --sidebearings [--units 1000] |  |
| spot | `-t kern=5, -t GPOS=6` | metrics --kerning (or dump -t GPOS=pairs) |  |
| pens.statisticsPen | font [glyph…] | metrics --statistics --glyphs … |  |
| pens.statisticsPen | `-y` | --face |  |
| pens.statisticsPen | `-c / --control` | --control-box |  |
| pens.statisticsPen | `-q / --quiet` | --font-only |  |
| pens.statisticsPen | `--variations` | --location |  |

### `convert`

Rewrites a font or source in another format while keeping its content: between outline flavours (CFF, CFF2, TrueType), between container and bare forms (OTF, bare CFF, Type 1 PFA/PFB, decrypted Type 1 text, TTX XML), from binary to UFO, and between source formats (native, .glyphs, UFO 2/3, designspace). Converting to the format the input already has re-reads and re-writes it, which normalises the file.

```
fontcraft-cli convert INPUT... --to FORMAT [-o PATH | --output-dir DIR] [options]
```

**Scope.** `convert` never changes what the font *is*: the glyph set, metrics, layout behaviour and names
stay, only the representation changes. Operations that change content have their own subcommands
(`subset`, `instance`, `transform`, `outlines …`), although the glyph-selection and snapshot options of
`tx` are accepted here too so that one call can replace one `tx` call.

**What survives each conversion.**

| From → to | Outlines | Hints | Other tables |
|---|---|---|---|
| OTF (CFF) → TTF | cubic to quadratic within `--max-error`, direction reversed | PostScript hints dropped | kept; `CFF`, `VORG` removed, `glyf`, `loca`, `maxp` 1.0, `post` built |
| TTF → OTF | quadratic to cubic (`--curve-conversion`), direction reversed, components decomposed | TrueType instructions dropped | kept; `glyf`, `loca`, `cvt`, `fpgm`, `prep`, `gasp`, `hdmx`, `LTSH`, `VDMX` removed |
| CFF → CFF2 (static) | unchanged | kept | widths move to `hmtx`, names to `post` |
| CFF2 → CFF | unchanged; a variable font must be pinned first | kept | glyph names from `post` |
| any binary → bare `cff` / `pfa` / `pfb` | cubic | kept | only the font program: everything outside the outline table is left behind |
| any binary → `ufo` | as in the font (TrueType curves stay quadratic, components stay) | kept in the glyph lib (`com.schriftgestaltung.hints`) | what `file.open` reads from a compiled font: font info, kerning and groups, feature code; a variable font is snapshotted at its default unless `--location` is given |
| `.glyphs` / native ↔ UFO + designspace | unchanged | manual hints kept in lib data | everything the target format can express; the rest is kept in lib keys and reported |
| TTX XML → binary | as written | as written | compiled table by table |

**Steps around the conversion**, in the order `tx` runs its input options:

1. `--location`: a variable font is snapshotted at the location (axes not named sit at their
   default; values are clamped to the axis ranges, or refused outside them with `--no-clamp`).
   Without `--location`, a variable font written as UFO or with another outline flavour is
   snapshotted at its default; one that keeps its flavour keeps its variations.
2. `--glyphs`, `--exclude-glyphs`, `--fd-select`, `--exclude-fd` and `--subset-to-goadb`: only
   the selected glyphs and `.notdef` stay, in font order; with several selections, the glyphs
   all of them select. `/CID` and `/CID-CID` items work in CID-keyed fonts; `--fd-select` works
   in CID-keyed CFF and in CFF2 fonts.
3. `--goadb`: glyphs are renamed from the second column to the first (not with `--no-rename`);
   `--goadb-order` puts the listed glyphs first, in the file's order.
4. The conversion.
5. `--remove-overlaps`: overlapping contours are merged on TrueType outlines, before converting
   a TrueType font or after converting to TrueType, and on CFF or CFF2 outlines before
   converting them. A merged CFF glyph gets a new charstring without stem hints.
6. `--cid-keyed[=R-O-S]`: a name-keyed CFF result becomes CID-keyed, CID n for glyph n, with one
   Font DICT (Adobe-Identity-0 unless a collection is given), as makeotf `-cn` does.

`tx` skips a glyph name `-g` gives that the font lacks; `convert` refuses it.

**Type 1.** Reading accepts PFA, PFB and the decrypted text form. Writing produces ordinary host fonts
only. Multiple-master Type 1 input is always snapshotted, at `--location` or at its default instance.

**CID-keyed UFOs** follow the AFDKO convention: `com.adobe.type.ROS`, `com.adobe.type.postscriptCIDMap`
and optionally `com.adobe.type.postscriptFDArray` in `lib.plist`, with one `FDArraySelect.<index>[.<name>]`
group per font dict. Converting a CID-keyed font to UFO names glyphs `cidNNNNN`; converting such a UFO to
CFF puts glyphs in CID order whatever the UFO's glyph order is.

**Name-keyed fonts above 65,000 glyphs** cannot be written as CFF (string index limit); the error says to
use `--cid-keyed`.

Engine command: `font.convert`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| tx | `-cff` | --to cff (bare) or --to otf |  |
| tx | `-cff2` | --to cff2 (bare) or --to otf-cff2 |  |
| tx | `-t1` | --to pfa |  |
| tx | `-t1 -pfb` | --to pfb |  |
| tx | `-t1 -0` | the only Type 1 form written (host font) |  |
| tx | `-t1 -l / -b` | --to pfa / --to pfb | binary eexec data is written only inside PFB segments |
| tx | `-ufo` | --to ufo |  |
| tx | `-svg` | --to svg-font |  |
| tx | `+S / -S` | --subroutinize / --no-subroutinize |  |
| tx | `-maxs` | --max-subrs |  |
| tx | `+T / -T, -no_opt` | --charstring-optimize specialize / none |  |
| tx | `+V / -V` | --remove-overlaps / (default) |  |
| tx | `-Z / +Z` | --flatten-seac / (default) |  |
| tx | `+b / -b` | --keep-glyph-order / (default) |  |
| tx | `+W / -W` | --optimize-widths / --no-optimize-widths |  |
| tx | `+E / -E` | --cff-embed-minimal / (default) |  |
| tx | `+F / -F` | --dedupe-family-blues / --no-dedupe-family-blues |  |
| tx | `+d / -d, +H / -H` | --warn-hints / --no-warn-hints |  |
| tx | `-n` | --no-hints (or dehint) |  |
| tx | `-std` | --standard-encoding |  |
| tx | `-go` | --goadb-order |  |
| tx | `-amnd` | --allow-missing-notdef |  |
| tx | `-e` | --t1-leniv |  |
| tx | `+q / -q` | --keep-unique-id / --no-keep-unique-id |  |
| tx | `-lf / -cr / -crlf` | --newline |  |
| tx | `-decid` | --decid |  |
| tx | `-usefd` | --fd |  |
| tx | `-gn0 / -gn1 / -gn2` | --svg-glyph-names none\|non-ascii\|all |  |
| tx | `-abs` | --svg-absolute |  |
| tx | `-sa` | --svg-standalone |  |
| tx | `-altLayer` | --layer |  |
| tx | `-g / -gx` | --glyphs / --exclude-glyphs |  |
| tx | `-gf / -gr / -gs` | --goadb / --no-rename / --subset-to-goadb |  |
| tx | `-fd / -fdx` | --fd-select / --exclude-fd |  |
| tx | `-U / -UNC` | --location / --no-clamp |  |
| tx | `-i / -y` | --face N / --face all |  |
| tx | `-x` | --curve-conversion exact |  |
| tx | `-f / -o / src dst` | INPUT... / -o |  |
| tx | `-a` | several INPUTs (default names: stem plus new extension) |  |
| tx | `-A` | --name-by postscript-name |  |
| tx | `-dd` | --output-dir |  |
| tx | `-s` | @FILE |  |
| tx | `-u / -h / -v` | -h / --help / --version -v |  |
| tx | `/? /h /help` | -h / --help |  |
| type1 | text [font] | convert TEXT --to pfa (decrypted Type 1 text is recognised as input) |  |
| type1 | `-h` | --help |  |
| detype1 | font [text] | convert FONT --to t1-text (same as dump --format text) |  |
| otf2ttf | INPUT... | convert INPUT... --to ttf |  |
| otf2ttf | `-o / --output` | -o / --output-dir |  |
| otf2ttf | `-e / --max-error` | --max-error | reference value is in font units; divide by units per em |
| otf2ttf | `--post-format` | --post-format |  |
| otf2ttf | `--keep-direction` | --keep-direction |  |
| otf2ttf | `--face-index` | --face |  |
| otf2ttf | `--overwrite` | --in-place |  |
| cffLib.CFFToCFF2 | input, -o / --output | convert --to otf-cff2 -o |  |
| cffLib.CFFToCFF2 | `--no-recalc-timestamp` | --timestamp keep |  |
| cffLib.CFFToCFF2 | `-v / -q` | -v / -q |  |
| cffLib.CFF2ToCFF | input, -o / --output | convert --to otf -o (a variable font needs --location or instance first) |  |
| cffLib.CFF2ToCFF | `--no-recalc-timestamp` | --timestamp keep |  |
| cffLib.CFF2ToCFF | `--remove-overlaps` | --remove-overlaps |  |
| cffLib.CFF2ToCFF | `--ignore-overlap-errors` | --ignore-overlap-errors |  |
| cffLib.CFF2ToCFF | `-v / -q` | -v / -q |  |
| ttx | FILE.ttx (compile direction) | convert FILE.ttx --to ttf\|otf |  |
| ttx | `-o / -d` | -o / --output-dir |  |
| ttx | `-b` | --no-recalc-bounds |  |
| ttx | `--recalc-timestamp / --no-recalc-timestamp` | --timestamp now / --timestamp keep |  |
| ttx | `--flavor` | --container woff\|woff2 |  |
| ttx | `--with-zopfli` | --zopfli |  |
| ttx | `--optimize-font-speed` | --optimize-for speed |  |
| ttx | `-a` | --allow-virtual-gids |  |
| ttLib | font -o / --output | convert FONT -o OUT (same format: load and save) |  |
| ttLib | `--flavor` | --container |  |
| ttLib | `--no-recalc-timestamp` | --timestamp keep |  |
| ttLib | `-b` | --no-recalc-bounds |  |
| ttLib | `--optimize-font-speed` | --optimize-for speed |  |
| ttLib | `-t / --table` | --rebuild-tables |  |
| ttLib | `-y` | --face |  |
| designspaceLib | input output | convert IN.designspace --to designspace -o OUT (add --normalize for canonical form) |  |
| fontmake | `-o ufo` | convert SOURCE --to ufo (masters; instances come from instance --to ufo) |  |
| fontmake | `--no-write-skipexportglyphs` | --skip-export-key glyphs |  |
| fontmake | `--ufo-structure / --indent-json / --save-ufo-as-zip` | --ufo-structure / --indent-json |  |
| fontmake | `--designspace-path / --master-dir / --family-name` | --emit-designspace / --master-dir / --family-name |  |
| fontmake | `--validate-ufo` | --validate-sources |  |
| makeotf | `-cn` | convert --cid-keyed on the finished font (or compile --cid-keyed) |  |
| mergefonts | `-cid` | --cid-keyed with --cid-info (single input); merge --cid-info for several |  |

### `subset`

Reduces a font to chosen glyphs and removes everything that becomes unreachable: cmap entries, layout rules, variation data, names, hint tables. The kept set starts from glyph names, glyph indices, code points or text, and is closed over components and over the substitutions of the retained layout features. Defaults follow pyftsubset, which aims at small web fonts; each trimming step can be switched off.

```
fontcraft-cli subset FONT [GLYPH...] [--unicodes LIST] [--text STRING] [--glyphs LIST] [--gids LIST] [options] [-o OUT]
```

**Glyph set.** The initial set is the union of every selection option. It is then closed over: composite
components; the `.notdef` policy; for each retained feature, the glyphs its substitution rules can produce
from glyphs already in the set (repeated until stable) unless `--no-layout-closure`; COLR layers and
paint glyphs; MATH variants; mirrored pairs. Giving no selection at all is an error; `'*'` must be
explicit.

**After the set is fixed** each table is trimmed: outlines and metrics of dropped glyphs go; cmap loses
their mappings; GSUB/GPOS/GDEF lose rules, classes and coverage entries that mention them, then lookups,
features, languages and scripts that became empty; variation data, COLR/CPAL, SVG, VORG, MATH and BASE
are cut the same way. Glyph indices are renumbered densely unless `--retain-gids`.

**Defaults that surprise.** Names other than IDs 0–6 in English are removed, glyph names are removed from
TrueType fonts, `.notdef` loses its outline, legacy cmap subtables and the `kern` table go, and only the
core feature list survives. A subset that should behave exactly like the original but with fewer glyphs
needs: `--layout-features '*' --glyph-names --symbol-cmap --legacy-cmap --notdef-outline --name-ids '*'
--name-legacy --name-languages '*'`.

**Missing items.** A requested glyph name or index that does not exist is an error by default; a requested
character that is not mapped is ignored by default. The asymmetry is inherited and kept for
compatibility.

**Variable components.** The closure follows `glyf` components and `VARC` components in one work list.
pyftsubset closes over `VARC` first and `glyf` second, so when only a `glyf` composite reaches a `VARC`
glyph, it keeps that glyph without the glyphs its variable components name, and then fails to write the
table. FontCraft keeps those glyphs too, so the subset can be written. A subset that never mixes the two
tables this way keeps the same glyphs as pyftsubset.

**Sources.** `subset` also works on UFO, designspace and `.glyphs` input: glyphs are removed from every
master and layer, and from groups, kerning and class definitions; feature code is left alone and a warning
lists rules that now refer to missing glyphs.

Engine command: `font.subset`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| pyftsubset | font-file, glyph… | FONT GLYPH... |  |
| pyftsubset | `--gids / --gids-file` | --gids / --gids-file |  |
| pyftsubset | `--glyphs / --glyphs-file` | --glyphs / --glyphs-file |  |
| pyftsubset | `--text / --text-file` | --text / --text-file |  |
| pyftsubset | `--unicodes / --unicodes-file` | --unicodes / --unicodes-file |  |
| pyftsubset | `--ignore-missing-glyphs / --no-ignore-missing-glyphs` | same |  |
| pyftsubset | `--ignore-missing-unicodes / --no-ignore-missing-unicodes` | same |  |
| pyftsubset | `--output-file` | -o |  |
| pyftsubset | `--flavor` | --container |  |
| pyftsubset | `--with-zopfli` | --zopfli |  |
| pyftsubset | `--retain-gids` | --retain-gids |  |
| pyftsubset | `--notdef-glyph / --no-notdef-glyph` | same |  |
| pyftsubset | `--notdef-outline / --no-notdef-outline` | same |  |
| pyftsubset | `--recommended-glyphs / --no-recommended-glyphs` | same |  |
| pyftsubset | `--layout-closure / --no-layout-closure` | same |  |
| pyftsubset | `--bidi-closure / --no-bidi-closure` | same |  |
| pyftsubset | `--layout-features[+\|-]=` | --layout-features (with +tag / -tag items) |  |
| pyftsubset | `--layout-scripts[+\|-]=` | --layout-scripts |  |
| pyftsubset | `--hinting / --no-hinting` | same |  |
| pyftsubset | `--hinting-tables` | --hinting-tables |  |
| pyftsubset | `--desubroutinize / --no-desubroutinize` | same |  |
| pyftsubset | `--drop-tables` | --drop-tables |  |
| pyftsubset | `--no-subset-tables` | --no-subset-tables |  |
| pyftsubset | `--passthrough-tables / --no-passthrough-tables` | same |  |
| pyftsubset | `--legacy-kern / --no-legacy-kern` | same |  |
| pyftsubset | `--name-IDs` | --name-ids |  |
| pyftsubset | `--name-legacy / --no-name-legacy` | same |  |
| pyftsubset | `--name-languages` | --name-languages |  |
| pyftsubset | `--obfuscate-names` | --obfuscate-names |  |
| pyftsubset | `--glyph-names / --no-glyph-names` | same |  |
| pyftsubset | `--legacy-cmap / --no-legacy-cmap` | same |  |
| pyftsubset | `--symbol-cmap / --no-symbol-cmap` | same |  |
| pyftsubset | `--recalc-bounds / --no-recalc-bounds` | same |  |
| pyftsubset | `--recalc-timestamp / --no-recalc-timestamp` | --timestamp now / --timestamp keep |  |
| pyftsubset | `--canonical-order / --no-canonical-order` | same |  |
| pyftsubset | `--prune-unicode-ranges / --no-prune-unicode-ranges` | same |  |
| pyftsubset | `--prune-codepage-ranges / --no-prune-codepage-ranges` | same |  |
| pyftsubset | `--recalc-average-width / --no-recalc-average-width` | same |  |
| pyftsubset | `--recalc-max-context / --no-recalc-max-context` | same |  |
| pyftsubset | `--font-number` | --face |  |
| pyftsubset | `--pretty-svg` | --pretty-svg |  |
| pyftsubset | `--verbose` | -v |  |
| pyftsubset | `--timing` | --timing |  |
| pyftsubset | --option=? (query current value) | --option? (same output line); --show-defaults prints every option |  |
| tx | `-g` | --glyphs |  |
| tx | `-gx` | --exclude-glyphs |  |
| tx | `-fd / -fdx` | --fd / --exclude-fd |  |
| tx | `-p` | --random PERCENT |  |
| tx | `-P` | --random PERCENT --seed random |  |
| tx | `-pg` | --retain-gids |  |
| tx | `-gs with -gf` | --goadb FILE |  |
| mergefonts | one source font with -g / -gx or a glyph alias file | subset (and glyphs rename for the renaming part) |  |
| makeotf | `-gs` | compile --subset-to-goadb, or subset --goadb on the finished font |  |
| fontmake | `--subset / --no-subset` | compile --subset; subset --export-flags on a source |  |
| fontc | `-g / --glyph-name-filter` | subset --glyphs re:PATTERN on a source, or compile --glyphs |  |

### `instance`

Produces an instance of a variable design: from a variable font, a static font or a variable font with fewer or narrower axes; from a designspace or other multi-master source, interpolated UFO or binary instances. Each axis can be pinned to a value, limited to a range, dropped at its default or left alone.

```
fontcraft-cli instance INPUT [AXIS=LOC...] [--named REGEX | --all | --index LIST] [--static] [--to same|ttf|otf|ufo] [-o OUT | --output-dir DIR]
```

**Variable font in, font out.** With every axis pinned (or `--static`) the result is a static font:
`fvar`, `avar`, `gvar`/CFF2 blends, `HVAR`, `VVAR`, `MVAR`, `cvar` and variation data inside
GDEF/GPOS/BASE/COLR are applied and removed, `STAT` is reduced, and feature variations are resolved to
the substitutions active at the location. With some axes left or limited the result is a smaller variable
font: deltas are rescaled to the new ranges, tuples that no longer contribute are dropped, the default
moves when `MIN:DEFAULT:MAX` says so, `avar` is rebuilt for the new ranges and named instances outside
them are removed.

**Naming.** By default names are not touched, so a static result still carries the variable font's
names. `--update-name-table` derives them from `STAT`; it fails if the font has no `STAT` axis values for
the location. `--named` and `--all` take the names from the named instance instead and name each output
file after its PostScript name.

**CFF2 outlines.** Every blend of the charstrings and Private DICTs is resolved at the location, the
deltas rounded half to even as varLib.instancer rounds them; a partial instance rebases each variation-store
subtable and keeps blends where something still varies. The result has no subroutines; hints are kept.
`HVAR` and `VVAR` move the advances. `--downgrade-cff2` writes a CID-keyed CFF table once nothing in the
outlines varies. Two differences from varLib.instancer change no outline: the charstring operators are kept
as they were (the instancer respecialises them), and a downgraded font has its `head` and `hhea` bounds and
the CFF FontBBox recomputed (the instancer keeps the input's `head` and `hhea` values when it downgrades).

**`avar` version 2.** A partial instance keeps the variation tables in the coordinates the input's
`avar` table gives, as varLib.instancer does. The `avar` segment maps are rebased (a pinned axis's map
becomes the identity map), its item variation store is rebased by the limits, and each limited axis gets
deltas on its own row that map the new coordinates back to the old ones (for a moved default, also at the
point where the old default lands and at the segment map's interior points, when that is closer). A
pinned axis whose final coordinate no longer depends on another axis leaves `fvar` and `avar`, and the
variation tables are instanced at that coordinate; any other pinned axis stays as a hidden axis whose
range is the pinned value. The instancer also drops variation regions the new ranges cannot reach and
re-packs the store; neither changes a value, and FontCraft does neither.

**`VARC`.** An axis a `VARC` table uses (in its variation regions, its axis-range conditions or its axis
index lists) cannot be pinned or limited; the instance is refused with a message, as varLib.instancer
refuses it. Other axes can be, and the table's axis indexes are renumbered when axes leave `fvar`. Its
variation data does not change.

**Variable `COLR`.** varLib.instancer leaves `COLR` as it is. FontCraft instances it as the instancer
instances `GDEF`: the store is rebased, and each varied field of a paint, colour stop or clip box takes
the delta the location folds into the default, rounded, in the field's own units. A static instance, or
a partial one that leaves no region, drops the store and the delta-set index map and sets every
variation index to "no variation"; the paints keep their variable formats.

**Tables recalculated on saving.** The instancer reads every font's glyph order, which loads `post`,
so `post` is written again as fontTools writes it: version 2 names stored again (standard names by
their index, unused strings past the last used one dropped), versions 1 and 3 as their header alone,
and version 4 with a code that an earlier glyph has, or a glyph past the stored codes, set to 0xFFFF.
When `glyf` was rewritten and bounds are recalculated, `maxp` and `head.flags` bit 1 are recalculated
too, as fontTools' `maxp.recalc` does.

**Overlaps.** Variable TrueType fonts normally contain overlapping contours. A static instance keeps
them and sets the overlap flags so that rasterisers use non-zero winding; `--remove-overlaps` merges them
instead and drops hinting of changed glyphs.

**Source in, sources or fonts out.** For a designspace or multi-master source, each requested instance is
interpolated: outlines, components, anchors, metrics, kerning, font info and lib data. By default the
instances are written as UFOs to the paths recorded in the designspace, rounded, PostScript-autohinted and
normalised; overlap removal is opt-in. `--to ttf|otf` compiles them in the same run, which is `compile
--instances`. Glyphs whose masters are not compatible are skipped with a warning, or stop the run with
`--strict`.

**Layout only.** `--layout-only` reads only the designspace (not its UFOs), or a `.glyphs` or `.glyphspackage` source, and the compiled masters. A designspace source names its master font or TTX file, or `--master-finder` turns the source's file name into the master's (`{fullname}`, `{dirname}`, `{basename}`, `{stem}`, `{ext}`; default `master_ttf_interpolatable/{stem}.ttf`, relative to the source's folder). For a `.glyphs` source the file names are those glyphsLib gives the master UFOs when fontmake builds its designspace: `master_ufo/` beside the source, the master's `UFO Filename` parameter, else `Family-Master.ufo` with the spaces taken out. The result is the base master with its `GPOS` interpolated at the location, as fontTools' `varLib.interpolate_layout` makes it; `GDEF`, `GSUB` and every other table stay the base master's. Masters whose `GPOS` tables differ in structure are refused.

**Binary layout.** `--interpolate-binary-layout [DIR]` with `--to ttf|otf` is fontmake's compiled-instance path: each instance is compiled, then its `GPOS` is replaced by the one interpolated from the compiled masters, and its `GDEF` and `GSUB` by the default master's when that master has them. The masters are `DIR/{stem}.ttf` (or `.otf`), `DIR` defaulting to `master_ttf` (or `master_otf`) beside the source; `--master-finder` names them another way. The masters are read before anything is compiled, so a missing master writes nothing. The instances are compiled into a scratch folder inside the destination and moved to their paths only once every layout is replaced, so a failure after compiling writes nothing either. A bare `--interpolate-binary-layout` takes the next argument as `DIR` only when it does not start with `-` and holds no `=`; this holds for a shortened name, another letter case and a response file too.

**Extrapolation.** Locations are clamped to the axis range unless `--no-clamp`. Extrapolation needs the
masters themselves and is only possible from a source or for CFF2 and multiple-master input.

Engine command: `font.instantiate`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| varLib.instancer | INPUT.ttf AXIS=LOC… | INPUT AXIS=LOC... |  |
| varLib.instancer | `-o / --output` | -o |  |
| varLib.instancer | `--static` | --static |  |
| varLib.instancer | `--no-optimize` | --no-optimize-gvar |  |
| varLib.instancer | `--no-overlap-flag` | --no-overlap-flag |  |
| varLib.instancer | `--remove-overlaps` | --remove-overlaps |  |
| varLib.instancer | `--ignore-overlap-errors` | --ignore-overlap-errors |  |
| varLib.instancer | `--update-name-table` | --update-name-table |  |
| varLib.instancer | `--downgrade-cff2` | --downgrade-cff2 |  |
| varLib.instancer | `--no-recalc-timestamp` | --timestamp keep |  |
| varLib.instancer | `--no-recalc-bounds` | --no-recalc-bounds |  |
| varLib.instancer | `-v / --verbose / -q / --quiet` | -v / -q |  |
| varLib.mutator | INPUT.ttf AXIS=LOC… | INPUT AXIS=LOC... --static |  |
| varLib.mutator | `-o / --output` | -o |  |
| varLib.mutator | `--no-recalc-timestamp` | --timestamp keep |  |
| varLib.mutator | `--no-overlap` | --no-overlap-flag |  |
| varLib.mutator | `-v / --verbose / -q / --quiet` | -v / -q |  |
| varLib.interpolate_layout | DESIGNSPACE LOCATION… | instance DESIGNSPACE AXIS=LOC... --layout-only |  |
| varLib.interpolate_layout | `-o / --output` | -o |  |
| varLib.interpolate_layout | `-l / --loglevel` | --log |  |
| makeinstancesufo | `-d / --designspace` | INPUT (no default file name) |  |
| makeinstancesufo | (all instances) | --all --to ufo |  |
| makeinstancesufo | `-i` | --index |  |
| makeinstancesufo | `-a / --no-autohint` | --no-autohint |  |
| makeinstancesufo | `-c / --no-remove-overlap` | (default; overlaps are removed only with --remove-overlaps) | default differs: AFDKO removes overlaps unless told not to |
| makeinstancesufo | `-n / --no-normalize` | --no-normalize |  |
| makeinstancesufo | `-r / --no-round` | --no-round |  |
| makeinstancesufo | `-s / --strict` | --strict |  |
| makeinstancesufo | `--ufo-version` | --ufo-version |  |
| makeinstancesufo | `--instance_info` | info --section instances |  |
| makeinstancesufo | `-v / --verbose / --version` | -v / --version |  |
| fontmake | `-i / --interpolate [NAME]` | instance SOURCE --all \| --named REGEX (binary output: compile --instances) |  |
| fontmake | `--round-instances` | --round |  |
| fontmake | `--instance-dir` | --instance-dir |  |
| fontmake | `--interpolate-binary-layout` | --to ttf\|otf --interpolate-binary-layout [DIR] (or --layout-only for the base master alone) |  |
| tx | `-U` | AXIS=LOC... (a bare number list is read in axis order) |  |
| tx | `-UNC` | --no-clamp |  |

### `merge`

Combines several fonts into one. Font-wide data comes from the first font; glyphs are added from the others, optionally only those listed in a per-font map file that can also rename them. Depending on --layout, the cmap and layout tables of all fonts are merged as well, or only the first font's are kept. The output has the format of the first font unless --to says otherwise.

```
fontcraft-cli merge FONT [--map FILE] [--glyphs LIST] FONT [--map FILE] ... -o OUT [options]
```

Two reference tools merge in different ways and `merge` covers both.

*Glyph merge* (AFDKO `mergefonts`): the base font supplies every font-level value; other fonts only
contribute outlines, widths and hints of the glyphs selected for them. A map file both selects and
renames. The same font may be given twice with different maps to duplicate glyphs under new names. The
result must contain `.notdef`: if every input has a map, one of them must map it. This is the mode for
bare font programs and for assembling CID-keyed fonts from per-row or per-script parts; `--layout base`
or `drop` selects it for sfnt input.

*Font merge* (fontTools `pyftmerge`): whole OpenType fonts are combined. Glyph order is the first font's
followed by every glyph of each later font; a name already used gets `.1`, `.2`… appended (`--duplicates`
can keep the earlier glyph, keep the later one or refuse instead); cmaps are united. When two fonts map
the same character, the earlier font keeps the mapping and a lookup added to the `locl` feature under
the later font's scripts substitutes the later font's glyph (this needs a GSUB table). fontTools 4.66
does this even when the two glyphs are identical, and so does `merge`. GSUB/GPOS script, feature and lookup lists are concatenated
with indices renumbered; GDEF classes are united; vertical extents such as `hhea` ascent take the
largest value; `OS/2` range bits are OR-ed; names and other metadata come from the first font. All fonts must share units per em (rescale with `transform --upem`
first) and outline flavour (use `convert`).

Glyph names are the ones fontTools gives each font: the `CFF ` charset; `post` version 1, 2 or 4
names (version 4 names come from its code points); else names made from the Unicode `cmap` (the
AGLFN name of a glyph's lowest code point, `uniXXXX`, `glyphNNNNN` for glyphs without one). The
result's `post` takes the highest version of the fonts: version 1 or 3 is the header alone, version 4
stores a code point for each merged name (0xFFFF for renamed duplicates and names without one), and
version 2 stores the merged names.

CFF fonts merge as fontTools merges them: subroutines are expanded, the first font's Top and Private
DICTs are kept, and each later glyph's width operand is rewritten for the first font's `defaultWidthX`
and `nominalWidthX`; `merge` then subroutinises the result again. `MATH` and `BASE` are merged too: the
first font's constants and baselines, every font's glyph data and scripts.

Refused, because fontTools cannot merge them either: `CFF2` fonts, CID-keyed CFF fonts, `JSTF`
tables (leave them out with `--drop-tables JSTF`), and `BASE` tables with variation data.

**Variable fonts.** fontTools' merge has no rule for variation tables: it drops them and keeps the
default outlines. `merge` keeps variable TrueType fonts variable when they share their axes. The rules
are FontCraft's own:

| Table | Rule |
|---|---|
| `fvar` | Every font must be variable, with the same axes (tag, minimum, default, maximum, in order); otherwise the merge is refused. The result has the first font's axis records and the named instances of every font; an instance at coordinates an earlier font already names is left out. A later font's instance names are copied into the result's `name` table. |
| `avar` | Must be the same in every font. |
| `gvar` | Each glyph keeps its variation data. The shared tuples of all fonts are united and the tuple headers renumbered. |
| `HVAR`, `VVAR` | Each font's store keeps the delta sets its glyphs use. The stores are joined (equal regions shared) and the index maps rewritten for the merged glyph order. A side-bearing or origin map is kept only when every font has one. When a font lacks the table, the result has none and its advances vary through `gvar`. |
| `GDEF`, `GPOS` | The `GDEF` variation stores are joined. Each font's variation indices in `GPOS` value records and anchors and in `GDEF` carets move to its part of the joined store. |
| `MVAR`, `STAT`, `cvar` | The first font's, as `OS/2`, `name` and `cvt ` are. A warning names a font whose `MVAR` differs. |

The test of these rules: instancing the merged font at a location gives the same glyphs, advances and
kerning as merging the inputs instanced at that location. With `--drop-tables fvar` the fonts merge as
their default outlines, as fontTools merges them. A static result has no `GDEF` variation store and no
variation-index devices: `GPOS` values, anchors and ligature carets keep their default values (fontTools
refuses fonts with `GDEF` stores). Not yet merged: feature variations in `GSUB` or
`GPOS` (refused), variable `COLR` and `VARC` (left out with a warning).

Engine command: `font.merge`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| mergefonts | output-font-file | -o |  |
| mergefonts | [glyph alias file] merge-font-file … | FONT --map FILE … |  |
| mergefonts | `-cid` | --cid-info |  |
| mergefonts | `-hints` | --hint-dict-from |  |
| mergefonts | `-g / -gx` | --glyphs / --exclude-glyphs after the first FONT |  |
| mergefonts | `-fd / -fdx` | --fd / --exclude-fd |  |
| mergefonts | glyph alias file header (FontName, LanguageGroup) | read from the map; --ps-name / --language-group override |  |
| mergefonts | tx output-mode options (-cff, -t1 …) | --to |  |
| mergefonts | `-u / -h / -v` | -h / --help / --version |  |
| pyftmerge | font1 … fontN | FONT... |  |
| pyftmerge | `--input-file` | --input-list |  |
| pyftmerge | `--output-file` | -o |  |
| pyftmerge | `--import-file` | --import-ttx |  |
| pyftmerge | `--drop-tables` | --drop-tables |  |
| pyftmerge | `--verbose` | -v |  |
| pyftmerge | `--timing` | --timing |  |

### `transform`

Applies one affine transformation to every glyph of a font (rotation, shift, scale, slant or an explicit matrix), or changes the units per em and scales the whole font to match. A per-glyph table can rename glyphs and set individual offsets and advance widths, which is how rotated copies of Latin glyphs are produced for vertical CJK text.

```
fontcraft-cli transform FONT [--rotate DEG] [--translate X,Y] [--scale SX[,SY]] [--slant DEG] [--matrix A,B,C,D,E,F] [--upem N] [--glyph-table FILE] [--widths auto|keep|transform|em] [-o OUT]
```

**Hints.** PostScript stem hints survive rotations by multiples of 90° (horizontal and vertical stems
swap at quarter turns), uniform or axis-aligned scaling and translation; any other transform removes
them with a warning. Alignment zones and standard stems are transformed when the hints are. TrueType
instructions are removed by any transform other than `--upem`.

**Order of operations** for the named options: scale, slant, rotate, translate. `--matrix` is for
anything else and is the PostScript matrix convention, not the font's FontMatrix.

**What `--upem` scales.** Outlines and components, advances and side bearings, vertical metrics in `hhea`,
`OS/2`, `vhea`, `post`, `BASE`; kerning and anchors in GPOS including device/variation deltas; `MATH`
constants; `gvar`, `cvar`, `HVAR`, `VVAR`, `MVAR` deltas; CFF private-dictionary zones and stems,
`cvt` values; COLR paint coordinates. Values are rounded to integers except in CFF2 and where the format
stores fractions. TrueType instructions are kept but rarely remain correct, so a warning recommends
re-hinting. `maxp` and `head.flags` bit 1 are recalculated as fontTools' `maxp.recalc` does, and
`post` is written again as scaleUpem writes it (see `instance` for the rule by version).

The single-glyph and selection forms of these operations are the editor command `path.transformations`;
`font.info.scaleToUnitsPerEm` is the document-level equivalent of `--upem`.

Engine command: `font.transform`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| rotatefont | `-rt degrees xOffset yOffset` | --rotate DEG --translate X,Y |  |
| rotatefont | `-matrix a b c d e f` | --matrix |  |
| rotatefont | `-fm` | --scale-em |  |
| rotatefont | `-rtf` | --glyph-table |  |
| rotatefont | `-row / -rtw / -rew` | --widths keep / transform / em |  |
| rotatefont | `-g / -gx / -fd / -fdx` | --glyphs / --exclude-glyphs / --fd / --exclude-fd |  |
| rotatefont | `-t1 / -cff and the other tx options` | --to and the convert options |  |
| rotatefont | `-u / -h / -v` | -h / --help / --version |  |
| ttLib.scaleUpem | font new-upem | transform FONT --upem N |  |
| ttLib.scaleUpem | `--output-file` | -o |  |

### `optimize`

Re-encodes parts of an existing binary font more compactly, or more simply for speed, without changing how it renders or shapes: CFF subroutines and operator forms, CFF default and nominal widths, class-kerning subtable layout, shared variation-store data, gvar delta optimisation, implied on-curve points and table order. Without options it applies every lossless size optimisation.

```
fontcraft-cli optimize FONT... [--for size|speed] [--subroutinize | --desubroutinize] [--cff-widths] [--charstrings specialize|generalize] [--gpos-compaction N] [--varstore] [--gvar-iup] [options] [-o OUT]
```

Every step is behaviour-preserving: rendering at any size and location and shaping of any text are the
same before and after, which is what the tests assert (see *Oracles*). The one exception is
`--varstore=QUANTUM` with a quantum above 1, which is lossy by design and says so.

`--report` makes the trade-offs visible before committing, for example that subroutinizing a small CFF
font can make the WOFF2 larger even though the OTF shrinks.

Class-kerning compaction exists because one big class-pair matrix is mostly zeros; splitting it into a few
denser matrices saves space at the cost of more subtables. Levels follow the fontTools setting of the
same name.

Engine command: `font.optimize`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| otlLib.optimize | font -o | optimize FONT -o OUT |  |
| otlLib.optimize | `--gpos-compression-level` | --gpos-compaction |  |
| otlLib.optimize | `-v / --verbose / -q / --quiet` | -v / -q |  |
| cffLib.width | FILE… | optimize --cff-widths |  |
| cffLib.width | `-b / --brute-force` | --cff-widths=exhaustive |  |
| cffLib.specializer | `--font FONTFILE, -o / --output-file` | optimize FONT --charstrings specialize -o |  |
| varLib.varStore | fontfile [outfile] | optimize --varstore |  |
| varLib.varStore | `--quantization` | --varstore=QUANTUM |  |
| tx | `+S` | --subroutinize |  |
| tx | `-S` | --desubroutinize |  |
| tx | `-maxs` | --max-subrs |  |
| tx | `+W` | --cff-widths |  |
| fontmake | `--optimize-cff 0\|1\|2` | --charstrings generalize \| --charstrings specialize \| --subroutinize |  |
| fontmake | `--no-optimize-gvar` | --no-gvar-iup |  |
| fontmake | `--drop-implied-oncurves` | --drop-implied-oncurves |  |
| varLib | `--disable-iup, --no-colr-layer-reuse, --drop-implied-oncurves` | --no-gvar-iup, --colr-layer-reuse, --drop-implied-oncurves (after the fact) |  |
| pyftsubset | `--desubroutinize` | --desubroutinize |  |
| pyftsubset | `--canonical-order` | --canonical-order |  |
| ttx | `--optimize-font-speed` | --for speed |  |
| ttLib | `--optimize-font-speed` | --for speed |  |

### `collection`

Group of actions on OpenType collections (.ttc, .otc): list the member fonts and the tables they share, build a collection from separate fonts, and split a collection into separate fonts. Without an action it lists.

```
fontcraft-cli collection list|pack|unpack FILE... [args]
```

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| otc2otf | `-r` | collection list |  |

#### `collection list`

Lists each member of a collection with its index, PostScript name, outline flavour and tables, marking the tables whose data is shared with another member. Writes nothing.

```
fontcraft-cli collection list FILE.ttc [--json]
```

Engine command: `collection.list`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| otc2otf | `-r / --report` | collection list |  |
| tx | (contents list shown for a TTC without -i) | collection list |  |

#### `collection pack`

Builds a collection from two or more fonts in the order given. Tables whose bytes are identical in several fonts are stored once. A table can also be forced to come from one font for all members.

```
fontcraft-cli collection pack FONT FONT... -o OUT.ttc [--share-table TAG=INDEX]... [--no-share]
```

Engine command: `collection.pack`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| otf2otc | `-o` | -o |  |
| otf2otc | `-t TAG=INDEX` | --share-table TAG=INDEX |  |
| otf2otc | input font files | FONT... |  |
| ttLib | several fonts with -o | collection pack |  |
| ttLib | `--flavor` | --container |  |

#### `collection unpack`

Writes each member of a collection as a separate font named after its PostScript name (or the collection name plus -fontN when it has none) with .ttf or .otf according to its outlines, or only the member chosen with --face. Shared tables are copied into every font that uses them.

```
fontcraft-cli collection unpack FILE.ttc [--face N|NAME] [--output-dir DIR]
```

Engine command: `collection.unpack`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| otc2otf | TTC FONT | collection unpack FILE.ttc |  |
| otc2otf | `-v / --verbose / --version` | -v / --version |  |
| ttLib | `-y N with -o` | collection unpack --face N -o OUT |  |
| tx | `-i N / -y` | --face N / (all members is the default) |  |

### `woff`

Group of two actions that wrap fonts in the web containers WOFF 1.0 and WOFF2 and unwrap them again. The font inside is not altered apart from the table transforms WOFF2 defines.

```
fontcraft-cli woff compress|decompress FONT... [options]
```

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| ttLib.woff2 | `-h / --help` | woff --help |  |

#### `woff compress`

Compresses a TrueType or OpenType font, or a collection, into WOFF2 (default) or WOFF 1.0. The default output name replaces the extension with .woff2 or .woff.

```
fontcraft-cli woff compress FONT... [--format woff2|woff] [--no-glyf-transform] [--hmtx-transform] [--metadata FILE] [--private-data FILE] [-o OUT]
```

Engine command: `woff.compress`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| ttLib.woff2 | compress INPUT | woff compress FONT |  |
| ttLib.woff2 | `-o / --output-file` | -o |  |
| ttLib.woff2 | `--no-glyf-transform` | --no-glyf-transform |  |
| ttLib.woff2 | `--hmtx-transform` | --hmtx-transform |  |
| ttLib.woff2 | `-v / --verbose / -q / --quiet` | -v / -q |  |
| ttx | `--flavor woff\|woff2` | woff compress --format (or --container on compile, convert, subset) |  |
| ttx | `--with-zopfli` | --zopfli |  |
| pyftsubset | `--flavor / --with-zopfli` | subset --container / --zopfli |  |

#### `woff decompress`

Unwraps a WOFF or WOFF2 file into the plain font or collection it contains, named with .ttf, .otf or .ttc according to the content. Metadata and private blocks can be saved separately.

```
fontcraft-cli woff decompress FONT.woff2|FONT.woff... [-o OUT]
```

Engine command: `woff.decompress`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| ttLib.woff2 | decompress INPUT | woff decompress FONT |  |
| ttLib.woff2 | `-o / --output-file` | -o |  |
| ttLib.woff2 | `-v / --verbose / -q / --quiet` | -v / -q |  |
| pyftsubset | `--flavor none` | subset --container plain |  |

A WOFF2 collection unwraps to a collection whose `head` tables keep bit 11 of `flags`, which the
encoder sets, and the `checkSumAdjustment` they had before. The directory checksums match those
bytes, so the `head` checksums differ from the collection that was compressed, and each
`checkSumAdjustment` no longer balances its member. A single font's adjustment is recomputed; a
collection's is kept as given, as the AFDKO collection tools keep it. Not yet checked against a
reference decoder.

### `diff`

Compares two fonts table by table and says which tables differ and how: presence, byte equality, the first differing offset, a hex diff, or a text diff of the decoded tables. Normalisation modes hide differences that do not change behaviour, for comparing two versions of a font or the output of two compilers. A and B may also be two folders, or a font and a folder, in which case fonts with the same file name are paired.

```
fontcraft-cli diff A B [--summary | --detail LEVEL] [-t TAG]... [-x TAG]... [--normalize none|dump|compiler] [--context N] [--output-dir DIR]
```

**Procedure.** (1) Pair the tables by tag and report tables present on one side only. (2) Compare bytes;
equal tables are done unless `--compare-identical`. (3) For the rest, according to `--detail`, report the
first differing offset, or diff hex dumps, or decode both tables to text (the `dump` text of the chosen
normalisation) and diff that. Tables the tool cannot decode fall back to the hex diff.

**Exit status.** 0 when no table differs after the chosen normalisation, 1 otherwise. `-q` prints
nothing.

**Normalisation modes.**

- `none`: tables are compared as stored. Two fonts that differ only in glyph order or lookup order
  will show large diffs. Right for "did this tool change anything".
- `dump`: the `ttxn` idea. Subroutines expanded, layout as feature text with member-derived class names,
  glyphs by name, checksums dropped; with `--no-version` and `--no-hints` also those. Right for "is the
  new release functionally the same as the old".
- `compiler`: the `ttx_diff` idea, described under *Oracles*. Adds dense `gvar` deltas, canonical
  ordering of order-free lists, name-ID indirection resolved to strings, contour order and start-point
  alignment where contours are otherwise equal, a small off-by-one budget, and compares kerning, mark
  attachment and ligature carets through `features normalize` instead of through table dumps. Right for
  "do two compilers agree on this source".

**JSON.** `{"tables": {"<tag>": {"a": bool, "b": bool, "equal": bool, "similarity"?: number,
"firstOffset"?: int, "sizeA": int, "sizeB": int}}, "identical": bool}`.

**Folders.** With two folders every file name found in both is compared and names found on one side only
are listed; with a font and a folder the font is compared with the file of the same name in the folder.

The editor's Compare Fonts window (`compareFonts.diff`) compares sources by design content; this
subcommand compares binaries by tables. They share nothing but the name.

Engine command: `font.diff`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| sfntdiff | fontfile1 fontfile2 \| fontfile dir \| dir1 dir2 | A B |  |
| sfntdiff | `-T` | --show-mtime |  |
| sfntdiff | `-d 0` | --detail tables |  |
| sfntdiff | `-d 1` | --detail offset |  |
| sfntdiff | `-d 2` | --detail hex |  |
| sfntdiff | `-d 3` | --detail fields |  |
| sfntdiff | `-d 4` | --detail verbose |  |
| sfntdiff | `-x` | -x |  |
| sfntdiff | `-i` | -t |  |
| sfntdiff | `-u / -h` | -h / --help |  |
| sfntdiff | `-X` | run --script |  |
| fonttools diff | FILE1 FILE2 | A B |  |
| fonttools diff | `-l / --summary` | --summary |  |
| fonttools diff | `-U / --lines` | --context |  |
| fonttools diff | `-t / --include` | -t |  |
| fonttools diff | `-x / --exclude` | -x |  |
| fonttools diff | `--diff` | --tool |  |
| fonttools diff | `--diff-arg` | --tool-args |  |
| fonttools diff | `--color` | --color |  |
| fonttools diff | `--y1 / --y2` | --face-a / --face-b |  |
| fonttools diff | `-a / --always` | --compare-identical |  |
| fonttools diff | `-b / --binary` | default: decode only tables whose bytes differ |  |
| fonttools diff | `-q / --quiet` | -q (exit status only) |  |
| ttxn | (run on two fonts, then diff the per-table files) | --normalize dump |  |
| ttxn | `-nh / -nv / -se` | --no-hints / --no-version / --show-extensions |  |
| ttx_diff | `--fontc_font A --fontmake_font B` | A B --normalize compiler |  |
| ttx_diff | `--off_by_one_budget` | --off-by-one-budget |  |
| ttx_diff | `--unwrap_extensions` | --unwrap-extensions |  |
| ttx_diff | `--json` | --json (with --similarity semantics per table) |  |
| ttx_diff | `--outdir` | --output-dir |  |

### `sources`

Two actions on source files (`.glyphs`, `.glyphspackage`, `.fontcraft`, UFO, designspace), after
the reference vendor's free source-comparison tool. Sources of different formats can be compared.

```
fontcraft-cli sources diff A B [--parts LIST] [--glyphs LIST] [--tolerance UNITS] [--format text|json]
fontcraft-cli sources merge BASE OTHER (--take ID,... | --take-all) [--parts LIST] [--glyphs LIST] (-o OUT | --in-place)
```

- Parts: `glyphs`, `info` (Font Info fields), `masters`, `instances`, `axes`, `kerning` (pairs and
  groups), `features` (feature code, classes, prefixes).
- Glyphs match by name; masters and instances by axis location, then by name. A glyph whose
  outlines and code points are unchanged under a new name is reported as renamed.
- Per glyph and master, `diff` reports outlines (with `--tolerance`), components, anchors,
  metrics, code points and glyph properties; for the other parts, each field with both values.
- Every change has an id. `merge` applies the chosen ids from OTHER to BASE and writes BASE's
  format. A change whose place in BASE no longer matches what `diff` saw refuses the whole merge.
  The merge is two-way: there is no common ancestor.
- Exit status of `diff`: 1 when the sources differ.

Engine commands: `source.diff` (the glyph part is `font.compareGlyphs`) and `source.merge`. The
window that does the same for the open document (`window.sourceDiff`,
`document.compareWithFile`, `document.mergeFromFile`) is M13.

### `outlines`

Group of whole-font outline repairs and conversions that work on sources and on binary fonts alike: merge overlapping contours, set contour direction, add extreme points, round coordinates, convert between cubic and quadratic curves, and turn repeated shapes into components or components back into contours. Finding problems without changing anything is 'check outlines'.

```
fontcraft-cli outlines remove-overlaps|direction|extremes|round|to-quadratic|to-cubic|componentize|decompose INPUT... [args]
```

Each action applies, glyph by glyph, the same geometry as the editor command named in the table and
therefore gives the same result as doing it by hand in the editor. The CLI adds what a batch run needs:
glyph selection, a choice of layer for UFOs, tolerance of failures, and for binary fonts a rewrite that
touches only the outline table and what depends on it (bounding boxes, `maxp`, `hmtx` side bearings,
hint data of changed glyphs).

| Action | Engine command | Editor counterpart |
|---|---|---|
| `remove-overlaps` | `outlines.removeOverlaps` | `path.removeOverlap` |
| `direction` | `outlines.correctDirection` | `path.correctPathDirection` |
| `extremes` | `outlines.addExtremes` | `path.addExtremes` |
| `round` | `outlines.roundCoordinates` | Path › Round Coordinates |
| `to-quadratic`, `to-cubic` | `outlines.convertCurves` | none (export does this implicitly) |
| `componentize` | `outlines.componentize` | none |
| `decompose` | `outlines.decompose` | `glyph.decomposeGlyph` |

Variable fonts: `remove-overlaps`, `extremes` and `decompose` change point counts and are refused for a
font with `gvar` or CFF2 blends unless every master can be processed identically; the error suggests
doing it in the sources.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| checkoutlinesufo | `-e / --error-correction-mode` | check outlines --fix (all repairs) or the single actions here |  |

#### `outlines remove-overlaps`

Merges overlapping contours and overlapping components of each glyph into a single non-overlapping outline. Composite glyphs whose components overlap are decomposed first. In a binary font, hints of glyphs that changed are dropped unless told otherwise.

```
fontcraft-cli outlines remove-overlaps INPUT [GLYPH...] [--keep-hinting] [--ignore-errors] [--keep-unused-subroutines] [-o OUT]
```

Engine command: `outlines.removeOverlaps`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| ttLib.removeOverlaps | INPUT.ttf OUTPUT.ttf [GLYPHS…] | outlines remove-overlaps INPUT [GLYPH...] -o OUTPUT |  |
| ttLib.removeOverlaps | `--keep-hinting` | --keep-hinting |  |
| ttLib.removeOverlaps | `--ignore-errors` | --ignore-errors |  |
| ttLib.removeOverlaps | `--keep-unused-subroutines` | --keep-unused-subroutines |  |
| tx | `+V` | outlines remove-overlaps (or convert --remove-overlaps) |  |
| checkoutlinesufo | -e (overlap part) | outlines remove-overlaps |  |
| checkoutlinesufo | `--ignore-contour-order` | --no-keep-contour-order |  |
| checkoutlinesufo | `--min-area` | --min-area |  |
| fontmake | --keep-overlaps (inverse) | compile --remove-overlaps / outlines remove-overlaps |  |

#### `outlines direction`

Sets the winding of every contour from how contours nest: outer contours counter-clockwise and inner ones clockwise for PostScript outlines, the opposite for TrueType, or simply reverses every contour.

```
fontcraft-cli outlines direction INPUT... [--flavour postscript|truetype|reverse] [-o OUT]
```

Engine command: `outlines.correctDirection`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| fontmake | `--keep-direction` | compile --keep-direction; outlines direction to fix a font afterwards |  |
| cu2qu | `--keep-direction` | outlines to-quadratic --keep-direction |  |

#### `outlines extremes`

Inserts on-curve points where curves reach a horizontal or vertical extreme, without changing the shape, skipping extremes that would leave a segment shorter than the threshold.

```
fontcraft-cli outlines extremes INPUT... [--min-segment N] [-o OUT]
```

Engine command: `outlines.addExtremes`.

#### `outlines round`

Rounds point coordinates, component offsets, anchors and advance widths to integers or to a coarser grid. The reference tools round by default and offer options to keep decimals; this action applies the rounding as a separate step.

```
fontcraft-cli outlines round INPUT... [--grid N] [-o OUT]
```

Engine command: `outlines.roundCoordinates`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| fontmake | `--round-instances` | instance --round (or outlines round afterwards) |  |
| fontmake | `--cff-round-tolerance` | compile --cff-round-tolerance; outlines round --tolerance |  |

#### `outlines to-quadratic`

Converts cubic curves to quadratic splines within an error bound, in sources (UFO in place or to a new UFO) or in a font. With --interpolatable several masters are converted together so that every master gets the same number of points per segment.

```
fontcraft-cli outlines to-quadratic INPUT... [--max-error EM] [--mixed] [--interpolatable] [--keep-direction] [-o OUT | --output-dir DIR]
```

Engine command: `outlines.convertCurves`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| cu2qu | INPUT… | outlines to-quadratic INPUT... |  |
| cu2qu | `-e / --conversion-error` | --max-error |  |
| cu2qu | `-m / --mixed` | --mixed |  |
| cu2qu | `--keep-direction` | --keep-direction |  |
| cu2qu | `-i / --interpolatable` | --interpolatable |  |
| cu2qu | `-j / --jobs` | -j / --jobs |  |
| cu2qu | `-o / --output-file` | -o |  |
| cu2qu | `-d / --output-dir` | --output-dir |  |
| cu2qu | `-v / --verbose / --version` | -v / --version |  |
| fontmake | `--ttf-curves / -e` | compile --ttf-curves / --conversion-error |  |

#### `outlines to-cubic`

Converts quadratic splines of a TrueType font to cubic curves where one cubic can replace several quadratic segments within the error bound, writing a glyf format 1 font with mixed curves; with --all-cubic every curve becomes cubic.

```
fontcraft-cli outlines to-cubic INPUT... [--max-error EM] [--all-cubic] [-o OUT | --output-dir DIR]
```

Engine command: `outlines.convertCurves`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| qu2cu | INPUT… | outlines to-cubic INPUT... |  |
| qu2cu | `-e / --conversion-error` | --max-error |  |
| qu2cu | `-c / --all-cubic` | --all-cubic |  |
| qu2cu | `-o / --output-file` | -o |  |
| qu2cu | `-d / --output-dir` | --output-dir |  |
| qu2cu | `-v / --verbose / --version` | -v / --version |  |
| tx | -x (exact quadratic to cubic) | convert --curve-conversion exact |  |

#### `outlines componentize`

Replaces contours of TrueType glyphs with component references. With a reference source, each glyph that the source builds purely from components that are only shifted gets the same components in the font, after a check (our addition) that they reproduce its points; the first component is flagged to supply the metrics when its advance equals the composite's. With --detect, glyphs whose outline is exactly the union of other glyphs' outlines at integer offsets are found without a source.

```
fontcraft-cli outlines componentize FONT.ttf [--reference UFO | --detect] [-o OUT]
```

Engine command: `outlines.componentize`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| ttfcomponentizer | FONT | outlines componentize FONT |  |
| ttfcomponentizer | `-o` | -o |  |
| ttfcomponentizer | (UFO found in the font's folder; names through public.postscriptNames or a GOADB) | --reference / --goadb |  |
| ttfcomponentizer | `--version` | --version |  |

#### `outlines decompose`

Replaces component references with the contours they stand for. In a TrueType font the decomposed glyphs lose their instructions. Options restrict the action to nested components (flattening to one level) or to components with a transform.

```
fontcraft-cli outlines decompose INPUT... [--nested-only] [--transformed-only] [-o OUT | --output-dir DIR]
```

Engine command: `outlines.decompose`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| ttfdecomponentizer | INPUT | outlines decompose INPUT |  |
| ttfdecomponentizer | `-d / --directory` | a folder as INPUT |  |
| ttfdecomponentizer | `-o` | -o / --output-dir |  |
| ttfdecomponentizer | `-v / --verbose` | --list |  |
| ttfdecomponentizer | `--version` | --version |  |
| fontmake | `-f / --flatten-components` | --nested-only |  |
| fontc | `--decompose-components / --decompose-transformed-components / --flatten-components` | (all) / --transformed-only / --nested-only |  |

### `autohint`

Adds hints automatically. For CFF, CFF2, Type 1 and cubic UFO input it runs the PostScript autohinter, which derives stem hints, hint masks, counter hints and flex from each glyph and the font's alignment zones and standard stems. For TrueType fonts it runs the TrueType autohinter with the given option string. Several masters can be hinted together so that their hints stay compatible.

```
fontcraft-cli autohint INPUT... [-o OUT...] [--glyphs LIST] [--all] [--reference FONT] [--allow-changes] [--report-only] [options]
```

The hinting algorithm, its inputs and each option's effect are specified with the PostScript hinting
inventory (`autohint.ps.*` rows); this section only fixes the command-line surface.

**Which glyphs.** All glyphs are candidates, but a glyph that was hinted before and whose outline has not
changed since is skipped, so that manual hinting survives a second run; `--all` re-hints everything and
replaces existing hints. The reference help ties this to UFO input (it compares stored outline
fingerprints) while its how-to describes it for Type 1 input too; what happens for binary fonts must be
observed (open question 5).

**Output.** The reference hints its inputs in place unless `-o` is given; here `--in-place` must be said
explicitly. `-o` takes as many paths as there are inputs. `--report-only` cannot be combined with
`--allow-changes`.

**Global data.** Results depend on the font's alignment zones and standard stems. Without them the run
stops unless `--no-zones-stems`. `stems` produces the reports from which to choose them.

**fontinfo overrides.** A file with `begin FDDict NAME … end FDDict NAME` blocks (zone pairs such as
`BaselineYCoord`/`BaselineOvershoot`, `CapHeight`/`CapOvershoot`, `LcHeight`/`LcOvershoot`, the stem
lists `DominantV` and `DominantH`, `FlexOK`, counter-hint glyph lists) and matching `begin GlyphSet NAME`
blocks listing glyph names or regular expressions assigns different zones and stems to groups of glyphs.
When any are defined the font's own global values are ignored, and glyphs matching no set are hinted
with zones placed outside the font's bounding box. A definition must precede the glyph set that uses it;
two glyph sets may not share a name; at least the baseline pair, one top zone pair and both stem lists are
required.

**Several masters.** `--reference` hints the reference first and then transfers its hint structure to
the other inputs so that all masters have the same stems in the same order, as CFF2 requires. A built
CFF2 variable font can be hinted directly; stems whose order changes between masters are removed unless
`--keep-conflicts`.

**TrueType.** For a TrueType font the command dispatches `autohint.tt.run`; only `-o`, `--glyphs`,
`--tt-options` and `--tt-control` apply.

Engine commands: `autohint.ps.run`, `autohint.ps.listFdDicts`, `autohint.tt.run` (defined with the
hinting inventories).

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| otfautohint | FONT… | INPUT... |  |
| otfautohint | `-o / --output` | -o |  |
| otfautohint | `-g / --glyphs, --glyphs-file` | --glyphs / --glyphs-file |  |
| otfautohint | `-x / --exclude-glyphs, --exclude-glyphs-file` | --exclude-glyphs / --exclude-glyphs-file |  |
| otfautohint | `-m / --overlap-glyphs, --overlaps-file` | --overlap-glyphs / --overlaps-file |  |
| otfautohint | `--force-overlap / --force-no-overlap` | same |  |
| otfautohint | `--loose-overlap-mapping` | --loose-overlap-mapping |  |
| otfautohint | `-r / --reference-font` | --reference |  |
| otfautohint | `-b / --reference-out` | --reference-out |  |
| otfautohint | `-a / --all` | --all |  |
| otfautohint | `-w / --write-to-default-layer` | (default) | the reference default, a separate processed layer, is --write-layer processed |
| otfautohint | `-k / --keep-conflicts` | --keep-conflicts |  |
| otfautohint | `-c / --allow-changes` | --allow-changes |  |
| otfautohint | `--report-only` | --report-only |  |
| otfautohint | `-d / --decimal` | --decimal |  |
| otfautohint | `--no-flex` | --no-flex |  |
| otfautohint | `--no-hint-sub` | --no-hint-sub |  |
| otfautohint | `--no-zones-stems` | --no-zones-stems |  |
| otfautohint | `--fontinfo-file / --ignore-fontinfo` | same |  |
| otfautohint | `--print-list-fddict / --print-all-fddict` | same |  |
| otfautohint | `--doc-fontinfo / --info` | --help (the fontinfo format is part of the help text) |  |
| otfautohint | `--max-segments` | --max-segments |  |
| otfautohint | `--log` | --log-file |  |
| otfautohint | `-p / --processes` | -j / --jobs |  |
| otfautohint | `-v / --verbose` | -v / -vv (near misses of zones and stems are reported at -v) |  |
| otfautohint | `--traceback` | --backtrace |  |
| otfautohint | `--version` | --version |  |
| fontmake | `-a / --autohint [ARGS]` | compile --autohint[=ARGS]; autohint FONT.ttf --tt-options ARGS |  |
| fontmake | `-A / --no-autohint` | compile --no-autohint |  |

### `stems`

Reports the stem widths found in a font's glyphs, most frequent first, as the basis for choosing standard stem values; with --zones it reports the heights of glyph tops and bottoms instead, as the basis for choosing alignment zones. The font is not changed.

```
fontcraft-cli stems INPUT... [--zones] [--all-stems] [--glyphs LIST] [-o BASEPATH]
```

Each report lists a value (a stem width, or a y coordinate), the number of glyphs in which it occurs and
the glyph names, sorted by count. `--json` returns the same as `{horizontal: [...], vertical: [...]}` or
`{top: [...], bottom: [...]}`. The measurement runs with flex off and does not need zones or stems to
exist already.

Engine commands: `autohint.ps.reportStems`, `autohint.ps.reportZones` (with `--zones`).

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| otfstemhist | FONT… | INPUT... |  |
| otfstemhist | `-o / --output` | -o |  |
| otfstemhist | `-z / --alignment-zones` | --zones |  |
| otfstemhist | `-a / --all` | --all-stems |  |
| otfstemhist | `-g / --glyphs, --glyphs-file, -x / --exclude-glyphs, --exclude-glyphs-file` | --glyphs / --glyphs-file / --exclude-glyphs / --exclude-glyphs-file |  |
| otfstemhist | `-m / --overlap-glyphs, --overlaps-file, --force-overlap, --force-no-overlap, --loose-overlap-mapping` | same |  |
| otfstemhist | `--log` | --log-file |  |
| otfstemhist | `-p / --processes` | -j / --jobs |  |
| otfstemhist | `-v / --verbose, --traceback, --version` | -v / --backtrace / --version |  |

### `dehint`

Removes hinting. From CFF outlines it removes stem hints, hint masks and flex; from TrueType glyphs their instructions; and unless told otherwise the font-wide hinting data as well. Hint-related leftovers in other tables are cleaned up. In a source it clears manual and stored automatic hints.

```
fontcraft-cli dehint INPUT... [--glyphs LIST] [--keep-font-hints] [--hinting-tables TAG,...] [-o OUT]
```

For TrueType fonts removing instructions also resets what depends on them. As pyftsubset does: the
`maxp` limits for zones, twilight points, storage, function and instruction definitions, stack depth and
instruction size are reset, and device tables and contour-point anchors in GPOS are removed. Our
additions: `head` flags that announce instruction-dependent metrics are cleared and `LTSH` is dropped.

For rehinting, run `dehint` and then `autohint`, or simply `autohint --all`, which replaces existing
hints glyph by glyph.

Engine command: `hints.remove`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| tx | `-n` | dehint (or convert --no-hints) |  |
| ttxn | `-nh` | dump --normalize --no-hints (comparison only) |  |
| pyftsubset | `--no-hinting` | dehint (or subset --no-hinting) |  |
| pyftsubset | `--hinting-tables` | --hinting-tables |  |

### `check`

Runs the quality checks of a profile on sources or binary fonts and prints the findings; several files or a folder are also checked as a family. Three action words give the classic single-purpose audits with the option sets of their reference tools: outlines, family and interpolation. Nothing is changed unless --fix is given. Exit status 1 when a finding reaches the --fail-on severity.

```
fontcraft-cli check [outlines|family|interpolation] INPUT... [--profile ID] [--check ID]... [--exclude ID]... [--fix] [--report PATH] [options]
```

The check registry, the profiles, the result document and the repairs are specified in
`docs/spec/21-qa.md`; that document also fixes the option-to-parameter mapping reproduced in the
option list of this row. In short: the run dispatches `audit.run`; `--fix` and `--fix-unsafe` follow it
with `audit.fix` (`safeOnly` true or false; `--dry-run` reports the planned edits; `-o` writes a fixed
copy); `--report` uses `audit.export`; `--list`, `--describe` and `--profiles` dispatch `audit.list`,
`audit.describe` and `audit.profile.list`.

Text output lists findings sorted by severity, then group, check, glyph order and master, and ends with
counts per severity. `-q` prints nothing.

**Exit status.** 0 when no finding reaches `--fail-on`, 1 when one does. The QA document assigns 2 to
both usage errors and unreadable targets; the suite-wide scheme of this document uses 2 for usage and 3
for unreadable input. This needs one decision (open question 20).

Engine commands: `audit.run`, `audit.fix`, `audit.export`, `audit.list`, `audit.describe`,
`audit.profile.list` (all defined in the QA inventory).

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| comparefamily | single-face tests run on one font | check FONT --profile strict (the numbered tests map to check ids; see check family) |  |
| tx | (conversion to CFF reports malformed data and unhinted glyphs) | check FONT --profile opentype |  |
| makeotf | `-shw / -nshw` | check --exclude or --check on the missing-stem-hints check |  |

#### `check outlines`

Examines glyph outlines for overlapping contours, coincident points, collinear line segments, curves that are really straight lines, contours too small to matter and glyphs without contours, in a source or a binary font. With --fix it repairs what it reports. This is the classic AFDKO outline check with the same options and defaults.

```
fontcraft-cli check outlines INPUT... [--fix] [--no-overlap-checks] [--no-basic-checks] [--min-area N] [--tolerance N] [--glyphs LIST] [--all] [-o OUT]
```

Tests, their order, the repair procedure and the hash cache are specified in `docs/spec/21-qa.md`
(section on checkoutlinesufo). Points that matter on the command line:

- Components are decomposed in a temporary copy before testing, so a composite is judged as it will be
  exported.
- For a binary font `--fix` needs `-o` or `--in-place`; only the outline table and what depends on it are
  rewritten.
- AFDKO pipelines expect the repaired outlines of a UFO in the layer `com.adobe.type.processedglyphs`
  and the originals untouched. Our default is the other way round. Whether a `--write-layer processed`
  mode is needed for drop-in compatibility is open (QA open question 3; `compile --layer processed`
  already reads such a layer).

Engine commands: `audit.checkOutlines`, `audit.clearCache`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| checkoutlinesufo | font_path | check outlines INPUT |  |
| checkoutlinesufo | `-e / --error-correction-mode` | --fix |  |
| checkoutlinesufo | `--no-overlap-checks` | --no-overlap-checks |  |
| checkoutlinesufo | `--no-basic-checks` | --no-basic-checks |  |
| checkoutlinesufo | `--min-area` | --min-area |  |
| checkoutlinesufo | `--tolerance` | --tolerance |  |
| checkoutlinesufo | `-g / --glyph-list` | --glyphs |  |
| checkoutlinesufo | `-f / --glyph-file` | --glyphs-file |  |
| checkoutlinesufo | `-d / --decimal` | --decimal |  |
| checkoutlinesufo | `--ignore-contour-order` | --no-keep-contour-order |  |
| checkoutlinesufo | `-o / --output-file` | -o |  |
| checkoutlinesufo | `-w / --write-to-default-layer` | --keep-original none | our default already writes the fixed outline to the glyph itself and keeps the original in a backup layer; the reference default is the reverse (fixed copy in a separate processed-glyphs layer) |
| checkoutlinesufo | `--clear-hash-map` | --clear-cache |  |
| checkoutlinesufo | `--all` | --all |  |
| checkoutlinesufo | `-q / --quiet-mode` | -q |  |
| checkoutlinesufo | `--version` | --version |  |

#### `check family`

Examines all OpenType fonts in a folder, or the fonts listed, as families and reports problems within each font and inconsistencies between them: naming and style linking, weight and width classes, vertical metrics, underline, embedding permissions, hint zones, script and feature coverage. Optional reports tabulate menu names, global metrics, PANOSE values and feature coverage side by side.

```
fontcraft-cli check family DIR|FONT... [--tolerance N] [--menu-names] [--metrics] [--panose] [--layout] [--single LIST] [--family LIST] [--no-hints]
```

Fonts are grouped twice, as the reference does: by name ID 1 into style-linked groups of at most four
(Regular, Bold, Italic, Bold Italic) and by name ID 16 (falling back to ID 1) into typographic families.
The numbered tests of the reference and the checks they map to are tabulated in `docs/spec/21-qa.md`.

Topics of the 34 single-face tests: length limits and uniqueness of menu names; agreement of full and
PostScript names between `name` and CFF; version string against `head`; presence of copyright, trademark
and vendor data; deprecated CFF operators; style name against style bits; weight class not below 250 and
bold not below 500; BASE table presence and plausibility; italic angle against the italic bit;
fixed-pitch flag against actual widths; bold and italic bits agreeing between `head` and `OS/2`; font
bounding box; ascender and descender against the em; equal widths of tabular glyphs; at least one hint
per glyph and charstring size; Unicode cmap present without double mappings; stray spaces in menu names;
ligature and accented-glyph widths; `size` feature ranges; unique-ID entries; glyph-name rules;
strikeout and sub/superscript positions; code-page bits; number, order and spacing of alignment zones.

Topics of the 21 family tests: group size; identical names across languages; same typographic family
name; family-wide `size` data; unique style per group; a base font exists; same copyright and trademark;
weight classes per style group; same width class; PANOSE presence and fixed-pitch agreement; Mac and
Windows menu names; same scripts, languages and features with `DFLT` and `latn` present; unique
weight/width/slope when fsSelection bit 8 is set; same embedding permissions; same underline; glyph
widths not wildly different between faces; `OS/2` version; alignment zones consistent across the family.

Style-linking guidance behind the weight tests (Windows weight notes): a regular face may use any weight
class from 250 up, and its bold partner must fall in a range that depends on the regular's value, for
example 600 to 850 for a regular of 400.

Engine commands: `audit.compareFamily`, and for the four reports `audit.report.menuNames`,
`audit.report.metrics`, `audit.report.panose`, `audit.report.featureLanguages`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| comparefamily | `-d` | DIR |  |
| comparefamily | `-tolerance` | --tolerance |  |
| comparefamily | `-rm` | --menu-names |  |
| comparefamily | `-rn` | --metrics |  |
| comparefamily | `-rp` | --panose |  |
| comparefamily | `-rf` | --layout |  |
| comparefamily | `-nohints` | --no-hints |  |
| comparefamily | `-l` | --log-file |  |
| comparefamily | `-st` | --single |  |
| comparefamily | `-ft` | --family |  |
| comparefamily | `-u / -h` | -h / --help |  |
| comparefamily | CF_DEFAULT_URL / CF_DEFAULT_FOUNDRY_CODE | --expect-url / --expect-vendor |  |

#### `check interpolation`

Tests whether the masters of a design interpolate cleanly: same glyphs, closed contours, same number of contours and points with the same curve types, contours in the same order, the same start point on each contour, and no kinks or weight anomalies appearing between masters. Input is one variable font, designspace or multi-master source, or several fonts or UFOs that are the masters.

```
fontcraft-cli check interpolation INPUT... [--glyphs LIST] [--tolerance F] [--kinkiness F] [--show-all] [--ignore-missing] [--master NAME]... [--report PATH]
```

Problem types, in the reference's order of severity: missing glyph, open path, path count, node count,
node type, contour order, wrong start point (which also reveals a reversed contour), kink, underweight,
overweight. Each master is compared with its neighbour toward the default master. The checks behind
each type and their thresholds are specified in `docs/spec/21-qa.md`.

Exit status 1 when any problem is found.

The editor query `interpolation.checkCompatibility` reports the first structural mismatch per layer pair
of an open document; this audit reports every mismatch and adds the geometric tests, and works on files
of any supported kind.

Engine command: `audit.checkInterpolation`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| varLib.interpolatable | FILE… | INPUT... |  |
| varLib.interpolatable | `--glyphs` | --glyphs |  |
| varLib.interpolatable | `--show-all` | --show-all |  |
| varLib.interpolatable | `--tolerance` | --tolerance |  |
| varLib.interpolatable | `--kinkiness` | --kinkiness |  |
| varLib.interpolatable | `--json` | --json |  |
| varLib.interpolatable | `--pdf / --html` | --report PATH | findings as a report; the illustrated side-by-side pages of the reference are not designed yet |
| varLib.interpolatable | `--quiet` | -q (exit status only) |  |
| varLib.interpolatable | `--output` | -o |  |
| varLib.interpolatable | `--ignore-missing` | --ignore-missing |  |
| varLib.interpolatable | `--name` | --master |  |
| varLib.interpolatable | `-v / --verbose / --debug` | -v / -vv |  |
| buildcff2vf | `-c / --check-compat` | check interpolation (compile --check-compatibility=fix to repair flat curves) |  |
| fontmake | `--check-compatibility` | check interpolation (or compile --check-compatibility) |  |

### `proof`

Draws proof sheets of a font's glyphs as PDF, SVG or HTML: overview palettes, one-glyph pages with points, coordinates and metrics, hint and alignment-zone overlays, size waterfalls, side-by-side comparison of several fonts, and sheets that show what layout features and the cmap do. Input may be a binary font, a Type 1 font or a source.

```
fontcraft-cli proof FONT... [--layout NAME] [--format pdf|svg|html] [--glyphs LIST] [--size PT | --glyphs-per-page N] [-o OUT] [options]
```

**Layouts and their reference presets.**

| Layout | Per page | Shows | Reference |
|---|---|---|---|
| `palette` | 156 by default | filled glyphs with name and a few metrics | `fontplot`, `tx -pdf` (which fits 320) |
| `boxes` | 156 | filled glyphs, a box of em height and advance width, centre registration marks | `fontplot2` |
| `outline` | 1 | outline with on-curve points, handles, point coordinates, all metric values, small filled preview | `charplot`, `tx -pdf -1` |
| `digi` | 1 glyph, two copies | left: in the em box with notes from an error log; right: outline with point-type labels | `digiplot` |
| `hints` | 4 | outline with horizontal and vertical stem hints and alignment zones, name and side bearings | `hintplot` |
| `waterfall` | groups of glyphs | the same line at each size, font embedded and referenced by character code so that the viewer's rasteriser and hinting are what is seen | `waterfallplot` |
| `compare` | rows per font | glyphs in index order at one-em spacing so the same index lines up across fonts; fonts sorted by glyph-set size, then glyph names, then PostScript name | `fontsetplot` |
| `synopsis` | many | index, name and shape in a dense table | `spot -G` |
| `features` | varies | for GSUB each rule's input and output glyphs; for GPOS the glyphs with the adjustment applied and its value | `spot -P` |
| `cmap` | many | each code point with its glyph | `spot -t cmap=9` |
| `vorg` | many | glyph in the em box with vertical origin and advance | `spot -t VORG=8` |
| `text` | 1 | a string laid out with the font's advances, as paths | fontTools `svgPathPen` |

All layouts except `waterfall` draw outlines as vector paths and do not embed the font, so they look the
same in every viewer. `waterfall` embeds the font on purpose in SVG and HTML. In PDF it draws the text as
outlines too: each character's glyph comes from `cmap`, its advance from the font at the proof's location,
and each pair of neighbours gets its `GPOS` (or `kern` table) pair kerning.

A PDF proof is one file holding every page, each at its own size. Labels are set in the standard
Helvetica font. Each glyph a cell or page is about sits in a `/Glyph` marked-content sequence with its
name and glyph id, so tools can read the glyph order back from the file.

**Hint overlays.** Successive stems are drawn slightly offset so that overlapping stems can be told
apart; areas where two stems overlap get their own colour. `--hint-groups` limits the overlay to chosen
hint-substitution groups.

**Low-level parameters.** The reference script exposes nearly eighty drawing parameters (page title font
and size, tile paddings, colours of zones, em box, hints and labels, label fonts and sizes, metadata block
placement, error-log columns). They are all available under their reference names through `--set`, and
`--list-settings` documents them; the frequently used ones have the dedicated options above.

**Glyph ranges.** In this subcommand `zero-nine` style name ranges are accepted as well as `zero..nine`
and are resolved by glyph index, as in the reference.

Engine command: `font.proof`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| proofpdf | -charplot (charplot) | --layout outline |  |
| proofpdf | -digiplot (digiplot) | --layout digi |  |
| proofpdf | -fontplot (fontplot) | --layout palette |  |
| proofpdf | -fontplot2 (fontplot2) | --layout boxes |  |
| proofpdf | -fontsetplot (fontsetplot) | --layout compare |  |
| proofpdf | -hintplot (hintplot) | --layout hints |  |
| proofpdf | -waterfallplot (waterfallplot) | --layout waterfall |  |
| proofpdf | `-wfr` | --sizes |  |
| proofpdf | `-repeatIndex` | --panel |  |
| proofpdf | `-alpha` | --sort name |  |
| proofpdf | `-g / -gf` | --glyphs / --glyphs-file |  |
| proofpdf | `-gpp` | --glyphs-per-page |  |
| proofpdf | `-pt` | --size |  |
| proofpdf | `-o` | -o |  |
| proofpdf | `-dno / -do` | (default) / --open |  |
| proofpdf | `-baseline` | --baseline |  |
| proofpdf | `-black` | --black |  |
| proofpdf | `-lf` | --cid-layout |  |
| proofpdf | `-select_hints` | --hint-groups |  |
| proofpdf | `-v / -vh` | --metrics v / --metrics both |  |
| proofpdf | `-q` | -q |  |
| proofpdf | `-u / -h` | -h / --help |  |
| proofpdf | `-help_params` | --list-settings |  |
| proofpdf | --<name> <value> (low-level parameters) | --set NAME=VALUE, same parameter names; the common ones also through --show/--hide, --page-size, --margins, --point-labels, --error-log |  |
| tx | `-pdf -0` | --layout palette |  |
| tx | `-pdf -1` | --layout palette+outline |  |
| tx | `-ps -0 / -1` | --layout palette / outline (as PDF) |  |
| tx | `-ps -e` | --sort unicode | glyphs ordered by code; the 16x16 grid of 8-bit encodings is not reproduced |
| tx | `-ps -l` | --hide point-labels |  |
| tx | `-g / -gx / -fd / -U` | --glyphs / --exclude-glyphs / --glyphs with fd selection / --location |  |
| spot | `-t CFF_=6, -t glyf=6` | --layout outline |  |
| spot | `-t CFF_=7 / =8, -t glyf=7 / =8, -G` | --layout synopsis |  |
| spot | `-br` | --layout synopsis --glyphs-per-page 1 |  |
| spot | `-t cmap=9 / =10` | --layout cmap |  |
| spot | `-t GPOS=8, -t GSUB=8, -P TAGS, -Proof` | --layout features --features TAGS\|* |  |
| spot | `-A` | --layout features (undocumented alias of -P in the reference) |  |
| spot | `-t VORG=8` | --layout vorg |  |
| spot | `-f` | --sort lookup |  |
| spot | `-V, -p6` | --vertical |  |
| spot | `-d` | --no-header |  |
| spot | -p N (proof policies, written -p1, -p2 …) | --show / --hide / --vertical / --no-palt-in-kern as listed in the next rows |  |
| spot | `-p1` | --hide name |  |
| spot | `-p2` | --hide numeric-labels |  |
| spot | `-p4` | --show em-box |  |
| spot | `-p5` | --show bbox |  |
| spot | `-p7` | --no-palt-in-kern |  |
| spot | `-@` | --size |  |
| spot | `-a` | --hide point-labels,contour-labels |  |
| spot | `-c` | --units 1000 |  |
| spot | `-g` | --glyphs |  |
| spot | `-b` | --fit-bbox |  |
| spot | `-s` | --scale |  |
| pens.svgPathPen | font [text] | --layout text --format svg --text STRING |  |
| pens.svgPathPen | `--glyphs` | --glyphs |  |
| pens.svgPathPen | `-y` | --face |  |
| pens.svgPathPen | `--variations` | --location |  |

### `features`

Group of actions on OpenType layout: list scripts, languages and features; compile feature code into a font; turn a font's layout tables back into feature code; check feature code without building; convert other layout source syntaxes; and print layout rules in a normalised form for comparison. Without an action it lists.

```
fontcraft-cli features list|compile|decompile|lint|convert|normalize INPUT [args]
```

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| spot | `-F` | features list |  |

#### `features list`

Lists, for GSUB and GPOS, every script and language system with its features, and optionally the lookups each feature uses with their types. Shows at a glance whether a feature is registered for all the language systems it should be.

```
fontcraft-cli features list FONT [--table GSUB|GPOS] [--lookups] [--json]
```

Engine command: `layout.listFeatures`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| spot | `-F` | features list |  |
| spot | `-t GPOS=5 / -t GSUB=5` | features list --table GPOS\|GSUB --lookups |  |
| comparefamily | `-rf` | features list --lookups (check family --layout compares several fonts) |  |

#### `features compile`

Compiles layout source into the tables of an existing font, replacing the tables it builds and leaving the rest of the font alone. Source is OpenType feature code by default; Monotype FontDame text and Microsoft VOLT projects are also accepted.

```
fontcraft-cli features compile FEATURES --font FONT [-o OUT] [--tables TAG,...] [--debug-lookups] [--include-dir DIR] [--syntax fea|mti|volt]
```

`features compile` is for patching layout into a finished font; building a font from source uses
`compile`, which runs the same feature compiler in the same way.

The feature language accepted is the OpenType feature file syntax of the AFDKO specification, including
the variable-font extensions (values that vary by location, conditions), and `include()` statements
resolved against `--include-dir`. Errors carry file, line and column and the include chain.

FontDame and VOLT input is first translated to the internal rule model shared with feature code, so the
same table writers produce the result whatever the syntax.

Engine command: `layout.compileFeatures`. The editor's `font.features.compile` compiles the feature code
of an open document for preview; both use the same compiler.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| feaLib | FEATURES INPUT_FONT | features compile FEATURES --font FONT |  |
| feaLib | `-o / --output` | -o |  |
| feaLib | `-t / --tables` | --tables |  |
| feaLib | `-d / --debug` | --debug-lookups |  |
| feaLib | `-v / --verbose` | -v |  |
| feaLib | `--traceback` | --backtrace |  |
| mtiLib | FILE… | features compile FILE... --syntax mti --to ttx |  |
| mtiLib | `--font / -f` | --font |  |
| mtiLib | `--table / -t` | --tables |  |
| voltLib | INPUT | features compile INPUT (--syntax volt) |  |
| voltLib | `-f / --font` | --font |  |
| voltLib | `-o / --output` | -o |  |
| voltLib | `-t / --tables` | --tables |  |
| voltLib | `-F / --debug-feature-file` | --emit-fea |  |
| voltLib | `--ship` | --ship |  |
| voltLib | `-v / --verbose` | -v |  |
| voltLib | `-T / --traceback` | --backtrace |  |
| addfeatures | `-ff with -f / -o` | features compile (layout tables only); compile on a bare font for the name, OS/2 and cmap work as well |  |
| fontmake | `--mti-source` | compile --mti-source; features compile --syntax mti |  |
| fontc | `--emit-lookup-debug-info` | --debug-lookups |  |

#### `features decompile`

Writes the layout tables of a binary font as OpenType feature code: language systems, glyph and mark classes, lookups and the features that reference them. The result is meant to be readable and to compile back to equivalent tables.

```
fontcraft-cli features decompile FONT [--table GSUB|GPOS|GDEF|BASE,...] [--features TAG,...] [--show-extensions] [-o OUT.fea]
```

Structure of the output: `languagesystem` statements for every script/language pair; glyph class
definitions, named from their first and last member (with a distinguishing member added when two classes
would collide) so that names are stable across builds; mark classes; then for each feature tag the
lookups, written once under a generated name of the form feature_script_language_index and referenced by
name where a later language system reuses them. Lookups shared with the default language system are not
repeated under other language systems; `exclude_dflt` is written where a language system does not
include the default lookups.

Not every table construct has feature syntax (some contextual formats, device tables in odd places). Those
are written as comments with a warning so that nothing is silently lost.

Engine command: `layout.decompileFeatures`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| spot | `-t GSUB=7 / -t GPOS=7 / -t GDEF=7` | features decompile --table GSUB\|GPOS\|GDEF |  |
| spot | `-m / -ag` | --names source --goadb FILE |  |
| ttxn | (GSUB/GPOS written as feature-file text) | features decompile (dump --normalize for the comparison form) |  |
| ttxn | `-se` | --show-extensions |  |

#### `features lint`

Parses and validates feature code without writing a font: syntax, unknown glyphs and classes, rules a lookup type cannot hold, unreachable rules, lookups too large for their offsets, glyphs given conflicting GDEF classes. Exit status 1 when there are errors.

```
fontcraft-cli features lint FEATURES [--font FONT | --glyph-order FILE] [--include-dir DIR] [--warnings default|all]
```

Engine command: `layout.lintFeatures`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| addfeatures | `-V` | --warnings all |  |
| makeotf | `-V / -nV` | --warnings all / default |  |

#### `features convert`

Translates layout source between syntaxes without compiling into a font: a VOLT project or a font carrying VOLT source tables to feature code, and FontDame text to feature code or to an XML table dump.

```
fontcraft-cli features convert INPUT --to fea|ttx [-o OUT] [--table GSUB|GPOS|GDEF,...]
```

Engine command: `layout.convertSyntax`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| voltLib.voltToFea | INPUT OUTPUT | features convert INPUT --to fea -o OUTPUT |  |
| voltLib.voltToFea | `-t / --table` | --table |  |
| voltLib.voltToFea | `-q / --quiet` | -q |  |
| voltLib.voltToFea | `--traceback` | --backtrace |  |
| mtiLib | FILE… (prints XML) | features convert FILE... --to ttx |  |
| mtiLib | `--font / -f, --table / -t` | --font / --table |  |

#### `features normalize`

Prints the effective layout rules of a font in a canonical text form that does not depend on how the compiler arranged lookups and subtables, so that two builds can be compared with a text diff. Covers GPOS kerning, mark attachment and cursive attachment, and the GDEF ligature-caret list. GSUB is accepted but prints no rules, as in otl-normalizer 1.0.0.

```
fontcraft-cli features normalize FONT [--table gpos|gsub|gdef|all] [--location AXIS=VALUE,...] [--face N|NAME] [-o OUT] [--no-clobber]
```

The method is described under *Oracles › Layout normalisation*. In short: rules are grouped by feature
and by the set of language systems that share exactly the same lookups, flattened to one line per glyph
pair or attachment, deduplicated the way a shaping engine would resolve them (the first subtable that
covers a glyph wins), and sorted. Glyphs are printed by name.

Engine command: `layout.normalize`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| otl-normalizer | FONT_PATH | features normalize FONT |  |
| otl-normalizer | `-o / --out` | -o |  |
| otl-normalizer | `-t / --table` | --table |  |
| otl-normalizer | `-i / --index` | --face |  |

### `variations`

Group of utilities for the variation data of a variable font or designspace: write an avar table from a designspace's axis maps, derive an avar that spaces an axis evenly by measurement, convert coordinates between user, design and normalised space, add advance-variation tables, and show or plot how the masters partition the design space.

```
fontcraft-cli variations build-avar|plan-avar|map|build-hvar|model|plot INPUT [args]
```

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| varLib.avar.unbuild | font | dump -t avar=designspace |  |

#### `variations build-avar`

Builds the avar table of a variable font from the axis mappings of a designspace (per-axis maps, and cross-axis mappings as avar version 2) and stores it in the font. Without a designspace it rebuilds avar from the font's own data, which normalises it.

```
fontcraft-cli variations build-avar FONT [DESIGNSPACE] [-o OUT]
```

Engine command: `variations.buildAvar`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| varLib.avar | font [designspace] | variations build-avar FONT [DESIGNSPACE] |  |
| varLib.avar | `-o / --output-file` | -o |  |
| varLib.avar | `-v / --verbose` | -v |  |

#### `variations plan-avar`

Computes an avar mapping from measurements of the font itself so that equal steps of an axis give evenly progressing results: it samples the weight, width, slant or optical-size axis, measures the glyphs at each sample, and solves for the positions at which the requested values are reached. The mapping is written into the font and can be printed as a designspace fragment.

```
fontcraft-cli variations plan-avar FONT [--weights LIST] [--widths LIST] [--slants LIST] [--sizes LIST] [--samples N] [--glyphs LIST] [--pins AXIS=FROM:TO,...] [--design-limits AXIS=MIN:DEF:MAX] [--sanitize] [--plot FILE] [-o OUT]
```

Engine command: `variations.planAvar`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| varLib.avar.plan | font | variations plan-avar FONT |  |
| varLib.avar.plan | `-o / --output-file` | -o |  |
| varLib.avar.plan | `--weights / --widths / --slants / --sizes` | same |  |
| varLib.avar.plan | `--samples` | --samples |  |
| varLib.avar.plan | `-s / --sanitize` | --sanitize |  |
| varLib.avar.plan | `-g / --glyphs` | --glyphs |  |
| varLib.avar.plan | `--weight-design-limits / --width-design-limits / --slant-design-limits / --optical-size-design-limits` | --design-limits wght=… / wdth=… / slnt=… / opsz=… |  |
| varLib.avar.plan | `--weight-pins / --width-pins / --slant-pins / --optical-size-pins` | --pins wght=… / wdth=… / slnt=… / opsz=… |  |
| varLib.avar.plan | `-p / --plot` | --plot FILE |  |
| varLib.avar.plan | `-v / --verbose / -q / --quiet` | -v / -q |  |

#### `variations map`

Converts a location between coordinate spaces of a variable font: user values through fvar normalisation and the avar table to final normalised coordinates, or back. Useful for checking what an avar mapping does at a given point.

```
fontcraft-cli variations map FONT [AXIS=VALUE...] [--from user|normalized] [--to normalized|user] [--all-axes]
```

Engine command: `variations.mapCoordinates`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| varLib.avar.map | font [AXIS=value…] | variations map FONT AXIS=VALUE... |  |
| varLib.avar.map | `-f` | --all-axes |  |
| varLib.avar.map | `-i` | --from normalized |  |
| varLib.avar.map | `-o` | --to normalized |  |

#### `variations build-hvar`

Adds an HVAR table to a variable TrueType font that lacks one, computing advance-width deltas from the phantom points in gvar, so that text engines need not process outlines to get advances. When the font has vertical metrics it adds VVAR as well.

```
fontcraft-cli variations build-hvar FONT [--no-vvar] [--replace] [-o OUT]
```

Engine command: `variations.buildHvar`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| varLib.hvar | font | variations build-hvar FONT |  |
| varLib.hvar | `-o / --output-file` | -o |  |

#### `variations model`

Shows how a set of master locations is turned into a variation model: the locations normalised to -1..1, the order in which masters are processed, and for each master the region of design space (its support) in which its deltas apply.

```
fontcraft-cli variations model (DESIGNSPACE | --locations LOC...) [--json]
```

Engine command: `variations.describeModel`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| varLib.models | `-d / --designspace` | DESIGNSPACE |  |
| varLib.models | `-l / --locations` | --locations |  |
| varLib.models | `--loglevel` | --log |  |

#### `variations plot`

Draws the supports of a variation model as charts, one per master: for one axis a curve of the master's influence along the axis, for two axes a surface, so that gaps and unexpected overlaps between masters can be seen.

```
fontcraft-cli variations plot (DESIGNSPACE | FONT | --locations LOC...) [-o OUT.svg]
```

Engine command: `variations.plot`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| varLib.plot | source.designspace | variations plot DESIGNSPACE |  |
| varLib.plot | location1 location2 … | --locations |  |

### `run`

Dispatches any registered engine command by id with a JSON parameter object and prints its JSON result, which makes every editor feature scriptable from a shell. A script file runs a sequence of commands, and session mode keeps documents open while commands arrive as JSON lines on standard input. Needed from the first milestone, since it is how the vertical slice is exercised headless.

```
fontcraft-cli run COMMAND-ID [--params JSON | --params-file FILE | KEY=VALUE...] [--document PATH] [--save] | run --script FILE | run --session
```

A script file holds one step per line: `ID {JSON}` runs an engine command on the open document, and a
complete `fontcraft-cli SUBCOMMAND …` line runs a subcommand on its own. The run stops at the first
failing step (exit status 5 when earlier steps succeeded) unless `--keep-going` is given.

`run` is the generic door; every other subcommand is a convenience in front of it. A command that is
`kind = "ui"` (view and window state) is refused headless with exit status 2 and a message saying so.
Commands that need a document get the one named by `--document` (or opened earlier in the session);
commands that take file paths in their parameters need none.

The session protocol is the same JSON-lines request and response shape as the control channel, so a
script developed against `run --session` works against a running editor and the other way round.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| tx | `-s` | @FILE for extra arguments; run --script for whole command lines |  |
| sfntedit | `-X` | run --script |  |
| spot | `-X` | run --script |  |
| sfntdiff | `-X` | run --script |  |

### `commands`

Lists the command registry: id, label, kind, menu path, shortcut and summary of every command, filtered by a pattern, kind or area; --describe prints one command's parameter documentation. This is how a person or an agent discovers what 'run' can dispatch.

```
fontcraft-cli commands [PATTERN] [--kind engine|ui] [--area AREA] [--describe ID] [--json]
```

Engine command: `commands.list`.

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| fonttools help | (no arguments) | commands --cli (and fontcraft-cli help) |  |
| afdko | `-a / --all` | commands --cli |  |

### `mcp`

Runs a Model Context Protocol server that exposes the command registry as tools, one tool per engine command with its parameter schema, plus resources for open documents. Agents then drive FontCraft headless exactly as the CLI does.

```
fontcraft-cli mcp [--connect ADDR | --headless] [--transport stdio|http] [--listen ADDR] [--automation-read-root DIR] [--automation-write-root DIR] [--read-only] [--allow PATTERN]... [--deny PATTERN]...
```

### `bench`

Times a subcommand: runs it several times on the given inputs, discards its outputs, and reports the minimum, median and spread of wall time and peak memory together with the per-stage breakdown. It can run reference tools found on PATH on the same input for a side-by-side figure.

```
fontcraft-cli bench [--runs N] [--warmup N] [--compare COMMAND]... [--stages] [--save FILE] [--baseline FILE] [--threshold PCT] [--json] SUBCOMMAND [ARGS...]
```

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| cu2qu | benchmark module | bench outlines to-quadratic … |  |
| qu2cu | benchmark module | bench outlines to-cubic … |  |

### `completion`

Prints a shell completion script for the named shell to standard output. Completions cover subcommands, actions, option names and enumerated option values.

```
fontcraft-cli completion bash|zsh|fish|powershell
```

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| afdko | completion | completion SHELL |  |

### `help`

Prints the list of subcommands with one line each, or the full reference of one subcommand: usage, options with defaults, examples and the engine command it runs. --all prints every subcommand's reference in one stream.

```
fontcraft-cli help [SUBCOMMAND [ACTION]] [--all]
```

| Reference tool | Reference option | Ours | Note |
|---|---|---|---|
| afdko | `-h / --help / help [command]` | help [SUBCOMMAND] |  |
| afdko | -s / --secondary, -p / --plot, -a / --all (help listing by category) | help --all (one list, no categories) |  |
| afdko | `-v / --version` | --version |  |
| afdko | abbreviated command names (mo, ah, se, mf, rf, bvf, bmo, miu, cou, sh, af, t1, dt1, cf) | unambiguous prefixes of subcommand names |  |
| fonttools help |  | help |  |

## Deliberately not carried over

Everything in the reference tools' option lists that has no counterpart above, with the reason. The bar
for leaving something out: it serves hardware or operating systems that no longer exist, it selects
between internal libraries of the reference implementation, or it is a test hook of that
implementation. Reading legacy formats is kept wherever it is cheap (Type 1 in all three forms,
multiple-master snapshots, CID-keyed sources with CMap resources, FontMenuNameDB in both syntaxes).

| Reference tool | Option or behaviour | Why it is left out |
|---|---|---|
| tx | -t1 -1, -t1 -2, -c (ASCII85), +S / -S in -t1 mode (ProcSet or Private OtherSubrs), +E / -E in -t1 mode (WasEmbedded key), +w / -w (widths-only font), +T / -T in -t1 mode | These produce the incremental-download forms of Type 1 fonts that printer drivers sent to PostScript printers. Only ordinary host fonts (PFA, PFB) are written. |
| tx | -LWFN | Classic Mac OS printer-font file kept in a resource fork. |
| tx | -cef, -F, -cefsvg | Compact Embedded Font is an Adobe-specific embedding container that nothing in the open toolchain consumes. |
| tx | +O / -O | Optimisation for fonts burnt into printer ROMs. |
| tx | -ps as an output language, -ps -d (duplex) | PostScript page descriptions and printer duplex setup; the same sheets are produced as PDF, SVG or HTML by proof. |
| tx | -path, -path -0, -path -1 | Self-test of the reference library's path code, not a font operation. |
| tx | -X | Emits the approximated and the exact quadratic-to-cubic conversion on top of each other to compare the two converters; a development aid for that converter. |
| tx | -t | Trace of PostScript tokens while parsing Type 1, used to find where a malformed font breaks; our parse errors report the byte offset and the token instead. |
| tx | -m | Simulates a failing memory allocation inside the reference implementation. |
| tx | -r, -R, -i <resource id>; FFIL, LWFN-POST and AppleSingle/AppleDouble input | Mac resource-fork and AppleSingle/AppleDouble containers. Collections are addressed with --face; fonts in these wrappers must be unwrapped first. |
| tx | -sr, -sd | Path prefixes for source files; shells, globs and @FILE response files cover this. |
| tx | -bc, -z, -sha1, -cmp | Bitmap-comparison mode; the reference itself has removed it and rejects these options. |
| tx | -no_futile | Already ignored by the reference with a deprecation warning. |
| spot | -r, -i | Mac resource map and resource ids (resource-fork fonts). |
| spot | -l, -O | Sending proof output to a print spooler or choosing between spooling and standard output; proof writes files. |
| spot | PostScript as the proof output language | Proofs are written as PDF, SVG or HTML. |
| spot | -R | Applies an extra rounding step to coordinate labels in glyph plots; our labels show the stored values. |
| makeotf | -adds / -nadds, -serif, -sans | Synthesising the Euro, Apple symbol and other missing glyphs from two built-in multiple-master fonts. It depends on glyph data built into the reference program that we do not have, the current makeotf help no longer lists the options, and the guide itself says the synthetic glyphs cannot match a design's stroke contrast. |
| makeotf | -dcs | Sets OS/2 usDefaultChar to the space glyph as a workaround for QuarkXPress 6.5; documented in the guide but gone from the current tool. |
| makeotf | automatic writing of current.fpr after every run | Hidden state next to the source makes builds depend on history. --save-options writes the file when asked. |
| makeotf | XUID project-file key | Type 1 era unique identifier that current guidelines say to leave out. |
| otfautohint | --test | Switches the reference into the mode its own test-suite needs. |
| makeinstancesufo | --use-varlib | Choice between two interpolation libraries of the reference. FontCraft has one interpolation model (the OpenType variation model) and reports where a designspace relies on behaviour only the other library had. |
| makeinstancesufo | default designspace file name font.designspace | The input is always given explicitly. |
| sfntedit | temporary file sfntedit.tmp in the working directory | In-place edits use a temporary file beside the target instead, so the working directory need not be writable. |
| proofpdf | requirement that label and title fonts are PostScript base fonts | Labels are set in a bundled open font; --set pageTitleFont and similar accept any installed font. |
| ttx | default behaviour of never overwriting (output renamed with #1, #2 …) | Outputs are replaced by default and --no-clobber makes an existing file an error; invented names are hard to script against. |
| pyftsubset | --harfbuzz-repacker / --no-harfbuzz-repacker | Selects between two table serialisers of the reference. There is one serialiser here, and it must resolve offset overflows on its own. |
| pyftsubset | --lazy (undocumented) | Loading strategy of the reference library. |
| ttLib | --lazy / --no-lazy | Loading strategy of the reference library; tables are always read on demand here. |
| varLib.interpolatable | --ps | PostScript report output; PDF and HTML reports are kept. |
| cffLib.specializer | charstring program typed as positional arguments, --num-regions | Developer harness that specialises a charstring given on the command line. optimize --charstrings works on fonts and reads the region count from the font. |
| varLib.merger | (library only, no command line) | Its job, merging per-master tables into variable tables, happens inside compile. |
| fontmake | --use-mutatormath | Hidden option for an interpolation library fontmake itself no longer uses. |
| fontmake | --overlaps-backend booleanOperations\|pathops | Choice between two overlap-removal libraries. There is one built-in implementation; results are compared against both in tests. |
| fontmake | --subroutinizer compreffor\|cffsubr | Choice between two subroutinizer libraries. There is one built-in subroutinizer for CFF and CFF2. |
| fontmake | Python class paths in --filter and --feature-writer (module::Class) | Loading Python code is not possible. The options take the names of registered filter commands and built-in feature generators; third-party ones come through the plug-in system. |
| ttx_diff | --normalizer_path | The layout normaliser is built in. |

## Oracles

For each subcommand: the reference tool whose output ours is compared with in differential tests, and
how the two outputs are compared. Byte equality is the exception; most comparisons first remove
differences that do not matter. The comparison methods are described after the table.

| Subcommand | Oracle | Comparison |
|---|---|---|
| `compile` from `.glyphs`, designspace, UFO to TTF / variable TTF | fontc; fontmake | *Compiler normalisation* (below). Target: every table identical after normalisation; the similarity ratio per table is tracked over a corpus |
| `compile` to OTF / CFF2 from the same sources | fontmake (`-o otf`, `otf-cff2`, `variable-cff2`) | Compiler normalisation with charstrings desubroutinized on both sides |
| `compile --afdko-project`, bare font plus features | AFDKO `makeotf` (which calls `tx` and `addfeatures`) | *Normalised dump* of both; layout through *layout normalisation*; `name`, `OS/2`, `cmap`, `hhea`, `post` field by field |
| `compile --format variable-cff2` from masters | AFDKO `buildmasterotfs` + `buildcff2vf` | Normalised dump; blends compared after instantiating at each master location (outlines must equal the masters) |
| `compile --master-finder` | fontTools `varLib` | Compiler normalisation |
| `dump --format ttx` | fontTools `ttx` | XML trees equal ignoring comments and the version attribute of the root; and round trip: `ttx` compiles our dump to the same tables, we compile ttx's dump to the same tables |
| `dump` text views, `info`, `glyphs list`, `features list` | AFDKO `spot`, `tx -dump`, `ttx -l` | The values, not the layout: both outputs are parsed into (key, value) records per table and the records compared. Our own text layout is covered by snapshot tests |
| `tables …` | AFDKO `sfntedit` | Extracted table files byte-identical. Edited fonts: same table set and table bytes, same directory order, valid checksums (verified by both tools) |
| `convert --to ttf` | AFDKO `otf2ttf` (fontTools cu2qu) | `glyf` point-for-point with the off-by-one budget; if point structure differs, maximum distance between our outline and the source outline must be within `--max-error` |
| `convert --to cff / cff2 / pfa / pfb / ufo / svg-font`, `convert` from Type 1 | AFDKO `tx` | *Abstract-font dump*: `tx -dump -6` of our output and of tx's output are equal apart from number formatting and subroutinization |
| `convert` TTX → binary | fontTools `ttx` | Tables byte-identical where encoding is forced by the data (head, hhea, maxp, OS/2, name, post, cmap after canonical ordering); normalised dump for the rest |
| `convert --to otf-cff2`, CFF2 → CFF | fontTools `cffLib.CFFToCFF2`, `CFF2ToCFF` | Normalised dump |
| `subset` | fontTools `pyftsubset` | First the retained glyph list and old→new index map must be identical. Then normalised dump of every table; layout through layout normalisation |
| `instance` on a variable font | fontTools `varLib.instancer` (and `varLib.mutator` for full instances) | Normalised dump with the off-by-one budget for rounded coordinates and metrics; partial instances additionally re-instantiated at several random locations by both tools and the static results compared |
| `instance` on a designspace | AFDKO `makeinstancesufo`; fontmake `-i` | UFOs compared after canonical writing (normalised glif and plist), numeric values within rounding |
| `merge` | fontTools `pyftmerge`; AFDKO `mergefonts` | Font merge: normalised dump. Glyph merge: abstract-font dump |
| `diff` | AFDKO `sfntdiff`, fontTools `diff` | Same verdict per table (equal or different) over a corpus of font pairs; our detail output is snapshot-tested |
| `check` (profiles) | FontBakery, check by check | Specified in `docs/spec/21-qa.md`; per check the same pass/fail verdict on a shared corpus |
| `check outlines` | AFDKO `checkoutlinesufo` | The set of (glyph, problem kind) reported must match; with `--fix`, the filled area of our result and the reference result may differ by at most a small fraction of a unit squared per glyph (overlap removal is not point-identical between implementations) |
| `check family` | AFDKO `comparefamily` | Set of (test number, font, message class) findings on a corpus of families |
| `check interpolation` | fontTools `varLib.interpolatable --json` | Set of (glyph, problem type, master pair) |
| `outlines remove-overlaps` | fontTools `ttLib.removeOverlaps`; AFDKO `checkoutlinesufo -e`; fontmake static builds | Geometric: symmetric difference of the two filled results below a threshold, same contour count, same winding. Never point equality |
| `outlines to-quadratic`, `to-cubic` | fontTools `cu2qu`, `qu2cu` | Point-identical where our algorithm is a port of the same method; otherwise same or fewer points and error within the bound |
| `outlines componentize`, `decompose` | AFDKO `ttfcomponentizer`, `ttfdecomponentizer` | `glyf` equal after sorting nothing: component lists and flags must match exactly |
| `autohint` | AFDKO `otfautohint` | Per glyph, the list of stems, hint masks, counter masks and flex from an abstract-font dump of both results. Tracked as the share of glyphs with identical hints; differences reviewed by category |
| `stems` | AFDKO `otfstemhist` | Report contents equal (values, counts, glyph lists) |
| `dehint` | AFDKO `tx -n`; fontTools `pyftsubset --no-hinting '*'` | Abstract-font dump (CFF); normalised dump (TrueType) |
| `transform` | AFDKO `rotatefont`; fontTools `ttLib.scaleUpem` | Abstract-font dump, values within rounding; for `--upem`, normalised dump with the off-by-one budget |
| `optimize` | fontTools `otlLib.optimize`, `cffLib.width`, `varLib.varStore` | Behaviour must not change: layout normalisation before and after equal; outlines equal after desubroutinizing; then size compared with the reference result (ours should not be larger by more than a few bytes) |
| `metrics` | AFDKO `tx -mtx`, `tx -afm`; fontTools `pens.statisticsPen` | Numbers equal within a small epsilon |
| `proof` | AFDKO `proofpdf` family, `tx -pdf`, `spot` proofs | No pixel oracle. Structural checks only: page count, glyphs per page, glyph order, presence of the requested elements; visual review of a fixed sample |
| `features compile` | fontTools `feaLib`; AFDKO `addfeatures`; fontc (fea-rs) | Layout normalisation for GPOS and GDEF; normalised dump for GSUB and for the other tables a feature file can set |
| `features decompile` | AFDKO `spot -t GSUB=7 / GPOS=7`, `ttxn` | Round trip: compiling our decompiled text reproduces tables that normalise equal to the original. Reference text is not compared literally |
| `features normalize` | fontc `otl-normalizer` | Text identical |
| `features convert` | fontTools `voltLib.voltToFea`, `mtiLib` | Compile both translations and compare with layout normalisation and normalised dump |
| `collection` | AFDKO `otf2otc`, `otc2otf`; fontTools `ttLib` | Unpacked members byte-identical to the reference's. Packed collections: same members, same set of shared tables, each member unpacks to identical tables |
| `woff` | fontTools `ttLib.woff2` | Decompressing our file with the reference gives tables identical to the input; decompressing the reference's file with ours gives the same |
| `glyphs rename`, `reorder` | AFDKO `tx -gf`; fontTools `ttLib.reorderGlyphs` | Normalised dump (which is glyph-order independent) plus the expected glyph order |
| `variations …` | fontTools `varLib.avar`, `avar.map`, `avar.plan`, `hvar`, `models` | `avar` / `HVAR` table dumps equal; coordinates and supports numerically equal |
| `oracle`, `run`, `commands`, `mcp`, `bench`, `completion`, `help` | none | Own tests |
| `tables show` | none | Tests on fonts built in code: every field of the tables we model appears with its name and value |
| `sources diff`, `sources merge` | none | Round trip: merging every change of A → B gives a document equal to B; a source compared with itself gives no change |

### Method: compiler normalisation (fontc's `ttx_diff`)

`ttx_diff` exists to compare what two compilers make of the same source. Its method, as read from its
README and source, and adopted for `diff --normalize compiler` and `oracle`:

1. **Same conditions.** Both builds run with the same `SOURCE_DATE_EPOCH` so that time stamps agree, and
   the reference compiler is given options that match the other's defaults (for example overlaps kept).
2. **Dump.** Each font is dumped to TTX XML. Independently, GPOS and the GDEF ligature-caret list of each
   font are written out by the layout normaliser (next method).
3. **Fill in optimised-away variation deltas.** Compilers may omit `gvar` deltas for points whose
   movement interpolation reproduces, and they may choose differently. Before comparing, every glyph's
   tuples are made dense: missing point deltas are computed by the interpolation rule and rounded, so
   only the effective movement is compared.
4. **Remove differences that do not matter**, in both dumps:
   - optionally strip Extension lookup wrappers so promoted and unpromoted lookups look alike;
   - sort lists whose order has no meaning: lookup indices inside a feature, feature indices inside a
     language system, mark filtering sets in GDEF;
   - renumber the classes of class-based contextual rules in a canonical way and reorder their rule sets
     accordingly;
   - replace name IDs that are merely references (axis names, instance names, STAT value names, feature
     UI names) by the strings they point to, then sort the name records, so different ID allocation is
     invisible;
   - treat a vendor ID of NULs and of spaces as the same; drop a few name records only one tool emits;
     remove the compiler's own version stamp; blank the whole-font checksum;
   - bring `STAT` into one canonical form;
   - delete from the GPOS dump the kerning, mark and cursive lookups, and from GDEF the caret list and
     variation store, because those are compared through the normaliser's output instead;
   - where two glyphs have the same contours in a different order or with a different starting point,
     reorder one to match, and apply the same point reordering to that glyph's `gvar` deltas;
   - allow a limited number of coordinates and deltas to differ by exactly one unit (rounding at a
     different stage): up to a budget, by default one in a thousand values, such values are made equal
     and marked; beyond the budget they count.
5. **Compare per table.** The XML is cut into one text per table (written to disk for inspection). For
   each table present in both fonts a similarity ratio between 0 and 1 is computed from a line-based
   sequence match; 1 means identical. Tables present on one side only are reported with that side's
   name. The normaliser outputs are compared the same way as two extra pseudo-tables.
6. **Sizes.** Table byte sizes are compared separately and reported when ours exceeds the reference's by
   more than a tenth, since equal content can still be encoded wastefully.
7. **Report.** Either a human summary or JSON: `{"success": {"<tag>": ratio | "<only side>"}}` or
   `{"error": {"<compiler>": {"command", "stderr"}}}`; status 0 only if everything is identical.

`fontc_crater` applies the same comparison to a large list of public source repositories, caches the
checkouts, stores the per-font JSON results and renders a summary page, so that the share of fonts that
compile identically can be followed over time. `oracle --corpus` is the counterpart.

### Method: layout normalisation (fontc's `otl-normalizer`)

Two compilers can express the same kerning or mark positioning with different lookups, subtable formats
and subtable splits, so table dumps cannot be compared directly. The normaliser prints what the rules
*do*:

1. **Glyph names.** Indices are printed as names: from `post` when it has them; otherwise the standard
   name of the character the glyph is mapped from, or `uniXXXX` / `uXXXXX`; otherwise a name built from
   the index.
2. **Group by feature and language systems.** For every feature it collects the lookups reached from
   each script/language pair. Pairs that reach exactly the same lookups for that feature are grouped;
   each group gets a header naming the feature and its script/language pairs, with `DFLT/dflt` first.
3. **Flatten lookups to rules.** Within a group, all subtables of all lookups are decomposed into
   single rules by kind: glyph-pair adjustments, mark-to-base, mark-to-mark, mark-to-ligature and
   cursive attachments. Subtable format disappears: a class-pair matrix and a list of glyph pairs yield
   the same kind of rule.
4. **Resolve precedence.** A shaping engine uses the first subtable of a lookup that covers the glyph
   (or first glyph of a pair). A rule in a later subtable that an earlier one would pre-empt is
   dropped. Adjustments that are entirely zero are dropped.
5. **Resolve values.** Device tables and variation indices are replaced by the actual deltas computed
   from the variation store, so that different store layouts compare equal.
6. **Print sorted.** Rules are printed one per line, sorted, under comment lines that give the rule
   kind and count, the lookup flag, and the mark filtering set as a glyph list when one applies.

The GDEF ligature-caret list is printed the same way (glyph name, caret positions with resolved
deltas). GSUB is a stub in the reference; ours prints substitutions flattened per feature group in the
same style, which has no oracle and is checked by round-trip instead.

### Method: normalised dump (AFDKO `ttxn`)

For comparing two *versions of one font* rather than two compilers. Each table is dumped to its own file
after removing what may legitimately differ: charstrings are written fully expanded so subroutinization
does not matter; GSUB and GPOS are written as feature-file text, organised by walking script → language
→ feature, with class names derived from the classes' members and lookups written once and referred to
afterwards, so record order and index allocation do not matter; glyphs are referred to by name so glyph
order does not matter; optionally hints, and the version fields (`head.fontRevision`, name ID 3), are
left out. The per-table files are then compared with an ordinary text diff. `dump --normalize` and
`diff --normalize dump` implement this.

### Method: abstract-font dump (AFDKO `tx -dump`)

For outline-level tools. `tx` reads any outline format into one abstract representation and can print
it as text: global font data, then per glyph the path in absolute coordinates with hints. Two fonts in
different containers (Type 1, CFF, CFF2 instance, TrueType) can therefore be compared by dumping both at
the most detailed level and comparing the text, after removing differences in number formatting. Our
`dump -t @font -t @glyphs --glyph-detail long` produces the equivalent for use when `tx` is not at hand.

### Method: geometric comparison

Where implementations legitimately produce different points for the same shape (overlap removal, curve
conversion, autohinter outline clean-up), outlines are compared as areas: the symmetric difference of
the two filled shapes must be below a threshold proportional to the em, contour counts and winding must
agree, and extremes must coincide within one unit. This has no reference tool; it is our own test
helper and is always paired with a cheaper structural comparison that passes in the common case.

### Round trips and invariants

Used wherever no oracle exists or as a second line of defence: `dump` then `convert` reproduces the
tables; `woff compress` then `decompress` reproduces the font; `collection pack` then `unpack`
reproduces the members; `features decompile` then `compile` normalises equal; `optimize` does not
change normalised layout or expanded outlines; `subset` with everything kept changes nothing but
table order; any subcommand run twice on the same input gives identical bytes.

## Open questions

Items the reference documentation and help texts leave unclear, or decisions that need confirming
before implementation.

1. **GOADB column order.** The MakeOTF user guide says the first column is the final name and the second
   the source name; the `tx` help text describes the first as the development name and the second as the
   final name. We follow the user guide. Confirm by running both tools on one file.
   **Answered (M12 plan, 2026-10-11):** the user guide is right. Running tx 5.0.1 with the line `finalA devA` renames the glyph `devA` to `finalA`: first column final name, second source name, third override. tx's help text describes the columns the other way round.
2. **makeotf options present in the guide but not in the current help** (`-addn`, `-adds`, `-serif`,
   `-sans`, `-dcs`, `-cv`): confirm which still work in the current release. `-cv` is in the
   `addfeatures` help, so it is kept; `-addn` is mapped to a generated box `.notdef`.
   **Answered (M12 plan):** makeotf 5.0.1 has none of `-addn`, `-adds`, `-serif`, `-sans`, `-dcs` in its help or its option table; they are not carried over. `-cv` is the vertical CMap path of addfeatures, used only for CID fonts.
3. **Exact effect of several `tx` switches**: which Top DICT keys `+E` removes; which charstring
   optimisations `+T` and `-no_opt` toggle; what distinguishes dump levels "short", "medium" and "long".
   Needs observing output.
   **Partly answered (M12 plan):** `tx -cff -h` gives the defaults `-E +F -O -S +T -V -Z -b +W`; `+E` removes purely descriptive keys, `+T` turns on charstring optimisations, `-no_opt` keeps operators unshortened. Dump levels `-0` to `-6` are global data and glyph data in short, medium and long form. The exact keys `+E` removes are observed by package M12.30.
4. **`spot` details**: the meaning of dump levels below 4 for tables without a usage entry, the proof
   options `-a`, `-c`, `-R` in practice, the undocumented `-A`, and what the `=7`/`=8` glyph synopses
   contain. Needs running `spot` on sample fonts.
   **Deferred to package M12.10 (M12 plan):** observed by running spot on corpus fonts while the text views are built.
5. **`otfautohint` default glyph selection for non-UFO input.** The help marks `-a/--all` as UFO-only;
   whether already hinted glyphs of an OTF are skipped by default must be observed.
   **Answered (M12 plan):** in otfautohint 5.0.1 an OTF input has every glyph hinted again and its old hints replaced; `--all` only changes UFO runs, where unchanged glyphs are skipped by stored hashes.
6. **`checkoutlinesufo -e`**: the exact list of repairs and their order, and how contour order is
   restored after overlap removal.
   **Answered (M12 plan, from the Apache-2.0 source of AFDKO 5.0.1):** coincident points, flat curves, collinear lines, overlap removal repeated until the rounded point list stops changing, tiny contours, collinear lines again; coordinates rounded (three decimals with `-d`); points closer than one unit merged; then unchanged contours are put back at their old index and the others sorted.
7. **`comparefamily` tests** are specified here only by title. Each needs its rule extracted from the
   source into a `[[check]]` row (single test 29 is an empty placeholder in the reference).
   **Answered (M12 plan):** AFDKO 5.0.1 has single-face tests 1 to 34 and family tests 1 to 21; every one maps to `[[check]]` rows (`21-qa.md` § 6). Package M12.58 reads each rule from the source.
8. **`pyftsubset` default feature list** includes "features required for script shaping"; the full list
   must be taken from the source (`--layout-features=?` prints it).
   **Answered (M12 plan):** fontTools 4.66.1 keeps 64 layout features by default (listed in the `subset` row's notes) plus four internal pseudo-tags we do not carry over.
9. **`varLib.interpolatable` thresholds.** What `--tolerance` and `--kinkiness` measure precisely, and
   how master pairs are chosen, is recorded in `docs/spec/21-qa.md` from the implementation; the illustrated
   PDF/HTML report of the reference has no counterpart yet.
   **Answered:** recorded in `21-qa.md` § 6; the illustrated report stays out of M12 (`21-qa.md` question 10).
10. **Adobe CMap resources.** makeotf finds default CMap files in its installed shared data. Decide
    whether FontCraft bundles the cmap-resources files or requires `--cmap-*` paths.
   **Answer (`docs/plans/M12.md` § 7, question 6):** CMap files given by path in M12; whether data is bundled is decided with M7's ROS row.
11. **Type 1 output.** PFA/PFB writing is kept for parity with `tx` and `mergefonts` workflows. Decide
    whether it is in scope for 1.0 or reading alone suffices.
   **Answer (`docs/plans/M12.md` § 7, question 5):** Type 1 is read, never written.
12. **Resource-fork fonts.** Reading `.dfont` (data-fork suitcases) is listed as not carried over along
    with the other Mac containers; reconsider if users still need it.
   **Answered (M12 plan, our decision):** not carried over; nothing in the corpus or the rows needs it.
13. **TrueType autohinter options.** fontmake passes a ttfautohint argument string. Whether
    `--tt-options` accepts that syntax unchanged depends on the TrueType autohinter design.
   **Answered (M12.39):** yes. `--tt-options` hands the string unchanged to the TrueType autohinter, which reads ttfautohint's long command-line options, as fontmake passes them.
14. **fontmake output layout.** `--output-layout by-format` must reproduce fontmake's folder and file
    naming exactly to be a drop-in; the rules need to be observed for each format.
   **Not M12:** `--output-layout` belongs to the `compile` row (M2); the rules are observed when that option is built.
15. **Filter and feature-writer names.** The built-in names accepted by `--filter` and
    `--feature-writer` must be defined so that the specs found in existing UFO libs
    (`com.github.googlei18n.ufo2ft.filters`, `…featureWriters`) can be translated.
   **Not M12:** belongs to `compile` (M2) and the feature writers (M8).
16. **Compatibility shims.** Should the binary react to being invoked as `makeotf`, `tx`, `ttx`,
    `pyftsubset` and translate the old option syntax? Not designed here; the mapping tables would make
    it mechanical.
   **Answered (M12 plan, our decision):** no shims in M12; the mapping tables keep them possible later.
17. **Schema.** `[[cli]]` ids of action rows contain a space (`tables list`); the README does not say
    whether that is allowed. The `command` field holds one id although `check`, `autohint` and `stems`
    dispatch a small family of commands (listed in their sections). Subcommands that reuse commands from
    other inventory files (`autohint.*`, `audit.*`) rely on those ids staying as they are.
   **Answered:** ids with a space pass `cargo xtask spec`; `command` names the main command and the sections list the others.
18. **Overlap between editor and whole-font commands.** `outlines.*`, `font.transform` and
    `audit.checkInterpolation` do for files what `path.removeOverlap`, `path.correctPathDirection`,
    `path.addExtremes`, `glyph.decomposeGlyph`, `path.transformations`, `font.info.scaleToUnitsPerEm`
    and `interpolation.checkCompatibility` do for an open document. Decide whether each pair should be
    one command with a `target` parameter.
   **Answered (decision D2):** one implementation per action. For a source, the whole-file command runs the document command on each glyph; for a binary, it decodes the glyph, runs the same routine and writes the glyph back.
19. **Additional oracles.** HarfBuzz's subsetter and shaper and the OpenType Sanitizer are not among the
    reference checkouts but would strengthen the `subset`, `optimize` and `features` tests.
   **Answered (M12 plan):** HarfBuzz's subsetter and shaper are used through uharfbuzz (installed with FontBakery) as second oracles for `subset` and the layout checks; the OpenType Sanitizer runs only when installed and asked for (`check.files.sanitiser`).
20. **Exit status.** `ttx_diff` uses 2 for "different"; ours uses 1 for findings and 2 for usage errors;
    the oracle harness translates. The QA inventory (`audit.exitStatus`) uses 2 for unreadable targets as
    well as usage errors, where this document uses 3. Settle one scheme before scripts depend on it.
   **Answered (M12 plan, our decision):** 3 for an unreadable input everywhere, `check` included; `audit.exitStatus` says the same. The oracle harness translates `ttx_diff`'s 2.
