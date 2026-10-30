# Roadmap

This file is a working plan, not a promise of dates. A milestone ships only when
its tests run in the workspace.

## Alpha: static render and local sessions

Implemented:

- Arena DOM, HTML parsing, CSS parsing and cascade.
- Block, inline, table, list, and form layout.
- Latin and Arabic shaping with per-script fallback.
- True inline-block and atomic inline flow for images, form controls, buttons,
  and shrink-to-fit inline blocks.
- PNG, JPEG, GIF, WebP, and `<img>` SVG resources.
- `@media` width queries, `display: none`, specificity, inheritance, and
  `!important`.
- CSS custom properties with inheritance, nested `var()`, fallbacks, and cycle
  detection.
- A first flexbox subset: row and column direction, wrapping, inline flex, gap,
  flex grow/shrink/basis, `justify-content`, `align-items`, and `align-self`.
- Relative positioning with `top`, `right`, `bottom`, and `left` offsets.
- A first grid subset: fixed, fractional, `repeat()`, `minmax()`, and
  `auto-fit`/`auto-fill` column tracks with row and column gaps.
- `border-radius`, `letter-spacing`, `text-transform`, and linear `calc()`
  lengths for component output.
- The universal selector, so CSS resets such as `* { box-sizing: border-box; }`
  participate in the cascade.
- Element index, contrast, target, label, overflow, clipping, empty, and overlap
  audits, with a hard zero score when unsupported features or an unexecuted
  script make the render approximate.
- A React 19 SSR corpus fixture, checked for unsupported features and card grid
  geometry.
- A trusted-local JavaScript runtime (QuickJS) with a DOM facade, script
  resources, style/layout reruns, compatibility blocking on script failures,
  and a timer queue with `setTimeout`, `setInterval`, clear functions,
  `requestAnimationFrame`, and microtask ordering.
- Shared document ownership between the prepared render and the retained
  QuickJS context; sessions no longer clone the document on every event.
- `Config::javascript`, `--no-javascript`, `Config::max_html_bytes`, and
  configurable script timeout/heap/stack limits, plus non-silent
  `Snapshot::warnings` for incomplete file snapshots.
- Deterministic `Session::advance_time` for timer-driven fixtures.
- Verified React 19 production and development client rendering and hydration
  without Chromium, including error-free hydration of the SSR DOM.
- Auto horizontal margins, CSS `color: inherit`, CSS image sizing over
  presentational attributes, and UTF-8-safe element text truncation.
- Retained QuickJS contexts in `Session`, with `addEventListener`,
  `removeEventListener`, bubbling, `preventDefault`, `stopPropagation`,
  `onclick`-style handler properties, and click/focus/input dispatch.
- Stable JavaScript identities for DOM nodes so framework internals such as
  React's fiber properties survive across accesses.
- React 19 production probes: state updates on click and controlled text input
  work through the retained session runtime.
- `Snapshot` with initial state, and `Session` with click, focus, type, and
  scroll.
- CLI and MCP server.

Next for alpha:

- Replace the temporary `markup5ever_rcdom` parse bridge with a custom
  `TreeSink` that writes directly into the arena.
- Split `js.rs`, `style.rs`, and `block.rs` along runtime, cascade, and layout
  responsibilities before adding the next large feature batch.
- Table rowspan and column width constraints.
- Image clipping inside overflow containers.
- Finish flexbox: reverse directions, `order`, baseline alignment, and auto
  margins.
- Finish grid: spans, explicit rows, and alignment.
- `min()`, `max()`, `clamp()`, and percentage gaps.
- More CSS properties: `background` shorthand details, `font` shorthand,
  `text-decoration` variants, gradients, and shadows.
- Audit checks that distinguish a deliberate overflow container from a defect.
- A corpus of local pages and reviewed PNG baselines, plus a golden geometry
  test that does not depend on a single font build. A compatibility manifest and
  integration test now cover the current corpus; a `libFuzzer` HTML target lives
  under `fuzz/`.
- More direct painter, layout, audit, and font tests; parser, serializer, DOM,
  timer, CLI, MCP, and browser-port helpers now have direct coverage.

## Beta: client-rendered pages and adapters

Implemented:

- `vision-browser` vertical slice: Chromium launch, DevTools transport, final
  DOM, linked CSS, images, viewport, DPR, scroll, and wait strategies.
- `vision capture` with PNG and snapshot JSON output.
- Browser integration tests for JavaScript hydration and preloaded resources.

Planned:

- `vision-browser`: Lightpanda support, redirect and data-URL resources, and a
  stable adapter API shared with `vision-build`.
- `vision-build`: Vite, Next.js, and a generic command adapter.
- `vision-wasm`: a small render entry that accepts `Snapshot` and returns PNG
  bytes.
- Harden the in-tree flex and grid layout against the React/Vue/Svelte corpus.
- Expand interaction events beyond click, focus, and input, including
  capture/bubble phases, `once`/`passive`, `dispatchEvent`, keyboard and pointer
  sequences.
- Expand DOM/browser APIs needed by hydration, routing, and component libraries.
- `@font-face`, local font resources, and font subsetting.
- CSS `url()` resources and `@import`.

## 1.0: compatibility and packaging

- Cross-platform prebuilt binaries for Linux ARM64, Linux x86_64, macOS, and
  Windows.
- A stable public API.
- Performance budgets on a low-memory ARM device.
- Fuzz targets for HTML (current), CSS, selectors, and image decoding.
- A documented compatibility matrix and a reproducible benchmark set.

## Explicit gaps

These are not silently approximated:

- Full browser-compatible JavaScript APIs, DOM events, navigation, and
  multi-context execution.
- Advanced flexbox: reverse directions, order, baseline groups, and auto
  margins.
- Advanced grid: spans, explicit rows, and alignment.
- Transforms, filters, gradients, shadows, and pseudo-elements.
- Inline SVG.
- Remote resources in static snapshots; the browser adapter loads page subresources.
- Navigation, form submission, label activation, hover, and active states.
- JavaScript sandboxing; embedded scripts are trusted local code only.
- Pixel identity with Chromium.
