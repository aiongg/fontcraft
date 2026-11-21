# Settings: behaviour notes

Source: Glyphs 4 Handbook, chapter "Settings". Inventory: `spec/inventory/16-settings.toml`.

## Settings window

- Opened with `app.settings` (application menu › Settings…, Cmd+Comma). On macOS 12 the item is
  named Preferences. FontCraft draws its own menu bar, so the item is the last one of the Edit
  menu on every platform.
- Seven sections (`window.settings` and one `settings.*` panel each): Updates, Appearance, User
  Settings, Sample Strings, Sharing, Addons, Shortcuts. In FontCraft a section that is not built
  yet stays in the switcher, greyed with the reason, and `app.settings` refuses to open it. Each
  section is an entry in `SECTIONS` (`crates/ui/src/shell/settings.rs`) whose body a
  package fills; a body changes settings only through `settings.set` or its own commands.
- Preferences persist across installing a newer or older version (`settings.surviveVersionChange`).
- Headless access is our addition: `settings.get {id?}` and `settings.set {id, value}` address any
  `[[setting]]` row by id. Button actions have their own commands.

## Updates

| Item | Id | Notes |
|---|---|---|
| Check Now | `settings.checkForUpdatesNow` | immediate check; a found update is offered for download together with its change list |
| Automatically check for updates | `updates.automaticallyCheck` | periodic background check; recommended on |
| Show cutting edge versions | `updates.showCuttingEdgeVersions` | include frequent, less-tested beta builds |

Going back to a stable build is done by downloading it again; there is no in-app downgrade. Users on
beta builds should work on copies of their files.

## Appearance

| Setting | Id | Kind | Effect |
|---|---|---|---|
| Display Unicode Value | `appearance.displayUnicodeValue` | bool | Font View cells of encoded glyphs show either a generic Unicode badge or the code point, bottom right |
| Always use Light Mode (Font View) | `appearance.alwaysLightModeFontView` | bool | light content under a dark system appearance |
| Always use Light Mode (Edit View) | `appearance.alwaysLightModeEditView` | bool | same for Edit View |
| Text View Width | `appearance.textViewWidth` | number | wrap width of Edit View text in 1/1000 em, independent of UPM |
| Handle size | `appearance.handleSize` | choice | drawn size of nodes, handles and anchors |
| Display Mark Cloud | `appearance.displayMarkCloud` | bool | overlay of all attaching combining marks while an anchor is selected |

Mark cloud: shown on every glyph as soon as an anchor is selected; which marks are shown per glyph
and anchor is defined by the glyph info database.

Colours: nine colours, each stored for the light and for the dark appearance
(`appearance.color.<name>.light` / `.dark`, 18 settings):

| `<name>` | Used for |
|---|---|
| `cornerNodes` | corner nodes |
| `smoothNodes` | smooth nodes |
| `alignmentZones` | alignment zones |
| `foregroundStroke` | outline of the foreground layer |
| `backgroundStroke` | outline of the background layer |
| `otherLayersStroke` | outlines of other layers made visible with the eye toggle |
| `canvas` | Edit View canvas |
| `kerningNegative` | negative-kerning indicator, default light blue |
| `kerningPositive` | positive-kerning indicator, default yellow |

`settings.resetColors` (the Standard button) restores all of them.

## User Settings

| Setting | Id | Effect |
|---|---|---|
| Keep Glyph Names from Imported Files | `user.keepGlyphNamesFromImportedFiles` | opened files keep their glyph names instead of being renamed to the default scheme |
| Disable Automatic Alignment for Imported Files | `user.disableAutomaticAlignmentForImportedFiles` | components of imported fonts are not auto-aligned |
| Disable Localization | `user.disableLocalization` | UI stays English; needs a restart |
| Use Versions | `user.useVersions` | system versioned saving: autosave on close, browsable history; off avoids writes on open/close |
| Send Crash Reports | `user.sendCrashReports` | send diagnostics after a crash |
| name, email | `user.crashReportName`, `user.crashReportEmail` | contact details attached to crash reports |

FontCraft stores Disable Localization, Use Versions, Send Crash Reports and the two contact fields
without effect: there are no translations, no version history and no crash-report service. The
line under each checkbox says so.

## Sample Strings

Data: an ordered list of groups, each a list of strings (`sampleStrings.groups`). Edit View offers
these strings for insertion (see the Edit View text notes).

- Plus adds a group (`settings.addSampleStringGroup`), minus deletes the selected group
  (`settings.removeSampleStringGroup`).
- The text field shows the selected group, one string per line (`settings.setSampleStrings` headless).
- Default (`settings.resetSampleStrings`) restores the built-in strings and deletes custom groups.

String syntax:

| Written | Result |
|---|---|
| any Unicode text | those characters, including combining marks |
| `\n` (backslash, n) | line break |
| `/name` + space | the glyph with that name; the space ends the name and is not output |
| `/a/b` | consecutive names need no space between them |
| `/Placeholder` | a placeholder that always shows whichever glyph is currently selected |

A literal space after a word is kept: a name-terminating space is only the one directly after a
slash-name.

## Sharing

`sharing.enableExternalPreview` lets a companion viewer app on a phone or tablet in the same network
mirror the active Edit View.

