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
| `--root DIR` | File parameters must lie inside DIR. Repeatable. Default: the working directory; `--root /` admits every path |
| `--read-only` | Only commands that read (`query: true` in the catalogue) run |
| `--allow PATTERN` | Only commands whose id matches a pattern run. Repeatable |
| `--deny PATTERN` | Commands whose id matches are refused, even when an allow pattern matches. Repeatable |

- Patterns are shell-style over the whole id: `*` is any run of characters, `?` one character
  (`glyph.*`, `kerning.set*`).
- A refused command is a tool result with `isError: true` and the reason. It changes nothing.
- `list_commands` lists only the commands the server admits. Resource reads and completions run
  query commands, so the same rules apply to them.
- `command.batch` is checked step by step: one refused step refuses the whole batch. It must
  itself be admitted.
- File parameters are the text values (or lists of texts) of the parameters `path`, `paths`,
  `file`, `files`, `folder`, `folders`, `destination`, `directory`, `font`, `fonts`, `input`,
  `inputs`, `output` and `outputDir`, at any depth. Each is resolved (relative to the working
  directory, symbolic links followed) and replaced by its absolute path before the command runs.
  A missing part of a path may not contain `..`.
- `file.save` without `path` writes the document's own file: under `--root`, every open
  document's file must lie inside the roots.
- Commands that run steps or open files the server cannot see in their parameters (`script.run`
  and the other macro commands, `file.openRecent`, `url.open`, `url.show`) are refused whenever
  any restriction is set, which with the default root is always. Run the steps with
  `command.batch` instead.
- `initialize` describes the restrictions in its `instructions`.

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
