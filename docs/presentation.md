# Vision — presentation outline

A ten-slide technical presentation for a release, a demo, or a maintainer
meeting.

## 1. Title

**Vision**  
Browserless UI rendering and layout auditing for coding agents.

- One image: a rendered component or the smoke corpus.
- One sentence: see and interact with web UI without Chromium.

## 2. The gap

- Agents can write HTML/CSS faster than they can inspect it.
- Browsers are heavy for phones, ARM boards, containers, and Termux.
- Screenshot services add network, privacy, and latency problems.
- Source reading does not reveal overlap, clipping, contrast, or focus order.

## 3. The approach

- A self-contained `Snapshot` with HTML, CSS, resources, viewport, and state.
- An in-tree arena DOM, cascade, layout, text shaping, paint, and audit.
- A retained session with events and timers.
- A local JSON-RPC interface for agents.

## 4. Architecture

Show the pipeline:

```text
Snapshot -> DOM -> cascade -> layout -> paint -> PNG
                         |-> element index
                         |-> audit
Session -> events/timers -> restyle -> recapture
```

Call out the optional Chromium adapter as a boundary, not a core dependency.

## 5. Demo: render

```sh
vision shot corpus/smoke.html -o smoke.png
vision inspect corpus/smoke.html
vision audit corpus/smoke.html
```

Show the PNG, a selected element rectangle, and the compatibility report.

## 6. Demo: interact

```text
vision_open  counter.html
vision_click counter.html#increment
vision_capture
```

Show the counter moving from 0 to 1 to 2 and the compatibility report remaining
empty.

## 7. Demo: honest failure

Render a page with a missing stylesheet or a disabled script.

- The compatibility report names the problem.
- The audit is blocked.
- The score is zero.
- No false success.

## 8. Footprint

- No browser process or browser engine.
- Alpha binaries around 6 MB.
- Runs on low-memory Linux and is a candidate for Termux builds.
- Configurable script timeout, heap, stack, and HTML limits.

## 9. Roadmap

- Event semantics: capture/bubble, `dispatchEvent`, keyboard, pointer.
- DOM API batch: `getComputedStyle`, fragments, cloning, media queries.
- Framework fixtures: React, Preact, Vue, Svelte, Solid.
- CSS P0: positioning, stacking, gradients, shadows, pseudo-elements.
- `vision-build`, `vision-wasm`, and cross-platform alpha releases.

## 10. Call to action

- Try the alpha release.
- Run the MCP server with a coding agent.
- Report a compatibility gap with a minimal fixture.
- Contribute a test or a corpus page.

Repository: <https://github.com/JamalMellouki/vision-mcp>
