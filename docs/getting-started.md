# Getting started

Vision renders local HTML and CSS without a browser. This guide covers the CLI,
the library, sessions, and the optional Chromium adapter.

## Requirements

- Rust 1.85 or newer for source builds.
- A Linux, macOS, or Windows machine with a C toolchain supported by Rust.
- No Chromium, WebView, Playwright, Node.js, or network access is needed for the
  core renderer.
- For the optional `vision-browser` adapter, an installed Chromium and a local
  page are required.

## Install a release

Download the archive matching your platform from
[Releases](https://github.com/JamalMellouki/vision-mcp/releases), extract it, and put
`vision` (and optionally `vision-mcp`) on `PATH`.

Release archives contain both binaries and the project licenses. Verify the
published SHA-256 checksum before running a downloaded binary.

## Build from source

```sh
git clone https://github.com/JamalMellouki/vision-mcp.git
cd vision-mcp
cargo build --release --bin vision --bin vision-mcp
cargo test --workspace
```

Install the CLI into Cargo's bin directory:

```sh
cargo install --path crates/vision-cli
```

## Render a page

```sh
vision shot page.html -o page.png
vision shot page.html -o page.png --viewport 390x844 --dpr 2
vision shot page.html -o page.png --no-javascript
```

The default viewport is `1280x800` with DPR 1. A page may reference local
stylesheets, images, and scripts relative to the HTML file. Everything must stay
inside the document directory.

`--no-javascript` makes a page explicitly static. If the document contains a
script, the compatibility report records a blocking
`script: disabled by configuration` entry and the audit score becomes zero.

## Inspect geometry

```sh
vision inspect page.html > elements.json
```

The JSON contains:

- output width, height, and document height;
- every laid-out element with tag, id, classes, text, and border box;
- compatibility notes;
- phase timings.

The element index is suitable for an agent that needs to find a target before
clicking or typing.

## Audit a page

```sh
vision audit page.html > audit.json
```

The audit checks small targets, missing form labels, low contrast, overflow,
clipping, empty elements, and sibling overlaps.

The audit is deliberately conservative. It reports `blocked: true` and a score
of zero when:

- a script exists but was not executed;
- a resource referenced by a file snapshot could not be loaded;
- layout or painting uses an unsupported feature that makes the render
  approximate.

A blocked audit is not a failure of the page; it means Vision cannot honestly
claim a precise result.

## Use the library

```rust
use vision::{Config, Engine, Snapshot, Viewport};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = Engine::native(Config::default())?;
    let snapshot = Snapshot::from_html("<h1>Hello</h1>")
        .with_viewport(Viewport::new(1280, 800));
    let capture = engine.render(&snapshot)?;
    std::fs::write("page.png", capture.png)?;
    Ok(())
}
```

A `Snapshot` can also be loaded from a file, deserialized from JSON, or built by
the optional browser adapter. The renderer performs no I/O after construction.

## Keep a session

A session retains the document, styles, layout, scroll offset, and QuickJS
context. It is the right interface for an agent that needs to act on a page.

```rust
use std::time::Duration;
use vision::{Engine, Snapshot, Target, Viewport};

let engine = Engine::native(Default::default())?;
let snapshot = Snapshot::from_html(
    r#"<button id="go">Go</button><div id="out">waiting</div>
       <script>
         document.getElementById("go").addEventListener("click", () => {
           document.getElementById("out").textContent = "done";
         });
       </script>"#,
).with_viewport(Viewport::new(390, 844));

let mut session = engine.open(snapshot)?;
session.click(&Target::Id("go".to_owned()))?;
session.advance_time(Duration::from_millis(16))?;
let capture = session.capture()?;
```

`click`, `focus`, `type_text`, and `scroll` dispatch events to registered
listeners. `advance_time` makes timer-driven behavior deterministic in a
headless test.

## Use the MCP server

Start the server:

```sh
vision mcp
```

Configure an MCP-capable client to launch that command. The server exposes the
same features as the CLI plus retained sessions. See
[agent-integration.md](agent-integration.md) for the tool contracts.

The standalone binary is equivalent:

```sh
vision-mcp
```

The server reads newline-delimited JSON-RPC from stdin and writes responses to
stdout. Diagnostics go to stderr.

## Capture with Chromium (optional)

The core does not need Chromium. If a client-rendered page cannot be represented
by a local snapshot, `vision-browser` can capture it through the DevTools
protocol:

```sh
vision capture http://127.0.0.1:3000/ -o page.png
vision capture http://127.0.0.1:3000/ --wait selector:.app-ready --snapshot page.json
```

The adapter launches an installed Chromium, waits for a selected readiness
condition, records the final DOM, preloads stylesheets and images, and drops the
browser before rendering. It is intentionally separate from the core engine.

## Troubleshooting

### The audit score is zero

Read `compat.unsupported` in the JSON. Common causes are an unexecuted script, a
missing stylesheet/image, absolute positioning, or another documented layout gap.

### A font is missing

The engine uses system fonts. Install a font that covers the script (Arabic and
CJK coverage depend on the host) or use a preloaded snapshot whose font is
available on the machine.

### Scripts do not run

Scripts are trusted-local only. Check that scripts are enabled
(`Config::javascript`), that the script file is inside the document directory,
and that the page is served from a snapshot with the script bytes preloaded.

### The page looks different from Chromium

Vision is not a pixel-identical browser. It implements a documented subset and
reports gaps. Use reviewed compatibility baselines for regressions, not blind
pixel comparisons with a browser.
