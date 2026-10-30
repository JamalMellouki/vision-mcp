# Compatibility and limitations

Vision implements a deliberate subset of the web platform. The goal is not
pixel identity with Chromium. The goal is a fast, self-contained render and an
honest report when the result is approximate.

## Rendering support

### Stable enough for the alpha corpus

- HTML parsing through `html5ever`, with an arena-backed DOM.
- CSS parsing, source order, specificity, inheritance, `!important`, and
  universal selectors.
- CSS custom properties, nested `var()`, fallbacks, and cycle detection.
- Block flow, inline flow, atomic inline content, tables, lists, and forms.
- A practical flexbox subset: row/column, wrapping, inline flex, gap, grow,
  shrink, basis, `justify-content`, `align-items`, and `align-self`.
- A practical grid subset: fixed/fractional tracks, `repeat()`, `minmax()`, and
  `auto-fit`/`auto-fill` with gaps.
- Relative positioning with `top`, `right`, `bottom`, and `left`.
- Border radius, letter spacing, text transform, box sizing, and linear
  `calc()` lengths.
- PNG, JPEG, GIF, WebP, and SVG resources.
- Latin and Arabic shaping with per-script font fallback.
- `@media` width queries and `display: none`.

### Verified JavaScript surface

- React 19 production and development client rendering.
- React 19 hydration over server-rendered DOM without recovery warnings in the
  maintained manual probes.
- DOM selection, mutation, text/attribute access, `innerHTML`, classes, styles,
  timers, and common node properties.
- A retained QuickJS context with `addEventListener`, `removeEventListener`,
  bubbling, `preventDefault`, `stopPropagation`, and click/focus/input dispatch.
- `setTimeout`, `setInterval`, clear functions, `requestAnimationFrame`, and
  `queueMicrotask` with deterministic session advancement.

JavaScript is trusted-local. QuickJS is not a security sandbox.

## Known gaps

These features are reported in `CompatReport` and block the layout audit when
they affect the result:

- absolute and fixed positioning, stacking contexts, and `z-index`;
- transforms, filters, shadows, gradients, and pseudo-elements;
- `@font-face`, local font resources, and font subsetting;
- overflow clipping in more than the current box-level cases;
- `object-fit`, inline SVG content, and advanced replaced content;
- advanced flexbox: reverse directions, `order`, baseline groups, and auto
  margins;
- advanced grid: spans, explicit rows, and alignment;
- table rowspan and full column-width constraint solving;
- RTL layout beyond current text direction handling;
- `min()`, `max()`, `clamp()`, and percentage gaps;
- modules, dynamic imports, network APIs, `fetch`, and `XMLHttpRequest`;
- capture-phase event options, `once`/`passive`, `dispatchEvent`, keyboard and
  pointer sequences, navigation, form submission, hover, and active states;
- iframes, WebGL, audio/video, Service Workers, and untrusted public web pages.

This list is not an excuse for silent approximation. A feature that changes the
result must appear in the compatibility report.

## Audit policy

`AuditReport.blocked` is true and `score` is zero when the render cannot be
trusted for an audit. This includes:

- an unexecuted script;
- a missing, unreadable, remote, or escaping local resource in a file snapshot;
- unsupported layout or painting features.

Agent integrations should check `blocked` before reporting a score or a visual
verdict.

## Framework matrix

| Framework | Status |
| --- | --- |
| React 19 CSR | verified in manual probes |
| React 19 hydration | verified in manual probes |
| React 19 interaction | click counters and controlled input verified |
| Preact | planned fixture |
| Vue 3 | planned fixture |
| Svelte 5 | planned fixture |
| Solid | planned fixture |

Fixtures will be committed as framework-focused pages. The repository does not
contain a product application or a remote API dependency.

## Platform matrix

CI builds and tests the workspace on Linux, macOS, and Windows. Release
automation produces:

- Linux x86_64 and ARM64;
- macOS x86_64 and ARM64;
- Windows x86_64.

The core targets low-memory Linux and Termux-class devices. The optional
Chromium adapter is not expected to run on the smallest devices; it is a
desktop/container escape hatch for pages that cannot be captured statically.
