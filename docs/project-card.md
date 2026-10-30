# Vision MCP — project card

> **Give coding agents eyes on the web without a browser.**

Vision MCP is a small Rust rendering and layout-auditing engine for coding
agents. It turns HTML/CSS into a PNG, an element index, an honest audit, and an
interactive session — on machines that cannot run Chromium, a WebView, a
browser, or Playwright.

| | |
| --- | --- |
| **Repository** | <https://github.com/JamalMellouki/vision-mcp> |
| **MCP server** | `vision mcp` / `vision-mcp` |
| **Status** | `0.0.1-alpha` |
| **License** | Apache-2.0 |
| **Core** | Rust, no browser engine |
| **Footprint** | Alpha binaries around 6 MB |
| **Platforms** | Linux x86_64/ARM64, macOS x86_64/ARM64, Windows x86_64 |
| **Primary users** | Coding agents, low-resource developers, CI/UI maintainers |

## The problem

A coding agent can edit a component but cannot see whether it fits, overlaps,
overflows, has an accessible control, or reacts to a click. A full browser is
often too heavy for the machine running the agent — a phone, a small ARM board,
a constrained container, or a Termux session.

## The idea

```mermaid
flowchart LR
    A[Agent] -->|MCP / JSON-RPC| B[Vision]
    B --> C[Snapshot]
    C --> D[DOM + CSS + Layout + Paint]
    D --> E[PNG]
    D --> F[Element geometry]
    D --> G[Audit]
    B --> H[Retained session]
    H --> I[Click / focus / type / scroll]
    I --> D
```

Vision provides the missing feedback loop without shipping a browser:

1. render a local page;
2. inspect exact element rectangles and text;
3. audit accessibility and layout;
4. click, focus, type, scroll, and re-capture;
5. report only what the engine can honestly verify.

## Philosophy

```mermaid
flowchart TB
    subgraph Principles[Design principles]
        P1[Small by default]
        P2[Agent-first interface]
        P3[Honest about limits]
        P4[Local and private]
        P5[Composable adapters]
    end
    P1 --> R1[No browser engine in the core]
    P2 --> R2[MCP tools and retained sessions]
    P3 --> R3[Unsupported features block the audit]
    P4 --> R4[No I/O after a snapshot]
    P5 --> R5[Chromium adapter is optional]
```

- **Small is a feature.** A renderer that fits on a weak device is more useful
  than a browser that does not fit at all.
- **Honesty beats a convincing screenshot.** A blocked audit is better than a
  high score on an approximate render.
- **The agent is a first-class user.** Screenshots, geometry, and session actions
  are the interface, not an afterthought.
- **Local by default.** The core performs no network or filesystem I/O after a
  snapshot is built.
- **Adapters stay outside.** Chromium capture is allowed, but never required.

## Goals

| Goal | Alpha state |
| --- | --- |
| Render HTML/CSS without Chromium or WebView | implemented subset |
| Produce a useful element index | implemented |
| Audit layout and accessibility honestly | implemented, blocked when approximate |
| Let an agent interact with a page | implemented for click/focus/type/scroll |
| Run on low-memory Linux and Termux-class devices | design target; CI builds Linux ARM64 |
| Make the MCP interface simple and scriptable | implemented |
| Support the React/framework matrix | React 19 verified; Preact/Vue/Svelte/Solid planned |
| Offer a stable 1.0 API | not yet |

## What it can do today

```mermaid
flowchart TB
    subgraph Render[Render]
        R1[HTML parsing]
        R2[CSS cascade]
        R3[Block / inline / table]
        R4[Flex / grid subset]
        R5[Arabic shaping]
        R6[PNG / JPEG / GIF / WebP / SVG]
    end
    subgraph Inspect[Inspect]
        I1[Element rectangles]
        I2[Text and attributes]
        I3[Contrast]
        I4[Targets and labels]
        I5[Overflow / clipping / overlap]
    end
    subgraph Interact[Interact]
        J1[QuickJS DOM]
        J2[Events and timers]
        J3[Retained sessions]
        J4[Click / focus / type / scroll]
    end
    subgraph Agent[Agent interface]
        A1[vision_shot]
        A2[vision_inspect]
        A3[vision_audit]
        A4[vision_open / click / type]
    end
```

## The agent loop

