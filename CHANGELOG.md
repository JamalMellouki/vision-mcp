# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Embedded QuickJS runtime with a DOM facade for trusted local scripts. React 19
  production and development client rendering and hydration run before style and
  layout without Chromium.
- Script resources in `Snapshot` loading, script compatibility blocking, DOM
  mutation, inline `innerHTML`, classes, styles, and common node properties.
- A timer queue with `setTimeout`, `setInterval`, their clear functions,
  `requestAnimationFrame`, `queueMicrotask`, microtask-before-timer ordering,
  and `Session::advance_time` for deterministic headless advancement.
- `Config::javascript` and `--no-javascript` for explicit static rendering. A
  disabled script is reported in the compatibility report and blocks audits.
- `Snapshot::warnings` records remote, escaping, unreadable, or missing local
  stylesheets and resources; the engine turns them into blocking compatibility
  errors instead of silently rendering an incomplete page.
- A checked-in corpus compatibility manifest with an integration test, and a
  standalone `libFuzzer` HTML target under `fuzz/`.
- `Config::max_html_bytes`, `Config::script_timeout`,
  `Config::script_memory_bytes`, `Config::script_stack_bytes`, and a 32 MiB
  file-snapshot cap.
- Interactive JavaScript sessions: retained QuickJS contexts, event listener
  registration and removal, bubbling, `preventDefault`, `stopPropagation`,
  `onclick`-style handler properties, and click/focus/input dispatch.
- Stable DOM wrapper identity per JavaScript context, preserving framework
  internals; React 19 click state updates and controlled text inputs now work
  through `Session`.
- Hydration DOM properties `firstChild`, `lastChild`, `childNodes`, and an
  attributes collection; React 19 production and development hydration now
  preserve the server DOM without React #418 or recovery errors.
- `vision-browser` captures a local Chromium page through the DevTools protocol,
  waits for load, network-idle, fonts, or a selector, and returns a
  self-contained `Snapshot` with final DOM, current form values, linked CSS,
  images, viewport, DPR, and scroll state.
- `vision capture` exposes the browser adapter from the command line and can
  write the captured snapshot as JSON alongside the rendered PNG.
- True atomic inline layout for images, form controls, buttons, and
  shrink-to-fit `inline-block` boxes. Atomic boxes wrap, align on the baseline,
  paint in line order, and are visible to the element index and session actions.
- Browser integration tests for a local page that hydrates its DOM after
  JavaScript and loads an external stylesheet and SVG image.

### Changed

- The old inline-parent promotion and its `inline formatting with an atomic
  child` compatibility warning are gone; atomic inline content now has its own
  line item and layout box.
- Corpus coverage includes inline-block chips and an image on a text baseline.
- The prepared document is now shared directly with the QuickJS context; event
  dispatch no longer clones the whole document on every action.
- `vision-browser` asks Chromium to choose an ephemeral DevTools port and reads
  `DevToolsActivePort`, removing the reserve-then-release port race.

### Fixed

- The public element index now reports input values, textarea content, and
  placeholders instead of only rendered text nodes.
- DOM serialization, subtree copying, `textContent`, and visible-text walks are
  iterative, so deeply nested documents cannot overflow the Rust stack.
- File snapshot resource loading no longer swallows missing or unreadable
  resources in `if let Ok(...)` branches.
- UTF-8-safe truncation of element text.
- CSS `width`/`height` override presentational image attributes.
- Auto horizontal margins center fixed-width blocks.
- Linear `calc()` lengths, `color: inherit`, and a zero basis in `flex` shorthand.
- CSS image aspect ratio follows the resolved width instead of stale HTML
  attributes.

## [0.0.1-alpha] - 2026-10-28

### Added

- Workspace, library crate, and `vision` command.
- HTML parsing, DOM arena, CSS cascade, block and inline layout.
- Text shaping with `rustybuzz`, glyph rasterization with `swash`, and per-script
  Latin/Arabic fallback.
- Table columns, captions, colspan, form controls, list markers, and local
  images.
- SVG images referenced by `<img>` are rasterized with resvg.
- `@media` width queries and a compatibility report.
- `vision shot`, `vision inspect`, and `vision audit`.
- `vision mcp` and a standalone `vision-mcp` server with render, inspect,
  audit, and session tools.
- Retained sessions with click, focus, type, scroll, and capture actions.
- Initial scroll and form values are applied from `Snapshot::state`.
- PNG output through `tiny-skia`.

[Unreleased]: https://github.com/JamalMellouki/vision-mcp/compare/v0.0.1-alpha...HEAD
[0.0.1-alpha]: https://github.com/JamalMellouki/vision-mcp/releases/tag/v0.0.1-alpha