- The viewer lists computers; picking one triggers a confirmation on the computer. Approved devices
  are trusted from then on.
- The viewer follows the active Edit View and updates on every edit. Pinch zooms, one-finger drag
  pans, touch-and-hold opens the viewer's menu.
- `settings.resetTrustedDevices` (Reset) clears the trusted list; the button appears only after a
  first connection.
- Connection problems: same wireless network, restart both, try another network, avoid routers
  running mixed wireless standards.

### External Preview in FontCraft

FontCraft has no companion app. While `sharing.enableExternalPreview` is on (off by default), the
desktop app serves a page that any browser on the local network can open
(`apps/fontcraft/src/external_preview.rs`).

| Step | What happens |
|---|---|
| Turn on | The app listens on a free port of every IPv4 interface, under a random path. `sharing.status` gives the address to type on the device. |
| First visit | The device, known by its IP address, gets a page asking it to wait. The page reloads every 2 seconds. `sharing.status` lists the device as waiting. |
| Answer | `sharing.answerDevice {device, trust}`. Trusted devices are kept in the user's settings (`sharing.trustedDevices`). Refused devices are turned away while the preview runs. |
| Watch | A trusted device gets the active tab's glyphs, filled on one line in the tab's master, as SVG inside the page. The page reloads every second with a meta refresh and has no script. The browser's own zoom and scrolling apply. |
| Reset | `settings.resetTrustedDevices` forgets every trusted device. |
| Turn off | The port closes; the address and the waiting and refused devices are forgotten. |

Limits: 64 trusted devices (the oldest go first), 8 waiting at once, 64 refused. The page is drawn
at most 4 times a second, and only while a device is trusted. The HTTP server is the hinting
preview's (`apps/fontcraft/src/preview_server.rs`): `GET` and `HEAD` only, one request per
connection, 4 connections, 64 KB of headers, 50 requests a second.

While automation roots are in force, turning the preview on and trusting a device are refused, so a
confined agent cannot send the open font to the network.

## Addons

| Setting | Id | Effect |
|---|---|---|
| Python version | no row | interpreter for scripts and plug-ins; Python 3+; relaunch to apply |
| Use system console for script output | `addons.useSystemConsoleForScriptOutput` | script log output is routed to the system console, not the Macro Panel |
| Alternate Plugin Repos | `addons.alternatePluginRepos` | extra repositories, one per line |

- If no interpreter works, install Python from Plugin Manager › Modules and choose the entry marked
  "(Glyphs)".
- FontCraft embeds no Python, so the Python version setting, the need for an interpreter, its
  installation from Modules and the relaunch after a change have no counterpart and no inventory row
  (`docs/plans/M13.md` § 7 question 2); see the Extensions notes.

Alternate repositories:

- Added to the main public repository, never replacing it.
- Entry forms: HTTPS URL, optionally protected by HTTP Basic authentication; or a full local path.
- Definition file: a dictionary `packages` with lists `plugins`, `scripts`, `modules` (unused lists
  may be omitted). Each entry: `titles` and `descriptions` (dictionaries by language code), `url`,
  `path`, `screenshot`. More fields exist in the main repository's file.
- The screenshot URL is requested with `Accept-Language` so servers can localise it.

## Shortcuts

Data: `shortcuts.custom`, a map from action to key combination (or none).

- A shortcut is one or more modifiers (Command, Option, Control, Shift) plus a regular key such as a
  letter, digit, Return or Escape. Displayed with key symbols.
- Record: select an action, click Record shortcut, press the keys (`settings.recordShortcut`).
  While recording, the curved arrow keeps the existing shortcut
  (`settings.cancelShortcutRecording`) and the cross removes it (`settings.removeShortcut`).
- List filtering (`settings.filterShortcuts`): All, Customized (changed shortcuts only), Menu
  (menu-bar actions), Commands; plus a search field.
- Actions are grouped in collapsible sections (`settings.toggleShortcutSection`).
- Scripts can be given shortcuts here (stated in the Extensions chapter).
- Ours: the list is generated from the command registry and keyed by command id
  (`shortcuts.oneRegistry`); `settings.listShortcuts` and `settings.resetShortcut` are additions.

## Open questions

- Default value of every checkbox and number (the chapter states none except the two kerning colours).
- Handle size: the list of choices and its default.
- Display Unicode Value: which state shows the code point.
- Text View Width: default, limits, and whether it applies to all tabs immediately.
- Exact default colours for both appearances; whether further colours exist that the chapter omits.
- Update check interval and where the change list is shown.
- Labels of the crash-report name and email fields; whether they are sent when the option is off.
- Whether a Reset Warnings control still exists and in which section.
- Sample strings: how groups are named and reordered; whether a literal slash or backslash can be
  escaped; what happens to an unknown glyph name; whether the group list has a fixed first group.
- Sharing: default state, discovery protocol, and whether the trusted list is shown.
- Python version: how interpreters are discovered and what the list shows when none is found.
- Alternate repositories: how credentials are entered (in the URL or prompted), and when the list is
  refreshed.
- Shortcuts: what the Commands scope really contains (the chapter's wording is unclear), whether a
  shortcut without a modifier can be recorded, how conflicts between two actions are reported, and
  whether there is a reset-to-default control.
