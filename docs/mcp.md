# MCP server

`fontcraft-cli mcp` runs a [Model Context Protocol](https://modelcontextprotocol.io) server on
stdio: newline-delimited JSON-RPC 2.0, protocol revision `2025-06-18` (`2024-11-05` is also accepted; `initialize` answers with the one
the client asked for when it is one of the two, else with the newest). Logs go
to stderr; stdout carries only protocol messages.

| Mode | How | What works |
|---|---|---|
| **Attached** | `fontcraft-cli mcp --connect 127.0.0.1:7985` (the app must run with `fontcraft --control 7985`) | Everything: calls are forwarded over the [control protocol](control-protocol.md), and you see the app change |
| **Headless** | `fontcraft-cli mcp --headless` | An engine session inside the process, starting with a new font. Everything except window and view commands, which answer that they need the running app |

With no flag the server tries `127.0.0.1:7985` and falls back to headless.

## Register it

```sh
cargo build --release -p fontcraft-cli
claude mcp add fontcraft -- "$PWD/target/release/fontcraft-cli" mcp
```

Other clients take the same command in their configuration:

```json
{"mcpServers": {"fontcraft": {"command": "/abs/path/target/release/fontcraft-cli", "args": ["mcp"]}}}
```

## Tools

| Tool | Arguments | Returns |
|---|---|---|
| `list_commands` | `{pattern?}` | every command (or those whose id or label contains `pattern`): id, label, menu path, shortcut, a description of its parameters and result, whether it is enabled |
| `run_command` | `{command, params?}` | the command's JSON result. A failed command is a result with `isError: true` and the reason; it changes nothing |
| `inspect_document` | `{}` | the open fonts: title, path, unsaved changes, revision, family name, glyph count, masters, undo and redo steps |

An agent's first call is `list_commands`; every command documents its own parameters. Example:

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"path.addPath","params":{"glyph":"A","nodes":[{"x":60,"y":0},{"x":560,"y":0},{"x":310,"y":700}]}}}}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"run_command","arguments":{"command":"file.exportStatic","params":{"destination":"demo.ttf"}}}}' \
  | fontcraft-cli mcp --headless
```

## Permissions

| Option | Effect |
|---|---|
| `--automation-read-root DIR` | Every file a command reads must lie inside DIR. Given once |
| `--automation-write-root DIR` | Every file a command writes must lie inside DIR. Given once |
| `--read-only` | Only commands that read (`query: true` in the catalogue) and write no files run |
| `--allow PATTERN` | Only commands whose id matches a pattern run. Repeatable |
| `--deny PATTERN` | Commands whose id matches are refused, even when an allow pattern matches. Repeatable |

Without any of these options nothing is restricted: commands read and write anywhere, and macros
run.

### Confining file access

```sh
fontcraft-cli mcp --automation-read-root /work/fonts --automation-write-root /work/fonts/build
```

```json
{"mcpServers": {"fontcraft": {"command": "/abs/path/target/release/fontcraft-cli",
  "args": ["mcp", "--automation-read-root", "/work/fonts", "--automation-write-root", "/work/fonts/build"]}}}
