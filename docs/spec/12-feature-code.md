# Feature Code

Sources: Glyphs 4 Handbook, chapter "Feature Code" (https://handbook.glyphsapp.com/feature-code/),
the Features section of chapter "Font Info", and the Appendix sections Text Tokens, Regular
Expressions, URL Scheme, Custom Feature Code Snippets and Automatic Feature Generation
(https://handbook.glyphsapp.com/appendix/), read 2026-10-07. Language reference: AFDKO
`docs/OpenTypeFeatureFileSpecification.md` v1.27, `Variable_Feature_Guide.md`,
`Feature_Parser_Notes.md` (Apache-2.0). Compiler reference: fea-rs in `fontc` (MIT OR Apache-2.0),
README, grammar and test data.

Inventory: `spec/inventory/12-feature-code.toml` (editor additions, tokens, extensions, automatic
features, text tokens, regex, URL scheme, snippets) and `spec/inventory/12b-feature-language.toml`
(one `fea.*` row per language construct).

The Features tab and its commands are specified in `docs/spec/06-font-info.md`
(`fontInfo.features`, `font.features.*`). This document covers what the code means and how it is
turned into compiler input.

## 1. From stored code to compiler input

Stored data (per font): ordered lists of prefixes, classes and features. Each entry has a name
(features: a tag), code, an automatic flag and notes; ss01-ss20 also carry localised names.

FontCraft builds one standard feature file per export target. The steps below are our design
(`featureEditor.afdkoSyntax`, `token.expansionTiming`); the reference documents only the results.

1. Pick the target: a static instance, the variable font, or a master for the Edit View preview.
2. Regenerate automatic entries for the target's glyph set (`font.features.update` logic, section 8).
   Instance parameters Add/Remove/Replace Prefix, Class and Feature are applied to the manual
   entries (`fontInfo.features.exportSpecific`).
3. Resolve `#ifdef VARIABLE` / `#ifndef VARIABLE` blocks (section 5).
4. Expand tokens against the target (section 3).
5. Lower the syntax extensions to standard syntax (sections 4 and 5).
6. Assemble the file: classes as `@Name = [ ... ];`, prefixes (the language systems are in the
   automatic Languagesystems prefix), then `feature tag { ... } tag;` blocks in sidebar order, as
   both reference compilers do (`featureEditor.outputOrder`,
   `featureEditor.featureBody`, `featureEditor.classContent`).
7. Generate the implicit features and splice them in at `# Automatic Code` markers or as new
   feature blocks (section 7).
8. Compile with the feature compiler; map every diagnostic back to entry and text range so
   `font.features.listErrors` and the inline markers can show it.

`font.features.expandCode` returns the result of steps 1-7 for a target; it is the headless way to
see what the compiler receives. Source maps from expanded text back to entry offsets are required
for step 8 because steps 3-5 change line and column positions.

Edit View uses the same pipeline for the current master after `font.features.compile`
(`features.menuNeedsCompile`, `features.previewScope` in the Edit View inventory).

## 2. Editor additions

Beyond the rows in `06-font-info`:

| Item | Id | Behaviour |
|---|---|---|
| Autocomplete menu | panel `fontInfo.featureAutocomplete`, query `font.features.complete` | Opens while a glyph, class or lookup name is typed; kind depends on position (glyphs after `sub`, lookups after `lookup`). Up/Down, type to narrow, Return to accept. |
| Glyph or class preview | panel `fontInfo.featureNamePreview`, `font.features.previewName` | Option-click on a name: glyph drawing, or every member of the class. |
| Token preview | panel `fontInfo.featureTokenPreview`, query `font.features.evaluateToken` | Option-click on a token: the computed number, or the list of glyphs a predicate matches. Evaluated for the master shown in the editor's context (which master that is: open question). |
| Error details | panel `fontInfo.featureErrorDetails`, `font.features.showErrorDetails`, `font.features.applyFix` | Click on an inline error icon: long description; errors with a wrench carry a fix. |
| Snippets menu | panel `fontInfo.featureSnippetsMenu`, `font.features.insertSnippet`, `font.features.showSnippetFolder` | See section 12. |
| Reordering | `font.features.reorder` (inferred) | Order of features = order of feature blocks; chained suffixes must follow it. |

## 3. Tokens

Tokens begin with `$` and are replaced by text before compilation (`token.kinds`). They may appear
in prefixes, classes and features, and in the Anchor Context field of contextual anchors (Edit View
chapter).

### Grammar

```
token            = number-token | property-token | predicate-token

number-token     = "$" name
                 | "${" name { operator operand } "}"
property-token   = "${" glyph-name ":" key-path { operator operand } "}"
operator         = "+" | "-" | "*" | "/"
operand          = number | name
key-path         = identifier { "." identifier }

predicate-token  = "$[" predicate "]"
predicate        = term { "OR" term }
term             = factor { "AND" factor }
factor           = [ "NOT" ] ( "(" predicate ")" | comparison )
comparison       = key-path comparator value
                 | string "in" key-path                       ; "ipa" in tags
                 | key-path "in" "class(" class-name ")"      ; name in class(Narrow)
comparator       = "==" | "!=" | "<" | "<=" | ">" | ">="
                 | "endswith" | "beginswith" | "contains"
value            = string | number | "true" | "false" | enum-word
string           = '"' characters '"'
```

Handbook-attested: `==`, `>`, `endswith`, `contains`, `in`, `AND`, `OR`, `NOT`, parentheses,
`class()`. Added from fea-rs's reading of the same syntax (`token.predicate.stringOperators`): `=`,
`!=`, `<>`, `beginswith`, single-quoted strings, operator keywords in any letter case. The other
numeric comparators are assumed. Precedence NOT > AND > OR is assumed; fea-rs refuses mixed
AND/OR without parentheses, and refuses NOT and parentheses altogether.

### Number value tokens

- `$name` inserts the master's number value `name` (Font Info > Masters > Number Values);
  for an instance the value is interpolated (`token.numberValue.simple`, `.interpolated`,
  also `fontInfo.numbers.tokensInterpolate`).
- `${name*2}`: arithmetic with `+ - * /`; the name comes first (`token.numberValue.arithmetic`).
  Following fea-rs: several operators per expression, `*` and `/` bind tighter, decimal and negative
  literals allowed, spaces allowed, result rounded to an integer
  (`token.numberValue.expressionDetails`).
- Variable font: the token becomes a variable value with one entry per master location
  (`token.numberValue.variable`). fea-rs can do this itself when given the number values per
  location; otherwise FontCraft emits the `(default loc:value ...)` form.
- Unknown name: compile error at the token (`token.numberValue.unknownName`, inferred).
- Typical use: `pos @Uppercase <$tracking 0 ${tracking*2} 0>;` in cpsp.

### Glyph property tokens

- `${glyph:property}` with property one of `width`, `LSB`, `RSB`, `TSB`, `BSB`, or
  `anchors.<name>.x` / `anchors.<name>.y` (`token.glyphProperty.*`).
- Arithmetic as above: `${comma:RSB*3}`.
- Read from the glyph's layer for the target master or the interpolated instance
  (`token.glyphProperty.layerChoice`, inferred). For the variable font FontCraft emits a variable
  value built from the master layers.
- Missing glyph, missing anchor or unknown property: compile error at the token (inferred).

### Glyph predicate tokens

- `$[predicate]` expands to the matching glyph names separated by spaces
  (`token.predicate.syntax`). In a class entry it stands among bare names; in prefix and feature
  code it must be inside class brackets (`token.predicate.inClassEntry`, `.inCode`).
- Result order: glyph order; non-exporting glyphs left out for export targets
  (`token.predicate.resultOrder`, inferred). Empty result: an empty class, which is an error where
  a rule needs members (`token.predicate.emptyResult`).

Attributes named in the handbook:

| Attribute | Type | Values / example | Row |
|---|---|---|---|
| `name` | string | `name endswith ".tf"`, `name in class(X)` | `token.predicate.string`, `.classFunction` |
| `script` | string | `script == "armenian"` | `token.predicate.string` |
| `category` | string | `category == "Mark"` | `token.predicate.string` |
| `note` | string | `note contains "todo"` | `token.predicate.string` |
| `tags` | set of strings | `"draft" in tags` | `token.predicate.tags` |
| `case` | enum | `noCase`, `upper`, `lower`, `smallCaps`, `minor`, `other` | `token.predicate.case` |
| `direction` | enum | `LTR`, `BIDI`, `RTL` | `token.predicate.direction` |
| `colorIndex` | number | colour label number | `token.predicate.colorIndex` |
| `countOfLayers`, `countOfUnicodes` | number | `countOfUnicodes == 1` | `token.predicate.numeric` |
| `isAligned`, `hasHints`, `isColorGlyph` | bool | `hasHints == true` | `token.predicate.boolean` |
| `layer.anchors.<name>.x/.y`, `layer.maxX` | number | `layer.anchors.center.y > 250` | `token.predicate.keyPaths` |

The reference evaluates predicates against its object model, so more attributes exist than the
handbook lists. FontCraft implements the table above first and reports any other attribute as an
error naming the attribute.

Compiler support: fea-rs evaluates only `name` predicates with string operators
(`token.predicate.compilerSupport`). FontCraft therefore expands every predicate itself in step 4
and never passes `$[...]` through.

## 4. Syntax extensions

The reference accepts these additions to the AFDKO language. "Lowering" is what FontCraft emits.

| Extension | Meaning | Lowering | fea-rs accepts as is | Rows |
|---|---|---|---|---|
| `delete g;` / `del g;` | Remove every `g` | `sub g by NULL;` | no | `featureCode.delete`, `.deleteLowering` |
| `delete g' ctx;` | Remove `g` in context | `sub g' ctx by NULL;` | no | `featureCode.deleteContextual` |
| `standalone lookup n { } n;` inside a feature | Define a lookup at this position in lookup order without registering it in the feature | close the feature block, define the lookup at top level, reopen the feature with its script, language and lookupflag statements repeated | no | `featureCode.standaloneLookup`, `.standaloneRedundant`, `.standaloneLowering` |
| `lookup n { ... };` | Closing name omitted | add the name | no | `featureCode.optionalLookupEndName` |
| `language A B C;` | Rules that follow apply to each language | repeat statement and rules per language, or pass through | yes | `featureCode.multipleLanguages`, `.multipleLanguagesCompiler` |
| `sub @A by @B @C;` (also contextual) | One-to-many with classes paired by position; equal sizes, singletons repeat | expand to one rule per member, or pass through | yes (classes on both sides) | `featureCode.multipleSubstitutionClasses`, `.multipleSubstitutionSingletons` |
| `lookupflag;` | Clear all flags | `lookupflag 0;` | no | `featureCode.emptyLookupflag` |
| `# Automatic Code` | Splice point for an implicit feature | handled in step 7 | yes (reported to the caller as an insertion marker) | `fontInfo.features.automaticCodePlaceholder` |

`featureCode.lookupflagReplaces` restates standard behaviour: each lookupflag statement replaces
the previous flags.

Contextual `delete` and class-based multiple substitution inside a context rely on in-line multiple
substitution in chaining rules, which fea-rs supports (`contextual_gsub2` tests).

## 5. Conditional feature code and variable positioning

### `#ifdef VARIABLE`

- `#ifdef VARIABLE ... #endif`: kept for the variable font only. `#ifndef VARIABLE ... #endif`:
  kept for static fonts only (`featureCode.ifdefVariable`, `.ifndefVariable`).
- The directives stand on their own lines. Only the symbol VARIABLE is documented; no `#else`,
  no nesting (`featureCode.ifdefOtherSymbols`).
- Because `#` starts a comment in standard syntax, a tool that does not know the directives sees
  both branches. FontCraft always resolves them before compiling.

### `condition`

```
#ifdef VARIABLE
condition 450 < wght < 800, 60 < wdth < 85;
sub yen by yen.heavynarrow;
#endif
```

- `condition range { "," range } ";"` with `range = [ min "<" ] tag [ "<" max ]`; at least one limit.
  All ranges must hold (`featureCode.condition`, `.conditionRange`, `.conditionMultipleAxes`).
- The lower limit is inclusive although written `<` (`featureCode.conditionLowerInclusive`).
- The rules after the condition apply only inside the region: an OpenType FeatureVariations record
  for the enclosing feature.
- Lowering (`featureCode.conditionLowering`): one `conditionset` per distinct condition and a
  `variation <feature tag> <set> { rules } <feature tag>;` block. Both are fea-rs / feaLib syntax,
  not part of AFDKO spec 1.27 (`fea.var.conditionset`, `fea.var.variation`).
- Scope and coordinate scale are undocumented (`featureCode.conditionScope`, open question).

### Variable GPOS

```
pos @Lowercase 8 (wght:700) 14 (wght:300 opsz:10) 22;
pos @Figure period' <12 0 24 0 (wght:700) 18 0 36 0 (wght:300 opsz:10) 6 0 12 0> @Figure;
```

- First value: default location. Each `(tag:coord ...)` group names a location in internal axis
  coordinates and is followed by the value there, one number or four inside a four-value record
  (`featureCode.variablePos`, `.variablePosMultiple`, `.variablePosFourValue`,
  `.variablePosCoordinates`).
- Lowering (`featureCode.variablePosLowering`): `(8 wght=700d:14 wght=300d,opsz=10d:22)` and, for
  records, either a variable value per metric or the per-location form (`fea.var.scalar`,
  `fea.var.valueRecordPerMetric`). fea-rs takes the unit suffix as optional; check which scale it
  assumes without one before relying on it.
- Static instance: interpolate to plain numbers.

## 6. Feature code across masters and instances

- One code base for all masters (`featureCode.mm.singleSource`). What varies per target: token
  values, implicit features, automatic entries regenerated for the instance's glyph set, and the
  instance parameters Add/Remove/Replace Prefix, Class, Feature.
- Static versus variable (`featureCode.mm.staticVersusVariable`):

| | Static instance | Variable font |
|---|---|---|
| `#ifdef VARIABLE` block | removed | kept |
| `#ifndef VARIABLE` block | kept | removed |
| `$number`, `${glyph:prop}` | interpolated number | variable value over master locations |
| `pos ... (axis:coord) value` | interpolated number | variable value |
| `condition` | not reachable (inside `#ifdef`) | feature variation |
| kern, mark, mkmk, curs | from interpolated kerning and anchors | variable values from all masters |

- Alternate layers in variable fonts use the feature from the Feature for Feature Variations
  parameter (see `10-interpolation`); hand-written conditions use the feature they sit in.

## 7. Implicit features

kern, dist, mark, abvm, blwm, mkmk and curs come from kerning and anchors, are produced per target
and never appear in the Features tab (`fontInfo.features.implicit`). Rows: `autoFeature.kern`,
`.dist`, `.mark`, `.abvm`, `.blwm`, `.mkmk`, `.curs`.

- Extending one: add a feature with the same tag and put `# Automatic Code` where the generated
  code belongs; code above it is emitted first, code below it after
  (`fontInfo.features.automaticCodePlaceholder`). Without the marker: a kern feature gets the generated lookups after its
  own code, and a mark-family feature gets none of the generated feature, as both reference
  compilers do (`implicit.markerMissing`).
- kern: kerning groups appear as `@MMK_L_<group>` (glyph on the left of the pair) and
  `@MMK_R_<group>` (glyph on the right); contextual kerning goes into an explicit kern feature
  (rows in `08-spacing-kerning`).
- mark / mkmk: anchors `name` on bases and `_name` on marks; marks with both feed mkmk. Marks are
  exported with zero advance (`implicit.markZeroWidth`). ccmp is generated alongside when dotless
  forms exist (`anchors.ccmpAlongsideMark`), and unlike mark it is listed in the tab.
- GDEF: ligature carets come from `caret_*` anchors (Edit View chapter); glyph classes are expected
  to come from glyph categories (inferred). The user writes no GDEF block.
- With fea-rs, implicit features are supplied through its feature-provider hook, which inserts
  externally built lookups at the marker positions (provider_* compile tests).

## 8. Automatic feature generation

General rule (`autoFeature.generalRule`): a dot suffix, or a feature tag used as suffix, yields a
substitution from the unsuffixed glyph to the suffixed one; names joined by underscores yield
ligatures; some features need particular glyphs or anchors. Chained suffixes follow the order of
the features (x.sc.ss01). Generation needs standard glyph names.

Rules common to all suffix features (inferred unless the handbook is cited in the table):
- The source glyph is the name with the suffix removed; no rule is generated when it is missing.
- Rules of one feature form one single-substitution lookup, written class-to-class when more than
  a few glyphs are involved.
- Only exporting glyphs take part; the result is recomputed per instance.
- Entries appear on `font.features.update`; cpsp and the automatic classes only when added by hand.

† = the handbook gives only the trigger; the rule shape is our inference and must be checked
against the reference's output.

| Tag | Feature | Trigger (glyph naming or data) | Generated code | Listed in Features tab |
|---|---|---|---|---|
| `aalt` | All Alternatives | At least one other feature in the font substitutes glyphs. | Built from every substituting feature in the list; expected form is one `feature <tag>;` line per such feature, leaving the grouping to the compiler †. | yes |
| `liga` | Standard Ligatures | A ligature glyph, i.e. a name made of component names joined by underscores. Only f_f_i, f_f_l, f_f, fi, fl, lu_lakkhangyao-thai and ru_lakkhangyao-thai land here by default; any other ligature is forced in by a `.liga` suffix (f_b.liga, yi_yi-cy.liga). | One ligature rule per glyph, `sub <components> by <ligature>;`, with the components read from the name parts †. | yes |
| `dlig` | Discretionary Ligatures | Any underscore-joined ligature name that no other ligature feature claims, e.g. f_odieresis. | One ligature rule per glyph from its name parts †. | yes |
| `hlig` | Historical Ligatures | Ligatures that contain longs as a component, or ligature names ending in `.hlig` or `.hist`. | One ligature rule per glyph from its name parts †. | yes |
| `rlig` | Required Ligatures | Ligature names ending in `.rlig`, and the Arabic lam-alef set: lam_alef-ar, lam_alefHamzaabove-ar, lam_alefHamzabelow-ar, lam_alefMadda-ar, lam_alefWasla-ar and the `.fina` form of each. | One ligature rule per glyph from its name parts †. | yes |
| `c2sc` | Small Capitals from Capitals | Glyphs ending in `.sc`, `.c2sc` or `.smcp`. To keep two separate sets, put `.c2sc` on uppercase base names and `.smcp` on lowercase base names. | Single substitutions from each capital to its small-cap glyph; with a shared `.sc` set the capital maps to the small cap named after the lowercase letter †. | yes |
| `smcp` | Small Capitals | Glyphs ending in `.sc` or `.smcp`. | Single substitutions `sub a by a.sc;` from each base glyph to its suffixed form. | yes |
| `c2pc` | Petite Capitals from Capitals | Glyphs ending in `.pc`, `.c2pc` or `.pcap`. | Single substitutions from each capital to its petite-cap glyph †. | yes |
| `pcap` | Petite Capitals | Glyphs ending in `.pc` or `.pcap`. | Single substitutions from each base glyph to its suffixed form †. | yes |
| `sups` | Superscript | Glyphs ending in `.sups`, or figure names with `superior` appended and no period (onesuperior). | Single substitutions from the default glyph to the superior form †. | yes |
| `subs` | Subscript | Names with `inferior` appended, or ending in `.sinf` or `.subs`. A single `.subs` set is enough: it feeds both subs and sinf. | Single substitutions from the default glyph to the inferior form †. | yes |
| `sinf` | Scientific Inferiors | Same glyphs as subs: `inferior` names, `.sinf` or `.subs`. | Single substitutions from the default glyph to the inferior form †. | yes |
| `afrc` | Alternative Fractions | The font has figures, slash and at least one nut-fraction glyph, named as numerator word, `over`, denominator word, from the fixed list (oneovertwo, zerooverthree, oneoverthree ... tenovereleven, elevenovertwelve, oneoveronehundred). | One ligature-style rule per nut fraction, replacing the figure-slash-figure sequence with the fraction glyph †. | yes |
| `frac` | Fractions | `.numr` and `.dnom` figures plus the fraction glyph. Without them, the feature falls back to whatever precomposed fractions exist. | With numerators and denominators: contextual rules that turn figures before the fraction slash into numerators and figures after it into denominators, and slash into fraction; in the fallback case: ligature rules onto the precomposed fractions †. | yes |
| `dnom` | Denominators | Glyphs ending in `.dnom`. | Single substitutions from the default glyph to the `.dnom` form †. | yes |
| `numr` | Numerators | Glyphs ending in `.numr`. | Single substitutions from the default glyph to the `.numr` form †. | yes |
| `onum` | Oldstyle Figures | Glyphs ending in `.osf` (proportional oldstyle) or `.tosf` (tabular oldstyle); works for non-figures such as currency signs too. | Single substitutions from lining forms to the matching oldstyle forms, keeping the current width class †. | yes |
| `tnum` | Tabular Figures | Glyphs ending in `.tf` (tabular lining) or `.tosf` (tabular oldstyle); also for other characters. The default figure set carries no figure suffix at all. | Single substitutions from proportional forms to the matching tabular forms (`sub zero by zero.tf;`). | yes |
| `pnum` | Proportional Figures | Glyphs ending in `.osf` (proportional oldstyle) or `.lf` (proportional lining); also for other characters. | Single substitutions from tabular forms to the matching proportional forms †. | yes |
| `lnum` | Lining Figures | Glyphs ending in `.lf` (proportional lining) or `.tf` (tabular lining); also for other characters. | Single substitutions from oldstyle forms to the matching lining forms †. | yes |
| `ordn` | Ordinals | The default figures together with numero, ordfeminine and ordmasculine are present. | Contextual substitutions after figures that turn a and o into ordfeminine and ordmasculine, and a rule producing numero from N o period †. | yes |
| `ornm` | Ornaments | Glyphs named with `.ornm` on a base name from A-Z or a-z, and a bullet glyph in the font. | An alternate substitution offering all ornaments from bullet, plus single substitutions from each letter to its ornament †. | yes |
| `hist` | Historical Forms | Glyphs ending in `.hist`. | Single substitutions from the base glyph to the `.hist` form †. | yes |
| `case` | Uppercase Forms | Glyphs ending in `.case`, and figures ending in `.lf`. | Single substitutions from the default glyph to the `.case` form, and from default figures to lining figures †. | yes |
| `cpsp` | Capital Spacing | Uppercase letters exist and the user adds the feature by choosing Capital Spacing from the plus menu at the bottom left of the Features tab. Update never adds it by itself. | A single positioning rule on the uppercase class that widens both sides, of the form `pos @Uppercase <n 0 2n 0>;`; the amount is not documented †. | yes |
| `locl` | Localized Forms | Glyphs ending in `.loclXXX` where XXX is an OpenType language tag (`.loclENG`, `.loclSVE`). Built-in triggers: idotaccent, i.TRK or i.loclTRK with i (TRK, AZE, CRT, KAZ, TAT); Scommaaccent, Tcommaaccent and their lowercase with the cedilla forms present (ROM, MOL); periodcentered.loclCAT (plus `.case`), L_periodcentered_L.loclCAT, l_periodcentered_l.loclCAT or legacy Ldot, ldot with L, l and periodcentered (CAT); Iacute_J.loclNLD, iacute_j.loclNLD or Jacute, jacute with Iacute, iacute, J, j (NLD); six-ar and `.urdu` figures such as four-persian.urdu (URD). | Per script and language, a `script`/`language` section holding single substitutions to the localised glyphs (ligature or contextual rules for the Catalan and Dutch cases); languages sharing rules are listed together †. | yes |
| `cv01-cv99` | Character Variants | Glyphs ending in `.cv01` to `.cv99`. | One feature per number with single substitutions from base to variant. Several variants of one glyph belong in one feature and need manual code such as `sub k from [k.loop k.open k.tall];`. | yes |
| `ss01-ss20` | Stylistic Sets | Glyphs ending in `.ss01` to `.ss20`. | One feature per number with single substitutions `sub g by g.ss01;`; each set can carry localised names, written as a featureNames block. | yes |
| `salt` | Stylistic Alternates | An ss01 feature exists. | A copy of the ss01 rules; Adobe Illustrator and Photoshop read this feature for their OpenType panels. | yes |
| `swsh` | Swashes | Glyphs ending in `.swsh`. | Single substitutions from base to `.swsh` form †. | yes |
| `titl` | Titling | Glyphs ending in `.titl`. | Single substitutions from base to `.titl` form †. | yes |
| `init` | Initial Forms | Glyphs ending in `.init`. | Single substitutions from the base glyph to the `.init` form †. | yes |
| `medi` | Medial Forms | Glyphs ending in `.medi`. | Single substitutions from the base glyph to the `.medi` form †. | yes |
| `med2` | Medial Forms 2 | Glyphs ending in `.med2`; Syriac only. | Single substitutions from the base glyph to the `.med2` form †. | yes |
| `fina` | Terminal Forms | Glyphs ending in `.fina`. | Single substitutions from the base glyph to the `.fina` form †. | yes |
| `fin2` | Terminal Forms 2 | Glyphs ending in `.fin2`; Syriac only. | Single substitutions from the base glyph to the `.fin2` form †. | yes |
| `fin3` | Terminal Forms 3 | Glyphs ending in `.fin3`; Syriac only. | Single substitutions from the base glyph to the `.fin3` form †. | yes |
| `fwid` | Full Widths | Glyphs ending in `.full` or `.fullwidth`. | Single substitutions to the full-width forms, plus space replaced by emspace. | yes |
| `hkna` | Horizontal Kana Alternates | Katakana or Hiragana glyphs ending in `.hori`. | Single substitutions from the kana to the `.hori` form †. | yes |
| `hojo` | Hojo Kanji Forms | Glyphs ending in `.hojo`, holding JIS X 0212-1990 shapes while the unsuffixed glyphs hold JIS X 0213:2004 shapes. | Single substitutions from the default kanji to the `.hojo` form †. | yes |
| `hwid` | Half Widths | Glyphs ending in `.half`. | Single substitutions to the half-width forms, plus space replaced by enspace. | yes |
| `ital` | Italics | Glyphs ending in `.italic`. | Single substitutions from the base glyph to the `.italic` form †. | yes |
| `pkna` | Proportional Kana | Katakana or Hiragana glyphs ending in `.proportional`. | Single substitutions from the kana to the `.proportional` form †. | yes |
| `pwid` | Proportional Widths | Glyphs ending in `.proportional`. | Single substitutions from the base glyph to the `.proportional` form †. | yes |
| `qwid` | Quarter Widths | Glyphs ending in `.qartwidth` (the handbook's spelling; whether `.quartwidth` is also recognised is an open question). | Single substitutions from the base glyph to the quarter-width form †. | yes |
| `ruby` | Ruby Notation Forms | Glyphs ending in `.ruby`. | Single substitutions from the base glyph to the `.ruby` form †. | yes |
| `twid` | Third Widths | Glyphs ending in `.thirdwidth`. | Single substitutions from the base glyph to the third-width form †. | yes |
| `vkna` | Vertical Kana Alternates | Katakana or Hiragana glyphs ending in `.vert`. | Single substitutions from the kana to the `.vert` form †. | yes |
| `vrt2` | Vertical Alternates and Rotation | Glyphs ending in `.vert`. | Single substitutions from the base glyph to the `.vert` form †. | yes |
| `abvs` | Above-base Substitutions | Certain pairings of a base with a mark, in the scripts of South Asia. | Script-specific ligature rules joining base and above-base mark; the exact glyph lists are not documented and must be taken from the open GlyphData naming plus observation. | yes |
| `akhn` | Akhands | South Asian scripts only: a particular group of letters and ligatures (the classic akhand conjuncts). | Ligature rules of consonant, halant, consonant onto the akhand glyph †. | yes |
| `blwf` | Below Base Forms | South Asian scripts: a halant, glyphs suffixed `.below`, and various further glyphs present with them. | Ligature rules of halant plus consonant onto the `.below` form †. | yes |
| `blws` | Below-base Substitutions | South Asian scripts: particular sequences of letters, such as Gujarati glyphs suffixed `.straight`, or specific combinations with halant in Oriya. | Script-specific contextual or ligature rules; details are not documented. | yes |
| `cjct` | Conjunct Forms | Conjunct cluster glyphs in South Asian scripts (ligature names made of consonant names). | Ligature rules from half forms or consonant-halant sequences onto the conjunct †. | yes |
| `half` | Half Forms | Glyphs whose names end in `Halfform`, together with the script's halant. | Ligature rules of consonant plus halant onto the half form †. | yes |
| `nukt` | Nukta Forms | Nukta ligatures whose names end in `Nukta` plus the script suffix, provided the nukta-less counterparts exist too. | Ligature rules of consonant plus nukta onto the nukta form †. | yes |
| `pref` | Pre-base Forms | South and Southeast Asian scripts: particular letter combinations that include a halant. | Script-specific ligature rules of halant plus consonant onto a pre-base form; details are not documented. | yes |
| `pres` | Pre-base Substitutions | South Asian scripts: particular combinations of matras and conjuncts. Width variants of a matra are numbered, e.g. iMatra-deva.01 to iMatra-deva.20. | Contextual rules that pick, for each conjunct, the numbered matra variant whose width fits; the matching method is not documented. | yes |
| `psts` | Post-base Substitutions | A halant together with base and matra arrangements that depend on the script, e.g. in Kannada, eMatra-kannada together with glyphs suffixed `.base` and `.base.e`. South Asian scripts only. | Script-specific ligature or contextual rules; details are not documented. | yes |
| `rkrf` | Rakar Forms | Devanagari, Gujarati and Malayalam only: rakar ligatures present together with their isolated letters and the halant. | Ligature rules of consonant, halant, ra onto the rakar ligature †. | yes |
| `rphf` | Reph Forms | The trio ra-deva, halant-deva, reph-deva, or its counterpart in another South Asian script. | A ligature rule `sub ra-deva halant-deva by reph-deva;` per script †. | yes |
| `vatu` | Vattu Variants | The rkrf feature contains particular lookups. | Rules derived from the rakar lookups; details are not documented. | yes |
| `ccmp` | Glyph Composition and Decomposition | Many constellations across scripts. Documented example: i and j, their dotless forms idotless and jdotless, and combining marks that sit on top. | For the Latin case, a lookup that swaps the dotted letter for its dotless form when a top mark follows; other scripts add their own lookups, which are not documented. | yes |
| `mark` | Mark to Base Positioning | Base glyphs carry anchors without underscore (top, bottom) and combining marks carry the matching underscore anchors (_top, _bottom). | Built at export only and never listed in the Features tab: mark classes per anchor name and mark-to-base lookups from the anchor positions of each master or instance. Mark glyphs get zero advance width at export. | no (implicit) |
| `mkmk` | Mark to Mark Positioning | Combining marks (U+0300 and above) that carry both an underscore anchor and its plain counterpart (_top and top). | Built at export only and never listed in the Features tab: mark-to-mark lookups from those anchors. | no (implicit) |
| `kern` | Kerning | Kerning values exist in a master (defined in Edit View or the Kerning window). | Built at export per master or instance and never listed in the Features tab: pair positioning rules using group classes named `@MMK_L_<group>` and `@MMK_R_<group>`; glyph pairs before class pairs is the expected order (inferred). | no (implicit) |
| `dist` | Distances | Same data as kerning for scripts that use dist instead of kern; the user cannot steer it. | Built at export like kern and never listed in the Features tab; which scripts are routed to dist is not documented. | no (implicit) |
| `curs` | Cursive Positioning | Glyphs carry `entry` and `exit` anchors. | Built at export and never listed in the Features tab: cursive attachment rules with the entry and exit anchor of each glyph. | no (implicit) |
| `abvm` | Above-base Mark Positioning | Same anchor convention as mark, for glyphs of scripts that use abvm. | Built at export like mark and never listed in the Features tab; the split between mark, abvm and blwm is not documented. | no (implicit) |
| `blwm` | Below-base Mark Positioning | Same anchor convention as mark, for glyphs of scripts that use blwm. | Built at export like mark and never listed in the Features tab; the split between mark, abvm and blwm is not documented. | no (implicit) |

Automatic classes: the handbook names `All` and uses `@Uppercase` in an example; the full list is
an open question (`autoClass.general`). The corpus has `Uppercase`, `Lowercase`, `AllLetters` and
`All`, one glyph name per line in glyph order; FontCraft writes those four.

Languagesystems prefix (`autoFeature.languageSystems`): `languagesystem DFLT dflt;`, then for each
script the `locl` code names, a blank line, the script's `dflt` line and one line per language,
in the order `locl` names them. A font without `locl` gets the `DFLT` line alone.

Update's merge rules: manual entries stay as they are; an automatic entry takes its regenerated
code, or is removed when its glyphs are gone; a missing entry is added in the reference's feature
order, unless it is optional (cpsp, the classes); aalt and the Languagesystems prefix are computed
last, from the merged list. The measure against the reference's stored code is
`crates/features/tests/corpus_auto.rs`.

## 9. Text tokens

`{{{token}}}` in supporting text fields is replaced when the field is used (`textToken.*`,
`font.expandTextTokens`).

| Token | Context | Result |
|---|---|---|
| `versionString` | any | application version |
| `buildNumber` | any | application build number |
| key path, e.g. `fullName` | font or instance | property of the font or instance |
| number value name | master | that master's number value |
| `glyph:path`, e.g. `space:width` | font, master or instance | layer property of the glyph, as in section 3 |

The reference resolves key paths through its scripting API. FontCraft defines its own list of
paths (family name, style name, full name, version, designer and the other Font Info fields) and
reports unknown paths; fields that accept tokens are listed in `fontInfo.general.textTokens`.

## 10. Regular expressions

Used where a field has a Regex option (Font View search: `fontView.search.regex`; Find and Replace:
`findReplace.regexMode`).

| Syntax | Meaning | Row |
|---|---|---|
| `.` | any character | `regex.anyCharacter` |
| `[xyz]`, `[^xyz]` | one of / none of | `regex.characterClass` |
| `\d`, `\D` | digit / non-digit | `regex.digit` |
| `\w`, `\W` | letter, digit or underscore / anything else | `regex.wordCharacter` |
| `?`, `*`, `+` | 0-1, 0+, 1+ of the preceding item | `regex.quantifiers` |
| `{n}`, `{n,m}`, `{n,}` | counted repetition | `regex.countedRepetition` |
| `\.` `\?` ... | literal special character | `regex.escape` |
| `( )` with `\1`, `\2` in the replacement | captured parts | `regex.captureReplace` |

Search patterns must match the whole glyph name; find-and-replace patterns match any part of it.
Engine: Rust `regex`; `\N` in replacements is rewritten to `${N}` (`regex.engine`).

## 11. URL scheme

The reference registers a generic and a version-specific scheme with one documented action, `show`.
FontCraft proposal (`url.*`, all inferred):

```
fontcraft://show/?path=/abs/One.fontcraft&glyph=A&layer=<id>&layer=<id>&glyph=B
                 &path=/abs/Two.glyphs&glyph=Gbreve&production=uni01F4
```

- `path` (absolute, repeatable) starts a file; the `glyph` / `production` entries after it list
  glyphs by name or production name; `layer` entries after a glyph pick layers by ID (a master
  layer's ID is the master ID); no `layer` means the main master's layer.
- Each file is opened unless already open, then gets one new tab with the listed layers in order.
- `url.open` parses and dispatches; `url.show` is the action and can be called directly with a
  structured argument. Values are percent-decoded; unknown glyphs or layers are skipped with a
  warning. URLs never run other commands (`url.safety`).

## 12. Snippets

- Menu behind the snippets button: built-in snippets (including `# Automatic Code`) and one group
  per user file (`fontInfo.featureSnippetsMenu`, `font.features.listSnippets`,
  `font.features.insertSnippet`).
- User files: `FeatureSnippets<NAME>.plist` in the snippet folder (`font.features.showSnippetFolder`);
  content is an array of dictionaries with `title` and `code` (`snippet.fileNaming`,
  `snippet.fileFormat`). FontCraft also reads the same structure as TOML (`snippet.nativeFormat`).
- Insertion at the caret, replacing the selection (`snippet.insertion`, inferred).

## 13. The feature-file language in brief

Full rows: `spec/inventory/12b-feature-language.toml`. Section numbers refer to the AFDKO
specification v1.27.

- **Lexical** (§2): `#` comments; free white space; `;` ends statements; glyph names (63 chars,
  letters, digits, `.`, `_`; development names add `* + - : ^ | ~`); `\name` escapes keywords,
  `\123` is a CID; tags up to four characters; numbers decimal, with octal and hex in name and OS/2
  fields.
- **Classes** (§2.g): `[a b c]`, ranges `[A.sc - Z.sc]`, `[x.01 - x.58]`, named `@name = [...]`,
  nesting, alias `@a = @b;`.
- **Values** (§2.e): value record as one number, `<xPla yPla xAdv yAdv>`, with devices, `<NULL>`,
  or `<NAME>` from `valueRecordDef`; anchors `<anchor x y>`, with `contourpoint`, with devices,
  `<anchor NULL>`, `<anchor NAME>` from `anchorDef`.
- **Files** (§3): `include(path);` anywhere statements are allowed, resolved against UFO folder,
  main file, including file; depth limit 50.
- **Features** (§4): `feature tag { } tag;`; `languagesystem script lang;` at the top (implicit
  `DFLT dflt`); inside a feature `script`, `language [exclude_dflt|include_dflt] [required]`;
  `lookupflag` named or numeric; `lookup name [useExtension] { } name;` inside or outside features
  and `lookup name;` references; `markClass glyphs <anchor> @class;`; `subtable;`.
- **GSUB** (§5): type 1 `sub a by b;` (class forms; `by NULL` deletes); type 2 `sub a by b c;`;
  type 3 `sub a from [..];`; type 4 `sub a b by c;`; types 5/6 by marking input with `'`, with
  `lookup name` references or one in-line single/ligature substitution; `ignore sub`; type 7 via
  `useExtension`; type 8 `rsub`.
- **GPOS** (§6): type 1 `pos a <vr>;`; type 2 `pos a b n;` or with two records, `enum pos`,
  class pairs when any side is a class; type 3 `pos cursive g <entry> <exit>;`; type 4
  `pos base g <anchor> mark @c ...;`; type 5 `pos ligature g ... ligComponent ...;`; type 6
  `pos mark g <anchor> mark @c;`; types 7/8 with marked input, lookup references or in-line
  records, in-line cursive and mark forms; `ignore pos`; type 9 via `useExtension`.
- **Ordering** (§7): lookups in file order, aalt first; rule order matters only in chaining
  lookups; ligatures sorted longest first.
- **Special features** (§8): `aalt` (`feature tag;` references plus explicit rules), `size`
  (`parameters`, `sizemenuname`), `featureNames` in ssXX, `cvParameters` in cvXX.
- **Tables** (§9): BASE, GDEF, head, hhea, name, OS/2, vhea, vmtx, STAT blocks.
- **Anonymous** (§10): `anon tag { raw } tag;` passed through unparsed.
- **Variable** (§2.e.iia onward, Variable Feature Guide): locations `wght=700d, opsz=12u`
  (units `d`, `u`, `n`, optional `+`/`-` nudge), `locationDef ... @name;`, variable values
  `(default loc:value ...)` in value records, anchors and table fields. Feature variations
  (`conditionset`, `variation`) exist only in feaLib and fea-rs.

### Compliance checklist

177 constructs. fea-rs status from its sources and tests: 154 yes, 7 partial, 5 no,
11 unknown. Tick a box when FontCraft has a passing test for the construct.

**Lexical structure**

- [ ] `fea.comment` — `#` comments · fea-rs: yes
- [ ] `fea.whitespace` — Free-form white space · fea-rs: yes
- [ ] `fea.keywords` — Global keyword set · fea-rs: yes
- [ ] `fea.statementTerminator` — Semicolon and punctuation · fea-rs: yes
- [ ] `fea.number` — Integer literal · fea-rs: yes
- [ ] `fea.numberRadix` — Octal and hexadecimal numbers · fea-rs: yes
- [ ] `fea.float` — Decimal fractions · fea-rs: yes
- [ ] `fea.metric` — Metric value · fea-rs: yes
- [ ] `fea.glyphName` — Production glyph names · fea-rs: yes
- [ ] `fea.glyphNameDevelopment` — Development glyph names · fea-rs: yes
- [ ] `fea.glyphNameEscape` — Backslash-escaped glyph names · fea-rs: yes
- [ ] `fea.glyphNameVsRange` — Hyphenated name or range · fea-rs: yes
- [ ] `fea.cid` — CID glyph references · fea-rs: yes
- [ ] `fea.tag` — Bare tags · fea-rs: yes
- [ ] `fea.tagQuoted` — Quoted tags · fea-rs: unknown
- [ ] `fea.tagDefaults` — DFLT and dflt · fea-rs: yes
- [ ] `fea.lookupLabel` — Lookup block labels · fea-rs: yes

**Glyph classes**

- [ ] `fea.glyphClass` — Bracketed glyph class · fea-rs: yes
- [ ] `fea.glyphRange.letter` — Letter ranges · fea-rs: yes
- [ ] `fea.glyphRange.numeric` — Numeric ranges · fea-rs: yes
- [ ] `fea.glyphRange.cid` — CID ranges · fea-rs: yes
- [ ] `fea.glyphRange.missingMembers` — Ranges skip missing glyphs · fea-rs: unknown
- [ ] `fea.namedGlyphClass` — Named glyph class definition · fea-rs: yes
- [ ] `fea.namedGlyphClass.nesting` — Classes inside classes · fea-rs: yes
- [ ] `fea.namedGlyphClass.alias` — Class alias without brackets · fea-rs: yes
- [ ] `fea.classSharing` — Shared coverage and class definitions · fea-rs: unknown

**Value records, anchors and devices**

- [ ] `fea.valueRecord.single` — Value record format A · fea-rs: yes
- [ ] `fea.valueRecord.verticalSingle` — Single value in vertical features · fea-rs: yes
- [ ] `fea.valueRecord.four` — Value record format C · fea-rs: yes
- [ ] `fea.valueRecord.device` — Value record with device tables · fea-rs: yes
- [ ] `fea.valueRecord.null` — Null value record · fea-rs: yes
- [ ] `fea.valueRecord.named` — Named value record reference · fea-rs: yes
- [ ] `fea.valueRecordDef` — valueRecordDef statement · fea-rs: yes
- [ ] `fea.device` — Device table · fea-rs: yes
- [ ] `fea.device.null` — Null device · fea-rs: yes
- [ ] `fea.contourPoint` — Contour point reference · fea-rs: yes
- [ ] `fea.anchor.coordinates` — Anchor format A · fea-rs: yes
- [ ] `fea.anchor.contourPoint` — Anchor format B · fea-rs: yes
- [ ] `fea.anchor.device` — Anchor format C · fea-rs: yes
- [ ] `fea.anchor.null` — Null anchor · fea-rs: yes
- [ ] `fea.anchor.named` — Named anchor reference · fea-rs: yes
- [ ] `fea.anchorDef` — anchorDef statement · fea-rs: yes

**Including files**

- [ ] `fea.include` — include directive · fea-rs: yes
- [ ] `fea.include.contexts` — Where include is allowed · fea-rs: yes
- [ ] `fea.include.resolution` — Relative path resolution · fea-rs: yes
- [ ] `fea.include.depth` — Include depth limit · fea-rs: yes

**Features, language systems and lookups**

- [ ] `fea.featureBlock` — feature block · fea-rs: yes
- [ ] `fea.rule.autoType` — Lookup type inferred from rule shape · fea-rs: yes
- [ ] `fea.lookup.implicit` — Implicit lookups from runs of rules · fea-rs: yes
- [ ] `fea.languagesystem` — languagesystem statement · fea-rs: yes
- [ ] `fea.languagesystem.default` — Implicit DFLT dflt · fea-rs: yes
- [ ] `fea.languagesystem.order` — Ordering of languagesystem statements · fea-rs: yes
- [ ] `fea.script` — script statement · fea-rs: yes
- [ ] `fea.language` — language statement · fea-rs: yes
- [ ] `fea.language.defaultInheritance` — Two levels of default lookups · fea-rs: yes
- [ ] `fea.language.excludeDflt` — exclude_dflt · fea-rs: yes
- [ ] `fea.language.includeDflt` — include_dflt · fea-rs: yes
- [ ] `fea.language.deprecatedKeywords` — excludeDFLT and includeDFLT · fea-rs: yes
- [ ] `fea.language.required` — required · fea-rs: yes
- [ ] `fea.parameters` — parameters statement · fea-rs: yes
- [ ] `fea.lookupflag.named` — lookupflag with named flags · fea-rs: yes
- [ ] `fea.lookupflag.markAttachmentType` — MarkAttachmentType · fea-rs: yes
- [ ] `fea.lookupflag.useMarkFilteringSet` — UseMarkFilteringSet · fea-rs: yes
- [ ] `fea.lookupflag.numeric` — lookupflag with a number · fea-rs: yes
- [ ] `fea.lookupflag.scope` — Lifetime of the lookup flag · fea-rs: yes
- [ ] `fea.lookupBlock` — Named lookup block · fea-rs: yes
- [ ] `fea.lookupBlock.standalone` — Standalone lookups · fea-rs: yes
- [ ] `fea.lookupBlock.useExtension` — useExtension on a lookup · fea-rs: yes
- [ ] `fea.lookupReference` — Lookup reference statement · fea-rs: yes
- [ ] `fea.markClass` — markClass statement · fea-rs: yes
- [ ] `fea.markClass.rules` — Mark class restrictions · fea-rs: yes
- [ ] `fea.markClass.gdefInference` — GDEF built from mark classes · fea-rs: yes
- [ ] `fea.subtable` — subtable statement · fea-rs: yes

**GSUB rules**

- [ ] `fea.gsub1.glyphByGlyph` — Single substitution, glyph by glyph · fea-rs: yes
- [ ] `fea.gsub1.classByGlyph` — Single substitution, class by glyph · fea-rs: yes
- [ ] `fea.gsub1.classByClass` — Single substitution, class by class · fea-rs: yes
- [ ] `fea.gsub1.null` — Deleting a glyph · fea-rs: yes
- [ ] `fea.gsub2.multiple` — Multiple substitution · fea-rs: yes
- [ ] `fea.gsub3.alternate` — Alternate substitution · fea-rs: yes
- [ ] `fea.gsub4.ligature` — Ligature substitution · fea-rs: yes
- [ ] `fea.gsub4.classEnumeration` — Classes in ligature sequences · fea-rs: yes
- [ ] `fea.gsub4.sorting` — Automatic ligature ordering · fea-rs: yes
- [ ] `fea.gsub5.contextual` — Contextual substitution · fea-rs: yes
- [ ] `fea.gsub6.marking` — Chaining context and marked input · fea-rs: yes
- [ ] `fea.gsub6.lookupReferences` — Explicit lookups at input positions · fea-rs: yes
- [ ] `fea.gsub6.inlineSingle` — In-line single substitution in context · fea-rs: yes
- [ ] `fea.gsub6.inlineLigature` — In-line ligature substitution in context · fea-rs: yes
- [ ] `fea.gsub6.ignore` — ignore substitute · fea-rs: yes
- [ ] `fea.gsub6.ignoreList` — Comma-separated ignore sequences · fea-rs: yes
- [ ] `fea.gsub7.extension` — Extension substitution · fea-rs: yes
- [ ] `fea.gsub8.reverse` — Reverse chaining single substitution · fea-rs: yes
- [ ] `fea.gsub8.null` — Reverse substitution by NULL · fea-rs: no

**GPOS rules**

- [ ] `fea.gpos1.single` — Single adjustment · fea-rs: yes
- [ ] `fea.gpos2.formatA` — Pair adjustment with two value records · fea-rs: yes
- [ ] `fea.gpos2.formatB` — Pair adjustment, kerning form · fea-rs: yes
- [ ] `fea.gpos2.classVsSpecific` — Class pairs versus glyph pairs · fea-rs: yes
- [ ] `fea.gpos2.enum` — enum pos · fea-rs: yes
- [ ] `fea.gpos2.conflicts` — Duplicate glyph pairs · fea-rs: yes
- [ ] `fea.gpos2.subtableBreaks` — Class kerning subtables · fea-rs: yes
- [ ] `fea.gpos3.cursive` — Cursive attachment · fea-rs: yes
- [ ] `fea.gpos4.markToBase` — Mark-to-base attachment · fea-rs: yes
- [ ] `fea.gpos5.markToLigature` — Mark-to-ligature attachment · fea-rs: yes
- [ ] `fea.gpos5.nullComponent` — Component without marks · fea-rs: yes
- [ ] `fea.gpos6.markToMark` — Mark-to-mark attachment · fea-rs: yes
- [ ] `fea.gpos7.contextual` — Contextual positioning · fea-rs: yes
- [ ] `fea.gpos8.marking` — Chaining positioning context · fea-rs: yes
- [ ] `fea.gpos8.lookupReferences` — Explicit lookups in positioning context · fea-rs: yes
- [ ] `fea.gpos8.inlineSingle` — In-line value records in context · fea-rs: yes
- [ ] `fea.gpos8.pairShorthand` — Contextual kern pair shorthand · fea-rs: yes
- [ ] `fea.gpos8.inlineCursive` — In-line cursive rule in context · fea-rs: unknown
- [ ] `fea.gpos8.inlineMark` — In-line mark attachment in context · fea-rs: unknown
- [ ] `fea.gpos8.ignore` — ignore position · fea-rs: yes
- [ ] `fea.gpos9.extension` — Extension positioning · fea-rs: yes

**Ordering**

- [ ] `fea.order.lookups` — Lookup order follows the file · fea-rs: yes
- [ ] `fea.order.aaltFirst` — aalt lookups come first · fea-rs: yes
- [ ] `fea.order.rules` — Rule order inside a lookup · fea-rs: yes
- [ ] `fea.order.subtables` — Subtable order · fea-rs: unknown

**Specially handled features**

- [ ] `fea.aalt.featureReference` — aalt built from other features · fea-rs: yes
- [ ] `fea.aalt.extraRules` — Explicit rules in aalt · fea-rs: yes
- [ ] `fea.aalt.grouping` — aalt output shape · fea-rs: partial
- [ ] `fea.aalt.restrictions` — What aalt may not contain · fea-rs: yes
- [ ] `fea.size.parameters` — size feature parameters · fea-rs: yes
- [ ] `fea.size.sizemenuname` — sizemenuname · fea-rs: yes
- [ ] `fea.size.restrictions` — size feature restrictions · fea-rs: yes
- [ ] `fea.featureNames` — featureNames block · fea-rs: yes
- [ ] `fea.cvParameters` — cvParameters block · fea-rs: yes
- [ ] `fea.cvParameters.character` — Character entries · fea-rs: yes

**Table blocks**

- [ ] `fea.table.block` — table block · fea-rs: yes
- [ ] `fea.table.base.tagList` — BASE BaseTagList · fea-rs: yes
- [ ] `fea.table.base.scriptList` — BASE BaseScriptList · fea-rs: yes
- [ ] `fea.table.base.minMax` — BASE MinMax · fea-rs: partial
- [ ] `fea.table.gdef.glyphClassDef` — GDEF GlyphClassDef · fea-rs: yes
- [ ] `fea.table.gdef.attach` — GDEF Attach · fea-rs: yes
- [ ] `fea.table.gdef.ligatureCaretByPos` — GDEF LigatureCaretByPos · fea-rs: yes
- [ ] `fea.table.gdef.ligatureCaretByIndex` — GDEF LigatureCaretByIndex · fea-rs: yes
- [ ] `fea.table.gdef.ligatureCaretByDev` — GDEF LigatureCaretByDev · fea-rs: no
- [ ] `fea.table.gdef.derivedClasses` — GDEF mark attachment classes and mark sets · fea-rs: yes
- [ ] `fea.table.head.fontRevision` — head FontRevision · fea-rs: yes
- [ ] `fea.table.hhea` — hhea fields · fea-rs: partial
- [ ] `fea.table.name.nameid` — name table records · fea-rs: yes
- [ ] `fea.table.name.reserved` — Reserved name IDs · fea-rs: unknown
- [ ] `fea.table.name.strings` — Name string syntax · fea-rs: yes
- [ ] `fea.table.os2.fsType` — OS/2 FSType · fea-rs: yes
- [ ] `fea.table.os2.panose` — OS/2 Panose · fea-rs: yes
- [ ] `fea.table.os2.unicodeRange` — OS/2 UnicodeRange · fea-rs: yes
- [ ] `fea.table.os2.codePageRange` — OS/2 CodePageRange · fea-rs: yes
- [ ] `fea.table.os2.typoMetrics` — OS/2 typo metrics · fea-rs: yes
- [ ] `fea.table.os2.winMetrics` — OS/2 win metrics · fea-rs: yes
- [ ] `fea.table.os2.heights` — OS/2 XHeight and CapHeight · fea-rs: yes
- [ ] `fea.table.os2.classes` — OS/2 WeightClass, WidthClass, FamilyClass · fea-rs: yes
- [ ] `fea.table.os2.vendor` — OS/2 Vendor · fea-rs: yes
- [ ] `fea.table.os2.opticalSize` — OS/2 LowerOpSize and UpperOpSize · fea-rs: yes
- [ ] `fea.table.os2.scriptAndStrikeout` — OS/2 subscript, superscript and strikeout · fea-rs: no
- [ ] `fea.table.vhea` — vhea fields · fea-rs: partial
- [ ] `fea.table.vmtx` — vmtx overrides · fea-rs: yes
- [ ] `fea.table.stat.elidedFallbackName` — STAT ElidedFallbackName · fea-rs: yes
- [ ] `fea.table.stat.designAxis` — STAT DesignAxis · fea-rs: yes
- [ ] `fea.table.stat.axisValue` — STAT AxisValue · fea-rs: yes
- [ ] `fea.table.stat.locationFormats` — STAT location statements · fea-rs: yes
- [ ] `fea.anonymous` — Anonymous data block · fea-rs: partial
- [ ] `fea.legacy.except` — Legacy except clause · fea-rs: no
- [ ] `fea.legacy.bareValueRecord` — Legacy unbracketed value record · fea-rs: unknown

**Variable-font extensions**

- [ ] `fea.var.location` — Location literal · fea-rs: yes
- [ ] `fea.var.locationUnits` — Axis value units · fea-rs: partial
- [ ] `fea.var.locationNudge` — Smallest step above or below · fea-rs: unknown
- [ ] `fea.var.locationDef` — Named locations · fea-rs: no
- [ ] `fea.var.scalar` — Variable single value · fea-rs: yes
- [ ] `fea.var.valueRecordPerMetric` — Four-value record with variable members · fea-rs: yes
- [ ] `fea.var.valueRecordPerLocation` — Whole records per location · fea-rs: unknown
- [ ] `fea.var.valueRecordDef` — Variable named value records · fea-rs: yes
- [ ] `fea.var.anchorPerMetric` — Anchor with variable coordinates · fea-rs: yes
- [ ] `fea.var.anchorPerLocation` — Anchor pairs per location · fea-rs: unknown
- [ ] `fea.var.anchorDef` — Variable named anchors · fea-rs: yes
- [ ] `fea.var.tableValues` — Variable table fields · fea-rs: partial
- [ ] `fea.var.conditionset` — conditionset block · fea-rs: yes
- [ ] `fea.var.variation` — variation block · fea-rs: yes
- [ ] `fea.var.variationNull` — variation with NULL · fea-rs: yes

### Where fea-rs falls short of the specification

| Row | fea-rs | Detail |
|---|---|---|
| `fea.gsub8.null` | no | rsub_null is an expected parse error |
| `fea.aalt.grouping` | partial | the fontTools spec8a_2 comparison is on its ignore list |
| `fea.table.base.minMax` | partial | parsed, minmax parse test; the compile step is marked not yet implemented |
| `fea.table.gdef.ligatureCaretByDev` | no | the statement is parsed and discarded |
| `fea.table.hhea` | partial | CaretOffset, Ascender, Descender and LineGap only; the caret slope keywords are not in its grammar |
| `fea.table.os2.scriptAndStrikeout` | no | these keywords do not appear in its grammar |
| `fea.table.vhea` | partial | the three VertTypo fields; caret fields unknown |
| `fea.anonymous` | partial | the block is parsed and skipped, spec10 test; handing the content to a caller is not shown |
| `fea.legacy.except` | no | no such keyword |
| `fea.var.locationUnits` | partial | the unit suffix is optional and described as experimental; how a value without unit is read is for the caller's variation info to decide |
| `fea.var.locationDef` | no | keyword not present |
| `fea.var.tableValues` | partial | parsed; the validator rejects variable metrics in several table blocks as not yet supported |

Glyphs extensions fea-rs lacks, to be handled by FontCraft's lowering step or contributed
upstream: `delete`, `standalone`, optional lookup end name, empty `lookupflag;`, `#ifdef`,
`condition`, the `(axis:coord) value` positioning form, glyph property tokens, and predicates
beyond `name`.

## Open questions

Editor
1. Which master does Option-click evaluation of a token use (the first master, the master selected
   in the main window, or all masters)?
   **Decision (M8 planning pass, guess on `token.numberValue.inspect`):** the master of the active
   Edit View tab, else the first master; to check in the reference app (`docs/plans/M8.md` § 8).
2. Does typing into an automatic entry switch it to manual, or is the editor read-only until the
   checkbox is cleared?
   **Decision:** the editor is read-only while Generate Feature Automatically is on, as the handbook
   has the user untick the box before editing; `font.features.setCode` refuses an automatic entry
   (guess on that row).
3. Which errors carry a Fix, and what do the fixes do?
   **Decision:** three fixes of ours: a glyph name that differs from an existing one only in case,
   a missing semicolon at the end of a statement, and a class used before it is defined (move the
   class entry up). Open for the reference's list (§ 8 of the plan).
4. Can sidebar entries be reordered by dragging, and is there a way to disable an entry without
   deleting it?
   **Partly answered:** the file format stores a `disabled` flag on prefixes, classes and features
   (corpus: 8 disabled entries, all in fixtures); both reference compilers leave disabled entries
   out. Dragging is our guess (`font.features.reorder`).
5. The built-in snippet list and the location of the snippet folder.
   **Decision:** our own list and folder (guesses on `snippet.builtIn` and
   `font.features.showSnippetFolder`); to check in the reference app.

Tokens
6. Full list of predicate attributes, operators and their spelling variants; precedence of AND and
   OR; whether keywords are case-sensitive.
   **Answered from glyphsLib's `builder/tokens.py` (oracle) and the predicate format the
   reference's scripting layer is built on (Apple's documented predicate format strings):**
   comparators `==` `=` `!=` `<>` `<` `<=` `=<` `>` `>=` `=>` `beginswith` `contains` `endswith`
   `like` `matches` `in` `between`, keywords in any case, `AND`/`&&`, `OR`/`||`, `NOT`/`!`, values
   in double or single quotes, `true`/`yes`/`false`/`no`, integers, bare words. glyphsLib reads
   AND and OR right to left without precedence; FontCraft uses NOT before AND before OR with
   parentheses, as the documented predicate format does. Attributes: glyphsLib accepts about 60
   glyph properties; FontCraft implements the handbook's table plus `production`,
   `leftKerningGroup`, `rightKerningGroup`, `leftMetricsKey`, `rightMetricsKey`, `unicode`,
   `subCategory`, `export`, `hasComponents`, `hasCorners`, `isSmartGlyph`, and reports others
   by name.
7. Do predicates include non-exporting glyphs, and in which order are results listed?
   **Answered from glyphsLib:** non-exporting glyphs are left out; glyph order.
8. Which glyph property paths exist beyond metrics and anchors; rounding of arithmetic results;
   division by zero.
   **Decision:** only metrics and anchors (guess on `token.glyphProperty.layerChoice`). Results are
   rounded half away from zero to an integer, as fea-rs rounds (glyphsLib truncates); division by
   zero is a compile error at the token.
9. May a number-value expression contain a second number value, and may it start with a literal?
   **Answered from glyphsLib and fea-rs:** both accept further number values and literals anywhere
   in the expression; glyphsLib also accepts parentheses, which FontCraft accepts too.
10. How do number and glyph property tokens behave in a variable export: variable values, or the
    value of the default master?
   **M9.** fea-rs resolves `$name` to a variable value through its variation-info interface;
    glyph property tokens need the same, built by M9 (`token.numberValue.variable`).

Extensions
11. Scope of a `condition`: until the next condition, the end of the feature, or `#endif`? Is
    there a way to return to unconditional rules inside the same block?
   **M9** (the `condition` rows are M9's).
12. Coordinate scale of `condition` limits and of `(axis:coord)` locations (internal versus
    external), and whether the upper limit is inclusive.
   **M9.**
13. Are `#else`, nesting or symbols other than VARIABLE accepted?
   **Decision for the static half built in M8:** no; another symbol or `#else` is an error at the
   directive. The rows stay M9 (`featureCode.ifdefOtherSymbols`).
14. Does `standalone` work on lookups in prefixes only as a no-op, and can a standalone lookup be
    called before its definition?
   **Decision:** a no-op in prefixes (handbook); calling a lookup before its definition is an
    error, as in standard syntax. The lowering is in `featureCode.standaloneLowering`.
15. Class-based multiple substitution: exact error when class sizes differ.
   **Answered:** fea-rs's message, mapped to the entry.

Automatic features
16. The exact generated code for every † row, including lookup structure, class names used and the
    languagesystem list.
   **Partly answered from the corpus** (the reference's stored output in 27 sources): case, sups,
    subs, sinf, numr, dnom, tnum, pnum, ssNN, liga, dlig, frac (fallback), ordn, aalt, locl,
    ccmp and the Languagesystems prefix; each finding is in its row's `notes`. The other † rows
    carry a guess and a check in `docs/plans/M8.md` § 8.
17. The complete list of automatic classes and of scripts routed to dist, abvm and blwm.
   **Partly answered:** automatic classes in the corpus are Uppercase, Lowercase, AllLetters and
    All (`autoClass.general`); dist, abvm and blwm follow fontc's writers (M5.17, M6.17).
18. Precedence when a ligature qualifies for several ligature features, and how liga's default list
    interacts with `.liga`, `.dlig` suffixes.
   **Decision:** the order liga list or `.liga`, then `.dlig`, then hlig, then rlig, then dlig
   (guess on `autoFeature.generalRule`).
19. Whether `.quartwidth` is recognised besides the handbook's `.qartwidth`.
   **Decision:** both are recognised (guess on `autoFeature.qwid`).
20. Behaviour of an explicit kern or mark feature without a `# Automatic Code` line.
   **Answered from the oracles** (`implicit.markerMissing`): kern gets the generated code after its
    own; the mark family gets none.
21. How frac is built when numerators and denominators are missing, and which precomposed
    fractions it then uses.
   **Partly answered from the corpus:** ligature rules onto the precomposed fractions the font has
    (`sub one slash four by onequarter;`), from the afrc list and the Unicode vulgar fractions.

Other
22. Which text fields expand `{{{...}}}` tokens, and the list of key paths users rely on.
23. Whether regex search is anchored in every field and which regex dialect features beyond the
    documented subset are used in practice (look-around, back-references).
24. Whether the reference URL scheme has actions other than `show`.

Language
25. fea-rs rows marked unknown in the checklist need a test each before they are relied on.
   **Planned:** M8.2 adds one test per such row through the wrapper.
26. AFDKO v1.27 variable syntax (units required, `locationDef`, per-location records) and the
    fea-rs/feaLib syntax differ; decide which FontCraft writes and whether it reads both.
   **Answered (open decision D5):** FontCraft writes what fea-rs accepts (the feaLib form) when it
    lowers the reference's variable syntax, and reads AFDKO's form where fea-rs does; built in M9.
