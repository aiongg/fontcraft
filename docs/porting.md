# Porting scripts and plug-ins

Scripts for the reference app are Python programs that run inside it against its object API
(`Glyphs.font`, `GSGlyph`, `GSLayer`…). FontCraft does not run Python and has no object API, so
those scripts do not load. Its API is the command registry: every action, from adding a node to
exporting a family, is a command with an id and JSON parameters, and the same commands run from
the menus, macros, the command line, the control channel and MCP.

This guide maps common tasks of the reference API to FontCraft commands. It names the reference
API only to show what each task looked like there.

## Choosing what to port to

| The reference script or plug-in… | Port it to |
|---|---|
| runs a fixed list of menu actions on the selected glyphs | a macro ([`macros.md`](macros.md)) |
| needs loops, conditions, files, the network or Python libraries | a program outside the app that sends commands over the control channel (below), MCP or `fontcraft-cli run` |
| is a filter, reporter or palette plug-in that works on one layer | a module ([`plugins.md`](plugins.md)): filters and palettes become filter modules, reporters become reporter modules |
| checks glyphs and reports problems | a check module, run by the audit |
| is a tool, file-format or general plug-in | a program outside the app; FontCraft has no module kind for interface code |

You can keep writing Python: a short client reaches the running app over the control channel
([`control-protocol.md`](control-protocol.md#clients) has clients in Python, JavaScript, Rust and
the shell). The repository ships no Python package; the examples are short enough to copy.

## What works differently

| In the reference API | In FontCraft |
|---|---|
| Objects you read and change (`layer.LSB = 50`) | Commands you run (`spacing.setLeftSidebearing {"glyphs": ["a"], "value": 50}`); queries return JSON |
| The current font, the selected layers, the active master | Explicit targets: every command names its glyphs, layers and nodes in its parameters. `ui.inspect` returns the selection when a program needs it |
| A layer addressed as `glyph.layers[master.id]` | A layer id in the `layer` or `layers` parameter; a master layer's id is its master's id. Left out, most commands use the first master's layer |
| `glyph.beginUndo()` and `glyph.endUndo()` | Each command is one undo step. `command.batch` or a macro makes several commands one undo step, and none takes effect if one fails |
| `font.disableUpdateInterface()` | Not needed: commands do not redraw while they run |
| `print()` to the Macro Panel | The Command Console shows every command with its result; filter modules write `log` lines there |
| `Glyphs.defaults["key"]` | `settings.get` and `settings.set` |

## Common tasks

Command ids are listed with the parameters that matter for the task. `commands.list` (or
`fontcraft-cli commands`) shows every parameter of every command.

### Files and fonts

| Task | Reference API | FontCraft |
|---|---|---|
| Open a font | `Glyphs.open(path)` | `file.open` `{"path"}` |
| Create a font | `GSFont()` | `file.new` |
| Save | `font.save()`, `font.save(path)` | `file.save` `{"path"?}` |
| Close | `font.close()` | `file.close` |
| Read a font file without opening it | | `font.inspect` `{"input"}` |
| Set the family name, units per em, version | `font.familyName`, `font.upm`, `font.versionMajor` | `font.info.set` `{"values": {"familyName", "unitsPerEm", "versionMajor"}}` |
| Add or change a custom parameter | `font.customParameters["name"] = value` | `font.customParameters.add`, `font.customParameters.set` `{"scope", "name", "value"}` |
| List the masters | `font.masters` | `font.master.list` |
| Set a master's x-height, cap height, ascender | `master.xHeight`, `master.capHeight`, `master.ascender` | `font.master.setMetric` `{"masters", "metric", "position"}` |
| List or add instances | `font.instances`, `GSInstance()` | `font.instance.list`, `font.instance.add` |
| Export static fonts | `instance.generate()` | `file.exportStatic` `{"outlines", "containers", "destination"}` |
| Export a variable font | `instance.generate()` on a variable font setting | `file.exportVariable` |

### Glyphs

| Task | Reference API | FontCraft |
|---|---|---|
| List the glyphs | `font.glyphs` | `font.listGlyphs` |
| Read a glyph's properties | `font.glyphs["a"]` | `glyph.getInfo` `{"glyphs": ["a"]}` |
| Find glyphs | a loop over `font.glyphs` with a test | `glyph.search` `{"query"}`, `fontView.queryGlyphs` `{"rules"}` |
| Add a glyph | `font.glyphs.append(GSGlyph("a.alt"))` | `glyph.newGlyph` `{"name"}` |
| Remove glyphs | `del font.glyphs["a.alt"]` | `glyph.removeGlyph` `{"glyphs"}` |
| Duplicate | `glyph.duplicate()` | `glyph.duplicateGlyph` `{"glyphs", "suffix"?}` |
| Rename, updating components, kerning and keys | `glyph.name = "b"` | `glyph.rename` `{"glyph", "newName"}` |
| Set code points | `glyph.unicodes = ["0061"]` | `glyph.setUnicodes` `{"glyphs", "unicodes"}` |
| Set whether it exports | `glyph.export = False` | `glyph.setExport` |
| Set the colour label | `glyph.color = 3` | `glyph.setColorLabel` `{"glyphs", "color"}` |
| Set the note, tags | `glyph.note`, `glyph.tags` | `glyph.setNote`, `glyph.setTags` |
| Set the category | `glyph.category`, `glyph.subCategory` | `glyph.setCategory` `{"glyphs", "category", "subCategory"}` |
| Set the production name | `glyph.productionName` | `glyph.setProductionName` |
| Update from the glyph data | `glyph.updateGlyphInfo()` | `glyph.updateGlyphInfo` |
| Look up the glyph data for a name or code point | `Glyphs.glyphInfoForName("a")`, `Glyphs.glyphInfoForUnicode("0061")` | `glyphData.lookup` `{"name"}` or `{"unicode"}` |

### Layers and metrics

| Task | Reference API | FontCraft |
|---|---|---|
| Read a layer's shapes, nodes and anchors | `layer.shapes`, `layer.paths`, `layer.anchors` | `layer.inspect` `{"glyph", "layer"?}` |
| Read the width and sidebearings | `layer.width`, `layer.LSB`, `layer.RSB` | `spacing.getMetrics` `{"glyph"}` |
| Read the bounds | `layer.bounds` | `font.listGlyphs` (each glyph's `bounds`) |
| Set the width | `layer.width = 600` | `spacing.setWidth` `{"glyphs", "value"}` |
| Set the sidebearings | `layer.LSB = 50`, `layer.RSB = 50` | `spacing.setLeftSidebearing`, `spacing.setRightSidebearing` `{"glyphs", "value"}` |
| Set a metrics key | `layer.leftMetricsKey = "=n"` | `spacing.setLeftSidebearing` `{"glyphs", "value": "=n"}` |
| Apply the metrics keys | `layer.syncMetrics()` | `glyph.updateMetrics` `{"glyphs"}` |
| Add a backup layer | `glyph.layers.append(layer.copy())` | `layers.addBackupLayer` `{"glyphs"}` |
| Rename a layer | `layer.name = "Draft"` | `layers.rename` `{"glyph", "layer", "name"}` |
| Add a guide | `layer.guides.append(GSGuide())` | `guide.add` `{"glyph", "x", "y", "angle"}` |

### Outlines

| Task | Reference API | FontCraft |
|---|---|---|
| Draw a path | `layer.shapes.append(GSPath())` with `GSNode`s | `path.addPath` `{"glyph", "nodes": [{"x", "y", "type"}]}` |
| Move nodes | `node.position = (x, y)` | `selection.move` `{"glyph", "points", "dx", "dy"}` |
| Remove overlaps | `layer.removeOverlap()` | `path.removeOverlap` `{"glyphs"}` |
| Correct path directions | `layer.correctPathDirection()` | `path.correctPathDirection` `{"glyphs"}` |
| Reverse a path | `path.reverse()` | `path.reverseContours` `{"glyph", "shapes"}` |
| Round coordinates | `layer.roundCoordinates()` | `path.roundCoordinates` `{"glyphs"}` |
| Add nodes at extremes | `layer.addNodesAtExtremes()` | `path.addExtremes` `{"glyphs"}` |
| Tidy up paths | `layer.cleanUpPaths()` | `path.tidyUpPaths` `{"glyphs"}` |
| Scale, slant, move | `layer.applyTransform(matrix)` | `path.transformations` `{"glyphs", "translate", "scale", "slant"}`, `transform.scale` |
| Copy the outline to the background | copying `layer.shapes` into `layer.background` | `path.selectionToBackground` `{"glyphs"}` |
| Empty the background | removing the shapes of `layer.background` | `path.clearBackground` `{"glyphs"}` |
| Run a built-in filter | the Filter menu, or the filter's own code | `filter.apply` `{"filter", "args", "glyphs"}` |

### Components and anchors

| Task | Reference API | FontCraft |
|---|---|---|
| Add a component | `layer.shapes.append(GSComponent("acutecomb", (x, y)))` | `glyph.addComponent` `{"glyph", "component"}`, then `component.setTransform` |
| Build composites from the glyph data | the Glyph › Create Composite menu item | `glyph.createComposite` |
| Decompose | `layer.decomposeComponents()` | `glyph.decomposeGlyph` `{"glyphs"}`; one level: `component.decompose` |
| List what uses a glyph | a loop over every layer's components | `glyph.listComposites` |
| Add an anchor | `layer.anchors.append(GSAnchor("top", (x, y)))` | `anchor.add` `{"glyph", "name", "x", "y"}` |
| Add the default anchors | the Glyph › Set Anchors menu item | `glyph.setAnchors` `{"glyphs"}` |
| Move an anchor | `anchor.position = (x, y)` | `anchor.setPosition` |

### Kerning

| Task | Reference API | FontCraft |
|---|---|---|
| Set a pair | `font.setKerningForPair(master.id, "T", "o", -40)` | `kerning.setPair` `{"left", "right", "value", "master"}` |
| Read a pair | `font.kerningForPair(master.id, "T", "o")` | `kerning.getPair` `{"left", "right", "master"}` |
| Remove a pair | `font.removeKerningForPair(master.id, "T", "o")` | `kerning.removePair` |
| List the pairs | `font.kerning` | `kerning.listPairs` |
| Set kerning groups | `glyph.leftKerningGroup`, `glyph.rightKerningGroup` | `kerning.setLeftGroup`, `kerning.setRightGroup` `{"glyphs", "group"}` |

### Features

| Task | Reference API | FontCraft |
|---|---|---|
| Add a feature | `font.features.append(GSFeature("ss01", code))` | `font.features.addFeature` `{"tag", "code"}` |
| Change a feature's code | `feature.code = …` | `font.features.setCode` `{"kind", "name", "code"}` |
| Add a class | `font.classes.append(GSClass("Uppercase", code))` | `font.features.addClass` `{"name", "code"}` |
| Regenerate the automatic features | `font.updateFeatures()` | `font.features.update` |
| Compile the features | `font.compileFeatures()` | `font.features.compile` |

### Checks

| Task | Reference API | FontCraft |
|---|---|---|
| Check that masters are compatible | `glyph.mastersCompatible` | `audit.run` |
| Check a font with your own rules | a script that loops over the glyphs | a check module, run by `audit.run` |

### Several steps as one

| Task | Reference API | FontCraft |
|---|---|---|
| Group edits into one undo step | `glyph.beginUndo()`, `glyph.endUndo()` | `command.batch` `{"label", "steps"}` |
| Run a saved script | the Script menu | `script.run` `{"macro"}` |
| Repeat the last script | the Script menu's item that runs the last script again | `script.rerunLast` |

## Plug-ins

The reference app's plug-ins are Python or Objective-C bundles built from its SDK templates.
FontCraft modules are WebAssembly files built from the templates in
[`examples/module-template/`](../examples/module-template/).

| Reference plug-in | FontCraft module | What changes |
|---|---|---|
| Filter with or without dialog: `filter(layer, inEditView, customParameters)` | filter module | the layer arrives as JSON and the module returns its new shapes; the dialog is built from the manifest's parameters |
| Reporter: `foreground(layer)`, `background(layer)` | reporter module | the module returns drawing primitives instead of drawing; sizes in screen pixels, coordinates in font units |
| Palette | filter module with parameters | it appears as a Palette section with a form and an Apply button |
| Custom parameter filter at export (`Filter` with a plug-in name) | filter module | the `Filter` and `PreFilter` parameters name the module by id or name; `include:` and `exclude:` work as before |
| Tool, file format, general | none | write a program outside the app that uses the control channel or MCP |

Because a module only sees the layer it is given, a plug-in that read other glyphs or the font's
settings needs that data as parameters, or becomes an outside program.

## A worked example

A reference script that sets both sidebearings of the selected glyphs to 50:

```python
for layer in Glyphs.font.selectedLayers:
    layer.LSB = 50
    layer.RSB = 50
```

As a macro, saved in the Macros folder and run from the Script menu:

```json
{
  "format": "fontcraft.macro",
  "version": 1,
  "title": "Sidebearings 50",
  "description": "Sets both sidebearings of the selected glyphs to 50 units.",
  "steps": [
    { "command": "spacing.setLeftSidebearing", "params": { "glyphs": { "$arg": "glyphs" }, "value": 50 } },
    { "command": "spacing.setRightSidebearing", "params": { "glyphs": { "$arg": "glyphs" }, "value": 50 } }
  ]
}
```

As a Python program outside the app, with the client from
[`control-protocol.md`](control-protocol.md#python) and the app started with `--control 7985`.
The program reads the selection itself and can add any logic around the commands:

```python
fc = FontCraft()
glyphs = fc.request("ui.inspect")["selected_glyphs"]
fc.run("command.batch", label="Sidebearings 50", steps=[
    {"command": "spacing.setLeftSidebearing", "params": {"glyphs": glyphs, "value": 50}},
    {"command": "spacing.setRightSidebearing", "params": {"glyphs": glyphs, "value": 50}},
])
```

Without the app, on a file:

```sh
fontcraft-cli run --in MyFont.glyphs \
  --cmd spacing.setLeftSidebearing --params '{"glyphs": ["a", "b"], "value": 50}' \
  --cmd spacing.setRightSidebearing --params '{"glyphs": ["a", "b"], "value": 50}' \
  --save MyFont.glyphs
```