```

- Read and write access are granted separately. To let an agent read and write one folder, give
  it to both flags. A root left out grants none of its access: with only a read root, nothing can
  be written to disk. Save and export commands without a path still return the bytes.
- Each root must be an existing folder; a missing one, or a file, stops the server with an error.
  Each flag is given once, as `--flag DIR` or `--flag=DIR`.
- The roots confine the headless server and imply `--headless`. With `--connect` they are an
  error: a running app confines itself. Start it as `fontcraft --control 7985
  --automation-read-root DIR --automation-write-root DIR` (see
  [Control protocol](control-protocol.md#confining-file-access)).
- What is checked:
  - Each command declares the parameters that name files (`file_params` in the catalogue: name,
    `read` or `write`, and form) and whether it writes to disk (`writes_files`). A `read`
    parameter must lie inside the read root, a `write` parameter inside the write root.
  - Form `path`: a path or a list of paths. Form `tables`: `"TAG"`, `"TAG=FILE"` (several
    comma-separated in one string) or `{tag, file?, from?: {font}}`; the file parts are checked.
    Form `document`: an open document's title or path is left as it is; anything else is checked
    as a path. `keywords` are values that are not paths (`"nice"` for a list of names).
  - A source a command reads may read other files in turn: the UFOs of a designspace, the Import
    Font and Import Master files, a project's linked font. They must lie inside the read root too,
    and are checked before anything is read.
  - Places a command uses without a parameter naming them:

    | Command | Checked |
    |---|---|
    | `file.save` without `path`; `file.exportWithProfiles` with `bySource` | Every open document's file lies inside the write root |
    | `project.save` without `path` | Every open project's file lies inside the write root |
    | `project.export` | Every open project's export destination lies inside the write root, its linked font inside the read root |
    | `script.saveActionSet` without `path`; `script.recordMacro` with `name` | Refused: they write into the Macros folder |

  - Files a document links to are read only inside the read root, whenever a command reads them:
    Import Font and Import Master sources (at export, in hinting previews, when reviewing external
    changes), images, glyph data files beside the document and feature includes. A link that
    points outside the read root is treated as a missing file, with a warning. This also covers
    links an agent sets during the session.
  - `command.batch` checks each step when it runs. A refused step fails the batch, and the
    session is as it was before it.
  - Commands that run steps or open files no parameter names (`script.run` and the other macro
    commands, `file.openRecent`, `url.open`, `url.show`) are refused, and `list_commands` leaves
    them out. Run the steps one by one, or with `command.batch`.
- How a path is compared:
  - It is made absolute against the server's working directory, and its links are followed. A
    file or folder link that leads out of a root is outside it.
  - For a file that does not exist yet, the links of its nearest existing folder are followed and
    the rest of the path must be plain names: `..` below a missing folder is refused, as is a link
    that leads nowhere (writing through it would create its target).
  - The result is compared with the root by whole folder names: `/work2/x` is not inside `/work`.
    On Windows case is ignored, UNC paths (`\\server\share\…`) are resolved as drive-letter paths
    are, and device names (`CON`, `NUL`, `COM1`…) are refused.
  - The command then runs on the absolute path that was checked.
- A refused path is an ordinary tool error the agent can read and correct, for example
  ``file.saveAs: `path`: automation path rejected: outside the write root /work/fonts/build: /etc/x.glyphs``
  or ``file.saveAs: `path`: automation filesystem access is not granted: write authority is absent: /work/fonts/x.glyphs``.
- Outside the desktop app, the engine writes no file that an instance's Export Folder parameter
  would put outside the export destination, so exports stay inside their destination.
- A path is checked, then opened. Another program that swaps in a link between the two can still
  win that race: the roots keep an agent to the files it was given; they do not guard against
  other software on the computer.

### Command restrictions

- Patterns are shell-style over the whole id: `*` is any run of characters, `?` one character
  (`glyph.*`, `kerning.set*`).
- A refused command is a tool result with `isError: true` and the reason. It changes nothing.
- `list_commands` lists only the commands the server admits. Resource reads and completions run
  query commands, so the same rules apply to them.
- `command.batch` is checked step by step before it runs: one refused step refuses the whole
  batch. It must itself be admitted.
- Under `--read-only`, a command that is not in the catalogue is refused, and so is a query that
  writes files (`writes_files`, or a `write` parameter, as `tables.delete` with `inPlace` or
  `font.dumpTables` with `output`).
- With `--read-only`, `--allow` or `--deny`, macros and the other commands that run steps the
  server cannot see are refused.
- `initialize` describes the restrictions and the roots in its `instructions`.

## Prompts

`prompts/list` and `prompts/get` offer workflows. Each prompt's text names the commands that do
the task, in order; when the server refuses some of them, the text says which.

| Prompt | Arguments |
|---|---|
| `start_family` | `family_name`, `glyphs?`, `file?` |
| `add_master` | `name`, `axes?`, `from_master?` |
| `build_accented_glyphs` | `glyphs` |
| `kern_pair_list` | `pairs` (one pair per line: left, right, optional value), `master?` |
| `prepare_release` | `destination?`, `version?` |

## Resources

| URI | Contents (JSON) |
|---|---|
| `fontcraft://document` | the open documents, as `inspect_document` returns them |
| `fontcraft://glyph/{name}` | the glyph's properties (`glyph.getInfo`) and, for each master (at most 64), its layer as `layer.inspect` returns it |
| `fontcraft://command/{id}` | the command's catalogue entry: label, menu, shortcut, parameters, `query` |
| `fontcraft://parameter/{name}` | the custom parameter's catalogue entry (name or another spelling, any case) |
| `fontcraft://feature/{tag}` | the feature's code, notes and labels (`font.features.get`) |

`resources/list` lists `fontcraft://document`; the others are templates in
`resources/templates/list`. Names in URIs are percent-encoded. A resource that does not exist,
or that the server's rules refuse, is error `-32002`.

## Completion

`completion/complete` completes the variables of the resource templates (glyph names, command
ids, custom parameter names, feature tags) and the prompt arguments that name glyphs or masters.
Matches start with the typed text, ignoring case, in font or catalogue order, at most 100. A value
with spaces completes its last word. With no document open, or when the needed query is refused,
the list is empty.

## Logging

`logging/setLevel {level}` turns on `notifications/message` at that level and above (`debug`,
`info`, `notice`, `warning`, `error`, `critical`, `alert`, `emergency`). No notifications are sent
before the client sets a level.

| Level | Sent when |
|---|---|
| `debug` | a prompt or resource is read; a completion found nothing to offer |
| `info` | a command ran: `{command, ok: true}` |
| `warning` | a command failed (`{command, error}`) or was refused (`{command, refused}`) |
| `error` | a bug inside the server was caught |

Notifications go out before the reply to the request that caused them; at most 1,000 per request.

## Protocol behaviour

- `initialize`, `ping`, `tools/list`, `tools/call`, `prompts/list`, `prompts/get`, `resources/list`,
  `resources/templates/list`, `resources/read`, `completion/complete`, `logging/setLevel`.
  Notifications from the client are accepted and never answered. Lists come whole: a `cursor` is
  refused.
- Malformed JSON is `-32700`; a request that is not an object, lacks `method`, or exceeds 4 MiB is
  `-32600`; an unknown method is `-32601`; an unknown tool, prompt or log level, a missing tool name, or wrong prompt arguments are `-32602`; a bug
  inside a tool is `-32603` and the server keeps serving.
- A line that is not UTF-8 gets a parse error and the session continues.
- One line in, one reply out: while a tool call runs the server reads nothing. Progress,
  cancellation, resource subscriptions and server-to-client requests therefore do not exist yet (the
  sibling apps have the same limit, for the same reason).

## Planned

| What | Milestone |
|---|---|
| Task-level tools over common command sequences: `open_font`, `draw_path`, `set_metrics`, `export_font`, `render_glyphs` (returns an image), `screenshot` | M3 |
| HTTP transport: `--transport http --listen ADDR` | M13 |
