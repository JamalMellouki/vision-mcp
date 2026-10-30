# Agent integration

Vision is designed to give coding agents a visual and structural check without
installing a browser. This document describes the public integration surface; it
does not contain model-specific prompts or internal instructions.

## Why use it

A coding agent can read source code, but it cannot reliably know whether a
component fits, overlaps, overflows, has an accessible control, or reacts to a
click. Vision provides:

1. a PNG of the rendered page;
2. element rectangles and text;
3. accessibility and layout findings;
4. a retained session for click, focus, typing, scrolling, and timers.

This is enough for a local edit-and-verify loop on a machine that cannot run a
browser.

## Start the server

```sh
vision mcp
```

The standalone binary is equivalent:

```sh
vision-mcp
```

The server speaks newline-delimited JSON-RPC on stdin and writes responses to
stdout. Diagnostics are written to stderr. It does not open a network port.

Generic MCP client configuration:

```json
{
  "mcpServers": {
    "vision": {
      "command": "vision",
      "args": ["mcp"]
    }
  }
}
```

Use the absolute path to the binary if the client does not inherit the shell
`PATH`. The same stdio contract works from a terminal, an editor-integrated
client, or a CI script.

## Tools

### `vision_shot`

Render a file or inline HTML to a PNG. Returns an image content block and JSON
metadata.

```json
{
  "input": "page.html",
  "viewport": "390x844",
  "dpr": 2,
  "javascript": true
}
```

You can pass `html` instead of `input`. `output` is optional; when present, the
PNG is written to that path and a text result is returned.

### `vision_inspect`

Render the page and return metadata without requiring the caller to decode a
PNG:

```json
{
  "html": "<button id=\"go\">Go</button>",
  "viewport": "390x844"
}
```

The text content is a JSON object containing `elements`, `compat`, and
`timings`. Use `elements[].rect` and `elements[].id` to choose the next target.

### `vision_audit`

Render with the audit enabled and return an audit report with the screenshot.

```json
{
  "input": "page.html",
  "viewport": "1280x800"
}
```

Always inspect `audit.blocked`. A blocked audit has `score: 0`; do not describe
the page as passing accessibility or layout checks.

### `vision_open`

Create a retained session:

```json
{
  "input": "page.html",
  "viewport": "390x844",
  "javascript": true
}
```

The text result contains a `session` id. Reuse that id for every subsequent
action. Sessions are bounded; close them when finished.

### `vision_click`

```json
{
  "session": "s0",
  "id": "go"
}
```

Targets can be selected by `id`, `selector`, `text_target`, or `x`/`y`. The
response is a new screenshot plus metadata, including the current element index
and compatibility report.

### `vision_focus`

```json
{
  "session": "s0",
  "selector": "input[name=q]"
}
```

### `vision_type`

```json
{
  "session": "s0",
  "id": "name",
  "text": "Ada"
}
```

Typing writes the value, dispatches an input event, and repaints. Use `inspect`
or the returned element index to check controlled components and preview text.

### `vision_scroll`

```json
{
  "session": "s0",
  "dx": 0,
  "dy": 240
}
```

### `vision_capture`

Return a screenshot of the current session state without changing it.

### `vision_close`

Release the session and its QuickJS context.

## Recommended loop

```text
1. vision_open or vision_shot
2. vision_inspect to locate the target
3. vision_click / vision_focus / vision_type
4. vision_capture to inspect the result
5. vision_audit for layout and accessibility
6. vision_close
```

Do not repeatedly open a new session for every action. A retained session is
what preserves form state, React state, timers, and event listeners.

## Example: verify a React counter

```text
vision_open  { input: "counter.html", viewport: "390x844" }
vision_click { session: "s0", id: "increment" }
vision_click { session: "s0", id: "increment" }
vision_capture { session: "s0" }
vision_close { session: "s0" }
```

Compare the second capture with the first. A healthy session should show the
updated counter text and an empty `compat.unsupported` set.

## Resource limits

- Inline HTML: 8 MiB over MCP.
- File snapshot: 32 MiB.
- Text per typing action: 1 MiB.
- Concurrent sessions: 16.
- Script deadline, heap, and stack are configurable in the Rust `Config`.
- The native engine makes no network requests.

These limits exist to keep an agent from accidentally exhausting a small device.
They are not a substitute for isolating untrusted content.

## Security and trust

The embedded QuickJS runtime executes trusted local scripts. It is **not** a
sandbox. Do not point the native engine at untrusted public pages.

For public pages, use the optional `vision-browser` adapter inside an isolated
container. It launches Chromium, captures the final DOM and resources, and
returns a snapshot. The adapter has its own trust model and is not part of the
core renderer.

## Low-resource devices

- Prefer file snapshots and `--no-javascript` when interaction is not needed.
- Use `viewport` and `dpr` deliberately; a 390x844 DPR 1 capture is much cheaper
  than a desktop retina capture.
- Close sessions promptly.
- Use `vision_audit` on pages that fit the supported CSS subset; check
  `compat.unsupported` before treating a score as meaningful.
- Build from source on Termux if a prebuilt glibc binary is not compatible with
  the device.