```mermaid
sequenceDiagram
    participant A as Coding agent
    participant M as vision-mcp
    participant S as Session
    participant E as Engine

    A->>M: vision_open(page, viewport)
    M->>E: prepare snapshot
    E->>S: DOM, layout, QuickJS
    M-->>A: session id + PNG + elements
    A->>M: vision_click(session, target)
    M->>S: dispatch event
    S->>E: restyle, relayout, repaint
    M-->>A: new PNG + elements + compat
    A->>M: vision_audit(session)
    E-->>M: audit report
    M-->>A: score, blocked, issues
    A->>M: vision_close(session)
```

The important rule: **check `compat.unsupported` and `audit.blocked` before
trusting a visual result.**

## Who it is for

### Target developers

- **Coding-agent and MCP client builders** who need a local visual tool.
- **Frontend and design-system engineers** working on constrained machines.
- **Termux, ARM, and low-RAM users** who cannot run a browser stack.
- **CI and release engineers** who want deterministic screenshots and geometry
  without Chromium in every job.
- **Open-source maintainers** who need a small, scriptable UI regression check.
- **Accessibility and layout reviewers** who need cheap first-pass findings.

### Not the primary target

- Teams that need pixel identity with Chromium.
- Untrusted public-page rendering in the native engine.
- Full browser API or JavaScript compatibility.
- A hosted screenshot service.

## Upcoming development

```mermaid
timeline
    title Vision MCP roadmap
    section 0.0.1-alpha
      Browserless renderer : Core DOM, CSS, layout, paint
      Agent interface : MCP server, sessions, inspect, audit
      Distribution : GitHub Actions alpha builds
    section Next
      Events : Capture/bubble, dispatchEvent, keyboard, pointer
      DOM APIs : getComputedStyle, fragments, cloning, matchMedia
      Frameworks : Preact, Vue 3, Svelte 5, Solid fixtures
      CSS : Positioning, stacking, gradients, shadows, pseudo-elements
    section Beta
      Capture : vision-build and framework adapters
      Runtime : vision-wasm and edge target
      Quality : PNG goldens, nightly fuzzing, benchmarks
    section 1.0
      Stability : Stable API and compatibility matrix
      Platforms : Reproducible cross-platform releases
```

Near-term work, in priority order:

1. Event semantics: capture/bubble, `once`/`passive`, `dispatchEvent`,
   keyboard, pointer, change, and submit.
2. DOM APIs needed by real components: `getComputedStyle`, `dataset`,
   `cloneNode`, fragments, `getBoundingClientRect`, and `matchMedia`.
3. Framework fixtures: React, Preact, Vue, Svelte, and Solid.
4. CSS P0: absolute/fixed positioning, stacking, gradients, shadows,
   pseudo-elements, and overflow clipping.
5. `vision-build` and `vision-wasm` vertical slices.
6. Pixel goldens, nightly fuzzing, and a low-memory benchmark on ARM.

## Technical identity

```mermaid
flowchart LR
    subgraph Core[Core engine]
        DOM[Arena DOM]
        CSS[CSS cascade]
        LAY[Layout]
        TXT[Text shaping]
        PNT[Paint]
        AUD[Audit]
    end
    subgraph Inputs[Inputs]
        SNAP[Snapshot]
        BR[Optional Chromium adapter]
    end
    subgraph Outputs[Outputs]
        PNG[PNG]
        ELEM[Element index]
        REPORT[Audit report]
    end
    BR --> SNAP
    SNAP --> DOM
    DOM --> CSS --> LAY --> PNT --> PNG
    LAY --> ELEM
    LAY --> AUD --> REPORT
```

- **No browser engine in the core.** No Chromium, WebKit, Blink, WebView,
  Playwright, or Lightpanda.
- **QuickJS** provides trusted local JavaScript with an in-tree DOM facade.
- **A `Snapshot`** is the trust boundary: after it is built, the renderer does
  not touch the network or filesystem.
- **A `Session`** keeps the document and runtime alive across agent actions.

## Project health

| Signal | Value |
| --- | --- |
| Workspace tests | 97 passing on the current development machine |
| CI | format, clippy, tests, rustdoc, fuzz-target check |
| Release automation | tag-driven GitHub Actions matrix |
| Browser test | opt-in via `VISION_BROWSER` |
| Fuzz target | `fuzz/fuzz_targets/html.rs` |
| License | Apache-2.0 |

## Non-goals

- Pixel identity with Chromium.
- A security sandbox for untrusted pages.
- A complete browser API surface.
- A remote screenshot service.
- A sample application that becomes the product.

## Links

- [README](../README.md)
- [Getting started](getting-started.md)
- [Compatibility](compatibility.md)
- [Agent integration](agent-integration.md)
- [Architecture](architecture.md)
- [Roadmap](roadmap.md)
- [Presentation outline](presentation.md)
