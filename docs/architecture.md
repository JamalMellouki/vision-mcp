# Architecture

Vision is split so that a renderer, a command-line tool, an integration server,
and client-rendered capture adapters can evolve independently.

## Crates

| Package | Status | Responsibility |
| --- | --- | --- |
| `vision-engine` | implemented | Document, styles, text, layout, paint, audit, image decoding, snapshots, sessions. |
| `vision-cli` | implemented | The `vision` binary: `shot`, `inspect`, `audit`, `capture`, `mcp`. |
| `vision-mcp` | implemented | JSON-RPC integration server and automation tools. |
| `vision-browser` | implemented (Chromium) | Capture a snapshot from a local Chromium page through the DevTools protocol. Lightpanda remains planned. |
| `vision-build` | planned | Run a local build or SSR entry and return a snapshot. |
| `vision-wasm` | planned | Expose the renderer to browsers and edge runtimes. |

Only packages with real code are workspace members. A placeholder crate that
does nothing is worse than a documented gap.

## Render pipeline

```text
HTML ──parse──> arena DOM
                 │
local scripts ───┤ (QuickJS + DOM facade, before styles)
                 │
author CSS ──────┼──> cascade ──> computed styles
UA CSS ──────────┘                    │
                                     ▼
fonts, images ───────────────> block/inline/table layout
                                     │
                                     ▼
                                  paint ──> PNG
                                     │
                                     ├──> element index
                                     └──> audit
```

`Snapshot` is the boundary between capture and rendering. It carries HTML,
preloaded stylesheets, preloaded binary and script resources, a viewport, and
initial state. After `Snapshot` is built, the renderer performs no network or
filesystem access. Trusted local scripts run in an embedded QuickJS runtime with
an in-tree DOM and browser-API subset.

The browser adapter uses one local Chromium process per capture. A DevTools
WebSocket creates a page target, waits for the requested readiness point, reads
the final DOM, and copies the stylesheets and images referenced by that DOM into
the snapshot. The browser is dropped before rendering. Network access belongs
to the adapter; layout and paint never use it.

## Invariants

- The core embeds QuickJS for trusted local scripts; it does not embed a browser
  and is not a security sandbox.
- The DOM is arena-based and indexed. The parser's temporary tree is discarded.
- Resource loading belongs to `Snapshot` construction, not to layout or paint.
- A compatibility report records unsupported features instead of silently
  producing a convincing but wrong image.
- A session retains parsed and laid-out state; it does not reparse the page for
  every action.
- The prepared layout and the QuickJS context share one `Rc<RefCell<Document>>`;
  sessions mutate that document in place instead of cloning it per event.
- A session retains the QuickJS context when scripts exist, dispatches DOM
  events to listeners, drains due timers and microtasks, then recomputes style
  and layout from the mutated DOM.
- `Session::advance_time` advances the monotonic timer clock deterministically;
  `Config::javascript = false` keeps scripts inert and marks their presence as
  a blocking compatibility issue.
- Each DOM node has one stable JavaScript wrapper per context. Framework expando
  properties therefore survive repeated property access.
- Rendering and auditing share the same layout tree.
- Script failures and unsupported APIs are compatibility errors and block the
  audit score.

## Dependency direction

```text
vision-browser ─┐
vision-build ───┼──> vision-engine <── vision-cli
vision-wasm ────┘                         │
                                          └──> vision-mcp
```

`vision-browser`, `vision-build`, and `vision-wasm` never become dependencies of
the renderer. The renderer must remain usable without a browser, a Node
toolchain, or a network.
